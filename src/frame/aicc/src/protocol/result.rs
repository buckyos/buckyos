use super::{ProtocolError, ProtocolResultValue};
use buckyos_api::{AiArtifact, AiUsage};
use futures_util::Stream;
use serde_json::Value;
use std::pin::Pin;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProtocolOutput {
    pub value: Value,
    pub usage: Option<AiUsage>,
    pub artifacts: Vec<AiArtifact>,
}

impl ProtocolOutput {
    pub(crate) fn new(value: Value) -> Self {
        Self {
            value,
            usage: None,
            artifacts: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProtocolEvent {
    Delta(Value),
    Progress(Value),
    Final(ProtocolOutput),
}

pub(crate) type ProtocolEventStream =
    Pin<Box<dyn Stream<Item = ProtocolResultValue<ProtocolEvent>> + Send + 'static>>;

pub(crate) struct ProtocolStream {
    pub events: ProtocolEventStream,
}

impl std::fmt::Debug for ProtocolStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtocolStream { events: <stream> }")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeTaskState {
    Submitted,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl NativeTaskState {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeTaskHandle {
    pub remote_task_id: String,
    pub state: NativeTaskState,
    pub poll_after: Option<Duration>,
    pub cancel_supported: bool,
    pub webhook_supported: bool,
}

impl NativeTaskHandle {
    pub(crate) fn new(remote_task_id: impl Into<String>) -> ProtocolResultValue<Self> {
        let remote_task_id = remote_task_id.into();
        if remote_task_id.trim().is_empty() {
            return Err(ProtocolError::invalid_response(
                "native task ID must not be empty",
            ));
        }
        Ok(Self {
            remote_task_id,
            state: NativeTaskState::Submitted,
            poll_after: None,
            cancel_supported: false,
            webhook_supported: false,
        })
    }
}

#[derive(Debug)]
pub(crate) enum ProtocolExecution {
    Immediate(ProtocolOutput),
    Stream(ProtocolStream),
    NativeTask(NativeTaskHandle),
}

impl ProtocolExecution {
    pub(crate) fn mode(&self) -> super::ExecutionMode {
        match self {
            Self::Immediate(_) => super::ExecutionMode::Immediate,
            Self::Stream(_) => super::ExecutionMode::Stream,
            Self::NativeTask(_) => super::ExecutionMode::NativeTask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unifies_immediate_and_native_task_modes() {
        assert_eq!(
            ProtocolExecution::Immediate(ProtocolOutput::new(json!({"ok": true}))).mode(),
            super::super::ExecutionMode::Immediate
        );
        assert_eq!(
            ProtocolExecution::NativeTask(NativeTaskHandle::new("remote-1").unwrap()).mode(),
            super::super::ExecutionMode::NativeTask
        );
    }
}
