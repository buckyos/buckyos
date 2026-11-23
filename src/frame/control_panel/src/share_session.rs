//! Browser proofs are host-only, short lived and usable for one invitation.
//! No root session is needed to review or accept a resource invitation.
use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::exclusive_invite::random_token;
use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, resource_share_acceptance_key, AccessDenial, AuthMethod, DenyReason,
    ResourceSessionRequest,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use serde_json::json;
use sha2::{Digest, Sha256};

pub(crate) struct NativeProof {
    did: String,
    purpose: String,
    id: String,
    until: u64,
}

pub(super) fn proof_cookie(kind: &str, id: &str, https: bool) -> String {
    let prefix = if https { "__Host-" } else { "" };
    format!("{prefix}buckyos_{kind}_{:x}", Sha256::digest(id.as_bytes()))
}

pub(super) fn attach_proofs(req: &mut RPCRequest, cookies: Option<&str>) {
    let Some(params) = req.params.as_object_mut() else {
        return;
    };
    params.remove("__native_proof");
    params.remove("__oauth_proof");
    let Some(id) = params
        .get("invite_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
    else {
        return;
    };
    let https = params
        .get(crate::REQ_ORIGIN_PARAM)
        .and_then(|v| v.as_str())
        .is_some_and(|origin| origin.starts_with("https://"));
    let native = match req.method.as_str() {
        "auth.share.accept" | "auth.share.identity" => "native_share",
        "user.invite.accept" | "user.invite.get" => "native_user_invite",
        _ => return,
    };
    for (field, kind) in [("__native_proof", native), ("__oauth_proof", "oauth_share")] {
        let name = proof_cookie(kind, &id, https);
        if let Some(value) = cookies.and_then(|c| {
            c.split(';')
                .filter_map(|item| item.trim().split_once('='))
                .find(|(key, _)| *key == name)
                .map(|(_, v)| v)
        }) {
            params.insert(field.into(), json!(value));
        }
    }
}

impl ControlPanelServer {
    pub(crate) async fn save_native_invite_identity(
        &self,
        purpose: &str,
        id: &str,
        did: String,
        origin: &str,
    ) -> Result<String, RPCErrors> {
        let now = buckyos_get_unix_timestamp();
        let secret = random_token(32);
        let mut cache = self.native_invite_proofs.lock().await;
        cache.retain(|_, p| p.until > now);
        if cache.len() >= 4096 {
            return Err(RPCErrors::ReasonError(
                "too many pending confirmations".into(),
            ));
        }
        cache.insert(
            secret.clone(),
            NativeProof {
                did,
                purpose: purpose.into(),
                id: id.into(),
                until: now + 300,
            },
        );
        let name = proof_cookie(
            &format!("native_{purpose}"),
            id,
            origin.starts_with("https://"),
        );
        let secure = if origin.starts_with("https://") {
            "; Secure"
        } else {
            ""
        };
        Ok(format!(
            "{name}={secret}; Path=/; HttpOnly; SameSite=Strict; Max-Age=300{secure}"
        ))
    }

    pub(crate) async fn native_invite_identity(
        &self,
        req: &RPCRequest,
        purpose: &str,
        invite_id: &str,
    ) -> Option<String> {
        let key = req.params.get("__native_proof")?.as_str()?;
        self.native_invite_proofs
            .lock()
            .await
            .get(key)
            .filter(|p| {
                p.until > buckyos_get_unix_timestamp() && p.id == invite_id && p.purpose == purpose
            })
            .map(|p| p.did.clone())
    }

    pub(crate) async fn handle_share_session(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let id = Self::require_param_str(&req, "invite_id")?;
        let runtime = get_buckyos_api_runtime()?;
        let kv = ZoneKv::connect().await?;
        let share = crate::resource_share::load_share(&kv, &id).await?;
        let account = principal
            .filter(|p| p.is_user_session && p.is_control_panel_session && !p.session.is_external);
        if principal.is_some() && account.is_none() {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed).into_rpc_error());
        }
        let session_did = match account {
            Some(p) => Some(
                buckyos_api::ControlPanelClient::from_shared(
                    runtime.get_system_config_client().await?,
                )
                .get_user_did(&p.username)
                .await?
                .to_string(),
            ),
            None => None,
        };
        let proof_did = self.native_invite_identity(&req, "share", &id).await;
        let native_account = match &proof_did {
            Some(did) => crate::resource_share::native_bound_account_did(&kv, did).await?,
            None => None,
        };
        let native = match (&proof_did, &native_account) {
            (Some(did), Some(account))
                if kv
                    .get(&resource_share_acceptance_key(&id, did))
                    .await?
                    .is_none() =>
            {
                Some(account.clone())
            }
            _ => proof_did,
        };
        let oauth = match req.params.get("__oauth_proof").and_then(|v| v.as_str()) {
            Some(proof) => Some(crate::oauth::share_identity(proof, &id).await?),
            None => None,
        };
        let oauth_did = if let Some(identity) = &oauth {
            Some(crate::oauth::share_subject_did(&kv, &id, identity).await?)
        } else {
            None
        };
        if req.method == "auth.share.identity" {
            let settings = crate::sso_settings::load_sso_settings(&kv).await?;
            let source = match &share.audience {
                buckyos_api::ShareAudience::Directed { did } => {
                    crate::oauth::external_source_for_did(&settings, did)
                }
                _ => None,
            };
            let mut mismatches = Vec::new();
            if let buckyos_api::ShareAudience::Directed { did: recipient } = &share.audience {
                let held = [
                    ("session", session_did.as_ref()),
                    ("native", native.as_ref()),
                    ("oauth", oauth_did.as_ref()),
                ];
                if held
                    .iter()
                    .any(|(_, did)| did.is_some_and(|did| did != recipient))
                {
                    let bound = if source.is_some() {
                        crate::oauth::bound_account_did(&kv, recipient).await?
                    } else {
                        crate::resource_share::native_bound_account_did(&kv, recipient).await?
                    };
                    for (source, did) in held {
                        if did.is_some_and(|did| did != recipient && Some(did) != bound.as_ref()) {
                            mismatches.push(source);
                        }
                    }
                }
            }
            return Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "native":native, "oauth":oauth_did, "session":session_did,
                    "required_source":source, "mismatches":mismatches
                })),
                req.seq,
            ));
        }
        let missing = || AccessDenial::new(DenyReason::CredentialState).into_rpc_error();
        let source = Self::require_param_str(&req, "source")?;
        let (did, method, upstream) = match source.as_str() {
            "session" => (
                session_did.clone().ok_or_else(missing)?,
                account
                    .and_then(|p| p.session.auth_method)
                    .ok_or_else(missing)?,
                None,
            ),
            "native" => (native.ok_or_else(missing)?, AuthMethod::DidFederation, None),
            "oauth" => (
                oauth_did.ok_or_else(missing)?,
                AuthMethod::OAuth,
                oauth.as_ref(),
            ),
            _ => return Err(missing()),
        };
        let linked_external = match session_did.as_ref().filter(|held| *held != &did) {
            Some(held) if upstream.is_some() => {
                crate::oauth::bound_account_did(&kv, &did).await?.as_ref() == Some(held)
            }
            Some(held) if source == "native" => native_account.as_ref() == Some(held),
            _ => false,
        };
        if session_did.as_ref().is_some_and(|held| held != &did) && !linked_external {
            return Err(AccessDenial::new(DenyReason::CredentialState)
                .with_detail("identity_conflict")
                .into_rpc_error());
        }
        let resolved = Self::resolve_sso_auth_target(
            &share.target_url,
            &Self::sso_zone_host(runtime)?,
            !runtime.force_https,
            runtime.node_gateway_port,
        )?;
        if resolved.auth_target != share.target {
            return Err(AccessDenial::new(DenyReason::EntryUnavailable).into_rpc_error());
        }
        let hub = runtime.get_verify_hub_client().await?;
        let parent_session = account
            .filter(|_| !linked_external)
            .and_then(|_| Self::extract_rpc_session_token(&req));
        let oauth_source_id = if method != AuthMethod::OAuth {
            None
        } else {
            upstream
                .map(|identity| identity.source_id.clone())
                .or_else(|| {
                    parent_session.as_ref().and_then(|raw| {
                        ::kRPC::RPCSessionToken::from_string(raw)
                            .ok()?
                            .extra
                            .get("oauth_source_id")?
                            .as_str()
                            .map(str::to_owned)
                    })
                })
        };
        if method == AuthMethod::OAuth
            && !crate::sso_settings::load_sso_settings(&kv)
                .await?
                .oauth_sources
                .iter()
                .any(|s| s.enabled && Some(&s.id) == oauth_source_id.as_ref())
        {
            return Err(AccessDenial::new(DenyReason::ZoneMethodDisabled).into_rpc_error());
        }
        crate::resource_share::accept_resource_share(&share, &did, method, upstream).await?;
        let pair = hub
            .issue_resource_share_session(ResourceSessionRequest {
                invite_id: id,
                did,
                target: share.target.clone(),
                auth_method: method,
                oauth_source_id,
                parent_session,
            })
            .await?;
        Self::validate_sso_token_pair(&pair.session_token, &pair.refresh_token, &share.target)?;
        let nonce = self
            .store_new_pending_sso_login(pair.session_token, pair.refresh_token, &resolved)
            .await;
        let callback_url = Self::sso_callback_url(&resolved, nonce)?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!({"status":"ready","callback_url":callback_url})),
            req.seq,
        ))
    }
}
