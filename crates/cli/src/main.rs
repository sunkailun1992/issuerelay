//! The `issuerelay` command: sends each subcommand to the running engine host over local IPC,
//! or serves the MCP control tools with `issuerelay mcp`.
use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use issuerelay_cli::{format, mcp};
use issuerelay_core::{
    config::{Binding, ExecutorKind, PermissionProfile},
    ipc::client,
    paths::AppPaths,
};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(
    name = "issuerelay",
    version,
    about = "Control the running IssueRelay engine"
)]
struct Cli {
    /// Print the raw JSON result.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Show monitoring status.
    Status,
    /// Pause monitoring (runs in progress continue).
    Pause,
    /// Resume monitoring.
    Resume,
    /// Check Linear now.
    Poll,
    /// Check executors, the Linear workspace and bindings.
    Doctor,
    /// List recent runs.
    Jobs {
        #[arg(long, default_value_t = 20)]
        limit: u64,
    },
    /// Show the last lines of a run's log.
    Logs {
        job_id: i64,
        #[arg(long, default_value_t = 50)]
        lines: u64,
    },
    /// Manage Linear project bindings.
    #[command(subcommand)]
    Bind(BindCommand),
    /// Run the MCP control server on stdio (for Claude Code / Codex).
    Mcp,
}

#[derive(Subcommand)]
enum BindCommand {
    /// List the saved bindings.
    List,
    /// Bind a Linear project to a local directory (replaces a binding with the same id).
    Add {
        /// The Linear project's uuid, not its `P-<TEAM>-N` identifier.
        #[arg(long)]
        project_id: String,
        /// The project's name, shown in lists and checks.
        #[arg(long)]
        project_name: String,
        /// The directory runs start in; it must exist.
        #[arg(long)]
        dir: PathBuf,
        /// `claude` or `codex`; omit it to use the default executor from the settings.
        #[arg(long, value_parser = parse_executor)]
        executor: Option<ExecutorKind>,
        /// `full`, `auto` or `personal`; omit it to use the default from the settings.
        #[arg(long, value_parser = parse_permission)]
        permission: Option<PermissionProfile>,
    },
    /// Remove a binding by its id (the project id it was added with).
    Remove { id: String },
}

fn parse_executor(text: &str) -> Result<ExecutorKind, String> {
    ExecutorKind::parse(text).ok_or_else(|| format!("expected `claude` or `codex`, got `{text}`"))
}

fn parse_permission(text: &str) -> Result<PermissionProfile, String> {
    PermissionProfile::parse(text)
        .ok_or_else(|| format!("expected `full`, `auto` or `personal`, got `{text}`"))
}

/// The IPC method and parameters a subcommand stands for.
fn request(command: &Command) -> Result<(&'static str, Value)> {
    Ok(match command {
        Command::Status => ("status", json!({})),
        Command::Pause => ("pause", json!({})),
        Command::Resume => ("resume", json!({})),
        Command::Poll => ("poll_now", json!({})),
        Command::Doctor => ("doctor", json!({})),
        Command::Jobs { limit } => ("list_jobs", json!({"limit": limit})),
        Command::Logs { job_id, lines } => ("logs", json!({"job_id": job_id, "lines": lines})),
        Command::Bind(BindCommand::List) => ("bindings.list", json!({})),
        Command::Bind(BindCommand::Remove { id }) => ("bindings.remove", json!({"id": id})),
        Command::Bind(BindCommand::Add {
            project_id,
            project_name,
            dir,
            executor,
            permission,
        }) => {
            // The engine does not look at the directory when saving, so a typo here would
            // otherwise give a binding whose jobs wait forever for a missing directory.
            if !dir.is_dir() {
                bail!("`{}` is not a directory", dir.display());
            }
            let binding = Binding {
                id: project_id.clone(),
                project_id: project_id.clone(),
                project_name: project_name.clone(),
                directory: std::path::absolute(dir)?,
                executor: *executor,
                permission: *permission,
                model: None,
                prompt_override: None,
            };
            ("bindings.add", json!({"binding": binding}))
        }
        Command::Mcp => unreachable!("handled before sending a request"),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let endpoint = AppPaths::system()?.ipc_endpoint();
    if matches!(cli.command, Command::Mcp) {
        return mcp::serve(endpoint).await;
    }
    let (method, params) = request(&cli.command)?;
    let value = client::call(&endpoint, method, params).await?;
    if cli.json {
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else {
        println!("{}", format::human(method, &value));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::CommandFactory;

    use super::*;

    fn collect_names(command: &clap::Command, names: &mut Vec<String>) {
        for sub in command.get_subcommands() {
            names.push(sub.get_name().to_ascii_lowercase());
            collect_names(sub, names);
        }
    }

    #[test]
    fn no_subcommand_marks_done_retries_or_cancels() {
        // Only the desktop panel may do these (AGENTS.md); the CLI must not offer them in any
        // spelling, at any depth of its command tree.
        let mut names = Vec::new();
        collect_names(&Cli::command(), &mut names);
        assert!(
            names.iter().any(|n| n == "status") && names.iter().any(|n| n == "add"),
            "the walk should reach top-level and nested subcommands: {names:?}"
        );
        for name in &names {
            for word in ["done", "retry", "cancel"] {
                assert!(
                    !name.contains(word),
                    "`{name}` looks like a panel-only action"
                );
            }
        }
    }

    fn bind_add(dir: &Path) -> Command {
        Command::Bind(BindCommand::Add {
            project_id: "proj-1".into(),
            project_name: "Demo Project".into(),
            dir: dir.to_path_buf(),
            executor: Some(ExecutorKind::Codex),
            permission: None,
        })
    }

    #[test]
    fn bind_add_refuses_a_directory_that_does_not_exist() {
        let root = tempfile::tempdir().unwrap();
        let missing = root.path().join("typo");
        let error = request(&bind_add(&missing)).unwrap_err();
        assert_eq!(
            error.to_string(),
            format!("`{}` is not a directory", missing.display())
        );
        // A file is no directory either.
        let file = root.path().join("notes.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(
            request(&bind_add(&file))
                .unwrap_err()
                .to_string()
                .ends_with("is not a directory")
        );
    }

    #[test]
    fn bind_add_sends_the_existing_directory_as_an_absolute_path() {
        // `.` exists wherever the tests run and is relative, which `absolute` must resolve.
        let (method, params) = request(&bind_add(Path::new("."))).unwrap();
        assert_eq!(method, "bindings.add");
        let binding = &params["binding"];
        assert_eq!(binding["id"], "proj-1");
        assert_eq!(binding["project_id"], "proj-1");
        assert_eq!(binding["executor"], "codex");
        let sent = PathBuf::from(binding["directory"].as_str().unwrap());
        assert_eq!(sent, std::path::absolute(".").unwrap());
        assert!(sent.is_absolute());
    }
}
