//! Decision grading.
//!
//! EV loss = max EV among the node's actions - EV of the chosen action, for the same node and
//! the same combo. Differences inside the tolerance are reported as "close" (próxima).

use serde::{Deserialize, Serialize};

use crate::cards::Combo;
use crate::solution::NodeSolution;
use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Tolerance {
    /// Absolute tolerance in big blinds.
    pub abs_bb: f64,
    /// Relative tolerance as % of the pot before the decision.
    pub pct_pot: f64,
}

impl Default for Tolerance {
    /// Defaults sized to the solve target (0.5% of the starting pot): a loss below
    /// max(0.05 bb, 1% of the pot) is not distinguishable from noise of the modeled tree.
    fn default() -> Self {
        Tolerance {
            abs_bb: 0.05,
            pct_pot: 1.0,
        }
    }
}

impl Tolerance {
    pub fn threshold_chips(&self, pot_chips: i32, chips_per_bb: i32) -> f64 {
        (self.abs_bb * chips_per_bb as f64).max(self.pct_pot / 100.0 * pot_chips as f64)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Grade {
    /// Chosen action has the maximum EV.
    Best,
    /// Loss within tolerance.
    Close,
    /// Loss up to 5% of the pot.
    Inaccuracy,
    /// Loss above 5% of the pot.
    Mistake,
}

impl Grade {
    pub fn label_pt(self) -> &'static str {
        match self {
            Grade::Best => "Melhor ação",
            Grade::Close => "Próxima (dentro da tolerância)",
            Grade::Inaccuracy => "Imprecisão",
            Grade::Mistake => "Erro",
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Grade::Best => "best",
            Grade::Close => "close",
            Grade::Inaccuracy => "inaccuracy",
            Grade::Mistake => "mistake",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionResult {
    pub index: usize,
    pub label: String,
    pub frequency: f32,
    pub ev_chips: f32,
    pub ev_bb: f64,
    /// EV loss of this action relative to the best one, bb.
    pub loss_bb: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feedback {
    pub combo: String,
    pub chosen_index: usize,
    pub best_index: usize,
    pub best_ev_chips: f64,
    pub chosen_ev_chips: f64,
    pub loss_chips: f64,
    pub loss_bb: f64,
    /// Loss as % of the pot before the decision.
    pub loss_pct_pot: f64,
    pub tolerance_chips: f64,
    pub grade: Grade,
    /// Solver frequency of the chosen action for this combo.
    pub chosen_frequency: f32,
    pub actions: Vec<ActionResult>,
}

pub fn evaluate(
    node: &NodeSolution,
    combo: &Combo,
    chosen: usize,
    chips_per_bb: i32,
    tol: Tolerance,
) -> Result<Feedback, CoreError> {
    let ci = node.combo_index(combo).ok_or_else(|| {
        CoreError::NotEvaluable(format!(
            "combo {combo} não tem peso no nó {} (bloqueado ou fora do range)",
            node.node_id
        ))
    })?;
    if chosen >= node.num_actions() {
        return Err(CoreError::NotEvaluable(format!(
            "ação {chosen} inexistente no nó {}",
            node.node_id
        )));
    }
    let evs: Vec<f64> = node.ev.iter().map(|row| row[ci] as f64).collect();
    let (best_index, best_ev) =
        evs.iter()
            .copied()
            .enumerate()
            .fold((0usize, f64::NEG_INFINITY), |acc, (i, v)| {
                if v > acc.1 {
                    (i, v)
                } else {
                    acc
                }
            });
    let chosen_ev = evs[chosen];
    let loss = (best_ev - chosen_ev).max(0.0);
    let bb = chips_per_bb as f64;
    let threshold = tol.threshold_chips(node.pot_chips, chips_per_bb);
    let loss_pct = if node.pot_chips > 0 {
        100.0 * loss / node.pot_chips as f64
    } else {
        0.0
    };
    let grade = if loss <= 0.0 {
        Grade::Best
    } else if loss <= threshold {
        Grade::Close
    } else if loss_pct <= 5.0 {
        Grade::Inaccuracy
    } else {
        Grade::Mistake
    };
    let actions = node
        .actions
        .iter()
        .enumerate()
        .map(|(i, a)| ActionResult {
            index: i,
            label: a.label.clone(),
            frequency: node.strategy[i][ci],
            ev_chips: node.ev[i][ci],
            ev_bb: node.ev[i][ci] as f64 / bb,
            loss_bb: (best_ev - evs[i]).max(0.0) / bb,
        })
        .collect();
    Ok(Feedback {
        combo: combo.to_string(),
        chosen_index: chosen,
        best_index,
        best_ev_chips: best_ev,
        chosen_ev_chips: chosen_ev,
        loss_chips: loss,
        loss_bb: loss / bb,
        loss_pct_pot: loss_pct,
        tolerance_chips: threshold,
        grade,
        chosen_frequency: node.strategy[chosen][ci],
        actions,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solution::{ActionInfo, ActionKind, Seat};
    use crate::spot::Street;

    fn node(evs: Vec<Vec<f32>>, strat: Vec<Vec<f32>>) -> NodeSolution {
        let a = evs.len();
        NodeSolution {
            node_id: "t".into(),
            history: vec![],
            line: vec![],
            street: Street::River,
            board: vec![],
            player: Seat::Oop,
            position: "BB".into(),
            pot_chips: 1000,
            to_call_chips: 0,
            stack_chips: 5000,
            actions: (0..a)
                .map(|i| ActionInfo {
                    kind: ActionKind::Check,
                    amount_chips: 0,
                    label: format!("a{i}"),
                })
                .collect(),
            combos: vec!["AsKs".into(), "QhQd".into()],
            weights: vec![1.0, 1.0],
            strategy: strat,
            ev: evs,
            equity: vec![0.5, 0.5],
            reach: 1.0,
            runout: vec![],
        }
    }

    #[test]
    fn loss_is_best_minus_chosen_same_combo() {
        // chips_per_bb = 100. Combo 0: check 240.0, bet 240.1 chips (2.400 vs 2.401 bb).
        let n = node(
            vec![vec![240.0, 50.0], vec![240.1, 300.0]],
            vec![vec![0.5, 0.0], vec![0.5, 1.0]],
        );
        let c = Combo::parse("AsKs").unwrap();
        let fb = evaluate(&n, &c, 0, 100, Tolerance::default()).unwrap();
        assert_eq!(fb.best_index, 1);
        assert!((fb.loss_bb - 0.001).abs() < 1e-6);
        assert_eq!(fb.grade, Grade::Close, "0.001 bb must be shown as close");
        let fb = evaluate(&n, &c, 1, 100, Tolerance::default()).unwrap();
        assert_eq!(fb.grade, Grade::Best);
        assert_eq!(fb.loss_bb, 0.0);
        // Combo 1 uses its own EV column: check loses 2.5 bb = 25% pot.
        let q = Combo::parse("QhQd").unwrap();
        let fb = evaluate(&n, &q, 0, 100, Tolerance::default()).unwrap();
        assert!((fb.loss_bb - 2.5).abs() < 1e-9);
        assert_eq!(fb.grade, Grade::Mistake);
        assert!((fb.loss_pct_pot - 25.0).abs() < 1e-9);
    }

    #[test]
    fn inaccuracy_band() {
        let n = node(
            vec![vec![0.0, 0.0], vec![30.0, 0.0]],
            vec![vec![0.0, 1.0], vec![1.0, 0.0]],
        );
        let fb = evaluate(
            &n,
            &Combo::parse("AsKs").unwrap(),
            0,
            100,
            Tolerance::default(),
        )
        .unwrap();
        // 30 chips = 3% of pot 1000 > tolerance (max(5, 10) = 10 chips)
        assert_eq!(fb.grade, Grade::Inaccuracy);
    }

    #[test]
    fn combo_outside_node_is_not_evaluable() {
        let n = node(vec![vec![0.0, 0.0]], vec![vec![1.0, 1.0]]);
        assert!(evaluate(
            &n,
            &Combo::parse("2c2d").unwrap(),
            0,
            100,
            Tolerance::default()
        )
        .is_err());
        assert!(evaluate(
            &n,
            &Combo::parse("AsKs").unwrap(),
            3,
            100,
            Tolerance::default()
        )
        .is_err());
    }
}
