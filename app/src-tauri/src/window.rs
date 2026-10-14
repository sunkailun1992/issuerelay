//! The panel window: shown on demand, hidden instead of closed. On macOS the Dock icon goes with
//! it: by default the icon stays for as long as the app runs, since a tray item alone is easily
//! lost on a crowded menu bar and then nobody can find the app (design 4.1); with "Show in Dock"
//! off (`Settings::show_in_dock`) the icon follows the window as it used to, shown only while the
//! panel is, also after a login start (lib.rs hides through `hide` too). `dock_policy` is that
//! one decision. Windows needs nothing of the kind: it has no Dock, and a window's taskbar button
//! comes and goes with the window itself (a hidden window has none). Opening the app again brings
//! the panel back: macOS reports it to the running process (`on_reopen`; clicking the Dock icon
//! does the same), Windows starts a second one (the single-instance callback in lib.rs).
use issuerelay_core::config::Settings;
use tauri::{AppHandle, Manager, Runtime, WindowEvent};

use crate::{
    logging,
    quit::{self, QuitEvent, QuitStep},
    state::AppState,
    tray,
};

/// What the Dock shows for the app: macOS's activation policy, as far as this app uses it. Defined
/// and decided on every platform, so that both platforms' tests check the decision; only macOS
/// acts on it (`apply_dock`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DockPolicy {
    /// An ordinary app: the Dock icon (and the application menu) shows.
    Regular,
    /// No Dock icon: the app lives in the menu bar and its panel alone.
    Accessory,
}

impl DockPolicy {
    #[cfg(target_os = "macos")]
    fn activation_policy(self) -> tauri::ActivationPolicy {
        match self {
            Self::Regular => tauri::ActivationPolicy::Regular,
            Self::Accessory => tauri::ActivationPolicy::Accessory,
        }
    }
}

/// Whether the Dock shows the app. With "Show in Dock" on it always does; off, only while the panel
/// is visible, which is how the app behaved before the setting existed.
pub fn dock_policy(show_in_dock: bool, panel_visible: bool) -> DockPolicy {
    if show_in_dock || panel_visible {
        DockPolicy::Regular
    } else {
        DockPolicy::Accessory
    }
}

/// `dock_policy` for the settings as far as they are known. Until the host has started and the
/// settings are read (`None`) the setting counts as off, so the icon follows the panel as it
/// always did.
fn dock_policy_for(settings: Option<&Settings>, panel_visible: bool) -> DockPolicy {
    dock_policy(settings.is_some_and(|s| s.show_in_dock), panel_visible)
}

/// The settings once the host has started, which is when they are first read; `None` before that,
/// and for good when the host could not start.
fn settings_if_started<R: Runtime>(app: &AppHandle<R>) -> Option<Settings> {
    let state = app.try_state::<AppState>()?;
    state.try_host().map(|host| host.engine().settings())
}

/// Gives the app the Dock icon that `policy` says, or takes it away.
fn apply_dock<R: Runtime>(app: &AppHandle<R>, policy: DockPolicy) {
    // `AppHandle::set_activation_policy` returns a `Result` in tauri 2.12.3.
    #[cfg(target_os = "macos")]
    logging::warn_failed(
        app.set_activation_policy(policy.activation_policy()),
        match policy {
            DockPolicy::Regular => "showing the Dock icon",
            DockPolicy::Accessory => "hiding the Dock icon",
        },
    );
    // Windows has no Dock: nothing to give or take away.
    #[cfg(not(target_os = "macos"))]
    let _ = (app, policy);
}

/// Puts the Dock icon where `dock_policy` says for the settings as they are, with the panel
/// visible or not.
fn apply_for_panel<R: Runtime>(app: &AppHandle<R>, panel_visible: bool) {
    let settings = settings_if_started(app);
    apply_dock(app, dock_policy_for(settings.as_ref(), panel_visible));
}

/// Whether the panel is on the screen now; a window that cannot say counts as hidden. Only the
/// main thread may ask without waiting for it.
fn panel_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    window.is_visible().unwrap_or_else(|e| {
        tracing::warn!(error = %e, "asking whether the panel is shown failed");
        false
    })
}

/// Brings the Dock icon in line with the settings and the panel as they are now, for when the
/// settings have just been read (the host started and the panel stays hidden) or saved: the icon
/// changes without the panel being shown or hidden. Callable from any thread: whether the panel is
/// visible is asked on the main thread and not waited for (`tray::on_main_thread`).
pub fn refresh_dock<R: Runtime>(app: &AppHandle<R>) {
    tray::on_main_thread(app, |app| apply_for_panel(app, panel_visible(app)));
}

/// Shows the panel, restored and focused; on macOS with its Dock icon, which no setting takes away
/// while the panel is up.
pub fn show<R: Runtime>(app: &AppHandle<R>) {
    apply_for_panel(app, true);
    match app.get_webview_window("main") {
        Some(window) => {
            logging::warn_failed(window.show(), "showing the panel");
            logging::warn_failed(window.unminimize(), "restoring the panel");
            logging::warn_failed(window.set_focus(), "focusing the panel");
        }
        None => tracing::warn!("the panel window is missing"),
    }
}

/// Hides the panel; on macOS its Dock icon goes with it, unless "Show in Dock" keeps it.
pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = app.get_webview_window("main") {
        logging::warn_failed(window.hide(), "hiding the panel");
    }
    apply_for_panel(app, false);
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

    #[test]
    fn the_icon_shows_while_the_panel_is_up_and_otherwise_as_the_setting_says() {
        // (show in Dock, panel visible) -> what the Dock shows: every row of the decision.
        for (show_in_dock, panel_visible, expected) in [
            (true, true, DockPolicy::Regular),
            (true, false, DockPolicy::Regular),
            (false, true, DockPolicy::Regular),
            (false, false, DockPolicy::Accessory),
        ] {
            assert_eq!(
                dock_policy(show_in_dock, panel_visible),
                expected,
                "show in Dock {show_in_dock}, panel visible {panel_visible}"
            );
        }
    }

    #[test]
    fn until_the_settings_are_read_the_icon_follows_the_panel_as_it_always_did() {
        assert_eq!(dock_policy_for(None, true), DockPolicy::Regular);
        assert_eq!(dock_policy_for(None, false), DockPolicy::Accessory);
    }

    #[test]
    fn a_new_install_keeps_the_icon_while_the_panel_is_hidden_and_switching_it_off_ends_that() {
        let new_install = Settings::default();
        assert_eq!(
            dock_policy_for(Some(&new_install), false),
            DockPolicy::Regular
        );
        let off = Settings {
            show_in_dock: false,
            ..Settings::default()
        };
        assert_eq!(dock_policy_for(Some(&off), false), DockPolicy::Accessory);
        assert_eq!(
            dock_policy_for(Some(&off), true),
            DockPolicy::Regular,
            "off still shows the icon while the panel is up"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_two_policies_are_the_two_macos_activation_policies() {
        // The mock runtime ignores the policy, so nothing else would see them swapped.
        assert!(matches!(
            DockPolicy::Regular.activation_policy(),
            tauri::ActivationPolicy::Regular
        ));
        assert!(matches!(
            DockPolicy::Accessory.activation_policy(),
            tauri::ActivationPolicy::Accessory
        ));
    }
}
