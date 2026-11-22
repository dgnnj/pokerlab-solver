//! Exercise themes from the shape of a decision: street, how the previous street went, who acts
//! and whether they face a bet or a raise. Shared by pack generation (themes of the exercises)
//! and the app (theme of a decision made in a full hand, to send it to the review queue).

use crate::spot::{SpotConfig, Street};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// Stable key, e.g. `srp_turn_after_bet_vs_bet`. Flop keys are unchanged from pack 1.0.0.
    pub key: String,
    /// Readable label, e.g. `BB no turn após aposta paga: enfrentar aposta`.
    pub label: String,
    /// Difficulty level 1..=3.
    pub level: u8,
}

fn is_street_marker(l: &str) -> bool {
    l.starts_with("Turn: ") || l.starts_with("River: ")
}

/// Splits a readable line (exported-node format, with `Turn: ...` / `River: ...` markers) into
/// (previous street actions, current street actions).
pub fn street_segments(line: &[String]) -> (Vec<&str>, Vec<&str>) {
    let marks: Vec<usize> = (0..line.len())
        .filter(|&i| is_street_marker(&line[i]))
        .collect();
    match marks.as_slice() {
        [] => (vec![], line.iter().map(String::as_str).collect()),
        [.., last] => {
            let prev_start = if marks.len() >= 2 {
                marks[marks.len() - 2] + 1
            } else {
                0
            };
            (
                line[prev_start..*last].iter().map(String::as_str).collect(),
                line[last + 1..].iter().map(String::as_str).collect(),
            )
        }
    }
}

/// Theme of a decision by `position` at `street` after `line`, facing `to_call_chips`.
pub fn classify(
    street: Street,
    line: &[String],
    to_call_chips: i32,
    position: &str,
    spot: &SpotConfig,
) -> Theme {
    let street_pt = street.label_pt();
    let (prev, cur) = street_segments(line);
    let facing_raise = cur
        .iter()
        .any(|l| l.contains("aumenta") || l.contains("all-in"));
    let is_ip = position == spot.ip_position;
    let (key, what, level) = if to_call_chips == 0 {
        if is_ip {
            ("ip_bet_or_check", "apostar ou passar após check", 1u8)
        } else {
            ("oop_lead", "liderar ou passar", 1)
        }
    } else if facing_raise {
        ("vs_raise", "enfrentar aumento", 3)
    } else {
        ("vs_bet", "enfrentar aposta", 2)
    };
    let (prior_key, prior_label) = if street == Street::Flop {
        ("", "")
    } else if prev
        .iter()
        .any(|l| l.contains("aposta") || l.contains("aumenta"))
    {
        ("_after_bet", " após aposta paga")
    } else {
        ("_after_check", " após check-check")
    };
    let level = match street {
        Street::Flop => level,
        Street::Turn => (level + 1).min(3),
        Street::River => 3,
    };
    Theme {
        key: format!(
            "{}_{}{}_{}",
            spot.pot_type.to_lowercase(),
            street_pt,
            prior_key,
            key
        ),
        label: format!("{position} no {street_pt}{prior_label}: {what}"),
        level,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn segments_split_on_the_last_street_marker() {
        let line = l(&[
            "BB passa",
            "BTN aposta 2",
            "BB paga",
            "Turn: As",
            "BB passa",
        ]);
        let (prev, cur) = street_segments(&line);
        assert_eq!(prev, vec!["BB passa", "BTN aposta 2", "BB paga"]);
        assert_eq!(cur, vec!["BB passa"]);
        let flop = l(&["BB passa"]);
        let (prev, cur) = street_segments(&flop);
        assert!(prev.is_empty());
        assert_eq!(cur, vec!["BB passa"]);
    }
}
