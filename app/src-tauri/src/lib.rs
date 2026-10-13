//! IssueRelay's desktop app: the tray icon and the panel around the engine.
// Every module is `pub`: only this crate's own executable uses the library, and what the panel's
// commands (Task 4) use later, such as `AppState::host`, is then not dead code before that.
pub mod commands;
pub mod events;
pub mod i18n;
pub mod logging;
pub mod notify;
pub mod quit;
pub mod startup;
pub mod state;
#[cfg(test)]
mod test_support;
pub mod tray;
pub mod window;

use tauri::{Manager, RunEvent};

/// Passed by the login item, so the app starts in the tray (design 9.2: autostart is switched
/// on by the user in the settings).
pub const AUTOSTART_ARG: &str = "--autostart";

/// Starts the app and runs its event loop until it exits.
pub fn run() {
    // `args_os`: `args` would panic on an argument that is not valid Unicode.
    let autostarted = std::env::args_os().any(|arg| arg == AUTOSTART_ARG);
    let logs_dir = issuerelay_core::paths::AppPaths::system()
        .ok()
        .map(|paths| paths.logs_dir());
    let mut log_guard = logging::init(logs_dir.as_deref());
    let builder = tauri::Builder::default()
        // Must come first: a second launch only brings the running instance's panel forward.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show(app)
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        // By default the plugin adds a script to every page that opens clicked links itself; the
        // panel opens only Linear's links, through IssueRelay's own command and check (Task 4).
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .manage(state::AppState::new(autostarted))
        .manage(commands::PendingLogin::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::startup_state,
            commands::status,
            commands::list_jobs,
            commands::job,
            commands::job_log,
            commands::issues,
            commands::pause,
            commands::resume,
            commands::poll_now,
            commands::cancel_job,
            commands::retry_job,
            commands::mark_done,
            commands::bindings,
            commands::save_binding,
            commands::remove_binding,
            commands::projects,
            commands::settings,
            commands::save_settings,
            commands::doctor,
            commands::linear_login,
            commands::linear_cancel_login,
            commands::linear_use_api_key,
            commands::linear_logout,
            commands::linear_reconnect,
            commands::setup_action,
            commands::control_servers,
            commands::linear_server_named,
            commands::open_linear,
            commands::choose_directory,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::quit,
            commands::request_quit,
        ])
        .setup(|app| {
            tray::create(app.handle())?;
            if app.state::<state::AppState>().autostarted {
                // A login start stays in the tray: through the same `hide` as closing the panel,
                // so macOS shows no Dock icon either.
                window::hide(app.handle());
            }
            tauri::async_runtime::spawn(startup::start(app.handle().clone()));
            Ok(())
        })
        .on_window_event(window::on_event);
    // Cmd+Q must ask like the tray's Quit; Windows has no application menu (see `quit::app_menu`).
    #[cfg(target_os = "macos")]
    let builder = builder.menu(quit::app_menu);
    builder
        .build(tauri::generate_context!())
        .expect("IssueRelay could not start")
        .run(move |app, event| {
            let exiting = matches!(event, RunEvent::Exit);
            quit::on_run_event(app, event);
            if exiting {
                // `run` ends the process with `std::process::exit`, which runs no destructors:
                // the log file's last lines are written out here.
                drop(log_guard.take());
            }
        });
}
