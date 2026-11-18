//! Nodes, agents and sealed node secrets (docs/design.md 3.2). All functions are org-scoped.

use crate::error::{Error, Result};
use crate::identity::found;
use crate::ids::{AgentId, NodeId, OrgId, SecretId};
use crate::secret::{SealedSecret, SecretText};
use crate::{new_uuid, now_utc};
use rusqlite::{Connection, OptionalExtension, params};

text_enum!(
    /// What kind of machine a node is.
    NodeKind {
        /// A remote server.
        Vps => "vps",
        /// The machine the product runs on.
        Local => "local",
    }
);

text_enum!(
    /// How the product reaches a node.
    NodeAccess {
        /// Through the agent (outbound connection).
        Agent => "agent",
        /// Through SSH.
        Ssh => "ssh",
        /// Directly (local machine).
        Local => "local",
    }
);

text_enum!(
    /// Lifecycle state of a node.
    NodeStatus {
        /// Registered, not yet seen.
        Pending => "pending",
        /// Reachable.
        Online => "online",
        /// Not reachable.
        Offline => "offline",
        /// Access withdrawn.
        Revoked => "revoked",
    }
);

/// Input for creating a node.
#[derive(Debug)]
pub struct NewNode<'a> {
    /// Unique name inside the org.
    pub name: &'a str,
    /// Machine kind.
    pub kind: NodeKind,
    /// Access method.
    pub access: NodeAccess,
    /// JSON object of labels.
    pub labels_json: &'a str,
}

/// A node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    /// Node id.
    pub id: NodeId,
    /// Name.
    pub name: String,
    /// Machine kind.
    pub kind: NodeKind,
    /// Access method.
    pub access: NodeAccess,
    /// State.
    pub status: NodeStatus,
    /// JSON object of labels.
    pub labels_json: String,
    /// Creation time.
    pub created_at: String,
}

/// Input for registering an agent.
#[derive(Debug)]
pub struct NewAgent<'a> {
    /// Node the agent runs on.
    pub node_id: &'a NodeId,
    /// Hash of the agent token.
    pub token_hash: &'a SecretText,
    /// Agent version.
    pub version: &'a str,
    /// Operating system.
    pub os: &'a str,
    /// CPU architecture.
    pub arch: &'a str,
}

/// An agent (token hash is not returned).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Agent {
    /// Agent id.
    pub id: AgentId,
    /// Node it runs on.
    pub node_id: NodeId,
    /// Version.
    pub version: String,
    /// Operating system.
    pub os: String,
    /// Architecture.
    pub arch: String,
    /// Last contact.
    pub last_seen_at: Option<String>,
    /// Revocation time, if revoked.
    pub revoked_at: Option<String>,
}

/// Secret metadata: everything except the ciphertext.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeSecretMeta {
    /// Secret id.
    pub id: SecretId,
    /// Node it belongs to.
    pub node_id: NodeId,
    /// Purpose, such as `ssh_key`.
    pub purpose: String,
    /// Master key version.
    pub key_id: String,
    /// Creation time.
    pub created_at: String,
}

/// Creates a node.
///
/// # Errors
/// Constraint violation for a duplicate name, bad labels JSON or unknown org.
pub fn create_node(conn: &Connection, org: &OrgId, new: &NewNode<'_>) -> Result<NodeId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO nodes (id, org_id, name, kind, access, labels) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![id, org.as_str(), new.name, new.kind.as_str(), new.access.as_str(), new.labels_json],
    )?;
    Ok(NodeId::from_db(id))
}

const NODE_COLUMNS: &str = "id, name, kind, access, status, labels, created_at";

fn map_node(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<(String, String, String, String, String, String, String)> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
    ))
}

fn build_node(t: (String, String, String, String, String, String, String)) -> Result<Node> {
    Ok(Node {
        id: NodeId::from_db(t.0),
        name: t.1,
        kind: NodeKind::parse(&t.2)?,
        access: NodeAccess::parse(&t.3)?,
        status: NodeStatus::parse(&t.4)?,
        labels_json: t.5,
        created_at: t.6,
    })
}

/// Reads one node.
///
/// # Errors
/// [`Error::NotFound`] if it is missing or belongs to another org.
pub fn get_node(conn: &Connection, org: &OrgId, id: &NodeId) -> Result<Node> {
    let row = conn
        .query_row(
            &format!("SELECT {NODE_COLUMNS} FROM nodes WHERE org_id = ?1 AND id = ?2"),
            params![org.as_str(), id.as_str()],
            map_node,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    build_node(row)
}

/// Lists the nodes of an org.
///
/// # Errors
/// Database failure.
pub fn list_nodes(conn: &Connection, org: &OrgId) -> Result<Vec<Node>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {NODE_COLUMNS} FROM nodes WHERE org_id = ?1 ORDER BY name, id"
    ))?;
    let rows = stmt.query_map(params![org.as_str()], map_node)?;
    rows.map(|r| build_node(r?)).collect()
}

/// Sets a node's status.
///
/// # Errors
/// [`Error::NotFound`] if the node is not in this org.
pub fn set_node_status(
    conn: &Connection,
    org: &OrgId,
    id: &NodeId,
    status: NodeStatus,
) -> Result<()> {
    let n = conn.execute(
        "UPDATE nodes SET status = ?3 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), status.as_str()],
    )?;
    found(n)
}

/// Deletes a node and, by cascade, its agents and secrets. Audit history is kept.
///
/// # Errors
/// [`Error::NotFound`] if the node is not in this org.
pub fn delete_node(conn: &Connection, org: &OrgId, id: &NodeId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM nodes WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    found(n)
}

/// Registers an agent for a node of this org.
///
/// # Errors
/// Constraint violation if the node is not in this org (composite foreign key) or the hash repeats.
pub fn register_agent(conn: &Connection, org: &OrgId, new: &NewAgent<'_>) -> Result<AgentId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO agents (id, org_id, node_id, token_hash, version, os, arch) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        params![
            id,
            org.as_str(),
            new.node_id.as_str(),
            new.token_hash.expose(),
            new.version,
            new.os,
            new.arch
        ],
    )?;
    Ok(AgentId::from_db(id))
}

/// Lists the agents of one node.
///
/// # Errors
/// Database failure.
pub fn list_agents(conn: &Connection, org: &OrgId, node: &NodeId) -> Result<Vec<Agent>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_id, version, os, arch, last_seen_at, revoked_at FROM agents \
         WHERE org_id = ?1 AND node_id = ?2 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map(params![org.as_str(), node.as_str()], |row| {
        Ok(Agent {
            id: AgentId::from_db(row.get(0)?),
            node_id: NodeId::from_db(row.get(1)?),
            version: row.get(2)?,
            os: row.get(3)?,
            arch: row.get(4)?,
            last_seen_at: row.get(5)?,
            revoked_at: row.get(6)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Records that an agent was just seen.
///
/// # Errors
/// [`Error::NotFound`] if the agent is not in this org.
pub fn touch_agent(conn: &Connection, org: &OrgId, id: &AgentId) -> Result<()> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE agents SET last_seen_at = ?3 WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), now],
    )?;
    found(n)
}

/// Revokes an agent.
///
/// # Errors
/// [`Error::NotFound`] if the agent is not in this org.
pub fn revoke_agent(conn: &Connection, org: &OrgId, id: &AgentId) -> Result<()> {
    let now = now_utc(conn)?;
    let n = conn.execute(
        "UPDATE agents SET revoked_at = coalesce(revoked_at, ?3) WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str(), now],
    )?;
    found(n)
}

/// Stores an already encrypted secret for a node of this org.
///
/// # Errors
/// Constraint violation if the node is not in this org or the ciphertext is empty.
pub fn put_node_secret(
    conn: &Connection,
    org: &OrgId,
    node: &NodeId,
    purpose: &str,
    sealed: &SealedSecret,
) -> Result<SecretId> {
    let id = new_uuid(conn)?;
    conn.execute(
        "INSERT INTO node_secrets (id, org_id, node_id, purpose, ciphertext, key_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            id,
            org.as_str(),
            node.as_str(),
            purpose,
            sealed.ciphertext,
            sealed.key_id
        ],
    )?;
    Ok(SecretId::from_db(id))
}

/// Reads a sealed secret (still encrypted). Only the executor should call this.
///
/// # Errors
/// [`Error::NotFound`] if the secret is not in this org.
pub fn get_node_secret(conn: &Connection, org: &OrgId, id: &SecretId) -> Result<SealedSecret> {
    conn.query_row(
        "SELECT ciphertext, key_id FROM node_secrets WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
        |row| {
            Ok(SealedSecret {
                ciphertext: row.get(0)?,
                key_id: row.get(1)?,
            })
        },
    )
    .optional()?
    .ok_or(Error::NotFound)
}

/// Lists secret metadata for a node (never the ciphertext); the API only says "stored or not".
///
/// # Errors
/// Database failure.
pub fn list_node_secrets(
    conn: &Connection,
    org: &OrgId,
    node: &NodeId,
) -> Result<Vec<NodeSecretMeta>> {
    let mut stmt = conn.prepare(
        "SELECT id, node_id, purpose, key_id, created_at FROM node_secrets \
         WHERE org_id = ?1 AND node_id = ?2 ORDER BY created_at, id",
    )?;
    let rows = stmt.query_map(params![org.as_str(), node.as_str()], |row| {
        Ok(NodeSecretMeta {
            id: SecretId::from_db(row.get(0)?),
            node_id: NodeId::from_db(row.get(1)?),
            purpose: row.get(2)?,
            key_id: row.get(3)?,
            created_at: row.get(4)?,
        })
    })?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

/// Deletes a secret.
///
/// # Errors
/// [`Error::NotFound`] if the secret is not in this org.
pub fn delete_node_secret(conn: &Connection, org: &OrgId, id: &SecretId) -> Result<()> {
    let n = conn.execute(
        "DELETE FROM node_secrets WHERE org_id = ?1 AND id = ?2",
        params![org.as_str(), id.as_str()],
    )?;
    found(n)
}
