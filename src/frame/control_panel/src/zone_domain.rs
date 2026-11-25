//! `zone.domain.*`: the Zone owner's domain and certificate settings. The node
//! does the work; control-panel checks the caller and forwards the request as
//! it is, with the caller's own token. Params carry private keys and DNS
//! service tokens: never log them.

use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{kRPC, RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{KEVENT_SERVICE_MAIN_PORT, ZONE_OWNER_USER_ID_KEY};
use log::warn;
use serde_json::{Map, Value};

pub(crate) const ZONE_DOMAIN_PREFIX: &str = "zone.domain.";
/// Writing records and waiting for a test challenge both take minutes.
const FORWARD_TIMEOUT_SECS: u64 = 600;
const REMOTE_FAILURE_PREFIX: &str = "rpc call error: ";

struct DomainMethod {
    node_method: &'static str,
    sudo: bool,
}

fn domain_method(method: &str) -> Option<DomainMethod> {
    let (node_method, sudo) = match method.strip_prefix(ZONE_DOMAIN_PREFIX)? {
        "status" => ("domain.status", false),
        "providers" => ("domain.providers", false),
        "check" => ("domain.check", true),
        "inspect_certificate" => ("domain.inspect_certificate", true),
        "plan" => ("domain.plan", true),
        "apply" => ("domain.apply", true),
        "verify" => ("domain.verify", true),
        "update" => ("domain.update", true),
        _ => return None,
    };
    Some(DomainMethod { node_method, sudo })
}

fn denied(code: &str, message: &str) -> RPCErrors {
    RPCErrors::NoPermission(format!("[domain:{code}] {message}"))
}

async fn authorize(
    kv: &dyn AgentKv,
    principal: &RpcAuthPrincipal,
    sudo: bool,
) -> Result<(), RPCErrors> {
    if !principal.is_user_session || !principal.is_control_panel_session {
        return Err(denied(
            "session_not_allowed",
            "the Zone domain needs a user-authenticated Control Panel session",
        ));
    }
    let owner = kv
        .get(ZONE_OWNER_USER_ID_KEY)
        .await?
        .and_then(|(raw, _)| serde_json::from_str::<String>(&raw).ok());
    if owner.as_deref() != Some(principal.username.as_str()) {
        return Err(denied(
            "not_owner",
            "only the Zone owner can manage the Zone domain",
        ));
    }
    if sudo && !principal.session.is_sudo {
        return Err(denied(
            "sudo_required",
            "sudo permission is required to change the Zone domain",
        ));
    }
    Ok(())
}

async fn forward(
    url: &str,
    method: &DomainMethod,
    params: Map<String, Value>,
    token: String,
    seq: u64,
) -> Result<RPCResponse, RPCErrors> {
    let client = kRPC::new_with_timeout_secs(url, Some(token), FORWARD_TIMEOUT_SECS);
    match client.call(method.node_method, Value::Object(params)).await {
        Ok(result) => Ok(RPCResponse::new(RPCResult::Success(result), seq)),
        Err(error @ RPCErrors::Detailed(_)) => Err(error),
        Err(RPCErrors::ReasonError(text)) => {
            let text = text
                .strip_prefix(REMOTE_FAILURE_PREFIX)
                .map(str::to_string)
                .unwrap_or(text);
            Ok(RPCResponse::new(RPCResult::Failed(text), seq))
        }
        Err(error) => {
            warn!("{} failed: {error}", method.node_method);
            Err(RPCErrors::ReasonError(
                "[domain:node_unavailable] the node service did not answer".to_string(),
            ))
        }
    }
}

impl ControlPanelServer {
    pub(crate) async fn handle_zone_domain(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let method = domain_method(&req.method)
            .ok_or_else(|| RPCErrors::UnknownMethod(req.method.clone()))?;
        authorize(&ZoneKv::connect().await?, principal, method.sudo).await?;
        let token = req
            .token
            .clone()
            .ok_or_else(|| RPCErrors::InvalidToken("missing caller session token".to_string()))?;
        forward(
            &format!("http://127.0.0.1:{KEVENT_SERVICE_MAIN_PORT}/kapi/node_daemon"),
            &method,
            Self::typed_params(&req)?,
            token,
            req.seq,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_mgr::tests::MemoryKv;
    use buckyos_api::{TokenPrincipalKind, UserType};
    use buckyos_kit::KVAction;
    use serde_json::json;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn principal(user: &str, sudo: bool) -> RpcAuthPrincipal {
        RpcAuthPrincipal {
            username: user.into(),
            owner_user_id: user.into(),
            owner_did: format!("did:bns:{user}"),
            is_user_session: true,
            is_control_panel_session: true,
            authenticated_app_id: "system:control-panel".into(),
            principal_kind: TokenPrincipalKind::User,
            user_type: UserType::Admin,
            agent_permission_mode: None,
            session: crate::PrincipalSession {
                is_sudo: sudo,
                ..Default::default()
            },
        }
    }

    async fn zone_of(owner: &str) -> MemoryKv {
        let kv = MemoryKv::default();
        kv.exec_tx(
            HashMap::from([(
                ZONE_OWNER_USER_ID_KEY.to_string(),
                KVAction::Create(json!(owner).to_string()),
            )]),
            None,
        )
        .await
        .unwrap();
        kv
    }

    #[test]
    fn only_the_contract_methods_exist_and_reads_need_no_sudo() {
        for (name, sudo) in [
            ("status", false),
            ("providers", false),
            ("check", true),
            ("inspect_certificate", true),
            ("plan", true),
            ("apply", true),
            ("verify", true),
            ("update", true),
        ] {
            let method = domain_method(&format!("zone.domain.{name}")).unwrap();
            assert_eq!(method.node_method, format!("domain.{name}"));
            assert_eq!(method.sudo, sudo, "{name}");
        }
        for method in [
            "zone.domain.",
            "zone.domain.bind",
            "zone.domain.status.x",
            "domain.status",
        ] {
            assert!(domain_method(method).is_none(), "{method}");
        }
    }

    #[tokio::test]
    async fn only_the_zone_owner_in_a_control_panel_session_gets_through() {
        let kv = zone_of("alice").await;
        let code = |error: RPCErrors| match error {
            RPCErrors::NoPermission(text) => text
                .strip_prefix("[domain:")
                .and_then(|text| text.split(']').next())
                .unwrap()
                .to_string(),
            other => panic!("{other}"),
        };

        authorize(&kv, &principal("alice", false), false)
            .await
            .unwrap();
        authorize(&kv, &principal("alice", true), true)
            .await
            .unwrap();
        assert_eq!(
            code(
                authorize(&kv, &principal("alice", false), true)
                    .await
                    .unwrap_err()
            ),
            "sudo_required"
        );
        for sudo in [false, true] {
            assert_eq!(
                code(
                    authorize(&kv, &principal("bob", true), sudo)
                        .await
                        .unwrap_err()
                ),
                "not_owner"
            );
        }

        let mut app_session = principal("alice", true);
        app_session.is_control_panel_session = false;
        assert_eq!(
            code(authorize(&kv, &app_session, false).await.unwrap_err()),
            "session_not_allowed"
        );
        let mut agent = principal("xiaobai.example.com", true);
        agent.owner_user_id = "alice".into();
        agent.is_user_session = false;
        assert_eq!(
            code(authorize(&kv, &agent, false).await.unwrap_err()),
            "session_not_allowed"
        );
        assert_eq!(
            code(
                authorize(&MemoryKv::default(), &principal("alice", true), false)
                    .await
                    .unwrap_err()
            ),
            "not_owner"
        );
    }

    type Seen = Arc<Mutex<Vec<RPCRequest>>>;

    async fn node_daemon(answer: fn(u64) -> Value) -> (String, Seen) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let seen = Seen::default();
        let recorded = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut bytes = Vec::new();
                let request = loop {
                    let mut chunk = [0; 4096];
                    let count = stream.read(&mut chunk).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&chunk[..count]);
                    let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") else {
                        continue;
                    };
                    let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    assert!(headers.starts_with("post /kapi/node_daemon "));
                    let length: usize = headers
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    if bytes.len() >= end + 4 + length {
                        break serde_json::from_slice::<RPCRequest>(
                            &bytes[end + 4..end + 4 + length],
                        )
                        .unwrap();
                    }
                };
                let body = answer(request.seq).to_string();
                recorded.lock().unwrap().push(request);
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
            }
        });
        (format!("http://{address}/kapi/node_daemon"), seen)
    }

    async fn call(url: &str, method: &str, params: Value) -> Result<RPCResponse, RPCErrors> {
        let mut req = RPCRequest::new(method, params);
        req.seq = 7;
        forward(
            url,
            &domain_method(method).unwrap(),
            ControlPanelServer::typed_params(&req)?,
            "caller-token".to_string(),
            req.seq,
        )
        .await
    }

    #[tokio::test]
    async fn params_token_and_result_pass_through_unchanged() {
        let (url, seen) = node_daemon(|seq| {
            json!({"result": {"credential_id": null, "results": [{"code": "credential_rejected"}]}, "sys": [seq]})
        })
        .await;
        let params = json!({
            "mode": "dns_api", "certificate": "acme", "provider": "cloudflare",
            "fields": {"api_token": "secret"}, "confirm": ["home.example.com"],
        });
        let mut sent = params.clone();
        for (key, value) in [
            ("x_req_host", "home.example.com"),
            ("x_req_origin", "https://home.example.com"),
            ("session_token", "caller-token"),
        ] {
            sent[key] = json!(value);
        }
        let response = call(&url, "zone.domain.apply", sent).await.unwrap();
        assert_eq!(response.seq, 7);
        assert_eq!(
            match response.result {
                RPCResult::Success(value) => value,
                RPCResult::Failed(error) => panic!("{error}"),
            },
            json!({"credential_id": null, "results": [{"code": "credential_rejected"}]})
        );
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "domain.apply");
        assert_eq!(seen[0].token.as_deref(), Some("caller-token"));
        assert_eq!(seen[0].params, params);
    }

    #[tokio::test]
    async fn node_failures_keep_their_text_and_an_absent_node_is_named() {
        let (url, _) = node_daemon(|seq| {
            json!({"error": "Failed due to reason: [domain:domain_not_ready] {\"items\":[]}", "sys": [seq]})
        })
        .await;
        match call(&url, "zone.domain.update", json!({}))
            .await
            .unwrap()
            .result
        {
            RPCResult::Failed(text) => assert_eq!(
                text,
                "Failed due to reason: [domain:domain_not_ready] {\"items\":[]}"
            ),
            RPCResult::Success(value) => panic!("{value}"),
        }

        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let closed = format!("http://{}/kapi/node_daemon", listener.local_addr().unwrap());
        drop(listener);
        let error = call(&closed, "zone.domain.status", json!({}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("[domain:node_unavailable]"), "{error}");
    }
}
