use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{AgentPermissionMode, AppInstanceId};

pub const NODE_EXECUTOR_PROTOCOL_VERSION: u32 = 1;
pub const NODE_EXECUTOR_SERVICE_PORT: u16 = 3184;
/// Node gateway SOCKS5 entry for Agents opening streams to remote executors.
pub const NODE_EXECUTOR_ENTRY_PORT: u16 = 3185;
pub const NODE_EXECUTOR_LEASE_MS: u64 = 30_000;
pub const NODE_EXECUTOR_RENEW_BEFORE_MS: u64 = 20_000;
pub const NODE_EXECUTOR_GATEWAY_BLOCK: &str = "node_executor";

pub fn node_settings_key(node_id: &str) -> String {
    format!("nodes/{node_id}/settings")
}

pub fn node_config_key(node_id: &str) -> String {
    format!("nodes/{node_id}/config")
}

pub fn node_executor_resource(node_id: &str, profile_id: &str) -> String {
    format!("obj://node/{node_id}/executor/{profile_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutorOp {
    OpenStream,
    Describe,
    Exec,
    Read,
    Write,
    Edit,
    Query,
    Cancel,
}

impl ExecutorOp {
    pub const ALL: [ExecutorOp; 8] = [
        ExecutorOp::OpenStream,
        ExecutorOp::Describe,
        ExecutorOp::Exec,
        ExecutorOp::Read,
        ExecutorOp::Write,
        ExecutorOp::Edit,
        ExecutorOp::Query,
        ExecutorOp::Cancel,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            ExecutorOp::OpenStream => "open_stream",
            ExecutorOp::Describe => "describe",
            ExecutorOp::Exec => "exec",
            ExecutorOp::Read => "read",
            ExecutorOp::Write => "write",
            ExecutorOp::Edit => "edit",
            ExecutorOp::Query => "query",
            ExecutorOp::Cancel => "cancel",
        }
    }

    pub fn rbac_action(&self) -> String {
        format!("executor.{}", self.as_str())
    }

    pub fn is_mutating(&self) -> bool {
        matches!(self, ExecutorOp::Exec | ExecutorOp::Write | ExecutorOp::Edit)
    }

    pub fn allowed_by(&self, mode: AgentPermissionMode) -> bool {
        mode == AgentPermissionMode::ReadWrite || !self.is_mutating()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorRunAs {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorProfile {
    /// OS identity a root Unix daemon switches to. Absent on hosts that run
    /// operations as their own user and cannot switch (Windows).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_as: Option<ExecutorRunAs>,
    pub workdir_roots: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub principals: Vec<String>,
    pub ops: Vec<ExecutorOp>,
}

impl ExecutorProfile {
    pub fn ops_set(&self) -> BTreeSet<ExecutorOp> {
        self.ops.iter().copied().collect()
    }

    pub fn validate(&self, profile_id: &str) -> Result<(), String> {
        if !is_valid_profile_id(profile_id) {
            return Err(format!("invalid executor profile id `{profile_id}`"));
        }
        if self.run_as.as_ref().is_some_and(|r| r.uid == 0 || r.gid == 0) {
            return Err(format!("profile `{profile_id}`: run_as must not be root"));
        }
        if self.workdir_roots.is_empty() {
            return Err(format!("profile `{profile_id}`: workdir_roots is empty"));
        }
        for root in &self.workdir_roots {
            if !is_absolute_node_path(root) || root.split(['/', '\\']).any(|c| c == "..") {
                return Err(format!(
                    "profile `{profile_id}`: workdir root `{root}` must be an absolute path"
                ));
            }
        }
        for key in self.env.keys() {
            if !is_valid_env_name(key) || is_forbidden_env_name(key) {
                return Err(format!("profile `{profile_id}`: env `{key}` is not allowed"));
            }
        }
        if self.env.values().any(|v| v.contains('\0')) {
            return Err(format!("profile `{profile_id}`: env value contains NUL"));
        }
        if self.principals.is_empty() {
            return Err(format!("profile `{profile_id}`: principals is empty"));
        }
        for principal in &self.principals {
            if !principal.starts_with("did:") || principal.contains('*') {
                return Err(format!(
                    "profile `{profile_id}`: principal `{principal}` must be an exact Agent DID"
                ));
            }
        }
        let ops = self.ops_set();
        if ops.len() != self.ops.len() {
            return Err(format!("profile `{profile_id}`: duplicated ops"));
        }
        if !ops.contains(&ExecutorOp::OpenStream) {
            return Err(format!("profile `{profile_id}`: ops must include open_stream"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExecutorLimits {
    pub max_streams: u32,
    pub max_concurrency: u32,
    pub max_execution_ms: u64,
    pub max_output_bytes_per_stream: u64,
    pub max_file_bytes: u64,
    pub result_retention_secs: u64,
    pub max_records: u64,
    pub max_store_bytes: u64,
}

impl Default for ExecutorLimits {
    fn default() -> Self {
        Self {
            max_streams: 4,
            max_concurrency: 2,
            max_execution_ms: 3_600_000,
            max_output_bytes_per_stream: 8 * 1024 * 1024,
            max_file_bytes: 1024 * 1024,
            result_retention_secs: 7 * 24 * 3600,
            max_records: 100_000,
            max_store_bytes: 2 * 1024 * 1024 * 1024,
        }
    }
}

impl ExecutorLimits {
    pub fn validate(&self) -> Result<(), String> {
        if self.max_streams == 0
            || self.max_concurrency == 0
            || self.max_execution_ms == 0
            || self.max_output_bytes_per_stream == 0
            || self.max_file_bytes == 0
            || self.result_retention_secs == 0
            || self.max_records == 0
            || self.max_store_bytes == 0
        {
            return Err("executor limits must all be positive".to_string());
        }
        Ok(())
    }
}

/// Administrator input at `nodes/<node>/settings` → `executor`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorSettings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_devices: Vec<String>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ExecutorProfile>,
    #[serde(default)]
    pub limits: ExecutorLimits,
}

impl ExecutorSettings {
    pub fn validate(&self) -> Result<(), String> {
        if !self.enabled {
            return Ok(());
        }
        if self.profiles.is_empty() {
            return Err("executor enabled without profiles".to_string());
        }
        validate_allowed_devices(&self.allowed_devices)?;
        for (profile_id, profile) in &self.profiles {
            profile.validate(profile_id)?;
        }
        self.limits.validate()
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NodeSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<ExecutorSettings>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorPrincipalBinding {
    pub app_instance_id: AppInstanceId,
    pub agent_generation: u64,
    pub binding_generation: u64,
    pub enabled: bool,
    pub app_installed: bool,
    pub permission_mode: AgentPermissionMode,
}

/// Scheduler-derived authorization snapshot at `nodes/<node>/config` → `executor`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutorConfig {
    pub enabled: bool,
    pub revision: String,
    pub generation: u64,
    pub auth_valid_until_ms: u64,
    #[serde(default)]
    pub allowed_devices: Vec<String>,
    #[serde(default)]
    pub principal_bindings: BTreeMap<String, ExecutorPrincipalBinding>,
    #[serde(default)]
    pub profiles: BTreeMap<String, ExecutorProfile>,
    #[serde(default)]
    pub limits: ExecutorLimits,
}

impl ExecutorConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.generation == 0 || !self.revision.starts_with("sha256:") {
            return Err("executor config lacks revision/generation".to_string());
        }
        if !self.enabled {
            return Ok(());
        }
        if self.profiles.is_empty() {
            return Err("executor enabled without profiles".to_string());
        }
        validate_allowed_devices(&self.allowed_devices)?;
        for (profile_id, profile) in &self.profiles {
            profile.validate(profile_id)?;
        }
        for (agent_did, binding) in &self.principal_bindings {
            if !agent_did.starts_with("did:") {
                return Err(format!("invalid principal binding key `{agent_did}`"));
            }
            if binding.agent_generation == 0 || binding.binding_generation == 0 {
                return Err(format!("principal `{agent_did}` binding lacks generation"));
            }
        }
        self.limits.validate()
    }

    pub fn is_open(&self) -> bool {
        self.enabled && !self.profiles.is_empty()
    }

    /// Operations an Agent may use on a profile, before RBAC and host capability.
    pub fn granted_ops(&self, profile_id: &str, agent_did: &str) -> BTreeSet<ExecutorOp> {
        let Some(profile) = self.profiles.get(profile_id) else {
            return BTreeSet::new();
        };
        let Some(binding) = self.principal_bindings.get(agent_did) else {
            return BTreeSet::new();
        };
        if !self.enabled
            || !binding.enabled
            || !binding.app_installed
            || !profile.principals.iter().any(|p| p == agent_did)
        {
            return BTreeSet::new();
        }
        profile
            .ops
            .iter()
            .copied()
            .filter(|op| op.allowed_by(binding.permission_mode))
            .collect()
    }
}

/// `sha256:<hex>` over the JSON encoding of an input made of ordered maps.
pub fn executor_revision<T: Serialize>(inputs: &T) -> String {
    let bytes = serde_json::to_vec(inputs).unwrap_or_default();
    format!("sha256:{}", hex_lower(&Sha256::digest(&bytes)))
}

pub(crate) fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

fn validate_allowed_devices(devices: &[String]) -> Result<(), String> {
    if devices.is_empty() {
        return Err("executor allowed_devices is empty".to_string());
    }
    for device in devices {
        if !device.starts_with("did:")
            || device.contains('*')
            || device.chars().any(|c| c.is_whitespace() || c == '"' || c == '\\')
        {
            return Err(format!("allowed device `{device}` must be an exact device DID"));
        }
    }
    Ok(())
}

/// Absolute path on a node of any supported OS: `/...` or `C:\...` / `C:/...`.
/// The node checks it again against its own path rules.
pub fn is_absolute_node_path(path: &str) -> bool {
    if path.contains('\0') {
        return false;
    }
    let b = path.as_bytes();
    path.starts_with('/')
        || (b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && matches!(b[2], b'\\' | b'/'))
}

pub fn is_valid_profile_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

fn is_valid_env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    matches!(bytes.next(), Some(b) if b.is_ascii_alphabetic() || b == b'_')
        && bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Identity and control-side variables never reach a worker.
pub fn is_forbidden_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    upper.starts_with("BUCKYOS_")
        || upper.starts_with("OPENDAN_")
        || upper.starts_with("XLLM_")
        || matches!(upper.as_str(), "SESSION_TOKEN" | "APP_TOKEN")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile() -> ExecutorProfile {
        ExecutorProfile {
            run_as: Some(ExecutorRunAs {
                uid: 1001,
                gid: 1001,
            }),
            workdir_roots: vec!["/home/owner/ws".into()],
            env: BTreeMap::from([("PATH".into(), "/usr/bin:/bin".into())]),
            principals: vec!["did:web:jarvis.test.buckyos.io".into()],
            ops: vec![ExecutorOp::OpenStream, ExecutorOp::Exec, ExecutorOp::Read],
        }
    }

    #[test]
    fn settings_reject_unknown_fields_and_open_profiles() {
        let raw = serde_json::json!({"enabled": true, "allowed_devices": ["did:dev:a"], "profiles": {}, "extra": 1});
        assert!(serde_json::from_value::<ExecutorSettings>(raw).is_err());

        let mut settings = ExecutorSettings {
            enabled: true,
            allowed_devices: vec!["did:dev:a".into()],
            profiles: BTreeMap::from([("dev".into(), profile())]),
            limits: ExecutorLimits::default(),
        };
        settings.validate().unwrap();

        settings.profiles.get_mut("dev").unwrap().run_as.as_mut().unwrap().uid = 0;
        assert!(settings.validate().is_err());
        settings.profiles.get_mut("dev").unwrap().run_as = None;
        settings.validate().unwrap();
        settings.profiles.get_mut("dev").unwrap().workdir_roots =
            vec![r"C:\Users\owner\ws".into(), "D:/data".into()];
        settings.validate().unwrap();
        for root in [r"C:\Users\..\ws", r"\\server\share", "C:ws", "ws"] {
            settings.profiles.get_mut("dev").unwrap().workdir_roots = vec![root.into()];
            assert!(settings.validate().is_err(), "{root}");
        }
        settings.profiles.get_mut("dev").unwrap().workdir_roots = vec!["/home/owner/ws".into()];
        settings.profiles.get_mut("dev").unwrap().principals = vec!["*".into()];
        assert!(settings.validate().is_err());
        settings.profiles.get_mut("dev").unwrap().principals =
            vec!["did:web:jarvis.test.buckyos.io".into()];
        settings
            .profiles
            .get_mut("dev")
            .unwrap()
            .env
            .insert("BUCKYOS_ROOT".into(), "/opt".into());
        assert!(settings.validate().is_err());
        settings.profiles.get_mut("dev").unwrap().env.clear();
        settings.allowed_devices.clear();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn read_only_agents_lose_mutating_ops() {
        let agent = "did:web:jarvis.test.buckyos.io".to_string();
        let mut config = ExecutorConfig {
            enabled: true,
            revision: "sha256:00".into(),
            generation: 1,
            auth_valid_until_ms: 1,
            allowed_devices: vec!["did:dev:a".into()],
            principal_bindings: BTreeMap::from([(
                agent.clone(),
                ExecutorPrincipalBinding {
                    app_instance_id: "jarvis.test.buckyos.io@devtest".parse().unwrap(),
                    agent_generation: 1,
                    binding_generation: 1,
                    enabled: true,
                    app_installed: true,
                    permission_mode: AgentPermissionMode::ReadOnly,
                },
            )]),
            profiles: BTreeMap::from([("dev".into(), profile())]),
            limits: ExecutorLimits::default(),
        };
        config.validate().unwrap();
        let ops = config.granted_ops("dev", &agent);
        assert!(ops.contains(&ExecutorOp::Read));
        assert!(!ops.contains(&ExecutorOp::Exec));

        config
            .principal_bindings
            .get_mut(&agent)
            .unwrap()
            .enabled = false;
        assert!(config.granted_ops("dev", &agent).is_empty());
    }

    #[test]
    fn node_config_without_executor_still_parses() {
        let raw = serde_json::json!({
            "node_id": "ood1", "node_did": "did:bns:ood1",
            "kernel": {}, "apps": {}, "frame_services": {}, "state": "running"
        });
        let config: crate::NodeConfig = serde_json::from_value(raw).unwrap();
        assert!(config.executor.is_none());
    }
}
