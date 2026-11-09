//! Select the pinned compiler on build runners without replacing their SDK.
//! Only development tooling calls this; product installation never downloads it.
use super::{Result, command, digest};
use serde::Deserialize;
use std::{fs, io::Write, path::Path, process::Command};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Compiler {
    version: String,
    url: String,
    sha256: String,
    signer: String,
}
fn compiler() -> Result<Compiler> {
    let spec: Compiler = toml::from_str(include_str!("../swift.toml"))?;
    if spec.version.split('.').count() != 3
        || spec.version.split('.').any(str::is_empty)
        || !spec
            .version
            .bytes()
            .all(|v| v.is_ascii_digit() || v == b'.')
        || spec.sha256.len() != 64
        || !spec.sha256.bytes().all(|v| v.is_ascii_hexdigit())
        || spec.signer.is_empty()
        || spec.signer.contains(['\n', '\r'])
        || spec.url
            != format!(
                "https://download.swift.org/swift-{}-release/xcode/swift-{}-RELEASE/swift-{}-RELEASE-osx.pkg",
                spec.version, spec.version, spec.version
            )
    {
        return Err("invalid native Swift distribution declaration".into());
    }
    Ok(spec)
}
pub(super) fn check_manifest() -> Result {
    compiler().map(|_| ())
}
fn verify(program: &Path, version: &str) -> Result {
    let output = Command::new(program).arg("--version").output()?;
    let text = String::from_utf8(output.stdout)?;
    let matches = text.lines().any(|line| {
        line.strip_prefix("Apple Swift version ")
            .or_else(|| line.strip_prefix("Swift version "))
            .and_then(|line| line.split_whitespace().next())
            == Some(version)
    });
    if !output.status.success() || !matches {
        return Err(format!("select the declared native Swift {version} before building").into());
    }
    print!("{text}");
    Ok(())
}
pub(super) fn verify_current() -> Result {
    verify(Path::new("swiftc"), &compiler()?.version)
}
pub(super) fn verify_binary(path: &Path) -> Result {
    for flag in ["-L", "-l"] {
        let output = Command::new("otool").arg(flag).arg(path).output()?;
        if !output.status.success()
            || String::from_utf8_lossy(&output.stdout).contains(".xctoolchain")
        {
            return Err(
                "native product must use system libraries, without a compiler-toolchain rpath"
                    .into(),
            );
        }
    }
    Ok(())
}
pub(super) fn setup() -> Result {
    if std::env::consts::OS != "macos" {
        return Err("native Swift setup runs only on a Mac build host".into());
    }
    let spec = compiler()?;
    let home = std::env::var_os("HOME").ok_or("build user's home missing")?;
    if !Path::new(&home).is_absolute() {
        return Err("absolute build user's home required".into());
    }
    let bundle = Path::new(&home)
        .join("Library/Developer/Toolchains")
        .join(format!("swift-{}-RELEASE.xctoolchain", spec.version));
    let bin = bundle.join("usr/bin");
    let swiftc = bin.join("swiftc");
    if !swiftc.is_file() {
        struct Staging(std::path::PathBuf);
        impl Drop for Staging {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
        let folder = std::env::temp_dir().join(format!(
            "systeminfo-swift-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)?
                .as_nanos()
        ));
        fs::create_dir(&folder)?;
        let staging = Staging(folder);
        let package = staging.0.join("swift.pkg");
        command(
            "curl",
            &[
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--proto",
                "=https",
                "--connect-timeout",
                "10",
                "--max-time",
                "600",
                "--output",
                package.to_str().ok_or("package path is not Unicode")?,
                &spec.url,
            ],
        )?;
        if digest(&package)? != spec.sha256 {
            return Err("native Swift package checksum mismatch".into());
        }
        let signature = Command::new("pkgutil")
            .args(["--check-signature"])
            .arg(&package)
            .env("LC_ALL", "C")
            .output()?;
        if !signature.status.success()
            || !String::from_utf8_lossy(&signature.stdout).contains(&spec.signer)
        {
            return Err("native Swift package is not trusted from the declared publisher".into());
        }
        command(
            "installer",
            &[
                "-target",
                "CurrentUserHomeDirectory",
                "-pkg",
                package.to_str().ok_or("package path is not Unicode")?,
            ],
        )?;
    }
    verify(&swiftc, &spec.version)?;
    let output = Command::new("plutil")
        .args(["-extract", "CFBundleIdentifier", "raw"])
        .arg(bundle.join("Info.plist"))
        .output()?;
    let identifier = String::from_utf8(output.stdout)?;
    let identifier = identifier.trim();
    if !output.status.success()
        || identifier.is_empty()
        || !identifier
            .bytes()
            .all(|v| v.is_ascii_alphanumeric() || matches!(v, b'.' | b'-'))
    {
        return Err("invalid native Swift toolchain identity".into());
    }
    for (name, line) in [
        ("GITHUB_ENV", format!("TOOLCHAINS={identifier}")),
        (
            "GITHUB_PATH",
            fs::canonicalize(bin)?.to_string_lossy().into_owned(),
        ),
    ] {
        if let Some(path) = std::env::var_os(name) {
            writeln!(fs::OpenOptions::new().append(true).open(path)?, "{line}")?;
        }
    }
    command("xcrun", &["--sdk", "macosx", "--show-sdk-version"])?;
    println!("TOOLCHAINS={identifier}");
    println!("verified compiler installed; CI selection recorded when available; SDK unchanged");
    Ok(())
}
