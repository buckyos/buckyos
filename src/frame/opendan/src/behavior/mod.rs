pub mod behavior;
pub mod config;
pub mod observability;
pub mod parser;
pub mod policy_adapter;
pub mod prompt;
pub mod sanitize;
pub mod tool_loop;
pub mod types;

pub use behavior::*;
pub use config::{BehaviorConfig, BehaviorConfigError};
pub use observability::{Event, WorklogSink};
pub use parser::BehaviorResultParser;
pub use policy_adapter::PolicyEngine;
pub use prompt::{ChatMessage, ChatRole, PromptBuilder, PromptPack, Truncator};
pub use sanitize::Sanitizer;
pub use tool_loop::ToolContext;
pub use types::*;

#[cfg(test)]
mod tests;
