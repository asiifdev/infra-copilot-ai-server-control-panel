use std::cell::RefCell;
use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::config::MonitorConfig;

/// Closed classification of a failed check. Never carries text from the probed server.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ErrorClass {
    Timeout,
    DnsFailure,
    ConnectFailed,
    TlsFailure,
    UnexpectedStatus(u16),
    KeywordMissing,
    /// The target resolved to an address the prober refuses to contact (SSRF guard).
    BlockedAddress,
    Protocol,
    Other,
}

/// Result of one probe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckOutcome {
    /// Target healthy (for `TlsExpiry`: reachable and certificate outside the warning window).
    Up {
        latency_ms: u32,
    },
    Down {
        class: ErrorClass,
    },
    /// Reachable, but the certificate expires within `warn_days` (`days_left` may be negative when
    /// already expired). Counts as available for incident purposes.
    CertExpiring {
        days_left: i64,
    },
}

/// Performs one check. Synchronous here; an async adapter comes later.
pub trait Prober {
    fn probe(&self, config: &MonitorConfig) -> CheckOutcome;
}

/// Scripted prober for tests: returns queued outcomes in order, then `fallback` forever.
#[derive(Debug)]
pub struct FakeProber {
    queue: RefCell<VecDeque<CheckOutcome>>,
    fallback: CheckOutcome,
}

impl FakeProber {
    #[must_use]
    pub fn new(script: impl IntoIterator<Item = CheckOutcome>, fallback: CheckOutcome) -> Self {
        Self {
            queue: RefCell::new(script.into_iter().collect()),
            fallback,
        }
    }
}

impl Prober for FakeProber {
    fn probe(&self, _config: &MonitorConfig) -> CheckOutcome {
        self.queue.borrow_mut().pop_front().unwrap_or(self.fallback)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::MonitorKind;

    #[test]
    fn fake_prober_plays_script_then_fallback() {
        let c = MonitorConfig::new(
            "m",
            MonitorKind::Tcp {
                host: "h".into(),
                port: 1,
            },
            60,
            10,
        );
        let down = CheckOutcome::Down {
            class: ErrorClass::Timeout,
        };
        let up = CheckOutcome::Up { latency_ms: 5 };
        let p = FakeProber::new([down, down], up);
        assert_eq!(p.probe(&c), down);
        assert_eq!(p.probe(&c), down);
        assert_eq!(p.probe(&c), up);
        assert_eq!(p.probe(&c), up);
    }
}
