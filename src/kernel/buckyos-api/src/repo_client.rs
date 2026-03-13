use crate::{AppDoc, AppType, SelectorType};
use ::kRPC::*;
use async_trait::async_trait;
use name_lib::DID;
use ndn_lib::{ActionObject, InclusionProof, ObjId};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::IpAddr;

pub const REPO_SERVICE_UNIQUE_ID: &str = "repo-service";
pub const REPO_SERVICE_SERVICE_NAME: &str = "repo-service";
pub const REPO_SERVICE_SERVICE_PORT: u16 = 4000;

pub const REPO_STATUS_COLLECTED: &str = "collected";
pub const REPO_STATUS_PINNED: &str = "pinned";

pub const REPO_ORIGIN_LOCAL: &str = "local";
pub const REPO_ORIGIN_REMOTE: &str = "remote";

pub const REPO_ACCESS_POLICY_FREE: &str = "free";
pub const REPO_ACCESS_POLICY_PAID: &str = "paid";

pub const REPO_PROOF_TYPE_COLLECTION: &str = "collection_proof";
pub const REPO_PROOF_TYPE_REFERRAL: &str = "referral_proof";
pub const REPO_PROOF_TYPE_DOWNLOAD: &str = "download_proof";
pub const REPO_PROOF_TYPE_INSTALL: &str = "install_proof";

pub const REPO_SERVE_STATUS_OK: &str = "ok";
pub const REPO_SERVE_STATUS_REJECT: &str = "reject";

pub const REPO_SERVE_REJECT_NOT_FOUND: &str = "not_found";
pub const REPO_SERVE_REJECT_NO_RECEIPT: &str = "no_receipt";
pub const REPO_SERVE_REJECT_INVALID_RECEIPT: &str = "invalid_receipt";

pub type RepoActionProof = ActionObject;
pub type RepoCollectionProof = InclusionProof;

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum RepoProof {
    Action(RepoActionProof),
    Collection(RepoCollectionProof),
}

impl RepoProof {
    pub fn action(proof: RepoActionProof) -> Self {
        Self::Action(proof)
    }

    pub fn collection(proof: RepoCollectionProof) -> Self {
        Self::Collection(proof)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoRecord {
    pub content_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_name: Option<String>,
    pub status: String,
    pub origin: String,
    pub meta: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_did: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    pub access_policy: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub collected_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<u64>,
}

impl RepoRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        content_id: String,
        content_name: Option<String>,
        status: String,
        origin: String,
        meta: Value,
        owner_did: Option<String>,
        author: Option<String>,
        access_policy: String,
        price: Option<String>,
        content_size: Option<u64>,
        collected_at: Option<u64>,
        pinned_at: Option<u64>,
        updated_at: Option<u64>,
    ) -> Self {
        Self {
            content_id,
            content_name,
            status,
            origin,
            meta,
            owner_did,
            author,
            access_policy,
            price,
            content_size,
            collected_at,
            pinned_at,
            updated_at,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoProofFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proof_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_did: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_did: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ts: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub end_ts: Option<u64>,
}

impl RepoProofFilter {
    pub fn new(
        proof_type: Option<String>,
        from_did: Option<String>,
        to_did: Option<String>,
        start_ts: Option<u64>,
        end_ts: Option<u64>,
    ) -> Self {
        Self {
            proof_type,
            from_did,
            to_did,
            start_ts,
            end_ts,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoListFilter {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_did: Option<String>,
}

impl RepoListFilter {
    pub fn new(
        status: Option<String>,
        origin: Option<String>,
        content_name: Option<String>,
        owner_did: Option<String>,
    ) -> Self {
        Self {
            status,
            origin,
            content_name,
            owner_did,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RepoStat {
    pub total_objects: u64,
    pub collected_objects: u64,
    pub pinned_objects: u64,
    pub local_objects: u64,
    pub remote_objects: u64,
    pub total_content_bytes: u64,
    pub total_proofs: u64,
}

impl RepoStat {
    pub fn new(
        total_objects: u64,
        collected_objects: u64,
        pinned_objects: u64,
        local_objects: u64,
        remote_objects: u64,
        total_content_bytes: u64,
        total_proofs: u64,
    ) -> Self {
        Self {
            total_objects,
            collected_objects,
            pinned_objects,
            local_objects,
            remote_objects,
            total_content_bytes,
            total_proofs,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoContentRef {
    pub content_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_url: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub metadata: Value,
}

impl RepoContentRef {
    pub fn new(content_id: String, access_url: Option<String>, metadata: Value) -> Self {
        Self {
            content_id,
            access_url,
            metadata,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RepoServeRequestContext {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_did: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requester_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<Value>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

impl RepoServeRequestContext {
    pub fn new(
        requester_did: Option<String>,
        requester_device_id: Option<String>,
        receipt: Option<Value>,
        extra: Value,
    ) -> Self {
        Self {
            requester_did,
            requester_device_id,
            receipt,
            extra,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RepoServeResult {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_ref: Option<RepoContentRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download_proof: Option<RepoActionProof>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
}

impl RepoServeResult {
    pub fn accepted(content_ref: RepoContentRef, download_proof: RepoActionProof) -> Self {
        Self {
            status: REPO_SERVE_STATUS_OK.to_string(),
            content_ref: Some(content_ref),
            download_proof: Some(download_proof),
            reject_code: None,
            reject_reason: None,
        }
    }

    pub fn rejected(reject_code: String, reject_reason: Option<String>) -> Self {
        Self {
            status: REPO_SERVE_STATUS_REJECT.to_string(),
            content_ref: None,
            download_proof: None,
            reject_code: Some(reject_code),
            reject_reason,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStoreReq {
    pub content_path: String,
}

impl RepoStoreReq {
    pub fn new(content_path: String) -> Self {
        Self { content_path }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoStoreReq: {}", error))
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RepoCollectReq {
    pub content_meta: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub referral_proof: Option<RepoActionProof>,
}

impl RepoCollectReq {
    pub fn new(content_meta: Value, referral_proof: Option<RepoActionProof>) -> Self {
        Self {
            content_meta,
            referral_proof,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoCollectReq: {}", error))
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RepoPinReq {
    pub content_id: String,
    pub download_proof: RepoActionProof,
}

impl RepoPinReq {
    pub fn new(content_id: String, download_proof: RepoActionProof) -> Self {
        Self {
            content_id,
            download_proof,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoPinReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoUnpinReq {
    pub content_id: String,
    #[serde(default)]
    pub force: bool,
}

impl RepoUnpinReq {
    pub fn new(content_id: String, force: bool) -> Self {
        Self { content_id, force }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoUnpinReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoUncollectReq {
    pub content_id: String,
    #[serde(default)]
    pub force: bool,
}

impl RepoUncollectReq {
    pub fn new(content_id: String, force: bool) -> Self {
        Self { content_id, force }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoUncollectReq: {}", error))
        })
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RepoAddProofReq {
    pub proof: RepoProof,
}

impl RepoAddProofReq {
    pub fn new(proof: RepoProof) -> Self {
        Self { proof }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoAddProofReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoGetProofsReq {
    pub content_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<RepoProofFilter>,
}

impl RepoGetProofsReq {
    pub fn new(content_id: String, filter: Option<RepoProofFilter>) -> Self {
        Self { content_id, filter }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoGetProofsReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoResolveReq {
    pub content_name: String,
}

impl RepoResolveReq {
    pub fn new(content_name: String) -> Self {
        Self { content_name }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoResolveReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoListReq {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<RepoListFilter>,
}

impl RepoListReq {
    pub fn new(filter: Option<RepoListFilter>) -> Self {
        Self { filter }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoListReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoStatReq {}

impl RepoStatReq {
    pub fn new() -> Self {
        Self {}
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoStatReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoServeReq {
    pub content_id: String,
    pub request_context: RepoServeRequestContext,
}

impl RepoServeReq {
    pub fn new(content_id: String, request_context: RepoServeRequestContext) -> Self {
        Self {
            content_id,
            request_context,
        }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoServeReq: {}", error))
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoAnnounceReq {
    pub content_id: String,
}

impl RepoAnnounceReq {
    pub fn new(content_id: String) -> Self {
        Self { content_id }
    }

    pub fn from_json(value: Value) -> std::result::Result<Self, RPCErrors> {
        serde_json::from_value(value).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Failed to parse RepoAnnounceReq: {}", error))
        })
    }
}

fn serialize_request<T: Serialize>(
    request: &T,
    type_name: &str,
) -> std::result::Result<Value, RPCErrors> {
    serde_json::to_value(request).map_err(|error| {
        RPCErrors::ReasonError(format!("Failed to serialize {}: {}", type_name, error))
    })
}

fn parse_response<T: DeserializeOwned>(
    value: Value,
    type_name: &str,
) -> std::result::Result<T, RPCErrors> {
    serde_json::from_value(value).map_err(|error| {
        RPCErrors::ParserResponseError(format!("Failed to parse {} response: {}", type_name, error))
    })
}

pub enum RepoClient {
    InProcess(Box<dyn RepoHandler>),
    KRPC(Box<kRPC>),
}

impl RepoClient {
    pub fn new(krpc_client: kRPC) -> Self {
        Self::new_krpc(Box::new(krpc_client))
    }

    pub fn new_in_process(handler: Box<dyn RepoHandler>) -> Self {
        Self::InProcess(handler)
    }

    pub fn new_krpc(krpc_client: Box<kRPC>) -> Self {
        Self::KRPC(krpc_client)
    }

    pub async fn set_context(&self, context: RPCContext) {
        match self {
            Self::InProcess(_) => {}
            Self::KRPC(client) => {
                client.set_context(context).await;
            }
        }
    }

    pub async fn store(&self, content_path: &str) -> std::result::Result<ObjId, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_store(content_path, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoStoreReq::new(content_path.to_string());
                let req_json = serialize_request(&req, "RepoStoreReq")?;
                let result = client.call("store", req_json).await?;
                parse_response(result, "ObjId")
            }
        }
    }

    pub async fn collect(
        &self,
        content_meta: Value,
        referral_proof: Option<RepoActionProof>,
    ) -> std::result::Result<String, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler
                    .handle_collect(content_meta, referral_proof, ctx)
                    .await
            }
            Self::KRPC(client) => {
                let req = RepoCollectReq::new(content_meta, referral_proof);
                let req_json = serialize_request(&req, "RepoCollectReq")?;
                let result = client.call("collect", req_json).await?;
                result
                    .as_str()
                    .map(|value| value.to_string())
                    .ok_or_else(|| {
                        RPCErrors::ParserResponseError(
                            "Expected content_id string from collect".to_string(),
                        )
                    })
            }
        }
    }

    pub async fn pin(
        &self,
        content_id: &str,
        download_proof: RepoActionProof,
    ) -> std::result::Result<bool, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_pin(content_id, download_proof, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoPinReq::new(content_id.to_string(), download_proof);
                let req_json = serialize_request(&req, "RepoPinReq")?;
                let result = client.call("pin", req_json).await?;
                result.as_bool().ok_or_else(|| {
                    RPCErrors::ParserResponseError("Expected bool from pin".to_string())
                })
            }
        }
    }

    pub async fn unpin(
        &self,
        content_id: &str,
        force: bool,
    ) -> std::result::Result<bool, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_unpin(content_id, force, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoUnpinReq::new(content_id.to_string(), force);
                let req_json = serialize_request(&req, "RepoUnpinReq")?;
                let result = client.call("unpin", req_json).await?;
                result.as_bool().ok_or_else(|| {
                    RPCErrors::ParserResponseError("Expected bool from unpin".to_string())
                })
            }
        }
    }

    pub async fn uncollect(
        &self,
        content_id: &str,
        force: bool,
    ) -> std::result::Result<bool, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_uncollect(content_id, force, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoUncollectReq::new(content_id.to_string(), force);
                let req_json = serialize_request(&req, "RepoUncollectReq")?;
                let result = client.call("uncollect", req_json).await?;
                result.as_bool().ok_or_else(|| {
                    RPCErrors::ParserResponseError("Expected bool from uncollect".to_string())
                })
            }
        }
    }

    pub async fn add_proof(&self, proof: RepoProof) -> std::result::Result<String, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_add_proof(proof, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoAddProofReq::new(proof);
                let req_json = serialize_request(&req, "RepoAddProofReq")?;
                let result = client.call("add_proof", req_json).await?;
                result
                    .as_str()
                    .map(|value| value.to_string())
                    .ok_or_else(|| {
                        RPCErrors::ParserResponseError(
                            "Expected proof_id string from add_proof".to_string(),
                        )
                    })
            }
        }
    }

    pub async fn get_proofs(
        &self,
        content_id: &str,
        filter: Option<RepoProofFilter>,
    ) -> std::result::Result<Vec<RepoProof>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_get_proofs(content_id, filter, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoGetProofsReq::new(content_id.to_string(), filter);
                let req_json = serialize_request(&req, "RepoGetProofsReq")?;
                let result = client.call("get_proofs", req_json).await?;
                parse_response(result, "Vec<RepoProof>")
            }
        }
    }

    pub async fn resolve(&self, content_name: &str) -> std::result::Result<Vec<ObjId>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_resolve(content_name, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoResolveReq::new(content_name.to_string());
                let req_json = serialize_request(&req, "RepoResolveReq")?;
                let result = client.call("resolve", req_json).await?;
                parse_response(result, "Vec<ObjId>")
            }
        }
    }

    pub async fn list(
        &self,
        filter: Option<RepoListFilter>,
    ) -> std::result::Result<Vec<RepoRecord>, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_list(filter, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoListReq::new(filter);
                let req_json = serialize_request(&req, "RepoListReq")?;
                let result = client.call("list", req_json).await?;
                parse_response(result, "Vec<RepoRecord>")
            }
        }
    }

    pub async fn stat(&self) -> std::result::Result<RepoStat, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_stat(ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoStatReq::new();
                let req_json = serialize_request(&req, "RepoStatReq")?;
                let result = client.call("stat", req_json).await?;
                parse_response(result, "RepoStat")
            }
        }
    }

    pub async fn serve(
        &self,
        content_id: &str,
        request_context: RepoServeRequestContext,
    ) -> std::result::Result<RepoServeResult, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_serve(content_id, request_context, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoServeReq::new(content_id.to_string(), request_context);
                let req_json = serialize_request(&req, "RepoServeReq")?;
                let result = client.call("serve", req_json).await?;
                parse_response(result, "RepoServeResult")
            }
        }
    }

    pub async fn announce(&self, content_id: &str) -> std::result::Result<bool, RPCErrors> {
        match self {
            Self::InProcess(handler) => {
                let ctx = RPCContext::default();
                handler.handle_announce(content_id, ctx).await
            }
            Self::KRPC(client) => {
                let req = RepoAnnounceReq::new(content_id.to_string());
                let req_json = serialize_request(&req, "RepoAnnounceReq")?;
                let result = client.call("announce", req_json).await?;
                result.as_bool().ok_or_else(|| {
                    RPCErrors::ParserResponseError("Expected bool from announce".to_string())
                })
            }
        }
    }
}

#[async_trait]
pub trait RepoHandler: Send + Sync {
    //TODO store成功返回ObjId
    async fn handle_store(
        &self,
        content_path: &str,
        ctx: RPCContext,
    ) -> std::result::Result<ObjId, RPCErrors>;

    async fn handle_collect(
        &self,
        content_meta: Value,
        referral_proof: Option<RepoActionProof>,
        ctx: RPCContext,
    ) -> std::result::Result<String, RPCErrors>;

    async fn handle_pin(
        &self,
        content_id: &str,
        download_proof: RepoActionProof,
        ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors>;

    async fn handle_unpin(
        &self,
        content_id: &str,
        force: bool,
        ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors>;

    async fn handle_uncollect(
        &self,
        content_id: &str,
        force: bool,
        ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors>;

    async fn handle_add_proof(
        &self,
        proof: RepoProof,
        ctx: RPCContext,
    ) -> std::result::Result<String, RPCErrors>;

    async fn handle_get_proofs(
        &self,
        content_id: &str,
        filter: Option<RepoProofFilter>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<RepoProof>, RPCErrors>;

    //TODO 返回本地Repo视角的，已经Pinned的ObjId列表
    async fn handle_resolve(
        &self,
        content_name: &str,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<ObjId>, RPCErrors>;

    async fn handle_list(
        &self,
        filter: Option<RepoListFilter>,
        ctx: RPCContext,
    ) -> std::result::Result<Vec<RepoRecord>, RPCErrors>;

    async fn handle_stat(&self, ctx: RPCContext) -> std::result::Result<RepoStat, RPCErrors>;

    async fn handle_serve(
        &self,
        content_id: &str,
        request_context: RepoServeRequestContext,
        ctx: RPCContext,
    ) -> std::result::Result<RepoServeResult, RPCErrors>;

    async fn handle_announce(
        &self,
        content_id: &str,
        ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors>;
}

pub struct RepoServerHandler<T: RepoHandler>(pub T);

impl<T: RepoHandler> RepoServerHandler<T> {
    pub fn new(handler: T) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<T: RepoHandler> RPCHandler for RepoServerHandler<T> {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let seq = req.seq;
        let trace_id = req.trace_id.clone();
        let ctx = RPCContext::from_request(&req, ip_from);

        let result = match req.method.as_str() {
            "store" => {
                let store_req = RepoStoreReq::from_json(req.params)?;
                let result = self.0.handle_store(&store_req.content_path, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "collect" => {
                let collect_req = RepoCollectReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_collect(collect_req.content_meta, collect_req.referral_proof, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "pin" => {
                let pin_req = RepoPinReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_pin(&pin_req.content_id, pin_req.download_proof, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "unpin" => {
                let unpin_req = RepoUnpinReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_unpin(&unpin_req.content_id, unpin_req.force, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "uncollect" => {
                let uncollect_req = RepoUncollectReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_uncollect(&uncollect_req.content_id, uncollect_req.force, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "add_proof" => {
                let proof_req = RepoAddProofReq::from_json(req.params)?;
                let result = self.0.handle_add_proof(proof_req.proof, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "get_proofs" => {
                let proofs_req = RepoGetProofsReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_get_proofs(&proofs_req.content_id, proofs_req.filter, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "resolve" => {
                let resolve_req = RepoResolveReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_resolve(&resolve_req.content_name, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "list" => {
                let list_req = RepoListReq::from_json(req.params)?;
                let result = self.0.handle_list(list_req.filter, ctx).await?;
                RPCResult::Success(json!(result))
            }
            "stat" => {
                let _ = RepoStatReq::from_json(req.params)?;
                let result = self.0.handle_stat(ctx).await?;
                RPCResult::Success(json!(result))
            }
            "serve" => {
                let serve_req = RepoServeReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_serve(&serve_req.content_id, serve_req.request_context, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            "announce" => {
                let announce_req = RepoAnnounceReq::from_json(req.params)?;
                let result = self
                    .0
                    .handle_announce(&announce_req.content_id, ctx)
                    .await?;
                RPCResult::Success(json!(result))
            }
            _ => return Err(RPCErrors::UnknownMethod(req.method.clone())),
        };

        Ok(RPCResponse {
            result,
            seq,
            trace_id,
        })
    }
}

pub fn generate_repo_service_doc() -> AppDoc {
    const VERSION: &str = env!("CARGO_PKG_VERSION");
    let owner_did = DID::from_str("did:bns:buckyos").unwrap();
    AppDoc::builder(
        AppType::Service,
        REPO_SERVICE_UNIQUE_ID,
        VERSION,
        "did:bns:buckyos",
        &owner_did,
    )
    .show_name("Repo Service")
    .selector_type(SelectorType::Random)
    .build()
    .unwrap()
}
