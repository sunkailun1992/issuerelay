//! Bounded private artifacts retained after the disposable verifier has exited.
//!
//! Only DIRECTORY is collected. Model-provided evidence strings are never paths to read.
//! The caller has already stopped the executor tree. Checks detect source replacement, but do
//! not claim isolation from arbitrary concurrently malicious processes with the same OS user.
use std::ffi::OsStr;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use aws_lc_rs::digest::{SHA256, digest};
use serde::{Deserialize, Serialize};

mod workspace;
pub(crate) use workspace::{Workspace, WorkspaceIdentity};

pub(crate) const DIRECTORY: &str = ".issuerelay-evidence";
const MAX_FILES: usize = 256;
const MAX_ENTRIES: usize = 4096;
const MAX_DEPTH: usize = 16;
const FILE_BYTES: u64 = 16 * 1024 * 1024;
const TOTAL_BYTES: u64 = 64 * 1024 * 1024;
const REPORT_BYTES: usize = 128 * 1024;
const INDEX_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy)]
struct Limits {
    files: usize,
    entries: usize,
    depth: usize,
    file_bytes: u64,
    total_bytes: u64,
}
const LIMITS: Limits = Limits {
    files: MAX_FILES,
    entries: MAX_ENTRIES,
    depth: MAX_DEPTH,
    file_bytes: FILE_BYTES,
    total_bytes: TOTAL_BYTES,
};

#[derive(Clone)]
struct Snapshot {
    metadata: Metadata,
    identity: (u64, u64),
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RetainedFile {
    path: String,
    full_path: PathBuf,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Index {
    version: u32,
    job_id: i64,
    root: PathBuf,
    file_count: usize,
    worker_file_count: usize,
    total_bytes: u64,
    files: Vec<RetainedFile>,
}

fn plain(metadata: &Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return false;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false; // FILE_ATTRIBUTE_REPARSE_POINT, including junctions.
        }
    }
    metadata.is_file() || metadata.is_dir()
}

fn open(path: &Path, directory: bool) -> Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(
            libc::O_NOFOLLOW | libc::O_NONBLOCK | if directory { libc::O_DIRECTORY } else { 0 },
        );
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Do not traverse a reparse point; deny concurrent write/delete sharing while held.
        options
            .custom_flags(0x0020_0000 | if directory { 0x0200_0000 } else { 0 })
            .share_mode(1);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = directory;
    Ok(options.open(path)?)
}

#[cfg(unix)]
fn identity(_file: &File, metadata: &Metadata) -> Result<(u64, u64)> {
    use std::os::unix::fs::MetadataExt;
    ensure!(
        metadata.is_dir() || metadata.nlink() == 1,
        "evidence hard links are not allowed"
    );
    Ok((metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn identity(file: &File, _metadata: &Metadata) -> Result<(u64, u64)> {
    use std::os::windows::io::AsRawHandle;
    // MetadataExt's file_index is still unstable. This stable Win32 API supplies the same
    // volume/file identity without a new dependency or changing process permissions.
    #[repr(C)]
    #[derive(Default)]
    struct Information {
        attributes: u32,
        creation: [u32; 2],
        access: [u32; 2],
        write: [u32; 2],
        volume: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Information)
        -> i32;
    }
    let mut info = Information::default();
    // SAFETY: the handle belongs to the live File and info has the Win32 struct's C layout.
    ensure!(
        unsafe { GetFileInformationByHandle(file.as_raw_handle(), &mut info) } != 0,
        "could not identify the evidence file"
    );
    ensure!(
        info.attributes & 0x400 == 0,
        "evidence reparse points are not allowed"
    );
    ensure!(
        info.attributes & 0x10 != 0 || info.links == 1,
        "evidence hard links are not allowed"
    );
    Ok((
        u64::from(info.volume),
        (u64::from(info.index_high) << 32) | u64::from(info.index_low),
    ))
}

#[cfg(not(any(unix, windows)))]
fn identity(_file: &File, _metadata: &Metadata) -> Result<(u64, u64)> {
    bail!("verification evidence is unsupported on this platform")
}

fn same_metadata(a: &Metadata, b: &Metadata) -> bool {
    if !plain(a) || !plain(b) || a.is_dir() != b.is_dir() {
        return false;
    }
    if a.is_file() && (a.len() != b.len() || a.modified().ok() != b.modified().ok()) {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev()
            || a.ino() != b.ino()
            || (a.is_file()
                && (a.nlink() != 1
                    || b.nlink() != 1
                    || a.ctime() != b.ctime()
                    || a.ctime_nsec() != b.ctime_nsec()))
        {
            return false;
        }
    }
    true
}

impl Snapshot {
    fn read(path: &Path) -> Result<Self> {
        let before = fs::symlink_metadata(path)?;
        ensure!(
            plain(&before),
            "evidence must contain only regular files and directories"
        );
        ensure!(
            fs::canonicalize(path)? == path,
            "evidence path contains an alias"
        );
        let file = open(path, before.is_dir())?;
        let metadata = file.metadata()?;
        ensure!(
            same_metadata(&before, &metadata),
            "evidence changed while being opened"
        );
        let identity = identity(&file, &metadata)?;
        Ok(Self { metadata, identity })
    }

    fn check(&self, path: &Path) -> Result<()> {
        let current = Self::read(path)?;
        ensure!(
            self.identity == current.identity && same_metadata(&self.metadata, &current.metadata),
            "evidence changed while being retained"
        );
        Ok(())
    }

    fn check_open(&self, file: &File) -> Result<()> {
        let metadata = file.metadata()?;
        ensure!(
            same_metadata(&self.metadata, &metadata) && identity(file, &metadata)? == self.identity,
            "evidence file changed while being retained"
        );
        Ok(())
    }
}

fn canonical_directory(path: &Path) -> Result<PathBuf> {
    let canonical = fs::canonicalize(path).context("verification directory is unavailable")?;
    ensure!(
        Snapshot::read(&canonical)?.metadata.is_dir(),
        "verification root must be a directory"
    );
    Ok(canonical)
}

fn component(name: &OsStr) -> Result<&str> {
    let name = name.to_str().context("evidence names must be UTF-8")?;
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|n| n.len() == 1 && matches!(n.as_bytes()[0], b'1'..=b'9'));
    ensure!(
        !name.is_empty()
            && name != "."
            && name != ".."
            && name.len() <= 255
            && !name.ends_with(['.', ' '])
            && !reserved
            && !name.chars().any(|c| {
                c.is_control() || matches!(c, '/' | '\\' | ':' | '<' | '>' | '"' | '|' | '?' | '*')
            }),
        "evidence name is not a portable relative component"
    );
    Ok(name)
}

fn private_directory(path: &Path) -> Result<()> {
    let builder = fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    builder.create(path)?;
    // Windows inherits the current user's AppData ACL; no ACL or privilege is broadened.
    Ok(())
}

fn check_parent_chain(path: &Path) -> Result<()> {
    let mut current = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, Component::CurDir | Component::ParentDir),
            "invalid host evidence path"
        );
        current.push(part.as_os_str());
        // A Windows Prefix such as \\?\C: is not a directory until its following RootDir
        // has been appended. Metadata on the bare prefix would reject valid canonical paths.
        if matches!(part, Component::Prefix(_)) {
            continue;
        }
        let metadata = fs::symlink_metadata(&current)?;
        ensure!(
            plain(&metadata) && metadata.is_dir(),
            "host evidence parent contains a link or non-directory"
        );
    }
    Ok(())
}

/// Reserve the only directory the worker may use for retained artifacts. Existing content is
/// refused so a delivery cannot prepopulate or redirect this directory before the run starts.
pub(crate) fn prepare(scratch: &Path) -> Result<()> {
    let scratch = canonical_directory(scratch)?;
    private_directory(&scratch.join(DIRECTORY))
}

fn collect(
    directory: &Path,
    relative: &Path,
    depth: usize,
    limits: Limits,
    entries: &mut usize,
    directories: &mut Vec<(PathBuf, Snapshot)>,
    files: &mut Vec<(PathBuf, Snapshot)>,
) -> Result<()> {
    ensure!(
        depth <= limits.depth,
        "verification evidence is too deeply nested"
    );
    let snapshot = Snapshot::read(directory)?;
    ensure!(
        snapshot.metadata.is_dir(),
        "evidence directory was replaced"
    );
    directories.push((directory.to_owned(), snapshot.clone()));
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        *entries = entries
            .checked_add(1)
            .context("too many evidence entries")?;
        ensure!(
            *entries <= limits.entries,
            "verification evidence contains too many entries"
        );
        component(&entry.file_name())?;
        let relative = relative.join(entry.file_name());
        ensure!(
            relative.as_os_str().len() <= 4096,
            "verification evidence path is too long"
        );
        let path = directory.join(entry.file_name());
        let item = Snapshot::read(&path)?;
        if item.metadata.is_dir() {
            collect(
                &path,
                &relative,
                depth + 1,
                limits,
                entries,
                directories,
                files,
            )?;
        } else {
            ensure!(
                item.metadata.is_file(),
                "verification evidence is not a regular file"
            );
            ensure!(
                files.len() < limits.files,
                "verification evidence contains too many files"
            );
            ensure!(
                item.metadata.len() <= limits.file_bytes,
                "verification evidence file is too large"
            );
            files.push((relative, item));
        }
    }
    snapshot.check(directory)?;
    Ok(())
}

fn read_file(path: &Path, snapshot: &Snapshot, limit: u64) -> Result<Vec<u8>> {
    snapshot.check(path)?;
    let file = open(path, false)?;
    snapshot.check_open(&file)?;
    let mut input = (&file).take(limit.checked_add(1).context("invalid evidence limit")?);
    let mut bytes = Vec::new();
    input.read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit && bytes.len() as u64 == snapshot.metadata.len(),
        "verification evidence grew or shrank while being read"
    );
    snapshot.check_open(&file)?;
    snapshot.check(path)?;
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    digest(&SHA256, bytes)
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn write_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600).custom_flags(libc::O_NOFOLLOW);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    open(path, true)?.sync_all()?;
    // Windows sync_all flushes each payload/index handle. Directory handles have no equivalent
    // portable std flush; publication below uses Windows' non-replacing directory rename.
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn publish(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::ffi::OsStrExt;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())?;
        let destination = std::ffi::CString::new(destination.as_os_str().as_bytes())?;
        // SAFETY: both owned C strings remain alive for the synchronous call. EXCL/NOREPLACE
        // makes an existing job directory a conflict, including an empty directory.
        #[cfg(target_os = "macos")]
        let result =
            unsafe { libc::renamex_np(source.as_ptr(), destination.as_ptr(), libc::RENAME_EXCL) };
        #[cfg(target_os = "linux")]
        let result = unsafe {
            libc::renameat2(
                libc::AT_FDCWD,
                source.as_ptr(),
                libc::AT_FDCWD,
                destination.as_ptr(),
                libc::RENAME_NOREPLACE,
            )
        };
        if result != 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(source: *const u16, destination: *const u16, flags: u32) -> i32;
        }
        let wide = |path: &Path| -> Result<Vec<u16>> {
            let mut text: Vec<u16> = path.as_os_str().encode_wide().collect();
            ensure!(!text.contains(&0), "invalid evidence publication path");
            text.push(0);
            Ok(text)
        };
        let source = wide(source)?;
        let destination = wide(destination)?;
        // SAFETY: the owned NUL-terminated UTF-16 buffers are alive for the call. Flags=0
        // refuses an existing destination. std::fs::rename may use a replacing fallback.
        if unsafe { MoveFileExW(source.as_ptr(), destination.as_ptr(), 0) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
    {
        let _ = (source, destination);
        bail!("atomic evidence publication is unsupported on this platform")
    }
}

fn retain_sync(
    scratch: &Path,
    logs_dir: &Path,
    job_id: i64,
    report: &serde_json::Value,
    limits: Limits,
) -> Result<PathBuf> {
    ensure!(job_id > 0, "invalid verification run ID");
    let scratch = canonical_directory(scratch)?;
    let source = scratch.join(DIRECTORY);
    let source_identity = Snapshot::read(&source)?;
    ensure!(
        source_identity.metadata.is_dir(),
        "verification evidence directory is missing or replaced"
    );
    let mut report_bytes = serde_json::to_vec_pretty(report)?;
    report_bytes.push(b'\n');
    ensure!(
        report_bytes.len() <= REPORT_BYTES && report_bytes.len() as u64 <= limits.total_bytes,
        "structured verification report is too large"
    );
    let mut directories = Vec::new();
    let mut files = Vec::new();
    collect(
        &source,
        Path::new(""),
        0,
        limits,
        &mut 0,
        &mut directories,
        &mut files,
    )?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    ensure!(
        logs_dir.is_absolute(),
        "verification logs directory must be absolute"
    );
    let parent = canonical_directory(
        logs_dir
            .parent()
            .context("verification logs have no data parent")?,
    )?;
    check_parent_chain(&parent)?;
    ensure!(
        !parent.starts_with(&scratch),
        "retained evidence must be outside the disposable scratch"
    );
    let root = parent.join("verification-evidence");
    match fs::symlink_metadata(&root) {
        Ok(_) => {
            ensure!(
                Snapshot::read(&root)?.metadata.is_dir(),
                "invalid host evidence root"
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => private_directory(&root)?,
        Err(error) => return Err(error.into()),
    }
    check_parent_chain(&root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    }
    let host_identity = Snapshot::read(&root)?;
    let destination = root.join(job_id.to_string());
    match fs::symlink_metadata(&destination) {
        Ok(_) => bail!("verification evidence for this run already exists"),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let temporary = tempfile::Builder::new()
        .prefix(".retaining-")
        .tempdir_in(&root)?;
    let temporary_root = temporary.path().to_owned();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        // tempfile follows the process umask for directories; restrict the new directory
        // explicitly before placing any report or worker bytes inside it.
        fs::set_permissions(&temporary_root, fs::Permissions::from_mode(0o700))?;
    }
    let payload_root = temporary_root.join("files");
    private_directory(&payload_root)?;
    let mut payload_directories = vec![payload_root.clone()];
    let mut retained = Vec::new();
    let worker_file_count = files.len();
    let mut total_bytes = report_bytes.len() as u64;
    for (relative, snapshot) in files {
        for (directory, snapshot) in &directories {
            snapshot.check(directory)?;
        }
        let remaining = limits
            .total_bytes
            .checked_sub(total_bytes)
            .context("verification evidence exceeds total size limit")?;
        let bytes = read_file(
            &source.join(&relative),
            &snapshot,
            limits.file_bytes.min(remaining),
        )?;
        let target = payload_root.join(&relative);
        let mut directory = payload_root.clone();
        if let Some(parent) = relative.parent() {
            for name in parent.components() {
                directory.push(name);
                if !payload_directories.contains(&directory) {
                    private_directory(&directory)?;
                    payload_directories.push(directory.clone());
                }
            }
        }
        write_file(&target, &bytes)?;
        total_bytes = total_bytes
            .checked_add(bytes.len() as u64)
            .context("verification evidence size overflow")?;
        let path = format!(
            "files/{}",
            relative
                .iter()
                .map(|n| n.to_str().expect("validated name"))
                .collect::<Vec<_>>()
                .join("/")
        );
        retained.push(RetainedFile {
            full_path: destination.join(&path),
            path,
            bytes: bytes.len() as u64,
            sha256: sha256(&bytes),
        });
    }
    write_file(&temporary_root.join("result.json"), &report_bytes)?;
    retained.push(RetainedFile {
        path: "result.json".into(),
        full_path: destination.join("result.json"),
        bytes: report_bytes.len() as u64,
        sha256: sha256(&report_bytes),
    });
    retained.sort_by(|a, b| a.path.cmp(&b.path));
    let index = Index {
        version: 1,
        job_id,
        root: destination.clone(),
        file_count: retained.len(),
        worker_file_count,
        total_bytes,
        files: retained,
    };
    let mut bytes = serde_json::to_vec_pretty(&index)?;
    bytes.push(b'\n');
    ensure!(
        bytes.len() <= INDEX_BYTES,
        "verification evidence index is too large"
    );
    write_file(&temporary_root.join("index.json"), &bytes)?;
    for (directory, snapshot) in &directories {
        snapshot.check(directory)?;
    }
    source_identity.check(&source)?;
    host_identity.check(&root)?;
    payload_directories.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for directory in payload_directories {
        sync_directory(&directory)?;
    }
    sync_directory(&temporary_root)?;
    publish(&temporary_root, &destination)?;
    sync_directory(&root)?;
    Ok(destination.join("index.json"))
}

/// Copies only fixed worker artifacts and the bounded host report, then atomically publishes a
/// new immutable job directory. Any invalid entry or I/O error rejects the entire retention.
pub(crate) async fn retain(
    scratch: &Path,
    logs_dir: &Path,
    job_id: i64,
    report: &serde_json::Value,
) -> Result<PathBuf> {
    let (scratch, logs_dir, report) = (scratch.to_owned(), logs_dir.to_owned(), report.clone());
    tokio::task::spawn_blocking(move || retain_sync(&scratch, &logs_dir, job_id, &report, LIMITS))
        .await
        .context("verification evidence retention task failed")?
}

/// A previous host may have published the immutable index just before losing its result
/// transaction. Reuse it only after checking its exact attempt identity and every payload;
/// the stored worker final text is never interpreted as an acceptance decision here.
pub(crate) fn published_for_workspace(
    logs_dir: &Path,
    workspace: &WorkspaceIdentity,
) -> Result<Option<PathBuf>> {
    let parent = canonical_directory(
        logs_dir
            .parent()
            .context("verification logs have no data parent")?,
    )?;
    check_parent_chain(&parent)?;
    let destination = parent
        .join("verification-evidence")
        .join(workspace.verification_job_id.to_string());
    match fs::symlink_metadata(&destination) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
        Ok(_) => {}
    }
    check_parent_chain(&destination)?;
    let index_path = destination.join("index.json");
    let index_snapshot = Snapshot::read(&index_path)?;
    let index: Index = serde_json::from_slice(&read_file(
        &index_path,
        &index_snapshot,
        INDEX_BYTES as u64,
    )?)?;
    ensure!(
        index.version == 1
            && index.job_id == workspace.verification_job_id
            && index.root == destination
            && index.file_count == index.files.len()
            && index.worker_file_count <= MAX_FILES
            && index.file_count == index.worker_file_count + 1
            && index.total_bytes <= TOTAL_BYTES,
        "published verification index does not match this workspace"
    );
    let mut dirs = Vec::new();
    let mut payload = Vec::new();
    collect(
        &destination.join("files"),
        Path::new(""),
        0,
        LIMITS,
        &mut 0,
        &mut dirs,
        &mut payload,
    )?;
    let mut actual = std::collections::BTreeMap::new();
    for (relative, snapshot) in payload {
        let path = format!(
            "files/{}",
            relative
                .iter()
                .map(|part| part.to_str().expect("validated path"))
                .collect::<Vec<_>>()
                .join("/")
        );
        let bytes = read_file(&destination.join(&path), &snapshot, FILE_BYTES)?;
        actual.insert(path, (bytes.len() as u64, sha256(&bytes)));
    }
    let report_path = destination.join("result.json");
    let report_snapshot = Snapshot::read(&report_path)?;
    let bytes = read_file(&report_path, &report_snapshot, REPORT_BYTES as u64)?;
    let report: serde_json::Value = serde_json::from_slice(&bytes)?;
    ensure!(
        report["job_id"] == workspace.verification_job_id
            && report["development_job_id"] == workspace.source_job_id
            && report["delivery_fingerprint"] == serde_json::to_value(&workspace.fingerprint)?
            && report["workspace"] == workspace.report_value()?,
        "published verification evidence belongs to another attempt"
    );
    actual.insert("result.json".into(), (bytes.len() as u64, sha256(&bytes)));
    let mut declared = std::collections::BTreeMap::new();
    for file in &index.files {
        ensure!(
            file.full_path == destination.join(&file.path)
                && declared
                    .insert(file.path.clone(), (file.bytes, file.sha256.clone()))
                    .is_none(),
            "invalid or duplicate published payload"
        );
    }
    ensure!(
        actual == declared && actual.values().map(|row| row.0).sum::<u64>() == index.total_bytes,
        "published verification evidence hash or path set changed"
    );
    let top: std::collections::BTreeSet<_> = fs::read_dir(&destination)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<_>>()?;
    ensure!(
        top == ["files", "index.json", "result.json"]
            .map(std::ffi::OsString::from)
            .into_iter()
            .collect(),
        "unexpected published verification files"
    );
    for (path, snapshot) in dirs {
        snapshot.check(&path)?;
    }
    index_snapshot.check(&index_path)?;
    report_snapshot.check(&report_path)?;
    Ok(Some(index_path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn report() -> Value {
        json!({"result":"verification_passed", "summary":"Synthetic report", "evidence":["No extra files"], "delivery":"synthetic"})
    }
    fn setup() -> (tempfile::TempDir, tempfile::TempDir) {
        let scratch = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        prepare(scratch.path()).unwrap();
        (scratch, data)
    }
    fn index(path: &Path) -> Value {
        serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
    }
    fn small() -> Limits {
        Limits {
            files: 2,
            entries: 8,
            depth: 2,
            file_bytes: 8,
            total_bytes: 1024,
        }
    }
    fn retained(scratch: &Path, data: &Path, limits: Limits) -> Result<PathBuf> {
        retain_sync(scratch, &data.join("logs"), 7, &report(), limits)
    }

    #[tokio::test]
    async fn report_and_index_survive_disposable_scratch_cleanup() {
        let (scratch, data) = setup();
        let path = retain(scratch.path(), &data.path().join("logs"), 7, &report())
            .await
            .unwrap();
        drop(scratch);
        let manifest = index(&path);
        assert_eq!(manifest["job_id"], 7);
        assert_eq!(manifest["worker_file_count"], 0);
        assert_eq!(manifest["file_count"], 1);
        let result = path.parent().unwrap().join("result.json");
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(result).unwrap()).unwrap(),
            report()
        );
    }

    #[test]
    fn binary_unicode_nested_files_have_a_complete_host_hash_index() {
        let (scratch, data) = setup();
        let source = scratch.path().join(DIRECTORY);
        fs::create_dir(source.join("nested")).unwrap();
        fs::write(source.join("nested/合成.bin"), [0, 255, 10, 3]).unwrap();
        fs::write(source.join("empty.txt"), []).unwrap();
        fs::write(source.join("index.json"), "Untrusted worker index").unwrap();
        fs::write(scratch.path().join("outside.txt"), "Must not be retained").unwrap();
        let path = retained(scratch.path(), data.path(), LIMITS).unwrap();
        let manifest = index(&path);
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), 4);
        assert_eq!(manifest["worker_file_count"], 3);
        let mut total = 0;
        for entry in files {
            let p = Path::new(entry["full_path"].as_str().unwrap());
            assert_eq!(
                p,
                path.parent().unwrap().join(entry["path"].as_str().unwrap())
            );
            let bytes = fs::read(p).unwrap();
            assert_eq!(entry["bytes"], bytes.len());
            assert_eq!(entry["sha256"], sha256(&bytes));
            total += bytes.len();
        }
        assert_eq!(manifest["total_bytes"], total);
        assert!(!path.parent().unwrap().join("files/outside.txt").exists());
        assert_eq!(
            fs::read(path.parent().unwrap().join("files/nested/合成.bin")).unwrap(),
            [0, 255, 10, 3]
        );
    }

    #[test]
    fn model_evidence_strings_never_become_paths_to_read() {
        let (scratch, data) = setup();
        let sentinel = data.path().join("private.txt");
        fs::write(&sentinel, "Synthetic sentinel").unwrap();
        let r = json!({"evidence":[sentinel, "../../private.txt", "C:\\secret", "https://example.invalid/file"]});
        let path = retain_sync(scratch.path(), &data.path().join("logs"), 7, &r, LIMITS).unwrap();
        assert_eq!(index(&path)["file_count"], 1);
        assert_eq!(
            fs::read_dir(path.parent().unwrap().join("files"))
                .unwrap()
                .count(),
            0
        );
    }

    #[test]
    fn prepare_refuses_existing_directory_and_retention_requires_it() {
        let (scratch, data) = setup();
        assert!(prepare(scratch.path()).is_err());
        fs::remove_dir(scratch.path().join(DIRECTORY)).unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
        fs::write(scratch.path().join(DIRECTORY), "not a directory").unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
    }

    #[test]
    fn existing_job_evidence_is_never_overwritten() {
        let (scratch, data) = setup();
        let first = retained(scratch.path(), data.path(), LIMITS).unwrap();
        let bytes = fs::read(&first).unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
        assert_eq!(fs::read(first).unwrap(), bytes);
        let root = data.path().join("verification-evidence");
        fs::create_dir(root.join("8")).unwrap();
        assert!(
            retain_sync(
                scratch.path(),
                &data.path().join("logs"),
                8,
                &report(),
                LIMITS
            )
            .is_err()
        );
        assert_eq!(fs::read_dir(root.join("8")).unwrap().count(), 0);
    }

    #[test]
    fn portable_names_reject_traversal_ads_control_and_windows_aliases() {
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "C:",
            "name:stream",
            "bad<name",
            "bad>name",
            "bad\"name",
            "bad|name",
            "bad?name",
            "bad*name",
            "control\n",
            "NUL",
            "con.txt",
            "COM1",
            "LPT9.log",
            "trailing.",
            "trailing ",
        ] {
            assert!(component(OsStr::new(name)).is_err(), "accepted {name:?}");
        }
        for name in ["report.md", "合成.bin", "file with space.txt", ".hidden"] {
            assert!(component(OsStr::new(name)).is_ok());
        }
    }

    // Windows reserves colon-containing filesystem names; component validation above is
    // platform independent, while this actual ordinary-file fixture is Unix-specific.
    #[cfg(unix)]
    #[test]
    fn invalid_name_fails_whole_collection_without_partial_publication() {
        let (scratch, data) = setup();
        fs::write(scratch.path().join(DIRECTORY).join("bad:name"), "x").unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
        assert!(!data.path().join("verification-evidence/7").exists());
    }

    #[test]
    fn file_count_and_size_accept_exact_limits_and_reject_one_more() {
        let (scratch, data) = setup();
        let source = scratch.path().join(DIRECTORY);
        fs::write(source.join("a"), [0; 8]).unwrap();
        fs::write(source.join("b"), [1; 8]).unwrap();
        assert!(retained(scratch.path(), data.path(), small()).is_ok());
        let (scratch, data) = setup();
        fs::write(scratch.path().join(DIRECTORY).join("a"), [0; 9]).unwrap();
        assert!(retained(scratch.path(), data.path(), small()).is_err());
        fs::write(scratch.path().join(DIRECTORY).join("a"), [0; 8]).unwrap();
        fs::write(scratch.path().join(DIRECTORY).join("b"), []).unwrap();
        fs::write(scratch.path().join(DIRECTORY).join("c"), []).unwrap();
        assert!(retained(scratch.path(), data.path(), small()).is_err());
    }

    #[test]
    fn total_payload_budget_includes_the_host_report() {
        let (scratch, data) = setup();
        fs::write(scratch.path().join(DIRECTORY).join("a"), [0; 8]).unwrap();
        let report_bytes = serde_json::to_vec_pretty(&report()).unwrap().len() as u64 + 1;
        let limits = Limits {
            total_bytes: report_bytes + 8,
            ..small()
        };
        assert!(retained(scratch.path(), data.path(), limits).is_ok());
        let (scratch, data) = setup();
        fs::write(scratch.path().join(DIRECTORY).join("a"), [0; 8]).unwrap();
        assert!(
            retained(
                scratch.path(),
                data.path(),
                Limits {
                    total_bytes: report_bytes + 7,
                    ..small()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn traversal_budget_bounds_empty_directories_and_depth() {
        let (scratch, data) = setup();
        let source = scratch.path().join(DIRECTORY);
        fs::create_dir_all(source.join("one/two")).unwrap();
        assert!(retained(scratch.path(), data.path(), small()).is_ok());
        let (scratch, data) = setup();
        let source = scratch.path().join(DIRECTORY);
        fs::create_dir_all(source.join("one/two/three")).unwrap();
        assert!(retained(scratch.path(), data.path(), small()).is_err());
        let (scratch, data) = setup();
        for n in 0..9 {
            fs::create_dir(scratch.path().join(DIRECTORY).join(n.to_string())).unwrap();
        }
        assert!(retained(scratch.path(), data.path(), small()).is_err());
    }

    #[test]
    fn oversized_host_report_and_invalid_job_id_are_rejected() {
        let (scratch, data) = setup();
        let oversized = json!({"summary":"x".repeat(REPORT_BYTES)});
        assert!(
            retain_sync(
                scratch.path(),
                &data.path().join("logs"),
                7,
                &oversized,
                LIMITS
            )
            .is_err()
        );
        assert!(
            retain_sync(
                scratch.path(),
                &data.path().join("logs"),
                0,
                &report(),
                LIMITS
            )
            .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn source_root_nested_and_leaf_links_are_rejected() {
        use std::os::unix::fs::symlink;
        for location in ["root", "directory", "file", "dangling"] {
            let (scratch, data) = setup();
            let source = scratch.path().join(DIRECTORY);
            fs::write(data.path().join("sentinel"), "Synthetic private sentinel").unwrap();
            match location {
                "root" => {
                    fs::remove_dir(&source).unwrap();
                    symlink(data.path(), &source).unwrap();
                }
                "directory" => symlink(data.path(), source.join("linked")).unwrap(),
                "file" => symlink(data.path().join("sentinel"), source.join("linked")).unwrap(),
                _ => symlink(data.path().join("missing"), source.join("linked")).unwrap(),
            }
            assert!(
                retained(scratch.path(), data.path(), LIMITS).is_err(),
                "accepted {location}"
            );
            assert!(!data.path().join("verification-evidence/7").exists());
        }
    }

    #[cfg(unix)]
    #[test]
    fn hard_links_and_non_regular_files_are_rejected_without_blocking() {
        use std::os::unix::net::UnixListener;
        let (scratch, data) = setup();
        fs::write(data.path().join("sentinel"), "Synthetic sentinel").unwrap();
        fs::hard_link(
            data.path().join("sentinel"),
            scratch.path().join(DIRECTORY).join("link"),
        )
        .unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
        let (scratch, data) = setup();
        let _socket = UnixListener::bind(scratch.path().join(DIRECTORY).join("socket")).unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
        let (scratch, data) = setup();
        use std::os::unix::ffi::OsStrExt;
        let p = std::ffi::CString::new(
            scratch
                .path()
                .join(DIRECTORY)
                .join("fifo")
                .as_os_str()
                .as_bytes(),
        )
        .unwrap();
        // SAFETY: the C string is alive and the path is a synthetic private test directory.
        assert_eq!(unsafe { libc::mkfifo(p.as_ptr(), 0o600) }, 0);
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
    }

    #[test]
    fn snapshot_rejects_same_length_replacement_and_growth() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().canonicalize().unwrap().join("file");
        fs::write(&path, "aaaa").unwrap();
        let snapshot = Snapshot::read(&path).unwrap();
        fs::write(path.with_extension("replacement"), "bbbb").unwrap();
        fs::remove_file(&path).unwrap();
        fs::rename(path.with_extension("replacement"), &path).unwrap();
        assert!(read_file(&path, &snapshot, 8).is_err());
        let snapshot = Snapshot::read(&path).unwrap();
        fs::write(&path, "bbbb-more").unwrap();
        assert!(read_file(&path, &snapshot, 8).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn host_root_links_are_rejected_and_permissions_are_private() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let (scratch, data) = setup();
        let other = tempfile::tempdir().unwrap();
        symlink(other.path(), data.path().join("verification-evidence")).unwrap();
        assert!(retained(scratch.path(), data.path(), LIMITS).is_err());
        assert_eq!(fs::read_dir(other.path()).unwrap().count(), 0);
        let (scratch, data) = setup();
        fs::write(
            scratch.path().join(DIRECTORY).join("report.md"),
            "Synthetic",
        )
        .unwrap();
        let path = retained(scratch.path(), data.path(), LIMITS).unwrap();
        for file in [
            &path,
            &path.parent().unwrap().join("result.json"),
            &path.parent().unwrap().join("files/report.md"),
        ] {
            assert_eq!(
                fs::metadata(file).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        for dir in [
            path.parent().unwrap(),
            path.parent().unwrap().parent().unwrap(),
        ] {
            assert_eq!(
                fs::metadata(dir).unwrap().permissions().mode() & 0o777,
                0o700
            );
        }
    }

    #[test]
    fn publication_does_not_replace_an_existing_empty_directory() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let destination = root.path().join("destination");
        fs::create_dir(&source).unwrap();
        fs::write(source.join("payload"), "new").unwrap();
        fs::create_dir(&destination).unwrap();
        assert!(publish(&source, &destination).is_err());
        assert!(source.join("payload").is_file());
        assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
    }

    #[test]
    fn canonical_parent_chain_accepts_a_real_absolute_root() {
        let data = tempfile::tempdir().unwrap();
        check_parent_chain(&data.path().canonicalize().unwrap()).unwrap();
    }
}
