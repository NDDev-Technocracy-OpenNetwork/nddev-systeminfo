use super::*;
const SOCKET: &str = "rldyour-sysinfod.socket";
const SERVICE: &str = "rldyour-sysinfod.service";
const UUID: &str = "rldyour-sysinfo@nddev-opennetwork";
fn manager(arguments: &[&str], required: bool) -> Result<Output> {
    let arguments: Vec<_> = std::iter::once(std::ffi::OsStr::new("--user"))
        .chain(arguments.iter().map(std::ffi::OsStr::new))
        .collect();
    run("systemctl", &arguments, required)
}
fn paths() -> Result<(PathBuf, PathBuf, PathBuf)> {
    let home = home()?;
    let units = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("systemd/user");
    let extensions = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"))
        .join("gnome-shell/extensions")
        .join(UUID);
    if !units.is_absolute() || !extensions.is_absolute() {
        return Err(InstallError::Invalid("XDG directories must be absolute"));
    }
    Ok((home.join(".local/bin"), units, extensions))
}
pub(super) fn install(source: &Path, manifest: &Manifest, provider_only: bool) -> Result {
    let (bin, units, extension) = paths()?;
    let existing = units.join(SOCKET).symlink_metadata().is_ok()
        || manager(&["show", SOCKET, "--property=LoadState", "--value"], false)
            .is_ok_and(|output| String::from_utf8_lossy(&output.stdout).trim() == "loaded");
    // Probe the real user manager before replacing any installed executable.
    manager(&["show-environment"], true)?;
    let masked = units.join(SOCKET).is_symlink()
        && fs::read_link(units.join(SOCKET))? == Path::new("/dev/null");
    let unit = include_str!("../../../daemon/systemd/rldyour-sysinfod.service").replace(
        "ExecStart=%h/.local/bin/rldyour-sysinfod",
        "ExecStart=%h/.local/bin/nddev-systeminfod",
    );
    for name in ["nddev-systeminfo", "nddev-systeminfod"] {
        copy_file(&source.join("bin").join(name), &bin.join(name), true)?;
    }
    atomic(&units.join(SERVICE), unit.as_bytes(), false)?;
    if !masked {
        atomic(
            &units.join(SOCKET),
            include_bytes!("../../../daemon/systemd/rldyour-sysinfod.socket"),
            false,
        )?;
    }
    if !provider_only {
        let payload = source.join("ui/extension");
        if !payload.is_dir() {
            return Err(InstallError::Invalid("release lacks the GNOME frontend"));
        }
        let staged = extension.with_file_name(format!(".{UUID}-new-{}", std::process::id()));
        copy_tree(&payload, &staged)?;
        run(
            "glib-compile-schemas",
            &[staged.join("schemas").as_os_str()],
            true,
        )?;
        let backup = extension.with_file_name(format!(".{UUID}-previous-{}", std::process::id()));
        if extension.exists() {
            if extension.is_symlink() {
                return Err(InstallError::Invalid("extension destination is redirected"));
            }
            fs::rename(&extension, &backup)?;
        }
        if let Err(error) = fs::rename(&staged, &extension) {
            if backup.exists() {
                let _ = fs::rename(&backup, &extension);
            }
            return Err(error.into());
        }
        if backup.exists() {
            fs::remove_dir_all(backup)?;
        }
    }
    manager(&["daemon-reload"], true)?;
    if !existing {
        manager(&["enable", "--now", SOCKET], true)?;
    } else {
        manager(&["try-restart", SERVICE], true)?;
    }
    receipt(manifest)?;
    // Legacy executable is retired only after its sole service owner has switched.
    remove_file(&bin.join("rldyour-sysinfod"))?;
    Ok(())
}
pub(super) fn service(operation: &str) -> Result<serde_json::Value> {
    match operation {
        "status" => Ok(
            serde_json::json!({"enabled":manager(&["is-enabled", SOCKET], false)?.status.success(),
            "active":manager(&["is-active", SERVICE], false)?.status.success()}),
        ),
        "apply" => {
            let active = manager(&["is-active", SERVICE], false)?.status.success();
            manager(&["try-restart", SERVICE], true)?;
            Ok(serde_json::json!({"applied":active}))
        }
        "start" => {
            manager(&["start", SOCKET], true)?;
            Ok(serde_json::json!({"active":true}))
        }
        "enable" => {
            manager(&["enable", "--now", SOCKET], true)?;
            Ok(serde_json::json!({"enabled":true}))
        }
        "stop" | "disable" => {
            if operation == "disable" {
                manager(&["disable", "--now", SOCKET], true)?;
            } else {
                manager(&["stop", SOCKET], true)?;
            }
            manager(&["stop", SERVICE], true)?;
            Ok(serde_json::json!({"enabled":false}))
        }
        "restart" => {
            manager(&["restart", SOCKET], true)?;
            manager(&["try-restart", SERVICE], true)?;
            Ok(serde_json::json!({"restarted":true}))
        }
        _ => Err(InstallError::Invalid("unknown service operation")),
    }
}
pub(super) fn uninstall() -> Result {
    let (bin, units, extension) = paths()?;
    if !state_dir()?.join("manifest.toml").is_file() {
        return Err(InstallError::Invalid("no owned user installation receipt"));
    }
    manager(&["disable", "--now", SOCKET], true)?;
    manager(&["stop", SERVICE], true)?;
    for name in [SOCKET, SERVICE] {
        remove_file(&units.join(name))?;
    }
    manager(&["daemon-reload"], true)?;
    for name in ["nddev-systeminfo", "nddev-systeminfod"] {
        remove_file(&bin.join(name))?;
    }
    if extension.exists() {
        if extension.is_symlink() {
            return Err(InstallError::Invalid("extension destination is redirected"));
        }
        fs::remove_dir_all(extension)?;
    }
    remove_file(&state_dir()?.join("manifest.toml"))?;
    Ok(())
}
