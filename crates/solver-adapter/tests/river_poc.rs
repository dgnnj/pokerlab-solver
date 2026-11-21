//! Integration tests against the real engine using the river PoC spot.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use poker_core::cards::Combo;
use poker_core::eval::{evaluate, Grade, Tolerance};
use poker_core::solution::SolutionFile;
use poker_core::spot::SpotConfig;
use solver_adapter::*;

fn spot() -> SpotConfig {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../data/spots/river_poc.toml");
    SpotConfig::from_toml(&std::fs::read_to_string(p).unwrap()).unwrap()
}

fn solved(spot: &SpotConfig) -> SolvedGame {
    let cancel = AtomicBool::new(false);
    solve_spot(spot, &cancel, |_| {}).unwrap()
}

#[test]
fn converges_to_target() {
    let s = spot();
    let sg = solved(&s);
    assert!(sg.info.final_exploitability_pct_pot <= s.solve.target_exploitability_pct);
    assert!(sg.info.final_exploitability_chips >= 0.0);
}

#[test]
fn combo_action_strategy_ev_indexing_matches_engine_aggregate() {
    // The engine's own `expected_values` = sum_a strategy[a][h] * ev_detail[a][h]. If our
    // [action][combo] mapping were wrong, recomputing it from the exported node would fail.
    let s = spot();
    let mut sg = solved(&s);
    let file = build_solution_file(&mut sg, &s, 4).unwrap();
    for node in &file.nodes {
        sg.game.apply_history(&node.history);
        sg.game.cache_normalized_weights();
        let p = sg.game.current_player();
        let ev_total = sg.game.expected_values(p);
        let cards: Vec<String> = sg
            .game
            .private_cards(p)
            .iter()
            .map(|&(a, b)| Combo::new(a, b).unwrap().to_string())
            .collect();
        for (ci, c) in node.combos.iter().enumerate() {
            let engine_idx = cards.iter().position(|x| x == c).unwrap();
            let mut sum_s = 0.0f64;
            let mut recon = 0.0f64;
            for a in 0..node.actions.len() {
                sum_s += node.strategy[a][ci] as f64;
                recon += node.strategy[a][ci] as f64 * node.ev[a][ci] as f64;
                assert!(node.ev[a][ci].is_finite());
            }
            assert!((sum_s - 1.0).abs() < 1e-3, "{c} strategy sums {sum_s}");
            let diff = (recon - ev_total[engine_idx] as f64).abs();
            assert!(
                diff < 0.05,
                "node {} combo {c}: recon {recon} engine {}",
                node.node_id,
                ev_total[engine_idx]
            );
        }
        sg.game.back_to_root();
    }
}

#[test]
fn blocked_combos_are_excluded() {
    let s = spot();
    let mut sg = solved(&s);
    let file = build_solution_file(&mut sg, &s, 2).unwrap();
    let root = file.node("root").unwrap();
    // OOP range has AK and 22; board Ks7d4c2h9s blocks AsKs and every 2h combo.
    assert!(!root.combos.contains(&"AsKs".to_string()));
    assert!(!root.combos.iter().any(|c| c.contains("2h")));
    assert!(root.combos.contains(&"AhKd".to_string()));
    assert!(root.combos.contains(&"2d2c".to_string()));
    let board_mask = s.board_mask().unwrap();
    for n in &file.nodes {
        for c in &n.combos {
            assert!(
                !Combo::parse(c).unwrap().is_blocked_by(board_mask),
                "{c} blocked in {}",
                n.node_id
            );
        }
    }
    // The engine itself returns zero weight for any hand overlapping the board.
    let (cards, raw, _nw) = raw_private_hands(&mut sg.game, &[]);
    for (i, c) in cards.iter().enumerate() {
        if Combo::parse(c).unwrap().is_blocked_by(board_mask) {
            assert_eq!(raw[i], 0.0, "{c}");
        }
    }
}

#[test]
fn save_reload_preserves_decision_values() {
    let s = spot();
    let mut sg = solved(&s);
    let file = build_solution_file(&mut sg, &s, 4).unwrap();
    let dir = std::env::temp_dir().join(format!("pokerlab-test-{}", std::process::id()));
    let json = dir.join("river.json");
    let gz = dir.join("river.json.gz");
    file.save(&json).unwrap();
    file.save(&gz).unwrap();
    let a = SolutionFile::load(&json).unwrap();
    let b = SolutionFile::load(&gz).unwrap();
    assert_eq!(a, file);
    assert_eq!(b, file);
    let combo = Combo::parse("AhKd").unwrap();
    let root = a.node("root").unwrap();
    let probe = probe_live(
        &mut sg.game,
        &s,
        &a.solution_id,
        &root.history,
        "root",
        &combo,
    )
    .unwrap();
    let ci = root.combo_index(&combo).unwrap();
    for j in 0..root.actions.len() {
        assert!((root.strategy[j][ci] - probe.frequencies[j]).abs() < 1e-4);
        assert!((root.ev[j][ci] - probe.evs_chips[j]).abs() < 0.01);
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ev_loss_from_real_solution() {
    let s = spot();
    let mut sg = solved(&s);
    let file = build_solution_file(&mut sg, &s, 2).unwrap();
    let node = file.node("x-b1000").expect("BB facing pot bet");
    assert_eq!(node.position, "BB");
    assert_eq!(node.to_call_chips, 1000);
    for (ci, c) in node.combos.iter().enumerate() {
        let combo = Combo::parse(c).unwrap();
        let best = (0..node.actions.len())
            .max_by(|&a, &b| node.ev[a][ci].partial_cmp(&node.ev[b][ci]).unwrap())
            .unwrap();
        for a in 0..node.actions.len() {
            let fb = evaluate(node, &combo, a, s.chips_per_bb, Tolerance::default()).unwrap();
            let expected = (node.ev[best][ci] - node.ev[a][ci]) as f64;
            assert!((fb.loss_chips - expected).abs() < 1e-3);
            if a == best {
                assert_eq!(fb.grade, Grade::Best);
            }
            // Fold is always EV 0 in the engine's convention.
            if node.actions[a].label == "Desistir" {
                assert_eq!(node.ev[a][ci], 0.0);
            }
        }
    }
}

#[test]
fn rake_is_charged_once() {
    // Sum of both players' range EV at the root = pot - expected rake.
    let mut s = spot();
    let total = |s: &SpotConfig| {
        let mut sg = solved(s);
        sg.game.cache_normalized_weights();
        let mut t = 0.0f64;
        for p in 0..2 {
            let ev = sg.game.expected_values(p);
            let w = sg.game.normalized_weights(p);
            let tw: f64 = w.iter().map(|&x| x as f64).sum();
            t += ev
                .iter()
                .zip(w)
                .map(|(&e, &x)| e as f64 * x as f64)
                .sum::<f64>()
                / tw;
        }
        t
    };
    let no_rake = total(&s);
    assert!(
        (no_rake - s.starting_pot_chips as f64).abs() < 1.0,
        "no rake total {no_rake}"
    );
    s.rake.rate = 0.05;
    s.rake.cap_chips = 300.0;
    let raked = total(&s);
    let max_rake = 0.05 * (s.starting_pot_chips + 2 * s.effective_stack_chips) as f64;
    assert!(raked < no_rake - 1.0, "raked {raked} vs {no_rake}");
    assert!(
        no_rake - raked <= max_rake.min(300.0) + 1.0,
        "rake charged more than once: {}",
        no_rake - raked
    );
}

#[test]
fn cancellation_publishes_nothing() {
    let s = spot();
    let cancel = AtomicBool::new(true);
    assert!(matches!(
        solve_spot(&s, &cancel, |_| {}),
        Err(AdapterError::Cancelled)
    ));
}

#[test]
fn tree_shape_and_labels() {
    let s = spot();
    let mut sg = solved(&s);
    let file = build_solution_file(&mut sg, &s, 3).unwrap();
    let labels: Vec<&str> = file.root_actions.iter().map(|a| a.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Passar (check)",
            "Apostar 5,00 bb (50% do pote)",
            "Apostar 10,00 bb (100% do pote)"
        ]
    );
    let n = file.node("b500").unwrap();
    assert_eq!(n.position, "BTN");
    assert_eq!(n.actions[2].label, "Aumentar para 15,00 bb");
    assert_eq!(n.pot_chips, 1500);
}
