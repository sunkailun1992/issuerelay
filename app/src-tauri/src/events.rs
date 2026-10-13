//! Forwards the engine's events to the panel, turns job changes into notifications and keeps
//! the tray current. A panel that fell behind gets one `engine-resync` and reads everything again.
use std::collections::HashMap;

use issuerelay_core::{
    engine::EngineEvent,
    host::Host,
    model::{Job, JobState},
};
use tauri::{AppHandle, Emitter};
use tokio::sync::broadcast::error::RecvError;

use crate::{logging, notify, tray};

/// An engine event for the panel; the payload is the `EngineEvent` as serialized.
pub const ENGINE_EVENT: &str = "engine-event";
/// Events were lost: the panel reads everything again.
pub const RESYNC_EVENT: &str = "engine-resync";
/// Quitting asks the panel "wait or stop now"; the payload is the number of runs in progress.
pub const QUIT_REQUESTED: &str = "quit-requested";
/// The host could not start; the payload is the `StartupState::Failed`.
pub const STARTUP_FAILED: &str = "startup-failed";

/// How many recent jobs seed the known states, so that existing jobs do not notify on start.
const SEED_JOBS: usize = 500;

/// The recent jobs' states; `None`, with a warning, when the job records cannot be read.
fn seed(host: &Host) -> Option<HashMap<i64, JobState>> {
    match host.engine().list_jobs(SEED_JOBS) {
        Ok(jobs) => Some(jobs.into_iter().map(|job| (job.id, job.state)).collect()),
        Err(e) => {
            tracing::warn!(
                error = %format!("{e:#}"),
                "could not read the jobs; no job notifies until they can be read"
            );
            None
        }
    }
}

/// The notification a job's change calls for. Without a seed nothing notifies: the seed is tried
/// again (`reseed`) and the job then counts as known, never as new, so a store error cannot make
/// every job look new.
fn notification(
    known: &mut Option<HashMap<i64, JobState>>,
    job: &Job,
    reseed: impl FnOnce() -> Option<HashMap<i64, JobState>>,
) -> Option<notify::NotificationKind> {
    let Some(states) = known.as_mut() else {
        *known = reseed();
        if let Some(states) = known.as_mut() {
            states.insert(job.id, job.state);
        }
        return None;
    };
    let previous = states.insert(job.id, job.state);
    notify::notification_for(previous, job)
}

/// What the forwarder does with one result from the engine's channel.
#[derive(Debug, PartialEq)]
pub enum Forward {
    /// Boxed: a job event carries a whole `Job`, which would make every `Forward` that large.
    Event(Box<EngineEvent>),
    /// Events were lost: one resync, then forwarding goes on.
    Resync,
    /// The engine is gone.
    Stop,
}

/// What to do with what the engine's channel gave.
pub fn forward(received: Result<EngineEvent, RecvError>) -> Forward {
    match received {
        Ok(event) => Forward::Event(Box::new(event)),
        Err(RecvError::Lagged(_)) => Forward::Resync,
        Err(RecvError::Closed) => Forward::Stop,
    }
}

/// Forwards the host's engine events to the panel, the notifications and the tray until the
/// engine is gone.
pub fn spawn_forwarder(app: AppHandle, host: Host) {
    tauri::async_runtime::spawn(async move {
        let mut events = host.engine().subscribe();
        let mut known = seed(&host);
        loop {
            match forward(events.recv().await) {
                Forward::Event(event) => {
                    if let EngineEvent::Job { job } = event.as_ref()
                        && let Some(kind) = notification(&mut known, job, || seed(&host))
                    {
                        notify::show(&app, &host, kind, job);
                    }
                    logging::debug_failed(
                        app.emit(ENGINE_EVENT, event.as_ref()),
                        "forwarding an engine event",
                    );
                    tray::refresh(&app, &host);
                }
                Forward::Resync => {
                    tracing::debug!("the panel fell behind the engine's events");
                    known = seed(&host);
                    logging::debug_failed(
                        app.emit(RESYNC_EVENT, ()),
                        "asking the panel to read everything again",
                    );
                    tray::refresh(&app, &host);
                }
                Forward::Stop => return,
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use tokio::sync::broadcast;

    use super::*;
    use crate::{notify::NotificationKind, test_support::job};

    #[tokio::test]
    async fn a_lagged_panel_gets_one_resync_and_then_the_events_after_it() {
        let (sender, mut receiver) = broadcast::channel(2);
        for _ in 0..4 {
            sender.send(EngineEvent::Status).unwrap();
        }
        sender.send(EngineEvent::Config).unwrap();
        drop(sender);
        let mut seen = Vec::new();
        loop {
            match forward(receiver.recv().await) {
                Forward::Stop => break,
                other => seen.push(other),
            }
        }
        // The channel kept the last two events; the three before them were lost.
        assert_eq!(
            seen,
            [
                Forward::Resync,
                Forward::Event(Box::new(EngineEvent::Status)),
                Forward::Event(Box::new(EngineEvent::Config))
            ]
        );
    }

    #[test]
    fn without_readable_job_records_no_job_notifies() {
        let mut known = None;
        let failed = job(JobState::Failed);
        assert_eq!(
            notification(&mut known, &failed, || None),
            None,
            "still unreadable: nothing notifies"
        );
        assert_eq!(
            notification(&mut known, &failed, || Some(HashMap::new())),
            None,
            "the first readable seed only records the job, it never makes it look new"
        );
        let running = job(JobState::Running);
        assert_eq!(
            notification(&mut known, &running, || unreachable!("seeded already")),
            Some(NotificationKind::RunStarted)
        );
    }

    #[test]
    fn a_seeded_job_notifies_only_when_its_state_changes() {
        let mut known = Some(HashMap::from([(1, JobState::Queued)]));
        assert_eq!(
            notification(&mut known, &job(JobState::Queued), || None),
            None
        );
        assert_eq!(
            notification(&mut known, &job(JobState::Running), || None),
            Some(NotificationKind::RunStarted)
        );
    }
}
