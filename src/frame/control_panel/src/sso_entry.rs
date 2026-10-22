//! Share entries (SSO-07, SSO-11, contract §8.2): the public description of a
//! resource-side entry after the Zone switches, and self-registration of a
//! Limited account through an entry that allows it.

use crate::agent_mgr::{is_dns_label, AgentKv, AgentNameIndex, ZoneKv};
use crate::sso_settings::load_sso_settings;
use crate::user_mgr::{
    after_account_created, new_account_actions, validate_password_hash, validate_username,
};
use crate::ControlPanelServer;
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, AccessDenial, DenyReason, EntryDescription, EntryDescriptor,
    EntryMethod, EntryRef, EntryService, EntryState, PasswordState, UserSettings, UserState,
    UserType, AIWORKSPACE_SERVICE_NAME, CONTROL_PANEL_SERVICE_UNIQUE_ID, MSG_CENTER_SERVICE_NAME,
};
use buckyos_kit::buckyos_get_unix_timestamp;
use log::{info, warn};
use name_lib::DID;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;

/// Self-registration bounds within the window: attempts per client address
/// (counted before any account lookup, so name probing is limited too) and
/// accounts actually created in the whole Zone. A client whose address is
/// unknown (see `forwarded_client_ip`) has no per-client bound.
const REGISTRATION_WINDOW_SECS: u64 = 600;
const REGISTRATION_ATTEMPTS_PER_CLIENT: usize = 20;
const REGISTRATIONS_PER_ZONE: usize = 100;

#[derive(Default)]
pub(crate) struct RegistrationLimiter {
    per_client: HashMap<IpAddr, VecDeque<u64>>,
    created: VecDeque<u64>,
}

impl RegistrationLimiter {
    fn expire(&mut self, now: u64) {
        let since = now.saturating_sub(REGISTRATION_WINDOW_SECS);
        let expire = |times: &mut VecDeque<u64>| {
            while times.front().is_some_and(|at| *at <= since) {
                times.pop_front();
            }
        };
        expire(&mut self.created);
        self.per_client.retain(|_, times| {
            expire(times);
            !times.is_empty()
        });
    }

    /// Records the attempt when the client and the Zone are within their bounds.
    /// A loopback address stands for "client unknown".
    pub(crate) fn admit(&mut self, client: IpAddr, now: u64) -> bool {
        self.expire(now);
        if self.created.len() >= REGISTRATIONS_PER_ZONE {
            return false;
        }
        if client.is_loopback() {
            return true;
        }
        let attempts = self.per_client.entry(client).or_default();
        if attempts.len() >= REGISTRATION_ATTEMPTS_PER_CLIENT {
            return false;
        }
        attempts.push_back(now);
        true
    }

    pub(crate) fn record_created(&mut self, now: u64) {
        self.created.push_back(now);
    }
}

/// Where the resource services publish their entries.
#[async_trait]
pub(crate) trait EntrySource: Send + Sync {
    async fn describe(&self, entry: &EntryRef) -> Result<EntryDescription, RPCErrors>;
}

pub(crate) struct ZoneEntrySource;

#[async_trait]
impl EntrySource for ZoneEntrySource {
    async fn describe(&self, entry: &EntryRef) -> Result<EntryDescription, RPCErrors> {
        let (service, method, params) = match entry.service {
            EntryService::AiWorkspace => (
                AIWORKSPACE_SERVICE_NAME,
                "entry.describe",
                json!({ "entry_id": entry.entry_id }),
            ),
            EntryService::MsgCenter => (
                MSG_CENTER_SERVICE_NAME,
                "group.describe_entry",
                json!({ "group_did": entry.entry_id }),
            ),
        };
        let value = get_buckyos_api_runtime()?
            .get_zone_service_krpc_client(service)
            .await?
            .call(method, params)
            .await?;
        serde_json::from_value(value)
            .map_err(|error| RPCErrors::ParserResponseError(format!("{service} {method}: {error}")))
    }
}

/// The entry after the Zone switches; an entry its service cannot describe looks
/// like a missing one (no detail leaks to the visitor).
pub(crate) async fn describe_entry(
    source: &dyn EntrySource,
    kv: &dyn AgentKv,
    entry: EntryRef,
) -> Result<EntryDescriptor, RPCErrors> {
    let description = match source.describe(&entry).await {
        Ok(description) => description,
        Err(error) => {
            warn!(
                "entry {:?}/{} is not describable: {error}",
                entry.service, entry.entry_id
            );
            EntryDescription::unavailable()
        }
    };
    let settings = load_sso_settings(kv).await?;
    Ok(EntryDescriptor::from_description(
        entry,
        description,
        &settings,
    ))
}

/// Self-registration needs an active entry that allows it after the Zone switch.
pub(crate) fn registration_gate(descriptor: &EntryDescriptor) -> Result<(), AccessDenial> {
    if descriptor.state != EntryState::Active {
        return Err(AccessDenial::new(DenyReason::EntryUnavailable));
    }
    if descriptor.methods.self_register {
        return Ok(());
    }
    if descriptor
        .zone_disabled
        .contains(&EntryMethod::SelfRegister)
    {
        return Err(AccessDenial::new(DenyReason::ZoneMethodDisabled));
    }
    Err(AccessDenial::new(DenyReason::EntryUnavailable))
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct EntryDescribeParams {
    service: EntryService,
    entry_id: String,
}

/// The only fields a visitor sets; type, groups, role, owner or zone are refused.
#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct RegisterParams {
    service: EntryService,
    entry_id: String,
    user_id: String,
    #[serde(default)]
    show_name: Option<String>,
    password_hash: String,
}

#[derive(Serialize, Debug, PartialEq)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum RegisterResult {
    Created { user_id: String },
    NameTaken,
    Denied { denial: AccessDenial },
}

/// Creates the Limited account of a validated registration. `Create` makes a
/// concurrent registration of the same name lose: it then reads `name_taken`.
pub(crate) async fn register_account(
    source: &dyn EntrySource,
    kv: &dyn AgentKv,
    zone_did: &DID,
    params: RegisterParams,
) -> Result<RegisterResult, RPCErrors> {
    validate_password_hash(&params.password_hash)?;
    // Not lowercased here: the page hashed the password with the name as sent,
    // so a name that is not already canonical is refused instead.
    let user_id = params.user_id.trim().to_string();
    validate_username(&user_id)?;
    if !is_dns_label(&user_id) {
        return Err(RPCErrors::ParseRequestError(
            "user_id must be 1-63 lowercase letters, digits or hyphens".to_string(),
        ));
    }
    let descriptor = describe_entry(
        source,
        kv,
        EntryRef {
            service: params.service,
            entry_id: params.entry_id,
        },
    )
    .await?;
    if let Err(denial) = registration_gate(&descriptor) {
        return Ok(RegisterResult::Denied { denial });
    }
    if AgentNameIndex::load(kv, zone_did)
        .await?
        .check(&user_id)
        .is_err()
    {
        return Ok(RegisterResult::NameTaken);
    }
    let settings = UserSettings {
        user_id: user_id.clone(),
        user_type: UserType::Limited,
        password: params.password_hash,
        state: UserState::Active,
        res_pool_id: "default".to_string(),
        is_local: true,
        allow_password_change: None,
        allow_contact_edit: None,
        groups: Vec::new(),
        password_state: PasswordState::Set,
    };
    let show_name = params
        .show_name
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| user_id.clone());
    let (actions, _) = new_account_actions(&settings, &show_name, zone_did, false)?;
    let settings_key = format!("users/{user_id}/settings");
    if let Err(error) = kv.exec_tx(actions, None).await {
        if kv.get(&settings_key).await?.is_some() {
            return Ok(RegisterResult::NameTaken);
        }
        return Err(error);
    }
    Ok(RegisterResult::Created { user_id })
}

impl ControlPanelServer {
    /// `auth.entry.describe {service: "aiworkspace"|"msg-center", entry_id}` (public)
    /// → `EntryDescriptor`.
    pub(crate) async fn handle_auth_entry_describe(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: EntryDescribeParams = Self::typed_params(&req)?;
        let descriptor = describe_entry(
            &ZoneEntrySource,
            &ZoneKv::connect().await?,
            EntryRef {
                service: params.service,
                entry_id: params.entry_id,
            },
        )
        .await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!(descriptor)),
            req.seq,
        ))
    }

    /// `auth.register {service, entry_id, user_id, show_name?, password_hash}` (public)
    /// → `{status:"created", user_id}` | `{status:"name_taken"}` | `{status:"denied", denial}`
    /// (an exhausted rate limit is `authorization_unavailable` / `rate_limited`). A malformed
    /// `user_id` or digest is a request error. The page then signs in with `auth.login` and
    /// joins the resource itself.
    pub(crate) async fn handle_auth_register(
        &self,
        req: RPCRequest,
        client_ip: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let params: RegisterParams = Self::typed_params(&req)?;
        let admitted = self
            .registration_limiter
            .lock()
            .unwrap()
            .admit(client_ip, buckyos_get_unix_timestamp());
        let result = if admitted {
            let runtime = get_buckyos_api_runtime()?;
            let kv = ZoneKv::connect().await?;
            let result = register_account(&ZoneEntrySource, &kv, &runtime.zone_id, params).await?;
            if let RegisterResult::Created { user_id } = &result {
                self.registration_limiter
                    .lock()
                    .unwrap()
                    .record_created(buckyos_get_unix_timestamp());
                info!("account '{user_id}' self-registered through a share entry");
                if let Err(error) = after_account_created(user_id, "auth.register").await {
                    warn!("RBAC refresh after self-registration of {user_id} failed: {error}");
                }
            }
            result
        } else {
            RegisterResult::Denied {
                denial: AccessDenial::new(DenyReason::AuthorizationUnavailable)
                    .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.register")
                    .with_detail("rate_limited"),
            }
        };
        Ok(RPCResponse::new(RPCResult::Success(json!(result)), req.seq))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exclusive_invite::tests::{zone, MemKv};
    use buckyos_api::{EntryMethods, EntryResource, SSO_SETTINGS_KEY};

    struct FakeSource(Option<EntryDescription>);

    #[async_trait]
    impl EntrySource for FakeSource {
        async fn describe(&self, _entry: &EntryRef) -> Result<EntryDescription, RPCErrors> {
            self.0
                .clone()
                .ok_or_else(|| RPCErrors::ReasonError("unreachable".to_string()))
        }
    }

    fn description(methods: EntryMethods) -> EntryDescription {
        EntryDescription {
            state: EntryState::Active,
            resource: Some(EntryResource {
                kind: "workspace".to_string(),
                title: "DJ".to_string(),
                owner_name: None,
                path: "/workspace/w1".to_string(),
            }),
            methods,
            expires_at: None,
        }
    }

    fn self_register() -> EntryMethods {
        EntryMethods {
            self_register: true,
            ..Default::default()
        }
    }

    fn register(user_id: &str) -> RegisterParams {
        serde_json::from_value(json!({
            "service": "aiworkspace",
            "entry_id": "se-1",
            "user_id": user_id,
            "password_hash": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn describe_intersects_zone_switches_and_hides_unreachable_entries() {
        let kv = MemKv::default();
        let entry = || EntryRef {
            service: EntryService::AiWorkspace,
            entry_id: "se-1".to_string(),
        };
        let descriptor = describe_entry(&FakeSource(None), &kv, entry())
            .await
            .unwrap();
        assert_eq!(descriptor.state, EntryState::Unavailable);
        assert!(descriptor.resource.is_none());

        kv.put_raw(SSO_SETTINGS_KEY, r#"{"self_register_enabled":false}"#);
        let descriptor = describe_entry(
            &FakeSource(Some(description(self_register()))),
            &kv,
            entry(),
        )
        .await
        .unwrap();
        assert!(!descriptor.methods.self_register);
        assert_eq!(descriptor.zone_disabled, vec![EntryMethod::SelfRegister]);
        assert_eq!(
            registration_gate(&descriptor).unwrap_err().reason,
            DenyReason::ZoneMethodDisabled
        );
    }

    #[test]
    fn registration_needs_an_active_entry_that_allows_it() {
        let entry = EntryRef {
            service: EntryService::MsgCenter,
            entry_id: "did:bns:support.test".to_string(),
        };
        let open = EntryDescriptor::from_description(
            entry.clone(),
            description(self_register()),
            &Default::default(),
        );
        assert!(registration_gate(&open).is_ok());
        let password_only = EntryDescriptor::from_description(
            entry.clone(),
            description(EntryMethods {
                password: true,
                ..Default::default()
            }),
            &Default::default(),
        );
        assert_eq!(
            registration_gate(&password_only).unwrap_err().reason,
            DenyReason::EntryUnavailable
        );
        let closed = EntryDescriptor::from_description(
            entry,
            EntryDescription::unavailable(),
            &Default::default(),
        );
        assert_eq!(
            registration_gate(&closed).unwrap_err().reason,
            DenyReason::EntryUnavailable
        );
    }

    #[tokio::test]
    async fn registration_creates_a_plain_limited_account_once() {
        let kv = MemKv::default();
        let source = FakeSource(Some(description(self_register())));
        let result = register_account(&source, &kv, &zone(), register("carol"))
            .await
            .unwrap();
        assert_eq!(
            result,
            RegisterResult::Created {
                user_id: "carol".to_string()
            }
        );
        let settings: UserSettings = kv.read("users/carol/settings");
        assert_eq!(settings.user_type, UserType::Limited);
        assert_eq!(settings.password_state, PasswordState::Set);
        assert!(settings.groups.is_empty() && settings.is_local);
        let doc: name_lib::OwnerDocument = kv.read("users/carol/doc");
        assert_eq!(doc.get_default_zone_did(), None);

        assert_eq!(
            register_account(&source, &kv, &zone(), register("carol"))
                .await
                .unwrap(),
            RegisterResult::NameTaken
        );
        for name in ["root", "bad name", "", "dot.name", "Carol"] {
            assert!(
                matches!(
                    register_account(&source, &kv, &zone(), register(name)).await,
                    Err(RPCErrors::ParseRequestError(_))
                ),
                "{name}"
            );
        }
        assert_eq!(
            register_account(&source, &kv, &zone(), register("www"))
                .await
                .unwrap(),
            RegisterResult::NameTaken
        );
        let closed = FakeSource(Some(description(EntryMethods::default())));
        assert!(matches!(
            register_account(&closed, &kv, &zone(), register("dave"))
                .await
                .unwrap(),
            RegisterResult::Denied { .. }
        ));
        assert!(!kv.has("users/dave/settings"));
        assert!(matches!(
            register_account(&source, &kv, &zone(), {
                let mut params = register("erin");
                params.password_hash = "plain".to_string();
                params
            })
            .await,
            Err(RPCErrors::ParseRequestError(_))
        ));
    }

    #[test]
    fn registration_params_refuse_privileged_fields() {
        for (key, value) in [
            ("user_type", json!("admin")),
            ("groups", json!(["staff"])),
            ("owner", json!("devtest")),
            ("role", json!("admin")),
            ("default_zone", json!("did:bns:other")),
        ] {
            let mut params = json!({
                "service": "aiworkspace",
                "entry_id": "se-1",
                "user_id": "carol",
                "password_hash": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            });
            params[key] = value;
            assert!(
                serde_json::from_value::<RegisterParams>(params).is_err(),
                "{key}"
            );
        }
    }

    #[test]
    fn registration_limiter_bounds_each_client_and_the_created_accounts() {
        let mut limiter = RegistrationLimiter::default();
        let a: IpAddr = "203.0.113.7".parse().unwrap();
        let b: IpAddr = "203.0.113.8".parse().unwrap();
        let now = 10_000;
        for _ in 0..REGISTRATION_ATTEMPTS_PER_CLIENT {
            assert!(limiter.admit(a, now));
        }
        assert!(!limiter.admit(a, now + 1));
        // One client's failed attempts do not block another.
        assert!(limiter.admit(b, now + 1));
        assert!(limiter.admit(a, now + REGISTRATION_WINDOW_SECS + 1));
        // Unknown clients (the gateway's loopback) are bounded only Zone-wide,
        // and only created accounts count there.
        let unknown: IpAddr = "127.0.0.1".parse().unwrap();
        let mut limiter = RegistrationLimiter::default();
        for _ in 0..REGISTRATION_ATTEMPTS_PER_CLIENT * 3 {
            assert!(limiter.admit(unknown, now));
        }
        for _ in 0..REGISTRATIONS_PER_ZONE {
            limiter.record_created(now);
        }
        assert!(!limiter.admit(unknown, now));
        assert!(!limiter.admit(b, now));
        assert!(limiter.admit(b, now + REGISTRATION_WINDOW_SECS + 1));
    }
}
