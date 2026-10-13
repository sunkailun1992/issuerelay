//! The commands the panel calls. Each waits for the host, does one thing and returns something
//! the panel can show; an error comes back as one cleaned line, or as a code the panel
//! translates (`not_a_linear_link`, `login_cancelled`). `mark_done`, `retry_job` and
//! `cancel_job` exist only here (design 5.6): IPC, the CLI and MCP never offer them. Plugins
//! (opener, dialog, autostart) are used from here, so the panel itself needs no plugin
//! permission. `Host::env()` holds the real proxy values (user name and password unmasked): the
//! commands use it only to start processes and never return it or anything taken from it; the
//! environment the panel sees is `Engine::doctor`'s, with the proxy values masked.
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::Context as _;
use issuerelay_core::{
    config::{Binding, ExecutorKind, Settings},
    doctor::{CommandRunner as _, DoctorReport, SystemRunner},
    engine::{IssueView, StatusSnapshot},
    executor,
    host::{Host, LinearSetup},
    linear::{Project, Viewer},
    model::Job,
    setup::{self, SetupAction},
    text::clean_text,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_dialog::DialogExt as _;
use tauri_plugin_opener::OpenerExt as _;
use tokio_util::sync::CancellationToken;

use crate::{
    quit::{self, QuitEvent},
    state::{AppState, StartupState},
    tray,
};

type CmdResult<T> = Result<T, String>;

/// The error code `linear_login` answers with when `linear_cancel_login` dropped it; the panel
/// shows no failure for it.
const LOGIN_CANCELLED: &str = "login_cancelled";

/// The browser login that is waiting, so that `linear_cancel_login` can drop it (Tauri managed
/// state, next to `AppState`).
#[derive(Default)]
pub struct PendingLogin(Mutex<Option<CancellationToken>>);

impl PendingLogin {
    /// The token of a login that starts now. A login still waiting is cancelled first: the panel
    /// offers one login at a time.
    fn begin(&self) -> CancellationToken {
        let token = CancellationToken::new();
        let earlier = self
            .0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .replace(token.clone());
        if let Some(earlier) = earlier {
            earlier.cancel();
        }
        token
    }

    /// Cancels the login that is waiting, if there is one.
    fn cancel(&self) {
        let waiting = self.0.lock().unwrap_or_else(PoisonError::into_inner).take();
        if let Some(waiting) = waiting {
            waiting.cancel();
        }
    }
}

/// The most jobs one list returns.
const MAX_JOBS: usize = 500;
/// The most log lines one read returns.
const MAX_LOG_LINES: usize = 5000;
/// How long `mcp get` may take.
const MCP_GET_LIMIT: Duration = Duration::from_secs(60);
/// The error code `open_linear` answers with for any other link; the panel translates it.
const NOT_A_LINEAR_LINK: &str = "not_a_linear_link";

/// An error as the panel shows it: the whole chain on one cleaned line (`clean_text`), since
/// core's errors can carry text from Linear, the executors or the system.
fn error(e: impl Into<anyhow::Error>) -> String {
    clean_text(&format!("{:#}", e.into()))
}

fn clamp_limit(limit: usize, max: usize) -> usize {
    limit.clamp(1, max)
}

/// The only links IssueRelay opens in the browser, from the panel (`open_linear`) and for the
/// Linear login (its authorization page): `https` on `linear.app` or one of its subdomains, such
/// as `mcp.linear.app`. Parsed, so user info (`https://linear.app@evil.example/`) and look-alike
/// hosts (`linear.app.evil.example`) are refused.
fn is_linear_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host == "linear.app" || host.ends_with(".linear.app"))
    })
}

/// Where the command-line program is installed: next to the app's own executable.
fn cli_next_to(exe: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "issuerelay.exe"
    } else {
        "issuerelay"
    };
    exe.with_file_name(name)
}

/// The command-line program, when it is installed next to the app (always in the installed
/// app; in development after `cargo build -p issuerelay-cli`). Always derived here, never taken
/// from the panel: it is the one variable argument of the setup commands.
fn cli_path() -> Option<PathBuf> {
    let cli = cli_next_to(&std::env::current_exe().ok()?);
    cli.is_file().then_some(cli)
}

fn action_executor(action: SetupAction) -> ExecutorKind {
    match action {
        SetupAction::AddLinearMcp(kind)
        | SetupAction::LoginLinearMcp(kind)
        | SetupAction::RegisterControlServer(kind)
        | SetupAction::RemoveControlServer(kind) => kind,
    }
}

/// The server name the one-click login passes to `mcp login`, read from core's own arguments so
/// the two never differ.
fn linear_server_name(kind: ExecutorKind) -> anyhow::Result<String> {
    setup::arguments(SetupAction::LoginLinearMcp(kind), None)?
        .last()
        .cloned()
        .context("the login command names no server")
}

/// The quit prompt's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuitMode {
    Wait,
    Stop,
}

#[derive(Serialize)]
pub struct StatusView {
    pub status: StatusSnapshot,
    pub linear: LinearSetup,
}

#[derive(Serialize)]
pub struct AppInfo {
    pub version: &'static str,
    pub cli_path: Option<String>,
    pub platform: &'static str,
    /// The system language as Rust reads it (`sys_locale`). The panel resolves "system" from it,
    /// so the panel, the tray and the notifications always agree.
    pub system_locale: Option<String>,
}

/// Whether IssueRelay's control server is registered with one executor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlServer {
    Registered,
    NotRegistered,
    NotInstalled,
    /// The check itself failed (timed out, did not start, was killed); it was logged and the
    /// panel offers to check again.
    Unknown,
}

#[derive(Serialize)]
pub struct ControlServers {
    pub claude: ControlServer,
    pub codex: ControlServer,
}

/// What a registration check of an installed executor came to; a failed check is logged.
fn registration(kind: ExecutorKind, checked: anyhow::Result<bool>) -> ControlServer {
    match checked {
        Ok(true) => ControlServer::Registered,
        Ok(false) => ControlServer::NotRegistered,
        Err(e) => {
            tracing::warn!(executor = kind.as_str(), error = %format!("{e:#}"), "could not check the control server registration");
            ControlServer::Unknown
        }
    }
}

async fn host(state: &State<'_, AppState>) -> CmdResult<Host> {
    state.host().await
}

#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        cli_path: cli_path().map(|p| p.display().to_string()),
        platform: std::env::consts::OS,
        system_locale: sys_locale::get_locale(),
    }
}

/// Starting, ready, or failed with a code and a cleaned detail; answers at once.
#[tauri::command]
pub fn startup_state(state: State<'_, AppState>) -> StartupState {
    state.startup_state()
}

#[tauri::command]
pub async fn status(state: State<'_, AppState>) -> CmdResult<StatusView> {
    let host = host(&state).await?;
    Ok(StatusView {
        status: host.engine().status(),
        linear: host.linear_setup(),
    })
}

#[tauri::command]
pub async fn list_jobs(state: State<'_, AppState>, limit: usize) -> CmdResult<Vec<Job>> {
    host(&state)
        .await?
        .engine()
        .list_jobs(clamp_limit(limit, MAX_JOBS))
        .map_err(error)
}

#[tauri::command]
pub async fn job(state: State<'_, AppState>, id: i64) -> CmdResult<Option<Job>> {
    host(&state).await?.engine().job(id).map_err(error)
}

#[tauri::command]
pub async fn job_log(state: State<'_, AppState>, id: i64, lines: usize) -> CmdResult<Vec<String>> {
    host(&state)
        .await?
        .engine()
        .log_tail(id, clamp_limit(lines, MAX_LOG_LINES))
        .map_err(error)
}

#[tauri::command]
pub async fn issues(state: State<'_, AppState>) -> CmdResult<Vec<IssueView>> {
    host(&state).await?.engine().issues().await.map_err(error)
}

#[tauri::command]
pub async fn pause(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let host = host(&state).await?;
    host.engine().pause().map_err(error)?;
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn resume(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let host = host(&state).await?;
    host.engine().resume().map_err(error)?;
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn poll_now(state: State<'_, AppState>) -> CmdResult<()> {
    host(&state).await?.engine().poll_now();
    Ok(())
}

#[tauri::command]
pub async fn cancel_job(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    host(&state).await?.engine().cancel_job(id).map_err(error)
}

#[tauri::command]
pub async fn retry_job(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    host(&state).await?.engine().retry_job(id).map_err(error)
}

/// The panel's "mark done": IssueRelay's only write to Linear (design 5.6).
#[tauri::command]
pub async fn mark_done(state: State<'_, AppState>, id: i64) -> CmdResult<()> {
    host(&state)
        .await?
        .engine()
        .mark_done(id)
        .await
        .map_err(error)
}

#[tauri::command]
pub async fn bindings(state: State<'_, AppState>) -> CmdResult<Vec<Binding>> {
    Ok(host(&state).await?.engine().bindings())
}

#[tauri::command]
pub async fn save_binding(state: State<'_, AppState>, binding: Binding) -> CmdResult<()> {
    host(&state)
        .await?
        .engine()
        .add_binding(binding)
        .map_err(error)
}

#[tauri::command]
pub async fn remove_binding(state: State<'_, AppState>, id: String) -> CmdResult<bool> {
    host(&state)
        .await?
        .engine()
        .remove_binding(&id)
        .map_err(error)
}

#[tauri::command]
pub async fn projects(state: State<'_, AppState>) -> CmdResult<Vec<Project>> {
    host(&state).await?.engine().projects().await.map_err(error)
}

#[tauri::command]
pub async fn settings(state: State<'_, AppState>) -> CmdResult<Settings> {
    Ok(host(&state).await?.engine().settings())
}

#[tauri::command]
pub async fn save_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> CmdResult<()> {
    let host = host(&state).await?;
    host.engine().update_settings(settings).map_err(error)?;
    // The language may have changed: the tray menu follows at once.
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn doctor(state: State<'_, AppState>) -> CmdResult<DoctorReport> {
    Ok(host(&state).await?.engine().doctor().await)
}

#[tauri::command]
pub async fn linear_login(
    app: AppHandle,
    state: State<'_, AppState>,
    pending: State<'_, PendingLogin>,
) -> CmdResult<Viewer> {
    let host = host(&state).await?;
    let opener = app.clone();
    // `linear_cancel_login` drops the login while it waits for the browser, which abandons it:
    // core writes the keychain only after the browser has answered (`Host::login`).
    let cancelled = pending.begin();
    let login = host.login(move |url| {
        // The same check as `open_linear`: the browser opens nothing but Linear.
        if !is_linear_url(url) {
            tracing::warn!(url = %clean_text(url), "refused to open an authorization page outside linear.app");
            return;
        }
        if let Err(e) = opener.opener().open_url(url, None::<&str>) {
            tracing::warn!(error = %e, "could not open the authorization page");
        }
    });
    let viewer = tokio::select! {
        viewer = login => viewer.map_err(error)?,
        () = cancelled.cancelled() => return Err(LOGIN_CANCELLED.into()),
    };
    tray::refresh(&app, &host);
    Ok(viewer)
}

/// Gives up the browser login that is waiting (the panel's Cancel); nothing happens when none is.
#[tauri::command]
pub fn linear_cancel_login(pending: State<'_, PendingLogin>) {
    pending.cancel();
}

#[tauri::command]
pub async fn linear_use_api_key(
    app: AppHandle,
    state: State<'_, AppState>,
    key: String,
) -> CmdResult<Viewer> {
    let host = host(&state).await?;
    let viewer = host.use_api_key(key.trim()).await.map_err(error)?;
    tray::refresh(&app, &host);
    Ok(viewer)
}

#[tauri::command]
pub async fn linear_logout(app: AppHandle, state: State<'_, AppState>) -> CmdResult<()> {
    let host = host(&state).await?;
    host.logout().await.map_err(error)?;
    tray::refresh(&app, &host);
    Ok(())
}

#[tauri::command]
pub async fn linear_reconnect(
    app: AppHandle,
    state: State<'_, AppState>,
) -> CmdResult<LinearSetup> {
    let host = host(&state).await?;
    let setup = host.reconnect().await;
    tray::refresh(&app, &host);
    Ok(setup)
}

/// Runs a one-click setup command with the executor found on the resolved PATH. Its output is not
/// shown: the panel checks again afterwards; a failure carries the cleaned output in its error.
#[tauri::command]
pub async fn setup_action(state: State<'_, AppState>, action: SetupAction) -> CmdResult<()> {
    let host = host(&state).await?;
    let env = host.env();
    let program = executor::resolve_program_in(action_executor(action), env.path.as_deref())
        .map_err(error)?;
    let runner = SystemRunner::with_env(env.shared());
    setup::run(action, &program, cli_path().as_deref(), &runner)
        .await
        .map_err(error)?;
    Ok(())
}

#[tauri::command]
pub async fn control_servers(state: State<'_, AppState>) -> CmdResult<ControlServers> {
    let host = host(&state).await?;
    let env = host.env();
    let runner = SystemRunner::with_env(env.clone().shared());
    let mut found = [ControlServer::NotInstalled; 2];
    for (slot, kind) in found
        .iter_mut()
        .zip([ExecutorKind::Claude, ExecutorKind::Codex])
    {
        // Not found on the resolved PATH: not installed. Found: the check decides, and a check
        // that fails is `Unknown`, never "not installed".
        if let Ok(program) = executor::resolve_program_in(kind, env.path.as_deref()) {
            *slot = registration(
                kind,
                setup::control_server_registered(&program, &runner).await,
            );
        }
    }
    Ok(ControlServers {
        claude: found[0],
        codex: found[1],
    })
}

/// Whether Linear's MCP server is registered under the name the one-click login uses. The
/// environment check finds Linear by its URL, whatever the name; under another name the
/// one-click login cannot work and the panel shows the manual command instead.
#[tauri::command]
pub async fn linear_server_named(
    state: State<'_, AppState>,
    executor: ExecutorKind,
) -> CmdResult<bool> {
    let host = host(&state).await?;
    let env = host.env();
    let program = executor::resolve_program_in(executor, env.path.as_deref()).map_err(error)?;
    let name = linear_server_name(executor).map_err(error)?;
    let runner = SystemRunner::with_env(env.shared());
    let output = runner
        .run_limited(&program, &["mcp", "get", &name], MCP_GET_LIMIT)
        .await
        .map_err(error)?;
    Ok(output.code == Some(0))
}

/// Opens a Linear link in the system browser; any other link is refused (and logged) with the
/// code `not_a_linear_link`.
#[tauri::command]
pub async fn open_linear(app: AppHandle, url: String) -> CmdResult<()> {
    if !is_linear_url(&url) {
        tracing::warn!(url = %clean_text(&url), "refused to open a link outside linear.app");
        return Err(NOT_A_LINEAR_LINK.into());
    }
    app.opener().open_url(&url, None::<&str>).map_err(error)
}

#[tauri::command]
pub async fn choose_directory(app: AppHandle) -> CmdResult<Option<String>> {
    let picked = tokio::task::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .map_err(error)?;
    Ok(picked
        .and_then(|path| path.into_path().ok())
        .map(|path| path.display().to_string()))
}

#[tauri::command]
pub async fn autostart_enabled(app: AppHandle) -> CmdResult<bool> {
    app.autolaunch().is_enabled().map_err(error)
}

#[tauri::command]
pub async fn set_autostart(app: AppHandle, enabled: bool) -> CmdResult<()> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    }
    .map_err(error)
}

/// The quit prompt's answer: wait for the runs (still switchable to "stop now"), or stop them.
/// The quit state machine (Task 3) decides what follows; what it returns only matters to the
/// window hook.
#[tauri::command]
pub fn quit(app: AppHandle, mode: QuitMode) {
    let event = match mode {
        QuitMode::Wait => QuitEvent::Wait,
        QuitMode::Stop => QuitEvent::StopNow,
    };
    quit::handle(&app, event);
}

/// The same as the tray's Quit; the startup error page uses it when the host never started.
#[tauri::command]
pub fn request_quit(app: AppHandle) {
    quit::request(&app);
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;

    use super::*;

    #[test]
    fn only_linear_links_are_opened() {
        assert!(is_linear_url("https://linear.app/demo/issue/DEMO-1"));
        // The OAuth authorization page the login opens lives on a subdomain.
        assert!(is_linear_url("https://mcp.linear.app/authorize"));
        assert!(!is_linear_url("http://linear.app/"));
        assert!(!is_linear_url("http://linear.app/demo/issue/DEMO-1"));
        // User info and look-alike hosts: parsed, the host is evil.example and linear.app.evil.example.
        assert!(!is_linear_url("https://linear.app@evil.example/"));
        assert!(!is_linear_url("https://linear.app.evil.example/"));
        assert!(!is_linear_url("https://evil.example/https://linear.app/"));
        assert!(!is_linear_url("file:///etc/passwd"));
        assert!(!is_linear_url("not a link"));
    }

    #[test]
    fn a_failed_registration_check_is_unknown_never_not_installed() {
        assert_eq!(
            registration(ExecutorKind::Claude, Ok(true)),
            ControlServer::Registered
        );
        assert_eq!(
            registration(ExecutorKind::Claude, Ok(false)),
            ControlServer::NotRegistered
        );
        assert_eq!(
            registration(
                ExecutorKind::Codex,
                Err(anyhow::anyhow!(
                    "`codex mcp get issuerelay` timed out after 60s"
                ))
            ),
            ControlServer::Unknown
        );
        assert_eq!(
            serde_json::to_value(ControlServer::NotInstalled).unwrap(),
            "not_installed"
        );
        assert_eq!(
            serde_json::to_value(ControlServer::Unknown).unwrap(),
            "unknown"
        );
    }

    #[test]
    fn the_cli_sits_next_to_the_app() {
        let exe =
            std::path::Path::new("/Applications/IssueRelay.app/Contents/MacOS/issuerelay-app");
        let expected = if cfg!(windows) {
            "issuerelay.exe"
        } else {
            "issuerelay"
        };
        assert_eq!(cli_next_to(exe).file_name().unwrap(), expected);
        assert_eq!(cli_next_to(exe).parent(), exe.parent());
    }

    #[test]
    fn list_sizes_are_bounded() {
        assert_eq!(clamp_limit(0, 500), 1);
        assert_eq!(clamp_limit(10_000, 500), 500);
        assert_eq!(clamp_limit(40, 500), 40);
    }

    #[test]
    fn setup_actions_name_their_executor() {
        use issuerelay_core::{config::ExecutorKind::*, setup::SetupAction::*};
        assert_eq!(action_executor(AddLinearMcp(Codex)), Codex);
        assert_eq!(action_executor(LoginLinearMcp(Claude)), Claude);
        assert_eq!(action_executor(RegisterControlServer(Codex)), Codex);
        assert_eq!(action_executor(RemoveControlServer(Claude)), Claude);
    }

    #[test]
    fn the_one_click_login_names_the_server_setup_registers() {
        assert_eq!(
            linear_server_name(ExecutorKind::Claude).unwrap(),
            "linear-server"
        );
        assert_eq!(linear_server_name(ExecutorKind::Codex).unwrap(), "linear");
    }

    #[test]
    fn a_new_login_cancels_the_one_still_waiting_and_cancel_ends_the_current_one() {
        let pending = PendingLogin::default();
        let first = pending.begin();
        let second = pending.begin();
        assert!(first.is_cancelled(), "one login waits at a time");
        assert!(!second.is_cancelled());
        pending.cancel();
        assert!(second.is_cancelled());
        // Nothing waits any more: a second cancel does nothing.
        pending.cancel();
    }

    #[test]
    fn quit_modes_read_from_the_panel() {
        assert_eq!(
            serde_json::from_str::<QuitMode>("\"wait\"").unwrap(),
            QuitMode::Wait
        );
        assert_eq!(
            serde_json::from_str::<QuitMode>("\"stop\"").unwrap(),
            QuitMode::Stop
        );
        assert!(serde_json::from_str::<QuitMode>("\"later\"").is_err());
    }

    /// One row of app/src/lib/clean-cases.json, which the panel's `cleanDisplayText` test reads.
    #[derive(Deserialize)]
    struct CleanCase {
        input: String,
        expected: String,
        ansi: bool,
    }

    #[test]
    fn display_text_cases_follow_cores_character_rules() {
        let cases: Vec<CleanCase> =
            serde_json::from_str(include_str!("../../src/lib/clean-cases.json"))
                .expect("clean-cases.json is valid");
        // Escape sequences are the panel's own addition; every other case is core's character
        // rule, checked against `clean_text` itself (it trims what it returns, the panel does not).
        for case in cases.iter().filter(|case| !case.ansi) {
            assert_eq!(
                clean_text(&case.input),
                case.expected.trim(),
                "{:?}",
                case.input
            );
        }
        assert!(
            cases
                .iter()
                .filter(|case| case.ansi)
                .all(|case| case.input.contains(['\u{1b}', '\u{9b}'])),
            "only escape-sequence cases skip the core check"
        );
    }
}
