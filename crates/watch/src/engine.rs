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
    AlertDetail, AlertEvent, AlertKind, ChannelConfig, NotificationChannel, SendError, Severity,
    Throttle, render, should_send,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::Duration;
use tokio::task::JoinSet;

/// Longest `retry_after` the engine will wait out for one retry.
const MAX_RETRY_WAIT_SECS: u32 = 60;
const THROTTLE_WINDOW_SECS: u32 = 60;
/// Most alerts held back at once; beyond this the least important oldest one is dropped.
pub const MAX_PENDING: usize = 128;

/// What happened to one alert. Closed: no server text, no token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Delivered on the first attempt.
    Sent,
    /// Delivered after one retry following a rate limit.
    SentAfterRetry,
    /// Not sent because its severity is below the user's minimum. A choice, not a loss.
    SkippedByPolicy,
    /// Held back by quiet hours or the per-minute alert cap. Kept in the pending queue and
    /// re-evaluated at every pass; reported again as `SentAfterDelay` when it goes out.
    Deferred,
    /// A previously deferred alert, delivered now (text says it was delayed).
    SentAfterDelay,
    /// Replaced by a newer alert for the same monitor (coalesced, or a recovery that carries
    /// the outage it ended). The newer alert tells the whole story.
    Superseded,
    /// Evicted because the pending queue was full (lowest severity, oldest first).
    Dropped,
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
    /// Alerts held back by quiet hours or the throttle, oldest first. Absent in old snapshots.
    #[serde(default)]
    pub pending: Vec<PendingAlert>,
}

/// An alert that was produced but not yet delivered. Stores the transition, not rendered text:
/// the message is built at delivery time from the owner's config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingAlert {
    /// Monitor id.
    pub id: String,
    /// What the monitor reported.
    pub transition: Transition,
    /// When it was raised (unix seconds).
    pub queued_at: i64,
    /// A recovery that replaced an undelivered outage alert: judged as `Critical` by the send
    /// policy, so a minimum-severity filter cannot hide an outage the user never heard about.
    #[serde(default)]
    pub escalated: bool,
    /// Index of this alert in the current pass's report; never persisted.
    #[serde(skip)]
    slot: Option<usize>,
}

impl PendingAlert {
    fn kind(&self) -> AlertKind {
        kind_of(&self.transition)
    }

    fn severity(&self) -> Severity {
        if self.escalated {
            Severity::Critical
        } else {
            self.kind().severity()
        }
    }
}

fn kind_of(t: &Transition) -> AlertKind {
    match t {
        Transition::Opened { .. } => AlertKind::Down,
        Transition::StillDown { .. } => AlertKind::StillDown,
        Transition::Recovered { .. } => AlertKind::Recovered,
        Transition::CertExpiring { .. } => AlertKind::CertExpiring,
        Transition::CertOk => AlertKind::CertOk,
    }
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
    pending: VecDeque<PendingAlert>,
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
            pending: VecDeque::new(),
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
            pending: self.pending.iter().cloned().collect(),
        }
    }

    /// Restores states by monitor id; ids that no longer exist are ignored, new monitors keep
    /// their fresh state. Pending alerts of unknown monitors are dropped and the queue is cut to
    /// [`MAX_PENDING`] (newest kept). Returns how many states were applied.
    pub fn restore(&mut self, snapshot: EngineSnapshot) -> usize {
        let mut applied = 0;
        for (id, state) in snapshot.states {
            if let Some(entry) = self.entries.iter_mut().find(|e| e.spec.config.id == id) {
                entry.state = state;
                applied += 1;
            }
        }
        let known = |id: &str| self.entries.iter().any(|e| e.spec.config.id == id);
        let mut pending: VecDeque<PendingAlert> = snapshot
            .pending
            .into_iter()
            .filter(|p| known(&p.id))
            .collect();
        while pending.len() > MAX_PENDING {
            pending.pop_front();
        }
        self.pending = pending;
        applied
    }

    /// Probes every due monitor (at most `max_in_flight` at once), observes the outcomes in config
    /// order, queues the resulting alerts and delivers every queued alert the policy allows now.
    /// Alerts held back by quiet hours or the throttle stay queued (see [`Delivery::Deferred`]). Never panics; a probe that panics counts as
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
        let mut fresh: Vec<(String, Transition)> = Vec::new();
        for (index, outcome) in due.iter().zip(outcomes) {
            let Some(entry) = self.entries.get_mut(*index) else {
                continue;
            };
            let transitions = entry.state.observe(&entry.spec.config, &outcome, now_u);
            let id = entry.spec.config.id.clone();
            for t in transitions {
                fresh.push((id.clone(), t));
            }
            report.probes.push(ProbeReport { id, outcome });
        }
        for (id, transition) in fresh {
            self.enqueue(id, transition, now, &mut report);
        }
        self.drain(now, &mut report).await;
        report
    }

    /// Puts one new alert in the queue. Rules:
    /// * below the user's minimum severity: reported `SkippedByPolicy`, not queued;
    /// * `Recovered` replaces every undelivered `Down`/`StillDown` of the monitor and is queued as
    ///   escalated, so the single message carries the real outage window and cannot be filtered;
    /// * an older queued alert of the same monitor and kind (not `Recovered`) is superseded;
    /// * at [`MAX_PENDING`] the oldest alert of the lowest severity is dropped.
    fn enqueue(&mut self, id: String, transition: Transition, now: i64, report: &mut RunReport) {
        let kind = kind_of(&transition);
        let outage_pending = |p: &PendingAlert| {
            p.id == id && matches!(p.kind(), AlertKind::Down | AlertKind::StillDown)
        };
        let escalated = kind == AlertKind::Recovered && self.pending.iter().any(outage_pending);
        let severity = if escalated {
            Severity::Critical
        } else {
            kind.severity()
        };
        if severity < self.channel_cfg.min_severity {
            report.alerts.push(AlertReport {
                id,
                kind,
                delivery: Delivery::SkippedByPolicy,
            });
            return;
        }
        let replaced = |p: &PendingAlert| {
            p.id == id
                && match kind {
                    AlertKind::Recovered => outage_pending(p),
                    _ => p.kind() == kind,
                }
        };
        let mut kept = VecDeque::with_capacity(self.pending.len() + 1);
        for item in std::mem::take(&mut self.pending) {
            if replaced(&item) {
                settle(report, &item, Delivery::Superseded);
            } else {
                kept.push_back(item);
            }
        }
        self.pending = kept;
        if self.pending.len() >= MAX_PENDING {
            let lowest = self.pending.iter().map(PendingAlert::severity).min();
            let victim = self
                .pending
                .iter()
                .position(|p| Some(p.severity()) == lowest);
            if let Some(item) = victim.and_then(|i| self.pending.remove(i)) {
                settle(report, &item, Delivery::Dropped);
            }
        }
        self.pending.push_back(PendingAlert {
            id: id.clone(),
            transition,
            queued_at: now,
            escalated,
            slot: Some(report.alerts.len()),
        });
        report.alerts.push(AlertReport {
            id,
            kind,
            delivery: Delivery::Deferred,
        });
    }

    /// Delivers queued alerts in order. Quiet hours are judged per alert (a critical one may pass
    /// a window that holds back an info one); an exhausted throttle stops the pass so order is
    /// kept. Everything not sent stays queued.
    async fn drain(&mut self, now: i64, report: &mut RunReport) {
        let mut kept = VecDeque::new();
        let mut throttled = false;
        for item in std::mem::take(&mut self.pending) {
            if throttled || !should_send(&self.channel_cfg, item.severity(), now) {
                kept.push_back(item);
                continue;
            }
            if !self.throttle.try_acquire(now) {
                throttled = true;
                kept.push_back(item);
                continue;
            }
            let Some(event) = self.event_for(&item) else {
                continue;
            };
            let delivery = match self.send(&event).await {
                Delivery::Sent | Delivery::SentAfterRetry if item.slot.is_none() => {
                    Delivery::SentAfterDelay
                }
                other => other,
            };
            settle(report, &item, delivery);
        }
        // Whatever stays queued now belongs to an earlier pass for the next report.
        for item in &mut kept {
            item.slot = None;
        }
        self.pending = kept;
    }

    /// Builds the alert for a queued item. An item from an earlier pass is marked as delayed.
    fn event_for(&self, item: &PendingAlert) -> Option<AlertEvent> {
        let entry = self.entries.iter().find(|e| e.spec.config.id == item.id)?;
        let mut event = to_event(&entry.spec.config, item.transition);
        if item.slot.is_none() {
            event.detail.delayed_since = Some(item.queued_at);
        }
        Some(event)
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

    async fn send(&self, event: &AlertEvent) -> Delivery {
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

/// Records the fate of `item` in this pass's report: updates its own line when it was raised in
/// this pass, otherwise adds one.
fn settle(report: &mut RunReport, item: &PendingAlert, delivery: Delivery) {
    match item.slot.and_then(|i| report.alerts.get_mut(i)) {
        Some(line) => line.delivery = delivery,
        None => report.alerts.push(AlertReport {
            id: item.id.clone(),
            kind: item.kind(),
            delivery,
        }),
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
        ErrorClass::KeywordMissing => notifier::ErrorClass::KeywordMissing,
        ErrorClass::BlockedAddress => notifier::ErrorClass::BlockedAddress,
        ErrorClass::Protocol => notifier::ErrorClass::Protocol,
        ErrorClass::Other => notifier::ErrorClass::Other,
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
/// `CertExpiring` shows days left only: the transition carries no exact expiry time.
pub fn to_event(cfg: &MonitorConfig, t: Transition) -> AlertEvent {
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
            None,
            AlertDetail {
                days_left: Some(days_left),
                ..AlertDetail::default()
            },
        ),
        Transition::CertOk => build(AlertKind::CertOk, None, None, AlertDetail::default()),
    }
}
