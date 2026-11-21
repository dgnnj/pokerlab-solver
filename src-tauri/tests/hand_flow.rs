//! Full-hand flow of the app layer with the real engine: background solve (with a superseded
//! request cancelled), hands dealt and played to the end, decisions and results persisted once,
//! history after reopening the database. Uses the small turn PoC spot so it runs in seconds.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use poker_core::spot::SpotConfig;
use pokerlab_lib::db::Db;
use pokerlab_lib::handplay::{history, BoardSource, EngineStatus, HandEngine};

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
                v = engine.act(&db, &v.token, a, Some(500)).unwrap();
                guard += 1;
                assert!(guard < 20);
            }
            assert!(
                engine.act(&db, &v.token, 0, None).is_err(),
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
}
