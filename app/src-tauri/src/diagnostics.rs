//! Chat projections of desktop diagnostics. Keep platform and updater details in the panel.
use issuerelay_core::{
    cli_path::CliPathStatus,
    ipc::server::{DesktopDiagnostic, DesktopDiagnostics},
};
use serde_json::{Value, json};
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    commands::{read_autostart_status, read_cli_path_status},
    state::AppState,
    update::{self, AppUpdates, UpdateStatus},
};

pub(crate) struct DesktopDiagnosticProvider<R: Runtime>(AppHandle<R>);

impl<R: Runtime> DesktopDiagnosticProvider<R> {
    pub(crate) fn new(app: AppHandle<R>) -> Self {
        Self(app)
    }
}

fn kind(kind: &str) -> Value {
    json!({"kind": kind})
}

fn cli_path_status(status: CliPathStatus) -> Value {
    kind(match status {
        CliPathStatus::Off => "off",
        CliPathStatus::On { .. } => "on",
        CliPathStatus::Stale { .. } => "stale",
        CliPathStatus::Blocked { .. } => "blocked",
        CliPathStatus::Unavailable { .. } => "unavailable",
    })
}

fn update_status(status: UpdateStatus) -> Value {
    match status {
        UpdateStatus::Unchecked => kind("unchecked"),
        UpdateStatus::UpToDate { .. } => kind("up_to_date"),
        UpdateStatus::Available { version }
            if version.len() <= 128 && semver::Version::parse(&version).is_ok() =>
        {
            json!({"kind": "available", "version": version})
        }
        UpdateStatus::Available { .. } | UpdateStatus::Failed(_) => kind("failed"),
    }
}

#[async_trait::async_trait]
impl<R: Runtime> DesktopDiagnostics for DesktopDiagnosticProvider<R> {
    async fn run(&self, diagnostic: DesktopDiagnostic) -> anyhow::Result<Value> {
        let app = &self.0;
        let result = match diagnostic {
            DesktopDiagnostic::AutostartStatus => match read_autostart_status(app) {
                Ok(true) => kind("enabled"),
                Ok(false) => kind("disabled"),
                Err(_) => kind("unavailable"),
            },
            DesktopDiagnostic::CliPathStatus => {
                let host = app
                    .try_state::<AppState>()
                    .and_then(|state| state.try_host());
                match host {
                    Some(host) => read_cli_path_status(&host)
                        .await
                        .map(cli_path_status)
                        .unwrap_or_else(|_| kind("unavailable")),
                    None => kind("unavailable"),
                }
            }
            DesktopDiagnostic::UpdateStatus => app
                .try_state::<AppUpdates<R>>()
                .map(|updates| update_status(updates.status()))
                .unwrap_or_else(|| kind("unavailable")),
            DesktopDiagnostic::CheckUpdate => {
                if app.try_state::<AppUpdates<R>>().is_some() {
                    update_status(update::check(app).await)
                } else {
                    kind("unavailable")
                }
            }
        };
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use tauri::test::{mock_builder, mock_context, noop_assets};

    use super::*;
    use crate::update::{Failure, FailureCode};

    #[test]
    fn chat_statuses_exclude_paths_shell_lines_and_error_details() {
        for (status, expected) in [
            (CliPathStatus::Off, "off"),
            (
                CliPathStatus::On {
                    shell_line: Some("local shell configuration".into()),
                },
                "on",
            ),
            (
                CliPathStatus::Stale {
                    target: "local target".into(),
                },
                "stale",
            ),
            (
                CliPathStatus::Blocked {
                    path: "local path".into(),
                },
                "blocked",
            ),
        ] {
            assert_eq!(cli_path_status(status), json!({"kind": expected}));
        }
        assert_eq!(
            update_status(UpdateStatus::UpToDate {
                checked_at: chrono::Utc::now(),
            }),
            json!({"kind": "up_to_date"})
        );
        assert_eq!(
            update_status(UpdateStatus::Available {
                version: "1.2.3".into(),
            }),
            json!({"kind": "available", "version": "1.2.3"})
        );
        assert_eq!(
            update_status(UpdateStatus::Available {
                version: "invalid version details".into(),
            }),
            json!({"kind": "failed"})
        );
    }

    #[tokio::test]
    async fn diagnostic_provider_handles_missing_components_and_projects_cached_failure() {
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let provider = DesktopDiagnosticProvider::new(app.handle().clone());
        for diagnostic in [
            DesktopDiagnostic::AutostartStatus,
            DesktopDiagnostic::CliPathStatus,
            DesktopDiagnostic::UpdateStatus,
            DesktopDiagnostic::CheckUpdate,
        ] {
            assert_eq!(
                provider.run(diagnostic).await.unwrap(),
                json!({"kind": "unavailable"})
            );
        }
        update::manage(
            app.handle(),
            "1.2.3",
            Some(Failure::new(FailureCode::Other, "private updater detail")),
        );
        assert_eq!(
            provider.run(DesktopDiagnostic::UpdateStatus).await.unwrap(),
            json!({"kind": "failed"})
        );
    }
}
