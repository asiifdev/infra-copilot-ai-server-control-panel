//! What a tool may read, and the database-backed implementation.
//!
//! The trait is the seam: tools never see a connection or SQL, and every method takes the
//! [`OrgId`] explicitly, so a read that forgets the tenant does not compile.

use db::{
    ComparisonRow, Connection, Error as DbError, FindingSeverity, InventoryId, ItemKind, NodeId,
    OrgId,
};
use inventory::{Finding, Inventory, Item, Kind, Platform, SeenVia, Severity, Status, SubjectRef};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::fmt;

/// Why a read failed. Carries no database text: that could name tables or paths.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// Missing, or belongs to another org (indistinguishable on purpose).
    NotFound,
    /// The store could not answer.
    Unavailable,
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "not_found",
            Self::Unavailable => "store_unavailable",
        })
    }
}

impl std::error::Error for StoreError {}

impl From<DbError> for StoreError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::NotFound => Self::NotFound,
            _ => Self::Unavailable,
        }
    }
}

/// A node as tools see it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeInfo {
    /// Unique name inside the org.
    pub name: String,
    /// `vps` or `local`.
    pub kind: &'static str,
    /// `pending`, `online`, `offline` or `revoked`.
    pub status: &'static str,
}

/// The findings of the latest comparison.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindingsView {
    /// Name of the reference node.
    pub source_node: String,
    /// Name of the checked node.
    pub target_node: String,
    /// When the comparison ran.
    pub created_at: String,
    /// Findings, most severe first.
    pub findings: Vec<Finding>,
}

/// Read access for tools. Read-only by construction: there is no write method.
pub trait ReadStore {
    /// Nodes of the org, by name.
    ///
    /// # Errors
    /// [`StoreError::Unavailable`] when the store cannot answer.
    fn nodes(&self, org: &OrgId) -> Result<Vec<NodeInfo>, StoreError>;

    /// The newest inventory of the node called `node`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when the node or its inventory does not exist in this org.
    fn latest_inventory(&self, org: &OrgId, node: &str) -> Result<Inventory, StoreError>;

    /// Findings of the newest comparison, at or above `min`, at most `limit`.
    ///
    /// # Errors
    /// [`StoreError::NotFound`] when the org has no comparison yet.
    fn latest_findings(
        &self,
        org: &OrgId,
        min: Option<Severity>,
        limit: u32,
    ) -> Result<FindingsView, StoreError>;
}

/// [`ReadStore`] over the SQLite layer.
pub struct DbStore<'a> {
    conn: &'a Connection,
}

impl<'a> DbStore<'a> {
    /// Reads through `conn` (which must come from `db::open`).
    #[must_use]
    pub fn new(conn: &'a Connection) -> Self {
        Self { conn }
    }

    fn node_id(&self, org: &OrgId, name: &str) -> Result<NodeId, StoreError> {
        Ok(db::list_nodes(self.conn, org)?
            .into_iter()
            .find(|n| n.name == name)
            .ok_or(StoreError::NotFound)?
            .id)
    }

    fn node_name(&self, org: &OrgId, inventory: &InventoryId) -> Result<String, StoreError> {
        let row = db::get_inventory(self.conn, org, inventory)?;
        Ok(db::list_nodes(self.conn, org)?
            .into_iter()
            .find(|n| n.id == row.node_id)
            .ok_or(StoreError::NotFound)?
            .name)
    }
}

/// Upper bound on items read back for one inventory (a machine with more is not a node).
const MAX_ITEMS: u32 = 100_000;

impl ReadStore for DbStore<'_> {
    fn nodes(&self, org: &OrgId) -> Result<Vec<NodeInfo>, StoreError> {
        Ok(db::list_nodes(self.conn, org)?
            .into_iter()
            .map(|n| NodeInfo {
                name: n.name,
                kind: n.kind.as_str(),
                status: n.status.as_str(),
            })
            .collect())
    }

    fn latest_inventory(&self, org: &OrgId, node: &str) -> Result<Inventory, StoreError> {
        let node = self.node_id(org, node)?;
        let row = db::latest_inventory(self.conn, org, &node)?;
        let rows = db::list_inventory_items(self.conn, org, &row.id, None, MAX_ITEMS)?;
        let mut items = Vec::with_capacity(rows.len());
        for r in rows {
            let data: Map<String, Value> =
                serde_json::from_str(&r.data_json).map_err(|_| StoreError::Unavailable)?;
            items.push(Item {
                kind: kind_from_db(r.kind),
                key: r.key,
                data,
                seen_via: via_from_db(r.seen_via),
            });
        }
        let mut inventory = Inventory::new(
            row.taken_at,
            row.collector_version,
            Platform {
                os: row.os,
                arch: row.arch,
            },
            items,
        )
        .map_err(|_| StoreError::Unavailable)?;
        inventory.status = match row.status {
            db::InventoryStatus::Complete => Status::Complete,
            db::InventoryStatus::Partial => Status::Partial,
        };
        inventory.warnings =
            serde_json::from_str(&row.warnings_json).map_err(|_| StoreError::Unavailable)?;
        Ok(inventory)
    }

    fn latest_findings(
        &self,
        org: &OrgId,
        min: Option<Severity>,
        limit: u32,
    ) -> Result<FindingsView, StoreError> {
        let ComparisonRow {
            id,
            source_inventory_id,
            target_inventory_id,
            created_at,
        } = db::latest_comparison(self.conn, org)?;
        let rows = db::list_findings(self.conn, org, &id, min.map(severity_to_db), limit)?;
        let mut findings = Vec::with_capacity(rows.len());
        for r in rows {
            let evidence: Vec<SubjectRef> =
                serde_json::from_str(&r.evidence_json).map_err(|_| StoreError::Unavailable)?;
            let args: BTreeMap<String, String> =
                serde_json::from_str(&r.args_json).map_err(|_| StoreError::Unavailable)?;
            findings.push(Finding {
                rule_id: r.rule_id,
                severity: severity_from_db(r.severity),
                subject: SubjectRef {
                    kind: kind_from_db(r.subject_kind),
                    key: r.subject_key,
                },
                evidence,
                args,
            });
        }
        Ok(FindingsView {
            source_node: self.node_name(org, &source_inventory_id)?,
            target_node: self.node_name(org, &target_inventory_id)?,
            created_at,
            findings,
        })
    }
}

const fn kind_from_db(kind: ItemKind) -> Kind {
    match kind {
        ItemKind::Listener => Kind::Listener,
        ItemKind::Process => Kind::Process,
        ItemKind::Cron => Kind::Cron,
        ItemKind::EnvRef => Kind::EnvRef,
    }
}

const fn via_from_db(via: db::ItemSource) -> SeenVia {
    match via {
        db::ItemSource::Proc => SeenVia::Proc,
        db::ItemSource::Systemd => SeenVia::Systemd,
        db::ItemSource::Docker => SeenVia::Docker,
        db::ItemSource::Config => SeenVia::Config,
    }
}

const fn severity_from_db(severity: FindingSeverity) -> Severity {
    match severity {
        FindingSeverity::Info => Severity::Info,
        FindingSeverity::Important => Severity::Important,
        FindingSeverity::Critical => Severity::Critical,
    }
}

const fn severity_to_db(severity: Severity) -> FindingSeverity {
    match severity {
        Severity::Info => FindingSeverity::Info,
        Severity::Important => FindingSeverity::Important,
        Severity::Critical => FindingSeverity::Critical,
    }
}
