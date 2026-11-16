//! Host-owned durable staging. Only `work` is writable by the verifier; its ownership marker
//! is outside that sandbox root. An activated workspace never disappears merely on Drop.
use super::*;
use crate::acceptance_delivery::DeliveryFingerprint;
use chrono::{DateTime, Utc};

const ROOT: &str = "verification-work";
const WORK: &str = "work";
const MARKER: &str = "owner.json";
const MARKER_BYTES: u64 = 16 * 1024;
const MAX_WORKSPACES: usize = 4096;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WorkspaceIdentity {
    pub version: u32,
    pub verification_job_id: i64,
    pub source_job_id: i64,
    pub fingerprint: DeliveryFingerprint,
    pub attempt: String,
    pub started_at: DateTime<Utc>,
}

pub(crate) struct Workspace {
    container: PathBuf,
    work: PathBuf,
    prepared: Option<tempfile::TempDir>,
    identity: Option<WorkspaceIdentity>,
}

fn root(logs: &Path) -> Result<PathBuf> {
    ensure!(
        logs.is_absolute(),
        "verification logs directory must be absolute"
    );
    let data = canonical_directory(
        logs.parent()
            .context("verification logs have no data parent")?,
    )?;
    check_parent_chain(&data)?;
    let path = data.join(ROOT);
    match fs::symlink_metadata(&path) {
        Ok(_) => ensure!(
            Snapshot::read(&path)?.metadata.is_dir(),
            "invalid verification workspace root"
        ),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => private_directory(&path)?,
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    check_parent_chain(&path)?;
    Ok(path)
}

impl Workspace {
    pub(crate) fn prepare(logs: &Path, job_id: i64) -> Result<Self> {
        ensure!(job_id > 0, "invalid verification workspace job");
        let parent = root(logs)?;
        let temporary = tempfile::Builder::new()
            .prefix(&format!("{job_id}-"))
            .tempdir_in(&parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))?;
        }
        let container = canonical_directory(temporary.path())?;
        let work = container.join(WORK);
        private_directory(&work)?;
        Ok(Self {
            container,
            work,
            prepared: Some(temporary),
            identity: None,
        })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.work
    }

    /// Commit ownership before spawning. Even a failed marker write preserves this attempt for
    /// inspection; the recovery reader will refuse an incomplete or mismatched marker.
    pub(crate) fn activate(
        &mut self,
        job_id: i64,
        source_job_id: i64,
        fingerprint: DeliveryFingerprint,
        started_at: DateTime<Utc>,
    ) -> Result<()> {
        let attempt = component(
            self.container
                .file_name()
                .context("workspace has no name")?,
        )?
        .to_owned();
        ensure!(
            job_id > 0
                && source_job_id > 0
                && job_id != source_job_id
                && attempt.starts_with(&format!("{job_id}-")),
            "workspace ownership mismatch"
        );
        ensure!(
            !fingerprint.0.is_empty() && fingerprint.0.len() <= 4096,
            "invalid workspace fingerprint"
        );
        if let Some(temporary) = self.prepared.take() {
            let _ = temporary.keep();
        }
        let identity = WorkspaceIdentity {
            version: 1,
            verification_job_id: job_id,
            source_job_id,
            fingerprint,
            attempt,
            started_at,
        };
        let bytes = serde_json::to_vec(&identity)?;
        ensure!(
            bytes.len() as u64 <= MARKER_BYTES,
            "workspace marker is too large"
        );
        write_file(&self.container.join(MARKER), &bytes)?;
        sync_directory(&self.work)?;
        sync_directory(&self.container)?;
        sync_directory(self.container.parent().context("workspace root missing")?)?;
        self.identity = Some(identity);
        Ok(())
    }

    pub(crate) fn identity(&self) -> Result<&WorkspaceIdentity> {
        self.identity
            .as_ref()
            .context("verification workspace was not activated")
    }

    fn load(container: PathBuf) -> Result<Self> {
        ensure!(
            Snapshot::read(&container)?.metadata.is_dir(),
            "workspace is not a plain directory"
        );
        let marker = container.join(MARKER);
        let snapshot = Snapshot::read(&marker)?;
        let identity: WorkspaceIdentity =
            serde_json::from_slice(&read_file(&marker, &snapshot, MARKER_BYTES)?)?;
        let attempt = component(container.file_name().context("workspace has no name")?)?;
        ensure!(
            identity.version == 1
                && identity.verification_job_id > 0
                && identity.source_job_id > 0
                && identity.verification_job_id != identity.source_job_id
                && identity.attempt == attempt
                && attempt.starts_with(&format!("{}-", identity.verification_job_id))
                && !identity.fingerprint.0.is_empty()
                && identity.fingerprint.0.len() <= 4096,
            "workspace ownership marker does not match its directory"
        );
        let work = container.join(WORK);
        // A missing/replaced work directory is handled as a retention failure. Never turn a
        // path from the descriptor into a read request: WORK is a fixed host-owned component.
        Ok(Self {
            container,
            work,
            prepared: None,
            identity: Some(identity),
        })
    }

    pub(crate) fn discover(logs: &Path) -> Result<Vec<Self>> {
        let parent = root(logs)?;
        let mut result = Vec::new();
        for (count, entry) in fs::read_dir(&parent)?.enumerate() {
            ensure!(
                count < MAX_WORKSPACES,
                "too many verification workspaces to recover"
            );
            let entry = entry?;
            match Self::load(entry.path()) {
                Ok(workspace) => result.push(workspace),
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "unrecognized verification workspace preserved; evidence recovery unavailable")
                }
            }
        }
        Ok(result)
    }

    /// Called only after an index was published and the normal result transaction committed.
    pub(crate) fn cleanup(self) -> Result<()> {
        let current = Self::load(self.container.clone())?;
        ensure!(
            current.identity == self.identity && current.work == self.work,
            "workspace changed before cleanup"
        );
        fs::remove_dir_all(&self.container)?;
        sync_directory(self.container.parent().context("workspace root missing")?)
    }
}

impl WorkspaceIdentity {
    pub(crate) fn report_value(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn activated(data: &Path, job: i64) -> Workspace {
        let mut workspace = Workspace::prepare(&data.join("logs"), job).unwrap();
        workspace
            .activate(
                job,
                1,
                DeliveryFingerprint("synthetic-reviewed-fingerprint".into()),
                Utc::now(),
            )
            .unwrap();
        workspace
    }

    #[test]
    fn activated_drop_keeps_raw_and_marker_outside_the_worker_write_root() {
        let data = tempfile::tempdir().unwrap();
        let workspace = activated(data.path(), 2);
        let work = workspace.path().to_owned();
        let container = workspace.container.clone();
        assert!(!container.join(MARKER).starts_with(&work));
        crate::acceptance_evidence::prepare(&work).unwrap();
        fs::write(work.join(DIRECTORY).join("raw.txt"), b"synthetic raw").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&container).unwrap().permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(container.join(MARKER))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(&work).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
        drop(workspace);
        assert_eq!(
            fs::read(work.join(DIRECTORY).join("raw.txt")).unwrap(),
            b"synthetic raw"
        );
        assert_eq!(
            Workspace::discover(&data.path().join("logs"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn unstarted_prepared_directory_can_be_cleaned_without_an_owned_attempt() {
        let data = tempfile::tempdir().unwrap();
        let workspace = Workspace::prepare(&data.path().join("logs"), 2).unwrap();
        let container = workspace.container.clone();
        drop(workspace);
        assert!(!container.exists());
    }

    #[test]
    fn a_copied_marker_cannot_claim_another_attempt_directory() {
        let data = tempfile::tempdir().unwrap();
        let workspace = activated(data.path(), 2);
        let other = workspace
            .container
            .parent()
            .unwrap()
            .join("2-other-attempt");
        private_directory(&other).unwrap();
        fs::copy(workspace.container.join(MARKER), other.join(MARKER)).unwrap();
        assert!(Workspace::load(other.clone()).is_err());
        assert!(other.join(MARKER).exists());
    }

    #[test]
    fn oversized_or_unknown_marker_is_left_unmodified() {
        let data = tempfile::tempdir().unwrap();
        let workspace = activated(data.path(), 2);
        let marker = workspace.container.join(MARKER);
        let bytes = vec![b'x'; MARKER_BYTES as usize + 1];
        fs::write(&marker, &bytes).unwrap();
        assert!(Workspace::load(workspace.container.clone()).is_err());
        assert_eq!(fs::read(marker).unwrap(), bytes);
    }

    #[cfg(unix)]
    #[test]
    fn linked_marker_and_linked_workspace_roots_are_not_followed() {
        let data = tempfile::tempdir().unwrap();
        let workspace = activated(data.path(), 2);
        let marker = workspace.container.join(MARKER);
        let outside = data.path().join("synthetic-outside.json");
        fs::rename(&marker, &outside).unwrap();
        std::os::unix::fs::symlink(&outside, &marker).unwrap();
        assert!(Workspace::load(workspace.container.clone()).is_err());
        assert!(outside.exists());
        let alias = workspace.container.parent().unwrap().join("3-alias");
        std::os::unix::fs::symlink(&workspace.container, &alias).unwrap();
        assert!(Workspace::load(alias).is_err());
    }

    #[test]
    fn cleanup_refuses_a_changed_owner_marker() {
        let data = tempfile::tempdir().unwrap();
        let workspace = activated(data.path(), 2);
        let container = workspace.container.clone();
        let marker = container.join(MARKER);
        let mut identity = workspace.identity().unwrap().clone();
        identity.source_job_id = 99;
        fs::write(marker, serde_json::to_vec(&identity).unwrap()).unwrap();
        assert!(workspace.cleanup().is_err());
        assert!(container.exists());
    }
}
