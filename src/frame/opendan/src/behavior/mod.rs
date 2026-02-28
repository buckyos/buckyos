pub mod behavior;
pub mod config;
pub mod observability;
pub mod policy_adapter;
pub mod prompt;
pub mod sanitize;
pub mod tool_loop;
pub mod types;

pub use crate::agent_tool::{ActionExecutionMode, ActionKind, ActionSpec, FsScope};
pub use behavior::*;
pub use config::{BehaviorConfig, BehaviorConfigError};
pub use observability::{AgentWorkEvent, WorklogSink};
pub use policy_adapter::PolicyEngine;
pub use prompt::{ChatMessage, ChatRole, PromptBuilder, Truncator};
pub use sanitize::Sanitizer;
pub use tool_loop::ToolContext;
pub use types::*;

#[cfg(test)]
mod tests;
