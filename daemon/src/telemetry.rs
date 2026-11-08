//! One local tracing subscriber. Records use OpenTelemetry field names.
use serde_json::{Map, Value};
use std::fmt;
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    fmt::{
        FmtContext, FormatEvent, FormatFields,
        format::Writer,
        time::{FormatTime, SystemTime},
    },
    registry::LookupSpan,
};

struct Fields(Map<String, Value>);
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.0
            .insert(field.name().into(), format!("{value:?}").into());
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.0.insert(field.name().into(), value.into());
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.0.insert(field.name().into(), value.into());
    }
}
struct Json;
impl<S, N> FormatEvent<S, N> for Json
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let mut timestamp = String::new();
        SystemTime.format_time(&mut Writer::new(&mut timestamp))?;
        let mut fields = Fields(Map::new());
        event.record(&mut fields);
        fields.0.insert("timestamp".into(), timestamp.into());
        fields
            .0
            .insert("severity".into(), event.metadata().level().as_str().into());
        fields
            .0
            .insert("service.name".into(), "nddev-systeminfo".into());
        fields
            .0
            .insert("deployment.environment.name".into(), "local".into());
        writeln!(writer, "{}", Value::Object(fields.0))
    }
}
pub fn init(level: &str) {
    let level = match level {
        "off" => tracing::level_filters::LevelFilter::OFF,
        "error" => tracing::level_filters::LevelFilter::ERROR,
        "warn" => tracing::level_filters::LevelFilter::WARN,
        "debug" => tracing::level_filters::LevelFilter::DEBUG,
        "trace" => tracing::level_filters::LevelFilter::TRACE,
        _ => tracing::level_filters::LevelFilter::INFO,
    };
    tracing_subscriber::fmt()
        .with_ansi(false)
        .event_format(Json)
        .with_writer(std::io::stdout)
        .with_max_level(level)
        .init();
}
