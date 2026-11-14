use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use aws_lc_rs::digest::{Context as Digest, SHA1_FOR_LEGACY_USE_ONLY, SHA256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    Delivery, DeliveryFingerprint, FILE_LIMIT, VerifiedFiles, checked, commit_sha, file_path,
    fingerprint_checked, hex,
};
use crate::workflow_git::DeliveryObjects;

const MAX_FILES: usize = 50_000;
const MAX_BYTES: u64 = 512 * 1024 * 1024;

fn exclude(path: &str) -> bool {
    path.split('/').any(|part| {
        let part = part.to_ascii_lowercase();
        matches!(
            part.as_str(),
            ".git"
                | ".codex"
                | ".claude"
                | ".mcp.json"
                | ".npmrc"
                | ".pypirc"
                | ".netrc"
                | ".ssh"
                | ".aws"
                | ".gradle"
                | ".m2"
                | ".cache"
                | "__pycache__"
                | "node_modules"
                | "dist"
                | "target"
                | "credentials"
                | "credentials.json"
                | "secrets"
        ) || part == ".env"
            || part.starts_with(".env.")
            || [".pem", ".key", ".p12", ".pfx"]
                .iter()
                .any(|suffix| part.ends_with(suffix))
    })
}

enum FileIdentity {
    Blob(String),
    Worktree(String),
}

struct StagedFile {
    identity: FileIdentity,
    executable: bool,
}

fn tree_entries(tree: &[u8], changed: &VerifiedFiles) -> Result<BTreeMap<String, StagedFile>> {
    let mut paths = BTreeMap::new();
    let records = if tree.is_empty() {
        tree
    } else {
        tree.strip_suffix(&[0])
            .context("Git returned incomplete delivery tree")?
    };
    for record in records
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty())
    {
        let record = std::str::from_utf8(record).context("delivery has a non-UTF-8 path")?;
        let (metadata, path) = record
            .split_once('\t')
            .context("Git returned an invalid delivery tree")?;
        let mut fields = metadata.split(' ');
        let mode = fields.next().context("Git tree lacks file mode")?;
        let kind = fields.next().context("Git tree lacks object kind")?;
        let hash = fields.next().context("Git tree lacks object identity")?;
        ensure!(
            fields.next().is_none() && commit_sha(hash),
            "delivery Git tree includes an invalid object"
        );
        file_path(path)?;
        if changed.contains_key(path) || exclude(path) || matches!(mode, "120000" | "160000") {
            continue;
        }
        ensure!(
            kind == "blob"
                && matches!(mode, "100644" | "100755")
                && paths
                    .insert(
                        path.into(),
                        StagedFile {
                            identity: FileIdentity::Blob(hash.into()),
                            executable: mode == "100755"
                        }
                    )
                    .is_none(),
            "delivery Git tree includes an unsupported or duplicate path"
        );
    }
    for (path, (hash, executable)) in changed {
        if let Some(hash) = hash {
            ensure!(
                !exclude(path),
                "a changed delivery path is excluded from verification scratch"
            );
            paths.insert(
                path.clone(),
                StagedFile {
                    identity: FileIdentity::Worktree(hash.clone()),
                    executable: *executable,
                },
            );
        }
    }
    ensure!(
        paths.len() <= MAX_FILES,
        "delivery contains too many files to stage"
    );
    Ok(paths)
}

async fn read_worktree_file(source: &Path, expected: &str, limit: u64) -> Result<Vec<u8>> {
    let metadata = tokio::fs::symlink_metadata(source).await?;
    ensure!(
        metadata.is_file()
            && metadata.len() <= limit
            && tokio::fs::canonicalize(source).await? == source,
        "scratch source is not a bounded regular file at its approved path"
    );
    let mut options = tokio::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    let input = options.open(source).await?;
    let opened = input.metadata().await?;
    ensure!(
        opened.is_file() && opened.len() == metadata.len(),
        "scratch source changed while being opened"
    );
    let mut input = input.take(limit + 1);
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes).await?;
    ensure!(
        bytes.len() as u64 == opened.len()
            && bytes.len() as u64 <= limit
            && hex(aws_lc_rs::digest::digest(&SHA256, &bytes).as_ref()) == expected,
        "scratch content does not match the approved delivery"
    );
    Ok(bytes)
}

fn verify_blob(bytes: &[u8], expected: &str) -> Result<()> {
    let algorithm = if expected.len() == 40 {
        // The legacy algorithm is used here only for Git object-format compatibility.
        &SHA1_FOR_LEGACY_USE_ONLY
    } else {
        &SHA256
    };
    let mut digest = Digest::new(algorithm);
    digest.update(format!("blob {}\0", bytes.len()).as_bytes());
    digest.update(bytes);
    ensure!(
        hex(digest.finish().as_ref()) == expected,
        "Git blob does not match the approved delivery"
    );
    Ok(())
}

async fn write_file(target: &Path, bytes: &[u8], executable: bool) -> Result<()> {
    let mut output = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .await?;
    output.write_all(bytes).await?;
    output.flush().await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let permissions = std::fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 });
        output.set_permissions(permissions).await?;
    }
    #[cfg(not(unix))]
    let _ = executable;
    Ok(())
}

/// Stage a verified copy for the independent executor. The caller owns and cleans the scratch
/// directory. Repositories retain their paths relative to the approved workspace.
pub async fn stage(
    directory: &Path,
    delivery: &Delivery,
    destination: &Path,
) -> Result<DeliveryFingerprint> {
    let directory = directory
        .canonicalize()
        .context("delivery directory is unavailable")?;
    ensure!(
        tokio::fs::symlink_metadata(destination).await?.is_dir(),
        "acceptance scratch must be a regular directory"
    );
    let destination = destination
        .canonicalize()
        .context("acceptance scratch is unavailable")?;
    ensure!(
        !destination.starts_with(&directory)
            && tokio::fs::read_dir(&destination)
                .await?
                .next_entry()
                .await?
                .is_none(),
        "acceptance scratch must be empty and outside the source workspace"
    );
    let before = checked(&directory, delivery).await?;
    let mut remaining_files = MAX_FILES;
    let mut remaining_bytes = MAX_BYTES;
    for (root, (head, changed)) in &before.1 {
        let relative_root = root
            .strip_prefix(&directory)
            .context("manifest repository is outside the approved workspace")?;
        let target_root = destination.join(relative_root);
        tokio::fs::create_dir_all(&target_root).await?;
        let paths = tree_entries(
            &crate::workflow_git::delivery_tree(root, head)
                .await
                .context("could not read the approved delivery Git tree")?,
            changed,
        )?;
        let mut objects = DeliveryObjects::open(root)?;
        for (path, file) in paths {
            remaining_files = remaining_files
                .checked_sub(1)
                .context("delivery has too many files")?;
            let path = file_path(&path)?;
            let limit = FILE_LIMIT.min(remaining_bytes);
            let bytes = match file.identity {
                FileIdentity::Blob(hash) => {
                    let bytes = objects.blob(&hash, limit).await?;
                    verify_blob(&bytes, &hash)?;
                    bytes
                }
                FileIdentity::Worktree(hash) => {
                    read_worktree_file(&root.join(path), &hash, limit).await?
                }
            };
            let target = target_root.join(path);
            tokio::fs::create_dir_all(target.parent().context("scratch file has no parent")?)
                .await?;
            write_file(&target, &bytes, file.executable).await?;
            remaining_bytes -= bytes.len() as u64;
        }
        objects.finish().await?;
    }
    ensure!(
        checked(&directory, delivery).await? == before,
        "delivery changed while being staged"
    );
    fingerprint_checked(&directory, &before)
}
