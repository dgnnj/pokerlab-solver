//! PokerLab desktop app (Tauri 2). Typed commands over the trainer, SQLite and the updater.

pub mod db;
pub mod handplay;
pub mod trainer;
mod updater;

use std::path::PathBuf;
use std::sync::Mutex;

use serde::Serialize;
use tauri::{Manager, State};

use trainer::{
    FeedbackView, Mode, Presented, ProgressFilter, ProgressReport, SessionInfo, SessionSummary,
    StudyEntry, StudyView, ThemeInfo, Trainer, TrainerError,
};

pub const BUNDLED_PACK: &str = "btn-vs-bb-srp-100bb";

pub struct AppState {
    pub trainer: Mutex<Option<Trainer>>,
    pub hands: handplay::HandEngine,
    pub startup_error: Mutex<Option<String>>,
    pub data_dir: PathBuf,
    pub health: Mutex<HealthInfo>,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct HealthInfo {
    pub version: String,
    pub previous_version: Option<String>,
    pub first_run: bool,
    pub updated: bool,
    pub checks: Vec<String>,
}

type CmdResult<T> = Result<T, TrainerError>;

fn with_trainer<T>(
    state: &State<AppState>,
    f: impl FnOnce(&mut Trainer) -> CmdResult<T>,
) -> CmdResult<T> {
    let mut g = state
        .trainer
        .lock()
        .map_err(|_| TrainerError::Msg("estado bloqueado".into()))?;
    match g.as_mut() {
        Some(t) => f(t),
        None => Err(TrainerError::Msg(
            state
                .startup_error
                .lock()
                .ok()
                .and_then(|e| e.clone())
                .unwrap_or_else(|| "treinador não inicializado".into()),
        )),
    }
}

#[derive(Serialize)]
struct AppInfo {
    version: String,
    data_dir: String,
    db_path: String,
    db_schema: i64,
    pack_id: String,
    pack_version: String,
    pack_title: String,
    pack_exercises: usize,
    pack_license: String,
    pack_provenance: String,
    engine_rev: String,
    review_due: i64,
    open_session: Option<SessionInfo>,
    health: HealthInfo,
    startup_error: Option<String>,
    update_channel: updater::ChannelInfo,
}

#[tauri::command]
fn app_info(app: tauri::AppHandle, state: State<AppState>) -> CmdResult<AppInfo> {
    let version = app.package_info().version.to_string();
    let health = state.health.lock().map(|h| h.clone()).unwrap_or_default();
    let startup_error = state.startup_error.lock().ok().and_then(|e| e.clone());
    let channel = updater::channel_info(&app);
    let g = state
        .trainer
        .lock()
        .map_err(|_| TrainerError::Msg("estado bloqueado".into()))?;
    let base = AppInfo {
        version,
        data_dir: state.data_dir.display().to_string(),
        db_path: String::new(),
        db_schema: 0,
        pack_id: String::new(),
        pack_version: String::new(),
        pack_title: String::new(),
        pack_exercises: 0,
        pack_license: String::new(),
        pack_provenance: String::new(),
        engine_rev: String::new(),
        review_due: 0,
        open_session: None,
        health,
        startup_error,
        update_channel: channel,
    };
    let Some(t) = g.as_ref() else { return Ok(base) };
    let m = &t.pack.manifest;
    Ok(AppInfo {
        db_path: t.db.path.display().to_string(),
        db_schema: t.db.schema_version()?,
        pack_id: m.pack_id.clone(),
        pack_version: m.version.clone(),
        pack_title: m.title.clone(),
        pack_exercises: m.exercises.len(),
        pack_license: m.license.clone(),
        pack_provenance: m.provenance.clone(),
        engine_rev: m.engine.git_rev.clone(),
        review_due: t.review_due_count()?,
        open_session: t.last_open_session()?,
        ..base
    })
}

#[tauri::command]
fn list_themes(state: State<AppState>) -> CmdResult<Vec<ThemeInfo>> {
    with_trainer(&state, |t| Ok(t.themes()))
}

#[tauri::command]
fn list_study_entries(state: State<AppState>) -> CmdResult<Vec<StudyEntry>> {
    with_trainer(&state, |t| Ok(t.study_entries()))
}

#[tauri::command]
fn start_session(
    state: State<AppState>,
    mode: Mode,
    theme: Option<String>,
    count: i64,
) -> CmdResult<SessionInfo> {
    with_trainer(&state, |t| t.start_session(mode, theme, count))
}

#[tauri::command]
fn next_exercise(state: State<AppState>, session_id: i64) -> CmdResult<Option<Presented>> {
    with_trainer(&state, |t| t.next_exercise(session_id))
}

#[tauri::command]
fn submit_answer(
    state: State<AppState>,
    token: String,
    action: usize,
    decision_ms: Option<i64>,
) -> CmdResult<FeedbackView> {
    with_trainer(&state, |t| t.submit(&token, action, decision_ms))
}

#[tauri::command]
fn end_session(state: State<AppState>, session_id: i64) -> CmdResult<SessionSummary> {
    with_trainer(&state, |t| t.end_session(session_id))
}

#[tauri::command]
fn mark_review(state: State<AppState>, attempt_id: i64, tags: Option<String>) -> CmdResult<()> {
    with_trainer(&state, |t| {
        t.mark_review(attempt_id, tags.as_deref().unwrap_or(""))
    })
}

#[tauri::command]
fn add_note(state: State<AppState>, session_id: Option<i64>, text: String) -> CmdResult<()> {
    with_trainer(&state, |t| t.add_note(session_id, &text))
}

#[tauri::command]
fn list_notes(state: State<AppState>) -> CmdResult<Vec<(String, String)>> {
    with_trainer(&state, |t| t.notes())
}

#[tauri::command]
fn study_node(
    state: State<AppState>,
    exercise_id: String,
    highlight: Option<String>,
) -> CmdResult<StudyView> {
    with_trainer(&state, |t| t.study(&exercise_id, highlight))
}

#[tauri::command]
fn progress(state: State<AppState>, filter: ProgressFilter) -> CmdResult<ProgressReport> {
    with_trainer(&state, |t| t.progress(&filter))
}

#[tauri::command]
fn history(state: State<AppState>, limit: Option<i64>) -> CmdResult<Vec<db::AttemptRow>> {
    with_trainer(&state, |t| t.history(limit.unwrap_or(100)))
}

#[tauri::command]
fn export_csv(state: State<AppState>) -> CmdResult<String> {
    let path = state
        .data_dir
        .join("exports")
        .join(format!("pokerlab-historico-{}.csv", db::unix_now()));
    with_trainer(&state, |t| {
        let n = t.export_csv(&path)?;
        Ok(format!("{n} tentativas exportadas para {}", path.display()))
    })
}

#[tauri::command]
fn backup_now(state: State<AppState>) -> CmdResult<String> {
    with_trainer(&state, |t| {
        let p = t.db.backup_to(&t.db.backup_path("manual"))?;
        Ok(p.display().to_string())
    })
}

#[tauri::command]
fn get_pref(state: State<AppState>, key: String) -> CmdResult<Option<String>> {
    with_trainer(&state, |t| Ok(t.db.get_pref(&key)?))
}

#[tauri::command]
fn set_pref(state: State<AppState>, key: String, value: String) -> CmdResult<()> {
    const ALLOWED: &[&str] = &[
        "auto_update",
        "update_postponed_until",
        "preferred_count",
        "show_pct_pot",
    ];
    if !ALLOWED.contains(&key.as_str()) {
        return Err(TrainerError::Msg(format!(
            "preferência desconhecida: {key}"
        )));
    }
    with_trainer(&state, |t| Ok(t.db.set_pref(&key, &value)?))
}

#[derive(Serialize)]
struct BoardOption {
    solution_id: String,
    board: String,
    textures: Vec<String>,
    pack_exploitability_pct: f64,
    pack_solve_ms: u64,
    memory_mib: u64,
}

#[tauri::command]
fn hand_boards(state: State<AppState>) -> CmdResult<Vec<BoardOption>> {
    with_trainer(&state, |t| {
        let mut v: Vec<BoardOption> = t
            .pack
            .solutions
            .values()
            .map(|s| BoardOption {
                solution_id: s.solution_id.clone(),
                board: s.spot.board.clone(),
                textures: poker_core::cards::parse_board(&s.spot.board)
                    .map(|b| poker_core::texture::flop_texture_tags(&b))
                    .unwrap_or_default(),
                pack_exploitability_pct: s.solve.final_exploitability_pct_pot,
                pack_solve_ms: s.solve.solve_ms,
                memory_mib: s.solve.memory_estimate_bytes >> 20,
            })
            .collect();
        v.sort_by(|a, b| a.board.cmp(&b.board));
        Ok(v)
    })
}

#[tauri::command]
fn hand_prepare(state: State<AppState>, solution_id: String) -> CmdResult<handplay::EngineStatus> {
    let src = with_trainer(&state, |t| {
        let s = t
            .pack
            .solutions
            .get(&solution_id)
            .ok_or_else(|| TrainerError::Msg(format!("solução desconhecida: {solution_id}")))?;
        Ok(handplay::BoardSource {
            solution_id: s.solution_id.clone(),
            spot: s.spot.clone(),
            pack_config_hash: s.config_hash.clone(),
            pack_exploitability_pct: s.solve.final_exploitability_pct_pot,
            pack_id: t.pack.manifest.pack_id.clone(),
            pack_version: t.pack.manifest.version.clone(),
        })
    })?;
    state.hands.prepare(src)
}

#[tauri::command]
fn hand_status(state: State<AppState>) -> CmdResult<handplay::EngineStatus> {
    state.hands.status()
}

#[tauri::command]
fn hand_cancel(state: State<AppState>) -> CmdResult<handplay::EngineStatus> {
    state.hands.cancel()
}

#[tauri::command]
fn hand_start(state: State<AppState>, hero: String) -> CmdResult<handplay::HandView> {
    with_trainer(&state, |t| state.hands.start(&t.db, &hero))
}

#[tauri::command]
fn hand_act(
    state: State<AppState>,
    token: String,
    action: usize,
    decision_ms: Option<i64>,
) -> CmdResult<handplay::HandView> {
    with_trainer(&state, |t| {
        state.hands.act(&t.db, &token, action, decision_ms)
    })
}

#[tauri::command]
fn hand_view(state: State<AppState>, token: String) -> CmdResult<handplay::HandView> {
    state.hands.view(&token)
}

#[tauri::command]
fn hand_history(state: State<AppState>, limit: Option<i64>) -> CmdResult<handplay::HandHistory> {
    with_trainer(&state, |t| handplay::history(&t.db, limit.unwrap_or(30)))
}

#[tauri::command]
fn hand_review(state: State<AppState>, id: i64) -> CmdResult<handplay::HandReview> {
    with_trainer(&state, |t| {
        handplay::review(&t.db, id, |sid| {
            t.pack
                .solutions
                .get(sid)
                .map(|s| s.spot.chips_per_bb as f64)
        })
    })
}

#[tauri::command]
fn export_hands_csv(state: State<AppState>) -> CmdResult<String> {
    let path = state
        .data_dir
        .join("exports")
        .join(format!("pokerlab-maos-{}.csv", db::unix_now()));
    with_trainer(&state, |t| {
        let (hands, rows) = handplay::export_csv(&t.db, &path)?;
        Ok(format!(
            "{hands} mãos ({rows} linhas, uma por decisão) exportadas para {}",
            path.display()
        ))
    })
}

fn init_trainer(
    app: &tauri::AppHandle,
    data_dir: &std::path::Path,
) -> Result<(Trainer, HealthInfo), String> {
    let version = app.package_info().version.to_string();
    let db = db::Db::open(&data_dir.join("pokerlab.sqlite3")).map_err(|e| e.to_string())?;
    let mut health = HealthInfo {
        version: version.clone(),
        ..Default::default()
    };
    let prev = db.get_pref("running_version").map_err(|e| e.to_string())?;
    health.first_run = prev.is_none();
    health.updated = prev.as_deref().is_some_and(|p| p != version);
    health.previous_version = prev;
    health.checks.push(format!(
        "banco aberto (schema v{})",
        db.schema_version().map_err(|e| e.to_string())?
    ));

    let resource_dir = app.path().resource_dir().map_err(|e| e.to_string())?;
    let bundled = std::env::var_os("POKERLAB_PACK_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            resource_dir
                .join("resources")
                .join("packs")
                .join(BUNDLED_PACK)
        });
    let pack = trainer::install_pack(&db, &bundled, &data_dir.join("packs"))
        .map_err(|e| format!("pacote inicial: {e}"))?;
    health.checks.push(format!(
        "pacote {} v{} íntegro ({} exercícios, SHA-256 verificado)",
        pack.manifest.pack_id,
        pack.manifest.version,
        pack.manifest.exercises.len()
    ));
    db.set_pref("running_version", &version)
        .map_err(|e| e.to_string())?;
    if db
        .get_pref("auto_update")
        .map_err(|e| e.to_string())?
        .is_none()
    {
        db.set_pref("auto_update", "true")
            .map_err(|e| e.to_string())?;
    }
    db.set_pref("last_health_check", &db::now())
        .map_err(|e| e.to_string())?;
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(7);
    Ok((Trainer::new(db, pack, seed), health))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let data_dir = std::env::var_os("POKERLAB_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or(app.path().app_local_data_dir()?);
            std::fs::create_dir_all(&data_dir)?;
            let handle = app.handle().clone();
            let (trainer, health, err) = match init_trainer(&handle, &data_dir) {
                Ok((t, h)) => (Some(t), h, None),
                Err(e) => (None, HealthInfo::default(), Some(e)),
            };
            if let Some(e) = &err {
                let _ = std::fs::write(
                    data_dir.join("startup-error.log"),
                    format!("{} {e}\n", db::now()),
                );
            }
            let seed = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(11);
            app.manage(AppState {
                trainer: Mutex::new(trainer),
                hands: handplay::HandEngine::new(seed.rotate_left(17)),
                startup_error: Mutex::new(err),
                data_dir,
                health: Mutex::new(health),
            });
            app.manage(updater::UpdateState::default());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app_info,
            list_themes,
            list_study_entries,
            start_session,
            next_exercise,
            submit_answer,
            end_session,
            mark_review,
            add_note,
            list_notes,
            study_node,
            progress,
            history,
            export_csv,
            backup_now,
            get_pref,
            set_pref,
            hand_boards,
            hand_prepare,
            hand_status,
            hand_cancel,
            hand_start,
            hand_act,
            hand_view,
            hand_history,
            hand_review,
            export_hands_csv,
            updater::check_update,
            updater::install_update,
        ])
        .run(tauri::generate_context!())
        .expect("erro ao iniciar o PokerLab");
}
