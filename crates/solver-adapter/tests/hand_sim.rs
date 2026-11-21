//! Full-hand simulation against the real engine (turn PoC spot: turn + river, 10 bb pot, 40 bb
//! stacks). Covers the cases required by the specification: incompatible hands, check by both
//! positions, raise, fold, all-in with runout, zero-weight lines, villain policy and accounting.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, OnceLock};

use poker_core::cards::{cards_mask, parse_board, parse_card, Combo};
use poker_core::eval::Tolerance;
use poker_core::sampling::Rng;
use poker_core::solution::ActionKind;
use poker_core::spot::{SpotConfig, Street};
use postflop_solver::PostFlopGame;
use solver_adapter::hand::*;
use solver_adapter::*;

fn load(name: &str) -> SpotConfig {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("../../data/spots/{name}"));
    SpotConfig::from_toml(&std::fs::read_to_string(p).unwrap()).unwrap()
}

/// One solved turn game shared by the tests (solving takes a few seconds).
fn shared() -> &'static Mutex<(SpotConfig, PostFlopGame)> {
    static G: OnceLock<Mutex<(SpotConfig, PostFlopGame)>> = OnceLock::new();
    G.get_or_init(|| {
        let s = load("turn_poc.toml");
        let sg = solve_spot(&s, &AtomicBool::new(false), |_| {}).unwrap();
        Mutex::new((s, sg.game))
    })
}

fn lock() -> std::sync::MutexGuard<'static, (SpotConfig, PostFlopGame)> {
    shared().lock().unwrap_or_else(|e| e.into_inner())
}

fn pick(kind: ActionKind) -> impl FnMut(&VillainChoice, &mut Rng) -> usize {
    move |c: &VillainChoice, _: &mut Rng| {
        c.actions
            .iter()
            .position(|a| a.kind == kind)
            .unwrap_or_else(|| passive(c))
    }
}

/// Check if possible, else call (never folds).
fn passive(c: &VillainChoice) -> usize {
    c.actions
        .iter()
        .position(|a| a.kind == ActionKind::Check)
        .or_else(|| c.actions.iter().position(|a| a.kind == ActionKind::Call))
        .unwrap_or(0)
}

fn idx(t: &HeroTurn, kind: ActionKind) -> usize {
    t.actions
        .iter()
        .position(|a| a.kind == kind)
        .unwrap_or_else(|| panic!("{kind:?} indisponível em {}", t.node_id))
}

fn assert_cards_consistent(sim: &HandSim, board: &[String]) {
    let mut seen = 0u64;
    let all: Vec<_> = board
        .iter()
        .map(|c| parse_card(c).unwrap())
        .chain([
            sim.hero_combo.0,
            sim.hero_combo.1,
            sim.villain_combo.0,
            sim.villain_combo.1,
        ])
        .collect();
    for c in all {
        assert_eq!(seen & (1 << c), 0, "carta repetida na mão");
        seen |= 1 << c;
    }
}

#[test]
fn deal_is_joint_weighted_and_conflict_free() {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let board = cards_mask(&parse_board(&s.board).unwrap());
    let mut rng = Rng::new(11);
    let (mut a4s, mut a5s) = (0usize, 0usize);
    for i in 0..20_000 {
        let sim = HandSim::deal(game, i % 2, &mut rng).unwrap();
        let (h, v) = (sim.hero_combo, sim.villain_combo);
        assert!(!h.is_blocked_by(board) && !v.is_blocked_by(board));
        assert_eq!(cards_mask(&[h.0, h.1]) & cards_mask(&[v.0, v.1]), 0);
        if sim.hero == 1 {
            let t = h.to_string();
            // IP range: A5s at weight 1 (4 combos), A4s at 0.5 (3 combos: Ac4c is on the board).
            if ["As4s", "Ah4h", "Ad4d"].contains(&t.as_str()) {
                a4s += 1;
            }
            if ["As5s", "Ah5h", "Ad5d", "Ac5c"].contains(&t.as_str()) {
                a5s += 1;
            }
            assert_ne!(t, "Ac4c");
        }
    }
    let ratio = a4s as f64 / a5s as f64;
    assert!((ratio - 0.375).abs() < 0.08, "A4s/A5s = {ratio}");
}

#[test]
fn incompatible_hands_are_never_dealt() {
    // Fully incompatible ranges: the engine itself refuses the game.
    let mut s = load("river_poc.toml");
    s.oop_range.text = "AhAd".into();
    s.ip_range.text = "AhAc".into();
    let err = build_game(&s).err().expect("ranges sem combinação válida");
    assert!(err.to_string().contains("Valid card assignment"), "{err}");
    // Mostly incompatible: AhAd always conflicts with AhAc, so only KcKd x AhAc is dealt.
    s.oop_range.text = "AhAd,KcKd".into();
    let game = &mut build_game(&s).unwrap();
    let mut rng = Rng::new(1);
    for i in 0..300 {
        let sim = HandSim::deal(game, i % 2, &mut rng).unwrap();
        let (oop, ip) = if sim.hero == 0 {
            (sim.hero_combo, sim.villain_combo)
        } else {
            (sim.villain_combo, sim.hero_combo)
        };
        assert_eq!(oop.to_string(), "KdKc");
        assert_eq!(ip.to_string(), "AhAc");
    }
}

/// Villain raises when it can, otherwise calls or checks; never folds.
fn raiser(c: &VillainChoice, _: &mut Rng) -> usize {
    c.actions
        .iter()
        .position(|a| a.kind == ActionKind::Raise)
        .unwrap_or_else(|| passive(c))
}

/// Hero bets/raises until an all-in is available (or calls the villain's all-in), then shoves.
fn shove(sim: &mut HandSim, game: &mut PostFlopGame, s: &SpotConfig, rng: &mut Rng) -> Outcome {
    let tol = Tolerance::default();
    let mut villain = raiser;
    let mut t = sim
        .advance(game, s, rng, &mut villain)
        .unwrap()
        .expect("vez do aluno");
    loop {
        let has = |k| t.actions.iter().any(|a| a.kind == k);
        if has(ActionKind::AllIn) {
            sim.act(game, s, idx(&t, ActionKind::AllIn), tol).unwrap();
            break;
        }
        if t.to_call_chips > 0 && t.to_call_chips >= t.stack_chips {
            sim.act(game, s, idx(&t, ActionKind::Call), tol).unwrap();
            break;
        }
        let k = if has(ActionKind::Bet) {
            ActionKind::Bet
        } else if has(ActionKind::Raise) {
            ActionKind::Raise
        } else {
            ActionKind::Call
        };
        sim.act(game, s, idx(&t, k), tol).unwrap();
        t = sim
            .advance(game, s, rng, &mut villain)
            .unwrap()
            .unwrap_or_else(|| panic!("mão terminou antes do all-in: {:?}", sim.line()));
    }
    assert!(sim.advance(game, s, rng, &mut villain).unwrap().is_none());
    sim.outcome.clone().unwrap()
}

fn check_down(hero: usize, seed: u64) {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let mut rng = Rng::new(seed);
    let mut sim = HandSim::deal(game, hero, &mut rng).unwrap();
    let mut villain = pick(ActionKind::Check);
    let mut turns = 0;
    while let Some(t) = sim.advance(game, s, &mut rng, &mut villain).unwrap() {
        assert_eq!(t.to_call_chips, 0);
        let d = sim
            .act(game, s, idx(&t, ActionKind::Check), Tolerance::default())
            .unwrap();
        assert!(d.feedback.is_some(), "check-down segue avaliável");
        turns += 1;
    }
    assert_eq!(turns, 2, "uma decisão no turn e uma no river");
    let o = sim.outcome.clone().unwrap();
    assert!(o.showdown && o.folded.is_none());
    assert_eq!(o.board.len(), 5);
    assert_cards_consistent(&sim, &o.board);
    assert_eq!(o.pot_chips, s.starting_pot_chips);
    assert_eq!(o.hero_invested_chips, 0);
    let expected = match o.winner {
        Winner::Hero => 1000.0,
        Winner::Villain => 0.0,
        Winner::Split => 500.0,
    };
    assert_eq!(o.hero_net_chips, expected);
    assert!(o.hero_hand.is_some() && o.villain_hand.is_some());
    // Turn x, x | river card | x, x.
    let id = sim.node_id();
    assert_eq!(id.matches('|').count(), 2, "{id}");
    assert!(id.starts_with("x-x|"), "{id}");
    let streets: Vec<_> = sim.events.iter().map(|e| e.street).collect();
    assert_eq!(streets.iter().filter(|&&x| x == Street::Turn).count(), 2);
    assert_eq!(streets.iter().filter(|&&x| x == Street::River).count(), 3);
    assert!(sim.line().iter().any(|l| l.starts_with("River: ")));
}

#[test]
fn check_down_hero_oop() {
    check_down(0, 3);
}

#[test]
fn check_down_hero_ip() {
    check_down(1, 4);
}

#[test]
fn raise_line_and_fold_settlement() {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let mut rng = Rng::new(21);
    // Hero OOP bets the turn, villain raises, hero folds: hero loses only the bet.
    let mut sim = HandSim::deal(game, 0, &mut rng).unwrap();
    let mut villain = pick(ActionKind::Raise);
    let t = sim
        .advance(game, s, &mut rng, &mut villain)
        .unwrap()
        .unwrap();
    let bet = t.actions[idx(&t, ActionKind::Bet)].amount_chips;
    sim.act(game, s, idx(&t, ActionKind::Bet), Tolerance::default())
        .unwrap();
    let t = sim
        .advance(game, s, &mut rng, &mut villain)
        .unwrap()
        .unwrap();
    assert!(sim.events.last().unwrap().label.contains("aumenta para"));
    assert!(t.to_call_chips > 0);
    let d = sim
        .act(game, s, idx(&t, ActionKind::Fold), Tolerance::default())
        .unwrap();
    assert!(d.feedback.is_some());
    assert_eq!(
        d.feedback.as_ref().unwrap().actions[0].ev_chips,
        0.0,
        "desistir = 0"
    );
    assert!(sim
        .advance(game, s, &mut rng, &mut villain)
        .unwrap()
        .is_none());
    let o = sim.outcome.clone().unwrap();
    assert_eq!(o.folded.as_deref(), Some("BB"));
    assert_eq!(o.winner, Winner::Villain);
    assert!(!o.showdown);
    assert_eq!(o.hero_invested_chips, bet);
    assert_eq!(o.hero_net_chips, -(bet as f64));
    assert_eq!(o.board.len(), 4, "mão termina no turn");

    // Hero IP bets after a check, villain folds: hero wins the starting pot.
    let mut sim = HandSim::deal(game, 1, &mut rng).unwrap();
    let mut first = true;
    let mut villain = move |c: &VillainChoice, r: &mut Rng| {
        if std::mem::take(&mut first) {
            pick(ActionKind::Check)(c, r)
        } else {
            pick(ActionKind::Fold)(c, r)
        }
    };
    let t = sim
        .advance(game, s, &mut rng, &mut villain)
        .unwrap()
        .unwrap();
    sim.act(game, s, idx(&t, ActionKind::Bet), Tolerance::default())
        .unwrap();
    assert!(sim
        .advance(game, s, &mut rng, &mut villain)
        .unwrap()
        .is_none());
    let o = sim.outcome.clone().unwrap();
    assert_eq!(o.winner, Winner::Hero);
    assert_eq!(o.folded.as_deref(), Some("BB"));
    assert_eq!(o.hero_invested_chips, 0, "aposta não paga volta ao aluno");
    assert_eq!(o.hero_net_chips, s.starting_pot_chips as f64);
}

#[test]
fn all_in_runs_out_the_board() {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let mut rng = Rng::new(5);
    for seed in 0..6 {
        let mut sim = HandSim::deal(game, seed % 2, &mut rng).unwrap();
        let o = shove(&mut sim, game, s, &mut rng);
        assert!(o.showdown);
        assert_eq!(o.board.len(), 5, "river distribuído depois do all-in");
        assert_cards_consistent(&sim, &o.board);
        assert_eq!(
            o.pot_chips,
            s.starting_pot_chips + 2 * s.effective_stack_chips
        );
        let stack = s.effective_stack_chips as f64;
        let expected = match o.winner {
            Winner::Hero => s.starting_pot_chips as f64 + stack,
            Winner::Villain => -stack,
            Winner::Split => s.starting_pot_chips as f64 / 2.0,
        };
        assert_eq!(o.hero_net_chips, expected);
        assert!(sim
            .decisions
            .iter()
            .all(|d| d.feedback.is_some() || d.not_evaluable.is_some()));
        if sim.decisions.last().unwrap().turn.street == Street::Turn {
            assert!(sim
                .events
                .iter()
                .any(|e| e.kind == EventKind::Card && e.label.starts_with("River: ")));
        }
    }
}

#[test]
fn zero_weight_line_stops_grading() {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let mut rng = Rng::new(99);
    let mut villain = |c: &VillainChoice, _: &mut Rng| passive(c);
    for _ in 0..400 {
        let mut sim = HandSim::deal(game, 0, &mut rng).unwrap();
        let t = sim
            .advance(game, s, &mut rng, &mut villain)
            .unwrap()
            .unwrap();
        // Probe the frequencies of every action for this combo without touching `sim`.
        let probe = sim
            .clone()
            .act(game, s, 0, Tolerance::default())
            .unwrap()
            .clone();
        let fb = probe.feedback.unwrap();
        let Some(never) = fb
            .actions
            .iter()
            .position(|a| a.frequency == 0.0 && t.actions[a.index].kind != ActionKind::Fold)
        else {
            continue;
        };
        let d = sim
            .act(game, s, never, Tolerance::default())
            .unwrap()
            .clone();
        assert!(d.feedback.is_some(), "a própria decisão ainda é avaliada");
        let Some(next) = sim.advance(game, s, &mut rng, &mut villain).unwrap() else {
            continue;
        };
        let d2 = sim
            .act(game, s, next.actions.len() - 1, Tolerance::default())
            .unwrap();
        assert!(d2.feedback.is_none());
        let why = d2.not_evaluable.as_deref().unwrap();
        assert!(why.contains("última decisão avaliável"));
        assert!(why.contains("abaixo de 0,001%"), "{why}");
        assert!(sim.cut.is_some());
        return;
    }
    panic!("nenhuma linha de frequência nula encontrada");
}

#[test]
fn villain_follows_engine_strategy_of_its_combo() {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let mut rng = Rng::new(7);
    // Hero IP: the villain (OOP) acts first. Find a deal where its first decision is mixed.
    for _ in 0..500 {
        let base = HandSim::deal(game, 1, &mut rng).unwrap();
        let mut probs: Vec<f32> = Vec::new();
        let mut counts: HashMap<usize, usize> = HashMap::new();
        let n = 4000;
        for _ in 0..n {
            let mut sim = base.clone();
            let mut taken = None;
            let mut policy = |c: &VillainChoice, r: &mut Rng| {
                if taken.is_some() {
                    return passive(c);
                }
                probs = c.probs.to_vec();
                let k = engine_policy(c, r);
                taken = Some(k);
                k
            };
            sim.advance(game, s, &mut rng, &mut policy).unwrap();
            *counts.entry(taken.unwrap()).or_default() += 1;
        }
        let mixed = probs.iter().filter(|&&p| (0.2..0.8).contains(&p)).count() >= 2;
        if !mixed {
            continue;
        }
        for (a, &p) in probs.iter().enumerate() {
            let got = *counts.get(&a).unwrap_or(&0) as f64 / n as f64;
            assert!((got - p as f64).abs() < 0.03, "ação {a}: {got} vs {p}");
        }
        return;
    }
    panic!("nenhuma decisão mista encontrada para o adversário");
}

#[test]
fn rake_is_taken_from_the_pot() {
    let mut s = load("river_poc.toml");
    s.rake.rate = 0.05;
    s.rake.cap_chips = 300.0;
    let sg = solve_spot(&s, &AtomicBool::new(false), |_| {}).unwrap();
    let mut owned = sg.game;
    let game = &mut owned;
    let mut rng = Rng::new(3);
    // River only: hero shoves, villain calls -> showdown on a 9000-chip pot, rake capped.
    let mut sim = HandSim::deal(game, 0, &mut rng).unwrap();
    let o = shove(&mut sim, game, &s, &mut rng);
    assert_eq!(o.pot_chips, 9000);
    assert_eq!(o.rake_chips, 300.0);
    let expected = match o.winner {
        Winner::Hero => 9000.0 - 300.0 - 4000.0,
        Winner::Villain => -4000.0,
        Winner::Split => (9000.0 - 300.0) / 2.0 - 4000.0,
    };
    assert_eq!(o.hero_net_chips, expected);
}

#[test]
fn node_ids_match_pack_format() {
    let mut g = lock();
    let (ref s, ref mut game) = *g;
    let mut rng = Rng::new(8);
    let mut sim = HandSim::deal(game, 1, &mut rng).unwrap();
    let mut villain = pick(ActionKind::Check);
    let t = sim
        .advance(game, s, &mut rng, &mut villain)
        .unwrap()
        .unwrap();
    assert_eq!(t.node_id, "x");
    let bet = idx(&t, ActionKind::Bet);
    let code = format!("b{}", t.actions[bet].amount_chips);
    sim.act(game, s, bet, Tolerance::default()).unwrap();
    let mut call = pick(ActionKind::Call);
    let t = sim.advance(game, s, &mut rng, &mut call).unwrap();
    let id = sim.node_id();
    // x-bN-c|<river card>|x ... (villain OOP checks the river first under `call` policy)
    assert!(id.starts_with(&format!("x-{code}-c|")), "{id}");
    if let Some(t) = t {
        assert_eq!(t.street, Street::River);
        assert_eq!(t.board.len(), 5);
    }
    let _ = Combo::parse("AsKs").unwrap();
}
