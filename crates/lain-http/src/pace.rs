//! Per-host pacing: a minimum interval between the starts of two
//! requests to one host, as Go's `politeClient` keeps per provider.
//! Callers reserve the next slot under the lock and sleep outside it,
//! so concurrent requests queue in order instead of bursting.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Pacer {
    hosts: Mutex<HashMap<String, Slot>>,
}

struct Slot {
    gap: Duration,
    next: Option<Instant>,
}

impl Pacer {
    pub(crate) fn set(&self, host_key: &str, gap: Duration) {
        let mut g = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        if gap.is_zero() {
            g.remove(host_key);
        } else {
            g.entry(host_key.to_string()).or_insert(Slot { gap, next: None }).gap = gap;
        }
    }

    /// How long the caller waits before it may start a request to this
    /// host; the slot after it is reserved at once.
    pub(crate) fn reserve(&self, host_key: &str) -> Duration {
        let mut g = self.hosts.lock().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = g.get_mut(host_key) else {
            return Duration::ZERO;
        };
        let now = Instant::now();
        let start = match slot.next {
            Some(t) if t > now => t,
            _ => now,
        };
        slot.next = Some(start + slot.gap);
        start - now
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reserves_consecutive_slots() {
        let p = Pacer::default();
        assert_eq!(p.reserve("a:443"), Duration::ZERO, "an unpaced host never waits");
        p.set("a:443", Duration::from_millis(100));
        assert_eq!(p.reserve("a:443"), Duration::ZERO);
        let second = p.reserve("a:443");
        let third = p.reserve("a:443");
        assert!(second > Duration::from_millis(80) && second <= Duration::from_millis(100), "{second:?}");
        assert!(third > Duration::from_millis(180) && third <= Duration::from_millis(200), "{third:?}");
        assert_eq!(p.reserve("b:443"), Duration::ZERO, "hosts are paced apart");
        p.set("a:443", Duration::ZERO);
        assert_eq!(p.reserve("a:443"), Duration::ZERO);
    }
}
