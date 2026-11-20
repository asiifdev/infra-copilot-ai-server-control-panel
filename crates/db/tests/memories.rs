//! Memories and full-text search: the trust model, the cap, injection safety and tenant isolation.

mod common;
use common::*;
use db::*;

fn new_memory<'a>(content: &'a str, source: MemorySource, status: MemoryStatus) -> NewMemory<'a> {
    NewMemory {
        user: None,
        kind: MemoryKind::OrgFact,
        content,
        source,
        conversation: None,
        status,
    }
}

fn pending<'a>(content: &'a str) -> NewMemory<'a> {
    new_memory(content, MemorySource::Assistant, MemoryStatus::Pending)
}

fn inventory(
    conn: &mut Connection,
    t: &Tenant,
    taken_at: &str,
    keys: &[(&str, &str)],
) -> Result<InventoryId> {
    let items: Vec<NewInventoryItem<'_>> = keys
        .iter()
        .map(|(key, data)| NewInventoryItem {
            kind: ItemKind::Listener,
            key,
            data_json: data,
            seen_via: ItemSource::Proc,
        })
        .collect();
    save_inventory(
        conn,
        &t.org,
        &NewInventory {
            node: &t.node,
            taken_at,
            collector_version: "0.1.0",
            status: InventoryStatus::Complete,
            os: "linux",
            arch: "x86_64",
            warnings_json: "[]",
        },
        &items,
    )
}

fn refs(hits: &[SearchHit]) -> Vec<&str> {
    hits.iter().map(|h| h.reference.as_str()).collect()
}

#[test]
fn fts5_is_compiled_in_and_the_tokenizer_is_configured() -> TestResult {
    let conn = open_in_memory()?;
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE name = 'search_fts'",
        [],
        |r| r.get(0),
    )?;
    assert!(sql.contains("unicode61 remove_diacritics 2"), "{sql}");
    Ok(())
}

#[test]
fn an_assistant_memory_cannot_start_active_not_even_by_raw_sql() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let refused = save_memory(
        &conn,
        &t.org,
        &new_memory("x", MemorySource::Assistant, MemoryStatus::Active),
    );
    assert!(matches!(
        refused,
        Err(Error::Invalid("memory_assistant_active"))
    ));
    // The schema refuses it too, whatever the Rust function says.
    let raw = conn.execute(
        "INSERT INTO memories (id, org_id, kind, content, source, status) \
         VALUES ('00000000-0000-4000-8000-0000000000aa', ?1, 'org_fact', 'x', 'assistant', 'active')",
        [t.org.as_str()],
    );
    assert!(raw.is_err());
    Ok(())
}

#[test]
fn pending_memories_are_hidden_from_prompts_and_search_until_approved() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let id = save_memory(
        &conn,
        &t.org,
        &pending("The billing cron runs on the old host"),
    )?;
    assert!(prompt_memories(&conn, &t.org, &t.user, 20)?.is_empty());
    assert!(search(&conn, &t.org, "billing", 10)?.is_empty());
    assert_eq!(
        count_memories(&conn, &t.org, Some(MemoryStatus::Pending), None)?,
        1
    );

    approve_memory(&conn, &t.org, &id)?;
    let shown = prompt_memories(&conn, &t.org, &t.user, 20)?;
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0].status, MemoryStatus::Active);
    let hits = search(&conn, &t.org, "billing", 10)?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, SearchKind::Memory);
    assert_eq!(hits[0].reference, format!("memory:{}", id.as_str()));
    // Approving twice is harmless.
    approve_memory(&conn, &t.org, &id)?;

    delete_memory(&conn, &t.org, &id)?;
    assert!(search(&conn, &t.org, "billing", 10)?.is_empty());
    assert!(is_not_found(&get_memory(&conn, &t.org, &id)));
    assert!(is_not_found(&delete_memory(&conn, &t.org, &id)));
    Ok(())
}

#[test]
fn text_is_redacted_cleaned_and_checked_after_redaction() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let secret = format!("{}{}", "sk-", "A1b2C3d4E5f6G7h8I9j0K1l2M3n4");
    let id = save_memory(
        &conn,
        &t.org,
        &pending(&format!("use key {secret}\nand\u{1b}[31m more")),
    )?;
    let row = get_memory(&conn, &t.org, &id)?;
    assert!(!row.content.contains(&secret), "{}", row.content);
    assert!(!row.content.chars().any(char::is_control));
    assert!(matches!(
        save_memory(&conn, &t.org, &pending("   \n\t ")),
        Err(Error::Invalid("memory_empty"))
    ));
    let long = "a".repeat(MAX_MEMORY_CHARS + 1);
    assert!(matches!(
        save_memory(&conn, &t.org, &pending(&long)),
        Err(Error::Invalid("memory_too_long"))
    ));
    assert!(save_memory(&conn, &t.org, &pending(&"a".repeat(MAX_MEMORY_CHARS))).is_ok());
    Ok(())
}

#[test]
fn the_org_cap_holds() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let other = populate(&conn, "beta")?;
    for i in 0..MAX_MEMORIES_PER_ORG {
        save_memory(&conn, &t.org, &pending(&format!("note number {i}")))?;
    }
    for _ in 0..3 {
        assert!(matches!(
            save_memory(&conn, &t.org, &pending("one more")),
            Err(Error::Invalid("memory_limit"))
        ));
    }
    assert_eq!(
        count_memories(&conn, &t.org, None, None)?,
        MAX_MEMORIES_PER_ORG
    );
    // Another org has its own budget.
    assert!(save_memory(&conn, &other.org, &pending("fine")).is_ok());
    Ok(())
}

#[test]
fn preferences_belong_to_a_member_and_stay_out_of_search_and_other_prompts() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let other = populate(&conn, "beta")?;
    let mine = NewMemory {
        user: Some(&t.user),
        kind: MemoryKind::Preference,
        content: "Prefer short answers",
        source: MemorySource::Person,
        conversation: None,
        status: MemoryStatus::Active,
    };
    save_memory(&conn, &t.org, &mine)?;
    assert_eq!(prompt_memories(&conn, &t.org, &t.user, 20)?.len(), 1);
    assert!(search(&conn, &t.org, "short answers", 10)?.is_empty());
    // A preference without a user, and a note with one, are refused.
    let bad = NewMemory {
        user: None,
        ..NewMemory {
            user: Some(&t.user),
            kind: MemoryKind::Preference,
            content: "x",
            source: MemorySource::Person,
            conversation: None,
            status: MemoryStatus::Active,
        }
    };
    assert!(matches!(
        save_memory(&conn, &t.org, &bad),
        Err(Error::Invalid("memory_user_mismatch"))
    ));
    // A user of another org cannot be the owner.
    let foreign = NewMemory {
        user: Some(&other.user),
        ..mine
    };
    assert!(is_not_found(&save_memory(&conn, &t.org, &foreign)));
    Ok(())
}

#[test]
fn memories_survive_the_deletion_of_their_conversation() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let conversation = create_conversation(&conn, &t.org, &t.user, "chat", None)?;
    let id = save_memory(
        &conn,
        &t.org,
        &NewMemory {
            conversation: Some(&conversation),
            ..pending("kept")
        },
    )?;
    assert_eq!(count_memories(&conn, &t.org, None, Some(&conversation))?, 1);
    delete_conversation(&conn, &t.org, &conversation)?;
    let row = get_memory(&conn, &t.org, &id)?;
    assert_eq!(row.conversation_id, None);
    Ok(())
}

#[test]
fn a_conversation_of_another_org_is_refused() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let other = populate(&conn, "beta")?;
    let foreign = create_conversation(&conn, &other.org, &other.user, "chat", None)?;
    let r = save_memory(
        &conn,
        &t.org,
        &NewMemory {
            conversation: Some(&foreign),
            ..pending("x")
        },
    );
    assert!(is_constraint(&r), "{r:?}");
    Ok(())
}

#[test]
fn search_finds_items_of_the_latest_inventory_only_and_follows_changes() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    inventory(
        &mut conn,
        &t,
        "2026-11-19T10:00:00Z",
        &[("tcp:8080", r#"{"process":"oldproxy"}"#)],
    )?;
    assert_eq!(search(&conn, &t.org, "oldproxy", 10)?.len(), 1);
    inventory(
        &mut conn,
        &t,
        "2026-11-19T11:00:00Z",
        &[("tcp:9090", r#"{"process":"newproxy"}"#)],
    )?;
    // The older snapshot is history: it is no longer found.
    assert!(search(&conn, &t.org, "oldproxy", 10)?.is_empty());
    let hits = search(&conn, &t.org, "newproxy", 10)?;
    assert_eq!(refs(&hits), ["item:listener:tcp:9090"]);
    assert_eq!(hits[0].node.as_deref(), Some("web-1"));
    // Punctuation splits words: the port number alone finds the key.
    assert_eq!(search(&conn, &t.org, "9090", 10)?.len(), 1);
    Ok(())
}

#[test]
fn search_finds_findings_of_the_latest_comparison() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let a = inventory(&mut conn, &t, "2026-11-19T10:00:00Z", &[("tcp:80", "{}")])?;
    let b = inventory(&mut conn, &t, "2026-11-19T11:00:00Z", &[("tcp:81", "{}")])?;
    let finding = |key| NewFinding {
        rule_id: "PAR-001",
        severity: FindingSeverity::Critical,
        subject_kind: ItemKind::Listener,
        subject_key: key,
        evidence_json: "[]",
        args_json: "{}",
    };
    save_comparison(&mut conn, &t.org, &a, &b, &[finding("tcp:7700")])?;
    std::thread::sleep(std::time::Duration::from_millis(5));
    let latest = save_comparison(&mut conn, &t.org, &a, &b, &[finding("tcp:7701")])?;
    assert!(search(&conn, &t.org, "7700", 10)?.is_empty());
    let hits = search(&conn, &t.org, "7701", 10)?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].kind, SearchKind::Finding);
    assert_eq!(hits[0].reference, "finding:PAR-001:listener:tcp:7701");
    let _ = latest;
    Ok(())
}

#[test]
fn the_index_follows_deletes_including_cascades_so_it_cannot_drift() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    inventory(&mut conn, &t, "2026-11-19T10:00:00Z", &[("tcp:4242", "{}")])?;
    let docs = |c: &Connection| -> rusqlite::Result<i64> {
        c.query_row("SELECT count(*) FROM search_docs", [], |r| r.get(0))
    };
    assert_eq!(docs(&conn)?, 1);
    // Deleting the node cascades through inventories to items; the index must follow.
    delete_node(&conn, &t.org, &t.node)?;
    assert_eq!(docs(&conn)?, 0);
    assert!(search(&conn, &t.org, "4242", 10)?.is_empty());
    // The FTS structure itself is consistent with its content table.
    conn.execute(
        "INSERT INTO search_fts (search_fts) VALUES ('integrity-check')",
        [],
    )?;
    Ok(())
}

#[test]
fn existing_rows_are_indexed_when_the_migration_runs() -> TestResult {
    // A database at schema version 8 with an inventory and a finding already in it.
    let mut conn = rusqlite::Connection::open_in_memory()?;
    conn.pragma_update(None, "foreign_keys", true)?;
    for sql in [
        include_str!("../migrations/0001_identity.sql"),
        include_str!("../migrations/0002_nodes.sql"),
        include_str!("../migrations/0003_audit_log.sql"),
        include_str!("../migrations/0004_inventories.sql"),
        include_str!("../migrations/0005_secrets_ai_providers.sql"),
        include_str!("../migrations/0006_conversations.sql"),
        include_str!("../migrations/0007_transports.sql"),
        include_str!("../migrations/0008_migration_plans.sql"),
    ] {
        conn.execute_batch(sql)?;
    }
    conn.pragma_update(None, "user_version", 8)?;
    let t = populate(&conn, "alpha")?;
    let inv = inventory(
        &mut conn,
        &t,
        "2026-11-19T10:00:00Z",
        &[("tcp:3306", r#"{"process":"mariadb"}"#)],
    )?;
    save_comparison(
        &mut conn,
        &t.org,
        &inv,
        &inv,
        &[NewFinding {
            rule_id: "PAR-002",
            severity: FindingSeverity::Important,
            subject_kind: ItemKind::Listener,
            subject_key: "tcp:3306",
            evidence_json: "[]",
            args_json: "{}",
        }],
    )?;
    assert_eq!(schema_version(&conn)?, 8);
    migrate(&mut conn)?;
    assert_eq!(schema_version(&conn)?, LATEST_VERSION);
    let hits = search(&conn, &t.org, "3306", 10)?;
    let mut found = refs(&hits);
    found.sort_unstable();
    assert_eq!(
        found,
        [
            "finding:PAR-002:listener:tcp:3306",
            "item:listener:tcp:3306"
        ]
    );
    Ok(())
}

#[test]
fn injection_attempts_in_the_query_are_harmless() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    inventory(
        &mut conn,
        &t,
        "2026-11-19T10:00:00Z",
        &[("tcp:5432", r#"{"process":"postgres"}"#)],
    )?;
    let id = save_memory(&conn, &t.org, &pending("hidden pending note"))?;
    let _ = id;
    let huge = "x".repeat(200_000);
    let many = "word ".repeat(10_000);
    for evil in [
        "\"",
        "\"unbalanced",
        "postgres OR secret",
        "NEAR(postgres hidden, 3)",
        "title:postgres",
        "body:hidden",
        "{title body}: postgres",
        "post*",
        "^postgres",
        "postgres AND NOT x",
        "postgres\0",
        "'; DROP TABLE memories; --",
        "((((",
        "\u{0}\u{1}\u{2}",
        "\u{b2}\u{b3}",
        &huge,
        &many,
        "",
        "   ",
    ] {
        // No query may error, and a pending memory must never appear.
        let hits = search(&conn, &t.org, evil, 10)?;
        assert!(
            hits.iter().all(|h| h.kind != SearchKind::Memory),
            "{evil:.40?}"
        );
    }
    // Operators were dropped, not interpreted: "post*" is the literal word "post", no prefix.
    assert!(search(&conn, &t.org, "post*", 10)?.is_empty());
    // A column filter is not a column filter: "title:postgres" is two plain words.
    let hits = search(&conn, &t.org, "title:postgres", 10)?;
    assert_eq!(hits.len(), 1);
    // The pending note is not findable by any phrasing.
    assert!(search(&conn, &t.org, "hidden pending note", 10)?.is_empty());
    // The table survived the DROP attempt.
    assert_eq!(count_memories(&conn, &t.org, None, None)?, 1);
    Ok(())
}

#[test]
fn org_a_never_sees_org_b_rows_through_any_query() -> TestResult {
    let mut conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    // The same words in both orgs, on both sides of every searchable kind.
    for t in [&a, &b] {
        inventory(
            &mut conn,
            t,
            "2026-11-19T10:00:00Z",
            &[("tcp:6379", r#"{"process":"cachebox"}"#)],
        )?;
    }
    let ia = latest_inventory(&conn, &a.org, &a.node)?.id;
    let ib = latest_inventory(&conn, &b.org, &b.node)?.id;
    save_comparison(
        &mut conn,
        &b.org,
        &ib,
        &ib,
        &[NewFinding {
            rule_id: "PAR-009",
            severity: FindingSeverity::Info,
            subject_kind: ItemKind::Listener,
            subject_key: "tcp:6379",
            evidence_json: "[]",
            args_json: "{}",
        }],
    )?;
    let secret_b = save_memory(
        &conn,
        &b.org,
        &new_memory(
            "cachebox belongs to beta only",
            MemorySource::Person,
            MemoryStatus::Active,
        ),
    )?;
    save_memory(
        &conn,
        &a.org,
        &new_memory(
            "alpha note about cachebox",
            MemorySource::Person,
            MemoryStatus::Active,
        ),
    )?;

    let hits = search(&conn, &a.org, "cachebox", 50)?;
    assert_eq!(hits.len(), 2, "{hits:?}");
    for hit in &hits {
        assert!(!hit.snippet.contains("beta"), "{hit:?}");
        assert!(hit.reference != format!("memory:{}", secret_b.as_str()));
        assert!(!hit.reference.starts_with("finding:"));
    }
    // The finding of org B is invisible to A even by its exact subject.
    assert!(
        search(&conn, &a.org, "PAR-009", 50)?
            .iter()
            .all(|h| h.kind != SearchKind::Finding)
    );
    // Memory reads, counts and lifecycle never cross either.
    assert!(is_not_found(&get_memory(&conn, &a.org, &secret_b)));
    assert!(is_not_found(&approve_memory(&conn, &a.org, &secret_b)));
    assert!(is_not_found(&delete_memory(&conn, &a.org, &secret_b)));
    assert_eq!(list_memories(&conn, &a.org, None)?.len(), 1);
    assert_eq!(prompt_memories(&conn, &a.org, &a.user, 20)?.len(), 1);
    assert_eq!(count_memories(&conn, &a.org, None, None)?, 1);
    // B still has its memory after A's attempts.
    assert!(get_memory(&conn, &b.org, &secret_b).is_ok());
    let _ = ia;
    Ok(())
}

#[test]
fn search_folds_case_and_diacritics_in_both_directions() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let note = |text| new_memory(text, MemorySource::Person, MemoryStatus::Active);
    save_memory(
        &conn,
        &t.org,
        &note("Sauvegarde du serveur de r\u{e9}sum\u{e9}"),
    )?;
    save_memory(&conn, &t.org, &note("Deploy runs from the CAFE host"))?;
    assert_eq!(search(&conn, &t.org, "resume", 10)?.len(), 1);
    assert_eq!(search(&conn, &t.org, "R\u{c9}SUM\u{c9}", 10)?.len(), 1);
    assert_eq!(search(&conn, &t.org, "caf\u{e9}", 10)?.len(), 1);
    Ok(())
}
