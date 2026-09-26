//! `LocalLLMContext` — the `xllm` SDK core (see `product/xllm/PRD.md`).
//!
//! 这个模块把 PRD 定义的“一次独立任务（Run）”落成可编程接口：
//!
//! ```text
//!   .llm_context 查找/合并 (F10)  ─┐
//!   提示词组 / section / 模板 (F11) ├─> XllmTask::prepare(...)  ─> PreparedTask
//!   工具 / actions / MCP (F04)      │        │
//!   Provider / 模型 (F03)          ─┘        ▼
//!                                   XllmRun::start(prepared, deps) ─> execute() ─> RunOutcome
//!                                   XllmRun::resume(runs_dir, run_id, ...)      ─> execute()
//!                                   list_runs / load_run / export_result (F06 / F07)
//! ```
//!
//! ## 分层
//!
//! 1. **配置层**：`.llm_context`（YAML）从工作目录向上逐级查找，按“最远父目录 →
//!    工作目录”合并成 [`MergedConfig`]，再叠加显式的 [`TaskOverrides`]。
//! 2. **组装层**：选组、计算有效工具、按行号拼接 system section、渲染模板、
//!    追加运行时协议，得到 [`PromptPlan`]；本次 user 提交由 [`TaskInput`] 组织。
//! 3. **执行层**：[`XllmRun`] 围绕 waist `LLMContext::run` 做 outcome 分发，
//!    每个 outcome 边界落盘，把 Provider 可恢复错误、中断、限制、完成映射到
//!    [`RunStatus`]。它不解释业务输出、不维护跨 Run 的记忆。
//! 4. **记录层**：Runs 目录中每个 Run 一个子目录（`run.json` + `snapshots/`），
//!    查询接口只读记录，不触发执行。
//!
//! ## Run 目录布局（新格式，不兼容旧 `runs/<id>/state.json`）
//!
//! ```text
//!   <runs_dir>/
//!   ├── <run_id>/
//!   │   ├── run.json              ← RunRecord：状态、输入、有效配置及来源、提示词、结果、错误
//!   │   ├── snapshots/0001.json   ← waist LLMContextSnapshot（轮前 checkpoint + outcome 边界）
//!   │   └── .lock                 ← 该 Run 的执行互斥（flock）
//!   └── ...
//!   <lock_dir>/<hash(workdir)>.lock  ← 启用工具的任务在同一工作目录内互斥
//! ```
//!
//! `runs_dir: none` 时不落盘：Run 只存在于内存，不能 resume，也不出现在列表里。
//!
//! ## 状态机
//!
//! | 状态 | 终态 | 说明 |
//! | --- | --- | --- |
//! | `Running` | 否 | 进程持锁执行中；进程消失后由锁判定为“已中断” |
//! | `Interrupted` | 否 | 用户中断 / 进程退出，保存了轮前快照 |
//! | `Paused` | 否 | Provider 超时、限流、临时故障或凭据问题；resume 重试未完成的请求 |
//! | `Completed` | 是 | 最终响应已保存并按 result_format 提取 |
//! | `Failed` | 是 | 不可恢复错误 |
//! | `LimitReached` | 是 | 工具轮数 / 总时长 / token 预算耗尽 |
//!
//! 终态 Run 不会再次执行：`resume` 只返回已保存的结果。

use std::collections::{BTreeMap, HashMap};
use std::fs::{File, OpenOptions};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ::kRPC::RPCErrors;
use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime, AiContent,
    AiMessage, AiMethodStatus, AiResponse, AiRole, AiToolCall, AiToolSpec, AiUsage,
    AiccExecutionMode, BuckyOSRuntimeType, HelperModelRequirement, LlmChatHelperRequest,
    LlmResponseFormat, ModelDisable, ResourceRef,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use llm_context::behavior_loop::{LLMBehaviorResult, LLMResultParser, SendMessageRecord};
use llm_context::deps::{
    LLMContextDeps, LlmClient, LlmInferenceRequest, ToolDispatchError, ToolManager, ToolSpecLite,
    TurnHook, WorkEvent, WorklogSink,
};
use llm_context::error::{ErrorSource, LLMComputeError, ProviderFailure};
use llm_context::observation::Observation;
use llm_context::outcome::{BudgetKind, ContextOutput, LLMContextOutcome, ResumeFill};
use llm_context::request::{
    BudgetSpec, ContextOwnerRef, ContextThreshold, ErrorPolicy, LLMContextRequest, ModelPolicy,
    OutputSpec, ToolMode, ToolPolicy,
};
use llm_context::state::LLMContextSnapshot;
use llm_context::{LLMContext, LLMContextInterruptHandle, XmlStepRenderer};

use crate::llm_compress::LlmSummarizeCompressor;
use crate::tool::TypedToolHandle;
use crate::{
    AgentTool, AgentToolError, AgentToolResult, AgentToolStatus, BinOverlayConfig, EditFileTool,
    ExecBashTool, FileToolConfig, LlmBashConfig, NoopFileWriteAudit, ReadFileTool,
    SessionRuntimeContext, ToolSpec, WriteFileTool, TOOL_EDIT_FILE, TOOL_WRITE_FILE,
};

// =========================================================================
// 常量与产品默认值
// =========================================================================

/// 目录配置文件名。
pub const LLM_CONTEXT_FILE_NAME: &str = ".llm_context";
/// 工具轮数默认上限（F08）。
pub const DEFAULT_MAX_ROUNDS: u32 = 8;
/// 本次命令总执行时长默认上限，秒（F08）。
pub const DEFAULT_TIMEOUT_SECS: u64 = 3600;
/// 单次 LLM 请求默认超时，秒（F08）。
pub const DEFAULT_LLM_TIMEOUT_SECS: u64 = 600;
/// 默认 Runs 目录（F10）。
pub const DEFAULT_RUNS_DIR: &str = "~/.xllm/runs";
/// 默认锁目录：启用工具的任务按工作目录互斥，与 Runs 目录无关（F05）。
pub const DEFAULT_LOCK_DIR: &str = "~/.xllm/locks";
/// 内置工具组 `bash` 提供的工具名。
pub const TOOL_EXEC: &str = "exec";
pub const BUILTIN_TOOL_GROUP_BASH: &str = "bash";
/// 运行时协议版本；resume 时校验当前执行器是否能处理保存的协议。
pub const RUNTIME_PROTOCOL_VERSION: &str = "xllm/1";
/// `run.json` 记录格式版本。
pub const RUN_RECORD_VERSION: u32 = 1;
/// 默认 context 压缩阈值（token window 的 75%）。
pub const DEFAULT_CONTEXT_YIELD_RATIO: f32 = 0.75;
/// 默认连续可纠正错误上限。
pub const DEFAULT_MAX_CONSECUTIVE_ERRORS: u32 = 3;
/// 自动压缩目标 token 数兜底。
pub const DEFAULT_AUTO_COMPRESS_TARGET_TOKENS: u32 = 32_768;
/// 单个 `--file` 文本材料上限（字节）。
pub const MAX_TEXT_ATTACHMENT_BYTES: usize = 8 * 1024 * 1024;
/// 单张图片上限（字节）。
pub const MAX_IMAGE_ATTACHMENT_BYTES: usize = 20 * 1024 * 1024;
/// 固定别名 → 行号（F11.1）。
pub const SECTION_ALIASES: &[(&str, u32)] = &[
    ("role", 10),
    ("contexts", 20),
    ("env", 20),
    ("rules", 30),
    ("cmd_manual", 40),
    ("output_format", 100),
];

const EXEC_DEFAULT_TIMEOUT_MS: u64 = 1_800_000;
const EXEC_MAX_TIMEOUT_MS: u64 = 3_600_000;
const EXEC_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const MCP_DISCOVERY_TIMEOUT_MS: u64 = 30_000;
const MCP_CALL_TIMEOUT_MS: u64 = 120_000;

// =========================================================================
// 错误类型
// =========================================================================

/// SDK 面向调用方的错误。凡是“尚未建立 Run”的预检错误（配置、输入、能力、
/// 工具来源）都在这里表达；Run 建立之后的执行结果通过 [`RunOutcome`] 表达，
/// 不走 `Err`。
#[derive(Debug, thiserror::Error)]
pub enum XllmError {
    /// `.llm_context`（或外部组文件）无效：指出文件、字段和原因。
    #[error("config error: {file}: {field}: {reason}")]
    Config {
        file: String,
        field: String,
        reason: String,
    },
    /// 本次输入无效（附件不存在、空任务、参数互斥等）。
    #[error("input error: {0}")]
    Input(String),
    /// Provider / 模型 / loop / 输出约束组合无效。
    #[error("capability error: {0}")]
    Capability(String),
    /// 工具来源、名称冲突、MCP 发现失败等。
    #[error("tools error: {0}")]
    Tools(String),
    /// 模板变量缺失或语法错误。
    #[error("template error in {origin}: {reason}")]
    Template { origin: String, reason: String },
    #[error("run `{run_id}` not found under {runs_dir}")]
    RunNotFound { run_id: String, runs_dir: String },
    #[error("run `{run_id}` is being executed by another process")]
    RunBusy { run_id: String },
    #[error(
        "working directory {workdir} already has a tool-enabled run `{run_id}` executing; wait for it or use another directory"
    )]
    WorkdirBusy { workdir: String, run_id: String },
    #[error("run `{run_id}` is terminal ({status}); {hint}")]
    RunTerminal {
        run_id: String,
        status: RunStatus,
        hint: String,
    },
    #[error("run `{run_id}` cannot be resumed: {reason}")]
    NotResumable { run_id: String, reason: String },
    #[error("run `{run_id}` record is corrupted: {reason}")]
    CorruptedRun { run_id: String, reason: String },
    #[error("storage error: {0}")]
    Storage(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("result extraction failed: {0}")]
    Extract(String),
    #[error("context compaction failed: {0}")]
    Compressor(String),
    #[error("{0}")]
    Other(String),
}

/// 上下文压缩策略抽象（`llm_compress::LlmSummarizeCompressor` 实现它）。
#[async_trait]
pub trait Compressor: Send + Sync {
    async fn compress(
        &self,
        accumulated: Vec<AiMessage>,
        dir: &Path,
    ) -> Result<Vec<AiMessage>, XllmError>;
}

impl XllmError {
    fn config(file: &Path, field: impl Into<String>, reason: impl Into<String>) -> Self {
        XllmError::Config {
            file: file.display().to_string(),
            field: field.into(),
            reason: reason.into(),
        }
    }
}

// =========================================================================
// 基础枚举
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Buckyos,
    Openai,
}

impl ProviderKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderKind::Buckyos => "buckyos",
            ProviderKind::Openai => "openai",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "buckyos" => Some(ProviderKind::Buckyos),
            "openai" => Some(ProviderKind::Openai),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoopModel {
    FunctionCall,
    Behavior,
}

impl LoopModel {
    pub fn as_str(&self) -> &'static str {
        match self {
            LoopModel::FunctionCall => "function_call",
            LoopModel::Behavior => "behavior",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "function_call" => Some(LoopModel::FunctionCall),
            "behavior" => Some(LoopModel::Behavior),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunLogLevel {
    Debug,
    Info,
    Warn,
    Result,
}

impl RunLogLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            RunLogLevel::Debug => "debug",
            RunLogLevel::Info => "info",
            RunLogLevel::Warn => "warn",
            RunLogLevel::Result => "result",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "debug" => Some(RunLogLevel::Debug),
            "info" => Some(RunLogLevel::Info),
            "warn" => Some(RunLogLevel::Warn),
            "result" => Some(RunLogLevel::Result),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptMode {
    Standard,
    Custom,
}

/// `result_format`：`raw` 或 `result.<path>`（F07）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResultFormat {
    Raw,
    Path { segments: Vec<String> },
}

impl ResultFormat {
    pub fn parse(s: &str) -> Result<Self, XllmError> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("raw") {
            return Ok(ResultFormat::Raw);
        }
        let Some(rest) = s.strip_prefix("result.") else {
            return Err(XllmError::Input(format!(
                "invalid result_format `{s}`: expected `raw` or `result.<path>`"
            )));
        };
        let segments: Vec<String> = rest.split('.').map(|p| p.trim().to_string()).collect();
        if segments.iter().any(|p| p.is_empty()) {
            return Err(XllmError::Input(format!(
                "invalid result_format `{s}`: empty path segment"
            )));
        }
        Ok(ResultFormat::Path { segments })
    }

    pub fn as_string(&self) -> String {
        match self {
            ResultFormat::Raw => "raw".to_string(),
            ResultFormat::Path { segments } => format!("result.{}", segments.join(".")),
        }
    }
}

/// Run 的用户可见状态（F06）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Running,
    Interrupted,
    Paused,
    Completed,
    Failed,
    LimitReached,
}

impl RunStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(
            self,
            RunStatus::Completed | RunStatus::Failed | RunStatus::LimitReached
        )
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Interrupted => "interrupted",
            RunStatus::Paused => "paused",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed",
            RunStatus::LimitReached => "limit_reached",
        }
    }

    /// User-facing status label.
    pub fn label(&self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Interrupted => "interrupted",
            RunStatus::Paused => "paused (recoverable error)",
            RunStatus::Completed => "completed",
            RunStatus::Failed => "failed (unrecoverable)",
            RunStatus::LimitReached => "limit reached",
        }
    }
}

impl std::fmt::Display for RunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// =========================================================================
// 配置文件模型（.llm_context）
// =========================================================================

/// 凭据引用：只保存来源，不保存值（F05）。`resolve()` 在需要时重新读取。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SecretRef {
    /// 手工写在配置文件里：记录文件路径与字段位置。
    ConfigFile { path: String, field: String },
    /// 来自环境变量。
    Env { name: String },
}

impl SecretRef {
    pub fn describe(&self) -> String {
        match self {
            SecretRef::ConfigFile { path, field } => format!("{path}:{field}"),
            SecretRef::Env { name } => format!("env:{name}"),
        }
    }

    /// 重新解析凭据值。配置文件来源会重新读取该文件的对应字段，这样修复
    /// 同一凭据引用后 resume 能生效。
    pub fn resolve(&self) -> Result<String, XllmError> {
        match self {
            SecretRef::Env { name } => std::env::var(name).map_err(|_| {
                XllmError::Capability(format!("environment variable `{name}` is not set"))
            }),
            SecretRef::ConfigFile { path, field } => {
                let raw = std::fs::read_to_string(path).map_err(|e| {
                    XllmError::Capability(format!(
                        "cannot re-read credential source {path}:{field}: {e}"
                    ))
                })?;
                let doc: serde_yaml::Value = serde_yaml::from_str(&raw).map_err(|e| {
                    XllmError::Capability(format!("cannot parse credential source {path}: {e}"))
                })?;
                let mut cur = &doc;
                for seg in field.split('.') {
                    cur = cur.get(seg).ok_or_else(|| {
                        XllmError::Capability(format!(
                            "credential field `{field}` no longer exists in {path}"
                        ))
                    })?;
                }
                cur.as_str()
                    .map(|s| s.to_string())
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| {
                        XllmError::Capability(format!(
                            "credential field `{field}` in {path} is empty or not a string"
                        ))
                    })
            }
        }
    }
}

/// Provider 接入配置（F03）。凭据只保留引用。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProviderConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ProviderKind>,
    /// buckyos：手工指定的 session_token 来源；`None` 表示沿用当前身份。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_token: Option<SecretRef>,
    /// openai：API 基地址（默认 `https://api.openai.com/v1`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// openai：API key 来源（文件中的 `api_key` 或 `api_key_env` 指定的环境变量）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<SecretRef>,
    /// openai：附加 HTTP 头。
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
}

impl ProviderConfig {
    pub fn effective_kind(&self) -> ProviderKind {
        self.kind.unwrap_or(ProviderKind::Buckyos)
    }
}

/// 工具来源（F04）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ToolSource {
    /// 内置工具组，如 `bash`。
    Group { groupname: String },
    /// MCP 服务地址；展开为该服务提供的全部工具。
    Mcp { endpoint: String },
    /// 单个已注册工具（由运行环境提供）。
    Named { name: String },
}

impl ToolSource {
    pub fn describe(&self) -> String {
        match self {
            ToolSource::Group { groupname } => format!("groupname:{groupname}"),
            ToolSource::Mcp { endpoint } => format!("mcp:{endpoint}"),
            ToolSource::Named { name } => format!("name:{name}"),
        }
    }
}

/// exec 命令手册条目（F04 `bash_tools`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BashToolManual {
    pub name: String,
    pub description: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemPolicy {
    #[default]
    Workspace,
    Unrestricted,
}

/// 工具配置对象；顶层、组内、`prompt.tools` 三处共用同一形状。
/// `None` 表示未声明（继承），`Some(vec![])` 表示显式清空。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filesystem_policy: Option<FilesystemPolicy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools2actions: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<ToolSource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<ToolSource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bash_tools: Option<Vec<BashToolManual>>,
}

impl ToolsConfig {
    /// 对象按字段合并：后层声明的字段覆盖前层；列表整体替换。
    pub fn merge_over(&mut self, over: &ToolsConfig) {
        if over.enabled.is_some() {
            self.enabled = over.enabled;
        }
        if over.filesystem_policy.is_some() {
            self.filesystem_policy = over.filesystem_policy;
        }
        if over.tools2actions.is_some() {
            self.tools2actions = over.tools2actions;
        }
        if over.tools.is_some() {
            self.tools = over.tools.clone();
        }
        if over.actions.is_some() {
            self.actions = over.actions.clone();
        }
        if over.bash_tools.is_some() {
            self.bash_tools = over.bash_tools.clone();
        }
    }
}

/// 一个 system section 的声明（已归一到行号）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionDef {
    /// 用户书写的键（别名或数字），用于诊断。
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `None` = 未声明文本（继承）；`Some("")` = 显式清空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// 声明该 section 的文件。
    pub source: String,
}

/// 提示词组（内联或外部文件展开后）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GroupDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_user: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<u32, SectionDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsConfig>,
    /// 组内容来源：内联时是 `.llm_context` 路径，外部文件时是该文件路径。
    #[serde(default)]
    pub sources: Vec<String>,
}

impl GroupDef {
    fn merge_over(&mut self, over: &GroupDef) {
        if over.default_user.is_some() {
            self.default_user = over.default_user.clone();
        }
        for (line, def) in &over.sections {
            self.sections.insert(*line, def.clone());
        }
        match (&mut self.tools, &over.tools) {
            (Some(base), Some(o)) => base.merge_over(o),
            (None, Some(o)) => self.tools = Some(o.clone()),
            _ => {}
        }
        for s in &over.sources {
            if !self.sources.contains(s) {
                self.sources.push(s.clone());
            }
        }
    }
}

/// `prompt` 块。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PromptConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<PromptMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub groups: BTreeMap<String, GroupDef>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub sections: BTreeMap<u32, SectionDef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

/// Runs 目录设置：显式 `none` 表示不落盘。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunsDirSetting {
    Disabled,
    Path { path: PathBuf },
}

/// 一份 `.llm_context` 解析后的内容（路径已按声明目录解析为绝对路径，
/// 外部组已展开，section 键已归一到行号）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LlmContextFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_dir: Option<RunsDirSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_model: Option<LoopModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_logs: Option<RunLogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_format: Option<ResultFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<PromptConfig>,
}

/// 一层配置：文件路径 + 解析结果。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConfigLayer {
    pub path: PathBuf,
    pub file: LlmContextFile,
}

// -------------------------------------------------------------------------
// YAML → 类型：手工转换，以便给出“文件 / 字段 / 原因”级别的诊断。
// -------------------------------------------------------------------------

type Yaml = serde_yaml::Value;

fn yaml_kind(v: &Yaml) -> &'static str {
    match v {
        Yaml::Null => "null",
        Yaml::Bool(_) => "bool",
        Yaml::Number(_) => "number",
        Yaml::String(_) => "string",
        Yaml::Sequence(_) => "list",
        Yaml::Mapping(_) => "object",
        Yaml::Tagged(_) => "tagged value",
    }
}

struct YamlCtx<'a> {
    file: &'a Path,
}

impl<'a> YamlCtx<'a> {
    fn err(&self, field: &str, reason: impl Into<String>) -> XllmError {
        XllmError::config(self.file, field, reason)
    }

    fn expect_map<'v>(
        &self,
        field: &str,
        v: &'v Yaml,
    ) -> Result<&'v serde_yaml::Mapping, XllmError> {
        match v {
            Yaml::Mapping(m) => Ok(m),
            other => Err(self.err(
                field,
                format!("expected an object, got {}", yaml_kind(other)),
            )),
        }
    }

    fn check_keys(
        &self,
        field: &str,
        m: &serde_yaml::Mapping,
        allowed: &[&str],
    ) -> Result<(), XllmError> {
        for k in m.keys() {
            let key = match k {
                Yaml::String(s) => s.clone(),
                Yaml::Number(n) => n.to_string(),
                other => {
                    return Err(self.err(
                        field,
                        format!("object keys must be strings, got {}", yaml_kind(other)),
                    ))
                }
            };
            if !allowed.contains(&key.as_str()) {
                return Err(self.err(
                    &join_field(field, &key),
                    format!("unknown field; allowed: {}", allowed.join(", ")),
                ));
            }
        }
        Ok(())
    }

    fn get_str(
        &self,
        field: &str,
        m: &serde_yaml::Mapping,
        key: &str,
    ) -> Result<Option<String>, XllmError> {
        match m.get(key) {
            None | Some(Yaml::Null) => Ok(None),
            Some(Yaml::String(s)) => Ok(Some(s.clone())),
            Some(Yaml::Number(n)) => Ok(Some(n.to_string())),
            Some(Yaml::Bool(b)) => Ok(Some(b.to_string())),
            Some(other) => Err(self.err(
                &join_field(field, key),
                format!("expected a string, got {}", yaml_kind(other)),
            )),
        }
    }

    fn get_bool(
        &self,
        field: &str,
        m: &serde_yaml::Mapping,
        key: &str,
    ) -> Result<Option<bool>, XllmError> {
        match m.get(key) {
            None | Some(Yaml::Null) => Ok(None),
            Some(Yaml::Bool(b)) => Ok(Some(*b)),
            Some(other) => Err(self.err(
                &join_field(field, key),
                format!("expected true/false, got {}", yaml_kind(other)),
            )),
        }
    }

    fn get_u64(
        &self,
        field: &str,
        m: &serde_yaml::Mapping,
        key: &str,
    ) -> Result<Option<u64>, XllmError> {
        match m.get(key) {
            None | Some(Yaml::Null) => Ok(None),
            Some(Yaml::Number(n)) => n
                .as_u64()
                .ok_or_else(|| {
                    self.err(
                        &join_field(field, key),
                        "expected a non-negative integer".to_string(),
                    )
                })
                .map(Some),
            Some(other) => Err(self.err(
                &join_field(field, key),
                format!("expected an integer, got {}", yaml_kind(other)),
            )),
        }
    }

    fn get_u32(
        &self,
        field: &str,
        m: &serde_yaml::Mapping,
        key: &str,
    ) -> Result<Option<u32>, XllmError> {
        match self.get_u64(field, m, key)? {
            None => Ok(None),
            Some(v) => u32::try_from(v)
                .map(Some)
                .map_err(|_| self.err(&join_field(field, key), "value too large")),
        }
    }
}

fn join_field(base: &str, key: &str) -> String {
    if base.is_empty() {
        key.to_string()
    } else {
        format!("{base}.{key}")
    }
}

/// 解析一个 section 键：数字行号或固定别名。
pub fn resolve_section_key(key: &str) -> Result<u32, String> {
    let k = key.trim();
    if k.is_empty() {
        return Err("empty section key".to_string());
    }
    if k.chars().all(|c| c.is_ascii_digit()) {
        return match k.parse::<u32>() {
            Ok(0) => Err("line number must be a positive integer".to_string()),
            Ok(n) => Ok(n),
            Err(_) => Err(format!("invalid line number `{k}`")),
        };
    }
    for (alias, line) in SECTION_ALIASES {
        if *alias == k {
            return Ok(*line);
        }
    }
    Err(format!(
        "unknown section alias `{k}` (fixed aliases: {}; or use a numeric line number)",
        SECTION_ALIASES
            .iter()
            .map(|(a, l)| format!("{a}={l}"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn alias_line(name: &str) -> Option<u32> {
    SECTION_ALIASES
        .iter()
        .find(|(a, _)| *a == name)
        .map(|(_, l)| *l)
}

/// 行号的标题名：固定别名优先，否则 `section_<line>`。
pub fn section_default_name(line: u32) -> String {
    match line {
        10 => "role".to_string(),
        20 => "contexts".to_string(),
        30 => "rules".to_string(),
        40 => "cmd_manual".to_string(),
        100 => "output_format".to_string(),
        n => format!("section_{n}"),
    }
}

fn parse_sections(
    ctx: &YamlCtx<'_>,
    field: &str,
    v: &Yaml,
    source: &Path,
) -> Result<BTreeMap<u32, SectionDef>, XllmError> {
    let m = ctx.expect_map(field, v)?;
    let mut out: BTreeMap<u32, SectionDef> = BTreeMap::new();
    for (k, entry) in m {
        let key = match k {
            Yaml::String(s) => s.clone(),
            Yaml::Number(n) => n.to_string(),
            other => {
                return Err(ctx.err(
                    field,
                    format!("section keys must be strings, got {}", yaml_kind(other)),
                ))
            }
        };
        let entry_field = join_field(field, &key);
        let line = resolve_section_key(&key).map_err(|reason| ctx.err(&entry_field, reason))?;
        let (name, text) = match entry {
            Yaml::String(s) => (None, Some(s.clone())),
            Yaml::Null => (None, Some(String::new())),
            Yaml::Mapping(obj) => {
                ctx.check_keys(&entry_field, obj, &["name", "text"])?;
                let name = ctx.get_str(&entry_field, obj, "name")?;
                let text = match obj.get("text") {
                    None => None,
                    Some(Yaml::Null) => Some(String::new()),
                    Some(Yaml::String(s)) => Some(s.clone()),
                    Some(other) => {
                        return Err(ctx.err(
                            &join_field(&entry_field, "text"),
                            format!("expected a string, got {}", yaml_kind(other)),
                        ))
                    }
                };
                (name, text)
            }
            other => {
                return Err(ctx.err(
                    &entry_field,
                    format!(
                        "expected a string or an object with `name`/`text`, got {}",
                        yaml_kind(other)
                    ),
                ))
            }
        };
        if let Some(n) = name.as_deref() {
            if let Some(alias_l) = alias_line(n) {
                if alias_l != line {
                    return Err(ctx.err(
                        &entry_field,
                        format!(
                            "name `{n}` is a fixed alias bound to line {alias_l}, but this section is at line {line}"
                        ),
                    ));
                }
            }
        }
        if let Some(prev) = out.get(&line) {
            return Err(ctx.err(
                field,
                format!(
                    "sections `{}` and `{}` both resolve to line {line}",
                    prev.key, key
                ),
            ));
        }
        out.insert(
            line,
            SectionDef {
                key,
                name,
                text,
                source: source.display().to_string(),
            },
        );
    }
    Ok(out)
}

fn parse_tool_sources(
    ctx: &YamlCtx<'_>,
    field: &str,
    v: &Yaml,
) -> Result<Vec<ToolSource>, XllmError> {
    let items = match v {
        Yaml::Null => return Ok(Vec::new()),
        Yaml::Sequence(items) => items,
        other => return Err(ctx.err(field, format!("expected a list, got {}", yaml_kind(other)))),
    };
    let mut out = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        let f = format!("{field}[{idx}]");
        let m = ctx.expect_map(&f, item)?;
        ctx.check_keys(&f, m, &["groupname", "mcp", "name"])?;
        let groupname = ctx.get_str(&f, m, "groupname")?;
        let mcp = ctx.get_str(&f, m, "mcp")?;
        let name = ctx.get_str(&f, m, "name")?;
        let count = [groupname.is_some(), mcp.is_some(), name.is_some()]
            .iter()
            .filter(|b| **b)
            .count();
        if count != 1 {
            return Err(ctx.err(&f, "exactly one of `groupname`, `mcp`, `name` must be set"));
        }
        if let Some(g) = groupname {
            out.push(ToolSource::Group {
                groupname: g.trim().to_string(),
            });
        } else if let Some(e) = mcp {
            out.push(ToolSource::Mcp {
                endpoint: e.trim().to_string(),
            });
        } else if let Some(n) = name {
            out.push(ToolSource::Named {
                name: n.trim().to_string(),
            });
        }
    }
    Ok(out)
}

fn parse_tools_config(ctx: &YamlCtx<'_>, field: &str, v: &Yaml) -> Result<ToolsConfig, XllmError> {
    // `tools: false` 是关闭工具的简写。
    if let Yaml::Bool(b) = v {
        return Ok(ToolsConfig {
            enabled: Some(*b),
            ..Default::default()
        });
    }
    let m = ctx.expect_map(field, v)?;
    ctx.check_keys(
        field,
        m,
        &[
            "enabled",
            "filesystem_policy",
            "tools2actions",
            "tools",
            "actions",
            "bash_tools",
        ],
    )?;
    let enabled = ctx.get_bool(field, m, "enabled")?;
    let filesystem_policy = match ctx.get_str(field, m, "filesystem_policy")? {
        None => None,
        Some(s) => Some(match s.trim().to_ascii_lowercase().as_str() {
            "workspace" => FilesystemPolicy::Workspace,
            "unrestricted" => FilesystemPolicy::Unrestricted,
            _ => {
                return Err(ctx.err(
                    &join_field(field, "filesystem_policy"),
                    format!(
                        "unsupported filesystem_policy `{s}` (supported: workspace, unrestricted)"
                    ),
                ))
            }
        }),
    };
    let tools2actions = ctx.get_bool(field, m, "tools2actions")?;
    let tools = match m.get("tools") {
        None => None,
        Some(v) => Some(parse_tool_sources(ctx, &join_field(field, "tools"), v)?),
    };
    let actions = match m.get("actions") {
        None => None,
        Some(v) => Some(parse_tool_sources(ctx, &join_field(field, "actions"), v)?),
    };
    let bash_tools = match m.get("bash_tools") {
        None => None,
        Some(Yaml::Null) => Some(Vec::new()),
        Some(Yaml::Sequence(items)) => {
            let f = join_field(field, "bash_tools");
            let mut out = Vec::new();
            for (idx, item) in items.iter().enumerate() {
                let ef = format!("{f}[{idx}]");
                let obj = ctx.expect_map(&ef, item)?;
                ctx.check_keys(&ef, obj, &["name", "description", "command", "usage"])?;
                let name = ctx
                    .get_str(&ef, obj, "name")?
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| ctx.err(&ef, "`name` is required"))?;
                let description = ctx
                    .get_str(&ef, obj, "description")?
                    .ok_or_else(|| ctx.err(&ef, "`description` is required"))?;
                let command = ctx
                    .get_str(&ef, obj, "command")?
                    .filter(|s| !s.trim().is_empty())
                    .ok_or_else(|| ctx.err(&ef, "`command` is required"))?;
                let usage = ctx.get_str(&ef, obj, "usage")?;
                out.push(BashToolManual {
                    name,
                    description,
                    command,
                    usage,
                });
            }
            Some(out)
        }
        Some(other) => {
            return Err(ctx.err(
                &join_field(field, "bash_tools"),
                format!("expected a list, got {}", yaml_kind(other)),
            ))
        }
    };
    Ok(ToolsConfig {
        enabled,
        filesystem_policy,
        tools2actions,
        tools,
        actions,
        bash_tools,
    })
}

fn parse_group(
    ctx: &YamlCtx<'_>,
    field: &str,
    v: &Yaml,
    source: &Path,
) -> Result<GroupDef, XllmError> {
    let m = ctx.expect_map(field, v)?;
    ctx.check_keys(field, m, &["default_user", "sections", "tools"])?;
    let default_user = ctx.get_str(field, m, "default_user")?;
    let sections = match m.get("sections") {
        None | Some(Yaml::Null) => BTreeMap::new(),
        Some(v) => parse_sections(ctx, &join_field(field, "sections"), v, source)?,
    };
    let tools = match m.get("tools") {
        None | Some(Yaml::Null) => None,
        Some(v) => Some(parse_tools_config(ctx, &join_field(field, "tools"), v)?),
    };
    Ok(GroupDef {
        default_user,
        sections,
        tools,
        sources: vec![source.display().to_string()],
    })
}

/// 展开外部组文件：根节点必须是一个组对象。`depth` 防止递归引用。
fn load_external_group(
    path: &Path,
    group_name: &str,
    referencing_file: &Path,
) -> Result<GroupDef, XllmError> {
    let raw = std::fs::read_to_string(path).map_err(|e| {
        XllmError::config(
            referencing_file,
            format!("prompt.groups.{group_name}"),
            format!("cannot read external group file {}: {e}", path.display()),
        )
    })?;
    let doc: Yaml = serde_yaml::from_str(&raw).map_err(|e| {
        XllmError::config(
            path,
            format!("(group `{group_name}`)"),
            format!("invalid YAML: {e}"),
        )
    })?;
    let ctx = YamlCtx { file: path };
    if let Yaml::String(_) = doc {
        return Err(ctx.err(
            &format!("(group `{group_name}`)"),
            "external group file must be a group object, not another file reference",
        ));
    }
    parse_group(&ctx, &format!("(group `{group_name}`)"), &doc, path)
}

fn parse_provider(ctx: &YamlCtx<'_>, field: &str, v: &Yaml) -> Result<ProviderConfig, XllmError> {
    let m = ctx.expect_map(field, v)?;
    ctx.check_keys(
        field,
        m,
        &[
            "type",
            "session_token",
            "base_url",
            "api_key",
            "api_key_env",
            "headers",
        ],
    )?;
    let kind = match ctx.get_str(field, m, "type")? {
        None => None,
        Some(s) => Some(ProviderKind::parse(&s).ok_or_else(|| {
            ctx.err(
                &join_field(field, "type"),
                format!("unsupported provider type `{s}` (supported: buckyos, openai)"),
            )
        })?),
    };
    let session_token = ctx
        .get_str(field, m, "session_token")?
        .filter(|s| !s.trim().is_empty())
        .map(|_| SecretRef::ConfigFile {
            path: ctx.file.display().to_string(),
            field: join_field(field, "session_token"),
        });
    let base_url = ctx.get_str(field, m, "base_url")?;
    let mut api_key = ctx
        .get_str(field, m, "api_key")?
        .filter(|s| !s.trim().is_empty())
        .map(|_| SecretRef::ConfigFile {
            path: ctx.file.display().to_string(),
            field: join_field(field, "api_key"),
        });
    if let Some(env_name) = ctx
        .get_str(field, m, "api_key_env")?
        .filter(|s| !s.trim().is_empty())
    {
        if api_key.is_some() {
            return Err(ctx.err(field, "`api_key` and `api_key_env` are mutually exclusive"));
        }
        api_key = Some(SecretRef::Env { name: env_name });
    }
    let mut headers = BTreeMap::new();
    if let Some(h) = m.get("headers") {
        let hf = join_field(field, "headers");
        let hm = ctx.expect_map(&hf, h)?;
        for k in hm.keys() {
            let key = k
                .as_str()
                .ok_or_else(|| ctx.err(&hf, "header names must be strings"))?;
            let val = ctx
                .get_str(&hf, hm, key)?
                .ok_or_else(|| ctx.err(&join_field(&hf, key), "header value must be a string"))?;
            headers.insert(key.to_string(), val);
        }
    }
    Ok(ProviderConfig {
        kind,
        session_token,
        base_url,
        api_key,
        headers,
    })
}

/// 展开 `~` 并把相对路径解析到 `base_dir`。
pub fn resolve_config_path(raw: &str, base_dir: &Path) -> PathBuf {
    let trimmed = raw.trim();
    if trimmed == "~" || trimmed.starts_with("~/") {
        if let Some(home) = home_dir() {
            return home.join(trimmed.trim_start_matches('~').trim_start_matches('/'));
        }
    }
    let p = Path::new(trimmed);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        normalize_path(&base_dir.join(p))
    }
}

/// 当前用户主目录。
pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
}

/// 词法归一化（去掉 `.` / `..`），不访问文件系统。
pub fn normalize_path(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 解析一份 `.llm_context` 文本。`path` 用于诊断与相对路径解析。
pub fn parse_llm_context_file(path: &Path, raw: &str) -> Result<LlmContextFile, XllmError> {
    let base_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    let doc: Yaml = serde_yaml::from_str(raw)
        .map_err(|e| XllmError::config(path, "(root)", format!("invalid YAML: {e}")))?;
    let ctx = YamlCtx { file: path };
    if matches!(doc, Yaml::Null) {
        return Ok(LlmContextFile::default());
    }
    let m = ctx.expect_map("(root)", &doc)?;
    ctx.check_keys(
        "",
        m,
        &[
            "provider",
            "model",
            "file_model",
            "max_tokens",
            "max_rounds",
            "llm_timeout",
            "timeout",
            "runs_dir",
            "loop_model",
            "run_logs",
            "result_format",
            "tools",
            "prompt",
        ],
    )?;

    let provider = match m.get("provider") {
        None | Some(Yaml::Null) => None,
        Some(v) => Some(parse_provider(&ctx, "provider", v)?),
    };
    let model = ctx
        .get_str("", m, "model")?
        .filter(|s| !s.trim().is_empty());
    let file_model = ctx
        .get_str("", m, "file_model")?
        .filter(|s| !s.trim().is_empty());
    let max_tokens = ctx.get_u32("", m, "max_tokens")?;
    let max_rounds = ctx.get_u32("", m, "max_rounds")?;
    let llm_timeout = ctx.get_u64("", m, "llm_timeout")?;
    let timeout = ctx.get_u64("", m, "timeout")?;
    let runs_dir = match m.get("runs_dir") {
        None => None,
        Some(Yaml::Null) => Some(RunsDirSetting::Disabled),
        Some(Yaml::String(s)) => {
            if s.trim().eq_ignore_ascii_case("none") {
                Some(RunsDirSetting::Disabled)
            } else if s.trim().is_empty() {
                return Err(ctx.err("runs_dir", "must be a path or `none`"));
            } else {
                Some(RunsDirSetting::Path {
                    path: resolve_config_path(s, &base_dir),
                })
            }
        }
        Some(other) => {
            return Err(ctx.err(
                "runs_dir",
                format!("expected a path string or `none`, got {}", yaml_kind(other)),
            ))
        }
    };
    let loop_model = match ctx.get_str("", m, "loop_model")? {
        None => None,
        Some(s) => Some(LoopModel::parse(&s).ok_or_else(|| {
            ctx.err(
                "loop_model",
                format!("unsupported loop_model `{s}` (supported: function_call, behavior)"),
            )
        })?),
    };
    let run_logs = match ctx.get_str("", m, "run_logs")? {
        None => None,
        Some(s) => Some(RunLogLevel::parse(&s).ok_or_else(|| {
            ctx.err(
                "run_logs",
                format!("unsupported run_logs `{s}` (supported: debug, info, warn, result)"),
            )
        })?),
    };
    let result_format = match ctx.get_str("", m, "result_format")? {
        None => None,
        Some(s) => {
            Some(ResultFormat::parse(&s).map_err(|e| ctx.err("result_format", e.to_string()))?)
        }
    };
    let tools = match m.get("tools") {
        None | Some(Yaml::Null) => None,
        Some(v) => Some(parse_tools_config(&ctx, "tools", v)?),
    };
    let prompt = match m.get("prompt") {
        None | Some(Yaml::Null) => None,
        Some(v) => {
            let pm = ctx.expect_map("prompt", v)?;
            ctx.check_keys(
                "prompt",
                pm,
                &["mode", "select", "groups", "sections", "tools", "system"],
            )?;
            let mode = match ctx.get_str("prompt", pm, "mode")? {
                None => None,
                Some(s) => Some(match s.trim().to_ascii_lowercase().as_str() {
                    "standard" => PromptMode::Standard,
                    "custom" => PromptMode::Custom,
                    other => {
                        return Err(ctx.err(
                            "prompt.mode",
                            format!("unsupported mode `{other}` (supported: standard, custom)"),
                        ))
                    }
                }),
            };
            let select = ctx
                .get_str("prompt", pm, "select")?
                .filter(|s| !s.trim().is_empty());
            let system = ctx.get_str("prompt", pm, "system")?;
            let mut groups = BTreeMap::new();
            if let Some(gv) = pm.get("groups") {
                if !matches!(gv, Yaml::Null) {
                    let gm = ctx.expect_map("prompt.groups", gv)?;
                    for (k, v) in gm {
                        let name = k
                            .as_str()
                            .ok_or_else(|| ctx.err("prompt.groups", "group names must be strings"))?
                            .to_string();
                        let gf = format!("prompt.groups.{name}");
                        let def = match v {
                            Yaml::String(path_str) => {
                                let ext = resolve_config_path(path_str, &base_dir);
                                load_external_group(&ext, &name, path)?
                            }
                            other => parse_group(&ctx, &gf, other, path)?,
                        };
                        groups.insert(name, def);
                    }
                }
            }
            let sections = match pm.get("sections") {
                None | Some(Yaml::Null) => BTreeMap::new(),
                Some(sv) => parse_sections(&ctx, "prompt.sections", sv, path)?,
            };
            let tools = match pm.get("tools") {
                None | Some(Yaml::Null) => None,
                Some(tv) => Some(parse_tools_config(&ctx, "prompt.tools", tv)?),
            };
            if mode == Some(PromptMode::Custom) && (select.is_some() || !sections.is_empty()) {
                return Err(ctx.err(
                    "prompt.mode",
                    "mode `custom` cannot be combined with `select` or `sections` in the same file",
                ));
            }
            if mode == Some(PromptMode::Custom) && system.is_none() {
                return Err(ctx.err("prompt.system", "mode `custom` requires `prompt.system`"));
            }
            Some(PromptConfig {
                mode,
                select,
                groups,
                sections,
                tools,
                system,
            })
        }
    };

    Ok(LlmContextFile {
        provider,
        model,
        file_model,
        max_tokens,
        max_rounds,
        llm_timeout,
        timeout,
        runs_dir,
        loop_model,
        run_logs,
        result_format,
        tools,
        prompt,
    })
}

// =========================================================================
// 配置查找与合并（F10）
// =========================================================================

/// 从 `workdir` 向上逐级查找 `.llm_context`，返回“最远父目录 → 工作目录”顺序。
pub fn discover_config_files(workdir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut cur: Option<&Path> = Some(workdir);
    while let Some(dir) = cur {
        let candidate = dir.join(LLM_CONTEXT_FILE_NAME);
        if candidate.is_file() {
            found.push(candidate);
        }
        cur = dir.parent();
    }
    found.reverse();
    found
}

/// 加载并解析所有层。
pub fn load_config_layers(workdir: &Path) -> Result<Vec<ConfigLayer>, XllmError> {
    let mut layers = Vec::new();
    for path in discover_config_files(workdir) {
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| XllmError::config(&path, "(file)", format!("cannot read: {e}")))?;
        let file = parse_llm_context_file(&path, &raw)?;
        layers.push(ConfigLayer { path, file });
    }
    Ok(layers)
}

/// 合并后的目录配置。每个可覆盖字段记录来源文件，供 status 展示。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MergedConfig {
    #[serde(default)]
    pub provider: ProviderConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_dir: Option<RunsDirSetting>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_model: Option<LoopModel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_logs: Option<RunLogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_format: Option<ResultFormat>,
    #[serde(default)]
    pub tools: ToolsConfig,
    #[serde(default)]
    pub prompt: PromptConfig,
    /// 字段 → 最后声明它的文件。
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
    /// 参与合并的文件（祖先 → 工作目录）。
    #[serde(default)]
    pub files: Vec<String>,
}

/// 按“祖先 → 子目录”顺序合并各层。标量覆盖、对象按字段合并、列表整体替换、
/// section 按行号合并、组按名合并。
pub fn merge_config_layers(layers: &[ConfigLayer]) -> Result<MergedConfig, XllmError> {
    let mut merged = MergedConfig::default();
    for layer in layers {
        let src = layer.path.display().to_string();
        merged.files.push(src.clone());
        let f = &layer.file;
        if let Some(p) = &f.provider {
            // Provider 类型切换时不继承另一类型的专属字段。
            if p.kind.is_some() && p.kind != merged.provider.kind && merged.provider.kind.is_some()
            {
                merged.provider = ProviderConfig::default();
            }
            if p.kind.is_some() {
                merged.provider.kind = p.kind;
                merged.sources.insert("provider.type".into(), src.clone());
            }
            if p.session_token.is_some() {
                merged.provider.session_token = p.session_token.clone();
                merged
                    .sources
                    .insert("provider.session_token".into(), src.clone());
            }
            if p.base_url.is_some() {
                merged.provider.base_url = p.base_url.clone();
                merged
                    .sources
                    .insert("provider.base_url".into(), src.clone());
            }
            if p.api_key.is_some() {
                merged.provider.api_key = p.api_key.clone();
                merged
                    .sources
                    .insert("provider.api_key".into(), src.clone());
            }
            for (k, v) in &p.headers {
                merged.provider.headers.insert(k.clone(), v.clone());
            }
        }
        macro_rules! scalar {
            ($field:ident, $name:expr) => {
                if f.$field.is_some() {
                    merged.$field = f.$field.clone();
                    merged.sources.insert($name.into(), src.clone());
                }
            };
        }
        scalar!(model, "model");
        scalar!(file_model, "file_model");
        scalar!(max_tokens, "max_tokens");
        scalar!(max_rounds, "max_rounds");
        scalar!(llm_timeout, "llm_timeout");
        scalar!(timeout, "timeout");
        scalar!(runs_dir, "runs_dir");
        scalar!(loop_model, "loop_model");
        scalar!(run_logs, "run_logs");
        scalar!(result_format, "result_format");
        if let Some(t) = &f.tools {
            merged.tools.merge_over(t);
            merged.sources.insert("tools".into(), src.clone());
            if t.filesystem_policy.is_some() {
                merged
                    .sources
                    .insert("tools.filesystem_policy".into(), src.clone());
            }
        }
        if let Some(p) = &f.prompt {
            if p.mode.is_some() {
                merged.prompt.mode = p.mode;
                merged.sources.insert("prompt.mode".into(), src.clone());
                if p.mode == Some(PromptMode::Custom) {
                    // 切换为完整自定义：不再沿用父目录的组选择与 section 覆盖。
                    merged.prompt.select = None;
                    merged.prompt.sections.clear();
                }
                if p.mode == Some(PromptMode::Standard) {
                    merged.prompt.system = None;
                }
            }
            if p.select.is_some() {
                merged.prompt.select = p.select.clone();
                merged.sources.insert("prompt.select".into(), src.clone());
                if merged.prompt.mode == Some(PromptMode::Custom) && p.mode.is_none() {
                    // 子目录只写 select：视为回到标准模式。
                    merged.prompt.mode = Some(PromptMode::Standard);
                    merged.prompt.system = None;
                }
            }
            if p.system.is_some() {
                merged.prompt.system = p.system.clone();
                merged.sources.insert("prompt.system".into(), src.clone());
            }
            for (name, def) in &p.groups {
                match merged.prompt.groups.get_mut(name) {
                    Some(existing) => existing.merge_over(def),
                    None => {
                        merged.prompt.groups.insert(name.clone(), def.clone());
                    }
                }
                merged
                    .sources
                    .insert(format!("prompt.groups.{name}"), src.clone());
            }
            for (line, def) in &p.sections {
                merged.prompt.sections.insert(*line, def.clone());
                merged
                    .sources
                    .insert(format!("prompt.sections.{line}"), src.clone());
                if merged.prompt.mode == Some(PromptMode::Custom) && p.mode.is_none() {
                    merged.prompt.mode = Some(PromptMode::Standard);
                    merged.prompt.system = None;
                }
            }
            if let Some(t) = &p.tools {
                match &mut merged.prompt.tools {
                    Some(base) => base.merge_over(t),
                    None => merged.prompt.tools = Some(t.clone()),
                }
                merged.sources.insert("prompt.tools".into(), src.clone());
                if t.filesystem_policy.is_some() {
                    merged
                        .sources
                        .insert("prompt.tools.filesystem_policy".into(), src.clone());
                }
            }
        }
    }
    Ok(merged)
}

// =========================================================================
// 显式参数与本次输入（F02 / 命令面）
// =========================================================================

/// 显式 CLI / SDK 参数。`None` 表示未传入，不覆盖文件配置（F10）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ProviderKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loop_model: Option<LoopModel>,
    /// `--tools` / `--no-tools`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<bool>,
    /// `--select <name>`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub select: Option<String>,
    /// `--system <text>`：完整自定义业务指令。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub llm_timeout_secs: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_logs: Option<RunLogLevel>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_format: Option<ResultFormat>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_dir: Option<PathBuf>,
    /// `--json`：要求按 result_format 选出的结果是合法 JSON。
    #[serde(default)]
    pub json: bool,
    /// SDK 调用方可附带 JSON schema（转发给 Provider，不做校验）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<Value>,
    /// 要求 Provider 关闭的能力（如 `web_search`），透传给模型策略。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable_capabilities: Vec<String>,
}

/// 一个显式附件（保持命令行顺序）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Attachment {
    /// `--file <path>`：文本材料。
    File { path: PathBuf },
    /// `--image <path-or-url>`。
    Image { source: String },
}

/// 本次任务的输入（F02）。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TaskInput {
    /// 位置问题或 `--user`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// 显式附件，按命令顺序。
    #[serde(default)]
    pub attachments: Vec<Attachment>,
    /// 已读取的管道文本。调用方负责“检测到管道但为空”的报错；这里也会校验。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    /// `--input-file`：结构化单次任务输入（已编译好的消息序列）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured: Option<Vec<AiMessage>>,
    /// 相对路径的基准（启动命令时的 cwd）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_dir: Option<PathBuf>,
}

impl TaskInput {
    pub fn question(text: impl Into<String>) -> Self {
        Self {
            user: Some(text.into()),
            ..Default::default()
        }
    }

    pub fn structured(messages: Vec<AiMessage>) -> Self {
        Self {
            structured: Some(messages),
            ..Default::default()
        }
    }

    pub fn with_file(mut self, path: impl Into<PathBuf>) -> Self {
        self.attachments
            .push(Attachment::File { path: path.into() });
        self
    }

    pub fn with_image(mut self, source: impl Into<String>) -> Self {
        self.attachments.push(Attachment::Image {
            source: source.into(),
        });
        self
    }

    pub fn with_stdin(mut self, text: impl Into<String>) -> Self {
        self.stdin = Some(text.into());
        self
    }
}

/// 已读取并校验的附件记录（随 Run 保存；图片正文进入消息快照，这里只留摘要）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttachmentRecord {
    pub index: usize,
    pub kind: String,
    /// 材料标签（文件名或 URL）。
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
}

/// 已加载的附件内容（内存）。
#[derive(Debug, Clone)]
pub enum LoadedAttachment {
    Text { label: String, text: String },
    Image { label: String, source: ResourceRef },
}

/// 图片 MIME 由扩展名判定（首版 PNG / JPEG / WebP）。
pub fn image_mime_from_path(path: &Path) -> Option<&'static str> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

fn is_url(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    l.starts_with("http://") || l.starts_with("https://")
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// 读取所有显式附件，保留顺序；任一附件不可用即报错，不会丢弃附件继续。
pub fn load_attachments(
    attachments: &[Attachment],
    base_dir: &Path,
) -> Result<(Vec<LoadedAttachment>, Vec<AttachmentRecord>), XllmError> {
    let mut loaded = Vec::new();
    let mut records = Vec::new();
    for (index, att) in attachments.iter().enumerate() {
        match att {
            Attachment::File { path } => {
                let abs = if path.is_absolute() {
                    path.clone()
                } else {
                    normalize_path(&base_dir.join(path))
                };
                let meta = std::fs::metadata(&abs).map_err(|e| {
                    XllmError::Input(format!(
                        "--file {}: cannot read (resolved to {}): {e}",
                        path.display(),
                        abs.display()
                    ))
                })?;
                if !meta.is_file() {
                    return Err(XllmError::Input(format!(
                        "--file {}: not a regular file (resolved to {})",
                        path.display(),
                        abs.display()
                    )));
                }
                if meta.len() as usize > MAX_TEXT_ATTACHMENT_BYTES {
                    return Err(XllmError::Input(format!(
                        "--file {}: {} bytes exceeds the {} byte limit; split the material or trim it",
                        path.display(),
                        meta.len(),
                        MAX_TEXT_ATTACHMENT_BYTES
                    )));
                }
                let bytes = std::fs::read(&abs)?;
                let text = String::from_utf8(bytes.clone()).map_err(|_| {
                    XllmError::Input(format!(
                        "--file {}: not valid UTF-8 text; first version only accepts text files (use --image for pictures)",
                        path.display()
                    ))
                })?;
                let label = abs
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| path.display().to_string());
                records.push(AttachmentRecord {
                    index,
                    kind: "file".into(),
                    label: label.clone(),
                    path: Some(abs.display().to_string()),
                    mime: Some("text/plain".into()),
                    bytes: Some(meta.len()),
                    sha256: Some(sha256_hex(&bytes)),
                });
                loaded.push(LoadedAttachment::Text { label, text });
            }
            Attachment::Image { source } => {
                if is_url(source) {
                    records.push(AttachmentRecord {
                        index,
                        kind: "image".into(),
                        label: source.clone(),
                        path: None,
                        mime: None,
                        bytes: None,
                        sha256: None,
                    });
                    loaded.push(LoadedAttachment::Image {
                        label: source.clone(),
                        source: ResourceRef::url(source.clone(), None),
                    });
                    continue;
                }
                let p = Path::new(source);
                let abs = if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    normalize_path(&base_dir.join(p))
                };
                let mime = image_mime_from_path(&abs).ok_or_else(|| {
                    XllmError::Input(format!(
                        "--image {source}: unsupported image format (first version supports PNG, JPEG, WebP)"
                    ))
                })?;
                let meta = std::fs::metadata(&abs).map_err(|e| {
                    XllmError::Input(format!(
                        "--image {source}: cannot read (resolved to {}): {e}",
                        abs.display()
                    ))
                })?;
                if meta.len() as usize > MAX_IMAGE_ATTACHMENT_BYTES {
                    return Err(XllmError::Input(format!(
                        "--image {source}: {} bytes exceeds the {} byte limit; shrink the image",
                        meta.len(),
                        MAX_IMAGE_ATTACHMENT_BYTES
                    )));
                }
                let bytes = std::fs::read(&abs)?;
                let label = abs
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| source.clone());
                records.push(AttachmentRecord {
                    index,
                    kind: "image".into(),
                    label: label.clone(),
                    path: Some(abs.display().to_string()),
                    mime: Some(mime.to_string()),
                    bytes: Some(meta.len()),
                    sha256: Some(sha256_hex(&bytes)),
                });
                loaded.push(LoadedAttachment::Image {
                    label,
                    source: ResourceRef::base64(
                        mime.to_string(),
                        general_purpose::STANDARD.encode(&bytes),
                    ),
                });
            }
        }
    }
    Ok((loaded, records))
}

// =========================================================================
// 有效配置（合并 + 选组 + CLI）
// =========================================================================

/// 执行限制（F08）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u32>,
    pub max_rounds: u32,
    pub timeout_secs: u64,
    pub llm_timeout_secs: u64,
}

/// 展开后的单个工具。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedTool {
    pub name: String,
    pub description: String,
    pub args_schema: Value,
    /// 来源描述（`groupname:bash` / `mcp:<url>` / `name:<x>`）。
    pub source: String,
}

/// 最终工具配置（F04），随 Run 保存。
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct EffectiveTools {
    pub enabled: bool,
    #[serde(default)]
    pub filesystem_policy: FilesystemPolicy,
    pub tools2actions: bool,
    /// 声明的来源（展开前）。
    #[serde(default)]
    pub tool_sources: Vec<ToolSource>,
    #[serde(default)]
    pub action_sources: Vec<ToolSource>,
    /// 展开后的原生 tools。
    #[serde(default)]
    pub native: Vec<ResolvedTool>,
    /// 展开后的 behavior actions。
    #[serde(default)]
    pub actions: Vec<ResolvedTool>,
    #[serde(default)]
    pub bash_tools: Vec<BashToolManual>,
    /// exec 是否属于本次实际可用能力（tools 或 actions 中任一）。
    pub exec_enabled: bool,
    /// 各字段的决定来源。
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
}

impl EffectiveTools {
    pub fn all_names(&self) -> Vec<String> {
        self.native
            .iter()
            .chain(self.actions.iter())
            .map(|t| t.name.clone())
            .collect()
    }
}

/// 本次 Run 的有效执行配置（随 Run 保存，resume 时不重新计算）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectiveConfig {
    pub provider: ProviderConfig,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model: Option<String>,
    pub loop_model: LoopModel,
    pub limits: RunLimits,
    pub run_logs: RunLogLevel,
    pub result_format: ResultFormat,
    pub json: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disable_capabilities: Vec<String>,
    pub tools: EffectiveTools,
    /// 参与合并的配置文件。
    #[serde(default)]
    pub config_files: Vec<String>,
    /// 字段 → 来源（文件路径 / `cli` / `default`）。
    #[serde(default)]
    pub sources: BTreeMap<String, String>,
}

/// 计算工具配置（不展开来源）：产品默认 → 顶层 tools → 选中组 tools →
/// prompt.tools → CLI 开关。
pub fn compute_tools_config(
    merged: &MergedConfig,
    group: Option<&GroupDef>,
    cli_tools: Option<bool>,
    include_group: bool,
) -> (ToolsConfig, BTreeMap<String, String>) {
    let mut cfg = ToolsConfig::default();
    let mut sources = BTreeMap::new();
    let top_src = merged
        .sources
        .get("tools")
        .cloned()
        .unwrap_or_else(|| "default".into());
    cfg.merge_over(&merged.tools);
    for key in ["enabled", "tools2actions", "tools", "actions", "bash_tools"] {
        sources.insert(key.to_string(), top_src.clone());
    }
    sources.insert(
        "filesystem_policy".into(),
        merged
            .sources
            .get("tools.filesystem_policy")
            .cloned()
            .unwrap_or_else(|| "default".into()),
    );
    if include_group {
        if let Some(gt) = group.and_then(|g| g.tools.as_ref()) {
            cfg.merge_over(gt);
            let gsrc = "selected group".to_string();
            if gt.enabled.is_some() {
                sources.insert("enabled".into(), gsrc.clone());
            }
            if gt.filesystem_policy.is_some() {
                sources.insert("filesystem_policy".into(), gsrc.clone());
            }
            if gt.tools2actions.is_some() {
                sources.insert("tools2actions".into(), gsrc.clone());
            }
            if gt.tools.is_some() {
                sources.insert("tools".into(), gsrc.clone());
            }
            if gt.actions.is_some() {
                sources.insert("actions".into(), gsrc.clone());
            }
            if gt.bash_tools.is_some() {
                sources.insert("bash_tools".into(), gsrc);
            }
        }
    }
    if let Some(pt) = &merged.prompt.tools {
        cfg.merge_over(pt);
        let psrc = merged
            .sources
            .get("prompt.tools")
            .cloned()
            .unwrap_or_else(|| "prompt.tools".into());
        if pt.enabled.is_some() {
            sources.insert("enabled".into(), psrc.clone());
        }
        if pt.filesystem_policy.is_some() {
            sources.insert(
                "filesystem_policy".into(),
                merged
                    .sources
                    .get("prompt.tools.filesystem_policy")
                    .cloned()
                    .unwrap_or_else(|| psrc.clone()),
            );
        }
        if pt.tools2actions.is_some() {
            sources.insert("tools2actions".into(), psrc.clone());
        }
        if pt.tools.is_some() {
            sources.insert("tools".into(), psrc.clone());
        }
        if pt.actions.is_some() {
            sources.insert("actions".into(), psrc.clone());
        }
        if pt.bash_tools.is_some() {
            sources.insert("bash_tools".into(), psrc);
        }
    }
    if let Some(b) = cli_tools {
        cfg.enabled = Some(b);
        sources.insert("enabled".into(), "cli".into());
    }
    (cfg, sources)
}

// =========================================================================
// 模板变量（F11.4）
// =========================================================================

/// 模板变量来源：内置 `runtime.*` 由系统提供；`env.*` 只按显式引用读取。
#[derive(Debug, Clone, Default)]
pub struct TemplateEnv {
    pub runtime: BTreeMap<String, String>,
    used: Arc<Mutex<BTreeMap<String, String>>>,
}

impl TemplateEnv {
    /// 为一次新 Run 生成本次时间、时区、操作系统、工作目录。
    pub fn for_new_run(workdir: &Path) -> Self {
        let now = chrono::Local::now();
        let mut runtime = BTreeMap::new();
        runtime.insert(
            "current_time".into(),
            now.format("%Y-%m-%d %H:%M:%S %:z").to_string(),
        );
        let tz = std::env::var("TZ")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| now.format("UTC%:z").to_string());
        runtime.insert("timezone".into(), tz);
        runtime.insert(
            "os".into(),
            format!("{} ({})", std::env::consts::OS, std::env::consts::ARCH),
        );
        runtime.insert("cwd".into(), workdir.display().to_string());
        Self {
            runtime,
            used: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// 已被模板引用过的变量及其值（随 Run 保存）。
    pub fn used_vars(&self) -> BTreeMap<String, String> {
        self.used.lock().expect("template env lock").clone()
    }

    fn lookup(&self, name: &str, source: &str) -> Result<String, XllmError> {
        let value = if let Some(key) = name.strip_prefix("runtime.") {
            self.runtime
                .get(key)
                .cloned()
                .ok_or_else(|| XllmError::Template {
                    origin: source.to_string(),
                    reason: format!(
                        "unknown runtime variable `{name}` (available: {})",
                        self.runtime
                            .keys()
                            .map(|k| format!("runtime.{k}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                })?
        } else if let Some(key) = name.strip_prefix("env.") {
            std::env::var(key).map_err(|_| XllmError::Template {
                origin: source.to_string(),
                reason: format!("environment variable `{key}` is not set (referenced as `{name}`)"),
            })?
        } else {
            return Err(XllmError::Template {
                origin: source.to_string(),
                reason: format!("unknown template variable `{name}` (use `runtime.*` or `env.*`)"),
            });
        };
        self.used
            .lock()
            .expect("template env lock")
            .insert(name.to_string(), value.clone());
        Ok(value)
    }

    /// 渲染 `{{ name }}`。`\{{` 输出字面 `{{`。替换值不递归解释。
    pub fn render(&self, text: &str, source: &str) -> Result<String, XllmError> {
        let mut out = String::with_capacity(text.len());
        let mut rest = text;
        loop {
            let Some(pos) = rest.find("{{") else {
                out.push_str(rest);
                break;
            };
            // 转义：`\{{`
            if pos > 0 && rest.as_bytes()[pos - 1] == b'\\' {
                out.push_str(&rest[..pos - 1]);
                out.push_str("{{");
                rest = &rest[pos + 2..];
                continue;
            }
            out.push_str(&rest[..pos]);
            let after = &rest[pos + 2..];
            let Some(end) = after.find("}}") else {
                return Err(XllmError::Template {
                    origin: source.to_string(),
                    reason: "unterminated `{{` placeholder".to_string(),
                });
            };
            let name = after[..end].trim();
            if name.is_empty()
                || !name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
            {
                return Err(XllmError::Template {
                    origin: source.to_string(),
                    reason: format!("invalid placeholder `{{{{{name}}}}}`"),
                });
            }
            out.push_str(&self.lookup(name, source)?);
            rest = &after[end + 2..];
        }
        Ok(out)
    }
}

// =========================================================================
// 提示词组装（F11）
// =========================================================================

/// 一个 section 的最终形态。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionRender {
    pub line: u32,
    pub name: String,
    /// 用户可编辑文本（模板渲染后）。`None` = 未声明或被清空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_text: Option<String>,
    /// 用户文本来源（文件路径 / 组名）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_source: Option<String>,
    /// 系统依据生效配置生成的说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_text: Option<String>,
}

impl SectionRender {
    pub fn is_empty(&self) -> bool {
        self.user_text
            .as_deref()
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
            && self
                .system_text
                .as_deref()
                .map(|s| s.trim().is_empty())
                .unwrap_or(true)
    }
}

/// 组装后的提示词计划（随 Run 保存，resume 直接使用）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PromptPlan {
    pub mode: PromptMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    pub loop_model: LoopModel,
    #[serde(default)]
    pub sections: Vec<SectionRender>,
    /// 完整自定义业务提示词（渲染后）及来源。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_system_source: Option<String>,
    pub runtime_protocol: String,
    pub protocol_version: String,
    /// 最终 system 提示词。
    pub system_prompt: String,
    /// 本次任务要求（渲染后）；`None` 表示由 stdin 或结构化输入提供。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_request: Option<String>,
    /// `explicit` / `group_default` / `stdin` / `structured`。
    pub user_request_source: String,
    /// 已使用的模板变量与值。
    #[serde(default)]
    pub template_vars: BTreeMap<String, String>,
    /// 本次 runtime 变量全集。
    #[serde(default)]
    pub runtime_vars: BTreeMap<String, String>,
}

/// 渲染单个 action 的调用形式（behavior 协议说明）。
fn render_action_usage(tool: &ResolvedTool) -> String {
    let props = tool
        .args_schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let required: Vec<String> = tool
        .args_schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let body_arg = action_body_arg(&tool.name, &tool.args_schema);
    let mut attrs = Vec::new();
    let mut child_tags = Vec::new();
    for (k, v) in &props {
        if Some(k.as_str()) == body_arg.as_deref() {
            continue;
        }
        let ty = v.get("type").and_then(Value::as_str).unwrap_or("string");
        let opt = if required.contains(k) { "" } else { "?" };
        if matches!(ty, "string" | "integer" | "number" | "boolean") {
            attrs.push(format!("{k}{opt}=\"<{ty}>\""));
        } else {
            child_tags.push(format!("<{k}{opt}><![CDATA[<json {ty}>]]></{k}>"));
        }
    }
    let attr_str = if attrs.is_empty() {
        String::new()
    } else {
        format!(" {}", attrs.join(" "))
    };
    let form = if let Some(b) = &body_arg {
        if child_tags.is_empty() {
            format!(
                "<{n}{a}><![CDATA[<{b} value>]]></{n}>",
                n = tool.name,
                a = attr_str,
                b = b
            )
        } else {
            format!(
                "<{n}{a}>{c}<{b}><![CDATA[...]]></{b}></{n}>",
                n = tool.name,
                a = attr_str,
                c = child_tags.join(""),
                b = b
            )
        }
    } else if child_tags.is_empty() {
        format!("<{n}{a}/>", n = tool.name, a = attr_str)
    } else {
        format!(
            "<{n}{a}>{c}</{n}>",
            n = tool.name,
            a = attr_str,
            c = child_tags.join("")
        )
    };
    let mut lines = vec![format!("- {form}")];
    if let Some(b) = &body_arg {
        if child_tags.is_empty() {
            lines.push(format!(
                "  Body/CDATA contains only the raw value of `{b}`; do not include the field name or a `{b}:` prefix."
            ));
        }
    }
    let desc = tool.description.trim();
    if !desc.is_empty() {
        lines.push(format!("  {desc}"));
    }
    for (k, v) in &props {
        if let Some(d) = v.get("description").and_then(Value::as_str) {
            lines.push(format!("  {k}: {d}"));
        }
    }
    lines.join("\n")
}

/// 决定哪个参数由 XML 元素正文承载：优先 `command` / `content` / `text`，
/// 否则取唯一的必填 string 参数。
fn action_body_arg(name: &str, schema: &Value) -> Option<String> {
    let props = schema.get("properties").and_then(Value::as_object)?;
    for cand in ["command", "content", "text", "body", "query", "message"] {
        if props.contains_key(cand) {
            return Some(cand.to_string());
        }
    }
    let _ = name;
    let required: Vec<&str> = schema
        .get("required")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let string_required: Vec<&str> = required
        .iter()
        .copied()
        .filter(|k| {
            props
                .get(*k)
                .and_then(|v| v.get("type"))
                .and_then(Value::as_str)
                == Some("string")
        })
        .collect();
    if string_required.len() == 1 {
        return Some(string_required[0].to_string());
    }
    None
}

/// 生成运行时协议（F11.2）。由 loop_model、实际可用能力与输出约束决定。
pub fn build_runtime_protocol(loop_model: LoopModel, tools: &EffectiveTools, json: bool) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "You are running inside xllm ({RUNTIME_PROTOCOL_VERSION}), a one-shot task runner: complete the task given in the user message and deliver one final result. There is no follow-up conversation, so do not ask the user questions; if something essential is missing, state it in the final result. Use only the material provided and the results you obtain during this run; distinguish verified facts from assumptions.\n"
    ));
    match loop_model {
        LoopModel::FunctionCall => {
            if tools.enabled && !tools.native.is_empty() {
                s.push_str("\nExecution loop (native function calling):\n");
                s.push_str("- When you need to act, call one or more of the provided tools; their results are returned to you and you decide the next step. Only the tools declared for this run exist.\n");
                s.push_str("- A failed tool call is not the end of the task: read the error, adjust, and continue within the round limit.\n");
                s.push_str("- When the task is complete, reply with the final result as plain assistant text and no tool calls. That text is delivered verbatim to the caller.\n");
            } else {
                s.push_str("\nExecution loop: no tools or actions are available in this run. Answer directly in a single reply; do not request or describe tool calls. Your reply is delivered verbatim to the caller.\n");
            }
            if json {
                s.push_str("\nOutput constraint: the final reply MUST be exactly one valid JSON value with no surrounding prose or code fences.\n");
            }
        }
        LoopModel::Behavior => {
            let has_actions = tools.enabled && !tools.actions.is_empty();
            let has_native = tools.enabled && !tools.native.is_empty();
            s.push_str("\nExecution loop (behavior protocol): every reply MUST be one XML document with this shape:\n");
            s.push_str("<response>\n  <observation>what the previous action results tell you (omit on the first step)</observation>\n  <thinking>brief reasoning about what to do next</thinking>\n");
            if has_actions {
                s.push_str("  <actions>\n    ...zero or more actions, executed in order...\n  </actions>\n");
            }
            s.push_str("  <report><![CDATA[the final result, only when the task is complete]]></report>\n</response>\n");
            s.push_str("Rules:\n");
            if has_actions {
                s.push_str("- To keep working, put one or more actions inside <actions> and do NOT include <report>; the results come back in the next turn.\n");
                s.push_str("- To finish, reply with no <actions> and put the complete final result inside <report>. The <report> content is delivered verbatim to the caller.\n");
                s.push_str("- Actions run in order; the first failed action stops the rest of that step. Read the error, adjust, and continue within the round limit.\n");
                s.push_str("- Only the actions listed below exist; do not invent others. Put multi-line or special-character values inside <![CDATA[ ... ]]>.\n");
                s.push_str("Available actions:\n");
                for t in &tools.actions {
                    s.push_str(&render_action_usage(t));
                    s.push('\n');
                }
            } else {
                s.push_str("- No actions are available in this run: reply with a single <response> whose <report> contains the complete final result. Do not include <actions>.\n");
            }
            if has_native {
                s.push_str("- Native function-call tools are also declared for this run; a reply that uses them is treated as a step with actions.\n");
            }
            if json {
                s.push_str("- Output constraint: the content of <report> MUST be exactly one valid JSON value.\n");
            }
        }
    }
    s
}

/// 依据生效工具生成 `rules`（30）的系统说明。
fn build_rules_system_text(loop_model: LoopModel, tools: &EffectiveTools) -> String {
    if !tools.enabled || (tools.native.is_empty() && tools.actions.is_empty()) {
        return "Capabilities for this run: no tools or actions can be called. Work only with the material submitted in this task.".to_string();
    }
    let mut lines = Vec::new();
    if !tools.native.is_empty() {
        lines.push(format!(
            "Native tools available in this run ({}): {}",
            loop_model.as_str(),
            tools
                .native
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !tools.actions.is_empty() {
        lines.push(format!(
            "Behavior actions available in this run: {}",
            tools
                .actions
                .iter()
                .map(|t| t.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if tools.tools2actions {
        lines.push("Configured tools were converted into actions; call them through the XML action protocol, not through native function calls.".to_string());
    }
    if tools
        .native
        .iter()
        .chain(&tools.actions)
        .any(|tool| tool.source == "groupname:bash")
    {
        lines.push(match tools.filesystem_policy {
            FilesystemPolicy::Workspace => "Builtin file tools and exec's cwd are restricted to the working directory. Shell commands are not sandboxed and run with the current user's permissions.",
            FilesystemPolicy::Unrestricted => "Builtin file tools and exec's cwd may access paths outside the working directory, subject to the current user's operating-system permissions. Relative paths default to the working directory.",
        }.to_string());
    }
    lines.push("Do not claim a tool or command exists unless it is listed here.".to_string());
    lines.join("\n")
}

/// 依据 exec 是否启用与 bash_tools 生成 `cmd_manual`（40）的系统说明。
fn build_cmd_manual_system_text(tools: &EffectiveTools) -> Option<String> {
    if !tools.enabled || !tools.exec_enabled {
        return None;
    }
    let mut s = String::from(
        "Commands run through `exec` in the working directory with the PATH inherited from the invoking shell. Documented commands:\n",
    );
    if tools.bash_tools.is_empty() {
        s.push_str("(no additional commands documented; standard commands on PATH may be used)\n");
    }
    for m in &tools.bash_tools {
        s.push_str(&format!(
            "- {}: {} Command: `{}`.{}\n",
            m.name,
            m.description.trim(),
            m.command,
            m.usage
                .as_deref()
                .map(|u| format!(" Usage: `{u}`."))
                .unwrap_or_default()
        ));
    }
    Some(s.trim_end().to_string())
}

fn build_contexts_system_text(
    env: &TemplateEnv,
    referenced: &BTreeMap<String, String>,
) -> Option<String> {
    let mut lines = Vec::new();
    for (key, label) in [
        ("current_time", "Current time"),
        ("timezone", "Timezone"),
        ("os", "Operating system"),
        ("cwd", "Working directory"),
    ] {
        if referenced.contains_key(&format!("runtime.{key}")) {
            continue;
        }
        if let Some(v) = env.runtime.get(key) {
            lines.push(format!("{label}: {v}"));
        }
    }
    if lines.is_empty() {
        None
    } else {
        Some(lines.join("\n"))
    }
}

/// 组装参数。
pub struct PromptAssembly<'a> {
    pub merged: &'a MergedConfig,
    pub overrides: &'a TaskOverrides,
    pub group_name: Option<&'a str>,
    pub group: Option<&'a GroupDef>,
    pub loop_model: LoopModel,
    pub tools: &'a EffectiveTools,
    pub env: &'a TemplateEnv,
    pub json: bool,
}

/// 按 F11.2 的步骤组装 system 提示词与组默认任务。
pub fn assemble_prompt(a: &PromptAssembly<'_>) -> Result<PromptPlan, XllmError> {
    let runtime_protocol = build_runtime_protocol(a.loop_model, a.tools, a.json);
    let mode = if a.overrides.system.is_some() {
        PromptMode::Custom
    } else if a.overrides.select.is_some() {
        PromptMode::Standard
    } else {
        a.merged.prompt.mode.unwrap_or(PromptMode::Standard)
    };

    let mut sections: Vec<SectionRender> = Vec::new();
    let mut custom_system = None;
    let mut custom_system_source = None;

    if mode == PromptMode::Custom {
        let (text, source) = if let Some(s) = &a.overrides.system {
            (s.clone(), "cli:--system".to_string())
        } else {
            let src = a
                .merged
                .sources
                .get("prompt.system")
                .cloned()
                .unwrap_or_else(|| "prompt.system".into());
            let text = a
                .merged
                .prompt
                .system
                .clone()
                .ok_or_else(|| XllmError::Config {
                    file: src.clone(),
                    field: "prompt.system".into(),
                    reason: "mode `custom` requires `prompt.system`".into(),
                })?;
            (text, src)
        };
        let rendered = a.env.render(&text, &source)?;
        custom_system = Some(rendered);
        custom_system_source = Some(source);
    } else {
        // 1) 系统默认（无用户文本）→ 组 → 目录级覆盖。
        let mut user_texts: BTreeMap<u32, (Option<String>, Option<String>, String)> =
            BTreeMap::new(); // line -> (name, text, source)
        if let Some(g) = a.group {
            for (line, def) in &g.sections {
                user_texts.insert(
                    *line,
                    (
                        def.name.clone(),
                        def.text.clone(),
                        format!("group:{}", a.group_name.unwrap_or("?")),
                    ),
                );
            }
        }
        for (line, def) in &a.merged.prompt.sections {
            let entry = user_texts
                .entry(*line)
                .or_insert((None, None, def.source.clone()));
            if def.name.is_some() {
                entry.0 = def.name.clone();
            }
            if def.text.is_some() {
                entry.1 = def.text.clone();
                entry.2 = def.source.clone();
            }
        }
        // 2) 固定行号始终存在（系统说明可能填充）。
        for line in [10u32, 20, 30, 40, 100] {
            user_texts
                .entry(line)
                .or_insert((None, None, "default".into()));
        }
        // 3) 渲染模板。
        for (line, (name, text, source)) in user_texts {
            let rendered = match text {
                Some(t) if !t.trim().is_empty() => {
                    Some(a.env.render(&t, &format!("{source} (section {line})"))?)
                }
                _ => None,
            };
            sections.push(SectionRender {
                line,
                name: name.unwrap_or_else(|| section_default_name(line)),
                user_text: rendered,
                user_source: Some(source),
                system_text: None,
            });
        }
    }

    // 组默认任务要求也支持模板。
    let group_default_user = match a.group.and_then(|g| g.default_user.as_ref()) {
        Some(t) if !t.trim().is_empty() => Some(a.env.render(
            t,
            &format!("group:{}:default_user", a.group_name.unwrap_or("?")),
        )?),
        _ => None,
    };

    // 4) 系统说明填入对应行号（在用户文本合并与渲染之后）。
    let referenced = a.env.used_vars();
    if mode == PromptMode::Standard {
        for sec in sections.iter_mut() {
            sec.system_text = match sec.line {
                20 => build_contexts_system_text(a.env, &referenced),
                30 => Some(build_rules_system_text(a.loop_model, a.tools)),
                40 => build_cmd_manual_system_text(a.tools),
                _ => None,
            };
            if sec.line == 40 && !a.tools.exec_enabled {
                // exec 未启用：不提供命令手册，也不把用户写的命令描述为可执行能力。
                sec.user_text = None;
            }
        }
    }

    // 5) 拼接 system。
    let mut system_prompt = String::new();
    match mode {
        PromptMode::Custom => {
            if let Some(cs) = &custom_system {
                system_prompt.push_str(cs.trim_end());
                system_prompt.push_str("\n\n");
            }
            // 自定义模式仍需说明实际能力。
            system_prompt.push_str("## capabilities\n");
            system_prompt.push_str(&build_rules_system_text(a.loop_model, a.tools));
            if let Some(m) = build_cmd_manual_system_text(a.tools) {
                system_prompt.push_str("\n\n");
                system_prompt.push_str(&m);
            }
            system_prompt.push_str("\n\n");
        }
        PromptMode::Standard => {
            for sec in &sections {
                if sec.is_empty() {
                    continue;
                }
                system_prompt.push_str(&format!("## {}\n", sec.name));
                if let Some(u) = &sec.user_text {
                    system_prompt.push_str(u.trim_end());
                    system_prompt.push('\n');
                }
                if let Some(s) = &sec.system_text {
                    system_prompt.push_str(s.trim_end());
                    system_prompt.push('\n');
                }
                system_prompt.push('\n');
            }
        }
    }
    system_prompt.push_str("## runtime_protocol\n");
    system_prompt.push_str(runtime_protocol.trim_end());
    system_prompt.push('\n');

    Ok(PromptPlan {
        mode,
        group: a.group_name.map(str::to_string),
        loop_model: a.loop_model,
        sections,
        custom_system,
        custom_system_source,
        runtime_protocol,
        protocol_version: RUNTIME_PROTOCOL_VERSION.to_string(),
        system_prompt,
        user_request: group_default_user,
        user_request_source: "group_default".into(),
        template_vars: a.env.used_vars(),
        runtime_vars: a.env.runtime.clone(),
    })
}

// =========================================================================
// XML 小工具（behavior 动作解析与 result.<path> 提取共用）
// =========================================================================

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XmlElement {
    pub name: String,
    pub attrs: Vec<(String, String)>,
    /// 原始正文（未做 CDATA / 实体处理）。
    pub body: String,
    pub self_closing: bool,
}

impl XmlElement {
    pub fn attr(&self, key: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// 正文文本：CDATA 取原文，否则反转义。
    pub fn text(&self) -> String {
        element_text(&self.body)
    }

    /// 直接子元素。
    pub fn children(&self) -> Vec<XmlElement> {
        scan_xml_elements(&self.body)
    }
}

pub fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// CDATA-aware 正文文本。
pub fn element_text(body: &str) -> String {
    let t = body.trim();
    if let Some(inner) = t.strip_prefix("<![CDATA[") {
        if let Some(inner) = inner.strip_suffix("]]>") {
            return inner.to_string();
        }
    }
    // 多段 CDATA 混排：逐段处理。
    if t.contains("<![CDATA[") {
        let mut out = String::new();
        let mut rest = t;
        while let Some(pos) = rest.find("<![CDATA[") {
            out.push_str(&xml_unescape(&rest[..pos]));
            let after = &rest[pos + 9..];
            match after.find("]]>") {
                Some(end) => {
                    out.push_str(&after[..end]);
                    rest = &after[end + 3..];
                }
                None => {
                    out.push_str(after);
                    rest = "";
                }
            }
        }
        out.push_str(&xml_unescape(rest));
        return out;
    }
    xml_unescape(t)
}

fn is_name_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' || c == ':'
}

/// 扫描 `input` 的直接子元素（跳过 CDATA / 注释 / 处理指令；容忍缺失闭合标签）。
pub fn scan_xml_elements(input: &str) -> Vec<XmlElement> {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let Some(rel) = input[i..].find('<') else {
            break;
        };
        let start = i + rel;
        let rest = &input[start..];
        if rest.starts_with("<![CDATA[") {
            i = match rest.find("]]>") {
                Some(e) => start + e + 3,
                None => bytes.len(),
            };
            continue;
        }
        if rest.starts_with("<!--") {
            i = match rest.find("-->") {
                Some(e) => start + e + 3,
                None => bytes.len(),
            };
            continue;
        }
        if rest.starts_with("<?") || rest.starts_with("<!") || rest.starts_with("</") {
            i = match rest.find('>') {
                Some(e) => start + e + 1,
                None => bytes.len(),
            };
            continue;
        }
        // 标签名
        let mut chars = rest[1..].char_indices();
        let Some((_, c0)) = chars.next() else { break };
        if !is_name_start(c0) {
            i = start + 1;
            continue;
        }
        let mut name_end = 1 + c0.len_utf8();
        for (idx, c) in chars {
            if is_name_char(c) {
                name_end = 1 + idx + c.len_utf8();
            } else {
                break;
            }
        }
        let name = rest[1..name_end].to_string();
        // 属性
        let mut attrs = Vec::new();
        let mut j = name_end;
        let tag_rest = rest.as_bytes();
        let mut self_closing = false;
        let mut closed = false;
        while j < tag_rest.len() {
            let c = tag_rest[j] as char;
            if c.is_whitespace() {
                j += 1;
                continue;
            }
            if c == '/' && tag_rest.get(j + 1) == Some(&b'>') {
                self_closing = true;
                j += 2;
                closed = true;
                break;
            }
            if c == '>' {
                j += 1;
                closed = true;
                break;
            }
            // attr name
            let an_start = j;
            while j < tag_rest.len() {
                let ch = tag_rest[j] as char;
                if ch.is_whitespace() || ch == '=' || ch == '>' || ch == '/' {
                    break;
                }
                j += 1;
            }
            let an = rest[an_start..j].to_string();
            if an.is_empty() {
                j += 1;
                continue;
            }
            while j < tag_rest.len() && (tag_rest[j] as char).is_whitespace() {
                j += 1;
            }
            if tag_rest.get(j) == Some(&b'=') {
                j += 1;
                while j < tag_rest.len() && (tag_rest[j] as char).is_whitespace() {
                    j += 1;
                }
                let quote = tag_rest.get(j).copied();
                if quote == Some(b'"') || quote == Some(b'\'') {
                    let q = quote.unwrap() as char;
                    let v_start = j + 1;
                    let v_end = rest[v_start..]
                        .find(q)
                        .map(|e| v_start + e)
                        .unwrap_or(tag_rest.len());
                    attrs.push((an, xml_unescape(&rest[v_start..v_end])));
                    j = (v_end + 1).min(tag_rest.len());
                } else {
                    let v_start = j;
                    while j < tag_rest.len() {
                        let ch = tag_rest[j] as char;
                        if ch.is_whitespace() || ch == '>' || ch == '/' {
                            break;
                        }
                        j += 1;
                    }
                    attrs.push((an, xml_unescape(&rest[v_start..j])));
                }
            } else {
                attrs.push((an, String::new()));
            }
        }
        if !closed {
            break;
        }
        let body_start = start + j;
        if self_closing {
            out.push(XmlElement {
                name,
                attrs,
                body: String::new(),
                self_closing: true,
            });
            i = body_start;
            continue;
        }
        // 找匹配的闭合标签（同名嵌套计数，跳过 CDATA）。
        let mut depth = 1usize;
        let mut k = body_start;
        let mut body_end = input.len();
        let mut after_close = input.len();
        while k < input.len() {
            let Some(rel) = input[k..].find('<') else {
                break;
            };
            let p = k + rel;
            let r = &input[p..];
            if r.starts_with("<![CDATA[") {
                k = match r.find("]]>") {
                    Some(e) => p + e + 3,
                    None => input.len(),
                };
                continue;
            }
            if r.starts_with("<!--") {
                k = match r.find("-->") {
                    Some(e) => p + e + 3,
                    None => input.len(),
                };
                continue;
            }
            let close_prefix = format!("</{name}");
            let open_prefix = format!("<{name}");
            if r.starts_with(&close_prefix)
                && r[close_prefix.len()..]
                    .chars()
                    .next()
                    .map(|c| c == '>' || c.is_whitespace())
                    .unwrap_or(false)
            {
                depth -= 1;
                let gt = r.find('>').map(|e| p + e + 1).unwrap_or(input.len());
                if depth == 0 {
                    body_end = p;
                    after_close = gt;
                    break;
                }
                k = gt;
                continue;
            }
            if r.starts_with(&open_prefix)
                && r[open_prefix.len()..]
                    .chars()
                    .next()
                    .map(|c| c == '>' || c == '/' || c.is_whitespace())
                    .unwrap_or(false)
            {
                // 自闭合的同名标签不增加深度。
                let gt = r.find('>').map(|e| p + e + 1).unwrap_or(input.len());
                let is_self = input[p..gt].trim_end_matches('>').ends_with('/');
                if !is_self {
                    depth += 1;
                }
                k = gt;
                continue;
            }
            k = p + 1;
        }
        out.push(XmlElement {
            name,
            attrs,
            body: input[body_start..body_end].to_string(),
            self_closing: false,
        });
        i = after_close;
    }
    out
}

/// 去掉 ```xml / ```json / ``` 围栏。
pub fn strip_code_fences(text: &str) -> String {
    let t = text.trim();
    if let Some(rest) = t.strip_prefix("```") {
        let rest = match rest.find('\n') {
            Some(nl) => &rest[nl + 1..],
            None => rest,
        };
        let rest = rest.trim_end();
        let rest = rest.strip_suffix("```").unwrap_or(rest);
        return rest.trim().to_string();
    }
    t.to_string()
}

// =========================================================================
// Behavior 动作解析器（动态标签集）
// =========================================================================

/// 解析 xllm behavior 协议响应。识别的动作标签由本次生效的 actions 决定；
/// 不在集合内的标签仍会生成一次调用，让 ToolManager 返回“tool not found”
/// 供模型纠正，而不是静默跳过。
#[derive(Debug, Clone, Default)]
pub struct XllmActionParser {
    /// action 名 → args schema。
    pub actions: BTreeMap<String, Value>,
}

impl XllmActionParser {
    pub fn new(actions: &[ResolvedTool]) -> Self {
        Self {
            actions: actions
                .iter()
                .map(|t| (t.name.clone(), t.args_schema.clone()))
                .collect(),
        }
    }

    fn coerce(schema: Option<&Value>, raw: String) -> Value {
        let ty = schema
            .and_then(|s| s.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("string");
        match ty {
            "integer" => raw
                .trim()
                .parse::<i64>()
                .map(Value::from)
                .unwrap_or(Value::String(raw)),
            "number" => raw
                .trim()
                .parse::<f64>()
                .map(Value::from)
                .unwrap_or(Value::String(raw)),
            "boolean" => match raw.trim() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => Value::String(raw),
            },
            "object" | "array" => {
                serde_json::from_str::<Value>(raw.trim()).unwrap_or(Value::String(raw))
            }
            _ => Value::String(raw),
        }
    }

    fn element_to_call(&self, el: &XmlElement, auto_id: &mut u32) -> AiToolCall {
        let schema = self.actions.get(&el.name);
        let props = schema
            .and_then(|s| s.get("properties"))
            .and_then(Value::as_object);
        let mut args: HashMap<String, Value> = HashMap::new();
        let mut call_id = None;
        for (k, v) in &el.attrs {
            if k == "call_id" {
                call_id = Some(v.clone());
                continue;
            }
            args.insert(
                k.clone(),
                Self::coerce(props.and_then(|p| p.get(k)), v.clone()),
            );
        }
        let children = el.children();
        if !children.is_empty() {
            for child in &children {
                args.insert(
                    child.name.clone(),
                    Self::coerce(props.and_then(|p| p.get(&child.name)), child.text()),
                );
            }
        } else if !el.body.trim().is_empty() {
            let body_arg = schema.and_then(|s| action_body_arg(&el.name, s));
            let text = el.text();
            match body_arg {
                Some(b) if !args.contains_key(&b) => {
                    args.insert(b, Value::String(text));
                }
                _ => {
                    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(text.trim()) {
                        for (k, v) in obj {
                            args.entry(k).or_insert(v);
                        }
                    } else if let Some(b) = body_arg {
                        args.entry(b).or_insert(Value::String(text));
                    } else {
                        args.insert("body".to_string(), Value::String(text));
                    }
                }
            }
        }
        *auto_id += 1;
        AiToolCall {
            name: el.name.clone(),
            args,
            call_id: call_id.unwrap_or_else(|| format!("action-{auto_id}")),
        }
    }
}

impl LLMResultParser for XllmActionParser {
    fn parse(&self, response: &AiResponse) -> Result<LLMBehaviorResult, String> {
        let raw_text = response.message.text_content();
        let provider_calls = response.message.tool_calls();
        if raw_text.trim().is_empty() && provider_calls.is_empty() {
            return Err("empty response: no text and no tool_calls".to_string());
        }
        let unfenced = strip_code_fences(&raw_text);
        let top = scan_xml_elements(&unfenced);
        let region_el = top.iter().find(|e| e.name == "response").cloned();
        let region_children = match &region_el {
            Some(r) => r.children(),
            None => top.clone(),
        };
        let first_text = |name: &str| -> Option<String> {
            region_children
                .iter()
                .find(|e| e.name == name)
                .map(|e| e.text().trim().to_string())
                .filter(|s| !s.is_empty())
        };
        let thought = first_text("thinking");
        let observation = first_text("observation");
        let mut next_behavior = first_text("next_behavior");
        let self_report = region_children
            .iter()
            .filter(|e| e.name == "report")
            .last()
            .map(|e| e.text());

        let mut do_actions = Vec::new();
        let mut messages_to_send = Vec::new();
        let mut auto_id = 0u32;
        if let Some(actions_el) = region_children.iter().find(|e| e.name == "actions") {
            for el in actions_el.children() {
                if el.name == "sendmsg" && !self.actions.contains_key("sendmsg") {
                    messages_to_send.push(SendMessageRecord {
                        target: el.attr("target").unwrap_or("user").to_string(),
                        body: el.text(),
                    });
                    continue;
                }
                do_actions.push(self.element_to_call(&el, &mut auto_id));
            }
        }
        if !provider_calls.is_empty() {
            do_actions = provider_calls;
        }
        // oneshot 终止规则：没有动作、带 report 的一步就是最终答案。
        if do_actions.is_empty()
            && messages_to_send.is_empty()
            && self_report.is_some()
            && next_behavior.is_none()
        {
            next_behavior = Some("done".to_string());
        }
        Ok(LLMBehaviorResult {
            do_actions,
            next_behavior,
            assistant_text: raw_text,
            observation,
            thought,
            self_report,
            messages_to_send,
        })
    }
}

// =========================================================================
// 观察者（F09）：SDK 只发事件，由调用方决定 stderr 详略。
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPhase {
    PreparingInput,
    FileModel,
    WaitingModel,
    ExecutingTool,
    CompactingContext,
    SavingResult,
}

impl RunPhase {
    pub fn label(&self) -> &'static str {
        match self {
            RunPhase::PreparingInput => "preparing input",
            RunPhase::FileModel => "analyzing attachments with file model",
            RunPhase::WaitingModel => "waiting for model",
            RunPhase::ExecutingTool => "executing tool",
            RunPhase::CompactingContext => "compacting context",
            RunPhase::SavingResult => "saving result",
        }
    }
}

#[derive(Debug, Clone)]
pub enum RunEvent {
    Phase {
        phase: RunPhase,
        detail: String,
    },
    LlmStarted {
        model: String,
    },
    LlmFinished {
        ok: bool,
        elapsed_ms: u64,
    },
    ToolStarted {
        name: String,
        call_id: String,
        command: Option<String>,
    },
    ToolFinished {
        name: String,
        call_id: String,
        command: Option<String>,
        ok: bool,
        duration_ms: u64,
    },
    Warning(String),
    Debug(String),
}

pub trait RunObserver: Send + Sync {
    fn on_event(&self, run_id: &str, event: RunEvent);
}

pub struct NoopRunObserver;

impl RunObserver for NoopRunObserver {
    fn on_event(&self, _run_id: &str, _event: RunEvent) {}
}

struct ObserverWorklog {
    run_id: String,
    observer: Arc<dyn RunObserver>,
    llm_started_at: Mutex<Option<u64>>,
    tool_commands: Mutex<HashMap<String, String>>,
}

#[async_trait]
impl WorklogSink for ObserverWorklog {
    async fn emit(&self, event: WorkEvent) {
        let ev = match event {
            WorkEvent::LLMStarted { model, .. } => {
                *self.llm_started_at.lock().expect("lock") = Some(now_ms());
                Some(RunEvent::LlmStarted { model })
            }
            WorkEvent::LLMFinished { ok, .. } => {
                let started = self.llm_started_at.lock().expect("lock").take();
                Some(RunEvent::LlmFinished {
                    ok,
                    elapsed_ms: started.map(|s| now_ms().saturating_sub(s)).unwrap_or(0),
                })
            }
            WorkEvent::LLMInferenceFailed { error, .. } => {
                Some(RunEvent::Warning(format!("model request failed: {error}")))
            }
            WorkEvent::ToolCallPlanned {
                tool,
                call_id,
                args,
                ..
            } => {
                let command = if tool == TOOL_EXEC {
                    args.get("command")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                } else {
                    None
                };
                let mut commands = self.tool_commands.lock().expect("lock");
                commands.remove(&call_id);
                if let Some(command) = &command {
                    commands.insert(call_id.clone(), command.clone());
                }
                Some(RunEvent::ToolStarted {
                    name: tool,
                    call_id,
                    command,
                })
            }
            WorkEvent::ToolCallFinished {
                tool,
                call_id,
                ok,
                duration_ms,
                ..
            } => Some(RunEvent::ToolFinished {
                name: tool,
                command: self.tool_commands.lock().expect("lock").remove(&call_id),
                call_id,
                ok,
                duration_ms,
            }),
            WorkEvent::ToolCallFailed {
                tool,
                call_id,
                message,
                ..
            } => Some(RunEvent::ToolFinished {
                name: format!("{tool} ({message})"),
                command: self.tool_commands.lock().expect("lock").remove(&call_id),
                call_id,
                ok: false,
                duration_ms: 0,
            }),
            WorkEvent::OutputParseFailed { error, .. } => {
                Some(RunEvent::Warning(format!("response parse failed: {error}")))
            }
            WorkEvent::ToolDispatchFailed {
                tool,
                call_id,
                message,
                ..
            } => {
                self.tool_commands.lock().expect("lock").remove(&call_id);
                Some(RunEvent::Warning(format!(
                    "tool `{tool}` dispatch failed: {message}"
                )))
            }
            WorkEvent::CheckpointFailed { error, .. } => {
                Some(RunEvent::Warning(format!("checkpoint failed: {error}")))
            }
            WorkEvent::ContextRewritten {
                from_messages,
                to_messages,
                ..
            } => Some(RunEvent::Debug(format!(
                "context rewritten: {from_messages} -> {to_messages} messages"
            ))),
            WorkEvent::SelfReportSet { chars, .. } => {
                Some(RunEvent::Debug(format!("report set ({chars} chars)")))
            }
            WorkEvent::MessageSent { target, .. } => Some(RunEvent::Debug(format!(
                "message intent to {target} recorded"
            ))),
        };
        if let Some(ev) = ev {
            self.observer.on_event(&self.run_id, ev);
        }
    }
}

// =========================================================================
// 工具装配（F04）
// =========================================================================

/// 运行环境注入：LLM 客户端工厂、具名工具、观察者。
#[derive(Clone)]
pub struct XllmDeps {
    pub llm_factory: Arc<dyn LlmClientFactory>,
    /// `tools: - name: xxx` 可选择的具名工具。
    pub host_tools: HashMap<String, Arc<dyn AgentTool>>,
    pub observer: Arc<dyn RunObserver>,
    /// 锁目录（默认 `~/.xllm/locks`）。
    pub lock_dir: Option<PathBuf>,
}

impl Default for XllmDeps {
    fn default() -> Self {
        Self {
            llm_factory: Arc::new(DefaultLlmClientFactory),
            host_tools: HashMap::new(),
            observer: Arc::new(NoopRunObserver),
            lock_dir: None,
        }
    }
}

impl XllmDeps {
    /// 测试 / 宿主直接注入一个 LlmClient（忽略 Provider 配置）。
    pub fn with_llm(mut self, llm: Arc<dyn LlmClient>) -> Self {
        self.llm_factory = Arc::new(FixedLlmClientFactory { llm });
        self
    }

    pub fn with_observer(mut self, observer: Arc<dyn RunObserver>) -> Self {
        self.observer = observer;
        self
    }

    pub fn with_host_tool(mut self, tool: Arc<dyn AgentTool>) -> Self {
        let name = tool.spec().name;
        self.host_tools.insert(name, tool);
        self
    }

    pub fn with_lock_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.lock_dir = Some(dir.into());
        self
    }

    fn effective_lock_dir(&self) -> PathBuf {
        self.lock_dir
            .clone()
            .unwrap_or_else(|| resolve_config_path(DEFAULT_LOCK_DIR, Path::new("/")))
    }
}

/// 把 waist 的 ToolManager 接到一组 `AgentTool` 上；同时记录产物路径。
pub struct XllmToolManager {
    workdir: PathBuf,
    tools: BTreeMap<String, Arc<dyn AgentTool>>,
    step_idx: AtomicU32,
    session_template: SessionRuntimeContext,
    artifacts: Mutex<Vec<String>>,
    cancel: Arc<tokio::sync::watch::Sender<bool>>,
    deadline: Mutex<Option<(tokio::time::Instant, u64)>>,
}

impl XllmToolManager {
    pub fn new(workdir: PathBuf, run_id: &str, loop_model: LoopModel) -> Self {
        Self {
            workdir,
            tools: BTreeMap::new(),
            step_idx: AtomicU32::new(0),
            session_template: SessionRuntimeContext {
                trace_id: run_id.to_string(),
                agent_name: "xllm".to_string(),
                behavior: loop_model.as_str().to_string(),
                step_idx: 0,
                wakeup_id: String::new(),
                session_id: run_id.to_string(),
                read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
            },
            artifacts: Mutex::new(Vec::new()),
            cancel: Arc::new(tokio::sync::watch::channel(false).0),
            deadline: Mutex::new(None),
        }
    }

    /// 本次执行的总时长边界：运行中的工具调用到点即被取消（F08 `timeout`）。
    fn set_deadline(&self, timeout_secs: u64) {
        let at = tokio::time::Instant::now() + Duration::from_secs(timeout_secs);
        *self.deadline.lock().expect("deadline lock") = Some((at, timeout_secs));
    }

    pub fn register(
        &mut self,
        tool: Arc<dyn AgentTool>,
        source: &str,
    ) -> Result<ResolvedTool, XllmError> {
        let spec = tool.spec();
        let name = spec.name.trim().to_string();
        if name.is_empty() {
            return Err(XllmError::Tools(format!("{source}: tool with empty name")));
        }
        if self.tools.contains_key(&name) {
            return Err(XllmError::Tools(format!(
                "tool name `{name}` from {source} conflicts with an already expanded tool of the same name"
            )));
        }
        self.tools.insert(name.clone(), tool);
        Ok(ResolvedTool {
            name,
            description: spec.description,
            args_schema: spec.args_schema,
            source: source.to_string(),
        })
    }

    pub fn artifacts(&self) -> Vec<String> {
        self.artifacts.lock().expect("artifacts lock").clone()
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }
}

#[async_trait]
impl ToolManager for XllmToolManager {
    async fn call_tool(&self, call: AiToolCall) -> Result<Observation, ToolDispatchError> {
        let call_id = call.call_id.clone();
        let mut ctx = self.session_template.clone();
        ctx.step_idx = self.step_idx.fetch_add(1, Ordering::SeqCst) + 1;
        let Some(tool) = self.tools.get(&call.name) else {
            return Ok(Observation::Error {
                call_id,
                message: format!(
                    "tool `{}` is not available in this run (available: {})",
                    call.name,
                    self.tools.keys().cloned().collect::<Vec<_>>().join(", ")
                ),
                tool_result: None,
            });
        };
        let path_arg = call
            .args
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_string);
        let args = Value::Object(call.args.into_iter().collect());
        let deadline = *self.deadline.lock().expect("deadline lock");
        let mut cancel_rx = self.cancel.subscribe();
        let result = tokio::select! {
            result = tool.call(&ctx, args) => result,
            _ = cancel_rx.wait_for(|cancelled| *cancelled) => {
                return Ok(Observation::Error {
                    call_id,
                    message: format!("tool `{}` was cancelled: run interrupted", call.name),
                    tool_result: None,
                });
            }
            _ = async {
                match deadline {
                    Some((at, _)) => tokio::time::sleep_until(at).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                return Ok(Observation::Error {
                    call_id,
                    message: format!(
                        "tool `{}` was cancelled: total execution time limit ({}s) reached",
                        call.name,
                        deadline.map(|(_, secs)| secs).unwrap_or_default()
                    ),
                    tool_result: None,
                });
            }
        };
        Ok(match result {
            Ok(res) => {
                if res.status == AgentToolStatus::Success
                    && matches!(call.name.as_str(), TOOL_WRITE_FILE | TOOL_EDIT_FILE)
                {
                    if let Some(p) = path_arg {
                        let abs = if Path::new(&p).is_absolute() {
                            PathBuf::from(&p)
                        } else {
                            normalize_path(&self.workdir.join(&p))
                        };
                        let mut a = self.artifacts.lock().expect("artifacts lock");
                        let s = abs.display().to_string();
                        if !a.contains(&s) {
                            a.push(s);
                        }
                    }
                }
                map_result_to_observation(call_id, res)
            }
            Err(e) => Observation::Error {
                call_id,
                message: e.to_string(),
                tool_result: None,
            },
        })
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        self.tools
            .values()
            .map(|t| {
                let spec = t.spec();
                ToolSpecLite {
                    name: spec.name,
                    description: spec.description,
                    args_schema: spec.args_schema,
                }
            })
            .collect()
    }
}

fn map_result_to_observation(call_id: String, result: AgentToolResult) -> Observation {
    let tool_result = Some(result.to_tool_result_view());
    match result.status {
        AgentToolStatus::Success => {
            let text = result
                .output
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| {
                    if result.summary.trim().is_empty() {
                        result.details.to_string()
                    } else {
                        result.summary.clone()
                    }
                });
            let bytes = text.len();
            let truncated = result
                .details
                .get("output_truncated")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            Observation::Success {
                call_id,
                content: Value::String(text),
                bytes,
                truncated,
                tool_result,
            }
        }
        AgentToolStatus::Error => {
            let summary = result.summary.trim();
            let output = result
                .output
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let message = match (summary.is_empty(), output) {
                (false, Some(out)) => format!("{summary}\n{out}"),
                (false, None) => summary.to_string(),
                (true, Some(out)) => out.to_string(),
                (true, None) => "tool error".to_string(),
            };
            Observation::Error {
                call_id,
                message,
                tool_result,
            }
        }
        AgentToolStatus::Pending => Observation::Pending {
            call_id,
            tool_result,
        },
    }
}

/// MCP 远端工具（HTTP JSON-RPC `tools/call`）。
pub struct McpRemoteTool {
    spec: ToolSpec,
    endpoint: String,
    remote_name: String,
    client: reqwest::Client,
}

#[async_trait]
impl AgentTool for McpRemoteTool {
    fn spec(&self) -> ToolSpec {
        self.spec.clone()
    }

    fn calling(&self) -> crate::CallingConventions {
        crate::CallingConventions::ALL
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let body = json!({
            "jsonrpc": "2.0",
            "id": format!("{}:{}:{}", ctx.trace_id, ctx.step_idx, self.spec.name),
            "method": "tools/call",
            "params": { "name": self.remote_name, "arguments": args }
        });
        let resp = tokio::time::timeout(
            Duration::from_millis(MCP_CALL_TIMEOUT_MS),
            self.client.post(&self.endpoint).json(&body).send(),
        )
        .await
        .map_err(|_| AgentToolError::Timeout)?
        .map_err(|e| AgentToolError::ExecFailed(format!("mcp request failed: {e}")))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| AgentToolError::ExecFailed(format!("read mcp response failed: {e}")))?;
        if !status.is_success() {
            return Err(AgentToolError::ExecFailed(format!(
                "mcp server returned http {}: {}",
                status.as_u16(),
                truncate_chars(&text, 512)
            )));
        }
        let payload: Value = serde_json::from_str(&text)
            .map_err(|e| AgentToolError::ExecFailed(format!("invalid mcp response json: {e}")))?;
        if let Some(err) = payload.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| err.to_string());
            return Err(AgentToolError::ExecFailed(format!(
                "mcp tool call error: {msg}"
            )));
        }
        let result = payload.get("result").cloned().ok_or_else(|| {
            AgentToolError::ExecFailed("mcp response missing `result` field".to_string())
        })?;
        let is_error = result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let content_text = result
            .get("content")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|c| c.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        if is_error {
            return Err(AgentToolError::ExecFailed(format!(
                "mcp tool returned error: {}",
                if content_text.is_empty() {
                    result.to_string()
                } else {
                    content_text
                }
            )));
        }
        let output = if content_text.is_empty() {
            result.to_string()
        } else {
            content_text
        };
        Ok(AgentToolResult::from_details(result)
            .with_title(format!("mcp {} => success", self.spec.name))
            .with_output(output))
    }
}

/// 通过 `tools/list` 发现 MCP 服务提供的工具。
pub async fn discover_mcp_tools(endpoint: &str) -> Result<Vec<Arc<dyn AgentTool>>, XllmError> {
    let client = reqwest::Client::builder()
        .build()
        .map_err(|e| XllmError::Tools(format!("mcp {endpoint}: build http client failed: {e}")))?;
    let body = json!({"jsonrpc":"2.0","id":"xllm-tools-list","method":"tools/list","params":{}});
    let resp = tokio::time::timeout(
        Duration::from_millis(MCP_DISCOVERY_TIMEOUT_MS),
        client.post(endpoint).json(&body).send(),
    )
    .await
    .map_err(|_| XllmError::Tools(format!("mcp {endpoint}: tools/list timed out")))?
    .map_err(|e| XllmError::Tools(format!("mcp {endpoint}: connection failed: {e}")))?;
    let status = resp.status();
    let text = resp
        .text()
        .await
        .map_err(|e| XllmError::Tools(format!("mcp {endpoint}: read response failed: {e}")))?;
    if !status.is_success() {
        return Err(XllmError::Tools(format!(
            "mcp {endpoint}: tools/list returned http {}: {}",
            status.as_u16(),
            truncate_chars(&text, 256)
        )));
    }
    let payload: Value = serde_json::from_str(&text)
        .map_err(|e| XllmError::Tools(format!("mcp {endpoint}: invalid JSON: {e}")))?;
    if let Some(err) = payload.get("error") {
        return Err(XllmError::Tools(format!(
            "mcp {endpoint}: tools/list error: {}",
            err.get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        )));
    }
    let tools = payload
        .get("result")
        .and_then(|r| r.get("tools"))
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            XllmError::Tools(format!(
                "mcp {endpoint}: tools/list result has no `tools` array"
            ))
        })?;
    let mut out: Vec<Arc<dyn AgentTool>> = Vec::new();
    for t in tools {
        let name = t
            .get("name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| XllmError::Tools(format!("mcp {endpoint}: a tool entry has no name")))?
            .to_string();
        let description = t
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args_schema = t
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| json!({"type":"object"}));
        out.push(Arc::new(McpRemoteTool {
            spec: ToolSpec {
                name: name.clone(),
                description,
                args_schema,
                output_schema: json!({"type":"object"}),
                usage: None,
            },
            endpoint: endpoint.to_string(),
            remote_name: name,
            client: client.clone(),
        }));
    }
    Ok(out)
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let t: String = s.chars().take(max).collect();
        format!("{t}…")
    }
}

/// 内置组 `bash`：`read_file` / `write_file` / `edit_file` / `exec`。
fn builtin_bash_group(
    workdir: &Path,
    filesystem_policy: FilesystemPolicy,
) -> Vec<Arc<dyn AgentTool>> {
    let mut cfg = FileToolConfig::new(workdir.to_path_buf());
    let restrict_cwd = filesystem_policy == FilesystemPolicy::Workspace;
    if !restrict_cwd {
        cfg.allowed_read_roots.clear();
        cfg.allowed_write_roots.clear();
    }
    let audit = Arc::new(NoopFileWriteAudit);
    let bash_cfg = LlmBashConfig::local_workspace(workdir.to_path_buf())
        .with_tool_name(TOOL_EXEC)
        .with_restrict_cwd(restrict_cwd)
        .with_overlay(BinOverlayConfig::disabled())
        .with_default_timeout_ms(EXEC_DEFAULT_TIMEOUT_MS)
        .with_max_timeout_ms(EXEC_MAX_TIMEOUT_MS)
        .with_max_output_bytes(EXEC_MAX_OUTPUT_BYTES)
        .with_allow_env(true);
    vec![
        Arc::new(TypedToolHandle::with_null_host(ReadFileTool::new(
            cfg.clone(),
        ))),
        Arc::new(TypedToolHandle::with_null_host(WriteFileTool::new(
            cfg.clone(),
            audit.clone(),
        ))),
        Arc::new(TypedToolHandle::with_null_host(EditFileTool::new(
            cfg, audit,
        ))),
        Arc::new(ExecBashTool::new(bash_cfg)),
    ]
}

async fn expand_tool_sources(
    sources: &[ToolSource],
    workdir: &Path,
    filesystem_policy: FilesystemPolicy,
    deps: &XllmDeps,
    manager: &mut XllmToolManager,
) -> Result<Vec<ResolvedTool>, XllmError> {
    let mut out = Vec::new();
    for src in sources {
        let desc = src.describe();
        match src {
            ToolSource::Group { groupname } => {
                if groupname != BUILTIN_TOOL_GROUP_BASH {
                    return Err(XllmError::Tools(format!(
                        "unknown builtin tool group `{groupname}` (available: {BUILTIN_TOOL_GROUP_BASH})"
                    )));
                }
                for t in builtin_bash_group(workdir, filesystem_policy) {
                    out.push(manager.register(t, &desc)?);
                }
            }
            ToolSource::Mcp { endpoint } => {
                let tools = discover_mcp_tools(endpoint).await?;
                if tools.is_empty() {
                    return Err(XllmError::Tools(format!(
                        "mcp {endpoint}: server exposes no tools"
                    )));
                }
                for t in tools {
                    out.push(manager.register(t, &desc)?);
                }
            }
            ToolSource::Named { name } => {
                let tool = deps.host_tools.get(name).cloned().ok_or_else(|| {
                    XllmError::Tools(format!(
                        "named tool `{name}` is not provided by the runtime environment{}",
                        if deps.host_tools.is_empty() {
                            String::new()
                        } else {
                            format!(
                                " (available: {})",
                                deps.host_tools
                                    .keys()
                                    .cloned()
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            )
                        }
                    ))
                })?;
                out.push(manager.register(tool, &desc)?);
            }
        }
    }
    Ok(out)
}

/// 依据工具配置展开来源并做 loop 校验，得到最终工具集与派发器。
pub async fn build_toolset(
    cfg: &ToolsConfig,
    sources: BTreeMap<String, String>,
    loop_model: LoopModel,
    workdir: &Path,
    run_id: &str,
    deps: &XllmDeps,
) -> Result<(EffectiveTools, XllmToolManager), XllmError> {
    let enabled = cfg.enabled.unwrap_or(false);
    let filesystem_policy = cfg.filesystem_policy.unwrap_or_default();
    let tools2actions = cfg.tools2actions.unwrap_or(false);
    let mut manager = XllmToolManager::new(workdir.to_path_buf(), run_id, loop_model);
    let mut eff = EffectiveTools {
        enabled,
        filesystem_policy,
        tools2actions,
        tool_sources: cfg.tools.clone().unwrap_or_default(),
        action_sources: cfg.actions.clone().unwrap_or_default(),
        native: Vec::new(),
        actions: Vec::new(),
        bash_tools: cfg.bash_tools.clone().unwrap_or_default(),
        exec_enabled: false,
        sources,
    };
    if loop_model == LoopModel::FunctionCall {
        if tools2actions {
            return Err(XllmError::Capability(
                "`tools2actions: true` requires `loop_model: behavior`; function_call cannot convert tools into actions".into(),
            ));
        }
        if cfg.actions.as_ref().map(|a| !a.is_empty()).unwrap_or(false) {
            return Err(XllmError::Capability(
                "`actions` are configured but `loop_model` is function_call; use `loop_model: behavior` or remove `actions`".into(),
            ));
        }
    }
    if !enabled {
        return Ok((eff, manager));
    }
    // 仅显式启用但未配置列表时使用默认 bash 组；显式空列表则不提供。
    let tool_sources = match &cfg.tools {
        Some(list) => list.clone(),
        None => vec![ToolSource::Group {
            groupname: BUILTIN_TOOL_GROUP_BASH.into(),
        }],
    };
    eff.tool_sources = tool_sources.clone();
    let native = expand_tool_sources(
        &tool_sources,
        workdir,
        filesystem_policy,
        deps,
        &mut manager,
    )
    .await?;
    let explicit_actions = expand_tool_sources(
        &eff.action_sources,
        workdir,
        filesystem_policy,
        deps,
        &mut manager,
    )
    .await?;
    match loop_model {
        LoopModel::FunctionCall => {
            eff.native = native;
        }
        LoopModel::Behavior => {
            if tools2actions {
                eff.actions = native;
                eff.actions.extend(explicit_actions);
                eff.native = Vec::new();
            } else {
                eff.native = native;
                eff.actions = explicit_actions;
            }
        }
    }
    eff.exec_enabled = manager.has(TOOL_EXEC)
        && (eff.native.iter().any(|t| t.name == TOOL_EXEC)
            || eff.actions.iter().any(|t| t.name == TOOL_EXEC));
    Ok((eff, manager))
}

// =========================================================================
// Provider / LlmClient（F03）
// =========================================================================

/// 依据 Provider 配置创建 LlmClient。
#[async_trait]
pub trait LlmClientFactory: Send + Sync {
    async fn create(&self, provider: &ProviderConfig) -> Result<Arc<dyn LlmClient>, XllmError>;
}

struct FixedLlmClientFactory {
    llm: Arc<dyn LlmClient>,
}

#[async_trait]
impl LlmClientFactory for FixedLlmClientFactory {
    async fn create(&self, _provider: &ProviderConfig) -> Result<Arc<dyn LlmClient>, XllmError> {
        Ok(self.llm.clone())
    }
}

/// 默认工厂：buckyos → AICC；openai → Chat Completions。
pub struct DefaultLlmClientFactory;

#[async_trait]
impl LlmClientFactory for DefaultLlmClientFactory {
    async fn create(&self, provider: &ProviderConfig) -> Result<Arc<dyn LlmClient>, XllmError> {
        match provider.effective_kind() {
            ProviderKind::Buckyos => {
                ensure_buckyos_runtime().await.map_err(|e| {
                    XllmError::Capability(format!(
                        "buckyos provider: cannot initialize BuckyOS runtime (check zone/session setup): {e}"
                    ))
                })?;
                let token = match &provider.session_token {
                    Some(r) => Some(r.resolve()?),
                    None => None,
                };
                Ok(Arc::new(AiccLlmClient::with_session_token(token)))
            }
            ProviderKind::Openai => {
                let api_key = match &provider.api_key {
                    Some(r) => Some(r.resolve()?),
                    None => std::env::var("OPENAI_API_KEY").ok(),
                };
                Ok(Arc::new(OpenAiLlmClient::new(
                    provider.base_url.clone(),
                    api_key,
                    provider.headers.clone(),
                )?))
            }
        }
    }
}

/// 初始化（或复用）BuckyOS API runtime，并完成登录。
///
/// 登录身份按以下顺序决定（可用 `BUCKYOS_APP_ID` 覆盖默认的 `buckycli`）：
/// 1. 设置了 `BUCKYOS_APPCLIENT_SESSION_TOKEN`：AppClient，直接使用该会话（OpenDAN
///    给工具注入的方式）。
/// 2. 在 OOD 本机且能读到设备私钥（`/opt/buckyos/security/...`）：以内核服务身份
///    （KernelService 语义，服务地址走 127.0.0.1）用设备私钥签登录断言，system-config 把它当作内核引导
///    断言接受，`buckycli` 在 RBAC 中属于 kernel 角色；这是 DV Test 环境下最直接的
///    方式。
/// 3. 否则：AppClient，用 dev 目录（`$BUCKYOS_DEV_HOME` / `~/.buckycli`）里的用户
///    私钥签断言，先到 verify-hub 换会话再登录（需要该 app 已安装在 zone 中）。
pub async fn ensure_buckyos_runtime() -> Result<(), Box<dyn std::error::Error>> {
    if get_buckyos_api_runtime().is_ok() {
        return Ok(());
    }
    let app_id = std::env::var("BUCKYOS_APP_ID")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "buckycli".to_string());
    let has_token = std::env::var("BUCKYOS_APPCLIENT_SESSION_TOKEN")
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if !has_token {
        // 2. OOD 本机：内核服务身份 + 设备私钥。服务型 runtime 要求存在
        //    `<APP_ID>_SESSION_TOKEN` 环境变量；留空表示“没有预置会话，用设备私钥登录”。
        let token_key = buckyos_api::get_service_session_token_env_key(app_id.as_str());
        if std::env::var_os(&token_key).is_none() {
            std::env::set_var(&token_key, "");
        }
        match init_buckyos_api_runtime(app_id.as_str(), None, BuckyOSRuntimeType::KernelService)
            .await
        {
            Ok(mut runtime) => match Box::pin(bootstrap_service_session(&mut runtime))
                .await
                .map_err(|e| e.to_string())
            {
                Ok(()) => {
                    Box::pin(runtime.login()).await?;
                    set_buckyos_api_runtime(runtime)?;
                    return Ok(());
                }
                Err(e) => {
                    log::info!(
                        "xllm: device-key login unavailable ({e}); falling back to AppClient login"
                    );
                }
            },
            Err(e) => {
                log::info!(
                    "xllm: service-style runtime init failed ({e}); falling back to AppClient login"
                );
            }
        }
    }

    // 1 / 3. AppClient。
    let mut runtime =
        init_buckyos_api_runtime(app_id.as_str(), None, BuckyOSRuntimeType::AppClient).await?;
    Box::pin(bootstrap_appclient_session(&mut runtime)).await?;
    Box::pin(runtime.login()).await?;
    set_buckyos_api_runtime(runtime)?;
    Ok(())
}

/// OOD 本机的服务身份：用设备密钥签一份 `sub = iss = 设备名` 的登录断言，通过
/// node gateway 上的 verify-hub 换取正式会话（system-config 只接受 verify-hub 会话或
/// node-daemon 签发的引导断言）。
async fn bootstrap_service_session(
    runtime: &mut buckyos_api::BuckyOSRuntime,
) -> Result<(), Box<dyn std::error::Error>> {
    if !runtime.session_token.read().await.trim().is_empty() {
        return Ok(());
    }
    runtime.load_device_private_key()?;
    let (Some(key), Some(device)) = (
        runtime.device_private_key.as_ref(),
        runtime.device_config.as_ref(),
    ) else {
        return Err("device config or signing key missing".into());
    };
    let (jwt, _) = buckyos_api::generate_service_login_assertion(
        device.name.as_str(),
        runtime.app_id.as_str(),
        device.name.as_str(),
        key,
    )?;
    let target = runtime.get_auth_target()?;
    let port = std::env::var("BUCKYOS_NODE_GATEWAY_PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(3180);
    let url = format!("http://127.0.0.1:{port}/kapi/verify-hub");
    let krpc = ::kRPC::kRPC::new_with_timeout_secs(&url, None, 30);
    let verify_hub = buckyos_api::VerifyHubClient::new(krpc);
    let pair = verify_hub.login_by_jwt(jwt.as_str(), target).await?;
    *runtime.session_token.write().await = pair.session_token;
    *runtime.refresh_token.write().await = pair.refresh_token;
    Ok(())
}

/// AppClient 且没有会话时：用本地私钥签登录断言，到 verify-hub 换取正式会话。
async fn bootstrap_appclient_session(
    runtime: &mut buckyos_api::BuckyOSRuntime,
) -> Result<(), Box<dyn std::error::Error>> {
    if !runtime.session_token.read().await.trim().is_empty() {
        return Ok(());
    }
    let target = runtime.get_auth_target()?;
    let jwt = if let (Some(key), Some(user_id)) =
        (runtime.user_private_key.as_ref(), runtime.user_id.as_ref())
    {
        buckyos_api::generate_user_login_assertion(user_id, runtime.app_id.as_str(), key)?.0
    } else {
        if let Err(e) = runtime.load_device_private_key() {
            log::info!("xllm: no user or device private key, login needs a session token: {e}");
            return Ok(());
        }
        let (Some(key), Some(device)) = (
            runtime.device_private_key.as_ref(),
            runtime.device_config.as_ref(),
        ) else {
            return Ok(());
        };
        let Some(owner) = runtime.app_owner_id.as_deref() else {
            return Ok(());
        };
        buckyos_api::generate_service_login_assertion(
            owner,
            runtime.app_id.as_str(),
            device.name.as_str(),
            key,
        )?
        .0
    };
    let verify_hub = runtime.get_verify_hub_client().await?;
    let pair = verify_hub.login_by_jwt(jwt.as_str(), target).await?;
    *runtime.session_token.write().await = pair.session_token;
    *runtime.refresh_token.write().await = pair.refresh_token;
    Ok(())
}

/// AICC → `LlmClient` 适配（buckyos Provider）。
pub struct AiccLlmClient {
    session_token: Option<String>,
}

impl AiccLlmClient {
    pub fn new() -> Self {
        Self {
            session_token: None,
        }
    }

    pub fn with_session_token(token: Option<String>) -> Self {
        Self {
            session_token: token,
        }
    }
}

impl Default for AiccLlmClient {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
fn build_aicc_llm_options(
    temperature: Option<f32>,
    max_completion_tokens: Option<u32>,
    force_json: bool,
    json_schema: Option<Value>,
    provider_options: Option<Value>,
) -> Value {
    let mut options = serde_json::Map::new();
    if let Some(temperature) = temperature {
        options.insert("temperature".into(), json!(temperature));
    }
    if let Some(max_completion_tokens) = max_completion_tokens {
        options.insert("max_tokens".into(), json!(max_completion_tokens));
    }
    if force_json {
        if let Some(schema) = json_schema {
            options.insert("response_schema".into(), schema);
        }
    }
    if let Some(extra) = provider_options {
        match extra {
            Value::Object(extra) => options.extend(extra),
            extra => {
                options.insert("provider_options".into(), extra);
            }
        }
    }
    Value::Object(options)
}

#[async_trait]
impl LlmClient for AiccLlmClient {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let LlmInferenceRequest {
            trace_id,
            messages,
            model_alias,
            fallbacks: _,
            temperature,
            max_completion_tokens,
            force_json,
            json_schema,
            provider_options,
            disable_capabilities,
            tool_specs,
            allow_tool_calls,
            abort: _,
        } = req;

        // Tool catalogue (only advertised when the policy lets the LLM call tools).
        let advertised_tools: Vec<AiToolSpec> = if allow_tool_calls {
            tool_specs
                .into_iter()
                .map(|spec| AiToolSpec {
                    tool_type: "function".to_string(),
                    name: spec.name,
                    description: spec.description,
                    args_json_schema: spec.args_schema,
                    output_schema: None,
                })
                .collect()
        } else {
            Vec::new()
        };
        let _ = provider_options;
        let mut disable = ModelDisable::default();
        for feature in disable_capabilities {
            disable.set_feature_disabled(&feature);
        }
        let response_format = if force_json {
            match json_schema {
                Some(schema) => Some(LlmResponseFormat::json_schema(
                    Some("llm_response".to_string()),
                    schema,
                    None,
                )),
                None => Some(LlmResponseFormat::json_object()),
            }
        } else {
            None
        };
        let request = LlmChatHelperRequest {
            logical_model: model_alias,
            trace_id,
            execution_mode: AiccExecutionMode::Immediate,
            requirements: HelperModelRequirement {
                tool_call: allow_tool_calls && !advertised_tools.is_empty(),
                json_schema: force_json,
                ..Default::default()
            },
            disable,
            policy: None,
            messages,
            tools: advertised_tools,
            response_format,
            temperature: temperature.map(f64::from),
            top_p: None,
            max_output_tokens: max_completion_tokens.map(u64::from),
            seed: None,
            stop: Vec::new(),
            output: None,
            idempotency_key: None,
            task_options: None,
            session_overlay: None,
            session_id: None,
        };

        let runtime = get_buckyos_api_runtime()
            .map_err(|e| provider_error_from_rpc("get buckyos runtime failed", e))?;
        if let Some(token) = &self.session_token {
            let mut guard = runtime.session_token.write().await;
            if guard.as_str() != token.as_str() {
                *guard = token.clone();
            }
        }
        let client = runtime
            .get_aicc_client()
            .await
            .map_err(|e| provider_error_from_rpc("get aicc client failed", e))?;
        let response = client
            .helper_llm_chat(request)
            .await
            .map_err(|e| provider_error_from_rpc("aicc helper.llm_chat failed", e))?;
        match response.status {
            AiMethodStatus::Succeeded => {
                let message = response.message.ok_or_else(|| {
                    LLMComputeError::provider(
                        ProviderFailure::Unknown,
                        "aicc helper.llm_chat succeeded but message is empty",
                    )
                })?;
                Ok(AiResponse {
                    message,
                    usage: response.usage,
                    cost: response.cost,
                    finish_reason: response.finish_reason,
                    provider_task_ref: response.provider_task_ref,
                    extra: None,
                })
            }
            AiMethodStatus::Failed => Err(LLMComputeError::provider(
                ProviderFailure::Unknown,
                format!(
                    "aicc helper.llm_chat failed: task_id={}, event_ref={}",
                    response.task_id,
                    response.event_ref.as_deref().unwrap_or("")
                ),
            )),
            AiMethodStatus::Running => Err(LLMComputeError::provider(
                ProviderFailure::Permanent,
                format!(
                    "aicc helper.llm_chat returned async task `{}`; xllm does not poll async tasks — use a synchronous-capable model",
                    response.task_id
                ),
            )),
        }
    }
}

/// kRPC 错误 → Provider 失败类别。
pub fn provider_error_from_rpc(context: &str, err: RPCErrors) -> LLMComputeError {
    let failure = match &err {
        RPCErrors::S2sTransientError(_) => ProviderFailure::Transient,
        RPCErrors::InvalidToken(_)
        | RPCErrors::TokenExpired(_)
        | RPCErrors::NoPermission(_)
        | RPCErrors::InvalidPassword
        | RPCErrors::UserNotFound(_)
        | RPCErrors::UnknownMethod(_)
        | RPCErrors::ServiceNotValid(_)
        | RPCErrors::S2sPermanentError(_) => ProviderFailure::Permanent,
        RPCErrors::ReasonError(_)
        | RPCErrors::ParseRequestError(_)
        | RPCErrors::ParserResponseError(_)
        | RPCErrors::KeyNotExist(_) => ProviderFailure::Unknown,
    };
    LLMComputeError::provider(failure, format!("{context}: {err}"))
}

/// OpenAI 兼容 Chat Completions 客户端（openai Provider）。
pub struct OpenAiLlmClient {
    base_url: String,
    api_key: Option<String>,
    headers: BTreeMap<String, String>,
    client: reqwest::Client,
}

impl OpenAiLlmClient {
    pub fn new(
        base_url: Option<String>,
        api_key: Option<String>,
        headers: BTreeMap<String, String>,
    ) -> Result<Self, XllmError> {
        let base_url = base_url
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "https://api.openai.com/v1".to_string())
            .trim_end_matches('/')
            .to_string();
        if api_key.as_deref().map(str::trim).unwrap_or("").is_empty() {
            return Err(XllmError::Capability(
                "openai provider: no API key; set `provider.api_key`, `provider.api_key_env` or the OPENAI_API_KEY environment variable".into(),
            ));
        }
        let client = reqwest::Client::builder().build().map_err(|e| {
            XllmError::Capability(format!("openai provider: build http client failed: {e}"))
        })?;
        Ok(Self {
            base_url,
            api_key,
            headers,
            client,
        })
    }

    fn content_to_openai(content: &[AiContent]) -> Value {
        let mut parts = Vec::new();
        for c in content {
            match c {
                AiContent::Text { text } => parts.push(json!({"type":"text","text":text})),
                AiContent::Image { source } => {
                    let url = match source {
                        ResourceRef::Url { url, .. } => url.clone(),
                        ResourceRef::Base64 { mime, data_base64 } => {
                            format!("data:{mime};base64,{data_base64}")
                        }
                        ResourceRef::NamedObject { obj_id } => obj_id.to_string(),
                    };
                    parts.push(json!({"type":"image_url","image_url":{"url":url}}));
                }
                AiContent::Document { source, title } => {
                    let label = title.clone().unwrap_or_else(|| "document".into());
                    let desc = match source {
                        ResourceRef::Url { url, .. } => url.clone(),
                        _ => "(inline document omitted)".into(),
                    };
                    parts.push(json!({"type":"text","text":format!("[{label}: {desc}]")}));
                }
                AiContent::Thinking { .. }
                | AiContent::ProviderState { .. }
                | AiContent::ToolUse { .. }
                | AiContent::ToolResult { .. } => {}
            }
        }
        if parts.len() == 1 && parts[0].get("type").and_then(Value::as_str) == Some("text") {
            return parts[0]["text"].clone();
        }
        Value::Array(parts)
    }

    fn messages_to_openai(messages: &[AiMessage]) -> Vec<Value> {
        let mut out = Vec::new();
        for m in messages {
            match m.role {
                AiRole::System | AiRole::Developer => out.push(json!({
                    "role": "system",
                    "content": m.text_content()
                })),
                AiRole::User => out.push(json!({
                    "role": "user",
                    "content": Self::content_to_openai(&m.content)
                })),
                AiRole::Assistant => {
                    let text = m.text_content();
                    let calls: Vec<Value> = m
                        .tool_calls()
                        .into_iter()
                        .map(|c| {
                            json!({
                                "id": c.call_id,
                                "type": "function",
                                "function": {
                                    "name": c.name,
                                    "arguments": Value::Object(c.args.into_iter().collect()).to_string()
                                }
                            })
                        })
                        .collect();
                    let mut obj = json!({"role":"assistant"});
                    obj["content"] = if text.is_empty() {
                        Value::Null
                    } else {
                        Value::String(text)
                    };
                    if !calls.is_empty() {
                        obj["tool_calls"] = Value::Array(calls);
                    }
                    out.push(obj);
                }
                AiRole::Tool => {
                    for block in &m.content {
                        if let AiContent::ToolResult {
                            call_id, content, ..
                        } = block
                        {
                            let text = content
                                .iter()
                                .filter_map(|c| c.text_str())
                                .collect::<Vec<_>>()
                                .join("\n");
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": call_id,
                                "content": text
                            }));
                        }
                    }
                }
            }
        }
        out
    }
}

#[async_trait]
impl LlmClient for OpenAiLlmClient {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        let mut body = json!({
            "model": req.model_alias,
            "messages": Self::messages_to_openai(&req.messages),
        });
        if let Some(t) = req.temperature {
            body["temperature"] = json!(t);
        }
        if let Some(m) = req.max_completion_tokens {
            body["max_tokens"] = json!(m);
        }
        if req.allow_tool_calls && !req.tool_specs.is_empty() {
            body["tools"] = Value::Array(
                req.tool_specs
                    .iter()
                    .map(|s| {
                        json!({
                            "type": "function",
                            "function": {
                                "name": s.name,
                                "description": s.description,
                                "parameters": s.args_schema
                            }
                        })
                    })
                    .collect(),
            );
            body["tool_choice"] = json!("auto");
        }
        if req.force_json {
            body["response_format"] = match req.json_schema {
                Some(schema) => {
                    json!({"type":"json_schema","json_schema":{"name":"xllm_result","schema":schema}})
                }
                None => json!({"type":"json_object"}),
            };
        }
        if let Some(Value::Object(extra)) = req.provider_options {
            for (k, v) in extra {
                body[k] = v;
            }
        }
        let mut request = self
            .client
            .post(format!("{}/chat/completions", self.base_url))
            .json(&body);
        if let Some(key) = &self.api_key {
            request = request.bearer_auth(key);
        }
        for (k, v) in &self.headers {
            request = request.header(k, v);
        }
        let abort = req.abort.clone();
        let response = tokio::select! {
            r = request.send() => r,
            _ = abort.cancelled() => return Err(LLMComputeError::Cancelled),
        }
        .map_err(|e| {
            let failure = if e.is_timeout() || e.is_connect() || e.is_request() {
                ProviderFailure::Transient
            } else {
                ProviderFailure::Unknown
            };
            LLMComputeError::provider(failure, format!("openai request failed: {e}"))
        })?;
        let status = response.status();
        let text = response.text().await.map_err(|e| {
            LLMComputeError::provider(
                ProviderFailure::Transient,
                format!("openai read body failed: {e}"),
            )
        })?;
        if !status.is_success() {
            let code = status.as_u16();
            let failure = match code {
                401 | 403 | 404 | 400 | 422 => ProviderFailure::Permanent,
                408 | 409 | 425 | 429 | 500 | 502 | 503 | 504 => ProviderFailure::Transient,
                _ => ProviderFailure::Unknown,
            };
            return Err(LLMComputeError::provider(
                failure,
                format!("openai http {code}: {}", truncate_chars(&text, 512)),
            ));
        }
        let parsed: Value = serde_json::from_str(&text).map_err(|e| {
            LLMComputeError::provider(
                ProviderFailure::Unknown,
                format!("openai invalid JSON: {e}"),
            )
        })?;
        let choice = parsed
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .cloned()
            .ok_or_else(|| {
                LLMComputeError::provider(
                    ProviderFailure::Unknown,
                    "openai response has no choices",
                )
            })?;
        let message = choice.get("message").cloned().unwrap_or(Value::Null);
        let content = message
            .get("content")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut tool_calls = Vec::new();
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            for c in calls {
                let id = c
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let name = c
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let args_raw = c
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(Value::as_str)
                    .unwrap_or("{}");
                let args: HashMap<String, Value> = match serde_json::from_str::<Value>(args_raw) {
                    Ok(Value::Object(m)) => m.into_iter().collect(),
                    _ => HashMap::new(),
                };
                tool_calls.push(AiToolCall {
                    name,
                    args,
                    call_id: id,
                });
            }
        }
        let mut resp = AiResponse::from_parts(content, tool_calls, Vec::new());
        resp.finish_reason = choice
            .get("finish_reason")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(u) = parsed.get("usage") {
            resp.usage = Some(AiUsage {
                input_tokens: u.get("prompt_tokens").and_then(Value::as_u64),
                output_tokens: u.get("completion_tokens").and_then(Value::as_u64),
                total_tokens: u.get("total_tokens").and_then(Value::as_u64),
                request_units: Some(1),
                ..Default::default()
            });
        }
        if let Some(model) = parsed.get("model").and_then(Value::as_str) {
            resp.extra = Some(json!({ "model": model }));
        }
        Ok(resp)
    }
}

/// 给任意 LlmClient 加单次请求超时（`llm_timeout`），并记录实际返回的模型信息。
pub struct TimeoutLlmClient {
    inner: Arc<dyn LlmClient>,
    timeout: Duration,
    last_model: Mutex<Option<String>>,
    calls: AtomicU64,
}

impl TimeoutLlmClient {
    pub fn new(inner: Arc<dyn LlmClient>, timeout: Duration) -> Self {
        Self {
            inner,
            timeout,
            last_model: Mutex::new(None),
            calls: AtomicU64::new(0),
        }
    }

    pub fn last_model(&self) -> Option<String> {
        self.last_model.lock().expect("lock").clone()
    }

    pub fn calls(&self) -> u64 {
        self.calls.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmClient for TimeoutLlmClient {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let result = tokio::time::timeout(self.timeout, self.inner.infer(req))
            .await
            .map_err(|_| LLMComputeError::Timeout)?;
        if let Ok(resp) = &result {
            let model = resp
                .extra
                .as_ref()
                .and_then(|e| e.get("model"))
                .and_then(Value::as_str)
                .map(str::to_string);
            if model.is_some() {
                *self.last_model.lock().expect("lock") = model;
            }
        }
        result
    }
}

// =========================================================================
// Run 记录（F05 / F06）
// =========================================================================

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct InputRecord {
    /// 本次任务要求（渲染后）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request: Option<String>,
    /// `explicit` / `group_default` / `stdin` / `structured`。
    pub request_source: String,
    #[serde(default)]
    pub attachments: Vec<AttachmentRecord>,
    /// stdin 的角色：`material`（补充材料）或 `request`（作为任务要求）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin_role: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin_chars: Option<usize>,
    /// 结构化输入的消息数。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_messages: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileModelStageRecord {
    pub model: String,
    pub analysis: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    pub completed_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunErrorRecord {
    /// `file_model` / `model` / `tool` / `storage` / `output`。
    pub phase: String,
    pub kind: String,
    pub message: String,
    pub recoverable: bool,
    /// 恢复条件说明（等待服务恢复 / 修复凭据 / 修复存储）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    pub at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunResultRecord {
    /// 模型最终完成响应的原文。
    pub raw: String,
    /// 按保存的 result_format 提取的结果。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extracted: Option<ExtractedValue>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_error: Option<String>,
    /// `--json` 校验结果（未要求时为 None）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_valid: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub main: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model: Option<AiUsage>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction: Option<AiUsage>,
    #[serde(default)]
    pub llm_requests: u64,
}

fn add_usage(base: &mut Option<AiUsage>, extra: &AiUsage) {
    let mut cur = base.clone().unwrap_or_default();
    fn add(a: &mut Option<u64>, b: Option<u64>) {
        if let Some(b) = b {
            *a = Some(a.unwrap_or(0) + b);
        }
    }
    add(&mut cur.input_tokens, extra.input_tokens);
    add(&mut cur.output_tokens, extra.output_tokens);
    add(&mut cur.total_tokens, extra.total_tokens);
    add(&mut cur.request_units, extra.request_units);
    *base = Some(cur);
}

impl UsageRecord {
    pub fn total(&self) -> Option<AiUsage> {
        let mut t = None;
        for u in [&self.main, &self.file_model, &self.compaction]
            .into_iter()
            .flatten()
        {
            add_usage(&mut t, u);
        }
        t
    }
}

/// `run.json`。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunRecord {
    pub version: u32,
    pub run_id: String,
    pub status: RunStatus,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_dir: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// 任务概要（任务要求首行）。
    pub summary: String,
    pub input: InputRecord,
    pub config: EffectiveConfig,
    pub prompt: PromptPlan,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model_stage: Option<FileModelStageRecord>,
    /// 首次快照前的输入；快照提交后清空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_input: Option<PendingInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_snapshot_idx: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<RunErrorRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<RunResultRecord>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default)]
    pub usage: UsageRecord,
    /// 达到限制时的说明。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_reason: Option<String>,
    /// 中断原因。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt_reason: Option<String>,
    /// 本次上下文整理次数。
    #[serde(default)]
    pub compactions: u32,
    #[serde(default)]
    pub pid: u32,
}

impl RunRecord {
    /// 构造一条最小记录（供测试与工具适配层使用；不写盘）。
    pub fn synthetic(run_id: &str, workdir: &Path, status: RunStatus) -> Self {
        let now = now_ms();
        let tools = EffectiveTools::default();
        let runtime_protocol = build_runtime_protocol(LoopModel::FunctionCall, &tools, false);
        Self {
            version: RUN_RECORD_VERSION,
            run_id: run_id.to_string(),
            status,
            workdir: workdir.display().to_string(),
            runs_dir: None,
            created_at_ms: now,
            updated_at_ms: now,
            summary: String::new(),
            input: InputRecord {
                request: None,
                request_source: "structured".into(),
                ..Default::default()
            },
            config: EffectiveConfig {
                provider: ProviderConfig::default(),
                model: "llm.chat".into(),
                file_model: None,
                loop_model: LoopModel::FunctionCall,
                limits: RunLimits {
                    max_tokens: None,
                    max_rounds: DEFAULT_MAX_ROUNDS,
                    timeout_secs: DEFAULT_TIMEOUT_SECS,
                    llm_timeout_secs: DEFAULT_LLM_TIMEOUT_SECS,
                },
                run_logs: RunLogLevel::Info,
                result_format: ResultFormat::Raw,
                json: false,
                json_schema: None,
                disable_capabilities: Vec::new(),
                tools,
                config_files: Vec::new(),
                sources: BTreeMap::new(),
            },
            prompt: PromptPlan {
                mode: PromptMode::Custom,
                group: None,
                loop_model: LoopModel::FunctionCall,
                sections: Vec::new(),
                custom_system: None,
                custom_system_source: None,
                runtime_protocol: runtime_protocol.clone(),
                protocol_version: RUNTIME_PROTOCOL_VERSION.into(),
                system_prompt: runtime_protocol,
                user_request: None,
                user_request_source: "structured".into(),
                template_vars: BTreeMap::new(),
                runtime_vars: BTreeMap::new(),
            },
            file_model_stage: None,
            pending_input: None,
            latest_snapshot_idx: None,
            last_error: None,
            result: None,
            artifacts: Vec::new(),
            usage: UsageRecord::default(),
            limit_reason: None,
            interrupt_reason: None,
            compactions: 0,
            pid: std::process::id(),
        }
    }

    /// 是否可恢复：非终态且存在快照或尚未开始主模型阶段。
    pub fn resumable(&self) -> bool {
        !self.status.is_terminal()
    }

    pub fn resume_command(&self) -> String {
        format!("xllm --resume --run {}", self.run_id)
    }
}

/// 列表条目。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub summary: String,
    pub workdir: String,
    pub status: RunStatus,
    pub status_label: String,
    pub is_terminal: bool,
    pub resumable: bool,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    /// 记录写着执行中但进程已消失。
    #[serde(default)]
    pub stale_running: bool,
}

// -------------------------------------------------------------------------
// 存储后端
// -------------------------------------------------------------------------

/// 内存后端（`runs_dir: none`）。
#[derive(Default)]
pub struct MemoryStore {
    records: HashMap<String, RunRecord>,
    snapshots: HashMap<String, Vec<LLMContextSnapshot>>,
}

/// Runs 目录访问。`Memory` 用于 `runs_dir: none`。
#[derive(Clone)]
pub enum RunStore {
    Disk { runs_dir: PathBuf },
    Memory(Arc<Mutex<MemoryStore>>),
}

impl std::fmt::Debug for RunStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunStore::Disk { runs_dir } => write!(f, "RunStore::Disk({})", runs_dir.display()),
            RunStore::Memory(_) => write!(f, "RunStore::Memory"),
        }
    }
}

impl RunStore {
    pub fn disk(runs_dir: impl Into<PathBuf>) -> Self {
        RunStore::Disk {
            runs_dir: runs_dir.into(),
        }
    }

    pub fn memory() -> Self {
        RunStore::Memory(Arc::new(Mutex::new(MemoryStore::default())))
    }

    pub fn runs_dir(&self) -> Option<&Path> {
        match self {
            RunStore::Disk { runs_dir } => Some(runs_dir),
            RunStore::Memory(_) => None,
        }
    }

    pub fn is_persistent(&self) -> bool {
        matches!(self, RunStore::Disk { .. })
    }

    pub fn run_dir(&self, run_id: &str) -> Option<PathBuf> {
        self.runs_dir().map(|d| d.join(run_id))
    }

    /// 确保 Runs 目录可写（F05：执行前检查）。
    pub fn ensure_writable(&self) -> Result<(), XllmError> {
        if let RunStore::Disk { runs_dir } = self {
            std::fs::create_dir_all(runs_dir).map_err(|e| {
                XllmError::Storage(format!(
                    "runs directory {} cannot be created: {e}",
                    runs_dir.display()
                ))
            })?;
            let probe = runs_dir.join(format!(".probe-{}", std::process::id()));
            std::fs::write(&probe, b"").map_err(|e| {
                XllmError::Storage(format!(
                    "runs directory {} is not writable: {e}",
                    runs_dir.display()
                ))
            })?;
            let _ = std::fs::remove_file(&probe);
        }
        Ok(())
    }

    /// 创建新的 Run 目录并返回 run_id（目录已存在时换编号重试）。
    fn create_run(&self) -> Result<String, XllmError> {
        for _ in 0..8 {
            let run_id = generate_run_id();
            match self {
                RunStore::Disk { runs_dir } => {
                    let dir = runs_dir.join(&run_id);
                    match std::fs::create_dir(&dir) {
                        Ok(()) => {
                            std::fs::create_dir_all(dir.join("snapshots"))?;
                            return Ok(run_id);
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                        Err(e) => {
                            return Err(XllmError::Storage(format!(
                                "cannot create run directory {}: {e}",
                                dir.display()
                            )))
                        }
                    }
                }
                RunStore::Memory(m) => {
                    let guard = m.lock().expect("memory store lock");
                    if guard.records.contains_key(&run_id) {
                        continue;
                    }
                    return Ok(run_id);
                }
            }
        }
        Err(XllmError::Storage("cannot allocate a unique run id".into()))
    }

    pub fn write_record(&self, rec: &RunRecord) -> Result<(), XllmError> {
        match self {
            RunStore::Disk { runs_dir } => {
                let dir = runs_dir.join(&rec.run_id);
                std::fs::create_dir_all(&dir)?;
                let path = dir.join("run.json");
                let tmp = dir.join(format!("run.json.tmp-{}", std::process::id()));
                let bytes = serde_json::to_vec_pretty(rec)
                    .map_err(|e| XllmError::Storage(format!("serialize run.json: {e}")))?;
                std::fs::write(&tmp, &bytes)
                    .map_err(|e| XllmError::Storage(format!("write {}: {e}", tmp.display())))?;
                std::fs::rename(&tmp, &path)
                    .map_err(|e| XllmError::Storage(format!("commit {}: {e}", path.display())))?;
                Ok(())
            }
            RunStore::Memory(m) => {
                m.lock()
                    .expect("memory store lock")
                    .records
                    .insert(rec.run_id.clone(), rec.clone());
                Ok(())
            }
        }
    }

    pub fn read_record(&self, run_id: &str) -> Result<RunRecord, XllmError> {
        match self {
            RunStore::Disk { runs_dir } => {
                let path = runs_dir.join(run_id).join("run.json");
                let bytes = match std::fs::read(&path) {
                    Ok(b) => b,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        return Err(XllmError::RunNotFound {
                            run_id: run_id.to_string(),
                            runs_dir: runs_dir.display().to_string(),
                        })
                    }
                    Err(e) => {
                        return Err(XllmError::Storage(format!("read {}: {e}", path.display())))
                    }
                };
                serde_json::from_slice::<RunRecord>(&bytes).map_err(|e| XllmError::CorruptedRun {
                    run_id: run_id.to_string(),
                    reason: format!("run.json unreadable: {e}"),
                })
            }
            RunStore::Memory(m) => m
                .lock()
                .expect("memory store lock")
                .records
                .get(run_id)
                .cloned()
                .ok_or_else(|| XllmError::RunNotFound {
                    run_id: run_id.to_string(),
                    runs_dir: "(memory)".into(),
                }),
        }
    }

    pub fn list_records(&self) -> Result<Vec<RunRecord>, XllmError> {
        match self {
            RunStore::Disk { runs_dir } => {
                let mut out = Vec::new();
                let entries = match std::fs::read_dir(runs_dir) {
                    Ok(e) => e,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
                    Err(e) => {
                        return Err(XllmError::Storage(format!(
                            "read runs directory {}: {e}",
                            runs_dir.display()
                        )))
                    }
                };
                for entry in entries.flatten() {
                    let path = entry.path().join("run.json");
                    if !path.is_file() {
                        continue;
                    }
                    if let Ok(bytes) = std::fs::read(&path) {
                        if let Ok(rec) = serde_json::from_slice::<RunRecord>(&bytes) {
                            out.push(rec);
                        }
                    }
                }
                Ok(out)
            }
            RunStore::Memory(m) => Ok(m
                .lock()
                .expect("memory store lock")
                .records
                .values()
                .cloned()
                .collect()),
        }
    }

    pub fn put_snapshot(&self, run_id: &str, snap: &LLMContextSnapshot) -> Result<u32, XllmError> {
        match self {
            RunStore::Disk { runs_dir } => {
                let dir = runs_dir.join(run_id).join("snapshots");
                std::fs::create_dir_all(&dir)?;
                let next = self
                    .list_snapshots(run_id)?
                    .last()
                    .copied()
                    .map_or(1, |i| i + 1);
                let path = dir.join(format!("{next:04}.json"));
                let tmp = dir.join(format!("{next:04}.json.tmp"));
                let bytes = serde_json::to_vec(snap)
                    .map_err(|e| XllmError::Storage(format!("serialize snapshot: {e}")))?;
                std::fs::write(&tmp, &bytes)?;
                std::fs::rename(&tmp, &path)?;
                Ok(next)
            }
            RunStore::Memory(m) => {
                let mut g = m.lock().expect("memory store lock");
                let v = g.snapshots.entry(run_id.to_string()).or_default();
                v.push(snap.clone());
                Ok(v.len() as u32)
            }
        }
    }

    pub fn get_snapshot(&self, run_id: &str, idx: u32) -> Result<LLMContextSnapshot, XllmError> {
        match self {
            RunStore::Disk { runs_dir } => {
                let path = runs_dir
                    .join(run_id)
                    .join("snapshots")
                    .join(format!("{idx:04}.json"));
                let bytes = std::fs::read(&path).map_err(|e| XllmError::CorruptedRun {
                    run_id: run_id.to_string(),
                    reason: format!("snapshot {idx} missing at {}: {e}", path.display()),
                })?;
                serde_json::from_slice(&bytes).map_err(|e| XllmError::CorruptedRun {
                    run_id: run_id.to_string(),
                    reason: format!("snapshot {idx} unreadable: {e}"),
                })
            }
            RunStore::Memory(m) => m
                .lock()
                .expect("memory store lock")
                .snapshots
                .get(run_id)
                .and_then(|v| v.get((idx as usize).saturating_sub(1)))
                .cloned()
                .ok_or_else(|| XllmError::CorruptedRun {
                    run_id: run_id.to_string(),
                    reason: format!("snapshot {idx} missing"),
                }),
        }
    }

    pub fn list_snapshots(&self, run_id: &str) -> Result<Vec<u32>, XllmError> {
        match self {
            RunStore::Disk { runs_dir } => {
                let dir = runs_dir.join(run_id).join("snapshots");
                if !dir.exists() {
                    return Ok(Vec::new());
                }
                let mut idxs: Vec<u32> = std::fs::read_dir(&dir)?
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let name = e.file_name().into_string().ok()?;
                        name.strip_suffix(".json")?.parse::<u32>().ok()
                    })
                    .collect();
                idxs.sort();
                Ok(idxs)
            }
            RunStore::Memory(m) => Ok((1..=m
                .lock()
                .expect("memory store lock")
                .snapshots
                .get(run_id)
                .map(|v| v.len())
                .unwrap_or(0) as u32)
                .collect()),
        }
    }

    /// 该 Run 是否正被某个进程执行（持有 `.lock`）。内存模式恒为 false。
    pub fn is_run_live(&self, run_id: &str) -> bool {
        let Some(dir) = self.run_dir(run_id) else {
            return false;
        };
        let lock_path = dir.join(".lock");
        let Ok(file) = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)
        else {
            return false;
        };
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                let _ = FileExt::unlock(&file);
                false
            }
            Err(_) => true,
        }
    }

    pub fn summarize(&self, rec: &RunRecord) -> RunSummary {
        let stale = rec.status == RunStatus::Running && !self.is_run_live(&rec.run_id);
        let status = if stale {
            RunStatus::Interrupted
        } else {
            rec.status
        };
        RunSummary {
            run_id: rec.run_id.clone(),
            summary: rec.summary.clone(),
            workdir: rec.workdir.clone(),
            status,
            status_label: status.label().to_string(),
            is_terminal: status.is_terminal(),
            resumable: !status.is_terminal() && status != RunStatus::Running,
            created_at_ms: rec.created_at_ms,
            updated_at_ms: rec.updated_at_ms,
            stale_running: stale,
        }
    }
}

/// 进程级 flock 句柄；drop 即释放。
pub struct FileLock {
    _file: File,
    pub path: PathBuf,
}

impl FileLock {
    /// 非阻塞获取；被占用返回 `Ok(None)`。
    pub fn try_acquire(path: &Path) -> Result<Option<FileLock>, XllmError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                XllmError::Storage(format!(
                    "cannot create lock directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|e| XllmError::Storage(format!("cannot open lock {}: {e}", path.display())))?;
        match FileExt::try_lock_exclusive(&file) {
            Ok(()) => {
                let _ = std::fs::write(path, format!("{}\n", std::process::id()));
                Ok(Some(FileLock {
                    _file: file,
                    path: path.to_path_buf(),
                }))
            }
            Err(_) => Ok(None),
        }
    }
}

fn workdir_lock_path(lock_dir: &Path, workdir: &Path) -> PathBuf {
    let key = blake3::hash(workdir.display().to_string().as_bytes()).to_hex();
    lock_dir.join(format!("{}.lock", &key.as_str()[..24]))
}

fn read_lock_owner(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

static RUN_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// `YYYYMMDD-HHMMSS-<6 hex>`：本地时间便于阅读，随机后缀防冲突。
pub fn generate_run_id() -> String {
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let counter = RUN_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let seed = format!("{nanos}:{}:{counter}", std::process::id());
    let hash = blake3::hash(seed.as_bytes()).to_hex();
    format!("{ts}-{}", &hash.as_str()[..6])
}

fn summarize_request(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("");
    truncate_chars(first, 80)
}

// =========================================================================
// 结果提取（F07）
// =========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExtractedValue {
    Text { text: String },
    Json { value: Value },
    Xml { xml: String },
}

impl ExtractedValue {
    /// 文本输出形态：字符串按正文输出，JSON 结构序列化，XML 保留结构。
    pub fn to_output_text(&self) -> String {
        match self {
            ExtractedValue::Text { text } => text.clone(),
            ExtractedValue::Json { value } => {
                serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
            }
            ExtractedValue::Xml { xml } => xml.clone(),
        }
    }

    /// `--json` 校验：必须是合法 JSON。
    pub fn as_json(&self) -> Result<Value, String> {
        match self {
            ExtractedValue::Json { value } => Ok(value.clone()),
            ExtractedValue::Text { text } => {
                let stripped = strip_code_fences(text);
                serde_json::from_str::<Value>(stripped.trim())
                    .map_err(|e| format!("result is not valid JSON: {e}"))
            }
            ExtractedValue::Xml { .. } => Err("result is XML, not JSON".to_string()),
        }
    }

    pub fn kind(&self) -> &'static str {
        match self {
            ExtractedValue::Text { .. } => "text",
            ExtractedValue::Json { .. } => "json",
            ExtractedValue::Xml { .. } => "xml",
        }
    }
}

/// 对最终完成响应应用 `result_format`。失败时明确报错，不回退 raw。
pub fn extract_result(raw: &str, fmt: &ResultFormat) -> Result<ExtractedValue, XllmError> {
    let segments = match fmt {
        ResultFormat::Raw => {
            return Ok(ExtractedValue::Text {
                text: raw.to_string(),
            })
        }
        ResultFormat::Path { segments } => segments,
    };
    let body = strip_code_fences(raw);
    let trimmed = body.trim();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        let mut cur: Value = serde_json::from_str(trimmed).map_err(|e| {
            XllmError::Extract(format!(
                "final response looks like JSON but does not parse: {e}"
            ))
        })?;
        for (i, seg) in segments.iter().enumerate() {
            let path_so_far = segments[..=i].join(".");
            cur = match cur {
                Value::Object(mut m) => m.remove(seg).ok_or_else(|| {
                    XllmError::Extract(format!(
                        "JSON field `result.{path_so_far}` is missing (available: {})",
                        m.keys().cloned().collect::<Vec<_>>().join(", ")
                    ))
                })?,
                Value::Array(mut a) => {
                    let idx: usize = seg.parse().map_err(|_| {
                        XllmError::Extract(format!(
                            "`result.{path_so_far}`: parent is an array, segment `{seg}` is not an index"
                        ))
                    })?;
                    if idx >= a.len() {
                        return Err(XllmError::Extract(format!(
                            "`result.{path_so_far}`: index {idx} out of range ({} items)",
                            a.len()
                        )));
                    }
                    a.swap_remove(idx)
                }
                other => {
                    return Err(XllmError::Extract(format!(
                        "`result.{path_so_far}`: cannot descend into a {} value",
                        match other {
                            Value::Null => "null",
                            Value::Bool(_) => "boolean",
                            Value::Number(_) => "number",
                            Value::String(_) => "string",
                            _ => "scalar",
                        }
                    )))
                }
            };
        }
        return Ok(match cur {
            Value::String(text) => ExtractedValue::Text { text },
            value => ExtractedValue::Json { value },
        });
    }
    if trimmed.starts_with('<') {
        let roots = scan_xml_elements(trimmed);
        let root = roots.into_iter().next().ok_or_else(|| {
            XllmError::Extract("final response looks like XML but has no root element".to_string())
        })?;
        let mut cur = root;
        for (i, seg) in segments.iter().enumerate() {
            let path_so_far = segments[..=i].join(".");
            let matches: Vec<XmlElement> = cur
                .children()
                .into_iter()
                .filter(|c| &c.name == seg)
                .collect();
            match matches.len() {
                0 => {
                    return Err(XllmError::Extract(format!(
                        "XML element `result.{path_so_far}` not found under <{}>",
                        cur.name
                    )))
                }
                1 => cur = matches.into_iter().next().unwrap(),
                n => {
                    return Err(XllmError::Extract(format!(
                    "XML path `result.{path_so_far}` matches {n} elements; the path must be unique"
                )))
                }
            }
        }
        return Ok(if cur.children().is_empty() {
            ExtractedValue::Text { text: cur.text() }
        } else {
            ExtractedValue::Xml {
                xml: cur.body.trim().to_string(),
            }
        });
    }
    Err(XllmError::Extract(format!(
        "final response is neither JSON nor XML, cannot apply `{}` (use raw or adjust output_format)",
        fmt.as_string()
    )))
}

/// 从已保存的最终原文重新导出（`xllm result`）。
pub fn export_result(
    record: &RunRecord,
    fmt: Option<&ResultFormat>,
) -> Result<ExtractedValue, XllmError> {
    let result = record.result.as_ref().ok_or_else(|| {
        XllmError::Other(format!(
            "run `{}` has no final response yet (status: {})",
            record.run_id,
            record.status.label()
        ))
    })?;
    let fmt = fmt.unwrap_or(&record.config.result_format);
    extract_result(&result.raw, fmt)
}

// =========================================================================
// 任务准备（F02 / F03 / F04 / F10 / F11 汇合点）
// =========================================================================

/// 文件模型阶段计划。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileStagePlan {
    pub model: String,
    pub request_text: String,
    pub images: Vec<(String, ResourceRef)>,
}

/// 首次快照之前需要保留的输入（含附件正文），使文件模型阶段失败后也能 resume。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingInput {
    pub initial_messages: Vec<AiMessage>,
    pub main_user_parts: Vec<AiContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_stage: Option<FileStagePlan>,
}

/// 预检完成、尚未建立 Run 的任务。
pub struct PreparedTask {
    pub workdir: PathBuf,
    pub store: RunStore,
    pub config: EffectiveConfig,
    pub merged: MergedConfig,
    pub prompt: PromptPlan,
    pub input: InputRecord,
    pub summary: String,
    /// 主模型的初始消息（不含文件模型分析；有文件阶段时在执行时重组）。
    pub initial_messages: Vec<AiMessage>,
    /// 文件模型阶段（有图片且配置了 file_model 时）。
    pub file_stage: Option<FileStagePlan>,
    /// 有文件阶段时，主模型 user 消息的文本部分（分析结果插入其后）。
    pub main_user_parts: Vec<AiContent>,
    pub tools: EffectiveTools,
    manager: XllmToolManager,
}

impl std::fmt::Debug for PreparedTask {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTask")
            .field("workdir", &self.workdir)
            .field("model", &self.config.model)
            .field("loop_model", &self.config.loop_model)
            .field("summary", &self.summary)
            .finish()
    }
}

/// 任务入口。
pub struct XllmTask;

impl XllmTask {
    /// 合并配置、选组、展开工具、读取材料、组装提示词并校验；不建立 Run。
    pub async fn prepare(
        workdir: &Path,
        input: TaskInput,
        overrides: TaskOverrides,
        deps: &XllmDeps,
    ) -> Result<PreparedTask, XllmError> {
        let workdir = workdir.canonicalize().map_err(|e| {
            XllmError::Input(format!(
                "working directory {} is not accessible: {e}",
                workdir.display()
            ))
        })?;
        if !workdir.is_dir() {
            return Err(XllmError::Input(format!(
                "working directory {} is not a directory",
                workdir.display()
            )));
        }
        let base_dir = input
            .base_dir
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| workdir.clone()));

        // ---- 输入互斥 ----
        if overrides.select.is_some() && overrides.system.is_some() {
            return Err(XllmError::Input(
                "--select and --system are mutually exclusive (custom prompt vs. standard group assembly)".into(),
            ));
        }
        if input.structured.is_some()
            && (input.user.is_some()
                || !input.attachments.is_empty()
                || input.stdin.is_some()
                || overrides.select.is_some()
                || overrides.system.is_some())
        {
            return Err(XllmError::Input(
                "--input-file is mutually exclusive with question/--user/--system/--select/--file/--image/stdin".into(),
            ));
        }
        if let Some(s) = &input.stdin {
            if s.trim().is_empty() {
                return Err(XllmError::Input(
                    "stdin was connected but contained no usable input; check the upstream command and its exit status".into(),
                ));
            }
        }

        // ---- 配置合并 ----
        let layers = load_config_layers(&workdir)?;
        let merged = merge_config_layers(&layers)?;
        let mut sources = merged.sources.clone();

        let mut provider = merged.provider.clone();
        if let Some(k) = overrides.provider {
            if provider.kind != Some(k) {
                provider = ProviderConfig {
                    kind: Some(k),
                    ..Default::default()
                };
            }
            sources.insert("provider.type".into(), "cli".into());
        }
        let provider_kind = provider.effective_kind();
        let model = match (&overrides.model, &merged.model) {
            (Some(m), _) => {
                sources.insert("model".into(), "cli".into());
                m.clone()
            }
            (None, Some(m)) => m.clone(),
            (None, None) => match provider_kind {
                ProviderKind::Buckyos => {
                    sources.insert("model".into(), "default".into());
                    "llm.chat".to_string()
                }
                ProviderKind::Openai => return Err(XllmError::Capability(
                    "openai provider requires a model: set `model` in .llm_context or pass --model"
                        .into(),
                )),
            },
        };
        let file_model = match (&overrides.file_model, &merged.file_model) {
            (Some(m), _) => {
                sources.insert("file_model".into(), "cli".into());
                Some(m.clone())
            }
            (None, m) => m.clone(),
        };
        let loop_model = match (overrides.loop_model, merged.loop_model) {
            (Some(l), _) => {
                sources.insert("loop_model".into(), "cli".into());
                l
            }
            (None, Some(l)) => l,
            (None, None) => {
                sources.insert("loop_model".into(), "default".into());
                LoopModel::FunctionCall
            }
        };
        let result_format = match (&overrides.result_format, &merged.result_format) {
            (Some(f), _) => {
                sources.insert("result_format".into(), "cli".into());
                f.clone()
            }
            (None, Some(f)) => f.clone(),
            (None, None) => {
                sources.insert("result_format".into(), "default".into());
                ResultFormat::Raw
            }
        };
        let run_logs = match (overrides.run_logs, merged.run_logs) {
            (Some(l), _) => {
                sources.insert("run_logs".into(), "cli".into());
                l
            }
            (None, Some(l)) => l,
            (None, None) => RunLogLevel::Info,
        };
        macro_rules! pick_limit {
            ($cli:expr, $file:expr, $default:expr, $name:expr) => {
                match ($cli, $file) {
                    (Some(v), _) => {
                        sources.insert($name.into(), "cli".into());
                        v
                    }
                    (None, Some(v)) => v,
                    (None, None) => {
                        sources.insert($name.into(), "default".into());
                        $default
                    }
                }
            };
        }
        let limits = RunLimits {
            max_tokens: match (overrides.max_tokens, merged.max_tokens) {
                (Some(v), _) => {
                    sources.insert("max_tokens".into(), "cli".into());
                    Some(v)
                }
                (None, v) => v,
            },
            max_rounds: pick_limit!(
                overrides.max_rounds,
                merged.max_rounds,
                DEFAULT_MAX_ROUNDS,
                "max_rounds"
            ),
            timeout_secs: pick_limit!(
                overrides.timeout_secs,
                merged.timeout,
                DEFAULT_TIMEOUT_SECS,
                "timeout"
            ),
            llm_timeout_secs: pick_limit!(
                overrides.llm_timeout_secs,
                merged.llm_timeout,
                DEFAULT_LLM_TIMEOUT_SECS,
                "llm_timeout"
            ),
        };
        let store = match (&overrides.runs_dir, &merged.runs_dir) {
            (Some(p), _) => {
                sources.insert("runs_dir".into(), "cli".into());
                RunStore::disk(resolve_config_path(&p.display().to_string(), &base_dir))
            }
            (None, Some(RunsDirSetting::Disabled)) => RunStore::memory(),
            (None, Some(RunsDirSetting::Path { path })) => RunStore::disk(path.clone()),
            (None, None) => {
                sources.insert("runs_dir".into(), "default".into());
                RunStore::disk(resolve_config_path(DEFAULT_RUNS_DIR, &base_dir))
            }
        };
        if overrides.json && loop_model == LoopModel::Behavior && result_format == ResultFormat::Raw
        {
            return Err(XllmError::Capability(
                "--json cannot be satisfied with `loop_model: behavior` and `result_format: raw` (the raw behavior response is XML); use e.g. `result_format: result.report`".into(),
            ));
        }

        // ---- 模式与选组 ----
        let structured = input.structured.is_some();
        let custom = overrides.system.is_some()
            || (overrides.select.is_none() && merged.prompt.mode == Some(PromptMode::Custom));
        let mut group_name: Option<String> = None;
        if !structured && !custom {
            if let Some(sel) = overrides
                .select
                .clone()
                .or_else(|| merged.prompt.select.clone())
            {
                if !merged.prompt.groups.contains_key(&sel) {
                    let available: Vec<String> = merged.prompt.groups.keys().cloned().collect();
                    let src = if overrides.select.is_some() {
                        "cli:--select".to_string()
                    } else {
                        merged
                            .sources
                            .get("prompt.select")
                            .cloned()
                            .unwrap_or_default()
                    };
                    return Err(XllmError::Config {
                        file: src,
                        field: "prompt.select".into(),
                        reason: format!(
                            "prompt group `{sel}` does not exist (available: {})",
                            if available.is_empty() {
                                "none".to_string()
                            } else {
                                available.join(", ")
                            }
                        ),
                    });
                }
                if overrides.select.is_some() {
                    sources.insert("prompt.select".into(), "cli".into());
                }
                group_name = Some(sel);
            }
        }
        let group = group_name
            .as_ref()
            .and_then(|n| merged.prompt.groups.get(n))
            .cloned();

        // ---- 工具 ----
        let (tools_cfg, tool_sources) =
            compute_tools_config(&merged, group.as_ref(), overrides.tools, group.is_some());
        let (tools, manager) = build_toolset(
            &tools_cfg,
            tool_sources,
            loop_model,
            &workdir,
            "pending",
            deps,
        )
        .await?;

        // ---- 提示词 ----
        let env = TemplateEnv::for_new_run(&workdir);
        let prompt = if structured {
            let runtime_protocol = build_runtime_protocol(loop_model, &tools, overrides.json);
            PromptPlan {
                mode: PromptMode::Custom,
                group: None,
                loop_model,
                sections: Vec::new(),
                custom_system: None,
                custom_system_source: Some("structured input".into()),
                runtime_protocol: runtime_protocol.clone(),
                protocol_version: RUNTIME_PROTOCOL_VERSION.into(),
                system_prompt: format!(
                    "## capabilities\n{}\n\n## runtime_protocol\n{}\n",
                    build_rules_system_text(loop_model, &tools),
                    runtime_protocol.trim_end()
                ),
                user_request: None,
                user_request_source: "structured".into(),
                template_vars: BTreeMap::new(),
                runtime_vars: env.runtime.clone(),
            }
        } else {
            assemble_prompt(&PromptAssembly {
                merged: &merged,
                overrides: &overrides,
                group_name: group_name.as_deref(),
                group: group.as_ref(),
                loop_model,
                tools: &tools,
                env: &env,
                json: overrides.json,
            })?
        };

        // ---- 任务要求 ----
        let explicit = input
            .user
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let (request, request_source, stdin_role) = if structured {
            (None, "structured".to_string(), None)
        } else if let Some(u) = explicit {
            (
                Some(u),
                "explicit".to_string(),
                input.stdin.as_ref().map(|_| "material".to_string()),
            )
        } else if let Some(g) = prompt.user_request.clone() {
            (
                Some(g),
                "group_default".to_string(),
                input.stdin.as_ref().map(|_| "material".to_string()),
            )
        } else if let Some(s) = input.stdin.as_ref() {
            (
                Some(s.trim_end().to_string()),
                "stdin".to_string(),
                Some("request".to_string()),
            )
        } else if input.user.is_some() {
            return Err(XllmError::Input("the task request is empty".into()));
        } else if group_name.is_some() {
            return Err(XllmError::Input(format!(
                "prompt group `{}` has no default task; pass a question or configure `default_user`",
                group_name.as_deref().unwrap_or("")
            )));
        } else if !input.attachments.is_empty() {
            return Err(XllmError::Input(
                "attachments were given without a task request; add a question or --user".into(),
            ));
        } else {
            return Err(XllmError::Input(
                "no task request: pass a question, --user, --select <group with default_user>, or pipe input".into(),
            ));
        };

        // ---- 附件 ----
        let (loaded, attachment_records) = load_attachments(&input.attachments, &base_dir)?;
        let has_images = loaded
            .iter()
            .any(|a| matches!(a, LoadedAttachment::Image { .. }));

        // ---- 消息组装 ----
        let mut user_parts: Vec<AiContent> = Vec::new();
        let mut text_material_parts: Vec<AiContent> = Vec::new();
        let mut images: Vec<(String, ResourceRef)> = Vec::new();
        if let Some(r) = &request {
            user_parts.push(AiContent::text(r.clone()));
        }
        let mut image_index = 0usize;
        for att in &loaded {
            match att {
                LoadedAttachment::Text { label, text } => {
                    let part = AiContent::text(format!(
                        "<material name=\"{}\">\n{}\n</material>",
                        xml_escape(label),
                        text
                    ));
                    text_material_parts.push(part);
                }
                LoadedAttachment::Image { label, source } => {
                    image_index += 1;
                    images.push((format!("image {image_index}: {label}"), source.clone()));
                }
            }
        }
        let stdin_part = match (&input.stdin, stdin_role.as_deref()) {
            (Some(s), Some("material")) => Some(AiContent::text(format!(
                "<stdin>\n{}\n</stdin>",
                s.trim_end()
            ))),
            _ => None,
        };

        let file_stage = if has_images {
            file_model.as_ref().map(|fm| FileStagePlan {
                model: fm.clone(),
                request_text: request.clone().unwrap_or_default(),
                images: images.clone(),
            })
        } else {
            None
        };

        // 主模型 user 消息：要求 → 材料（命令顺序；图片在无文件模型时直接进入）→ stdin。
        let mut main_user_parts = user_parts.clone();
        let mut ordered_parts: Vec<AiContent> = Vec::new();
        let mut img_iter = images.iter();
        for att in &loaded {
            match att {
                LoadedAttachment::Text { .. } => {
                    if let Some(p) = text_material_parts.first().cloned() {
                        text_material_parts.remove(0);
                        ordered_parts.push(p);
                    }
                }
                LoadedAttachment::Image { .. } => {
                    if let Some((label, src)) = img_iter.next() {
                        if file_stage.is_none() {
                            ordered_parts.push(AiContent::text(format!("[{label}]")));
                            ordered_parts.push(AiContent::image(src.clone()));
                        }
                    }
                }
            }
        }
        main_user_parts.extend(ordered_parts);
        if let Some(p) = stdin_part {
            main_user_parts.push(p);
        }

        let initial_messages: Vec<AiMessage> = if let Some(msgs) = input.structured.clone() {
            let mut out = vec![AiMessage::text(
                AiRole::System,
                prompt.system_prompt.clone(),
            )];
            out.extend(msgs);
            out
        } else {
            vec![
                AiMessage::text(AiRole::System, prompt.system_prompt.clone()),
                AiMessage::new(AiRole::User, main_user_parts.clone()),
            ]
        };

        let summary = request
            .as_deref()
            .map(summarize_request)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "(structured input)".to_string());

        let mut prompt = prompt;
        prompt.user_request = request.clone();
        prompt.user_request_source = request_source.clone();

        let config = EffectiveConfig {
            provider,
            model,
            file_model,
            loop_model,
            limits,
            run_logs,
            result_format,
            json: overrides.json,
            json_schema: overrides.json_schema.clone(),
            disable_capabilities: overrides.disable_capabilities.clone(),
            tools,
            config_files: merged.files.clone(),
            sources,
        };
        let input_record = InputRecord {
            request: request.clone(),
            request_source,
            attachments: attachment_records,
            stdin_role,
            stdin_chars: input.stdin.as_ref().map(|s| s.chars().count()),
            structured_messages: input.structured.as_ref().map(|m| m.len()),
        };
        let tools = config.tools.clone();
        Ok(PreparedTask {
            workdir,
            store,
            config,
            merged,
            prompt,
            input: input_record,
            summary,
            initial_messages,
            file_stage,
            main_user_parts,
            tools,
            manager,
        })
    }
}

// =========================================================================
// 执行（F05 / F08 / F09）
// =========================================================================

/// 一次 `execute()` 的结果。Run 记录里保存了完整状态。
#[derive(Debug, Clone)]
pub enum RunOutcome {
    Completed(RunRecord),
    Paused(RunRecord),
    Interrupted(RunRecord),
    Failed(RunRecord),
    LimitReached(RunRecord),
}

impl RunOutcome {
    pub fn record(&self) -> &RunRecord {
        match self {
            RunOutcome::Completed(r)
            | RunOutcome::Paused(r)
            | RunOutcome::Interrupted(r)
            | RunOutcome::Failed(r)
            | RunOutcome::LimitReached(r) => r,
        }
    }

    pub fn status(&self) -> RunStatus {
        self.record().status
    }

    fn from_record(r: RunRecord) -> Self {
        match r.status {
            RunStatus::Completed => RunOutcome::Completed(r),
            RunStatus::Paused => RunOutcome::Paused(r),
            RunStatus::Interrupted | RunStatus::Running => RunOutcome::Interrupted(r),
            RunStatus::Failed => RunOutcome::Failed(r),
            RunStatus::LimitReached => RunOutcome::LimitReached(r),
        }
    }
}

/// resume 时允许显式调整的执行限制（只对非终态任务生效）。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResumeLimits {
    pub max_tokens: Option<u32>,
    pub max_rounds: Option<u32>,
    pub timeout_secs: Option<u64>,
    pub llm_timeout_secs: Option<u64>,
}

impl ResumeLimits {
    pub fn is_empty(&self) -> bool {
        *self == ResumeLimits::default()
    }
}

/// `resume` 的两种结果：终态只展示结果；非终态得到可执行的 Run。
pub enum ResumeStart {
    Terminal(RunRecord),
    Run(Box<XllmRun>),
}

impl std::fmt::Debug for ResumeStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResumeStart::Terminal(r) => {
                write!(f, "ResumeStart::Terminal({}, {})", r.run_id, r.status)
            }
            ResumeStart::Run(r) => write!(f, "ResumeStart::Run({})", r.run_id),
        }
    }
}

impl std::fmt::Debug for XllmRun {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XllmRun")
            .field("run_id", &self.run_id)
            .field("store", &self.store)
            .field("status", &self.record().status)
            .finish()
    }
}

/// 外部中断句柄（Ctrl-C）。
#[derive(Clone)]
pub struct XllmInterrupter {
    requested: Arc<Mutex<Option<String>>>,
    handle: Arc<Mutex<Option<LLMContextInterruptHandle>>>,
    tool_cancel: Arc<tokio::sync::watch::Sender<bool>>,
}

impl XllmInterrupter {
    pub fn interrupt(&self, reason: impl Into<String>) {
        let reason = reason.into();
        *self.requested.lock().expect("lock") = Some(reason.clone());
        if let Some(h) = self.handle.lock().expect("lock").as_ref() {
            h.interrupt(reason);
        }
        self.tool_cancel.send_replace(true);
    }

    fn requested(&self) -> Option<String> {
        self.requested.lock().expect("lock").clone()
    }
}

struct SnapshotHook {
    store: RunStore,
    record: Arc<Mutex<RunRecord>>,
}

impl SnapshotHook {
    fn commit(&self, snapshot: &LLMContextSnapshot) -> Result<u32, XllmError> {
        let run_id = self.record.lock().expect("record lock").run_id.clone();
        let idx = self.store.put_snapshot(&run_id, snapshot)?;
        let mut rec = self.record.lock().expect("record lock");
        rec.latest_snapshot_idx = Some(idx);
        rec.updated_at_ms = now_ms();
        self.store.write_record(&rec)?;
        Ok(idx)
    }
}

impl TurnHook for SnapshotHook {
    fn before_inference(&self, snapshot: &LLMContextSnapshot) -> Result<(), String> {
        self.commit(snapshot).map(|_| ()).map_err(|e| e.to_string())
    }
}

/// 一次 Run 的执行句柄。
pub struct XllmRun {
    run_id: String,
    store: RunStore,
    record: Arc<Mutex<RunRecord>>,
    deps: XllmDeps,
    waist_deps: LLMContextDeps,
    llm: Arc<TimeoutLlmClient>,
    manager: Arc<XllmToolManager>,
    ctx: Option<LLMContext>,
    interrupter: XllmInterrupter,
    file_stage: Option<FileStagePlan>,
    initial_messages: Vec<AiMessage>,
    main_user_parts: Vec<AiContent>,
    _run_lock: Option<FileLock>,
    _workdir_lock: Option<FileLock>,
}

impl XllmRun {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn store(&self) -> &RunStore {
        &self.store
    }

    pub fn record(&self) -> RunRecord {
        self.record.lock().expect("record lock").clone()
    }

    pub fn interrupter(&self) -> XllmInterrupter {
        self.interrupter.clone()
    }

    fn update<F: FnOnce(&mut RunRecord)>(&self, f: F) -> Result<(), XllmError> {
        let mut rec = self.record.lock().expect("record lock");
        f(&mut rec);
        rec.updated_at_ms = now_ms();
        rec.artifacts = self.manager.artifacts();
        self.store.write_record(&rec)
    }

    fn emit(&self, event: RunEvent) {
        self.deps.observer.on_event(&self.run_id, event);
    }

    async fn acquire_locks(
        store: &RunStore,
        run_id: &str,
        workdir: &Path,
        tools_enabled: bool,
        deps: &XllmDeps,
    ) -> Result<(Option<FileLock>, Option<FileLock>), XllmError> {
        let run_lock = match store.run_dir(run_id) {
            Some(dir) => match FileLock::try_acquire(&dir.join(".lock"))? {
                Some(l) => Some(l),
                None => {
                    return Err(XllmError::RunBusy {
                        run_id: run_id.to_string(),
                    })
                }
            },
            None => None,
        };
        let workdir_lock = if tools_enabled {
            let path = workdir_lock_path(&deps.effective_lock_dir(), workdir);
            match FileLock::try_acquire(&path)? {
                Some(l) => {
                    let _ = std::fs::write(&l.path, format!("{run_id}\n"));
                    Some(l)
                }
                None => {
                    return Err(XllmError::WorkdirBusy {
                        workdir: workdir.display().to_string(),
                        run_id: read_lock_owner(&path).unwrap_or_else(|| "unknown".into()),
                    })
                }
            }
        } else {
            None
        };
        Ok((run_lock, workdir_lock))
    }

    fn build_waist_deps(
        store: &RunStore,
        record: &Arc<Mutex<RunRecord>>,
        run_id: &str,
        llm: Arc<TimeoutLlmClient>,
        manager: Arc<XllmToolManager>,
        loop_model: LoopModel,
        tools: &EffectiveTools,
        deps: &XllmDeps,
    ) -> LLMContextDeps {
        let hook: Arc<dyn TurnHook> = Arc::new(SnapshotHook {
            store: store.clone(),
            record: record.clone(),
        });
        let worklog: Arc<dyn WorklogSink> = Arc::new(ObserverWorklog {
            run_id: run_id.to_string(),
            observer: deps.observer.clone(),
            llm_started_at: Mutex::new(None),
            tool_commands: Mutex::new(HashMap::new()),
        });
        let tools_dyn: Arc<dyn ToolManager> = manager;
        let llm_dyn: Arc<dyn LlmClient> = llm;
        let mut d = LLMContextDeps::new(llm_dyn, tools_dyn)
            .with_worklog(worklog)
            .with_turn_hook(hook);
        if loop_model == LoopModel::Behavior {
            d = d
                .with_result_parser(Arc::new(XllmActionParser::new(&tools.actions)))
                .with_step_renderer(Arc::new(XmlStepRenderer::new()));
        }
        d
    }

    /// 建立 Run：分配编号、加锁、创建 Provider 客户端、在首次模型请求前保存记录。
    pub async fn start(prepared: PreparedTask, deps: XllmDeps) -> Result<XllmRun, XllmError> {
        let PreparedTask {
            workdir,
            store,
            config,
            merged: _,
            prompt,
            input,
            summary,
            initial_messages,
            file_stage,
            main_user_parts,
            tools,
            mut manager,
        } = prepared;
        store.ensure_writable()?;
        let run_id = store.create_run()?;
        manager.bind_run(&run_id);
        let (run_lock, workdir_lock) =
            Self::acquire_locks(&store, &run_id, &workdir, tools.enabled, &deps).await?;
        let inner = deps.llm_factory.create(&config.provider).await?;
        let llm = Arc::new(TimeoutLlmClient::new(
            inner,
            Duration::from_secs(config.limits.llm_timeout_secs.max(1)),
        ));
        let now = now_ms();
        let record = RunRecord {
            version: RUN_RECORD_VERSION,
            run_id: run_id.clone(),
            status: RunStatus::Running,
            workdir: workdir.display().to_string(),
            runs_dir: store.runs_dir().map(|p| p.display().to_string()),
            created_at_ms: now,
            updated_at_ms: now,
            summary,
            input,
            config: config.clone(),
            prompt,
            file_model_stage: None,
            pending_input: Some(PendingInput {
                initial_messages: initial_messages.clone(),
                main_user_parts: main_user_parts.clone(),
                file_stage: file_stage.clone(),
            }),
            latest_snapshot_idx: None,
            last_error: None,
            result: None,
            artifacts: Vec::new(),
            usage: UsageRecord::default(),
            limit_reason: None,
            interrupt_reason: None,
            compactions: 0,
            pid: std::process::id(),
        };
        store.write_record(&record)?;
        let record = Arc::new(Mutex::new(record));
        let manager = Arc::new(manager);
        let tool_cancel = manager.cancel.clone();
        let waist_deps = Self::build_waist_deps(
            &store,
            &record,
            &run_id,
            llm.clone(),
            manager.clone(),
            config.loop_model,
            &tools,
            &deps,
        );
        Ok(XllmRun {
            run_id,
            store,
            record,
            deps,
            waist_deps,
            llm,
            manager,
            ctx: None,
            interrupter: XllmInterrupter {
                requested: Arc::new(Mutex::new(None)),
                handle: Arc::new(Mutex::new(None)),
                tool_cancel,
            },
            file_stage,
            initial_messages,
            main_user_parts,
            _run_lock: run_lock,
            _workdir_lock: workdir_lock,
        })
    }

    /// 恢复：按编号或当前工作目录最近的未完成任务。终态只返回记录。
    pub async fn resume(
        store: &RunStore,
        run_id: Option<&str>,
        workdir_filter: Option<&Path>,
        limits: ResumeLimits,
        deps: XllmDeps,
    ) -> Result<ResumeStart, XllmError> {
        let record = match run_id {
            Some(id) => store.read_record(id)?,
            None => {
                let wd = workdir_filter.ok_or_else(|| {
                    XllmError::Input("resume without --run needs a working directory".into())
                })?;
                latest_run(store, Some(wd), true)?.ok_or_else(|| XllmError::NotResumable {
                    run_id: "(latest)".into(),
                    reason: format!(
                        "no unfinished run for working directory {} under {}",
                        wd.display(),
                        store
                            .runs_dir()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "(memory)".into())
                    ),
                })?
            }
        };
        if record.status.is_terminal() {
            if !limits.is_empty() {
                return Err(XllmError::RunTerminal {
                    run_id: record.run_id.clone(),
                    status: record.status,
                    hint:
                        "execution limits do not apply to a terminal run; start a new task instead"
                            .into(),
                });
            }
            return Ok(ResumeStart::Terminal(record));
        }
        if record.status == RunStatus::Running && store.is_run_live(&record.run_id) {
            return Err(XllmError::RunBusy {
                run_id: record.run_id.clone(),
            });
        }
        if record.prompt.protocol_version != RUNTIME_PROTOCOL_VERSION {
            return Err(XllmError::NotResumable {
                run_id: record.run_id.clone(),
                reason: format!(
                    "saved runtime protocol `{}` is not supported by this executor (`{}`)",
                    record.prompt.protocol_version, RUNTIME_PROTOCOL_VERSION
                ),
            });
        }
        let workdir = PathBuf::from(&record.workdir);
        if !workdir.is_dir() {
            return Err(XllmError::NotResumable {
                run_id: record.run_id.clone(),
                reason: format!(
                    "original working directory {} no longer exists",
                    workdir.display()
                ),
            });
        }
        let run_id_s = record.run_id.clone();
        let (run_lock, workdir_lock) = Self::acquire_locks(
            store,
            &run_id_s,
            &workdir,
            record.config.tools.enabled,
            &deps,
        )
        .await?;

        // 重新展开工具（同一来源），重新连接 Provider（重新解析凭据引用）。
        let tools_cfg = ToolsConfig {
            enabled: Some(record.config.tools.enabled),
            filesystem_policy: Some(record.config.tools.filesystem_policy),
            tools2actions: Some(record.config.tools.tools2actions),
            tools: Some(record.config.tools.tool_sources.clone()),
            actions: Some(record.config.tools.action_sources.clone()),
            bash_tools: Some(record.config.tools.bash_tools.clone()),
        };
        let (_rebuilt, mut manager) = build_toolset(
            &tools_cfg,
            record.config.tools.sources.clone(),
            record.config.loop_model,
            &workdir,
            &run_id_s,
            &deps,
        )
        .await?;
        manager.bind_run(&run_id_s);
        let mut record = record;
        if let Some(v) = limits.max_tokens {
            record.config.limits.max_tokens = Some(v);
        }
        if let Some(v) = limits.timeout_secs {
            record.config.limits.timeout_secs = v;
        }
        if let Some(v) = limits.llm_timeout_secs {
            record.config.limits.llm_timeout_secs = v;
        }
        let old_max_rounds = record.config.limits.max_rounds;
        if let Some(v) = limits.max_rounds {
            record.config.limits.max_rounds = v;
        }
        let inner = deps.llm_factory.create(&record.config.provider).await?;
        let llm = Arc::new(TimeoutLlmClient::new(
            inner,
            Duration::from_secs(record.config.limits.llm_timeout_secs.max(1)),
        ));
        let tools = record.config.tools.clone();
        let loop_model = record.config.loop_model;

        // 装回快照（若已进入主模型阶段）。
        let snapshot = match record.latest_snapshot_idx {
            Some(idx) => Some(store.get_snapshot(&run_id_s, idx)?),
            None => None,
        };
        record.status = RunStatus::Running;
        record.pid = std::process::id();
        record.updated_at_ms = now_ms();
        store.write_record(&record)?;
        let record = Arc::new(Mutex::new(record));
        let manager = Arc::new(manager);
        let waist_deps = Self::build_waist_deps(
            store,
            &record,
            &run_id_s,
            llm.clone(),
            manager.clone(),
            loop_model,
            &tools,
            &deps,
        );
        let mut ctx = None;
        if let Some(mut snap) = snapshot {
            // 本次命令的执行时长从恢复启动重新计算；轮数额度沿用已消耗值。
            let limits_now = record.lock().expect("record lock").config.limits.clone();
            snap.state.started_at_ms = now_ms();
            snap.request.budget.max_wallclock_ms = Some(limits_now.timeout_secs * 1000);
            snap.request.model_policy.max_completion_tokens = limits_now.max_tokens;
            if limits_now.max_rounds != old_max_rounds {
                let consumed = old_max_rounds.saturating_sub(snap.state.rounds_left);
                snap.state.rounds_left = limits_now.max_rounds.saturating_sub(consumed);
                snap.request.tool_policy.max_rounds = limits_now.max_rounds;
            }
            snap.state.pending_tool_calls.clear();
            let resumed =
                LLMContext::resume(snap, ResumeFill::ResumeFromMidRun, waist_deps.clone())
                    .map_err(|e| XllmError::NotResumable {
                        run_id: run_id_s.clone(),
                        reason: format!("saved progress cannot be loaded: {e}"),
                    })?;
            ctx = Some(resumed);
        }
        let interrupter = XllmInterrupter {
            requested: Arc::new(Mutex::new(None)),
            handle: Arc::new(Mutex::new(None)),
            tool_cancel: manager.cancel.clone(),
        };
        if let Some(c) = &ctx {
            *interrupter.handle.lock().expect("lock") = Some(c.interrupt_handle());
        }
        let (file_stage, main_user_parts, initial_messages) = {
            let rec = record.lock().expect("record lock");
            if rec.latest_snapshot_idx.is_none() {
                let pending = rec
                    .pending_input
                    .clone()
                    .ok_or_else(|| XllmError::NotResumable {
                        run_id: run_id_s.clone(),
                        reason: "the run has neither a saved snapshot nor its initial input".into(),
                    })?;
                (
                    pending.file_stage,
                    pending.main_user_parts,
                    pending.initial_messages,
                )
            } else {
                (None, Vec::new(), Vec::new())
            }
        };
        Ok(ResumeStart::Run(Box::new(XllmRun {
            run_id: run_id_s,
            store: store.clone(),
            record,
            deps,
            waist_deps,
            llm,
            manager,
            ctx,
            interrupter,
            file_stage,
            initial_messages,
            main_user_parts,
            _run_lock: run_lock,
            _workdir_lock: workdir_lock,
        })))
    }

    fn classify_error(err: &LLMComputeError, provider: &ProviderConfig) -> RunErrorRecord {
        let at_ms = now_ms();
        let (recoverable, kind, condition): (bool, &str, Option<String>) = match err {
            LLMComputeError::Timeout => (
                true,
                "provider_timeout",
                Some("the model request timed out; resume to retry it".into()),
            ),
            LLMComputeError::Cancelled => (true, "cancelled", Some("resume to retry".into())),
            LLMComputeError::Provider { failure, message } => match failure {
                ProviderFailure::Transient => (
                    true,
                    "provider_transient",
                    Some("wait for the model service to recover, then resume".into()),
                ),
                ProviderFailure::Permanent => {
                    let m = message.to_ascii_lowercase();
                    let auth = [
                        "token",
                        "unauthorized",
                        "401",
                        "403",
                        "auth",
                        "expired",
                        "api key",
                        "permission",
                        "credential",
                    ]
                    .iter()
                    .any(|k| m.contains(k));
                    if auth {
                        let src = provider
                            .session_token
                            .as_ref()
                            .or(provider.api_key.as_ref())
                            .map(|r| r.describe())
                            .unwrap_or_else(|| "the current identity".into());
                        (
                            true,
                            "credentials",
                            Some(format!(
                                "fix the credential referenced by {src}, then resume"
                            )),
                        )
                    } else {
                        (false, "provider_permanent", None)
                    }
                }
                ProviderFailure::Unknown => (false, "provider_error", None),
            },
            LLMComputeError::Checkpoint { .. } => (
                true,
                "storage",
                Some("fix the runs directory (permissions / disk space), then resume".into()),
            ),
            LLMComputeError::ToolRuntime { effect_unknown, .. } => (
                true,
                "tool_runtime",
                Some(if *effect_unknown {
                    "the tool infrastructure failed and the last action's effect is unknown; check the working directory, then resume".into()
                } else {
                    "the tool infrastructure failed before the action ran; fix it, then resume"
                        .into()
                }),
            ),
            LLMComputeError::OutputParse(_) => (false, "llm_output", None),
            LLMComputeError::PolicyRejected(_) | LLMComputeError::ToolFailed { .. } => {
                (false, "tool_errors_exhausted", None)
            }
            LLMComputeError::SnapshotCorrupted(_) => (false, "snapshot", None),
            LLMComputeError::Internal(_) => (false, "internal", None),
        };
        RunErrorRecord {
            phase: "model".into(),
            kind: kind.into(),
            message: err.to_string(),
            recoverable,
            condition,
            at_ms,
        }
    }

    async fn run_file_stage(
        &mut self,
        plan: &FileStagePlan,
    ) -> Result<Option<RunOutcome>, XllmError> {
        self.emit(RunEvent::Phase {
            phase: RunPhase::FileModel,
            detail: format!("{} image(s) via {}", plan.images.len(), plan.model),
        });
        let provider = self.record().config.provider.clone();
        let inner = self.deps.llm_factory.create(&provider).await?;
        let llm_timeout = self.record().config.limits.llm_timeout_secs.max(1);
        let client = TimeoutLlmClient::new(inner, Duration::from_secs(llm_timeout));
        let mut parts = vec![AiContent::text(format!(
            "Analyze the attached image(s) for the following task. Describe everything relevant to the task in detail, keep the image order and labels, and when several images are given, describe the relationships and differences between them explicitly. Do not answer the task itself; produce a faithful analysis that another model will use.\n\nTask: {}",
            plan.request_text
        ))];
        for (label, src) in &plan.images {
            parts.push(AiContent::text(format!("[{label}]")));
            parts.push(AiContent::image(src.clone()));
        }
        let req = LlmInferenceRequest {
            trace_id: None,
            messages: vec![
                AiMessage::text(
                    AiRole::System,
                    "You are an image analysis assistant. You receive real images; describe their actual content precisely.",
                ),
                AiMessage::new(AiRole::User, parts),
            ],
            model_alias: plan.model.clone(),
            fallbacks: Vec::new(),
            temperature: None,
            max_completion_tokens: self.record().config.limits.max_tokens,
            force_json: false,
            json_schema: None,
            provider_options: None,
            disable_capabilities: Vec::new(),
            tool_specs: Vec::new(),
            allow_tool_calls: false,
            abort: llm_context::InferenceAbortToken::noop(),
        };
        self.emit(RunEvent::LlmStarted {
            model: plan.model.clone(),
        });
        let started = now_ms();
        let result = client.infer(req).await;
        self.emit(RunEvent::LlmFinished {
            ok: result.is_ok(),
            elapsed_ms: now_ms().saturating_sub(started),
        });
        match result {
            Ok(resp) => {
                let analysis = resp.message.text_content();
                if analysis.trim().is_empty() {
                    let err = RunErrorRecord {
                        phase: "file_model".into(),
                        kind: "empty_analysis".into(),
                        message: format!("file model `{}` returned no analysis", plan.model),
                        recoverable: false,
                        condition: None,
                        at_ms: now_ms(),
                    };
                    self.update(|r| {
                        r.status = RunStatus::Failed;
                        r.last_error = Some(err);
                        r.usage.llm_requests += 1;
                    })?;
                    return Ok(Some(RunOutcome::Failed(self.record())));
                }
                let usage = resp.usage.clone();
                self.update(|r| {
                    r.file_model_stage = Some(FileModelStageRecord {
                        model: plan.model.clone(),
                        analysis: analysis.clone(),
                        usage: usage.clone(),
                        response_model: client.last_model(),
                        completed_at_ms: now_ms(),
                    });
                    if let Some(u) = &usage {
                        add_usage(&mut r.usage.file_model, u);
                    }
                    r.usage.llm_requests += 1;
                })?;
                Ok(None)
            }
            Err(e) => {
                let mut err = Self::classify_error(&e, &provider);
                err.phase = "file_model".into();
                let status = if err.recoverable {
                    RunStatus::Paused
                } else {
                    RunStatus::Failed
                };
                self.update(|r| {
                    r.status = status;
                    r.last_error = Some(err);
                    r.usage.llm_requests += 1;
                })?;
                Ok(Some(RunOutcome::from_record(self.record())))
            }
        }
    }

    fn build_initial_context(&mut self) -> Result<(), XllmError> {
        let rec = self.record();
        let mut messages = self.initial_messages.clone();
        if let Some(stage) = &rec.file_model_stage {
            // 主模型收到带来源的分析结果，不再直接收到图片。
            let mut parts = self.main_user_parts.clone();
            parts.push(AiContent::text(format!(
                "<image_analysis model=\"{}\">\n{}\n</image_analysis>",
                xml_escape(&stage.model),
                stage.analysis.trim_end()
            )));
            messages = vec![
                AiMessage::text(AiRole::System, rec.prompt.system_prompt.clone()),
                AiMessage::new(AiRole::User, parts),
            ];
        }
        let cfg = &rec.config;
        let native_names: Vec<String> = cfg.tools.native.iter().map(|t| t.name.clone()).collect();
        let action_names: Vec<String> = cfg.tools.actions.iter().map(|t| t.name.clone()).collect();
        let tool_policy = ToolPolicy {
            mode: if native_names.is_empty() {
                ToolMode::None
            } else {
                ToolMode::Whitelist
            },
            whitelist: native_names,
            action_mode: if action_names.is_empty() {
                ToolMode::None
            } else {
                ToolMode::Whitelist
            },
            action_whitelist: action_names,
            max_rounds: if cfg.tools.enabled {
                cfg.limits.max_rounds
            } else {
                0
            },
            max_calls_per_round: 16,
            max_observation_bytes: 64 * 1024,
            disable_capabilities: cfg.disable_capabilities.clone(),
            parallel: false,
            allow_deferred: false,
        };
        let output = if cfg.json && cfg.loop_model == LoopModel::FunctionCall {
            OutputSpec::Json {
                schema: cfg.json_schema.clone(),
                strict: false,
            }
        } else {
            OutputSpec::Text
        };
        let request = LLMContextRequest {
            owner: ContextOwnerRef::OneShot {
                id: self.run_id.clone(),
            },
            trace: Some(self.run_id.clone()),
            objective: rec.summary.clone(),
            behavior_name: if cfg.loop_model == LoopModel::Behavior {
                "xllm".to_string()
            } else {
                String::new()
            },
            input: messages,
            model_policy: ModelPolicy {
                preferred: cfg.model.clone(),
                fallbacks: Vec::new(),
                temperature: None,
                max_completion_tokens: cfg.limits.max_tokens,
                provider_options: None,
            },
            tool_policy,
            output,
            budget: BudgetSpec {
                max_total_tokens: None,
                max_completion_tokens: cfg.limits.max_tokens,
                max_wallclock_ms: Some(cfg.limits.timeout_secs * 1000),
                max_cost_units: None,
                on_exhausted: llm_context::request::BudgetAction::Fail,
                context_yield_threshold: Some(ContextThreshold::Ratio {
                    value: DEFAULT_CONTEXT_YIELD_RATIO,
                }),
            },
            human_policy: Default::default(),
            error_policy: ErrorPolicy {
                max_consecutive_errors: DEFAULT_MAX_CONSECUTIVE_ERRORS,
            },
            forbid_next_behavior: false,
        };
        let ctx = LLMContext::new(request, self.waist_deps.clone());
        *self.interrupter.handle.lock().expect("lock") = Some(ctx.interrupt_handle());
        // 首次 Provider 请求前的快照。
        let hook = SnapshotHook {
            store: self.store.clone(),
            record: self.record.clone(),
        };
        hook.commit(&ctx.snapshot())?;
        self.ctx = Some(ctx);
        self.update(|r| {
            r.pending_input = None;
        })?;
        Ok(())
    }

    fn finish_with_response(
        &mut self,
        raw: String,
        usage: AiUsage,
    ) -> Result<RunOutcome, XllmError> {
        self.emit(RunEvent::Phase {
            phase: RunPhase::SavingResult,
            detail: String::new(),
        });
        let cfg = self.record().config.clone();
        let (extracted, extract_error) = match extract_result(&raw, &cfg.result_format) {
            Ok(v) => (Some(v), None),
            Err(e) => (None, Some(e.to_string())),
        };
        let (json_valid, json_error) = if cfg.json {
            match extracted.as_ref().map(|v| v.as_json()) {
                Some(Ok(_)) => (Some(true), None),
                Some(Err(e)) => (Some(false), Some(e)),
                None => (Some(false), Some("extraction failed".to_string())),
            }
        } else {
            (None, None)
        };
        let response_model = self.llm.last_model();
        let calls = self.llm.calls();
        self.update(|r| {
            r.status = RunStatus::Completed;
            r.last_error = None;
            r.usage.main = Some(usage);
            r.usage.llm_requests += calls;
            r.result = Some(RunResultRecord {
                raw,
                extracted,
                extract_error,
                json_valid,
                json_error,
                response_model,
            });
        })?;
        Ok(RunOutcome::Completed(self.record()))
    }

    /// 驱动到本次命令的停止点：完成 / 暂停 / 中断 / 失败 / 达到限制。
    pub async fn execute(&mut self) -> Result<RunOutcome, XllmError> {
        self.manager
            .set_deadline(self.record().config.limits.timeout_secs);
        self.emit(RunEvent::Phase {
            phase: RunPhase::PreparingInput,
            detail: String::new(),
        });
        if let Some(reason) = self.interrupter.requested() {
            self.update(|r| {
                r.status = RunStatus::Interrupted;
                r.interrupt_reason = Some(reason);
            })?;
            return Ok(RunOutcome::Interrupted(self.record()));
        }
        if self.ctx.is_none() {
            if let Some(plan) = self.file_stage.clone() {
                if self.record().file_model_stage.is_none() {
                    if let Some(outcome) = self.run_file_stage(&plan).await? {
                        return Ok(outcome);
                    }
                }
            }
            self.build_initial_context()?;
        }
        let provider = self.record().config.provider.clone();
        let model = self.record().config.model.clone();
        loop {
            if let Some(reason) = self.interrupter.requested() {
                self.update(|r| {
                    r.status = RunStatus::Interrupted;
                    r.interrupt_reason = Some(reason);
                })?;
                return Ok(RunOutcome::Interrupted(self.record()));
            }
            self.emit(RunEvent::Phase {
                phase: RunPhase::WaitingModel,
                detail: model.clone(),
            });
            let mut ctx = self
                .ctx
                .take()
                .ok_or_else(|| XllmError::Other("no active context (already finished?)".into()))?;
            let outcome = ctx.run().await;
            // outcome 边界快照。
            let hook = SnapshotHook {
                store: self.store.clone(),
                record: self.record.clone(),
            };
            let snapshot = ctx.snapshot();
            if let Err(e) = hook.commit(&snapshot) {
                self.ctx = Some(ctx);
                let err = RunErrorRecord {
                    phase: "storage".into(),
                    kind: "storage".into(),
                    message: e.to_string(),
                    recoverable: true,
                    condition: Some("fix the runs directory, then resume".into()),
                    at_ms: now_ms(),
                };
                let _ = self.update(|r| {
                    r.status = RunStatus::Paused;
                    r.last_error = Some(err);
                });
                return Ok(RunOutcome::Paused(self.record()));
            }
            match outcome {
                LLMContextOutcome::Done {
                    output,
                    usage,
                    response,
                    ..
                } => {
                    let raw = match &output {
                        ContextOutput::Json { content } => {
                            let text = response.message.text_content();
                            if text.trim().is_empty() {
                                content.to_string()
                            } else {
                                text
                            }
                        }
                        ContextOutput::Text { .. } => response.message.text_content(),
                    };
                    return self.finish_with_response(raw, usage);
                }
                LLMContextOutcome::BudgetExhausted { which, usage, .. } => {
                    let calls = self.llm.calls();
                    let reason = match which {
                        BudgetKind::ToolRounds => format!(
                            "tool round limit ({}) reached",
                            self.record().config.limits.max_rounds
                        ),
                        BudgetKind::Wallclock => format!(
                            "total execution time limit ({}s) reached",
                            self.record().config.limits.timeout_secs
                        ),
                        BudgetKind::Tokens => "token budget exhausted".to_string(),
                        BudgetKind::CostUnits => "cost budget exhausted".to_string(),
                    };
                    self.update(|r| {
                        r.status = RunStatus::LimitReached;
                        r.usage.main = Some(usage);
                        r.usage.llm_requests += calls;
                        r.limit_reason = Some(reason);
                    })?;
                    return Ok(RunOutcome::LimitReached(self.record()));
                }
                LLMContextOutcome::Error { error, usage, .. } => {
                    let calls = self.llm.calls();
                    let err = Self::classify_error(&error, &provider);
                    let status = if err.recoverable {
                        RunStatus::Paused
                    } else {
                        RunStatus::Failed
                    };
                    if error.source() == ErrorSource::Runtime {
                        // 上下文仍有效；本进程不再继续，交给 resume。
                        self.ctx = Some(ctx);
                    }
                    self.update(|r| {
                        r.status = status;
                        r.usage.main = Some(usage);
                        r.usage.llm_requests += calls;
                        r.last_error = Some(err);
                    })?;
                    return Ok(RunOutcome::from_record(self.record()));
                }
                LLMContextOutcome::Interrupted { reason, usage, .. } => {
                    let calls = self.llm.calls();
                    self.update(|r| {
                        r.status = RunStatus::Interrupted;
                        r.usage.main = Some(usage);
                        r.usage.llm_requests += calls;
                        r.interrupt_reason = Some(reason);
                    })?;
                    return Ok(RunOutcome::Interrupted(self.record()));
                }
                LLMContextOutcome::PendingTool { .. } => {
                    let calls = self.llm.calls();
                    self.update(|r| {
                        r.status = RunStatus::Failed;
                        r.usage.llm_requests += calls;
                        r.last_error = Some(RunErrorRecord {
                            phase: "tool".into(),
                            kind: "deferred_tool".into(),
                            message:
                                "a tool returned a deferred result, which xllm does not support"
                                    .into(),
                            recoverable: false,
                            condition: None,
                            at_ms: now_ms(),
                        });
                    })?;
                    return Ok(RunOutcome::Failed(self.record()));
                }
                LLMContextOutcome::ContextLimitReached {
                    accumulated,
                    snapshot,
                    usage,
                    ..
                } => {
                    self.emit(RunEvent::Phase {
                        phase: RunPhase::CompactingContext,
                        detail: format!("{} messages", accumulated.len()),
                    });
                    let compactions = self.record().compactions;
                    if compactions >= 3 {
                        let calls = self.llm.calls();
                        self.update(|r| {
                            r.status = RunStatus::Failed;
                            r.usage.main = Some(usage);
                            r.usage.llm_requests += calls;
                            r.last_error = Some(RunErrorRecord {
                                phase: "model".into(),
                                kind: "context_capacity".into(),
                                message: "the task context still exceeds the model capacity after repeated compaction".into(),
                                recoverable: false,
                                condition: None,
                                at_ms: now_ms(),
                            });
                        })?;
                        return Ok(RunOutcome::Failed(self.record()));
                    }
                    let compressor = LlmSummarizeCompressor::new(
                        self.waist_deps.clone().into_traditional(),
                        model.clone(),
                        DEFAULT_AUTO_COMPRESS_TARGET_TOKENS,
                    );
                    let rewritten = match compressor
                        .compress(accumulated, Path::new(&self.record().workdir))
                        .await
                    {
                        Ok(r) => r,
                        Err(e) => {
                            let calls = self.llm.calls();
                            self.update(|r| {
                                r.status = RunStatus::Failed;
                                r.usage.main = Some(usage);
                                r.usage.llm_requests += calls;
                                r.last_error = Some(RunErrorRecord {
                                    phase: "model".into(),
                                    kind: "context_compaction".into(),
                                    message: format!("context compaction failed: {e}"),
                                    recoverable: false,
                                    condition: None,
                                    at_ms: now_ms(),
                                });
                            })?;
                            return Ok(RunOutcome::Failed(self.record()));
                        }
                    };
                    let mut prepared = snapshot;
                    prepared.state.accumulated = rewritten.clone();
                    hook.commit(&prepared)?;
                    let resumed = LLMContext::resume(
                        prepared,
                        ResumeFill::RewrittenHistory { history: rewritten },
                        self.waist_deps.clone(),
                    )
                    .map_err(|e| {
                        XllmError::Other(format!("resume after compaction failed: {e}"))
                    })?;
                    *self.interrupter.handle.lock().expect("lock") =
                        Some(resumed.interrupt_handle());
                    self.ctx = Some(resumed);
                    self.update(|r| {
                        r.compactions += 1;
                    })?;
                    continue;
                }
            }
        }
    }
}

impl XllmToolManager {
    fn bind_run(&mut self, run_id: &str) {
        self.session_template.trace_id = run_id.to_string();
        self.session_template.session_id = run_id.to_string();
    }
}

// =========================================================================
// 查询（F06）与结构化结果（F07）
// =========================================================================

fn same_workdir(a: &str, b: &Path) -> bool {
    let pa = Path::new(a);
    let ca = pa.canonicalize().unwrap_or_else(|_| pa.to_path_buf());
    let cb = b.canonicalize().unwrap_or_else(|_| b.to_path_buf());
    ca == cb
}

/// 最近的多次任务，按更新时间倒序。
pub fn list_runs(
    store: &RunStore,
    workdir: Option<&Path>,
    limit: usize,
) -> Result<Vec<RunSummary>, XllmError> {
    let mut records = store.list_records()?;
    if let Some(wd) = workdir {
        records.retain(|r| same_workdir(&r.workdir, wd));
    }
    records.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
    Ok(records
        .iter()
        .take(limit)
        .map(|r| store.summarize(r))
        .collect())
}

/// 当前工作目录最新任务（可选只看未完成）。
pub fn latest_run(
    store: &RunStore,
    workdir: Option<&Path>,
    only_unfinished: bool,
) -> Result<Option<RunRecord>, XllmError> {
    let mut records = store.list_records()?;
    if let Some(wd) = workdir {
        records.retain(|r| same_workdir(&r.workdir, wd));
    }
    if only_unfinished {
        records.retain(|r| !r.status.is_terminal());
    }
    records.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms));
    Ok(records.into_iter().next())
}

/// 读取一条记录及其用户可见状态。
pub fn load_run(store: &RunStore, run_id: &str) -> Result<(RunRecord, RunSummary), XllmError> {
    let rec = store.read_record(run_id)?;
    let summary = store.summarize(&rec);
    Ok((rec, summary))
}

/// 结构化执行结果（`--format json` 的载体）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct XllmResult {
    pub run_id: String,
    pub status: RunStatus,
    pub status_label: String,
    pub is_terminal: bool,
    pub resumable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json_error: Option<String>,
    #[serde(default)]
    pub artifacts: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<AiUsage>,
    pub usage_detail: UsageRecord,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunErrorRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interrupt_reason: Option<String>,
    pub provider: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    pub loop_model: LoopModel,
    pub result_format: String,
    pub workdir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runs_dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_command: Option<String>,
    #[serde(default)]
    pub config_files: Vec<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

/// 从记录构造结构化结果。`fmt` 可临时更换提取方式（`xllm result`）。
pub fn build_result_view(
    record: &RunRecord,
    summary: &RunSummary,
    fmt: Option<&ResultFormat>,
) -> XllmResult {
    let (answer, answer_kind, extract_error, json_error) = match &record.result {
        Some(res) => {
            let extracted = match fmt {
                Some(f) => extract_result(&res.raw, f).map(Some).unwrap_or_else(|e| {
                    let _ = e;
                    None
                }),
                None => res.extracted.clone(),
            };
            let extract_error = match fmt {
                Some(f) => extract_result(&res.raw, f).err().map(|e| e.to_string()),
                None => res.extract_error.clone(),
            };
            let (answer, kind) = match &extracted {
                Some(v) => {
                    let value = if record.config.json {
                        v.as_json()
                            .ok()
                            .unwrap_or_else(|| Value::String(v.to_output_text()))
                    } else {
                        match v {
                            ExtractedValue::Json { value } => value.clone(),
                            other => Value::String(other.to_output_text()),
                        }
                    };
                    (Some(value), Some(v.kind().to_string()))
                }
                None => (None, None),
            };
            (answer, kind, extract_error, res.json_error.clone())
        }
        None => (None, None, None, None),
    };
    XllmResult {
        run_id: record.run_id.clone(),
        status: summary.status,
        status_label: summary.status_label.clone(),
        is_terminal: summary.is_terminal,
        resumable: summary.resumable,
        answer,
        answer_kind,
        extract_error,
        json_error,
        artifacts: record.artifacts.clone(),
        usage: record.usage.total(),
        usage_detail: record.usage.clone(),
        error: record.last_error.clone(),
        limit_reason: record.limit_reason.clone(),
        interrupt_reason: record.interrupt_reason.clone(),
        provider: record.config.provider.effective_kind().as_str().to_string(),
        model: record.config.model.clone(),
        file_model: record.config.file_model.clone(),
        response_model: record
            .result
            .as_ref()
            .and_then(|r| r.response_model.clone()),
        loop_model: record.config.loop_model,
        result_format: fmt
            .map(|f| f.as_string())
            .unwrap_or_else(|| record.config.result_format.as_string()),
        workdir: record.workdir.clone(),
        runs_dir: record.runs_dir.clone(),
        resume_command: if summary.resumable {
            Some(record.resume_command())
        } else {
            None
        },
        config_files: record.config.config_files.clone(),
        created_at_ms: record.created_at_ms,
        updated_at_ms: record.updated_at_ms,
    }
}

// =========================================================================
// 测试
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::AiRole;
    use llm_context::deps::LlmInferenceRequest;
    use std::sync::Mutex as StdMutex;

    /// 记录每次推理请求的关键信息。
    #[derive(Clone, Debug)]
    struct SeenRequest {
        model: String,
        messages: Vec<AiMessage>,
        tool_names: Vec<String>,
        allow_tool_calls: bool,
        force_json: bool,
    }

    struct ScriptedLlm {
        script: StdMutex<Vec<Result<AiResponse, LLMComputeError>>>,
        seen: StdMutex<Vec<SeenRequest>>,
    }

    impl ScriptedLlm {
        fn new(items: Vec<Result<AiResponse, LLMComputeError>>) -> Arc<Self> {
            Arc::new(Self {
                script: StdMutex::new(items.into_iter().rev().collect()),
                seen: StdMutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<SeenRequest> {
            self.seen.lock().unwrap().clone()
        }

        fn calls(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
    }

    #[async_trait]
    impl LlmClient for ScriptedLlm {
        async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponse, LLMComputeError> {
            self.seen.lock().unwrap().push(SeenRequest {
                model: req.model_alias.clone(),
                messages: req.messages.clone(),
                tool_names: req.tool_specs.iter().map(|t| t.name.clone()).collect(),
                allow_tool_calls: req.allow_tool_calls,
                force_json: req.force_json,
            });
            match self.script.lock().unwrap().pop() {
                Some(r) => r,
                None => Err(LLMComputeError::Internal("script exhausted".into())),
            }
        }
    }

    fn text(s: &str) -> Result<AiResponse, LLMComputeError> {
        Ok(AiResponse::text(s))
    }

    fn tool_call(name: &str, args: Value, id: &str) -> Result<AiResponse, LLMComputeError> {
        let args: HashMap<String, Value> = args
            .as_object()
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .collect();
        Ok(AiResponse::from_parts(
            None,
            vec![AiToolCall {
                name: name.into(),
                args,
                call_id: id.into(),
            }],
            vec![],
        ))
    }

    struct Env {
        tmp: tempfile::TempDir,
        workdir: PathBuf,
        runs_dir: PathBuf,
    }

    impl Env {
        fn new() -> Self {
            let tmp = tempfile::tempdir().unwrap();
            let workdir = tmp.path().join("project").join("src");
            std::fs::create_dir_all(&workdir).unwrap();
            let runs_dir = tmp.path().join("runs");
            Self {
                tmp,
                workdir,
                runs_dir,
            }
        }

        fn project(&self) -> PathBuf {
            self.tmp.path().join("project")
        }

        fn write(&self, rel: &str, content: &str) {
            let p = self.tmp.path().join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, content).unwrap();
        }

        fn deps(&self, llm: Arc<ScriptedLlm>) -> XllmDeps {
            XllmDeps::default()
                .with_llm(llm)
                .with_lock_dir(self.tmp.path().join("locks"))
        }

        fn overrides(&self) -> TaskOverrides {
            TaskOverrides {
                runs_dir: Some(self.runs_dir.clone()),
                ..Default::default()
            }
        }

        fn store(&self) -> RunStore {
            RunStore::disk(self.runs_dir.clone())
        }

        async fn run(
            &self,
            input: TaskInput,
            overrides: TaskOverrides,
            llm: Arc<ScriptedLlm>,
        ) -> Result<RunOutcome, XllmError> {
            let deps = self.deps(llm);
            let prepared = XllmTask::prepare(&self.workdir, input, overrides, &deps).await?;
            let mut run = XllmRun::start(prepared, deps).await?;
            run.execute().await
        }
    }

    fn system_text(req: &SeenRequest) -> String {
        req.messages
            .iter()
            .find(|m| m.role == AiRole::System)
            .map(|m| m.text_content())
            .unwrap_or_default()
    }

    fn user_text(req: &SeenRequest) -> String {
        req.messages
            .iter()
            .filter(|m| m.role == AiRole::User)
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn pos(hay: &str, needle: &str) -> usize {
        hay.find(needle)
            .unwrap_or_else(|| panic!("`{needle}` not found in:\n{hay}"))
    }

    const PARENT_CONFIG: &str = r#"
model: llm.chat
max_rounds: 3
prompt:
  select: review
  groups:
    checkpr: "./checkpr.yaml"
    review:
      default_user: |
        Review the code in {{runtime.cwd}}.
      sections:
        role:
          text: You are a code reviewer.
        contexts:
          text: "Project: {{env.XLLM_TEST_PROJECT}}. Cwd: {{runtime.cwd}}."
        rules:
          text: Read before judging.
        cmd_manual:
          text: Prefer rg.
        output_format:
          text: Answer in the report field.
      tools:
        enabled: true
        bash_tools:
          - name: rg
            description: search
            command: rg
            usage: "rg -n <pattern>"
    docs:
      sections:
        role:
          text: You are a docs editor.
  sections:
    "50":
      name: delivery_checklist
      text: Check references.
"#;

    const CHILD_CONFIG: &str = r#"
prompt:
  select: review
  sections:
    rules:
      text: Only report grounded findings.
    "25":
      name: project_conventions
      text: Public API changes need caller checks.
    "50":
      text: ""
    "100":
      text: Report file, evidence, suggestion.
"#;

    const CHECKPR_GROUP: &str = r#"
default_user: Check the git diff.
sections:
  role:
    text: You are a change reviewer.
tools:
  enabled: false
"#;

    #[test]
    fn discovery_walks_up_and_orders_ancestor_first() {
        let env = Env::new();
        env.write("project/.llm_context", "model: a\n");
        env.write("project/src/.llm_context", "model: b\n");
        let found = discover_config_files(&env.workdir);
        assert_eq!(found.len(), 2);
        assert!(found[0].starts_with(env.project()));
        assert!(found[1].starts_with(&env.workdir));
        let merged = merge_config_layers(&load_config_layers(&env.workdir).unwrap()).unwrap();
        assert_eq!(merged.model.as_deref(), Some("b"));
        assert_eq!(merged.files.len(), 2);
    }

    #[test]
    fn section_key_errors_are_diagnosable() {
        let path = Path::new("/tmp/x/.llm_context");
        let err =
            parse_llm_context_file(path, "prompt:\n  sections:\n    rules: a\n    \"30\": b\n")
                .unwrap_err();
        assert!(err.to_string().contains("line 30"), "{err}");
        let err = parse_llm_context_file(path, "prompt:\n  sections:\n    bogus: a\n").unwrap_err();
        assert!(err.to_string().contains("unknown section alias"), "{err}");
        let err = parse_llm_context_file(
            path,
            "prompt:\n  sections:\n    \"25\":\n      name: rules\n      text: x\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("bound to line 30"), "{err}");
        let err = parse_llm_context_file(path, "prompt:\n  sections:\n    \"0\": a\n").unwrap_err();
        assert!(err.to_string().contains("positive"), "{err}");
        let err = parse_llm_context_file(path, "unknown_field: 1\n").unwrap_err();
        assert!(err.to_string().contains("unknown field"), "{err}");
        let err = parse_llm_context_file(
            path,
            "prompt:\n  mode: custom\n  system: x\n  select: review\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("custom"), "{err}");
        let err = parse_llm_context_file(
            path,
            "tools:\n  tools:\n    - groupname: bash\n      mcp: x\n",
        )
        .unwrap_err();
        assert!(err.to_string().contains("exactly one"), "{err}");
        assert_eq!(
            parse_llm_context_file(path, "runs_dir: none\n")
                .unwrap()
                .runs_dir,
            Some(RunsDirSetting::Disabled)
        );
    }

    #[tokio::test]
    async fn sections_assemble_by_line_with_inheritance_and_clearing() {
        let env = Env::new();
        env.write("project/.llm_context", PARENT_CONFIG);
        env.write("project/src/.llm_context", CHILD_CONFIG);
        env.write("project/checkpr.yaml", CHECKPR_GROUP);
        std::env::set_var("XLLM_TEST_PROJECT", "demo");
        let llm = ScriptedLlm::new(vec![text("fine")]);
        let outcome = env
            .run(TaskInput::default(), env.overrides(), llm.clone())
            .await
            .unwrap();
        let record = outcome.record().clone();
        assert_eq!(record.status, RunStatus::Completed);
        assert_eq!(record.prompt.group.as_deref(), Some("review"));
        let sys = system_text(&llm.seen()[0]);
        let p_role = pos(&sys, "## role");
        let p_ctx = pos(&sys, "## contexts");
        let p_25 = pos(&sys, "## project_conventions");
        let p_rules = pos(&sys, "## rules");
        let p_cmd = pos(&sys, "## cmd_manual");
        let p_out = pos(&sys, "## output_format");
        let p_proto = pos(&sys, "## runtime_protocol");
        assert!(
            p_role < p_ctx
                && p_ctx < p_25
                && p_25 < p_rules
                && p_rules < p_cmd
                && p_cmd < p_out
                && p_out < p_proto
        );
        assert!(
            !sys.contains("delivery_checklist"),
            "cleared section must vanish"
        );
        assert!(!sys.contains("Check references"));
        assert!(sys.contains("Only report grounded findings"));
        assert!(
            !sys.contains("Read before judging"),
            "child replaces parent text"
        );
        assert!(sys.contains("Report file, evidence, suggestion"));
        assert!(
            !sys.contains("Answer in the report field"),
            "100 == output_format"
        );
        assert!(sys.contains("Project: demo."));
        assert!(sys.contains(&format!("Cwd: {}", record.workdir)));
        // contexts 的 cwd 已由模板呈现，系统不重复追加；时间仍追加。
        assert!(!sys.contains("Working directory:"));
        assert!(sys.contains("Current time:"));
        // exec 启用：命令手册出现；rules 列出工具。
        assert!(sys.contains("rg -n <pattern>"));
        assert!(sys.contains("Native tools available"));
        assert!(sys.contains(TOOL_EXEC));
        // 组默认任务进入 user。
        let user = user_text(&llm.seen()[0]);
        assert!(user.contains(&format!("Review the code in {}", record.workdir)));
        assert_eq!(record.input.request_source, "group_default");
        assert_eq!(record.config.limits.max_rounds, 3);
        assert!(record
            .prompt
            .template_vars
            .contains_key("env.XLLM_TEST_PROJECT"));
        assert!(record.config.tools.enabled);
        assert_eq!(record.config.tools.native.len(), 4);
    }

    #[tokio::test]
    async fn select_switches_group_and_external_group_is_expanded() {
        let env = Env::new();
        env.write("project/.llm_context", PARENT_CONFIG);
        env.write("project/src/.llm_context", CHILD_CONFIG);
        env.write("project/checkpr.yaml", CHECKPR_GROUP);
        std::env::set_var("XLLM_TEST_PROJECT", "demo");
        let llm = ScriptedLlm::new(vec![text("ok")]);
        let overrides = TaskOverrides {
            select: Some("checkpr".into()),
            ..env.overrides()
        };
        let outcome = env
            .run(TaskInput::default(), overrides, llm.clone())
            .await
            .unwrap();
        let rec = outcome.record();
        assert_eq!(rec.prompt.group.as_deref(), Some("checkpr"));
        let sys = system_text(&llm.seen()[0]);
        assert!(sys.contains("You are a change reviewer"));
        assert!(
            !sys.contains("You are a code reviewer"),
            "no text from unselected group"
        );
        assert!(
            sys.contains("Only report grounded findings"),
            "directory-level sections still apply"
        );
        assert!(!rec.config.tools.enabled, "external group disables tools");
        assert!(sys.contains("no tools or actions can be called"));
        assert!(!sys.contains("## cmd_manual"));
        assert!(user_text(&llm.seen()[0]).contains("Check the git diff"));

        // 不存在的组
        let err = XllmTask::prepare(
            &env.workdir,
            TaskInput::default(),
            TaskOverrides {
                select: Some("nope".into()),
                ..env.overrides()
            },
            &env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("does not exist"), "{err}");
        assert!(err.to_string().contains("checkpr"), "{err}");
    }

    #[tokio::test]
    async fn custom_system_overrides_groups_but_keeps_protocol_and_capabilities() {
        let env = Env::new();
        env.write("project/.llm_context", PARENT_CONFIG);
        env.write("project/checkpr.yaml", CHECKPR_GROUP);
        std::env::set_var("XLLM_TEST_PROJECT", "demo");
        let llm = ScriptedLlm::new(vec![text("ok")]);
        let overrides = TaskOverrides {
            system: Some("You are a log analyst in {{runtime.cwd}}.".into()),
            tools: Some(false),
            ..env.overrides()
        };
        let outcome = env
            .run(TaskInput::question("explain"), overrides, llm.clone())
            .await
            .unwrap();
        let rec = outcome.record();
        assert_eq!(rec.prompt.mode, PromptMode::Custom);
        assert!(rec.prompt.group.is_none());
        let sys = system_text(&llm.seen()[0]);
        assert!(sys.starts_with("You are a log analyst in"));
        assert!(!sys.contains("## role"));
        assert!(sys.contains("## runtime_protocol"));
        assert!(sys.contains("no tools or actions"));
        assert!(!rec.config.tools.enabled, "--no-tools beats group enabled");
    }

    #[tokio::test]
    async fn template_errors_are_reported_before_any_model_call() {
        let env = Env::new();
        env.write(
            "project/.llm_context",
            "prompt:\n  sections:\n    role:\n      text: \"{{env.XLLM_DEFINITELY_MISSING}}\"\n",
        );
        std::env::remove_var("XLLM_DEFINITELY_MISSING");
        let llm = ScriptedLlm::new(vec![text("ok")]);
        let err = env
            .run(TaskInput::question("q"), env.overrides(), llm.clone())
            .await
            .unwrap_err();
        assert!(matches!(err, XllmError::Template { .. }), "{err}");
        assert!(err.to_string().contains("XLLM_DEFINITELY_MISSING"));
        assert_eq!(llm.calls(), 0);
        assert!(
            list_runs(&env.store(), None, 10).unwrap().is_empty(),
            "no run created"
        );

        let tenv = TemplateEnv::for_new_run(Path::new("/w"));
        assert_eq!(tenv.render("a \\{{b}} c", "t").unwrap(), "a {{b}} c");
        assert_eq!(tenv.render("cwd={{ runtime.cwd }}", "t").unwrap(), "cwd=/w");
        assert!(tenv.render("{{runtime.nope}}", "t").is_err());
        assert!(tenv.render("{{bad", "t").is_err());
    }

    #[tokio::test]
    async fn tools_priority_group_then_prompt_tools_then_cli() {
        let env = Env::new();
        env.write("project/.llm_context", PARENT_CONFIG);
        env.write("project/checkpr.yaml", CHECKPR_GROUP);
        env.write(
            "project/src/.llm_context",
            "prompt:\n  select: review\n  tools:\n    tools: []\n",
        );
        std::env::set_var("XLLM_TEST_PROJECT", "demo");
        // 显式空列表：启用但没有工具。
        let llm = ScriptedLlm::new(vec![text("ok")]);
        let rec = env
            .run(TaskInput::question("q"), env.overrides(), llm.clone())
            .await
            .unwrap()
            .record()
            .clone();
        assert!(rec.config.tools.enabled);
        assert!(rec.config.tools.native.is_empty());
        assert!(!rec.config.tools.exec_enabled);
        assert_eq!(
            rec.config.tools.sources.get("tools").map(String::as_str),
            Some(rec.config.config_files[1].as_str())
        );
        assert!(!llm.seen()[0].allow_tool_calls);

        // 默认关闭：没有任何配置时不主动调用工具。
        let env2 = Env::new();
        let llm2 = ScriptedLlm::new(vec![text("ok")]);
        let rec2 = env2
            .run(TaskInput::question("q"), env2.overrides(), llm2.clone())
            .await
            .unwrap()
            .record()
            .clone();
        assert!(!rec2.config.tools.enabled);
        assert!(llm2.seen()[0].tool_names.is_empty());

        // --tools 且未配置列表 → 默认 bash 组。
        let llm3 = ScriptedLlm::new(vec![text("ok")]);
        let rec3 = env2
            .run(
                TaskInput::question("q"),
                TaskOverrides {
                    tools: Some(true),
                    ..env2.overrides()
                },
                llm3.clone(),
            )
            .await
            .unwrap()
            .record()
            .clone();
        let mut names = rec3.config.tools.all_names();
        names.sort();
        assert_eq!(names, vec!["edit_file", "exec", "read_file", "write_file"]);
        assert_eq!(llm3.seen()[0].tool_names.len(), 4);
        assert!(llm3.seen()[0].allow_tool_calls);

        // function_call + tools2actions → 配置不匹配。
        let env3 = Env::new();
        env3.write(
            "project/.llm_context",
            "tools:\n  enabled: true\n  tools2actions: true\n",
        );
        let err = env3
            .run(
                TaskInput::question("q"),
                env3.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, XllmError::Capability(_)), "{err}");
        // 未知工具组 / 未知具名工具
        let env4 = Env::new();
        env4.write(
            "project/.llm_context",
            "tools:\n  enabled: true\n  tools:\n    - groupname: nope\n",
        );
        let err = env4
            .run(
                TaskInput::question("q"),
                env4.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("unknown builtin tool group `nope`"),
            "{err}"
        );
        env4.write(
            "project/.llm_context",
            "tools:\n  enabled: true\n  tools:\n    - name: sendmsg\n",
        );
        let err = env4
            .run(
                TaskInput::question("q"),
                env4.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("sendmsg"), "{err}");
    }

    #[test]
    fn action_parser_handles_dynamic_tags_and_terminal_report() {
        let tools = vec![
            ResolvedTool {
                name: "exec".into(),
                description: "".into(),
                args_schema: json!({"type":"object","properties":{"command":{"type":"string"}},"required":["command"]}),
                source: "t".into(),
            },
            ResolvedTool {
                name: "write_file".into(),
                description: "".into(),
                args_schema: json!({"type":"object","properties":{"path":{"type":"string"},"content":{"type":"string"}},"required":["path","content"]}),
                source: "t".into(),
            },
            ResolvedTool {
                name: "edit_file".into(),
                description: "".into(),
                args_schema: json!({"type":"object","properties":{"path":{"type":"string"},"old_string":{"type":"string"},"new_string":{"type":"string"}},"required":["path","old_string","new_string"]}),
                source: "t".into(),
            },
            ResolvedTool {
                name: "mytool".into(),
                description: "".into(),
                args_schema: json!({"type":"object","properties":{"foo":{"type":"integer"},"bar":{"type":"string"},"opts":{"type":"object"}}}),
                source: "t".into(),
            },
        ];
        let parser = XllmActionParser::new(&tools);
        let xml = r#"```xml
<response>
  <thinking>plan</thinking>
  <actions>
    <exec><![CDATA[ls -la && echo "<done>"]]></exec>
    <write_file path="a.txt"><![CDATA[hi
there]]></write_file>
    <edit_file path="x.rs"><old_string><![CDATA[a]]></old_string><new_string>b &amp; c</new_string></edit_file>
    <mytool foo="7" bar="x"><![CDATA[{"opts":{"k":1}}]]></mytool>
    <mytool foo="8"><opts><![CDATA[{"k":2}]]></opts></mytool>
  </actions>
</response>
```"#;
        let r = parser.parse(&AiResponse::text(xml)).unwrap();
        assert_eq!(r.do_actions.len(), 5);
        assert_eq!(r.do_actions[0].name, "exec");
        assert_eq!(
            r.do_actions[0].args["command"],
            json!("ls -la && echo \"<done>\"")
        );
        assert_eq!(r.do_actions[1].args["path"], json!("a.txt"));
        assert_eq!(r.do_actions[1].args["content"], json!("hi\nthere"));
        assert_eq!(r.do_actions[2].args["old_string"], json!("a"));
        assert_eq!(r.do_actions[2].args["new_string"], json!("b & c"));
        assert_eq!(r.do_actions[3].args["foo"], json!(7));
        assert_eq!(r.do_actions[3].args["opts"], json!({"k":1}));
        assert_eq!(r.do_actions[4].args["opts"], json!({"k":2}));
        assert_eq!(r.thought.as_deref(), Some("plan"));
        assert!(r.next_behavior.is_none());
        assert!(r.self_report.is_none());

        let done = parser
            .parse(&AiResponse::text(
                "<response><report><![CDATA[final <answer>]]></report></response>",
            ))
            .unwrap();
        assert!(done.do_actions.is_empty());
        assert_eq!(done.self_report.as_deref(), Some("final <answer>"));
        assert_eq!(done.next_behavior.as_deref(), Some("done"));

        // 未知标签仍生成调用（由 ToolManager 报 not available）。
        let unk = parser
            .parse(&AiResponse::text(
                "<response><actions><nope x=\"1\"/></actions></response>",
            ))
            .unwrap();
        assert_eq!(unk.do_actions[0].name, "nope");
        assert!(parser.parse(&AiResponse::text("   ")).is_err());
    }

    #[test]
    fn action_usage_and_parser_agree_on_raw_body_values() {
        let tool = ResolvedTool {
            name: "read_file".into(),
            description: "Read a file".into(),
            args_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": "File path"}},
                "required": ["path"]
            }),
            source: "test".into(),
        };
        let parser = XllmActionParser::new(&[tool.clone()]);
        for action in [
            r#"<read_file path="fixture.txt"><![CDATA[]]></read_file>"#,
            r#"<read_file path="fixture.txt"><![CDATA[   ]]></read_file>"#,
            r#"<read_file path="fixture.txt"/>"#,
            "<read_file><![CDATA[fixture.txt]]></read_file>",
            "<read_file><path>fixture.txt</path></read_file>",
        ] {
            let response =
                AiResponse::text(format!("<response><actions>{action}</actions></response>"));
            let parsed = parser.parse(&response).unwrap();
            assert_eq!(
                parsed.do_actions[0].args["path"],
                json!("fixture.txt"),
                "{action}"
            );
        }
        let usage = render_action_usage(&tool);
        let example = usage.lines().next().unwrap().strip_prefix("- ").unwrap();
        assert!(example.contains("<![CDATA[<path value>]]>"), "{usage}");
        let response = AiResponse::text(format!(
            "<response><actions>{}</actions></response>",
            example.replace("<path value>", "fixture.txt")
        ));
        assert_eq!(
            parser.parse(&response).unwrap().do_actions[0].args["path"],
            json!("fixture.txt")
        );

        let parser = XllmActionParser::new(&[ResolvedTool {
            name: "write_file".into(),
            args_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
                "required": ["path", "content"]
            }),
            ..tool
        }]);
        for content in ["", "   ", "path: fixture.txt"] {
            let response = AiResponse::text(format!(
                r#"<response><actions><write_file path="out.txt"><![CDATA[{content}]]></write_file></actions></response>"#
            ));
            let parsed = parser.parse(&response).unwrap();
            assert_eq!(parsed.do_actions[0].args["content"], json!(content));
        }
    }

    #[test]
    fn result_extraction_json_xml_and_errors() {
        let fmt = ResultFormat::parse("result.report").unwrap();
        let v = extract_result(r#"{"report":"评审结论","n":1}"#, &fmt).unwrap();
        assert_eq!(
            v,
            ExtractedValue::Text {
                text: "评审结论".into()
            }
        );
        let v = extract_result("```json\n{\"report\": {\"a\": [1,2]}}\n```", &fmt).unwrap();
        assert_eq!(
            v,
            ExtractedValue::Json {
                value: json!({"a":[1,2]})
            }
        );
        let err = extract_result(r#"{"other":1}"#, &fmt).unwrap_err();
        assert!(err.to_string().contains("missing"), "{err}");
        let xml =
            "<response><thinking>t</thinking><report><![CDATA[done & dusted]]></report></response>";
        let v = extract_result(xml, &fmt).unwrap();
        assert_eq!(
            v,
            ExtractedValue::Text {
                text: "done & dusted".into()
            }
        );
        let v = extract_result("<r><report><a>1</a><b>2</b></report></r>", &fmt).unwrap();
        assert!(matches!(v, ExtractedValue::Xml { .. }));
        let err = extract_result("<r><report>1</report><report>2</report></r>", &fmt).unwrap_err();
        assert!(err.to_string().contains("matches 2"), "{err}");
        let err = extract_result("plain text", &fmt).unwrap_err();
        assert!(err.to_string().contains("neither JSON nor XML"), "{err}");
        let nested = ResultFormat::parse("result.a.b").unwrap();
        assert_eq!(
            extract_result(r#"{"a":{"b":"x"}}"#, &nested).unwrap(),
            ExtractedValue::Text { text: "x".into() }
        );
        assert!(ResultFormat::parse("bogus").is_err());
        assert_eq!(
            extract_result("anything", &ResultFormat::Raw).unwrap(),
            ExtractedValue::Text {
                text: "anything".into()
            }
        );
        assert!(ExtractedValue::Text {
            text: "{\"a\":1}".into()
        }
        .as_json()
        .is_ok());
        assert!(ExtractedValue::Text {
            text: "nope".into()
        }
        .as_json()
        .is_err());
    }

    #[tokio::test]
    async fn run_persists_record_and_materials_in_order() {
        let env = Env::new();
        env.write("project/src/notes.txt", "line one\nline two\n");
        let llm = ScriptedLlm::new(vec![text("the answer")]);
        let input = TaskInput {
            user: Some("summarize".into()),
            attachments: vec![Attachment::File {
                path: PathBuf::from("notes.txt"),
            }],
            stdin: Some("piped material".into()),
            structured: None,
            base_dir: Some(env.workdir.clone()),
        };
        let outcome = env.run(input, env.overrides(), llm.clone()).await.unwrap();
        let rec = outcome.record().clone();
        assert_eq!(rec.status, RunStatus::Completed);
        assert_eq!(rec.result.as_ref().unwrap().raw, "the answer");
        assert_eq!(
            rec.result.as_ref().unwrap().extracted,
            Some(ExtractedValue::Text {
                text: "the answer".into()
            })
        );
        assert_eq!(rec.input.request_source, "explicit");
        assert_eq!(rec.input.stdin_role.as_deref(), Some("material"));
        assert_eq!(rec.input.attachments[0].label, "notes.txt");
        assert!(rec.input.attachments[0].sha256.is_some());
        let user = user_text(&llm.seen()[0]);
        let p_req = pos(&user, "summarize");
        let p_mat = pos(&user, "<material name=\"notes.txt\">");
        let p_stdin = pos(&user, "<stdin>");
        assert!(p_req < p_mat && p_mat < p_stdin);
        assert!(user.contains("line two"));
        assert!(user.contains("piped material"));

        // 磁盘记录 + 列表 + 状态
        let store = env.store();
        let on_disk = store.read_record(&rec.run_id).unwrap();
        assert_eq!(on_disk.status, RunStatus::Completed);
        assert!(
            on_disk.pending_input.is_none(),
            "pending input cleared after first snapshot"
        );
        assert!(store.list_snapshots(&rec.run_id).unwrap().len() >= 2);
        let listed = list_runs(&store, Some(&env.workdir), 20).unwrap();
        assert_eq!(listed.len(), 1);
        assert!(listed[0].is_terminal && !listed[0].resumable);
        assert!(list_runs(&store, Some(&env.project()), 20)
            .unwrap()
            .is_empty());
        let (_, summary) = load_run(&store, &rec.run_id).unwrap();
        let view = build_result_view(&on_disk, &summary, None);
        assert_eq!(view.answer, Some(Value::String("the answer".into())));
        assert!(view.resume_command.is_none());
        assert_eq!(
            export_result(&on_disk, Some(&ResultFormat::Raw))
                .unwrap()
                .to_output_text(),
            "the answer"
        );
        assert!(store.read_record("missing").is_err());
    }

    #[tokio::test]
    async fn stdin_becomes_request_when_nothing_else_and_empty_pipe_fails() {
        let env = Env::new();
        let llm = ScriptedLlm::new(vec![text("ok")]);
        let rec = env
            .run(
                TaskInput {
                    stdin: Some("explain event-driven\n".into()),
                    ..Default::default()
                },
                env.overrides(),
                llm.clone(),
            )
            .await
            .unwrap()
            .record()
            .clone();
        assert_eq!(rec.input.request_source, "stdin");
        assert_eq!(rec.input.request.as_deref(), Some("explain event-driven"));
        assert!(!user_text(&llm.seen()[0]).contains("<stdin>"));

        let err = env
            .run(
                TaskInput {
                    user: Some("q".into()),
                    stdin: Some("   \n".into()),
                    ..Default::default()
                },
                env.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, XllmError::Input(_)), "{err}");
        let err = env
            .run(
                TaskInput::default(),
                env.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("no task request"), "{err}");
        let err = env
            .run(
                TaskInput::default().with_file("nope.txt"),
                env.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string()
                .contains("attachments were given without a task request"),
            "{err}"
        );
        let err = env
            .run(
                TaskInput::question("q").with_file("nope.txt"),
                env.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("nope.txt"), "{err}");
    }

    #[tokio::test]
    async fn transient_provider_error_pauses_and_resume_completes_same_run() {
        let env = Env::new();
        let llm = ScriptedLlm::new(vec![Err(LLMComputeError::provider(
            ProviderFailure::Transient,
            "aicc down",
        ))]);
        let outcome = env
            .run(TaskInput::question("hello"), env.overrides(), llm.clone())
            .await
            .unwrap();
        let rec = outcome.record().clone();
        assert!(matches!(outcome, RunOutcome::Paused(_)));
        assert_eq!(rec.status, RunStatus::Paused);
        let err = rec.last_error.clone().unwrap();
        assert!(err.recoverable);
        assert_eq!(err.kind, "provider_transient");
        assert!(rec.latest_snapshot_idx.is_some());
        let store = env.store();
        let listed = list_runs(&store, Some(&env.workdir), 5).unwrap();
        assert!(listed[0].resumable && !listed[0].is_terminal);

        // 第二次 resume 仍失败：仍可恢复，不因次数变终态。
        let llm2 = ScriptedLlm::new(vec![Err(LLMComputeError::Timeout)]);
        let start = XllmRun::resume(
            &store,
            None,
            Some(&env.workdir),
            ResumeLimits::default(),
            env.deps(llm2),
        )
        .await
        .unwrap();
        let ResumeStart::Run(mut run) = start else {
            panic!("expected run")
        };
        assert_eq!(run.run_id(), rec.run_id);
        let o2 = run.execute().await.unwrap();
        assert!(matches!(o2, RunOutcome::Paused(_)));
        assert_eq!(
            o2.record().last_error.as_ref().unwrap().kind,
            "provider_timeout"
        );
        drop(run);

        // 服务恢复：同一 run_id 完成，原始输入沿用。
        let llm3 = ScriptedLlm::new(vec![text("recovered")]);
        let start = XllmRun::resume(
            &store,
            Some(&rec.run_id),
            None,
            ResumeLimits::default(),
            env.deps(llm3.clone()),
        )
        .await
        .unwrap();
        let ResumeStart::Run(mut run) = start else {
            panic!("expected run")
        };
        let o3 = run.execute().await.unwrap();
        assert!(matches!(o3, RunOutcome::Completed(_)));
        assert_eq!(o3.record().run_id, rec.run_id);
        assert_eq!(o3.record().result.as_ref().unwrap().raw, "recovered");
        assert!(user_text(&llm3.seen()[0]).contains("hello"));
        assert!(llm3.seen()[0]
            .messages
            .iter()
            .any(|m| m.role == AiRole::System));
        drop(run);

        // 终态 resume 只返回记录；带限制参数报错。
        match XllmRun::resume(
            &store,
            Some(&rec.run_id),
            None,
            ResumeLimits::default(),
            env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap()
        {
            ResumeStart::Terminal(r) => assert_eq!(r.status, RunStatus::Completed),
            ResumeStart::Run(_) => panic!("terminal run must not re-run"),
        }
        let err = XllmRun::resume(
            &store,
            Some(&rec.run_id),
            None,
            ResumeLimits {
                max_rounds: Some(20),
                ..Default::default()
            },
            env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, XllmError::RunTerminal { .. }), "{err}");
        // 没有未完成任务时 resume 明确失败。
        let err = XllmRun::resume(
            &store,
            None,
            Some(&env.workdir),
            ResumeLimits::default(),
            env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, XllmError::NotResumable { .. }), "{err}");
    }

    #[tokio::test]
    async fn permanent_and_credential_errors_are_classified() {
        let env = Env::new();
        let llm = ScriptedLlm::new(vec![Err(LLMComputeError::provider(
            ProviderFailure::Permanent,
            "model xyz not found",
        ))]);
        let o = env
            .run(TaskInput::question("q"), env.overrides(), llm)
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::Failed(_)));
        assert!(!o.record().last_error.as_ref().unwrap().recoverable);

        let llm = ScriptedLlm::new(vec![Err(LLMComputeError::provider(
            ProviderFailure::Permanent,
            "invalid token: expired",
        ))]);
        let o = env
            .run(TaskInput::question("q"), env.overrides(), llm)
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::Paused(_)));
        let e = o.record().last_error.clone().unwrap();
        assert_eq!(e.kind, "credentials");
        assert!(e.condition.unwrap().contains("credential"));

        let llm = ScriptedLlm::new(vec![Err(LLMComputeError::provider(
            ProviderFailure::Unknown,
            "?",
        ))]);
        let o = env
            .run(TaskInput::question("q"), env.overrides(), llm)
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::Failed(_)));
    }

    #[tokio::test]
    async fn tool_round_limit_ends_in_limit_reached_and_artifacts_are_tracked() {
        let env = Env::new();
        let llm = ScriptedLlm::new(vec![
            tool_call("write_file", json!({"path":"out.txt","content":"x"}), "c1"),
            tool_call("exec", json!({"command":"echo hi"}), "c2"),
            text("never reached"),
        ]);
        let overrides = TaskOverrides {
            tools: Some(true),
            max_rounds: Some(1),
            ..env.overrides()
        };
        let o = env
            .run(
                TaskInput::question("write then run"),
                overrides,
                llm.clone(),
            )
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::LimitReached(_)), "{:?}", o.status());
        let rec = o.record();
        assert!(rec.limit_reason.as_ref().unwrap().contains("round"));
        assert_eq!(rec.artifacts.len(), 1);
        assert!(rec.artifacts[0].ends_with("out.txt"));
        assert_eq!(
            std::fs::read_to_string(env.workdir.join("out.txt")).unwrap(),
            "x"
        );
        assert_eq!(
            llm.calls(),
            2,
            "one tool round then the limit stops before the next tool"
        );
        assert!(
            !llm.seen()[1].allow_tool_calls,
            "no rounds left => no tools advertised"
        );
        // 终态：resume 只展示。
        match XllmRun::resume(
            &env.store(),
            Some(&rec.run_id),
            None,
            ResumeLimits::default(),
            env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap()
        {
            ResumeStart::Terminal(r) => assert_eq!(r.status, RunStatus::LimitReached),
            _ => panic!(),
        }
    }

    #[tokio::test]
    async fn function_call_tool_loop_completes_with_observation() {
        let env = Env::new();
        let llm = ScriptedLlm::new(vec![
            tool_call("exec", json!({"command":"echo tool-output-42"}), "c1"),
            text("saw 42"),
        ]);
        let overrides = TaskOverrides {
            tools: Some(true),
            ..env.overrides()
        };
        let o = env
            .run(TaskInput::question("run it"), overrides, llm.clone())
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::Completed(_)));
        let second = &llm.seen()[1];
        let joined: String = second
            .messages
            .iter()
            .map(|m| serde_json::to_string(m).unwrap())
            .collect();
        assert!(joined.contains("tool-output-42"));
        assert_eq!(o.record().usage.llm_requests, 2);
    }

    #[tokio::test]
    async fn behavior_loop_end_to_end_with_tools2actions() {
        let env = Env::new();
        env.write(
            "project/.llm_context",
            "loop_model: behavior\nresult_format: result.report\ntools:\n  enabled: true\n  tools2actions: true\n",
        );
        let llm = ScriptedLlm::new(vec![
            text("<response><thinking>run</thinking><actions><exec><![CDATA[echo behavior-77]]></exec></actions></response>"),
            text("<response><observation>saw it</observation><report><![CDATA[{\"report\":\"结论 77\"}]]></report></response>"),
        ]);
        let o = env
            .run(TaskInput::question("do it"), env.overrides(), llm.clone())
            .await
            .unwrap();
        assert!(
            matches!(o, RunOutcome::Completed(_)),
            "{:?}",
            o.record().last_error
        );
        let rec = o.record();
        assert_eq!(rec.config.loop_model, LoopModel::Behavior);
        assert!(rec.config.tools.native.is_empty());
        assert_eq!(rec.config.tools.actions.len(), 4);
        assert!(rec.config.tools.exec_enabled);
        let sys = system_text(&llm.seen()[0]);
        assert!(sys.contains("Available actions:"));
        assert!(sys.contains("<exec"), "{sys}");
        assert!(sys.contains("Behavior actions available in this run"));
        assert!(!llm.seen()[0].allow_tool_calls || llm.seen()[0].tool_names.is_empty());
        let second = user_text(&llm.seen()[1]);
        assert!(
            second.contains("behavior-77"),
            "action result must be rendered back:\n{second}"
        );
        let res = rec.result.as_ref().unwrap();
        assert!(res.raw.starts_with("<response>"));
        assert_eq!(
            res.extracted,
            Some(ExtractedValue::Text {
                text: "{\"report\":\"结论 77\"}".into()
            })
        );
        // 从原文换一种提取方式
        assert_eq!(
            export_result(
                rec,
                Some(&ResultFormat::parse("result.observation").unwrap())
            )
            .unwrap()
            .to_output_text(),
            "saw it"
        );
    }

    #[tokio::test]
    async fn behavior_file_actions_respect_arguments_and_persist_round_limit() {
        let env = Env::new();
        env.write(
            "project/.llm_context",
            "loop_model: behavior\nmax_rounds: 2\ntools:\n  enabled: true\n  tools2actions: true\n",
        );
        std::fs::write(env.workdir.join("fixture.txt"), "behavior-fixture-3142").unwrap();
        let llm = ScriptedLlm::new(vec![
            text(r#"<response><actions><read_file path="fixture.txt"><![CDATA[]]></read_file></actions></response>"#),
            text("<response><actions><read_file><![CDATA[missing.txt]]></read_file></actions></response>"),
            text(r#"<response><actions><write_file path="excess.txt"><![CDATA[excess]]></write_file></actions><next_behavior>END</next_behavior></response>"#),
        ]);
        let outcome = env
            .run(
                TaskInput::question("read fixture"),
                env.overrides(),
                llm.clone(),
            )
            .await
            .unwrap();
        assert!(
            matches!(outcome, RunOutcome::LimitReached(_)),
            "{:?}",
            outcome.status()
        );
        assert_eq!(llm.calls(), 3);
        assert!(user_text(&llm.seen()[1]).contains("behavior-fixture-3142"));
        assert!(!env.workdir.join("excess.txt").exists());
        let record = outcome.record();
        assert!(record.limit_reason.as_ref().unwrap().contains("round"));
        let snapshot = env
            .store()
            .get_snapshot(&record.run_id, record.latest_snapshot_idx.unwrap())
            .unwrap();
        assert_eq!(snapshot.state.rounds_left, 0);
        let steps: Vec<_> = snapshot
            .state
            .steps
            .iter()
            .chain(snapshot.state.last_step.iter())
            .collect();
        assert_eq!(steps.len(), 2);
        assert!(matches!(
            steps[0].action_results[0],
            Observation::Success { .. }
        ));
        assert!(matches!(
            steps[1].action_results[0],
            Observation::Error { .. }
        ));
    }

    #[tokio::test]
    async fn json_flag_is_validated_after_extraction() {
        let env = Env::new();
        let o = env
            .run(
                TaskInput::question("q"),
                TaskOverrides {
                    json: true,
                    ..env.overrides()
                },
                ScriptedLlm::new(vec![text("not json")]),
            )
            .await
            .unwrap();
        let res = o.record().result.clone().unwrap();
        assert_eq!(res.json_valid, Some(false));
        assert!(res.json_error.is_some());
        let o = env
            .run(
                TaskInput::question("q"),
                TaskOverrides {
                    json: true,
                    ..env.overrides()
                },
                ScriptedLlm::new(vec![text("{\"a\": 1}")]),
            )
            .await
            .unwrap();
        assert_eq!(o.record().result.as_ref().unwrap().json_valid, Some(true));
        let err = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("q"),
            TaskOverrides {
                json: true,
                loop_model: Some(LoopModel::Behavior),
                ..env.overrides()
            },
            &env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, XllmError::Capability(_)), "{err}");
    }

    #[tokio::test]
    async fn runs_dir_none_uses_memory_store() {
        let env = Env::new();
        env.write("project/.llm_context", "runs_dir: none\n");
        let deps = env.deps(ScriptedLlm::new(vec![text("ok")]));
        let prepared = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("q"),
            TaskOverrides::default(),
            &deps,
        )
        .await
        .unwrap();
        assert!(!prepared.store.is_persistent());
        let mut run = XllmRun::start(prepared, deps).await.unwrap();
        let o = run.execute().await.unwrap();
        assert!(matches!(o, RunOutcome::Completed(_)));
        assert!(!env.runs_dir.exists());
    }

    #[tokio::test]
    async fn tool_enabled_runs_are_exclusive_per_workdir() {
        let env = Env::new();
        let deps_a = env.deps(ScriptedLlm::new(vec![]));
        let prepared = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("a"),
            TaskOverrides {
                tools: Some(true),
                ..env.overrides()
            },
            &deps_a,
        )
        .await
        .unwrap();
        let run_a = XllmRun::start(prepared, deps_a).await.unwrap();

        let deps_b = env.deps(ScriptedLlm::new(vec![]));
        let prepared_b = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("b"),
            TaskOverrides {
                tools: Some(true),
                ..env.overrides()
            },
            &deps_b,
        )
        .await
        .unwrap();
        let err = XllmRun::start(prepared_b, deps_b).await.unwrap_err();
        match err {
            XllmError::WorkdirBusy { run_id, .. } => assert_eq!(run_id, run_a.run_id()),
            other => panic!("unexpected: {other}"),
        }
        // 无工具任务可并行。
        let deps_c = env.deps(ScriptedLlm::new(vec![text("c")]));
        let prepared_c = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("c"),
            env.overrides(),
            &deps_c,
        )
        .await
        .unwrap();
        let mut run_c = XllmRun::start(prepared_c, deps_c).await.unwrap();
        assert!(matches!(
            run_c.execute().await.unwrap(),
            RunOutcome::Completed(_)
        ));
        // 同一 Run 不能被另一个进程/句柄重复推进。
        let err = XllmRun::resume(
            &env.store(),
            Some(run_a.run_id()),
            None,
            ResumeLimits::default(),
            env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, XllmError::RunBusy { .. }), "{err}");
        // 记录仍是 running 且进程活着 → 列表显示执行中。
        let listed = list_runs(&env.store(), Some(&env.workdir), 10).unwrap();
        let a = listed.iter().find(|s| s.run_id == run_a.run_id()).unwrap();
        assert_eq!(a.status, RunStatus::Running);
        assert!(!a.stale_running);
        drop(run_a);
        // 锁释放后：残留的 running 记录被识别为已中断、可恢复。
        let listed = list_runs(&env.store(), Some(&env.workdir), 10).unwrap();
        let a = listed.iter().find(|s| s.run_id != run_c.run_id()).unwrap();
        assert_eq!(a.status, RunStatus::Interrupted);
        assert!(a.stale_running && a.resumable);
        drop(run_c);
        // 从 pending_input 恢复（尚无快照）。
        let llm = ScriptedLlm::new(vec![text("resumed a")]);
        let ResumeStart::Run(mut run) = XllmRun::resume(
            &env.store(),
            Some(&a.run_id),
            None,
            ResumeLimits::default(),
            env.deps(llm.clone()),
        )
        .await
        .unwrap() else {
            panic!()
        };
        let o = run.execute().await.unwrap();
        assert!(matches!(o, RunOutcome::Completed(_)));
        assert!(user_text(&llm.seen()[0]).contains('a'));
    }

    #[tokio::test]
    async fn interrupt_is_saved_and_resumable() {
        let env = Env::new();
        let deps = env.deps(ScriptedLlm::new(vec![text("late")]));
        let prepared = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("q"),
            env.overrides(),
            &deps,
        )
        .await
        .unwrap();
        let mut run = XllmRun::start(prepared, deps).await.unwrap();
        run.interrupter().interrupt("user interrupt (test)");
        let o = run.execute().await.unwrap();
        assert!(matches!(o, RunOutcome::Interrupted(_)));
        assert_eq!(
            o.record().interrupt_reason.as_deref(),
            Some("user interrupt (test)")
        );
        drop(run);
        let llm = ScriptedLlm::new(vec![text("after")]);
        let ResumeStart::Run(mut run) = XllmRun::resume(
            &env.store(),
            None,
            Some(&env.workdir),
            ResumeLimits::default(),
            env.deps(llm),
        )
        .await
        .unwrap() else {
            panic!()
        };
        let o = run.execute().await.unwrap();
        assert_eq!(o.record().result.as_ref().unwrap().raw, "after");
    }

    #[tokio::test]
    async fn file_model_stage_feeds_analysis_to_main_model() {
        let env = Env::new();
        env.write("project/.llm_context", "file_model: llm.vision\n");
        let png = env.workdir.join("shot.png");
        std::fs::write(&png, b"\x89PNG\r\n\x1a\nfake").unwrap();
        let llm = ScriptedLlm::new(vec![text("a red button"), text("it is red")]);
        let input = TaskInput::question("what colour?").with_image(png.display().to_string());
        let o = env.run(input, env.overrides(), llm.clone()).await.unwrap();
        assert!(matches!(o, RunOutcome::Completed(_)));
        let rec = o.record();
        let seen = llm.seen();
        assert_eq!(seen[0].model, "llm.vision");
        assert!(seen[0].messages.iter().any(|m| m
            .content
            .iter()
            .any(|c| matches!(c, AiContent::Image { .. }))));
        assert!(user_text(&seen[0]).contains("what colour?"));
        assert_eq!(seen[1].model, "llm.chat");
        assert!(!seen[1].messages.iter().any(|m| m
            .content
            .iter()
            .any(|c| matches!(c, AiContent::Image { .. }))));
        assert!(user_text(&seen[1]).contains("<image_analysis model=\"llm.vision\">"));
        assert!(user_text(&seen[1]).contains("a red button"));
        assert_eq!(
            rec.file_model_stage.as_ref().unwrap().analysis,
            "a red button"
        );
        assert_eq!(rec.usage.llm_requests, 2);
        assert_eq!(rec.input.attachments[0].mime.as_deref(), Some("image/png"));

        // 未配置 file_model：主模型直接收到图片。
        let env2 = Env::new();
        let png2 = env2.workdir.join("s.jpg");
        std::fs::write(&png2, b"\xff\xd8\xff").unwrap();
        let llm2 = ScriptedLlm::new(vec![text("ok")]);
        env2.run(
            TaskInput::question("q").with_image(png2.display().to_string()),
            env2.overrides(),
            llm2.clone(),
        )
        .await
        .unwrap();
        assert!(llm2.seen()[0].messages.iter().any(|m| m
            .content
            .iter()
            .any(|c| matches!(c, AiContent::Image { .. }))));
        // 不支持的格式 / 不存在的图片
        let err = env2
            .run(
                TaskInput::question("q").with_image("x.gif"),
                env2.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("unsupported image format"),
            "{err}"
        );

        // 文件阶段失败（可恢复）→ 暂停；resume 重做文件阶段。
        let env3 = Env::new();
        env3.write("project/.llm_context", "file_model: llm.vision\n");
        let png3 = env3.workdir.join("shot.png");
        std::fs::write(&png3, b"\x89PNG\r\n\x1a\nfake").unwrap();
        let llm3 = ScriptedLlm::new(vec![Err(LLMComputeError::Timeout)]);
        let o = env3
            .run(
                TaskInput::question("q").with_image(png3.display().to_string()),
                env3.overrides(),
                llm3,
            )
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::Paused(_)));
        assert_eq!(o.record().last_error.as_ref().unwrap().phase, "file_model");
        std::fs::remove_file(&png3).unwrap(); // 原图消失也不影响恢复
        let llm4 = ScriptedLlm::new(vec![text("analysis"), text("final")]);
        let ResumeStart::Run(mut run) = XllmRun::resume(
            &env3.store(),
            None,
            Some(&env3.workdir),
            ResumeLimits::default(),
            env3.deps(llm4.clone()),
        )
        .await
        .unwrap() else {
            panic!()
        };
        let o = run.execute().await.unwrap();
        assert!(matches!(o, RunOutcome::Completed(_)));
        assert_eq!(llm4.seen()[0].model, "llm.vision");
        assert_eq!(o.record().result.as_ref().unwrap().raw, "final");
    }

    #[tokio::test]
    async fn structured_input_gets_protocol_and_skips_group_defaults() {
        let env = Env::new();
        env.write("project/.llm_context", PARENT_CONFIG);
        env.write("project/checkpr.yaml", CHECKPR_GROUP);
        let llm = ScriptedLlm::new(vec![text("{\"x\":1}")]);
        let input = TaskInput::structured(vec![
            AiMessage::text(AiRole::System, "custom sys"),
            AiMessage::text(AiRole::User, "custom user"),
        ]);
        let o = env
            .run(
                input,
                TaskOverrides {
                    json: true,
                    ..env.overrides()
                },
                llm.clone(),
            )
            .await
            .unwrap();
        assert!(matches!(o, RunOutcome::Completed(_)));
        let seen = llm.seen();
        assert!(seen[0].force_json);
        assert_eq!(seen[0].messages.len(), 3);
        assert!(seen[0].messages[0]
            .text_content()
            .contains("## runtime_protocol"));
        assert_eq!(seen[0].messages[1].text_content(), "custom sys");
        assert!(!seen[0].messages[0]
            .text_content()
            .contains("You are a code reviewer"));
        assert!(
            !o.record().config.tools.enabled,
            "unselected group tools do not apply"
        );
        assert_eq!(o.record().input.request_source, "structured");
        let err = env
            .run(
                TaskInput {
                    structured: Some(vec![]),
                    user: Some("x".into()),
                    ..Default::default()
                },
                env.overrides(),
                ScriptedLlm::new(vec![]),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("mutually exclusive"), "{err}");
    }

    #[tokio::test]
    async fn resume_limits_raise_rounds_without_resetting_consumed() {
        for behavior in [false, true] {
            let env = Env::new();
            if behavior {
                env.write(
                    "project/.llm_context",
                    "loop_model: behavior\ntools:\n  tools2actions: true\n",
                );
            }
            let llm = ScriptedLlm::new(vec![
                if behavior {
                    text("<response><actions><exec><![CDATA[echo one]]></exec></actions></response>")
                } else {
                    tool_call("exec", json!({"command":"echo one"}), "c1")
                },
                Err(LLMComputeError::provider(ProviderFailure::Transient, "x")),
            ]);
            let o = env
                .run(
                    TaskInput::question("q"),
                    TaskOverrides {
                        tools: Some(true),
                        max_rounds: Some(2),
                        ..env.overrides()
                    },
                    llm,
                )
                .await
                .unwrap();
            assert!(matches!(o, RunOutcome::Paused(_)));
            let run_id = o.record().run_id.clone();
            let snap = env
                .store()
                .get_snapshot(&run_id, o.record().latest_snapshot_idx.unwrap())
                .unwrap();
            assert_eq!(snap.state.rounds_left, 1);
            let ResumeStart::Run(run) = XllmRun::resume(
                &env.store(),
                Some(&run_id),
                None,
                ResumeLimits {
                    max_rounds: Some(5),
                    timeout_secs: Some(42),
                    ..Default::default()
                },
                env.deps(ScriptedLlm::new(vec![])),
            )
            .await
            .unwrap() else {
                panic!()
            };
            let rec = run.record();
            assert_eq!(rec.config.limits.max_rounds, 5);
            assert_eq!(rec.config.limits.timeout_secs, 42);
            // 新快照会在下一次推理前提交；这里直接检查内存中的上下文状态。
            let s = run.ctx.as_ref().unwrap().snapshot();
            assert_eq!(s.state.rounds_left, 4, "consumed round is not refunded");
            assert_eq!(s.request.budget.max_wallclock_ms, Some(42_000));
        }
    }

    #[test]
    fn xml_scanner_handles_nesting_cdata_and_self_closing() {
        let els = scan_xml_elements(
            "<a x=\"1\" y='2'><b><![CDATA[<a>not a tag</a>]]></b><a>inner</a><c/></a><d/>",
        );
        assert_eq!(els.len(), 2);
        assert_eq!(els[0].name, "a");
        assert_eq!(els[0].attr("x"), Some("1"));
        assert_eq!(els[0].attr("y"), Some("2"));
        let kids = els[0].children();
        assert_eq!(
            kids.iter().map(|k| k.name.as_str()).collect::<Vec<_>>(),
            vec!["b", "a", "c"]
        );
        assert_eq!(kids[0].text(), "<a>not a tag</a>");
        assert!(kids[2].self_closing);
        assert_eq!(els[1].name, "d");
        // 缺失闭合标签容忍
        let els = scan_xml_elements("<report>unterminated");
        assert_eq!(els[0].text(), "unterminated");
        assert_eq!(strip_code_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(element_text("a &lt;b&gt; <![CDATA[c<]]> d"), "a <b> c< d");
    }

    #[test]
    fn config_path_resolution_and_run_ids() {
        let base = Path::new("/base/dir");
        assert_eq!(
            resolve_config_path("./x/../y", base),
            PathBuf::from("/base/dir/y")
        );
        assert_eq!(resolve_config_path("/abs", base), PathBuf::from("/abs"));
        let home = home_dir().unwrap();
        assert_eq!(
            resolve_config_path("~/.xllm/runs", base),
            home.join(".xllm/runs")
        );
        let a = generate_run_id();
        let b = generate_run_id();
        assert_ne!(a, b);
        assert_eq!(a.len(), "20260101-000000-abcdef".len());
        assert_eq!(section_default_name(20), "contexts");
        assert_eq!(resolve_section_key("env").unwrap(), 20);
        assert_eq!(resolve_section_key("110").unwrap(), 110);
    }

    #[tokio::test]
    async fn cli_overrides_beat_files_and_untouched_fields_inherit() {
        let env = Env::new();
        env.write(
            "project/.llm_context",
            "model: file-model\nmax_rounds: 4\nllm_timeout: 7\nresult_format: result.report\nrun_logs: warn\n",
        );
        let llm = ScriptedLlm::new(vec![text("{\"report\":\"r\"}")]);
        let o = env
            .run(
                TaskInput::question("q"),
                TaskOverrides {
                    model: Some("cli-model".into()),
                    ..env.overrides()
                },
                llm.clone(),
            )
            .await
            .unwrap();
        let rec = o.record();
        assert_eq!(rec.config.model, "cli-model");
        assert_eq!(
            rec.config.sources.get("model").map(String::as_str),
            Some("cli")
        );
        assert_eq!(rec.config.limits.max_rounds, 4);
        assert_eq!(rec.config.limits.llm_timeout_secs, 7);
        assert_eq!(rec.config.limits.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert_eq!(rec.config.run_logs, RunLogLevel::Warn);
        assert_eq!(rec.config.result_format.as_string(), "result.report");
        assert_eq!(llm.seen()[0].model, "cli-model");
        assert_eq!(
            rec.result.as_ref().unwrap().extracted,
            Some(ExtractedValue::Text { text: "r".into() })
        );
        assert!(rec
            .config
            .sources
            .get("max_rounds")
            .unwrap()
            .ends_with(".llm_context"));
    }

    #[test]
    fn filesystem_policy_config_precedence_and_validation() {
        let env = Env::new();
        env.write(
            "project/.llm_context",
            "tools:\n  filesystem_policy: unrestricted\nprompt:\n  groups:\n    general:\n      tools:\n        filesystem_policy: workspace\n",
        );
        env.write("project/src/.llm_context", "tools:\n  enabled: true\n");
        let merged = merge_config_layers(&load_config_layers(&env.workdir).unwrap()).unwrap();
        let group = merged.prompt.groups.get("general");
        let (top, sources) = compute_tools_config(&merged, None, None, true);
        assert_eq!(top.filesystem_policy, Some(FilesystemPolicy::Unrestricted));
        assert_eq!(top.enabled, Some(true));
        assert_eq!(
            sources["filesystem_policy"],
            env.project().join(".llm_context").to_str().unwrap()
        );
        let (selected, sources) = compute_tools_config(&merged, group, None, true);
        assert_eq!(
            selected.filesystem_policy,
            Some(FilesystemPolicy::Workspace)
        );
        assert_eq!(sources["filesystem_policy"], "selected group");
        env.write(
            "project/src/.llm_context",
            "prompt:\n  tools:\n    filesystem_policy: unrestricted\n",
        );
        let merged = merge_config_layers(&load_config_layers(&env.workdir).unwrap()).unwrap();
        let (cfg, sources) = compute_tools_config(
            &merged,
            merged.prompt.groups.get("general"),
            Some(false),
            true,
        );
        assert_eq!(cfg.filesystem_policy, Some(FilesystemPolicy::Unrestricted));
        assert_eq!(cfg.enabled, Some(false));
        assert!(sources["filesystem_policy"].ends_with("src/.llm_context"));
        for raw in [
            "tools:\n  filesystem_policy: invalid\n",
            "prompt:\n  tools:\n    filesystem_policy: false\n",
            "prompt:\n  groups:\n    general:\n      tools:\n        filesystem_policy: invalid\n",
        ] {
            let err = parse_llm_context_file(Path::new(".llm_context"), raw).unwrap_err();
            assert!(matches!(err, XllmError::Config { .. }), "{err}");
            assert!(err.to_string().contains("filesystem_policy"), "{err}");
        }
    }

    #[tokio::test]
    async fn filesystem_policy_controls_builtin_paths() {
        for loop_model in [LoopModel::FunctionCall, LoopModel::Behavior] {
            for policy in [None, Some(FilesystemPolicy::Unrestricted)] {
                let env = Env::new();
                env.write("project/fixture.txt", "outside-original");
                let cfg = ToolsConfig {
                    enabled: Some(true),
                    filesystem_policy: policy,
                    tools2actions: Some(loop_model == LoopModel::Behavior),
                    ..Default::default()
                };
                let (effective, manager) = build_toolset(
                    &cfg,
                    BTreeMap::new(),
                    loop_model,
                    &env.workdir,
                    "test-policy",
                    &env.deps(ScriptedLlm::new(vec![])),
                )
                .await
                .unwrap();
                assert_eq!(effective.filesystem_policy, policy.unwrap_or_default());
                let unrestricted = policy == Some(FilesystemPolicy::Unrestricted);
                let outside = env.project();
                let calls = [
                    ("read_file", json!({"path": outside.join("fixture.txt")})),
                    ("read_file", json!({"path": "../fixture.txt"})),
                    (
                        "write_file",
                        json!({"path": outside.join("created.txt"), "content": "created"}),
                    ),
                    (
                        "edit_file",
                        json!({"path": "../fixture.txt", "old_string": "outside-original", "new_string": "outside-edited"}),
                    ),
                    (TOOL_EXEC, json!({"command": "pwd", "cwd": outside})),
                    (TOOL_EXEC, json!({"command": "pwd", "cwd": ".."})),
                ];
                for (name, args) in calls {
                    let obs = manager
                        .call_tool(AiToolCall {
                            name: name.into(),
                            args: serde_json::from_value(args).unwrap(),
                            call_id: "test-call".into(),
                        })
                        .await
                        .unwrap();
                    if unrestricted {
                        let Observation::Success { content, .. } = obs else {
                            panic!("{name}: {obs:?}");
                        };
                        let content = content.as_str().expect("text observation");
                        if name == "read_file" {
                            assert!(content.contains("outside-original"), "{content}");
                        }
                        if name == TOOL_EXEC {
                            assert!(content.contains(outside.to_str().unwrap()), "{content}");
                        }
                    } else {
                        let Observation::Error { message, .. } = obs else {
                            panic!("{name} escaped workspace: {obs:?}");
                        };
                        assert!(
                            message.contains("policy") || message.contains("workspace scope"),
                            "{message}"
                        );
                    }
                }
                assert_eq!(outside.join("created.txt").exists(), unrestricted);
                assert_eq!(
                    std::fs::read_to_string(outside.join("fixture.txt")).unwrap(),
                    if unrestricted {
                        "outside-edited"
                    } else {
                        "outside-original"
                    },
                );
                let obs = manager
                    .call_tool(AiToolCall {
                        name: "write_file".into(),
                        args: serde_json::from_value(
                            json!({"path": "local.txt", "content": "local"}),
                        )
                        .unwrap(),
                        call_id: "local-call".into(),
                    })
                    .await
                    .unwrap();
                assert!(matches!(obs, Observation::Success { .. }), "{obs:?}");
                assert_eq!(
                    std::fs::read_to_string(env.workdir.join("local.txt")).unwrap(),
                    "local"
                );
                let obs = manager.call_tool(exec_call("pwd")).await.unwrap();
                let Observation::Success { content, .. } = obs else {
                    panic!("{obs:?}")
                };
                let content = content.as_str().expect("text observation");
                assert!(content.contains(env.workdir.to_str().unwrap()), "{content}");
            }
        }
    }

    #[tokio::test]
    async fn documented_filesystem_policy_survives_resume() {
        let env = Env::new();
        let template = include_str!("../../../../product/xllm/PRD.md")
            .split("#### 4.9.1 ")
            .nth(1)
            .unwrap()
            .split("```yaml\n")
            .nth(1)
            .unwrap()
            .split("```")
            .next()
            .unwrap();
        env.write("project/.llm_context", template);
        let disabled = XllmTask::prepare(
            &env.workdir,
            TaskInput::question("q"),
            TaskOverrides {
                tools: Some(false),
                ..env.overrides()
            },
            &env.deps(ScriptedLlm::new(vec![])),
        )
        .await
        .unwrap();
        assert!(!disabled.tools.enabled);
        assert!(disabled.tools.native.is_empty());
        assert!(disabled.manager.tools.is_empty());
        let llm = ScriptedLlm::new(vec![
            tool_call(
                "write_file",
                json!({"path": "../shared.txt", "content": "before-resume"}),
                "write",
            ),
            Err(LLMComputeError::provider(
                ProviderFailure::Transient,
                "temporary outage",
            )),
        ]);
        let outcome = env
            .run(
                TaskInput::question("edit shared file"),
                env.overrides(),
                llm.clone(),
            )
            .await
            .unwrap();
        assert!(matches!(outcome, RunOutcome::Paused(_)), "{outcome:?}");
        let record = outcome.record();
        assert_eq!(
            record.config.tools.filesystem_policy,
            FilesystemPolicy::Unrestricted
        );
        assert!(system_text(&llm.seen()[0]).contains("may access paths outside"));
        assert_eq!(
            std::fs::read_to_string(env.project().join("shared.txt")).unwrap(),
            "before-resume"
        );
        env.write(
            "project/.llm_context",
            "tools:\n  enabled: true\n  filesystem_policy: workspace\n",
        );
        let llm = ScriptedLlm::new(vec![
            tool_call(
                "edit_file",
                json!({"path": "../shared.txt", "old_string": "before-resume", "new_string": "after-resume"}),
                "edit",
            ),
            tool_call(
                TOOL_EXEC,
                json!({"command": "pwd > resumed-cwd.txt", "cwd": ".."}),
                "exec",
            ),
            text("done"),
        ]);
        let ResumeStart::Run(mut run) = XllmRun::resume(
            &env.store(),
            Some(&record.run_id),
            None,
            ResumeLimits::default(),
            env.deps(llm.clone()),
        )
        .await
        .unwrap() else {
            panic!("expected resumable run")
        };
        let outcome = run.execute().await.unwrap();
        assert!(matches!(outcome, RunOutcome::Completed(_)), "{outcome:?}");
        assert_eq!(
            outcome.record().config.tools.filesystem_policy,
            FilesystemPolicy::Unrestricted
        );
        assert_eq!(
            std::fs::read_to_string(env.project().join("shared.txt")).unwrap(),
            "after-resume"
        );
        assert_eq!(
            std::fs::read_to_string(env.project().join("resumed-cwd.txt"))
                .unwrap()
                .trim(),
            env.project().to_str().unwrap(),
        );
    }

    fn exec_manager(workdir: &Path) -> XllmToolManager {
        let mut manager =
            XllmToolManager::new(workdir.to_path_buf(), "run-test", LoopModel::FunctionCall);
        for t in builtin_bash_group(workdir, FilesystemPolicy::Workspace) {
            manager.register(t, "groupname:bash").expect("register");
        }
        manager
    }

    fn exec_call(command: &str) -> AiToolCall {
        AiToolCall {
            name: TOOL_EXEC.to_string(),
            args: HashMap::from([("command".to_string(), Value::String(command.to_string()))]),
            call_id: "c1".to_string(),
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_deadline_cancels_running_exec_and_kills_it() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("survived");
        let manager = exec_manager(dir.path());
        manager.set_deadline(1);
        let started = std::time::Instant::now();
        let obs = manager
            .call_tool(exec_call(&format!("sleep 2; touch {}", marker.display())))
            .await
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(2));
        match obs {
            Observation::Error { message, .. } => {
                assert!(
                    message.contains("total execution time limit (1s)"),
                    "{message}"
                )
            }
            other => panic!("unexpected {other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(2500)).await;
        assert!(!marker.exists(), "cancelled exec kept running");
    }

    #[tokio::test]
    async fn interrupt_cancels_running_exec() {
        let dir = tempfile::tempdir().unwrap();
        let manager = Arc::new(exec_manager(dir.path()));
        let interrupter = XllmInterrupter {
            requested: Arc::new(Mutex::new(None)),
            handle: Arc::new(Mutex::new(None)),
            tool_cancel: manager.cancel.clone(),
        };
        let m = manager.clone();
        let call = tokio::spawn(async move { m.call_tool(exec_call("sleep 30")).await });
        tokio::time::sleep(Duration::from_millis(200)).await;
        interrupter.interrupt("test");
        let obs = tokio::time::timeout(Duration::from_secs(5), call)
            .await
            .expect("cancel must be prompt")
            .unwrap()
            .unwrap();
        match obs {
            Observation::Error { message, .. } => {
                assert!(message.contains("run interrupted"), "{message}")
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[tokio::test]
    async fn failed_exec_observation_carries_output() {
        let dir = tempfile::tempdir().unwrap();
        let manager = exec_manager(dir.path());
        let obs = manager
            .call_tool(exec_call(
                "echo compile error: missing semicolon >&2; exit 2",
            ))
            .await
            .unwrap();
        match obs {
            Observation::Error { message, .. } => {
                assert!(message.starts_with("exit=2"), "{message}");
                assert!(message.contains("missing semicolon"), "{message}");
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn exec_spec_advertises_xllm_limits() {
        let dir = tempfile::tempdir().unwrap();
        let manager = exec_manager(dir.path());
        let spec = manager.tools.get(TOOL_EXEC).unwrap().spec();
        assert_eq!(
            spec.args_schema["properties"]["timeout_ms"]["maximum"],
            EXEC_MAX_TIMEOUT_MS
        );
        assert!(spec
            .description
            .contains("Default timeout 1800s, max 3600s"));
    }
}
