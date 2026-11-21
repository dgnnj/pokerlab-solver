//! Card and combo helpers.
//!
//! Card ids follow the same convention as `postflop-solver`: `id = 4 * rank + suit`,
//! with rank `2 = 0 .. A = 12` and suit `c = 0, d = 1, h = 2, s = 3`.
//! Combos are written with the higher card id first (e.g. `AsKh`, `Ac4c`), which is the
//! format produced by the solver's `holes_to_strings`.

use crate::CoreError;

pub type Card = u8;

const RANKS: &[u8; 13] = b"23456789TJQKA";
const SUITS: &[u8; 4] = b"cdhs";

pub fn parse_card(s: &str) -> Result<Card, CoreError> {
    let b = s.trim().as_bytes();
    if b.len() != 2 {
        return Err(CoreError::InvalidCard(s.to_string()));
    }
    let r = RANKS
        .iter()
        .position(|&c| c == b[0].to_ascii_uppercase())
        .ok_or_else(|| CoreError::InvalidCard(s.to_string()))?;
    let su = SUITS
        .iter()
        .position(|&c| c == b[1].to_ascii_lowercase())
        .ok_or_else(|| CoreError::InvalidCard(s.to_string()))?;
    Ok((r * 4 + su) as Card)
}

pub fn card_to_string(c: Card) -> String {
    let r = RANKS[(c / 4) as usize] as char;
    let s = SUITS[(c % 4) as usize] as char;
    format!("{r}{s}")
}

pub fn rank_of(c: Card) -> u8 {
    c / 4
}

pub fn suit_of(c: Card) -> u8 {
    c % 4
}

/// Parses a board written as concatenated cards ("Ks7d4c") or separated by spaces/commas.
pub fn parse_board(s: &str) -> Result<Vec<Card>, CoreError> {
    let cleaned: String = s.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if !cleaned.len().is_multiple_of(2) {
        return Err(CoreError::InvalidBoard(s.to_string()));
    }
    let mut cards = Vec::new();
    for i in (0..cleaned.len()).step_by(2) {
        cards.push(parse_card(&cleaned[i..i + 2])?);
    }
    if !(3..=5).contains(&cards.len()) {
        return Err(CoreError::InvalidBoard(s.to_string()));
    }
    let mut mask = 0u64;
    for &c in &cards {
        if mask & (1 << c) != 0 {
            return Err(CoreError::DuplicateCard(card_to_string(c)));
        }
        mask |= 1 << c;
    }
    Ok(cards)
}

pub fn cards_mask(cards: &[Card]) -> u64 {
    cards.iter().fold(0u64, |m, &c| m | (1u64 << c))
}

/// A two-card private hand, stored with the higher card id first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Combo(pub Card, pub Card);

impl Combo {
    pub fn new(a: Card, b: Card) -> Result<Self, CoreError> {
        if a == b || a >= 52 || b >= 52 {
            return Err(CoreError::InvalidCombo(format!("{a},{b}")));
        }
        Ok(if a > b { Combo(a, b) } else { Combo(b, a) })
    }

    pub fn parse(s: &str) -> Result<Self, CoreError> {
        let t = s.trim();
        if t.len() != 4 {
            return Err(CoreError::InvalidCombo(s.to_string()));
        }
        Combo::new(parse_card(&t[0..2])?, parse_card(&t[2..4])?)
    }

    pub fn mask(&self) -> u64 {
        (1u64 << self.0) | (1u64 << self.1)
    }

    /// True if the combo shares a card with `dead` (board or other dead cards).
    pub fn is_blocked_by(&self, dead: u64) -> bool {
        self.mask() & dead != 0
    }

    /// Hand class in 13x13 grid notation: `AA`, `AKs`, `AKo`.
    pub fn hand_class(&self) -> String {
        let (r1, r2) = (rank_of(self.0), rank_of(self.1));
        let (hi, lo) = if r1 >= r2 { (r1, r2) } else { (r2, r1) };
        let h = RANKS[hi as usize] as char;
        let l = RANKS[lo as usize] as char;
        if hi == lo {
            format!("{h}{l}")
        } else if suit_of(self.0) == suit_of(self.1) {
            format!("{h}{l}s")
        } else {
            format!("{h}{l}o")
        }
    }
}

impl std::fmt::Display for Combo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", card_to_string(self.0), card_to_string(self.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn card_roundtrip_matches_solver_convention() {
        assert_eq!(parse_card("2c").unwrap(), 0);
        assert_eq!(parse_card("2s").unwrap(), 3);
        assert_eq!(parse_card("Ac").unwrap(), 48);
        assert_eq!(parse_card("As").unwrap(), 51);
        for id in 0..52u8 {
            assert_eq!(parse_card(&card_to_string(id)).unwrap(), id);
        }
    }

    #[test]
    fn combo_is_normalized_high_card_first() {
        let c = Combo::parse("4cAc").unwrap();
        assert_eq!(c.to_string(), "Ac4c");
        assert_eq!(c.hand_class(), "A4s");
        assert_eq!(Combo::parse("KhAs").unwrap().hand_class(), "AKo");
        assert_eq!(Combo::parse("7d7s").unwrap().hand_class(), "77");
    }

    #[test]
    fn board_rejects_duplicates_and_bad_lengths() {
        assert!(parse_board("KsKs4c").is_err());
        assert!(parse_board("Ks7d").is_err());
        assert_eq!(parse_board("Ks 7d 4c 2h 9s").unwrap().len(), 5);
    }

    #[test]
    fn blocked_combos_detected() {
        let board = cards_mask(&parse_board("Ks7d4c2h9s").unwrap());
        assert!(Combo::parse("AsKs").unwrap().is_blocked_by(board));
        assert!(!Combo::parse("AsQs").unwrap().is_blocked_by(board));
    }
}
