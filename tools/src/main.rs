//! Reproducible source checks, tools and packaging. Run builds on a build host.
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};
type Result<T = ()> = std::result::Result<T, Box<dyn Error>>;

#[derive(Deserialize)]
struct Toolchain {
    tools: Vec<Tool>,
}
#[derive(Deserialize)]
struct Tool {
    name: String,
    version: String,
    platforms: BTreeMap<String, Artifact>,
}
#[derive(Deserialize)]
struct Artifact {
    url: String,
    sha256: String,
}

fn command(program: impl AsRef<std::ffi::OsStr>, args: &[&str]) -> Result {
    let result = Command::new(program).args(args).status()?;
    if !result.success() {
        return Err(format!("command failed: {args:?} ({result})").into());
    }
    Ok(())
}
fn binary(name: &str) -> PathBuf {
    Path::new(".tools/bin").join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}
fn files(path: &Path) -> Result<Vec<PathBuf>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            result.extend(files(&path)?);
        } else {
            result.push(path);
        }
    }
    result.sort();
    Ok(result)
}
fn digest(path: &Path) -> Result<String> {
    let argument = path.to_str().ok_or("non-UTF8 artifact path")?;
    let output = if cfg!(target_os = "windows") {
        Command::new("certutil")
            .args(["-hashfile", argument, "SHA256"])
            .output()?
    } else if cfg!(target_os = "macos") {
        Command::new("shasum")
            .args(["-a", "256", argument])
            .output()?
    } else {
        Command::new("sha256sum").arg(path).output()?
    };
    if !output.status.success() {
        return Err("artifact digest failed".into());
    }
    String::from_utf8(output.stdout)?
        .split_whitespace()
        .find(|word| word.len() == 64 && word.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .map(|value| value.to_ascii_lowercase())
        .ok_or_else(|| "artifact digest is missing".into())
}
fn setup() -> Result {
    let tools: Toolchain = toml::from_str(include_str!("../toolchain.toml"))?;
    let host = format!("{}-{}", std::env::consts::OS, std::env::consts::ARCH);
    fs::create_dir_all(".tools/bin")?;
    for tool in tools.tools {
        let artifact = tool
            .platforms
            .get(&host)
            .ok_or_else(|| format!("unsupported tool host: {host}"))?;
        let destination = binary(&tool.name);
        let record = destination.with_extension("sha256");
        if destination.is_file()
            && record.is_file()
            && digest(&destination)? == fs::read_to_string(&record)?.trim()
        {
            continue;
        }
        let folder = Path::new(".tools/downloads").join(&tool.name);
        if folder.exists() {
            fs::remove_dir_all(&folder)?;
        }
        fs::create_dir_all(&folder)?;
        let archive = folder.join("tool.tar.gz");
        command(
            "curl",
            &[
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--output",
                archive.to_str().unwrap(),
                &artifact.url,
            ],
        )?;
        if digest(&archive)? != artifact.sha256 {
            return Err(format!("checksum mismatch: {} {}", tool.name, tool.version).into());
        }
        command(
            "tar",
            &[
                "-xzf",
                archive.to_str().unwrap(),
                "-C",
                folder.to_str().unwrap(),
            ],
        )?;
        let filename = format!("{}{}", tool.name, std::env::consts::EXE_SUFFIX);
        let executable = files(&folder)?
            .into_iter()
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name == filename.as_str())
            })
            .ok_or("tool executable missing from verified archive")?;
        fs::copy(&executable, &destination)?;
        fs::write(record, digest(&destination)?)?;
        fs::remove_dir_all(folder)?;
        println!(
            "installed {} {} from its verified release",
            tool.name, tool.version
        );
    }
    let own_binary = binary("systeminfo-tools");
    if !own_binary.exists()
        || fs::canonicalize(std::env::current_exe()?)? != fs::canonicalize(&own_binary)?
    {
        fs::copy(std::env::current_exe()?, own_binary)?;
    }
    command("cargo", &["fetch", "--locked"])?;
    command(binary("cargo-deny"), &["deny", "fetch"])?;
    if let Some(path) = std::env::var_os("GITHUB_PATH") {
        use std::io::Write;
        writeln!(
            fs::OpenOptions::new().append(true).open(path)?,
            "{}",
            fs::canonicalize(".tools/bin")?.display()
        )?;
    }
    Ok(())
}
fn version() -> Result<String> {
    let manifest: toml::Value = toml::from_str(&fs::read_to_string("Cargo.toml")?)?;
    Ok(manifest["workspace"]["package"]["version"]
        .as_str()
        .ok_or("workspace version missing")?
        .to_string())
}
fn generate() -> Result {
    let version = version()?;
    let mut python: toml::Value = toml::from_str(&fs::read_to_string("python/pyproject.toml")?)?;
    python["project"]["version"] = toml::Value::String(version.clone());
    fs::write("python/pyproject.toml", toml::to_string_pretty(&python)?)?;
    let path = "python/src/rldyour_sysinfo/__init__.py";
    let source = fs::read_to_string(path)?;
    let lines: Vec<String> = source
        .lines()
        .map(|line| {
            if line.starts_with("__version__ =") {
                format!("__version__ = {version:?}")
            } else {
                line.to_string()
            }
        })
        .collect();
    fs::write(path, lines.join("\n") + "\n")?;
    let mut metadata: serde_json::Value =
        serde_json::from_str(&fs::read_to_string("extension/metadata.json")?)?;
    metadata["version-name"] = version.clone().into();
    let segments: Vec<u64> = version
        .split('.')
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    if segments.len() != 3 {
        return Err("version needs three numbers".into());
    }
    metadata["version"] = (segments[0] * 10000 + segments[1] * 100 + segments[2]).into();
    fs::write(
        "extension/metadata.json",
        serde_json::to_string_pretty(&metadata)? + "\n",
    )?;
    Ok(())
}
fn check() -> Result {
    let version = version()?;
    let python: toml::Value = toml::from_str(&fs::read_to_string("python/pyproject.toml")?)?;
    if python["project"]["version"].as_str() != Some(&version)
        || !fs::read_to_string("python/src/rldyour_sysinfo/__init__.py")?
            .contains(&format!("__version__ = {version:?}"))
    {
        return Err("generated versions drifted; run just generate".into());
    }
    let metadata: serde_json::Value =
        serde_json::from_str(&fs::read_to_string("extension/metadata.json")?)?;
    if metadata["version-name"].as_str() != Some(&version) {
        return Err("extension version drifted; run just generate".into());
    }
    for key in [
        "uuid",
        "name",
        "description",
        "shell-version",
        "url",
        "settings-schema",
    ] {
        if metadata.get(key).is_none() {
            return Err(format!("extension metadata missing: {key}").into());
        }
    }
    let schema = files(Path::new("extension/schemas"))?
        .iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "xml"))
        .map(fs::read_to_string)
        .collect::<std::result::Result<Vec<_>, _>>()?
        .join("\n");
    if !schema.contains(&format!(
        "schema id={:?}",
        metadata["settings-schema"]
            .as_str()
            .ok_or("settings schema missing")?
    )) {
        return Err("extension settings schema is not defined".into());
    }
    for path in files(Path::new("extension"))? {
        if !path.extension().is_some_and(|extension| extension == "js") {
            continue;
        }
        command("node", &["--check", path.to_str().unwrap()])?;
        let text = fs::read_to_string(&path)?;
        let preferences = path.file_name().is_some_and(|name| name == "prefs.js");
        let test = path.components().any(|part| part.as_os_str() == "tests");
        let forbidden = if preferences {
            vec!["gi://St", "gi://Clutter", "gi://Meta", "gi://Shell"]
        } else if !test {
            vec!["gi://Gtk", "gi://Adw", "gi://Gdk"]
        } else {
            Vec::new()
        };
        if forbidden.iter().any(|import| text.contains(import))
            || [
                "imports.mainloop",
                "imports.lang",
                "imports.byteArray",
                "gi://ByteArray",
            ]
            .iter()
            .any(|import| text.contains(import))
        {
            return Err(format!("extension process boundary violation: {}", path.display()).into());
        }
        for pattern in [
            "get_boolean('",
            "get_int('",
            "get_string('",
            "get_double('",
            "key: '",
            "settings.bind('",
        ] {
            for tail in text.split(pattern).skip(1) {
                let key = tail.split('\'').next().unwrap();
                if !schema.contains(&format!("key name=\"{key}\"")) {
                    return Err(format!("undeclared extension setting: {key}").into());
                }
            }
        }
    }
    if cfg!(target_os = "linux") {
        command(
            "glib-compile-schemas",
            &["--strict", "--dry-run", "extension/schemas"],
        )?;
    }
    command("git", &["diff", "--check"])?;
    println!("source boundaries, extension, generated versions and whitespace passed ({version})");
    Ok(())
}
fn test() -> Result {
    command(
        "cargo",
        &[
            "clippy",
            "--locked",
            "--workspace",
            "--all-targets",
            "--all-features",
            "--",
            "-D",
            "warnings",
        ],
    )?;
    let binaries = build_binaries(false)?;
    let mut runner = Command::new(binary("cargo-nextest"));
    runner.args([
        "nextest",
        "run",
        "--locked",
        "--workspace",
        "--all-features",
    ]);
    for name in ["nddev-systeminfod", "nddev-systeminfo"] {
        runner.env(
            format!("CARGO_BIN_EXE_{name}"),
            binaries
                .get(name)
                .ok_or("native binary missing from Cargo receipt")?,
        );
    }
    if !runner.status()?.success() {
        return Err("native test suite failed".into());
    }
    Ok(())
}

fn build_binaries(release: bool) -> Result<BTreeMap<String, PathBuf>> {
    use std::io::{BufRead, BufReader};
    let mut cargo = Command::new("cargo");
    cargo.args([
        "build",
        "--locked",
        "--workspace",
        "--all-features",
        "--bins",
        "--message-format=json",
    ]);
    if release {
        cargo.arg("--release");
    }
    let mut build = cargo.stdout(std::process::Stdio::piped()).spawn()?;
    let mut binaries = BTreeMap::new();
    for line in BufReader::new(build.stdout.take().unwrap()).lines() {
        let line = line?;
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
            eprintln!("{line}");
            continue;
        };
        if message["reason"] == "compiler-artifact"
            && let (Some(name), Some(executable)) = (
                message["target"]["name"].as_str(),
                message["executable"].as_str(),
            )
        {
            binaries.insert(name.to_string(), PathBuf::from(executable));
        }
        if message["reason"] == "compiler-message"
            && let Some(rendered) = message["message"]["rendered"].as_str()
        {
            eprint!("{rendered}");
        }
    }
    if !build.wait()?.success() {
        return Err("native binary build failed".into());
    }
    Ok(binaries)
}
fn run() -> Result {
    match std::env::args().nth(1).as_deref() {
        Some("setup") => setup(),
        Some("generate") => generate(),
        Some("check") => check(),
        Some("test") => test(),
        _ => Err("usage: systeminfo-tools <setup|generate|check|test>".into()),
    }
}
fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
