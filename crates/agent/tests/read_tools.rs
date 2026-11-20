//! The tier 0 tools against a real database: correct answers, tenant isolation, and the
//! contract suite on every built-in tool.

use agent::{
    CallStatus, DbStore, Executor, ToolContext, assistant_registry, persist_comparison,
    persist_inventory, testing,
};
use db::{
    Connection, NewNode, NodeAccess, NodeKind, OrgId, create_node, create_org, open_in_memory,
};
use i18n::{Catalog, Locale};
use inventory::{Inventory, Item, Kind, Platform, RuleContext, SeenVia};
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn item(kind: Kind, key: &str, data: Value) -> Item {
    let Value::Object(data) = data else {
        unreachable!("test data is an object")
    };
    Item {
        kind,
        key: key.to_owned(),
        data,
        seen_via: SeenVia::Proc,
    }
}

fn listener(port: u16, process: &str) -> Item {
    item(
        Kind::Listener,
        &format!("tcp:{port}"),
        json!({"proto":"tcp","port":port,"bind":"0.0.0.0","process":process,"pid_hint":null,"exposed":"public"}),
    )
}

fn inventory(items: Vec<Item>) -> Result<Inventory, Box<dyn std::error::Error>> {
    Ok(Inventory::new(
        "2026-11-19T10:00:00Z".to_owned(),
        "0.1.0".to_owned(),
        Platform {
            os: "linux".to_owned(),
            arch: "x86_64".to_owned(),
        },
        items,
    )?)
}

struct World {
    conn: Connection,
    org: OrgId,
}

fn node(conn: &Connection, org: &OrgId, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    create_node(
        conn,
        org,
        &NewNode {
            name,
            kind: NodeKind::Vps,
            access: NodeAccess::Agent,
            labels_json: "{}",
        },
    )?;
    Ok(())
}

/// An org with an old server that runs a search engine and a new one that does not.
fn world(slug: &str) -> Result<World, Box<dyn std::error::Error>> {
    let mut conn = open_in_memory()?;
    let org = create_org(&conn, slug, slug, "free")?.id;
    node(&conn, &org, "old")?;
    node(&conn, &org, "new")?;
    let old = inventory(vec![listener(80, "nginx"), listener(7700, "meilisearch")])?;
    let new = inventory(vec![listener(80, "nginx")])?;
    let a = persist_inventory(&mut conn, &org, "old", &old)?;
    let b = persist_inventory(&mut conn, &org, "new", &new)?;
    let report = parity::compare(&old, &new, &RuleContext::default());
    persist_comparison(&mut conn, &org, &a, &b, &report.findings)?;
    Ok(World { conn, org })
}

fn ask(
    world: &World,
    org: &OrgId,
    tool: &str,
    args: &str,
) -> Result<agent::Outcome, Box<dyn std::error::Error>> {
    let executor = Executor::new(assistant_registry()?);
    let catalog = Catalog::builtin()?;
    let store = DbStore::new(&world.conn);
    let ctx = ToolContext::new(org, &store, &catalog, Locale::En);
    Ok(executor.call(&ctx, tool, args))
}

#[test]
fn the_registry_holds_the_read_tools_and_propose_fix() -> TestResult {
    let registry = assistant_registry()?;
    assert_eq!(registry.len(), 9);
    assert!(
        registry
            .definitions()
            .iter()
            .all(|d| d.parameters["additionalProperties"] == false)
    );
    Ok(())
}

#[test]
fn lists_nodes_and_summarizes_an_inventory() -> TestResult {
    let w = world("acme")?;
    let out = ask(&w, &w.org, "list_nodes", "{}")?;
    assert_eq!(out.record.status, CallStatus::Ok);
    assert!(
        out.content.contains("old | vps | pending"),
        "{}",
        out.content
    );
    let out = ask(&w, &w.org, "get_inventory_summary", r#"{"node":"old"}"#)?;
    assert!(out.content.contains("listener: 2"), "{}", out.content);
    assert!(out.content.contains("status: complete"), "{}", out.content);
    let out = ask(
        &w,
        &w.org,
        "list_inventory_items",
        r#"{"node":"old","kind":"listener","limit":1}"#,
    )?;
    assert!(out.content.contains("tcp:7700") || out.content.contains("tcp:80"));
    assert!(out.content.contains("[1 more not shown"), "{}", out.content);
    Ok(())
}

#[test]
fn finds_what_the_new_server_is_missing_and_explains_it() -> TestResult {
    let w = world("acme")?;
    let out = ask(
        &w,
        &w.org,
        "list_findings",
        r#"{"min_severity":"important"}"#,
    )?;
    assert!(
        out.content.contains("comparison: old -> new"),
        "{}",
        out.content
    );
    assert!(
        out.content.contains("PAR-001 listener:tcp:7700"),
        "{}",
        out.content
    );
    let out = ask(
        &w,
        &w.org,
        "explain_finding",
        r#"{"rule_id":"PAR-001","subject_key":"tcp:7700"}"#,
    )?;
    assert_eq!(out.record.status, CallStatus::Ok);
    assert!(out.content.contains("advice:"), "{}", out.content);
    assert!(out.content.contains("meilisearch"), "{}", out.content);
    let out = ask(
        &w,
        &w.org,
        "explain_finding",
        r#"{"rule_id":"PAR-001","subject_key":"tcp:9"}"#,
    )?;
    assert_eq!(out.record.status, CallStatus::ToolFailed("not_found"));
    Ok(())
}

#[test]
fn another_org_sees_nothing_of_this_orgs_servers_or_findings() -> TestResult {
    let w = world("acme")?;
    let other = create_org(&w.conn, "rival", "rival", "free")?.id;
    // The rival has its own node, and no data at all.
    node(&w.conn, &other, "solo")?;
    let out = ask(&w, &other, "list_nodes", "{}")?;
    assert!(
        out.content.contains("solo") && !out.content.contains("old"),
        "{}",
        out.content
    );
    for (tool, args) in [
        ("get_inventory_summary", r#"{"node":"old"}"#),
        ("list_inventory_items", r#"{"node":"new"}"#),
        ("list_findings", "{}"),
        (
            "explain_finding",
            r#"{"rule_id":"PAR-001","subject_key":"tcp:7700"}"#,
        ),
    ] {
        let out = ask(&w, &other, tool, args)?;
        assert_eq!(
            out.record.status,
            CallStatus::ToolFailed("not_found"),
            "{tool}"
        );
        assert!(
            !out.content.contains("meilisearch") && !out.content.contains("7700"),
            "{tool}"
        );
    }
    Ok(())
}

#[test]
fn a_node_name_that_looks_like_sql_or_a_path_is_just_not_found() -> TestResult {
    let w = world("acme")?;
    for name in ["old' OR '1'='1", "../old", "OLD", "old "] {
        let args = json!({ "node": name }).to_string();
        let out = ask(&w, &w.org, "get_inventory_summary", &args)?;
        assert_eq!(
            out.record.status,
            CallStatus::ToolFailed("not_found"),
            "{name}"
        );
    }
    Ok(())
}

#[test]
fn every_builtin_tool_passes_the_contract_suite() -> TestResult {
    // The suite builds its own registry per tool, so rebuild each tool through the public one.
    let registry = assistant_registry()?;
    for def in registry.definitions() {
        let valid = match def.name {
            "get_inventory_summary" | "list_inventory_items" => r#"{"node":"x"}"#,
            "explain_finding" => r#"{"rule_id":"PAR-001","subject_key":"tcp:1"}"#,
            "propose_fix" => r#"{"title":"t","rationale":"r","suggested_action":"a"}"#,
            "search_inventory" => r#"{"query":"nginx"}"#,
            "remember" => r#"{"kind":"task","content":"c"}"#,
            _ => "{}",
        };
        // Against the null store the tools answer not_found, which is a tool failure, not a
        // contract violation: only argument handling and fencing are under test here.
        let executor = Executor::new(assistant_registry()?);
        testing::with_null_context(|ctx| {
            let ok = executor.call(ctx, def.name, valid);
            assert!(
                matches!(ok.record.status, CallStatus::Ok | CallStatus::ToolFailed(_)),
                "{}",
                def.name
            );
            assert!(ok.content.starts_with("<untrusted_output"), "{}", def.name);
            for bad in [r#"{"org_id":"x"}"#, r#"{"zz":1}"#, "{", "[]"] {
                let out = executor.call(ctx, def.name, bad);
                assert!(
                    matches!(out.record.status, CallStatus::BadArguments(_)),
                    "{} {bad}",
                    def.name
                );
            }
        });
    }
    Ok(())
}

#[test]
fn indonesian_locale_changes_the_explanation_text() -> TestResult {
    let w = world("acme")?;
    let executor = Executor::new(assistant_registry()?);
    let catalog = Catalog::builtin()?;
    let store = DbStore::new(&w.conn);
    let en = ToolContext::new(&w.org, &store, &catalog, Locale::En);
    let id = ToolContext::new(&w.org, &store, &catalog, Locale::Id);
    let args = r#"{"rule_id":"PAR-001","subject_key":"tcp:7700"}"#;
    let a = executor.call(&en, "explain_finding", args);
    let b = executor.call(&id, "explain_finding", args);
    assert_ne!(a.content, b.content);
    Ok(())
}
