//! The tray icon and its menu (design 9.1): the monitoring status, pause or resume, check now,
//! open the panel, quit. macOS shows a monochrome template image, Windows a colour one. Which
//! state is a problem is the rule app/src/lib/problem-cases.json pins down; the panel's sidebar
//! follows the same cases.
use issuerelay_core::{
    engine::StatusSnapshot,
    host::{Host, LinearSetup},
};
use tauri::{
    AppHandle, Manager,
    image::Image,
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent},
};

use crate::{i18n::Translator, logging, quit, state::AppState, window};

pub const TRAY_ID: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayState {
    Idle,
    Running,
    Paused,
    Problem,
}

/// What the tray shows: the icon's state, the status line and the label of pause or resume.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayView {
    pub state: TrayState,
    pub status_line: String,
    pub toggle_label: String,
}

/// What the tray shows for the engine's status. A problem (not connected to Linear, the last
/// check failed, a job could not start) wins over paused, which wins over running. While the
/// first connection is under way (`Connecting`, the host no longer waits for it) nothing is a
/// problem yet: the checks fail until it is done.
pub fn tray_view(status: &StatusSnapshot, setup: &LinearSetup, t: &Translator) -> TrayView {
    let problem = match setup {
        LinearSetup::Connected { .. } => {
            (status.last_poll_at.is_some() && !status.linear_ok) || status.last_error.is_some()
        }
        LinearSetup::Connecting => false,
        _ => true,
    };
    let state = if problem {
        TrayState::Problem
    } else if status.paused {
        TrayState::Paused
    } else if status.running > 0 {
        TrayState::Running
    } else {
        TrayState::Idle
    };
    let mut status_line = if status.paused {
        t.t("tray.status_paused")
    } else {
        t.t_with(
            "tray.status_counts",
            &[
                ("running", &status.running.to_string()),
                ("queued", &status.queued.to_string()),
            ],
        )
    };
    if problem {
        status_line = format!("{status_line} · {}", t.t("tray.status_problem"));
    }
    let toggle_label = if status.paused {
        t.t("tray.resume")
    } else {
        t.t("tray.pause")
    };
    TrayView {
        state,
        status_line,
        toggle_label,
    }
}

/// The icon for `state`. Placeholders until Task 11 draws the real ones.
fn icon(state: TrayState) -> Image<'static> {
    let bytes: &'static [u8] = match state {
        TrayState::Idle => include_bytes!("../icons/tray/idle.png"),
        TrayState::Running => include_bytes!("../icons/tray/running.png"),
        TrayState::Paused => include_bytes!("../icons/tray/paused.png"),
        TrayState::Problem => include_bytes!("../icons/tray/problem.png"),
    };
    Image::from_bytes(bytes).expect("tray icons are valid PNG files")
}

/// The menu items `refresh` updates.
struct TrayItems {
    status: MenuItem<tauri::Wry>,
    toggle: MenuItem<tauri::Wry>,
    poll: MenuItem<tauri::Wry>,
    open: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
}

/// Puts the icon in the tray, with its menu in the system language until the host has started.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let t = Translator::new(crate::i18n::resolve_language(
        "system",
        sys_locale::get_locale().as_deref(),
    ));
    let status = MenuItem::with_id(app, "status", t.t("tray.starting"), false, None::<&str>)?;
    let toggle = MenuItem::with_id(app, "toggle", t.t("tray.pause"), false, None::<&str>)?;
    let poll = MenuItem::with_id(app, "poll", t.t("tray.poll_now"), false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", t.t("tray.open_panel"), true, None::<&str>)?;
    let quit = MenuItem::with_id(
        app,
        quit::QUIT_MENU_ID,
        t.t("tray.quit"),
        true,
        None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &PredefinedMenuItem::separator(app)?,
            &toggle,
            &poll,
            &open,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    app.manage(TrayItems {
        status,
        toggle,
        poll,
        open,
        quit,
    });
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(TrayState::Idle))
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("IssueRelay")
        .menu(&menu)
        // A menu bar item on macOS opens its menu on a left click; on Windows the left button
        // opens the panel and the right one the menu.
        .show_menu_on_left_click(cfg!(target_os = "macos"))
        // Gets the events of every menu, the application menu's Quit (macOS) included.
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if !cfg!(target_os = "macos")
                && let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
            {
                window::show(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// A click in a menu: the tray's items, and the application menu's Quit on macOS.
fn on_menu(app: &AppHandle, event: MenuEvent) {
    let host = app.state::<AppState>().try_host();
    match (event.id().as_ref(), host) {
        ("toggle", Some(host)) => {
            let engine = host.engine();
            let switched = if engine.status().paused {
                engine.resume()
            } else {
                engine.pause()
            };
            if let Err(e) = switched {
                tracing::warn!(error = %format!("{e:#}"), "could not switch monitoring");
            }
            refresh(app, &host);
        }
        ("poll", Some(host)) => host.engine().poll_now(),
        ("open", _) => window::show(app),
        (quit::QUIT_MENU_ID, _) => quit::request(app),
        _ => {}
    }
}

/// After a failed start: the problem icon and "needs attention" in the system language instead
/// of "starting…" for ever. The panel, shown at the same time, says why; pause and check now stay
/// off, there is no engine.
pub fn show_failed_start(app: &AppHandle) {
    let t = Translator::new(crate::i18n::resolve_language(
        "system",
        sys_locale::get_locale().as_deref(),
    ));
    match app.try_state::<TrayItems>() {
        Some(items) => logging::warn_failed(
            items.status.set_text(t.t("tray.status_problem")),
            "updating the tray status",
        ),
        None => tracing::warn!("the tray menu is missing"),
    }
    match app.tray_by_id(TRAY_ID) {
        Some(tray) => set_icon(&tray, TrayState::Problem),
        None => tracing::warn!("the tray icon is missing"),
    }
}

/// Shows the icon of `state`. Setting an icon drops macOS's template flag (tray-icon 0.25.1 sets
/// it to false), so the flag is set again each time.
fn set_icon(tray: &TrayIcon, state: TrayState) {
    logging::warn_failed(tray.set_icon(Some(icon(state))), "updating the tray icon");
    logging::warn_failed(
        tray.set_icon_as_template(cfg!(target_os = "macos")),
        "updating the tray icon",
    );
}

/// Brings the tray in line with the engine, in the language the settings ask for.
pub fn refresh(app: &AppHandle, host: &Host) {
    let t = Translator::for_settings(&host.engine().settings());
    let view = tray_view(&host.engine().status(), &host.linear_setup(), &t);
    match app.try_state::<TrayItems>() {
        Some(items) => {
            logging::warn_failed(
                items.status.set_text(&view.status_line),
                "updating the tray status",
            );
            logging::warn_failed(
                items.toggle.set_text(&view.toggle_label),
                "updating the tray menu",
            );
            logging::warn_failed(items.toggle.set_enabled(true), "updating the tray menu");
            logging::warn_failed(
                items.poll.set_text(t.t("tray.poll_now")),
                "updating the tray menu",
            );
            logging::warn_failed(items.poll.set_enabled(true), "updating the tray menu");
            logging::warn_failed(
                items.open.set_text(t.t("tray.open_panel")),
                "updating the tray menu",
            );
            logging::warn_failed(
                items.quit.set_text(t.t("tray.quit")),
                "updating the tray menu",
            );
        }
        None => tracing::warn!("the tray menu is missing"),
    }
    match app.tray_by_id(TRAY_ID) {
        Some(tray) => set_icon(&tray, view.state),
        None => tracing::warn!("the tray icon is missing"),
    }
}

#[cfg(test)]
mod tests {
    use issuerelay_core::host::AuthMethod;
    use serde::Deserialize;

    use super::*;
    use crate::test_support::checked;

    fn connected() -> LinearSetup {
        LinearSetup::Connected {
            method: AuthMethod::OAuth,
        }
    }

    #[test]
    fn the_tray_shows_what_the_engine_is_doing() {
        let t = Translator::new("en");
        let idle = tray_view(&checked(), &connected(), &t);
        assert_eq!(idle.state, TrayState::Idle);
        assert_eq!(idle.status_line, "Running 0 · Queued 0");
        assert_eq!(idle.toggle_label, "Pause monitoring");

        let busy = StatusSnapshot {
            running: 2,
            queued: 1,
            ..checked()
        };
        assert_eq!(
            tray_view(&busy, &connected(), &t).status_line,
            "Running 2 · Queued 1"
        );

        let paused = StatusSnapshot {
            paused: true,
            running: 1,
            ..checked()
        };
        let view = tray_view(&paused, &connected(), &t);
        assert_eq!(view.status_line, "Monitoring paused");
        assert_eq!(view.toggle_label, "Resume monitoring");

        let not_logged_in = tray_view(&checked(), &LinearSetup::NotConfigured, &t);
        assert_eq!(
            not_logged_in.status_line,
            "Running 0 · Queued 0 · Needs attention"
        );
    }

    /// One row of app/src/lib/problem-cases.json, which the panel's sidebar test reads too.
    #[derive(Deserialize)]
    struct ProblemCase {
        name: String,
        linear: String,
        checked: bool,
        linear_ok: bool,
        last_error: bool,
        paused: bool,
        running: usize,
        expected: String,
    }

    #[test]
    fn the_tray_state_follows_the_cases_the_panel_shares() {
        let cases: Vec<ProblemCase> =
            serde_json::from_str(include_str!("../../src/lib/problem-cases.json"))
                .expect("problem-cases.json is valid");
        assert!(cases.len() >= 14, "the shared cases were read");
        let t = Translator::new("en");
        for case in cases {
            let setup = match case.linear.as_str() {
                "connected" => connected(),
                "not_configured" => LinearSetup::NotConfigured,
                "interface_changed" => LinearSetup::InterfaceChanged {
                    message: "list_issues changed".into(),
                },
                "connect_failed" => LinearSetup::ConnectFailed {
                    message: "network is down".into(),
                },
                "connecting" => LinearSetup::Connecting,
                other => panic!("unknown Linear state {other} in {:?}", case.name),
            };
            let status = StatusSnapshot {
                paused: case.paused,
                linear_ok: case.linear_ok,
                last_poll_at: case.checked.then(chrono::Utc::now),
                last_error: case
                    .last_error
                    .then(|| "could not start DEMO-1".to_string()),
                running: case.running,
                ..checked()
            };
            let state = match tray_view(&status, &setup, &t).state {
                TrayState::Idle => "idle",
                TrayState::Running => "running",
                TrayState::Paused => "paused",
                TrayState::Problem => "problem",
            };
            assert_eq!(state, case.expected, "{}", case.name);
        }
    }

    #[test]
    fn every_tray_icon_is_a_png_the_tray_can_show() {
        // `icon` decodes the embedded file and panics on a broken one; the icons are replaced
        // by hand (Task 11), so a bad file must fail here and not when the tray first changes.
        for state in [
            TrayState::Idle,
            TrayState::Running,
            TrayState::Paused,
            TrayState::Problem,
        ] {
            let image = icon(state);
            assert!(image.width() > 0 && image.height() > 0, "{state:?}");
        }
    }
}
