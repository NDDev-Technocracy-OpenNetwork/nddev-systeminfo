//! User-scoped installation and native service control. Release manifests own
//! the file set; OS managers own enabled/disabled state; TOML stays untouched.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
    process::{Command, Output, Stdio},
    thread,
    time::{Duration, Instant},
};
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(target_os = "windows")]
use windows as platform;

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("installation I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid installation manifest")]
    Manifest(#[from] toml::de::Error),
    #[error("cannot encode installation receipt")]
    Serialize(#[from] toml::ser::Error),
    #[error("{0}")]
    Invalid(&'static str),
    #[error("native service command failed: {0}")]
    Manager(String),
}
type Result<T = ()> = std::result::Result<T, InstallError>;
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub format: u32,
    pub version: String,
    pub os: String,
    pub architecture: String,
    pub files: Vec<File>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct File {
    pub path: PathBuf,
    pub sha256: String,
    pub executable: bool,
}

pub fn digest(path: &Path) -> io::Result<String> {
    let mut source = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = source.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
impl Manifest {
    pub fn verify(&self, root: &Path) -> Result {
        if self.format != 1
            || self.version != env!("CARGO_PKG_VERSION")
            || self.os != std::env::consts::OS
            || self.architecture != std::env::consts::ARCH
        {
            return Err(InstallError::Invalid(
                "release does not match this installer or platform",
            ));
        }
        let root = fs::canonicalize(root)?;
        let mut seen = std::collections::BTreeSet::new();
        for file in &self.files {
            if file.path.as_os_str().is_empty()
                || !file
                    .path
                    .components()
                    .all(|part| matches!(part, Component::Normal(_)))
                || !seen.insert(&file.path)
            {
                return Err(InstallError::Invalid("invalid or repeated release path"));
            }
            let path = root.join(&file.path);
            if fs::symlink_metadata(&path)?.file_type().is_symlink()
                || !fs::canonicalize(&path)?.starts_with(&root)
                || digest(&path)? != file.sha256
            {
                return Err(InstallError::Invalid(
                    "release file failed integrity verification",
                ));
            }
        }
        fn actual(
            root: &Path,
            directory: &Path,
            found: &mut std::collections::BTreeSet<PathBuf>,
        ) -> Result {
            for entry in fs::read_dir(directory)? {
                let entry = entry?;
                if entry.file_type()?.is_symlink() {
                    return Err(InstallError::Invalid("release contains a redirected path"));
                }
                if entry.file_type()?.is_dir() {
                    actual(root, &entry.path(), found)?;
                } else {
                    let path = entry
                        .path()
                        .strip_prefix(root)
                        .map_err(|_| InstallError::Invalid("release path escaped root"))?
                        .to_path_buf();
                    if path != Path::new("manifest.toml") {
                        found.insert(path);
                    }
                }
            }
            Ok(())
        }
        let mut files = std::collections::BTreeSet::new();
        actual(&root, &root, &mut files)?;
        if files != seen.into_iter().cloned().collect() {
            return Err(InstallError::Invalid(
                "release file set differs from its manifest",
            ));
        }
        for required in ["nddev-systeminfo", "nddev-systeminfod"] {
            if !self
                .files
                .iter()
                .any(|file| file.path == Path::new("bin").join(exe(required)))
            {
                return Err(InstallError::Invalid("release lacks a required executable"));
            }
        }
        Ok(())
    }
}
pub fn command(arguments: &[String]) -> Result<Option<serde_json::Value>> {
    let Some(operation) = arguments.first().map(String::as_str) else {
        return Ok(None);
    };
    if !["install", "uninstall", "service"].contains(&operation) {
        return Ok(None);
    }
    let directory = state_dir()?;
    fs::create_dir_all(&directory)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("operation.lock"))?;
    lock.try_lock().map_err(|_| {
        InstallError::Invalid("another installation or service operation is running")
    })?;
    match operation {
        "install" => {
            let mut source = std::env::current_exe()?
                .parent()
                .and_then(Path::parent)
                .ok_or(InstallError::Invalid("release root missing"))?
                .to_path_buf();
            let mut provider_only = false;
            let mut rest = arguments.iter().skip(1);
            while let Some(argument) = rest.next() {
                match argument.as_str() {
                    "--source" => {
                        source = rest
                            .next()
                            .ok_or(InstallError::Invalid(
                                "--source needs the verified release directory",
                            ))?
                            .into()
                    }
                    "--provider-only" => provider_only = true,
                    _ => return Err(InstallError::Invalid("unknown installation option")),
                }
            }
            let manifest: Manifest =
                toml::from_str(&fs::read_to_string(source.join("manifest.toml"))?)?;
            manifest.verify(&source)?;
            platform::install(&source, &manifest, provider_only)?;
            Ok(Some(
                serde_json::json!({"installed":true,"version":manifest.version}),
            ))
        }
        "uninstall" if arguments.len() == 1 => {
            platform::uninstall()?;
            Ok(Some(
                serde_json::json!({"removed":true,"configuration_preserved":true}),
            ))
        }
        "service" if arguments.len() == 2 => Ok(Some(platform::service(&arguments[1])?)),
        "uninstall" | "service" => Err(InstallError::Invalid("invalid installation command")),
        _ => Ok(None),
    }
}
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
fn home() -> Result<PathBuf> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or(InstallError::Invalid("absolute user home unavailable"))?;
    Ok(home)
}
fn state_dir() -> Result<PathBuf> {
    Ok(crate::config::default_path()?
        .parent()
        .ok_or(InstallError::Invalid("configuration parent unavailable"))?
        .join("installation"))
}
fn atomic(path: &Path, bytes: &[u8], executable: bool) -> Result {
    let parent = path
        .parent()
        .ok_or(InstallError::Invalid("destination parent missing"))?;
    fs::create_dir_all(parent)?;
    if fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
        return Err(InstallError::Invalid(
            "installation destination is a redirected file",
        ));
    }
    let temporary = parent.join(format!(
        ".systeminfo-install-{}-{}.tmp",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(if executable { 0o755 } else { 0o644 });
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    })();
    let _ = fs::remove_file(temporary);
    #[cfg(not(unix))]
    let _ = executable;
    result
}
fn copy_file(source: &Path, destination: &Path, executable: bool) -> Result {
    atomic(destination, &fs::read(source)?, executable)
}
fn copy_tree(source: &Path, destination: &Path) -> Result {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        if entry.file_type()?.is_symlink() {
            return Err(InstallError::Invalid("release directory contains a link"));
        }
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &destination.join(entry.file_name()))?;
        } else {
            fs::copy(entry.path(), destination.join(entry.file_name()))?;
        }
    }
    Ok(())
}
fn run(program: &str, arguments: &[&std::ffi::OsStr], required: bool) -> Result<Output> {
    let mut command = Command::new(program);
    command
        .args(arguments)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(InstallError::Manager(format!("{program} timed out")));
        }
        thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output()?;
    if required && !output.status.success() {
        return Err(InstallError::Manager(format!(
            "{program} exited {}",
            output.status
        )));
    }
    Ok(output)
}
fn receipt(manifest: &Manifest) -> Result {
    atomic(
        &state_dir()?.join("manifest.toml"),
        toml::to_string_pretty(manifest)?.as_bytes(),
        false,
    )
}
fn remove_file(path: &Path) -> Result {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}
