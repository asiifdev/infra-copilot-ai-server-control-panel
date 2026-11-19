//! Writing what collectors and parity rules produced into the database, so the read tools (and
//! later the CLI and server) see it. Kept apart from [`crate::ReadStore`]: the model's path has
//! no write method at all.

use crate::store::StoreError;
use db::{
    ComparisonId, Connection, FindingSeverity, InventoryId, InventoryStatus, ItemKind, ItemSource,
    NewFinding, NewInventory, NewInventoryItem, OrgId,
};
use inventory::{Finding, Inventory, Kind, SeenVia, Severity, Status};
use serde_json::Value;

/// Stores `inventory` as the newest snapshot of the node called `node`.
///
/// # Errors
/// [`StoreError::NotFound`] when the node does not exist in this org; [`StoreError::Unavailable`]
/// for any other failure.
pub fn persist_inventory(
    conn: &mut Connection,
    org: &OrgId,
    node: &str,
    inventory: &Inventory,
) -> Result<InventoryId, StoreError> {
    let node = db::list_nodes(conn, org)?
        .into_iter()
        .find(|n| n.name == node)
        .ok_or(StoreError::NotFound)?
        .id;
    let data: Vec<String> = inventory
        .items()
        .iter()
        .map(|i| Value::Object(i.data.clone()).to_string())
        .collect();
    let items: Vec<NewInventoryItem<'_>> = inventory
        .items()
        .iter()
        .zip(&data)
        .map(|(item, data_json)| NewInventoryItem {
            kind: kind_to_db(item.kind),
            key: &item.key,
            data_json,
            seen_via: via_to_db(item.seen_via),
        })
        .collect();
    let warnings =
        serde_json::to_string(&inventory.warnings).map_err(|_| StoreError::Unavailable)?;
    let new = NewInventory {
        node: &node,
        taken_at: &inventory.taken_at,
        collector_version: &inventory.collector_version,
        status: match inventory.status {
            Status::Complete => InventoryStatus::Complete,
            Status::Partial => InventoryStatus::Partial,
        },
        os: &inventory.platform.os,
        arch: &inventory.platform.arch,
        warnings_json: &warnings,
    };
    Ok(db::save_inventory(conn, org, &new, &items)?)
}

/// Stores the result of comparing two stored inventories.
///
/// # Errors
/// [`StoreError::NotFound`] when either inventory is not in this org.
pub fn persist_comparison(
    conn: &mut Connection,
    org: &OrgId,
    source: &InventoryId,
    target: &InventoryId,
    findings: &[Finding],
) -> Result<ComparisonId, StoreError> {
    let mut json = Vec::with_capacity(findings.len());
    for f in findings {
        let evidence = serde_json::to_string(&f.evidence).map_err(|_| StoreError::Unavailable)?;
        let args = serde_json::to_string(&f.args).map_err(|_| StoreError::Unavailable)?;
        json.push((evidence, args));
    }
    let rows: Vec<NewFinding<'_>> = findings
        .iter()
        .zip(&json)
        .map(|(f, (evidence_json, args_json))| NewFinding {
            rule_id: &f.rule_id,
            severity: match f.severity {
                Severity::Info => FindingSeverity::Info,
                Severity::Important => FindingSeverity::Important,
                Severity::Critical => FindingSeverity::Critical,
            },
            subject_kind: kind_to_db(f.subject.kind),
            subject_key: &f.subject.key,
            evidence_json,
            args_json,
        })
        .collect();
    Ok(db::save_comparison(conn, org, source, target, &rows)?)
}

const fn kind_to_db(kind: Kind) -> ItemKind {
    match kind {
        Kind::Listener => ItemKind::Listener,
        Kind::Process => ItemKind::Process,
        Kind::Cron => ItemKind::Cron,
        Kind::EnvRef => ItemKind::EnvRef,
    }
}

const fn via_to_db(via: SeenVia) -> ItemSource {
    match via {
        SeenVia::Proc => ItemSource::Proc,
        SeenVia::Systemd => ItemSource::Systemd,
        SeenVia::Docker => ItemSource::Docker,
        SeenVia::Config => ItemSource::Config,
    }
}
