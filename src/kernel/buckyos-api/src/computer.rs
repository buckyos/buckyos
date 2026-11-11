use serde::{Deserialize, Serialize};

use crate::ExecutorOp;

pub const COMPUTER_API_VERSION: &str = "buckyos.computer.v1";
pub const COMPUTER_MAX_TEXT_BYTES: usize = 16 * 1024;
pub const COMPUTER_MAX_EXECUTION_MS: u64 = 30_000;
pub const COMPUTER_MAX_IMAGE_BYTES: usize = 16 * 1024 * 1024;
/// Upper bound of one serialized semantic payload; it travels in the
/// control frame (EXECUTOR_MAX_CONTROL_FRAME), never in the content channel.
pub const COMPUTER_MAX_SEMANTIC_BYTES: usize = 256 * 1024;
pub const COMPUTER_SEMANTIC_FORMAT: &str = "outline-v1";
pub const COMPUTER_SEMANTIC_MAX_NODES: usize = 400;
pub const COMPUTER_SEMANTIC_MAX_DEPTH: usize = 32;
pub const COMPUTER_SEMANTIC_MAX_TEXT_BYTES: usize = 32 * 1024;
pub const COMPUTER_SEMANTIC_SNAPSHOT_MS: u64 = 3_000;
pub const COMPUTER_MAX_WINDOWS: usize = 32;

macro_rules! computer_names {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, schemars::JsonSchema)]
        pub enum $name {
            $(#[serde(rename = $text)] $variant),+
        }

        impl $name {
            pub const ALL: &'static [Self] = &[$(Self::$variant),+];

            pub fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $text),+
                }
            }
        }
    };
}

computer_names! {
    /// Cross-platform element role (WAI-ARIA names) shared by the Windows
    /// UIA, macOS AX and browser backends; anything else is `generic`.
    ComputerRole {
        Window => "window",
        Dialog => "dialog",
        Document => "document",
        Group => "group",
        Toolbar => "toolbar",
        Heading => "heading",
        Button => "button",
        Link => "link",
        Tab => "tab",
        Tablist => "tablist",
        Menu => "menu",
        Menubar => "menubar",
        Menuitem => "menuitem",
        Checkbox => "checkbox",
        Radio => "radio",
        Switch => "switch",
        Textbox => "textbox",
        Combobox => "combobox",
        Slider => "slider",
        List => "list",
        Listitem => "listitem",
        Tree => "tree",
        Treeitem => "treeitem",
        Table => "table",
        Row => "row",
        Cell => "cell",
        Image => "image",
        Text => "text",
        Generic => "generic",
    }
}

computer_names! {
    /// Cross-platform element state names.
    ComputerElementState {
        Disabled => "disabled",
        Focused => "focused",
        Selected => "selected",
        Checked => "checked",
        Mixed => "mixed",
        Expanded => "expanded",
        Collapsed => "collapsed",
        Readonly => "readonly",
    }
}

computer_names! {
    /// Cross-platform element action names; each semantic command is only
    /// accepted for an element whose observation declared the action.
    ComputerElementAction {
        Focus => "focus",
        Invoke => "invoke",
        SetValue => "set_value",
        Toggle => "toggle",
        Select => "select",
        Expand => "expand",
        Collapse => "collapse",
        ScrollIntoView => "scroll_into_view",
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerObservationMode {
    #[default]
    Visual,
    Auto,
    Semantic,
    Hybrid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerFeedback {
    #[default]
    After,
    None,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerButton {
    #[default]
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerTextDelivery {
    #[default]
    Text,
    Keys,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerScrollUnit {
    Pixel,
    Line,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerCoordinateSpace {
    Screenshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
    /// An element `ref` from that observation's semantic outline.
    Element {
        observation_id: String,
        #[serde(rename = "ref")]
        element_ref: String,
    },
}

impl ComputerTarget {
    pub fn observation_id(&self) -> &str {
        match self {
            Self::Point { observation_id, .. }
            | Self::Focus { observation_id }
            | Self::Element { observation_id, .. } => observation_id,
        }
    }

    pub fn element_ref(&self) -> Option<&str> {
        match self {
            Self::Element { element_ref, .. } => Some(element_ref),
            _ => None,
        }
    }
}

/// Which window's control tree a semantic observation returns.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerObserveScope {
    #[default]
    Foreground,
    /// A window `ref` from an earlier observation's window list.
    Window {
        observation_id: String,
        #[serde(rename = "ref")]
        window_ref: String,
    },
}

fn valid_ref(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn valid_observation_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum ComputerRequest {
    Inspect {
        binding_id: String,
    },
    Observe {
        binding_id: String,
        #[serde(default)]
        mode: ComputerObservationMode,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        scope: Option<ComputerObserveScope>,
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
    /// The element's default action (UIA Invoke, AX Press, DOM click)
    /// without moving the pointer.
    Invoke {
        binding_id: String,
        target: ComputerTarget,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    /// Replace the element's whole value (`semantic.fill`).
    SetValue {
        binding_id: String,
        target: ComputerTarget,
        value: String,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    /// Flip a checkbox or switch (UIA Toggle, AX Press).
    Toggle {
        binding_id: String,
        target: ComputerTarget,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    /// Select a tab, list item, row or radio button (UIA SelectionItem).
    Select {
        binding_id: String,
        target: ComputerTarget,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    /// Expand a tree item, combobox or menu (UIA ExpandCollapse).
    Expand {
        binding_id: String,
        target: ComputerTarget,
        #[serde(default)]
        feedback: ComputerFeedback,
        #[serde(default)]
        feedback_mode: Option<ComputerObservationMode>,
        #[serde(default = "default_timeout")]
        execution_timeout_ms: u64,
    },
    Collapse {
        binding_id: String,
        target: ComputerTarget,
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
            | Self::WaitFor { binding_id, .. }
            | Self::Invoke { binding_id, .. }
            | Self::SetValue { binding_id, .. }
            | Self::Toggle { binding_id, .. }
            | Self::Select { binding_id, .. }
            | Self::Expand { binding_id, .. }
            | Self::Collapse { binding_id, .. } => binding_id,
        }
    }

    pub fn command(&self) -> &'static str {
        match self {
            Self::Inspect { .. } => "inspect",
            Self::Observe { .. } => "observe",
            Self::Click { .. } => "click",
            Self::Type { .. } => "type",
            Self::Keypress { .. } => "keypress",
            Self::Scroll { .. } => "scroll",
            Self::WaitFor { .. } => "wait_for",
            Self::Invoke { .. } => "invoke",
            Self::SetValue { .. } => "set_value",
            Self::Toggle { .. } => "toggle",
            Self::Select { .. } => "select",
            Self::Expand { .. } => "expand",
            Self::Collapse { .. } => "collapse",
        }
    }

    /// The element action a semantic command performs; the target element's
    /// observation must have declared it.
    pub fn element_action(&self) -> Option<ComputerElementAction> {
        Some(match self {
            Self::Invoke { .. } => ComputerElementAction::Invoke,
            Self::SetValue { .. } => ComputerElementAction::SetValue,
            Self::Toggle { .. } => ComputerElementAction::Toggle,
            Self::Select { .. } => ComputerElementAction::Select,
            Self::Expand { .. } => ComputerElementAction::Expand,
            Self::Collapse { .. } => ComputerElementAction::Collapse,
            _ => return None,
        })
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
            }
            | Self::Invoke {
                execution_timeout_ms,
                ..
            }
            | Self::SetValue {
                execution_timeout_ms,
                ..
            }
            | Self::Toggle {
                execution_timeout_ms,
                ..
            }
            | Self::Select {
                execution_timeout_ms,
                ..
            }
            | Self::Expand {
                execution_timeout_ms,
                ..
            }
            | Self::Collapse {
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
            | Self::WaitFor { target, .. }
            | Self::Invoke { target, .. }
            | Self::SetValue { target, .. }
            | Self::Toggle { target, .. }
            | Self::Select { target, .. }
            | Self::Expand { target, .. }
            | Self::Collapse { target, .. } => Some(target),
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
            }
            | Self::Invoke {
                feedback,
                feedback_mode,
                ..
            }
            | Self::SetValue {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Toggle {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Select {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Expand {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Collapse {
                feedback,
                feedback_mode,
                ..
            } => (
                *feedback,
                feedback_mode.unwrap_or(ComputerObservationMode::Auto),
            ),
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
            if !valid_observation_id(target.observation_id()) {
                return Err("invalid observation_id".into());
            }
            if target.element_ref().is_some_and(|r| !valid_ref(r)) {
                return Err("invalid element ref".into());
            }
        }
        let element = matches!(self.target(), Some(ComputerTarget::Element { .. }));
        match self {
            Self::Observe {
                scope:
                    Some(ComputerObserveScope::Window {
                        observation_id,
                        window_ref,
                    }),
                ..
            } => {
                if !valid_observation_id(observation_id) || !valid_ref(window_ref) {
                    return Err("observe scope requires an observation_id and window ref".into());
                }
            }
            Self::Click {
                target,
                count,
                modifiers,
                ..
            } => {
                if matches!(target, ComputerTarget::Focus { .. }) || !(1..=2).contains(count) {
                    return Err("click requires a point or element target and count 1 or 2".into());
                }
                validate_modifiers(modifiers)?;
            }
            Self::Type { target, text, .. } => {
                if matches!(target, ComputerTarget::Point { .. })
                    || text.is_empty()
                    || text.len() > COMPUTER_MAX_TEXT_BYTES
                    || text.contains('\0')
                {
                    return Err(
                        "type requires focus or element and 1..16384 UTF-8 bytes without NUL"
                            .into(),
                    );
                }
            }
            Self::Keypress { target, keys, .. } => {
                if matches!(target, ComputerTarget::Point { .. })
                    || keys.is_empty()
                    || keys.len() > 5
                {
                    return Err(
                        "keypress requires focus or element and modifiers plus one ordinary key"
                            .into(),
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
                if matches!(target, ComputerTarget::Focus { .. })
                    || delta_x.unsigned_abs() > 10_000
                    || delta_y.unsigned_abs() > 10_000
                    || (*delta_x == 0 && *delta_y == 0)
                {
                    return Err(
                        "scroll requires a point or element and nonzero deltas within +/-10000"
                            .into(),
                    );
                }
            }
            Self::WaitFor {
                condition: ComputerWaitCondition::Delay { duration_ms },
                execution_timeout_ms,
                ..
            } => {
                if element {
                    return Err("wait_for does not track elements yet".into());
                }
                if *duration_ms > *execution_timeout_ms {
                    return Err("delay exceeds execution timeout".into());
                }
            }
            Self::Invoke { .. }
            | Self::Toggle { .. }
            | Self::Select { .. }
            | Self::Expand { .. }
            | Self::Collapse { .. } => {
                if !element {
                    return Err(format!("{} requires an element target", self.command()));
                }
            }
            Self::SetValue { value, .. } => {
                if !element || value.len() > COMPUTER_MAX_TEXT_BYTES || value.contains('\0') {
                    return Err(
                        "set_value requires an element and at most 16384 UTF-8 bytes without NUL"
                            .into(),
                    );
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
            }
            | Self::Invoke {
                feedback,
                feedback_mode,
                ..
            }
            | Self::SetValue {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Toggle {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Select {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Expand {
                feedback,
                feedback_mode,
                ..
            }
            | Self::Collapse {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerExecutionState {
    NotStarted,
    InProgress,
    Completed,
    Partial,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerExecution {
    pub state: ComputerExecutionState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ComputerFeedbackState {
    NotRequested,
    Pending,
    Included,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerFeedbackResult {
    pub state: ComputerFeedbackState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerCapability {
    pub supported: bool,
    pub ready: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerCapabilities {
    pub capture: ComputerCapability,
    pub input: ComputerCapability,
    pub accessibility: ComputerCapability,
    pub observation_modes: Vec<ComputerObservationMode>,
    pub commands: Vec<String>,
    pub focus_guard: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerScreenshot {
    pub artifact_ref: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
    pub space: ComputerCoordinateSpace,
    pub interactive: bool,
    pub captured_at_ms: u64,
}

/// A rectangle in the observation's screenshot coordinate space (the space
/// of point targets); it may extend past the display.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerBounds {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerWindow {
    #[serde(rename = "ref")]
    pub window_ref: String,
    pub title: String,
    pub process: String,
    pub pid: u32,
    pub bounds: ComputerBounds,
    pub foreground: bool,
    /// False for windows this user process may not read (higher integrity,
    /// another account): listed without a control tree.
    pub accessible: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerSemanticCoverage {
    pub visited: u32,
    pub returned: u32,
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The semantic part of an observation: one outline line per element
/// (`[ref] role "name" ... (actions) @x,y wxh`), the window list and coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ComputerSemantic {
    pub format: String,
    pub text: String,
    pub windows: Vec<ComputerWindow>,
    pub coverage: ComputerSemanticCoverage,
    pub captured_at_ms: u64,
}

impl ComputerSemantic {
    pub fn within_limit(&self) -> bool {
        self.format == COMPUTER_SEMANTIC_FORMAT
            && self.text.len() <= COMPUTER_SEMANTIC_MAX_TEXT_BYTES
            && self.windows.len() <= COMPUTER_MAX_WINDOWS
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= COMPUTER_MAX_SEMANTIC_BYTES)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screenshot: Option<ComputerScreenshot>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic: Option<ComputerSemantic>,
    pub focus_guard: String,
    pub expires_at_ms: u64,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
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

    fn element(command: &str) -> serde_json::Value {
        json!({"command":command,"binding_id":"dev:display:1","target":{"kind":"element","observation_id":"obs","ref":"e12"}})
    }

    #[test]
    fn element_targets_follow_command_rules() {
        let mut clicked = element("click");
        assert!(serde_json::from_value::<ComputerRequest>(clicked.clone())
            .unwrap()
            .validate()
            .is_ok());
        clicked["target"]["ref"] = json!("e1 2");
        assert!(serde_json::from_value::<ComputerRequest>(clicked)
            .unwrap()
            .validate()
            .is_err());
        let mut typed = element("type");
        typed["text"] = json!("hello");
        let mut keys = element("keypress");
        keys["keys"] = json!(["Control", "a"]);
        let mut scroll = element("scroll");
        scroll["delta_x"] = json!(0);
        scroll["delta_y"] = json!(3);
        scroll["unit"] = json!("line");
        let mut set_value = element("set_value");
        set_value["value"] = json!("");
        for valid in [typed, keys, scroll, element("invoke"), set_value.clone()] {
            let request: ComputerRequest = serde_json::from_value(valid).unwrap();
            assert!(request.validate().is_ok(), "{request:?}");
            assert_eq!(request.target().unwrap().element_ref(), Some("e12"));
            assert_eq!(request.feedback().1, ComputerObservationMode::Auto);
        }
        set_value["value"] = json!("x".repeat(COMPUTER_MAX_TEXT_BYTES + 1));
        let mut wait = element("wait_for");
        wait["condition"] = json!({"kind":"delay","duration_ms":10});
        let mut invalid = vec![set_value, wait];
        for command in ["invoke", "toggle", "select", "expand", "collapse"] {
            let mut point = click();
            point["command"] = json!(command);
            invalid.push(point);
        }
        for invalid in invalid {
            assert!(serde_json::from_value::<ComputerRequest>(invalid)
                .unwrap()
                .validate()
                .is_err());
        }
        let mut misnamed = element("invoke");
        misnamed["target"] = json!({"kind":"element","observation_id":"obs","element_ref":"e1"});
        assert!(serde_json::from_value::<ComputerRequest>(misnamed).is_err());
    }

    #[test]
    fn semantic_commands_are_named_after_their_element_action() {
        for (command, action) in [
            ("invoke", ComputerElementAction::Invoke),
            ("set_value", ComputerElementAction::SetValue),
            ("toggle", ComputerElementAction::Toggle),
            ("select", ComputerElementAction::Select),
            ("expand", ComputerElementAction::Expand),
            ("collapse", ComputerElementAction::Collapse),
        ] {
            let mut value = element(command);
            if command == "set_value" {
                value["value"] = json!("x");
            }
            let request: ComputerRequest = serde_json::from_value(value).unwrap();
            assert!(request.validate().is_ok(), "{command}");
            assert_eq!((request.command(), request.element_action()), (command, Some(action)));
            assert_eq!(action.as_str(), command);
            assert_eq!(request.required_op(), ExecutorOp::ComputerAct);
            assert_eq!(request.feedback().1, ComputerObservationMode::Auto);
        }
        assert_eq!(
            serde_json::from_value::<ComputerRequest>(element("click"))
                .unwrap()
                .element_action(),
            None
        );
        assert!(serde_json::from_value::<ComputerRequest>(element("scroll_into_view")).is_err());
        let mut silent = element("toggle");
        silent["feedback"] = json!("none");
        silent["feedback_mode"] = json!("semantic");
        assert!(serde_json::from_value::<ComputerRequest>(silent)
            .unwrap()
            .validate()
            .is_err());
    }

    #[test]
    fn semantic_commands_need_control_and_scope_is_validated() {
        let invoke: ComputerRequest = serde_json::from_value(element("invoke")).unwrap();
        assert_eq!(invoke.required_op(), ExecutorOp::ComputerAct);
        assert!(invoke.required_op().is_mutating());
        assert!(!invoke
            .required_op()
            .allowed_by(crate::AgentPermissionMode::ReadOnly));
        let observe: ComputerRequest = serde_json::from_value(json!({
            "command":"observe","binding_id":"dev:display:1","mode":"semantic",
            "scope":{"kind":"window","observation_id":"obs","ref":"w3"}
        }))
        .unwrap();
        assert!(observe.validate().is_ok());
        assert_eq!(observe.required_op(), ExecutorOp::ComputerObserve);
        let plain: ComputerRequest =
            serde_json::from_value(json!({"command":"observe","binding_id":"dev:display:1"}))
                .unwrap();
        assert!(serde_json::to_value(&plain).unwrap().get("scope").is_none());
        assert!(serde_json::from_value::<ComputerRequest>(json!({
            "command":"observe","binding_id":"dev:display:1",
            "scope":{"kind":"window","observation_id":"obs","ref":""}
        }))
        .unwrap()
        .validate()
        .is_err());
        assert!(serde_json::from_value::<ComputerRequest>(json!({
            "command":"observe","binding_id":"dev:display:1","scope":{"kind":"desktop"}
        }))
        .is_err());
    }

    #[test]
    fn common_names_serialize_as_their_outline_names() {
        for role in ComputerRole::ALL {
            assert_eq!(serde_json::to_value(role).unwrap(), json!(role.as_str()));
        }
        for state in ComputerElementState::ALL {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(state.as_str()));
        }
        for action in ComputerElementAction::ALL {
            assert_eq!(serde_json::to_value(action).unwrap(), json!(action.as_str()));
        }
        assert_eq!(ComputerElementAction::SetValue.as_str(), "set_value");
        assert_eq!(ComputerElementAction::ScrollIntoView.as_str(), "scroll_into_view");
        assert_eq!(ComputerRole::ALL.len(), 29);
    }

    #[test]
    fn semantic_payload_has_its_own_limit_below_the_control_frame() {
        assert!(COMPUTER_MAX_SEMANTIC_BYTES < crate::EXECUTOR_MAX_CONTROL_FRAME);
        let mut semantic = ComputerSemantic {
            format: COMPUTER_SEMANTIC_FORMAT.into(),
            text: "[e1] window \"Notepad\" @0,0 800x600".into(),
            windows: vec![],
            coverage: ComputerSemanticCoverage {
                visited: 1,
                returned: 1,
                truncated: false,
                reason: None,
            },
            captured_at_ms: 1,
        };
        assert!(semantic.within_limit());
        semantic.text = "x".repeat(COMPUTER_SEMANTIC_MAX_TEXT_BYTES + 1);
        assert!(!semantic.within_limit());
        semantic.text.clear();
        semantic.format = "json".into();
        assert!(!semantic.within_limit());
        let observation: ComputerObservation = serde_json::from_value(json!({
            "observation_id":"obs","binding_id":"b","surface_id":"display:1","surface_epoch":1,
            "interaction_revision":1,"requested_mode":"auto","effective_mode":"semantic",
            "selection_reason":"semantic_available","capture_started_at_ms":1,"capture_completed_at_ms":2,
            "consistency":"best_effort","focus_guard":"surface","expires_at_ms":3,"warnings":[],
            "semantic":{"format":"outline-v1","text":"","windows":[{"ref":"w1","title":"t","process":"p.exe",
                "pid":4,"bounds":{"x":-8,"y":0,"width":10,"height":10},"foreground":true,"accessible":true}],
                "coverage":{"visited":0,"returned":0,"truncated":false},"captured_at_ms":2}
        }))
        .unwrap();
        assert!(observation.screenshot.is_none());
        assert_eq!(observation.semantic.unwrap().windows[0].bounds.x, -8);
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
