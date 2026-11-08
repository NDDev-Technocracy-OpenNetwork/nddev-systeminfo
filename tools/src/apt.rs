//! Signed APT publication from original qualified Debian bytes.
use crate::{Result, command, files};
use serde::Deserialize;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Repository {
    origin: String,
    label: String,
    suite: String,
    architecture: String,
    component: String,
    package: String,
    transition: String,
    key: String,
}
fn configuration() -> Result<Repository> {
    Ok(toml::from_str(include_str!(
        "../../packaging/apt/repository.toml"
    ))?)
}
fn status(mut command: Command) -> Result {
    let status = command.status()?;
    if !status.success() {
        return Err(format!("APT native operation failed: {status}").into());
    }
    Ok(())
}
fn output(mut command: Command, path: &Path) -> Result {
    command.stdout(fs::File::create(path)?);
    status(command)
}
struct Keyring(PathBuf);
impl Drop for Keyring {
    fn drop(&mut self) {
        let _ = Command::new("gpgconf")
            .arg("--homedir")
            .arg(&self.0)
            .args(["--kill", "gpg-agent"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::remove_dir_all(&self.0);
    }
}
pub(super) fn build() -> Result {
    build_at(
        Path::new("site/apt"),
        Path::new("released"),
        &Path::new("packaging/apt").join(configuration()?.key),
        std::env::var("APT_SIGNING_KEY")?.as_bytes(),
    )
}
fn build_at(root: &Path, packages: &Path, public_key: &Path, secret: &[u8]) -> Result {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let config = configuration()?;
    let pool = root.join(format!("pool/main/n/{}", config.package));
    let stable = root.join("dists").join(&config.suite);
    let indexes = stable
        .join(&config.component)
        .join(format!("binary-{}", config.architecture));
    fs::create_dir_all(&pool)?;
    fs::create_dir_all(&indexes)?;
    let packages: Vec<_> = files(packages)?
        .into_iter()
        .filter(|file| file.extension().is_some_and(|extension| extension == "deb"))
        .collect();
    if packages.len() != 2 {
        return Err("APT distribution requires both qualified packages".into());
    }
    for file in packages {
        fs::copy(
            &file,
            pool.join(file.file_name().ok_or("Debian filename missing")?),
        )?;
    }
    let mut scan = Command::new("dpkg-scanpackages");
    scan.current_dir(root)
        .args(["--arch", &config.architecture, "pool"]);
    output(scan, &indexes.join("Packages"))?;
    let mut gzip = Command::new("gzip");
    gzip.args(["-9", "-c"]).arg(indexes.join("Packages"));
    output(gzip, &indexes.join("Packages.gz"))?;
    let mut archive = Command::new("apt-ftparchive");
    archive.current_dir(root);
    for (key, value) in [
        ("Origin", &config.origin),
        ("Label", &config.label),
        ("Suite", &config.suite),
        ("Codename", &config.suite),
        ("Architectures", &config.architecture),
        ("Components", &config.component),
    ] {
        archive
            .arg("-o")
            .arg(format!("APT::FTPArchive::Release::{key}={value}"));
    }
    archive
        .arg("release")
        .arg(format!("dists/{}", config.suite));
    output(archive, &stable.join("Release"))?;
    let keyring =
        Keyring(std::env::current_dir()?.join(format!(".tools/apt-key-{}", std::process::id())));
    fs::DirBuilder::new().mode(0o700).create(&keyring.0)?;
    let gpg = || {
        let mut command = Command::new("gpg");
        command
            .env("GNUPGHOME", &keyring.0)
            .args(["--batch", "--yes"]);
        command
    };
    let mut import = gpg()
        .arg("--import")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()?;
    import
        .stdin
        .take()
        .ok_or("signing input missing")?
        .write_all(secret)?;
    if !import.wait()?.success() {
        return Err("APT signing key import failed".into());
    }
    for (kind, name) in [
        ("--detach-sign", "Release.gpg"),
        ("--clearsign", "InRelease"),
    ] {
        let mut sign = gpg();
        sign.arg("--armor")
            .arg(kind)
            .arg("--output")
            .arg(stable.join(name))
            .arg(stable.join("Release"));
        status(sign)?;
    }
    fs::copy(public_key, root.join(&config.key))?;
    // Only public repository data enters Pages. The sandboxed _apt downloader
    // must be able to traverse and read both plain and compressed indexes.
    fn permissions(path: &Path) -> Result {
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if path.is_dir() { 0o755 } else { 0o644 }),
        )?;
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                permissions(&entry?.path())?;
            }
        }
        Ok(())
    }
    permissions(root)?;
    Ok(())
}
pub(super) fn test() -> Result {
    test_at(
        Path::new("site/apt"),
        &Path::new("packaging/apt").join(configuration()?.key),
    )
}
fn disposable() -> Result {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true")
        || std::env::var_os("RUNNER_TEMP").is_none()
    {
        return Err("APT installation acceptance requires a disposable runner".into());
    }
    Ok(())
}
fn test_at(root: &Path, public_key: &Path) -> Result {
    disposable()?;
    let config = configuration()?;
    command(
        "sudo",
        &[
            "install",
            "-m644",
            public_key.to_str().ok_or("public key path must be UTF-8")?,
            &format!("/usr/share/keyrings/{}", config.key),
        ],
    )?;
    let source = Path::new(".tools/apt-source.list");
    fs::write(
        source,
        format!(
            "deb [arch={} signed-by=/usr/share/keyrings/{}] file:{} {} {}\n",
            config.architecture,
            config.key,
            fs::canonicalize(root)?.display(),
            config.suite,
            config.component
        ),
    )?;
    command(
        "sudo",
        &[
            "install",
            "-m644",
            source.to_str().unwrap(),
            "/etc/apt/sources.list.d/rldyour-sysinfo.list",
        ],
    )?;
    command("sudo", &["apt-get", "update"])?;
    command(
        "sudo",
        &[
            "apt-get",
            "install",
            "-y",
            &config.package,
            &config.transition,
        ],
    )?;
    command("/usr/bin/nddev-systeminfod", &["--version"])?;
    command("/usr/bin/nddev-systeminfo", &["--version"])?;
    let unit = fs::read_to_string("/usr/lib/systemd/user/rldyour-sysinfod.service")?;
    if !unit
        .lines()
        .any(|line| line == "ExecStart=/usr/bin/nddev-systeminfod")
    {
        return Err("APT provider unit points outside its package".into());
    }
    command(
        "systemctl",
        &["--global", "is-enabled", "rldyour-sysinfod.socket"],
    )?;
    Ok(())
}

/// Exercise actual signing and APT installation before a release has access
/// to the real signing key. This throwaway identity never enters a distribution.
pub(super) fn fixture() -> Result {
    use std::os::unix::fs::DirBuilderExt;
    disposable()?;
    let directory =
        std::env::current_dir()?.join(format!(".tools/apt-fixture-{}", std::process::id()));
    fs::create_dir(&directory)?;
    let keys = Keyring(directory.join("keys"));
    fs::DirBuilder::new().mode(0o700).create(&keys.0)?;
    let gpg = || {
        let mut command = Command::new("gpg");
        command.env("GNUPGHOME", &keys.0).args([
            "--batch",
            "--yes",
            "--pinentry-mode",
            "loopback",
            "--passphrase",
            "",
        ]);
        command
    };
    let mut generate = gpg();
    generate.args([
        "--quick-generate-key",
        "NDDev Systeminfo acceptance",
        "ed25519",
        "sign",
        "1d",
    ]);
    status(generate)?;
    let public = directory.join("fixture.asc");
    let mut export = gpg();
    export.args(["--armor", "--export"]);
    output(export, &public)?;
    let secret = gpg().args(["--armor", "--export-secret-keys"]).output()?;
    if !secret.status.success() {
        return Err("throwaway APT signer could not be exported".into());
    }
    let repository = directory.join("apt");
    build_at(&repository, Path::new("dist"), &public, &secret.stdout)?;
    test_at(&repository, &public)?;
    drop(keys);
    fs::remove_dir_all(directory)?;
    println!("PASS: real throwaway GPG signing, authenticated APT indexes and installed packages");
    Ok(())
}
