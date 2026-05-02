//! Workflow DSL definition types.
//!
//! 这些类型刻画 Workflow Service 接收的 DSL JSON 形态（schema_version、steps、
//! nodes、edges、guards 等），是“客户端 → workflow service”这条链路上的 wire
//! schema，所以放在 buckyos-api 里给所有提交 / 查询 workflow 定义的组件共享。
//! workflow crate 内部也通过这里复用，避免再造一份。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDefinition {
    pub schema_version: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub trigger: Value,
    pub steps: Vec<StepDefinition>,
    #[serde(default)]
    pub nodes: Vec<ControlNodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
    #[serde(default)]
    pub guards: Option<GuardConfig>,
    #[serde(default)]
    pub defs: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepDefinition {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub executor: Option<String>,
    #[serde(rename = "type")]
    pub step_type: StepType,
    #[serde(default)]
    pub input: Option<Value>,
    #[serde(default)]
    pub input_schema: Option<Value>,
    pub output_schema: Value,
    #[serde(default)]
    pub subject_ref: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default = "default_true")]
    pub idempotent: bool,
    #[serde(default = "default_true")]
    pub skippable: bool,
    #[serde(default)]
    pub output_mode: OutputMode,
    #[serde(default)]
    pub guards: Option<GuardConfig>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StepType {
    Autonomous,
    HumanConfirm,
    HumanRequired,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    #[default]
    Single,
    FiniteSeekable,
    FiniteSequential,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ControlNodeDefinition {
    #[serde(rename = "branch")]
    Branch(BranchNodeDefinition),
    #[serde(rename = "parallel")]
    Parallel(ParallelNodeDefinition),
    #[serde(rename = "for_each")]
    ForEach(ForEachNodeDefinition),
}

impl ControlNodeDefinition {
    pub fn id(&self) -> &str {
        match self {
            Self::Branch(node) => &node.id,
            Self::Parallel(node) => &node.id,
            Self::ForEach(node) => &node.id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchNodeDefinition {
    pub id: String,
    pub on: String,
    pub paths: BTreeMap<String, String>,
    pub max_iterations: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParallelNodeDefinition {
    pub id: String,
    pub branches: Vec<String>,
    pub join: JoinMode,
    #[serde(default)]
    pub n: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForEachNodeDefinition {
    pub id: String,
    pub items: String,
    pub steps: Vec<String>,
    pub max_items: u32,
    #[serde(default = "default_one")]
    pub concurrency: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JoinMode {
    All,
    Any,
    NOfM,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeDefinition {
    pub from: String,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuardConfig {
    #[serde(default)]
    pub budget: Option<BudgetGuard>,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub retry: Option<RetryGuard>,
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(default)]
    pub amendment_auto_approve: Option<bool>,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
    #[serde(default)]
    pub max_duration: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BudgetGuard {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
    #[serde(default)]
    pub max_duration: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetryGuard {
    #[serde(default = "default_retry_attempts")]
    pub max_attempts: u32,
    #[serde(default)]
    pub backoff: Option<String>,
    #[serde(default)]
    pub fallback: Option<RetryFallback>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetryFallback {
    Human,
    Abort,
}

impl Default for RetryFallback {
    fn default() -> Self {
        Self::Human
    }
}

pub fn default_true() -> bool {
    true
}

pub fn default_one() -> u32 {
    1
}

pub fn default_retry_attempts() -> u32 {
    1
}
