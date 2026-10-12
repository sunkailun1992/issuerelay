//! The environment of executor runs and check commands: the PATH `claude` and `codex` are
//! found on, and the proxy they reach Linear through.
//!
//! A desktop app started from Finder or at login gets launchd's minimal PATH, without Homebrew
//! or npm's global directory, and Claude Code ignores the macOS system proxy: it reads only
//! `HTTPS_PROXY` and its relatives (design section 8; M1 acceptance H1). `resolve` looks at the
//! host's own environment, then the user's login shell, then the system proxy settings, and
//! `ExecEnv::apply` hands the result to every child.

use std::collections::{BTreeMap, HashMap, HashSet};
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
    /// The operating system's proxy settings.
    SystemSettings,
    /// Nowhere: no proxy is set.
    None,
}

/// The environment children get on top of the host's own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

/// `url` with its user-info part (`user:password@`) replaced by `***@`. Text that is not a URL
/// with an authority is returned as it is.
pub fn mask_credentials(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let authority_end = rest.find('/').unwrap_or(rest.len());
    match rest[..authority_end].rfind('@') {
        Some(at) => format!("{scheme}://***@{}", &rest[at + 1..]),
        None => url.to_string(),
    }
}

fn interesting(name: &str) -> bool {
    name == "PATH" || PROXY_VARS.contains(&name)
}

fn has_proxy(vars: &Vars) -> bool {
    PROXY_SETTING_VARS
        .iter()
        .any(|name| vars.get(*name).is_some_and(|v| !v.trim().is_empty()))
}

/// The host's own PATH and proxy variables. Variables that are not valid Unicode are skipped
/// (`std::env::vars` would panic on any such variable, even an unrelated one).
fn host_vars() -> Vars {
    std::env::vars_os()
        .filter_map(|(name, value)| {
            let name = name.into_string().ok()?;
            interesting(&name).then_some(())?;
            Some((name, value.into_string().ok()?))
        })
        .collect()
}

/// Works out the environment children should get (see the module documentation). Never fails:
/// what could not be read is listed in `problems`.
pub async fn resolve() -> ExecEnv {
    let host = host_vars();
    let shell = login_shell_vars().await;
    let system = system_proxy().await;
    combine(&host, shell, system)
}

/// Puts the three sources together. PATH: the login shell's, followed by the host's entries it
/// lacks; without a login shell PATH, the host's. Proxy: the host's own when it sets one, else
/// the login shell's, else the system settings'.
pub fn combine(
    host: &Vars,
    shell: Result<Vars, String>,
    system: Result<Vec<(String, String)>, String>,
) -> ExecEnv {
    let mut problems = Vec::new();
    let shell = shell.unwrap_or_else(|e| {
        problems.push(format!("could not read the login shell's environment: {e}"));
        Vars::new()
    });
    let system = system.unwrap_or_else(|e| {
        problems.push(format!("could not read the system proxy settings: {e}"));
        Vec::new()
    });
    let (path, path_source) = match shell.get("PATH").filter(|p| !p.trim().is_empty()) {
        Some(shell_path) => (
            Some(merge_paths(
                shell_path,
                host.get("PATH").map(String::as_str),
            )),
            EnvSource::LoginShell,
        ),
        None => (None, EnvSource::Host),
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

/// Windows has no login shell: a program started from the Start menu or at login already gets
/// the user's full PATH from the system.
#[cfg(windows)]
async fn login_shell_vars() -> Result<Vars, String> {
    Ok(Vars::new())
}

/// Runs `shell -l -i -c <script>`, where the script prints `env` between two markers. The shell
/// leads a session and a process group of its own; the group, with anything the rc files left
/// running in it, is killed once the output is read, or after `limit` if the shell has not
/// finished by then.
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
    use tokio::io::AsyncReadExt;

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
    let read = async {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).await.map(|_| bytes)
    };
    let outcome = tokio::time::timeout(limit, read).await;
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

    #[test]
    fn the_hosts_own_proxy_wins_and_is_passed_on_unchanged() {
        let host = vars(&[("PATH", "/usr/bin"), ("HTTPS_PROXY", "http://host-proxy:1")]);
        let shell = Ok(vars(&[
            ("PATH", "/opt/demo/bin:/usr/bin"),
            ("HTTPS_PROXY", "http://shell-proxy:2"),
        ]));
        let env = combine(
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
        let env = combine(
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
        let env = combine(&host, Ok(Vars::new()), Ok(system.clone()));
        assert_eq!(env.proxy_source, EnvSource::SystemSettings);
        assert_eq!(env.proxy, system);
        assert_eq!(
            env.path, None,
            "no login shell PATH: the host's PATH is kept"
        );
        assert_eq!(env.path_source, EnvSource::Host);

        let none = combine(&host, Ok(Vars::new()), Ok(Vec::new()));
        assert_eq!(none.proxy_source, EnvSource::None);
        assert!(none.proxy.is_empty());
    }

    #[test]
    fn what_could_not_be_read_is_listed_as_a_problem() {
        let env = combine(
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

    #[cfg(unix)]
    fn fake_shell(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("fake-shell");
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
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
        let vars = shell_vars(&shell, Duration::from_secs(5)).await.unwrap();
        assert!(vars["PATH"].starts_with("/opt/demo/bin:"));
        assert_eq!(vars["HTTPS_PROXY"], "http://127.0.0.1:9");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hanging_login_shell_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let shell = fake_shell(dir.path(), "sleep 30");
        let started = std::time::Instant::now();
        let error = shell_vars(&shell, Duration::from_millis(300))
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
