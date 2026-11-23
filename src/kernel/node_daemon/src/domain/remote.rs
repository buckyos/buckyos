//! The SN and BNS side of [`super::service`].

use std::sync::Arc;

use ::kRPC::RPCErrors;
use async_trait::async_trait;
use bns_client::{BnsIndexerApi, BnsIndexerClient};
use cyfs_gateway_api::{SnClient, SnDnsRecordReq};
use jsonwebtoken::jwk::Jwk;
use name_lib::OwnerDocument;

use super::check::DomainBinding;
use super::plan::canonical_domain;
use super::service::{ChallengeFailure, ChallengeWriter, OwnerKeys};
use super::{DomainError, DomainResult};

/// With the account's own session: activation.
pub struct AccountBinding {
    pub sn: Arc<SnClient>,
}

#[async_trait]
impl DomainBinding for AccountBinding {
    async fn bound(&self, domain: &str) -> Result<bool, RPCErrors> {
        let profile = self.sn.get_profile().await?;
        Ok(profile
            .user_domain
            .as_deref()
            .and_then(canonical_domain)
            .is_some_and(|bound| bound == domain))
    }

    async fn bind(&self, domain: &str) -> Result<(), RPCErrors> {
        let result = self.sn.bind_domain(domain).await?;
        if result.domain != domain {
            return Err(RPCErrors::ReasonError(format!(
                "SN verified {} instead of {domain}",
                result.domain
            )));
        }
        Ok(())
    }
}

/// An activated node has no account session: it can see whose the domain is,
/// but not bind it.
pub struct HostnameBinding {
    pub sn: Arc<SnClient>,
    pub owner_name: String,
}

#[async_trait]
impl DomainBinding for HostnameBinding {
    async fn bound(&self, domain: &str) -> Result<bool, RPCErrors> {
        match self.sn.resolve_ood_by_hostname(domain).await {
            Ok(ood) => Ok(ood.owner_id == self.owner_name),
            Err(error)
                if super::check::sn_error_name(&error).as_deref() == Some("hostname_not_found") =>
            {
                Ok(false)
            }
            Err(error) => Err(error),
        }
    }

    async fn bind(&self, _domain: &str) -> Result<(), RPCErrors> {
        Err(RPCErrors::InvalidToken(
            "binding a domain needs the account's session".to_string(),
        ))
    }
}

/// Writes the challenge at the SN, as the device that will run ACME.
pub struct SnChallenge {
    pub sn: Arc<SnClient>,
    pub device_did: String,
}

impl SnChallenge {
    fn request(&self, name: &str, value: &str) -> SnDnsRecordReq {
        SnDnsRecordReq {
            device_did: self.device_did.clone(),
            domain: name.to_string(),
            record_type: "TXT".to_string(),
            record: Some(value.to_string()),
            ttl: Some(60),
            has_cert: None,
        }
    }
}

fn write_failed(error: RPCErrors) -> ChallengeFailure {
    ChallengeFailure {
        code: "challenge_write_failed",
        detail: error.to_string(),
    }
}

#[async_trait]
impl ChallengeWriter for SnChallenge {
    async fn add(&self, name: &str, value: &str) -> Result<(), ChallengeFailure> {
        self.sn
            .add_dns_record(self.request(name, value))
            .await
            .map(|_| ())
            .map_err(write_failed)
    }

    async fn remove(&self, name: &str, value: &str) -> Result<(), ChallengeFailure> {
        self.sn
            .remove_dns_record(self.request(name, value))
            .await
            .map(|_| ())
            .map_err(write_failed)
    }
}

/// `Ok(None)`: no such name.
pub async fn read_owner_document(
    bns_url: &str,
    owner_name: &str,
) -> DomainResult<Option<OwnerDocument>> {
    let client = BnsIndexerClient::new_bns_server_url(bns_url, None);
    let projected = match client.resolve_document(owner_name, "owner").await {
        Ok(projected) => projected,
        Err(error)
            if error.is_registry_code("DOCUMENT_NOT_FOUND")
                || error.is_registry_code("NAME_NOT_FOUND") =>
        {
            return Ok(None)
        }
        Err(error) => return Err(DomainError::new("bns_unavailable", error.to_string())),
    };
    serde_json::from_slice(projected.document_state.document.inline_document.as_slice())
        .map(Some)
        .map_err(|error| DomainError::new("owner_document_invalid", error.to_string()))
}

pub struct BnsOwnerKeys {
    pub bns_url: String,
}

#[async_trait]
impl OwnerKeys for BnsOwnerKeys {
    async fn owner_key(&self, owner_name: &str) -> DomainResult<Jwk> {
        read_owner_document(&self.bns_url, owner_name)
            .await?
            .and_then(|document| document.get_default_key())
            .ok_or_else(|| DomainError::new("owner_not_found", owner_name.to_string()))
    }
}

/// The owner key an activated node trusts; it was read from BNS at activation.
pub struct LocalOwnerKey {
    pub owner_name: String,
    pub key: Jwk,
}

#[async_trait]
impl OwnerKeys for LocalOwnerKey {
    async fn owner_key(&self, owner_name: &str) -> DomainResult<Jwk> {
        if owner_name != self.owner_name {
            return Err(DomainError::new("owner_not_found", owner_name.to_string()));
        }
        Ok(self.key.clone())
    }
}
