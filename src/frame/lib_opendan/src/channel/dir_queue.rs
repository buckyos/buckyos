//! A file-backed [`MsgQueueHandler`] with kmsg semantics, for development
//! and tests without a kmsg service (several processes may share one
//! directory; every operation runs under an exclusive flock).
//!
//! It deliberately mirrors kmsg's behaviour — non-idempotent
//! `create_queue` / `subscribe`, cumulative `commit_ack` that never moves
//! back, acknowledged messages dropped unless `keep_acked`,
//! `max_messages` / `retention_seconds` trimming,
//! `Subscription not found` after `forget_subscriptions` (simulating a
//! service restart that lost cursors, D-09) — so the session input rules
//! are exercised against the same edge cases.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use buckyos_api::msg_queue::*;
use fs2::FileExt;
use ::kRPC::{RPCContext, RPCErrors};
use serde::{Deserialize, Serialize};

use crate::fsutil;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct QueueMeta {
    next_index: u64,
    message_count: u64,
    first_index: u64,
    last_index: u64,
    size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SubState {
    queue_urn: String,
    cursor: u64,
}

pub struct DirMsgQueue {
    root: PathBuf,
}

fn err(msg: impl Into<String>) -> RPCErrors {
    RPCErrors::ReasonError(msg.into())
}

impl DirMsgQueue {
    pub fn new(root: impl AsRef<Path>) -> std::io::Result<Self> {
        let root = root.as_ref().to_path_buf();
        std::fs::create_dir_all(root.join("queues"))?;
        std::fs::create_dir_all(root.join("subs"))?;
        Ok(Self { root })
    }

    /// Build a `MsgQueueClient` backed by this directory.
    pub fn client(root: impl AsRef<Path>) -> std::io::Result<MsgQueueClient> {
        Ok(MsgQueueClient::new_in_process(Box::new(Self::new(root)?)))
    }

    fn guard(&self) -> Result<std::fs::File, RPCErrors> {
        let f = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(self.root.join(".lock"))
            .map_err(|e| err(e.to_string()))?;
        f.lock_exclusive().map_err(|e| err(e.to_string()))?;
        Ok(f)
    }

    fn qdir(&self, urn: &str) -> PathBuf {
        self.root.join("queues").join(hex::encode(urn.as_bytes()))
    }

    fn sub_path(&self, sub: &str) -> PathBuf {
        self.root.join("subs").join(format!("{}.json", hex::encode(sub.as_bytes())))
    }

    fn meta(&self, urn: &str) -> Result<QueueMeta, RPCErrors> {
        fsutil::read_json_opt::<QueueMeta>(&self.qdir(urn).join("meta.json"))
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("Queue not found: {urn}")))
    }

    fn write_meta(&self, urn: &str, m: &QueueMeta) -> Result<(), RPCErrors> {
        fsutil::atomic_replace_json(&self.qdir(urn).join("meta.json"), m)
            .map_err(|e| err(e.to_string()))
    }

    fn msg_path(&self, urn: &str, index: u64) -> PathBuf {
        self.qdir(urn).join("msgs").join(format!("{index:020}.json"))
    }

    fn read_from(&self, urn: &str, cursor: u64, length: usize) -> Result<Vec<Message>, RPCErrors> {
        let meta = self.meta(urn)?;
        let mut out = Vec::new();
        let mut i = cursor.max(1);
        while i <= meta.last_index && out.len() < length {
            if let Some(m) = fsutil::read_json_opt::<Message>(&self.msg_path(urn, i))
                .map_err(|e| err(e.to_string()))?
            {
                out.push(m);
            }
            i += 1;
        }
        Ok(out)
    }

    fn config(&self, urn: &str) -> Result<QueueConfig, RPCErrors> {
        Ok(fsutil::read_json_opt::<QueueConfig>(&self.qdir(urn).join("config.json"))
            .map_err(|e| err(e.to_string()))?
            .unwrap_or_default())
    }

    fn subs_of(&self, urn: &str) -> Result<Vec<SubState>, RPCErrors> {
        let mut out = Vec::new();
        for e in std::fs::read_dir(self.root.join("subs"))
            .map_err(|e| err(e.to_string()))?
            .flatten()
        {
            if let Ok(Some(s)) = fsutil::read_json_opt::<SubState>(&e.path()) {
                if s.queue_urn == urn {
                    out.push(s);
                }
            }
        }
        Ok(out)
    }

    /// Cursor of a subscription position (on an empty queue `Earliest` is
    /// the next index, like `Latest`).
    fn position_cursor(m: &QueueMeta, position: SubPosition) -> u64 {
        let next = m.next_index.max(1);
        match position {
            SubPosition::Earliest if m.first_index == 0 => next,
            SubPosition::Earliest => m.first_index,
            SubPosition::Latest => next,
            SubPosition::At(i) => i,
        }
    }

    /// Remove the messages below `bound` (caller holds the guard). Only
    /// `first_index` / counters change, indexes are never reused.
    fn remove_before(&self, urn: &str, bound: u64) -> Result<u64, RPCErrors> {
        let mut m = self.meta(urn)?;
        if m.first_index == 0 || m.first_index >= bound {
            return Ok(0);
        }
        let end = bound.min(m.last_index + 1);
        let mut removed = 0;
        let mut bytes = 0;
        for i in m.first_index..end {
            let path = self.msg_path(urn, i);
            if let Ok(Some(msg)) = fsutil::read_json_opt::<Message>(&path) {
                bytes += msg.payload.len() as u64;
            }
            if std::fs::remove_file(path).is_ok() {
                removed += 1;
            }
        }
        if end > m.last_index {
            m.first_index = 0;
            m.message_count = 0;
            m.size_bytes = 0;
        } else {
            m.first_index = end;
            m.message_count = m.message_count.saturating_sub(removed);
            m.size_bytes = m.size_bytes.saturating_sub(bytes);
        }
        self.write_meta(urn, &m)?;
        Ok(removed)
    }

    /// Unless `keep_acked`: drop what every subscription acknowledged.
    fn trim_acked(&self, urn: &str) -> Result<(), RPCErrors> {
        if !self.qdir(urn).join("meta.json").exists() || self.config(urn)?.keep_acked {
            return Ok(());
        }
        if let Some(floor) = self.subs_of(urn)?.iter().map(|s| s.cursor).min() {
            self.remove_before(urn, floor)?;
        }
        Ok(())
    }

    /// `max_messages` / `retention_seconds`.
    fn enforce_limits(&self, urn: &str, config: &QueueConfig) -> Result<(), RPCErrors> {
        if let Some(max) = config.max_messages.filter(|m| *m > 0) {
            let m = self.meta(urn)?;
            if m.first_index != 0 && m.last_index + 1 - m.first_index > max {
                self.remove_before(urn, m.last_index + 1 - max)?;
            }
        }
        if let Some(seconds) = config.retention_seconds.filter(|s| *s > 0) {
            let cutoff = (crate::now_ms() / 1000).saturating_sub(seconds);
            let m = self.meta(urn)?;
            if m.first_index != 0 {
                let fresh = self
                    .read_from(urn, m.first_index, usize::MAX)?
                    .into_iter()
                    .find(|msg| msg.created_at >= cutoff)
                    .map(|msg| msg.index)
                    .unwrap_or(m.last_index + 1);
                self.remove_before(urn, fresh)?;
            }
        }
        Ok(())
    }

    /// Simulate a kmsg restart that lost every subscription cursor (D-09).
    pub fn forget_subscriptions(&self) -> std::io::Result<()> {
        let _g = self.guard().map_err(|e| std::io::Error::other(e.to_string()))?;
        for e in std::fs::read_dir(self.root.join("subs"))?.flatten() {
            let _ = std::fs::remove_file(e.path());
        }
        Ok(())
    }

    /// Simulate a kmsg whose data was lost (its node-local store wiped):
    /// every queue and subscription is gone.
    pub fn wipe(&self) -> std::io::Result<()> {
        let _g = self.guard().map_err(|e| std::io::Error::other(e.to_string()))?;
        for d in ["queues", "subs"] {
            let dir = self.root.join(d);
            std::fs::remove_dir_all(&dir)?;
            std::fs::create_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Current cursor of a subscription (tests / diagnostics).
    pub fn cursor(&self, sub_id: &str) -> Option<u64> {
        fsutil::read_json_opt::<SubState>(&self.sub_path(sub_id))
            .ok()
            .flatten()
            .map(|s| s.cursor)
    }
}

#[async_trait]
impl MsgQueueHandler for DirMsgQueue {
    async fn handle_create_queue(
        &self,
        name: Option<&str>,
        appid: &str,
        app_owner: &str,
        config: QueueConfig,
        _ctx: RPCContext,
    ) -> Result<QueueUrn, RPCErrors> {
        let _g = self.guard()?;
        let name = name
            .map(str::to_string)
            .unwrap_or_else(|| format!("queue-{}", uuid::Uuid::new_v4().simple()));
        let urn = calc_queue_urn(appid, app_owner, &name);
        let dir = self.qdir(&urn);
        if dir.join("meta.json").exists() {
            return Err(err(format!("Queue already exists: {urn}")));
        }
        std::fs::create_dir_all(dir.join("msgs")).map_err(|e| err(e.to_string()))?;
        fsutil::atomic_replace_json(&dir.join("config.json"), &config)
            .map_err(|e| err(e.to_string()))?;
        fsutil::atomic_replace(&dir.join("urn"), urn.as_bytes()).map_err(|e| err(e.to_string()))?;
        self.write_meta(
            &urn,
            &QueueMeta {
                next_index: 1,
                ..Default::default()
            },
        )?;
        Ok(urn)
    }

    async fn handle_delete_queue(&self, queue_urn: &str, _ctx: RPCContext) -> Result<(), RPCErrors> {
        let _g = self.guard()?;
        let dir = self.qdir(queue_urn);
        if !dir.exists() {
            return Err(err(format!("Queue not found: {queue_urn}")));
        }
        std::fs::remove_dir_all(&dir).map_err(|e| err(e.to_string()))?;
        for e in std::fs::read_dir(self.root.join("subs"))
            .map_err(|e| err(e.to_string()))?
            .flatten()
        {
            if let Ok(Some(s)) = fsutil::read_json_opt::<SubState>(&e.path()) {
                if s.queue_urn == queue_urn {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(())
    }

    async fn handle_get_queue_stats(
        &self,
        queue_urn: &str,
        _ctx: RPCContext,
    ) -> Result<QueueStats, RPCErrors> {
        let _g = self.guard()?;
        let m = self.meta(queue_urn)?;
        Ok(QueueStats {
            message_count: m.message_count,
            first_index: m.first_index,
            last_index: m.last_index,
            size_bytes: m.size_bytes,
        })
    }

    async fn handle_update_queue_config(
        &self,
        queue_urn: &str,
        config: QueueConfig,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        let _g = self.guard()?;
        let _ = self.meta(queue_urn)?;
        fsutil::atomic_replace_json(&self.qdir(queue_urn).join("config.json"), &config)
            .map_err(|e| err(e.to_string()))
    }

    async fn handle_post_message(
        &self,
        queue_urn: &str,
        mut message: Message,
        _ctx: RPCContext,
    ) -> Result<MsgIndex, RPCErrors> {
        let _g = self.guard()?;
        let mut m = self.meta(queue_urn)?;
        let index = m.next_index.max(1);
        m.next_index = index + 1;
        m.message_count += 1;
        if m.first_index == 0 {
            m.first_index = index;
        }
        m.last_index = index;
        m.size_bytes += message.payload.len() as u64;
        message.index = index;
        if message.created_at == 0 {
            message.created_at = crate::now_ms() / 1000;
        }
        fsutil::atomic_replace_json(&self.msg_path(queue_urn, index), &message)
            .map_err(|e| err(e.to_string()))?;
        self.write_meta(queue_urn, &m)?;
        self.enforce_limits(queue_urn, &self.config(queue_urn)?)?;
        Ok(index)
    }

    async fn handle_subscribe(
        &self,
        queue_urn: &str,
        _user_id: &str,
        _app_id: &str,
        sub_id: Option<String>,
        position: SubPosition,
        _ctx: RPCContext,
    ) -> Result<SubscriptionId, RPCErrors> {
        let _g = self.guard()?;
        let m = self.meta(queue_urn)?;
        let cursor = Self::position_cursor(&m, position);
        let sub_id = sub_id.unwrap_or_else(|| format!("sub-{}", uuid::Uuid::new_v4().simple()));
        let p = self.sub_path(&sub_id);
        if p.exists() {
            return Err(err(format!("Subscription already exists: {sub_id}")));
        }
        fsutil::atomic_replace_json(
            &p,
            &SubState {
                queue_urn: queue_urn.to_string(),
                cursor,
            },
        )
        .map_err(|e| err(e.to_string()))?;
        Ok(sub_id)
    }

    async fn handle_unsubscribe(&self, sub_id: &str, _ctx: RPCContext) -> Result<(), RPCErrors> {
        let _g = self.guard()?;
        let p = self.sub_path(sub_id);
        let sub = fsutil::read_json_opt::<SubState>(&p)
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("Subscription not found: {sub_id}")))?;
        std::fs::remove_file(p).map_err(|e| err(e.to_string()))?;
        self.trim_acked(&sub.queue_urn)
    }

    async fn handle_fetch_messages(
        &self,
        sub_id: &str,
        length: usize,
        auto_commit: bool,
        _ctx: RPCContext,
    ) -> Result<Vec<Message>, RPCErrors> {
        let _g = self.guard()?;
        let p = self.sub_path(sub_id);
        let mut sub = fsutil::read_json_opt::<SubState>(&p)
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("Subscription not found: {sub_id}")))?;
        let msgs = self.read_from(&sub.queue_urn, sub.cursor, length)?;
        if auto_commit {
            if let Some(last) = msgs.last() {
                sub.cursor = last.index + 1;
                fsutil::atomic_replace_json(&p, &sub).map_err(|e| err(e.to_string()))?;
                self.trim_acked(&sub.queue_urn)?;
            }
        }
        Ok(msgs)
    }

    async fn handle_read_message(
        &self,
        queue_urn: &str,
        cursor: MsgIndex,
        length: usize,
        _ctx: RPCContext,
    ) -> Result<Vec<Message>, RPCErrors> {
        let _g = self.guard()?;
        self.read_from(queue_urn, cursor, length)
    }

    async fn handle_commit_ack(
        &self,
        sub_id: &str,
        index: MsgIndex,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        let _g = self.guard()?;
        let p = self.sub_path(sub_id);
        let mut sub = fsutil::read_json_opt::<SubState>(&p)
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("Subscription not found: {sub_id}")))?;
        let m = self.meta(&sub.queue_urn)?;
        if index >= m.next_index.max(1) {
            return Err(err(format!(
                "Invalid ack index {index} for {}: the queue has assigned indexes below {}",
                sub.queue_urn, m.next_index
            )));
        }
        // Cumulative, never moves back (that is `seek`).
        if index + 1 <= sub.cursor {
            return Ok(());
        }
        sub.cursor = index + 1;
        fsutil::atomic_replace_json(&p, &sub).map_err(|e| err(e.to_string()))?;
        self.trim_acked(&sub.queue_urn)
    }

    async fn handle_seek(
        &self,
        sub_id: &str,
        index: SubPosition,
        _ctx: RPCContext,
    ) -> Result<(), RPCErrors> {
        let _g = self.guard()?;
        let p = self.sub_path(sub_id);
        let mut sub = fsutil::read_json_opt::<SubState>(&p)
            .map_err(|e| err(e.to_string()))?
            .ok_or_else(|| err(format!("Subscription not found: {sub_id}")))?;
        let m = self.meta(&sub.queue_urn)?;
        sub.cursor = Self::position_cursor(&m, index);
        fsutil::atomic_replace_json(&p, &sub).map_err(|e| err(e.to_string()))?;
        self.trim_acked(&sub.queue_urn)
    }

    async fn handle_delete_message_before(
        &self,
        queue_urn: &str,
        index: MsgIndex,
        _ctx: RPCContext,
    ) -> Result<u64, RPCErrors> {
        let _g = self.guard()?;
        self.remove_before(queue_urn, index)
    }
}
