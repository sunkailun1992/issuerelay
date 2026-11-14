//! A checked version of the local delivery reviewed by an independent executor.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use aws_lc_rs::digest::{Context as Sha256, SHA256};
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;

const MANIFEST_LIMIT: u64 = 1024 * 1024;
const FILE_LIMIT: u64 = 64 * 1024 * 1024;
const CHANGE_BYTE_LIMIT: u64 = 512 * 1024 * 1024;
const REPOSITORY_LIMIT: usize = 32;
const CHANGE_LIMIT: usize = 4096;

type VerifiedFiles = BTreeMap<String, (Option<String>, bool)>;
type VerifiedRepositories = BTreeMap<PathBuf, (String, VerifiedFiles)>;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Delivery {
    Commit { sha: String },
    Manifest { path: PathBuf },
}

impl Delivery {
    pub fn parse(value: &str) -> Result<Self> {
        let value = value.trim();
        if commit_sha(value) {
            return Ok(Self::Commit {
                sha: value.to_ascii_lowercase(),
            });
        }
        let path = PathBuf::from(value);
        ensure!(
            path.is_absolute(),
            "use a full Git commit SHA or an absolute JSON delivery manifest path"
        );
        Ok(Self::Manifest { path })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DeliveryFingerprint(pub String);

#[path = "acceptance_stage.rs"]
mod staging;
pub use staging::stage;

#[derive(Deserialize)]
#[serde(untagged)]
enum DeliveryManifest {
    Single(ManifestRepository),
    Multiple(Vec<ManifestRepository>),
}

/// The array and the repository/base/files spelling match existing delivery manifests.
#[derive(Deserialize)]
struct ManifestRepository {
    repository: Option<PathBuf>,
    #[serde(alias = "head")]
    base: String,
    files: Vec<ManifestFile>,
}

#[derive(Deserialize)]
struct ManifestFile {
    path: String,
    sha256: Option<String>,
}

fn commit_sha(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

fn file_path(value: &str) -> Result<&Path> {
    let path = Path::new(value);
    ensure!(
        !value.is_empty()
            && !value.chars().any(char::is_control)
            && !value.contains('\\')
            && !value.contains(':')
            && value
                .split('/')
                .all(|part| !matches!(part, "" | "." | ".."))
            && path
                .components()
                .all(|part| matches!(part, Component::Normal(_))),
        "manifest contains an invalid repository-relative file path"
    );
    Ok(path)
}

fn changed_paths(status: &[u8]) -> Result<BTreeMap<&str, &[u8]>> {
    let mut changed = BTreeMap::new();
    if status.is_empty() {
        return Ok(changed);
    }
    let records = status
        .strip_suffix(&[0])
        .context("Git returned incomplete delivery status")?;
    for record in records.split(|byte| *byte == 0) {
        let (code, path) = record
            .split_at_checked(3)
            .context("Git returned invalid delivery status")?;
        ensure!(
            matches!(
                code,
                b"?? " | b" M " | b" D " | b" T " | b"M  " | b"D  " | b"T  " | b"A  "
            ),
            "delivery contains an unresolved conflict or differing staged and working copies"
        );
        let path = std::str::from_utf8(path).context("delivery contains a non-UTF-8 path")?;
        file_path(path)?;
        ensure!(
            changed.insert(path, &code[..2]).is_none(),
            "Git returned a duplicate delivery path"
        );
        ensure!(
            changed.len() <= CHANGE_LIMIT,
            "delivery contains too many changed files"
        );
    }
    Ok(changed)
}

async fn git_state(root: &Path) -> Result<(String, Vec<u8>)> {
    let (head, status) = crate::workflow_git::delivery_state(root)
        .await
        .context("could not inspect the complete delivery worktree safely")?;
    ensure!(
        commit_sha(&head),
        "delivery repository has no full committed HEAD"
    );
    Ok((head, status))
}

async fn file_digest(root: &Path, path: &Path, remaining: u64) -> Result<(String, bool, u64)> {
    let file = root.join(path);
    let metadata = tokio::fs::symlink_metadata(&file).await?;
    let limit = FILE_LIMIT.min(remaining);
    ensure!(
        metadata.is_file() && metadata.len() <= limit,
        "changed delivery path is not a bounded regular file"
    );
    ensure!(
        tokio::fs::canonicalize(&file).await? == file,
        "changed delivery file uses a symbolic link or path alias"
    );
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let handle = options.open(&file).await?;
    let opened = handle.metadata().await?;
    ensure!(
        opened.is_file() && opened.len() == metadata.len(),
        "changed delivery file moved while being read"
    );
    #[cfg(unix)]
    let executable = {
        use std::os::unix::fs::PermissionsExt;
        opened.permissions().mode() & 0o111 != 0
    };
    #[cfg(not(unix))]
    let executable = false;
    let mut input = handle.take(limit + 1);
    let mut digest = Sha256::new(&SHA256);
    let mut buffer = vec![0; 64 * 1024];
    let mut size = 0;
    loop {
        let count = input.read(&mut buffer).await?;
        if count == 0 {
            break;
        }
        size += count as u64;
        digest.update(&buffer[..count]);
    }
    ensure!(
        size == opened.len() && size <= limit,
        "changed delivery file grew or shrank while being read"
    );
    Ok((hex(digest.finish().as_ref()), executable, size))
}

async fn repositories(path: &Path) -> Result<Vec<ManifestRepository>> {
    ensure!(
        path.is_absolute(),
        "delivery manifest path must be absolute"
    );
    let metadata = tokio::fs::symlink_metadata(path).await?;
    ensure!(
        metadata.is_file() && metadata.len() <= MANIFEST_LIMIT,
        "delivery manifest must be a bounded regular JSON file"
    );
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let handle = options.open(path).await?;
    ensure!(
        handle.metadata().await?.is_file(),
        "delivery manifest must be a regular file"
    );
    let mut input = handle.take(MANIFEST_LIMIT + 1);
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() as u64 <= MANIFEST_LIMIT,
        "delivery manifest is too large"
    );
    let manifest: DeliveryManifest = serde_json::from_slice(&bytes)
        .context("delivery manifest is not supported structured JSON")?;
    let repositories = match manifest {
        DeliveryManifest::Single(repository) => vec![repository],
        DeliveryManifest::Multiple(repositories) => repositories,
    };
    ensure!(
        !repositories.is_empty() && repositories.len() <= REPOSITORY_LIMIT,
        "delivery manifest must list between 1 and {REPOSITORY_LIMIT} repositories"
    );
    Ok(repositories)
}

async fn check_repository(
    root: &Path,
    snapshot: ManifestRepository,
    remaining: &mut u64,
) -> Result<(String, VerifiedFiles)> {
    let initial = git_state(root).await?;
    let (head, status) = &initial;
    ensure!(
        commit_sha(&snapshot.base) && snapshot.base.eq_ignore_ascii_case(head),
        "delivery manifest no longer matches the repository HEAD"
    );
    let paths = changed_paths(status)?;
    let mut files = BTreeMap::new();
    ensure!(
        snapshot.files.len() == paths.len(),
        "delivery manifest does not list every changed file"
    );
    for file in snapshot.files {
        let path = file_path(&file.path)?;
        let code = paths
            .get(file.path.as_str())
            .context("delivery manifest includes a file outside the current change set")?;
        let entry = if code.contains(&b'D') {
            ensure!(
                file.sha256.is_none()
                    && tokio::fs::symlink_metadata(root.join(path))
                        .await
                        .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
                "deleted delivery file must have no hash and must be absent"
            );
            (None, false)
        } else {
            let expected = file
                .sha256
                .context("changed delivery file requires SHA-256")?;
            let (actual, executable, size) = file_digest(root, path, *remaining).await?;
            *remaining -= size;
            ensure!(
                expected.eq_ignore_ascii_case(&actual),
                "changed delivery file hash does not match the manifest"
            );
            (Some(actual), executable)
        };
        ensure!(
            files.insert(file.path, entry).is_none(),
            "delivery manifest includes a duplicate file"
        );
    }
    ensure!(
        git_state(root).await? == initial,
        "delivery worktree changed while being inspected"
    );
    Ok((initial.0, files))
}

/// Reads the exact delivered version without using the manifest as a source of commands or
/// fetching Git objects. An input which cannot be verified is rejected.
pub async fn fingerprint(directory: &Path, delivery: &Delivery) -> Result<DeliveryFingerprint> {
    let directory = directory
        .canonicalize()
        .context("delivery directory is unavailable")?;
    ensure!(directory.is_dir(), "delivery workspace is not a directory");
    fingerprint_checked(&directory, &checked(&directory, delivery).await?)
}

fn fingerprint_checked(
    directory: &Path,
    checked: &(&str, VerifiedRepositories),
) -> Result<DeliveryFingerprint> {
    let payload = serde_json::to_vec(&(checked.0, directory, &checked.1))?;
    let digest = aws_lc_rs::digest::digest(&SHA256, &payload);
    Ok(DeliveryFingerprint(hex(digest.as_ref())))
}

async fn checked(
    directory: &Path,
    delivery: &Delivery,
) -> Result<(&'static str, VerifiedRepositories)> {
    let mut verified = BTreeMap::new();
    let kind = match delivery {
        Delivery::Commit { sha } => {
            ensure!(commit_sha(sha), "delivery requires a full Git commit SHA");
            let (head, status) = git_state(directory).await?;
            ensure!(
                sha.eq_ignore_ascii_case(&head) && status.is_empty(),
                "delivery commit must be the clean current HEAD"
            );
            verified.insert(directory.to_owned(), (head, BTreeMap::new()));
            "commit"
        }
        Delivery::Manifest { path } => {
            let mut remaining = CHANGE_BYTE_LIMIT;
            for repository in repositories(path).await? {
                let root = match &repository.repository {
                    Some(path) => {
                        ensure!(
                            path.is_absolute(),
                            "manifest repository path must be absolute"
                        );
                        path.canonicalize()
                            .context("manifest repository is unavailable")?
                    }
                    None => directory.to_owned(),
                };
                ensure!(
                    root.starts_with(directory),
                    "manifest repository is outside the approved workspace"
                );
                if verified.contains_key(&root) {
                    bail!("delivery manifest lists a repository more than once");
                }
                let snapshot = check_repository(&root, repository, &mut remaining).await?;
                verified.insert(root, snapshot);
            }
            "manifest"
        }
    };
    Ok((kind, verified))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::{fs, process::Command};

    fn git(root: &Path, args: &[&str]) -> String {
        let output = Command::new(if cfg!(windows) { "git.exe" } else { "git" })
            .current_dir(root)
            .args(["-c", "core.autocrlf=false", "-c", "commit.gpgsign=false"])
            .args(args)
            .output()
            .expect("fixture requires Git");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    fn repository(root: &Path) -> String {
        fs::create_dir_all(root).unwrap();
        git(root, &["init", "-q"]);
        git(root, &["config", "user.name", "Demo User"]);
        git(root, &["config", "user.email", "demo@example.invalid"]);
        git(root, &["config", "core.autocrlf", "false"]);
        fs::write(root.join("source.txt"), "checked in\n").unwrap();
        fs::write(root.join(".gitignore"), "private.txt\n").unwrap();
        fs::write(root.join(".env"), "DEMO_FIXTURE=excluded\n").unwrap();
        git(root, &["add", "."]);
        git(root, &["commit", "-qm", "Initial fixture"]);
        git(root, &["rev-parse", "HEAD"])
    }

    fn hash(value: &str) -> String {
        hex(aws_lc_rs::digest::digest(&SHA256, value.as_bytes()).as_ref())
    }

    #[tokio::test]
    async fn clean_commit_is_ready_and_ignores_private_files_when_staged() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("repository");
        let head = repository(&root);
        let delivery = Delivery::parse(&head).unwrap();
        let approved = fingerprint(&root, &delivery).await.unwrap();
        assert_eq!(approved.0.len(), 64);
        fs::write(root.join("private.txt"), "ignored").unwrap();
        let scratch = temporary.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        assert_eq!(stage(&root, &delivery, &scratch).await.unwrap(), approved);
        assert_eq!(
            fs::read_to_string(scratch.join("source.txt")).unwrap(),
            "checked in\n"
        );
        for excluded in [".git", ".env", "private.txt"] {
            assert!(!scratch.join(excluded).exists());
        }
        fs::write(root.join("source.txt"), "modified after approval").unwrap();
        assert!(fingerprint(&root, &delivery).await.is_err());
        let stale = Delivery::Commit {
            sha: "1".repeat(40),
        };
        assert!(fingerprint(&root, &stale).await.is_err());
        assert!(Delivery::parse("abcd1234").is_err());
    }

    #[tokio::test]
    async fn assumed_unchanged_files_are_not_a_valid_clean_delivery() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("repository");
        let head = repository(&root);
        git(&root, &["update-index", "--assume-unchanged", "source.txt"]);
        fs::write(root.join("source.txt"), "hidden modified fixture").unwrap();
        let delivery = Delivery::parse(&head).unwrap();
        assert!(fingerprint(&root, &delivery).await.is_err());
    }

    #[tokio::test]
    async fn staging_uses_committed_bytes_when_checkout_has_crlf() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("repository");
        repository(&root);
        fs::write(root.join(".gitattributes"), "source.txt text eol=crlf\n").unwrap();
        git(&root, &["add", ".gitattributes"]);
        git(&root, &["commit", "-qm", "Track CRLF checkout policy"]);
        let head = git(&root, &["rev-parse", "HEAD"]);
        fs::write(root.join("source.txt"), "checked in\r\n").unwrap();
        git(&root, &["add", "source.txt"]);
        assert!(git(&root, &["status", "--porcelain=v1"]).is_empty());
        let delivery = Delivery::parse(&head).unwrap();
        let scratch = temporary.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        stage(&root, &delivery, &scratch).await.unwrap();
        assert_eq!(
            fs::read(scratch.join("source.txt")).unwrap(),
            b"checked in\n"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn staging_uses_approved_modes_with_git_filemode_disabled() {
        use std::os::unix::fs::PermissionsExt;
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("repository");
        let head = repository(&root);
        git(&root, &["config", "core.filemode", "false"]);
        fs::set_permissions(root.join("source.txt"), fs::Permissions::from_mode(0o755)).unwrap();
        let scratch = temporary.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        stage(&root, &Delivery::parse(&head).unwrap(), &scratch)
            .await
            .unwrap();
        assert_eq!(
            fs::metadata(scratch.join("source.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0
        );

        fs::write(root.join("source.txt"), "approved changed source\n").unwrap();
        let manifest = temporary.path().join("version.json");
        fs::write(
            &manifest,
            json!({
                "base": head, "files": [
                    {"path":"source.txt", "sha256":hash("approved changed source\n")}
                ]
            })
            .to_string(),
        )
        .unwrap();
        let delivery = Delivery::Manifest { path: manifest };
        let approved = fingerprint(&root, &delivery).await.unwrap();
        let scratch = temporary.path().join("scratch-changed");
        fs::create_dir(&scratch).unwrap();
        assert_eq!(stage(&root, &delivery, &scratch).await.unwrap(), approved);
        assert_eq!(
            fs::metadata(scratch.join("source.txt"))
                .unwrap()
                .permissions()
                .mode()
                & 0o111,
            0o100
        );
        fs::set_permissions(root.join("source.txt"), fs::Permissions::from_mode(0o644)).unwrap();
        assert_ne!(fingerprint(&root, &delivery).await.unwrap(), approved);
    }

    #[tokio::test]
    async fn existing_multi_repo_shape_binds_complete_changes_and_is_path_independent() {
        let temporary = tempfile::tempdir().unwrap();
        let workspace = temporary.path().join("workspace");
        let client = workspace.join("client");
        let service = workspace.join("service");
        let client_head = repository(&client);
        let service_head = repository(&service);
        fs::write(client.join("source.txt"), "modify source\n").unwrap();
        fs::write(service.join("new.txt"), "new file\n").unwrap();
        let manifest = json!([
            {"repository":client, "base":client_head, "branch":"demo", "patch":"client.patch",
             "files":[{"path":"source.txt","sha256":hash("modify source\n")}]},
            {"repository":service, "base":service_head, "branch":"demo", "patch":"service.patch",
             "files":[{"path":"new.txt","sha256":hash("new file\n")}]},
        ]);
        let path = temporary.path().join("version.json");
        let copy = temporary.path().join("version-copy.json");
        fs::write(&path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        fs::write(&copy, serde_json::to_vec_pretty(&manifest).unwrap()).unwrap();
        let delivery = Delivery::Manifest { path };
        let version = fingerprint(&workspace, &delivery).await.unwrap();
        assert_eq!(
            version,
            fingerprint(&workspace, &Delivery::Manifest { path: copy })
                .await
                .unwrap()
        );
        let scratch = temporary.path().join("scratch");
        fs::create_dir(&scratch).unwrap();
        assert_eq!(
            stage(&workspace, &delivery, &scratch).await.unwrap(),
            version
        );
        assert_eq!(
            fs::read_to_string(scratch.join("client/source.txt")).unwrap(),
            "modify source\n"
        );
        assert_eq!(
            fs::read_to_string(scratch.join("service/new.txt")).unwrap(),
            "new file\n"
        );
        fs::write(service.join("unlisted.txt"), "not reviewed").unwrap();
        assert!(fingerprint(&workspace, &delivery).await.is_err());
        fs::remove_file(service.join("unlisted.txt")).unwrap();
        fs::write(service.join("new.txt"), "mismatched hash").unwrap();
        assert!(fingerprint(&workspace, &delivery).await.is_err());
    }

    #[tokio::test]
    async fn a_summary_of_paths_cannot_substitute_for_a_reviewable_manifest() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("repository");
        repository(&root);
        let path = temporary.path().join("version.json");
        fs::write(
            &path,
            br#"[{"repo":"repo","file":"source.txt","bytes":123}]"#,
        )
        .unwrap();
        assert!(
            fingerprint(&root, &Delivery::Manifest { path })
                .await
                .is_err()
        );
    }
}
