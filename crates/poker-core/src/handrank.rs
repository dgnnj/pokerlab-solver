//! Showdown hand ranking (best five of seven cards), used by the full-hand simulation to settle
//! the pot. Independent from the engine's internal evaluator, which is not public.

use crate::cards::{rank_of, suit_of, Card};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    HighCard = 0,
    Pair = 1,
    TwoPair = 2,
    Trips = 3,
    Straight = 4,
    Flush = 5,
    FullHouse = 6,
    Quads = 7,
    StraightFlush = 8,
}

/// Comparable strength: higher is better, equal means split pot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct HandValue {
    pub category: Category,
    /// Tie-break ranks (0 = deuce .. 12 = ace), most significant first, padded with zeros.
    pub kickers: [u8; 5],
}

const RANK_PT: [&str; 13] = [
    "2", "3", "4", "5", "6", "7", "8", "9", "10", "valete", "dama", "rei", "ás",
];
const RANK_PT_PLURAL: [&str; 13] = [
    "2", "3", "4", "5", "6", "7", "8", "9", "10", "valetes", "damas", "reis", "ases",
];

impl HandValue {
    /// Portuguese description, e.g. "dois pares, reis e setes".
    pub fn describe_pt(&self) -> String {
        let k = &self.kickers;
        let pl = |r: u8| RANK_PT_PLURAL[r as usize];
        let one = |r: u8| RANK_PT[r as usize];
        match self.category {
            Category::HighCard => format!("carta alta, {}", one(k[0])),
            Category::Pair => format!("par de {}", pl(k[0])),
            Category::TwoPair => format!("dois pares, {} e {}", pl(k[0]), pl(k[1])),
            Category::Trips => format!("trinca de {}", pl(k[0])),
            Category::Straight => format!("sequência até {}", one(k[0])),
            Category::Flush => format!("flush, {} alto", one(k[0])),
            Category::FullHouse => {
                format!("full house (trinca de {} e par de {})", pl(k[0]), pl(k[1]))
            }
            Category::Quads => format!("quadra de {}", pl(k[0])),
            Category::StraightFlush if k[0] == 12 => "royal flush".to_string(),
            Category::StraightFlush => format!("straight flush até {}", one(k[0])),
        }
    }
}

/// Highest rank of a five-in-a-row inside `mask` (bit r = rank r present), wheel included.
fn straight_high(mask: u16) -> Option<u8> {
    for high in (4..=12u8).rev() {
        let need = 0b11111u16 << (high - 4);
        if mask & need == need {
            return Some(high);
        }
    }
    // A-2-3-4-5
    let wheel = (1u16 << 12) | 0b1111;
    if mask & wheel == wheel {
        return Some(3);
    }
    None
}

fn top_ranks(mask: u16, n: usize, out: &mut Vec<u8>) {
    for r in (0..13u8).rev() {
        if out.len() >= n {
            break;
        }
        if mask & (1 << r) != 0 {
            out.push(r);
        }
    }
}

fn value(category: Category, ranks: &[u8]) -> HandValue {
    let mut kickers = [0u8; 5];
    for (i, &r) in ranks.iter().take(5).enumerate() {
        kickers[i] = r;
    }
    HandValue { category, kickers }
}

/// Evaluates 5 to 7 distinct cards.
pub fn evaluate(cards: &[Card]) -> HandValue {
    assert!((5..=7).contains(&cards.len()), "5 a 7 cartas");
    let mut counts = [0u8; 13];
    let mut suit_masks = [0u16; 4];
    let mut rank_mask = 0u16;
    for &c in cards {
        let r = rank_of(c);
        counts[r as usize] += 1;
        suit_masks[suit_of(c) as usize] |= 1 << r;
        rank_mask |= 1 << r;
    }

    if let Some(&fm) = suit_masks.iter().find(|m| m.count_ones() >= 5) {
        if let Some(h) = straight_high(fm) {
            return value(Category::StraightFlush, &[h]);
        }
    }

    let ranks_with = |n: u8| -> Vec<u8> {
        (0..13u8)
            .rev()
            .filter(|&r| counts[r as usize] == n)
            .collect()
    };
    let quads = ranks_with(4);
    let trips = ranks_with(3);
    let pairs = ranks_with(2);

    if let Some(&q) = quads.first() {
        let mut v = vec![q];
        top_ranks(rank_mask & !(1 << q), 2, &mut v);
        return value(Category::Quads, &v);
    }
    if let Some(&t) = trips.first() {
        // Second trips counts as the pair of a full house.
        let pair = trips
            .get(1)
            .copied()
            .into_iter()
            .chain(pairs.iter().copied())
            .max();
        if let Some(p) = pair {
            return value(Category::FullHouse, &[t, p]);
        }
    }
    if let Some(&fm) = suit_masks.iter().find(|m| m.count_ones() >= 5) {
        let mut v = Vec::new();
        top_ranks(fm, 5, &mut v);
        return value(Category::Flush, &v);
    }
    if let Some(h) = straight_high(rank_mask) {
        return value(Category::Straight, &[h]);
    }
    if let Some(&t) = trips.first() {
        let mut v = vec![t];
        top_ranks(rank_mask & !(1 << t), 3, &mut v);
        return value(Category::Trips, &v);
    }
    if pairs.len() >= 2 {
        let (p1, p2) = (pairs[0], pairs[1]);
        let mut v = vec![p1, p2];
        top_ranks(rank_mask & !(1 << p1) & !(1 << p2), 3, &mut v);
        return value(Category::TwoPair, &v);
    }
    if let Some(&p) = pairs.first() {
        let mut v = vec![p];
        top_ranks(rank_mask & !(1 << p), 4, &mut v);
        return value(Category::Pair, &v);
    }
    let mut v = Vec::new();
    top_ranks(rank_mask, 5, &mut v);
    value(Category::HighCard, &v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(s: &str) -> HandValue {
        let cleaned: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
        let cards: Vec<Card> = (0..cleaned.len())
            .step_by(2)
            .map(|i| crate::cards::parse_card(&cleaned[i..i + 2]).unwrap())
            .collect();
        evaluate(&cards)
    }

    #[test]
    fn categories() {
        assert_eq!(ev("AsKsQsJsTs2d3c").category, Category::StraightFlush);
        assert_eq!(ev("AsKsQsJsTs2d3c").describe_pt(), "royal flush");
        assert_eq!(ev("5s4s3s2sAs9d9c").kickers[0], 3, "steel wheel");
        assert_eq!(ev("9c9d9h9s2c3d4h").category, Category::Quads);
        assert_eq!(ev("9c9d9hKsKc3d4h").category, Category::FullHouse);
        assert_eq!(
            ev("9c9d9hKsKcKd4h").kickers[..2],
            [11, 7],
            "two trips: KKK99"
        );
        assert_eq!(ev("Ah9h4h2h7h7c7d").category, Category::Flush);
        assert_eq!(ev("Ad2c3h4s5d9c9d").category, Category::Straight);
        assert_eq!(ev("Ad2c3h4s5d9c9d").kickers[0], 3);
        assert_eq!(ev("7c7d7h2s4dJcQd").category, Category::Trips);
        assert_eq!(ev("7c7d2h2s4dJcQd").category, Category::TwoPair);
        assert_eq!(
            ev("7c7d2h2s4d4cQd").kickers[..3],
            [5, 2, 10],
            "best two pairs + Q"
        );
        assert_eq!(ev("7c7d2h3s9dJcQd").category, Category::Pair);
        assert_eq!(ev("7c8d2h3s9dJcQd").category, Category::HighCard);
        assert_eq!(ev("7c7d2h2s4dJcQd").describe_pt(), "dois pares, 7 e 2");
    }

    #[test]
    fn comparisons_and_ties() {
        // Same board plays: split.
        let board = "AhKdQcJsTh";
        assert_eq!(ev(&format!("{board}2c3d")), ev(&format!("{board}4c5d")));
        // Kicker decides.
        assert!(ev("AhAd9c5s2hKcQd") > ev("AhAd9c5s2hJcTd"));
        // Higher straight beats wheel.
        assert!(ev("6d2c3h4s5d9cKd") > ev("Ad2c3h4s5d9cKd"));
        // Flush beats straight, full beats flush.
        assert!(ev("Ah9h4h2h7hKcQd") > ev("Ad2c3h4s5d9cKd"));
        assert!(ev("9c9d9hKsKc3d4h") > ev("Ah9h4h2h7hKcQd"));
    }
}
