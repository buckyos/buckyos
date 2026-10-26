//! Node Executor authorization snapshot (doc/arch/node_exector.md §5.4–5.6).
//!
//! `nodes/<node>/settings.executor` (administrator input) plus the
//! authoritative device, AgentSpec, AgentSettings and App records become
//! `nodes/<node>/config.executor` and exact RBAC rules in one transaction.
//! The revision digests the inputs; the generation grows only when the
//! revision changes; the lease is renewed only after the inputs are verified.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use buckyos_api::{
    agent_settings_key, executor_revision, owner_agent_profile_id, user_app_spec_key,
    AgentSettings, AgentSpec, AppInstanceId, AppServiceSpec, ExecutorConfig, ExecutorLimits,
    ExecutorPrincipalBinding, ExecutorProfile, ExecutorSettings, NodeSettings,
    NODE_EXECUTOR_LEASE_MS, NODE_EXECUTOR_RENEW_BEFORE_MS, ZONE_OWNER_USER_ID_KEY,
};
use buckyos_kit::KVAction;
use log::*;
use name_lib::DeviceInfo;
use serde_json::{json, Value};

struct Derived {
    allowed_devices: Vec<String>,
    principal_bindings: BTreeMap<String, ExecutorPrincipalBinding>,
    profiles: BTreeMap<String, ExecutorProfile>,
    limits: ExecutorLimits,
}

fn node_ids(input: &HashMap<String, String>) -> Vec<String> {
    let mut ids: Vec<String> = input
        .keys()
        .filter_map(|k| {
            let parts: Vec<&str> = k.split('/').collect();
            match parts.as_slice() {
                ["nodes", node, "config"] => Some(node.to_string()),
                _ => None,
            }
        })
        .collect();
    ids.sort();
    ids
}

fn executor_settings(input: &HashMap<String, String>, node_id: &str) -> Option<ExecutorSettings> {
    let raw = input.get(&buckyos_api::node_settings_key(node_id))?;
    match serde_json::from_str::<NodeSettings>(raw) {
        Ok(settings) => settings.executor,
        Err(err) => {
            warn!("executor: nodes/{node_id}/settings is invalid: {err}");
            None
        }
    }
}

fn current_config(input: &HashMap<String, String>, node_id: &str) -> Option<ExecutorConfig> {
    let raw = input.get(&buckyos_api::node_config_key(node_id))?;
    let value: Value = serde_json::from_str(raw).ok()?;
    let executor = value.get("executor")?;
    if executor.is_null() {
        return None;
    }
    serde_json::from_value(executor.clone()).ok()
}

fn agent_specs(input: &HashMap<String, String>) -> Vec<AgentSpec> {
    let mut keys: Vec<&String> = input.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter(|k| matches!(k.split('/').collect::<Vec<_>>().as_slice(), ["users", _, "agents", _, "spec"]))
        .filter_map(|k| match serde_json::from_str::<AgentSpec>(&input[k]) {
            Ok(spec) if spec.validate().is_ok() => Some(spec),
            _ => {
                warn!("executor: ignore invalid AgentSpec at {k}");
                None
            }
        })
        .collect()
}

fn device_dids(input: &HashMap<String, String>) -> BTreeSet<String> {
    input
        .iter()
        .filter(|(k, _)| k.starts_with("devices/") && k.ends_with("/info"))
        .filter_map(|(_, v)| serde_json::from_str::<DeviceInfo>(v).ok())
        .map(|info| info.device_doc.id.to_string())
        .collect()
}

fn agent_settings(input: &HashMap<String, String>, spec: &AgentSpec) -> Option<AgentSettings> {
    let owner = spec.binding.target_app_instance_id.owner_user_id();
    let raw = input.get(&agent_settings_key(owner, &spec.agent_id))?;
    match serde_json::from_str::<AgentSettings>(raw) {
        Ok(settings) => Some(settings),
        Err(err) => {
            warn!("executor: settings of {} are invalid: {err}", spec.agent_id);
            None
        }
    }
}

fn agent_enabled(input: &HashMap<String, String>, spec: &AgentSpec) -> bool {
    let owner = spec.binding.target_app_instance_id.owner_user_id();
    match input.get(&agent_settings_key(owner, &spec.agent_id)) {
        None => true,
        Some(_) => agent_settings(input, spec).map(|s| s.enabled).unwrap_or(false),
    }
}

fn zone_owner(input: &HashMap<String, String>) -> Option<String> {
    let raw = input.get(ZONE_OWNER_USER_ID_KEY)?;
    serde_json::from_str::<String>(raw)
        .ok()
        .or_else(|| Some(raw.trim().trim_matches('"').to_string()))
        .filter(|s| !s.is_empty())
}

/// The node's device DID as the device reports it (`devices/<node>/info`,
/// the form `allowed_devices` and the Agents' runtime targets use);
/// `nodes/<node>/config.node_did` may carry another form of the same DID.
fn node_did(input: &HashMap<String, String>, node_id: &str) -> Option<String> {
    if let Some(info) = input
        .get(&format!("devices/{node_id}/info"))
        .and_then(|raw| serde_json::from_str::<DeviceInfo>(raw).ok())
    {
        return Some(info.device_doc.id.to_string());
    }
    let raw = input.get(&buckyos_api::node_config_key(node_id))?;
    let value: Value = serde_json::from_str(raw).ok()?;
    value.get("node_did").and_then(Value::as_str).map(str::to_string)
}

/// Profiles of the node owner's Agents (integration TODO §0 R4): the node's
/// `owner_agents` policy multiplied by each Agent's own runtime settings
/// that name this node. Administrator profiles with the same id win.
fn owner_agent_profiles(
    node_id: &str,
    settings: &ExecutorSettings,
    input: &HashMap<String, String>,
    specs: &[AgentSpec],
) -> BTreeMap<String, ExecutorProfile> {
    let mut out = BTreeMap::new();
    let Some(policy) = settings.owner_agents.as_ref().filter(|p| p.enabled) else {
        return out;
    };
    let Some(owner) = zone_owner(input) else {
        warn!("executor: nodes/{node_id}: owner_agents is open but the zone has no owner user id");
        return out;
    };
    for spec in specs {
        if spec.binding.target_app_instance_id.owner_user_id() != owner {
            continue;
        }
        let Some(agent_settings) = agent_settings(input, spec) else {
            continue;
        };
        if agent_settings.validate_runtimes().is_err() {
            warn!("executor: runtime settings of {} are inconsistent; not mapped", spec.agent_id);
            continue;
        }
        let Some((_, runtime)) = agent_settings.node_runtime(node_id) else {
            continue;
        };
        let agent_did = spec.agent_did.to_string();
        let profile_id = owner_agent_profile_id(&agent_did);
        if settings.profiles.contains_key(&profile_id) {
            continue;
        }
        let profile = policy.profile_for(&agent_did, runtime.computer);
        if let Err(err) = profile.validate(&profile_id) {
            warn!("executor: derived profile {profile_id} rejected: {err}");
            continue;
        }
        out.insert(profile_id, profile);
    }
    out
}

fn app_installed(input: &HashMap<String, String>, instance: &AppInstanceId) -> bool {
    input
        .get(&user_app_spec_key(instance.owner_user_id(), instance.app_id()))
        .and_then(|raw| serde_json::from_str::<AppServiceSpec>(raw).ok())
        .map(|spec| spec.is_installed())
        .unwrap_or(false)
}

/// Principal mapping of §7.2: one existing AgentSpec per App instance
/// (disabled Agents count), AppDID == AgentDID.
fn derive(
    node_id: &str,
    settings: &ExecutorSettings,
    input: &HashMap<String, String>,
    specs: &[AgentSpec],
) -> Option<Derived> {
    if !settings.enabled {
        return None;
    }
    if let Err(err) = settings.validate() {
        warn!("executor: nodes/{node_id}/settings.executor rejected: {err}");
        return None;
    }
    let known = device_dids(input);
    let mut profiles = settings.profiles.clone();
    profiles.extend(owner_agent_profiles(node_id, settings, input, specs));
    let mut wanted_devices = settings.allowed_devices.clone();
    if settings.owner_agents_open() {
        // The owner's Agents run on this node itself (or reach it through
        // its own gateway): the node is always an allowed source.
        wanted_devices.extend(node_did(input, node_id));
    }
    let allowed_devices: Vec<String> = wanted_devices
        .iter()
        .filter(|d| {
            let ok = known.contains(*d);
            if !ok {
                warn!("executor: nodes/{node_id}: allowed device {d} is not a known device");
            }
            ok
        })
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if allowed_devices.is_empty() {
        warn!("executor: nodes/{node_id}: no known allowed device");
        return None;
    }
    if profiles.is_empty() {
        return None;
    }
    let mut per_instance: HashMap<&AppInstanceId, usize> = HashMap::new();
    for spec in specs {
        *per_instance
            .entry(&spec.binding.target_app_instance_id)
            .or_default() += 1;
    }
    let principals: BTreeSet<&String> = profiles
        .values()
        .flat_map(|p| p.principals.iter())
        .collect();
    let mut principal_bindings = BTreeMap::new();
    for agent_did in principals {
        let Some(spec) = specs.iter().find(|s| &s.agent_did.to_string() == agent_did) else {
            warn!("executor: principal {agent_did} has no AgentSpec");
            continue;
        };
        let instance = &spec.binding.target_app_instance_id;
        if per_instance.get(instance).copied().unwrap_or(0) != 1 {
            warn!("executor: principal {agent_did} shares runtime {instance}; not mapped");
            continue;
        }
        if instance.app_id().app_did().to_string() != *agent_did {
            warn!("executor: principal {agent_did} runtime {instance} has a different AppDID");
            continue;
        }
        principal_bindings.insert(
            agent_did.clone(),
            ExecutorPrincipalBinding {
                app_instance_id: instance.clone(),
                agent_generation: spec.generation,
                binding_generation: spec.binding.generation,
                enabled: agent_enabled(input, spec),
                app_installed: app_installed(input, instance),
                permission_mode: spec.permission_mode,
            },
        );
    }
    Some(Derived {
        allowed_devices,
        principal_bindings,
        profiles,
        limits: settings.limits.clone(),
    })
}

fn revision_of(node_id: &str, d: &Derived) -> String {
    executor_revision(&json!({
        "node": node_id,
        "allowed_devices": d.allowed_devices,
        "principal_bindings": d.principal_bindings,
        "profiles": d.profiles,
        "limits": d.limits,
    }))
}

fn set_executor(node_id: &str, config: &ExecutorConfig) -> (String, KVAction) {
    let value = serde_json::to_value(config).unwrap_or(Value::Null);
    (
        buckyos_api::node_config_key(node_id),
        KVAction::SetByJsonPath(HashMap::from([("/executor".to_string(), Some(value))])),
    )
}

/// Snapshot writes for every node, and whether any generation changed (the
/// RBAC tail must then be rebuilt in the same transaction).
pub(crate) fn update_executor_snapshots(
    input: &HashMap<String, String>,
    now_ms: u64,
) -> (HashMap<String, KVAction>, bool) {
    let specs = agent_specs(input);
    let mut actions = HashMap::new();
    let mut changed = false;
    for node_id in node_ids(input) {
        let current = current_config(input, &node_id);
        let derived = executor_settings(input, &node_id)
            .and_then(|s| derive(&node_id, &s, input, &specs));
        let next = match (derived, current) {
            (Some(d), current) => {
                let revision = revision_of(&node_id, &d);
                match current {
                    Some(c) if c.enabled && c.revision == revision => {
                        if c.auth_valid_until_ms <= now_ms + NODE_EXECUTOR_RENEW_BEFORE_MS {
                            Some(ExecutorConfig {
                                auth_valid_until_ms: now_ms + NODE_EXECUTOR_LEASE_MS,
                                ..c
                            })
                        } else {
                            None
                        }
                    }
                    c => {
                        changed = true;
                        Some(ExecutorConfig {
                            enabled: true,
                            revision,
                            generation: c.map(|c| c.generation + 1).unwrap_or(1),
                            auth_valid_until_ms: now_ms + NODE_EXECUTOR_LEASE_MS,
                            allowed_devices: d.allowed_devices,
                            principal_bindings: d.principal_bindings,
                            profiles: d.profiles,
                            limits: d.limits,
                        })
                    }
                }
            }
            (None, Some(c)) if c.enabled => {
                changed = true;
                Some(ExecutorConfig {
                    enabled: false,
                    revision: executor_revision(&json!({"node": node_id, "disabled": true})),
                    generation: c.generation + 1,
                    auth_valid_until_ms: now_ms,
                    allowed_devices: Vec::new(),
                    principal_bindings: BTreeMap::new(),
                    profiles: BTreeMap::new(),
                    limits: c.limits,
                })
            }
            (None, _) => None,
        };
        if let Some(next) = next {
            let (key, action) = set_executor(&node_id, &next);
            actions.insert(key, action);
        }
    }
    (actions, changed)
}

fn rbac_literal(value: &str) -> String {
    let mut escaped = String::new();
    for ch in value.chars() {
        if "\\.^$|?*+()[]{}".contains(ch) {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped
}

/// Exact rules for both RBAC sides of the mapped App token: the owner user
/// and `app:<AppId>`, per node profile, actions limited to the granted ops.
pub(crate) fn executor_rbac_lines(input: &HashMap<String, String>) -> Vec<String> {
    let specs = agent_specs(input);
    let mut lines = Vec::new();
    for node_id in node_ids(input) {
        let Some(settings) = executor_settings(input, &node_id) else {
            continue;
        };
        let Some(d) = derive(&node_id, &settings, input, &specs) else {
            continue;
        };
        let snapshot = ExecutorConfig {
            enabled: true,
            revision: String::new(),
            generation: 1,
            auth_valid_until_ms: 0,
            allowed_devices: d.allowed_devices,
            principal_bindings: d.principal_bindings,
            profiles: d.profiles,
            limits: d.limits,
        };
        for (profile_id, profile) in &snapshot.profiles {
            let resource = format!(
                "obj://node/{}/executor/{}",
                rbac_literal(&node_id),
                rbac_literal(profile_id)
            );
            for agent_did in &profile.principals {
                let ops = snapshot.granted_ops(profile_id, agent_did);
                if ops.is_empty() {
                    continue;
                }
                let binding = &snapshot.principal_bindings[agent_did];
                let actions = format!(
                    "^executor\\.({})$",
                    ops.iter().map(|op| op.as_str()).collect::<Vec<_>>().join("|")
                );
                let owner = binding.app_instance_id.owner_user_id();
                let app = binding.app_instance_id.app_id().as_str();
                lines.push(format!("p, {owner}, {resource}, {actions}, allow"));
                lines.push(format!("p, app:{app}, {resource}, {actions}, allow"));
            }
        }
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AgentId, AgentPermissionMode, AgentServiceBinding, ExecutorOp, ExecutorRunAs,
        AGENT_SPEC_SCHEMA_VERSION,
    };
    use name_lib::{generate_ed25519_key_pair, AgentDocument, DeviceDocument, DID};

    const NOW: u64 = 1_800_000_000_000;

    fn agent_spec(owner: &str, name: &str, instance_name: &str) -> AgentSpec {
        let agent_did = DID::new("web", &format!("{name}.example.com"));
        let (_, public_key) = generate_ed25519_key_pair();
        let agent_doc = AgentDocument::new(
            agent_did.clone(),
            DID::new("web", &format!("{owner}.example.com")),
            serde_json::from_value(public_key).unwrap(),
        );
        let (agent_doc_object_id, _) = ndn_lib::build_named_object_by_json(
            "agentdoc",
            &serde_json::to_value(&agent_doc).unwrap(),
        );
        let agent_id = AgentId::from_agent_did(&agent_did).unwrap();
        AgentSpec {
            schema_version: AGENT_SPEC_SCHEMA_VERSION,
            agent_id,
            agent_did: agent_did.clone(),
            agent_doc_object_id: agent_doc_object_id.clone(),
            agent_doc,
            permission_mode: AgentPermissionMode::ReadWrite,
            binding: AgentServiceBinding {
                schema_version: AGENT_SPEC_SCHEMA_VERSION,
                agent_did,
                agent_doc_object_id,
                target_app_instance_id: format!("{instance_name}.example.com@{owner}")
                    .parse()
                    .unwrap(),
                service_name: "www".into(),
                generation: 1,
            },
            generation: 1,
        }
    }

    fn put_spec(input: &mut HashMap<String, String>, spec: &AgentSpec) {
        let owner = spec.binding.target_app_instance_id.owner_user_id().to_string();
        input.insert(
            buckyos_api::agent_spec_key(&owner, &spec.agent_id),
            serde_json::to_string(spec).unwrap(),
        );
    }

    fn put_app(input: &mut HashMap<String, String>, instance: &AppInstanceId) {
        let spec = crate::system_config_agent::app_lifecycle_tests::app_spec();
        input.insert(
            user_app_spec_key(instance.owner_user_id(), instance.app_id()),
            serde_json::to_string(&spec).unwrap(),
        );
    }

    fn base_input(agent: &AgentSpec) -> (HashMap<String, String>, String) {
        let device = DeviceInfo::from_device_doc(&DeviceDocument::new(
            "ood1",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA".into(),
        ));
        let device_did = device.device_doc.id.to_string();
        let settings = NodeSettings {
            executor: Some(ExecutorSettings {
                enabled: true,
                allowed_devices: vec![device_did.clone(), "did:dev:unknown".into()],
                profiles: BTreeMap::from([(
                    "dev".to_string(),
                    ExecutorProfile {
                        run_as: Some(ExecutorRunAs { uid: 1001, gid: 1001 }),
                        workdir_roots: vec!["/home/devtest/ws".into()],
                        env: BTreeMap::new(),
                        principals: vec![agent.agent_did.to_string()],
                        ops: ExecutorOp::ALL.to_vec(),
                    },
                )]),
                owner_agents: None,
                limits: ExecutorLimits::default(),
            }),
        };
        let mut input = HashMap::from([
            ("devices/ood1/info".to_string(), serde_json::to_string(&device).unwrap()),
            (
                "nodes/ood1/config".to_string(),
                json!({"node_id": "ood1", "node_did": device_did, "kernel": {}, "apps": {},
                       "frame_services": {}, "state": "running"})
                .to_string(),
            ),
            ("nodes/ood1/settings".to_string(), serde_json::to_string(&settings).unwrap()),
        ]);
        put_spec(&mut input, agent);
        (input, device_did)
    }

    fn apply(input: &mut HashMap<String, String>, actions: &HashMap<String, KVAction>) -> Option<ExecutorConfig> {
        for (key, action) in actions {
            let KVAction::SetByJsonPath(paths) = action else { panic!() };
            let mut value: Value = serde_json::from_str(&input[key]).unwrap();
            for (path, v) in paths {
                buckyos_kit::set_json_by_path(&mut value, path, v.as_ref());
            }
            input.insert(key.clone(), value.to_string());
        }
        current_config(input, "ood1")
    }

    #[test]
    fn snapshot_generation_lease_and_one_to_one_mapping() {
        let jarvis = agent_spec("devtest", "jarvis", "jarvis");
        let (mut input, device_did) = base_input(&jarvis);
        let (actions, changed) = update_executor_snapshots(&input, NOW);
        assert!(changed);
        let first = apply(&mut input, &actions).unwrap();
        assert_eq!(first.generation, 1);
        assert_eq!(first.allowed_devices, vec![device_did]);
        assert_eq!(first.auth_valid_until_ms, NOW + NODE_EXECUTOR_LEASE_MS);
        let binding = &first.principal_bindings[&jarvis.agent_did.to_string()];
        assert!(binding.enabled && !binding.app_installed);

        // Inside the lease nothing is written; near its end only the deadline moves.
        assert!(update_executor_snapshots(&input, NOW + 1_000).0.is_empty());
        let (actions, changed) = update_executor_snapshots(&input, NOW + 15_000);
        assert!(!changed);
        let renewed = apply(&mut input, &actions).unwrap();
        assert_eq!((renewed.generation, &renewed.revision), (1, &first.revision));
        assert_eq!(renewed.auth_valid_until_ms, NOW + 15_000 + NODE_EXECUTOR_LEASE_MS);

        // An input change bumps the generation.
        put_app(&mut input, &jarvis.binding.target_app_instance_id);
        let (actions, changed) = update_executor_snapshots(&input, NOW + 16_000);
        assert!(changed);
        let installed = apply(&mut input, &actions).unwrap();
        assert_eq!(installed.generation, 2);
        assert!(installed.principal_bindings[&jarvis.agent_did.to_string()].app_installed);

        // A second Agent on the same runtime (not even a principal) removes the mapping.
        let helper = agent_spec("devtest", "helper", "jarvis");
        put_spec(&mut input, &helper);
        let actions = update_executor_snapshots(&input, NOW + 17_000).0;
        let shared = apply(&mut input, &actions).unwrap();
        assert_eq!(shared.generation, 3);
        assert!(shared.principal_bindings.is_empty());

        // Removing the settings publishes a disabled snapshot once.
        input.remove("nodes/ood1/settings");
        let actions = update_executor_snapshots(&input, NOW + 18_000).0;
        let disabled = apply(&mut input, &actions).unwrap();
        assert!(!disabled.enabled);
        assert_eq!(disabled.generation, 4);
        assert!(update_executor_snapshots(&input, NOW + 19_000).0.is_empty());
    }

    /// Integration TODO §0 R4: the node's `owner_agents` policy × the
    /// Agent's own runtime setting derives a per-Agent profile; closing
    /// either side removes it; another Agent's capability is untouched.
    #[test]
    fn owner_agents_policy_derives_per_agent_profiles() {
        use buckyos_api::{AgentRuntimeSettings, AgentRuntimeTarget, ComputerAccess, OwnerAgentsPolicy};
        let jarvis = agent_spec("devtest", "jarvis", "jarvis");
        let helper = agent_spec("devtest", "helper", "helper");
        let (mut input, device_did) = base_input(&jarvis);
        // The node config names the device in another DID form than its
        // device document (as a real zone does); the device document wins.
        input.insert(
            "nodes/ood1/config".to_string(),
            json!({"node_id": "ood1", "node_did": "did:bns:ood1", "kernel": {}, "apps": {},
                   "frame_services": {}, "state": "running"})
            .to_string(),
        );
        put_spec(&mut input, &helper);
        put_app(&mut input, &jarvis.binding.target_app_instance_id);
        put_app(&mut input, &helper.binding.target_app_instance_id);
        input.insert(ZONE_OWNER_USER_ID_KEY.to_string(), json!("devtest").to_string());
        let mut settings: NodeSettings = serde_json::from_str(&input["nodes/ood1/settings"]).unwrap();
        let executor = settings.executor.as_mut().unwrap();
        executor.profiles.clear();
        executor.allowed_devices.clear();
        executor.owner_agents = Some(OwnerAgentsPolicy {
            enabled: true,
            workdir_roots: vec!["/home/devtest".into()],
            env: BTreeMap::new(),
            ops: Vec::new(),
            computer: ComputerAccess::Control,
            run_as: None,
        });
        input.insert("nodes/ood1/settings".into(), serde_json::to_string(&settings).unwrap());
        let runtime = |computer: ComputerAccess| AgentSettings {
            runtimes: BTreeMap::from([(
                "host".to_string(),
                AgentRuntimeSettings {
                    enabled: true,
                    computer,
                    target: AgentRuntimeTarget::Node { node_id: "ood1".into(), node_did: device_did.clone() },
                    baseline: None,
                },
            )]),
            default_runtime_id: Some("host".into()),
            ..AgentSettings::default()
        };
        input.insert(agent_settings_key("devtest", &jarvis.agent_id), serde_json::to_string(&runtime(ComputerAccess::Observe)).unwrap());
        input.insert(agent_settings_key("devtest", &helper.agent_id), serde_json::to_string(&runtime(ComputerAccess::Control)).unwrap());

        let (actions, changed) = update_executor_snapshots(&input, NOW);
        assert!(changed);
        let config = apply(&mut input, &actions).unwrap();
        assert_eq!(config.allowed_devices, vec![device_did.clone()], "the node itself is allowed");
        let jarvis_profile = owner_agent_profile_id(&jarvis.agent_did.to_string());
        let helper_profile = owner_agent_profile_id(&helper.agent_did.to_string());
        assert_eq!(config.profiles.len(), 2);
        assert!(config.profiles[&jarvis_profile].run_as.is_none());
        let j = config.granted_ops(&jarvis_profile, &jarvis.agent_did.to_string());
        let h = config.granted_ops(&helper_profile, &helper.agent_did.to_string());
        assert!(j.contains(&ExecutorOp::Exec) && j.contains(&ExecutorOp::ComputerObserve) && !j.contains(&ExecutorOp::ComputerAct));
        assert!(h.contains(&ExecutorOp::ComputerAct));
        assert!(config.granted_ops(&helper_profile, &jarvis.agent_did.to_string()).is_empty());

        // The owner switches jarvis' host off: only jarvis loses its profile.
        let mut off = runtime(ComputerAccess::Observe);
        off.runtimes.get_mut("host").unwrap().enabled = false;
        off.default_runtime_id = None;
        input.insert(agent_settings_key("devtest", &jarvis.agent_id), serde_json::to_string(&off).unwrap());
        let actions = update_executor_snapshots(&input, NOW + 1_000).0;
        let config = apply(&mut input, &actions).unwrap();
        assert_eq!(config.generation, 2);
        assert!(!config.profiles.contains_key(&jarvis_profile) && config.profiles.contains_key(&helper_profile));

        // The administrator closes the policy: the node has no profile left.
        settings.executor.as_mut().unwrap().owner_agents.as_mut().unwrap().enabled = false;
        input.insert("nodes/ood1/settings".into(), serde_json::to_string(&settings).unwrap());
        let actions = update_executor_snapshots(&input, NOW + 2_000).0;
        let config = apply(&mut input, &actions).unwrap();
        assert!(!config.enabled);
    }

    #[test]
    fn agent_did_must_equal_app_did_and_disabled_agents_are_flagged() {
        let odd = agent_spec("devtest", "odd", "runtime");
        let (input, _) = base_input(&odd);
        let config = {
            let mut input = input.clone();
            let actions = update_executor_snapshots(&input, NOW).0;
            apply(&mut input, &actions).unwrap()
        };
        assert!(config.principal_bindings.is_empty());

        let jarvis = agent_spec("devtest", "jarvis", "jarvis");
        let (mut input, _) = base_input(&jarvis);
        input.insert(
            agent_settings_key("devtest", &jarvis.agent_id),
            json!({"enabled": false}).to_string(),
        );
        let actions = update_executor_snapshots(&input, NOW).0;
        let config = apply(&mut input, &actions).unwrap();
        assert!(!config.principal_bindings[&jarvis.agent_did.to_string()].enabled);
        assert!(executor_rbac_lines(&input).is_empty());
    }

    #[tokio::test]
    async fn rbac_lines_grant_both_sides_and_respect_read_only() {
        let mut jarvis = agent_spec("devtest", "jarvis", "jarvis");
        let (mut input, _) = base_input(&jarvis);
        put_app(&mut input, &jarvis.binding.target_app_instance_id);
        let lines = executor_rbac_lines(&input);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("p, devtest, obj://node/ood1/executor/dev, ^executor\\.("));
        assert!(lines[1].starts_with("p, app:jarvis.example.com, obj://node/ood1/executor/dev,"));

        let enforce = |policy: String| async move {
            let config = buckyos_api::build_current_rbac_config(Some(&policy));
            rbac::create_enforcer(&config.model, &config.policy).await.unwrap();
        };
        let resource = "obj://node/ood1/executor/dev";
        let KVAction::Update(policy) = crate::system_config_agent::update_rbac_for_test(&input).await["system/rbac/policy"].clone() else { panic!() };
        enforce(policy).await;
        assert!(rbac::enforce("devtest", "app:jarvis.example.com", resource, "executor.exec", None).await);
        assert!(!rbac::enforce("devtest", "app:jarvis.example.com", "obj://node/ood2/executor/dev", "executor.exec", None).await);
        assert!(!rbac::enforce("mallory", "app:jarvis.example.com", resource, "executor.exec", None).await);

        jarvis.permission_mode = AgentPermissionMode::ReadOnly;
        put_spec(&mut input, &jarvis);
        let KVAction::Update(policy) = crate::system_config_agent::update_rbac_for_test(&input).await["system/rbac/policy"].clone() else { panic!() };
        enforce(policy).await;
        assert!(rbac::enforce("devtest", "app:jarvis.example.com", resource, "executor.read", None).await);
        assert!(!rbac::enforce("devtest", "app:jarvis.example.com", resource, "executor.exec", None).await);
    }
}
