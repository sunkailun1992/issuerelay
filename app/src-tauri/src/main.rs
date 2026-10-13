//! The desktop app's executable: it only starts `issuerelay_app::run`.

// No console window next to the tray app in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    issuerelay_app::run();
}
