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
pub mod debian;
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
    #[error("{operation}: {source}")]
    Operation {
        operation: &'static str,
        #[source]
        source: io::Error,
    },
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
    const HEX: &[u8; 16] = b"0123456789abcdef";
    Ok(digest
        .finalize()
        .iter()
        .flat_map(|byte| {
            [
                HEX[(byte >> 4) as usize] as char,
                HEX[(byte & 15) as usize] as char,
            ]
        })
        .collect())
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
    match operation {
        #[cfg(target_os = "windows")]
        "uninstall"
            if arguments.get(1).map(String::as_str) == Some("--windows-maintenance")
                && arguments.len() <= 3 =>
        {
            platform::maintenance(arguments.get(2).map(String::as_str))?;
            Ok(Some(
                serde_json::json!({"removed":true,"configuration_preserved":true}),
            ))
        }
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
            let _lock = lock()?;
            platform::install(&source, &manifest, provider_only)?;
            Ok(Some(
                serde_json::json!({"installed":true,"version":manifest.version}),
            ))
        }
        "uninstall" if arguments.len() == 1 => {
            let _lock = lock()?;
            platform::uninstall()?;
            Ok(Some(if cfg!(target_os = "windows") {
                serde_json::json!({"pending":true,"configuration_preserved":true})
            } else {
                serde_json::json!({"removed":true,"configuration_preserved":true})
            }))
        }
        "service" if arguments.len() == 2 => {
            let _lock = if arguments[1] == "status" {
                None
            } else {
                Some(lock()?)
            };
            Ok(Some(platform::service(&arguments[1])?))
        }
        "uninstall" | "service" => Err(InstallError::Invalid("invalid installation command")),
        _ => Ok(None),
    }
}
fn lock() -> Result<fs::File> {
    let directory = state_dir()?;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(directory.parent().ok_or(InstallError::Invalid(
        "installation state parent unavailable",
    ))?)?;
    builder.create(&directory)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join("operation.lock"))?;
    file.try_lock().map_err(|_| {
        InstallError::Invalid("another installation or service operation is running")
    })?;
    Ok(file)
}
fn exe(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}
#[cfg(unix)]
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
#[cfg(unix)]
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

#[cfg(test)]
mod tests {
    use super::*;
    struct Release(PathBuf);
    impl Release {
        fn new() -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "si-release-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            fs::create_dir_all(root.join("bin")).unwrap();
            for name in ["nddev-systeminfo", "nddev-systeminfod"] {
                fs::write(root.join("bin").join(exe(name)), b"abc").unwrap();
            }
            Self(root)
        }
        fn manifest(&self) -> Manifest {
            Manifest {
                format: 1,
                version: env!("CARGO_PKG_VERSION").into(),
                os: std::env::consts::OS.into(),
                architecture: std::env::consts::ARCH.into(),
                files: ["nddev-systeminfo", "nddev-systeminfod"]
                    .into_iter()
                    .map(|name| File {
                        path: Path::new("bin").join(exe(name)),
                        sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
                            .into(),
                        executable: true,
                    })
                    .collect(),
            }
        }
    }
    impl Drop for Release {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn release_integrity_rejects_changed_extra_and_escaped_files() {
        let release = Release::new();
        let mut manifest = release.manifest();
        manifest.verify(&release.0).unwrap();
        fs::write(release.0.join("unexpected"), b"extra").unwrap();
        assert!(manifest.verify(&release.0).is_err());
        fs::remove_file(release.0.join("unexpected")).unwrap();
        fs::write(release.0.join(&manifest.files[0].path), b"corrupt").unwrap();
        assert!(manifest.verify(&release.0).is_err());
        manifest.files[0].path = Path::new("..").join("outside");
        assert!(manifest.verify(&release.0).is_err());
    }
    #[test]
    fn atomic_install_writes_do_not_follow_a_redirected_destination() {
        let release = Release::new();
        let output = release.0.join("installed");
        atomic(&output, b"old", false).unwrap();
        atomic(&output, b"new", false).unwrap();
        assert_eq!(fs::read(&output).unwrap(), b"new");
        #[cfg(unix)]
        {
            let link = release.0.join("redirected");
            std::os::unix::fs::symlink(&output, &link).unwrap();
            assert!(atomic(&link, b"overwrite", false).is_err());
            assert_eq!(fs::read(&output).unwrap(), b"new");
        }
        assert!(!fs::read_dir(&release.0).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".systeminfo-install-")
        }));
    }
}
