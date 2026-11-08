//! Native window orchestration. The shared Rust analysis domain is also used by
//! AppKit, GTK and CLI consumers; this module owns no unsafe OS access.
use crate::{
    analysis::{Selection, Store, View},
    client::{Cancellation, Client},
    config::Config,
    presentation::{self as ui, Catalog, Colors, Direction},
    protocol,
};
use nddev_systeminfo_native::{
    transport,
    tray::{
        self, Proxy,
        dashboard::{self, Action, Model, Row},
    },
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    sync::{Arc, Condvar, Mutex},
    thread,
    time::Duration,
};
const BACK: u32 = 902;
const PAUSE: u32 = 903;
const SETTINGS: u32 = 904;
const COPY: u32 = 905;
const WEBSITE: u32 = 906;
const PREVIOUS: u32 = 907;
const NEXT: u32 = 908;
const RANGE: u32 = 920;
const CELL: u32 = 1000;
const CORE: u32 = 11000;
const SERIES: u32 = 20000;
#[derive(Deserialize)]
struct Resource {
    website: String,
    cells: Vec<Cell>,
    analysis: Limits,
    typography: BTreeMap<String, Typography>,
}
#[derive(Deserialize)]
struct Typography {
    body: f64,
    card: f64,
}
#[derive(Deserialize)]
struct Cell {
    label: String,
    metric: String,
}
#[derive(Deserialize)]
struct Limits {
    history_seconds: u64,
    ranges_seconds: Vec<u64>,
    cores_per_page: usize,
    plot_columns: usize,
}
struct State {
    config: Config,
    catalog: Catalog,
    resource: Resource,
    open: bool,
    paused: bool,
    selected: Option<String>,
    range: u64,
    page: usize,
    view: Option<View>,
    live: bool,
    stopped: bool,
    generation: u64,
    cancel: Option<Cancellation>,
}
impl State {
    fn new(config: Config) -> Self {
        let resource: Resource =
            serde_json::from_str(include_str!("../../extension/resources/ui.json"))
                .expect("checked native resources");
        let range = resource.analysis.history_seconds;
        let catalog = Catalog::load(&config.display.locale, &[&tray::system_language()]);
        Self {
            config,
            catalog,
            resource,
            open: false,
            paused: false,
            selected: None,
            range,
            page: 0,
            view: None,
            live: false,
            stopped: false,
            generation: 0,
            cancel: None,
        }
    }
}
pub(super) fn native_model(config: &Config, view: &View, selected: Option<&str>) -> Model {
    let mut state = State::new(config.clone());
    state.selected = selected.map(str::to_owned);
    state.view = Some(view.clone());
    state.live = true;
    model(&state)
}
type Shared = Arc<(Mutex<State>, Condvar)>;
pub(super) struct AnalysisUI {
    proxy: Proxy,
    worker: Option<(Shared, thread::JoinHandle<()>)>,
}
impl AnalysisUI {
    pub(super) fn new(proxy: Proxy) -> Self {
        Self {
            proxy,
            worker: None,
        }
    }
    pub(super) fn configure(&mut self, config: &Config) {
        let Some((shared, _)) = &self.worker else {
            return;
        };
        let mut s = shared.0.lock().unwrap();
        if serde_json::to_vec(&s.config).ok() != serde_json::to_vec(config).ok() {
            s.config = config.clone();
            s.catalog = Catalog::load(&s.config.display.locale, &[&tray::system_language()]);
            restart(&mut s, &shared.1);
            if s.open {
                self.proxy.dashboard(Some(model(&s)));
            }
        }
    }
    pub(super) fn command(&mut self, id: u32, config: &Config) -> bool {
        if id == dashboard::OPEN && self.worker.is_none() {
            let shared = Arc::new((Mutex::new(State::new(config.clone())), Condvar::new()));
            let worker_state = shared.clone();
            let proxy = self.proxy.clone();
            match thread::Builder::new()
                .name("analysis-reader".into())
                .stack_size(config.transport.accept_stack_bytes.max(128 * 1024))
                .spawn(move || collect(worker_state, proxy))
            {
                Ok(worker) => self.worker = Some((shared, worker)),
                Err(error) => {
                    tracing::error!(event.name="systeminfo.analysis.start_failed",error.type="io",message=%error);
                    return true;
                }
            }
        }
        let Some((shared, _)) = &self.worker else {
            return false;
        };
        let mut s = shared.0.lock().unwrap();
        let mut reconnect = true;
        match id {
            dashboard::OPEN => {
                s.open = true;
                s.paused = false;
            }
            dashboard::CLOSED => {
                s.open = false;
                s.view = None;
                s.live = false;
            }
            BACK => s.selected = None,
            PAUSE => s.paused = !s.paused,
            SETTINGS => {
                self.proxy.open_menu();
                return true;
            }
            COPY => {
                if let Some(view) = &s.view
                    && let Ok(text) = serde_json::to_string_pretty(view)
                {
                    let _ = tray::copy_text(&text);
                }
                return true;
            }
            WEBSITE => {
                let _ = tray::open_url(&s.resource.website);
                return true;
            }
            PREVIOUS => {
                s.page = s.page.saturating_sub(1);
                reconnect = false;
            }
            NEXT => {
                let count = s
                    .view
                    .as_ref()
                    .and_then(|v| v.latest.as_ref())
                    .map_or(0, |f| f.detail.cores.len());
                s.page = s
                    .page
                    .saturating_add(1)
                    .min(count.saturating_sub(1) / s.resource.analysis.cores_per_page);
                reconnect = false;
            }
            id if (RANGE..RANGE + 16).contains(&id) => {
                if let Some(&range) = s
                    .resource
                    .analysis
                    .ranges_seconds
                    .get((id - RANGE) as usize)
                {
                    s.range = range;
                }
            }
            id if (CELL..CELL + 64).contains(&id) => {
                if let Some(cell) = s.resource.cells.get((id - CELL) as usize) {
                    s.selected = Some(cell.metric.clone());
                    s.paused = false;
                }
            }
            id if (CORE..CORE + 64).contains(&id) => {
                let size = s.resource.analysis.cores_per_page;
                let index = s.page * size + (id - CORE) as usize;
                if let Some(core) = s
                    .view
                    .as_ref()
                    .and_then(|v| v.latest.as_ref())
                    .and_then(|f| f.detail.cores.get(index))
                {
                    s.selected = Some(format!("core:{}", core.id));
                    s.paused = false;
                }
            }
            id if (SERIES..SERIES + 512).contains(&id) => {
                if let Some((key, _)) = choices(&s).get((id - SERIES) as usize) {
                    s.selected = Some(key.clone());
                    s.paused = false;
                }
            }
            _ => return false,
        }
        if reconnect {
            restart(&mut s, &shared.1);
        }
        self.proxy.dashboard(s.open.then(|| model(&s)));
        true
    }
}
impl Drop for AnalysisUI {
    fn drop(&mut self) {
        if let Some((shared, worker)) = self.worker.take() {
            {
                let mut s = shared.0.lock().unwrap();
                s.stopped = true;
                if let Some(cancel) = s.cancel.take() {
                    cancel.cancel();
                }
                shared.1.notify_one();
            }
            if worker.join().is_err() {
                tracing::error!(event.name="systeminfo.analysis.worker_failed",error.type="panic");
            }
        }
    }
}
fn restart(s: &mut State, wake: &Condvar) {
    s.generation = s.generation.wrapping_add(1);
    if let Some(cancel) = s.cancel.take() {
        cancel.cancel();
    }
    wake.notify_one();
}
fn collect(shared: Shared, proxy: Proxy) {
    loop {
        let (config, generation, selection, range, columns) = {
            let mut s = shared.0.lock().unwrap();
            while !s.stopped && (!s.open || s.paused) {
                s = shared.1.wait(s).unwrap();
            }
            if s.stopped {
                return;
            }
            (
                s.config.clone(),
                s.generation,
                Selection::parse(s.selected.as_deref().unwrap_or("cpu.usage"))
                    .expect("validated series"),
                s.range,
                s.resource.analysis.plot_columns,
            )
        };
        let connection = transport::socket_path()
            .map_err(crate::client::ClientError::from)
            .and_then(|path| {
                Client::connect_analysis(
                    &path,
                    config.sampling.interval_seconds,
                    Duration::from_millis(config.transport.client_timeout_ms),
                    true,
                )
            });
        if let Ok(mut client) = connection {
            {
                let mut s = shared.0.lock().unwrap();
                if s.stopped {
                    return;
                }
                if s.generation != generation {
                    continue;
                }
                s.cancel = Some(client.cancellation());
            }
            let mut store = Store::default();
            let mut replaying = true;
            loop {
                let result = client.next_analysis();
                let mut s = shared.0.lock().unwrap();
                if s.stopped {
                    return;
                }
                if s.generation != generation {
                    break;
                }
                match result {
                    Ok(Some(frame)) => {
                        store.push(frame);
                        if replaying {
                            continue;
                        }
                    }
                    Ok(None) => replaying = false,
                    Err(_) => {
                        s.live = false;
                        s.cancel = None;
                        proxy.dashboard(s.open.then(|| model(&s)));
                        break;
                    }
                }
                s.view = Some(store.view(&selection, range, columns));
                s.live = true;
                proxy.dashboard(s.open.then(|| model(&s)));
            }
        } else {
            let mut s = shared.0.lock().unwrap();
            if s.generation == generation {
                s.live = false;
                proxy.dashboard(s.open.then(|| model(&s)));
            }
        }
        let s = shared.0.lock().unwrap();
        if s.stopped {
            return;
        }
        if s.generation == generation {
            drop(
                shared
                    .1
                    .wait_timeout_while(
                        s,
                        Duration::from_secs(config.transport.reconnect_seconds),
                        |s| !s.stopped && s.generation == generation,
                    )
                    .unwrap(),
            );
        }
    }
}
fn action(id: u32, text: String, selected: bool) -> Action {
    Action {
        id,
        text,
        selected,
        enabled: true,
    }
}
fn choices(s: &State) -> Vec<(String, String)> {
    let c = &s.catalog;
    let mut choices: Vec<_> = protocol::METRICS
        .iter()
        .map(|m| (m.key.into(), c.text(m.label).into()))
        .collect();
    if let Some(frame) = s.view.as_ref().and_then(|v| v.latest.as_ref()) {
        for gpu in &frame.detail.gpus {
            for (field, label) in [
                ("usage", "gpu"),
                ("temperature", "gpu_temperature"),
                ("dedicated_used", "dedicated_memory"),
                ("shared_used", "shared_memory"),
            ] {
                choices.push((
                    format!("gpu:{}:{field}", gpu.id),
                    format!("{} · {}", gpu.name, c.text(label)),
                ));
            }
        }
    }
    if let Some(key) = s.selected.as_ref().filter(|k| k.starts_with("core:")) {
        choices.push((
            key.clone(),
            c.text("logical_core")
                .replace("{id}", key.strip_prefix("core:").unwrap()),
        ));
    }
    choices
}
fn value(value: Option<f64>, unit: &str) -> String {
    match unit {
        "percent" => ui::percent(value),
        "celsius" => ui::temperature(value),
        "bytes" => value
            .map(|mut n| {
                let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
                let mut u = 0;
                while n >= 1024.0 && u < units.len() - 1 {
                    n /= 1024.0;
                    u += 1;
                }
                format!("{n:.1} {}", units[u])
            })
            .unwrap_or_else(|| ui::ABSENT.into()),
        _ => ui::rate(value.map(|v| v.clamp(0.0, u64::MAX as f64) as u64)),
    }
}
fn model(s: &State) -> Model {
    let c = &s.catalog;
    let display = &s.config.display;
    let (dark, contrast, _) = tray::system_appearance();
    let theme = ui::theme(display, dark, contrast);
    let colors = Colors::resolve(theme, Direction::On);
    let rgb = |c: nddev_opennetwork_design_tokens::Color| {
        u32::from(c.r) | (u32::from(c.g) << 8) | (u32::from(c.b) << 16)
    };
    let m = ui::metrics(display);
    let typography = &s.resource.typography[c.locale];
    let font = typography.body.round() as i32;
    let card = typography.card.round() as i32;
    let mut rows = vec![
        Row::Text(format!(
            "{} · {}",
            c.text("app_title"),
            env!("CARGO_PKG_VERSION")
        )),
        Row::Text(
            c.text(if s.paused {
                "paused"
            } else if s.live {
                "connected"
            } else {
                "disconnected"
            })
            .into(),
        ),
    ];
    let button_row = |items| Row::Buttons {
        columns: 3,
        large: false,
        items,
    };
    if s.selected.is_none() {
        let readings = ui::readings(if s.live || s.paused {
            s.view
                .as_ref()
                .and_then(|v| v.latest.as_ref())
                .map(|f| &f.sample)
        } else {
            None
        });
        rows.push(Row::Buttons {
            columns: 2,
            large: true,
            items: s
                .resource
                .cells
                .iter()
                .enumerate()
                .map(|(i, cell)| {
                    let value = if cell.label == "history" {
                        format!(
                            "{} min",
                            s.view.as_ref().map_or(0, |v| v.available_ms / 60000)
                        )
                    } else {
                        protocol::METRICS
                            .iter()
                            .find(|m| m.key == cell.metric)
                            .and_then(|m| readings.iter().find(|(key, _)| *key == m.label))
                            .map_or(ui::ABSENT.into(), |(_, value)| value.clone())
                    };
                    action(
                        CELL + i as u32,
                        format!("{}\n{value}", c.text(&cell.label)),
                        false,
                    )
                })
                .collect(),
        });
    } else {
        rows.push(button_row(vec![
            action(BACK, c.text("back").into(), false),
            action(
                PAUSE,
                c.text(if s.paused { "resume" } else { "pause" }).into(),
                s.paused,
            ),
            action(COPY, c.text("copy_chart").into(), false),
        ]));
        let choices = choices(s);
        let selected = choices
            .iter()
            .position(|(key, _)| Some(key) == s.selected.as_ref())
            .unwrap_or(0);
        rows.push(Row::Choice {
            id: SERIES,
            items: choices.into_iter().map(|(_, label)| label).collect(),
            selected,
        });
        rows.push(button_row(
            s.resource
                .analysis
                .ranges_seconds
                .iter()
                .enumerate()
                .map(|(i, &range)| {
                    action(
                        RANGE + i as u32,
                        c.text(&format!("range_{}", range / 60)).into(),
                        s.range == range,
                    )
                })
                .collect(),
        ));
        if let Some(v) = &s.view {
            if let Some(frame) = &v.latest {
                if s.selected
                    .as_ref()
                    .is_some_and(|key| key.starts_with("core:") || key == "cpu.usage")
                {
                    let size = s.resource.analysis.cores_per_page;
                    let count = frame.detail.cores.len();
                    let page = s.page.min(count.saturating_sub(1) / size);
                    rows.push(Row::Text(
                        c.text("core_page")
                            .replace(
                                "{first}",
                                &if count == 0 {
                                    "0".into()
                                } else {
                                    (page * size + 1).to_string()
                                },
                            )
                            .replace("{last}", &count.min((page + 1) * size).to_string())
                            .replace("{count}", &count.to_string()),
                    ));
                    rows.push(Row::Buttons {
                        columns: 4,
                        large: false,
                        items: frame
                            .detail
                            .cores
                            .iter()
                            .skip(page * size)
                            .take(size)
                            .enumerate()
                            .map(|(i, core)| {
                                action(
                                    CORE + i as u32,
                                    format!(
                                        "{}: {}",
                                        c.text("logical_core")
                                            .replace("{id}", &core.id.to_string()),
                                        ui::percent(if s.live || s.paused {
                                            core.usage.map(f64::from)
                                        } else {
                                            None
                                        })
                                    ),
                                    s.selected.as_deref() == Some(&format!("core:{}", core.id)),
                                )
                            })
                            .collect(),
                    });
                    rows.push(button_row(vec![
                        action(PREVIOUS, c.text("previous").into(), false),
                        action(NEXT, c.text("next").into(), false),
                    ]));
                }
                if s.selected
                    .as_ref()
                    .is_some_and(|key| key.starts_with("gpu"))
                {
                    if !frame.detail.gpu_enabled {
                        rows.push(Row::Text(c.text("gpu_disabled").into()));
                    }
                    for gpu in &frame.detail.gpus {
                        rows.push(Row::Text(format!(
                            "{} · {}",
                            gpu.name,
                            c.text(if gpu.status == "available" {
                                "connected"
                            } else {
                                "unavailable"
                            })
                        )));
                    }
                }
            }
            rows.push(Row::Text(format!(
                "{} — {}  ·  −{} min … 0",
                value(Some(v.y_max), &v.unit),
                value(Some(v.y_min), &v.unit),
                v.range_seconds / 60
            )));
            let normalize = |value: f64| ((value - v.y_min) / (v.y_max - v.y_min)).clamp(0.0, 1.0);
            rows.push(Row::Plot {
                points: v
                    .points
                    .iter()
                    .map(|p| dashboard::Point {
                        x: (1.0
                            - (v.end_ms.saturating_sub(p.x_ms)) as f64
                                / (v.range_seconds * 1000) as f64)
                            .clamp(0.0, 1.0),
                        low: normalize(p.low),
                        high: normalize(p.high),
                        value: normalize(p.value),
                        gap: p.gap,
                    })
                    .collect(),
                description: c.text(&v.label).replace(
                    "{id}",
                    s.selected
                        .as_deref()
                        .unwrap_or("")
                        .strip_prefix("core:")
                        .unwrap_or(""),
                ),
            });
            let stats = &v.statistics;
            rows.push(Row::Text(
                [
                    (
                        "current",
                        if s.live || s.paused {
                            stats.current
                        } else {
                            None
                        },
                    ),
                    ("minimum", stats.minimum),
                    ("average", stats.average),
                    ("maximum", stats.maximum),
                ]
                .map(|(key, reading)| c.reading(key, &value(reading, &v.unit)))
                .join("     "),
            ));
            rows.push(Row::Text(
                c.text("history_coverage")
                    .replace("{count}", &stats.samples.to_string())
                    .replace(
                        "{minutes}",
                        &format!("{:.1}", stats.covered_ms as f64 / 60000.0),
                    ),
            ));
        }
    }
    rows.push(Row::Text(c.text("history_hint").into()));
    rows.push(button_row(vec![
        action(WEBSITE, c.text("website").into(), false),
        action(SETTINGS, c.text("settings").into(), false),
        action(dashboard::CLOSED, c.text("close").into(), false),
    ]));
    Model {
        title: c.text("app_title").into(),
        rtl: c.rtl,
        colors: [
            colors.surface_base,
            colors.surface_raised,
            colors.text_primary,
            colors.text_secondary,
            colors.border_control,
            colors.accent_text,
            colors.focus_ring,
        ]
        .map(rgb),
        padding: m.space_4 as i32,
        gap: m.space_2 as i32,
        control: m.size_control_sm as i32,
        font,
        card,
        radius: m.radius_md as i32,
        plot_height: m.size_featured_card_min as i32 / 2,
        width: m.size_featured_card_min as i32 * 3,
        height: m.size_featured_card_min as i32 * 2 + m.space_4 as i32 * 5,
        rows,
    }
}
