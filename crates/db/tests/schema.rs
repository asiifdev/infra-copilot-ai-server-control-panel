mod common;
use common::*;
use db::*;
use rusqlite::Connection;
use std::path::{Path, PathBuf};

fn temp_db_path(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "infra-copilot-db-test-{}-{tag}.sqlite",
        std::process::id()
    ));
    p
}

fn remove_db(path: &Path) {
    for suffix in ["", "-wal", "-shm"] {
        let mut s = path.to_path_buf().into_os_string();
        s.push(suffix);
        let _ = std::fs::remove_file(s);
    }
}

fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

#[test]
fn migrations_are_valid_and_reach_the_latest_version_from_empty() -> TestResult {
    let conn = open_in_memory()?;
    assert_eq!(user_version(&conn)?, 8);
    assert_eq!(schema_version(&conn)?, LATEST_VERSION);
    let tables: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN \
         ('organizations','users','memberships','api_tokens','nodes','agents','node_secrets','audit_log','inventories','inventory_items','comparisons','findings','secrets','ai_providers','conversations','messages','tool_calls','proposals','transports','migration_plans','migration_steps')",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(tables, 21);
    Ok(())
}

#[test]
fn migrations_are_idempotent_and_keep_data() -> TestResult {
    let path = temp_db_path("idem");
    remove_db(&path);
    let org = {
        let conn = open(&path)?;
        populate(&conn, "alpha")?.org
    };
    let mut conn = open(&path)?; // second open migrates again: must be a no-op
    migrate(&mut conn)?;
    assert_eq!(user_version(&conn)?, 8);
    assert_eq!(list_nodes(&conn, &org)?.len(), 1);
    assert_eq!(
        verify_audit_chain(&conn, &org)?,
        ChainStatus::Intact { entries: 1 }
    );
    drop(conn);
    remove_db(&path);
    Ok(())
}

#[test]
fn pragmas_are_applied() -> TestResult {
    let mem = open_in_memory()?;
    let fk: i64 = mem.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    let busy: i64 = mem.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
    assert_eq!(fk, 1);
    assert_eq!(busy, 5000);

    let path = temp_db_path("pragma");
    remove_db(&path);
    let file = open(&path)?;
    let mode: String = file.query_row("PRAGMA journal_mode", [], |r| r.get(0))?;
    let fk: i64 = file.query_row("PRAGMA foreign_keys", [], |r| r.get(0))?;
    let busy: i64 = file.query_row("PRAGMA busy_timeout", [], |r| r.get(0))?;
    assert_eq!(mode.to_ascii_lowercase(), "wal");
    assert_eq!(fk, 1);
    assert_eq!(busy, 5000);
    drop(file);
    remove_db(&path);
    Ok(())
}

fn bad_sql(conn: &Connection, sql: &str) -> bool {
    let err = conn.execute_batch(sql).err().map(Error::from);
    matches!(&err, Some(e) if e.is_constraint_violation())
}

#[test]
fn foreign_keys_and_checks_reject_bad_data() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let org = t.org.as_str();
    let user = t.user.as_str();
    let uuid = "123e4567-e89b-42d3-a456-426614174000";

    // Foreign keys.
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO memberships (id, org_id, user_id, role) VALUES ('{uuid}', '{uuid}', '{user}', 'viewer')"
        )
    ));
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO nodes (id, org_id, name, kind, access) VALUES ('{uuid}', '{uuid}', 'n', 'vps', 'ssh')"
        )
    ));
    // Enum CHECKs.
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO memberships (id, org_id, user_id, role) VALUES ('{uuid}', '{org}', '{user}', 'root')"
        )
    ));
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO nodes (id, org_id, name, kind, access) VALUES ('{uuid}', '{org}', 'n2', 'mainframe', 'ssh')"
        )
    ));
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO nodes (id, org_id, name, kind, access) VALUES ('{uuid}', '{org}', 'n3', 'vps', 'telepathy')"
        )
    ));
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO nodes (id, org_id, name, kind, access, status) VALUES ('{uuid}', '{org}', 'n4', 'vps', 'ssh', 'zombie')"
        )
    ));
    // json_valid CHECKs.
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO nodes (id, org_id, name, kind, access, labels) VALUES ('{uuid}', '{org}', 'n5', 'vps', 'ssh', '{{oops')"
        )
    ));
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, scopes) VALUES ('{uuid}', '{org}', '{user}', 't', 'h', 'nope')"
        )
    ));
    // Shape CHECKs.
    assert!(bad_sql(
        &conn,
        "INSERT INTO organizations (id, name, slug) VALUES ('short', 'x', 'x')"
    ));
    assert!(bad_sql(
        &conn,
        &format!("INSERT INTO organizations (id, name, slug) VALUES ('{uuid}', 'x', 'Bad Slug!')")
    ));
    assert!(bad_sql(
        &conn,
        &format!("INSERT INTO users (id, email, name) VALUES ('{uuid}', 'UPPER@X.TEST', 'n')")
    ));
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO api_tokens (id, org_id, user_id, name, token_hash, expires_at) VALUES ('{uuid}', '{org}', '{user}', 't', 'h2', 'tomorrow-ish')"
        )
    ));
    // Empty ciphertext.
    let node = t.node.as_str();
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO node_secrets (id, org_id, node_id, purpose, ciphertext, key_id) VALUES ('{uuid}', '{org}', '{node}', 'p', x'', 'k')"
        )
    ));
    // Audit enums.
    assert!(bad_sql(
        &conn,
        &format!(
            "INSERT INTO audit_log (id, org_id, seq, actor_kind, action, subject, result, created_at, prev_hash, hash) \
         VALUES ('{uuid}', '{org}', 9, 'ghost', 'a', '{{}}', 'success', 'now', '{h}', '{h}')",
            h = "0".repeat(64)
        )
    ));

    // Duplicates.
    let dup_user = create_user(&conn, "alpha@example.test", "Again", None);
    assert!(is_constraint(&dup_user));
    let dup_org = create_org(&conn, "Other", "alpha", "free");
    assert!(is_constraint(&dup_org));
    // Membership uniqueness.
    assert!(is_constraint(&add_member(
        &conn,
        &t.org,
        &t.user,
        Role::Viewer
    )));
    // Foreign key through the typed API: unknown org.
    let ghost = OrgId::parse(uuid)?;
    assert!(is_constraint(&add_member(
        &conn,
        &ghost,
        &t.user,
        Role::Viewer
    )));
    Ok(())
}

#[test]
fn unknown_stored_ids_are_rejected_at_parse() {
    assert!(OrgId::parse("DROP TABLE users").is_err());
    assert!(Role::parse("root").is_err());
    assert!(NodeStatus::parse("zombie").is_err());
}

#[test]
fn a_newer_database_is_refused_and_a_failed_migration_rolls_back() -> TestResult {
    let mut conn = rusqlite::Connection::open_in_memory()?;
    conn.pragma_update(None, "user_version", 99)?;
    assert!(matches!(
        migrate(&mut conn),
        Err(Error::DatabaseNewer { .. })
    ));

    // A half-migrated database (version 1 recorded, but a later table already exists) must fail
    // on the conflicting migration and stay at the last good version.
    let mut broken = rusqlite::Connection::open_in_memory()?;
    migrate(&mut broken)?;
    broken.pragma_update(None, "user_version", 1)?;
    assert!(matches!(
        migrate(&mut broken),
        Err(Error::Migration { version: 2, .. })
    ));
    assert_eq!(schema_version(&broken)?, 1);
    Ok(())
}
