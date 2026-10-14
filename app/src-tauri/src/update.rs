//! Automatic updates (design 12). The updater plugin reads
//! `https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json` a minute
//! after the start and every six hours while `settings.auto_update` is on, and whenever the
//! settings page asks. Only a newer version is offered, and only a payload that verifies against
//! the public key in tauri.conf.json (and names the announced version, `requireSignedVersion`)
//! is ever kept. Installing waits for the user's "install and restart", which goes through the
//! quit flow (`quit::request_update`): with runs in progress the panel asks the same "wait for
//! completion / stop now" as quitting, and once the host has stopped the quit flow installs
//! (`QuitState::Installing`) and then restarts. A failed check, a missing platform, a bad
//! signature or a version that is not newer is shown and logged and changes nothing else:
//! monitoring goes on. An install that fails after the host stopped restarts the app as it is and
//! is shown by the panel after the restart (`record_install_failure`), never as a notification.
//! Everything that takes the app is generic over the runtime, like quit.rs.
use std::{
    future::Future,
    path::Path,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};

use chrono::{DateTime, Utc};
use issuerelay_core::{
    cli_path::{AppLocation, running_app_location},
    host::Host,
    paths::AppPaths,
    text::clean_text,
};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_updater::UpdaterExt;

use crate::{
    events, logging, notify,
    quit::{self, QuitEvent},
    state::AppState,
};

/// The first background check after the start.
pub const FIRST_CHECK: Duration = Duration::from_secs(60);
/// The background checks after the first.
pub const CHECK_EVERY: Duration = Duration::from_secs(6 * 60 * 60);
/// How long one request for the manifest may take. The plugin sets no limit of its own, and a
/// check that never answers (a proxy that accepts and then says nothing) would hold the
/// background checks up for good.
const CHECK_LIMIT: Duration = Duration::from_secs(30);
/// How long the download of a payload may take, from the request to its last byte. Generous for a
/// slow line, but a download that stalls ends and can be started again.
const DOWNLOAD_LIMIT: Duration = Duration::from_secs(30 * 60);
/// What a failed install leaves in the data folder for the next start: the process restarts as it
/// is right after the failure, and the panel then shows the cleaned reason once.
pub const INSTALL_FAILURE_FILE: &str = "update-install-failure.txt";

/// Why a check, a download or an install failed, as a code the panel translates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureCode {
    /// No valid release manifest could be fetched: offline, a proxy, or nothing published yet.
    Unreachable,
    /// The manifest has no entry for this platform.
    NoPlatform,
    /// The payload's signature does not verify against the public key, or names another version.
    BadSignature,
    /// IssueRelay cannot replace itself from where it runs: a development build, the disk image
    /// or a translocated copy.
    Unavailable,
    /// The verified payload could not be installed.
    InstallFailed,
    Other,
}

impl FailureCode {
    /// Every code, in the order of app/src/lib/update-failure-cases.json, which the panel's tests
    /// read too.
    pub const ALL: [FailureCode; 6] = [
        Self::Unreachable,
        Self::NoPlatform,
        Self::BadSignature,
        Self::Unavailable,
        Self::InstallFailed,
        Self::Other,
    ];

    /// The error code `install_update` answers with; the panel translates it (`errors.*`).
    pub fn error_code(self) -> &'static str {
        match self {
            Self::Unreachable => "update_unreachable",
            Self::NoPlatform => "update_no_platform",
            Self::BadSignature => "update_bad_signature",
            Self::Unavailable => "update_unavailable",
            Self::InstallFailed => "update_install_failed",
            Self::Other => "update_failed",
        }
    }
}

/// A failure: its code and the cleaned error line the panel shows under the translated headline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Failure {
    pub code: FailureCode,
    pub detail: String,
}

impl Failure {
    pub fn new(code: FailureCode, detail: impl std::fmt::Display) -> Self {
        Self {
            code,
            detail: clean_text(&detail.to_string()),
        }
    }
}

/// What the panel shows about updates: `update_status`, `check_update` and `update-status`.
/// The release's own text (`notes` in latest.json) is written for the download page, not for an
/// installed copy, and is not carried.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Not checked since the start.
    Unchecked,
    /// The last check found nothing newer than the running version.
    UpToDate { checked_at: DateTime<Utc> },
    /// A newer version is offered.
    Available { version: String },
    /// The check failed, or (after a restart) the previous install did; nothing was installed.
    Failed(Failure),
}

/// Where updates come from: the updater plugin in the app, a fake in the tests.
pub trait UpdateSource: Send + Sync + 'static {
    /// An update a check found; kept until it is downloaded and installed.
    type Offer: Send + Sync + 'static;
    fn version(offer: &Self::Offer) -> &str;
    /// The newest release when it is newer than the running version.
    fn check(&self) -> impl Future<Output = Result<Option<Self::Offer>, Failure>> + Send;
    /// The offer's payload, verified against the public key: nothing unverified comes back.
    fn download(
        &self,
        offer: &Self::Offer,
    ) -> impl Future<Output = Result<Vec<u8>, Failure>> + Send;
}

/// A downloaded and verified update waiting for the quit flow.
pub struct Pending<O> {
    pub offer: Arc<O>,
    pub bytes: Vec<u8>,
}

/// The update state the app keeps: the last check, the offer it found, the verified download
/// waiting for the quit flow, and the version that already notified.
pub struct Updates<S: UpdateSource> {
    source: S,
    current: String,
    status: Mutex<UpdateStatus>,
    offer: Mutex<Option<Arc<S::Offer>>>,
    pending: Mutex<Option<Pending<S::Offer>>>,
    notified: Mutex<Option<String>>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Whether `offered` is a newer semantic version than `current`; a leading `v` is allowed and
/// anything unparsable is not newer.
pub fn is_newer(current: &str, offered: &str) -> bool {
    match (
        semver::Version::parse(current),
        semver::Version::parse(offered.trim_start_matches('v')),
    ) {
        (Ok(current), Ok(offered)) => offered > current,
        _ => false,
    }
}

impl<S: UpdateSource> Updates<S> {
    pub fn new(source: S, current: &str) -> Self {
        Self {
            source,
            current: current.to_string(),
            status: Mutex::new(UpdateStatus::Unchecked),
            offer: Mutex::new(None),
            pending: Mutex::new(None),
            notified: Mutex::new(None),
        }
    }

    /// What the panel shows now.
    pub fn status(&self) -> UpdateStatus {
        lock(&self.status).clone()
    }

    /// Starts with a failure on show: the previous run's failed install (`take_install_failure`).
    pub fn show_failure(&self, failure: Failure) {
        *lock(&self.status) = UpdateStatus::Failed(failure);
    }

    /// Asks the source and returns this check's outcome. Only a newer version becomes an offer.
    /// A failed check is logged and leaves a version already offered on show.
    pub async fn check(&self) -> UpdateStatus {
        let outcome = match self.source.check().await {
            Ok(Some(offer)) if is_newer(&self.current, S::version(&offer)) => {
                tracing::info!(version = S::version(&offer), "a newer version is available");
                let status = UpdateStatus::Available {
                    version: S::version(&offer).to_string(),
                };
                *lock(&self.offer) = Some(Arc::new(offer));
                status
            }
            // The plugin already drops versions that are not newer; a manifest that slips one
            // through is treated the same.
            Ok(_) => {
                tracing::debug!("no newer version is offered");
                *lock(&self.offer) = None;
                UpdateStatus::UpToDate {
                    checked_at: Utc::now(),
                }
            }
            Err(failure) => {
                tracing::warn!(code = ?failure.code, detail = %failure.detail, "the update check failed");
                let outcome = UpdateStatus::Failed(failure);
                let mut status = lock(&self.status);
                if !matches!(*status, UpdateStatus::Available { .. }) {
                    *status = outcome.clone();
                }
                return outcome;
            }
        };
        *lock(&self.status) = outcome.clone();
        outcome
    }

    /// Downloads and verifies the offered version for "install and restart" and returns it.
    /// Nothing is kept unless the payload verified; a download already waiting is reused.
    pub async fn prepare(&self) -> Result<String, Failure> {
        let offer = lock(&self.offer).clone().ok_or_else(|| {
            Failure::new(
                FailureCode::Other,
                "no newer version is offered; check for updates first",
            )
        })?;
        let version = S::version(&offer).to_string();
        if lock(&self.pending)
            .as_ref()
            .is_some_and(|pending| S::version(&pending.offer) == version)
        {
            return Ok(version);
        }
        let bytes = self.source.download(&offer).await?;
        tracing::info!(%version, bytes = bytes.len(), "the update was downloaded and verified");
        *lock(&self.pending) = Some(Pending { offer, bytes });
        Ok(version)
    }

    pub fn pending_version(&self) -> Option<String> {
        lock(&self.pending)
            .as_ref()
            .map(|pending| S::version(&pending.offer).to_string())
    }

    /// The verified download, once: the quit flow installs it.
    pub fn take_pending(&self) -> Option<Pending<S::Offer>> {
        lock(&self.pending).take()
    }

    /// Drops the verified download: the user quit instead of installing.
    pub fn clear_pending(&self) {
        lock(&self.pending).take();
    }

    /// Whether a background check that found `version` shows a notification: once per version.
    pub fn should_notify(&self, version: &str) -> bool {
        let mut notified = lock(&self.notified);
        if notified.as_deref() == Some(version) {
            return false;
        }
        *notified = Some(version.to_string());
        true
    }
}

/// The updater plugin as the source of updates.
pub struct PluginSource<R: Runtime>(pub AppHandle<R>);

impl<R: Runtime> UpdateSource for PluginSource<R> {
    type Offer = tauri_plugin_updater::Update;

    fn version(offer: &Self::Offer) -> &str {
        &offer.version
    }

    async fn check(&self) -> Result<Option<Self::Offer>, Failure> {
        let updater = self
            .0
            .updater_builder()
            .timeout(CHECK_LIMIT)
            .build()
            .map_err(plugin_failure)?;
        updater.check().await.map_err(plugin_failure)
    }

    async fn download(&self, offer: &Self::Offer) -> Result<Vec<u8>, Failure> {
        // A copy with a limit of its own: the offer a check returns carries none.
        let mut offer = offer.clone();
        offer.timeout = Some(DOWNLOAD_LIMIT);
        // `download` verifies the minisign signature (and the version it names) before it
        // returns; a payload that does not verify is an error and nothing is kept.
        offer
            .download(|_, _| {}, || {})
            .await
            .map_err(plugin_failure)
    }
}

fn plugin_failure(error: tauri_plugin_updater::Error) -> Failure {
    Failure::new(failure_code(&error), &error)
}

/// The plugin's errors as codes (variants of `tauri_plugin_updater::Error` in 2.13.3, which is
/// `#[non_exhaustive]`: any other, an unsupported system or a bad header say, is `Other`).
pub fn failure_code(error: &tauri_plugin_updater::Error) -> FailureCode {
    use tauri_plugin_updater::Error as E;
    match error {
        E::ReleaseNotFound | E::Reqwest(_) | E::Serialization(_) | E::Network(_) => {
            FailureCode::Unreachable
        }
        E::TargetNotFound(_) | E::TargetsNotFound(_) => FailureCode::NoPlatform,
        E::Minisign(_)
        | E::Base64(_)
        | E::SignatureUtf8(_)
        | E::SignedVersionMismatch { .. }
        | E::MissingSignedVersion => FailureCode::BadSignature,
        _ => FailureCode::Other,
    }
}

/// The update state the app manages; `tauri::Wry` in the app, tauri's mock runtime in tests.
pub type AppUpdates<R = tauri::Wry> = Updates<PluginSource<R>>;

/// Whether a verified update waits for the quit flow.
pub fn pending<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<AppUpdates<R>>()
        .is_some_and(|updates| updates.pending_version().is_some())
}

pub fn take_pending<R: Runtime>(
    app: &AppHandle<R>,
) -> Option<Pending<tauri_plugin_updater::Update>> {
    app.try_state::<AppUpdates<R>>()
        .and_then(|updates| updates.take_pending())
}

pub fn clear_pending<R: Runtime>(app: &AppHandle<R>) {
    if let Some(updates) = app.try_state::<AppUpdates<R>>() {
        updates.clear_pending();
    }
}

/// Checks now, records the outcome and tells an open panel (`update-status` carries what the
/// panel should keep showing).
pub async fn check<R: Runtime>(app: &AppHandle<R>) -> UpdateStatus {
    let updates = app.state::<AppUpdates<R>>();
    let outcome = updates.check().await;
    logging::debug_failed(
        app.emit(events::UPDATE_STATUS, updates.status()),
        "telling the panel about the update check",
    );
    outcome
}

/// "Install and restart": refuses where the app cannot replace itself, downloads and verifies the
/// offered version, then hands over to the quit flow, which asks about runs in progress exactly
/// as quitting does and installs once the host has stopped.
pub async fn install<R: Runtime>(app: &AppHandle<R>) -> Result<(), Failure> {
    match running_app_location() {
        AppLocation::Installed => {}
        AppLocation::NotInstalled => {
            return Err(Failure::new(
                FailureCode::Unavailable,
                "IssueRelay runs from the disk image or a temporary copy",
            ));
        }
        AppLocation::Development => {
            return Err(Failure::new(
                FailureCode::Unavailable,
                "a development build does not install updates",
            ));
        }
    }
    app.state::<AppUpdates<R>>().prepare().await?;
    quit::request_update(app);
    Ok(())
}

/// Installs a verified download once the quit flow has stopped the host (`QuitState::Installing`)
/// and then lets it restart (`QuitEvent::Installed`). macOS swaps the app bundle in place and the
/// restart opens the new one; on Windows `install` starts the NSIS installer (`/P /UPDATE /R`),
/// which ends this process and starts the new version itself. If installing fails the app
/// restarts as it is, so monitoring resumes, and the panel says why after the restart.
pub fn install_and_restart<R: Runtime>(
    app: &AppHandle<R>,
    pending: Pending<tauri_plugin_updater::Update>,
) {
    let app = app.clone();
    // Off the main thread: `install` may wait for the main thread (macOS asks for administrator
    // rights there), and the restart that follows then delivers the exit event first, so the
    // single-instance lock is released before the new instance starts.
    tauri::async_runtime::spawn_blocking(move || {
        let version = pending.offer.version.clone();
        tracing::info!(%version, "installing the update");
        if let Err(detail) = install_outcome(|| pending.offer.install(&pending.bytes)) {
            tracing::error!(error = %detail, %version, "installing the update failed; restarting the current version");
            match AppPaths::system() {
                Ok(paths) => record_install_failure(&paths.data_dir, &detail),
                Err(error) => {
                    tracing::warn!(error = %format!("{error:#}"), "cannot keep the install failure for the panel")
                }
            }
        }
        quit::handle(&app, QuitEvent::Installed);
    });
}

/// Runs `install` and says why it did not install, if it did not: its error, or a panic. An
/// installer that unwinds must not leave quitting in `Installing`, where exit requests are held
/// and nothing would ever restart the app.
fn install_outcome<E: std::fmt::Display>(
    install: impl FnOnce() -> Result<(), E>,
) -> Result<(), String> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(install)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => Err(error.to_string()),
        Err(_) => Err("the installer stopped unexpectedly".to_string()),
    }
}

/// Keeps a failed install's cleaned error line for the next start, whose panel shows it once
/// (`take_install_failure`); a failure to write it is only logged.
pub fn record_install_failure(data_dir: &Path, detail: &str) {
    let file = data_dir.join(INSTALL_FAILURE_FILE);
    let written =
        std::fs::create_dir_all(data_dir).and_then(|()| std::fs::write(&file, clean_text(detail)));
    if let Err(e) = written {
        tracing::warn!(error = %e, "cannot keep the install failure for the panel");
    }
}

/// The failed install the previous run left, once: the file is removed as it is read.
pub fn take_install_failure(data_dir: &Path) -> Option<Failure> {
    let file = data_dir.join(INSTALL_FAILURE_FILE);
    let detail = std::fs::read_to_string(&file).ok()?;
    if let Err(e) = std::fs::remove_file(&file) {
        tracing::warn!(error = %e, "cannot remove the install failure note; the panel may show it again");
    }
    Some(Failure::new(FailureCode::InstallFailed, detail))
}

/// The host, when the background check may run: started, and automatic checks on.
async fn auto_check_host<R: Runtime>(app: &AppHandle<R>) -> Option<Host> {
    let host = app.state::<AppState>().host().await.ok()?;
    host.engine().settings().auto_update.then_some(host)
}

/// The background check: a minute after the start, then every six hours, only while
/// `settings.auto_update` is on, which is also the only switch of the "new version" notice
/// (design 9.5's four switchable notifications are about runs). A newer version notifies once and
/// shows the banner. Development builds check only when asked.
pub fn spawn_checker<R: Runtime>(app: AppHandle<R>) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if let Some(host) = auto_check_host(&app).await
                && let UpdateStatus::Available { version } = check(&app).await
                && app.state::<AppUpdates<R>>().should_notify(&version)
            {
                notify::show_update_available(&app, &host, &version);
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use tauri::test::{MockRuntime, mock_builder, mock_context, noop_assets};

    use super::*;

    /// What the fake source offers.
    #[derive(Debug)]
    struct FakeOffer(String);

    /// Answers checks and downloads from a script and counts the downloads.
    struct FakeSource {
        check: Mutex<Result<Option<String>, Failure>>,
        download: Mutex<Result<Vec<u8>, Failure>>,
        downloads: Mutex<usize>,
    }

    impl FakeSource {
        fn new(check: Result<Option<&str>, Failure>, download: Result<&[u8], Failure>) -> Self {
            Self {
                check: Mutex::new(check.map(|offered| offered.map(str::to_string))),
                download: Mutex::new(download.map(<[u8]>::to_vec)),
                downloads: Mutex::new(0),
            }
        }
    }

    impl UpdateSource for FakeSource {
        type Offer = FakeOffer;

        fn version(offer: &FakeOffer) -> &str {
            &offer.0
        }

        async fn check(&self) -> Result<Option<FakeOffer>, Failure> {
            self.check
                .lock()
                .unwrap()
                .clone()
                .map(|offered| offered.map(FakeOffer))
        }

        async fn download(&self, _offer: &FakeOffer) -> Result<Vec<u8>, Failure> {
            *self.downloads.lock().unwrap() += 1;
            self.download.lock().unwrap().clone()
        }
    }

    fn failure(code: FailureCode) -> Failure {
        Failure::new(code, "boom")
    }

    fn fake_updates(
        check: Result<Option<&str>, Failure>,
        download: Result<&[u8], Failure>,
    ) -> Updates<FakeSource> {
        Updates::new(FakeSource::new(check, download), "0.1.0")
    }

    #[tokio::test]
    async fn only_a_newer_version_is_offered() {
        for (offered, newer) in [
            ("0.2.0", true),
            ("v0.1.1", true),
            ("0.1.0", false),
            ("0.0.9", false),
            ("0.1.0-rc.1", false),
            ("latest", false),
        ] {
            let updates = fake_updates(Ok(Some(offered)), Ok(b"payload"));
            let status = updates.check().await;
            assert_eq!(
                matches!(status, UpdateStatus::Available { .. }),
                newer,
                "{offered}"
            );
            // Nothing that is not newer can be downloaded, let alone installed.
            assert_eq!(updates.prepare().await.is_ok(), newer, "{offered}");
        }
        let updates = fake_updates(Ok(None), Ok(b"payload"));
        assert!(matches!(
            updates.check().await,
            UpdateStatus::UpToDate { .. }
        ));
    }

    #[tokio::test]
    async fn a_failed_check_only_informs() {
        for code in [FailureCode::Unreachable, FailureCode::NoPlatform] {
            let updates = fake_updates(Err(failure(code)), Ok(b"payload"));
            assert_eq!(updates.check().await, UpdateStatus::Failed(failure(code)));
            assert_eq!(updates.status(), UpdateStatus::Failed(failure(code)));
            assert!(updates.prepare().await.is_err());
            assert_eq!(updates.pending_version(), None);
            assert_eq!(*updates.source.downloads.lock().unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn a_bad_signature_keeps_nothing_to_install() {
        let updates = fake_updates(Ok(Some("0.2.0")), Err(failure(FailureCode::BadSignature)));
        updates.check().await;
        assert_eq!(
            updates.prepare().await.unwrap_err().code,
            FailureCode::BadSignature
        );
        assert_eq!(
            updates.pending_version(),
            None,
            "nothing unverified waits for the quit flow"
        );
        assert!(updates.take_pending().is_none());
        // The offer itself stays: the banner still says a new version exists.
        assert!(matches!(updates.status(), UpdateStatus::Available { .. }));
    }

    #[tokio::test]
    async fn a_verified_download_waits_for_the_quit_flow_and_is_reused() {
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.check().await;
        assert_eq!(updates.prepare().await.unwrap(), "0.2.0");
        assert_eq!(updates.pending_version().as_deref(), Some("0.2.0"));
        // "Keep running" in the prompt, then "install and restart" again: no second download.
        assert_eq!(updates.prepare().await.unwrap(), "0.2.0");
        assert_eq!(*updates.source.downloads.lock().unwrap(), 1);
        // A plain quit drops it (quit::request), so quitting never installs.
        updates.clear_pending();
        assert_eq!(updates.pending_version(), None);
        updates.prepare().await.unwrap();
        let pending = updates.take_pending().expect("downloaded");
        assert_eq!(pending.bytes, b"payload");
        assert!(updates.take_pending().is_none(), "installed once");
    }

    #[tokio::test]
    async fn a_newer_version_found_later_is_downloaded_instead_of_the_one_that_waits() {
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"old"));
        updates.check().await;
        updates.prepare().await.unwrap();
        *updates.source.check.lock().unwrap() = Ok(Some("0.3.0".to_string()));
        *updates.source.download.lock().unwrap() = Ok(b"new".to_vec());
        updates.check().await;
        // "Install and restart" now means the newest one, not the download that was waiting.
        assert_eq!(updates.prepare().await.unwrap(), "0.3.0");
        assert_eq!(updates.pending_version().as_deref(), Some("0.3.0"));
        assert_eq!(updates.take_pending().expect("downloaded").bytes, b"new");
    }

    #[tokio::test]
    async fn an_offer_that_is_gone_at_the_next_check_cannot_be_installed() {
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.check().await;
        *updates.source.check.lock().unwrap() = Ok(None);
        assert!(matches!(
            updates.check().await,
            UpdateStatus::UpToDate { .. }
        ));
        assert!(matches!(updates.status(), UpdateStatus::UpToDate { .. }));
        assert!(updates.prepare().await.is_err());
        assert_eq!(*updates.source.downloads.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn a_failed_check_keeps_the_version_already_offered() {
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.check().await;
        *updates.source.check.lock().unwrap() = Err(failure(FailureCode::Unreachable));
        // This check's outcome is the failure; what the panel keeps showing is the offer.
        assert_eq!(
            updates.check().await,
            UpdateStatus::Failed(failure(FailureCode::Unreachable))
        );
        assert_eq!(
            updates.status(),
            UpdateStatus::Available {
                version: "0.2.0".into()
            }
        );
    }

    #[test]
    fn an_install_that_fails_or_panics_says_why_and_never_hangs_quitting() {
        assert_eq!(install_outcome(|| Ok::<(), String>(())), Ok(()));
        assert_eq!(
            install_outcome(|| Err("disk is full")),
            Err("disk is full".to_string())
        );
        // A panic is an outcome too, whatever it carried (the panic hook has logged it by then).
        assert_eq!(
            install_outcome(|| -> Result<(), String> { panic!("boom") }),
            Err("the installer stopped unexpectedly".to_string())
        );
    }

    #[test]
    fn each_version_notifies_once() {
        let updates = fake_updates(Ok(None), Ok(b""));
        assert!(updates.should_notify("0.2.0"));
        assert!(!updates.should_notify("0.2.0"));
        assert!(updates.should_notify("0.3.0"));
    }

    #[test]
    fn plugin_errors_become_codes() {
        use tauri_plugin_updater::Error as E;
        assert_eq!(failure_code(&E::ReleaseNotFound), FailureCode::Unreachable);
        assert_eq!(
            failure_code(&E::TargetsNotFound(vec!["darwin-aarch64".into()])),
            FailureCode::NoPlatform
        );
        // What else says nothing could be fetched, that there is no package for this system, that
        // the payload is not the signed one, or none of those (the plugin's enum is
        // `#[non_exhaustive]`; the variants with the plugin's own dependency types, such as a
        // failed request or a signature that does not verify, are not made here).
        let not_json = serde_json::from_str::<()>("not json").unwrap_err();
        for (error, code) in [
            (E::Serialization(not_json), FailureCode::Unreachable),
            (
                E::Network("Download request failed with status: 404 Not Found".into()),
                FailureCode::Unreachable,
            ),
            (
                E::TargetNotFound("darwin-aarch64".into()),
                FailureCode::NoPlatform,
            ),
            (
                E::SignatureUtf8("not base64".into()),
                FailureCode::BadSignature,
            ),
            (
                E::SignedVersionMismatch {
                    signed: "0.1.0".into(),
                    announced: "9.9.9".into(),
                },
                FailureCode::BadSignature,
            ),
            (E::MissingSignedVersion, FailureCode::BadSignature),
            (E::EmptyEndpoints, FailureCode::Other),
            (E::UnsupportedOs, FailureCode::Other),
            (
                E::Io(std::io::Error::other("disk full")),
                FailureCode::Other,
            ),
        ] {
            assert_eq!(failure_code(&error), code, "{error}");
        }
    }

    #[test]
    fn the_panel_reads_the_status() {
        let failed = UpdateStatus::Failed(Failure::new(
            FailureCode::BadSignature,
            "The signature verification failed",
        ));
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            serde_json::json!({"kind": "failed", "code": "bad_signature", "detail": "The signature verification failed"})
        );
        let available = UpdateStatus::Available {
            version: "0.2.0".into(),
        };
        assert_eq!(
            serde_json::to_value(available).unwrap(),
            serde_json::json!({"kind": "available", "version": "0.2.0"})
        );
        // A detail with control characters arrives cleaned.
        assert_eq!(
            Failure::new(FailureCode::Other, "bad\u{1b}[2Jline\n").detail,
            "bad [2Jline"
        );
    }

    #[test]
    fn the_failure_codes_follow_the_shared_case_table() {
        // The panel's codes, the error codes `install_update` answers with and, as
        // `errors.<error_code>`, the translation keys: one table, read by the TS tests too.
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../src/lib/update-failure-cases.json")).unwrap();
        assert_eq!(cases.len(), FailureCode::ALL.len(), "one case per code");
        for (case, code) in cases.iter().zip(FailureCode::ALL) {
            assert_eq!(
                case["code"],
                serde_json::to_value(code).unwrap(),
                "{code:?}"
            );
            assert_eq!(case["error_code"], code.error_code(), "{code:?}");
        }
    }

    #[test]
    fn a_failed_install_is_shown_once_after_the_restart() {
        let data = tempfile::tempdir().unwrap();
        assert_eq!(take_install_failure(data.path()), None, "nothing recorded");
        // Written by the process that failed to install, just before it restarts as it is.
        record_install_failure(data.path(), "cannot replace\u{1b}[2J the app\n");
        let failure = take_install_failure(data.path()).expect("recorded before the restart");
        assert_eq!(failure.code, FailureCode::InstallFailed);
        assert_eq!(failure.detail, "cannot replace [2J the app");
        assert_eq!(
            take_install_failure(data.path()),
            None,
            "shown once: the file is gone"
        );
        // A data folder that is not there is made; the note lands in it.
        let nested = data.path().join("not-yet");
        record_install_failure(&nested, "no space left");
        assert_eq!(
            take_install_failure(&nested).map(|failure| failure.detail),
            Some("no space left".to_string())
        );
        // The restarted app starts with it on show, until a check has something to say.
        let updates = fake_updates(Ok(Some("0.2.0")), Ok(b"payload"));
        updates.show_failure(failure.clone());
        assert_eq!(updates.status(), UpdateStatus::Failed(failure));
    }

    #[test]
    fn the_plugin_builds_an_updater_from_the_apps_own_configuration() {
        // The plugin as lib.rs registers it, reading the `plugins.updater` section of the real
        // tauri.conf.json: without that section, or with a key it cannot read, the app would not
        // even start. (Asking for an update would reach GitHub: not done here.)
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let mut context = mock_context(noop_assets());
        context
            .config_mut()
            .plugins
            .0
            .insert("updater".into(), config["plugins"]["updater"].clone());
        let app = mock_builder()
            .plugin(tauri_plugin_updater::Builder::new().build())
            .build(context)
            .expect("the plugin starts with the app's configuration");
        let updater = app.handle().updater_builder().timeout(CHECK_LIMIT).build();
        assert!(updater.is_ok(), "{:?}", updater.err());
    }

    #[test]
    fn the_quit_flow_helpers_run_on_any_runtime() {
        // Generic over the runtime like quit.rs, so the quit flow that calls them runs on tauri's
        // mock runtime in quit.rs's tests: without updater state nothing is pending.
        let app = mock_builder()
            .manage(crate::state::AppState::new(false))
            .build(mock_context(noop_assets()))
            .unwrap();
        assert!(!pending(app.handle()));
        assert!(take_pending(app.handle()).is_none());
        clear_pending(app.handle());
        // With the state the app manages: still nothing until a verified download exists.
        app.manage(AppUpdates::new(PluginSource(app.handle().clone()), "0.1.0"));
        assert!(!pending(app.handle()));
        assert_eq!(
            app.state::<AppUpdates<MockRuntime>>().status(),
            UpdateStatus::Unchecked
        );
    }
}
