use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A token bucket in bytes per second; a rate of 0 is unlimited. The
/// rate can change while transfers run (caps are settings).
pub struct Limiter {
    inner: Mutex<Bucket>,
}

struct Bucket {
    rate: i64,
    tokens: f64,
    last: Instant,
}

impl Limiter {
    pub fn new(rate: i64) -> Limiter {
        Limiter { inner: Mutex::new(Bucket { rate, tokens: 0.0, last: Instant::now() }) }
    }

    pub fn set_rate(&self, rate: i64) {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).rate = rate;
    }

    pub fn rate(&self) -> i64 {
        self.inner.lock().unwrap_or_else(|e| e.into_inner()).rate
    }

    /// How long n bytes must wait before they may pass; `None` takes
    /// them now.
    fn reserve(&self, n: usize) -> Option<Duration> {
        let mut b = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if b.rate <= 0 {
            return None;
        }
        let now = Instant::now();
        let rate = b.rate as f64;
        b.tokens += now.duration_since(b.last).as_secs_f64() * rate;
        b.last = now;
        // At most one second of burst.
        b.tokens = b.tokens.min(rate);
        // A block larger than a second's worth needs a full bucket.
        let need = (n as f64).min(rate);
        if b.tokens >= need {
            b.tokens -= n as f64;
            return None;
        }
        Some(Duration::from_secs_f64((need - b.tokens) / rate))
    }

    /// Waits until n bytes may pass (callers race it against their
    /// cancellation).
    pub async fn wait(&self, n: usize) {
        while let Some(d) = self.reserve(n) {
            tokio::time::sleep(d).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn honors_the_rate_and_changes_live() {
        let rt = tokio::runtime::Builder::new_current_thread().enable_time().build().unwrap();
        rt.block_on(async {
            let l = Limiter::new(1_000_000);
            let start = Instant::now();
            for _ in 0..(1_500_000 / 16384) {
                l.wait(16384).await;
            }
            let took = start.elapsed();
            assert!(took >= Duration::from_millis(350) && took < Duration::from_secs(3), "took {took:?}");
            l.set_rate(0);
            let start = Instant::now();
            for _ in 0..1000 {
                l.wait(1 << 20).await;
            }
            assert!(start.elapsed() < Duration::from_millis(100), "unlimited must not wait");
        });
    }
}
