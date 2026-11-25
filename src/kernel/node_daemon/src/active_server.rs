use ::kRPC::*;
use async_trait::async_trait;
use bns_client::{BnsIndexerApi, BnsIndexerClient};
use buckyos_api::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler_with_limit, server_err, HttpServer, Runner, ServerError,
    ServerErrorCode, ServerResult, StreamInfo,
};
use buckyos_kit::*;
use bytes::Bytes;
use cyfs_dns_provider::SystemDnsLookup;
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
use name_client::{
    select_authority_current, IdentityRoots, NameClient, ResolveOpts, Revision, SourceRef,
    GLOBAL_NAME_CLIENT,
};
use name_lib::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::fmt::{Debug, Formatter};
use std::net::IpAddr;
use std::path::PathBuf;
use std::process::exit;
use std::result::Result;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::domain::check::{CheckReport, DomainBinding};
use crate::domain::plan::{validate_public_ips, DomainSetup, NetId, SnNames};
use crate::domain::remote::{
    read_owner_document, AccountBinding, BnsOwnerKeys, SnChallenge,
};
use crate::domain::service::{
    credential_of, install_domain, remove_credential, remove_other_credentials, run_challenge,
    ApplyCredential, ApplyResponse, CertificateTool, CommitRequest, DnsServices, DomainCommit,
    DomainService, IdentityCertificates, InspectCertificateResponse, InspectDomainResponse,
    PlanResponse, Trial,
};
use crate::domain::DomainError;
use crate::region_probe::RegionProbeController;
use crate::sn_zone_info::save_sn_zone_info_in;
use crate::zone_boot_resolve::replace_last_known_zone_state_in;

const ACTIVE_SERVICE_MAIN_PORT: u16 = 3182;
const DEFAULT_RTCP_PORT: u32 = 2980;
const MAX_INLINE_DOCUMENT: usize = 4096;
const OWNER_DERIVATION_PATH: &str = "m/9777'/0'/0'";
const EVM_DERIVATION_PATH: &str = "m/44'/60'/0'/0/0";
const PROJECTION_DEADLINE: Duration = Duration::from_secs(60);
pub(crate) const START_CONFIG_FILE: &str = "start_config.json";
const START_CONFIG_OWNER_MNEMONIC: &str = "owner_mnemonic_words";
/// Certificates travel as base64 in the request.
const MAX_REQUEST_BODY: usize = 2 * 1024 * 1024;
const CHECK_TRIAL_WAIT: Duration = Duration::from_secs(8);
/// A browser gives up on a request after 5 minutes (Firefox): the trials of one
/// commit stay well below.
const COMMIT_TRIAL_WAIT: Duration = Duration::from_secs(200);
const SN_TRIAL_STEP_WAIT: Duration = Duration::from_secs(100);

fn parse_params<T: DeserializeOwned>(value: Value, type_name: &str) -> Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParseRequestError(format!("Failed to parse {}: {}", type_name, error))
    })
}

/// A type error quotes the value it found; here that may be a secret.
pub(crate) fn parse_secret_params<T: DeserializeOwned>(
    value: Value,
    type_name: &str,
) -> Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        let text = error.to_string();
        let names_fields = ["missing field", "unknown field", "duplicate field"]
            .iter()
            .any(|prefix| text.starts_with(prefix));
        RPCErrors::ParseRequestError(format!(
            "Failed to parse {}: {}",
            type_name,
            if names_fields { text.as_str() } else { "invalid value" }
        ))
    })
}

macro_rules! impl_from_secret_json {
    ($type:ty) => {
        impl $type {
            pub fn from_json(value: Value) -> Result<Self, RPCErrors> {
                parse_secret_params(value, stringify!($type))
            }
        }
    };
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
    /// `wan` only: the fixed addresses the owner signs.
    pub public_ips: Vec<IpAddr>,
    pub sn_url: String,
}

fn default_rtcp_port() -> u32 {
    DEFAULT_RTCP_PORT
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
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
    /// ACME directory of custom domains instead of Let's Encrypt production.
    #[serde(default)]
    acme_issuer: Option<String>,
}

impl ActiveServiceConfig {
    fn sn_url(&self) -> String {
        format!("{}://sn.{}/kapi/sn", self.http_schema, self.sn_base_host)
    }

    fn bns_url(&self) -> String {
        format!("{}://bns.{}/kapi/bns", self.http_schema, self.sn_base_host)
    }
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
#[serde(deny_unknown_fields)]
pub struct StartRegionProbeReq {
    #[serde(default)]
    pub force: bool,
}
impl_from_json!(StartRegionProbeReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetRegionProbeStatusReq {}
impl_from_json!(GetRegionProbeStatusReq);

#[derive(Clone, Serialize, Deserialize)]
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

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignWebActiveDocumentsReq {
    pub mnemonic_words: Vec<String>,
    pub prepared: PreparedActiveDocuments,
}
impl_from_secret_json!(SignWebActiveDocumentsReq);

#[derive(Clone, Serialize, Deserialize)]
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
    pub host_account: String,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnCommitConfig {
    pub sn_url: String,
    pub bns_url: String,
    pub access_token: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DomainCommitConfig {
    pub setup: DomainSetup,
    #[serde(default)]
    pub credential_id: Option<String>,
    #[serde(default)]
    pub certificate_id: Option<String>,
    pub accept_warnings: bool,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitActiveReq {
    pub owner_document: OwnerDocument,
    pub prepared: PreparedActiveDocuments,
    pub signed_documents: SignedActiveDocuments,
    pub device_private_key: String,
    pub system_settings: LocalSystemSettings,
    pub sn: SnCommitConfig,
    /// Exactly when `names.use_self_domain`.
    pub domain: Option<DomainCommitConfig>,
    #[serde(default)]
    pub owner_mnemonic_words: Option<Vec<String>>,
}
impl_from_secret_json!(CommitActiveReq);

macro_rules! impl_redacted_debug {
    ($type:ident { $($field:ident),* }) => {
        impl Debug for $type {
            fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($type))
                    $(.field(stringify!($field), &self.$field))*
                    .finish_non_exhaustive()
            }
        }
    };
}

impl_redacted_debug!(WebOwnerMaterial { owner_public_jwk, evm_address });
impl_redacted_debug!(DeviceKeyPair { public_key });
impl_redacted_debug!(SignWebActiveDocumentsReq { prepared });
impl_redacted_debug!(LocalSystemSettings { guest_access, enabled_features, host_account });
impl_redacted_debug!(SnCommitConfig { sn_url, bns_url });
impl_redacted_debug!(CommitActiveReq { prepared, system_settings, sn, domain });
impl_redacted_debug!(ApplyDomainRecordsReq { setup, provider, credential_id, confirm });

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeriveWebOwnerMaterialReq {
    pub mnemonic_words: Vec<String>,
}
impl_from_secret_json!(DeriveWebOwnerMaterialReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoadOwnerDocumentReq {
    pub owner_name: String,
}
impl_from_json!(LoadOwnerDocumentReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListPublicAddressesReq {}
impl_from_json!(ListPublicAddressesReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListDnsProvidersReq {}
impl_from_json!(ListDnsProvidersReq);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectDomainReq {
    pub domain: String,
    pub net_id: NetId,
    pub owner_name: String,
    #[serde(default)]
    pub access_token: Option<String>,
}
impl_from_secret_json!(InspectDomainReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanDomainRecordsReq {
    pub setup: DomainSetup,
}
impl_from_json!(PlanDomainRecordsReq);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyDomainRecordsReq {
    pub setup: DomainSetup,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub fields: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub endpoint: Option<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
    pub confirm: Vec<String>,
}
impl_from_secret_json!(ApplyDomainRecordsReq);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDomainRecordsReq {
    pub setup: DomainSetup,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub credential_id: Option<String>,
    #[serde(default)]
    pub certificate_id: Option<String>,
}
impl_from_secret_json!(CheckDomainRecordsReq);

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectCertificateReq {
    pub domain: String,
    pub files: Vec<String>,
    #[serde(default)]
    pub pfx_password: Option<String>,
}
impl_from_secret_json!(InspectCertificateReq);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommitActiveResp {
    pub status: String,
    pub access_hostname: String,
}

#[derive(Clone)]
struct ActiveServer {
    device_mini_info: DeviceMiniInfo,
    config: ActiveServiceConfig,
    region_probe: RegionProbeController,
    domain: Arc<DomainService>,
    target: ActivationTarget,
}

/// The services a commit talks to and what it leaves behind.
#[derive(Clone)]
struct ActivationTarget {
    sn_url: String,
    bns_url: String,
    paths: ActivationPaths,
    /// `None`: the process's own.
    name_client: Option<Arc<NameClient>>,
    exit_when_done: bool,
}

impl ActivationTarget {
    fn local(config: &ActiveServiceConfig, roots: IdentityRoots) -> Self {
        Self {
            sn_url: config.sn_url(),
            bns_url: config.bns_url(),
            paths: ActivationPaths {
                etc_dir: get_buckyos_system_etc_dir(),
                roots,
            },
            name_client: None,
            exit_when_done: true,
        }
    }

    fn name_client(&self) -> Option<&NameClient> {
        self.name_client
            .as_deref()
            .or_else(|| GLOBAL_NAME_CLIENT.get())
    }
}

fn active_error(error: DomainError) -> RPCErrors {
    error.into_rpc_error("active")
}

fn apply_credential(req: &ApplyDomainRecordsReq) -> Result<ApplyCredential, RPCErrors> {
    match (&req.provider, &req.fields, &req.credential_id) {
        (Some(provider), Some(fields), None) => Ok(ApplyCredential::New(credential_of(
            provider.clone(),
            fields.clone(),
            req.endpoint.clone(),
        ))),
        (None, None, Some(id)) if req.endpoint.is_none() => Ok(ApplyCredential::Known(id.clone())),
        _ => Err(RPCErrors::ParseRequestError(
            "apply needs either provider and fields or credential_id".to_string(),
        )),
    }
}

/// Public addresses of this machine's interfaces.
fn public_addresses() -> Vec<IpAddr> {
    let mut addresses = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .map(|interface| interface.ip())
        .filter(is_public_address)
        .collect::<Vec<_>>();
    addresses.sort();
    addresses.dedup();
    addresses
}

fn is_public_address(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || ip.is_documentation()
                || a == 0
                || a >= 240
                || (a == 100 && (64..128).contains(&b))
                || (a == 192 && b == 0 && c == 0)
                || (a == 198 && (18..20).contains(&b)))
        }
        // Global unicast without the documentation prefix.
        IpAddr::V6(ip) => {
            let segments = ip.segments();
            segments[0] & 0xe000 == 0x2000 && !(segments[0] == 0x2001 && segments[1] == 0xdb8)
        }
    }
}

/// `wan_dyn` is named in the mini document: the SN answers the reported
/// address for it. The others are answered by `gateway_ips` or the relay.
fn device_mini_document(
    device_document: &DeviceDocument,
    net_id: &str,
    iat: u64,
) -> DeviceMiniDocument {
    let mut document = DeviceMiniDocument::new_by_device_document(device_document);
    document.extra_info.insert("iat".to_string(), json!(iat));
    if net_id == "wan_dyn" {
        document
            .extra_info
            .insert("net_id".to_string(), json!(net_id));
    }
    document
}

impl ActiveServer {
    fn new(
        config: ActiveServiceConfig,
        domain: Arc<DomainService>,
        target: ActivationTarget,
    ) -> Self {
        Self {
            device_mini_info: DeviceMiniInfo::default(),
            region_probe: RegionProbeController::new(config.sn_url()),
            config,
            domain,
            target,
        }
    }

    fn sn_client(&self, access_token: &str) -> Arc<SnClient> {
        Arc::new(SnClient::new_krpc(
            self.target.sn_url.as_str(),
            Some(access_token.to_string()),
        ))
    }

    fn binding(&self, access_token: Option<&str>) -> Option<AccountBinding> {
        access_token
            .map(str::trim)
            .filter(|token| !token.is_empty())
            .map(|token| AccountBinding {
                sn: self.sn_client(token),
            })
    }

    async fn inspect_domain(
        &self,
        req: InspectDomainReq,
    ) -> Result<InspectDomainResponse, RPCErrors> {
        let binding = self.binding(req.access_token.as_deref());
        self.domain
            .inspect_domain(
                &req.domain,
                req.net_id,
                &req.owner_name,
                binding.as_ref().map(|binding| binding as &dyn DomainBinding),
            )
            .await
            .map_err(active_error)
    }

    async fn plan_domain_records(
        &self,
        req: PlanDomainRecordsReq,
    ) -> Result<PlanResponse, RPCErrors> {
        self.domain.plan(&req.setup).await.map_err(active_error)
    }

    async fn apply_domain_records(
        &self,
        req: ApplyDomainRecordsReq,
    ) -> Result<ApplyResponse, RPCErrors> {
        let credential = apply_credential(&req)?;
        self.domain
            .apply(&req.setup, credential, &req.confirm)
            .await
            .map_err(active_error)
    }

    async fn check_domain_records(
        &self,
        req: CheckDomainRecordsReq,
    ) -> Result<CheckReport, RPCErrors> {
        let binding = self.binding(req.access_token.as_deref());
        self.domain
            .check(
                &req.setup,
                binding.as_ref().map(|binding| binding as &dyn DomainBinding),
                req.credential_id.as_deref(),
                req.certificate_id.as_deref(),
                Trial::Run(CHECK_TRIAL_WAIT),
            )
            .await
            .map_err(active_error)
    }

    async fn inspect_certificate(
        &self,
        req: InspectCertificateReq,
    ) -> Result<InspectCertificateResponse, RPCErrors> {
        self.domain
            .inspect_certificate(&req.domain, &req.files, req.pfx_password)
            .await
            .map_err(active_error)
    }

    fn derive_web_owner_material(
        &self,
        req: DeriveWebOwnerMaterialReq,
    ) -> Result<WebOwnerMaterial, RPCErrors> {
        let words = req
            .mnemonic_words
            .iter()
            .map(|word| word.trim().to_lowercase())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>();
        web_owner_material(words.join(" ").as_str())
            .map_err(|_| active_error(DomainError::new("mnemonic_invalid", "")))
    }

    async fn load_owner_document(&self, req: LoadOwnerDocumentReq) -> Result<Value, RPCErrors> {
        let owner_name = req.owner_name.trim().to_lowercase();
        let document = read_owner_document(self.target.bns_url.as_str(), owner_name.as_str())
            .await
            .map_err(active_error)?
            .ok_or_else(|| active_error(DomainError::new("owner_not_found", owner_name)))?;
        Ok(json!({ "owner_document": document }))
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
        web_owner_material(mnemonic.as_str())
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

        let boot_iat = buckyos_get_unix_timestamp();
        let device_mini_iat = next_document_iat("BootDocument", boot_iat)?;
        let device_iat = next_document_iat("DeviceMiniDocument", device_mini_iat)?;
        let exp = boot_iat + DEFAULT_EXPIRE_TIME;
        let ood_net_id = if req.topology.net_id == "nat" {
            None
        } else {
            Some(req.topology.net_id.clone())
        };
        let ood =
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, ood_net_id, None);
        let boot_document = ZoneBootDocument {
            id: Some(req.names.zone_did.clone()),
            oods: vec![ood],
            sn: Some(sn_host_from_url(req.topology.sn_url.as_str())?),
            exp,
            owner: None,
            owner_key: None,
            extra_info: HashMap::from([("iat".to_string(), json!(boot_iat))]),
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
        device_document.ddns_sn_url = Some(req.topology.sn_url.clone());
        device_document.ips = req.topology.public_ips.clone();
        device_document.iat = device_iat;
        device_document.exp = exp;
        let device_mini_document = device_mini_document(
            &device_document,
            req.topology.net_id.as_str(),
            device_mini_iat,
        );
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
        let signing_key = owner_signing_key(&req.mnemonic_words, &req.prepared.owner_document)?;
        let boot_document_jwt = req
            .prepared
            .boot_document
            .encode(Some(&signing_key))
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
            .to_string();
        let device_mini_document_jwt = req
            .prepared
            .device_mini_document
            .to_jwt(&signing_key)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        let device_document_jwt = req
            .prepared
            .device_document
            .encode(Some(&signing_key))
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?
            .to_string();
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
        let sn_client = self.sn_client(req.sn.access_token.as_str());
        let domain_commit = match req.domain.as_ref() {
            None => None,
            Some(domain) => {
                let binding = AccountBinding {
                    sn: sn_client.clone(),
                };
                Some(
                    self.domain
                        .prepare_commit(
                            &CommitRequest {
                                setup: &domain.setup,
                                credential_id: domain.credential_id.as_deref(),
                                certificate_id: domain.certificate_id.as_deref(),
                                accept_warnings: domain.accept_warnings,
                            },
                            Some(&binding),
                            self.config.acme_issuer.clone(),
                            Trial::Run(COMMIT_TRIAL_WAIT),
                        )
                        .await
                        .map_err(active_error)?,
                )
            }
        };
        let bns_client = BnsIndexerClient::new_bns_server_url(self.target.bns_url.as_str(), None);
        let projected = bns_client
            .resolve_document(req.prepared.names.bns_publish_name.as_str(), "owner")
            .await
            .map_err(|error| RPCErrors::ReasonError(format!("BNS owner read failed: {error}")))?;
        let projected_owner: OwnerDocument =
            serde_json::from_slice(projected.document_state.document.inline_document.as_slice())
                .map_err(|error| {
                    RPCErrors::ReasonError(format!("invalid projected OwnerDocument: {error}"))
                })?;
        let (effective_owner, needs_owner_publish) = prepare_owner_binding_for_activation(
            &req.owner_document,
            &projected_owner,
            &req.prepared.names.zone_did,
            req.signed_documents.zone_document.iat,
        )?;

        if needs_owner_publish {
            let owner_value = serde_json::to_value(&effective_owner)
                .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
            let owner_bytes = canonical_json_bytes(&owner_value)?;
            let request_id = content_request_id_with_context(
                "owner",
                req.prepared.names.owner_name.as_str(),
                owner_bytes.as_slice(),
                req.signed_documents.zone_document_jwt.as_bytes(),
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

        let device_doc_type = req.prepared.device_document.name.as_str();
        let device_jwt = req.signed_documents.device_document_jwt.as_str();
        let device_request_id = content_request_id(
            "device",
            req.prepared.names.owner_name.as_str(),
            device_jwt.as_bytes(),
        );
        if !projection_matches_bytes(
            &bns_client,
            req.prepared.names.bns_publish_name.as_str(),
            device_doc_type,
            device_jwt.as_bytes(),
        )
        .await?
        {
            sn_client
                .publish_document(SnBnsPublishDocumentReq {
                    name: req.prepared.names.bns_publish_name.clone(),
                    doc_type: device_doc_type.to_string(),
                    document: SnBnsPublishDocumentContent::Jwt(device_jwt.to_string()),
                    request_id: Some(device_request_id),
                })
                .await?;
            wait_for_bytes_projection(
                &bns_client,
                req.prepared.names.bns_publish_name.as_str(),
                device_doc_type,
                device_jwt.as_bytes(),
            )
            .await?;
        }

        if req.prepared.device_document.id.method == "bns" {
            wait_for_device_authority(self.target.name_client(), &req).await?;
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

        if let Some(commit) = domain_commit.as_ref() {
            if commit.hosting.mode == DomainHostingMode::Sn && commit.hosting.uses_acme() {
                self.check_sn_challenge(
                    sn_client.clone(),
                    device_key_did,
                    commit.hosting.domain.as_str(),
                )
                .await?;
            }
        }

        let owner_value = serde_json::to_value(&effective_owner)
            .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
        if !projection_matches_json(
            &bns_client,
            req.prepared.names.bns_publish_name.as_str(),
            "owner",
            &owner_value,
        )
        .await?
        {
            return Err(RPCErrors::ReasonError(
                "OwnerDocument changed during activation; retry activation".to_string(),
            ));
        }

        persist_activation(
            &self.target.paths,
            self.domain.certificates(),
            &req,
            &effective_owner,
            &zone_info,
            domain_commit.as_ref(),
        )?;
        crate::host_account::save_in(
            &self.target.paths.etc_dir,
            req.system_settings.host_account.as_str(),
        )
        .map_err(RPCErrors::ReasonError)?;

        if let Some(name_client) = self.target.name_client() {
            // 激活时签出的 Zone Document 以本机配置 trust 候选投递(不改变 latest)。
            if let Err(error) = name_client
                .commit_candidate_doc(
                    &req.prepared.names.zone_did,
                    Some(DidDocType::Zone),
                    &EncodedDocument::Jwt(req.signed_documents.zone_document_jwt.clone()),
                    SourceRef::configured_trust("node-daemon-activation"),
                    name_client.default_resolve_opts(),
                )
                .await
            {
                warn!("project activated zone document into DID cache failed: {error}");
            }
        } else {
            warn!("Name client not initialized; skip activated zone document cache projection");
        }

        if self.target.exit_when_done {
            tokio::task::spawn(async move {
                tokio::time::sleep(Duration::from_secs(2)).await;
                exit(0);
            });
        }

        Ok(CommitActiveResp {
            status: "completed".to_string(),
            access_hostname: req.prepared.names.access_hostname,
        })
    }

    /// The SN only takes records of a registered device, so the trial of an SN
    /// hosted domain waits for `device.register`. Nothing of this node's
    /// identity is written yet.
    async fn check_sn_challenge(
        &self,
        sn: Arc<SnClient>,
        device_did: String,
        domain: &str,
    ) -> Result<(), RPCErrors> {
        let mut timing = self.domain.timing();
        timing.authoritative = timing.authoritative.min(SN_TRIAL_STEP_WAIT);
        timing.public = timing.public.min(SN_TRIAL_STEP_WAIT);
        let trial = tokio::spawn(run_challenge(
            self.domain.lookup.clone(),
            Arc::new(SnChallenge { sn, device_did }),
            domain.to_string(),
            vec![self.domain.names.name_server()],
            timing,
        ));
        let failure = match trial.await {
            Ok(outcome) => outcome.failure,
            Err(error) => {
                return Err(active_error(DomainError::new(
                    "sn_challenge_failed",
                    error.to_string(),
                )))
            }
        };
        match failure {
            None => Ok(()),
            Some(failure) => Err(active_error(DomainError::new(
                "sn_challenge_failed",
                format!("{}: {}", failure.code, failure.detail),
            ))),
        }
    }

    async fn handle_get_mini_device_info(
        &self,
        _req: http::Request<BoxBody<Bytes, ServerError>>,
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

fn web_owner_material(mnemonic: &str) -> Result<WebOwnerMaterial, RPCErrors> {
    let owner = derive_bucky_key_from_mnemonic(mnemonic, None, 0)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let evm = derive_evm_key_from_mnemonic(mnemonic, None, 0)
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

fn owner_signing_key(
    mnemonic_words: &[String],
    owner_document: &OwnerDocument,
) -> Result<EncodingKey, RPCErrors> {
    let mnemonic = mnemonic_words.join(" ");
    let owner = derive_bucky_key_from_mnemonic(mnemonic.as_str(), None, 0)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let evm = derive_evm_key_from_mnemonic(mnemonic.as_str(), None, 0)
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))?;
    let owner_key = owner_document
        .get_default_key()
        .ok_or_else(|| RPCErrors::ReasonError("OwnerDocument key is missing".to_string()))?;
    if serde_json::to_value(owner_key).ok() != Some(owner.public_jwk.clone()) {
        return Err(RPCErrors::ReasonError(
            "mnemonic owner key does not match OwnerDocument".to_string(),
        ));
    }
    let wallet = owner_main_wallet(owner_document)?;
    if !wallet.address.eq_ignore_ascii_case(evm.address.as_str()) {
        return Err(RPCErrors::ReasonError(
            "mnemonic EVM address does not match OwnerDocument".to_string(),
        ));
    }
    EncodingKey::from_ed_pem(owner.private_key_pem.as_bytes())
        .map_err(|error| RPCErrors::ReasonError(error.to_string()))
}

async fn wait_for_device_authority(
    client: Option<&NameClient>,
    req: &CommitActiveReq,
) -> Result<(), RPCErrors> {
    let client = client.ok_or_else(|| {
        RPCErrors::ReasonError("Device authority validation requires NameClient".into())
    })?;
    let deadline = Instant::now() + PROJECTION_DEADLINE;
    loop {
        let check = check_device_authority(
            client,
            &req.prepared.device_document,
            &req.signed_documents.device_document_jwt,
        );
        let error =
            match tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), check).await {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => error,
                Err(_) => "Device authority resolution timed out".to_string(),
            };
        if Instant::now() >= deadline {
            return Err(RPCErrors::ReasonError(format!(
                "Activation is not complete: Relay AuthorityCurrent validation failed for {}: {}",
                req.prepared.device_document.id.to_string(),
                error
            )));
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

async fn check_device_authority(
    client: &NameClient,
    device: &DeviceDocument,
    expected_jwt: &str,
) -> Result<(), String> {
    // AuthorityCurrent 门禁:关闭两种 locker,只查 authority 并要求本轮真实回答;
    // 只接受本轮 authority receipt 精确绑定的有效文档。
    let opts = ResolveOpts::default()
        .authority_only()
        .without_local_lock()
        .without_zone_lock()
        .with_force_authority_query()
        .normalize(std::time::Duration::from_secs(20));
    let result = client
        .resolve_did_ex(&device.id, Some(DidDocType::Device), Revision::current(), opts.clone())
        .await
        .map_err(|error| error.to_string())?;
    let current = select_authority_current(&result, opts.receipts());
    let matches = current
        .as_ref()
        .map_or(false, |choice| choice.doc == EncodedDocument::Jwt(expected_jwt.to_string()));
    if device.exp <= buckyos_get_unix_timestamp() || !matches {
        return Err(format!(
            "Device authority did not return the current published DeviceDocument JWT: {}",
            result.reason()
        ));
    }
    Ok(())
}

fn prepare_owner_binding_for_activation(
    owner_document: &OwnerDocument,
    projected_owner: &OwnerDocument,
    zone_did: &DID,
    activation_iat: u64,
) -> Result<(OwnerDocument, bool), RPCErrors> {
    validate_owner_document(projected_owner)?;
    if projected_owner.id != owner_document.id
        || projected_owner.name != owner_document.name
        || projected_owner.get_default_key() != owner_document.get_default_key()
        || projected_owner.wallets != owner_document.wallets
    {
        return Err(RPCErrors::ReasonError(
            "OwnerDocument identity changed; reload owner before activation".to_string(),
        ));
    }
    let mut effective_owner = projected_owner.clone();
    effective_owner.set_default_zone_did(zone_did.clone());
    let needs_owner_publish = effective_owner != *projected_owner;
    if needs_owner_publish {
        effective_owner.iat =
            activation_iat.max(next_document_iat("OwnerDocument", projected_owner.iat)?);
    }
    validate_owner_document(&effective_owner)?;
    Ok((effective_owner, needs_owner_publish))
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

/// A node of a product without containers is activated as one: its
/// DeviceDocument says `support_container = false`, whatever the client
/// sends.
fn validate_topology_for_product(
    topology: &GatewayTopology,
    product: &ProductProfile,
) -> Result<(), RPCErrors> {
    if topology.support_container && !product.container_enabled() {
        return Err(RPCErrors::ReasonError(format!(
            "this BuckyOS edition ({}) runs no containers: activate with support_container = false",
            product.edition
        )));
    }
    Ok(())
}

fn installed_product() -> Result<&'static ProductProfile, RPCErrors> {
    crate::product::installed()
        .map_err(|error| RPCErrors::ReasonError(format!("product profile unusable: {error}")))
}

fn validate_topology(topology: &GatewayTopology) -> Result<(), RPCErrors> {
    validate_topology_for_product(topology, installed_product()?)?;
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
    validate_public_ips(topology.net_id.as_str(), &topology.public_ips)
        .map_err(RPCErrors::ReasonError)
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
    let boot_iat = extra_document_iat("BootDocument", &boot.extra_info)?;
    let device_mini_iat = extra_document_iat(
        "DeviceMiniDocument",
        &prepared.device_mini_document.extra_info,
    )?;
    if device_mini_iat != next_document_iat("BootDocument", boot_iat)?
        || device.iat != next_document_iat("DeviceMiniDocument", device_mini_iat)?
    {
        return Err(RPCErrors::ReasonError(format!(
            "active document iat order mismatch: boot {}, device mini {}, device {}",
            boot_iat, device_mini_iat, device.iat
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
    let expected_sn = Some(sn_host_from_url(topology.sn_url.as_str())?);
    if boot.oods != vec![expected_ood] || boot.sn != expected_sn {
        return Err(RPCErrors::ReasonError(
            "BootDocument OOD/SN topology mismatch".to_string(),
        ));
    }
    let expected_device_did =
        build_device_did("ood1", &names.zone_did).map_err(RPCErrors::ReasonError)?;
    let expected_ddns_sn_url = Some(topology.sn_url.clone());
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
    if device.device_mini_document_jwt.is_some() {
        return Err(RPCErrors::ReasonError(
            "DeviceMiniDocument JWT must not be embedded in DeviceDocument".to_string(),
        ));
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
    if device.ips != topology.public_ips {
        return Err(RPCErrors::ReasonError(format!(
            "DeviceDocument ips mismatch: expected {:?}, got {:?}",
            topology.public_ips, device.ips
        )));
    }
    let expected_device_mini =
        device_mini_document(device, topology.net_id.as_str(), device_mini_iat);
    if prepared.device_mini_document != expected_device_mini {
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

fn extra_document_iat(
    doc_type: &str,
    extra_info: &HashMap<String, Value>,
) -> Result<u64, RPCErrors> {
    extra_info
        .get("iat")
        .and_then(Value::as_u64)
        .ok_or_else(|| RPCErrors::ReasonError(format!("{} iat is missing or invalid", doc_type)))
}

fn next_document_iat(doc_type: &str, iat: u64) -> Result<u64, RPCErrors> {
    iat.checked_add(1)
        .ok_or_else(|| RPCErrors::ReasonError(format!("{} iat cannot be incremented", doc_type)))
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
    zone_document.owner = prepared.names.owner_did.clone();
    zone_document.iat = next_document_iat("DeviceDocument", prepared.device_document.iat)?;
    zone_document.hostname = prepared.names.access_hostname.clone();
    if !prepared.topology.public_ips.is_empty() {
        zone_document.extra_info.insert(
            "gateway_ips".to_string(),
            json!(prepared.topology.public_ips),
        );
    }
    zone_document.devices.insert(
        prepared.device_document.name.clone(),
        prepared.device_document.clone(),
    );
    zone_document.mini_device_jwts.insert(
        prepared.device_document.name.clone(),
        device_mini_document_jwt.to_string(),
    );
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
    if signed.zone_document.boot_jwt != signed.boot_document_jwt {
        return Err(RPCErrors::ReasonError(
            "ZoneDocument BootDocument JWT mismatch".to_string(),
        ));
    }
    if signed
        .zone_document
        .devices
        .get(prepared.device_document.name.as_str())
        != Some(&signed.device_document)
    {
        return Err(RPCErrors::ReasonError(
            "ZoneDocument embedded DeviceDocument mismatch".to_string(),
        ));
    }
    if signed
        .zone_document
        .mini_device_jwts
        .get(prepared.device_document.name.as_str())
        != Some(&signed.device_mini_document_jwt)
    {
        return Err(RPCErrors::ReasonError(
            "ZoneDocument DeviceMiniDocument JWT mismatch".to_string(),
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
    validate_domain_commit(req)?;
    validate_prepared_relationships(&req.prepared)?;
    verify_signed_documents(&req.prepared, &req.signed_documents)?;
    validate_document_times(&req.prepared)?;
    validate_device_private_key(
        req.device_private_key.as_str(),
        &req.prepared.device_document,
    )?;
    if let Some(words) = req.owner_mnemonic_words.as_deref() {
        owner_signing_key(words, &req.owner_document)?;
    }
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
    validate_host_account(
        req.system_settings.host_account.as_str(),
        &crate::host_account::available_accounts(),
    )
}

fn validate_domain_commit(req: &CommitActiveReq) -> Result<(), RPCErrors> {
    let names = &req.prepared.names;
    if req.domain.is_some() != names.use_self_domain {
        return Err(RPCErrors::ReasonError(
            "domain goes with use_self_domain, and only with it".to_string(),
        ));
    }
    let Some(setup) = req.domain.as_ref().map(|domain| &domain.setup) else {
        return Ok(());
    };
    let topology = &req.prepared.topology;
    if setup.domain != names.access_hostname
        || setup.owner_name != names.owner_name
        || setup.net_id.as_str() != topology.net_id
        || setup.public_ips != topology.public_ips
    {
        return Err(RPCErrors::ReasonError(
            "domain setup differs from the prepared documents".to_string(),
        ));
    }
    Ok(())
}

fn validate_host_account(account: &str, available: &[String]) -> Result<(), RPCErrors> {
    if account.is_empty() || available.iter().any(|name| name == account) {
        return Ok(());
    }
    Err(RPCErrors::ReasonError(format!(
        "host account `{account}` is not available on this node"
    )))
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

fn content_request_id_with_context(
    operation: &str,
    owner_name: &str,
    bytes: &[u8],
    context: &[u8],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update((bytes.len() as u64).to_be_bytes());
    hasher.update(bytes);
    hasher.update((context.len() as u64).to_be_bytes());
    hasher.update(context);
    let digest = hasher.finalize();
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

fn build_start_config(
    req: &CommitActiveReq,
    owner_document: &OwnerDocument,
    hosting: Option<&DomainHosting>,
) -> Value {
    let mut config = json!({
        "schema": buckyos_api::START_CONFIG_SCHEMA,
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
        "enabled_features": req.system_settings.enabled_features
    });
    if let Some(hosting) = hosting {
        config[buckyos_api::START_CONFIG_DOMAIN_HOSTING] = json!(hosting);
    }
    if let Some(words) = req.owner_mnemonic_words.as_ref() {
        config[START_CONFIG_OWNER_MNEMONIC] = json!(words);
    }
    config
}

#[derive(Clone)]
struct ActivationPaths {
    etc_dir: PathBuf,
    roots: IdentityRoots,
}

/// The credential and the certificate go first, the identity that marks the
/// node as activated after them.
fn persist_activation(
    paths: &ActivationPaths,
    certificates: &dyn CertificateTool,
    req: &CommitActiveReq,
    effective_owner: &OwnerDocument,
    zone_info: &SnZoneInfoResp,
    domain: Option<&DomainCommit>,
) -> Result<(), RPCErrors> {
    if let Some(domain) = domain {
        install_domain(&paths.roots.security_root, certificates, domain)
            .map_err(RPCErrors::ReasonError)?;
    }
    let credential_id = domain.and_then(|domain| domain.hosting.credential_id.as_deref());
    if let Err(error) = write_activation(paths, req, effective_owner, zone_info, domain) {
        if let Some(id) = credential_id {
            remove_credential(&paths.roots.security_root, id);
        }
        return Err(RPCErrors::ReasonError(error));
    }
    remove_other_credentials(&paths.roots.security_root, credential_id);
    replace_last_known_zone_state_in(
        paths.etc_dir.as_path(),
        &req.signed_documents.zone_document,
    );
    Ok(())
}

fn write_activation(
    paths: &ActivationPaths,
    req: &CommitActiveReq,
    effective_owner: &OwnerDocument,
    zone_info: &SnZoneInfoResp,
    domain: Option<&DomainCommit>,
) -> Result<(), String> {
    let etc_dir = paths.etc_dir.as_path();
    let start_config = build_start_config(
        req,
        effective_owner,
        domain.map(|domain| &domain.hosting),
    );
    let start_config_path = paths.roots.security_root.join(START_CONFIG_FILE);
    if req.owner_mnemonic_words.is_some() {
        std::fs::create_dir_all(&paths.roots.security_root)
            .and_then(|_| crate::boot::write_private_json(&start_config_path, &start_config))
            .map_err(|error| format!("write {} failed: {error}", start_config_path.display()))?;
    } else {
        let content =
            serde_json::to_vec_pretty(&start_config).map_err(|error| error.to_string())?;
        atomic_write(start_config_path.as_path(), &content)?;
    }
    save_sn_zone_info_in(etc_dir, zone_info)?;
    save_zone_document_jwt(etc_dir, req.signed_documents.zone_document_jwt.as_str())?;

    let device = &req.prepared.device_document;
    let node_identity = LocalNodeIdentityConfig::new(
        req.prepared.names.zone_did.clone(),
        req.prepared.names.owner_did.clone(),
        effective_owner
            .get_default_key()
            .ok_or_else(|| "OwnerDocument default key is missing".to_string())?,
        device.name.clone(),
        device.id.clone(),
        req.signed_documents.zone_document.iat as u32,
    );
    save_local_device_identity_for_roots(
        etc_dir,
        &paths.roots,
        &node_identity,
        device,
        req.signed_documents.device_document_jwt.as_str(),
        req.signed_documents.device_mini_document_jwt.as_str(),
        req.device_private_key.as_str(),
    )?;
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
            "get_product_profile" => rpc_success(installed_product()?, seq),
            "list_host_accounts" => rpc_success(
                json!({ "accounts": crate::host_account::available_accounts() }),
                seq,
            ),
            "derive_web_owner_material" => {
                let params = DeriveWebOwnerMaterialReq::from_json(req.params)?;
                rpc_success(self.derive_web_owner_material(params)?, seq)
            }
            "load_owner_document" => {
                let params = LoadOwnerDocumentReq::from_json(req.params)?;
                rpc_success(self.load_owner_document(params).await?, seq)
            }
            "list_public_addresses" => {
                ListPublicAddressesReq::from_json(req.params)?;
                rpc_success(json!({ "addresses": public_addresses() }), seq)
            }
            "list_dns_providers" => {
                ListDnsProvidersReq::from_json(req.params)?;
                rpc_success(json!({ "providers": DomainService::list_providers() }), seq)
            }
            "inspect_domain" => {
                let params = InspectDomainReq::from_json(req.params)?;
                rpc_success(self.inspect_domain(params).await?, seq)
            }
            "plan_domain_records" => {
                let params = PlanDomainRecordsReq::from_json(req.params)?;
                rpc_success(self.plan_domain_records(params).await?, seq)
            }
            "apply_domain_records" => {
                let params = ApplyDomainRecordsReq::from_json(req.params)?;
                rpc_success(self.apply_domain_records(params).await?, seq)
            }
            "check_domain_records" => {
                let params = CheckDomainRecordsReq::from_json(req.params)?;
                rpc_success(self.check_domain_records(params).await?, seq)
            }
            "inspect_certificate" => {
                let params = InspectCertificateReq::from_json(req.params)?;
                rpc_success(self.inspect_certificate(params).await?, seq)
            }
            "start_region_probe" => {
                let params = StartRegionProbeReq::from_json(req.params)?;
                rpc_success(self.region_probe.start(params.force).await, seq)
            }
            "get_region_probe_status" => {
                GetRegionProbeStatusReq::from_json(req.params)?;
                rpc_success(self.region_probe.status().await, seq)
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
            return serve_http_by_rpc_handler_with_limit(req, info, self, MAX_REQUEST_BODY).await;
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
    let config: ActiveServiceConfig = match std::fs::read(&config_path)
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
    let roots = match identity_roots() {
        Ok(roots) => roots,
        Err(error) => {
            error!("Failed to load identity roots: {}", error);
            return;
        }
    };
    let domain = Arc::new(DomainService::new(
        SnNames::new(config.sn_base_host.as_str()),
        SystemDnsLookup::new(),
        Arc::new(DnsServices),
        Arc::new(BnsOwnerKeys {
            bns_url: config.bns_url(),
        }),
        Arc::new(IdentityCertificates {
            roots: roots.clone(),
        }),
    ));
    let target = ActivationTarget::local(&config, roots);
    let mut active_server = ActiveServer::new(config, domain, target);
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
    if let Err(error) = runner.run().await {
        error!("active server stopped: {:?}", error);
    }
}

#[cfg(test)]
#[path = "active_commit_tests.rs"]
mod commit_tests;

#[cfg(test)]
#[path = "active_sn_tests.rs"]
mod sn_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_account_must_be_empty_or_offered_by_the_node() {
        let available = vec!["root".to_string(), "lzc".to_string()];
        validate_host_account("", &[]).unwrap();
        validate_host_account("lzc", &available).unwrap();
        validate_host_account("root", &available).unwrap();
        assert!(validate_host_account("daemon", &available).is_err());
        assert!(validate_host_account("root", &[]).is_err());
    }

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
            public_ips: Vec::new(),
            sn_url: "https://sn.example.com/kapi/sn".to_string(),
        }
    }

    fn topology_of(net_id: &str, public_ips: &[&str]) -> GatewayTopology {
        GatewayTopology {
            net_id: net_id.to_string(),
            public_ips: public_ips.iter().map(|ip| ip.parse().unwrap()).collect(),
            ..topology()
        }
    }

    fn active_service_config() -> ActiveServiceConfig {
        ActiveServiceConfig {
            sn_base_host: "example.com".to_string(),
            http_schema: "https".to_string(),
            acme_issuer: None,
        }
    }

    fn active_server() -> ActiveServer {
        let config = active_service_config();
        let target = ActivationTarget::local(
            &config,
            identity_roots_for_buckyos_root(std::path::Path::new("/nonexistent")),
        );
        ActiveServer::new(
            config,
            Arc::new(crate::domain::testing::fake_service()),
            target,
        )
    }

    const MNEMONIC: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    async fn signed_documents(
        domain: Option<&str>,
        topology: GatewayTopology,
    ) -> (PreparedActiveDocuments, SignedActiveDocuments, String) {
        let owner = owner_document(MNEMONIC);
        let names = ActiveNameMapping::derive(
            &owner,
            domain.unwrap_or("alice.web3.example.com"),
            domain.is_some(),
        );
        let (device_private_key, device_public_key) = generate_ed25519_key_pair();
        let server = active_server();
        let prepared = server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: owner,
                names,
                topology,
                device_public_key,
            })
            .await
            .unwrap();
        validate_prepared_relationships(&prepared).unwrap();
        let signed = server
            .sign_web_active_documents(SignWebActiveDocumentsReq {
                mnemonic_words: MNEMONIC.split_whitespace().map(ToString::to_string).collect(),
                prepared: prepared.clone(),
            })
            .unwrap();
        verify_signed_documents(&prepared, &signed).unwrap();
        (prepared, signed, device_private_key)
    }

    fn commit_request(
        prepared: &PreparedActiveDocuments,
        signed: SignedActiveDocuments,
        device_private_key: &str,
        domain: Option<DomainCommitConfig>,
    ) -> CommitActiveReq {
        CommitActiveReq {
            owner_document: prepared.owner_document.clone(),
            prepared: prepared.clone(),
            signed_documents: signed,
            device_private_key: device_private_key.to_string(),
            system_settings: LocalSystemSettings {
                admin_password_hash: "required-admin-hash".to_string(),
                guest_access: false,
                friend_passcode: String::new(),
                enabled_features: json!({}),
                host_account: String::new(),
            },
            sn: SnCommitConfig {
                sn_url: "https://sn.example.com/kapi/sn/".to_string(),
                bns_url: "https://bns.example.com/kapi/bns".to_string(),
                access_token: "secret-access-token".to_string(),
            },
            domain,
            owner_mnemonic_words: None,
        }
    }

    fn sn_zone_info() -> SnZoneInfoResp {
        SnZoneInfoResp {
            code: 0,
            zone: "alice".to_string(),
            bns_name: "alice".to_string(),
            relay_sn: Some("relay.example.com".to_string()),
            self_cert: false,
            cert_checked_at: None,
            cert_expires_at: None,
            source_version: Some("v2".to_string()),
            updated_at: 1,
        }
    }

    #[tokio::test]
    #[ignore = "cross-repository BNS/Relay integration fixture export"]
    async fn export_node_active_authority_fixture() {
        let path = std::env::var("BUCKYOS_ACTIVE_FIXTURE").expect("fixture output path");
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = active_server();
        let prepared = server
            .prepare_active_documents(PrepareActiveDocumentsReq {
                owner_document: owner,
                names,
                topology: topology(),
                device_public_key,
            })
            .await
            .unwrap();
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
        std::fs::write(
            path,
            serde_json::to_vec(&json!({
                "owner": prepared.owner_document,
                "zone_jwt": signed.zone_document_jwt,
                "device_jwt": signed.device_document_jwt,
                "device": prepared.device_document
            }))
            .unwrap(),
        )
        .unwrap();
    }


    struct PublishedDevice(std::sync::Mutex<Option<EncodedDocument>>);

    #[async_trait]
    impl name_client::Source for PublishedDevice {
        fn id(&self) -> String {
            "published-device".to_string()
        }

        async fn get_document(
            &self,
            _did: &DID,
            _doc_type: &DidDocType,
            _revision: &Revision,
            _opts: &ResolveOpts,
        ) -> name_client::SourceAnswer {
            let answer = name_client::SourceAnswer::new(SourceRef::provider(
                self.id(),
                name_client::SourceTier::Authority,
            ));
            match self.0.lock().unwrap().clone() {
                Some(document) => answer.with_body(document, name_client::Carrier::Body),
                None => answer,
            }
        }
    }

    #[tokio::test]
    async fn the_published_bns_device_is_accepted_and_stale_ones_are_not() {
        use name_client::{CacheBackend, NameClientConfig};

        let owner = derive_bucky_key_from_mnemonic(MNEMONIC, None, 0).unwrap();
        let key = EncodingKey::from_ed_pem(owner.private_key_pem.as_bytes()).unwrap();
        let (_, public) = generate_ed25519_key_pair();
        let mut device = new_device_config_by_jwk_with_did(
            "ood1",
            serde_json::from_value(public).unwrap(),
            &DID::new("bns", "ood1.alice"),
        )
        .unwrap();
        device.owner = DID::new("bns", "alice");
        device.zone_did = Some(DID::new("bns", "alice"));
        let jwt = device.encode(Some(&key)).unwrap().to_string();
        let published = Arc::new(PublishedDevice(std::sync::Mutex::new(Some(
            EncodedDocument::Jwt(jwt.clone()),
        ))));
        let client = NameClient::new(NameClientConfig {
            cache_backend: CacheBackend::Memory,
            enable_zone_locker: false,
            ..Default::default()
        });
        client.set_method_authority("bns", published.clone());
        client.lock_document(
            DID::new("bns", "alice"),
            DidDocType::Owner,
            EncodedDocument::JsonLd(serde_json::to_value(owner_document(MNEMONIC)).unwrap()),
            name_client::LockOptions {
                scope: "test".to_string(),
                simulate_history: true,
                ..Default::default()
            },
        );
        let publish = |document: Option<EncodedDocument>| *published.0.lock().unwrap() = document;

        check_device_authority(&client, &device, &jwt).await.unwrap();
        publish(None);
        assert!(check_device_authority(&client, &device, &jwt).await.is_err());
        let json = device.encode(Some(&key)).unwrap().to_json_value().unwrap();
        publish(Some(EncodedDocument::JsonLd(json)));
        assert!(check_device_authority(&client, &device, &jwt).await.is_err());
        let mut replaced = device.clone();
        replaced.iat += 1;
        let replacement = replaced.encode(Some(&key)).unwrap().to_string();
        publish(Some(EncodedDocument::Jwt(replacement.clone())));
        assert!(check_device_authority(&client, &device, &jwt).await.is_err());
        check_device_authority(&client, &replaced, &replacement).await.unwrap();
        replaced.exp = buckyos_get_unix_timestamp() - 1;
        let expired = replaced.encode(Some(&key)).unwrap().to_string();
        publish(Some(EncodedDocument::Jwt(expired.clone())));
        assert!(check_device_authority(&client, &replaced, &expired).await.is_err());
    }

    #[test]
    fn every_request_struct_has_strict_parsing() {
        assert!(GenerateWebOwnerMaterialReq::from_json(json!({})).is_ok());
        assert!(GenerateDeviceKeyPairReq::from_json(json!({})).is_ok());
        assert!(StartRegionProbeReq::from_json(json!({})).is_ok());
        assert!(StartRegionProbeReq::from_json(json!({"force": true})).is_ok());
        assert!(GetRegionProbeStatusReq::from_json(json!({})).is_ok());
        assert!(GenerateWebOwnerMaterialReq::from_json(json!({"extra": true})).is_err());
        assert!(StartRegionProbeReq::from_json(json!({"extra": true})).is_err());
        assert!(GetRegionProbeStatusReq::from_json(json!({"extra": true})).is_err());
        assert!(ListPublicAddressesReq::from_json(json!({})).is_ok());
        assert!(ListPublicAddressesReq::from_json(json!({"extra": true})).is_err());
        assert!(ListDnsProvidersReq::from_json(json!({})).is_ok());
        assert!(ListDnsProvidersReq::from_json(json!({"extra": true})).is_err());
        let words = json!({"mnemonic_words": ["abandon"]});
        assert!(DeriveWebOwnerMaterialReq::from_json(words).is_ok());
        assert!(DeriveWebOwnerMaterialReq::from_json(json!({})).is_err());
        assert!(LoadOwnerDocumentReq::from_json(json!({"owner_name": "alice"})).is_ok());
        assert!(LoadOwnerDocumentReq::from_json(json!({})).is_err());
        let setup = json!({"domain": "home.example.org", "mode": "dns_api", "certificate": "acme",
            "net_id": "nat", "public_ips": [], "owner_name": "alice"});
        fn accepts<T>(
            parse: impl Fn(Value) -> Result<T, RPCErrors>,
            value: Value,
        ) {
            assert!(parse(value.clone()).is_ok(), "{value}");
            let mut extra = value.clone();
            extra["extra"] = json!(true);
            assert!(parse(extra).is_err(), "{value}");
            assert!(parse(json!({})).is_err(), "{value}");
        }
        accepts(
            InspectDomainReq::from_json,
            json!({"domain": "home.example.org", "net_id": "wan", "owner_name": "alice"}),
        );
        accepts(PlanDomainRecordsReq::from_json, json!({"setup": setup}));
        accepts(
            ApplyDomainRecordsReq::from_json,
            json!({"setup": setup, "provider": "cloudflare", "fields": {"api_token": "x"}, "confirm": []}),
        );
        accepts(
            ApplyDomainRecordsReq::from_json,
            json!({"setup": setup, "credential_id": "x", "confirm": ["home.example.org"]}),
        );
        accepts(
            CheckDomainRecordsReq::from_json,
            json!({"setup": setup, "access_token": "t", "credential_id": "c", "certificate_id": null}),
        );
        accepts(
            InspectCertificateReq::from_json,
            json!({"domain": "home.example.org", "files": ["YQ=="]}),
        );
        accepts(
            |value| parse_params::<DomainCommitConfig>(value, "DomainCommitConfig"),
            json!({"setup": setup, "credential_id": "c", "accept_warnings": false}),
        );
        for error in [
            DeriveWebOwnerMaterialReq::from_json(json!({"mnemonic_words": "secret words"})).err(),
            ApplyDomainRecordsReq::from_json(
                json!({"setup": setup, "provider": "x", "fields": "secret words", "confirm": []}),
            )
            .err(),
            InspectCertificateReq::from_json(json!({"domain": "x", "files": "secret words"})).err(),
        ] {
            let error = error.unwrap().to_string();
            assert!(!error.contains("secret"), "{error}");
        }
        let mut bad_setup = setup.clone();
        bad_setup["token"] = json!("x");
        assert!(PlanDomainRecordsReq::from_json(json!({"setup": bad_setup})).is_err());
        assert!(serde_json::from_value::<LocalSystemSettings>(
            json!({"admin_password_hash": "x", "ai_provider_config": {}})
        )
        .is_err());
        assert!(serde_json::from_value::<GatewayTopology>(
            json!({"net_id": "nat", "uses_sn_relay": true, "public_ips": [], "sn_url": "https://sn.example.com/kapi/sn"})
        )
        .is_err());
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

    #[test]
    fn activation_explicitly_binds_same_name_default_zone_as_v2() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let (effective, needs_publish) =
            prepare_owner_binding_for_activation(&owner, &owner, &owner.id, owner.iat).unwrap();

        assert!(needs_publish);
        let value = serde_json::to_value(&effective).unwrap();
        assert_eq!(value["zone_binding_model_version"], json!(2));
        assert_eq!(value["binded_zone_list"], json!(["did:bns:alice"]));
        assert!(value["service"].as_array().unwrap().iter().any(|service| {
            service["id"] == "did:bns:alice#lastDoc"
                && service["serviceEndpoint"] == "https://alice.bns.did/resolve/did:bns:alice"
        }));
    }

    #[test]
    fn activation_promotes_selected_zone_without_losing_other_zones() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mut owner = owner_document(mnemonic);
        let first = DID::new("web", "first.example.com");
        let selected = DID::new("web", "selected.example.com");
        owner.set_default_zone_did(first.clone());
        owner.set_default_zone_did(selected.clone());
        owner.set_default_zone_did(first);

        let (effective, needs_publish) =
            prepare_owner_binding_for_activation(&owner, &owner, &selected, owner.iat).unwrap();
        assert!(needs_publish);
        assert_eq!(
            effective
                .binded_zone_list
                .iter()
                .map(DID::to_string)
                .collect::<Vec<_>>(),
            vec![
                "did:web:selected.example.com".to_string(),
                "did:web:first.example.com".to_string(),
            ]
        );

        let (unchanged, needs_publish) = prepare_owner_binding_for_activation(
            &effective, &effective, &selected, effective.iat + 60,
        ).unwrap();
        assert!(!needs_publish);
        assert_eq!(unchanged, effective);
    }

    #[test]
    fn activation_rebind_uses_projected_owner_instead_of_stale_input() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let mut projected = owner_document(mnemonic);
        let activation_iat = buckyos_get_unix_timestamp();
        projected.iat = activation_iat + 60;
        let mut stale = projected.clone();
        stale.set_default_zone_did(stale.id.clone());
        projected.set_default_zone_did(DID::new("web", "other.example.com"));
        projected.display_name = "Updated remote profile".to_string();
        let (effective, needs_publish) =
            prepare_owner_binding_for_activation(&stale, &projected, &stale.id, activation_iat).unwrap();
        assert!(needs_publish);
        assert_eq!(effective.display_name, projected.display_name);
        assert_eq!(effective.iat, projected.iat + 1);
        let (retry, _) = prepare_owner_binding_for_activation(
            &stale, &projected, &stale.id, activation_iat,
        ).unwrap();
        assert_eq!(retry, effective);
        assert_eq!(
            effective.binded_zone_list,
            vec![stale.id.clone(), DID::new("web", "other.example.com")]
        );
        let (unchanged, needs_publish) =
            prepare_owner_binding_for_activation(&stale, &effective, &stale.id, activation_iat).unwrap();
        assert!(!needs_publish);
        assert_eq!(unchanged, effective);

        let mut unbound = owner_document(mnemonic);
        unbound.iat = activation_iat - 100;
        let (rebound, needs_publish) =
            prepare_owner_binding_for_activation(&stale, &unbound, &stale.id, activation_iat).unwrap();
        assert!(needs_publish);
        assert_eq!(rebound.binded_zone_list, vec![stale.id.clone()]);
        assert!(rebound.iat > unbound.iat);
        assert_eq!(rebound.iat, activation_iat);
        projected.id = DID::new("bns", "other");
        assert!(prepare_owner_binding_for_activation(&stale, &projected, &stale.id, activation_iat).is_err());
    }

    #[test]
    fn owner_publish_request_id_is_stable_per_attempt_but_fresh_for_rebind() {
        let owner = br#"{"id":"did:bns:alice","binded_zone_list":["did:bns:alice"]}"#;
        let first_attempt =
            content_request_id_with_context("owner", "alice", owner, b"first-zone-document-jwt");
        let first_retry =
            content_request_id_with_context("owner", "alice", owner, b"first-zone-document-jwt");
        let rebind_attempt =
            content_request_id_with_context("owner", "alice", owner, b"second-zone-document-jwt");

        assert_eq!(first_attempt, first_retry);
        assert_ne!(first_attempt, rebind_attempt);
    }

    #[tokio::test]
    async fn web_signing_round_trip_builds_four_verified_documents() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = active_server();
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
        let boot_iat = boot_document.get("iat").unwrap().as_u64().unwrap();
        let device_mini_iat = serde_json::to_value(&prepared.device_mini_document)
            .unwrap()
            .get("iat")
            .unwrap()
            .as_u64()
            .unwrap();
        assert_eq!(device_mini_iat, boot_iat + 1);
        assert_eq!(prepared.device_document.iat, device_mini_iat + 1);
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
        assert_eq!(signed.zone_document.owner, prepared.names.owner_did);
        assert_eq!(signed.zone_document.iat, prepared.device_document.iat + 1);
        assert_eq!(signed.zone_document.boot_jwt, signed.boot_document_jwt);
        assert_eq!(
            signed.zone_document.devices.get("ood1"),
            Some(&signed.device_document)
        );
        assert!(signed
            .zone_document
            .devices
            .get("ood1")
            .unwrap()
            .device_mini_document_jwt
            .is_none());
        assert_eq!(
            signed.zone_document.mini_device_jwts.get("ood1"),
            Some(&signed.device_mini_document_jwt)
        );
        assert!(signed.zone_document_jwt.len() < MAX_INLINE_DOCUMENT);

        let mut commit_req =
            commit_request(&prepared, signed, "not-persisted-in-start-config", None);
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
        let config = build_start_config(&commit_req, &prepared.owner_document, None);
        buckyos_api::validate_start_config_schema(&config).unwrap();
        let config_text = serde_json::to_string(&config).unwrap();
        for forbidden in [
            "mnemonic_words",
            "device_private_key",
            "secret-access-token",
            "pwd_hash",
        ] {
            assert!(!config_text.contains(forbidden));
        }
        for removed in ["owner_telegram_account_id", "ai_provider_config", "domain_hosting"] {
            assert!(config.get(removed).is_none(), "{removed}");
        }

        let mut zone_info = sn_zone_info();
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
    async fn prepared_documents_cannot_change_after_signing() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = active_server();
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
        let device_mini_iat = extra_document_iat(
            "DeviceMiniDocument",
            &prepared.device_mini_document.extra_info,
        )
        .unwrap();
        prepared
            .device_mini_document
            .extra_info
            .insert("iat".to_string(), json!(prepared.device_document.iat));
        assert!(validate_prepared_relationships(&prepared).is_err());
        prepared
            .device_mini_document
            .extra_info
            .insert("iat".to_string(), json!(device_mini_iat));
        prepared.device_document.device_mini_document_jwt = Some("mini.jwt.value".to_string());
        assert!(validate_prepared_relationships(&prepared).is_err());
        prepared.device_document.device_mini_document_jwt = None;
        prepared.topology.net_id = "wan".to_string();
        assert!(validate_prepared_relationships(&prepared).is_err());
    }

    #[tokio::test]
    async fn tampered_nested_jwt_is_rejected() {
        let mnemonic = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
        let owner = owner_document(mnemonic);
        let names = ActiveNameMapping::derive(&owner, "alice.web3.example.com", false);
        let (_, device_public_key) = generate_ed25519_key_pair();
        let server = active_server();
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

    #[test]
    fn a_product_without_containers_activates_only_without_them() {
        let lite = ProductProfile::parse(include_str!("../../../product/lite.json")).unwrap();
        let mut topology = GatewayTopology {
            net_id: "nat".to_string(),
            rtcp_port: 2980,
            support_container: true,
            public_ips: Vec::new(),
            sn_url: "https://sn.buckyos.ai/kapi/sn".to_string(),
        };
        assert!(validate_topology_for_product(&topology, &lite).is_err());
        assert!(validate_topology_for_product(&topology, &ProductProfile::full()).is_ok());
        topology.support_container = false;
        assert!(validate_topology_for_product(&topology, &lite).is_ok());
    }

    /// What the SN answers for the Zone, by the rules of cyfs-sn
    /// `sn_resolver.rs`: `find_gateway_ips` (Zone document path
    /// `gateway_ips`) first, then the reported address when
    /// `device_document_network_type` (mini JWT payload path `net_id`) is
    /// `wan_dyn` or `portmap`, the relay otherwise.
    fn sn_answers(signed: &SignedActiveDocuments) -> String {
        let payload = |jwt: &str| {
            EncodedDocument::Jwt(jwt.to_string())
                .to_json_value()
                .unwrap()
        };
        let zone = payload(&signed.zone_document_jwt);
        let gateway_ips = zone["gateway_ips"]
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|ip| ip.as_str().unwrap().parse::<IpAddr>().unwrap())
            .collect::<Vec<_>>();
        for other in ["gateway", "addresses"] {
            assert!(zone.get(other).is_none());
        }
        if let Some(ip) = gateway_ips.iter().find(|ip| ip.is_ipv4()) {
            return ip.to_string();
        }
        let mini = payload(zone["mini_device_jwts"]["ood1"].as_str().unwrap());
        match mini.get("net_id").and_then(Value::as_str) {
            Some("wan_dyn" | "portmap") => "reported".to_string(),
            _ => "relay".to_string(),
        }
    }

    #[tokio::test]
    async fn every_access_method_publishes_what_the_sn_answers_from() {
        for domain in [None, Some("home.example.org")] {
            for (net_id, ips, answer) in [
                ("nat", &[][..], "relay"),
                ("portmap", &[], "relay"),
                ("wan_dyn", &[], "reported"),
                ("wan", &["203.0.113.7"], "203.0.113.7"),
                ("wan", &["203.0.113.7", "2001:db8::7"], "203.0.113.7"),
            ] {
                let topology = topology_of(net_id, ips);
                validate_topology(&topology).unwrap();
                let (prepared, signed, _) = signed_documents(domain, topology.clone()).await;
                assert_eq!(sn_answers(&signed), answer, "{domain:?} {net_id}");
                assert_eq!(signed.zone_document.sn.as_deref(), Some("sn.example.com"));
                assert_eq!(signed.boot_document.sn.as_deref(), Some("sn.example.com"));
                assert_eq!(
                    signed.device_document.ddns_sn_url.as_deref(),
                    Some("https://sn.example.com/kapi/sn")
                );
                assert_eq!(signed.device_document.ips, topology.public_ips);
                assert_eq!(signed.device_document.net_id.as_deref(), Some(net_id));
                assert_eq!(
                    crate::sn_zone_info::device_requires_sn_keep_tunnel(&signed.device_document),
                    net_id != "wan",
                );
                assert_eq!(
                    signed.zone_document.extra_info.contains_key("gateway_ips"),
                    net_id == "wan"
                );
                assert_eq!(
                    prepared.device_mini_document.extra_info.get("net_id"),
                    (net_id == "wan_dyn").then(|| json!("wan_dyn")).as_ref()
                );
            }
        }
    }

    #[tokio::test]
    async fn the_zone_document_stays_inline_with_gateway_addresses() {
        let long = format!("{}.{}.example.org", "a".repeat(40), "b".repeat(40));
        let (_, signed, _) = signed_documents(
            Some(long.as_str()),
            topology_of("wan", &["203.0.113.107", "2001:db8:1111:2222:3333:4444:5555:6666"]),
        )
        .await;
        assert!(
            signed.zone_document_jwt.len() < MAX_INLINE_DOCUMENT,
            "{}",
            signed.zone_document_jwt.len()
        );
    }

    #[test]
    fn public_ips_belong_to_wan() {
        for bad in [
            topology_of("wan", &[]),
            topology_of("wan", &["2001:db8::7"]),
            topology_of("wan", &["203.0.113.7", "203.0.113.8"]),
            topology_of("nat", &["203.0.113.7"]),
            topology_of("wan_dyn", &["203.0.113.7"]),
        ] {
            assert!(validate_topology(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_global_addresses_are_offered() {
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700::1111"] {
            assert!(is_public_address(&ip.parse().unwrap()), "{ip}");
        }
        for ip in [
            "10.0.0.1", "192.168.1.2", "172.16.0.1", "100.64.0.1", "127.0.0.1", "169.254.1.1",
            "198.18.0.1", "203.0.113.7", "0.0.0.0", "240.0.0.1", "::1", "fe80::1", "fd00::1",
            "2001:db8::1",
        ] {
            assert!(!is_public_address(&ip.parse().unwrap()), "{ip}");
        }
    }

    #[test]
    fn owner_material_is_derived_again_from_the_mnemonic() {
        let server = active_server();
        let first = server
            .derive_web_owner_material(DeriveWebOwnerMaterialReq {
                mnemonic_words: MNEMONIC.split(' ').map(ToString::to_string).collect(),
            })
            .unwrap();
        let expected = derive_bucky_key_from_mnemonic(MNEMONIC, None, 0).unwrap();
        assert_eq!(first.owner_public_jwk, expected.public_jwk);
        assert_eq!(first.mnemonic_words.len(), 12);
        let error = server
            .derive_web_owner_material(DeriveWebOwnerMaterialReq {
                mnemonic_words: vec!["not".to_string(), "a-mnemonic".to_string()],
            })
            .unwrap_err()
            .to_string();
        assert!(error.contains("[active:mnemonic_invalid]"), "{error}");
        assert!(!error.contains("a-mnemonic"), "{error}");
    }

    fn domain_setup(domain: &str, mode: &str, certificate: &str) -> DomainSetup {
        serde_json::from_value(json!({
            "domain": domain, "mode": mode, "certificate": certificate,
            "net_id": "nat", "public_ips": [], "owner_name": "alice"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn the_domain_setup_must_match_the_documents() {
        let (prepared, signed, key) = signed_documents(Some("home.example.org"), topology()).await;
        let config = |setup: DomainSetup| {
            Some(DomainCommitConfig {
                setup,
                credential_id: None,
                certificate_id: None,
                accept_warnings: false,
            })
        };
        let good = domain_setup("home.example.org", "sn", "acme");
        let request = |domain| commit_request(&prepared, signed.clone(), &key, domain);
        validate_commit_request(&request(config(good.clone())), &active_service_config()).unwrap();
        let mut other_net = good.clone();
        other_net.net_id = NetId::WanDyn;
        let mut other_owner = good.clone();
        other_owner.owner_name = "bob".to_string();
        for domain in [
            None,
            config(domain_setup("other.example.org", "sn", "acme")),
            config(other_net),
            config(other_owner),
        ] {
            assert!(validate_commit_request(&request(domain), &active_service_config()).is_err());
        }

        let (prepared, signed, key) = signed_documents(None, topology()).await;
        let mut request = commit_request(&prepared, signed, &key, None);
        validate_commit_request(&request, &active_service_config()).unwrap();
        request.domain = config(good);
        assert!(validate_commit_request(&request, &active_service_config()).is_err());
    }

    use crate::domain::testing::{FakeCertificates, TEST_TOKEN};

    fn commit_of(mode: &str, certificate: &str, credential_id: Option<&str>) -> DomainCommit {
        let credential = credential_id.map(|_| {
            credential_of(
                "cloudflare".to_string(),
                BTreeMap::from([("api_token".to_string(), TEST_TOKEN.to_string())]),
                None,
            )
        });
        DomainCommit {
            hosting: serde_json::from_value(json!({
                "schema_version": 1, "mode": mode, "certificate": certificate, "domain": "home.example.org",
                "provider": credential.as_ref().map(|c| c.provider.clone()),
                "credential_id": credential_id
            }))
            .unwrap(),
            credential,
            certificates: None,
        }
    }

    fn activation_paths(root: &std::path::Path) -> ActivationPaths {
        ActivationPaths {
            etc_dir: root.join("etc"),
            roots: identity_roots_for_buckyos_root(root),
        }
    }

    #[tokio::test]
    async fn owner_mnemonic_is_stored_only_on_request() {
        let (prepared, signed, key) = signed_documents(None, topology()).await;
        let words = |text: &str| text.split_whitespace().map(ToString::to_string).collect::<Vec<_>>();
        let mut request = commit_request(&prepared, signed, &key, None);
        let store = |request: &CommitActiveReq| {
            let root = tempfile::tempdir().unwrap();
            let paths = activation_paths(root.path());
            std::fs::create_dir_all(&paths.etc_dir).unwrap();
            persist_activation(
                &paths,
                &FakeCertificates::default(),
                request,
                &prepared.owner_document,
                &sn_zone_info(),
                None,
            )
            .unwrap();
            let path = paths.roots.security_root.join(START_CONFIG_FILE);
            #[cfg(unix)]
            if request.owner_mnemonic_words.is_some() {
                use std::os::unix::fs::PermissionsExt;
                let mode = std::fs::metadata(&path).unwrap().permissions().mode();
                assert_eq!(mode & 0o777, 0o600);
            }
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains("PRIVATE KEY"));
            serde_json::from_str::<Value>(&text).unwrap()
        };

        validate_commit_request(&request, &active_service_config()).unwrap();
        let mut plain = store(&request);
        assert!(!plain.to_string().contains("abandon"));

        request.owner_mnemonic_words = Some(words(MNEMONIC));
        validate_commit_request(&request, &active_service_config()).unwrap();
        assert!(!format!("{request:?}").contains("abandon"));
        let stored = store(&request);
        buckyos_api::validate_start_config_schema(&stored).unwrap();
        plain[START_CONFIG_OWNER_MNEMONIC] = json!(words(MNEMONIC));
        assert_eq!(stored, plain);

        for other in [
            "legal winner thank year wave sausage worth useful legal winner thank yellow",
            "abandon",
            "",
        ] {
            request.owner_mnemonic_words = Some(words(other));
            assert!(
                validate_commit_request(&request, &active_service_config()).is_err(),
                "{other}"
            );
        }
    }

    #[tokio::test]
    async fn activation_stores_the_hosting_and_not_the_token() {
        let root = tempfile::tempdir().unwrap();
        let paths = activation_paths(root.path());
        std::fs::create_dir_all(&paths.etc_dir).unwrap();
        let old = paths.roots.security_root.join("dns_credentials/old-domain.json");
        std::fs::create_dir_all(old.parent().unwrap()).unwrap();
        std::fs::write(&old, "{}").unwrap();

        let (prepared, signed, key) = signed_documents(Some("home.example.org"), topology()).await;
        let request = commit_request(&prepared, signed, &key, None);
        let commit = commit_of("dns_api", "acme", Some("home-example-org-1"));
        persist_activation(
            &paths,
            &FakeCertificates::default(),
            &request,
            &prepared.owner_document,
            &sn_zone_info(),
            Some(&commit),
        )
        .unwrap();

        let start_config =
            std::fs::read_to_string(paths.roots.security_root.join(START_CONFIG_FILE)).unwrap();
        assert!(!start_config.contains(TEST_TOKEN));
        let start_config: Value = serde_json::from_str(&start_config).unwrap();
        let hosting: DomainHosting =
            serde_json::from_value(start_config[START_CONFIG_DOMAIN_HOSTING].clone()).unwrap();
        assert_eq!(hosting, commit.hosting);
        hosting.validate().unwrap();
        let credential = paths
            .roots
            .security_root
            .join("dns_credentials/home-example-org-1.json");
        assert!(std::fs::read_to_string(&credential).unwrap().contains(TEST_TOKEN));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&credential).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        assert!(!old.exists());
        assert!(paths.etc_dir.join("node_identity.json").exists());
    }

    #[tokio::test]
    async fn a_failed_activation_keeps_no_credential_and_no_identity() {
        let root = tempfile::tempdir().unwrap();
        let paths = activation_paths(root.path());
        // etc is a file: nothing can be written into it.
        std::fs::write(root.path().join("etc"), "x").unwrap();
        let (prepared, signed, key) = signed_documents(Some("home.example.org"), topology()).await;
        let request = commit_request(&prepared, signed, &key, None);
        let commit = commit_of("dns_api", "acme", Some("home-example-org-1"));
        assert!(persist_activation(
            &paths,
            &FakeCertificates::default(),
            &request,
            &prepared.owner_document,
            &sn_zone_info(),
            Some(&commit),
        )
        .is_err());
        assert!(!paths
            .roots
            .security_root
            .join("dns_credentials/home-example-org-1.json")
            .exists());

        let root = tempfile::tempdir().unwrap();
        let paths = activation_paths(root.path());
        let certificates = FakeCertificates::failing();
        let mut commit = commit_of("dns_api", "upload", Some("home-example-org-1"));
        commit.certificates = Some(Vec::new());
        assert!(persist_activation(
            &paths,
            &certificates,
            &request,
            &prepared.owner_document,
            &sn_zone_info(),
            Some(&commit),
        )
        .is_err());
        assert!(!paths.roots.security_root.join(START_CONFIG_FILE).exists());
        assert!(!paths.etc_dir.join("node_identity.json").exists());
        assert!(!paths
            .roots
            .security_root
            .join("dns_credentials/home-example-org-1.json")
            .exists());
    }

    #[tokio::test]
    async fn activation_leaves_a_zone_state_the_next_boot_starts_from() {
        let root = tempfile::tempdir().unwrap();
        let paths = activation_paths(root.path());
        std::fs::create_dir_all(&paths.etc_dir).unwrap();
        let persist = |request: &CommitActiveReq| {
            persist_activation(
                &paths,
                &FakeCertificates::default(),
                request,
                &request.owner_document,
                &sn_zone_info(),
                None,
            )
            .unwrap()
        };
        let boot = || {
            let identity =
                load_local_node_identity_config(&paths.etc_dir.join("node_identity.json")).unwrap();
            crate::zone_boot_resolve::boot_without_network(&paths.etc_dir, &identity)
        };
        let (prepared, signed, key) = signed_documents(Some("home.example.org"), topology()).await;
        let first = signed.zone_document.clone();
        persist(&commit_request(&prepared, signed, &key, None));
        assert_eq!(boot().unwrap(), first);

        let (prepared, signed, key) =
            signed_documents(Some("home.example.org"), topology_of("wan_dyn", &[])).await;
        let second = signed.zone_document.clone();
        assert_ne!(first, second);
        persist(&commit_request(&prepared, signed, &key, None));
        assert_eq!(boot().unwrap(), second);
    }
}
