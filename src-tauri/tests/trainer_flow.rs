//! End-to-end trainer flow on the real bundled packs (no mocks): install + verify packs, present a
//! decision with hidden strategy, grade it with real EVs, persist once, reopen and find it again.

use std::path::PathBuf;

use poker_core::pack::ExerciseDef;
use poker_core::solution::NodeSolution;
use pokerlab_lib::db::Db;
use pokerlab_lib::handplay::ReviewMatch;
use pokerlab_lib::trainer::{install_pack, LoadedPack, Mode, ProgressFilter, Trainer};

fn packs_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources/packs")
}

/// The first bundled pack (BTN x BB), used by the single-pack checks.
fn bundled() -> PathBuf {
    packs_root().join(pokerlab_lib::BUNDLED_PACKS[0])
}

fn open(dir: &std::path::Path, seed: u64) -> Trainer {
    let db = Db::open(&dir.join("pokerlab.sqlite3")).unwrap();
    let packs = pokerlab_lib::bundled_pack_dirs(&packs_root())
        .unwrap()
        .iter()
        .map(|d| install_pack(&db, d, &dir.join("packs")).unwrap())
        .collect();
    Trainer::new(db, packs, seed)
}

fn main_pack(t: &Trainer) -> &LoadedPack {
    &t.packs.list[0]
}

/// Exercise and node of a presented exercise, whatever pack it came from.
fn exercise_node(t: &Trainer, id: &str) -> (ExerciseDef, NodeSolution) {
    let (p, e) = t.packs.exercise(id).unwrap();
    (e.clone(), p.node(e).unwrap().1.clone())
}

#[test]
fn full_flow_persists_across_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let (attempt_id, combo, chosen_label, loss);
    {
        let mut t = open(dir.path(), 1);
        assert!(
            main_pack(&t).manifest.exercises.len() >= 12,
            "pack has exercises"
        );
        let s = t
            .start_session(Mode::Representative, None, None, 3)
            .unwrap();
        let ex = t.next_exercise(s.id).unwrap().expect("exercise");
        // Strategy is hidden: the presented exercise only carries labels.
        let json = serde_json::to_string(&ex).unwrap();
        assert!(
            !json.contains("strategy") && !json.contains("\"ev"),
            "strategy leaked: {json}"
        );
        let fb = t.submit(&ex.token, 0, Some(1234)).unwrap();
        // Recompute from the pack: loss = max EV - chosen EV for this combo at this node.
        let label0 = {
            let (_, node) = exercise_node(&t, &ex.exercise_id);
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
    let s = t
        .start_session(Mode::Representative, None, None, 40)
        .unwrap();
    // Play until we produce a non-best answer by always choosing the worst-EV action.
    let mut wrong = None;
    for _ in 0..40 {
        let ex = t.next_exercise(s.id).unwrap().unwrap();
        let worst = {
            let (_, node) = exercise_node(&t, &ex.exercise_id);
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
    let r = t.start_session(Mode::Review, None, None, 1).unwrap();
    let ex = t.next_exercise(r.id).unwrap().expect("review item");
    assert_eq!(ex.exercise_id, ex_id);
    assert_eq!(format!("{}{}", ex.hero_cards[0], ex.hero_cards[1]), combo);
    assert_eq!(ex.mode, "review");
    // Answer correctly: the item is rescheduled into the future.
    let best = {
        let (_, node) = exercise_node(&t, &ex.exercise_id);
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
    let first = main_pack(&t).manifest.exercises[0].id.clone();
    let v = t.study(&first, None).unwrap();
    assert_eq!(v.grid.len(), 169);
    let filled: f64 = v.grid.iter().map(|c| c.weight).sum();
    assert!(filled > 0.0);
    for c in v.grid.iter().filter(|c| c.weight > 0.0) {
        let s: f64 = c.strategy.iter().sum();
        assert!((s - 1.0).abs() < 1e-3, "{} sums {s}", c.hand);
    }
    let s = t.start_session(Mode::Hard, None, None, 2).unwrap();
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
        main_pack(&t).dir.clone()
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
    let p = main_pack(&t);
    assert_eq!(p.solutions.len(), p.manifest.files.len());
    let m = poker_core::pack::PackManifest::load(&p.dir).unwrap();
    m.verify_and_load(&p.dir).unwrap();
}

#[test]
fn turn_and_river_drills_use_dealt_board_and_line() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = open(dir.path(), 11);
    for (street, cards, marker) in [("turn", 4, "Turn: "), ("river", 5, "River: ")] {
        let s = t
            .start_session(
                Mode::Representative,
                None,
                Some(format!("street:{street}")),
                3,
            )
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
        .start_session(Mode::Representative, None, Some("street:preflop".into()), 1)
        .is_err());
}

#[test]
fn rebuilt_pack_with_same_version_replaces_installed_copy() {
    let dir = tempfile::tempdir().unwrap();
    let installed = {
        let t = open(dir.path(), 21);
        main_pack(&t).dir.clone()
    };
    // Simulate an older build of the same version: a different (but internally valid) manifest.
    let mpath = installed.join("manifest.json");
    let mut m: serde_json::Value = serde_json::from_slice(&std::fs::read(&mpath).unwrap()).unwrap();
    m["description"] = serde_json::Value::String("build antigo".into());
    std::fs::write(&mpath, serde_json::to_vec(&m).unwrap()).unwrap();
    let t = open(dir.path(), 22);
    assert_ne!(main_pack(&t).manifest.description, "build antigo");
    let bundled = std::fs::read(bundled().join("manifest.json")).unwrap();
    assert_eq!(
        std::fs::read(main_pack(&t).dir.join("manifest.json")).unwrap(),
        bundled
    );
}

/// The app classifies hand decisions with the same function the pack generator used: every
/// exercise of every bundled pack gets its own theme back from its node.
#[test]
fn shared_theme_classifier_matches_pack_exercises() {
    let dir = tempfile::tempdir().unwrap();
    let t = open(dir.path(), 1);
    for (p, ex) in t
        .packs
        .list
        .iter()
        .flat_map(|p| p.manifest.exercises.iter().map(move |e| (p, e)))
    {
        let (sol, node) = p.node(ex).unwrap();
        let th = poker_core::theme::classify(
            node.street,
            &node.line,
            node.to_call_chips,
            &node.position,
            &sol.spot,
        );
        assert_eq!(th.key, ex.theme, "{}", ex.id);
        assert_eq!(th.label, ex.theme_label, "{}", ex.id);
    }
}

/// Inserts a played hand with one decision and its review item, as the hand engine does.
fn queue_hand_mistake(t: &Trainer, exercise_id: &str, combo: &str, theme: &str) -> i64 {
    let c = &t.db.conn;
    c.execute(
        "INSERT INTO hands(token, started_at, ended_at, solution_id, config_hash, same_config_as_pack, pack_id,
            pack_version, engine_rev, solve_exploitability_pct, hero_position, villain_position, hero_combo,
            villain_combo, flop)
         VALUES (?1, 'x', 'y', 's', 'h', 1, 'p', '1', 'e', 0.4, 'BTN', 'BB', ?2, 'QdQc', 'Ah7c2d')",
        [format!("h-{exercise_id}-{combo}"), combo.to_string()],
    )
    .unwrap();
    let hand = c.last_insert_rowid();
    c.execute(
        "INSERT INTO hand_decisions(hand_id, seq, created_at, node_id, street, board, pot_chips, chosen_index,
            chosen_label, grade, decision_ms, theme)
         VALUES (?1, 0, 'x', 'n', 'flop', 'Ah7c2d', 500, 0, 'Passa', 'mistake', 800, ?2)",
        rusqlite::params![hand, theme],
    )
    .unwrap();
    let d = c.last_insert_rowid();
    c.execute(
        "INSERT INTO reviews(hand_decision_id, exercise_id, combo, theme, next_review_at, created_at)
         VALUES (?1, ?2, ?3, ?4, '2000-01-01T00:00:00Z', '2000-01-01T00:00:00Z')",
        rusqlite::params![d, exercise_id, combo, theme],
    )
    .unwrap();
    c.last_insert_rowid()
}

#[test]
fn hand_mistakes_are_reviewed_with_pack_exercises() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = open(dir.path(), 3);
    let ex = main_pack(&t).manifest.exercises[0].clone();
    let combo = {
        let (_, node) = exercise_node(&t, &ex.id);
        node.combos[0].clone()
    };
    // What the pack can offer for a decision made in a full hand.
    assert_eq!(
        t.review_match(&ex.id, &combo, &ex.theme),
        ReviewMatch::Exact
    );
    let off_pack = format!("{}:x-x|2c|x-b999", ex.solution_id);
    assert_eq!(
        t.review_match(&off_pack, &combo, &ex.theme),
        ReviewMatch::Theme
    );
    assert_eq!(
        t.review_match(&off_pack, &combo, "srp_flop_vs_lead_raise"),
        ReviewMatch::None,
        "o pacote não tem exercícios desse tema"
    );

    // Exact item: same situation, same hand; a good answer schedules it for later.
    let rid = queue_hand_mistake(&t, &ex.id, &combo, &ex.theme);
    assert_eq!(t.review_due_count().unwrap(), 1);
    let s = t.start_session(Mode::Review, None, None, 2).unwrap();
    let p = t.next_exercise(s.id).unwrap().expect("item da mão");
    assert_eq!(p.exercise_id, ex.id);
    assert_eq!(format!("{}{}", p.hero_cards[0], p.hero_cards[1]), combo);
    assert_eq!(
        p.review_note.as_deref(),
        Some("Revisão de erro em mão completa: mesma situação e mesma mão")
    );
    let best = {
        let (_, node) = exercise_node(&t, &ex.id);
        (0..node.actions.len())
            .max_by(|&a, &b| node.ev[a][0].partial_cmp(&node.ev[b][0]).unwrap())
            .unwrap()
    };
    let fb = t.submit(&p.token, best, None).unwrap();
    assert!(fb.in_review);
    let (reps, attempt): (i64, Option<i64>) =
        t.db.conn
            .query_row(
                "SELECT repetitions, attempt_id FROM reviews WHERE id = ?1",
                [rid],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
    assert_eq!((reps, attempt), (1, None), "o item continua ligado à mão");
    assert_eq!(t.review_due_count().unwrap(), 0);

    // Off-pack node: another exercise of the same theme.
    queue_hand_mistake(&t, &off_pack, &combo, &ex.theme);
    let p = t.next_exercise(s.id).unwrap().expect("item por tema");
    assert_eq!(t.packs.exercise(&p.exercise_id).unwrap().1.theme, ex.theme);
    assert_eq!(
        p.pack_id,
        main_pack(&t).manifest.pack_id,
        "fica no pacote da mão"
    );
    assert_eq!(
        p.review_note.as_deref(),
        Some("Revisão de erro em mão completa: mesmo tema, outra mão")
    );
}

/// Several packs: a session of one pack only draws from it, progress keeps the themes of each
/// pack apart, and a review item of an older pack version falls back inside the same spot.
#[test]
fn sessions_and_reviews_stay_within_their_pack() {
    let dir = tempfile::tempdir().unwrap();
    let mut t = open(dir.path(), 9);
    let ids: Vec<String> = t
        .packs
        .list
        .iter()
        .map(|p| p.manifest.pack_id.clone())
        .collect();
    assert_eq!(ids, pokerlab_lib::BUNDLED_PACKS);
    for id in &ids {
        let s = t
            .start_session(Mode::Representative, Some(id.clone()), None, 3)
            .unwrap();
        assert_eq!(s.pack_id.as_deref(), Some(id.as_str()));
        for _ in 0..3 {
            let ex = t.next_exercise(s.id).unwrap().unwrap();
            assert_eq!(&ex.pack_id, id);
            assert_eq!(
                &t.packs
                    .exercise(&ex.exercise_id)
                    .unwrap()
                    .0
                    .manifest
                    .pack_id,
                id
            );
            t.submit(&ex.token, 0, None).unwrap();
        }
    }
    // Theme keys repeat across packs: one progress group per pack and theme, named with the pack.
    let rep = t.progress(&ProgressFilter::default()).unwrap();
    for g in &rep.groups {
        let (pack, _) = g.key.split_once('|').expect("pack|theme");
        assert!(ids.iter().any(|i| i == pack), "{}", g.key);
        assert!(g.label.contains(" · "), "{}", g.label);
    }
    let co = t
        .progress(&ProgressFilter {
            pack_id: Some(ids[1].clone()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(co.total.n, 3);
    assert!(t
        .start_session(Mode::Representative, Some("nao-existe".into()), None, 1)
        .is_err());

    // Review item from an older version of BTN x BB (solution id `-v1`): the theme fallback
    // stays in BTN x BB, although CO x BB has the same theme key.
    let main_id = main_pack(&t).manifest.pack_id.clone();
    let ex = main_pack(&t)
        .manifest
        .exercises
        .iter()
        .find(|e| e.theme.starts_with("srp_flop"))
        .unwrap()
        .clone();
    let old_sid = ex.solution_id.replace("-v2", "-v1");
    assert_ne!(old_sid, ex.solution_id);
    let old_id = format!("{old_sid}:{}", ex.node_id);
    assert_eq!(
        t.review_match(&old_id, "AsKs", &ex.theme),
        ReviewMatch::Theme
    );
    assert!(
        t.packs
            .exercises(Some(&ids[1]))
            .any(|e| e.theme == ex.theme),
        "tema também existe no CO x BB"
    );
    queue_hand_mistake(&t, &old_id, "AsKs", &ex.theme);
    let r = t
        .start_session(Mode::Review, Some(ids[1].clone()), None, 1)
        .unwrap();
    assert_eq!(r.pack_id, None, "a revisão usa a fila inteira");
    let p = t.next_exercise(r.id).unwrap().unwrap();
    assert_eq!(
        p.pack_id, main_id,
        "{} {:?} {old_id}",
        p.exercise_id, p.review_note
    );
}
