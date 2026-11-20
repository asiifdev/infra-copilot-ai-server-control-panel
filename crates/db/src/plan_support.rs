//! What the AI planning tools need beyond the plain plan functions: a draft insert that checks the
//! per-org limits and the name inside one write transaction, a lookup by name, and the latest
//! comparison between two named machines. Every function filters on the org.

use crate::error::{Error, Result};
use crate::ids::{ComparisonId, InventoryId, NodeId, OrgId, PlanId};
use crate::new_uuid;
use crate::plans::{
    NewPlan, NewPlanStep, PLAN_COLUMNS, PlanRow, build_plan, check_new, insert_plan_row,
    insert_steps, map_plan,
};
use crate::snapshots::ComparisonRow;
use rusqlite::{Connection, OptionalExtension, Transaction, TransactionBehavior, params};
use std::fmt;

/// Why a guarded draft insert was refused.
#[derive(Debug)]
pub enum DraftError {
    /// The org already has a plan with this name (compared without regard to ASCII case).
    NameTaken,
    /// The org already holds the maximum number of drafts.
    TooManyDrafts,
    /// Any other failure (a node outside the org is [`Error::NotFound`]).
    Db(Error),
}

impl fmt::Display for DraftError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NameTaken => f.write_str("plan name already taken"),
            Self::TooManyDrafts => f.write_str("too many draft plans"),
            Self::Db(err) => err.fmt(f),
        }
    }
}

impl std::error::Error for DraftError {}

impl From<Error> for DraftError {
    fn from(err: Error) -> Self {
        Self::Db(err)
    }
}

impl From<rusqlite::Error> for DraftError {
    fn from(err: rusqlite::Error) -> Self {
        Self::Db(err.into())
    }
}

/// Stores a new **draft** plan with its steps, after checking in the same write transaction that
/// the name is free and the org holds fewer than `max_drafts` drafts. The transaction takes the
/// write lock before it reads (`BEGIN IMMEDIATE`), so two callers on two connections cannot both
/// pass the check. Works through a shared connection reference because the assistant holds one.
///
/// # Errors
/// [`DraftError::NameTaken`], [`DraftError::TooManyDrafts`], or [`DraftError::Db`] (a node that is
/// not in this org is `NotFound`; the same node twice is `Invalid`).
pub fn create_draft_plan(
    conn: &Connection,
    org: &OrgId,
    new: &NewPlan<'_>,
    steps: &[NewPlanStep<'_>],
    max_drafts: u32,
) -> std::result::Result<PlanId, DraftError> {
    let id = new_uuid(conn)?;
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let taken: bool = tx.query_row(
        "SELECT EXISTS (SELECT 1 FROM migration_plans WHERE org_id = ?1 AND lower(name) = lower(?2))",
        params![org.as_str(), new.name],
        |row| row.get(0),
    )?;
    if taken {
        return Err(DraftError::NameTaken);
    }
    let drafts: i64 = tx.query_row(
        "SELECT COUNT(*) FROM migration_plans WHERE org_id = ?1 AND status = 'draft'",
        params![org.as_str()],
        |row| row.get(0),
    )?;
    if drafts >= i64::from(max_drafts) {
        return Err(DraftError::TooManyDrafts);
    }
    check_new(&tx, org, new)?;
    insert_plan_row(&tx, org, &id, new)?;
    insert_steps(&tx, org, &id, steps)?;
    tx.commit()?;
    Ok(PlanId::from_db(id))
}

/// The plan called `name` in this org (ASCII case ignored, like the name check on insert).
///
/// # Errors
/// [`Error::NotFound`] if there is none, or it belongs to another org.
pub fn find_plan_by_name(conn: &Connection, org: &OrgId, name: &str) -> Result<PlanRow> {
    let raw = conn
        .query_row(
            &format!(
                "SELECT {PLAN_COLUMNS} FROM migration_plans \
                 WHERE org_id = ?1 AND lower(name) = lower(?2) ORDER BY created_at, id LIMIT 1"
            ),
            params![org.as_str(), name],
            map_plan,
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    build_plan(raw)
}

/// The most recent comparison whose reference inventory belongs to `source` and whose checked
/// inventory belongs to `target`.
///
/// # Errors
/// [`Error::NotFound`] if no such comparison exists in this org.
pub fn latest_comparison_between(
    conn: &Connection,
    org: &OrgId,
    source: &NodeId,
    target: &NodeId,
) -> Result<ComparisonRow> {
    let (id, source_inventory, target_inventory, created_at): (String, String, String, String) =
        conn.query_row(
            "SELECT c.id, c.source_inventory_id, c.target_inventory_id, c.created_at \
             FROM comparisons c \
             JOIN inventories si ON si.org_id = c.org_id AND si.id = c.source_inventory_id \
             JOIN inventories ti ON ti.org_id = c.org_id AND ti.id = c.target_inventory_id \
             WHERE c.org_id = ?1 AND si.node_id = ?2 AND ti.node_id = ?3 \
             ORDER BY c.created_at DESC, c.id DESC LIMIT 1",
            params![org.as_str(), source.as_str(), target.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .optional()?
        .ok_or(Error::NotFound)?;
    Ok(ComparisonRow {
        id: ComparisonId::from_db(id),
        source_inventory_id: InventoryId::from_db(source_inventory),
        target_inventory_id: InventoryId::from_db(target_inventory),
        created_at,
    })
}
