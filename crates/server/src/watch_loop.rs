//! Background monitor/alert loop and interactive Telegram bot router (M1-b).
//!
//! * `run_watch_tick` probes active `monitor_targets` via `watch::Probe`, opens/resolves
//!   `incidents`, respects `alert_rules` quiet hours (holding alerts in `pending_alerts` and
//!   flushing them when quiet hours end), and delivers via `notifier` using vault-sealed tokens.
//! * `handle_telegram_update` routes `/link`, `/status`, `/log`, `/bahasa`, `/restart`, and
//!   inline approval callbacks. Approval via Telegram is **strictly limited to tiers 0–2**; tier
//!   3–4 actions are always refused in the bot and directed to the web UI.

use crate::api::monitors::CHANNEL_TOKEN_PURPOSE;
use crate::app::App;
use crate::util::clean_text;
use actions::{Action, ServiceManagerKind};
use db::{
    AlertSeverity, ChannelKind, MonitorKind, MonitorStatus, NewProposal, NodeAccess, NodeKind,
    ProposalDecision, ProposalId,
};
use i18n::{Locale, MessageKey};
use monitor::{CheckOutcome, MonitorConfig, MonitorKind as ProbeKind};
use notifier::telegram::{BotDecision, BotEvent, Telegram, parse_bot_event};
use notifier::{HttpTransport, Secret};
use std::sync::Arc;
use watch::{MonitorSpec, NetProber, NetTransport, Probe};

#[allow(dead_code)]
const KEY_LINKED: MessageKey = MessageKey::new("bot.link.success");
#[allow(dead_code)]
const KEY_LINK_INVALID: MessageKey = MessageKey::new("bot.link.invalid");
#[allow(dead_code)]
const KEY_UNLINKED: MessageKey = MessageKey::new("bot.auth.unlinked");
#[allow(dead_code)]
const KEY_LANG_SET: MessageKey = MessageKey::new("bot.lang.updated");
#[allow(dead_code)]
const KEY_TIER_REFUSED: MessageKey = MessageKey::new("bot.proposal.tier_refused");
#[allow(dead_code)]
const KEY_APPROVED: MessageKey = MessageKey::new("bot.proposal.approved");
#[allow(dead_code)]
const KEY_REJECTED: MessageKey = MessageKey::new("bot.proposal.rejected");
#[allow(dead_code)]
const KEY_ALREADY_DECIDED: MessageKey = MessageKey::new("bot.proposal.already_decided");
#[allow(dead_code)]
const KEY_RESTART_PROPOSED: MessageKey = MessageKey::new("bot.restart.proposed");
#[allow(dead_code)]
const KEY_RESTART_INVALID: MessageKey = MessageKey::new("bot.restart.invalid");
#[allow(dead_code)]
const KEY_NO_LOCAL_NODE: MessageKey = MessageKey::new("bot.restart.no_node");
#[allow(dead_code)]
const KEY_APPROVE_BTN: MessageKey = MessageKey::new("bot.button.approve");
#[allow(dead_code)]
const KEY_REJECT_BTN: MessageKey = MessageKey::new("bot.button.reject");

fn split_host_port(target: &str, default_port: u16) -> (String, u16) {
    if let Some((h, p)) = target.rsplit_once(':')
        && let Ok(port) = p.parse::<u16>()
        && !h.is_empty()
    {
        return (h.to_owned(), port);
    }
    (target.to_owned(), default_port)
}

fn to_monitor_spec(row: &db::MonitorRow) -> Option<MonitorSpec> {
    let kind = match row.kind {
        MonitorKind::Http => ProbeKind::Http {
            url: row.target.clone(),
            expect_status: None,
            keyword: None,
        },
        MonitorKind::Tcp => {
            let (host, port) = split_host_port(&row.target, 80);
            ProbeKind::Tcp { host, port }
        }
        MonitorKind::Tls => {
            let (host, port) = split_host_port(&row.target, 443);
            ProbeKind::TlsExpiry {
                host,
                port,
                warn_days: 14,
            }
        }
    };
    let cfg = MonitorConfig::new(
        row.id.as_str(),
        kind,
        u64::from(row.interval_sec),
        u64::from(row.timeout_sec),
    );
    Some(MonitorSpec {
        config: cfg,
        allow_private: row.allow_private,
    })
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let hh: u32 = h.parse().ok()?;
    let mm: u32 = m.parse().ok()?;
    if hh < 24 && mm < 60 {
        Some(hh * 60 + mm)
    } else {
        None
    }
}

/// Whether `now_minute` (0..1440 UTC) falls inside `[start, end)` (supports midnight wrap).
#[must_use]
pub(crate) fn in_quiet_hours(now_minute: u32, start: Option<&str>, end: Option<&str>) -> bool {
    let (Some(s), Some(e)) = (start.and_then(parse_hhmm), end.and_then(parse_hhmm)) else {
        return false;
    };
    if s == e {
        return false;
    }
    if s < e {
        now_minute >= s && now_minute < e
    } else {
        now_minute >= s || now_minute < e
    }
}

fn severity_rank(s: AlertSeverity) -> u8 {
    match s {
        AlertSeverity::Info => 0,
        AlertSeverity::Warning => 1,
        AlertSeverity::Critical => 2,
    }
}

/// Runs one monitor + alert cycle using `prober` and `tg_transport` at `now_unix` seconds.
pub(crate) async fn run_watch_tick<P: Probe, T: HttpTransport>(
    app: &App,
    prober: &P,
    tg_transport: &T,
    now_unix: i64,
) {
    let targets = {
        let Ok(conn) = db::open(&app.db_path) else {
            return;
        };
        db::list_monitors(&conn, &app.scope.org).unwrap_or_default()
    };

    let mut alerts_to_dispatch: Vec<(String, String, AlertSeverity)> = Vec::new();
    for target in targets.into_iter().filter(|m| m.is_active) {
        let Some(spec) = to_monitor_spec(&target) else {
            continue;
        };
        let outcome = prober.probe(&spec, now_unix).await;
        let (new_status, cause_opt) = match &outcome {
            CheckOutcome::Up { .. } => (MonitorStatus::Up, None),
            CheckOutcome::CertExpiring { .. } => (MonitorStatus::Degraded, None),
            CheckOutcome::Down { class, .. } => (MonitorStatus::Down, Some(format!("{class:?}"))),
        };
        if let Ok(conn) = db::open(&app.db_path) {
            let _ = db::update_monitor_status(&conn, &app.scope.org, &target.id, new_status);
            match cause_opt {
                Some(cause) => {
                    let open = db::list_incidents(&conn, &app.scope.org, Some(true), 200)
                        .unwrap_or_default();
                    if !open.iter().any(|i| i.monitor_id == target.id) {
                        let _ = db::open_incident(
                            &conn,
                            &app.scope.org,
                            &target.id,
                            &cause,
                            AlertSeverity::Critical,
                        );
                        alerts_to_dispatch.push((
                            format!("Monitor down: {}", clean_text(&target.name)),
                            format!(
                                "{} ({}) failed: {cause}",
                                clean_text(&target.name),
                                clean_text(&target.target)
                            ),
                            AlertSeverity::Critical,
                        ));
                    }
                }
                None => {
                    if let Ok(resolved) =
                        db::resolve_open_incidents(&conn, &app.scope.org, &target.id)
                        && resolved > 0
                    {
                        alerts_to_dispatch.push((
                            format!("Monitor recovered: {}", clean_text(&target.name)),
                            format!(
                                "{} ({}) is up again.",
                                clean_text(&target.name),
                                clean_text(&target.target)
                            ),
                            AlertSeverity::Info,
                        ));
                    }
                }
            }
        }
    }

    let now_minute = u32::try_from(now_unix.rem_euclid(86_400) / 60).unwrap_or(0);
    let (rules, channels) = {
        let Ok(conn) = db::open(&app.db_path) else {
            return;
        };
        (
            db::list_alert_rules(&conn, &app.scope.org).unwrap_or_default(),
            db::list_channels(&conn, &app.scope.org).unwrap_or_default(),
        )
    };

    for rule in rules.into_iter().filter(|r| r.is_active) {
        let Some(channel) = channels
            .iter()
            .find(|c| c.id == rule.channel_id && c.is_active)
        else {
            continue;
        };
        let quiet = in_quiet_hours(
            now_minute,
            rule.quiet_start.as_deref(),
            rule.quiet_end.as_deref(),
        );
        if quiet {
            if let Ok(conn) = db::open(&app.db_path) {
                for (title, body, sev) in &alerts_to_dispatch {
                    if severity_rank(*sev) >= severity_rank(rule.min_severity) {
                        let _ = db::enqueue_pending_alert(
                            &conn,
                            &app.scope.org,
                            &channel.id,
                            title,
                            body,
                            *sev,
                        );
                    }
                }
            }
            continue;
        }

        // Outside quiet hours: flush pending alerts and send new alerts.
        let pending = {
            let Ok(conn) = db::open(&app.db_path) else {
                continue;
            };
            db::list_unflushed_alerts(&conn, &app.scope.org)
                .unwrap_or_default()
                .into_iter()
                .filter(|p| p.channel_id == channel.id)
                .collect::<Vec<_>>()
        };

        if channel.kind == ChannelKind::Telegram {
            let Some((token, chat_id)) = resolve_telegram_channel(app, channel) else {
                continue;
            };
            let tg = Telegram::new(tg_transport);
            for p in pending {
                let text = format!("[Delayed] {}\n{}", p.title, p.body);
                if tg.send_text(&token, chat_id, &text).await.is_ok()
                    && let Ok(conn) = db::open(&app.db_path)
                {
                    let _ = db::mark_alert_flushed(&conn, &app.scope.org, &p.id);
                }
            }
            for (title, body, sev) in &alerts_to_dispatch {
                if severity_rank(*sev) >= severity_rank(rule.min_severity) {
                    let text = format!("{title}\n{body}");
                    let _ = tg.send_text(&token, chat_id, &text).await;
                }
            }
        }
    }
}

fn resolve_telegram_channel(app: &App, channel: &db::ChannelRow) -> Option<(Secret, i64)> {
    let vault = app.vault.as_ref()?;
    let sec_id = channel.secret_id.as_ref()?;
    let conn = db::open(&app.db_path).ok()?;
    let (purpose, _) = db::get_secret(&conn, &app.scope.org, sec_id).ok()?;
    if purpose != CHANNEL_TOKEN_PURPOSE {
        return None;
    }
    let tok = vault.load(&conn, &app.scope.org, sec_id).ok()?;
    let cfg: serde_json::Value = serde_json::from_str(&channel.config_json).ok()?;
    let chat_id = cfg.get("chat_id").and_then(|v| {
        v.as_i64()
            .or_else(|| v.as_str().and_then(|s| s.parse::<i64>().ok()))
    })?;
    Some((Secret::new(tok.expose_secret().to_owned()), chat_id))
}

/// Checks whether there is any active monitor or unflushed alert to process before allocating
/// an HTTP/TLS client.
fn has_watch_work(app: &App) -> bool {
    let Ok(conn) = db::open(&app.db_path) else {
        return false;
    };
    let has_monitors = db::list_monitors(&conn, &app.scope.org)
        .map(|list| list.into_iter().any(|m| m.is_active))
        .unwrap_or(false);
    if has_monitors {
        return true;
    }
    !db::list_unflushed_alerts(&conn, &app.scope.org)
        .unwrap_or_default()
        .is_empty()
}

/// Runs the periodic monitor/alert background loop until `shutdown` fires.
///
/// The HTTP/TLS client (`net::Client`, which loads system root certificates) is allocated lazily
/// only on a tick that actually has active monitors or pending alerts, keeping idle RSS and
/// startup latency minimal.
pub(crate) async fn run_background_loop(
    app: Arc<App>,
    mut shutdown: tokio::sync::watch::Receiver<bool>,
) {
    let mut cached_client: Option<Arc<net::Client>> = None;
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(60));
    // Skip immediate first tick at startup; run every 60s.
    ticker.tick().await;
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if !has_watch_work(&app) {
                    continue;
                }
                let client = match &cached_client {
                    Some(c) => Arc::clone(c),
                    None => {
                        let Ok(c) = net::Client::new() else {
                            continue;
                        };
                        let arc = Arc::new(c);
                        cached_client = Some(Arc::clone(&arc));
                        arc
                    }
                };
                let prober = NetProber::new(Arc::clone(&client));
                let tg_transport = NetTransport::new(client);
                let now_unix = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0));
                run_watch_tick(&app, &prober, &tg_transport, now_unix).await;
            }
            _ = shutdown.changed() => {
                if *shutdown.borrow() {
                    break;
                }
            }
        }
    }
}

#[allow(dead_code)]
fn locale_from_str(s: &str) -> Locale {
    if s == "id" { Locale::Id } else { Locale::En }
}

/// Processes a single incoming Telegram Bot API update (private chat commands and callbacks).
/// Approval via Telegram is strictly restricted to tiers 0–2; tier 3–4 actions are refused and
/// directed to the web UI.
#[allow(dead_code)]
pub(crate) async fn handle_telegram_update<T: HttpTransport>(
    app: &App,
    tg_transport: &T,
    bot_token: &Secret,
    update_json: &[u8],
) {
    let Some(event) = parse_bot_event(update_json) else {
        return;
    };
    let tg = Telegram::new(tg_transport);
    match event {
        BotEvent::Link { chat_id, code } => {
            let hash = db::sha256_hex(code.as_bytes());
            let chat_str = chat_id.to_string();
            let linked = db::open(&app.db_path).ok().and_then(|conn| {
                db::verify_telegram_link(&conn, &app.scope.org, &hash, &chat_str).ok()
            });
            match linked {
                Some(row) => {
                    let loc = locale_from_str(&row.language);
                    let msg = app.catalog.get(loc, KEY_LINKED, &[]).text;
                    let _ = tg.send_text(bot_token, chat_id, &msg).await;
                }
                None => {
                    let msg = app.catalog.get(Locale::En, KEY_LINK_INVALID, &[]).text;
                    let _ = tg.send_text(bot_token, chat_id, &msg).await;
                }
            }
        }
        BotEvent::Status { chat_id } => {
            let Some((conn, loc)) = linked_conn(app, chat_id) else {
                let msg = app.catalog.get(Locale::En, KEY_UNLINKED, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            };
            let monitors = db::list_monitors(&conn, &app.scope.org).unwrap_or_default();
            let open_inc =
                db::list_incidents(&conn, &app.scope.org, Some(true), 50).unwrap_or_default();
            let mut lines = vec![format!(
                "Monitors: {} | Open incidents: {}",
                monitors.len(),
                open_inc.len()
            )];
            for m in monitors.iter().take(10) {
                lines.push(format!(
                    "- {} [{}]",
                    clean_text(&m.name),
                    m.current_status.as_str()
                ));
            }
            let _ = loc;
            let _ = tg.send_text(bot_token, chat_id, &lines.join("\n")).await;
        }
        BotEvent::Log { chat_id, target } => {
            let Some((conn, _)) = linked_conn(app, chat_id) else {
                let msg = app.catalog.get(Locale::En, KEY_UNLINKED, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            };
            let monitors = db::list_monitors(&conn, &app.scope.org).unwrap_or_default();
            let incidents = db::list_incidents(&conn, &app.scope.org, None, 10).unwrap_or_default();
            let mut lines = Vec::new();
            for inc in incidents {
                let m_name = monitors
                    .iter()
                    .find(|m| m.id == inc.monitor_id)
                    .map_or("monitor", |m| m.name.as_str());
                if let Some(ref filter) = target
                    && !m_name.contains(filter.as_str())
                {
                    continue;
                }
                lines.push(format!(
                    "- {} [{}]: {}",
                    clean_text(m_name),
                    inc.severity.as_str(),
                    clean_text(&inc.cause)
                ));
            }
            let reply = if lines.is_empty() {
                "No recent incidents.".to_owned()
            } else {
                lines.join("\n")
            };
            let _ = tg.send_text(bot_token, chat_id, &reply).await;
        }
        BotEvent::Language { chat_id, lang } => {
            let Some((conn, _)) = linked_conn(app, chat_id) else {
                let msg = app.catalog.get(Locale::En, KEY_UNLINKED, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            };
            let chat_str = chat_id.to_string();
            let _ = db::set_telegram_link_language(&conn, &app.scope.org, &chat_str, &lang);
            let loc = locale_from_str(&lang);
            let msg = app.catalog.get(loc, KEY_LANG_SET, &[("lang", &lang)]).text;
            let _ = tg.send_text(bot_token, chat_id, &msg).await;
        }
        BotEvent::Restart { chat_id, service } => {
            let Some((conn, loc)) = linked_conn(app, chat_id) else {
                let msg = app.catalog.get(Locale::En, KEY_UNLINKED, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            };
            let Ok(action) = Action::restart_service(ServiceManagerKind::Systemd, &service) else {
                let msg = app.catalog.get(loc, KEY_RESTART_INVALID, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            };
            // Tier 3–4 actions cannot be approved/run from Telegram.
            if !action.tier().approvable_from_chat_bot() {
                let msg = app.catalog.get(loc, KEY_TIER_REFUSED, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            }
            let local_node = db::list_nodes(&conn, &app.scope.org)
                .ok()
                .and_then(|nodes| {
                    nodes
                        .into_iter()
                        .find(|n| n.kind == NodeKind::Local && n.access == NodeAccess::Local)
                });
            let Some(node) = local_node else {
                let msg = app.catalog.get(loc, KEY_NO_LOCAL_NODE, &[]).text;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            };
            let title = format!("Restart {}", action.target_label());
            let Ok(prop_id) = db::create_action_proposal(
                &conn,
                &app.scope.org,
                &NewProposal {
                    conversation: None,
                    tool: action.tool(),
                    tier: action.tier().level(),
                    title: &title,
                    rationale: "Requested via Telegram /restart",
                    suggested_action: &title,
                    impact: "assistant.proposal.impact",
                    rule_id: None,
                    subject_key: None,
                    args_hash: &action.hash(),
                },
                &node.id,
                &action.canonical_json(),
            ) else {
                return;
            };
            let prompt = app
                .catalog
                .get(
                    loc,
                    KEY_RESTART_PROPOSED,
                    &[("service", action.target_label())],
                )
                .text;
            let approve_lbl = app.catalog.get(loc, KEY_APPROVE_BTN, &[]).text;
            let reject_lbl = app.catalog.get(loc, KEY_REJECT_BTN, &[]).text;
            let _ = tg
                .send_proposal_prompt(
                    bot_token,
                    chat_id,
                    &prompt,
                    prop_id.as_str(),
                    &approve_lbl,
                    &reject_lbl,
                )
                .await;
        }
        BotEvent::ProposalCallback {
            chat_id,
            callback_id,
            proposal_id,
            decision,
        } => {
            let Some((conn, loc)) = linked_conn(app, chat_id) else {
                let msg = app.catalog.get(Locale::En, KEY_UNLINKED, &[]).text;
                let _ = tg.answer_callback(bot_token, &callback_id, &msg).await;
                return;
            };
            let Ok(pid) = ProposalId::parse(&proposal_id) else {
                return;
            };
            let Ok(row) = db::get_proposal(&conn, &app.scope.org, &pid) else {
                return;
            };
            // Strict policy: tier 3–4 proposals can NEVER be approved from Telegram.
            if row.tier > 2 {
                let msg = app.catalog.get(loc, KEY_TIER_REFUSED, &[]).text;
                let _ = tg.answer_callback(bot_token, &callback_id, &msg).await;
                let _ = tg.send_text(bot_token, chat_id, &msg).await;
                return;
            }
            let db_dec = match decision {
                BotDecision::Approve => ProposalDecision::Approve,
                BotDecision::Reject => ProposalDecision::Reject,
            };
            match db::decide_proposal_with_details(
                &conn,
                &app.scope.org,
                &pid,
                db_dec,
                &app.scope.user,
                false,
                Some("telegram_bot"),
            ) {
                Ok(db::DecisionOutcome::Decided(_)) => {
                    let key = match decision {
                        BotDecision::Approve => KEY_APPROVED,
                        BotDecision::Reject => KEY_REJECTED,
                    };
                    let msg = app.catalog.get(loc, key, &[]).text;
                    let _ = tg.answer_callback(bot_token, &callback_id, &msg).await;
                    let _ = tg.send_text(bot_token, chat_id, &msg).await;
                }
                Ok(_) => {
                    let msg = app.catalog.get(loc, KEY_ALREADY_DECIDED, &[]).text;
                    let _ = tg.answer_callback(bot_token, &callback_id, &msg).await;
                }
                Err(_) => {}
            }
        }
    }
}

#[allow(dead_code)]
fn linked_conn(app: &App, chat_id: i64) -> Option<(db::Connection, Locale)> {
    let conn = db::open(&app.db_path).ok()?;
    let chat_str = chat_id.to_string();
    let link = db::get_telegram_link_by_chat(&conn, &app.scope.org, &chat_str).ok()?;
    Some((conn, locale_from_str(&link.language)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Config;
    use crate::clock::Clock;
    use bootstrap::Scope;
    use monitor::ErrorClass;
    use notifier::testing::FakeTransport;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use vault::Vault;

    struct FakeProber {
        down: AtomicBool,
    }

    impl Probe for FakeProber {
        fn probe(
            &self,
            _spec: &MonitorSpec,
            _now: i64,
        ) -> impl std::future::Future<Output = CheckOutcome> + Send {
            let is_down = self.down.load(Ordering::Relaxed);
            async move {
                if is_down {
                    CheckOutcome::Down {
                        class: ErrorClass::Timeout,
                    }
                } else {
                    CheckOutcome::Up { latency_ms: 12 }
                }
            }
        }
    }

    fn test_app() -> Result<(App, std::path::PathBuf), Box<dyn std::error::Error>> {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("nodivara-watch-test-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let db_path = dir.join("test.db");
        let conn = db::open(&db_path)?;
        let org = db::create_org(&conn, "Watch Org", "watch-org", "free")?.id;
        let user = db::create_user(&conn, "watch@example.test", "Watch User", None)?.id;
        let _ = db::add_member(&conn, &org, &user, db::Role::Owner)?;
        let _ = db::create_node(
            &conn,
            &org,
            &db::NewNode {
                name: "local-box",
                kind: NodeKind::Local,
                access: NodeAccess::Local,
                labels_json: "{}",
            },
        )?;
        let vault = Arc::new(Vault::new(
            "k1:00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff",
            &[],
        )?);
        let cfg = Config {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            db_path,
            scope: Scope { org, user },
            vault: Some(vault),
            clock: Clock::system(),
            catalog: i18n::Catalog::builtin()?,
            provider_test_timeout: std::time::Duration::from_secs(5),
            executor: None,
            enable_watch: false,
            ssh_runner: None,
            logs_dir: None,
            sites_dir: None,
        };
        Ok((
            App::new(cfg, SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 7878)),
            dir,
        ))
    }

    #[tokio::test]
    async fn watch_tick_holds_alerts_during_quiet_hours_and_flushes_after()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, dir) = test_app()?;
        let conn = db::open(&app.db_path)?;
        let vault = app.vault.as_ref().ok_or("missing vault")?;
        let tok = format!("{}:{}", "1".repeat(9), "Z".repeat(35));
        let sec = vault.store(&conn, &app.scope.org, CHANNEL_TOKEN_PURPOSE, tok.as_bytes())?;
        let ch = db::create_channel(
            &conn,
            &app.scope.org,
            &db::NewChannel {
                kind: ChannelKind::Telegram,
                name: "ops",
                config_json: r#"{"chat_id":"12345"}"#,
                secret: Some(&sec),
            },
        )?;
        let _rule = db::create_alert_rule(
            &conn,
            &app.scope.org,
            &db::NewAlertRule {
                name: "night",
                channel: &ch,
                min_severity: AlertSeverity::Info,
                quiet_start: Some("22:00"),
                quiet_end: Some("07:00"),
            },
        )?;
        let _mon = db::create_monitor(
            &conn,
            &app.scope.org,
            &db::NewMonitor {
                name: "web",
                kind: MonitorKind::Http,
                target: "https://example.test",
                interval_sec: 60,
                timeout_sec: 5,
                allow_private: false,
            },
        )?;
        drop(conn);

        let prober = FakeProber {
            down: AtomicBool::new(true),
        };
        let tg = FakeTransport::default();

        // 23:00 UTC -> inside quiet hours [22:00, 07:00): alert is queued, nothing sent.
        run_watch_tick(&app, &prober, &tg, 23 * 3600).await;
        assert_eq!(tg.requests().len(), 0);
        let conn = db::open(&app.db_path)?;
        assert_eq!(db::list_unflushed_alerts(&conn, &app.scope.org)?.len(), 1);
        assert_eq!(
            db::list_incidents(&conn, &app.scope.org, Some(true), 10)?.len(),
            1
        );
        drop(conn);

        // 08:00 UTC + recovered -> outside quiet hours: flushes delayed alert + sends recovery.
        prober.down.store(false, Ordering::Relaxed);
        run_watch_tick(&app, &prober, &tg, 8 * 3600).await;
        assert_eq!(tg.requests().len(), 2);
        let conn = db::open(&app.db_path)?;
        assert!(db::list_unflushed_alerts(&conn, &app.scope.org)?.is_empty());
        assert!(db::list_incidents(&conn, &app.scope.org, Some(true), 10)?.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }

    #[tokio::test]
    async fn telegram_bot_routes_commands_and_blocks_tier3_4_approvals()
    -> Result<(), Box<dyn std::error::Error>> {
        let (app, dir) = test_app()?;
        let conn = db::open(&app.db_path)?;
        let code = "BOTCODE99";
        db::create_telegram_link(
            &conn,
            &app.scope.org,
            &app.scope.user,
            &db::sha256_hex(code.as_bytes()),
            "en",
        )?;
        // Also create a tier 3 proposal to verify bot callback refuses it.
        let crit = Action::restart_service(ServiceManagerKind::Systemd, "sshd.service")?;
        let local_id = db::list_nodes(&conn, &app.scope.org)?[0].id.clone();
        let tier3_id = db::create_action_proposal(
            &conn,
            &app.scope.org,
            &NewProposal {
                conversation: None,
                tool: crit.tool(),
                tier: 3,
                title: "Restart sshd",
                rationale: "test",
                suggested_action: "restart",
                impact: "assistant.proposal.impact",
                rule_id: None,
                subject_key: None,
                args_hash: &crit.hash(),
            },
            &local_id,
            &crit.canonical_json(),
        )?;
        drop(conn);

        let tg = FakeTransport::default();
        let token = Secret::new(format!("{}:{}", "1".repeat(9), "Z".repeat(35)));

        // 1. /link
        handle_telegram_update(
            &app,
            &tg,
            &token,
            br#"{"message":{"chat":{"id":77,"type":"private"},"text":"/link BOTCODE99"}}"#,
        )
        .await;
        // 2. /status & /bahasa id & /log
        handle_telegram_update(
            &app,
            &tg,
            &token,
            br#"{"message":{"chat":{"id":77,"type":"private"},"text":"/status"}}"#,
        )
        .await;
        handle_telegram_update(
            &app,
            &tg,
            &token,
            br#"{"message":{"chat":{"id":77,"type":"private"},"text":"/bahasa id"}}"#,
        )
        .await;
        handle_telegram_update(
            &app,
            &tg,
            &token,
            br#"{"message":{"chat":{"id":77,"type":"private"},"text":"/log"}}"#,
        )
        .await;
        // 3. /restart sshd (tier 3) -> refused!
        handle_telegram_update(
            &app,
            &tg,
            &token,
            br#"{"message":{"chat":{"id":77,"type":"private"},"text":"/restart sshd"}}"#,
        )
        .await;
        // 4. /restart nginx (tier 2) -> creates proposal + sends inline keyboard.
        handle_telegram_update(
            &app,
            &tg,
            &token,
            br#"{"message":{"chat":{"id":77,"type":"private"},"text":"/restart nginx"}}"#,
        )
        .await;

        let conn = db::open(&app.db_path)?;
        let props = db::list_proposals(&conn, &app.scope.org, None, 10)?;
        let tier2 = props
            .iter()
            .find(|p| p.tier == 2)
            .ok_or("missing tier 2 proposal")?
            .id
            .clone();
        drop(conn);

        // 5. Callback approve tier 3 -> MUST BE REFUSED (stays pending).
        let cb3 = format!(
            r#"{{"callback_query":{{"id":"cb3","data":"approve:{}","message":{{"chat":{{"id":77,"type":"private"}}}}}}}}"#,
            tier3_id.as_str()
        );
        handle_telegram_update(&app, &tg, &token, cb3.as_bytes()).await;
        let conn = db::open(&app.db_path)?;
        assert_eq!(
            db::get_proposal(&conn, &app.scope.org, &tier3_id)?.status,
            db::ProposalStatus::Pending
        );
        drop(conn);

        // 6. Callback approve tier 2 -> APPROVED!
        let cb2 = format!(
            r#"{{"callback_query":{{"id":"cb2","data":"approve:{}","message":{{"chat":{{"id":77,"type":"private"}}}}}}}}"#,
            tier2.as_str()
        );
        handle_telegram_update(&app, &tg, &token, cb2.as_bytes()).await;
        let conn = db::open(&app.db_path)?;
        assert_eq!(
            db::get_proposal(&conn, &app.scope.org, &tier2)?.status,
            db::ProposalStatus::Approved
        );
        let _ = std::fs::remove_dir_all(&dir);
        Ok(())
    }
}
