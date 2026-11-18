//! Shared fixtures for the integration tests.
#![allow(dead_code)]

use db::*;
use rusqlite::Connection;

pub type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

/// A fully populated tenant.
pub struct Tenant {
    pub org: OrgId,
    pub user: UserId,
    pub membership: MembershipId,
    pub token: ApiTokenId,
    pub node: NodeId,
    pub agent: AgentId,
    pub secret: SecretId,
}

pub fn populate(conn: &Connection, slug: &str) -> std::result::Result<Tenant, Error> {
    let org = create_org(conn, &format!("Org {slug}"), slug, "free")?.id;
    let user = create_user(
        conn,
        &format!("{slug}@example.test"),
        "Test User",
        Some(&SecretText::new(format!("hash-{slug}"))),
    )?
    .id;
    let membership = add_member(conn, &org, &user, Role::Owner)?;
    let token = create_api_token(
        conn,
        &org,
        &NewApiToken {
            user_id: &user,
            name: "ci",
            token_hash: &SecretText::new(format!("token-hash-{slug}")),
            scopes_json: "[\"read\"]",
            expires_at: None,
        },
    )?;
    let node = create_node(
        conn,
        &org,
        &NewNode {
            name: "web-1",
            kind: NodeKind::Vps,
            access: NodeAccess::Agent,
            labels_json: "{}",
        },
    )?;
    let agent = register_agent(
        conn,
        &org,
        &NewAgent {
            node_id: &node,
            token_hash: &SecretText::new(format!("agent-hash-{slug}")),
            version: "0.0.1",
            os: "linux",
            arch: "x86_64",
        },
    )?;
    let secret = put_node_secret(
        conn,
        &org,
        &node,
        "ssh_key",
        &SealedSecret {
            ciphertext: vec![1, 2, 3, 4],
            key_id: "k1".into(),
        },
    )?;
    append_audit(
        conn,
        &org,
        &NewAuditEntry {
            actor_kind: ActorKind::User,
            actor_id: Some(user.as_str()),
            action: "node.create",
            node_id: Some(&node),
            subject_json: "{\"name\":\"web-1\"}",
            result: AuditOutcome::Success,
        },
    )?;
    Ok(Tenant {
        org,
        user,
        membership,
        token,
        node,
        agent,
        secret,
    })
}

pub fn is_not_found<T>(r: &std::result::Result<T, Error>) -> bool {
    matches!(r, Err(Error::NotFound))
}

pub fn is_constraint<T>(r: &std::result::Result<T, Error>) -> bool {
    matches!(r, Err(e) if e.is_constraint_violation())
}
