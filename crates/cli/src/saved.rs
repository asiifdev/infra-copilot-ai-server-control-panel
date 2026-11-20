//! `collect --save --node N` and `compare --save --source-node A --target-node B`: the stored
//! variants of the two file-based commands. They reuse the same collection and report code; this
//! module only adds reading from and writing to the local database.

use crate::Ui;
use crate::args::Globals;
use agent::{DbStore, ReadStore, StoreError, persist_comparison, persist_inventory};
use db::{Connection, InventoryId, OrgId};
use i18n::MessageKey;
use inventory::Inventory;

const SAVED_INVENTORY: MessageKey = MessageKey::new("cli.collect.saved");
const NODE_UNKNOWN: MessageKey = MessageKey::new("cli.collect.node_unknown");
const NO_INVENTORY: MessageKey = MessageKey::new("cli.compare.no_inventory");
const SAVED_COMPARISON: MessageKey = MessageKey::new("cli.compare.saved");

impl Ui<'_> {
    /// Stores a collected inventory as the newest snapshot of `node`.
    pub(crate) fn save_collected(
        &mut self,
        globals: &Globals,
        node: &str,
        inventory: &Inventory,
    ) -> u8 {
        let (mut conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        match persist_inventory(&mut conn, &scope.org, node, inventory) {
            Ok(id) => {
                let text = self.tr(SAVED_INVENTORY, &[("id", id.as_str()), ("node", node)]);
                self.note_err(&text);
                crate::EXIT_OK
            }
            Err(StoreError::NotFound) => self.fail(NODE_UNKNOWN, &[("node", node)]),
            Err(StoreError::Unavailable) => self.store_reason("store_unavailable"),
        }
    }

    /// The newest stored inventory of `node` and its id.
    fn stored_inventory(
        &mut self,
        conn: &Connection,
        org: &OrgId,
        node: &str,
    ) -> Result<(Inventory, InventoryId), u8> {
        let missing = |ui: &mut Self| ui.fail(NO_INVENTORY, &[("node", node)]);
        let inventory = match DbStore::new(conn).latest_inventory(org, node) {
            Ok(inventory) => inventory,
            Err(StoreError::NotFound) => return Err(missing(self)),
            Err(StoreError::Unavailable) => {
                return Err(self.store_reason("store_unavailable"));
            }
        };
        let id = db::list_nodes(conn, org)
            .ok()
            .and_then(|nodes| nodes.into_iter().find(|n| n.name == node))
            .and_then(|n| db::latest_inventory(conn, org, &n.id).ok())
            .map(|row| row.id);
        match id {
            Some(id) => Ok((inventory, id)),
            None => Err(missing(self)),
        }
    }

    /// Compares the newest stored inventories of two nodes, prints the report (same output and
    /// exit codes as the file-based comparison) and stores the result.
    pub(crate) fn compare_saved(&mut self, globals: &Globals, source: &str, target: &str) -> u8 {
        let (mut conn, _) = match self.open_db(globals, false) {
            Ok(opened) => opened,
            Err(code) => return code,
        };
        let scope = match self.scope(&conn, globals) {
            Ok(scope) => scope,
            Err(code) => return code,
        };
        let (source_inventory, source_id) = match self.stored_inventory(&conn, &scope.org, source) {
            Ok(found) => found,
            Err(code) => return code,
        };
        let (target_inventory, target_id) = match self.stored_inventory(&conn, &scope.org, target) {
            Ok(found) => found,
            Err(code) => return code,
        };
        let (code, findings) = self.report(&source_inventory, &target_inventory);
        match persist_comparison(&mut conn, &scope.org, &source_id, &target_id, &findings) {
            Ok(id) => {
                let text = self.tr(SAVED_COMPARISON, &[("id", id.as_str())]);
                self.note_err(&text);
                code
            }
            Err(_) => self.store_reason("comparison_not_saved"),
        }
    }
}
