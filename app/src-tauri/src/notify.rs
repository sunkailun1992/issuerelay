//! System notifications for the four moments design 9.5 names, each of which can be switched
//! off. A notification names the issue and says what happened in the user's language; the
//! engine's own text (a failure's reason) stays in the panel, under a translated headline.
use issuerelay_core::{
    config::NotificationSettings,
    host::Host,
    model::{Job, JobState},
};
use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

use crate::i18n::Translator;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    RunStarted,
    AwaitingAcceptance,
    NeedsInfo,
    Failed,
}

/// The notification a job's change calls for: only when its state changed into one of the four.
pub fn notification_for(previous: Option<JobState>, job: &Job) -> Option<NotificationKind> {
    if previous == Some(job.state) {
        return None;
    }
    match job.state {
        JobState::Running => Some(NotificationKind::RunStarted),
        JobState::AwaitingAcceptance => Some(NotificationKind::AwaitingAcceptance),
        JobState::NeedsInfo => Some(NotificationKind::NeedsInfo),
        JobState::Failed => Some(NotificationKind::Failed),
        _ => None,
    }
}

/// Whether the settings let `kind` be shown.
pub fn enabled(kind: NotificationKind, settings: &NotificationSettings) -> bool {
    match kind {
        NotificationKind::RunStarted => settings.run_started,
        NotificationKind::AwaitingAcceptance => settings.awaiting_acceptance,
        NotificationKind::NeedsInfo => settings.needs_info,
        NotificationKind::Failed => settings.failed,
    }
}

/// Title and body: the issue (its title was cleaned when the engine stored it) and a translated
/// sentence. Never the engine's text.
pub fn content(kind: NotificationKind, job: &Job, t: &Translator) -> (String, String) {
    let title = format!("{} {}", job.issue_identifier, job.title);
    let key = match kind {
        NotificationKind::RunStarted => "notify.run_started",
        NotificationKind::AwaitingAcceptance => "notify.awaiting_acceptance",
        NotificationKind::NeedsInfo => "notify.needs_info",
        NotificationKind::Failed => "notify.failed",
    };
    (title, t.t(key))
}

/// Shows the notification unless the settings switched that kind off.
pub fn show(app: &AppHandle, host: &Host, kind: NotificationKind, job: &Job) {
    let settings = host.engine().settings();
    if !enabled(kind, &settings.notifications) {
        return;
    }
    let (title, body) = content(kind, job, &Translator::for_settings(&settings));
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        tracing::warn!(error = %e, "could not show a notification");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::job;

    #[test]
    fn notifications_follow_state_changes_only() {
        use JobState::*;
        assert_eq!(
            notification_for(Some(Queued), &job(Running)),
            Some(NotificationKind::RunStarted)
        );
        assert_eq!(
            notification_for(None, &job(Running)),
            Some(NotificationKind::RunStarted),
            "a job first seen running"
        );
        assert_eq!(
            notification_for(Some(Running), &job(AwaitingAcceptance)),
            Some(NotificationKind::AwaitingAcceptance)
        );
        assert_eq!(
            notification_for(Some(Running), &job(NeedsInfo)),
            Some(NotificationKind::NeedsInfo)
        );
        assert_eq!(
            notification_for(Some(Running), &job(Failed)),
            Some(NotificationKind::Failed)
        );
        assert_eq!(
            notification_for(Some(Running), &job(Running)),
            None,
            "no change, no notification"
        );
        for quiet in [Queued, WaitingBinding, Done, Canceled] {
            assert_eq!(
                notification_for(Some(Running), &job(quiet)),
                None,
                "{quiet:?}"
            );
        }
    }

    #[test]
    fn each_kind_can_be_switched_off() {
        let mut settings = NotificationSettings::default();
        assert!(enabled(NotificationKind::Failed, &settings));
        settings.failed = false;
        assert!(!enabled(NotificationKind::Failed, &settings));
        assert!(enabled(NotificationKind::RunStarted, &settings));
    }

    #[test]
    fn a_notification_names_the_issue_and_never_carries_the_engines_text() {
        let mut failed = job(JobState::Failed);
        failed.reason = Some("run exceeded the time limit".into());
        let (title, body) = content(NotificationKind::Failed, &failed, &Translator::new("en"));
        assert_eq!(title, "DEMO-1 Demo task");
        assert_eq!(body, "The run failed. The panel shows why.");
        assert!(
            !body.contains("time limit"),
            "the reason stays in the panel"
        );
    }
}
