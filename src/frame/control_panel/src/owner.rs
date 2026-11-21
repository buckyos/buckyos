use crate::agent_mgr::{AgentKv, ZoneKv};
use crate::{ControlPanelServer, RpcAuthPrincipal};
use buckyos_api::{
    agent_spec_key, get_buckyos_api_runtime, obj_service_url, AgentId, AgentPermissionMode,
    AgentSpec, AppInstanceId, TokenPrincipalKind,
};
use buckyos_kit::KVAction;
use kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

type Result<T> = std::result::Result<T, RPCErrors>;

fn invalid(msg: &str) -> RPCErrors {
    RPCErrors::ParseRequestError(msg.into())
}
fn denied() -> RPCErrors {
    RPCErrors::NoPermission(
        "a verified owner session with the required permission is needed".into(),
    )
}
fn fail(code: &str, message: &str, detail: Value) -> Value {
    json!({"ok":false,"error":{"code":code,"message":message,"detail":detail}})
}
fn field<'a>(p: &'a Value, key: &str) -> Result<&'a str> {
    p[key]
        .as_str()
        .filter(|s| !s.trim().is_empty() && s.len() <= 4096)
        .ok_or_else(|| invalid(key))
}
fn digest(v: &Value) -> String {
    hex::encode(Sha256::digest(v.to_string().as_bytes()))
}

async fn load(kv: &dyn AgentKv, key: &str) -> Result<Option<(Value, u64)>> {
    kv.get(key)
        .await?
        .map(|(s, rev)| {
            serde_json::from_str(&s)
                .map(|v| (v, rev))
                .map_err(|_| RPCErrors::ReasonError("corrupt owner resource state".into()))
        })
        .transpose()
}

async fn save(kv: &dyn AgentKv, key: &str, version: Option<u64>, value: &Value) -> Result<bool> {
    let action = if version.is_some() {
        KVAction::Update(value.to_string())
    } else {
        KVAction::Create(value.to_string())
    };
    match kv
        .exec_tx(
            HashMap::from([(key.to_string(), action)]),
            version.map(|v| (key.to_string(), v)),
        )
        .await
    {
        Ok(()) => Ok(true),
        Err(e) => {
            if kv.get(key).await?.map(|(_, v)| v) != version {
                Ok(false)
            } else {
                Err(e)
            }
        }
    }
}

fn slot(p: &Value) -> Result<&str> {
    match p["slot"].as_str() {
        Some(s @ ("calendar" | "todo")) => Ok(s),
        _ => Err(invalid("slot must be calendar or todo")),
    }
}
fn binding_key(owner: &str, slot: &str) -> String {
    format!("users/{owner}/resource_bindings/{slot}")
}
fn default_binding(slot: &str) -> Value {
    json!({"schema_version":1,"slot":slot,"mode":"default","binding_revision":0})
}
fn actions(slot: &str) -> &'static [&'static str] {
    if slot == "calendar" {
        &["list", "add", "update", "rm", "skip", "postpone"]
    } else {
        &["list", "add", "done", "undone", "update", "move", "rm"]
    }
}

fn provider_ref(raw: &str) -> Result<url::Url> {
    let url = url::Url::parse(raw).map_err(|_| invalid("invalid provider_ref"))?;
    if !matches!(url.scheme(), "https" | "obj")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid("provider_ref must be a credential-free object URL"));
    }
    Ok(url)
}

fn profile_error(e: name_lib::NSError) -> RPCErrors {
    match e {
        name_lib::NSError::Forbid => {
            RPCErrors::NoPermission("access to the provider profile was denied".into())
        }
        name_lib::NSError::Failed(ref msg) if msg.contains("HTTP 401") => {
            RPCErrors::InvalidToken("provider authorization expired".into())
        }
        name_lib::NSError::InvalidData | name_lib::NSError::InvalidParam(_) => {
            invalid("PROVIDER_TYPE_MISMATCH: invalid Object Profile")
        }
        _ => RPCErrors::ReasonError(
            "PROVIDER_UNAVAILABLE: unable to read the provider profile".into(),
        ),
    }
}

async fn declared_actions(
    slot: &str,
    raw: &str,
    token: Option<&str>,
) -> Result<(Vec<String>, Option<Value>)> {
    let url = provider_ref(raw)?;
    let rt = get_buckyos_api_runtime()?;
    let origin = format!(
        "http://{}:{}",
        rt.get_local_gateway_host(),
        rt.node_gateway_port
    );
    let zone = rt
        .get_zone_config()
        .ok_or_else(denied)?
        .zone_document()
        .map_err(|_| denied())?;
    let zone_host = buckyos_api::zone_document_hostname(&zone);
    let zone_https = |url: &url::Url| {
        url.scheme() == "https"
            && url
                .host_str()
                .is_some_and(|host| host.eq_ignore_ascii_case(&zone_host))
            && url.port_or_known_default() == Some(443)
    };
    let local_source = url.scheme() == "obj" || zone_https(&url);
    let object =
        buckyos_api::parse_object_url(raw).map_err(|_| invalid("unsupported provider_ref"))?;
    let endpoint =
        buckyos_api::object_endpoint(&object, &origin, None).unwrap_or_else(|| url.to_string());
    // Fetch each document separately. A Card's profile location can have a different audience.
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(12))
        .build()
        .map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
    let mut card_http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(12));
    if local_source {
        let mut headers = reqwest::header::HeaderMap::new();
        let auth = format!("Bearer {}", token.ok_or_else(denied)?)
            .parse()
            .map_err(|_| denied())?;
        headers.insert(reqwest::header::AUTHORIZATION, auth);
        card_http = card_http.default_headers(headers);
    }
    let local = name_client::DIDObjectClient::with_http_client(
        card_http
            .build()
            .map_err(|e| RPCErrors::ReasonError(e.to_string()))?,
    );
    let card = local.resolve_card(&endpoint).await.map_err(profile_error)?;

    let href = name_lib::resolve_href(
        card.service_endpoint()
            .map_err(|_| invalid("missing provider endpoint"))?,
        card.profile().map_err(|_| invalid("missing profile"))?,
    );
    let dest = url::Url::parse(&href).map_err(|_| invalid("invalid profile URL"))?;
    let same_origin = url::Url::parse(&endpoint).is_ok_and(|u| u.origin() == dest.origin());
    if !same_origin && dest.scheme() != "https" {
        return Err(invalid("alternate profile origins require HTTPS"));
    }
    let anonymous = name_client::DIDObjectClient::with_http_client(http);
    let profile_client = if same_origin || (local_source && zone_https(&dest)) {
        &local
    } else {
        &anonymous
    };
    let profile = profile_client
        .resolve_profile_from_card(&card)
        .await
        .map_err(profile_error)?;
    if !buckyos_api::owner_resource::matches_slot(&profile, slot) {
        return Err(invalid(
            "PROVIDER_TYPE_MISMATCH: expected a typed personal resource with a list action",
        ));
    }
    let upstream = match profile.extra.get("x-buckyos:resource-binding") {
        Some(v) => {
            let source_zone = field(v, "zone_did")?;
            let source_did = field(v, "owner_did")?;
            let source_slot = field(v, "slot")?;
            if name_lib::DID::from_str(source_zone).is_err()
                || name_lib::DID::from_str(source_did).is_err()
                || !matches!(source_slot, "calendar" | "todo")
            {
                return Err(invalid("invalid resource-binding declaration"));
            }
            (source_zone == zone.id.to_string())
                .then(|| json!({"owner_did":source_did,"slot":source_slot,"zone_did":source_zone}))
        }
        None => None,
    };
    let supported = actions(slot)
        .iter()
        .filter(|a| buckyos_api::owner_resource::action_effect(&profile, a).is_some())
        .map(|s| s.to_string())
        .collect();
    Ok((supported, upstream))
}

async fn commit_binding(
    kv: &dyn AgentKv,
    key: &str,
    expected_version: Option<u64>,
    next: &Value,
    owner_did: &str,
    extra: HashMap<String, KVAction>,
) -> Result<bool> {
    const GRAPH: &str = "services/control_panel/owner_records/binding_graph";
    let mut retries = 0;
    loop {
        let graph = load(kv, GRAPH).await?;
        let (mut edges, revision) = graph
            .map(|(v, r)| (v, Some(r)))
            .unwrap_or_else(|| (json!({}), None));
        if kv.get(key).await?.map(|(_, v)| v) != expected_version {
            return Ok(false);
        }
        let node = digest(&json!([owner_did, slot(next)?]));
        let map = edges
            .as_object_mut()
            .ok_or_else(|| invalid("invalid binding graph"))?;
        map.remove(&node);
        if let Some(upstream) = next.get("upstream") {
            let dest = digest(&json!([field(upstream, "owner_did")?, slot(upstream)?]));
            map.insert(node.clone(), json!(dest));
            let mut seen = std::collections::HashSet::new();
            let mut cursor = node.as_str();
            while let Some(dest) = map.get(cursor).and_then(Value::as_str) {
                if !seen.insert(cursor) {
                    return Err(invalid(
                        "RESOURCE_BINDING_CYCLE: binding would create a personal resource loop",
                    ));
                }
                cursor = dest;
            }
        }
        let mut tx = extra.clone();
        let change = |v: &Value, exists: bool| {
            if exists {
                KVAction::Update(v.to_string())
            } else {
                KVAction::Create(v.to_string())
            }
        };
        tx.insert(key.to_owned(), change(next, expected_version.is_some()));
        tx.insert(GRAPH.to_owned(), change(&edges, revision.is_some()));
        match kv
            .exec_tx(tx, revision.map(|r| (GRAPH.to_owned(), r)))
            .await
        {
            Ok(()) => return Ok(true),
            Err(e) => {
                if kv.get(GRAPH).await?.map(|(_, v)| v) == revision || retries >= 16 {
                    return Err(e);
                }
                retries += 1;
            }
        }
    }
}

async fn binding(
    kv: &dyn AgentKv,
    owner: &str,
    subject: &str,
    owner_did: &str,
    method: &str,
    p: &Value,
    token: Option<&str>,
) -> Result<Value> {
    let s = slot(p)?;
    let key = binding_key(owner, s);
    let current = load(kv, &key).await?;
    let (old, version) = current
        .map(|(v, r)| (v, Some(r)))
        .unwrap_or_else(|| (default_binding(s), None));
    if matches!(method, "owner.binding.get" | "owner.resolve") {
        let mut v = old;
        if v["mode"] == "default" {
            v["actions"] = json!(actions(s));
        }
        if method == "owner.resolve" {
            return Ok(json!({
                "slot": s,
                "mode": v["mode"],
                "binding_revision": v["binding_revision"],
                "actions": v.get("actions").cloned().unwrap_or_else(|| json!([])),
            }));
        }
        return Ok(v);
    }
    let expected = p["if_revision"]
        .as_u64()
        .ok_or_else(|| invalid("if_revision required"))?;
    let conflict = |v: &Value| {
        fail(
            "BINDING_REVISION_CONFLICT",
            "the binding changed",
            json!({"binding_revision":v["binding_revision"]}),
        )
    };
    if old["binding_revision"] != expected {
        return Ok(conflict(&old));
    }
    let mode = if method == "owner.binding.reset" {
        "default"
    } else {
        field(p, "mode")?
    };
    let mut next = json!({"schema_version":1,"slot":s,"binding_revision":expected.checked_add(1).ok_or_else(||invalid("revision overflow"))?,"mode":mode});
    let mut extra = HashMap::new();
    match mode {
        "object" => {
            let raw = field(p, "provider_ref")?;
            let (declared, upstream) = declared_actions(s, raw, token).await?;
            next["actions"] = json!(declared);
            next["provider_ref"] = json!(raw);
            if let Some(upstream) = upstream {
                next["upstream"] = upstream;
            }
            let marker = digest(&json!([subject, s, next["binding_revision"]]));
            next["record_binding"] = json!(marker);
            extra.insert(
                history_key(subject, s, &marker),
                KVAction::Create(next.to_string()),
            );
        }
        "default" | "disabled" if !p.get("provider_ref").is_some_and(|r| !r.is_null()) => {
            if mode == "default" {
                let revision = next["binding_revision"].to_string();
                extra.insert(
                    history_key(subject, s, &revision),
                    KVAction::Create(next.to_string()),
                );
            }
        }
        _ => return Err(invalid("invalid binding mode or provider_ref")),
    }
    if commit_binding(kv, &key, version, &next, owner_did, extra).await? {
        Ok(next)
    } else {
        Ok(conflict(
            &load(kv, &key)
                .await?
                .map(|(v, _)| v)
                .unwrap_or_else(|| default_binding(s)),
        ))
    }
}

fn native_revision(raw: &str, slot: &str) -> Option<u64> {
    let u = url::Url::parse(raw).ok()?;
    let path: Vec<_> = u.path_segments()?.collect();
    if u.scheme() != "obj"
        || u.host_str() != Some("aiworkspace")
        || u.fragment().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || !matches!(path.as_slice(), [ws, "entities", table] if !ws.is_empty() && *table == slot)
    {
        return None;
    }
    let mut revisions = u.query_pairs().filter(|(key, _)| key == "binding_revision");
    let revision = revisions.next()?.1.parse().ok()?;
    revisions.next().is_none().then_some(revision)
}
fn history_key(subject: &str, slot: &str, marker: &str) -> String {
    format!(
        "services/control_panel/owner_records/bindings/{}",
        digest(&json!([subject, slot, marker]))
    )
}

async fn record_target(
    kv: &dyn AgentKv,
    subject: &str,
    slot: &str,
    reference: &str,
) -> Result<Option<(Value, String)>> {
    if let Some((raw, marker)) = buckyos_api::owner_record_ref::split_owner_record_ref(reference) {
        return Ok(load(kv, &history_key(subject, slot, marker))
            .await?
            .map(|(v, _)| (v, raw.to_owned())));
    }
    if let Some(revision) = native_revision(reference, slot) {
        let native = if revision == 0 {
            Some(default_binding(slot))
        } else {
            load(kv, &history_key(subject, slot, &revision.to_string()))
                .await?
                .map(|(v, _)| v)
                .filter(|v| v["mode"] == "default")
        };
        return Ok(native.map(|v| (v, reference.to_owned())));
    }
    Ok(None)
}

fn same_resource(a: &Value, b: &Value) -> bool {
    a["mode"] == b["mode"] && a["provider_ref"] == b["provider_ref"]
}

async fn prepare(
    kv: &dyn AgentKv,
    owner: &str,
    subject: &str,
    caller: &str,
    method: &str,
    p: &Value,
) -> Result<Value> {
    if !matches!(method, "owner.prepare" | "owner.pin") {
        return Err(invalid("unknown resource operation"));
    }
    let s = slot(p)?;
    let action = field(p, "action")?;
    if !actions(s).contains(&action) {
        return Err(invalid("unsupported action"));
    }
    let business_key = if action == "list" {
        ""
    } else {
        field(p, "idempotency_key")?
    };
    let key = format!(
        "services/control_panel/owner_operations/{}",
        digest(&json!([subject, caller, s, action, business_key]))
    );
    let hash = field(p, "input_digest")?;
    let old = if action == "list" {
        None
    } else {
        load(kv, &key).await?
    };
    if let Some((mut op, r)) = old {
        if op["input_digest"] != hash {
            return Ok(fail(
                "IDEMPOTENCY_CONFLICT",
                "key already identifies a different request",
                Value::Null,
            ));
        }
        match method {
            "owner.pin" if op["workspace_id"].is_null() => {
                if op["mode"] != "default" {
                    return Err(invalid("cannot pin an external claim"));
                }
                op["workspace_id"] = json!(field(p, "workspace_id")?);
                op["epoch"] = json!(field(p, "epoch")?);
                if !save(kv, &key, Some(r), &op).await? {
                    op = load(kv, &key)
                        .await?
                        .ok_or_else(|| invalid("missing claim"))?
                        .0;
                }
            }
            _ => {}
        }
        op["replay"] = json!(
            method == "owner.prepare" && (op["mode"] == "object" || !op["workspace_id"].is_null())
        );
        op.as_object_mut().unwrap().remove("receipt");
        return Ok(op);
    }
    if method != "owner.prepare" {
        return Err(invalid("missing original claim"));
    }
    let mut selected = None;
    let mut unwrapped = serde_json::Map::new();
    for name in ["record_ref", "record_refs", "before", "after"] {
        let Some(value) = p.get(name) else { continue };
        let anchors = matches!(name, "after" | "before");
        if anchors && selected.is_none() {
            selected = Some(
                load(kv, &binding_key(owner, s))
                    .await?
                    .map(|(v, _)| v)
                    .unwrap_or_else(|| default_binding(s)),
            );
        }
        let refs = if name == "record_refs" {
            if p.get("record_ref").is_some() {
                return Err(invalid("choose record_ref or record_refs"));
            }
            value
                .as_array()
                .filter(|a| !a.is_empty() && a.len() <= 1000)
                .ok_or_else(|| invalid("invalid record_refs"))?
                .as_slice()
        } else {
            std::slice::from_ref(value)
        };
        let mut original = Vec::with_capacity(refs.len());
        for r in refs {
            let Some((target, raw)) = record_target(
                kv,
                subject,
                s,
                r.as_str()
                    .ok_or_else(|| invalid("invalid record reference"))?,
            )
            .await?
            else {
                return Ok(fail(
                    "UNKNOWN_RECORD_REF",
                    "the original resource is unavailable",
                    Value::Null,
                ));
            };
            if selected
                .as_ref()
                .is_some_and(|v| !same_resource(v, &target))
            {
                return Err(invalid("all references must belong to one resource"));
            }
            if selected.is_none() {
                selected = Some(target);
            }
            original.push(Value::String(raw));
        }
        unwrapped.insert(
            name.to_string(),
            if name == "record_refs" {
                Value::Array(original)
            } else {
                original.remove(0)
            },
        );
    }
    let mut op = match selected {
        Some(v) => v,
        None => load(kv, &binding_key(owner, s))
            .await?
            .map(|(v, _)| v)
            .unwrap_or_else(|| default_binding(s)),
    };
    if !unwrapped.is_empty() && op["mode"] == "object" {
        op["provider_params"] = Value::Object(unwrapped);
    }
    if op["mode"] == "disabled" {
        return Ok(fail(
            "RESOURCE_DISABLED",
            "this personal resource is disabled",
            Value::Null,
        ));
    }
    if action == "list" {
        return Ok(op);
    }
    op["record_id"] = json!(uuid::Uuid::new_v4().simple().to_string());
    op["idempotency_key"] = json!(business_key);
    op["input_digest"] = json!(hash);
    op["alias_revision"] = p["alias_revision"].clone();
    op["object"] = p["object"].clone();
    if !save(kv, &key, None, &op).await? {
        let (mut prior, _) = load(kv, &key)
            .await?
            .ok_or_else(|| invalid("missing concurrent claim"))?;
        if prior["input_digest"] != hash {
            return Ok(fail(
                "IDEMPOTENCY_CONFLICT",
                "key already identifies a different request",
                Value::Null,
            ));
        }
        prior["replay"] = json!(prior["mode"] == "object" || !prior["workspace_id"].is_null());
        prior.as_object_mut().unwrap().remove("receipt");
        return Ok(prior);
    }
    op["replay"] = json!(false);
    Ok(op)
}

fn manager(p: &RpcAuthPrincipal) -> bool {
    p.is_user_session
        && (p.is_control_panel_session
            || p.authenticated_app_id == format!("app:buckycli@{}", p.owner_user_id))
}

fn alias_name(s: &str) -> bool {
    s.len() <= 128
        && s.split('.').all(|part| {
            part.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
}

async fn aliases(kv: &dyn AgentKv, owner: &str, method: &str, p: &Value) -> Result<Value> {
    let prefix = format!("users/{owner}/object_aliases");
    if method == "owner.alias.list" {
        let mut out = Vec::new();
        for name in kv.list(&prefix).await? {
            if let Some((v, _)) = load(kv, &format!("{prefix}/{name}")).await? {
                out.push(v)
            }
        }
        return Ok(json!({"aliases":out}));
    }
    let name = field(p, "name")?;
    if !alias_name(name) {
        return Err(invalid("invalid alias name"));
    }
    let key = format!("{prefix}/{name}");
    let old = load(kv, &key).await?;
    let rev = old
        .as_ref()
        .and_then(|(v, _)| v["alias_revision"].as_u64())
        .unwrap_or(0);
    let conflict = |r| {
        fail(
            "ALIAS_REVISION_CONFLICT",
            "the alias changed",
            json!({"alias_revision":r}),
        )
    };
    if p["if_revision"].as_u64() != Some(rev) {
        return Ok(conflict(rev));
    }
    let kind = if method == "owner.alias.rm" {
        "default"
    } else {
        field(p, "kind")?
    };
    let mut v = json!({"name":name,"kind":kind,"alias_revision":rev.checked_add(1).ok_or_else(||invalid("revision overflow"))?});
    match kind {
        "link" => {
            let raw = field(p, "url")?;
            let url = url::Url::parse(raw).map_err(|_| invalid("invalid object URL"))?;
            if !matches!(url.scheme(), "obj" | "https" | "cyfs")
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(invalid("invalid object URL"));
            }
            v["url"] = json!(raw);
        }
        "local" => {
            let id = field(p, "object")?;
            if id.split('/').count() != 2
                || id.split('/').any(|s| {
                    s.is_empty()
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                })
            {
                return Err(invalid("invalid object id"));
            }
            v["object"] = json!(id);
        }
        "disabled" | "default" => {}
        _ => return Err(invalid("invalid alias kind")),
    }
    if save(kv, &key, old.map(|(_, r)| r), &v).await? {
        Ok(v)
    } else {
        Ok(conflict(
            load(kv, &key)
                .await?
                .and_then(|(v, _)| v["alias_revision"].as_u64())
                .unwrap_or(0),
        ))
    }
}

async fn account_did(kv: &dyn AgentKv, owner: &str) -> Result<Option<String>> {
    let key = format!("users/{owner}/doc");
    kv.get(&key)
        .await?
        .map(|(raw, _)| buckyos_api::user_did_from_doc_value(&key, &raw).map(|d| d.to_string()))
        .transpose()
}

async fn eligible_account(kv: &dyn AgentKv, owner: &str) -> Result<bool> {
    let Some((value, _)) = load(kv, &format!("users/{owner}/settings")).await? else {
        return Ok(false);
    };
    let settings: buckyos_api::UserSettings =
        serde_json::from_value(value).map_err(|_| denied())?;
    Ok(settings.is_active() && settings.user_type != buckyos_api::UserType::Limited)
}

async fn sharing(
    kv: &dyn AgentKv,
    principal: &RpcAuthPrincipal,
    method: &str,
    p: &Value,
) -> Result<Value> {
    let writing = matches!(method, "owner.sharing.grant" | "owner.sharing.revoke");
    let (subject, to) = if writing {
        if field(p, "slot")? != "availability" {
            return Err(invalid("sharing supports availability"));
        }
        (principal.owner_did.as_str(), field(p, "to")?)
    } else {
        (field(p, "did")?, principal.owner_did.as_str())
    };
    let key = format!(
        "services/control_panel/owner_shares/{}",
        digest(&json!([subject, to]))
    );
    let old = load(kv, &key).await?;
    if writing {
        if !manager(principal) {
            return Err(denied());
        }
        if method == "owner.sharing.grant" {
            let mut local = false;
            for user in kv.list("users").await? {
                if account_did(kv, &user).await?.as_deref() == Some(to) {
                    local = eligible_account(kv, &user).await?;
                    break;
                }
            }
            if !local {
                return Err(invalid("recipient must be a local user DID"));
            }
        }
        let enabled = method == "owner.sharing.grant";
        let next = json!({"owner":principal.owner_user_id,"enabled":enabled});
        if !save(kv, &key, old.map(|(_, r)| r), &next).await? {
            return Ok(fail(
                "SHARING_CONFLICT",
                "sharing changed concurrently",
                Value::Null,
            ));
        }
        return Ok(json!({"granted":enabled,"slot":"availability","to":to}));
    }
    let visible = old.map(|(v, _)| v).filter(|v| v["enabled"] == true);
    let owner = if subject == to {
        Some(principal.owner_user_id.as_str())
    } else {
        visible.as_ref().and_then(|v| v["owner"].as_str())
    };
    let Some(owner) = owner else {
        return if method == "owner.sharing.discover" {
            Ok(json!({}))
        } else {
            Err(denied())
        };
    };
    if account_did(kv, owner).await?.as_deref() != Some(subject) {
        return Err(denied());
    }
    if !eligible_account(kv, owner).await? {
        return Err(denied());
    }
    let mut out = json!({"availability_ref":obj_service_url("aiworkspace",&["people",subject,"availability"])});
    if method == "owner.sharing.check" {
        let binding = load(kv, &binding_key(owner, "calendar"))
            .await?
            .map(|(v, _)| v)
            .unwrap_or_else(|| default_binding("calendar"));
        out["mode"] = binding["mode"].clone();
        out["binding_revision"] = binding["binding_revision"].clone();
        out["owner_user_id"] = json!(owner);
    }
    Ok(out)
}

async fn identity(kv: &dyn AgentKv, p: &RpcAuthPrincipal) -> Result<String> {
    if p.session.auth_method == Some(buckyos_api::AuthMethod::AppAccessToken)
        || p.owner_did.is_empty()
        || p.is_restricted()
        || p.user_type == buckyos_api::UserType::Limited
    {
        return Err(denied());
    }
    match p.principal_kind {
        TokenPrincipalKind::User => Ok(p.owner_did.clone()),
        TokenPrincipalKind::Agent => Ok(p.username.clone()),
        TokenPrincipalKind::App => {
            let instance: AppInstanceId = p
                .authenticated_app_id
                .strip_prefix("app:")
                .ok_or_else(denied)?
                .parse()
                .map_err(|_| denied())?;
            if instance.owner_user_id() != p.owner_user_id || p.agent_permission_mode.is_none() {
                return Err(denied());
            }
            let id = AgentId::parse(instance.app_id().as_str()).map_err(|_| denied())?;
            let v = load(kv, &agent_spec_key(&p.owner_user_id, &id))
                .await?
                .ok_or_else(denied)?
                .0;
            let spec: AgentSpec = serde_json::from_value(v).map_err(|_| denied())?;
            if spec.validate().is_err()
                || spec.agent_id != id
                || !spec.binding.references_runtime(&instance)
            {
                return Err(denied());
            }
            Ok(spec.agent_did.to_string())
        }
        _ => Err(denied()),
    }
}

impl ControlPanelServer {
    pub(crate) async fn handle_owner(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse> {
        let p = Self::require_rpc_principal(principal)?;
        let kv = ZoneKv::connect().await?;
        let did = identity(&kv, p).await?;
        let method = req.method.as_str();
        let owner = &p.owner_user_id;
        let input = &req.params;
        let zone = get_buckyos_api_runtime()?
            .get_zone_config()
            .ok_or_else(denied)?
            .zone_document()
            .map_err(|_| denied())?;
        let read_only = p.agent_permission_mode == Some(AgentPermissionMode::ReadOnly);
        let result = match method {
            "owner.context" => {
                let mut out = json!({"owner_user_id":owner,"user_id":owner,"owner_did":p.owner_did,"caller_did":did,"caller_app":p.authenticated_app_id,"read_only":read_only,"zone_did":zone.id.to_string()});
                if let Some(alias) = input.get("alias").and_then(Value::as_str) {
                    if !alias_name(alias) {
                        return Err(invalid("invalid alias"));
                    }
                    out["object_alias"] =
                        load(&kv, &format!("users/{owner}/object_aliases/{alias}"))
                            .await?
                            .map(|(v, _)| v)
                            .unwrap_or(Value::Null);
                }
                out
            }
            "owner.alias.list" | "owner.alias.set" | "owner.alias.rm" => {
                if !manager(p) {
                    return Err(denied());
                }
                aliases(&kv, owner, method, input).await?
            }
            "owner.resolve" | "owner.binding.get" | "owner.binding.set" | "owner.binding.reset" => {
                if method != "owner.resolve" && !manager(p) {
                    return Err(denied());
                }
                binding(
                    &kv,
                    owner,
                    &json!([zone.id.to_string(), p.owner_did]).to_string(),
                    &p.owner_did.to_string(),
                    method,
                    input,
                    req.token.as_deref(),
                )
                .await?
            }
            "owner.prepare" | "owner.pin" => {
                if read_only && (method != "owner.prepare" || input["action"] != "list") {
                    return Err(denied());
                }
                let caller =
                    json!([p.principal_kind.as_str(), did, p.authenticated_app_id]).to_string();
                prepare(
                    &kv,
                    owner,
                    &json!([zone.id.to_string(), p.owner_did]).to_string(),
                    &caller,
                    method,
                    input,
                )
                .await?
            }
            "owner.sharing.grant"
            | "owner.sharing.revoke"
            | "owner.sharing.discover"
            | "owner.sharing.check" => sharing(&kv, p, method, input).await?,
            _ => return Err(invalid("unknown owner method")),
        };
        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }
}

#[cfg(test)]
#[path = "owner_access_tests.rs"]
mod access_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_mgr::tests::MemoryKv;

    #[tokio::test]
    async fn resource_revision_and_operation_identity_survive_rebinding() {
        let kv = MemoryKv::default();
        let p = json!({"slot":"calendar","if_revision":0,"mode":"disabled"});
        let input = json!({"slot":"calendar","action":"add","input_digest":"review","idempotency_key":"friday","alias_revision":"a"});
        let first = prepare(
            &kv,
            "alice",
            "zone/alice",
            "jarvis",
            "owner.prepare",
            &input,
        )
        .await
        .unwrap();
        let mut pinned = input.clone();
        pinned["workspace_id"] = json!("ws-a");
        pinned["epoch"] = json!("e");
        let pinned = prepare(&kv, "alice", "zone/alice", "jarvis", "owner.pin", &pinned)
            .await
            .unwrap();
        assert_eq!(pinned["workspace_id"], "ws-a");
        let (a, b) = tokio::join!(
            binding(
                &kv,
                "alice",
                "zone/alice",
                "did:bns:alice",
                "owner.binding.set",
                &p,
                None
            ),
            binding(
                &kv,
                "alice",
                "zone/alice",
                "did:bns:alice",
                "owner.binding.set",
                &p,
                None
            )
        );
        let replies = [a.unwrap(), b.unwrap()];
        assert_eq!(
            replies
                .iter()
                .filter(|r| r["binding_revision"] == 1)
                .count(),
            1
        );
        assert!(replies
            .iter()
            .any(|r| r["error"]["code"] == "BINDING_REVISION_CONFLICT"));
        let replay = prepare(
            &kv,
            "alice",
            "zone/alice",
            "jarvis",
            "owner.prepare",
            &input,
        )
        .await
        .unwrap();
        assert_eq!(replay["record_id"], first["record_id"]);
        assert_eq!(replay["workspace_id"], "ws-a");
        assert_eq!(replay["binding_revision"], 0);
        assert_eq!(replay["replay"], true);
        let mut conflict = input.clone();
        conflict["input_digest"] = json!("a different meeting");
        assert_eq!(
            prepare(
                &kv,
                "alice",
                "zone/alice",
                "jarvis",
                "owner.prepare",
                &conflict
            )
            .await
            .unwrap()["error"]["code"],
            "IDEMPOTENCY_CONFLICT"
        );
        assert_eq!(
            prepare(
                &kv,
                "alice",
                "zone/alice",
                "second-agent",
                "owner.prepare",
                &input
            )
            .await
            .unwrap()["error"]["code"],
            "RESOURCE_DISABLED"
        );
        let foreign = prepare(&kv, "bob", "zone/bob", "jarvis", "owner.prepare", &input)
            .await
            .unwrap();
        assert_ne!(foreign["record_id"], first["record_id"]);
        let reset = binding(
            &kv,
            "alice",
            "zone/alice",
            "did:bns:alice",
            "owner.binding.reset",
            &json!({"slot":"calendar","if_revision":1}),
            None,
        )
        .await
        .unwrap();
        assert_eq!(reset["binding_revision"], 2);
        assert_eq!(reset["mode"], "default");
        for (revision, expected) in [(0, Some(0)), (1, None), (2, Some(2)), (3, None)] {
            let reference = format!(
                "obj://aiworkspace/ws_2611abcd/entities/calendar?binding_revision={revision}"
            );
            let target = record_target(&kv, "zone/alice", "calendar", &reference)
                .await
                .unwrap();
            assert_eq!(
                target.and_then(|(v, _)| v["binding_revision"].as_u64()),
                expected
            );
            assert!(record_target(&kv, "zone/alice", "todo", &reference)
                .await
                .unwrap()
                .is_none());
        }
    }

    #[tokio::test]
    async fn the_same_provider_record_url_stays_bound_to_its_original_resource() {
        let kv = MemoryKv::default();
        let subject = "zone/alice";
        let raw = "https://vendor.invalid/records/7?x=1#local-view";
        let mut refs = Vec::new();
        for (rev, provider) in [
            (1, "https://one.invalid/todos"),
            (2, "https://two.invalid/todos"),
        ] {
            let marker = digest(&json!([subject, "todo", rev]));
            let target = json!({"mode":"object", "slot":"todo", "binding_revision":rev, "provider_ref":provider, "record_binding":marker});
            let key = binding_key("alice", "todo");
            let old = kv.get(&key).await.unwrap().map(|(_, r)| r);
            let extra = HashMap::from([(
                history_key(subject, "todo", &marker),
                KVAction::Create(target.to_string()),
            )]);
            assert!(
                commit_binding(&kv, &key, old, &target, "did:bns:alice", extra)
                    .await
                    .unwrap()
            );
            refs.push(
                buckyos_api::owner_record_ref::scoped_owner_record_ref(raw, &marker).unwrap(),
            );
        }
        assert_ne!(refs[0], refs[1]);
        let update = |r: &str, key: &str| json!({"slot":"todo", "action":"done", "record_ref":r, "input_digest":"h", "idempotency_key":key});
        for (i, provider) in ["https://one.invalid/todos", "https://two.invalid/todos"]
            .iter()
            .enumerate()
        {
            let dest = prepare(
                &kv,
                "alice",
                subject,
                "second-agent",
                "owner.prepare",
                &update(&refs[i], provider),
            )
            .await
            .unwrap();
            assert_eq!(dest["provider_ref"], *provider);
            assert_eq!(dest["provider_params"]["record_ref"], raw);
        }
        let bad = prepare(
            &kv,
            "bob",
            "zone/bob",
            "bob",
            "owner.prepare",
            &update(&refs[0], "x"),
        )
        .await
        .unwrap();
        assert_eq!(bad["error"]["code"], "UNKNOWN_RECORD_REF");
        let list = json!({"slot":"todo","action":"list","input_digest":"h","provider_ref":"https://fake.invalid/","issued":[raw]});
        assert!(prepare(
            &kv,
            "alice",
            subject,
            "reader",
            "owner.remember_list",
            &list
        )
        .await
        .is_err());
        assert_eq!(
            prepare(&kv, "alice", subject, "reader", "owner.prepare", &list)
                .await
                .unwrap()["binding_revision"],
            2
        );
        assert_eq!(
            prepare(
                &kv,
                "alice",
                subject,
                "writer",
                "owner.prepare",
                &update(raw, "forged")
            )
            .await
            .unwrap()["error"]["code"],
            "UNKNOWN_RECORD_REF"
        );
        let mixed = json!({"slot":"todo","action":"rm","record_refs":refs,"input_digest":"h","idempotency_key":"mixed"});
        assert!(
            prepare(&kv, "alice", subject, "writer", "owner.prepare", &mixed)
                .await
                .is_err()
        );
    }
}
