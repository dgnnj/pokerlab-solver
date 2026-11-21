//! Local SQLite storage: versioned migrations, backups, sessions, attempts, reviews, preferences.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

pub const SCHEMA_VERSION: i64 = 2;

/// Migrations indexed by target version. Never edit a released migration; append a new one.
const MIGRATIONS: &[(i64, &str)] = &[
    (
        1,
        r#"
CREATE TABLE packs (
    pack_id TEXT NOT NULL,
    version TEXT NOT NULL,
    title TEXT NOT NULL,
    manifest_sha256 TEXT NOT NULL,
    engine_rev TEXT NOT NULL,
    path TEXT NOT NULL,
    exercises INTEGER NOT NULL,
    installed_at TEXT NOT NULL,
    PRIMARY KEY (pack_id, version)
);
CREATE TABLE sessions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at TEXT NOT NULL,
    ended_at TEXT,
    mode TEXT NOT NULL,            -- representative | hard | review
    theme TEXT,                    -- NULL = all themes
    target_count INTEGER NOT NULL,
    pack_id TEXT NOT NULL,
    pack_version TEXT NOT NULL
);
CREATE TABLE attempts (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL UNIQUE,    -- one attempt per presented exercise
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    exercise_id TEXT NOT NULL,
    solution_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    pack_id TEXT NOT NULL,
    pack_version TEXT NOT NULL,
    engine_rev TEXT NOT NULL,
    combo TEXT NOT NULL,
    chosen_index INTEGER NOT NULL,
    chosen_label TEXT NOT NULL,
    best_index INTEGER NOT NULL,
    best_label TEXT NOT NULL,
    best_ev_chips REAL NOT NULL,
    chosen_ev_chips REAL NOT NULL,
    loss_chips REAL NOT NULL,
    loss_bb REAL NOT NULL,
    loss_pct_pot REAL NOT NULL,
    grade TEXT NOT NULL,
    chosen_frequency REAL NOT NULL,
    sampling_mode TEXT NOT NULL,
    decision_ms INTEGER NOT NULL,
    theme TEXT NOT NULL,
    theme_label TEXT NOT NULL,
    street TEXT NOT NULL,
    position TEXT NOT NULL,
    pot_type TEXT NOT NULL,
    board TEXT NOT NULL,
    textures TEXT NOT NULL
);
CREATE INDEX attempts_session ON attempts(session_id);
CREATE INDEX attempts_theme ON attempts(theme);
CREATE TABLE reviews (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    attempt_id INTEGER NOT NULL UNIQUE REFERENCES attempts(id) ON DELETE CASCADE,
    exercise_id TEXT NOT NULL,
    combo TEXT NOT NULL,
    theme TEXT NOT NULL,
    tags TEXT NOT NULL DEFAULT '',
    repetitions INTEGER NOT NULL DEFAULT 0,
    last_reviewed_at TEXT,
    next_review_at TEXT NOT NULL,
    created_at TEXT NOT NULL,
    manual INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE notes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    created_at TEXT NOT NULL,
    session_id INTEGER REFERENCES sessions(id) ON DELETE SET NULL,
    text TEXT NOT NULL
);
CREATE TABLE preferences (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#,
    ),
    (
        2,
        r#"
-- Full hands simulated against the engine policy (flop to river).
CREATE TABLE hands (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    token TEXT NOT NULL UNIQUE,
    started_at TEXT NOT NULL,
    ended_at TEXT,                 -- NULL = interrupted
    solution_id TEXT NOT NULL,
    config_hash TEXT NOT NULL,
    same_config_as_pack INTEGER NOT NULL,
    pack_id TEXT NOT NULL,
    pack_version TEXT NOT NULL,
    engine_rev TEXT NOT NULL,
    solve_exploitability_pct REAL NOT NULL,
    hero_position TEXT NOT NULL,
    villain_position TEXT NOT NULL,
    hero_combo TEXT NOT NULL,
    villain_combo TEXT NOT NULL,
    flop TEXT NOT NULL,
    final_board TEXT,
    line TEXT,
    showdown INTEGER,
    winner TEXT,                   -- hero | villain | split
    pot_chips INTEGER,
    hero_net_bb REAL,              -- postflop result
    decisions INTEGER NOT NULL DEFAULT 0,
    graded INTEGER NOT NULL DEFAULT 0,
    total_loss_bb REAL NOT NULL DEFAULT 0,
    cut_reason TEXT
);
CREATE TABLE hand_decisions (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    hand_id INTEGER NOT NULL REFERENCES hands(id) ON DELETE CASCADE,
    seq INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    node_id TEXT NOT NULL,
    street TEXT NOT NULL,
    board TEXT NOT NULL,
    pot_chips INTEGER NOT NULL,
    chosen_index INTEGER NOT NULL,
    chosen_label TEXT NOT NULL,
    best_index INTEGER,
    best_label TEXT,
    best_ev_chips REAL,
    chosen_ev_chips REAL,
    loss_bb REAL,
    loss_pct_pot REAL,
    grade TEXT,                    -- NULL = not evaluable
    chosen_frequency REAL,
    equity REAL,
    not_evaluable TEXT,
    decision_ms INTEGER NOT NULL,
    UNIQUE (hand_id, seq)
);
CREATE INDEX hand_decisions_hand ON hand_decisions(hand_id);
"#,
    ),
];

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("banco: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(
        "banco em versão {found} mais nova que a suportada ({supported}); atualize o aplicativo"
    )]
    TooNew { found: i64, supported: i64 },
    #[error("E/S: {0}")]
    Io(String),
}

pub struct Db {
    pub conn: Connection,
    pub path: PathBuf,
}

pub fn now() -> String {
    poker_core::now_iso8601()
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Db {
    pub fn open(path: &Path) -> Result<Self, DbError> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| DbError::Io(e.to_string()))?;
        }
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let mut db = Db {
            conn,
            path: path.to_path_buf(),
        };
        db.migrate()?;
        Ok(db)
    }

    pub fn schema_version(&self) -> Result<i64, DbError> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))?)
    }

    fn migrate(&mut self) -> Result<(), DbError> {
        let current = self.schema_version()?;
        if current > SCHEMA_VERSION {
            return Err(DbError::TooNew {
                found: current,
                supported: SCHEMA_VERSION,
            });
        }
        let pending: Vec<_> = MIGRATIONS.iter().filter(|(v, _)| *v > current).collect();
        if pending.is_empty() {
            return Ok(());
        }
        // Consistent backup before touching an existing database.
        if current > 0 {
            self.backup_to(&self.backup_path(&format!("pre-migracao-v{current}")))?;
        }
        for (v, sql) in pending {
            let tx = self.conn.transaction()?;
            tx.execute_batch(sql)?;
            tx.pragma_update(None, "user_version", v)?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn backup_path(&self, label: &str) -> PathBuf {
        let dir = self.path.parent().unwrap_or(Path::new(".")).join("backups");
        dir.join(format!("pokerlab-{}-{}.sqlite3", label, unix_now()))
    }

    /// Online backup through SQLite's backup API (safe while the app is running).
    pub fn backup_to(&self, dest: &Path) -> Result<PathBuf, DbError> {
        if let Some(d) = dest.parent() {
            std::fs::create_dir_all(d).map_err(|e| DbError::Io(e.to_string()))?;
        }
        let mut out = Connection::open(dest)?;
        let backup = rusqlite::backup::Backup::new(&self.conn, &mut out)?;
        backup.run_to_completion(256, std::time::Duration::from_millis(5), None)?;
        Ok(dest.to_path_buf())
    }

    pub fn get_pref(&self, key: &str) -> Result<Option<String>, DbError> {
        Ok(self
            .conn
            .query_row("SELECT value FROM preferences WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn set_pref(&self, key: &str, value: &str) -> Result<(), DbError> {
        self.conn.execute(
            "INSERT INTO preferences(key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    pub fn all_prefs(&self) -> Result<Vec<(String, String)>, DbError> {
        let mut st = self
            .conn
            .prepare("SELECT key, value FROM preferences ORDER BY key")?;
        let rows = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct AttemptRow {
    pub id: i64,
    pub session_id: i64,
    pub created_at: String,
    pub exercise_id: String,
    pub combo: String,
    pub chosen_label: String,
    pub best_label: String,
    pub loss_bb: f64,
    pub loss_pct_pot: f64,
    pub grade: String,
    pub sampling_mode: String,
    pub theme: String,
    pub theme_label: String,
    pub street: String,
    pub position: String,
    pub board: String,
    pub pack_version: String,
    pub decision_ms: i64,
    pub in_review: bool,
}

pub const ATTEMPT_COLUMNS: &str = "a.id, a.session_id, a.created_at, a.exercise_id, a.combo, a.chosen_label, a.best_label,
     a.loss_bb, a.loss_pct_pot, a.grade, a.sampling_mode, a.theme, a.theme_label, a.street, a.position,
     a.board, a.pack_version, a.decision_ms, (SELECT COUNT(*) FROM reviews r WHERE r.attempt_id = a.id) > 0";

pub fn map_attempt(r: &rusqlite::Row) -> rusqlite::Result<AttemptRow> {
    Ok(AttemptRow {
        id: r.get(0)?,
        session_id: r.get(1)?,
        created_at: r.get(2)?,
        exercise_id: r.get(3)?,
        combo: r.get(4)?,
        chosen_label: r.get(5)?,
        best_label: r.get(6)?,
        loss_bb: r.get(7)?,
        loss_pct_pot: r.get(8)?,
        grade: r.get(9)?,
        sampling_mode: r.get(10)?,
        theme: r.get(11)?,
        theme_label: r.get(12)?,
        street: r.get(13)?,
        position: r.get(14)?,
        board: r.get(15)?,
        pack_version: r.get(16)?,
        decision_ms: r.get(17)?,
        in_review: r.get(18)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.sqlite3");
        {
            let db = Db::open(&p).unwrap();
            assert_eq!(db.schema_version().unwrap(), SCHEMA_VERSION);
            db.set_pref("k", "v").unwrap();
        }
        let db = Db::open(&p).unwrap();
        assert_eq!(db.get_pref("k").unwrap().as_deref(), Some("v"));
    }

    #[test]
    fn v1_database_migrates_to_v2_with_backup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.sqlite3");
        {
            let c = Connection::open(&p).unwrap();
            c.execute_batch(MIGRATIONS[0].1).unwrap();
            c.pragma_update(None, "user_version", 1).unwrap();
            c.execute("INSERT INTO preferences VALUES ('k', 'v1')", [])
                .unwrap();
        }
        let db = Db::open(&p).unwrap();
        assert_eq!(db.schema_version().unwrap(), 2);
        assert_eq!(db.get_pref("k").unwrap().as_deref(), Some("v1"));
        let n: i64 = db
            .conn
            .query_row("SELECT COUNT(*) FROM hands", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
        let backups: Vec<_> = std::fs::read_dir(dir.path().join("backups"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        assert!(
            backups.iter().any(|b| b.contains("pre-migracao-v1")),
            "{backups:?}"
        );
    }

    #[test]
    fn refuses_newer_schema() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.sqlite3");
        {
            let db = Db::open(&p).unwrap();
            db.conn.pragma_update(None, "user_version", 99).unwrap();
        }
        assert!(matches!(Db::open(&p), Err(DbError::TooNew { .. })));
    }

    #[test]
    fn backup_is_readable() {
        let dir = tempfile::tempdir().unwrap();
        let db = Db::open(&dir.path().join("t.sqlite3")).unwrap();
        db.set_pref("x", "1").unwrap();
        let b = db.backup_to(&dir.path().join("b.sqlite3")).unwrap();
        let c = Connection::open(b).unwrap();
        let v: String = c
            .query_row("SELECT value FROM preferences WHERE key='x'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(v, "1");
    }
}
