//! Deterministic choice of representative turn/river cards for exported drills.
//!
//! Exporting every runout of a flop tree is too large for the installer, so each pack exports a
//! small set of cards that cover the main categories (flush card, overcard, pairing card, low
//! card, middle card). Drills started on the turn/river use the real ranges of the selected line
//! for that card; the choice of cards itself is a coverage decision, recorded in the exercise tags.

use crate::cards::{card_to_string, rank_of, suit_of, Card};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunoutKind {
    /// Brings a third (or more) card of one suit to the board.
    Flush,
    /// Higher than every board card.
    Over,
    /// Pairs the highest board rank.
    PairTop,
    /// Lower than every board card.
    Low,
    /// Between the lowest and highest board ranks, not pairing.
    Middle,
    /// Any other card used to complete the requested count.
    Other,
}

impl RunoutKind {
    pub fn tag(self) -> &'static str {
        match self {
            RunoutKind::Flush => "flush",
            RunoutKind::Over => "over",
            RunoutKind::PairTop => "pair",
            RunoutKind::Low => "low",
            RunoutKind::Middle => "mid",
            RunoutKind::Other => "other",
        }
    }
    pub fn label_pt(self) -> &'static str {
        match self {
            RunoutKind::Flush => "carta do naipe do board",
            RunoutKind::Over => "overcard",
            RunoutKind::PairTop => "pareia a carta alta",
            RunoutKind::Low => "carta baixa",
            RunoutKind::Middle => "carta do meio",
            RunoutKind::Other => "outra",
        }
    }
}

fn suit_count(board: &[Card], suit: u8) -> usize {
    board.iter().filter(|&&c| suit_of(c) == suit).count()
}

fn classify(board: &[Card], c: Card) -> (bool, bool, bool, bool, bool) {
    let r = rank_of(c);
    let max = board.iter().map(|&b| rank_of(b)).max().unwrap_or(0);
    let min = board.iter().map(|&b| rank_of(b)).min().unwrap_or(0);
    let pairs = board.iter().any(|&b| rank_of(b) == r);
    let flush = suit_count(board, suit_of(c)) >= 2;
    (
        flush,
        r > max,
        pairs && r == max,
        r < min,
        r > min && r < max && !pairs,
    )
}

/// Picks up to `n` cards not in `dead` (board + any excluded cards), one per category first,
/// then spreads the remaining picks over ranks. Non-flush categories prefer suits that do not
/// add to a flush, so the categories stay distinct.
pub fn select_runout_cards(board: &[Card], dead: u64, n: usize) -> Vec<(Card, RunoutKind)> {
    let avail: Vec<Card> = (0..52u8).filter(|&c| dead & (1u64 << c) == 0).collect();
    let mut out: Vec<(Card, RunoutKind)> = Vec::new();
    let taken = |out: &Vec<(Card, RunoutKind)>, c: Card| out.iter().any(|&(x, _)| x == c);
    let push = |out: &mut Vec<(Card, RunoutKind)>, pick: Option<Card>, k: RunoutKind| {
        if let Some(c) = pick {
            if out.len() < n && !out.iter().any(|&(x, _)| x == c) {
                out.push((c, k));
            }
        }
    };
    let max = board.iter().map(|&b| rank_of(b)).max().unwrap_or(0) as i32;
    let min = board.iter().map(|&b| rank_of(b)).min().unwrap_or(0) as i32;
    let mid = (max + min) as f64 / 2.0;
    let non_flush = |c: &Card| !classify(board, *c).0;

    // Flush card: middle-ranked card of the most common board suit.
    let flush: Vec<Card> = avail
        .iter()
        .copied()
        .filter(|c| classify(board, *c).0)
        .collect();
    let fpick = flush.iter().copied().min_by(|a, b| {
        ((rank_of(*a) as f64 - mid).abs())
            .total_cmp(&(rank_of(*b) as f64 - mid).abs())
            .then(a.cmp(b))
    });
    push(&mut out, fpick, RunoutKind::Flush);
    // Overcard: the highest available rank above the board.
    let over = avail
        .iter()
        .copied()
        .filter(|c| classify(board, *c).1 && non_flush(c))
        .max_by_key(|&c| (rank_of(c), c));
    push(&mut out, over, RunoutKind::Over);
    // Pairs the top card.
    let pair = avail
        .iter()
        .copied()
        .filter(|c| classify(board, *c).2 && non_flush(c))
        .max();
    push(&mut out, pair, RunoutKind::PairTop);
    // Low card: the lowest available rank under the board.
    let low = avail
        .iter()
        .copied()
        .filter(|c| classify(board, *c).3 && non_flush(c))
        .min_by_key(|&c| (rank_of(c), c));
    push(&mut out, low, RunoutKind::Low);
    // Middle card closest to the board's midpoint.
    let midc = avail
        .iter()
        .copied()
        .filter(|c| classify(board, *c).4 && non_flush(c))
        .min_by(|a, b| {
            ((rank_of(*a) as f64 - mid).abs())
                .total_cmp(&(rank_of(*b) as f64 - mid).abs())
                .then(a.cmp(b))
        });
    push(&mut out, midc, RunoutKind::Middle);
    // Fill: spread over ranks from high to low, skipping ranks already used.
    let mut rest: Vec<Card> = avail
        .iter()
        .copied()
        .filter(|&c| !taken(&out, c) && non_flush(&c))
        .collect();
    rest.sort_by_key(|&c| std::cmp::Reverse((rank_of(c), c)));
    let mut used_ranks: Vec<u8> = out.iter().map(|&(c, _)| rank_of(c)).collect();
    for c in rest {
        if out.len() >= n {
            break;
        }
        if !used_ranks.contains(&rank_of(c)) && (rank_of(c) as i32 - max).abs() > 0 {
            used_ranks.push(rank_of(c));
            out.push((c, RunoutKind::Other));
        }
    }
    out
}

/// Readable runout label. For flush cards the label states how many cards of that suit the board
/// now has (3ª, 4ª, 5ª), which depends on the board the card is dealt to.
pub fn describe(c: Card, k: RunoutKind, board: &[Card]) -> String {
    let label = match k {
        RunoutKind::Flush => {
            const SUIT_PT: [&str; 4] = ["paus", "ouros", "copas", "espadas"];
            let n = suit_count(board, suit_of(c)) + 1;
            format!("{n}ª carta de {}", SUIT_PT[suit_of(c) as usize])
        }
        other => other.label_pt().to_string(),
    };
    format!("{} ({label})", card_to_string(c))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::{cards_mask, parse_board, parse_card};

    #[test]
    fn categories_on_two_tone_flop() {
        let b = parse_board("Td9d6h").unwrap();
        let sel = select_runout_cards(&b, cards_mask(&b), 6);
        assert_eq!(sel.len(), 6);
        let kinds: Vec<_> = sel.iter().map(|&(_, k)| k).collect();
        assert_eq!(
            &kinds[..5],
            &[
                RunoutKind::Flush,
                RunoutKind::Over,
                RunoutKind::PairTop,
                RunoutKind::Low,
                RunoutKind::Middle
            ]
        );
        // Flush card is a diamond; non-flush picks are never diamonds.
        assert_eq!(suit_of(sel[0].0), suit_of(parse_card("2d").unwrap()));
        for &(c, k) in &sel[1..] {
            assert!(k == RunoutKind::Flush || suit_of(c) != suit_of(parse_card("2d").unwrap()));
            assert!(!b.contains(&c));
        }
        assert_eq!(rank_of(sel[1].0), 12, "overcard is an ace");
        assert_eq!(rank_of(sel[2].0), rank_of(parse_card("Tc").unwrap()));
        assert_eq!(rank_of(sel[3].0), 0, "low card is a deuce");
    }

    #[test]
    fn deterministic_and_respects_dead_cards() {
        let b = parse_board("Ah7c2d").unwrap();
        let a = select_runout_cards(&b, cards_mask(&b), 5);
        let again = select_runout_cards(&b, cards_mask(&b), 5);
        assert_eq!(a, again);
        // No card above an ace: the "over" slot is skipped, others still found.
        assert!(a.iter().all(|&(_, k)| k != RunoutKind::Over));
        let dead = cards_mask(&b) | (1u64 << a[0].0);
        let b2 = select_runout_cards(&b, dead, 5);
        assert!(b2.iter().all(|&(c, _)| c != a[0].0));
    }

    #[test]
    fn flush_label_counts_suit_on_the_board() {
        let flop = parse_board("Td9d6h").unwrap();
        let turn = parse_board("Td9d6h8d").unwrap();
        let d7 = parse_card("7d").unwrap();
        assert_eq!(
            describe(d7, RunoutKind::Flush, &flop),
            "7d (3ª carta de ouros)"
        );
        assert_eq!(
            describe(d7, RunoutKind::Flush, &turn),
            "7d (4ª carta de ouros)"
        );
        assert_eq!(describe(d7, RunoutKind::Low, &flop), "7d (carta baixa)");
    }

    #[test]
    fn river_selection_on_four_cards() {
        let b = parse_board("Ks7d4c2h").unwrap();
        let sel = select_runout_cards(&b, cards_mask(&b), 4);
        assert_eq!(sel.len(), 4);
        let mut seen = 0u64;
        for &(c, _) in &sel {
            assert_eq!(seen & (1u64 << c), 0);
            seen |= 1u64 << c;
        }
    }
}
