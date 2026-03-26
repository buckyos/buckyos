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
pub struct ThunkParams {
    pub param_type: ThunkParamType,
    pub values: Value,
    #[serde(default)]
    pub obj_refs: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThunkParamType {
    Fixed,
    Normal,
    CheckByRunner,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequirements {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_duration: Option<String>,
    #[serde(default)]
    pub gpu_required: bool,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkMetadata {
    pub run_id: String,
    pub node_id: String,
    pub attempt: u32,
    #[serde(default)]
    pub shard: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkObject {
    pub thunk_obj_id: String,
    pub fun_id: String,
    pub params: ThunkParams,
    pub idempotent: bool,
    pub resource_requirements: ResourceRequirements,
    pub metadata: ThunkMetadata,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThunkMetrics {
    #[serde(default)]
    pub tokens_used: Option<u64>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub cost_usdb: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThunkExecutionStatus {
    Success,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkExecutionResult {
    pub thunk_obj_id: String,
    pub status: ThunkExecutionStatus,
    #[serde(default)]
    pub result_obj_id: Option<String>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub metrics: ThunkMetrics,
    #[serde(default)]
    pub side_effect_receipt: Option<Value>,
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
