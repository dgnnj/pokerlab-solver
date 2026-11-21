//! End-to-end trainer flow on the real bundled pack (no mocks): install + verify pack, present a
//! decision with hidden strategy, grade it with real EVs, persist once, reopen and find it again.

use std::path::PathBuf;

use pokerlab_lib::db::Db;
use pokerlab_lib::trainer::{install_pack, Mode, ProgressFilter, Trainer};

fn bundled() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/packs/btn-vs-bb-srp-100bb")
}

fn open(dir: &std::path::Path, seed: u64) -> Trainer {
    let db = Db::open(&dir.join("pokerlab.sqlite3")).unwrap();
    let pack = install_pack(&db, &bundled(), &dir.join("packs")).unwrap();
    Trainer::new(db, pack, seed)
}

#[test]
fn full_flow_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let (attempt_id, combo, chosen_label, loss);
    {
        let mut t = open(dir.path(), 1);
        assert!(t.pack.manifest.exercises.len() >= 12, "pack has exercises");
        let s = t.start_session(Mode::Representative, None, 3).unwrap();
        let ex = t.next_exercise(s.id).unwrap().expect("exercise");
        // Strategy is hidden: the presented exercise only carries labels.
        let json = serde_json::to_string(&ex).unwrap();
        assert!(
            !json.contains("strategy") && !json.contains("\"ev"),
            "strategy leaked: {json}"
        );
        let fb = t.submit(&ex.token, 0, Some(1234)).unwrap();
        // Recompute from the pack: loss = max EV - chosen EV for this combo at this node.
        let e = t.pack.exercise(&ex.exercise_id).unwrap().clone();
        let label0 = {
            let (_, node) = t.pack.node(&e).unwrap();
            let ci = node
                .combo_index(&poker_core::cards::Combo::parse(&fb.feedback.combo).unwrap())
                .unwrap();
            let best = (0..node.actions.len())
                .map(|a| node.ev[a][ci])
                .fold(f32::MIN, f32::max) as f64;
            assert!((fb.feedback.loss_chips - (best - node.ev[0][ci] as f64)).abs() < 1e-3);
            node.actions[0].label.clone()
        };
        // A second submission of the same exercise is rejected (recorded once).
        assert!(t.submit(&ex.token, 1, None).is_err());
        attempt_id = fb.attempt_id;
        combo = fb.feedback.combo.clone();
        chosen_label = label0;
        loss = fb.feedback.loss_bb;
        assert_eq!(t.session(s.id).unwrap().done, 1);
    }
    // "Close and reopen": new connection, pack re-verified from the installed copy.
    let t = open(dir.path(), 2);
    let h = t.history(10).unwrap();
    assert_eq!(h.len(), 1);
    assert_eq!(h[0].id, attempt_id);
    assert_eq!(h[0].combo, combo);
    assert_eq!(h[0].chosen_label, chosen_label);
    assert!((h[0].loss_bb - loss).abs() < 1e-9);
    assert_eq!(h[0].decision_ms, 1234);
    // Open session can be continued.
    let open_s = t.last_open_session().unwrap().expect("open session");
    assert_eq!(open_s.done, 1);
}

#[test]
fn mistakes_enter_review_and_review_repeats_same_hand() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = open(dir.path(), 7);
    let s = t.start_session(Mode::Representative, None, 40).unwrap();
    // Play until we produce a non-best answer by always choosing the worst-EV action.
    let mut wrong = None;
    for _ in 0..40 {
        let ex = t.next_exercise(s.id).unwrap().unwrap();
        let e = t.pack.exercise(&ex.exercise_id).unwrap().clone();
        let worst = {
            let (_, node) = t.pack.node(&e).unwrap();
            let combo = poker_core::cards::Combo::parse(&format!(
                "{}{}",
                ex.hero_cards[0], ex.hero_cards[1]
            ))
            .unwrap();
            let ci = node.combo_index(&combo).unwrap();
            (0..node.actions.len())
                .min_by(|&a, &b| node.ev[a][ci].partial_cmp(&node.ev[b][ci]).unwrap())
                .unwrap()
        };
        let fb = t.submit(&ex.token, worst, None).unwrap();
        if matches!(
            fb.feedback.grade,
            poker_core::eval::Grade::Inaccuracy | poker_core::eval::Grade::Mistake
        ) {
            assert!(fb.in_review);
            wrong = Some((ex.exercise_id.clone(), fb.feedback.combo.clone()));
            break;
        }
    }
    let (ex_id, combo) = wrong.expect("produced a mistake");
    assert!(t.review_due_count().unwrap() >= 1);
    let summary = t.end_session(s.id).unwrap();
    assert!(summary.decisions >= 1);
    let r = t.start_session(Mode::Review, None, 1).unwrap();
    let ex = t.next_exercise(r.id).unwrap().expect("review item");
    assert_eq!(ex.exercise_id, ex_id);
    assert_eq!(format!("{}{}", ex.hero_cards[0], ex.hero_cards[1]), combo);
    assert_eq!(ex.mode, "review");
    // Answer correctly: the item is rescheduled into the future.
    let e = t.pack.exercise(&ex.exercise_id).unwrap().clone();
    let best = {
        let (_, node) = t.pack.node(&e).unwrap();
        let ci = node
            .combo_index(&poker_core::cards::Combo::parse(&combo).unwrap())
            .unwrap();
        (0..node.actions.len())
            .max_by(|&a, &b| node.ev[a][ci].partial_cmp(&node.ev[b][ci]).unwrap())
            .unwrap()
    };
    t.submit(&ex.token, best, None).unwrap();
    assert_eq!(t.review_due_count().unwrap(), 0);
    // Stats keep sampling modes apart.
    let rep = t
        .progress(&ProgressFilter {
            group_by: Some("sampling_mode".into()),
            ..Default::default()
        })
        .unwrap();
    let keys: Vec<_> = rep.groups.iter().map(|g| g.key.as_str()).collect();
    assert!(
        keys.contains(&"representative") && keys.contains(&"review"),
        "{keys:?}"
    );
    let only_rep = t
        .progress(&ProgressFilter {
            mode: Some("review".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(only_rep.total.n, 1);
}

#[test]
fn study_view_and_csv_export() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = open(dir.path(), 3);
    let first = t.pack.manifest.exercises[0].id.clone();
    let v = t.study(&first, None).unwrap();
    assert_eq!(v.grid.len(), 169);
    let filled: f64 = v.grid.iter().map(|c| c.weight).sum();
    assert!(filled > 0.0);
    for c in v.grid.iter().filter(|c| c.weight > 0.0) {
        let s: f64 = c.strategy.iter().sum();
        assert!((s - 1.0).abs() < 1e-3, "{} sums {s}", c.hand);
    }
    let s = t.start_session(Mode::Hard, None, 2).unwrap();
    let ex = t.next_exercise(s.id).unwrap().unwrap();
    t.submit(&ex.token, 0, None).unwrap();
    let out = dir.path().join("h.csv");
    assert_eq!(t.export_csv(&out).unwrap(), 1);
    let text = std::fs::read_to_string(out).unwrap();
    assert!(text.lines().count() == 2 && text.contains(",hard,"));
}

#[test]
fn corrupted_installed_pack_is_restored_from_bundle() {
    let dir = tempfile::tempdir().unwrap();
    let installed = {
        let t = open(dir.path(), 4);
        t.pack.dir.clone()
    };
    let sol = std::fs::read_dir(installed.join("solutions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let mut bytes = std::fs::read(&sol).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 0xFF;
    std::fs::write(&sol, bytes).unwrap();
    // Reopen: integrity check fails on the installed copy, the bundled copy is reinstalled.
    let t = open(dir.path(), 5);
    assert_eq!(t.pack.solutions.len(), t.pack.manifest.files.len());
    let m = poker_core::pack::PackManifest::load(&t.pack.dir).unwrap();
    m.verify_and_load(&t.pack.dir).unwrap();
}

#[test]
fn turn_and_river_drills_use_dealt_board_and_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = open(dir.path(), 11);
    for (street, cards, marker) in [("turn", 4, "Turn: "), ("river", 5, "River: ")] {
        let s = t
            .start_session(Mode::Representative, Some(format!("street:{street}")), 3)
            .unwrap();
        for _ in 0..3 {
            let ex = t.next_exercise(s.id).unwrap().unwrap();
            assert_eq!(ex.street, street);
            assert_eq!(ex.board.len(), cards);
            assert!(
                ex.line.iter().any(|l| l.starts_with(marker)),
                "{:?}",
                ex.line
            );
            // Hero cards never collide with the dealt board.
            for c in &ex.hero_cards {
                assert!(!ex.board.contains(c), "{c} on board {:?}", ex.board);
            }
            let fb = t.submit(&ex.token, 0, None).unwrap();
            assert!(fb.feedback.loss_bb >= 0.0);
        }
    }
    let rep = t
        .progress(&ProgressFilter {
            street: Some("river".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(rep.total.n, 3);
    assert!(t
        .start_session(Mode::Representative, Some("street:preflop".into()), 1)
        .is_err());
}

#[test]
fn rebuilt_pack_with_same_version_replaces_installed_copy() {
    let dir = tempfile::tempdir().unwrap();
    let installed = {
        let t = open(dir.path(), 21);
        t.pack.dir.clone()
    };
    // Simulate an older build of the same version: a different (but internally valid) manifest.
    let mpath = installed.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mpath).unwrap()).unwrap();
    m["description"] = serde_json::Value::String("build antigo".into());
    std::fs::write(&mpath, serde_json::to_vec(&m).unwrap()).unwrap();
    let t = open(dir.path(), 22);
    assert_ne!(t.pack.manifest.description, "build antigo");
    let bundled = std::fs::read(bundled().join("manifest.json")).unwrap();
    assert_eq!(
        std::fs::read(t.pack.dir.join("manifest.json")).unwrap(),
        bundled
    );
}
