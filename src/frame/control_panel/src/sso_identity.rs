//! The current identity of a browser session (contract §2.2): `/sso_refresh`,
//! `auth.login` and `auth.current_identity` answer with the same shape.

use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult, RPCSessionToken};
use buckyos_api::{
    get_buckyos_api_runtime, token_auth_method, token_session_ceiling,
    validate_verify_hub_token_claims, AccessDenial, ControlPanelClient, CurrentIdentity,
    DenyReason, FederationProfile, TokenPrincipalKind, TokenUse, UserPrivateProfile, UserSettings,
    CONTROL_PANEL_SERVICE_UNIQUE_ID,
};
use log::warn;
use name_lib::DID;
use serde_json::json;

/// External display names kept from DID logins; bounded, lost on restart.
const EXTERNAL_PROFILE_CACHE_LIMIT: usize = 1024;

/// The identity of an account session from its records and verified claims.
pub(crate) fn account_identity(
    token: &RPCSessionToken,
    settings: &UserSettings,
    did: &DID,
    profile: Option<&UserPrivateProfile>,
    zone_host: &str,
) -> Result<CurrentIdentity, RPCErrors> {
    if !settings.is_active() {
        return Err(AccessDenial::new(DenyReason::AccountState).into_rpc_error());
    }
    let ceiling = token_session_ceiling(token)?;
    let show_name = profile
        .and_then(|profile| {
            profile
                .display_name
                .clone()
                .or_else(|| profile.name.clone())
        })
        .filter(|name| !name.trim().is_empty())
        .unwrap_or_else(|| settings.user_id.clone());
    Ok(CurrentIdentity::Account {
        user_id: settings.user_id.clone(),
        did: did.to_string(),
        user_type: settings.user_type,
        show_name,
        zone_host: zone_host.to_string(),
        auth_method: token_auth_method(token),
        restricted: ceiling.restricted,
        scopes: ceiling.scopes.into_iter().collect(),
        password_set: settings.password_state.is_set(),
    })
}

/// The identity of an External session: the DID is the principal; the display
/// name is the one its canonical zone shared at login, or the DID's own id.
pub(crate) fn external_identity(
    token: &RPCSessionToken,
    did: &str,
    profile: Option<&FederationProfile>,
    zone_host: &str,
) -> Result<CurrentIdentity, RPCErrors> {
    let ceiling = token_session_ceiling(token)?;
    let show_name = profile
        .map(|profile| profile.show_name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| did.rsplit(':').next().unwrap_or(did).to_string());
    Ok(CurrentIdentity::External {
        did: did.to_string(),
        show_name,
        zone_host: zone_host.to_string(),
        auth_method: token_auth_method(token),
        restricted: ceiling.restricted,
        scopes: ceiling.scopes.into_iter().collect(),
    })
}

impl ControlPanelServer {
    /// The identity a session token speaks for. The token is one verify-hub has just
    /// issued or the dispatcher has verified; accounts are read with the service
    /// identity, and only `users/<id>/doc` gives the DID.
    pub(crate) async fn current_identity_for_token(
        &self,
        session_token: &str,
    ) -> Result<CurrentIdentity, RPCErrors> {
        let token = RPCSessionToken::from_string(session_token)?;
        let claims = validate_verify_hub_token_claims(&token, TokenUse::Session)?;
        if claims.principal_kind != TokenPrincipalKind::User {
            return Err(RPCErrors::InvalidToken(
                "current identity needs a user session".to_string(),
            ));
        }
        let subject = token
            .sub
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| RPCErrors::InvalidToken("session token missing subject".to_string()))?
            .to_string();
        let runtime = get_buckyos_api_runtime()?;
        let zone_host = Self::sso_zone_host(runtime)?;
        if buckyos_api::token_is_external(&token) {
            let profile = self.external_profile(&subject);
            return external_identity(&token, &subject, profile.as_ref(), &zone_host);
        }
        let client = runtime.get_system_config_client().await?;
        let settings_value = client
            .get(&format!("users/{subject}/settings"))
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("load account {subject}: {error}")))?;
        let settings: UserSettings = serde_json::from_str(&settings_value.value)
            .map_err(|error| RPCErrors::ReasonError(format!("account {subject}: {error}")))?;
        let did = ControlPanelClient::from_shared(client.clone())
            .get_user_did(&subject)
            .await?;
        let profile = crate::user_mgr::load_user_profile(&client, &subject)
            .await
            .unwrap_or_else(|error| {
                warn!("identity of {subject} shows the account id: {error}");
                None
            });
        account_identity(&token, &settings, &did, profile.as_ref(), &zone_host)
    }

    pub(crate) fn remember_external_profile(&self, did: &str, profile: FederationProfile) {
        let mut cache = self.external_profiles.lock().unwrap();
        if cache.len() >= EXTERNAL_PROFILE_CACHE_LIMIT && !cache.contains_key(did) {
            cache.clear();
        }
        cache.insert(did.to_string(), profile);
    }

    fn external_profile(&self, did: &str) -> Option<FederationProfile> {
        self.external_profiles.lock().unwrap().get(did).cloned()
    }

    /// `auth.current_identity {}` → `CurrentIdentity`; any verified user session,
    /// restricted ones included.
    pub(crate) async fn handle_auth_current_identity(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_rpc_principal(principal)?;
        if !principal.is_user_session {
            return Err(AccessDenial::new(DenyReason::SessionKindNotAllowed)
                .at(CONTROL_PANEL_SERVICE_UNIQUE_ID, "auth.current_identity")
                .into_rpc_error());
        }
        let token = Self::extract_rpc_session_token(&req)
            .ok_or_else(|| RPCErrors::InvalidToken("missing session token".to_string()))?;
        let identity = self.current_identity_for_token(&token).await?;
        Ok(RPCResponse::new(
            RPCResult::Success(json!(identity)),
            req.seq,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        bind_token_auth_method, bind_token_ceiling, AuthMethod, PasswordState, SessionCeiling,
        SessionScope, UserState, UserType,
    };
    use std::collections::HashMap;

    fn token(sub: &str) -> RPCSessionToken {
        RPCSessionToken {
            token_type: ::kRPC::RPCSessionTokenType::JWT,
            token: None,
            aud: None,
            exp: Some(u64::MAX),
            iss: Some(buckyos_api::VERIFY_HUB_TOKEN_ISSUER.to_string()),
            jti: Some("1".to_string()),
            sub: Some(sub.to_string()),
            appid: None,
            sudo: false,
            extra: HashMap::new(),
        }
    }

    fn settings(user_type: UserType, password_state: PasswordState) -> UserSettings {
        UserSettings {
            user_id: "acme-c1".to_string(),
            user_type,
            password: String::new(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
            allow_contact_edit: None,
            groups: Vec::new(),
            password_state,
        }
    }

    #[test]
    fn account_identity_carries_ceiling_method_and_password_state() {
        let mut limited = token("acme-c1");
        bind_token_ceiling(
            &mut limited,
            &SessionCeiling::for_user_type(UserType::Limited),
        );
        bind_token_auth_method(&mut limited, AuthMethod::ExclusiveInvite);
        let mut profile: UserPrivateProfile =
            serde_json::from_value(json!({"did": "did:bns:acme-c1.test", "display_name": "Alice"}))
                .unwrap();
        let did = DID::from_str("did:bns:acme-c1.test").unwrap();
        let identity = account_identity(
            &limited,
            &settings(UserType::Limited, PasswordState::Unset),
            &did,
            Some(&profile),
            "test.buckyos.io",
        )
        .unwrap();
        let fixture: CurrentIdentity = serde_json::from_str(include_str!(
            "../../../kernel/buckyos-api/tests/fixtures/hybrid_sso/current_identity_account.json"
        ))
        .unwrap();
        assert_eq!(identity, fixture);

        // Without a display name the account id is shown; an ordinary session is unrestricted.
        profile.display_name = None;
        profile.name = None;
        let mut admin = token("acme-c1");
        bind_token_auth_method(&mut admin, AuthMethod::Password);
        let identity = account_identity(
            &admin,
            &settings(UserType::Admin, PasswordState::Set),
            &did,
            Some(&profile),
            "test.buckyos.io",
        )
        .unwrap();
        let wire = serde_json::to_value(&identity).unwrap();
        assert_eq!(wire["show_name"], "acme-c1");
        assert_eq!(wire["restricted"], false);
        assert_eq!(wire["password_set"], true);
        assert!(wire.get("scopes").is_none());

        let mut suspended = settings(UserType::User, PasswordState::Set);
        suspended.state = UserState::Suspended("abuse".to_string());
        let error =
            account_identity(&admin, &suspended, &did, None, "test.buckyos.io").unwrap_err();
        assert_eq!(
            AccessDenial::from_rpc_error(&error).map(|denial| denial.reason),
            Some(DenyReason::AccountState)
        );
    }

    #[test]
    fn external_identity_uses_the_shared_profile_or_the_did() {
        let mut external = token("did:bns:alice");
        buckyos_api::bind_token_external(&mut external);
        bind_token_ceiling(&mut external, &SessionCeiling::external());
        bind_token_auth_method(&mut external, AuthMethod::DidFederation);
        let profile = FederationProfile {
            show_name: "Alice".to_string(),
            avatar: None,
        };
        let identity =
            external_identity(&external, "did:bns:alice", Some(&profile), "b.example").unwrap();
        let fixture: CurrentIdentity = serde_json::from_str(include_str!(
            "../../../kernel/buckyos-api/tests/fixtures/hybrid_sso/current_identity_external.json"
        ))
        .unwrap();
        assert_eq!(identity, fixture);
        let CurrentIdentity::External {
            show_name, scopes, ..
        } = external_identity(&external, "did:bns:alice", None, "b.example").unwrap()
        else {
            panic!("external identity expected");
        };
        assert_eq!(show_name, "alice");
        assert!(!scopes.contains(&SessionScope::AccountSelf));
    }
}
