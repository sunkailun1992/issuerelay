//! SQLite persistence for workflow context, run evidence and local handoff ownership.
//! The caller holds the engine's store guard across decisions involving both workflow and jobs.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, params};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::config::ExecutorKind;
use crate::workflow::{ArchiveReceipt, IssueContext, RunMemory};

const MAX_PAYLOAD_BYTES: usize = 256 * 1024;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS workflow_context (
    issue_key TEXT PRIMARY KEY,
    revision INTEGER NOT NULL CHECK (revision > 0),
    payload TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_memory (
    job_id INTEGER PRIMARY KEY CHECK (job_id > 0),
    payload TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_archive (
    job_id INTEGER PRIMARY KEY CHECK (job_id > 0),
    payload TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS workflow_claim (
    job_id INTEGER PRIMARY KEY CHECK (job_id > 0),
    issue_key TEXT NOT NULL UNIQUE,
    directory TEXT NOT NULL UNIQUE,
    executor TEXT NOT NULL CHECK (executor IN ('claude', 'codex')),
    claim_id TEXT NOT NULL UNIQUE
);
";

/// The current holder of one issue and its canonical directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Claim {
    pub job_id: i64,
    pub issue_key: String,
    pub directory: PathBuf,
    pub executor: ExecutorKind,
    pub claim_id: String,
}

/// Creates the additive tables under the store migration's transaction.
pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;
    Ok(())
}

fn local_issue_key(uuid: &str) -> Result<String> {
    ensure!(
        !uuid.is_empty()
            && uuid.len() <= 64
            && uuid.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
        "invalid Linear issue identity"
    );
    Ok(uuid.to_ascii_lowercase())
}

pub fn read_context(conn: &Connection, key: &str) -> Result<Option<IssueContext>> {
    let key = local_issue_key(key)?;
    let row: Option<(i64, String)> = conn
        .query_row(
            "SELECT revision, payload FROM workflow_context WHERE issue_key = ?1",
            [key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    row.map(|(revision, payload)| {
        let context: IssueContext = decode(&payload)?;
        ensure!(
            i64::try_from(context.revision) == Ok(revision),
            "invalid stored context revision"
        );
        context.validate()?;
        Ok(context)
    })
    .transpose()
}

/// Inserts at expected revision zero or advances a matching revision in a single statement.
pub fn write_context(
    conn: &Connection,
    key: &str,
    expected_revision: u64,
    context: &IssueContext,
) -> Result<IssueContext> {
    let key = local_issue_key(key)?;
    let expected = i64::try_from(expected_revision).context("invalid context revision")?;
    let revision = expected
        .checked_add(1)
        .context("context revision is exhausted")?;
    let context = IssueContext {
        revision: u64::try_from(revision).context("invalid next context revision")?,
        ..context.clone()
    };
    context.validate()?;
    let payload = encode(&context)?;
    let changed = if expected == 0 {
        conn.execute(
            "INSERT INTO workflow_context (issue_key, revision, payload)
             VALUES (?1, ?2, ?3) ON CONFLICT(issue_key) DO NOTHING",
            params![key, revision, payload],
        )?
    } else {
        conn.execute(
            "UPDATE workflow_context SET revision = ?2, payload = ?3
             WHERE issue_key = ?1 AND revision = ?4",
            params![key, revision, payload, expected],
        )?
    };
    ensure!(
        changed == 1,
        "issue context revision changed; read it and try again"
    );
    Ok(context)
}

pub fn read_memory(conn: &Connection, job_id: i64) -> Result<Option<RunMemory>> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM workflow_memory WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            let memory: RunMemory = decode(&payload)?;
            memory.validate()?;
            Ok(memory)
        })
        .transpose()
}

pub fn write_memory(conn: &Connection, job_id: i64, memory: &RunMemory) -> Result<()> {
    memory.validate()?;
    conn.execute(
        "INSERT INTO workflow_memory (job_id, payload) VALUES (?1, ?2)
         ON CONFLICT(job_id) DO UPDATE SET payload = excluded.payload",
        params![job_id, encode(memory)?],
    )?;
    Ok(())
}

/// Claims both the issue and directory for this executor. The caller supplies a canonical path.
pub fn claim(
    conn: &Connection,
    job_id: i64,
    key: &str,
    directory: &Path,
    executor: ExecutorKind,
) -> Result<Claim> {
    let key = local_issue_key(key)?;
    ensure!(
        directory.is_absolute(),
        "handoff directory must be absolute"
    );
    let path = directory
        .to_str()
        .context("handoff directory is not valid Unicode")?;
    ensure!(
        path.len() <= 128 * 1024 && !path.contains('\0'),
        "invalid handoff directory"
    );
    let tx = conn.unchecked_transaction()?;
    let id: Option<String> = tx
        .query_row(
            "INSERT INTO workflow_claim (job_id, issue_key, directory, executor, claim_id)
         VALUES (?1, ?2, ?3, ?4, lower(hex(randomblob(16))))
         ON CONFLICT DO NOTHING RETURNING claim_id",
            params![job_id, key, path, executor.as_str()],
            |row| row.get(0),
        )
        .optional()?;
    // Two sessions of the same executor must not both receive a successful claim.
    let claim_id = id.context("the run, issue or directory is already claimed")?;
    clear_archive(&tx, job_id)?;
    tx.commit()?;
    Ok(Claim {
        job_id,
        issue_key: key,
        directory: directory.into(),
        executor,
        claim_id,
    })
}

pub fn read_claim(conn: &Connection, job_id: i64) -> Result<Option<Claim>> {
    let row: Option<(String, String, String, String)> = conn
        .query_row(
            "SELECT issue_key, directory, executor, claim_id FROM workflow_claim WHERE job_id = ?1",
            [job_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?;
    row.map(|(issue_key, directory, executor, claim_id)| {
        Ok(Claim {
            job_id,
            issue_key,
            directory: directory.into(),
            executor: ExecutorKind::parse(&executor).context("invalid stored claim executor")?,
            claim_id,
        })
    })
    .transpose()
}

pub fn release(conn: &Connection, job_id: i64, claim_id: &str) -> Result<bool> {
    ensure!(
        claim_id.len() == 32 && claim_id.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid claim ID"
    );
    let changed = conn.execute(
        "DELETE FROM workflow_claim WHERE job_id = ?1 AND claim_id = ?2",
        params![job_id, claim_id],
    )?;
    Ok(changed == 1)
}

pub fn claimed_directories(conn: &Connection) -> Result<Vec<PathBuf>> {
    let mut statement = conn.prepare("SELECT directory FROM workflow_claim ORDER BY job_id")?;
    let rows = statement.query_map([], |row| row.get::<_, String>(0).map(PathBuf::from))?;
    Ok(rows.collect::<rusqlite::Result<_>>()?)
}

pub fn read_archive(conn: &Connection, job_id: i64) -> Result<Option<ArchiveReceipt>> {
    let payload: Option<String> = conn
        .query_row(
            "SELECT payload FROM workflow_archive WHERE job_id = ?1",
            [job_id],
            |row| row.get(0),
        )
        .optional()?;
    payload
        .map(|payload| {
            let receipt: ArchiveReceipt = decode(&payload)?;
            receipt.validate()?;
            Ok(receipt)
        })
        .transpose()
}

pub fn write_archive(conn: &Connection, job_id: i64, receipt: &ArchiveReceipt) -> Result<()> {
    receipt.validate()?;
    conn.execute(
        "INSERT INTO workflow_archive (job_id, payload) VALUES (?1, ?2)
         ON CONFLICT(job_id) DO UPDATE SET payload = excluded.payload",
        params![job_id, encode(receipt)?],
    )?;
    Ok(())
}

/// A manual rerun reuses the job ID, but its Wiki receipt belongs to the earlier attempt.
pub fn clear_archive(conn: &Connection, job_id: i64) -> Result<()> {
    conn.execute("DELETE FROM workflow_archive WHERE job_id = ?1", [job_id])?;
    Ok(())
}

fn encode(payload: &impl Serialize) -> Result<String> {
    let text = serde_json::to_string(payload).context("cannot serialize workflow metadata")?;
    ensure!(
        text.len() <= MAX_PAYLOAD_BYTES,
        "workflow metadata is too large"
    );
    Ok(text)
}

fn decode<T: DeserializeOwned>(payload: &str) -> Result<T> {
    ensure!(
        payload.len() <= MAX_PAYLOAD_BYTES,
        "workflow metadata is too large"
    );
    serde_json::from_str(payload).map_err(|_| anyhow::anyhow!("invalid stored workflow metadata"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::{Checkpoint, GitFacts, Interruption};
    use chrono::Utc;

    #[test]
    fn context_cas_rejects_stale_writers_and_survives_reopen() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workflow.sqlite3");
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        let original = IssueContext {
            request: "Fix the demo".into(),
            scope: "Only the demo fixture".into(),
            origin_executor: Some(ExecutorKind::Codex),
            ..IssueContext::default()
        };
        let first = write_context(&conn, "demo-1", 0, &original).unwrap();
        assert_eq!(first.revision, 1);
        let changed = IssueContext {
            request: "Keep the public behavior".into(),
            ..first
        };
        assert_eq!(
            write_context(&conn, "DEMO-1", 1, &changed)
                .unwrap()
                .revision,
            2
        );
        assert!(write_context(&conn, "demo-1", 1, &original).is_err());
        drop(conn);
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        let persisted = read_context(&conn, "DeMo-1").unwrap().unwrap();
        assert_eq!(persisted.revision, 2);
        assert_eq!(persisted.request, "Keep the public behavior");
        assert_eq!(persisted.origin_executor, Some(ExecutorKind::Codex));
    }

    #[test]
    fn claim_is_exclusive_by_issue_and_directory_across_connections() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().canonicalize().unwrap();
        let other_dir = tempfile::tempdir().unwrap();
        let other_directory = other_dir.path().canonicalize().unwrap();
        let path = root.path().join("workflow.sqlite3");
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        let owner = claim(&conn, 1, "demo-1", &directory, ExecutorKind::Codex).unwrap();
        assert_eq!(owner.issue_key, "demo-1");
        assert!(claim(&conn, 1, "DEMO-1", &directory, ExecutorKind::Codex).is_err());
        let other = Connection::open(&path).unwrap();
        assert!(claim(&other, 2, "DEMO-1", &other_directory, ExecutorKind::Claude).is_err());
        assert!(claim(&other, 2, "DEMO-2", &directory, ExecutorKind::Claude).is_err());
        assert!(!release(&other, 1, &"0".repeat(32)).unwrap());
        assert_eq!(read_claim(&other, 1).unwrap(), Some(owner.clone()));
        assert_eq!(
            claimed_directories(&other).unwrap(),
            vec![directory.clone()]
        );
        assert!(release(&other, 1, &owner.claim_id).unwrap());
        let renewed = claim(&other, 1, "DEMO-1", &directory, ExecutorKind::Codex).unwrap();
        assert_ne!(owner.claim_id, renewed.claim_id);
        assert!(!release(&conn, 1, &owner.claim_id).unwrap());
        assert_eq!(read_claim(&conn, 1).unwrap(), Some(renewed.clone()));
        assert!(release(&other, 1, &renewed.claim_id).unwrap());
        assert!(claim(&other, 2, "DEMO-2", &directory, ExecutorKind::Claude).is_ok());
    }

    #[test]
    fn archive_receipt_survives_reopen_and_can_be_cleared_for_a_new_attempt() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("workflow.sqlite3");
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        let receipt = ArchiveReceipt {
            wiki_root: root.path().canonicalize().unwrap(),
            page: "projects/demo.md".into(),
            commit: Some("a".repeat(40)),
            recorded_at: Utc::now(),
        };
        write_archive(&conn, 7, &receipt).unwrap();
        assert!(read_archive(&conn, 8).unwrap().is_none());
        drop(conn);
        let conn = Connection::open(&path).unwrap();
        ensure_schema(&conn).unwrap();
        assert_eq!(read_archive(&conn, 7).unwrap(), Some(receipt.clone()));
        let invalid = ArchiveReceipt {
            page: "../demo.md".into(),
            ..receipt.clone()
        };
        assert!(write_archive(&conn, 7, &invalid).is_err());
        assert_eq!(read_archive(&conn, 7).unwrap(), Some(receipt));
        clear_archive(&conn, 7).unwrap();
        assert!(read_archive(&conn, 7).unwrap().is_none());
    }

    #[test]
    fn checkpoint_and_quota_are_read_back_independently_of_run_logs() {
        let conn = Connection::open_in_memory().unwrap();
        ensure_schema(&conn).unwrap();
        let memory = RunMemory {
            context: None,
            milestones: vec![],
            decision: None,
            checkpoint: Some(Checkpoint {
                summary: "Implemented the demo".into(),
                done: vec!["Read the fixture".into()],
                next: vec!["Review the changes".into()],
                verification: vec![],
                blockers: vec![],
            }),
            interruption: Some(Interruption {
                code: "usage_limit_reached".into(),
                reset_at: None,
            }),
            git: Some(GitFacts {
                head: None,
                branch: Some("demo".into()),
                changed: vec![" M demo.rs".into()],
            }),
            updated_at: Utc::now(),
        };
        write_memory(&conn, 7, &memory).unwrap();
        assert_eq!(read_memory(&conn, 7).unwrap(), Some(memory));
        assert!(read_memory(&conn, 8).unwrap().is_none());
    }
}
