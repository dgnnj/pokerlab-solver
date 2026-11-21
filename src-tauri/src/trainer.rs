//! Trainer logic, independent from Tauri so it can be unit-tested: pack installation, exercise
//! selection and sampling, grading, persistence of attempts, review queue, study views, stats.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;

use poker_core::cards::Combo;
use poker_core::eval::{evaluate, Feedback, Grade, Tolerance};
use poker_core::pack::{sha256_hex, ExerciseDef, PackManifest};
use poker_core::sampling::{weighted_index, Rng};
use poker_core::solution::{NodeSolution, SolutionFile};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::db::{map_attempt, now, unix_now, AttemptRow, Db, DbError, ATTEMPT_COLUMNS};

#[derive(Debug, thiserror::Error)]
pub enum TrainerError {
    #[error(transparent)]
    Db(#[from] DbError),
    #[error("banco: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Core(#[from] poker_core::CoreError),
    #[error("{0}")]
    Msg(String),
}

impl Serialize for TrainerError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

type R<T> = Result<T, TrainerError>;

fn msg<T>(m: impl Into<String>) -> R<T> {
    Err(TrainerError::Msg(m.into()))
}

pub struct LoadedPack {
    pub manifest: PackManifest,
    pub manifest_sha256: String,
    pub dir: PathBuf,
    pub solutions: HashMap<String, SolutionFile>,
}

impl LoadedPack {
    pub fn exercise(&self, id: &str) -> Option<&ExerciseDef> {
        self.manifest.exercises.iter().find(|e| e.id == id)
    }
    pub fn node(&self, ex: &ExerciseDef) -> Option<(&SolutionFile, &NodeSolution)> {
        let s = self.solutions.get(&ex.solution_id)?;
        Some((s, s.node(&ex.node_id)?))
    }
}

fn copy_dir(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let to = dst.join(e.file_name());
        if e.file_type()?.is_dir() {
            copy_dir(&e.path(), &to)?;
        } else {
            std::fs::copy(e.path(), to)?;
        }
    }
    Ok(())
}

/// Installs (or re-validates) a bundled pack into the user's data dir, registers it in SQLite
/// and loads it. A corrupted installed copy is replaced from the bundled source.
pub fn install_pack(db: &Db, bundled: &Path, packs_root: &Path) -> R<LoadedPack> {
    let src_manifest = PackManifest::load(bundled)?;
    let target = packs_root
        .join(&src_manifest.pack_id)
        .join(&src_manifest.version);
    let valid = PackManifest::load(&target)
        .ok()
        .and_then(|m| m.verify_and_load(&target).ok().map(|s| (m, s)));
    let (manifest, sols) = match valid {
        Some(v) => v,
        None => {
            // Validate the source before copying, copy to a staging dir, then publish atomically.
            src_manifest.verify_and_load(bundled)?;
            let staging = target.with_extension("staging");
            let _ = std::fs::remove_dir_all(&staging);
            copy_dir(bundled, &staging)
                .map_err(|e| TrainerError::Msg(format!("copiando pacote: {e}")))?;
            let _ = std::fs::remove_dir_all(&target);
            std::fs::rename(&staging, &target)
                .map_err(|e| TrainerError::Msg(format!("publicando pacote: {e}")))?;
            let m = PackManifest::load(&target)?;
            let s = m.verify_and_load(&target)?;
            (m, s)
        }
    };
    let manifest_sha256 =
        sha256_hex(&std::fs::read(target.join("manifest.json")).unwrap_or_default());
    db.conn.execute(
        "INSERT INTO packs(pack_id, version, title, manifest_sha256, engine_rev, path, exercises, installed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(pack_id, version) DO UPDATE SET manifest_sha256 = excluded.manifest_sha256, path = excluded.path",
        params![
            manifest.pack_id,
            manifest.version,
            manifest.title,
            manifest_sha256,
            manifest.engine.git_rev,
            target.display().to_string(),
            manifest.exercises.len() as i64,
            now()
        ],
    )?;
    let solutions = sols
        .into_iter()
        .map(|s| (s.solution_id.clone(), s))
        .collect();
    Ok(LoadedPack {
        manifest,
        manifest_sha256,
        dir: target,
        solutions,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Exercises uniform within the theme, combos by their weight at the node.
    Representative,
    /// Combos weighted towards mixed strategies (close decisions). Stats kept separate.
    Hard,
    /// Due items from the review queue.
    Review,
}

impl Mode {
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Representative => "representative",
            Mode::Hard => "hard",
            Mode::Review => "review",
        }
    }
    fn parse(s: &str) -> Mode {
        match s {
            "hard" => Mode::Hard,
            "review" => Mode::Review,
            _ => Mode::Representative,
        }
    }
}

struct Pending {
    session_id: i64,
    exercise_id: String,
    combo: Combo,
    mode: Mode,
    review_id: Option<i64>,
    presented: Instant,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeInfo {
    pub theme: String,
    pub label: String,
    pub exercises: usize,
    pub level: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: i64,
    pub mode: String,
    pub theme: Option<String>,
    pub target_count: i64,
    pub done: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ActionOption {
    pub index: usize,
    pub label: String,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Presented {
    pub token: String,
    pub session: SessionInfo,
    pub exercise_id: String,
    pub theme_label: String,
    pub mode: String,
    pub hero_position: String,
    pub villain_position: String,
    pub hero_cards: [String; 2],
    pub board: Vec<String>,
    pub street: String,
    pub pot_bb: f64,
    pub to_call_bb: f64,
    pub stack_bb: f64,
    pub preflop_line: String,
    pub line: Vec<String>,
    pub actions: Vec<ActionOption>,
    pub pack_version: String,
    pub review_note: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FeedbackView {
    pub attempt_id: i64,
    pub feedback: Feedback,
    pub grade_label: String,
    pub explanation: Vec<String>,
    pub equity: f32,
    pub pot_bb: f64,
    pub tolerance_bb: f64,
    pub session: SessionInfo,
    pub in_review: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThemeStat {
    pub key: String,
    pub label: String,
    pub n: i64,
    pub avg_loss_bb: f64,
    pub total_loss_bb: f64,
    pub best_or_close_pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub session: SessionInfo,
    pub decisions: i64,
    pub avg_loss_bb: f64,
    pub total_loss_bb: f64,
    pub grades: BTreeMap<String, i64>,
    pub worst_themes: Vec<ThemeStat>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProgressFilter {
    pub position: Option<String>,
    pub street: Option<String>,
    pub pot_type: Option<String>,
    pub texture: Option<String>,
    pub theme: Option<String>,
    pub mode: Option<String>,
    pub since: Option<String>,
    pub pack_version: Option<String>,
    /// theme | street | position | sampling_mode | pack_version | board
    pub group_by: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressReport {
    pub total: ThemeStat,
    pub groups: Vec<ThemeStat>,
    pub recent: Vec<AttemptRow>,
}

#[derive(Debug, Clone, Serialize)]
pub struct GridCell {
    pub hand: String,
    pub weight: f64,
    /// Weighted strategy per action (sums to 1 when weight > 0).
    pub strategy: Vec<f64>,
    pub ev_bb: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComboRow {
    pub combo: String,
    pub hand: String,
    pub weight: f32,
    pub equity: f32,
    pub strategy: Vec<f32>,
    pub ev_bb: Vec<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudyView {
    pub exercise_id: String,
    pub theme_label: String,
    pub position: String,
    pub board: Vec<String>,
    pub pot_bb: f64,
    pub to_call_bb: f64,
    pub stack_bb: f64,
    pub preflop_line: String,
    pub line: Vec<String>,
    pub actions: Vec<ActionOption>,
    pub aggregate: Vec<f64>,
    pub grid: Vec<GridCell>,
    pub combos: Vec<ComboRow>,
    pub highlight: Option<String>,
    pub solve_info: String,
    pub ranges_note: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StudyEntry {
    pub exercise_id: String,
    pub theme_label: String,
    pub board: String,
    pub position: String,
    pub line: Vec<String>,
}

pub struct Trainer {
    pub db: Db,
    pub pack: LoadedPack,
    pub tolerance: Tolerance,
    rng: Rng,
    pending: HashMap<String, Pending>,
    token_seq: u64,
}

const RANKS: &str = "AKQJT98765432";

fn grid_hands() -> Vec<String> {
    let r: Vec<char> = RANKS.chars().collect();
    let mut out = Vec::with_capacity(169);
    for i in 0..13 {
        for j in 0..13 {
            out.push(if i == j {
                format!("{}{}", r[i], r[j])
            } else if i < j {
                format!("{}{}s", r[i], r[j])
            } else {
                format!("{}{}o", r[j], r[i])
            });
        }
    }
    out
}

impl Trainer {
    pub fn new(db: Db, pack: LoadedPack, seed: u64) -> Self {
        Trainer {
            db,
            pack,
            tolerance: Tolerance::default(),
            rng: Rng::new(seed),
            pending: HashMap::new(),
            token_seq: 0,
        }
    }

    fn cpb(&self, sol: &SolutionFile) -> f64 {
        sol.spot.chips_per_bb as f64
    }

    pub fn themes(&self) -> Vec<ThemeInfo> {
        let mut map: BTreeMap<String, ThemeInfo> = BTreeMap::new();
        for e in &self.pack.manifest.exercises {
            let t = map.entry(e.theme.clone()).or_insert(ThemeInfo {
                theme: e.theme.clone(),
                label: e.theme_label.clone(),
                exercises: 0,
                level: e.level,
            });
            t.exercises += 1;
        }
        let mut v: Vec<_> = map.into_values().collect();
        v.sort_by_key(|t| (t.level, t.label.clone()));
        v
    }

    pub fn study_entries(&self) -> Vec<StudyEntry> {
        self.pack
            .manifest
            .exercises
            .iter()
            .filter_map(|e| {
                let (_, n) = self.pack.node(e)?;
                Some(StudyEntry {
                    exercise_id: e.id.clone(),
                    theme_label: e.theme_label.clone(),
                    board: e.board.clone(),
                    position: e.position.clone(),
                    line: n.line.clone(),
                })
            })
            .collect()
    }

    pub fn session(&self, id: i64) -> R<SessionInfo> {
        Ok(self.db.conn.query_row(
            "SELECT s.id, s.mode, s.theme, s.target_count, (SELECT COUNT(*) FROM attempts a WHERE a.session_id = s.id),
                    s.started_at, s.ended_at FROM sessions s WHERE s.id = ?1",
            [id],
            |r| {
                Ok(SessionInfo {
                    id: r.get(0)?,
                    mode: r.get(1)?,
                    theme: r.get(2)?,
                    target_count: r.get(3)?,
                    done: r.get(4)?,
                    started_at: r.get(5)?,
                    ended_at: r.get(6)?,
                })
            },
        )?)
    }

    pub fn last_open_session(&self) -> R<Option<SessionInfo>> {
        let id: Option<i64> = self
            .db
            .conn
            .query_row(
                "SELECT id FROM sessions WHERE ended_at IS NULL ORDER BY id DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .optional()?;
        match id {
            Some(id) => {
                let s = self.session(id)?;
                Ok(if s.done < s.target_count {
                    Some(s)
                } else {
                    None
                })
            }
            None => Ok(None),
        }
    }

    pub fn start_session(
        &mut self,
        mode: Mode,
        theme: Option<String>,
        count: i64,
    ) -> R<SessionInfo> {
        if !(1..=200).contains(&count) {
            return msg("número de decisões deve estar entre 1 e 200");
        }
        if let Some(t) = &theme {
            if !self.pack.manifest.exercises.iter().any(|e| &e.theme == t) {
                return msg(format!("tema desconhecido: {t}"));
            }
        }
        self.db.conn.execute(
            "INSERT INTO sessions(started_at, mode, theme, target_count, pack_id, pack_version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![now(), mode.as_str(), theme, count, self.pack.manifest.pack_id, self.pack.manifest.version],
        )?;
        let id = self.db.conn.last_insert_rowid();
        self.session(id)
    }

    pub fn end_session(&mut self, id: i64) -> R<SessionSummary> {
        self.db.conn.execute(
            "UPDATE sessions SET ended_at = COALESCE(ended_at, ?2) WHERE id = ?1",
            params![id, now()],
        )?;
        self.pending.retain(|_, p| p.session_id != id);
        let session = self.session(id)?;
        let (n, avg, total): (i64, f64, f64) = self.db.conn.query_row(
            "SELECT COUNT(*), COALESCE(AVG(loss_bb), 0), COALESCE(SUM(loss_bb), 0) FROM attempts WHERE session_id = ?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut grades = BTreeMap::new();
        let mut st = self
            .db
            .conn
            .prepare("SELECT grade, COUNT(*) FROM attempts WHERE session_id = ?1 GROUP BY grade")?;
        for row in st.query_map([id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
            let (g, c) = row?;
            grades.insert(g, c);
        }
        let worst_themes = self
            .group_stats(
                "WHERE a.session_id = ?1",
                &[&id as &dyn rusqlite::ToSql],
                "theme",
            )?
            .into_iter()
            .filter(|t| t.total_loss_bb > 0.0)
            .take(3)
            .collect();
        Ok(SessionSummary {
            session,
            decisions: n,
            avg_loss_bb: avg,
            total_loss_bb: total,
            grades,
            worst_themes,
        })
    }

    fn sample_combo(&mut self, node: &NodeSolution, mode: Mode) -> Option<Combo> {
        let weights: Vec<f32> = match mode {
            Mode::Hard => (0..node.combos.len())
                .map(|i| {
                    let maxf = (0..node.num_actions())
                        .map(|a| node.strategy[a][i])
                        .fold(0.0f32, f32::max);
                    node.weights[i] * (1.05 - maxf)
                })
                .collect(),
            _ => node.weights.clone(),
        };
        let i = weighted_index(&weights, &mut self.rng)?;
        Combo::parse(&node.combos[i]).ok()
    }

    fn due_review(&self) -> R<Option<(i64, String, String, String)>> {
        Ok(self
            .db
            .conn
            .query_row(
                "SELECT id, exercise_id, combo, theme FROM reviews WHERE next_review_at <= ?1 ORDER BY next_review_at, id LIMIT 1",
                [now()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?)
    }

    pub fn review_due_count(&self) -> R<i64> {
        Ok(self.db.conn.query_row(
            "SELECT COUNT(*) FROM reviews WHERE next_review_at <= ?1",
            [now()],
            |r| r.get(0),
        )?)
    }

    pub fn next_exercise(&mut self, session_id: i64) -> R<Option<Presented>> {
        let session = self.session(session_id)?;
        if session.ended_at.is_some() {
            return msg("sessão encerrada");
        }
        if session.done >= session.target_count {
            return Ok(None);
        }
        let mode = Mode::parse(&session.mode);
        let mut review_id = None;
        let mut review_note = None;
        let (ex, combo) = if mode == Mode::Review {
            let Some((rid, ex_id, combo_s, theme)) = self.due_review()? else {
                return Ok(None);
            };
            review_id = Some(rid);
            let same = self.pack.exercise(&ex_id).cloned().and_then(|ex| {
                let (_, node) = self.pack.node(&ex)?;
                let c = Combo::parse(&combo_s).ok()?;
                node.combo_index(&c).map(|_| (ex.clone(), c))
            });
            match same {
                Some(v) => {
                    review_note = Some("Revisão: mesma situação e mesma mão".to_string());
                    v
                }
                None => {
                    // Same theme, another hand (pack updated or node unavailable).
                    let pool: Vec<ExerciseDef> = self
                        .pack
                        .manifest
                        .exercises
                        .iter()
                        .filter(|e| e.theme == theme)
                        .cloned()
                        .collect();
                    if pool.is_empty() {
                        self.db
                            .conn
                            .execute("DELETE FROM reviews WHERE id = ?1", [rid])?;
                        return msg("item de revisão sem exercício disponível; removido da fila");
                    }
                    let ex = pool[self.rng.below(pool.len())].clone();
                    let node = self
                        .pack
                        .node(&ex)
                        .map(|(_, n)| n.clone())
                        .ok_or_else(|| TrainerError::Msg("nó ausente".into()))?;
                    let c = self
                        .sample_combo(&node, Mode::Representative)
                        .ok_or_else(|| TrainerError::Msg("sem combos".into()))?;
                    review_note = Some("Revisão: mesmo tema, outra mão".to_string());
                    (ex, c)
                }
            }
        } else {
            let pool: Vec<ExerciseDef> = self
                .pack
                .manifest
                .exercises
                .iter()
                .filter(|e| session.theme.as_ref().is_none_or(|t| &e.theme == t))
                .cloned()
                .collect();
            if pool.is_empty() {
                return msg("nenhum exercício para este tema");
            }
            let ex = pool[self.rng.below(pool.len())].clone();
            let node = self
                .pack
                .node(&ex)
                .map(|(_, n)| n.clone())
                .ok_or_else(|| TrainerError::Msg("nó ausente".into()))?;
            let c = self
                .sample_combo(&node, mode)
                .ok_or_else(|| TrainerError::Msg("sem combos elegíveis".into()))?;
            (ex, c)
        };
        let (sol, node) = self
            .pack
            .node(&ex)
            .ok_or_else(|| TrainerError::Msg("nó ausente".into()))?;
        let cpb = self.cpb(sol);
        self.token_seq += 1;
        let token = format!(
            "{}-{}-{}",
            unix_now(),
            self.token_seq,
            self.rng.next_u64() % 1_000_000
        );
        let villain = if node.position == sol.spot.ip_position {
            sol.spot.oop_position.clone()
        } else {
            sol.spot.ip_position.clone()
        };
        let presented = Presented {
            token: token.clone(),
            session: session.clone(),
            exercise_id: ex.id.clone(),
            theme_label: ex.theme_label.clone(),
            mode: mode.as_str().to_string(),
            hero_position: node.position.clone(),
            villain_position: villain,
            hero_cards: {
                let s = combo.to_string();
                [s[0..2].to_string(), s[2..4].to_string()]
            },
            board: node.board.clone(),
            street: node.street.label_pt().to_string(),
            pot_bb: node.pot_chips as f64 / cpb,
            to_call_bb: node.to_call_chips as f64 / cpb,
            stack_bb: node.stack_chips as f64 / cpb,
            preflop_line: sol
                .spot
                .preflop
                .as_ref()
                .map(|p| p.line.clone())
                .unwrap_or_default(),
            line: node.line.clone(),
            actions: node
                .actions
                .iter()
                .enumerate()
                .map(|(i, a)| ActionOption {
                    index: i,
                    label: a.label.clone(),
                    kind: format!("{:?}", a.kind).to_lowercase(),
                })
                .collect(),
            pack_version: self.pack.manifest.version.clone(),
            review_note,
        };
        self.pending.insert(
            token,
            Pending {
                session_id,
                exercise_id: ex.id.clone(),
                combo,
                mode,
                review_id,
                presented: Instant::now(),
            },
        );
        Ok(Some(presented))
    }

    pub fn submit(
        &mut self,
        token: &str,
        action: usize,
        decision_ms: Option<i64>,
    ) -> R<FeedbackView> {
        let Some(p) = self.pending.remove(token) else {
            return msg("decisão já registrada ou exercício expirado");
        };
        let ex = self
            .pack
            .exercise(&p.exercise_id)
            .cloned()
            .ok_or_else(|| TrainerError::Msg("exercício ausente".into()))?;
        let (sol, node) = self
            .pack
            .node(&ex)
            .ok_or_else(|| TrainerError::Msg("nó ausente".into()))?;
        let fb = match evaluate(
            node,
            &p.combo,
            action,
            sol.spot.chips_per_bb,
            self.tolerance,
        ) {
            Ok(f) => f,
            Err(e) => {
                // Keep the exercise answerable if the action index was invalid.
                let combo = p.combo;
                self.pending.insert(token.to_string(), p);
                return msg(format!("{e} ({combo})"));
            }
        };
        let cpb = sol.spot.chips_per_bb as f64;
        let ms = decision_ms.unwrap_or_else(|| p.presented.elapsed().as_millis() as i64);
        let ci = node.combo_index(&p.combo).unwrap_or(0);
        let equity = node.equity[ci];
        let pot_bb = node.pot_chips as f64 / cpb;
        let explanation = explain(node, &fb, equity, cpb);
        let tx = self.db.conn.transaction()?;
        tx.execute(
            "INSERT INTO attempts(token, session_id, created_at, exercise_id, solution_id, node_id, pack_id, pack_version,
                engine_rev, combo, chosen_index, chosen_label, best_index, best_label, best_ev_chips, chosen_ev_chips,
                loss_chips, loss_bb, loss_pct_pot, grade, chosen_frequency, sampling_mode, decision_ms, theme, theme_label,
                street, position, pot_type, board, textures)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30)",
            params![
                token,
                p.session_id,
                now(),
                ex.id,
                ex.solution_id,
                ex.node_id,
                self.pack.manifest.pack_id,
                self.pack.manifest.version,
                sol.engine.git_rev,
                fb.combo,
                fb.chosen_index as i64,
                node.actions[fb.chosen_index].label,
                fb.best_index as i64,
                node.actions[fb.best_index].label,
                fb.best_ev_chips,
                fb.chosen_ev_chips,
                fb.loss_chips,
                fb.loss_bb,
                fb.loss_pct_pot,
                fb.grade.as_str(),
                fb.chosen_frequency as f64,
                p.mode.as_str(),
                ms,
                ex.theme,
                ex.theme_label,
                ex.street.label_pt(),
                ex.position,
                ex.pot_type,
                ex.board,
                ex.textures.join(","),
            ],
        )?;
        let attempt_id = tx.last_insert_rowid();
        let good = matches!(fb.grade, Grade::Best | Grade::Close);
        if let Some(rid) = p.review_id {
            let reps: i64 = tx
                .query_row(
                    "SELECT repetitions FROM reviews WHERE id = ?1",
                    [rid],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            let (reps, next) = if good {
                let days = [1, 3, 7, 14, 30, 60][(reps as usize).min(5)];
                (reps + 1, unix_now() + days * 86_400)
            } else {
                (0, unix_now() + 600)
            };
            tx.execute(
                "UPDATE reviews SET repetitions = ?2, last_reviewed_at = ?3, next_review_at = ?4 WHERE id = ?1",
                params![rid, reps, now(), poker_core::iso8601_from_unix(next)],
            )?;
        } else if !good {
            tx.execute(
                "INSERT OR IGNORE INTO reviews(attempt_id, exercise_id, combo, theme, next_review_at, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                params![attempt_id, ex.id, fb.combo, ex.theme, now()],
            )?;
        }
        tx.commit()?;
        let in_review: bool = self.db.conn.query_row(
            "SELECT COUNT(*) > 0 FROM reviews WHERE attempt_id = ?1",
            [attempt_id],
            |r| r.get(0),
        )?;
        Ok(FeedbackView {
            attempt_id,
            grade_label: fb.grade.label_pt().to_string(),
            tolerance_bb: fb.tolerance_chips / cpb,
            feedback: fb,
            explanation,
            equity,
            pot_bb,
            session: self.session(p.session_id)?,
            in_review: in_review || p.review_id.is_some(),
        })
    }

    pub fn mark_review(&mut self, attempt_id: i64, tags: &str) -> R<()> {
        let (ex, combo, theme): (String, String, String) = self.db.conn.query_row(
            "SELECT exercise_id, combo, theme FROM attempts WHERE id = ?1",
            [attempt_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        self.db.conn.execute(
            "INSERT INTO reviews(attempt_id, exercise_id, combo, theme, tags, next_review_at, created_at, manual)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, 1)
             ON CONFLICT(attempt_id) DO UPDATE SET tags = excluded.tags, manual = 1",
            params![attempt_id, ex, combo, theme, tags, now()],
        )?;
        Ok(())
    }

    pub fn add_note(&mut self, session_id: Option<i64>, text: &str) -> R<()> {
        let t = text.trim();
        if t.is_empty() {
            return msg("nota vazia");
        }
        self.db.conn.execute(
            "INSERT INTO notes(created_at, session_id, text) VALUES (?1, ?2, ?3)",
            params![now(), session_id, t],
        )?;
        Ok(())
    }

    pub fn notes(&self) -> R<Vec<(String, String)>> {
        let mut st = self
            .db
            .conn
            .prepare("SELECT created_at, text FROM notes ORDER BY id DESC LIMIT 50")?;
        let v = st
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    fn group_stats(
        &self,
        where_sql: &str,
        args: &[&dyn rusqlite::ToSql],
        group: &str,
    ) -> R<Vec<ThemeStat>> {
        let (key, label) = match group {
            "street" => ("a.street", "a.street"),
            "position" => ("a.position", "a.position"),
            "sampling_mode" => ("a.sampling_mode", "a.sampling_mode"),
            "pack_version" => (
                "a.pack_id || ' ' || a.pack_version",
                "a.pack_id || ' v' || a.pack_version",
            ),
            "board" => ("a.board", "a.board"),
            _ => ("a.theme", "MAX(a.theme_label)"),
        };
        let sql = format!(
            "SELECT {key}, {label}, COUNT(*), AVG(a.loss_bb), SUM(a.loss_bb),
                    100.0 * SUM(CASE WHEN a.grade IN ('best','close') THEN 1 ELSE 0 END) / COUNT(*)
             FROM attempts a {where_sql} GROUP BY {key} ORDER BY SUM(a.loss_bb) DESC"
        );
        let mut st = self.db.conn.prepare(&sql)?;
        let rows = st.query_map(args, |r| {
            Ok(ThemeStat {
                key: r.get(0)?,
                label: r.get(1)?,
                n: r.get(2)?,
                avg_loss_bb: r.get(3)?,
                total_loss_bb: r.get(4)?,
                best_or_close_pct: r.get(5)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    pub fn progress(&self, f: &ProgressFilter) -> R<ProgressReport> {
        let mut conds = Vec::new();
        let mut vals: Vec<String> = Vec::new();
        let mut add = |col: &str, v: &Option<String>, op: &str| {
            if let Some(v) = v.as_ref().filter(|s| !s.is_empty()) {
                vals.push(if op == "LIKE" {
                    format!("%{v}%")
                } else {
                    v.clone()
                });
                conds.push(format!("{col} {op} ?{}", vals.len()));
            }
        };
        add("a.position", &f.position, "=");
        add("a.street", &f.street, "=");
        add("a.pot_type", &f.pot_type, "=");
        add("a.textures", &f.texture, "LIKE");
        add("a.theme", &f.theme, "=");
        add("a.sampling_mode", &f.mode, "=");
        add("a.created_at", &f.since, ">=");
        add("a.pack_version", &f.pack_version, "=");
        let where_sql = if conds.is_empty() {
            String::new()
        } else {
            format!("WHERE {}", conds.join(" AND "))
        };
        let args: Vec<&dyn rusqlite::ToSql> =
            vals.iter().map(|v| v as &dyn rusqlite::ToSql).collect();
        let groups =
            self.group_stats(&where_sql, &args, f.group_by.as_deref().unwrap_or("theme"))?;
        let total = self.db.conn.query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(AVG(a.loss_bb),0), COALESCE(SUM(a.loss_bb),0),
                        COALESCE(100.0 * SUM(CASE WHEN a.grade IN ('best','close') THEN 1 ELSE 0 END) / NULLIF(COUNT(*),0), 0)
                 FROM attempts a {where_sql}"
            ),
            args.as_slice(),
            |r| {
                Ok(ThemeStat {
                    key: "total".into(),
                    label: "Total".into(),
                    n: r.get(0)?,
                    avg_loss_bb: r.get(1)?,
                    total_loss_bb: r.get(2)?,
                    best_or_close_pct: r.get(3)?,
                })
            },
        )?;
        let mut st = self.db.conn.prepare(&format!(
            "SELECT {ATTEMPT_COLUMNS} FROM attempts a {where_sql} ORDER BY a.id DESC LIMIT 50"
        ))?;
        let recent = st
            .query_map(args.as_slice(), map_attempt)?
            .collect::<Result<_, _>>()?;
        Ok(ProgressReport {
            total,
            groups,
            recent,
        })
    }

    pub fn study(&self, exercise_id: &str, highlight: Option<String>) -> R<StudyView> {
        let ex = self
            .pack
            .exercise(exercise_id)
            .ok_or_else(|| TrainerError::Msg("exercício inexistente".into()))?;
        let (sol, node) = self
            .pack
            .node(ex)
            .ok_or_else(|| TrainerError::Msg("nó ausente".into()))?;
        let cpb = self.cpb(sol);
        let a = node.num_actions();
        let mut cells: BTreeMap<String, (f64, Vec<f64>, Vec<f64>)> = BTreeMap::new();
        let mut combos = Vec::with_capacity(node.combos.len());
        for (i, c) in node.combos.iter().enumerate() {
            let combo = Combo::parse(c)?;
            let hand = combo.hand_class();
            let w = node.weights[i] as f64;
            let e = cells
                .entry(hand.clone())
                .or_insert((0.0, vec![0.0; a], vec![0.0; a]));
            e.0 += w;
            for j in 0..a {
                e.1[j] += w * node.strategy[j][i] as f64;
                e.2[j] += w * node.ev[j][i] as f64 / cpb;
            }
            combos.push(ComboRow {
                combo: c.clone(),
                hand,
                weight: node.weights[i],
                equity: node.equity[i],
                strategy: (0..a).map(|j| node.strategy[j][i]).collect(),
                ev_bb: (0..a).map(|j| node.ev[j][i] as f64 / cpb).collect(),
            });
        }
        let grid = grid_hands()
            .into_iter()
            .map(|h| match cells.get(&h) {
                Some((w, s, e)) if *w > 0.0 => GridCell {
                    hand: h,
                    weight: *w,
                    strategy: s.iter().map(|x| x / w).collect(),
                    ev_bb: e.iter().map(|x| x / w).collect(),
                },
                _ => GridCell {
                    hand: h,
                    weight: 0.0,
                    strategy: vec![],
                    ev_bb: vec![],
                },
            })
            .collect();
        Ok(StudyView {
            exercise_id: ex.id.clone(),
            theme_label: ex.theme_label.clone(),
            position: node.position.clone(),
            board: node.board.clone(),
            pot_bb: node.pot_chips as f64 / cpb,
            to_call_bb: node.to_call_chips as f64 / cpb,
            stack_bb: node.stack_chips as f64 / cpb,
            preflop_line: sol
                .spot
                .preflop
                .as_ref()
                .map(|p| p.line.clone())
                .unwrap_or_default(),
            line: node.line.clone(),
            actions: node
                .actions
                .iter()
                .enumerate()
                .map(|(i, x)| ActionOption {
                    index: i,
                    label: x.label.clone(),
                    kind: format!("{:?}", x.kind).to_lowercase(),
                })
                .collect(),
            aggregate: node.aggregate_frequencies(),
            grid,
            combos,
            highlight,
            solve_info: format!(
                "{} @ {} · explorabilidade {:.3}% do pote · {} · {}",
                sol.engine.name,
                &sol.engine.git_rev[..10],
                sol.solve.final_exploitability_pct_pot,
                sol.spot.rake.structure,
                sol.created_at
            ),
            ranges_note: format!(
                "Ranges: {} (OOP) / {} (IP). Premissas de estudo.",
                sol.spot.oop_range.source, sol.spot.ip_range.source
            ),
        })
    }

    pub fn history(&self, limit: i64) -> R<Vec<AttemptRow>> {
        let mut st = self.db.conn.prepare(&format!(
            "SELECT {ATTEMPT_COLUMNS} FROM attempts a ORDER BY a.id DESC LIMIT ?1"
        ))?;
        let v = st
            .query_map([limit], map_attempt)?
            .collect::<Result<_, _>>()?;
        Ok(v)
    }

    pub fn export_csv(&self, path: &Path) -> R<usize> {
        let mut st = self.db.conn.prepare(
            "SELECT a.id, a.created_at, a.session_id, a.sampling_mode, a.pack_id, a.pack_version, a.engine_rev, a.exercise_id,
                    a.theme, a.street, a.position, a.pot_type, a.board, a.textures, a.combo, a.chosen_label, a.best_label,
                    a.best_ev_chips, a.chosen_ev_chips, a.loss_bb, a.loss_pct_pot, a.grade, a.chosen_frequency, a.decision_ms
             FROM attempts a ORDER BY a.id",
        )?;
        let cols = st.column_count();
        let mut out = String::from(
            "id,created_at,session_id,sampling_mode,pack_id,pack_version,engine_rev,exercise_id,theme,street,position,pot_type,board,textures,combo,chosen,best,best_ev_chips,chosen_ev_chips,loss_bb,loss_pct_pot,grade,chosen_frequency,decision_ms\n",
        );
        let mut rows = st.query([])?;
        let mut n = 0;
        while let Some(r) = rows.next()? {
            let mut fields = Vec::with_capacity(cols);
            for i in 0..cols {
                let v: rusqlite::types::Value = r.get(i)?;
                let s = match v {
                    rusqlite::types::Value::Null => String::new(),
                    rusqlite::types::Value::Integer(x) => x.to_string(),
                    rusqlite::types::Value::Real(x) => format!("{x}"),
                    rusqlite::types::Value::Text(t) => t,
                    rusqlite::types::Value::Blob(_) => String::new(),
                };
                fields.push(if s.contains([',', '"', '\n']) {
                    format!("\"{}\"", s.replace('"', "\"\""))
                } else {
                    s
                });
            }
            out.push_str(&fields.join(","));
            out.push('\n');
            n += 1;
        }
        poker_core::solution::write_atomic(path, out.as_bytes())?;
        Ok(n)
    }
}

/// Number with Brazilian decimal comma.
fn br(v: f64, digits: usize) -> String {
    format!("{v:.digits$}").replace('.', ",")
}

/// Portuguese explanation built only from the solution data.
pub fn explain(node: &NodeSolution, fb: &Feedback, equity: f32, cpb: f64) -> Vec<String> {
    let best = &node.actions[fb.best_index].label;
    let chosen = &node.actions[fb.chosen_index].label;
    let mut v = Vec::new();
    v.push(format!(
        "Melhor ação para {}: {} (EV {} bb).",
        fb.combo,
        best,
        br(fb.best_ev_chips / cpb, 3)
    ));
    if fb.chosen_index == fb.best_index {
        v.push("Você escolheu a ação de maior EV.".into());
    } else {
        v.push(format!(
            "{} perde {} bb ({}% do pote) em relação à melhor.",
            chosen,
            br(fb.loss_bb, 3),
            br(fb.loss_pct_pot, 2)
        ));
        if fb.loss_chips <= fb.tolerance_chips {
            v.push(format!(
                "A diferença está dentro da tolerância ({} bb): decisões praticamente equivalentes neste modelo.",
                br(fb.tolerance_chips / cpb, 3)
            ));
        }
    }
    let f = fb.chosen_frequency as f64;
    if f >= 0.1 && fb.chosen_index != fb.best_index {
        v.push(format!(
            "O solver também usa {} com esta mão em {}% das vezes (estratégia mista).",
            chosen,
            br(f * 100.0, 0)
        ));
    } else if f < 0.01 {
        v.push(format!(
            "O solver praticamente nunca escolhe {} com esta mão.",
            chosen
        ));
    }
    v.push(format!(
        "Equidade da mão contra o range do adversário neste ponto: {}%.",
        br(equity as f64 * 100.0, 1)
    ));
    v
}
