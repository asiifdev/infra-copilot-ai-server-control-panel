//! Sealed secrets and AI providers: tenant isolation and integrity rules.

mod common;
use common::*;
use db::*;

fn sealed(byte: u8) -> SealedSecret {
    SealedSecret {
        ciphertext: vec![byte; 40],
        key_id: "k1".to_owned(),
    }
}

fn provider<'a>(
    name: &'a str,
    secret: Option<&'a SecretId>,
    is_default: bool,
) -> NewAiProvider<'a> {
    NewAiProvider {
        name,
        base_url: "https://api.example.test/v1",
        default_model: "model-1",
        secret,
        params_json: "{}",
        is_default,
    }
}

#[test]
fn stores_and_reads_back_a_sealed_secret_and_a_provider() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let secret = put_secret(&conn, &t.org, "ai_provider_key", &sealed(7))?;
    let (purpose, back) = get_secret(&conn, &t.org, &secret)?;
    assert_eq!((purpose.as_str(), back), ("ai_provider_key", sealed(7)));
    let id = create_ai_provider(&conn, &t.org, &provider("main", Some(&secret), true))?;
    let p = get_ai_provider(&conn, &t.org, &id)?;
    assert_eq!(p.secret_id.as_ref(), Some(&secret));
    assert!(p.is_default);
    assert_eq!(default_ai_provider(&conn, &t.org)?.id, id);
    assert_eq!(list_ai_providers(&conn, &t.org)?.len(), 1);
    // A keyless local provider is allowed.
    create_ai_provider(&conn, &t.org, &provider("local", None, false))?;
    Ok(())
}

#[test]
fn org_a_cannot_read_use_or_delete_org_b_secrets_and_providers() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let b_secret = put_secret(&conn, &b.org, "ai_provider_key", &sealed(9))?;
    let b_provider = create_ai_provider(&conn, &b.org, &provider("main", Some(&b_secret), true))?;

    assert!(is_not_found(&get_secret(&conn, &a.org, &b_secret)));
    assert!(is_not_found(&delete_secret(&conn, &a.org, &b_secret)));
    assert!(is_not_found(&get_ai_provider(&conn, &a.org, &b_provider)));
    assert!(is_not_found(&delete_ai_provider(
        &conn,
        &a.org,
        &b_provider
    )));
    assert!(is_not_found(&default_ai_provider(&conn, &a.org)));
    assert!(list_ai_providers(&conn, &a.org)?.is_empty());
    // A provider of A may not point at B's secret: the composite foreign key refuses it.
    assert!(is_constraint(&create_ai_provider(
        &conn,
        &a.org,
        &provider("steal", Some(&b_secret), false)
    )));
    // B is untouched.
    assert_eq!(get_secret(&conn, &b.org, &b_secret)?.1, sealed(9));
    Ok(())
}

#[test]
fn only_one_default_and_unique_names_per_org() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    create_ai_provider(&conn, &t.org, &provider("one", None, true))?;
    assert!(is_constraint(&create_ai_provider(
        &conn,
        &t.org,
        &provider("two", None, true)
    )));
    assert!(is_constraint(&create_ai_provider(
        &conn,
        &t.org,
        &provider("one", None, false)
    )));
    create_ai_provider(&conn, &t.org, &provider("two", None, false))?;
    // Another org may reuse names and have its own default.
    let b = populate(&conn, "beta")?;
    create_ai_provider(&conn, &b.org, &provider("one", None, true))?;
    Ok(())
}

#[test]
fn a_secret_in_use_cannot_be_deleted_and_bad_input_is_refused() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let secret = put_secret(&conn, &t.org, "ai_provider_key", &sealed(1))?;
    let provider_id = create_ai_provider(&conn, &t.org, &provider("main", Some(&secret), false))?;
    assert!(is_constraint(&delete_secret(&conn, &t.org, &secret)));
    delete_ai_provider(&conn, &t.org, &provider_id)?;
    delete_secret(&conn, &t.org, &secret)?;
    let empty = SealedSecret {
        ciphertext: vec![],
        key_id: "k1".into(),
    };
    assert!(is_constraint(&put_secret(&conn, &t.org, "x", &empty)));
    let bad = NewAiProvider {
        params_json: "not json",
        ..provider("bad", None, false)
    };
    assert!(is_constraint(&create_ai_provider(&conn, &t.org, &bad)));
    Ok(())
}
