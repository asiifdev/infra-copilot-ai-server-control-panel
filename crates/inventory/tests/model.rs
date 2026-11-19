use inventory::{Inventory, InventoryError, Item, Kind, Platform, SeenVia, Status};
use serde_json::{Map, Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn platform() -> Platform {
    Platform {
        os: "linux".into(),
        arch: "x86_64".into(),
    }
}

fn item(kind: Kind, key: &str) -> Item {
    let mut data = Map::new();
    data.insert("port".into(), Value::from(8080));
    Item {
        kind,
        key: key.into(),
        data,
        seen_via: SeenVia::Proc,
    }
}

fn build(items: Vec<Item>) -> Result<Inventory, InventoryError> {
    Inventory::new(
        "2026-01-01T00:00:00Z".into(),
        "0.0.0".into(),
        platform(),
        items,
    )
}

#[test]
fn items_are_sorted_and_lookup_works() -> TestResult {
    let inv = build(vec![
        item(Kind::Process, "proc:web"),
        item(Kind::Listener, "tcp:9000"),
        item(Kind::Listener, "tcp:80"),
    ])?;
    let keys: Vec<_> = inv.items().iter().map(|i| i.key.as_str()).collect();
    // Order is by kind (Listener before Process), then key.
    assert_eq!(keys, ["tcp:80", "tcp:9000", "proc:web"]);
    assert!(inv.get(Kind::Listener, "tcp:80").is_some());
    assert!(inv.get(Kind::Listener, "tcp:81").is_none());
    assert_eq!(inv.items_of(Kind::Listener).count(), 2);
    Ok(())
}

#[test]
fn duplicates_and_bad_keys_are_rejected() {
    let dup = build(vec![
        item(Kind::Listener, "tcp:80"),
        item(Kind::Listener, "tcp:80"),
    ]);
    assert!(matches!(
        dup,
        Err(InventoryError::DuplicateItem(Kind::Listener, _))
    ));
    // The same key under a different kind is fine.
    assert!(build(vec![item(Kind::Listener, "x"), item(Kind::Process, "x")]).is_ok());
    for bad in ["", "tab\there", "nul\0", &"k".repeat(513)] {
        assert!(matches!(
            build(vec![item(Kind::Listener, bad)]),
            Err(InventoryError::InvalidKey(_))
        ));
    }
}

#[test]
fn json_round_trip_keeps_everything() -> TestResult {
    let inv = build(vec![
        item(Kind::Listener, "tcp:80"),
        item(Kind::Cron, "cron:root:* * * * *:job"),
    ])?;
    let json = inv.to_json()?;
    assert!(json.contains("\"schema_version\": 1"));
    assert!(json.contains("\"kind\": \"listener\""));
    assert_eq!(Inventory::from_json(&json)?, inv);
    Ok(())
}

#[test]
fn foreign_schema_versions_and_garbage_are_refused() {
    let doc = |version: u32, items: Value| {
        json!({"schema_version": version, "taken_at": "t", "collector_version": "c",
               "platform": {"os": "linux", "arch": "x86_64"}, "status": "complete",
               "warnings": [], "items": items})
        .to_string()
    };
    assert!(matches!(
        Inventory::from_json(&doc(2, json!([]))),
        Err(InventoryError::UnsupportedSchema(2))
    ));
    // Duplicates are caught when reading JSON too, not only through the constructor.
    let dup_item = json!({"kind": "listener", "key": "tcp:1", "data": {}, "seen_via": "proc"});
    assert!(matches!(
        Inventory::from_json(&doc(1, json!([dup_item.clone(), dup_item]))),
        Err(InventoryError::DuplicateItem(..))
    ));
    for garbage in ["", "{", "null", "[]", "{\"schema_version\":1}"] {
        assert!(matches!(
            Inventory::from_json(garbage),
            Err(InventoryError::Parse(_))
        ));
    }
}

#[test]
fn partial_inventories_keep_their_warnings_through_json() -> TestResult {
    let complete = build(vec![item(Kind::Listener, "tcp:80")])?;
    assert_eq!(complete.status, Status::Complete);
    let partial = complete.with_warnings(vec!["cannot read /proc/1/fd".into()]);
    assert_eq!(partial.status, Status::Partial);
    let back = Inventory::from_json(&partial.to_json()?)?;
    assert_eq!(back.status, Status::Partial);
    assert_eq!(back.warnings, ["cannot read /proc/1/fd"]);
    // No warnings means it stays complete.
    assert_eq!(
        build(vec![])?.with_warnings(vec![]).status,
        Status::Complete
    );
    Ok(())
}
