//! Read-only Linux collectors behind the [`inventory::Collector`] contract (docs/design.md 4, 12.4).
//!
//! * [`Listeners`]: listening TCP ports (`/proc/net/tcp*`, owner from `/proc/<pid>/fd`).
//! * [`Processes`]: running processes (`/proc/<pid>/{comm,cmdline,cwd}`), merged by name.
//! * [`Cron`]: crontab files.
//!
//! External programs are run only through [`CommandRunner`] with a closed list of read-only [`Cmd`]s
//! (decision 0014); tests use [`FakeRunner`].
//!
//! Every collector reads through a [`RootFs`], which has no write methods, so production points it
//! at `/` and tests at a fixture tree. Collectors read files, never use `unsafe`, and never read process
//! environments. A missing or unreadable source becomes a warning in
//! [`inventory::Collection::warnings`] (the inventory then becomes `partial`), never a silent "nothing
//! there". Values that cannot be determined are `null`/`unknown`, never guessed. Command lines and
//! cron commands are redacted with [`core_domain::redact::redact`] before they enter an item.
//!
//! [`collect_inventory`] runs all three and builds the [`inventory::Inventory`]. The caller supplies
//! the timestamp and platform, so this crate has no clock dependency.
//!
//! Also: [`Systemd`] (running services and timers), [`Docker`] (running containers) and [`Pm2`]
//! (apps saved in `dump.pm2`, read from files only).
//!
//! Not covered yet: `env_ref`, databases, service versions, macOS.

mod cron;
mod docker;
mod exec;
mod listeners;
mod pm2;
mod processes;
mod procfs;
mod rootfs;
mod supervision;
mod systemd;
mod util;

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::sync::Arc;

use inventory::{CollectError, Collector, Inventory, Item, Kind, Platform};

pub use cron::Cron;
pub use docker::Docker;
pub use exec::{Cmd, CommandRunner, ExecError, FakeRunner, HostRunner};
pub use listeners::Listeners;
pub use pm2::Pm2;
pub use processes::Processes;
pub use rootfs::{DirNames, FileText, FsError, RootFs};
pub use systemd::Systemd;

/// Version of this collector set, recorded in the inventory.
pub const COLLECTOR_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs every collector against `root` and assembles the inventory, without running any external
/// program (Docker and systemd are skipped as "not available"). Use [`collect_inventory_with`] on
/// a real host.
///
/// # Errors
/// See [`collect_inventory_with`].
pub fn collect_inventory(
    root: &RootFs,
    taken_at: String,
    platform: Platform,
) -> Result<Inventory, CollectError> {
    collect_inventory_with(root, Arc::new(FakeRunner::new()), taken_at, platform)
}

/// Runs the listener, process, cron, PM2, systemd and Docker collectors and assembles the
/// inventory. Programs run only through `runner` (decision 0014).
///
/// Warnings from the collectors are prefixed with the collector kind and make the inventory
/// `partial`.
///
/// # Errors
/// [`CollectError`] when a collector cannot work at all (for example the root is not a directory)
/// or the assembled items are not a valid inventory.
pub fn collect_inventory_with(
    root: &RootFs,
    runner: Arc<dyn CommandRunner>,
    taken_at: String,
    platform: Platform,
) -> Result<Inventory, CollectError> {
    let collectors: [Box<dyn Collector>; 6] = [
        Box::new(Listeners::new(root.clone())),
        Box::new(Processes::new(root.clone())),
        Box::new(Cron::new(root.clone())),
        Box::new(Pm2::new(root.clone())),
        Box::new(Systemd::new(Arc::clone(&runner))),
        Box::new(Docker::new(runner)),
    ];
    let mut items = Vec::new();
    let mut warnings = Vec::new();
    for collector in &collectors {
        let collection = collector.run()?;
        items.extend(collection.items);
        warnings.extend(
            collection
                .warnings
                .into_iter()
                .map(|warning| format!("{}: {warning}", collector.kind())),
        );
    }
    let items = sanitize_items(items, &mut warnings);
    // `sanitize_items` removed everything `Inventory::new` rejects (bad or too long keys,
    // duplicates), so user-controlled input can no longer make this fail.
    let inventory = Inventory::new(taken_at, COLLECTOR_VERSION.to_owned(), platform, items)
        .map_err(|err| CollectError(err.to_string()))?;
    Ok(inventory.with_warnings(warnings))
}

/// Longest item key kept, in bytes (the inventory accepts 512; this leaves headroom).
const MAX_KEY_BYTES: usize = 500;

/// Data fields that count how many source entries an item stands for; summed when items merge.
const COUNT_FIELDS: [&str; 2] = ["occurrences", "instances"];

/// Makes `items` acceptable to [`Inventory::new`]: keys longer than [`MAX_KEY_BYTES`] are cut at a
/// character boundary, items whose keys then collide are merged (the first wins, counters are
/// added), and items with an empty or control-character key are dropped. A single summary warning
/// per kind of repair is added, never the key text itself.
fn sanitize_items(items: Vec<Item>, warnings: &mut Vec<String>) -> Vec<Item> {
    let (mut shortened, mut merged, mut dropped) = (0usize, 0usize, 0usize);
    let mut out: Vec<Item> = Vec::with_capacity(items.len());
    let mut index: BTreeMap<(Kind, String), usize> = BTreeMap::new();
    for mut item in items {
        if item.key.is_empty() || item.key.chars().any(char::is_control) {
            dropped += 1;
            continue;
        }
        if item.key.len() > MAX_KEY_BYTES {
            let mut cut = MAX_KEY_BYTES;
            while !item.key.is_char_boundary(cut) {
                cut -= 1;
            }
            item.key.truncate(cut);
            shortened += 1;
        }
        match index.entry((item.kind, item.key.clone())) {
            Entry::Occupied(slot) => {
                merged += 1;
                if let Some(first) = out.get_mut(*slot.get()) {
                    for field in COUNT_FIELDS {
                        if let (Some(a), Some(b)) = (first.u64_field(field), item.u64_field(field))
                        {
                            first
                                .data
                                .insert(field.into(), serde_json::Value::from(a + b));
                        }
                    }
                }
            }
            Entry::Vacant(slot) => {
                slot.insert(out.len());
                out.push(item);
            }
        }
    }
    if shortened > 0 {
        warnings.push(format!(
            "inventory: {shortened} item key(s) were too long and were shortened"
        ));
    }
    if merged > 0 {
        warnings.push(format!(
            "inventory: {merged} item(s) with identical keys were merged"
        ));
    }
    if dropped > 0 {
        warnings.push(format!(
            "inventory: {dropped} item(s) with an empty or invalid key were dropped"
        ));
    }
    out
}
