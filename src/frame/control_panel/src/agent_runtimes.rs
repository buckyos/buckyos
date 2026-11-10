//! Execution runtimes an owner gives their Agents (runtime lifecycle TODO
//! §10). Three things, each kept in one place:
//!
//! - a runtime: a node that opened itself to the owner's Agents
//!   (`nodes/<node>/settings.executor.owner_agents`) or a device joined by
//!   daemon URL (`executor.device.*`). What it allows — directories,
//!   operations, ComputerUse, declared baseline, term — is its own
//!   configuration, changed here for a node (`node.owner_agents_set`) and
//!   fixed when a device is issued;
//! - an assignment: `AgentSettings.runtimes[id] = {enabled, target}` plus
//!   `default_runtime_id` (`agent.update`). A device is used by one Agent at
//!   a time; a node gives every Agent its own profile;
//! - the owner's directory of both with who uses what
//!   (`agent.runtime_catalog`).
//!
//! An Agent whose default runtime is the owner's `host` runs its Agent
//! Service natively there (native-agent-service TODO): on a Linux node that
//! would be the node's service user, so there the container is the default
//! where one exists.

use std::collections::{BTreeMap, HashMap};

use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    agent_settings_key, is_valid_runtime_id, node_settings_key, AgentId, AgentRuntimeSettings, AgentRuntimeTarget,
    AgentSettings, AppInstanceId, ComputerAccess, NodeSettings, ProductProfile, RuntimeBaseline, TokenPrincipalKind,
    CONTAINER_RUNTIME_ID, HOST_RUNTIME_ID, PRODUCT_PROFILE_KEY, ZONE_OWNER_USER_ID_KEY,
};
use buckyos_kit::KVAction;
use name_lib::DeviceInfo;
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent_mgr::{get_value, reason, AgentKv, ZoneKv};
use crate::{ControlPanelServer, RpcAuthPrincipal};

/// Runtime id of a device joined by daemon URL: `exec-<executor_id>`.
pub(crate) const DEVICE_RUNTIME_PREFIX: &str = "exec-";

pub(crate) fn device_runtime_id(executor_id: &str) -> String {
    format!("{DEVICE_RUNTIME_PREFIX}{executor_id}")
}

/// One runtime of an owner's directory: a node the owner's Agents may run
/// on, or a device joined by daemon URL.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NodeRuntimeOffer {
    pub runtime_id: String,
    /// Node runtimes; empty for a device.
    pub node_id: String,
    pub node_did: String,
    pub device_type: String,
    /// Device runtimes.
    pub executor_id: Option<String>,
    pub name: String,
    /// The ComputerUse the runtime allows.
    pub computer: ComputerAccess,
    pub workdir_roots: Vec<String>,
    /// No write operation is allowed.
    pub read_only: bool,
    /// The environment the runtime is declared to provide.
    pub baseline: RuntimeBaseline,
    pub expires_at_ms: Option<u64>,
    /// A node that executes as a chosen OS account.
    pub run_as: bool,
    /// A node whose administrator closed the policy, or a device that is
    /// revoked or expired.
    pub closed: bool,
    /// `linux | windows | apple` as the device reports it (unknown for a
    /// device joined by URL).
    pub os: String,
}

impl NodeRuntimeOffer {
    pub(crate) fn is_device(&self) -> bool {
        self.executor_id.is_some()
    }

    fn target(&self) -> AgentRuntimeTarget {
        match &self.executor_id {
            Some(executor_id) => AgentRuntimeTarget::Executor { executor_id: executor_id.clone(), profile_id: None },
            None => AgentRuntimeTarget::Node { node_id: self.node_id.clone(), node_did: self.node_did.clone() },
        }
    }

    fn is_target(&self, target: &AgentRuntimeTarget) -> bool {
        match (target, &self.executor_id) {
            (AgentRuntimeTarget::Executor { executor_id, .. }, Some(own)) => executor_id == own,
            (AgentRuntimeTarget::Node { node_id, .. }, None) => node_id == &self.node_id,
            _ => false,
        }
    }
}

async fn zone_owner(kv: &dyn AgentKv) -> Option<String> {
    get_value::<String>(kv, ZONE_OWNER_USER_ID_KEY).await
}

fn node_runtime_id(index: usize, node_id: &str) -> String {
    if index == 0 {
        return HOST_RUNTIME_ID.to_string();
    }
    let mut id = String::from("node-");
    for ch in node_id.chars() {
        let c = ch.to_ascii_lowercase();
        id.push(if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' { c } else { '-' });
    }
    id.truncate(64);
    id
}

/// The runtimes `owner` may assign to their Agents: for the zone owner the
/// nodes whose `owner_agents` policy exists (desktop nodes first, so the
/// owner's desktop is `host`; other users have no node runtimes), then the
/// devices the owner issued a daemon URL for.
pub(crate) async fn runtime_directory(kv: &dyn AgentKv, owner: &str) -> Result<Vec<NodeRuntimeOffer>, RPCErrors> {
    let mut offers = Vec::new();
    if zone_owner(kv).await.as_deref() == Some(owner) {
        for node_id in kv.list("nodes").await? {
            let Some(settings) = get_value::<NodeSettings>(kv, &node_settings_key(&node_id)).await else {
                continue;
            };
            let Some(executor) = settings.executor.as_ref() else {
                continue;
            };
            let Some(policy) = executor.owner_agents.as_ref() else {
                continue;
            };
            let Some(info) = get_value::<DeviceInfo>(kv, &format!("devices/{node_id}/info")).await else {
                continue;
            };
            offers.push(NodeRuntimeOffer {
                runtime_id: String::new(),
                node_id: node_id.clone(),
                node_did: info.device_doc.id.to_string(),
                device_type: info.device_doc.device_type.clone(),
                executor_id: None,
                name: info.device_doc.name.clone(),
                computer: policy.computer,
                workdir_roots: policy.workdir_roots.clone(),
                read_only: !policy.effective_ops().iter().any(|op| op.is_mutating() && !op.is_computer()),
                baseline: policy.baseline.unwrap_or(RuntimeBaseline::Standard),
                expires_at_ms: None,
                run_as: policy.run_as.is_some(),
                closed: !executor.enabled || !policy.enabled,
                os: info.os.clone(),
            });
        }
        offers.sort_by(|a, b| {
            let rank = |o: &NodeRuntimeOffer| (o.device_type != "desktop", o.closed, o.node_id.clone());
            rank(a).cmp(&rank(b))
        });
        for (index, offer) in offers.iter_mut().enumerate() {
            offer.runtime_id = node_runtime_id(index, &offer.node_id);
        }
    }
    for (device, active) in crate::agent_daemon::owner_devices(kv, owner).await? {
        // A device is one sandbox: its first profile is what it allows.
        let Some(profile) = device.profiles.values().next() else {
            continue;
        };
        offers.push(NodeRuntimeOffer {
            runtime_id: device_runtime_id(&device.executor_id),
            node_id: String::new(),
            node_did: String::new(),
            device_type: String::new(),
            executor_id: Some(device.executor_id.clone()),
            name: device.name.clone(),
            computer: profile.computer(),
            workdir_roots: profile.workdir_roots.clone(),
            read_only: !profile.ops.iter().any(|op| op.is_mutating() && !op.is_computer()),
            baseline: profile.baseline.unwrap_or(RuntimeBaseline::Standard),
            expires_at_ms: device.expires_at_ms,
            run_as: false,
            closed: !active,
            os: String::new(),
        });
    }
    Ok(offers)
}

/// The runtime settings of every Agent of `owner`.
pub(crate) async fn assignments(kv: &dyn AgentKv, owner: &str) -> Vec<(AgentId, AgentSettings)> {
    let mut out = Vec::new();
    for entry in kv.list(&format!("users/{owner}/agents")).await.unwrap_or_default() {
        let Ok(agent_id) = AgentId::parse(&entry) else {
            continue;
        };
        if let Some(settings) = get_value::<AgentSettings>(kv, &agent_settings_key(owner, &agent_id)).await {
            out.push((agent_id, settings));
        }
    }
    out
}

/// The Agent that has the device `executor_id` switched on (a device is
/// used by one Agent at a time).
pub(crate) fn device_user<'a>(assigned: &'a [(AgentId, AgentSettings)], executor_id: &str) -> Option<&'a AgentId> {
    assigned.iter().find_map(|(agent_id, settings)| {
        settings
            .runtimes
            .values()
            .any(|r| r.enabled && matches!(&r.target, AgentRuntimeTarget::Executor { executor_id: id, .. } if id == executor_id))
            .then_some(agent_id)
    })
}

/// Whether an Agent container can run anywhere in the zone (a node that
/// takes app instances and supports containers) and the product runs
/// containers at all (BuckyOS Desktop Lite does not).
pub(crate) async fn container_available(kv: &dyn AgentKv) -> bool {
    let product = match kv.get(PRODUCT_PROFILE_KEY).await {
        Ok(Some((raw, _))) => ProductProfile::parse(&raw).ok(),
        Ok(None) => Some(ProductProfile::full()),
        Err(_) => None,
    };
    if !product.is_some_and(|product| product.container_enabled()) {
        return false;
    }
    let Ok(devices) = kv.list("devices").await else {
        return false;
    };
    for device in devices {
        if let Some(info) = get_value::<DeviceInfo>(kv, &format!("devices/{device}/info")).await {
            let doc = &info.device_doc;
            if doc.support_container && matches!(doc.device_type.as_str(), "ood" | "server") {
                return true;
            }
        }
    }
    false
}

/// Whether a new Agent runs on the owner's `host` by default: on a Windows
/// or macOS desktop yes; on a Linux node only when no container can run.
pub(crate) fn host_by_default(host: &NodeRuntimeOffer, container_available: bool) -> bool {
    !host.closed && (host.os != "linux" || !container_available)
}

/// A new Agent of the zone owner: the owner's machine (`host`) assigned and
/// on, and the owner's default where [`host_by_default`] says so. Nothing
/// when the owner's zone offers no open node. Every Agent gets `host`: the
/// node's policy is a template, each Agent has its own profile there.
pub(crate) async fn apply_create_defaults(kv: &dyn AgentKv, owner: &str, settings: &mut AgentSettings) {
    let offers = match runtime_directory(kv, owner).await {
        Ok(offers) => offers,
        Err(error) => {
            log::warn!("runtime directory unavailable; the new Agent starts in its container only: {error}");
            return;
        }
    };
    let Some(host) = offers.iter().find(|o| o.runtime_id == HOST_RUNTIME_ID && !o.closed) else {
        return;
    };
    if !host_by_default(host, container_available(kv).await) {
        return;
    }
    settings.runtimes.insert(
        HOST_RUNTIME_ID.to_string(),
        AgentRuntimeSettings { enabled: true, target: host.target() },
    );
    settings.default_runtime_id = Some(HOST_RUNTIME_ID.to_string());
}

/// `agent.update` runtime changes: `runtimes: { "<id>": {enabled?} | null }`
/// and `default_runtime_id: "<id>" | null`. A runtime is assigned only from
/// the owner's directory (the target is the directory's, never the
/// caller's) and switched on only while it is open; a device is on for one
/// Agent at a time (`agent`: the Agent being changed). `null` takes a
/// runtime away from the Agent. The default must be one of the Agent's
/// runtimes; switching it off keeps it configured.
pub(crate) async fn apply_update(
    kv: &dyn AgentKv,
    owner: &str,
    agent: Option<&AgentId>,
    settings: &mut AgentSettings,
    runtimes: Option<&Value>,
    default_runtime_id: Option<&Value>,
) -> Result<bool, RPCErrors> {
    let mut changed = false;
    if let Some(runtimes) = runtimes {
        let map = runtimes
            .as_object()
            .ok_or_else(|| RPCErrors::ParseRequestError("runtimes must be an object".into()))?;
        let offers = runtime_directory(kv, owner).await?;
        let mut others: Option<Vec<(AgentId, AgentSettings)>> = None;
        for (id, change) in map {
            if !is_valid_runtime_id(id) || id == CONTAINER_RUNTIME_ID {
                return Err(reason("invalid_runtime", format!("`{id}` is not a runtime id")));
            }
            if change.is_null() {
                if settings.runtimes.remove(id).is_some() {
                    changed = true;
                    if settings.default_runtime_id.as_deref() == Some(id) && default_runtime_id.is_none() {
                        settings.default_runtime_id = None;
                    }
                }
                continue;
            }
            let change = change.as_object().ok_or_else(|| {
                RPCErrors::ParseRequestError(format!("runtime `{id}` must be an object or null"))
            })?;
            if let Some(other) = change.keys().find(|key| key.as_str() != "enabled") {
                return Err(RPCErrors::ParseRequestError(format!(
                    "runtime `{id}`: `{other}` is not set per Agent; an assignment takes `enabled` only (what a runtime allows is its own configuration)"
                )));
            }
            let enabled = match change.get("enabled") {
                None => None,
                Some(Value::Bool(enabled)) => Some(*enabled),
                Some(_) => return Err(RPCErrors::ParseRequestError(format!("runtime `{id}` enabled must be a boolean"))),
            };
            let entry = settings.runtimes.get(id).cloned();
            let offer = match &entry {
                Some(current) => offers.iter().find(|o| o.is_target(&current.target)),
                None => offers.iter().find(|o| &o.runtime_id == id),
            };
            let entry = match entry {
                Some(current) => current,
                None => AgentRuntimeSettings {
                    enabled: false,
                    target: offer
                        .ok_or_else(|| {
                            reason("runtime_not_offered", format!("runtime `{id}` is not in this owner's runtime directory (see agent.runtime_catalog)"))
                        })?
                        .target(),
                },
            };
            let mut next = entry.clone();
            if let Some(enabled) = enabled {
                if enabled && !entry.enabled {
                    let offer = offer.filter(|o| !o.closed).ok_or_else(|| {
                        reason("runtime_not_offered", format!("runtime `{id}` is not open to this owner's Agents right now"))
                    })?;
                    if let Some(executor_id) = &offer.executor_id {
                        if others.is_none() {
                            others = Some(assignments(kv, owner).await.into_iter().filter(|(other, _)| Some(other) != agent).collect());
                        }
                        if let Some(user) = device_user(others.as_deref().unwrap_or_default(), executor_id) {
                            return Err(reason(
                                "runtime_in_use",
                                format!("device `{}` is in use by Agent `{user}`: a device is used by one Agent at a time; switch it off there first", offer.name),
                            ));
                        }
                    }
                }
                next.enabled = enabled;
            }
            if next != entry || !settings.runtimes.contains_key(id) {
                settings.runtimes.insert(id.clone(), next);
                changed = true;
            }
        }
    }
    if let Some(default) = default_runtime_id {
        let next = match default {
            Value::Null => None,
            Value::String(s) if s == CONTAINER_RUNTIME_ID || s.is_empty() => None,
            Value::String(s) => Some(s.clone()),
            _ => return Err(RPCErrors::ParseRequestError("default_runtime_id must be a string or null".into())),
        };
        if next != settings.default_runtime_id {
            settings.default_runtime_id = next;
            changed = true;
        }
    }
    settings
        .validate_runtimes()
        .map_err(|e| reason("invalid_runtimes", e))?;
    Ok(changed)
}

/// A device was revoked or deleted: it is taken away from every Agent of
/// its owner (their records of it are kept as history by the Agent).
pub(crate) async fn unassign_device(kv: &dyn AgentKv, owner: &str, executor_id: &str) -> Result<Vec<AgentId>, RPCErrors> {
    let mut changed = Vec::new();
    for (agent_id, mut settings) in assignments(kv, owner).await {
        let ids: Vec<String> = settings
            .runtimes
            .iter()
            .filter(|(_, r)| matches!(&r.target, AgentRuntimeTarget::Executor { executor_id: id, .. } if id == executor_id))
            .map(|(id, _)| id.clone())
            .collect();
        if ids.is_empty() {
            continue;
        }
        for id in &ids {
            settings.runtimes.remove(id);
            if settings.default_runtime_id.as_ref() == Some(id) {
                settings.default_runtime_id = None;
            }
        }
        let raw = serde_json::to_string(&settings).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
        kv.exec_tx(HashMap::from([(agent_settings_key(owner, &agent_id), KVAction::Update(raw))]), None).await?;
        changed.push(agent_id);
    }
    Ok(changed)
}

/// How an Agent with `settings` runs its Agent Service: natively on the
/// node of its default runtime when that is an open node runtime of the
/// owner (the scheduler's `native_agent_node`), else in its container.
pub(crate) fn agent_service_view(offers: &[NodeRuntimeOffer], settings: &AgentSettings) -> Value {
    let default = settings.effective_default_runtime();
    match settings.runtimes.get(default).map(|r| &r.target) {
        Some(AgentRuntimeTarget::Node { node_id, .. })
            if offers.iter().any(|o| !o.is_device() && &o.node_id == node_id && !o.closed) =>
        {
            json!({"mode": "native", "node_id": node_id})
        }
        _ => json!({"mode": "container"}),
    }
}

fn offer_view(o: &NodeRuntimeOffer) -> Value {
    json!({
        "runtime_id": o.runtime_id,
        "kind": if o.is_device() { "executor" } else { "node" },
        "node_id": o.node_id,
        "node_did": o.node_did,
        "device_type": o.device_type,
        "executor_id": o.executor_id,
        "name": o.name,
        "computer": o.computer,
        "workdir_roots": o.workdir_roots,
        "read_only": o.read_only,
        "baseline": o.baseline,
        "expires_at_ms": o.expires_at_ms,
        "run_as": o.run_as,
        "closed": o.closed,
        "os": o.os,
    })
}

/// The directory as the owner sees it for one Agent (`others`: the owner's
/// other Agents, for the device another one is using).
pub(crate) fn directory_view(
    offers: &[NodeRuntimeOffer],
    settings: Option<&AgentSettings>,
    container_available: bool,
    others: &[(AgentId, AgentSettings)],
) -> Value {
    let mut out: Vec<Value> = offers
        .iter()
        .map(|o| {
            let chosen = settings.and_then(|s| s.runtimes.iter().find(|(_, r)| o.is_target(&r.target)));
            let mut view = offer_view(o);
            if let Some((id, _)) = chosen {
                view["runtime_id"] = json!(id);
            }
            view["assigned"] = json!(chosen.is_some());
            view["enabled"] = json!(chosen.is_some_and(|(_, c)| c.enabled));
            view["in_use_by"] = json!(o.executor_id.as_deref().and_then(|id| device_user(others, id)).map(ToString::to_string));
            view
        })
        .collect();
    // Agent settings that name a runtime no longer in the directory stay
    // visible as what they are: bound to a target the system withdrew.
    if let Some(settings) = settings {
        for (id, r) in &settings.runtimes {
            if offers.iter().any(|o| o.is_target(&r.target)) {
                continue;
            }
            let (kind, target) = match &r.target {
                AgentRuntimeTarget::Node { node_id, node_did } => ("node", json!({"node_id": node_id, "node_did": node_did})),
                AgentRuntimeTarget::Executor { executor_id, profile_id } => ("executor", json!({"executor_id": executor_id, "profile_id": profile_id})),
            };
            let mut entry = json!({
                "runtime_id": id, "kind": kind, "closed": true, "withdrawn": true,
                "assigned": true, "enabled": r.enabled, "computer": ComputerAccess::None,
                "baseline": RuntimeBaseline::Standard,
            });
            if let Some(t) = target.as_object() {
                entry.as_object_mut().unwrap().extend(t.clone());
            }
            out.push(entry);
        }
    }
    let host_default = offers
        .iter()
        .find(|o| o.runtime_id == HOST_RUNTIME_ID)
        .is_some_and(|host| host_by_default(host, container_available));
    let mut view = json!({
        "runtimes": out,
        "default_runtime_id": settings.map(|s| s.effective_default_runtime().to_string()),
        "configured_default_runtime_id": settings.and_then(|s| s.default_runtime_id.clone()),
        "container": CONTAINER_RUNTIME_ID,
        "container_available": container_available,
        "host_by_default": host_default,
    });
    if let Some(settings) = settings {
        view["agent_service"] = agent_service_view(offers, settings);
    }
    view
}

/// The owner's directory with who uses what (`agent.runtime_catalog`).
pub(crate) fn catalog_view(
    offers: &[NodeRuntimeOffer],
    assigned: &[(AgentId, AgentSettings)],
    online: &dyn Fn(&str) -> bool,
    container_available: bool,
) -> Value {
    let runtimes: Vec<Value> = offers
        .iter()
        .map(|o| {
            let mut view = offer_view(o);
            view["online"] = match &o.executor_id {
                Some(executor_id) => json!(online(executor_id)),
                None => Value::Null,
            };
            view["assigned_to"] = json!(assigned
                .iter()
                .flat_map(|(agent_id, settings)| {
                    settings.runtimes.iter().filter(|(_, r)| o.is_target(&r.target)).map(move |(id, r)| {
                        json!({
                            "agent_id": agent_id.to_string(),
                            "runtime_id": id,
                            "enabled": r.enabled,
                            "default": settings.default_runtime_id.as_ref() == Some(id),
                        })
                    })
                })
                .collect::<Vec<_>>());
            view
        })
        .collect();
    let host_default = offers
        .iter()
        .find(|o| o.runtime_id == HOST_RUNTIME_ID)
        .is_some_and(|host| host_by_default(host, container_available));
    json!({
        "runtimes": runtimes,
        "container": CONTAINER_RUNTIME_ID,
        "container_available": container_available,
        "host_by_default": host_default,
    })
}

pub(crate) fn settings_view(settings: &AgentSettings) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "runtimes".to_string(),
            json!(settings
                .runtimes
                .iter()
                .map(|(id, r)| (id.clone(), json!({"enabled": r.enabled, "target": r.target})))
                .collect::<BTreeMap<_, _>>()),
        ),
        ("default_runtime_id".to_string(), json!(settings.effective_default_runtime())),
        ("configured_default_runtime_id".to_string(), json!(settings.default_runtime_id)),
    ])
}

/// `node.owner_agents_set` on the stored node settings: only the policy's
/// `enabled`, `workdir_roots`, `computer` and `baseline` change; `run_as`,
/// `env`, `ops` and everything else of the node's settings stay as they
/// are. A node without a policy is not given one here: which OS account its
/// Agents run as is chosen when the node is activated.
pub(crate) fn patch_owner_agents(raw: &str, params: &Value) -> Result<String, RPCErrors> {
    let mut settings: Value = serde_json::from_str(raw).map_err(|e| RPCErrors::ReasonError(format!("node settings: {e}")))?;
    let policy = settings
        .pointer_mut("/executor/owner_agents")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| reason("no_policy", "this node is not open to Agents: no account for them was chosen when it was activated"))?;
    let mut touched = false;
    if let Some(enabled) = params.get("enabled").filter(|v| !v.is_null()) {
        let enabled = enabled.as_bool().ok_or_else(|| RPCErrors::ParseRequestError("enabled must be a boolean".into()))?;
        policy.insert("enabled".into(), json!(enabled));
        touched = true;
    }
    if let Some(roots) = params.get("workdir_roots").filter(|v| !v.is_null()) {
        let roots: Vec<String> = serde_json::from_value(roots.clone())
            .map_err(|_| RPCErrors::ParseRequestError("workdir_roots must be a list of absolute paths".into()))?;
        policy.insert("workdir_roots".into(), json!(roots));
        touched = true;
    }
    if let Some(computer) = params.get("computer").filter(|v| !v.is_null()) {
        let computer: ComputerAccess = serde_json::from_value(computer.clone())
            .map_err(|_| RPCErrors::ParseRequestError("computer takes none | observe | control".into()))?;
        policy.insert("computer".into(), json!(computer));
        touched = true;
    }
    if let Some(baseline) = params.get("baseline") {
        if baseline.is_null() {
            policy.remove("baseline");
        } else {
            let baseline: RuntimeBaseline = serde_json::from_value(baseline.clone())
                .map_err(|_| RPCErrors::ParseRequestError("baseline takes standard | extended | null".into()))?;
            policy.insert("baseline".into(), json!(baseline));
        }
        touched = true;
    }
    if !touched {
        return Err(RPCErrors::ParseRequestError(
            "node.owner_agents_set changes enabled, workdir_roots, computer or baseline".into(),
        ));
    }
    let parsed: NodeSettings = serde_json::from_value(settings.clone()).map_err(|e| reason("invalid_policy", e))?;
    if let Some(executor) = &parsed.executor {
        if let Some(policy) = executor.owner_agents.as_ref().filter(|p| p.enabled) {
            policy.validate().map_err(|e| reason("invalid_policy", e))?;
        }
    }
    serde_json::to_string(&settings).map_err(|e| RPCErrors::ReasonError(e.to_string()))
}

impl ControlPanelServer {
    /// `agent.runtime_catalog`: the caller's runtime directory — nodes (the
    /// zone owner only) and devices — each with what it allows and the
    /// Agents it is assigned to.
    pub(crate) async fn handle_agent_runtime_catalog(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let kv = ZoneKv::connect().await?;
        let owner = principal.owner_user_id.clone();
        let offers = runtime_directory(&kv, &owner).await?;
        let assigned = assignments(&kv, &owner).await;
        let online = self.agent_daemon.online_ids().await;
        let containers = container_available(&kv).await;
        let view = catalog_view(&offers, &assigned, &|id| online.contains(id), containers);
        Ok(RPCResponse::new(RPCResult::Success(view), req.seq))
    }

    /// `agent.runtime_enable {runtime_id}`: an Agent's own service switches
    /// on a runtime the owner assigned to it (the owner's click on the
    /// Agent's home page). Here because a device is on for one Agent at a
    /// time and only the control panel sees the owner's other Agents.
    pub(crate) async fn handle_agent_runtime_enable(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        let runtime_id = Self::require_param_str(&req, "runtime_id")?;
        let agent_id = (principal.principal_kind == TokenPrincipalKind::App)
            .then(|| principal.authenticated_app_id.parse::<AppInstanceId>().ok())
            .flatten()
            .and_then(|instance| AgentId::parse(instance.app_id().as_str()).ok())
            .ok_or_else(|| RPCErrors::NoPermission("agent.runtime_enable is called by an Agent's own service".into()))?;
        let owner = principal.owner_user_id.clone();
        let kv = ZoneKv::connect().await?;
        let key = agent_settings_key(&owner, &agent_id);
        let mut settings = get_value::<AgentSettings>(&kv, &key)
            .await
            .ok_or_else(|| reason("agent_not_found", &agent_id))?;
        if !settings.runtimes.contains_key(&runtime_id) {
            return Err(reason("runtime_not_assigned", format!("runtime `{runtime_id}` is not assigned to this Agent")));
        }
        let change = json!({ runtime_id.clone(): { "enabled": true } });
        if apply_update(&kv, &owner, Some(&agent_id), &mut settings, Some(&change), None).await? {
            let raw = serde_json::to_string(&settings).map_err(|e| RPCErrors::ReasonError(e.to_string()))?;
            kv.exec_tx(HashMap::from([(key, KVAction::Update(raw))]), None).await?;
            if let Err(error) = crate::user_mgr::refresh_rbac_by_scheduler("agent runtime settings changed").await {
                log::warn!("scheduler refresh after runtime change of {agent_id} failed: {error}");
            }
        }
        Ok(RPCResponse::new(RPCResult::Success(json!(settings_view(&settings))), req.seq))
    }

    /// `node.owner_agents_set {node_id, enabled?, workdir_roots?, computer?,
    /// baseline?}`: the zone owner changes what a node allows the owner's
    /// Agents — the node as a runtime.
    pub(crate) async fn handle_node_owner_agents_set(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        crate::user_mgr::require_control_panel_admin(principal)?;
        let kv = ZoneKv::connect().await?;
        if zone_owner(&kv).await.as_deref() != Some(principal.owner_user_id.as_str()) {
            return Err(RPCErrors::NoPermission("only the zone owner changes what a node allows Agents".into()));
        }
        let node_id = Self::require_param_str(&req, "node_id")?;
        let key = node_settings_key(&node_id);
        let (raw, _) = kv
            .get(&key)
            .await?
            .ok_or_else(|| reason("no_policy", format!("node `{node_id}` has no settings")))?;
        let next = patch_owner_agents(&raw, &req.params)?;
        if next != raw {
            kv.exec_tx(HashMap::from([(key, KVAction::Update(next))]), None).await?;
            if let Err(error) = crate::user_mgr::refresh_rbac_by_scheduler("node owner_agents policy changed").await {
                log::warn!("scheduler refresh after the owner_agents change of {node_id} failed: {error}");
            }
        }
        let offers = runtime_directory(&kv, &principal.owner_user_id).await?;
        let offer = offers.iter().find(|o| !o.is_device() && o.node_id == node_id).map(offer_view);
        Ok(RPCResponse::new(RPCResult::Success(json!({ "runtime": offer })), req.seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(name: &str) -> AgentId {
        AgentId::parse(&format!("{name}.test.buckyos.io")).unwrap()
    }

    fn device_offer(executor_id: &str) -> NodeRuntimeOffer {
        NodeRuntimeOffer {
            runtime_id: device_runtime_id(executor_id),
            node_id: String::new(),
            node_did: String::new(),
            device_type: String::new(),
            executor_id: Some(executor_id.to_string()),
            name: "friend-pc".into(),
            computer: ComputerAccess::Observe,
            workdir_roots: vec!["/home/friend".into()],
            read_only: false,
            baseline: RuntimeBaseline::Standard,
            expires_at_ms: Some(42),
            run_as: false,
            closed: false,
            os: String::new(),
        }
    }

    fn using(executor_id: &str, enabled: bool) -> AgentSettings {
        AgentSettings {
            runtimes: BTreeMap::from([(
                device_runtime_id(executor_id),
                AgentRuntimeSettings {
                    enabled,
                    target: AgentRuntimeTarget::Executor { executor_id: executor_id.into(), profile_id: None },
                },
            )]),
            default_runtime_id: enabled.then(|| device_runtime_id(executor_id)),
            ..AgentSettings::default()
        }
    }

    #[test]
    fn the_catalog_says_who_uses_what_and_a_device_has_one_user() {
        const DEVICE: &str = "5f0c2b9e-7a1d-4c53-9d0e-0b6a3c1f2e44";
        let offers = vec![device_offer(DEVICE)];
        let assigned = vec![(agent("jarvis"), using(DEVICE, true)), (agent("helper"), using(DEVICE, false))];
        assert_eq!(device_user(&assigned, DEVICE), Some(&agent("jarvis")));
        assert_eq!(device_user(&assigned[1..], DEVICE), None, "assigned but off is not in use");
        let view = catalog_view(&offers, &assigned, &|id| id == DEVICE, true);
        let runtime = &view["runtimes"][0];
        assert_eq!(runtime["kind"], "executor");
        assert_eq!(runtime["online"], true);
        assert_eq!(runtime["expires_at_ms"], 42);
        assert_eq!(
            runtime["assigned_to"],
            json!([
                {"agent_id": "jarvis.test.buckyos.io", "runtime_id": device_runtime_id(DEVICE), "enabled": true, "default": true},
                {"agent_id": "helper.test.buckyos.io", "runtime_id": device_runtime_id(DEVICE), "enabled": false, "default": false},
            ])
        );
        let helper = directory_view(&offers, Some(&assigned[1].1), true, &assigned[..1]);
        assert_eq!(helper["runtimes"][0]["in_use_by"], "jarvis.test.buckyos.io");
        assert_eq!((helper["runtimes"][0]["assigned"].as_bool(), helper["runtimes"][0]["enabled"].as_bool()), (Some(true), Some(false)));
    }

    #[test]
    fn the_zone_owner_changes_a_policy_without_losing_the_account_it_runs_as() {
        let raw = json!({
            "executor": {
                "enabled": true,
                "owner_agents": {
                    "enabled": true,
                    "workdir_roots": ["/home/dev"],
                    "computer": "none",
                    "run_as": {"uid": 1000, "gid": 1000},
                    "env": {"LANG": "C"},
                },
                "limits": {"max_streams": 8},
            },
            "kept": {"by": "someone else"},
        })
        .to_string();
        let next: Value = serde_json::from_str(
            &patch_owner_agents(&raw, &json!({"workdir_roots": ["/home/dev", "/srv/data"], "baseline": "extended", "enabled": false})).unwrap(),
        )
        .unwrap();
        let policy = &next["executor"]["owner_agents"];
        assert_eq!(policy["run_as"], json!({"uid": 1000, "gid": 1000}));
        assert_eq!(policy["env"], json!({"LANG": "C"}));
        assert_eq!(policy["workdir_roots"], json!(["/home/dev", "/srv/data"]));
        assert_eq!((policy["baseline"].as_str(), policy["enabled"].as_bool()), (Some("extended"), Some(false)));
        assert_eq!(next["kept"]["by"], "someone else");
        assert_eq!(next["executor"]["limits"]["max_streams"], 8);

        assert!(patch_owner_agents(&raw, &json!({"workdir_roots": ["relative"]})).unwrap_err().to_string().contains("invalid_policy"));
        assert!(patch_owner_agents(&raw, &json!({"computer": "all"})).is_err());
        assert!(patch_owner_agents(&raw, &json!({})).is_err(), "nothing to change");
        let closed = json!({"executor": {"enabled": true}}).to_string();
        assert!(patch_owner_agents(&closed, &json!({"enabled": true})).unwrap_err().to_string().contains("no_policy"));
    }
}
