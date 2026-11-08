//! Real-account lifecycle acceptance on disposable public runners only. It must
//! never install a test release over a developer's working desktop provider.
use crate::{Result, command};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) fn run() -> Result {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var_os("RUNNER_TEMP").is_none()
    {
        return Err("installation acceptance requires a disposable GitHub runner".into());
    }
    let source = crate::packaging::package()?;
    let cli = source
        .join("bin")
        .join(format!("nddev-systeminfo{}", std::env::consts::EXE_SUFFIX));
    let mut environment: Vec<(String, String)> = Vec::new();
    if cfg!(target_os = "linux") {
        let output = Command::new("id").arg("-u").output()?;
        let uid: u32 = String::from_utf8(output.stdout)?.trim().parse()?;
        command(
            "sudo",
            &["systemctl", "start", &format!("user@{uid}.service")],
        )?;
        environment.push(("XDG_RUNTIME_DIR".into(), format!("/run/user/{uid}")));
        environment.push((
            "DBUS_SESSION_BUS_ADDRESS".into(),
            format!("unix:path=/run/user/{uid}/bus"),
        ));
    }
    let invoke = |executable: &std::path::Path, arguments: &[&str]| -> Result<Output> {
        eprintln!("native installation step: {arguments:?}");
        let mut child = Command::new(executable)
            .envs(environment.iter().cloned())
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            if let Some(status) = child.try_wait()? {
                eprintln!("native installation process exited: {status}");
                break;
            }
            if Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("native installation step timed out: {arguments:?}").into());
            }
            thread::sleep(Duration::from_millis(10));
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            #[cfg(target_os = "windows")]
            if let Some(local) = std::env::var_os("LOCALAPPDATA") {
                for name in ["maintenance-install.log", "maintenance-uninstall.log"] {
                    let log = PathBuf::from(&local)
                        .join("nddev-systeminfo/logs")
                        .join(name);
                    if let Ok(bytes) = fs::read(log) {
                        let log = if bytes.starts_with(&[0xff, 0xfe]) {
                            String::from_utf16_lossy(
                                &bytes[2..]
                                    .chunks_exact(2)
                                    .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
                                    .collect::<Vec<_>>(),
                            )
                        } else {
                            String::from_utf8_lossy(&bytes).into_owned()
                        };
                        eprintln!("{name}:\n{log}");
                    }
                }
            }
            if arguments == ["uninstall"] {
                eprintln!(
                    "uninstall executable remaining={}, readonly={:?}",
                    executable.exists(),
                    executable
                        .metadata()
                        .ok()
                        .map(|data| data.permissions().readonly())
                );
            }
            return Err(format!(
                "native command failed: {arguments:?}, {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(output)
    };
    invoke(&cli, &["install", "--source", source.to_str().unwrap()])?;
    let installed = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("local app data absent")?)
            .join("nddev-systeminfo/versions")
            .join(crate::version()?)
            .join("bin/nddev-systeminfo.exe")
    } else {
        PathBuf::from(std::env::var_os("HOME").ok_or("home absent")?)
            .join(".local/bin/nddev-systeminfo")
    };
    invoke(&installed, &["--version"])?;
    let path = invoke(&installed, &["config", "path"])?;
    let config = PathBuf::from(String::from_utf8(path.stdout)?.trim());
    if !config.exists() {
        invoke(&installed, &["config", "init"])?;
    }
    invoke(
        &installed,
        &["config", "set", "sampling.interval_seconds", "0"],
    )?;
    invoke(
        &installed,
        &["config", "set", "display.show_network", "false"],
    )?;
    let saved = fs::read(&config)?;
    invoke(&installed, &["service", "disable"])?;
    let before: serde_json::Value =
        serde_json::from_slice(&invoke(&installed, &["service", "status"])?.stdout)?;
    if before["enabled"] != false {
        return Err("native service disable did not report disabled state".into());
    }
    invoke(&cli, &["install", "--source", source.to_str().unwrap()])?;
    let status: serde_json::Value =
        serde_json::from_slice(&invoke(&installed, &["service", "status"])?.stdout)?;
    if status["enabled"] != false {
        return Err("reinstall re-enabled a disabled provider".into());
    }
    if fs::read(&config)? != saved {
        return Err("reinstall changed existing user preferences".into());
    }
    invoke(&installed, &["service", "enable"])?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let result = Command::new(&installed)
            .envs(environment.iter().cloned())
            .args(["sample", "--once"])
            .output()?;
        if result.status.success() {
            nddev_systeminfo::protocol::decode(result.stdout.trim_ascii_end())?;
            break;
        }
        if Instant::now() >= deadline {
            return Err("enabled native provider did not serve the real CLI".into());
        }
        thread::sleep(Duration::from_millis(20));
    }
    invoke(&cli, &["install", "--source", source.to_str().unwrap()])?;
    invoke(&installed, &["service", "apply"])?;
    invoke(&installed, &["sample", "--once"])?;
    // Uninstall through the installed executable, including Windows' executable
    // removal boundary. The user must not need a second download to uninstall.
    invoke(&installed, &["uninstall"])?;
    let deadline = Instant::now() + Duration::from_secs(30);
    while installed.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if installed.exists() {
        #[cfg(target_os = "windows")]
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            let log = PathBuf::from(local).join("nddev-systeminfo/logs/maintenance-uninstall.log");
            eprintln!("{}", fs::read_to_string(log).unwrap_or_default());
        }
        return Err("uninstall left the installed executable".into());
    }
    if fs::read(&config)? != saved {
        return Err("uninstall changed user configuration".into());
    }
    let artifacts = PathBuf::from(".tools/install-acceptance");
    fs::create_dir_all(&artifacts)?;
    fs::write(
        artifacts.join("result.json"),
        serde_json::to_string_pretty(&serde_json::json!({
            "os":std::env::consts::OS,"architecture":std::env::consts::ARCH,
            "install":true,"repeat":true,"disabled_preserved":true,"preferences_preserved":true,
            "real_cli":true,"service_apply":true,"uninstalled_self":true
        }))?,
    )?;
    println!(
        "PASS: real native install, repeat, disabled state, preferences, CLI, service apply and self-uninstall"
    );
    Ok(())
}
