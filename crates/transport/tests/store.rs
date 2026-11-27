//! Persistence: sealed credentials, org isolation, and no plaintext in the database file.

use db::{OrgId, SealedSecret, create_org, open, open_in_memory};
use std::collections::BTreeMap;
use transport::store::{self, CREDENTIALS_PURPOSE, StoreError};
use transport::{
    Credentials, Progress, RcloneSpec, RemoteKind, Report, Secret, SshTarget, TransportSpec,
    testing::SENTINEL,
};
use vault::Vault;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const KEY: &str = "k1:000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn org(conn: &db::Connection, slug: &str) -> Result<OrgId, Box<dyn std::error::Error>> {
    Ok(create_org(conn, &format!("Org {slug}"), slug, "free")?.id)
}

fn spec() -> Result<TransportSpec, Box<dyn std::error::Error>> {
    let mut options = BTreeMap::new();
    options.insert("region".to_owned(), "eu-west-1".to_owned());
    Ok(TransportSpec::Rclone(RcloneSpec::new(
        RemoteKind::S3,
        "offsite",
        "bk",
        options,
    )?))
}

fn creds() -> Result<Credentials, Box<dyn std::error::Error>> {
    Ok(Credentials::new(vec![
        ("access_key_id".to_owned(), Secret::new("AKIDEXAMPLE0123")),
        ("secret_access_key".to_owned(), Secret::new(SENTINEL)),
    ])?)
}

#[test]
fn saves_and_loads_a_transport_with_sealed_credentials() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    let report = Report {
        listing_ok: true,
        speed_bytes_per_sec: Some(1000),
        ..Report::default()
    };
    let id = store::save(
        &conn,
        &vault,
        &a,
        "offsite",
        &spec()?,
        Some(&creds()?),
        Some(&report),
    )?;
    let stored = store::load(&conn, &a, &id)?;
    assert_eq!(stored.spec, spec()?);
    assert!(stored.verified_at.is_some());
    assert!(
        stored
            .last_check_json
            .as_deref()
            .is_some_and(|j| j.contains("1000"))
    );
    let back = store::load_credentials(&conn, &vault, &a, &stored)?.ok_or("no credentials")?;
    assert_eq!(
        back.names().collect::<Vec<_>>(),
        ["access_key_id", "secret_access_key"]
    );
    assert!(
        back.pairs()
            .iter()
            .any(|(_, v)| v.expose_secret() == SENTINEL)
    );
    assert_eq!(store::list(&conn, &a)?.len(), 1);
    // The secret is sealed under the transport purpose.
    let secret_id = stored.secret_id.ok_or("no secret id")?;
    let (purpose, sealed) = db::get_secret(&conn, &a, &secret_id)?;
    assert_eq!(purpose, CREDENTIALS_PURPOSE);
    assert!(
        !sealed
            .ciphertext
            .windows(SENTINEL.len())
            .any(|w| w == SENTINEL.as_bytes())
    );
    store::delete(&conn, &a, &id)?;
    assert!(matches!(
        db::get_secret(&conn, &a, &secret_id),
        Err(db::Error::NotFound)
    ));
    Ok(())
}

#[test]
fn ssh_transports_need_no_secret_and_refuse_credentials() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    let t = TransportSpec::Rsync(SshTarget::new("h.example.test", "deploy", None, "/srv/in")?);
    let id = store::save(&conn, &vault, &a, "rsync-1", &t, None, None)?;
    let stored = store::load(&conn, &a, &id)?;
    assert_eq!(stored.secret_id, None);
    assert!(store::load_credentials(&conn, &vault, &a, &stored)?.is_none());
    assert_eq!(
        store::save(&conn, &vault, &a, "rsync-2", &t, Some(&creds()?), None).err(),
        Some(StoreError::CredentialsNotAccepted)
    );
    assert_eq!(store::list(&conn, &a)?.len(), 1);
    Ok(())
}

#[test]
fn managed_and_bad_names_cannot_be_saved() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    assert_eq!(
        store::save(&conn, &vault, &a, "m", &TransportSpec::Managed, None, None).err(),
        Some(StoreError::UnsupportedManaged)
    );
    for bad in ["", "a\nb", "a\0b", &"n".repeat(101)] {
        assert!(matches!(
            store::save(&conn, &vault, &a, bad, &spec()?, None, None),
            Err(StoreError::Invalid(_))
        ));
    }
    Ok(())
}

#[test]
fn a_duplicate_name_leaves_no_orphan_secret() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    store::save(
        &conn,
        &vault,
        &a,
        "offsite",
        &spec()?,
        Some(&creds()?),
        None,
    )?;
    let again = store::save(
        &conn,
        &vault,
        &a,
        "offsite",
        &spec()?,
        Some(&creds()?),
        None,
    );
    assert_eq!(again.err(), Some(StoreError::Database));
    let secrets: i64 = conn.query_row("SELECT count(*) FROM secrets", [], |r| r.get(0))?;
    assert_eq!(secrets, 1);
    Ok(())
}

#[test]
fn another_org_cannot_read_list_open_or_delete() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    let b = org(&conn, "bravo")?;
    let id = store::save(
        &conn,
        &vault,
        &a,
        "offsite",
        &spec()?,
        Some(&creds()?),
        None,
    )?;
    let stored = store::load(&conn, &a, &id)?;

    assert_eq!(
        store::load(&conn, &b, &id).err(),
        Some(StoreError::NotFound)
    );
    assert!(store::list(&conn, &b)?.is_empty());
    assert_eq!(
        store::delete(&conn, &b, &id).err(),
        Some(StoreError::NotFound)
    );
    // Even holding org A's stored value, org B cannot open its credentials.
    assert!(store::load_credentials(&conn, &vault, &b, &stored).is_err());
    // Nor can org B point its own transport at org A's secret.
    let secret_a = stored.secret_id.clone().ok_or("no secret")?;
    let sneaky = db::create_transport(
        &conn,
        &b,
        &db::NewTransport {
            name: "sneaky",
            kind: "rclone",
            config_json: &stored.spec.to_config_json(),
            secret: Some(&secret_a),
        },
    );
    assert!(sneaky.is_err());
    // Org A still has everything.
    assert!(store::load_credentials(&conn, &vault, &a, &stored)?.is_some());
    Ok(())
}

#[test]
fn a_transport_cannot_be_pointed_at_a_secret_of_another_purpose() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    // A sealed AI provider key in the same org.
    let ai_key = vault.store(&conn, &a, "ai_provider_key", b"sk-not-for-transports")?;
    let id = db::create_transport(
        &conn,
        &a,
        &db::NewTransport {
            name: "confused",
            kind: "rclone",
            config_json: &spec()?.to_config_json(),
            secret: Some(&ai_key),
        },
    )?;
    let stored = store::load(&conn, &a, &id)?;
    assert_eq!(
        store::load_credentials(&conn, &vault, &a, &stored).err(),
        Some(StoreError::CorruptCredentials)
    );
    Ok(())
}

#[test]
fn tampered_or_foreign_sealed_credentials_do_not_open() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    let id = store::save(
        &conn,
        &vault,
        &a,
        "offsite",
        &spec()?,
        Some(&creds()?),
        None,
    )?;
    let stored = store::load(&conn, &a, &id)?;
    let secret_id = stored.secret_id.clone().ok_or("no secret")?;
    let (_, sealed) = db::get_secret(&conn, &a, &secret_id)?;
    let mut bad = sealed.ciphertext.clone();
    let last = bad.len() - 1;
    bad[last] ^= 1;
    db::replace_secret(
        &conn,
        &a,
        &secret_id,
        &SealedSecret {
            ciphertext: bad,
            key_id: sealed.key_id,
        },
    )?;
    assert!(store::load_credentials(&conn, &vault, &a, &stored).is_err());
    // A different master key cannot open it either.
    let other = Vault::new(
        "k1:ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100",
        &[],
    )?;
    assert!(store::load_credentials(&conn, &other, &a, &stored).is_err());
    Ok(())
}

#[test]
fn a_corrupt_row_is_refused_not_trusted() -> TestResult {
    let conn = open_in_memory()?;
    let a = org(&conn, "alpha")?;
    // Written around the crate, as a hostile or buggy writer could: the stored host is an option.
    let id = db::create_transport(
        &conn,
        &a,
        &db::NewTransport {
            name: "bad",
            kind: "rsync",
            config_json: r#"{"host":"-oProxyCommand=evil","user":"u","port":22,"path":"/p"}"#,
            secret: None,
        },
    )?;
    assert!(matches!(
        store::load(&conn, &a, &id),
        Err(StoreError::CorruptRow(_))
    ));
    assert!(store::list(&conn, &a).is_err());
    Ok(())
}

#[test]
fn the_database_file_never_contains_plaintext_credentials() -> TestResult {
    let mut dir = std::env::temp_dir();
    dir.push(format!("nodivara-transport-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("db.sqlite");
    let result = (|| -> TestResult {
        let conn = open(&path)?;
        let vault = Vault::new(KEY, &[])?;
        let a = org(&conn, "alpha")?;
        let report = Report {
            listing_ok: true,
            ..Report::default()
        };
        store::save(
            &conn,
            &vault,
            &a,
            "offsite",
            &spec()?,
            Some(&creds()?),
            Some(&report),
        )?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        let mut scanned = 0;
        for suffix in ["", "-wal"] {
            let mut name = path.clone().into_os_string();
            name.push(suffix);
            let Ok(bytes) = std::fs::read(&name) else {
                continue;
            };
            scanned += bytes.len();
            for needle in [SENTINEL, "AKIDEXAMPLE0123"] {
                assert!(
                    !bytes.windows(needle.len()).any(|w| w == needle.as_bytes()),
                    "plaintext credential found in the database file"
                );
            }
        }
        assert!(scanned > 0, "the scan must have read something");
        // Sanity: the scan would notice plaintext, so the absence above means something.
        conn.execute(
            "UPDATE transports SET last_check = ?1",
            [format!("{{\"leak\":\"{SENTINEL}\"}}")],
        )?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;
        let bytes = std::fs::read(&path)?;
        assert!(
            bytes
                .windows(SENTINEL.len())
                .any(|w| w == SENTINEL.as_bytes())
        );
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}

#[test]
fn the_wizard_result_feeds_straight_into_save() -> TestResult {
    use transport::{FakeRunner, RunOutput, Wizard, WizardParams};
    let runner = FakeRunner::new();
    runner.push(Ok(RunOutput::ok("rclone v1.76.0")));
    runner.push(Ok(RunOutput::ok("")));
    runner.push(Ok(RunOutput::ok(r#"{"free":9000000000}"#)));
    runner.push(Ok(RunOutput::ok("")));
    let mut w = Wizard::new(
        match spec()? {
            TransportSpec::Rclone(s) => s,
            _ => return Err("not rclone".into()),
        },
        creds()?,
        WizardParams::default(),
    );
    while !matches!(w.advance(&runner), Progress::ReadyToSave) {
        if w.step() == transport::Step::Save {
            break;
        }
    }
    let done = w.finish()?;
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    let id = store::save(
        &conn,
        &vault,
        &a,
        "from-wizard",
        &done.spec,
        Some(&done.credentials),
        Some(&done.report),
    )?;
    assert!(store::load(&conn, &a, &id)?.verified_at.is_some());
    Ok(())
}

#[test]
fn update_credentials_rotates_secret_resets_verification_and_is_tenant_isolated() -> TestResult {
    let conn = open_in_memory()?;
    let vault = Vault::new(KEY, &[])?;
    let a = org(&conn, "alpha")?;
    let b = org(&conn, "beta")?;
    let id = store::save(
        &conn,
        &vault,
        &a,
        "s3-prod",
        &spec()?,
        Some(&creds()?),
        None,
    )?;
    db::record_transport_check(&conn, &a, &id, r#"{"listing_ok":true}"#, true)?;
    let before = store::load(&conn, &a, &id)?;
    assert!(before.verified_at.is_some());
    let old_secret = before.secret_id.clone().ok_or("missing old secret")?;

    let new_creds = Credentials::new(vec![
        (
            "access_key_id".to_owned(),
            Secret::new("NEW-KEY-123".to_owned()),
        ),
        (
            "secret_access_key".to_owned(),
            Secret::new("NEW-SECRET-456".to_owned()),
        ),
    ])?;

    // Rival org cannot update.
    assert!(matches!(
        store::update_credentials(&conn, &vault, &b, &id, &new_creds),
        Err(StoreError::NotFound)
    ));

    store::update_credentials(&conn, &vault, &a, &id, &new_creds)?;
    let after = store::load(&conn, &a, &id)?;
    assert!(after.verified_at.is_none());
    assert!(after.last_check_json.is_none());
    assert_ne!(after.secret_id.as_ref(), Some(&old_secret));
    // Old secret was deleted.
    assert!(db::get_secret(&conn, &a, &old_secret).is_err());

    let loaded = store::load_credentials(&conn, &vault, &a, &after)?.ok_or("missing creds")?;
    assert!(
        loaded
            .to_vault_text()
            .expose_secret()
            .contains("NEW-KEY-123")
    );
    Ok(())
}
