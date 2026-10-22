//! Shared model of hybrid SSO (doc/arch/verify-hub/混合SSO技术契约.md): session
//! ceilings carried in signed claims, the D-4 denial codes, the current
//! identity response, Zone SSO settings, exclusive invites and share entries.

use crate::UserType;
use ::kRPC::{RPCErrorDetail, RPCErrors, RPCSessionToken};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;

pub const TOKEN_AUTH_METHOD_CLAIM: &str = "auth_method";
pub const TOKEN_RESTRICTED_CLAIM: &str = "restricted";
pub const TOKEN_SCOPES_CLAIM: &str = "scopes";

/// How the session family was established; delegation and refresh keep it.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum AuthMethod {
    Password,
    ExclusiveInvite,
    DidFederation,
    Assertion,
}

impl AuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Password => "password",
            Self::ExclusiveInvite => "exclusive_invite",
            Self::DidFederation => "did_federation",
            Self::Assertion => "assertion",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "password" => Some(Self::Password),
            "exclusive_invite" => Some(Self::ExclusiveInvite),
            "did_federation" => Some(Self::DidFederation),
            "assertion" => Some(Self::Assertion),
            _ => None,
        }
    }
}

/// What a restricted session may reach at most; services open methods per scope.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum SessionScope {
    /// Own settings (read), profile, address book, password, desktop state.
    AccountSelf,
    /// msg-center group and Session paths.
    MsgGroup,
    #[serde(rename = "aiworkspace")]
    AiWorkspace,
    #[serde(rename = "homestation")]
    HomeStation,
}

impl SessionScope {
    pub const ALL: [SessionScope; 4] = [
        SessionScope::AccountSelf,
        SessionScope::MsgGroup,
        SessionScope::AiWorkspace,
        SessionScope::HomeStation,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::AccountSelf => "account_self",
            Self::MsgGroup => "msg_group",
            Self::AiWorkspace => "aiworkspace",
            Self::HomeStation => "homestation",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|scope| scope.as_str() == value)
    }
}

/// The approved upper bound of a session. Unrestricted sessions behave as
/// before; restricted ones reach only the services that open their scopes.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct SessionCeiling {
    pub restricted: bool,
    pub scopes: BTreeSet<SessionScope>,
}

impl SessionCeiling {
    pub fn unrestricted() -> Self {
        Self::default()
    }

    pub fn restricted(scopes: impl IntoIterator<Item = SessionScope>) -> Self {
        Self {
            restricted: true,
            scopes: scopes.into_iter().collect(),
        }
    }

    /// Ceiling of an account session by its current type.
    pub fn for_user_type(user_type: UserType) -> Self {
        match user_type {
            UserType::Limited => Self::restricted(SessionScope::ALL),
            UserType::Admin | UserType::User | UserType::Root => Self::unrestricted(),
        }
    }

    /// Ceiling of an External (authenticated DID without an account here).
    pub fn external() -> Self {
        Self::restricted([
            SessionScope::MsgGroup,
            SessionScope::AiWorkspace,
            SessionScope::HomeStation,
        ])
    }

    /// The narrower of two ceilings: refresh and delegation never widen.
    pub fn intersect(&self, other: &Self) -> Self {
        match (self.restricted, other.restricted) {
            (false, false) => Self::unrestricted(),
            (true, false) => self.clone(),
            (false, true) => other.clone(),
            (true, true) => Self::restricted(self.scopes.intersection(&other.scopes).copied()),
        }
    }

    pub fn allows(&self, scope: SessionScope) -> bool {
        !self.restricted || self.scopes.contains(&scope)
    }

    pub fn scope_names(&self) -> Vec<&'static str> {
        self.scopes.iter().map(|scope| scope.as_str()).collect()
    }
}

/// Write the ceiling into token claims; an unrestricted ceiling adds nothing.
pub fn bind_token_ceiling(token: &mut RPCSessionToken, ceiling: &SessionCeiling) {
    token.extra.remove(TOKEN_RESTRICTED_CLAIM);
    token.extra.remove(TOKEN_SCOPES_CLAIM);
    if ceiling.restricted {
        token
            .extra
            .insert(TOKEN_RESTRICTED_CLAIM.to_string(), Value::Bool(true));
        token.extra.insert(
            TOKEN_SCOPES_CLAIM.to_string(),
            Value::Array(
                ceiling
                    .scopes
                    .iter()
                    .map(|scope| Value::String(scope.as_str().to_string()))
                    .collect(),
            ),
        );
    }
}

/// The ceiling a verified token carries. A restricted token without a valid
/// scope list is rejected rather than read as unrestricted.
pub fn token_session_ceiling(token: &RPCSessionToken) -> Result<SessionCeiling, RPCErrors> {
    match token.extra.get(TOKEN_RESTRICTED_CLAIM) {
        None | Some(Value::Bool(false)) => {
            if token.extra.contains_key(TOKEN_SCOPES_CLAIM) {
                return Err(RPCErrors::InvalidToken(
                    "scopes claim requires a restricted session".to_string(),
                ));
            }
            Ok(SessionCeiling::unrestricted())
        }
        Some(Value::Bool(true)) => {
            let scopes = token
                .extra
                .get(TOKEN_SCOPES_CLAIM)
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    RPCErrors::InvalidToken("restricted session is missing scopes".to_string())
                })?;
            let mut parsed = BTreeSet::new();
            for scope in scopes {
                let scope = scope
                    .as_str()
                    .and_then(SessionScope::parse)
                    .ok_or_else(|| RPCErrors::InvalidToken("unknown session scope".to_string()))?;
                parsed.insert(scope);
            }
            Ok(SessionCeiling::restricted(parsed))
        }
        Some(_) => Err(RPCErrors::InvalidToken(
            "restricted claim must be a boolean".to_string(),
        )),
    }
}

pub fn bind_token_auth_method(token: &mut RPCSessionToken, method: AuthMethod) {
    token.extra.insert(
        TOKEN_AUTH_METHOD_CLAIM.to_string(),
        Value::String(method.as_str().to_string()),
    );
}

/// `None` for tokens issued before the claim existed or by non-login paths.
pub fn token_auth_method(token: &RPCSessionToken) -> Option<AuthMethod> {
    token
        .extra
        .get(TOKEN_AUTH_METHOD_CLAIM)
        .and_then(Value::as_str)
        .and_then(AuthMethod::parse)
}

// ─── D-4 denial codes ───────────────────────────────────────────────────────

pub const ACCESS_DENIED_ERROR_CODE: &str = "access_denied";

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum DenyReason {
    SessionKindNotAllowed,
    SessionCeilingExceeded,
    ResourceGrantMissing,
    EntryUnavailable,
    ZoneMethodDisabled,
    AccountState,
    CredentialState,
    SourceRevoked,
    AuthorizationUnavailable,
}

/// Who can change the outcome of a denial.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdjustParty {
    /// A product boundary: no setting changes it.
    Product,
    /// The visitor (sign in again, set a password, request a new link).
    #[serde(rename = "self")]
    SelfParty,
    Sharer,
    Admin,
    None,
}

/// Where the adjusting party makes the change.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AdjustLocation {
    Reauth,
    Credential,
    SharePanel,
    #[serde(rename = "cp:sso_settings")]
    CpSsoSettings,
    #[serde(rename = "cp:users")]
    CpUsers,
    #[serde(rename = "cp:exclusive_invites")]
    CpExclusiveInvites,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct DenyAdjust {
    pub party: AdjustParty,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<AdjustLocation>,
}

impl DenyReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SessionKindNotAllowed => "session_kind_not_allowed",
            Self::SessionCeilingExceeded => "session_ceiling_exceeded",
            Self::ResourceGrantMissing => "resource_grant_missing",
            Self::EntryUnavailable => "entry_unavailable",
            Self::ZoneMethodDisabled => "zone_method_disabled",
            Self::AccountState => "account_state",
            Self::CredentialState => "credential_state",
            Self::SourceRevoked => "source_revoked",
            Self::AuthorizationUnavailable => "authorization_unavailable",
        }
    }

    /// The registered adjusting party and place of each reason (contract §4.2).
    pub fn adjust(self) -> DenyAdjust {
        let (party, location) = match self {
            Self::SessionKindNotAllowed => (AdjustParty::Product, None),
            Self::SessionCeilingExceeded => (AdjustParty::SelfParty, Some(AdjustLocation::Reauth)),
            Self::ResourceGrantMissing | Self::EntryUnavailable => {
                (AdjustParty::Sharer, Some(AdjustLocation::SharePanel))
            }
            Self::ZoneMethodDisabled => (AdjustParty::Admin, Some(AdjustLocation::CpSsoSettings)),
            Self::AccountState => (AdjustParty::Admin, Some(AdjustLocation::CpUsers)),
            Self::CredentialState => (AdjustParty::SelfParty, Some(AdjustLocation::Credential)),
            Self::SourceRevoked => (AdjustParty::Admin, Some(AdjustLocation::CpExclusiveInvites)),
            Self::AuthorizationUnavailable => (AdjustParty::None, None),
        };
        DenyAdjust { party, location }
    }
}

/// A structured denial (D-4). Travels as `RPCErrors::Detailed` with code
/// `access_denied`, or inside a `status: denied` success envelope.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct AccessDenial {
    pub reason: DenyReason,
    pub adjust: DenyAdjust,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    /// A finer outcome a page branches on (e.g. `expired`, `password_required`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl AccessDenial {
    pub fn new(reason: DenyReason) -> Self {
        Self {
            reason,
            adjust: reason.adjust(),
            service: None,
            method: None,
            detail: None,
        }
    }

    pub fn at(mut self, service: impl Into<String>, method: impl Into<String>) -> Self {
        self.service = Some(service.into());
        self.method = Some(method.into());
        self
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn into_rpc_error(self) -> RPCErrors {
        let message = format!("access denied: {}", self.reason.as_str());
        RPCErrors::Detailed(RPCErrorDetail::new(
            ACCESS_DENIED_ERROR_CODE,
            message,
            serde_json::to_value(&self).ok(),
        ))
    }

    pub fn from_rpc_error(error: &RPCErrors) -> Option<Self> {
        let detail = error.detail()?;
        if detail.code != ACCESS_DENIED_ERROR_CODE {
            return None;
        }
        serde_json::from_value(detail.data.clone()?).ok()
    }
}

// ─── current identity ──────────────────────────────────────────────────────

/// Whom the browser session speaks for (contract §2.2).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CurrentIdentity {
    Account {
        user_id: String,
        did: String,
        user_type: UserType,
        show_name: String,
        zone_host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_method: Option<AuthMethod>,
        restricted: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scopes: Vec<SessionScope>,
        password_set: bool,
    },
    External {
        did: String,
        show_name: String,
        zone_host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        auth_method: Option<AuthMethod>,
        restricted: bool,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scopes: Vec<SessionScope>,
    },
    Anonymous,
}

// ─── Zone SSO settings ─────────────────────────────────────────────────────

pub const SSO_SETTINGS_KEY: &str = "system/sso/settings";
pub const DEFAULT_JUMP_BASE_URL: &str = "https://jump.buckyos.org";

fn default_true() -> bool {
    true
}

/// `system/sso/settings`; a missing key means every default (D-8a: both on).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SsoSettings {
    #[serde(default = "default_true")]
    pub self_register_enabled: bool,
    #[serde(default = "default_true")]
    pub did_login_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_base_url: Option<String>,
    /// Lets DID login exchange codes over HTTP; local development only.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dev_federation_http: bool,
}

impl Default for SsoSettings {
    fn default() -> Self {
        Self {
            self_register_enabled: true,
            did_login_enabled: true,
            jump_base_url: None,
            dev_federation_http: false,
        }
    }
}

impl SsoSettings {
    pub fn effective_jump_base_url(&self) -> &str {
        self.jump_base_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_JUMP_BASE_URL)
    }

    pub fn public_config(&self) -> SsoPublicConfig {
        SsoPublicConfig {
            self_register_enabled: self.self_register_enabled,
            did_login_enabled: self.did_login_enabled,
            jump_base_url: self.effective_jump_base_url().to_string(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct SsoPublicConfig {
    pub self_register_enabled: bool,
    pub did_login_enabled: bool,
    pub jump_base_url: String,
}

// ─── share entries ─────────────────────────────────────────────────────────

/// Authentication methods an entry offers (contract §8.2).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct EntryMethods {
    #[serde(default)]
    pub guest: bool,
    /// An existing local account of this Zone may sign in.
    #[serde(default)]
    pub password: bool,
    #[serde(default)]
    pub self_register: bool,
    #[serde(default)]
    pub did_login: bool,
}

impl EntryMethods {
    /// Entry methods ∩ Zone switches; also returns the methods the Zone closed.
    pub fn effective(self, settings: &SsoSettings) -> (Self, Vec<EntryMethod>) {
        let mut closed = Vec::new();
        let mut effective = self;
        if self.self_register && !settings.self_register_enabled {
            effective.self_register = false;
            closed.push(EntryMethod::SelfRegister);
        }
        if self.did_login && !settings.did_login_enabled {
            effective.did_login = false;
            closed.push(EntryMethod::DidLogin);
        }
        // An entry open to self-registration also admits existing local accounts,
        // and keeps admitting them when the Zone closes self-registration.
        if self.self_register {
            effective.password = true;
        }
        (effective, closed)
    }

    pub fn allows(&self, method: EntryMethod) -> bool {
        match method {
            EntryMethod::Guest => self.guest,
            EntryMethod::Password => self.password || self.self_register,
            EntryMethod::SelfRegister => self.self_register,
            EntryMethod::DidLogin => self.did_login,
        }
    }

    /// The entry method a verified session arrived by.
    pub fn allows_session(&self, auth_method: Option<AuthMethod>) -> bool {
        match auth_method {
            Some(AuthMethod::DidFederation) => self.did_login,
            _ => self.password || self.self_register,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EntryMethod {
    Guest,
    Password,
    SelfRegister,
    DidLogin,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryService {
    #[serde(rename = "aiworkspace")]
    AiWorkspace,
    #[serde(rename = "msg-center")]
    MsgCenter,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EntryRef {
    pub service: EntryService,
    pub entry_id: String,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EntryState {
    Active,
    /// Missing, closed and expired look the same to visitors.
    Unavailable,
}

/// Public description of a resource-side entry (no private content).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EntryResource {
    pub kind: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_name: Option<String>,
    /// Zone-relative path of the resource page.
    pub path: String,
}

/// What the resource service reports; `methods` are its own allowance.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EntryDescription {
    pub state: EntryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<EntryResource>,
    #[serde(default)]
    pub methods: EntryMethods,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl EntryDescription {
    pub fn unavailable() -> Self {
        Self {
            state: EntryState::Unavailable,
            resource: None,
            methods: EntryMethods::default(),
            expires_at: None,
        }
    }
}

/// `auth.entry.describe`: the resource description after the Zone switches.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct EntryDescriptor {
    pub entry: EntryRef,
    pub state: EntryState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<EntryResource>,
    pub methods: EntryMethods,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub zone_disabled: Vec<EntryMethod>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl EntryDescriptor {
    pub fn from_description(
        entry: EntryRef,
        description: EntryDescription,
        settings: &SsoSettings,
    ) -> Self {
        if description.state != EntryState::Active {
            return Self {
                entry,
                state: EntryState::Unavailable,
                resource: None,
                methods: EntryMethods::default(),
                zone_disabled: Vec::new(),
                expires_at: None,
            };
        }
        let (methods, zone_disabled) = description.methods.effective(settings);
        Self {
            entry,
            state: EntryState::Active,
            resource: description.resource,
            methods,
            zone_disabled,
            expires_at: description.expires_at,
        }
    }
}

// ─── exclusive invites ─────────────────────────────────────────────────────

pub const EXCLUSIVE_INVITE_PREFIX: &str = "services/control_panel/exclusive_invites";
/// Q-01: the password-free credential lives 48 hours, not adjustable in v1.
pub const EXCLUSIVE_INVITE_TTL_SECS: u64 = 48 * 3600;
pub const EXCLUSIVE_INVITE_RESEND_INTERVAL_SECS: u64 = 10 * 60;

pub fn exclusive_invite_key(invite_id: &str) -> String {
    format!("{EXCLUSIVE_INVITE_PREFIX}/{invite_id}")
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InviteIssuer {
    User { user_id: String },
    App { app_instance_id: String },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InviteChannel {
    /// `email`, `sms` or another channel the issuer uses.
    pub kind: String,
    /// Masked address shown to admins; the issuer keeps the full one.
    #[serde(default)]
    pub target_hint: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InviteResource {
    GroupSession {
        group_did: String,
        session_id: String,
        #[serde(default)]
        title: String,
    },
    Workspace {
        workspace_id: String,
        #[serde(default)]
        title: String,
    },
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InviteState {
    Active,
    Revoked,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InviteCredentialState {
    Active,
    Superseded,
    ConsumedByPassword,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InviteCredential {
    pub version: u32,
    /// base64url(sha256(secret)); the secret itself is never stored.
    pub digest: String,
    pub issued_at: u64,
    pub expires_at: u64,
    pub state: InviteCredentialState,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InviteResourceState {
    Done,
    Pending,
    None,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct InviteResend {
    pub requested_at: u64,
    pub count: u32,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ExclusiveInvite {
    pub invite_id: String,
    pub user_id: String,
    pub issuer: InviteIssuer,
    pub channel: InviteChannel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<InviteResource>,
    #[serde(default = "invite_resource_state_none")]
    pub resource_state: InviteResourceState,
    /// Zone-relative path the visitor returns to after signing in.
    pub return_path: String,
    pub state: InviteState,
    pub credential: InviteCredential,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resend: Option<InviteResend>,
    pub created_at: u64,
    pub updated_at: u64,
}

fn invite_resource_state_none() -> InviteResourceState {
    InviteResourceState::None
}

/// Why an exclusive invite cannot sign the visitor in (contract §5.3).
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InviteLoginRefusal {
    Revoked,
    AccountInactive,
    PasswordRequired,
    LinkReplaced,
    Expired,
}

impl InviteLoginRefusal {
    pub fn denial(self) -> AccessDenial {
        let (reason, detail) = match self {
            Self::Revoked => (DenyReason::SourceRevoked, "revoked"),
            Self::AccountInactive => (DenyReason::AccountState, "account_inactive"),
            Self::PasswordRequired => (DenyReason::CredentialState, "password_required"),
            Self::LinkReplaced => (DenyReason::CredentialState, "link_replaced"),
            Self::Expired => (DenyReason::CredentialState, "expired"),
        };
        AccessDenial::new(reason).with_detail(detail)
    }
}

impl ExclusiveInvite {
    /// The login decision for a presented secret digest (account checks are
    /// the caller's: it reads the account record at the same time).
    pub fn check_credential(&self, digest: &str, now: u64) -> Result<(), InviteLoginRefusal> {
        if self.state != InviteState::Active {
            return Err(InviteLoginRefusal::Revoked);
        }
        if self.credential.state == InviteCredentialState::ConsumedByPassword {
            return Err(InviteLoginRefusal::PasswordRequired);
        }
        if self.credential.state != InviteCredentialState::Active
            || !constant_time_eq(self.credential.digest.as_bytes(), digest.as_bytes())
        {
            return Err(InviteLoginRefusal::LinkReplaced);
        }
        if now > self.credential.expires_at {
            return Err(InviteLoginRefusal::Expired);
        }
        Ok(())
    }
}

/// Session source kept in the verify-hub family record.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionSource {
    ExclusiveInvite { invite_id: String, version: u32 },
    DidFederation { canonical_host: String },
}

pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// base64url(sha256(secret)) as stored in invite records.
pub fn invite_secret_digest(secret: &str) -> String {
    use base64::Engine;
    use sha2::Digest;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(sha2::Sha256::digest(secret.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::kRPC::RPCSessionTokenType;
    use std::collections::HashMap;

    fn token() -> RPCSessionToken {
        RPCSessionToken {
            token_type: RPCSessionTokenType::Normal,
            appid: None,
            jti: None,
            aud: None,
            sub: Some("carol".to_string()),
            token: None,
            iss: None,
            exp: None,
            sudo: false,
            extra: HashMap::new(),
        }
    }

    #[test]
    fn ceilings_by_principal_and_intersection_never_widen() {
        assert!(!SessionCeiling::for_user_type(UserType::User).restricted);
        let limited = SessionCeiling::for_user_type(UserType::Limited);
        assert!(limited.allows(SessionScope::AccountSelf));
        let external = SessionCeiling::external();
        assert!(!external.allows(SessionScope::AccountSelf));
        assert_eq!(limited.intersect(&external), external);
        // An upgraded Limited account keeps its restricted ceiling.
        assert_eq!(
            limited.intersect(&SessionCeiling::for_user_type(UserType::User)),
            limited
        );
        // A downgraded User narrows to Limited.
        assert_eq!(SessionCeiling::unrestricted().intersect(&limited), limited);
    }

    #[test]
    fn ceiling_claims_round_trip_and_fail_closed() {
        let mut t = token();
        bind_token_ceiling(&mut t, &SessionCeiling::external());
        assert_eq!(token_session_ceiling(&t).unwrap(), SessionCeiling::external());
        bind_token_ceiling(&mut t, &SessionCeiling::unrestricted());
        assert!(!t.extra.contains_key(TOKEN_RESTRICTED_CLAIM));
        assert_eq!(token_session_ceiling(&t).unwrap(), SessionCeiling::unrestricted());

        t.extra.insert(TOKEN_RESTRICTED_CLAIM.into(), Value::Bool(true));
        assert!(token_session_ceiling(&t).is_err());
        t.extra.insert(TOKEN_SCOPES_CLAIM.into(), serde_json::json!(["msg_group", "bogus"]));
        assert!(token_session_ceiling(&t).is_err());
        t.extra.insert(TOKEN_RESTRICTED_CLAIM.into(), Value::String("yes".into()));
        assert!(token_session_ceiling(&t).is_err());
        let mut scoped_unrestricted = token();
        scoped_unrestricted
            .extra
            .insert(TOKEN_SCOPES_CLAIM.into(), serde_json::json!(["msg_group"]));
        assert!(token_session_ceiling(&scoped_unrestricted).is_err());
    }

    #[test]
    fn denial_round_trips_through_the_rpc_error_code() {
        let denial = AccessDenial::new(DenyReason::ResourceGrantMissing).at("aiworkspace", "doc.read");
        let error = denial.clone().into_rpc_error();
        assert_eq!(error.detail().unwrap().code, ACCESS_DENIED_ERROR_CODE);
        assert_eq!(AccessDenial::from_rpc_error(&error), Some(denial));
        assert_eq!(
            AccessDenial::from_rpc_error(&RPCErrors::NoPermission("x".into())),
            None
        );
        let wire = serde_json::to_value(AccessDenial::new(DenyReason::ZoneMethodDisabled)).unwrap();
        assert_eq!(wire["adjust"]["party"], "admin");
        assert_eq!(wire["adjust"]["location"], "cp:sso_settings");
        let wire = serde_json::to_value(AccessDenial::new(DenyReason::CredentialState)).unwrap();
        assert_eq!(wire["adjust"]["party"], "self");
    }

    #[test]
    fn entry_methods_intersect_zone_switches() {
        let methods = EntryMethods {
            guest: true,
            password: false,
            self_register: true,
            did_login: true,
        };
        let settings = SsoSettings {
            did_login_enabled: false,
            ..SsoSettings::default()
        };
        let (effective, closed) = methods.effective(&settings);
        assert!(effective.guest && effective.self_register && effective.password);
        assert!(!effective.did_login);
        assert_eq!(closed, vec![EntryMethod::DidLogin]);
        assert!(effective.allows_session(Some(AuthMethod::Password)));
        assert!(!effective.allows_session(Some(AuthMethod::DidFederation)));

        let register_only = EntryMethods {
            self_register: true,
            ..EntryMethods::default()
        };
        let closed_zone = SsoSettings {
            self_register_enabled: false,
            ..SsoSettings::default()
        };
        let (effective, closed) = register_only.effective(&closed_zone);
        assert!(!effective.self_register && effective.password);
        assert_eq!(closed, vec![EntryMethod::SelfRegister]);
        assert!(effective.allows_session(Some(AuthMethod::Password)));
    }

    #[test]
    fn sso_settings_default_on_and_jump_falls_back() {
        let settings: SsoSettings = serde_json::from_str("{}").unwrap();
        assert!(settings.self_register_enabled && settings.did_login_enabled);
        assert_eq!(settings.effective_jump_base_url(), DEFAULT_JUMP_BASE_URL);
        let custom: SsoSettings =
            serde_json::from_str(r#"{"jump_base_url":"https://jump.example"}"#).unwrap();
        assert_eq!(custom.public_config().jump_base_url, "https://jump.example");
    }

    fn invite(now: u64) -> ExclusiveInvite {
        ExclusiveInvite {
            invite_id: "ei-1".into(),
            user_id: "acme-c1".into(),
            issuer: InviteIssuer::User {
                user_id: "devtest".into(),
            },
            channel: InviteChannel {
                kind: "email".into(),
                target_hint: "a***@acme.com".into(),
            },
            resource: None,
            resource_state: InviteResourceState::None,
            return_path: "/messagehub".into(),
            state: InviteState::Active,
            credential: InviteCredential {
                version: 1,
                digest: invite_secret_digest("secret"),
                issued_at: now,
                expires_at: now + EXCLUSIVE_INVITE_TTL_SECS,
                state: InviteCredentialState::Active,
            },
            resend: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn invite_credential_decisions_follow_the_contract_order() {
        let now = 1_000;
        let good = invite_secret_digest("secret");
        let mut record = invite(now);
        assert!(record.check_credential(&good, now + 10).is_ok());
        // Reusable within the TTL: a second presentation still works.
        assert!(record.check_credential(&good, now + 3600).is_ok());
        assert_eq!(
            record.check_credential(&good, now + EXCLUSIVE_INVITE_TTL_SECS + 1),
            Err(InviteLoginRefusal::Expired)
        );
        assert_eq!(
            record.check_credential(&invite_secret_digest("other"), now),
            Err(InviteLoginRefusal::LinkReplaced)
        );
        record.credential.state = InviteCredentialState::ConsumedByPassword;
        assert_eq!(
            record.check_credential(&good, now),
            Err(InviteLoginRefusal::PasswordRequired)
        );
        record.state = InviteState::Revoked;
        assert_eq!(record.check_credential(&good, now), Err(InviteLoginRefusal::Revoked));
        assert_eq!(
            InviteLoginRefusal::Expired.denial().reason,
            DenyReason::CredentialState
        );
    }
}
