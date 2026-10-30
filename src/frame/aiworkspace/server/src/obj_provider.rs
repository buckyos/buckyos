//! DID Object Provider of AI Workspace (notepads/agent-object-url-todo.md §4.4, D7):
//! `obj://aiworkspace/<ws_id>`, `.../entities/<entity_id>[?mode=&selector=&version=]`
//! and `.../path/<...>`, served at `/kapi/aiworkspace/obj/...`. Reads are the kRPC
//! methods (`ws.get_info`, `doc.outline`, `doc.read`, `doc.resolve`, ...) through the
//! service's own kRPC entry with the caller's credentials, so sessions, run tokens,
//! guest tokens and Workspace grants decide exactly as for kRPC.

use std::net::IpAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use buckyos_api::{Captures, KrpcObjectProvider, ObjectKind, ProviderRpc, RpcBinding, AIWORKSPACE_HTTP_PATH};
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
            return Err(match result["error"]["code"].as_str() {
                Some("PERMISSION_DENIED") => RPCErrors::NoPermission(detail),
                Some("NOT_FOUND") | Some("TARGET_DELETED") => RPCErrors::KeyNotExist(detail),
                Some("INVALID_OPERATION") | Some("INVALID_SCHEMA") => RPCErrors::ParseRequestError(detail),
                _ => RPCErrors::ReasonError(detail),
            });
        }
        Ok(result)
    }
}

fn cap(c: &Captures, name: &str) -> String {
    c.get(name).cloned().unwrap_or_default()
}

/// `selector` / `version` of the request query (canonical JSON), `mode` defaults to live.
fn read_params(c: &Captures) -> Result<Value, DIDObjectError> {
    let mut params = json!({ "workspace_id": cap(c, "ws"), "entity_id": cap(c, "entity") });
    if let Some(selector) = c.get("?selector") {
        params["selector"] = serde_json::from_str(selector).map_err(|e| DIDObjectError::new("invalid_params", format!("selector: {e}")))?;
    }
    Ok(params)
}

fn outline_rows(c: &Captures, outline: Value) -> Value {
    let ws = cap(c, "ws");
    let rows: Vec<Value> = outline["entities"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|env| {
            let id = env.get("entity_id").or_else(|| env.get("id"))?.as_str()?.to_string();
            let reference = json!({ "workspace_id": ws, "entity_id": id, "mode": "live" });
            Some(json!({
                "row_id": id,
                "object": buckyos_api::aiws_reference_url(&reference, None).ok()?,
                "cells": { "kind": env["kind"], "title": env.get("title").or_else(|| env.get("name")).cloned().unwrap_or(Value::Null), "parent": env.get("parent_id").cloned().unwrap_or(Value::Null) },
            }))
        })
        .collect();
    json!({ "rows": rows })
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

pub fn object_kinds() -> Vec<ObjectKind> {
    let object = json!({ "type": "object" });
    let list = || RpcBinding::new("ws.list", |_, _| Ok(json!({}))).map(|_, v| workspace_rows(v));
    let root = ObjectKind::new("", "aiworkspace.root", "AI Workspaces", "The AI Workspaces the caller can see. Rows link to the workspace objects.")
        .summary_property("workspaces", "The visible workspaces", object.clone(), list())
        .index("List the visible workspaces", json!({ "title": "string", "head_seq": "integer" }), list());
    let workspace = ObjectKind::new("{ws}", "aiworkspace.workspace", "AI Workspace", "A workspace: its info and the readable entity tree. Rows link to the entity objects.")
        .summary_property("info", "Workspace info (title, owner, head, publications)", object.clone(), RpcBinding::new("ws.get_info", |c, _| Ok(json!({ "workspace_id": cap(c, "ws") }))))
        .index("The readable entity tree", json!({ "kind": "string", "title": "string", "parent": "string" }),
            RpcBinding::new("doc.outline", |c, _| Ok(json!({ "workspace_id": cap(c, "ws") }))).map(outline_rows))
        .event("head", "A new head was committed")
        .event("locks", "The lock table changed");
    let entity = ObjectKind::new(
        "{ws}/entities/{entity}",
        "aiworkspace.entity",
        "AI Workspace entity",
        "One entity of a workspace. `?selector=<canonical json>&mode=fixed&version=<json>` select a view or version of the same object.",
    )
    .summary_property("content", "The entity as read (with the selector of the url, if any)", object.clone(), RpcBinding::new("doc.read", |c, _| read_params(c)))
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
        }));
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
