//! IssueRelay's desktop app: the tray icon and the panel around the engine.
// Every module is `pub`: only this crate's own executable uses the library, and an item that only
// one platform uses (the macOS application menu's texts, say) is then not dead code on the other.
pub mod commands;
pub mod events;
pub mod i18n;
pub mod logging;
pub mod notify;
pub mod quit;
pub mod startup;
pub mod state;
#[cfg(test)]
mod test_support;
pub mod tray;
pub mod update;
pub mod window;

use tauri::{Manager, RunEvent};

/// Passed by the login item, so the app starts in the tray (design 9.2: autostart is switched
/// on by the user in the settings).
pub const AUTOSTART_ARG: &str = "--autostart";

/// Starts the app and runs its event loop until it exits.
pub fn run() {
    // `args_os`: `args` would panic on an argument that is not valid Unicode.
    let autostarted = std::env::args_os().any(|arg| arg == AUTOSTART_ARG);
    let logs_dir = issuerelay_core::paths::AppPaths::system()
        .ok()
        .map(|paths| paths.logs_dir());
    let mut log_guard = logging::init(logs_dir.as_deref());
    logging::install_panic_hook();
    let builder = tauri::Builder::default()
        // Must come first: a second launch only brings the running instance's panel forward. (On
        // macOS, opening the app again through Launch Services starts no second process and never
        // gets here: the system's reopen event reaches `window::on_reopen` through the run loop.)
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show(app)
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        // By default the plugin adds a script to every page that opens clicked links itself; the
        // panel opens only Linear's links, through IssueRelay's own command and check
        // (`commands::open_linear`).
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        // Only IssueRelay's own commands drive it (`commands::{update_status, check_update,
        // install_update}`): the panel's capability grants no permission of this plugin.
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state::AppState::new(autostarted))
        .manage(commands::PendingLogin::default())
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::startup_state,
            commands::status,
            commands::list_jobs,
            commands::job,
            commands::job_log,
            commands::issues,
            commands::pause,
            commands::resume,
            commands::poll_now,
            commands::cancel_job,
            commands::retry_job,
            commands::mark_done,
            commands::bindings,
            commands::save_binding,
            commands::remove_binding,
            commands::projects,
            commands::settings,
            commands::save_settings,
            commands::doctor,
            commands::linear_login,
            commands::linear_cancel_login,
            commands::linear_use_api_key,
            commands::linear_logout,
            commands::linear_reconnect,
            commands::setup_action,
            commands::control_servers,
            commands::linear_server_named,
            commands::open_linear,
            commands::choose_directory,
            commands::autostart_enabled,
            commands::set_autostart,
            commands::quit,
            commands::request_quit,
            commands::cli_on_path,
            commands::install_cli_on_path,
            commands::remove_cli_from_path,
            commands::update_status,
            commands::check_update,
            commands::install_update,
        ])
        .setup(|app| {
            let updates = update::AppUpdates::new(
                update::PluginSource(app.handle().clone()),
                env!("CARGO_PKG_VERSION"),
            );
            // The previous run could not install an update and restarted as it was: the banner
            // says why, once.
            if let Some(failure) = issuerelay_core::paths::AppPaths::system()
                .ok()
                .and_then(|paths| update::take_install_failure(&paths.data_dir))
            {
                updates.show_failure(failure);
            }
            app.manage(updates);
            tray::create(app.handle())?;
            if app.state::<state::AppState>().autostarted {
                // A login start stays in the tray: through the same `hide` as closing the panel,
                // so macOS shows no Dock icon either.
                // That holds until the settings are read: `startup::start` brings the icon back
                // when "Show in Dock" is on, which it is unless the user switched it off.
                window::hide(app.handle());
            }
            tauri::async_runtime::spawn(startup::start(app.handle().clone()));
            update::spawn_checker(app.handle().clone());
            Ok(())
        })
        .on_window_event(window::on_event);
    // Cmd+Q must ask like the tray's Quit; Windows has no application menu (see `quit::app_menu`).
    #[cfg(target_os = "macos")]
    let builder = builder.menu(quit::app_menu);
    builder
        .build(tauri::generate_context!())
        .expect("IssueRelay could not start")
        .run(move |app, event| {
            let exiting = matches!(event, RunEvent::Exit);
            quit::on_run_event(app, event);
            if exiting {
                // `run` ends the process with `std::process::exit`, which runs no destructors:
                // the log file's last lines are written out here.
                drop(log_guard.take());
            }
        });
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{BTreeMap, BTreeSet},
        path::PathBuf,
    };

    use serde_json::Value;
    use tauri::utils::{
        acl::{ExecutionContext, capability::Capability, manifest::Manifest, resolved::Resolved},
        platform::Target,
    };

    /// The core permissions the panel uses: `api.ts` subscribes to the app's events with
    /// `listen`, and the function `listen` returns calls `unlisten`. Nothing else of Tauri's own.
    const PANEL_CORE_PERMISSIONS: [&str; 2] =
        ["core:event:allow-listen", "core:event:allow-unlisten"];

    /// The commands those two permissions allow, as the ACL names them.
    const PANEL_CORE_COMMANDS: [&str; 2] = ["plugin:event|listen", "plugin:event|unlisten"];

    /// The entries of the `[...]` list that starts right after `marker`, which occurs once.
    fn list_after<'a>(source: &'a str, marker: &str) -> Vec<&'a str> {
        assert_eq!(source.matches(marker).count(), 1, "`{marker}` occurs once");
        let start = source.find(marker).expect("counted above") + marker.len();
        let end = start + source[start..].find(']').expect("the list ends");
        source[start..end]
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .collect()
    }

    /// The commands the invoke handler registers, in order.
    fn registered() -> Vec<String> {
        // Written in two parts so that this line is not the list it looks for.
        let marker = concat!(".invoke_handler(tauri::", "generate_handler![");
        list_after(include_str!("lib.rs"), marker)
            .into_iter()
            .map(|entry| {
                entry
                    .strip_prefix("commands::")
                    .unwrap_or_else(|| panic!("`{entry}` is not a command of commands.rs"))
                    .to_string()
            })
            .collect()
    }

    /// The commands build.rs declares in the app manifest.
    fn declared() -> Vec<String> {
        list_after(
            include_str!("../build.rs"),
            "const PANEL_COMMANDS: &[&str] = &[",
        )
        .into_iter()
        .map(|entry| {
            entry
                .strip_prefix('"')
                .and_then(|entry| entry.strip_suffix('"'))
                .unwrap_or_else(|| panic!("`{entry}` is not a string literal"))
                .to_string()
        })
        .collect()
    }

    /// The functions commands.rs marks `#[tauri::command]`.
    fn defined() -> Vec<String> {
        let mut names = Vec::new();
        let mut lines = include_str!("commands.rs").lines().map(str::trim);
        while let Some(line) = lines.next() {
            if line != "#[tauri::command]" {
                continue;
            }
            let signature = lines
                .by_ref()
                .find(|line| !line.starts_with("#["))
                .expect("a function follows the attribute");
            let name = signature
                .strip_prefix("pub async fn ")
                .or_else(|| signature.strip_prefix("pub fn "))
                .and_then(|rest| rest.split(['(', '<']).next())
                .unwrap_or_else(|| panic!("not a command function: {signature}"));
            names.push(name.to_string());
        }
        names
    }

    fn sorted(mut names: Vec<String>) -> Vec<String> {
        names.sort();
        names
    }

    /// The webview boundary: every command the invoke handler registers is declared in the app
    /// manifest (which puts the app's own commands under the ACL) and granted to the panel's
    /// window, and the capability grants nothing else but the two event permissions the panel
    /// needs: no plugin permission, no other core permission, no scope.
    #[test]
    fn the_panel_is_granted_exactly_the_registered_commands() {
        let registered = registered();
        assert_eq!(
            registered.iter().collect::<BTreeSet<_>>().len(),
            registered.len(),
            "a command is registered twice: {registered:?}"
        );
        assert_eq!(
            sorted(defined()),
            sorted(registered.clone()),
            "the commands of commands.rs and the invoke handler"
        );
        let declared = declared();
        assert_eq!(
            declared.iter().collect::<BTreeSet<_>>().len(),
            declared.len(),
            "a command is declared twice: {declared:?}"
        );
        assert_eq!(
            sorted(declared),
            sorted(registered.clone()),
            "build.rs's app manifest and the invoke handler"
        );

        let capability: Value =
            serde_json::from_str(include_str!("../capabilities/default.json")).unwrap();
        let keys: BTreeSet<&str> = capability
            .as_object()
            .expect("the capability is an object")
            .keys()
            .map(String::as_str)
            .collect();
        // No `remote`, `local`, `webviews` or `platforms`: local content in the main window only.
        assert_eq!(
            keys,
            BTreeSet::from([
                "$schema",
                "identifier",
                "description",
                "windows",
                "permissions"
            ])
        );
        assert_eq!(capability["identifier"], "default");
        assert_eq!(capability["windows"], serde_json::json!(["main"]));
        let granted: Vec<&str> = capability["permissions"]
            .as_array()
            .expect("a list of permissions")
            .iter()
            .map(|entry| entry.as_str().expect("a plain identifier, without a scope"))
            .collect();
        let unique: BTreeSet<&str> = granted.iter().copied().collect();
        assert_eq!(unique.len(), granted.len(), "a permission is granted twice");
        let expected: BTreeSet<String> = PANEL_CORE_PERMISSIONS
            .iter()
            .map(|permission| permission.to_string())
            .chain(
                registered
                    .iter()
                    .map(|command| format!("allow-{}", command.replace('_', "-"))),
            )
            .collect();
        assert_eq!(
            unique,
            expected.iter().map(String::as_str).collect::<BTreeSet<_>>()
        );
    }

    /// What Tauri makes of it: the ACL this build resolved (the manifests and capability files in
    /// OUT_DIR, which `generate_context!` reads) checks the app's own commands and allows the
    /// main window exactly the registered ones and the two event commands, for local content.
    #[test]
    fn the_acl_the_build_resolves_allows_only_the_panels_commands() {
        let acl: BTreeMap<String, Manifest> = serde_json::from_str(include_str!(concat!(
            env!("OUT_DIR"),
            "/acl-manifests.json"
        )))
        .unwrap();
        let capabilities: BTreeMap<String, Capability> =
            serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/capabilities.json")))
                .unwrap();
        assert_eq!(
            capabilities.keys().collect::<Vec<_>>(),
            ["default"],
            "one capability"
        );
        let resolved = Resolved::resolve(&acl, capabilities, Target::current()).unwrap();
        assert!(
            resolved.has_app_acl,
            "without an app manifest Tauri lets the panel call every registered command"
        );
        let registered = registered();
        let expected: BTreeSet<&str> = registered
            .iter()
            .map(String::as_str)
            .chain(PANEL_CORE_COMMANDS)
            .collect();
        let allowed: BTreeSet<&str> = resolved
            .allowed_commands
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(allowed, expected);
        for (command, grants) in &resolved.allowed_commands {
            for grant in grants {
                assert_eq!(grant.context, ExecutionContext::Local, "{command}");
                let windows: Vec<&str> = grant.windows.iter().map(|w| w.as_str()).collect();
                assert_eq!(windows, ["main"], "{command}");
                assert!(
                    grant.webviews.is_empty() && grant.scope_id.is_none(),
                    "{command}"
                );
            }
        }
        assert!(resolved.denied_commands.is_empty());
        assert!(resolved.global_scope.is_empty() && resolved.command_scope.is_empty());
    }

    /// Nothing else can widen the grant: capabilities/ holds only default.json, and the
    /// configuration defines no capability of its own.
    #[test]
    fn no_other_capability_exists() {
        let files: Vec<String> =
            std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/capabilities"))
                .unwrap()
                .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
        assert_eq!(files, ["default.json"]);
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert!(config["app"]["security"].get("capabilities").is_none());
    }

    /// The webview's content security policy: local resources and the IPC only; no `<base>` that
    /// would move where relative addresses point, no form that posts anywhere, no plugin objects.
    #[test]
    fn the_csp_allows_only_local_content_and_nothing_else() {
        let config: Value = serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let csp = config["app"]["security"]["csp"]
            .as_str()
            .expect("a CSP string");
        let directives: BTreeMap<&str, &str> = csp
            .split(';')
            .map(str::trim)
            .filter(|directive| !directive.is_empty())
            .map(|directive| directive.split_once(' ').expect("a directive has a value"))
            .collect();
        assert_eq!(
            directives,
            BTreeMap::from([
                ("default-src", "'self'"),
                ("img-src", "'self' data:"),
                ("style-src", "'self' 'unsafe-inline'"),
                ("connect-src", "ipc: http://ipc.localhost"),
                ("base-uri", "'none'"),
                ("form-action", "'none'"),
                ("object-src", "'none'"),
            ])
        );
    }

    /// Nor can the app widen it while it runs, which the checks above would not see: Tauri's
    /// `Manager` call that adds a capability at run time (its name is put together below, so
    /// that this file does not contain it) appears in no source file of the crate.
    #[test]
    fn no_capability_is_added_at_run_time() {
        let call = concat!("add_", "capability");
        let mut pending = vec![PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))];
        let mut read = 0;
        while let Some(path) = pending.pop() {
            if path.is_dir() {
                pending.extend(
                    std::fs::read_dir(&path)
                        .unwrap()
                        .map(|entry| entry.unwrap().path()),
                );
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                let source = std::fs::read_to_string(&path).unwrap();
                assert!(
                    !source.contains(call),
                    "{} adds a capability at run time",
                    path.display()
                );
                read += 1;
            }
        }
        assert!(read >= 10, "the crate's sources were read: {read}");
    }
}
