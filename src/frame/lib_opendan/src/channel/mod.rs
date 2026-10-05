//! Session inputs (§4.5): kmsg queue per session + kevent wake-ups. No
//! queues are re-implemented on the file system of the session.

pub mod dir_queue;
pub mod kmsg;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use buckyos_api::msg_queue::MsgQueueClient;

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

pub use dir_queue::DirMsgQueue;
pub use kmsg::KmsgInput;

/// One input source of a session. Only the lease holder consumes.
#[async_trait]
pub trait InputSource: Send + Sync {
    fn id(&self) -> &str;
    /// Deliveries after the committed progress that are not consumed yet
    /// (oldest first, at most `max`).
    async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<FetchedInput>>;
    /// Confirm (cumulative ack) the committed contiguous position. Always
    /// safe to repeat; never called with a position below a previous one.
    async fn confirm(&self, progress: &SourceProgress) -> Result<()>;
    /// Lowest index the source still holds (`None` = empty / unknown).
    async fn first_available(&self) -> Result<Option<u64>>;
    /// `false` only when the service reports the source gone (a kmsg queue
    /// whose data was lost while the session directory survived).
    async fn exists(&self) -> Result<bool> {
        Ok(true)
    }
    /// Create a missing source again under its fixed name; deliveries are
    /// numbered from the start again.
    async fn recreate(&self) -> Result<()> {
        Ok(())
    }
    /// Give the source back: the session will not consume input any more
    /// (a missing source counts as released).
    async fn release(&self) -> Result<()> {
        Ok(())
    }
}

/// Opens the input sources declared in `session_config.channels`.
#[async_trait]
pub trait InputChannelFactory: Send + Sync {
    async fn open(&self, cfg: &SessionConfig) -> Result<Vec<Arc<dyn InputSource>>>;
    /// Queue client used to create queues and post inputs.
    fn queue_client(&self) -> Option<Arc<MsgQueueClient>>;
}

/// kmsg-backed factory (real kmsg service or a [`DirMsgQueue`]).
pub struct KmsgChannels {
    client: Arc<MsgQueueClient>,
}

impl KmsgChannels {
    pub fn new(client: Arc<MsgQueueClient>) -> Self {
        Self { client }
    }

    /// Development queue directory (no kmsg service needed).
    pub fn dir(root: impl AsRef<std::path::Path>) -> Result<Self> {
        let client = DirMsgQueue::client(root.as_ref())
            .map_err(|e| OpenDanError::io(root.as_ref(), e))?;
        Ok(Self::new(Arc::new(client)))
    }

    pub fn client(&self) -> Arc<MsgQueueClient> {
        self.client.clone()
    }
}

#[async_trait]
impl InputChannelFactory for KmsgChannels {
    async fn open(&self, cfg: &SessionConfig) -> Result<Vec<Arc<dyn InputSource>>> {
        let mut out: Vec<Arc<dyn InputSource>> = Vec::new();
        for src in &cfg.channels.inputs {
            match src {
                InputSourceConfig::Kmsg {
                    id,
                    queue,
                    subscriber,
                } => out.push(Arc::new(KmsgInput::new(
                    id,
                    queue,
                    subscriber,
                    &cfg.session.driver.principal,
                    self.client.clone(),
                ))),
            }
        }
        Ok(out)
    }

    fn queue_client(&self) -> Option<Arc<MsgQueueClient>> {
        Some(self.client.clone())
    }
}

/// Waits for "something changed" (kevent) with a polling fallback.
#[async_trait]
pub trait Waker: Send + Sync {
    /// Return when woken for `wake_event` or after `timeout`.
    async fn wait(&self, wake_event: Option<&str>, timeout: Duration);
    /// Publish a wake event (best effort; losses are covered by polling).
    async fn notify(&self, wake_event: &str, data: serde_json::Value);
}

/// Pure polling.
pub struct PollWaker;

#[async_trait]
impl Waker for PollWaker {
    async fn wait(&self, _wake_event: Option<&str>, timeout: Duration) {
        tokio::time::sleep(timeout).await;
    }

    async fn notify(&self, _wake_event: &str, _data: serde_json::Value) {}
}

/// kevent wake-ups; falls back to polling when the reader cannot be created.
pub struct KEventWaker {
    client: Arc<buckyos_api::KEventClient>,
}

impl KEventWaker {
    pub fn new(client: Arc<buckyos_api::KEventClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl Waker for KEventWaker {
    async fn wait(&self, wake_event: Option<&str>, timeout: Duration) {
        let Some(ev) = wake_event else {
            tokio::time::sleep(timeout).await;
            return;
        };
        match self.client.create_event_reader(vec![ev.to_string()]).await {
            Ok(reader) => {
                let _ = reader.pull_event(Some(timeout.as_millis() as u64)).await;
                let _ = reader.close().await;
            }
            Err(e) => {
                log::debug!("kevent reader for {ev} unavailable ({e}); polling");
                tokio::time::sleep(timeout).await;
            }
        }
    }

    async fn notify(&self, wake_event: &str, data: serde_json::Value) {
        if let Err(e) = self.client.pub_event(wake_event, data).await {
            log::debug!("kevent publish {wake_event} failed: {e}");
        }
    }
}

/// State change notifications (registry reports are separate).
#[async_trait]
pub trait Notifier: Send + Sync {
    async fn session_changed(&self, sid: &str, rev: u64);
}

pub struct NoopNotifier;

#[async_trait]
impl Notifier for NoopNotifier {
    async fn session_changed(&self, _sid: &str, _rev: u64) {}
}

/// Read-only source over `prompt.initial_inputs` ([`BOOTSTRAP_SRC`]): the
/// bootstrap material of a session without an input queue. Index = position
/// from 1; consumption is recorded in state like any source, confirming is a
/// no-op.
pub struct BootstrapSource {
    records: Vec<PostedInput>,
}

impl BootstrapSource {
    pub fn new(records: &[PostedInput]) -> Self {
        Self {
            records: records.to_vec(),
        }
    }
}

#[async_trait]
impl InputSource for BootstrapSource {
    fn id(&self) -> &str {
        BOOTSTRAP_SRC
    }

    async fn fetch(&self, progress: &SourceProgress, max: usize) -> Result<Vec<FetchedInput>> {
        Ok(self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (i as u64 + 1, r))
            .filter(|(index, _)| !progress.is_consumed(*index))
            .take(max)
            .map(|(index, r)| FetchedInput {
                src: BOOTSTRAP_SRC.to_string(),
                index,
                kind: r.input.type_name().to_string(),
                key: r.key.clone(),
                from: r.from.clone(),
                at_ms: r.at_ms,
                input: Ok(r.input.clone()),
            })
            .collect())
    }

    async fn confirm(&self, _progress: &SourceProgress) -> Result<()> {
        Ok(())
    }

    async fn first_available(&self) -> Result<Option<u64>> {
        Ok((!self.records.is_empty()).then_some(1))
    }
}
