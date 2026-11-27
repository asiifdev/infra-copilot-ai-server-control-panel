//! Inventories, comparisons and findings (docs/design.md 3.3). Every function takes an
//! [`OrgId`] and filters on it; a row of another org is [`Error::NotFound`].
//!
//! This crate stores and returns JSON as text and never parses it: the inventory model lives in
//! the `inventory` crate, which `db` deliberately does not depend on.

use crate::error::{Error, Result};
use crate::ids::{ComparisonId, InventoryId, NodeId, OrgId};
use crate::{new_uuid, now_utc};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};

text_enum!(
    /// What an item describes (mirrors `inventory::Kind`).
    ItemKind {
        /// A listening port.
        Listener => "listener",
        /// A running process.
        Process => "process",
        /// A scheduled job.
        Cron => "cron",
        /// A configuration reference to a host and port.
        EnvRef => "env_ref",
        /// A web virtual host.
        Vhost => "vhost",
        /// TLS certificate metadata.
        Cert => "cert",
        /// Database metadata.
        Database => "database",
        /// Public DNS check.
        Dns => "dns",
        /// Functional check result.
        Check => "check",
    }
);

text_enum!(
    /// How an item was discovered (mirrors `inventory::SeenVia`).
    ItemSource {
        /// Process filesystem.
        Proc => "proc",
        /// systemd.
        Systemd => "systemd",
        /// Container runtime.
        Docker => "docker",
        /// Configuration file.
        Config => "config",
    }
);

text_enum!(
    /// Whether every collector could read its source.
    InventoryStatus {
        /// Everything was read.
        Complete => "complete",
        /// Something could not be read; see the warnings.
        Partial => "partial",
    }
);

text_enum!(
    /// Weight of a finding, lowest first (mirrors `inventory::Severity`).
    FindingSeverity {
        /// Informational.
        Info => "info",
        /// Handle before cutover.
        Important => "important",
        /// Blocks cutover.
        Critical => "critical",
    }
);

text_enum!(
    /// What was done about a finding.
    FindingStatus {
        /// Not looked at yet.
        New => "new",
        /// Accepted as is.
        Accepted => "accepted",
        /// Fixed.
        Fixed => "fixed",
    }
);

/// One item to store.
#[derive(Debug)]
pub struct NewInventoryItem<'a> {
    /// Kind.
    pub kind: ItemKind,
    /// Stable key, unique per kind inside the inventory.
    pub key: &'a str,
    /// JSON object text, no secret values.
    pub data_json: &'a str,
    /// How it was found.
    pub seen_via: ItemSource,
}

/// An inventory to store.
#[derive(Debug)]
pub struct NewInventory<'a> {
    /// The machine it describes.
    pub node: &'a NodeId,
    /// When the snapshot was taken (ISO-8601 UTC).
    pub taken_at: &'a str,
    /// Collector set version.
    pub collector_version: &'a str,
    /// Complete or partial.
    pub status: InventoryStatus,
    /// Platform OS.
    pub os: &'a str,
    /// Platform architecture.
    pub arch: &'a str,
    /// JSON array text of warnings.
    pub warnings_json: &'a str,
}

/// A stored inventory (without its items).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRow {
    /// Id.
    pub id: InventoryId,
    /// Node.
    pub node_id: NodeId,
    /// When it was taken.
    pub taken_at: String,
    /// Collector set version.
    pub collector_version: String,
    /// Complete or partial.
    pub status: InventoryStatus,
    /// Platform OS.
    pub os: String,
    /// Platform architecture.
    pub arch: String,
    /// JSON array text of warnings.
    pub warnings_json: String,
}

/// A stored item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ItemRow {
    /// Kind.
    pub kind: ItemKind,
    /// Key.
    pub key: String,
    /// JSON object text.
    pub data_json: String,
    /// How it was found.
    pub seen_via: ItemSource,
}

/// A finding to store.
#[derive(Debug)]
pub struct NewFinding<'a> {
    /// Rule id, e.g. `PAR-001`.
    pub rule_id: &'a str,
    /// Weight.
    pub severity: FindingSeverity,
    /// Kind of the item it is about.
    pub subject_kind: ItemKind,
    /// Key of the item it is about.
    pub subject_key: &'a str,
    /// JSON array text of supporting `{kind, key}` references.
    pub evidence_json: &'a str,
    /// JSON object text of message arguments (names and ports only).
    pub args_json: &'a str,
}

/// A comparison header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComparisonRow {
    /// Id.
    pub id: ComparisonId,
    /// The inventory that is the reference.
    pub source_inventory_id: InventoryId,
    /// The inventory that is checked against it.
    pub target_inventory_id: InventoryId,
    /// When it ran.
    pub created_at: String,
}

/// A stored finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FindingRow {
    /// Position inside the comparison.
    pub seq: u32,
    /// Rule id.
    pub rule_id: String,
    /// Weight.
    pub severity: FindingSeverity,
    /// Kind of the item it is about.
    pub subject_kind: ItemKind,
    /// Key of the item it is about.
    pub subject_key: String,
    /// JSON array text.
    pub evidence_json: String,
    /// JSON object text.
    pub args_json: String,
    /// Handling state.
    pub status: FindingStatus,
}

/// Stores an inventory and its items atomically.
///
/// # Errors
/// [`Error::NotFound`] if the node is not in this org; a constraint violation for duplicate
/// item keys or malformed JSON.
pub fn save_inventory(
    conn: &mut Connection,
    org: &OrgId,
    new: &NewInventory<'_>,
    items: &[NewInventoryItem<'_>],
) -> Result<InventoryId> {
    let id = new_uuid(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let known: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM nodes WHERE org_id = ?1 AND id = ?2)",
        params![org.as_str(), new.node.as_str()],
        |row| row.get(0),
    )?;
    if !known {
        return Err(Error::NotFound);
    }
    tx.execute(
        "INSERT INTO inventories (id, org_id, node_id, taken_at, collector_version, status, os, arch, warnings) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            id,
            org.as_str(),
            new.node.as_str(),
            new.taken_at,
            new.collector_version,
            new.status.as_str(),
            new.os,
            new.arch,
            new.warnings_json
        ],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO inventory_items (org_id, inventory_id, kind, key, data, seen_via) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )?;
        for item in items {
            stmt.execute(params![
                org.as_str(),
                id,
                item.kind.as_str(),
                item.key,
                item.data_json,
                item.seen_via.as_str()
            ])?;
        }
    }
    tx.commit()?;
    Ok(InventoryId::from_db(id))
}

type RawInventory = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
);

const INVENTORY_COLUMNS: &str =
    "id, node_id, taken_at, collector_version, status, os, arch, warnings";

fn map_inventory(row: &rusqlite::Row<'_>) -> rusqlite::Result<RawInventory> {
    Ok((
        row.get(0)?,
        row.get(1)?,
        row.get(2)?,
        row.get(3)?,
        row.get(4)?,
        row.get(5)?,
        row.get(6)?,
        row.get(7)?,
    ))
}

fn build_inventory(t: RawInventory) -> Result<InventoryRow> {
    Ok(InventoryRow {
        id: InventoryId::from_db(t.0),
        node_id: NodeId::from_db(t.1),
        taken_at: t.2,
        collector_version: t.3,
        status: InventoryStatus::parse(&t.4)?,
        os: t.5,
        arch: t.6,
        warnings_json: t.7,
    })
}

/// The most recent inventory of a node.
///
/// # Errors
/// [`Error::NotFound`] if the node has none or is not in this org.
pub fn latest_inventory(conn: &Connection, org: &OrgId, node: &NodeId) -> Result<InventoryRow> {
    let row = conn
        .query_row(
            &format!(
                "SELECT {INVENTORY_COLUMNS} FROM inventories WHERE org_id = ?1 AND node_id = ?2 \
                 ORDER BY taken_at DESC, created_at DESC, id DESC LIMIT 1"
            ),
            params![org.as_str(), node.as_str()],
            map_inventory,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    build_inventory(row)
}

/// One inventory by id.
///
/// # Errors
/// [`Error::NotFound`] if it is missing or belongs to another org.
pub fn get_inventory(conn: &Connection, org: &OrgId, id: &InventoryId) -> Result<InventoryRow> {
    let row = conn
        .query_row(
            &format!("SELECT {INVENTORY_COLUMNS} FROM inventories WHERE org_id = ?1 AND id = ?2"),
            params![org.as_str(), id.as_str()],
            map_inventory,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    build_inventory(row)
}

/// Items of an inventory, optionally of one kind, sorted by kind then key, at most `limit`.
///
/// An inventory of another org yields an empty list (the org filter is on the item rows too).
///
/// # Errors
/// Database failure.
pub fn list_inventory_items(
    conn: &Connection,
    org: &OrgId,
    inventory: &InventoryId,
    kind: Option<ItemKind>,
    limit: u32,
) -> Result<Vec<ItemRow>> {
    let mut stmt = conn.prepare(
        "SELECT kind, key, data, seen_via FROM inventory_items \
         WHERE org_id = ?1 AND inventory_id = ?2 AND (?3 IS NULL OR kind = ?3) \
         ORDER BY kind, key LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![
            org.as_str(),
            inventory.as_str(),
            kind.map(ItemKind::as_str),
            limit
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        },
    )?;
    rows.map(|r| {
        let (kind, key, data_json, via) = r?;
        Ok(ItemRow {
            kind: ItemKind::parse(&kind)?,
            key,
            data_json,
            seen_via: ItemSource::parse(&via)?,
        })
    })
    .collect()
}

/// How many items of each kind an inventory holds, kinds in name order.
///
/// # Errors
/// Database failure.
pub fn count_inventory_items(
    conn: &Connection,
    org: &OrgId,
    inventory: &InventoryId,
) -> Result<Vec<(ItemKind, u64)>> {
    let mut stmt = conn.prepare(
        "SELECT kind, COUNT(*) FROM inventory_items WHERE org_id = ?1 AND inventory_id = ?2 \
         GROUP BY kind ORDER BY kind",
    )?;
    let rows = stmt.query_map(params![org.as_str(), inventory.as_str()], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;
    rows.map(|r| {
        let (kind, count) = r?;
        Ok((ItemKind::parse(&kind)?, u64::try_from(count).unwrap_or(0)))
    })
    .collect()
}

/// Stores a comparison and its findings atomically. Both inventories must be in this org.
///
/// # Errors
/// [`Error::NotFound`] if either inventory is not in this org; a constraint violation for
/// malformed JSON.
pub fn save_comparison(
    conn: &mut Connection,
    org: &OrgId,
    source: &InventoryId,
    target: &InventoryId,
    findings: &[NewFinding<'_>],
) -> Result<ComparisonId> {
    let id = new_uuid(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    for inventory in [source, target] {
        let known: bool = tx.query_row(
            "SELECT EXISTS (SELECT 1 FROM inventories WHERE org_id = ?1 AND id = ?2)",
            params![org.as_str(), inventory.as_str()],
            |row| row.get(0),
        )?;
        if !known {
            return Err(Error::NotFound);
        }
    }
    let created_at = now_utc(&tx)?;
    tx.execute(
        "INSERT INTO comparisons (id, org_id, source_inventory_id, target_inventory_id, created_at) \
         VALUES (?1, ?2, ?3, ?4, ?5)",
        params![id, org.as_str(), source.as_str(), target.as_str(), created_at],
    )?;
    {
        let mut stmt = tx.prepare(
            "INSERT INTO findings (org_id, comparison_id, seq, rule_id, severity, subject_kind, \
             subject_key, evidence, args) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )?;
        for (seq, f) in findings.iter().enumerate() {
            let seq = i64::try_from(seq).map_err(|_| Error::Invalid("finding count"))?;
            stmt.execute(params![
                org.as_str(),
                id,
                seq,
                f.rule_id,
                f.severity.as_str(),
                f.subject_kind.as_str(),
                f.subject_key,
                f.evidence_json,
                f.args_json
            ])?;
        }
    }
    tx.commit()?;
    Ok(ComparisonId::from_db(id))
}

/// The most recent comparison of the org.
///
/// # Errors
/// [`Error::NotFound`] if the org has none.
pub fn latest_comparison(conn: &Connection, org: &OrgId) -> Result<ComparisonRow> {
    let (id, source, target, created_at): (String, String, String, String) = conn
        .query_row(
            "SELECT id, source_inventory_id, target_inventory_id, created_at FROM comparisons \
             WHERE org_id = ?1 ORDER BY created_at DESC, id DESC LIMIT 1",
            params![org.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(ComparisonRow {
        id: ComparisonId::from_db(id),
        source_inventory_id: InventoryId::from_db(source),
        target_inventory_id: InventoryId::from_db(target),
        created_at,
    })
}

/// Findings of a comparison, most severe first, at most `limit`, optionally only those at or
/// above `min`. A comparison of another org yields an empty list.
///
/// # Errors
/// Database failure.
pub fn list_findings(
    conn: &Connection,
    org: &OrgId,
    comparison: &ComparisonId,
    min: Option<FindingSeverity>,
    limit: u32,
) -> Result<Vec<FindingRow>> {
    let min_rank = min.map_or(0_i64, severity_rank);
    let mut stmt = conn.prepare(
        "SELECT seq, rule_id, severity, subject_kind, subject_key, evidence, args, status \
         FROM findings WHERE org_id = ?1 AND comparison_id = ?2 \
         AND (CASE severity WHEN 'critical' THEN 2 WHEN 'important' THEN 1 ELSE 0 END) >= ?3 \
         ORDER BY (CASE severity WHEN 'critical' THEN 2 WHEN 'important' THEN 1 ELSE 0 END) DESC, seq \
         LIMIT ?4",
    )?;
    let rows = stmt.query_map(
        params![org.as_str(), comparison.as_str(), min_rank, limit],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
            ))
        },
    )?;
    rows.map(|r| {
        let (seq, rule_id, severity, kind, key, evidence_json, args_json, status) = r?;
        Ok(FindingRow {
            seq: u32::try_from(seq).map_err(|_| Error::Invalid("finding seq"))?,
            rule_id,
            severity: FindingSeverity::parse(&severity)?,
            subject_kind: ItemKind::parse(&kind)?,
            subject_key: key,
            evidence_json,
            args_json,
            status: FindingStatus::parse(&status)?,
        })
    })
    .collect()
}

const fn severity_rank(severity: FindingSeverity) -> i64 {
    match severity {
        FindingSeverity::Info => 0,
        FindingSeverity::Important => 1,
        FindingSeverity::Critical => 2,
    }
}
