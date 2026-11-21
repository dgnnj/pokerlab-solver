//! Full hand (flop to river) against the engine policy.
//!
//! The pack only carries the main lines, so a full hand needs the whole flop tree. The app solves
//! it in a background thread with the exact spot stored in the pack solution (same configuration
//! hash), keeps one solved board in memory and plays any number of hands on it. Independent from
//! Tauri so it can be tested.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use poker_core::eval::{Feedback, Tolerance};
use poker_core::sampling::Rng;
use poker_core::solution::SolveInfo;
use poker_core::spot::SpotConfig;
use rusqlite::params;
use serde::Serialize;
use solver_adapter::hand::{engine_policy, Decision, HandSim, HeroTurn, Winner};
use solver_adapter::{solve_spot, AdapterError, PostFlopGame, ENGINE_REV};

use crate::db::{now, unix_now, Db};
use crate::trainer::{explain, ActionOption, TrainerError};

type R<T> = Result<T, TrainerError>;

fn msg<T>(m: impl Into<String>) -> R<T> {
    Err(TrainerError::Msg(m.into()))
}

/// Above this estimate the solve uses the engine's compressed storage (16-bit), about half the
/// memory. Every board of the bundled pack fits uncompressed (largest ≈ 4.2 GiB).
pub const MEMORY_LIMIT_MIB: u64 = 6 * 1024;

/// What the app needs to know about the pack solution a board comes from.
#[derive(Debug, Clone)]
pub struct BoardSource {
    pub solution_id: String,
    pub spot: SpotConfig,
    /// `config_hash` recorded in the pack (to confirm the in-app solve uses the same tree).
    pub pack_config_hash: String,
    pub pack_exploitability_pct: f64,
    pub pack_id: String,
    pub pack_version: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum EngineStatus {
    Idle,
    Solving {
        solution_id: String,
        board: String,
        iteration: u32,
        max_iterations: u32,
        exploitability_pct: f64,
        target_pct: f64,
        elapsed_ms: u64,
    },
    Ready {
        solution_id: String,
        board: String,
        exploitability_pct: f64,
        pack_exploitability_pct: f64,
        solve_ms: u64,
        memory_mib: u64,
        compressed: bool,
        same_config_as_pack: bool,
        threads: usize,
    },
    Failed {
        solution_id: String,
        board: String,
        error: String,
    },
}

struct ActiveHand {
    sim: HandSim,
    started_at: String,
}

pub struct Ready {
    src: BoardSource,
    game: PostFlopGame,
    info: SolveInfo,
    config_hash: String,
    hands: HashMap<String, ActiveHand>,
    rng: Rng,
    seq: u64,
}

enum Slot {
    Idle,
    Solving {
        src: BoardSource,
        progress: (u32, u32, f64),
        started: Instant,
    },
    Ready(Box<Ready>),
    Failed {
        src: BoardSource,
        error: String,
    },
}

/// One solved board at a time; a new `prepare` cancels the running solve and frees the old tree.
pub struct HandEngine {
    slot: Arc<Mutex<Slot>>,
    generation: Arc<AtomicU64>,
    cancel: Mutex<Arc<AtomicBool>>,
    pub tolerance: Tolerance,
    seed: u64,
}

fn lock_err<T>(_: T) -> TrainerError {
    TrainerError::Msg("estado do motor bloqueado".into())
}

impl HandEngine {
    pub fn new(seed: u64) -> Self {
        HandEngine {
            slot: Arc::new(Mutex::new(Slot::Idle)),
            generation: Arc::new(AtomicU64::new(0)),
            cancel: Mutex::new(Arc::new(AtomicBool::new(false))),
            tolerance: Tolerance::default(),
            seed,
        }
    }

    pub fn status(&self) -> R<EngineStatus> {
        let g = self.slot.lock().map_err(lock_err)?;
        Ok(match &*g {
            Slot::Idle => EngineStatus::Idle,
            Slot::Solving {
                src,
                progress,
                started,
            } => EngineStatus::Solving {
                solution_id: src.solution_id.clone(),
                board: src.spot.board.clone(),
                iteration: progress.0,
                max_iterations: progress.1,
                exploitability_pct: progress.2,
                target_pct: src.spot.solve.target_exploitability_pct,
                elapsed_ms: started.elapsed().as_millis() as u64,
            },
            Slot::Ready(r) => EngineStatus::Ready {
                solution_id: r.src.solution_id.clone(),
                board: r.src.spot.board.clone(),
                exploitability_pct: r.info.final_exploitability_pct_pot,
                pack_exploitability_pct: r.src.pack_exploitability_pct,
                solve_ms: r.info.solve_ms,
                memory_mib: r.info.memory_estimate_bytes >> 20,
                compressed: r.info.compressed,
                same_config_as_pack: r.config_hash == r.src.pack_config_hash,
                threads: r.info.threads,
            },
            Slot::Failed { src, error } => EngineStatus::Failed {
                solution_id: src.solution_id.clone(),
                board: src.spot.board.clone(),
                error: error.clone(),
            },
        })
    }

    /// Starts solving `src` in the background (no-op if it is already solving or solved).
    pub fn prepare(&self, src: BoardSource) -> R<EngineStatus> {
        {
            let mut g = self.slot.lock().map_err(lock_err)?;
            match &*g {
                Slot::Ready(r) if r.src.solution_id == src.solution_id => {
                    drop(g);
                    return self.status();
                }
                Slot::Solving { src: s, .. } if s.solution_id == src.solution_id => {
                    drop(g);
                    return self.status();
                }
                _ => {}
            }
            // Cancel the running solve and drop the previous tree before allocating a new one.
            self.cancel
                .lock()
                .map_err(lock_err)?
                .store(true, Ordering::Relaxed);
            *g = Slot::Solving {
                src: src.clone(),
                progress: (0, src.spot.solve.max_iterations, f64::NAN),
                started: Instant::now(),
            };
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.lock().map_err(lock_err)? = cancel.clone();
        let gen = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let generation = self.generation.clone();
        let slot = self.slot.clone();
        let seed = self.seed ^ gen.wrapping_mul(0x9E37_79B9);
        std::thread::Builder::new()
            .name("pokerlab-solver".into())
            .spawn(move || {
                let mut spot = src.spot.clone();
                spot.solve.memory_limit_mib = MEMORY_LIMIT_MIB;
                let pot = spot.starting_pot_chips as f64;
                let result = solve_spot(&spot, &cancel, |p| {
                    if generation.load(Ordering::SeqCst) != gen {
                        return;
                    }
                    if let Ok(mut g) = slot.lock() {
                        if let Slot::Solving { progress, .. } = &mut *g {
                            *progress = (
                                p.iteration,
                                p.max_iterations,
                                100.0 * p.exploitability_chips as f64 / pot,
                            );
                        }
                    }
                });
                if generation.load(Ordering::SeqCst) != gen {
                    return; // superseded: drop the result
                }
                let Ok(mut g) = slot.lock() else { return };
                *g = match result {
                    Ok(solved) => Slot::Ready(Box::new(Ready {
                        // The hash of the spot as stored in the pack (memory limit is not part
                        // of the tree, so compare the unmodified spot).
                        config_hash: src.spot.config_hash(ENGINE_REV),
                        src,
                        game: solved.game,
                        info: solved.info,
                        hands: HashMap::new(),
                        rng: Rng::new(seed),
                        seq: 0,
                    })),
                    Err(AdapterError::Cancelled) => Slot::Idle,
                    Err(e) => Slot::Failed {
                        src,
                        error: e.to_string(),
                    },
                };
            })
            .map_err(|e| TrainerError::Msg(format!("thread do solver: {e}")))?;
        self.status()
    }

    pub fn cancel(&self) -> R<EngineStatus> {
        self.cancel
            .lock()
            .map_err(lock_err)?
            .store(true, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::SeqCst);
        {
            let mut g = self.slot.lock().map_err(lock_err)?;
            if matches!(&*g, Slot::Solving { .. }) {
                *g = Slot::Idle;
            }
        }
        self.status()
    }

    fn with_ready<T>(&self, f: impl FnOnce(&mut Ready) -> R<T>) -> R<T> {
        let mut g = self.slot.lock().map_err(lock_err)?;
        match &mut *g {
            Slot::Ready(r) => f(r),
            Slot::Solving { .. } => msg("a árvore ainda está sendo resolvida"),
            _ => msg("prepare um board antes de jogar uma mão"),
        }
    }

    /// Deals a new hand. `hero` is a position ("BB", "BTN") or "random".
    pub fn start(&self, db: &Db, hero: &str) -> R<HandView> {
        self.with_ready(|r| {
            let spot = &r.src.spot;
            let player = if hero == spot.oop_position {
                0
            } else if hero == spot.ip_position {
                1
            } else if hero == "random" {
                r.rng.below(2)
            } else {
                return msg(format!("posição desconhecida: {hero}"));
            };
            let mut sim = HandSim::deal(&mut r.game, player, &mut r.rng)?;
            sim.advance(&mut r.game, spot, &mut r.rng, &mut engine_policy)?;
            r.seq += 1;
            let token = format!("h{}-{}-{}", unix_now(), r.seq, r.rng.next_u64() % 1_000_000);
            let started_at = now();
            db.conn.execute(
                "INSERT INTO hands(token, started_at, solution_id, config_hash, same_config_as_pack, pack_id,
                    pack_version, engine_rev, solve_exploitability_pct, hero_position, villain_position,
                    hero_combo, villain_combo, flop)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)",
                params![
                    token,
                    started_at,
                    r.src.solution_id,
                    r.config_hash,
                    r.config_hash == r.src.pack_config_hash,
                    r.src.pack_id,
                    r.src.pack_version,
                    ENGINE_REV,
                    r.info.final_exploitability_pct_pot,
                    position(spot, player),
                    position(spot, player ^ 1),
                    sim.hero_combo.to_string(),
                    sim.villain_combo.to_string(),
                    spot.board,
                ],
            )?;
            r.hands.insert(token.clone(), ActiveHand { sim, started_at });
            // Same path as `act`, in case the hand ended before the hero's first decision.
            finalize_if_done(db, r, &token)?;
            view(r, &token)
        })
    }

    pub fn act(
        &self,
        db: &Db,
        token: &str,
        action: usize,
        decision_ms: Option<i64>,
    ) -> R<HandView> {
        let tol = self.tolerance;
        self.with_ready(|r| {
            let Ready {
                src,
                game,
                hands,
                rng,
                ..
            } = r;
            let h = hands
                .get_mut(token)
                .ok_or_else(|| TrainerError::Msg("mão encerrada ou de outro board".into()))?;
            if h.sim.is_finished() {
                return msg("a mão já terminou");
            }
            let d = h.sim.act(game, &src.spot, action, tol)?.clone();
            insert_decision(db, token, &d, decision_ms.unwrap_or(0))?;
            h.sim.advance(game, &src.spot, rng, &mut engine_policy)?;
            finalize_if_done(db, r, token)?;
            view(r, token)
        })
    }

    pub fn view(&self, token: &str) -> R<HandView> {
        self.with_ready(|r| view(r, token))
    }

    /// Drops finished hands from memory (their record stays in the database).
    pub fn forget(&self, token: &str) -> R<()> {
        self.with_ready(|r| {
            r.hands.remove(token);
            Ok(())
        })
    }
}

fn position(spot: &SpotConfig, player: usize) -> String {
    if player == 0 {
        spot.oop_position.clone()
    } else {
        spot.ip_position.clone()
    }
}

fn insert_decision(db: &Db, token: &str, d: &Decision, ms: i64) -> R<()> {
    let fb = d.feedback.as_ref();
    let label = |i: usize| d.turn.actions.get(i).map(|a| a.label.clone());
    db.conn.execute(
        "INSERT INTO hand_decisions(hand_id, seq, created_at, node_id, street, board, pot_chips, chosen_index,
            chosen_label, best_index, best_label, best_ev_chips, chosen_ev_chips, loss_bb, loss_pct_pot, grade,
            chosen_frequency, equity, not_evaluable, decision_ms)
         SELECT id, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20
         FROM hands WHERE token = ?1",
        params![
            token,
            d.index as i64,
            now(),
            d.turn.node_id,
            d.turn.street.label_pt(),
            d.turn.board.join(""),
            d.turn.pot_chips,
            d.chosen as i64,
            d.chosen_label,
            fb.map(|f| f.best_index as i64),
            fb.and_then(|f| label(f.best_index)),
            fb.map(|f| f.best_ev_chips),
            fb.map(|f| f.chosen_ev_chips),
            fb.map(|f| f.loss_bb),
            fb.map(|f| f.loss_pct_pot),
            fb.map(|f| f.grade.as_str()),
            fb.map(|f| f.chosen_frequency as f64),
            d.equity.map(|e| e as f64),
            d.not_evaluable,
            ms,
        ],
    )?;
    Ok(())
}

fn finalize_if_done(db: &Db, r: &mut Ready, token: &str) -> R<()> {
    let Some(h) = r.hands.get(token) else {
        return Ok(());
    };
    let Some(o) = &h.sim.outcome else {
        return Ok(());
    };
    let cpb = r.src.spot.chips_per_bb as f64;
    let graded: Vec<&Feedback> = h
        .sim
        .decisions
        .iter()
        .filter_map(|d| d.feedback.as_ref())
        .collect();
    db.conn.execute(
        "UPDATE hands SET ended_at = ?2, final_board = ?3, line = ?4, showdown = ?5, winner = ?6, pot_chips = ?7,
            hero_net_bb = ?8, decisions = ?9, graded = ?10, total_loss_bb = ?11, cut_reason = ?12
         WHERE token = ?1 AND ended_at IS NULL",
        params![
            token,
            now(),
            o.board.join(""),
            h.sim.line().join(" · "),
            o.showdown,
            winner_str(o.winner),
            o.pot_chips,
            o.hero_net_chips / cpb,
            h.sim.decisions.len() as i64,
            graded.len() as i64,
            graded.iter().map(|f| f.loss_bb).sum::<f64>(),
            h.sim.cut,
        ],
    )?;
    Ok(())
}

fn winner_str(w: Winner) -> &'static str {
    match w {
        Winner::Hero => "hero",
        Winner::Villain => "villain",
        Winner::Split => "split",
    }
}

// ---------- Views ----------

#[derive(Debug, Clone, Serialize)]
pub struct EventView {
    pub street: String,
    pub kind: String,
    pub hero: bool,
    pub label: String,
    pub pot_bb: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DecisionView {
    pub seq: usize,
    pub street: String,
    pub board: Vec<String>,
    pub line: Vec<String>,
    pub pot_bb: f64,
    pub to_call_bb: f64,
    pub actions: Vec<ActionOption>,
    pub chosen_index: usize,
    pub chosen_label: String,
    pub feedback: Option<Feedback>,
    pub grade_label: Option<String>,
    pub explanation: Vec<String>,
    pub equity: Option<f32>,
    pub tolerance_bb: Option<f64>,
    pub not_evaluable: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OutcomeView {
    pub showdown: bool,
    pub folded: Option<String>,
    pub winner: String,
    pub pot_bb: f64,
    pub rake_bb: f64,
    pub hero_invested_bb: f64,
    /// Postflop result (starting pot counted as won by whoever takes the pot).
    pub hero_net_bb: f64,
    /// Including the preflop investment, when the spot documents it.
    pub hero_total_bb: Option<f64>,
    pub hero_hand: Option<String>,
    pub villain_hand: Option<String>,
    pub summary: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandSummary {
    pub decisions: usize,
    pub graded: usize,
    pub total_loss_bb: f64,
    pub avg_loss_bb: f64,
    pub grades: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandView {
    pub token: String,
    pub finished: bool,
    pub solution_id: String,
    pub hero_position: String,
    pub villain_position: String,
    pub hero_cards: [String; 2],
    /// Revealed when the hand ends.
    pub villain_cards: Option<[String; 2]>,
    pub board: Vec<String>,
    pub street: String,
    pub pot_bb: f64,
    pub to_call_bb: f64,
    pub hero_stack_bb: f64,
    pub villain_stack_bb: f64,
    pub preflop_line: String,
    pub events: Vec<EventView>,
    pub actions: Vec<ActionOption>,
    pub decisions: Vec<DecisionView>,
    pub outcome: Option<OutcomeView>,
    pub cut: Option<String>,
    pub summary: HandSummary,
    pub started_at: String,
}

fn split_combo(s: &str) -> [String; 2] {
    [s[0..2].to_string(), s[2..4].to_string()]
}

fn action_options(t: &HeroTurn) -> Vec<ActionOption> {
    t.actions
        .iter()
        .enumerate()
        .map(|(i, a)| ActionOption {
            index: i,
            label: a.label.clone(),
            kind: format!("{:?}", a.kind).to_lowercase(),
        })
        .collect()
}

fn decision_view(d: &Decision, cpb: f64) -> DecisionView {
    let explanation = match (&d.feedback, d.equity) {
        (Some(fb), Some(eq)) => explain(&d.turn.actions, fb, eq, cpb),
        _ => Vec::new(),
    };
    DecisionView {
        seq: d.index,
        street: d.turn.street.label_pt().to_string(),
        board: d.turn.board.clone(),
        line: d.line.clone(),
        pot_bb: d.turn.pot_chips as f64 / cpb,
        to_call_bb: d.turn.to_call_chips as f64 / cpb,
        actions: action_options(&d.turn),
        chosen_index: d.chosen,
        chosen_label: d.chosen_label.clone(),
        grade_label: d.feedback.as_ref().map(|f| f.grade.label_pt().to_string()),
        tolerance_bb: d.feedback.as_ref().map(|f| f.tolerance_chips / cpb),
        feedback: d.feedback.clone(),
        explanation,
        equity: d.equity,
        not_evaluable: d.not_evaluable.clone(),
    }
}

fn view(r: &mut Ready, token: &str) -> R<HandView> {
    let Ready {
        src, game, hands, ..
    } = r;
    let spot = &src.spot;
    let h = hands
        .get(token)
        .ok_or_else(|| TrainerError::Msg("mão encerrada ou de outro board".into()))?;
    let sim = &h.sim;
    let cpb = spot.chips_per_bb as f64;
    let turn = sim.current_turn(game, spot);
    // Pot and stacks from the current node (or the final state).
    game.apply_history(&sim.history);
    let tb = game.total_bet_amount();
    let board_now: Vec<String> = game
        .current_board()
        .iter()
        .map(|&c| poker_core::cards::card_to_string(c))
        .collect();
    game.back_to_root();
    let (hero, villain) = (sim.hero, sim.villain());
    let finished = sim.outcome.is_some();
    let put_in = |p: usize| if finished { tb[0].min(tb[1]) } else { tb[p] };
    let outcome = sim.outcome.as_ref().map(|o| {
        let preflop = spot
            .preflop
            .as_ref()
            .map(|p| p.invested_per_player_chips as f64);
        let net = o.hero_net_chips / cpb;
        let summary = match (o.winner, &o.folded) {
            (Winner::Hero, Some(f)) => format!("{f} desistiu: você leva o pote."),
            (Winner::Villain, Some(_)) => "Você desistiu.".to_string(),
            (Winner::Hero, None) => format!(
                "Showdown: você vence com {}.",
                o.hero_hand.clone().unwrap_or_default()
            ),
            (Winner::Villain, None) => format!(
                "Showdown: o adversário vence com {}.",
                o.villain_hand.clone().unwrap_or_default()
            ),
            (Winner::Split, _) => "Showdown: pote dividido.".to_string(),
        };
        OutcomeView {
            showdown: o.showdown,
            folded: o.folded.clone(),
            winner: winner_str(o.winner).to_string(),
            pot_bb: o.pot_chips as f64 / cpb,
            rake_bb: o.rake_chips / cpb,
            hero_invested_bb: o.hero_invested_chips as f64 / cpb,
            hero_net_bb: net,
            hero_total_bb: preflop.map(|p| (o.hero_net_chips - p) / cpb),
            hero_hand: o.hero_hand.clone(),
            villain_hand: o.villain_hand.clone(),
            summary,
        }
    });
    let decisions: Vec<DecisionView> = sim
        .decisions
        .iter()
        .map(|d| decision_view(d, cpb))
        .collect();
    let graded: Vec<&Feedback> = sim
        .decisions
        .iter()
        .filter_map(|d| d.feedback.as_ref())
        .collect();
    let mut grades = BTreeMap::new();
    for f in &graded {
        *grades.entry(f.grade.as_str().to_string()).or_insert(0) += 1;
    }
    let total: f64 = graded.iter().map(|f| f.loss_bb).sum();
    let board = sim
        .outcome
        .as_ref()
        .map(|o| o.board.clone())
        .unwrap_or(board_now);
    Ok(HandView {
        token: token.to_string(),
        finished: sim.is_finished(),
        solution_id: src.solution_id.clone(),
        hero_position: position(spot, hero),
        villain_position: position(spot, villain),
        hero_cards: split_combo(&sim.hero_combo.to_string()),
        villain_cards: sim
            .is_finished()
            .then(|| split_combo(&sim.villain_combo.to_string())),
        street: match board.len() {
            3 => "flop",
            4 => "turn",
            _ => "river",
        }
        .to_string(),
        board,
        // At the end, unmatched chips are back with their owner: show the contested pot.
        pot_bb: match &sim.outcome {
            Some(o) => o.pot_chips as f64 / cpb,
            None => (spot.starting_pot_chips + tb[0] + tb[1]) as f64 / cpb,
        },
        to_call_bb: turn
            .as_ref()
            .map(|t| t.to_call_chips as f64 / cpb)
            .unwrap_or(0.0),
        hero_stack_bb: (spot.effective_stack_chips - put_in(hero)) as f64 / cpb,
        villain_stack_bb: (spot.effective_stack_chips - put_in(villain)) as f64 / cpb,
        preflop_line: spot
            .preflop
            .as_ref()
            .map(|p| p.line.clone())
            .unwrap_or_default(),
        events: sim
            .events
            .iter()
            .map(|e| EventView {
                street: e.street.label_pt().to_string(),
                kind: match e.kind {
                    solver_adapter::hand::EventKind::Action => "action",
                    solver_adapter::hand::EventKind::Card => "card",
                }
                .to_string(),
                hero: e.hero,
                label: e.label.clone(),
                pot_bb: e.pot_chips as f64 / cpb,
            })
            .collect(),
        actions: turn.as_ref().map(action_options).unwrap_or_default(),
        decisions,
        outcome,
        cut: sim.cut.clone(),
        summary: HandSummary {
            decisions: sim.decisions.len(),
            graded: graded.len(),
            total_loss_bb: total,
            avg_loss_bb: if graded.is_empty() {
                0.0
            } else {
                total / graded.len() as f64
            },
            grades,
        },
        started_at: h.started_at.clone(),
    })
}

// ---------- History ----------

#[derive(Debug, Clone, Serialize)]
pub struct HandRow {
    pub id: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub hero_position: String,
    pub hero_combo: String,
    pub villain_combo: String,
    pub flop: String,
    pub final_board: Option<String>,
    pub line: Option<String>,
    pub winner: Option<String>,
    pub showdown: Option<bool>,
    pub hero_net_bb: Option<f64>,
    pub decisions: i64,
    pub graded: i64,
    pub total_loss_bb: f64,
    pub cut_reason: Option<String>,
    pub pack_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandStats {
    pub hands: i64,
    pub finished: i64,
    pub net_bb: f64,
    pub decisions: i64,
    pub graded: i64,
    pub total_loss_bb: f64,
    pub avg_loss_per_decision_bb: f64,
    pub avg_loss_per_hand_bb: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HandHistory {
    pub stats: HandStats,
    pub by_position: Vec<(String, HandStats)>,
    pub recent: Vec<HandRow>,
}

fn stats(db: &Db, filter: &str, args: &[&dyn rusqlite::ToSql]) -> R<HandStats> {
    let sql = format!(
        "SELECT COUNT(*), COUNT(ended_at), COALESCE(SUM(hero_net_bb), 0), COALESCE(SUM(decisions), 0),
                COALESCE(SUM(graded), 0), COALESCE(SUM(total_loss_bb), 0)
         FROM hands {filter}"
    );
    let (hands, finished, net, dec, graded, loss): (i64, i64, f64, i64, i64, f64) =
        db.conn.query_row(&sql, args, |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        })?;
    Ok(HandStats {
        hands,
        finished,
        net_bb: net,
        decisions: dec,
        graded,
        total_loss_bb: loss,
        avg_loss_per_decision_bb: if graded > 0 {
            loss / graded as f64
        } else {
            0.0
        },
        avg_loss_per_hand_bb: if finished > 0 {
            loss / finished as f64
        } else {
            0.0
        },
    })
}

pub fn history(db: &Db, limit: i64) -> R<HandHistory> {
    let mut st = db.conn.prepare(
        "SELECT id, started_at, ended_at, hero_position, hero_combo, villain_combo, flop, final_board, line,
                winner, showdown, hero_net_bb, decisions, graded, total_loss_bb, cut_reason, pack_version
         FROM hands ORDER BY id DESC LIMIT ?1",
    )?;
    let recent = st
        .query_map([limit], |r| {
            Ok(HandRow {
                id: r.get(0)?,
                started_at: r.get(1)?,
                ended_at: r.get(2)?,
                hero_position: r.get(3)?,
                hero_combo: r.get(4)?,
                villain_combo: r.get(5)?,
                flop: r.get(6)?,
                final_board: r.get(7)?,
                line: r.get(8)?,
                winner: r.get(9)?,
                showdown: r.get(10)?,
                hero_net_bb: r.get(11)?,
                decisions: r.get(12)?,
                graded: r.get(13)?,
                total_loss_bb: r.get(14)?,
                cut_reason: r.get(15)?,
                pack_version: r.get(16)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut by_position = Vec::new();
    let mut pst = db
        .conn
        .prepare("SELECT DISTINCT hero_position FROM hands ORDER BY hero_position")?;
    let positions: Vec<String> = pst.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for p in positions {
        let s = stats(
            db,
            "WHERE hero_position = ?1",
            &[&p as &dyn rusqlite::ToSql],
        )?;
        by_position.push((p, s));
    }
    Ok(HandHistory {
        stats: stats(db, "", &[])?,
        by_position,
        recent,
    })
}

/// CSV columns of the full-hand export: one row per hero decision, with the hand's data repeated
/// on each row (hands with no decision yet, e.g. interrupted right after the deal, get one row
/// with empty decision columns). An empty `ended_at` marks an interrupted hand.
const HAND_CSV_COLUMNS: &[(&str, &str)] = &[
    ("h.id", "hand_id"),
    ("h.started_at", "started_at"),
    ("h.ended_at", "ended_at"),
    ("h.pack_id", "pack_id"),
    ("h.pack_version", "pack_version"),
    ("h.engine_rev", "engine_rev"),
    ("h.solution_id", "solution_id"),
    ("h.same_config_as_pack", "same_config_as_pack"),
    ("h.solve_exploitability_pct", "solve_exploitability_pct"),
    ("h.hero_position", "hero_position"),
    ("h.villain_position", "villain_position"),
    ("h.hero_combo", "hero_combo"),
    ("h.villain_combo", "villain_combo"),
    ("h.flop", "flop"),
    ("h.final_board", "final_board"),
    ("h.line", "line"),
    ("h.showdown", "showdown"),
    ("h.winner", "winner"),
    ("h.pot_chips", "final_pot_chips"),
    ("h.hero_net_bb", "hero_net_bb"),
    ("h.decisions", "hand_decisions"),
    ("h.graded", "hand_graded"),
    ("h.total_loss_bb", "hand_total_loss_bb"),
    ("h.cut_reason", "cut_reason"),
    ("d.seq", "decision_seq"),
    ("d.created_at", "decision_at"),
    ("d.node_id", "node_id"),
    ("d.street", "street"),
    ("d.board", "board"),
    ("d.pot_chips", "pot_chips"),
    ("d.chosen_label", "chosen"),
    ("d.best_label", "best"),
    ("d.best_ev_chips", "best_ev_chips"),
    ("d.chosen_ev_chips", "chosen_ev_chips"),
    ("d.loss_bb", "loss_bb"),
    ("d.loss_pct_pot", "loss_pct_pot"),
    ("d.grade", "grade"),
    ("d.chosen_frequency", "chosen_frequency"),
    ("d.equity", "equity"),
    ("d.not_evaluable", "not_evaluable"),
    ("d.decision_ms", "decision_ms"),
];

/// Writes every full hand and its decisions to `path` (see [`HAND_CSV_COLUMNS`]). Returns
/// `(hands, rows)`.
pub fn export_csv(db: &Db, path: &std::path::Path) -> R<(usize, usize)> {
    let (out, rows) = db.query_csv(
        HAND_CSV_COLUMNS,
        "FROM hands h LEFT JOIN hand_decisions d ON d.hand_id = h.id ORDER BY h.id, d.seq",
    )?;
    let hands: i64 = db
        .conn
        .query_row("SELECT COUNT(*) FROM hands", [], |r| r.get(0))?;
    poker_core::solution::write_atomic(path, out.as_bytes())?;
    Ok((hands as usize, rows))
}
