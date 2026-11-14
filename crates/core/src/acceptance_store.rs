//! SQLite relationship and approval records for independent acceptance runs.

use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::acceptance_delivery::DeliveryFingerprint;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AcceptanceRecord {
    pub source_job_id: i64,
    pub verification_job_id: i64,
    pub repair_job_id: Option<i64>,
    pub approval: Value,
    pub fingerprint: DeliveryFingerprint,
    pub recommendation: Option<Value>,
    pub returned: Option<Value>,
}

const COLUMNS: &str = "source_job_id, verification_job_id, repair_job_id, approval,
    fingerprint, recommendation, returned";

pub(crate) fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS acceptance_runs (
            source_job_id INTEGER NOT NULL REFERENCES jobs(id),
            verification_job_id INTEGER PRIMARY KEY REFERENCES jobs(id),
            repair_job_id INTEGER UNIQUE REFERENCES jobs(id),
            approval TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            recommendation TEXT,
            returned TEXT,
            CHECK (source_job_id != verification_job_id),
            CHECK (repair_job_id IS NULL OR
                (repair_job_id != source_job_id AND repair_job_id != verification_job_id))
         );
         CREATE INDEX IF NOT EXISTS acceptance_runs_by_source
             ON acceptance_runs (source_job_id, verification_job_id);
         CREATE TABLE IF NOT EXISTS acceptance_repairs (
             job_id INTEGER PRIMARY KEY REFERENCES jobs(id),
             verification_job_id INTEGER NOT NULL REFERENCES acceptance_runs(verification_job_id)
         );",
    )?;
    Ok(())
}

fn json_optional(value: Option<&impl Serialize>) -> Result<Option<String>> {
    Ok(value.map(serde_json::to_string).transpose()?)
}

fn parse_json(index: usize, value: &str) -> rusqlite::Result<Value> {
    serde_json::from_str(value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn row_to_record(row: &Row<'_>) -> rusqlite::Result<AcceptanceRecord> {
    Ok(AcceptanceRecord {
        source_job_id: row.get(0)?,
        verification_job_id: row.get(1)?,
        repair_job_id: row.get(2)?,
        approval: parse_json(3, &row.get::<_, String>(3)?)?,
        fingerprint: DeliveryFingerprint(row.get(4)?),
        recommendation: row
            .get::<_, Option<String>>(5)?
            .as_deref()
            .map(|value| parse_json(5, value))
            .transpose()?,
        returned: row
            .get::<_, Option<String>>(6)?
            .as_deref()
            .map(|value| parse_json(6, value))
            .transpose()?,
    })
}

/// Create the association in the same transaction as the queued verification job.
pub fn create(conn: &Connection, record: &AcceptanceRecord) -> Result<()> {
    let rows = conn.execute(
        "INSERT INTO acceptance_runs (
            source_job_id, verification_job_id, repair_job_id, approval,
            fingerprint, recommendation, returned
         )
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7 FROM jobs source JOIN jobs verification
             ON verification.id = ?2 AND verification.issue_uuid = source.issue_uuid
         WHERE source.id = ?1
             AND NOT EXISTS (SELECT 1 FROM acceptance_runs WHERE verification_job_id = ?1)
             AND (?3 IS NULL OR EXISTS (
                 SELECT 1 FROM jobs repair WHERE repair.id = ?3 AND repair.issue_uuid = source.issue_uuid
             ))",
        params![
            record.source_job_id,
            record.verification_job_id,
            record.repair_job_id,
            serde_json::to_string(&record.approval)?,
            record.fingerprint.0,
            json_optional(record.recommendation.as_ref())?,
            json_optional(record.returned.as_ref())?,
        ],
    )?;
    ensure!(
        rows == 1,
        "acceptance source and child runs do not belong to the same issue"
    );
    Ok(())
}

/// Persist the recommendation, decision or repair link without changing the approved version.
pub fn update(conn: &Connection, record: &AcceptanceRecord) -> Result<()> {
    let rows = conn.execute(
        "UPDATE acceptance_runs SET repair_job_id = ?3, recommendation = ?6, returned = ?7
         WHERE source_job_id = ?1 AND verification_job_id = ?2 AND approval = ?4 AND fingerprint = ?5
             AND (?3 IS NULL OR EXISTS (
                 SELECT 1 FROM jobs repair JOIN jobs source ON source.issue_uuid = repair.issue_uuid
                 WHERE repair.id = ?3 AND source.id = ?1
             ))",
        params![
            record.source_job_id,
            record.verification_job_id,
            record.repair_job_id,
            serde_json::to_string(&record.approval)?,
            record.fingerprint.0,
            json_optional(record.recommendation.as_ref())?,
            json_optional(record.returned.as_ref())?,
        ],
    )?;
    ensure!(
        rows == 1,
        "acceptance record or its approved delivery no longer matches"
    );
    Ok(())
}

pub fn get(conn: &Connection, verification_job_id: i64) -> Result<Option<AcceptanceRecord>> {
    let sql = format!("SELECT {COLUMNS} FROM acceptance_runs WHERE verification_job_id = ?1");
    Ok(conn
        .query_row(&sql, [verification_job_id], row_to_record)
        .optional()?)
}

pub fn latest_for_source(
    conn: &Connection,
    source_job_id: i64,
) -> Result<Option<AcceptanceRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM acceptance_runs WHERE source_job_id = ?1
         ORDER BY verification_job_id DESC LIMIT 1"
    );
    Ok(conn
        .query_row(&sql, [source_job_id], row_to_record)
        .optional()?)
}

pub fn latest_by_repair(conn: &Connection, repair_job_id: i64) -> Result<Option<AcceptanceRecord>> {
    let sql = format!(
        "SELECT {COLUMNS} FROM acceptance_runs WHERE repair_job_id = ?1
         OR verification_job_id IN (
             SELECT verification_job_id FROM acceptance_repairs WHERE job_id = ?1
         ) ORDER BY verification_job_id DESC LIMIT 1"
    );
    Ok(conn
        .query_row(&sql, [repair_job_id], row_to_record)
        .optional()?)
}

pub(crate) fn link_repair(conn: &Connection, job_id: i64, verification_job_id: i64) -> Result<()> {
    let rows = conn.execute(
        "INSERT INTO acceptance_repairs (job_id, verification_job_id)
         SELECT repair.id, acceptance.verification_job_id
         FROM jobs repair JOIN acceptance_runs acceptance ON acceptance.verification_job_id = ?2
         JOIN jobs source ON source.id = acceptance.source_job_id
         WHERE repair.id = ?1 AND repair.issue_uuid = source.issue_uuid
         AND repair.id > acceptance.verification_job_id
         ON CONFLICT (job_id) DO UPDATE SET verification_job_id = excluded.verification_job_id",
        params![job_id, verification_job_id],
    )?;
    ensure!(
        rows == 1,
        "repair does not belong to the failed verification"
    );
    Ok(())
}

pub(crate) fn inherit_repair(conn: &Connection, child: i64, parent: i64) -> Result<()> {
    if let Some(record) = latest_by_repair(conn, parent)? {
        link_repair(conn, child, record.verification_job_id)?;
    }
    Ok(())
}
