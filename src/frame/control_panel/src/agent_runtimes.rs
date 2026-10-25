//! Execution runtimes an owner may give their Agents (integration TODO §3.1):
//! the system directory of nodes that opened themselves to the owner's
//! Agents (`nodes/<node>/settings.executor.owner_agents`), the Agent's own
//! choice in `AgentSettings.runtimes`, and the defaults a Desktop owner's
//! new Agent gets. The control panel never writes node settings: the node
//! policy is the administrator's (and the desktop seed's); the scheduler
//! multiplies both into the Agent's profile.

use std::collections::BTreeMap;

use ::kRPC::RPCErrors;
use buckyos_api::{
    is_valid_runtime_id, node_settings_key, AgentRuntimeSettings, AgentRuntimeTarget,
    AgentSettings, ComputerAccess, NodeSettings, CONTAINER_RUNTIME_ID, HOST_RUNTIME_ID,
    ZONE_OWNER_USER_ID_KEY,
};
use name_lib::DeviceInfo;
use serde::Serialize;
use serde_json::{json, Value};

use crate::agent_mgr::{get_value, reason, AgentKv};

/// One node the owner's Agents may run on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct NodeRuntimeOffer {
    pub runtime_id: String,
    pub node_id: String,
    pub node_did: String,
    pub device_type: String,
    pub name: String,
    /// The most ComputerUse the node grants.
    pub computer: ComputerAccess,
    pub workdir_roots: Vec<String>,
    /// The policy exists but the administrator closed it.
    pub closed: bool,
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

/// The nodes whose `owner_agents` policy exists, for the zone owner's
/// Agents; desktop nodes first, so the owner's desktop is `host`. Other
/// users have no node runtimes (integration TODO §3.1: only the Desktop
/// owner's Agents are opened by default).
pub(crate) async fn runtime_directory(kv: &dyn AgentKv, owner: &str) -> Result<Vec<NodeRuntimeOffer>, RPCErrors> {
    if zone_owner(kv).await.as_deref() != Some(owner) {
        return Ok(Vec::new());
    }
    let mut offers = Vec::new();
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
            name: info.device_doc.name.clone(),
            computer: policy.computer,
            workdir_roots: policy.workdir_roots.clone(),
            closed: !executor.enabled || !policy.enabled,
        });
    }
    offers.sort_by(|a, b| {
        let rank = |o: &NodeRuntimeOffer| (o.device_type != "desktop", o.closed, o.node_id.clone());
        rank(a).cmp(&rank(b))
    });
    for (index, offer) in offers.iter_mut().enumerate() {
        offer.runtime_id = node_runtime_id(index, &offer.node_id);
    }
    Ok(offers)
}

/// A new Agent of the Desktop owner: the owner's desktop (`host`) enabled
/// with the node's full ComputerUse grant, and the default runtime of new
/// sessions. Nothing when the owner's zone offers no open node.
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
    settings.runtimes.insert(
        HOST_RUNTIME_ID.to_string(),
        AgentRuntimeSettings {
            enabled: true,
            computer: host.computer,
            target: AgentRuntimeTarget::Node {
                node_id: host.node_id.clone(),
                node_did: host.node_did.clone(),
            },
        },
    );
    settings.default_runtime_id = Some(HOST_RUNTIME_ID.to_string());
}

fn parse_computer(value: Option<&Value>) -> Result<Option<ComputerAccess>, RPCErrors> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(v) => serde_json::from_value::<ComputerAccess>(v.clone())
            .map(Some)
            .map_err(|_| RPCErrors::ParseRequestError("computer takes none | observe | control".into())),
    }
}

/// `agent.update` runtime changes: `runtimes: { "<id>": {enabled?, computer?} | null }`
/// and `default_runtime_id: "<id>" | null`. A runtime is enabled only from
/// the directory (the target is the directory's, never the caller's), its
/// ComputerUse never exceeds the node's grant, and the default must be an
/// enabled runtime.
pub(crate) async fn apply_update(
    kv: &dyn AgentKv,
    owner: &str,
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
        for (id, change) in map {
            if !is_valid_runtime_id(id) || id == CONTAINER_RUNTIME_ID {
                return Err(reason("invalid_runtime", format!("`{id}` is not a runtime id")));
            }
            if change.is_null() {
                changed |= settings.runtimes.remove(id).is_some();
                continue;
            }
            let change = change.as_object().ok_or_else(|| {
                RPCErrors::ParseRequestError(format!("runtime `{id}` must be an object or null"))
            })?;
            let enabled = match change.get("enabled") {
                None => None,
                Some(Value::Bool(enabled)) => Some(*enabled),
                Some(_) => return Err(RPCErrors::ParseRequestError(format!("runtime `{id}` enabled must be a boolean"))),
            };
            let computer = parse_computer(change.get("computer"))?;
            let offer = offers.iter().find(|o| &o.runtime_id == id);
            let entry = match settings.runtimes.get(id).cloned() {
                Some(current) => current,
                None => {
                    let offer = offer.ok_or_else(|| {
                        reason("runtime_not_offered", format!("runtime `{id}` is not offered to this owner (see agent.runtime_directory)"))
                    })?;
                    AgentRuntimeSettings {
                        enabled: false,
                        computer: ComputerAccess::None,
                        target: AgentRuntimeTarget::Node {
                            node_id: offer.node_id.clone(),
                            node_did: offer.node_did.clone(),
                        },
                    }
                }
            };
            let mut next = entry.clone();
            if let Some(enabled) = enabled {
                if enabled && offer.is_none_or(|o| o.closed) {
                    return Err(reason(
                        "runtime_not_offered",
                        format!("runtime `{id}` is not open to this owner's Agents right now"),
                    ));
                }
                next.enabled = enabled;
            }
            if let Some(computer) = computer {
                if let Some(offer) = offer {
                    if computer > offer.computer {
                        return Err(reason(
                            "computer_exceeds_node",
                            format!("runtime `{id}` grants at most `{}` desktop access", offer.computer.as_str()),
                        ));
                    }
                }
                next.computer = computer;
            }
            if next != entry {
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

/// The directory as the owner sees it for one Agent.
pub(crate) fn directory_view(offers: &[NodeRuntimeOffer], settings: Option<&AgentSettings>) -> Value {
    let mut out: Vec<Value> = offers
        .iter()
        .map(|o| {
            let chosen = settings.and_then(|s| s.runtimes.get(&o.runtime_id));
            json!({
                "runtime_id": o.runtime_id,
                "kind": "node",
                "node_id": o.node_id,
                "node_did": o.node_did,
                "device_type": o.device_type,
                "name": o.name,
                "computer_max": o.computer,
                "workdir_roots": o.workdir_roots,
                "closed": o.closed,
                "enabled": chosen.is_some_and(|c| c.enabled),
                "computer": chosen.map(|c| c.computer).unwrap_or_default(),
            })
        })
        .collect();
    // Agent settings that name a node no longer offered stay visible as
    // what they are: bound to a target the system withdrew.
    if let Some(settings) = settings {
        for (id, r) in &settings.runtimes {
            if offers.iter().any(|o| &o.runtime_id == id) {
                continue;
            }
            let (kind, target) = match &r.target {
                AgentRuntimeTarget::Node { node_id, node_did } => ("node", json!({"node_id": node_id, "node_did": node_did})),
                AgentRuntimeTarget::Executor { executor_id } => ("executor", json!({"executor_id": executor_id})),
            };
            let mut entry = json!({
                "runtime_id": id, "kind": kind, "closed": true, "withdrawn": true,
                "enabled": r.enabled, "computer": r.computer, "computer_max": ComputerAccess::None,
            });
            if let Some(t) = target.as_object() {
                entry.as_object_mut().unwrap().extend(t.clone());
            }
            out.push(entry);
        }
    }
    json!({
        "runtimes": out,
        "default_runtime_id": settings.map(|s| s.effective_default_runtime().to_string()),
        "container": CONTAINER_RUNTIME_ID,
    })
}

pub(crate) fn settings_view(settings: &AgentSettings) -> BTreeMap<String, Value> {
    BTreeMap::from([
        (
            "runtimes".to_string(),
            json!(settings
                .runtimes
                .iter()
                .map(|(id, r)| (id.clone(), json!({"enabled": r.enabled, "computer": r.computer, "target": r.target})))
                .collect::<BTreeMap<_, _>>()),
        ),
        ("default_runtime_id".to_string(), json!(settings.effective_default_runtime())),
    ])
}
