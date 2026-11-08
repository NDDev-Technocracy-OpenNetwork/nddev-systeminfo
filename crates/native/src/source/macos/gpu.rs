//! Read only the accelerator statistics needed by the wire contract.
use super::{mean, registry::Registry};

pub(super) struct Reading {
    pub(super) usage: Option<f32>,
    pub(super) temperature: Option<f32>,
}

pub(super) struct Gpu(Option<Registry<3>>);
impl Gpu {
    pub(super) fn new() -> Self {
        Self(Registry::new(
            c"IOAccelerator",
            c"PerformanceStatistics",
            [
                c"Device Utilization %",
                c"GPU Activity(%)",
                c"Temperature(C)",
            ],
        ))
    }
    pub(super) fn read(&mut self) -> Reading {
        let mut usages = (0.0, 0);
        let mut temperatures = (0.0, 0);
        if let Some(registry) = &mut self.0 {
            registry.visit(|values| {
                if let Some(usage) = values[0].or(values[1]).map(|number| number.decimal())
                    && (0.0..=100.0).contains(&usage)
                {
                    usages.0 += usage;
                    usages.1 += 1;
                }
                if let Some(temperature) = values[2].map(|number| number.decimal())
                    && (1.0..=120.0).contains(&temperature)
                {
                    temperatures.0 += temperature;
                    temperatures.1 += 1;
                }
            });
        }
        Reading {
            usage: mean(usages.0, usages.1),
            temperature: mean(temperatures.0, temperatures.1),
        }
    }
}
