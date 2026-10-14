//! What the app keeps for its whole life: the host once started (or why it could not start),
//! where quitting stands, and whether the login item started it.
use std::sync::Mutex;

use issuerelay_core::host::Host;
use serde::Serialize;
use tokio::sync::{Notify, OnceCell};

use crate::quit::QuitState;

/// Why the host could not start, as a code the panel translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupCode {
    /// Another host (the tray app or the devhost) serves this user.
    AlreadyRunning,
    /// The run records (`state.db`) cannot be opened.
    StoreUnavailable,
    Other,
}

/// A failed start: its code, and the cleaned error the panel shows as a detail line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StartupFailure {
    pub code: StartupCode,
    pub detail: String,
}

/// How the start of the host went: the `startup_state` command's answer, and the payload of
/// `startup-failed` for a panel that is already open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StartupState {
    Starting,
    /// `Host::start` returned. Linear may still be connecting (`LinearSetup::Connecting`):
    /// the host no longer waits for it.
    Ready,
    Failed(StartupFailure),
}

pub struct AppState {
    /// Set once by the startup task: the running host, or why it could not start.
    host: OnceCell<Result<Host, StartupFailure>>,
    ready: Notify,
    /// Where quitting stands; only `quit::handle` changes it.
    pub quit: Mutex<QuitState>,
    /// Started by the login item (`AUTOSTART_ARG`): stay in the tray instead of showing the panel.
    pub autostarted: bool,
}

impl AppState {
    pub fn new(autostarted: bool) -> Self {
        Self {
            host: OnceCell::new(),
            ready: Notify::new(),
            quit: Mutex::new(QuitState::Idle),
            autostarted,
        }
    }

    /// Records how the start went and wakes every command waiting for it.
    pub fn set_host(&self, result: Result<Host, StartupFailure>) {
        if self.host.set(result).is_err() {
            tracing::warn!("the start of the host was recorded twice; the first is kept");
        }
        self.ready.notify_waiters();
    }

    /// The host, waiting while it starts; the failure's detail when it could not start.
    pub async fn host(&self) -> Result<Host, String> {
        loop {
            // Created before the check, so a `set_host` in between still wakes this wait
            // (`notify_waiters` reaches every `Notified` created before it, polled or not).
            let notified = self.ready.notified();
            if let Some(result) = self.host.get() {
                return result.clone().map_err(|failure| failure.detail);
            }
            notified.await;
        }
    }

    /// The host if it has started.
    pub fn try_host(&self) -> Option<Host> {
        self.host
            .get()
            .and_then(|result| result.as_ref().ok().cloned())
    }

    /// Starting, ready or failed (with why), at any time.
    pub fn startup_state(&self) -> StartupState {
        match self.host.get() {
            None => StartupState::Starting,
            Some(Ok(_)) => StartupState::Ready,
            Some(Err(failure)) => StartupState::Failed(failure.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Duration};

    use serde_json::json;

    use super::*;

    #[tokio::test]
    async fn commands_wait_for_the_host_and_see_a_failed_start() {
        let state = Arc::new(AppState::new(false));
        assert_eq!(state.startup_state(), StartupState::Starting);
        let waiting = tokio::spawn({
            let state = state.clone();
            async move { state.host().await.err() }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        let failure = StartupFailure {
            code: StartupCode::AlreadyRunning,
            detail: "another IssueRelay instance is already running".into(),
        };
        state.set_host(Err(failure.clone()));
        assert_eq!(
            waiting.await.unwrap().as_deref(),
            Some("another IssueRelay instance is already running")
        );
        assert!(state.try_host().is_none());
        // Still known afterwards: a panel that opens later learns why, not only one that listened.
        assert_eq!(state.startup_state(), StartupState::Failed(failure));
    }

    #[test]
    fn the_startup_state_has_the_shape_the_panel_reads() {
        assert_eq!(
            serde_json::to_value(StartupState::Starting).unwrap(),
            json!({ "kind": "starting" })
        );
        assert_eq!(
            serde_json::to_value(StartupState::Ready).unwrap(),
            json!({ "kind": "ready" })
        );
        let failed = StartupState::Failed(StartupFailure {
            code: StartupCode::StoreUnavailable,
            detail: "cannot open state.db".into(),
        });
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            json!({ "kind": "failed", "code": "store_unavailable", "detail": "cannot open state.db" })
        );
    }
}
