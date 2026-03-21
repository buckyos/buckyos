use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::repo_db::{
    RepoDb, RepoDbStat, RepoObjectRecord as DbRecord, RepoProofRecord,
    RepoReceiptRecord as ReceiptRecord, SqliteRepoDb,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, BuckyOSRuntimeType,
    RepoActionProof, RepoContentRef, RepoHandler, RepoListFilter, RepoProof, RepoProofFilter,
    RepoRecord, RepoServeRequestContext, RepoServeResult, RepoServerHandler, RepoStat,
    REPO_ACCESS_POLICY_FREE, REPO_ACCESS_POLICY_PAID, REPO_ORIGIN_LOCAL, REPO_ORIGIN_REMOTE,
    REPO_PROOF_TYPE_COLLECTION, REPO_PROOF_TYPE_DOWNLOAD, REPO_PROOF_TYPE_INSTALL,
    REPO_PROOF_TYPE_REFERRAL, REPO_SERVE_REJECT_NOT_FOUND, REPO_SERVE_REJECT_NO_RECEIPT,
    REPO_SERVICE_SERVICE_NAME, REPO_SERVICE_SERVICE_PORT, REPO_STATUS_COLLECTED,
    REPO_STATUS_PINNED,
};
use buckyos_kit::{buckyos_get_unix_timestamp, init_logging};
use bytes::Bytes;
use cyfs_gateway_lib::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use kRPC::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResponse};
use log::{info, warn};
use name_lib::decode_jwt_claim_without_verify;
use named_store::{NamedLocalStore, NamedStoreMgr, StoreLayout, StoreTarget};
use ndn_lib::{
    build_obj_id, load_named_object_from_obj_str, verify_named_object, ActionObject, ChunkId,
    FileObject, InclusionProof, NamedObject, ObjId, StoreMode, ACTION_TYPE_DOWNLOAD,
    ACTION_TYPE_INSTALLED, ACTION_TYPE_SHARED,
};
use ndn_toolkit::{cacl_file_object, CheckMode};
use serde_json::{json, Value};
use server_runner::Runner;
use tokio::fs;

const REPO_DB_FILE: &str = "repo.db";
const ANNOUNCES_DIR: &str = "pending_announces";
const READY_CHECK_STORE_DIR: &str = "ready_check_store";

#[derive(Clone)]
pub struct RepoService {
    db: Arc<dyn RepoDb>,
    state: Arc<RepoState>,
}

#[derive(Debug)]
struct RepoState {
    announces_dir: PathBuf,
    ready_check_store_dir: PathBuf,
}

impl RepoService {
    pub async fn new(data_dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&data_dir)
            .await
            .with_context(|| format!("create repo data dir failed: {}", data_dir.display()))?;
        let announces_dir = data_dir.join(ANNOUNCES_DIR);
        let ready_check_store_dir = data_dir.join(READY_CHECK_STORE_DIR);
        fs::create_dir_all(&announces_dir)
            .await
            .with_context(|| format!("create announce dir failed: {}", announces_dir.display()))?;
        fs::create_dir_all(&ready_check_store_dir)
            .await
            .with_context(|| {
                format!(
                    "create ready check store dir failed: {}",
                    ready_check_store_dir.display()
                )
            })?;

        let state = Arc::new(RepoState {
            announces_dir,
            ready_check_store_dir,
        });
        let db = Arc::new(
            SqliteRepoDb::open(data_dir.join(REPO_DB_FILE))
                .await
                .context("init repo-db failed")?,
        );
        let service = Self { db, state };
        Ok(service)
    }

    async fn get_named_store_mgr(&self) -> std::result::Result<NamedStoreMgr, RPCErrors> {
        if let Ok(runtime) = get_buckyos_api_runtime() {
            return runtime.get_named_store().await.map_err(|err| {
                RPCErrors::ReasonError(format!("open system named store failed: {err}"))
            });
        }

        create_ready_check_store_mgr(&self.state.ready_check_store_dir).await
    }

    async fn inspect_local_content(
        &self,
        content_id: &ObjId,
    ) -> std::result::Result<(Option<u64>, Option<Value>), RPCErrors> {
        let store_mgr = self.get_named_store_mgr().await?;
        if content_id.is_chunk() {
            let chunk_id = ChunkId::from_obj_id(content_id);
            if !store_mgr.have_chunk(&chunk_id).await {
                return Err(RPCErrors::ReasonError(format!(
                    "content {} is not available in named store",
                    content_id
                )));
            }
            return Ok((chunk_id.get_length(), None));
        }

        let object_str = store_mgr.get_object(content_id).await.map_err(|err| {
            RPCErrors::ReasonError(format!(
                "load published content {} from named store failed: {err}",
                content_id
            ))
        })?;
        let object_json = serde_json::from_str::<Value>(object_str.as_str())
            .or_else(|_| load_named_object_from_obj_str(object_str.as_str()))
            .map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "parse published content {} from named store failed: {err}",
                    content_id
                ))
            })?;

        let chunk_ids_result = if let Ok(runtime) = get_buckyos_api_runtime() {
            runtime
                .get_chunklist_from_known_named_object(content_id, &object_json)
                .await
        } else {
            ndn_toolkit::get_chunklist_from_known_named_object(&store_mgr, content_id, &object_json)
                .await
                .map_err(|err| {
                    RPCErrors::ReasonError(format!(
                        "get chunklist from content {} failed: {err}",
                        content_id
                    ))
                })
        };

        let chunk_ids = match chunk_ids_result {
            Ok(chunk_ids) => chunk_ids,
            Err(error) => {
                let error_text = error.to_string();
                if error_text.contains("not a supported known named object")
                    || error_text.contains("invalid obj type")
                {
                    warn!(
                        "treat content {} as metadata-only object because chunklist lookup is unsupported: {}",
                        content_id, error_text
                    );
                    let object_size = u64::try_from(object_str.len()).ok();
                    return Ok((object_size, Some(object_json)));
                }
                return Err(error);
            }
        };

        for chunk_id in &chunk_ids {
            if !store_mgr.have_chunk(chunk_id).await {
                return Err(RPCErrors::ReasonError(format!(
                    "content {} is missing chunk {} in named store",
                    content_id,
                    chunk_id.to_string()
                )));
            }
        }

        let content_size = if chunk_ids.is_empty() {
            Some(0)
        } else {
            let mut total_size = 0u64;
            for chunk_id in &chunk_ids {
                let chunk_len = chunk_id.get_length().ok_or_else(|| {
                    RPCErrors::ReasonError(format!(
                        "content {} has chunk {} without encoded length",
                        content_id,
                        chunk_id.to_string()
                    ))
                })?;
                total_size += chunk_len;
            }
            Some(total_size)
        };

        Ok((content_size, Some(object_json)))
    }

    async fn ensure_local_content(
        &self,
        content_id: &ObjId,
        record: &DbRecord,
    ) -> std::result::Result<Option<u64>, RPCErrors> {
        let (content_size, _) = self.inspect_local_content(content_id).await?;
        Ok(content_size
            .or(record.content_size)
            .or_else(|| extract_content_size(&record.meta)))
    }

    async fn write_object_record(&self, record: DbRecord) -> std::result::Result<(), RPCErrors> {
        self.db.upsert_object(record).await.map_err(to_rpc_error)
    }

    async fn load_record(
        &self,
        content_id: String,
    ) -> std::result::Result<Option<DbRecord>, RPCErrors> {
        self.db.get_object(&content_id).await.map_err(to_rpc_error)
    }

    async fn load_all_records(&self) -> std::result::Result<Vec<DbRecord>, RPCErrors> {
        self.db.list_objects().await.map_err(to_rpc_error)
    }

    async fn save_proof(&self, proof: RepoProof) -> std::result::Result<String, RPCErrors> {
        let insert = proof_to_insert(&proof)?;
        let proof_id = insert.proof_id.clone();
        self.db.upsert_proof(insert).await.map_err(to_rpc_error)?;
        Ok(proof_id)
    }

    async fn load_proofs(
        &self,
        content_id: String,
    ) -> std::result::Result<Vec<RepoProofRecord>, RPCErrors> {
        self.db.list_proofs(&content_id).await.map_err(to_rpc_error)
    }

    async fn cache_receipt(&self, receipt: ReceiptRecord) -> std::result::Result<(), RPCErrors> {
        self.db.upsert_receipt(receipt).await.map_err(to_rpc_error)
    }

    async fn cache_announce_request(
        &self,
        content_id: &str,
        meta: &Value,
    ) -> std::result::Result<(), RPCErrors> {
        let file_name = format!(
            "{}-{}.json",
            buckyos_get_unix_timestamp(),
            sanitize_content_id(content_id)
        );
        let payload = json!({
            "content_id": content_id,
            "meta": meta,
            "requested_at": buckyos_get_unix_timestamp(),
        });
        fs::write(
            self.state.announces_dir.join(file_name),
            serde_json::to_vec_pretty(&payload).map_err(|err| {
                RPCErrors::ReasonError(format!("serialize announce payload failed: {err}"))
            })?,
        )
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("write announce request failed: {err}")))
    }

    async fn pin_record(
        &self,
        mut record: DbRecord,
        content_id: &ObjId,
        content_size: Option<u64>,
        download_proof: Option<RepoActionProof>,
    ) -> std::result::Result<(), RPCErrors> {
        if let Some(download_proof) = download_proof {
            validate_action_target(&download_proof, content_id, ACTION_TYPE_DOWNLOAD)?;
            self.save_proof(RepoProof::action(download_proof)).await?;
        }

        let now = buckyos_get_unix_timestamp();
        record.status = REPO_STATUS_PINNED.to_string();
        record.content_size = content_size.or(record.content_size);
        record.pinned_at = Some(now);
        record.updated_at = Some(now);
        record.collected_at = record.collected_at.or(Some(now));
        self.write_object_record(record).await
    }
}

#[async_trait]
impl RepoHandler for RepoService {
    async fn handle_store(
        &self,
        content_id: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<ObjId, RPCErrors> {
        let content_id = parse_obj_id(content_id)?;
        let existing = self.load_record(content_id.to_string()).await?;
        let (inspected_size, inspected_meta) = self.inspect_local_content(&content_id).await?;
        let content_size = inspected_size
            .or_else(|| existing.as_ref().and_then(|row| row.content_size))
            .or_else(|| {
                existing
                    .as_ref()
                    .and_then(|row| extract_content_size(&row.meta))
            })
            .unwrap_or(0);
        let meta = existing
            .as_ref()
            .map(|row| row.meta.clone())
            .or(inspected_meta)
            .map(|meta| normalize_store_meta(meta, &content_id.to_string(), content_size))
            .unwrap_or_else(|| fallback_meta_for_store(&content_id.to_string(), content_size));
        let record = DbRecord {
            content_id: content_id.to_string(),
            content_name: extract_content_name(&meta),
            origin: REPO_ORIGIN_LOCAL.to_string(),
            meta: meta.clone(),
            owner_did: extract_owner_did(&meta),
            author: extract_author(&meta),
            access_policy: extract_access_policy(&meta),
            price: extract_price(&meta),
            status: existing
                .as_ref()
                .map(|row| row.status.clone())
                .unwrap_or_else(|| REPO_STATUS_COLLECTED.to_string()),
            content_size: Some(content_size),
            collected_at: existing.as_ref().and_then(|row| row.collected_at),
            pinned_at: existing.as_ref().and_then(|row| row.pinned_at),
            updated_at: existing.as_ref().and_then(|row| row.updated_at),
        };
        self.pin_record(record, &content_id, Some(content_size), None)
            .await?;
        Ok(content_id)
    }

    async fn handle_collect(
        &self,
        content_meta: Value,
        referral_proof: Option<RepoActionProof>,
        _ctx: RPCContext,
    ) -> std::result::Result<String, RPCErrors> {
        let meta = normalize_meta_input(content_meta)?;
        let content_id = extract_content_id_from_meta(&meta)?;
        if let Some(referral_proof) = referral_proof {
            validate_action_target(&referral_proof, &content_id, ACTION_TYPE_SHARED)?;
            self.save_proof(RepoProof::action(referral_proof)).await?;
        }

        let existing = self.load_record(content_id.to_string()).await?;
        let now = buckyos_get_unix_timestamp();
        let record = DbRecord {
            content_id: content_id.to_string(),
            content_name: extract_content_name(&meta),
            status: existing
                .as_ref()
                .map(|row| row.status.clone())
                .unwrap_or_else(|| REPO_STATUS_COLLECTED.to_string()),
            origin: existing
                .as_ref()
                .map(|row| row.origin.clone())
                .unwrap_or_else(|| REPO_ORIGIN_REMOTE.to_string()),
            meta: meta.clone(),
            owner_did: extract_owner_did(&meta),
            author: extract_author(&meta),
            access_policy: extract_access_policy(&meta),
            price: extract_price(&meta),
            content_size: existing
                .as_ref()
                .and_then(|row| row.content_size)
                .or_else(|| extract_content_size(&meta)),
            collected_at: existing
                .as_ref()
                .and_then(|row| row.collected_at)
                .or(Some(now)),
            pinned_at: existing.as_ref().and_then(|row| row.pinned_at),
            updated_at: Some(now),
        };
        self.write_object_record(record).await?;
        Ok(content_id.to_string())
    }

    async fn handle_pin(
        &self,
        content_id: &str,
        download_proof: RepoActionProof,
        _ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors> {
        let content_id = parse_obj_id(content_id)?;
        let record = self
            .load_record(content_id.to_string())
            .await?
            .ok_or_else(|| {
                RPCErrors::ReasonError(format!("content {} not collected", content_id))
            })?;
        let content_size = self.ensure_local_content(&content_id, &record).await?;
        self.pin_record(record, &content_id, content_size, Some(download_proof))
            .await?;
        Ok(true)
    }

    async fn handle_unpin(
        &self,
        content_id: &str,
        force: bool,
        _ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors> {
        let content_id = parse_obj_id(content_id)?;
        let record = self
            .load_record(content_id.to_string())
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("content {} not found", content_id)))?;
        if record.status != REPO_STATUS_PINNED {
            return Ok(true);
        }
        if record.origin == REPO_ORIGIN_LOCAL && !force {
            return Err(RPCErrors::ReasonError(format!(
                "content {} was stored locally; force=true required to unpin",
                content_id
            )));
        }

        let now = buckyos_get_unix_timestamp();
        let updated = DbRecord {
            status: REPO_STATUS_COLLECTED.to_string(),
            pinned_at: None,
            updated_at: Some(now),
            ..record
        };
        self.write_object_record(updated).await?;
        Ok(true)
    }

    async fn handle_uncollect(
        &self,
        content_id: &str,
        force: bool,
        ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors> {
        let content_id = parse_obj_id(content_id)?;
        let record = self
            .load_record(content_id.to_string())
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("content {} not found", content_id)))?;

        if record.origin == REPO_ORIGIN_LOCAL && !force {
            return Err(RPCErrors::ReasonError(format!(
                "content {} was stored locally; force=true required to uncollect",
                content_id
            )));
        }
        if record.status == REPO_STATUS_PINNED {
            let content_id_string = content_id.to_string();
            self.handle_unpin(&content_id_string, true, ctx).await?;
        }

        self.db
            .delete_object(&content_id.to_string())
            .await
            .map_err(to_rpc_error)?;
        Ok(true)
    }

    async fn handle_add_proof(
        &self,
        proof: RepoProof,
        _ctx: RPCContext,
    ) -> std::result::Result<String, RPCErrors> {
        self.save_proof(proof).await
    }

    async fn handle_get_proofs(
        &self,
        content_id: &str,
        filter: Option<RepoProofFilter>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<RepoProof>, RPCErrors> {
        let content_id = parse_obj_id(content_id)?;
        let rows = self.load_proofs(content_id.to_string()).await?;
        let mut result = Vec::new();
        for row in rows {
            let proof = proof_from_row(&row)?;
            if filter
                .as_ref()
                .map(|filter| proof_matches_filter(&proof, filter))
                .unwrap_or(true)
            {
                result.push(proof);
            }
        }
        Ok(result)
    }

    async fn handle_resolve(
        &self,
        content_name: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<ObjId>, RPCErrors> {
        let records = self.load_all_records().await?;
        records
            .into_iter()
            .filter(|record| {
                record.status == REPO_STATUS_PINNED
                    && record.content_name.as_deref() == Some(content_name)
            })
            .map(|record| parse_obj_id(&record.content_id))
            .collect()
    }

    async fn handle_list(
        &self,
        filter: Option<RepoListFilter>,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<RepoRecord>, RPCErrors> {
        let records = self.load_all_records().await?;
        Ok(records
            .into_iter()
            .filter(|record| {
                filter
                    .as_ref()
                    .map(|filter| record_matches_filter(record, filter))
                    .unwrap_or(true)
            })
            .map(|record| {
                RepoRecord::new(
                    record.content_id,
                    record.content_name,
                    record.status,
                    record.origin,
                    record.meta,
                    record.owner_did,
                    record.author,
                    record.access_policy,
                    record.price,
                    record.content_size,
                    record.collected_at,
                    record.pinned_at,
                    record.updated_at,
                )
            })
            .collect())
    }

    async fn handle_stat(&self, _ctx: RPCContext) -> std::result::Result<RepoStat, RPCErrors> {
        let RepoDbStat {
            total_objects,
            collected_objects,
            pinned_objects,
            local_objects,
            remote_objects,
            total_content_bytes,
            total_proofs,
        } = self.db.stat().await.map_err(to_rpc_error)?;
        Ok(RepoStat::new(
            total_objects,
            collected_objects,
            pinned_objects,
            local_objects,
            remote_objects,
            total_content_bytes,
            total_proofs,
        ))
    }

    async fn handle_serve(
        &self,
        content_id: &str,
        request_context: RepoServeRequestContext,
        _ctx: RPCContext,
    ) -> std::result::Result<RepoServeResult, RPCErrors> {
        let content_id = parse_obj_id(content_id)?;
        let record = match self.load_record(content_id.to_string()).await? {
            Some(record) if record.status == REPO_STATUS_PINNED => record,
            _ => {
                return Ok(RepoServeResult::rejected(
                    REPO_SERVE_REJECT_NOT_FOUND.to_string(),
                    Some("content not found or not pinned".to_string()),
                ));
            }
        };
        if let Err(err) = self.ensure_local_content(&content_id, &record).await {
            warn!(
                "serve rejected for {} because content is not ready: {}",
                content_id, err
            );
            return Ok(RepoServeResult::rejected(
                REPO_SERVE_REJECT_NOT_FOUND.to_string(),
                Some("content not found or not pinned".to_string()),
            ));
        }

        if record.access_policy == REPO_ACCESS_POLICY_PAID {
            let receipt_value = match request_context.receipt.clone() {
                Some(receipt) => receipt,
                None => {
                    return Ok(RepoServeResult::rejected(
                        REPO_SERVE_REJECT_NO_RECEIPT.to_string(),
                        Some("receipt is required for paid content".to_string()),
                    ));
                }
            };
            let receipt = validate_receipt(&record, receipt_value)?;
            self.cache_receipt(receipt).await?;
        }

        let actor_identity = request_context
            .requester_did
            .clone()
            .or(request_context.requester_device_id.clone())
            .unwrap_or_else(|| "anonymous".to_string());
        let download_proof = ActionObject {
            subject: build_actor_obj_id(&actor_identity),
            action: ACTION_TYPE_DOWNLOAD.to_string(),
            target: content_id.clone(),
            base_on: None,
            details: Some(json!({
                "requester_did": request_context.requester_did,
                "requester_device_id": request_context.requester_device_id,
                "served_by": REPO_SERVICE_SERVICE_NAME,
            })),
            iat: buckyos_get_unix_timestamp(),
            exp: buckyos_get_unix_timestamp() + 3600 * 24 * 30,
        };
        self.save_proof(RepoProof::action(download_proof.clone()))
            .await?;

        Ok(RepoServeResult::accepted(
            RepoContentRef::new(content_id.to_string(), None, record.meta.clone()),
            download_proof,
        ))
    }

    async fn handle_announce(
        &self,
        content_id: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<bool, RPCErrors> {
        let record = self
            .load_record(content_id.to_string())
            .await?
            .ok_or_else(|| RPCErrors::ReasonError(format!("content {} not found", content_id)))?;
        if record.origin != REPO_ORIGIN_LOCAL {
            return Err(RPCErrors::ReasonError(format!(
                "content {} is not local; only locally stored content can be announced",
                content_id
            )));
        }
        if record.content_name.is_none() {
            return Err(RPCErrors::ReasonError(format!(
                "content {} has no content_name; cannot announce",
                content_id
            )));
        }
        self.cache_announce_request(content_id, &record.meta)
            .await?;
        warn!(
            "repo announce request queued locally for {}; external BNS publishing is not wired in this repository yet",
            content_id
        );
        Ok(true)
    }
}

struct RepoHttpServer {
    rpc_handler: RepoServerHandler<RepoService>,
}

impl RepoHttpServer {
    fn new(service: RepoService) -> Self {
        Self {
            rpc_handler: RepoServerHandler::new(service),
        }
    }
}

#[async_trait]
impl RPCHandler for RepoHttpServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: std::net::IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        self.rpc_handler.handle_rpc_call(req, ip_from).await
    }
}

#[async_trait]
impl HttpServer for RepoHttpServer {
    async fn serve_request(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
        info: StreamInfo,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        if *req.method() == Method::POST {
            return serve_http_by_rpc_handler(req, info, self).await;
        }
        Err(server_err!(
            ServerErrorCode::BadRequest,
            "Method not allowed"
        ))
    }

    fn id(&self) -> String {
        REPO_SERVICE_SERVICE_NAME.to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

pub async fn run_service() -> Result<()> {
    init_logging("repo_service", true);
    let mut runtime = init_buckyos_api_runtime(
        REPO_SERVICE_SERVICE_NAME,
        None,
        BuckyOSRuntimeType::KernelService,
    )
    .await
    .context("init repo-service runtime failed")?;
    runtime.login().await.context("repo-service login failed")?;
    runtime
        .set_main_service_port(REPO_SERVICE_SERVICE_PORT)
        .await;
    let data_dir = runtime
        .get_data_folder()
        .context("repo-service resolve data folder failed")?;
    set_buckyos_api_runtime(runtime).context("register repo-service runtime failed")?;

    let service = RepoService::new(data_dir).await?;
    let server = Arc::new(RepoHttpServer::new(service));
    let runner = Runner::new(REPO_SERVICE_SERVICE_PORT);
    runner
        .add_http_server("/kapi/repo-service".to_string(), server.clone())
        .context("register /kapi/repo-service failed")?;
    runner
        .add_http_server("/kapi/repo".to_string(), server)
        .context("register /kapi/repo failed")?;

    if let Ok(runtime) = get_buckyos_api_runtime() {
        info!(
            "repo-service started: port={}, data_dir={}",
            REPO_SERVICE_SERVICE_PORT,
            runtime
                .get_data_folder()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|error| format!("<unavailable: {}>", error))
        );
    }
    runner.run().await.context("repo-service runner failed")
}

fn to_rpc_error(err: anyhow::Error) -> RPCErrors {
    RPCErrors::ReasonError(err.to_string())
}

fn parse_obj_id(raw: &str) -> std::result::Result<ObjId, RPCErrors> {
    ObjId::new(raw).map_err(|err| RPCErrors::ReasonError(format!("invalid obj id `{raw}`: {err}")))
}

fn build_actor_obj_id(raw: &str) -> ObjId {
    build_obj_id("actor", raw)
}

fn extract_content_name(meta: &Value) -> Option<String> {
    meta.as_object().and_then(|meta| {
        meta.get("content_name")
            .and_then(Value::as_str)
            .or_else(|| meta.get("name").and_then(Value::as_str))
            .map(|value| value.to_string())
    })
}

fn extract_owner_did(meta: &Value) -> Option<String> {
    meta.as_object().and_then(|meta| {
        meta.get("owner_did")
            .and_then(Value::as_str)
            .or_else(|| meta.get("owner").and_then(Value::as_str))
            .or_else(|| meta.get("did").and_then(Value::as_str))
            .map(|value| value.to_string())
    })
}

fn extract_author(meta: &Value) -> Option<String> {
    meta.as_object()
        .and_then(|meta| meta.get("author").and_then(Value::as_str))
        .map(|value| value.to_string())
}

fn extract_access_policy(meta: &Value) -> String {
    meta.as_object()
        .and_then(|meta| meta.get("access_policy").and_then(Value::as_str))
        .unwrap_or(REPO_ACCESS_POLICY_FREE)
        .to_string()
}

fn extract_price(meta: &Value) -> Option<String> {
    meta.as_object().and_then(|meta| {
        meta.get("price").map(|value| {
            if let Some(value) = value.as_str() {
                value.to_string()
            } else {
                value.to_string()
            }
        })
    })
}

fn extract_content_size(meta: &Value) -> Option<u64> {
    meta.as_object().and_then(|meta| {
        meta.get("size")
            .or_else(|| meta.get("content_size"))
            .and_then(|value| match value {
                Value::Number(number) => number.as_u64(),
                Value::String(raw) => raw.parse::<u64>().ok(),
                _ => None,
            })
    })
}

fn normalize_meta_input(value: Value) -> std::result::Result<Value, RPCErrors> {
    match value {
        Value::String(raw) => {
            if raw.trim_start().starts_with('{') {
                serde_json::from_str::<Value>(&raw).map_err(|err| {
                    RPCErrors::ReasonError(format!("parse metadata json string failed: {err}"))
                })
            } else {
                decode_jwt_claim_without_verify(raw.trim()).map_err(|err| {
                    RPCErrors::ReasonError(format!("decode metadata jwt failed: {err}"))
                })
            }
        }
        other => Ok(other),
    }
}

fn normalize_store_meta(meta: Value, content_id: &str, content_size: u64) -> Value {
    let mut meta = match meta {
        Value::Object(map) => Value::Object(map),
        _ => fallback_meta_for_store(content_id, content_size),
    };
    if let Some(object) = meta.as_object_mut() {
        object.insert(
            "content_id".to_string(),
            Value::String(content_id.to_string()),
        );
        object.insert("content".to_string(), Value::String(content_id.to_string()));
        object.insert("size".to_string(), Value::Number(content_size.into()));
        if !object.contains_key("content_name") {
            if let Some(name) = object.get("name").and_then(Value::as_str) {
                object.insert("content_name".to_string(), Value::String(name.to_string()));
            } else {
                object.insert(
                    "content_name".to_string(),
                    Value::String(content_id.to_string()),
                );
            }
        }
    }
    meta
}

fn fallback_meta_for_store(content_id: &str, content_size: u64) -> Value {
    json!({
        "name": content_id,
        "content_name": content_id,
        "author": REPO_SERVICE_SERVICE_NAME,
        "content_id": content_id,
        "content": content_id,
        "size": content_size,
    })
}

fn extract_content_id_from_meta(meta: &Value) -> std::result::Result<ObjId, RPCErrors> {
    if let Some(content_id) = meta
        .get("content_id")
        .and_then(Value::as_str)
        .or_else(|| meta.get("content").and_then(Value::as_str))
        .or_else(|| meta.get("obj_id").and_then(Value::as_str))
    {
        return parse_obj_id(content_id);
    }
    Err(RPCErrors::ReasonError(
        "content metadata must include content_id or content".to_string(),
    ))
}

fn validate_action_target(
    proof: &ActionObject,
    target: &ObjId,
    expected_action: &str,
) -> std::result::Result<(), RPCErrors> {
    if proof.action != expected_action {
        return Err(RPCErrors::ReasonError(format!(
            "expected action `{expected_action}`, got `{}`",
            proof.action
        )));
    }
    if &proof.target != target {
        return Err(RPCErrors::ReasonError(format!(
            "proof target {} does not match content {}",
            proof.target, target
        )));
    }
    Ok(())
}

fn proof_to_insert(proof: &RepoProof) -> std::result::Result<RepoProofRecord, RPCErrors> {
    match proof {
        RepoProof::Action(action) => {
            let proof_id = action.gen_obj_id().0.to_string();
            Ok(RepoProofRecord {
                proof_id,
                content_id: action.target.to_string(),
                proof_kind: "action".to_string(),
                action_type: Some(action.action.clone()),
                subject_id: Some(action.subject.to_string()),
                target_id: Some(action.target.to_string()),
                base_on: action.base_on.as_ref().map(ToString::to_string),
                curator_did: None,
                proof_data: serde_json::to_string(action).map_err(|err| {
                    RPCErrors::ReasonError(format!("serialize action proof failed: {err}"))
                })?,
                created_at: action.iat,
            })
        }
        RepoProof::Collection(collection) => {
            let proof_id = collection.gen_obj_id().0.to_string();
            let content_id = parse_obj_id(&collection.content_id)?;
            if !verify_named_object(&content_id, &collection.content_obj) {
                return Err(RPCErrors::ReasonError(format!(
                    "collection proof content_obj does not match content_id {}",
                    collection.content_id
                )));
            }
            Ok(RepoProofRecord {
                proof_id,
                content_id: collection.content_id.clone(),
                proof_kind: "collection".to_string(),
                action_type: None,
                subject_id: None,
                target_id: None,
                base_on: None,
                curator_did: Some(collection.curator.to_string()),
                proof_data: serde_json::to_string(collection).map_err(|err| {
                    RPCErrors::ReasonError(format!("serialize collection proof failed: {err}"))
                })?,
                created_at: collection.iat,
            })
        }
    }
}

fn proof_from_row(row: &RepoProofRecord) -> std::result::Result<RepoProof, RPCErrors> {
    match row.proof_kind.as_str() {
        "action" => serde_json::from_str::<ActionObject>(&row.proof_data)
            .map(RepoProof::action)
            .map_err(|err| {
                RPCErrors::ReasonError(format!("parse action proof {} failed: {err}", row.proof_id))
            }),
        "collection" => serde_json::from_str::<InclusionProof>(&row.proof_data)
            .map(RepoProof::collection)
            .map_err(|err| {
                RPCErrors::ReasonError(format!(
                    "parse collection proof {} failed: {err}",
                    row.proof_id
                ))
            }),
        other => Err(RPCErrors::ReasonError(format!(
            "unsupported proof kind `{other}`"
        ))),
    }
}

fn proof_matches_filter(proof: &RepoProof, filter: &RepoProofFilter) -> bool {
    if let Some(proof_type) = filter.proof_type.as_deref() {
        let matched = match proof {
            RepoProof::Collection(_) => {
                proof_type == REPO_PROOF_TYPE_COLLECTION || proof_type == "collection"
            }
            RepoProof::Action(action) => match proof_type {
                REPO_PROOF_TYPE_REFERRAL => action.action == ACTION_TYPE_SHARED,
                REPO_PROOF_TYPE_DOWNLOAD => action.action == ACTION_TYPE_DOWNLOAD,
                REPO_PROOF_TYPE_INSTALL => action.action == ACTION_TYPE_INSTALLED,
                "shared" => action.action == ACTION_TYPE_SHARED,
                "download" => action.action == ACTION_TYPE_DOWNLOAD,
                "installed" => action.action == ACTION_TYPE_INSTALLED,
                "action" => true,
                _ => false,
            },
        };
        if !matched {
            return false;
        }
    }

    if let Some(from_did) = filter.from_did.as_deref() {
        let matched = match proof {
            RepoProof::Collection(collection) => collection.curator.to_string() == from_did,
            RepoProof::Action(action) => action
                .details
                .as_ref()
                .and_then(|details| details.get("subject_did").and_then(Value::as_str))
                .map(|value| value == from_did)
                .unwrap_or_else(|| action.subject.to_string() == from_did),
        };
        if !matched {
            return false;
        }
    }

    if let Some(to_did) = filter.to_did.as_deref() {
        let matched = match proof {
            RepoProof::Collection(_) => false,
            RepoProof::Action(action) => action
                .details
                .as_ref()
                .and_then(|details| details.get("target_did").and_then(Value::as_str))
                .or_else(|| {
                    action
                        .details
                        .as_ref()
                        .and_then(|details| details.get("requester_did").and_then(Value::as_str))
                })
                .map(|value| value == to_did)
                .unwrap_or_else(|| action.target.to_string() == to_did),
        };
        if !matched {
            return false;
        }
    }

    let proof_ts = match proof {
        RepoProof::Action(action) => action.iat,
        RepoProof::Collection(collection) => collection.iat,
    };
    if let Some(start_ts) = filter.start_ts {
        if proof_ts < start_ts {
            return false;
        }
    }
    if let Some(end_ts) = filter.end_ts {
        if proof_ts > end_ts {
            return false;
        }
    }

    true
}

fn record_matches_filter(record: &DbRecord, filter: &RepoListFilter) -> bool {
    if let Some(status) = filter.status.as_deref() {
        if record.status != status {
            return false;
        }
    }
    if let Some(origin) = filter.origin.as_deref() {
        if record.origin != origin {
            return false;
        }
    }
    if let Some(content_name) = filter.content_name.as_deref() {
        if !record
            .content_name
            .as_deref()
            .map(|name| name.starts_with(content_name))
            .unwrap_or(false)
        {
            return false;
        }
    }
    if let Some(owner_did) = filter.owner_did.as_deref() {
        if record.owner_did.as_deref() != Some(owner_did) {
            return false;
        }
    }
    true
}

fn validate_receipt(
    record: &DbRecord,
    receipt_value: Value,
) -> std::result::Result<ReceiptRecord, RPCErrors> {
    let receipt_id = receipt_value
        .get("receipt_id")
        .or_else(|| receipt_value.get("id"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RPCErrors::ReasonError("paid content receipt must include receipt_id".to_string())
        })?
        .to_string();
    let content_name = receipt_value
        .get("content_name")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            RPCErrors::ReasonError("paid content receipt must include content_name".to_string())
        })?
        .to_string();
    let expected_name = record.content_name.as_deref().ok_or_else(|| {
        RPCErrors::ReasonError("content has no content_name but requires paid receipt".to_string())
    })?;
    if content_name != expected_name {
        return Err(RPCErrors::ReasonError(format!(
            "receipt content_name `{content_name}` does not match `{expected_name}`"
        )));
    }
    let buyer_did = receipt_value
        .get("buyer_did")
        .or_else(|| receipt_value.get("buyer"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RPCErrors::ReasonError("paid content receipt must include buyer_did".to_string())
        })?
        .to_string();
    let seller_did = receipt_value
        .get("seller_did")
        .or_else(|| receipt_value.get("seller"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .or_else(|| record.owner_did.as_deref())
        .ok_or_else(|| {
            RPCErrors::ReasonError("paid content receipt must include seller_did".to_string())
        })?
        .to_string();
    let signature = receipt_value
        .get("signature")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            RPCErrors::ReasonError("paid content receipt must include signature".to_string())
        })?
        .to_string();

    Ok(ReceiptRecord {
        receipt_id,
        content_name,
        buyer_did,
        seller_did,
        signature,
        receipt_data: receipt_value,
        created_at: buckyos_get_unix_timestamp(),
    })
}

async fn create_ready_check_store_mgr(
    store_root: &Path,
) -> std::result::Result<NamedStoreMgr, RPCErrors> {
    let store = NamedLocalStore::get_named_store_by_path(store_root.to_path_buf())
        .await
        .map_err(|err| {
            RPCErrors::ReasonError(format!(
                "open ready check named store failed at {}: {err}",
                store_root.display()
            ))
        })?;
    let store_id = store.store_id().to_string();
    let store_ref = Arc::new(tokio::sync::Mutex::new(store));

    let store_mgr = NamedStoreMgr::new();
    store_mgr.register_store(store_ref).await;
    store_mgr
        .add_layout(StoreLayout::new(
            1,
            vec![StoreTarget {
                store_id,
                device_did: None,
                capacity: None,
                used: None,
                readonly: false,
                enabled: true,
                weight: 1,
            }],
            0,
            0,
        ))
        .await;
    Ok(store_mgr)
}

fn sanitize_content_id(content_id: &str) -> String {
    content_id.replace(['/', ':'], "_")
}

#[cfg(test)]
mod tests {
    use super::*;

    use buckyos_api::{RepoClient, RepoListFilter};
    use name_lib::DID;
    use package_lib::PackageMeta;
    use tempfile::TempDir;

    async fn test_service() -> (TempDir, RepoService, RepoClient) {
        let dir = tempfile::tempdir().expect("create temp dir");
        let service = RepoService::new(dir.path().to_path_buf())
            .await
            .expect("init repo service");
        let client = RepoClient::new_in_process(Box::new(service.clone()));
        (dir, service, client)
    }

    async fn write_file(path: &Path, content: &[u8]) {
        fs::write(path, content).await.expect("write file");
    }

    #[tokio::test]
    async fn store_creates_local_pinned_record() {
        let (_dir, service, client) = test_service().await;
        let source_dir = tempfile::tempdir().expect("create source dir");
        let file_path = source_dir.path().join("demo.tar.gz");
        write_file(&file_path, b"repo-service-store").await;
        let store_mgr = service
            .get_named_store_mgr()
            .await
            .expect("load named store");
        let file_template = FileObject::default();
        let (file_object, _file_obj_id, _file_obj_str) = cacl_file_object(
            Some(&store_mgr),
            &file_path,
            &file_template,
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .expect("store content into named store");
        let content_id = parse_obj_id(&file_object.content).expect("parse content id");

        let stored_id = client
            .store(&content_id.to_string())
            .await
            .expect("store content");
        let records = client
            .list(Some(RepoListFilter::new(
                Some(REPO_STATUS_PINNED.to_string()),
                Some(REPO_ORIGIN_LOCAL.to_string()),
                None,
                None,
            )))
            .await
            .expect("list records");

        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(stored_id, content_id);
        assert_eq!(record.content_id, content_id.to_string());
        assert_eq!(record.status, REPO_STATUS_PINNED);
        assert_eq!(record.origin, REPO_ORIGIN_LOCAL);

        let (_reader, stored_size) = store_mgr
            .open_reader(&content_id, None)
            .await
            .expect("open stored content");
        assert_eq!(stored_size, b"repo-service-store".len() as u64);
    }

    #[tokio::test]
    async fn store_promotes_existing_record_to_local_via_pin_path() {
        let (_dir, service, client) = test_service().await;
        let content_dir = tempfile::tempdir().expect("create content dir");
        let content_path = content_dir.path().join("pkg.tar.gz");
        write_file(&content_path, b"repo-service-store-existing").await;

        let store_mgr = service
            .get_named_store_mgr()
            .await
            .expect("load named store");
        let file_template = FileObject::default();
        let (file_object, _file_obj_id, _file_obj_str) = cacl_file_object(
            Some(&store_mgr),
            &content_path,
            &file_template,
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .expect("store content into named store");
        let content_id = parse_obj_id(&file_object.content).expect("parse content id");

        let owner = DID::from_str("did:bns:publisher").expect("parse owner did");
        let mut meta = PackageMeta::new("existing-app", "1.0.0", "publisher", &owner, None);
        meta.content = content_id.to_string();
        meta.size = file_object.size;
        client
            .collect(
                serde_json::to_value(meta.clone()).expect("serialize meta"),
                None,
            )
            .await
            .expect("collect content");

        let stored_id = client
            .store(&content_id.to_string())
            .await
            .expect("store content");
        assert_eq!(stored_id, content_id);

        let records = client
            .list(Some(RepoListFilter::new(
                None,
                None,
                Some("existing-app".to_string()),
                None,
            )))
            .await
            .expect("list records");
        assert_eq!(records.len(), 1);
        let record = &records[0];
        assert_eq!(record.content_id, content_id.to_string());
        assert_eq!(record.status, REPO_STATUS_PINNED);
        assert_eq!(record.origin, REPO_ORIGIN_LOCAL);
    }

    #[tokio::test]
    async fn collect_pin_and_serve_round_trip() {
        let (_dir, service, client) = test_service().await;
        let source_dir = tempfile::tempdir().expect("create source dir");
        let content_path = source_dir.path().join("pkg.tar.gz");
        write_file(&content_path, b"repo-service-collect-pin").await;

        let store_mgr = service
            .get_named_store_mgr()
            .await
            .expect("load named store");
        let file_template = FileObject::default();
        let (file_object, _file_obj_id, _file_obj_str) = cacl_file_object(
            Some(&store_mgr),
            &content_path,
            &file_template,
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .expect("store content into named store");
        let content_id = parse_obj_id(&file_object.content).expect("parse content id");
        let size = file_object.size;

        let owner = DID::from_str("did:bns:publisher").expect("parse owner did");
        let mut meta = PackageMeta::new("shared-app", "2.0.0", "publisher", &owner, None);
        meta.content = content_id.to_string();
        meta.size = size;
        let meta_value = serde_json::to_value(meta).expect("serialize package meta");

        let referral = ActionObject {
            subject: build_actor_obj_id("did:bns:alice"),
            action: ACTION_TYPE_SHARED.to_string(),
            target: content_id.clone(),
            base_on: None,
            details: Some(json!({"subject_did":"did:bns:alice"})),
            iat: buckyos_get_unix_timestamp(),
            exp: buckyos_get_unix_timestamp() + 3600,
        };
        client
            .collect(meta_value, Some(referral.clone()))
            .await
            .expect("collect content");

        let download = ActionObject {
            subject: build_actor_obj_id("did:bns:bob"),
            action: ACTION_TYPE_DOWNLOAD.to_string(),
            target: content_id.clone(),
            base_on: Some(referral.gen_obj_id().0),
            details: Some(json!({"subject_did":"did:bns:bob"})),
            iat: buckyos_get_unix_timestamp(),
            exp: buckyos_get_unix_timestamp() + 3600,
        };
        client
            .pin(&content_id.to_string(), download.clone())
            .await
            .expect("pin content");

        let proofs = client
            .get_proofs(&content_id.to_string(), None)
            .await
            .expect("get proofs");
        assert_eq!(proofs.len(), 2);

        let serve_result = client
            .serve(
                &content_id.to_string(),
                RepoServeRequestContext::new(
                    Some("did:bns:charlie".to_string()),
                    Some("device-1".to_string()),
                    None,
                    Value::Null,
                ),
            )
            .await
            .expect("serve content");
        assert_eq!(serve_result.status, "ok");
        assert_eq!(
            serve_result
                .content_ref
                .as_ref()
                .map(|content_ref| content_ref.content_id.clone()),
            Some(content_id.to_string())
        );

        let proofs = client
            .get_proofs(
                &content_id.to_string(),
                Some(RepoProofFilter::new(
                    Some(REPO_PROOF_TYPE_DOWNLOAD.to_string()),
                    None,
                    None,
                    None,
                    None,
                )),
            )
            .await
            .expect("get filtered proofs");
        assert_eq!(proofs.len(), 2);
    }
}
