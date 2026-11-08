//! Safe presentation and worker state. All Win32 ownership stays in native.
use crate::{
    client::{Cancellation, Client},
    config::{self, Config},
    presentation::{self as ui, Catalog, Colors, Direction, LANGUAGES},
    protocol::WireSnapshot,
};
use nddev_systeminfo_native::{
    transport,
    tray::{self, Entry, Event, Model, Proxy, Tray},
};
use std::{
    io,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};

const RECONNECT: u32 = 1;
const COPY: u32 = 2;
const QUIT: u32 = 3;
struct State {
    config: Config,
    catalog: Catalog,
    sample: Option<WireSnapshot>,
    generation: u64,
    stopped: bool,
    cancel: Option<Cancellation>,
    icon: Vec<u8>,
    side: u32,
    notice: Option<&'static str>,
}
type Shared = Arc<(Mutex<State>, Condvar)>;

/// A native frontend and its acceptance harness consume the same presentation
/// model. This function owns no collector, window, timer or background process.
pub fn native_model(config: &Config, sample: Option<&WireSnapshot>) -> Model {
    let mut state = State {
        config: config.clone(),
        catalog: Catalog::load(&config.display.locale, &[&tray::system_language()]),
        sample: sample.cloned(),
        generation: 0,
        stopped: false,
        cancel: None,
        icon: Vec::new(),
        side: 32,
        notice: None,
    };
    state.refresh_icon();
    state.model()
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::load(None)?;
    crate::telemetry::init(&config.logging.level);
    let language = tray::system_language();
    let catalog = Catalog::load(&config.display.locale, &[&language]);
    let mut state = State {
        config,
        catalog,
        sample: None,
        generation: 0,
        stopped: false,
        cancel: None,
        icon: Vec::new(),
        side: 32,
        notice: None,
    };
    state.refresh_icon();
    let Some(tray) = Tray::new("org.nddev.systeminfo.tray", state.model())? else {
        return Ok(());
    };
    state.side = ((tray.dpi() * 16 / 96).div_ceil(16) * 16).clamp(16, 128);
    state.refresh_icon();
    let proxy = tray.proxy();
    proxy.update(state.model());
    let shared = Arc::new((Mutex::new(state), Condvar::new()));
    let worker_state = shared.clone();
    let worker_proxy = proxy.clone();
    let worker = thread::Builder::new()
        .name("systeminfo-reader".into())
        .stack_size(128 * 1024)
        .spawn(move || collect(worker_state, worker_proxy))?;
    let mut watcher = tray.watch_config(&config::default_path()?);
    if let Err(error) = &watcher {
        tracing::warn!(event.name="systeminfo.config.watch_failed", error.type="io", message=%error);
    }
    let result = tray.run(|event, proxy| {
        let mut state = shared.0.lock().unwrap();
        match event {
            Event::Command(QUIT) => { proxy.close(); }
            Event::Command(COPY) => {
                if let Some(sample) = &state.sample {
                    let result = serde_json::to_string_pretty(sample)
                        .map_err(io::Error::other).and_then(|text| tray::copy_text(&text));
                    if result.is_err() { state.notice = Some("error"); }
                }
            }
            Event::Command(RECONNECT) => restart(&mut state, &shared.1),
            Event::Command(id) => {
                if let Some((key, value)) = settings(&state).into_iter().find_map(|(item, key, value)| (item == id).then_some((key, value))) {
                    let mut candidate = state.config.clone();
                    let result = candidate.set(&key, &value).and_then(|()| candidate.store(&config::default_path()?));
                    match result {
                        Ok(()) => {
                            state.config = candidate;
                            state.catalog = Catalog::load(&state.config.display.locale, &[&tray::system_language()]);
                            state.refresh_icon();
                            restart(&mut state, &shared.1);
                            if key.starts_with("sampling.") { state.notice = Some("restart_provider"); }
                        }
                        Err(error) => { state.notice = Some("error"); tracing::error!(event.name="systeminfo.config.save_failed", error.type="config", message=%error); }
                    }
                }
            }
            Event::ConfigurationChanged => match Config::load(None) {
                Ok(config) => {
                    if let Ok(path) = config::default_path()
                        && let Some(parent) = path.parent()
                        && parent.exists()
                        && watcher.as_ref().is_ok_and(|watcher| watcher.directory() != parent)
                    { watcher = proxy.watch_config(&path); }
                    // Configuration files change atomically. Comparing their
                    // canonical form avoids churn from unrelated directory writes.
                    if serde_json::to_vec(&config).ok() != serde_json::to_vec(&state.config).ok() {
                        state.config = config;
                        state.catalog = Catalog::load(&state.config.display.locale, &[&tray::system_language()]);
                        state.refresh_icon();
                        restart(&mut state, &shared.1);
                    }
                }
                Err(_) => state.notice = Some("error"),
            },
            Event::AppearanceChanged => { state.refresh_icon(); }
        }
        proxy.update(state.model());
    });
    {
        let mut state = shared.0.lock().unwrap();
        state.stopped = true;
        if let Some(cancel) = state.cancel.take() {
            cancel.cancel();
        }
        shared.1.notify_one();
    }
    drop(watcher);
    worker
        .join()
        .map_err(|_| io::Error::other("native reader failed"))?;
    result?;
    Ok(())
}
fn restart(state: &mut State, wake: &Condvar) {
    state.generation = state.generation.wrapping_add(1);
    state.sample = None;
    state.notice = None;
    if let Some(cancel) = state.cancel.take() {
        cancel.cancel();
    }
    wake.notify_one();
}
fn collect(shared: Shared, proxy: Proxy) {
    loop {
        let (config, generation) = {
            let state = shared.0.lock().unwrap();
            if state.stopped {
                return;
            }
            (state.config.clone(), state.generation)
        };
        let connect = transport::socket_path()
            .map_err(crate::client::ClientError::from)
            .and_then(|path| {
                Client::connect(
                    &path,
                    config.sampling.interval_seconds,
                    Duration::from_millis(config.transport.client_timeout_ms),
                )
            });
        if let Ok(mut client) = connect {
            {
                let mut state = shared.0.lock().unwrap();
                if state.stopped {
                    return;
                }
                if state.generation != generation {
                    continue;
                }
                state.cancel = client.cancellation().ok();
            }
            loop {
                let frame = client.next_sample();
                let mut state = shared.0.lock().unwrap();
                if state.stopped {
                    return;
                }
                if state.generation != generation {
                    break;
                }
                match frame {
                    Ok(sample) => {
                        state.sample = Some(sample);
                        proxy.update(state.model());
                    }
                    Err(_) => {
                        state.sample = None;
                        state.cancel = None;
                        proxy.update(state.model());
                        break;
                    }
                }
            }
        }
        let state = shared.0.lock().unwrap();
        if state.stopped {
            return;
        }
        if state.generation == generation {
            drop(
                shared
                    .1
                    .wait_timeout_while(
                        state,
                        Duration::from_secs(config.transport.reconnect_seconds),
                        |state| !state.stopped && state.generation == generation,
                    )
                    .unwrap(),
            );
        }
    }
}

fn settings(state: &State) -> Vec<(u32, String, String)> {
    let mut values = Vec::new();
    let mut add = |key: &str, value: String| {
        values.push((100 + values.len() as u32, key.into(), value));
    };
    let mut intervals = vec![0, 2, 5, 10, state.config.sampling.interval_seconds];
    intervals.sort_unstable();
    intervals.dedup();
    for value in intervals {
        add("sampling.interval_seconds", value.to_string());
    }
    add("sampling.gpu", (!state.config.sampling.gpu).to_string());
    for (key, value) in [
        ("show_cpu", state.config.display.show_cpu),
        ("show_memory", state.config.display.show_memory),
        ("show_gpu", state.config.display.show_gpu),
        ("show_network", state.config.display.show_network),
        ("show_temperatures", state.config.display.show_temperatures),
    ] {
        add(&format!("display.{key}"), (!value).to_string());
    }
    for value in ["system", "light", "dark", "light-contrast", "dark-contrast"] {
        add("display.theme", value.into());
    }
    for value in ["compact", "comfortable"] {
        add("display.density", value.into());
    }
    for value in ["system", "en", "ru", "kk", "ar", "zh"] {
        add("display.locale", value.into());
    }
    values
}
impl State {
    fn refresh_icon(&mut self) {
        let (dark, contrast, system_color) = tray::system_appearance();
        let colors = Colors::resolve(
            ui::theme(&self.config.display, dark, contrast),
            Direction::On,
        );
        let color = if contrast {
            system_color
        } else {
            [
                colors.accent_text.r,
                colors.accent_text.g,
                colors.accent_text.b,
            ]
        };
        let mask = include_bytes!("../../assets/brand-mask-64.bin");
        let side = self.side;
        self.icon = (0..side)
            .flat_map(|y| {
                (0..side).flat_map(move |x| {
                    let alpha = mask[((y * 64 / side) * 64 + x * 64 / side) as usize];
                    [
                        ((u16::from(color[2]) * u16::from(alpha)) / 255) as u8,
                        ((u16::from(color[1]) * u16::from(alpha)) / 255) as u8,
                        ((u16::from(color[0]) * u16::from(alpha)) / 255) as u8,
                        alpha,
                    ]
                })
            })
            .collect();
    }
    fn model(&self) -> Model {
        let catalog = &self.catalog;
        let mut entries = vec![
            Entry::Label(format!(
                "{} · {}",
                catalog.text("app_title"),
                env!("CARGO_PKG_VERSION")
            )),
            Entry::Label(
                catalog
                    .text(self.notice.unwrap_or(if self.sample.is_some() {
                        "connected"
                    } else {
                        "disconnected"
                    }))
                    .into(),
            ),
        ];
        entries.extend(
            ui::readings(self.sample.as_ref())
                .into_iter()
                .map(|(key, value)| Entry::Label(self.reading(key, &value))),
        );
        entries.push(Entry::Separator);
        for (id, key, enabled) in [
            (COPY, "copy_report", self.sample.is_some()),
            (RECONNECT, "retry", true),
        ] {
            entries.push(Entry::Action {
                id,
                title: catalog.text(key).into(),
                enabled,
                checked: false,
            });
        }
        let mut groups: Vec<(&str, Vec<Entry>)> = [
            "interval", "readings", "panel", "theme", "density", "language",
        ]
        .into_iter()
        .map(|key| (key, Vec::new()))
        .collect();
        for (id, key, value) in settings(self) {
            let (group, title, checked) = match key.as_str() {
                "sampling.interval_seconds" => {
                    let interval: u64 = value.parse().expect("typed cadence");
                    let label = match interval {
                        0 => "realtime",
                        2 => "fast",
                        5 => "standard",
                        10 => "economy",
                        _ => "custom_interval",
                    };
                    (
                        0,
                        catalog.text(label).replace("{seconds}", &value),
                        self.config.sampling.interval_seconds == interval,
                    )
                }
                "sampling.gpu" => (
                    1,
                    catalog.text("collect_gpu").into(),
                    self.config.sampling.gpu,
                ),
                "display.theme" => (
                    3,
                    catalog.text(&value).into(),
                    self.config.display.theme == value,
                ),
                "display.density" => (
                    4,
                    catalog.text(&value).into(),
                    self.config.display.density == value,
                ),
                "display.locale" => (
                    5,
                    if value == "system" {
                        catalog.text("system").into()
                    } else {
                        LANGUAGES
                            .iter()
                            .find(|(code, _)| *code == value)
                            .unwrap()
                            .1
                            .into()
                    },
                    self.config.display.locale == value,
                ),
                _ => (
                    2,
                    catalog
                        .text(key.strip_prefix("display.").expect("display setting"))
                        .into(),
                    value == "false",
                ),
            };
            groups[group].1.push(Entry::Action {
                id,
                title,
                enabled: true,
                checked,
            });
        }
        entries.push(Entry::Submenu {
            title: catalog.text("settings").into(),
            entries: groups
                .into_iter()
                .map(|(key, entries)| Entry::Submenu {
                    title: catalog.text(key).into(),
                    entries,
                })
                .collect(),
        });
        entries.push(Entry::Separator);
        entries.push(Entry::Action {
            id: QUIT,
            title: catalog.text("quit").into(),
            enabled: true,
            checked: false,
        });
        let readings = ui::readings(self.sample.as_ref());
        let display = &self.config.display;
        let mut tooltip = vec![format!(
            "{} · {}",
            catalog.text("app_title"),
            catalog.text(if self.sample.is_some() {
                "connected"
            } else {
                "disconnected"
            })
        )];
        for (show, key) in [
            (display.show_cpu, "cpu"),
            (display.show_memory, "memory"),
            (display.show_gpu, "gpu"),
            (display.show_network, "network_rx"),
            (display.show_network, "network_tx"),
            (display.show_temperatures, "cpu_temperature"),
        ] {
            if show {
                let value = &readings.iter().find(|(name, _)| *name == key).unwrap().1;
                tooltip.push(self.reading(key, value));
            }
        }
        Model {
            tooltip: tooltip.join("\n"),
            entries,
            rtl: catalog.rtl,
            icon: self.icon.clone(),
            icon_side: self.side,
        }
    }
    fn reading(&self, key: &str, value: &str) -> String {
        // Classic USER32 menus implement bidi marks, but render Unicode isolate
        // controls as missing glyphs. Bound the machine value with LRM in RTL.
        let label = self.catalog.text(key);
        if self.catalog.rtl {
            format!("{label}: \u{200e}{value}\u{200e}")
        } else {
            format!("{label}: {value}")
        }
    }
}
