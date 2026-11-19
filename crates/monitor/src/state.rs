use serde::{Deserialize, Serialize};

use crate::config::MonitorConfig;
use crate::outcome::{CheckOutcome, ErrorClass, Prober};
use crate::uptime::{UptimeStats, uptime_from_intervals};

/// Closed incidents kept for uptime accounting; the oldest are dropped beyond this.
pub const MAX_HISTORY: usize = 256;

/// A closed incident, `[start, end)` in unix seconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incident {
    pub start: u64,
    pub end: u64,
}

/// The currently open incident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenIncident {
    /// Time of the first failure of the streak that opened it (not the time the threshold was met).
    pub since: u64,
    pub class: ErrorClass,
    /// Whether an `Opened` transition has been delivered for it.
    pub notified: bool,
    pub last_notified: u64,
    pub repeat_count: u32,
}

/// What the caller should act on (notify). Contains only numbers and closed enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Transition {
    Opened {
        since: u64,
        class: ErrorClass,
    },
    StillDown {
        since: u64,
        class: ErrorClass,
        repeat_count: u32,
    },
    /// `until` is the time of the first successful check that ended the incident.
    Recovered {
        since: u64,
        until: u64,
    },
    CertExpiring {
        days_left: i64,
    },
    CertOk,
}

/// Per-monitor mutable state. Feed it outcomes with [`MonitorState::observe`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitorState {
    consecutive_failures: u32,
    consecutive_successes: u32,
    streak_start: u64,
    success_start: Option<u64>,
    incident: Option<OpenIncident>,
    history: Vec<Incident>,
    last_check: Option<u64>,
    /// Highest time seen; the effective clock never goes backwards.
    high_water: u64,
    last_opened: Option<u64>,
    cert_expiring: bool,
    pending_recovery: Option<(u64, u64)>,
    suppressed: u32,
}

impl MonitorState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn is_down(&self) -> bool {
        self.incident.is_some()
    }

    #[must_use]
    pub fn open_incident(&self) -> Option<&OpenIncident> {
        self.incident.as_ref()
    }

    #[must_use]
    pub fn incidents(&self) -> &[Incident] {
        &self.history
    }

    /// Transitions swallowed by maintenance windows so far.
    #[must_use]
    pub fn suppressed(&self) -> u32 {
        self.suppressed
    }

    #[must_use]
    pub fn last_check(&self) -> Option<u64> {
        self.last_check
    }

    #[must_use]
    pub fn cert_expiring(&self) -> bool {
        self.cert_expiring
    }

    /// Whether a check is due. Never checked: due. Clock behind the last check (skew): due, so the
    /// monitor re-syncs instead of stalling until the clock catches up.
    #[must_use]
    pub fn due(&self, config: &MonitorConfig, now: u64) -> bool {
        match self.last_check {
            None => true,
            Some(last) if now < last => true,
            Some(last) => now.saturating_sub(last) >= config.interval_secs,
        }
    }

    /// Probe and observe if due. `None` when not due.
    pub fn tick(
        &mut self,
        config: &MonitorConfig,
        prober: &dyn Prober,
        now: u64,
    ) -> Option<Vec<Transition>> {
        if !self.due(config, now) {
            return None;
        }
        let outcome = prober.probe(config);
        Some(self.observe(config, &outcome, now))
    }

    /// Downtime over `[from, to)`; an open incident counts up to `to`.
    #[must_use]
    pub fn uptime(&self, from: u64, to: u64) -> UptimeStats {
        let mut iv: Vec<(u64, u64)> = self.history.iter().map(|i| (i.start, i.end)).collect();
        if let Some(inc) = &self.incident {
            iv.push((inc.since, u64::MAX));
        }
        uptime_from_intervals(&iv, from, to)
    }

    /// Feed one outcome observed at `now` (unix seconds).
    ///
    /// Semantics:
    /// * An incident opens after `fail_threshold` consecutive `Down` outcomes; its `since` is the first
    ///   failure of the streak. It closes after `recover_threshold` consecutive non-`Down` outcomes.
    /// * `CertExpiring` counts as available. It emits `Transition::CertExpiring` once on entering and
    ///   `CertOk` on the next `Up`.
    /// * In a maintenance window state still updates, but nothing is emitted (counted in
    ///   `suppressed`). An incident that opened silently is announced (`Opened`) at the first check
    ///   outside maintenance if still down; a recovery that happened inside maintenance after an
    ///   announced `Opened` is delivered at the first check outside it. An incident never announced
    ///   emits no `Recovered`.
    /// * `StillDown` repeats every `repeat_every_secs` (0 = never).
    /// * Flapping guard: `Opened` is held back until `min_reopen_secs` after the previous `Opened`.
    /// * A `now` earlier than one already seen is treated as the highest time seen.
    pub fn observe(
        &mut self,
        config: &MonitorConfig,
        outcome: &CheckOutcome,
        now: u64,
    ) -> Vec<Transition> {
        let t = now.max(self.high_water);
        self.high_water = t;
        self.last_check = Some(now);
        let maint = config.in_maintenance(t);
        let mut out = Vec::new();
        if !maint && let Some((since, until)) = self.pending_recovery.take() {
            out.push(Transition::Recovered { since, until });
        }
        match *outcome {
            CheckOutcome::Down { class } => self.on_down(config, class, t, maint, &mut out),
            CheckOutcome::Up { .. } => {
                self.on_available(config, t, maint, &mut out);
                if self.cert_expiring {
                    self.cert_expiring = false;
                    self.emit(maint, &mut out, Transition::CertOk);
                }
            }
            CheckOutcome::CertExpiring { days_left } => {
                self.on_available(config, t, maint, &mut out);
                if !self.cert_expiring {
                    self.cert_expiring = true;
                    self.emit(maint, &mut out, Transition::CertExpiring { days_left });
                }
            }
        }
        out
    }

    fn emit(&mut self, maint: bool, out: &mut Vec<Transition>, t: Transition) {
        if maint {
            self.suppressed = self.suppressed.saturating_add(1);
        } else {
            out.push(t);
        }
    }

    fn on_down(
        &mut self,
        config: &MonitorConfig,
        class: ErrorClass,
        t: u64,
        maint: bool,
        out: &mut Vec<Transition>,
    ) {
        self.consecutive_successes = 0;
        self.success_start = None;
        if self.consecutive_failures == 0 {
            self.streak_start = t;
        }
        self.consecutive_failures = self.consecutive_failures.saturating_add(1);

        if self.incident.is_none() && self.consecutive_failures >= config.fail_threshold {
            self.incident = Some(OpenIncident {
                since: self.streak_start,
                class,
                notified: false,
                last_notified: t,
                repeat_count: 0,
            });
            if maint {
                self.suppressed = self.suppressed.saturating_add(1);
            }
        }
        let flap_ok = self
            .last_opened
            .is_none_or(|o| t >= o.saturating_add(config.min_reopen_secs));
        let Some(mut inc) = self.incident else {
            return;
        };
        inc.class = class;
        if !inc.notified {
            if !maint && flap_ok {
                inc.notified = true;
                inc.last_notified = t;
                self.last_opened = Some(t);
                out.push(Transition::Opened {
                    since: inc.since,
                    class,
                });
            }
        } else if config.repeat_every_secs > 0
            && t.saturating_sub(inc.last_notified) >= config.repeat_every_secs
        {
            inc.last_notified = t;
            if maint {
                self.suppressed = self.suppressed.saturating_add(1);
            } else {
                inc.repeat_count = inc.repeat_count.saturating_add(1);
                out.push(Transition::StillDown {
                    since: inc.since,
                    class,
                    repeat_count: inc.repeat_count,
                });
            }
        }
        self.incident = Some(inc);
    }

    fn on_available(
        &mut self,
        config: &MonitorConfig,
        t: u64,
        maint: bool,
        out: &mut Vec<Transition>,
    ) {
        self.consecutive_failures = 0;
        if self.consecutive_successes == 0 {
            self.success_start = Some(t);
        }
        self.consecutive_successes = self.consecutive_successes.saturating_add(1);
        if self.consecutive_successes < config.recover_threshold {
            return;
        }
        let Some(inc) = self.incident.take() else {
            return;
        };
        let until = self.success_start.unwrap_or(t).max(inc.since);
        self.history.push(Incident {
            start: inc.since,
            end: until,
        });
        if self.history.len() > MAX_HISTORY {
            let excess = self.history.len().saturating_sub(MAX_HISTORY);
            self.history.drain(..excess);
        }
        if inc.notified {
            if maint {
                self.pending_recovery = Some((inc.since, until));
                self.suppressed = self.suppressed.saturating_add(1);
            } else {
                out.push(Transition::Recovered {
                    since: inc.since,
                    until,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{MaintenanceWindow, MonitorKind};
    use crate::outcome::FakeProber;

    const DOWN: CheckOutcome = CheckOutcome::Down {
        class: ErrorClass::Timeout,
    };
    const UP: CheckOutcome = CheckOutcome::Up { latency_ms: 10 };

    fn cfg() -> MonitorConfig {
        let mut c = MonitorConfig::new(
            "m",
            MonitorKind::Tcp {
                host: "h.test".into(),
                port: 1,
            },
            60,
            10,
        );
        c.min_reopen_secs = 0;
        c
    }

    fn opened(since: u64) -> Transition {
        Transition::Opened {
            since,
            class: ErrorClass::Timeout,
        }
    }

    #[test]
    fn opens_only_after_threshold() {
        let c = cfg();
        let mut s = MonitorState::new();
        assert!(s.observe(&c, &DOWN, 100).is_empty());
        assert!(s.observe(&c, &DOWN, 160).is_empty());
        assert!(!s.is_down());
        assert_eq!(s.observe(&c, &DOWN, 220), vec![opened(100)]);
        assert!(s.is_down());
        // Already open: no second Opened.
        assert!(s.observe(&c, &DOWN, 280).is_empty());
    }

    #[test]
    fn success_resets_failure_streak() {
        let c = cfg();
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 0);
        s.observe(&c, &DOWN, 60);
        assert!(s.observe(&c, &UP, 120).is_empty());
        s.observe(&c, &DOWN, 180);
        s.observe(&c, &DOWN, 240);
        assert!(!s.is_down());
        // Streak start restarted at 180.
        assert_eq!(s.observe(&c, &DOWN, 300), vec![opened(180)]);
    }

    #[test]
    fn recovery_and_recover_threshold() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.recover_threshold = 2;
        let mut s = MonitorState::new();
        assert_eq!(s.observe(&c, &DOWN, 100), vec![opened(100)]);
        assert!(s.observe(&c, &UP, 160).is_empty());
        // A failure in between resets the success streak.
        assert!(s.observe(&c, &DOWN, 220).is_empty());
        assert!(s.observe(&c, &UP, 280).is_empty());
        assert_eq!(
            s.observe(&c, &UP, 340),
            vec![Transition::Recovered {
                since: 100,
                until: 280
            }]
        );
        assert!(!s.is_down());
        assert_eq!(
            s.incidents(),
            &[Incident {
                start: 100,
                end: 280
            }]
        );
        assert!(s.observe(&c, &UP, 400).is_empty());
    }

    #[test]
    fn class_updates_while_down() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.repeat_every_secs = 60;
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 0);
        let out = s.observe(
            &c,
            &CheckOutcome::Down {
                class: ErrorClass::UnexpectedStatus(503),
            },
            60,
        );
        assert_eq!(
            out,
            vec![Transition::StillDown {
                since: 0,
                class: ErrorClass::UnexpectedStatus(503),
                repeat_count: 1
            }]
        );
    }

    #[test]
    fn repeat_interval() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.repeat_every_secs = 300;
        let mut s = MonitorState::new();
        assert_eq!(s.observe(&c, &DOWN, 0), vec![opened(0)]);
        assert!(s.observe(&c, &DOWN, 60).is_empty());
        assert!(s.observe(&c, &DOWN, 299).is_empty());
        assert_eq!(
            s.observe(&c, &DOWN, 300),
            vec![Transition::StillDown {
                since: 0,
                class: ErrorClass::Timeout,
                repeat_count: 1
            }]
        );
        assert!(s.observe(&c, &DOWN, 400).is_empty());
        assert_eq!(
            s.observe(&c, &DOWN, 600),
            vec![Transition::StillDown {
                since: 0,
                class: ErrorClass::Timeout,
                repeat_count: 2
            }]
        );
    }

    #[test]
    fn repeat_zero_never_repeats() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.repeat_every_secs = 0;
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 0);
        for i in 1..50u64 {
            assert!(s.observe(&c, &DOWN, i * 3600).is_empty());
        }
    }

    #[test]
    fn maintenance_suppresses_then_announces_late() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.maintenance.push(MaintenanceWindow {
            start: 100,
            end: 200,
        });
        let mut s = MonitorState::new();
        assert!(s.observe(&c, &DOWN, 100).is_empty());
        assert!(s.is_down());
        assert_eq!(s.suppressed(), 1);
        assert!(s.observe(&c, &DOWN, 150).is_empty());
        // Window over, still down: announced now, with the original start.
        assert_eq!(s.observe(&c, &DOWN, 200), vec![opened(100)]);
    }

    #[test]
    fn silent_incident_that_recovers_in_maintenance_emits_nothing() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.maintenance.push(MaintenanceWindow {
            start: 100,
            end: 200,
        });
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 100);
        assert!(s.observe(&c, &UP, 120).is_empty());
        assert!(s.observe(&c, &UP, 300).is_empty());
        assert_eq!(
            s.incidents(),
            &[Incident {
                start: 100,
                end: 120
            }]
        );
    }

    #[test]
    fn recovery_during_maintenance_is_delivered_after() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.maintenance.push(MaintenanceWindow {
            start: 100,
            end: 200,
        });
        let mut s = MonitorState::new();
        assert_eq!(s.observe(&c, &DOWN, 10), vec![opened(10)]);
        assert!(s.observe(&c, &UP, 120).is_empty());
        assert_eq!(s.suppressed(), 1);
        assert_eq!(
            s.observe(&c, &UP, 200),
            vec![Transition::Recovered {
                since: 10,
                until: 120
            }]
        );
        assert!(s.observe(&c, &UP, 260).is_empty());
    }

    #[test]
    fn repeat_in_maintenance_is_suppressed() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.repeat_every_secs = 60;
        c.maintenance.push(MaintenanceWindow {
            start: 100,
            end: 200,
        });
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 0);
        assert!(s.observe(&c, &DOWN, 120).is_empty());
        assert_eq!(s.suppressed(), 1);
        assert!(s.observe(&c, &DOWN, 150).is_empty());
        assert_eq!(s.suppressed(), 1, "not due yet");
        assert!(s.observe(&c, &DOWN, 180).is_empty());
        assert_eq!(s.suppressed(), 2);
    }

    #[test]
    fn flapping_guard_holds_back_reopen() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.min_reopen_secs = 300;
        let mut s = MonitorState::new();
        assert_eq!(s.observe(&c, &DOWN, 0), vec![opened(0)]);
        assert_eq!(
            s.observe(&c, &UP, 60),
            vec![Transition::Recovered {
                since: 0,
                until: 60
            }]
        );
        // Re-fails inside the guard: tracked, not announced, and its recovery is silent too.
        assert!(s.observe(&c, &DOWN, 120).is_empty());
        assert!(s.is_down());
        assert!(s.observe(&c, &UP, 180).is_empty());
        // Fails again, still inside the guard (opened at 0), then announced once it expires.
        assert!(s.observe(&c, &DOWN, 240).is_empty());
        assert_eq!(s.observe(&c, &DOWN, 300), vec![opened(240)]);
        // Accounting counted every incident.
        assert_eq!(s.uptime(0, 600).down_secs, 60 + 60 + 360);
    }

    #[test]
    fn cert_transitions() {
        let c = cfg();
        let mut s = MonitorState::new();
        assert!(s.observe(&c, &UP, 0).is_empty());
        assert_eq!(
            s.observe(&c, &CheckOutcome::CertExpiring { days_left: 9 }, 60),
            vec![Transition::CertExpiring { days_left: 9 }]
        );
        assert!(
            s.observe(&c, &CheckOutcome::CertExpiring { days_left: 8 }, 120)
                .is_empty()
        );
        assert!(s.cert_expiring());
        assert_eq!(s.observe(&c, &UP, 180), vec![Transition::CertOk]);
        assert!(s.observe(&c, &UP, 240).is_empty());
        // Expiring cert never opens an incident.
        assert!(!s.is_down());
    }

    #[test]
    fn cert_alert_in_maintenance_is_suppressed() {
        let mut c = cfg();
        c.maintenance.push(MaintenanceWindow { start: 0, end: 100 });
        let mut s = MonitorState::new();
        assert!(
            s.observe(&c, &CheckOutcome::CertExpiring { days_left: -1 }, 10)
                .is_empty()
        );
        assert_eq!(s.suppressed(), 1);
        assert!(s.cert_expiring());
    }

    #[test]
    fn clock_going_backwards_is_safe() {
        let mut c = cfg();
        c.fail_threshold = 1;
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 1000);
        // Clock jumps back: effective time stays 1000, nothing panics or goes negative.
        assert_eq!(
            s.observe(&c, &UP, 10),
            vec![Transition::Recovered {
                since: 1000,
                until: 1000
            }]
        );
        let inc = s.incidents().first().copied();
        assert_eq!(
            inc,
            Some(Incident {
                start: 1000,
                end: 1000
            })
        );
        assert_eq!(s.uptime(0, 2000).down_secs, 0);
        assert!(s.due(&c, 5), "behind last_check: due to resync");
        assert_eq!(
            s.uptime(500, 100),
            UptimeStats {
                total_secs: 0,
                down_secs: 0
            }
        );
    }

    #[test]
    fn due_scheduling() {
        let c = cfg();
        let mut s = MonitorState::new();
        assert!(s.due(&c, 0));
        s.observe(&c, &UP, 1000);
        assert!(!s.due(&c, 1000));
        assert!(!s.due(&c, 1059));
        assert!(s.due(&c, 1060));
        assert!(s.due(&c, 999));
    }

    #[test]
    fn tick_uses_prober_only_when_due() {
        let c = cfg();
        let mut s = MonitorState::new();
        let p = FakeProber::new([DOWN, DOWN, DOWN], UP);
        assert!(s.tick(&c, &p, 0).is_some());
        assert!(s.tick(&c, &p, 30).is_none());
        assert!(s.tick(&c, &p, 60).is_some());
        assert_eq!(s.tick(&c, &p, 120), Some(vec![opened(0)]));
        assert_eq!(
            s.tick(&c, &p, 180),
            Some(vec![Transition::Recovered {
                since: 0,
                until: 180
            }])
        );
    }

    #[test]
    fn uptime_with_open_incident() {
        let mut c = cfg();
        c.fail_threshold = 1;
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 100);
        s.observe(&c, &UP, 150);
        s.observe(&c, &DOWN, 200);
        let st = s.uptime(0, 400);
        assert_eq!((st.total_secs, st.down_secs), (400, 50 + 200));
        assert!((st.percent() - 37.5).abs() < 1e-9);
        // Window entirely before the open incident.
        assert_eq!(s.uptime(0, 100).down_secs, 0);
    }

    #[test]
    fn history_is_bounded() {
        let mut c = cfg();
        c.fail_threshold = 1;
        c.min_reopen_secs = 0;
        let mut s = MonitorState::new();
        for i in 0..(MAX_HISTORY as u64 + 10) {
            s.observe(&c, &DOWN, i * 100);
            s.observe(&c, &UP, i * 100 + 50);
        }
        assert_eq!(s.incidents().len(), MAX_HISTORY);
        assert_eq!(s.incidents().first().map(|i| i.start), Some(1000));
    }

    #[test]
    fn extreme_times_do_not_panic() {
        let mut c = cfg();
        c.fail_threshold = 1;
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, u64::MAX);
        s.observe(&c, &UP, u64::MAX);
        s.observe(&c, &DOWN, 0);
        let _ = s.uptime(0, u64::MAX);
        let _ = s.due(&c, u64::MAX);
    }

    #[test]
    fn state_serde_roundtrip() {
        let mut c = cfg();
        c.fail_threshold = 1;
        let mut s = MonitorState::new();
        s.observe(&c, &DOWN, 10);
        s.observe(&c, &UP, 20);
        s.observe(&c, &DOWN, 30);
        let json = serde_json::to_string(&s);
        assert!(json.is_ok());
        let back: Result<MonitorState, _> = serde_json::from_str(json.as_deref().unwrap_or(""));
        assert_eq!(back.ok(), Some(s));
        let cj = serde_json::to_string(&c).unwrap_or_default();
        // Old configs without the optional fields still load with defaults.
        let minimal = r#"{"id":"a","kind":{"Tcp":{"host":"h","port":1}},"interval_secs":60,"timeout_secs":5}"#;
        let loaded: Result<MonitorConfig, _> = serde_json::from_str(minimal);
        assert_eq!(loaded.map(|m| m.fail_threshold).ok(), Some(3));
        assert!(!cj.is_empty());
    }

    /// Property-style: for every short Down/Up pattern, invariants hold regardless of thresholds.
    #[test]
    fn exhaustive_patterns_keep_invariants() {
        for fail in [1u32, 2, 3] {
            for rec in [1u32, 2] {
                for pattern in 0u32..(1 << 10) {
                    let mut c = cfg();
                    c.fail_threshold = fail;
                    c.recover_threshold = rec;
                    c.repeat_every_secs = 120;
                    let mut s = MonitorState::new();
                    let (mut opens, mut recs) = (0u32, 0u32);
                    for bit in 0..10u32 {
                        let o = if pattern >> bit & 1 == 1 { DOWN } else { UP };
                        for t in s.observe(&c, &o, u64::from(bit) * 60) {
                            match t {
                                Transition::Opened { .. } => opens += 1,
                                Transition::Recovered { .. } => recs += 1,
                                _ => {}
                            }
                        }
                        // Opened/Recovered strictly alternate; Recovered never leads.
                        assert!(
                            recs <= opens && opens - recs <= 1,
                            "fail={fail} rec={rec} p={pattern:b}"
                        );
                        assert_eq!(
                            s.is_down(),
                            opens > recs,
                            "fail={fail} rec={rec} p={pattern:b}"
                        );
                    }
                    let st = s.uptime(0, 600);
                    assert!(st.down_secs <= st.total_secs);
                }
            }
        }
    }
}
