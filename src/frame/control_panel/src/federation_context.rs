//! The requesting Zone signs the purpose shown on the home Zone's consent page.
//! Only a device key published by the requesting Zone for one of its OODs is trusted.
use crate::federation::FederationParams;
use ::kRPC::RPCErrors;
use buckyos_api::{AuthTarget, DeviceSigner};
use jsonwebtoken::{decode, decode_header, Algorithm, Validation};
use name_lib::{DIDDocumentTrait, ZoneDocument};
use serde::{Deserialize, Serialize};

const TTL: u64 = 600;

#[derive(Serialize, Deserialize, PartialEq)]
struct AppContext {
    purpose: String,
    client_zone: String,
    did: String,
    redirect_uri: String,
    state: String,
    nonce: String,
    code_challenge: String,
    target: AuthTarget,
    host: String,
    iat: u64,
    exp: u64,
}

fn invalid() -> RPCErrors {
    RPCErrors::InvalidToken("invalid federation app_context".into())
}

pub(super) fn sign(
    signer: &DeviceSigner,
    params: &FederationParams,
    target: AuthTarget,
    host: String,
    now: u64,
) -> Result<String, RPCErrors> {
    signer.sign(&AppContext {
        purpose: "federation_app_context".into(),
        client_zone: params.client_zone.clone(),
        did: params.did.clone(),
        redirect_uri: params.redirect_uri.clone(),
        state: params.state.clone(),
        nonce: params.nonce.clone(),
        code_challenge: params.code_challenge.clone(),
        target,
        host,
        iat: now,
        exp: now + TTL,
    })
}

/// Compact public Zone documents name their OODs but publish each signed device document separately.
pub(super) async fn load_signing_ood(
    params: &FederationParams,
    zone: &mut ZoneDocument,
) -> Result<(), RPCErrors> {
    if params.app_context.len() > 8192 || zone.id.to_string() != params.client_zone {
        return Err(invalid());
    }
    let header = decode_header(&params.app_context).map_err(|_| invalid())?;
    let kid = header.kid.ok_or_else(invalid)?;
    if header.alg != Algorithm::EdDSA {
        return Err(invalid());
    }
    for ood in zone.oods.iter().filter(|ood| ood.node_type.is_ood()) {
        if let Some(device) = zone.get_device_document(&ood.name) {
            if format!("{}#main_key", device.id.to_string()) == kid {
                return Ok(());
            }
            continue;
        }
        let id = name_lib::zone_child_did(&zone.id, &ood.name).map_err(|_| invalid())?;
        if format!("{}#main_key", id.to_string()) != kid {
            continue;
        }
        let encoded = crate::federation::resolve(&id, name_lib::DidDocType::Device).await?;
        if !matches!(&encoded, name_lib::EncodedDocument::Jwt(_)) {
            return Err(invalid());
        }
        let key = zone.get_auth_key(Some("#main_key")).ok_or_else(invalid)?.0;
        let device =
            name_lib::DeviceDocument::decode(&encoded, Some(&key)).map_err(|_| invalid())?;
        if device.id != id
            || device.zone_did.as_ref() != Some(&zone.id)
            || device.name != ood.name
            || device.exp <= buckyos_kit::buckyos_get_unix_timestamp()
        {
            return Err(invalid());
        }
        zone.devices.insert(ood.name.clone(), device);
        return Ok(());
    }
    Err(invalid())
}

pub(super) fn verify(
    params: &FederationParams,
    zone: &ZoneDocument,
    zone_host: &str,
    now: u64,
) -> Result<(AuthTarget, String), RPCErrors> {
    let jwt = &params.app_context;
    if jwt.len() > 8192 {
        return Err(invalid());
    }
    let header = decode_header(jwt).map_err(|_| invalid())?;
    if header.alg != Algorithm::EdDSA {
        return Err(invalid());
    }
    let kid = header.kid.ok_or_else(invalid)?;
    let key = zone
        .oods
        .iter()
        .filter(|ood| ood.node_type.is_ood())
        .filter_map(|ood| zone.get_device_document(&ood.name))
        .find(|dev| format!("{}#main_key", dev.id.to_string()) == kid)
        .and_then(|dev| dev.get_auth_key(Some("#main_key")))
        .ok_or_else(invalid)?
        .0;
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.validate_exp = false;
    let c = decode::<AppContext>(jwt, &key, &validation)
        .map_err(|_| invalid())?
        .claims;
    if c.purpose != "federation_app_context"
        || c.client_zone != params.client_zone
        || zone.id.to_string() != params.client_zone
        || c.did != params.did
        || c.redirect_uri != params.redirect_uri
        || c.state != params.state
        || c.nonce != params.nonce
        || c.code_challenge != params.code_challenge
        || c.iat > now.saturating_add(30)
        || c.exp <= now
        || c.exp <= c.iat
        || c.exp - c.iat > TTL
        || !(c.host == zone_host
            || c.host
                .strip_suffix(zone_host)
                .is_some_and(|label| label.ends_with('.') && label.len() > 1))
        || url::Host::parse(&c.host)
            .ok()
            .is_none_or(|host| host.to_string() != c.host)
    {
        return Err(invalid());
    }
    Ok((c.target, c.host))
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::custody_cases::ALICE_OOD_KEY;
    use jsonwebtoken::{jwk::Jwk, EncodingKey};
    use name_lib::{DeviceDocument, DID};

    #[test]
    fn app_context_requires_published_ood_and_matches_this_request() {
        let zone_id = DID::from_str("did:bns:alice").unwrap();
        let key: Jwk = serde_json::from_value(serde_json::json!({
            "kty": "OKP", "crv": "Ed25519", "x": "T4Quc1L6Ogu4N2tTKOvneV1yYnBcmhP89B_RsuFsJZ8"
        }))
        .unwrap();
        let dev = DeviceDocument::new_by_jwk("ood1", key.clone());
        let signer = DeviceSigner::new(
            dev.id.clone(),
            EncodingKey::from_ed_pem(ALICE_OOD_KEY.as_bytes()).unwrap(),
        );
        let mut zone = ZoneDocument::new(zone_id.clone(), zone_id.clone(), key);
        zone.oods.push("ood1@wan".parse().unwrap());
        zone.devices.insert("ood1".to_string(), dev);
        let target = AuthTarget::from_canonical_key("system:aiworkspace").unwrap();
        let mut req = FederationParams {
            did: "did:bns:bob".into(),
            client_zone: zone_id.to_string(),
            redirect_uri: "https://alice.example/sso/did_callback".into(),
            state: "state".into(),
            nonce: "nonce".into(),
            code_challenge: "challenge".into(),
            remember: false,
            app_context: String::new(),
        };
        let host = "aiws-one.alice.example";
        req.app_context = sign(&signer, &req, target.clone(), host.into(), 100).unwrap();
        assert_eq!(
            verify(&req, &zone, "alice.example", 101).unwrap(),
            (target.clone(), host.to_string())
        );
        assert!(verify(&req, &zone, "alice.example", 100 + TTL).is_err());
        assert!(verify(&req, &zone, "other.example", 101).is_err());
        let mut other = req.clone();
        other.state = "different".into();
        assert!(verify(&other, &zone, "alice.example", 101).is_err());
        req.app_context = sign(&signer, &req, target, "other.example".into(), 100).unwrap();
        assert!(verify(&req, &zone, "alice.example", 101).is_err());
        other.state = req.state;
        zone.devices.clear();
        assert!(verify(&other, &zone, "alice.example", 101).is_err());
    }
}
