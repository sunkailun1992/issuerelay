//! In-memory LinearApi used by doctor and engine tests.
use std::sync::{Arc, Mutex};

use anyhow::{Result, anyhow};
use chrono::Duration;
use issuerelay_core::{
    linear::{LinearApi, Project, Team, Viewer},
    model::Issue,
    trigger::same_name,
};

#[derive(Default)]
struct State {
    me: String,
    issues: Vec<Issue>,
    fail_next: Option<String>,
    set_state_calls: Vec<(String, String)>,
    statuses: Vec<String>,
}

#[derive(Clone, Default)]
pub struct FakeLinear {
    state: Arc<Mutex<State>>,
}

impl FakeLinear {
    pub fn new(me: &str) -> Self {
        let fake = Self::default();
        {
            let mut s = fake.state.lock().unwrap();
            s.me = me.into();
            s.statuses = ["Todo", "In Progress", "In Testing", "Done"]
                .map(String::from)
                .to_vec();
        }
        fake
    }

    pub fn upsert(&self, issue: Issue) {
        let mut s = self.state.lock().unwrap();
        s.issues.retain(|i| i.uuid != issue.uuid);
        s.issues.push(issue);
    }

    /// Applies `change` and moves `updated_at` forward by one second, like Linear does.
    pub fn update(&self, uuid: &str, change: impl FnOnce(&mut Issue)) {
        let mut s = self.state.lock().unwrap();
        let issue = s
            .issues
            .iter_mut()
            .find(|i| i.uuid == uuid || i.identifier == uuid)
            .expect("unknown issue");
        change(issue);
        issue.updated_at += Duration::seconds(1);
    }

    pub fn get(&self, uuid: &str) -> Issue {
        self.state
            .lock()
            .unwrap()
            .issues
            .iter()
            .find(|i| i.uuid == uuid)
            .cloned()
            .expect("unknown issue")
    }

    pub fn fail_next(&self, message: &str) {
        self.state.lock().unwrap().fail_next = Some(message.into());
    }

    pub fn set_statuses(&self, statuses: &[&str]) {
        self.state.lock().unwrap().statuses = statuses.iter().map(|s| s.to_string()).collect();
    }

    pub fn set_state_calls(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().set_state_calls.clone()
    }

    fn check_failure(&self) -> Result<()> {
        match self.state.lock().unwrap().fail_next.take() {
            Some(message) => Err(anyhow!(message)),
            None => Ok(()),
        }
    }
}

#[async_trait::async_trait]
impl LinearApi for FakeLinear {
    async fn viewer(&self) -> Result<Viewer> {
        self.check_failure()?;
        Ok(Viewer {
            id: self.state.lock().unwrap().me.clone(),
            name: "Jane Doe".into(),
        })
    }

    async fn candidate_issues(&self, label: &str, state: &str) -> Result<Vec<Issue>> {
        self.check_failure()?;
        let s = self.state.lock().unwrap();
        Ok(s.issues
            .iter()
            .filter(|i| i.assignee_id.as_deref() == Some(s.me.as_str()))
            .filter(|i| i.labels.iter().any(|l| same_name(l, label)) && same_name(&i.state, state))
            .cloned()
            .collect())
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        self.check_failure()?;
        Ok(self
            .state
            .lock()
            .unwrap()
            .issues
            .iter()
            .find(|i| i.uuid == id || i.identifier == id)
            .cloned())
    }

    async fn set_state(&self, id: &str, state: &str) -> Result<()> {
        self.check_failure()?;
        self.state
            .lock()
            .unwrap()
            .set_state_calls
            .push((id.into(), state.into()));
        self.update(id, |i| i.state = state.into());
        Ok(())
    }

    async fn projects(&self) -> Result<Vec<Project>> {
        self.check_failure()?;
        Ok(vec![Project {
            id: "proj-1".into(),
            name: "Demo Project".into(),
        }])
    }

    async fn labels(&self) -> Result<Vec<String>> {
        self.check_failure()?;
        Ok(vec!["ai-ready".into(), "Bug".into()])
    }

    async fn teams(&self) -> Result<Vec<Team>> {
        self.check_failure()?;
        Ok(vec![Team {
            id: "team-1".into(),
            name: "Demo".into(),
        }])
    }

    async fn statuses(&self, _team_id: &str) -> Result<Vec<String>> {
        self.check_failure()?;
        Ok(self.state.lock().unwrap().statuses.clone())
    }
}
