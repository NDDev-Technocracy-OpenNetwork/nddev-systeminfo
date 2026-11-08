//! Debian helpers own enablement; restart admission is scoped to each live user
//! manager. This hook never traverses homes. DPKG_ROOT never contacts the host.
use super::*;
use std::os::unix::fs::PermissionsExt;
const SOCKET: &str = "rldyour-sysinfod.socket";
const SERVICE: &str = "rldyour-sysinfod.service";

fn live_host() -> bool {
    std::env::var_os("DPKG_ROOT").is_none_or(|root| root.is_empty())
        && Path::new("/run/systemd/system").exists()
}
fn active_user_ids() -> Result<Vec<u32>> {
    let users = run(
        "systemctl",
        &[
            "--no-legend".as_ref(),
            "--plain".as_ref(),
            "--quiet".as_ref(),
            "list-units".as_ref(),
            "user@*.service".as_ref(),
        ],
        true,
    )?;
    Ok(String::from_utf8_lossy(&users.stdout)
        .lines()
        .filter_map(|line| {
            line.split_whitespace()
                .next()
                .and_then(|name| name.strip_prefix("user@"))
                .and_then(|name| name.strip_suffix(".service"))
                .and_then(|uid| uid.parse().ok())
        })
        .collect())
}
fn stop_ui_users() -> Result {
    if !live_host() || !Path::new("/usr/bin/gapplication").is_file() {
        return Ok(());
    }
    for uid in active_user_ids()? {
        let bus = format!("/run/user/{uid}/bus");
        if !Path::new(&bus).exists() {
            continue;
        }
        let passwd = run(
            "getent",
            &["passwd".as_ref(), uid.to_string().as_ref()],
            false,
        )?;
        let record = String::from_utf8_lossy(&passwd.stdout);
        let Some(user) = record
            .lines()
            .next()
            .and_then(|line| line.split(':').next())
            .filter(|user| !user.is_empty())
        else {
            continue;
        };
        // Only the product's own action on an existing user's bus. runuser drops
        // root privileges and supplementary groups; no home is traversed.
        let address = format!("DBUS_SESSION_BUS_ADDRESS=unix:path={bus}");
        let _ = run(
            "runuser",
            &[
                "--user".as_ref(),
                user.as_ref(),
                "--".as_ref(),
                "env".as_ref(),
                address.as_ref(),
                "/usr/bin/gapplication".as_ref(),
                "action".as_ref(),
                crate::desktop::application_id().as_ref(),
                "quit".as_ref(),
            ],
            false,
        )?;
    }
    Ok(())
}

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
    if !live_host() {
        return Ok(());
    }
    if action == "restart" {
        return restart_users(units);
    }
    let arguments: Vec<_> = ["--user", action]
        .into_iter()
        .chain(units.iter().copied())
        .map(std::ffi::OsStr::new)
        .collect();
    run("deb-systemd-invoke", &arguments, true)?;
    Ok(())
}

fn restart_users(units: &[&str]) -> Result {
    // Debian 1.69's invoke helper retains its restart list between user
    // instances. Never let one user's admission affect a later masked user.
    let policy = "/usr/sbin/policy-rc.d";
    if fs::metadata(policy)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
    {
        for unit in units {
            let result = run(policy, &[unit.as_ref(), "restart".as_ref()], false)?;
            match result.status.code() {
                Some(0 | 104) => {}
                Some(101) => return Ok(()),
                _ => {
                    return Err(InstallError::Manager(format!(
                        "service restart policy exited {}",
                        result.status
                    )));
                }
            }
        }
    }
    for uid in active_user_ids()? {
        let instance = format!("{uid}@");
        let mut admitted: Vec<&std::ffi::OsStr> = Vec::new();
        for unit in units {
            let enabled = run(
                "systemctl",
                &[
                    "--user".as_ref(),
                    "--machine".as_ref(),
                    instance.as_ref(),
                    "is-enabled".as_ref(),
                    "--".as_ref(),
                    unit.as_ref(),
                ],
                false,
            )?;
            let state = String::from_utf8_lossy(&enabled.stdout);
            if matches!(state.trim(), "masked" | "masked-runtime") {
                continue;
            }
            let active = run(
                "systemctl",
                &[
                    "--user".as_ref(),
                    "--machine".as_ref(),
                    instance.as_ref(),
                    "--quiet".as_ref(),
                    "is-active".as_ref(),
                    "--".as_ref(),
                    unit.as_ref(),
                ],
                false,
            )?;
            if matches!(state.trim(), "enabled" | "enabled-runtime") || active.status.success() {
                admitted.push(unit.as_ref());
            }
        }
        if !admitted.is_empty() {
            let mut arguments = vec![
                "--user".as_ref(),
                "--machine".as_ref(),
                instance.as_ref(),
                "restart".as_ref(),
                "--".as_ref(),
            ];
            arguments.extend(admitted);
            run("systemctl", &arguments, true)?;
        }
    }
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
            stop_ui_users()?;
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
