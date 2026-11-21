//! PokerLab domain crate. Independent from the solver engine so that the desktop app can read
//! packs and grade decisions without linking the solver.

pub mod cards;
pub mod eval;
pub mod pack;
pub mod runout;
pub mod sampling;
pub mod solution;
pub mod spot;
pub mod texture;

#[derive(Debug, thiserror::Error, Clone, PartialEq)]
pub enum CoreError {
    #[error("carta inválida: {0}")]
    InvalidCard(String),
    #[error("carta repetida: {0}")]
    DuplicateCard(String),
    #[error("board inválido: {0}")]
    InvalidBoard(String),
    #[error("combinação inválida: {0}")]
    InvalidCombo(String),
    #[error("configuração inválida: {0}")]
    Config(String),
    #[error("solução inválida: {0}")]
    InvalidSolution(String),
    #[error("pacote inválido: {0}")]
    InvalidPack(String),
    #[error("decisão sem avaliação direta: {0}")]
    NotEvaluable(String),
    #[error("E/S: {0}")]
    Io(String),
}

/// Current UTC time as ISO-8601 (`2026-11-20T12:34:56Z`), without external crates.
pub fn now_iso8601() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    iso8601_from_unix(secs)
}

pub fn iso8601_from_unix(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60
    )
}

/// Formats chips as big blinds with Brazilian decimal comma, e.g. `1,82`.
pub fn fmt_bb(chips: f64, chips_per_bb: i32) -> String {
    let v = chips / chips_per_bb as f64;
    let s = format!("{v:.2}");
    s.replace('.', ",")
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_dates() {
        assert_eq!(super::iso8601_from_unix(0), "1970-01-01T00:00:00Z");
        assert_eq!(
            super::iso8601_from_unix(1_795_132_800),
            "2026-11-20T00:00:00Z"
        );
        assert_eq!(super::fmt_bb(182.0, 100), "1,82");
    }
}
