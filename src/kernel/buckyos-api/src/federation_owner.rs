//! Resolve an identity for federation or an invitation, including accounts hosted by a BNS Zone.
use bns_client::{BnsIndexerApi, BnsIndexerClient};
use buckyos_kit::BuckyOSMachineConfig;
use name_client::{Budget, DidResolutionEnvelope, Revision};
use name_lib::{DIDDocumentTrait, DidDocType, NSError, NSResult, OwnerDocument, ZoneDocument, DID};
use std::time::Duration;

fn hosted_bns_parent(did: &DID) -> Option<DID> {
    if did.method != "bns" {
        return None;
    }
    let (user, parent) = did.id.split_once('.')?;
    if parent
        .split('.')
        .any(|label| name_lib::validate_zone_child_label(label).is_err())
    {
        return None;
    }
    let parent = DID::new("bns", parent);
    if name_lib::zone_child_did(&parent, user).ok().as_ref() != Some(did) {
        return None;
    }
    Some(parent)
}

fn https_origin(host: &str) -> NSResult<String> {
    if host.len() > 253
        || host
            .split('.')
            .any(|label| name_lib::validate_zone_child_label(label).is_err())
    {
        return Err(NSError::InvalidDID("invalid resolver host".into()));
    }
    let url = url::Url::parse(&format!("https://{host}"))
        .map_err(|_| NSError::InvalidDID("invalid resolver host".into()))?;
    if url.domain() != Some(host)
        || url.port().is_some()
        || url.path() != "/"
        || url.fragment().is_some()
        || url.query().is_some()
    {
        return Err(NSError::InvalidDID("invalid resolver host".into()));
    }
    Ok(url.origin().ascii_serialization())
}

fn hosted_owner(
    did: &DID,
    parent: &DID,
    envelope: DidResolutionEnvelope,
) -> NSResult<OwnerDocument> {
    let denied = || NSError::InvalidDID("invalid hosted OwnerDocument".into());
    let metadata = envelope.did_document_metadata.as_ref().ok_or_else(denied)?;
    let publication = metadata.buckyos.as_ref().ok_or_else(denied)?;
    if envelope.error_code().is_some()
        || metadata.deactivated == Some(true)
        || publication.doc_type.as_deref() != Some("owner")
        || publication.document_status.as_deref() != Some("active")
        || publication.effective_owner.as_deref() != Some(&did.to_string())
    {
        return Err(denied());
    }
    let document = envelope.document()?.ok_or_else(denied)?;
    let owner = OwnerDocument::decode(&document, None)?;
    let public_key = owner.get_default_key().ok_or_else(denied)?;
    // This namespace belongs to the HTTPS Zone resolver. A signed response must also
    // verify under the account key. Unsigned JSON is the Zone's normal hosted-account response.
    if document.is_proof() {
        let key = jsonwebtoken::DecodingKey::from_jwk(&public_key).map_err(|_| denied())?;
        OwnerDocument::decode(&document, Some(&key))?;
    }
    let now = buckyos_kit::buckyos_get_unix_timestamp();
    if owner.id != *did
        || owner.get_default_zone_did().as_ref() != Some(parent)
        || owner.exp <= now
        || owner.iat > now.saturating_add(60)
    {
        return Err(denied());
    }
    Ok(owner)
}

async fn resolve_owner(did: &DID) -> NSResult<OwnerDocument> {
    let missing = match name_client::resolve_owner_document(did).await {
        Ok(owner) if owner.id == *did => return Ok(owner),
        Ok(_) => return Err(NSError::InvalidDID("OwnerDocument DID mismatch".into())),
        Err(err @ NSError::NotFound(_)) => err,
        Err(err) => return Err(err),
    };
    let Some(parent) = hosted_bns_parent(did) else {
        return Err(missing);
    };
    // A registered or revoked subname has its own authority. A missing document alone
    // cannot distinguish it from an unregistered name in the parent's account namespace.
    let registry = BuckyOSMachineConfig::load_machine_config()
        .unwrap_or_default()
        .bns_resolver_host();
    let bns = BnsIndexerClient::new_bns_server_url(&https_origin(&registry)?, None);
    if bns
        .query_name_state(&did.id)
        .await
        .map_err(|e| NSError::Failed(format!("query BNS name: {e}")))?
        .is_some()
    {
        return Err(missing);
    }
    let encoded = name_client::resolve_did(&parent, Some(DidDocType::Zone)).await?;
    let zone = ZoneDocument::decode(&encoded, None)?;
    if zone.id != parent {
        return Err(NSError::InvalidDID("parent Zone DID mismatch".into()));
    }
    let host = crate::zone_document_hostname(&zone)
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let origin = https_origin(&host)?;
    let endpoint = name_client::resolver_api_url(
        &origin,
        did,
        Some(&DidDocType::Owner),
        &Revision::current(),
    )?;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| NSError::Failed(e.to_string()))?;
    let response = client
        .get(endpoint)
        .header(reqwest::header::ACCEPT, name_client::DID_RESOLUTION_ACCEPT)
        .send()
        .await
        .map_err(|e| NSError::Failed(format!("hosted account resolver: {e}")))?;
    if !response.status().is_success() {
        return Err(missing);
    }
    let parts = name_client::read_response(response, &Budget::new(Duration::from_secs(5))).await?;
    let envelope = serde_json::from_slice(&parts.body)
        .map_err(|e| NSError::Failed(format!("invalid DID resolution response: {e}")))?;
    hosted_owner(did, &parent, envelope)
}

pub async fn resolve_federation_owner(did: &DID) -> NSResult<OwnerDocument> {
    tokio::time::timeout(Duration::from_secs(15), resolve_owner(did))
        .await
        .map_err(|_| NSError::Failed("federation owner resolution timed out".into()))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_a_bns_account_label_may_use_its_immediate_parent() {
        assert_eq!(
            hosted_bns_parent(&DID::new("bns", "ann.family.alice")),
            Some(DID::new("bns", "family.alice"))
        );
        for did in [
            DID::new("bns", "alice"),
            DID::new("web", "ann.alice.com"),
            DID::new("bns", "ann..alice"),
            DID::new("bns", "bad:obj.alice"),
        ] {
            assert_eq!(hosted_bns_parent(&did), None);
        }
        for bad in [
            "127.0.0.1",
            "good.test@evil.test",
            "good.test/path",
            "good.test:444",
            "bad..test",
        ] {
            assert!(https_origin(bad).is_err());
        }
    }

    #[test]
    fn hosted_document_requires_live_matching_owner_and_canonical_zone() {
        let did = DID::new("bns", "ann.alice");
        let parent = DID::new("bns", "alice");
        let (_, key) = name_lib::generate_ed25519_key_pair();
        let mut owner = OwnerDocument::new(
            did.clone(),
            "ann".into(),
            "Ann".into(),
            serde_json::from_value(key).unwrap(),
        );
        owner.set_default_zone_did(parent.clone());
        let original = json!({"didDocument":owner,"didResolutionMetadata":{},
            "didDocumentMetadata":{"deactivated":false,"buckyos":{"documentStatus":"active","docType":"owner","effectiveOwner":did.to_string()}}});
        let check = |v| hosted_owner(&did, &parent, serde_json::from_value(v).unwrap()).is_ok();
        assert!(check(original.clone()));
        for (pointer, value) in [
            ("/didDocument/id", json!("did:bns:bob.alice")),
            ("/didDocument/binded_zone_list", json!(["did:bns:other"])),
            ("/didDocument/exp", json!(1)),
            ("/didDocumentMetadata/deactivated", json!(true)),
            (
                "/didDocumentMetadata/buckyos/documentStatus",
                json!("revoked"),
            ),
            ("/didDocumentMetadata/buckyos/docType", json!("zone")),
            (
                "/didDocumentMetadata/buckyos/effectiveOwner",
                json!("did:bns:other"),
            ),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(pointer).unwrap() = value;
            assert!(!check(changed), "{pointer}");
        }
    }
}
