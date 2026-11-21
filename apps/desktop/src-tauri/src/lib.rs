//! The FileSweep desktop application: a Tauri window over a scan session.
//!
//! The shell holds no logic. Checks, state and results belong to
//! `filesweep-session`; a command here only carries a call across.

#![forbid(unsafe_code)]

mod commands;
#[cfg(test)]
mod tests;

use filesweep_session::Session;

/// Generic over the runtime so that the tests build the same application on
/// a mock one.
fn shell<R: tauri::Runtime>(builder: tauri::Builder<R>) -> tauri::Builder<R> {
    builder
        .manage(Session::new())
        .invoke_handler(tauri::generate_handler![
            commands::scan_start,
            commands::scan_pause,
            commands::scan_resume,
            commands::scan_cancel,
            commands::scan_status,
            commands::scan_groups,
            commands::scan_skipped,
            commands::suggested_roots,
        ])
}

/// Opens the window and returns when it is closed.
pub fn run() -> tauri::Result<()> {
    shell(tauri::Builder::default())
        .plugin(tauri_plugin_dialog::init())
        .run(tauri::generate_context!())
}
