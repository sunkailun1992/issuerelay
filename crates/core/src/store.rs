//! SQLite-backed run records.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, SecondsFormat, Utc};
use rusqlite::{Connection, OptionalExtension, Row, params, params_from_iter, types::Type};

use crate::config::{Binding, ExecutorKind};
use crate::model::{Issue, Job, JobState};

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS jobs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    issue_uuid TEXT NOT NULL,
    issue_identifier TEXT NOT NULL,
    title TEXT NOT NULL,
    binding_id TEXT,
    directory TEXT,
    executor TEXT,
    state TEXT NOT NULL,
    reason TEXT,
    priority INTEGER NOT NULL,
    issue_created_at TEXT NOT NULL,
    discovered_updated_at TEXT NOT NULL,
    verified_updated_at TEXT,
    created_at TEXT NOT NULL,
    started_at TEXT,
    finished_at TEXT,
    log_path TEXT,
    summary TEXT
);
CREATE INDEX IF NOT EXISTS jobs_by_issue ON jobs (issue_uuid, id);
CREATE INDEX IF NOT EXISTS jobs_by_state ON jobs (state);
";

const COLUMNS: &str = "id, issue_uuid, issue_identifier, title, binding_id, directory, executor, \
    state, reason, priority, issue_created_at, discovered_updated_at, verified_updated_at, \
    created_at, started_at, finished_at, log_path, summary";

pub struct NewJob<'a> {
    pub issue: &'a Issue,
    pub binding: Option<&'a Binding>,
    pub executor: Option<ExecutorKind>,
    pub state: JobState,
    pub reason: Option<String>,
}

pub struct Store {
    conn: Connection,
}

fn fmt(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Millis, true)
}

fn conversion(column: usize, message: String) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(column, Type::Text, message.into())
}

fn ts(column: usize, text: String) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&text)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| conversion(column, e.to_string()))
}

fn opt_ts(column: usize, text: Option<String>) -> rusqlite::Result<Option<DateTime<Utc>>> {
    text.map(|t| ts(column, t)).transpose()
}

fn row_to_job(row: &Row<'_>) -> rusqlite::Result<Job> {
    let state: String = row.get(7)?;
    let executor: Option<String> = row.get(6)?;
    let priority: i64 = row.get(9)?;
    Ok(Job {
        id: row.get(0)?,
        issue_uuid: row.get(1)?,
        issue_identifier: row.get(2)?,
        title: row.get(3)?,
        binding_id: row.get(4)?,
        directory: row.get::<_, Option<String>>(5)?.map(PathBuf::from),
        executor: executor.as_deref().and_then(ExecutorKind::parse),
        state: JobState::parse(&state)
            .ok_or_else(|| conversion(7, format!("unknown state {state}")))?,
        reason: row.get(8)?,
        priority: priority.clamp(0, 4) as u8,
        issue_created_at: ts(10, row.get(10)?)?,
        discovered_updated_at: ts(11, row.get(11)?)?,
        verified_updated_at: opt_ts(12, row.get(12)?)?,
        created_at: ts(13, row.get(13)?)?,
        started_at: opt_ts(14, row.get(14)?)?,
        finished_at: opt_ts(15, row.get(15)?)?,
        log_path: row.get::<_, Option<String>>(16)?.map(PathBuf::from),
        summary: row.get(17)?,
    })
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let conn =
            Connection::open(path).with_context(|| format!("cannot open {}", path.display()))?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init(conn)
    }

    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self> {
        conn.execute_batch(SCHEMA)?;
        Ok(Self { conn })
    }

    pub fn insert(&self, new: NewJob<'_>) -> Result<Job> {
        let issue = new.issue;
        self.conn.execute(
            "INSERT INTO jobs (issue_uuid, issue_identifier, title, binding_id, directory, executor, \
             state, reason, priority, issue_created_at, discovered_updated_at, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                issue.uuid,
                issue.identifier,
                issue.title,
                new.binding.map(|b| b.id.clone()),
                new.binding.map(|b| b.directory.to_string_lossy().into_owned()),
                new.executor.map(|e| e.as_str()),
                new.state.as_str(),
                new.reason,
                i64::from(issue.priority),
                fmt(issue.created_at),
                fmt(issue.updated_at),
                fmt(Utc::now()),
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.get(id)?.context("inserted job disappeared")
    }

    pub fn get(&self, id: i64) -> Result<Option<Job>> {
        let sql = format!("SELECT {COLUMNS} FROM jobs WHERE id = ?1");
        Ok(self
            .conn
            .query_row(&sql, params![id], row_to_job)
            .optional()?)
    }

    pub fn latest_for_issue(&self, uuid: &str) -> Result<Option<Job>> {
        let sql =
            format!("SELECT {COLUMNS} FROM jobs WHERE issue_uuid = ?1 ORDER BY id DESC LIMIT 1");
        Ok(self
            .conn
            .query_row(&sql, params![uuid], row_to_job)
            .optional()?)
    }

    pub fn list_by_state(&self, states: &[JobState]) -> Result<Vec<Job>> {
        if states.is_empty() {
            return Ok(Vec::new());
        }
        let placeholders = (1..=states.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!("SELECT {COLUMNS} FROM jobs WHERE state IN ({placeholders}) ORDER BY id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(
            params_from_iter(states.iter().map(|s| s.as_str())),
            row_to_job,
        )?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn list_recent(&self, limit: usize) -> Result<Vec<Job>> {
        let sql = format!("SELECT {COLUMNS} FROM jobs ORDER BY id DESC LIMIT ?1");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![limit as i64], row_to_job)?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn assign(
        &self,
        id: i64,
        state: JobState,
        binding: Option<&Binding>,
        executor: Option<ExecutorKind>,
        reason: Option<&str>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, binding_id = ?3, directory = ?4, executor = ?5, reason = ?6 WHERE id = ?1",
            params![
                id,
                state.as_str(),
                binding.map(|b| b.id.clone()),
                binding.map(|b| b.directory.to_string_lossy().into_owned()),
                executor.map(|e| e.as_str()),
                reason,
            ],
        )?;
        Ok(())
    }

    pub fn set_state(&self, id: i64, state: JobState, reason: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = ?3 WHERE id = ?1",
            params![id, state.as_str(), reason],
        )?;
        Ok(())
    }

    pub fn mark_started(&self, id: i64, log_path: &Path) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = NULL, started_at = ?3, finished_at = NULL, log_path = ?4 WHERE id = ?1",
            params![
                id,
                JobState::Running.as_str(),
                fmt(Utc::now()),
                log_path.to_string_lossy().into_owned(),
            ],
        )?;
        Ok(())
    }

    pub fn mark_finished(
        &self,
        id: i64,
        state: JobState,
        reason: Option<&str>,
        summary: Option<&str>,
        verified_updated_at: Option<DateTime<Utc>>,
    ) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = ?3, summary = ?4, verified_updated_at = ?5, finished_at = ?6 WHERE id = ?1",
            params![id, state.as_str(), reason, summary, verified_updated_at.map(fmt), fmt(Utc::now())],
        )?;
        Ok(())
    }

    pub fn requeue(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = NULL, summary = NULL, started_at = NULL, finished_at = NULL WHERE id = ?1",
            params![id, JobState::Queued.as_str()],
        )?;
        Ok(())
    }

    /// Runs left in `running` belong to a previous process that stopped; they are not retried.
    pub fn recover_interrupted(&self) -> Result<usize> {
        Ok(self.conn.execute(
            "UPDATE jobs SET state = ?2, reason = 'interrupted: IssueRelay stopped while the run was in progress', finished_at = ?1 WHERE state = ?3",
            params![fmt(Utc::now()), JobState::Failed.as_str(), JobState::Running.as_str()],
        )?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Binding, ExecutorKind};
    use crate::model::{Issue, JobState};
    use chrono::{Duration, TimeZone, Utc};
    use std::path::{Path, PathBuf};

    fn issue(uuid: &str) -> Issue {
        let t = Utc.with_ymd_and_hms(2026, 10, 1, 8, 0, 0).unwrap();
        Issue {
            uuid: uuid.into(),
            identifier: format!("DEMO-{uuid}"),
            title: "Demo 需求".into(),
            url: "https://linear.app/demo/issue/DEMO-1".into(),
            state: "Todo".into(),
            labels: vec!["ai-ready".into()],
            project_id: Some("proj-1".into()),
            project_name: Some("Demo Project".into()),
            assignee_id: Some("me".into()),
            parent_identifier: None,
            priority: 2,
            created_at: t,
            updated_at: t + Duration::milliseconds(321),
        }
    }

    fn binding() -> Binding {
        Binding {
            id: "proj-1".into(),
            project_id: "proj-1".into(),
            project_name: "Demo Project".into(),
            directory: PathBuf::from("demo dir"),
            executor: None,
            permission: None,
            model: None,
            prompt_override: None,
        }
    }

    fn queued(store: &Store, uuid: &str) -> Job {
        let i = issue(uuid);
        let b = binding();
        store
            .insert(NewJob {
                issue: &i,
                binding: Some(&b),
                executor: Some(ExecutorKind::Claude),
                state: JobState::Queued,
                reason: None,
            })
            .unwrap()
    }

    #[test]
    fn insert_and_get_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        let loaded = store.get(job.id).unwrap().unwrap();
        assert_eq!(loaded, job);
        assert_eq!(loaded.issue_identifier, "DEMO-1");
        assert_eq!(loaded.directory, Some(PathBuf::from("demo dir")));
        assert_eq!(loaded.executor, Some(ExecutorKind::Claude));
        assert_eq!(loaded.priority, 2);
        assert_eq!(loaded.discovered_updated_at, issue("1").updated_at);
        assert!(store.get(999).unwrap().is_none());
    }

    #[test]
    fn latest_for_issue_returns_newest() {
        let store = Store::open_in_memory().unwrap();
        let first = queued(&store, "1");
        store
            .set_state(first.id, JobState::Failed, Some("boom"))
            .unwrap();
        let second = queued(&store, "1");
        queued(&store, "2");
        assert_eq!(store.latest_for_issue("1").unwrap().unwrap().id, second.id);
        assert!(store.latest_for_issue("3").unwrap().is_none());
    }

    #[test]
    fn list_by_state_filters() {
        let store = Store::open_in_memory().unwrap();
        let a = queued(&store, "1");
        let b = queued(&store, "2");
        store
            .set_state(b.id, JobState::WaitingBinding, Some("unbound"))
            .unwrap();
        let only_queued = store.list_by_state(&[JobState::Queued]).unwrap();
        assert_eq!(
            only_queued.iter().map(|j| j.id).collect::<Vec<_>>(),
            vec![a.id]
        );
        assert_eq!(
            store
                .list_by_state(&[JobState::Queued, JobState::WaitingBinding])
                .unwrap()
                .len(),
            2
        );
        assert!(store.list_by_state(&[]).unwrap().is_empty());
    }

    #[test]
    fn lifecycle_updates() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        store
            .mark_started(job.id, Path::new("logs/1.jsonl"))
            .unwrap();
        let running = store.get(job.id).unwrap().unwrap();
        assert_eq!(running.state, JobState::Running);
        assert!(running.started_at.is_some());
        assert_eq!(running.log_path, Some(PathBuf::from("logs/1.jsonl")));
        let verified = Utc.with_ymd_and_hms(2026, 10, 1, 9, 0, 0).unwrap();
        store
            .mark_finished(
                job.id,
                JobState::AwaitingAcceptance,
                None,
                Some("pushed to test"),
                Some(verified),
            )
            .unwrap();
        let done = store.get(job.id).unwrap().unwrap();
        assert_eq!(done.state, JobState::AwaitingAcceptance);
        assert_eq!(done.summary.as_deref(), Some("pushed to test"));
        assert_eq!(done.verified_updated_at, Some(verified));
        assert!(done.finished_at.is_some());
        store.requeue(job.id).unwrap();
        let again = store.get(job.id).unwrap().unwrap();
        assert_eq!(again.state, JobState::Queued);
        assert!(
            again.started_at.is_none() && again.finished_at.is_none() && again.summary.is_none()
        );
    }

    #[test]
    fn assign_changes_binding_and_state() {
        let store = Store::open_in_memory().unwrap();
        let i = issue("1");
        let waiting = store
            .insert(NewJob {
                issue: &i,
                binding: None,
                executor: None,
                state: JobState::WaitingBinding,
                reason: Some("unbound".into()),
            })
            .unwrap();
        assert_eq!(waiting.directory, None);
        let b = binding();
        store
            .assign(
                waiting.id,
                JobState::Queued,
                Some(&b),
                Some(ExecutorKind::Codex),
                None,
            )
            .unwrap();
        let now = store.get(waiting.id).unwrap().unwrap();
        assert_eq!(now.state, JobState::Queued);
        assert_eq!(now.binding_id.as_deref(), Some("proj-1"));
        assert_eq!(now.executor, Some(ExecutorKind::Codex));
        assert_eq!(now.reason, None);
    }

    #[test]
    fn recover_marks_running_as_failed() {
        let store = Store::open_in_memory().unwrap();
        let job = queued(&store, "1");
        store.mark_started(job.id, Path::new("x.jsonl")).unwrap();
        queued(&store, "2");
        assert_eq!(store.recover_interrupted().unwrap(), 1);
        let recovered = store.get(job.id).unwrap().unwrap();
        assert_eq!(recovered.state, JobState::Failed);
        assert!(recovered.reason.unwrap().contains("interrupted"));
        assert_eq!(store.recover_interrupted().unwrap(), 0);
    }

    #[test]
    fn file_store_persists_across_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("state.db");
        let id = {
            let store = Store::open(&path).unwrap();
            queued(&store, "1").id
        };
        let store = Store::open(&path).unwrap();
        assert_eq!(store.get(id).unwrap().unwrap().state, JobState::Queued);
        assert_eq!(store.list_recent(10).unwrap().len(), 1);
    }
}
