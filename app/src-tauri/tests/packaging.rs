//! The packaging configuration (design 12). `tauri.conf.json` is read by every build, including
//! `tauri dev` and ci.yml's clippy and tests; what only bundling needs is merged from
//! `tauri.bundle.conf.json` with `--config`.
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
