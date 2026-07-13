use ::kRPC::*;
use async_trait::async_trait;
use bns_client::{BnsIndexerApi, BnsIndexerClient};
use buckyos_api::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, Runner, ServerError, ServerErrorCode,
    ServerResult, StreamInfo,
};
use buckyos_kit::*;
use bytes::Bytes;
use cyfs_gateway_api::{
    SnBnsPublishDocumentContent, SnBnsPublishDocumentReq, SnClient, SnDeviceOnlineReportReq,
    SnZoneInfoResp,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use log::*;
use name_client::{update_did_cache, UpdateSource};
use name_lib::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::IpAddr;
use std::process::exit;
use std::result::Result;
use std::str::FromStr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::sn_zone_info::save_sn_zone_info;

const ACTIVE_SERVICE_MAIN_PORT: u16 = 3182;
const DEFAULT_RTCP_PORT: u32 = 2980;
const MAX_INLINE_DOCUMENT: usize = 4096;
const OWNER_DERIVATION_PATH: &str = "m/9777'/0'/0'";
const EVM_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";
const PROJECTION_DEADLINE: Duration = Duration::from_secs(60);

fn parse_params<T: DeserializeOwned>(value: Value, type_name: &str) -> Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParseRequestError(format!("Failed to parse {}: {}", type_name, error))
    })
}

macro_rules! impl_from_json {
    ($type:ty) => {
        impl $type {
            pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
                parse_params(value, stringify!($type))
            }
        }
    };
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ActiveNameMapping {
    pub owner_name: String,
    pub owner_did: DID,
    pub zone_did: DID,
    pub access_hostname: String,
    pub bns_publish_name: String,
    pub use_self_domain: bool,
}

impl ActiveNameMapping {
    fn derive(
        owner_document: &OwnerDocument,
        access_hostname: &str,
        use_self_domain: bool,
    ) -> Self {
        let access_hostname = access_hostname.trim().to_lowercase();
        Self {
            owner_name: owner_document.name.clone(),
            owner_did: owner_document.id.clone(),
            zone_did: if use_self_domain {
                DID::new("web", access_hostname.as_str())
            } else {
                owner_document.id.clone()
            },
            access_hostname,
            bns_publish_name: owner_document.name.clone(),
            use_self_domain,
        }
    }

    fn validate(&self, owner_document: &OwnerDocument) -> Result<(), RPCErrors> {
        let expected = Self::derive(
            owner_document,
            self.access_hostname.as_str(),
            self.use_self_domain,
        );
        if self != &expected {
            return Err(RPCErrors::ReasonError(
                "active name mapping is inconsistent with OwnerDocument".to_string(),
            ));
        }
        validate_hostname(self.access_hostname.as_str())?;
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GatewayTopology {
    pub net_id: String,
    #[serde(default = "default_rtcp_port")]
    pub rtcp_port: u32,
    #[serde(default = "default_true")]
    pub support_container: bool,
    pub uses_sn_relay: bool,
    pub sn_url: String,
}

fn default_rtcp_port() -> u32 {
    DEFAULT_RTCP_PORT
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct WebOwnerMaterial {
    pub mnemonic_words: Vec<String>,
    pub owner_public_jwk: Value,
    pub owner_derivation_path: String,
    pub evm_address: String,
    pub evm_derivation_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ActiveServiceConfig {
    sn_base_host: String,
    http_schema: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PreparedActiveDocuments {
    pub owner_document: OwnerDocument,
    pub names: ActiveNameMapping,
    pub topology: GatewayTopology,
    pub boot_document: ZoneBootDocument,
    pub device_document: DeviceDocument,
    pub device_mini_document: DeviceMiniDocument,
    pub device_info: DeviceInfo,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct SignedActiveDocuments {
    pub boot_document: ZoneBootDocument,
    pub boot_document_jwt: String,
    pub device_document: DeviceDocument,
    pub device_document_jwt: String,
    pub device_mini_document: DeviceMiniDocument,
    pub device_mini_document_jwt: String,
    pub zone_document: ZoneDocument,
    pub zone_document_jwt: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateWebOwnerMaterialReq {}
impl_from_json!(GenerateWebOwnerMaterialReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GenerateDeviceKeyPairReq {}
impl_from_json!(GenerateDeviceKeyPairReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceKeyPair {
    pub public_key: Value,
    pub private_key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrepareActiveDocumentsReq {
    pub owner_document: OwnerDocument,
    pub names: ActiveNameMapping,
    pub topology: GatewayTopology,
    pub device_public_key: Value,
}
impl_from_json!(PrepareActiveDocumentsReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssembleZoneDocumentReq {
    pub prepared: PreparedActiveDocuments,
    pub boot_document_jwt: String,
    pub device_document_jwt: String,
    pub device_mini_document_jwt: String,
}
impl_from_json!(AssembleZoneDocumentReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignWebActiveDocumentsReq {
    pub mnemonic_words: Vec<String>,
    pub prepared: PreparedActiveDocuments,
}
impl_from_json!(SignWebActiveDocumentsReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalSystemSettings {
    pub admin_password_hash: String,
    #[serde(default)]
    pub guest_access: bool,
    #[serde(default)]
    pub friend_passcode: String,
    #[serde(default)]
    pub enabled_features: Value,
    #[serde(default)]
    pub ai_provider_config: Value,
    #[serde(default)]
    pub jarvis_msg_tunnel_config: Value,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnCommitConfig {
    pub sn_url: String,
    pub bns_url: String,
    pub access_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitActiveReq {
    pub owner_document: OwnerDocument,
    pub prepared: PreparedActiveDocuments,
    pub signed_documents: SignedActiveDocuments,
    pub device_private_key: String,
    pub system_settings: LocalSystemSettings,
    pub sn: SnCommitConfig,
}
impl_from_json!(CommitActiveReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitActiveResp {
    pub status: String,
    pub access_hostname: String,
}

#[derive(Clone)]
struct ActiveServer {
    device_mini_info: DeviceMiniInfo,
    config: ActiveServiceConfig,
}

impl ActiveServer {
    fn new(config: ActiveServiceConfig) -> Self {
        Self {
            device_mini_info: DeviceMiniInfo::default(),
            config,
        }
    }

    async fn auto_fill_device_mini_info(&mut self) {
        if let Err(error) = self.device_mini_info.auto_fill_by_system_info().await {
            warn!("fill active device info failed: {}", error);
        }
        self.device_mini_info.active_url = Some("./index.html".to_string());
    }

    fn generate_web_owner_material(&self) -> Result<WebOwnerMaterial, RPCErrors> {
        let mnemonic = generate_buckyos_mnemonic()
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let owner = derive_bucky_key_from_mnemonic(mnemonic.as_str(), None, 0)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let evm = derive_evm_key_from_mnemonic(mnemonic.as_str(), None, 0)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        Ok(WebOwnerMaterial {
            mnemonic_words: mnemonic
                .split_whitespace()
                .map(ToString::to_string)
                .collect(),
            owner_public_jwk: owner.public_jwk,
            owner_derivation_path: OWNER_DERIVATION_PATH.to_string(),
            evm_address: evm.address,
            evm_derivation_path: EVM_DERIVATION_PATH.to_string(),
        })
    }

    async fn prepare_active_documents(
        &self,
        req: PrepareActiveDocumentsReq,
    ) -> Result<PreparedActiveDocuments, RPCErrors> {
        validate_owner_document(&req.owner_document)?;
        req.names.validate(&req.owner_document)?;
        validate_topology(&req.topology)?;
        let device_public_key: Jwk = serde_json::from_value(req.device_public_key)
            .map_err(|error| RPCErrors::ParseRequestError(error.to_string()))?;
        validate_ed25519_jwk(&device_public_key, "device public key")?;

        let now = buckyos_get_unix_timestamp();
        let exp = now + DEFAULT_EXPIRE_TIME;
        let ood_net_id = if req.topology.net_id == "nat" {
            None
        } else {
            Some(req.topology.net_id.clone())
        };
        let ood =
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, ood_net_id, None);
        let sn_relay_host = if req.topology.uses_sn_relay {
            Some(sn_host_from_url(req.topology.sn_url.as_str())?)
        } else {
            None
        };
        let boot_document = ZoneBootDocument {
            id: Some(req.names.zone_did.clone()),
            oods: vec![ood],
            sn: sn_relay_host,
            exp,
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };

        let device_did =
            build_device_did("ood1", &req.names.zone_did).map_err(RPCErrors::ReasonError)?;
        let mut device_document =
            new_device_config_by_jwk_with_did("ood1", device_public_key, &device_did)
                .map_err(RPCErrors::ReasonError)?;
        device_document.owner = req.names.owner_did.clone();
        device_document.zone_did = Some(req.names.zone_did.clone());
        device_document.net_id = Some(req.topology.net_id.clone());
        device_document.rtcp_port = Some(req.topology.rtcp_port);
        device_document.support_container = req.topology.support_container;
        device_document.ddns_sn_url = if req.topology.uses_sn_relay {
            Some(req.topology.sn_url.clone())
        } else {
            None
        };
        device_document.iat = now;
        device_document.exp = exp;
        device_document.version_seq = Some(0);

        let device_mini_document = DeviceMiniDocument::new_by_device_document(&device_document);
        let mut device_info = DeviceInfo::from_device_doc(&device_document);
        device_info
            .auto_fill_by_system_info()
            .await
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        Ok(PreparedActiveDocuments {
            owner_document: req.owner_document,
            names: req.names,
            topology: req.topology,
            boot_document,
            device_document,
            device_mini_document,
            device_info,
        })
    }

    fn assemble_zone_document(
        &self,
        req: AssembleZoneDocumentReq,
    ) -> Result<ZoneDocument, RPCErrors> {
        assemble_zone_document_internal(
            &req.prepared,
            req.boot_document_jwt.as_str(),
            req.device_document_jwt.as_str(),
            req.device_mini_document_jwt.as_str(),
        )
    }

    fn sign_web_active_documents(
        &self,
        req: SignWebActiveDocumentsReq,
    ) -> Result<SignedActiveDocuments, RPCErrors> {
        let mnemonic = req.mnemonic_words.join(" ");
        let owner = derive_bucky_key_from_mnemonic(mnemonic.as_str(), None, 0)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let evm = derive_evm_key_from_mnemonic(mnemonic.as_str(), None, 0)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let owner_key = req
            .prepared
            .owner_document
            .get_default_key()
            .ok_or_else(|| RPCErrors::ReasonError("OwnerDocument key is missing".to_string()))?;
        if serde_json::to_value(owner_key).ok() != Some(owner.public_jwk.clone()) {
            return Err(RPCErrors::ReasonError(
                "mnemonic owner key does not match OwnerDocument".to_string(),
            ));
        }
        let wallet = owner_main_wallet(&req.prepared.owner_document)?;
        if !wallet.address.eq_ignore_ascii_case(evm.address.as_str()) {
            return Err(RPCErrors::ReasonError(
                "mnemonic EVM address does not match OwnerDocument".to_string(),
            ));
        }
        let signing_key = EncodingKey::from_ed_pem(owner.private_key_pem.as_bytes())
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let boot_document_jwt = req
            .prepared
            .boot_document
            .encode(Some(&signing_key))
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
            .to_string();
        let device_document_jwt = req
            .prepared
            .device_document
            .encode(Some(&signing_key))
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
            .to_string();
        let device_mini_document_jwt = req
            .prepared
            .device_mini_document
            .to_jwt(&signing_key)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let zone_document = assemble_zone_document_internal(
            &req.prepared,
            boot_document_jwt.as_str(),
            device_document_jwt.as_str(),
            device_mini_document_jwt.as_str(),
        )?;
        let zone_document_jwt = zone_document
            .encode(Some(&signing_key))
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
            .to_string();
        Ok(SignedActiveDocuments {
            boot_document: req.prepared.boot_document,
            boot_document_jwt,
            device_document: req.prepared.device_document,
            device_document_jwt,
            device_mini_document: req.prepared.device_mini_document,
            device_mini_document_jwt,
            zone_document,
            zone_document_jwt,
        })
    }

    async fn commit_active(&self, req: CommitActiveReq) -> Result<CommitActiveResp, RPCErrors> {
        validate_commit_request(&req, &self.config)?;
        let mut effective_owner = req.owner_document.clone();
        let needs_owner_publish = req.prepared.names.zone_did != effective_owner.id
            && !effective_owner.is_bound_to_zone(&req.prepared.names.zone_did);
        if needs_owner_publish {
            effective_owner.set_default_zone_did(req.prepared.names.zone_did.clone());
            validate_owner_document(&effective_owner)?;
        }

        let bns_client = BnsIndexerClient::new_bns_server_url(req.sn.bns_url.as_str(), None);
        let sn_client =
            SnClient::new_krpc(req.sn.sn_url.as_str(), Some(req.sn.access_token.clone()));

        if req.prepared.names.use_self_domain {
            let domain = req.prepared.names.access_hostname.as_str();
            let result = sn_client.bind_domain(domain).await?;
            if result.domain != domain {
                return Err(RPCErrors::ReasonError(
                    "SN verified a different custom domain".to_string(),
                ));
            }
        }

        if needs_owner_publish {
            let owner_value = serde_json::to_value(&effective_owner)
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
            let owner_bytes = canonical_json_bytes(&owner_value)?;
            let request_id = content_request_id(
                "owner",
                req.prepared.names.owner_name.as_str(),
                owner_bytes.as_slice(),
            );
            if !projection_matches_json(
                &bns_client,
                req.prepared.names.bns_publish_name.as_str(),
                "owner",
                &owner_value,
            )
            .await?
            {
                let document = owner_value.as_object().cloned().ok_or_else(|| {
                    RPCErrors::ReasonError("OwnerDocument must serialize as object".to_string())
                })?;
                sn_client
                    .publish_document(SnBnsPublishDocumentReq {
                        name: req.prepared.names.bns_publish_name.clone(),
                        doc_type: "owner".to_string(),
                        document: SnBnsPublishDocumentContent::JsonObject(document),
                        request_id: Some(request_id),
                    })
                    .await?;
                wait_for_json_projection(
                    &bns_client,
                    req.prepared.names.bns_publish_name.as_str(),
                    "owner",
                    &owner_value,
                )
                .await?;
            }
        }

        let zone_jwt = req.signed_documents.zone_document_jwt.as_str();
        let zone_request_id = content_request_id(
            "zone",
            req.prepared.names.owner_name.as_str(),
            zone_jwt.as_bytes(),
        );
        if !projection_matches_bytes(
            &bns_client,
            req.prepared.names.bns_publish_name.as_str(),
            "zone",
            zone_jwt.as_bytes(),
        )
        .await?
        {
            sn_client
                .publish_document(SnBnsPublishDocumentReq {
                    name: req.prepared.names.bns_publish_name.clone(),
                    doc_type: "zone".to_string(),
                    document: SnBnsPublishDocumentContent::Jwt(zone_jwt.to_string()),
                    request_id: Some(zone_request_id),
                })
                .await?;
            wait_for_bytes_projection(
                &bns_client,
                req.prepared.names.bns_publish_name.as_str(),
                "zone",
                zone_jwt.as_bytes(),
            )
            .await?;
        }

        let device_key_did = device_key_did_from_doc(&req.prepared.device_document)?;
        sn_client
            .register_device_online(build_sn_device_online_report(
                req.prepared.device_document.name.as_str(),
                device_key_did.as_str(),
                &req.prepared.device_info,
            )?)
            .await?;
        let zone_info = sn_client.get_zone_info().await?;
        validate_sn_zone_info(&req, &zone_info)?;

        update_did_cache(
            req.prepared.names.zone_did.clone(),
            Some(DidDocType::Zone),
            EncodedDocument::Jwt(req.signed_documents.zone_document_jwt.clone()),
            Some(UpdateSource::Authority),
        )
        .await
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;

        persist_activation(&req, &effective_owner, &zone_info)?;

        tokio::task::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            exit(0);
        });

        Ok(CommitActiveResp {
            status: "completed".to_string(),
            access_hostname: req.prepared.names.access_hostname,
        })
    }

    async fn handle_get_mini_device_info(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let device_info_json = serde_json::to_string(&self.device_mini_info)
            .map_err(|error| server_err!(ServerErrorCode::InvalidData, "{}", error))?;
        Ok(http::Response::builder()
            .body(BoxBody::new(
                Full::new(Bytes::from(device_info_json))
                    .map_err(|never: std::convert::Infallible| -> ServerError { match never {} })
                    .boxed(),
            ))
            .map_err(|error| server_err!(ServerErrorCode::InvalidData, "{}", error))?)
    }
}

fn validate_hostname(value: &str) -> Result<(), RPCErrors> {
    if value.is_empty()
        || value.len() > 253
        || value.contains('/')
        || value.contains(':')
        || value.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || label.starts_with('-')
                || label.ends_with('-')
                || !label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-')
        })
    {
        return Err(RPCErrors::ReasonError(
            "invalid access hostname".to_string(),
        ));
    }
    Ok(())
}

fn validate_sn_zone_info(
    req: &CommitActiveReq,
    zone_info: &SnZoneInfoResp,
) -> Result<(), RPCErrors> {
    if zone_info.zone != req.owner_document.name {
        return Err(RPCErrors::ReasonError(format!(
            "SN zone mismatch: expected {}, got {}",
            req.owner_document.name, zone_info.zone
        )));
    }
    if let Some(relay_node) = zone_info.relay_sn.as_deref() {
        validate_hostname(relay_node.trim())?;
    }
    Ok(())
}

fn validate_topology(topology: &GatewayTopology) -> Result<(), RPCErrors> {
    if !matches!(
        topology.net_id.as_str(),
        "nat" | "wan_dyn" | "portmap" | "wan"
    ) {
        return Err(RPCErrors::ReasonError("invalid gateway net_id".to_string()));
    }
    if topology.rtcp_port == 0 || topology.rtcp_port > 65535 {
        return Err(RPCErrors::ReasonError("invalid RTCP port".to_string()));
    }
    if topology.sn_url.trim().is_empty() {
        return Err(RPCErrors::ReasonError(
            "SN control-plane URL is required".to_string(),
        ));
    }
    if topology.net_id == "wan" && topology.uses_sn_relay {
        return Err(RPCErrors::ReasonError(
            "WAN topology cannot use SN relay".to_string(),
        ));
    }
    Ok(())
}

fn validate_prepared_relationships(prepared: &PreparedActiveDocuments) -> Result<(), RPCErrors> {
    let names = &prepared.names;
    let topology = &prepared.topology;
    let boot = &prepared.boot_document;
    let device = &prepared.device_document;
    if boot.id.as_ref() != Some(&names.zone_did) {
        return Err(RPCErrors::ReasonError(format!(
            "BootDocument zone id mismatch: expected {:?}, got {:?}",
            names.zone_did, boot.id
        )));
    }
    if boot.owner.is_some() || boot.owner_key.is_some() {
        return Err(RPCErrors::ReasonError(
            "BootDocument must not embed owner or owner_key".to_string(),
        ));
    }
    if boot.exp != device.exp {
        return Err(RPCErrors::ReasonError(format!(
            "BootDocument expiry mismatch: boot exp {}, device exp {}",
            boot.exp, device.exp
        )));
    }
    let expected_ood = OODDescriptionString::new(
        "ood1".to_string(),
        DeviceNodeType::OOD,
        if topology.net_id == "nat" {
            None
        } else {
            Some(topology.net_id.clone())
        },
        None,
    );
    let expected_sn = if topology.uses_sn_relay {
        Some(sn_host_from_url(topology.sn_url.as_str())?)
    } else {
        None
    };
    if boot.oods != vec![expected_ood] || boot.sn != expected_sn {
        return Err(RPCErrors::ReasonError(
            "BootDocument OOD/SN topology mismatch".to_string(),
        ));
    }
    let expected_device_did =
        build_device_did("ood1", &names.zone_did).map_err(RPCErrors::ReasonError)?;
    let expected_ddns_sn_url = topology.uses_sn_relay.then(|| topology.sn_url.clone());
    if device.id != expected_device_did {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument id mismatch: expected {:?}, got {:?}",
            expected_device_did, device.id
        )));
    }
    if device.name != "ood1" {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument name mismatch: expected ood1, got {:?}",
            device.name
        )));
    }
    if device.owner != names.owner_did {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument owner mismatch: expected {:?}, got {:?}",
            names.owner_did, device.owner
        )));
    }
    if device.zone_did.as_ref() != Some(&names.zone_did) {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument zone_did mismatch: expected {:?}, got {:?}",
            names.zone_did, device.zone_did
        )));
    }
    if device.net_id.as_deref() != Some(topology.net_id.as_str()) {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument net_id mismatch: expected {:?}, got {:?}",
            topology.net_id, device.net_id
        )));
    }
    if device.rtcp_port != Some(topology.rtcp_port) {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument RTCP port mismatch: expected {}, got {:?}",
            topology.rtcp_port, device.rtcp_port
        )));
    }
    if device.support_container != topology.support_container {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument container support mismatch: expected {}, got {}",
            topology.support_container, device.support_container
        )));
    }
    if device.ddns_sn_url != expected_ddns_sn_url {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument DDNS SN URL mismatch: expected {:?}, got {:?}",
            expected_ddns_sn_url, device.ddns_sn_url
        )));
    }
    if prepared.device_mini_document != DeviceMiniDocument::new_by_device_document(device) {
        return Err(RPCErrors::ReasonError(
            "DeviceMiniDocument differs from DeviceDocument".to_string(),
        ));
    }
    if prepared.device_info.id != device.id {
        return Err(RPCErrors::ReasonError(
            "DeviceInfo id differs from DeviceDocument".to_string(),
        ));
    }
    if prepared.device_info.name != device.name {
        return Err(RPCErrors::ReasonError(
            "DeviceInfo name differs from DeviceDocument".to_string(),
        ));
    }
    if prepared.device_info.owner != device.owner {
        return Err(RPCErrors::ReasonError(
            "DeviceInfo owner differs from DeviceDocument".to_string(),
        ));
    }
    if prepared.device_info.zone_did != device.zone_did {
        return Err(RPCErrors::ReasonError(
            "DeviceInfo zone_did differs from DeviceDocument".to_string(),
        ));
    }
    if prepared.device_info.get_default_key() != device.get_default_key() {
        return Err(RPCErrors::ReasonError(
            "DeviceInfo default key differs from DeviceDocument".to_string(),
        ));
    }
    Ok(())
}

fn sn_host_from_url(sn_url: &str) -> Result<String, RPCErrors> {
    url::Url::parse(sn_url)
        .ok()
        .and_then(|url| url.host_str().map(ToString::to_string))
        .ok_or_else(|| RPCErrors::ReasonError("invalid SN URL".to_string()))
}

fn normalize_endpoint(value: &str, label: &str) -> Result<url::Url, RPCErrors> {
    let mut endpoint = url::Url::parse(value.trim())
        .map_err(|error| RPCErrors::ReasonError(format!("invalid {}: {}", label, error)))?;
    if endpoint.host_str().is_none()
        || !matches!(endpoint.scheme(), "http" | "https")
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.query().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(RPCErrors::ReasonError(format!(
            "{} must be an HTTP endpoint with a host and no credentials, query, or fragment",
            label
        )));
    }
    let normalized_path = endpoint.path().trim_end_matches('/').to_string();
    endpoint.set_path(if normalized_path.is_empty() {
        "/"
    } else {
        normalized_path.as_str()
    });
    Ok(endpoint)
}

fn validate_commit_endpoints(
    req: &CommitActiveReq,
    config: &ActiveServiceConfig,
) -> Result<(), RPCErrors> {
    let prepared_sn = normalize_endpoint(&req.prepared.topology.sn_url, "topology SN URL")?;
    let commit_sn = normalize_endpoint(&req.sn.sn_url, "commit SN URL")?;
    if prepared_sn != commit_sn {
        return Err(RPCErrors::ReasonError(format!(
            "SN endpoint mismatch: prepared {}, commit {}",
            prepared_sn, commit_sn
        )));
    }
    let commit_bns = normalize_endpoint(&req.sn.bns_url, "commit BNS URL")?;
    if !matches!(config.http_schema.as_str(), "http" | "https") {
        return Err(RPCErrors::ReasonError(format!(
            "invalid active service HTTP schema {:?}",
            config.http_schema
        )));
    }
    let base_host = config.sn_base_host.trim().to_lowercase();
    validate_hostname(base_host.as_str())?;
    let expected_sn = normalize_endpoint(
        format!("{}://sn.{}/kapi/sn", config.http_schema, base_host).as_str(),
        "configured SN URL",
    )?;
    let expected_bns = normalize_endpoint(
        format!("{}://bns.{}/kapi/bns", config.http_schema, base_host).as_str(),
        "configured BNS URL",
    )?;
    if commit_sn != expected_sn {
        return Err(RPCErrors::ReasonError(format!(
            "commit SN endpoint differs from active service config: expected {}, got {}",
            expected_sn, commit_sn
        )));
    }
    if commit_bns != expected_bns {
        return Err(RPCErrors::ReasonError(format!(
            "commit BNS endpoint differs from active service config: expected {}, got {}",
            expected_bns, commit_bns
        )));
    }
    if !req.prepared.names.use_self_domain {
        let expected = format!("{}.web3.{}", req.prepared.names.owner_name, base_host);
        if req.prepared.names.access_hostname != expected {
            return Err(RPCErrors::ReasonError(format!(
                "default access hostname mismatch: expected {:?}, got {:?}",
                expected, req.prepared.names.access_hostname
            )));
        }
    }
    Ok(())
}

fn validate_ed25519_jwk(jwk: &Jwk, label: &str) -> Result<(), RPCErrors> {
    let value =
        serde_json::to_value(jwk).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let valid = value.get("kty").and_then(Value::as_str) == Some("OKP")
        && value.get("crv").and_then(Value::as_str) == Some("Ed25519")
        && value
            .get("x")
            .and_then(Value::as_str)
            .is_some_and(|x| !x.is_empty());
    if !valid {
        return Err(RPCErrors::ReasonError(format!(
            "{} must be an Ed25519 JWK",
            label
        )));
    }
    DecodingKey::from_jwk(jwk)
        .map(|_| ())
        .map_err(|error| RPCErrors::ReasonError(format!("invalid {}: {}", label, error)))
}

fn owner_main_wallet(owner_document: &OwnerDocument) -> Result<&OwnerWallet, RPCErrors> {
    let wallet = owner_document.wallets.get("main").ok_or_else(|| {
        RPCErrors::ReasonError("OwnerDocument wallets.main is missing".to_string())
    })?;
    if wallet.wallet_type != "eth"
        || wallet.address.len() != 42
        || !wallet.address.starts_with("0x")
        || !wallet.address[2..]
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(RPCErrors::ReasonError(
            "OwnerDocument wallets.main must be a valid EVM address".to_string(),
        ));
    }
    Ok(wallet)
}

fn contains_sensitive_owner_field(value: &Value) -> bool {
    match value {
        Value::Object(object) => object.iter().any(|(key, value)| {
            let key = key.to_ascii_lowercase();
            key.contains("mnemonic")
                || key.contains("private_key")
                || key.contains("password")
                || key.contains("pwd_hash")
                || key.contains("active_code")
                || key.contains("token")
                || key == "email"
                || contains_sensitive_owner_field(value)
        }),
        Value::Array(values) => values.iter().any(contains_sensitive_owner_field),
        _ => false,
    }
}

fn validate_owner_document(owner_document: &OwnerDocument) -> Result<(), RPCErrors> {
    let normalized_name = owner_document.name.trim().to_lowercase();
    if normalized_name.is_empty()
        || owner_document.name != normalized_name
        || owner_document.id != DID::new("bns", normalized_name.as_str())
    {
        return Err(RPCErrors::ReasonError(
            "OwnerDocument id/name mismatch".to_string(),
        ));
    }
    let owner_key = owner_document.get_default_key().ok_or_else(|| {
        RPCErrors::ReasonError("OwnerDocument default key is missing".to_string())
    })?;
    validate_ed25519_jwk(&owner_key, "OwnerDocument default key")?;
    owner_main_wallet(owner_document)?;
    let value = serde_json::to_value(owner_document)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    if contains_sensitive_owner_field(&value) {
        return Err(RPCErrors::ReasonError(
            "OwnerDocument contains sensitive fields".to_string(),
        ));
    }
    let round_trip: OwnerDocument = serde_json::from_value(value.clone())
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    if &round_trip != owner_document {
        return Err(RPCErrors::ReasonError(
            "OwnerDocument round-trip mismatch".to_string(),
        ));
    }
    if serde_json::to_vec(&value)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
        .len()
        >= MAX_INLINE_DOCUMENT
    {
        return Err(RPCErrors::ReasonError(
            "OwnerDocument exceeds 4KB".to_string(),
        ));
    }
    Ok(())
}

fn owner_decoding_key(owner_document: &OwnerDocument) -> Result<DecodingKey, RPCErrors> {
    let jwk = owner_document.get_default_key().ok_or_else(|| {
        RPCErrors::ReasonError("OwnerDocument default key is missing".to_string())
    })?;
    DecodingKey::from_jwk(&jwk)
        .map_err(|error| RPCErrors::ReasonError(format!("invalid owner key: {}", error)))
}

fn assemble_zone_document_internal(
    prepared: &PreparedActiveDocuments,
    boot_document_jwt: &str,
    device_document_jwt: &str,
    device_mini_document_jwt: &str,
) -> Result<ZoneDocument, RPCErrors> {
    validate_owner_document(&prepared.owner_document)?;
    prepared.names.validate(&prepared.owner_document)?;
    let owner_key = owner_decoding_key(&prepared.owner_document)?;
    let boot_document = ZoneBootDocument::decode(
        &EncodedDocument::Jwt(boot_document_jwt.to_string()),
        Some(&owner_key),
    )
    .map_err(|error| RPCErrors::ReasonError(format!("invalid Boot JWT: {}", error)))?;
    if boot_document != prepared.boot_document {
        return Err(RPCErrors::ReasonError(
            "Boot JWT payload differs from prepared document".to_string(),
        ));
    }
    let device_document = DeviceDocument::decode(
        &EncodedDocument::Jwt(device_document_jwt.to_string()),
        Some(&owner_key),
    )
    .map_err(|error| RPCErrors::ReasonError(format!("invalid Device JWT: {}", error)))?;
    if device_document != prepared.device_document {
        return Err(RPCErrors::ReasonError(
            "Device JWT payload differs from prepared document".to_string(),
        ));
    }
    let device_mini_document = DeviceMiniDocument::from_jwt(device_mini_document_jwt, &owner_key)
        .map_err(|error| {
        RPCErrors::ReasonError(format!("invalid DeviceMini JWT: {}", error))
    })?;
    if device_mini_document != prepared.device_mini_document {
        return Err(RPCErrors::ReasonError(
            "DeviceMini JWT payload differs from prepared document".to_string(),
        ));
    }

    let owner_jwk = prepared.owner_document.get_default_key().ok_or_else(|| {
        RPCErrors::ReasonError("OwnerDocument default key is missing".to_string())
    })?;
    let mut zone_document = ZoneDocument::new(
        prepared.names.zone_did.clone(),
        prepared.names.owner_did.clone(),
        owner_jwk,
    );
    zone_document.init_by_boot_document(&prepared.boot_document, &boot_document_jwt.to_string());
    zone_document.hostname = prepared.names.access_hostname.clone();
    let mut embedded_device = prepared.device_document.clone();
    embedded_device.device_mini_document_jwt = Some(device_mini_document_jwt.to_string());
    zone_document
        .devices
        .insert(embedded_device.name.clone(), embedded_device);
    zone_document.mini_device_jwts.insert(
        prepared.device_document.name.clone(),
        device_mini_document_jwt.to_string(),
    );
    zone_document.version_seq = Some(0);
    Ok(zone_document)
}

fn verify_signed_documents(
    prepared: &PreparedActiveDocuments,
    signed: &SignedActiveDocuments,
) -> Result<(), RPCErrors> {
    if signed.boot_document != prepared.boot_document
        || signed.device_document != prepared.device_document
        || signed.device_mini_document != prepared.device_mini_document
    {
        return Err(RPCErrors::ReasonError(
            "signed document payload differs from prepared document".to_string(),
        ));
    }
    let expected_zone = assemble_zone_document_internal(
        prepared,
        signed.boot_document_jwt.as_str(),
        signed.device_document_jwt.as_str(),
        signed.device_mini_document_jwt.as_str(),
    )?;
    if signed.zone_document != expected_zone {
        return Err(RPCErrors::ReasonError(
            "ZoneDocument nested documents are inconsistent".to_string(),
        ));
    }
    let owner_key = owner_decoding_key(&prepared.owner_document)?;
    let decoded_zone = ZoneDocument::decode(
        &EncodedDocument::Jwt(signed.zone_document_jwt.clone()),
        Some(&owner_key),
    )
    .map_err(|error| RPCErrors::ReasonError(format!("invalid Zone JWT: {}", error)))?;
    if decoded_zone != expected_zone {
        return Err(RPCErrors::ReasonError(
            "Zone JWT payload differs from assembled ZoneDocument".to_string(),
        ));
    }
    if signed.zone_document.boot_jwt != signed.boot_document_jwt
        || signed
            .zone_document
            .mini_device_jwts
            .get(prepared.device_document.name.as_str())
            != Some(&signed.device_mini_document_jwt)
    {
        return Err(RPCErrors::ReasonError(
            "ZoneDocument nested JWT mismatch".to_string(),
        ));
    }
    Ok(())
}

fn validate_document_times(prepared: &PreparedActiveDocuments) -> Result<(), RPCErrors> {
    let now = buckyos_get_unix_timestamp();
    if prepared.boot_document.exp <= now {
        return Err(RPCErrors::ReasonError(format!(
            "BootDocument expired: exp {}, now {}",
            prepared.boot_document.exp, now
        )));
    }
    if prepared.device_document.exp <= now {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument expired: exp {}, now {}",
            prepared.device_document.exp, now
        )));
    }
    if prepared.device_mini_document.exp <= now {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceMiniDocument expired: exp {}, now {}",
            prepared.device_mini_document.exp, now
        )));
    }
    if prepared.device_document.iat > now + 300 {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument issued too far in the future: iat {}, now {}",
            prepared.device_document.iat, now
        )));
    }
    if prepared.device_document.version_seq.is_none() {
        return Err(RPCErrors::ReasonError(
            "DeviceDocument version_seq is missing".to_string(),
        ));
    }
    Ok(())
}

fn validate_device_private_key(
    private_key_pem: &str,
    device_document: &DeviceDocument,
) -> Result<(), RPCErrors> {
    let encoding_key = EncodingKey::from_ed_pem(private_key_pem.as_bytes()).map_err(|error| {
        RPCErrors::ReasonError(format!("invalid device private key: {}", error))
    })?;
    let claims = json!({
        "iat": buckyos_get_unix_timestamp(),
        "exp": buckyos_get_unix_timestamp() + 60
    });
    let mut header = Header::new(Algorithm::EdDSA);
    header.typ = None;
    let jwt = encode(&header, &claims, &encoding_key)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let public_key = device_document.get_default_key().ok_or_else(|| {
        RPCErrors::ReasonError("DeviceDocument default key is missing".to_string())
    })?;
    decode_json_from_jwt_with_default_pk(jwt.as_str(), &public_key).map_err(|_| {
        RPCErrors::ReasonError(
            "device private key does not match DeviceDocument public key".to_string(),
        )
    })?;
    Ok(())
}

fn validate_commit_request(
    req: &CommitActiveReq,
    config: &ActiveServiceConfig,
) -> Result<(), RPCErrors> {
    validate_owner_document(&req.owner_document)?;
    if req.owner_document != req.prepared.owner_document {
        return Err(RPCErrors::ReasonError(
            "commit OwnerDocument differs from prepared OwnerDocument".to_string(),
        ));
    }
    req.prepared.names.validate(&req.owner_document)?;
    validate_topology(&req.prepared.topology)?;
    validate_commit_endpoints(req, config)?;
    validate_prepared_relationships(&req.prepared)?;
    verify_signed_documents(&req.prepared, &req.signed_documents)?;
    validate_document_times(&req.prepared)?;
    validate_device_private_key(
        req.device_private_key.as_str(),
        &req.prepared.device_document,
    )?;
    if req.signed_documents.zone_document_jwt.len() >= MAX_INLINE_DOCUMENT {
        return Err(RPCErrors::ReasonError(
            "ZoneDocument JWT exceeds 4KB".to_string(),
        ));
    }
    if req.sn.access_token.trim().is_empty()
        || req.sn.sn_url.trim().is_empty()
        || req.sn.bns_url.trim().is_empty()
    {
        return Err(RPCErrors::ReasonError(
            "SN access token and endpoints are required".to_string(),
        ));
    }
    Ok(())
}

fn device_info_report_ip(device_info: &DeviceInfo) -> String {
    device_info
        .all_ip
        .first()
        .or_else(|| device_info.ips.first())
        .map(ToString::to_string)
        .unwrap_or_else(|| "127.0.0.1".to_string())
}

fn device_key_did_from_doc(device_doc: &DeviceDocument) -> Result<String, RPCErrors> {
    let default_key = device_doc.get_default_key().ok_or_else(|| {
        RPCErrors::ReasonError("DeviceDocument default key is missing".to_string())
    })?;
    let x =
        get_x_from_jwk(&default_key).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    Ok(format!("did:dev:{}", x))
}

fn build_sn_device_online_report(
    device_name: &str,
    device_key_did: &str,
    device_info: &DeviceInfo,
) -> Result<SnDeviceOnlineReportReq, RPCErrors> {
    Ok(SnDeviceOnlineReportReq {
        device_name: device_name.to_string(),
        device_did: Some(device_key_did.to_string()),
        device_ip: device_info_report_ip(device_info),
        device_info: serde_json::to_value(device_info)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?,
        endpoints: Vec::new(),
        report_seq: None,
        ttl: None,
    })
}

fn content_request_id(operation: &str, owner_name: &str, bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!(
        "node-active:{}:{}:{}",
        operation,
        owner_name,
        hex::encode(digest)
    )
}

fn canonical_json_value(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut keys: Vec<&String> = object.keys().collect();
            keys.sort();
            let mut canonical = Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_json_value(&object[key]));
            }
            Value::Object(canonical)
        }
        Value::Array(values) => Value::Array(values.iter().map(canonical_json_value).collect()),
        _ => value.clone(),
    }
}

fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, RPCErrors> {
    serde_json::to_vec(&canonical_json_value(value))
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))
}

async fn projection_matches_bytes(
    client: &dyn BnsIndexerApi,
    name: &str,
    doc_type: &str,
    expected: &[u8],
) -> Result<bool, RPCErrors> {
    match client.resolve_document(name, doc_type).await {
        Ok(result) => Ok(result.document_state.document.inline_document == expected),
        Err(error)
            if error.is_registry_code("DOCUMENT_NOT_FOUND")
                || error.is_registry_code("NAME_NOT_FOUND") =>
        {
            Ok(false)
        }
        Err(error) => Err(RPCErrors::ReasonError(format!(
            "BNS projection read failed: {}",
            error
        ))),
    }
}

async fn projection_matches_json(
    client: &dyn BnsIndexerApi,
    name: &str,
    doc_type: &str,
    expected: &Value,
) -> Result<bool, RPCErrors> {
    match client.resolve_document(name, doc_type).await {
        Ok(result) => serde_json::from_slice::<Value>(
            result.document_state.document.inline_document.as_slice(),
        )
        .map(|value| value == *expected)
        .map_err(|error| RPCErrors::ReasonError(format!("invalid projected JSON: {}", error))),
        Err(error)
            if error.is_registry_code("DOCUMENT_NOT_FOUND")
                || error.is_registry_code("NAME_NOT_FOUND") =>
        {
            Ok(false)
        }
        Err(error) => Err(RPCErrors::ReasonError(format!(
            "BNS projection read failed: {}",
            error
        ))),
    }
}

async fn wait_for_bytes_projection(
    client: &dyn BnsIndexerApi,
    name: &str,
    doc_type: &str,
    expected: &[u8],
) -> Result<(), RPCErrors> {
    let started = Instant::now();
    let mut delay = Duration::from_millis(200);
    while started.elapsed() < PROJECTION_DEADLINE {
        if projection_matches_bytes(client, name, doc_type, expected).await? {
            return Ok(());
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
    Err(RPCErrors::ReasonError(format!(
        "projection_timeout waiting for {}/{}",
        name, doc_type
    )))
}

async fn wait_for_json_projection(
    client: &dyn BnsIndexerApi,
    name: &str,
    doc_type: &str,
    expected: &Value,
) -> Result<(), RPCErrors> {
    let started = Instant::now();
    let mut delay = Duration::from_millis(200);
    while started.elapsed() < PROJECTION_DEADLINE {
        if projection_matches_json(client, name, doc_type, expected).await? {
            return Ok(());
        }
        tokio::time::sleep(delay).await;
        delay = (delay * 2).min(Duration::from_secs(2));
    }
    Err(RPCErrors::ReasonError(format!(
        "projection_timeout waiting for {}/{}",
        name, doc_type
    )))
}

fn build_start_config(req: &CommitActiveReq, owner_document: &OwnerDocument) -> Value {
    json!({
        "user_name": owner_document.name,
        "owner_document": owner_document,
        "zone_name": req.prepared.names.zone_did.to_string(),
        "access_hostname": req.prepared.names.access_hostname,
        "zone_document_jwt": req.signed_documents.zone_document_jwt,
        "boot_config_jwt": req.signed_documents.boot_document_jwt,
        "device_doc_jwt": req.signed_documents.device_document_jwt,
        "device_mini_doc_jwt": req.signed_documents.device_mini_document_jwt,
        "ood_jwt": req.signed_documents.device_document_jwt,
        "admin_password_hash": req.system_settings.admin_password_hash,
        "guest_access": req.system_settings.guest_access,
        "friend_passcode": req.system_settings.friend_passcode,
        "enabled_features": req.system_settings.enabled_features,
        "ai_provider_config": req.system_settings.ai_provider_config,
        "jarvis_msg_tunnel_config": req.system_settings.jarvis_msg_tunnel_config
    })
}

fn persist_activation(
    req: &CommitActiveReq,
    effective_owner: &OwnerDocument,
    zone_info: &SnZoneInfoResp,
) -> Result<(), RPCErrors> {
    let etc_dir = get_buckyos_system_etc_dir();
    let start_config = serde_json::to_vec_pretty(&build_start_config(req, effective_owner))
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    atomic_write(etc_dir.join("start_config.json").as_path(), &start_config)
        .map_err(RPCErrors::ReasonError)?;
    save_sn_zone_info(zone_info).map_err(RPCErrors::ReasonError)?;
    save_zone_document_jwt(
        etc_dir.as_path(),
        req.signed_documents.zone_document_jwt.as_str(),
    )
    .map_err(RPCErrors::ReasonError)?;

    let device = &req.prepared.device_document;
    let node_identity = LocalNodeIdentityConfig::new(
        req.prepared.names.zone_did.clone(),
        req.prepared.names.owner_did.clone(),
        effective_owner.get_default_key().ok_or_else(|| {
            RPCErrors::ReasonError("OwnerDocument default key is missing".to_string())
        })?,
        device.name.clone(),
        device.id.clone(),
        req.signed_documents.zone_document.iat as u32,
    );
    save_local_device_identity(
        etc_dir.as_path(),
        &node_identity,
        device,
        req.signed_documents.device_document_jwt.as_str(),
        req.signed_documents.device_mini_document_jwt.as_str(),
        req.device_private_key.as_str(),
    )
    .map_err(RPCErrors::ReasonError)?;
    Ok(())
}

fn rpc_success<T: Serialize>(value: T, seq: u64) -> Result<RPCResponse, RPCErrors> {
    let value =
        serde_json::to_value(value).map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    Ok(RPCResponse::new(RPCResult::Success(value), seq))
}

#[async_trait]
impl RPCHandler for ActiveServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        _ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        match req.method.as_str() {
            "generate_web_owner_material" => {
                GenerateWebOwnerMaterialReq::from_json(req.params)?;
                rpc_success(self.generate_web_owner_material()?, seq)
            }
            "generate_device_key_pair" => {
                GenerateDeviceKeyPairReq::from_json(req.params)?;
                let (private_key, public_key) = generate_ed25519_key_pair();
                rpc_success(
                    DeviceKeyPair {
                        public_key,
                        private_key,
                    },
                    seq,
                )
            }
            "prepare_active_documents" => {
                let params = PrepareActiveDocumentsReq::from_json(req.params)?;
                rpc_success(self.prepare_active_documents(params).await?, seq)
            }
            "assemble_zone_document" => {
                let params = AssembleZoneDocumentReq::from_json(req.params)?;
                rpc_success(self.assemble_zone_document(params)?, seq)
            }
            "sign_web_active_documents" => {
                let params = SignWebActiveDocumentsReq::from_json(req.params)?;
                rpc_success(self.sign_web_active_documents(params)?, seq)
            }
            "commit_active" => {
                let params = CommitActiveReq::from_json(req.params)?;
                rpc_success(self.commit_active(params).await?, seq)
            }
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        }
    }
}

#[async_trait]
impl HttpServer for ActiveServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        if *req.method() == Method::GET && req.uri().path() == "/device" {
            return self.handle_get_mini_device_info(req).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        "active-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn start_node_active_service() {
    let active_server_dir = get_buckyos_system_bin_dir().join("node-active");
    let config_path = active_server_dir.join("active_config.json");
    let config = match std::fs::read(&config_path)
        .map_err(|error| error.to_string())
        .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
    {
        Ok(config) => config,
        Err(error) => {
            error!(
                "Failed to load active service config from {:?}: {}",
                config_path, error
            );
            return;
        }
    };
    let runner = Runner::new(ACTIVE_SERVICE_MAIN_PORT);
    let mut active_server = ActiveServer::new(config);
    active_server.auto_fill_device_mini_info().await;
    let active_server = Arc::new(active_server);
    if let Err(error) = runner.add_http_server("/kapi/active".to_string(), active_server.clone()) {
        error!("Failed to add active server: {}", error);
        return;
    }
    if let Err(error) = runner.add_http_server("/device".to_string(), active_server) {
        error!("Failed to add device endpoint: {}", error);
        return;
    }
    if let Err(error) = runner
        .add_dir_handler("/".to_string(), active_server_dir)
        .await
    {
        error!("Failed to add active UI: {}", error);
        return;
    }
    runner.run().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner_document(mnemonic: &str) -> OwnerDocument {
        let owner_key = derive_bucky_key_from_mnemonic(mnemonic, None, 0).unwrap();
        let evm_key = derive_evm_key_from_mnemonic(mnemonic, None, 0).unwrap();
        let mut owner = OwnerDocument::new(
            DID::new("bns", "alice"),
            "alice".to_string(),
            "alice".to_string(),
            serde_json::from_value(owner_key.public_jwk).unwrap(),
        );
        owner.wallets.insert(
            "main".to_string(),
            OwnerWallet {
                wallet_type: "eth".to_string(),
                address: evm_key.address,
            },
        );
        owner
    }

    fn topology() -> GatewayTopology {
        GatewayTopology {
            net_id: "nat".to_string(),
            rtcp_port: DEFAULT_RTCP_PORT,
            support_container: true,
            uses_sn_relay: true,
            sn_url: "https://sn.example.com/kapi/sn".to_string(),
        }
    }

    fn active_service_config() -> ActiveServiceConfig {
        ActiveServiceConfig {
            sn_base_host: "example.com".to_string(),
            http_schema: "https".to_string(),
        }
    }

    #[test]
    fn every_request_struct_has_strict_parsing() {
        assert!(GenerateWebOwnerMaterialReq::from_json(json!({})).is_ok());
        assert!(GenerateDeviceKeyPairReq::from_json(json!({})).is_ok());
        assert!(GenerateWebOwnerMaterialReq::from_json(json!({"extra": true})).is_err());
        assert!(PrepareActiveDocumentsReq::from_json(json!({})).is_err());
        assert!(AssembleZoneDocumentReq::from_json(json!({})).is_err());
        assert!(SignWebActiveDocumentsReq::from_json(json!({})).is_err());
        assert!(CommitActiveReq::from_json(json!({})).is_err());
    }

    #[test]
    fn active_name_mapping_has_unambiguous_default_and_custom_domain_values() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let default = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        assert_eq!(default.owner_name, "alice");
        assert_eq!(default.owner_did.to_string(), "did:bns:alice");
        assert_eq!(default.zone_did.to_string(), "did:bns:alice");
        assert_eq!(default.bns_publish_name, "alice");

        let custom = ActiveNameMapping::derive(&owner, "home.example.com", true);
        assert_eq!(custom.owner_did.to_string(), "did:bns:alice");
        assert_eq!(custom.zone_did.to_string(), "did:web:home.example.com");
        assert_eq!(custom.bns_publish_name, "alice");
    }

    #[tokio::test]
    async fn web_signing_round_trip_builds_four_verified_documents() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = ActiveServer::new(active_service_config());
        let prepared = server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: owner,
                names,
                topology: topology(),
                device_public_key,
            })
            .await
            .unwrap();
        let boot_document = serde_json::to_value(&prepared.boot_document).unwrap();
        assert!(boot_document.get("owner").is_none());
        assert!(boot_document.get("owner_key").is_none());
        let signed = server
            .sign_web_active_documents(SignWebActiveDocumentsReq {
                mnemonic_words: mnemonic
                    .split_whitespace()
                    .map(ToString::to_string)
                    .collect(),
                prepared: prepared.clone(),
            })
            .unwrap();
        verify_signed_documents(&prepared, &signed).unwrap();
        assert_eq!(signed.zone_document.boot_jwt, signed.boot_document_jwt);
        assert!(signed.zone_document_jwt.len() < MAX_INLINE_DOCUMENT);

        let mut commit_req = CommitActiveReq {
            owner_document: prepared.owner_document.clone(),
            prepared: prepared.clone(),
            signed_documents: signed,
            device_private_key: "not-persisted-in-start-config".to_string(),
            system_settings: LocalSystemSettings {
                admin_password_hash: "required-admin-hash".to_string(),
                guest_access: false,
                friend_passcode: String::new(),
                enabled_features: json!({}),
                ai_provider_config: json!({}),
                jarvis_msg_tunnel_config: json!({}),
            },
            sn: SnCommitConfig {
                sn_url: "https://sn.example.com/kapi/sn/".to_string(),
                bns_url: "https://bns.example.com/kapi/bns".to_string(),
                access_token: "secret-access-token".to_string(),
            },
        };
        validate_commit_endpoints(&commit_req, &active_service_config()).unwrap();
        commit_req.sn.sn_url = "https://sn.other.example/kapi/sn".to_string();
        assert!(
            validate_commit_endpoints(&commit_req, &active_service_config())
                .unwrap_err()
                .to_string()
                .contains("SN endpoint mismatch")
        );
        commit_req.sn.sn_url = "https://sn.example.com/kapi/sn".to_string();
        commit_req.prepared.names.access_hostname = "other.example.com".to_string();
        assert!(
            validate_commit_endpoints(&commit_req, &active_service_config())
                .unwrap_err()
                .to_string()
                .contains("default access hostname mismatch")
        );
        commit_req.prepared.names.access_hostname = "alice.web3.example.com".to_string();
        let config = build_start_config(&commit_req, &prepared.owner_document);
        let config_text = serde_json::to_string(&config).unwrap();
        for forbidden in [
            "mnemonic_words",
            "device_private_key",
            "secret-access-token",
            "pwd_hash",
        ] {
            assert!(!config_text.contains(forbidden));
        }

        let mut zone_info = SnZoneInfoResp {
            code: 0,
            zone: "alice".to_string(),
            bns_name: "alice".to_string(),
            relay_sn: Some("relay.example.com".to_string()),
            self_cert: false,
            cert_checked_at: None,
            cert_expires_at: None,
            source_version: Some("v2".to_string()),
            updated_at: 1,
        };
        validate_sn_zone_info(&commit_req, &zone_info).unwrap();
        zone_info.relay_sn = None;
        validate_sn_zone_info(&commit_req, &zone_info).unwrap();
        zone_info.relay_sn = Some("relay.example.com".to_string());
        zone_info.zone = "bob".to_string();
        assert!(validate_sn_zone_info(&commit_req, &zone_info)
            .unwrap_err()
            .to_string()
            .contains("SN zone mismatch"));
    }

    #[tokio::test]
    async fn prepared_gateway_topology_cannot_change_after_signing() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = ActiveServer::new(active_service_config());
        let mut prepared = server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: owner,
                names,
                topology: topology(),
                device_public_key,
            })
            .await
            .unwrap();
        validate_prepared_relationships(&prepared).unwrap();
        prepared.topology.net_id = "wan".to_string();
        prepared.topology.uses_sn_relay = false;
        assert!(validate_prepared_relationships(&prepared).is_err());
    }

    #[tokio::test]
    async fn tampered_nested_jwt_is_rejected() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = ActiveServer::new(active_service_config());
        let prepared = server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: owner,
                names,
                topology: topology(),
                device_public_key,
            })
            .await
            .unwrap();
        let mut signed = server
            .sign_web_active_documents(SignWebActiveDocumentsReq {
                mnemonic_words: mnemonic
                    .split_whitespace()
                    .map(ToString::to_string)
                    .collect(),
                prepared: prepared.clone(),
            })
            .unwrap();
        signed.device_mini_document_jwt.push('x');
        assert!(verify_signed_documents(&prepared, &signed).is_err());
    }

    #[test]
    fn owner_document_rejects_sensitive_fields_and_wrong_evm_wallet() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mut owner = owner_document(mnemonic);
        owner
            .extra_info
            .insert("email".to_string(), json!("alice@example.com"));
        assert!(validate_owner_document(&owner).is_err());
        owner.extra_info.clear();
        owner.wallets.get_mut("main").unwrap().address = "0x1234".to_string();
        assert!(validate_owner_document(&owner).is_err());
    }
}
