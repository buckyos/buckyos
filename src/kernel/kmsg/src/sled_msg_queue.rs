use ::kRPC::*;
use async_trait::async_trait;
use buckyos_api::msg_queue::*;
use buckyos_kit::get_buckyos_service_local_data_dir;
use bytes::Bytes;
use cyfs_gateway_lib::{
    HttpServer, ServerError, ServerErrorCode, ServerResult, StreamInfo, serve_http_by_rpc_handler,
    server_err,
};
use http::{Method, Version};
use http_body_util::combinators::BoxBody;
use serde::{Deserialize, Serialize};
use sled::{Db, IVec, Tree, transaction::Transactional};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::task;

pub struct SledMsgQueueServer {
    handler: MsgQueueServerHandler<SledMsgQueue>,
}

impl SledMsgQueueServer {
    pub fn new() -> Self {
        let queue = SledMsgQueue::new().expect("Failed to open kmsg sled database");
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
        self.handler.handle_rpc_call(req, ip_from).await
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
    meta: Tree,
}

impl SledMsgQueue {
    pub fn new() -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let data_path = get_buckyos_service_local_data_dir("kmsg", None);
        Self::new_in_dir(data_path)
    }

    pub fn new_in_dir<P: AsRef<Path>>(
        path: P,
    ) -> std::result::Result<Self, Box<dyn std::error::Error>> {
        let db = sled::open(path)?;
        Ok(Self {
            queues: db.open_tree("queues")?,
            queue_meta: db.open_tree("queue_meta")?,
            messages: db.open_tree("messages")?,
            subs: db.open_tree("subs")?,
            meta: db.open_tree("meta")?,
            db: Arc::new(db),
        })
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

    fn decode_index_from_key(key: &[u8]) -> Option<MsgIndex> {
        if key.len() < 8 {
            return None;
        }
        let start = key.len() - 8;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&key[start..]);
        Some(MsgIndex::from_be_bytes(buf))
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

        let sub_keys: Vec<Vec<u8>> = self
            .subs
            .iter()
            .filter_map(|item| item.ok())
            .filter_map(|(key, value)| {
                let sub: SubscriptionState = serde_json::from_slice(&value).ok()?;
                if sub.queue_urn == queue_urn {
                    Some(key.to_vec())
                } else {
                    None
                }
            })
            .collect();
        for key in sub_keys {
            let _ = self.subs.remove(key);
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
        let first_index = if meta.first_index == 0 {
            1
        } else {
            meta.first_index
        };
        let last_index = meta.last_index;
        let cursor = match position {
            SubPosition::Earliest => first_index,
            SubPosition::Latest => last_index + 1,
            SubPosition::At(index) => index,
        };

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
        if self
            .subs
            .remove(sub_id.as_bytes())
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
            .is_none()
        {
            return Err(RPCErrors::ReasonError(format!(
                "Subscription not found: {}",
                sub_id
            )));
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
        sub.cursor = index + 1;
        let data =
            serde_json::to_vec(&sub).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.subs
            .insert(sub_id.as_bytes(), data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
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
        let first_index = if meta.first_index == 0 {
            1
        } else {
            meta.first_index
        };
        let last_index = meta.last_index;
        sub.cursor = match index {
            SubPosition::Earliest => first_index,
            SubPosition::Latest => last_index + 1,
            SubPosition::At(value) => value,
        };

        let data =
            serde_json::to_vec(&sub).map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        self.subs
            .insert(sub_id.as_bytes(), data)
            .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
        Ok(())
    }

    async fn handle_delete_message_before(
        &self,
        queue_urn: &str,
        index: MsgIndex,
        _ctx: RPCContext,
    ) -> std::result::Result<u64, RPCErrors> {
        let config = self.get_queue_config(queue_urn)?;
        let queue_urn = queue_urn.to_string();
        let messages = self.messages.clone();
        let queue_meta = self.queue_meta.clone();

        let removed = task::spawn_blocking(move || {
            let meta_value = queue_meta
                .get(SledMsgQueue::queue_key(&queue_urn))
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?
                .ok_or_else(|| RPCErrors::ReasonError(format!("Queue not found: {}", queue_urn)))?;
            let mut meta = SledMsgQueue::decode_queue_meta(&meta_value)?;

            let start = SledMsgQueue::message_key(&queue_urn, 0);
            let end = SledMsgQueue::message_key(&queue_urn, index);
            let mut removed_count = 0u64;
            let mut removed_bytes = 0u64;
            let mut removed_indexes = Vec::new();

            for item in messages.range(start..end) {
                let (key, value) = item.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                if let Ok(msg) = SledMsgQueue::decode_message(&value) {
                    removed_bytes += msg.payload.len() as u64;
                }
                removed_count += 1;
                removed_indexes.push(key.to_vec());
            }

            for key in removed_indexes {
                let _ = messages.remove(key);
            }

            if removed_count == 0 {
                return Ok(0u64);
            }

            meta.message_count = meta.message_count.saturating_sub(removed_count);
            meta.size_bytes = meta.size_bytes.saturating_sub(removed_bytes);

            if meta.message_count == 0 {
                meta.first_index = 0;
                meta.last_index = 0;
            } else {
                let scan_start = SledMsgQueue::message_key(&queue_urn, index);
                let scan_end = SledMsgQueue::message_key(&queue_urn, u64::MAX);
                let mut new_first = None;
                for item in messages.range(scan_start..=scan_end) {
                    let (key, _) = item.map_err(|err| RPCErrors::ReasonError(err.to_string()))?;
                    if let Some(idx) = SledMsgQueue::decode_index_from_key(&key) {
                        new_first = Some(idx);
                        break;
                    }
                }
                if let Some(idx) = new_first {
                    meta.first_index = idx;
                }
            }

            let meta_data = SledMsgQueue::encode_queue_meta(&meta)?;
            queue_meta
                .insert(SledMsgQueue::queue_key(&queue_urn), meta_data)
                .map_err(|err| RPCErrors::ReasonError(err.to_string()))?;

            Ok(removed_count)
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(err.to_string()))??;

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
        let config = QueueConfig::default();
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
                QueueConfig::default(),
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
}
