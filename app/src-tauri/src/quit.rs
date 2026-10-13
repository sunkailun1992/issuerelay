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
use std::{sync::PoisonError, time::Duration};

use issuerelay_core::host::StopMode;
use tauri::{AppHandle, Emitter, Manager, RunEvent};

use crate::{events, logging, state::AppState, window};

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
    /// Show the panel and ask (`quit-requested` with the number of runs in progress).
    Ask(usize),
    /// Show the panel again: it still shows the waiting prompt with "stop now".
    ShowPanel,
    /// Stop the host as the mode says, in the background; `Stopped` follows.
    Stop(StopMode),
    /// Hide the window instead of letting it close.
    HideWindow,
    /// Exit the process.
    Exit,
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
        (S::Waiting, E::Requested { .. }) => (S::Waiting, QuitStep::ShowPanel),
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
pub fn handle(app: &AppHandle, event: QuitEvent) -> QuitStep {
    let state = app.state::<AppState>();
    let step = {
        let mut quit = state.quit.lock().unwrap_or_else(PoisonError::into_inner);
        let (next_state, step) = next(*quit, event);
        *quit = next_state;
        step
    };
    match step {
        QuitStep::Nothing | QuitStep::HideWindow => {}
        QuitStep::Ask(running) => {
            window::show(app);
            logging::debug_failed(
                app.emit(events::QUIT_REQUESTED, running),
                "asking the panel about quitting",
            );
        }
        QuitStep::ShowPanel => window::show(app),
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
            // Nothing to stop.
            None => {
                handle(app, QuitEvent::Stopped);
            }
        },
        QuitStep::Exit => app.exit(0),
    }
    step
}

/// The tray's Quit, Cmd+Q and an exit request, with the host as it is now.
pub fn request(app: &AppHandle) {
    let host = app.state::<AppState>().try_host();
    let running = host
        .as_ref()
        .map_or(0, |host| host.engine().status().running);
    handle(
        app,
        QuitEvent::Requested {
            host_ready: host.is_some(),
            running,
        },
    );
}

/// Where quitting stands now.
fn current(app: &AppHandle) -> QuitState {
    *app.state::<AppState>()
        .quit
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
}

/// The app's event loop hook. A request to exit (`ExitRequested`) goes ahead only once quitting is
/// done (`may_exit`), which is when IssueRelay's own `exit(0)` comes; any other is held and goes
/// through `request`, which asks while runs are in progress. An exit that cannot be held
/// (`RunEvent::Exit` before quitting is done: the system is ending the app) stops the host first,
/// waiting at most `FORCED_EXIT_STOP`, so that the runs end with their trees and are recorded.
pub fn on_run_event(app: &AppHandle, event: RunEvent) {
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
            if may_exit(current(app)) {
                return;
            }
            let Some(host) = app.state::<AppState>().try_host() else {
                return;
            };
            tracing::warn!(
                "the system is ending IssueRelay without asking; stopping the runs first"
            );
            let stopped = wait_bounded(
                async move { host.stop(StopMode::StopRuns).await },
                FORCED_EXIT_STOP,
            );
            if !stopped {
                tracing::error!(
                    seconds = FORCED_EXIT_STOP.as_secs(),
                    "the host did not stop in time; IssueRelay ends anyway"
                );
            }
        }
        _ => {}
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

/// macOS's application menu: Tauri's default one (`Menu::default` in tauri 2.12.3) except for
/// Quit, which is an item of IssueRelay's own with Cmd+Q instead of the system's `terminate:`, so
/// that quitting asks while runs are in progress (see the module comment). Windows has no
/// application menu: there the tray's Quit is the way to quit.
#[cfg(target_os = "macos")]
pub fn app_menu(app: &AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use tauri::menu::{
        AboutMetadata, HELP_SUBMENU_ID, Menu, MenuItem, PredefinedMenuItem, Submenu,
        WINDOW_SUBMENU_ID,
    };

    use crate::i18n::{Translator, resolve_language};

    let package = app.package_info();
    let config = app.config();
    let about = AboutMetadata {
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
    let quit = MenuItem::with_id(
        app,
        QUIT_MENU_ID,
        t.t("tray.quit"),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    Menu::with_items(
        app,
        &[
            &Submenu::with_items(
                app,
                &package.name,
                true,
                &[
                    &PredefinedMenuItem::about(app, None, Some(about))?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::services(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::hide(app, None)?,
                    &PredefinedMenuItem::hide_others(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &quit,
                ],
            )?,
            &Submenu::with_items(
                app,
                "File",
                true,
                &[&PredefinedMenuItem::close_window(app, None)?],
            )?,
            &Submenu::with_items(
                app,
                "Edit",
                true,
                &[
                    &PredefinedMenuItem::undo(app, None)?,
                    &PredefinedMenuItem::redo(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::cut(app, None)?,
                    &PredefinedMenuItem::copy(app, None)?,
                    &PredefinedMenuItem::paste(app, None)?,
                    &PredefinedMenuItem::select_all(app, None)?,
                ],
            )?,
            &Submenu::with_items(
                app,
                "View",
                true,
                &[&PredefinedMenuItem::fullscreen(app, None)?],
            )?,
            &Submenu::with_id_and_items(
                app,
                WINDOW_SUBMENU_ID,
                "Window",
                true,
                &[
                    &PredefinedMenuItem::minimize(app, None)?,
                    &PredefinedMenuItem::maximize(app, None)?,
                    &PredefinedMenuItem::separator(app)?,
                    &PredefinedMenuItem::close_window(app, None)?,
                ],
            )?,
            &Submenu::with_id_and_items(app, HELP_SUBMENU_ID, "Help", true, &[])?,
        ],
    )
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::QuitEvent::{Requested, StopNow, Stopped, Wait, WindowClosed};
    use super::*;

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
        // reached `on_run_event`. Whether the system's own quit reaches the app at all is for
        // Task 13 Step 4 to check by hand; what the hook does with a request is `may_exit`.
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
        // Still waiting: Quit in the tray brings the panel back, which still shows "stop now".
        assert_eq!(
            next(
                state,
                Requested {
                    host_ready: true,
                    running: 1
                }
            ),
            (QuitState::Waiting, QuitStep::ShowPanel)
        );
        // The user switches to "stop now" while `stop(WaitForRuns)` has not returned: a second
        // stop with `StopRuns` starts, which ends the first; both return (core Task 9).
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
}
