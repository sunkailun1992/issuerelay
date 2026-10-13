//! The commands the panel calls; Task 4 fills them in.

/// No commands yet.
pub fn handler() -> impl Fn(tauri::ipc::Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![]
}
