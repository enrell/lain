//! The event stream client for `GET /api/events` (server-sent events,
//! D-169). It reconnects on its own with exponential backoff and
//! re-reads the session on a 401, so a fresh login elsewhere in the app
//! resumes the stream without a restart.
//!
//! The server sends no replay: events missed while disconnected are
//! gone. A client refetches what it shows when it sees
//! [`StreamItem::Connected`] after a reconnect, and flushes queued
//! progress ([`crate::ProgressSync::flush`]) then.

use std::time::Duration;

use hyper::body::Incoming;
use serde::de::DeserializeOwned;
use tokio::sync::Mutex;

use lain_api::{LibraryChangedEvent, ModuleStateEvent, PluginChangedEvent, ProgressChangedEvent, ReadyEvent};

use crate::api::Client;
use crate::error::ClientError;
use crate::http::next_chunk;

/// One decoded server event.
#[derive(Debug, Clone, PartialEq)]
pub enum ServerEvent {
    ModuleState(ModuleStateEvent),
    LibraryChanged(LibraryChangedEvent),
    ProgressChanged(ProgressChangedEvent),
    PluginChanged(PluginChangedEvent),
    /// An event this build does not know, kept raw (forward compatible).
    Other {
        event: String,
        data: String,
    },
}

/// What [`EventStream::next`] yields.
#[derive(Debug, Clone, PartialEq)]
pub enum StreamItem {
    /// The stream is (re)connected: the server's `ready` frame.
    Connected(ReadyEvent),
    Event(ServerEvent),
    /// The connection dropped; the next call waits `delay_ms` and retries.
    Reconnecting {
        attempt: u32,
        delay_ms: u64,
        reason: String,
    },
    /// The server refused the session. Log in again; the next call
    /// waits the backoff, re-reads the session and retries.
    AuthRequired,
}

/// Timing of a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamConfig {
    pub initial_backoff_ms: u64,
    pub max_backoff_ms: u64,
    /// No byte for this long means a dead connection. The server sends a
    /// keep-alive every 15 s, so the default allows two to go missing.
    pub idle_timeout_ms: u64,
}

impl Default for StreamConfig {
    fn default() -> Self {
        StreamConfig { initial_backoff_ms: 250, max_backoff_ms: 30_000, idle_timeout_ms: 45_000 }
    }
}

/// One SSE frame.
#[derive(Debug, Default, PartialEq)]
struct Frame {
    event: String,
    data: String,
}

/// An incremental SSE parser (WHATWG `text/event-stream`): `event:`,
/// `data:` (lines joined with `\n`), comments and `id:` ignored, a blank
/// line dispatches. `\n`, `\r\n` and `\r` all end lines.
#[derive(Default)]
struct Parser {
    buf: Vec<u8>,
    cur: Frame,
    has_data: bool,
}

impl Parser {
    fn feed(&mut self, bytes: &[u8]) -> Vec<Frame> {
        self.buf.extend_from_slice(bytes);
        let mut out = Vec::new();
        while let Some(end) = self.buf.iter().position(|&b| b == b'\n' || b == b'\r') {
            // A lone `\r` at the end may be half of `\r\n`: wait for more.
            if self.buf[end] == b'\r' && end + 1 == self.buf.len() {
                break;
            }
            let skip = if self.buf[end] == b'\r' && self.buf.get(end + 1) == Some(&b'\n') { 2 } else { 1 };
            let line = String::from_utf8_lossy(&self.buf[..end]).into_owned();
            self.buf.drain(..end + skip);
            if line.is_empty() {
                if self.has_data {
                    out.push(std::mem::take(&mut self.cur));
                }
                self.cur = Frame::default();
                self.has_data = false;
                continue;
            }
            if line.starts_with(':') {
                continue;
            }
            let (field, value) = match line.split_once(':') {
                Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
                None => (line.as_str(), ""),
            };
            match field {
                "event" => self.cur.event = value.to_string(),
                "data" => {
                    if self.has_data {
                        self.cur.data.push('\n');
                    }
                    self.cur.data.push_str(value);
                    self.has_data = true;
                }
                _ => {}
            }
        }
        out
    }
}

fn parse<T: DeserializeOwned>(f: &Frame) -> Option<T> {
    serde_json::from_str(&f.data).ok()
}

fn decode(f: Frame) -> Option<StreamItem> {
    let ev = match f.event.as_str() {
        "ready" => return parse(&f).map(StreamItem::Connected),
        "module-state" => parse(&f).map(ServerEvent::ModuleState),
        "library-changed" => parse(&f).map(ServerEvent::LibraryChanged),
        "progress-changed" => parse(&f).map(ServerEvent::ProgressChanged),
        "plugin-changed" => parse(&f).map(ServerEvent::PluginChanged),
        _ => None,
    };
    Some(StreamItem::Event(ev.unwrap_or(ServerEvent::Other { event: f.event, data: f.data })))
}

struct Live {
    body: Incoming,
    parser: Parser,
    pending: std::collections::VecDeque<StreamItem>,
}

struct State {
    live: Option<Live>,
    attempt: u32,
    /// A wait owed before the next connect attempt.
    wait_ms: u64,
    closed: bool,
}

/// A self-healing subscription to `GET /api/events`.
pub struct EventStream {
    client: Client,
    cfg: StreamConfig,
    state: Mutex<State>,
}

/// A cheap jitter source (±20%) so many clients do not reconnect in
/// lockstep after a server restart; it needs no randomness crate.
fn jitter(ms: u64) -> u64 {
    let nanos =
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_nanos()).unwrap_or(0);
    let spread = ms / 5;
    if spread == 0 { ms } else { ms - spread + (nanos as u64 % (2 * spread + 1)) }
}

impl EventStream {
    pub fn new(client: Client, cfg: StreamConfig) -> Self {
        EventStream { client, cfg, state: Mutex::new(State { live: None, attempt: 0, wait_ms: 0, closed: false }) }
    }

    fn backoff(&self, attempt: u32) -> u64 {
        let exp = self.cfg.initial_backoff_ms.saturating_mul(1u64 << attempt.min(20));
        jitter(exp.min(self.cfg.max_backoff_ms))
    }

    /// Ends the stream: `next` then returns `None`.
    pub async fn close(&self) {
        let mut st = self.state.lock().await;
        st.closed = true;
        st.live = None;
    }

    /// The next item. It never fails: connection trouble comes back as
    /// [`StreamItem::Reconnecting`] or [`StreamItem::AuthRequired`] and
    /// the following call retries. `None` only after [`Self::close`].
    pub async fn next(&self) -> Option<StreamItem> {
        let mut st = self.state.lock().await;
        loop {
            if st.closed {
                return None;
            }
            if let Some(live) = st.live.as_mut() {
                if let Some(item) = live.pending.pop_front() {
                    return Some(item);
                }
                let idle = Duration::from_millis(self.cfg.idle_timeout_ms);
                let reason = match tokio::time::timeout(idle, next_chunk(&mut live.body)).await {
                    Err(_) => "no keep-alive from the server".to_string(),
                    Ok(Ok(None)) => "the server closed the stream".to_string(),
                    Ok(Err(e)) => e.to_string(),
                    Ok(Ok(Some(chunk))) => {
                        for f in live.parser.feed(&chunk) {
                            if let Some(item) = decode(f) {
                                live.pending.push_back(item);
                            }
                        }
                        continue;
                    }
                };
                st.live = None;
                // A stream that worked starts the backoff over.
                let delay = self.backoff(0);
                st.attempt = 1;
                st.wait_ms = delay;
                return Some(StreamItem::Reconnecting { attempt: 1, delay_ms: delay, reason });
            }
            if st.wait_ms > 0 {
                tokio::time::sleep(Duration::from_millis(st.wait_ms)).await;
                st.wait_ms = 0;
            }
            match self.connect().await {
                Ok(live) => {
                    st.attempt = 0;
                    st.live = Some(live);
                }
                Err(ClientError::Unauthorized) => {
                    // Pick up a login done elsewhere; when there is none,
                    // tell the app and wait before asking again.
                    if matches!(self.client.reload_session(), Ok(true)) {
                        continue;
                    }
                    st.wait_ms = self.backoff(st.attempt);
                    st.attempt = st.attempt.saturating_add(1);
                    return Some(StreamItem::AuthRequired);
                }
                Err(e) => {
                    st.attempt = st.attempt.saturating_add(1);
                    let delay = self.backoff(st.attempt - 1);
                    st.wait_ms = delay;
                    return Some(StreamItem::Reconnecting {
                        attempt: st.attempt,
                        delay_ms: delay,
                        reason: e.to_string(),
                    });
                }
            }
        }
    }

    async fn connect(&self) -> Result<Live, ClientError> {
        let headers = [("accept", "text/event-stream".to_string())];
        let s = tokio::time::timeout(self.client.timeout(), self.client.open("/api/events", &headers))
            .await
            .map_err(|_| ClientError::Timeout)??;
        if s.status != 200 {
            let body = http_body_util::BodyExt::collect(s.body).await.map(|b| b.to_bytes()).unwrap_or_default();
            return Err(ClientError::from_response(s.status, "GET /api/events", &body));
        }
        Ok(Live { body: s.body, parser: Parser::default(), pending: Default::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_parse_across_chunks() {
        let mut p = Parser::default();
        assert!(p.feed(b"id: 1\nevent: ready\nda").is_empty());
        let f = p.feed(b"ta: {\"server\":\"lain-rs\",\"version\":\"0.1.0-rs\"}\n\n: keep-alive\n\n");
        assert_eq!(
            f,
            vec![Frame { event: "ready".into(), data: r#"{"server":"lain-rs","version":"0.1.0-rs"}"#.into() }]
        );
        let f = p.feed(b"event: x\r\ndata: a\r\ndata:b\r");
        assert!(f.is_empty());
        let f = p.feed(b"\n\r\n");
        assert_eq!(f, vec![Frame { event: "x".into(), data: "a\nb".into() }]);
        // A frame with no data dispatches nothing.
        assert!(p.feed(b"event: lone\n\n").is_empty());
    }

    #[test]
    fn events_decode() {
        let f = Frame {
            event: "progress-changed".into(),
            data: r#"{"item_id":"i","progress":{"item_id":"i","position_sec":5}}"#.into(),
        };
        match decode(f) {
            Some(StreamItem::Event(ServerEvent::ProgressChanged(e))) => assert_eq!(e.progress.position_sec, 5.0),
            other => panic!("{other:?}"),
        }
        let f = Frame { event: "future-thing".into(), data: "{}".into() };
        assert_eq!(
            decode(f),
            Some(StreamItem::Event(ServerEvent::Other { event: "future-thing".into(), data: "{}".into() }))
        );
    }

    #[test]
    fn jitter_stays_within_a_fifth() {
        for _ in 0..50 {
            let j = jitter(1000);
            assert!((800..=1200).contains(&j), "{j}");
        }
    }
}
