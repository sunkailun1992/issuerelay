//! Development-only headless host for the IssueRelay engine. Not shipped; the M2 tray app
//! embeds the engine the same way.
use std::{
    io::{self, Read, Write},
    process::ExitCode,
    sync::Arc,
    time::Duration,
};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use issuerelay_core::{
    doctor::SystemReadiness,
    engine::{Engine, EngineDeps, ProcessJobRunner},
    ipc::server::IpcServer,
    linear::{
        LinearApi, auth,
        mcp::{LinearClient, McpLinear},
    },
    paths::AppPaths,
    store::Store,
    text::clean_text,
};
use tokio_util::sync::CancellationToken;

#[derive(Parser)]
#[command(
    name = "issuerelay-devhost",
    about = "Development-only headless host for IssueRelay (not shipped)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Authorize with Linear in the browser and keep the token in the system keychain.
    Login,
    /// Read a Linear personal API key from stdin and keep it in the system keychain.
    ApiKey,
    /// Forget stored Linear credentials.
    Logout,
    /// List Linear projects (id and name) for `issuerelay bind add`.
    Projects,
    /// Run the engine and the IPC server until Ctrl+C.
    Run,
}

/// Connects with the credentials in the system keychain: the OAuth credentials that `login`
/// stored first, then a saved personal API key. Stored OAuth credentials that cannot be read are
/// an error that says to log in again (from `auth::connect_stored`), not a reason to try the API
/// key.
async fn connect() -> Result<LinearClient> {
    if let Some(client) = auth::connect_stored(auth::LINEAR_MCP_URL).await? {
        return Ok(client);
    }
    if let Some(key) = auth::load_api_key()? {
        return auth::connect_api_key(auth::LINEAR_MCP_URL, &key).await;
    }
    bail!("no Linear credentials; run `issuerelay-devhost login` (or `api-key`) first")
}

/// Runs one subcommand. The viewer's name and the projects' ids and names come from Linear, whose
/// content is untrusted and may carry terminal escape sequences, so they are printed through
/// `clean_text`.
async fn run(command: Command) -> Result<()> {
    match command {
        Command::Login => {
            let client = auth::login(auth::LINEAR_MCP_URL, |url| {
                // Not passed through `clean_text`: rmcp serializes the URL with `url::Url`, so it
                // holds no control characters, and `clean_text` would cut a long one.
                println!("Open this URL to authorize IssueRelay:\n{url}");
                let _ = open::that_detached(url);
            })
            .await?;
            let linear = McpLinear::new(client).await?;
            let viewer = linear.viewer().await?;
            println!("Logged in to Linear as {}", clean_text(&viewer.name));
            linear.close().await
        }
        Command::ApiKey => {
            let mut key = String::new();
            io::stdin().read_to_string(&mut key)?;
            let key = key.trim();
            if key.is_empty() {
                bail!("no API key on stdin");
            }
            let linear =
                McpLinear::new(auth::connect_api_key(auth::LINEAR_MCP_URL, key).await?).await?;
            let viewer = linear.viewer().await?;
            println!("API key works for {}", clean_text(&viewer.name));
            auth::save_api_key(key)?;
            linear.close().await
        }
        Command::Logout => {
            auth::clear_credentials()?;
            println!("Stored Linear credentials removed.");
            Ok(())
        }
        Command::Projects => {
            let linear = McpLinear::new(connect().await?).await?;
            for project in linear.projects().await? {
                println!("{}\t{}", clean_text(&project.id), clean_text(&project.name));
            }
            linear.close().await
        }
        Command::Run => {
            let paths = AppPaths::system()?;
            let server = IpcServer::bind(&paths.ipc_endpoint()).await?;
            let linear = Arc::new(McpLinear::new(connect().await?).await?);
            let engine = Engine::new(EngineDeps {
                config_path: paths.config_file(),
                store: Store::open(&paths.state_db())?,
                linear,
                readiness: Arc::new(SystemReadiness::new(Duration::from_secs(300))),
                job_runner: Arc::new(ProcessJobRunner),
                logs_dir: paths.logs_dir(),
            })?;
            let shutdown = CancellationToken::new();
            let ipc = tokio::spawn(server.run(engine.clone(), shutdown.clone()));
            let worker = {
                let engine = engine.clone();
                let shutdown = shutdown.clone();
                tokio::spawn(async move { engine.run_forever(shutdown).await })
            };
            tracing::info!("IssueRelay devhost is running; press Ctrl+C to stop");
            tokio::signal::ctrl_c().await?;
            shutdown.cancel();
            let _ = worker.await;
            let _ = ipc.await;
            engine.wait_idle().await;
            Ok(())
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    // Logs go to stderr: stdout carries only what a subcommand prints as its result.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("info,issuerelay_core=debug")
            }),
        )
        .with_writer(io::stderr)
        .init();
    match run(Cli::parse().command).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // One line with the whole cause chain. The chain can carry Linear's text, so it is
            // cleaned. If stderr is gone there is nobody to tell, and the exit code still says
            // the command failed.
            let _ = writeln!(io::stderr(), "Error: {}", clean_text(&format!("{error:#}")));
            ExitCode::FAILURE
        }
    }
}
