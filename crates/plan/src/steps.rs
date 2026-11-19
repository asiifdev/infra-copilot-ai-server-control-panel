//! The steps of a migration plan and the order they must keep.

use core_domain::tier::Tier;
use serde::{Deserialize, Serialize};

use crate::error::PlanError;

/// What a step does. Stable wire names (they are stored in `migration_steps.kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    /// Read what runs on a machine.
    Inventory,
    /// Take the snapshot of the data.
    Backup,
    /// Move the snapshot to the target.
    Transfer,
    /// Put the data back on the target.
    Restore,
    /// Check that the target works and matches.
    Verify,
    /// Copy what changed since the snapshot, shortly before cutover.
    Delta,
    /// Move traffic to the target.
    Cutover,
    /// Check again after traffic moved.
    PostVerify,
    /// Shut the old server down. Always tier 4.
    DecommissionSource,
}

impl StepKind {
    /// Every kind, in the order a plan runs them.
    pub const ALL: [StepKind; 9] = [
        Self::Inventory,
        Self::Backup,
        Self::Transfer,
        Self::Restore,
        Self::Verify,
        Self::Delta,
        Self::Cutover,
        Self::PostVerify,
        Self::DecommissionSource,
    ];

    /// The wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Inventory => "inventory",
            Self::Backup => "backup",
            Self::Transfer => "transfer",
            Self::Restore => "restore",
            Self::Verify => "verify",
            Self::Delta => "delta",
            Self::Cutover => "cutover",
            Self::PostVerify => "post_verify",
            Self::DecommissionSource => "decommission_source",
        }
    }

    /// Parses a wire name.
    ///
    /// # Errors
    /// [`PlanError::UnknownKind`] for anything else.
    pub fn parse(text: &str) -> Result<Self, PlanError> {
        Self::ALL
            .into_iter()
            .find(|kind| kind.as_str() == text)
            .ok_or(PlanError::UnknownKind)
    }

    /// Position in the required order. A step may only depend on steps of a lower or equal rank,
    /// and every step must (transitively) depend on every step of a lower rank.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Inventory => 0,
            Self::Backup => 1,
            Self::Transfer => 2,
            Self::Restore => 3,
            Self::Verify => 4,
            Self::Delta => 5,
            Self::Cutover => 6,
            Self::PostVerify => 7,
            Self::DecommissionSource => 8,
        }
    }

    /// Risk tier. Decided here, in code; a plan document cannot lower it.
    #[must_use]
    pub const fn tier(self) -> Tier {
        match self {
            Self::Inventory | Self::Verify | Self::PostVerify => Tier::Read,
            Self::Backup | Self::Transfer => Tier::LightChange,
            Self::Restore | Self::Delta | Self::Cutover => Tier::RiskyChange,
            Self::DecommissionSource => Tier::Destructive,
        }
    }

    /// Whether the plan must contain at least one step of this kind.
    #[must_use]
    pub const fn required(self) -> bool {
        !matches!(self, Self::Delta | Self::DecommissionSource)
    }

    /// Whether the plan may contain at most one step of this kind.
    #[must_use]
    pub const fn single(self) -> bool {
        matches!(self, Self::Cutover | Self::DecommissionSource)
    }
}

/// Which machine an inventory step reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The machine data comes from.
    Source,
    /// The machine data goes to.
    Target,
}

/// One step. `seq` is its number inside the plan; `depends_on` lists the steps that must be done
/// first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    /// Number of the step, unique inside the plan.
    pub seq: u32,
    /// What it does.
    pub kind: StepKind,
    /// Steps that must finish first.
    pub depends_on: Vec<u32>,
    /// Inventory steps only: which machine they read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub side: Option<Side>,
    /// Restore and verify steps only: the database key from the spec they act on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
}

impl Step {
    /// A step without side or database.
    #[must_use]
    pub fn new(seq: u32, kind: StepKind, depends_on: Vec<u32>) -> Self {
        Self {
            seq,
            kind,
            depends_on,
            side: None,
            database: None,
        }
    }

    /// Sets the side (inventory steps).
    #[must_use]
    pub fn on(mut self, side: Side) -> Self {
        self.side = Some(side);
        self
    }

    /// Sets the database key (restore and verify steps).
    #[must_use]
    pub fn for_database(mut self, key: &str) -> Self {
        self.database = Some(key.to_owned());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip_and_unknown_is_refused() {
        for kind in StepKind::ALL {
            assert_eq!(StepKind::parse(kind.as_str()), Ok(kind));
        }
        assert_eq!(StepKind::parse("rm_rf"), Err(PlanError::UnknownKind));
        assert_eq!(StepKind::parse("Backup"), Err(PlanError::UnknownKind));
    }

    #[test]
    fn ranks_follow_the_all_order_strictly() {
        for pair in StepKind::ALL.windows(2) {
            assert!(pair[0].rank() < pair[1].rank());
        }
    }

    #[test]
    fn only_decommission_is_destructive_and_it_is_tier_four() {
        for kind in StepKind::ALL {
            let destructive = kind.tier() == Tier::Destructive;
            assert_eq!(destructive, kind == StepKind::DecommissionSource);
        }
        assert!(
            StepKind::DecommissionSource
                .tier()
                .requires_typed_confirmation()
        );
        assert!(!StepKind::Cutover.tier().approvable_from_chat_bot());
    }

    #[test]
    fn serde_names_match_as_str() {
        for kind in StepKind::ALL {
            let json = serde_json::to_string(&kind).unwrap_or_default();
            assert_eq!(json, format!("\"{}\"", kind.as_str()));
        }
    }
}
