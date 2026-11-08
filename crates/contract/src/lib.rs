//! Generated v1 types; the JSON Schema is the authoritative public contract.
use serde::Deserialize;
use std::fmt::Write;

/// Machine-readable public contract.
pub const SCHEMA: &str = include_str!("../../../contracts/snapshot-v1.schema.json");

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("invalid or oversized systeminfo frame")]
    InvalidFrame,
    #[error("unsupported systeminfo protocol version")]
    UnsupportedVersion,
    #[error("invalid metric in systeminfo sample")]
    InvalidMetric,
}

include!(concat!(env!("OUT_DIR"), "/snapshot.rs"));

/// Check outside data once, including mandatory nullable fields.
pub fn decode(bytes: &[u8]) -> Result<WireSnapshot, ProtocolError> {
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let sample =
        WireSnapshot::deserialize(&mut decoder).map_err(|_| ProtocolError::InvalidFrame)?;
    decoder.end().map_err(|_| ProtocolError::InvalidFrame)?;
    sample.validate()?;
    Ok(sample)
}

fn decimal(out: &mut String, value: Option<f32>) {
    match value {
        Some(value) if value.is_finite() => {
            let _ = write!(out, "{value:.1}");
        }
        _ => out.push_str("null"),
    }
}
fn counter(out: &mut String, value: Option<u64>) {
    match value {
        Some(value) => {
            let _ = write!(out, "{value}");
        }
        None => out.push_str("null"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_every_metric() {
        let snapshot = Snapshot {
            cpu: Some(55.44),
            cpu_temperature: Some(88.6),
            memory: Some(54.7),
            // Exactly representable and exactly halfway, so it pins the
            // rounding rule: Rust formats ties to even, giving 4.2.
            swap: Some(4.25),
            gpu: Some(7.0),
            gpu_memory: Some(9.6),
            gpu_temperature: Some(69.0),
            disk_read: Some(10628),
            disk_write: Some(3789480),
            disk_temperature: Some(47.8),
            net_rx: Some(18360),
            net_tx: Some(342578),
        };

        let mut out = String::new();
        snapshot.encode(&mut out);

        assert_eq!(
            out,
            concat!(
                r#"{"v":1,"cpu":{"usage":55.4,"temp":88.6},"#,
                r#""memory":{"used":54.7,"swap":4.2},"#,
                r#""gpu":{"usage":7.0,"memory":9.6,"temp":69.0},"#,
                r#""disk":{"read":10628,"write":3789480,"temp":47.8},"#,
                r#""net":{"rx":18360,"tx":342578}}"#,
                "\n"
            )
        );
    }

    #[test]
    fn absent_metrics_are_null_not_missing() {
        let mut out = String::new();
        Snapshot::default().encode(&mut out);

        assert!(out.starts_with(r#"{"v":1,"cpu":{"usage":null,"temp":null}"#));
        assert!(out.contains(r#""gpu":{"usage":null,"memory":null,"temp":null}"#));
        assert!(out.ends_with("}\n"));
        // A client must never have to tell "unsupported" from "absent key".
        assert_eq!(out.matches("null").count(), 12);
    }

    #[test]
    fn a_non_finite_reading_is_reported_as_absent() {
        let mut out = String::new();
        Snapshot {
            cpu: Some(f32::NAN),
            ..Snapshot::default()
        }
        .encode(&mut out);
        assert!(out.contains(r#""usage":null"#));
    }

    #[test]
    fn encoding_reuses_the_buffer() {
        let mut out = String::from("stale contents");
        Snapshot::default().encode(&mut out);
        assert!(!out.contains("stale"));
    }

    #[test]
    fn decoder_requires_every_nullable_field_and_rejects_invalid_metrics() {
        let mut line = String::new();
        Snapshot::default().encode(&mut line);
        let original: serde_json::Value = serde_json::from_str(&line).unwrap();
        for (group, fields) in [
            ("cpu", vec!["usage", "temp"]),
            ("memory", vec!["used", "swap"]),
            ("gpu", vec!["usage", "memory", "temp"]),
            ("disk", vec!["read", "write", "temp"]),
            ("net", vec!["rx", "tx"]),
        ] {
            for field in fields {
                let mut value = original.clone();
                value[group].as_object_mut().unwrap().remove(field);
                assert!(decode(&serde_json::to_vec(&value).unwrap()).is_err());
            }
        }
        for invalid in [-1.0, 100.1] {
            let mut value = original.clone();
            value["cpu"]["usage"] = invalid.into();
            assert!(matches!(
                decode(&serde_json::to_vec(&value).unwrap()),
                Err(ProtocolError::InvalidMetric)
            ));
        }
        let mut version = original.clone();
        version["v"] = 2.into();
        assert!(matches!(
            decode(&serde_json::to_vec(&version).unwrap()),
            Err(ProtocolError::UnsupportedVersion)
        ));
        let mut future = original;
        future["future"] = true.into();
        assert!(decode(&serde_json::to_vec(&future).unwrap()).is_ok());
        assert!(decode(&vec![b' '; MAX_FRAME_BYTES + 1]).is_err());
        assert!(decode(format!("{line}{{}}").as_bytes()).is_err());
    }
}
