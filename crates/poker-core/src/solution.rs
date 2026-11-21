//! Engine-independent solution format (schema v1).
//!
//! A `SolutionFile` holds the spot configuration, solve metadata and a set of exported decision
//! nodes. Each node stores, for every eligible combo (non-zero reach, not blocked by the board),
//! the normalized weight, the strategy and the EV of each action. Values are in chips; use
//! `spot.chips_per_bb` to convert. The format does not depend on the solver's internal binary
//! layout, so updating the engine does not invalidate stored solutions.

use std::io::{Read, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::cards::Combo;
use crate::spot::{SpotConfig, Street};
use crate::CoreError;

pub const SOLUTION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EngineInfo {
    pub name: String,
    pub git_rev: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveInfo {
    pub method: String,
    pub max_iterations: u32,
    pub target_exploitability_chips: f64,
    pub final_exploitability_chips: f64,
    /// Final exploitability as % of the starting pot.
    pub final_exploitability_pct_pot: f64,
    pub compressed: bool,
    pub memory_estimate_bytes: u64,
    pub solve_ms: u64,
    pub threads: usize,
    pub status: SolveStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolveStatus {
    /// Target exploitability reached.
    Converged,
    /// Iteration limit reached before the target.
    IterationLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Fold,
    Check,
    Call,
    Bet,
    Raise,
    AllIn,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionInfo {
    pub kind: ActionKind,
    /// For bet/raise/all-in: the total amount the player has put in on this street after the
    /// action ("apostar X" / "aumentar para X"), in chips. 0 otherwise.
    pub amount_chips: i32,
    /// Display label in Portuguese, e.g. "Apostar 1,82 bb (33%)".
    pub label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Seat {
    Oop,
    Ip,
}

impl Seat {
    pub fn index(self) -> usize {
        match self {
            Seat::Oop => 0,
            Seat::Ip => 1,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSolution {
    /// Stable id inside the solution, e.g. `"root"`, `"x"`, `"x-b182"`.
    pub node_id: String,
    /// Action indices from the root (solver `apply_history` format).
    pub history: Vec<usize>,
    /// Human-readable line, e.g. ["BB check", "BTN aposta 1,82 bb"].
    pub line: Vec<String>,
    pub street: Street,
    pub board: Vec<String>,
    pub player: Seat,
    pub position: String,
    /// Pot before the current decision (starting pot + all bets so far), chips.
    pub pot_chips: i32,
    /// Amount the current player must call, chips.
    pub to_call_chips: i32,
    /// Stack remaining for the current player before the decision, chips.
    pub stack_chips: i32,
    pub actions: Vec<ActionInfo>,
    /// Eligible combos for the current player.
    pub combos: Vec<String>,
    /// Normalized weight of each combo at this node (reach-weighted number of combos).
    pub weights: Vec<f32>,
    /// `strategy[action][combo]`, sums to 1 over actions for each combo.
    pub strategy: Vec<Vec<f32>>,
    /// `ev[action][combo]`, chips. Solver convention: fold = 0 and every action EV of the same
    /// node shares the same baseline, so differences between actions are exact EV losses.
    pub ev: Vec<Vec<f32>>,
    /// Equity of each combo against the opponent's range at this node (0..1).
    pub equity: Vec<f32>,
    /// Probability mass of reaching this node relative to the root (joint, both players).
    /// Nodes reached only through near-zero frequencies are not used as exercises.
    #[serde(default = "one")]
    pub reach: f32,
    /// Runout categories dealt to reach this node, e.g. `["turn:over", "river:low"]`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runout: Vec<String>,
}

fn one() -> f32 {
    1.0
}

impl NodeSolution {
    pub fn combo_index(&self, combo: &Combo) -> Option<usize> {
        let s = combo.to_string();
        self.combos.iter().position(|c| *c == s)
    }

    pub fn num_actions(&self) -> usize {
        self.actions.len()
    }

    /// Range-level frequency of each action (weighted by combo weights).
    pub fn aggregate_frequencies(&self) -> Vec<f64> {
        let total: f64 = self.weights.iter().map(|&w| w as f64).sum();
        self.strategy
            .iter()
            .map(|row| {
                if total <= 0.0 {
                    return 0.0;
                }
                row.iter()
                    .zip(&self.weights)
                    .map(|(&s, &w)| s as f64 * w as f64)
                    .sum::<f64>()
                    / total
            })
            .collect()
    }

    /// Structural checks: dimensions, finite values, normalized strategy.
    pub fn validate(&self, board_mask: u64) -> Result<(), CoreError> {
        let n = self.combos.len();
        let a = self.actions.len();
        let fail = |m: String| {
            Err(CoreError::InvalidSolution(format!(
                "nó {}: {m}",
                self.node_id
            )))
        };
        if a == 0 {
            return fail("sem ações".into());
        }
        if self.weights.len() != n || self.equity.len() != n {
            return fail("tamanho de pesos/equidade diverge dos combos".into());
        }
        if self.strategy.len() != a || self.ev.len() != a {
            return fail("tamanho de estratégia/EV diverge das ações".into());
        }
        for row in self.strategy.iter().chain(self.ev.iter()) {
            if row.len() != n {
                return fail("linha com tamanho errado".into());
            }
            if row.iter().any(|v| !v.is_finite()) {
                return fail("valor não finito".into());
            }
        }
        for (i, c) in self.combos.iter().enumerate() {
            let combo = Combo::parse(c)?;
            if combo.is_blocked_by(board_mask) {
                return fail(format!("combo {c} bloqueado pelo board"));
            }
            if self.weights[i] <= 0.0 {
                return fail(format!("combo {c} com peso nulo exportado"));
            }
            let sum: f32 = (0..a).map(|j| self.strategy[j][i]).sum();
            if (sum - 1.0).abs() > 2e-3 {
                return fail(format!("estratégia de {c} soma {sum}"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolutionFile {
    pub schema_version: u32,
    pub solution_id: String,
    pub config_hash: String,
    pub created_at: String,
    pub engine: EngineInfo,
    pub spot: SpotConfig,
    pub solve: SolveInfo,
    /// Root-level available actions (tree shape check).
    pub root_actions: Vec<ActionInfo>,
    /// Canonical JSON of the export plan used (which nodes and runouts were exported).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub export_plan: Option<String>,
    pub nodes: Vec<NodeSolution>,
}

impl SolutionFile {
    pub fn node(&self, node_id: &str) -> Option<&NodeSolution> {
        self.nodes.iter().find(|n| n.node_id == node_id)
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        if self.schema_version != SOLUTION_SCHEMA_VERSION {
            return Err(CoreError::InvalidSolution(format!(
                "schema {} não suportado (esperado {})",
                self.schema_version, SOLUTION_SCHEMA_VERSION
            )));
        }
        self.spot.validate()?;
        let mask = self.spot.board_mask()?;
        for n in &self.nodes {
            // Turn/river nodes also exclude combos blocked by the dealt runout cards.
            let mut node_mask = mask;
            for c in &n.board {
                node_mask |= 1u64 << crate::cards::parse_card(c)?;
            }
            n.validate(node_mask)?;
        }
        Ok(())
    }

    pub fn to_json_bytes(&self) -> Result<Vec<u8>, CoreError> {
        serde_json::to_vec(self).map_err(|e| CoreError::Io(e.to_string()))
    }

    pub fn from_json_bytes(bytes: &[u8]) -> Result<Self, CoreError> {
        let s: SolutionFile =
            serde_json::from_slice(bytes).map_err(|e| CoreError::InvalidSolution(e.to_string()))?;
        s.validate()?;
        Ok(s)
    }

    /// Saves as JSON (gzip if path ends with `.gz`), atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> Result<(), CoreError> {
        let json = self.to_json_bytes()?;
        let bytes = if path.extension().is_some_and(|e| e == "gz") {
            gzip(&json)?
        } else {
            json
        };
        write_atomic(path, &bytes)
    }

    pub fn load(path: &Path) -> Result<Self, CoreError> {
        let raw =
            std::fs::read(path).map_err(|e| CoreError::Io(format!("{}: {e}", path.display())))?;
        let json = if path.extension().is_some_and(|e| e == "gz") {
            gunzip(&raw)?
        } else {
            raw
        };
        Self::from_json_bytes(&json)
    }
}

pub fn gzip(data: &[u8]) -> Result<Vec<u8>, CoreError> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(data)
        .map_err(|e| CoreError::Io(e.to_string()))?;
    enc.finish().map_err(|e| CoreError::Io(e.to_string()))
}

pub fn gunzip(data: &[u8]) -> Result<Vec<u8>, CoreError> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(data)
        .read_to_end(&mut out)
        .map_err(|e| CoreError::Io(e.to_string()))?;
    Ok(out)
}

/// Writes to `<path>.tmp` then renames over `path`, so readers never see a partial file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), CoreError> {
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            std::fs::create_dir_all(dir).map_err(|e| CoreError::Io(e.to_string()))?;
        }
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension().and_then(|e| e.to_str()).unwrap_or("")
    ));
    {
        let mut f = std::fs::File::create(&tmp).map_err(|e| CoreError::Io(e.to_string()))?;
        f.write_all(bytes)
            .map_err(|e| CoreError::Io(e.to_string()))?;
        f.sync_all().map_err(|e| CoreError::Io(e.to_string()))?;
    }
    std::fs::rename(&tmp, path).map_err(|e| CoreError::Io(e.to_string()))
}

/// Rounds for compact storage. Frequencies to 1e-5, EVs to 1e-3 chips.
pub fn round_freq(v: f32) -> f32 {
    (v * 100_000.0).round() / 100_000.0
}
pub fn round_ev(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}
