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

/// (history, node id, readable line, runout tags) of an exportable decision node.
pub type NodeRef = (Vec<usize>, String, Vec<String>, Vec<String>);

/// Lists decision nodes reachable from the root without crossing a chance node, up to `max_depth`
/// actions. Returns (history, node_id, readable line).
pub fn enumerate_decision_nodes(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    max_depth: usize,
) -> Vec<(Vec<usize>, String, Vec<String>)> {
    enumerate_from(game, spot, &Base::root(), max_depth)
        .into_iter()
        .map(|(h, id, l, _)| (h, id, l))
        .collect()
}

/// Starting point of an enumeration: a history plus the id/line/runout tags that lead to it.
#[derive(Debug, Clone, Default)]
pub struct Base {
    pub history: Vec<usize>,
    pub id: String,
    pub line: Vec<String>,
    pub runout: Vec<String>,
}

impl Base {
    pub fn root() -> Self {
        Base::default()
    }
}

/// Decision nodes reachable from `base` (a decision node or the first node after a dealt card)
/// without crossing another chance node, up to `max_depth` actions after `base`.
pub fn enumerate_from(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    base: &Base,
    max_depth: usize,
) -> Vec<NodeRef> {
    let mut out = Vec::new();
    // (history, local action codes, readable line, depth after base)
    type Frame = (Vec<usize>, Vec<String>, Vec<String>, usize);
    let mut stack: Vec<Frame> = vec![(base.history.clone(), vec![], base.line.clone(), 0)];
    while let Some((hist, codes, line, depth)) = stack.pop() {
        game.apply_history(&hist);
        if game.is_terminal_node() || game.is_chance_node() {
            continue;
        }
        let local = codes.join("-");
        let id = match (base.id.is_empty(), local.is_empty()) {
            (true, true) => "root".to_string(),
            (true, false) => local,
            (false, true) => base.id.clone(),
            (false, false) => format!("{}|{local}", base.id),
        };
        out.push((hist.clone(), id, line.clone(), base.runout.clone()));
        if depth >= max_depth {
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
            stack.push((h, c, l, depth + 1));
        }
    }
    game.back_to_root();
    out.sort_by(|a, b| a.0.len().cmp(&b.0.len()).then(a.0.cmp(&b.0)));
    out
}

/// One runout stage of an export plan: from the current street, follow each `line` to the chance
/// node, deal `cards` representative cards and export decisions up to `depth` actions. The first
/// `expand` cards of every line continue into `next` (e.g. turn -> river).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct RunoutStage {
    /// Lines in action tokens: x (check), c (call), f (fold), bS / bL (smallest / largest bet),
    /// r (first raise), a (all-in). Example: "x-bS-c".
    pub lines: Vec<String>,
    pub cards: usize,
    pub depth: usize,
    #[serde(default)]
    pub expand: usize,
    #[serde(default)]
    pub next: Option<Box<RunoutStage>>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ExportPlan {
    /// Decision depth on the initial street.
    pub depth: usize,
    #[serde(default)]
    pub runouts: Option<RunoutStage>,
}

impl ExportPlan {
    pub fn initial_only(depth: usize) -> Self {
        ExportPlan {
            depth,
            runouts: None,
        }
    }
}

type Resolved = (Vec<usize>, Vec<String>, Vec<String>);

/// Follows `line` tokens from `base`; returns the extended history, codes and labels.
fn resolve_line(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    base: &[usize],
    line: &str,
) -> Result<Resolved, AdapterError> {
    let mut hist = base.to_vec();
    let mut codes = Vec::new();
    let mut labels = Vec::new();
    for tok in line.split('-').map(str::trim).filter(|t| !t.is_empty()) {
        game.apply_history(&hist);
        if game.is_terminal_node() || game.is_chance_node() {
            game.back_to_root();
            return Err(AdapterError::Navigation(format!(
                "linha '{line}' termina antes de '{tok}'"
            )));
        }
        let player = game.current_player();
        let actions = describe_actions(game, spot);
        let bets: Vec<usize> = (0..actions.len())
            .filter(|&i| actions[i].kind == ActionKind::Bet)
            .collect();
        let idx = match tok {
            "x" => actions.iter().position(|a| a.kind == ActionKind::Check),
            "c" => actions.iter().position(|a| a.kind == ActionKind::Call),
            "f" => actions.iter().position(|a| a.kind == ActionKind::Fold),
            "bS" | "b" => bets.first().copied(),
            "bL" => bets.last().copied(),
            "r" => actions.iter().position(|a| a.kind == ActionKind::Raise),
            "a" => actions.iter().position(|a| a.kind == ActionKind::AllIn),
            _ => None,
        }
        .ok_or_else(|| {
            AdapterError::Navigation(format!("ação '{tok}' indisponível na linha '{line}'"))
        })?;
        labels.push(line_label(
            &seat_position(spot, player),
            &actions[idx],
            spot.chips_per_bb,
        ));
        codes.push(action_code(&actions[idx]));
        hist.push(idx);
    }
    game.back_to_root();
    Ok((hist, codes, labels))
}

fn export_stage(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    base: &Base,
    stage: &RunoutStage,
    out: &mut Vec<NodeRef>,
) -> Result<(), AdapterError> {
    for line in &stage.lines {
        let (hist, codes, labels) = resolve_line(game, spot, &base.history, line)?;
        game.apply_history(&hist);
        if !game.is_chance_node() {
            game.back_to_root();
            return Err(AdapterError::Navigation(format!(
                "linha '{line}' não leva a uma nova carta (fim de mão ou all-in)"
            )));
        }
        let board = game.current_board();
        let possible = game.possible_cards();
        game.back_to_root();
        let street_name = if board.len() == 3 { "Turn" } else { "River" };
        let picks = poker_core::runout::select_runout_cards(&board, !possible, stage.cards);
        let line_id = codes.join("-");
        for (k, &(card, kind)) in picks.iter().enumerate() {
            let mut h = hist.clone();
            h.push(card as usize);
            let id = if base.id.is_empty() {
                format!("{line_id}|{}", card_to_string(card))
            } else if line_id.is_empty() {
                format!("{}|{}", base.id, card_to_string(card))
            } else {
                format!("{}|{line_id}|{}", base.id, card_to_string(card))
            };
            let mut l = base.line.clone();
            l.extend(labels.iter().cloned());
            l.push(format!(
                "{street_name}: {}",
                poker_core::runout::describe(card, kind, &board)
            ));
            let mut r = base.runout.clone();
            r.push(format!("{}:{}", street_name.to_lowercase(), kind.tag()));
            let next_base = Base {
                history: h,
                id,
                line: l,
                runout: r,
            };
            out.extend(enumerate_from(game, spot, &next_base, stage.depth));
            if let Some(next) = &stage.next {
                if k < stage.expand {
                    export_stage(game, spot, &next_base, next, out)?;
                }
            }
        }
    }
    Ok(())
}

/// All nodes selected by the plan: initial street up to `plan.depth`, plus the runout stages.
pub fn plan_nodes(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    plan: &ExportPlan,
) -> Result<Vec<NodeRef>, AdapterError> {
    let root = Base::root();
    let mut out = enumerate_from(game, spot, &root, plan.depth);
    if let Some(stage) = &plan.runouts {
        export_stage(game, spot, &root, stage, &mut out)?;
    }
    game.back_to_root();
    let mut seen = std::collections::HashSet::new();
    out.retain(|n| seen.insert(n.1.clone()));
    Ok(out)
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
    extract_node_with_runout(game, spot, history, node_id, line, root_mass, vec![])
}

/// Same as [`extract_node`], tagging the node with the runout categories used to reach it.
pub fn extract_node_with_runout(
    game: &mut PostFlopGame,
    spot: &SpotConfig,
    history: &[usize],
    node_id: &str,
    line: Vec<String>,
    root_mass: f64,
    runout: Vec<String>,
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
        runout,
    };
    game.back_to_root();
    Ok(node)
}

/// Builds the versioned solution file exporting the initial street up to `max_depth` actions.
pub fn build_solution_file(
    solved: &mut SolvedGame,
    spot: &SpotConfig,
    max_depth: usize,
) -> Result<SolutionFile, AdapterError> {
    build_solution_file_with_plan(solved, spot, &ExportPlan::initial_only(max_depth))
}

/// Builds the versioned solution file from a solved game and an export plan.
pub fn build_solution_file_with_plan(
    solved: &mut SolvedGame,
    spot: &SpotConfig,
    plan: &ExportPlan,
) -> Result<SolutionFile, AdapterError> {
    let game = &mut solved.game;
    game.back_to_root();
    let root_actions = describe_actions(game, spot);
    let root_mass = joint_mass(game, &[]);
    let list = plan_nodes(game, spot, plan)?;
    let mut nodes = Vec::with_capacity(list.len());
    for (hist, id, line, runout) in list {
        let node = extract_node_with_runout(game, spot, &hist, &id, line, root_mass, runout)?;
        // Lines the equilibrium never reaches have no eligible combo: nothing to study or drill.
        if !node.combos.is_empty() {
            nodes.push(node);
        }
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
        export_plan: serde_json::to_string(plan).ok(),
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
