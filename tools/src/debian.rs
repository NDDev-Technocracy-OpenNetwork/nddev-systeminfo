//! Native Debian production and real helper/package lifecycle acceptance.
use crate::{Result, command, files};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
const SOCKET: &str = "rldyour-sysinfod.socket";
mod users;

fn copy(from: &Path, to: &Path, executable: bool) -> Result {
    use std::os::unix::fs::PermissionsExt;
    fs::create_dir_all(to.parent().ok_or("package parent missing")?)?;
    fs::copy(from, to)?;
    fs::set_permissions(
        to,
        fs::Permissions::from_mode(if executable { 0o755 } else { 0o644 }),
    )?;
    Ok(())
}
fn stage(name: &str) -> Result<PathBuf> {
    let path = Path::new(".tools/debian").join(name);
    if path.exists() {
        fs::remove_dir_all(&path)?;
    }
    fs::create_dir_all(path.join("DEBIAN"))?;
    Ok(path)
}
fn archive(root: &Path, output: &Path) -> Result {
    use std::os::unix::fs::PermissionsExt;
    fn directories(path: &Path) -> Result {
        fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
        for entry in fs::read_dir(path)? {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                directories(&entry.path())?;
            }
        }
        Ok(())
    }
    directories(root)?;
    command(
        "dpkg-deb",
        &[
            "--root-owner-group",
            "--build",
            root.to_str().unwrap(),
            output.to_str().unwrap(),
        ],
    )?;
    Ok(())
}
pub(super) fn package() -> Result {
    if std::env::consts::ARCH != "x86_64" {
        return Err("Debian package currently qualifies amd64".into());
    }
    let binaries = crate::build_binaries(true)?;
    let release = crate::packaging::package()?;
    let version = crate::version()?;
    let root = stage("nddev-systeminfo")?;
    let cli = &binaries["nddev-systeminfo"];
    let provider = &binaries["nddev-systeminfod"];
    let hook = &binaries["nddev-systeminfo-package"];
    for (name, binary) in [("nddev-systeminfo", cli), ("nddev-systeminfod", provider)] {
        copy(binary, &root.join("usr/bin").join(name), true)?;
    }
    for name in ["rldyour-sysinfod.socket", "rldyour-sysinfod.service"] {
        let source = fs::read_to_string(Path::new("daemon/systemd").join(name))?.replace(
            "ExecStart=%h/.local/bin/nddev-systeminfod",
            "ExecStart=/usr/bin/nddev-systeminfod",
        );
        let target = root.join("usr/lib/systemd/user").join(name);
        fs::create_dir_all(target.parent().unwrap())?;
        fs::write(target, source)?;
    }
    let extension = release.join("ui/extension");
    let metadata: serde_json::Value =
        serde_json::from_slice(&fs::read(extension.join("metadata.json"))?)?;
    let uuid = metadata["uuid"].as_str().ok_or("extension UUID missing")?;
    for source in files(&extension)? {
        copy(
            &source,
            &root
                .join("usr/share/gnome-shell/extensions")
                .join(uuid)
                .join(source.strip_prefix(&extension)?),
            false,
        )?;
    }
    copy(
        Path::new("packaging/linux/nddev-systeminfo.desktop"),
        &root.join(format!(
            "usr/share/applications/{}.desktop",
            nddev_systeminfo::desktop::application_id()
        )),
        false,
    )?;
    copy(
        &extension.join("resources/brand-symbolic.svg"),
        &root.join("usr/share/icons/hicolor/scalable/apps/nddev-systeminfo-symbolic.svg"),
        false,
    )?;
    for file in ["LICENSE", "NOTICE", "NOTICE.third-party.md"] {
        copy(
            Path::new(file),
            &root
                .join("usr/share/doc/nddev-systeminfo")
                .join(if file == "LICENSE" { "copyright" } else { file }),
            false,
        )?;
    }
    let scratch = Path::new(".tools/debian/dependencies");
    fs::create_dir_all(scratch.join("debian"))?;
    fs::write(
        scratch.join("debian/control"),
        "Source: nddev-systeminfo\nMaintainer: NDDev OpenNetwork <danil@nddev.it.com>\n\nPackage: nddev-systeminfo\nArchitecture: amd64\nDescription: Native system information\n",
    )?;
    let mut dependencies = Command::new("dpkg-shlibdeps");
    dependencies.current_dir(scratch).arg("-O");
    for binary in [cli, provider, hook] {
        dependencies.arg("-e").arg(fs::canonicalize(binary)?);
    }
    let output = dependencies.output()?;
    if !output.status.success() {
        return Err(format!(
            "native shared-library dependency scan failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    let output = String::from_utf8(output.stdout)?;
    let libraries = output
        .trim()
        .strip_prefix("shlibs:Depends=")
        .filter(|value| !value.is_empty())
        .ok_or("native library dependencies missing")?;
    let control = fs::read_to_string("packaging/deb/control")?
        .replace("@VERSION@", &version)
        .replace("@SHLIBS@", libraries);
    fs::write(root.join("DEBIAN/control"), control)?;
    for name in ["postinst", "prerm", "postrm"] {
        copy(hook, &root.join("DEBIAN").join(name), true)?;
    }
    archive(
        &root,
        &Path::new("dist").join(format!("nddev-systeminfo_{version}_amd64.deb")),
    )?;
    let transition = stage("rldyour-sysinfo")?;
    fs::write(
        transition.join("DEBIAN/control"),
        format!(
            "Package: rldyour-sysinfo\nVersion: {version}\nArchitecture: all\nMaintainer: NDDev OpenNetwork <danil@nddev.it.com>\nDepends: nddev-systeminfo (= {version})\nSection: oldlibs\nPriority: optional\nDescription: Transitional package for NDDev Systeminfo\n Existing installations upgrade to the independent native product.\n"
        ),
    )?;
    archive(
        &transition,
        &Path::new("dist").join(format!("rldyour-sysinfo_{version}_all.deb")),
    )?;
    Ok(())
}

pub(super) fn test() -> Result {
    package()?;
    let binaries = crate::build_binaries(true)?;
    let root = stage("helper-fixture")?;
    let unit = root.join("usr/lib/systemd/user").join(SOCKET);
    copy(
        Path::new("daemon/systemd").join(SOCKET).as_path(),
        &unit,
        false,
    )?;
    let link = root
        .join("etc/systemd/user/sockets.target.wants")
        .join(SOCKET);
    let invoke = |name: &str, action: &str| -> Result {
        let status = Command::new(&binaries["nddev-systeminfo-package"])
            .arg(action)
            .env("DPKG_ROOT", fs::canonicalize(&root)?)
            .env("DPKG_MAINTSCRIPT_PACKAGE", "nddev-systeminfo")
            .env("DPKG_MAINTSCRIPT_NAME", name)
            .status()?;
        if !status.success() {
            return Err(format!("native Debian hook failed: {name} {action}").into());
        }
        Ok(())
    };
    invoke("postinst", "configure")?;
    if !link.is_symlink() {
        return Err("fresh Debian socket was not enabled".into());
    }
    invoke("postinst", "configure")?;
    if !link.is_symlink() {
        return Err("enabled Debian upgrade lost enablement".into());
    }
    fs::remove_file(&link)?;
    invoke("postinst", "configure")?;
    if link.is_symlink() {
        return Err("disabled Debian upgrade was re-enabled".into());
    }
    invoke("postrm", "purge")?;
    fs::remove_dir_all(root)?;
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return Ok(());
    }
    let version = crate::version()?;
    let users = users::Users::create()?;
    command(
        "sudo",
        &[
            "dpkg",
            "-i",
            &format!("dist/nddev-systeminfo_{version}_amd64.deb"),
            &format!("dist/rldyour-sysinfo_{version}_all.deb"),
        ],
    )?;
    users.verify()?;
    command("systemctl", &["--global", "is-enabled", SOCKET])?;
    command("/usr/bin/nddev-systeminfo", &["--version"])?;
    let portable = crate::packaging::package()?;
    let conflict = Command::new(&binaries["nddev-systeminfo"])
        .args(["install", "--provider-only", "--source"])
        .arg(&portable)
        .output()?;
    if conflict.status.success()
        || !String::from_utf8_lossy(&conflict.stderr).contains("a system package owns the provider")
    {
        return Err("portable installation did not preserve the system package's ownership".into());
    }
    command("sudo", &["systemctl", "--global", "disable", SOCKET])?;
    command(
        "sudo",
        &[
            "dpkg",
            "-i",
            &format!("dist/nddev-systeminfo_{version}_amd64.deb"),
        ],
    )?;
    if Command::new("systemctl")
        .args(["--global", "is-enabled", SOCKET])
        .stdout(Stdio::null())
        .status()?
        .success()
    {
        return Err("real dpkg reinstall re-enabled the socket".into());
    }
    command(
        "sudo",
        &["dpkg", "--purge", "rldyour-sysinfo", "nddev-systeminfo"],
    )?;
    if Path::new("/usr/bin/nddev-systeminfod").exists() {
        return Err("dpkg purge left the provider".into());
    }
    users.finish()?;
    println!("PASS: real dpkg install, transition, disabled reinstall, native hooks and purge");
    Ok(())
}
