use super::*;
use nddev_systeminfo_native::tray;
const RUN: &str = "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const PROVIDER: &str = "rldyour-sysinfod";
const UI: &str = "nddev-systeminfo";
fn root() -> Result<PathBuf> {
    Ok(crate::config::default_path()?
        .parent()
        .ok_or(InstallError::Invalid("local application data unavailable"))?
        .to_path_buf())
}
fn current() -> Result<PathBuf> {
    let path = state_dir()?.join("current.txt");
    let version = fs::read_to_string(path)?;
    if version.trim().is_empty()
        || !version
            .trim()
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Err(InstallError::Invalid("invalid installed version"));
    }
    Ok(root()?.join("versions").join(version.trim()))
}
fn enabled(name: &str) -> Result<bool> {
    Ok(run(
        "reg.exe",
        &["query".as_ref(), RUN.as_ref(), "/v".as_ref(), name.as_ref()],
        false,
    )?
    .status
    .success())
}
fn autorun(name: &str, executable: Option<&Path>) -> Result {
    if let Some(path) = executable {
        let command = format!("\"{}\"", path.display());
        run(
            "reg.exe",
            &[
                "add".as_ref(),
                RUN.as_ref(),
                "/v".as_ref(),
                name.as_ref(),
                "/t".as_ref(),
                "REG_SZ".as_ref(),
                "/d".as_ref(),
                command.as_ref(),
                "/f".as_ref(),
            ],
            true,
        )?;
    } else if enabled(name)? {
        run(
            "reg.exe",
            &[
                "delete".as_ref(),
                RUN.as_ref(),
                "/v".as_ref(),
                name.as_ref(),
                "/f".as_ref(),
            ],
            true,
        )?;
    }
    Ok(())
}
fn spawn(path: &Path) -> Result {
    use std::os::windows::process::CommandExt;
    let logs = root()?.join("logs");
    fs::create_dir_all(&logs)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(logs.join(format!(
            "{}.jsonl",
            path.file_stem().unwrap().to_string_lossy()
        )))?;
    Command::new(path)
        .creation_flags(0x08000000)
        .stdin(Stdio::null())
        .stdout(file.try_clone()?)
        .stderr(file)
        .spawn()?;
    Ok(())
}
fn binaries(directory: &Path) -> [PathBuf; 2] {
    [
        directory.join("bin/nddev-systeminfod.exe"),
        directory.join("bin/nddev-systeminfo-tray.exe"),
    ]
}
pub(super) fn install(source: &Path, manifest: &Manifest, provider_only: bool) -> Result {
    let target = root()?.join("versions").join(&manifest.version);
    let old = current().ok();
    let legacy = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or(InstallError::Invalid("local application data unavailable"))?
        .join("rldyour-sysinfo/rldyour-sysinfod.exe");
    let existed = old.is_some() || legacy.exists();
    let provider_enabled = enabled(PROVIDER)?;
    let ui_enabled = enabled(UI)?;
    let previous = old
        .as_ref()
        .map(|directory| binaries(directory).to_vec())
        .unwrap_or_else(|| vec![legacy.clone()]);
    let active = tray::running_products(&previous);
    if target.exists() {
        let installed: Manifest =
            toml::from_str(&fs::read_to_string(target.join("manifest.toml"))?)?;
        installed.verify(&target)?;
        if toml::to_string(&installed)? != toml::to_string(manifest)? {
            return Err(InstallError::Invalid(
                "installed immutable version differs from this release",
            ));
        }
    } else {
        let staged = target.with_file_name(format!(".install-{}", std::process::id()));
        if staged.exists() {
            return Err(InstallError::Invalid(
                "an incomplete release stage needs recovery",
            ));
        }
        copy_tree(source, &staged)?;
        fs::rename(staged, &target)?;
    }
    tray::stop_products(&previous)?;
    if !existed || provider_enabled {
        autorun(PROVIDER, Some(&binaries(&target)[0]))?;
    }
    if !provider_only && (old.is_none() || ui_enabled) {
        autorun(UI, Some(&binaries(&target)[1]))?;
    }
    atomic(
        &state_dir()?.join("current.txt"),
        manifest.version.as_bytes(),
        false,
    )?;
    receipt(manifest)?;
    // Preserve StartupApproved and every unrelated Run value. Upgrades resume
    // only a process that was running; explicit disable/stop remains stopped.
    if !existed || active.contains(&previous[0]) {
        spawn(&binaries(&target)[0])?;
    }
    if !provider_only
        && (old.is_none() || previous.get(1).is_some_and(|path| active.contains(path)))
    {
        spawn(&binaries(&target)[1])?;
    }
    if legacy.exists() {
        remove_file(&legacy)?;
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let shim = PathBuf::from(appdata)
            .join("Microsoft/Windows/Start Menu/Programs/Startup/rldyour-sysinfod.cmd");
        if fs::read_to_string(&shim).is_ok_and(|text| text.contains("rldyour-sysinfod.exe")) {
            remove_file(&shim)?;
        }
    }
    Ok(())
}
pub(super) fn service(operation: &str) -> Result<serde_json::Value> {
    let installed = current();
    if installed.is_err() && operation == "status" {
        return Ok(serde_json::json!({"managed":false,"enabled":false,"active":false}));
    }
    if installed.is_err() && operation == "apply" {
        return Ok(serde_json::json!({"applied":false}));
    }
    let provider = binaries(&installed?)[0].clone();
    let running = !tray::running_products(std::slice::from_ref(&provider)).is_empty();
    match operation {
        "status" => {
            Ok(serde_json::json!({"managed":true,"active":running,"enabled":enabled(PROVIDER)?}))
        }
        "start" | "enable" => {
            if operation == "enable" {
                autorun(PROVIDER, Some(&provider))?;
            }
            if !running {
                spawn(&provider)?;
            }
            Ok(serde_json::json!({"active":true}))
        }
        "stop" | "disable" => {
            if operation == "disable" {
                autorun(PROVIDER, None)?;
            }
            tray::stop_products(&[provider])?;
            Ok(serde_json::json!({"active":false}))
        }
        "restart" | "apply" => {
            if running || operation == "restart" {
                tray::stop_products(std::slice::from_ref(&provider))?;
                spawn(&provider)?;
            }
            Ok(serde_json::json!({"applied":running || operation == "restart"}))
        }
        _ => Err(InstallError::Invalid("unknown service operation")),
    }
}
pub(super) fn uninstall() -> Result {
    if !state_dir()?.join("manifest.toml").exists() {
        return Err(InstallError::Invalid("no owned user installation receipt"));
    }
    let directory = current()?;
    tray::stop_products(&binaries(&directory))?;
    autorun(PROVIDER, None)?;
    autorun(UI, None)?;
    // The portable CLI outside the installation can remove every version. The
    // installed CLI runs from a version directory; Windows removal semantics are
    // qualified on the real OS before this path is shipped.
    fs::remove_dir_all(root()?.join("versions"))?;
    remove_file(&state_dir()?.join("current.txt"))?;
    remove_file(&state_dir()?.join("manifest.toml"))?;
    Ok(())
}
