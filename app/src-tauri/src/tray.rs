//! The tray icon and its menu (design 9.1): the monitoring status, pause or resume, check now,
//! open the panel, quit. macOS shows a monochrome template image, Windows a colour one. Which
//! state is a problem is the rule app/src/lib/problem-cases.json pins down; the panel's sidebar
//! follows the same cases.
use issuerelay_core::{
    engine::StatusSnapshot,
    host::{Host, LinearSetup},
};
use tauri::{
    AppHandle, Manager, Runtime,
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

/// Which images the tray shows. macOS gets template images (black plus transparency, which the
/// system tints for a light or a dark menu bar), Windows colour icons. Both sets are compiled in
/// on every platform, picked by a constant and not by `#[cfg]`: neither build can lack the other's
/// code, and the tests on either platform check both sets. The images are made from
/// app/icons-src by `pnpm --dir app icons --option <x>` (design 9.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IconSet {
    /// `icons/tray/macos/*`, 44 px.
    MacTemplate,
    /// `icons/tray/*`, 32 px.
    WindowsColour,
}

impl IconSet {
    /// The set this platform shows.
    const CURRENT: IconSet = if cfg!(target_os = "macos") {
        IconSet::MacTemplate
    } else {
        IconSet::WindowsColour
    };

    /// Whether the system must treat the images as templates. Only the macOS set is one: tinted
    /// as a template, a colour icon would lose its colours.
    fn is_template(self) -> bool {
        self == IconSet::MacTemplate
    }
}

/// The icon for `state` in `set`.
fn icon_in(set: IconSet, state: TrayState) -> Image<'static> {
    let bytes: &'static [u8] = match (set, state) {
        (IconSet::MacTemplate, TrayState::Idle) => include_bytes!("../icons/tray/macos/idle.png"),
        (IconSet::MacTemplate, TrayState::Running) => {
            include_bytes!("../icons/tray/macos/running.png")
        }
        (IconSet::MacTemplate, TrayState::Paused) => {
            include_bytes!("../icons/tray/macos/paused.png")
        }
        (IconSet::MacTemplate, TrayState::Problem) => {
            include_bytes!("../icons/tray/macos/problem.png")
        }
        (IconSet::WindowsColour, TrayState::Idle) => include_bytes!("../icons/tray/idle.png"),
        (IconSet::WindowsColour, TrayState::Running) => {
            include_bytes!("../icons/tray/running.png")
        }
        (IconSet::WindowsColour, TrayState::Paused) => include_bytes!("../icons/tray/paused.png"),
        (IconSet::WindowsColour, TrayState::Problem) => {
            include_bytes!("../icons/tray/problem.png")
        }
    };
    Image::from_bytes(bytes).expect("tray icons are valid PNG files")
}

/// The icon for `state` on this platform.
fn icon(state: TrayState) -> Image<'static> {
    icon_in(IconSet::CURRENT, state)
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
        .icon_as_template(IconSet::CURRENT.is_template())
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

/// A click in a menu: the tray's items, and the application menu's Quit on macOS. It comes on the
/// main thread, so pausing or resuming (which writes the settings file) is done on the async
/// runtime, like the panel's own pause and resume commands; the tray follows once it is done.
fn on_menu(app: &AppHandle, event: MenuEvent) {
    let host = app.state::<AppState>().try_host();
    match (event.id().as_ref(), host) {
        ("toggle", Some(host)) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let engine = host.engine();
                let switched = if engine.status().paused {
                    engine.resume()
                } else {
                    engine.pause()
                };
                if let Err(e) = switched {
                    tracing::warn!(error = %format!("{e:#}"), "could not switch monitoring");
                }
                refresh(&app, &host);
            });
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
    let status = t.t("tray.status_problem");
    on_main_thread(app, move |app| {
        match app.try_state::<TrayItems>() {
            Some(items) => {
                logging::warn_failed(items.status.set_text(&status), "updating the tray status")
            }
            None => tracing::warn!("the tray menu is missing"),
        }
        match app.tray_by_id(TRAY_ID) {
            Some(tray) => set_icon(&tray, TrayState::Problem),
            None => tracing::warn!("the tray icon is missing"),
        }
    });
}

/// Shows the icon of `state`. Setting an icon drops macOS's template flag (tray-icon 0.25.1 sets
/// it to false), so the flag is set again each time.
fn set_icon(tray: &TrayIcon, state: TrayState) {
    logging::warn_failed(tray.set_icon(Some(icon(state))), "updating the tray icon");
    logging::warn_failed(
        tray.set_icon_as_template(IconSet::CURRENT.is_template()),
        "updating the tray icon",
    );
}

/// The texts `refresh` gives the menu items besides those of the view.
struct Labels {
    poll: String,
    open: String,
    quit: String,
    /// The application menu's texts by key (`quit::APP_MENU_KEYS`); only macOS has that menu.
    #[cfg(target_os = "macos")]
    app_menu: Vec<(&'static str, String)>,
}

/// Brings the tray in line with the engine, in the language the settings ask for: the tray's
/// menu and icon, and on macOS the application menu. Works out what to show here and leaves the
/// showing to the main thread (`on_main_thread`).
pub fn refresh(app: &AppHandle, host: &Host) {
    let t = Translator::for_settings(&host.engine().settings());
    let view = tray_view(&host.engine().status(), &host.linear_setup(), &t);
    let labels = Labels {
        poll: t.t("tray.poll_now"),
        open: t.t("tray.open_panel"),
        quit: t.t("tray.quit"),
        #[cfg(target_os = "macos")]
        app_menu: quit::APP_MENU_KEYS
            .iter()
            .map(|key| (*key, quit::app_menu_text(&t, key, &app.package_info().name)))
            .collect(),
    };
    on_main_thread(app, move |app| show_view(app, &view, &labels));
}

/// Puts `view` and `labels` into the menus and the icon; on the main thread.
fn show_view(app: &AppHandle, view: &TrayView, labels: &Labels) {
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
            logging::warn_failed(items.poll.set_text(&labels.poll), "updating the tray menu");
            logging::warn_failed(items.poll.set_enabled(true), "updating the tray menu");
            logging::warn_failed(items.open.set_text(&labels.open), "updating the tray menu");
            logging::warn_failed(items.quit.set_text(&labels.quit), "updating the tray menu");
        }
        None => tracing::warn!("the tray menu is missing"),
    }
    // Only macOS has an application menu (`quit::app_menu`).
    #[cfg(target_os = "macos")]
    match app.try_state::<quit::AppMenu>() {
        Some(menu) => {
            for (key, item) in &menu.0 {
                if let Some((_, text)) = labels.app_menu.iter().find(|(wanted, _)| wanted == key) {
                    logging::warn_failed(item.set_text(text), "updating the application menu");
                }
            }
        }
        None => tracing::warn!("the application menu is missing"),
    }
    match app.tray_by_id(TRAY_ID) {
        Some(tray) => set_icon(&tray, view.state),
        None => tracing::warn!("the tray icon is missing"),
    }
}

/// Runs `apply` on the main thread, which menus and tray icons need, without waiting for it.
/// Their setters called from another thread would wait for the main thread to run them, and an
/// exit the system forces keeps the main thread waiting for the host's stop (`quit::forced_exit`),
/// which runs on the async runtime's threads: the event forwarder or a panel command waiting there
/// could hold up that stop. From the main thread itself, `apply` runs at once. A window's getters
/// are no different (`window::refresh_dock` asks whether the panel is visible).
pub(crate) fn on_main_thread<R: Runtime>(
    app: &AppHandle<R>,
    apply: impl FnOnce(&AppHandle<R>) + Send + 'static,
) {
    let handle = app.clone();
    logging::warn_failed(
        app.run_on_main_thread(move || apply(&handle)),
        "handing work to the main thread",
    );
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    use issuerelay_core::host::AuthMethod;
    use serde::Deserialize;
    use tauri::{RunEvent, WebviewUrl, WebviewWindowBuilder};

    use super::*;
    use crate::test_support::{checked, run_mock_app};

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

    const STATES: [TrayState; 4] = [
        TrayState::Idle,
        TrayState::Running,
        TrayState::Paused,
        TrayState::Problem,
    ];
    /// Both sets with the size their images have (design 9.6); the tests on either platform check both.
    const SETS: [(IconSet, u32); 2] = [(IconSet::MacTemplate, 44), (IconSet::WindowsColour, 32)];

    #[test]
    fn every_state_has_an_icon_on_each_platform() {
        // `icon_in` decodes the embedded file and panics on a broken one, so a bad file fails
        // here and not when the tray first changes. The macOS images are templates, which the
        // system tints: black plus transparency, no colour and no grey. The Windows ones are
        // colour, so the two sets cannot be swapped by mistake.
        for (set, size) in SETS {
            for state in STATES {
                let image = icon_in(set, state);
                assert_eq!(
                    (image.width(), image.height()),
                    (size, size),
                    "{set:?} {state:?}"
                );
                let (pixels, _) = image.rgba().as_chunks::<4>();
                assert!(
                    pixels.iter().any(|&[_, _, _, a]| a > 0),
                    "{set:?} {state:?} is empty"
                );
                if set.is_template() {
                    assert!(
                        pixels.iter().all(|&[r, g, b, _]| (r, g, b) == (0, 0, 0)),
                        "{state:?} is not black plus transparency"
                    );
                } else {
                    assert!(
                        pixels
                            .iter()
                            .any(|&[r, g, b, a]| a > 0 && !(r == g && g == b)),
                        "{state:?} has no colour"
                    );
                }
            }
        }
    }

    #[test]
    fn every_tray_state_has_a_distinct_valid_icon() {
        for (set, _) in SETS {
            let icons: Vec<Vec<u8>> = STATES
                .into_iter()
                .map(|state| icon_in(set, state).rgba().to_vec())
                .collect();
            // `assert!` and not `assert_ne!`, which would print both images' pixels on a failure.
            for (i, a) in icons.iter().enumerate() {
                for (j, b) in icons.iter().enumerate().skip(i + 1) {
                    assert!(
                        a != b,
                        "{:?} and {:?} share a tray icon in {set:?}",
                        STATES[i],
                        STATES[j]
                    );
                }
            }
        }
    }

    #[test]
    fn only_the_macos_icons_are_templates_and_each_platform_shows_its_own_set() {
        assert!(IconSet::MacTemplate.is_template());
        assert!(!IconSet::WindowsColour.is_template());
        let (own, other, size) = if cfg!(target_os = "macos") {
            (IconSet::MacTemplate, IconSet::WindowsColour, 44)
        } else {
            (IconSet::WindowsColour, IconSet::MacTemplate, 32)
        };
        assert_eq!(IconSet::CURRENT, own);
        // What the tray is given is this platform's image, at its size, and never the other's.
        for state in STATES {
            let shown = icon(state);
            assert_eq!((shown.width(), shown.height()), (size, size), "{state:?}");
            assert!(
                shown.rgba() != icon_in(other, state).rgba(),
                "{state:?} shows the other platform's image"
            );
        }
    }

    #[test]
    fn a_tray_update_from_another_thread_never_waits_for_the_main_thread() {
        // While the main thread is busy, as during an exit the system forces (it waits there for
        // the host's stop, which runs on the async runtime's threads), an update sent from such a
        // thread must return at once, or the stop could not go on. It runs on the main thread
        // once that is free again. On tauri's mock runtime, whose loop thread is the main thread.
        let ran_on = Arc::new(Mutex::new(None));
        let main = Arc::new(Mutex::new(None));
        let (record, main_id) = (ran_on.clone(), main.clone());
        run_mock_app(
            Duration::from_secs(30),
            |_| {},
            move |app, event| {
                if let RunEvent::Ready = event {
                    *main_id.lock().unwrap() = Some(std::thread::current().id());
                    let (returned, wait) = mpsc::channel();
                    let (handle, ran) = (app.clone(), record.clone());
                    std::thread::spawn(move || {
                        on_main_thread(&handle, move |_| {
                            *ran.lock().unwrap() = Some(std::thread::current().id());
                        });
                        returned.send(()).unwrap();
                    });
                    // The main thread stays busy here until the update has returned.
                    wait.recv_timeout(Duration::from_secs(10))
                        .expect("the update waited for the main thread");
                    assert!(
                        record.lock().unwrap().is_none(),
                        "it ran while the main thread was busy"
                    );
                    // The last window goes, which ends the loop once the update has run.
                    WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                        .build()
                        .unwrap()
                        .destroy()
                        .unwrap();
                }
            },
        );
        let main = *main.lock().unwrap();
        assert!(main.is_some());
        assert_eq!(*ran_on.lock().unwrap(), main, "it ran on the main thread");
    }
}
