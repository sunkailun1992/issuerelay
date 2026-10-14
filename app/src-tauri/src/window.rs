//! The panel window: shown on demand, hidden instead of closed. On macOS the Dock icon goes with
//! it: by default the icon stays for as long as the app runs, since a tray item alone is easily
//! lost on a crowded menu bar and then nobody can find the app (design 4.1); with "Show in Dock"
//! off (`Settings::show_in_dock`) the icon follows the window as it used to, shown only while the
//! panel is, also after a login start (lib.rs hides through `hide` too). `dock_policy` is that
//! one decision; `DockSetting` is how much of the setting is known when it is made. Windows needs
//! nothing of the kind: it has no Dock, and a window's taskbar button comes and goes with the
//! window itself (a hidden window has none). Opening the app again brings the panel back: macOS
//! reports it to the running process (`on_reopen`; clicking the Dock icon does the same), Windows
//! starts a second one (the single-instance callback in lib.rs).
use issuerelay_core::config::Settings;
use tauri::{AppHandle, Manager, Runtime, WindowEvent};

use crate::{
    logging,
    quit::{self, QuitEvent, QuitStep},
    state::{AppState, StartupState},
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

/// What is known of "Show in Dock" when the icon is decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DockSetting {
    /// The host is still starting, so the settings are not read yet. The icon follows the panel, as
    /// it always did: someone who switched the icon off must not see it flash at a login start.
    NotRead,
    /// The host could not start, so the settings never will be read. The panel shows why, and it
    /// is all there is of the app: the icon stays, as it does by default, so that closing that page
    /// does not take away the one place the user then looks for the app.
    HostFailed,
    /// The setting as the settings say.
    Read(bool),
}

impl DockSetting {
    /// Whether the icon stays after the panel is hidden.
    fn show_in_dock(self) -> bool {
        match self {
            Self::NotRead => false,
            Self::HostFailed => Settings::default().show_in_dock,
            Self::Read(show_in_dock) => show_in_dock,
        }
    }
}

/// Whether the Dock shows the app. A visible panel always has the icon; a hidden one has it only
/// with "Show in Dock" on, which is how the app behaved before the setting existed. The setting is
/// asked only when the answer depends on it: asking copies the whole configuration out of the
/// engine, which showing the panel does not need.
pub fn dock_policy(panel_shown: bool, show_in_dock: impl FnOnce() -> bool) -> DockPolicy {
    if panel_shown || show_in_dock() {
        DockPolicy::Regular
    } else {
        DockPolicy::Accessory
    }
}

/// What is known of the setting now, from how the start of the host went (`AppState`).
fn dock_setting<R: Runtime>(app: &AppHandle<R>) -> DockSetting {
    let Some(state) = app.try_state::<AppState>() else {
        return DockSetting::NotRead;
    };
    match state.startup_state() {
        StartupState::Starting => DockSetting::NotRead,
        StartupState::Failed(_) => DockSetting::HostFailed,
        StartupState::Ready => state.try_host().map_or(DockSetting::NotRead, |host| {
            DockSetting::Read(host.engine().settings().show_in_dock)
        }),
    }
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

/// Puts the Dock icon where `dock_policy` says, for the panel shown or hidden and the setting as
/// far as it is known. Every change of the icon goes through `apply_dock`.
fn apply_for_panel<R: Runtime>(app: &AppHandle<R>, panel_shown: bool) {
    apply_dock(
        app,
        dock_policy(panel_shown, || dock_setting(app).show_in_dock()),
    );
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
    use tauri::test::{mock_builder, mock_context, noop_assets};

    use super::*;
    use crate::state::{StartupCode, StartupFailure};

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
        // (panel shown, show in Dock) -> what the Dock shows: every row of the decision.
        for (panel_shown, show_in_dock, expected) in [
            (true, true, DockPolicy::Regular),
            (true, false, DockPolicy::Regular),
            (false, true, DockPolicy::Regular),
            (false, false, DockPolicy::Accessory),
        ] {
            assert_eq!(
                dock_policy(panel_shown, || show_in_dock),
                expected,
                "panel shown {panel_shown}, show in Dock {show_in_dock}"
            );
        }
    }

    #[test]
    fn showing_the_panel_does_not_ask_for_the_setting() {
        // Asking copies the whole configuration out of the engine, and a visible panel has the icon
        // whatever it says.
        assert_eq!(
            dock_policy(true, || panic!("the setting was asked for")),
            DockPolicy::Regular
        );
    }

    #[test]
    fn what_is_known_of_the_setting_decides_the_icon_of_a_hidden_panel() {
        // (what is known of the setting, panel shown) -> what the Dock shows.
        for (setting, panel_shown, expected) in [
            // Until the settings are read the icon follows the panel, as it always did.
            (DockSetting::NotRead, true, DockPolicy::Regular),
            (DockSetting::NotRead, false, DockPolicy::Accessory),
            // The host could not start, so they never will be: the failure page is all there is of
            // the app, and closing it keeps the icon, as the default does.
            (DockSetting::HostFailed, true, DockPolicy::Regular),
            (DockSetting::HostFailed, false, DockPolicy::Regular),
            // A new install keeps the icon while the panel is hidden; switching it off ends that,
            // except while the panel is up.
            (
                DockSetting::Read(Settings::default().show_in_dock),
                false,
                DockPolicy::Regular,
            ),
            (DockSetting::Read(false), true, DockPolicy::Regular),
            (DockSetting::Read(false), false, DockPolicy::Accessory),
        ] {
            assert_eq!(
                dock_policy(panel_shown, || setting.show_in_dock()),
                expected,
                "{setting:?}, panel shown {panel_shown}"
            );
        }
    }

    #[test]
    fn what_is_known_of_the_setting_follows_how_the_start_of_the_host_went() {
        // `Read` needs a host that has started, which no unit test builds: the acceptance on the
        // real app looks at it.
        let bare = mock_builder().build(mock_context(noop_assets())).unwrap();
        assert_eq!(dock_setting(bare.handle()), DockSetting::NotRead);
        let app = mock_builder()
            .manage(AppState::new(false))
            .build(mock_context(noop_assets()))
            .unwrap();
        assert_eq!(dock_setting(app.handle()), DockSetting::NotRead, "starting");
        app.state::<AppState>().set_host(Err(StartupFailure {
            code: StartupCode::AlreadyRunning,
            detail: "another IssueRelay instance is already running".into(),
        }));
        assert_eq!(dock_setting(app.handle()), DockSetting::HostFailed);
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
