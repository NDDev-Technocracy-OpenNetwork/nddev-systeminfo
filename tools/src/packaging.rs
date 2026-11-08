//! Packages qualified binaries and their native presentation. The same typed
//! manifest is used by this producer and the installer's integrity boundary.
use crate::{Result, command, files};
use nddev_systeminfo::installation::{File, Manifest, digest};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn copy(source: impl AsRef<Path>, destination: impl AsRef<Path>) -> Result {
    let destination = destination.as_ref();
    fs::create_dir_all(destination.parent().ok_or("destination parent missing")?)?;
    fs::copy(source, destination)?;
    Ok(())
}
fn tree(source: &Path, destination: &Path) -> Result {
    for path in files(source)? {
        if path.components().any(|part| part.as_os_str() == "tests") {
            continue;
        }
        copy(&path, destination.join(path.strip_prefix(source)?))?;
    }
    Ok(())
}
pub(super) fn package() -> Result<PathBuf> {
    if cfg!(target_os = "windows") && !cfg!(target_feature = "crt-static") {
        return Err(
            "Windows distribution requires the repository's static C runtime configuration".into(),
        );
    }
    let binaries = crate::build_binaries(true)?;
    let version = crate::version()?;
    let name = format!(
        "nddev-systeminfo-{version}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS
    );
    let root = Path::new(".tools/package").join(&name);
    if root.exists() {
        fs::remove_dir_all(&root)?;
    }
    fs::create_dir_all(root.join("bin"))?;
    for binary in ["nddev-systeminfo", "nddev-systeminfod"] {
        copy(
            &binaries[binary],
            root.join("bin")
                .join(format!("{binary}{}", std::env::consts::EXE_SUFFIX)),
        )?;
    }
    if cfg!(target_os = "windows") {
        copy(
            &binaries["nddev-systeminfo-tray"],
            root.join("bin/nddev-systeminfo-tray.exe"),
        )?;
    }
    if cfg!(target_os = "linux") {
        tree(Path::new("extension"), &root.join("ui/extension"))?;
        command(
            "glib-compile-schemas",
            &[root.join("ui/extension/schemas").to_str().unwrap()],
        )?;
    }
    if cfg!(target_os = "macos") {
        macos(&root, &version)?;
    }
    for file in [
        "LICENSE",
        "NOTICE",
        "THIRD_PARTY_NOTICES.md",
        "CHANGELOG.md",
    ] {
        copy(file, root.join(file))?;
    }
    if cfg!(target_os = "windows") {
        crate::windows_msi::build(&root, &version)?;
    }
    let release = Manifest {
        format: 1,
        version,
        os: std::env::consts::OS.into(),
        architecture: std::env::consts::ARCH.into(),
        files: files(&root)?
            .into_iter()
            .map(|path| {
                let relative = path.strip_prefix(&root)?.to_path_buf();
                let executable = relative.starts_with("bin")
                    || relative
                        .components()
                        .any(|part| part.as_os_str() == "MacOS");
                Ok(File {
                    path: relative,
                    sha256: digest(&path)?,
                    executable,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    };
    fs::write(
        root.join("manifest.toml"),
        toml::to_string_pretty(&release)?,
    )?;
    release.verify(&root)?;
    fs::create_dir_all("dist")?;
    let extension = if cfg!(target_os = "linux") {
        "tar.gz"
    } else {
        "zip"
    };
    let archive = Path::new("dist").join(format!("{name}.{extension}"));
    if archive.exists() {
        fs::remove_file(&archive)?;
    }
    if cfg!(target_os = "linux") {
        command(
            "tar",
            &[
                "-czf",
                archive.to_str().unwrap(),
                "-C",
                ".tools/package",
                &name,
            ],
        )?;
    } else if cfg!(target_os = "macos") {
        command(
            "ditto",
            &[
                "-c",
                "-k",
                "--keepParent",
                root.to_str().unwrap(),
                archive.to_str().unwrap(),
            ],
        )?;
    } else {
        command(
            "tar.exe",
            &[
                "-a",
                "-cf",
                archive.to_str().unwrap(),
                "-C",
                ".tools/package",
                &name,
            ],
        )?;
    }
    fs::write(
        archive.with_file_name(format!("{name}.{extension}.sha256")),
        format!(
            "{}  {}\n",
            digest(&archive)?,
            archive.file_name().unwrap().to_string_lossy()
        ),
    )?;
    println!("verified native package: {}", archive.display());
    Ok(root)
}
fn macos(root: &Path, version: &str) -> Result {
    let app = root.join("ui/NDDev Systeminfo.app");
    let executables = app.join("Contents/MacOS");
    fs::create_dir_all(&executables)?;
    let output = executables.join("nddev-systeminfo-app");
    let architecture = if std::env::consts::ARCH == "aarch64" {
        "arm64"
    } else {
        std::env::consts::ARCH
    };
    let mut arguments = vec![
        "-target".to_string(),
        format!("{architecture}-apple-macosx12.0"),
        "-swift-version".into(),
        "6".into(),
        "-parse-as-library".into(),
        "-warnings-as-errors".into(),
        "-O".into(),
        "-framework".into(),
        "AppKit".into(),
    ];
    arguments.extend(
        files(Path::new("macos"))?
            .into_iter()
            .filter(|path| {
                path.parent() == Some(Path::new("macos"))
                    && path
                        .extension()
                        .is_some_and(|extension| extension == "swift")
            })
            .map(|path| path.to_string_lossy().into_owned()),
    );
    arguments.extend(["-o".into(), output.to_string_lossy().into_owned()]);
    command(
        "swiftc",
        &arguments.iter().map(String::as_str).collect::<Vec<_>>(),
    )?;
    copy(
        root.join("bin/nddev-systeminfo"),
        executables.join("nddev-systeminfo"),
    )?;
    copy(
        "extension/resources/ui.json",
        app.join("Contents/Resources/ui.json"),
    )?;
    fs::write(
        app.join("Contents/Info.plist"),
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleExecutable</key><string>nddev-systeminfo-app</string>
<key>CFBundleIdentifier</key><string>com.nddev-opennetwork.rldyour-sysinfo</string>
<key>CFBundleName</key><string>NDDev Systeminfo</string>
<key>CFBundleDisplayName</key><string>NDDev Systeminfo</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>LSUIElement</key><true/>
<key>NSHighResolutionCapable</key><true/>
</dict></plist>
"#
        ),
    )?;
    command(
        "codesign",
        &["--force", "--deep", "--sign", "-", app.to_str().unwrap()],
    )?;
    command(
        "codesign",
        &["--verify", "--deep", "--strict", app.to_str().unwrap()],
    )?;
    Ok(())
}
