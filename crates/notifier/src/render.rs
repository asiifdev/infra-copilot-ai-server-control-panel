//! Turns an [`AlertEvent`] into a localized, channel-neutral [`Message`].

use crate::event::{AlertEvent, AlertKind, ErrorClass, sanitise_target, sanitise_text};
use crate::{Message, Severity};
use i18n::{Catalog, Locale, MessageKey};

const TITLE_DOWN: MessageKey = MessageKey::new("notify.down.title");
const TITLE_STILL_DOWN: MessageKey = MessageKey::new("notify.still_down.title");
const TITLE_RECOVERED: MessageKey = MessageKey::new("notify.recovered.title");
const TITLE_CERT_EXPIRING: MessageKey = MessageKey::new("notify.cert_expiring.title");
const TITLE_CERT_OK: MessageKey = MessageKey::new("notify.cert_ok.title");
const TITLE_TEST: MessageKey = MessageKey::new("notify.test.title");
const BODY_TEST: MessageKey = MessageKey::new("notify.test.body");
const LINE_TARGET: MessageKey = MessageKey::new("notify.line.target");
const LINE_REASON: MessageKey = MessageKey::new("notify.line.reason");
const LINE_STATUS: MessageKey = MessageKey::new("notify.line.status");
const LINE_SINCE: MessageKey = MessageKey::new("notify.line.since");
const LINE_RESOLVED: MessageKey = MessageKey::new("notify.line.resolved");
const LINE_EXPIRES: MessageKey = MessageKey::new("notify.line.expires");
const LINE_REPEATS: MessageKey = MessageKey::new("notify.line.repeats");
const LINE_DAYS_LEFT: MessageKey = MessageKey::new("notify.line.days_left");

const fn reason_key(class: ErrorClass) -> MessageKey {
    match class {
        ErrorClass::Dns => MessageKey::new("notify.error.dns"),
        ErrorClass::Connect => MessageKey::new("notify.error.connect"),
        ErrorClass::Tls => MessageKey::new("notify.error.tls"),
        ErrorClass::Timeout => MessageKey::new("notify.error.timeout"),
        ErrorClass::HttpStatus => MessageKey::new("notify.error.http"),
        ErrorClass::Other => MessageKey::new("notify.error.other"),
    }
}

/// Formats unix seconds as `YYYY-MM-DD HH:MM UTC` (locale neutral). Out-of-range input is clamped.
pub fn format_utc(ts: i64) -> String {
    const LIMIT: i64 = 253_402_300_799; // 9999-12-31T23:59:59Z
    let ts = ts.clamp(-62_135_596_800, LIMIT);
    let days = ts.div_euclid(86_400);
    let secs = ts.rem_euclid(86_400);
    // Civil-from-days (proleptic Gregorian), after H. Hinnant.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        secs / 3_600,
        secs % 3_600 / 60
    )
}

/// Renders `event` for `locale`. Names and targets are sanitised again (defence in depth); all
/// other text comes from the catalog.
pub fn render(event: &AlertEvent, locale: Locale, catalog: &Catalog) -> Message {
    let name = sanitise_text(&event.monitor_name, 100);
    let target = sanitise_target(&event.target);
    let tr = |key: MessageKey, args: &[(&str, &str)]| catalog.get(locale, key, args).text;

    let title_key = match event.kind {
        AlertKind::Down => TITLE_DOWN,
        AlertKind::StillDown => TITLE_STILL_DOWN,
        AlertKind::Recovered => TITLE_RECOVERED,
        AlertKind::CertExpiring => TITLE_CERT_EXPIRING,
        AlertKind::CertOk => TITLE_CERT_OK,
    };
    let title = tr(title_key, &[("name", &name)]);

    let mut lines = Vec::new();
    if !target.is_empty() {
        lines.push(tr(LINE_TARGET, &[("target", &target)]));
    }
    if let Some(class) = event.detail.error {
        let reason = tr(reason_key(class), &[]);
        lines.push(tr(LINE_REASON, &[("reason", &reason)]));
    }
    if let Some(status) = event.detail.http_status {
        lines.push(tr(LINE_STATUS, &[("status", &status.to_string())]));
    }
    if let Some(since) = event.since {
        lines.push(tr(LINE_SINCE, &[("time", &format_utc(since))]));
    }
    if let Some(until) = event.until {
        let key = match event.kind {
            AlertKind::CertExpiring | AlertKind::CertOk => LINE_EXPIRES,
            _ => LINE_RESOLVED,
        };
        lines.push(tr(key, &[("time", &format_utc(until))]));
    }
    if let Some(days) = event.detail.days_left {
        lines.push(tr(LINE_DAYS_LEFT, &[("days", &days.to_string())]));
    }
    if let Some(count) = event.detail.repeat_count {
        lines.push(tr(LINE_REPEATS, &[("count", &count.to_string())]));
    }
    Message {
        severity: event.kind.severity(),
        title,
        body: lines.join("\n"),
    }
}

/// The localized test message used by `NotificationChannel::test`.
pub fn render_test(locale: Locale, catalog: &Catalog) -> Message {
    Message {
        severity: Severity::Info,
        title: catalog.get(locale, TITLE_TEST, &[]).text,
        body: catalog.get(locale, BODY_TEST, &[]).text,
    }
}
