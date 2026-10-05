use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::msg_queue::*;
use buckyos_http_server::{
    HttpServer, ServerError, ServerErrorCode, ServerResult, StreamInfo, serve_http_by_rpc_handler,
    server_err,
};
use buckyos_kit::get_buckyos_service_local_data_dir;
use bytes::Bytes;
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use serde::{Deserialize, Serialize};
use sled::{Db, IVec, Tree, transaction::Transactional};
use log::{info, warn};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::task;

pub struct SledMsgQueueServer {
    handler: MsgQueueServerHandler<SledMsgQueue>,
}

/// How often messages older than a queue's `retention_seconds` are swept
/// when nothing is posted to it.
const RETENTION_SWEEP_INTERVAL: Duration = Duration::from_secs(60);

impl SledMsgQueueServer {
    pub fn new() -> Self {
        let queue = SledMsgQueue::new().expect("Failed to open kmsg sled database");
        if tokio::runtime::Handle::try_current().is_ok() {
            let sweeper = queue.clone();
            tokio::spawn(async move {
                loop {
                    tokio::time::sleep(RETENTION_SWEEP_INTERVAL).await;
                    let q = sweeper.clone();
                    match task::spawn_blocking(move || q.sweep_retention()).await {
                        Ok(Ok(0)) => {}
                        Ok(Ok(n)) => info!("kmsg retention sweep removed {} messages", n),
                        Ok(Err(err)) => warn!("kmsg retention sweep failed: {}", err),
                        Err(err) => warn!("kmsg retention sweep panicked: {}", err),
                    }
                }
            });
        }
        Self {
            handler: MsgQueueServerHandler::new(queue),
        }
    }
}

#[async_trait]
impl RPCHandler for SledMsgQueueServer {
    async fn handle_rpc_call(
        &self,
        req: RPCRequest,
        ip_from: std::net::IpAddr,
    ) -> std::result::Result<RPCResponse, RPCErrors> {
        let req_seq = req.seq;
        let req_trace_id = req.trace_id.clone();
        match self.handler.handle_rpc_call(req, ip_from).await {
            Ok(resp) => Ok(resp),
            Err(err) => Ok(RPCResponse {
                result: RPCResult::Failed(err.to_string()),
                seq: req_seq,
                trace_id: req_trace_id,
            }),
        }
    }
}

#[async_trait]
impl HttpServer for SledMsgQueueServer {
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
        "kmsg-server".to_string()
    }

    fn http_version(&self) -> Version {
        Version::HTTP_11
    }

    fn http3_port(&self) -> Option<u16> {
        None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct QueueMeta {
    next_index: MsgIndex,
    message_count: u64,
    first_index: MsgIndex,
    last_index: MsgIndex,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubscriptionState {
    queue_urn: QueueUrn,
    cursor: MsgIndex,
}

#[derive(Clone)]
pub struct SledMsgQueue {
    db: Arc<Db>,
    queues: Tree,
    queue_meta: Tree,
    messages: Tree,
    subs: Tree,
    /// `<queue_urn>\0<sub_id>` → empty: the subscriptions of each queue.
    queue_subs: Tree,
    meta: Tree,
}

/// Messages removed per transaction when trimming a queue.
const TRIM_CHUNK: u64 = 512;

fn storage_err(err: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(err.to_string())
}

impl SledMsgQueue {
    pub fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let data_path = get_buckyos_service_local_data_dir("kmsg");
        Self::new_in_dir(data_path)
    }

    pub fn new_in_dir<P: AsRef<Path>>(
        path: P,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let db = sled::open(path)?;
        let queue = Self {
            queues: db.open_tree("queues")?,
            queue_meta: db.open_tree("queue_meta")?,
            messages: db.open_tree("messages")?,
            subs: db.open_tree("subs")?,
            queue_subs: db.open_tree("queue_subs")?,
            meta: db.open_tree("meta")?,
            db: Arc::new(db),
        };
        // The index is derived from `subs`: rebuilt on open (databases
        // written before it existed, interrupted updates).
        queue.queue_subs.clear()?;
        for item in queue.subs.iter() {
            let (sub_id, value) = item?;
            if let Ok(sub) = serde_json::from_slice::<SubscriptionState>(&value) {
                queue.queue_subs.insert(
                    Self::queue_sub_key(&sub.queue_urn, &String::from_utf8_lossy(&sub_id)),
                    &[],
                )?;
            }
        }
        Ok(queue)
    }

    fn now_seconds() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    fn queue_key(queue_urn: &str) -> Vec<u8> {
        queue_urn.as_bytes().to_vec()
    }

    fn message_prefix(queue_urn: &str) -> Vec<u8> {
        let mut key = Vec::with_capacity(queue_urn.len() + 1);
        key.extend_from_slice(queue_urn.as_bytes());
        key.push(0u8);
        key
    }

    fn message_key(queue_urn: &str, index: MsgIndex) -> Vec<u8> {
        let mut key = Self::message_prefix(queue_urn);
        key.extend_from_slice(&index.to_be_bytes());
        key
    }

    fn queue_sub_key(queue_urn: &str, sub_id: &str) -> Vec<u8> {
        let mut key = Self::message_prefix(queue_urn);
        key.extend_from_slice(sub_id.as_bytes());
        key
    }

    /// Subscription ids of a queue (from the index).
    fn queue_sub_ids(&self, queue_urn: &str) -> std::result::Result<Vec<Vec<u8>>, RPCErrors> {
        let prefix = Self::message_prefix(queue_urn);
        let mut out = Vec::new();
        for item in self.queue_subs.scan_prefix(&prefix) {
            let (key, _) = item.map_err(storage_err)?;
            out.push(key[prefix.len()..].to_vec());
        }
        Ok(out)
    }

    /// Lowest cursor among the queue's subscriptions: every message below it
    /// is acknowledged by all of them. `None` without subscriptions.
    fn acked_floor(&self, queue_urn: &str) -> std::result::Result<Option<MsgIndex>, RPCErrors> {
        let mut floor: Option<MsgIndex> = None;
        for sub_id in self.queue_sub_ids(queue_urn)? {
            let Some(value) = self.subs.get(&sub_id).map_err(storage_err)? else {
                continue;
            };
            let sub: SubscriptionState = serde_json::from_slice(&value).map_err(storage_err)?;
            if sub.queue_urn != queue_urn {
                continue;
            }
            floor = Some(floor.map_or(sub.cursor, |f| f.min(sub.cursor)));
        }
        Ok(floor)
    }

    /// Remove every message with an index below `bound`, in transactions
    /// with the queue meta (a concurrent post is never lost, indexes are
    /// never reused). Only `first_index` / counters change.
    fn remove_before(&self, queue_urn: &str, bound: MsgIndex) -> std::result::Result<u64, RPCErrors> {
        let queue_key = Self::queue_key(queue_urn);
        let mut total = 0u64;
        loop {
            let (removed, done) = (&self.messages, &self.queue_meta)
                .transaction(|(messages, queue_meta)| {
                    let abort = |err: RPCErrors| sled::transaction::ConflictableTransactionError::Abort(err);
                    let meta_value = queue_meta.get(&queue_key)?.ok_or_else(|| {
                        abort(RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn)))
                    })?;
                    let mut meta = Self::decode_queue_meta(&meta_value).map_err(abort)?;
                    if meta.first_index == 0 || meta.first_index >= bound {
                        return Ok((0u64, true));
                    }
                    let stop = bound.min(meta.last_index + 1);
                    let end = stop.min(meta.first_index + TRIM_CHUNK);
                    let mut removed = 0u64;
                    let mut bytes = 0u64;
                    for index in meta.first_index..end {
                        if let Some(value) = messages.remove(Self::message_key(queue_urn, index))? {
                            removed += 1;
                            if let Ok(msg) = Self::decode_message(&value) {
                                bytes += msg.payload.len() as u64;
                            }
                        }
                    }
                    if end > meta.last_index {
                        meta.first_index = 0;
                        meta.message_count = 0;
                        meta.size_bytes = 0;
                    } else {
                        meta.first_index = end;
                        meta.message_count = meta.message_count.saturating_sub(removed);
                        meta.size_bytes = meta.size_bytes.saturating_sub(bytes);
                    }
                    queue_meta.insert(queue_key.clone(), Self::encode_queue_meta(&meta).map_err(abort)?)?;
                    Ok((removed, end >= stop))
                })
                .map_err(|err| match err {
                    sled::transaction::TransactionError::Abort(err) => err,
                    sled::transaction::TransactionError::Storage(err) => storage_err(err),
                })?;
            total += removed;
            if done {
                return Ok(total);
            }
        }
    }

    /// Unless `keep_acked`: drop what every subscription has acknowledged.
    fn trim_acked(&self, queue_urn: &str) -> std::result::Result<u64, RPCErrors> {
        let config = match self.get_queue_config(queue_urn) {
            Ok(config) => config,
            // Deleted meanwhile: nothing left to trim.
            Err(_) => return Ok(0),
        };
        if config.keep_acked {
            return Ok(0);
        }
        match self.acked_floor(queue_urn)? {
            Some(floor) => self.remove_before(queue_urn, floor),
            None => Ok(0),
        }
    }

    /// `max_messages` / `retention_seconds`: the oldest messages beyond the
    /// limits are dropped, consumed or not.
    fn enforce_limits(
        &self,
        queue_urn: &str,
        config: &QueueConfig,
    ) -> std::result::Result<u64, RPCErrors> {
        let mut removed = 0;
        if let Some(max) = config.max_messages.filter(|max| *max > 0) {
            let meta = self.get_queue_meta(queue_urn)?;
            if meta.first_index != 0 && meta.last_index + 1 - meta.first_index > max {
                removed += self.remove_before(queue_urn, meta.last_index + 1 - max)?;
            }
        }
        if let Some(seconds) = config.retention_seconds.filter(|s| *s > 0) {
            let cutoff = Self::now_seconds().saturating_sub(seconds);
            let meta = self.get_queue_meta(queue_urn)?;
            if meta.first_index != 0 {
                let start = Self::message_key(queue_urn, meta.first_index);
                let end = Self::message_key(queue_urn, u64::MAX);
                let mut bound = meta.last_index + 1;
                for item in self.messages.range(start..=end) {
                    let (_, value) = item.map_err(storage_err)?;
                    let msg = Self::decode_message(&value)?;
                    if msg.created_at >= cutoff {
                        bound = msg.index;
                        break;
                    }
                }
                removed += self.remove_before(queue_urn, bound)?;
            }
        }
        Ok(removed)
    }

    /// Apply `retention_seconds` to every queue declaring it.
    pub fn sweep_retention(&self) -> std::result::Result<u64, RPCErrors> {
        let mut removed = 0;
        for item in self.queues.iter() {
            let (key, value) = item.map_err(storage_err)?;
            let Ok(config) = Self::decode_queue_config(&value) else {
                continue;
            };
            if config.retention_seconds.filter(|s| *s > 0).is_none() {
                continue;
            }
            let queue_urn = String::from_utf8_lossy(&key).to_string();
            removed += self.enforce_limits(&queue_urn, &config)?;
        }
        Ok(removed)
    }

    fn decode_queue_config(value: &IVec) -> std::result::Result<QueueConfig, RPCErrors> {
        serde_json::from_slice(value).map_err(|err| {
            RPCErrors::ReasonError(format!("Failed to decode queue config: {}", err))
        })
    }

    fn decode_queue_meta(value: &IVec) -> std::result::Result<QueueMeta, RPCErrors> {
        serde_json::from_slice(value)
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to decode queue meta: {}", err)))
    }

    fn encode_queue_meta(meta: &QueueMeta) -> std::result::Result<Vec<u8>, RPCErrors> {
        serde_json::to_vec(meta)
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to encode queue meta: {}", err)))
    }

    fn decode_message(value: &IVec) -> std::result::Result<Message, RPCErrors> {
        serde_json::from_slice(value)
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to decode message: {}", err)))
    }

    #[allow(dead_code)]
    fn encode_message(message: &Message) -> std::result::Result<Vec<u8>, RPCErrors> {
        serde_json::to_vec(message)
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to encode message: {}", err)))
    }

    fn next_id(&self, key: &str) -> std::result::Result<u64, RPCErrors> {
        let key = key.as_bytes();
        loop {
            let current = self
                .meta
                .get(key)
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
            let value = match current.as_ref() {
                Some(bytes) => {
                    let mut buf = [0u8; 8];
                    buf.copy_from_slice(bytes.as_ref());
                    u64::from_be_bytes(buf)
                }
                None => 0,
            };
            let next = value + 1;
            let result = self
                .meta
                .compare_and_swap(key, current, Some(next.to_be_bytes().to_vec()))
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
            if result.is_ok() {
                return Ok(next);
            }
        }
    }

    fn get_queue_meta(&self, queue_urn: &str) -> std::result::Result<QueueMeta, RPCErrors> {
        let key = Self::queue_key(queue_urn);
        let meta = self
            .queue_meta
            .get(key)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .ok_or_else(|| RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn)))?;
        Self::decode_queue_meta(&meta)
    }

    /// Cursor of a subscription position. On an empty queue `Earliest` is
    /// the next index to be assigned (= `Latest`).
    fn position_cursor(meta: &QueueMeta, position: SubPosition) -> MsgIndex {
        let next = meta.next_index.max(1);
        match position {
            SubPosition::Earliest if meta.first_index == 0 => next,
            SubPosition::Earliest => meta.first_index,
            SubPosition::Latest => next,
            SubPosition::At(index) => index,
        }
    }

    fn get_queue_config(&self, queue_urn: &str) -> std::result::Result<QueueConfig, RPCErrors> {
        let key = Self::queue_key(queue_urn);
        let config = self
            .queues
            .get(key)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .ok_or_else(|| RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn)))?;
        Self::decode_queue_config(&config)
    }

    fn store_queue_meta(
        &self,
        queue_urn: &str,
        meta: &QueueMeta,
    ) -> std::result::Result<(), RPCErrors> {
        let key = Self::queue_key(queue_urn);
        let data = Self::encode_queue_meta(meta)?;
        self.queue_meta
            .insert(key, data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        Ok(())
    }
}

#[async_trait]
impl MsgQueueHandler for SledMsgQueue {
    async fn handle_create_queue(
        &self,
        name: Option<&str>,
        appid: &str,
        app_owner: &str,
        config: QueueConfig,
        _ctx: RPCContext,
    ) -> std::result::Result<QueueUrn, RPCErrors> {
        let name = match name {
            Some(value) => value.to_string(),
            None => format!("queue-{}", self.next_id("queue_id")?),
        };
        let queue_urn = calc_queue_urn(appid, app_owner, &name);
        let key = Self::queue_key(&queue_urn);
        let config_data =
            serde_json::to_vec(&config).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        let create_result = self
            .queues
            .compare_and_swap(key.clone(), None as Option<IVec>, Some(config_data))
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        if create_result.is_err() {
            return Err(RPCErrors::ReasonError(format!(
                "Queue already exists: {}",
                queue_urn
            )));
        }

        let meta = QueueMeta {
            next_index: 1,
            message_count: 0,
            first_index: 0,
            last_index: 0,
            size_bytes: 0,
        };
        if let Err(err) = self.store_queue_meta(&queue_urn, &meta) {
            let _ = self.queues.remove(key);
            return Err(err);
        }

        if config.sync_write {
            self.db
                .flush()
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        }

        Ok(queue_urn)
    }

    async fn handle_delete_queue(
        &self,
        queue_urn: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        let key = Self::queue_key(queue_urn);
        if self
            .queues
            .remove(key)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .is_none()
        {
            return Err(RPCErrors::ReasonError(format!(
                "Queue not found: {}",
                queue_urn
            )));
        }

        let prefix = Self::message_prefix(queue_urn);
        let message_keys: Vec<Vec<u8>> = self
            .messages
            .scan_prefix(prefix)
            .filter_map(|item| item.ok().map(|(key, _)| key.to_vec()))
            .collect();
        for key in message_keys {
            let _ = self.messages.remove(key);
        }

        for sub_id in self.queue_sub_ids(queue_urn)? {
            let owned = self
                .subs
                .get(&sub_id)
                .map_err(storage_err)?
                .and_then(|value| serde_json::from_slice::<SubscriptionState>(&value).ok())
                .is_some_and(|sub| sub.queue_urn == queue_urn);
            if owned {
                let _ = self.subs.remove(&sub_id);
            }
            let _ = self
                .queue_subs
                .remove(Self::queue_sub_key(queue_urn, &String::from_utf8_lossy(&sub_id)));
        }

        let _ = self.queue_meta.remove(Self::queue_key(queue_urn));
        self.db
            .flush()
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        Ok(())
    }

    async fn handle_get_queue_stats(
        &self,
        queue_urn: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<QueueStats, RPCErrors> {
        let meta = self.get_queue_meta(queue_urn)?;
        Ok(QueueStats {
            message_count: meta.message_count,
            first_index: meta.first_index,
            last_index: meta.last_index,
            size_bytes: meta.size_bytes,
        })
    }

    async fn handle_update_queue_config(
        &self,
        queue_urn: &str,
        config: QueueConfig,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        let key = Self::queue_key(queue_urn);
        if self
            .queues
            .get(key.clone())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .is_none()
        {
            return Err(RPCErrors::ReasonError(format!(
                "Queue not found: {}",
                queue_urn
            )));
        }
        let config_data =
            serde_json::to_vec(&config).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.queues
            .insert(key, config_data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        if config.sync_write {
            self.db
                .flush()
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        }
        Ok(())
    }

    async fn handle_post_message(
        &self,
        queue_urn: &str,
        mut message: Message,
        _ctx: RPCContext,
    ) -> std::result::Result<MsgIndex, RPCErrors> {
        let config = self.get_queue_config(queue_urn)?;
        let now = Self::now_seconds();
        if message.created_at == 0 {
            message.created_at = now;
        }

        let queue_key = Self::queue_key(queue_urn);
        let messages = &self.messages;
        let queue_meta = &self.queue_meta;
        let payload_len = message.payload.len() as u64;

        let result = (messages, queue_meta)
            .transaction(|trees| {
                let (messages, queue_meta) = trees;
                let meta_value = queue_meta.get(&queue_key)?.ok_or_else(|| {
                    sled::transaction::ConflictableTransactionError::Abort(RPCErrors::ReasonError(
                        format!("Queue not found: {}", queue_urn),
                    ))
                })?;
                let mut meta: QueueMeta = serde_json::from_slice(&meta_value).map_err(|err| {
                    sled::transaction::ConflictableTransactionError::Abort(RPCErrors::ReasonError(
                        format!("Failed to decode queue meta: {}", err),
                    ))
                })?;
                let index = meta.next_index;
                meta.next_index += 1;
                meta.message_count += 1;
                if meta.first_index == 0 {
                    meta.first_index = index;
                }
                meta.last_index = index;
                meta.size_bytes += payload_len;

                let mut stored_message = message.clone();
                stored_message.index = index;
                let data = serde_json::to_vec(&stored_message).map_err(|err| {
                    sled::transaction::ConflictableTransactionError::Abort(RPCErrors::ReasonError(
                        format!("Failed to encode message: {}", err),
                    ))
                })?;
                let msg_key = SledMsgQueue::message_key(queue_urn, index);
                messages.insert(msg_key, data)?;
                queue_meta.insert(
                    queue_key.clone(),
                    serde_json::to_vec(&meta).map_err(|err| {
                        sled::transaction::ConflictableTransactionError::Abort(
                            RPCErrors::ReasonError(format!("Failed to encode queue meta: {}", err)),
                        )
                    })?,
                )?;
                Ok(index)
            })
            .map_err(|err| match err {
                sled::transaction::TransactionError::Abort(err) => err,
                sled::transaction::TransactionError::Storage(err) => {
                    RPCErrors::ReasonError(err.to_string())
                }
            })?;

        if config.sync_write {
            self.db
                .flush()
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        }
        if config.max_messages.is_some() || config.retention_seconds.is_some() {
            self.enforce_limits(queue_urn, &config)?;
        }

        Ok(result)
    }

    async fn handle_subscribe(
        &self,
        queue_urn: &str,
        _user_id: &str,
        _app_id: &str,
        sub_id: Option<String>,
        position: SubPosition,
        _ctx: RPCContext,
    ) -> std::result::Result<SubscriptionId, RPCErrors> {
        let meta = self.get_queue_meta(queue_urn)?;
        let cursor = Self::position_cursor(&meta, position);

        let sub_id = match sub_id {
            Some(value) => value,
            None => format!("sub-{}", self.next_id("sub_id")?),
        };
        if self
            .subs
            .get(sub_id.as_bytes())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .is_some()
        {
            return Err(RPCErrors::ReasonError(format!(
                "Subscription already exists: {}",
                sub_id
            )));
        }

        let sub = SubscriptionState {
            queue_urn: queue_urn.to_string(),
            cursor,
        };
        let data =
            serde_json::to_vec(&sub).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.queue_subs
            .insert(Self::queue_sub_key(queue_urn, &sub_id), &[])
            .map_err(storage_err)?;
        self.subs
            .insert(sub_id.as_bytes(), data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        Ok(sub_id)
    }

    async fn handle_unsubscribe(
        &self,
        sub_id: &str,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        let Some(value) = self
            .subs
            .remove(sub_id.as_bytes())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
        else {
            return Err(RPCErrors::ReasonError(format!(
                "Subscription not found: {}",
                sub_id
            )));
        };
        if let Ok(sub) = serde_json::from_slice::<SubscriptionState>(&value) {
            self.queue_subs
                .remove(Self::queue_sub_key(&sub.queue_urn, sub_id))
                .map_err(storage_err)?;
            // It may have been the slowest consumer.
            self.trim_acked(&sub.queue_urn)?;
        }
        Ok(())
    }

    async fn handle_fetch_messages(
        &self,
        sub_id: &str,
        length: usize,
        auto_commit: bool,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<Message>, RPCErrors> {
        let sub_value = self
            .subs
            .get(sub_id.as_bytes())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .ok_or_else(|| RPCErrors::ReasonError(format!("Subscription not found: {}", sub_id)))?;
        let mut sub: SubscriptionState = serde_json::from_slice(&sub_value)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

        let start = Self::message_key(&sub.queue_urn, sub.cursor);
        let end = Self::message_key(&sub.queue_urn, u64::MAX);
        let mut messages = Vec::new();
        for item in self.messages.range(start..=end) {
            let (_, value) = item.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
            let msg = Self::decode_message(&value)?;
            messages.push(msg);
            if messages.len() >= length {
                break;
            }
        }

        if auto_commit {
            if let Some(last) = messages.last() {
                sub.cursor = last.index + 1;
                let data = serde_json::to_vec(&sub)
                    .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                self.subs
                    .insert(sub_id.as_bytes(), data)
                    .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                self.trim_acked(&sub.queue_urn)?;
            }
        }

        Ok(messages)
    }

    async fn handle_read_message(
        &self,
        queue_urn: &str,
        cursor: MsgIndex,
        length: usize,
        _ctx: RPCContext,
    ) -> std::result::Result<Vec<Message>, RPCErrors> {
        let _ = self.get_queue_meta(queue_urn)?;
        let start = Self::message_key(queue_urn, cursor);
        let end = Self::message_key(queue_urn, u64::MAX);
        let mut messages = Vec::new();
        for item in self.messages.range(start..=end) {
            let (_, value) = item.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
            let msg = Self::decode_message(&value)?;
            messages.push(msg);
            if messages.len() >= length {
                break;
            }
        }

        Ok(messages)
    }

    async fn handle_commit_ack(
        &self,
        sub_id: &str,
        index: MsgIndex,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        let sub_value = self
            .subs
            .get(sub_id.as_bytes())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .ok_or_else(|| RPCErrors::ReasonError(format!("Subscription not found: {}", sub_id)))?;
        let mut sub: SubscriptionState = serde_json::from_slice(&sub_value)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        // Cumulative: `index` and everything before it is consumed. A
        // smaller index than already acknowledged changes nothing (moving
        // back is `seek`); an index not assigned yet is refused.
        let meta = self.get_queue_meta(&sub.queue_urn)?;
        if index >= meta.next_index {
            return Err(RPCErrors::ReasonError(format!(
                "Invalid ack index {} for {}: the queue has assigned indexes below {}",
                index, sub.queue_urn, meta.next_index
            )));
        }
        if index + 1 <= sub.cursor {
            return Ok(());
        }
        sub.cursor = index + 1;
        let data =
            serde_json::to_vec(&sub).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.subs
            .insert(sub_id.as_bytes(), data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.trim_acked(&sub.queue_urn)?;
        if self
            .get_queue_config(&sub.queue_urn)
            .is_ok_and(|config| config.sync_write)
        {
            self.db.flush().map_err(storage_err)?;
        }
        Ok(())
    }

    async fn handle_seek(
        &self,
        sub_id: &str,
        index: SubPosition,
        _ctx: RPCContext,
    ) -> std::result::Result<(), RPCErrors> {
        let sub_value = self
            .subs
            .get(sub_id.as_bytes())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .ok_or_else(|| RPCErrors::ReasonError(format!("Subscription not found: {}", sub_id)))?;
        let mut sub: SubscriptionState = serde_json::from_slice(&sub_value)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

        let meta = self.get_queue_meta(&sub.queue_urn)?;
        sub.cursor = Self::position_cursor(&meta, index);

        let data =
            serde_json::to_vec(&sub).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.subs
            .insert(sub_id.as_bytes(), data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.trim_acked(&sub.queue_urn)?;
        Ok(())
    }

    async fn handle_delete_message_before(
        &self,
        queue_urn: &str,
        index: MsgIndex,
        _ctx: RPCContext,
    ) -> std::result::Result<u64, RPCErrors> {
        let config = self.get_queue_config(queue_urn)?;
        let removed = self.remove_before(queue_urn, index)?;
        if config.sync_write {
            self.db
                .flush()
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        }
        Ok(removed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::msg_queue::{QueueConfig, SubPosition};
    use tempfile::TempDir;

    fn setup_queue() -> (TempDir, SledMsgQueue) {
        let temp = TempDir::new().expect("create temp dir");
        let queue = SledMsgQueue::new_in_dir(temp.path()).expect("create sled queue");
        (temp, queue)
    }

    fn make_message(text: &str) -> Message {
        Message::new(text.as_bytes().to_vec())
    }

    async fn push_messages(queue: &SledMsgQueue, queue_urn: &str, count: usize) {
        for i in 1..=count {
            let _ = queue
                .handle_post_message(
                    queue_urn,
                    make_message(&format!("m{}", i)),
                    RPCContext::default(),
                )
                .await
                .unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_msg_queue_end_to_end() -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (_tmp, queue) = setup_queue();
        // Log semantics: history stays readable after acks.
        let config = QueueConfig {
            keep_acked: true,
            ..QueueConfig::default()
        };
        let queue_urn = queue
            .handle_create_queue(
                Some("inbox"),
                "app",
                "owner",
                config.clone(),
                RPCContext::default(),
            )
            .await?;
        // duplicate creation should fail
        assert!(
            queue
                .handle_create_queue(
                    Some("inbox"),
                    "app",
                    "owner",
                    config.clone(),
                    RPCContext::default(),
                )
                .await
                .is_err()
        );

        // update config and verify persisted
        let mut new_cfg = config.clone();
        new_cfg.sync_write = true;
        queue
            .handle_update_queue_config(&queue_urn, new_cfg.clone(), RPCContext::default())
            .await?;
        let stored = queue.get_queue_config(&queue_urn)?;
        assert!(stored.sync_write);

        // post messages
        let idx1 = queue
            .handle_post_message(&queue_urn, make_message("m1"), RPCContext::default())
            .await?;
        let idx2 = queue
            .handle_post_message(&queue_urn, make_message("m2"), RPCContext::default())
            .await?;
        let idx3 = queue
            .handle_post_message(&queue_urn, make_message("m3"), RPCContext::default())
            .await?;
        assert_eq!((idx1, idx2, idx3), (1, 2, 3));

        // stats after post
        let stats = queue
            .handle_get_queue_stats(&queue_urn, RPCContext::default())
            .await?;
        assert_eq!(stats.message_count, 3);
        assert_eq!(stats.first_index, 1);
        assert_eq!(stats.last_index, 3);

        // subscribe from earliest
        let sub_id = queue
            .handle_subscribe(
                &queue_urn,
                "user",
                "app",
                None,
                SubPosition::Earliest,
                RPCContext::default(),
            )
            .await?;
        let msgs = queue
            .handle_fetch_messages(&sub_id, 2, true, RPCContext::default())
            .await?;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].index, 1);
        assert_eq!(msgs[1].index, 2);

        // fetch remaining without auto commit then ack
        let msgs = queue
            .handle_fetch_messages(&sub_id, 2, false, RPCContext::default())
            .await?;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].index, 3);
        queue
            .handle_commit_ack(&sub_id, 3, RPCContext::default())
            .await?;
        let msgs = queue
            .handle_fetch_messages(&sub_id, 1, false, RPCContext::default())
            .await?;
        assert!(msgs.is_empty());
        let history = queue
            .handle_read_message(&queue_urn, 1, 2, RPCContext::default())
            .await?;
        assert_eq!(
            history.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![1, 2]
        );
        let msgs = queue
            .handle_fetch_messages(&sub_id, 1, false, RPCContext::default())
            .await?;
        assert!(msgs.is_empty());

        // seek back to earliest and read again
        queue
            .handle_seek(&sub_id, SubPosition::Earliest, RPCContext::default())
            .await?;
        let msgs = queue
            .handle_fetch_messages(&sub_id, 1, false, RPCContext::default())
            .await?;
        assert_eq!(msgs[0].index, 1);

        // subscribe at latest should see nothing until new message
        let sub_latest = queue
            .handle_subscribe(
                &queue_urn,
                "user",
                "app",
                Some("latest".to_string()),
                SubPosition::Latest,
                RPCContext::default(),
            )
            .await?;
        let msgs = queue
            .handle_fetch_messages(&sub_latest, 1, false, RPCContext::default())
            .await?;
        assert!(msgs.is_empty());

        // delete messages before index 3 (drops 1 and 2)
        let removed = queue
            .handle_delete_message_before(&queue_urn, 3, RPCContext::default())
            .await?;
        assert_eq!(removed, 2);
        let stats = queue
            .handle_get_queue_stats(&queue_urn, RPCContext::default())
            .await?;
        assert_eq!(stats.message_count, 1);
        assert_eq!(stats.first_index, 3);
        assert_eq!(stats.last_index, 3);

        // unsubscribe both
        queue
            .handle_unsubscribe(&sub_id, RPCContext::default())
            .await?;
        queue
            .handle_unsubscribe(&sub_latest, RPCContext::default())
            .await?;
        assert!(
            queue
                .handle_fetch_messages(&sub_id, 1, false, RPCContext::default())
                .await
                .is_err()
        );

        // delete queue
        queue
            .handle_delete_queue(&queue_urn, RPCContext::default())
            .await?;
        assert!(
            queue
                .handle_get_queue_stats(&queue_urn, RPCContext::default())
                .await
                .is_err()
        );

        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_multiple_subscribers_and_messages()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let (_tmp, queue) = setup_queue();
        let queue_urn = queue
            .handle_create_queue(
                Some("multi"),
                "app",
                "owner",
                QueueConfig {
                    keep_acked: true,
                    ..QueueConfig::default()
                },
                RPCContext::default(),
            )
            .await?;

        // seed 10 messages
        push_messages(&queue, &queue_urn, 10).await;

        // subscribers at different positions
        let sub_earliest = queue
            .handle_subscribe(
                &queue_urn,
                "user",
                "app",
                Some("earliest".into()),
                SubPosition::Earliest,
                RPCContext::default(),
            )
            .await?;
        let sub_latest = queue
            .handle_subscribe(
                &queue_urn,
                "user",
                "app",
                Some("latest".into()),
                SubPosition::Latest,
                RPCContext::default(),
            )
            .await?;
        let sub_mid = queue
            .handle_subscribe(
                &queue_urn,
                "user",
                "app",
                Some("mid".into()),
                SubPosition::At(5),
                RPCContext::default(),
            )
            .await?;

        // earliest reads first three and auto commits
        let msgs = queue
            .handle_fetch_messages(&sub_earliest, 3, true, RPCContext::default())
            .await?;
        assert_eq!(
            msgs.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );

        // mid reads two without commit, then commit to 6
        let msgs = queue
            .handle_fetch_messages(&sub_mid, 2, false, RPCContext::default())
            .await?;
        assert_eq!(msgs.iter().map(|m| m.index).collect::<Vec<_>>(), vec![5, 6]);
        queue
            .handle_commit_ack(&sub_mid, 6, RPCContext::default())
            .await?;

        // latest should see nothing until new messages arrive
        let msgs = queue
            .handle_fetch_messages(&sub_latest, 5, true, RPCContext::default())
            .await?;
        assert!(msgs.is_empty());

        // add two more messages (indexes 11,12)
        queue
            .handle_post_message(&queue_urn, make_message("m11"), RPCContext::default())
            .await?;
        queue
            .handle_post_message(&queue_urn, make_message("m12"), RPCContext::default())
            .await?;

        // latest now gets both new messages
        let msgs = queue
            .handle_fetch_messages(&sub_latest, 10, true, RPCContext::default())
            .await?;
        assert_eq!(
            msgs.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![11, 12]
        );

        // prune older messages (<6)
        let removed = queue
            .handle_delete_message_before(&queue_urn, 6, RPCContext::default())
            .await?;
        assert_eq!(removed, 5); // messages 1-5 removed
        let stats = queue
            .handle_get_queue_stats(&queue_urn, RPCContext::default())
            .await?;
        assert_eq!(stats.first_index, 6);
        assert_eq!(stats.last_index, 12);
        assert_eq!(stats.message_count, 7);

        // earliest cursor was at 4 (after auto commit) and should now see from 6
        let msgs = queue
            .handle_fetch_messages(&sub_earliest, 3, true, RPCContext::default())
            .await?;
        assert_eq!(
            msgs.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![6, 7, 8]
        );

        // mid cursor at 7 after commit; fetch remaining
        let msgs = queue
            .handle_fetch_messages(&sub_mid, 10, true, RPCContext::default())
            .await?;
        assert_eq!(
            msgs.iter().map(|m| m.index).collect::<Vec<_>>(),
            vec![7, 8, 9, 10, 11, 12]
        );

        // cleanup
        queue
            .handle_unsubscribe(&sub_earliest, RPCContext::default())
            .await?;
        queue
            .handle_unsubscribe(&sub_latest, RPCContext::default())
            .await?;
        queue
            .handle_unsubscribe(&sub_mid, RPCContext::default())
            .await?;
        queue
            .handle_delete_queue(&queue_urn, RPCContext::default())
            .await?;

        Ok(())
    }

    #[tokio::test(flavor = "current_thread")]
    async fn test_path_queue_name_roundtrip() -> std::result::Result<(), Box<dyn std::error::Error>>
    {
        let (_tmp, queue) = setup_queue();
        let queue_urn = "/jarvis.test.buckyos.io/sessions/tg:lzc_jarvis:5397330802/msg";
        let sub_id = "/jarvis.test.buckyos.io/sessions/tg:lzc_jarvis:5397330802/msg_subscription";

        let created = queue
            .handle_create_queue(
                Some(queue_urn),
                "opendan",
                "jarvis.test.buckyos.io",
                QueueConfig::default(),
                RPCContext::default(),
            )
            .await?;
        assert_eq!(created, queue_urn);

        queue
            .handle_post_message(queue_urn, make_message("hello"), RPCContext::default())
            .await?;

        queue
            .handle_subscribe(
                queue_urn,
                "jarvis.test.buckyos.io",
                "opendan",
                Some(sub_id.to_string()),
                SubPosition::Earliest,
                RPCContext::default(),
            )
            .await?;

        let messages = queue
            .handle_fetch_messages(sub_id, 1, true, RPCContext::default())
            .await?;
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].payload, b"hello".to_vec());

        Ok(())
    }
}
