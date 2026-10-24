use serde::{Deserialize, Serialize};

use crate::ExecutorOp;

pub const COMPUTER_API_VERSION: &str = "buckyos.computer.v1";
pub const COMPUTER_MAX_TEXT_BYTES: usize = 16 * 1024;
pub const COMPUTER_MAX_EXECUTION_MS: u64 = 30_000;
pub const COMPUTER_MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerObservationMode {
    #[default]
    Visual,
    Auto,
    Semantic,
    Hybrid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerFeedback {
    #[default]
    After,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerTextDelivery {
    #[default]
    Text,
    Keys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerScrollUnit {
    Pixel,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerCoordinateSpace {
    Screenshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerTarget {
    Point {
        observation_id: String,
        x: u32,
        y: u32,
        space: ComputerCoordinateSpace,
    },
    Focus {
        observation_id: String,
    },
}

impl ComputerTarget {
    pub fn observation_id(&self) -> &str {
        match self {
            Self::Point { observation_id, .. } | Self::Focus { observation_id } => observation_id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerWaitCondition {
    Delay { duration_ms: u64 },
}

fn default_timeout() -> u64 {
    10_000
}
fn default_click_count() -> u8 {
    1
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerRequest {
    Inspect {
        binding_id: String,
    },
    Observe {
        binding_id: String,
        #[serde(default)]
        mode: ComputerObservationMode,
    },
    Click {
        binding_id: String,
        target: ComputerTarget,
        #[serde(default)]
        button: ComputerButton,
        #[serde(default = "default_click_count")]
        count: u8,
        #[serde(default)]
        modifiers: Vec<String>,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    Type {
        binding_id: String,
        target: ComputerTarget,
        text: String,
        #[serde(default)]
        delivery: ComputerTextDelivery,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    Keypress {
        binding_id: String,
        target: ComputerTarget,
        keys: Vec<String>,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    Scroll {
        binding_id: String,
        target: ComputerTarget,
        delta_x: i32,
        delta_y: i32,
        unit: ComputerScrollUnit,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    WaitFor {
        binding_id: String,
        target: ComputerTarget,
        condition: ComputerWaitCondition,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
}

impl ComputerRequest {
    pub fn binding_id(&self) -> &str {
        match self {
            Self::Inspect { binding_id }
            | Self::Observe { binding_id, .. }
            | Self::Click { binding_id, .. }
            | Self::Type { binding_id, .. }
            | Self::Keypress { binding_id, .. }
            | Self::Scroll { binding_id, .. }
            | Self::WaitFor { binding_id, .. } => binding_id,
        }
    }

    pub fn required_op(&self) -> ExecutorOp {
        match self {
            Self::Inspect { .. } => ExecutorOp::ComputerInspect,
            Self::Observe { .. } => ExecutorOp::ComputerObserve,
            _ => ExecutorOp::ComputerAct,
        }
    }

    pub fn execution_timeout_ms(&self) -> u64 {
        match self {
            Self::Inspect { .. } | Self::Observe { .. } => default_timeout(),
            Self::Click {
                execution_timeout_ms,
                ..
            }
            | Self::Type {
                execution_timeout_ms,
                ..
            }
            | Self::Keypress {
                execution_timeout_ms,
                ..
            }
            | Self::Scroll {
                execution_timeout_ms,
                ..
            }
            | Self::WaitFor {
                execution_timeout_ms,
                ..
            } => *execution_timeout_ms,
        }
    }

    pub fn target(&self) -> Option<&ComputerTarget> {
        match self {
            Self::Inspect { .. } | Self::Observe { .. } => None,
            Self::Click { target, .. }
            | Self::Type { target, .. }
            | Self::Keypress { target, .. }
            | Self::Scroll { target, .. }
            | Self::WaitFor { target, .. } => Some(target),
        }
    }

    pub fn feedback(&self) -> (ComputerFeedback, ComputerObservationMode) {
        match self {
            Self::Inspect { .. } => (ComputerFeedback::None, ComputerObservationMode::Visual),
            Self::Observe { mode, .. } => (ComputerFeedback::After, *mode),
            Self::Click {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Type {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Keypress {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Scroll {
                feedback,
                feedback_mode,
                ..
            }
            | Self::WaitFor {
                feedback,
                feedback_mode,
                ..
            } => (*feedback, feedback_mode.unwrap_or_default()),
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        let binding = self.binding_id();
        if binding.is_empty() || binding.len() > 256 || binding.chars().any(char::is_control) {
            return Err("binding_id must identify one explicit surface".into());
        }
        if !(1..=COMPUTER_MAX_EXECUTION_MS).contains(&self.execution_timeout_ms()) {
            return Err("execution_timeout_ms must be between 1 and 30000".into());
        }
        if let Some(target) = self.target() {
            if target.observation_id().is_empty() || target.observation_id().len() > 128 {
                return Err("invalid observation_id".into());
            }
        }
        match self {
            Self::Click {
                target,
                count,
                modifiers,
                ..
            } => {
                if !matches!(target, ComputerTarget::Point { .. }) || !(1..=2).contains(count) {
                    return Err("click requires a point target and count 1 or 2".into());
                }
                validate_modifiers(modifiers)?;
            }
            Self::Type { target, text, .. } => {
                if !matches!(target, ComputerTarget::Focus { .. })
                    || text.is_empty()
                    || text.len() > COMPUTER_MAX_TEXT_BYTES
                    || text.contains('\0')
                {
                    return Err("type requires focus and 1..16384 UTF-8 bytes without NUL".into());
                }
            }
            Self::Keypress { target, keys, .. } => {
                if !matches!(target, ComputerTarget::Focus { .. })
                    || keys.is_empty()
                    || keys.len() > 5
                {
                    return Err(
                        "keypress requires focus and modifiers plus one ordinary key".into(),
                    );
                }
                validate_modifiers(&keys[..keys.len() - 1])?;
                let key = keys.last().unwrap();
                if is_modifier(key)
                    || key.is_empty()
                    || key.len() > 32
                    || key.chars().any(char::is_control)
                {
                    return Err("keypress requires exactly one ordinary key at the end".into());
                }
            }
            Self::Scroll {
                target,
                delta_x,
                delta_y,
                ..
            } => {
                if !matches!(target, ComputerTarget::Point { .. })
                    || delta_x.unsigned_abs() > 10_000
                    || delta_y.unsigned_abs() > 10_000
                    || (*delta_x == 0 && *delta_y == 0)
                {
                    return Err("scroll requires a point and nonzero deltas within +/-10000".into());
                }
            }
            Self::WaitFor {
                condition: ComputerWaitCondition::Delay { duration_ms },
                execution_timeout_ms,
                ..
            } => {
                if *duration_ms > *execution_timeout_ms {
                    return Err("delay exceeds execution timeout".into());
                }
            }
            _ => {}
        }
        match self {
            Self::Click {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Type {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Keypress {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Scroll {
                feedback,
                feedback_mode,
                ..
            }
            | Self::WaitFor {
                feedback,
                feedback_mode,
                ..
            } if *feedback == ComputerFeedback::None && feedback_mode.is_some() => {
                return Err("feedback_mode requires feedback=after".into());
            }
            _ => {}
        }
        Ok(())
    }
}

pub fn is_modifier(key: &str) -> bool {
    matches!(key, "Control" | "Meta" | "Alt" | "Shift")
}

fn validate_modifiers(keys: &[String]) -> Result<(), String> {
    let mut seen = std::collections::BTreeSet::new();
    if keys.iter().any(|k| !is_modifier(k) || !seen.insert(k)) {
        return Err("modifiers must be distinct Control, Meta, Alt or Shift".into());
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerExecutionState {
    NotStarted,
    InProgress,
    Completed,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerOperationState {
    Queued,
    AwaitingApproval,
    Running,
    Completed,
    Failed,
    Cancelled,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerExecution {
    pub state: ComputerExecutionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ComputerFeedbackState {
    NotRequested,
    Pending,
    Included,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerFeedbackResult {
    pub state: ComputerFeedbackState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerCapability {
    pub supported: bool,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerCapabilities {
    pub capture: ComputerCapability,
    pub input: ComputerCapability,
    pub accessibility: ComputerCapability,
    pub observation_modes: Vec<ComputerObservationMode>,
    pub commands: Vec<String>,
    pub focus_guard: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerBinding {
    pub binding_id: String,
    pub executor_id: String,
    pub profile_id: String,
    pub store_id: String,
    pub origin_generation: u64,
    pub authorized_principal: String,
    pub interactive_user: String,
    pub interactive_session_id: String,
    pub desktop_epoch: String,
    pub computer_session_id: String,
    pub surface_id: String,
    pub surface_epoch: u64,
    pub interaction_resource_id: String,
    pub expires_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerScreenshot {
    pub artifact_ref: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub space: ComputerCoordinateSpace,
    pub interactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerObservation {
    pub observation_id: String,
    pub binding_id: String,
    pub surface_id: String,
    pub surface_epoch: u64,
    pub interaction_revision: u64,
    pub requested_mode: ComputerObservationMode,
    pub effective_mode: ComputerObservationMode,
    pub selection_reason: String,
    pub capture_started_at_ms: u64,
    pub capture_completed_at_ms: u64,
    pub consistency: String,
    pub screenshot: ComputerScreenshot,
    pub focus_guard: String,
    pub expires_at_ms: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComputerResult {
    pub api_version: String,
    pub operation_state: ComputerOperationState,
    pub operation_id: String,
    pub binding_id: String,
    pub execution: ComputerExecution,
    pub feedback: ComputerFeedbackResult,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation: Option<ComputerObservation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<ComputerBinding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capabilities: Option<ComputerCapabilities>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ComputerError>,
}

impl ComputerResult {
    pub fn new(operation_id: &str, binding_id: &str, state: ComputerExecutionState) -> Self {
        Self {
            api_version: COMPUTER_API_VERSION.into(),
            operation_state: match state {
                ComputerExecutionState::Completed => ComputerOperationState::Completed,
                ComputerExecutionState::Unknown => ComputerOperationState::Unknown,
                ComputerExecutionState::InProgress => ComputerOperationState::Running,
                _ => ComputerOperationState::Failed,
            },
            operation_id: operation_id.into(),
            binding_id: binding_id.into(),
            execution: ComputerExecution { state },
            feedback: ComputerFeedbackResult {
                state: ComputerFeedbackState::NotRequested,
            },
            observation: None,
            binding: None,
            capabilities: None,
            error: None,
        }
    }

    pub fn failed(
        operation_id: &str,
        binding_id: &str,
        state: ComputerExecutionState,
        code: &str,
        message: impl Into<String>,
    ) -> Self {
        let mut result = Self::new(operation_id, binding_id, state);
        result.operation_state = if state == ComputerExecutionState::Unknown {
            ComputerOperationState::Unknown
        } else if code == "cancelled" {
            ComputerOperationState::Cancelled
        } else {
            ComputerOperationState::Failed
        };
        result.error = Some(ComputerError {
            code: code.into(),
            message: message.into(),
        });
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn click() -> serde_json::Value {
        json!({"command":"click","binding_id":"dev:display:1","target":{"kind":"point","observation_id":"obs","x":10,"y":20,"space":"screenshot"}})
    }

    #[test]
    fn requests_reject_unknown_fields_and_invalid_unions() {
        let request: ComputerRequest = serde_json::from_value(click()).unwrap();
        assert!(request.validate().is_ok());
        for field in ["approved", "principal", "force", "script"] {
            let mut value = click();
            value[field] = json!(true);
            assert!(serde_json::from_value::<ComputerRequest>(value).is_err());
        }
        for invalid in [json!(-1), json!(1.2), json!(4294967296_u64), json!("1")] {
            let mut value = click();
            value["target"]["x"] = invalid;
            assert!(serde_json::from_value::<ComputerRequest>(value).is_err());
        }
        for invalid in [
            json!({"command":"click","binding_id":"dev:display:1","target":{"kind":"focus","observation_id":"obs"}}),
            json!({"command":"type","binding_id":"dev:display:1","target":{"kind":"point","observation_id":"obs","x":1,"y":2,"space":"screenshot"},"text":"hello"}),
            json!({"command":"keypress","binding_id":"dev:display:1","target":{"kind":"focus","observation_id":"obs"},"keys":["Control","Meta"]}),
            json!({"command":"keypress","binding_id":"dev:display:1","target":{"kind":"focus","observation_id":"obs"},"keys":["Control","Control","a"]}),
        ] {
            assert!(serde_json::from_value::<ComputerRequest>(invalid)
                .unwrap()
                .validate()
                .is_err());
        }
    }

    #[test]
    fn bounds_and_feedback_are_validated_on_server() {
        for (field, invalid) in [
            ("count", json!(3)),
            ("execution_timeout_ms", json!(0)),
            ("execution_timeout_ms", json!(30001)),
            ("binding_id", json!("")),
        ] {
            let mut value = click();
            value[field] = invalid;
            assert!(serde_json::from_value::<ComputerRequest>(value)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut value = click();
        value["feedback"] = json!("none");
        value["feedback_mode"] = json!("visual");
        assert!(serde_json::from_value::<ComputerRequest>(value)
            .unwrap()
            .validate()
            .is_err());
        let request: ComputerRequest = serde_json::from_value(json!({
            "command":"type","binding_id":"dev:display:1","target":{"kind":"focus","observation_id":"obs"},"text":"x".repeat(COMPUTER_MAX_TEXT_BYTES + 1)
        })).unwrap();
        assert!(request.validate().is_err());
    }

    #[test]
    fn input_and_observation_permissions_are_distinct() {
        assert!(!ExecutorOp::ComputerAct.allowed_by(crate::AgentPermissionMode::ReadOnly));
        assert!(ExecutorOp::ComputerObserve.allowed_by(crate::AgentPermissionMode::ReadOnly));
        assert_eq!(
            serde_json::from_value::<ComputerRequest>(click())
                .unwrap()
                .required_op(),
            ExecutorOp::ComputerAct
        );
    }
}
