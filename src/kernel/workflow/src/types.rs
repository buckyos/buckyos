use crate::dsl::{GuardConfig, OutputMode, RetryFallback, RetryGuard, StepType};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RefPath {
    pub node_id: String,
    pub field_path: Vec<String>,
}

impl RefPath {
    pub fn parse(raw: &str) -> Option<Self> {
        if !raw.starts_with("${") || !raw.ends_with('}') {
            return None;
        }
        let inner = &raw[2..raw.len() - 1];
        let mut parts = inner.split('.');
        let node_id = parts.next()?.to_string();
        match parts.next() {
            Some("output") => {}
            _ => return None,
        }
        let field_path = parts.map(|part| part.to_string()).collect::<Vec<_>>();
        if node_id.is_empty() {
            return None;
        }
        Some(Self {
            node_id,
            field_path,
        })
    }

    pub fn as_string(&self) -> String {
        if self.field_path.is_empty() {
            format!("${{{}.output}}", self.node_id)
        } else {
            format!("${{{}.output.{}}}", self.node_id, self.field_path.join("."))
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ValueTemplate {
    Literal(Value),
    Reference(RefPath),
    Array(Vec<ValueTemplate>),
    Object(BTreeMap<String, ValueTemplate>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum AwaitKind {
    Confirm,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JoinStrategy {
    All,
    Any,
    NOfM(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Expr {
    Apply {
        executor: String,
        fun_id: String,
        params: BTreeMap<String, ValueTemplate>,
        output_mode: OutputMode,
        idempotent: bool,
        step_type: StepType,
        guards: GuardConfig,
    },
    Match {
        on: RefPath,
        cases: BTreeMap<String, String>,
        max_iterations: u32,
    },
    Par {
        branches: Vec<String>,
        join: JoinStrategy,
    },
    Map {
        collection: RefPath,
        steps: Vec<String>,
        max_items: u32,
        concurrency: u32,
        actual_concurrency: u32,
    },
    Await {
        kind: AwaitKind,
        subject: Option<RefPath>,
        prompt: Option<String>,
        output_schema: Value,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub fallback: RetryFallback,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            fallback: RetryFallback::Human,
        }
    }
}

impl From<Option<RetryGuard>> for RetryPolicy {
    fn from(value: Option<RetryGuard>) -> Self {
        let Some(value) = value else {
            return Self::default();
        };
        Self {
            max_attempts: value.max_attempts.max(1),
            fallback: value.fallback.unwrap_or_default(),
        }
    }
}
