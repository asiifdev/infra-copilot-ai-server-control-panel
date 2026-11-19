//! The engine: probes due monitors concurrently, feeds the state machines, turns transitions into
//! localized alerts and delivers them through one channel.

use crate::clock::Clock;
use crate::config::{MonitorSpec, WatchConfig};
use crate::error::WatchError;
use crate::probe::Probe;
use crate::secret::{SecretError, SecretSource};
use i18n::Catalog;
use monitor::{CheckOutcome, ErrorClass, MonitorConfig, MonitorKind, MonitorState, Transition};
use notifier::{
    AlertDetail, AlertEvent, AlertKind, ChannelConfig, NotificationChannel, SendError, Throttle,
    render, should_send,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;
use tokio::task::JoinSet;

/// Longest `retry_after` the engine will wait out for one retry.
const MAX_RETRY_WAIT_SECS: u32 = 60;
const THROTTLE_WINDOW_SECS: u32 = 60;
const SECS_PER_DAY: i64 = 86_400;

/// What happened to one alert. Closed: no server text, no token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Delivered on the first attempt.
    Sent,
    /// Delivered after one retry following a rate limit.
    SentAfterRetry,
    /// Held back by minimum severity or quiet hours.
    SkippedByPolicy,
    /// Held back by the per-minute alert cap.
    Throttled,
    /// The token could not be resolved.
    SecretUnavailable(SecretError),
    /// The channel refused or failed.
    Failed(SendError),
}

/// One probe result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeReport {
    /// Monitor id.
    pub id: String,
    /// What the probe found.
    pub outcome: CheckOutcome,
}

/// One alert and its fate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertReport {
    /// Monitor id.
    pub id: String,
    /// What the alert was about.
    pub kind: AlertKind,
    /// What became of it.
    pub delivery: Delivery,
}

/// Result of one [`Engine::run_once`]. Only closed enums, numbers and monitor ids.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunReport {
    /// Probes run in this pass.
    pub probes: Vec<ProbeReport>,
    /// Alerts produced in this pass.
    pub alerts: Vec<AlertReport>,
}

/// Serialisable engine state, keyed by monitor id, for a later persistence step.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineSnapshot {
    /// `(monitor id, state)` pairs.
    pub states: Vec<(String, MonitorState)>,
}

struct Entry {
    spec: MonitorSpec,
    state: MonitorState,
}

/// Owns the monitors, their states, one channel and the send policy.
pub struct Engine<P: Probe, C: NotificationChannel, S: SecretSource> {
    entries: Vec<Entry>,
    prober: Arc<P>,
    channel: C,
    secrets: S,
    token_ref: String,
    channel_cfg: ChannelConfig,
    catalog: Catalog,
    throttle: Throttle,
    max_in_flight: usize,
}

impl<P: Probe, C: NotificationChannel, S: SecretSource> Engine<P, C, S> {
    /// Validates `cfg` (monitors, ids, token name, locale, limits and the channel's own check)
    /// and builds the engine with fresh monitor states.
    pub fn new(cfg: WatchConfig, prober: P, channel: C, secrets: S) -> Result<Self, WatchError> {
        cfg.validate()?;
        let channel_cfg = cfg.channel.to_channel_config()?;
        channel
            .validate_config(&channel_cfg)
            .map_err(WatchError::Channel)?;
        let catalog = Catalog::builtin().map_err(|_| WatchError::Catalog)?;
        Ok(Engine {
            entries: cfg
                .monitors
                .into_iter()
                .map(|spec| Entry {
                    spec,
                    state: MonitorState::new(),
                })
                .collect(),
            prober: Arc::new(prober),
            channel,
            secrets,
            token_ref: cfg.token_ref,
            channel_cfg,
            catalog,
            throttle: Throttle::new(
                usize::try_from(cfg.max_alerts_per_minute).unwrap_or(usize::MAX),
                THROTTLE_WINDOW_SECS,
            ),
            max_in_flight: cfg.max_in_flight,
        })
    }

    /// Monitors and their current states, in config order.
    pub fn monitors(&self) -> impl Iterator<Item = (&MonitorSpec, &MonitorState)> {
        self.entries.iter().map(|e| (&e.spec, &e.state))
    }

    /// Copies out every monitor state (serde-serialisable).
    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            states: self
                .entries
                .iter()
                .map(|e| (e.spec.config.id.clone(), e.state.clone()))
                .collect(),
        }
    }

    /// Restores states by monitor id; ids that no longer exist are ignored, new monitors keep
    /// their fresh state. Returns how many states were applied.
    pub fn restore(&mut self, snapshot: EngineSnapshot) -> usize {
        let mut applied = 0;
        for (id, state) in snapshot.states {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.spec.config.id == id) {
                entry.state = state;
                applied += 1;
            }
        }
        applied
    }

    /// Probes every due monitor (at most `max_in_flight` at once), observes the outcomes in config
    /// order, and delivers the resulting alerts. Never panics; a probe that panics counts as
    /// `Down(Other)`. Each probe is bounded by its monitor timeout, each send by the transport.
    pub async fn run_once(&mut self, now: i64) -> RunReport {
        let now_u = u64::try_from(now).unwrap_or(0);
        let due: Vec<usize> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| e.state.due(&e.spec.config, now_u))
            .map(|(i, _)| i)
            .collect();
        let outcomes = self.probe_all(&due, now).await;

        let mut report = RunReport::default();
        let mut events: Vec<(String, AlertEvent)> = Vec::new();
        for (index, outcome) in due.iter().zip(outcomes) {
            let Some(entry) = self.entries.get_mut(*index) else {
                continue;
            };
            let transitions = entry.state.observe(&entry.spec.config, &outcome, now_u);
            let id = entry.spec.config.id.clone();
            for t in transitions {
                events.push((id.clone(), to_event(&entry.spec.config, t, now)));
            }
            report.probes.push(ProbeReport { id, outcome });
        }
        for (id, event) in events {
            let delivery = self.deliver(&event, now).await;
            report.alerts.push(AlertReport {
                id,
                kind: event.kind,
                delivery,
            });
        }
        report
    }

    /// Runs until `stop` completes: one pass per tick, then waits `tick`. `on_report` sees every
    /// pass. Dropping `stop`'s trigger side is the caller's cancellation mechanism.
    pub async fn run<K: Clock>(
        &mut self,
        clock: &K,
        stop: impl Future<Output = ()>,
        tick: Duration,
        mut on_report: impl FnMut(&RunReport),
    ) {
        let mut stop = pin!(stop);
        loop {
            let pass = self.run_once(clock.now());
            let Some(report) = until(stop.as_mut(), pass).await else {
                return;
            };
            on_report(&report);
            if until(stop.as_mut(), clock.sleep(tick)).await.is_none() {
                return;
            }
        }
    }

    async fn probe_all(&self, due: &[usize], now: i64) -> Vec<CheckOutcome> {
        let mut results = vec![
            CheckOutcome::Down {
                class: ErrorClass::Other
            };
            due.len()
        ];
        let mut set: JoinSet<(usize, CheckOutcome)> = JoinSet::new();
        let mut slots = HashMap::new();
        let mut next = 0;
        loop {
            while next < due.len() && set.len() < self.max_in_flight {
                let Some(entry) = due.get(next).and_then(|i| self.entries.get(*i)) else {
                    break;
                };
                let spec = entry.spec.clone();
                let prober = Arc::clone(&self.prober);
                let slot = next;
                let handle = set.spawn(async move { (slot, prober.probe(&spec, now).await) });
                slots.insert(handle.id(), slot);
                next += 1;
            }
            match set.join_next_with_id().await {
                None => break,
                Some(Ok((_, (slot, outcome)))) => {
                    if let Some(r) = results.get_mut(slot) {
                        *r = outcome;
                    }
                }
                // Panicked or cancelled probe: the pre-filled `Down(Other)` stands.
                Some(Err(_)) => {}
            }
        }
        results
    }

    async fn deliver(&mut self, event: &AlertEvent, now: i64) -> Delivery {
        if !should_send(&self.channel_cfg, event.kind.severity(), now) {
            return Delivery::SkippedByPolicy;
        }
        if !self.throttle.try_acquire(now) {
            return Delivery::Throttled;
        }
        let message = render(event, self.channel_cfg.locale, &self.catalog);
        let secret = match self.secrets.get(&self.token_ref).await {
            Ok(secret) => secret,
            Err(e) => return Delivery::SecretUnavailable(e),
        };
        match self
            .channel
            .send(&message, &self.channel_cfg, &secret)
            .await
        {
            Ok(_) => Delivery::Sent,
            Err(SendError::RateLimited { retry_after_secs })
                if retry_after_secs <= MAX_RETRY_WAIT_SECS =>
            {
                tokio::time::sleep(Duration::from_secs(u64::from(retry_after_secs))).await;
                match self
                    .channel
                    .send(&message, &self.channel_cfg, &secret)
                    .await
                {
                    Ok(_) => Delivery::SentAfterRetry,
                    Err(e) => Delivery::Failed(e),
                }
            }
            Err(e) => Delivery::Failed(e),
        }
    }
}

/// Polls `stop` first; `None` when it fired before `work` finished. Needs no `select!` macro.
async fn until<T>(
    mut stop: std::pin::Pin<&mut impl Future<Output = ()>>,
    work: impl Future<Output = T>,
) -> Option<T> {
    let mut work = pin!(work);
    std::future::poll_fn(|cx| {
        if stop.as_mut().poll(cx).is_ready() {
            return Poll::Ready(None);
        }
        work.as_mut().poll(cx).map(Some)
    })
    .await
}

/// Maps the monitor's closed class onto the notifier's closed class.
pub fn map_error_class(class: ErrorClass) -> notifier::ErrorClass {
    match class {
        ErrorClass::Timeout => notifier::ErrorClass::Timeout,
        ErrorClass::DnsFailure => notifier::ErrorClass::Dns,
        ErrorClass::ConnectFailed => notifier::ErrorClass::Connect,
        ErrorClass::TlsFailure => notifier::ErrorClass::Tls,
        ErrorClass::UnexpectedStatus(_) => notifier::ErrorClass::HttpStatus,
        ErrorClass::KeywordMissing
        | ErrorClass::BlockedAddress
        | ErrorClass::Protocol
        | ErrorClass::Other => notifier::ErrorClass::Other,
    }
}

fn status_of(class: ErrorClass) -> Option<u16> {
    match class {
        ErrorClass::UnexpectedStatus(code) => Some(code),
        _ => None,
    }
}

/// Target shown in alerts: the URL or `host:port` from the config (sanitised by the notifier).
fn target_of(cfg: &MonitorConfig) -> String {
    match &cfg.kind {
        MonitorKind::Http { url, .. } => url.clone(),
        MonitorKind::Tcp { host, port } | MonitorKind::TlsExpiry { host, port, .. } => {
            format!("{host}:{port}")
        }
    }
}

fn secs(t: u64) -> i64 {
    i64::try_from(t).unwrap_or(i64::MAX)
}

/// Builds the alert for one transition. Name and target come from the owner's config only.
pub fn to_event(cfg: &MonitorConfig, t: Transition, now: i64) -> AlertEvent {
    let target = target_of(cfg);
    let build =
        |kind, since, until, detail| AlertEvent::new(kind, &cfg.id, &target, since, until, detail);
    match t {
        Transition::Opened { since, class } => build(
            AlertKind::Down,
            Some(secs(since)),
            None,
            AlertDetail {
                error: Some(map_error_class(class)),
                http_status: status_of(class),
                ..AlertDetail::default()
            },
        ),
        Transition::StillDown {
            since,
            class,
            repeat_count,
        } => build(
            AlertKind::StillDown,
            Some(secs(since)),
            None,
            AlertDetail {
                error: Some(map_error_class(class)),
                http_status: status_of(class),
                repeat_count: Some(repeat_count),
                ..AlertDetail::default()
            },
        ),
        Transition::Recovered { since, until } => build(
            AlertKind::Recovered,
            Some(secs(since)),
            Some(secs(until)),
            AlertDetail::default(),
        ),
        Transition::CertExpiring { days_left } => build(
            AlertKind::CertExpiring,
            None,
            Some(now.saturating_add(days_left.saturating_mul(SECS_PER_DAY))),
            AlertDetail {
                days_left: Some(days_left),
                ..AlertDetail::default()
            },
        ),
        Transition::CertOk => build(AlertKind::CertOk, None, None, AlertDetail::default()),
    }
}
