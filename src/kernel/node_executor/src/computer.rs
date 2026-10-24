use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use buckyos_api::{ComputerRequest, ComputerResult};
use tokio::sync::watch;

use crate::{ops::CancelReason, Actor};

#[derive(Clone)]
pub struct ComputerCancellation {
    pub(crate) receiver: watch::Receiver<Option<CancelReason>>,
    pub(crate) deadline: Instant,
    pub(crate) suspend_deadline_ms: u64,
}

impl ComputerCancellation {
    pub fn is_cancelled(&self) -> bool {
        self.receiver.borrow().is_some() || self.is_timed_out()
    }

    pub fn is_timed_out(&self) -> bool {
        Instant::now() >= self.deadline
            || crate::auth::suspend_clock_ms() >= self.suspend_deadline_ms
    }
}

#[derive(Clone)]
pub struct ComputerContext {
    pub actor: Actor,
    pub executor_id: String,
    pub profile_id: String,
    pub store_id: String,
    pub server_epoch: String,
    pub auth_generation: u64,
    pub auth_valid_until_ms: u64,
    pub op_id: String,
    pub execution_timeout_ms: u64,
    pub max_image_bytes: usize,
    pub cancellation: ComputerCancellation,
}

pub struct ComputerOutput {
    pub result: ComputerResult,
    pub png: Option<Vec<u8>>,
}

#[async_trait]
pub trait ComputerDriver: Send + Sync {
    async fn execute(&self, context: ComputerContext, request: ComputerRequest) -> ComputerOutput;

    fn invalidate(&self) {}

    fn set_paused(&self, _paused: bool) {}

    fn is_paused(&self) -> bool {
        false
    }

    fn set_generation(&self, _generation: u64) {
        self.invalidate();
    }

    /// `{ui, capture, input, accessibility}` readiness as the Driver sees
    /// the desktop now (reported by `describe`).
    fn capabilities(&self) -> serde_json::Value {
        serde_json::json!({})
    }
}

pub(crate) type SharedComputerDriver = Option<Arc<dyn ComputerDriver>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn suspend_deadline_cannot_be_extended_by_an_unexpired_instant() {
        let (_sender, receiver) = watch::channel(None);
        let cancellation = ComputerCancellation {
            receiver,
            deadline: Instant::now() + Duration::from_secs(30),
            suspend_deadline_ms: crate::auth::suspend_clock_ms().saturating_sub(1),
        };
        assert!(cancellation.is_timed_out());
        assert!(cancellation.is_cancelled());
    }
}
