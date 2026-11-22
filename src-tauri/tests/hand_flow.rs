//! Full-hand flow of the app layer with the real engine: background solve (with a superseded
//! request cancelled), hands dealt and played to the end, decisions and results persisted once,
//! history after reopening the database. Uses the small turn PoC spot so it runs in seconds.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use poker_core::spot::SpotConfig;
use pokerlab_lib::db::Db;
use pokerlab_lib::handplay::{
    export_csv, history, review, BoardSource, EngineStatus, HandEngine, ReviewMatch,
};

fn source(name: &str, id: &str) -> BoardSource {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../data/spots/{name}"));
    let spot = SpotConfig::from_toml(&std::fs::read_to_string(p).unwrap()).unwrap();
    BoardSource {
        solution_id: id.into(),
        pack_config_hash: spot.config_hash(solver_adapter::ENGINE_REV),
        spot,
        pack_exploitability_pct: 0.5,
        pack_id: "test".into(),
        pack_version: "0".into(),
    }
}

/// Stand-in for the pack lookup: no equivalent for raises, otherwise exact or same theme
/// depending on the node (deterministic, so both kinds show up).
fn fake_lookup(exercise_id: &str, combo: &str, theme: &str) -> ReviewMatch {
    assert!(exercise_id.starts_with("turn:"), "{exercise_id}");
    assert_eq!(combo.len(), 4);
    if theme.ends_with("_vs_raise") {
        ReviewMatch::None
    } else if exercise_id.len().is_multiple_of(2) {
        ReviewMatch::Exact
    } else {
        ReviewMatch::Theme
    }
}

fn wait_ready(e: &HandEngine) -> EngineStatus {
    let t = Instant::now();
    loop {
        let s = e.status().unwrap();
        match &s {
            EngineStatus::Ready { .. } | EngineStatus::Failed { .. } => return s,
            _ if t.elapsed() > Duration::from_secs(300) => panic!("solve demorou demais: {s:?}"),
            _ => std::thread::sleep(Duration::from_millis(50)),
        }
    }
}

#[test]
fn hands_are_played_against_the_engine_and_persisted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pokerlab.sqlite3");
    let engine = HandEngine::new(42);
    assert!(
        engine.start(&Db::open(&path).unwrap(), "BB").is_err(),
        "sem board preparado"
    );

    // A second prepare supersedes the first; only the last board becomes ready.
    engine.prepare(source("river_poc.toml", "river")).unwrap();
    engine.prepare(source("turn_poc.toml", "turn")).unwrap();
    match wait_ready(&engine) {
        EngineStatus::Ready {
            solution_id,
            same_config_as_pack,
            exploitability_pct,
            ..
        } => {
            assert_eq!(solution_id, "turn");
            assert!(same_config_as_pack);
            assert!(exploitability_pct <= 0.5 + 1e-9);
        }
        s => panic!("{s:?}"),
    }

    let mut finished = Vec::new();
    {
        let db = Db::open(&path).unwrap();
        for (i, hero) in ["BB", "BTN", "random", "BB", "BTN", "random"]
            .iter()
            .enumerate()
        {
            let mut v = engine.start(&db, hero).unwrap();
            if *hero != "random" {
                assert_eq!(&v.hero_position, hero);
            }
            assert!(v.villain_cards.is_none(), "cartas do adversário ocultas");
            let json = serde_json::to_string(&v).unwrap();
            assert!(!json.contains("\"strategy\""), "estratégia exposta");
            let mut guard = 0;
            while !v.finished {
                assert!(!v.actions.is_empty(), "vez do aluno sem ações");
                // Alternate: first action, last action (exercise folds, checks and aggression).
                let a = if (i + guard) % 2 == 0 {
                    0
                } else {
                    v.actions.len() - 1
                };
                v = engine
                    .act(&db, &v.token, a, Some(500), &fake_lookup)
                    .unwrap();
                guard += 1;
                assert!(guard < 20);
            }
            assert!(
                engine.act(&db, &v.token, 0, None, &fake_lookup).is_err(),
                "mão já encerrada"
            );
            let o = v.outcome.clone().unwrap();
            assert!(v.villain_cards.is_some());
            if o.showdown {
                assert_eq!(v.board.len(), 5);
                assert!(o.hero_hand.is_some() && o.villain_hand.is_some());
            }
            for d in &v.decisions {
                assert!(d.feedback.is_some() != d.not_evaluable.is_some());
                // Inaccuracies and mistakes (and only those) carry a review outcome.
                let bad = d
                    .feedback
                    .as_ref()
                    .is_some_and(|f| matches!(f.grade.as_str(), "inaccuracy" | "mistake"));
                assert_eq!(d.review.is_some(), bad);
                if let Some(f) = &d.feedback {
                    assert!(f.loss_bb >= 0.0 && !d.explanation.is_empty());
                }
            }
            // Same view again from the engine (navigation away and back).
            let again = engine.view(&v.token).unwrap();
            assert_eq!(again.decisions.len(), v.decisions.len());
            finished.push(v);
        }
    }

    // Reopen: every hand ended once, with its decisions.
    let db = Db::open(&path).unwrap();
    let h = history(&db, 50).unwrap();
    assert_eq!(h.stats.hands, 6);
    assert_eq!(h.stats.finished, 6);
    let decisions: usize = finished.iter().map(|v| v.decisions.len()).sum();
    assert_eq!(h.stats.decisions as usize, decisions);
    let rows: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM hand_decisions", [], |r| r.get(0))
        .unwrap();
    assert_eq!(rows as usize, decisions);
    let net: f64 = finished
        .iter()
        .map(|v| v.outcome.as_ref().unwrap().hero_net_bb)
        .sum();
    assert!((h.stats.net_bb - net).abs() < 1e-9);
    let loss: f64 = finished.iter().map(|v| v.summary.total_loss_bb).sum();
    assert!((h.stats.total_loss_bb - loss).abs() < 1e-9);
    assert!(h
        .recent
        .iter()
        .all(|r| r.ended_at.is_some() && r.line.is_some()));

    // Spaced review: one item per queued decision, linked to it, with its theme and the hero hand.
    let queued: usize = finished.iter().map(|v| v.summary.review_queued).sum();
    let bad: usize = finished
        .iter()
        .flat_map(|v| &v.decisions)
        .filter(|d| d.review.is_some())
        .count();
    assert!(bad > 0, "nenhuma imprecisão/erro nas mãos do teste");
    let items: Vec<(String, String, String, String, String, String)> = db
        .conn
        .prepare(
            "SELECT r.exercise_id, r.combo, r.theme, d.node_id, d.theme, h.hero_combo
             FROM reviews r JOIN hand_decisions d ON d.id = r.hand_decision_id
             JOIN hands h ON h.id = d.hand_id WHERE r.attempt_id IS NULL",
        )
        .unwrap()
        .query_map([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(items.len(), queued);
    for (ex, combo, theme, node, d_theme, hero) in &items {
        assert_eq!(ex, &format!("turn:{node}"));
        assert_eq!(theme, d_theme);
        assert_eq!(combo, hero);
        assert_ne!(fake_lookup(ex, combo, theme), ReviewMatch::None);
    }
    let untagged: i64 = db
        .conn
        .query_row(
            "SELECT COUNT(*) FROM hand_decisions WHERE theme IS NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(untagged, 0, "toda decisão tem tema");

    // CSV export: one row per decision, plus one row for a hand interrupted before any decision.
    let interrupted = engine.start(&db, "BB").unwrap();
    assert!(!interrupted.finished);
    let out = dir.path().join("exports").join("maos.csv");
    let (hands, rows) = export_csv(&db, &out).unwrap();
    assert_eq!(hands, 7);
    assert_eq!(rows, decisions + 1);
    let text = std::fs::read_to_string(&out).unwrap();
    let table: Vec<Vec<String>> = text.lines().map(csv_fields).collect();
    let header = &table[0];
    assert_eq!(table.len(), rows + 1);
    assert!(
        table.iter().all(|r| r.len() == header.len()),
        "colunas desalinhadas"
    );
    let col = |name: &str| header.iter().position(|h| h == name).unwrap();
    let (hand_id, seq, ended, grade, loss) = (
        col("hand_id"),
        col("decision_seq"),
        col("ended_at"),
        col("grade"),
        col("loss_bb"),
    );
    let last = table.last().unwrap();
    assert!(
        last[ended].is_empty() && last[seq].is_empty(),
        "mão interrompida sem decisão"
    );
    // Each finished hand's rows carry its decisions in order and its total loss.
    for v in &finished {
        let id = db
            .conn
            .query_row("SELECT id FROM hands WHERE token = ?1", [&v.token], |r| {
                r.get::<_, i64>(0)
            })
            .unwrap()
            .to_string();
        let mine: Vec<&Vec<String>> = table[1..].iter().filter(|r| r[hand_id] == id).collect();
        assert_eq!(mine.len(), v.decisions.len());
        for (i, r) in mine.iter().enumerate() {
            assert_eq!(r[seq], i.to_string());
            assert!(!r[ended].is_empty());
            assert_eq!(r[grade].is_empty(), v.decisions[i].feedback.is_none());
        }
        let sum: f64 = mine
            .iter()
            .filter_map(|r| r[loss].parse::<f64>().ok())
            .sum();
        assert!((sum - v.summary.total_loss_bb).abs() < 1e-6);
    }

    // Review of past hands from the database (after reopening, engine not needed).
    let cpb = |_: &str| Some(1.0);
    let id_of = |token: &str| -> i64 {
        db.conn
            .query_row("SELECT id FROM hands WHERE token = ?1", [token], |r| {
                r.get(0)
            })
            .unwrap()
    };
    for v in &finished {
        let rv = review(&db, id_of(&v.token), cpb).unwrap();
        assert!(rv.row.ended_at.is_some());
        let snap = rv.view.expect("instantâneo gravado");
        // The snapshot is exactly the final view the student saw.
        let shown: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(v).unwrap()).unwrap();
        assert_eq!(snap, shown);
        assert_eq!(rv.decisions.len(), v.decisions.len());
        for (s, d) in rv.decisions.iter().zip(&v.decisions) {
            assert_eq!(s.chosen_label, d.chosen_label);
            assert_eq!(s.grade.is_some(), d.feedback.is_some());
            assert!(s.pot_bb.is_some());
        }
    }
    let rv = review(&db, id_of(&interrupted.token), cpb).unwrap();
    assert!(rv.row.ended_at.is_none() && rv.decisions.is_empty());
    assert_eq!(rv.view.unwrap()["finished"], false);
    // Hands played before snapshots existed: stored decisions only; unknown spot = no bb amounts.
    let legacy = &finished[0];
    db.conn
        .execute(
            "UPDATE hands SET view_json = NULL WHERE token = ?1",
            [&legacy.token],
        )
        .unwrap();
    let rv = review(&db, id_of(&legacy.token), |_| None).unwrap();
    assert!(rv.view.is_none());
    assert_eq!(rv.decisions.len(), legacy.decisions.len());
    assert!(rv.decisions.iter().all(|d| d.pot_bb.is_none()));
    assert!(review(&db, 9999, cpb).is_err());
}

/// Minimal RFC 4180 field splitter for one line (no embedded newlines in this export).
fn csv_fields(line: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                chars.next();
                out.last_mut().unwrap().push('"');
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(String::new()),
            c => out.last_mut().unwrap().push(c),
        }
    }
    out
}
