use crate::{
    AppDoc, AppServiceSpec, AppType, ServiceSpecConfig, ServiceState, SystemConfigClient,
    SystemConfigError, UserSettings, UserState, UserType,
};
use ::kRPC::{RPCErrors, RPCSessionToken};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;

pub const APP_AVAILABILITY_SCHEMA_VERSION: u32 = 1;
pub const APP_AVAILABILITY_POLICY_PREFIX: &str = "services/control_panel/app_availability/policies";
pub const APP_AVAILABILITY_AUDIT_PREFIX: &str = "services/control_panel/app_availability/audit";
pub const ZONE_APP_PREFIX: &str = "zone/apps";
pub const SYSTEM_APP_OWNER_ID: &str = "system";
pub const APP_INSTANCE_ID_CLAIM: &str = "app_instance_id";
pub const APP_OWNER_USER_ID_CLAIM: &str = "app_owner_user_id";
pub const TOKEN_PRINCIPAL_KIND_CLAIM: &str = "principal_kind";
pub const TOKEN_PRINCIPAL_KIND_USER: &str = "user";
pub const TOKEN_PRINCIPAL_KIND_DEVICE: &str = "device";
pub const TOKEN_PRINCIPAL_KIND_SERVICE: &str = "service";

const SYSTEM_LOGIN_TARGETS: &[&str] = &["control-panel", "kernel", "system-config"];

const SYSTEM_APP_AUTHOR: &str = "did:bns:buckyos";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AppClass {
    SystemBuiltin,
    UserInstalled,
    ZoneInstalled,
}

impl Default for AppClass {
    fn default() -> Self {
        Self::UserInstalled
    }
}

impl TryFrom<&str> for AppClass {
    type Error = &'static str;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "system_builtin" => Ok(Self::SystemBuiltin),
            "user_installed" => Ok(Self::UserInstalled),
            "zone_installed" => Ok(Self::ZoneInstalled),
            _ => Err("invalid app class"),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityEffect {
    Allow,
    Deny,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppAvailabilityGroupRule {
    pub group_id: String,
    pub effect: AvailabilityEffect,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppAvailabilityUserRule {
    pub user_id: String,
    pub effect: AvailabilityEffect,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppAvailabilityPolicy {
    pub schema_version: u32,
    pub app_instance_id: String,
    pub default_effect: AvailabilityEffect,
    pub group_rules: Vec<AppAvailabilityGroupRule>,
    pub user_rules: Vec<AppAvailabilityUserRule>,
    pub revision: u64,
    pub updated_by: String,
    pub updated_at: u64,
}

impl AppAvailabilityPolicy {
    pub fn owner_default(app_instance_id: impl Into<String>) -> Self {
        Self {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id: app_instance_id.into(),
            default_effect: AvailabilityEffect::Deny,
            group_rules: Vec::new(),
            user_rules: Vec::new(),
            revision: 0,
            updated_by: String::new(),
            updated_at: 0,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityMatchType {
    SystemBuiltin,
    Owner,
    ZoneAllUsers,
    Group,
    ExactUser,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AvailabilityMatch {
    #[serde(rename = "type")]
    pub match_type: AvailabilityMatchType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

impl AvailabilityMatch {
    fn new(match_type: AvailabilityMatchType, subject: Option<String>) -> Self {
        Self {
            match_type,
            subject,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AppAvailabilityDecision {
    pub allowed: bool,
    pub app_id: String,
    pub app_instance_id: String,
    pub app_class: AppClass,
    pub owner_user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub availability_match: Option<AvailabilityMatch>,
    pub reason: String,
}

#[derive(Clone, Debug)]
pub struct ResolvedAppInstallation {
    pub spec: AppServiceSpec,
    pub spec_path: String,
}

#[derive(Clone, Copy, Debug)]
pub struct SystemBuiltinAppDescriptor {
    pub app_id: &'static str,
    pub show_name: &'static str,
    pub icon_url: &'static str,
    pub description: &'static str,
    pub app_index: u16,
}

pub const SYSTEM_BUILTIN_APPS: &[SystemBuiltinAppDescriptor] = &[
    SystemBuiltinAppDescriptor {
        app_id: "messagehub",
        show_name: "Message Hub",
        icon_url: "res/messagehub/appicon.png",
        description: "BuckyOS 内置的统一消息中心",
        app_index: 100,
    },
    SystemBuiltinAppDescriptor {
        app_id: "homestation",
        show_name: "Home Station",
        icon_url: "res/homestation/appicon.png",
        description: "BuckyOS 内置的家庭门户",
        app_index: 101,
    },
    SystemBuiltinAppDescriptor {
        app_id: "content-store",
        show_name: "Content Store",
        icon_url: "res/content-store/appicon.png",
        description: "BuckyOS 内置的内容仓库",
        app_index: 102,
    },
];

pub fn find_system_builtin_app(app_id: &str) -> Option<&'static SystemBuiltinAppDescriptor> {
    SYSTEM_BUILTIN_APPS.iter().find(|app| app.app_id == app_id)
}

pub fn is_system_login_target(app_id: &str) -> bool {
    SYSTEM_LOGIN_TARGETS.contains(&app_id) || find_system_builtin_app(app_id).is_some()
}

pub fn bind_token_app_instance(
    token: &mut RPCSessionToken,
    app_instance_id: &str,
    owner_user_id: Option<&str>,
) {
    token.extra.insert(
        APP_INSTANCE_ID_CLAIM.to_string(),
        serde_json::Value::String(app_instance_id.to_string()),
    );
    if let Some(owner_user_id) = owner_user_id {
        token.extra.insert(
            APP_OWNER_USER_ID_CLAIM.to_string(),
            serde_json::Value::String(owner_user_id.to_string()),
        );
    }
}

pub fn token_app_instance_id(token: &RPCSessionToken) -> Result<&str, RPCErrors> {
    token
        .extra
        .get(APP_INSTANCE_ID_CLAIM)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RPCErrors::InvalidToken("missing app_instance_id claim".to_string()))
}

pub fn build_system_builtin_app_spec(
    app_id: &str,
    version: &str,
) -> Result<AppServiceSpec, RPCErrors> {
    let descriptor = find_system_builtin_app(app_id)
        .ok_or_else(|| RPCErrors::ReasonError(format!("unknown system built-in app `{app_id}`")))?;
    let owner = DID::from_str(SYSTEM_APP_AUTHOR).map_err(|error| {
        RPCErrors::ReasonError(format!("failed to build system owner DID: {error}"))
    })?;
    let app_doc = AppDoc::builder(
        AppType::Service,
        descriptor.app_id,
        version,
        SYSTEM_APP_AUTHOR,
        &owner,
    )
    .show_name(descriptor.show_name)
    .app_icon_url(descriptor.icon_url)
    .description_detail(descriptor.description)
    .build()
    .map_err(|error| {
        RPCErrors::ReasonError(format!(
            "failed to build system app doc `{app_id}`: {error}"
        ))
    })?;

    Ok(AppServiceSpec {
        permission: app_doc.permissions.clone(),
        app_doc,
        app_index: descriptor.app_index,
        user_id: SYSTEM_APP_OWNER_ID.to_string(),
        app_class: AppClass::SystemBuiltin,
        enable: true,
        expected_instance_count: 1,
        state: ServiceState::Running,
        spec_config: ServiceSpecConfig::default(),
    })
}

pub fn app_instance_id(app_id: &str, owner_user_id: &str) -> String {
    format!("{app_id}@{owner_user_id}")
}

pub fn parse_app_instance_id(value: &str) -> Result<(String, String), RPCErrors> {
    let value = value.trim();
    let (app_id, owner_user_id) = value.rsplit_once('@').ok_or_else(|| {
        RPCErrors::ParseRequestError("app_instance_id must be `<app_id>@<owner_user_id>`".into())
    })?;
    if app_id.is_empty() || owner_user_id.is_empty() {
        return Err(RPCErrors::ParseRequestError(
            "app_instance_id must contain non-empty app and owner ids".into(),
        ));
    }
    let valid_component = |component: &str, max_len: usize| {
        component.len() <= max_len
            && component
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
    };
    if !valid_component(app_id, 128) || !valid_component(owner_user_id, 64) {
        return Err(RPCErrors::ParseRequestError(
            "app_instance_id contains invalid app or owner characters".into(),
        ));
    }
    Ok((app_id.to_string(), owner_user_id.to_string()))
}

pub fn user_app_spec_key(owner_user_id: &str, app_id: &str) -> String {
    format!("users/{owner_user_id}/apps/{app_id}/spec")
}

pub fn zone_app_spec_key(app_id: &str) -> String {
    format!("{ZONE_APP_PREFIX}/{app_id}/spec")
}

pub fn app_availability_policy_key(app_instance_id: &str) -> String {
    format!("{APP_AVAILABILITY_POLICY_PREFIX}/{app_instance_id}")
}

pub fn app_availability_audit_key(app_instance_id: &str, revision: u64) -> String {
    format!("{APP_AVAILABILITY_AUDIT_PREFIX}/{app_instance_id}/{revision}")
}

pub fn system_group_for_user_type(user_type: &UserType) -> Option<&'static str> {
    match user_type {
        UserType::Admin => Some("admins"),
        UserType::User => Some("users"),
        UserType::Limited => Some("limited"),
        UserType::Guest => Some("guest"),
        UserType::Root => None,
    }
}

pub fn user_is_active(settings: &UserSettings) -> bool {
    matches!(settings.state, UserState::Active)
        && !matches!(settings.user_type, UserType::Root | UserType::Guest)
}

pub fn evaluate_app_availability(
    target_user: Option<&UserSettings>,
    target_user_id: &str,
    owner_settings: Option<&UserSettings>,
    installation: &ResolvedAppInstallation,
    policy: Option<&AppAvailabilityPolicy>,
) -> AppAvailabilityDecision {
    let spec = &installation.spec;
    let app_id = spec.app_id().to_string();
    let app_instance_id = spec.app_instance_id();
    let denied = |reason: &str| AppAvailabilityDecision {
        allowed: false,
        app_id: app_id.clone(),
        app_instance_id: app_instance_id.clone(),
        app_class: spec.app_class,
        owner_user_id: spec.user_id.clone(),
        availability_match: None,
        reason: reason.to_string(),
    };
    let allowed = |match_type, subject, reason: &str| AppAvailabilityDecision {
        allowed: true,
        app_id: app_id.clone(),
        app_instance_id: app_instance_id.clone(),
        app_class: spec.app_class,
        owner_user_id: spec.user_id.clone(),
        availability_match: Some(AvailabilityMatch::new(match_type, subject)),
        reason: reason.to_string(),
    };

    if matches!(spec.state, ServiceState::Deleted) {
        return denied("app_deleted");
    }

    let is_guest = target_user_id == "guest" && target_user.is_none();
    if !is_guest && !target_user.map(user_is_active).unwrap_or(false) {
        return denied("user_not_active");
    }

    match spec.app_class {
        AppClass::SystemBuiltin => {
            if is_guest {
                denied("guest_not_declared_public")
            } else {
                allowed(AvailabilityMatchType::SystemBuiltin, None, "system_builtin")
            }
        }
        AppClass::ZoneInstalled => {
            if is_guest {
                denied("zone_app_requires_login")
            } else {
                allowed(AvailabilityMatchType::ZoneAllUsers, None, "zone_all_users")
            }
        }
        AppClass::UserInstalled => {
            if !owner_settings.map(user_is_active).unwrap_or(false) {
                return denied("owner_not_active");
            }
            if !is_guest && target_user_id == spec.user_id {
                return allowed(
                    AvailabilityMatchType::Owner,
                    Some(target_user_id.to_string()),
                    "owner",
                );
            }
            let Some(policy) = policy else {
                return denied("default_deny");
            };
            if policy.schema_version != APP_AVAILABILITY_SCHEMA_VERSION
                || policy.app_instance_id != app_instance_id
                || policy.default_effect != AvailabilityEffect::Deny
                || validate_availability_rules(&policy.group_rules, &policy.user_rules).is_err()
            {
                return denied("invalid_policy");
            }
            if !is_guest {
                if let Some(rule) = policy
                    .user_rules
                    .iter()
                    .find(|rule| rule.user_id == target_user_id)
                {
                    return match rule.effect {
                        AvailabilityEffect::Allow => allowed(
                            AvailabilityMatchType::ExactUser,
                            Some(target_user_id.to_string()),
                            "exact_user_allow",
                        ),
                        AvailabilityEffect::Deny => denied("exact_user_deny"),
                    };
                }
            }

            let group_id = if is_guest {
                Some("guest")
            } else {
                target_user.and_then(|settings| system_group_for_user_type(&settings.user_type))
            };
            if let Some(group_id) = group_id {
                let matching = policy
                    .group_rules
                    .iter()
                    .filter(|rule| rule.group_id == group_id)
                    .collect::<Vec<_>>();
                if matching
                    .iter()
                    .any(|rule| rule.effect == AvailabilityEffect::Deny)
                {
                    return denied("group_deny");
                }
                if matching
                    .iter()
                    .any(|rule| rule.effect == AvailabilityEffect::Allow)
                {
                    return allowed(
                        AvailabilityMatchType::Group,
                        Some(group_id.to_string()),
                        "group_allow",
                    );
                }
            }

            match policy.default_effect {
                AvailabilityEffect::Allow => denied("invalid_default_effect"),
                AvailabilityEffect::Deny => denied("default_deny"),
            }
        }
    }
}

pub fn validate_availability_rules(
    group_rules: &[AppAvailabilityGroupRule],
    user_rules: &[AppAvailabilityUserRule],
) -> Result<(), RPCErrors> {
    const GROUPS: [&str; 4] = ["admins", "users", "limited", "guest"];
    let mut seen_groups = HashSet::new();
    for rule in group_rules {
        if !GROUPS.contains(&rule.group_id.as_str()) {
            return Err(RPCErrors::ParseRequestError(format!(
                "unknown availability group `{}`",
                rule.group_id
            )));
        }
        if !seen_groups.insert(rule.group_id.as_str()) {
            return Err(RPCErrors::ParseRequestError(format!(
                "duplicate availability group rule `{}`",
                rule.group_id
            )));
        }
    }
    let mut seen_users = HashSet::new();
    for rule in user_rules {
        if rule.user_id.trim().is_empty() || rule.user_id == "guest" || rule.user_id == "root" {
            return Err(RPCErrors::ParseRequestError(format!(
                "invalid availability user `{}`",
                rule.user_id
            )));
        }
        if !seen_users.insert(rule.user_id.as_str()) {
            return Err(RPCErrors::ParseRequestError(format!(
                "duplicate availability user rule `{}`",
                rule.user_id
            )));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct AppAvailabilityResolver {
    client: Arc<SystemConfigClient>,
    system_version: String,
}

impl AppAvailabilityResolver {
    pub fn new(client: Arc<SystemConfigClient>, system_version: impl Into<String>) -> Self {
        Self {
            client,
            system_version: system_version.into(),
        }
    }

    pub async fn get_user_settings(&self, user_id: &str) -> Result<UserSettings, RPCErrors> {
        let key = format!("users/{user_id}/settings");
        let value = self.client.get(&key).await.map_err(|error| {
            RPCErrors::ReasonError(format!("user `{user_id}` not found: {error}"))
        })?;
        let settings: UserSettings = serde_json::from_str(&value.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid user settings for `{user_id}`: {error}"))
        })?;
        if !user_is_active(&settings) {
            return Err(RPCErrors::NoPermission(format!(
                "user `{user_id}` is not active"
            )));
        }
        Ok(settings)
    }

    pub async fn resolve_installation(
        &self,
        app_instance_id: &str,
    ) -> Result<ResolvedAppInstallation, RPCErrors> {
        let (app_id, owner_user_id) = parse_app_instance_id(app_instance_id)?;
        if owner_user_id == SYSTEM_APP_OWNER_ID {
            if find_system_builtin_app(&app_id).is_some() {
                return Ok(ResolvedAppInstallation {
                    spec: build_system_builtin_app_spec(&app_id, &self.system_version)?,
                    spec_path: format!("system/apps/{app_id}/spec"),
                });
            }
            let spec_path = zone_app_spec_key(&app_id);
            let value = self.client.get(&spec_path).await.map_err(|error| {
                RPCErrors::ReasonError(format!(
                    "app instance `{app_instance_id}` not found: {error}"
                ))
            })?;
            let spec: AppServiceSpec = serde_json::from_str(&value.value).map_err(|error| {
                RPCErrors::ReasonError(format!("invalid app spec `{spec_path}`: {error}"))
            })?;
            if spec.app_class != AppClass::ZoneInstalled
                || spec.app_instance_id() != app_instance_id
            {
                return Err(RPCErrors::ReasonError(format!(
                    "zone app spec does not match `{app_instance_id}`"
                )));
            }
            return Ok(ResolvedAppInstallation { spec, spec_path });
        }

        let spec_path = user_app_spec_key(&owner_user_id, &app_id);
        let value = self.client.get(&spec_path).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "app instance `{app_instance_id}` not found: {error}"
            ))
        })?;
        let spec: AppServiceSpec = serde_json::from_str(&value.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid app spec `{spec_path}`: {error}"))
        })?;
        if spec.app_class != AppClass::UserInstalled || spec.app_instance_id() != app_instance_id {
            return Err(RPCErrors::ReasonError(format!(
                "user app spec does not match `{app_instance_id}`"
            )));
        }
        Ok(ResolvedAppInstallation { spec, spec_path })
    }

    pub async fn load_policy(
        &self,
        app_instance_id: &str,
    ) -> Result<Option<(AppAvailabilityPolicy, u64)>, RPCErrors> {
        let key = app_availability_policy_key(app_instance_id);
        match self.client.get(&key).await {
            Ok(value) => {
                let policy: AppAvailabilityPolicy =
                    serde_json::from_str(&value.value).map_err(|error| {
                        RPCErrors::ReasonError(format!(
                            "invalid availability policy `{key}`: {error}"
                        ))
                    })?;
                if policy.schema_version != APP_AVAILABILITY_SCHEMA_VERSION
                    || policy.app_instance_id != app_instance_id
                    || policy.default_effect != AvailabilityEffect::Deny
                    || validate_availability_rules(&policy.group_rules, &policy.user_rules).is_err()
                {
                    return Err(RPCErrors::ReasonError(format!(
                        "invalid availability policy `{key}`"
                    )));
                }
                Ok(Some((policy, value.version)))
            }
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(error) => Err(RPCErrors::ReasonError(error.to_string())),
        }
    }

    pub async fn check_user(
        &self,
        user_id: &str,
        app_instance_id: &str,
    ) -> Result<AppAvailabilityDecision, RPCErrors> {
        let target_user = self.get_user_settings(user_id).await?;
        let installation = self.resolve_installation(app_instance_id).await?;
        let owner_settings = if installation.spec.app_class == AppClass::UserInstalled {
            self.get_user_settings(&installation.spec.user_id)
                .await
                .ok()
        } else {
            None
        };
        let policy = self
            .load_policy(app_instance_id)
            .await?
            .map(|(policy, _)| policy);
        Ok(evaluate_app_availability(
            Some(&target_user),
            user_id,
            owner_settings.as_ref(),
            &installation,
            policy.as_ref(),
        ))
    }

    pub async fn check_guest(
        &self,
        app_instance_id: &str,
    ) -> Result<AppAvailabilityDecision, RPCErrors> {
        let installation = self.resolve_installation(app_instance_id).await?;
        let owner_settings = if installation.spec.app_class == AppClass::UserInstalled {
            self.get_user_settings(&installation.spec.user_id)
                .await
                .ok()
        } else {
            None
        };
        let policy = self
            .load_policy(app_instance_id)
            .await?
            .map(|(policy, _)| policy);
        Ok(evaluate_app_availability(
            None,
            "guest",
            owner_settings.as_ref(),
            &installation,
            policy.as_ref(),
        ))
    }

    pub async fn list_user_installations(
        &self,
        user_id: &str,
    ) -> Result<Vec<(ResolvedAppInstallation, AppAvailabilityDecision)>, RPCErrors> {
        let target_user = self.get_user_settings(user_id).await?;
        let mut candidates = BTreeMap::<String, ResolvedAppInstallation>::new();

        for descriptor in SYSTEM_BUILTIN_APPS {
            let spec = build_system_builtin_app_spec(descriptor.app_id, &self.system_version)?;
            candidates.insert(
                spec.app_instance_id(),
                ResolvedAppInstallation {
                    spec,
                    spec_path: format!("system/apps/{}/spec", descriptor.app_id),
                },
            );
        }

        for app_id in list_children_or_empty(&self.client, ZONE_APP_PREFIX).await? {
            let instance_id = app_instance_id(&app_id, SYSTEM_APP_OWNER_ID);
            if let Ok(installation) = self.resolve_installation(&instance_id).await {
                candidates.insert(instance_id, installation);
            }
        }

        for owner_user_id in list_children_or_empty(&self.client, "users").await? {
            let app_root = format!("users/{owner_user_id}/apps");
            for app_id in list_children_or_empty(&self.client, &app_root).await? {
                let instance_id = app_instance_id(&app_id, &owner_user_id);
                if let Ok(installation) = self.resolve_installation(&instance_id).await {
                    candidates.insert(instance_id, installation);
                }
            }
        }

        let mut result = Vec::new();
        for (instance_id, installation) in candidates {
            if installation.spec.app_doc.get_app_type() == AppType::Agent {
                continue;
            }
            let owner_settings = if installation.spec.app_class == AppClass::UserInstalled {
                self.get_user_settings(&installation.spec.user_id)
                    .await
                    .ok()
            } else {
                None
            };
            let policy = self
                .load_policy(&instance_id)
                .await?
                .map(|(policy, _)| policy);
            let decision = evaluate_app_availability(
                Some(&target_user),
                user_id,
                owner_settings.as_ref(),
                &installation,
                policy.as_ref(),
            );
            if decision.allowed {
                result.push((installation, decision));
            }
        }
        Ok(result)
    }
}

async fn list_children_or_empty(
    client: &SystemConfigClient,
    key: &str,
) -> Result<Vec<String>, RPCErrors> {
    match client.list(key).await {
        Ok(items) => Ok(items),
        Err(SystemConfigError::KeyNotFound(_)) => Ok(Vec::new()),
        Err(error) => Err(RPCErrors::ReasonError(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(user_id: &str, user_type: UserType, state: UserState) -> UserSettings {
        UserSettings {
            user_id: user_id.to_string(),
            user_type,
            password: String::new(),
            state,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: Some(true),
        }
    }

    fn installation(owner: &str) -> ResolvedAppInstallation {
        let owner_did = DID::new("bns", owner);
        let app_doc = AppDoc::builder(AppType::Service, "notes", "1.0.0", owner, &owner_did)
            .build()
            .unwrap();
        ResolvedAppInstallation {
            spec: AppServiceSpec {
                permission: Vec::new(),
                app_doc,
                app_index: 1,
                user_id: owner.to_string(),
                app_class: AppClass::UserInstalled,
                enable: true,
                expected_instance_count: 1,
                state: ServiceState::Running,
                spec_config: ServiceSpecConfig::default(),
            },
            spec_path: user_app_spec_key(owner, "notes"),
        }
    }

    #[test]
    fn exact_user_rule_overrides_group_rule() {
        let owner = settings("alice", UserType::User, UserState::Active);
        let bob = settings("bob", UserType::User, UserState::Active);
        let app = installation("alice");
        let policy = AppAvailabilityPolicy {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id: "notes@alice".to_string(),
            default_effect: AvailabilityEffect::Deny,
            group_rules: vec![AppAvailabilityGroupRule {
                group_id: "users".to_string(),
                effect: AvailabilityEffect::Allow,
            }],
            user_rules: vec![AppAvailabilityUserRule {
                user_id: "bob".to_string(),
                effect: AvailabilityEffect::Deny,
            }],
            revision: 1,
            updated_by: "alice".to_string(),
            updated_at: 1,
        };
        let decision =
            evaluate_app_availability(Some(&bob), "bob", Some(&owner), &app, Some(&policy));
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "exact_user_deny");

        let mut allow_exact = policy.clone();
        allow_exact.group_rules[0].effect = AvailabilityEffect::Deny;
        allow_exact.user_rules[0].effect = AvailabilityEffect::Allow;
        let decision =
            evaluate_app_availability(Some(&bob), "bob", Some(&owner), &app, Some(&allow_exact));
        assert!(decision.allowed);
        assert_eq!(
            decision.availability_match.unwrap().match_type,
            AvailabilityMatchType::ExactUser
        );
    }

    #[test]
    fn owner_is_always_allowed_but_inactive_owner_blocks_shares() {
        let active_owner = settings("alice", UserType::User, UserState::Active);
        let bob = settings("bob", UserType::User, UserState::Active);
        let app = installation("alice");
        let owner_decision = evaluate_app_availability(
            Some(&active_owner),
            "alice",
            Some(&active_owner),
            &app,
            None,
        );
        assert!(owner_decision.allowed);

        let banned_owner = settings(
            "alice",
            UserType::User,
            UserState::Banned("test".to_string()),
        );
        let policy = AppAvailabilityPolicy {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id: "notes@alice".to_string(),
            default_effect: AvailabilityEffect::Deny,
            group_rules: vec![AppAvailabilityGroupRule {
                group_id: "users".to_string(),
                effect: AvailabilityEffect::Allow,
            }],
            user_rules: Vec::new(),
            revision: 1,
            updated_by: "alice".to_string(),
            updated_at: 1,
        };
        let shared_decision =
            evaluate_app_availability(Some(&bob), "bob", Some(&banned_owner), &app, Some(&policy));
        assert!(!shared_decision.allowed);
        assert_eq!(shared_decision.reason, "owner_not_active");
    }

    #[test]
    fn duplicate_or_untrusted_rules_are_rejected() {
        assert!(validate_availability_rules(
            &[
                AppAvailabilityGroupRule {
                    group_id: "users".into(),
                    effect: AvailabilityEffect::Allow,
                },
                AppAvailabilityGroupRule {
                    group_id: "users".into(),
                    effect: AvailabilityEffect::Deny,
                },
            ],
            &[],
        )
        .is_err());
        assert!(validate_availability_rules(
            &[AppAvailabilityGroupRule {
                group_id: "profile-admins".into(),
                effect: AvailabilityEffect::Allow,
            }],
            &[],
        )
        .is_err());
    }

    #[test]
    fn app_instance_id_rejects_path_components() {
        assert_eq!(
            parse_app_instance_id("demo-app@alice").unwrap(),
            ("demo-app".to_string(), "alice".to_string())
        );
        assert!(parse_app_instance_id("../../spec@alice").is_err());
        assert!(parse_app_instance_id("demo@app/owner").is_err());
    }

    #[test]
    fn system_and_zone_apps_allow_active_users_but_not_guest() {
        let bob = settings("bob", UserType::User, UserState::Active);
        let system = ResolvedAppInstallation {
            spec: build_system_builtin_app_spec("messagehub", "1.0.0").unwrap(),
            spec_path: "system/apps/messagehub/spec".to_string(),
        };
        let system_decision = evaluate_app_availability(Some(&bob), "bob", None, &system, None);
        assert!(system_decision.allowed);
        assert_eq!(
            system_decision.availability_match.unwrap().match_type,
            AvailabilityMatchType::SystemBuiltin
        );
        assert!(!evaluate_app_availability(None, "guest", None, &system, None).allowed);

        let mut zone = installation(SYSTEM_APP_OWNER_ID);
        zone.spec.app_class = AppClass::ZoneInstalled;
        let zone_decision = evaluate_app_availability(Some(&bob), "bob", None, &zone, None);
        assert!(zone_decision.allowed);
        assert_eq!(
            zone_decision.availability_match.unwrap().match_type,
            AvailabilityMatchType::ZoneAllUsers
        );
    }

    #[test]
    fn guest_access_is_driven_only_by_guest_policy() {
        let owner = settings("alice", UserType::User, UserState::Active);
        let app = installation("alice");
        let policy = AppAvailabilityPolicy {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id: "notes@alice".to_string(),
            default_effect: AvailabilityEffect::Deny,
            group_rules: vec![AppAvailabilityGroupRule {
                group_id: "guest".to_string(),
                effect: AvailabilityEffect::Allow,
            }],
            user_rules: Vec::new(),
            revision: 1,
            updated_by: "alice".to_string(),
            updated_at: 1,
        };
        let decision = evaluate_app_availability(None, "guest", Some(&owner), &app, Some(&policy));
        assert!(decision.allowed);
        assert_eq!(
            decision.availability_match.unwrap(),
            AvailabilityMatch::new(AvailabilityMatchType::Group, Some("guest".to_string()))
        );
    }

    #[test]
    fn deleted_apps_and_inactive_users_are_denied() {
        let owner = settings("alice", UserType::User, UserState::Active);
        let banned = settings("bob", UserType::User, UserState::Banned("test".to_string()));
        let guest_account = settings("guest", UserType::Guest, UserState::Active);
        let mut app = installation("alice");
        let decision = evaluate_app_availability(Some(&banned), "bob", Some(&owner), &app, None);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "user_not_active");

        let decision =
            evaluate_app_availability(Some(&guest_account), "guest", Some(&owner), &app, None);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "user_not_active");

        app.spec.state = ServiceState::Deleted;
        let decision = evaluate_app_availability(Some(&owner), "alice", Some(&owner), &app, None);
        assert!(!decision.allowed);
        assert_eq!(decision.reason, "app_deleted");
    }
}
