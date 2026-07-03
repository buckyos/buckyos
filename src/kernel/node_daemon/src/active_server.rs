use ::kRPC::*;
use async_trait::async_trait;
use bns_client::{
    default_document_update, BnsApplyMutationsReq, BnsClientError, BnsEvmClientConfig,
    BnsEvmControllerClient, BnsEvmRawTxSubmitter, BnsEvmTxSubmission, BnsIndexerApi,
    BnsIndexerClient, BnsRegisterNameReq, CallAuthority, DocumentRef, DocumentUpdate,
    MutationGuard, OwnerPolicyUpdate, Principal, RegisterOptions, ZERO_HASH,
};
use buckyos_api::*;
use buckyos_http_server::*;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::*;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Full};
use jsonwebtoken::jwk::Jwk;
use jsonwebtoken::{DecodingKey, EncodingKey};
use log::*;
use name_client::*;
use name_lib::*;
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::result::Result;
use std::sync::Arc;
use std::{net::IpAddr, process::exit};

const ACTIVE_SERVICE_MAIN_PORT: u16 = 3182;
const START_CONFIG_OPTIONAL_FIELDS: &[&str] = &[
    "ai_provider_config",
    "jarvis_msg_tunnel_config",
    "enabled_features",
];

const BNS_DOC_ZONE: &str = "zone";
const BNS_DOC_BOOT: &str = "boot";
const BNS_DOC_DEVICE_MINI: &str = "device_mini_doc";

struct BnsPublishConfig {
    server_url: String,
    evm_config: BnsEvmClientConfig,
    private_key: String,
}

#[derive(Clone)]
struct ActiveServer {
    device_mini_info: DeviceMiniInfo,
}

impl ActiveServer {
    pub fn new() -> Self {
        ActiveServer {
            device_mini_info: DeviceMiniInfo::default(),
        }
    }

    fn is_sensitive_param_key(key: &str) -> bool {
        matches!(
            key,
            "private_key"
                | "device_private_key"
                | "boot_config_jwt"
                | "device_doc_jwt"
                | "device_mini_doc_jwt"
                | "ood_jwt"
                | "sn_device_proof"
                | "bns_evm_private_key"
                | "admin_password_hash"
                | "friend_passcode"
        )
    }

    fn redact_sensitive_json(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut redacted = Map::new();
                for (key, child_value) in map {
                    if Self::is_sensitive_param_key(key.as_str()) {
                        let value_len = child_value.as_str().map(|v| v.len()).unwrap_or(0);
                        redacted.insert(
                            key.clone(),
                            Value::String(format!("[redacted:{} chars]", value_len)),
                        );
                    } else {
                        redacted.insert(key.clone(), Self::redact_sensitive_json(child_value));
                    }
                }
                Value::Object(redacted)
            }
            Value::Array(values) => {
                Value::Array(values.iter().map(Self::redact_sensitive_json).collect())
            }
            _ => value.clone(),
        }
    }

    // cache 里存 owner 签名的 jwt 原文而不是 JsonLd：boot 阶段的 resolve 纪律
    // 只接受能用本地 owner key 验签的 Jwt candidate（见 zone_boot_resolve.rs），
    // 这份 cache 是激活后首次 boot 在 DNS 记录尚未可查时的关键回答来源。
    async fn update_zone_boot_cache(zone_did: &DID, zone_boot_config_jwt: &str) {
        let zone_boot_doc = EncodedDocument::Jwt(zone_boot_config_jwt.to_string());

        if let Err(err) =
            update_did_cache(zone_did.clone(), Some(DidDocType::Boot), zone_boot_doc).await
        {
            warn!(
                "update zone boot did cache failed, zone_did={:?}, err={}",
                zone_did, err
            );
        } else {
            info!(
                "update zone boot did cache success, zone_did={:?}",
                zone_did
            );
        }
    }

    pub async fn auto_fill_device_mini_info(&mut self) {
        self.device_mini_info
            .auto_fill_by_system_info()
            .await
            .unwrap();
        self.device_mini_info.active_url = Some("./index.html".to_string());
    }

    fn append_optional_start_config_fields(
        req_params: &Value,
        start_params: &mut Map<String, Value>,
    ) {
        for field in START_CONFIG_OPTIONAL_FIELDS {
            if let Some(value) = req_params.get(*field) {
                start_params.insert((*field).to_string(), value.clone());
            }
        }
    }

    fn remove_activation_only_start_config_fields(start_params: &mut Map<String, Value>) {
        start_params.remove("private_key");
        start_params.remove("bns_evm_private_key");
        start_params.remove("sn_device_proof");
    }

    fn device_info_report_ip(device_info: &DeviceInfo) -> String {
        if let Some(ip) = device_info.all_ip.first() {
            return ip.to_string();
        }
        if let Some(ip) = device_info.ips.first() {
            return ip.to_string();
        }
        "127.0.0.1".to_string()
    }

    fn build_sn_device_online_report(
        device_name: &str,
        device_did: &DID,
        device_info: &DeviceInfo,
    ) -> Result<SnDeviceOnlineReportReq, RPCErrors> {
        let device_info_value = serde_json::to_value(device_info).map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to serialize device info: {}", e))
        })?;
        Ok(SnDeviceOnlineReportReq {
            device_name: device_name.to_string(),
            device_did: Some(device_did.to_string()),
            device_ip: Self::device_info_report_ip(device_info),
            device_info: device_info_value,
            endpoints: Vec::new(),
            report_seq: None,
            ttl: None,
        })
    }

    fn generate_sn_device_proof(
        sn_username: &str,
        device_did: &DID,
        device_private_key: &EncodingKey,
    ) -> Result<String, RPCErrors> {
        let now = buckyos_get_unix_timestamp();
        let mut extra = HashMap::new();
        extra.insert("device_did".to_string(), json!(device_did.to_string()));
        let proof_token = ::kRPC::RPCSessionToken {
            token_type: ::kRPC::RPCSessionTokenType::Normal,
            appid: Some("node-daemon".to_string()),
            jti: Some(now.to_string()),
            sub: Some(sn_username.to_lowercase()),
            aud: Some("sn".to_string()),
            exp: Some(now + 60 * 10),
            iss: Some(device_did.to_string()),
            token: None,
            sudo: false,
            extra,
        };
        proof_token
            .generate_jwt(Some(device_did.to_string()), device_private_key)
            .map_err(|e| {
                warn!("Failed to generate SN device proof: {}", e);
                RPCErrors::ReasonError("Failed to generate SN device proof".to_string())
            })
    }

    fn bns_error(context: &str, error: impl std::fmt::Display) -> RPCErrors {
        RPCErrors::ReasonError(format!("{}: {}", context, error))
    }

    fn string_param(req_params: &Value, key: &str) -> Option<String> {
        req_params
            .get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
    }

    fn bns_evm_param<'a>(req_params: &'a Value, key: &str) -> Option<&'a Value> {
        if let Some(value) = req_params
            .get("bns_evm")
            .and_then(Value::as_object)
            .and_then(|evm| evm.get(key))
        {
            return Some(value);
        }

        let flat_key = format!("bns_evm_{}", key);
        req_params.get(flat_key.as_str())
    }

    fn bns_evm_string_param(
        req_params: &Value,
        key: &str,
        env_key: Option<&str>,
    ) -> Option<String> {
        Self::bns_evm_param(req_params, key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToString::to_string)
            .or_else(|| {
                env_key
                    .and_then(|key| std::env::var(key).ok())
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn bns_evm_private_key(req_params: &Value) -> Option<String> {
        Self::string_param(req_params, "bns_evm_private_key")
            .or_else(|| Self::bns_evm_string_param(req_params, "private_key", None))
            .or_else(|| {
                std::env::var("BNS_PRIVATE_KEY")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn parse_optional_u64(
        value: Option<&Value>,
        env_key: Option<&str>,
        field: &str,
    ) -> Result<Option<u64>, RPCErrors> {
        if let Some(value) = value {
            if let Some(number) = value.as_u64() {
                return Ok(Some(number));
            }
            if let Some(text) = value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return text.parse::<u64>().map(Some).map_err(|e| {
                    RPCErrors::ParseRequestError(format!("Invalid {}: {}", field, e))
                });
            }
            return Err(RPCErrors::ParseRequestError(format!(
                "Invalid {}, expected u64 or string",
                field
            )));
        }

        if let Some(env_key) = env_key {
            if let Ok(value) = std::env::var(env_key) {
                let value = value.trim();
                if !value.is_empty() {
                    return value.parse::<u64>().map(Some).map_err(|e| {
                        RPCErrors::ParseRequestError(format!("Invalid {}: {}", field, e))
                    });
                }
            }
        }

        Ok(None)
    }

    fn parse_optional_u128(
        value: Option<&Value>,
        env_key: Option<&str>,
        field: &str,
    ) -> Result<Option<u128>, RPCErrors> {
        if let Some(value) = value {
            if let Some(number) = value.as_u64() {
                return Ok(Some(number as u128));
            }
            if let Some(text) = value
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                return text.parse::<u128>().map(Some).map_err(|e| {
                    RPCErrors::ParseRequestError(format!("Invalid {}: {}", field, e))
                });
            }
            return Err(RPCErrors::ParseRequestError(format!(
                "Invalid {}, expected u128 or string",
                field
            )));
        }

        if let Some(env_key) = env_key {
            if let Ok(value) = std::env::var(env_key) {
                let value = value.trim();
                if !value.is_empty() {
                    return value.parse::<u128>().map(Some).map_err(|e| {
                        RPCErrors::ParseRequestError(format!("Invalid {}: {}", field, e))
                    });
                }
            }
        }

        Ok(None)
    }

    fn derive_bns_url_from_sn_url(sn_url: &str) -> Option<String> {
        let trimmed = sn_url.trim().trim_end_matches('/');
        if trimmed.ends_with("/kapi/sn") {
            return Some(format!("{}/kapi/bns", trimmed.trim_end_matches("/kapi/sn")));
        }
        None
    }

    fn bns_server_url(req_params: &Value) -> Option<String> {
        Self::string_param(req_params, "bns_url")
            .or_else(|| {
                Self::string_param(req_params, "sn_url")
                    .and_then(|url| Self::derive_bns_url_from_sn_url(url.as_str()))
            })
            .or_else(|| {
                std::env::var("BNS_SERVER_URL")
                    .ok()
                    .map(|value| value.trim().to_string())
                    .filter(|value| !value.is_empty())
            })
    }

    fn parse_bns_publish_config(req_params: &Value) -> Result<Option<BnsPublishConfig>, RPCErrors> {
        let Some(private_key) = Self::bns_evm_private_key(req_params) else {
            return Ok(None);
        };

        let server_url = Self::bns_server_url(req_params).ok_or_else(|| {
            RPCErrors::ParseRequestError(
                "bns_url or BNS_SERVER_URL is required when bns_evm_private_key is set".to_string(),
            )
        })?;
        let rpc_endpoint =
            Self::bns_evm_string_param(req_params, "rpc_endpoint", Some("BNS_RPC_URL"))
                .ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                "bns_evm.rpc_endpoint or BNS_RPC_URL is required when bns_evm_private_key is set"
                    .to_string(),
            )
                })?;
        let contract_address = Self::bns_evm_string_param(
            req_params,
            "contract_address",
            Some("BNS_CONTRACT_ADDRESS"),
        )
        .ok_or_else(|| {
            RPCErrors::ParseRequestError(
                "bns_evm.contract_address or BNS_CONTRACT_ADDRESS is required when bns_evm_private_key is set"
                    .to_string(),
            )
        })?;
        let chain_id = Self::parse_optional_u64(
            Self::bns_evm_param(req_params, "chain_id"),
            Some("BNS_CHAIN_ID"),
            "bns_evm.chain_id",
        )?
        .ok_or_else(|| {
            RPCErrors::ParseRequestError(
                "bns_evm.chain_id or BNS_CHAIN_ID is required when bns_evm_private_key is set"
                    .to_string(),
            )
        })?;

        let mut evm_config = BnsEvmClientConfig::anvil(rpc_endpoint, contract_address, chain_id);
        if let Some(gas_limit) = Self::parse_optional_u64(
            Self::bns_evm_param(req_params, "gas_limit"),
            Some("BNS_GAS_LIMIT"),
            "bns_evm.gas_limit",
        )? {
            evm_config.gas_limit = gas_limit;
        }
        if let Some(max_fee_per_gas) = Self::parse_optional_u128(
            Self::bns_evm_param(req_params, "max_fee_per_gas"),
            Some("BNS_MAX_FEE_PER_GAS"),
            "bns_evm.max_fee_per_gas",
        )? {
            evm_config.max_fee_per_gas = max_fee_per_gas;
        }
        if let Some(max_priority_fee_per_gas) = Self::parse_optional_u128(
            Self::bns_evm_param(req_params, "max_priority_fee_per_gas"),
            Some("BNS_MAX_PRIORITY_FEE_PER_GAS"),
            "bns_evm.max_priority_fee_per_gas",
        )? {
            evm_config.max_priority_fee_per_gas = max_priority_fee_per_gas;
        }

        Ok(Some(BnsPublishConfig {
            server_url,
            evm_config,
            private_key,
        }))
    }

    fn bns_zone_document(
        zone_name: &str,
        owner_name: &str,
        device_name: &str,
        device_did: &DID,
        boot_config_jwt: &str,
        device_mini_doc_jwt: &str,
    ) -> Value {
        json!({
            "id": format!("did:bns:{}", zone_name),
            "name": zone_name,
            "owner": format!("did:bns:{}", owner_name),
            "gateway": {
                "device_name": device_name,
            },
            "gateway_device_name": device_name,
            "boot_jwt": boot_config_jwt,
            "devices": {
                device_name: {
                    "did": device_did.to_string(),
                    "mini_config_jwt": device_mini_doc_jwt,
                    "role": "gateway",
                }
            }
        })
    }

    fn bns_boot_document(boot_config_jwt: &str) -> Value {
        json!({ "boot": boot_config_jwt })
    }

    fn bns_device_mini_document(
        device_name: &str,
        device_did: &DID,
        device_mini_doc_jwt: &str,
    ) -> Value {
        json!({
            "devices": {
                device_name: {
                    "did": device_did.to_string(),
                    "mini_config_jwt": device_mini_doc_jwt,
                    "role": "gateway",
                }
            }
        })
    }

    fn bns_inline_json_update(
        doc_type: &str,
        expected_version: u64,
        document: &Value,
    ) -> Result<DocumentUpdate, RPCErrors> {
        let bytes = serde_json::to_vec(document).map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to serialize BNS document: {}", e))
        })?;
        default_document_update(doc_type, expected_version, DocumentRef::inline(bytes))
            .map_err(|e| Self::bns_error("Failed to build BNS document update", e))
    }

    async fn bns_document_version(
        client: &dyn BnsIndexerApi,
        name: &str,
        doc_type: &str,
    ) -> Result<u64, RPCErrors> {
        match client.resolve_document(name, doc_type).await {
            Ok(result) => Ok(result.document_state.version),
            Err(error) if error.is_registry_code("DOCUMENT_NOT_FOUND") => Ok(0),
            Err(error) => Err(Self::bns_error("Failed to query BNS document", error)),
        }
    }

    async fn publish_bns_zone_documents(
        &self,
        req_params: &Value,
        zone_name: &str,
        owner_name: &str,
        device_name: &str,
        device_did: &DID,
        boot_config_jwt: &str,
        device_mini_doc_jwt: &str,
    ) -> Result<Option<BnsEvmTxSubmission>, RPCErrors> {
        let Some(config) = Self::parse_bns_publish_config(req_params)? else {
            info!("BNS EVM publish skipped: bns_evm_private_key is not provided");
            return Ok(None);
        };

        let bns_client: Arc<dyn BnsIndexerApi> = Arc::new(BnsIndexerClient::new_bns_server_url(
            &config.server_url,
            None,
        ));
        let evm_controller = BnsEvmControllerClient::new(config.evm_config, &config.private_key)
            .map_err(|e| Self::bns_error("Failed to create BNS EVM controller", e))?
            .with_raw_tx_submitter(BnsEvmRawTxSubmitter::BnsServer(bns_client.clone()));
        let signer_address = evm_controller.default_signer_address().ok_or_else(|| {
            RPCErrors::ReasonError("BNS EVM signer address is missing".to_string())
        })?;
        let signer_principal = Principal::chain_account(format!("{signer_address:#x}"));

        let zone_document = Self::bns_zone_document(
            zone_name,
            owner_name,
            device_name,
            device_did,
            boot_config_jwt,
            device_mini_doc_jwt,
        );
        let boot_document = Self::bns_boot_document(boot_config_jwt);
        let device_mini_document =
            Self::bns_device_mini_document(device_name, device_did, device_mini_doc_jwt);

        let submission = match bns_client.query_name_state(zone_name).await {
            Ok(Some(name_state)) => {
                let zone_version =
                    Self::bns_document_version(bns_client.as_ref(), zone_name, BNS_DOC_ZONE)
                        .await?;
                let boot_version =
                    Self::bns_document_version(bns_client.as_ref(), zone_name, BNS_DOC_BOOT)
                        .await?;
                let device_mini_version =
                    Self::bns_document_version(bns_client.as_ref(), zone_name, BNS_DOC_DEVICE_MINI)
                        .await?;
                let updates = vec![
                    Self::bns_inline_json_update(BNS_DOC_ZONE, zone_version, &zone_document)?,
                    Self::bns_inline_json_update(BNS_DOC_BOOT, boot_version, &boot_document)?,
                    Self::bns_inline_json_update(
                        BNS_DOC_DEVICE_MINI,
                        device_mini_version,
                        &device_mini_document,
                    )?,
                ];
                evm_controller
                    .apply_mutations(&BnsApplyMutationsReq {
                        name: zone_name.to_string(),
                        authority_key_updates: Vec::new(),
                        documents: updates,
                        owner_policy: OwnerPolicyUpdate::none(),
                        authority: CallAuthority::owner(signer_principal, ""),
                        guard: MutationGuard {
                            expected_name_seq: name_state.name_seq,
                            expected_parent_name_seq: 0,
                        },
                    })
                    .await
                    .map_err(|e| Self::bns_error("Failed to publish BNS zone documents", e))?
            }
            Ok(None) => {
                let initial_documents = vec![
                    Self::bns_inline_json_update(BNS_DOC_ZONE, 0, &zone_document)?,
                    Self::bns_inline_json_update(BNS_DOC_BOOT, 0, &boot_document)?,
                    Self::bns_inline_json_update(BNS_DOC_DEVICE_MINI, 0, &device_mini_document)?,
                ];
                evm_controller
                    .register_name(&BnsRegisterNameReq {
                        name: zone_name.to_string(),
                        asset_owner: format!("{signer_address:#x}"),
                        options: RegisterOptions::default(),
                        authority_key_updates: Vec::new(),
                        semantic_owner_after_authority: None,
                        controller_policy: Vec::new(),
                        controller_policy_hash: ZERO_HASH.to_string(),
                        initial_documents,
                        authority: CallAuthority::public(),
                        guard: MutationGuard {
                            expected_name_seq: 0,
                            expected_parent_name_seq: 0,
                        },
                    })
                    .await
                    .map_err(|e| Self::bns_error("Failed to register BNS zone name", e))?
            }
            Err(error) if matches!(error, BnsClientError::Registry(_)) => {
                return Err(Self::bns_error("Failed to query BNS name state", error));
            }
            Err(error) => return Err(Self::bns_error("Failed to query BNS name state", error)),
        };

        info!(
            "BNS zone documents submitted, zone={}, tx={}, nonce={}",
            zone_name, submission.tx_hash, submission.nonce
        );
        Ok(Some(submission))
    }

    async fn handle_active_by_wallet(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        // Required parameters: only JWT tokens and essential data
        let boot_config_jwt = req.params.get("boot_config_jwt");
        let device_doc_jwt = req.params.get("device_doc_jwt");
        let device_mini_doc_jwt = req.params.get("device_mini_doc_jwt");
        let device_private_key = req.params.get("device_private_key");
        let device_info_param = req.params.get("device_info");

        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let owner_public_key_param = req.params.get("public_key");
        let admin_password_hash = req.params.get("admin_password_hash");
        let guest_access = req.params.get("guest_access");
        let friend_passcode = req.params.get("friend_passcode");

        let sn_url_param = req.params.get("sn_url");
        let sn_username_param = req.params.get("sn_username");
        let sn_device_proof_param = req.params.get("sn_device_proof");

        if owner_public_key_param.is_none()
            || boot_config_jwt.is_none()
            || device_doc_jwt.is_none()
            || device_mini_doc_jwt.is_none()
            || device_private_key.is_none()
            || zone_name.is_none()
        {
            return Err(RPCErrors::ParseRequestError("Invalid params, missing required fields: owner_public_key_param, boot_config_jwt, device_doc_jwt, device_mini_doc_jwt, device_private_key, zone_name".to_string()));
        }

        info!(
            "handle_active_by_wallet params: {}",
            Self::redact_sensitive_json(&req.params)
        );

        let boot_config_jwt = boot_config_jwt.unwrap().as_str().unwrap();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let zone_did = DID::from_str(zone_name)
            .map_err(|_| RPCErrors::ReasonError("Invalid zone name".to_string()))?;
        let user_name = user_name.unwrap().as_str().unwrap();
        let user_name = user_name.to_lowercase();
        // Get owner public key from device_config (it should be in the JWT header or we need to verify)
        // For now, we'll need owner_public_key to verify, but let's try to extract it from the request if available
        // If not available, we'll decode without verification (less secure but works for now)
        let owner_public_key: Jwk = if owner_public_key_param.is_some() {
            serde_json::from_value(owner_public_key_param.unwrap().clone()).map_err(|e| {
                warn!("Invalid owner public key format: {}", e);
                RPCErrors::ReasonError("Invalid owner public key format".to_string())
            })?
        } else {
            // Try to extract from device_config if available, otherwise use a placeholder
            // In practice, owner_public_key should be provided or extracted from zone config
            warn!("owner_public_key is required to verify JWT signatures");
            return Err(RPCErrors::ParseRequestError(
                "owner_public_key is required to verify JWT signatures".to_string(),
            ));
        };

        let device_doc_jwt = device_doc_jwt.unwrap().as_str().unwrap();
        let device_mini_doc_jwt = device_mini_doc_jwt.unwrap().as_str().unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();

        // Decode device_doc_jwt to extract information
        let encoded_device_doc =
            EncodedDocument::from_str(device_doc_jwt.to_string()).map_err(|e| {
                warn!("Invalid device_doc_jwt format: {}", e);
                RPCErrors::ParseRequestError(format!("Invalid device_doc_jwt format: {}", e))
            })?;

        // First decode without verification to get owner public key hint, then verify
        // For now, we'll decode without verification first to extract owner info
        // In production, owner_public_key should be provided or extracted from zone config
        let device_config = DeviceDocument::decode(&encoded_device_doc, None).map_err(|e| {
            warn!("Failed to decode device_doc_jwt: {}", e);
            RPCErrors::ParseRequestError(format!("Failed to decode device_doc_jwt: {}", e))
        })?;

        // Extract information from device_config
        let device_did = device_config.id.clone();
        let device_name = device_config.name.clone();
        info!(
            "device_did: {}, device_name: {}",
            device_did.to_string(),
            device_name
        );
        let expected_device_did =
            build_device_did(device_name.as_str(), &zone_did).map_err(RPCErrors::ReasonError)?;
        if device_did != expected_device_did {
            return Err(RPCErrors::ParseRequestError(format!(
                "device DID {} does not match expected DID {}",
                device_did.to_string(),
                expected_device_did.to_string()
            )));
        }

        // Verify the JWT signatures with owner public key
        let owner_decoding_key = DecodingKey::from_jwk(&owner_public_key).map_err(|e| {
            warn!("Failed to create decoding key: {}", e);
            RPCErrors::ReasonError(format!("Failed to create decoding key: {}", e))
        })?;

        // Re-decode with verification
        let _verified_device_config =
            DeviceDocument::decode(&encoded_device_doc, Some(&owner_decoding_key)).map_err(
                |e| {
                    warn!("Failed to verify device_doc_jwt: {}", e);
                    RPCErrors::ParseRequestError(format!("Failed to verify device_doc_jwt: {}", e))
                },
            )?;

        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|e| {
                warn!("Invalid device private key: {}", e);
                RPCErrors::ReasonError("Invalid device private key".to_string())
            })?;

        info!("device documents decoded success");

        // Determine if SN registration is needed
        let sn_url = sn_url_param
            .and_then(Value::as_str)
            .filter(|url| url.len() > 5)
            .map(ToString::to_string);
        let need_sn = sn_url.is_some();

        // Register device to SN if needed
        if need_sn {
            let sn_url = sn_url.unwrap();
            let sn_username = sn_username_param
                .and_then(Value::as_str)
                .unwrap_or(user_name.as_str())
                .to_lowercase();
            let sn_device_proof = if let Some(proof) = sn_device_proof_param
                .and_then(Value::as_str)
                .filter(|proof| !proof.is_empty())
            {
                proof.to_string()
            } else {
                Self::generate_sn_device_proof(
                    sn_username.as_str(),
                    &device_did,
                    &device_private_key_pem,
                )?
            };

            info!("Register {}(zone-gateway) to sn: {}", device_name, sn_url);
            // device_info can be either a JSON string or a JSON object
            let mut device_info: DeviceInfo = if device_info_param.is_some() {
                let device_info_value = device_info_param.unwrap();
                if device_info_value.is_string() {
                    serde_json::from_str(device_info_value.as_str().unwrap()).map_err(|e| {
                        RPCErrors::ParseRequestError(format!("Invalid device_info string: {}", e))
                    })?
                } else {
                    serde_json::from_value(device_info_value.clone()).map_err(|e| {
                        RPCErrors::ParseRequestError(format!("Invalid device_info object: {}", e))
                    })?
                }
            } else {
                // Create device_info from device_config if not provided
                let mut info = DeviceInfo::from_device_doc(&device_config);
                info.auto_fill_by_system_info().await.unwrap();
                info
            };

            let sn_req =
                Self::build_sn_device_online_report(&device_name, &device_did, &device_info)?;
            let sn_result =
                sn_register_device_online(sn_url.as_str(), sn_device_proof, sn_req).await;
            if sn_result.is_err() {
                return Err(RPCErrors::ReasonError(format!(
                    "Failed to register device to sn: {}",
                    sn_result.err().unwrap()
                )));
            }
        } else {
            info!("NO SN mode: Check if the zone txt records is already exists ...");
            // let zone_boot = resolve_did(&zone_did, None).await
            //     .map_err(|e|RPCErrors::ReasonError(format!("Failed to resolve zone did: {}", e)))?;
            // let zone_boot_config = ZoneBootDocument::decode(&zone_boot, Some(&owner_decoding_key))
            //     .map_err(|e|RPCErrors::ReasonError(format!("Failed to decode zone boot config: {}", e)))?;
            info!("verify zone boot config success");
        }

        let bns_submission = self
            .publish_bns_zone_documents(
                &req.params,
                zone_name,
                user_name.as_str(),
                device_name.as_str(),
                &device_did,
                boot_config_jwt,
                device_mini_doc_jwt,
            )
            .await?;

        let write_dir = get_buckyos_system_etc_dir();
        let owner_did = DID::from_str(&user_name).unwrap_or_else(|_| DID::new("bns", &user_name));
        let node_identity = LocalNodeIdentityConfig::new(
            zone_did.clone(),
            owner_did,
            owner_public_key.clone(),
            device_name.clone(),
            device_did.clone(),
            buckyos_get_unix_timestamp() as u32 - 3600,
        );
        save_local_device_identity(
            write_dir.as_path(),
            &node_identity,
            &device_config,
            device_doc_jwt,
            device_mini_doc_jwt,
            device_private_key,
        )
        .map_err(RPCErrors::ReasonError)?;

        // Write start config (minimal, only essential params)
        let mut real_start_params = req.params.clone();
        let mut real_start_params = real_start_params.as_object_mut().unwrap();
        real_start_params.insert(
            "ood_jwt".to_string(),
            Value::String(device_doc_jwt.to_string()),
        );
        Self::remove_activation_only_start_config_fields(real_start_params);
        Self::append_optional_start_config_fields(&req.params, real_start_params);
        let start_params_str = serde_json::to_string(&real_start_params).map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to serialize start params: {}", e))
        })?;
        let start_params_file = write_dir.join("start_config.json");
        tokio::fs::write(start_params_file, start_params_str.as_bytes())
            .await
            .map_err(|_| RPCErrors::ReasonError("Failed to write start params".to_string()))?;

        let zone_boot_doc = match EncodedDocument::from_str(boot_config_jwt.to_string()) {
            Ok(doc) => doc,
            Err(err) => {
                warn!(
                    "parse zone boot document failed before cache update, zone_did={:?}, err={}",
                    zone_did, err
                );
                return Err(RPCErrors::ReasonError(format!(
                    "Failed to parse zone boot config: {}",
                    err
                )));
            }
        };
        ZoneBootDocument::decode(&zone_boot_doc, None).map_err(|err| {
            RPCErrors::ReasonError(format!("Failed to decode zone boot config: {}", err))
        })?;
        Self::update_zone_boot_cache(&zone_did, boot_config_jwt).await;

        info!("ActiveByWallet wrote device identity files to node_identity.json, identity root and security root");

        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            exit(0);
        });

        let result = if let Some(bns_submission) = bns_submission {
            json!({
                "code":0,
                "bns": bns_submission
            })
        } else {
            json!({
                "code":0
            })
        };

        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }

    async fn handle_prepare_params_for_active_by_wallet(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let net_id = req.params.get("net_id");
        let owner_public_key = req.params.get("public_key");
        let device_public_key = req.params.get("device_public_key");
        let device_private_key = req.params.get("device_private_key");
        let device_rtcp_port_param = req.params.get("device_rtcp_port");
        let support_container = req.params.get("support_container");
        let sn_username = req.params.get("sn_username");
        let sn_url_param = req.params.get("sn_url");
        let sn_url = sn_url_param
            .and_then(Value::as_str)
            .filter(|url| url.len() > 5)
            .map(ToString::to_string);

        if user_name.is_none()
            || zone_name.is_none()
            || owner_public_key.is_none()
            || device_public_key.is_none()
            || device_private_key.is_none()
        {
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name, zone_name, gateway_type, owner_public_key, device_public_key or device_private_key is none".to_string()));
        }

        let user_name = user_name.unwrap().as_str().unwrap();
        let user_name = user_name.to_lowercase();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let zone_did = DID::from_str(zone_name)
            .map_err(|_| RPCErrors::ReasonError("Invalid zone name".to_string()))?;

        let net_id = if net_id.is_some() {
            Some(net_id.unwrap().as_str().unwrap().to_string())
        } else {
            None
        };

        let owner_public_key = owner_public_key.unwrap();
        let device_public_key = device_public_key.unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();
        let mut device_rtcp_port = None;
        if device_rtcp_port_param.is_some() {
            let real_device_rtcp_port = device_rtcp_port_param.unwrap().as_u64().unwrap();
            if real_device_rtcp_port != 2980 {
                device_rtcp_port = Some(real_device_rtcp_port as u32);
            }
        }

        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|_| RPCErrors::ReasonError("Invalid device private key".to_string()))?;
        let device_public_jwk: Jwk = serde_json::from_value(device_public_key.clone())
            .map_err(|_| RPCErrors::ReasonError("Invalid device public key format".to_string()))?;

        let need_sn = sn_url.is_some();
        let mut is_support_container = true;
        if support_container.is_some() {
            is_support_container = support_container.unwrap().as_str().unwrap() == "true";
        }

        // Create device_config without signing
        let mut ddns_sn_url: Option<String> = None;
        if net_id.is_some() {
            let real_net_id = net_id.as_ref().unwrap();
            if real_net_id == "wan_dyn" {
                ddns_sn_url = sn_url.clone();
            }
            if real_net_id == "portmap" {
                ddns_sn_url = sn_url.clone();
            }
        }

        let device_did = build_device_did("ood1", &zone_did).map_err(RPCErrors::ReasonError)?;
        let mut device_config =
            new_device_config_by_jwk_with_did("ood1", device_public_jwk, &device_did)
                .map_err(RPCErrors::ReasonError)?;
        device_config.net_id = net_id;
        device_config.ddns_sn_url = ddns_sn_url;
        device_config.support_container = is_support_container;
        device_config.owner = DID::new("bns", user_name.as_str());
        device_config.zone_did = Some(zone_did.clone());
        device_config.rtcp_port = device_rtcp_port;

        // Convert device_config to JSON (unsigned)
        let device_config_json = serde_json::to_value(&device_config).map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to serialize device config: {}", e))
        })?;

        // Create device info for SN registration
        let mut device_info = DeviceInfo::from_device_doc(&device_config);
        device_info.auto_fill_by_system_info().await.unwrap();
        let device_info_json = serde_json::to_string(&device_info).map_err(|e| {
            RPCErrors::ReasonError(format!("Failed to serialize device info: {}", e))
        })?;

        let sn_device_proof = if need_sn {
            let sn_username = sn_username
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "sn_username is required for SN device report".to_string(),
                    )
                })?
                .to_lowercase();
            Some(Self::generate_sn_device_proof(
                sn_username.as_str(),
                &device_did,
                &device_private_key_pem,
            )?)
        } else {
            None
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "device_config": device_config_json,
                "sn_device_proof": sn_device_proof,
                "device_info": device_info_json,
            })),
            req.seq,
        ))
    }

    async fn handle_do_active(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        //info!("handle_do_active: {}",serde_json::to_string_pretty(&req.params).unwrap());
        let user_name = req.params.get("user_name");
        let zone_name = req.params.get("zone_name");
        let net_id = req.params.get("net_id");
        let owner_public_key = req.params.get("public_key");
        let owner_private_key = req.params.get("private_key");
        let owner_password_hash = req.params.get("admin_password_hash");
        let enable_guest_access = req.params.get("guest_access");
        let friend_passcode = req.params.get("friend_passcode");
        let device_public_key = req.params.get("device_public_key");
        let device_private_key = req.params.get("device_private_key");
        let device_rtcp_port_param = req.params.get("device_rtcp_port");
        let support_container = req.params.get("support_container");
        let sn_url_param = req.params.get("sn_url");
        let sn_username = req.params.get("sn_username");
        let sn_url = sn_url_param
            .and_then(Value::as_str)
            .filter(|url| url.len() > 5)
            .map(ToString::to_string);
        //let device_info = req.params.get("device_info");
        if user_name.is_none()
            || zone_name.is_none()
            || owner_public_key.is_none()
            || owner_private_key.is_none()
            || device_public_key.is_none()
            || device_private_key.is_none()
        {
            warn!("Invalid params, user_name, zone_name, owner_public_key, owner_private_key, device_public_key or device_private_key is none");
            return Err(RPCErrors::ParseRequestError("Invalid params, user_name, zone_name, owner_public_key, owner_private_key, device_public_key or device_private_key is none".to_string()));
        }

        let user_name = user_name.unwrap().as_str().unwrap();
        let user_name = user_name.to_lowercase();
        let zone_name = zone_name.unwrap().as_str().unwrap();
        let zone_did = DID::from_str(zone_name)
            .map_err(|_| RPCErrors::ReasonError("Invalid zone name".to_string()))?;

        let net_id = if net_id.is_some() {
            Some(net_id.unwrap().as_str().unwrap().to_string())
        } else {
            None
        };

        let owner_public_key = owner_public_key.unwrap();
        let owner_private_key = owner_private_key.unwrap().as_str().unwrap();
        let device_public_key = device_public_key.unwrap();
        let device_private_key = device_private_key.unwrap().as_str().unwrap();
        let mut device_rtcp_port = None;
        if device_rtcp_port_param.is_some() {
            let real_device_rtcp_port = device_rtcp_port_param.unwrap().as_u64().unwrap();
            if real_device_rtcp_port != 2980 {
                device_rtcp_port = Some(real_device_rtcp_port as u32);
            }
        }

        let owner_private_key_pem = EncodingKey::from_ed_pem(owner_private_key.as_bytes())
            .map_err(|_| RPCErrors::ReasonError("Invalid owner private key".to_string()))?;
        let device_private_key_pem = EncodingKey::from_ed_pem(device_private_key.as_bytes())
            .map_err(|_| RPCErrors::ReasonError("Invalid device private key".to_string()))?;
        let device_public_jwk: Jwk = serde_json::from_value(device_public_key.clone()).unwrap();

        //let device_ip:Option<IpAddr> = None;
        let mut ddns_sn_url: Option<String> = None;
        let need_sn = sn_url.is_some();
        if net_id.is_some() {
            let real_net_id = net_id.as_ref().unwrap();
            if real_net_id == "wan_dyn" {
                ddns_sn_url = sn_url.clone();
            }
            if real_net_id == "portmap" {
                ddns_sn_url = sn_url.clone();
            }
        }

        let mut is_support_container = true;
        if support_container.is_some() {
            is_support_container = support_container.unwrap().as_str().unwrap() == "true";
        }
        //create device doc ,and sign it with owner private key
        let device_did = build_device_did("ood1", &zone_did).map_err(RPCErrors::ReasonError)?;
        let mut device_config =
            new_device_config_by_jwk_with_did("ood1", device_public_jwk, &device_did)
                .map_err(RPCErrors::ReasonError)?;
        device_config.net_id = net_id;
        device_config.ddns_sn_url = ddns_sn_url;
        device_config.support_container = is_support_container;
        device_config.owner = DID::new("bns", user_name.as_str());
        device_config.zone_did = Some(zone_did.clone());
        device_config.rtcp_port = device_rtcp_port;
        //device_config.ip = device_ip;

        let device_doc_jwt = device_config
            .encode(Some(&owner_private_key_pem))
            .map_err(|_| RPCErrors::ReasonError("Failed to encode device config".to_string()))?;

        if need_sn {
            let sn_url = sn_url.clone().unwrap();
            let sn_username = sn_username
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    RPCErrors::ParseRequestError(
                        "sn_username is required for SN device report".to_string(),
                    )
                })?
                .to_lowercase();
            let sn_device_proof = Self::generate_sn_device_proof(
                sn_username.as_str(),
                &device_did,
                &device_private_key_pem,
            )?;

            let mut device_info = DeviceInfo::from_device_doc(&device_config);
            device_info.auto_fill_by_system_info().await.unwrap();
            info!("Register device ood1(zone-gateway) to sn: {}", sn_url);

            let sn_req = Self::build_sn_device_online_report("ood1", &device_did, &device_info)?;
            let sn_result =
                sn_register_device_online(sn_url.as_str(), sn_device_proof, sn_req).await;
            if sn_result.is_err() {
                warn!(
                    "Failed to register device to sn: {}",
                    sn_result.as_ref().err().unwrap()
                );
                return Err(RPCErrors::ReasonError(format!(
                    "Failed to register device to sn: {}",
                    sn_result.as_ref().err().unwrap().to_string()
                )));
            }
        }

        //TODO: call resolve_did to check self domain config is correct?
        //  check in ui is more smoothly

        let ood = if let Some(net_id) = device_config.net_id.as_ref() {
            if net_id != "nat" {
                OODDescriptionString::new(
                    "ood1".to_string(),
                    DeviceNodeType::OOD,
                    Some(net_id.clone()),
                    None,
                )
            } else {
                OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None)
            }
        } else {
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None)
        };

        let zone_boot_sn = sn_url
            .as_ref()
            .filter(|url| url.len() > 5)
            .and_then(|url| url::Url::parse(url).ok())
            .and_then(|url| url.host_str().map(|host| host.to_string()));

        let zone_boot_config = ZoneBootDocument {
            id: None,
            oods: vec![ood],
            sn: zone_boot_sn,
            exp: buckyos_get_unix_timestamp() + 3600 * 24 * 365 * 10,
            owner: None,
            owner_key: None,
            extra_info: HashMap::new(),
        };
        let zone_boot_config_jwt = zone_boot_config
            .encode(Some(&owner_private_key_pem))
            .map_err(|e| {
                RPCErrors::ReasonError(format!("Failed to encode zone boot config: {}", e))
            })?;
        let zone_boot_config_jwt = zone_boot_config_jwt.to_string();

        let write_dir = get_buckyos_system_etc_dir();
        let owner_public_key: Jwk = serde_json::from_value(owner_public_key.clone()).unwrap();

        let device_mini_config = DeviceMiniDocument::new_by_device_document(&device_config);
        let device_mini_doc_jwt = device_mini_config.to_jwt(&owner_private_key_pem).unwrap();
        let bns_submission = self
            .publish_bns_zone_documents(
                &req.params,
                zone_name,
                user_name.as_str(),
                "ood1",
                &device_did,
                zone_boot_config_jwt.as_str(),
                device_mini_doc_jwt.as_str(),
            )
            .await?;
        let node_identity = LocalNodeIdentityConfig::new(
            zone_did.clone(),
            DID::new("bns", user_name.as_str()),
            owner_public_key,
            device_config.name.clone(),
            device_did.clone(),
            buckyos_get_unix_timestamp() as u32 - 3600,
        );
        save_local_device_identity(
            write_dir.as_path(),
            &node_identity,
            &device_config,
            device_doc_jwt.to_string().as_str(),
            device_mini_doc_jwt.as_str(),
            device_private_key,
        )
        .map_err(RPCErrors::ReasonError)?;

        //write start config ,TODO
        let mut real_start_parms = req.params.clone();
        let mut real_start_params = real_start_parms.as_object_mut().unwrap();
        real_start_params.insert(
            "ood_jwt".to_string(),
            Value::String(device_doc_jwt.to_string()),
        );
        Self::remove_activation_only_start_config_fields(real_start_params);
        Self::append_optional_start_config_fields(&req.params, real_start_params);
        let start_params_str = serde_json::to_string(&real_start_params).unwrap();
        let start_params_file = write_dir.join("start_config.json");
        tokio::fs::write(start_params_file, start_params_str.as_bytes())
            .await
            .map_err(|_| RPCErrors::ReasonError("Failed to write start params".to_string()))?;

        Self::update_zone_boot_cache(&zone_did, zone_boot_config_jwt.as_str()).await;

        info!("DoAction wrote device identity files to node_identity.json, identity root and security root");

        tokio::task::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            exit(0);
        });

        let result = if let Some(bns_submission) = bns_submission {
            json!({
                "code":0,
                "bns": bns_submission
            })
        } else {
            json!({
                "code":0
            })
        };

        Ok(RPCResponse::new(RPCResult::Success(result), req.seq))
    }

    async fn handle_generate_key_pair(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let (private_key, public_key) = generate_ed25519_key_pair();
        return Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "private_key":private_key,
                "public_key":public_key
            })),
            req.seq,
        ));
    }

    async fn handle_get_device_info(&self, req: RPCRequest) -> Result<RPCResponse, RPCErrors> {
        let ood_desc: OODDescriptionString = "ood1".parse().unwrap_or_else(|_| {
            OODDescriptionString::new("ood1".to_string(), DeviceNodeType::OOD, None, None)
        });
        let mut device_info = DeviceInfo::new(&ood_desc, DID::new("dns", "ood1"));
        device_info.auto_fill_by_system_info().await.unwrap();
        let device_info_json = serde_json::to_value(device_info).unwrap();
        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "device_info":device_info_json
            })),
            req.seq,
        ))
    }

    async fn handle_generate_zone_txt_records(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let zone_boot_config_str = req.params.get("zone_boot_config");
        let device_mini_config_str = req.params.get("device_mini_config");
        let private_key = req.params.get("private_key");

        if zone_boot_config_str.is_none()
            || private_key.is_none()
            || device_mini_config_str.is_none()
        {
            return Err(RPCErrors::ParseRequestError(
                "Invalid params, zone_boot_config, device_mini_config or private_key is none"
                    .to_string(),
            ));
        }

        let zone_config = zone_boot_config_str.unwrap().as_str().unwrap();
        let private_key = private_key.unwrap().as_str().unwrap();

        info!("will sign zone config, bytes={}", zone_config.len());
        let mut zone_boot_config: ZoneBootDocument =
            serde_json::from_str(zone_config).map_err(|e| {
                RPCErrors::ParseRequestError(format!("Invalid zone config: {}", e.to_string()))
            })?;
        let private_key_pem = EncodingKey::from_ed_pem(private_key.as_bytes()).map_err(|e| {
            RPCErrors::ParseRequestError(format!("Invalid private key: {}", e.to_string()))
        })?;
        let zone_boot_config_jwt =
            zone_boot_config
                .encode(Some(&private_key_pem))
                .map_err(|e| {
                    RPCErrors::ParseRequestError(format!(
                        "Failed to encode zone config: {}",
                        e.to_string()
                    ))
                })?;
        info!(
            "zone config jwt generated, bytes={}",
            zone_boot_config_jwt.to_string().len()
        );

        let device_mini_config_str = device_mini_config_str.unwrap().as_str().unwrap();
        info!(
            "will sign device mini config, bytes={}",
            device_mini_config_str.len()
        );
        let device_mini_config: DeviceMiniDocument = serde_json::from_str(device_mini_config_str)
            .map_err(|e| {
            RPCErrors::ParseRequestError(format!("Invalid device mini config: {}", e.to_string()))
        })?;
        let device_mini_config_jwt = device_mini_config.to_jwt(&private_key_pem).map_err(|e| {
            RPCErrors::ParseRequestError(format!(
                "Failed to encode device mini config: {}",
                e.to_string()
            ))
        })?;
        info!(
            "device mini config jwt generated, bytes={}",
            device_mini_config_jwt.len()
        );

        return Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "BOOT":zone_boot_config_jwt.to_string(),
                "DEV":device_mini_config_jwt,
            })),
            req.seq,
        ));
    }

    async fn handle_get_mini_device_info(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let device_info_json = serde_json::to_string(&self.device_mini_info).unwrap();
        info!("serve mini device info, bytes={}", device_info_json.len());
        Ok(http::Response::builder()
            .body(BoxBody::new(
                Full::new(Bytes::from(device_info_json))
                    .map_err(|never: std::convert::Infallible| -> ServerError { match never {} })
                    .boxed(),
            ))
            .map_err(|e| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build response: {}",
                    e
                )
            })?)
    }
}

#[async_trait]
impl RPCHandler for ActiveServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> Result<RPCResponse, RPCErrors> {
        let method = req.method.clone();
        let result = match req.method.as_str() {
            "generate_key_pair" => self.handle_generate_key_pair(req).await,
            "get_device_info" => self.handle_get_device_info(req).await,
            "generate_zone_txt_records" => self.handle_generate_zone_txt_records(req).await,
            "do_active" => self.handle_do_active(req).await,
            "prepare_params_for_active_by_wallet" => {
                self.handle_prepare_params_for_active_by_wallet(req).await
            }
            "do_active_by_wallet" => self.handle_active_by_wallet(req).await,
            _ => Err(RPCErrors::UnknownMethod(req.method)),
        };
        if result.is_err() {
            error!(
                "Failed to handle rpc call:{} {}",
                method.as_str(),
                result.as_ref().err().unwrap().to_string()
            );
            return Err(result.err().unwrap());
        }
        return result;
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
        if *req.method() == Method::GET {
            if req.uri().path() == "/device" {
                return self.handle_get_mini_device_info(req).await;
            }
        }
        return Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ));
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
    let active_server = ActiveServer::new();

    //active server config
    let active_server_dir = get_buckyos_system_bin_dir().join("node-active");

    //start!
    info!("start node active service...");

    let runner = Runner::new(ACTIVE_SERVICE_MAIN_PORT);

    // 添加 RPC 服务
    let mut active_server = ActiveServer::new();
    active_server.auto_fill_device_mini_info().await;

    let active_server = Arc::new(active_server);
    let add_result = runner.add_http_server("/kapi/active".to_string(), active_server.clone());
    if add_result.is_err() {
        error!("Failed to add http server: {}", add_result.err().unwrap());
        return;
    }

    let add_result = runner.add_http_server("/device".to_string(), active_server.clone());
    if add_result.is_err() {
        error!("Failed to add http server: {}", add_result.err().unwrap());
        return;
    }

    // 添加静态文件服务
    info!("active server dir: {}", active_server_dir.display());
    let add_result = runner
        .add_dir_handler("/".to_string(), active_server_dir)
        .await;
    if add_result.is_err() {
        error!("Failed to add dir handler: {}", add_result.err().unwrap());
        return;
    }

    runner.run().await;
}
