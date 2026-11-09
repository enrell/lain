//! Small concurrency helpers: a cancellation token, a wait group the
//! blocking API waits on, and a test PRNG.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use tokio::sync::Notify;

/// A one-shot cancellation signal for tasks.
#[derive(Default)]
pub(crate) struct Cancel {
    flag: AtomicBool,
    notify: Notify,
}

impl Cancel {
    pub fn new() -> Arc<Cancel> {
        Arc::new(Cancel::default())
    }
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }
    pub fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
    pub async fn cancelled(&self) {
        loop {
            let n = self.notify.notified();
            tokio::pin!(n);
            n.as_mut().enable();
            if self.is_cancelled() {
                return;
            }
            n.await;
        }
    }
}

/// Go's sync.WaitGroup for tasks: the blocking side waits on a condvar.
#[derive(Clone, Default)]
pub(crate) struct WaitGroup(Arc<(Mutex<usize>, Condvar)>);

pub(crate) struct WgGuard(WaitGroup);

impl Drop for WgGuard {
    fn drop(&mut self) {
        let (m, cv) = &*self.0.0;
        let mut n = m.lock().unwrap_or_else(|e| e.into_inner());
        *n = n.saturating_sub(1);
        if *n == 0 {
            cv.notify_all();
        }
    }
}

impl WaitGroup {
    /// Counts one task; the guard ends it when dropped.
    pub fn add(&self) -> WgGuard {
        *self.0.0.lock().unwrap_or_else(|e| e.into_inner()) += 1;
        WgGuard(self.clone())
    }
    /// Blocks until every guard is dropped (bounded, so a wedged task
    /// cannot hang shutdown forever).
    pub fn wait(&self, limit: Duration) {
        let (m, cv) = &*self.0;
        let n = m.lock().unwrap_or_else(|e| e.into_inner());
        let _ = cv.wait_timeout_while(n, limit, |n| *n > 0);
    }
}

/// xorshift64*, for deterministic garbage in tests.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct Rng(u64);

#[cfg_attr(not(test), allow(dead_code))]
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 { 0 } else { (self.next() % n as u64) as usize }
    }
    /// Flips, inserts, deletes or duplicates a few bytes.
    pub fn mutate(&mut self, data: &mut Vec<u8>) {
        for _ in 0..1 + self.below(4) {
            let at = self.below(data.len() + 1);
            match self.below(5) {
                0 if at < data.len() => data[at] ^= 1 << self.below(8),
                1 => data.insert(at, self.next() as u8),
                2 if at < data.len() => {
                    data.remove(at);
                }
                3 if at < data.len() => {
                    let end = (at + self.below(8)).min(data.len());
                    let chunk = data[at..end].to_vec();
                    data.splice(at..at, chunk);
                }
                _ => data.insert(at, b"ield0123456789:-"[self.below(16)]),
            }
        }
    }
}
