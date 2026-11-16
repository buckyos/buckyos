//! DID Object Provider of AI Workspace (notepads/done/agent-object-url-todo.md §4.4, D7; 核心 API 调整 R1):
//! `obj://aiworkspace/<ws_id>[?op=ls|find|grep&at=&…]`, `.../entities/<entity_id>[?mode=&selector=&version=]`
//! and `.../path/<...>`, served at `/kapi/aiworkspace/obj/...`. Reads are the kRPC
//! methods (`ws.get_info`, `doc.index`, `doc.read`, `doc.resolve`, ...) through the
//! service's own kRPC entry with the caller's credentials, so sessions, run tokens,
//! guest tokens and Workspace grants decide exactly as for kRPC. So do the actions (AI Native 余下任务 C1, R43): a
//! wish's stages and a Source's refresh are `proc.start`, authorized as that method, with no approval gate of their own
//! (Q11); their `risk` says what kind of call it is, and whoever calls decides whether to ask first.

use std::net::IpAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use aiworkspace_core::model::TYPE_WISH;
use aiworkspace_core::wish::STAGES;
use aiworkspace_store::source::SOURCE_PROGRAM;
use aiworkspace_store::wish::runs::PROGRAM_XLLM;
use aiworkspace_store::workspace::random_id;
use buckyos_api::{ActionRisk, Captures, KrpcObjectProvider, ObjectKind, ProviderRpc, RpcBinding, AIWORKSPACE_HTTP_PATH};
use buckyos_http_server::{DIDObjectError, DIDObjectHttpServer, HttpServer, StreamInfo};
use http_body_util::BodyExt;
use kRPC::RPCErrors;
use name_lib::DID;
use serde_json::{json, Value};

use crate::AppState;

const MAX_OBJECT_REQUEST_BYTES: usize = 1024 * 1024;

pub fn obj_path() -> String {
    format!("{AIWORKSPACE_HTTP_PATH}/obj")
}

struct KrpcEntry {
    state: Arc<AppState>,
}

#[async_trait::async_trait]
impl ProviderRpc for KrpcEntry {
    async fn call(&self, method: &str, params: Value, token: Option<String>, _ip: Option<IpAddr>) -> Result<Value, RPCErrors> {
        let body = json!({ "method": method, "params": params, "sys": [0, token] });
        let response = crate::krpc(State(self.state.clone()), bytes::Bytes::from(body.to_string())).await;
        let bytes = response
            .into_body()
            .collect()
            .await
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?
            .to_bytes();
        let reply: Value = serde_json::from_slice(&bytes).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        if let Some(error) = reply.get("error") {
            return Err(RPCErrors::NoPermission(error.as_str().map(str::to_string).unwrap_or_else(|| error.to_string())));
        }
        let result = reply.get("result").cloned().unwrap_or(Value::Null);
        if result.get("ok") == Some(&Value::Bool(false)) {
            let detail = result["error"].to_string();
            // the HTTP status follows the code: a version that is not kept is not found (as the source routes answer),
            // a request the service cannot serve yet (a fixed version of meta / body) is the caller's to change
            return Err(match result["error"]["code"].as_str() {
                Some("PERMISSION_DENIED") => RPCErrors::NoPermission(detail),
                Some("NOT_FOUND") | Some("TARGET_DELETED") | Some("VERSION_NOT_RETAINED") | Some("REFERENCE_BROKEN") => RPCErrors::KeyNotExist(detail),
                Some("INVALID_OPERATION") | Some("INVALID_SCHEMA") | Some("UNSUPPORTED_VERSION") => RPCErrors::ParseRequestError(detail),
                _ => RPCErrors::ReasonError(detail),
            });
        }
        Ok(result)
    }
}

fn cap(c: &Captures, name: &str) -> String {
    c.get(name).cloned().unwrap_or_default()
}

fn json_param(c: &Captures, key: &str) -> Result<Option<Value>, DIDObjectError> {
    match c.get(&format!("?{key}")) {
        Some(v) => serde_json::from_str(v).map(Some).map_err(|e| DIDObjectError::new("invalid_params", format!("{key}: {e}"))),
        None => Ok(None),
    }
}

/// `selector` / `version` of the request query (canonical JSON). A fixed version (`mode=fixed&version=`) is read or
/// fails, never read as the head (核心 API 调整 R2).
fn read_params(c: &Captures) -> Result<Value, DIDObjectError> {
    let mut params = json!({ "workspace_id": cap(c, "ws"), "entity_id": cap(c, "entity") });
    if let Some(selector) = json_param(c, "selector")? {
        params["selector"] = selector;
    }
    match (c.get("?mode").map(String::as_str), json_param(c, "version")?) {
        (Some("fixed"), Some(version)) => params["version"] = version,
        (Some("fixed"), None) => return Err(DIDObjectError::new("invalid_params", "mode=fixed needs version")),
        (_, Some(_)) => return Err(DIDObjectError::new("invalid_params", "version needs mode=fixed")),
        _ => {}
    }
    Ok(params)
}

/// `meta` / `body` of an entity url: the head (a fixed version is refused by the service).
fn as_params(c: &Captures, what: &str) -> Result<Value, DIDObjectError> {
    let mut params = read_params(c)?;
    params["as"] = json!(what);
    if what == "body" {
        if let Some(v) = c.get("?cursor") {
            params["cursor"] = json!(v);
        }
        if let Some(v) = c.get("?max_bytes") {
            params["max_bytes"] = json!(v.parse::<u64>().map_err(|e| DIDObjectError::new("invalid_params", format!("max_bytes: {e}")))?);
        }
        if c.get("?parts").is_some_and(|v| v == "true" || v == "1") {
            params["parts"] = json!(true);
        }
    }
    Ok(params)
}

/// The query of the Workspace index (核心 API 调整 R1; design draft §8): the url's query parameters, overridden by
/// the `query` object of the action; `page_size` and `cursor` of the action page it.
fn index_params(c: &Captures, input: &Value) -> Result<Value, DIDObjectError> {
    let mut params = json!({ "workspace_id": cap(c, "ws") });
    for key in ["op", "at", "type", "not_type", "schema", "depth", "name", "text", "case"] {
        if let Some(v) = c.get(&format!("?{key}")) {
            params[key] = match key {
                "depth" => json!(v.parse::<u64>().map_err(|e| DIDObjectError::new("invalid_params", format!("depth: {e}")))?),
                "case" => json!(v == "true" || v == "1"),
                _ => json!(v),
            };
        }
    }
    if let Some(q) = input.get("query").and_then(Value::as_object) {
        for (k, v) in q {
            if !matches!(k.as_str(), "workspace_id" | "limit" | "cursor") {
                params[k] = v.clone();
            }
        }
    }
    if let Some(n) = input.get("page_size").and_then(Value::as_u64) {
        params["limit"] = json!(n);
    }
    if let Some(cursor) = input.get("cursor").and_then(Value::as_str) {
        params["cursor"] = json!(cursor);
    }
    Ok(params)
}

/// `doc.index` answers (list / find items or text hits) as index rows.
fn index_rows(c: &Captures, page: Value) -> Value {
    let ws = cap(c, "ws");
    let list = page.get("items").or_else(|| page.get("hits")).and_then(Value::as_array).cloned().unwrap_or_default();
    let rows: Vec<Value> = list
        .into_iter()
        .filter_map(|item| {
            let id = item.get("entity_id")?.as_str()?.to_string();
            let mut reference = json!({ "workspace_id": ws, "entity_id": id, "mode": "live" });
            if let Some(sel) = item.get("selector").filter(|s| !s.is_null()) {
                reference["selector"] = sel.clone();
            }
            let row_id = match item.get("selector").filter(|s| !s.is_null()) {
                Some(sel) => format!("{id}#{}", buckyos_api::canonical_json(sel)),
                None => match item.get("on").and_then(Value::as_str) {
                    Some(on) => format!("{id}#{on}"),
                    None => id.clone(),
                },
            };
            let parent = item.get("path").and_then(Value::as_str).and_then(|p| p.rsplit_once('/')).map(|(p, _)| if p.is_empty() { "/".to_string() } else { p.to_string() });
            Some(json!({
                "row_id": row_id,
                "object": buckyos_api::aiws_reference_url(&reference, None).ok()?,
                "cells": {
                    "type": item.get("type").cloned().unwrap_or(Value::Null), "kind": item.get("kind").cloned().unwrap_or(Value::Null),
                    "name": item.get("name").cloned().unwrap_or(Value::Null), "title": item.get("title").cloned().unwrap_or(Value::Null),
                    "path": item.get("path").cloned().unwrap_or(Value::Null), "parent": parent,
                    "on": item.get("on").cloned().unwrap_or(Value::Null), "selector": item.get("selector").cloned().unwrap_or(Value::Null),
                    "snippet": item.get("snippet").cloned().unwrap_or(Value::Null),
                },
            }))
        })
        .collect();
    let mut out = json!({ "rows": rows });
    for k in ["next_cursor", "coverage", "complete", "at", "head_seq"] {
        if let Some(v) = page.get(k) {
            out[k] = v.clone();
        }
    }
    out
}

/// `ws.list` as index rows: one per visible workspace, linking to its object.
fn workspace_rows(page: Value) -> Value {
    let rows: Vec<Value> = page["workspaces"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|ws| {
            let id = ws.get("workspace_id")?.as_str()?.to_string();
            Some(json!({
                "row_id": id,
                "object": buckyos_api::obj_service_url("aiworkspace", &[id.as_str()]),
                "cells": { "title": ws.get("title").cloned().unwrap_or(Value::Null), "head_seq": ws.get("head_seq").cloned().unwrap_or(Value::Null) },
            }))
        })
        .collect();
    json!({ "rows": rows })
}

/// How a kRPC method is authorized: the methods table's words (an action is judged exactly as its method).
fn authorization(method: &str) -> String {
    crate::methods::spec(method).map(|s| s.checks.iter().map(|c| c.describe()).collect::<Vec<_>>().join("; ")).unwrap_or_default()
}

/// `proc.start` of `program` on the entity of the url: the action's input is the program's `params` (what the url
/// names wins), its `idempotency_key` the call's (a new run each call without one).
fn proc_start(program: &'static str, named: impl Fn(&Captures) -> Value + Send + Sync + 'static) -> RpcBinding {
    RpcBinding::new("proc.start", move |c, input| {
        let mut params = input.as_object().cloned().unwrap_or_default();
        let key = params.remove("idempotency_key").and_then(|k| k.as_str().map(str::to_string)).unwrap_or_else(|| random_id("xcall/"));
        if let Value::Object(m) = named(c) {
            params.extend(m);
        }
        Ok(json!({ "workspace_id": cap(c, "ws"), "program": program, "params": params, "idempotency_key": key }))
    })
}

/// The actions of an entity (declared by the entities they apply to, from their `meta`): a wish's three stages, the
/// refresh of an entity a Source writes.
fn entity_actions(mut entity: ObjectKind) -> ObjectKind {
    let auth = authorization("proc.start");
    let key = json!({ "type": "string", "description": "the same key with the same input answers the same run (a retry starts nothing new)" });
    let stage_input = json!({ "type": "object", "properties": {
        "execution": { "type": "object", "properties": { "mode": { "type": "string", "enum": ["local", "hosted"] } },
                       "description": "who executes it: local (the caller; the answer carries its claim_token), hosted (the owner's configured Agent); the service's default when absent" },
        "request": { "type": "object", "description": "fixed parameters of the run (today, timezone)" },
        "location": { "type": "object", "description": "where the task is: cell_id, surface_id, selection, viewport" },
        "idempotency_key": key } });
    for &stage in STAGES {
        let what = match stage {
            "analyze" => "settles the task contract (inputs, output contract, checks) and writes the analysis into the wish",
            "code" => "writes the wish's macro and trial-runs it; only the macro is written",
            _ => "runs the stored macro on current data and writes its results (results a person edited are overwritten)",
        };
        let description = format!(
            "Start the {stage} stage of this wish: `proc.start {{ program: \"{PROGRAM_XLLM}\", params: {{ wish_id, stage: \"{stage}\", …input }} }}`. \
             When it ends the service {what}, in one Commit, and adds a record to the wish's history (`proc.rollback` takes it back). \
             Authorized as that kRPC method: {auth}. Answers the run; follow it with `proc.get`."
        );
        entity = entity
            .action(stage, &description, stage_input.clone(), ActionRisk::Write, "write",
                    proc_start(PROGRAM_XLLM, move |c| json!({ "wish_id": cap(c, "entity"), "stage": stage })))
            .only_if(|meta| meta["type_id"] == TYPE_WISH);
    }
    entity
        .action(
            "refresh",
            &format!(
                "Refresh this entity from its Source: `proc.start {{ program: \"{SOURCE_PROGRAM}\", params: {{ entity_id }} }}`. The run writes what the \
                 Source holds now in one Commit; a failure keeps the last valid version. Authorized as that kRPC method: {auth}. Answers the run; \
                 follow it with `proc.get`."
            ),
            json!({ "type": "object", "properties": { "idempotency_key": key } }),
            ActionRisk::IdempotentWrite,
            "write",
            proc_start(SOURCE_PROGRAM, |c| json!({ "entity_id": cap(c, "entity") })),
        )
        .only_if(|meta| meta["data"]["write"] == "refresh" && meta["data"]["source"]["kind"] != "session")
}

pub fn object_kinds() -> Vec<ObjectKind> {
    let object = json!({ "type": "object" });
    let list = || RpcBinding::new("ws.list", |_, _| Ok(json!({}))).map(|_, v| workspace_rows(v));
    let root = ObjectKind::new("", "aiworkspace.root", "AI Workspaces", "The AI Workspaces the caller can see. Rows link to the workspace objects.")
        .summary_property("workspaces", "The visible workspaces", object.clone(), list())
        .property("controls", "The Block-controls a canvas offers: what each is and when to use it (control.list; one with the self-description \
            to follow: control.describe { kind })", object.clone(), RpcBinding::new("control.list", |_, _| Ok(json!({}))))
        .index("List the visible workspaces", json!({ "title": "string", "head_seq": "integer" }), list());
    let workspace = ObjectKind::new(
        "{ws}",
        "aiworkspace.workspace",
        "AI Workspace",
        "A workspace as a directory of entities. The index lists (`op=ls`, default), finds by name or title (`op=find&name=`) or \
         looks for a literal in the bodies (`op=grep&text=`); `at` (a path or entity id), `type` / `not_type` (short types, comma \
         separated), `schema`, `depth` narrow it. Rows link to the entity objects (a body hit with its selector).",
    )
    .summary_property("info", "Workspace info (title, owner, head, publications)", object.clone(), RpcBinding::new("ws.get_info", |c, _| Ok(json!({ "workspace_id": cap(c, "ws") }))))
    .property("outline", "The whole readable entity tree in one answer (doc.outline)", object.clone(), RpcBinding::new("doc.outline", |c, _| Ok(json!({ "workspace_id": cap(c, "ws") }))))
    .index(
        "ls / find / grep under a node, paged",
        json!({ "type": "string", "kind": "string", "name": "string", "title": "string", "path": "string", "parent": "string", "on": "string", "selector": "object", "snippet": "string" }),
        RpcBinding::new("doc.index", index_params).map(index_rows),
    )
    .event("head", "A new head was committed")
    .event("locks", "The lock table changed");
    let entity = entity_actions(ObjectKind::new(
        "{ws}/entities/{entity}",
        "aiworkspace.entity",
        "AI Workspace entity",
        "One entity of a workspace. `meta`: what it is (type, path, versions, body state, capabilities, operations); `body`: \
         its plain-text form, paged (`?cursor=&max_bytes=&parts=true`, `?selector=` for one part); `content`: the native \
         content (`?selector=<canonical json>`; `?mode=fixed&version=<json>` reads that version or fails).",
    )
    .summary_property("meta", "What the entity is: type, path, versions, body state, capabilities, operations", object.clone(), RpcBinding::new("doc.read", |c, _| as_params(c, "meta")))
    .property("body", "The plain-text form of the entity, paged", object.clone(), RpcBinding::new("doc.read", |c, _| as_params(c, "body")))
    .property("content", "The native content (with the selector of the url, if any; a fixed version when the url names one)", object.clone(), RpcBinding::new("doc.read", |c, _| read_params(c)))
    .property("relations", "What the entity references, is referenced by, was generated from", object.clone(),
        RpcBinding::new("doc.relations", |c, _| Ok(json!({ "workspace_id": cap(c, "ws"), "entity_id": cap(c, "entity") }))))
    .property("freshness", "Dependency-based freshness", object.clone(),
        RpcBinding::new("doc.freshness", |c, _| Ok(json!({ "workspace_id": cap(c, "ws"), "entity_ids": [cap(c, "entity")] }))))
    .property("versions", "Addressable versions, newest first", object.clone(),
        RpcBinding::new("doc.list_versions", |c, _| Ok(json!({ "workspace_id": cap(c, "ws"), "entity_id": cap(c, "entity") }))))
    .property("reference", "The reference this url projects (doc.resolve)", object.clone(),
        RpcBinding::new("doc.resolve", |c, _| {
            let mut reference = json!({ "workspace_id": cap(c, "ws"), "entity_id": cap(c, "entity"), "mode": c.get("?mode").cloned().unwrap_or_else(|| "live".into()) });
            for key in ["selector", "version"] {
                if let Some(v) = c.get(&format!("?{key}")) {
                    reference[key] = serde_json::from_str(v).map_err(|e| DIDObjectError::new("invalid_params", format!("{key}: {e}")))?;
                }
            }
            Ok(json!({ "workspace_id": cap(c, "ws"), "reference": reference }))
        })));
    let by_path = ObjectKind::new(
        "{ws}/path/{*path}",
        "aiworkspace.path",
        "AI Workspace path",
        "Finds an entity by its path in the tree. A path can be rebound; the entity url (`resolved`) is the stable identity.",
    )
    .summary_property("resolved", "The entity at this path", object, RpcBinding::new("doc.resolve", |c, _| {
        Ok(json!({ "workspace_id": cap(c, "ws"), "path": format!("/{}", cap(c, "path")) }))
    }).map(|c, v| {
        let mut v = v;
        let ws = cap(c, "ws");
        let object = buckyos_api::aiws_reference_url(&v["canonical"], Some(&ws))
            .or_else(|_| buckyos_api::aiws_reference_url(&v["reference"], Some(&ws)));
        if let Ok(object) = object {
            v["object"] = json!(object);
        }
        v
    }));
    vec![root, workspace, entity, by_path]
}

pub fn provider(state: Arc<AppState>, zone_did: DID) -> DIDObjectHttpServer<KrpcObjectProvider> {
    KrpcObjectProvider::new("aiworkspace", zone_did, object_kinds(), Arc::new(KrpcEntry { state })).into_http_server()
}

/// axum handler of `/kapi/aiworkspace/obj/...`.
pub async fn serve(server: Arc<DIDObjectHttpServer<KrpcObjectProvider>>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let bytes = match axum::body::to_bytes(body, MAX_OBJECT_REQUEST_BYTES).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::PAYLOAD_TOO_LARGE, "request too large").into_response(),
    };
    match server.serve_request(buckyos_api::buffered_request(parts, bytes), StreamInfo::default()).await {
        Ok(response) => response.map(Body::new),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}
