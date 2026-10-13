//! The panel window: shown on demand, hidden instead of closed. On macOS the Dock icon follows
//! the window: a tray app shows no Dock icon while its panel is hidden, also after a login start
//! (lib.rs hides through `hide` too). Windows needs nothing of the kind: a hidden window has no
//! taskbar button.
use tauri::{AppHandle, Manager, WindowEvent};

use crate::{
    logging,
    quit::{self, QuitEvent, QuitStep},
};

/// Shows the panel, restored and focused; on macOS with its Dock icon.
pub fn show(app: &AppHandle) {
    // `AppHandle::set_activation_policy` returns a `Result` in tauri 2.12.3.
    #[cfg(target_os = "macos")]
    logging::warn_failed(
        app.set_activation_policy(tauri::ActivationPolicy::Regular),
        "showing the Dock icon",
    );
    match app.get_webview_window("main") {
        Some(window) => {
            logging::warn_failed(window.show(), "showing the panel");
            logging::warn_failed(window.unminimize(), "restoring the panel");
            logging::warn_failed(window.set_focus(), "focusing the panel");
        }
        None => tracing::warn!("the panel window is missing"),
    }
}

/// Hides the panel; on macOS its Dock icon goes with it.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        logging::warn_failed(window.hide(), "hiding the panel");
    }
    #[cfg(target_os = "macos")]
    logging::warn_failed(
        app.set_activation_policy(tauri::ActivationPolicy::Accessory),
        "hiding the Dock icon",
    );
}

/// Closing the panel hides it; monitoring goes on in the tray. The quit state machine decides:
/// only once quitting is done may the window really close, so closing it while the app waits for
/// runs neither ends the app early nor loses the "stop now" prompt.
pub fn on_event(window: &tauri::Window, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event
        && window.label() == "main"
        && quit::handle(window.app_handle(), QuitEvent::WindowClosed) == QuitStep::HideWindow
    {
        api.prevent_close();
        hide(window.app_handle());
    }
}
