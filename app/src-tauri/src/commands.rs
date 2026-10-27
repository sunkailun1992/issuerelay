//! The commands the panel calls. Each waits for the host, does one thing and returns something
//! the panel can show; an error comes back as one cleaned line, or as a code the panel
//! translates (`not_a_linear_link`, `login_cancelled`, and the `update_*` codes of
//! `install_update`). `mark_done`, `retry_job` and `cancel_job` exist only here (design 5.6): IPC,
//! the CLI and MCP never offer them. Plugins (opener, dialog, autostart; the updater through
//! `update.rs`) are used from here, so the panel itself needs no plugin permission.
//! `Host::env()` holds the real proxy values (user name and password unmasked): neither it nor
//! any proxy value ever reaches the panel. The commands use it to start processes, and
//! `cli_setup` reads its PATH only to tell whether `~/.local/bin` is on it, a fact without
//! secrets (the environment check shows the PATH anyway); the environment the panel sees is
//! `Engine::doctor`'s, with the proxy values masked.
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use anyhow::Context as _;
use issuerelay_core::{
    cli_path::{CliPathStatus, CliSetup},
    config::{Binding, ExecutorKind, Settings},
    doctor::{CommandRunner as _, DoctorReport, SystemRunner},
    engine::{IssueView, StatusSnapshot},
    env::EnvSource,
    executor,
    host::{Host, LinearSetup},
    linear::{Project, Viewer},
    model::Job,
    setup::{self, SetupAction},
    text::{clean_text, strip_control},
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
    update::{self, AppUpdates, UpdateStatus},
    window,
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

/// The most characters of an error the panel is given.
const MAX_ERROR_CHARS: usize = 500;
/// How many of them a longer error keeps from its start; the rest come from its end.
const ERROR_HEAD_CHARS: usize = 200;

/// An error as the panel shows it: the whole chain on one line, cleaned of control and invisible
/// characters (`strip_control`), since core's errors can carry text from Linear, the executors or
/// the system. anyhow writes the root cause last, so a chain longer than `MAX_ERROR_CHARS` keeps
/// its start and its end, with `…` where the middle was.
fn error(e: impl Into<anyhow::Error>) -> String {
    let flat = strip_control(&format!("{:#}", e.into()));
    let line = flat.trim();
    let length = line.chars().count();
    if length <= MAX_ERROR_CHARS {
        return line.to_string();
    }
    let head: String = line.chars().take(ERROR_HEAD_CHARS).collect();
    let tail: String = line
        .chars()
        .skip(length - (MAX_ERROR_CHARS - ERROR_HEAD_CHARS))
        .collect();
    format!("{}…{}", head.trim_end(), tail.trim_start())
}

fn clamp_limit(limit: usize, max: usize) -> usize {
    limit.clamp(1, max)
}

/// The only links IssueRelay opens in the browser, from the panel (`open_linear`) and for the
/// Linear login (its authorization page), as `url::Url` parses them: `https` on `linear.app` or
/// one of its subdomains (such as `mcp.linear.app`), without a user name or password and without
/// an empty label (`linear.app.`, `.linear.app`). `None` for anything else, so user info
/// (`https://linear.app@evil.example/`) and look-alike hosts (`linear.app.evil.example`) are
/// refused. The browser is given this parsed URL, never the text as it came: the system's
/// opener parses links its own way (macOS hands them to NSURL, which reads
/// `https://linear.app\@evil.example/` as a link to evil.example where `url` reads linear.app),
/// and only the normalized form reads the same to both.
fn linear_url(text: &str) -> Option<url::Url> {
    let url = url::Url::parse(text).ok()?;
    let linear = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.host_str().is_some_and(|host| {
            !host.split('.').any(str::is_empty)
                && (host == "linear.app" || host.ends_with(".linear.app"))
        });
    linear.then_some(url)
}

/// Why `open_linear_link` opened nothing.
#[derive(Debug)]
enum NotOpened {
    /// Not a Linear link (`linear_url`); the refusal is logged.
    NotLinear,
    /// The system's opener failed.
    Failed(anyhow::Error),
}

/// Opens `text` with `open` when it is a Linear link, handing `open` the normalized URL the
/// check parsed (`linear_url`), never `text` itself; any other link is refused and logged.
/// `open` is the opener plugin in the app and a recorder in the tests.
fn open_linear_link<E: Into<anyhow::Error>>(
    text: &str,
    open: impl FnOnce(&str) -> Result<(), E>,
) -> Result<(), NotOpened> {
    let Some(url) = linear_url(text) else {
        tracing::warn!(url = %clean_text(text), "refused to open a link outside linear.app");
        return Err(NotOpened::NotLinear);
    };
    open(url.as_str()).map_err(|e| NotOpened::Failed(e.into()))
}

/// Whether `command` (`<executor> mcp get <name>`) found Linear's server under that name, from
/// its exit code: 0 when it did, another code when it did not. A command ended by a signal
/// (`None`) says nothing about the name: it is logged and an error (cleaned), so the panel shows
/// a failure it can retry instead of the manual command for a server under another name.
fn named(command: &str, code: Option<i32>) -> CmdResult<bool> {
    match code {
        Some(0) => Ok(true),
        Some(_) => Ok(false),
        None => {
            tracing::warn!(command = %clean_text(command), "could not check the name of Linear's server: the command was ended by a signal");
            Err(error(anyhow::anyhow!("`{command}` was ended by a signal")))
        }
    }
}

/// Waits for `login` unless `cancelled` fires first: the login's own answer (an error cleaned),
/// or the code `login_cancelled`. A cancelled login's future is dropped, which abandons it
/// (`Host::login`). A login that has finished wins over a cancel that comes at the same moment:
/// by then the host uses the new login, so its answer is what happened.
async fn cancellable<T>(
    login: impl Future<Output = anyhow::Result<T>>,
    cancelled: CancellationToken,
) -> CmdResult<T> {
    tokio::select! {
        biased;
        answer = login => answer.map_err(error),
        () = cancelled.cancelled() => Err(LOGIN_CANCELLED.into()),
    }
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

/// Linear's label names, for choosing a binding's routing labels.
#[tauri::command]
pub async fn labels(state: State<'_, AppState>) -> CmdResult<Vec<String>> {
    host(&state).await?.engine().labels().await.map_err(error)
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
    // The language may have changed: the tray menu follows at once. So may "Show in Dock": the
    // icon follows too (macOS; Windows has none).
    tray::refresh(&app, &host);
    window::refresh_dock(&app);
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
        // The same check as `open_linear`: the browser opens nothing but Linear, and only the
        // URL the check parsed. A refusal is logged there.
        if let Err(NotOpened::Failed(e)) =
            open_linear_link(url, |url| opener.opener().open_url(url, None::<&str>))
        {
            tracing::warn!(error = %format!("{e:#}"), "could not open the authorization page");
        }
    });
    let viewer = cancellable(login, cancelled).await?;
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
    let args = ["mcp", "get", name.as_str()];
    let output = runner
        .run_limited(&program, &args, MCP_GET_LIMIT)
        .await
        .map_err(error)?;
    named(
        &format!("{} {}", program.display(), args.join(" ")),
        output.code,
    )
}

/// Opens a Linear link in the system browser, as the normalized URL the check parsed; any other
/// link is refused (and logged) with the code `not_a_linear_link`.
#[tauri::command]
pub async fn open_linear(app: AppHandle, url: String) -> CmdResult<()> {
    open_linear_link(&url, |url| app.opener().open_url(url, None::<&str>)).map_err(|failure| {
        match failure {
            NotOpened::NotLinear => NOT_A_LINEAR_LINK.into(),
            NotOpened::Failed(e) => error(e),
        }
    })
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
/// The quit state machine (`quit::next`) decides what follows; what it returns only matters to
/// the window hook.
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

/// What the PATH switch needs: the CLI next to the app, where the app runs from, and the login
/// shell's PATH the host resolved (only a PATH read from the login shell tells whether
/// `~/.local/bin` is on it). The PATH is only read here: the commands answer with what follows
/// from it (on PATH or not, and the fixed `SHELL_LINE`), never with the PATH itself.
///
/// That PATH is the host's merged one: the login shell's PATH followed by the entries of the
/// host's own PATH that it lacks (`env::combine`). A host started from a terminal whose PATH has
/// `~/.local/bin` can therefore be told "already on PATH" while a new login shell lacks it. A
/// host started by Launch Services (Finder, the Dock, the login item, `open`, and so
/// `issuerelay start`) has only launchd's minimal PATH of its own, so those starts are not
/// affected.
async fn cli_setup(state: &State<'_, AppState>) -> CmdResult<CliSetup> {
    let env = host(state).await?.env();
    let shell_path = (env.path_source == EnvSource::LoginShell)
        .then_some(env.path)
        .flatten();
    CliSetup::for_running_app(cli_path(), shell_path).map_err(error)
}

/// Runs one of core's `cli_path` calls on a blocking thread. On Windows the registry write and the
/// `WM_SETTINGCHANGE` broadcast after it can take seconds (`SendMessageTimeoutW` waits up to five
/// of them for each window that does not answer), and a command must not hold an async worker for
/// that long. An error comes back as the cleaned line of its chain like every command's; a call
/// that panicked is an error too, not a crash of the command.
async fn cli_path_job<T: Send + 'static>(
    setup: CliSetup,
    job: impl FnOnce(&CliSetup) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    tauri::async_runtime::spawn_blocking(move || job(&setup))
        .await
        .map_err(error)?
        .map_err(error)
}

/// Whether the command line is on PATH through IssueRelay (design 12).
#[tauri::command]
pub async fn cli_on_path(state: State<'_, AppState>) -> CmdResult<CliPathStatus> {
    cli_path_job(cli_setup(&state).await?, issuerelay_core::cli_path::status).await
}

/// Puts the command line on PATH without administrator rights: `~/.local/bin` on macOS, the
/// user's own `Path` on Windows. The panel offers it again while a file that is not IssueRelay's is
/// in the way: each call looks at the slot afresh and answers `blocked` while the file is there.
#[tauri::command]
pub async fn install_cli_on_path(state: State<'_, AppState>) -> CmdResult<CliPathStatus> {
    cli_path_job(cli_setup(&state).await?, issuerelay_core::cli_path::install).await
}

/// Takes the command line off PATH; only what IssueRelay added is removed.
#[tauri::command]
pub async fn remove_cli_from_path(state: State<'_, AppState>) -> CmdResult<CliPathStatus> {
    cli_path_job(cli_setup(&state).await?, issuerelay_core::cli_path::remove).await
}

/// The last update check's outcome, for a panel opened after it; answers at once.
#[tauri::command]
pub fn update_status(updates: State<'_, AppUpdates>) -> UpdateStatus {
    updates.status()
}

/// Checks for an update now (the settings page's button); never installs anything.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> UpdateStatus {
    update::check(&app).await
}

/// "Install and restart": downloads and verifies the offered update, then hands over to the quit
/// flow. A failure is answered as a code (`update_*`) the panel translates and changes nothing.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> CmdResult<()> {
    update::install(&app).await.map_err(|failure| {
        tracing::warn!(code = ?failure.code, detail = %failure.detail, "the update was not installed");
        failure.code.error_code().to_string()
    })
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use serde::Deserialize;

    use super::*;

    /// What `open_linear_link` hands the system's opener for `text`; `None` when it refuses.
    /// Whatever it opens must read the same to any parser: the canonical form of an `https` URL
    /// on linear.app or one of its subdomains, without user info, and without anything another
    /// parser could split differently (a backslash, whitespace, a control character, an `@` in
    /// the authority).
    fn opened(text: &str) -> Option<String> {
        let mut handed = None;
        let outcome = open_linear_link(text, |url| {
            handed = Some(url.to_string());
            Ok::<(), anyhow::Error>(())
        });
        match outcome {
            Ok(()) => {}
            Err(NotOpened::NotLinear) => {
                assert_eq!(handed, None, "{text:?}: a refused link opens nothing");
                return None;
            }
            Err(NotOpened::Failed(e)) => panic!("{text:?}: {e:#}"),
        }
        let handed = handed.expect("an accepted link is opened");
        let url = url::Url::parse(&handed).expect("what is opened parses");
        assert_eq!(
            url.as_str(),
            handed,
            "{text:?}: opened in its canonical form"
        );
        assert_eq!(url.scheme(), "https", "{text:?}");
        let host = url.host_str().expect("a host");
        assert!(
            host == "linear.app" || host.ends_with(".linear.app"),
            "{text:?}: {host}"
        );
        assert!(
            url.username().is_empty() && url.password().is_none(),
            "{text:?}"
        );
        assert!(
            !handed.contains(|c: char| c == '\\' || c.is_whitespace() || c.is_control()),
            "{text:?}: {handed:?}"
        );
        let authority = handed["https://".len()..]
            .split(['/', '?', '#'])
            .next()
            .expect("an authority");
        assert!(!authority.contains('@'), "{text:?}: {handed:?}");
        Some(handed)
    }

    #[test]
    fn only_linear_links_are_opened_and_only_as_parsed() {
        // Linear's links open as they are; the OAuth authorization page lives on a subdomain.
        for link in [
            "https://linear.app/demo/issue/DEMO-1",
            "https://mcp.linear.app/authorize",
            "https://mcp.linear.app/authorize?client_id=demo&redirect_uri=http%3A%2F%2F127.0.0.1%3A1234%2Fcallback",
        ] {
            assert_eq!(opened(link).as_deref(), Some(link));
        }
        // Refused: another scheme or host, any user info, look-alike hosts (one with a Cyrillic
        // "a", as typed and as the punycode `url` turns it into), a trailing dot or an empty
        // label, and text that is no link at all.
        for refused in [
            "https://line\u{0430}r.app/demo/issue/DEMO-1",
            "https://xn--liner-7ve.app/demo/issue/DEMO-1",
            "https://mcp.line\u{0430}r.app/authorize",
            "http://linear.app/",
            "http://linear.app/demo/issue/DEMO-1",
            "https://linear.app@evil.example/",
            "https://linear.app%5C@evil.example/",
            "https://evil.example%5C@linear.app/",
            "https://evil.example\\@linear.app/",
            "https://user:secret@linear.app/",
            "https://linear.app.evil.example/",
            "https://evillinear.app/",
            "https://linear.app\t.evil.example/",
            "https://linear.app .evil.example/",
            "https://linear.app./",
            "https://mcp.linear.app./authorize",
            "https://.linear.app/",
            "https://evil.example/https://linear.app/",
            "file:///etc/passwd",
            "not a link",
        ] {
            assert_eq!(opened(refused), None, "{refused:?}");
        }
        // Accepted, but opened only as `url` parsed them, never as given: macOS's opener hands
        // the text to NSURL, which reads the first one as a link to evil.example.
        for (text, normalized) in [
            (
                "https://linear.app\\@evil.example/",
                "https://linear.app/@evil.example/",
            ),
            ("HTTPS://LINEAR.APP/demo", "https://linear.app/demo"),
            (
                "HTTPS://MCP.Linear.App/authorize",
                "https://mcp.linear.app/authorize",
            ),
            ("https://lin\near.app/demo", "https://linear.app/demo"),
            ("https://linear.app\r\n/demo", "https://linear.app/demo"),
            (" https://linear.app/demo ", "https://linear.app/demo"),
            (
                "https://linear.app#@evil.example/",
                "https://linear.app/#@evil.example/",
            ),
            (
                "https://linear.app/demo issue",
                "https://linear.app/demo%20issue",
            ),
        ] {
            assert_eq!(opened(text).as_deref(), Some(normalized), "{text:?}");
        }
    }

    #[test]
    fn a_long_error_keeps_its_start_and_its_root_cause() {
        // anyhow writes the chain outermost first: the root cause is at the end of the line.
        let chain = anyhow::anyhow!("root cause\u{1b}[2J")
            .context("x".repeat(2000))
            .context("outer\n");
        let line = error(chain);
        assert!(line.starts_with("outer : xxx"), "{line}");
        assert!(line.ends_with("xxx: root cause [2J"), "{line}");
        assert_eq!(line.matches('…').count(), 1, "{line}");
        assert!(line.chars().count() <= MAX_ERROR_CHARS + 1, "{line}");
        // A short one is the whole chain on one line: control characters become spaces,
        // invisible ones go, and the ends are trimmed.
        assert_eq!(
            error(anyhow::anyhow!("cannot read\tstate.db\u{200b} ").context(" store")),
            "store: cannot read state.db"
        );
    }

    #[test]
    fn a_link_that_cannot_be_opened_says_why() {
        let outcome = open_linear_link("https://linear.app/", |_| {
            Err(anyhow::anyhow!("no browser"))
        });
        assert!(
            matches!(&outcome, Err(NotOpened::Failed(e)) if format!("{e:#}") == "no browser"),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_killed_mcp_get_is_an_error_never_another_name() {
        let command = "claude mcp get linear-server";
        assert_eq!(named(command, Some(0)), Ok(true));
        assert_eq!(named(command, Some(1)), Ok(false));
        // Ended by a signal it says nothing about the name: an error the panel offers to retry,
        // not "registered under another name" with the manual command. The error is cleaned.
        assert_eq!(
            named("claude\x1b[2J mcp get linear-server", None),
            Err("`claude [2J mcp get linear-server` was ended by a signal".to_string())
        );
    }

    /// How long a login test waits for an answer that should come at once.
    const LOGIN_LIMIT: Duration = Duration::from_secs(5);

    /// A login that waits for ever (for the browser), and a flag that its drop sets.
    fn endless_login() -> (
        impl Future<Output = anyhow::Result<Viewer>> + Send,
        Arc<AtomicBool>,
    ) {
        struct Dropped(Arc<AtomicBool>);
        impl Drop for Dropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let dropped = Arc::new(AtomicBool::new(false));
        let guard = Dropped(dropped.clone());
        let login = async move {
            let _guard = guard;
            std::future::pending::<anyhow::Result<Viewer>>().await
        };
        (login, dropped)
    }

    #[tokio::test]
    async fn a_cancelled_login_stops_waiting_and_is_abandoned() {
        let pending = PendingLogin::default();
        let (login, dropped) = endless_login();
        let waiting = tokio::spawn(cancellable(login, pending.begin()));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "the login waits for the browser");
        // The panel's Cancel (`linear_cancel_login`).
        pending.cancel();
        let answer = tokio::time::timeout(LOGIN_LIMIT, waiting)
            .await
            .expect("a cancelled login answers at once")
            .unwrap();
        assert_eq!(answer, Err(LOGIN_CANCELLED.to_string()));
        assert!(
            dropped.load(Ordering::SeqCst),
            "the login's future is dropped, which abandons it"
        );
    }

    #[tokio::test]
    async fn a_finished_login_wins() {
        let viewer = Viewer {
            id: "user-1".into(),
            name: "Jane Doe".into(),
        };
        let finished = || std::future::ready(anyhow::Ok(viewer.clone()));
        let answer = cancellable(finished(), CancellationToken::new());
        assert_eq!(
            tokio::time::timeout(LOGIN_LIMIT, answer).await.unwrap(),
            Ok(viewer.clone())
        );
        // Finished when the cancel comes: the host uses the new login already, so its answer is
        // what happened. Repeated, because a race that picked either would fail about half the
        // time.
        for _ in 0..32 {
            let cancel = CancellationToken::new();
            cancel.cancel();
            let answer = cancellable(finished(), cancel);
            assert_eq!(
                tokio::time::timeout(LOGIN_LIMIT, answer).await.unwrap(),
                Ok(viewer.clone())
            );
        }
        // A login that failed gives its error, cleaned.
        let failed = cancellable(
            std::future::ready(Err::<Viewer, _>(anyhow::anyhow!(
                "Linear refused\nthe login"
            ))),
            CancellationToken::new(),
        );
        assert_eq!(
            tokio::time::timeout(LOGIN_LIMIT, failed).await.unwrap(),
            Err("Linear refused the login".to_string())
        );
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

    /// A `CliSetup` for the tests of `cli_path_job`: it is never read, only handed to the call.
    fn demo_setup() -> CliSetup {
        CliSetup {
            cli: None,
            location: issuerelay_core::cli_path::AppLocation::Development,
            home: PathBuf::from("/tmp/demo-home"),
            shell_path: None,
        }
    }

    #[tokio::test]
    async fn the_cli_path_calls_run_off_the_async_workers_and_fail_like_the_other_commands() {
        // The Windows registry write and the broadcast after it can take seconds: the call runs
        // on a blocking thread, not on the thread that polls the command.
        let here = std::thread::current().id();
        let elsewhere = cli_path_job(demo_setup(), move |_| {
            Ok(std::thread::current().id() != here)
        })
        .await;
        assert_eq!(elsewhere, Ok(true));
        // It is given the setup it was asked about.
        let home = cli_path_job(demo_setup(), |setup| Ok(setup.home.clone())).await;
        assert_eq!(home, Ok(PathBuf::from("/tmp/demo-home")));
        // An error comes back as the cleaned line of its whole chain, as every command's does.
        let refused = cli_path_job(demo_setup(), |_| -> anyhow::Result<()> {
            Err(anyhow::anyhow!("cannot create the link\u{1b}[2J").context("outer"))
        })
        .await;
        assert_eq!(
            refused,
            Err("outer: cannot create the link [2J".to_string())
        );
        // A call that panicked is an error too: it does not take the command down with it.
        let panicked = cli_path_job(demo_setup(), |_| -> anyhow::Result<()> {
            panic!("demo panic")
        })
        .await;
        assert!(panicked.is_err(), "{panicked:?}");
    }

    #[test]
    fn the_shell_line_the_panel_shows_is_cores() {
        // The mock backend shows the line from the same file (`DEMO_SHELL_LINE`).
        let shared: serde_json::Value =
            serde_json::from_str(include_str!("../../src/lib/shell-line.json"))
                .expect("shell-line.json is valid");
        assert_eq!(shared["shell_line"], issuerelay_core::cli_path::SHELL_LINE);
    }

    #[test]
    fn the_panels_default_trigger_names_are_cores() {
        // The panel shows these names until its settings have been read, and the mock's default
        // settings use them; app/src/lib/trigger-defaults.json is the only copy. Compared as JSON
        // so that a missing or extra name fails too (`TriggerNames` fills missing ones in).
        let panel: serde_json::Value =
            serde_json::from_str(include_str!("../../src/lib/trigger-defaults.json"))
                .expect("trigger-defaults.json is valid");
        let core = serde_json::to_value(issuerelay_core::config::TriggerNames::default())
            .expect("the trigger names serialize");
        assert_eq!(panel, core);
    }

    /// One row of app/src/lib/model-name-cases.json, which the panel's `isValidModelName` test
    /// (app/src/lib/model-name.test.ts) reads as well; a row's `note` is for the reader only.
    #[derive(Deserialize)]
    struct ModelNameCase {
        name: String,
        valid: bool,
    }

    #[test]
    fn model_name_cases_follow_core() {
        let cases: Vec<ModelNameCase> =
            serde_json::from_str(include_str!("../../src/lib/model-name-cases.json"))
                .expect("model-name-cases.json is valid");
        // The panel rejects a model name before it is saved, the engine rejects it again when it
        // is: the two must never disagree about a name.
        assert!(!cases.is_empty());
        for case in cases {
            assert_eq!(
                issuerelay_core::config::is_valid_model_name(&case.name),
                case.valid,
                "{:?}",
                case.name
            );
        }
    }

    /// One row of app/src/lib/mask-cases.json, which the panel's `maskCredentials` test reads.
    #[derive(Deserialize)]
    struct MaskCase {
        value: String,
        masked: String,
    }

    #[test]
    fn mask_cases_follow_core() {
        let cases: Vec<MaskCase> =
            serde_json::from_str(include_str!("../../src/lib/mask-cases.json"))
                .expect("mask-cases.json is valid");
        // The engine masks a proxy value before the environment check carries it to the panel, and
        // the panel masks what it shows once more: the two must never disagree about a value.
        assert!(!cases.is_empty());
        for case in cases {
            assert_eq!(
                issuerelay_core::env::mask_credentials(&case.value),
                case.masked,
                "{:?}",
                case.value
            );
        }
    }

    /// app/src/lib/routing-cases.json, which the panel's routing test (app/src/lib/routing.test.ts)
    /// reads as well; a row's `note` is for the reader only.
    #[derive(Deserialize)]
    struct RoutingCases {
        overlap: Vec<OverlapCase>,
        ids: Vec<IdCase>,
    }

    #[derive(Deserialize)]
    struct OverlapCase {
        a: Vec<String>,
        b: Vec<String>,
        overlap: bool,
    }

    #[derive(Deserialize)]
    struct IdCase {
        project: String,
        labels: Vec<String>,
        id: String,
    }

    #[test]
    fn routing_cases_follow_core() {
        let cases: RoutingCases =
            serde_json::from_str(include_str!("../../src/lib/routing-cases.json"))
                .expect("routing-cases.json is valid");
        // The binding form refuses a binding that would take another one's issues, and names a new
        // one; the engine replaces the bindings a saved one overlaps: the two must agree.
        assert!(!cases.overlap.is_empty() && !cases.ids.is_empty());
        let binding = |labels: &[String]| -> issuerelay_core::config::Binding {
            serde_json::from_value(serde_json::json!({
                "id": "proj-1",
                "project_id": "proj-1",
                "project_name": "Demo Project",
                "directory": "/demo",
                "labels": labels,
            }))
            .expect("a binding")
        };
        for case in cases.overlap {
            let (a, b) = (binding(&case.a), binding(&case.b));
            assert_eq!(
                issuerelay_core::config::bindings_overlap(&a, &b),
                case.overlap,
                "{:?} {:?}",
                case.a,
                case.b
            );
            assert_eq!(
                issuerelay_core::config::bindings_overlap(&b, &a),
                case.overlap
            );
        }
        for case in cases.ids {
            assert_eq!(
                issuerelay_core::config::binding_id_for(&case.project, &case.labels),
                case.id
            );
        }
    }
}
