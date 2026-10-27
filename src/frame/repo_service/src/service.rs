use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use crate::repo_db::{
    RepoDb, RepoDbStat, RepoObjectRecord as DbRecord, RepoProofRecord,
    RepoReceiptRecord as ReceiptRecord,
};
use anyhow::{Context, Result};
use async_trait::async_trait;
use buckyos_api::bucket_client::{now_ms, Conditions, ListRequest, PutRequest};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, is_precondition_failed,
    repo_service_packages_bucket, set_buckyos_api_runtime, BucketClient, BuckyOSRuntimeType,
    RepoActionProof, RepoContentRef, RepoHandler, RepoListFilter, RepoProof, RepoProofFilter,
    RepoRecord, RepoServeRequestContext, RepoServeResult, RepoServerHandler, RepoStat,
    REPO_ACCESS_POLICY_FREE, REPO_ACCESS_POLICY_PAID, REPO_ORIGIN_LOCAL, REPO_ORIGIN_REMOTE,
    REPO_PROOF_TYPE_COLLECTION, REPO_PROOF_TYPE_DOWNLOAD, REPO_PROOF_TYPE_INSTALL,
    REPO_PROOF_TYPE_REFERRAL, REPO_SERVE_REJECT_NOT_FOUND, REPO_SERVE_REJECT_NO_RECEIPT,
    REPO_SERVICE_SERVICE_NAME, REPO_SERVICE_SERVICE_PORT, REPO_STATUS_COLLECTED,
    REPO_STATUS_PINNED,
};
use buckyos_http_server::Runner;
use buckyos_http_server::{
    serve_http_by_rpc_handler, server_err, HttpServer, ServerError, ServerErrorCode, ServerResult,
    StreamInfo,
};
use buckyos_kit::{buckyos_get_unix_timestamp, init_logging};
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use kRPC::{RPCContext, RPCErrors, RPCHandler, RPCRequest, RPCResponse};
use log::{info, warn};
use name_lib::decode_jwt_claim_without_verify;
use named_store::NamedDataMgr;
use ndn_lib::{
    build_obj_id, load_named_obj, load_named_object_from_obj_str, verify_named_object,
    ActionObject, ChunkId, ChunkList, DirObject, FileObject, InclusionProof, NamedObject, ObjId,
    ACTION_TYPE_DOWNLOAD, ACTION_TYPE_INSTALLED, ACTION_TYPE_SHARED, OBJ_TYPE_CHUNK_LIST,
    OBJ_TYPE_DIR, OBJ_TYPE_FILE,
};
use serde_json::{json, Value};

const RECONCILE_GRACE_MS: i64 = 10 * 60 * 1000;

/// Where content is read from and who retains it: the node named store and the
/// zone bucket `zone/repo-service/_/packages`.
#[derive(Clone)]
pub enum RepoStorage {
    Runtime,
    #[cfg(test)]
    Fixed {
        store: NamedDataMgr,
        buckets: Arc<BucketClient>,
    },
}

impl RepoStorage {
    async fn store(&self) -> std::result::Result<NamedDataMgr, RPCErrors> {
        match self {
            RepoStorage::Runtime => get_buckyos_api_runtime()
                .map_err(to_rpc_error)?
                .get_local_named_store()
                .await
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("open system named store failed: {err}"))
                }),
            #[cfg(test)]
            RepoStorage::Fixed { store, .. } => Ok(store.clone()),
        }
    }

    async fn buckets(&self) -> std::result::Result<Arc<BucketClient>, RPCErrors> {
        match self {
            RepoStorage::Runtime => get_buckyos_api_runtime()
                .map_err(to_rpc_error)?
                .get_bucket_client()
                .await
                .map(Arc::new)
                .map_err(|err| {
                    RPCErrors::ReasonError(format!("bucket service unavailable: {err}"))
                }),
            #[cfg(test)]
            RepoStorage::Fixed { buckets, .. } => Ok(buckets.clone()),
        }
    }
}

#[derive(Clone)]
pub struct RepoService {
    db: Arc<RepoDb>,
    storage: RepoStorage,
}

impl RepoService {
    pub fn new(db: Arc<RepoDb>, storage: RepoStorage) -> Self {
        Self { db, storage }
    }

    /// Size (and object JSON) of a content that is complete in the named store.
    /// Only what a data bucket retains is accepted: a file, a dir, or raw chunk /
    /// chunk list content.
    async fn inspect_local_content(
        &self,
        content_id: &ObjId,
    ) -> std::result::Result<(u64, Option<Value>), RPCErrors> {
        let store_mgr = self.storage.store().await?;
        if !store_mgr
            .is_object_stored(content_id, None)
            .await
            .unwrap_or(false)
        {
            return Err(RPCErrors::ReasonError(format!(
                "content {} is not available in named store",
                content_id
            )));
        }
        if content_id.is_chunk() {
            let size = ChunkId::from_obj_id(content_id)
                .get_length()
                .ok_or_else(|| {
                    RPCErrors::ReasonError(format!("chunk {} has no encoded length", content_id))
                })?;
            return Ok((size, None));
        }

        let object_str = store_mgr.get_object(content_id).await.map_err(|err| {
            RPCErrors::ReasonError(format!(
                "load published content {} from named store failed: {err}",
                content_id
            ))
        })?;
        let parse_error = |err: ndn_lib::NdnError| {
            RPCErrors::ReasonError(format!(
                "parse published content {} from named store failed: {err}",
                content_id
            ))
        };
        let size = match content_id.obj_type.as_str() {
            OBJ_TYPE_FILE => {
                load_named_obj::<FileObject>(&object_str)
                    .map_err(parse_error)?
                    .size
            }
            OBJ_TYPE_DIR => {
                load_named_obj::<DirObject>(&object_str)
                    .map_err(parse_error)?
                    .total_size
            }
            OBJ_TYPE_CHUNK_LIST => {
                ChunkList::from_json(&object_str)
                    .map_err(parse_error)?
                    .total_size
            }
            other => {
                return Err(RPCErrors::ReasonError(format!(
                    "content {} is a {other}; repo-service keeps files, dirs and chunk content",
                    content_id
                )))
            }
        };
        let object_json = serde_json::from_str::<Value>(object_str.as_str())
            .or_else(|_| load_named_object_from_obj_str(object_str.as_str()))
            .map_err(parse_error)?;
        Ok((size, Some(object_json)))
    }

    /// Binds a pinned content in `zone/repo-service/_/packages`: a file or dir as
    /// itself, raw chunk content wrapped in a FileObject named after the package.
    /// The key is never rebound to other content; binding the same object again
    /// is accepted, so a retry replays. Returns whether this call created the key.
    async fn bind_content(
        &self,
        key: &str,
        content_id: &ObjId,
        name: &str,
        size: u64,
    ) -> std::result::Result<bool, RPCErrors> {
        let buckets = self.storage.buckets().await?;
        let bucket = repo_service_packages_bucket();
        let (obj_id, put) = if content_id.is_chunk() || content_id.is_chunk_list() {
            let (obj_id, obj_str) =
                FileObject::new(name.to_string(), size, content_id.to_string()).gen_obj_id();
            let store = self.storage.store().await?;
            let put = buckets
                .put_object_str(
                    &store,
                    &bucket,
                    key,
                    &obj_id,
                    &obj_str,
                    Conditions::create_only(),
                )
                .await;
            (obj_id, put)
        } else {
            let req = PutRequest {
                key: key.to_string(),
                obj_id: content_id.to_string(),
                conditions: Conditions::create_only(),
                ..Default::default()
            };
            (content_id.clone(), buckets.put(&bucket, &req).await)
        };
        match put {
            Ok(result) => Ok(result.created),
            Err(err) if is_precondition_failed(&err) => {
                let bound = buckets.get(&bucket, key, None).await?;
                if bound.obj_id.as_deref() == Some(obj_id.to_string().as_str()) {
                    Ok(false)
                } else {
                    Err(RPCErrors::ReasonError(format!(
                        "{key} is already pinned with other content ({}); unpin it first",
                        bound.obj_id.unwrap_or_default()
                    )))
                }
            }
            Err(err) => Err(err),
        }
    }

    /// Releases a binding; a failure leaves extra retention that reconciliation removes.
    async fn unbind(&self, key: &str) {
        let result = match self.storage.buckets().await {
            Ok(buckets) => buckets
                .delete(&repo_service_packages_bucket(), key, None)
                .await
                .map(|_| ()),
            Err(err) => Err(err),
        };
        if let Err(err) = result {
            warn!("unbind repo package {key} failed, left to reconciliation: {err}");
        }
    }

    /// Design §15.1 reconciliation of `objects` with the packages bucket: keys no
    /// pinned row references are unbound (unless bound within `grace_ms`, which
    /// may be a pin still writing its row); a pinned row whose binding is gone is
    /// bound again while its content is still in the named store.
    pub async fn reconcile(&self, grace_ms: i64) -> std::result::Result<(), RPCErrors> {
        let buckets = self.storage.buckets().await?;
        let bucket = repo_service_packages_bucket();
        let records = self.load_all_records().await?;
        let referenced: HashSet<String> = records
            .iter()
            .filter(|record| record.status == REPO_STATUS_PINNED)
            .filter_map(|record| record.bucket_key.clone())
            .collect();
        let cutoff = now_ms() - grace_ms;
        let mut bound = HashSet::new();
        let mut start_after = None;
        loop {
            let page = buckets
                .list_objects(
                    &bucket,
                    &ListRequest {
                        start_after: start_after.take(),
                        limit: Some(1000),
                        ..Default::default()
                    },
                )
                .await?;
            for item in page.items {
                if referenced.contains(&item.key) {
                    bound.insert(item.key);
                } else if item.created_at <= cutoff {
                    self.unbind(&item.key).await;
                }
            }
            match page.next_start_after {
                Some(next) => start_after = Some(next),
                None => break,
            }
        }
        for mut record in records {
            if record.status != REPO_STATUS_PINNED
                || record
                    .bucket_key
                    .as_ref()
                    .is_some_and(|key| bound.contains(key))
            {
                continue;
            }
            let content_id = parse_obj_id(&record.content_id)?;
            let key = package_key(&record);
            let rebound = match self.inspect_local_content(&content_id).await {
                Ok((size, _)) => {
                    self.bind_content(&key, &content_id, &package_name(&record), size)
                        .await
                }
                Err(err) => Err(err),
            };
            match rebound {
                Ok(_) if record.bucket_key.as_deref() != Some(key.as_str()) => {
                    record.bucket_key = Some(key);
                    self.write_object_record(record).await?;
                }
                Ok(_) => {}
                Err(err) => warn!(
                    "pinned repo content {} has no binding and cannot be bound again: {err}",
                    record.content_id
                ),
            }
        }
        Ok(())
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

    /// Bind first, then write the row (design §15.1): a failed row write deletes
    /// the binding this call created; a key the row no longer uses is released.
    async fn pin_record(
        &self,
        mut record: DbRecord,
        content_id: &ObjId,
        content_size: u64,
        download_proof: Option<RepoActionProof>,
    ) -> std::result::Result<(), RPCErrors> {
        if let Some(download_proof) = download_proof {
            validate_action_target(&download_proof, content_id, ACTION_TYPE_DOWNLOAD)?;
            self.save_proof(RepoProof::action(download_proof)).await?;
        }

        let key = package_key(&record);
        let created = self
            .bind_content(&key, content_id, &package_name(&record), content_size)
            .await?;
        let previous = record
            .bucket_key
            .replace(key.clone())
            .filter(|old| old != &key);
        let now = buckyos_get_unix_timestamp();
        record.status = REPO_STATUS_PINNED.to_string();
        record.content_size = Some(content_size);
        record.pinned_at = Some(now);
        record.updated_at = Some(now);
        record.collected_at = record.collected_at.or(Some(now));
        if let Err(err) = self.write_object_record(record).await {
            if created {
                self.unbind(&key).await;
            }
            return Err(err);
        }
        if let Some(previous) = previous {
            self.unbind(&previous).await;
        }
        Ok(())
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
        let (content_size, inspected_meta) = self.inspect_local_content(&content_id).await?;
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
            bucket_key: existing.as_ref().and_then(|row| row.bucket_key.clone()),
            collected_at: existing.as_ref().and_then(|row| row.collected_at),
            pinned_at: existing.as_ref().and_then(|row| row.pinned_at),
            updated_at: existing.as_ref().and_then(|row| row.updated_at),
        };
        self.pin_record(record, &content_id, content_size, None)
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
            bucket_key: existing.as_ref().and_then(|row| row.bucket_key.clone()),
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
        let (content_size, _) = self.inspect_local_content(&content_id).await?;
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
        let key = record.bucket_key.clone();
        let updated = DbRecord {
            status: REPO_STATUS_COLLECTED.to_string(),
            pinned_at: None,
            bucket_key: None,
            updated_at: Some(now),
            ..record
        };
        self.write_object_record(updated).await?;
        if let Some(key) = key {
            self.unbind(&key).await;
        }
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
        if let Err(err) = self.inspect_local_content(&content_id).await {
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
        self.db
            .upsert_announce(
                content_id,
                record.content_name.as_deref(),
                &record.meta,
                buckyos_get_unix_timestamp(),
            )
            .await
            .map_err(to_rpc_error)?;
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
    set_buckyos_api_runtime(runtime).context("register repo-service runtime failed")?;

    let db = Arc::new(
        RepoDb::open_from_service_spec()
            .await
            .map_err(|err| anyhow::anyhow!("init repo-db failed: {}", err))?,
    );
    info!("repo-service database initialized");
    let service = RepoService::new(db, RepoStorage::Runtime);
    let reconciler = service.clone();
    tokio::spawn(async move {
        while let Err(err) = reconciler.reconcile(RECONCILE_GRACE_MS).await {
            warn!("repo-service package reconciliation failed, retrying in 60s: {err}");
            tokio::time::sleep(Duration::from_secs(60)).await;
        }
    });
    let server = Arc::new(RepoHttpServer::new(service));
    let runner = Runner::new(REPO_SERVICE_SERVICE_PORT);
    runner
        .add_http_server("/kapi/repo-service".to_string(), server.clone())
        .context("register /kapi/repo-service failed")?;
    runner
        .add_http_server("/kapi/repo".to_string(), server)
        .context("register /kapi/repo failed")?;

    info!("repo-service started: port={}", REPO_SERVICE_SERVICE_PORT);
    runner.run().await.context("repo-service runner failed")
}

fn to_rpc_error<E: std::fmt::Display>(err: E) -> RPCErrors {
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
        .or(record.owner_did.as_deref())
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

/// Bucket key of a pinned content: `<content_name>/<version>`, or `<content_name>`
/// for content whose meta has no version.
fn package_key(record: &DbRecord) -> String {
    let name = package_name(record);
    match record.meta.get("version").and_then(Value::as_str) {
        Some(version) => format!("{name}/{version}"),
        None => name,
    }
}

fn package_name(record: &DbRecord) -> String {
    record
        .content_name
        .clone()
        .unwrap_or_else(|| record.content_id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    use buckyos_api::{
        is_no_such_key, repo_service_bucket_decls, RdbBackend, RepoClient, RepoListFilter,
        REPO_SERVICE_STORAGE_QUOTA_MIB,
    };
    use name_lib::DID;
    use ndn_lib::{FileObject, StoreMode};
    use ndn_toolkit::{cacl_file_object, CheckMode};
    use nfs_server::config::{NamedStoreSource, ServerConfig};
    use package_lib::PackageMeta;
    use std::path::Path;
    use tempfile::TempDir;

    struct TestRepo {
        _dir: TempDir,
        service: RepoService,
        client: RepoClient,
        store: NamedDataMgr,
        buckets: Arc<BucketClient>,
    }

    /// A repo-service whose bucket service is a standalone nfs-server (no auth)
    /// declaring `zone/repo-service/_/packages`, sharing its named store.
    async fn test_service() -> TestRepo {
        let dir = tempfile::tempdir().expect("create temp dir");
        let exports = dir.path().join("exports");
        std::fs::create_dir_all(&exports).expect("create exports");
        let mut config = ServerConfig::new(
            dir.path().join("fsdb"),
            vec![nfs_server::ExportRoot {
                id: "home".into(),
                path: exports,
            }],
        );
        config.named_store = NamedStoreSource::Dir(dir.path().join("store"));
        config.bucket_specs.insert(
            REPO_SERVICE_SERVICE_NAME.to_string(),
            nfs_server::bucket::spec::AppBuckets {
                buckets: repo_service_bucket_decls(),
                storage_quota_mib: Some(REPO_SERVICE_STORAGE_QUOTA_MIB),
                ..Default::default()
            },
        );
        let state = nfs_server::AppState::new(config).expect("nfs-server state");
        let store = state
            .share
            .store(&state.config)
            .await
            .expect("named store")
            .clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind nfs-server");
        let base = format!("http://{}", listener.local_addr().unwrap());
        let app = nfs_server::build_router(state);
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let buckets = Arc::new(BucketClient::new(kRPC::kRPC::new(
            &format!("{base}/kapi/nfs-server"),
            None,
        )));

        let conn = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("repo.db").to_str().unwrap()
        );
        let db = Arc::new(
            RepoDb::open(&conn, RdbBackend::Sqlite, None)
                .await
                .expect("open test repo db"),
        );
        let service = RepoService::new(
            db,
            RepoStorage::Fixed {
                store: store.clone(),
                buckets: buckets.clone(),
            },
        );
        let client = RepoClient::new_in_process(Box::new(service.clone()));
        TestRepo {
            _dir: dir,
            service,
            client,
            store,
            buckets,
        }
    }

    /// Stores `content` in the named store; returns its FileObject and raw content id.
    async fn put_content(store: &NamedDataMgr, name: &str, content: &[u8]) -> (FileObject, ObjId) {
        let source_dir = tempfile::tempdir().expect("create source dir");
        let path = source_dir.path().join(name);
        tokio::fs::write(&path, content).await.expect("write file");
        let (file_object, _file_obj_id, _file_obj_str) = cacl_file_object(
            Some(store),
            Path::new(&path),
            &FileObject::default(),
            true,
            &CheckMode::ByFullHash,
            StoreMode::StoreInNamedMgr,
            None,
        )
        .await
        .expect("store content into named store");
        let content_id = parse_obj_id(&file_object.content).expect("parse content id");
        (file_object, content_id)
    }

    fn package_meta(name: &str, version: &str, file: &FileObject) -> Value {
        let owner = DID::from_str("did:bns:publisher").expect("parse owner did");
        let mut meta = PackageMeta::new(name, version, "publisher", &owner, None);
        meta.content = file.content.clone();
        meta.size = file.size;
        serde_json::to_value(meta).expect("serialize package meta")
    }

    fn download(content_id: &ObjId, who: &str) -> ActionObject {
        ActionObject {
            subject: build_actor_obj_id(who),
            action: ACTION_TYPE_DOWNLOAD.to_string(),
            target: content_id.clone(),
            base_on: None,
            details: Some(json!({"subject_did": who})),
            iat: buckyos_get_unix_timestamp(),
            exp: buckyos_get_unix_timestamp() + 3600,
        }
    }

    async fn binding(repo: &TestRepo, key: &str) -> std::result::Result<FileObject, RPCErrors> {
        let bound = repo
            .buckets
            .get(&repo_service_packages_bucket(), key, None)
            .await?;
        let obj_id = parse_obj_id(bound.obj_id.as_deref().unwrap()).unwrap();
        Ok(load_named_obj(&repo.store.get_object(&obj_id).await.unwrap()).unwrap())
    }

    #[tokio::test]
    async fn store_creates_local_pinned_record() {
        let repo = test_service().await;
        let (_, content_id) = put_content(&repo.store, "demo.tar.gz", b"repo-service-store").await;

        let stored_id = repo
            .client
            .store(&content_id.to_string())
            .await
            .expect("store content");
        let records = repo
            .client
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

        let wrapper = binding(&repo, &content_id.to_string())
            .await
            .expect("content is bound under its id");
        assert_eq!(wrapper.content, content_id.to_string());
        assert_eq!(wrapper.size, b"repo-service-store".len() as u64);
        assert!(repo
            .client
            .unpin(&content_id.to_string(), false)
            .await
            .is_err());
        repo.client
            .unpin(&content_id.to_string(), true)
            .await
            .expect("force unpin");
        let gone = binding(&repo, &content_id.to_string()).await.unwrap_err();
        assert!(is_no_such_key(&gone));
    }

    #[tokio::test]
    async fn store_promotes_existing_record_to_local_via_pin_path() {
        let repo = test_service().await;
        let (file_object, content_id) =
            put_content(&repo.store, "pkg.tar.gz", b"repo-service-store-existing").await;
        repo.client
            .collect(package_meta("existing-app", "1.0.0", &file_object), None)
            .await
            .expect("collect content");

        let stored_id = repo
            .client
            .store(&content_id.to_string())
            .await
            .expect("store content");
        assert_eq!(stored_id, content_id);

        let records = repo
            .client
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
        let wrapper = binding(&repo, "existing-app/1.0.0")
            .await
            .expect("package bound by name and version");
        assert_eq!(wrapper.name, "existing-app");
        assert_eq!(wrapper.content, content_id.to_string());
    }

    #[tokio::test]
    async fn collect_pin_and_serve_round_trip() {
        let repo = test_service().await;
        let (file_object, content_id) =
            put_content(&repo.store, "pkg.tar.gz", b"repo-service-collect-pin").await;

        let referral = ActionObject {
            subject: build_actor_obj_id("did:bns:alice"),
            action: ACTION_TYPE_SHARED.to_string(),
            target: content_id.clone(),
            base_on: None,
            details: Some(json!({"subject_did":"did:bns:alice"})),
            iat: buckyos_get_unix_timestamp(),
            exp: buckyos_get_unix_timestamp() + 3600,
        };
        repo.client
            .collect(
                package_meta("shared-app", "2.0.0", &file_object),
                Some(referral.clone()),
            )
            .await
            .expect("collect content");
        assert!(is_no_such_key(
            &binding(&repo, "shared-app/2.0.0").await.unwrap_err()
        ));

        let mut download = download(&content_id, "did:bns:bob");
        download.base_on = Some(referral.gen_obj_id().0);
        repo.client
            .pin(&content_id.to_string(), download.clone())
            .await
            .expect("pin content");
        binding(&repo, "shared-app/2.0.0")
            .await
            .expect("pinned package is bound");

        let proofs = repo
            .client
            .get_proofs(&content_id.to_string(), None)
            .await
            .expect("get proofs");
        assert_eq!(proofs.len(), 2);

        let serve_result = repo
            .client
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

        let proofs = repo
            .client
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

        repo.client
            .uncollect(&content_id.to_string(), false)
            .await
            .expect("uncollect remote content");
        assert!(is_no_such_key(
            &binding(&repo, "shared-app/2.0.0").await.unwrap_err()
        ));
    }

    #[tokio::test]
    async fn same_package_version_with_other_content_is_refused() {
        let repo = test_service().await;
        let (first_file, first) = put_content(&repo.store, "a.tar.gz", b"first build").await;
        let (second_file, second) = put_content(&repo.store, "b.tar.gz", b"second build").await;
        for file in [&first_file, &second_file] {
            repo.client
                .collect(package_meta("dup-app", "1.0.0", file), None)
                .await
                .expect("collect");
        }
        repo.client
            .pin(&first.to_string(), download(&first, "did:bns:bob"))
            .await
            .expect("pin first");
        repo.client
            .pin(&first.to_string(), download(&first, "did:bns:bob"))
            .await
            .expect("pinning again replays");
        assert!(repo
            .client
            .pin(&second.to_string(), download(&second, "did:bns:bob"))
            .await
            .is_err());
        let second_record = repo
            .service
            .db
            .get_object(&second.to_string())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(second_record.status, REPO_STATUS_COLLECTED);
        assert_eq!(
            binding(&repo, "dup-app/1.0.0").await.unwrap().content,
            first.to_string()
        );
    }

    #[tokio::test]
    async fn reconcile_matches_the_bucket_to_pinned_rows() {
        let repo = test_service().await;
        let (file_object, content_id) = put_content(&repo.store, "pkg.tar.gz", b"reconciled").await;
        repo.client
            .collect(package_meta("kept-app", "3.0.0", &file_object), None)
            .await
            .expect("collect");
        repo.client
            .pin(
                &content_id.to_string(),
                download(&content_id, "did:bns:bob"),
            )
            .await
            .expect("pin");
        let bucket = repo_service_packages_bucket();
        let (orphan_file, _) = put_content(&repo.store, "orphan.tar.gz", b"orphan").await;
        let (orphan_id, orphan_str) = orphan_file.gen_obj_id();
        repo.buckets
            .put_object_str(
                &repo.store,
                &bucket,
                "orphan/1.0.0",
                &orphan_id,
                &orphan_str,
                Conditions::default(),
            )
            .await
            .expect("bind orphan");
        repo.buckets
            .delete(&bucket, "kept-app/3.0.0", None)
            .await
            .expect("drop pinned binding");

        repo.service.reconcile(0).await.expect("reconcile");
        assert!(is_no_such_key(
            &binding(&repo, "orphan/1.0.0").await.unwrap_err()
        ));
        assert_eq!(
            binding(&repo, "kept-app/3.0.0").await.unwrap().content,
            content_id.to_string()
        );
    }

    #[tokio::test]
    async fn content_missing_from_the_store_is_not_pinned() {
        let repo = test_service().await;
        let (file_object, content_id) = put_content(&repo.store, "pkg.tar.gz", b"missing").await;
        let mut meta = package_meta("missing-app", "1.0.0", &file_object);
        let (fake, _) = FileObject::new("x".into(), 1, content_id.to_string()).gen_obj_id();
        meta["content"] = json!(fake.to_string());
        repo.client.collect(meta, None).await.expect("collect");
        assert!(repo
            .client
            .pin(&fake.to_string(), download(&fake, "did:bns:bob"))
            .await
            .is_err());
        assert!(is_no_such_key(
            &binding(&repo, "missing-app/1.0.0").await.unwrap_err()
        ));
    }
}
