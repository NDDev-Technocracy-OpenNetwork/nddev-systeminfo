//! Negotiated details reuse the stable aggregate sample without enlarging its
//! ordinary wire frame. Limits and plain data types come from the schema.
use super::{ProtocolError, Snapshot, WireSnapshot};
use serde::Deserialize;
use std::collections::BTreeSet;

pub const SCHEMA: &str = include_str!("../../../contracts/analysis-v1.schema.json");
include!(concat!(env!("OUT_DIR"), "/analysis.rs"));

#[derive(Clone, serde::Deserialize, serde::Serialize)]
pub struct Frame {
    #[serde(flatten)]
    pub sample: WireSnapshot,
    pub detail: Details,
}

pub fn decode(bytes: &[u8]) -> Result<Frame, ProtocolError> {
    if bytes.len() > MAX_ANALYSIS_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let mut decoder = serde_json::Deserializer::from_slice(bytes);
    let frame = Frame::deserialize(&mut decoder).map_err(|_| ProtocolError::InvalidFrame)?;
    decoder.end().map_err(|_| ProtocolError::InvalidFrame)?;
    frame.sample.validate()?;
    frame.detail.validate()?;
    Ok(frame)
}

impl Details {
    pub fn validate(&self) -> Result<(), ProtocolError> {
        let percent =
            |v: Option<f64>| v.is_none_or(|v| v.is_finite() && (0.0..=100.0).contains(&v));
        if self.cores.len() > MAX_CORES
            || self.gpus.len() > MAX_GPUS
            || !(500..=60000).contains(&self.period_ms)
        {
            return Err(ProtocolError::InvalidFrame);
        }
        let mut cores = BTreeSet::new();
        if self
            .cores
            .iter()
            .any(|v| !cores.insert(v.id) || !percent(v.usage))
        {
            return Err(ProtocolError::InvalidMetric);
        }
        let mut gpus = BTreeSet::new();
        for gpu in &self.gpus {
            if gpu.id.is_empty()
                || gpu.id.len() > 128
                || gpu.name.is_empty()
                || gpu.name.len() > 128
                || !gpus.insert(&gpu.id)
                || !percent(gpu.usage)
                || gpu.temperature.is_some_and(|v| !v.is_finite())
                || !["available", "sleeping", "unavailable"].contains(&gpu.status.as_str())
                || gpu
                    .dedicated_used
                    .zip(gpu.dedicated_total)
                    .is_some_and(|(used, total)| used > total)
                || gpu.status == "sleeping"
                    && (gpu.usage.is_some()
                        || (gpu.dedicated_used.is_some() || gpu.shared_used.is_some())
                        || gpu.temperature.is_some())
            {
                return Err(ProtocolError::InvalidMetric);
            }
        }
        Ok(())
    }
    pub fn encode(&self, sample: Snapshot, out: &mut String) -> Result<(), ProtocolError> {
        self.validate()?;
        sample.encode(out);
        out.truncate(out.len() - 2);
        out.push_str(",\"detail\":");
        out.push_str(&serde_json::to_string(self).map_err(|_| ProtocolError::InvalidFrame)?);
        out.push_str("}\n");
        if out.len() > MAX_ANALYSIS_FRAME_BYTES {
            return Err(ProtocolError::InvalidFrame);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn analysis_does_not_change_the_ordinary_v1_contract() {
        let detail = Details {
            at_ms: 1,
            elapsed_ms: 0,
            sequence: 1,
            period_ms: 500,
            gpu_enabled: false,
            cores: vec![Core { id: 0, usage: None }],
            gpus: vec![],
        };
        let mut out = String::new();
        detail.encode(Snapshot::default(), &mut out).unwrap();
        assert!(super::super::decode(out.as_bytes()).is_ok());
        let mut frame = decode(out.as_bytes()).unwrap();
        assert_eq!(frame.detail.cores.len(), 1);
        frame.detail.cores.push(Core {
            id: 0,
            usage: Some(20.0),
        });
        assert!(frame.detail.validate().is_err());
    }
    #[test]
    fn invalid_core_and_gpu_values_are_rejected() {
        let mut detail = Details {
            at_ms: 1,
            elapsed_ms: 0,
            sequence: 1,
            period_ms: 500,
            gpu_enabled: true,
            cores: vec![Core {
                id: 0,
                usage: Some(f64::NAN),
            }],
            gpus: vec![],
        };
        assert!(detail.validate().is_err());
        detail.cores.clear();
        detail.gpus.push(Gpu {
            id: "a".into(),
            name: "GPU".into(),
            status: "sleeping".into(),
            usage: Some(0.0),
            dedicated_used: None,
            shared_used: None,
            dedicated_total: None,

            temperature: None,
        });
        assert!(detail.validate().is_err());
        detail.gpus[0].usage = None;
        assert!(detail.validate().is_ok());
    }
}
