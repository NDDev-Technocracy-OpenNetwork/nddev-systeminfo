//! Debian helpers own enablement and policy; this hook never traverses homes or
//! starts a second provider. DPKG_ROOT fixtures never contact the host manager.
use super::*;
const SOCKET: &str = "rldyour-sysinfod.socket";
const SERVICE: &str = "rldyour-sysinfod.service";

fn helper(arguments: &[&str], probe: bool) -> Result<bool> {
    let arguments: Vec<_> = arguments.iter().map(std::ffi::OsStr::new).collect();
    let output = run("deb-systemd-helper", &arguments, false)?;
    match output.status.code() {
        Some(0) => Ok(true),
        Some(1) if probe => Ok(false),
        _ => Err(InstallError::Manager(format!(
            "deb-systemd-helper exited {}",
            output.status
        ))),
    }
}
fn manager(action: &str, units: &[&str]) -> Result {
    if std::env::var_os("DPKG_ROOT").is_some_and(|root| !root.is_empty())
        || !Path::new("/run/systemd/system").exists()
    {
        return Ok(());
    }
    let arguments: Vec<_> = ["--user", action]
        .into_iter()
        .chain(units.iter().copied())
        .map(std::ffi::OsStr::new)
        .collect();
    run("deb-systemd-invoke", &arguments, true)?;
    Ok(())
}
pub fn maintain() -> Result {
    let name = std::env::var("DPKG_MAINTSCRIPT_NAME")
        .map_err(|_| InstallError::Invalid("dpkg maintainer context missing"))?;
    let action = std::env::args()
        .nth(1)
        .ok_or(InstallError::Invalid("dpkg action missing"))?;
    match (name.as_str(), action.as_str()) {
        ("postinst", "configure" | "abort-upgrade" | "abort-deconfigure" | "abort-remove") => {
            helper(&["--user", "unmask", SOCKET], false)?;
            let operation = if helper(&["--quiet", "--user", "was-enabled", SOCKET], true)? {
                "enable"
            } else {
                "update-state"
            };
            helper(&["--user", operation, SOCKET], false)?;
            manager("daemon-reload", &[])?;
            manager("restart", &[SOCKET, SERVICE])?;
        }
        ("prerm", "remove" | "deconfigure") => {
            manager("stop", &[SERVICE, SOCKET])?;
            helper(&["--user", "mask", SOCKET], false)?;
        }
        ("postrm", "remove" | "purge") => {
            if action == "purge" {
                helper(&["--user", "purge", SOCKET], false)?;
            }
            helper(&["--user", "unmask", SOCKET], false)?;
            manager("daemon-reload", &[])?;
        }
        ("postinst" | "prerm" | "postrm", _) => {}
        _ => return Err(InstallError::Invalid("unknown dpkg maintainer hook")),
    }
    Ok(())
}
