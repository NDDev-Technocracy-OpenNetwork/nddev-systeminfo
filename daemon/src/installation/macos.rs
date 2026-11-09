use super::*;
const PROVIDER: &str = "com.nddev-opennetwork.rldyour-sysinfod";
const UI: &str = "com.nddev-opennetwork.rldyour-sysinfo";
fn domain() -> Result<String> {
    let output = run("id", &["-u".as_ref()], true)?;
    let uid: u32 = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .map_err(|_| InstallError::Invalid("user ID unavailable"))?;
    Ok(format!("gui/{uid}"))
}
fn ctl(arguments: &[&str], required: bool) -> Result<Output> {
    run(
        "launchctl",
        &arguments
            .iter()
            .map(std::ffi::OsStr::new)
            .collect::<Vec<_>>(),
        required,
    )
}
fn loaded(domain: &str, name: &str) -> Result<bool> {
    Ok(ctl(&["print", &format!("{domain}/{name}")], false)?
        .status
        .success())
}
fn disabled(domain: &str, name: &str) -> Result<bool> {
    let output = ctl(&["print-disabled", domain], true)?;
    disabled_state(&String::from_utf8_lossy(&output.stdout), name)
}
fn disabled_state(document: &str, name: &str) -> Result<bool> {
    for line in document.lines() {
        if let Some((key, value)) = line.trim().split_once("=>")
            && key.trim() == format!("\"{name}\"")
        {
            return match value.trim().trim_end_matches([',', ';']) {
                "disabled" | "true" => Ok(true),
                "enabled" | "false" => Ok(false),
                _ => Err(InstallError::Invalid(
                    "unrecognized native service enablement state",
                )),
            };
        }
    }
    Ok(false)
}
fn agent(name: &str) -> Result<PathBuf> {
    Ok(home()?
        .join("Library/LaunchAgents")
        .join(format!("{name}.plist")))
}
fn bootstrap(domain: &str, name: &str) -> Result {
    let path = agent(name)?;
    let path = path
        .to_str()
        .ok_or(InstallError::Invalid("agent path is not Unicode"))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut delay = Duration::from_millis(25);
    loop {
        let output = ctl(&["bootstrap", domain, path], false)?;
        if output.status.success() {
            return Ok(());
        }
        // bootout can return before launchd has removed the old registration.
        // EIO is generic: retry only this bounded transition, never unload an
        // unexpected job, change enablement or start an unmanaged substitute.
        if output.status.code() != Some(5) || Instant::now() >= deadline {
            return Err(InstallError::Manager(format!(
                "launchctl bootstrap {name} exited {}",
                output.status
            )));
        }
        thread::sleep(delay.min(deadline.saturating_duration_since(Instant::now())));
        delay = (delay * 2).min(Duration::from_millis(500));
    }
}
fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn owned_bundle(path: &Path) -> Result {
    if path.is_symlink() {
        return Err(InstallError::Invalid(
            "application destination is redirected",
        ));
    }
    if path.exists() {
        let output = run(
            "/usr/libexec/PlistBuddy",
            &[
                "-c".as_ref(),
                "Print :CFBundleIdentifier".as_ref(),
                path.join("Contents/Info.plist").as_os_str(),
            ],
            true,
        )?;
        if String::from_utf8_lossy(&output.stdout).trim() != UI {
            return Err(InstallError::Invalid(
                "destination contains a different application",
            ));
        }
    }
    Ok(())
}
pub(super) fn install(source: &Path, manifest: &Manifest, provider_only: bool) -> Result {
    let home = home()?;
    let domain = domain()?;
    let bin = home.join(".local/bin");
    let app = home.join("Applications/NDDev Systeminfo.app");
    let legacy = home.join("Applications/rldyour-sysinfo.app");
    let mut previous = Vec::new();
    for name in if provider_only {
        vec![PROVIDER]
    } else {
        vec![PROVIDER, UI]
    } {
        let exists = agent(name)?.exists();
        previous.push((
            name,
            exists,
            loaded(&domain, name)?,
            disabled(&domain, name)?,
        ));
    }
    let staged = app.with_file_name(format!(".systeminfo-app-{}", std::process::id()));
    if !provider_only {
        owned_bundle(&app)?;
        owned_bundle(&legacy)?;
        copy_tree(&source.join("ui/NDDev Systeminfo.app"), &staged)?;
        run(
            "codesign",
            &[
                "--verify".as_ref(),
                "--deep".as_ref(),
                "--strict".as_ref(),
                staged.as_os_str(),
            ],
            true,
        )?;
    }
    for (name, _, was_loaded, _) in &previous {
        if *was_loaded {
            ctl(&["bootout", &format!("{domain}/{name}")], true)?;
        }
    }
    for name in ["nddev-systeminfo", "nddev-systeminfod"] {
        copy_file(&source.join("bin").join(name), &bin.join(name), true)?;
    }
    if !provider_only {
        let backup = app.with_file_name(format!(".systeminfo-previous-{}", std::process::id()));
        if app.exists() {
            fs::rename(&app, &backup)?;
        }
        if let Err(error) = fs::rename(&staged, &app) {
            if backup.exists() {
                let _ = fs::rename(&backup, &app);
            }
            return Err(error.into());
        }
        if backup.exists() {
            fs::remove_dir_all(backup)?;
        }
        if legacy.exists() {
            // The previous installer sometimes started the app with `open`.
            // Stop only its verified bundle's exact executable command line.
            let executable = legacy
                .join("Contents/MacOS/rldyour-sysinfo")
                .to_string_lossy()
                .into_owned();
            let escaped: String = executable
                .chars()
                .flat_map(|c| {
                    if ".^$*+?()[]{}|\\".contains(c) {
                        vec!['\\', c]
                    } else {
                        vec![c]
                    }
                })
                .collect();
            run(
                "pkill",
                &[
                    "-TERM".as_ref(),
                    "-f".as_ref(),
                    format!("^{escaped}( |$)").as_ref(),
                ],
                false,
            )?;
            fs::remove_dir_all(legacy)?;
        }
    }
    let mut directory = fs::DirBuilder::new();
    use std::os::unix::fs::DirBuilderExt;
    directory
        .recursive(true)
        .mode(0o700)
        .create(home.join("Library/Application Support/rldyour-sysinfo"))?;
    fs::create_dir_all(home.join("Library/Logs"))?;
    for (name, existed, was_loaded, was_disabled) in previous {
        let template = if name == PROVIDER {
            include_str!("../../../macos/com.nddev-opennetwork.rldyour-sysinfod.plist")
        } else {
            include_str!("../../../macos/com.nddev-opennetwork.rldyour-sysinfo.plist")
        };
        let text = template.replace(
            "@HOME@",
            &xml(home
                .to_str()
                .ok_or(InstallError::Invalid("home path is not Unicode"))?),
        );
        atomic(&agent(name)?, text.as_bytes(), false)?;
        run(
            "plutil",
            &["-lint".as_ref(), agent(name)?.as_os_str()],
            true,
        )?;
        if !was_disabled && (!existed || was_loaded) {
            bootstrap(&domain, name)?;
        }
    }
    receipt(manifest)?;
    remove_file(&bin.join("rldyour-sysinfod"))?;
    Ok(())
}
pub(super) fn service(operation: &str) -> Result<serde_json::Value> {
    let domain = domain()?;
    let job = format!("{domain}/{PROVIDER}");
    match operation {
        "status" => Ok(
            serde_json::json!({"enabled":!disabled(&domain, PROVIDER)?,"loaded":loaded(&domain, PROVIDER)?}),
        ),
        "apply" => {
            let output = ctl(&["print", &job], false)?;
            let active = output.status.success()
                && String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .any(|line| line.trim() == "state = running");
            if active {
                ctl(&["kickstart", "-k", &job], true)?;
            }
            Ok(serde_json::json!({"applied":active}))
        }
        "enable" | "start" => {
            if operation == "enable" {
                ctl(&["enable", &job], true)?;
            }
            if !loaded(&domain, PROVIDER)? {
                bootstrap(&domain, PROVIDER)?;
            }
            Ok(serde_json::json!({"loaded":true}))
        }
        "disable" | "stop" => {
            if loaded(&domain, PROVIDER)? {
                ctl(&["bootout", &job], true)?;
            }
            if operation == "disable" {
                ctl(&["disable", &job], true)?;
            }
            Ok(serde_json::json!({"loaded":false}))
        }
        "restart" => {
            ctl(&["kickstart", "-k", &job], true)?;
            Ok(serde_json::json!({"restarted":true}))
        }
        _ => Err(InstallError::Invalid("unknown service operation")),
    }
}
pub(super) fn uninstall() -> Result {
    if !state_dir()?.join("manifest.toml").exists() {
        return Err(InstallError::Invalid("no owned user installation receipt"));
    }
    let domain = domain()?;
    for name in [UI, PROVIDER] {
        if loaded(&domain, name)? {
            ctl(&["bootout", &format!("{domain}/{name}")], true)?;
        }
        remove_file(&agent(name)?)?;
    }
    let app = home()?.join("Applications/NDDev Systeminfo.app");
    owned_bundle(&app)?;
    if app.exists() {
        fs::remove_dir_all(app)?;
    }
    for name in ["nddev-systeminfo", "nddev-systeminfod"] {
        remove_file(&home()?.join(".local/bin").join(name))?;
    }
    remove_file(&state_dir()?.join("manifest.toml"))?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launchd_disabled_overrides_accept_both_native_output_formats() {
        for value in ["disabled", "true"] {
            assert!(
                disabled_state(
                    &format!("disabled services = {{\n\t\"{PROVIDER}\" => {value}\n}}"),
                    PROVIDER
                )
                .unwrap()
            );
        }
        for value in ["enabled", "false"] {
            assert!(!disabled_state(&format!("\"{PROVIDER}\" => {value}"), PROVIDER).unwrap());
        }
        assert!(!disabled_state("disabled services = {}", PROVIDER).unwrap());
        assert!(disabled_state(&format!("\"{PROVIDER}\" => unknown"), PROVIDER).is_err());
    }
}
