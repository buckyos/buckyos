use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::exclusive_invite::{random_token, validate_return_path};
use crate::federation::{NameServiceDirectory, ZoneDirectory};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use buckyos_api::{
    get_buckyos_api_runtime, invite_secret_digest, resource_share_acceptance_key,
    resource_share_key, user_app_spec_key, valid_resource_share_id, AppInstanceId, AppServiceSpec,
    AuthMethod, AuthTarget, ResourceShare, ResourceShareAcceptance, ShareAudience, ShareLevel,
    TokenPrincipalKind,
};
use buckyos_kit::{buckyos_get_unix_timestamp, KVAction};
use kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use name_lib::DID;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

const CALLBACK_PREFIX: &str = "services/control_panel/resource_share_callbacks";
const REQUESTS_PREFIX: &str = "services/control_panel/resource_share_requests";

fn denied(reason: &str) -> RPCErrors {
    RPCErrors::NoPermission(reason.into())
}

async fn read<T: DeserializeOwned>(kv: &dyn AgentKv, key: &str) -> Result<Option<T>, RPCErrors> {
    kv.get(key)
        .await?
        .map(|(raw, _)| {
            serde_json::from_str(&raw)
                .map_err(|e| RPCErrors::ReasonError(format!("invalid {key}: {e}")))
        })
        .transpose()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Issue {
    request_id: String,
    resource_id: String,
    audience: IssueAudience,
    recipient_email: Option<RecipientEmail>,
    callback_secret: String,
    return_path: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipientEmail {
    source_id: String,
    email: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum IssueAudience {
    Directed { did: Option<String> },
    Open,
}

#[derive(Deserialize, Serialize, PartialEq, Eq)]
struct ResourceFacts {
    sharer: String,
    title: String,
    operations: Vec<String>,
    methods: buckyos_api::EntryMethods,
    expires_at: Option<u64>,
}

#[derive(Deserialize, Serialize)]
struct RequestIndex {
    invite_id: String,
    digest: String,
}

fn issue_target(principal: &RpcAuthPrincipal) -> Result<AuthTarget, RPCErrors> {
    let target = AuthTarget::from_canonical_key(&principal.authenticated_app_id)
        .map_err(RPCErrors::InvalidToken)?;
    match (&target, principal.principal_kind) {
        (AuthTarget::App { app_instance_id }, TokenPrincipalKind::App)
            if app_instance_id.owner_user_id() == principal.username
                && !principal.session.is_sudo =>
        {
            Ok(target)
        }
        (
            AuthTarget::System { service_id },
            TokenPrincipalKind::System | TokenPrincipalKind::Device,
        ) if !principal.session.is_sudo
            && matches!(service_id.as_str(), "aiworkspace" | "msg-center") =>
        {
            Ok(target)
        }
        _ => Err(denied(
            "resource.share.issue requires the resource service's own identity",
        )),
    }
}

async fn installation(
    kv: &dyn AgentKv,
    id: &AppInstanceId,
) -> Result<(AppServiceSpec, u64), RPCErrors> {
    let key = user_app_spec_key(id.owner_user_id(), id.app_id());
    let (raw, version) = kv
        .get(&key)
        .await?
        .ok_or_else(|| denied("installation unavailable"))?;
    let spec: AppServiceSpec =
        serde_json::from_str(&raw).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
    if spec.app_instance_id != *id
        || spec.owner_user_id != id.owner_user_id()
        || !spec.is_installed()
    {
        return Err(denied("installation unavailable"));
    }
    Ok((spec, version))
}

async fn active_installation(
    kv: &dyn AgentKv,
    id: &AppInstanceId,
) -> Result<AppServiceSpec, RPCErrors> {
    let (spec, _) = installation(kv, id).await?;
    if !spec.is_runtime_enabled() {
        return Err(denied("installation unavailable"));
    }
    Ok(spec)
}

async fn check_policy(
    kv: &dyn AgentKv,
    target: &AuthTarget,
    audience: &ShareAudience,
) -> Result<(), RPCErrors> {
    if let AuthTarget::App { app_instance_id } = target {
        let spec = active_installation(kv, app_instance_id).await?;
        let now = buckyos_get_unix_timestamp();
        if ShareLevel::from_permissions(&spec.permission, now)
            .min(ShareLevel::from_permissions(&spec.app_doc.permissions, now))
            < audience.level()
        {
            return Err(denied(
                "installation does not allow this invitation audience",
            ));
        }
    }
    Ok(())
}

async fn check_audience(kv: &dyn AgentKv, audience: &ShareAudience) -> Result<(), RPCErrors> {
    let ShareAudience::Directed { did } = audience else {
        return Ok(());
    };
    let invalid = || denied("本区未启用该登录来源，请改邀对方的 BuckyOS 账号 DID");
    if did.len() > 256
        || did.contains(['*', '/', '#', '?', '%'])
        || did.chars().any(char::is_whitespace)
    {
        return Err(invalid());
    }
    let parsed = DID::from_str(did).map_err(|_| invalid())?;
    if parsed.to_string() != *did {
        return Err(invalid());
    }
    let settings = crate::sso_settings::load_sso_settings(kv).await?;
    if let Some(id) = crate::oauth::external_source_for_did(&settings, did) {
        let source = settings
            .oauth_sources
            .iter()
            .find(|s| s.id == id && s.enabled)
            .ok_or_else(invalid)?;
        let label = parsed
            .id
            .strip_suffix(&format!(".{}", source.webhost))
            .ok_or_else(invalid)?;
        return buckyos_api::valid_oauth_label(id == "google", label)
            .then_some(())
            .ok_or_else(invalid);
    }
    let runtime = get_buckyos_api_runtime()?;
    let zone = ControlPanelServer::sso_zone_host(runtime)?;
    if let Some(user) = parsed
        .id
        .strip_suffix(&format!(".{zone}"))
        .filter(|u| crate::agent_mgr::is_dns_label(u))
    {
        let key = format!("users/{user}/doc");
        if let Some((raw, _)) = kv.get(&key).await? {
            if buckyos_api::user_did_from_doc_value(&key, &raw)? == parsed {
                return Ok(());
            }
        }
    }
    if !settings.did_login_enabled {
        return Err(invalid());
    }
    NameServiceDirectory
        .canonical_host(&parsed)
        .await
        .map(|_| ())
        .map_err(|_| invalid())
}

fn target_url(target: &AuthTarget, resource: &str, path: &str) -> Result<String, RPCErrors> {
    validate_return_path(path)?;
    let runtime = get_buckyos_api_runtime()?;
    let zone = ControlPanelServer::sso_zone_host(runtime)?;
    let label = match target {
        AuthTarget::App { app_instance_id } => crate::app_login::app_host_label(app_instance_id)?,
        AuthTarget::System { service_id } if service_id.as_str() == "aiworkspace" => {
            let workspace = resource
                .split_once('~')
                .and_then(|(ws, _)| ws.strip_prefix("ws_"))
                .filter(|id| {
                    !id.is_empty()
                        && id
                            .bytes()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
                })
                .ok_or_else(|| denied("invalid workspace reference"))?;
            format!("aiws-{workspace}")
        }
        AuthTarget::System { service_id } if service_id.as_str() == "msg-center" => {
            if !path.starts_with("/messagehub/guest?") {
                return Err(denied("invalid Message Hub visitor path"));
            }
            "messagehub".to_string()
        }
        _ => return Err(denied("service has no registered share delivery host")),
    };
    let url = format!("https://{label}.{zone}{path}");
    let resolved = ControlPanelServer::resolve_sso_auth_target(
        &url,
        &zone,
        !runtime.force_https,
        runtime.node_gateway_port,
    )?;
    if &resolved.auth_target != target {
        return Err(denied("share route changed"));
    }
    Ok(url)
}

async fn callback(
    share: &ResourceShare,
    secret: &str,
    operation: &str,
    identity: Option<(&str, Option<AuthMethod>)>,
) -> Result<Value, RPCErrors> {
    let mut body = json!({"operation":operation,"resource_id":share.resource_id,"invite_id":share.invite_id,"callback_secret":secret});
    if let Some((did, method)) = identity {
        body["did"] = json!(did);
        if let Some(method) = method {
            body["auth_method"] = json!(method);
        }
    }
    let result: Value = match &share.target {
        AuthTarget::System { service_id } => {
            get_buckyos_api_runtime()?
                .get_zone_service_krpc_client(service_id.as_str())
                .await?
                .call("share.callback", body)
                .await?
        }
        AuthTarget::App { .. } => {
            let mut url = url::Url::parse(&share.target_url)
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
            url.set_path("/sdk/appservice/resource-share/callback");
            url.set_query(None);
            url.set_fragment(None);
            let http = reqwest::Client::builder()
                .timeout(Duration::from_secs(10))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
            http.post(url)
                .json(&body)
                .send()
                .await
                .and_then(|r| r.error_for_status())
                .map_err(|_| RPCErrors::ReasonError("share resource service unavailable".into()))?
                .json()
                .await
                .map_err(|_| RPCErrors::ReasonError("invalid share response".into()))?
        }
    };
    if operation == "resume" && result["ok"] == false {
        return Ok(result);
    }
    if result["ok"] != true {
        return Err(denied("resource no longer grants this invitation"));
    }
    Ok(result)
}

fn facts(value: &Value) -> Result<ResourceFacts, RPCErrors> {
    let data: ResourceFacts = serde_json::from_value(value.clone())
        .map_err(|_| denied("resource did not authorize sharing"))?;
    if data.sharer.trim().is_empty()
        || data.title.trim().is_empty()
        || data.operations.is_empty()
        || data
            .expires_at
            .is_some_and(|t| t <= buckyos_get_unix_timestamp())
    {
        return Err(denied("resource unavailable"));
    }
    Ok(data)
}

pub(crate) async fn load_share(kv: &dyn AgentKv, id: &str) -> Result<ResourceShare, RPCErrors> {
    if !valid_resource_share_id(id) {
        return Err(denied("invalid invitation"));
    }
    read(kv, &resource_share_key(id))
        .await?
        .ok_or_else(|| denied("invitation unavailable"))
}

async fn route_credential(kv: &dyn AgentKv, share: &ResourceShare) -> Result<String, RPCErrors> {
    if let AuthTarget::App { app_instance_id } = &share.target {
        active_installation(kv, app_instance_id).await?;
    }
    let url = url::Url::parse(&share.target_url).map_err(|_| denied("invalid target"))?;
    if target_url(
        &share.target,
        &share.resource_id,
        &url[url::Position::BeforePath..],
    )? != share.target_url
    {
        return Err(denied("share route changed"));
    }
    let secret = read::<String>(kv, &format!("{CALLBACK_PREFIX}/{}", share.invite_id))
        .await?
        .ok_or_else(|| denied("invitation unavailable"))?;
    Ok(secret)
}

async fn check_current(kv: &dyn AgentKv, share: &ResourceShare) -> Result<String, RPCErrors> {
    check_policy(kv, &share.target, &share.audience).await?;
    if share
        .expires_at
        .is_some_and(|t| t <= buckyos_get_unix_timestamp())
    {
        return Err(denied("invitation expired"));
    }
    check_audience(kv, &share.audience).await?;
    let secret = route_credential(kv, share).await?;
    let current = facts(&callback(share, &secret, "inspect", None).await?)?;
    if current.sharer != share.sharer
        || current.operations != share.operations
        || current.methods != share.methods
        || current.expires_at != share.expires_at
    {
        return Err(denied("resource sharing authorization changed"));
    }
    Ok(secret)
}

pub(crate) async fn checked_share(id: &str) -> Result<ResourceShare, RPCErrors> {
    let kv = ZoneKv::connect().await?;
    let share = load_share(&kv, id).await?;
    check_current(&kv, &share).await?;
    Ok(share)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Describe {
    invite_id: String,
}

impl ControlPanelServer {
    pub(crate) async fn handle_share_verify_grant(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if principal.session.is_sudo
            || principal.authenticated_app_id != "system:verify-hub"
            || !matches!(
                principal.principal_kind,
                TokenPrincipalKind::System | TokenPrincipalKind::Device
            )
        {
            return Err(denied("verify-hub service session required"));
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Probe {
            invite_id: String,
            did: String,
        }
        let query: Probe = Self::typed_params(&req)?;
        let kv = ZoneKv::connect().await?;
        let ok = async {
            let share = load_share(&kv, &query.invite_id).await?;
            let grant = read::<ResourceShareAcceptance>(
                &kv,
                &resource_share_acceptance_key(&query.invite_id, &query.did),
            )
            .await?
            .filter(|g| {
                g.invite_id == query.invite_id && g.did == query.did && g.target == share.target
            });
            if grant.is_none() {
                return Ok(false);
            }
            let secret = route_credential(&kv, &share).await?;
            let outcome = callback(&share, &secret, "resume", Some((&query.did, None))).await?;
            Ok(outcome["ok"] == true)
        }
        .await;
        let ok = match ok {
            Ok(ok) => ok,
            Err(RPCErrors::NoPermission(_)) => false,
            Err(error) => return Err(error),
        };
        Ok(RPCResponse::new(
            RPCResult::Success(json!({"ok":ok})),
            req.seq,
        ))
    }
    pub(crate) async fn handle_share_describe(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let p: Describe = Self::typed_params(&req)?;
        let kv = ZoneKv::connect().await?;
        let share = load_share(&kv, &p.invite_id).await?;
        let reason = check_current(&kv, &share)
            .await
            .err()
            .map(|e| e.to_string());
        let mut view = json!(share);
        if let Some(reason) = reason {
            view["unavailable_reason"] = json!(reason);
        }
        Ok(RPCResponse::new(RPCResult::Success(view), req.seq))
    }

    pub(crate) async fn handle_share_issue(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let target = issue_target(Self::require_rpc_principal(principal)?)?;
        let p: Issue = Self::typed_params(&req)?;
        if p.request_id.is_empty()
            || p.request_id.len() > 256
            || p.resource_id.is_empty()
            || p.resource_id.len() > 512
            || !(32..=256).contains(&p.callback_secret.len())
        {
            return Err(denied("invalid share request"));
        }
        let kv = ZoneKv::connect().await?;
        let audience = match (&p.audience, &p.recipient_email) {
            (IssueAudience::Directed { did: None }, Some(email)) => ShareAudience::Directed {
                did: crate::oauth::resolve_email(&kv, &email.source_id, &email.email).await?,
            },
            (IssueAudience::Directed { did: Some(did) }, None) => {
                ShareAudience::Directed { did: did.clone() }
            }
            (IssueAudience::Open, None) => ShareAudience::Open,
            _ => {
                return Err(denied(
                    "select one recipient or explicitly open the invitation",
                ))
            }
        };
        check_policy(&kv, &target, &audience).await?;
        check_audience(&kv, &audience).await?;
        let runtime = get_buckyos_api_runtime()?;
        let host = Self::sso_zone_host(runtime)?;
        let key = format!(
            "{REQUESTS_PREFIX}/{}",
            invite_secret_digest(&json!([target, p.request_id]).to_string())
        );
        let digest = invite_secret_digest(
            &json!([
                target,
                p.resource_id,
                p.return_path,
                p.audience,
                p.recipient_email,
                p.callback_secret
            ])
            .to_string(),
        );
        let reply = |id: &str| {
            RPCResponse::new(
                RPCResult::Success(
                    json!({"invite_id":id,"url":format!("https://{host}/sso/share/{id}")}),
                ),
                req.seq,
            )
        };
        let replay = |index: RequestIndex| {
            if index.digest == digest {
                Ok(reply(&index.invite_id))
            } else {
                Err(denied("request_id already used for another invitation"))
            }
        };
        if let Some(index) = read(&kv, &key).await? {
            return replay(index);
        }
        let id = format!("inv-{}", random_token(24));
        let mut share = ResourceShare {
            invite_id: id.clone(),
            issuer: runtime.zone_id.to_string(),
            target_url: target_url(&target, &p.resource_id, &p.return_path)?,
            resource_id: p.resource_id,
            target,
            audience,
            title: String::new(),
            methods: Default::default(),
            operations: vec![],
            sharer: String::new(),
            expires_at: None,
        };
        let checked = facts(&callback(&share, &p.callback_secret, "inspect", None).await?)?;
        share.methods = checked.methods;
        share.title = checked.title;
        share.operations = checked.operations;
        share.sharer = checked.sharer;
        share.expires_at = checked.expires_at;
        let mut actions = HashMap::from([
            (
                resource_share_key(&id),
                KVAction::Create(json!(share).to_string()),
            ),
            (
                format!("{CALLBACK_PREFIX}/{id}"),
                KVAction::Create(json!(p.callback_secret).to_string()),
            ),
            (
                key.clone(),
                KVAction::Create(
                    json!(RequestIndex {
                        invite_id: id.clone(),
                        digest: digest.clone()
                    })
                    .to_string(),
                ),
            ),
        ]);
        if let Some(email) = &p.recipient_email {
            actions.insert(
                format!("services/control_panel/resource_share_recipients/{id}"),
                KVAction::Create(json!(email).to_string()),
            );
        }
        if let Err(error) = kv.exec_tx(actions, None).await {
            if let Some(index) = read(&kv, &key).await? {
                return replay(index);
            }
            return Err(error);
        }
        Ok(reply(&id))
    }
}

pub(crate) async fn native_bound_account_did(
    kv: &dyn AgentKv,
    recipient: &str,
) -> Result<Option<String>, RPCErrors> {
    let Some(username) =
        read::<String>(kv, &buckyos_api::native_login_binding_key(recipient)).await?
    else {
        return Ok(None);
    };
    let key = format!("users/{username}/doc");
    let Some((raw, _)) = kv.get(&key).await? else {
        return Err(denied("bound account unavailable"));
    };
    let account = read::<buckyos_api::UserSettings>(kv, &format!("users/{username}/settings"))
        .await?
        .ok_or_else(|| denied("bound account unavailable"))?;
    if account.user_id != username || !matches!(account.state, buckyos_api::UserState::Active) {
        return Err(denied("bound account unavailable"));
    }
    Ok(Some(
        buckyos_api::user_did_from_doc_value(&key, &raw)?.to_string(),
    ))
}

pub(crate) async fn accept_resource_share(
    share: &ResourceShare,
    did: &str,
    method: AuthMethod,
    oauth_identity: Option<&crate::oauth::VerifiedOAuth>,
) -> Result<(), RPCErrors> {
    let kv = ZoneKv::connect().await?;
    let share = load_share(&kv, &share.invite_id).await?;
    let secret = route_credential(&kv, &share).await?;
    accept_registered_share(&kv, &share, &secret, did, method, oauth_identity).await
}

async fn accept_registered_share(
    kv: &dyn AgentKv,
    share: &ResourceShare,
    secret: &str,
    did: &str,
    method: AuthMethod,
    oauth_identity: Option<&crate::oauth::VerifiedOAuth>,
) -> Result<(), RPCErrors> {
    let key = resource_share_acceptance_key(&share.invite_id, did);
    let identity = Some((did, None));
    let (external, mut actions) = match oauth_identity {
        Some(identity) => {
            let (subject, actions) = crate::oauth::subject_actions(kv, identity, None).await?;
            (Some(subject.did), actions)
        }
        None => (None, HashMap::new()),
    };
    if let Some(accepted) = read::<ResourceShareAcceptance>(kv, &key).await? {
        if accepted.did != did || accepted.target != share.target || !actions.is_empty() {
            return Err(denied("inconsistent resource acceptance"));
        }
        if callback(share, secret, "resume", identity).await?["ok"] != true {
            return Err(denied("resource authorization was removed"));
        }
        return Ok(());
    }
    let pending_key = format!(
        "services/control_panel/resource_share_pending/{}/{}",
        share.invite_id,
        invite_secret_digest(did)
    );
    let pending = read::<ResourceShareAcceptance>(kv, &pending_key).await?;
    if let Some(grant) = pending.as_ref() {
        if grant.invite_id != share.invite_id
            || grant.did != did
            || grant.target != share.target
            || !actions.is_empty()
        {
            return Err(denied("inconsistent pending acceptance"));
        }
        // The resource may already have committed when a CP transaction or response was interrupted.
        if callback(share, secret, "resume", identity).await?["ok"] == true {
            return finalize_acceptance(kv, key, pending_key, grant).await;
        }
    }
    check_current(kv, share).await?;
    let settings = crate::sso_settings::load_sso_settings(kv).await?;
    let mut admitted_method = method;
    if let ShareAudience::Directed { did: recipient } = &share.audience {
        if recipient != did && external.as_ref() != Some(recipient) {
            if crate::oauth::bound_account_did(kv, recipient)
                .await?
                .as_deref()
                == Some(did)
            {
                admitted_method = AuthMethod::OAuth;
            } else if native_bound_account_did(kv, recipient).await?.as_deref() == Some(did) {
                admitted_method = AuthMethod::DidFederation;
            } else {
                return Err(denied("invitation is for a different identity"));
            }
        }
    }
    if !share
        .methods
        .effective(&settings)
        .0
        .allows_session(Some(admitted_method))
    {
        return Err(denied("invitation does not allow this sign-in method"));
    }
    let identity = Some((did, Some(admitted_method)));
    let accepted = ResourceShareAcceptance {
        invite_id: share.invite_id.clone(),
        did: did.into(),
        target: share.target.clone(),
        accepted_at: buckyos_get_unix_timestamp(),
    };
    callback(share, secret, "prepare", identity).await?;
    if pending.is_none() {
        actions.insert(
            pending_key.clone(),
            KVAction::Create(json!(accepted).to_string()),
        );
        if let Err(error) = kv.exec_tx(actions, None).await {
            if read::<ResourceShareAcceptance>(kv, &pending_key)
                .await?
                .is_none()
                && read::<ResourceShareAcceptance>(kv, &key).await?.is_none()
            {
                return Err(error);
            }
        }
    }
    if let Some(identity) = oauth_identity {
        let (subject, missing) = crate::oauth::subject_actions(kv, identity, None).await?;
        if !missing.is_empty() || external.as_ref() != Some(&subject.did) {
            return Err(denied("OAuth subject changed during acceptance"));
        }
    }
    callback(share, secret, "commit", identity).await?;
    finalize_acceptance(kv, key, pending_key, &accepted).await
}

async fn finalize_acceptance(
    kv: &dyn AgentKv,
    key: String,
    pending_key: String,
    accepted: &ResourceShareAcceptance,
) -> Result<(), RPCErrors> {
    let finalize = HashMap::from([
        (key.clone(), KVAction::Create(json!(accepted).to_string())),
        (pending_key, KVAction::Remove),
    ]);
    if let Err(error) = kv.exec_tx(finalize, None).await {
        if read::<ResourceShareAcceptance>(kv, &key).await?.is_none() {
            return Err(error);
        }
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyParams {
    app_instance_id: AppInstanceId,
    level: Option<ShareLevel>,
}

impl ControlPanelServer {
    pub(crate) async fn handle_share_policy(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let params: PolicyParams = Self::typed_params(&req)?;
        if !principal.is_user_session
            || !principal.is_control_panel_session
            || principal.is_restricted()
            || (principal.username != params.app_instance_id.owner_user_id()
                && !principal.user_type.is_privileged())
        {
            return Err(denied("installation owner or administrator required"));
        }
        let kv = ZoneKv::connect().await?;
        let (mut spec, version) = installation(&kv, &params.app_instance_id).await?;
        let now = buckyos_get_unix_timestamp();
        let maximum = ShareLevel::from_permissions(&spec.app_doc.permissions, now);
        if req.method == "apps.share.set" {
            let level = params.level.ok_or_else(|| denied("level required"))?;
            if level > maximum {
                return Err(denied("invitation level exceeds App declaration"));
            }
            spec.permission.retain(|p| {
                !matches!(
                    p.scope_path.as_str(),
                    buckyos_api::SHARE_OPEN_PERMISSION | buckyos_api::SHARE_DIRECTED_PERMISSION
                )
            });
            for permission in &spec.app_doc.permissions {
                if (permission.scope_path == buckyos_api::SHARE_OPEN_PERMISSION
                    && level == ShareLevel::Open)
                    || (permission.scope_path == buckyos_api::SHARE_DIRECTED_PERMISSION
                        && level >= ShareLevel::Directed)
                {
                    spec.permission.push(permission.clone());
                }
            }
            let key = user_app_spec_key(
                params.app_instance_id.owner_user_id(),
                params.app_instance_id.app_id(),
            );
            kv.exec_tx(
                HashMap::from([(key.clone(), KVAction::Update(json!(spec).to_string()))]),
                Some((key, version)),
            )
            .await?;
        }
        Ok(RPCResponse::new(
            RPCResult::Success(
                json!({"level":ShareLevel::from_permissions(&spec.permission,now).min(maximum),"maximum":maximum}),
            ),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrincipalSession;
    use async_trait::async_trait;

    #[tokio::test]
    async fn lost_commit_response_resumes_once_across_two_tabs_and_rechecks_the_grant() {
        use crate::exclusive_invite::tests::MemKv;
        use bytes::Bytes;
        use http_body_util::{BodyExt, Full};
        use std::sync::Arc;
        use tokio::sync::Barrier;

        const DID: &str = "did:web:visitor.example";
        const SECRET: &str = "test-registration-secret";
        let endpoint = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let share = ResourceShare {
            invite_id: format!("inv-{}", "a".repeat(32)),
            issuer: "did:web:zone.example".into(),
            target: AuthTarget::from_canonical_key("app:notes.example@alice").unwrap(),
            target_url: format!("http://{}/docs/42", endpoint.local_addr().unwrap()),
            resource_id: "document-42-invite".into(),
            sharer: "alice".into(),
            title: "Document 42".into(),
            operations: vec!["read".into()],
            methods: buckyos_api::EntryMethods {
                did_login: true,
                ..Default::default()
            },
            audience: ShareAudience::Open,
            expires_at: Some(1), // New admissions have closed when the tabs retry.
        };
        let kv = MemKv::default();
        let acceptance_key = resource_share_acceptance_key(&share.invite_id, DID);
        let pending_key = format!(
            "services/control_panel/resource_share_pending/{}/{}",
            share.invite_id,
            invite_secret_digest(DID)
        );
        let grant = ResourceShareAcceptance {
            invite_id: share.invite_id.clone(),
            did: DID.into(),
            target: share.target.clone(),
            accepted_at: 1,
        };
        kv.put(&pending_key, &grant);

        // The App committed its grant. Its first HTTP response is lost before CP can finalize.
        // Hold both retries at the resource until both have read the same pending record.
        let server = tokio::spawn(async move {
            let barrier = Arc::new(Barrier::new(2));
            let mut handlers = tokio::task::JoinSet::new();
            for i in 0..5 {
                let (stream, _) = endpoint.accept().await.unwrap();
                let barrier = barrier.clone();
                handlers.spawn(async move {
                    let handle =
                        hyper::service::service_fn(move |req: http::Request<hyper::body::Incoming>| {
                            let barrier = barrier.clone();
                            async move {
                                assert_eq!(req.uri().path(), "/sdk/appservice/resource-share/callback");
                                let bytes = req.into_body().collect().await.unwrap().to_bytes();
                                let body: Value = serde_json::from_slice(&bytes).unwrap();
                                assert_eq!(body["resource_id"], "document-42-invite");
                                assert_eq!(body["callback_secret"], SECRET);
                                assert_eq!(body["did"], DID);
                                assert_eq!(body["operation"], if i == 0 { "commit" } else { "resume" });
                                if i == 1 || i == 2 {
                                    barrier.wait().await;
                                }
                                let status = if i == 0 { 503 } else { 200 };
                                let response = http::Response::builder()
                                    .status(status)
                                    .body(Full::new(Bytes::from(json!({"ok": i != 4}).to_string())))
                                    .unwrap();
                                Ok::<_, std::convert::Infallible>(response)
                            }
                        });
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(hyper_util::rt::TokioIo::new(stream), handle)
                        .await
                        .unwrap();
                });
            }
            while let Some(result) = handlers.join_next().await {
                result.unwrap();
            }
        });
        let method = AuthMethod::DidFederation;
        assert!(
            callback(&share, SECRET, "commit", Some((DID, Some(method))))
                .await
                .is_err()
        );
        assert!(kv.has(&pending_key));
        assert!(!kv.has(&acceptance_key));
        let resume = || accept_registered_share(&kv, &share, SECRET, DID, method, None);
        let (first, second) = tokio::join!(resume(), resume());
        first.unwrap();
        second.unwrap();
        assert_eq!(kv.read::<ResourceShareAcceptance>(&acceptance_key), grant);
        assert!(!kv.has(&pending_key));
        let accepted = kv.get(&acceptance_key).await.unwrap();
        resume().await.unwrap();
        assert_eq!(kv.get(&acceptance_key).await.unwrap(), accepted);
        assert!(matches!(resume().await, Err(RPCErrors::NoPermission(_))));
        server.await.unwrap();
    }

    #[test]
    fn system_and_device_sessions_must_name_the_registered_resource_service() {
        let mut p = RpcAuthPrincipal {
            username: "ood1".into(),
            owner_user_id: "ood1".into(),
            authenticated_app_id: "system:aiworkspace".into(),
            user_type: buckyos_api::UserType::Limited,
            owner_did: String::new(),
            is_user_session: false,
            is_control_panel_session: false,
            principal_kind: TokenPrincipalKind::Device,
            agent_permission_mode: None,
            session: PrincipalSession::default(),
        };
        assert!(issue_target(&p).is_ok());
        p.authenticated_app_id = "system:verify-hub".into();
        assert!(issue_target(&p).is_err());
        p.authenticated_app_id = "system:msg-center".into();
        p.principal_kind = TokenPrincipalKind::System;
        assert!(issue_target(&p).is_ok());
        p.principal_kind = TokenPrincipalKind::User;
        assert!(issue_target(&p).is_err());
    }

    struct Directory(HashMap<String, String>);
    #[async_trait]
    impl AgentKv for Directory {
        async fn get(&self, path: &str) -> Result<Option<(String, u64)>, RPCErrors> {
            Ok(self.0.get(path).map(|v| (v.clone(), 1)))
        }
        async fn list(&self, _: &str) -> Result<Vec<String>, RPCErrors> {
            unreachable!()
        }
        async fn exec_tx(
            &self,
            _: HashMap<String, KVAction>,
            _: Option<(String, u64)>,
        ) -> Result<(), RPCErrors> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn native_mapping_resolves_only_a_bound_active_local_account() {
        let native = "did:bns:traveller";
        let local = "did:web:reader.example.com";
        let mut settings = json!({"user_id":"reader","password":"","type":"user","state":"active","res_pool_id":"default"});
        let mut directory = Directory(HashMap::from([
            (
                buckyos_api::native_login_binding_key(native),
                json!("reader").to_string(),
            ),
            ("users/reader/doc".into(), json!({"id":local}).to_string()),
            ("users/reader/settings".into(), settings.to_string()),
        ]));
        assert_eq!(
            native_bound_account_did(&directory, native)
                .await
                .unwrap()
                .as_deref(),
            Some(local)
        );
        assert_eq!(
            native_bound_account_did(&directory, "did:bns:other")
                .await
                .unwrap(),
            None
        );
        settings["state"] = json!("deleted");
        directory
            .0
            .insert("users/reader/settings".into(), settings.to_string());
        assert!(native_bound_account_did(&directory, native).await.is_err());
    }
}
