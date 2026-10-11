mod common;

use common::fake_linear::FakeLinear;
use issuerelay_core::{config::TriggerNames, doctor::check_workspace};

#[tokio::test]
async fn workspace_with_all_names_is_ok() {
    let report = check_workspace(&FakeLinear::new("user-1"), &TriggerNames::default())
        .await
        .unwrap();
    assert!(report.ok(), "{report:?}");
}

#[tokio::test]
async fn missing_status_and_label_are_reported() {
    let linear = FakeLinear::new("user-1");
    linear.set_statuses(&["Todo", "In Progress", "Done"]);
    let names = TriggerNames {
        label: "robot".into(),
        ..TriggerNames::default()
    };
    let report = check_workspace(&linear, &names).await.unwrap();
    assert!(!report.label_ok);
    assert_eq!(report.missing_states, ["Demo: In Testing"]);
}
