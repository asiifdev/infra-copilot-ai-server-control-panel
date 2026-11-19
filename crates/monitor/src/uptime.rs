use serde::{Deserialize, Serialize};

/// Availability over a window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct UptimeStats {
    pub total_secs: u64,
    pub down_secs: u64,
}

impl UptimeStats {
    /// Percentage up in `0.0..=100.0`. An empty window counts as 100.
    #[must_use]
    pub fn percent(&self) -> f64 {
        if self.total_secs == 0 {
            return 100.0;
        }
        let down = self.down_secs.min(self.total_secs);
        let up = self.total_secs.saturating_sub(down);
        (up as f64 / self.total_secs as f64) * 100.0
    }
}

/// Downtime inside `[from, to)` from `[start, end)` intervals. Intervals may overlap, be unsorted,
/// extend past the window or be inverted (ignored); `to <= from` yields an empty window.
#[must_use]
pub fn uptime_from_intervals(intervals: &[(u64, u64)], from: u64, to: u64) -> UptimeStats {
    let total_secs = to.saturating_sub(from);
    if total_secs == 0 {
        return UptimeStats {
            total_secs: 0,
            down_secs: 0,
        };
    }
    let mut clipped: Vec<(u64, u64)> = intervals
        .iter()
        .map(|&(s, e)| (s.max(from), e.min(to)))
        .filter(|&(s, e)| e > s)
        .collect();
    clipped.sort_unstable();
    let mut down_secs = 0u64;
    let mut cur: Option<(u64, u64)> = None;
    for (s, e) in clipped {
        cur = match cur {
            Some((cs, ce)) if s <= ce => Some((cs, ce.max(e))),
            Some((cs, ce)) => {
                down_secs = down_secs.saturating_add(ce.saturating_sub(cs));
                Some((s, e))
            }
            None => Some((s, e)),
        };
    }
    if let Some((cs, ce)) = cur {
        down_secs = down_secs.saturating_add(ce.saturating_sub(cs));
    }
    UptimeStats {
        total_secs,
        down_secs: down_secs.min(total_secs),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table() {
        // (intervals, from, to, want_down, want_total)
        type Case = (Vec<(u64, u64)>, u64, u64, u64, u64);
        let cases: Vec<Case> = vec![
            (vec![], 0, 100, 0, 100),
            (vec![(10, 20)], 0, 100, 10, 100),
            (vec![(10, 20), (15, 30)], 0, 100, 20, 100),
            (vec![(50, 60), (10, 20)], 0, 100, 20, 100),
            (vec![(0, 1000)], 100, 200, 100, 100),
            (vec![(300, 400)], 0, 100, 0, 100),
            (vec![(20, 10)], 0, 100, 0, 100),
            (vec![(10, 20)], 100, 100, 0, 0),
            (vec![(10, 20)], 100, 50, 0, 0),
            (vec![(10, 20), (20, 30)], 0, 100, 20, 100),
            (vec![(0, u64::MAX)], 0, u64::MAX, u64::MAX, u64::MAX),
        ];
        for (i, (iv, f, t, down, total)) in cases.into_iter().enumerate() {
            let s = uptime_from_intervals(&iv, f, t);
            assert_eq!((s.down_secs, s.total_secs), (down, total), "case {i}");
            let p = s.percent();
            assert!((0.0..=100.0).contains(&p), "case {i}: {p}");
        }
    }

    #[test]
    fn percent_values() {
        assert!(
            (UptimeStats {
                total_secs: 0,
                down_secs: 0
            }
            .percent()
                - 100.0)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (UptimeStats {
                total_secs: 200,
                down_secs: 50
            }
            .percent()
                - 75.0)
                .abs()
                < 1e-9
        );
        assert!(
            UptimeStats {
                total_secs: 10,
                down_secs: 10
            }
            .percent()
            .abs()
                < f64::EPSILON
        );
        // down > total is clamped, never negative.
        assert!(
            UptimeStats {
                total_secs: 10,
                down_secs: 99
            }
            .percent()
            .abs()
                < f64::EPSILON
        );
    }
}
