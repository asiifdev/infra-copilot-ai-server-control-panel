//! Rendering, sanitising and policy behaviour.

use i18n::{Catalog, Locale};
use notifier::{
    AlertDetail, AlertEvent, AlertKind, ChannelConfig, ErrorClass, QuietHours, Severity, Throttle,
    format_utc, render, sanitise_target, should_send,
};

fn catalog() -> Result<Catalog, String> {
    Catalog::builtin().map_err(|e| e.to_string())
}

#[test]
fn every_kind_renders_completely_in_both_locales() -> Result<(), String> {
    let catalog = catalog()?;
    let detail = AlertDetail {
        error: Some(ErrorClass::HttpStatus),
        http_status: Some(503),
        days_left: Some(5),
        repeat_count: Some(3),
    };
    for locale in Locale::ALL {
        for kind in AlertKind::ALL {
            let e = AlertEvent::new(
                kind,
                "shop",
                "https://example.test/",
                Some(0),
                Some(60),
                detail,
            );
            let m = render(&e, locale, &catalog);
            assert_eq!(m.severity, kind.severity());
            for text in [&m.title, &m.body] {
                assert!(
                    !text.contains("notify."),
                    "missing key in {kind:?}/{locale:?}: {text}"
                );
                assert!(!text.contains('{'), "unresolved placeholder: {text}");
            }
            assert!(m.title.contains("shop"));
        }
        for class in [
            ErrorClass::Dns,
            ErrorClass::Connect,
            ErrorClass::Tls,
            ErrorClass::Timeout,
            ErrorClass::HttpStatus,
            ErrorClass::Other,
        ] {
            let d = AlertDetail {
                error: Some(class),
                ..AlertDetail::default()
            };
            let e = AlertEvent::new(AlertKind::Down, "n", "h", None, None, d);
            assert!(!render(&e, locale, &catalog).body.contains("notify."));
        }
    }
    let down = AlertEvent::new(
        AlertKind::Down,
        "n",
        "h",
        None,
        None,
        AlertDetail::default(),
    );
    assert!(
        render(&down, Locale::Id, &catalog)
            .title
            .starts_with("MATI")
    );
    assert!(
        render(&down, Locale::En, &catalog)
            .title
            .starts_with("DOWN")
    );
    Ok(())
}

#[test]
fn names_and_targets_are_sanitised() {
    let e = AlertEvent::new(
        AlertKind::Down,
        "a\nb\r\u{202E}c\u{0}",
        "https://user:pw@host.test:8443/p?q=secret#frag",
        None,
        None,
        AlertDetail::default(),
    );
    assert_eq!(e.monitor_name, "a b c");
    assert_eq!(e.target, "https://host.test:8443/p");
    assert_eq!(sanitise_target("user:pw@host.test"), "host.test");
    assert_eq!(sanitise_target("host.test?x=1"), "host.test");
    assert_eq!(sanitise_target("a@b@host.test/x"), "host.test/x");
    assert_eq!(
        sanitise_target(&sanitise_target("https://u@h/p?q")),
        "https://h/p"
    );
    assert_eq!(sanitise_target(&"x".repeat(1000)).chars().count(), 200);
}

#[test]
fn formats_utc() {
    assert_eq!(format_utc(0), "1970-01-01 00:00 UTC");
    assert_eq!(format_utc(1_700_000_000), "2023-11-14 22:13 UTC");
    assert_eq!(format_utc(951_782_400), "2000-02-29 00:00 UTC");
    assert_eq!(format_utc(i64::MAX), "9999-12-31 23:59 UTC");
    assert!(format_utc(i64::MIN).starts_with("0001-01-01"));
}

fn cfg(min: Severity, quiet: Option<QuietHours>) -> ChannelConfig {
    ChannelConfig {
        chat_id: "1".into(),
        min_severity: min,
        quiet_hours: quiet,
        locale: Locale::En,
    }
}

fn quiet(start: u16, end: u16, off: i16, bypass: bool) -> Option<QuietHours> {
    Some(QuietHours {
        start_min: start,
        end_min: end,
        utc_offset_min: off,
        critical_bypasses: bypass,
    })
}

#[test]
fn min_severity_filters() {
    let c = cfg(Severity::Warning, None);
    assert!(!should_send(&c, Severity::Info, 0));
    assert!(should_send(&c, Severity::Warning, 0));
    assert!(should_send(&c, Severity::Critical, 0));
}

#[test]
fn quiet_hours_wrap_offsets_and_bypass() {
    // 22:00 to 07:00 local, UTC+7. UTC 15:00 is 22:00 local.
    let c = cfg(Severity::Info, quiet(22 * 60, 7 * 60, 420, false));
    let day = 1_700_006_400; // 2023-11-15 00:00 UTC
    let at = |h: i64, m: i64| day + h * 3600 + m * 60;
    assert!(!should_send(&c, Severity::Critical, at(15, 0))); // 22:00 local, start inclusive
    assert!(should_send(&c, Severity::Critical, at(14, 59))); // 21:59 local
    assert!(!should_send(&c, Severity::Info, at(23, 59))); // 06:59 local next day
    assert!(should_send(&c, Severity::Info, at(24, 0))); // 07:00 local, end exclusive
    assert!(should_send(&c, Severity::Info, at(5, 0))); // 12:00 local
    let b = cfg(Severity::Info, quiet(22 * 60, 7 * 60, 420, true));
    assert!(should_send(&b, Severity::Critical, at(15, 0)));
    assert!(!should_send(&b, Severity::Warning, at(15, 0)));
    // Same-day window, negative offset, pre-epoch time.
    let d = cfg(Severity::Info, quiet(60, 120, -300, false));
    assert!(!should_send(
        &d,
        Severity::Info,
        6 * 3600 + 30 * 60 - 86_400 * 3
    ));
    assert!(should_send(&d, Severity::Info, 12 * 3600));
    // Empty window never blocks.
    let e = cfg(Severity::Info, quiet(100, 100, 0, false));
    assert!(should_send(&e, Severity::Info, 100 * 60));
}

#[test]
fn throttle_is_a_sliding_window() {
    let mut t = Throttle::new(2, 60);
    assert!(t.try_acquire(100));
    assert!(t.try_acquire(110));
    assert!(!t.try_acquire(120));
    assert!(!t.try_acquire(159));
    assert!(t.try_acquire(160)); // the send at 100 left the window
    assert!(!t.try_acquire(161));
    assert!(!t.try_acquire(0)); // clock going backwards frees nothing
    let mut none = Throttle::new(0, 60);
    assert!(!none.try_acquire(0));
}
