//! A clock the tests can move: sessions and sign-in tokens expire by it.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Seconds since the clock was made, plus a skew that tests can add to. It is monotonic (it
/// counts from an [`Instant`]), so a change of the wall clock cannot revive an expired session.
#[derive(Clone, Debug)]
pub struct Clock {
    start: Instant,
    skew: Arc<AtomicU64>,
}

impl Clock {
    /// A clock that starts now.
    #[must_use]
    pub fn system() -> Self {
        Self {
            start: Instant::now(),
            skew: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Seconds elapsed (real time plus any skew).
    #[must_use]
    pub fn now(&self) -> u64 {
        self.start
            .elapsed()
            .as_secs()
            .saturating_add(self.skew.load(Ordering::Relaxed))
    }

    /// Moves the clock forward (tests of expiry). Clones share the skew, so a test can keep a
    /// handle after giving one to the server.
    pub fn advance(&self, secs: u64) {
        self.skew.fetch_add(secs, Ordering::Relaxed);
    }
}

impl Default for Clock {
    fn default() -> Self {
        Self::system()
    }
}

#[cfg(test)]
mod tests {
    use super::Clock;

    #[test]
    fn advancing_is_shared_between_clones() {
        let clock = Clock::system();
        let other = clock.clone();
        let before = other.now();
        clock.advance(100);
        assert!(other.now() >= before + 100);
    }
}
