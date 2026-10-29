//! DID Object Provider over a service's existing kRPC methods
//! (notepads/agent-object-url-todo.md §2): object kinds are declared by path
//! pattern under `obj://<service>/`, properties / actions / IndexTrait queries
//! call the service's kRPC methods with the caller's own session token, so
//! RBAC and service ACL stay exactly those of the kRPC method. HTTP layout:
//! `/kapi/<service>/obj/<path>` (path-type Object URL, Card in the body,
//! Profile at `?type=profile`).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_http_server::{
    DIDObjectActionRequest, DIDObjectActionSuccess, DIDObjectError, DIDObjectHttpServer,
    DIDObjectRequestContext, DIDObjectServer, DIDObjectServerResult,
};
use bytes::Bytes;
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use kRPC::RPCErrors;
use name_lib::{DIDObjectCard, DID, DID_OBJECT_CONTEXT, INDEX_TRAIT_URI, WOT_TD_CONTEXT};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::object_url::{canonical_json, encode_obj_segment, parse_object_url, ObjectUrl};

pub type Captures = HashMap<String, String>;
/// Builds kRPC params from the path captures and the request input (property: `null`).
pub type ParamsFn = Arc<dyn Fn(&Captures, &Value) -> Result<Value, DIDObjectError> + Send + Sync>;
/// Maps a kRPC result to the value returned to the caller.
pub type ResultFn = Arc<dyn Fn(&Captures, Value) -> Value + Send + Sync>;

/// `x-buckyos:risk` of an action (RBAC权限设计 "Profile x-buckyos:rbac / risk").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionRisk {
    ReadOnly,
    IdempotentWrite,
    Write,
    Destructive,
    Admin,
}

impl ActionRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            ActionRisk::ReadOnly => "read_only",
            ActionRisk::IdempotentWrite => "idempotent_write",
            ActionRisk::Write => "write",
            ActionRisk::Destructive => "destructive",
            ActionRisk::Admin => "admin",
        }
    }
}

#[async_trait]
pub trait ProviderRpc: Send + Sync + 'static {
    async fn call(
        &self,
        method: &str,
        params: Value,
        token: Option<String>,
        ip: Option<IpAddr>,
    ) -> Result<Value, RPCErrors>;
}

/// The service's own kRPC dispatcher as the Provider backend (weak: the HTTP
/// server owns both).
pub struct RpcHandlerBackend<H: kRPC::RPCHandler + Send + Sync + 'static>(pub std::sync::Weak<H>);

#[async_trait]
impl<H: kRPC::RPCHandler + Send + Sync + 'static> ProviderRpc for RpcHandlerBackend<H> {
    async fn call(
        &self,
        method: &str,
        params: Value,
        token: Option<String>,
        ip: Option<IpAddr>,
    ) -> Result<Value, RPCErrors> {
        let handler = self
            .0
            .upgrade()
            .ok_or_else(|| RPCErrors::ReasonError("service is stopping".into()))?;
        let mut req = kRPC::RPCRequest::new(method, params);
        req.token = token;
        let ip = ip.unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST));
        match handler.handle_rpc_call(req, ip).await?.result {
            kRPC::RPCResult::Success(value) => Ok(value),
            kRPC::RPCResult::Failed(message) => Err(RPCErrors::ReasonError(message)),
        }
    }
}

#[derive(Clone)]
pub struct RpcBinding {
    pub method: String,
    pub params: ParamsFn,
    pub result: Option<ResultFn>,
}

impl RpcBinding {
    pub fn new(
        method: &str,
        params: impl Fn(&Captures, &Value) -> Result<Value, DIDObjectError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            method: method.to_string(),
            params: Arc::new(params),
            result: None,
        }
    }

    pub fn map(mut self, result: impl Fn(&Captures, Value) -> Value + Send + Sync + 'static) -> Self {
        self.result = Some(Arc::new(result));
        self
    }
}

#[derive(Clone)]
pub struct PropertySpec {
    pub name: String,
    pub description: String,
    pub schema: Value,
    pub rpc: RpcBinding,
}

#[derive(Clone)]
pub struct ActionSpec {
    pub name: String,
    pub description: String,
    pub input: Value,
    pub risk: ActionRisk,
    /// RBAC action of the object's resource key (`read`, `write`, `create`, ...).
    pub rbac_action: String,
    pub rpc: RpcBinding,
}

#[derive(Clone)]
pub struct EventSpec {
    pub name: String,
    pub description: String,
}

/// IndexTrait (`query` / `page`): the binding returns `{rows, next_cursor?}`; rows
/// carry `object` (an obj:// url) and `cells`.
#[derive(Clone)]
pub struct IndexSpec {
    pub description: String,
    pub columns: Value,
    pub rpc: RpcBinding,
}

#[derive(Clone)]
pub struct ObjectKind {
    /// Path segments under `obj://<service>/`; `{name}` captures one segment, a final
    /// `{*name}` the rest. Query selectors of the request are captured as `?<key>`.
    pub pattern: Vec<String>,
    pub kind: String,
    pub title: String,
    pub description: String,
    pub properties: Vec<PropertySpec>,
    pub actions: Vec<ActionSpec>,
    pub events: Vec<EventSpec>,
    pub index: Option<IndexSpec>,
    /// Property read before answering the Card: the caller must be able to read the object.
    pub access: Option<String>,
    /// Properties worth reading for a summary (`x-buckyos:summary_props`).
    pub summary: Vec<String>,
    /// The service publishes the object at `https://<zone>/<same path>` too: that URL
    /// is its default Object URL across zones (doc/key url.md §6).
    pub same_path_https: bool,
}

impl ObjectKind {
    pub fn new(pattern: &str, kind: &str, title: &str, description: &str) -> Self {
        Self {
            pattern: pattern
                .split('/')
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect(),
            kind: kind.to_string(),
            title: title.to_string(),
            description: description.to_string(),
            properties: Vec::new(),
            actions: Vec::new(),
            events: Vec::new(),
            index: None,
            access: None,
            summary: Vec::new(),
            same_path_https: false,
        }
    }

    pub fn same_path_https(mut self) -> Self {
        self.same_path_https = true;
        self
    }

    pub fn property(mut self, name: &str, description: &str, schema: Value, rpc: RpcBinding) -> Self {
        self.properties.push(PropertySpec {
            name: name.to_string(),
            description: description.to_string(),
            schema,
            rpc,
        });
        self
    }

    pub fn summary_property(mut self, name: &str, description: &str, schema: Value, rpc: RpcBinding) -> Self {
        self.summary.push(name.to_string());
        if self.access.is_none() {
            self.access = Some(name.to_string());
        }
        self.property(name, description, schema, rpc)
    }

    pub fn action(
        mut self,
        name: &str,
        description: &str,
        input: Value,
        risk: ActionRisk,
        rbac_action: &str,
        rpc: RpcBinding,
    ) -> Self {
        self.actions.push(ActionSpec {
            name: name.to_string(),
            description: description.to_string(),
            input,
            risk,
            rbac_action: rbac_action.to_string(),
            rpc,
        });
        self
    }

    pub fn event(mut self, name: &str, description: &str) -> Self {
        self.events.push(EventSpec {
            name: name.to_string(),
            description: description.to_string(),
        });
        self
    }

    pub fn index(mut self, description: &str, columns: Value, rpc: RpcBinding) -> Self {
        self.index = Some(IndexSpec {
            description: description.to_string(),
            columns,
            rpc,
        });
        self
    }

    /// `{name}` captures one segment, a final `{*name}` the rest of the path (`/`-joined, at least one segment).
    fn captures(&self, segments: &[String]) -> Option<Captures> {
        let tail = self
            .pattern
            .last()
            .and_then(|p| p.strip_prefix("{*"))
            .and_then(|p| p.strip_suffix('}'));
        let fixed = if tail.is_some() { self.pattern.len() - 1 } else { self.pattern.len() };
        if (tail.is_none() && segments.len() != fixed) || (tail.is_some() && segments.len() <= fixed) {
            return None;
        }
        let mut captures = Captures::new();
        for (pattern, segment) in self.pattern.iter().take(fixed).zip(segments) {
            match pattern.strip_prefix('{').and_then(|p| p.strip_suffix('}')) {
                Some(name) => {
                    captures.insert(name.to_string(), segment.clone());
                }
                None if pattern == segment => {}
                None => return None,
            }
        }
        if let Some(name) = tail {
            captures.insert(name.to_string(), segments[fixed..].join("/"));
        }
        Some(captures)
    }
}

pub struct KrpcObjectProvider {
    service: String,
    base_path: String,
    zone_did: DID,
    kinds: Vec<ObjectKind>,
    rpc: Arc<dyn ProviderRpc>,
    root_card: DIDObjectCard,
    root_profile: Value,
}

/// `/kapi/<service>/obj`: the HTTP layout of a current-zone service Provider.
pub fn obj_provider_base_path(service: &str) -> String {
    format!("/kapi/{service}/obj")
}

/// Gateway-relative endpoint path of an object (`obj://<host>/<p>` →
/// `/kapi/<host>/obj/<p>`): current-zone services, and zone-hosted entities served
/// as an app of their own name (an Agent mounts at `/kapi/<agent_id>`).
pub fn obj_provider_path(url: &ObjectUrl) -> Option<String> {
    match url {
        ObjectUrl::Obj { host, path, .. } => Some(format!("{}{}", obj_provider_base_path(host.as_str()), path)),
        _ => None,
    }
}

impl KrpcObjectProvider {
    pub fn new(service: &str, zone_did: DID, kinds: Vec<ObjectKind>, rpc: Arc<dyn ProviderRpc>) -> Self {
        let base_path = obj_provider_base_path(service);
        let root_url = format!("obj://{service}");
        let root_card = DIDObjectCard::new(
            zone_did.clone(),
            root_url.clone(),
            None,
            format!("{root_url}?type=profile"),
            Some(format!("{service}.root")),
        );
        let root_profile = json!({
            "@context": [WOT_TD_CONTEXT, DID_OBJECT_CONTEXT],
            "id": format!("{root_url}?type=profile"),
            "title": service,
        });
        Self {
            service: service.to_string(),
            base_path,
            zone_did,
            kinds,
            rpc,
            root_card,
            root_profile,
        }
    }

    pub fn into_http_server(self) -> DIDObjectHttpServer<Self> {
        let pattern = format!("{}/*", self.base_path);
        let id = format!("{}-did-object", self.service);
        DIDObjectHttpServer::new(id, Arc::new(self)).with_obj_url_pattern(pattern)
    }

    pub fn base_path(&self) -> &str {
        &self.base_path
    }

    fn locate(&self, ctx: &DIDObjectRequestContext) -> DIDObjectServerResult<(&ObjectKind, Captures, ObjectUrl)> {
        let rest = ctx
            .object_path
            .strip_prefix(&self.base_path)
            .ok_or_else(|| DIDObjectError::not_found("not a DID Object path of this service"))?;
        let segments: Vec<String> = rest
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| {
                percent_encoding_decode(s)
                    .ok_or_else(|| DIDObjectError::bad_request(format!("invalid path segment '{s}'")))
            })
            .collect::<Result<_, _>>()?;
        let path: String = segments.iter().map(|s| format!("/{}", encode_obj_segment(s))).collect();
        let url = parse_object_url(&format!("obj://{}{}", self.service, path))
            .map_err(|e| DIDObjectError::bad_request(e.to_string()))?;
        let selectors = match ctx.query.as_deref().filter(|q| !q.is_empty()) {
            Some(query) => match parse_object_url(&format!("obj://{}{}?{query}", self.service, path))
                .map_err(|e| DIDObjectError::bad_request(e.to_string()))?
            {
                ObjectUrl::Obj { query, .. } => query,
                _ => Default::default(),
            },
            None => Default::default(),
        };
        for kind in &self.kinds {
            if let Some(mut captures) = kind.captures(&segments) {
                for (key, value) in &selectors {
                    captures.insert(format!("?{key}"), value.clone());
                }
                return Ok((kind, captures, url));
            }
        }
        Err(DIDObjectError::not_found(format!("no object kind at {url}")))
    }

    async fn call(&self, binding: &RpcBinding, captures: &Captures, input: &Value, ctx: &DIDObjectRequestContext) -> DIDObjectServerResult<Value> {
        let params = (binding.params)(captures, input)?;
        let value = self
            .rpc
            .call(&binding.method, params, request_token(&ctx.headers), request_ip(ctx))
            .await
            .map_err(rpc_error)?;
        Ok(match &binding.result {
            Some(map) => map(captures, value),
            None => value,
        })
    }

    fn profile(&self, kind: &ObjectKind, endpoint: &str, url: &ObjectUrl) -> Value {
        let mut properties = Map::new();
        for p in &kind.properties {
            properties.insert(
                p.name.clone(),
                property_affordance(&p.description, p.schema.clone(), &p.name),
            );
        }
        let resource = url.rbac_resource_key(None).unwrap_or_default();
        let mut actions = Map::new();
        for a in &kind.actions {
            actions.insert(
                a.name.clone(),
                action_affordance(&a.description, a.input.clone(), a.risk, &resource, &a.rbac_action, &a.name),
            );
        }
        let mut traits = Vec::new();
        if let Some(index) = &kind.index {
            traits.push(INDEX_TRAIT_URI.to_string());
            properties.insert(
                "index_schema".into(),
                property_affordance("Columns of the index rows", json!({ "type": "object" }), "index_schema"),
            );
            let query_input = json!({ "type": "object", "properties": {
                "query": { "type": "object" }, "page_size": { "type": "integer", "minimum": 1 } } });
            let page_input = json!({ "type": "object", "properties": {
                "cursor": { "type": "string" }, "page_size": { "type": "integer", "minimum": 1 } }, "required": ["cursor"] });
            actions.insert(
                "query".into(),
                action_affordance(&index.description, query_input, ActionRisk::ReadOnly, &resource, "list", "query"),
            );
            actions.insert(
                "page".into(),
                action_affordance("Next page of a query", page_input, ActionRisk::ReadOnly, &resource, "list", "page"),
            );
        }
        let mut events = Map::new();
        for e in &kind.events {
            let kevent = url.object_event_key(&e.name).unwrap_or_default();
            events.insert(
                e.name.clone(),
                json!({
                    "description": e.description,
                    "x-buckyos:kevent": kevent,
                    "x-buckyos:rbac": { "resource": resource, "action": "subscribe" },
                    "forms": [{ "href": format!("events/{}", e.name), "op": "subscribeevent", "contentType": "application/json" }],
                }),
            );
        }
        let mut profile = json!({
            "@context": [WOT_TD_CONTEXT, DID_OBJECT_CONTEXT],
            "id": format!("{endpoint}?type=profile"),
            "title": kind.title,
            "description": kind.description,
            "x-buckyos:kind": kind.kind,
            "x-buckyos:summary_props": kind.summary,
            "properties": properties,
            "actions": actions,
            "events": events,
        });
        if !traits.is_empty() {
            profile["x-buckyos:traits"] = json!(traits);
        }
        profile
    }

    fn card(&self, kind: &ObjectKind, ctx: &DIDObjectRequestContext, url: &ObjectUrl) -> DIDObjectCard {
        let endpoint = request_endpoint(ctx);
        let id = url.object_did(&self.zone_did).unwrap_or_else(|| self.zone_did.clone());
        let mut card = DIDObjectCard::new(
            id,
            endpoint.clone(),
            Some(self.zone_did.clone()),
            format!("{endpoint}?type=profile"),
            Some(kind.kind.clone()),
        );
        if kind.same_path_https {
            if let ObjectUrl::Obj { path, .. } = url {
                card.also_known_as.insert(0, format!("https://{}{path}", request_host(ctx)));
            }
        }
        card.also_known_as.push(url.to_string());
        card
    }
}

#[async_trait]
impl DIDObjectServer for KrpcObjectProvider {
    fn object_card(&self) -> &DIDObjectCard {
        &self.root_card
    }

    fn object_profile(&self) -> &Value {
        &self.root_profile
    }

    async fn object_card_for(&self, ctx: DIDObjectRequestContext) -> DIDObjectServerResult<DIDObjectCard> {
        let (kind, captures, url) = self.locate(&ctx)?;
        if let Some(access) = kind.access.as_deref() {
            if let Some(property) = kind.properties.iter().find(|p| p.name == access) {
                self.call(&property.rpc, &captures, &Value::Null, &ctx).await?;
            }
        }
        Ok(self.card(kind, &ctx, &url))
    }

    async fn object_profile_for(&self, ctx: DIDObjectRequestContext) -> DIDObjectServerResult<Value> {
        let (kind, _, url) = self.locate(&ctx)?;
        Ok(self.profile(kind, &request_endpoint(&ctx), &url))
    }

    async fn read_property(&self, name: &str, ctx: DIDObjectRequestContext) -> DIDObjectServerResult<Value> {
        let (kind, captures, _) = self.locate(&ctx)?;
        if name == "index_schema" {
            if let Some(index) = &kind.index {
                return Ok(index_schema(&index.columns));
            }
        }
        let property = kind
            .properties
            .iter()
            .find(|p| p.name == name)
            .ok_or_else(|| DIDObjectError::new("unknown_property", format!("property {name} is not declared")))?;
        self.call(&property.rpc, &captures, &Value::Null, &ctx).await
    }

    async fn invoke_action(
        &self,
        request: DIDObjectActionRequest,
        ctx: DIDObjectRequestContext,
    ) -> DIDObjectServerResult<DIDObjectActionSuccess> {
        let (kind, captures, url) = self.locate(&ctx)?;
        if let (Some(index), "query" | "page") = (&kind.index, request.method.as_str()) {
            let mut page = self.call(&index.rpc, &captures, &request.params, &ctx).await?;
            page["index"] = json!(url.to_string());
            page["schema_hash"] = json!(index_schema(&index.columns)["schema_hash"]);
            return Ok(DIDObjectActionSuccess::new(page));
        }
        let action = kind
            .actions
            .iter()
            .find(|a| a.name == request.method)
            .ok_or_else(|| DIDObjectError::new("unknown_action", format!("action {} is not declared", request.method)))?;
        let result = self.call(&action.rpc, &captures, &request.params, &ctx).await?;
        Ok(DIDObjectActionSuccess::new(result))
    }
}

fn property_affordance(description: &str, schema: Value, name: &str) -> Value {
    let mut value = match schema {
        Value::Object(map) => Value::Object(map),
        _ => json!({}),
    };
    value["description"] = json!(description);
    value["readOnly"] = json!(true);
    value["forms"] = json!([{ "href": format!("props/{name}"), "op": "readproperty", "contentType": "application/json" }]);
    value
}

fn action_affordance(description: &str, input: Value, risk: ActionRisk, resource: &str, rbac_action: &str, name: &str) -> Value {
    json!({
        "description": description,
        "input": input,
        "x-buckyos:risk": risk.as_str(),
        "x-buckyos:rbac": { "resource": resource, "action": rbac_action },
        "forms": [{ "href": format!("methods/{name}"), "op": "invokeaction", "contentType": "application/json" }],
    })
}

fn index_schema(columns: &Value) -> Value {
    let canonical = canonical_json(columns);
    let hash = Sha256::digest(canonical.as_bytes());
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    json!({ "columns": columns, "schema_hash": format!("sha256:{hex}") })
}

fn percent_encoding_decode(segment: &str) -> Option<String> {
    let bytes = segment.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = segment.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `Authorization: Bearer <token>`, else the SSO cookie (`buckyos_session_token`).
pub fn request_token(headers: &http::HeaderMap) -> Option<String> {
    if let Some(token) = headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        return Some(token.to_string());
    }
    headers
        .get_all(http::header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == "buckyos_session_token")
        .map(|(_, value)| value.to_string())
        .filter(|t| !t.is_empty())
}

fn request_ip(ctx: &DIDObjectRequestContext) -> Option<IpAddr> {
    ctx.stream_info
        .src_addr
        .as_deref()
        .and_then(|addr| addr.parse::<std::net::SocketAddr>().ok())
        .map(|addr| addr.ip())
}

/// Object URL of the request as the caller reached it: forwarded host / proto
/// from the gateway, https unless the host is local.
fn request_host(ctx: &DIDObjectRequestContext) -> String {
    let header = |name: &str| ctx.headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_string);
    header("x-forwarded-host")
        .or_else(|| header("host"))
        .unwrap_or_else(|| "localhost".to_string())
}

fn request_endpoint(ctx: &DIDObjectRequestContext) -> String {
    let host = request_host(ctx);
    let local = host.starts_with("localhost") || host.starts_with("127.") || host.starts_with('[');
    let proto = ctx
        .headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .unwrap_or_else(|| if local { "http".into() } else { "https".into() });
    format!("{proto}://{host}{}", ctx.object_path)
}

fn rpc_error(error: RPCErrors) -> DIDObjectError {
    match error {
        RPCErrors::NoPermission(m) | RPCErrors::InvalidToken(m) => DIDObjectError::new("permission_denied", m),
        RPCErrors::KeyNotExist(m) => DIDObjectError::not_found(m),
        RPCErrors::ParseRequestError(m) => DIDObjectError::new("invalid_params", m),
        other => DIDObjectError::new("internal", other.to_string()),
    }
}

/// A request from another HTTP stack (axum), body already read, in the body
/// type of [`buckyos_http_server::HttpServer`]; DID Object requests are small.
pub fn buffered_request(
    parts: http::request::Parts,
    body: Bytes,
) -> http::Request<BoxBody<Bytes, buckyos_http_server::ServerError>> {
    http::Request::from_parts(parts, Full::new(body).map_err(|never| match never {}).boxed())
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_http_server::{HttpServer, StreamInfo};
    use name_lib::ObjectProfile;
    use std::sync::Mutex;

    struct FakeRpc {
        calls: Mutex<Vec<(String, Value, Option<String>)>>,
    }

    #[async_trait]
    impl ProviderRpc for FakeRpc {
        async fn call(&self, method: &str, params: Value, token: Option<String>, _ip: Option<IpAddr>) -> Result<Value, RPCErrors> {
            self.calls.lock().unwrap().push((method.to_string(), params.clone(), token.clone()));
            if token.as_deref() != Some("good") {
                return Err(RPCErrors::NoPermission("no".into()));
            }
            Ok(json!({ "method": method, "params": params, "items": [{ "id": "a" }] }))
        }
    }

    fn kinds() -> Vec<ObjectKind> {
        vec![ObjectKind::new("owners/{owner}/notes", "kmsg.notes", "Notes", "Notes of an owner")
            .summary_property("recent", "Recent notes", json!({ "type": "object" }), RpcBinding::new("notes.list", |c, _| Ok(json!({ "owner": c["owner"] }))))
            .index("Search notes", json!({ "title": "string" }), RpcBinding::new("notes.list", |c, input| Ok(json!({ "owner": c["owner"], "q": input["query"] })))
                .map(|c, v| json!({ "rows": [{ "object": format!("obj://kmsg/owners/{}/notes/a", c["owner"]), "cells": v["params"] }] })))
            .event("changed", "Notes changed")]
    }

    async fn get(server: &DIDObjectHttpServer<KrpcObjectProvider>, path: &str, token: Option<&str>) -> (u16, Value) {
        let mut builder = http::Request::builder().method("GET").uri(path).header("host", "zone.example");
        if let Some(token) = token {
            builder = builder.header("authorization", format!("Bearer {token}"));
        }
        let req = builder.body(Full::new(Bytes::new()).map_err(|never| match never {}).boxed()).unwrap();
        let resp = server.serve_request(req, StreamInfo::default()).await.unwrap();
        let status = resp.status().as_u16();
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&body).unwrap())
    }

    #[tokio::test]
    async fn serves_card_profile_property_and_index_with_the_callers_token() {
        let rpc = Arc::new(FakeRpc { calls: Mutex::new(Vec::new()) });
        let zone = DID::from_str("did:web:zone.example").unwrap();
        let server = KrpcObjectProvider::new("kmsg", zone, kinds(), rpc.clone()).into_http_server();

        let (status, card) = get(&server, "/kapi/kmsg/obj/owners/did:bns:bob/notes", Some("good")).await;
        assert_eq!(status, 200, "{card}");
        assert_eq!(card["id"], "did:web:zone.example:kmsg:owners:did%3Abns%3Abob:notes");
        assert_eq!(card["alsoKnownAs"], json!(["https://zone.example/kapi/kmsg/obj/owners/did:bns:bob/notes", "obj://kmsg/owners/did:bns:bob/notes"]));
        let (status, _) = get(&server, "/kapi/kmsg/obj/owners/did:bns:bob/notes", Some("bad")).await;
        assert_eq!(status, 403);
        let (status, _) = get(&server, "/kapi/kmsg/obj/owners/did:bns:bob/other", Some("good")).await;
        assert_eq!(status, 404);

        let (status, profile) = get(&server, "/kapi/kmsg/obj/owners/did:bns:bob/notes?type=profile", None).await;
        assert_eq!(status, 200);
        let parsed = ObjectProfile::from_value(profile.clone()).unwrap();
        parsed.validate().unwrap();
        assert!(parsed.implements_trait(INDEX_TRAIT_URI));
        assert_eq!(profile["x-buckyos:summary_props"], json!(["recent"]));
        assert_eq!(profile["actions"]["query"]["x-buckyos:rbac"], json!({ "resource": "obj://kmsg/owners/did:bns:bob/notes", "action": "list" }));
        assert_eq!(profile["events"]["changed"]["x-buckyos:kevent"], "/obj/kmsg/owners/x6469643a626e733a626f62/notes/changed");

        let (status, value) = get(&server, "/kapi/kmsg/obj/owners/did:bns:bob/notes/props/recent", Some("good")).await;
        assert_eq!(status, 200);
        assert_eq!(value["params"], json!({ "owner": "did:bns:bob" }));

        let body = serde_json::to_vec(&json!({ "method": "query", "params": { "query": { "q": "x" } } })).unwrap();
        let req = http::Request::builder()
            .method("POST")
            .uri("/kapi/kmsg/obj/owners/did:bns:bob/notes/methods/query")
            .header("host", "zone.example")
            .header("cookie", "a=b; buckyos_session_token=good")
            .body(Full::new(Bytes::from(body)).map_err(|never| match never {}).boxed())
            .unwrap();
        let resp = server.serve_request(req, StreamInfo::default()).await.unwrap();
        let page: Value = serde_json::from_slice(&resp.into_body().collect().await.unwrap().to_bytes()).unwrap();
        assert_eq!(page["result"]["rows"][0]["object"], "obj://kmsg/owners/did:bns:bob/notes/a");
        assert_eq!(page["result"]["index"], "obj://kmsg/owners/did:bns:bob/notes");
        assert!(page["result"]["schema_hash"].as_str().unwrap().starts_with("sha256:"));
        assert!(rpc.calls.lock().unwrap().iter().all(|(m, _, _)| m == "notes.list"));
    }
}
