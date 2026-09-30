//! kmsg session input (§4.5) with the usage rules derived from the current
//! kmsg behaviour (§1.5):
//!
//! | kmsg reality | rule |
//! |---|---|
//! | `create_queue` / `subscribe` not idempotent | "already exists" = success, verified with `get_queue_stats` |
//! | sub ids share one namespace | `opendan.<agent_id>.<sid>` |
//! | cursors are not persisted (D-09) | "Subscription not found" → re-subscribe `At(acked + 1)` |
//! | `commit_ack` sets `cursor = index + 1`, may move back | only ack the committed contiguous position, never less than acked before |
//! | `delete_message_before` races with post | never delete messages here |
//! | no permission checks (D-07), `from` self-reported | `from` is audit only |

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::msg_queue::{Message, MsgQueueClient, QueueConfig, SubPosition};
use serde_json::Value;

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

/// Create the session queue; "already exists" counts as success.
pub async fn ensure_queue(
    client: &MsgQueueClient,
    name: &str,
    appid: &str,
    owner: &str,
) -> Result<String> {
    let config = QueueConfig {
        sync_write: true,
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

/// Encode an input into a kmsg message.
pub fn encode_input(input: &Input, from: &str) -> Result<Message> {
    let payload = serde_json::to_vec(&input.payload)
        .map_err(|e| OpenDanError::InvalidArgument(format!("payload: {e}")))?;
    if payload.len() > MAX_PAYLOAD_BYTES {
        return Err(OpenDanError::InvalidArgument(format!(
            "input payload is {} bytes; put large content into the session directory or NamedStore and post a reference (limit {MAX_PAYLOAD_BYTES})",
            payload.len()
        )));
    }
    let mut msg = Message::new(payload);
    let mut headers = HashMap::new();
    headers.insert(HEADER_TYPE.to_string(), input.kind.as_str().to_string());
    headers.insert(HEADER_KEY.to_string(), input.key.clone());
    headers.insert(HEADER_FROM.to_string(), from.to_string());
    headers.insert(HEADER_AT_MS.to_string(), crate::now_ms().to_string());
    if let Some(i) = &input.intent {
        headers.insert(HEADER_INTENT.to_string(), i.clone());
    }
    if let Some(r) = &input.reply_to {
        headers.insert(HEADER_REPLY_TO.to_string(), r.clone());
    }
    msg.headers = headers;
    Ok(msg)
}

/// Decode a kmsg message; undecodable deliveries come back `malformed`.
pub fn decode_message(src: &str, m: &Message) -> InputMessage {
    let h = |k: &str| m.headers.get(k).cloned().unwrap_or_default();
    let kind_raw = h(HEADER_TYPE);
    let payload: std::result::Result<Value, _> = serde_json::from_slice(&m.payload);
    let mut malformed = None;
    let kind = match InputKind::parse(&kind_raw) {
        Some(k) => k,
        None => {
            malformed = Some(format!("unknown input type `{kind_raw}`"));
            InputKind::Msg
        }
    };
    let payload = match payload {
        Ok(v) => v,
        Err(e) => {
            malformed.get_or_insert(format!("payload is not JSON: {e}"));
            Value::Null
        }
    };
    let mut msg = InputMessage {
        src: src.to_string(),
        index: m.index,
        kind,
        key: h(HEADER_KEY),
        from: h(HEADER_FROM),
        at_ms: h(HEADER_AT_MS).parse().unwrap_or(m.created_at * 1000),
        intent: m.headers.get(HEADER_INTENT).cloned(),
        reply_to: m.headers.get(HEADER_REPLY_TO).cloned(),
        payload,
        malformed,
    };
    if msg.malformed.is_none() && kind == InputKind::Control && msg.control().is_none() {
        msg.malformed = Some("control payload is not a known command".into());
    }
    msg
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

    async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<InputMessage>> {
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
}

/// Post an input to a session queue (anyone with write access).
pub async fn post_to_queue(
    client: &MsgQueueClient,
    queue: &str,
    input: &Input,
    from: &str,
) -> Result<u64> {
    let msg = encode_input(input, from)?;
    Ok(client.post_message(queue, msg).await?)
}
