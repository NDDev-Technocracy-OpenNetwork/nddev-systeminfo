//! Negotiated details reuse the stable aggregate sample without enlarging its
//! ordinary wire frame. Limits and plain data types come from the schema.
use super::{ProtocolError, Snapshot, WireSnapshot};
use serde::Deserialize;
use std::collections::BTreeSet;

pub const SCHEMA: &str = include_str!("../../../contracts/analysis-v1.schema.json");
pub const VIEW_SCHEMA: &str = include_str!("../../../contracts/analysis-view-v1.schema.json");
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

/// Validate the versioned plot at the optional presentation/integration edge.
pub fn decode_view(bytes: &[u8]) -> Result<View, ProtocolError> {
    if bytes.len() > MAX_VIEW_FRAME_BYTES {
        return Err(ProtocolError::InvalidFrame);
    }
    let view: View = serde_json::from_slice(bytes).map_err(|_| ProtocolError::InvalidFrame)?;
    if view.v != VIEW_VERSION
        || view.metric.chars().count() > 160
        || view.label.chars().count() > 64
        || view.unit.chars().count() > 32
        || !(1..=HISTORY_SECONDS as u64).contains(&view.range_seconds)
        || view.start_ms > view.end_ms
        || view.available_ms > view.range_seconds * 1000
        || view.statistics.covered_ms > view.range_seconds * 1000
        || view.statistics.samples > HISTORY_RECORDS
        || view.points.len() > MAX_PLOT_COLUMNS
        || !view.y_min.is_finite()
        || !view.y_max.is_finite()
        || view.y_min >= view.y_max
        || view.points.iter().any(|p| {
            p.x_ms < view.start_ms
                || p.x_ms > view.end_ms
                || ![p.low, p.high, p.value].iter().all(|v| v.is_finite())
                || p.low > p.value
                || p.value > p.high
        })
        || [
            view.statistics.current,
            view.statistics.minimum,
            view.statistics.maximum,
            view.statistics.average,
        ]
        .iter()
        .flatten()
        .any(|v| !v.is_finite())
    {
        return Err(ProtocolError::InvalidFrame);
    }
    if let Some(frame) = &view.latest {
        frame.sample.validate()?;
        frame.detail.validate()?;
    }
    Ok(view)
}

fn percent32<'de, D: serde::Deserializer<'de>>(decoder: D) -> Result<Option<f32>, D::Error> {
    let value = Option::<f64>::deserialize(decoder)?;
    if value.is_some_and(|v| !v.is_finite() || !(0.0..=100.0).contains(&v)) {
        return Err(serde::de::Error::custom("invalid core percentage"));
    }
    Ok(value.map(|v| v as f32))
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
            .any(|v| !cores.insert(v.id) || !percent(v.usage.map(f64::from)))
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
        let excessive = out.replace("\"usage\":null}", "\"usage\":100.00000001}");
        assert!(
            decode(excessive.as_bytes()).is_err(),
            "validate before f32 narrowing"
        );
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
                usage: Some(f32::NAN),
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
