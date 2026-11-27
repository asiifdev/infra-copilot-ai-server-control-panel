//! The read side of the API against a real database: session, texts, nodes, inventories,
//! findings, plans, transports, memories, proposals and conversations; tenant isolation through
//! the API; and the hostile data a server (or a model) can put into them.

mod common;

use agent::{
    DbPlanSink, DbPlanStore, DbStore, DraftContext, DraftRequest, persist_comparison,
    persist_inventory,
};
use common::*;
use db::{
    Connection, MemoryKind, MemorySource, MemoryStatus, NewMemory, NewProposal, NewTransport,
    OrgId, SealedSecret, put_secret, record_transport_check,
};
use i18n::{Catalog, Locale};
use inventory::{Finding, Inventory, Item, Kind, Platform, SeenVia, Severity, SubjectRef};
use serde_json::{Map, Value, json};
use std::collections::BTreeMap;

const CONFIG_MARK: &str = "zz-config-marker-bucket";
const PLAIN_KEY: &str = "pm2:evilname";
const ESC_KEY: &str = "pm2:evil\u{1b}[31mname";

fn item(kind: Kind, key: &str, data: Value) -> Item {
    let Value::Object(map) = data else {
        unreachable!("object")
    };
    Item {
        kind,
        key: key.to_owned(),
        data: map,
        seen_via: SeenVia::Proc,
    }
}

fn listener(port: u16) -> Item {
    item(
        Kind::Listener,
        &format!("tcp:{port}"),
        json!({"proto":"tcp","port":port,"bind":"0.0.0.0","process":"app","pid_hint":null,"exposed":"public"}),
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

fn critical(key: &str) -> Finding {
    Finding {
        rule_id: "PAR-001".to_owned(),
        severity: Severity::Critical,
        subject: SubjectRef {
            kind: Kind::Listener,
            key: key.to_owned(),
        },
        evidence: vec![SubjectRef {
            kind: Kind::Listener,
            key: "tcp:80".to_owned(),
        }],
        args: BTreeMap::from([("port".to_owned(), "7700".to_owned())]),
    }
}

fn important(key: &str) -> Finding {
    Finding {
        severity: Severity::Important,
        subject: SubjectRef {
            kind: Kind::Process,
            key: key.to_owned(),
        },
        evidence: Vec::new(),
        rule_id: "PAR-002".to_owned(),
        args: BTreeMap::new(),
    }
}

/// Everything the read tests look at, for one organization.
struct Seeded {
    proposal: String,
    memory: String,
    conversation: String,
}

fn seed(
    world: &World,
    conn: &mut Connection,
    org: &OrgId,
    tag: &str,
) -> Result<Seeded, Box<dyn std::error::Error>> {
    for name in ["old", "new", "bare"] {
        node(conn, org, name)?;
    }
    let secret_marker = format!("{}{}", "sk-", "Zq8Lm3Rt".repeat(4));
    let old = inventory(vec![
        listener(80),
        listener(7700),
        item(
            Kind::Process,
            PLAIN_KEY,
            json!({"cmd": format!("run --token {secret_marker}"), "mode": "prod"}),
        ),
    ])?;
    let new = inventory(vec![listener(80)])?;
    let a = persist_inventory(conn, org, "old", &old)?;
    let b = persist_inventory(conn, org, "new", &new)?;
    persist_comparison(
        conn,
        org,
        &a,
        &b,
        &[critical("tcp:7700"), important(PLAIN_KEY)],
    )?;
    // A transport with settings and sealed credentials, and a check result with a string in it.
    let secret = put_secret(
        conn,
        org,
        "transport",
        &SealedSecret {
            ciphertext: vec![7; 32],
            key_id: "k1".to_owned(),
        },
    )?;
    let id = db::create_transport(
        conn,
        org,
        &NewTransport {
            name: "main",
            kind: "rclone",
            config_json: &format!(r#"{{"remote":"{CONFIG_MARK}"}}"#),
            secret: Some(&secret),
        },
    )?;
    record_transport_check(
        conn,
        org,
        &id,
        r#"{"free":5,"ok":true,"note":"a-string"}"#,
        true,
    )?;
    // A draft plan.
    {
        let catalog = Catalog::builtin()?;
        let (store, plans, sink) = (
            DbStore::new(conn),
            DbPlanStore::new(conn),
            DbPlanSink::new(conn),
        );
        let ctx = DraftContext {
            org,
            store: &store,
            plans: &plans,
            sink: &sink,
            catalog: &catalog,
            locale: Locale::En,
        };
        agent::draft_migration_plan(
            &ctx,
            &DraftRequest {
                source: "old",
                target: "new",
                transport: Some("main"),
                name: Some(&format!("{tag} plan")),
                ..DraftRequest::default()
            },
        )
        .map_err(|e| format!("{e:?}"))?;
    }
    // Memories: one pending note, and a preference of each of the two people of org A.
    let user = if *org == world.org {
        &world.user
    } else {
        &world.rival_user
    };
    let memory = db::save_memory(
        conn,
        org,
        &NewMemory {
            user: None,
            kind: MemoryKind::OrgFact,
            content: &format!("{tag} remembers \u{1b}[2J this"),
            source: MemorySource::Assistant,
            conversation: None,
            status: MemoryStatus::Pending,
        },
    )?;
    db::save_memory(
        conn,
        org,
        &NewMemory {
            user: Some(user),
            kind: MemoryKind::Preference,
            content: &format!("{tag} prefers short answers"),
            source: MemorySource::Person,
            conversation: None,
            status: MemoryStatus::Active,
        },
    )?;
    let proposal = db::create_proposal(
        conn,
        org,
        &NewProposal {
            conversation: None,
            tool: "propose_fix",
            tier: 2,
            title: &format!("{tag} proposal"),
            rationale: "because",
            suggested_action: "do it",
            impact: "small",
            rule_id: Some("PAR-001"),
            subject_key: Some("tcp:7700"),
            args_hash: &"a".repeat(64),
        },
    )?;
    let conversation = db::create_conversation(conn, org, user, &format!("{tag} chat"), None)?;
    Ok(Seeded {
        proposal: proposal.as_str().to_owned(),
        memory: memory.as_str().to_owned(),
        conversation: conversation.as_str().to_owned(),
    })
}

/// The model refuses control characters, the database does not, and the server must not trust
/// either: plants some the way a tampered database file would have them.
fn tamper(world: &World) -> Result<(), Box<dyn std::error::Error>> {
    let conn = world.conn()?;
    conn.execute(
        "UPDATE inventory_items SET key = ?1 WHERE key = ?2",
        [ESC_KEY, PLAIN_KEY],
    )?;
    conn.execute(
        "UPDATE findings SET args = ?1 WHERE rule_id = 'PAR-001'",
        [r#"{"port":"77\u001b[2J00"}"#],
    )?;
    Ok(())
}

struct Fixture {
    world: World,
    server: TestServer,
    session: Session,
    mine: Seeded,
    theirs: Seeded,
}

fn fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
    let world = World::new()?;
    let mut conn = world.conn()?;
    let mine = seed(&world, &mut conn, &world.org, "acme")?;
    let theirs = seed(&world, &mut conn, &world.rival, "rival")?;
    // A preference and a conversation of the second person of org A: not the signed-in person.
    db::save_memory(
        &conn,
        &world.org,
        &NewMemory {
            user: Some(&world.second),
            kind: MemoryKind::Preference,
            content: "second prefers long answers",
            source: MemorySource::Person,
            conversation: None,
            status: MemoryStatus::Active,
        },
    )?;
    db::create_conversation(&conn, &world.org, &world.second, "second's chat", None)?;
    drop(conn);
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    Ok(Fixture {
        world,
        server,
        session,
        mine,
        theirs,
    })
}

fn names(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v["name"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

// ---- session and texts ---------------------------------------------------------------------------

#[test]
fn the_session_describes_the_signed_in_person_and_setup() -> TestResult {
    let f = fixture()?;
    let r = f.session.get("/api/session");
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(
        body["user"],
        json!({"email": "owner@example.test", "name": "Olive Owner"})
    );
    assert_eq!(body["org"], json!({"name": "Acme", "slug": "acme"}));
    assert_eq!(body["locale"], "en");
    assert_eq!(body["master_key_configured"], false);
    assert_eq!(body["execution_enabled"], false);
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(body.as_object().map(Map::len), Some(6));
    assert_eq!(f.session.get("/api/session?lang=id").json()["locale"], "id");
    assert_eq!(
        f.session.get_lang("/api/session", "id-ID,en;q=0.5").json()["locale"],
        "id"
    );
    assert_eq!(
        f.session.get_lang("/api/session", "fr").json()["locale"],
        "en"
    );
    let keyed = TestServer::start_with(
        &f.world,
        Opts {
            vault: Some(vault()?),
            ..Opts::default()
        },
    )?;
    let session = keyed.sign_in()?;
    assert_eq!(
        session.get("/api/session").json()["master_key_configured"],
        true
    );
    Ok(())
}

#[test]
fn the_ui_texts_come_from_the_catalog_with_english_as_the_fallback() -> TestResult {
    let world = World::new()?;
    let catalog = Catalog::from_sources(
        "ui.nav.nodes = Nodes\nui.nav.plans = Plans\nui.greeting.hello = Hello {name}, {{braces}}\nother.area.thing = hidden\n",
        "ui.nav.nodes = Server\nother.area.thing = tersembunyi\n",
    )?;
    let server = TestServer::start_with(
        &world,
        Opts {
            catalog: Some(catalog),
            ..Opts::default()
        },
    )?;
    let session = server.sign_in()?;
    let en = session.get("/api/i18n/en").json();
    assert_eq!(
        en,
        json!({"ui.nav.nodes": "Nodes", "ui.nav.plans": "Plans", "ui.greeting.hello": "Hello {name}, {{braces}}"})
    );
    let id = session.get("/api/i18n/id").json();
    assert_eq!(id["ui.nav.nodes"], "Server");
    assert_eq!(id["ui.nav.plans"], "Plans");
    assert!(id.get("other.area.thing").is_none());
    assert_eq!(session.get("/api/i18n/id-ID").status, 200);
    assert_eq!(session.get("/api/i18n/xx").status, 404);
    assert_eq!(session.get("/api/i18n/..%2f..%2fetc").status, 404);
    Ok(())
}

// ---- nodes and inventories -----------------------------------------------------------------------

#[test]
fn nodes_list_their_latest_inventory_and_nothing_of_the_rival() -> TestResult {
    let f = fixture()?;
    let body = f.session.get("/api/nodes").json();
    assert_eq!(names(&body), ["bare", "new", "old"]);
    let old = &body[2];
    assert_eq!(old["kind"], "vps");
    assert_eq!(old["status"], "pending");
    assert_eq!(old["latest_inventory"]["status"], "complete");
    assert_eq!(old["latest_inventory"]["taken_at"], "2026-11-19T10:00:00Z");
    assert_eq!(
        old["latest_inventory"]["counts"],
        json!({
            "listener": 2,
            "process": 1,
            "cron": 0,
            "env_ref": 0,
            "vhost": 0,
            "cert": 0,
            "database": 0,
            "dns": 0,
            "check": 0
        })
    );
    assert_eq!(body[0]["latest_inventory"], Value::Null);
    Ok(())
}

#[test]
fn an_inventory_is_listed_with_redacted_and_cleaned_values() -> TestResult {
    let f = fixture()?;
    tamper(&f.world)?;
    let r = f.session.get("/api/nodes/old/inventory");
    assert_eq!(r.status, 200);
    let body = r.json();
    assert_eq!(body["status"], "complete");
    assert_eq!(body["warnings"], json!([]));
    let items = body["items"].as_array().ok_or("items")?;
    assert_eq!(items.len(), 3);
    assert_eq!(items[0]["kind"], "listener");
    // Sorted by kind, then by key as text.
    assert_eq!(items[0]["key"], "tcp:7700");
    assert_eq!(items[0]["data"]["port"], 7700);
    // The process: a control character in its key is replaced, a secret in its command redacted.
    let process = items
        .iter()
        .find(|i| i["kind"] == "process")
        .ok_or("process")?;
    let text = r.text();
    assert!(
        !text.contains('\u{1b}') && !text.contains("\\u001b"),
        "{text}"
    );
    assert!(!text.contains("Zq8LmZq8Lm"), "{text}");
    assert!(!text.contains(&"Zq8Lm3Rt".repeat(4)), "{text}");
    assert!(
        process["data"]["cmd"]
            .as_str()
            .is_some_and(|c| c.contains("run --token"))
    );
    let only = f
        .session
        .get("/api/nodes/old/inventory?kind=listener&limit=1")
        .json();
    assert_eq!(only["items"].as_array().map(Vec::len), Some(1));
    for bad in ["kind=nope", "limit=0", "limit=99999", "limit=x"] {
        assert_eq!(
            f.session
                .get(&format!("/api/nodes/old/inventory?{bad}"))
                .status,
            400,
            "{bad}"
        );
    }
    assert_eq!(f.session.get("/api/nodes/bare/inventory").status, 404);
    assert_eq!(f.session.get("/api/nodes/missing/inventory").status, 404);
    Ok(())
}

// ---- findings ------------------------------------------------------------------------------------

/// The rules mark "the collector could not tell which process" with an internal marker that the
/// renderer turns into the localized word. Cleaning control characters must not destroy the marker
/// (it once turned into a replacement character followed by the word "unknown").
#[test]
fn an_unknown_process_is_shown_as_the_localized_word() -> TestResult {
    let f = fixture()?;
    f.world.conn()?.execute(
        "UPDATE findings SET args = ?1 WHERE rule_id = 'PAR-001'",
        [r#"{"port":"22","process":"\u0000unknown","bind":"10.0.2.15"}"#],
    )?;
    let catalog = Catalog::builtin()?;
    for (lang, locale) in [("en", Locale::En), ("id", Locale::Id)] {
        let body = f.session.get(&format!("/api/findings?lang={lang}")).text();
        let word = catalog
            .raw(locale, "parity.arg.unknown")
            .ok_or("catalog word")?;
        assert!(body.contains(word), "{lang}: {body}");
        assert!(!body.contains('\u{FFFD}'), "{lang}: replacement character");
        assert!(!body.contains("\\u0000"), "{lang}: raw marker");
    }
    Ok(())
}

#[test]
fn findings_are_rendered_in_the_language_of_the_request() -> TestResult {
    let f = fixture()?;
    tamper(&f.world)?;
    let en = f.session.get("/api/findings").json();
    assert_eq!(en["comparison"]["source"], "old");
    assert_eq!(en["comparison"]["target"], "new");
    let list = en["findings"].as_array().ok_or("findings")?;
    assert_eq!(list.len(), 2);
    // Most severe first.
    assert_eq!(list[0]["rule_id"], "PAR-001");
    assert_eq!(list[0]["severity"], "critical");
    assert_eq!(
        list[0]["subject"],
        json!({"kind": "listener", "key": "tcp:7700"})
    );
    assert_eq!(
        list[0]["evidence"],
        json!([{"kind": "listener", "key": "tcp:80"}])
    );
    for field in ["title", "explanation", "advice"] {
        assert!(
            list[0][field].as_str().is_some_and(|t| !t.is_empty()),
            "{field}"
        );
    }
    assert_eq!(list[0]["note"], Value::Null);
    let id = f.session.get("/api/findings?lang=id").json();
    assert_ne!(id["findings"][0]["title"], en["findings"][0]["title"]);
    // A control character from the inventory never reaches the output.
    assert!(!f.session.get("/api/findings").text().contains("\\u001b"));
    Ok(())
}

#[test]
fn findings_can_be_filtered_and_limited_and_bad_parameters_are_refused() -> TestResult {
    let f = fixture()?;
    let critical_only = f.session.get("/api/findings?min_severity=critical").json();
    assert_eq!(critical_only["findings"].as_array().map(Vec::len), Some(1));
    let one = f.session.get("/api/findings?limit=1").json();
    assert_eq!(one["findings"].as_array().map(Vec::len), Some(1));
    for bad in ["min_severity=urgent", "limit=0", "limit=100000", "limit=-1"] {
        assert_eq!(
            f.session.get(&format!("/api/findings?{bad}")).status,
            400,
            "{bad}"
        );
    }
    Ok(())
}

#[test]
fn without_a_comparison_there_are_no_findings_and_no_comparison() -> TestResult {
    let world = World::new()?;
    let server = TestServer::start(&world)?;
    let session = server.sign_in()?;
    assert_eq!(
        session.get("/api/findings").json(),
        json!({"comparison": null, "findings": []})
    );
    assert_eq!(session.get("/api/nodes").json(), json!([]));
    assert_eq!(session.get("/api/plans").json(), json!([]));
    Ok(())
}

// ---- plans and transports --------------------------------------------------------------------------

#[test]
fn a_plan_is_shown_with_its_gates_and_its_text() -> TestResult {
    let f = fixture()?;
    let list = f.session.get("/api/plans").json();
    assert_eq!(names(&list), ["acme plan"]);
    assert_eq!(list[0]["status"], "draft");
    assert_eq!(list[0]["source_node"], "old");
    assert_eq!(list[0]["target_node"], "new");
    let r = f.session.get("/api/plans/acme%20plan");
    assert_eq!(r.status, 200, "{}", r.text());
    let body = r.json();
    assert_eq!(body["name"], "acme plan");
    assert_eq!(body["status"], "draft");
    assert!(body["spec"].is_object());
    assert!(body["steps"].as_array().is_some_and(|s| s.len() == 10));
    assert!(
        body["rendered"]
            .as_str()
            .is_some_and(|t| t.contains("acme plan"))
    );
    // The route names the nodes, not their row ids.
    let rendered = body["rendered"].as_str().unwrap_or_default();
    assert!(
        rendered.contains("Data moves from node old to node new."),
        "{rendered}"
    );
    assert!(!rendered.contains(body["spec"]["source"]["node_id"].as_str().unwrap_or("-")));
    // The transport is named too.
    assert!(rendered.contains("Transport: main;"), "{rendered}");
    assert!(
        !rendered.contains(
            body["spec"]["transport"]["transport_id"]
                .as_str()
                .unwrap_or("-")
        )
    );
    let gates = &body["gates"];
    assert_eq!(gates["cutover_allowed"], false);
    assert_eq!(gates["shutdown_allowed"], false);
    assert_eq!(gates["open_critical"][0]["rule_id"], "PAR-001");
    assert_eq!(
        gates["open_critical"][0]["subject"],
        json!({"kind": "listener", "key": "tcp:7700"})
    );
    assert_eq!(gates["important"], 1);
    assert_eq!(gates["accepted_critical"], json!([]));
    // The plan text follows the request language.
    let id = f.session.get("/api/plans/acme%20plan?lang=id").json();
    assert_ne!(id["rendered"], body["rendered"]);
    assert_eq!(f.session.get("/api/plans/nope").status, 404);
    // The rival's plan does not exist here.
    assert_eq!(f.session.get("/api/plans/rival%20plan").status, 404);
    Ok(())
}

#[test]
fn transports_show_no_settings_no_credentials_and_no_text_from_a_check() -> TestResult {
    let f = fixture()?;
    let r = f.session.get("/api/transports");
    let body = r.json();
    assert_eq!(names(&body), ["main"]);
    assert_eq!(body[0]["kind"], "rclone");
    assert!(body[0]["verified_at"].is_string());
    assert_eq!(body[0]["last_check"], json!({"free": 5, "ok": true}));
    assert!(body[0]["id"].is_string());
    assert_eq!(body[0].as_object().map(Map::len), Some(5));
    let text = r.text();
    assert!(
        !text.contains(CONFIG_MARK) && !text.contains("a-string"),
        "{text}"
    );
    assert!(!text.contains("secret"));
    Ok(())
}

// ---- memories and proposals -----------------------------------------------------------------------

#[test]
fn memories_are_approved_and_deleted_only_by_a_person_here() -> TestResult {
    let f = fixture()?;
    let pending = f.session.get("/api/memories?status=pending").json();
    assert_eq!(pending.as_array().map(Vec::len), Some(1));
    assert_eq!(pending[0]["id"], f.mine.memory.as_str());
    assert_eq!(pending[0]["status"], "pending");
    assert_eq!(pending[0]["source"], "assistant");
    // Model-written text is shown without control characters.
    assert!(!f.session.get("/api/memories").text().contains("\\u001b"));
    // Another person's preference is invisible; the rival's notes too.
    let all = f.session.get("/api/memories").json();
    let texts: Vec<&str> = all
        .as_array()
        .ok_or("list")?
        .iter()
        .filter_map(|m| m["content"].as_str())
        .collect();
    assert!(texts.iter().any(|t| t.contains("acme prefers")));
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("second prefers") || t.contains("rival")),
        "{texts:?}"
    );
    assert_eq!(f.session.get("/api/memories?status=bogus").status, 400);

    let r = f
        .session
        .post_empty(&format!("/api/memories/{}/approve", f.mine.memory));
    assert_eq!(r.status, 204);
    assert_eq!(
        f.session.get("/api/memories?status=pending").json(),
        json!([])
    );
    assert_eq!(
        f.session
            .get("/api/memories?status=active")
            .json()
            .as_array()
            .map(Vec::len),
        Some(2)
    );
    assert_eq!(
        f.session
            .delete(&format!("/api/memories/{}", f.mine.memory))
            .status,
        204
    );
    assert_eq!(
        f.session
            .delete(&format!("/api/memories/{}", f.mine.memory))
            .status,
        404
    );
    Ok(())
}

#[test]
fn memory_ids_of_others_or_malformed_ones_behave_like_missing_ones() -> TestResult {
    let f = fixture()?;
    let second_pref = f.world.conn()?.query_row(
        "SELECT id FROM memories WHERE content = 'second prefers long answers'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    for id in [
        f.theirs.memory.as_str(),
        second_pref.as_str(),
        "not-a-uuid",
        "../../etc",
        "00000000-0000-4000-8000-000000000000",
    ] {
        assert_eq!(
            f.session
                .post_empty(&format!("/api/memories/{id}/approve"))
                .status,
            404,
            "{id}"
        );
        assert_eq!(
            f.session.delete(&format!("/api/memories/{id}")).status,
            404,
            "{id}"
        );
    }
    // The rival's pending note is still pending, and the other person's preference still exists.
    let conn = f.world.conn()?;
    let pending: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE status = 'pending'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(pending, 2);
    let still: i64 = conn.query_row(
        "SELECT COUNT(*) FROM memories WHERE id = ?1",
        [&second_pref],
        |r| r.get(0),
    )?;
    assert_eq!(still, 1);
    Ok(())
}

#[test]
fn proposals_are_listed_and_a_person_can_decide_them_once() -> TestResult {
    let f = fixture()?;
    let body = f.session.get("/api/proposals").json();
    assert_eq!(body.as_array().map(Vec::len), Some(1));
    assert_eq!(body[0]["title"], "acme proposal");
    assert_eq!(body[0]["status"], "pending");
    assert_eq!(body[0]["tier"], 2);
    assert!(body[0].get("args_hash").is_none());
    assert_eq!(
        f.session.get("/api/proposals?status=executed").json(),
        json!([])
    );
    assert_eq!(f.session.get("/api/proposals?status=bogus").status, 400);

    let path = format!("/api/proposals/{}/decision", f.mine.proposal);
    // Bad bodies change nothing.
    for bad in [
        json!({}),
        json!({"decision": "run"}),
        json!({"decision": "approve", "x": 1}),
        json!({"decision": 1}),
    ] {
        assert_eq!(f.session.post(&path, &bad).status, 400, "{bad}");
    }
    let r = f.session.post(&path, &json!({"decision": "approve"}));
    assert_eq!(r.status, 200);
    assert_eq!(r.json()["status"], "approved");
    // The second decision loses with 409 and the status stays.
    let again = f.session.post(&path, &json!({"decision": "reject"}));
    assert_eq!(again.status, 409);
    assert_eq!(
        f.session
            .get("/api/proposals?status=approved")
            .json()
            .as_array()
            .map(Vec::len),
        Some(1)
    );
    // Approving executed nothing and left one audit entry.
    let audited: i64 = f.world.conn()?.query_row(
        "SELECT COUNT(*) FROM audit_log WHERE action = 'proposal.approve'",
        [],
        |r| r.get(0),
    )?;
    assert_eq!(audited, 1);
    // The rival's proposal and malformed ids behave like missing ones.
    for id in [f.theirs.proposal.as_str(), "not-a-uuid", "../x"] {
        assert_eq!(
            f.session
                .post(
                    &format!("/api/proposals/{id}/decision"),
                    &json!({"decision": "approve"})
                )
                .status,
            404,
            "{id}"
        );
    }
    Ok(())
}

// ---- conversations -------------------------------------------------------------------------------

#[test]
fn conversations_are_the_signed_in_persons_own() -> TestResult {
    let f = fixture()?;
    let list = f.session.get("/api/conversations").json();
    let titles: Vec<&str> = list
        .as_array()
        .ok_or("list")?
        .iter()
        .filter_map(|c| c["title"].as_str())
        .collect();
    assert_eq!(
        titles,
        ["acme chat"],
        "no conversation of the second person or of the rival"
    );
    let created = f.session.post(
        "/api/conversations",
        &json!({"title": "Disk \u{1b}[2J space"}),
    );
    assert_eq!(created.status, 201);
    let body = created.json();
    assert!(body["id"].is_string());
    assert!(
        !body["title"]
            .as_str()
            .unwrap_or_default()
            .contains('\u{1b}')
    );
    assert_eq!(body["archived"], false);
    // No body, and no title: the default title is localized.
    let en = f.session.post_empty("/api/conversations").json();
    assert_eq!(en["title"], "New conversation");
    let id = f
        .session
        .post("/api/conversations?lang=id", &json!({}))
        .json();
    assert_eq!(id["title"], "Percakapan baru");
    for bad in [
        json!({"title": 5}),
        json!({"title": "x".repeat(201)}),
        json!({"other": 1}),
        json!([]),
    ] {
        assert_eq!(
            f.session.post("/api/conversations", &bad).status,
            400,
            "{bad}"
        );
    }
    let messages = f.session.get(&format!(
        "/api/conversations/{}/messages",
        f.mine.conversation
    ));
    assert_eq!(messages.status, 200);
    assert_eq!(messages.json(), json!([]));
    Ok(())
}

#[test]
fn another_persons_or_orgs_conversation_is_a_missing_one() -> TestResult {
    let f = fixture()?;
    let second_chat = f.world.conn()?.query_row(
        "SELECT id FROM conversations WHERE title = 'second''s chat'",
        [],
        |row| row.get::<_, String>(0),
    )?;
    for id in [
        f.theirs.conversation.as_str(),
        second_chat.as_str(),
        "nope",
        "00000000-0000-4000-8000-000000000000",
    ] {
        let r = f.session.get(&format!("/api/conversations/{id}/messages"));
        assert_eq!((r.status, r.code().as_str()), (404, "not_found"), "{id}");
    }
    Ok(())
}

// ---- isolation, in one place ---------------------------------------------------------------------

#[test]
fn nothing_of_the_rival_organization_is_visible_anywhere() -> TestResult {
    let f = fixture()?;
    let mut all = String::new();
    for path in [
        "/api/nodes",
        "/api/nodes/old/inventory",
        "/api/findings",
        "/api/plans",
        "/api/transports",
        "/api/providers",
        "/api/memories",
        "/api/proposals",
        "/api/conversations",
        "/api/session",
    ] {
        all.push_str(&f.session.get(path).text());
    }
    for marker in ["rival", "Rival", "boss@rival.test"] {
        assert!(!all.contains(marker), "{marker} leaked");
    }
    // Its node name resolves only inside its own org, where `old` is a different node.
    assert_eq!(f.session.get("/api/nodes/rivalonly/inventory").status, 404);
    // Sanity: the rival's data does exist in the database.
    let theirs: i64 = f.world.conn()?.query_row(
        "SELECT COUNT(*) FROM nodes WHERE org_id = ?1",
        [f.world.rival.as_str()],
        |r| r.get(0),
    )?;
    assert_eq!(theirs, 3);
    drop(f.server);
    Ok(())
}
