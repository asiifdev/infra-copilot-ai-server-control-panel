mod common;
use common::*;
use db::*;
use rusqlite::{Connection, params};

fn append(conn: &Connection, org: &OrgId, action: &str, subject: &str) -> Result<AuditEntry> {
    append_audit(
        conn,
        org,
        &NewAuditEntry {
            actor_kind: ActorKind::System,
            actor_id: None,
            action,
            node_id: None,
            subject_json: subject,
            result: AuditOutcome::Success,
        },
    )
}

fn chain_of_three() -> std::result::Result<(Connection, OrgId), Box<dyn std::error::Error>> {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    append(&conn, &t.org, "a", "{}")?;
    append(&conn, &t.org, "b", "{}")?;
    assert_eq!(
        verify_audit_chain(&conn, &t.org)?,
        ChainStatus::Intact { entries: 3 }
    );
    Ok((conn, t.org))
}

#[test]
fn entries_are_chained_from_genesis() -> TestResult {
    let (conn, org) = chain_of_three()?;
    let entries = list_audit(&conn, &org, 10)?;
    assert_eq!(
        entries.iter().map(|e| e.seq).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(entries[0].prev_hash, entries[1].hash);
    assert_eq!(entries[1].prev_hash, entries[2].hash);
    assert_eq!(entries[2].prev_hash, "0".repeat(64));
    Ok(())
}

#[test]
fn update_and_delete_are_rejected() -> TestResult {
    let (conn, org) = chain_of_three()?;
    let upd = conn.execute(
        "UPDATE audit_log SET action = 'x' WHERE org_id = ?1",
        params![org.as_str()],
    );
    let del = conn.execute(
        "DELETE FROM audit_log WHERE org_id = ?1",
        params![org.as_str()],
    );
    for r in [upd, del] {
        let err = r.err().map(Error::from);
        assert!(
            matches!(&err, Some(e) if e.is_constraint_violation()),
            "got {err:?}"
        );
    }
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Intact { entries: 3 }
    );
    Ok(())
}

#[test]
fn verification_detects_a_modified_entry() -> TestResult {
    let (conn, org) = chain_of_three()?;
    // Simulate someone who bypasses the trigger (e.g. editing the file with another tool).
    conn.execute_batch("DROP TRIGGER audit_log_no_update;")?;
    conn.execute(
        "UPDATE audit_log SET action = 'forged' WHERE org_id = ?1 AND seq = 2",
        params![org.as_str()],
    )?;
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Broken {
            seq: 2,
            kind: ChainBreak::HashMismatch
        }
    );
    Ok(())
}

#[test]
fn verification_detects_a_removed_middle_entry() -> TestResult {
    let (conn, org) = chain_of_three()?;
    conn.execute_batch("DROP TRIGGER audit_log_no_delete;")?;
    conn.execute(
        "DELETE FROM audit_log WHERE org_id = ?1 AND seq = 2",
        params![org.as_str()],
    )?;
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Broken {
            seq: 3,
            kind: ChainBreak::SequenceGap { expected: 2 }
        }
    );
    Ok(())
}

#[test]
fn verification_detects_a_removed_first_entry() -> TestResult {
    let (conn, org) = chain_of_three()?;
    conn.execute_batch("DROP TRIGGER audit_log_no_delete;")?;
    conn.execute(
        "DELETE FROM audit_log WHERE org_id = ?1 AND seq = 1",
        params![org.as_str()],
    )?;
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Broken {
            seq: 2,
            kind: ChainBreak::SequenceGap { expected: 1 }
        }
    );
    Ok(())
}

#[test]
fn verification_detects_a_relinked_entry() -> TestResult {
    let (conn, org) = chain_of_three()?;
    conn.execute_batch("DROP TRIGGER audit_log_no_update;")?;
    conn.execute(
        "UPDATE audit_log SET prev_hash = ?2 WHERE org_id = ?1 AND seq = 3",
        params![org.as_str(), "f".repeat(64)],
    )?;
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Broken {
            seq: 3,
            kind: ChainBreak::PrevHashMismatch
        }
    );
    Ok(())
}

#[test]
fn secrets_in_the_subject_are_redacted_before_storage() -> TestResult {
    let (conn, org) = chain_of_three()?;
    let e = append(
        &conn,
        &org,
        "login",
        "{\"password\": \"hunter2-very-secret\", \"user\": \"bob\"}",
    )?;
    assert!(!e.subject_json.contains("hunter2"));
    assert!(e.subject_json.contains("bob"));
    let stored: String = conn.query_row(
        "SELECT subject FROM audit_log WHERE id = ?1",
        params![e.id.as_str()],
        |r| r.get(0),
    )?;
    assert!(!stored.contains("hunter2"));
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Intact { entries: 4 }
    );
    Ok(())
}

#[test]
fn invalid_subject_json_is_rejected() -> TestResult {
    let (conn, org) = chain_of_three()?;
    assert!(is_constraint(&append(&conn, &org, "x", "{not json")));
    // A failed append leaves the chain intact and appendable.
    append(&conn, &org, "y", "{}")?;
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Intact { entries: 4 }
    );
    Ok(())
}

#[test]
fn sensitive_looking_keys_do_not_break_the_append() -> TestResult {
    let (conn, org) = chain_of_three()?;
    for subject in [
        "{\"token_count\":5,\"action\":\"x\"}",
        "{\"key\":1}",
        "{\"secret\":null,\"a\":1}",
    ] {
        let entry = append(&conn, &org, "z", subject)?;
        // Whatever the redactor did, the stored subject is valid JSON and the chain stays intact.
        let valid: bool = conn.query_row(
            "SELECT json_valid(subject) FROM audit_log WHERE id = ?1",
            params![entry.id.as_str()],
            |r| r.get(0),
        )?;
        assert!(valid);
    }
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Intact { entries: 6 }
    );
    Ok(())
}

#[test]
fn insert_or_replace_cannot_overwrite_an_entry() -> TestResult {
    let (conn, org) = chain_of_three()?;
    let replaced = conn.execute(
        "INSERT OR REPLACE INTO audit_log (id, org_id, seq, actor_kind, action, subject, result, \
         created_at, prev_hash, hash) VALUES (?1, ?2, 2, 'system', 'evil', '{}', 'success', \
         'now', ?3, ?3)",
        params![
            "00000000-0000-4000-8000-000000000000",
            org.as_str(),
            "1".repeat(64)
        ],
    );
    assert!(replaced.is_err());
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Intact { entries: 3 }
    );
    Ok(())
}
