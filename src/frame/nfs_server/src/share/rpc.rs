//! NFSP extension methods `share.*` (design §3.6). Same envelope, session and
//! authentication as the file methods; write methods are in `WRITE_METHODS`
//! (seq replay) and creation also takes an explicit `idempotency_key`.

use super::db::*;
use super::snapshot::{self, SourcePlan};
use super::{pin_owner, settle_share, ShareMgr};
use crate::acl::{Actor, Level, PUBLIC};
use crate::error::*;
use crate::fsutil::unix_now;
use crate::state::SharedState;
use crate::types::Locator;
use serde::Deserialize;
use serde_json::{json, Value};

pub const WRITE_METHODS: &[&str] = &[
    "share.create",
    "share.update",
    "share.stop",
    "share.hide",
    "share.app.handoff",
    "share.app.report",
];

const EXPIRY_PRESETS: &[Option<i64>] = &[Some(86400), Some(7 * 86400), Some(30 * 86400), None];
const DEFAULT_EXPIRY: i64 = 7 * 86400;
/// "Expiring soon" in statistics; shorter than the default expiry, or every new link would count.
const EXPIRING_WITHIN: i64 = 3 * 86400;
/// HomeStation attachments: one chunk (design §2.5).
pub const HOMESTATION_MAX_ITEM: u64 = 32 * 1024 * 1024;
/// msg-center's object route serves single-chunk files only (one 32 MiB chunk).
pub const MESSAGEHUB_MAX_ITEM: u64 = 32 * 1024 * 1024;
const TITLE_MAX: usize = 200;

/// Who a `share.*` call acts for.
struct Principal {
    /// The user owning the shares this call creates or sees.
    owner: String,
    /// Who actually called (the owner, an admin, or "system").
    actor_user: String,
    app: Option<String>,
    source: &'static str,
    admin: bool,
    /// File access used for snapshot sources.
    file_actor: Actor,
}

async fn principal(state: &SharedState, actor: &Actor, args: &Value) -> NfsResult<Principal> {
    let hint = match args["source_hint"].as_str() {
        Some("cli") => "cli",
        Some("agent") => "agent",
        _ => "gui",
    };
    match actor {
        Actor::Anonymous => Err(NfsError::new(ErrorCode::PermissionDenied, "sign in to manage shares")
            .with("reason", json!("unauthenticated"))),
        Actor::Admin(None) => {
            // Zone services and devices (the CLI on a node) act for the zone owner unless they name a user.
            let owner = match args["on_behalf_of"].as_str().filter(|u| !u.is_empty()) {
                Some(u) => u.to_string(),
                None => match crate::auth::zone_owner_id(state).await {
                    Some(owner) => owner,
                    // Standalone without auth: everyone is one local user.
                    None if matches!(state.config.auth, crate::config::AuthMode::Disabled) => "local".to_string(),
                    None => return Err(invalid("the zone owner is unknown; pass on_behalf_of")),
                },
            };
            let file_actor = owner_actor(state, &owner).await;
            let source = if hint == "gui" { "system" } else { hint };
            Ok(Principal { owner, actor_user: "system".into(), app: None, source, admin: true, file_actor })
        }
        Actor::Admin(Some(u)) => Ok(Principal {
            owner: u.clone(),
            actor_user: u.clone(),
            app: None,
            source: hint,
            admin: true,
            file_actor: actor.clone(),
        }),
        Actor::User(u) => Ok(Principal {
            owner: u.clone(),
            actor_user: u.clone(),
            app: None,
            source: hint,
            admin: false,
            file_actor: actor.clone(),
        }),
        Actor::Delegated { user, app, .. } => Ok(Principal {
            owner: user.clone(),
            actor_user: user.clone(),
            app: Some(app.clone()),
            source: if hint == "gui" { "agent" } else { hint },
            admin: false,
            file_actor: actor.file_actor(),
        }),
    }
}

/// The file actor of a share owner (zone owner = admin), for background work.
pub async fn owner_actor(state: &SharedState, user: &str) -> Actor {
    if crate::auth::is_zone_owner(state, user).await {
        Actor::Admin(Some(user.to_string()))
    } else {
        Actor::User(user.to_string())
    }
}

pub async fn dispatch(
    state: &SharedState,
    actor: &Actor,
    method: &str,
    args: &Value,
    bearer: Option<&str>,
) -> NfsResult<Value> {
    require_agent_share_permission(state, actor, method).await?;
    let p = principal(state, actor, args).await?;
    match method {
        "share.capabilities" => Ok(capabilities(state)),
        "share.plan" => plan(state, &p, args).await,
        "share.create" => create(state, &p, args, bearer, Intent::Link).await,
        "share.app.handoff" => create(state, &p, args, bearer, Intent::Handoff).await,
        "share.operation" => operation(state, &p, args),
        "share.get" => get(state, &p, args),
        "share.list" => list(state, &p, args),
        "share.update" => update(state, &p, args),
        "share.stop" => stop(state, &p, args).await,
        "share.hide" => hide(state, &p, args),
        "share.app.report" => report(state, &p, args).await,
        "share.public.list" => public_list(state, args).await,
        "share.stats" => stats(state, &p, args),
        other => Err(NfsError::new(ErrorCode::Unsupported, format!("unknown method '{}'", other))),
    }
}

async fn require_agent_share_permission(
    state: &SharedState,
    actor: &Actor,
    method: &str,
) -> NfsResult<()> {
    if !WRITE_METHODS.contains(&method)
        || !matches!(state.config.auth, crate::config::AuthMode::Runtime)
    {
        return Ok(());
    }
    let Actor::Delegated { user, app, .. } = actor else {
        return Ok(());
    };
    let runtime = buckyos_api::get_buckyos_api_runtime()
        .map_err(|error| internal(error.to_string()))?;
    let client = runtime
        .get_system_config_client()
        .await
        .map_err(|error| internal(error.to_string()))?;
    let mode = if app.starts_with("app:") || app.starts_with("system:") {
        let target = buckyos_api::AuthTarget::from_canonical_key(app)
            .map_err(|error| NfsError::new(ErrorCode::PermissionDenied, error))?;
        let buckyos_api::AuthTarget::App { app_instance_id } = target else {
            return Ok(());
        };
        buckyos_api::load_agent_permission_mode(client.as_ref(), &app_instance_id)
            .await
            .map_err(|error| internal(error.to_string()))?
    } else {
        let agent_id = buckyos_api::AgentId::parse(app)
            .map_err(|error| NfsError::new(ErrorCode::PermissionDenied, error))?;
        let key = buckyos_api::agent_spec_key(user, &agent_id);
        let value = client.get(&key).await.map_err(|error| internal(error.to_string()))?;
        let spec: buckyos_api::AgentSpec =
            serde_json::from_str(&value.value).map_err(|error| internal(error.to_string()))?;
        spec.validate().map_err(internal)?;
        if spec.agent_id != agent_id
            || spec.binding.target_app_instance_id.owner_user_id() != user
        {
            return Err(NfsError::new(
                ErrorCode::PermissionDenied,
                "Agent identity or owner does not match its permission record",
            ));
        }
        Some(spec.permission_mode)
    };
    check_agent_share_permission(mode, method)
}

fn check_agent_share_permission(
    mode: Option<buckyos_api::AgentPermissionMode>,
    method: &str,
) -> NfsResult<()> {
    if mode == Some(buckyos_api::AgentPermissionMode::ReadOnly)
        && WRITE_METHODS.contains(&method)
    {
        return Err(
            NfsError::new(
                ErrorCode::PermissionDenied,
                format!("read-only Agent cannot call {method}"),
            )
            .with("reason", json!("agent_read_only")),
        );
    }
    Ok(())
}

#[cfg(test)]
mod agent_permission_tests {
    use super::*;
    use buckyos_api::AgentPermissionMode::{ReadOnly, ReadWrite};

    #[test]
    fn agent_permission_blocks_share_writes_but_preserves_queries_and_ordinary_apps() {
        for method in WRITE_METHODS {
            let error = check_agent_share_permission(Some(ReadOnly), method).unwrap_err();
            assert_eq!(error.code, ErrorCode::PermissionDenied);
            assert_eq!(error.details["reason"], "agent_read_only");
            assert!(check_agent_share_permission(Some(ReadWrite), method).is_ok());
            assert!(check_agent_share_permission(None, method).is_ok());
        }
        for method in ["share.get", "share.list", "share.plan", "share.operation", "share.stats"] {
            assert!(check_agent_share_permission(Some(ReadOnly), method).is_ok());
        }
    }
}

// ---------- helpers ----------

fn new_id(prefix: &str) -> String {
    format!("{}_{}", prefix, &uuid::Uuid::new_v4().simple().to_string()[..16])
}

fn str_arg<'a>(args: &'a Value, key: &str) -> NfsResult<&'a str> {
    args[key].as_str().filter(|s| !s.is_empty()).ok_or_else(|| invalid(format!("{} required", key)))
}

fn source_arg(args: &Value) -> NfsResult<Locator> {
    let v = args.get("source").filter(|v| !v.is_null()).ok_or_else(|| invalid("source required"))?;
    serde_json::from_value(v.clone()).map_err(|e| invalid(format!("bad source locator: {}", e)))
}

fn clean_title(raw: Option<&str>, fallback: &str) -> String {
    let t = raw.map(str::trim).filter(|t| !t.is_empty()).unwrap_or(fallback);
    t.chars().take(TITLE_MAX).collect()
}

pub fn link_url(state: &SharedState, cap_id: Option<&str>) -> Option<String> {
    let base = state.config.share.base_url.as_deref()?;
    Some(format!("{}/s/{}", base.trim_end_matches('/'), state.share.token_for(cap_id?)))
}

fn effective_status(share: &ShareRow, cap: Option<&CapRow>) -> String {
    let expired = cap.and_then(|c| c.expires_at).is_some_and(|e| e <= unix_now());
    if share.status == "active" && expired {
        "expired".into()
    } else {
        share.status.clone()
    }
}

/// A share the caller may manage: its owner, or an admin. Others see "not found".
fn load_share(state: &SharedState, p: &Principal, share_id: &str) -> NfsResult<ShareRow> {
    match state.db.share_get(share_id)? {
        Some(s) if s.owner_user_id == p.owner || p.admin => Ok(s),
        _ => Err(not_found(format!("share '{}' not found", share_id))),
    }
}

fn audit_source(p: &Principal, share: &ShareRow) -> &'static str {
    if share.owner_user_id != p.owner && p.admin {
        "admin"
    } else {
        p.source
    }
}

fn kind_of(mime: Option<&str>, dir: bool) -> &'static str {
    if dir {
        return "folder";
    }
    match mime.and_then(|m| m.split('/').next()) {
        Some("image") => "image",
        Some("video") => "video",
        Some("audio") => "audio",
        _ => "file",
    }
}

pub fn share_view(state: &SharedState, viewer: &str, s: &ShareRow) -> NfsResult<Value> {
    let version = state.db.share_version_get(&s.obj_id)?;
    let policy = state.db.share_policy(&s.share_id)?;
    let cap = match &s.cap_id {
        Some(c) => state.db.share_cap(c)?,
        None => None,
    };
    let delivery = state.db.share_delivery(&s.share_id)?;
    let (totals, last) = state.db.share_stats_total(&s.share_id)?;
    let retained = state.db.share_retentions(&s.share_id)?.iter().any(|r| r.released_at.is_none());
    let mine = s.owner_user_id == viewer;
    let mut v = json!({
        "share_id": s.share_id,
        "kind": s.kind,
        "title": s.title,
        "status": effective_status(s, cap.as_ref()),
        "status_reason": s.status_reason,
        "hidden": s.hidden,
        "revision": s.revision,
        "created_at": s.created_at,
        "updated_at": s.updated_at,
        "stopped_at": s.stopped_at,
        "owner_user_id": s.owner_user_id,
        "created_by_app": s.created_by_app,
        "related_share_id": s.related_share_id,
        "obj_id": s.obj_id,
        "source_path": s.source_path,
        "retained": retained,
        "stats": {"views": totals.views, "downloads": totals.downloads, "bytes": totals.bytes, "visitors": totals.visitors, "last_access_at": last},
    });
    if let Some(ver) = version {
        v["obj_type"] = json!(ver.obj_type);
        v["name"] = json!(ver.root_name);
        v["mime"] = json!(ver.mime);
        v["total_size"] = json!(ver.total_size);
        v["item_count"] = json!(ver.item_count);
        v["content_kind"] = json!(kind_of(ver.mime.as_deref(), ver.obj_type == ndn_lib::OBJ_TYPE_DIR));
        v["excluded"] = ver.excluded;
        v["prepared_at"] = json!(ver.prepared_at);
    }
    if let Some(pol) = policy {
        v["access"] = json!(pol.access);
        v["has_password"] = json!(pol.password_hash.is_some());
        v["allow_download"] = json!(pol.allow_download);
    }
    if let Some(c) = &cap {
        v["expires_at"] = json!(c.expires_at);
        if mine {
            v["url"] = json!(link_url(state, Some(&c.cap_id)));
        }
    }
    if let Some(d) = delivery {
        v["target_app"] = json!(d.target_app);
        v["delivery_state"] = json!(d.delivery_state);
        v["business_ref"] = json!(d.business_ref);
        v["handoff_at"] = json!(d.handoff_at);
        v["reported_at"] = json!(d.reported_at);
        v["delivery_error"] = json!(d.error);
    }
    Ok(v)
}

impl crate::state::AppState {
    /// `signed` access URLs: the owner's active link shares of this source.
    pub fn share_signed_urls(&self, owner: &str, source_path: &str) -> Vec<Value> {
        let Ok(rows) = self.db.share_list(&ListFilter {
            owner: Some(owner.to_string()),
            kind: Some("link".into()),
            status: Some("active".into()),
            source_path: Some(source_path.to_string()),
            include_hidden: true,
            limit: 20,
            ..Default::default()
        }) else {
            return Vec::new();
        };
        let base = self.config.share.base_url.as_deref();
        rows.into_iter()
            .filter_map(|s| {
                let cap = self.db.share_cap(s.cap_id.as_deref()?).ok()??;
                if cap.expires_at.is_some_and(|e| e <= unix_now()) {
                    return None;
                }
                let url = format!("{}/s/{}", base?.trim_end_matches('/'), self.share.token_for(&cap.cap_id));
                Some(json!({"kind": "signed", "url": url, "expires_at": cap.expires_at, "cap_id": cap.cap_id, "share_id": s.share_id, "revocable": true}))
            })
            .collect()
    }
}

// ---------- capabilities / plan ----------

fn capabilities(state: &SharedState) -> Value {
    let mut blocked = Vec::new();
    if !ShareMgr::store_configured(&state.config) {
        blocked.push(json!({"action": "create_link", "reason": "named_store_unavailable"}));
        blocked.push(json!({"action": "app_handoff", "reason": "named_store_unavailable"}));
    } else if state.config.share.base_url.is_none() {
        blocked.push(json!({"action": "create_link", "reason": "share_host_unavailable"}));
    }
    let cfg = &state.config.share;
    json!({
        "actions": ["create_link", "app_handoff"],
        "access": ["holder", "password"],
        "expiry_presets": EXPIRY_PRESETS,
        "default_expiry": DEFAULT_EXPIRY,
        "limits": {"sync_threshold": cfg.sync_threshold, "max_total_size": cfg.max_total_size, "max_items": cfg.max_items},
        "targets": [
            {"app": "messagehub", "accepts": "any", "max_item_size": MESSAGEHUB_MAX_ITEM, "fallback": "link"},
            {"app": "homestation", "accepts": ["image", "video", "audio"], "max_item_size": HOMESTATION_MAX_ITEM, "fallback": "link"}
        ],
        "blocked": blocked,
        "share_base_url": cfg.base_url,
        "public": {"dir": "cyfs:///srv/publish", "base_url": cfg.public_base_url},
    })
}

/// How an app target takes this content: as an object attachment, or as a
/// non-expiring holder link (D14) when it cannot.
fn target_fit(target: &str, plan: &SourcePlan) -> NfsResult<Value> {
    let kind = kind_of(plan.mime.as_deref(), plan.kind == "dir");
    let (mode, why) = match target {
        "homestation" => {
            if plan.kind == "dir" {
                ("link", Some("folder"))
            } else if !matches!(kind, "image" | "video" | "audio") {
                ("link", Some("not_media"))
            } else if plan.total_size > HOMESTATION_MAX_ITEM {
                ("link", Some("too_large"))
            } else {
                ("attachment", None)
            }
        }
        "messagehub" => {
            if plan.kind == "dir" {
                ("link", Some("folder"))
            } else if plan.total_size > MESSAGEHUB_MAX_ITEM {
                ("link", Some("too_large"))
            } else {
                ("attachment", None)
            }
        }
        other => return Err(reason(ErrorCode::Unsupported, "target_unsupported", format!("unknown share target '{}'", other))),
    };
    Ok(json!({"app": target, "mode": mode, "reason": why, "kind": kind}))
}

fn source_public(state: &SharedState, source_path: &str) -> Value {
    let rel_all = source_path.strip_prefix("cyfs:///").unwrap_or("");
    let (root, rel) = rel_all.split_once('/').unwrap_or((rel_all, ""));
    let in_publish = root == "srv" && (rel == "publish" || rel.starts_with("publish/"));
    let public_acl = state.acls.level(&Actor::Anonymous, root, rel) >= Level::Read;
    let url = if in_publish {
        super::public::public_url(state, rel.strip_prefix("publish").unwrap_or("").trim_start_matches('/'))
    } else {
        None
    };
    json!({"in_publish_dir": in_publish, "public_acl": public_acl, "public_url": url, "principal": PUBLIC})
}

async fn plan(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let intent = args["intent"].as_str().unwrap_or("create_link");
    if !matches!(intent, "create_link" | "app_handoff") {
        return Err(invalid("intent must be create_link or app_handoff"));
    }
    let source = source_arg(args)?;
    let plan = state.share_plan_source(&p.file_actor, &source)?;
    let mut warnings = Vec::new();
    if !plan.excluded.is_empty() {
        warnings.push("excluded_items");
    }
    if plan.total_size > state.config.share.sync_threshold {
        warnings.push("runs_in_background");
    }
    let public = source_public(state, &plan.source_path);
    if public["in_publish_dir"] == true || public["public_acl"] == true {
        warnings.push("already_public");
    }
    let existing = state
        .db
        .share_list(&ListFilter {
            owner: Some(p.owner.clone()),
            source_path: Some(plan.source_path.clone()),
            status: Some("active".into()),
            include_hidden: true,
            limit: 20,
            ..Default::default()
        })?
        .iter()
        .map(|s| share_view(state, &p.owner, s))
        .collect::<NfsResult<Vec<_>>>()?
        .into_iter()
        .filter(|v| v["status"] == "active")
        .collect::<Vec<_>>();
    // A folder with an index page could be a website (D9: going live is App Service's job).
    let has_index = matches!(&plan.root, snapshot::PlanNode::Dir { children, .. }
        if children.iter().any(|c| matches!(c, snapshot::PlanNode::File { name, .. } if name.eq_ignore_ascii_case("index.html"))));
    let mut out = json!({
        "site": {"has_index_html": has_index},
        "plan_fingerprint": plan.fingerprint,
        "source": {"path": plan.source_path, "name": plan.name, "kind": plan.kind, "mime": plan.mime},
        "estimate": {"total_size": plan.total_size, "item_count": plan.item_count, "excluded": plan.excluded},
        "warnings": warnings,
        "requires_task": plan.total_size > state.config.share.sync_threshold,
        "existing": existing,
        "source_public": public,
    });
    if let Some(target) = args["target"].as_str().or(args["target_app"].as_str()) {
        out["target_fit"] = target_fit(target, &plan)?;
    }
    Ok(out)
}

// ---------- create / handoff (operations) ----------

#[derive(Clone, Copy, PartialEq)]
enum Intent {
    Link,
    Handoff,
}

#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct PolicyInput {
    pub access: String,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default = "yes")]
    pub allow_download: bool,
    /// PHC hash; the plaintext never reaches filedb.
    #[serde(default)]
    pub password_hash: Option<String>,
}

fn yes() -> bool {
    true
}

/// What an operation runs; stored in `share_operations.input_json`.
#[derive(Debug, Clone, serde::Serialize, Deserialize)]
pub struct OperationInput {
    pub intent: String,
    pub source: Locator,
    pub title: Option<String>,
    #[serde(default)]
    pub policy: Option<PolicyInput>,
    #[serde(default)]
    pub target_app: Option<String>,
    #[serde(default)]
    pub plan_fingerprint: Option<String>,
    /// Comparable digest of the request (idempotent replays must match it).
    pub digest: String,
}

fn parse_policy(state: &SharedState, v: &Value) -> NfsResult<(PolicyInput, Option<String>)> {
    let access = v["access"].as_str().unwrap_or("holder");
    if !matches!(access, "holder" | "password") {
        return Err(invalid("policy.access must be holder or password"));
    }
    let password = v["password"].as_str().filter(|p| !p.is_empty());
    if access == "password" && password.is_none() {
        return Err(invalid("policy.password required for password access"));
    }
    if password.is_some_and(|p| p.chars().count() > 128) {
        return Err(invalid("password too long (max 128)"));
    }
    let expires_at = match v.get("expires_at") {
        None => Some(unix_now() + DEFAULT_EXPIRY),
        Some(Value::Null) => None,
        Some(e) => {
            let e = e.as_i64().ok_or_else(|| invalid("policy.expires_at must be unix seconds or null"))?;
            if e <= unix_now() {
                return Err(invalid("policy.expires_at must be in the future"));
            }
            Some(e)
        }
    };
    // Password digest for idempotency comparison only (keyed, not stored as a hash of the password alone).
    let digest = password.map(|pw| state.share.token_for(&format!("password:{pw}")));
    Ok((
        PolicyInput {
            access: if password.is_some() { "password".into() } else { "holder".into() },
            expires_at,
            allow_download: v["allow_download"].as_bool().unwrap_or(true),
            password_hash: password.map(ShareMgr::hash_password),
        },
        digest,
    ))
}

async fn create(state: &SharedState, p: &Principal, args: &Value, bearer: Option<&str>, intent: Intent) -> NfsResult<Value> {
    let key = args["idempotency_key"]
        .as_str()
        .filter(|k| !k.is_empty() && k.len() <= 200)
        .ok_or_else(|| invalid("idempotency_key required (max 200 bytes)"))?;
    let source = source_arg(args)?;
    let (policy, target_app, pw_digest) = match intent {
        Intent::Link => {
            let (policy, digest) = parse_policy(state, &args["policy"])?;
            (Some(policy), None, digest)
        }
        Intent::Handoff => {
            let target = str_arg(args, "target_app")?;
            if !matches!(target, "messagehub" | "homestation") {
                return Err(reason(ErrorCode::Unsupported, "target_unsupported", format!("unknown share target '{}'", target)));
            }
            (None, Some(target.to_string()), None)
        }
    };
    let title = args["title"].as_str().map(|t| clean_title(Some(t), ""));
    let digest = {
        let body = json!([
            if intent == Intent::Link { "create_link" } else { "app_handoff" },
            args["source"],
            title,
            policy.as_ref().map(|p| json!([p.access, args["policy"].get("expires_at"), p.allow_download])),
            pw_digest,
            target_app
        ]);
        ShareMgr::token_hash(&body.to_string())
    };
    if let Some(op) = state.db.share_operation_by_key(&p.owner, key)? {
        return replay(state, p, op, &digest);
    }
    let plan = state.share_plan_source(&p.file_actor, &source)?;
    if let Some(fp) = args["plan_fingerprint"].as_str() {
        if fp != plan.fingerprint {
            return Err(reason(ErrorCode::Stale, "plan_stale", "the source changed since it was planned; plan again"));
        }
    }
    if intent == Intent::Link && state.config.share.base_url.is_none() {
        return Err(reason(ErrorCode::Unsupported, "share_host_unavailable", "share links are not configured on this server"));
    }
    state.share.store(&state.config).await?;
    let input = OperationInput {
        intent: if intent == Intent::Link { "create_link".into() } else { "app_handoff".into() },
        source,
        title,
        policy,
        target_app,
        plan_fingerprint: Some(plan.fingerprint.clone()),
        digest: digest.clone(),
    };
    let now = unix_now();
    let mut op = OperationRow {
        operation_id: new_id("op"),
        intent: input.intent.clone(),
        owner_user_id: p.owner.clone(),
        actor_user_id: p.actor_user.clone(),
        actor_app_id: p.app.clone(),
        input: serde_json::to_value(&input)?,
        idempotency_key: key.to_string(),
        task_id: None,
        share_id: None,
        state: "running".into(),
        error: None,
        progress: Some(json!({"done_bytes": 0, "total_bytes": plan.total_size})),
        result: None,
        created_at: now,
        updated_at: now,
    };
    if !state.db.share_operation_insert(&op)? {
        let existing = state.db.share_operation_by_key(&p.owner, key)?.ok_or_else(|| internal("operation vanished"))?;
        return replay(state, p, existing, &digest);
    }
    if plan.total_size <= state.config.share.sync_threshold {
        let op = execute(state, op, Some(plan), p.source).await?;
        return operation_view(state, p, &op);
    }
    op.task_id = mirror_task(state, &op, &plan, bearer).await;
    state.db.share_operation_update(&op)?;
    spawn_operation(state.clone(), op.operation_id.clone(), p.source);
    operation_view(state, p, &op)
}

fn replay(state: &SharedState, p: &Principal, op: OperationRow, digest: &str) -> NfsResult<Value> {
    if op.input["digest"].as_str() != Some(digest) {
        return Err(reason(
            ErrorCode::NamespaceConflict,
            "idempotency_mismatch",
            "idempotency_key already used with a different request",
        ));
    }
    operation_view(state, p, &op)
}

/// TaskManager mirror for background snapshots (best effort: the operation
/// row is the business record, so a missing TaskManager only loses the task UI).
async fn mirror_task(state: &SharedState, op: &OperationRow, plan: &SourcePlan, bearer: Option<&str>) -> Option<String> {
    let token = bearer?;
    let claims = buckyos_api::verify_nfs_session_token(token, match &state.config.auth {
        crate::config::AuthMode::TestKey { public_key, .. } => Some(public_key.as_str()),
        _ => state.config.copy_test.as_ref().map(|c| c.public_key.as_str()),
    })
    .await
    .ok()?;
    let api = crate::copy::client(state).await.ok()?;
    let req = buckyos_api::CreateDelegatedTaskReq {
        task_id: None,
        name: format!("Prepare share: {}", plan.name),
        schema_id: buckyos_api::NFS_SHARE_SNAPSHOT_SCHEMA_ID.into(),
        schema_version: Some(1),
        input: json!({
            "operation_id": op.operation_id,
            "intent": op.intent,
            "source_path": plan.source_path,
            "total_size": plan.total_size,
            "item_count": plan.item_count,
        }),
        creator: buckyos_api::ActorRef::new(op.owner_user_id.clone(), claims.app_id),
        runner_app_instance_id: None,
        parent_id: None,
        child_control_policy: None,
        policy_preset: None,
        permission_boundary: false,
        storage_domain: Some(buckyos_api::StorageDomain::System),
        idempotency_key: format!("share:{}", op.operation_id),
        retry_of: None,
        supersedes: None,
        message: None,
    };
    match api.create_delegated_task(req).await {
        Ok(task) => Some(task.task_id),
        Err(e) => {
            log::warn!("share operation {}: no task mirror: {}", op.operation_id, e);
            None
        }
    }
}

pub fn spawn_operation(state: SharedState, operation_id: String, source: &'static str) {
    tokio::spawn(async move {
        let Ok(Some(op)) = state.db.share_operation(&operation_id) else { return };
        if !matches!(op.state.as_str(), "preparing" | "running") || state.share.running(&operation_id).is_some() {
            return;
        }
        let task = op.task_id.clone();
        let progress = state.share.progress_for(&operation_id);
        let reporter = task.clone().map(|task_id| {
            let st = state.clone();
            let progress = progress.clone();
            let total = op.progress.as_ref().and_then(|p| p["total_bytes"].as_u64()).unwrap_or(0);
            tokio::spawn(async move {
                let Ok(api) = crate::copy::client(&st).await else { return };
                let _ = api.runner_start(&task_id).await;
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    let done = progress.done_bytes.load(std::sync::atomic::Ordering::Relaxed);
                    if let Ok(t) = api.get_task(&task_id).await {
                        if t.phase.is_terminal()
                            || t.pending_control.as_ref().is_some_and(|c| c.action == buckyos_api::TaskControlAction::Cancel)
                        {
                            progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                            return;
                        }
                    }
                    let _ = api
                        .runner_progress(&task_id, Some(json!({"done_bytes": done, "total_bytes": total})), Some(format!("{done} / {total} bytes")))
                        .await;
                }
            })
        });
        let result = execute(&state, op, None, source).await;
        if let Some(r) = reporter {
            r.abort();
        }
        if let (Some(task_id), Ok(api)) = (task, crate::copy::client(&state).await) {
            let outcome = match &result {
                Ok(op) if op.state == "completed" => {
                    api.runner_complete(&task_id, json!({"obj_id": op.result.as_ref().map(|r| r["obj_id"].clone()), "share_id": op.share_id})).await
                }
                Ok(op) => {
                    let msg = op.error.as_ref().and_then(|e| e["message"].as_str()).unwrap_or("share snapshot failed").to_string();
                    api.runner_fail(&task_id, "SHARE_SNAPSHOT_FAILED", msg, op.error.clone()).await
                }
                Err(e) => api.runner_fail(&task_id, "SHARE_SNAPSHOT_FAILED", e.message.clone(), None).await,
            };
            if let Err(e) = outcome {
                log::warn!("share task {} close failed: {}", task_id, e);
            }
        }
    });
}

/// Restart recovery (design §3.7): background operations resume; in-request
/// ones died with the request.
pub fn recover_operations(state: &SharedState) {
    let Ok(ops) = state.db.share_operations_unfinished() else { return };
    for mut op in ops {
        let background = op.progress.as_ref().and_then(|p| p["total_bytes"].as_u64()).unwrap_or(0) > state.config.share.sync_threshold;
        if background {
            spawn_operation(state.clone(), op.operation_id.clone(), "system");
        } else {
            op.state = "failed".into();
            op.error = Some(reason(ErrorCode::Internal, "interrupted", "the server restarted during this operation").to_json());
            let _ = state.db.share_operation_update(&op);
        }
    }
}

fn fail(state: &SharedState, mut op: OperationRow, e: NfsError) -> NfsResult<OperationRow> {
    op.state = if e.details["reason"] == "canceled" { "canceled".into() } else { "failed".into() };
    op.error = Some(e.to_json());
    state.db.share_operation_update(&op)?;
    Ok(op)
}

/// Snapshot the source and create the share(s). `plan` is reused when the
/// request just made it; background runs plan again (ACL re-checked).
async fn execute(state: &SharedState, mut op: OperationRow, plan: Option<SourcePlan>, source: &'static str) -> NfsResult<OperationRow> {
    let input: OperationInput = serde_json::from_value(op.input.clone())?;
    let plan = match plan {
        Some(p) => p,
        None => {
            let actor = owner_actor(state, &op.owner_user_id).await;
            match state.share_plan_source(&actor, &input.source) {
                Ok(p) => p,
                Err(e) => return fail(state, op, e),
            }
        }
    };
    let store = match state.share.store(&state.config).await {
        Ok(s) => s,
        Err(e) => return fail(state, op, e),
    };
    let progress = state.share.progress_for(&op.operation_id);
    let snap = snapshot::run_snapshot(store, &plan, &progress).await;
    state.share.finish_running(&op.operation_id);
    let snap = match snap {
        Ok(s) => s,
        Err(e) => return fail(state, op, e),
    };
    let now = unix_now();
    state.db.share_version_put(&VersionRow {
        obj_id: snap.obj_id.clone(),
        obj_type: snap.obj_type.to_string(),
        root_name: plan.name.clone(),
        mime: plan.mime.clone(),
        total_size: snap.total_size as i64,
        item_count: snap.item_count as i64,
        source_path: plan.source_path.clone(),
        source_mtime: Some(plan.mtime),
        source_size: Some(plan.total_size as i64),
        excluded: json!(plan.excluded),
        prepared_at: now,
    })?;
    let title = input.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| clean_title(None, &plan.name));
    let ctx = NewCtx { op: &op, title: &title, obj_id: &snap.obj_id, source_path: &plan.source_path, source };
    let created = match input.intent.as_str() {
        "create_link" => {
            let policy = input.policy.clone().ok_or_else(|| invalid("missing policy"))?;
            create_link_share(state, &ctx, &policy, None, None).await.map(|id| (id, None))
        }
        _ => {
            let target = input.target_app.clone().unwrap_or_default();
            let fit = target_fit(&target, &plan)?;
            create_delivery(state, &ctx, &target, fit["mode"] == "link").await
        }
    };
    let (share_id, link_share_id) = match created {
        Ok(ids) => ids,
        Err(e) => return fail(state, op, e),
    };
    op.state = "completed".into();
    op.share_id = Some(share_id);
    op.progress = Some(json!({"done_bytes": snap.total_size, "total_bytes": snap.total_size}));
    op.result = Some(json!({
        "obj_id": snap.obj_id,
        "total_size": snap.total_size,
        "item_count": snap.item_count,
        "source_changed": snap.source_changed,
        "link_share_id": link_share_id,
        "kind": kind_of(plan.mime.as_deref(), plan.kind == "dir"),
    }));
    state.db.share_operation_update(&op)?;
    Ok(op)
}

struct NewCtx<'a> {
    op: &'a OperationRow,
    title: &'a str,
    obj_id: &'a str,
    source_path: &'a str,
    source: &'static str,
}

async fn pin(state: &SharedState, share_id: &str, obj_id: &str) -> NfsResult<()> {
    let store = state.share.store(&state.config).await?;
    let id = ndn_lib::ObjId::new(obj_id).map_err(|e| internal(format!("bad obj id: {e}")))?;
    store
        .pin(&id, &pin_owner(share_id), named_store::PinScope::Recursive, None)
        .await
        .map_err(|e| internal(format!("pin snapshot: {e}")))
}

fn base_row(ctx: &NewCtx, share_id: &str, kind: &str, related: Option<&str>, cap_id: Option<&str>) -> ShareRow {
    let now = unix_now();
    ShareRow {
        share_id: share_id.to_string(),
        kind: kind.to_string(),
        owner_user_id: ctx.op.owner_user_id.clone(),
        created_by_app: ctx.op.actor_app_id.clone(),
        title: ctx.title.to_string(),
        obj_id: ctx.obj_id.to_string(),
        source_path: ctx.source_path.to_string(),
        cap_id: cap_id.map(String::from),
        related_share_id: related.map(String::from),
        status: "active".into(),
        status_reason: None,
        hidden: false,
        revision: 1,
        created_at: now,
        updated_at: now,
        stopped_at: None,
    }
}

async fn insert_pinned(state: &SharedState, new: NewShare<'_>) -> NfsResult<()> {
    pin(state, &new.share.share_id, &new.share.obj_id).await?;
    let (share_id, obj_id) = (new.share.share_id.clone(), new.share.obj_id.clone());
    if let Err(e) = state.db.share_create(new) {
        super::release_pin(state, &share_id, &obj_id).await;
        return Err(e);
    }
    Ok(())
}

async fn create_link_share(
    state: &SharedState,
    ctx: &NewCtx<'_>,
    policy: &PolicyInput,
    share_id: Option<String>,
    related: Option<&str>,
) -> NfsResult<String> {
    let share_id = share_id.unwrap_or_else(|| new_id("sh"));
    let cap_id = new_id("cap");
    let token_hash = ShareMgr::token_hash(&state.share.token_for(&cap_id));
    let subtree = format!("obj:{}", ctx.obj_id);
    let row = base_row(ctx, &share_id, "link", related, Some(&cap_id));
    let policy_row = PolicyRow {
        share_id: share_id.clone(),
        access: policy.access.clone(),
        password_hash: policy.password_hash.clone(),
        password_rev: 0,
        allow_download: policy.allow_download,
    };
    insert_pinned(
        state,
        NewShare {
            share: &row,
            grant: Some((&cap_id, &token_hash, &subtree, policy.expires_at)),
            policy: Some(&policy_row),
            delivery: None,
            pin_owner: &pin_owner(&share_id),
            audit: AuditEntry {
                share_id: &share_id,
                action: "create",
                revision_before: None,
                revision_after: Some(1),
                actor_user_id: &ctx.op.actor_user_id,
                actor_app_id: ctx.op.actor_app_id.as_deref(),
                source: ctx.source,
                reason: None,
                outcome: "ok",
                at: row.created_at,
            },
        },
    )
    .await?;
    Ok(share_id)
}

/// An app delivery; in link mode also a non-expiring holder link (D14), the
/// two shares pointing at each other.
async fn create_delivery(state: &SharedState, ctx: &NewCtx<'_>, target: &str, link_mode: bool) -> NfsResult<(String, Option<String>)> {
    let share_id = new_id("sh");
    let link_id = link_mode.then(|| new_id("sh"));
    let row = base_row(ctx, &share_id, "app_delivery", link_id.as_deref(), None);
    let delivery = DeliveryRow {
        share_id: share_id.clone(),
        target_app: target.to_string(),
        handoff_at: row.created_at,
        delivery_state: "handed_off".into(),
        business_ref: None,
        reported_at: None,
        error: None,
    };
    insert_pinned(
        state,
        NewShare {
            share: &row,
            grant: None,
            policy: None,
            delivery: Some(&delivery),
            pin_owner: &pin_owner(&share_id),
            audit: AuditEntry {
                share_id: &share_id,
                action: "create",
                revision_before: None,
                revision_after: Some(1),
                actor_user_id: &ctx.op.actor_user_id,
                actor_app_id: ctx.op.actor_app_id.as_deref(),
                source: ctx.source,
                reason: Some(target),
                outcome: "ok",
                at: row.created_at,
            },
        },
    )
    .await?;
    if let Some(link_id) = &link_id {
        if state.config.share.base_url.is_none() {
            return Err(reason(ErrorCode::Unsupported, "share_host_unavailable", "share links are not configured on this server"));
        }
        let policy = PolicyInput { access: "holder".into(), expires_at: None, allow_download: true, password_hash: None };
        create_link_share(state, ctx, &policy, Some(link_id.clone()), Some(&share_id)).await?;
    }
    Ok((share_id, link_id))
}

/// The launch payload the Desktop passes to the target app (design §3.10).
fn handoff_view(state: &SharedState, op: &OperationRow) -> NfsResult<Option<Value>> {
    let (Some(share_id), Some(result)) = (&op.share_id, &op.result) else { return Ok(None) };
    let Some(delivery) = state.db.share_delivery(share_id)? else { return Ok(None) };
    let Some(share) = state.db.share_get(share_id)? else { return Ok(None) };
    let version = state.db.share_version_get(&share.obj_id)?;
    let (name, size, mime) = match &version {
        Some(v) => (v.root_name.clone(), v.total_size, v.mime.clone()),
        None => (share.title.clone(), 0, None),
    };
    let link_share_id = result["link_share_id"].as_str();
    let url = match link_share_id {
        Some(id) => state.db.share_get(id)?.and_then(|s| link_url(state, s.cap_id.as_deref())),
        None => None,
    };
    let kind = result["kind"].as_str().unwrap_or("file");
    let mode = if link_share_id.is_some() { "link" } else { "attachment" };
    let payload = match (delivery.target_app.as_str(), mode) {
        ("messagehub", "attachment") => json!({"share": {"shareId": share_id, "objId": share.obj_id, "name": name, "size": size, "mime": mime}}),
        ("messagehub", _) => json!({"share": {"shareId": share_id, "name": name, "size": size, "mime": mime, "url": url, "title": share.title}}),
        (_, "attachment") => json!({"compose": {"shareId": share_id, "attachments": [{"kind": kind, "name": name, "object": share.obj_id, "status": "uploaded"}]}}),
        _ => json!({"compose": {"shareId": share_id, "link": {"url": url, "title": share.title.chars().take(120).collect::<String>()}}}),
    };
    Ok(Some(json!({
        "share_id": share_id,
        "obj_id": share.obj_id,
        "name": name,
        "size": size,
        "mime": mime,
        "kind": kind,
        "mode": mode,
        "link_share_id": link_share_id,
        "url": url,
        "launch": {"appId": delivery.target_app, "payload": payload},
    })))
}

fn operation_view(state: &SharedState, p: &Principal, op: &OperationRow) -> NfsResult<Value> {
    let mut progress = op.progress.clone().unwrap_or(json!({}));
    if let Some(live) = state.share.running(&op.operation_id) {
        progress["done_bytes"] = json!(live.done_bytes.load(std::sync::atomic::Ordering::Relaxed));
    }
    let mut v = json!({
        "operation_id": op.operation_id,
        "intent": op.intent,
        "state": op.state,
        "share_id": op.share_id,
        "task_id": op.task_id,
        "error": op.error,
        "progress": progress,
        "created_at": op.created_at,
        "updated_at": op.updated_at,
    });
    if let Some(r) = &op.result {
        v["source_changed"] = r["source_changed"].clone();
        v["obj_id"] = r["obj_id"].clone();
    }
    if op.state == "completed" {
        if let Some(share_id) = &op.share_id {
            if let Some(share) = state.db.share_get(share_id)? {
                v["share"] = share_view(state, &p.owner, &share)?;
                if share.owner_user_id == p.owner {
                    v["url"] = json!(link_url(state, share.cap_id.as_deref()));
                }
            }
        }
        if op.intent == "app_handoff" {
            v["handoff"] = json!(handoff_view(state, op)?);
        }
    }
    Ok(v)
}

fn operation(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let id = str_arg(args, "operation_id")?;
    let op = match state.db.share_operation(id)? {
        Some(op) if op.owner_user_id == p.owner || p.admin => op,
        _ => return Err(not_found(format!("operation '{}' not found", id))),
    };
    if args["cancel"].as_bool() == Some(true) {
        if let Some(progress) = state.share.running(id) {
            progress.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
    operation_view(state, p, &op)
}

// ---------- read ----------

fn get(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let share = load_share(state, p, str_arg(args, "share_id")?)?;
    let mut v = share_view(state, &p.owner, &share)?;
    v["retention"] = json!(state
        .db
        .share_retentions(&share.share_id)?
        .into_iter()
        .map(|r| json!({"obj_id": r.obj_id, "pin_owner": r.pin_owner, "pinned_at": r.pinned_at, "released_at": r.released_at}))
        .collect::<Vec<_>>());
    v["audit"] = json!(state.db.share_audit(&share.share_id, 50)?);
    if let Some(rel) = &share.related_share_id {
        if let Some(r) = state.db.share_get(rel)? {
            v["related"] = share_view(state, &p.owner, &r)?;
        }
    }
    Ok(v)
}

fn list(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let limit = args["limit"].as_i64().unwrap_or(50).clamp(1, 200);
    let offset = args["cursor"].as_str().and_then(|c| c.parse::<i64>().ok()).unwrap_or(0).max(0);
    let owner = match args["owner"].as_str() {
        Some("*") if p.admin => None,
        Some(u) if p.admin => Some(u.to_string()),
        _ => Some(p.owner.clone()),
    };
    let source_path = match args.get("source").filter(|v| !v.is_null()) {
        Some(Value::String(s)) => Some(s.clone()),
        Some(v) => {
            let loc: Locator = serde_json::from_value(v.clone()).map_err(|e| invalid(format!("bad source: {e}")))?;
            Some(state.share_plan_source(&p.file_actor, &loc)?.source_path)
        }
        None => None,
    };
    let rows = state.db.share_list(&ListFilter {
        owner,
        kind: args["kind"].as_str().map(String::from),
        status: args["status"].as_str().map(String::from),
        source_path,
        obj_id: args["obj_id"].as_str().map(String::from),
        q: args["q"].as_str().map(String::from),
        include_hidden: args["include_hidden"].as_bool().unwrap_or(false),
        offset,
        limit: limit + 1,
    })?;
    let more = rows.len() as i64 > limit;
    let items = rows
        .iter()
        .take(limit as usize)
        .map(|s| share_view(state, &p.owner, s))
        .collect::<NfsResult<Vec<_>>>()?;
    Ok(json!({"items": items, "next_cursor": more.then(|| (offset + limit).to_string())}))
}

// ---------- update / stop / hide ----------

fn conflict(state: &SharedState, p: &Principal, current: &ShareRow) -> NfsError {
    let view = share_view(state, &p.owner, current).unwrap_or(Value::Null);
    reason(ErrorCode::RevMismatch, "revision_conflict", "the share changed; review the current state")
        .with("current", view)
        .with("revision", json!(current.revision))
}

fn require_active(share: &ShareRow, cap: Option<&CapRow>) -> NfsResult<()> {
    if effective_status(share, cap) != "active" {
        return Err(reason(ErrorCode::Stale, "share_not_active", format!("share is {}", effective_status(share, cap))));
    }
    Ok(())
}

fn update(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let share = load_share(state, p, str_arg(args, "share_id")?)?;
    let expected = args["revision"].as_i64().ok_or_else(|| invalid("revision required"))?;
    let changes = args.get("changes").filter(|c| c.is_object()).ok_or_else(|| invalid("changes required"))?;
    let cap = match &share.cap_id {
        Some(c) => state.db.share_cap(c)?,
        None => None,
    };
    require_active(&share, cap.as_ref())?;
    let title = match changes.get("title") {
        Some(Value::String(t)) if !t.trim().is_empty() => Some(clean_title(Some(t), "")),
        Some(Value::Null) | None => None,
        _ => return Err(invalid("title must be a non-empty string")),
    };
    let link_change = ["expires_at", "password", "allow_download", "access"].iter().any(|k| changes.get(*k).is_some());
    if link_change && share.kind != "link" {
        return Err(invalid("only link shares have access settings"));
    }
    let expires: Option<Option<i64>> = match changes.get("expires_at") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(v) => {
            let e = v.as_i64().ok_or_else(|| invalid("expires_at must be unix seconds or null"))?;
            if e <= unix_now() {
                return Err(invalid("expires_at must be in the future"));
            }
            Some(Some(e))
        }
    };
    let password: Option<Option<String>> = match changes.get("password") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::String(pw)) if !pw.is_empty() && pw.chars().count() <= 128 => Some(Some(ShareMgr::hash_password(pw))),
        _ => return Err(invalid("password must be a 1–128 character string or null")),
    };
    let access = match changes["access"].as_str() {
        Some("holder") => Some("holder"),
        Some("password") => Some("password"),
        None => None,
        _ => return Err(invalid("access must be holder or password")),
    };
    let allow_download = changes.get("allow_download").map(|v| v.as_bool().ok_or_else(|| invalid("allow_download must be bool"))).transpose()?;
    let policy = state.db.share_policy(&share.share_id)?;
    // Resulting access: setting a password implies password access; clearing it implies holder.
    let new_access = match (&password, access) {
        (Some(Some(_)), _) => Some("password"),
        (Some(None), _) => Some("holder"),
        (None, Some("password")) if policy.as_ref().and_then(|p| p.password_hash.as_ref()).is_none() => {
            return Err(invalid("set a password to use password access"));
        }
        (None, a) => a,
    };
    let now = unix_now();
    let source = audit_source(p, &share);
    let reason_text = args["reason"].as_str();
    let outcome = state.db.share_update(&share.share_id, expected, |tx, cur| {
        use rusqlite::params;
        tx.execute(
            "UPDATE shares SET title=COALESCE(?2, title), revision=revision+1, updated_at=?3 WHERE share_id=?1",
            params![cur.share_id, title, now],
        )?;
        if let (Some(e), Some(cap)) = (expires, &cur.cap_id) {
            tx.execute("UPDATE grants SET expires_at=?2 WHERE cap_id=?1", params![cap, e])?;
        }
        if let Some(pw) = &password {
            tx.execute(
                "UPDATE share_link_policy SET password_hash=?2, password_rev=password_rev+1 WHERE share_id=?1",
                params![cur.share_id, pw],
            )?;
        }
        if let Some(a) = new_access {
            tx.execute("UPDATE share_link_policy SET access=?2 WHERE share_id=?1", params![cur.share_id, a])?;
        }
        if let Some(d) = allow_download {
            tx.execute("UPDATE share_link_policy SET allow_download=?2 WHERE share_id=?1", params![cur.share_id, d as i64])?;
        }
        insert_audit(
            tx,
            &AuditEntry {
                share_id: &cur.share_id,
                action: "update",
                revision_before: Some(cur.revision),
                revision_after: Some(cur.revision + 1),
                actor_user_id: &p.actor_user,
                actor_app_id: p.app.as_deref(),
                source,
                reason: reason_text,
                outcome: "ok",
                at: now,
            },
        )?;
        Ok(cur.revision + 1)
    })?;
    match outcome {
        Ok(rev) => {
            let fresh = state.db.share_get(&share.share_id)?.ok_or_else(|| internal("share vanished"))?;
            Ok(json!({"revision": rev, "share": share_view(state, &p.owner, &fresh)?}))
        }
        Err(current) => Err(conflict(state, p, &current)),
    }
}

async fn stop(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let share = load_share(state, p, str_arg(args, "share_id")?)?;
    let expected = args["revision"].as_i64().ok_or_else(|| invalid("revision required"))?;
    if matches!(share.status.as_str(), "stopped" | "stopping" | "expired" | "failed") {
        return Ok(json!({"status": share.status, "revision": share.revision}));
    }
    let now = unix_now();
    let source = audit_source(p, &share);
    let reason_text = args["reason"].as_str();
    let outcome = state.db.share_update(&share.share_id, expected, |tx, cur| {
        use rusqlite::params;
        tx.execute(
            "UPDATE shares SET status='stopping', status_reason=?2, revision=revision+1, updated_at=?3 WHERE share_id=?1",
            params![cur.share_id, reason_text, now],
        )?;
        if let Some(cap) = &cur.cap_id {
            tx.execute("UPDATE grants SET revoked=1 WHERE cap_id=?1", params![cap])?;
        }
        insert_audit(
            tx,
            &AuditEntry {
                share_id: &cur.share_id,
                action: "stop",
                revision_before: Some(cur.revision),
                revision_after: Some(cur.revision + 1),
                actor_user_id: &p.actor_user,
                actor_app_id: p.app.as_deref(),
                source,
                reason: reason_text,
                outcome: "ok",
                at: now,
            },
        )?;
        Ok(cur.revision + 1)
    })?;
    let rev = match outcome {
        Ok(rev) => rev,
        Err(current) => return Err(conflict(state, p, &current)),
    };
    let stopping = state.db.share_get(&share.share_id)?.ok_or_else(|| internal("share vanished"))?;
    settle_share(state, &stopping, "stopped", "stop").await;
    let status = state.db.share_get(&share.share_id)?.map(|s| s.status).unwrap_or_else(|| "stopping".into());
    Ok(json!({"status": status, "revision": rev}))
}

fn hide(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let share = load_share(state, p, str_arg(args, "share_id")?)?;
    let hidden = args["hidden"].as_bool().ok_or_else(|| invalid("hidden must be bool"))?;
    let now = unix_now();
    let source = audit_source(p, &share);
    let outcome = state.db.share_update(&share.share_id, share.revision, |tx, cur| {
        tx.execute(
            "UPDATE shares SET hidden=?2, revision=revision+1, updated_at=?3 WHERE share_id=?1",
            rusqlite::params![cur.share_id, hidden as i64, now],
        )?;
        insert_audit(
            tx,
            &AuditEntry {
                share_id: &cur.share_id,
                action: "hide",
                revision_before: Some(cur.revision),
                revision_after: Some(cur.revision + 1),
                actor_user_id: &p.actor_user,
                actor_app_id: p.app.as_deref(),
                source,
                reason: Some(if hidden { "hide" } else { "show" }),
                outcome: "ok",
                at: now,
            },
        )?;
        Ok(cur.revision + 1)
    })?;
    match outcome {
        Ok(rev) => Ok(json!({"hidden": hidden, "revision": rev})),
        Err(current) => Err(conflict(state, p, &current)),
    }
}

// ---------- app report ----------

async fn report(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let share = load_share(state, p, str_arg(args, "share_id")?)?;
    if share.owner_user_id != p.owner {
        return Err(reason(ErrorCode::PermissionDenied, "not_owner", "only the share owner reports its delivery"));
    }
    let delivery_state = str_arg(args, "delivery_state")?;
    if !matches!(delivery_state, "confirmed" | "canceled" | "failed" | "unknown") {
        return Err(invalid("delivery_state must be confirmed, canceled, failed or unknown"));
    }
    if state.db.share_delivery(&share.share_id)?.is_none() {
        return Err(invalid("not an app delivery"));
    }
    let observed_at = args["observed_at"].as_i64().unwrap_or_else(|| unix_now() * 1000);
    let business_ref = args.get("business_ref").filter(|v| !v.is_null());
    let error = args["error"].as_str();
    let accepted = state.db.share_delivery_report(
        &share.share_id,
        delivery_state,
        business_ref,
        error,
        observed_at,
        AuditEntry {
            share_id: &share.share_id,
            action: "report",
            revision_before: Some(share.revision),
            revision_after: Some(share.revision),
            actor_user_id: &p.actor_user,
            actor_app_id: p.app.as_deref(),
            source: p.source,
            reason: Some(delivery_state),
            outcome: "ok",
            at: unix_now(),
        },
    )?;
    if !accepted {
        return Err(reason(ErrorCode::Stale, "stale_report", "a newer delivery fact is already recorded"));
    }
    // Nothing was sent: release the snapshot (and the link made for this post).
    if matches!(delivery_state, "canceled" | "failed") && share.status == "active" {
        for id in std::iter::once(share.share_id.clone()).chain(share.related_share_id.clone()) {
            if let Some(s) = state.db.share_get(&id)? {
                if s.status == "active" {
                    if let Some(cap) = &s.cap_id {
                        state.db.share_update(&s.share_id, s.revision, |tx, _| {
                            tx.execute("UPDATE grants SET revoked=1 WHERE cap_id=?1", rusqlite::params![cap])?;
                            Ok(())
                        })?.ok();
                    }
                    state.db.share_set_status(&s.share_id, "stopping", Some(delivery_state), true, None)?;
                    let stopping = state.db.share_get(&s.share_id)?.ok_or_else(|| internal("share vanished"))?;
                    settle_share(state, &stopping, "stopped", "stop").await;
                }
            }
        }
    }
    Ok(json!({"accepted": true, "delivery_state": delivery_state}))
}

// ---------- public / stats ----------

async fn public_list(state: &SharedState, args: &Value) -> NfsResult<Value> {
    if args["refresh"].as_bool() == Some(true) || state.share.public_scan_state().is_none() {
        let st = state.clone();
        tokio::task::spawn_blocking(move || st.share_public_scan())
            .await
            .map_err(|e| internal(format!("public scan: {e}")))?;
    }
    let (last, ok) = state.share.public_scan_state().unwrap_or((0, false));
    let entries = state
        .db
        .public_observations()?
        .into_iter()
        .map(|o| {
            let mut v = json!(o);
            if o.entry_kind != "dir" && o.missing_since.is_none() {
                v["url"] = json!(super::public::public_url(state, &o.rel_path));
            }
            v
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "entries": entries,
        "last_scan_at": last,
        "scan_ok": ok,
        "dir": "cyfs:///srv/publish",
        "base_url": state.config.share.public_base_url,
    }))
}

fn stats(state: &SharedState, p: &Principal, args: &Value) -> NfsResult<Value> {
    let now = unix_now();
    let daily = args["granularity"].as_str() != Some("hour");
    let to = args["to"].as_i64().unwrap_or(now + 1);
    let from = args["from"].as_i64().unwrap_or(to - 30 * 86400);
    if from >= to || to - from > 400 * 86400 {
        return Err(invalid("from/to must span 1 second to 400 days"));
    }
    let ids: Vec<String> = match args["share_id"].as_str() {
        Some(id) => vec![load_share(state, p, id)?.share_id],
        None => {
            let mut ids = Vec::new();
            let mut offset = 0;
            loop {
                let page = state.db.share_list(&ListFilter {
                    owner: Some(p.owner.clone()),
                    include_hidden: true,
                    offset,
                    limit: 1000,
                    ..Default::default()
                })?;
                let n = page.len();
                ids.extend(page.into_iter().map(|s| s.share_id));
                if n < 1000 {
                    break;
                }
                offset += 1000;
            }
            ids
        }
    };
    let series = state.db.share_stats_series(&ids, from, to, daily)?;
    let mut totals = StatsPoint::default();
    for s in &series {
        totals.views += s.views;
        totals.downloads += s.downloads;
        totals.bytes += s.bytes;
        totals.visitors += s.visitors;
    }
    let (active, expiring) = state.db.share_count_expiring(Some(&p.owner), now, EXPIRING_WITHIN)?;
    Ok(json!({
        "granularity": if daily { "day" } else { "hour" },
        "from": from,
        "to": to,
        "series": series,
        "totals": {"views": totals.views, "downloads": totals.downloads, "bytes": totals.bytes, "visitors": totals.visitors},
        "summary": {"active": active, "expiring_soon": expiring, "expiring_within": EXPIRING_WITHIN},
        // Channels nfs-server cannot observe; the UI says so instead of guessing.
        "unavailable": ["public", "messagehub", "homestation"],
    }))
}
