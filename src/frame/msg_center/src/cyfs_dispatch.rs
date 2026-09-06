use crate::msg_center::MessageCenter;
use buckyos_api::{get_buckyos_api_runtime, DeliveryReportResult, DispatchResult};
use buckyos_http_server::ServerError;
use bytes::Bytes;
use http::{Request, Response, StatusCode};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use name_lib::DID;
use ndn_lib::*;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Duration;

type Body = BoxBody<Bytes, ServerError>;

fn default_max_bytes() -> usize {
    65536
}
fn default_timeout() -> u64 {
    3000
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct NativeDispatchRoute {
    pub target: String,
    pub upstream: String,
    pub authorization: String,
    #[serde(default = "default_timeout")]
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct CyfsDispatchSettings {
    pub target_zone: Option<String>,
    pub accepted_paths: HashMap<String, DID>,
    pub principal_dids: HashMap<String, DID>,
    pub outgoing: HashMap<String, NativeDispatchRoute>,
    pub max_object_bytes: usize,
}

impl Default for CyfsDispatchSettings {
    fn default() -> Self {
        Self {
            target_zone: None,
            accepted_paths: HashMap::new(),
            principal_dids: HashMap::new(),
            outgoing: HashMap::new(),
            max_object_bytes: default_max_bytes(),
        }
    }
}

impl CyfsDispatchSettings {
    pub fn parse(settings: &serde_json::Value) -> anyhow::Result<Self> {
        let mut config: Self = settings
            .get("cyfs_dispatch")
            .cloned()
            .map(serde_json::from_value)
            .transpose()?
            .unwrap_or_default();
        if config.max_object_bytes == 0 || config.max_object_bytes > 1024 * 1024 {
            anyhow::bail!("cyfs_dispatch.max_object_bytes must be in 1..=1048576");
        }
        if !config.accepted_paths.is_empty() && config.target_zone.is_none() {
            anyhow::bail!("CYFS receive paths require target_zone");
        }
        if let Some(zone) = &config.target_zone {
            let mut paths = HashMap::new();
            for (path, did) in &config.accepted_paths {
                let target = normalize_cyfs_dispatch_target(zone, path)?;
                let parsed = reqwest::Url::parse(&target)?;
                if paths
                    .insert(parsed.path().to_string(), did.clone())
                    .is_some()
                {
                    anyhow::bail!("duplicate normalized CYFS receive path");
                }
            }
            config.accepted_paths = paths;
        }
        for (did, route) in &config.outgoing {
            DID::from_str(did).map_err(|e| anyhow::anyhow!("invalid native recipient: {e}"))?;
            let target = reqwest::Url::parse(&route.target)?;
            if target.scheme() != "cyfs"
                || target.query().is_some()
                || target.fragment().is_some()
                || normalize_cyfs_dispatch_target(target.host_str().unwrap_or(""), target.path())?
                    != route.target
            {
                anyhow::bail!("native dispatch target must be a canonical CYFS target");
            }
            let upstream = reqwest::Url::parse(&route.upstream)?;
            if !matches!(upstream.scheme(), "http" | "https")
                || upstream.host_str().is_none()
                || upstream.path() != "/"
                || upstream.query().is_some()
                || upstream.fragment().is_some()
                || !upstream.username().is_empty()
                || upstream.password().is_some()
                || route.timeout_ms == 0
                || route.authorization.is_empty()
                || http::HeaderValue::from_str(&route.authorization).is_err()
            {
                anyhow::bail!("native dispatch requires an http(s) origin, authorization and positive timeout");
            }
        }
        Ok(config)
    }
}

pub(crate) fn response(status: StatusCode, result: &CyfsDispatchResult) -> Response<Body> {
    let mut response = Response::new(
        Full::new(Bytes::from(serde_json::to_vec(result).unwrap()))
            .map_err(|e| match e {})
            .boxed(),
    );
    *response.status_mut() = status;
    result.apply_headers(response.headers_mut());
    response
}

fn rejected(code: StatusCode, target: &str, id: Option<ObjId>, reason: &str) -> Response<Body> {
    response(
        code,
        &CyfsDispatchResult::rejected(id, target.into(), reason, code.is_server_error()),
    )
}

fn unknown(code: StatusCode, target: &str, reason: &str) -> Response<Body> {
    let mut response = Response::new(
        Full::new(Bytes::from(
            serde_json::json!({"target":target,"error":reason}).to_string(),
        ))
        .map_err(|e| match e {})
        .boxed(),
    );
    *response.status_mut() = code;
    response
        .headers_mut()
        .insert(CYFS_HEADER_DISPATCH_ERROR, reason.parse().unwrap());
    response
        .headers_mut()
        .insert("cache-control", "no-store".parse().unwrap());
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    response
}

fn dispatch_result(result: DispatchResult, target: String, receiver: &DID) -> CyfsDispatchResult {
    if result.ok
        && (result.delivered_recipients.contains(receiver)
            || result.delivered_group.as_ref() == Some(receiver))
    {
        CyfsDispatchResult::new(Some(result.msg_id), target, CyfsDispatchStatus::Accepted)
    } else {
        CyfsDispatchResult::rejected(Some(result.msg_id), target, "acl-denied", false)
    }
}

pub(crate) async fn serve(center: &MessageCenter, mut req: Request<Body>) -> Response<Body> {
    let config = center.cyfs_dispatch.read().unwrap().clone();
    let host = req
        .headers()
        .get("host")
        .and_then(|v| v.to_str().ok())
        .or_else(|| req.uri().host())
        .unwrap_or("");
    let target = match normalize_cyfs_dispatch_target(host, req.uri().path()) {
        Ok(target) => target,
        Err(_) => return rejected(StatusCode::BAD_REQUEST, "", None, "invalid-target"),
    };
    let expected = config
        .target_zone
        .as_deref()
        .and_then(|zone| normalize_cyfs_dispatch_target(zone, req.uri().path()).ok());
    if expected.as_deref() != Some(&target) {
        return rejected(StatusCode::NOT_FOUND, &target, None, "no-handler");
    }
    let parsed = reqwest::Url::parse(&target).unwrap();
    let receiver = match config.accepted_paths.get(parsed.path()) {
        Some(receiver) if center.is_local_recipient(receiver) => receiver,
        _ => return rejected(StatusCode::NOT_FOUND, &target, None, "no-handler"),
    };
    if req.method() != http::Method::PUT && req.method() != http::Method::GET {
        return rejected(
            StatusCode::METHOD_NOT_ALLOWED,
            &target,
            None,
            "method-not-allowed",
        );
    }
    let token = match req
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(token) => token,
        None => return rejected(StatusCode::UNAUTHORIZED, &target, None, "unauthenticated"),
    };
    let runtime = match get_buckyos_api_runtime() {
        Ok(runtime) => runtime,
        Err(_) => {
            return unknown(
                StatusCode::SERVICE_UNAVAILABLE,
                &target,
                "authentication-unavailable",
            )
        }
    };
    let principal = match runtime.verify_trusted_session_token(token).await {
        Ok(token) => match token.sub.and_then(|s| {
            config
                .principal_dids
                .get(&s)
                .cloned()
                .or_else(|| DID::from_str(&s).ok())
        }) {
            Some(principal) => principal,
            None => return rejected(StatusCode::UNAUTHORIZED, &target, None, "invalid-principal"),
        },
        Err(_) => return rejected(StatusCode::UNAUTHORIZED, &target, None, "unauthenticated"),
    };
    if req.method() == http::Method::GET {
        let id = match parse_cyfs_dispatch_status_query(req.uri().query().unwrap_or("")) {
            Ok(id) => id,
            Err(_) => return rejected(StatusCode::BAD_REQUEST, &target, None, "invalid-query"),
        };
        return match center
            .query_cyfs_dispatch(&principal.to_string(), &target, &id)
            .await
        {
            Ok(Some(result)) => {
                let mut result = dispatch_result(result, target, receiver);
                result.source = Some(CyfsDispatchSource::Upstream);
                response(StatusCode::OK, &result)
            }
            Ok(None) => unknown(StatusCode::NOT_FOUND, &target, CYFS_DISPATCH_ERROR_UNKNOWN),
            Err(_) => unknown(
                StatusCode::SERVICE_UNAVAILABLE,
                &target,
                "status-unavailable",
            ),
        };
    }
    if req.uri().query().is_some() {
        return rejected(StatusCode::BAD_REQUEST, &target, None, "invalid-query");
    }
    if !req
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(is_cyfs_named_object_content_type)
        || req.headers().contains_key("content-encoding")
    {
        return rejected(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            &target,
            None,
            "unsupported-content-type",
        );
    }
    let claimed = req
        .headers()
        .get(CYFS_HEADER_OBJ_ID)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    if req.headers().get("content-length").is_some_and(|v| {
        v.to_str()
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .is_none_or(|v| v > config.max_object_bytes)
    }) {
        return rejected(
            StatusCode::PAYLOAD_TOO_LARGE,
            &target,
            None,
            "object-too-large",
        );
    }
    let mut body = Vec::new();
    while let Some(frame) = req.body_mut().frame().await {
        match frame {
            Ok(frame) => {
                if let Some(bytes) = frame.data_ref() {
                    if bytes.len() > config.max_object_bytes - body.len() {
                        return rejected(
                            StatusCode::PAYLOAD_TOO_LARGE,
                            &target,
                            None,
                            "object-too-large",
                        );
                    }
                    body.extend_from_slice(bytes);
                }
            }
            Err(_) => return rejected(StatusCode::BAD_REQUEST, &target, None, "body-read-failed"),
        }
    }
    let id = match validate_cyfs_dispatch_object(&body, claimed.as_deref()) {
        Ok(id) => id,
        Err(_) => return rejected(StatusCode::BAD_REQUEST, &target, None, "invalid-object"),
    };
    let msg: MsgObject = match serde_json::from_slice(&body) {
        Ok(msg) => msg,
        Err(_) => {
            return rejected(
                StatusCode::BAD_REQUEST,
                &target,
                Some(id),
                "invalid-message",
            )
        }
    };
    if msg.gen_obj_id().0 != id {
        return rejected(
            StatusCode::BAD_REQUEST,
            &target,
            Some(id),
            "invalid-message-object-id",
        );
    }
    if msg.from != principal || !msg.to.contains(receiver) {
        return rejected(
            StatusCode::FORBIDDEN,
            &target,
            Some(id),
            "sender-or-target-mismatch",
        );
    }
    match center
        .dispatch_to_receiver(msg, receiver.clone(), target.clone())
        .await
    {
        Ok(result) => {
            let result = dispatch_result(result, target, receiver);
            response(
                if result.status == CyfsDispatchStatus::Accepted {
                    StatusCode::OK
                } else {
                    StatusCode::FORBIDDEN
                },
                &result,
            )
        }
        Err(kRPC::RPCErrors::NoPermission(_)) => {
            rejected(StatusCode::FORBIDDEN, &target, Some(id), "acl-denied")
        }
        Err(error) => {
            log::warn!("CYFS receive transaction failed: {}", error);
            unknown(
                StatusCode::SERVICE_UNAVAILABLE,
                &target,
                CYFS_DISPATCH_ERROR_OUTCOME_UNKNOWN,
            )
        }
    }
}

pub(crate) fn delivery_report(result: CyfsDispatchResult) -> DeliveryReportResult {
    match result.status {
        CyfsDispatchStatus::Accepted => DeliveryReportResult {
            ok: true,
            external_msg_id: result.obj_id.map(|id| id.to_string()),
            ..Default::default()
        },
        CyfsDispatchStatus::Cached => DeliveryReportResult {
            ok: false,
            error_code: Some("cyfs-cached".into()),
            error_message: Some("Receiver cached the object; acceptance is pending".into()),
            retryable: Some(true),
            retry_after_ms: Some(30_000),
            ..Default::default()
        },
        CyfsDispatchStatus::Rejected => DeliveryReportResult {
            ok: false,
            error_code: Some("cyfs-rejected".into()),
            error_message: result.reason,
            retryable: result.retryable,
            ..Default::default()
        },
    }
}

pub(crate) async fn send(
    route: &NativeDispatchRoute,
    msg: &MsgObject,
    expected_id: &ObjId,
) -> anyhow::Result<DeliveryReportResult> {
    let (id, body) = msg.gen_obj_id();
    if &id != expected_id {
        anyhow::bail!("native delivery body does not match the queued ObjectId");
    }
    let target = reqwest::Url::parse(&route.target)?;
    let mut url = reqwest::Url::parse(&route.upstream)?;
    url.set_path(target.path());
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let operation = async {
        let mut response = client
            .put(url)
            .header("host", target.host_str().unwrap())
            .header("content-type", CYFS_CONTENT_TYPE_NAMED_OBJECT_JSON)
            .header(CYFS_HEADER_OBJ_ID, id.to_string())
            .header(CYFS_HEADER_ORIGINAL_USER, msg.from.to_string())
            .header("authorization", &route.authorization)
            .body(body)
            .send()
            .await?;
        let status = response.status();
        let headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len() + chunk.len() > 65536 {
                anyhow::bail!("dispatch response too large");
            }
            bytes.extend_from_slice(&chunk);
        }
        let result = parse_cyfs_dispatch_result(
            status.as_u16(),
            &headers,
            &bytes,
            &id,
            &route.target,
            false,
        )?;
        Ok::<_, anyhow::Error>(delivery_report(result))
    };
    tokio::time::timeout(Duration::from_millis(route.timeout_ms), operation).await?
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[tokio::test]
    async fn cyfs_dispatch_sender_replays_identical_object_until_accepted() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let route = NativeDispatchRoute {
            target: "cyfs://alice.example/inbox".into(),
            upstream: format!("http://{}", listener.local_addr().unwrap()),
            authorization: "Bearer original-token".into(),
            timeout_ms: 1000,
        };
        let msg = MsgObject {
            from: DID::new("bns", "bob"),
            to: vec![DID::new("bns", "alice")],
            created_at_ms: 1,
            ..Default::default()
        };
        let (id, original) = msg.gen_obj_id();
        let server_id = id.clone();
        let task = tokio::spawn(async move {
            for (status, code) in [
                (CyfsDispatchStatus::Cached, "202 Accepted"),
                (CyfsDispatchStatus::Accepted, "200 OK"),
            ] {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                loop {
                    let mut chunk = [0u8; 4096];
                    let size = socket.read(&mut chunk).await.unwrap();
                    assert!(size > 0);
                    request.extend_from_slice(&chunk[..size]);
                    if let Some(split) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        if request.len() >= split + 4 + original.len() {
                            let headers = std::str::from_utf8(&request[..split])
                                .unwrap()
                                .to_ascii_lowercase();
                            assert!(headers.starts_with("put /inbox http/1.1"));
                            assert!(headers.contains("host: alice.example"));
                            assert!(headers.contains("authorization: bearer original-token"));
                            assert_eq!(&request[split + 4..], original.as_bytes());
                            break;
                        }
                    }
                }
                let result = CyfsDispatchResult::new(
                    Some(server_id.clone()),
                    "cyfs://alice.example/inbox".into(),
                    status,
                );
                let body = serde_json::to_string(&result).unwrap();
                let response = format!("HTTP/1.1 {code}\r\nContent-Type: application/json\r\nCache-Control: no-store\r\ncyfs-dispatch-status: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", status.as_str(), body.len());
                socket.write_all(response.as_bytes()).await.unwrap();
            }
        });
        let pending = send(&route, &msg, &id).await.unwrap();
        assert!(!pending.ok);
        assert_eq!(pending.retryable, Some(true));
        assert_eq!(pending.error_code.as_deref(), Some("cyfs-cached"));
        let accepted = send(&route, &msg, &id).await.unwrap();
        assert!(accepted.ok);
        assert_eq!(
            accepted.external_msg_id.as_deref(),
            Some(id.to_string().as_str())
        );
        task.await.unwrap();
    }

    #[test]
    fn cyfs_dispatch_settings_reject_ambiguous_routes() {
        for target in [
            "cyfs://alice.example/inbox/@/field",
            "cyfs://alice.example/a/../inbox",
            "http://alice.example/inbox",
        ] {
            assert!(CyfsDispatchSettings::parse(&serde_json::json!({"cyfs_dispatch":{
                "outgoing":{"did:bns:alice":{"target":target,"upstream":"http://localhost:4050","authorization":"Bearer token"}}
            }})).is_err());
        }
        assert!(CyfsDispatchSettings::parse(
            &serde_json::json!({"cyfs_dispatch":{"accepted_paths":{"/inbox":"did:bns:alice"}}})
        )
        .is_err());
    }
}
