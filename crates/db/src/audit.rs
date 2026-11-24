//! Append-only, hash-chained audit trail (docs/design.md 3.6).
//!
//! Each org has its own chain: entry `n` stores the hash of entry `n - 1` (`prev_hash`) and its own
//! `hash = SHA-256(domain tag || length-prefixed fields)`. SQLite triggers reject `UPDATE` and
//! `DELETE`; the chain additionally exposes tampering done behind the triggers' back (for example
//! with the file opened by another tool).
//!
//! Limits, stated honestly: the hash is unkeyed, so someone who can rewrite the whole file can
//! recompute the chain, and dropping the newest entries is invisible without an external anchor.
//! Anchoring (for example periodically sending the head hash elsewhere) is not part of M0.

use crate::error::Result;
use crate::ids::{AuditId, NodeId, OrgId};
use crate::sha256::sha256_hex;
use crate::{new_uuid, now_utc};
use core_domain::redact::redact;
use rusqlite::{Connection, Transaction, TransactionBehavior, params};

text_enum!(
    /// Who performed the audited action.
    ActorKind {
        /// A signed-in user.
        User => "user",
        /// A node agent.
        Agent => "agent",
        /// An API token.
        ApiToken => "api_token",
        /// The product itself.
        System => "system",
    }
);

text_enum!(
    /// Outcome of the audited action.
    AuditOutcome {
        /// It happened.
        Success => "success",
        /// It was attempted and failed.
        Failure => "failure",
        /// It was refused (policy, approval, role).
        Denied => "denied",
    }
);

/// `prev_hash` of the first entry of a chain.
const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// An entry to append.
#[derive(Debug)]
pub struct NewAuditEntry<'a> {
    /// Kind of actor.
    pub actor_kind: ActorKind,
    /// Actor identifier, if any.
    pub actor_id: Option<&'a str>,
    /// Stable action name, such as `node.create`.
    pub action: &'a str,
    /// Node concerned, if any (kept even after the node is deleted).
    pub node_id: Option<&'a NodeId>,
    /// JSON describing the subject. Secrets are redacted before storage.
    pub subject_json: &'a str,
    /// Outcome.
    pub result: AuditOutcome,
}

/// A stored entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// Entry id.
    pub id: AuditId,
    /// Position in the org's chain, starting at 1.
    pub seq: i64,
    /// Kind of actor.
    pub actor_kind: ActorKind,
    /// Actor identifier.
    pub actor_id: Option<String>,
    /// Action name.
    pub action: String,
    /// Node concerned.
    pub node_id: Option<String>,
    /// Stored (redacted) subject JSON.
    pub subject_json: String,
    /// Outcome.
    pub result: AuditOutcome,
    /// Time of the entry.
    pub created_at: String,
    /// Hash of the previous entry.
    pub prev_hash: String,
    /// Hash of this entry.
    pub hash: String,
}

/// Why verification failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainBreak {
    /// An entry is missing or out of order at this position.
    SequenceGap {
        /// The `seq` that was expected.
        expected: i64,
    },
    /// The entry does not point at the previous entry's hash.
    PrevHashMismatch,
    /// The stored hash does not match the entry's content.
    HashMismatch,
}

/// Result of [`verify_audit_chain`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChainStatus {
    /// Every entry checked out.
    Intact {
        /// How many entries were verified.
        entries: u64,
    },
    /// The chain is broken at `seq`.
    Broken {
        /// The entry at which verification failed.
        seq: i64,
        /// What was wrong.
        kind: ChainBreak,
    },
}

fn push_field(buf: &mut Vec<u8>, value: &str) {
    buf.extend_from_slice(&(value.len() as u64).to_le_bytes());
    buf.extend_from_slice(value.as_bytes());
}

#[allow(clippy::too_many_arguments)]
fn entry_hash(
    org: &str,
    seq: i64,
    actor_kind: &str,
    actor_id: Option<&str>,
    action: &str,
    node_id: Option<&str>,
    subject: &str,
    result: &str,
    created_at: &str,
    prev_hash: &str,
) -> String {
    let mut buf = b"infra-copilot/audit/v1".to_vec();
    push_field(&mut buf, org);
    push_field(&mut buf, &seq.to_string());
    push_field(&mut buf, actor_kind);
    // Distinguish absent from empty so the two never hash the same.
    for opt in [actor_id, node_id] {
        match opt {
            Some(v) => {
                buf.push(1);
                push_field(&mut buf, v);
            }
            None => buf.push(0),
        }
    }
    push_field(&mut buf, action);
    push_field(&mut buf, subject);
    push_field(&mut buf, result);
    push_field(&mut buf, created_at);
    push_field(&mut buf, prev_hash);
    sha256_hex(&buf)
}

/// Redacts secrets in the subject. The redactor is text based and not JSON aware, so it can leave
/// invalid JSON behind (for example an unquoted value under a sensitive key such as `token_count`).
/// If the input was valid JSON but the redacted text is not, the redacted text is kept as a JSON
/// string, so the entry is still written.
fn redacted_subject(conn: &Connection, subject_json: &str) -> Result<String> {
    let redacted = redact(subject_json).text;
    let input_valid: bool =
        conn.query_row("SELECT json_valid(?1)", params![subject_json], |r| r.get(0))?;
    let redacted_valid: bool =
        conn.query_row("SELECT json_valid(?1)", params![redacted], |r| r.get(0))?;
    // Invalid input is returned unchanged so the table's CHECK rejects it, as before.
    if redacted_valid || !input_valid {
        return Ok(redacted);
    }
    Ok(conn.query_row(
        "SELECT json_object('redacted_subject', ?1)",
        params![redacted],
        |r| r.get(0),
    )?)
}

/// Appends an entry to the org's chain. Runs in an immediate transaction so concurrent writers
/// cannot fork the chain (and `UNIQUE (org_id, seq)` backs that up).
///
/// # Errors
/// Constraint violation for an invalid actor, action or JSON; database failure.
pub fn append_audit(conn: &Connection, org: &OrgId, new: &NewAuditEntry<'_>) -> Result<AuditEntry> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let entry = append_audit_in_tx(&tx, org, new)?;
    tx.commit()?;
    Ok(entry)
}

/// The body of [`append_audit`] for a caller that already holds an `IMMEDIATE` transaction, so the
/// entry commits or rolls back together with the change it records.
pub(crate) fn append_audit_in_tx(
    tx: &Connection,
    org: &OrgId,
    new: &NewAuditEntry<'_>,
) -> Result<AuditEntry> {
    let last: Option<(i64, String)> = tx
        .query_row(
            "SELECT seq, hash FROM audit_log WHERE org_id = ?1 ORDER BY seq DESC LIMIT 1",
            params![org.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let (seq, prev_hash) = match last {
        Some((seq, hash)) => (seq + 1, hash),
        None => (1, GENESIS.to_owned()),
    };
    let id = new_uuid(tx)?;
    let created_at = now_utc(tx)?;
    let subject = redacted_subject(tx, new.subject_json)?;
    let node_id = new.node_id.map(NodeId::as_str);
    let hash = entry_hash(
        org.as_str(),
        seq,
        new.actor_kind.as_str(),
        new.actor_id,
        new.action,
        node_id,
        &subject,
        new.result.as_str(),
        &created_at,
        &prev_hash,
    );
    tx.execute(
        "INSERT INTO audit_log (id, org_id, seq, actor_kind, actor_id, action, node_id, subject, \
         result, created_at, prev_hash, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        params![
            id,
            org.as_str(),
            seq,
            new.actor_kind.as_str(),
            new.actor_id,
            new.action,
            node_id,
            subject,
            new.result.as_str(),
            created_at,
            prev_hash,
            hash
        ],
    )?;
    Ok(AuditEntry {
        id: AuditId::from_db(id),
        seq,
        actor_kind: new.actor_kind,
        actor_id: new.actor_id.map(str::to_owned),
        action: new.action.to_owned(),
        node_id: node_id.map(str::to_owned),
        subject_json: subject,
        result: new.result,
        created_at,
        prev_hash,
        hash,
    })
}

const ENTRY_COLUMNS: &str = "id, seq, actor_kind, actor_id, action, node_id, subject, result, \
                             created_at, prev_hash, hash";

fn read_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawEntry> {
    Ok(RawEntry {
        id: row.get(0)?,
        seq: row.get(1)?,
        actor_kind: row.get(2)?,
        actor_id: row.get(3)?,
        action: row.get(4)?,
        node_id: row.get(5)?,
        subject: row.get(6)?,
        result: row.get(7)?,
        created_at: row.get(8)?,
        prev_hash: row.get(9)?,
        hash: row.get(10)?,
    })
}

struct RawEntry {
    id: String,
    seq: i64,
    actor_kind: String,
    actor_id: Option<String>,
    action: String,
    node_id: Option<String>,
    subject: String,
    result: String,
    created_at: String,
    prev_hash: String,
    hash: String,
}

impl RawEntry {
    fn recomputed_hash(&self, org: &OrgId) -> String {
        entry_hash(
            org.as_str(),
            self.seq,
            &self.actor_kind,
            self.actor_id.as_deref(),
            &self.action,
            self.node_id.as_deref(),
            &self.subject,
            &self.result,
            &self.created_at,
            &self.prev_hash,
        )
    }

    fn into_entry(self) -> Result<AuditEntry> {
        Ok(AuditEntry {
            id: AuditId::from_db(self.id),
            seq: self.seq,
            actor_kind: ActorKind::parse(&self.actor_kind)?,
            actor_id: self.actor_id,
            action: self.action,
            node_id: self.node_id,
            subject_json: self.subject,
            result: AuditOutcome::parse(&self.result)?,
            created_at: self.created_at,
            prev_hash: self.prev_hash,
            hash: self.hash,
        })
    }
}

/// Lists the newest entries of an org, newest first.
///
/// # Errors
/// Database failure.
pub fn list_audit(conn: &Connection, org: &OrgId, limit: u32) -> Result<Vec<AuditEntry>> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM audit_log WHERE org_id = ?1 ORDER BY seq DESC LIMIT ?2"
    ))?;
    let rows = stmt.query_map(params![org.as_str(), i64::from(limit)], read_entry)?;
    rows.map(|r| r?.into_entry()).collect()
}

/// Recomputes the whole chain of an org and reports the first inconsistency.
///
/// # Errors
/// Database failure (a broken chain is reported as [`ChainStatus::Broken`], not as an error).
pub fn verify_audit_chain(conn: &Connection, org: &OrgId) -> Result<ChainStatus> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {ENTRY_COLUMNS} FROM audit_log WHERE org_id = ?1 ORDER BY seq ASC"
    ))?;
    let rows = stmt.query_map(params![org.as_str()], read_entry)?;
    let mut expected_prev = GENESIS.to_owned();
    let mut verified = 0_u64;
    for (expected_seq, row) in (1_i64..).zip(rows) {
        let entry = row?;
        let broken = |kind| {
            Ok(ChainStatus::Broken {
                seq: entry.seq,
                kind,
            })
        };
        if entry.seq != expected_seq {
            return broken(ChainBreak::SequenceGap {
                expected: expected_seq,
            });
        }
        if entry.prev_hash != expected_prev {
            return broken(ChainBreak::PrevHashMismatch);
        }
        if entry.recomputed_hash(org) != entry.hash {
            return broken(ChainBreak::HashMismatch);
        }
        expected_prev = entry.hash;
        verified += 1;
    }
    Ok(ChainStatus::Intact { entries: verified })
}
