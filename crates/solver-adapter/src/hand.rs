//! Full-hand simulation against the engine policy (flop to river) on a solved game.
//!
//! - Private hands are drawn jointly from the initial ranges: each player's combo by its weight,
//!   rejecting pairs that share a card or touch the board, so the joint draw is proportional to
//!   `w_hero * w_villain` over compatible pairs. Both hands stay fixed until the end.
//! - The villain acts by sampling the engine strategy of *its* combo at the current node.
//! - Turn and river are dealt uniformly from the cards not on the board and not in either hand
//!   (no replacement).
//! - Each hero decision is graded with the same EV-loss rule as the drills (same node, same combo).
//!   Once the hero's own line has (practically) zero weight for the hero combo, the engine's
//!   strategy beyond that point is not reliable for that hand, so grading stops at the last
//!   evaluable decision and the reason is recorded.
//! - The pot is settled with the engine's accounting: pot = starting pot + 2 x matched amount,
//!   rake = min(pot x rate, cap) taken from the winner (also on a fold), unmatched chips returned.

use poker_core::cards::{card_to_string, cards_mask, Card, Combo};
use poker_core::eval::{evaluate, Feedback, Tolerance};
use poker_core::handrank;
use poker_core::sampling::{weighted_index, Rng};
use poker_core::solution::{ActionInfo, ActionKind};
use poker_core::spot::{SpotConfig, Street};
use postflop_solver::PostFlopGame;
use serde::{Deserialize, Serialize};

use crate::{
    action_code, describe_actions, extract_node_with_runout, line_label, seat_position,
    AdapterError,
};

/// Below this fraction of its initial weight, the hero combo is treated as having zero weight on
/// the line (the same resolution at which pack frequencies are stored).
pub const MIN_HERO_REACH: f64 = 1e-5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    Action,
    Card,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HandEvent {
    pub street: Street,
    pub kind: EventKind,
    /// Acting position (actions only).
    pub position: Option<String>,
    pub hero: bool,
    pub label: String,
    /// Pot after the event, chips.
    pub pot_chips: i32,
}

/// The decision the hero faces now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct HeroTurn {
    pub node_id: String,
    pub street: Street,
    pub board: Vec<String>,
    pub pot_chips: i32,
    pub to_call_chips: i32,
    pub stack_chips: i32,
    pub actions: Vec<ActionInfo>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Decision {
    pub index: usize,
    pub turn: HeroTurn,
    /// Readable line before the decision (same format as exported nodes).
    pub line: Vec<String>,
    pub chosen: usize,
    pub chosen_label: String,
    /// Grading; `None` when the decision is not evaluable.
    pub feedback: Option<Feedback>,
    /// Hero equity against the villain's range at the node (engine), when evaluable.
    pub equity: Option<f32>,
    pub not_evaluable: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Winner {
    Hero,
    Villain,
    Split,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Outcome {
    pub showdown: bool,
    /// Position that folded, if any.
    pub folded: Option<String>,
    pub winner: Winner,
    /// Pot contested (starting pot + both matched contributions), before rake.
    pub pot_chips: i32,
    pub rake_chips: f64,
    /// Hero chips put in after the flop (matched part; unmatched chips are returned).
    pub hero_invested_chips: i32,
    /// Postflop result: chips received from the pot minus `hero_invested_chips`.
    pub hero_net_chips: f64,
    pub hero_hand: Option<String>,
    pub villain_hand: Option<String>,
    pub board: Vec<String>,
}

/// What the villain policy sees: the actions and the engine frequencies of the villain combo.
pub struct VillainChoice<'a> {
    pub actions: &'a [ActionInfo],
    pub probs: &'a [f32],
}

/// Default policy: sample the engine strategy of the villain combo.
pub fn engine_policy(c: &VillainChoice, rng: &mut Rng) -> usize {
    weighted_index(c.probs, rng).unwrap_or(0)
}

/// (feedback, hero equity, reason it is not evaluable).
type Grading = (Option<Feedback>, Option<f32>, Option<String>);

#[derive(Debug, Clone)]
pub struct HandSim {
    /// Engine player index of the hero (0 = OOP, 1 = IP).
    pub hero: usize,
    pub hero_combo: Combo,
    pub villain_combo: Combo,
    pub history: Vec<usize>,
    pub events: Vec<HandEvent>,
    pub decisions: Vec<Decision>,
    pub outcome: Option<Outcome>,
    /// Why grading stopped (zero-weight line), if it did.
    pub cut: Option<String>,
    pub initial_board: Vec<String>,
    hero_w0: f32,
    tokens: Vec<(bool, String)>,
    labels: Vec<String>,
    last: Option<(usize, ActionKind)>,
}

fn combo_mask(c: &Combo) -> u64 {
    cards_mask(&[c.0, c.1])
}

fn combo_index(game: &PostFlopGame, player: usize, combo: &Combo) -> Option<usize> {
    game.private_cards(player)
        .iter()
        .position(|&(a, b)| Combo::new(a, b).map(|c| c == *combo).unwrap_or(false))
}

fn street_of(board_len: usize) -> Street {
    match board_len {
        3 => Street::Flop,
        4 => Street::Turn,
        _ => Street::River,
    }
}

impl HandSim {
    /// Deals both private hands from the root ranges. `hero` is the engine player index.
    pub fn deal(game: &mut PostFlopGame, hero: usize, rng: &mut Rng) -> Result<Self, AdapterError> {
        if hero > 1 {
            return Err(AdapterError::Config(format!("jogador inválido: {hero}")));
        }
        game.back_to_root();
        let board = game.current_board();
        let board_mask = cards_mask(&board);
        let candidates = |p: usize| -> Vec<(Combo, f32)> {
            game.private_cards(p)
                .iter()
                .zip(game.weights(p))
                .filter_map(|(&(a, b), &w)| {
                    let c = Combo::new(a, b).ok()?;
                    (w > 0.0 && !c.is_blocked_by(board_mask)).then_some((c, w))
                })
                .collect()
        };
        let hc = candidates(hero);
        let vc = candidates(hero ^ 1);
        if hc.is_empty() || vc.is_empty() {
            return Err(AdapterError::Config(
                "range vazio depois de remover as cartas do board".into(),
            ));
        }
        let hw: Vec<f32> = hc.iter().map(|x| x.1).collect();
        let vw: Vec<f32> = vc.iter().map(|x| x.1).collect();
        // Rejection sampling: exact joint distribution over compatible pairs.
        for _ in 0..200_000 {
            let h = hc[weighted_index(&hw, rng).expect("pesos positivos")];
            let v = vc[weighted_index(&vw, rng).expect("pesos positivos")];
            if combo_mask(&h.0) & combo_mask(&v.0) == 0 {
                return Ok(HandSim {
                    hero,
                    hero_combo: h.0,
                    villain_combo: v.0,
                    history: Vec::new(),
                    events: Vec::new(),
                    decisions: Vec::new(),
                    outcome: None,
                    cut: None,
                    initial_board: board.iter().map(|&c| card_to_string(c)).collect(),
                    hero_w0: h.1,
                    tokens: Vec::new(),
                    labels: Vec::new(),
                    last: None,
                });
            }
        }
        Err(AdapterError::Config(
            "ranges incompatíveis: nenhuma combinação dos dois jogadores sem cartas em comum"
                .into(),
        ))
    }

    pub fn villain(&self) -> usize {
        self.hero ^ 1
    }

    pub fn is_finished(&self) -> bool {
        self.outcome.is_some()
    }

    /// Readable line so far (exported-node format: actions, plus "Turn: ..." / "River: ...").
    pub fn line(&self) -> &[String] {
        &self.labels
    }

    /// Node id in the pack format, e.g. `x-b182-c|Th|x`.
    pub fn node_id(&self) -> String {
        if self.tokens.is_empty() {
            return "root".into();
        }
        let mut s = String::new();
        let mut prev_card = false;
        for (i, (is_card, t)) in self.tokens.iter().enumerate() {
            if i > 0 {
                s.push(if *is_card || prev_card { '|' } else { '-' });
            }
            s.push_str(t);
            prev_card = *is_card;
        }
        s
    }

    fn pot(game: &PostFlopGame, spot: &SpotConfig) -> i32 {
        let tb = game.total_bet_amount();
        spot.starting_pot_chips + tb[0] + tb[1]
    }

    /// Plays villain actions and deals cards until the hero must act (returns the decision) or
    /// the hand ends (returns `None`, `outcome` set).
    pub fn advance(
        &mut self,
        game: &mut PostFlopGame,
        spot: &SpotConfig,
        rng: &mut Rng,
        policy: &mut dyn FnMut(&VillainChoice, &mut Rng) -> usize,
    ) -> Result<Option<HeroTurn>, AdapterError> {
        if self.outcome.is_some() {
            return Ok(None);
        }
        loop {
            game.apply_history(&self.history);
            if game.is_terminal_node() {
                self.settle(game, spot, rng);
                game.back_to_root();
                return Ok(None);
            }
            if game.is_chance_node() {
                let dead = combo_mask(&self.hero_combo) | combo_mask(&self.villain_combo);
                let avail = game.possible_cards() & !dead;
                let cards: Vec<Card> = (0..52u8).filter(|&c| avail & (1u64 << c) != 0).collect();
                if cards.is_empty() {
                    return Err(AdapterError::Navigation("nenhuma carta disponível".into()));
                }
                let card = cards[rng.below(cards.len())];
                let before = game.current_board();
                let street = if before.len() == 3 { "Turn" } else { "River" };
                let label = format!(
                    "{street}: {}",
                    poker_core::runout::describe_dealt(card, &before)
                );
                self.history.push(card as usize);
                self.tokens.push((true, card_to_string(card)));
                self.labels.push(label.clone());
                self.events.push(HandEvent {
                    street: street_of(before.len() + 1),
                    kind: EventKind::Card,
                    position: None,
                    hero: false,
                    label,
                    pot_chips: Self::pot(game, spot),
                });
                continue;
            }
            let p = game.current_player();
            if p == self.hero {
                let turn = self.turn_view(game, spot);
                game.back_to_root();
                return Ok(Some(turn));
            }
            let idx = combo_index(game, p, &self.villain_combo).ok_or_else(|| {
                AdapterError::Navigation(format!(
                    "combo {} fora do range do adversário",
                    self.villain_combo
                ))
            })?;
            let n = game.private_cards(p).len();
            let strategy = game.strategy();
            let actions = describe_actions(game, spot);
            let probs: Vec<f32> = (0..actions.len()).map(|j| strategy[j * n + idx]).collect();
            let choice = policy(
                &VillainChoice {
                    actions: &actions,
                    probs: &probs,
                },
                rng,
            );
            if choice >= actions.len() {
                return Err(AdapterError::Navigation(format!(
                    "ação {choice} inexistente"
                )));
            }
            self.push_action(game, spot, p, choice, &actions);
        }
    }

    fn turn_view(&self, game: &PostFlopGame, spot: &SpotConfig) -> HeroTurn {
        let tb = game.total_bet_amount();
        let board = game.current_board();
        HeroTurn {
            node_id: self.node_id(),
            street: street_of(board.len()),
            board: board.iter().map(|&c| card_to_string(c)).collect(),
            pot_chips: spot.starting_pot_chips + tb[0] + tb[1],
            to_call_chips: (tb[self.villain()] - tb[self.hero]).max(0),
            stack_chips: spot.effective_stack_chips - tb[self.hero],
            actions: describe_actions(game, spot),
        }
    }

    /// The hero decision pending now, if any (does not advance the hand).
    pub fn current_turn(&self, game: &mut PostFlopGame, spot: &SpotConfig) -> Option<HeroTurn> {
        if self.outcome.is_some() {
            return None;
        }
        game.apply_history(&self.history);
        let t = (!game.is_terminal_node()
            && !game.is_chance_node()
            && game.current_player() == self.hero)
            .then(|| self.turn_view(game, spot));
        game.back_to_root();
        t
    }

    fn push_action(
        &mut self,
        game: &mut PostFlopGame,
        spot: &SpotConfig,
        player: usize,
        idx: usize,
        actions: &[ActionInfo],
    ) {
        let a = &actions[idx];
        let pos = seat_position(spot, player);
        let label = line_label(&pos, a, spot.chips_per_bb);
        let street = street_of(game.current_board().len());
        self.labels.push(label.clone());
        self.tokens.push((false, action_code(a)));
        self.history.push(idx);
        self.last = Some((player, a.kind));
        game.apply_history(&self.history);
        self.events.push(HandEvent {
            street,
            kind: EventKind::Action,
            position: Some(pos),
            hero: player == self.hero,
            label,
            pot_chips: Self::pot(game, spot),
        });
    }

    /// Applies the hero's choice at the pending decision and grades it.
    pub fn act(
        &mut self,
        game: &mut PostFlopGame,
        spot: &SpotConfig,
        choice: usize,
        tol: Tolerance,
    ) -> Result<&Decision, AdapterError> {
        let turn = self
            .current_turn(game, spot)
            .ok_or_else(|| AdapterError::Navigation("não é a vez do aluno".into()))?;
        if choice >= turn.actions.len() {
            return Err(AdapterError::Navigation(format!(
                "ação {choice} inexistente"
            )));
        }
        let line = self.labels.clone();
        let (feedback, equity, not_evaluable) = self.grade(game, spot, &turn, choice, tol)?;
        game.apply_history(&self.history);
        let actions = turn.actions.clone();
        self.push_action(game, spot, self.hero, choice, &actions);
        game.back_to_root();
        self.decisions.push(Decision {
            index: self.decisions.len(),
            chosen_label: actions[choice].label.clone(),
            turn,
            line,
            chosen: choice,
            feedback,
            equity,
            not_evaluable,
        });
        Ok(self.decisions.last().expect("acabou de inserir"))
    }

    fn grade(
        &mut self,
        game: &mut PostFlopGame,
        spot: &SpotConfig,
        turn: &HeroTurn,
        choice: usize,
        tol: Tolerance,
    ) -> Result<Grading, AdapterError> {
        if let Some(reason) = &self.cut {
            return Ok((None, None, Some(reason.clone())));
        }
        game.apply_history(&self.history);
        let idx = combo_index(game, self.hero, &self.hero_combo).ok_or_else(|| {
            AdapterError::Navigation(format!("combo {} fora do range", self.hero_combo))
        })?;
        let w = game.weights(self.hero)[idx];
        game.back_to_root();
        let reach = if self.hero_w0 > 0.0 {
            w as f64 / self.hero_w0 as f64
        } else {
            0.0
        };
        if reach < MIN_HERO_REACH {
            let reason = format!(
                "Na solução, {} praticamente nunca chega a este ponto pela linha que você jogou \
                 (frequência acumulada {:.6}%). O motor não tem estratégia confiável para esta \
                 mão a partir daqui; a nota termina na última decisão avaliável.",
                self.hero_combo,
                reach * 100.0
            );
            self.cut = Some(reason.clone());
            return Ok((None, None, Some(reason)));
        }
        let node = extract_node_with_runout(
            game,
            spot,
            &self.history,
            &turn.node_id,
            self.labels.clone(),
            1.0,
            vec![],
        )?;
        match evaluate(&node, &self.hero_combo, choice, spot.chips_per_bb, tol) {
            Ok(fb) => {
                let eq = node.combo_index(&self.hero_combo).map(|ci| node.equity[ci]);
                Ok((Some(fb), eq, None))
            }
            Err(e) => {
                let reason = format!("{e}. A nota termina na última decisão avaliável.");
                self.cut = Some(reason.clone());
                Ok((None, None, Some(reason)))
            }
        }
    }
}

impl HandSim {
    fn settle(&mut self, game: &PostFlopGame, spot: &SpotConfig, rng: &mut Rng) {
        let tb = game.total_bet_amount();
        let amount = tb[0].min(tb[1]);
        let pot = spot.starting_pot_chips + 2 * amount;
        let rake = (pot as f64 * spot.rake.rate).min(spot.rake.cap_chips);
        let mut board = game.current_board();
        let folded = match self.last {
            Some((p, ActionKind::Fold)) => Some(p),
            _ => None,
        };
        // An all-in called before the river is a terminal node in the engine (it averages over
        // the remaining cards). Deal the rest of the board here, without replacement.
        if folded.is_none() {
            while board.len() < 5 {
                let dead = cards_mask(&board)
                    | combo_mask(&self.hero_combo)
                    | combo_mask(&self.villain_combo);
                let cards: Vec<Card> = (0..52u8).filter(|&c| dead & (1u64 << c) == 0).collect();
                let card = cards[rng.below(cards.len())];
                let street = if board.len() == 3 { "Turn" } else { "River" };
                let label = format!(
                    "{street}: {}",
                    poker_core::runout::describe_dealt(card, &board)
                );
                board.push(card);
                self.tokens.push((true, card_to_string(card)));
                self.labels.push(label.clone());
                self.events.push(HandEvent {
                    street: street_of(board.len()),
                    kind: EventKind::Card,
                    position: None,
                    hero: false,
                    label,
                    pot_chips: spot.starting_pot_chips + tb[0] + tb[1],
                });
            }
        }
        let (winner, hero_hand, villain_hand) = match folded {
            Some(p) if p == self.hero => (Winner::Villain, None, None),
            Some(_) => (Winner::Hero, None, None),
            None => {
                let mut h: Vec<Card> = board.clone();
                h.extend([self.hero_combo.0, self.hero_combo.1]);
                let mut v: Vec<Card> = board.clone();
                v.extend([self.villain_combo.0, self.villain_combo.1]);
                let (hv, vv) = (handrank::evaluate(&h), handrank::evaluate(&v));
                let w = match hv.cmp(&vv) {
                    std::cmp::Ordering::Greater => Winner::Hero,
                    std::cmp::Ordering::Less => Winner::Villain,
                    std::cmp::Ordering::Equal => Winner::Split,
                };
                (w, Some(hv.describe_pt()), Some(vv.describe_pt()))
            }
        };
        let received = match winner {
            Winner::Hero => pot as f64 - rake,
            Winner::Villain => 0.0,
            Winner::Split => (pot as f64 - rake) / 2.0,
        };
        self.outcome = Some(Outcome {
            showdown: folded.is_none(),
            folded: folded.map(|p| seat_position(spot, p)),
            winner,
            pot_chips: pot,
            rake_chips: rake,
            hero_invested_chips: amount,
            hero_net_chips: received - amount as f64,
            hero_hand,
            villain_hand,
            board: board.iter().map(|&c| card_to_string(c)).collect(),
        });
    }
}
