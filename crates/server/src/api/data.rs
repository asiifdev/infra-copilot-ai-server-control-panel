//! Read-only views: nodes, inventories, findings, plans and transports.
//!
//! Everything here comes from servers (names, keys, values in inventories) or from people (plan
//! names): it is data. It is returned as JSON strings for the UI to show as text, secret-looking
//! values are redacted, and control characters are replaced.

use super::{Reply, json_ok, with_db};
use crate::app::App;
use crate::errors::{INVALID_INPUT, NOT_FOUND, STORE_FAILED, from_db};
use crate::guard::Lang;
use crate::util::{clean_text, json_object, limit_param, query_get, redact_json};
use agent::{
    DbPlanStore, DbStore, DefaultPlanStepRunner, PlanRunError, PlanStore, ReadStore, StoreError,
    approve_plan, gates_for_stored, run_plan,
};
use axum::Extension;
use axum::body::Bytes;
use axum::extract::{Path, RawQuery, State};
use db::ItemKind;
use inventory::{Finding, Severity};
use serde_json::{Map, Value, json};
use std::sync::Arc;

fn kind_value(kind: inventory::Kind) -> Value {
    serde_json::to_value(kind).unwrap_or(Value::Null)
}

fn subject(kind: inventory::Kind, key: &str) -> Value {
    json!({"kind": kind_value(kind), "key": clean_text(key)})
}

fn store_error(error: StoreError) -> crate::errors::ApiError {
    match error {
        StoreError::NotFound => NOT_FOUND,
        StoreError::Unavailable => STORE_FAILED,
    }
}

/// `GET /api/nodes`.
pub(crate) async fn nodes(State(app): State<Arc<App>>) -> Reply {
    let list = with_db(&app, |conn, app| {
        let org = &app.scope.org;
        let mut out = Vec::new();
        for node in db::list_nodes(conn, org).map_err(|e| from_db(&e))? {
            let latest = match db::latest_inventory(conn, org, &node.id) {
                Ok(inventory) => {
                    let counts = db::count_inventory_items(conn, org, &inventory.id)
                        .map_err(|e| from_db(&e))?;
                    let count = |kind: ItemKind| {
                        counts
                            .iter()
                            .find(|(k, _)| *k == kind)
                            .map_or(0, |(_, n)| *n)
                    };
                    json!({
                        "taken_at": inventory.taken_at,
                        "status": inventory.status.as_str(),
                        "counts": {
                            "listener": count(ItemKind::Listener),
                            "process": count(ItemKind::Process),
                            "cron": count(ItemKind::Cron),
                            "env_ref": count(ItemKind::EnvRef),
                            "vhost": count(ItemKind::Vhost),
                            "cert": count(ItemKind::Cert),
                            "database": count(ItemKind::Database),
                            "dns": count(ItemKind::Dns),
                            "check": count(ItemKind::Check),
                        },
                    })
                }
                Err(db::Error::NotFound) => Value::Null,
                Err(error) => return Err(from_db(&error)),
            };
            out.push(json!({
                "name": clean_text(&node.name),
                "kind": node.kind.as_str(),
                "status": node.status.as_str(),
                "latest_inventory": latest,
            }));
        }
        Ok(Value::Array(out))
    })
    .await?;
    Ok(json_ok(&list))
}

/// `GET /api/nodes/{name}/inventory?kind=&limit=`.
pub(crate) async fn inventory(
    State(app): State<Arc<App>>,
    Path(name): Path<String>,
    RawQuery(query): RawQuery,
) -> Reply {
    let kind = match query_get(query.as_deref(), "kind") {
        None => None,
        Some(text) => Some(ItemKind::parse(&text).map_err(|_| INVALID_INPUT)?),
    };
    let limit = limit_param(query.as_deref(), "limit", 500, 5000).map_err(|()| INVALID_INPUT)?;
    let body = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let node = db::list_nodes(conn, org)
            .map_err(|e| from_db(&e))?
            .into_iter()
            .find(|n| n.name == name)
            .ok_or(NOT_FOUND)?;
        let inventory = db::latest_inventory(conn, org, &node.id).map_err(|e| from_db(&e))?;
        let rows = db::list_inventory_items(conn, org, &inventory.id, kind, limit)
            .map_err(|e| from_db(&e))?;
        let mut items = Vec::with_capacity(rows.len());
        for row in rows {
            let mut data: Value = serde_json::from_str(&row.data_json).unwrap_or(Value::Null);
            redact_json(&mut data);
            items.push(json!({
                "kind": row.kind.as_str(),
                "key": core_domain::redact::redact(&clean_text(&row.key)).text,
                "data": data,
            }));
        }
        let mut warnings: Value =
            serde_json::from_str(&inventory.warnings_json).unwrap_or_else(|_| json!([]));
        redact_json(&mut warnings);
        Ok(json!({
            "taken_at": inventory.taken_at,
            "status": inventory.status.as_str(),
            "warnings": warnings,
            "items": items,
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

fn severity_of(text: &str) -> Option<Severity> {
    match text {
        "info" => Some(Severity::Info),
        "important" => Some(Severity::Important),
        "critical" => Some(Severity::Critical),
        _ => None,
    }
}

/// A finding as the UI shows it: the text rendered in `locale`, values from the inventories
/// cleaned first (they are untrusted).
fn finding_json(original: &Finding, catalog: &i18n::Catalog, locale: i18n::Locale) -> Value {
    let finding = parity::cleaned(original, &clean_text);
    let rendered = parity::render(&finding, catalog, locale);
    json!({
        "rule_id": finding.rule_id,
        "severity": serde_json::to_value(finding.severity).unwrap_or(Value::Null),
        "subject": subject(finding.subject.kind, &finding.subject.key),
        "title": rendered.title,
        "explanation": rendered.explanation,
        "advice": rendered.advice,
        "note": rendered.note,
        "evidence": finding
            .evidence
            .iter()
            .map(|e| subject(e.kind, &e.key))
            .collect::<Vec<_>>(),
    })
}

/// `GET /api/findings?min_severity=&limit=`: the findings of the latest comparison.
pub(crate) async fn findings(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    RawQuery(query): RawQuery,
) -> Reply {
    let min = match query_get(query.as_deref(), "min_severity") {
        None => None,
        Some(text) => Some(severity_of(&text).ok_or(INVALID_INPUT)?),
    };
    let limit = limit_param(query.as_deref(), "limit", 200, 1000).map_err(|()| INVALID_INPUT)?;
    let body = with_db(&app, move |conn, app| {
        match DbStore::new(conn).latest_findings(&app.scope.org, min, limit) {
            Ok(view) => Ok(json!({
                "comparison": {
                    "source": clean_text(&view.source_node),
                    "target": clean_text(&view.target_node),
                    "created_at": view.created_at,
                },
                "findings": view
                    .findings
                    .iter()
                    .map(|f| finding_json(f, &app.catalog, lang.0))
                    .collect::<Vec<_>>(),
            })),
            Err(StoreError::NotFound) => Ok(json!({"comparison": null, "findings": []})),
            Err(error) => Err(store_error(error)),
        }
    })
    .await?;
    Ok(json_ok(&body))
}

/// `GET /api/plans`.
pub(crate) async fn plans(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, |conn, app| {
        let list = DbPlanStore::new(conn)
            .plans(&app.scope.org)
            .map_err(store_error)?;
        Ok(Value::Array(
            list.iter()
                .map(|p| {
                    json!({
                        "id": p.id,
                        "name": clean_text(&p.name),
                        "status": p.status,
                        "source_node": clean_text(&p.source_node),
                        "target_node": clean_text(&p.target_node),
                        "created_at": p.created_at,
                    })
                })
                .collect(),
        ))
    })
    .await?;
    Ok(json_ok(&body))
}

fn key_json(key: &plan::FindingKey) -> Value {
    json!({"rule_id": key.rule_id, "subject": subject(key.subject.kind, &key.subject.key)})
}

/// `GET /api/plans/{name}`: the stored plan, its gates and the text of the plan.
pub(crate) async fn plan_detail(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    Path(name): Path<String>,
) -> Reply {
    let body = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let plans = DbPlanStore::new(conn);
        let stored = plans.plan(org, &name).map_err(store_error)?;
        let (gates, evidence) =
            gates_for_stored(&plans, &app.catalog, lang.0, org, &stored).map_err(store_error)?;
        let rendered = plan::render_plan(
            &stored.plan,
            &gates,
            &stored.plan_names(),
            &app.catalog,
            lang.0,
        );
        let summary = &stored.summary;
        let mut gate_map = Map::new();
        gate_map.insert("cutover_allowed".to_owned(), json!(gates.cutover_allowed()));
        // Nothing is verified after cutover in this milestone: the shutdown gate stays closed.
        gate_map.insert(
            "shutdown_allowed".to_owned(),
            json!(gates.decommission_allowed(false)),
        );
        gate_map.insert(
            "inventory_incomplete".to_owned(),
            json!(gates.inventory_incomplete),
        );
        gate_map.insert(
            "comparison_current".to_owned(),
            json!(evidence.is_current()),
        );
        gate_map.insert(
            "open_critical".to_owned(),
            Value::Array(gates.open_critical.iter().map(key_json).collect()),
        );
        gate_map.insert(
            "accepted_critical".to_owned(),
            Value::Array(
                gates
                    .accepted_critical
                    .iter()
                    .map(|a| {
                        let mut entry = key_json(&a.key);
                        if let Value::Object(map) = &mut entry {
                            map.insert("reason".to_owned(), json!(clean_text(&a.reason)));
                        }
                        entry
                    })
                    .collect(),
            ),
        );
        gate_map.insert("important".to_owned(), json!(gates.important));
        gate_map.insert("info".to_owned(), json!(gates.info));
        let plan_id = db::PlanId::parse(&summary.id).map_err(|_| STORE_FAILED)?;
        let step_rows = db::list_plan_steps(conn, org, &plan_id).map_err(|e| from_db(&e))?;
        let status_by_seq: std::collections::BTreeMap<u32, &'static str> = step_rows
            .into_iter()
            .map(|r| (r.seq, r.status.as_str()))
            .collect();
        let steps_json: Vec<Value> = stored
            .plan
            .steps()
            .iter()
            .map(|s| {
                let mut val = serde_json::to_value(s).unwrap_or(Value::Null);
                if let Value::Object(ref mut obj) = val {
                    obj.insert(
                        "status".to_owned(),
                        json!(status_by_seq.get(&s.seq).copied().unwrap_or("pending")),
                    );
                }
                val
            })
            .collect();
        Ok(json!({
            "id": summary.id,
            "name": clean_text(&summary.name),
            "status": summary.status,
            "source_node": clean_text(&summary.source_node),
            "target_node": clean_text(&summary.target_node),
            "created_at": summary.created_at,
            "spec": serde_json::to_value(stored.plan.spec()).unwrap_or(Value::Null),
            "steps": steps_json,
            "rendered": rendered,
            "gates": Value::Object(gate_map),
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// Keeps numbers, booleans and nulls of a stored check result and drops every string: the result
/// holds numbers and booleans by construction, and this makes sure no text reaches the UI.
fn numbers_only(value: Value) -> Value {
    match value {
        Value::Number(_) | Value::Bool(_) | Value::Null => value,
        Value::String(_) => Value::Null,
        Value::Array(items) => Value::Array(items.into_iter().map(numbers_only).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .filter(|(_, v)| !v.is_string())
                .map(|(k, v)| (k, numbers_only(v)))
                .collect(),
        ),
    }
}

/// `GET /api/transports`: name, kind and verification state; never the settings or the sealed
/// credentials.
pub(crate) async fn transports(State(app): State<Arc<App>>) -> Reply {
    let body = with_db(&app, |conn, app| {
        let rows = db::list_transports(conn, &app.scope.org).map_err(|e| from_db(&e))?;
        Ok(Value::Array(
            rows.into_iter()
                .map(|row| {
                    let last_check = row
                        .last_check_json
                        .as_deref()
                        .and_then(|text| serde_json::from_str::<Value>(text).ok())
                        .map_or(Value::Null, numbers_only);
                    json!({
                        "id": row.id.as_str(),
                        "name": clean_text(&row.name),
                        "kind": row.kind,
                        "verified_at": row.verified_at,
                        "last_check": last_check,
                    })
                })
                .collect(),
        ))
    })
    .await?;
    Ok(json_ok(&body))
}

fn plan_run_error(err: PlanRunError) -> crate::errors::ApiError {
    match err {
        PlanRunError::NotFound => NOT_FOUND,
        PlanRunError::NotApprovable => crate::errors::ApiError::new("plan_not_approvable"),
        PlanRunError::NotApproved => crate::errors::ApiError::new("plan_not_approved"),
        PlanRunError::GateBlocked(_) => crate::errors::ApiError::new("plan_gate_blocked"),
        PlanRunError::Store(e) => store_error(e),
    }
}

/// `POST /api/plans/{id}/approve`.
pub(crate) async fn plan_approve(State(app): State<Arc<App>>, Path(id): Path<String>) -> Reply {
    let body = with_db(&app, move |conn, app| {
        let stored = approve_plan(
            conn,
            &app.scope.org,
            &id,
            db::ActorKind::User,
            Some(app.scope.user.as_str()),
        )
        .map_err(plan_run_error)?;
        Ok(json!({
            "id": stored.summary.id,
            "name": clean_text(&stored.summary.name),
            "status": stored.summary.status,
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/plans/{id}/run` (requires `--enable-execution`).
pub(crate) async fn plan_run(
    State(app): State<Arc<App>>,
    Extension(lang): Extension<Lang>,
    Path(id): Path<String>,
) -> Reply {
    if app.executor.is_none() {
        return Err(crate::errors::ApiError::new("execution_disabled"));
    }
    let body = with_db(&app, move |conn, app| {
        let runner = DefaultPlanStepRunner;
        let report = run_plan(
            conn,
            &app.catalog,
            lang.0,
            &app.scope.org,
            &id,
            false,
            &runner,
            db::ActorKind::User,
            Some(app.scope.user.as_str()),
        )
        .map_err(plan_run_error)?;
        Ok(json!({
            "id": report.plan_id,
            "name": clean_text(&report.name),
            "status": report.status,
            "steps": report.steps.iter().map(|s| json!({
                "seq": s.seq,
                "kind": s.kind,
                "status": s.status,
                "reason": s.reason,
            })).collect::<Vec<_>>(),
        }))
    })
    .await?;
    Ok(json_ok(&body))
}

/// `POST /api/transports/{id}/credentials {"credentials": {"key": "value", ...}}`: seals new
/// credentials in the vault, resets verification state, and records an audit entry without secrets.
pub(crate) async fn transport_credentials(
    State(app): State<Arc<App>>,
    Path(id): Path<String>,
    body: Bytes,
) -> Reply {
    let Some(vault) = app.vault.clone() else {
        return Err(crate::errors::ApiError::new("master_key_missing"));
    };
    let map = json_object(&body, false).ok_or(INVALID_INPUT)?;
    if map.len() != 1 {
        return Err(INVALID_INPUT);
    }
    let Some(Value::Object(creds_obj)) = map.get("credentials") else {
        return Err(INVALID_INPUT);
    };
    let mut pairs = Vec::with_capacity(creds_obj.len());
    for (k, v) in creds_obj {
        let Some(val_str) = v.as_str() else {
            return Err(INVALID_INPUT);
        };
        pairs.push((k.clone(), transport::Secret::new(val_str.to_owned())));
    }
    let creds = transport::Credentials::new(pairs)
        .map_err(|_| crate::errors::ApiError::new("bad_credentials"))?;
    if creds.is_empty() {
        return Err(crate::errors::ApiError::new("bad_credentials"));
    }

    let resp = with_db(&app, move |conn, app| {
        let org = &app.scope.org;
        let row = if let Ok(tid) = db::TransportId::parse(&id) {
            db::get_transport(conn, org, &tid).map_err(|e| from_db(&e))?
        } else {
            db::list_transports(conn, org)
                .map_err(|e| from_db(&e))?
                .into_iter()
                .find(|t| t.name == id)
                .ok_or(NOT_FOUND)?
        };
        transport::store::update_credentials(conn, &vault, org, &row.id, &creds).map_err(|e| {
            match e {
                transport::store::StoreError::NotFound => NOT_FOUND,
                transport::store::StoreError::CredentialsNotAccepted => {
                    crate::errors::ApiError::new("credentials_not_accepted")
                }
                transport::store::StoreError::CorruptCredentials => {
                    crate::errors::ApiError::new("bad_credentials")
                }
                _ => STORE_FAILED,
            }
        })?;
        let subject = format!(
            r#"{{"transport":"{}","name":{}}}"#,
            row.id.as_str(),
            serde_json::to_string(&row.name).unwrap_or_else(|_| "\"\"".to_owned())
        );
        db::append_audit(
            conn,
            org,
            &db::NewAuditEntry {
                actor_kind: db::ActorKind::User,
                actor_id: Some(app.scope.user.as_str()),
                action: "transport.credentials.update",
                node_id: None,
                subject_json: &subject,
                result: db::AuditOutcome::Success,
            },
        )
        .map_err(|e| from_db(&e))?;
        Ok(json!({
            "id": row.id.as_str(),
            "name": clean_text(&row.name),
            "verified_at": Value::Null,
        }))
    })
    .await?;
    Ok(json_ok(&resp))
}

#[cfg(test)]
mod tests {
    use super::numbers_only;
    use serde_json::json;

    #[test]
    fn check_results_lose_every_string() {
        let value = json!({"free": 5, "ok": true, "note": "secret", "nested": {"a": "x", "b": 1}, "l": ["x", 2]});
        assert_eq!(
            numbers_only(value),
            json!({"free": 5, "ok": true, "nested": {"b": 1}, "l": [null, 2]})
        );
    }
}
