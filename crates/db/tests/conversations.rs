//! Conversations, messages, tool call records, the turn claim and proposals: tenant isolation,
//! ordering, claim semantics and cascades.

mod common;
use common::*;
use db::*;
use rusqlite::params;

fn is_not_found<T>(r: &Result<T>) -> bool {
    matches!(r, Err(Error::NotFound))
}

fn msg(role: MessageRole, content: &str) -> NewMessage<'_> {
    NewMessage {
        role,
        content,
        tool_calls_json: None,
        tool_call_id: None,
        token_usage_json: None,
        redactions: 0,
    }
}

fn claim(conn: &Connection, t: &Tenant, c: &ConversationId) -> TestResultOf<TurnClaim> {
    match claim_turn(conn, &t.org, c, 600)? {
        ClaimOutcome::Claimed(token) => Ok(token),
        ClaimOutcome::Busy => Err("unexpectedly busy".into()),
    }
}

type TestResultOf<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn proposal<'a>(conversation: Option<&'a ConversationId>, hash: &'a str) -> NewProposal<'a> {
    NewProposal {
        conversation,
        tool: "propose_fix",
        tier: 1,
        title: "Open the port",
        rationale: "The old server listened on it.",
        suggested_action: "Allow the port in the firewall.",
        impact: "Records a pending proposal; changes nothing.",
        rule_id: Some("PAR-001"),
        subject_key: Some("tcp:443"),
        args_hash: hash,
    }
}

#[test]
fn messages_keep_their_order_and_sequence_numbers() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let c = create_conversation(&conn, &t.org, &t.user, "Why is the port closed?", None)?;
    let token = claim(&conn, &t, &c)?;
    let seqs = append_messages(
        &conn,
        &t.org,
        &c,
        &token,
        &[
            msg(MessageRole::User, "one"),
            msg(MessageRole::Assistant, "two"),
        ],
        &[],
    )?;
    assert_eq!(seqs, vec![1, 2]);
    let more = append_messages(
        &conn,
        &t.org,
        &c,
        &token,
        &[msg(MessageRole::User, "three")],
        &[],
    )?;
    assert_eq!(more, vec![3]);
    let all = list_messages(&conn, &t.org, &c, 100)?;
    assert_eq!(
        all.iter().map(|m| m.content.as_str()).collect::<Vec<_>>(),
        ["one", "two", "three"]
    );
    // `last` keeps the newest, still oldest first.
    let tail = list_messages(&conn, &t.org, &c, 2)?;
    assert_eq!(tail.iter().map(|m| m.seq).collect::<Vec<_>>(), vec![2, 3]);
    Ok(())
}

#[test]
fn a_sequence_number_cannot_be_reused() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let c = create_conversation(&conn, &t.org, &t.user, "t", None)?;
    let token = claim(&conn, &t, &c)?;
    append_messages(
        &conn,
        &t.org,
        &c,
        &token,
        &[msg(MessageRole::User, "a")],
        &[],
    )?;
    let again = conn.execute(
        "INSERT INTO messages (org_id, conversation_id, seq, role, content) VALUES (?1, ?2, 1, 'user', 'dup')",
        params![t.org.as_str(), c.as_str()],
    );
    assert!(again.is_err());
    Ok(())
}

#[test]
fn org_b_cannot_read_continue_or_delete_org_a_conversations() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let c = create_conversation(&conn, &a.org, &a.user, "secret plans", None)?;
    let token = claim(&conn, &a, &c)?;
    append_messages(
        &conn,
        &a.org,
        &c,
        &token,
        &[msg(MessageRole::User, "hi")],
        &[],
    )?;
    release_turn(&conn, &a.org, &c, &token)?;

    assert!(is_not_found(&get_conversation(&conn, &b.org, &c)));
    assert!(is_not_found(&list_messages(&conn, &b.org, &c, 10)));
    assert!(is_not_found(&list_tool_calls(&conn, &b.org, &c)));
    assert!(is_not_found(&claim_turn(&conn, &b.org, &c, 600)));
    assert!(is_not_found(&archive_conversation(&conn, &b.org, &c)));
    assert!(is_not_found(&delete_conversation(&conn, &b.org, &c)));
    assert!(list_conversations(&conn, &b.org, &a.user, true, 10)?.is_empty());
    // A token of A's conversation does not open it under B's org either.
    let held = claim(&conn, &a, &c)?;
    let stolen = append_messages(
        &conn,
        &b.org,
        &c,
        &held,
        &[msg(MessageRole::User, "x")],
        &[],
    );
    assert!(is_not_found(&stolen));
    // A user that is not a member of the org cannot own a conversation there.
    assert!(is_not_found(&create_conversation(
        &conn, &a.org, &b.user, "x", None
    )));
    // A provider of another org is refused by the composite key.
    let other = create_ai_provider(
        &conn,
        &b.org,
        &NewAiProvider {
            name: "p",
            base_url: "https://api.example.test/v1",
            default_model: "m",
            secret: None,
            params_json: "{}",
            is_default: false,
        },
    )?;
    let linked = create_conversation(&conn, &a.org, &a.user, "x", Some(&other));
    assert!(linked.is_err_and(|e| e.is_constraint_violation()));
    // A is untouched.
    assert_eq!(list_messages(&conn, &a.org, &c, 10)?.len(), 1);
    Ok(())
}

#[test]
fn a_second_claim_is_busy_until_release_or_staleness() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let c = create_conversation(&conn, &t.org, &t.user, "t", None)?;
    let first = claim(&conn, &t, &c)?;
    assert_eq!(claim_turn(&conn, &t.org, &c, 600)?, ClaimOutcome::Busy);

    release_turn(&conn, &t.org, &c, &first)?;
    let second = claim(&conn, &t, &c)?;
    // Releasing again with the old, already released token must not free the new claim.
    release_turn(&conn, &t.org, &c, &first)?;
    assert_eq!(claim_turn(&conn, &t.org, &c, 600)?, ClaimOutcome::Busy);

    // Make the claim stale by moving its heartbeat into the past.
    conn.execute(
        "UPDATE conversations SET turn_claimed_at = '2000-01-01T00:00:00.000Z' WHERE id = ?1",
        params![c.as_str()],
    )?;
    let third = match claim_turn(&conn, &t.org, &c, 600)? {
        ClaimOutcome::Claimed(token) => token,
        ClaimOutcome::Busy => return Err("a stale claim must be taken over".into()),
    };
    // The old holder can neither write nor heartbeat nor release the new claim.
    let late = append_messages(
        &conn,
        &t.org,
        &c,
        &second,
        &[msg(MessageRole::User, "late")],
        &[],
    );
    assert!(is_not_found(&late));
    assert!(is_not_found(&touch_turn(&conn, &t.org, &c, &second)));
    release_turn(&conn, &t.org, &c, &second)?;
    assert_eq!(claim_turn(&conn, &t.org, &c, 600)?, ClaimOutcome::Busy);
    touch_turn(&conn, &t.org, &c, &third)?;
    Ok(())
}

#[test]
fn an_archived_conversation_cannot_be_claimed_or_written() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let c = create_conversation(&conn, &t.org, &t.user, "t", None)?;
    let token = claim(&conn, &t, &c)?;
    archive_conversation(&conn, &t.org, &c)?;
    let write = append_messages(
        &conn,
        &t.org,
        &c,
        &token,
        &[msg(MessageRole::User, "x")],
        &[],
    );
    assert!(is_not_found(&write));
    release_turn(&conn, &t.org, &c, &token)?;
    assert!(matches!(
        claim_turn(&conn, &t.org, &c, 600),
        Err(Error::Invalid(_))
    ));
    assert!(list_conversations(&conn, &t.org, &t.user, false, 10)?.is_empty());
    assert_eq!(
        list_conversations(&conn, &t.org, &t.user, true, 10)?.len(),
        1
    );
    Ok(())
}

#[test]
fn tool_calls_attach_to_their_message_and_cascade_with_the_conversation() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let c = create_conversation(&conn, &t.org, &t.user, "t", None)?;
    let token = claim(&conn, &t, &c)?;
    let calls_json = r#"[{"id":"c1","name":"list_nodes","arguments":"{}"}]"#;
    let assistant = NewMessage {
        tool_calls_json: Some(calls_json),
        ..msg(MessageRole::Assistant, "")
    };
    let result = NewMessage {
        tool_call_id: Some("c1"),
        ..msg(MessageRole::Tool, "web-1")
    };
    let call = NewToolCall {
        message_index: 0,
        idx: 0,
        call_id: "c1",
        tool: "list_nodes",
        tier: Some(0),
        args: "{}",
        status: "ok",
        duration_ms: 3,
        redactions: 0,
        truncated: false,
    };
    let seqs = append_messages(
        &conn,
        &t.org,
        &c,
        &token,
        &[assistant, result],
        std::slice::from_ref(&call),
    )?;
    assert_eq!(seqs, vec![1, 2]);
    let rows = list_tool_calls(&conn, &t.org, &c)?;
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].message_seq, rows[0].tier), (1, Some(0)));

    // A bad message index writes nothing at all.
    let bad = NewToolCall {
        message_index: 5,
        ..call.clone()
    };
    let before = list_messages(&conn, &t.org, &c, 100)?.len();
    assert!(matches!(
        append_messages(
            &conn,
            &t.org,
            &c,
            &token,
            &[msg(MessageRole::User, "z")],
            &[bad]
        ),
        Err(Error::Invalid(_))
    ));
    // A failing batch rolls back the messages written before the failure.
    let too_long = "x".repeat(5000);
    let broken = NewToolCall {
        args: &too_long,
        ..call
    };
    assert!(
        append_messages(
            &conn,
            &t.org,
            &c,
            &token,
            &[msg(MessageRole::User, "z")],
            &[broken]
        )
        .is_err()
    );
    assert_eq!(list_messages(&conn, &t.org, &c, 100)?.len(), before);

    release_turn(&conn, &t.org, &c, &token)?;
    delete_conversation(&conn, &t.org, &c)?;
    let left: i64 = conn.query_row("SELECT COUNT(*) FROM messages", [], |r| r.get(0))?;
    let left_calls: i64 = conn.query_row("SELECT COUNT(*) FROM tool_calls", [], |r| r.get(0))?;
    assert_eq!((left, left_calls), (0, 0));
    Ok(())
}

#[test]
fn a_tool_call_cannot_point_at_another_conversation_message() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let ca = create_conversation(&conn, &a.org, &a.user, "a", None)?;
    let token = claim(&conn, &a, &ca)?;
    append_messages(
        &conn,
        &a.org,
        &ca,
        &token,
        &[msg(MessageRole::Assistant, "x")],
        &[],
    )?;
    // Raw insert with B's org id and A's conversation: the composite key refuses it.
    let forged = conn.execute(
        "INSERT INTO tool_calls (id, org_id, conversation_id, message_seq, idx, call_id, tool, args, status, duration_ms) \
         VALUES ('11111111-1111-4111-8111-111111111111', ?1, ?2, 1, 0, 'c', 't', '{}', 'ok', 0)",
        params![b.org.as_str(), ca.as_str()],
    );
    assert!(forged.is_err());
    Ok(())
}

#[test]
fn the_conversation_title_is_redacted_and_must_not_be_empty() -> TestResult {
    let conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let c = create_conversation(
        &conn,
        &t.org,
        &t.user,
        "fix DB_PASSWORD=hunter2hunter2 now",
        None,
    )?;
    let row = get_conversation(&conn, &t.org, &c)?;
    assert!(!row.title.contains("hunter2"), "{}", row.title);
    assert!(matches!(
        create_conversation(&conn, &t.org, &t.user, "   ", None),
        Err(Error::Invalid(_))
    ));
    Ok(())
}

#[test]
fn proposals_are_pending_scoped_and_survive_only_as_records() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let c = create_conversation(&conn, &a.org, &a.user, "t", None)?;
    let hash = sha256_hex(b"args");
    let id = create_proposal(&conn, &a.org, &proposal(Some(&c), &hash))?;
    let row = get_proposal(&conn, &a.org, &id)?;
    assert_eq!(row.status, ProposalStatus::Pending);
    assert_eq!((row.tier, row.conversation_id.as_ref()), (1, Some(&c)));
    assert_eq!(count_pending_proposals(&conn, &a.org, &c)?, 1);
    assert_eq!(
        list_proposals(&conn, &a.org, Some(ProposalStatus::Pending), 10)?.len(),
        1
    );
    assert!(list_proposals(&conn, &a.org, Some(ProposalStatus::Approved), 10)?.is_empty());

    // B sees nothing of A, and cannot attach a proposal to A's conversation.
    assert!(is_not_found(&get_proposal(&conn, &b.org, &id)));
    assert!(list_proposals(&conn, &b.org, None, 10)?.is_empty());
    let cross = create_proposal(&conn, &b.org, &proposal(Some(&c), &hash));
    assert!(cross.is_err_and(|e| e.is_constraint_violation()));

    // Out-of-range values and a status outside the set are refused by the schema.
    let bad_tier = NewProposal {
        tier: 0,
        ..proposal(None, &hash)
    };
    assert!(create_proposal(&conn, &a.org, &bad_tier).is_err());
    let bad_hash = NewProposal {
        args_hash: "short",
        ..proposal(None, &hash)
    };
    assert!(create_proposal(&conn, &a.org, &bad_hash).is_err());
    let bad_status = conn.execute(
        "UPDATE proposals SET status = 'running' WHERE id = ?1",
        params![id.as_str()],
    );
    assert!(bad_status.is_err());

    // A conversation with proposals can be archived but not deleted.
    assert!(delete_conversation(&conn, &a.org, &c).is_err_and(|e| e.is_constraint_violation()));
    archive_conversation(&conn, &a.org, &c)?;
    // A proposal without a conversation is allowed (nullable link).
    create_proposal(&conn, &a.org, &proposal(None, &hash))?;
    Ok(())
}

#[test]
fn the_sha256_helper_matches_the_known_vector() {
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
}
