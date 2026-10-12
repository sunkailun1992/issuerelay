//! In-memory LinearApi used by doctor and engine tests.
use std::{
    collections::HashSet,
    sync::{Arc, Mutex},
};

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
    /// The message the next calls fail with, and how many calls are left to fail.
    fail_next: Option<(String, usize)>,
    set_state_calls: Vec<(String, String)>,
    statuses: Vec<String>,
    /// Uuids whose `issue()` reads fail until `clear_issue_failures`.
    failing_issue_reads: HashSet<String>,
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

    /// The next call, whatever it is, fails with `message`.
    pub fn fail_next(&self, message: &str) {
        self.fail_calls(message, 1);
    }

    /// The next `calls` calls, whatever they are, fail with `message`.
    pub fn fail_calls(&self, message: &str, calls: usize) {
        self.state.lock().unwrap().fail_next = (calls > 0).then(|| (message.into(), calls));
    }

    /// Makes every `issue(uuid)` read fail until `clear_issue_failures`.
    pub fn fail_issue_reads(&self, uuid: &str) {
        self.state
            .lock()
            .unwrap()
            .failing_issue_reads
            .insert(uuid.into());
    }

    pub fn clear_issue_failures(&self) {
        self.state.lock().unwrap().failing_issue_reads.clear();
    }

    pub fn set_statuses(&self, statuses: &[&str]) {
        self.state.lock().unwrap().statuses = statuses.iter().map(|s| s.to_string()).collect();
    }

    pub fn set_state_calls(&self) -> Vec<(String, String)> {
        self.state.lock().unwrap().set_state_calls.clone()
    }

    fn check_failure(&self) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        match state.fail_next.take() {
            Some((message, left)) => {
                if left > 1 {
                    state.fail_next = Some((message.clone(), left - 1));
                }
                Err(anyhow!(message))
            }
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

    async fn labeled_issues(&self, label: &str) -> Result<Vec<Issue>> {
        self.check_failure()?;
        let s = self.state.lock().unwrap();
        let mut issues: Vec<Issue> = s
            .issues
            .iter()
            .filter(|i| {
                i.labels.iter().any(|l| same_name(l, label))
                    && i.assignee_id.as_deref() == Some(s.me.as_str())
            })
            .cloned()
            .collect();
        issues.sort_by_key(|i| std::cmp::Reverse(i.updated_at));
        issues.truncate(50);
        Ok(issues)
    }

    async fn issue(&self, id: &str) -> Result<Option<Issue>> {
        self.check_failure()?;
        let s = self.state.lock().unwrap();
        let found = s.issues.iter().find(|i| i.uuid == id || i.identifier == id);
        // A read by identifier fails like a read by uuid.
        let uuid = found.map_or(id, |i| i.uuid.as_str());
        if s.failing_issue_reads.contains(uuid) {
            return Err(anyhow!("Linear is unreachable"));
        }
        Ok(found.cloned())
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
