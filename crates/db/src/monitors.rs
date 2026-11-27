//! Monitor targets, incidents, notification channels, alert rules, pending alerts, and
//! Telegram chat links (M1-b). Every query filters on `org_id`.

use crate::error::{Error, Result};
use crate::ids::{
    AlertRuleId, ChannelId, IncidentId, MonitorId, OrgId, PendingAlertId, SecretId, TelegramLinkId,
    UserId,
};
use crate::{new_uuid, now_utc};
use rusqlite::{Connection, OptionalExtension, params};

text_enum!(
    /// Kind of monitor probe.
    MonitorKind {
        /// HTTP/HTTPS endpoint check.
        Http => "http",
        /// TCP connect check.
        Tcp => "tcp",
        /// TLS certificate expiry check.
        Tls => "tls",
    }
);

text_enum!(
    /// Current status of a monitor target.
    MonitorStatus {
        /// Not checked yet.
        Unknown => "unknown",
        /// Healthy.
        Up => "up",
        /// Failing.
        Down => "down",
        /// Degraded (e.g. TLS expiring soon).
        Degraded => "degraded",
    }
);

text_enum!(
    /// Severity of an incident or alert.
    AlertSeverity {
        /// Informational.
        Info => "info",
        /// Warning.
        Warning => "warning",
        /// Critical.
        Critical => "critical",
    }
);

text_enum!(
    /// Kind of notification channel.
    ChannelKind {
        /// Telegram Bot API.
        Telegram => "telegram",
        /// Generic webhook.
        Webhook => "webhook",
    }
);

text_enum!(
    /// Status of a Telegram link.
    TelegramLinkStatus {
        /// Waiting for `/link <code>` or `/start <code>`.
        Pending => "pending",
        /// Linked to a chat.
        Linked => "linked",
        /// Revoked.
        Revoked => "revoked",
    }
);

/// A monitor target to create.
#[derive(Debug)]
pub struct NewMonitor<'a> {
    /// Display name (unique per org).
    pub name: &'a str,
    /// Probe kind (`http`, `tcp`, `tls`).
    pub kind: MonitorKind,
    /// Target URL or `host:port`.
    pub target: &'a str,
    /// Probe interval in seconds (10..=86400).
    pub interval_sec: u32,
    /// Probe timeout in seconds (1..=60).
    pub timeout_sec: u32,
    /// Whether private/loopback IPs are allowed.
    pub allow_private: bool,
}

/// A stored monitor target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MonitorRow {
    /// Id.
    pub id: MonitorId,
    /// Display name.
    pub name: String,
    /// Kind.
    pub kind: MonitorKind,
    /// Target.
    pub target: String,
    /// Interval in seconds.
    pub interval_sec: u32,
    /// Timeout in seconds.
    pub timeout_sec: u32,
    /// Allow private IPs.
    pub allow_private: bool,
    /// Whether active.
    pub is_active: bool,
    /// Current status.
    pub current_status: MonitorStatus,
    /// Last check timestamp.
    pub last_checked_at: Option<String>,
    /// Creation time.
    pub created_at: String,
}

/// Creates a monitor target.
///
/// # Errors
/// Constraint violation for duplicate name or out-of-range fields.
pub fn create_monitor(conn: &Connection, org: &OrgId, new: &NewMonitor<'_>) -> Result<MonitorId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO monitor_targets \
         (id, org_id, name, kind, target, interval_sec, timeout_sec, allow_private) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![
            id,
            org.as_str(),
            new.name,
            new.kind.as_str(),
            new.target,
            i64::from(new.interval_sec),
            i64::from(new.timeout_sec),
            i64::from(new.allow_private),
        ],
    )?;
    Ok(MonitorId::from_db(id))
}

fn map_monitor(row: &rusqlite::Row<'_>) -> rusqlite::Result<MonitorRow> {
    let kind: String = row.get(2)?;
    let status: String = row.get(8)?;
    Ok(MonitorRow {
        id: MonitorId::from_db(row.get(0)?),
        name: row.get(1)?,
        kind: MonitorKind::parse(&kind).unwrap_or(MonitorKind::Http),
        target: row.get(3)?,
        interval_sec: u32::try_from(row.get::<_, i64>(4)?).unwrap_or(60),
        timeout_sec: u32::try_from(row.get::<_, i64>(5)?).unwrap_or(10),
        allow_private: row.get::<_, i64>(6)? != 0,
        is_active: row.get::<_, i64>(7)? != 0,
        current_status: MonitorStatus::parse(&status).unwrap_or(MonitorStatus::Unknown),
        last_checked_at: row.get(9)?,
        created_at: row.get(10)?,
    })
}

const MONITOR_COLUMNS: &str = "id, name, kind, target, interval_sec, timeout_sec, allow_private, \
    is_active, current_status, last_checked_at, created_at";

/// Lists monitor targets of an org.
///
/// # Errors
/// Database failure.
pub fn list_monitors(conn: &Connection, org: &OrgId) -> Result<Vec<MonitorRow>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {MONITOR_COLUMNS} FROM monitor_targets WHERE org_id = ?1 ORDER BY name, id"
    ))?;
    let rows = stmt.query_map(params![org.as_str()], map_monitor)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Gets one monitor target.
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn get_monitor(conn: &Connection, org: &OrgId, id: &MonitorId) -> Result<MonitorRow> {
    conn.query_row(
        &format!("SELECT {MONITOR_COLUMNS} FROM monitor_targets WHERE org_id = ?1 AND id = ?2"),
        params![org.as_str(), id.as_str()],
        map_monitor,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Updates a monitor target's status and `last_checked_at`.
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn update_monitor_status(
    conn: &Connection,
    org: &OrgId,
    id: &MonitorId,
    status: MonitorStatus,
) -> Result<()> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE monitor_targets SET current_status = ?3, last_checked_at = ?4 \
         WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), status.as_str(), now],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

/// Deletes a monitor target (cascades to its incidents).
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn delete_monitor(conn: &Connection, org: &OrgId, id: &MonitorId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM monitor_targets WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

// ---- incidents --------------------------------------------------------------------------------

/// A stored incident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentRow {
    /// Id.
    pub id: IncidentId,
    /// Monitor id.
    pub monitor_id: MonitorId,
    /// Start time.
    pub started_at: String,
    /// Resolve time (`None` while open).
    pub resolved_at: Option<String>,
    /// Bounded cause text.
    pub cause: String,
    /// Severity.
    pub severity: AlertSeverity,
    /// User who acknowledged, if any.
    pub acknowledged_by: Option<UserId>,
    /// Acknowledge time.
    pub acknowledged_at: Option<String>,
}

fn map_incident(row: &rusqlite::Row<'_>) -> rusqlite::Result<IncidentRow> {
    let sev: String = row.get(5)?;
    Ok(IncidentRow {
        id: IncidentId::from_db(row.get(0)?),
        monitor_id: MonitorId::from_db(row.get(1)?),
        started_at: row.get(2)?,
        resolved_at: row.get(3)?,
        cause: row.get(4)?,
        severity: AlertSeverity::parse(&sev).unwrap_or(AlertSeverity::Warning),
        acknowledged_by: row.get::<_, Option<String>>(6)?.map(UserId::from_db),
        acknowledged_at: row.get(7)?,
    })
}

/// Opens an incident for `monitor`.
///
/// # Errors
/// [`Error::NotFound`] if `monitor` is not in this org; constraint violation for invalid text.
pub fn open_incident(
    conn: &Connection,
    org: &OrgId,
    monitor: &MonitorId,
    cause: &str,
    severity: AlertSeverity,
) -> Result<IncidentId> {
    get_monitor(conn, org, monitor)?;
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO incidents (id, org_id, monitor_id, cause, severity) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, org.as_str(), monitor.as_str(), cause, severity.as_str()],
    )?;
    Ok(IncidentId::from_db(id))
}

/// Resolves all open incidents for `monitor`. Returns the number resolved.
///
/// # Errors
/// Database failure.
pub fn resolve_open_incidents(
    conn: &Connection,
    org: &OrgId,
    monitor: &MonitorId,
) -> Result<usize> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE incidents SET resolved_at = ?3 \
         WHERE org_id = ?1 AND monitor_id = ?2 AND resolved_at IS NULL",
        params![org.as_str(), monitor.as_str(), now],
    )?;
    Ok(n)
}

/// Lists incidents of an org, newest first. When `only_open` is `Some(true)`, only unresolved
/// incidents are returned; `Some(false)` returns only resolved ones.
///
/// # Errors
/// Database failure.
pub fn list_incidents(
    conn: &Connection,
    org: &OrgId,
    only_open: Option<bool>,
    limit: u32,
) -> Result<Vec<IncidentRow>> {
    let filter = match only_open {
        Some(true) => "AND resolved_at IS NULL",
        Some(false) => "AND resolved_at IS NOT NULL",
        None => "",
    };
    let mut stmt = conn.prepare(&format!(
        "SELECT id, monitor_id, started_at, resolved_at, cause, severity, acknowledged_by, acknowledged_at \
         FROM incidents WHERE org_id = ?1 {filter} ORDER BY started_at DESC, id LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![org.as_str(), i64::from(limit)], map_incident)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Acknowledges an incident.
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn acknowledge_incident(
    conn: &Connection,
    org: &OrgId,
    id: &IncidentId,
    user: &UserId,
) -> Result<()> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE incidents SET acknowledged_by = ?3, acknowledged_at = ?4 \
         WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), user.as_str(), now],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

// ---- notification channels --------------------------------------------------------------------

/// A notification channel to create. Secret tokens are sealed in `secrets` and referenced by
/// `secret_id`, never stored plaintext in `config_json`.
#[derive(Debug)]
pub struct NewChannel<'a> {
    /// Channel kind (`telegram`, `webhook`).
    pub kind: ChannelKind,
    /// Display name (unique per org).
    pub name: &'a str,
    /// Non-secret JSON configuration.
    pub config_json: &'a str,
    /// Sealed secret id (e.g. Telegram bot token), if any.
    pub secret: Option<&'a SecretId>,
}

/// A stored notification channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelRow {
    /// Id.
    pub id: ChannelId,
    /// Kind.
    pub kind: ChannelKind,
    /// Name.
    pub name: String,
    /// Non-secret JSON config.
    pub config_json: String,
    /// Sealed secret reference, if any.
    pub secret_id: Option<SecretId>,
    /// Whether active.
    pub is_active: bool,
    /// Creation time.
    pub created_at: String,
}

fn map_channel(row: &rusqlite::Row<'_>) -> rusqlite::Result<ChannelRow> {
    let kind: String = row.get(1)?;
    Ok(ChannelRow {
        id: ChannelId::from_db(row.get(0)?),
        kind: ChannelKind::parse(&kind).unwrap_or(ChannelKind::Webhook),
        name: row.get(2)?,
        config_json: row.get(3)?,
        secret_id: row.get::<_, Option<String>>(4)?.map(SecretId::from_db),
        is_active: row.get::<_, i64>(5)? != 0,
        created_at: row.get(6)?,
    })
}

/// Creates a notification channel.
///
/// # Errors
/// Constraint violation for duplicate name, invalid JSON, or cross-org secret.
pub fn create_channel(conn: &Connection, org: &OrgId, new: &NewChannel<'_>) -> Result<ChannelId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO notification_channels (id, org_id, kind, name, config_json, secret_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            org.as_str(),
            new.kind.as_str(),
            new.name,
            new.config_json,
            new.secret.map(SecretId::as_str),
        ],
    )?;
    Ok(ChannelId::from_db(id))
}

/// Lists notification channels of an org.
///
/// # Errors
/// Database failure.
pub fn list_channels(conn: &Connection, org: &OrgId) -> Result<Vec<ChannelRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, kind, name, config_json, secret_id, is_active, created_at \
         FROM notification_channels WHERE org_id = ?1 ORDER BY name, id",
    )?;
    let rows = stmt.query_map(params![org.as_str()], map_channel)?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Gets one notification channel.
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn get_channel(conn: &Connection, org: &OrgId, id: &ChannelId) -> Result<ChannelRow> {
    conn.query_row(
        "SELECT id, kind, name, config_json, secret_id, is_active, created_at \
         FROM notification_channels WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
        map_channel,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Deletes a notification channel (and returns its `secret_id` if any so the caller can clean it
/// up).
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn delete_channel(conn: &Connection, org: &OrgId, id: &ChannelId) -> Result<Option<SecretId>> {
    let row = get_channel(conn, org, id)?;
    let n = conn.execute(
        "DELETE FROM notification_channels WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    if n == 0 {
        Err(Error::NotFound)
    } else {
        Ok(row.secret_id)
    }
}

// ---- alert rules & pending alerts -------------------------------------------------------------

/// An alert rule to create.
#[derive(Debug)]
pub struct NewAlertRule<'a> {
    /// Rule name.
    pub name: &'a str,
    /// Target channel.
    pub channel: &'a ChannelId,
    /// Minimum severity.
    pub min_severity: AlertSeverity,
    /// Optional quiet hours start (`HH:MM` UTC).
    pub quiet_start: Option<&'a str>,
    /// Optional quiet hours end (`HH:MM` UTC).
    pub quiet_end: Option<&'a str>,
}

/// A stored alert rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlertRuleRow {
    /// Id.
    pub id: AlertRuleId,
    /// Name.
    pub name: String,
    /// Channel id.
    pub channel_id: ChannelId,
    /// Minimum severity.
    pub min_severity: AlertSeverity,
    /// Quiet hours start (`HH:MM` UTC).
    pub quiet_start: Option<String>,
    /// Quiet hours end (`HH:MM` UTC).
    pub quiet_end: Option<String>,
    /// Active flag.
    pub is_active: bool,
    /// Creation time.
    pub created_at: String,
}

/// Creates an alert rule.
///
/// # Errors
/// [`Error::NotFound`] if the channel is not in this org; constraint violation on bad fields.
pub fn create_alert_rule(
    conn: &Connection,
    org: &OrgId,
    new: &NewAlertRule<'_>,
) -> Result<AlertRuleId> {
    get_channel(conn, org, new.channel)?;
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO alert_rules (id, org_id, name, channel_id, min_severity, quiet_start, quiet_end) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            org.as_str(),
            new.name,
            new.channel.as_str(),
            new.min_severity.as_str(),
            new.quiet_start,
            new.quiet_end,
        ],
    )?;
    Ok(AlertRuleId::from_db(id))
}

/// Lists alert rules of an org.
///
/// # Errors
/// Database failure.
pub fn list_alert_rules(conn: &Connection, org: &OrgId) -> Result<Vec<AlertRuleRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, name, channel_id, min_severity, quiet_start, quiet_end, is_active, created_at \
         FROM alert_rules WHERE org_id = ?1 ORDER BY name, id",
    )?;
    let rows = stmt.query_map(params![org.as_str()], |row| {
        let sev: String = row.get(3)?;
        Ok(AlertRuleRow {
            id: AlertRuleId::from_db(row.get(0)?),
            name: row.get(1)?,
            channel_id: ChannelId::from_db(row.get(2)?),
            min_severity: AlertSeverity::parse(&sev).unwrap_or(AlertSeverity::Warning),
            quiet_start: row.get(4)?,
            quiet_end: row.get(5)?,
            is_active: row.get::<_, i64>(6)? != 0,
            created_at: row.get(7)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// A pending alert held during quiet hours.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAlertRow {
    /// Id.
    pub id: PendingAlertId,
    /// Target channel.
    pub channel_id: ChannelId,
    /// Alert title.
    pub title: String,
    /// Alert body.
    pub body: String,
    /// Severity.
    pub severity: AlertSeverity,
    /// Creation time.
    pub created_at: String,
    /// When it was flushed, if flushed.
    pub flushed_at: Option<String>,
}

/// Enqueues an alert held during quiet hours.
///
/// # Errors
/// [`Error::NotFound`] if the channel is not in this org.
pub fn enqueue_pending_alert(
    conn: &Connection,
    org: &OrgId,
    channel: &ChannelId,
    title: &str,
    body: &str,
    severity: AlertSeverity,
) -> Result<PendingAlertId> {
    get_channel(conn, org, channel)?;
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO pending_alerts (id, org_id, channel_id, title, body, severity) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            org.as_str(),
            channel.as_str(),
            title,
            body,
            severity.as_str()
        ],
    )?;
    Ok(PendingAlertId::from_db(id))
}

/// Lists unflushed pending alerts for an org, oldest first.
///
/// # Errors
/// Database failure.
pub fn list_unflushed_alerts(conn: &Connection, org: &OrgId) -> Result<Vec<PendingAlertRow>> {
    let mut stmt = conn.prepare(
        "SELECT id, channel_id, title, body, severity, created_at, flushed_at \
         FROM pending_alerts WHERE org_id = ?1 AND flushed_at IS NULL \
         ORDER BY created_at ASC, id",
    )?;
    let rows = stmt.query_map(params![org.as_str()], |row| {
        let sev: String = row.get(4)?;
        Ok(PendingAlertRow {
            id: PendingAlertId::from_db(row.get(0)?),
            channel_id: ChannelId::from_db(row.get(1)?),
            title: row.get(2)?,
            body: row.get(3)?,
            severity: AlertSeverity::parse(&sev).unwrap_or(AlertSeverity::Warning),
            created_at: row.get(5)?,
            flushed_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Marks a pending alert as flushed.
///
/// # Errors
/// [`Error::NotFound`] when missing or in another org.
pub fn mark_alert_flushed(conn: &Connection, org: &OrgId, id: &PendingAlertId) -> Result<()> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE pending_alerts SET flushed_at = ?3 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), now],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}

// ---- telegram links ---------------------------------------------------------------------------

/// A stored Telegram link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TelegramLinkRow {
    /// Id.
    pub id: TelegramLinkId,
    /// User id.
    pub user_id: UserId,
    /// Telegram chat id (once linked).
    pub chat_id: Option<String>,
    /// Preferred language (`en` or `id`).
    pub language: String,
    /// Status (`pending`, `linked`, `revoked`).
    pub status: TelegramLinkStatus,
    /// Expiry time for the link code.
    pub expires_at: String,
    /// Verification time.
    pub verified_at: Option<String>,
    /// Creation time.
    pub created_at: String,
}

fn map_telegram_link(row: &rusqlite::Row<'_>) -> rusqlite::Result<TelegramLinkRow> {
    let status: String = row.get(4)?;
    Ok(TelegramLinkRow {
        id: TelegramLinkId::from_db(row.get(0)?),
        user_id: UserId::from_db(row.get(1)?),
        chat_id: row.get(2)?,
        language: row.get(3)?,
        status: TelegramLinkStatus::parse(&status).unwrap_or(TelegramLinkStatus::Revoked),
        expires_at: row.get(5)?,
        verified_at: row.get(6)?,
        created_at: row.get(7)?,
    })
}

/// Creates a pending Telegram link code (stored as SHA-256 hash) expiring in 15 minutes.
///
/// # Errors
/// Database failure.
pub fn create_telegram_link(
    conn: &Connection,
    org: &OrgId,
    user: &UserId,
    link_code_hash: &str,
    language: &str,
) -> Result<TelegramLinkId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO telegram_links (id, org_id, user_id, link_code_hash, language, expires_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ', 'now', '+15 minutes'))",
        params![id, org.as_str(), user.as_str(), link_code_hash, language],
    )?;
    Ok(TelegramLinkId::from_db(id))
}

/// Verifies a pending, unexpired Telegram link code and binds it to `chat_id`.
///
/// # Errors
/// [`Error::NotFound`] when the code is unknown, expired, or belongs to another org.
pub fn verify_telegram_link(
    conn: &Connection,
    org: &OrgId,
    link_code_hash: &str,
    chat_id: &str,
) -> Result<TelegramLinkRow> {
    let tx = conn.unchecked_transaction()?;
    let now = now_utc(&tx)?;
    let id: Option<String> = tx
        .query_row(
            "SELECT id FROM telegram_links \
             WHERE org_id = ?1 AND link_code_hash = ?2 AND status = 'pending' AND expires_at > ?3",
            params![org.as_str(), link_code_hash, now],
            |row| row.get(0),
        )
        .optional()?;
    let Some(id) = id else {
        return Err(Error::NotFound);
    };
    // If this chat_id was previously linked in this org, revoke the old row so UNIQUE(org_id, chat_id) holds.
    tx.execute(
        "UPDATE telegram_links SET status = 'revoked', chat_id = NULL \
         WHERE org_id = ?1 AND chat_id = ?2 AND id != ?3",
        params![org.as_str(), chat_id, id],
    )?;
    tx.execute(
        "UPDATE telegram_links SET chat_id = ?3, status = 'linked', verified_at = ?4 \
         WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id, chat_id, now],
    )?;
    let row = tx.query_row(
        "SELECT id, user_id, chat_id, language, status, expires_at, verified_at, created_at \
         FROM telegram_links WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id],
        map_telegram_link,
    )?;
    tx.commit()?;
    Ok(row)
}

/// Looks up a linked Telegram chat in `org`.
///
/// # Errors
/// [`Error::NotFound`] when not linked in this org.
pub fn get_telegram_link_by_chat(
    conn: &Connection,
    org: &OrgId,
    chat_id: &str,
) -> Result<TelegramLinkRow> {
    conn.query_row(
        "SELECT id, user_id, chat_id, language, status, expires_at, verified_at, created_at \
         FROM telegram_links WHERE org_id = ?1 AND chat_id = ?2 AND status = 'linked'",
        params![org.as_str(), chat_id],
        map_telegram_link,
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Updates the preferred language (`en` or `id`) of a linked Telegram chat.
///
/// # Errors
/// [`Error::NotFound`] when not linked in this org.
pub fn set_telegram_link_language(
    conn: &Connection,
    org: &OrgId,
    chat_id: &str,
    language: &str,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE telegram_links SET language = ?3 \
         WHERE org_id = ?1 AND chat_id = ?2 AND status = 'linked'",
        params![org.as_str(), chat_id, language],
    )?;
    if n == 0 { Err(Error::NotFound) } else { Ok(()) }
}
