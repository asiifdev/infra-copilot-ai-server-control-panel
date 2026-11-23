//! Org A must not read, change or delete anything of org B through any public function,
//! even when it guesses B's ids.

mod common;
use common::*;
use db::*;

#[test]
fn org_a_cannot_touch_org_b_through_any_function() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let before_b_nodes = list_nodes(&conn, &b.org)?;
    let before_b_members = list_members(&conn, &b.org)?;
    let before_b_tokens = list_api_tokens(&conn, &b.org)?;
    let before_b_secret = get_node_secret(&conn, &b.org, &b.secret)?;
    let before_b_audit = list_audit(&conn, &b.org, 100)?;

    // Reads with B's guessed ids, scoped as A.
    assert!(is_not_found(&get_node(&conn, &a.org, &b.node)));
    assert!(is_not_found(&get_node_secret(&conn, &a.org, &b.secret)));
    // A user is only visible through an org they belong to.
    assert!(is_not_found(&get_member_user(&conn, &a.org, &b.user)));
    assert!(get_member_user(&conn, &b.org, &b.user).is_ok());
    assert!(list_agents(&conn, &a.org, &b.node)?.is_empty());
    assert!(list_node_secrets(&conn, &a.org, &b.node)?.is_empty());

    // Lists contain only A's rows.
    assert!(list_nodes(&conn, &a.org)?.iter().all(|n| n.id != b.node));
    assert!(
        list_members(&conn, &a.org)?
            .iter()
            .all(|m| m.user_id != b.user)
    );
    assert!(
        list_api_tokens(&conn, &a.org)?
            .iter()
            .all(|t| t.id != b.token)
    );
    assert!(
        list_audit(&conn, &a.org, 100)?
            .iter()
            .all(|e| e.node_id.as_deref() != Some(b.node.as_str()))
    );

    // Writes with B's guessed ids, scoped as A, find nothing.
    assert!(is_not_found(&set_member_role(
        &conn,
        &a.org,
        &b.membership,
        Role::Viewer
    )));
    assert!(is_not_found(&remove_member(&conn, &a.org, &b.membership)));
    assert!(is_not_found(&revoke_api_token(&conn, &a.org, &b.token)));
    assert!(is_not_found(&set_node_status(
        &conn,
        &a.org,
        &b.node,
        NodeStatus::Revoked
    )));
    assert!(is_not_found(&touch_agent(&conn, &a.org, &b.agent)));
    assert!(is_not_found(&revoke_agent(&conn, &a.org, &b.agent)));
    assert!(is_not_found(&delete_node_secret(&conn, &a.org, &b.secret)));
    assert!(is_not_found(&delete_node(&conn, &a.org, &b.node)));

    // B's user is not a member of A, so A cannot mint a token for them.
    let hash = SecretText::new("x");
    let minted = create_api_token(
        &conn,
        &a.org,
        &NewApiToken {
            user_id: &b.user,
            name: "t",
            token_hash: &hash,
            scopes_json: "[]",
            expires_at: None,
        },
    );
    assert!(is_not_found(&minted));

    // Cross-org links are refused by the schema (composite foreign keys).
    let agent = register_agent(
        &conn,
        &a.org,
        &NewAgent {
            node_id: &b.node,
            token_hash: &SecretText::new("y"),
            version: "1",
            os: "linux",
            arch: "arm64",
        },
    );
    assert!(is_constraint(&agent));
    let secret = put_node_secret(
        &conn,
        &a.org,
        &b.node,
        "ssh_key",
        &SealedSecret {
            ciphertext: vec![9],
            key_id: "k".into(),
        },
    );
    assert!(is_constraint(&secret));

    // Audit chains are independent and both intact.
    assert_eq!(
        verify_audit_chain(&conn, &a.org)?,
        ChainStatus::Intact { entries: 1 }
    );
    assert_eq!(
        verify_audit_chain(&conn, &b.org)?,
        ChainStatus::Intact { entries: 1 }
    );

    // B is untouched.
    assert_eq!(list_nodes(&conn, &b.org)?, before_b_nodes);
    assert_eq!(list_members(&conn, &b.org)?, before_b_members);
    assert_eq!(list_api_tokens(&conn, &b.org)?, before_b_tokens);
    assert_eq!(get_node_secret(&conn, &b.org, &b.secret)?, before_b_secret);
    assert_eq!(list_audit(&conn, &b.org, 100)?, before_b_audit);
    assert_eq!(list_agents(&conn, &b.org, &b.node)?.len(), 1);
    Ok(())
}

#[test]
fn same_names_in_different_orgs_do_not_collide() -> TestResult {
    let conn = open_in_memory()?;
    // Both tenants own a node called "web-1"; each only sees its own.
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    assert_ne!(a.node, b.node);
    assert_eq!(list_nodes(&conn, &a.org)?.len(), 1);
    assert_eq!(list_nodes(&conn, &b.org)?.len(), 1);
    // Within one org the name is unique.
    let dup = create_node(
        &conn,
        &a.org,
        &NewNode {
            name: "web-1",
            kind: NodeKind::Vps,
            access: NodeAccess::Ssh,
            labels_json: "{}",
        },
    );
    assert!(is_constraint(&dup));
    Ok(())
}

#[test]
fn deleting_a_node_keeps_its_audit_history_and_removes_children() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    delete_node(&conn, &a.org, &a.node)?;
    assert!(list_agents(&conn, &a.org, &a.node)?.is_empty());
    assert!(is_not_found(&get_node_secret(&conn, &a.org, &a.secret)));
    assert_eq!(list_audit(&conn, &a.org, 10)?.len(), 1);
    assert_eq!(
        verify_audit_chain(&conn, &a.org)?,
        ChainStatus::Intact { entries: 1 }
    );
    Ok(())
}

#[test]
fn removing_a_member_deletes_their_api_tokens() -> TestResult {
    let conn = open_in_memory()?;
    let a = populate(&conn, "tok-a")?;
    assert_eq!(list_api_tokens(&conn, &a.org)?.len(), 1);
    remove_member(&conn, &a.org, &a.membership)?;
    assert!(list_api_tokens(&conn, &a.org)?.is_empty());
    Ok(())
}
