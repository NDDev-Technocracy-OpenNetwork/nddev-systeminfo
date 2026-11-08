//! Windows native notification-area frontend; no WebView or rendering loop.
#![cfg_attr(windows, windows_subsystem = "windows")]
use std::process::ExitCode;
fn main() -> ExitCode {
    #[cfg(target_os = "windows")]
    match nddev_systeminfo::windows_ui::run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            nddev_systeminfo::telemetry::failure(&error.to_string());
            ExitCode::FAILURE
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        eprintln!("The notification-area frontend requires Windows.");
        ExitCode::FAILURE
    }
}
