//! Putting the bundled `issuerelay` command on PATH without administrator rights (design 12),
//! and telling where the running app is installed. macOS: a symbolic link
//! `~/.local/bin/issuerelay` to the CLI inside the app bundle; a file IssueRelay did not make is
//! never overwritten and shell profiles are never edited (the panel shows the line to add).
//! Windows: the install directory in the user's own `Path` (`HKCU\Environment`), kept as the
//! registry type it had, compared without regard to case or spelling, never added twice;
//! removing takes out only that entry. The decisions are pure functions over what was read and
//! are tested on every platform; the reads and writes sit in `platform`.
use std::{
    path::{Path, PathBuf},
    string::FromUtf16Error,
};

use anyhow::{Context, Result};
use serde::Serialize;

/// The line the panel shows when `~/.local/bin` is not on the login shell's PATH (zsh, bash).
pub const SHELL_LINE: &str = r#"export PATH="$HOME/.local/bin:$PATH""#;
/// The longest value Windows allows for a user environment variable, in UTF-16 units.
pub const MAX_ENV_CHARS: usize = 32_767;

/// Where the running app is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AppLocation {
    /// Installed: the CLI next to it may be linked, and an update may replace it.
    Installed,
    /// macOS: run from the disk image or from Gatekeeper's randomized read-only copy (App
    /// Translocation). A link to it breaks and an update cannot replace it; the app should be
    /// moved to Applications first.
    NotInstalled,
    /// A development build: a debug build, or an executable outside an app bundle.
    Development,
}

/// `<…>/<Name>.app/Contents/MacOS/<file>`.
fn in_app_bundle(path: &Path) -> bool {
    let mut up = path.ancestors().skip(1);
    let (macos, contents, bundle) = (up.next(), up.next(), up.next());
    macos
        .and_then(Path::file_name)
        .is_some_and(|name| name == "MacOS")
        && contents
            .and_then(Path::file_name)
            .is_some_and(|name| name == "Contents")
        && bundle
            .and_then(Path::extension)
            .is_some_and(|ext| ext == "app")
}

/// macOS: where an executable is, judged by its path. `read_only` says that its volume is mounted
/// read-only, which is true of a disk image and of a translocated copy.
pub fn bundle_location(exe: &Path, read_only: bool) -> AppLocation {
    if !in_app_bundle(exe) {
        return AppLocation::Development;
    }
    let translocated = exe
        .components()
        .any(|part| part.as_os_str() == "AppTranslocation");
    if read_only || translocated {
        AppLocation::NotInstalled
    } else {
        AppLocation::Installed
    }
}

/// Where the app whose executable is `exe` runs from; `debug` is `cfg!(debug_assertions)`.
/// Windows has no disk image or translocation: a release build counts as installed.
pub fn app_location(exe: &Path, read_only: bool, debug: bool) -> AppLocation {
    if debug {
        AppLocation::Development
    } else if cfg!(target_os = "macos") {
        bundle_location(exe, read_only)
    } else {
        AppLocation::Installed
    }
}

/// Whether `path` is on a volume mounted read-only.
#[cfg(unix)]
pub fn on_read_only_volume(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: `c_path` is NUL-terminated and outlives the call; statvfs fills `stats` when it
    // returns 0, and `stats` is read only then.
    unsafe {
        libc::statvfs(c_path.as_ptr(), stats.as_mut_ptr()) == 0
            && stats.assume_init().f_flag & libc::ST_RDONLY != 0
    }
}

/// Whether `path` is on a volume mounted read-only. Windows installs never are.
#[cfg(windows)]
pub fn on_read_only_volume(_path: &Path) -> bool {
    false
}

/// `app_location` for this process.
pub fn running_app_location() -> AppLocation {
    match std::env::current_exe() {
        Ok(exe) => app_location(&exe, on_read_only_volume(&exe), cfg!(debug_assertions)),
        Err(e) => {
            tracing::warn!(error = %e, "cannot tell where the app runs from");
            AppLocation::Development
        }
    }
}

/// What sits where the link goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkSlot {
    Empty,
    /// A symbolic link, and the path it points to as written.
    Link(PathBuf),
    /// A file, a folder or anything else that is not a symbolic link.
    Other,
}

/// The link compared with the CLI it should point to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Missing,
    /// It points at this app's CLI.
    Ours,
    /// IssueRelay's link to another copy of the app (moved or reinstalled); it may be repointed.
    Stale(PathBuf),
    /// Not made by IssueRelay: never changed or removed.
    Foreign,
}

/// A CLI inside an app bundle, as IssueRelay links it.
fn is_bundled_cli(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name == "issuerelay") && in_app_bundle(path)
}

pub fn link_state(slot: &LinkSlot, cli: &Path) -> LinkState {
    match slot {
        LinkSlot::Empty => LinkState::Missing,
        LinkSlot::Link(target) if target == cli => LinkState::Ours,
        LinkSlot::Link(target) if is_bundled_cli(target) => LinkState::Stale(target.clone()),
        LinkSlot::Link(_) | LinkSlot::Other => LinkState::Foreign,
    }
}

/// `~/.local/bin/issuerelay`.
pub fn link_path(home: &Path) -> PathBuf {
    home.join(".local").join("bin").join("issuerelay")
}

/// `path` without trailing or doubled separators.
fn tidy(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Whether `dir` is an entry of the PATH value `path_var`. `~`, `$HOME` and `${HOME}` at the
/// start of an entry stand for `home`; a trailing `/` does not count.
pub fn unix_path_has(dir: &Path, path_var: &str, home: &Path) -> bool {
    let wanted = tidy(dir);
    path_var
        .split(':')
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .any(|entry| {
            let expanded = match ["~", "$HOME", "${HOME}"]
                .iter()
                .find_map(|prefix| entry.strip_prefix(prefix))
            {
                Some("") => home.to_path_buf(),
                Some(rest) if rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
                _ => PathBuf::from(entry),
            };
            tidy(&expanded) == wanted
        })
}

/// `%NAME%` references in `text` replaced by `lookup(NAME)`; unknown ones stay as written.
pub fn expand_vars(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) if end > 0 => {
                let name = &after[..end];
                match lookup(name) {
                    Some(value) => out.push_str(&value),
                    None => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// A `Path` entry as compared: references expanded, blanks, quotes and trailing separators
/// removed, lower case (Windows paths ignore case).
fn comparable(entry: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    expand_vars(entry.trim(), lookup)
        .trim()
        .trim_matches('"')
        .trim_end_matches(['\\', '/'])
        .to_lowercase()
}

/// Whether the user `Path` value names `dir`.
pub fn windows_path_has(dir: &str, value: &str, lookup: &dyn Fn(&str) -> Option<String>) -> bool {
    let wanted = comparable(dir, lookup);
    !wanted.is_empty()
        && value
            .split(';')
            .any(|entry| comparable(entry, lookup) == wanted)
}

/// The new value would be longer than Windows allows.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "the user Path would be {0} characters long, more than Windows allows ({MAX_ENV_CHARS}); it was not changed"
)]
pub struct PathTooLong(pub usize);

/// The user `Path` with `dir` appended, or `None` when it is already there. Every other entry
/// keeps its exact text, `%VAR%` references and empty entries included.
pub fn windows_path_with(
    dir: &str,
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Result<Option<String>, PathTooLong> {
    if windows_path_has(dir, value, lookup) {
        return Ok(None);
    }
    let joined = if value.is_empty() {
        dir.to_string()
    } else if value.ends_with(';') {
        format!("{value}{dir}")
    } else {
        format!("{value};{dir}")
    };
    let length = joined.encode_utf16().count();
    if length >= MAX_ENV_CHARS {
        return Err(PathTooLong(length));
    }
    Ok(Some(joined))
}

/// The user `Path` without the entries that name `dir`, or `None` when none does. The other
/// entries keep their exact text and order, empty ones included.
pub fn windows_path_without(
    dir: &str,
    value: &str,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Option<String> {
    let wanted = comparable(dir, lookup);
    if wanted.is_empty() {
        // Nothing to look for: the empty entries must not be taken for it.
        return None;
    }
    let entries: Vec<&str> = value.split(';').collect();
    let kept: Vec<&str> = entries
        .iter()
        .copied()
        .filter(|entry| comparable(entry, lookup) != wanted)
        .collect();
    (kept.len() != entries.len()).then(|| kept.join(";"))
}

/// Why a registry string is not taken as text.
#[derive(Debug, thiserror::Error)]
pub enum RegStringError {
    /// Not valid UTF-16 (a lone surrogate).
    #[error("it is not valid UTF-16")]
    NotUtf16(#[from] FromUtf16Error),
    /// Something follows the text that is not part of it: data after the terminating NUL (a list
    /// of strings, or what another program left there), or a stray byte at the end.
    #[error("it holds data after the end of the text")]
    TrailingData,
}

/// A registry string (UTF-16LE, usually NUL-terminated) as text. What is not plainly text is an
/// error, so that the value is never written back changed: invalid UTF-16, a stray byte at the
/// end, and anything but NULs after the first NUL (the text would be cut there and the rest lost).
pub fn decode_reg_string(bytes: &[u8]) -> Result<String, RegStringError> {
    let (pairs, stray) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs.iter().map(|pair| u16::from_le_bytes(*pair)).collect();
    let end = units
        .iter()
        .position(|&unit| unit == 0)
        .unwrap_or(units.len());
    if !stray.is_empty() || units[end..].iter().any(|&unit| unit != 0) {
        return Err(RegStringError::TrailingData);
    }
    Ok(String::from_utf16(&units[..end])?)
}

/// `text` as a NUL-terminated UTF-16LE registry string.
pub fn encode_reg_string(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .chain(std::iter::once(0))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// Why the CLI cannot be put on PATH from where the app runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CliUnavailable {
    /// No `issuerelay` next to the app.
    MissingCli,
    /// macOS: run from the disk image or a translocated copy.
    NotInstalled,
    /// A development build.
    Development,
}

/// What the settings page shows for "put the command line on PATH".
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CliPathStatus {
    /// IssueRelay has not put the CLI on PATH.
    Off,
    /// On PATH through IssueRelay. `shell_line` (macOS): `~/.local/bin` is not on the login
    /// shell's PATH, or that PATH could not be read; the panel shows this line to add.
    On { shell_line: Option<String> },
    /// macOS: `~/.local/bin/issuerelay` is IssueRelay's link to another copy of the app.
    Stale { target: String },
    /// macOS: `~/.local/bin/issuerelay` is something IssueRelay did not make; left alone.
    Blocked { path: String },
    /// Not possible from where the app runs.
    Unavailable { reason: CliUnavailable },
}

/// What the decisions read from the system, gathered by the caller.
#[derive(Debug, Clone)]
pub struct CliSetup {
    /// The CLI next to the app, when it is there.
    pub cli: Option<PathBuf>,
    pub location: AppLocation,
    /// The user's home folder (the macOS link lives in it).
    pub home: PathBuf,
    /// The login shell's PATH when it could be read (macOS).
    pub shell_path: Option<String>,
}

impl CliSetup {
    /// The setup of this process: where it runs from and the user's home folder.
    pub fn for_running_app(cli: Option<PathBuf>, shell_path: Option<String>) -> Result<Self> {
        Ok(Self {
            cli,
            location: running_app_location(),
            home: dirs::home_dir().context("this system has no home folder")?,
            shell_path,
        })
    }

    fn usable_cli(&self) -> Result<&Path, CliUnavailable> {
        match self.location {
            AppLocation::Development => Err(CliUnavailable::Development),
            AppLocation::NotInstalled => Err(CliUnavailable::NotInstalled),
            AppLocation::Installed => self.cli.as_deref().ok_or(CliUnavailable::MissingCli),
        }
    }
}

/// Whether the CLI is on PATH through IssueRelay.
pub fn status(setup: &CliSetup) -> Result<CliPathStatus> {
    match setup.usable_cli() {
        Ok(cli) => platform::status(setup, cli),
        Err(reason) => Ok(CliPathStatus::Unavailable { reason }),
    }
}

/// Puts the CLI on PATH; returns the new status (a blocked or unavailable one changes nothing).
pub fn install(setup: &CliSetup) -> Result<CliPathStatus> {
    match setup.usable_cli() {
        Ok(cli) => platform::install(setup, cli),
        Err(reason) => Ok(CliPathStatus::Unavailable { reason }),
    }
}

/// Takes the CLI off PATH (only what IssueRelay added); returns the new status.
pub fn remove(setup: &CliSetup) -> Result<CliPathStatus> {
    platform::remove(setup)?;
    status(setup)
}

#[cfg(unix)]
mod platform {
    use std::io;

    use super::*;

    fn read_slot(link: &Path) -> io::Result<LinkSlot> {
        match std::fs::symlink_metadata(link) {
            Ok(meta) if meta.file_type().is_symlink() => {
                Ok(LinkSlot::Link(std::fs::read_link(link)?))
            }
            Ok(_) => Ok(LinkSlot::Other),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(LinkSlot::Empty),
            Err(e) => Err(e),
        }
    }

    pub fn status(setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let link = link_path(&setup.home);
        let slot = read_slot(&link).with_context(|| format!("cannot read {}", link.display()))?;
        Ok(match link_state(&slot, cli) {
            LinkState::Missing => CliPathStatus::Off,
            LinkState::Ours => {
                let bin = link.parent().expect("the link sits in a folder");
                let reachable = setup
                    .shell_path
                    .as_deref()
                    .is_some_and(|path| unix_path_has(bin, path, &setup.home));
                CliPathStatus::On {
                    shell_line: (!reachable).then(|| SHELL_LINE.to_string()),
                }
            }
            LinkState::Stale(target) => CliPathStatus::Stale {
                target: target.display().to_string(),
            },
            LinkState::Foreign => CliPathStatus::Blocked {
                path: link.display().to_string(),
            },
        })
    }

    pub fn install(setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let link = link_path(&setup.home);
        let bin = link.parent().expect("the link sits in a folder");
        let slot = read_slot(&link).with_context(|| format!("cannot read {}", link.display()))?;
        match link_state(&slot, cli) {
            LinkState::Ours | LinkState::Foreign => {}
            LinkState::Missing => {
                std::fs::create_dir_all(bin)
                    .with_context(|| format!("cannot create {}", bin.display()))?;
                std::os::unix::fs::symlink(cli, &link)
                    .with_context(|| format!("cannot create {}", link.display()))?;
            }
            LinkState::Stale(_) => {
                // In one step: a new link under a temporary name, renamed over the old one, so
                // that there is no moment without a link. The slot was read above and is replaced
                // here: if another program put its own file at this exact path in between, the
                // rename would replace it. That check-then-act window is accepted: it is a race
                // of microseconds against a program of the same user, and neither `rename` nor
                // `unlink` can say "only if it is still this link". (A link where there was none
                // cannot overwrite anything: `symlink` fails when the path exists.)
                let temporary = bin.join(format!(".issuerelay-link-{}", std::process::id()));
                let _ = std::fs::remove_file(&temporary);
                std::os::unix::fs::symlink(cli, &temporary)
                    .with_context(|| format!("cannot create {}", temporary.display()))?;
                if let Err(e) = std::fs::rename(&temporary, &link) {
                    // Do not leave the temporary link behind.
                    let _ = std::fs::remove_file(&temporary);
                    return Err(e).with_context(|| format!("cannot replace {}", link.display()));
                }
            }
        }
        status(setup, cli)
    }

    pub fn remove(setup: &CliSetup) -> Result<()> {
        let link = link_path(&setup.home);
        // The same accepted check-then-act window as in `install`: the link is read here and
        // removed below, and `remove_file` takes away whatever sits at the path by then, so a
        // file another program put there in between would go too.
        if let LinkSlot::Link(target) =
            read_slot(&link).with_context(|| format!("cannot read {}", link.display()))?
            && (setup.cli.as_deref() == Some(target.as_path()) || is_bundled_cli(&target))
        {
            std::fs::remove_file(&link)
                .with_context(|| format!("cannot remove {}", link.display()))?;
        }
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use std::io;

    use winreg::{
        RegKey, RegValue,
        enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, RegType},
    };

    use super::*;

    fn lookup(name: &str) -> Option<String> {
        std::env::var(name).ok()
    }

    /// The user's environment key. Only a change asks for the right to write: reading how things
    /// stand must work where writing is not allowed.
    fn open(write: bool) -> Result<RegKey> {
        let access = if write {
            KEY_READ | KEY_SET_VALUE
        } else {
            KEY_READ
        };
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Environment", access)
            .context("cannot open the user environment (HKCU\\Environment)")
    }

    /// The user `Path` as stored, and its registry type; a missing value reads as an empty
    /// `REG_EXPAND_SZ`. Never the expanded process PATH.
    fn read(key: &RegKey) -> Result<(String, RegType)> {
        match key.get_raw_value("Path") {
            Ok(value) => {
                anyhow::ensure!(
                    matches!(value.vtype, RegType::REG_SZ | RegType::REG_EXPAND_SZ),
                    "the user Path has an unexpected registry type; it was not changed"
                );
                let text = decode_reg_string(&value.bytes)
                    .context("the user Path is not valid text; it was not changed")?;
                Ok((text, value.vtype))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                Ok((String::new(), RegType::REG_EXPAND_SZ))
            }
            Err(e) => Err(e).context("cannot read the user Path"),
        }
    }

    /// Writes the user `Path` with the type it had and tells running programs.
    fn write(key: &RegKey, text: &str, vtype: RegType) -> Result<()> {
        key.set_raw_value(
            "Path",
            &RegValue {
                bytes: encode_reg_string(text).into(),
                vtype,
            },
        )
        .context("cannot write the user Path")?;
        broadcast();
        Ok(())
    }

    /// Tells running programs (Explorer, and so new terminals) that the environment changed.
    fn broadcast() {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            HWND_BROADCAST, SMTO_ABORTIFHUNG, SendMessageTimeoutW, WM_SETTINGCHANGE,
        };
        let name: Vec<u16> = "Environment"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let mut result = 0usize;
        // SAFETY: `name` is NUL-terminated and outlives the call; `result` is a valid out pointer.
        let sent = unsafe {
            SendMessageTimeoutW(
                HWND_BROADCAST,
                WM_SETTINGCHANGE,
                0,
                name.as_ptr() as isize,
                SMTO_ABORTIFHUNG,
                5000,
                &mut result,
            )
        };
        if sent == 0 {
            tracing::warn!(
                "could not tell running programs that PATH changed; terminals opened later may need a new sign-in"
            );
        }
    }

    fn dir_of(cli: &Path) -> Result<String> {
        Ok(cli
            .parent()
            .context("the CLI has no folder")?
            .display()
            .to_string())
    }

    pub fn status(_setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let (value, _) = read(&open(false)?)?;
        Ok(if windows_path_has(&dir_of(cli)?, &value, &lookup) {
            CliPathStatus::On { shell_line: None }
        } else {
            CliPathStatus::Off
        })
    }

    pub fn install(setup: &CliSetup, cli: &Path) -> Result<CliPathStatus> {
        let key = open(true)?;
        let (value, vtype) = read(&key)?;
        if let Some(changed) = windows_path_with(&dir_of(cli)?, &value, &lookup)? {
            write(&key, &changed, vtype)?;
        }
        status(setup, cli)
    }

    pub fn remove(setup: &CliSetup) -> Result<()> {
        let Some(cli) = setup.cli.as_deref() else {
            return Ok(());
        };
        let key = open(true)?;
        let (value, vtype) = read(&key)?;
        if let Some(changed) = windows_path_without(&dir_of(cli)?, &value, &lookup) {
            write(&key, &changed, vtype)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::*;

    const INSTALLED: &str = "/Applications/IssueRelay.app/Contents/MacOS/issuerelay-app";
    const CLI: &str = "/Applications/IssueRelay.app/Contents/MacOS/issuerelay";

    #[test]
    fn only_an_app_bundle_on_a_writable_volume_counts_as_installed() {
        assert_eq!(
            bundle_location(Path::new(INSTALLED), false),
            AppLocation::Installed
        );
        // The user's own Applications folder is just as good.
        let personal = "/tmp/demo-home/Applications/IssueRelay.app/Contents/MacOS/issuerelay-app";
        assert_eq!(
            bundle_location(Path::new(personal), false),
            AppLocation::Installed
        );
        // A mounted disk image is read-only; so is Gatekeeper's randomized copy.
        assert_eq!(
            bundle_location(Path::new(INSTALLED), true),
            AppLocation::NotInstalled
        );
        let translocated = "/private/var/folders/ab/xyz/T/AppTranslocation/0A1B2C3D/d/IssueRelay.app/Contents/MacOS/issuerelay-app";
        assert_eq!(
            bundle_location(Path::new(translocated), false),
            AppLocation::NotInstalled
        );
        // Outside an app bundle: a development build.
        assert_eq!(
            bundle_location(Path::new("/tmp/demo/target/release/issuerelay-app"), false),
            AppLocation::Development
        );
        assert_eq!(
            bundle_location(Path::new("/tmp/demo/Contents/MacOS/issuerelay-app"), false),
            AppLocation::Development
        );
        // A debug build never counts as installed, wherever it is.
        assert_eq!(
            app_location(Path::new(INSTALLED), false, true),
            AppLocation::Development
        );
    }

    #[test]
    fn the_link_is_ours_stale_or_someone_elses() {
        let cli = Path::new(CLI);
        assert_eq!(link_state(&LinkSlot::Empty, cli), LinkState::Missing);
        assert_eq!(
            link_state(&LinkSlot::Link(PathBuf::from(CLI)), cli),
            LinkState::Ours
        );
        // IssueRelay's link to a copy that was moved or reinstalled elsewhere.
        let old =
            PathBuf::from("/tmp/demo-home/Downloads/IssueRelay.app/Contents/MacOS/issuerelay");
        assert_eq!(
            link_state(&LinkSlot::Link(old.clone()), cli),
            LinkState::Stale(old)
        );
        // Anything else is never touched: another program's link, or a plain file.
        assert_eq!(
            link_state(
                &LinkSlot::Link(PathBuf::from("/opt/demo/bin/issuerelay")),
                cli
            ),
            LinkState::Foreign
        );
        let other_app = PathBuf::from("/Applications/Other.app/Contents/MacOS/other");
        assert_eq!(
            link_state(&LinkSlot::Link(other_app), cli),
            LinkState::Foreign
        );
        assert_eq!(link_state(&LinkSlot::Other, cli), LinkState::Foreign);
        assert_eq!(
            link_path(Path::new("/tmp/demo-home")),
            Path::new("/tmp/demo-home/.local/bin/issuerelay")
        );
    }

    #[test]
    fn the_shell_path_is_read_with_tilde_home_and_trailing_slashes() {
        let home = Path::new("/tmp/demo-home");
        let bin = Path::new("/tmp/demo-home/.local/bin");
        for path in [
            "/usr/bin:/tmp/demo-home/.local/bin:/bin",
            "/tmp/demo-home/.local/bin/",
            "~/.local/bin:/usr/bin",
            "$HOME/.local/bin",
            "${HOME}/.local/bin",
            "/usr/bin:/tmp/demo-home//.local/bin",
        ] {
            assert!(unix_path_has(bin, path, home), "{path}");
        }
        for path in [
            "/usr/bin:/bin",
            "/tmp/demo-home/.local/bin2",
            "/tmp/demo-home/.local",
            "",
        ] {
            assert!(!unix_path_has(bin, path, home), "{path}");
        }
    }

    fn lookup(name: &str) -> Option<String> {
        match name.to_ascii_uppercase().as_str() {
            "LOCALAPPDATA" => Some(r"D:\Profiles\demo\AppData\Local".into()),
            "USERPROFILE" => Some(r"D:\Profiles\demo".into()),
            _ => None,
        }
    }

    const INSTALL_DIR: &str = r"D:\Profiles\demo\AppData\Local\IssueRelay";

    #[test]
    fn environment_references_expand_and_unknown_ones_stay() {
        assert_eq!(
            expand_vars(r"%LOCALAPPDATA%\IssueRelay", &lookup),
            INSTALL_DIR
        );
        assert_eq!(
            expand_vars(r"%localappdata%\IssueRelay", &lookup),
            INSTALL_DIR
        );
        assert_eq!(expand_vars(r"%NOPE%\bin", &lookup), r"%NOPE%\bin");
        assert_eq!(expand_vars("50%", &lookup), "50%");
        assert_eq!(expand_vars("%%", &lookup), "%%");
    }

    #[test]
    fn the_install_dir_is_found_however_it_is_written() {
        for value in [
            r"%LOCALAPPDATA%\IssueRelay",
            r"d:\profiles\DEMO\appdata\local\issuerelay\",
            r#""D:\Profiles\demo\AppData\Local\IssueRelay""#,
            r"C:\Tools; D:\Profiles\demo\AppData\Local\IssueRelay ;",
        ] {
            assert!(windows_path_has(INSTALL_DIR, value, &lookup), "{value}");
        }
        assert!(!windows_path_has(
            INSTALL_DIR,
            r"D:\Profiles\demo\AppData\Local\IssueRelay2",
            &lookup
        ));
        assert!(!windows_path_has(INSTALL_DIR, "", &lookup));
    }

    #[test]
    fn adding_appends_once_and_keeps_every_other_entry_as_written() {
        let value = r"%USERPROFILE%\bin;;C:\Tools";
        assert_eq!(
            windows_path_with(INSTALL_DIR, value, &lookup).unwrap(),
            Some(format!(r"%USERPROFILE%\bin;;C:\Tools;{INSTALL_DIR}"))
        );
        assert_eq!(
            windows_path_with(INSTALL_DIR, r"C:\Tools;", &lookup).unwrap(),
            Some(format!(r"C:\Tools;{INSTALL_DIR}"))
        );
        assert_eq!(
            windows_path_with(INSTALL_DIR, "", &lookup).unwrap(),
            Some(INSTALL_DIR.to_string())
        );
        // Already there under another spelling: nothing to write.
        assert_eq!(
            windows_path_with(INSTALL_DIR, r"%LOCALAPPDATA%\IssueRelay\", &lookup).unwrap(),
            None
        );
        // Windows allows 32,767 characters per user variable.
        let long = "x".repeat(MAX_ENV_CHARS - 10);
        assert!(matches!(
            windows_path_with(INSTALL_DIR, &long, &lookup),
            Err(PathTooLong(_))
        ));
    }

    #[test]
    fn a_path_that_would_not_fit_is_refused_and_says_how_long_it_would_be() {
        // The new value and its terminating NUL must fit in 32,767 characters.
        let value_of = |joined: usize| "x".repeat(joined - INSTALL_DIR.len() - 1);
        let fits = windows_path_with(INSTALL_DIR, &value_of(MAX_ENV_CHARS - 1), &lookup);
        assert_eq!(fits.unwrap().unwrap().len(), MAX_ENV_CHARS - 1);
        let too_long = windows_path_with(INSTALL_DIR, &value_of(MAX_ENV_CHARS), &lookup);
        assert_eq!(too_long, Err(PathTooLong(MAX_ENV_CHARS)));
        assert_eq!(
            PathTooLong(40_000).to_string(),
            "the user Path would be 40000 characters long, more than Windows allows (32767); it was not changed"
        );
    }

    #[test]
    fn removing_takes_out_only_the_install_dir() {
        let value =
            format!(r"%USERPROFILE%\bin;{INSTALL_DIR};;C:\Tools;%LOCALAPPDATA%\IssueRelay\;");
        assert_eq!(
            windows_path_without(INSTALL_DIR, &value, &lookup),
            Some(r"%USERPROFILE%\bin;;C:\Tools;".to_string())
        );
        assert_eq!(
            windows_path_without(INSTALL_DIR, INSTALL_DIR, &lookup),
            Some(String::new())
        );
        assert_eq!(
            windows_path_without(INSTALL_DIR, r"C:\Tools", &lookup),
            None
        );
    }

    #[test]
    fn registry_strings_round_trip_and_bad_text_is_refused() {
        let encoded = encode_reg_string(r"%USERPROFILE%\bin;C:\Tools");
        assert_eq!(&encoded[encoded.len() - 2..], &[0, 0], "NUL-terminated");
        assert_eq!(
            decode_reg_string(&encoded).unwrap(),
            r"%USERPROFILE%\bin;C:\Tools"
        );
        // Without the terminator the text is the same.
        assert_eq!(
            decode_reg_string(&encoded[..encoded.len() - 2]).unwrap(),
            r"%USERPROFILE%\bin;C:\Tools"
        );
        // A lone surrogate is not text: the value is left alone rather than rewritten lossily.
        assert!(decode_reg_string(&[0x00, 0xd8, 0x41, 0x00]).is_err());
    }

    #[test]
    fn a_registry_string_with_more_after_its_end_is_refused_not_cut() {
        let units =
            |text: &str| -> Vec<u8> { text.encode_utf16().flat_map(u16::to_le_bytes).collect() };
        // "A", the terminator, then "B": decoding would stop at the NUL, and writing the text back
        // would lose the "B".
        let mut more = units("A");
        more.extend([0, 0]);
        more.extend(units("B"));
        assert!(matches!(
            decode_reg_string(&more),
            Err(RegStringError::TrailingData)
        ));
        // A list of strings ("a", NUL, "b", NUL, NUL) is such a value too.
        let mut list = units("a");
        list.extend([0, 0]);
        list.extend(units("b"));
        list.extend([0, 0, 0, 0]);
        assert!(matches!(
            decode_reg_string(&list),
            Err(RegStringError::TrailingData)
        ));
        // A byte that makes no whole character is no text either.
        let mut stray = units("A");
        stray.push(0x42);
        assert!(matches!(
            decode_reg_string(&stray),
            Err(RegStringError::TrailingData)
        ));
        // Padding is nothing: only NULs follow the terminator.
        let mut padded = units("A");
        padded.extend([0, 0, 0, 0, 0, 0]);
        assert_eq!(decode_reg_string(&padded).unwrap(), "A");
        // A lone surrogate keeps its own reason.
        assert!(matches!(
            decode_reg_string(&[0x00, 0xd8, 0x41, 0x00]),
            Err(RegStringError::NotUtf16(_))
        ));
        // Both read as one sentence about the user Path in the panel.
        let reason = decode_reg_string(&more).unwrap_err().to_string();
        assert_eq!(reason, "it holds data after the end of the text");
    }

    #[test]
    fn the_status_reads_as_the_panel_expects() {
        let on = CliPathStatus::On {
            shell_line: Some(SHELL_LINE.into()),
        };
        assert_eq!(
            serde_json::to_value(on).unwrap(),
            serde_json::json!({"kind": "on", "shell_line": "export PATH=\"$HOME/.local/bin:$PATH\""})
        );
        let unavailable = CliPathStatus::Unavailable {
            reason: CliUnavailable::NotInstalled,
        };
        assert_eq!(
            serde_json::to_value(unavailable).unwrap(),
            serde_json::json!({"kind": "unavailable", "reason": "not_installed"})
        );
    }

    // Unix only: on Windows `on_read_only_volume` is a constant `false`, nothing to test there.
    #[cfg(unix)]
    #[test]
    fn a_read_only_volume_is_told_from_a_writable_one() {
        let root = tempfile::tempdir().unwrap();
        assert!(!on_read_only_volume(root.path()));
        // Nothing is known about a path that is not there: it does not count as read-only.
        assert!(!on_read_only_volume(&root.path().join("missing")));
        assert!(!on_read_only_volume(Path::new("")));
        // The sealed system volume of macOS is mounted read-only (probed on a mounted disk image
        // as well, which this test cannot make).
        #[cfg(target_os = "macos")]
        assert!(on_read_only_volume(Path::new("/System")));
    }

    // Unix only, with no Windows counterpart on purpose: these tests link in a temporary home,
    // while the Windows reads and writes go to the real registry (`HKCU\Environment`), which no
    // test touches. The pure functions above (the `Path` comparison, adding and removing the
    // folder, the registry string codec) carry the Windows rules and run on every platform.
    #[cfg(unix)]
    mod link {
        use super::*;

        /// A fake installed app with its CLI, and a home without `~/.local/bin`.
        fn setup(root: &Path, shell_path: Option<&str>) -> CliSetup {
            let macos = root.join("Applications/IssueRelay.app/Contents/MacOS");
            std::fs::create_dir_all(&macos).unwrap();
            let cli = macos.join("issuerelay");
            std::fs::write(&cli, "cli").unwrap();
            let home = root.join("home");
            std::fs::create_dir_all(&home).unwrap();
            CliSetup {
                cli: Some(cli),
                location: AppLocation::Installed,
                home,
                shell_path: shell_path.map(str::to_string),
            }
        }

        #[test]
        fn turning_it_on_links_the_cli_and_says_what_the_shell_still_needs() {
            let root = tempfile::tempdir().unwrap();
            let setup = setup(root.path(), Some("/usr/bin:/bin"));
            assert_eq!(status(&setup).unwrap(), CliPathStatus::Off);
            let on = install(&setup).unwrap();
            assert_eq!(
                on,
                CliPathStatus::On {
                    shell_line: Some(SHELL_LINE.to_string())
                }
            );
            let link = link_path(&setup.home);
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                setup.cli.clone().unwrap()
            );
            // Doing it again changes nothing.
            assert_eq!(install(&setup).unwrap(), on);
            // With ~/.local/bin on the login shell's PATH nothing is left to do.
            let bin = setup.home.join(".local/bin");
            let reachable = CliSetup {
                shell_path: Some(format!("/usr/bin:{}", bin.display())),
                ..setup.clone()
            };
            assert_eq!(
                status(&reachable).unwrap(),
                CliPathStatus::On { shell_line: None }
            );
            // An unreadable login shell PATH shows the line too.
            let unknown = CliSetup {
                shell_path: None,
                ..setup.clone()
            };
            assert_eq!(
                status(&unknown).unwrap(),
                CliPathStatus::On {
                    shell_line: Some(SHELL_LINE.to_string())
                }
            );
            assert_eq!(remove(&setup).unwrap(), CliPathStatus::Off);
            assert!(std::fs::symlink_metadata(&link).is_err());
        }

        #[test]
        fn a_file_issuerelay_did_not_make_is_never_touched() {
            let root = tempfile::tempdir().unwrap();
            let setup = setup(root.path(), None);
            let link = link_path(&setup.home);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            std::fs::write(&link, "someone else's script").unwrap();
            let blocked = CliPathStatus::Blocked {
                path: link.display().to_string(),
            };
            assert_eq!(install(&setup).unwrap(), blocked);
            assert_eq!(remove(&setup).unwrap(), blocked);
            assert_eq!(
                std::fs::read_to_string(&link).unwrap(),
                "someone else's script"
            );
            // Another program's link is left alone as well.
            std::fs::remove_file(&link).unwrap();
            std::os::unix::fs::symlink("/opt/demo/bin/issuerelay", &link).unwrap();
            assert_eq!(install(&setup).unwrap(), blocked);
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                Path::new("/opt/demo/bin/issuerelay")
            );
        }

        #[test]
        fn a_link_to_a_moved_copy_is_repointed() {
            let root = tempfile::tempdir().unwrap();
            let setup = setup(root.path(), None);
            let link = link_path(&setup.home);
            std::fs::create_dir_all(link.parent().unwrap()).unwrap();
            let old = root
                .path()
                .join("Downloads/IssueRelay.app/Contents/MacOS/issuerelay");
            std::os::unix::fs::symlink(&old, &link).unwrap();
            assert_eq!(
                status(&setup).unwrap(),
                CliPathStatus::Stale {
                    target: old.display().to_string()
                }
            );
            install(&setup).unwrap();
            assert_eq!(
                std::fs::read_link(&link).unwrap(),
                setup.cli.clone().unwrap()
            );
            // Only the link itself is left in the folder: the temporary one the swap is made with
            // (`.issuerelay-link-*`) is gone.
            let names: Vec<_> = std::fs::read_dir(link.parent().unwrap())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            assert_eq!(names, ["issuerelay"], "a temporary link was left behind");
        }

        #[test]
        fn nothing_is_linked_from_a_disk_image_a_development_build_or_without_the_cli() {
            let root = tempfile::tempdir().unwrap();
            let base = setup(root.path(), None);
            for (location, cli, reason) in [
                (
                    AppLocation::NotInstalled,
                    base.cli.clone(),
                    CliUnavailable::NotInstalled,
                ),
                (
                    AppLocation::Development,
                    base.cli.clone(),
                    CliUnavailable::Development,
                ),
                (AppLocation::Installed, None, CliUnavailable::MissingCli),
            ] {
                let setup = CliSetup {
                    location,
                    cli,
                    ..base.clone()
                };
                assert_eq!(
                    install(&setup).unwrap(),
                    CliPathStatus::Unavailable { reason }
                );
                assert!(std::fs::symlink_metadata(link_path(&setup.home)).is_err());
            }
        }
    }
}
