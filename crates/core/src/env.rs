//! The environment of executor runs and check commands: the PATH `claude` and `codex` are
//! found on, and the proxy they reach Linear through.
//!
//! A desktop app started from Finder or at login gets launchd's minimal PATH, without Homebrew
//! or npm's global directory, and Claude Code ignores the macOS system proxy: it reads only
//! `HTTPS_PROXY` and its relatives (design section 8; M1 acceptance H1). A Windows program keeps
//! the PATH it was started with, so an executor installed while it runs is only found through
//! the registry, where installers add their folders. `resolve` looks at the host's own
//! environment, then the user's login shell (Unix) or the PATH in the registry (Windows), then
//! the system proxy settings, and `ExecEnv::apply` hands the result to every child.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::ffi::OsString;
use std::future::Future;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
#[cfg(unix)]
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Proxy variables in both spellings: Claude Code and Codex read the upper-case names, curl and
/// many other tools the lower-case ones.
pub const PROXY_VARS: [&str; 8] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
    "NO_PROXY",
    "no_proxy",
];

/// The variables that name a proxy; `NO_PROXY` alone sets none.
const PROXY_SETTING_VARS: [&str; 6] = [
    "HTTPS_PROXY",
    "https_proxy",
    "HTTP_PROXY",
    "http_proxy",
    "ALL_PROXY",
    "all_proxy",
];

/// Removed from every child. A host started from inside a Claude Code session inherits them, and
/// Claude then treats the run as a nested session (M1 final review).
pub const REMOVED_VARS: [&str; 2] = ["CLAUDECODE", "CLAUDE_CODE_ENTRYPOINT"];

/// Hosts that never go through the proxy; every `NO_PROXY` built from system settings has them.
const LOCAL_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "::1"];

/// How long the login shell may take to print its environment. An interactive shell runs the
/// user's rc files, which can be slow (version managers, prompts) or hang.
#[cfg(unix)]
const SHELL_TIMEOUT: Duration = Duration::from_secs(10);

/// How long `scutil --proxy` may take.
#[cfg(target_os = "macos")]
const SCUTIL_TIMEOUT: Duration = Duration::from_secs(5);

/// Printed before and after `env` so that whatever the rc files print is ignored.
const BEGIN: &str = "__ISSUERELAY_ENV_BEGIN__";
const END: &str = "__ISSUERELAY_ENV_END__";

/// The shell used when `SHELL` is not set.
#[cfg(target_os = "macos")]
const DEFAULT_SHELL: &str = "/bin/zsh";
#[cfg(all(unix, not(target_os = "macos")))]
const DEFAULT_SHELL: &str = "/bin/sh";

/// PATH and proxy variables by name.
pub type Vars = BTreeMap<String, String>;

/// Where a part of the environment came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvSource {
    /// The host's own environment, passed on unchanged.
    Host,
    /// The user's login shell.
    LoginShell,
    /// The operating system's settings: the proxy settings, and on Windows the PATH in the
    /// registry.
    SystemSettings,
    /// Nowhere: no proxy is set.
    None,
}

/// The environment children get on top of the host's own.
///
/// `Debug` prints the proxy variables masked (`proxy_for_display`), so a log line or a failed
/// assertion that shows an environment, or a report holding one, cannot show a password.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecEnv {
    /// The PATH children get and executors are looked up on; `None` keeps the host's.
    pub path: Option<String>,
    pub path_source: EnvSource,
    /// Proxy variables set on children; empty when the host's own are passed on or none exist.
    pub proxy: Vec<(String, String)>,
    pub proxy_source: EnvSource,
    /// What could not be read, for the environment check.
    pub problems: Vec<String>,
}

/// One environment, shared by the job runner and the readiness checks; the environment check
/// replaces it with a freshly resolved one.
pub type SharedEnv = Arc<RwLock<ExecEnv>>;

impl std::fmt::Debug for ExecEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecEnv")
            .field("path", &self.path)
            .field("path_source", &self.path_source)
            .field("proxy", &self.proxy_for_display())
            .field("proxy_source", &self.proxy_source)
            .field("problems", &self.problems)
            .finish()
    }
}

impl ExecEnv {
    /// The host's environment as it is, without any lookup.
    pub fn inherit() -> Self {
        Self {
            path: None,
            path_source: EnvSource::Host,
            proxy: Vec::new(),
            proxy_source: EnvSource::Host,
            problems: Vec::new(),
        }
    }

    /// Wraps `self` for sharing between the job runner and the readiness checks.
    pub fn shared(self) -> SharedEnv {
        Arc::new(RwLock::new(self))
    }

    /// Sets the PATH and the proxy variables on `command` and removes `REMOVED_VARS`; everything
    /// else is inherited from the host as before.
    pub fn apply(&self, command: &mut tokio::process::Command) {
        if let Some(path) = &self.path {
            command.env("PATH", path);
        }
        for (name, value) in &self.proxy {
            command.env(name, value);
        }
        for name in REMOVED_VARS {
            command.env_remove(name);
        }
    }

    /// The proxy variables for display, with any user name and password in a URL masked.
    pub fn proxy_for_display(&self) -> Vec<(String, String)> {
        self.proxy
            .iter()
            .map(|(name, value)| (name.clone(), mask_credentials(value)))
            .collect()
    }

    /// The PATH, when `source` is where it came from.
    fn path_from(&self, source: EnvSource) -> Option<&str> {
        self.path.as_deref().filter(|_| self.path_source == source)
    }

    /// The proxy variables, when `source` is where they came from; none otherwise.
    fn proxy_from(&self, source: EnvSource) -> &[(String, String)] {
        if self.proxy_source == source {
            &self.proxy
        } else {
            &[]
        }
    }
}

/// `url` with its user-info part (`user:password@`) replaced by `***@`. The user-info ends at
/// the last `@` before the first `/`, so a password that holds an `@` is masked whole.
///
/// What stands in front of the first `://` is the scheme and is kept as it is, whatever it holds
/// (a quote, a blank, a `NAME=` prefix), unless it holds an `@` or a `/`: then the `://` belongs
/// to a path or a query, and the value is read from its start, with no scheme (curl also accepts
/// `user:password@host:port` as a proxy). Text without such an `@` is returned as it is. Masking
/// a masked value changes nothing.
pub fn mask_credentials(url: &str) -> String {
    let (prefix, rest) = match url.split_once("://") {
        Some((before, rest)) if !before.contains(['@', '/']) => {
            (&url[..before.len() + "://".len()], rest)
        }
        _ => ("", url),
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let Some(at) = rest[..authority_end].rfind('@') else {
        return url.to_string();
    };
    format!("{prefix}***@{}", &rest[at + 1..])
}

fn interesting(name: &str) -> bool {
    name == "PATH" || PROXY_VARS.contains(&name)
}

fn has_proxy(vars: &Vars) -> bool {
    PROXY_SETTING_VARS
        .iter()
        .any(|name| vars.get(*name).is_some_and(|v| !v.trim().is_empty()))
}

/// The host's own PATH and proxy variables.
fn host_vars() -> Vars {
    host_vars_from(std::env::vars_os(), !cfg!(windows))
}

/// The PATH and proxy variables of the environment `vars`. Where variable names have no case
/// (`names_have_case` false: Windows), PATH is found in any spelling and kept as `PATH`: Windows
/// spells it `Path`, the name of the registry value. Variables that are not valid Unicode are
/// skipped (`std::env::vars` would panic on any such variable, even an unrelated one).
fn host_vars_from(vars: impl Iterator<Item = (OsString, OsString)>, names_have_case: bool) -> Vars {
    vars.filter_map(|(name, value)| {
        let name = name.into_string().ok()?;
        let name = if !names_have_case && name.eq_ignore_ascii_case("PATH") {
            "PATH".to_string()
        } else if interesting(&name) {
            name
        } else {
            return None;
        };
        Some((name, value.into_string().ok()?))
    })
    .collect()
}

/// The machine's and the user's PATH from the operating system's settings, in that order (Windows:
/// the `Path` values in the registry, expanded). Unix keeps no PATH there, the login shell is
/// where it is set: `(None, None)`.
pub(crate) type SystemPath = (Option<String>, Option<String>);

/// What each source of the environment gave, read once (`read_sources`); `combine` puts them
/// together.
pub(crate) struct Sources {
    /// The host's own PATH and proxy variables.
    pub(crate) host: Vars,
    /// The login shell's PATH and proxy variables; always empty on Windows, which has none.
    pub(crate) shell: Result<Vars, String>,
    /// The proxy variables for the operating system's proxy settings.
    pub(crate) system_proxy: Result<Vec<(String, String)>, String>,
    /// The PATH in the operating system's settings.
    pub(crate) system_path: Result<SystemPath, String>,
}

/// Works out the environment children should get (see the module documentation). Never fails:
/// what could not be read is listed in `problems`.
pub async fn resolve() -> ExecEnv {
    combine(&read_sources().await, None)
}

/// Reads every source of the environment once.
pub(crate) async fn read_sources() -> Sources {
    read_sources_from(
        host_vars(),
        login_shell_vars(),
        system_proxy(),
        system_path(),
    )
    .await
}

/// The sources as the given reads find them. The login shell and the system settings are read
/// together: any of them can be slow (the shell runs the user's rc files), and a fresh
/// environment check waits for this while it holds the readiness cache's lock.
async fn read_sources_from(
    host: Vars,
    shell: impl Future<Output = Result<Vars, String>>,
    system_proxy: impl Future<Output = Result<Vec<(String, String)>, String>>,
    system_path: impl Future<Output = Result<SystemPath, String>>,
) -> Sources {
    let (shell, system_proxy, system_path) = tokio::join!(shell, system_proxy, system_path);
    Sources {
        host,
        shell,
        system_proxy,
        system_path,
    }
}

/// Puts the sources together. PATH: the login shell's, followed by the host's entries it
/// lacks; else the system settings' (Windows: the machine's and the user's `Path`), followed by
/// the host's entries they lack (`windows_path_from`); else the host's own. Proxy: the host's
/// own when it sets one, else the login shell's, else the system settings'.
///
/// A source that cannot be read is listed in `problems`. Given `previous`, the environment this
/// one replaces, such a source keeps what it gave there (the PATH or the proxy whose source it
/// was), and the problem says so: one slow rc file during a re-check must not swap a good PATH
/// for the host's own, which for an app started from Finder is launchd's minimal one. A source
/// that gave nothing there falls back as on a first read.
pub(crate) fn combine(sources: &Sources, previous: Option<&ExecEnv>) -> ExecEnv {
    let host = &sources.host;
    let mut problems = Vec::new();
    let shell = match &sources.shell {
        Ok(vars) => vars.clone(),
        Err(e) => {
            let mut kept = Vars::new();
            if let Some(previous) = previous {
                if let Some(path) = previous.path_from(EnvSource::LoginShell) {
                    kept.insert("PATH".to_string(), path.to_string());
                }
                kept.extend(previous.proxy_from(EnvSource::LoginShell).iter().cloned());
            }
            problems.push(unreadable(
                "the login shell's environment",
                e,
                !kept.is_empty(),
            ));
            kept
        }
    };
    let system = match &sources.system_proxy {
        Ok(vars) => vars.clone(),
        Err(e) => {
            let kept = previous
                .map(|previous| previous.proxy_from(EnvSource::SystemSettings).to_vec())
                .unwrap_or_default();
            problems.push(unreadable("the system proxy settings", e, !kept.is_empty()));
            kept
        }
    };
    let (machine_path, user_path) = match &sources.system_path {
        Ok(parts) => parts.clone(),
        Err(e) => {
            // What the system settings gave before, the host's entries already merged in.
            let kept = previous
                .and_then(|previous| previous.path_from(EnvSource::SystemSettings))
                .map(str::to_string);
            problems.push(unreadable(
                "the PATH in the system settings",
                e,
                kept.is_some(),
            ));
            (kept, None)
        }
    };
    let host_path = host.get("PATH").map(String::as_str);
    let (path, path_source) =
        if let Some(shell_path) = shell.get("PATH").filter(|p| !p.trim().is_empty()) {
            (
                Some(merge_paths(shell_path, host_path)),
                EnvSource::LoginShell,
            )
        } else if let Some(path) =
            windows_path_from(machine_path.as_deref(), user_path.as_deref(), host_path)
        {
            (Some(path), EnvSource::SystemSettings)
        } else {
            (None, EnvSource::Host)
        };
    let (proxy, proxy_source) = if has_proxy(host) {
        (Vec::new(), EnvSource::Host)
    } else if has_proxy(&shell) {
        let proxy = shell
            .iter()
            .filter(|(name, value)| PROXY_VARS.contains(&name.as_str()) && !value.trim().is_empty())
            .map(|(name, value)| (name.clone(), value.clone()))
            .collect();
        (proxy, EnvSource::LoginShell)
    } else if !system.is_empty() {
        (system, EnvSource::SystemSettings)
    } else {
        (Vec::new(), EnvSource::None)
    };
    ExecEnv {
        path,
        path_source,
        proxy,
        proxy_source,
        problems,
    }
}

/// The problem for a source that could not be read; `kept` says that what it gave before is
/// still used.
fn unreadable(what: &str, error: &str, kept: bool) -> String {
    if kept {
        format!("could not read {what}: {error} (what it gave before is still used)")
    } else {
        format!("could not read {what}: {error}")
    }
}

/// `first`'s entries, then those of `second` that `first` lacks; empty entries are dropped.
pub fn merge_paths(first: &str, second: Option<&str>) -> String {
    let mut seen = HashSet::new();
    let entries: Vec<PathBuf> = std::env::split_paths(first)
        .chain(second.into_iter().flat_map(std::env::split_paths))
        .filter(|entry| !entry.as_os_str().is_empty() && seen.insert(entry.clone()))
        .collect();
    std::env::join_paths(entries)
        .map(|joined| joined.to_string_lossy().into_owned())
        .unwrap_or_else(|_| first.to_string())
}

/// The PATH for the machine's and the user's `Path` from the registry (already expanded) and the
/// host's own PATH: the machine's entries, then the user's, as Windows joins them for a program
/// started now, then the host's entries that neither has, so nothing the host could run is lost.
/// Entries compare as Windows compares paths, ignoring letter case and a trailing backslash (but
/// `C:\` is not `C:`); the first spelling stays, and blank entries are dropped. `None` when the
/// registry holds no entry at all. Written out rather than with `std::env::split_paths`, whose
/// rules depend on the platform, so that the Windows rules are tested on every platform.
fn windows_path_from(
    machine: Option<&str>,
    user: Option<&str>,
    host: Option<&str>,
) -> Option<String> {
    let registry: Vec<String> = machine
        .into_iter()
        .chain(user)
        .flat_map(windows_path_entries)
        .filter(|entry| !entry.trim().is_empty())
        .collect();
    if registry.is_empty() {
        return None;
    }
    let mut seen = HashSet::new();
    let entries: Vec<String> = registry
        .into_iter()
        .chain(host.into_iter().flat_map(windows_path_entries))
        .filter(|entry| !entry.trim().is_empty() && seen.insert(windows_path_key(entry)))
        .map(|entry| {
            // A `;` inside an entry only survives in quotes, as `std::env::join_paths` writes it.
            if entry.contains(';') {
                format!("\"{entry}\"")
            } else {
                entry
            }
        })
        .collect();
    Some(entries.join(";"))
}

/// The entries of a Windows PATH: split at `;`, except inside `"…"`, and the quotes dropped, as
/// Windows and `std::env::split_paths` read it there.
fn windows_path_entries(path: &str) -> Vec<String> {
    let mut entries = Vec::new();
    let mut entry = String::new();
    let mut quoted = false;
    for c in path.chars() {
        match c {
            '"' => quoted = !quoted,
            ';' if !quoted => entries.push(std::mem::take(&mut entry)),
            c => entry.push(c),
        }
    }
    entries.push(entry);
    entries
}

/// What two spellings of one Windows folder have in common: lower case, without trailing
/// backslashes, unless that would leave a bare drive (`C:`, the current folder on that drive).
fn windows_path_key(entry: &str) -> String {
    let lower = entry.to_lowercase();
    let trimmed = lower.trim_end_matches('\\');
    if trimmed.is_empty() || trimmed.ends_with(':') {
        lower
    } else {
        trimmed.to_string()
    }
}

/// `text` with every `%NAME%` that `lookup` knows replaced by its value, as Windows expands a
/// `REG_EXPAND_SZ` value: a reference to an unknown variable, an empty name, a name that holds
/// `=` and a `%` without a partner stay as they are. After an unknown name its closing `%` may
/// open the next reference. Only the Windows registry read uses it; the tests run it on every
/// platform.
#[cfg(any(windows, test))]
fn expand_references(text: &str, lookup: &dyn Fn(&str) -> Option<String>) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        expanded.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            expanded.push_str(&rest[start..]);
            return expanded;
        };
        let name = &after[..end];
        let value = if name.is_empty() || name.contains(['=', '\0']) {
            None
        } else {
            lookup(name)
        };
        match value {
            Some(value) => {
                expanded.push_str(&value);
                rest = &after[end + 1..];
            }
            None => {
                expanded.push('%');
                rest = after;
            }
        }
    }
    expanded.push_str(rest);
    expanded
}

/// The PATH and proxy variables `env` printed between the two markers; `None` when the markers
/// are missing (the shell did not run the script).
pub fn parse_env_output(text: &str) -> Option<Vars> {
    let start = text.find(BEGIN)? + BEGIN.len();
    let end = start + text[start..].rfind(END)?;
    Some(
        text[start..end]
            .lines()
            .filter_map(|line| line.split_once('='))
            .filter(|(name, _)| interesting(name))
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect(),
    )
}

/// The login shell's PATH and proxy variables. The shell runs interactively, so the rc files
/// that usually set PATH (`.zshrc`, `.bashrc`) are read as well as the profile.
#[cfg(unix)]
async fn login_shell_vars() -> Result<Vars, String> {
    let shell = std::env::var_os("SHELL")
        .filter(|shell| !shell.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(DEFAULT_SHELL));
    shell_vars(&shell, SHELL_TIMEOUT).await
}

/// Windows has no login shell: the PATH comes from the registry (`system_path`) and the proxy
/// from the Internet Settings (`system_proxy`).
#[cfg(windows)]
async fn login_shell_vars() -> Result<Vars, String> {
    Ok(Vars::new())
}

/// Unix keeps no PATH in the system settings: the login shell is where it is set.
#[cfg(unix)]
async fn system_path() -> Result<SystemPath, String> {
    Ok((None, None))
}

/// Windows: the machine's and the user's `Path` from the registry, expanded, which is what a
/// program started now gets (`windows_path_from` joins them). Installers add their folders
/// there, while the host's own PATH stays as it was when the host started.
#[cfg(windows)]
async fn system_path() -> Result<SystemPath, String> {
    tokio::task::spawn_blocking(read_windows_path)
        .await
        .map_err(|e| format!("reading the PATH from the registry stopped: {e}"))?
}

/// The registry keys that hold the machine's and the current user's environment variables.
#[cfg(windows)]
const MACHINE_ENVIRONMENT: &str = r"SYSTEM\CurrentControlSet\Control\Session Manager\Environment";
#[cfg(windows)]
const USER_ENVIRONMENT: &str = "Environment";

/// Reads both `Path` values. A `REG_EXPAND_SZ` value has its `%NAME%` references expanded: with
/// the host's own variables first (`SystemRoot`, `APPDATA` and the rest as the host started with
/// them), then the user's and the machine's in the registry, for a variable an installer added
/// after the host started (`%NVM_HOME%`); one level deep, as a variable's own references are not
/// expanded again. A key or a value that does not exist is no value; any other failure fails the
/// read.
#[cfg(windows)]
fn read_windows_path() -> Result<SystemPath, String> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};

    let machine = open_registry_key(HKEY_LOCAL_MACHINE, MACHINE_ENVIRONMENT)?;
    let user = open_registry_key(HKEY_CURRENT_USER, USER_ENVIRONMENT)?;
    let in_registry = |key: &Option<winreg::RegKey>, name: &str| {
        key.as_ref()
            .and_then(|key| registry_text(key, name).ok().flatten())
            .map(|(text, _)| text)
    };
    let lookup = |name: &str| {
        std::env::var(name)
            .ok()
            .or_else(|| in_registry(&user, name))
            .or_else(|| in_registry(&machine, name))
    };
    let path = |key: &Option<winreg::RegKey>| -> Result<Option<String>, String> {
        let Some(key) = key else {
            return Ok(None);
        };
        Ok(registry_text(key, "Path")?.map(|(text, expandable)| {
            if expandable {
                expand_references(&text, &lookup)
            } else {
                text
            }
        }))
    };
    Ok((path(&machine)?, path(&user)?))
}

/// The key `subkey` under `root`; `None` when it does not exist.
#[cfg(windows)]
fn open_registry_key(root: winreg::HKEY, subkey: &str) -> Result<Option<winreg::RegKey>, String> {
    match winreg::RegKey::predef(root).open_subkey(subkey) {
        Ok(key) => Ok(Some(key)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("cannot open the registry key {subkey}: {e}")),
    }
}

/// The text of the value `name` and whether it is to be expanded (`REG_EXPAND_SZ`); `None` when
/// there is no such value. A value that is not text is an error.
#[cfg(windows)]
fn registry_text(key: &winreg::RegKey, name: &str) -> Result<Option<(String, bool)>, String> {
    use winreg::enums::{REG_EXPAND_SZ, REG_SZ};
    use winreg::types::FromRegValue;

    let value = match key.get_raw_value(name) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot read {name}: {e}")),
    };
    let expandable = value.vtype == REG_EXPAND_SZ;
    if !expandable && value.vtype != REG_SZ {
        return Err(format!("{name} is not text ({:?})", value.vtype));
    }
    let text = String::from_reg_value(&value).map_err(|e| format!("cannot read {name}: {e}"))?;
    Ok(Some((text, expandable)))
}

/// Runs `shell -l -i -c <script>`, where the script prints `env` between two markers. The shell
/// leads a session and a process group of its own; the group, with anything the rc files left
/// running in it, is killed once the end marker has arrived (or the shell has ended), or after
/// `limit` if neither has happened by then.
///
/// The read stops at the end marker, not at the end of the pipe: an rc file can leave a process
/// running that holds the shell's stdout, and then the pipe would stay open until `limit` and
/// the variables, which are complete by then, would be lost.
///
/// A session of its own, not just a group: an interactive zsh in a background process group of
/// the host's terminal is stopped by the terminal and never prints (measured with zsh 5.9 on
/// macOS 27 on 2026-10-13: a new group of a terminal's session hung until the limit, a new
/// session finished in 10 ms; bash is not affected). That is the host started from a terminal,
/// as the dev host usually is; started from Finder it has no terminal and either way works.
#[cfg(unix)]
pub async fn shell_vars(shell: &Path, limit: Duration) -> Result<Vars, String> {
    use std::process::Stdio;

    use process_wrap::tokio::{CommandWrap, KillOnDrop, ProcessSession};

    let script = format!("printf '\\n%s\\n' {BEGIN}; env; printf '\\n%s\\n' {END}");
    let mut command = CommandWrap::with_new(shell, |c| {
        c.args(["-l", "-i", "-c", script.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
    });
    command.wrap(ProcessSession);
    command.wrap(KillOnDrop);
    let mut child = command
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", shell.display()))?;
    let Some(mut stdout) = child.stdout().take() else {
        let _ = child.start_kill();
        return Err(format!("{} has no output pipe", shell.display()));
    };
    let outcome = tokio::time::timeout(limit, read_until_end_marker(&mut stdout)).await;
    // Ends the shell and whatever its rc files left running in its group.
    let _ = child.start_kill();
    match outcome {
        Ok(Ok(bytes)) => parse_env_output(&String::from_utf8_lossy(&bytes))
            .ok_or_else(|| format!("{} printed no environment", shell.display())),
        Ok(Err(e)) => Err(format!("cannot read from {}: {e}", shell.display())),
        Err(_) => Err(format!(
            "{} did not finish within {limit:?}",
            shell.display()
        )),
    }
}

/// What `pipe` delivers up to and including the end marker, or up to the end of the pipe when the
/// marker does not come. The marker is on a line of its own, whatever the rc files printed before
/// it.
#[cfg(unix)]
async fn read_until_end_marker<R>(mut pipe: R) -> std::io::Result<Vec<u8>>
where
    R: tokio::io::AsyncRead + Unpin,
{
    use tokio::io::AsyncReadExt;

    let marker = format!("\n{END}");
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        let read = pipe.read(&mut chunk).await?;
        if read == 0 {
            return Ok(bytes);
        }
        // The marker may be split across two reads, so the search starts before the new bytes.
        let from = bytes.len().saturating_sub(marker.len() - 1);
        bytes.extend_from_slice(&chunk[..read]);
        if bytes[from..]
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
        {
            return Ok(bytes);
        }
    }
}

/// macOS: the system proxy settings as `scutil --proxy` prints them.
#[cfg(target_os = "macos")]
async fn system_proxy() -> Result<Vec<(String, String)>, String> {
    let run = tokio::process::Command::new("/usr/sbin/scutil")
        .arg("--proxy")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    let output = tokio::time::timeout(SCUTIL_TIMEOUT, run)
        .await
        .map_err(|_| format!("scutil --proxy did not answer within {SCUTIL_TIMEOUT:?}"))?
        .map_err(|e| format!("cannot run scutil: {e}"))?;
    if !output.status.success() {
        return Err(format!("scutil --proxy exited with {}", output.status));
    }
    Ok(parse_scutil_proxy(&String::from_utf8_lossy(&output.stdout)))
}

/// Windows: the current user's Internet Settings.
#[cfg(windows)]
async fn system_proxy() -> Result<Vec<(String, String)>, String> {
    tokio::task::spawn_blocking(read_windows_proxy)
        .await
        .map_err(|e| format!("reading the proxy settings stopped: {e}"))?
}

#[cfg(windows)]
fn read_windows_proxy() -> Result<Vec<(String, String)>, String> {
    use winreg::{RegKey, enums::HKEY_CURRENT_USER};

    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .map_err(|e| format!("cannot open the Internet Settings key: {e}"))?;
    let enabled: u32 = key.get_value("ProxyEnable").unwrap_or(0);
    let server: String = key.get_value("ProxyServer").unwrap_or_default();
    let overrides: String = key.get_value("ProxyOverride").unwrap_or_default();
    Ok(parse_windows_proxy(enabled != 0, &server, &overrides))
}

/// Other Unix systems (not target platforms) have no single place for proxy settings.
#[cfg(all(unix, not(target_os = "macos")))]
async fn system_proxy() -> Result<Vec<(String, String)>, String> {
    Ok(Vec::new())
}

/// The proxy variables for what `scutil --proxy` prints. Only the top-level dictionary counts
/// (`__SCOPED__` holds per-interface copies). HTTPS and HTTP proxies map to `HTTPS_PROXY` and
/// `HTTP_PROXY`; a SOCKS proxy maps to `ALL_PROXY` only when neither is set. A proxy
/// auto-configuration (PAC) file cannot be expressed in variables and is left out.
pub fn parse_scutil_proxy(text: &str) -> Vec<(String, String)> {
    let mut values = HashMap::new();
    let mut exceptions = Vec::new();
    let mut depth = 0usize;
    let mut in_exceptions = false;
    for line in text.lines().map(str::trim) {
        if line == "}" {
            depth = depth.saturating_sub(1);
            if depth < 2 {
                in_exceptions = false;
            }
            continue;
        }
        let Some((key, value)) = line.split_once(" : ") else {
            if line.ends_with('{') {
                depth += 1;
            }
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if value.ends_with('{') {
            if depth == 1 && key == "ExceptionsList" {
                in_exceptions = true;
            }
            depth += 1;
        } else if in_exceptions && depth == 2 {
            exceptions.push(value.to_string());
        } else if depth == 1 {
            values.insert(key.to_string(), value.to_string());
        }
    }
    let proxy = |enable: &str, host: &str, port: &str, scheme: &str| -> Option<String> {
        if values.get(enable).map(String::as_str) != Some("1") {
            return None;
        }
        let host = values.get(host).filter(|h| !h.is_empty())?;
        let port = values.get(port).filter(|p| !p.is_empty())?;
        Some(format!("{scheme}://{host}:{port}"))
    };
    proxy_vars(
        proxy("HTTPSEnable", "HTTPSProxy", "HTTPSPort", "http"),
        proxy("HTTPEnable", "HTTPProxy", "HTTPPort", "http"),
        proxy("SOCKSEnable", "SOCKSProxy", "SOCKSPort", "socks5h"),
        exceptions.iter().map(String::as_str),
    )
}

/// The proxy variables for the current Windows user's Internet Settings: `ProxyEnable`,
/// `ProxyServer` (`host:port` for every protocol, or `http=host:port;https=host:port;socks=…`)
/// and `ProxyOverride` (`;`-separated; `<local>` means plain host names, already covered by the
/// local hosts).
pub fn parse_windows_proxy(enabled: bool, server: &str, overrides: &str) -> Vec<(String, String)> {
    let server = server.trim();
    if !enabled || server.is_empty() {
        return Vec::new();
    }
    let url = |value: &str, scheme: &str| {
        if value.contains("://") {
            value.to_string()
        } else {
            format!("{scheme}://{value}")
        }
    };
    let (https, http, socks) = if server.contains('=') {
        let (mut https, mut http, mut socks) = (None, None, None);
        for part in server.split(';') {
            let Some((protocol, value)) = part.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if value.is_empty() {
                continue;
            }
            match protocol.trim().to_ascii_lowercase().as_str() {
                "https" => https = Some(url(value, "http")),
                "http" => http = Some(url(value, "http")),
                "socks" => socks = Some(url(value, "socks5h")),
                _ => {}
            }
        }
        (https, http, socks)
    } else {
        let value = url(server, "http");
        (Some(value.clone()), Some(value), None)
    };
    let exceptions = overrides
        .split(';')
        .map(str::trim)
        .filter(|entry| !entry.is_empty() && !entry.eq_ignore_ascii_case("<local>"));
    proxy_vars(https, http, socks, exceptions)
}

fn push_both(vars: &mut Vec<(String, String)>, name: &str, value: &str) {
    vars.push((name.to_string(), value.to_string()));
    vars.push((name.to_ascii_lowercase(), value.to_string()));
}

/// Both spellings of each variable for the proxies found, plus a `NO_PROXY` that always holds
/// the local hosts; nothing at all when no proxy is set.
fn proxy_vars<'a>(
    https: Option<String>,
    http: Option<String>,
    socks: Option<String>,
    exceptions: impl Iterator<Item = &'a str>,
) -> Vec<(String, String)> {
    let mut vars = Vec::new();
    if let Some(value) = &https {
        push_both(&mut vars, "HTTPS_PROXY", value);
    }
    if let Some(value) = &http {
        push_both(&mut vars, "HTTP_PROXY", value);
    }
    if https.is_none()
        && http.is_none()
        && let Some(value) = &socks
    {
        push_both(&mut vars, "ALL_PROXY", value);
    }
    if vars.is_empty() {
        return vars;
    }
    let mut hosts: Vec<String> = LOCAL_HOSTS.iter().map(|h| h.to_string()).collect();
    for entry in exceptions
        .flat_map(|e| e.split([',', ';']))
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        // `*.local` means every host under `.local`, which `NO_PROXY` writes as `.local`.
        let entry = entry.strip_prefix('*').unwrap_or(entry).to_string();
        if !hosts.contains(&entry) {
            hosts.push(entry);
        }
    }
    push_both(&mut vars, "NO_PROXY", &hosts.join(","));
    vars
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vars {
        pairs
            .iter()
            .map(|(n, v)| (n.to_string(), v.to_string()))
            .collect()
    }

    /// The sources as given, with no PATH in the system settings (as on Unix).
    fn sources(
        host: Vars,
        shell: Result<Vars, String>,
        system_proxy: Result<Vec<(String, String)>, String>,
    ) -> Sources {
        Sources {
            host,
            shell,
            system_proxy,
            system_path: Ok((None, None)),
        }
    }

    /// `combine` over a first read, with no environment before it.
    fn first(
        host: &Vars,
        shell: Result<Vars, String>,
        system_proxy: Result<Vec<(String, String)>, String>,
    ) -> ExecEnv {
        combine(&sources(host.clone(), shell, system_proxy), None)
    }

    #[test]
    fn the_hosts_own_proxy_wins_and_is_passed_on_unchanged() {
        let host = vars(&[("PATH", "/usr/bin"), ("HTTPS_PROXY", "http://host-proxy:1")]);
        let shell = Ok(vars(&[
            ("PATH", "/opt/demo/bin:/usr/bin"),
            ("HTTPS_PROXY", "http://shell-proxy:2"),
        ]));
        let env = first(
            &host,
            shell,
            Ok(vec![("HTTPS_PROXY".into(), "http://system:3".into())]),
        );
        assert_eq!(env.proxy_source, EnvSource::Host);
        assert!(
            env.proxy.is_empty(),
            "the host's own variables are inherited, not set again"
        );
        assert_eq!(
            env.path.as_deref(),
            Some(merge_paths("/opt/demo/bin:/usr/bin", Some("/usr/bin")).as_str())
        );
        assert_eq!(env.path_source, EnvSource::LoginShell);
        assert!(env.problems.is_empty());
    }

    #[test]
    fn the_login_shells_proxy_comes_before_the_system_settings() {
        let host = vars(&[
            ("PATH", "/usr/bin"),
            ("NO_PROXY", "only-no-proxy-is-not-a-proxy"),
        ]);
        let shell = Ok(vars(&[
            ("PATH", "/usr/bin"),
            ("https_proxy", "http://shell-proxy:2"),
            ("NO_PROXY", "localhost"),
        ]));
        let env = first(
            &host,
            shell,
            Ok(vec![("HTTPS_PROXY".into(), "http://system:3".into())]),
        );
        assert_eq!(env.proxy_source, EnvSource::LoginShell);
        assert_eq!(
            env.proxy,
            vec![
                ("NO_PROXY".to_string(), "localhost".to_string()),
                (
                    "https_proxy".to_string(),
                    "http://shell-proxy:2".to_string()
                ),
            ]
        );
    }

    #[test]
    fn system_settings_are_the_last_resort_and_nothing_means_no_proxy() {
        let host = vars(&[("PATH", "/usr/bin")]);
        let system = vec![(
            "HTTPS_PROXY".to_string(),
            "http://127.0.0.1:7890".to_string(),
        )];
        let env = first(&host, Ok(Vars::new()), Ok(system.clone()));
        assert_eq!(env.proxy_source, EnvSource::SystemSettings);
        assert_eq!(env.proxy, system);
        assert_eq!(
            env.path, None,
            "no login shell PATH: the host's PATH is kept"
        );
        assert_eq!(env.path_source, EnvSource::Host);

        let none = first(&host, Ok(Vars::new()), Ok(Vec::new()));
        assert_eq!(none.proxy_source, EnvSource::None);
        assert!(none.proxy.is_empty());
    }

    #[test]
    fn what_could_not_be_read_is_listed_as_a_problem() {
        let env = first(
            &Vars::new(),
            Err("timed out".into()),
            Err("no scutil".into()),
        );
        assert_eq!(env.problems.len(), 2);
        assert!(env.problems[0].contains("login shell") && env.problems[0].contains("timed out"));
        assert!(env.problems[1].contains("system proxy") && env.problems[1].contains("no scutil"));
        assert_eq!(env.path_source, EnvSource::Host);
        assert_eq!(env.proxy_source, EnvSource::None);
    }

    /// A first read in which the login shell gave a PATH and a proxy.
    fn read_once() -> ExecEnv {
        first(
            &Vars::new(),
            Ok(vars(&[
                ("PATH", "/opt/demo/bin:/usr/bin"),
                ("HTTPS_PROXY", "http://127.0.0.1:9"),
            ])),
            Ok(Vec::new()),
        )
    }

    // One slow rc file during a re-check must not swap a good login-shell PATH for the host's
    // own, which for an app started from Finder is launchd's minimal one.
    #[test]
    fn a_login_shell_that_fails_later_keeps_what_it_gave_before() {
        let before = read_once();
        assert_eq!(before.path_source, EnvSource::LoginShell);
        assert_eq!(before.proxy_source, EnvSource::LoginShell);
        let failed = sources(
            Vars::new(),
            Err("/bin/zsh did not finish within 10s".into()),
            Ok(Vec::new()),
        );
        let again = combine(&failed, Some(&before));
        assert_eq!(again.path, before.path);
        assert_eq!(again.path_source, EnvSource::LoginShell);
        assert_eq!(again.proxy, before.proxy);
        assert_eq!(again.proxy_source, EnvSource::LoginShell);
        assert_eq!(again.problems.len(), 1, "{:?}", again.problems);
        let problem = &again.problems[0];
        assert!(
            problem.contains("login shell") && problem.contains("did not finish within 10s"),
            "{problem}"
        );
        assert!(problem.contains("before"), "says what is kept: {problem}");
        // Without an earlier environment the failed read falls back as it always did.
        let alone = combine(&failed, None);
        assert_eq!(alone.path, None);
        assert_eq!(alone.path_source, EnvSource::Host);
        assert_eq!(alone.proxy_source, EnvSource::None);
    }

    #[test]
    fn a_source_that_answers_replaces_what_it_gave_before() {
        let before = read_once();
        let answered = sources(
            Vars::new(),
            Ok(vars(&[("PATH", "/opt/new/bin")])),
            Ok(Vec::new()),
        );
        let again = combine(&answered, Some(&before));
        assert_eq!(again.path.as_deref(), Some("/opt/new/bin"));
        assert_eq!(again.path_source, EnvSource::LoginShell);
        assert_eq!(
            again.proxy_source,
            EnvSource::None,
            "the login shell no longer sets a proxy"
        );
        assert!(again.proxy.is_empty());
        assert!(again.problems.is_empty(), "{:?}", again.problems);
    }

    #[test]
    fn only_what_the_failed_source_gave_is_kept() {
        // The PATH came from the login shell, the proxy from the system settings.
        let before = first(
            &Vars::new(),
            Ok(vars(&[("PATH", "/opt/demo/bin")])),
            Ok(vec![("HTTPS_PROXY".into(), "http://127.0.0.1:7890".into())]),
        );
        assert_eq!(before.proxy_source, EnvSource::SystemSettings);
        // The system settings fail: their proxy stays, and the login shell's new PATH is used.
        let again = combine(
            &sources(
                Vars::new(),
                Ok(vars(&[("PATH", "/opt/new/bin")])),
                Err("scutil --proxy did not answer within 5s".into()),
            ),
            Some(&before),
        );
        assert_eq!(again.path.as_deref(), Some("/opt/new/bin"));
        assert_eq!(again.proxy, before.proxy);
        assert_eq!(again.proxy_source, EnvSource::SystemSettings);
        assert!(
            again.problems[0].contains("system proxy"),
            "{:?}",
            again.problems
        );
        // The login shell fails: its PATH stays, and the system settings' new proxy is used.
        let again = combine(
            &sources(
                Vars::new(),
                Err("timed out".into()),
                Ok(vec![("HTTPS_PROXY".into(), "http://127.0.0.1:7891".into())]),
            ),
            Some(&before),
        );
        assert_eq!(again.path, before.path);
        assert_eq!(again.path_source, EnvSource::LoginShell);
        assert_eq!(
            again.proxy,
            [(
                "HTTPS_PROXY".to_string(),
                "http://127.0.0.1:7891".to_string()
            )]
        );
    }

    #[test]
    fn a_source_that_never_answered_has_nothing_to_keep() {
        let before = first(&Vars::new(), Err("timed out".into()), Ok(Vec::new()));
        let again = combine(
            &sources(Vars::new(), Err("timed out".into()), Ok(Vec::new())),
            Some(&before),
        );
        assert_eq!(again.path, None);
        assert_eq!(again.path_source, EnvSource::Host);
        assert_eq!(again.proxy_source, EnvSource::None);
        assert_eq!(again.problems.len(), 1, "{:?}", again.problems);
        assert!(
            !again.problems[0].contains("before"),
            "nothing was kept: {:?}",
            again.problems
        );
    }

    /// A read on Windows: no login shell, the registry's PATH as given.
    fn windows_read(host_path: &str, system_path: Result<SystemPath, String>) -> Sources {
        Sources {
            host: vars(&[("PATH", host_path)]),
            shell: Ok(Vars::new()),
            system_proxy: Ok(Vec::new()),
            system_path,
        }
    }

    // A Windows program keeps the PATH it started with; the registry holds the one a program
    // started now gets, with the folder of an executor installed meanwhile.
    #[test]
    fn without_a_login_shell_the_path_comes_from_the_system_settings() {
        let host = r"C:\Windows\system32;C:\Tools\only-the-host";
        let registry = (
            Some(r"C:\Windows\system32;C:\Windows".to_string()),
            Some(r"C:\Program Files\nodejs".to_string()),
        );
        let env = combine(&windows_read(host, Ok(registry)), None);
        assert_eq!(
            env.path.as_deref(),
            Some(r"C:\Windows\system32;C:\Windows;C:\Program Files\nodejs;C:\Tools\only-the-host")
        );
        assert_eq!(env.path_source, EnvSource::SystemSettings);
        assert!(env.problems.is_empty(), "{:?}", env.problems);

        // A later read of the registry fails: the PATH it gave before stays.
        let failed = || windows_read(host, Err("cannot open the registry key: denied".into()));
        let again = combine(&failed(), Some(&env));
        assert_eq!(again.path, env.path);
        assert_eq!(again.path_source, EnvSource::SystemSettings);
        let problem = &again.problems[0];
        assert!(
            problem.contains("PATH") && problem.contains("denied"),
            "{problem}"
        );
        // A first read that fails keeps the host's PATH, as before the registry was read at all.
        let alone = combine(&failed(), None);
        assert_eq!(alone.path, None);
        assert_eq!(alone.path_source, EnvSource::Host);
        assert_eq!(alone.problems.len(), 1, "{:?}", alone.problems);
    }

    /// An environment block as `std::env::vars_os` hands it out.
    fn block(pairs: &[(&str, &str)]) -> impl Iterator<Item = (OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect::<Vec<_>>()
            .into_iter()
    }

    // Windows spells the variable `Path` (the name of the registry value), and its variable names
    // have no case. The host's own entries, such as the shims a terminal profile added, must
    // survive the merge with the registry's PATH: the children get only the merged PATH, and an
    // executor found only on the host's would read as not installed.
    #[test]
    fn where_names_have_no_case_a_host_path_spelled_path_keeps_its_own_entries() {
        let host = host_vars_from(
            block(&[
                ("Path", r"C:\Windows\system32;C:\Tools\fnm-shims"),
                ("https_proxy", "http://127.0.0.1:7890"),
                ("TEMP", r"C:\Temp"),
            ]),
            false,
        );
        assert_eq!(
            host.get("PATH").map(String::as_str),
            Some(r"C:\Windows\system32;C:\Tools\fnm-shims")
        );
        assert_eq!(
            host.get("https_proxy").map(String::as_str),
            Some("http://127.0.0.1:7890")
        );
        assert_eq!(host.len(), 2, "{:?}", host.keys().collect::<Vec<_>>());
        let read = Sources {
            host,
            shell: Ok(Vars::new()),
            system_proxy: Ok(Vec::new()),
            system_path: Ok((Some(r"C:\Windows\system32".to_string()), None)),
        };
        let env = combine(&read, None);
        assert_eq!(
            env.path.as_deref(),
            Some(r"C:\Windows\system32;C:\Tools\fnm-shims")
        );
        assert_eq!(env.path_source, EnvSource::SystemSettings);
    }

    #[test]
    fn where_names_have_case_only_path_in_capitals_is_the_path() {
        let host = host_vars_from(block(&[("Path", "/opt/other"), ("PATH", "/usr/bin")]), true);
        assert_eq!(host.get("PATH").map(String::as_str), Some("/usr/bin"));
        assert_eq!(host.len(), 1, "{:?}", host.keys().collect::<Vec<_>>());
    }

    #[cfg(windows)]
    #[test]
    fn on_windows_the_hosts_path_is_found_whatever_its_spelling() {
        let host = host_vars();
        assert!(
            host.get("PATH").is_some_and(|path| !path.is_empty()),
            "{:?}",
            host.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn the_registry_path_is_the_machines_then_the_users_then_what_only_the_host_has() {
        let path = windows_path_from(
            Some(r"C:\Windows\system32;C:\Windows"),
            Some(r"C:\Program Files\nodejs;C:\Tools\npm"),
            Some(r"C:\Windows\system32;C:\Tools\only-the-host"),
        );
        assert_eq!(
            path.as_deref(),
            Some(
                r"C:\Windows\system32;C:\Windows;C:\Program Files\nodejs;C:\Tools\npm;C:\Tools\only-the-host"
            )
        );
    }

    #[test]
    fn either_registry_path_may_be_missing() {
        let host = Some(r"C:\Tools\only-the-host");
        assert_eq!(
            windows_path_from(Some(r"C:\Windows"), None, host).as_deref(),
            Some(r"C:\Windows;C:\Tools\only-the-host")
        );
        assert_eq!(
            windows_path_from(None, Some(r"C:\Tools\npm"), host).as_deref(),
            Some(r"C:\Tools\npm;C:\Tools\only-the-host")
        );
        assert_eq!(
            windows_path_from(Some(r"C:\Windows"), Some(""), None).as_deref(),
            Some(r"C:\Windows")
        );
        // Nothing in the registry: no PATH from the system settings, whatever the host has.
        assert_eq!(windows_path_from(None, None, host), None);
        assert_eq!(windows_path_from(Some(" ; ;"), Some(""), host), None);
    }

    #[test]
    fn registry_entries_that_differ_only_in_case_or_a_trailing_backslash_are_one_entry() {
        let path = windows_path_from(
            Some(r"C:\Windows\System32;C:\Program Files\nodejs\"),
            Some(r"c:\windows\system32\;C:\PROGRAM FILES\NODEJS;C:\Tools\npm"),
            Some(r"C:\TOOLS\NPM\;C:\Windows\System32"),
        );
        // The first spelling stays.
        assert_eq!(
            path.as_deref(),
            Some(r"C:\Windows\System32;C:\Program Files\nodejs\;C:\Tools\npm")
        );
        // A drive's root keeps its backslash: `C:` alone is the current folder on that drive.
        assert_eq!(
            windows_path_from(Some(r"C:\;C:"), None, None).as_deref(),
            Some(r"C:\;C:")
        );
    }

    #[test]
    fn empty_registry_entries_are_dropped() {
        let path = windows_path_from(
            Some(r";C:\Windows;;  ;"),
            Some(r";C:\Tools\npm;"),
            Some(";;"),
        );
        assert_eq!(path.as_deref(), Some(r"C:\Windows;C:\Tools\npm"));
    }

    #[test]
    fn a_quoted_registry_entry_may_hold_a_semicolon() {
        // Windows reads a `"…"` part of a PATH as one entry, as `std::env::split_paths` does
        // there, and `std::env::join_paths` quotes such an entry again.
        let path = windows_path_from(Some(r#"C:\Windows;"C:\Tools\a;b";C:\Tools\c"#), None, None);
        assert_eq!(
            path.as_deref(),
            Some(r#"C:\Windows;"C:\Tools\a;b";C:\Tools\c"#)
        );
        // Quotes around an entry without a semicolon are dropped.
        assert_eq!(
            windows_path_from(Some(r#""C:\Tools\d""#), None, None).as_deref(),
            Some(r"C:\Tools\d")
        );
    }

    /// Whether the Windows PATH `path` holds the system folder (`%SystemRoot%\system32`).
    #[cfg(windows)]
    fn holds_the_system_folder(path: &str) -> bool {
        let system = format!(r"{}\system32", std::env::var("SystemRoot").unwrap());
        windows_path_entries(path)
            .iter()
            .any(|entry| windows_path_key(entry) == windows_path_key(&system))
    }

    // The registry read itself runs on Windows only (CI). The machine's Path always holds the
    // system folder, which Windows writes there as `%SystemRoot%\system32`.
    #[cfg(windows)]
    #[test]
    fn the_machines_path_in_the_registry_holds_the_system_folder_expanded() {
        let (machine, _user) = read_windows_path().unwrap();
        let machine = machine.expect("the machine's Path");
        assert!(holds_the_system_folder(&machine), "{machine}");
        assert!(
            !machine.to_lowercase().contains("%systemroot%"),
            "{machine}"
        );
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn on_windows_the_path_comes_from_the_registry() {
        let env = resolve().await;
        assert_eq!(env.path_source, EnvSource::SystemSettings, "{env:?}");
        let path = env.path.expect("a PATH from the registry");
        assert!(holds_the_system_folder(&path), "{path}");
    }

    #[test]
    fn registry_values_expand_the_variables_they_name() {
        let lookup = |name: &str| match name {
            "SystemRoot" => Some(r"C:\Windows".to_string()),
            "ProgramFiles" => Some(r"C:\Program Files".to_string()),
            _ => None,
        };
        assert_eq!(
            expand_references(r"%SystemRoot%\system32;%ProgramFiles%\nodejs", &lookup),
            r"C:\Windows\system32;C:\Program Files\nodejs"
        );
        // An unknown variable, a lone `%`, an empty name and a name with `=` stay as they are.
        assert_eq!(
            expand_references(r"%NOT_SET%\bin;%SystemRoot%", &lookup),
            r"%NOT_SET%\bin;C:\Windows"
        );
        assert_eq!(expand_references("100%", &lookup), "100%");
        assert_eq!(expand_references("%%", &lookup), "%%");
        assert_eq!(expand_references("%=C:%", &lookup), "%=C:%");
        assert_eq!(expand_references("", &lookup), "");
    }

    #[cfg(unix)]
    #[test]
    fn paths_merge_in_order_without_duplicates_or_empty_entries() {
        assert_eq!(merge_paths("/a:/b::/a", Some("/b:/c")), "/a:/b:/c");
        assert_eq!(merge_paths("/a", None), "/a");
    }

    #[cfg(windows)]
    #[test]
    fn paths_merge_in_order_without_duplicates_or_empty_entries() {
        assert_eq!(
            merge_paths(r"C:\a;C:\b;;C:\a", Some(r"C:\b;C:\c")),
            r"C:\a;C:\b;C:\c"
        );
    }

    #[test]
    fn credentials_in_a_proxy_url_are_masked() {
        assert_eq!(
            mask_credentials("http://jane:s3cret@proxy.example:8080"),
            "http://***@proxy.example:8080"
        );
        assert_eq!(
            mask_credentials("http://token@proxy.example:8080/path@x"),
            "http://***@proxy.example:8080/path@x"
        );
        assert_eq!(
            mask_credentials("http://proxy.example:8080"),
            "http://proxy.example:8080"
        );
        assert_eq!(
            mask_credentials("localhost,127.0.0.1"),
            "localhost,127.0.0.1"
        );
        let env = ExecEnv {
            proxy: vec![(
                "HTTPS_PROXY".into(),
                "http://jane:s3cret@proxy.example:8080".into(),
            )],
            ..ExecEnv::inherit()
        };
        assert_eq!(
            env.proxy_for_display()[0].1,
            "http://***@proxy.example:8080"
        );
    }

    #[test]
    fn credentials_without_a_scheme_are_masked_too() {
        // curl accepts `user:password@host:port` as a proxy; the credentials end at the last `@`
        // before the first `/`, so a password that holds an `@` is masked whole.
        assert_eq!(
            mask_credentials("jane:s3cret@proxy.example:8080"),
            "***@proxy.example:8080"
        );
        assert_eq!(
            mask_credentials("token@proxy.example:8080"),
            "***@proxy.example:8080"
        );
        assert_eq!(
            mask_credentials("jane:p@ss@proxy.example:8080"),
            "***@proxy.example:8080"
        );
        assert_eq!(
            mask_credentials("jane:s3cret@proxy.example:8080/path@x"),
            "***@proxy.example:8080/path@x"
        );
        assert_eq!(
            mask_credentials("socks5h://jane:s3cret@proxy.example:1080"),
            "socks5h://***@proxy.example:1080"
        );
        // An `@` after the first `/` belongs to a path, not to credentials.
        assert_eq!(
            mask_credentials("proxy.example:8080/path@x"),
            "proxy.example:8080/path@x"
        );
        assert_eq!(mask_credentials("proxy.example:8080"), "proxy.example:8080");
        assert_eq!(mask_credentials(""), "");
        // `://` further in is not a scheme: the credentials in front of it are still found.
        assert_eq!(
            mask_credentials("jane:s3cret@proxy.example/redirect?to=http://other.example"),
            "***@proxy.example/redirect?to=http://other.example"
        );
    }

    #[test]
    fn credentials_are_masked_behind_any_prefix() {
        // Whatever stands in front of the first `://` is the scheme as far as masking goes (a
        // quote, a blank, a `NAME=`), unless it holds an `@` or a `/`: then the `://` belongs to
        // a path or a query and the value is read from its start.
        for (value, masked) in [
            (
                " http://jane:s3cret@proxy.example:8080",
                " http://***@proxy.example:8080",
            ),
            (
                "\thttp://jane:s3cret@proxy.example:8080",
                "\thttp://***@proxy.example:8080",
            ),
            (
                "\"http://jane:s3cret@proxy.example:8080\"",
                "\"http://***@proxy.example:8080\"",
            ),
            (
                "'http://jane:s3cret@proxy.example:8080'",
                "'http://***@proxy.example:8080'",
            ),
            (
                "HTTPS_PROXY=http://jane:s3cret@proxy.example:8080",
                "HTTPS_PROXY=http://***@proxy.example:8080",
            ),
            (
                "http://jane:p@ss@proxy.example:8080",
                "http://***@proxy.example:8080",
            ),
            (
                "jane:s3cret@proxy.example/redirect?to=http://other.example",
                "***@proxy.example/redirect?to=http://other.example",
            ),
        ] {
            assert_eq!(mask_credentials(value), masked, "{value:?}");
            assert!(!masked.contains("s3cret"));
        }
    }

    #[test]
    fn masking_twice_changes_nothing() {
        for value in [
            "http://jane:s3cret@proxy.example:8080",
            "http://token@proxy.example:8080/path@x",
            "http://proxy.example:8080",
            "localhost,127.0.0.1",
            "jane:s3cret@proxy.example:8080",
            "jane:p@ss@proxy.example:8080",
            "jane:s3cret@proxy.example:8080/path@x",
            "socks5h://jane:s3cret@proxy.example:1080",
            "proxy.example:8080/path@x",
            "jane:s3cret@proxy.example/redirect?to=http://other.example",
            " http://jane:s3cret@proxy.example:8080",
            "\"http://jane:s3cret@proxy.example:8080\"",
            "HTTPS_PROXY=http://jane:s3cret@proxy.example:8080",
            "http://jane:p@ss@proxy.example:8080",
            "",
        ] {
            let once = mask_credentials(value);
            assert_eq!(mask_credentials(&once), once, "{value:?}");
        }
    }

    #[test]
    fn debug_output_shows_the_proxy_masked() {
        let env = ExecEnv {
            proxy: vec![
                (
                    "HTTPS_PROXY".into(),
                    "http://jane:s3cret@proxy.example:8080".into(),
                ),
                ("ALL_PROXY".into(), "jane:s3cret@proxy.example:1080".into()),
            ],
            proxy_source: EnvSource::LoginShell,
            problems: vec!["demo problem".into()],
            ..ExecEnv::inherit()
        };
        for text in [format!("{env:?}"), format!("{env:#?}")] {
            assert!(!text.contains("s3cret"), "{text}");
            assert!(text.contains("http://***@proxy.example:8080"), "{text}");
            assert!(text.contains("***@proxy.example:1080"), "{text}");
            // Everything else is printed as the derive would print it.
            assert!(text.contains("ExecEnv"), "{text}");
            assert!(text.contains("LoginShell"), "{text}");
            assert!(text.contains("demo problem"), "{text}");
        }
    }

    #[test]
    fn env_output_is_read_between_the_markers_only() {
        let text = format!(
            "rc noise PATH=/wrong\n\n{BEGIN}\nPATH=/opt/demo/bin:/usr/bin\nHOME=/home/demo\nHTTPS_PROXY=http://p:1/?a=b\n\n{END}\nmore noise HTTP_PROXY=x\n"
        );
        let parsed = parse_env_output(&text).unwrap();
        assert_eq!(
            parsed.get("PATH").map(String::as_str),
            Some("/opt/demo/bin:/usr/bin")
        );
        assert_eq!(
            parsed.get("HTTPS_PROXY").map(String::as_str),
            Some("http://p:1/?a=b")
        );
        assert!(
            !parsed.contains_key("HOME"),
            "only PATH and proxy variables are kept"
        );
        assert!(
            !parsed.contains_key("HTTP_PROXY"),
            "text after the end marker is ignored"
        );
        assert_eq!(parse_env_output("no markers at all"), None);
    }

    /// `scutil --proxy` as macOS prints it, with a per-interface dictionary whose values must not
    /// override the top level. (The interface's proxy is a documentation address, RFC 5737.)
    const SCUTIL: &str = "<dictionary> {
  ExceptionsList : <array> {
    0 : *.local
    1 : 169.254/16
  }
  FTPPassive : 1
  HTTPEnable : 1
  HTTPPort : 6152
  HTTPProxy : 127.0.0.1
  HTTPSEnable : 1
  HTTPSPort : 6152
  HTTPSProxy : 127.0.0.1
  SOCKSEnable : 1
  SOCKSPort : 6153
  SOCKSProxy : 127.0.0.1
  __SCOPED__ : <dictionary> {
    en0 : <dictionary> {
      HTTPSEnable : 1
      HTTPSPort : 9999
      HTTPSProxy : 203.0.113.9
    }
  }
}
";

    #[test]
    fn scutil_output_maps_to_proxy_variables() {
        let vars = parse_scutil_proxy(SCUTIL);
        let get = |name: &str| {
            vars.iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        assert_eq!(get("HTTPS_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("https_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("HTTP_PROXY"), Some("http://127.0.0.1:6152"));
        assert_eq!(get("http_proxy"), Some("http://127.0.0.1:6152"));
        assert_eq!(
            get("ALL_PROXY"),
            None,
            "SOCKS is used only when there is no HTTP(S) proxy"
        );
        assert_eq!(
            get("NO_PROXY"),
            Some("localhost,127.0.0.1,::1,.local,169.254/16")
        );
        assert_eq!(get("no_proxy"), get("NO_PROXY"));
    }

    #[test]
    fn scutil_with_only_socks_or_nothing_enabled() {
        let socks_only =
            "<dictionary> {\n  SOCKSEnable : 1\n  SOCKSPort : 1080\n  SOCKSProxy : 127.0.0.1\n}\n";
        let vars = parse_scutil_proxy(socks_only);
        assert!(vars.contains(&(
            "ALL_PROXY".to_string(),
            "socks5h://127.0.0.1:1080".to_string()
        )));
        assert!(vars.iter().any(|(n, _)| n == "NO_PROXY"));

        let off = "<dictionary> {\n  HTTPEnable : 0\n  HTTPPort : 8080\n  HTTPProxy : proxy\n  ProxyAutoConfigEnable : 1\n  ProxyAutoConfigURLString : http://wpad/x.pac\n}\n";
        assert!(
            parse_scutil_proxy(off).is_empty(),
            "a PAC file cannot be expressed in variables"
        );
        assert!(parse_scutil_proxy("").is_empty());
    }

    #[test]
    fn windows_internet_settings_map_to_proxy_variables() {
        assert!(parse_windows_proxy(false, "proxy:8080", "").is_empty());
        assert!(parse_windows_proxy(true, "  ", "").is_empty());

        let single = parse_windows_proxy(true, "proxy.example:8080", "<local>;*.corp.example");
        let get = |vars: &[(String, String)], name: &str| {
            vars.iter().find(|(n, _)| n == name).map(|(_, v)| v.clone())
        };
        assert_eq!(
            get(&single, "HTTPS_PROXY").as_deref(),
            Some("http://proxy.example:8080")
        );
        assert_eq!(
            get(&single, "HTTP_PROXY").as_deref(),
            Some("http://proxy.example:8080")
        );
        assert_eq!(
            get(&single, "NO_PROXY").as_deref(),
            Some("localhost,127.0.0.1,::1,.corp.example")
        );

        let per_protocol = parse_windows_proxy(true, "http=h1:1;https=h2:2;socks=h3:3", "");
        assert_eq!(
            get(&per_protocol, "HTTP_PROXY").as_deref(),
            Some("http://h1:1")
        );
        assert_eq!(
            get(&per_protocol, "HTTPS_PROXY").as_deref(),
            Some("http://h2:2")
        );
        assert_eq!(get(&per_protocol, "ALL_PROXY"), None);

        let socks = parse_windows_proxy(true, "socks=h3:3", "");
        assert_eq!(get(&socks, "ALL_PROXY").as_deref(), Some("socks5h://h3:3"));

        let with_scheme = parse_windows_proxy(true, "https=https://h4:4", "");
        assert_eq!(
            get(&with_scheme, "HTTPS_PROXY").as_deref(),
            Some("https://h4:4")
        );
    }

    #[tokio::test]
    async fn apply_sets_path_and_proxy_and_removes_the_nested_session_markers() {
        let env = ExecEnv {
            path: Some("/opt/demo/bin".into()),
            proxy: vec![("HTTPS_PROXY".into(), "http://p:1".into())],
            ..ExecEnv::inherit()
        };
        let mut command = tokio::process::Command::new("unused");
        env.apply(&mut command);
        let envs: Vec<(String, Option<String>)> = command
            .as_std()
            .get_envs()
            .map(|(n, v)| {
                (
                    n.to_string_lossy().into_owned(),
                    v.map(|v| v.to_string_lossy().into_owned()),
                )
            })
            .collect();
        assert!(envs.contains(&("PATH".into(), Some("/opt/demo/bin".into()))));
        assert!(envs.contains(&("HTTPS_PROXY".into(), Some("http://p:1".into()))));
        for name in REMOVED_VARS {
            assert!(
                envs.contains(&(name.to_string(), None)),
                "{name} must be removed"
            );
        }
        let mut untouched = tokio::process::Command::new("unused");
        ExecEnv::inherit().apply(&mut untouched);
        assert!(
            untouched
                .as_std()
                .get_envs()
                .all(|(n, v)| v.is_none() && REMOVED_VARS.contains(&n.to_str().unwrap()))
        );
    }

    // Every source can be slow (the login shell runs the user's rc files) and a fresh check reads
    // them while the readiness check holds its lock: they are read together, not one after the
    // other.
    #[tokio::test(start_paused = true)]
    async fn the_login_shell_and_the_system_settings_are_read_at_the_same_time() {
        use std::time::Duration;

        let started = tokio::time::Instant::now();
        let shell = async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(vars(&[("PATH", "/opt/demo/bin")]))
        };
        let system_proxy = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok(vec![(
                "HTTPS_PROXY".to_string(),
                "http://127.0.0.1:7890".to_string(),
            )])
        };
        let system_path = async {
            tokio::time::sleep(Duration::from_secs(5)).await;
            Ok((None, None))
        };
        let read = read_sources_from(Vars::new(), shell, system_proxy, system_path).await;
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(10),
            "one after the other would take 20 s"
        );
        let env = combine(&read, None);
        assert_eq!(env.path_source, EnvSource::LoginShell);
        assert_eq!(env.proxy_source, EnvSource::SystemSettings);
    }

    #[cfg(unix)]
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-shell");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    // The pipe delivers one byte at a time, so the marker is split across reads at every
    // position, and it never ends: the read has to stop at the marker.
    #[cfg(unix)]
    #[tokio::test]
    async fn reading_stops_at_the_end_marker_even_when_it_arrives_in_pieces() {
        use tokio::io::AsyncWriteExt;

        let (mut far_end, near_end) = tokio::io::duplex(1);
        let text = format!("rc noise\n\n{BEGIN}\nPATH=/opt/demo/bin\n\n{END}\nlater output\n");
        let writer = tokio::spawn(async move {
            let _ = far_end.write_all(text.as_bytes()).await;
            // Not closed: whoever reads to the end of the pipe waits forever.
            std::future::pending::<()>().await;
        });
        let bytes = tokio::time::timeout(Duration::from_secs(5), read_until_end_marker(near_end))
            .await
            .expect("the read waited for the end of the pipe")
            .unwrap();
        writer.abort();
        let text = String::from_utf8(bytes).unwrap();
        assert!(text.ends_with(END), "{text:?}");
        assert_eq!(parse_env_output(&text).unwrap()["PATH"], "/opt/demo/bin");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn reading_without_an_end_marker_goes_on_to_the_end_of_the_pipe() {
        // A marker in the middle of a line is not the marker: it is on a line of its own.
        let text = format!("noise {END} more noise\n").into_bytes();
        let bytes = read_until_end_marker(&text[..]).await.unwrap();
        assert_eq!(bytes, text);
        assert!(read_until_end_marker(&b""[..]).await.unwrap().is_empty());
    }

    /// `shell_vars`, trying again while the OS reports the fake shell as busy: executing a script
    /// right after writing it can fail with `ETXTBSY` when another test thread forks in between
    /// (`tests/doctor.rs::run_script` does the same).
    #[cfg(unix)]
    async fn shell_vars_retrying(shell: &Path, limit: Duration) -> Result<Vars, String> {
        let busy = format!("(os error {})", libc::ETXTBSY);
        let mut retries = 0;
        loop {
            match shell_vars(shell, limit).await {
                Err(error) if retries < 5 && error.contains(&busy) => {
                    retries += 1;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                other => return other,
            }
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_login_shell_reports_its_path_and_proxy() {
        let dir = tempfile::tempdir().unwrap();
        // The fake shell gets `-l -i -c <script>`: it prints rc-file noise, sets what a profile
        // would, then runs the script it was given.
        let shell = fake_shell(
            dir.path(),
            "echo 'welcome from the rc file'\nPATH=/opt/demo/bin:$PATH; export PATH\nHTTPS_PROXY=http://127.0.0.1:9; export HTTPS_PROXY\neval \"$4\"",
        );
        let vars = shell_vars_retrying(&shell, Duration::from_secs(5))
            .await
            .unwrap();
        assert!(vars["PATH"].starts_with("/opt/demo/bin:"));
        assert_eq!(vars["HTTPS_PROXY"], "http://127.0.0.1:9");
    }

    // An rc file can leave a process running that holds the shell's stdout (a daemon started from
    // `.zshrc`): the pipe then does not end when the shell does. The variables are complete once the
    // end marker has arrived, so the read stops there, and the session, that process included, is
    // ended.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_process_the_rc_file_leaves_holding_stdout_does_not_delay_the_result() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("background.pid");
        let shell = fake_shell(
            dir.path(),
            &format!(
                "sleep 30 &\necho $! > \"{}\"\nPATH=/opt/demo/bin:$PATH; export PATH\neval \"$4\"",
                pid_file.display()
            ),
        );
        let started = std::time::Instant::now();
        let vars = shell_vars_retrying(&shell, Duration::from_secs(5))
            .await
            .unwrap();
        let took = started.elapsed();
        assert!(
            took < Duration::from_secs(2),
            "waited for the pipe to end: {took:?}"
        );
        assert!(vars["PATH"].starts_with("/opt/demo/bin:"));

        let pid: libc::pid_t = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let give_up = std::time::Instant::now() + Duration::from_secs(5);
        // SAFETY: signal 0 sends nothing; it only reports whether the process exists.
        while unsafe { libc::kill(pid, 0) } == 0 {
            if std::time::Instant::now() >= give_up {
                // SAFETY: a plain signal call for the process this test started.
                unsafe { libc::kill(pid, libc::SIGKILL) };
                panic!("the process the rc file left running was not ended");
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hanging_login_shell_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 30");
        let started = std::time::Instant::now();
        let error = shell_vars_retrying(&shell, Duration::from_millis(300))
            .await
            .unwrap_err();
        assert!(error.contains("did not finish"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(5));
        let missing = shell_vars(&dir.path().join("no-such-shell"), Duration::from_secs(1))
            .await
            .unwrap_err();
        assert!(missing.contains("cannot start"), "{missing}");
    }
}
