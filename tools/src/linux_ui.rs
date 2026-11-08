//! Real Shell/GTK acceptance in a private D-Bus and headless Wayland session.
use crate::{Result, build_binaries, command};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Scope {
    unit: String,
    stopped: bool,
}
impl Scope {
    fn stop(&mut self) -> Result {
        if self.stopped {
            return Ok(());
        }
        let status = Command::new("systemctl")
            .args(["--user", "stop", &self.unit])
            .output()?;
        if !status.status.success() {
            let state = Command::new("systemctl")
                .args([
                    "--user",
                    "show",
                    &self.unit,
                    "--property=LoadState",
                    "--value",
                ])
                .output()?;
            if String::from_utf8_lossy(&state.stdout).trim() != "not-found" {
                return Err("could not stop the owned UI process scope".into());
            }
        }
        self.stopped = true;
        Ok(())
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
fn copy_tree(from: &Path, to: &Path) -> Result {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &to.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
    }
    Ok(())
}
pub(super) fn run() -> Result {
    if !cfg!(target_os = "linux") {
        return Err("GNOME acceptance requires a Linux build host".into());
    }
    let binaries = build_binaries(false)?;
    let root = Directory(std::env::temp_dir().join(format!(
            "si-ui-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        )));
    let artifacts = std::env::current_dir()?.join(".tools/ui-linux");
    if artifacts.exists() {
        fs::remove_dir_all(&artifacts)?;
    }
    fs::create_dir_all(&artifacts)?;
    for name in ["run", "config", "data", "cache", "bin"] {
        fs::create_dir_all(root.0.join(name))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&root.0, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(root.0.join("run"), fs::Permissions::from_mode(0o700))?;
    }
    for name in ["nddev-systeminfo", "nddev-systeminfod"] {
        fs::copy(
            binaries.get(name).ok_or("native binary missing")?,
            root.0.join("bin").join(name),
        )?;
    }
    let metadata: serde_json::Value =
        serde_json::from_str(&fs::read_to_string("extension/metadata.json")?)?;
    let uuid = metadata["uuid"].as_str().ok_or("extension UUID missing")?;
    let extensions = root.0.join("data/gnome-shell/extensions");
    let extension = extensions.join(uuid);
    copy_tree(Path::new("extension"), &extension)?;
    command(
        "glib-compile-schemas",
        &[extension.join("schemas").to_str().unwrap()],
    )?;
    let driver = extensions.join("systeminfo-ui-test@opennetwork");
    fs::create_dir_all(&driver)?;
    fs::write(
        driver.join("extension.js"),
        include_str!("../templates/linux-ui-extension.js"),
    )?;
    fs::write(
        driver.join("gtk-inspect.js"),
        include_str!("../templates/gtk-inspect.js"),
    )?;
    fs::write(
        driver.join("metadata.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"uuid":"systeminfo-ui-test@opennetwork","name":"Systeminfo native acceptance","description":"Isolated test driver","shell-version":metadata["shell-version"]}),
        )?,
    )?;
    let path = std::env::join_paths(std::iter::once(root.0.join("bin")).chain(
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
    ))?;
    // D-Bus activated GTK/portal helpers can outlive the compositor. The owned
    // systemd scope includes them even if a helper forks or changes sessions.
    let mut scope = Scope {
        unit: format!(
            "nddev-{}.scope",
            root.0.file_name().unwrap().to_string_lossy()
        ),
        stopped: false,
    };
    let mut session = Command::new("systemd-run");
    session
        .args([
            "--user",
            "--scope",
            "--quiet",
            "--collect",
            "--unit",
            &scope.unit,
            "--",
            "env",
            "-u",
            "DISPLAY",
            "-u",
            "SESSION_MANAGER",
        ])
        .arg(format!("XDG_RUNTIME_DIR={}", root.0.join("run").display()))
        .arg(format!(
            "XDG_CONFIG_HOME={}",
            root.0.join("config").display()
        ))
        .arg(format!("XDG_DATA_HOME={}", root.0.join("data").display()))
        .arg(format!("XDG_CACHE_HOME={}", root.0.join("cache").display()))
        .arg(format!("PATH={}", path.to_string_lossy()))
        .arg("WAYLAND_DISPLAY=systeminfo-test")
        .args(["dbus-run-session", "--"])
        .arg(std::env::current_exe()?)
        .arg("ui-linux-session")
        .arg(&root.0)
        .arg(&artifacts)
        .stdout(fs::File::create(artifacts.join("session.log"))?)
        .stderr(fs::File::create(artifacts.join("session.stderr"))?);
    let status = session.status();
    scope.stop()?;
    if !status?.success() {
        return Err(format!(
            "native Shell acceptance failed; inspect {}",
            artifacts.display()
        )
        .into());
    }
    println!(
        "native Shell and GTK acceptance passed; {}",
        artifacts.display()
    );
    fs::remove_dir_all(&root.0)?;
    Ok(())
}
pub(super) fn session() -> Result {
    let mut arguments = std::env::args().skip(2);
    let root = PathBuf::from(arguments.next().ok_or("UI fixture missing")?);
    let artifacts = PathBuf::from(arguments.next().ok_or("UI artifacts missing")?);
    let schema_path =
        root.join("data/gnome-shell/extensions/rldyour-sysinfo@nddev-opennetwork/schemas");
    command(
        "gsettings",
        &[
            "--schemadir",
            schema_path.to_str().unwrap(),
            "set",
            "org.gnome.shell.extensions.rldyour-sysinfo",
            "interval",
            "0",
        ],
    )?;
    command(
        "gsettings",
        &[
            "--schemadir",
            schema_path.to_str().unwrap(),
            "set",
            "org.gnome.shell.extensions.rldyour-sysinfo",
            "show-network",
            "false",
        ],
    )?;
    command(
        "gsettings",
        &[
            "set",
            "org.gnome.shell",
            "enabled-extensions",
            "['rldyour-sysinfo@nddev-opennetwork', 'systeminfo-ui-test@opennetwork']",
        ],
    )?;
    command(
        "gsettings",
        &["set", "org.gnome.shell", "disable-user-extensions", "false"],
    )?;
    command(
        "gsettings",
        &[
            "set",
            "org.gnome.desktop.interface",
            "enable-animations",
            "false",
        ],
    )?;
    // Only this disposable compositor: it has no login/display manager.
    command(
        "gsettings",
        &[
            "set",
            "org.gnome.desktop.screensaver",
            "lock-enabled",
            "false",
        ],
    )?;
    let mut provider = Process(
        Command::new("nddev-systeminfod")
            .stdout(fs::File::create(artifacts.join("provider.jsonl"))?)
            .stderr(fs::File::create(artifacts.join("provider.stderr"))?)
            .spawn()?,
    );
    fs::write(root.join("provider.pid"), provider.0.id().to_string())?;
    command("gjs", &["-m", "extension/tests/smoke.js"])?;
    let result_path = artifacts.join("result.json");
    if result_path.exists() {
        fs::remove_file(&result_path)?;
    }
    let mut shell = Process(
        Command::new("gnome-shell")
            .args([
                "--headless",
                "--wayland",
                "--no-x11",
                "--virtual-monitor",
                "1280x768",
                "--wayland-display",
                "systeminfo-test",
            ])
            .env("GNOME_SHELL_SESSION_MODE", "user")
            .env_remove("WAYLAND_DISPLAY")
            .env("LIBGL_ALWAYS_SOFTWARE", "1")
            .env("SYSTEMINFO_TEST_ARTIFACTS", &artifacts)
            .env("SYSTEMINFO_TEST_ROOT", &root)
            .stdin(Stdio::null())
            .stdout(fs::File::create(artifacts.join("shell.log"))?)
            .stderr(fs::File::create(artifacts.join("shell.stderr"))?)
            .spawn()?,
    );
    let deadline = Instant::now() + Duration::from_secs(90);
    while !result_path.exists() {
        if let Some(status) = shell.0.try_wait()? {
            return Err(format!("headless Shell exited: {status}").into());
        }
        if let Some(status) = provider.0.try_wait()? {
            let expected_stop = fs::read_to_string(artifacts.join("expected-provider-stop"))
                .ok()
                .and_then(|value| value.parse::<u32>().ok());
            if expected_stop != Some(provider.0.id()) {
                return Err(format!("test provider exited: {status}").into());
            }
        }
        if Instant::now() >= deadline {
            return Err("native UI readiness deadline exceeded".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let result: serde_json::Value = serde_json::from_str(&fs::read_to_string(result_path)?)?;
    if result["passed"] != true {
        return Err(format!("native UI failed: {result}").into());
    }
    println!("{result}");
    Ok(())
}
