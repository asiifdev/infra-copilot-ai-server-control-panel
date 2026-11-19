//! The migration spec (docs/design.md section 6): where data comes from, where it goes, how it
//! travels and how traffic moves. Closed enums and unknown fields are rejected, so a typo or a
//! value this build does not understand can never be read as something harmless.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

use crate::error::PlanError;
use crate::exclude::validate_pattern;

/// Largest accepted spec or plan JSON document, in bytes.
pub const MAX_JSON_BYTES: usize = 256 * 1024;
/// Most databases a spec may list.
pub const MAX_DATABASES: usize = 256;
/// Most exclusion entries a spec may list.
pub const MAX_EXCLUDES: usize = 128;
/// Longest node or transport id, in bytes.
const MAX_ID_LEN: usize = 64;
/// Longest database key, in bytes.
const MAX_DB_KEY_LEN: usize = 200;

/// Rejects blank, over-long or control-character text.
pub(crate) fn check_text(value: &str, field: &'static str, max: usize) -> Result<(), PlanError> {
    if value.trim().is_empty() || value.len() > max || value.chars().any(char::is_control) {
        return Err(PlanError::Invalid(field));
    }
    Ok(())
}

/// A reference to a node by id. The database layer checks that the node belongs to the org.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRef {
    /// Node id.
    pub node_id: String,
}

/// Where the backup is kept while it travels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupLocation {
    /// On the source machine.
    Source,
    /// On the person's own machine.
    Local,
    /// On an intermediary storage (a remote, a bucket).
    Intermediary,
}

/// How the data is copied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackupStrategy {
    /// A full snapshot first, then a delta shortly before cutover.
    #[serde(rename = "snapshot+delta")]
    SnapshotDelta,
}

/// The backup part of the spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Backup {
    /// Where the backup is kept.
    pub location: BackupLocation,
    /// How the data is copied.
    pub strategy: BackupStrategy,
    /// Rebuildable data left out. Always shown to the person (see [`crate::render_plan`]).
    pub exclude: Vec<String>,
}

/// Whether the backup is encrypted before it leaves the client.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Encryption {
    /// Encrypted on the client before upload.
    ClientSide,
    /// Not encrypted by this tool.
    None,
}

/// A transport, by id. The transport itself (rclone, rsync, ...) is configured elsewhere and its
/// secrets never appear in a plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportRef {
    /// Id of the configured transport.
    pub transport_id: String,
    /// Encryption of the data in transit and at rest on the intermediary.
    pub encryption: Encryption,
}

/// What happens to one database.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DbMode {
    /// Restore it on the target.
    Restore,
    /// Leave it alone.
    Skip,
    /// Restore it next to what exists, under a suffixed name.
    CopyAsSuffix,
}

/// One database entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseEntry {
    /// Stable key, e.g. `pg:5432:appdb`.
    pub key: String,
    /// What to do with it.
    pub mode: DbMode,
}

/// What to do when the target already holds data. There is exactly one policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConflictPolicy {
    /// A database that already holds data is never overwritten; the copy is kept under a suffix.
    NeverOverwrite,
}

/// The data part of the spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Data {
    /// Databases and what happens to each.
    pub databases: Vec<DatabaseEntry>,
    /// Conflict policy.
    pub conflict_policy: ConflictPolicy,
}

/// How traffic moves to the target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutoverMethod {
    /// Through a tunnel.
    Tunnel,
    /// By changing DNS records.
    Dns,
}

/// The cutover part of the spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cutover {
    /// Tunnel or DNS.
    pub method: CutoverMethod,
    /// Record the current DNS before changing it.
    pub dns_snapshot: bool,
    /// Keep a way back.
    pub rollback: bool,
}

/// Conditions before the old server may be shut down. Both are always required; the fields exist
/// so the stored document says so explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Gates {
    /// No critical finding may be open (an accepted one, with a reason, is not open).
    pub no_open_critical_findings: bool,
    /// The old server stays until the new one is verified.
    pub keep_source_until_verified: bool,
}

/// The whole spec.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanSpec {
    /// The machine data comes from.
    pub source: NodeRef,
    /// The machine data goes to.
    pub target: NodeRef,
    /// Backup settings.
    pub backup: Backup,
    /// Transport settings.
    pub transport: TransportRef,
    /// Data settings.
    pub data: Data,
    /// Cutover settings.
    pub cutover: Cutover,
    /// Gates.
    pub gates: Gates,
}

impl PlanSpec {
    /// Parses and validates a spec from JSON.
    ///
    /// # Errors
    /// [`PlanError::TooLarge`] for a document over [`MAX_JSON_BYTES`], [`PlanError::Parse`] for
    /// unknown fields, wrong types or unknown enum values, and any error of [`PlanSpec::validate`].
    pub fn from_json(json: &str) -> Result<Self, PlanError> {
        if json.len() > MAX_JSON_BYTES {
            return Err(PlanError::TooLarge {
                what: "spec document",
                max: MAX_JSON_BYTES,
            });
        }
        let spec: Self =
            serde_json::from_str(json).map_err(|err| PlanError::Parse(err.to_string()))?;
        spec.validate()?;
        Ok(spec)
    }

    /// Serializes to compact JSON.
    ///
    /// # Errors
    /// Only if serialization fails, which cannot happen for these types.
    pub fn to_json(&self) -> Result<String, PlanError> {
        serde_json::to_string(self).map_err(|err| PlanError::Parse(err.to_string()))
    }

    /// Checks everything a parser cannot: bounds, duplicates and the safety rules of the plan.
    ///
    /// # Errors
    /// The first rule that is broken.
    pub fn validate(&self) -> Result<(), PlanError> {
        check_text(&self.source.node_id, "source.node_id", MAX_ID_LEN)?;
        check_text(&self.target.node_id, "target.node_id", MAX_ID_LEN)?;
        if self.source.node_id == self.target.node_id {
            return Err(PlanError::SameNode);
        }
        check_text(
            &self.transport.transport_id,
            "transport.transport_id",
            MAX_ID_LEN,
        )?;

        if self.backup.exclude.len() > MAX_EXCLUDES {
            return Err(PlanError::TooLarge {
                what: "backup.exclude",
                max: MAX_EXCLUDES,
            });
        }
        let mut seen = BTreeSet::new();
        for pattern in &self.backup.exclude {
            validate_pattern(pattern)?;
            if !seen.insert(pattern.trim()) {
                return Err(PlanError::Invalid("backup.exclude"));
            }
        }

        if self.data.databases.len() > MAX_DATABASES {
            return Err(PlanError::TooLarge {
                what: "data.databases",
                max: MAX_DATABASES,
            });
        }
        let mut keys = BTreeSet::new();
        for db in &self.data.databases {
            check_text(&db.key, "data.databases.key", MAX_DB_KEY_LEN)?;
            if !keys.insert(db.key.as_str()) {
                return Err(PlanError::DuplicateDatabase);
            }
        }

        if self.cutover.method == CutoverMethod::Dns
            && !(self.cutover.dns_snapshot && self.cutover.rollback)
        {
            return Err(PlanError::UnsafeDnsCutover);
        }
        if self.backup.location == BackupLocation::Intermediary
            && self.transport.encryption != Encryption::ClientSide
        {
            return Err(PlanError::IntermediaryNeedsEncryption);
        }
        if !self.gates.no_open_critical_findings {
            return Err(PlanError::GateDisabled("no_open_critical_findings"));
        }
        if !self.gates.keep_source_until_verified {
            return Err(PlanError::GateDisabled("keep_source_until_verified"));
        }
        Ok(())
    }

    /// The mode of a database listed in the spec.
    #[must_use]
    pub fn database_mode(&self, key: &str) -> Option<DbMode> {
        self.data
            .databases
            .iter()
            .find(|db| db.key == key)
            .map(|db| db.mode)
    }
}
