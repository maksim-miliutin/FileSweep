// Without this a release build on Windows opens a console window next to the
// application.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::process::ExitCode;

fn main() -> ExitCode {
    match filesweep_desktop::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            // A release build on Windows has no console to show this in. A
            // message the user can see belongs to the installer stage.
            eprintln!("FileSweep could not start: {error}");
            ExitCode::FAILURE
        }
    }
}
