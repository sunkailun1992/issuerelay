//! Quitting (design 4.1, 6.2). Every way to quit comes here: the tray's Quit, Cmd+Q, the system
//! asking the app to exit, the panel's answer and the window being closed. `next` is the whole
//! decision, a pure function of where quitting stands and what happened; `handle` applies it.
//! With runs in progress the panel asks whether to wait for them or stop them, and a user who
//! chose to wait can still stop them: `Host::stop(StopRuns)` ends a `stop(WaitForRuns)` that is
//! still waiting, and both return.
//!
//! The process ends only once quitting is `Done`, and only a returned `Host::stop` leads there
//! (or there being no host to stop): by then the runs have ended with their process trees and
//! their results are recorded. That order matters because `AppHandle::exit` ends the process
//! with `std::process::exit`, which runs no destructors: a run's guard would never end its tree.
//!
//! What reaches the app when the system quits it (tauri 2.12.3, tauri-runtime-wry 2.12.1 and tao
//! 0.37.1, read on 2026-10-13): `RunEvent::ExitRequested` comes only for `AppHandle::exit`
//! (`code` is `Some`) and when the last window is gone (`None`). macOS's own Quit
//! (`terminate:`: the default application menu's Cmd+Q, the Dock's Quit, logging out, shutting
//! down) and Windows ending the session arrive only as `RunEvent::Exit`, which cannot be refused.
//! So Cmd+Q is an item of IssueRelay's own (`app_menu`) that asks like the tray's Quit, and an
//! exit that cannot be refused still stops the host first, for a bounded while.
use std::{
    sync::{Mutex, PoisonError},
    time::Duration,
};

use issuerelay_core::host::StopMode;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime};

use crate::{events, i18n::Translator, logging, state::AppState, window};

/// The id of the Quit items: the tray menu's and, on macOS, the application menu's. The tray's
/// menu handler gets the events of every menu (`TrayIconBuilder::on_menu_event`), so both go to
/// `request`.
pub const QUIT_MENU_ID: &str = "quit";

/// How long an exit that cannot be refused waits for the host to stop. Stopping the runs takes a
/// few seconds (an executor gets five to end what it started) and recording them a moment more;
/// the system does not wait for ever for a process that is ending.
const FORCED_EXIT_STOP: Duration = Duration::from_secs(10);

/// Where quitting stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitState {
    /// Not quitting.
    Idle,
    /// The panel asks "wait for completion" or "stop now".
    Asking,
    /// Waiting for the runs in progress; nothing new starts.
    Waiting,
    /// Stopping the runs in progress.
    Stopping,
    /// The host has stopped, or never started: the app exits.
    Done,
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitEvent {
    /// The tray's Quit, Cmd+Q or the system asked to quit; the host as it was at that moment.
    Requested { host_ready: bool, running: usize },
    /// The panel's answer: wait for the runs, then quit.
    Wait,
    /// The panel's answer: stop the runs now, also while already waiting.
    StopNow,
    /// A `Host::stop` returned.
    Stopped,
    /// The panel window is being closed.
    WindowClosed,
}

/// What `handle` does after a transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitStep {
    Nothing,
    /// Show the panel and ask (`quit-requested` with the runs in progress, not waiting yet).
    Ask(usize),
    /// Show the panel again and tell it that quitting waits for the runs in progress
    /// (`quit-requested` with `waiting`): a panel that was reloaded meanwhile has lost the waiting
    /// prompt, and shows it again with "stop now".
    ShowPanel(usize),
    /// Stop the host as the mode says, in the background; `Stopped` follows.
    Stop(StopMode),
    /// Hide the window instead of letting it close.
    HideWindow,
    /// Exit the process.
    Exit,
}

/// The panel's question (`quit-requested`): how many runs are in progress, and whether quitting
/// already waits for them, in which case the panel offers only "stop now".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct QuitPrompt {
    pub running: usize,
    pub waiting: bool,
}

/// The next state and what to do, for `event` in `state`.
pub fn next(state: QuitState, event: QuitEvent) -> (QuitState, QuitStep) {
    use QuitEvent as E;
    use QuitState as S;
    match (state, event) {
        (S::Done, E::WindowClosed) => (S::Done, QuitStep::Nothing),
        (_, E::WindowClosed) => (state, QuitStep::HideWindow),
        (
            S::Idle | S::Asking,
            E::Requested {
                host_ready: false, ..
            },
        ) => (S::Done, QuitStep::Exit),
        (S::Idle | S::Asking, E::Requested { running: 0, .. }) => {
            (S::Stopping, QuitStep::Stop(StopMode::StopRuns))
        }
        (S::Idle | S::Asking, E::Requested { running, .. }) => (S::Asking, QuitStep::Ask(running)),
        (S::Waiting, E::Requested { running, .. }) => (S::Waiting, QuitStep::ShowPanel(running)),
        (S::Idle | S::Asking, E::Wait) => (S::Waiting, QuitStep::Stop(StopMode::WaitForRuns)),
        (S::Idle | S::Asking | S::Waiting, E::StopNow) => {
            (S::Stopping, QuitStep::Stop(StopMode::StopRuns))
        }
        (S::Waiting | S::Stopping, E::Stopped) => (S::Done, QuitStep::Exit),
        _ => (state, QuitStep::Nothing),
    }
}

/// Whether the process may end now: only once quitting is done, that is after a `Host::stop`
/// returned or when there was no host to stop. The event loop hook (`on_run_event`) decides by it.
pub fn may_exit(state: QuitState) -> bool {
    state == QuitState::Done
}

/// Applies `event`: moves quitting to its next state and does what the step says. Returns the
/// step, which the window hook reads (`HideWindow`).
pub fn handle<R: Runtime>(app: &AppHandle<R>, event: QuitEvent) -> QuitStep {
    let state = app.state::<AppState>();
    let step = {
        let mut quit = state.quit.lock().unwrap_or_else(PoisonError::into_inner);
        let (next_state, step) = next(*quit, event);
        *quit = next_state;
        step
    };
    match step {
        QuitStep::Nothing | QuitStep::HideWindow => {}
        QuitStep::Ask(running) => ask_panel(
            app,
            QuitPrompt {
                running,
                waiting: false,
            },
        ),
        QuitStep::ShowPanel(running) => ask_panel(
            app,
            QuitPrompt {
                running,
                waiting: true,
            },
        ),
        QuitStep::Stop(mode) => match state.try_host() {
            Some(host) => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    host.stop(mode).await;
                    // Only now: the runs have ended with their trees, their results are recorded
                    // and the single-instance lock is released. `Stopped` leads to `Done`, and
                    // only that transition asks for `Exit`.
                    handle(&app, QuitEvent::Stopped);
                });
            }
            None => {
                no_host_to_stop(app, "asked to stop the host");
                handle(app, QuitEvent::Stopped);
            }
        },
        QuitStep::Exit => app.exit(0),
    }
    step
}

/// Shows the panel and puts `prompt` to it (`quit-requested`).
fn ask_panel<R: Runtime>(app: &AppHandle<R>, prompt: QuitPrompt) {
    window::show(app);
    logging::debug_failed(
        app.emit(events::QUIT_REQUESTED, prompt),
        "asking the panel about quitting",
    );
}

/// The tray's Quit, Cmd+Q and an exit request, with the host as it is now.
pub fn request<R: Runtime>(app: &AppHandle<R>) {
    let host = app.state::<AppState>().try_host();
    let running = match &host {
        Some(host) => host.engine().status().running,
        None => {
            no_host_to_stop(app, "quitting");
            0
        }
    };
    handle(
        app,
        QuitEvent::Requested {
            host_ready: host.is_some(),
            running,
        },
    );
}

/// Says that quitting found no host to stop: it may still be starting (`Host::start` has not
/// returned, so what it began is left to the next start's sweep) or it failed to start.
fn no_host_to_stop<R: Runtime>(app: &AppHandle<R>, what: &str) {
    let startup = app.state::<AppState>().startup_state();
    tracing::warn!(?startup, "{what}, but there is no host to stop");
}

/// Where quitting stands now.
fn current<R: Runtime>(app: &AppHandle<R>) -> QuitState {
    *app.state::<AppState>()
        .quit
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The app's event loop hook. A request to exit (`ExitRequested`) goes ahead only once quitting is
/// done (`may_exit`), which is when IssueRelay's own `exit(0)` comes; any other is held and goes
/// through `request`, which asks while runs are in progress. An exit that cannot be held
/// (`RunEvent::Exit` before quitting is done: the system is ending the app) stops the host first
/// (`forced_exit`), so that the runs end with their trees and are recorded. Not about quitting, but
/// the only other run event the app acts on: on macOS the system opening the app again (`Reopen`)
/// brings the panel back (`window::on_reopen`).
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    match event {
        RunEvent::ExitRequested { api, code, .. } => {
            if may_exit(current(app)) {
                return;
            }
            tracing::info!(?code, "asked to exit; quitting decides");
            api.prevent_exit();
            request(app);
        }
        RunEvent::Exit => {
            let state = app.state::<AppState>();
            let stop = state
                .try_host()
                .map(|host| async move { host.stop(StopMode::StopRuns).await });
            match forced_exit(&state.quit, stop, FORCED_EXIT_STOP) {
                ForcedExit::Done => {}
                ForcedExit::Stopped => tracing::info!("the host has stopped; IssueRelay ends"),
                ForcedExit::NoHost => no_host_to_stop(app, "the system is ending IssueRelay"),
                ForcedExit::TimedOut => tracing::error!(
                    seconds = FORCED_EXIT_STOP.as_secs(),
                    "the host did not stop in time; IssueRelay ends anyway"
                ),
            }
        }
        // macOS only. Opening the app again while it runs (Finder, Spotlight, `open`, a click on
        // the Dock icon) starts no second process: Launch Services sends the running one this
        // event instead, so the single-instance callback (lib.rs) never runs. Windows has no such
        // event: a second launch there is a second process, which that callback handles. (tauri
        // 2.12.3 and tao 0.37.1, read on 2026-10-14: the variant is `#[non_exhaustive]`, hence
        // the `..`; tao raises it from the app delegate's `applicationShouldHandleReopen`.)
        #[cfg(target_os = "macos")]
        RunEvent::Reopen {
            has_visible_windows,
            ..
        } => window::on_reopen(app, has_visible_windows),
        _ => {}
    }
}

/// What an exit that cannot be refused found, and how its stop went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForcedExit {
    /// Quitting was done already: the host had stopped, or there never was one.
    Done,
    /// No host, still starting or failed to start: nothing to stop.
    NoHost,
    /// The host's stop returned in time.
    Stopped,
    /// The host's stop did not return within the limit.
    TimedOut,
}

/// An exit that cannot be refused. Once quitting is done, or without a host to stop (`stop` is
/// `None`), there is nothing to do. Otherwise quitting counts as stopping from here on, and
/// `stop`, the host's `stop(StopRuns)`, runs on the async runtime while this thread, the event
/// loop's, waits for it at most `limit`. Nothing that stop needs waits for the main thread in
/// turn: the tray's updates are only handed to it (`tray::refresh`).
fn forced_exit<F>(quit: &Mutex<QuitState>, stop: Option<F>, limit: Duration) -> ForcedExit
where
    F: Future<Output = ()> + Send + 'static,
{
    let mut state = quit.lock().unwrap_or_else(PoisonError::into_inner);
    if may_exit(*state) {
        return ForcedExit::Done;
    }
    let Some(stop) = stop else {
        return ForcedExit::NoHost;
    };
    *state = QuitState::Stopping;
    // Not held while waiting: a stop the panel asked for earlier ends in `handle`, which takes it.
    drop(state);
    tracing::warn!("the system is ending IssueRelay without asking; stopping the runs first");
    if wait_bounded(stop, limit) {
        ForcedExit::Stopped
    } else {
        ForcedExit::TimedOut
    }
}

/// Runs `task` on the async runtime and waits for it on this thread, at most `limit`; whether it
/// ended in time. For the event loop's thread, which is none of the runtime's own.
fn wait_bounded(task: impl Future<Output = ()> + Send + 'static, limit: Duration) -> bool {
    let (ended, wait) = std::sync::mpsc::channel();
    tauri::async_runtime::spawn(async move {
        task.await;
        logging::debug_failed(ended.send(()), "reporting the end of a bounded wait");
    });
    wait.recv_timeout(limit).is_ok()
}

/// The texts of macOS's application menu, by translation key, in the order of the menu; Quit is
/// the tray's `tray.quit`. They are in the system language until the host has started, then in the
/// language the settings ask for (`tray::refresh`), like the tray's items.
pub const APP_MENU_KEYS: [&str; 20] = [
    "menu.about",
    "menu.services",
    "menu.hide",
    "menu.hide_others",
    "tray.quit",
    "menu.file",
    "menu.close_window",
    "menu.edit",
    "menu.undo",
    "menu.redo",
    "menu.cut",
    "menu.copy",
    "menu.paste",
    "menu.select_all",
    "menu.view",
    "menu.fullscreen",
    "menu.window",
    "menu.minimize",
    "menu.zoom",
    "menu.help",
];

/// The text of the application menu's item `key` in `t`'s language, with the app's `name` for
/// `{{name}}` ("About IssueRelay").
pub fn app_menu_text(t: &Translator, key: &str, name: &str) -> String {
    t.t_with(key, &[("name", name)])
}

/// One item of the application menu (macOS) whose text follows the interface language.
#[cfg(target_os = "macos")]
pub enum AppMenuItem {
    Submenu(tauri::menu::Submenu<tauri::Wry>),
    Predefined(tauri::menu::PredefinedMenuItem<tauri::Wry>),
    Item(tauri::menu::MenuItem<tauri::Wry>),
}

#[cfg(target_os = "macos")]
impl AppMenuItem {
    pub fn set_text(&self, text: &str) -> tauri::Result<()> {
        match self {
            AppMenuItem::Submenu(item) => item.set_text(text),
            AppMenuItem::Predefined(item) => item.set_text(text),
            AppMenuItem::Item(item) => item.set_text(text),
        }
    }
}

/// The application menu's items (macOS) with the key of each one's text, kept so that
/// `tray::refresh` can give them the language the settings ask for, as it does the tray's items.
#[cfg(target_os = "macos")]
pub struct AppMenu(pub Vec<(&'static str, AppMenuItem)>);

/// macOS's application menu: Tauri's default one (`Menu::default` in tauri 2.12.3) with every text
/// from the panel's translation files (`APP_MENU_KEYS`) instead of AppKit's English, and with a
/// Quit of IssueRelay's own with Cmd+Q instead of the system's `terminate:`, so that quitting asks
/// while runs are in progress (see the module comment). The texts start in the system language
/// (`AppMenu`). Windows has no application menu: there the tray's Quit is the way to quit.
#[cfg(target_os = "macos")]
pub fn app_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{
        AboutMetadata, HELP_SUBMENU_ID, Menu, MenuItem, PredefinedMenuItem, Submenu,
        WINDOW_SUBMENU_ID,
    };

    use AppMenuItem::{Item, Predefined, Submenu as Sub};

    use crate::i18n::resolve_language;

    let package = app.package_info();
    let config = app.config();
    let metadata = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        copyright: config.bundle.copyright.clone(),
        authors: config
            .bundle
            .publisher
            .clone()
            .map(|publisher| vec![publisher]),
        ..Default::default()
    };
    let t = Translator::new(resolve_language(
        "system",
        sys_locale::get_locale().as_deref(),
    ));
    let text = |key: &str| app_menu_text(&t, key, &package.name);
    let separator = || PredefinedMenuItem::separator(app);

    let about = PredefinedMenuItem::about(app, Some(&text("menu.about")), Some(metadata))?;
    let services = PredefinedMenuItem::services(app, Some(&text("menu.services")))?;
    let hide = PredefinedMenuItem::hide(app, Some(&text("menu.hide")))?;
    let hide_others = PredefinedMenuItem::hide_others(app, Some(&text("menu.hide_others")))?;
    let quit = MenuItem::with_id(
        app,
        QUIT_MENU_ID,
        text("tray.quit"),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    let close_file = PredefinedMenuItem::close_window(app, Some(&text("menu.close_window")))?;
    let undo = PredefinedMenuItem::undo(app, Some(&text("menu.undo")))?;
    let redo = PredefinedMenuItem::redo(app, Some(&text("menu.redo")))?;
    let cut = PredefinedMenuItem::cut(app, Some(&text("menu.cut")))?;
    let copy = PredefinedMenuItem::copy(app, Some(&text("menu.copy")))?;
    let paste = PredefinedMenuItem::paste(app, Some(&text("menu.paste")))?;
    let select_all = PredefinedMenuItem::select_all(app, Some(&text("menu.select_all")))?;
    let fullscreen = PredefinedMenuItem::fullscreen(app, Some(&text("menu.fullscreen")))?;
    let minimize = PredefinedMenuItem::minimize(app, Some(&text("menu.minimize")))?;
    let zoom = PredefinedMenuItem::maximize(app, Some(&text("menu.zoom")))?;
    let close_window = PredefinedMenuItem::close_window(app, Some(&text("menu.close_window")))?;

    let app_submenu = Submenu::with_items(
        app,
        &package.name,
        true,
        &[
            &about,
            &separator()?,
            &services,
            &separator()?,
            &hide,
            &hide_others,
            &separator()?,
            &quit,
        ],
    )?;
    let file = Submenu::with_items(app, text("menu.file"), true, &[&close_file])?;
    let edit = Submenu::with_items(
        app,
        text("menu.edit"),
        true,
        &[
            &undo,
            &redo,
            &separator()?,
            &cut,
            &copy,
            &paste,
            &select_all,
        ],
    )?;
    let view = Submenu::with_items(app, text("menu.view"), true, &[&fullscreen])?;
    let window = Submenu::with_id_and_items(
        app,
        WINDOW_SUBMENU_ID,
        text("menu.window"),
        true,
        &[&minimize, &zoom, &separator()?, &close_window],
    )?;
    let help = Submenu::with_id_and_items(app, HELP_SUBMENU_ID, text("menu.help"), true, &[])?;
    let menu = Menu::with_items(app, &[&app_submenu, &file, &edit, &view, &window, &help])?;
    app.manage(AppMenu(vec![
        ("menu.about", Predefined(about)),
        ("menu.services", Predefined(services)),
        ("menu.hide", Predefined(hide)),
        ("menu.hide_others", Predefined(hide_others)),
        ("tray.quit", Item(quit)),
        ("menu.file", Sub(file)),
        ("menu.close_window", Predefined(close_file)),
        ("menu.edit", Sub(edit)),
        ("menu.undo", Predefined(undo)),
        ("menu.redo", Predefined(redo)),
        ("menu.cut", Predefined(cut)),
        ("menu.copy", Predefined(copy)),
        ("menu.paste", Predefined(paste)),
        ("menu.select_all", Predefined(select_all)),
        ("menu.view", Sub(view)),
        ("menu.fullscreen", Predefined(fullscreen)),
        ("menu.window", Sub(window)),
        ("menu.minimize", Predefined(minimize)),
        ("menu.zoom", Predefined(zoom)),
        ("menu.close_window", Predefined(close_window)),
        ("menu.help", Sub(help)),
    ]));
    Ok(menu)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{Duration, Instant},
    };

    use tauri::{
        Listener, WebviewUrl, WebviewWindowBuilder,
        test::{mock_builder, mock_context, noop_assets},
    };

    use super::QuitEvent::{Requested, StopNow, Stopped, Wait, WindowClosed};
    use super::*;
    use crate::test_support::run_mock_app;

    #[test]
    fn quitting_asks_only_while_runs_are_in_progress() {
        assert_eq!(
            next(
                QuitState::Idle,
                Requested {
                    host_ready: false,
                    running: 0
                }
            ),
            (QuitState::Done, QuitStep::Exit),
            "no host: nothing to stop"
        );
        assert_eq!(
            next(
                QuitState::Idle,
                Requested {
                    host_ready: true,
                    running: 0
                }
            ),
            (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns))
        );
        assert_eq!(
            next(
                QuitState::Idle,
                Requested {
                    host_ready: true,
                    running: 2
                }
            ),
            (QuitState::Asking, QuitStep::Ask(2))
        );
    }

    #[test]
    fn a_request_with_runs_in_progress_asks_and_a_repeat_is_decided_afresh() {
        // The tray's Quit, Cmd+Q (IssueRelay's own menu item on macOS) and an exit request that
        // reached `on_run_event`. Whether the system's own quit reaches the app at all is checked
        // by hand in the live acceptance; what the hook does with a request is `may_exit`.
        let (state, step) = next(
            QuitState::Idle,
            Requested {
                host_ready: true,
                running: 1,
            },
        );
        assert_eq!((state, step), (QuitState::Asking, QuitStep::Ask(1)));
        assert_eq!(
            next(
                state,
                Requested {
                    host_ready: true,
                    running: 1
                }
            ),
            (QuitState::Asking, QuitStep::Ask(1)),
            "asked again while the prompt is open: it shows again, nothing stops"
        );
        assert_eq!(
            next(
                state,
                Requested {
                    host_ready: true,
                    running: 0
                }
            ),
            (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns)),
            "the runs ended meanwhile: no question"
        );
    }

    #[test]
    fn waiting_can_still_become_stopping_now() {
        let (state, step) = next(QuitState::Asking, Wait);
        assert_eq!(
            (state, step),
            (QuitState::Waiting, QuitStep::Stop(StopMode::WaitForRuns))
        );
        // Still waiting: Quit in the tray brings the panel back and tells it again that quitting
        // waits, so that it shows "stop now" also after a reload.
        assert_eq!(
            next(
                state,
                Requested {
                    host_ready: true,
                    running: 1
                }
            ),
            (QuitState::Waiting, QuitStep::ShowPanel(1))
        );
        // The user switches to "stop now" while `stop(WaitForRuns)` has not returned: a second
        // stop with `StopRuns` starts, which ends the first; both return (core's host test
        // `stopping_now_ends_a_stop_that_is_still_waiting_for_the_runs`).
        let (state, step) = next(state, StopNow);
        assert_eq!(
            (state, step),
            (QuitState::Stopping, QuitStep::Stop(StopMode::StopRuns))
        );
        let (state, step) = next(state, Stopped);
        assert_eq!((state, step), (QuitState::Done, QuitStep::Exit));
        assert_eq!(
            next(state, Stopped),
            (QuitState::Done, QuitStep::Nothing),
            "the other stop returning exits nothing again"
        );
    }

    #[test]
    fn closing_the_window_while_quitting_does_not_exit_early() {
        for state in [
            QuitState::Idle,
            QuitState::Asking,
            QuitState::Waiting,
            QuitState::Stopping,
        ] {
            assert_eq!(
                next(state, WindowClosed),
                (state, QuitStep::HideWindow),
                "{state:?}"
            );
        }
        assert_eq!(
            next(QuitState::Done, WindowClosed),
            (QuitState::Done, QuitStep::Nothing),
            "only once quitting is done may it close"
        );
        assert_eq!(
            next(
                QuitState::Stopping,
                Requested {
                    host_ready: true,
                    running: 1
                }
            ),
            (QuitState::Stopping, QuitStep::Nothing),
            "a second Cmd+Q while stopping changes nothing"
        );
    }

    #[test]
    fn the_process_may_exit_only_after_a_stop_returned() {
        // Every way through quitting, step by step: exiting is allowed (and asked for, `Exit`)
        // only by the transition a returned `Host::stop` causes (`Stopped`), or when there was no
        // host to stop. The event loop hook lets an exit through only then (`may_exit`).
        let paths: [&[QuitEvent]; 4] = [
            &[Requested {
                host_ready: true,
                running: 0,
            }],
            &[
                Requested {
                    host_ready: true,
                    running: 2,
                },
                Wait,
            ],
            &[
                Requested {
                    host_ready: true,
                    running: 2,
                },
                Wait,
                WindowClosed,
                StopNow,
            ],
            &[
                Requested {
                    host_ready: true,
                    running: 1,
                },
                StopNow,
                Requested {
                    host_ready: true,
                    running: 1,
                },
            ],
        ];
        for events in paths {
            let mut state = QuitState::Idle;
            for event in events {
                let (after, step) = next(state, *event);
                assert_ne!(
                    step,
                    QuitStep::Exit,
                    "{events:?}: exit before any stop returned"
                );
                assert!(
                    !may_exit(after),
                    "{events:?}: {after:?} lets the process exit"
                );
                state = after;
            }
            assert_eq!(
                next(state, Stopped),
                (QuitState::Done, QuitStep::Exit),
                "{events:?}"
            );
            assert!(may_exit(QuitState::Done));
        }
        // No host at all: nothing to stop, so quitting is done at once.
        let (state, step) = next(
            QuitState::Idle,
            Requested {
                host_ready: false,
                running: 0,
            },
        );
        assert_eq!((state, step), (QuitState::Done, QuitStep::Exit));
        assert!(may_exit(state));
    }

    #[test]
    fn a_stop_that_does_not_end_is_given_up_on_in_time() {
        // A forced exit (`RunEvent::Exit`) waits a bounded while for the host's stop.
        assert!(wait_bounded(async {}, Duration::from_secs(5)));
        assert!(!wait_bounded(
            std::future::pending::<()>(),
            Duration::from_millis(50)
        ));
    }

    #[test]
    fn a_forced_exit_marks_quitting_as_stopping_and_then_waits_for_the_stop() {
        let quit = Arc::new(Mutex::new(QuitState::Asking));
        let (seen, observed) = mpsc::channel();
        let state = quit.clone();
        let outcome = forced_exit(
            &quit,
            Some(async move {
                // What quitting says while the stop runs, that is while the main thread waits.
                seen.send(*state.lock().unwrap()).unwrap();
            }),
            Duration::from_secs(5),
        );
        assert_eq!(outcome, ForcedExit::Stopped);
        assert_eq!(
            observed.try_recv(),
            Ok(QuitState::Stopping),
            "the stop ran, and only once quitting was marked as stopping"
        );
    }

    #[test]
    fn a_forced_exit_gives_up_on_a_stop_that_does_not_end() {
        let quit = Mutex::new(QuitState::Idle);
        let began = Instant::now();
        assert_eq!(
            forced_exit(
                &quit,
                Some(std::future::pending::<()>()),
                Duration::from_millis(100)
            ),
            ForcedExit::TimedOut
        );
        assert!(began.elapsed() < Duration::from_secs(5), "bounded");
        assert_eq!(*quit.lock().unwrap(), QuitState::Stopping);
    }

    #[test]
    fn a_forced_exit_once_quitting_is_done_or_without_a_host_stops_nothing() {
        let ran = Arc::new(AtomicBool::new(false));
        let quit = Mutex::new(QuitState::Done);
        let flag = ran.clone();
        let outcome = forced_exit(
            &quit,
            Some(async move { flag.store(true, Ordering::SeqCst) }),
            Duration::from_secs(5),
        );
        assert_eq!(outcome, ForcedExit::Done);
        assert!(!ran.load(Ordering::SeqCst), "the host stopped already");
        assert_eq!(*quit.lock().unwrap(), QuitState::Done);

        let quit = Mutex::new(QuitState::Idle);
        let outcome = forced_exit(
            &quit,
            None::<std::future::Ready<()>>,
            Duration::from_secs(5),
        );
        assert_eq!(outcome, ForcedExit::NoHost);
        assert_eq!(
            *quit.lock().unwrap(),
            QuitState::Idle,
            "nothing to wait for"
        );
    }

    #[test]
    fn the_application_menu_is_in_the_interface_language() {
        // Every key is one Rust looks up, so every language is checked to have it (i18n.rs).
        for key in APP_MENU_KEYS {
            assert!(
                crate::i18n::USED_KEYS.contains(&key),
                "{key} is missing from USED_KEYS"
            );
        }
        let text = |lang: &str, key: &str| app_menu_text(&Translator::new(lang), key, "IssueRelay");
        assert_eq!(text("en", "menu.about"), "About IssueRelay");
        assert_eq!(text("en", "menu.select_all"), "Select All");
        assert_eq!(text("zh-CN", "menu.about"), "关于 IssueRelay");
        assert_eq!(text("zh-CN", "menu.edit"), "编辑");
        assert_eq!(text("ja", "menu.hide"), "IssueRelayを隠す");
        assert_eq!(text("ko", "tray.quit"), "IssueRelay 종료");
        // No item is left in English in another language: the menu bar reads in one language.
        for lang in ["zh-CN", "zh-TW", "ja", "ko"] {
            for key in APP_MENU_KEYS {
                assert_ne!(text(lang, key), text("en", key), "{lang}: {key}");
            }
        }
    }

    #[test]
    fn the_quit_prompt_has_the_shape_the_panel_reads() {
        assert_eq!(
            serde_json::to_value(QuitPrompt {
                running: 2,
                waiting: false
            })
            .unwrap(),
            serde_json::json!({ "running": 2, "waiting": false })
        );
        assert_eq!(
            serde_json::to_value(QuitPrompt {
                running: 1,
                waiting: true
            })
            .unwrap(),
            serde_json::json!({ "running": 1, "waiting": true })
        );
    }

    /// What `handle` tells the panel (`quit-requested`, each payload as JSON) for `event` with
    /// quitting in `state`, on tauri's mock runtime, and where quitting stands afterwards. The
    /// mock app has no window: showing the panel only logs that it is missing.
    fn told_for(state: QuitState, event: QuitEvent) -> (Vec<serde_json::Value>, QuitState) {
        let app = mock_builder()
            .manage(AppState::new(false))
            .build(mock_context(noop_assets()))
            .unwrap();
        *app.state::<AppState>().quit.lock().unwrap() = state;
        let told = Arc::new(Mutex::new(Vec::new()));
        let log = told.clone();
        app.listen(events::QUIT_REQUESTED, move |event| {
            log.lock()
                .unwrap()
                .push(serde_json::from_str(event.payload()).unwrap());
        });
        handle(app.handle(), event);
        let told = told.lock().unwrap().clone();
        (told, current(app.handle()))
    }

    #[test]
    fn asking_tells_the_panel_the_runs_and_that_nothing_waits_yet() {
        let request = Requested {
            host_ready: true,
            running: 2,
        };
        for state in [QuitState::Idle, QuitState::Asking] {
            assert_eq!(
                told_for(state, request),
                (
                    vec![serde_json::json!({ "running": 2, "waiting": false })],
                    QuitState::Asking
                ),
                "{state:?}"
            );
        }
    }

    #[test]
    fn a_request_while_waiting_tells_the_panel_again_that_quitting_waits() {
        // A panel reloaded meanwhile has lost the waiting prompt: told again, it shows "stop now".
        assert_eq!(
            told_for(
                QuitState::Waiting,
                Requested {
                    host_ready: true,
                    running: 1
                }
            ),
            (
                vec![serde_json::json!({ "running": 1, "waiting": true })],
                QuitState::Waiting
            )
        );
        // While stopping there is nothing left to ask.
        assert_eq!(
            told_for(
                QuitState::Stopping,
                Requested {
                    host_ready: true,
                    running: 1
                }
            ),
            (vec![], QuitState::Stopping)
        );
    }

    #[test]
    fn the_event_loop_hook_lets_a_forced_exit_without_a_host_end_at_once() {
        // The hook itself, on tauri's mock runtime: with no host there is nothing to stop and
        // nothing to wait for. (With a host it hands `forced_exit` the host's stop; making one
        // takes a real host, which these tests do not start.)
        let app = mock_builder()
            .manage(AppState::new(false))
            .build(mock_context(noop_assets()))
            .unwrap();
        let began = Instant::now();
        on_run_event(app.handle(), RunEvent::Exit);
        // Far below the 10 s a forced exit waits for a stop, with room for a slow CI machine.
        assert!(began.elapsed() < Duration::from_secs(5));
        assert_eq!(current(app.handle()), QuitState::Idle);
    }

    /// Runs the event loop of a mock app through `on_run_event` with quitting in `state`, closes
    /// the panel window (destroys it: the last window, so the loop asks to exit), and once that
    /// request was held lets quitting be done and closes a window again. Returns what reached the
    /// hook, in order.
    fn exit_requests_while(state: QuitState) -> Vec<String> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let mut released = false;
        run_mock_app(
            Duration::from_secs(30),
            move |app| *app.state::<AppState>().quit.lock().unwrap() = state,
            move |app, event| {
                match &event {
                    RunEvent::Ready => {
                        WebviewWindowBuilder::new(app, "main", WebviewUrl::default())
                            .build()
                            .unwrap()
                            .destroy()
                            .unwrap();
                    }
                    RunEvent::ExitRequested { .. } => log
                        .lock()
                        .unwrap()
                        .push(format!("exit requested while {:?}", current(app))),
                    // A held request leaves the loop running: the next round comes.
                    RunEvent::MainEventsCleared if !released && !log.lock().unwrap().is_empty() => {
                        released = true;
                        *app.state::<AppState>().quit.lock().unwrap() = QuitState::Done;
                        WebviewWindowBuilder::new(app, "again", WebviewUrl::default())
                            .build()
                            .unwrap()
                            .destroy()
                            .unwrap();
                    }
                    RunEvent::Exit => log.lock().unwrap().push("exit".into()),
                    _ => {}
                }
                on_run_event(app, event);
            },
        );
        let seen = seen.lock().unwrap();
        seen.clone()
    }

    #[test]
    fn every_exit_request_before_quitting_is_done_is_held() {
        // Through the real hook on tauri's mock runtime. It can only ask to exit the way a last
        // window going does (`code: None`): its `request_exit` is unimplemented, and so is every
        // way that ends in `AppHandle::exit`. Idle and Asking are therefore left out: without a
        // host, `request` there quits at once. The hook does not look at the state otherwise
        // (`may_exit`, tested above).
        for state in [QuitState::Waiting, QuitState::Stopping] {
            assert_eq!(
                exit_requests_while(state),
                [
                    format!("exit requested while {state:?}"),
                    "exit requested while Done".to_string(),
                    "exit".to_string()
                ],
                "the first request was not held"
            );
        }
    }
}
