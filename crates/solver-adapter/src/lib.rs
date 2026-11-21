//! The only module that talks to `postflop-solver`.
//!
//! Responsibilities: translate a [`SpotConfig`] into the engine's `CardConfig`/`TreeConfig`,
//! estimate memory, solve with progress and cooperative cancellation, navigate the tree and
//! export decision nodes into the engine-independent [`SolutionFile`] format.
//!
//! Seat mapping: engine player 0 = OOP, player 1 = IP. Real positions (e.g. BB/BTN) are kept in
//! the spot and attached to every exported node.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use poker_core::cards::{card_to_string, Combo};
use poker_core::solution::{
    round_ev, round_freq, ActionInfo, ActionKind, EngineInfo, NodeSolution, Seat, SolutionFile,
    SolveInfo, SolveStatus, SOLUTION_SCHEMA_VERSION,
};
use poker_core::spot::{SpotConfig, Street};
use poker_core::{fmt_bb, now_iso8601};
use postflop_solver::{
    compute_exploitability, finalize, solve_step, Action, ActionTree, BetSizeOptions, BoardState,
    CardConfig, DonkSizeOptions, PostFlopGame, Range, TreeConfig, NOT_DEALT,
};

pub const ENGINE_NAME: &str = "postflop-solver";
/// Must match the `rev` pinned in the workspace Cargo.toml.
pub const ENGINE_REV: &str = "9d1509fe5077d019825f833eed04b16d342dfda1";

#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("configuração: {0}")]
    Config(String),
    #[error(
        "memória estimada {estimate_mib} MiB excede o limite {limit_mib} MiB (mesmo comprimido)"
    )]
    OutOfMemory { estimate_mib: u64, limit_mib: u64 },
    #[error("cálculo cancelado")]
    Cancelled,
    #[error("navegação: {0}")]
    Navigation(String),
    #[error(transparent)]
    Core(#[from] poker_core::CoreError),
}

pub fn engine_info() -> EngineInfo {
    EngineInfo {
        name: ENGINE_NAME.into(),
        git_rev: ENGINE_REV.into(),
    }
}

fn sizes(bet: &str, raise: &str) -> Result<BetSizeOptions, AdapterError> {
    BetSizeOptions::try_from((bet, raise))
        .map_err(|e| AdapterError::Config(format!("tamanhos '{bet}'/'{raise}': {e}")))
}

fn donk(s: &str) -> Result<Option<DonkSizeOptions>, AdapterError> {
    if s.trim().is_empty() {
        return Ok(None);
    }
    DonkSizeOptions::try_from(s)
        .map(Some)
        .map_err(|e| AdapterError::Config(format!("donk '{s}': {e}")))
}

/// Builds the engine game (tree + ranges), without allocating solver memory.
pub fn build_game(spot: &SpotConfig) -> Result<PostFlopGame, AdapterError> {
    spot.validate()?;
    let board = spot.board_cards()?;
    let oop: Range = spot
        .oop_range
        .text
        .parse()
        .map_err(|e| AdapterError::Config(format!("range OOP: {e}")))?;
    let ip: Range = spot
        .ip_range
        .text
        .parse()
        .map_err(|e| AdapterError::Config(format!("range IP: {e}")))?;
    let card_config = CardConfig {
        range: [oop, ip],
        flop: [board[0], board[1], board[2]],
        turn: board.get(3).copied().unwrap_or(NOT_DEALT),
        river: board.get(4).copied().unwrap_or(NOT_DEALT),
    };
    let t = &spot.tree;
    let tree_config = TreeConfig {
        initial_state: match spot.initial_street {
            Street::Flop => BoardState::Flop,
            Street::Turn => BoardState::Turn,
            Street::River => BoardState::River,
        },
        starting_pot: spot.starting_pot_chips,
        effective_stack: spot.effective_stack_chips,
        rake_rate: spot.rake.rate,
        rake_cap: spot.rake.cap_chips,
        flop_bet_sizes: [
            sizes(&t.oop.flop.bet, &t.oop.flop.raise)?,
            sizes(&t.ip.flop.bet, &t.ip.flop.raise)?,
        ],
        turn_bet_sizes: [
            sizes(&t.oop.turn.bet, &t.oop.turn.raise)?,
            sizes(&t.ip.turn.bet, &t.ip.turn.raise)?,
        ],
        river_bet_sizes: [
            sizes(&t.oop.river.bet, &t.oop.river.raise)?,
            sizes(&t.ip.river.bet, &t.ip.river.raise)?,
        ],
        turn_donk_sizes: donk(&t.turn_donk)?,
        river_donk_sizes: donk(&t.river_donk)?,
        add_allin_threshold: t.add_allin_threshold,
        force_allin_threshold: t.force_allin_threshold,
        merging_threshold: t.merging_threshold,
    };
    let tree = ActionTree::new(tree_config).map_err(AdapterError::Config)?;
    PostFlopGame::with_config(card_config, tree).map_err(AdapterError::Config)
}

#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub iteration: u32,
    pub max_iterations: u32,
    pub exploitability_chips: f32,
}

pub struct SolvedGame {
    pub game: PostFlopGame,
    pub info: SolveInfo,
}

/// Allocates memory (uncompressed if it fits, else compressed) and runs Discounted CFR with
/// `solve_step`, checking exploitability every 10 iterations, reporting progress and honoring
/// `cancel`. A cancelled solve returns `Err(Cancelled)` and nothing is published.
pub fn solve_spot(
    spot: &SpotConfig,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(Progress),
) -> Result<SolvedGame, AdapterError> {
    let mut game = build_game(spot)?;
    let (mem, mem_c) = game.memory_usage();
    let limit = spot.solve.memory_limit_mib * 1024 * 1024;
    let compressed = if mem <= limit {
        false
    } else if mem_c <= limit {
        true
    } else {
        return Err(AdapterError::OutOfMemory {
            estimate_mib: mem_c >> 20,
            limit_mib: spot.solve.memory_limit_mib,
        });
    };
    game.allocate_memory(compressed);

    let max_it = spot.solve.max_iterations;
    let target =
        spot.starting_pot_chips as f32 * (spot.solve.target_exploitability_pct as f32 / 100.0);
    let start = Instant::now();
    let mut expl = compute_exploitability(&game);
    on_progress(Progress {
        iteration: 0,
        max_iterations: max_it,
        exploitability_chips: expl,
    });
    let mut it = 0;
    while it < max_it && expl > target {
        if cancel.load(Ordering::Relaxed) {
            return Err(AdapterError::Cancelled);
        }
        solve_step(&game, it);
        it += 1;
        if it % 10 == 0 || it == max_it {
            expl = compute_exploitability(&game);
            on_progress(Progress {
                iteration: it,
                max_iterations: max_it,
                exploitability_chips: expl,
            });
        }
    }
    finalize(&mut game);
    let solve_ms = start.elapsed().as_millis() as u64;
    let info = SolveInfo {
        method: "Discounted CFR (solve_step + finalize)".into(),
        max_iterations: max_it,
        target_exploitability_chips: target as f64,
        final_exploitability_chips: expl as f64,
        final_exploitability_pct_pot: 100.0 * expl as f64 / spot.starting_pot_chips as f64,
        compressed,
        memory_estimate_bytes: if compressed { mem_c } else { mem },
        solve_ms,
        threads: std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1),
        status: if expl <= target {
            SolveStatus::Converged
        } else {
            SolveStatus::IterationLimit
        },
    };
    Ok(SolvedGame { game, info })
}

fn seat_position(spot: &SpotConfig, player: usize) -> String {
    if player == 0 {
        spot.oop_position.clone()
    } else {
        spot.ip_position.clone()
    }
}

fn street_of(board_len: usize) -> Street {
    match board_len {
        3 => Street::Flop,
        4 => Street::Turn,
        _ => Street::River,
    }
}

/// Converts engine actions at the current node into labeled actions.
pub fn describe_actions(game: &PostFlopGame, spot: &SpotConfig) -> Vec<ActionInfo> {
    let tb = game.total_bet_amount();
    let pot = spot.starting_pot_chips + tb[0] + tb[1];
    let cpb = spot.chips_per_bb;
    game.available_actions()
        .into_iter()
        .map(|a| {
            let (kind, amount, label) = match a {
                Action::Fold => (ActionKind::Fold, 0, "Desistir".to_string()),
                Action::Check => (ActionKind::Check, 0, "Passar (check)".to_string()),
                Action::Call => {
                    let to_call = (tb[0] - tb[1]).abs();
                    (
                        ActionKind::Call,
                        0,
                        format!("Pagar {} bb", fmt_bb(to_call as f64, cpb)),
                    )
                }
                Action::Bet(x) => {
                    let pct = 100.0 * x as f64 / pot as f64;
                    (
                        ActionKind::Bet,
                        x,
                        format!("Apostar {} bb ({:.0}% do pote)", fmt_bb(x as f64, cpb), pct),
                    )
                }
                Action::Raise(x) => (
                    ActionKind::Raise,
                    x,
                    format!("Aumentar para {} bb", fmt_bb(x as f64, cpb)),
                ),
                Action::AllIn(x) => (
                    ActionKind::AllIn,
                    x,
                    format!("All-in ({} bb)", fmt_bb(x as f64, cpb)),
                ),
                Action::None | Action::Chance(_) => (ActionKind::Check, 0, format!("{a:?}")),
            };
            ActionInfo {
                kind,
                amount_chips: amount,
                label,
            }
        })
        .collect()
}

fn action_code(a: &ActionInfo) -> String {
    match a.kind {
        ActionKind::Fold => "f".into(),
        ActionKind::Check => "x".into(),
        ActionKind::Call => "c".into(),
        ActionKind::Bet => format!("b{}", a.amount_chips),
        ActionKind::Raise => format!("r{}", a.amount_chips),
        ActionKind::AllIn => format!("a{}", a.amount_chips),
    }
}

fn line_label(position: &str, a: &ActionInfo, cpb: i32) -> String {
    match a.kind {
        ActionKind::Fold => format!("{position} desiste"),
        ActionKind::Check => format!("{position} passa"),
        ActionKind::Call => format!("{position} paga"),
        ActionKind::Bet => format!(
            "{position} aposta {} bb",
            fmt_bb(a.amount_chips as f64, cpb)
        ),
        ActionKind::Raise => format!(
            "{position} aumenta para {} bb",
            fmt_bb(a.amount_chips as f64, cpb)
        ),
        ActionKind::AllIn => format!(
            "{position} all-in {} bb",
            fmt_bb(a.amount_chips as f64, cpb)
        ),
    }
}

/// Lists decision nodes reachable from the root without crossing a chance node, up to `max_depth`
/// actions. Returns (history, node_id, readable line).
pub fn enumerate_decision_nodes(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    max_depth: usize,
) -> Vec<(Vec<usize>, String, Vec<String>)> {
    let mut out = Vec::new();
    let mut stack: Vec<(Vec<usize>, Vec<String>, Vec<String>)> = vec![(vec![], vec![], vec![])];
    while let Some((hist, codes, line)) = stack.pop() {
        game.apply_history(&hist);
        if game.is_terminal_node() || game.is_chance_node() {
            continue;
        }
        let id = if codes.is_empty() {
            "root".to_string()
        } else {
            codes.join("-")
        };
        out.push((hist.clone(), id, line.clone()));
        if hist.len() >= max_depth {
            continue;
        }
        let player = game.current_player();
        let pos = seat_position(spot, player);
        let actions = describe_actions(game, spot);
        for (i, a) in actions.iter().enumerate().rev() {
            let mut h = hist.clone();
            h.push(i);
            let mut c = codes.clone();
            c.push(action_code(a));
            let mut l = line.clone();
            l.push(line_label(&pos, a, spot.chips_per_bb));
            stack.push((h, c, l));
        }
    }
    game.back_to_root();
    out.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)));
    out
}

/// Exports the decision node at `history`: eligible combos (non-zero normalized weight and not
/// blocked by the board), strategy and EV per action, equity.
pub fn extract_node(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    history: &[usize],
    node_id: &str,
    line: Vec<String>,
    root_mass: f64,
) -> Result<NodeSolution, AdapterError> {
    game.apply_history(history);
    if game.is_terminal_node() || game.is_chance_node() {
        return Err(AdapterError::Navigation(format!(
            "nó {node_id} não é decisão"
        )));
    }
    game.cache_normalized_weights();
    let player = game.current_player();
    let board_cards = game.current_board();
    let board_mask = board_cards.iter().fold(0u64, |m, &c| m | (1u64 << c));
    let cards = game.private_cards(player).to_vec();
    let nw = game.normalized_weights(player).to_vec();
    let strategy = game.strategy();
    let evd = game.expected_values_detail(player);
    let equity = game.equity(player);
    let actions = describe_actions(game, spot);
    let n = cards.len();
    let a = actions.len();
    assert_eq!(strategy.len(), a * n, "strategy layout");
    assert_eq!(evd.len(), a * n, "EV layout");

    let mut combos = Vec::new();
    let mut weights = Vec::new();
    let mut eq = Vec::new();
    let mut strat_rows = vec![Vec::new(); a];
    let mut ev_rows = vec![Vec::new(); a];
    for (i, &(c1, c2)) in cards.iter().enumerate() {
        let combo = Combo::new(c1, c2)?;
        if nw[i] <= 0.0 || combo.is_blocked_by(board_mask) {
            continue;
        }
        combos.push(combo.to_string());
        weights.push(nw[i]);
        eq.push(round_freq(equity[i]));
        for j in 0..a {
            strat_rows[j].push(round_freq(strategy[j * n + i]));
            ev_rows[j].push(round_ev(evd[j * n + i]));
        }
    }
    let mass: f64 = nw.iter().map(|&w| w as f64).sum();
    let tb = game.total_bet_amount();
    let node = NodeSolution {
        node_id: node_id.to_string(),
        history: history.to_vec(),
        line,
        street: street_of(board_cards.len()),
        board: board_cards.iter().map(|&c| card_to_string(c)).collect(),
        player: if player == 0 { Seat::Oop } else { Seat::Ip },
        position: seat_position(spot, player),
        pot_chips: spot.starting_pot_chips + tb[0] + tb[1],
        to_call_chips: (tb[player ^ 1] - tb[player]).max(0),
        stack_chips: spot.effective_stack_chips - tb[player],
        actions,
        combos,
        weights,
        strategy: strat_rows,
        ev: ev_rows,
        equity: eq,
        reach: if root_mass > 0.0 {
            (mass / root_mass) as f32
        } else {
            0.0
        },
    };
    game.back_to_root();
    Ok(node)
}

/// Builds the versioned solution file from a solved game and a list of nodes to export.
pub fn build_solution_file(
    solved: &mut SolvedGame,
    spot: &SpotConfig,
    max_depth: usize,
) -> Result<SolutionFile, AdapterError> {
    let game = &mut solved.game;
    game.back_to_root();
    let root_actions = describe_actions(game, spot);
    let root_mass = joint_mass(game, &[]);
    let list = enumerate_decision_nodes(game, spot, max_depth);
    let mut nodes = Vec::with_capacity(list.len());
    for (hist, id, line) in list {
        nodes.push(extract_node(game, spot, &hist, &id, line, root_mass)?);
    }
    let file = SolutionFile {
        schema_version: SOLUTION_SCHEMA_VERSION,
        solution_id: format!("{}-v{}", spot.id, spot.version),
        config_hash: spot.config_hash(ENGINE_REV),
        created_at: now_iso8601(),
        engine: engine_info(),
        spot: spot.clone(),
        solve: solved.info.clone(),
        root_actions,
        nodes,
    };
    file.validate()?;
    Ok(file)
}

/// Joint reach mass at a node (sum of the current player's normalized weights).
pub fn joint_mass(game: &mut PostFlopGame, history: &[usize]) -> f64 {
    game.apply_history(history);
    game.cache_normalized_weights();
    let p = if game.is_terminal_node() || game.is_chance_node() {
        0
    } else {
        game.current_player()
    };
    let m = game.normalized_weights(p).iter().map(|&w| w as f64).sum();
    game.back_to_root();
    m
}

/// Full-precision values read directly from the live engine, used to check that a saved and
/// reloaded solution reproduces the same decision.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct Probe {
    pub solution_id: String,
    pub node_id: String,
    pub combo: String,
    pub actions: Vec<String>,
    pub frequencies: Vec<f32>,
    pub evs_chips: Vec<f32>,
}

pub fn probe_live(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    solution_id: &str,
    history: &[usize],
    node_id: &str,
    combo: &Combo,
) -> Result<Probe, AdapterError> {
    game.apply_history(history);
    game.cache_normalized_weights();
    let p = game.current_player();
    let cards = game.private_cards(p).to_vec();
    let idx = cards
        .iter()
        .position(|&(a, b)| Combo::new(a, b).map(|c| c == *combo).unwrap_or(false))
        .ok_or_else(|| {
            AdapterError::Navigation(format!(
                "combo {combo} fora do range de {}",
                seat_position(spot, p)
            ))
        })?;
    let n = cards.len();
    let strategy = game.strategy();
    let evd = game.expected_values_detail(p);
    let actions = describe_actions(game, spot);
    let probe = Probe {
        solution_id: solution_id.to_string(),
        node_id: node_id.to_string(),
        combo: combo.to_string(),
        actions: actions.iter().map(|a| a.label.clone()).collect(),
        frequencies: (0..actions.len()).map(|j| strategy[j * n + idx]).collect(),
        evs_chips: (0..actions.len()).map(|j| evd[j * n + idx]).collect(),
    };
    game.back_to_root();
    Ok(probe)
}

/// Raw engine view for tests: (combo strings incl. blocked, normalized weights) at a node.
pub fn raw_private_hands(
    game: &mut PostFlopGame,
    history: &[usize],
) -> (Vec<String>, Vec<f32>, Vec<f32>) {
    game.apply_history(history);
    game.cache_normalized_weights();
    let p = game.current_player();
    let cards: Vec<String> = game
        .private_cards(p)
        .iter()
        .map(|&(a, b)| Combo::new(a, b).unwrap().to_string())
        .collect();
    let raw = game.weights(p).to_vec();
    let nw = game.normalized_weights(p).to_vec();
    game.back_to_root();
    (cards, raw, nw)
}
