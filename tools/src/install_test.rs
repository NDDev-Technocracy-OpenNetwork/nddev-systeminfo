//! Real-account lifecycle acceptance on disposable public runners only. It must
//! never install a test release over a developer's working desktop provider.
use crate::{Result, command};
use std::{
    fs,
    path::PathBuf,
    process::{Command, Output},
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
        let output = Command::new(executable)
            .envs(environment.iter().cloned())
            .args(arguments)
            .output()?;
        if !output.status.success() {
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
    if installed.exists() {
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
