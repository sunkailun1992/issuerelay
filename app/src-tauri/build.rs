//! Build script of the desktop app: a placeholder panel when `../dist` is missing, then the Tauri
//! build steps (with a Windows manifest that test executables get too).

use std::{env, fs, path::Path};

/// What tauri-build embeds into the app by default: it asks Windows for Common Controls v6.
const WINDOWS_MANIFEST: &str = r#"<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity
        type="win32"
        name="Microsoft.Windows.Common-Controls"
        version="6.0.0.0"
        processorArchitecture="*"
        publicKeyToken="6595b64144ccf1df"
        language="*"
      />
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    // Release builds (the `custom-protocol` feature, which `tauri build` turns on) embed `../dist`
    // through `tauri::generate_context!` and fail when it does not exist; debug builds, `cargo
    // test` and `cargo clippy` embed nothing and point at `devUrl`. So that no build depends on the
    // panel having been built first, give a missing `../dist` a placeholder page; `pnpm build`
    // (and `pnpm tauri build`) replace it with the real panel. (Checked with tauri 2.12.3 on
    // 2026-10-13.)
    let dist = Path::new("../dist");
    if !dist.join("index.html").exists() {
        fs::create_dir_all(dist).expect("cannot create ../dist");
        fs::write(
            dist.join("index.html"),
            "<!doctype html><title>IssueRelay</title><p>Run <code>pnpm --dir app build</code> to build the panel.</p>",
        )
        .expect("cannot write the placeholder panel");
    }

    if targets_windows_msvc() {
        // tauri-build links its manifest into the app binary only (`rustc-link-arg-bins`). A test
        // executable that links Tauri without it loads Common Controls v5 and does not even start
        // (STATUS_ENTRYPOINT_NOT_FOUND, tauri-apps/tauri#13419, open on 2026-10-13, tauri-build
        // 2.7.2). The scoped `rustc-link-arg-tests` does not reach the unit tests of this crate,
        // so the linker embeds the same manifest into every executable of the package instead.
        embed_windows_manifest();
        tauri_build::try_build(
            tauri_build::Attributes::new()
                .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
        )
        .expect("tauri-build failed");
    } else {
        tauri_build::build()
    }
}

fn targets_windows_msvc() -> bool {
    env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows")
        && env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|abi| abi == "msvc")
}

fn embed_windows_manifest() {
    let out_dir = env::var_os("OUT_DIR").expect("cargo sets OUT_DIR for build scripts");
    let manifest = Path::new(&out_dir).join("windows-app-manifest.xml");
    fs::write(&manifest, WINDOWS_MANIFEST).expect("cannot write the Windows manifest");
    println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
    println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
}
