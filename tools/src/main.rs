//! Reproducible source checks, tools and packaging. Run builds on a build host.
use serde::Deserialize;
mod install_test;
mod linux_ui;
mod packaging;
mod resources;
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
    Ok(nddev_systeminfo::installation::digest(path)?)
}
fn release_asset(source: &str, destination: &Path) -> Result {
    let (repository, release) = source
        .strip_prefix("https://github.com/")
        .and_then(|source| source.split_once("/releases/download/"))
        .ok_or("tool source must be an official GitHub release")?;
    let (tag, name) = release.split_once('/').ok_or("invalid release source")?;
    let request = |url: &str, accept: &str| {
        let mut curl = Command::new("curl");
        curl.args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--connect-timeout",
            "10",
            "--max-time",
            "60",
            "--header",
            accept,
        ]);
        // CI supplies a contents-read token. It never enters error arguments or
        // a receipt; curl strips Authorization when following a foreign host.
        if let Some(token) = std::env::var_os("GH_TOKEN") {
            let mut header = std::ffi::OsString::from("Authorization: Bearer ");
            header.push(token);
            curl.arg("--header").arg(header);
        }
        curl.arg(url);
        curl
    };
    let metadata = request(
        &format!("https://api.github.com/repos/{repository}/releases/tags/{tag}"),
        "Accept: application/vnd.github+json",
    )
    .output()?;
    if !metadata.status.success() {
        return Err(format!(
            "cannot read tool release: {repository}@{tag} ({})",
            metadata.status
        )
        .into());
    }
    let metadata: serde_json::Value = serde_json::from_slice(&metadata.stdout)?;
    let assets: Vec<_> = metadata["assets"]
        .as_array()
        .ok_or("release assets missing")?
        .iter()
        .filter(|asset| asset["name"].as_str() == Some(name))
        .collect();
    if assets.len() != 1 {
        return Err("tool release asset is missing or ambiguous".into());
    }
    let id = assets[0]["id"].as_u64().ok_or("invalid release asset id")?;
    let result = request(
        &format!("https://api.github.com/repos/{repository}/releases/assets/{id}"),
        "Accept: application/octet-stream",
    )
    .arg("--output")
    .arg(destination)
    .output()?;
    if !result.status.success() {
        return Err(format!(
            "cannot fetch tool asset: {repository}@{tag}/{name} ({})",
            result.status
        )
        .into());
    }
    Ok(())
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
            && format!("{} {}", artifact.sha256, digest(&destination)?)
                == fs::read_to_string(&record)?.trim()
        {
            continue;
        }
        let folder = Path::new(".tools/downloads").join(&tool.name);
        if folder.exists() {
            fs::remove_dir_all(&folder)?;
        }
        fs::create_dir_all(&folder)?;
        let archive = folder.join("tool.tar.gz");
        release_asset(&artifact.url, &archive)?;
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
        fs::write(
            record,
            format!("{} {}", artifact.sha256, digest(&destination)?),
        )?;
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
    fs::create_dir_all("extension/resources")?;
    fs::write("extension/resources/ui.json", ui_resource()?)?;
    fs::write("extension/lib/contract.js", js_contract()?)?;
    fs::write("macos/Sample.swift", swift_contract()?)?;
    fs::create_dir_all("assets")?;
    fs::write("assets/brand-mask-64.bin", brand_mask()?)?;
    fs::write("extension/resources/brand-symbolic.svg", brand_svg())?;
    Ok(())
}
fn brand_svg() -> String {
    use nddev_opennetwork_design_tokens::BRAND_MARK;
    format!(
        "<!-- Generated from the pinned OpenNetwork brand mark. -->\n<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{} {} {} {}\">{}</svg>\n",
        BRAND_MARK.min_x,
        BRAND_MARK.min_y,
        BRAND_MARK.width,
        BRAND_MARK.height,
        BRAND_MARK
            .shapes
            .iter()
            .map(|shape| format!("<path fill=\"currentColor\" d=\"{}\"/>", shape.d))
            .collect::<String>()
    )
}
fn brand_mask() -> Result<Vec<u8>> {
    use nddev_opennetwork_design_tokens::BRAND_MARK;
    let mut polygons = Vec::new();
    for shape in BRAND_MARK.shapes {
        if shape
            .d
            .chars()
            .any(|c| c.is_alphabetic() && !['M', 'L', 'Z'].contains(&c))
        {
            return Err("brand projection requires the released polygon geometry".into());
        }
        let numbers: Vec<f64> = shape
            .d
            .replace(['M', 'L', 'Z', ','], " ")
            .split_whitespace()
            .map(str::parse)
            .collect::<std::result::Result<_, _>>()?;
        if !numbers.len().is_multiple_of(2) {
            return Err("invalid brand polygon".into());
        }
        polygons.push(
            numbers
                .as_chunks::<2>()
                .0
                .iter()
                .map(|point| (point[0], point[1]))
                .collect::<Vec<_>>(),
        );
    }
    let mut mask = vec![0; 64 * 64];
    for y in 0..64 {
        for x in 0..64 {
            let mut coverage = 0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let px = f64::from(BRAND_MARK.min_x)
                        + (x as f64 + (f64::from(sx) + 0.5) / 4.0) * f64::from(BRAND_MARK.width)
                            / 64.0;
                    let py = f64::from(BRAND_MARK.min_y)
                        + (y as f64 + (f64::from(sy) + 0.5) / 4.0) * f64::from(BRAND_MARK.height)
                            / 64.0;
                    if polygons.iter().any(|polygon| {
                        let mut inside = false;
                        let mut previous = *polygon.last().expect("nonempty released polygon");
                        for &(a, b) in polygon {
                            if (b > py) != (previous.1 > py)
                                && px < (previous.0 - a) * (py - b) / (previous.1 - b) + a
                            {
                                inside = !inside;
                            }
                            previous = (a, b);
                        }
                        inside
                    }) {
                        coverage += 1;
                    }
                }
            }
            mask[y * 64 + x] = (coverage * 255 / 16) as u8;
        }
    }
    Ok(mask)
}
fn js_contract() -> Result<String> {
    use serde_json::{Map, Value};
    let schema: Value =
        serde_json::from_str(&fs::read_to_string("contracts/snapshot-v1.schema.json")?)?;
    let mut metrics = Map::new();
    for group in schema["x-groups"]
        .as_array()
        .ok_or("schema groups missing")?
    {
        let group = group.as_str().ok_or("invalid schema group")?;
        let mut fields = Map::new();
        for key in schema["properties"][group]["x-fields"]
            .as_array()
            .ok_or("schema fields missing")?
        {
            let key = key.as_str().ok_or("invalid field")?;
            let field = &schema["properties"][group]["properties"][key];
            let kind = if field["type"][0] == "integer" {
                "integer"
            } else if field["x-unit"] == "percent" {
                "percent"
            } else {
                "number"
            };
            fields.insert(key.into(), kind.into());
        }
        metrics.insert(group.into(), fields.into());
    }
    Ok(format!(
        "// Generated from contracts/snapshot-v1.schema.json; do not edit.\nexport const PROTOCOL_VERSION = {};\nexport const MAX_LINE = {};\nexport const MAX_INTERVAL = {};\nexport const REALTIME_MS = {};\nexport const METRICS = {};\n",
        schema["properties"]["v"]["const"],
        schema["x-max-frame-bytes"],
        schema["x-cadence"]["maximum-seconds"],
        schema["x-cadence"]["realtime-ms"],
        serde_json::to_string_pretty(&metrics)?
    ))
}
fn swift_contract() -> Result<String> {
    use std::fmt::Write;
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string("contracts/snapshot-v1.schema.json")?)?;
    let mut result = format!(
        "// Generated from contracts/snapshot-v1.schema.json; do not edit.\nimport Foundation\n\nenum WireContract {{\n    static let version = {}\n    static let maxFrame = {}\n    static let maxInterval = {}\n    static let realtimeMs = {}\n}}\n\nstruct MetricSample: Codable, Sendable {{\n",
        schema["properties"]["v"]["const"],
        schema["x-max-frame-bytes"],
        schema["x-cadence"]["maximum-seconds"],
        schema["x-cadence"]["realtime-ms"]
    );
    let groups = schema["x-groups"]
        .as_array()
        .ok_or("schema groups missing")?;
    for group in groups {
        let name = group.as_str().ok_or("invalid group")?;
        let title = name.to_uppercase();
        writeln!(result, "    struct {title}: Codable, Sendable {{")?;
        let fields = schema["properties"][name]["x-fields"]
            .as_array()
            .ok_or("schema fields missing")?;
        writeln!(
            result,
            "        enum CodingKeys: String, CodingKey {{ case {} }}",
            fields
                .iter()
                .map(|key| key.as_str().ok_or("invalid field"))
                .collect::<std::result::Result<Vec<_>, _>>()?
                .join(", ")
        )?;
        for key in fields {
            let key = key.as_str().ok_or("invalid field")?;
            let kind = if schema["properties"][name]["properties"][key]["type"][0] == "integer" {
                "UInt64"
            } else {
                "Double"
            };
            writeln!(result, "        let {key}: {kind}?")?;
        }
        result.push_str("        init(from decoder: any Decoder) throws {\n            let values = try decoder.container(keyedBy: CodingKeys.self)\n");
        for key in fields {
            let key = key.as_str().ok_or("invalid field")?;
            let field = &schema["properties"][name]["properties"][key];
            let integer = field["type"][0] == "integer";
            let kind = if integer { "UInt64" } else { "Double" };
            writeln!(
                result,
                "            {key} = try values.decode({kind}?.self, forKey: .{key})"
            )?;
            if !integer {
                let range = if field["x-unit"] == "percent" {
                    " && (0...100).contains(value)"
                } else {
                    ""
                };
                writeln!(
                    result,
                    "            if let value = {key}, !(value.isFinite{range}) {{\n                throw DecodingError.dataCorruptedError(forKey: .{key}, in: values, debugDescription: \"Invalid metric\")\n            }}"
                )?;
            }
        }
        result.push_str("        }\n        func encode(to encoder: any Encoder) throws {\n            var values = encoder.container(keyedBy: CodingKeys.self)\n");
        for key in fields {
            let key = key.as_str().ok_or("invalid field")?;
            writeln!(
                result,
                "            try values.encode({key}, forKey: .{key})"
            )?;
        }
        result.push_str("        }\n    }\n");
    }
    result.push_str("    let v: Int\n");
    for group in groups {
        let name = group.as_str().ok_or("invalid group")?;
        writeln!(result, "    let {name}: {}", name.to_uppercase())?;
    }
    result.push_str("    init(from decoder: any Decoder) throws {\n        let values = try decoder.container(keyedBy: CodingKeys.self)\n        v = try values.decode(Int.self, forKey: .v)\n        guard v == WireContract.version else {\n            throw DecodingError.dataCorruptedError(forKey: .v, in: values, debugDescription: \"Unsupported protocol version\")\n        }\n");
    for group in groups {
        let name = group.as_str().ok_or("invalid group")?;
        writeln!(
            result,
            "        {name} = try values.decode({}.self, forKey: .{name})",
            name.to_uppercase()
        )?;
    }
    result.push_str("    }\n}\n");
    Ok(result)
}
fn ui_resource() -> Result<String> {
    use nddev_opennetwork_design_tokens::{
        BRAND_MARK, Color, Colors, Density, Direction, Metrics, Platform, Script, TextStyle, Theme,
        TypeRole,
    };
    use serde_json::{Map, Value, json};
    let mut catalogs = Map::new();
    let english: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string("locales/en.json")?)?;
    for code in ["en", "ru", "kk", "ar", "zh"] {
        let source = fs::read_to_string(format!("locales/{code}.json"))?;
        let catalog: BTreeMap<String, String> = serde_json::from_str(&source)?;
        if !catalog.keys().eq(english.keys()) || catalog.values().any(|text| text.trim().is_empty())
        {
            return Err(format!("incomplete locale: {code}").into());
        }
        for (key, value) in &catalog {
            if value.contains("{seconds}") != english[key].contains("{seconds}") {
                return Err(format!("locale placeholder mismatch: {code}/{key}").into());
            }
        }
        catalogs.insert(code.into(), serde_json::to_value(catalog)?);
    }
    let source: toml::Value = toml::from_str(&fs::read_to_string("design-system.toml")?)?;
    let manifest: toml::Value = toml::from_str(&fs::read_to_string("Cargo.toml")?)?;
    let dependency = &manifest["workspace"]["dependencies"]["nddev-opennetwork-design-tokens"];
    if dependency["git"] != source["repository"] || dependency["rev"] != source["revision"] {
        return Err("design package pin drifted".into());
    }
    let hex = |color: Color| format!("#{:02X}{:02X}{:02X}", color.r, color.g, color.b);
    let mut themes = Map::new();
    for theme in Theme::ALL {
        let c = Colors::resolve(theme, Direction::On);
        themes.insert(theme.id().into(), json!({"surface":hex(c.surface_base), "raised":hex(c.surface_raised), "text":hex(c.text_primary), "secondary":hex(c.text_secondary), "border":hex(c.border_control), "accent":hex(c.accent_text), "focus":hex(c.focus_ring)}));
    }
    let mut metrics = Map::new();
    for density in Density::ALL {
        let m = Metrics::resolve(Platform::Desktop, density);
        metrics.insert(density.id().into(), json!({"gap":m.space_2,"small":m.space_1,"padding":m.space_4,"row_padding":m.space_3,"radius":m.radius_md,"icon":m.size_icon,"brand":m.size_brand,"min_width":m.size_featured_card_min,"control":m.size_control_sm,"border":m.border_width_thin}));
    }
    let mut typography = Map::new();
    for (code, script) in [
        ("en", Script::Latin),
        ("ru", Script::Cyrillic),
        ("kk", Script::Cyrillic),
        ("ar", Script::Arabic),
        ("zh", Script::Han),
    ] {
        let body = TextStyle::resolve(TypeRole::Body, Platform::Desktop, script);
        let label = TextStyle::resolve(TypeRole::Label, Platform::Desktop, script);
        typography.insert(
            code.into(),
            json!({"body":body.size,"label":label.size,"line_height":body.line_height}),
        );
    }
    let defaults: toml::Value = toml::from_str(&fs::read_to_string("config/default.toml")?)?;
    let data = json!({"version":version()?,"source":source,"defaults":defaults,"catalogs":catalogs,"themes":themes,"metrics":metrics,"typography":typography,"brand":{"view_box":[BRAND_MARK.min_x,BRAND_MARK.min_y,BRAND_MARK.width,BRAND_MARK.height],"paths":BRAND_MARK.shapes.iter().map(|shape| Value::String(shape.d.into())).collect::<Vec<_>>()}});
    Ok(serde_json::to_string_pretty(&data)? + "\n")
}
fn check() -> Result {
    let version = version()?;
    if fs::read_to_string("extension/resources/ui.json")? != ui_resource()? {
        return Err("native resources drifted; run just generate".into());
    }
    if fs::read_to_string("extension/lib/contract.js")? != js_contract()? {
        return Err("native wire contract drifted; run just generate".into());
    }
    if fs::read_to_string("macos/Sample.swift")? != swift_contract()? {
        return Err("native Swift wire contract drifted; run just generate".into());
    }
    if fs::read("assets/brand-mask-64.bin")? != brand_mask()? {
        return Err("native icon mask drifted; run just generate".into());
    }
    if fs::read_to_string("extension/resources/brand-symbolic.svg")? != brand_svg() {
        return Err("native brand mark drifted; run just generate".into());
    }
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

fn build_targets(
    package: &str,
    binary: Option<&str>,
    release: bool,
) -> Result<BTreeMap<String, PathBuf>> {
    use std::io::{BufRead, BufReader};
    let mut cargo = Command::new("cargo");
    cargo.args([
        "build",
        "--locked",
        "--package",
        package,
        "--all-features",
        "--message-format=json",
    ]);
    if let Some(binary) = binary {
        cargo.args(["--bin", binary]);
    } else {
        cargo.arg("--bins");
    }
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
fn build_binaries(release: bool) -> Result<BTreeMap<String, PathBuf>> {
    // A running tool must never rebuild/replace its own executable on Windows.
    // Product and acceptance targets have separate, explicit build receipts.
    let binaries = build_targets("nddev-systeminfo", None, release)?;
    fs::create_dir_all(".tools")?;
    fs::write(
        if release {
            ".tools/build-release.json"
        } else {
            ".tools/build-debug.json"
        },
        serde_json::to_string_pretty(&binaries)?,
    )?;
    Ok(binaries)
}
fn run() -> Result {
    match std::env::args().nth(1).as_deref() {
        Some("setup") => setup(),
        Some("generate") => generate(),
        Some("check") => check(),
        Some("test") => test(),
        Some("ui-linux") => linux_ui::run(),
        Some("ui-linux-session") => linux_ui::session(),
        Some("ui-windows") => {
            let binaries = build_binaries(false)?;
            let acceptance =
                build_targets("systeminfo-tools", Some("windows-ui-acceptance"), false)?;
            let status = Command::new(&acceptance["windows-ui-acceptance"])
                .env(
                    "CARGO_BIN_EXE_nddev-systeminfod",
                    &binaries["nddev-systeminfod"],
                )
                .status()?;
            if status.success() {
                Ok(())
            } else {
                Err("native Windows UI acceptance failed".into())
            }
        }
        Some("measure") => resources::measure(),
        Some("measure-legacy") => resources::legacy(Path::new(
            &std::env::args()
                .nth(2)
                .ok_or("legacy measurement needs the original binary path")?,
        )),
        Some("package") => packaging::package().map(|_| ()),
        Some("install-test") => install_test::run(),
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
