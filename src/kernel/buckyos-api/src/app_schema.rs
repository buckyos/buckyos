use crate::{
    AgentId, AppId, AppInstanceId, AppType, DeploymentIdentity, PermissionItem, ServiceSpecConfig,
    SubPkgDesc,
};
use name_lib::{AgentDocument, DID};
use ndn_lib::{build_named_object_by_json, ObjId};
use package_lib::PackageId;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const APP_REGISTRY_SCHEMA_VERSION: u32 = 1;
pub const AGENT_SPEC_SCHEMA_VERSION: u32 = 1;
pub const NODE_EXECUTION_SPEC_SCHEMA_VERSION: u32 = 1;
pub const APP_REGISTRY_KEY: &str = "system/app_registry";
pub const ZONE_OWNER_USER_ID_KEY: &str = "system/zone_owner_user_id";
pub const MAX_ALLOCATABLE_APP_INDEX: u16 = 3470;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppAllocation {
    pub app_did: DID,
    pub app_name: String,
    pub allocated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppInstanceAllocation {
    pub app_id: AppId,
    pub owner_user_id: String,
    pub app_host_name: String,
    pub app_index: u16,
    pub allocated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppRegistry {
    pub schema_version: u32,
    pub next_app_index: u32,
    pub apps: BTreeMap<AppId, AppAllocation>,
    pub instances: BTreeMap<AppInstanceId, AppInstanceAllocation>,
    pub updated_at: u64,
}

impl Default for AppRegistry {
    fn default() -> Self {
        Self {
            schema_version: APP_REGISTRY_SCHEMA_VERSION,
            next_app_index: 1,
            apps: BTreeMap::new(),
            instances: BTreeMap::new(),
            updated_at: 0,
        }
    }
}

impl AppRegistry {
    pub fn validate(&self, reserved_hostnames: &BTreeSet<String>) -> Result<(), String> {
        if self.schema_version != APP_REGISTRY_SCHEMA_VERSION {
            return Err(format!(
                "unsupported AppRegistry schema_version {}",
                self.schema_version
            ));
        }
        if self.next_app_index == 0 || self.next_app_index > MAX_ALLOCATABLE_APP_INDEX as u32 + 1 {
            return Err("AppRegistry next_app_index is outside the allocatable range".into());
        }

        let mut app_names = BTreeMap::<&str, &AppId>::new();
        for (app_id, allocation) in &self.apps {
            if AppId::from_app_did(&allocation.app_did).as_ref() != Ok(app_id) {
                return Err(format!(
                    "AppRegistry app key `{app_id}` does not match app_did"
                ));
            }
            validate_dns_label(&allocation.app_name)?;
            if reserved_hostnames.contains(&allocation.app_name) {
                return Err(format!("reserved app_name `{}`", allocation.app_name));
            }
            if app_names.insert(&allocation.app_name, app_id).is_some() {
                return Err(format!("duplicate app_name `{}`", allocation.app_name));
            }
        }

        let mut hostnames = BTreeMap::<&str, &AppInstanceId>::new();
        let mut indexes = BTreeSet::new();
        for (instance_id, allocation) in &self.instances {
            if instance_id.app_id() != &allocation.app_id
                || instance_id.owner_user_id() != allocation.owner_user_id
            {
                return Err(format!(
                    "AppRegistry instance key `{instance_id}` does not match allocation"
                ));
            }
            let app = self.apps.get(&allocation.app_id).ok_or_else(|| {
                format!("AppRegistry instance `{instance_id}` references a missing app")
            })?;
            validate_dns_label(&allocation.app_host_name)?;
            if reserved_hostnames.contains(&allocation.app_host_name) {
                return Err(format!(
                    "reserved app_host_name `{}`",
                    allocation.app_host_name
                ));
            }
            if let Some(other_app_id) = app_names.get(allocation.app_host_name.as_str()) {
                if *other_app_id != &allocation.app_id || allocation.app_host_name != app.app_name {
                    return Err(format!(
                        "app_host_name `{}` conflicts with an AppName",
                        allocation.app_host_name
                    ));
                }
            }
            if hostnames
                .insert(&allocation.app_host_name, instance_id)
                .is_some()
            {
                return Err(format!(
                    "duplicate app_host_name `{}`",
                    allocation.app_host_name
                ));
            }
            if allocation.app_index == 0
                || allocation.app_index > MAX_ALLOCATABLE_APP_INDEX
                || !indexes.insert(allocation.app_index)
            {
                return Err(format!(
                    "invalid or duplicate app_index {}",
                    allocation.app_index
                ));
            }
        }
        Ok(())
    }

    pub fn allocate(
        &mut self,
        app_did: &DID,
        owner_user_id: &str,
        zone_owner_user_id: &str,
        reserved_hostnames: &BTreeSet<String>,
        allocated_at: u64,
    ) -> Result<(AppInstanceId, AppInstanceAllocation), String> {
        self.validate(reserved_hostnames)?;
        let app_id = AppId::from_app_did(app_did)?;
        if !self.apps.contains_key(&app_id) {
            let app_name = self.allocate_app_name(app_did, &app_id, reserved_hostnames)?;
            self.apps.insert(
                app_id.clone(),
                AppAllocation {
                    app_did: app_did.clone(),
                    app_name,
                    allocated_at,
                },
            );
        }
        let instance_id = AppInstanceId::new(app_id.clone(), owner_user_id)?;
        if let Some(allocation) = self.instances.get(&instance_id) {
            return Ok((instance_id, allocation.clone()));
        }
        if self.next_app_index > MAX_ALLOCATABLE_APP_INDEX as u32 {
            return Err("AppIndex capacity exhausted".into());
        }

        let app_name = self.apps[&app_id].app_name.clone();
        let owner_label = owner_dns_label(owner_user_id);
        let preferred = if owner_user_id == zone_owner_user_id {
            app_name.clone()
        } else {
            fit_dns_label(
                &format!("{app_name}-{owner_label}"),
                instance_id.to_string().as_bytes(),
            )
        };
        let app_host_name =
            self.first_available_hostname(preferred, &instance_id, &app_id, reserved_hostnames);
        let allocation = AppInstanceAllocation {
            app_id,
            owner_user_id: owner_user_id.to_string(),
            app_host_name,
            app_index: self.next_app_index as u16,
            allocated_at,
        };
        self.next_app_index += 1;
        self.updated_at = allocated_at;
        self.instances
            .insert(instance_id.clone(), allocation.clone());
        self.validate(reserved_hostnames)?;
        Ok((instance_id, allocation))
    }

    fn allocate_app_name(
        &self,
        app_did: &DID,
        app_id: &AppId,
        reserved_hostnames: &BTreeSet<String>,
    ) -> Result<String, String> {
        let labels: Vec<&str> = app_id.as_str().split('.').collect();
        for count in 1..=labels.len() {
            let candidate =
                fit_dns_label(&labels[..count].join("-"), app_did.to_string().as_bytes());
            if self.hostname_is_available(&candidate, app_id, reserved_hostnames) {
                return Ok(candidate);
            }
        }
        let candidate = with_hash_suffix(labels[0], app_did.to_string().as_bytes());
        if self.hostname_is_available(&candidate, app_id, reserved_hostnames) {
            return Ok(candidate);
        }
        Err("stable AppName hash candidate unexpectedly conflicts".into())
    }

    fn first_available_hostname(
        &self,
        preferred: String,
        instance_id: &AppInstanceId,
        app_id: &AppId,
        reserved_hostnames: &BTreeSet<String>,
    ) -> String {
        if self.instance_hostname_is_available(&preferred, instance_id, app_id, reserved_hostnames)
        {
            return preferred;
        }
        with_hash_suffix(&preferred, instance_id.to_string().as_bytes())
    }

    fn hostname_is_available(
        &self,
        candidate: &str,
        app_id: &AppId,
        reserved_hostnames: &BTreeSet<String>,
    ) -> bool {
        !reserved_hostnames.contains(candidate)
            && !self
                .apps
                .iter()
                .any(|(id, allocation)| id != app_id && allocation.app_name == candidate)
            && !self
                .instances
                .values()
                .any(|allocation| allocation.app_host_name == candidate)
    }

    fn instance_hostname_is_available(
        &self,
        candidate: &str,
        instance_id: &AppInstanceId,
        app_id: &AppId,
        reserved_hostnames: &BTreeSet<String>,
    ) -> bool {
        !reserved_hostnames.contains(candidate)
            && !self
                .apps
                .iter()
                .any(|(id, allocation)| allocation.app_name == candidate && id != app_id)
            && !self
                .instances
                .iter()
                .any(|(id, allocation)| allocation.app_host_name == candidate && id != instance_id)
    }
}

fn validate_dns_label(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 63
        || value.starts_with('-')
        || value.ends_with('-')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!("invalid DNS label `{value}`"));
    }
    Ok(())
}

fn owner_dns_label(owner_user_id: &str) -> String {
    let mut label = String::with_capacity(owner_user_id.len());
    for byte in owner_user_id.bytes() {
        let value = if byte.is_ascii_lowercase() || byte.is_ascii_digit() {
            byte as char
        } else {
            '-'
        };
        if value != '-' || !label.ends_with('-') {
            label.push(value);
        }
    }
    let label = label.trim_matches('-');
    let label = if label.is_empty() { "user" } else { label };
    fit_dns_label(label, owner_user_id.as_bytes())
}

fn fit_dns_label(value: &str, hash_material: &[u8]) -> String {
    if validate_dns_label(value).is_ok() {
        value.to_string()
    } else {
        with_hash_suffix(value, hash_material)
    }
}

fn with_hash_suffix(base: &str, material: &[u8]) -> String {
    let digest = Sha256::digest(material);
    let suffix = digest[..5]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let normalized = base
        .bytes()
        .map(|byte| {
            if byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' {
                byte as char
            } else {
                '-'
            }
        })
        .collect::<String>();
    let normalized = normalized.trim_matches('-');
    let normalized = if normalized.is_empty() {
        "app"
    } else {
        normalized
    };
    let prefix_len = 63usize.saturating_sub(suffix.len() + 1);
    let prefix = normalized[..normalized.len().min(prefix_len)].trim_end_matches('-');
    format!("{prefix}-{suffix}")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentServiceBinding {
    pub schema_version: u32,
    pub agent_did: DID,
    pub agent_doc_object_id: ObjId,
    pub target_app_instance_id: AppInstanceId,
    pub service_name: String,
    pub generation: u64,
}

impl AgentServiceBinding {
    pub fn references_runtime(&self, app_instance_id: &AppInstanceId) -> bool {
        &self.target_app_instance_id == app_instance_id
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentSpec {
    pub schema_version: u32,
    pub agent_id: AgentId,
    pub agent_did: DID,
    pub agent_doc_object_id: ObjId,
    pub agent_doc: AgentDocument,
    pub binding: AgentServiceBinding,
    pub generation: u64,
}

impl AgentSpec {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != AGENT_SPEC_SCHEMA_VERSION
            || self.binding.schema_version != AGENT_SPEC_SCHEMA_VERSION
            || self.generation == 0
            || self.binding.generation == 0
        {
            return Err("unsupported or invalid AgentSpec generation/schema".into());
        }
        if self.agent_id != AgentId::from_agent_did(&self.agent_did)?
            || self.agent_doc.id != self.agent_did
            || self.binding.agent_did != self.agent_did
            || self.binding.agent_doc_object_id != self.agent_doc_object_id
            || self.binding.service_name.is_empty()
        {
            return Err("AgentSpec identity or binding is inconsistent".into());
        }
        let agent_doc_value = serde_json::to_value(&self.agent_doc)
            .map_err(|error| format!("cannot canonicalize AgentDocument snapshot: {error}"))?;
        let (actual_object_id, _) = build_named_object_by_json("agentdoc", &agent_doc_value);
        if actual_object_id != self.agent_doc_object_id {
            return Err("AgentDocument snapshot does not match agent_doc_object_id".into());
        }
        Ok(())
    }
}

pub fn agent_spec_key(owner_user_id: &str, agent_id: &AgentId) -> String {
    format!("users/{owner_user_id}/agents/{agent_id}/spec")
}

pub fn agent_key_path(owner_user_id: &str, agent_id: &AgentId) -> String {
    format!("users/{owner_user_id}/agents/{agent_id}/key")
}

pub fn agent_settings_key(owner_user_id: &str, agent_id: &AgentId) -> String {
    format!("users/{owner_user_id}/agents/{agent_id}/settings")
}

pub fn agent_profile_key(owner_user_id: &str, agent_id: &AgentId) -> String {
    format!("users/{owner_user_id}/agents/{agent_id}/profile")
}

pub fn agent_info_key(owner_user_id: &str, agent_id: &AgentId) -> String {
    format!("users/{owner_user_id}/agents/{agent_id}/info")
}

pub fn agent_template_record_key(owner_user_id: &str, app_id: &AppId) -> String {
    format!("users/{owner_user_id}/agent_templates/{app_id}")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentInstallState {
    Provisioning,
    Bound,
    Ready,
    Failed,
    Removed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentCreateStep {
    Runtime,
    Bind,
    Tunnel,
    Start,
    Done,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentTunnelState {
    #[default]
    None,
    Pending,
    Bound,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTemplateSource {
    Bundled,
    Installed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentCreateError {
    pub step: AgentCreateStep,
    pub code: String,
    pub message: String,
    pub retryable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentInstallRecord {
    pub schema_version: u32,
    pub agent_id: AgentId,
    pub agent_doc_object_id: ObjId,
    pub target_app_instance_id: AppInstanceId,
    pub service_name: String,
    pub generation: u64,
    pub state: AgentInstallState,
    pub step: AgentCreateStep,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<AgentCreateError>,
    pub idempotency_key: String,
    pub request_fingerprint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_spec: Option<AgentSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runtime_task_id: Option<String>,
    #[serde(default)]
    pub tunnel_state: AgentTunnelState,
    pub template_id: String,
    pub template_source: AgentTemplateSource,
    pub template_app_did: DID,
    pub template_app_doc_object_id: ObjId,
    pub template_version: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentMsgTunnelSettings {
    pub platform: String,
    pub bot_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bot_account_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub auto_start: bool,
    #[serde(default)]
    pub allow_other_users: bool,
    #[serde(default)]
    pub allow_group: bool,
    #[serde(default)]
    pub role_supplement: String,
    #[serde(default = "default_true")]
    pub template_auto_update: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop_entry: Option<String>,
    #[serde(default)]
    pub msg_tunnels: Vec<AgentMsgTunnelSettings>,
}

impl Default for AgentSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            auto_start: true,
            allow_other_users: false,
            allow_group: false,
            role_supplement: String::new(),
            template_auto_update: true,
            desktop_entry: None,
            msg_tunnels: Vec::new(),
        }
    }
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bio: Option<String>,
}

impl AgentProfile {
    pub fn resolved_display_name(&self, agent_id: &AgentId) -> String {
        self.display_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| agent_short_name(agent_id))
    }
}

pub fn agent_short_name(agent_id: &AgentId) -> String {
    agent_id
        .as_str()
        .split('.')
        .next()
        .unwrap_or(agent_id.as_str())
        .to_string()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentRuntimeInfo {
    pub agent_doc_object_id: ObjId,
    pub generation: u64,
    pub loaded_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_version: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentTemplateRecord {
    pub app_did: DID,
    pub app_doc_object_id: ObjId,
    pub version: String,
    pub digest: String,
    pub pikg_path: String,
    pub installed_at: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeploymentPackage {
    pub sub_pkg_name: String,
    pub pkg_id: String,
    pub package_meta_object_id: ObjId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docker_image_digest: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeExecutionSpec {
    pub schema_version: u32,
    pub app_instance_id: AppInstanceId,
    pub app_did: DID,
    pub app_doc_object_id: ObjId,
    pub spec_generation: u64,
    pub app_type: AppType,
    pub packages: BTreeMap<String, SubPkgDesc>,
    pub permission: Vec<PermissionItem>,
    pub service_spec_config: ServiceSpecConfig,
    pub app_name: String,
    pub app_host_name: String,
    pub app_index: u16,
}

impl NodeExecutionSpec {
    pub fn validate_against(&self, deployment: &DeploymentIdentity) -> Result<(), String> {
        if self.schema_version != NODE_EXECUTION_SPEC_SCHEMA_VERSION
            || self.app_instance_id != deployment.app_instance_id
            || self.app_doc_object_id != deployment.app_doc_object_id
            || self.spec_generation != deployment.spec_generation
            || AppId::from_app_did(&self.app_did).as_ref() != Ok(self.app_instance_id.app_id())
        {
            return Err("NodeExecutionSpec and DeploymentIdentity are inconsistent".into());
        }
        for (sub_pkg_name, package) in &self.packages {
            let package_meta_object_id = package.pkg_objid.as_ref().ok_or_else(|| {
                format!("execution package `{sub_pkg_name}` has no Package Meta ObjectId")
            })?;
            let package_id = PackageId::parse(&package.pkg_id).map_err(|error| {
                format!("execution package `{sub_pkg_name}` has invalid PackageId: {error}")
            })?;
            if package_id.objid.as_deref() != Some(package_meta_object_id.to_string().as_str()) {
                return Err(format!(
                    "execution package `{sub_pkg_name}` does not pin its Package Meta ObjectId"
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reserved() -> BTreeSet<String> {
        ["_", "www", "sys"]
            .into_iter()
            .map(ToOwned::to_owned)
            .collect()
    }

    #[test]
    fn registry_allocates_stable_owner_scoped_instances() {
        let mut registry = AppRegistry::default();
        let did = DID::from_str("did:web:filebrowser.buckyos.ai").unwrap();
        let (alice_id, alice) = registry
            .allocate(&did, "alice", "alice", &reserved(), 10)
            .unwrap();
        let (bob_id, bob) = registry
            .allocate(&did, "bob", "alice", &reserved(), 11)
            .unwrap();
        assert_eq!(alice_id.to_string(), "filebrowser.buckyos.ai@alice");
        assert_eq!(alice.app_host_name, "filebrowser");
        assert_eq!(bob.app_host_name, "filebrowser-bob");
        assert_ne!(alice.app_index, bob.app_index);
        assert_ne!(alice_id, bob_id);
        let (_, again) = registry
            .allocate(&did, "alice", "alice", &reserved(), 12)
            .unwrap();
        assert_eq!(again, alice);
    }

    #[test]
    fn registry_rejects_unknown_schema_and_index_exhaustion() {
        let mut registry = AppRegistry::default();
        registry.schema_version = 2;
        assert!(registry.validate(&reserved()).is_err());

        let mut registry = AppRegistry::default();
        registry.next_app_index = MAX_ALLOCATABLE_APP_INDEX as u32 + 1;
        let did = DID::from_str("did:web:another.example.com").unwrap();
        assert!(registry
            .allocate(&did, "alice", "alice", &reserved(), 1)
            .is_err());
        assert_eq!(
            crate::BASE_APP_PORT as u32 + MAX_ALLOCATABLE_APP_INDEX as u32 * 16,
            65_520
        );
    }

    #[test]
    fn multiple_agents_can_share_one_runtime_binding() {
        let runtime = "opendan.buckyos.ai@alice".parse::<AppInstanceId>().unwrap();
        let object_id = ObjId::new("agentdoc:0123456789abcdef").unwrap();
        let bindings =
            ["did:web:jarvis.example.com", "did:web:helper.example.com"].map(|agent_did| {
                AgentServiceBinding {
                    schema_version: AGENT_SPEC_SCHEMA_VERSION,
                    agent_did: DID::from_str(agent_did).unwrap(),
                    agent_doc_object_id: object_id.clone(),
                    target_app_instance_id: runtime.clone(),
                    service_name: "www".to_string(),
                    generation: 1,
                }
            });

        assert!(bindings
            .iter()
            .all(|binding| binding.references_runtime(&runtime)));
        assert_ne!(bindings[0].agent_did, bindings[1].agent_did);
    }
}
