//! Starts the host in the background so the tray appears at once, then decides whether to show
//! the panel (the window is shown once `Host::start` returned or failed; it no longer waits for
//! Linear, which connects in the background). How the start went is kept in `AppState`
//! (`startup_state`), so a panel that opens later still learns it; `startup-failed` tells a panel
//! that is already open.
use std::{path::Path, sync::Arc};

use issuerelay_core::{
    host::{Host, HostParts, SystemConnector},
    ipc::server::AlreadyRunning,
    paths::AppPaths,
    text::clean_text,
};
use tauri::{AppHandle, Emitter, Manager};

use crate::{
    events, logging,
    state::{AppState, StartupCode, StartupFailure, StartupState},
    tray, window,
};

/// Whether the panel shows once the host is up (`start`). A login start stays in the tray, unless
/// the first-run guide has been neither finished nor set aside, which the panel must show, or the
/// previous run left the panel marker: the install it began has ended
/// (`update::install_and_restart`), and the restart keeps `--autostart`, so its outcome would go
/// unseen.
pub fn shows_panel(autostarted: bool, onboarding_completed: bool, panel_requested: bool) -> bool {
    !autostarted || !onboarding_completed || panel_requested
}

/// Whether the window is hidden from the first moment (lib.rs): a login start does that, unless
/// the previous run asked for the panel.
pub fn starts_hidden(autostarted: bool, panel_requested: bool) -> bool {
    autostarted && !panel_requested
}

/// Starts the host with the system's paths and the real connector, records how that went and
/// shows the panel as `shows_panel` says (`panel_requested`: the previous run's marker).
pub async fn start(app: AppHandle, panel_requested: bool) {
    let result = match AppPaths::system() {
        Ok(paths) => {
            let state_db = paths.state_db();
            Host::start(paths, Arc::new(SystemConnector), HostParts::default())
                .await
                .map_err(|error| failure(&error, &state_db))
        }
        Err(error) => Err(StartupFailure {
            code: StartupCode::Other,
            detail: clean_text(&format!("{error:#}")),
        }),
    };
    let state = app.state::<AppState>();
    match result {
        Ok(host) => {
            // Recorded before anything else happens. From here on a quit (the tray's, Cmd+Q's,
            // the system's) finds this host and stops it, an exit the system forces waits for its
            // stop, and a panel the window below shows reads `Ready`. Recorded after the
            // forwarder, the tray and the window instead, a quit in between would end the app
            // without stopping a host whose engine already runs. (`start` needs a real host, so
            // this order is argued here rather than tested.)
            state.set_host(Ok(host.clone()));
            events::spawn_forwarder(app.clone(), host.clone());
            tray::refresh(&app, &host);
            // A login start stays in the tray (lib.rs hid it already), unless the first-run guide
            // has been neither finished nor set aside ("Set up later"): then the panel shows it.
            let onboarded = host.engine().settings().onboarding_completed;
            if shows_panel(state.autostarted, onboarded, panel_requested) {
                window::show(&app);
            }
        }
        Err(failure) => {
            tracing::error!(
                code = ?failure.code,
                detail = %failure.detail,
                "IssueRelay could not start its engine"
            );
            tray::show_failed_start(&app);
            window::show(&app);
            state.set_host(Err(failure));
            // Recorded first, then announced: a panel that hears this and asks `startup_state`
            // gets the same answer.
            if let StartupState::Failed(failure) = state.startup_state() {
                logging::debug_failed(
                    app.emit(events::STARTUP_FAILED, StartupState::Failed(failure)),
                    "telling the panel the start failed",
                );
            }
        }
    }
}

/// A start failure as the panel shows it: a code it translates, and the cleaned error as the
/// detail line.
pub fn failure(error: &anyhow::Error, state_db: &Path) -> StartupFailure {
    let message = format!("{error:#}");
    let folder = state_db.parent().unwrap_or(state_db);
    let code = if error.downcast_ref::<AlreadyRunning>().is_some() {
        StartupCode::AlreadyRunning
    } else if message.contains(&state_db.display().to_string())
        || message.starts_with(&format!("cannot create {}", folder.display()))
    {
        // `Store::open` names the database file, or its folder when it cannot create it (on macOS
        // the IPC endpoint creates that same folder first, in the same words): either way the run
        // records cannot be kept. The other IPC errors name the socket or its lock and stay
        // `Other`.
        StartupCode::StoreUnavailable
    } else {
        StartupCode::Other
    };
    StartupFailure {
        code,
        detail: clean_text(&message),
    }
}

#[cfg(test)]
mod tests {
    use anyhow::anyhow;

    use super::*;

    #[test]
    fn a_login_start_shows_the_panel_only_when_it_has_something_to_tell() {
        // (started by the login item, first-run guide done, an update's restart asked for the
        // panel) and whether the panel shows once the host is up.
        for (autostarted, onboarded, asked_for, shows) in [
            (false, true, false, true),
            (false, false, false, true),
            (true, false, false, true),
            (true, true, false, false),
            (true, true, true, true),
        ] {
            assert_eq!(
                shows_panel(autostarted, onboarded, asked_for),
                shows,
                "autostarted {autostarted}, onboarded {onboarded}, asked for {asked_for}"
            );
        }
    }

    #[test]
    fn a_login_start_is_hidden_from_the_first_moment_unless_the_panel_was_asked_for() {
        assert!(starts_hidden(true, false));
        assert!(!starts_hidden(true, true), "the update's restart asked");
        assert!(!starts_hidden(false, false));
        assert!(!starts_hidden(false, true));
    }

    #[test]
    fn start_failures_are_told_apart() {
        let db = Path::new("data").join("state.db");
        let second = failure(&anyhow::Error::new(AlreadyRunning), &db);
        assert_eq!(second.code, StartupCode::AlreadyRunning);
        assert_eq!(
            second.detail,
            "another IssueRelay instance is already running"
        );
        let store = anyhow!("disk I/O error\n").context(format!("cannot open {}", db.display()));
        let store = failure(&store, &db);
        assert_eq!(store.code, StartupCode::StoreUnavailable);
        assert!(
            !store.detail.contains('\n'),
            "the detail is one cleaned line: {:?}",
            store.detail
        );
        let folder = anyhow!("permission denied")
            .context(format!("cannot create {}", Path::new("data").display()));
        assert_eq!(
            failure(&folder, &db).code,
            StartupCode::StoreUnavailable,
            "the run records' folder"
        );
        let lock = anyhow!("permission denied").context(format!(
            "cannot lock {}",
            Path::new("data").join("issuerelay.lock").display()
        ));
        assert_eq!(
            failure(&lock, &db).code,
            StartupCode::Other,
            "the IPC endpoint's lock is not the store"
        );
        let other = failure(&anyhow!("this system has no local data directory"), &db);
        assert_eq!(other.code, StartupCode::Other);
    }
}
