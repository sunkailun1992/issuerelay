//! The panel window: shown on demand, hidden instead of closed. On macOS the Dock icon follows
//! the window: a tray app shows no Dock icon while its panel is hidden, also after a login start
//! (lib.rs hides through `hide` too). Windows needs nothing of the kind: a hidden window has no
//! taskbar button. Opening the app again brings the panel back: macOS reports it to the running
//! process (`on_reopen`), Windows starts a second one (the single-instance callback in lib.rs).
use tauri::{AppHandle, Manager, Runtime, WindowEvent};

use crate::{
    logging,
    quit::{self, QuitEvent, QuitStep},
};

/// Shows the panel, restored and focused; on macOS with its Dock icon.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
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
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
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
pub fn on_event<R: Runtime>(window: &tauri::Window<R>, event: &WindowEvent) {
    if let WindowEvent::CloseRequested { api, .. } = event
        && window.label() == "main"
        && quit::handle(window.app_handle(), QuitEvent::WindowClosed) == QuitStep::HideWindow
    {
        api.prevent_close();
        hide(window.app_handle());
    }
}

/// The app was opened again while it runs, so the panel comes forward as it does for a second
/// launch (design 4.1). Only macOS reports this to the running process (`RunEvent::Reopen`, which
/// `quit::on_run_event` hands here); on Windows opening the app again starts a second process, and
/// the single-instance callback in lib.rs shows the panel.
///
/// The panel is shown whatever `has_visible_windows` says: it is AppKit's finding about every
/// window of the app, not about the panel, and `show` on an open panel only brings it forward.
pub fn on_reopen<R: Runtime>(app: &AppHandle<R>, has_visible_windows: bool) {
    reopen_with(has_visible_windows, || show(app));
}

/// `on_reopen` with the showing handed in, so that a test can see whether it was asked for:
/// `RunEvent::Reopen` cannot be built outside tauri (a non-exhaustive variant), the mock runtime
/// sends none, and its windows do not keep whether they are shown.
fn reopen_with(has_visible_windows: bool, show_panel: impl FnOnce()) {
    tracing::info!(
        has_visible_windows,
        "the app was opened again; showing the panel"
    );
    show_panel();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whether a reopen with `has_visible_windows` asks for the panel to be shown.
    fn shown_on_reopen(has_visible_windows: bool) -> bool {
        let mut shown = false;
        reopen_with(has_visible_windows, || shown = true);
        shown
    }

    #[test]
    fn opening_the_app_again_shows_the_panel_whether_or_not_a_window_is_visible() {
        // The decision behind the `RunEvent::Reopen` arm of `quit::on_run_event`, which no test
        // can reach: tauri 2.12.3 marks the variant `#[non_exhaustive]`, so it cannot be built
        // here (E0639), and the mock runtime sends none. That the arm calls `on_reopen` is for the
        // acceptance on the real app to check.
        assert!(shown_on_reopen(false), "the panel was closed to the tray");
        assert!(
            shown_on_reopen(true),
            "a window is visible, but perhaps not the panel"
        );
    }
}
