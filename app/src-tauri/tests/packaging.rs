//! The packaging configuration (design 12). `tauri.conf.json` is read by every build, including
//! `tauri dev` and ci.yml's clippy and tests; what only bundling needs is merged from
//! `tauri.bundle.conf.json` with `--config`, or, when the release workflow signs update payloads,
//! from `tauri.updater.conf.json` (one `--config` at most).
use serde_json::{Value, json};

fn config(name: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn every_build_works_without_the_staged_cli() {
    let bundle = &config("tauri.conf.json")["bundle"];
    // tauri-build demands binaries/issuerelay-<triple> for every cargo build of this crate once
    // externalBin is in the base config.
    assert!(
        bundle.get("externalBin").is_none(),
        "externalBin belongs in tauri.bundle.conf.json"
    );
    // Signing update payloads needs the private key; only the release workflow switches it on.
    assert_eq!(bundle["createUpdaterArtifacts"], false);
}

#[test]
fn the_version_comes_from_the_cargo_workspace() {
    assert!(
        config("tauri.conf.json").get("version").is_none(),
        "the workspace Cargo.toml is the only version"
    );
    // Tauri then takes the crate's version, which must be the workspace's, not one of its own.
    let manifest = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
    )
    .expect("the app crate's Cargo.toml reads");
    let inherits = manifest
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .skip(1)
        .take_while(|line| !line.starts_with('['))
        .map(|line| line.replace(' ', ""))
        .any(|line| line == "version.workspace=true" || line == "version={workspace=true}");
    assert!(
        inherits,
        "issuerelay-app must use `version.workspace = true`"
    );
}

#[test]
fn the_main_executable_never_collides_with_the_cli() {
    let base = config("tauri.conf.json");
    let main = base["mainBinaryName"]
        .as_str()
        .expect("mainBinaryName is set");
    assert_eq!(main, issuerelay_core::paths::APP_EXECUTABLE);
    // APFS and NTFS ignore case: the two files would silently replace each other in the bundle.
    assert!(!main.eq_ignore_ascii_case("issuerelay"));
}

#[test]
fn installers_are_per_user_and_mac_bundles_are_sealed() {
    let bundle = &config("tauri.conf.json")["bundle"];
    assert_eq!(bundle["targets"], json!(["app", "dmg", "msi", "nsis"]));
    assert_eq!(bundle["windows"]["nsis"]["installMode"], "currentUser");
    // Without an Apple certificate the app is signed ad hoc: an arm64 app with only the linker's
    // signature has no sealed resources, and macOS calls a downloaded copy damaged.
    assert_eq!(bundle["macOS"]["signingIdentity"], "-");
    // The panel uses Tailwind 4 (oklch, color-mix, @property), whose baseline is Safari 16.4:
    // the system WebKit of macOS 13.3 (ruling M3-R4).
    assert_eq!(bundle["macOS"]["minimumSystemVersion"], "13.3");
}

#[test]
fn bundling_adds_the_cli() {
    assert_eq!(
        config("tauri.bundle.conf.json")["bundle"]["externalBin"],
        json!(["binaries/issuerelay"])
    );
}

#[test]
fn updates_come_signed_from_the_fixed_manifest() {
    let updater = &config("tauri.conf.json")["plugins"]["updater"];
    assert_eq!(
        updater["endpoints"],
        json!(["https://github.com/sunkailun1992/issuerelay/releases/latest/download/latest.json"])
    );
    // A signed payload names the version it was signed for, and that must be the version the
    // manifest announces: a newer number cannot be paired with an older, genuine payload.
    assert_eq!(updater["requireSignedVersion"], true);
    assert_eq!(updater["windows"]["installMode"], "passive");
    // The .pub file's one base64 line: a minisign public key box, which starts with
    // "untrusted comment: minisign public k". A file path, or anything else, is refused here.
    let pubkey = updater["pubkey"].as_str().expect("pubkey is set");
    assert!(
        pubkey.starts_with("dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBr"),
        "not a minisign public key"
    );
    assert!(!pubkey.contains(char::is_whitespace));
}

#[test]
fn release_builds_add_the_cli_and_the_update_payloads() {
    let release = &config("tauri.updater.conf.json")["bundle"];
    assert_eq!(
        release["externalBin"],
        config("tauri.bundle.conf.json")["bundle"]["externalBin"]
    );
    // A boolean, not a string: tauri-action reads it with `=== true` to pick the signed names
    // that scripts/release-checks.mjs expects.
    assert_eq!(release["createUpdaterArtifacts"], true);
}

#[test]
fn bundle_configs_never_change_what_the_app_trusts() {
    // scripts/verify-update-signature.mjs checks every payload against the pubkey committed in
    // tauri.conf.json. A bundle config with a `plugins` section would let the shipped app trust
    // another key than the one the release is checked against.
    for name in bundle_configs() {
        assert!(
            config(name).get("plugins").is_none(),
            "{name} must not touch the plugins' configuration"
        );
    }
}

/// The configs `--config` is given when bundling, one at most per build (bundle.yml): the CLI's
/// (`tauri.bundle.conf.json`), or `tauri.updater.conf.json` instead when the release workflow
/// signs update payloads, which repeats the CLI's part.
fn bundle_configs() -> Vec<&'static str> {
    ["tauri.bundle.conf.json", "tauri.updater.conf.json"]
        .into_iter()
        .filter(|name| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join(name)
                .exists()
        })
        .collect()
}

#[test]
fn bundling_embeds_the_panel_as_it_was_built_and_checked() {
    // The release workflow builds the panel and gates it (scripts/release-checks.mjs panel) in a
    // step without secrets. `tauri build` runs beforeBuildCommand (`pnpm build`) otherwise, with
    // the signing secrets in its environment: a bundle config removes the hook. `--config` is an
    // RFC 7396 merge patch, so a null deletes the key (a missing key would leave the hook).
    assert!(
        !bundle_configs().is_empty(),
        "no bundle config found: the check below would pass for nothing"
    );
    for name in bundle_configs() {
        assert_eq!(
            config(name)["build"].get("beforeBuildCommand"),
            Some(&Value::Null),
            "{name} must set build.beforeBuildCommand to null"
        );
    }
}
