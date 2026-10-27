#![allow(dead_code)]
pub mod engine_kit;
pub mod fake_linear;
pub mod fake_server;
#[cfg(target_os = "macos")]
pub mod orphan;

use chrono::{TimeZone, Utc};
use issuerelay_core::model::Issue;

pub fn issue(uuid: &str, project: &str) -> Issue {
    let t = Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap();
    Issue {
        uuid: uuid.into(),
        identifier: format!("DEMO-{uuid}"),
        title: format!("Demo task {uuid}"),
        url: format!("https://linear.app/demo/issue/DEMO-{uuid}"),
        state: "Todo".into(),
        labels: vec!["ai-ready".into()],
        project_id: Some(project.into()),
        project_name: Some("Demo Project".into()),
        assignee_id: Some("user-1".into()),
        parent_identifier: None,
        priority: 0,
        created_at: t,
        updated_at: t,
        state_type: None,
        blocked_by: None,
    }
}
