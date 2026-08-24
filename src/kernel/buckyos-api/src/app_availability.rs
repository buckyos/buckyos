use crate::{
    AppId, AppInstanceId, AppServiceSpec, AuthTarget, ServiceState, SystemConfigClient,
    SystemConfigError, SystemServiceId, UserSettings, UserState, UserType,
};
use ::kRPC::{RPCErrors, RPCSessionToken};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use std::str::FromStr;
use std::sync::Arc;

pub const APP_AVAILABILITY_SCHEMA_VERSION: u32 = 1;
pub const APP_AVAILABILITY_POLICY_PREFIX: &str = "services/control_panel/app_availability/policies";
pub const APP_AVAILABILITY_AUDIT_PREFIX: &str = "services/control_panel/app_availability/audit";
pub const APP_INSTANCE_ID_CLAIM: &str = "app_instance_id";
pub const APP_OWNER_USER_ID_CLAIM: &str = "app_owner_user_id";
pub const TOKEN_PRINCIPAL_KIND_CLAIM: &str = "principal_kind";
pub const TOKEN_TARGET_KIND_CLAIM: &str = "target_kind";
pub const TOKEN_USE_CLAIM: &str = "token_use";
pub const VERIFY_HUB_TOKEN_ISSUER: &str = "verify-hub";
pub const TOKEN_PRINCIPAL_KIND_USER: &str = "user";
pub const TOKEN_PRINCIPAL_KIND_DEVICE: &str = "device";
pub const TOKEN_PRINCIPAL_KIND_APP: &str = "app";
pub const TOKEN_PRINCIPAL_KIND_SYSTEM: &str = "system";
pub const TOKEN_PRINCIPAL_KIND_AGENT: &str = "agent";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenPrincipalKind {
    User,
    Device,
    App,
    System,
    Agent,
}

impl TokenPrincipalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => TOKEN_PRINCIPAL_KIND_USER,
            Self::Device => TOKEN_PRINCIPAL_KIND_DEVICE,
            Self::App => TOKEN_PRINCIPAL_KIND_APP,
            Self::System => TOKEN_PRINCIPAL_KIND_SYSTEM,
            Self::Agent => TOKEN_PRINCIPAL_KIND_AGENT,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TokenUse {
    Session,
    Refresh,
}

impl TokenUse {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session",
            Self::Refresh => "refresh",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidatedTokenClaims {
    pub principal_kind: TokenPrincipalKind,
    pub token_use: TokenUse,
    pub target: AuthTarget,
}

const SYSTEM_LOGIN_TARGETS: &[&str] = &["control-panel", "kernel", "system-config"];

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AvailabilityEffect {
    Allow,
    Deny,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppAvailabilityGroupRule {
    pub group_id: String,
    pub effect: AvailabilityEffect,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppAvailabilityUserRule {
    pub user_id: String,
    pub effect: AvailabilityEffect,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppAvailabilityPolicy {
    pub schema_version: u32,
    pub app_instance_id: AppInstanceId,
    pub default_effect: AvailabilityEffect,
    pub group_rules: Vec<AppAvailabilityGroupRule>,
    pub user_rules: Vec<AppAvailabilityUserRule>,
    pub revision: u64,
    pub updated_by: String,
    pub updated_at: u64,
}

impl AppAvailabilityPolicy {
    pub fn owner_default(app_instance_id: AppInstanceId) -> Self {
        Self {
            schema_version: APP_AVAILABILITY_SCHEMA_VERSION,
            app_instance_id,
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
    Owner,
    ZoneAllUsers,
    Group,
    ExactUser,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AvailabilityMatch {
    #[serde(rename = "type")]
    pub match_type: AvailabilityMatchType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct AppAvailabilityDecision {
    pub allowed: bool,
    pub app_id: AppId,
    pub app_instance_id: AppInstanceId,
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
    pub service_id: &'static str,
    pub show_name: &'static str,
    pub icon_url: &'static str,
    pub description: &'static str,
}

pub const SYSTEM_BUILTIN_APPS: &[SystemBuiltinAppDescriptor] = &[
    SystemBuiltinAppDescriptor {
        service_id: "messagehub",
        show_name: "Message Hub",
        icon_url: "res/messagehub/appicon.png",
        description: "BuckyOS 内置的统一消息中心",
    },
    SystemBuiltinAppDescriptor {
        service_id: "homestation",
        show_name: "Home Station",
        icon_url: "res/homestation/appicon.png",
        description: "BuckyOS 内置的家庭门户",
    },
    SystemBuiltinAppDescriptor {
        service_id: "content-store",
        show_name: "Content Store",
        icon_url: "res/content-store/appicon.png",
        description: "BuckyOS 内置的内容仓库",
    },
];

pub fn find_system_builtin_app(service_id: &str) -> Option<&'static SystemBuiltinAppDescriptor> {
    SYSTEM_BUILTIN_APPS
        .iter()
        .find(|app| app.service_id == service_id)
}

pub fn is_system_login_target(service_id: &str) -> bool {
    SYSTEM_LOGIN_TARGETS.contains(&service_id) || find_system_builtin_app(service_id).is_some()
}

pub fn bind_token_principal_kind(token: &mut RPCSessionToken, principal_kind: TokenPrincipalKind) {
    token.extra.insert(
        TOKEN_PRINCIPAL_KIND_CLAIM.to_string(),
        serde_json::Value::String(principal_kind.as_str().to_string()),
    );
}

pub fn token_principal_kind(token: &RPCSessionToken) -> Result<TokenPrincipalKind, RPCErrors> {
    match token
        .extra
        .get(TOKEN_PRINCIPAL_KIND_CLAIM)
        .and_then(serde_json::Value::as_str)
    {
        Some(TOKEN_PRINCIPAL_KIND_USER) => Ok(TokenPrincipalKind::User),
        Some(TOKEN_PRINCIPAL_KIND_DEVICE) => Ok(TokenPrincipalKind::Device),
        Some(TOKEN_PRINCIPAL_KIND_APP) => Ok(TokenPrincipalKind::App),
        Some(TOKEN_PRINCIPAL_KIND_SYSTEM) => Ok(TokenPrincipalKind::System),
        Some(TOKEN_PRINCIPAL_KIND_AGENT) => Ok(TokenPrincipalKind::Agent),
        _ => Err(RPCErrors::InvalidToken(
            "missing or invalid principal_kind claim".to_string(),
        )),
    }
}

pub fn bind_token_target(
    token: &mut RPCSessionToken,
    target: &AuthTarget,
    token_use: TokenUse,
) -> Result<(), RPCErrors> {
    if token_use == TokenUse::Refresh && token.sudo {
        return Err(RPCErrors::InvalidToken(
            "refresh token cannot be a sudo token".to_string(),
        ));
    }

    token.appid = Some(target.appid_claim().to_string());
    token.extra.remove(APP_INSTANCE_ID_CLAIM);
    token.extra.remove(APP_OWNER_USER_ID_CLAIM);
    token.extra.insert(
        TOKEN_USE_CLAIM.to_string(),
        serde_json::Value::String(token_use.as_str().to_string()),
    );
    match target {
        AuthTarget::App { app_instance_id } => {
            token.extra.insert(
                TOKEN_TARGET_KIND_CLAIM.to_string(),
                serde_json::Value::String("app".to_string()),
            );
            token.extra.insert(
                APP_INSTANCE_ID_CLAIM.to_string(),
                serde_json::Value::String(app_instance_id.to_string()),
            );
            token.extra.insert(
                APP_OWNER_USER_ID_CLAIM.to_string(),
                serde_json::Value::String(app_instance_id.owner_user_id().to_string()),
            );
        }
        AuthTarget::System { .. } => {
            token.extra.insert(
                TOKEN_TARGET_KIND_CLAIM.to_string(),
                serde_json::Value::String("system".to_string()),
            );
        }
    }
    Ok(())
}

pub fn token_use(token: &RPCSessionToken) -> Result<TokenUse, RPCErrors> {
    let value = token
        .extra
        .get(TOKEN_USE_CLAIM)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RPCErrors::InvalidToken("missing token_use claim".to_string()))?;
    let token_use = match value {
        "session" => TokenUse::Session,
        "refresh" => TokenUse::Refresh,
        _ => {
            return Err(RPCErrors::InvalidToken(
                "invalid token_use claim".to_string(),
            ));
        }
    };
    if token_use == TokenUse::Refresh && token.sudo {
        return Err(RPCErrors::InvalidToken(
            "refresh token cannot be a sudo token".to_string(),
        ));
    }
    Ok(token_use)
}

pub fn token_auth_target(token: &RPCSessionToken) -> Result<AuthTarget, RPCErrors> {
    let appid = token
        .appid
        .as_deref()
        .ok_or_else(|| RPCErrors::InvalidToken("missing appid claim".to_string()))?;
    let target_kind = token
        .extra
        .get(TOKEN_TARGET_KIND_CLAIM)
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| RPCErrors::InvalidToken("missing target_kind claim".to_string()))?;
    match target_kind {
        "app" => {
            let instance_value = token
                .extra
                .get(APP_INSTANCE_ID_CLAIM)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    RPCErrors::InvalidToken("missing app_instance_id claim".to_string())
                })?;
            let app_instance_id = AppInstanceId::from_str(instance_value).map_err(|error| {
                RPCErrors::InvalidToken(format!("invalid app_instance_id claim: {error}"))
            })?;
            if app_instance_id.app_id().as_str() != appid {
                return Err(RPCErrors::InvalidToken(
                    "appid and app_instance_id claims do not match".to_string(),
                ));
            }
            let owner = token
                .extra
                .get(APP_OWNER_USER_ID_CLAIM)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| {
                    RPCErrors::InvalidToken("missing app_owner_user_id claim".to_string())
                })?;
            if owner != app_instance_id.owner_user_id() {
                return Err(RPCErrors::InvalidToken(
                    "app_owner_user_id and app_instance_id claims do not match".to_string(),
                ));
            }
            Ok(AuthTarget::app(app_instance_id))
        }
        "system" => {
            if token.extra.contains_key(APP_INSTANCE_ID_CLAIM)
                || token.extra.contains_key(APP_OWNER_USER_ID_CLAIM)
            {
                return Err(RPCErrors::InvalidToken(
                    "system target cannot contain app instance claims".to_string(),
                ));
            }
            let service_id = SystemServiceId::parse(appid).map_err(|error| {
                RPCErrors::InvalidToken(format!("invalid system service target: {error}"))
            })?;
            Ok(AuthTarget::system(service_id))
        }
        _ => Err(RPCErrors::InvalidToken(
            "invalid target_kind claim".to_string(),
        )),
    }
}

pub fn validate_token_claims(
    token: &RPCSessionToken,
    expected_use: TokenUse,
) -> Result<ValidatedTokenClaims, RPCErrors> {
    let actual_use = token_use(token)?;
    if actual_use != expected_use {
        return Err(RPCErrors::InvalidToken(format!(
            "token_use mismatch: expected {}, got {}",
            expected_use.as_str(),
            actual_use.as_str()
        )));
    }
    Ok(ValidatedTokenClaims {
        principal_kind: token_principal_kind(token)?,
        token_use: actual_use,
        target: token_auth_target(token)?,
    })
}

pub fn validate_verify_hub_token_claims(
    token: &RPCSessionToken,
    expected_use: TokenUse,
) -> Result<ValidatedTokenClaims, RPCErrors> {
    if token.iss.as_deref() != Some(VERIFY_HUB_TOKEN_ISSUER) {
        return Err(RPCErrors::InvalidToken(
            "ordinary session tokens must be issued by verify-hub".to_string(),
        ));
    }
    validate_token_claims(token, expected_use)
}

pub fn token_app_instance_id(token: &RPCSessionToken) -> Result<AppInstanceId, RPCErrors> {
    match token_auth_target(token)? {
        AuthTarget::App { app_instance_id } => Ok(app_instance_id),
        AuthTarget::System { .. } => Err(RPCErrors::InvalidToken(
            "system target has no app_instance_id".to_string(),
        )),
    }
}

pub fn parse_app_instance_id(value: &str) -> Result<(AppId, String), RPCErrors> {
    let instance_id = AppInstanceId::from_str(value).map_err(RPCErrors::ParseRequestError)?;
    Ok((
        instance_id.app_id().clone(),
        instance_id.owner_user_id().to_string(),
    ))
}

pub fn user_app_spec_key(owner_user_id: &str, app_id: &AppId) -> String {
    format!("users/{owner_user_id}/apps/{app_id}/spec")
}

pub fn app_availability_policy_key(app_instance_id: &AppInstanceId) -> String {
    format!("{APP_AVAILABILITY_POLICY_PREFIX}/{app_instance_id}")
}

pub fn app_availability_audit_key(app_instance_id: &AppInstanceId, revision: u64) -> String {
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
    let denied = |reason: &str| AppAvailabilityDecision {
        allowed: false,
        app_id: spec.app_id().clone(),
        app_instance_id: spec.app_instance_id.clone(),
        owner_user_id: spec.owner_user_id.clone(),
        availability_match: None,
        reason: reason.to_string(),
    };
    let allowed = |match_type, subject, reason: &str| AppAvailabilityDecision {
        allowed: true,
        app_id: spec.app_id().clone(),
        app_instance_id: spec.app_instance_id.clone(),
        owner_user_id: spec.owner_user_id.clone(),
        availability_match: Some(AvailabilityMatch {
            match_type,
            subject,
        }),
        reason: reason.to_string(),
    };

    if matches!(spec.state, ServiceState::Deleted) {
        return denied("app_deleted");
    }
    if !owner_settings.map(user_is_active).unwrap_or(false) {
        return denied("owner_not_active");
    }
    let is_guest = target_user_id == "guest" && target_user.is_none();
    if !is_guest && !target_user.map(user_is_active).unwrap_or(false) {
        return denied("user_not_active");
    }
    if !is_guest && target_user_id == spec.owner_user_id {
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
        || policy.app_instance_id != spec.app_instance_id
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
        if let Some(rule) = policy
            .group_rules
            .iter()
            .find(|rule| rule.group_id == group_id)
        {
            return match rule.effect {
                AvailabilityEffect::Allow => allowed(
                    AvailabilityMatchType::Group,
                    Some(group_id.to_string()),
                    "group_allow",
                ),
                AvailabilityEffect::Deny => denied("group_deny"),
            };
        }
    }
    match policy.default_effect {
        AvailabilityEffect::Allow => allowed(
            AvailabilityMatchType::ZoneAllUsers,
            None,
            "policy_default_allow",
        ),
        AvailabilityEffect::Deny => denied("default_deny"),
    }
}

pub fn validate_availability_rules(
    group_rules: &[AppAvailabilityGroupRule],
    user_rules: &[AppAvailabilityUserRule],
) -> Result<(), RPCErrors> {
    const GROUPS: [&str; 4] = ["admins", "users", "limited", "guest"];
    let mut seen_groups = HashSet::new();
    for rule in group_rules {
        if !GROUPS.contains(&rule.group_id.as_str()) || !seen_groups.insert(&rule.group_id) {
            return Err(RPCErrors::ParseRequestError(format!(
                "invalid or duplicate availability group `{}`",
                rule.group_id
            )));
        }
    }
    let mut seen_users = HashSet::new();
    for rule in user_rules {
        if rule.user_id.trim().is_empty()
            || matches!(rule.user_id.as_str(), "guest" | "root")
            || !seen_users.insert(&rule.user_id)
        {
            return Err(RPCErrors::ParseRequestError(format!(
                "invalid or duplicate availability user `{}`",
                rule.user_id
            )));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct AppAvailabilityResolver {
    client: Arc<SystemConfigClient>,
}

impl AppAvailabilityResolver {
    pub fn new(client: Arc<SystemConfigClient>) -> Self {
        Self { client }
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
        app_instance_id: &AppInstanceId,
    ) -> Result<ResolvedAppInstallation, RPCErrors> {
        let spec_path =
            user_app_spec_key(app_instance_id.owner_user_id(), app_instance_id.app_id());
        let value = self.client.get(&spec_path).await.map_err(|error| {
            RPCErrors::ReasonError(format!(
                "app instance `{app_instance_id}` not found: {error}"
            ))
        })?;
        let spec: AppServiceSpec = serde_json::from_str(&value.value).map_err(|error| {
            RPCErrors::ReasonError(format!("invalid app spec `{spec_path}`: {error}"))
        })?;
        if spec.app_instance_id != *app_instance_id
            || spec.owner_user_id != app_instance_id.owner_user_id()
        {
            return Err(RPCErrors::ReasonError(format!(
                "app spec does not match `{app_instance_id}`"
            )));
        }
        Ok(ResolvedAppInstallation { spec, spec_path })
    }

    pub async fn load_policy(
        &self,
        app_instance_id: &AppInstanceId,
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
                    || policy.app_instance_id != *app_instance_id
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
        app_instance_id: &AppInstanceId,
    ) -> Result<AppAvailabilityDecision, RPCErrors> {
        let target_user = self.get_user_settings(user_id).await?;
        let installation = self.resolve_installation(app_instance_id).await?;
        let owner_settings = self
            .get_user_settings(&installation.spec.owner_user_id)
            .await
            .ok();
        let policy = self
            .load_policy(app_instance_id)
            .await?
            .map(|value| value.0);
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
        app_instance_id: &AppInstanceId,
    ) -> Result<AppAvailabilityDecision, RPCErrors> {
        let installation = self.resolve_installation(app_instance_id).await?;
        let owner_settings = self
            .get_user_settings(&installation.spec.owner_user_id)
            .await
            .ok();
        let policy = self
            .load_policy(app_instance_id)
            .await?
            .map(|value| value.0);
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
        let mut candidates = BTreeMap::<AppInstanceId, ResolvedAppInstallation>::new();
        for owner_user_id in list_children_or_empty(&self.client, "users").await? {
            let root = format!("users/{owner_user_id}/apps");
            for raw_app_id in list_children_or_empty(&self.client, &root).await? {
                let app_id = match AppId::parse(&raw_app_id) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                let instance_id = match AppInstanceId::new(app_id, owner_user_id.clone()) {
                    Ok(value) => value,
                    Err(_) => continue,
                };
                if let Ok(installation) = self.resolve_installation(&instance_id).await {
                    candidates.insert(instance_id, installation);
                }
            }
        }

        let mut result = Vec::new();
        for (instance_id, installation) in candidates {
            let owner_settings = self
                .get_user_settings(&installation.spec.owner_user_id)
                .await
                .ok();
            let policy = self.load_policy(&instance_id).await?.map(|value| value.0);
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
        Ok(values) => Ok(values),
        Err(SystemConfigError::KeyNotFound(_)) => Ok(Vec::new()),
        Err(error) => Err(RPCErrors::ReasonError(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token() -> RPCSessionToken {
        RPCSessionToken {
            token_type: ::kRPC::RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(u64::MAX),
            iss: Some(VERIFY_HUB_TOKEN_ISSUER.to_string()),
            jti: Some("1".to_string()),
            sub: Some("alice".to_string()),
            appid: None,
            sudo: false,
            extra: std::collections::HashMap::new(),
        }
    }

    #[test]
    fn parse_app_instance_selector_rejects_noncanonical_values() {
        let parsed = parse_app_instance_id("notes.example.com@alice").unwrap();
        assert_eq!(parsed.0.as_str(), "notes.example.com");
        assert_eq!(parsed.1, "alice");
        assert!(parse_app_instance_id("Notes@alice").is_err());
        assert!(parse_app_instance_id("../../spec@alice").is_err());
    }

    #[test]
    fn availability_rules_are_explicit() {
        assert!(validate_availability_rules(
            &[AppAvailabilityGroupRule {
                group_id: "users".into(),
                effect: AvailabilityEffect::Allow,
            }],
            &[],
        )
        .is_ok());
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
    }

    #[test]
    fn token_target_bind_and_parse_round_trip() {
        let targets = [
            AuthTarget::app("notes.example.com@alice".parse().unwrap()),
            AuthTarget::system("control-panel".parse().unwrap()),
        ];
        for target in targets {
            let mut token = token();
            bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
            bind_token_target(&mut token, &target, TokenUse::Session).unwrap();
            let validated = validate_verify_hub_token_claims(&token, TokenUse::Session).unwrap();
            assert_eq!(validated.principal_kind, TokenPrincipalKind::User);
            assert_eq!(validated.target, target);
        }
    }

    #[test]
    fn token_claim_validation_fails_closed() {
        let app_target = AuthTarget::app("notes.example.com@alice".parse().unwrap());
        let mut valid = token();
        bind_token_principal_kind(&mut valid, TokenPrincipalKind::User);
        bind_token_target(&mut valid, &app_target, TokenUse::Session).unwrap();

        let mut cases = Vec::new();
        let mut missing_kind = valid.clone();
        missing_kind.extra.remove(TOKEN_TARGET_KIND_CLAIM);
        cases.push(missing_kind);
        let mut missing_use = valid.clone();
        missing_use.extra.remove(TOKEN_USE_CLAIM);
        cases.push(missing_use);
        let mut bad_instance = valid.clone();
        bad_instance.extra.insert(
            APP_INSTANCE_ID_CLAIM.to_string(),
            serde_json::Value::String("other.example.com@alice".to_string()),
        );
        cases.push(bad_instance);
        let mut bad_owner = valid.clone();
        bad_owner.extra.insert(
            APP_OWNER_USER_ID_CLAIM.to_string(),
            serde_json::Value::String("bob".to_string()),
        );
        cases.push(bad_owner);
        let mut wrong_issuer = valid.clone();
        wrong_issuer.iss = Some("alice".to_string());
        cases.push(wrong_issuer);

        for token in cases {
            assert!(validate_verify_hub_token_claims(&token, TokenUse::Session).is_err());
        }

        assert!(validate_verify_hub_token_claims(&valid, TokenUse::Refresh).is_err());
    }

    #[test]
    fn token_use_and_sudo_combinations_are_strict() {
        let target = AuthTarget::system("control-panel".parse().unwrap());
        for (token_use, sudo, expected_ok) in [
            (TokenUse::Session, false, true),
            (TokenUse::Session, true, true),
            (TokenUse::Refresh, false, true),
            (TokenUse::Refresh, true, false),
        ] {
            let mut token = token();
            token.sudo = sudo;
            bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
            let result = bind_token_target(&mut token, &target, token_use);
            assert_eq!(result.is_ok(), expected_ok);
            if expected_ok {
                assert!(validate_token_claims(&token, token_use).is_ok());
            }
        }
    }

    #[test]
    fn system_target_rejects_app_claims() {
        let target = AuthTarget::system("control-panel".parse().unwrap());
        let mut token = token();
        bind_token_principal_kind(&mut token, TokenPrincipalKind::User);
        bind_token_target(&mut token, &target, TokenUse::Session).unwrap();
        token.extra.insert(
            APP_INSTANCE_ID_CLAIM.to_string(),
            serde_json::Value::String("control-panel.example@alice".to_string()),
        );
        assert!(token_auth_target(&token).is_err());
    }
}
