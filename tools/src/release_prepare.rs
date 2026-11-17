//! Source-owned patch preparation before qualification; never publishes refs or assets.
use std::{fs, process::Command};

use crate::{Result, command, generate, version};

fn parts(value: &str) -> Result<[u64; 3]> {
    let values: Vec<_> = value
        .split('.')
        .map(|part| {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err("release version must contain three numeric parts".into());
            }
            Ok(part.parse::<u64>()?)
        })
        .collect::<Result<_>>()?;
    values
        .try_into()
        .map_err(|_| "release version must contain three numeric parts".into())
}

fn prepared_version(base: &str, current: &str) -> Result<String> {
    let [major, minor, patch] = parts(base)?;
    let next = format!(
        "{major}.{minor}.{}",
        patch.checked_add(1).ok_or("patch version overflow")?
    );
    if current != base && current != next {
        return Err("source version differs from both the base and its next patch".into());
    }
    Ok(next)
}

fn manifest(source: &str, next: &str) -> Result<String> {
    let data: toml::Value = toml::from_str(source)?;
    let current = data["workspace"]["package"]["version"]
        .as_str()
        .ok_or("workspace version missing")?;
    let dependencies = data["workspace"]["dependencies"]
        .as_table()
        .ok_or("workspace dependencies missing")?;
    let mut own = std::collections::BTreeMap::new();
    for (name, value) in dependencies {
        if value.get("path").is_some() {
            let requirement = value["version"]
                .as_str()
                .ok_or("self dependency version missing")?;
            let prefix = if requirement.starts_with('=') {
                "="
            } else {
                ""
            };
            if requirement.trim_start_matches('=') != current {
                return Err("self dependency identity differs from the workspace".into());
            }
            own.insert(name.as_str(), (requirement, format!("{prefix}{next}")));
        }
    }
    let mut section = "";
    let mut changed = std::collections::BTreeSet::new();
    let mut output = String::new();
    for line in source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed;
        }
        let mut line = line.to_string();
        if section == "[workspace.package]"
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "version")
        {
            let old = format!("{current:?}");
            if !line.contains(&old) {
                return Err("workspace version is not a concrete quoted declaration".into());
            }
            line = line.replacen(&old, &format!("{next:?}"), 1);
        } else if section == "[workspace.dependencies]"
            && let Some((key, _)) = trimmed.split_once('=')
            && let Some((old, new)) = own.get(key.trim())
        {
            let previous = format!("version = {old:?}");
            if !line.contains(&previous) {
                return Err("self dependency needs an explicit inline version declaration".into());
            }
            line = line.replacen(&previous, &format!("version = {new:?}"), 1);
            changed.insert(key.trim());
        }
        output.push_str(&line);
        output.push('\n');
    }
    if changed.len() != own.len() {
        return Err("a self dependency declaration was not prepared".into());
    }
    let verified: toml::Value = toml::from_str(&output)?;
    if verified["workspace"]["package"]["version"].as_str() != Some(next) {
        return Err("workspace patch identity was not prepared".into());
    }
    Ok(output)
}

fn external_packages(source: &str) -> Result<Vec<toml::Value>> {
    let lock: toml::Value = toml::from_str(source)?;
    Ok(lock["package"]
        .as_array()
        .ok_or("lock packages missing")?
        .iter()
        .filter(|package| package.get("source").is_some())
        .cloned()
        .collect())
}

pub(super) fn run(base: &str) -> Result {
    if base.len() != 40 || !base.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("preparation needs a full base commit SHA".into());
    }
    command("git", &["merge-base", "--is-ancestor", base, "HEAD"])?;
    let original = Command::new("git")
        .args(["show", &format!("{base}:Cargo.toml")])
        .output()?;
    if !original.status.success() {
        return Err("base source identity is unavailable".into());
    }
    let original: toml::Value = toml::from_str(std::str::from_utf8(&original.stdout)?)?;
    let previous = original["workspace"]["package"]["version"]
        .as_str()
        .ok_or("base workspace version missing")?;
    let current = version()?;
    let next = prepared_version(previous, &current)?;
    let external = external_packages(&fs::read_to_string("Cargo.lock")?)?;
    let prepared = manifest(&fs::read_to_string("Cargo.toml")?, &next)?;
    fs::write("Cargo.toml", prepared)?;
    command("cargo", &["update", "--workspace", "--offline"])?;
    if external_packages(&fs::read_to_string("Cargo.lock")?)? != external {
        return Err("patch preparation changed an external dependency identity".into());
    }
    generate()?;
    println!("Prepared source patch {next} from base {base}; no tag or publication performed");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_patch_is_idempotent_for_the_original_base() {
        assert_eq!(prepared_version("0.3.14", "0.3.14").unwrap(), "0.3.15");
        assert_eq!(prepared_version("0.3.14", "0.3.15").unwrap(), "0.3.15");
        assert!(prepared_version("0.3.14", "0.3.16").is_err());
        assert!(prepared_version("0.3.14-beta", "0.3.14-beta").is_err());
    }

    #[test]
    fn source_preparation_preserves_comments_and_external_versions() {
        let original = "# retained reason\n[workspace.package]\nversion = \"0.3.14\" # identity\n[workspace.dependencies]\ninternal = { path = \"daemon\", version = \"=0.3.14\" }\nexternal = \"=0.3.14\"\n";
        let prepared = manifest(original, "0.3.15").unwrap();
        assert!(prepared.contains("# retained reason"));
        assert!(prepared.contains("# identity"));
        let data: toml::Value = toml::from_str(&prepared).unwrap();
        assert_eq!(
            data["workspace"]["dependencies"]["external"].as_str(),
            Some("=0.3.14")
        );
        assert_eq!(
            data["workspace"]["dependencies"]["internal"]["version"].as_str(),
            Some("=0.3.15")
        );
        assert_eq!(manifest(&prepared, "0.3.15").unwrap(), prepared);
    }
}
