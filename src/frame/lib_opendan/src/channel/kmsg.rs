//! kmsg session input (§4.5) with the usage rules derived from the current
//! kmsg behaviour (§1.5):
//!
//! | kmsg reality | rule |
//! |---|---|
//! | `create_queue` / `subscribe` not idempotent | "already exists" = success, verified with `get_queue_stats` |
//! | sub ids share one namespace | `opendan.<agent_id>.<sid>` |
//! | cursors are not persisted (D-09) | "Subscription not found" → re-subscribe `At(acked + 1)` |
//! | `commit_ack` is cumulative; kmsg drops what every subscription acknowledged (unless `keep_acked`) | only ack the committed contiguous position: what is acked is gone |
//! | `delete_message_before` / retention limits drop records regardless of consumption | never used for session queues |
//! | a queue outlives its sessions unless deleted | the driver deletes it once the session takes no more input |
//! | queue data is node-local and may be lost | the driver recreates the queue under its fixed name (progress reset first); producers get `queue_missing` |
//! | no permission checks (D-07), `from` self-reported | `from` is audit only |

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::msg_queue::{Message, MsgQueueClient, QueueConfig, SubPosition};
use crate::error::{OpenDanError, Result};
use crate::protocol::*;

use super::InputSource;

pub const FETCH_BATCH: usize = 64;

fn is_already_exists(e: &::kRPC::RPCErrors) -> bool {
    e.to_string().to_ascii_lowercase().contains("already exists")
}

fn is_sub_not_found(e: &::kRPC::RPCErrors) -> bool {
    e.to_string().contains("Subscription not found")
}

/// The queue does not exist (never created, or its data was lost).
pub fn is_queue_not_found(e: &::kRPC::RPCErrors) -> bool {
    e.to_string().contains("Queue not found")
}

/// Create the session queue; "already exists" counts as success.
pub async fn ensure_queue(
    client: &MsgQueueClient,
    name: &str,
    appid: &str,
    owner: &str,
) -> Result<String> {
    let config = QueueConfig {
        sync_write: true,
        // Producer / consumer: what the driver acknowledged (committed to
        // state.json first) is not kept.
        keep_acked: false,
        other_app_can_write: true,
        other_app_can_read: false,
        ..QueueConfig::default()
    };
    match client.create_queue(Some(name), appid, owner, config).await {
        Ok(urn) => Ok(urn),
        Err(e) if is_already_exists(&e) => {
            let urn = buckyos_api::msg_queue::calc_queue_urn(appid, owner, name);
            client.get_queue_stats(&urn).await?;
            Ok(urn)
        }
        Err(e) => Err(e.into()),
    }
}

/// Create the subscription; "already exists" counts as success.
pub async fn ensure_subscription(
    client: &MsgQueueClient,
    queue: &str,
    sub_id: &str,
    user: &str,
    app: &str,
    position: SubPosition,
) -> Result<()> {
    match client
        .subscribe(queue, user, app, Some(sub_id.to_string()), position)
        .await
    {
        Ok(_) => Ok(()),
        Err(e) if is_already_exists(&e) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

/// Encode a logical record into a kmsg message: the envelope as headers
/// (all strings), the payload as UTF-8 JSON. A record that would be
/// rejected when consumed is refused here.
pub fn encode_input(input: &PostedInput) -> Result<Message> {
    input.validate()?;
    let mut msg = Message::new(input.payload_bytes()?);
    let mut headers = HashMap::new();
    headers.insert(HEADER_SCHEMA.to_string(), input.schema.clone());
    headers.insert(HEADER_TYPE.to_string(), input.input.type_name().to_string());
    headers.insert(HEADER_KEY.to_string(), input.key.clone());
    headers.insert(HEADER_FROM.to_string(), input.from.clone());
    headers.insert(HEADER_AT_MS.to_string(), input.at_ms.to_string());
    msg.headers = headers;
    Ok(msg)
}

/// Decode a kmsg message with the rules a record is posted with; a record
/// that does not pass comes back rejected.
pub fn decode_message(src: &str, m: &Message) -> FetchedInput {
    let h = |k: &str| m.headers.get(k).cloned().unwrap_or_default();
    let kind = h(HEADER_TYPE);
    let key = h(HEADER_KEY);
    let from = h(HEADER_FROM);
    let at_ms = m.headers.get(HEADER_AT_MS).and_then(|v| v.parse::<u64>().ok());
    let input = parse_record(
        m.headers.get(HEADER_SCHEMA).map(String::as_str),
        &kind,
        &key,
        &from,
        at_ms,
        &m.payload,
    );
    FetchedInput {
        src: src.to_string(),
        index: m.index,
        kind,
        key,
        from,
        at_ms: at_ms.unwrap_or(m.created_at * 1000),
        input,
    }
}

/// kmsg input source of one session.
pub struct KmsgInput {
    id: String,
    queue: String,
    subscriber: String,
    user: String,
    app: String,
    client: Arc<MsgQueueClient>,
}

impl KmsgInput {
    pub fn new(
        id: &str,
        queue: &str,
        subscriber: &str,
        driver_principal: &str,
        client: Arc<MsgQueueClient>,
    ) -> Self {
        let (app, user) = crate::ids::parse_app_principal(driver_principal)
            .unwrap_or_else(|| ("opendan".to_string(), driver_principal.to_string()));
        Self {
            id: id.to_string(),
            queue: queue.to_string(),
            subscriber: subscriber.to_string(),
            user,
            app,
            client,
        }
    }

    async fn resubscribe(&self, acked: u64) -> Result<()> {
        ensure_subscription(
            &self.client,
            &self.queue,
            &self.subscriber,
            &self.user,
            &self.app,
            SubPosition::At(acked + 1),
        )
        .await
    }
}

#[async_trait]
impl InputSource for KmsgInput {
    fn id(&self) -> &str {
        &self.id
    }

    async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<FetchedInput>> {
        let mut out = Vec::new();
        let mut cursor = progress.acked_index + 1;
        loop {
            let batch = self
                .client
                .read_message(&self.queue, cursor, FETCH_BATCH)
                .await?;
            if batch.is_empty() {
                break;
            }
            for m in &batch {
                cursor = cursor.max(m.index + 1);
                if progress.is_consumed(m.index) {
                    continue;
                }
                out.push(decode_message(&self.id, m));
                if out.len() >= max {
                    return Ok(out);
                }
            }
            if batch.len() < FETCH_BATCH {
                break;
            }
        }
        Ok(out)
    }

    async fn confirm(&self, progress: &SourceProgress) -> Result<()> {
        if progress.acked_index == 0 {
            return Ok(());
        }
        match self
            .client
            .commit_ack(&self.subscriber, progress.acked_index)
            .await
        {
            Ok(()) => Ok(()),
            Err(e) if is_sub_not_found(&e) => {
                self.resubscribe(progress.acked_index).await?;
                Ok(())
            }
            Err(e) => Err(e.into()),
        }
    }

    async fn first_available(&self) -> Result<Option<u64>> {
        let stats = self.client.get_queue_stats(&self.queue).await?;
        Ok(if stats.first_index == 0 {
            None
        } else {
            Some(stats.first_index)
        })
    }

    async fn exists(&self) -> Result<bool> {
        match self.client.get_queue_stats(&self.queue).await {
            Ok(_) => Ok(true),
            Err(e) if is_queue_not_found(&e) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    async fn recreate(&self) -> Result<()> {
        // The URN is `<appid>::<owner>::<name>` and fixed per session.
        let mut parts = self.queue.splitn(3, "::");
        let (Some(app), Some(owner), Some(name)) = (parts.next(), parts.next(), parts.next())
        else {
            return Err(OpenDanError::Channel(format!(
                "cannot recreate queue {}: not an `<appid>::<owner>::<name>` urn",
                self.queue
            )));
        };
        let urn = ensure_queue(&self.client, name, app, owner).await?;
        if urn != self.queue {
            return Err(OpenDanError::Channel(format!(
                "recreated queue {urn} instead of {}",
                self.queue
            )));
        }
        ensure_subscription(
            &self.client,
            &self.queue,
            &self.subscriber,
            &self.user,
            &self.app,
            SubPosition::Earliest,
        )
        .await
    }

    async fn release(&self) -> Result<()> {
        match self.client.delete_queue(&self.queue).await {
            Ok(()) => Ok(()),
            Err(e) if is_queue_not_found(&e) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

/// Post a record to a session queue (anyone with write access). This is
/// the raw append: the pending-input limit is enforced by
/// `SessionRegistry::post_input`, which knows the session's consumption
/// progress.
pub async fn post_to_queue(client: &MsgQueueClient, queue: &str, input: &PostedInput) -> Result<u64> {
    let msg = encode_input(input)?;
    Ok(client.post_message(queue, msg).await?)
}
