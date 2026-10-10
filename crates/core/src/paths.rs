//! Where IssueRelay keeps its configuration, state, logs and IPC endpoint.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const APP_ID: &str = "com.sunkailun.issuerelay";
/// macOS limits Unix socket paths to 104 bytes; stay well below it.
const UNIX_SOCKET_MAX: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IpcEndpoint {
    Unix(PathBuf),
    Pipe(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppPaths {
    pub config_dir: PathBuf,
    pub data_dir: PathBuf,
}

impl AppPaths {
    /// Config may roam with the user's profile; runtime data (database, logs) stays on this
    /// machine: Windows' local app data, the same Application Support folder on macOS.
    pub fn system() -> Result<Self> {
        let config = dirs::config_dir().context("this system has no config directory")?;
        let data = dirs::data_local_dir().context("this system has no local data directory")?;
        Ok(Self {
            config_dir: config.join(APP_ID),
            data_dir: data.join(APP_ID),
        })
    }

    pub fn under(root: &Path) -> Self {
        Self {
            config_dir: root.join("config"),
            data_dir: root.join("data"),
        }
    }

    pub fn config_file(&self) -> PathBuf {
        self.config_dir.join("config.toml")
    }

    pub fn state_db(&self) -> PathBuf {
        self.data_dir.join("state.db")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// The Unix socket (macOS) or named pipe (Windows) the CLI uses to reach the running host.
    pub fn ipc_endpoint(&self) -> IpcEndpoint {
        let user = sanitize(
            &std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .unwrap_or_default(),
        );
        let tag = short_hash(&self.data_dir.to_string_lossy());
        if cfg!(windows) {
            return IpcEndpoint::Pipe(format!(r"\\.\pipe\issuerelay-{user}-{tag}"));
        }
        let preferred = self.data_dir.join("issuerelay.sock");
        if preferred.as_os_str().len() <= UNIX_SOCKET_MAX {
            IpcEndpoint::Unix(preferred)
        } else {
            IpcEndpoint::Unix(
                std::env::temp_dir()
                    .join(format!("issuerelay-{user}-{tag}"))
                    .join("issuerelay.sock"),
            )
        }
    }
}

fn sanitize(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    if cleaned.is_empty() {
        "user".into()
    } else {
        cleaned
    }
}

/// FNV-1a; only needs to be stable and spread out, not cryptographic.
fn short_hash(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{:08x}", (hash >> 32) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn under_builds_expected_files() {
        let root = Path::new("/tmp/issuerelay-test");
        let p = AppPaths::under(root);
        assert_eq!(p.config_file(), root.join("config").join("config.toml"));
        assert_eq!(p.state_db(), root.join("data").join("state.db"));
        assert_eq!(p.logs_dir(), root.join("data").join("logs"));
    }

    #[test]
    fn endpoint_is_stable_and_unique_per_data_dir() {
        let a = AppPaths::under(Path::new("/tmp/a"));
        let b = AppPaths::under(Path::new("/tmp/b"));
        assert_eq!(a.ipc_endpoint(), a.ipc_endpoint());
        assert_ne!(a.ipc_endpoint(), b.ipc_endpoint());
    }

    #[cfg(unix)]
    #[test]
    fn long_socket_path_falls_back_to_temp_dir() {
        let long = format!("/tmp/{}", "d".repeat(120));
        match AppPaths::under(Path::new(&long)).ipc_endpoint() {
            IpcEndpoint::Unix(path) => {
                assert!(path.starts_with(std::env::temp_dir()));
                assert!(path.as_os_str().len() < 104 + 40);
            }
            other => panic!("expected a unix socket, got {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_uses_a_named_pipe() {
        match AppPaths::under(Path::new(r"C:\issuerelay-test")).ipc_endpoint() {
            IpcEndpoint::Pipe(name) => assert!(name.starts_with(r"\\.\pipe\issuerelay-")),
            other => panic!("expected a named pipe, got {other:?}"),
        }
    }

    #[test]
    fn system_paths_end_with_the_app_id() {
        let paths = AppPaths::system().unwrap();
        assert!(paths.config_dir.ends_with(APP_ID));
        assert!(paths.data_dir.ends_with(APP_ID));
    }
}
