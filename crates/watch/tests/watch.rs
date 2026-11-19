//! Behaviour tests. Local listeners on 127.0.0.1 only, fakes for the channel, secrets and clock.
//! No third-party host is contacted.

use monitor::{CheckOutcome, ErrorClass, MaintenanceWindow, MonitorConfig, MonitorKind};
use net::Client;
use notifier::telegram::Telegram;
use notifier::{
    ChannelConfig, ConfigError, Message, NotificationChannel, Secret, SendError, SendReceipt,
};
use rcgen::{CertificateParams, DnType, KeyPair, date_time_ymd};
use rustls::ServerConfig;
use rustls::crypto::ring;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::collections::{HashMap, VecDeque};
use std::error::Error;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use watch::{
    ChannelSpec, Clock, Delivery, Engine, EngineSnapshot, EnvSecrets, FakeClock, MinSeverity,
    MonitorSpec, NetProber, NetTransport, Probe, QuietSpec, SecretError, SecretSource, WatchConfig,
    WatchError,
};

type R = Result<(), Box<dyn Error>>;

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A token-shaped secret built at run time, so no literal exists in the source.
fn fake_token() -> String {
    format!("{}:{}", 7_070_707_070_u64, "QqSecretQq".repeat(3))
}

// ---------- fakes ----------

#[derive(Default)]
struct ScriptChannel {
    sent: Mutex<Vec<Message>>,
    script: Mutex<VecDeque<Result<(), SendError>>>,
    calls: AtomicUsize,
    secrets_seen: Mutex<Vec<String>>,
}

impl ScriptChannel {
    fn with_script(script: impl IntoIterator<Item = Result<(), SendError>>) -> Self {
        ScriptChannel {
            script: Mutex::new(script.into_iter().collect()),
            ..ScriptChannel::default()
        }
    }
}

struct Chan(Arc<ScriptChannel>);

impl NotificationChannel for Chan {
    fn kind(&self) -> &'static str {
        "script"
    }
    fn validate_config(&self, cfg: &ChannelConfig) -> Result<(), ConfigError> {
        if cfg.chat_id.is_empty() {
            Err(ConfigError::ChatId)
        } else {
            Ok(())
        }
    }
    async fn send(
        &self,
        msg: &Message,
        _cfg: &ChannelConfig,
        secret: &Secret,
    ) -> Result<SendReceipt, SendError> {
        let c = &self.0;
        c.calls.fetch_add(1, Ordering::SeqCst);
        lock(&c.secrets_seen).push(secret.expose_secret().to_owned());
        match lock(&c.script).pop_front() {
            Some(Err(e)) => Err(e),
            _ => {
                lock(&c.sent).push(msg.clone());
                Ok(SendReceipt::default())
            }
        }
    }
}

struct MapSecrets(HashMap<String, String>);

impl SecretSource for MapSecrets {
    async fn get(&self, name: &str) -> Result<Secret, SecretError> {
        self.0
            .get(name)
            .map(|v| Secret::new(v.clone()))
            .ok_or(SecretError::NotFound)
    }
}

const TOKEN_REF: &str = "WATCH_TEST_TOKEN";

fn secrets(token: &str) -> MapSecrets {
    MapSecrets(HashMap::from([(TOKEN_REF.to_owned(), token.to_owned())]))
}

type Script = Box<dyn Fn(&str, i64) -> CheckOutcome + Send + Sync>;

/// Scripted prober that also measures how many probes overlap.
struct ScriptProbe {
    script: Script,
    delay: Duration,
    in_flight: AtomicUsize,
    peak: AtomicUsize,
}

impl ScriptProbe {
    fn new(script: impl Fn(&str, i64) -> CheckOutcome + Send + Sync + 'static) -> Self {
        ScriptProbe {
            script: Box::new(script),
            delay: Duration::ZERO,
            in_flight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }
}

struct Prb(Arc<ScriptProbe>);

impl Probe for Prb {
    fn probe(&self, spec: &MonitorSpec, now: i64) -> impl Future<Output = CheckOutcome> + Send {
        let this = Arc::clone(&self.0);
        let id = spec.config.id.clone();
        async move {
            let now_in = this.in_flight.fetch_add(1, Ordering::SeqCst) + 1;
            this.peak.fetch_max(now_in, Ordering::SeqCst);
            if !this.delay.is_zero() {
                tokio::time::sleep(this.delay).await;
            }
            this.in_flight.fetch_sub(1, Ordering::SeqCst);
            (this.script)(&id, now)
        }
    }
}

fn down() -> CheckOutcome {
    CheckOutcome::Down {
        class: ErrorClass::ConnectFailed,
    }
}

fn up() -> CheckOutcome {
    CheckOutcome::Up { latency_ms: 3 }
}

// ---------- config helpers ----------

fn tcp_monitor(id: &str, fail: u32) -> MonitorSpec {
    let mut config = MonitorConfig::new(
        id,
        MonitorKind::Tcp {
            host: "host.invalid".into(),
            port: 80,
        },
        10,
        5,
    );
    config.fail_threshold = fail;
    MonitorSpec {
        config,
        allow_private: false,
    }
}

fn watch_cfg(monitors: Vec<MonitorSpec>) -> WatchConfig {
    WatchConfig {
        monitors,
        channel: ChannelSpec {
            chat_id: "12345".into(),
            min_severity: MinSeverity::Info,
            quiet_hours: None,
            locale: "en".into(),
        },
        token_ref: TOKEN_REF.into(),
        max_alerts_per_minute: 20,
        max_in_flight: 16,
    }
}

fn engine(
    cfg: WatchConfig,
    probe: &Arc<ScriptProbe>,
    channel: &Arc<ScriptChannel>,
) -> Result<Engine<Prb, Chan, MapSecrets>, WatchError> {
    Engine::new(
        cfg,
        Prb(Arc::clone(probe)),
        Chan(Arc::clone(channel)),
        secrets(&fake_token()),
    )
}

// ---------- engine behaviour ----------

#[tokio::test]
async fn down_alert_after_threshold_then_recovery_alert() -> R {
    let healthy = Arc::new(AtomicBool::new(false));
    let h = Arc::clone(&healthy);
    let probe = Arc::new(ScriptProbe::new(move |_, _| {
        if h.load(Ordering::SeqCst) {
            up()
        } else {
            down()
        }
    }));
    let channel = Arc::new(ScriptChannel::default());
    let mut eng = engine(watch_cfg(vec![tcp_monitor("web", 2)]), &probe, &channel)?;

    let r = eng.run_once(1_000).await;
    assert!(r.alerts.is_empty(), "first failure is below the threshold");
    let r = eng.run_once(1_010).await;
    assert_eq!(r.alerts.len(), 1);
    assert_eq!(r.alerts[0].delivery, Delivery::Sent);
    let r = eng.run_once(1_020).await;
    assert!(r.alerts.is_empty(), "no repeat inside repeat_every_secs");

    healthy.store(true, Ordering::SeqCst);
    let r = eng.run_once(1_030).await;
    assert_eq!(r.alerts.len(), 1);

    let sent = lock(&channel.sent).clone();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].title.contains("web") && sent[0].body.contains("host.invalid:80"));
    assert!(sent[1].title.contains("web"));
    assert_ne!(sent[0].title, sent[1].title);
    // Not due again before the interval.
    assert!(eng.run_once(1_035).await.probes.is_empty());
    Ok(())
}

#[tokio::test]
async fn maintenance_suppresses_alerts() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::default());
    let mut spec = tcp_monitor("web", 1);
    spec.config.maintenance = vec![MaintenanceWindow {
        start: 0,
        end: 100_000,
    }];
    let mut eng = engine(watch_cfg(vec![spec]), &probe, &channel)?;
    for t in [1_000, 1_010, 1_020] {
        assert!(eng.run_once(t).await.alerts.is_empty());
    }
    assert_eq!(channel.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn severity_policy_skips_recovery_when_min_is_critical() -> R {
    let healthy = Arc::new(AtomicBool::new(false));
    let h = Arc::clone(&healthy);
    let probe = Arc::new(ScriptProbe::new(move |_, _| {
        if h.load(Ordering::SeqCst) {
            up()
        } else {
            down()
        }
    }));
    let channel = Arc::new(ScriptChannel::default());
    let mut cfg = watch_cfg(vec![tcp_monitor("web", 1)]);
    cfg.channel.min_severity = MinSeverity::Critical;
    let mut eng = engine(cfg, &probe, &channel)?;
    assert_eq!(eng.run_once(1_000).await.alerts[0].delivery, Delivery::Sent);
    healthy.store(true, Ordering::SeqCst);
    let r = eng.run_once(1_010).await;
    assert_eq!(r.alerts[0].delivery, Delivery::SkippedByPolicy);
    assert_eq!(channel.calls.load(Ordering::SeqCst), 1);
    assert!(
        eng.snapshot().pending.is_empty(),
        "a user's severity choice is not a backlog"
    );
    Ok(())
}

#[test]
fn distinct_reasons_map_to_distinct_alert_classes() {
    use notifier::ErrorClass as N;
    let all = [
        (ErrorClass::Timeout, N::Timeout),
        (ErrorClass::DnsFailure, N::Dns),
        (ErrorClass::ConnectFailed, N::Connect),
        (ErrorClass::TlsFailure, N::Tls),
        (ErrorClass::UnexpectedStatus(500), N::HttpStatus),
        (ErrorClass::KeywordMissing, N::KeywordMissing),
        (ErrorClass::BlockedAddress, N::BlockedAddress),
        (ErrorClass::Protocol, N::Protocol),
        (ErrorClass::Other, N::Other),
    ];
    for (class, expected) in all {
        assert_eq!(watch::map_error_class(class), expected);
    }
}

#[test]
fn cert_expiring_alert_carries_days_but_no_guessed_time() {
    let spec = tcp_monitor("tls", 1);
    let event = watch::to_event(
        &spec.config,
        monitor::Transition::CertExpiring { days_left: 6 },
    );
    assert_eq!(event.detail.days_left, Some(6));
    assert_eq!(event.until, None);
}

// ---------- pending queue ----------

/// 20000 days after the epoch, midnight UTC.
const DAY0: i64 = 1_728_000_000;
const H: i64 = 3_600;

/// Quiet hours 22:00-06:00 UTC (wraps past midnight).
fn quiet_cfg(monitors: Vec<MonitorSpec>, critical_bypasses: bool) -> WatchConfig {
    let mut cfg = watch_cfg(monitors);
    cfg.channel.quiet_hours = Some(QuietSpec {
        start_min: 22 * 60,
        end_min: 6 * 60,
        utc_offset_min: 0,
        critical_bypasses,
    });
    cfg
}

fn toggle_probe() -> (Arc<AtomicBool>, Arc<ScriptProbe>) {
    let healthy = Arc::new(AtomicBool::new(false));
    let h = Arc::clone(&healthy);
    let probe = Arc::new(ScriptProbe::new(move |_, _| {
        if h.load(Ordering::SeqCst) {
            up()
        } else {
            down()
        }
    }));
    (healthy, probe)
}

fn quiet_monitor(id: &str) -> MonitorSpec {
    let mut m = tcp_monitor(id, 1);
    m.config.repeat_every_secs = 0;
    m
}

fn deliveries(r: &watch::RunReport) -> Vec<Delivery> {
    r.alerts.iter().map(|a| a.delivery).collect()
}

#[tokio::test]
async fn quiet_hours_across_midnight_defer_then_deliver() -> R {
    let (_, probe) = toggle_probe();
    let channel = Arc::new(ScriptChannel::default());
    let mut eng = engine(
        quiet_cfg(vec![quiet_monitor("web")], false),
        &probe,
        &channel,
    )?;

    let r = eng.run_once(DAY0 + 23 * H + 1_800).await;
    assert_eq!(deliveries(&r), vec![Delivery::Deferred]);
    assert!(deliveries(&eng.run_once(DAY0 + 86_400 + 2 * H).await).is_empty());
    assert!(deliveries(&eng.run_once(DAY0 + 86_400 + 6 * H - 60).await).is_empty());
    assert!(lock(&channel.sent).is_empty());
    assert_eq!(eng.snapshot().pending.len(), 1);

    let r = eng.run_once(DAY0 + 86_400 + 6 * H + 1_800).await;
    assert_eq!(deliveries(&r), vec![Delivery::SentAfterDelay]);
    let sent = lock(&channel.sent).clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].title.contains("web"));
    assert!(sent[0].body.contains("Delayed alert"), "{}", sent[0].body);
    assert!(sent[0].body.contains("23:30 UTC"), "{}", sent[0].body);
    assert!(eng.snapshot().pending.is_empty());
    Ok(())
}

#[tokio::test]
async fn critical_bypass_sends_now_but_info_waits() -> R {
    let (healthy, probe) = toggle_probe();
    let channel = Arc::new(ScriptChannel::default());
    let mut eng = engine(
        quiet_cfg(vec![quiet_monitor("web")], true),
        &probe,
        &channel,
    )?;
    let r = eng.run_once(DAY0 + 23 * H).await;
    assert_eq!(deliveries(&r), vec![Delivery::Sent]);
    healthy.store(true, Ordering::SeqCst);
    let r = eng.run_once(DAY0 + 23 * H + 600).await;
    assert_eq!(deliveries(&r), vec![Delivery::Deferred]);
    let r = eng.run_once(DAY0 + 86_400 + 7 * H).await;
    assert_eq!(deliveries(&r), vec![Delivery::SentAfterDelay]);
    assert_eq!(lock(&channel.sent).len(), 2);
    Ok(())
}

#[tokio::test]
async fn throttle_backlog_drains_in_order() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::default());
    let mut cfg = watch_cfg(vec![
        quiet_monitor("a"),
        quiet_monitor("b"),
        quiet_monitor("c"),
    ]);
    cfg.max_alerts_per_minute = 1;
    let mut eng = engine(cfg, &probe, &channel)?;
    let r = eng.run_once(1_000).await;
    assert_eq!(
        deliveries(&r),
        vec![Delivery::Sent, Delivery::Deferred, Delivery::Deferred]
    );
    assert!(deliveries(&eng.run_once(1_030).await).is_empty());
    let r = eng.run_once(1_061).await;
    assert_eq!(deliveries(&r), vec![Delivery::SentAfterDelay]);
    assert_eq!(r.alerts[0].id, "b");
    let r = eng.run_once(1_122).await;
    assert_eq!(deliveries(&r), vec![Delivery::SentAfterDelay]);
    assert_eq!(r.alerts[0].id, "c");
    let titles: Vec<String> = lock(&channel.sent)
        .iter()
        .map(|m| m.title.clone())
        .collect();
    assert_eq!(titles.len(), 3);
    for (title, name) in titles.iter().zip(["a", "b", "c"]) {
        assert!(title.contains(name), "{title}");
    }
    assert!(eng.snapshot().pending.is_empty());
    Ok(())
}

#[tokio::test]
async fn repeated_reminders_coalesce_while_held_back() -> R {
    let (_, probe) = toggle_probe();
    let channel = Arc::new(ScriptChannel::default());
    let mut spec = tcp_monitor("web", 1);
    spec.config.repeat_every_secs = 20;
    let mut eng = engine(quiet_cfg(vec![spec], false), &probe, &channel)?;
    let t = DAY0 + 23 * H;
    assert_eq!(deliveries(&eng.run_once(t).await), vec![Delivery::Deferred]);
    assert_eq!(
        deliveries(&eng.run_once(t + 30).await),
        vec![Delivery::Deferred]
    );
    let r = eng.run_once(t + 60).await;
    assert_eq!(
        deliveries(&r),
        vec![Delivery::Superseded, Delivery::Deferred],
        "the older reminder is replaced by the newer"
    );
    assert_eq!(eng.snapshot().pending.len(), 2, "one Down, one StillDown");

    // The pass after the window also raises a fresh reminder, which replaces the queued one.
    let r = eng.run_once(DAY0 + 86_400 + 7 * H).await;
    assert_eq!(
        deliveries(&r),
        vec![
            Delivery::Superseded,
            Delivery::Sent,
            Delivery::SentAfterDelay
        ]
    );
    let sent = lock(&channel.sent).clone();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].title.starts_with("DOWN"), "{}", sent[0].title);
    assert!(sent[1].title.starts_with("STILL DOWN"), "{}", sent[1].title);
    assert!(sent[1].body.contains("in a row: 3"), "{}", sent[1].body);
    Ok(())
}

#[tokio::test]
async fn the_pending_queue_is_bounded() -> R {
    let (_, probe) = toggle_probe();
    let channel = Arc::new(ScriptChannel::default());
    let n = watch::MAX_PENDING + 2;
    let monitors = (0..n).map(|i| quiet_monitor(&format!("m{i:03}"))).collect();
    let mut eng = engine(quiet_cfg(monitors, false), &probe, &channel)?;
    let r = eng.run_once(DAY0 + 23 * H).await;
    let dropped: Vec<&str> = r
        .alerts
        .iter()
        .filter(|a| a.delivery == Delivery::Dropped)
        .map(|a| a.id.as_str())
        .collect();
    assert_eq!(dropped, vec!["m000", "m001"], "oldest go first");
    assert_eq!(eng.snapshot().pending.len(), watch::MAX_PENDING);
    Ok(())
}

#[tokio::test]
async fn old_snapshots_without_a_queue_still_restore() -> R {
    let (_, probe) = toggle_probe();
    let channel = Arc::new(ScriptChannel::default());
    let cfg = quiet_cfg(vec![quiet_monitor("web")], false);
    let old: EngineSnapshot = serde_json::from_str(r#"{"states":[]}"#)?;
    assert!(old.pending.is_empty());
    let mut eng = engine(cfg.clone(), &probe, &channel)?;
    assert_eq!(eng.restore(old), 0);

    // A queue survives a JSON round trip and is delivered by the new engine, as delayed.
    eng.run_once(DAY0 + 23 * H).await;
    let mut snap: EngineSnapshot = serde_json::from_str(&serde_json::to_string(&eng.snapshot())?)?;
    assert_eq!(snap.pending.len(), 1);
    let mut stray = snap.pending[0].clone();
    stray.id = "gone".into();
    snap.pending.push(stray);
    let mut fresh = engine(cfg, &probe, &channel)?;
    assert_eq!(fresh.restore(snap), 1);
    assert_eq!(fresh.snapshot().pending.len(), 1, "unknown monitor dropped");
    let r = fresh.run_once(DAY0 + 86_400 + 7 * H).await;
    assert_eq!(deliveries(&r), vec![Delivery::SentAfterDelay]);
    assert!(lock(&channel.sent)[0].body.contains("Delayed alert"));
    Ok(())
}

#[tokio::test]
async fn recovery_replaces_an_unsent_outage_alert_and_is_never_filtered() -> R {
    for min in [MinSeverity::Info, MinSeverity::Critical] {
        let (healthy, probe) = toggle_probe();
        let channel = Arc::new(ScriptChannel::default());
        let mut cfg = quiet_cfg(vec![quiet_monitor("web")], false);
        cfg.channel.min_severity = min;
        let mut eng = engine(cfg, &probe, &channel)?;
        let t = DAY0 + 23 * H + 1_800;
        assert_eq!(deliveries(&eng.run_once(t).await), vec![Delivery::Deferred]);
        healthy.store(true, Ordering::SeqCst);
        let r = eng.run_once(t + 600).await;
        assert_eq!(
            deliveries(&r),
            vec![Delivery::Superseded, Delivery::Deferred],
            "min severity {min:?}: the outage must still be announced"
        );
        let pending = eng.snapshot().pending;
        assert_eq!(pending.len(), 1);
        assert!(pending[0].escalated);

        let r = eng.run_once(DAY0 + 86_400 + 7 * H).await;
        assert_eq!(deliveries(&r), vec![Delivery::SentAfterDelay]);
        let sent = lock(&channel.sent).clone();
        assert_eq!(sent.len(), 1, "one message tells the whole story");
        assert!(sent[0].title.starts_with("Recovered"), "{}", sent[0].title);
        assert!(sent[0].body.contains("Since: ") && sent[0].body.contains("23:30 UTC"));
        assert!(sent[0].body.contains("Resolved: ") && sent[0].body.contains("23:40 UTC"));
        assert!(sent[0].body.contains("Delayed alert"));
    }
    Ok(())
}

#[tokio::test]
async fn recovery_after_a_delivered_outage_alert_stays_plain() -> R {
    let (healthy, probe) = toggle_probe();
    let channel = Arc::new(ScriptChannel::default());
    let mut cfg = quiet_cfg(vec![quiet_monitor("web")], true);
    cfg.channel.min_severity = MinSeverity::Critical;
    let mut eng = engine(cfg, &probe, &channel)?;
    assert_eq!(
        deliveries(&eng.run_once(DAY0 + 23 * H).await),
        vec![Delivery::Sent]
    );
    healthy.store(true, Ordering::SeqCst);
    let r = eng.run_once(DAY0 + 23 * H + 600).await;
    assert_eq!(deliveries(&r), vec![Delivery::SkippedByPolicy]);
    assert!(eng.snapshot().pending.is_empty());
    Ok(())
}

#[tokio::test]
async fn throttle_caps_alerts_per_minute() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::default());
    let mut cfg = watch_cfg(vec![tcp_monitor("a", 1), tcp_monitor("b", 1)]);
    cfg.max_alerts_per_minute = 1;
    let mut eng = engine(cfg, &probe, &channel)?;
    let r = eng.run_once(1_000).await;
    let d: Vec<Delivery> = r.alerts.iter().map(|a| a.delivery).collect();
    assert_eq!(d, vec![Delivery::Sent, Delivery::Deferred]);
    Ok(())
}

#[tokio::test]
async fn rate_limit_is_retried_once_only_when_short() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let short = Arc::new(ScriptChannel::with_script([Err(SendError::RateLimited {
        retry_after_secs: 0,
    })]));
    let mut eng = engine(watch_cfg(vec![tcp_monitor("a", 1)]), &probe, &short)?;
    let r = eng.run_once(1_000).await;
    assert_eq!(r.alerts[0].delivery, Delivery::SentAfterRetry);
    assert_eq!(short.calls.load(Ordering::SeqCst), 2);

    let long = Arc::new(ScriptChannel::with_script([Err(SendError::RateLimited {
        retry_after_secs: 61,
    })]));
    let mut eng = engine(watch_cfg(vec![tcp_monitor("a", 1)]), &probe, &long)?;
    let started = Instant::now();
    let r = eng.run_once(1_000).await;
    assert_eq!(
        r.alerts[0].delivery,
        Delivery::Failed(SendError::RateLimited {
            retry_after_secs: 61
        })
    );
    assert_eq!(long.calls.load(Ordering::SeqCst), 1);
    assert!(started.elapsed() < Duration::from_secs(5), "no long wait");

    // A second rate limit on the retry is reported, not retried again.
    let twice = Arc::new(ScriptChannel::with_script([
        Err(SendError::RateLimited {
            retry_after_secs: 0,
        }),
        Err(SendError::ServerError),
    ]));
    let mut eng = engine(watch_cfg(vec![tcp_monitor("a", 1)]), &probe, &twice)?;
    let r = eng.run_once(1_000).await;
    assert_eq!(
        r.alerts[0].delivery,
        Delivery::Failed(SendError::ServerError)
    );
    assert_eq!(twice.calls.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn probes_respect_the_in_flight_bound() -> R {
    let mut p = ScriptProbe::new(|_, _| up());
    p.delay = Duration::from_millis(30);
    let probe = Arc::new(p);
    let channel = Arc::new(ScriptChannel::default());
    let monitors = (0..40).map(|i| tcp_monitor(&format!("m{i}"), 1)).collect();
    let mut cfg = watch_cfg(monitors);
    cfg.max_in_flight = 4;
    let mut eng = engine(cfg, &probe, &channel)?;
    let r = eng.run_once(1_000).await;
    assert_eq!(r.probes.len(), 40, "every due monitor is probed");
    assert_eq!(probe.peak.load(Ordering::SeqCst), 4);
    assert_eq!(probe.in_flight.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn panicking_probe_counts_as_down_and_others_still_run() -> R {
    let probe = Arc::new(ScriptProbe::new(|id, _| {
        if id == "bad" {
            // A misbehaving prober implementation must not take the engine down.
            std::panic::resume_unwind(Box::new("boom"));
        }
        up()
    }));
    let channel = Arc::new(ScriptChannel::default());
    let mut eng = engine(
        watch_cfg(vec![tcp_monitor("bad", 1), tcp_monitor("good", 1)]),
        &probe,
        &channel,
    )?;
    let r = eng.run_once(1_000).await;
    let outcome = |id: &str| r.probes.iter().find(|p| p.id == id).map(|p| p.outcome);
    assert_eq!(
        outcome("bad"),
        Some(CheckOutcome::Down {
            class: ErrorClass::Other
        })
    );
    assert_eq!(outcome("good"), Some(up()));
    Ok(())
}

#[tokio::test]
async fn run_loop_with_fake_clock_alerts_once_and_stops() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::default());
    let mut eng = engine(watch_cfg(vec![tcp_monitor("web", 2)]), &probe, &channel)?;
    let clock = FakeClock::new(1_000);
    let stop = async {
        while clock.now() < 1_100 {
            tokio::task::yield_now().await;
        }
    };
    let mut passes = 0_usize;
    eng.run(&clock, stop, Duration::from_secs(5), |_| passes += 1)
        .await;
    assert!(passes >= 10, "ran repeatedly: {passes}");
    assert_eq!(lock(&channel.sent).len(), 1, "one Down alert, no spam");
    Ok(())
}

#[tokio::test]
async fn snapshot_round_trips_through_json() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::default());
    let cfg = watch_cfg(vec![tcp_monitor("web", 1)]);
    let mut eng = engine(cfg.clone(), &probe, &channel)?;
    eng.run_once(1_000).await;
    let text = serde_json::to_string(&eng.snapshot())?;
    let back: EngineSnapshot = serde_json::from_str(&text)?;
    let mut fresh = engine(cfg, &probe, &channel)?;
    assert_eq!(fresh.restore(back), 1);
    assert!(fresh.monitors().all(|(_, s)| s.is_down()));
    // Restored state does not re-alert for the same incident.
    assert!(fresh.run_once(1_010).await.alerts.is_empty());
    Ok(())
}

// ---------- secrets ----------

#[tokio::test]
async fn token_reaches_the_channel_but_never_any_output() -> R {
    let token = fake_token();
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::with_script([Err(SendError::BadRequest)]));
    let cfg = watch_cfg(vec![tcp_monitor("a", 1), tcp_monitor("b", 1)]);
    let mut eng = Engine::new(
        cfg.clone(),
        Prb(Arc::clone(&probe)),
        Chan(Arc::clone(&channel)),
        secrets(&token),
    )?;
    let r = eng.run_once(1_000).await;
    assert_eq!(
        lock(&channel.secrets_seen).first(),
        Some(&token),
        "injected at send time"
    );

    let mut dump = format!("{r:?} {cfg:?}");
    dump.push_str(&serde_json::to_string(&eng.snapshot())?);
    dump.push_str(&serde_json::to_string(&cfg)?);
    for m in lock(&channel.sent).iter() {
        dump.push_str(&format!("{m:?}"));
    }
    for e in [WatchError::TokenRef, WatchError::Parse] {
        dump.push_str(&format!("{e} {e:?}"));
    }
    assert!(!dump.contains(&token));
    assert!(!dump.contains("QqSecret"));
    Ok(())
}

#[tokio::test]
async fn missing_secret_is_a_closed_report_not_a_panic() -> R {
    let probe = Arc::new(ScriptProbe::new(|_, _| down()));
    let channel = Arc::new(ScriptChannel::default());
    let mut eng = Engine::new(
        watch_cfg(vec![tcp_monitor("a", 1)]),
        Prb(Arc::clone(&probe)),
        Chan(Arc::clone(&channel)),
        MapSecrets(HashMap::new()),
    )?;
    let r = eng.run_once(1_000).await;
    assert_eq!(
        r.alerts[0].delivery,
        Delivery::SecretUnavailable(SecretError::NotFound)
    );
    assert_eq!(channel.calls.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn env_secret_source_reports_missing_variable() -> R {
    let missing = EnvSecrets.get("WATCH_TEST_SURELY_UNSET_VARIABLE_7").await;
    assert_eq!(missing.err(), Some(SecretError::NotFound));
    // PATH exists in every environment these tests run in; its value must stay masked.
    let present = EnvSecrets.get("PATH").await?;
    assert_eq!(format!("{present:?} {present}"), "Secret(***) ***");
    Ok(())
}

/// Telegram over `NetTransport` plugs into the engine; with no token available nothing is sent.
#[tokio::test]
async fn telegram_over_net_transport_wires_up() -> R {
    let client = Arc::new(trusting_client(&make_cert((2020, 1, 1), (2100, 1, 1))?)?);
    let prober = NetProber::new(Arc::clone(&client));
    let channel = Telegram::new(NetTransport::new(client));
    let mut eng = Engine::new(
        watch_cfg(vec![tcp_monitor("a", 1)]),
        ProbeAlwaysDown(prober),
        channel,
        EnvSecrets,
    )?;
    let r = eng.run_once(1_000).await;
    // token_ref WATCH_TEST_TOKEN is not set in the environment either.
    assert_eq!(
        r.alerts[0].delivery,
        Delivery::SecretUnavailable(SecretError::NotFound)
    );
    Ok(())
}

struct ProbeAlwaysDown(NetProber);

impl Probe for ProbeAlwaysDown {
    fn probe(&self, _spec: &MonitorSpec, _now: i64) -> impl Future<Output = CheckOutcome> + Send {
        let _ = &self.0;
        std::future::ready(down())
    }
}

// ---------- config ----------

#[test]
fn config_parses_with_defaults_and_validates() -> R {
    let json = r#"{
        "monitors": [
            {"id":"site","kind":{"Http":{"url":"https://example.test/","expect_status":null,"keyword":"ok"}},
             "interval_secs":60,"timeout_secs":10},
            {"id":"lan","kind":{"Tcp":{"host":"10.0.0.5","port":22}},"interval_secs":30,"timeout_secs":5,"allow_private":true}
        ],
        "channel": {"chat_id":"-100123","locale":"id"},
        "token_ref": "TELEGRAM_BOT_TOKEN"
    }"#;
    let cfg = WatchConfig::from_json(json)?;
    assert_eq!(cfg.validate(), Ok(()));
    assert!(!cfg.monitors[0].allow_private, "SSRF guard on by default");
    assert!(cfg.monitors[1].allow_private);
    assert_eq!((cfg.max_in_flight, cfg.max_alerts_per_minute), (16, 20));
    let again = WatchConfig::from_json(&serde_json::to_string(&cfg)?)?;
    assert_eq!(again, cfg);
    assert_eq!(WatchConfig::from_json("{nope"), Err(WatchError::Parse));
    Ok(())
}

#[test]
fn validation_returns_closed_errors() {
    let ok = || watch_cfg(vec![tcp_monitor("a", 1)]);
    assert_eq!(ok().validate(), Ok(()));

    let mut c = ok();
    c.monitors.push(tcp_monitor("a", 1));
    assert_eq!(c.validate(), Err(WatchError::DuplicateId));

    let mut c = ok();
    c.monitors[0].config.interval_secs = 1;
    assert_eq!(
        c.validate(),
        Err(WatchError::Monitor {
            index: 0,
            error: monitor::ConfigError::IntervalOutOfRange
        })
    );

    for bad in ["", "has space", "1ABC", "123:abc", "a-b", &"X".repeat(200)] {
        let mut c = ok();
        c.token_ref = bad.into();
        assert_eq!(c.validate(), Err(WatchError::TokenRef), "{bad:?}");
    }

    let mut c = ok();
    c.channel.locale = "xx".into();
    assert_eq!(c.validate(), Err(WatchError::Locale));

    let mut c = ok();
    c.max_in_flight = 0;
    assert_eq!(c.validate(), Err(WatchError::Limits));

    let mut c = ok();
    c.monitors = (0..1001)
        .map(|i| tcp_monitor(&format!("m{i}"), 1))
        .collect();
    assert_eq!(c.validate(), Err(WatchError::TooManyMonitors));
}

#[tokio::test]
async fn engine_rejects_a_channel_config_the_channel_refuses() {
    let probe = Arc::new(ScriptProbe::new(|_, _| up()));
    let channel = Arc::new(ScriptChannel::default());
    let mut cfg = watch_cfg(vec![]);
    cfg.channel.chat_id.clear();
    let built = engine(cfg, &probe, &channel);
    assert_eq!(built.err(), Some(WatchError::Channel(ConfigError::ChatId)));
}

// ---------- NetProber against local servers ----------

enum Reply {
    Body(u16, &'static str),
    Hang,
}

async fn http_server(reply: Reply) -> Result<u16, Box<dyn Error>> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let reply = Arc::clone(&reply);
            tokio::spawn(async move {
                serve(&mut s, &reply).await;
            });
        }
    });
    Ok(port)
}

async fn serve<S: AsyncReadExt + AsyncWriteExt + Unpin>(s: &mut S, reply: &Reply) {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 2048];
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut chunk).await {
            Ok(0) | Err(_) => return,
            Ok(n) => buf.extend_from_slice(chunk.get(..n).unwrap_or_default()),
        }
    }
    match reply {
        Reply::Body(status, body) => {
            let head = format!(
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = s.write_all(head.as_bytes()).await;
            let _ = s.shutdown().await;
        }
        Reply::Hang => tokio::time::sleep(Duration::from_secs(30)).await,
    }
}

struct TestCert {
    der: Vec<u8>,
    key: Vec<u8>,
}

fn make_cert(nb: (i32, u8, u8), na: (i32, u8, u8)) -> Result<TestCert, Box<dyn Error>> {
    let mut params = CertificateParams::new(vec!["localhost".to_owned()])?;
    params.not_before = date_time_ymd(nb.0, nb.1, nb.2);
    params.not_after = date_time_ymd(na.0, na.1, na.2);
    params
        .distinguished_name
        .push(DnType::CommonName, "watch test cert");
    let key = KeyPair::generate()?;
    let cert = params.self_signed(&key)?;
    Ok(TestCert {
        der: cert.der().to_vec(),
        key: key.serialize_der(),
    })
}

fn trusting_client(cert: &TestCert) -> Result<Client, net::NetError> {
    Client::with_root_certificates([cert.der.clone()])
}

async fn tls_server(cert: &TestCert) -> Result<u16, Box<dyn Error>> {
    let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(cert.der.clone())],
            PrivateKeyDer::from(PrivatePkcs8KeyDer::from(cert.key.clone())),
        )?;
    let acceptor = TlsAcceptor::from(Arc::new(config));
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                if let Ok(mut tls) = acceptor.accept(stream).await {
                    serve(&mut tls, &Reply::Body(200, "ok")).await;
                }
            });
        }
    });
    Ok(port)
}

fn http_spec(id: &str, port: u16, status: Option<(u16, u16)>, kw: Option<&str>) -> MonitorSpec {
    let mut config = MonitorConfig::new(
        id,
        MonitorKind::Http {
            url: format!("http://127.0.0.1:{port}/"),
            expect_status: status,
            keyword: kw.map(str::to_owned),
        },
        10,
        2,
    );
    config.fail_threshold = 1;
    MonitorSpec {
        config,
        allow_private: true,
    }
}

fn prober() -> Result<NetProber, Box<dyn Error>> {
    let cert = make_cert((2020, 1, 1), (2100, 1, 1))?;
    Ok(NetProber::new(Arc::new(trusting_client(&cert)?)))
}

fn is_up(o: CheckOutcome) -> bool {
    matches!(o, CheckOutcome::Up { .. })
}

#[tokio::test]
async fn http_status_and_keyword_checks() -> R {
    let p = prober()?;
    let ok = http_server(Reply::Body(200, "hello world")).await?;
    let bad = http_server(Reply::Body(500, "oops")).await?;
    let now = 1_000;
    assert!(is_up(
        p.probe_spec(&http_spec("a", ok, None, Some("hello")), now)
            .await
    ));
    assert_eq!(
        p.probe_spec(&http_spec("a", ok, None, Some("absent")), now)
            .await,
        CheckOutcome::Down {
            class: ErrorClass::KeywordMissing
        }
    );
    assert_eq!(
        p.probe_spec(&http_spec("a", bad, None, None), now).await,
        CheckOutcome::Down {
            class: ErrorClass::UnexpectedStatus(500)
        }
    );
    assert!(is_up(
        p.probe_spec(&http_spec("a", bad, Some((500, 500)), None), now)
            .await
    ));
    Ok(())
}

#[tokio::test]
async fn private_targets_are_blocked_unless_the_monitor_opts_in() -> R {
    let p = prober()?;
    let port = http_server(Reply::Body(200, "x")).await?;
    let mut spec = http_spec("a", port, None, None);
    spec.allow_private = false;
    assert_eq!(
        p.probe_spec(&spec, 1_000).await,
        CheckOutcome::Down {
            class: ErrorClass::BlockedAddress
        }
    );
    // The strict convenience entry point has no opt-in at all.
    assert_eq!(
        p.probe(&spec.config).await,
        CheckOutcome::Down {
            class: ErrorClass::BlockedAddress
        }
    );
    spec.allow_private = true;
    assert!(is_up(p.probe_spec(&spec, 1_000).await));
    Ok(())
}

#[tokio::test]
async fn tcp_check_up_and_refused() -> R {
    let p = prober()?;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();
    let spec = |port| {
        let mut s = tcp_monitor("t", 1);
        s.config.kind = MonitorKind::Tcp {
            host: "127.0.0.1".into(),
            port,
        };
        s.allow_private = true;
        s
    };
    assert!(is_up(p.probe_spec(&spec(port), 0).await));
    drop(listener);
    assert_eq!(
        p.probe_spec(&spec(port), 0).await,
        CheckOutcome::Down {
            class: ErrorClass::ConnectFailed
        }
    );
    Ok(())
}

#[tokio::test]
async fn tls_expiry_up_expiring_and_expired() -> R {
    let far = make_cert((2020, 1, 1), (2100, 1, 1))?;
    let soon = make_cert((2020, 1, 1), (2027, 1, 1))?;
    let gone = make_cert((2020, 1, 1), (2021, 1, 1))?;
    let spec = |port, warn| {
        let mut s = tcp_monitor("tls", 1);
        s.config.kind = MonitorKind::TlsExpiry {
            host: "localhost".into(),
            port,
            warn_days: warn,
        };
        s.allow_private = true;
        s
    };
    // 2027-01-01T00:00:00Z is 1_798_761_600; "now" is ten days before it.
    let now = 1_798_761_600 - 10 * 86_400;

    let p = NetProber::new(Arc::new(trusting_client(&far)?));
    assert!(is_up(
        p.probe_spec(&spec(tls_server(&far).await?, 30), now).await
    ));

    let p = NetProber::new(Arc::new(trusting_client(&soon)?));
    assert_eq!(
        p.probe_spec(&spec(tls_server(&soon).await?, 30), now).await,
        CheckOutcome::CertExpiring { days_left: 10 }
    );
    assert!(is_up(
        p.probe_spec(&spec(tls_server(&soon).await?, 5), now).await
    ));

    let p = NetProber::new(Arc::new(trusting_client(&gone)?));
    assert_eq!(
        p.probe_spec(&spec(tls_server(&gone).await?, 30), now).await,
        CheckOutcome::Down {
            class: ErrorClass::TlsFailure
        }
    );

    // A certificate the client does not trust is a TLS failure too.
    let p = NetProber::new(Arc::new(trusting_client(&far)?));
    assert_eq!(
        p.probe_spec(&spec(tls_server(&soon).await?, 30), now).await,
        CheckOutcome::Down {
            class: ErrorClass::TlsFailure
        }
    );
    Ok(())
}

#[tokio::test]
async fn a_slow_target_cannot_delay_the_others_beyond_its_timeout() -> R {
    let hang = http_server(Reply::Hang).await?;
    let fast = http_server(Reply::Body(200, "hello")).await?;
    let mut specs = vec![http_spec("slow", hang, None, None)];
    specs.extend((0..5).map(|i| http_spec(&format!("fast{i}"), fast, None, None)));
    let channel = Arc::new(ScriptChannel::default());
    let mut cfg = watch_cfg(specs);
    cfg.max_in_flight = 3;
    let mut eng = Engine::new(
        cfg,
        prober()?,
        Chan(Arc::clone(&channel)),
        secrets(&fake_token()),
    )?;
    let started = Instant::now();
    let r = eng.run_once(1_000).await;
    let took = started.elapsed();
    assert!(
        took < Duration::from_secs(4),
        "bounded by the 2 s monitor timeout, took {took:?}"
    );
    let get = |id: &str| r.probes.iter().find(|p| p.id == id).map(|p| p.outcome);
    assert_eq!(
        get("slow"),
        Some(CheckOutcome::Down {
            class: ErrorClass::Timeout
        })
    );
    assert!((0..5).all(|i| get(&format!("fast{i}")).is_some_and(is_up)));
    Ok(())
}
