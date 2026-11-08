//! A small `tracing` subscriber that writes one JSON object per event
//! (`{"time","level","msg",...fields}`, the shape of the Go server's
//! slog output) to stderr or a log file. Spans are not tracked; the
//! server logs events only. Written here because D-172 lists `tracing`
//! without `tracing-subscriber`.

use std::fmt::Write as _;
use std::io::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Event, Level, Metadata, Subscriber};

/// Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// RFC 3339 UTC with milliseconds.
pub fn rfc3339(unix_millis: i64) -> String {
    let secs = unix_millis.div_euclid(1000);
    let ms = unix_millis.rem_euclid(1000);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    let t = secs.rem_euclid(86_400);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{ms:03}Z", t / 3600, t % 3600 / 60, t % 60)
}

struct Fields {
    msg: String,
    rest: String,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.msg = value.to_string();
        } else {
            let _ = write!(self.rest, ",{}:{}", serde_json::Value::from(field.name()), serde_json::Value::from(value));
        }
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        let _ = write!(self.rest, ",{}:{value}", serde_json::Value::from(field.name()));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        let _ = write!(self.rest, ",{}:{value}", serde_json::Value::from(field.name()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        let _ = write!(self.rest, ",{}:{value}", serde_json::Value::from(field.name()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.record_str(field, &format!("{value:?}"));
    }
}

/// Where log lines go.
pub enum Sink {
    Stderr,
    File(std::fs::File),
}

pub struct JsonLog {
    max: Level,
    sink: Mutex<Sink>,
    next_id: AtomicU64,
}

impl JsonLog {
    pub fn new(max: Level, sink: Sink) -> Self {
        JsonLog { max, sink: Mutex::new(sink), next_id: AtomicU64::new(1) }
    }
}

/// Parses `debug|info|warn|error`.
pub fn parse_level(s: &str) -> Result<Level, String> {
    match s.to_ascii_lowercase().as_str() {
        "debug" => Ok(Level::DEBUG),
        "info" => Ok(Level::INFO),
        "warn" | "warning" => Ok(Level::WARN),
        "error" => Ok(Level::ERROR),
        other => Err(format!("unknown log level {other:?} (debug, info, warn, error)")),
    }
}

impl Subscriber for JsonLog {
    fn enabled(&self, m: &Metadata<'_>) -> bool {
        *m.level() <= self.max
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_id.fetch_add(1, Ordering::Relaxed))
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn event(&self, e: &Event<'_>) {
        let mut f = Fields { msg: String::new(), rest: String::new() };
        e.record(&mut f);
        let level = match *e.metadata().level() {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            Level::INFO => "INFO",
            _ => "DEBUG",
        };
        let line = format!(
            "{{\"time\":\"{}\",\"level\":\"{level}\",\"msg\":{}{}}}\n",
            rfc3339(lain_core::now_unix_millis()),
            serde_json::Value::from(f.msg),
            f.rest
        );
        let mut sink = self.sink.lock().unwrap_or_else(|e| e.into_inner());
        let _ = match &mut *sink {
            Sink::Stderr => std::io::stderr().write_all(line.as_bytes()),
            Sink::File(file) => file.write_all(line.as_bytes()),
        };
    }
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
}

/// Installs the subscriber process-wide (once).
pub fn init(level: Level, sink: Sink) {
    let _ = tracing::subscriber::set_global_default(JsonLog::new(level, sink));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamps() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(rfc3339(951_782_400_123), "2000-02-29T00:00:00.123Z");
        assert_eq!(rfc3339(1_794_173_709_000), "2026-11-08T21:35:09.000Z");
        assert!(parse_level("nope").is_err());
    }
}
