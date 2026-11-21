//! Simple flop texture classification used to tag exercises.

use crate::cards::{rank_of, suit_of, Card};

pub fn flop_texture_tags(board: &[Card]) -> Vec<String> {
    let flop = &board[..3.min(board.len())];
    let mut tags = Vec::new();
    let suits: std::collections::BTreeSet<u8> = flop.iter().map(|&c| suit_of(c)).collect();
    tags.push(
        match suits.len() {
            1 => "monotone",
            2 => "two-tone",
            _ => "rainbow",
        }
        .to_string(),
    );
    let mut ranks: Vec<u8> = flop.iter().map(|&c| rank_of(c)).collect();
    ranks.sort_unstable_by(|a, b| b.cmp(a));
    let distinct: std::collections::BTreeSet<u8> = ranks.iter().copied().collect();
    if distinct.len() < ranks.len() {
        tags.push("paired".into());
    }
    let high = ranks[0];
    tags.push(
        if high == 12 {
            "A-high"
        } else if high >= 9 {
            "broadway-high"
        } else if high >= 6 {
            "middle"
        } else {
            "low"
        }
        .to_string(),
    );
    if distinct.len() == 3 && ranks[0] - ranks[2] <= 4 {
        tags.push("connected".into());
    }
    tags
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cards::parse_board;

    #[test]
    fn textures() {
        let t = flop_texture_tags(&parse_board("Ks7d4c").unwrap());
        assert_eq!(t, vec!["rainbow", "broadway-high"]);
        let t = flop_texture_tags(&parse_board("9h8h7h").unwrap());
        assert_eq!(t, vec!["monotone", "middle", "connected"]);
        let t = flop_texture_tags(&parse_board("AsAd5s").unwrap());
        assert_eq!(t, vec!["two-tone", "paired", "A-high"]);
    }
}
