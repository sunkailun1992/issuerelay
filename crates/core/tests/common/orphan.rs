//! An executor that a crashed host left behind, for the tests of the sweep that ends such
//! executors (`runner::end_leftover`, macOS only): fake-agent or a shell started the way a run
//! starts an executor, leading a process group of its own, but owned by no run.
use std::{
    path::Path,
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};

/// Owns the leftover's leader. Dropping it ends the leader's process group and reaps the leader,
/// unless the test reaped it already, so a test leaves nothing of the group running however it
/// ends. A descendant that left the group is out of its reach: fake-agent's heartbeats stop by
/// themselves once their file is gone with the test's directory.
pub struct Orphan {
    child: Child,
    /// Set once the leader was reaped: from then on its PID, which is also the group id, may
    /// belong to another process, so the group is not signalled any more.
    reaped: bool,
}

impl Orphan {
    /// Starts fake-agent with `script`, leading a process group of its own.
    pub fn spawn(script: &Path) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_fake-agent"));
        command.arg(script);
        Self::start(command)
    }

    /// Starts `/bin/sh -c <script> sh <args…>`, leading a process group of its own: for an
    /// executor that handles SIGTERM, which fake-agent does not.
    pub fn shell(script: &str, args: &[&Path]) -> Self {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", script, "sh"]).args(args);
        Self::start(command)
    }

    fn start(mut command: Command) -> Self {
        use std::os::unix::process::CommandExt;
        let child = command
            .process_group(0)
            .spawn()
            .expect("start the leftover");
        Self {
            child,
            reaped: false,
        }
    }

    /// The leader's PID.
    pub fn id(&self) -> u32 {
        self.child.id()
    }

    /// Whether the leader still runs; a leader that has exited is reaped.
    pub fn is_running(&mut self) -> bool {
        match self.child.try_wait().expect("check on fake-agent") {
            Some(_) => {
                self.reaped = true;
                false
            }
            None => true,
        }
    }

    /// The leader's exit status once it has exited, waiting at most `limit`; `None` while it
    /// still runs. A leader that has exited is reaped.
    pub async fn exit_within(&mut self, limit: Duration) -> Option<ExitStatus> {
        let give_up = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().expect("check on fake-agent") {
                self.reaped = true;
                return Some(status);
            }
            if Instant::now() >= give_up {
                return None;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Orphan {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
            // SAFETY: a plain signal call. The leader has not been reaped, so its PID, and with
            // it the group id, still belongs to the group it created.
            unsafe { libc::killpg(pid, libc::SIGKILL) };
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
