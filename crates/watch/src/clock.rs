//! Time source for the run loop: the system clock and a fake for tests.

use std::future::Future;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Unix-seconds clock that can also wait.
pub trait Clock: Send + Sync {
    /// Current unix time in seconds.
    fn now(&self) -> i64;
    /// Waits for `d`.
    fn sleep(&self, d: Duration) -> impl Future<Output = ()> + Send;
}

/// Real time (`SystemTime` and tokio timers).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|d| i64::try_from(d.as_secs()).ok())
            .unwrap_or(0)
    }

    async fn sleep(&self, d: Duration) {
        tokio::time::sleep(d).await;
    }
}

/// Deterministic clock: `sleep` advances time instead of waiting (and yields once so other
/// tasks run).
#[derive(Debug, Default)]
pub struct FakeClock {
    now: AtomicI64,
}

impl FakeClock {
    /// Starts at `now` unix seconds.
    pub fn new(now: i64) -> Self {
        FakeClock {
            now: AtomicI64::new(now),
        }
    }

    /// Moves time forward by `secs`.
    pub fn advance(&self, secs: i64) {
        self.now.fetch_add(secs, Ordering::SeqCst);
    }
}

impl Clock for FakeClock {
    fn now(&self) -> i64 {
        self.now.load(Ordering::SeqCst)
    }

    async fn sleep(&self, d: Duration) {
        self.advance(i64::try_from(d.as_secs()).unwrap_or(i64::MAX));
        tokio::task::yield_now().await;
    }
}
