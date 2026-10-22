//! DID federation (contract §9, D-7). Canonical side: one-time codes bound to the
//! target zone's pending login by PKCE. Target side: the canonical host comes only
//! from the DID resolution chain, and the code is exchanged there over HTTPS.

use ::kRPC::{kRPC, RPCErrors};
use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use buckyos_api::{
    constant_time_eq, zone_document_hostname, ExchangeFederationCodeRequest,
    ExchangeFederationCodeResult, FederationAssertion, FederationProfile, VerifyHubClient,
    FEDERATION_PURPOSE_LOGIN,
};
use name_lib::{DIDDocumentTrait, DidDocType, ZoneDocument, DID};
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

type Result<T> = std::result::Result<T, RPCErrors>;

pub const FEDERATION_CODE_TTL_SECONDS: u64 = 60;
/// Q-04: a federated family lives at most this long, then federates again.
pub const FEDERATION_SESSION_MAX_SECONDS: u64 = 30 * 24 * 3600;

pub struct FederationCode {
    pub did: String,
    pub client_zone: String,
    pub redirect_uri: String,
    pub nonce: String,
    pub code_challenge: String,
    pub profile: FederationProfile,
    pub issued_at: u64,
}

/// Codes this zone issued as a canonical zone; in memory, single use.
#[derive(Default)]
pub struct FederationCodes {
    codes: Mutex<HashMap<String, FederationCode>>,
}

pub fn pkce_s256(code_verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code_verifier.as_bytes()))
}

impl FederationCodes {
    pub fn issue(&self, code: FederationCode, now: u64) -> String {
        let mut secret = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut secret);
        let id = format!("fc-{}", URL_SAFE_NO_PAD.encode(secret));
        let mut codes = self
            .codes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        codes.retain(|_, code| code.issued_at + FEDERATION_CODE_TTL_SECONDS > now);
        codes.insert(id.clone(), code);
        id
    }

    /// Any lookup that finds the code consumes it, whether the exchange succeeds or not.
    pub fn exchange(
        &self,
        request: &ExchangeFederationCodeRequest,
        now: u64,
    ) -> ExchangeFederationCodeResult {
        let code = self
            .codes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&request.code);
        let Some(code) = code else {
            return ExchangeFederationCodeResult::InvalidCode;
        };
        if now >= code.issued_at + FEDERATION_CODE_TTL_SECONDS
            || code.client_zone != request.client_zone
            || code.redirect_uri != request.redirect_uri
            || !constant_time_eq(
                pkce_s256(&request.code_verifier).as_bytes(),
                code.code_challenge.as_bytes(),
            )
        {
            return ExchangeFederationCodeResult::InvalidCode;
        }
        ExchangeFederationCodeResult::Ok {
            assertion: FederationAssertion {
                did: code.did,
                client_zone: code.client_zone,
                nonce: code.nonce,
                purpose: FEDERATION_PURPOSE_LOGIN.to_string(),
                issued_at: code.issued_at,
                profile: code.profile,
            },
        }
    }
}

/// The target zone's view of a DID's canonical zone.
#[async_trait]
pub trait FederationPeer: Send + Sync {
    /// Hostname of the DID's canonical zone: DID → OwnerDocument → default zone →
    /// ZoneDocument hostname. An error means the canonical zone is unavailable.
    async fn canonical_host(&self, did: &DID) -> Result<String>;

    async fn exchange(
        &self,
        canonical_host: &str,
        use_http: bool,
        request: ExchangeFederationCodeRequest,
    ) -> Result<ExchangeFederationCodeResult>;
}

/// Resolution through name-client, exchange through the canonical `/kapi/verify-hub`.
pub struct NameFederationPeer;

fn resolve_error(step: &str, error: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(format!("{step}: {error}"))
}

#[async_trait]
impl FederationPeer for NameFederationPeer {
    async fn canonical_host(&self, did: &DID) -> Result<String> {
        let owner = name_client::resolve_owner_document(did)
            .await
            .map_err(|error| resolve_error("resolve OwnerDocument", error))?;
        let zone_did = owner.get_default_zone_did().ok_or_else(|| {
            RPCErrors::ReasonError(format!("{} names no default zone", did.to_string()))
        })?;
        let encoded = name_client::resolve_did(&zone_did, Some(DidDocType::Zone))
            .await
            .map_err(|error| resolve_error("resolve ZoneDocument", error))?;
        let zone_document = ZoneDocument::decode(&encoded, None)
            .map_err(|error| resolve_error("decode ZoneDocument", error))?;
        let host = zone_document_hostname(&zone_document)
            .trim_end_matches('.')
            .to_ascii_lowercase();
        if host.is_empty()
            || host
                .chars()
                .any(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '.'))
        {
            return Err(RPCErrors::ReasonError(format!(
                "ZoneDocument of {} has no usable hostname",
                zone_did.to_string()
            )));
        }
        Ok(host)
    }

    async fn exchange(
        &self,
        canonical_host: &str,
        use_http: bool,
        request: ExchangeFederationCodeRequest,
    ) -> Result<ExchangeFederationCodeResult> {
        let scheme = if use_http { "http" } else { "https" };
        let url = format!("{scheme}://{canonical_host}/kapi/verify-hub");
        VerifyHubClient::new(kRPC::new(&url, None))
            .exchange_federation_code(request)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(code: &str, verifier: &str) -> ExchangeFederationCodeRequest {
        ExchangeFederationCodeRequest {
            code: code.to_string(),
            code_verifier: verifier.to_string(),
            client_zone: "did:bns:b".to_string(),
            redirect_uri: "https://b.example/sso/did_callback".to_string(),
        }
    }

    fn issue(codes: &FederationCodes, now: u64) -> String {
        codes.issue(
            FederationCode {
                did: "did:bns:alice".into(),
                client_zone: "did:bns:b".into(),
                redirect_uri: "https://b.example/sso/did_callback".into(),
                nonce: "n-1".into(),
                code_challenge: pkce_s256("verifier"),
                profile: FederationProfile {
                    show_name: "Alice".into(),
                    avatar: None,
                },
                issued_at: now,
            },
            now,
        )
    }

    #[test]
    fn codes_are_single_use_and_bound_to_verifier_zone_and_redirect() {
        let codes = FederationCodes::default();
        let now = 1_000;
        let code = issue(&codes, now);
        assert!(code.starts_with("fc-"));
        match codes.exchange(&request(&code, "verifier"), now + 1) {
            ExchangeFederationCodeResult::Ok { assertion } => {
                assert_eq!(assertion.did, "did:bns:alice");
                assert_eq!(assertion.nonce, "n-1");
                assert_eq!(assertion.purpose, FEDERATION_PURPOSE_LOGIN);
            }
            other => panic!("expected ok, got {other:?}"),
        }
        assert_eq!(
            codes.exchange(&request(&code, "verifier"), now + 1),
            ExchangeFederationCodeResult::InvalidCode
        );

        let code = issue(&codes, now);
        assert_eq!(
            codes.exchange(&request(&code, "wrong"), now),
            ExchangeFederationCodeResult::InvalidCode
        );
        // A failed attempt burns the code.
        assert_eq!(
            codes.exchange(&request(&code, "verifier"), now),
            ExchangeFederationCodeResult::InvalidCode
        );

        let code = issue(&codes, now);
        let mut other_zone = request(&code, "verifier");
        other_zone.client_zone = "did:bns:c".into();
        assert_eq!(
            codes.exchange(&other_zone, now),
            ExchangeFederationCodeResult::InvalidCode
        );

        let code = issue(&codes, now);
        let mut other_redirect = request(&code, "verifier");
        other_redirect.redirect_uri = "https://evil.example/sso/did_callback".into();
        assert_eq!(
            codes.exchange(&other_redirect, now),
            ExchangeFederationCodeResult::InvalidCode
        );

        let code = issue(&codes, now);
        assert_eq!(
            codes.exchange(
                &request(&code, "verifier"),
                now + FEDERATION_CODE_TTL_SECONDS
            ),
            ExchangeFederationCodeResult::InvalidCode
        );
    }
}
