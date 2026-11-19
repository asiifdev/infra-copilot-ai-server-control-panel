//! Pure send-policy helpers. Time is always passed in.

use crate::{ChannelConfig, Severity};
use std::cmp::Ordering;
use std::collections::VecDeque;

/// Decides whether a message of `severity` may be sent at `now` (unix seconds).
///
/// Below `min_severity`: no. Inside quiet hours: no, unless the window allows critical messages
/// and this one is `Critical`. Fixed offsets only; the window wraps past midnight when
/// `start > end`, and `start == end` is an empty window.
pub fn should_send(cfg: &ChannelConfig, severity: Severity, now: i64) -> bool {
    if severity < cfg.min_severity {
        return false;
    }
    let Some(q) = cfg.quiet_hours else {
        return true;
    };
    if q.critical_bypasses && severity == Severity::Critical {
        return true;
    }
    let local = now.saturating_add(i64::from(q.utc_offset_min) * 60);
    let minute = local.rem_euclid(86_400) / 60;
    let (start, end) = (i64::from(q.start_min), i64::from(q.end_min));
    let inside = match start.cmp(&end) {
        Ordering::Equal => false,
        Ordering::Less => minute >= start && minute < end,
        Ordering::Greater => minute >= start || minute < end,
    };
    !inside
}

/// At most `max` sends per `window_secs`, for one channel. Sliding window.
#[derive(Debug, Clone)]
pub struct Throttle {
    max: usize,
    window_secs: i64,
    sent: VecDeque<i64>,
}

impl Throttle {
    /// `max == 0` blocks everything.
    pub fn new(max: usize, window_secs: u32) -> Self {
        Throttle {
            max,
            window_secs: i64::from(window_secs),
            sent: VecDeque::new(),
        }
    }

    /// Records a send at `now` and returns true if it is allowed; returns false (and records
    /// nothing) when the window is full. A clock that jumps backwards never frees capacity early.
    pub fn try_acquire(&mut self, now: i64) -> bool {
        let cutoff = now.saturating_sub(self.window_secs);
        self.sent.retain(|t| *t > cutoff);
        if self.sent.len() >= self.max {
            return false;
        }
        self.sent.push_back(now);
        true
    }
}
