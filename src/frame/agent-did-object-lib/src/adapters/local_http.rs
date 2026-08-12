use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    AdapterCallStatus, AdapterConfig, AdapterEventSubscription, AdapterReadRequest,
    AdapterReadResponse, AdapterSubscribeEventRequest, AdapterUnsubscribeEventRequest,
    AdapterXCallRequest, AdapterXCallResponse, AgentObjectAdapter,
};
use crate::error::AgentDIDObjectError;
use crate::router::RouteTrace;
use crate::types::{
    EventBridgeSubscription, PromptGuidance, ReadAttachedError, ReadMeta, TrustGuidance,
};

pub const LOCAL_HTTP_ADAPTER_PROTOCOL: &str = "agent-did-object-adapter/1";
pub const LOCAL_HTTP_READ_PATH: &str = "/adapter/read";
pub const LOCAL_HTTP_X_CALL_PATH: &str = "/adapter/x-call";
pub const LOCAL_HTTP_EVENT_SUBSCRIBE_PATH: &str = "/adapter/events/subscribe";
pub const LOCAL_HTTP_EVENT_UNSUBSCRIBE_PATH: &str = "/adapter/events/unsubscribe";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpRouteRef {
    #[serde(alias = "route_id")]
    pub id: String,
    pub adapter: String,
}

impl LocalHttpRouteRef {
    fn from_trace(trace: &RouteTrace) -> Self {
        Self {
            id: trace.route_id.clone(),
            adapter: trace.adapter.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpReadRequest {
    pub protocol: String,
    pub object: String,
    pub route: LocalHttpRouteRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub options: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpXCallRequest {
    pub protocol: String,
    pub object: String,
    pub route: LocalHttpRouteRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub options: Value,
    pub action: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm_token: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpSubscribeEventRequest {
    pub protocol: String,
    pub object: String,
    pub route: LocalHttpRouteRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default)]
    pub options: Value,
    pub event: String,
    #[serde(default)]
    pub filter: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpUnsubscribeEventRequest {
    pub protocol: String,
    pub subscription_id: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpReadResponse {
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default)]
    pub meta: ReadMeta,
    #[serde(default)]
    pub prompt_guidance: Vec<PromptGuidance>,
    #[serde(default)]
    pub trust_guidance: Vec<TrustGuidance>,
    #[serde(default)]
    pub errors: Vec<ReadAttachedError>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<LocalHttpRouteRef>,
    #[serde(default)]
    pub adapt_meta: Value,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalHttpSubscribeEventResponse {
    pub subscription_id: String,
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_did: Option<String>,
    pub event: String,
    pub kevent_pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<LocalHttpRouteRef>,
}

pub struct LocalHttpAdapter {
    id: String,
    client: Client,
    endpoint: String,
    auth_token_env: Option<String>,
}

impl LocalHttpAdapter {
    pub fn new(id: String, config: AdapterConfig) -> Result<Self, AgentDIDObjectError> {
        Ok(Self {
            id,
            client: Client::new(),
            endpoint: config.endpoint.ok_or_else(|| {
                AgentDIDObjectError::InvalidConfig("local_http endpoint missing".to_string())
            })?,
            auth_token_env: config.auth_token_env,
        })
    }

    fn post(&self, path: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.post(format!(
            "{}/{}",
            self.endpoint.trim_end_matches('/'),
            path.trim_start_matches('/')
        ));
        if let Some(env_name) = &self.auth_token_env {
            if let Ok(token) = std::env::var(env_name) {
                req = req.bearer_auth(token);
            }
        }
        req
    }

    fn read_request(&self, req: &AdapterReadRequest) -> LocalHttpReadRequest {
        LocalHttpReadRequest {
            protocol: LOCAL_HTTP_ADAPTER_PROTOCOL.to_string(),
            object: req.object_ref.raw.clone(),
            route: LocalHttpRouteRef::from_trace(&req.route_trace),
            session_id: req.input.session_id.clone(),
            trace_id: None,
            options: req.input.options.clone(),
        }
    }

    fn x_call_request(&self, req: &AdapterXCallRequest) -> LocalHttpXCallRequest {
        LocalHttpXCallRequest {
            protocol: LOCAL_HTTP_ADAPTER_PROTOCOL.to_string(),
            object: req.object_ref.raw.clone(),
            route: LocalHttpRouteRef::from_trace(&req.route_trace),
            session_id: req.input.session_id.clone(),
            trace_id: req.input.trace_id.clone(),
            options: req.route.options.clone(),
            action: req.input.action.clone(),
            params: req.input.params.clone(),
            idempotency_key: req.input.idempotency_key.clone(),
            confirm_token: req.input.confirm_token.clone(),
        }
    }

    fn subscribe_event_request(
        &self,
        req: &AdapterSubscribeEventRequest,
    ) -> LocalHttpSubscribeEventRequest {
        LocalHttpSubscribeEventRequest {
            protocol: LOCAL_HTTP_ADAPTER_PROTOCOL.to_string(),
            object: req.object_ref.raw.clone(),
            route: LocalHttpRouteRef::from_trace(&req.route_trace),
            session_id: req.input.session_id.clone(),
            trace_id: req.input.trace_id.clone(),
            options: req.route.options.clone(),
            event: req.input.event.clone(),
            filter: req.input.filter.clone(),
            ttl_ms: req.input.ttl_ms,
            cursor: req.input.cursor.clone(),
        }
    }
}

#[async_trait]
impl AgentObjectAdapter for LocalHttpAdapter {
    fn id(&self) -> &str {
        &self.id
    }

    async fn read(
        &self,
        req: AdapterReadRequest,
    ) -> Result<AdapterReadResponse, AgentDIDObjectError> {
        let response = self
            .post(LOCAL_HTTP_READ_PATH)
            .json(&self.read_request(&req))
            .send()
            .await?;
        let response: LocalHttpReadResponse = parse_json_response(response).await?;
        Ok(AdapterReadResponse {
            object: response.object,
            object_did: response.object_did,
            content: response.content,
            meta: response.meta,
            prompt_guidance: response.prompt_guidance,
            trust_guidance: response.trust_guidance,
            errors: response.errors,
            cache_key: response.cache_key,
            version: response.version,
            route: req.route_trace,
            adapt_meta: response.adapt_meta,
        })
    }

    async fn x_call(
        &self,
        req: AdapterXCallRequest,
    ) -> Result<AdapterXCallResponse, AgentDIDObjectError> {
        let response = self
            .post(LOCAL_HTTP_X_CALL_PATH)
            .json(&self.x_call_request(&req))
            .send()
            .await?;
        let value: Value = parse_json_response(response).await?;
        if value.get("agent_tool_protocol").and_then(Value::as_str) == Some("1") {
            return Ok(AdapterXCallResponse {
                status: match value.get("status").and_then(Value::as_str) {
                    Some("error") => AdapterCallStatus::Error,
                    Some("pending") => AdapterCallStatus::Pending,
                    _ => AdapterCallStatus::Success,
                },
                output: value
                    .get("output")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                detail: value
                    .get("detail")
                    .or_else(|| value.get("details"))
                    .cloned()
                    .unwrap_or_else(|| json!({})),
                title: Some(format!("x-call `{}` completed", req.input.action)),
                summary: Some(format!(
                    "Local HTTP adapter `{}` returned an AgentToolResult.",
                    self.id
                )),
                route: req.route_trace,
            });
        }
        Ok(AdapterXCallResponse {
            status: match value.get("status").and_then(Value::as_str) {
                Some("error") => AdapterCallStatus::Error,
                Some("pending") => AdapterCallStatus::Pending,
                _ => AdapterCallStatus::Success,
            },
            output: value
                .get("output")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            detail: value,
            title: Some(format!("x-call `{}` completed", req.input.action)),
            summary: Some(format!(
                "Local HTTP adapter `{}` returned a response.",
                self.id
            )),
            route: req.route_trace,
        })
    }

    async fn subscribe_event(
        &self,
        req: AdapterSubscribeEventRequest,
    ) -> Result<AdapterEventSubscription, AgentDIDObjectError> {
        let response = self
            .post(LOCAL_HTTP_EVENT_SUBSCRIBE_PATH)
            .json(&self.subscribe_event_request(&req))
            .send()
            .await?;
        let response: LocalHttpSubscribeEventResponse = parse_json_response(response).await?;
        let subscription = EventBridgeSubscription {
            subscription_id: response.subscription_id,
            object: response.object,
            object_did: response.object_did,
            event: response.event,
            kevent_pattern: response.kevent_pattern,
            expires_at: response.expires_at,
            cursor: response.cursor,
            route: req.route_trace,
        };
        Ok(AdapterEventSubscription {
            subscription,
            transport: None,
            unsubscribe_via_adapter: true,
        })
    }

    async fn unsubscribe_event(
        &self,
        req: AdapterUnsubscribeEventRequest,
    ) -> Result<(), AgentDIDObjectError> {
        let payload = LocalHttpUnsubscribeEventRequest {
            protocol: LOCAL_HTTP_ADAPTER_PROTOCOL.to_string(),
            subscription_id: req.input.subscription_id,
        };
        let response = self
            .post(LOCAL_HTTP_EVENT_UNSUBSCRIBE_PATH)
            .json(&payload)
            .send()
            .await?;
        parse_json_response::<Value>(response).await?;
        Ok(())
    }
}

async fn parse_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, AgentDIDObjectError> {
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(AgentDIDObjectError::HttpError(format!(
            "local_http adapter returned {status}: {body}"
        )));
    }
    serde_json::from_str(&body).map_err(|err| AgentDIDObjectError::ProtocolError(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AdapterType, ObjectRoute};
    use crate::router::{RouteMatchType, RouteMethod, RouteTrace};
    use crate::types::{
        ObjectRef, ReadInput, SubscribeEventInput, UnsubscribeEventInput, XCallInput,
    };
    use std::sync::Arc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;

    #[test]
    fn builds_loopback_adapter() {
        let adapter = LocalHttpAdapter::new(
            "local".to_string(),
            AdapterConfig {
                id: "local".to_string(),
                adapter_type: AdapterType::LocalHttp,
                endpoint: Some("http://127.0.0.1:8787".to_string()),
                auth_token_env: None,
                options: json!({}),
            },
        )
        .unwrap();
        assert_eq!(adapter.id(), "local");
    }

    fn local_adapter_config(endpoint: &str) -> AdapterConfig {
        AdapterConfig {
            id: "local".to_string(),
            adapter_type: AdapterType::LocalHttp,
            endpoint: Some(endpoint.to_string()),
            auth_token_env: None,
            options: json!({}),
        }
    }

    fn route() -> ObjectRoute {
        ObjectRoute {
            id: "local-obj".to_string(),
            priority: 0,
            match_type: RouteMatchType::Scheme,
            pattern: "obj".to_string(),
            adapter: "local".to_string(),
            methods: vec![RouteMethod::Read],
            options: json!({"route_opt": true}),
        }
    }

    fn trace(method: RouteMethod) -> RouteTrace {
        RouteTrace {
            route_id: "local-obj".to_string(),
            adapter: "local".to_string(),
            method,
        }
    }

    fn read_request() -> AdapterReadRequest {
        AdapterReadRequest {
            object_ref: ObjectRef::parse("obj://example/item/1").unwrap(),
            input: ReadInput {
                object: "obj://example/item/1".to_string(),
                purpose: None,
                session_id: Some("s".to_string()),
                content_only: false,
                range: None,
                max_tokens: None,
                options: json!({"a": 1}),
            },
            route: route(),
            route_trace: trace(RouteMethod::Read),
            adapter_config: local_adapter_config("http://127.0.0.1:8787"),
        }
    }

    fn x_call_request(action: &str) -> AdapterXCallRequest {
        AdapterXCallRequest {
            object_ref: ObjectRef::parse("obj://example/item/1").unwrap(),
            input: XCallInput {
                object: "obj://example/item/1".to_string(),
                action: action.to_string(),
                params: json!({"count": 1}),
                session_id: Some("s".to_string()),
                idempotency_key: Some("idem".to_string()),
                confirm_token: Some("confirm".to_string()),
                trace_id: Some("trace".to_string()),
            },
            route: route(),
            route_trace: trace(RouteMethod::XCall),
            adapter_config: local_adapter_config("http://127.0.0.1:8787"),
        }
    }

    fn subscribe_request() -> AdapterSubscribeEventRequest {
        AdapterSubscribeEventRequest {
            object_ref: ObjectRef::parse("obj://example/item/1").unwrap(),
            input: SubscribeEventInput {
                object: "obj://example/item/1".to_string(),
                event: "changed".to_string(),
                filter: json!({"field": "name"}),
                session_id: Some("s".to_string()),
                ttl_ms: Some(300_000),
                cursor: None,
                trace_id: Some("trace".to_string()),
            },
            route: route(),
            route_trace: trace(RouteMethod::SubscribeEvent),
            adapter_config: local_adapter_config("http://127.0.0.1:8787"),
        }
    }

    #[test]
    fn request_payloads_match_local_http_contract() {
        let adapter = LocalHttpAdapter::new(
            "local".to_string(),
            local_adapter_config("http://127.0.0.1:8787"),
        )
        .unwrap();

        let read = adapter.read_request(&read_request());
        assert_eq!(read.protocol, LOCAL_HTTP_ADAPTER_PROTOCOL);
        assert_eq!(read.object, "obj://example/item/1");
        assert_eq!(read.route.id, "local-obj");
        assert_eq!(read.route.adapter, "local");
        assert_eq!(read.session_id.as_deref(), Some("s"));
        assert_eq!(read.options["a"], 1);

        let x_call = adapter.x_call_request(&x_call_request("reserve"));
        assert_eq!(x_call.action, "reserve");
        assert_eq!(x_call.params["count"], 1);
        assert_eq!(x_call.trace_id.as_deref(), Some("trace"));
        assert_eq!(x_call.idempotency_key.as_deref(), Some("idem"));
        assert_eq!(x_call.confirm_token.as_deref(), Some("confirm"));
        assert_eq!(x_call.options["route_opt"], true);

        let subscribe = adapter.subscribe_event_request(&subscribe_request());
        assert_eq!(subscribe.event, "changed");
        assert_eq!(subscribe.filter["field"], "name");
        assert_eq!(subscribe.ttl_ms, Some(300_000));
    }

    #[tokio::test]
    async fn local_http_adapter_roundtrips_with_test_server() {
        let (endpoint, observed) = spawn_test_server().await;
        let token_var = "AGENT_DID_OBJECT_LIB_LOCAL_HTTP_TEST_TOKEN";
        std::env::set_var(token_var, "secret");

        let mut config = local_adapter_config(&endpoint);
        config.auth_token_env = Some(token_var.to_string());
        let adapter = LocalHttpAdapter::new("local".to_string(), config).unwrap();

        let read = adapter.read(read_request()).await.unwrap();
        assert_eq!(read.content.as_deref(), Some("Item full content"));
        assert_eq!(read.route.route_id, "local-obj");

        let x_call = adapter.x_call(x_call_request("reserve")).await.unwrap();
        assert_eq!(x_call.status, AdapterCallStatus::Success);
        assert_eq!(x_call.detail["reserved"], true);
        assert_eq!(x_call.route.route_id, "local-obj");

        let x_call_tool = adapter.x_call(x_call_request("return_tool")).await.unwrap();
        assert_eq!(x_call_tool.status, AdapterCallStatus::Pending);
        assert_eq!(x_call_tool.detail["queued"], true);
        assert_eq!(
            x_call_tool.title.as_deref(),
            Some("x-call `return_tool` completed")
        );

        let subscription = adapter.subscribe_event(subscribe_request()).await.unwrap();
        assert_eq!(subscription.subscription.subscription_id, "sub-1");
        assert_eq!(
            subscription.subscription.kevent_pattern,
            "agent.object.changed"
        );
        assert_eq!(subscription.subscription.route.route_id, "local-obj");

        adapter
            .unsubscribe_event(AdapterUnsubscribeEventRequest {
                input: UnsubscribeEventInput {
                    subscription_id: "sub-1".to_string(),
                },
            })
            .await
            .unwrap();

        std::env::remove_var(token_var);

        let observed = observed.lock().await;
        let paths = observed
            .iter()
            .map(|request| request.path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            paths,
            vec![
                LOCAL_HTTP_READ_PATH,
                LOCAL_HTTP_X_CALL_PATH,
                LOCAL_HTTP_X_CALL_PATH,
                LOCAL_HTTP_EVENT_SUBSCRIBE_PATH,
                LOCAL_HTTP_EVENT_UNSUBSCRIBE_PATH,
            ]
        );
        assert!(observed
            .iter()
            .all(|request| request.authorization.as_deref() == Some("Bearer secret")));
        assert_eq!(
            observed[0].body["protocol"].as_str(),
            Some(LOCAL_HTTP_ADAPTER_PROTOCOL)
        );
        assert_eq!(observed[0].body["route"]["id"].as_str(), Some("local-obj"));
        assert_eq!(observed[4].body["subscription_id"].as_str(), Some("sub-1"));
    }

    #[derive(Debug)]
    struct ObservedRequest {
        path: String,
        authorization: Option<String>,
        body: Value,
    }

    async fn spawn_test_server() -> (String, Arc<Mutex<Vec<ObservedRequest>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let server_observed = observed.clone();
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    break;
                };
                let observed = server_observed.clone();
                tokio::spawn(async move {
                    handle_test_connection(stream, observed).await;
                });
            }
        });
        (format!("http://{addr}"), observed)
    }

    async fn handle_test_connection(
        mut stream: tokio::net::TcpStream,
        observed: Arc<Mutex<Vec<ObservedRequest>>>,
    ) {
        let mut buffer = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let read = stream.read(&mut chunk).await.unwrap();
            if read == 0 {
                return;
            }
            buffer.extend_from_slice(&chunk[..read]);
            if let Some(header_end) = find_header_end(&buffer) {
                let headers = String::from_utf8_lossy(&buffer[..header_end]);
                let content_length = content_length(&headers);
                let body_start = header_end + 4;
                if buffer.len() >= body_start + content_length {
                    break;
                }
            }
        }

        let header_end = find_header_end(&buffer).unwrap();
        let headers = String::from_utf8_lossy(&buffer[..header_end]).to_string();
        let path = headers
            .lines()
            .next()
            .and_then(|line| line.split_whitespace().nth(1))
            .unwrap()
            .to_string();
        let authorization = headers.lines().find_map(|line| {
            let (name, value) = line.split_once(':')?;
            if name.eq_ignore_ascii_case("authorization") {
                Some(value.trim().to_string())
            } else {
                None
            }
        });
        let body_start = header_end + 4;
        let content_length = content_length(&headers);
        let body: Value =
            serde_json::from_slice(&buffer[body_start..body_start + content_length]).unwrap();
        observed.lock().await.push(ObservedRequest {
            path: path.clone(),
            authorization,
            body: body.clone(),
        });

        let response = match path.as_str() {
            LOCAL_HTTP_READ_PATH => json!({
                "object": body["object"],
                "object_did": null,
                "content": "Item full content",
                "meta": {
                    "title": "Item title",
                    "content_type": "text/plain",
                    "updated_at": "optional"
                },
                "prompt_guidance": [],
                "trust_guidance": [],
                "errors": [],
                "cache_key": body["object"],
                "version": "optional",
                "route": {
                    "id": "external-route",
                    "adapter": "external-adapter"
                },
                "adapt_meta": {"source": "test"}
            }),
            LOCAL_HTTP_X_CALL_PATH if body["action"].as_str() == Some("return_tool") => json!({
                "agent_tool_protocol": "1",
                "status": "pending",
                "cmd_name": "wrong",
                "title": "wrong",
                "summary": "wrong",
                "detail": {"queued": true},
                "output": "queued"
            }),
            LOCAL_HTTP_X_CALL_PATH => json!({
                "status": "success",
                "reserved": true,
                "action": body["action"]
            }),
            LOCAL_HTTP_EVENT_SUBSCRIBE_PATH => json!({
                "subscription_id": "sub-1",
                "object": body["object"],
                "object_did": null,
                "event": body["event"],
                "kevent_pattern": "agent.object.changed",
                "expires_at": null,
                "cursor": null,
                "route": {
                    "id": "external-route",
                    "adapter": "external-adapter"
                }
            }),
            LOCAL_HTTP_EVENT_UNSUBSCRIBE_PATH => json!({"ok": true}),
            _ => json!({"error": "not found"}),
        };
        let status = if response.get("error").is_some() {
            "404 Not Found"
        } else {
            "200 OK"
        };
        let response_body = serde_json::to_vec(&response).unwrap();
        let head = format!(
            "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
            response_body.len()
        );
        stream.write_all(head.as_bytes()).await.unwrap();
        stream.write_all(&response_body).await.unwrap();
    }

    fn find_header_end(buffer: &[u8]) -> Option<usize> {
        buffer.windows(4).position(|window| window == b"\r\n\r\n")
    }

    fn content_length(headers: &str) -> usize {
        headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.eq_ignore_ascii_case("content-length") {
                    value.trim().parse().ok()
                } else {
                    None
                }
            })
            .unwrap_or(0)
    }
}
