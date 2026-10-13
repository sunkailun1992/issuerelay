//! Values the unit tests of several modules share: a job with every field filled in, and the
//! status of an engine whose last check worked. All of it is fictional.
use chrono::{TimeZone, Utc};
use issuerelay_core::{
    config::ExecutorKind,
    engine::StatusSnapshot,
    linear::Viewer,
    model::{Job, JobState},
};

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
