//! Actual bounded patch-preparation CLI behavior in a disposable source fixture.
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

struct Fixture(PathBuf);
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn run(root: &Path, program: &str, args: &[&str]) -> std::process::Output {
    Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .unwrap()
}
fn copy(source: &Path, target: &Path) {
    fs::create_dir_all(target).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        if [".git", ".tools", "target", "dist"]
            .iter()
            .any(|name| entry.file_name() == *name)
        {
            continue;
        }
        let destination = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy(&entry.path(), &destination);
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
}
#[test]
fn ordinary_base_preparation_is_real_bounded_and_repeatable() {
    let directory = std::env::temp_dir().join(format!(
        "systeminfo-prepare-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let root = Fixture(directory);
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    copy(source, &root.0);
    let current = fs::read_to_string(root.0.join("Cargo.toml")).unwrap();
    let metadata: toml::Value = toml::from_str(&current).unwrap();
    let version = metadata["workspace"]["package"]["version"]
        .as_str()
        .unwrap();
    let parts: Vec<u64> = version
        .split('.')
        .map(|part| part.parse().unwrap())
        .collect();
    let expected = format!("{}.{}.{}", parts[0], parts[1], parts[2] + 1);
    assert!(run(&root.0, "git", &["init", "--quiet"]).status.success());
    assert!(run(&root.0, "git", &["add", "."]).status.success());
    assert!(
        run(
            &root.0,
            "git",
            &[
                "-c",
                "user.name=Fixture",
                "-c",
                "user.email=fixture@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "--quiet",
                "-m",
                "fixture base"
            ]
        )
        .status
        .success()
    );
    let base = String::from_utf8(run(&root.0, "git", &["rev-parse", "HEAD"]).stdout).unwrap();
    let binary = env!("CARGO_BIN_EXE_systeminfo-tools");
    let first = run(&root.0, binary, &["prepare-release", base.trim()]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let prepared = fs::read_to_string(root.0.join("Cargo.toml")).unwrap();
    let metadata: toml::Value = toml::from_str(&prepared).unwrap();
    assert_eq!(
        metadata["workspace"]["package"]["version"].as_str(),
        Some(expected.as_str())
    );
    assert!(
        fs::read_to_string(root.0.join("python/pyproject.toml"))
            .unwrap()
            .contains(&expected)
    );
    let lock = fs::read(root.0.join("Cargo.lock")).unwrap();
    let second = run(&root.0, binary, &["prepare-release", base.trim()]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(
        fs::read_to_string(root.0.join("Cargo.toml")).unwrap(),
        prepared
    );
    assert_eq!(fs::read(root.0.join("Cargo.lock")).unwrap(), lock);
    assert!(
        !run(&root.0, binary, &["prepare-release", "--all"])
            .status
            .success()
    );
}
