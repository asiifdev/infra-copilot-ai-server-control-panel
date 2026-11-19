//! A dated, validated snapshot of one machine.

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

use crate::item::{Item, Kind};

/// Version of the inventory wire format. Bumped on any incompatible change.
pub const SCHEMA_VERSION: u32 = 1;

/// Longest accepted item key, to keep hostile or broken input bounded.
const MAX_KEY_LEN: usize = 512;

/// Whether every collector could read everything it needed (docs/design.md 3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Everything was read.
    Complete,
    /// Some sources could not be read; see `warnings`. Never treat a partial inventory as proof
    /// that something is absent.
    Partial,
}

/// Operating system and architecture of the machine the inventory describes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Platform {
    /// `linux` or `darwin`.
    pub os: String,
    /// `x86_64` or `arm64`.
    pub arch: String,
}

/// Why an inventory was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InventoryError {
    /// The JSON could not be parsed.
    Parse(String),
    /// `schema_version` is not the one this build understands.
    UnsupportedSchema(u32),
    /// Two items share a kind and key.
    DuplicateItem(Kind, String),
    /// An item key is empty, too long, or contains control characters.
    InvalidKey(String),
}

impl fmt::Display for InventoryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Parse(msg) => write!(f, "invalid inventory JSON: {msg}"),
            Self::UnsupportedSchema(v) => {
                write!(
                    f,
                    "unsupported inventory schema version {v} (expected {SCHEMA_VERSION})"
                )
            }
            Self::DuplicateItem(kind, key) => {
                write!(f, "duplicate {} item: {key}", kind.as_str())
            }
            Self::InvalidKey(key) => write!(f, "invalid item key: {key:?}"),
        }
    }
}

impl std::error::Error for InventoryError {}

#[derive(Deserialize)]
struct RawInventory {
    schema_version: u32,
    taken_at: String,
    collector_version: String,
    platform: Platform,
    status: Status,
    warnings: Vec<String>,
    items: Vec<Item>,
}

/// A snapshot of one machine. Items are sorted by `(kind, key)` and unique, so two snapshots can be
/// compared and diffed deterministically.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawInventory")]
pub struct Inventory {
    schema_version: u32,
    /// When the snapshot was taken (ISO-8601 UTC).
    pub taken_at: String,
    /// Version of the collector set that produced it.
    pub collector_version: String,
    /// The machine's platform.
    pub platform: Platform,
    /// Complete or partial.
    pub status: Status,
    /// What could not be read, when the snapshot is partial.
    pub warnings: Vec<String>,
    items: Vec<Item>,
}

impl TryFrom<RawInventory> for Inventory {
    type Error = InventoryError;

    fn try_from(raw: RawInventory) -> Result<Self, Self::Error> {
        if raw.schema_version != SCHEMA_VERSION {
            return Err(InventoryError::UnsupportedSchema(raw.schema_version));
        }
        let mut inventory =
            Self::new(raw.taken_at, raw.collector_version, raw.platform, raw.items)?;
        inventory.status = raw.status;
        inventory.warnings = raw.warnings;
        Ok(inventory)
    }
}

impl Inventory {
    /// Builds a snapshot, sorting the items and rejecting duplicates and malformed keys.
    ///
    /// # Errors
    /// [`InventoryError::DuplicateItem`] or [`InventoryError::InvalidKey`].
    pub fn new(
        taken_at: String,
        collector_version: String,
        platform: Platform,
        mut items: Vec<Item>,
    ) -> Result<Self, InventoryError> {
        let mut seen = BTreeSet::new();
        for item in &items {
            if item.key.is_empty()
                || item.key.len() > MAX_KEY_LEN
                || item.key.chars().any(char::is_control)
            {
                return Err(InventoryError::InvalidKey(item.key.clone()));
            }
            if !seen.insert((item.kind, item.key.clone())) {
                return Err(InventoryError::DuplicateItem(item.kind, item.key.clone()));
            }
        }
        items.sort_by(|a, b| (a.kind, &a.key).cmp(&(b.kind, &b.key)));
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            taken_at,
            collector_version,
            platform,
            status: Status::Complete,
            warnings: Vec::new(),
            items,
        })
    }

    /// Marks the snapshot partial and records why. An empty `warnings` list keeps it complete.
    #[must_use]
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        if !warnings.is_empty() {
            self.status = Status::Partial;
            self.warnings = warnings;
        }
        self
    }

    /// All items, sorted by kind then key.
    #[must_use]
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The items of one kind, in key order.
    pub fn items_of(&self, kind: Kind) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |item| item.kind == kind)
    }

    /// One item by kind and key.
    #[must_use]
    pub fn get(&self, kind: Kind, key: &str) -> Option<&Item> {
        self.items
            .binary_search_by(|item| (item.kind, item.key.as_str()).cmp(&(kind, key)))
            .ok()
            .and_then(|index| self.items.get(index))
    }

    /// Parses and validates an inventory from JSON.
    ///
    /// # Errors
    /// [`InventoryError`] for malformed JSON, an unsupported schema version, duplicates or bad keys.
    pub fn from_json(json: &str) -> Result<Self, InventoryError> {
        // Through `RawInventory` directly so validation errors keep their variant.
        let raw: RawInventory =
            serde_json::from_str(json).map_err(|err| InventoryError::Parse(err.to_string()))?;
        Self::try_from(raw)
    }

    /// Serializes to pretty JSON.
    ///
    /// # Errors
    /// Only if serialization fails, which cannot happen for these types.
    pub fn to_json(&self) -> Result<String, InventoryError> {
        serde_json::to_string_pretty(self).map_err(|err| InventoryError::Parse(err.to_string()))
    }
}
