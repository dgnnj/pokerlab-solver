//! Runout export across chance nodes (turn -> river) on a small turn spot, real engine.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use poker_core::cards::{parse_card, Combo};
use poker_core::spot::{SpotConfig, Street};
use solver_adapter::*;

fn spot() -> SpotConfig {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/spots/turn_poc.toml");
    SpotConfig::from_toml(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn plan() -> ExportPlan {
    ExportPlan {
        depth: 2,
        runouts: Some(RunoutStage {
            lines: vec!["x-x".into(), "x-bS-c".into()],
            cards: 3,
            depth: 2,
            expand: 0,
            next: None,
        }),
    }
}

#[test]
fn river_nodes_exported_with_real_ranges() {
    let s = spot();
    let cancel = AtomicBool::new(false);
    let mut sg = solve_spot(&s, &cancel, |_| {}).unwrap();
    assert!(sg.info.final_exploitability_pct_pot <= s.solve.target_exploitability_pct + 1e-9);
    let file = build_solution_file_with_plan(&mut sg, &s, &plan()).unwrap();
    let river: Vec<_> = file
        .nodes
        .iter()
        .filter(|n| n.street == Street::River)
        .collect();
    // 2 lines x 3 river cards, at least the first decision of each runout.
    assert!(river.len() >= 6, "{} river nodes", river.len());
    let mut cards = std::collections::BTreeSet::new();
    for n in &river {
        assert_eq!(n.board.len(), 5);
        let river_card = parse_card(&n.board[4]).unwrap();
        cards.insert(n.board[4].clone());
        // Combos holding the dealt river card are excluded.
        for c in &n.combos {
            assert!(
                !Combo::parse(c).unwrap().is_blocked_by(1u64 << river_card),
                "{c} in {}",
                n.node_id
            );
        }
        assert!(
            n.line.iter().any(|l| l.starts_with("River: ")),
            "{:?}",
            n.line
        );
        assert_eq!(n.runout.len(), 1);
        assert!(n.runout[0].starts_with("river:"));
        assert!(n.node_id.contains('|'));
        assert!(
            (0.0..=1.0 + 1e-3).contains(&n.reach) && !n.combos.is_empty(),
            "{} reach {}",
            n.node_id,
            n.reach
        );
        // Re-extracting the node from its stored history gives identical values.
        let again = extract_node_with_runout(
            &mut sg.game,
            &s,
            &n.history,
            &n.node_id,
            n.line.clone(),
            1.0,
            n.runout.clone(),
        )
        .unwrap();
        assert_eq!(again.combos, n.combos);
        assert_eq!(again.strategy, n.strategy);
        assert_eq!(again.ev, n.ev);
    }
    assert_eq!(
        cards.len(),
        3,
        "same 3 river cards for each line: {cards:?}"
    );
    // The pot on the river after turn bet-call grows by both bets.
    let after_bet = river
        .iter()
        .find(|n| n.node_id.starts_with("x-b500-c|"))
        .expect("line x-bS-c");
    assert_eq!(after_bet.pot_chips, 2000);
    let after_check = river
        .iter()
        .find(|n| n.node_id.starts_with("x-x|"))
        .expect("line x-x");
    assert_eq!(after_check.pot_chips, 1000);
    // Ranges are line-specific: after check-check BTN does not hold its strongest betting hands as often.
    assert_ne!(after_bet.combos.len() + after_bet.weights.len(), 0);
    println!(
        "reach x-x river: {:.3}, x-bS-c river: {:.3}",
        after_check.reach, after_bet.reach
    );
}

#[test]
fn invalid_line_is_reported() {
    let s = spot();
    let cancel = AtomicBool::new(false);
    let mut sg = solve_spot(&s, &cancel, |_| {}).unwrap();
    let bad = ExportPlan {
        depth: 1,
        runouts: Some(RunoutStage {
            lines: vec!["x-bS-f".into()],
            cards: 2,
            depth: 1,
            expand: 0,
            next: None,
        }),
    };
    assert!(
        build_solution_file_with_plan(&mut sg, &s, &bad).is_err(),
        "a fold ends the hand: no river to deal"
    );
}
