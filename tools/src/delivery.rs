//! Publish the exact qualified CI artifacts. Never rebuild a product here.
use crate::{Result, command, digest, files};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    format: u32,
    github: Repository,
    artifacts: Artifacts,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Repository {
    repository_id: u64,
    workflow_id: u64,
    development_branch: String,
    stable_branch: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Artifacts {
    required: Vec<String>,
}

struct Github {
    repository: String,
}
enum Download {
    CiArchive(u64),
    ReleaseAsset(u64),
}
impl Download {
    fn request(self) -> (String, &'static str) {
        match self {
            Self::CiArchive(id) => (
                format!("actions/artifacts/{id}/zip"),
                "Accept: application/json",
            ),
            Self::ReleaseAsset(id) => (
                format!("releases/assets/{id}"),
                "Accept: application/octet-stream",
            ),
        }
    }
}
impl Github {
    fn resource(&self, path: &str) -> String {
        let root = format!("repos/{}", self.repository);
        if path.is_empty() {
            root
        } else {
            format!("{root}/{path}")
        }
    }
    fn api(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value> {
        let mut request = Command::new("gh");
        request.args([
            "api",
            "--hostname",
            "github.com",
            "--method",
            method,
            &self.resource(path),
        ]);
        if body.is_some() {
            request.args(["--input", "-"]).stdin(Stdio::piped());
        }
        let mut child = request
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        if let Some(body) = body {
            let mut input = child.stdin.take().ok_or("GitHub request input missing")?;
            serde_json::to_writer(&mut input, &body)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            return Err(format!(
                "GitHub {method} {path}: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        if output.stdout.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_slice(&output.stdout)?)
        }
    }
    fn optional(&self, path: &str) -> Result<Option<Value>> {
        // A 404 is the one expected missing state. Other API failures never
        // turn into permission to create or overwrite a release.
        let output = Command::new("gh")
            .args(["api", "--hostname", "github.com", &self.resource(path)])
            .output()?;
        if output.status.success() {
            return Ok(Some(serde_json::from_slice(&output.stdout)?));
        }
        if String::from_utf8_lossy(&output.stderr).contains("HTTP 404") {
            return Ok(None);
        }
        Err(format!(
            "GitHub read {path}: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into())
    }
    fn pages(&self, path: &str, field: &str) -> Result<Vec<Value>> {
        let mut values = Vec::new();
        for page in 1..=100 {
            let result = self.api("GET", &format!("{path}?per_page=100&page={page}"), None)?;
            let items = result[field]
                .as_array()
                .ok_or("GitHub collection missing")?;
            values.extend_from_slice(items);
            if items.len() < 100 {
                return Ok(values);
            }
        }
        Err("GitHub collection exceeds the release metadata bound".into())
    }
    fn release(&self, tag: &str) -> Result<Option<Value>> {
        if let Some(release) = self.optional(&format!("releases/tags/{tag}"))? {
            return Ok(Some(release));
        }
        // The tag endpoint finds published releases. Authenticated listing also
        // includes drafts with a pending tag, needed after interrupted uploads.
        for page in 1..=100 {
            let releases = self.api("GET", &format!("releases?per_page=100&page={page}"), None)?;
            let releases = releases.as_array().ok_or("release collection missing")?;
            let matches: Vec<_> = releases
                .iter()
                .filter(|release| release["tag_name"] == tag)
                .collect();
            if matches.len() > 1 {
                return Err("release tag is ambiguous".into());
            }
            if let Some(release) = matches.first() {
                return Ok(Some((*release).clone()));
            }
            if releases.len() < 100 {
                return Ok(None);
            }
        }
        Err("release collection exceeds its metadata bound".into())
    }
    fn download(&self, source: Download, destination: &Path) -> Result {
        let (path, accept) = source.request();
        let output = Command::new("gh")
            .args([
                "api",
                "--hostname",
                "github.com",
                "--header",
                accept,
                &self.resource(&path),
            ])
            .stdout(fs::File::create(destination)?)
            .stderr(Stdio::piped())
            .output()?;
        if !output.status.success() {
            return Err(format!(
                "GitHub artifact download: {}",
                String::from_utf8_lossy(&output.stderr)
            )
            .into());
        }
        Ok(())
    }
}
fn configuration() -> Result<(Configuration, Github)> {
    let configuration: Configuration = toml::from_str(&fs::read_to_string("release.toml")?)?;
    if configuration.format != 1 {
        return Err("unsupported release configuration".into());
    }
    let repository = std::env::var("GITHUB_REPOSITORY")?;
    let manifest: toml::Value = toml::from_str(&fs::read_to_string("Cargo.toml")?)?;
    if manifest["workspace"]["package"]["repository"].as_str()
        != Some(format!("https://github.com/{repository}").as_str())
    {
        return Err("release repository differs from its source identity".into());
    }
    let github = Github { repository };
    let actual = github.api("GET", "", None)?;
    if actual["id"].as_u64() != Some(configuration.github.repository_id) {
        return Err("release repository ID differs from the original".into());
    }
    Ok((configuration, github))
}
fn publisher_prefix() -> Result<String> {
    let source = fs::read_to_string(".github/workflows/ci.yml")?;
    let lines = source
        .lines()
        .skip_while(|line| *line != "  release:")
        .skip(1);
    let name = lines
        .take_while(|line| line.starts_with("    ") || line.is_empty())
        .find_map(|line| line.strip_prefix("    name: "))
        .ok_or("publication job identity missing")?;
    if !name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b' ') {
        return Err("publication job identity must be plain text".into());
    }
    Ok(format!("{name} / "))
}
fn qualified(
    run: &Value,
    jobs: &[Value],
    configuration: &Configuration,
    commit: &str,
    publication_prefix: &str,
) -> Result {
    if run["event"] != "push"
        || run["head_branch"].as_str() != Some(&configuration.github.development_branch)
        || run["head_sha"] != commit
        || run["workflow_id"].as_u64() != Some(configuration.github.workflow_id)
        || run["repository"]["id"].as_u64() != Some(configuration.github.repository_id)
        || run["head_repository"]["id"].as_u64() != Some(configuration.github.repository_id)
        || run["path"] != ".github/workflows/ci.yml"
    {
        return Err("release qualification must be the original repository's dev push CI".into());
    }
    let qualification: Vec<_> = jobs
        .iter()
        .filter(|job| {
            !job["name"]
                .as_str()
                .unwrap_or_default()
                .starts_with(publication_prefix)
        })
        .collect();
    if qualification.is_empty() {
        return Err("CI qualification jobs missing".into());
    }
    for job in qualification {
        let expected_skip = job["name"] == "check / merge stops" && job["conclusion"] == "skipped";
        if job["status"] != "completed" || !(job["conclusion"] == "success" || expected_skip) {
            return Err(format!("qualification job did not pass: {}", job["name"]).into());
        }
    }
    Ok(())
}
fn full_sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn source_file(commit: &str, path: &str) -> Result<String> {
    let output = Command::new("git")
        .arg("show")
        .arg(format!("{commit}:{path}"))
        .output()?;
    if !output.status.success() {
        return Err("qualified source data is unavailable".into());
    }
    Ok(String::from_utf8(output.stdout)?)
}
fn tag_commit(github: &Github, tag: &Value) -> Result<String> {
    if tag["object"]["type"] != "tag" {
        return Err("release tag must be annotated".into());
    }
    let tag = github.api(
        "GET",
        &format!(
            "git/tags/{}",
            tag["object"]["sha"].as_str().ok_or("tag object missing")?
        ),
        None,
    )?;
    if tag["object"]["type"] != "commit" {
        return Err("release tag does not identify a commit".into());
    }
    Ok(tag["object"]["sha"]
        .as_str()
        .ok_or("tag commit missing")?
        .into())
}
fn unpack(github: &Github, artifact: &Value, destination: &Path) -> Result {
    let id = artifact["id"].as_u64().ok_or("artifact id missing")?;
    if artifact["expired"] != false
        || artifact["size_in_bytes"]
            .as_u64()
            .is_none_or(|size| size > 256 * 1024 * 1024)
    {
        return Err("artifact is expired or exceeds its size bound".into());
    }
    fs::create_dir_all(destination)?;
    let zip = destination.with_extension("zip");
    github.download(Download::CiArchive(id), &zip)?;
    if artifact["digest"].as_str() != Some(format!("sha256:{}", digest(&zip)?).as_str()) {
        return Err("CI artifact ZIP digest differs from GitHub metadata".into());
    }
    let listing = Command::new("unzip").args(["-Z1"]).arg(&zip).output()?;
    if !listing.status.success() {
        return Err("invalid CI ZIP archive".into());
    }
    let listing = String::from_utf8(listing.stdout)?;
    let mut seen = std::collections::BTreeSet::new();
    for name in listing.lines() {
        if name.is_empty()
            || name == "."
            || name == ".."
            || !name
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            || !seen.insert(name)
        {
            return Err("CI archive must contain unique plain filenames".into());
        }
    }
    if seen.is_empty() || seen.len() > 99 {
        return Err("qualified archive is empty or exceeds its file bound".into());
    }
    for name in seen {
        // Stream entry bytes to a file we own. ZIP attributes can never create
        // a symlink or directory, including before later entries are examined.
        let result = Command::new("unzip")
            .arg("-p")
            .arg(&zip)
            .arg(name)
            .stdout(fs::File::create(destination.join(name))?)
            .stderr(Stdio::piped())
            .output()?;
        if !result.status.success() {
            return Err("cannot read a qualified archive entry".into());
        }
    }
    Ok(())
}

fn release_notes(changelog: &str, version: &str) -> Result<String> {
    let heading = format!("## {version} ");
    let mut lines = changelog
        .lines()
        .skip_while(|line| !line.starts_with(&heading));
    let first = lines.next().ok_or("release changelog section missing")?;
    Ok(std::iter::once(first)
        .chain(lines.take_while(|line| !line.starts_with("## ")))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string())
}

pub(super) fn publish() -> Result {
    if std::env::var("GITHUB_ACTIONS").as_deref() != Ok("true") {
        return Err("publication requires GitHub's scoped delivery job".into());
    }
    let run_id: u64 = std::env::var("QUALIFICATION_RUN_ID")?.parse()?;
    let commit = std::env::var("QUALIFICATION_COMMIT")?;
    if !full_sha(&commit) {
        return Err("qualification commit must be a full SHA".into());
    }
    let (configuration, github) = configuration()?;
    let run = github.api("GET", &format!("actions/runs/{run_id}"), None)?;
    let jobs = github.pages(&format!("actions/runs/{run_id}/jobs"), "jobs")?;
    qualified(&run, &jobs, &configuration, &commit, &publisher_prefix()?)?;
    let revision = Command::new("git").args(["rev-parse", "HEAD"]).output()?;
    let publisher_commit = String::from_utf8(revision.stdout)?.trim().to_string();
    let development = github.api(
        "GET",
        &format!("git/ref/heads/{}", configuration.github.development_branch),
        None,
    )?;
    if development["object"]["sha"] != publisher_commit {
        return Err("publisher must run from the maintained dev branch".into());
    }
    if !Command::new("git")
        .args(["diff", "--quiet", "HEAD", "--"])
        .status()?
        .success()
    {
        return Err("publisher source has uncommitted changes".into());
    }
    let manifest: toml::Value = toml::from_str(&source_file(&commit, "Cargo.toml")?)?;
    let version = manifest["workspace"]["package"]["version"]
        .as_str()
        .ok_or("qualified product version missing")?
        .to_string();
    if version.split('.').count() != 3 || !version.bytes().all(|c| c.is_ascii_digit() || c == b'.')
    {
        return Err("stable release needs a numeric semantic version".into());
    }
    let tag = format!("v{version}");
    let existing_tag = github.optional(&format!("git/ref/tags/{tag}"))?;
    if let Some(existing) = &existing_tag
        && tag_commit(&github, existing)? != commit
    {
        return Err("published version belongs to another immutable commit".into());
    }
    let existing_release = github.release(&tag)?;
    if existing_tag.is_none()
        && existing_release
            .as_ref()
            .is_some_and(|release| release["draft"] != true)
    {
        return Err("a published release has lost its immutable tag; it must not be recreated from another qualification".into());
    }
    let directory = Path::new(".tools/delivery").join(run_id.to_string());
    if directory.exists() {
        fs::remove_dir_all(&directory)?;
    }
    fs::create_dir_all(&directory)?;
    let available = github.pages(&format!("actions/runs/{run_id}/artifacts"), "artifacts")?;
    let mut payload = BTreeMap::<String, (PathBuf, String)>::new();
    let mut evidence = Vec::new();
    for name in &configuration.artifacts.required {
        let matches: Vec<_> = available
            .iter()
            .filter(|artifact| artifact["name"].as_str() == Some(name))
            .collect();
        if matches.len() != 1 {
            return Err(format!("qualified artifact missing or ambiguous: {name}").into());
        }
        let artifact = matches[0];
        if artifact["workflow_run"]["head_sha"] != commit
            || artifact["workflow_run"]["head_repository_id"].as_u64()
                != Some(configuration.github.repository_id)
        {
            return Err("artifact source identity differs from qualification".into());
        }
        let target = directory.join(name);
        unpack(&github, artifact, &target)?;
        for file in files(&target)? {
            if fs::symlink_metadata(&file)?.file_type().is_symlink() {
                return Err("release artifact contains a redirected file".into());
            }
            let name = file
                .file_name()
                .ok_or("asset name missing")?
                .to_str()
                .ok_or("asset name must be UTF-8")?
                .to_string();
            if file.parent() != Some(target.as_path())
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"._-".contains(&c))
            {
                return Err("release asset must be one plain filename".into());
            }
            let hash = digest(&file)?;
            if ![
                ".zip",
                ".tar.gz",
                ".deb",
                ".whl",
                ".zip.sha256",
                ".tar.gz.sha256",
            ]
            .iter()
            .any(|extension| name.ends_with(extension))
            {
                return Err(
                    "CI payload contains a file outside the release distribution contract".into(),
                );
            }
            if payload.insert(name, (file, hash)).is_some() {
                return Err("duplicate qualified release asset".into());
            }
        }
        evidence.push(json!({"name":name,"id":artifact["id"],"digest":artifact["digest"]}));
    }
    let provenance = directory.join("provenance.json");
    let asset_hashes: BTreeMap<_, _> = payload
        .iter()
        .map(|(name, (_, hash))| (name.clone(), hash.clone()))
        .collect();
    fs::write(
        &provenance,
        serde_json::to_string_pretty(
            // This immutable record describes the qualified product only.
            // A newer maintained publisher must reproduce it during recovery.
            &json!({"format":1,"repository":github.repository,"repository_id":configuration.github.repository_id,"commit":commit,"qualification_run_id":run_id,"artifacts":evidence,"assets":asset_hashes}),
        )?,
    )?;
    payload.insert(
        "provenance.json".into(),
        (provenance.clone(), digest(&provenance)?),
    );
    let sums = directory.join("SHA256SUMS");
    fs::write(
        &sums,
        payload
            .iter()
            .map(|(name, (_, hash))| format!("{hash}  {name}\n"))
            .collect::<String>(),
    )?;
    payload.insert("SHA256SUMS".into(), (sums.clone(), digest(&sums)?));
    if payload.len() > 99 {
        return Err("release exceeds its asset bound".into());
    }
    // No remote mutation occurs before every qualification and artifact check.
    let development = github.api(
        "GET",
        &format!("git/ref/heads/{}", configuration.github.development_branch),
        None,
    )?;
    if existing_tag.is_none() && development["object"]["sha"] != commit {
        return Err("a newer dev commit must finish qualification first".into());
    }
    let stable = github.api(
        "GET",
        &format!("git/ref/heads/{}", configuration.github.stable_branch),
        None,
    )?;
    let old = stable["object"]["sha"]
        .as_str()
        .ok_or("stable ref missing")?;
    let current = old == commit;
    let recovering_previous = !current
        && existing_tag.is_some()
        && Command::new("git")
            .args(["merge-base", "--is-ancestor", &commit, old])
            .status()?
            .success();
    if !current && !recovering_previous {
        if !Command::new("git")
            .args(["merge-base", "--is-ancestor", old, &commit])
            .status()?
            .success()
        {
            return Err("stable promotion would not fast-forward".into());
        }
        github.api(
            "PATCH",
            &format!("git/refs/heads/{}", configuration.github.stable_branch),
            Some(json!({"sha":commit,"force":false})),
        )?;
    }
    if existing_tag.is_none() {
        let object = github.api("POST", "git/tags", Some(json!({"tag":tag,"message":format!("NDDev Systeminfo {version}\nQualified CI: https://github.com/{}/actions/runs/{run_id}", github.repository),"object":commit,"type":"commit"})))?;
        github.api(
            "POST",
            "git/refs",
            Some(json!({"ref":format!("refs/tags/{tag}"),"sha":object["sha"]})),
        )?;
    }
    let release = if let Some(release) = existing_release {
        release
    } else {
        github.api("POST", "releases", Some(json!({"tag_name":tag,"target_commitish":commit,"name":format!("NDDev Systeminfo {version}"),"body":release_notes(&source_file(&commit,"CHANGELOG.md")?, &version)?,"draft":true,"prerelease":false})))?
    };
    let release_id = release["id"].as_u64().ok_or("release id missing")?;
    for (name, (path, hash)) in &payload {
        let assets = github.api(
            "GET",
            &format!("releases/{release_id}/assets?per_page=100"),
            None,
        )?;
        let matches: Vec<_> = assets
            .as_array()
            .ok_or("release assets missing")?
            .iter()
            .filter(|asset| asset["name"].as_str() == Some(name))
            .collect();
        if matches.len() > 1 {
            return Err("duplicate published asset identity".into());
        }
        if let Some(asset) = matches.first() {
            let copy = directory.join(format!("verify-{name}"));
            github.download(
                Download::ReleaseAsset(asset["id"].as_u64().ok_or("asset id missing")?),
                &copy,
            )?;
            if digest(&copy)? == *hash {
                continue;
            }
            if release["draft"] != true {
                return Err("immutable published asset differs from qualified bytes".into());
            }
            if github.api("GET", &format!("releases/{release_id}"), None)?["draft"] != true {
                return Err("release was published while its draft was being recovered".into());
            }
            github.api("DELETE", &format!("releases/assets/{}", asset["id"]), None)?;
        }
        if github.api("GET", &format!("releases/{release_id}"), None)?["draft"] != true {
            return Err("immutable published release is missing a qualified asset".into());
        }
        command(
            "gh",
            &[
                "release",
                "upload",
                &tag,
                path.to_str().unwrap(),
                "--repo",
                &github.repository,
            ],
        )?;
        let assets = github.api(
            "GET",
            &format!("releases/{release_id}/assets?per_page=100"),
            None,
        )?;
        let asset = assets
            .as_array()
            .ok_or("uploaded assets missing")?
            .iter()
            .find(|asset| asset["name"].as_str() == Some(name))
            .ok_or("uploaded asset missing")?;
        let copy = directory.join(format!("verify-{name}"));
        github.download(
            Download::ReleaseAsset(asset["id"].as_u64().ok_or("asset id missing")?),
            &copy,
        )?;
        if digest(&copy)? != *hash {
            return Err("uploaded release bytes differ from qualification".into());
        }
    }
    let assets = github.api(
        "GET",
        &format!("releases/{release_id}/assets?per_page=100"),
        None,
    )?;
    let assets = assets.as_array().ok_or("final release asset set missing")?;
    if assets.len() != payload.len()
        || assets.iter().any(|asset| {
            asset["name"]
                .as_str()
                .is_none_or(|name| !payload.contains_key(name))
        })
    {
        return Err("release contains assets outside its qualified distribution".into());
    }
    if release["draft"] == true {
        github.api(
            "PATCH",
            &format!("releases/{release_id}"),
            Some(
                json!({"draft":false,"make_latest":if recovering_previous {"false"} else {"true"}}),
            ),
        )?;
    }
    let latest = github.api("GET", "releases/latest", None)?["tag_name"] == tag;
    let output = std::env::var_os("GITHUB_OUTPUT").ok_or("delivery output missing")?;
    writeln!(
        fs::OpenOptions::new().append(true).open(output)?,
        "tag={tag}\ncommit={commit}\nlatest={latest}"
    )?;
    println!(
        "published verified artifacts: https://github.com/{}/releases/tag/{tag}",
        github.repository
    );
    Ok(())
}

/// Signing consumes the original qualified Debian bytes, independently of the
/// release's own checksum file. The private signing key is never read here.
pub(super) fn fetch_deb() -> Result {
    let tag = std::env::var("RELEASE_TAG")?;
    if !tag.starts_with('v')
        || tag[1..].split('.').count() != 3
        || !tag[1..].bytes().all(|c| c.is_ascii_digit() || c == b'.')
    {
        return Err("invalid stable release tag".into());
    }
    let (_, github) = configuration()?;
    let release = github.api("GET", &format!("releases/tags/{tag}"), None)?;
    if release["draft"] != false
        || release["prerelease"] != false
        || github.api("GET", "releases/latest", None)?["tag_name"] != tag
    {
        return Err("APT publication requires the latest published stable release".into());
    }
    fs::create_dir_all("released")?;
    command(
        "gh",
        &[
            "release",
            "download",
            &tag,
            "--repo",
            &github.repository,
            "--dir",
            "released",
            "--pattern",
            "*.deb",
        ],
    )?;
    for file in files(Path::new("released"))? {
        let output = Command::new("dpkg-deb")
            .arg("--field")
            .arg(file)
            .arg("Version")
            .output()?;
        if !output.status.success() || String::from_utf8(output.stdout)?.trim() != &tag[1..] {
            return Err("Debian package version differs from released tag".into());
        }
    }
    Ok(())
}
pub(super) fn verify_deb() -> Result {
    verify_distribution("debian-amd64", "deb")
}
pub(super) fn verify_python() -> Result {
    verify_distribution("python-sdk", "python")
}
fn verify_distribution(artifact_name: &str, distribution: &str) -> Result {
    let tag = std::env::var("RELEASE_TAG")?;
    if !tag.starts_with('v') || !tag[1..].bytes().all(|c| c.is_ascii_digit() || c == b'.') {
        return Err("invalid stable release tag".into());
    }
    let (configuration, github) = configuration()?;
    let release = github.api("GET", &format!("releases/tags/{tag}"), None)?;
    if release["draft"] != false
        || release["prerelease"] != false
        || github.api("GET", "releases/latest", None)?["tag_name"] != tag
    {
        return Err("distribution publication requires the latest published stable release".into());
    }
    let commit = tag_commit(
        &github,
        &github.api("GET", &format!("git/ref/tags/{tag}"), None)?,
    )?;
    let assets = release["assets"]
        .as_array()
        .ok_or("release assets missing")?;
    let provenance = assets
        .iter()
        .find(|asset| asset["name"] == "provenance.json")
        .ok_or("release provenance missing")?;
    let directory = Path::new(".tools/distribution-verification").join(artifact_name);
    fs::create_dir_all(&directory)?;
    let record = directory.join("provenance.json");
    github.download(
        Download::ReleaseAsset(provenance["id"].as_u64().ok_or("provenance id missing")?),
        &record,
    )?;
    let provenance: Value = serde_json::from_slice(&fs::read(record)?)?;
    if provenance["format"] != 1
        || provenance["repository_id"].as_u64() != Some(configuration.github.repository_id)
        || provenance["repository"].as_str() != Some(&github.repository)
        || provenance["commit"] != commit
    {
        return Err("release provenance differs from the original repository and tag".into());
    }
    let run_id = provenance["qualification_run_id"]
        .as_u64()
        .ok_or("qualification run missing")?;
    let run = github.api("GET", &format!("actions/runs/{run_id}"), None)?;
    let jobs = github.pages(&format!("actions/runs/{run_id}/jobs"), "jobs")?;
    qualified(&run, &jobs, &configuration, &commit, &publisher_prefix()?)?;
    let available = github.pages(&format!("actions/runs/{run_id}/artifacts"), "artifacts")?;
    let original: Vec<_> = available
        .iter()
        .filter(|artifact| artifact["name"] == artifact_name)
        .collect();
    if original.len() != 1
        || original[0]["workflow_run"]["head_sha"] != commit
        || original[0]["workflow_run"]["head_repository_id"].as_u64()
            != Some(configuration.github.repository_id)
    {
        return Err("original qualified distribution is missing or ambiguous".into());
    }
    let target = directory.join("qualified");
    if target.exists() {
        fs::remove_dir_all(&target)?;
    }
    unpack(&github, original[0], &target)?;
    let packages = files(&target)?;
    if packages.len() != 2 {
        return Err("qualified distribution must contain both packages".into());
    }
    for original in packages {
        let name = original
            .file_name()
            .ok_or("qualified Debian filename missing")?;
        let filename = name.to_str().ok_or("distribution filename must be UTF-8")?;
        if !(distribution == "deb" && filename.ends_with(".deb")
            || distribution == "python"
                && (filename.ends_with(".whl") || filename.ends_with(".tar.gz")))
        {
            return Err("unexpected distribution artifact member".into());
        }
        let released = Path::new("released").join(name);
        if digest(&original)? != digest(&released)? {
            return Err("released distribution bytes differ from original successful CI".into());
        }
    }
    println!("verified {artifact_name} bytes against original qualified CI {run_id} ({commit})");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_endpoints_keep_their_distinct_github_media_contracts() {
        assert_eq!(
            Download::CiArchive(42).request(),
            (
                "actions/artifacts/42/zip".into(),
                "Accept: application/json"
            )
        );
        assert_eq!(
            Download::ReleaseAsset(42).request(),
            (
                "releases/assets/42".into(),
                "Accept: application/octet-stream"
            )
        );
    }
    #[test]
    fn repository_identity_uses_the_real_github_root_route() {
        let github = Github {
            repository: "example/project".into(),
        };
        assert_eq!(github.resource(""), "repos/example/project");
        assert_eq!(
            github.resource("actions/runs/42/jobs?per_page=100&page=1"),
            "repos/example/project/actions/runs/42/jobs?per_page=100&page=1"
        );
    }
    #[test]
    fn publication_rejects_other_sources_and_incomplete_qualification() {
        let config: Configuration = toml::from_str(include_str!("../../release.toml")).unwrap();
        let sha = "d".repeat(40);
        let run = json!({"event":"push", "head_branch":config.github.development_branch, "head_sha":sha, "workflow_id":config.github.workflow_id, "repository":{"id":config.github.repository_id}, "head_repository":{"id":config.github.repository_id}, "path":".github/workflows/ci.yml"});
        let jobs = vec![
            json!({"name":"Daemon (windows-2025)","status":"completed","conclusion":"success"}),
            json!({"name":"check / merge stops","status":"completed","conclusion":"skipped"}),
            json!({"name":"Release / Publish verified artifacts","status":"in_progress","conclusion":null}),
        ];
        assert!(qualified(&run, &jobs, &config, &sha, "Release / ").is_ok());
        for (pointer, invalid) in [
            ("/event", json!("pull_request")),
            ("/head_branch", json!("main")),
            ("/head_repository/id", json!(42)),
            ("/repository/id", json!(42)),
            ("/head_sha", json!("a".repeat(40))),
            ("/workflow_id", json!(0)),
            ("/path", json!(".github/workflows/untrusted.yml")),
        ] {
            let mut changed = run.clone();
            *changed.pointer_mut(pointer).unwrap() = invalid;
            assert!(
                qualified(&changed, &jobs, &config, &sha, "Release / ").is_err(),
                "accepted {pointer}"
            );
        }
        for conclusion in ["failure", "cancelled", "skipped"] {
            let mut changed = jobs.clone();
            changed[0]["conclusion"] = json!(conclusion);
            assert!(qualified(&run, &changed, &config, &sha, "Release / ").is_err());
        }
        assert!(qualified(&run, &[], &config, &sha, "Release / ").is_err());
    }
    #[test]
    fn notes_only_contain_the_released_version() {
        assert_eq!(release_notes("# Changelog\n\n## 0.3.2 — today\n\nNew behavior.\n\n## 0.3.1 — before\nOld behavior.","0.3.2").unwrap(),"## 0.3.2 — today\n\nNew behavior.");
        assert!(release_notes("# Changelog\n## 0.3.1 — before", "0.3.2").is_err());
    }
}
