//! `GET /api/events` (D-169): server-sent events so clients and the hq
//! dashboard stop polling. Events:
//!
//! | event              | data                                         | who gets it |
//! |--------------------|----------------------------------------------|-------------|
//! | `ready`            | `{server, version}` on connect               | the stream  |
//! | `module-state`     | `{module, enabled, state}`                   | everyone    |
//! | `library-changed`  | `{library_id, action, items_removed?}`       | everyone    |
//! | `library-changed` `scanned` | `{library_id, action, items, missing, restored, migrated}` | who sees the library |
//! | `library-changed` `item-deleted` | `{library_id, action, item_id}` | who sees the library |
//! | `scan-state`       | `{state, trigger, libraries, libraries_done, stats?, error?}` | everyone (private unreadable roots: admins) |
//! | `progress-changed` | `{item_id, progress}`                        | that user   |
//! | `library-changed` of a private library | (same)                   | admins and members |
//! | `plugin-changed`   | `{provider, action}`                         | admins      |
//!
//! Events about items of a private library reach only sessions that
//! unlocked it (D-156). Each frame carries an `id:` sequence. A slow client whose buffer
//! fills is disconnected (it reconnects); memory stays bounded. The
//! token is re-verified on every keep-alive, so a disabled account or a
//! rotated password ends the stream like any other request.

use std::convert::Infallible;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use hyper::body::{Bytes, Frame};
use serde_json::Value;
use tokio::sync::mpsc;

/// Who an event is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    All,
    Admins,
    User(String),
}

/// Who a stream belongs to.
#[derive(Debug, Clone)]
pub struct SubInfo {
    pub user_id: String,
    pub admin: bool,
    /// The token's session (private-library unlocks are per session).
    pub session: String,
}

struct Subscriber {
    info: SubInfo,
    tx: mpsc::Sender<Bytes>,
}

/// The fan-out point.
#[derive(Default)]
pub struct EventBus {
    subs: Mutex<Vec<Subscriber>>,
    seq: AtomicU64,
}

const BUFFER: usize = 256;

fn frame(id: u64, event: &str, data: &Value) -> Bytes {
    Bytes::from(format!("id: {id}\nevent: {event}\ndata: {data}\n\n"))
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a stream; the first frame is `ready`.
    pub fn subscribe(&self, info: SubInfo, ready: &Value) -> mpsc::Receiver<Bytes> {
        let (tx, rx) = mpsc::channel(BUFFER);
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = tx.try_send(frame(id, "ready", ready));
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).push(Subscriber { info, tx });
        rx
    }

    /// Delivers to every matching stream; closed or full ones are
    /// dropped.
    pub fn publish(&self, scope: Scope, event: &str, data: Value) {
        self.publish_where(event, data, &|s| match &scope {
            Scope::All => true,
            Scope::Admins => s.admin,
            Scope::User(u) => &s.user_id == u,
        });
    }

    /// Delivers to the streams `keep` accepts (privacy filters, D-156).
    pub fn publish_where(&self, event: &str, data: Value, keep: &dyn Fn(&SubInfo) -> bool) {
        let id = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
        let bytes = frame(id, event, &data);
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| !keep(&s.info) || s.tx.try_send(bytes.clone()).is_ok());
    }

    /// Ends every stream (shutdown).
    pub fn close_all(&self) {
        self.subs.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    pub fn subscribers(&self) -> usize {
        let mut subs = self.subs.lock().unwrap_or_else(|e| e.into_inner());
        subs.retain(|s| !s.tx.is_closed());
        subs.len()
    }
}

type Check = Box<dyn Fn() -> bool + Send + Sync>;

/// The response body of one stream: queued frames, and a keep-alive
/// comment on each tick that also re-checks the token.
pub struct SseBody {
    rx: mpsc::Receiver<Bytes>,
    tick: tokio::time::Interval,
    still_valid: Check,
}

impl SseBody {
    pub fn new(rx: mpsc::Receiver<Bytes>, keep_alive: Duration, still_valid: Check) -> Self {
        let mut tick = tokio::time::interval_at(tokio::time::Instant::now() + keep_alive, keep_alive);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        SseBody { rx, tick, still_valid }
    }
}

impl hyper::body::Body for SseBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
        let this = self.get_mut();
        match this.rx.poll_recv(cx) {
            Poll::Ready(Some(b)) => return Poll::Ready(Some(Ok(Frame::data(b)))),
            Poll::Ready(None) => return Poll::Ready(None),
            Poll::Pending => {}
        }
        match this.tick.poll_tick(cx) {
            Poll::Ready(_) => {
                if !(this.still_valid)() {
                    return Poll::Ready(None);
                }
                Poll::Ready(Some(Ok(Frame::data(Bytes::from_static(b": keep-alive\n\n")))))
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn scopes_and_slow_clients() {
        let bus = EventBus::new();
        let info = |u: &str, admin: bool| SubInfo { user_id: u.into(), admin, session: format!("s-{u}") };
        let mut a = bus.subscribe(info("u1", false), &json!({}));
        let mut admin = bus.subscribe(info("u2", true), &json!({}));
        assert!(String::from_utf8_lossy(&a.try_recv().unwrap()).contains("event: ready"));
        admin.try_recv().unwrap();
        bus.publish(Scope::User("u1".into()), "progress-changed", json!({"item_id": "i"}));
        bus.publish(Scope::Admins, "plugin-changed", json!({}));
        let got = String::from_utf8_lossy(&a.try_recv().unwrap()).into_owned();
        assert!(
            got.starts_with("id: ") && got.contains("event: progress-changed\ndata: {\"item_id\":\"i\"}\n\n"),
            "{got}"
        );
        assert!(a.try_recv().is_err());
        assert!(String::from_utf8_lossy(&admin.try_recv().unwrap()).contains("plugin-changed"));
        // A client that never reads is dropped once its buffer fills.
        for _ in 0..BUFFER + 1 {
            bus.publish(Scope::All, "library-changed", json!({}));
        }
        drop(admin);
        assert_eq!(bus.subscribers(), 0);
    }
}
