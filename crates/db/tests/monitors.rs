//! Monitor targets, incidents, notification channels, alert rules, pending alerts, and
//! Telegram links: lifecycle, secret sealing FK, and tenant isolation.

mod common;
use common::*;
use db::*;

#[test]
fn monitor_and_incident_lifecycle_and_tenant_isolation() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;

    let m = create_monitor(
        &conn,
        &a.org,
        &NewMonitor {
            name: "api-health",
            kind: MonitorKind::Http,
            target: "https://example.test/healthz",
            interval_sec: 60,
            timeout_sec: 10,
            allow_private: false,
        },
    )?;
    assert_eq!(list_monitors(&conn, &a.org)?.len(), 1);
    assert!(list_monitors(&conn, &b.org)?.is_empty());
    assert!(matches!(
        get_monitor(&conn, &b.org, &m),
        Err(Error::NotFound)
    ));

    update_monitor_status(&conn, &a.org, &m, MonitorStatus::Down)?;
    let row = get_monitor(&conn, &a.org, &m)?;
    assert_eq!(row.current_status, MonitorStatus::Down);
    assert!(row.last_checked_at.is_some());

    // Open & acknowledge & resolve incident.
    assert!(matches!(
        open_incident(&conn, &b.org, &m, "down", AlertSeverity::Critical),
        Err(Error::NotFound)
    ));
    let inc = open_incident(&conn, &a.org, &m, "http_500", AlertSeverity::Critical)?;
    assert_eq!(list_incidents(&conn, &a.org, Some(true), 10)?.len(), 1);
    assert!(list_incidents(&conn, &b.org, None, 10)?.is_empty());

    assert!(matches!(
        acknowledge_incident(&conn, &b.org, &inc, &b.user),
        Err(Error::NotFound)
    ));
    acknowledge_incident(&conn, &a.org, &inc, &a.user)?;
    let resolved = resolve_open_incidents(&conn, &a.org, &m)?;
    assert_eq!(resolved, 1);
    assert!(list_incidents(&conn, &a.org, Some(true), 10)?.is_empty());
    assert_eq!(list_incidents(&conn, &a.org, Some(false), 10)?.len(), 1);

    assert!(matches!(
        delete_monitor(&conn, &b.org, &m),
        Err(Error::NotFound)
    ));
    delete_monitor(&conn, &a.org, &m)?;
    assert!(list_incidents(&conn, &a.org, None, 10)?.is_empty());
    Ok(())
}

#[test]
fn channel_secret_sealing_rules_pending_alerts_and_telegram_links() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;

    let sec = put_secret(
        &conn,
        &a.org,
        "telegram_bot_token",
        &SealedSecret {
            ciphertext: vec![9; 32],
            key_id: "k1".to_owned(),
        },
    )?;
    // Cross-org secret FK is refused.
    assert!(
        create_channel(
            &conn,
            &b.org,
            &NewChannel {
                kind: ChannelKind::Telegram,
                name: "bad",
                config_json: "{}",
                secret: Some(&sec),
            },
        )
        .is_err()
    );

    let ch = create_channel(
        &conn,
        &a.org,
        &NewChannel {
            kind: ChannelKind::Telegram,
            name: "ops-tg",
            config_json: r#"{"chat_id":"12345"}"#,
            secret: Some(&sec),
        },
    )?;
    let _rule = create_alert_rule(
        &conn,
        &a.org,
        &NewAlertRule {
            name: "night-quiet",
            channel: &ch,
            min_severity: AlertSeverity::Warning,
            quiet_start: Some("22:00"),
            quiet_end: Some("07:00"),
        },
    )?;
    assert_eq!(list_alert_rules(&conn, &a.org)?.len(), 1);
    assert!(list_alert_rules(&conn, &b.org)?.is_empty());

    let pa = enqueue_pending_alert(
        &conn,
        &a.org,
        &ch,
        "Monitor down",
        "api-health failed",
        AlertSeverity::Critical,
    )?;
    assert_eq!(list_unflushed_alerts(&conn, &a.org)?.len(), 1);
    assert!(matches!(
        mark_alert_flushed(&conn, &b.org, &pa),
        Err(Error::NotFound)
    ));
    mark_alert_flushed(&conn, &a.org, &pa)?;
    assert!(list_unflushed_alerts(&conn, &a.org)?.is_empty());

    // Telegram link lifecycle.
    let hash = sha256_hex(b"CODE-1234");
    create_telegram_link(&conn, &a.org, &a.user, &hash, "en")?;
    assert!(matches!(
        verify_telegram_link(&conn, &b.org, &hash, "999"),
        Err(Error::NotFound)
    ));
    let linked = verify_telegram_link(&conn, &a.org, &hash, "999")?;
    assert_eq!(linked.status, TelegramLinkStatus::Linked);
    assert_eq!(linked.chat_id.as_deref(), Some("999"));
    set_telegram_link_language(&conn, &a.org, "999", "id")?;
    assert_eq!(
        get_telegram_link_by_chat(&conn, &a.org, "999")?.language,
        "id"
    );

    let removed_sec = delete_channel(&conn, &a.org, &ch)?;
    assert_eq!(removed_sec, Some(sec));
    Ok(())
}
