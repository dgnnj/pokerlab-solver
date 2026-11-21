//! Spot (scenario) configuration, loaded from versioned TOML files in `data/spots/`.
//!
//! All monetary values are integers in *chips*, with an explicit `chips_per_bb` scale.
//! Conversion to big blinds happens only for display.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cards::{cards_mask, parse_board, Card};
use crate::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Street {
    Flop,
    Turn,
    River,
}

impl Street {
    pub fn board_len(self) -> usize {
        match self {
            Street::Flop => 3,
            Street::Turn => 4,
            Street::River => 5,
        }
    }
    pub fn label_pt(self) -> &'static str {
        match self {
            Street::Flop => "flop",
            Street::Turn => "turn",
            Street::River => "river",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RangeRef {
    /// Range in solver syntax, e.g. `"AA,KQs:0.5,AsKh"`.
    pub text: String,
    /// Where the range comes from.
    pub source: String,
    /// True when the range was built by hand as a study premise.
    #[serde(default)]
    pub manual_premise: bool,
    #[serde(default)]
    pub notes: String,
}

/// Bet/raise sizes per street for one player, in solver syntax (e.g. `"33%, 75%"`, `"3x"`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StreetSizes {
    #[serde(default)]
    pub bet: String,
    #[serde(default)]
    pub raise: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct PlayerSizes {
    #[serde(default)]
    pub flop: StreetSizes,
    #[serde(default)]
    pub turn: StreetSizes,
    #[serde(default)]
    pub river: StreetSizes,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TreeParams {
    pub oop: PlayerSizes,
    pub ip: PlayerSizes,
    /// Donk sizes for OOP on turn/river after IP called. Empty = use default bet sizes.
    #[serde(default)]
    pub turn_donk: String,
    #[serde(default)]
    pub river_donk: String,
    #[serde(default = "default_add_allin")]
    pub add_allin_threshold: f64,
    #[serde(default = "default_force_allin")]
    pub force_allin_threshold: f64,
    #[serde(default = "default_merging")]
    pub merging_threshold: f64,
}

fn default_add_allin() -> f64 {
    1.5
}
fn default_force_allin() -> f64 {
    0.15
}
fn default_merging() -> f64 {
    0.1
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolveParams {
    pub max_iterations: u32,
    /// Target exploitability as a percentage of the starting pot (e.g. 0.5 = 0.5%).
    pub target_exploitability_pct: f64,
    /// Refuse to allocate if estimated memory exceeds this (MiB). Compression is tried first.
    #[serde(default = "default_mem_limit")]
    pub memory_limit_mib: u64,
}

fn default_mem_limit() -> u64 {
    12 * 1024
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Blinds {
    pub sb_chips: i32,
    pub bb_chips: i32,
    #[serde(default)]
    pub ante_chips: i32,
}

/// Documented preflop accounting. Used to validate pot/stack values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PreflopAccounting {
    pub starting_stack_chips: i32,
    pub blinds: Blinds,
    /// Human-readable preflop line, e.g. "BTN abre 2,5 bb, SB desiste, BB paga".
    pub line: String,
    /// Chips each active player put in preflop.
    pub invested_per_player_chips: i32,
    /// Dead money from folded players (e.g. the SB).
    pub dead_chips: i32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rake {
    /// Fraction of the pot, 0.0..=1.0.
    pub rate: f64,
    /// Cap in chips (0 = no rake when rate = 0).
    pub cap_chips: f64,
    pub structure: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SpotConfig {
    pub id: String,
    pub version: u32,
    pub title: String,
    #[serde(default = "default_format")]
    pub format: String,
    pub oop_position: String,
    pub ip_position: String,
    pub pot_type: String,
    pub initial_street: Street,
    pub board: String,
    pub chips_per_bb: i32,
    pub starting_pot_chips: i32,
    pub effective_stack_chips: i32,
    #[serde(default)]
    pub preflop: Option<PreflopAccounting>,
    pub rake: Rake,
    pub tree: TreeParams,
    pub oop_range: RangeRef,
    pub ip_range: RangeRef,
    pub solve: SolveParams,
    /// Optional study tags (texture etc). Computed automatically when empty.
    #[serde(default)]
    pub tags: Vec<String>,
}

fn default_format() -> String {
    "NLHE cash 6-max".to_string()
}

impl SpotConfig {
    pub fn from_toml(s: &str) -> Result<Self, CoreError> {
        let spot: SpotConfig = toml::from_str(s).map_err(|e| CoreError::Config(e.to_string()))?;
        spot.validate()?;
        Ok(spot)
    }

    pub fn board_cards(&self) -> Result<Vec<Card>, CoreError> {
        parse_board(&self.board)
    }

    pub fn board_mask(&self) -> Result<u64, CoreError> {
        Ok(cards_mask(&self.board_cards()?))
    }

    pub fn validate(&self) -> Result<(), CoreError> {
        let board = self.board_cards()?;
        if board.len() != self.initial_street.board_len() {
            return Err(CoreError::Config(format!(
                "board '{}' tem {} cartas, mas a rua inicial {:?} exige {}",
                self.board,
                board.len(),
                self.initial_street,
                self.initial_street.board_len()
            )));
        }
        if self.chips_per_bb <= 0 {
            return Err(CoreError::Config("chips_per_bb deve ser > 0".into()));
        }
        if self.starting_pot_chips <= 0 || self.effective_stack_chips <= 0 {
            return Err(CoreError::Config("pote e stack devem ser > 0".into()));
        }
        if !(0.0..=1.0).contains(&self.rake.rate) || self.rake.cap_chips < 0.0 {
            return Err(CoreError::Config("rake inválido".into()));
        }
        if self.oop_position == self.ip_position {
            return Err(CoreError::Config("posições OOP e IP iguais".into()));
        }
        if let Some(p) = &self.preflop {
            // `dead_chips` includes folded blinds and all antes.
            let pot = 2 * p.invested_per_player_chips + p.dead_chips;
            if pot != self.starting_pot_chips && self.initial_street == Street::Flop {
                return Err(CoreError::Config(format!(
                    "contabilidade pré-flop inconsistente: 2 x {} + {} = {} != pote {}",
                    p.invested_per_player_chips, p.dead_chips, pot, self.starting_pot_chips
                )));
            }
            let stack = p.starting_stack_chips - p.invested_per_player_chips;
            if stack != self.effective_stack_chips && self.initial_street == Street::Flop {
                return Err(CoreError::Config(format!(
                    "stack efetivo inconsistente: {} - {} = {} != {}",
                    p.starting_stack_chips,
                    p.invested_per_player_chips,
                    stack,
                    self.effective_stack_chips
                )));
            }
        }
        Ok(())
    }

    pub fn chips_to_bb(&self, chips: f64) -> f64 {
        chips / self.chips_per_bb as f64
    }

    /// Stable hash of everything that changes the solution (config, ranges, sizes, rake,
    /// solve params) plus the engine revision. Used as cache key.
    pub fn config_hash(&self, engine_rev: &str) -> String {
        let mut canon = self.clone();
        canon.title.clear();
        canon.tags.clear();
        canon.oop_range.notes.clear();
        canon.ip_range.notes.clear();
        let json = serde_json::to_string(&canon).expect("spot serializes");
        let mut h = Sha256::new();
        h.update(json.as_bytes());
        h.update(b"|engine=");
        h.update(engine_rev.as_bytes());
        hex::encode(h.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample_toml() -> &'static str {
        r#"
id = "btn_bb_srp_test"
version = 1
title = "teste"
oop_position = "BB"
ip_position = "BTN"
pot_type = "SRP"
initial_street = "flop"
board = "Ks7d4c"
chips_per_bb = 100
starting_pot_chips = 550
effective_stack_chips = 9750

[preflop]
starting_stack_chips = 10000
line = "BTN abre 2,5 bb, SB desiste, BB paga"
invested_per_player_chips = 250
dead_chips = 50
[preflop.blinds]
sb_chips = 50
bb_chips = 100

[rake]
rate = 0.0
cap_chips = 0.0
structure = "sem rake (fixture de referência)"

[tree]
add_allin_threshold = 1.5
[tree.oop.flop]
bet = ""
raise = "3x"
[tree.ip.flop]
bet = "33%"
raise = "3x"

[oop_range]
text = "AA,KK"
source = "teste"
[ip_range]
text = "QQ,JJ"
source = "teste"

[solve]
max_iterations = 100
target_exploitability_pct = 0.5
"#
    }

    #[test]
    fn btn_vs_bb_accounting_fixture() {
        // Blinds 0.5/1, BTN opens 2.5 bb, SB folds, BB calls: pot 5.5 bb, 97.5 bb behind.
        let spot = SpotConfig::from_toml(sample_toml()).unwrap();
        assert_eq!(spot.chips_to_bb(spot.starting_pot_chips as f64), 5.5);
        assert_eq!(spot.chips_to_bb(spot.effective_stack_chips as f64), 97.5);
    }

    #[test]
    fn inconsistent_accounting_rejected() {
        let bad = sample_toml().replace("starting_pot_chips = 550", "starting_pot_chips = 500");
        assert!(SpotConfig::from_toml(&bad).is_err());
    }

    #[test]
    fn hash_changes_with_ranges_and_sizes() {
        let a = SpotConfig::from_toml(sample_toml()).unwrap();
        let mut b = a.clone();
        b.ip_range.text = "QQ".into();
        let mut c = a.clone();
        c.tree.ip.flop.bet = "75%".into();
        let mut d = a.clone();
        d.title = "outro título".into();
        assert_ne!(a.config_hash("r"), b.config_hash("r"));
        assert_ne!(a.config_hash("r"), c.config_hash("r"));
        assert_eq!(a.config_hash("r"), d.config_hash("r"));
        assert_ne!(a.config_hash("r1"), a.config_hash("r2"));
    }
}
