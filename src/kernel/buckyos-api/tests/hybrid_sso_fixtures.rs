//! Wire fixtures shared with the WebSDK (`tests/hybrid_sso_fixtures.test.ts`):
//! each file must parse into the typed model and serialize back unchanged.

use buckyos_api::*;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;

fn fixture(name: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/hybrid_sso")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

fn round_trip<T: DeserializeOwned + Serialize>(name: &str) -> T {
    let wire = fixture(name);
    let typed: T = serde_json::from_value(wire.clone()).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(
        serde_json::to_value(&typed).unwrap(),
        wire,
        "{name} does not round-trip"
    );
    typed
}

#[test]
fn identity_fixtures_round_trip() {
    match round_trip::<CurrentIdentity>("current_identity_account.json") {
        CurrentIdentity::Account {
            user_type,
            restricted,
            scopes,
            password_set,
            ..
        } => {
            assert_eq!(user_type, UserType::Limited);
            assert!(restricted && !password_set);
            assert_eq!(scopes.len(), 4);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        round_trip::<CurrentIdentity>("current_identity_admin.json"),
        CurrentIdentity::Account {
            restricted: false,
            ..
        }
    ));
    assert!(matches!(
        round_trip::<CurrentIdentity>("current_identity_external.json"),
        CurrentIdentity::External {
            restricted: true,
            ..
        }
    ));
    assert_eq!(
        round_trip::<CurrentIdentity>("current_identity_anonymous.json"),
        CurrentIdentity::Anonymous
    );
}

#[test]
fn denial_fixtures_match_the_registered_adjusters() {
    for name in [
        "access_denial_grant_missing.json",
        "access_denial_invite_expired.json",
        "access_denial_zone_disabled.json",
        "access_denial_kind_not_allowed.json",
    ] {
        let denial: AccessDenial = round_trip(name);
        assert_eq!(denial.adjust, denial.reason.adjust(), "{name}");
    }
    let wire = fixture("rpc_failure_access_denied.json");
    let detail = ::kRPC::RPCErrorDetail::from_response_value(&wire).unwrap();
    let error = ::kRPC::RPCErrors::Detailed(detail);
    let denial = AccessDenial::from_rpc_error(&error).unwrap();
    assert_eq!(denial.reason, DenyReason::ResourceGrantMissing);
}

#[test]
fn entry_and_settings_fixtures_round_trip() {
    let entry: EntryDescriptor = round_trip("entry_descriptor.json");
    assert_eq!(entry.zone_disabled, vec![EntryMethod::DidLogin]);
    let unavailable: EntryDescriptor = round_trip("entry_descriptor_unavailable.json");
    assert_eq!(unavailable.state, EntryState::Unavailable);
    let config: SsoPublicConfig = round_trip("sso_public_config.json");
    assert!(!config.did_login_enabled);
}

#[test]
fn verify_hub_result_fixtures_round_trip() {
    assert!(matches!(
        round_trip::<ExclusiveInviteLoginResult>("exclusive_invite_login_issued.json"),
        ExclusiveInviteLoginResult::Issued { .. }
    ));
    match round_trip::<ExclusiveInviteLoginResult>("exclusive_invite_login_denied.json") {
        ExclusiveInviteLoginResult::Denied { denial } => {
            assert_eq!(denial.detail.as_deref(), Some("password_required"))
        }
        other => panic!("unexpected {other:?}"),
    }
    assert!(matches!(
        round_trip::<LoginByRemoteVerifyHubResult>("remote_login_issued_external.json"),
        LoginByRemoteVerifyHubResult::Issued { user_id: None, .. }
    ));
    assert!(matches!(
        round_trip::<LoginByRemoteVerifyHubResult>("remote_login_canonical_unavailable.json"),
        LoginByRemoteVerifyHubResult::CanonicalUnavailable
    ));
    assert!(matches!(
        round_trip::<ExchangeFederationCodeResult>("federation_exchange_ok.json"),
        ExchangeFederationCodeResult::Ok { .. }
    ));
    assert_eq!(
        round_trip::<ExchangeFederationCodeResult>("federation_exchange_invalid.json"),
        ExchangeFederationCodeResult::InvalidCode
    );
}

#[test]
fn exclusive_invite_record_round_trips_and_digest_matches() {
    let record: ExclusiveInvite = round_trip("exclusive_invite_record.json");
    assert_eq!(record.credential.digest, invite_secret_digest("secret"));
}
