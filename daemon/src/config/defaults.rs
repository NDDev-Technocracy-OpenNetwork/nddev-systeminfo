// Generated from config/default.toml. Edit its authoritative TOML, then just generate.
use super::{Config, Display, Logging, Sampling, Transport};

pub(super) fn load() -> Config {
    Config {
        display: Display {
            density: "compact".into(),
            locale: "system".into(),
            show_cpu: true,
            show_gpu: false,
            show_memory: true,
            show_network: true,
            show_temperatures: false,
            theme: "system".into(),
        },
        logging: Logging {
            level: "info".into(),
        },
        sampling: Sampling {
            gpu: false,
            idle_release_seconds: 30,
            interval_seconds: 5,
        },
        transport: Transport {
            accept_stack_bytes: 65536,
            client_timeout_ms: 5000,
            handshake_bytes: 256,
            handshake_timeout_ms: 250,
            max_clients: 32,
            reconnect_seconds: 5,
        },
    }
}
