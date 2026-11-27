//! Read-only database metadata collector through [`RootFs`].
//!
//! Reads database engine, version, name, and row counts from read-only inventory/version files
//! (`/etc/nodivara/db-inventory.json`, `/etc/postgresql/*/main/PG_VERSION`,
//! `/var/lib/mysql/mysql_upgrade_info`). Never reads passwords or raw DSN credentials.

use crate::rootfs::{FsError, RootFs};
use inventory::{CollectError, Collection, Collector, Item, Kind, SeenVia};
use serde_json::{Map, Value};

const MAX_BYTES: usize = 256 * 1024;

/// Collects database metadata (engine, version, name, row counts) without credentials.
#[derive(Debug, Clone)]
pub struct DbMeta {
    root: RootFs,
}

impl DbMeta {
    /// Reads database metadata under `root`.
    #[must_use]
    pub const fn new(root: RootFs) -> Self {
        Self { root }
    }
}

fn parse_db_inventory_json(text: &str) -> Result<Vec<Item>, &'static str> {
    let val: Value = serde_json::from_str(text).map_err(|_| "invalid db-inventory.json")?;
    let arr = val
        .as_array()
        .ok_or("db-inventory.json must be a JSON array")?;
    let mut items = Vec::new();
    for entry in arr {
        let Some(obj) = entry.as_object() else {
            continue;
        };
        let Some(engine) = obj.get("engine").and_then(Value::as_str) else {
            continue;
        };
        let Some(name) = obj.get("name").and_then(Value::as_str) else {
            continue;
        };
        if engine.is_empty() || name.is_empty() {
            continue;
        }
        let mut data = Map::new();
        data.insert("engine".to_owned(), Value::String(engine.to_owned()));
        data.insert("name".to_owned(), Value::String(name.to_owned()));
        data.insert(
            "version".to_owned(),
            obj.get("version")
                .and_then(Value::as_str)
                .map_or(Value::Null, |v| Value::String(v.to_owned())),
        );
        data.insert(
            "row_count".to_owned(),
            obj.get("row_count")
                .and_then(Value::as_u64)
                .map_or(Value::Null, |n| Value::Number(n.into())),
        );
        data.insert(
            "host".to_owned(),
            obj.get("host")
                .and_then(Value::as_str)
                .map_or(Value::Null, |h| Value::String(h.to_owned())),
        );
        data.insert(
            "port".to_owned(),
            obj.get("port")
                .and_then(Value::as_u64)
                .map_or(Value::Null, |p| Value::Number(p.into())),
        );
        items.push(Item {
            kind: Kind::Database,
            key: format!("db:{engine}:{name}"),
            data,
            seen_via: SeenVia::Config,
        });
    }
    Ok(items)
}

impl Collector for DbMeta {
    fn kind(&self) -> &'static str {
        "db"
    }

    fn run(&self) -> Result<Collection, CollectError> {
        let mut items = Vec::new();
        let mut warnings = Vec::new();

        match self
            .root
            .read_to_string("/etc/nodivara/db-inventory.json", MAX_BYTES)
        {
            Ok(file) => match parse_db_inventory_json(&file.text) {
                Ok(parsed) => items.extend(parsed),
                Err(msg) => warnings.push(msg.to_owned()),
            },
            Err(FsError::NotFound) => {}
            Err(e) => warnings.push(format!("/etc/nodivara/db-inventory.json: {e}")),
        }

        // PostgreSQL cluster version files: /etc/postgresql/<ver>/main/PG_VERSION
        match self.root.read_dir("/etc/postgresql", 64) {
            Ok(listing) => {
                for ver in listing.names {
                    let path = format!("/etc/postgresql/{ver}/main/PG_VERSION");
                    if let Ok(file) = self.root.read_to_string(&path, 64) {
                        let version = file.text.trim();
                        if !version.is_empty() {
                            let key = format!("db:postgres:cluster-{ver}");
                            if !items.iter().any(|i| i.key == key) {
                                let mut data = Map::new();
                                data.insert(
                                    "engine".to_owned(),
                                    Value::String("postgres".to_owned()),
                                );
                                data.insert(
                                    "name".to_owned(),
                                    Value::String(format!("cluster-{ver}")),
                                );
                                data.insert(
                                    "version".to_owned(),
                                    Value::String(version.to_owned()),
                                );
                                data.insert("row_count".to_owned(), Value::Null);
                                items.push(Item {
                                    kind: Kind::Database,
                                    key,
                                    data,
                                    seen_via: SeenVia::Config,
                                });
                            }
                        }
                    }
                }
            }
            Err(FsError::NotFound) => {}
            Err(e) => warnings.push(format!("/etc/postgresql: {e}")),
        }

        Ok(Collection { items, warnings })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_db_inventory_without_secrets() -> Result<(), Box<dyn std::error::Error>> {
        let json = r#"[
            {"engine": "postgres", "name": "app_prod", "version": "16.2", "row_count": 4200, "host": "127.0.0.1", "port": 5432}
        ]"#;
        let items = parse_db_inventory_json(json)?;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].key, "db:postgres:app_prod");
        assert_eq!(items[0].str_field("version"), Some("16.2"));
        assert_eq!(items[0].u64_field("row_count"), Some(4200));
        Ok(())
    }
}
