//! Inventories, comparisons and findings: storage rules and tenant isolation.

mod common;
use common::*;
use db::*;

fn new_inventory<'a>(node: &'a NodeId, taken_at: &'a str) -> NewInventory<'a> {
    NewInventory {
        node,
        taken_at,
        collector_version: "0.1.0",
        status: InventoryStatus::Complete,
        os: "linux",
        arch: "x86_64",
        warnings_json: "[]",
    }
}

fn listener(key: &str) -> NewInventoryItem<'_> {
    NewInventoryItem {
        kind: ItemKind::Listener,
        key,
        data_json: r#"{"port":80}"#,
        seen_via: ItemSource::Proc,
    }
}

fn finding<'a>(rule: &'a str, severity: FindingSeverity, key: &'a str) -> NewFinding<'a> {
    NewFinding {
        rule_id: rule,
        severity,
        subject_kind: ItemKind::Listener,
        subject_key: key,
        evidence_json: "[]",
        args_json: "{}",
    }
}

#[test]
fn saves_and_reads_back_an_inventory_with_counts() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let items = [
        listener("tcp:80"),
        listener("tcp:443"),
        NewInventoryItem {
            kind: ItemKind::Process,
            key: "pm2:web",
            data_json: "{}",
            seen_via: ItemSource::Config,
        },
    ];
    let id = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-19T10:00:00Z"),
        &items,
    )?;
    let row = latest_inventory(&conn, &t.org, &t.node)?;
    assert_eq!(row.id, id);
    assert_eq!(row.status, InventoryStatus::Complete);
    let all = list_inventory_items(&conn, &t.org, &id, None, 100)?;
    assert_eq!(all.len(), 3);
    let only = list_inventory_items(&conn, &t.org, &id, Some(ItemKind::Process), 100)?;
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].key, "pm2:web");
    assert_eq!(list_inventory_items(&conn, &t.org, &id, None, 1)?.len(), 1);
    assert_eq!(
        count_inventory_items(&conn, &t.org, &id)?,
        vec![(ItemKind::Listener, 2), (ItemKind::Process, 1)]
    );
    Ok(())
}

#[test]
fn the_latest_inventory_is_the_newest_by_time() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let _old = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-18T10:00:00Z"),
        &[],
    )?;
    let new = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-19T10:00:00Z"),
        &[],
    )?;
    let _older = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-17T10:00:00Z"),
        &[],
    )?;
    assert_eq!(latest_inventory(&conn, &t.org, &t.node)?.id, new);
    Ok(())
}

#[test]
fn a_failed_save_leaves_nothing_behind() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let dup = [listener("tcp:80"), listener("tcp:80")];
    let result = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-19T10:00:00Z"),
        &dup,
    );
    assert!(is_constraint(&result));
    assert!(is_not_found(&latest_inventory(&conn, &t.org, &t.node)));
    let bad = [NewInventoryItem {
        data_json: "not json",
        ..listener("tcp:81")
    }];
    let result = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-19T10:00:00Z"),
        &bad,
    );
    assert!(is_constraint(&result));
    assert!(is_not_found(&latest_inventory(&conn, &t.org, &t.node)));
    Ok(())
}

#[test]
fn findings_come_back_most_severe_first_and_can_be_filtered() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let a = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-18T10:00:00Z"),
        &[],
    )?;
    let b = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-19T10:00:00Z"),
        &[],
    )?;
    let findings = [
        finding("PAR-008", FindingSeverity::Info, "tcp:1"),
        finding("PAR-001", FindingSeverity::Critical, "tcp:2"),
        finding("PAR-006", FindingSeverity::Important, "tcp:3"),
        finding("PAR-002", FindingSeverity::Critical, "tcp:4"),
    ];
    let cmp = save_comparison(&mut conn, &t.org, &a, &b, &findings)?;
    assert_eq!(latest_comparison(&conn, &t.org)?.id, cmp);
    let all = list_findings(&conn, &t.org, &cmp, None, 100)?;
    let order: Vec<_> = all.iter().map(|f| f.rule_id.as_str()).collect();
    assert_eq!(order, ["PAR-001", "PAR-002", "PAR-006", "PAR-008"]);
    let important = list_findings(&conn, &t.org, &cmp, Some(FindingSeverity::Important), 100)?;
    assert_eq!(important.len(), 3);
    assert_eq!(
        list_findings(&conn, &t.org, &cmp, Some(FindingSeverity::Critical), 1)?.len(),
        1
    );
    assert!(all.iter().all(|f| f.status == FindingStatus::New));
    Ok(())
}

#[test]
fn org_a_cannot_read_or_link_org_b_inventories() -> TestResult {
    let mut conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let a_inv = save_inventory(
        &mut conn,
        &a.org,
        &new_inventory(&a.node, "2026-11-19T10:00:00Z"),
        &[listener("tcp:80")],
    )?;
    let b_inv = save_inventory(
        &mut conn,
        &b.org,
        &new_inventory(&b.node, "2026-11-19T10:00:00Z"),
        &[listener("tcp:5432")],
    )?;
    let b_cmp = save_comparison(
        &mut conn,
        &b.org,
        &b_inv,
        &b_inv,
        &[finding("PAR-001", FindingSeverity::Critical, "tcp:5432")],
    )?;

    // Reads as A with B's ids.
    assert!(is_not_found(&latest_inventory(&conn, &a.org, &b.node)));
    assert!(is_not_found(&get_inventory(&conn, &a.org, &b_inv)));
    assert!(list_inventory_items(&conn, &a.org, &b_inv, None, 100)?.is_empty());
    assert!(count_inventory_items(&conn, &a.org, &b_inv)?.is_empty());
    assert!(list_findings(&conn, &a.org, &b_cmp, None, 100)?.is_empty());
    assert!(is_not_found(&latest_comparison(&conn, &a.org)));

    // Writes as A that reference B's rows.
    let r = save_inventory(
        &mut conn,
        &a.org,
        &new_inventory(&b.node, "2026-11-19T11:00:00Z"),
        &[],
    );
    assert!(is_not_found(&r));
    assert!(is_not_found(&save_comparison(
        &mut conn,
        &a.org,
        &a_inv,
        &b_inv,
        &[]
    )));
    assert!(is_not_found(&save_comparison(
        &mut conn,
        &a.org,
        &b_inv,
        &a_inv,
        &[]
    )));

    // B is untouched.
    assert_eq!(
        list_inventory_items(&conn, &b.org, &b_inv, None, 100)?.len(),
        1
    );
    assert_eq!(list_findings(&conn, &b.org, &b_cmp, None, 100)?.len(), 1);
    Ok(())
}

#[test]
fn the_schema_itself_refuses_a_cross_org_link() -> TestResult {
    let mut conn = open_in_memory()?;
    let a = populate(&conn, "alpha")?;
    let b = populate(&conn, "beta")?;
    let b_inv = save_inventory(
        &mut conn,
        &b.org,
        &new_inventory(&b.node, "2026-11-19T10:00:00Z"),
        &[],
    )?;
    // Raw SQL that bypasses the Rust checks: org A row pointing at B's inventory must fail.
    let result = conn.execute(
        "INSERT INTO inventory_items (org_id, inventory_id, kind, key, data, seen_via) \
         VALUES (?1, ?2, 'listener', 'tcp:1', '{}', 'proc')",
        rusqlite::params![a.org.as_str(), b_inv.as_str()],
    );
    assert!(result.is_err());
    Ok(())
}

#[test]
fn deleting_a_node_removes_its_inventories_items_and_comparisons() -> TestResult {
    let mut conn = open_in_memory()?;
    let t = populate(&conn, "alpha")?;
    let inv = save_inventory(
        &mut conn,
        &t.org,
        &new_inventory(&t.node, "2026-11-19T10:00:00Z"),
        &[listener("tcp:80")],
    )?;
    let _cmp = save_comparison(
        &mut conn,
        &t.org,
        &inv,
        &inv,
        &[finding("PAR-001", FindingSeverity::Info, "tcp:80")],
    )?;
    delete_node(&conn, &t.org, &t.node)?;
    let items: i64 = conn.query_row("SELECT count(*) FROM inventory_items", [], |r| r.get(0))?;
    let findings: i64 = conn.query_row("SELECT count(*) FROM findings", [], |r| r.get(0))?;
    assert_eq!((items, findings), (0, 0));
    Ok(())
}
