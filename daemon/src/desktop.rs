//! Launch the installed native presentation. No display toolkit enters the CLI
//! or the headless provider. Existing service and settings ownership is kept.
use crate::analysis::Selection;
use std::{io, path::Path, process::Command};

pub fn application_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        let product: serde_json::Value =
            serde_json::from_str(include_str!("../../config/product.json"))
                .expect("checked product metadata");
        product["application_id"]
            .as_str()
            .expect("checked application identity")
            .to_owned()
    })
}
#[cfg(target_os = "linux")]
pub fn stop_ui() {
    // Missing desktop tools/session are normal on a headless provider host.
    // This requests only this user's explicit native application action.
    let _ = Command::new("gapplication")
        .args(["action", application_id(), "quit"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

pub fn open(config: Option<&Path>, metric: Option<&str>) -> io::Result<()> {
    if metric.is_some_and(|key| Selection::parse(key).is_none()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown metric series",
        ));
    }
    #[cfg(any(target_os = "macos", target_os = "windows"))]
    if config.is_some() || metric.is_some() {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "the installed native app uses its saved profile; select a metric in its window",
        ));
    }
    #[cfg(target_os = "linux")]
    let mut command = {
        let home =
            std::env::var_os("HOME").ok_or_else(|| io::Error::other("home directory missing"))?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| Path::new(&home).join(".local/share"));
        let relative =
            "gnome-shell/extensions/rldyour-sysinfo@nddev-opennetwork/analysis-window.js";
        let path = std::iter::once(data)
            .chain(std::env::split_paths(
                &std::env::var_os("XDG_DATA_DIRS")
                    .unwrap_or_else(|| "/usr/local/share:/usr/share".into()),
            ))
            .map(|root| root.join(relative))
            .find(|file| file.is_file())
            .ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "install the native Systeminfo UI")
            })?;
        let mut command = Command::new("gjs");
        command
            .arg("-m")
            .arg(path)
            .arg("--cli")
            .arg(std::env::current_exe()?);
        if let Some(path) = config {
            command.arg("--config").arg(path);
        }
        if let Some(key) = metric {
            command.arg("--metric").arg(key);
        }
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command
            .arg("-a")
            .arg("NDDev Systeminfo")
            .arg("--args")
            .arg("--show");
        command
    };
    #[cfg(target_os = "windows")]
    let mut command = {
        let path = std::env::current_exe()?.with_file_name("nddev-systeminfo-tray.exe");
        let mut command = Command::new(path);
        command.arg("--show");
        command
    };
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other("native presentation could not start"))
    }
}
