//! DID Object Provider of HomeStation (notepads/agent-object-url-todo.md §4.5, D10 / D11):
//! `obj://homestation/home/<user>`, `/feed`, `/feed/@/<key>`, `/inbox` and
//! `obj://homestation/home/~zone`, served at `/kapi/homestation/obj/...`. Their default
//! Object URL is the same path over https (`https://<zone>/home/<user>/...`), the
//! cross-zone reference. Reads go through the protocol handlers of `/home/<user>/...`
//! with the caller's credentials, so reader rules are those of the public paths.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::{Path, Query, Request, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use buckyos_api::{Captures, KrpcObjectProvider, ObjectKind, ProviderRpc, RpcBinding};
use buckyos_http_server::{DIDObjectHttpServer, HttpServer, StreamInfo};
use http_body_util::BodyExt;
use kRPC::RPCErrors;
use name_lib::DID;
use serde_json::{json, Value};

use crate::http::AppState;
use crate::protocol::NS_FEED;

pub const OBJ_PATH: &str = "/kapi/homestation/obj";
const MAX_OBJECT_REQUEST_BYTES: usize = 1024 * 1024;

struct HomeBackend {
    state: AppState,
}

fn param(params: &Value, name: &str) -> String {
    params.get(name).and_then(Value::as_str).unwrap_or_default().to_string()
}

#[async_trait::async_trait]
impl ProviderRpc for HomeBackend {
    async fn call(&self, method: &str, params: Value, token: Option<String>, _ip: Option<IpAddr>) -> Result<Value, RPCErrors> {
        let mut headers = HeaderMap::new();
        if let Some(token) = token {
            if let Ok(value) = HeaderValue::from_str(&format!("Bearer {token}")) {
                headers.insert(header::AUTHORIZATION, value);
            }
        }
        let state = State(self.state.clone());
        let user = param(&params, "user");
        let mut query = HashMap::new();
        for key in ["limit", "cursor"] {
            if let Some(value) = params.get(key).filter(|v| !v.is_null()) {
                query.insert(key.to_string(), value.as_str().map(str::to_string).unwrap_or_else(|| value.to_string()));
            }
        }
        let response = match method {
            "home.profile" => crate::http::profile(state, Path(user), headers, Query(query)).await,
            "home.feed" => crate::http::feed(state, Path(user), headers, Query(query)).await,
            "home.entry" => crate::http::entry_head(state, headers, Path((user, param(&params, "ns"), param(&params, "key"))), Query(query)).await,
            "home.inbox" => {
                let home = crate::protocol::HomeRef::new(&self.state.host.cfg.zone, &user);
                return Ok(json!({ "target": home.inbox(), "content_types": ["application/cyfs-named-object+jwt"] }));
            }
            other => return Err(RPCErrors::ReasonError(format!("unknown object method {other}"))),
        };
        response_value(response).await
    }
}

async fn response_value(response: Response) -> Result<Value, RPCErrors> {
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .map_err(|e| RPCErrors::ReasonError(e.to_string()))?
        .to_bytes();
    let text = String::from_utf8_lossy(&bytes).to_string();
    match status {
        s if s.is_success() => Ok(serde_json::from_slice(&bytes).unwrap_or(Value::String(text))),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => Err(RPCErrors::NoPermission(text)),
        StatusCode::NOT_FOUND => Err(RPCErrors::KeyNotExist(text)),
        StatusCode::BAD_REQUEST => Err(RPCErrors::ParseRequestError(text)),
        _ => Err(RPCErrors::ReasonError(text)),
    }
}

fn user_of(c: &Captures) -> Value {
    json!(c.get("user").cloned().unwrap_or_default())
}

fn feed_rows(page: Value) -> Value {
    let rows: Vec<Value> = page["items"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|item| {
            let entry = item.get("entry").and_then(Value::as_str)?.to_string();
            Some(json!({ "row_id": entry, "object": entry, "cells": item }))
        })
        .collect();
    json!({ "rows": rows, "next_cursor": page.get("next_cursor").or_else(|| page.get("cursor")).cloned().unwrap_or(Value::Null) })
}

pub fn object_kinds() -> Vec<ObjectKind> {
    let object = json!({ "type": "object" });
    let home = ObjectKind::new("home/{user}", "homestation.home", "HomeStation home", "A user's home (or `~zone`, the zone feed): profile, publisher capabilities, stream and inbox addresses.")
        .summary_property("profile", "Profile of the home", object.clone(), RpcBinding::new("home.profile", |c, _| Ok(json!({ "user": user_of(c) }))))
        .same_path_https();
    let feed = ObjectKind::new("home/{user}/feed", "homestation.feed", "HomeStation feed", "The user's published stream: entries visible to the reader, newest first. Rows are entry URLs.")
        .summary_property("recent", "The 20 latest visible entries", object.clone(), RpcBinding::new("home.feed", |c, _| Ok(json!({ "user": user_of(c), "limit": 20 }))).map(|_, v| feed_rows(v)))
        .index(
            "Page through the visible entries",
            json!({ "entry": "string", "head": "object" }),
            RpcBinding::new("home.feed", |c, input| {
                Ok(json!({ "user": user_of(c), "limit": input.get("page_size").cloned().unwrap_or(json!(20)), "cursor": input.get("cursor").cloned().unwrap_or(Value::Null) }))
            })
            .map(|_, v| feed_rows(v)),
        )
        .same_path_https();
    let entry = ObjectKind::new("home/{user}/feed/@/{key}", "homestation.entry", "HomeStation entry", "A mutable entry of the feed; each version is a cyfeed / cyfhead object, read by its cyfs:// ObjId.")
        .summary_property(
            "head",
            "Current signed Head of the entry",
            object.clone(),
            RpcBinding::new("home.entry", |c, _| Ok(json!({ "user": user_of(c), "ns": NS_FEED, "key": c.get("key").cloned().unwrap_or_default() }))),
        )
        .same_path_https();
    let inbox = ObjectKind::new("home/{user}/inbox", "homestation.inbox", "HomeStation inbox", "Delivery point of the user's home: signed NamedObjects are PUT to its https target (CYFS dispatch).")
        .summary_property("target", "The https dispatch target and accepted encodings", object, RpcBinding::new("home.inbox", |c, _| Ok(json!({ "user": user_of(c) }))))
        .same_path_https();
    vec![home, feed, entry, inbox]
}

pub fn provider(state: AppState) -> DIDObjectHttpServer<KrpcObjectProvider> {
    let zone_did = state
        .host
        .cfg
        .zone_did
        .as_deref()
        .and_then(|d| DID::from_str(d).ok())
        .unwrap_or_else(|| DID::new("web", &state.host.cfg.zone));
    KrpcObjectProvider::new("homestation", zone_did, object_kinds(), Arc::new(HomeBackend { state })).into_http_server()
}

/// axum handler of `/kapi/homestation/obj/...`.
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
