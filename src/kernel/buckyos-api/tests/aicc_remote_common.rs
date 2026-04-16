#![allow(dead_code)]

use ::kRPC::*;
use async_trait::async_trait;
use base64::Engine;
use buckyos_api::{
    AiMessage, AiPayload, AiccClient, AiccHandler, AiccServerHandler, CancelResponse, Capability,
    CompleteRequest, CompleteResponse, CompleteStatus, ModelSpec, Requirements, VerifyHubClient,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;

pub fn base_request() -> CompleteRequest {
    CompleteRequest::new(
        Capability::LlmRouter,
        ModelSpec::new("llm.plan.default".to_string(), None),
        Requirements::new(vec!["plan".to_string()], Some(3000), Some(0.2), None),
        AiPayload::new(
            Some("hello".to_string()),
            vec![AiMessage::new(
                "user".to_string(),
                "write summary".to_string(),
            )],
            vec![],
            vec![],
            None,
            Some(json!({"temperature": 0.2})),
        ),
        Some("idem-test".to_string()),
    )
}

#[derive(Default)]
pub struct MockRemoteAicc {
    seq: AtomicU64,
    owner_by_task: Mutex<HashMap<String, Option<String>>>,
}

#[async_trait]
impl AiccHandler for MockRemoteAicc {
    async fn handle_complete(
        &self,
        _request: CompleteRequest,
        ctx: RPCContext,
    ) -> std::result::Result<CompleteResponse, RPCErrors> {
        let id = self.seq.fetch_add(1, Ordering::Relaxed) + 1;
        let task_id = format!("task-{}", id);
        self.owner_by_task
            .lock()
            .expect("owner map lock")
            .insert(task_id.clone(), ctx.token.clone());
        Ok(CompleteResponse::new(
            task_id,
            CompleteStatus::Succeeded,
            None,
            None,
        ))
    }

    async fn handle_cancel(
        &self,
        task_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<CancelResponse, RPCErrors> {
        let owner = self
            .owner_by_task
            .lock()
            .expect("owner map lock")
            .get(task_id)
            .cloned();
        let Some(owner) = owner else {
            return Err(RPCErrors::ReasonError(format!(
                "task {} not found",
                task_id
            )));
        };

        if owner != ctx.token {
            return Err(RPCErrors::ReasonError(
                "NoPermission: cross tenant cancel denied".to_string(),
            ));
        }

        Ok(CancelResponse::new(task_id.to_string(), true))
    }
}

pub struct RpcHttpTestServer {
    pub endpoint: String,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for RpcHttpTestServer {
    fn drop(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
    }
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

pub async fn spawn_rpc_http_server(
    handler: Arc<dyn RPCHandler + Send + Sync>,
) -> RpcHttpTestServer {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind rpc test server");
    let addr = listener.local_addr().expect("rpc test server local addr");
    let (shutdown_tx, mut shutdown_rx) = oneshot::channel::<()>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut shutdown_rx => {
                    break;
                }
                accepted = listener.accept() => {
                    let (mut socket, peer_addr) = match accepted {
                        Ok(pair) => pair,
                        Err(_) => break,
                    };
                    let handler = handler.clone();
                    tokio::spawn(async move {
                        let mut buffer = vec![0u8; 16384];
                        let mut total = 0usize;
                        let mut header_end = None;
                        loop {
                            if total >= buffer.len() {
                                break;
                            }
                            let n = match socket.read(&mut buffer[total..]).await {
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            if n == 0 {
                                return;
                            }
                            total += n;
                            if let Some(pos) = find_header_end(&buffer[..total]) {
                                header_end = Some(pos);
                                break;
                            }
                        }

                        let Some(header_end) = header_end else {
                            return;
                        };

                        let header_bytes = &buffer[..header_end];
                        let headers_text = String::from_utf8_lossy(header_bytes);
                        let mut content_length = 0usize;
                        for line in headers_text.lines() {
                            let lower = line.to_ascii_lowercase();
                            if let Some(v) = lower.strip_prefix("content-length:") {
                                content_length = v.trim().parse::<usize>().unwrap_or(0);
                            }
                        }

                        let body_start = header_end + 4;
                        let body_end = body_start.saturating_add(content_length);
                        if body_end > buffer.len() {
                            buffer.resize(body_end, 0);
                        }
                        while total < body_end {
                            let n = match socket.read(&mut buffer[total..]).await {
                                Ok(n) => n,
                                Err(_) => return,
                            };
                            if n == 0 {
                                return;
                            }
                            total += n;
                        }
                        let body = &buffer[body_start..body_end];

                        let parsed_req: std::result::Result<RPCRequest, _> = serde_json::from_slice(body);
                        let response_body = match parsed_req {
                            Ok(req) => match handler.handle_rpc_call(req, peer_addr.ip()).await {
                                Ok(resp) => serde_json::to_string(&resp)
                                    .unwrap_or_else(|e| format!("{{\"error\":\"{}\"}}", e)),
                                Err(e) => serde_json::json!({"error": e.to_string()}).to_string(),
                            },
                            Err(e) => serde_json::json!({"error": format!("invalid rpc request json: {}", e)}).to_string(),
                        };

                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            response_body.len(),
                            response_body
                        );
                        let _ = socket.write_all(response.as_bytes()).await;
                        let _ = socket.shutdown().await;
                    });
                }
            }
        }
    });

    RpcHttpTestServer {
        endpoint: format!("http://{}/kapi/aicc", addr),
        shutdown: Some(shutdown_tx),
    }
}

pub async fn post_rpc_over_http(
    endpoint: &str,
    req: &RPCRequest,
) -> std::result::Result<RPCResponse, String> {
    let client = reqwest::Client::new();
    let resp = client
        .post(endpoint)
        .json(req)
        .send()
        .await
        .map_err(|e| format!("http request failed: {}", e))?;

    let status = resp.status();
    let value: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("decode response json failed: {}", e))?;
    if !status.is_success() {
        return Err(format!("http status {} body {}", status, value));
    }
    if let Some(err) = value.get("error").and_then(|v| v.as_str()) {
        return Err(err.to_string());
    }
    serde_json::from_value(value).map_err(|e| format!("parse rpc response failed: {}", e))
}

pub struct RpcTestEndpoint {
    pub endpoint: String,
    pub is_remote: bool,
    _local_server: Option<RpcHttpTestServer>,
}

impl RpcTestEndpoint {
    pub fn from_remote(endpoint: String) -> Self {
        Self {
            endpoint,
            is_remote: true,
            _local_server: None,
        }
    }

    pub fn from_local(server: RpcHttpTestServer) -> Self {
        Self {
            endpoint: server.endpoint.clone(),
            is_remote: false,
            _local_server: Some(server),
        }
    }
}

fn first_non_empty_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        std::env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn endpoint_from_host(host: &str, path: &str) -> Option<String> {
    let mut parsed = reqwest::Url::parse(host.trim()).ok()?;
    parsed.set_path(path);
    parsed.set_query(None);
    Some(parsed.to_string())
}

pub fn resolve_endpoint_from_env(
    endpoint_keys: &[&str],
    host_keys: &[&str],
    path: &str,
) -> Option<String> {
    if let Some(endpoint) = first_non_empty_env(endpoint_keys) {
        return Some(endpoint);
    }
    first_non_empty_env(host_keys).and_then(|host| endpoint_from_host(&host, path))
}

pub fn resolve_krpc_aicc_endpoint_from_env() -> Option<String> {
    resolve_endpoint_from_env(&[], &["AICC_KRPC_HOST", "AICC_HOST"], "/kapi/aicc")
}

pub fn resolve_gateway_aicc_endpoint_from_env() -> Option<String> {
    resolve_endpoint_from_env(&[], &["AICC_GATEWAY_HOST", "AICC_HOST"], "/kapi/aicc")
}

pub fn resolve_gateway_system_config_endpoint_from_env() -> Option<String> {
    resolve_gateway_aicc_endpoint_from_env().and_then(|gateway_endpoint| {
        endpoint_from_host(gateway_endpoint.as_str(), "/kapi/system_config")
    })
}

fn verify_hub_endpoints_from_hint(endpoint_hint: Option<&str>) -> Vec<String> {
    let mut endpoints = vec![];

    if let Some(hint) = endpoint_hint {
        if let Ok(mut url) = reqwest::Url::parse(hint.trim()) {
            for path in ["/kapi/verify_hub", "/kapi/verify-hub"] {
                url.set_path(path);
                url.set_query(None);
                let endpoint = url.to_string();
                if !endpoints.contains(&endpoint) {
                    endpoints.push(endpoint);
                }
            }
        }
    }

    for env_key in ["AICC_GATEWAY_HOST", "AICC_HOST"] {
        if let Some(host) = first_non_empty_env(&[env_key]) {
            for path in ["/kapi/verify_hub", "/kapi/verify-hub"] {
                if let Some(endpoint) = endpoint_from_host(host.as_str(), path) {
                    if !endpoints.contains(&endpoint) {
                        endpoints.push(endpoint);
                    }
                }
            }
        }
    }

    endpoints
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn derive_login_password_hash(username: &str, password: &str, login_nonce: u64) -> String {
    let stage1 = {
        let mut hasher = Sha256::new();
        hasher.update(format!("{}{}.buckyos", password, username).as_bytes());
        base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
    };

    let mut hasher = Sha256::new();
    hasher.update(format!("{}{}", stage1, login_nonce).as_bytes());
    base64::engine::general_purpose::STANDARD.encode(hasher.finalize())
}

pub async fn resolve_remote_test_token(
    endpoint_hint: Option<&str>,
) -> std::result::Result<Option<String>, String> {
    if let Some(token) = first_non_empty_env(&["AICC_RPC_TOKEN"]) {
        return Ok(Some(token));
    }

    let username = first_non_empty_env(&[
        "AICC_RPC_USERNAME",
        "AICC_LOGIN_USERNAME",
        "BUCKYOS_USERNAME",
    ]);
    let password = first_non_empty_env(&[
        "AICC_RPC_PASSWORD",
        "AICC_LOGIN_PASSWORD",
        "BUCKYOS_PASSWORD",
    ]);
    let appid = first_non_empty_env(&["AICC_RPC_APPID", "AICC_LOGIN_APPID"])
        .unwrap_or_else(|| "aicc".to_string());
    let (Some(username), Some(password)) = (username, password) else {
        return Ok(None);
    };

    let endpoints = verify_hub_endpoints_from_hint(endpoint_hint);
    if endpoints.is_empty() {
        return Err("cannot resolve verify-hub endpoint for password login".to_string());
    }

    let mut errs = vec![];
    for endpoint in endpoints {
        let login_nonce = now_millis();
        let password_hash = derive_login_password_hash(&username, &password, login_nonce);
        let client = VerifyHubClient::new(kRPC::new(endpoint.as_str(), None));
        match client
            .login_by_password(
                username.clone(),
                password_hash,
                appid.clone(),
                Some(login_nonce),
            )
            .await
        {
            Ok(resp) => {
                let token = resp.session_token.trim().to_string();
                if !token.is_empty() {
                    return Ok(Some(token));
                }
                errs.push(format!("{} => empty session_token", endpoint));
            }
            Err(err) => errs.push(format!("{} => {}", endpoint, err)),
        }
    }

    Err(format!(
        "failed to resolve remote token by password login, tried endpoints: {}",
        errs.join(" | ")
    ))
}

pub async fn resolve_krpc_target() -> RpcTestEndpoint {
    if let Some(endpoint) = resolve_krpc_aicc_endpoint_from_env() {
        return RpcTestEndpoint::from_remote(endpoint);
    }
    let handler = Arc::new(AiccServerHandler::new(MockRemoteAicc::default()));
    let server = spawn_rpc_http_server(handler).await;
    RpcTestEndpoint::from_local(server)
}

pub async fn resolve_gateway_target() -> RpcTestEndpoint {
    if let Some(endpoint) = resolve_gateway_aicc_endpoint_from_env() {
        return RpcTestEndpoint::from_remote(endpoint);
    }
    let handler = Arc::new(AiccServerHandler::new(MockRemoteAicc::default()));
    let server = spawn_rpc_http_server(handler).await;
    RpcTestEndpoint::from_local(server)
}

pub fn build_client(endpoint: &str) -> AiccClient {
    let client = kRPC::new(endpoint, None);
    AiccClient::new(client)
}
