//! Log output: one JSON object per event through `tracing-subscriber`
//! (D-173), to stderr or a log file.

use std::sync::Mutex;

use tracing::Level;

/// Where log lines go.
pub enum Sink {
    Stderr,
    File(std::fs::File),
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

/// Installs the subscriber process-wide (once; later calls are no-ops).
pub fn init(level: Level, sink: Sink) {
    let fmt = tracing_subscriber::fmt().json().with_max_level(level).with_target(false).flatten_event(true);
    let _ = match sink {
        Sink::Stderr => fmt.with_writer(std::io::stderr).try_init(),
        Sink::File(f) => fmt.with_writer(Mutex::new(f)).try_init(),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        assert_eq!(parse_level("WARN").unwrap(), Level::WARN);
        assert!(parse_level("nope").is_err());
    }
}
