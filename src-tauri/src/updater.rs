//! Launcher-side update flow on top of the official `tauri-plugin-updater`.
//!
//! The plugin verifies the minisign signature of the downloaded installer against the public key
//! embedded in `tauri.conf.json` before running it. Endpoint resolution:
//! 1. `POKERLAB_UPDATE_ENDPOINT` (test feeds), else
//! 2. `plugins.updater.endpoints` from the bundled config.
//!
//! While the endpoint still contains the `OWNER/REPO` placeholder the channel is reported as
//! "not configured" and no network request is made.

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::ipc::Channel;
use tauri::Manager;
use tauri_plugin_updater::{Update, UpdaterExt};
use url::Url;

use crate::AppState;

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ChannelInfo {
    pub configured: bool,
    pub endpoint: String,
    pub note: String,
}

fn configured_endpoint(app: &tauri::AppHandle) -> Option<String> {
    if let Ok(e) = std::env::var("POKERLAB_UPDATE_ENDPOINT") {
        if !e.trim().is_empty() {
            return Some(e);
        }
    }
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("endpoints"))
        .and_then(|e| e.as_array())
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .map(str::to_string)
}

pub fn channel_info(app: &tauri::AppHandle) -> ChannelInfo {
    match configured_endpoint(app) {
        Some(e) if e.contains("OWNER/REPO") => ChannelInfo {
            configured: false,
            endpoint: e,
            note:
                "Canal de atualização ainda não configurado (repositório de distribuição pendente)."
                    .into(),
        },
        Some(e) => ChannelInfo {
            configured: true,
            endpoint: e,
            note: "GitHub Releases (canal estável)".into(),
        },
        None => ChannelInfo {
            configured: false,
            endpoint: String::new(),
            note: "Sem endpoint de atualização.".into(),
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateCheck {
    /// up_to_date | available | unconfigured | error
    pub status: String,
    pub current_version: String,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub date: Option<String>,
    pub error: Option<String>,
}

fn build_updater(
    app: &tauri::AppHandle,
    info: &ChannelInfo,
) -> Result<tauri_plugin_updater::Updater, String> {
    let url = Url::parse(&info.endpoint).map_err(|e| format!("endpoint inválido: {e}"))?;
    app.updater_builder()
        .endpoints(vec![url])
        .and_then(|b| b.timeout(Duration::from_secs(8)).build())
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn check_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
) -> Result<UpdateCheck, String> {
    let current = app.package_info().version.to_string();
    let info = channel_info(&app);
    let base = UpdateCheck {
        status: "unconfigured".into(),
        current_version: current,
        version: None,
        notes: None,
        date: None,
        error: None,
    };
    if !info.configured {
        return Ok(UpdateCheck {
            error: Some(info.note),
            ..base
        });
    }
    let updater = match build_updater(&app, &info) {
        Ok(u) => u,
        Err(e) => {
            return Ok(UpdateCheck {
                status: "error".into(),
                error: Some(e),
                ..base
            })
        }
    };
    match updater.check().await {
        Ok(Some(u)) => {
            let out = UpdateCheck {
                status: "available".into(),
                version: Some(u.version.clone()),
                notes: u.body.clone(),
                date: u.date.map(|d| d.to_string()),
                ..base
            };
            *state.pending.lock().map_err(|_| "estado bloqueado")? = Some(u);
            Ok(out)
        }
        Ok(None) => Ok(UpdateCheck {
            status: "up_to_date".into(),
            ..base
        }),
        Err(e) => Ok(UpdateCheck {
            status: "error".into(),
            error: Some(e.to_string()),
            ..base
        }),
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum DownloadEvent {
    Started {
        content_length: Option<u64>,
    },
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    Finished,
}

/// Downloads to the updater's staging location, verifies the signature (inside the plugin) and
/// launches the NSIS installer in passive mode. On Windows the app exits while the installer
/// runs; the installer relaunches it. Local data in %LOCALAPPDATA% is untouched.
#[tauri::command]
pub async fn install_update(
    app: tauri::AppHandle,
    state: tauri::State<'_, UpdateState>,
    on_event: Channel<DownloadEvent>,
) -> Result<(), String> {
    let pending = state.pending.lock().map_err(|_| "estado bloqueado")?.take();
    let update = match pending {
        Some(u) => u,
        // Retry after a failure: query the feed again instead of reusing a consumed handle.
        None => {
            let info = channel_info(&app);
            if !info.configured {
                return Err(info.note);
            }
            build_updater(&app, &info)?
                .check()
                .await
                .map_err(|e| format!("falha ao consultar o feed: {e}"))?
                .ok_or("nenhuma atualização disponível")?
        }
    };
    // Flush the SQLite WAL so the database file is self-contained before the process exits.
    if let Some(s) = app.try_state::<AppState>() {
        if let Ok(g) = s.trainer.lock() {
            if let Some(t) = g.as_ref() {
                let _ = t.db.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);");
            }
        }
    }
    let mut downloaded: u64 = 0;
    let mut started = false;
    let ev = on_event.clone();
    let result = update
        .download_and_install(
            |chunk, total| {
                if !started {
                    let _ = ev.send(DownloadEvent::Started {
                        content_length: total,
                    });
                    started = true;
                }
                downloaded += chunk as u64;
                let _ = ev.send(DownloadEvent::Progress {
                    downloaded,
                    content_length: total,
                });
            },
            || {
                let _ = on_event.send(DownloadEvent::Finished);
            },
        )
        .await;
    match result {
        Ok(()) => {
            app.restart();
        }
        Err(e) => Err(format!("falha na atualização: {e}")),
    }
}
