use crate::app_mgr::*;
use crate::system_config::*;
use crate::{
    default_contact_projection_rules, user_group_config_key, validate_user_group_id, AppDoc,
    AppInstanceId, AppType, ContactProjectionConfig, ContactProjectionRule, SelectorType,
    UserGroup, CONTACT_PROJECTIONS_CONFIG_KEY, USER_GROUPS_CONFIG_PREFIX,
};
use ::kRPC::*;
use log::warn;
use name_lib::{DIDDocumentTrait, DeviceDocument, DeviceInfo, EncodedDocument, OwnerDocument, DID};
pub use name_lib::{
    ProfileContact, ProfileLink, ProfilePrivacyRule, ProfileVisibility, UserPrivateProfile,
    UserProfile, UserProfilePrivacy,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub const CONTROL_PANEL_SERVICE_NAME: &str = "control-panel";
pub const CONTROL_PANEL_SERVICE_UNIQUE_ID: &str = "control-panel";
pub const CONTROL_PANEL_SERVICE_PORT: u16 = 4020;

const DEVICE_DOC_RETRY_DELAY_MS: u64 = 500;

fn is_transient_system_config_error(error: &SystemConfigError) -> bool {
    matches!(
        error,
        SystemConfigError::ReasonError(_) | SystemConfigError::Timeout(_)
    )
}

/// Map a system_config read failure to RPCErrors without losing its category:
/// only a real missing key becomes `KeyNotExist`.
fn system_config_read_error(path: &str, error: SystemConfigError) -> RPCErrors {
    match error {
        SystemConfigError::KeyNotFound(_) => RPCErrors::KeyNotExist(path.to_string()),
        SystemConfigError::NoPermission(reason) => {
            RPCErrors::NoPermission(format!("read {} failed: {}", path, reason))
        }
        other => RPCErrors::ReasonError(format!("read {} failed: {}", path, other)),
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(try_from = "String", into = "String")]
pub enum UserState {
    Active,
    Pending,
    Suspended(String), //suspend reason
    Deleted,           //delete reason
    Banned(String),    //ban reason
}

impl TryFrom<String> for UserState {
    type Error = &'static str;
    fn try_from(value: String) -> std::result::Result<Self, Self::Error> {
        let split_result = value.split(":").collect::<Vec<&str>>();
        let state_str = split_result[0];
        let reason = split_result.get(1).unwrap_or(&"");
        match state_str {
            "active" => Ok(UserState::Active),
            "pending" => Ok(UserState::Pending),
            "suspended" => Ok(UserState::Suspended(reason.to_string())),
            "deleted" => Ok(UserState::Deleted),
            "banned" => Ok(UserState::Banned(reason.to_string())),
            _ => Err("Invalid user state"),
        }
    }
}

impl From<UserState> for String {
    fn from(value: UserState) -> Self {
        match value {
            UserState::Active => "active".to_string(),
            UserState::Pending => "pending".to_string(),
            UserState::Suspended(reason) => format!("suspended:{}", reason),
            UserState::Deleted => "deleted".to_string(),
            UserState::Banned(reason) => format!("banned:{}", reason),
        }
    }
}

/// The mutually exclusive RBAC group of an account record
/// (product/users_and_agents/UserType.md). `Root` marks the Zone owner record
/// and never logs in; anonymous visitors and External users have no record.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum UserType {
    Admin,
    User,
    Root,
    Limited,
}

impl UserType {
    /// Types an admin may create, invite or change an account to.
    pub const ASSIGNABLE: [UserType; 3] = [UserType::Admin, UserType::User, UserType::Limited];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::User => "user",
            Self::Root => "root",
            Self::Limited => "limited",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "admin" => Some(Self::Admin),
            "user" => Some(Self::User),
            "root" => Some(Self::Root),
            "limited" => Some(Self::Limited),
            _ => None,
        }
    }

    pub fn is_privileged(self) -> bool {
        matches!(self, Self::Admin | Self::Root)
    }

    /// The RBAC role group scheduler puts the account in; Root has none.
    pub fn rbac_group(self) -> Option<&'static str> {
        match self {
            Self::Admin => Some("admin"),
            Self::User => Some("users"),
            Self::Limited => Some("limited"),
            Self::Root => None,
        }
    }

    pub fn can_login(self) -> bool {
        !matches!(self, Self::Root)
    }

    pub fn is_assignable(self) -> bool {
        !matches!(self, Self::Root)
    }

    pub fn can_create_agent(self) -> bool {
        !matches!(self, Self::Limited)
    }

    /// Admin and User accounts own a personal msg-center inbox and are local
    /// recipients; Limited accounts take part through group sessions only.
    pub fn can_receive_direct_messages(self) -> bool {
        matches!(self, Self::Admin | Self::User)
    }

    /// Only Admin and User accounts may be vouched for to other Zones.
    pub fn can_federate_out(self) -> bool {
        matches!(self, Self::Admin | Self::User)
    }

    pub fn default_can_change_password(self) -> bool {
        !matches!(self, Self::Limited)
    }

    pub fn default_can_edit_contacts(self) -> bool {
        !matches!(self, Self::Limited)
    }
}

impl std::fmt::Display for UserType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UserTunnelBinding {
    pub platform: String,
    pub account_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_id: Option<String>,
    /// Stable tunnel instance id (e.g. `tg-main-tunnel`), NOT the transport DID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tunnel_instance_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_sync_at: Option<u64>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub meta: HashMap<String, String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UserContactSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub did: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Contact tags the Zone suggests for this user. Authorization groups are
    /// system user groups (`UserSettings.groups`), not contact settings.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<UserTunnelBinding>,
}

//did:bns:$user_id user_id is-> UserSettings
#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserSettings {
    pub user_id: String,
    //rename to type
    #[serde(rename = "type")]
    pub user_type: UserType,
    pub password: String,
    pub state: UserState,
    pub res_pool_id: String,
    #[serde(default)]
    pub is_local: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_password_change: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow_contact_edit: Option<bool>,
    /// System user group ids (`system/user_groups/<id>`); only admins change them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    #[serde(default, skip_serializing_if = "PasswordState::is_set")]
    pub password_state: PasswordState,
}

/// Whether the account has a password. `Unset` accounts (exclusive invites
/// before the first password) keep an empty `password` and never pass a
/// password check; the state is explicit, not a placeholder password.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PasswordState {
    #[default]
    Set,
    Unset,
}

impl PasswordState {
    pub fn is_set(&self) -> bool {
        matches!(self, Self::Set)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct UserInfo {
    pub show_name: String,
    pub user_id: String,
    pub state: UserState,
    pub user_type: UserType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
}

impl UserSettings {
    pub fn to_user_info(&self) -> UserInfo {
        UserInfo {
            show_name: self.user_id.clone(),
            user_id: self.user_id.clone(),
            state: self.state.clone(),
            user_type: self.user_type,
            groups: self.groups.clone(),
        }
    }

    /// An active account other than the Zone owner record.
    pub fn is_active(&self) -> bool {
        matches!(self.state, UserState::Active) && self.user_type.can_login()
    }

    pub fn can_change_password(&self) -> bool {
        self.allow_password_change
            .unwrap_or_else(|| self.user_type.default_can_change_password())
    }

    pub fn can_edit_contacts(&self) -> bool {
        self.allow_contact_edit
            .unwrap_or_else(|| self.user_type.default_can_edit_contacts())
    }

    /// The UserType group and the system user groups the account belongs to.
    pub fn group_refs(&self) -> Vec<crate::UserGroupRef> {
        if !self.user_type.is_assignable() {
            return Vec::new();
        }
        let mut refs = vec![crate::UserGroupRef::Type(self.user_type)];
        for group_id in &self.groups {
            let group = crate::UserGroupRef::Group(group_id.clone());
            if !refs.contains(&group) {
                refs.push(group);
            }
        }
        refs
    }
}

/// Whom a user session speaks for (T-4 of the user type definition).
#[derive(Clone, Debug)]
pub enum ZoneUserPrincipal {
    Account(UserSettings),
    /// Authenticated, but this Zone keeps no account record for the DID.
    External(DID),
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct UserGroupDirectory {
    pub groups: Vec<UserGroup>,
    /// `None` until an admin saved rules; [`Self::projection_rules`] then
    /// returns the defaults.
    pub projections: Option<ContactProjectionConfig>,
}

impl UserGroupDirectory {
    pub fn defined_group_ids(&self) -> std::collections::HashSet<String> {
        self.groups
            .iter()
            .map(|group| group.group_id.clone())
            .collect()
    }

    pub fn projection_rules(&self) -> Vec<ContactProjectionRule> {
        self.projections
            .as_ref()
            .map(|config| config.rules.clone())
            .unwrap_or_else(default_contact_projection_rules)
    }
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum NodeState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Restarting,
    Initializing,
    Removed,
    Maintenance,
    Repalcing,
}

//load from SystemConfig,node的配置分为下面几个部分
// 固定的硬件配置，一般只有硬件改变或损坏才会修改
// 系统资源情况，（比如可用内存等），改变密度很大。这一块暂时不用etcd实现，而是用专门的监控服务保存
// RunItem的配置。这块由调度器改变，一旦改变,node_daemon就会产生相应的控制命令
// Task(Cmd)配置，暂时不实现
#[derive(Serialize, Deserialize)]
pub struct NodeConfig {
    //pub pure_version: u64,
    pub node_id: String,
    pub node_did: String,
    pub kernel: HashMap<String, KernelServiceInstanceConfig>,
    pub apps: HashMap<AppInstanceId, AppServiceInstanceConfig>,
    pub frame_services: HashMap<String, FrameServiceInstanceConfig>,
    pub state: NodeState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executor: Option<crate::ExecutorConfig>,
}

impl NodeConfig {
    pub fn is_running(&self) -> bool {
        matches!(self.state, NodeState::Running)
    }
}

pub struct ControlPanelClient {
    system_config_client: Arc<SystemConfigClient>,
}

impl ControlPanelClient {
    pub fn new(system_config_client: SystemConfigClient) -> Self {
        Self {
            system_config_client: Arc::new(system_config_client),
        }
    }

    pub fn from_shared(system_config_client: Arc<SystemConfigClient>) -> Self {
        Self {
            system_config_client,
        }
    }

    pub async fn set_context(&self, context: RPCContext) -> SytemConfigResult<()> {
        self.system_config_client.set_context(context).await
    }

    //return (rbac_model,rbac_policy)
    pub async fn load_rbac_config(&self) -> Result<(String, String)> {
        let rbac_config =
            crate::load_current_rbac_config(self.system_config_client.as_ref()).await?;
        Ok((rbac_config.model, rbac_config.policy))
    }

    pub async fn load_zone_config(&self) -> Result<ZoneConfig> {
        let zone_config_path = "boot/config";
        let zone_config_result = self.system_config_client.get(zone_config_path).await;
        if zone_config_result.is_err() {
            return Err(RPCErrors::ReasonError(format!(
                "get boot config(Zone config) failed:{}",
                zone_config_result.err().unwrap()
            )));
        }
        let zone_config_result = zone_config_result.unwrap();
        let zone_config: ZoneConfig = serde_json::from_str(&zone_config_result.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(zone_config)
    }

    pub async fn get_device_info(&self, device_id: &str) -> Result<DeviceInfo> {
        let device_info_path = format!("devices/{}/info", device_id);
        let get_result = self
            .system_config_client
            .get(device_info_path.as_str())
            .await;
        if get_result.is_err() {
            return Err(RPCErrors::ReasonError("Device info not found".to_string()));
        }

        let get_result = get_result.unwrap();
        let device_info: DeviceInfo = serde_json::from_str(&get_result.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(device_info)
    }

    /// Returns `RPCErrors::KeyNotExist` only when the device doc is really absent.
    /// Transport / timeout failures are retried once and then surface as
    /// `RPCErrors::ReasonError`, so callers can tell "no such device" from
    /// "system_config could not be read this time".
    pub async fn get_device_config(&self, device_id: &str) -> Result<DeviceDocument> {
        let device_doc_path = format!("devices/{}/doc", device_id);
        let mut get_result = self
            .system_config_client
            .get(device_doc_path.as_str())
            .await;
        if let Err(error) = &get_result {
            if is_transient_system_config_error(error) {
                warn!(
                    "load device doc {} failed, retrying once: {}",
                    device_doc_path, error
                );
                tokio::time::sleep(std::time::Duration::from_millis(DEVICE_DOC_RETRY_DELAY_MS))
                    .await;
                get_result = self
                    .system_config_client
                    .get(device_doc_path.as_str())
                    .await;
            }
        }
        let get_result =
            get_result.map_err(|error| system_config_read_error(&device_doc_path, error))?;

        let device_doc: EncodedDocument = EncodedDocument::from_str(get_result.value.clone())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        let device_doc: DeviceDocument = DeviceDocument::decode(&device_doc, None)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

        Ok(device_doc)
    }

    pub async fn get_user_config(&self, user_id: &str) -> Result<OwnerDocument> {
        let user_doc_path = format!("users/{}/doc", user_id);
        let get_result = self
            .system_config_client
            .get(user_doc_path.as_str())
            .await
            .map_err(|error| system_config_read_error(&user_doc_path, error))?;

        let user_doc: OwnerDocument = serde_json::from_str(&get_result.value)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

        Ok(user_doc)
    }

    /// The account's DID: the id of its OwnerDocument (`users/<id>/doc`), the
    /// only binding the account holder cannot rewrite; never `profile.did`.
    pub async fn get_user_did(&self, user_id: &str) -> Result<DID> {
        let key = format!("users/{user_id}/doc");
        let value = self
            .system_config_client
            .get(&key)
            .await
            .map_err(|error| system_config_read_error(&key, error))?;
        user_did_from_doc_value(&key, &value.value)
    }

    pub async fn get_user_list(&self) -> Result<Vec<String>> {
        let user_list = self.system_config_client.list("users").await;
        if user_list.is_err() {
            return Err(RPCErrors::ReasonError("user list not found".to_string()));
        }
        Ok(user_list.unwrap())
    }

    pub async fn get_user_settings_by_username(&self, username: &str) -> Result<UserSettings> {
        let user_info_path = format!("users/{}/settings", username);
        let user_info = self.system_config_client.get(user_info_path.as_str()).await;
        if user_info.is_err() {
            return Err(RPCErrors::ReasonError("user info not found".to_string()));
        }
        let user_info = user_info.unwrap();
        let user_info: UserSettings = serde_json::from_str(&user_info.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(user_info)
    }

    /// Load every user group definition and the contact projection rules
    /// (defaults while none were saved).
    pub async fn load_user_groups(&self) -> Result<UserGroupDirectory> {
        let mut groups = Vec::new();
        let ids = match self
            .system_config_client
            .list(USER_GROUPS_CONFIG_PREFIX)
            .await
        {
            Ok(ids) => ids,
            Err(SystemConfigError::KeyNotFound(_)) => Vec::new(),
            Err(error) => return Err(system_config_read_error(USER_GROUPS_CONFIG_PREFIX, error)),
        };
        for group_id in ids {
            if validate_user_group_id(&group_id).is_err() {
                continue;
            }
            let key = user_group_config_key(&group_id);
            let value = self
                .system_config_client
                .get(&key)
                .await
                .map_err(|error| system_config_read_error(&key, error))?;
            let group: UserGroup = serde_json::from_str(&value.value).map_err(|error| {
                RPCErrors::ReasonError(format!("invalid user group `{key}`: {error}"))
            })?;
            groups.push(group);
        }
        groups.sort_by(|left, right| left.group_id.cmp(&right.group_id));
        let projections = match self
            .system_config_client
            .get(CONTACT_PROJECTIONS_CONFIG_KEY)
            .await
        {
            Ok(value) => Some(
                serde_json::from_str::<ContactProjectionConfig>(&value.value).map_err(|error| {
                    RPCErrors::ReasonError(format!(
                        "invalid `{CONTACT_PROJECTIONS_CONFIG_KEY}`: {error}"
                    ))
                })?,
            ),
            Err(SystemConfigError::KeyNotFound(_)) => None,
            Err(error) => {
                return Err(system_config_read_error(
                    CONTACT_PROJECTIONS_CONFIG_KEY,
                    error,
                ))
            }
        };
        Ok(UserGroupDirectory {
            groups,
            projections,
        })
    }

    /// Resolve whom an ordinary user session token speaks for: an account
    /// record of this Zone, or an External DID (no record).
    pub async fn resolve_user_principal(
        &self,
        token: &RPCSessionToken,
    ) -> Result<ZoneUserPrincipal> {
        let subject = token
            .sub
            .as_deref()
            .ok_or_else(|| RPCErrors::InvalidToken("missing sub".to_string()))?;
        match crate::token_principal_kind(token)? {
            crate::TokenPrincipalKind::User if crate::token_is_external(token) => {
                let did = DID::from_str(subject).map_err(|error| {
                    RPCErrors::InvalidToken(format!("External subject is not a DID: {error}"))
                })?;
                Ok(ZoneUserPrincipal::External(did))
            }
            crate::TokenPrincipalKind::User => {
                let key = format!("users/{subject}/settings");
                let value = self
                    .system_config_client
                    .get(&key)
                    .await
                    .map_err(|error| system_config_read_error(&key, error))?;
                let settings: UserSettings =
                    serde_json::from_str(&value.value).map_err(|error| {
                        RPCErrors::InvalidToken(format!("unusable account record `{key}`: {error}"))
                    })?;
                Ok(ZoneUserPrincipal::Account(settings))
            }
            kind => Err(RPCErrors::InvalidToken(format!(
                "principal kind `{}` is not a user",
                kind.as_str()
            ))),
        }
    }

    pub async fn update_service_instance_info(
        &self,
        service_name: &str,
        node_name: &str,
        instance_info: &ServiceInstanceReportInfo,
    ) -> Result<u64> {
        let service_info_path = format!("services/{}/instances/{}", service_name, node_name);
        let service_info_str = serde_json::to_string(instance_info).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "serialize service instance info {}@{} failed: {}",
                service_name, node_name, error
            ))
        })?;
        self.system_config_client
            .set(service_info_path.as_str(), service_info_str.as_str())
            .await
            .map_err(|e| {
                RPCErrors::ReasonError(format!(
                    "update service instance info {}@{} failed, err:{}",
                    service_name, node_name, e
                ))
            })?;
        Ok(0)
    }

    pub async fn get_services_info(&self, service_name: &str) -> Result<ServiceInfo> {
        let service_info_path = format!("services/{}/info", service_name);
        let service_info = self
            .system_config_client
            .get(service_info_path.as_str())
            .await;
        if service_info.is_err() {
            return Err(RPCErrors::ServiceNotValid(
                "service info not found".to_string(),
            ));
        }
        let service_info = service_info.unwrap();
        let service_info: ServiceInfo = serde_json::from_str(&service_info.value)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(service_info)
    }
}

/// DID of a stored `users/<id>/doc` (an OwnerDocument JSON, `id` or `did`).
pub fn user_did_from_doc_value(key: &str, raw: &str) -> Result<DID> {
    let value: serde_json::Value = serde_json::from_str(raw)
        .map_err(|error| RPCErrors::ReasonError(format!("invalid `{key}`: {error}")))?;
    let did = value
        .get("id")
        .or_else(|| value.get("did"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RPCErrors::ReasonError(format!("`{key}` has no id")))?;
    DID::from_str(did).map_err(|error| RPCErrors::ReasonError(format!("`{key}` id: {error}")))
}

pub fn generate_control_panel_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        CONTROL_PANEL_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("System Control Panel")
    .selector_type(SelectorType::Single)
    .build()
    .unwrap()
}

#[cfg(test)]
mod system_config_read_error_tests {
    use super::*;

    #[test]
    fn only_missing_key_maps_to_key_not_exist() {
        let path = "devices/ood1/doc";
        assert!(matches!(
            system_config_read_error(path, SystemConfigError::KeyNotFound(path.to_string())),
            RPCErrors::KeyNotExist(p) if p == path
        ));
        assert!(matches!(
            system_config_read_error(path, SystemConfigError::ReasonError("timeout".to_string())),
            RPCErrors::ReasonError(msg) if msg.contains(path) && msg.contains("timeout")
        ));
        assert!(matches!(
            system_config_read_error(path, SystemConfigError::NoPermission("denied".to_string())),
            RPCErrors::NoPermission(_)
        ));
    }

    #[test]
    fn transient_errors_are_retryable() {
        assert!(is_transient_system_config_error(
            &SystemConfigError::ReasonError("rpc timeout".to_string())
        ));
        assert!(is_transient_system_config_error(
            &SystemConfigError::Timeout("15s".to_string())
        ));
        assert!(!is_transient_system_config_error(
            &SystemConfigError::KeyNotFound("devices/ood1/doc".to_string())
        ));
        assert!(!is_transient_system_config_error(
            &SystemConfigError::NoPermission("denied".to_string())
        ));
    }
}
