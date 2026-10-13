//! IssueRelay's desktop app: the tray icon and the panel around the engine.

pub mod i18n;

/// Starts the app and runs its event loop until it exits.
pub fn run() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("IssueRelay could not start");
}
