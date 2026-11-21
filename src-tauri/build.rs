// Commands exposed to the UI. Each one gets an `allow-<name>` permission that must be granted
// explicitly in `capabilities/default.json` (no implicit access to app commands).
const COMMANDS: &[&str] = &[
    "app_info",
    "list_themes",
    "list_study_entries",
    "start_session",
    "next_exercise",
    "submit_answer",
    "end_session",
    "mark_review",
    "add_note",
    "list_notes",
    "study_node",
    "progress",
    "history",
    "export_csv",
    "backup_now",
    "get_pref",
    "set_pref",
    "hand_boards",
    "hand_prepare",
    "hand_status",
    "hand_cancel",
    "hand_start",
    "hand_act",
    "hand_view",
    "hand_history",
    "hand_review",
    "export_hands_csv",
    "check_update",
    "install_update",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
