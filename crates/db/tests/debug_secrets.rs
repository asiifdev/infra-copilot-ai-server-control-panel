mod common;
use common::*;
use db::*;

#[test]
fn debug_output_never_contains_secret_values() -> TestResult {
    let conn = open_in_memory()?;
    let pw = SecretText::new("correct-horse-battery-staple");
    let dbg = format!("{pw:?}");
    assert!(!dbg.contains("correct-horse"));
    assert!(dbg.contains("REDACTED"));

    let sealed = SealedSecret {
        ciphertext: b"TOPSECRETBYTES".to_vec(),
        key_id: "k1".into(),
    };
    let dbg = format!("{sealed:?}");
    assert!(!dbg.contains("TOPSECRET") && !dbg.contains("84, 79, 80"));
    assert!(dbg.contains("14 bytes") && dbg.contains("k1"));

    // Input structs that carry hashes.
    let user = create_user(&conn, "dbg@example.test", "Dbg", Some(&pw))?;
    let t = populate(&conn, "alpha")?;
    let token_hash = SecretText::new("tokenhash-secret-value");
    let new_token = NewApiToken {
        user_id: &t.user,
        name: "n",
        token_hash: &token_hash,
        scopes_json: "[]",
        expires_at: None,
    };
    let new_agent = NewAgent {
        node_id: &t.node,
        token_hash: &token_hash,
        version: "1",
        os: "linux",
        arch: "arm64",
    };
    for text in [
        format!("{new_token:?}"),
        format!("{new_agent:?}"),
        format!("{user:?}"),
        format!("{:?}", list_api_tokens(&conn, &t.org)?),
        format!("{:?}", list_agents(&conn, &t.org, &t.node)?),
        format!("{:?}", list_node_secrets(&conn, &t.org, &t.node)?),
        format!("{:?}", get_node_secret(&conn, &t.org, &t.secret)?),
    ] {
        assert!(!text.contains("tokenhash-secret-value"), "{text}");
        assert!(!text.contains("correct-horse"), "{text}");
        assert!(!text.contains("hash-alpha"), "{text}");
    }
    Ok(())
}
