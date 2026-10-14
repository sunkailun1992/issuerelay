//! What the unit tests of several modules share: a job with every field filled in, the status of
//! an engine whose last check worked (all of it fictional), a mock app (tauri's `MockRuntime`)
//! whose event loop runs on a thread of its own, and a downloaded update that installs by writing
//! down what it was given.
use std::{
    sync::{
        Arc, Mutex,
        mpsc::{self, RecvTimeoutError},
    },
    time::Duration,
};

use chrono::{TimeZone, Utc};
use issuerelay_core::{
    config::ExecutorKind,
    engine::StatusSnapshot,
    linear::Viewer,
    model::{Job, JobState},
};
use tauri::{
    App, AppHandle, RunEvent,
    test::{MockRuntime, mock_builder, mock_context, noop_assets},
};

use crate::{state::AppState, update::Install};

/// Builds a mock app with `AppState` managed, lets `setup` prepare it, runs its event loop with
/// `on_event` as the hook and returns once the loop has ended; fails if it has not ended within
/// `limit`. The loop runs on a thread of its own, which is then the app's main thread; it ends
/// once its last window is gone and the exit that follows is not held. (The mock loop handles
/// one message per round and sleeps a second between rounds.)
pub fn run_mock_app<S, E>(limit: Duration, setup: S, mut on_event: E)
where
    S: FnOnce(&App<MockRuntime>) + Send + 'static,
    E: FnMut(&AppHandle<MockRuntime>, RunEvent) + Send + 'static,
{
    let (ended, wait) = mpsc::channel();
    let thread = std::thread::spawn(move || {
        let app = mock_builder()
            .manage(AppState::new(false))
            .build(mock_context(noop_assets()))
            .expect("the mock app builds");
        setup(&app);
        app.run(move |app, event| on_event(app, event));
        ended.send(()).expect("the test waits for the loop");
    });
    match wait.recv_timeout(limit) {
        Ok(()) => {}
        // The app's thread panicked, an assertion in `on_event` say: pass that panic on.
        Err(RecvTimeoutError::Disconnected) => {
            if let Err(panic) = thread.join() {
                std::panic::resume_unwind(panic);
            }
        }
        Err(RecvTimeoutError::Timeout) => {
            panic!("the mock app's event loop did not end within {limit:?}")
        }
    }
}

/// `DEMO-1` in `state`, with each of `Job`'s 18 fields set.
pub fn job(state: JobState) -> Job {
    let at = Utc.with_ymd_and_hms(2026, 10, 12, 9, 0, 0).unwrap();
    Job {
        id: 1,
        issue_uuid: "issue-1".into(),
        issue_identifier: "DEMO-1".into(),
        title: "Demo task".into(),
        binding_id: Some("proj-1".into()),
        directory: Some("demo-app".into()),
        executor: Some(ExecutorKind::Claude),
        state,
        reason: None,
        priority: 2,
        issue_created_at: at,
        discovered_updated_at: at,
        verified_updated_at: None,
        created_at: at,
        started_at: Some(at),
        finished_at: None,
        log_path: None,
        summary: None,
    }
}

/// An engine that checked Linear just now and has nothing to do, with each of
/// `StatusSnapshot`'s 10 fields set.
pub fn checked() -> StatusSnapshot {
    StatusSnapshot {
        paused: false,
        linear_ok: true,
        viewer: Some(Viewer {
            id: "user-1".into(),
            name: "Jane Doe".into(),
        }),
        last_poll_at: Some(Utc::now()),
        last_error: None,
        running: 0,
        queued: 0,
        waiting: 0,
        draining: false,
        held: Vec::new(),
    }
}

/// What the fake installs wrote down: the payload of each install, in order.
pub type InstallLog = Arc<Mutex<Vec<Vec<u8>>>>;

/// A verified update that installs by recording its payload in `installed`, so that a test can see
/// whether, and what, was installed. Nothing of the updater plugin is involved.
pub struct FakeInstall {
    pub version: String,
    pub payload: Vec<u8>,
    pub installed: InstallLog,
}

/// A fake for `version` with a payload, and the log of what its install records.
pub fn fake_install(version: &str, payload: &[u8]) -> (Box<dyn Install>, InstallLog) {
    let installed = InstallLog::default();
    let fake = FakeInstall {
        version: version.to_string(),
        payload: payload.to_vec(),
        installed: installed.clone(),
    };
    (Box::new(fake), installed)
}

impl Install for FakeInstall {
    fn version(&self) -> &str {
        &self.version
    }

    fn install(self: Box<Self>) -> Result<(), String> {
        self.installed.lock().unwrap().push(self.payload);
        Ok(())
    }
}
