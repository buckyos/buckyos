//! DID Object Provider of DFS (notepads/agent-object-url-todo.md §4.6): `obj://dfs/<path>`
//! is the object view of the same resource as `cyfs:///<path>` (one RBAC key, one ACL),
//! served at `/kapi/dfs/obj/...`. The object describes the file or directory (stat,
//! listing); its bytes stay the file resource (`cyfs:///<path>`, `/nfs/v1/read/<node>`).
//! Every read is resolved with the caller's identity through the path ACL, exactly like
//! NFSP `stat` / `list`.

use std::net::IpAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::Request;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use buckyos_api::{encode_obj_segment, Captures, KrpcObjectProvider, ObjectKind, ProviderRpc, RpcBinding};
use buckyos_http_server::{DIDObjectHttpServer, HttpServer, StreamInfo};
use kRPC::RPCErrors;
use name_lib::DID;
use serde_json::{json, Value};

use crate::acl::Level;
use crate::auth::{authenticate, require_file_actor, Credentials};
use crate::error::{ErrorCode, NfsError};
use crate::namespace::ListArgs;
use crate::state::SharedState;
use crate::types::{Locator, WantMask};

pub const OBJ_PATH: &str = "/kapi/dfs/obj";
const MAX_OBJECT_REQUEST_BYTES: usize = 1024 * 1024;

struct DfsBackend {
    state: SharedState,
}

fn rpc_error(e: NfsError) -> RPCErrors {
    let text = e.to_string();
    match e.code {
        ErrorCode::PermissionDenied => RPCErrors::NoPermission(text),
        ErrorCode::NotFound => RPCErrors::KeyNotExist(text),
        ErrorCode::InvalidArgument | ErrorCode::NotAContainer => RPCErrors::ParseRequestError(text),
        _ => RPCErrors::ReasonError(text),
    }
}

fn dfs_uri(path: &str) -> String {
    let encoded: String = path.split('/').filter(|s| !s.is_empty()).map(|s| format!("/{}", encode_obj_segment(s))).collect();
    format!("cyfs://{}", if encoded.is_empty() { "/".to_string() } else { encoded })
}

#[async_trait::async_trait]
impl ProviderRpc for DfsBackend {
    async fn call(&self, method: &str, params: Value, token: Option<String>, _ip: Option<IpAddr>) -> Result<Value, RPCErrors> {
        let mut headers = HeaderMap::new();
        if let Some(value) = token.and_then(|t| HeaderValue::from_str(&format!("Bearer {t}")).ok()) {
            headers.insert(header::AUTHORIZATION, value);
        }
        let actor = authenticate(&self.state, &Credentials::from_headers(&headers)).await.map_err(rpc_error)?;
        require_file_actor(&actor).map_err(rpc_error)?;
        let path = params["path"].as_str().unwrap_or_default().to_string();
        let at = Locator { uri: Some(dfs_uri(&path)), ..Default::default() };
        let want = WantMask::from_opt(&None);
        let state = &self.state;
        match method {
            "dfs.stat" => {
                let node = state.resolve_for(&actor, &at, Level::Traverse, "read").map_err(rpc_error)?;
                let mut info = state.node_info(&actor, &node, &want).map_err(rpc_error)?;
                info["uri"] = json!(dfs_uri(&path));
                Ok(info)
            }
            "dfs.list" => {
                let node = state.resolve_for(&actor, &at, Level::Traverse, "read").map_err(rpc_error)?;
                let args: ListArgs = serde_json::from_value(json!({
                    "cursor": params.get("cursor").cloned().unwrap_or(Value::Null),
                    "limit": params.get("limit").cloned().unwrap_or(Value::Null),
                }))
                .map_err(|e| RPCErrors::ParseRequestError(e.to_string()))?;
                state.list_node(&actor, &node, &args, &want).map_err(rpc_error)
            }
            other => Err(RPCErrors::ReasonError(format!("unknown object method {other}"))),
        }
    }
}

fn path_of(c: &Captures) -> String {
    c.get("path").cloned().unwrap_or_default()
}

fn listing_rows(c: &Captures, listing: Value) -> Value {
    let parent = path_of(c);
    let rows: Vec<Value> = listing["entries"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|entry| {
            let name = entry.get("name")?.as_str()?.to_string();
            let segments: Vec<&str> = parent.split('/').filter(|s| !s.is_empty()).chain(std::iter::once(name.as_str())).collect();
            Some(json!({ "row_id": name, "object": buckyos_api::obj_service_url("dfs", &segments), "cells": entry }))
        })
        .collect();
    json!({ "rows": rows, "next_cursor": listing.get("next_cursor").cloned().unwrap_or(Value::Null) })
}

pub fn object_kinds() -> Vec<ObjectKind> {
    let object = json!({ "type": "object" });
    let list = |root: bool| {
        RpcBinding::new("dfs.list", move |c, input| {
            Ok(json!({
                "path": if root { String::new() } else { path_of(c) },
                "cursor": input.get("cursor").cloned().unwrap_or(Value::Null),
                "limit": input.get("page_size").cloned().unwrap_or(json!(100)),
            }))
        })
        .map(listing_rows)
    };
    let root = ObjectKind::new("", "dfs.root", "Zone files", "The DFS root: the exported top-level directories visible to the caller.")
        .summary_property("listing", "Visible top-level entries", object.clone(), list(true))
        .index("List the visible top-level entries", json!({ "kind": "string", "size": "integer", "mtime": "integer" }), list(true));
    let entry = ObjectKind::new(
        "{*path}",
        "dfs.entry",
        "Zone file or directory",
        "A file or directory of the Zone's file system (`cyfs:///<path>`, the same resource). `info` describes it; a directory also lists its entries. The bytes are the file resource, not a property.",
    )
    .summary_property("info", "stat of the node (kind, size, mtime, revision, access level, cyfs uri)", object,
        RpcBinding::new("dfs.stat", |c, _| Ok(json!({ "path": path_of(c) }))))
    .index("List a directory", json!({ "kind": "string", "size": "integer", "mtime": "integer" }), list(false));
    vec![root, entry]
}

pub fn provider(state: SharedState) -> DIDObjectHttpServer<KrpcObjectProvider> {
    let zone_did = buckyos_api::get_buckyos_api_runtime()
        .map(|runtime| runtime.zone_id.clone())
        .unwrap_or_else(|_| DID::new("web", "localhost"));
    KrpcObjectProvider::new("dfs", zone_did, object_kinds(), Arc::new(DfsBackend { state })).into_http_server()
}

/// axum handler of `/kapi/dfs/obj/...`.
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
