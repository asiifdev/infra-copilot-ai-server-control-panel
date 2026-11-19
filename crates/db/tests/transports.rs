//! Transports: tenant isolation and integrity rules.

mod common;
use common::*;
use db::*;

fn sealed(byte: u8) -> SealedSecret {
    SealedSecret {
        ciphertext: vec![byte; 40],
        key_id: "k1".to_owned(),
    }
}

fn new<'a>(name: &'a str, kind: &'a str, secret: Option<&'a SecretId>) -> NewTransport<'a> {
    NewTransport {
        name,
        kind,
        config_json: "{\"remote\":\"r\"}",
        secret,
    }
}

#[test]
fn stores_lists_records_a_check_and_deletes() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let secret = put_secret(&conn, &t.org, "transport_credentials", &sealed(1))?;
    let id = create_transport(&conn, &t.org, &new("drive", "rclone", Some(&secret)))?;
    let row = get_transport(&conn, &t.org, &id)?;
    assert_eq!(row.secret_id.as_ref(), Some(&secret));
    assert_eq!(row.verified_at, None);
    record_transport_check(&conn, &t.org, &id, "{\"ok\":true}", true)?;
    let row = get_transport(&conn, &t.org, &id)?;
    assert!(row.verified_at.is_some());
    // A failed check keeps the earlier verification time.
    let before = row.verified_at.clone();
    record_transport_check(&conn, &t.org, &id, "{\"ok\":false}", false)?;
    let row = get_transport(&conn, &t.org, &id)?;
    assert_eq!(row.verified_at, before);
    assert_eq!(row.last_check_json.as_deref(), Some("{\"ok\":false}"));
    assert_eq!(list_transports(&conn, &t.org)?.len(), 1);
    // The secret cannot be deleted while a transport uses it.
    assert!(delete_secret(&conn, &t.org, &secret).is_err());
    delete_transport(&conn, &t.org, &id)?;
    delete_secret(&conn, &t.org, &secret)?;
    Ok(())
}

#[test]
fn another_org_cannot_read_list_change_delete_or_reference() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "bravo")?;
    let secret_a = put_secret(&conn, &a.org, "transport_credentials", &sealed(1))?;
    let id_a = create_transport(&conn, &a.org, &new("drive", "rclone", Some(&secret_a)))?;

    assert!(matches!(
        get_transport(&conn, &b.org, &id_a),
        Err(Error::NotFound)
    ));
    assert!(list_transports(&conn, &b.org)?.is_empty());
    assert!(matches!(
        record_transport_check(&conn, &b.org, &id_a, "{}", true),
        Err(Error::NotFound)
    ));
    assert!(matches!(
        delete_transport(&conn, &b.org, &id_a),
        Err(Error::NotFound)
    ));
    // The row is untouched by the failed attempts.
    assert_eq!(get_transport(&conn, &a.org, &id_a)?.verified_at, None);
    // Org B cannot reference org A's secret: the composite foreign key refuses it.
    assert!(create_transport(&conn, &b.org, &new("steal", "rclone", Some(&secret_a))).is_err());
    // The same name is fine in another org.
    create_transport(&conn, &b.org, &new("drive", "rclone", None))?;
    Ok(())
}

#[test]
fn the_schema_refuses_bad_rows() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    create_transport(&conn, &t.org, &new("one", "rclone", None))?;
    // Duplicate name inside the org.
    assert!(create_transport(&conn, &t.org, &new("one", "rsync", None)).is_err());
    // Unknown kind.
    assert!(create_transport(&conn, &t.org, &new("two", "ftp", None)).is_err());
    // Config must be a JSON object.
    for bad in ["not json", "[]", "\"x\"", "null"] {
        let n = NewTransport {
            name: "three",
            kind: "scp",
            config_json: bad,
            secret: None,
        };
        assert!(create_transport(&conn, &t.org, &n).is_err(), "{bad}");
    }
    // A secret id that does not exist.
    let ghost = SecretId::parse("00000000-0000-4000-8000-000000000001")?;
    assert!(create_transport(&conn, &t.org, &new("four", "rclone", Some(&ghost))).is_err());
    Ok(())
}
