use serde_json::Value;
use std::{env, fmt::Write, fs, path::PathBuf};

fn main() {
    let source = "../../contracts/snapshot-v1.schema.json";
    println!("cargo:rerun-if-changed={source}");
    let schema: Value = serde_json::from_str(&fs::read_to_string(source).unwrap()).unwrap();
    let version = schema["properties"]["v"]["const"].as_u64().unwrap();
    let maximum = schema["x-max-frame-bytes"].as_u64().unwrap();
    let max_interval = schema["x-cadence"]["maximum-seconds"].as_u64().unwrap();
    let realtime_ms = schema["x-cadence"]["realtime-ms"].as_u64().unwrap();
    let groups = schema["x-groups"].as_array().unwrap();
    let mut flat = format!(
        "pub const PROTOCOL_VERSION: u32 = {version};\npub const MAX_FRAME_BYTES: usize = {maximum};\npub const MAX_INTERVAL: u64 = {max_interval};\npub const REALTIME_MS: u64 = {realtime_ms};\n#[derive(Clone, Copy, Default)]\npub struct Snapshot {{\n"
    );
    let mut wire = String::from(
        "#[derive(Clone, serde::Deserialize, serde::Serialize)]\npub struct WireSnapshot {\n    pub v: u32,\n",
    );
    let mut types = String::new();
    let mut encode = format!(
        "impl Snapshot {{\n    pub fn encode(&self, out: &mut String) {{\n        out.clear();\n        out.push_str({:?});\n",
        format!("{{\"v\":{version}")
    );
    let mut validation = String::from(
        "impl WireSnapshot {\n    pub fn validate(&self) -> Result<(), ProtocolError> {\n        if self.v != PROTOCOL_VERSION { return Err(ProtocolError::UnsupportedVersion); }\n",
    );
    for group in groups {
        let name = group.as_str().unwrap();
        let definition = &schema["properties"][name];
        let mut chars = name.chars();
        let title = chars.next().unwrap().to_uppercase().to_string() + chars.as_str();
        writeln!(wire, "    pub {name}: {title},").unwrap();
        writeln!(
            types,
            "#[derive(Clone, serde::Deserialize, serde::Serialize)]\npub struct {title} {{"
        )
        .unwrap();
        writeln!(
            encode,
            "        out.push_str({:?});",
            format!(",\"{name}\":{{")
        )
        .unwrap();
        for (index, key) in definition["x-fields"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let key = key.as_str().unwrap();
            let leaf = &definition["properties"][key];
            let field = leaf["x-field"].as_str().unwrap();
            let integer = leaf["type"][0] == "integer";
            let flat_type = if integer { "u64" } else { "f32" };
            let wire_type = if integer { "u64" } else { "f64" };
            writeln!(flat, "    pub {field}: Option<{flat_type}>,").unwrap();
            writeln!(types, "    #[serde(deserialize_with = \"Option::deserialize\")]\n    pub {key}: Option<{wire_type}>,").unwrap();
            let prefix = format!("{}\"{key}\":", if index == 0 { "" } else { "," });
            writeln!(encode, "        out.push_str({prefix:?});").unwrap();
            let input = if leaf["x-unit"] == "percent" {
                format!("self.{field}.filter(|value| (0.0..=100.0).contains(value))")
            } else {
                format!("self.{field}")
            };
            writeln!(
                encode,
                "        {}(out, {input});",
                if integer { "counter" } else { "decimal" }
            )
            .unwrap();
            if !integer {
                let bounds = if leaf["x-unit"] == "percent" {
                    " || !(0.0..=100.0).contains(&value)"
                } else {
                    ""
                };
                writeln!(validation, "        if self.{name}.{key}.is_some_and(|value| !value.is_finite(){bounds}) {{ return Err(ProtocolError::InvalidMetric); }}").unwrap();
            }
        }
        types.push_str("}\n");
        encode.push_str("        out.push('}');\n");
    }
    flat.push_str("}\n");
    wire.push_str("}\n");
    encode.push_str("        out.push_str(\"}\\n\");\n    }\n}\n");
    validation.push_str("        Ok(())\n    }\n}\n");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("snapshot.rs");
    fs::write(output, flat + &wire + &types + &encode + &validation).unwrap();
    analysis();
}

fn analysis() {
    let path = "../../contracts/analysis-v1.schema.json";
    println!("cargo:rerun-if-changed={path}");
    let schema: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let mut output = String::new();
    for (name, key) in [
        ("MAX_ANALYSIS_FRAME_BYTES", "x-max-frame-bytes"),
        ("MAX_CORES", "x-max-cores"),
        ("MAX_GPUS", "x-max-gpus"),
    ] {
        writeln!(output, "pub const {name}: usize = {};", schema[key]).unwrap();
    }
    for (name, key) in [
        ("HISTORY_SECONDS", "retention-seconds"),
        ("HISTORY_RECORDS", "max-records"),
        ("HISTORY_BYTES", "max-bytes"),
        ("PLOT_COLUMNS", "plot-columns"),
    ] {
        writeln!(
            output,
            "pub const {name}: usize = {};",
            schema["x-history"][key]
        )
        .unwrap();
    }
    for (name, definition) in schema["$defs"].as_object().unwrap() {
        writeln!(
            output,
            "#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]\npub struct {name} {{"
        )
        .unwrap();
        for (field, value) in definition["properties"].as_object().unwrap() {
            let (kind, nullable) = analysis_type(value);
            if nullable {
                output.push_str("    #[serde(deserialize_with = \"Option::deserialize\")]\n");
            }
            writeln!(
                output,
                "    pub {field}: {},",
                if nullable {
                    format!("Option<{kind}>")
                } else {
                    kind
                }
            )
            .unwrap();
        }
        output.push_str("}\n");
    }
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("analysis.rs"),
        output,
    )
    .unwrap();
}
fn analysis_type(value: &Value) -> (String, bool) {
    if let Some(reference) = value["$ref"].as_str() {
        return (reference.rsplit('/').next().unwrap().into(), false);
    }
    let nullable = value["type"].is_array();
    let kind = if nullable {
        value["type"][0].as_str().unwrap()
    } else {
        value["type"].as_str().unwrap()
    };
    let result = match kind {
        "integer" => value["x-rust-type"].as_str().unwrap_or("u64").into(),
        "number" => "f64".into(),
        "string" => "String".into(),
        "boolean" => "bool".into(),
        "array" => format!("Vec<{}>", analysis_type(&value["items"]).0),
        _ => panic!("unsupported analysis schema type"),
    };
    (result, nullable)
}
