use std::env;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, load_app_identity_from_env,
    parse_typed_task_data, BuckyOSRuntimeType, CommitResultReq, FailTaskReq,
    RunnerWriteEnvelope, Task, TaskDataType, TaskError, TaskManagerClient, TaskOutcome, TaskPhase,
    ToolExecBashTaskData, TypedTaskData,
};
use kRPC::kRPC;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::io::{self, AsyncReadExt};
use tokio::process::Command;

use agent_did_object_lib::{
    AgentDIDObjectRuntime, ObjectRouteConfig, ReadInput as ObjectReadInput, ReadLineRange,
    XCallInput as ObjectXCallInput,
};
use agent_tool::agent_attention_signal::{
    CreateExtractionWindowInput, DiscoverEventArgs, DiscoverObjectObservationArgs,
    DiscoverRelationshipArgs, DiscoverSkillCoverageGapArgs, SignalLifecycleStatus,
};
use agent_tool::agent_memory::{
    AddObservationOp, AgentMemory, AgentMemoryConfig, AgentMemoryError, FlatSetOp, LoadOptions,
    ObjectAliasInput, OccasionAddInput, ReinforceObjectWeightOp, SetStatusOp, SourceRef,
    UpsertObjectOp, UpsertRelationOp,
};
use agent_tool::agent_notebook::{
    self as nb, AgentNotebook, AgentNotebookConfig, AppendItemRemarkInput, AppendNoteInput,
    BuildHintsInput, BuildRegistryContextInput, BuildSystemContextInput, Confidence,
    CreateOrUpdateNotebookInput, ListItemRemarksInput, ListNotebooksInput, MarkNoteStatusInput,
    NotebookError, NotebookItemStatus, NotebookKind, NotebookReadResult, PromoteToSystemInput,
    PromoteToSystemResult, ReadNotebookInput, RemoveItemRemarkInput, WriteReason,
};
use agent_tool::llm_tool_carft::{self, CommandNotFoundRequest};
use agent_tool::skills_mgr::{
    CreateCandidateInput as SkillCreateCandidateInput, LifecycleState as SkillLifecycleState,
    ListSkillsInput, OwnerScope as SkillOwnerScope,
    RenderSelectedInput as SkillRenderSelectedInput, RiskLevel as SkillRiskLevel, SkillSourceType,
    SkillTaskResult, SkillType, SkillsMgr, SkillsMgrConfig, SkillsMgrError,
    UsageMode as SkillUsageMode, UserFeedback as SkillUserFeedback,
    ValidateSelectionInput as SkillValidateSelectionInput, DEFAULT_MAX_SELECTED_SKILLS,
    DEFAULT_RENDER_TOKEN_BUDGET, SKILLS_DIR,
};
use agent_tool::{
    cli_error_result, cli_exit_code_for_error, cli_result_from_tool_result, cli_success_result,
    normalize_abs_path, now_ms, render_cli_output, session_record_path, AgentAttentionSignalStore,
    AgentToolError, AgentToolManager, AgentToolPendingReason, AgentToolResult, AgentToolStatus,
    AttentionSignalStoreConfig, AttentionSignalToolRuntime, BindWorkspaceTool, CliRunOutput,
    CreateWorkspaceTool, DcrontabTool, DiscoverEventTool, DiscoverObjectObservationTool,
    DiscoverRelationshipTool, DiscoverSkillCoverageGapTool, EditFileTool, FileToolConfig,
    GetSessionTool, GlobTool, GrepTool, NoopFileWriteAudit, ReadFileTool, RuntimeContext,
    SessionRuntimeContext, SessionViewBackend, TodoTool, TodoToolConfig, ToolCtx, TypedTool,
    WorkspaceToolBackend, WriteFileTool, DEFAULT_READ_TOKEN_LIMIT,
};
use agent_tool::{llm_explore, llm_understand_media, run_local_llm};
use chrono::{DateTime, Duration, Utc};
use opendan::buildin_tool::{
    AlreadyImprovedOutput, BeginAttentionSignalExtractionArgs,
    BeginAttentionSignalExtractionOutput, CommitSessionHistoryImprovedArgs,
    CommitSessionHistoryImprovedOutput, CompleteAttentionSignalExtractionArgs,
    CompleteAttentionSignalExtractionOutput, ListPendingAttentionSignalsArgs,
    ListPendingAttentionSignalsOutput, MarkAttentionSignalConsumedArgs,
    MarkAttentionSignalConsumedOutput, ReadSessionHistoryArgs, ReadSessionHistoryOutput,
    SessionHistoryMessageOutput,
};
use opendan::round_history::{
    SessionHistoryQuery, SessionHistoryReadOptions, SessionHistoryReader,
};
use opendan::session_model::{AlreadyImprovedState, SessionKind, SessionMeta};

const TOOL_CHECK_TASK: &str = "check_task";
const TOOL_CANCEL_TASK: &str = "cancel_task";
const TOOL_FINISH_TASK: &str = "finish_task";
const TOOL_READ_OBJECT: &str = "read";
const TOOL_X_CALL: &str = "x-call";
const TOOL_X_CALL_SNAKE: &str = "x_call";
const TOOL_AGENT_MEMORY: &str = "agent-memory";
const TOOL_AGENT_MEMORY_SNAKE: &str = "agent_memory";
const TOOL_AGENT_NOTEBOOK: &str = "agent-notebook";
const TOOL_AGENT_NOTEBOOK_SNAKE: &str = "agent_notebook";
const TOOL_AGENT_SKILLS: &str = "agent-skills";
const TOOL_AGENT_SKILLS_SNAKE: &str = "agent_skills";
const TOOL_READ_SESSION_HISTORY: &str = "read_session_history";
const TOOL_COMMIT_SESSION_HISTORY_IMPROVED: &str = "commit_session_history_improved";
const TOOL_BEGIN_ATTENTION_SIGNAL_EXTRACTION: &str = "BeginAttentionSignalExtraction";
const TOOL_COMPLETE_ATTENTION_SIGNAL_EXTRACTION: &str = "CompleteAttentionSignalExtraction";
const TOOL_LIST_PENDING_ATTENTION_SIGNALS: &str = "ListPendingAttentionSignals";
const TOOL_MARK_ATTENTION_SIGNAL_CONSUMED: &str = "MarkAttentionSignalConsumed";
const TOOL_NAMES: [&str; 32] = [
    "Glob",
    "Grep",
    "dcrontab",
    TOOL_READ_OBJECT,
    TOOL_X_CALL,
    TOOL_X_CALL_SNAKE,
    "read_file",
    "write_file",
    "edit_file",
    "todo",
    "get_session",
    "create_workspace",
    "bind_workspace",
    TOOL_AGENT_MEMORY,
    TOOL_AGENT_MEMORY_SNAKE,
    TOOL_AGENT_NOTEBOOK,
    TOOL_AGENT_NOTEBOOK_SNAKE,
    TOOL_AGENT_SKILLS,
    TOOL_AGENT_SKILLS_SNAKE,
    TOOL_CHECK_TASK,
    TOOL_CANCEL_TASK,
    TOOL_FINISH_TASK,
    TOOL_READ_SESSION_HISTORY,
    TOOL_COMMIT_SESSION_HISTORY_IMPROVED,
    TOOL_BEGIN_ATTENTION_SIGNAL_EXTRACTION,
    TOOL_COMPLETE_ATTENTION_SIGNAL_EXTRACTION,
    agent_tool::TOOL_DISCOVER_EVENT,
    agent_tool::TOOL_DISCOVER_OBJECT_OBSERVATION,
    agent_tool::TOOL_DISCOVER_RELATIONSHIP,
    agent_tool::TOOL_DISCOVER_SKILL_COVERAGE_GAP,
    TOOL_LIST_PENDING_ATTENTION_SIGNALS,
    TOOL_MARK_ATTENTION_SIGNAL_CONSUMED,
];
const AGENT_MEMORY_ROOT_ENV: &str = "AGENT_MEMORY_ROOT";
const AGENT_MEMORY_DIR_NAME: &str = "memory";
const AGENT_NOTEBOOK_ROOT_ENV: &str = "AGENT_NOTEBOOK_ROOT";
const AGENT_NOTEBOOK_DIR_NAME: &str = "notebook";
const AGENT_SKILLS_ROOT_ENV: &str = "AGENT_SKILLS_ROOT";
const EXIT_SUCCESS: i32 = agent_tool::CLI_EXIT_SUCCESS;
const COMMAND_NOT_FOUND_PROXY: &str = agent_tool::CLI_COMMAND_NOT_FOUND_SUBCOMMAND;
const MAIN_BINARY_NAME: &str = "agent_tool";
const DEFAULT_AGENT_NAME: &str = "did:opendan:cli";
const DEFAULT_WAKEUP_ID: &str = "cli-wakeup";
const DEFAULT_BEHAVIOR: &str = "cli";
const SESSION_RECORD_FILE: &str = "session.json";
const SESSION_WORKSPACE_BINDINGS_REL_PATH: &str = "workspaces/session_workspace_bindings.json";
const WORKSPACE_INDEX_FILE: &str = "index.json";
const DEFAULT_HISTORY_PAGE_SIZE: usize = 50;
const MAX_HISTORY_PAGE_SIZE: usize = 200;
const DEFAULT_HISTORY_TOKEN_LIMIT: u32 = 40 * 1024;
const DEFAULT_HISTORY_WINDOW_MS: i64 = 10 * 60 * 1000;
const ATTENTION_EXTRACTION_RUNTIME_REL_PATH: &str =
    ".runtime/attention_signal_extraction/current.json";
const OBJECT_ROUTE_CONFIG_ENV: &str = "AGENT_DID_OBJECT_ROUTE_CONFIG";
const OPENDAN_OBJECT_ROUTE_CONFIG_ENV: &str = "OPENDAN_AGENT_OBJECT_ROUTE_CONFIG";
const DEFAULT_OBJECT_ROUTE_CONFIG: &str = r#"
version = 1

[[adapters]]
id = "filesystem"
type = "filesystem"

[[adapters]]
id = "web"
type = "web"

[[adapters]]
id = "agent-runtime"
type = "agent_runtime"

[[adapters]]
id = "did-object"
type = "did_object"

[[routes]]
id = "file-read"
priority = 100
match_type = "scheme"
pattern = "file"
adapter = "filesystem"
methods = ["read"]

[[routes]]
id = "http-web-read"
priority = 10
match_type = "scheme"
pattern = "http"
adapter = "web"
methods = ["read"]

[[routes]]
id = "https-web-read"
priority = 10
match_type = "scheme"
pattern = "https"
adapter = "web"
methods = ["read"]

[[routes]]
id = "https-did-object-call"
priority = 10
match_type = "scheme"
pattern = "https"
adapter = "did-object"
methods = ["x_call", "subscribe_event"]

[[routes]]
id = "agent-runtime"
priority = 10
match_type = "scheme"
pattern = "agent"
adapter = "agent-runtime"
"#;

#[derive(Clone, Debug)]
struct CliRuntimeEnv {
    agent_env_root: PathBuf,
    has_agent_env: bool,
    current_dir: PathBuf,
    stdout_is_terminal: bool,
    runtime_context: RuntimeContext,
    call_ctx: SessionRuntimeContext,
}

impl CliRuntimeEnv {
    fn from_process() -> Result<Self, AgentToolError> {
        let current_dir = env::current_dir()
            .map(|path| canonicalize_or_normalize(path, None))
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("resolve current dir failed: {err}"))
            })?;
        let runtime_context = RuntimeContext::from_process_env(&current_dir, true)?;
        let agent_env_root = runtime_context.agent_root.clone();
        let has_agent_env = !runtime_context.is_dev_fallback();
        let agent_name = resolve_runtime_agent_name(&runtime_context)?;
        let trace_id = runtime_context.trace_id.clone();
        let session_id = runtime_context.session_id.clone();

        Ok(Self {
            agent_env_root,
            has_agent_env,
            current_dir,
            stdout_is_terminal: std::io::stdout().is_terminal(),
            runtime_context,
            call_ctx: SessionRuntimeContext {
                trace_id,
                agent_name,
                behavior: DEFAULT_BEHAVIOR.to_string(),
                step_idx: 0,
                wakeup_id: DEFAULT_WAKEUP_ID.to_string(),
                session_id,
                read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
            },
        })
    }

    fn use_plain_text_read_output(&self) -> bool {
        !self.has_agent_env && !self.stdout_is_terminal
    }

    fn allow_dev_overrides(&self) -> bool {
        self.runtime_context.is_dev_fallback()
    }
}

fn resolve_runtime_agent_name(runtime_context: &RuntimeContext) -> Result<String, AgentToolError> {
    if let Some(identity) = runtime_context.identity.as_ref() {
        return Ok(identity.agent_id.clone());
    }
    if let Ok(Some((app_id, _owner_id))) = load_app_identity_from_env() {
        let app_id = app_id.trim().to_string();
        if !app_id.is_empty() {
            return Ok(app_id);
        }
    }
    if runtime_context.is_dev_fallback() {
        return Ok(DEFAULT_AGENT_NAME.to_string());
    }
    Err(AgentToolError::ExecFailed(format!(
        "missing Agent RootFS identity metadata under {}; expected owner_user_id and agent_id",
        runtime_context.agent_root.display()
    )))
}

/// What the parser produced. The dispatcher resolves the tool against
/// the registry and asks it to parse its own argv via
/// `AgentTool::parse_cli_args`. Pseudo-tools (`check_task`/`cancel_task`/
/// `finish_task`) stay as variants because they don't live in the tool registry.
#[derive(Clone, Debug)]
enum ParsedCommand {
    CommandNotFound {
        command: Option<String>,
        argv: Vec<String>,
    },
    Help {
        tool_name: Option<String>,
    },
    Tool {
        tool_name: String,
        raw_tokens: Vec<String>,
    },
    ObjectRead {
        route_config_path: Option<PathBuf>,
        input: ObjectReadInput,
    },
    ObjectXCall {
        tool_name: String,
        route_config_path: Option<PathBuf>,
        input: ObjectXCallInput,
    },
    CheckTask {
        tool_name: String,
        task_id: String,
    },
    CancelTask {
        tool_name: String,
        task_id: String,
        recursive: bool,
    },
    FinishTask {
        tool_name: String,
        task_id: String,
        outcome: FinishTaskOutcome,
        message: Option<String>,
    },
    AgentMemory {
        tool_name: String,
        invocation: AgentMemoryInvocation,
    },
    AgentNotebook {
        tool_name: String,
        invocation: AgentNotebookInvocation,
    },
    AgentSkills {
        tool_name: String,
        invocation: AgentSkillsInvocation,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FinishTaskOutcome {
    Completed,
    Failed,
}

/// Parsed `agent-memory` command before execution. Mirrors the v2.10 Memory
/// Graph CLI while preserving the old flat `set/get/list/load` forms.
#[derive(Clone, Debug)]
struct AgentMemoryInvocation {
    root_override: Option<PathBuf>,
    quiet: bool,
    verb: AgentMemoryVerb,
}

#[derive(Clone, Debug)]
struct AgentSkillsInvocation {
    root_override: Option<PathBuf>,
    verb: AgentSkillsVerb,
}

#[derive(Clone, Debug)]
enum AgentSkillsVerb {
    Init,
    Install {
        path: PathBuf,
        id: Option<String>,
        name: Option<String>,
        title: Option<String>,
        description: Option<String>,
        skill_type: Option<SkillType>,
        group_id: Option<String>,
        intent_tags: Vec<String>,
        required_tools: Vec<String>,
        risk_level: Option<SkillRiskLevel>,
        source_type: SkillSourceType,
        target_scope: SkillOwnerScope,
        trust_hint: String,
        installed_by: Option<String>,
        promote: bool,
    },
    List {
        filter: ListSkillsInput,
    },
    ExplorerHints {
        filter: ListSkillsInput,
    },
    View {
        skill_ref: String,
        package_path: Option<String>,
    },
    Promote {
        skill_ref: String,
        actor: Option<String>,
    },
    Archive {
        skill_ref: String,
        reason: String,
        actor: Option<String>,
    },
    Restore {
        skill_ref: String,
        reason: String,
        actor: Option<String>,
    },
    Validate {
        requested: Vec<String>,
        max_count: usize,
        allowed_states: Vec<SkillLifecycleState>,
        risk_budget: SkillRiskLevel,
    },
    SelectSession {
        session_id: Option<String>,
        requested: Vec<String>,
        risk_budget: SkillRiskLevel,
        max_count: usize,
    },
    SelectTodo {
        session_id: Option<String>,
        todo_id: String,
        requested: Vec<String>,
        risk_budget: SkillRiskLevel,
        max_count: usize,
    },
    UnselectSession {
        session_id: Option<String>,
        skill_refs: Vec<String>,
    },
    UnselectTodo {
        session_id: Option<String>,
        todo_id: String,
        skill_refs: Vec<String>,
    },
    SelectedList {
        session_id: Option<String>,
        todo_id: Option<String>,
    },
    RenderSelected {
        session_id: Option<String>,
        todo_id: Option<String>,
        max_skills: usize,
        token_budget: usize,
        include_metadata: bool,
        include_usage_instructions: bool,
    },
    RecordUsed {
        usage_id: String,
        mode: SkillUsageMode,
        evidence: Vec<String>,
    },
    RecordResult {
        usage_id: String,
        result: SkillTaskResult,
        feedback: Option<SkillUserFeedback>,
        failure_reason: Option<String>,
        output_refs: Vec<String>,
    },
}

#[derive(Clone, Debug)]
enum AgentMemoryVerb {
    Init,
    OccasionAdd {
        occasion_type: String,
        summary: String,
        occurred_at: Option<String>,
        source_ref: Option<SourceRef>,
        tags: Vec<String>,
    },
    Set {
        key: String,
        /// `Some` → content was passed as positional argv (form A).
        /// `None` → content must come from stdin (form B).
        content: Option<String>,
        reason: String,
        entities: Vec<String>,
        tags: Vec<String>,
    },
    Remove {
        key: String,
        reason: Option<String>,
    },
    Get {
        kind: Option<String>,
        id: String,
    },
    List {
        prefix: Option<String>,
    },
    ListObjects {
        kind: Option<String>,
    },
    ObserveAdd {
        occasion_id: String,
        op: AddObservationOp,
    },
    ObjectUpsert {
        occasion_id: String,
        op: UpsertObjectOp,
    },
    ObjectReinforce {
        occasion_id: String,
        op: ReinforceObjectWeightOp,
    },
    Relate {
        occasion_id: String,
        op: UpsertRelationOp,
    },
    SetStatus {
        occasion_id: String,
        op: SetStatusOp,
    },
    Load {
        tags: Vec<String>,
        objects: Vec<String>,
        aliases: Vec<String>,
        max_records: Option<usize>,
        max_bytes: Option<usize>,
    },
    Verify {
        repair: bool,
    },
    Compact,
}

pub async fn run_process() -> CliRunOutput {
    let args = env::args_os().collect::<Vec<_>>();

    // `agent_tool run_local_llm ...` / `agent_tool llm_explore ...` 走独立的
    // dev/test 子命令，不经过 tool dispatcher（它们不是 AgentTool）。这里
    // 短路掉，让它们自己负责 stdout / stderr / exit code（直接 println /
    // eprintln，避免 buffer 大段 JSON）。
    if args.get(1).and_then(|v| v.to_str()) == Some("run_local_llm") {
        let sub_args: Vec<String> = args
            .iter()
            .skip(2)
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        let exit_code = run_local_llm::run_subcommand(sub_args).await;
        return CliRunOutput {
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
        };
    }

    if args.get(1).and_then(|v| v.to_str()) == Some("llm_explore") {
        let sub_args: Vec<String> = args
            .iter()
            .skip(2)
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        let exit_code = llm_explore::run_subcommand(sub_args).await;
        return CliRunOutput {
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
        };
    }

    if args.get(1).and_then(|v| v.to_str()) == Some("llm_understand_media") {
        let sub_args: Vec<String> = args
            .iter()
            .skip(2)
            .map(|v| v.to_string_lossy().into_owned())
            .collect();
        let exit_code = llm_understand_media::run_subcommand(sub_args).await;
        return CliRunOutput {
            exit_code,
            stdout: String::new(),
            stderr: String::new(),
        };
    }

    let env = match CliRuntimeEnv::from_process() {
        Ok(env) => env,
        Err(err) => {
            let exit_code = cli_exit_code_for_error(&err);
            return render_cli_output(&cli_error_result(None, &err), exit_code);
        }
    };

    match execute(args, env, None).await {
        Ok(output) => output,
        Err(err) => {
            let exit_code = cli_exit_code_for_error(&err);
            render_cli_output(&cli_error_result(None, &err), exit_code)
        }
    }
}

async fn execute(
    args: Vec<OsString>,
    env: CliRuntimeEnv,
    stdin_override: Option<String>,
) -> Result<CliRunOutput, AgentToolError> {
    let parsed = parse_command(&args, &env.current_dir)?;
    match parsed {
        ParsedCommand::CommandNotFound { command, argv } => {
            // Delegate to `llm_tool_carft` — the intent-engine-bypass scaffold.
            // Today the scaffold's step 1 always skips (no behavior cfg toggle
            // wired yet), so the visible behavior matches the old placeholder:
            // exit 127 + a one-liner explaining why. Once behavior cfg can flip
            // the bypass on, the same dispatch will start exercising step 2-4
            // without further changes here.
            let req = CommandNotFoundRequest {
                command,
                argv,
                current_dir: env.current_dir.clone(),
                agent_env_root: env.has_agent_env.then(|| env.agent_env_root.clone()),
            };
            let (result, exit_code) = llm_tool_carft::run_subcommand(req).await;
            Ok(render_cli_output(&result, exit_code))
        }
        ParsedCommand::Help { tool_name } => Ok(render_cli_output(
            &build_help_result(&env, tool_name.as_deref()).await,
            EXIT_SUCCESS,
        )),
        ParsedCommand::Tool {
            tool_name,
            raw_tokens,
        } => {
            let mgr = build_cli_tool_manager(&env).await?;
            let Some(tool) = mgr.get_any_tool(&tool_name) else {
                return Err(AgentToolError::NotFound(tool_name));
            };
            let invocation = tool.parse_cli_args(&raw_tokens, Some(env.current_dir.as_path()))?;

            // Tools that opt in to plain-text stdout (read_file) get the
            // payload unwrapped when the CLI is being piped to another
            // process. Otherwise emit the standard JSON result.
            let plain = tool.cli_plain_text_stdout() && env.use_plain_text_read_output();
            if plain {
                return match dispatch_tool(&env, tool.as_ref(), invocation, stdin_override).await {
                    Ok(result) => Ok(render_plain_read_file_output(result)),
                    Err(err) => Ok(render_plain_error_output(&err)),
                };
            }
            let result = dispatch_tool(&env, tool.as_ref(), invocation, stdin_override).await?;
            Ok(render_cli_output(
                &success_result(&tool_name, result),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::ObjectRead {
            route_config_path,
            input,
        } => dispatch_object_read(&env, route_config_path, input).await,
        ParsedCommand::ObjectXCall {
            tool_name,
            route_config_path,
            input,
        } => dispatch_object_x_call(&env, &tool_name, route_config_path, input).await,
        ParsedCommand::CheckTask { tool_name, task_id } => {
            let task_mgr = build_task_manager_client(&env).await?;
            let task = task_mgr.get_task(&task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("get task `{task_id}` failed: {err}"))
            })?;
            Ok(render_cli_output(
                &build_check_task_result(&tool_name, task),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::AgentMemory {
            tool_name,
            invocation,
        } => Ok(dispatch_agent_memory(&env, &tool_name, invocation, stdin_override).await),
        ParsedCommand::AgentNotebook {
            tool_name,
            invocation,
        } => Ok(dispatch_agent_notebook(&env, &tool_name, invocation, stdin_override).await),
        ParsedCommand::AgentSkills {
            tool_name,
            invocation,
        } => Ok(dispatch_agent_skills(&env, &tool_name, invocation).await),
        ParsedCommand::CancelTask {
            tool_name,
            task_id,
            recursive,
        } => {
            let task_mgr = build_task_manager_client(&env).await?;
            let before = task_mgr.get_task(&task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("get task `{task_id}` failed: {err}"))
            })?;
            task_mgr
                .cancel_task(&task_id, recursive)
                .await
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("cancel task `{task_id}` failed: {err}"))
                })?;
            let interrupt_error = interrupt_task_if_supported(&before).await;
            let after = task_mgr.get_task(&task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("reload task `{task_id}` failed: {err}"))
            })?;
            Ok(render_cli_output(
                &build_cancel_task_result(&tool_name, after, recursive, interrupt_error),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::FinishTask {
            tool_name,
            task_id,
            outcome,
            message,
        } => {
            let task_mgr = build_task_manager_client(&env).await?;
            let current = task_mgr.get_task(&task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("get task `{task_id}` failed: {err}"))
            })?;
            if !current.phase.is_terminal() {
                match outcome {
                    FinishTaskOutcome::Completed => {
                        task_mgr
                            .commit_result(CommitResultReq {
                                task_id: current.task_id.clone(),
                                result: serde_json::json!({
                                    "message": message.clone(),
                                    "finished_by": "finish_task",
                                }),
                                app_instance_id: None,
                                runner_epoch: Some(current.runner_epoch),
                                expected_revision: current.revision,
                            })
                            .await
                    }
                    FinishTaskOutcome::Failed => {
                        let error_message = message
                            .clone()
                            .unwrap_or_else(|| "failed by finish_task".to_string());
                        task_mgr
                            .fail_task(FailTaskReq {
                                envelope: RunnerWriteEnvelope {
                                    task_id: current.task_id.clone(),
                                    app_instance_id: None,
                                    runner_epoch: current.runner_epoch,
                                    expected_revision: current.revision,
                                },
                                error: TaskError::new("finish_task_failed", error_message),
                            })
                            .await
                    }
                }
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("finish task `{task_id}` failed: {err}"))
                })?;
            }
            let task = task_mgr.get_task(&task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("reload task `{task_id}` failed: {err}"))
            })?;
            Ok(render_cli_output(
                &build_finish_task_result(&tool_name, task, outcome),
                EXIT_SUCCESS,
            ))
        }
    }
}

/// Routes a CliInvocation through `exec` (bash form) or `call` (json
/// form), resolving any optional stdin pickup before the JSON args go
/// in.
async fn dispatch_tool(
    env: &CliRuntimeEnv,
    tool: &dyn agent_tool::AgentTool,
    invocation: agent_tool::CliInvocation,
    stdin_override: Option<String>,
) -> Result<AgentToolResult, AgentToolError> {
    match invocation {
        agent_tool::CliInvocation::Bash { line } => {
            tool.exec(&env.call_ctx, &line, Some(env.current_dir.as_path()))
                .await
        }
        agent_tool::CliInvocation::Json {
            mut args,
            content_input,
        } => {
            if let Some((field, ci)) = content_input {
                let content = resolve_content_input(ci, stdin_override).await?;
                let map = args.as_object_mut().ok_or_else(|| {
                    AgentToolError::InvalidArgs(format!("{} args must be object", tool.spec().name))
                })?;
                map.insert(field, Json::String(content));
            }
            tool.call(&env.call_ctx, args).await
        }
    }
}

async fn dispatch_object_read(
    env: &CliRuntimeEnv,
    route_config_path: Option<PathBuf>,
    mut input: ObjectReadInput,
) -> Result<CliRunOutput, AgentToolError> {
    if input.session_id.is_none() {
        input.session_id = Some(env.call_ctx.session_id.clone());
    }
    if input.max_tokens.is_none() {
        input.max_tokens = Some(env.call_ctx.read_token_limit as usize);
    }
    let runtime = build_object_runtime(env, route_config_path).await?;
    match runtime.read(input).await {
        Ok(result) => Ok(render_cli_output(
            &success_result(TOOL_READ_OBJECT, result),
            EXIT_SUCCESS,
        )),
        Err(err) => Ok(render_object_error_output(Some(TOOL_READ_OBJECT), err)),
    }
}

async fn dispatch_object_x_call(
    env: &CliRuntimeEnv,
    tool_name: &str,
    route_config_path: Option<PathBuf>,
    mut input: ObjectXCallInput,
) -> Result<CliRunOutput, AgentToolError> {
    if input.session_id.is_none() {
        input.session_id = Some(env.call_ctx.session_id.clone());
    }
    if input.trace_id.is_none() {
        input.trace_id = Some(env.call_ctx.trace_id.clone());
    }
    let runtime = build_object_runtime(env, route_config_path).await?;
    match runtime.x_call(input).await {
        Ok(result) => {
            let exit_code = result.return_code.unwrap_or_else(|| {
                if result.status == AgentToolStatus::Error {
                    agent_tool::CLI_EXIT_ERROR
                } else {
                    EXIT_SUCCESS
                }
            });
            Ok(render_cli_output(
                &success_result(tool_name, result),
                exit_code,
            ))
        }
        Err(err) => Ok(render_object_error_output(Some(tool_name), err)),
    }
}

async fn build_object_runtime(
    env: &CliRuntimeEnv,
    route_config_path: Option<PathBuf>,
) -> Result<AgentDIDObjectRuntime, AgentToolError> {
    let config = load_object_route_config(env, route_config_path).await?;
    AgentDIDObjectRuntime::new(config).map_err(object_error_to_agent_tool_error)
}

async fn load_object_route_config(
    env: &CliRuntimeEnv,
    route_config_path: Option<PathBuf>,
) -> Result<ObjectRouteConfig, AgentToolError> {
    if let Some(path) = route_config_path.or_else(|| {
        first_path_env(
            &[OBJECT_ROUTE_CONFIG_ENV, OPENDAN_OBJECT_ROUTE_CONFIG_ENV],
            &env.current_dir,
        )
    }) {
        return ObjectRouteConfig::from_toml_file(&path)
            .await
            .map_err(object_error_to_agent_tool_error);
    }
    ObjectRouteConfig::from_toml_str(DEFAULT_OBJECT_ROUTE_CONFIG)
        .map_err(object_error_to_agent_tool_error)
}

fn render_object_error_output(
    tool_name: Option<&str>,
    err: agent_did_object_lib::AgentDIDObjectError,
) -> CliRunOutput {
    let agent_err = object_error_to_agent_tool_error(err);
    let exit_code = cli_exit_code_for_error(&agent_err);
    render_cli_output(&cli_error_result(tool_name, &agent_err), exit_code)
}

async fn resolve_content_input(
    input: agent_tool::ContentInput,
    stdin_override: Option<String>,
) -> Result<String, AgentToolError> {
    match input {
        agent_tool::ContentInput::Inline(value) => Ok(value),
        agent_tool::ContentInput::Stdin => {
            if let Some(value) = stdin_override {
                return Ok(value);
            }
            let mut stdin = io::stdin();
            let mut buf = String::new();
            stdin
                .read_to_string(&mut buf)
                .await
                .map_err(|err| AgentToolError::ExecFailed(format!("read stdin failed: {err}")))?;
            Ok(buf)
        }
    }
}

fn parse_command(args: &[OsString], current_dir: &Path) -> Result<ParsedCommand, AgentToolError> {
    let argv0 = args
        .first()
        .and_then(|value| Path::new(value).file_name())
        .and_then(|value| value.to_str())
        .unwrap_or(MAIN_BINARY_NAME);
    let rest = args
        .iter()
        .skip(1)
        .map(os_to_string)
        .collect::<Result<Vec<_>, _>>()?;

    if is_tool_name(argv0) {
        return parse_tool_command(argv0.to_string(), &rest, current_dir);
    }

    if rest.first().map(|value| value.as_str()) == Some(COMMAND_NOT_FOUND_PROXY) {
        let Some(tool_name) = rest.get(1) else {
            return Ok(ParsedCommand::CommandNotFound {
                command: None,
                argv: vec![],
            });
        };
        if !is_tool_name(tool_name) {
            return Ok(ParsedCommand::CommandNotFound {
                command: Some(tool_name.clone()),
                argv: rest[1..].to_vec(),
            });
        }
        return parse_tool_command(tool_name.to_string(), &rest[2..], current_dir);
    }

    if rest.is_empty() || matches!(rest[0].as_str(), "--help" | "-h" | "help") {
        let tool_name = rest.get(1).cloned().filter(|value| is_tool_name(value));
        return Ok(ParsedCommand::Help { tool_name });
    }

    let tool_name = rest[0].clone();
    if !is_tool_name(&tool_name) {
        return Err(AgentToolError::InvalidArgs(format!(
            "unsupported tool `{tool_name}`\nUsage: {}",
            generic_usage()
        )));
    }

    parse_tool_command(tool_name, &rest[1..], current_dir)
}

fn parse_tool_command(
    tool_name: String,
    tokens: &[String],
    current_dir: &Path,
) -> Result<ParsedCommand, AgentToolError> {
    if matches!(tokens, [flag] if flag == "--help" || flag == "-h") {
        return Ok(ParsedCommand::Help {
            tool_name: Some(tool_name),
        });
    }

    match tool_name.as_str() {
        TOOL_READ_OBJECT => parse_object_read_cli_command(tokens, current_dir),
        TOOL_X_CALL | TOOL_X_CALL_SNAKE => {
            parse_object_x_call_cli_command(tool_name, tokens, current_dir)
        }
        TOOL_CHECK_TASK => parse_check_task_cli_command(tool_name, tokens),
        TOOL_CANCEL_TASK => parse_cancel_task_cli_command(tool_name, tokens),
        TOOL_FINISH_TASK => parse_finish_task_cli_command(tool_name, tokens),
        TOOL_AGENT_MEMORY | TOOL_AGENT_MEMORY_SNAKE => {
            parse_agent_memory_cli_command(tool_name, tokens)
        }
        TOOL_AGENT_NOTEBOOK | TOOL_AGENT_NOTEBOOK_SNAKE => {
            parse_agent_notebook_cli_command(tool_name, tokens)
        }
        TOOL_AGENT_SKILLS | TOOL_AGENT_SKILLS_SNAKE => {
            parse_agent_skills_cli_command(tool_name, tokens)
        }
        _ => {
            // All real tools defer their argv parsing to the registry's
            // `AgentTool::parse_cli_args`; the dispatcher will look up
            // `tool_name` in the manager built per-process.
            let _ = current_dir;
            Ok(ParsedCommand::Tool {
                tool_name,
                raw_tokens: tokens.to_vec(),
            })
        }
    }
}

fn parse_object_read_cli_command(
    tokens: &[String],
    current_dir: &Path,
) -> Result<ParsedCommand, AgentToolError> {
    let mut route_config_path = None;
    let mut object = None;
    let mut purpose = None;
    let mut session_id = None;
    let mut content_only = false;
    let mut offset = None;
    let mut limit = None;
    let mut options = json!({});
    let mut idx = 0usize;

    while idx < tokens.len() {
        let token = &tokens[idx];
        match token.as_str() {
            "--config" | "--route-config" => {
                idx += 1;
                let Some(path) = tokens.get(idx) else {
                    return Err(with_tool_usage(
                        "missing route config path",
                        TOOL_READ_OBJECT,
                    ));
                };
                route_config_path = Some(resolve_cli_path(path, current_dir));
            }
            "--purpose" => {
                idx += 1;
                purpose = Some(required_token(tokens, idx, "--purpose", TOOL_READ_OBJECT)?);
            }
            "--session-id" => {
                idx += 1;
                session_id = Some(required_token(
                    tokens,
                    idx,
                    "--session-id",
                    TOOL_READ_OBJECT,
                )?);
            }
            "--content-only" => content_only = true,
            "--offset" => {
                idx += 1;
                offset = Some(parse_object_usize(
                    &required_token(tokens, idx, "--offset", TOOL_READ_OBJECT)?,
                    "offset",
                    TOOL_READ_OBJECT,
                )?);
            }
            "--limit" => {
                idx += 1;
                limit = Some(parse_object_usize(
                    &required_token(tokens, idx, "--limit", TOOL_READ_OBJECT)?,
                    "limit",
                    TOOL_READ_OBJECT,
                )?);
            }
            "--options" => {
                idx += 1;
                options = parse_json_arg(
                    &required_token(tokens, idx, "--options", TOOL_READ_OBJECT)?,
                    "--options",
                    TOOL_READ_OBJECT,
                )?;
            }
            _ if token.starts_with("--") => {
                return Err(with_tool_usage(
                    format!("unsupported flag `{token}`"),
                    TOOL_READ_OBJECT,
                ));
            }
            _ => {
                if let Some((key, value)) = token.split_once('=') {
                    match key {
                        "object" | "uri" => object = Some(value.to_string()),
                        "purpose" => purpose = Some(value.to_string()),
                        "session_id" | "session-id" => session_id = Some(value.to_string()),
                        "content_only" | "content-only" => {
                            content_only = parse_object_bool(value, key, TOOL_READ_OBJECT)?
                        }
                        "offset" => {
                            offset = Some(parse_object_usize(value, "offset", TOOL_READ_OBJECT)?)
                        }
                        "limit" => {
                            limit = Some(parse_object_usize(value, "limit", TOOL_READ_OBJECT)?)
                        }
                        "options" => options = parse_json_arg(value, "options", TOOL_READ_OBJECT)?,
                        _ => {
                            return Err(with_tool_usage(
                                format!("unsupported key `{key}`"),
                                TOOL_READ_OBJECT,
                            ));
                        }
                    }
                } else if object.is_none() {
                    object = Some(token.clone());
                } else {
                    return Err(with_tool_usage(
                        format!("unexpected positional argument `{token}`"),
                        TOOL_READ_OBJECT,
                    ));
                }
            }
        }
        idx += 1;
    }

    let object = object
        .map(|value| canonical_object_url_for_cli(&value, current_dir))
        .transpose()?
        .ok_or_else(|| with_tool_usage("missing object/uri", TOOL_READ_OBJECT))?;

    Ok(ParsedCommand::ObjectRead {
        route_config_path,
        input: ObjectReadInput {
            object,
            purpose,
            session_id,
            content_only,
            range: (offset.is_some() || limit.is_some()).then_some(ReadLineRange {
                offset: offset.unwrap_or(1),
                limit,
            }),
            max_tokens: None,
            options,
        },
    })
}

fn parse_object_x_call_cli_command(
    tool_name: String,
    tokens: &[String],
    current_dir: &Path,
) -> Result<ParsedCommand, AgentToolError> {
    let mut route_config_path = None;
    let mut object = None;
    let mut action = None;
    let mut params = json!({});
    let mut session_id = None;
    let mut idempotency_key = None;
    let mut confirm_token = None;
    let mut trace_id = None;
    let mut positionals = Vec::new();
    let mut param_pairs = serde_json::Map::new();
    let mut idx = 0usize;

    while idx < tokens.len() {
        let token = &tokens[idx];
        match token.as_str() {
            "--config" | "--route-config" => {
                idx += 1;
                let Some(path) = tokens.get(idx) else {
                    return Err(with_tool_usage("missing route config path", &tool_name));
                };
                route_config_path = Some(resolve_cli_path(path, current_dir));
            }
            "--params" => {
                idx += 1;
                params = parse_json_arg(
                    &required_token(tokens, idx, "--params", &tool_name)?,
                    "--params",
                    &tool_name,
                )?;
            }
            "--session-id" => {
                idx += 1;
                session_id = Some(required_token(tokens, idx, "--session-id", &tool_name)?);
            }
            "--idempotency-key" => {
                idx += 1;
                idempotency_key = Some(required_token(
                    tokens,
                    idx,
                    "--idempotency-key",
                    &tool_name,
                )?);
            }
            "--confirm-token" => {
                idx += 1;
                confirm_token = Some(required_token(tokens, idx, "--confirm-token", &tool_name)?);
            }
            "--trace-id" => {
                idx += 1;
                trace_id = Some(required_token(tokens, idx, "--trace-id", &tool_name)?);
            }
            _ if token.starts_with("--") => {
                if let Some((key, value)) =
                    token.strip_prefix("--").and_then(|arg| arg.split_once('='))
                {
                    match key {
                        "config" | "route-config" => {
                            route_config_path = Some(resolve_cli_path(value, current_dir));
                        }
                        "params" => {
                            params = parse_json_arg(value, "--params", &tool_name)?;
                        }
                        "session_id" | "session-id" => session_id = Some(value.to_string()),
                        "idempotency_key" | "idempotency-key" => {
                            idempotency_key = Some(value.to_string())
                        }
                        "confirm_token" | "confirm-token" => {
                            confirm_token = Some(value.to_string())
                        }
                        "trace_id" | "trace-id" => trace_id = Some(value.to_string()),
                        key if !key.trim().is_empty() && !key.starts_with('-') => {
                            param_pairs.insert(key.to_string(), parse_scalar_json_value(value));
                        }
                        _ => {
                            return Err(with_tool_usage(
                                format!("unsupported flag `{token}`"),
                                &tool_name,
                            ));
                        }
                    }
                } else {
                    return Err(with_tool_usage(
                        format!("unsupported flag `{token}`"),
                        &tool_name,
                    ));
                }
            }
            _ => {
                if let Some((key, value)) = token.split_once('=') {
                    match key {
                        "object" => object = Some(value.to_string()),
                        "action" => action = Some(value.to_string()),
                        "params" => params = parse_json_arg(value, "params", &tool_name)?,
                        "session_id" | "session-id" => session_id = Some(value.to_string()),
                        "idempotency_key" | "idempotency-key" => {
                            idempotency_key = Some(value.to_string())
                        }
                        "confirm_token" | "confirm-token" => {
                            confirm_token = Some(value.to_string())
                        }
                        "trace_id" | "trace-id" => trace_id = Some(value.to_string()),
                        key => {
                            param_pairs.insert(key.to_string(), parse_scalar_json_value(value));
                        }
                    }
                } else {
                    positionals.push(token.clone());
                }
            }
        }
        idx += 1;
    }

    if object.is_none() {
        object = positionals.first().cloned();
    }
    if action.is_none() {
        action = positionals.get(1).cloned();
    }
    if let Some(raw_params) = positionals.get(2) {
        params = parse_json_arg(raw_params, "params", &tool_name)?;
    }
    if positionals.len() > 3 {
        return Err(with_tool_usage(
            format!(
                "unexpected positional argument `{}`",
                positionals.get(3).unwrap()
            ),
            &tool_name,
        ));
    }
    if !param_pairs.is_empty() {
        if params.as_object().is_some_and(|map| map.is_empty()) {
            params = Json::Object(param_pairs);
        } else if let Some(map) = params.as_object_mut() {
            map.extend(param_pairs);
        } else {
            return Err(with_tool_usage(
                "key=value params require object params",
                &tool_name,
            ));
        }
    }

    let object = object
        .map(|value| canonical_object_url_for_cli(&value, current_dir))
        .transpose()?
        .ok_or_else(|| with_tool_usage("missing object", &tool_name))?;
    let action = action
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| with_tool_usage("missing action", &tool_name))?;

    Ok(ParsedCommand::ObjectXCall {
        tool_name,
        route_config_path,
        input: ObjectXCallInput {
            object,
            action,
            params,
            session_id,
            idempotency_key,
            confirm_token,
            trace_id,
        },
    })
}

fn parse_check_task_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    Ok(ParsedCommand::CheckTask {
        tool_name,
        task_id: parse_task_id_arg(tokens, TOOL_CHECK_TASK)?,
    })
}

fn parse_cancel_task_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    let mut recursive = false;
    let mut task_tokens = Vec::new();
    for token in tokens {
        match token.as_str() {
            "--recursive" => recursive = true,
            "--no-recursive" => recursive = false,
            _ => task_tokens.push(token.clone()),
        }
    }

    Ok(ParsedCommand::CancelTask {
        tool_name,
        task_id: parse_task_id_arg(&task_tokens, TOOL_CANCEL_TASK)?,
        recursive,
    })
}

fn parse_finish_task_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    let mut outcome = FinishTaskOutcome::Completed;
    let mut message: Option<String> = None;
    let mut task_tokens = Vec::new();
    let mut idx = 0usize;
    while idx < tokens.len() {
        let token = &tokens[idx];
        match token.as_str() {
            "--failed" | "--fail" => outcome = FinishTaskOutcome::Failed,
            "--success" | "--completed" | "--complete" => outcome = FinishTaskOutcome::Completed,
            "--status" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage("missing value for `--status`", TOOL_FINISH_TASK)
                })?;
                outcome = parse_finish_task_outcome(value)?;
            }
            "--message" | "--reason" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage(format!("missing value for `{token}`"), TOOL_FINISH_TASK)
                })?;
                message = Some(value.clone());
            }
            value if matches_finish_task_outcome_token(value) => {
                outcome = parse_finish_task_outcome(value)?;
            }
            value if value.contains('=') => {
                let (key, raw_value) = value
                    .split_once('=')
                    .ok_or_else(|| with_tool_usage("invalid key=value arg", TOOL_FINISH_TASK))?;
                match key {
                    "status" | "outcome" => outcome = parse_finish_task_outcome(raw_value)?,
                    "message" | "reason" | "error" | "error_message" => {
                        message = Some(raw_value.to_string())
                    }
                    _ => task_tokens.push(value.to_string()),
                }
            }
            _ => task_tokens.push(token.clone()),
        }
        idx += 1;
    }

    Ok(ParsedCommand::FinishTask {
        tool_name,
        task_id: parse_task_id_arg(&task_tokens, TOOL_FINISH_TASK)?,
        outcome,
        message,
    })
}

fn matches_finish_task_outcome_token(value: &str) -> bool {
    matches!(
        value,
        "success"
            | "succeeded"
            | "complete"
            | "completed"
            | "finish"
            | "finished"
            | "fail"
            | "failed"
            | "error"
    )
}

fn parse_finish_task_outcome(value: &str) -> Result<FinishTaskOutcome, AgentToolError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "success" | "succeeded" | "complete" | "completed" | "finish" | "finished" => {
            Ok(FinishTaskOutcome::Completed)
        }
        "fail" | "failed" | "error" => Ok(FinishTaskOutcome::Failed),
        _ => Err(with_tool_usage(
            format!("unsupported finish status `{}`", value.trim()),
            TOOL_FINISH_TASK,
        )),
    }
}

fn parse_task_id_arg(tokens: &[String], tool_name: &str) -> Result<String, AgentToolError> {
    if tokens.is_empty() {
        return Err(with_tool_usage("missing required arg `task_id`", tool_name));
    }

    let mut task_id: Option<String> = None;
    let mut idx = 0usize;
    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "--task-id" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| with_tool_usage("missing value for `--task-id`", tool_name))?;
                task_id = Some(parse_task_id_value(value, tool_name)?);
            }
            token if token.starts_with("--") => {
                return Err(with_tool_usage(
                    format!("unsupported flag `{token}`"),
                    tool_name,
                ));
            }
            token if token.contains('=') => {
                let (key, value) = token
                    .split_once('=')
                    .ok_or_else(|| with_tool_usage("invalid key=value arg", tool_name))?;
                match key {
                    "task_id" | "task" | "id" => {
                        task_id = Some(parse_task_id_value(value, tool_name)?);
                    }
                    _ => {
                        return Err(with_tool_usage(
                            format!("unsupported arg `{key}`"),
                            tool_name,
                        ));
                    }
                }
            }
            value => {
                if task_id.is_some() {
                    return Err(with_tool_usage(
                        format!("unexpected positional arg `{value}`"),
                        tool_name,
                    ));
                }
                task_id = Some(parse_task_id_value(value, tool_name)?);
            }
        }
        idx += 1;
    }

    task_id.ok_or_else(|| with_tool_usage("missing required arg `task_id`", tool_name))
}

fn parse_task_id_value(raw: &str, tool_name: &str) -> Result<String, AgentToolError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(with_tool_usage("invalid empty task_id", tool_name));
    }
    Ok(trimmed.to_string())
}

// =================================================================
//  agent-memory CLI
// =================================================================

const AGENT_MEMORY_USAGE: &str = "agent-memory [--root <path>] [--quiet] \
<init|occasion|object|observe|relate|set-status|set|remove|get|list|load|verify|compact> [...]";

fn agent_memory_invalid(message: impl Into<String>) -> AgentToolError {
    AgentToolError::InvalidArgs(format!("{}\nUsage: {}", message.into(), AGENT_MEMORY_USAGE))
}

/// Parse `agent-memory` argv per §3.1 + §4.x.
///
/// Global flags (`--root`, `--quiet`) are recognized before the verb.
/// Each verb has its own positional/flag rules; per §4.2 the `set` verb's
/// disambiguation between argv-form and stdin-form looks ONLY at positional
/// count.
fn parse_agent_memory_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    let mut root_override: Option<PathBuf> = None;
    let mut quiet = false;
    let mut idx = 0usize;

    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "--root" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| agent_memory_invalid("missing value for `--root`"))?;
                root_override = Some(PathBuf::from(value));
            }
            v if v.starts_with("--root=") => {
                root_override = Some(PathBuf::from(&v["--root=".len()..]));
            }
            "--quiet" => {
                quiet = true;
            }
            // First non-flag token ends the global-flag region.
            _ => break,
        }
        idx += 1;
    }

    let verb_token = tokens
        .get(idx)
        .ok_or_else(|| agent_memory_invalid("missing verb"))?
        .clone();
    let rest = &tokens[idx + 1..];

    let verb = match verb_token.as_str() {
        "init" => parse_agent_memory_init(rest)?,
        "occasion" => parse_agent_memory_occasion(rest)?,
        "object" => parse_agent_memory_object(rest)?,
        "observe" => parse_agent_memory_observe(rest)?,
        "relate" => parse_agent_memory_relate(rest)?,
        "set-status" => parse_agent_memory_set_status(rest)?,
        "set" => parse_agent_memory_set(rest)?,
        "remove" => parse_agent_memory_remove(rest)?,
        "get" => parse_agent_memory_get(rest)?,
        "list" => parse_agent_memory_list(rest)?,
        "load" => parse_agent_memory_load(rest)?,
        "verify" => parse_agent_memory_verify(rest)?,
        "compact" => parse_agent_memory_compact(rest)?,
        other => {
            return Err(agent_memory_invalid(format!("unknown verb `{other}`")));
        }
    };

    Ok(ParsedCommand::AgentMemory {
        tool_name,
        invocation: AgentMemoryInvocation {
            root_override,
            quiet,
            verb,
        },
    })
}

fn parse_agent_memory_init(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    if !rest.is_empty() {
        return Err(agent_memory_invalid(format!(
            "`init` takes no arguments, got `{}`",
            rest.join(" ")
        )));
    }
    Ok(AgentMemoryVerb::Init)
}

fn parse_agent_memory_occasion(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    if rest.first().map(String::as_str) != Some("add") {
        return Err(agent_memory_invalid("`occasion` expects subcommand `add`"));
    }
    let mut occasion_type: Option<String> = None;
    let mut summary: Option<String> = None;
    let mut occurred_at: Option<String> = None;
    let mut source_ref: Option<SourceRef> = None;
    let mut tags = Vec::new();
    let mut idx = 1usize;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--type" => {
                idx += 1;
                occasion_type = Some(required_flag_value(rest, idx, "--type")?);
            }
            v if v.starts_with("--type=") => occasion_type = Some(v["--type=".len()..].to_string()),
            "--summary" => {
                idx += 1;
                summary = Some(required_flag_value(rest, idx, "--summary")?);
            }
            v if v.starts_with("--summary=") => summary = Some(v["--summary=".len()..].to_string()),
            "--occurred-at" => {
                idx += 1;
                occurred_at = Some(required_flag_value(rest, idx, "--occurred-at")?);
            }
            v if v.starts_with("--occurred-at=") => {
                occurred_at = Some(v["--occurred-at=".len()..].to_string())
            }
            "--source" => {
                idx += 1;
                source_ref = Some(parse_source_ref(&required_flag_value(
                    rest, idx, "--source",
                )?)?);
            }
            v if v.starts_with("--source=") => {
                source_ref = Some(parse_source_ref(&v["--source=".len()..])?)
            }
            "--tags" => {
                idx += 1;
                tags = split_csv(&required_flag_value(rest, idx, "--tags")?);
            }
            v if v.starts_with("--tags=") => tags = split_csv(&v["--tags=".len()..]),
            v => {
                return Err(agent_memory_invalid(format!(
                    "unsupported argument `{v}` for `occasion add`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentMemoryVerb::OccasionAdd {
        occasion_type: occasion_type
            .ok_or_else(|| agent_memory_invalid("`occasion add` requires `--type`"))?,
        summary: summary
            .ok_or_else(|| agent_memory_invalid("`occasion add` requires `--summary`"))?,
        occurred_at,
        source_ref,
        tags,
    })
}

fn parse_agent_memory_object(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    match rest.first().map(String::as_str) {
        Some("upsert") => parse_agent_memory_object_upsert(&rest[1..]),
        Some("reinforce") => parse_agent_memory_object_reinforce(&rest[1..]),
        _ => Err(agent_memory_invalid(
            "`object` expects subcommand `upsert` or `reinforce`",
        )),
    }
}

fn parse_agent_memory_object_upsert(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut occasion_id = None;
    let mut kind = None;
    let mut name = None;
    let mut object_id = None;
    let mut aliases: Vec<ObjectAliasInput> = Vec::new();
    let mut pending_alias: Option<String> = None;
    let mut pending_alias_type: Option<String> = None;
    let mut evidence = Vec::new();
    let mut weight = None;
    let mut confidence = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--occasion" => {
                idx += 1;
                occasion_id = Some(required_flag_value(rest, idx, "--occasion")?);
            }
            v if v.starts_with("--occasion=") => {
                occasion_id = Some(v["--occasion=".len()..].to_string())
            }
            "--kind" => {
                idx += 1;
                kind = Some(required_flag_value(rest, idx, "--kind")?);
            }
            v if v.starts_with("--kind=") => kind = Some(v["--kind=".len()..].to_string()),
            "--name" => {
                idx += 1;
                name = Some(required_flag_value(rest, idx, "--name")?);
            }
            v if v.starts_with("--name=") => name = Some(v["--name=".len()..].to_string()),
            "--object" => {
                idx += 1;
                object_id = Some(required_flag_value(rest, idx, "--object")?);
            }
            v if v.starts_with("--object=") => object_id = Some(v["--object=".len()..].to_string()),
            "--alias" => {
                idx += 1;
                pending_alias = Some(required_flag_value(rest, idx, "--alias")?);
            }
            v if v.starts_with("--alias=") => {
                pending_alias = Some(v["--alias=".len()..].to_string())
            }
            "--alias-type" => {
                idx += 1;
                pending_alias_type = Some(required_flag_value(rest, idx, "--alias-type")?);
            }
            v if v.starts_with("--alias-type=") => {
                pending_alias_type = Some(v["--alias-type=".len()..].to_string())
            }
            "--evidence" => {
                idx += 1;
                evidence = split_csv(&required_flag_value(rest, idx, "--evidence")?);
            }
            v if v.starts_with("--evidence=") => evidence = split_csv(&v["--evidence=".len()..]),
            "--weight" => {
                idx += 1;
                weight = Some(parse_f64_flag(
                    &required_flag_value(rest, idx, "--weight")?,
                    "weight",
                )?);
            }
            v if v.starts_with("--weight=") => {
                weight = Some(parse_f64_flag(&v["--weight=".len()..], "weight")?)
            }
            "--confidence" => {
                idx += 1;
                confidence = Some(parse_f64_flag(
                    &required_flag_value(rest, idx, "--confidence")?,
                    "confidence",
                )?);
            }
            v if v.starts_with("--confidence=") => {
                confidence = Some(parse_f64_flag(&v["--confidence=".len()..], "confidence")?)
            }
            v => {
                return Err(agent_memory_invalid(format!(
                    "unsupported argument `{v}` for `object upsert`"
                )))
            }
        }
        idx += 1;
    }
    if let Some(alias) = pending_alias {
        aliases.push(ObjectAliasInput {
            alias,
            alias_type: pending_alias_type.unwrap_or_else(|| "name".to_string()),
            confidence: confidence.unwrap_or(0.5),
        });
    }
    Ok(AgentMemoryVerb::ObjectUpsert {
        occasion_id: occasion_id
            .ok_or_else(|| agent_memory_invalid("`object upsert` requires `--occasion`"))?,
        op: UpsertObjectOp {
            object_id,
            kind: kind.ok_or_else(|| agent_memory_invalid("`object upsert` requires `--kind`"))?,
            canonical_name: name
                .ok_or_else(|| agent_memory_invalid("`object upsert` requires `--name`"))?,
            aliases,
            evidence,
            weight,
            confidence: confidence
                .ok_or_else(|| agent_memory_invalid("`object upsert` requires `--confidence`"))?,
            merge_into: None,
        },
    })
}

fn parse_agent_memory_object_reinforce(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut occasion_id = None;
    let mut object_id = None;
    let mut delta = None;
    let mut evidence = Vec::new();
    let mut reason = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--occasion" => {
                idx += 1;
                occasion_id = Some(required_flag_value(rest, idx, "--occasion")?);
            }
            v if v.starts_with("--occasion=") => {
                occasion_id = Some(v["--occasion=".len()..].to_string())
            }
            "--object" => {
                idx += 1;
                object_id = Some(required_flag_value(rest, idx, "--object")?);
            }
            v if v.starts_with("--object=") => object_id = Some(v["--object=".len()..].to_string()),
            "--delta" => {
                idx += 1;
                delta = Some(parse_f64_flag(
                    &required_flag_value(rest, idx, "--delta")?,
                    "delta",
                )?);
            }
            v if v.starts_with("--delta=") => {
                delta = Some(parse_f64_flag(&v["--delta=".len()..], "delta")?)
            }
            "--evidence" => {
                idx += 1;
                evidence = split_csv(&required_flag_value(rest, idx, "--evidence")?);
            }
            v if v.starts_with("--evidence=") => evidence = split_csv(&v["--evidence=".len()..]),
            "--reason" => {
                idx += 1;
                reason = Some(required_flag_value(rest, idx, "--reason")?);
            }
            v if v.starts_with("--reason=") => reason = Some(v["--reason=".len()..].to_string()),
            v => {
                return Err(agent_memory_invalid(format!(
                    "unsupported argument `{v}` for `object reinforce`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentMemoryVerb::ObjectReinforce {
        occasion_id: occasion_id
            .ok_or_else(|| agent_memory_invalid("`object reinforce` requires `--occasion`"))?,
        op: ReinforceObjectWeightOp {
            object_id: object_id
                .ok_or_else(|| agent_memory_invalid("`object reinforce` requires `--object`"))?,
            delta: delta
                .ok_or_else(|| agent_memory_invalid("`object reinforce` requires `--delta`"))?,
            reason: reason
                .ok_or_else(|| agent_memory_invalid("`object reinforce` requires `--reason`"))?,
            evidence,
        },
    })
}

fn parse_agent_memory_observe(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    if rest.first().map(String::as_str) != Some("add") {
        return Err(agent_memory_invalid("`observe` expects subcommand `add`"));
    }
    let mut occasion_id = None;
    let mut kind = None;
    let mut entities = Vec::new();
    let mut confidence = None;
    let mut content_parts = Vec::new();
    let mut idx = 1usize;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--occasion" => {
                idx += 1;
                occasion_id = Some(required_flag_value(rest, idx, "--occasion")?);
            }
            v if v.starts_with("--occasion=") => {
                occasion_id = Some(v["--occasion=".len()..].to_string())
            }
            "--kind" => {
                idx += 1;
                kind = Some(required_flag_value(rest, idx, "--kind")?);
            }
            v if v.starts_with("--kind=") => kind = Some(v["--kind=".len()..].to_string()),
            "--entities" => {
                idx += 1;
                entities = split_csv(&required_flag_value(rest, idx, "--entities")?);
            }
            v if v.starts_with("--entities=") => entities = split_csv(&v["--entities=".len()..]),
            "--confidence" => {
                idx += 1;
                confidence = Some(parse_f64_flag(
                    &required_flag_value(rest, idx, "--confidence")?,
                    "confidence",
                )?);
            }
            v if v.starts_with("--confidence=") => {
                confidence = Some(parse_f64_flag(&v["--confidence=".len()..], "confidence")?)
            }
            v if v.starts_with("--") => {
                return Err(agent_memory_invalid(format!(
                    "unsupported flag `{v}` for `observe add`"
                )))
            }
            v => content_parts.push(v.to_string()),
        }
        idx += 1;
    }
    Ok(AgentMemoryVerb::ObserveAdd {
        occasion_id: occasion_id
            .ok_or_else(|| agent_memory_invalid("`observe add` requires `--occasion`"))?,
        op: AddObservationOp {
            observation_id: None,
            kind: kind.ok_or_else(|| agent_memory_invalid("`observe add` requires `--kind`"))?,
            entities,
            content: content_parts.join(" "),
            source_excerpt: None,
            source_ref: None,
            confidence: confidence
                .ok_or_else(|| agent_memory_invalid("`observe add` requires `--confidence`"))?,
        },
    })
}

fn parse_agent_memory_set(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut reason: Option<String> = None;
    let mut entities = Vec::new();
    let mut tags = Vec::new();
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--reason" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_memory_invalid("missing value for `--reason`"))?;
                reason = Some(value.clone());
            }
            v if v.starts_with("--reason=") => {
                reason = Some(v["--reason=".len()..].to_string());
            }
            "--entities" => {
                idx += 1;
                entities = split_csv(&required_flag_value(rest, idx, "--entities")?);
            }
            v if v.starts_with("--entities=") => {
                entities = split_csv(&v["--entities=".len()..]);
            }
            "--tags" => {
                idx += 1;
                tags = split_csv(&required_flag_value(rest, idx, "--tags")?);
            }
            v if v.starts_with("--tags=") => {
                tags = split_csv(&v["--tags=".len()..]);
            }
            v if v.starts_with("--") => {
                return Err(agent_memory_invalid(format!(
                    "unsupported flag `{v}` for `set`"
                )));
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    let reason = reason.ok_or_else(|| agent_memory_invalid("`set` requires `--reason`"))?;
    if reason.trim().is_empty() {
        return Err(agent_memory_invalid("`--reason` must not be empty"));
    }
    match positionals.len() {
        2 => {
            let mut it = positionals.into_iter();
            let key = it.next().unwrap();
            let content = it.next().unwrap();
            Ok(AgentMemoryVerb::Set {
                key,
                content: Some(content),
                reason,
                entities,
                tags,
            })
        }
        1 => {
            let key = positionals.into_iter().next().unwrap();
            Ok(AgentMemoryVerb::Set {
                key,
                content: None,
                reason,
                entities,
                tags,
            })
        }
        n => Err(agent_memory_invalid(format!(
            "`set` expects 1 or 2 positional arguments, got {n}"
        ))),
    }
}

fn parse_agent_memory_remove(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut reason: Option<String> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--reason" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_memory_invalid("missing value for `--reason`"))?;
                reason = Some(value.clone());
            }
            v if v.starts_with("--reason=") => {
                reason = Some(v["--reason=".len()..].to_string());
            }
            v if v.starts_with("--") => {
                return Err(agent_memory_invalid(format!(
                    "unsupported flag `{v}` for `remove`"
                )));
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_memory_invalid(format!(
            "`remove` expects exactly 1 positional argument (key), got {}",
            positionals.len()
        )));
    }
    Ok(AgentMemoryVerb::Remove {
        key: positionals.into_iter().next().unwrap(),
        reason,
    })
}

fn parse_agent_memory_get(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    if rest.len() == 1 {
        return Ok(AgentMemoryVerb::Get {
            kind: None,
            id: rest[0].clone(),
        });
    }
    if rest.len() == 2 {
        return Ok(AgentMemoryVerb::Get {
            kind: Some(rest[0].clone()),
            id: rest[1].clone(),
        });
    }
    if rest.len() != 1 {
        return Err(agent_memory_invalid(format!(
            "`get` expects `<key>` or `<item|object|observation> <id>`, got {} arguments",
            rest.len()
        )));
    }
    unreachable!()
}

fn parse_agent_memory_list(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    if rest.first().map(String::as_str) == Some("objects") {
        let mut kind = None;
        let mut idx = 1usize;
        while idx < rest.len() {
            match rest[idx].as_str() {
                "--kind" => {
                    idx += 1;
                    kind = Some(required_flag_value(rest, idx, "--kind")?);
                }
                v if v.starts_with("--kind=") => kind = Some(v["--kind=".len()..].to_string()),
                v => {
                    return Err(agent_memory_invalid(format!(
                        "unsupported argument `{v}` for `list objects`"
                    )))
                }
            }
            idx += 1;
        }
        return Ok(AgentMemoryVerb::ListObjects { kind });
    }
    match rest.len() {
        0 => Ok(AgentMemoryVerb::List { prefix: None }),
        1 => Ok(AgentMemoryVerb::List {
            prefix: Some(rest[0].clone()),
        }),
        n => Err(agent_memory_invalid(format!(
            "`list` expects 0 or 1 positional arguments, got {n}"
        ))),
    }
}

fn parse_agent_memory_load(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut tags_arg: Option<String> = None;
    let mut objects = Vec::new();
    let mut aliases = Vec::new();
    let mut max_records: Option<usize> = None;
    let mut max_bytes: Option<usize> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--max-records" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_memory_invalid("missing value for `--max-records`"))?;
                max_records = Some(parse_load_count(value, "max-records")?);
            }
            v if v.starts_with("--max-records=") => {
                max_records = Some(parse_load_count(
                    &v["--max-records=".len()..],
                    "max-records",
                )?);
            }
            "--max-bytes" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_memory_invalid("missing value for `--max-bytes`"))?;
                max_bytes = Some(parse_load_count(value, "max-bytes")?);
            }
            v if v.starts_with("--max-bytes=") => {
                max_bytes = Some(parse_load_count(&v["--max-bytes=".len()..], "max-bytes")?);
            }
            "--tags" => {
                idx += 1;
                tags_arg = Some(required_flag_value(rest, idx, "--tags")?);
            }
            v if v.starts_with("--tags=") => tags_arg = Some(v["--tags=".len()..].to_string()),
            "--objects" => {
                idx += 1;
                objects = split_csv(&required_flag_value(rest, idx, "--objects")?);
            }
            v if v.starts_with("--objects=") => objects = split_csv(&v["--objects=".len()..]),
            "--aliases" => {
                idx += 1;
                aliases = split_csv(&required_flag_value(rest, idx, "--aliases")?);
            }
            v if v.starts_with("--aliases=") => aliases = split_csv(&v["--aliases=".len()..]),
            v if v.starts_with("--") => {
                return Err(agent_memory_invalid(format!(
                    "unsupported flag `{v}` for `load`"
                )));
            }
            v => {
                if tags_arg.is_some() {
                    return Err(agent_memory_invalid(
                        "`load` takes a single positional <tag1,tag2,...>",
                    ));
                }
                tags_arg = Some(v.to_string());
            }
        }
        idx += 1;
    }

    let raw_tags = tags_arg.unwrap_or_default();
    let tags: Vec<String> = if raw_tags.is_empty() {
        Vec::new()
    } else {
        raw_tags
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    Ok(AgentMemoryVerb::Load {
        tags,
        objects,
        aliases,
        max_records,
        max_bytes,
    })
}

fn parse_agent_memory_relate(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut occasion_id = None;
    let mut subject = None;
    let mut predicate = None;
    let mut object = None;
    let mut weight = None;
    let mut confidence = None;
    let mut evidence = Vec::new();
    let mut reason = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--occasion" => {
                idx += 1;
                occasion_id = Some(required_flag_value(rest, idx, "--occasion")?);
            }
            v if v.starts_with("--occasion=") => {
                occasion_id = Some(v["--occasion=".len()..].to_string())
            }
            "--subject" => {
                idx += 1;
                subject = Some(required_flag_value(rest, idx, "--subject")?);
            }
            v if v.starts_with("--subject=") => subject = Some(v["--subject=".len()..].to_string()),
            "--predicate" => {
                idx += 1;
                predicate = Some(required_flag_value(rest, idx, "--predicate")?);
            }
            v if v.starts_with("--predicate=") => {
                predicate = Some(v["--predicate=".len()..].to_string())
            }
            "--object" => {
                idx += 1;
                object = Some(required_flag_value(rest, idx, "--object")?);
            }
            v if v.starts_with("--object=") => object = Some(v["--object=".len()..].to_string()),
            "--weight" => {
                idx += 1;
                weight = Some(parse_f64_flag(
                    &required_flag_value(rest, idx, "--weight")?,
                    "weight",
                )?);
            }
            v if v.starts_with("--weight=") => {
                weight = Some(parse_f64_flag(&v["--weight=".len()..], "weight")?)
            }
            "--confidence" => {
                idx += 1;
                confidence = Some(parse_f64_flag(
                    &required_flag_value(rest, idx, "--confidence")?,
                    "confidence",
                )?);
            }
            v if v.starts_with("--confidence=") => {
                confidence = Some(parse_f64_flag(&v["--confidence=".len()..], "confidence")?)
            }
            "--evidence" => {
                idx += 1;
                evidence = split_csv(&required_flag_value(rest, idx, "--evidence")?);
            }
            v if v.starts_with("--evidence=") => evidence = split_csv(&v["--evidence=".len()..]),
            "--reason" => {
                idx += 1;
                reason = Some(required_flag_value(rest, idx, "--reason")?);
            }
            v if v.starts_with("--reason=") => reason = Some(v["--reason=".len()..].to_string()),
            v => {
                return Err(agent_memory_invalid(format!(
                    "unsupported argument `{v}` for `relate`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentMemoryVerb::Relate {
        occasion_id: occasion_id
            .ok_or_else(|| agent_memory_invalid("`relate` requires `--occasion`"))?,
        op: UpsertRelationOp {
            subject: subject
                .ok_or_else(|| agent_memory_invalid("`relate` requires `--subject`"))?,
            predicate: predicate
                .ok_or_else(|| agent_memory_invalid("`relate` requires `--predicate`"))?,
            object: object.ok_or_else(|| agent_memory_invalid("`relate` requires `--object`"))?,
            weight: weight.ok_or_else(|| agent_memory_invalid("`relate` requires `--weight`"))?,
            confidence: confidence
                .ok_or_else(|| agent_memory_invalid("`relate` requires `--confidence`"))?,
            evidence,
            write_reason: reason
                .ok_or_else(|| agent_memory_invalid("`relate` requires `--reason`"))?,
            replaces: Vec::new(),
        },
    })
}

fn parse_agent_memory_set_status(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut occasion_id = None;
    let mut target_kind = None;
    let mut target_id = None;
    let mut status = None;
    let mut reason = None;
    let mut replaced_by = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        match rest[idx].as_str() {
            "--occasion" => {
                idx += 1;
                occasion_id = Some(required_flag_value(rest, idx, "--occasion")?);
            }
            v if v.starts_with("--occasion=") => {
                occasion_id = Some(v["--occasion=".len()..].to_string())
            }
            "--target-kind" => {
                idx += 1;
                target_kind = Some(required_flag_value(rest, idx, "--target-kind")?);
            }
            v if v.starts_with("--target-kind=") => {
                target_kind = Some(v["--target-kind=".len()..].to_string())
            }
            "--target" => {
                idx += 1;
                target_id = Some(required_flag_value(rest, idx, "--target")?);
            }
            v if v.starts_with("--target=") => target_id = Some(v["--target=".len()..].to_string()),
            "--status" => {
                idx += 1;
                status = Some(required_flag_value(rest, idx, "--status")?);
            }
            v if v.starts_with("--status=") => status = Some(v["--status=".len()..].to_string()),
            "--reason" => {
                idx += 1;
                reason = Some(required_flag_value(rest, idx, "--reason")?);
            }
            v if v.starts_with("--reason=") => reason = Some(v["--reason=".len()..].to_string()),
            "--replaced-by" => {
                idx += 1;
                replaced_by = Some(required_flag_value(rest, idx, "--replaced-by")?);
            }
            v if v.starts_with("--replaced-by=") => {
                replaced_by = Some(v["--replaced-by=".len()..].to_string())
            }
            v => {
                return Err(agent_memory_invalid(format!(
                    "unsupported argument `{v}` for `set-status`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentMemoryVerb::SetStatus {
        occasion_id: occasion_id
            .ok_or_else(|| agent_memory_invalid("`set-status` requires `--occasion`"))?,
        op: SetStatusOp {
            target_kind: target_kind
                .ok_or_else(|| agent_memory_invalid("`set-status` requires `--target-kind`"))?,
            target_id: target_id
                .ok_or_else(|| agent_memory_invalid("`set-status` requires `--target`"))?,
            status: status
                .ok_or_else(|| agent_memory_invalid("`set-status` requires `--status`"))?,
            reason: reason
                .ok_or_else(|| agent_memory_invalid("`set-status` requires `--reason`"))?,
            replaced_by,
        },
    })
}

fn parse_load_count(raw: &str, name: &str) -> Result<usize, AgentToolError> {
    raw.trim()
        .parse::<usize>()
        .map_err(|_| agent_memory_invalid(format!("invalid `--{name}` value `{raw}`")))
}

fn parse_f64_flag(raw: &str, name: &str) -> Result<f64, AgentToolError> {
    raw.trim()
        .parse::<f64>()
        .map_err(|_| agent_memory_invalid(format!("invalid `--{name}` value `{raw}`")))
}

fn required_flag_value(
    tokens: &[String],
    idx: usize,
    flag: &str,
) -> Result<String, AgentToolError> {
    tokens
        .get(idx)
        .cloned()
        .ok_or_else(|| agent_memory_invalid(format!("missing value for `{flag}`")))
}

fn parse_source_ref(raw: &str) -> Result<SourceRef, AgentToolError> {
    serde_json::from_str(raw)
        .map_err(|err| agent_memory_invalid(format!("invalid `--source` JSON: {err}")))
}

fn parse_agent_memory_verify(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    let mut repair = false;
    for token in rest {
        match token.as_str() {
            "--repair" => repair = true,
            v => {
                return Err(agent_memory_invalid(format!(
                    "unsupported argument `{v}` for `verify`"
                )))
            }
        }
    }
    Ok(AgentMemoryVerb::Verify { repair })
}

fn parse_agent_memory_compact(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
    if !rest.is_empty() {
        return Err(agent_memory_invalid(format!(
            "`compact` takes no arguments, got `{}`",
            rest.join(" ")
        )));
    }
    Ok(AgentMemoryVerb::Compact)
}

// =================================================================
//  agent-skills CLI
// =================================================================

const AGENT_SKILLS_USAGE: &str = "agent-skills [--root <path> | env AGENT_SKILLS_ROOT] \
<init|install|list|hints|explorer-hints|view|promote|archive|restore|validate|\
select-session|select-todo|unselect-session|unselect-todo|selected-list|render-selected|\
record-used|record-result> [...]";

fn agent_skills_invalid(message: impl Into<String>) -> AgentToolError {
    AgentToolError::InvalidArgs(format!("{}\nUsage: {}", message.into(), AGENT_SKILLS_USAGE))
}

fn parse_agent_skills_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    let mut root_override: Option<PathBuf> = None;
    let mut idx = 0usize;
    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "--root" => {
                idx += 1;
                root_override = Some(PathBuf::from(skill_required_flag(tokens, idx, "--root")?));
            }
            v if v.starts_with("--root=") => {
                root_override = Some(PathBuf::from(&v["--root=".len()..]));
            }
            _ => break,
        }
        idx += 1;
    }

    let verb_token = tokens
        .get(idx)
        .ok_or_else(|| agent_skills_invalid("missing verb"))?
        .clone();
    let rest = &tokens[idx + 1..];
    let verb = match verb_token.as_str() {
        "init" => {
            if !rest.is_empty() {
                return Err(agent_skills_invalid("`init` takes no arguments"));
            }
            AgentSkillsVerb::Init
        }
        "install" => parse_agent_skills_install(rest)?,
        "list" | "hints" => AgentSkillsVerb::List {
            filter: parse_agent_skills_filter(rest, false)?,
        },
        "explorer-hints" => AgentSkillsVerb::ExplorerHints {
            filter: parse_agent_skills_filter(rest, false)?,
        },
        "view" => parse_agent_skills_view(rest)?,
        "promote" => parse_agent_skills_promote(rest)?,
        "archive" => parse_agent_skills_archive(rest)?,
        "restore" => parse_agent_skills_restore(rest)?,
        "validate" => parse_agent_skills_validate(rest)?,
        "select-session" => parse_agent_skills_select_session(rest)?,
        "select-todo" => parse_agent_skills_select_todo(rest)?,
        "unselect-session" => parse_agent_skills_unselect_session(rest)?,
        "unselect-todo" => parse_agent_skills_unselect_todo(rest)?,
        "selected-list" => parse_agent_skills_selected_list(rest)?,
        "render-selected" => parse_agent_skills_render_selected(rest)?,
        "record-used" => parse_agent_skills_record_used(rest)?,
        "record-result" => parse_agent_skills_record_result(rest)?,
        other => return Err(agent_skills_invalid(format!("unknown verb `{other}`"))),
    };
    Ok(ParsedCommand::AgentSkills {
        tool_name,
        invocation: AgentSkillsInvocation {
            root_override,
            verb,
        },
    })
}

fn parse_agent_skills_install(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let mut positionals = Vec::new();
    let mut id = None;
    let mut name = None;
    let mut title = None;
    let mut description = None;
    let mut skill_type = None;
    let mut group_id = None;
    let mut intent_tags = Vec::new();
    let mut required_tools = Vec::new();
    let mut risk_level = None;
    let mut source_type = SkillSourceType::OwnerInstalled;
    let mut target_scope = SkillOwnerScope::Agent;
    let mut trust_hint = "medium".to_string();
    let mut installed_by = None;
    let mut promote = false;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--id" => {
                idx += 1;
                id = Some(skill_required_flag(rest, idx, "--id")?);
            }
            v if v.starts_with("--id=") => id = Some(v["--id=".len()..].to_string()),
            "--name" => {
                idx += 1;
                name = Some(skill_required_flag(rest, idx, "--name")?);
            }
            v if v.starts_with("--name=") => name = Some(v["--name=".len()..].to_string()),
            "--title" => {
                idx += 1;
                title = Some(skill_required_flag(rest, idx, "--title")?);
            }
            v if v.starts_with("--title=") => title = Some(v["--title=".len()..].to_string()),
            "--description" => {
                idx += 1;
                description = Some(skill_required_flag(rest, idx, "--description")?);
            }
            v if v.starts_with("--description=") => {
                description = Some(v["--description=".len()..].to_string())
            }
            "--type" => {
                idx += 1;
                skill_type = Some(parse_skill_type(&skill_required_flag(
                    rest, idx, "--type",
                )?)?);
            }
            v if v.starts_with("--type=") => {
                skill_type = Some(parse_skill_type(&v["--type=".len()..])?)
            }
            "--group" | "--group-id" => {
                idx += 1;
                group_id = Some(skill_required_flag(rest, idx, token)?);
            }
            v if v.starts_with("--group=") => group_id = Some(v["--group=".len()..].to_string()),
            v if v.starts_with("--group-id=") => {
                group_id = Some(v["--group-id=".len()..].to_string())
            }
            "--tags" => {
                idx += 1;
                intent_tags = split_csv(&skill_required_flag(rest, idx, "--tags")?);
            }
            v if v.starts_with("--tags=") => intent_tags = split_csv(&v["--tags=".len()..]),
            "--tools" => {
                idx += 1;
                required_tools = split_csv(&skill_required_flag(rest, idx, "--tools")?);
            }
            v if v.starts_with("--tools=") => required_tools = split_csv(&v["--tools=".len()..]),
            "--risk" | "--risk-level" => {
                idx += 1;
                risk_level = Some(parse_skill_risk(&skill_required_flag(rest, idx, token)?)?);
            }
            v if v.starts_with("--risk=") => {
                risk_level = Some(parse_skill_risk(&v["--risk=".len()..])?)
            }
            v if v.starts_with("--risk-level=") => {
                risk_level = Some(parse_skill_risk(&v["--risk-level=".len()..])?)
            }
            "--source-type" => {
                idx += 1;
                source_type =
                    parse_skill_source_type(&skill_required_flag(rest, idx, "--source-type")?)?;
            }
            v if v.starts_with("--source-type=") => {
                source_type = parse_skill_source_type(&v["--source-type=".len()..])?
            }
            "--scope" => {
                idx += 1;
                target_scope = parse_skill_scope(&skill_required_flag(rest, idx, "--scope")?)?;
            }
            v if v.starts_with("--scope=") => {
                target_scope = parse_skill_scope(&v["--scope=".len()..])?
            }
            "--trust" | "--trust-hint" => {
                idx += 1;
                trust_hint = skill_required_flag(rest, idx, token)?;
            }
            v if v.starts_with("--trust=") => trust_hint = v["--trust=".len()..].to_string(),
            v if v.starts_with("--trust-hint=") => {
                trust_hint = v["--trust-hint=".len()..].to_string()
            }
            "--installed-by" | "--actor" => {
                idx += 1;
                installed_by = Some(skill_required_flag(rest, idx, token)?);
            }
            v if v.starts_with("--installed-by=") => {
                installed_by = Some(v["--installed-by=".len()..].to_string())
            }
            v if v.starts_with("--actor=") => {
                installed_by = Some(v["--actor=".len()..].to_string())
            }
            "--promote" => promote = true,
            v if v.starts_with("--") => {
                return Err(agent_skills_invalid(format!(
                    "unsupported flag `{v}` for `install`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_skills_invalid(format!(
            "`install` expects exactly one package/file path, got {}",
            positionals.len()
        )));
    }
    Ok(AgentSkillsVerb::Install {
        path: PathBuf::from(positionals.remove(0)),
        id,
        name,
        title,
        description,
        skill_type,
        group_id,
        intent_tags,
        required_tools,
        risk_level,
        source_type,
        target_scope,
        trust_hint,
        installed_by,
        promote,
    })
}

fn parse_agent_skills_filter(
    rest: &[String],
    include_all_default: bool,
) -> Result<ListSkillsInput, AgentToolError> {
    let mut filter = ListSkillsInput::default();
    if include_all_default {
        filter.states = vec![
            SkillLifecycleState::Candidate,
            SkillLifecycleState::Active,
            SkillLifecycleState::Preferred,
            SkillLifecycleState::NeedsReverification,
            SkillLifecycleState::Stale,
            SkillLifecycleState::Archived,
            SkillLifecycleState::Blocked,
            SkillLifecycleState::Rejected,
        ];
    }
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--all" => {
                filter.states = vec![
                    SkillLifecycleState::Candidate,
                    SkillLifecycleState::Active,
                    SkillLifecycleState::Preferred,
                    SkillLifecycleState::NeedsReverification,
                    SkillLifecycleState::Stale,
                    SkillLifecycleState::Archived,
                    SkillLifecycleState::Blocked,
                    SkillLifecycleState::Rejected,
                ];
            }
            "--state" => {
                idx += 1;
                filter.states =
                    parse_skill_states_csv(&skill_required_flag(rest, idx, "--state")?)?;
            }
            v if v.starts_with("--state=") => {
                filter.states = parse_skill_states_csv(&v["--state=".len()..])?
            }
            "--type" => {
                idx += 1;
                filter.types = parse_skill_types_csv(&skill_required_flag(rest, idx, "--type")?)?;
            }
            v if v.starts_with("--type=") => {
                filter.types = parse_skill_types_csv(&v["--type=".len()..])?
            }
            "--tags" => {
                idx += 1;
                filter.intent_tags = split_csv(&skill_required_flag(rest, idx, "--tags")?);
            }
            v if v.starts_with("--tags=") => filter.intent_tags = split_csv(&v["--tags=".len()..]),
            "--object-types" => {
                idx += 1;
                filter.object_types = split_csv(&skill_required_flag(rest, idx, "--object-types")?);
            }
            v if v.starts_with("--object-types=") => {
                filter.object_types = split_csv(&v["--object-types=".len()..])
            }
            "--tools" => {
                idx += 1;
                filter.required_tools = split_csv(&skill_required_flag(rest, idx, "--tools")?);
            }
            v if v.starts_with("--tools=") => {
                filter.required_tools = split_csv(&v["--tools=".len()..])
            }
            "--risk-budget" => {
                idx += 1;
                filter.risk_budget = Some(parse_skill_risk(&skill_required_flag(
                    rest,
                    idx,
                    "--risk-budget",
                )?)?);
            }
            v if v.starts_with("--risk-budget=") => {
                filter.risk_budget = Some(parse_skill_risk(&v["--risk-budget=".len()..])?)
            }
            "--limit" => {
                idx += 1;
                filter.limit = Some(parse_skill_usize(
                    &skill_required_flag(rest, idx, "--limit")?,
                    "limit",
                )?);
            }
            v if v.starts_with("--limit=") => {
                filter.limit = Some(parse_skill_usize(&v["--limit=".len()..], "limit")?)
            }
            v => {
                return Err(agent_skills_invalid(format!(
                    "unsupported filter arg `{v}`"
                )))
            }
        }
        idx += 1;
    }
    Ok(filter)
}

fn parse_agent_skills_view(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    if rest.is_empty() || rest.len() > 2 {
        return Err(agent_skills_invalid(
            "`view` expects <skill_ref> [package_path]",
        ));
    }
    Ok(AgentSkillsVerb::View {
        skill_ref: rest[0].clone(),
        package_path: rest.get(1).cloned(),
    })
}

fn parse_agent_skills_promote(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (skill_ref, actor) = parse_skill_ref_actor(rest, "promote")?;
    Ok(AgentSkillsVerb::Promote { skill_ref, actor })
}

fn parse_agent_skills_archive(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (skill_ref, actor, reason) = parse_skill_ref_actor_reason(rest, "archive")?;
    Ok(AgentSkillsVerb::Archive {
        skill_ref,
        actor,
        reason,
    })
}

fn parse_agent_skills_restore(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (skill_ref, actor, reason) = parse_skill_ref_actor_reason(rest, "restore")?;
    Ok(AgentSkillsVerb::Restore {
        skill_ref,
        actor,
        reason,
    })
}

fn parse_agent_skills_validate(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (requested, max_count, allowed_states, risk_budget, _session, _todo) =
        parse_selection_like_args(rest, "validate")?;
    Ok(AgentSkillsVerb::Validate {
        requested,
        max_count,
        allowed_states,
        risk_budget,
    })
}

fn parse_agent_skills_select_session(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (requested, max_count, _allowed_states, risk_budget, session_id, _todo) =
        parse_selection_like_args(rest, "select-session")?;
    Ok(AgentSkillsVerb::SelectSession {
        session_id,
        requested,
        risk_budget,
        max_count,
    })
}

fn parse_agent_skills_select_todo(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (requested, max_count, _allowed_states, risk_budget, session_id, todo_id) =
        parse_selection_like_args(rest, "select-todo")?;
    Ok(AgentSkillsVerb::SelectTodo {
        session_id,
        todo_id: todo_id.ok_or_else(|| agent_skills_invalid("`select-todo` requires `--todo`"))?,
        requested,
        risk_budget,
        max_count,
    })
}

fn parse_agent_skills_unselect_session(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (skill_refs, _max_count, _allowed_states, _risk_budget, session_id, _todo) =
        parse_selection_like_args(rest, "unselect-session")?;
    Ok(AgentSkillsVerb::UnselectSession {
        session_id,
        skill_refs,
    })
}

fn parse_agent_skills_unselect_todo(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (skill_refs, _max_count, _allowed_states, _risk_budget, session_id, todo_id) =
        parse_selection_like_args(rest, "unselect-todo")?;
    Ok(AgentSkillsVerb::UnselectTodo {
        session_id,
        todo_id: todo_id
            .ok_or_else(|| agent_skills_invalid("`unselect-todo` requires `--todo`"))?,
        skill_refs,
    })
}

fn parse_agent_skills_selected_list(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let (session_id, todo_id) = parse_session_todo_flags(rest, "selected-list")?;
    Ok(AgentSkillsVerb::SelectedList {
        session_id,
        todo_id,
    })
}

fn parse_agent_skills_render_selected(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let mut session_id = None;
    let mut todo_id = None;
    let mut max_skills = DEFAULT_MAX_SELECTED_SKILLS;
    let mut token_budget = DEFAULT_RENDER_TOKEN_BUDGET;
    let mut include_metadata = true;
    let mut include_usage_instructions = true;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--session" => {
                idx += 1;
                session_id = Some(skill_required_flag(rest, idx, "--session")?);
            }
            v if v.starts_with("--session=") => {
                session_id = Some(v["--session=".len()..].to_string())
            }
            "--todo" => {
                idx += 1;
                todo_id = Some(skill_required_flag(rest, idx, "--todo")?);
            }
            v if v.starts_with("--todo=") => todo_id = Some(v["--todo=".len()..].to_string()),
            "--max-skills" => {
                idx += 1;
                max_skills = parse_skill_usize(
                    &skill_required_flag(rest, idx, "--max-skills")?,
                    "max-skills",
                )?;
            }
            v if v.starts_with("--max-skills=") => {
                max_skills = parse_skill_usize(&v["--max-skills=".len()..], "max-skills")?
            }
            "--token-budget" => {
                idx += 1;
                token_budget = parse_skill_usize(
                    &skill_required_flag(rest, idx, "--token-budget")?,
                    "token-budget",
                )?;
            }
            v if v.starts_with("--token-budget=") => {
                token_budget = parse_skill_usize(&v["--token-budget=".len()..], "token-budget")?
            }
            "--no-metadata" => include_metadata = false,
            "--no-usage-instructions" => include_usage_instructions = false,
            v => {
                return Err(agent_skills_invalid(format!(
                    "unsupported arg `{v}` for `render-selected`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentSkillsVerb::RenderSelected {
        session_id,
        todo_id,
        max_skills,
        token_budget,
        include_metadata,
        include_usage_instructions,
    })
}

fn parse_agent_skills_record_used(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let mut positionals = Vec::new();
    let mut mode = SkillUsageMode::Applied;
    let mut evidence = Vec::new();
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--mode" => {
                idx += 1;
                mode = parse_skill_usage_mode(&skill_required_flag(rest, idx, "--mode")?)?;
            }
            v if v.starts_with("--mode=") => mode = parse_skill_usage_mode(&v["--mode=".len()..])?,
            "--evidence" => {
                idx += 1;
                evidence = split_csv(&skill_required_flag(rest, idx, "--evidence")?);
            }
            v if v.starts_with("--evidence=") => evidence = split_csv(&v["--evidence=".len()..]),
            v if v.starts_with("--") => {
                return Err(agent_skills_invalid(format!(
                    "unsupported flag `{v}` for `record-used`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_skills_invalid("`record-used` expects <usage_id>"));
    }
    Ok(AgentSkillsVerb::RecordUsed {
        usage_id: positionals.remove(0),
        mode,
        evidence,
    })
}

fn parse_agent_skills_record_result(rest: &[String]) -> Result<AgentSkillsVerb, AgentToolError> {
    let mut positionals = Vec::new();
    let mut result = None;
    let mut feedback = None;
    let mut failure_reason = None;
    let mut output_refs = Vec::new();
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--result" => {
                idx += 1;
                result = Some(parse_skill_task_result(&skill_required_flag(
                    rest, idx, "--result",
                )?)?);
            }
            v if v.starts_with("--result=") => {
                result = Some(parse_skill_task_result(&v["--result=".len()..])?)
            }
            "--feedback" => {
                idx += 1;
                feedback = Some(parse_skill_feedback(&skill_required_flag(
                    rest,
                    idx,
                    "--feedback",
                )?)?);
            }
            v if v.starts_with("--feedback=") => {
                feedback = Some(parse_skill_feedback(&v["--feedback=".len()..])?)
            }
            "--failure-reason" => {
                idx += 1;
                failure_reason = Some(skill_required_flag(rest, idx, "--failure-reason")?);
            }
            v if v.starts_with("--failure-reason=") => {
                failure_reason = Some(v["--failure-reason=".len()..].to_string())
            }
            "--output-refs" => {
                idx += 1;
                output_refs = split_csv(&skill_required_flag(rest, idx, "--output-refs")?);
            }
            v if v.starts_with("--output-refs=") => {
                output_refs = split_csv(&v["--output-refs=".len()..])
            }
            v if v.starts_with("--") => {
                return Err(agent_skills_invalid(format!(
                    "unsupported flag `{v}` for `record-result`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_skills_invalid("`record-result` expects <usage_id>"));
    }
    Ok(AgentSkillsVerb::RecordResult {
        usage_id: positionals.remove(0),
        result: result
            .ok_or_else(|| agent_skills_invalid("`record-result` requires `--result`"))?,
        feedback,
        failure_reason,
        output_refs,
    })
}

fn parse_skill_ref_actor(
    rest: &[String],
    verb: &str,
) -> Result<(String, Option<String>), AgentToolError> {
    let mut positionals = Vec::new();
    let mut actor = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--actor" => {
                idx += 1;
                actor = Some(skill_required_flag(rest, idx, "--actor")?);
            }
            v if v.starts_with("--actor=") => actor = Some(v["--actor=".len()..].to_string()),
            v if v.starts_with("--") => {
                return Err(agent_skills_invalid(format!(
                    "unsupported flag `{v}` for `{verb}`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_skills_invalid(format!(
            "`{verb}` expects <skill_ref>"
        )));
    }
    Ok((positionals.remove(0), actor))
}

fn parse_skill_ref_actor_reason(
    rest: &[String],
    verb: &str,
) -> Result<(String, Option<String>, String), AgentToolError> {
    let mut positionals = Vec::new();
    let mut actor = None;
    let mut reason = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--actor" => {
                idx += 1;
                actor = Some(skill_required_flag(rest, idx, "--actor")?);
            }
            v if v.starts_with("--actor=") => actor = Some(v["--actor=".len()..].to_string()),
            "--reason" => {
                idx += 1;
                reason = Some(skill_required_flag(rest, idx, "--reason")?);
            }
            v if v.starts_with("--reason=") => reason = Some(v["--reason=".len()..].to_string()),
            v if v.starts_with("--") => {
                return Err(agent_skills_invalid(format!(
                    "unsupported flag `{v}` for `{verb}`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_skills_invalid(format!(
            "`{verb}` expects <skill_ref>"
        )));
    }
    Ok((
        positionals.remove(0),
        actor,
        reason.ok_or_else(|| agent_skills_invalid(format!("`{verb}` requires `--reason`")))?,
    ))
}

fn parse_selection_like_args(
    rest: &[String],
    verb: &str,
) -> Result<
    (
        Vec<String>,
        usize,
        Vec<SkillLifecycleState>,
        SkillRiskLevel,
        Option<String>,
        Option<String>,
    ),
    AgentToolError,
> {
    let mut requested = Vec::new();
    let mut max_count = DEFAULT_MAX_SELECTED_SKILLS;
    let mut allowed_states = vec![SkillLifecycleState::Active, SkillLifecycleState::Preferred];
    let mut risk_budget = SkillRiskLevel::Medium;
    let mut session_id = None;
    let mut todo_id = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--max-count" => {
                idx += 1;
                max_count = parse_skill_usize(
                    &skill_required_flag(rest, idx, "--max-count")?,
                    "max-count",
                )?;
            }
            v if v.starts_with("--max-count=") => {
                max_count = parse_skill_usize(&v["--max-count=".len()..], "max-count")?
            }
            "--allowed-states" => {
                idx += 1;
                allowed_states =
                    parse_skill_states_csv(&skill_required_flag(rest, idx, "--allowed-states")?)?;
            }
            v if v.starts_with("--allowed-states=") => {
                allowed_states = parse_skill_states_csv(&v["--allowed-states=".len()..])?
            }
            "--risk-budget" => {
                idx += 1;
                risk_budget = parse_skill_risk(&skill_required_flag(rest, idx, "--risk-budget")?)?;
            }
            v if v.starts_with("--risk-budget=") => {
                risk_budget = parse_skill_risk(&v["--risk-budget=".len()..])?
            }
            "--session" => {
                idx += 1;
                session_id = Some(skill_required_flag(rest, idx, "--session")?);
            }
            v if v.starts_with("--session=") => {
                session_id = Some(v["--session=".len()..].to_string())
            }
            "--todo" => {
                idx += 1;
                todo_id = Some(skill_required_flag(rest, idx, "--todo")?);
            }
            v if v.starts_with("--todo=") => todo_id = Some(v["--todo=".len()..].to_string()),
            v if v.starts_with("--") => {
                return Err(agent_skills_invalid(format!(
                    "unsupported flag `{v}` for `{verb}`"
                )))
            }
            v => requested.push(v.to_string()),
        }
        idx += 1;
    }
    if requested.is_empty() && !matches!(verb, "selected-list" | "render-selected") {
        return Err(agent_skills_invalid(format!(
            "`{verb}` expects at least one skill ref"
        )));
    }
    Ok((
        requested,
        max_count,
        allowed_states,
        risk_budget,
        session_id,
        todo_id,
    ))
}

fn parse_session_todo_flags(
    rest: &[String],
    verb: &str,
) -> Result<(Option<String>, Option<String>), AgentToolError> {
    let mut session_id = None;
    let mut todo_id = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--session" => {
                idx += 1;
                session_id = Some(skill_required_flag(rest, idx, "--session")?);
            }
            v if v.starts_with("--session=") => {
                session_id = Some(v["--session=".len()..].to_string())
            }
            "--todo" => {
                idx += 1;
                todo_id = Some(skill_required_flag(rest, idx, "--todo")?);
            }
            v if v.starts_with("--todo=") => todo_id = Some(v["--todo=".len()..].to_string()),
            v => {
                return Err(agent_skills_invalid(format!(
                    "unsupported arg `{v}` for `{verb}`"
                )))
            }
        }
        idx += 1;
    }
    Ok((session_id, todo_id))
}

fn skill_required_flag(
    tokens: &[String],
    idx: usize,
    flag: &str,
) -> Result<String, AgentToolError> {
    tokens
        .get(idx)
        .cloned()
        .ok_or_else(|| agent_skills_invalid(format!("missing value for `{flag}`")))
}

fn parse_skill_usize(raw: &str, name: &str) -> Result<usize, AgentToolError> {
    raw.trim()
        .parse::<usize>()
        .map_err(|_| agent_skills_invalid(format!("invalid `--{name}` value `{raw}`")))
}

fn parse_skill_type(raw: &str) -> Result<SkillType, AgentToolError> {
    SkillType::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_types_csv(raw: &str) -> Result<Vec<SkillType>, AgentToolError> {
    split_csv(raw)
        .into_iter()
        .map(|value| parse_skill_type(&value))
        .collect()
}

fn parse_skill_source_type(raw: &str) -> Result<SkillSourceType, AgentToolError> {
    SkillSourceType::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_scope(raw: &str) -> Result<SkillOwnerScope, AgentToolError> {
    SkillOwnerScope::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_risk(raw: &str) -> Result<SkillRiskLevel, AgentToolError> {
    SkillRiskLevel::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_state(raw: &str) -> Result<SkillLifecycleState, AgentToolError> {
    SkillLifecycleState::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_states_csv(raw: &str) -> Result<Vec<SkillLifecycleState>, AgentToolError> {
    split_csv(raw)
        .into_iter()
        .map(|value| parse_skill_state(&value))
        .collect()
}

fn parse_skill_usage_mode(raw: &str) -> Result<SkillUsageMode, AgentToolError> {
    SkillUsageMode::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_task_result(raw: &str) -> Result<SkillTaskResult, AgentToolError> {
    SkillTaskResult::parse(raw).map_err(agent_skills_error)
}

fn parse_skill_feedback(raw: &str) -> Result<SkillUserFeedback, AgentToolError> {
    SkillUserFeedback::parse(raw).map_err(agent_skills_error)
}

fn agent_skills_error(err: SkillsMgrError) -> AgentToolError {
    AgentToolError::InvalidArgs(err.to_string())
}

fn resolve_agent_memory_root(env: &CliRuntimeEnv, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path {
        return canonicalize_or_normalize(p, Some(env.current_dir.as_path()));
    }
    if env.allow_dev_overrides() {
        if let Some(value) = first_path_env(&[AGENT_MEMORY_ROOT_ENV], &env.current_dir) {
            return value;
        }
    }
    cli_state_root(env).join(AGENT_MEMORY_DIR_NAME)
}

fn resolve_agent_notebook_root(env: &CliRuntimeEnv, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path {
        return canonicalize_or_normalize(p, Some(env.current_dir.as_path()));
    }
    if env.allow_dev_overrides() {
        if let Some(value) = first_path_env(&[AGENT_NOTEBOOK_ROOT_ENV], &env.current_dir) {
            return value;
        }
    }
    cli_state_root(env).join(AGENT_NOTEBOOK_DIR_NAME)
}

fn resolve_agent_skills_root(env: &CliRuntimeEnv, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path {
        return canonicalize_or_normalize(p, Some(env.current_dir.as_path()));
    }
    if env.allow_dev_overrides() {
        if let Some(value) = first_path_env(&[AGENT_SKILLS_ROOT_ENV], &env.current_dir) {
            return value;
        }
    }
    cli_state_root(env).join(SKILLS_DIR)
}

fn resolve_agent_notebook_session_id(env: &CliRuntimeEnv) -> Option<String> {
    Some(env.runtime_context.session_id.clone()).filter(|v| !v.trim().is_empty())
}

fn require_runtime_token_for_rpc(env: &CliRuntimeEnv) -> Result<(), AgentToolError> {
    env.runtime_context
        .require_appclient_session_token()
        .map(|_| ())
}

fn resolve_dev_task_manager_client() -> Option<TaskManagerClient> {
    let url = first_string_env(&[
        "OPENDAN_TASK_MANAGER_URL",
        "OPENDAN_TASK_MANAGER_RPC",
        "TASK_MANAGER_URL",
        "TASK_MANAGER_RPC",
    ])?;
    let session_token = first_string_env(&["OPENDAN_SESSION_TOKEN", "SESSION_TOKEN"]);
    Some(TaskManagerClient::new(kRPC::new(
        url.as_str(),
        session_token,
    )))
}

fn agent_memory_exit_code(err: &AgentMemoryError) -> i32 {
    err.exit_code()
}

/// Map an `AgentMemoryError` to a CLI run output. By spec §3 the default
/// channel is plain text on stdout and a short message on stderr; no JSON
/// envelope.
fn agent_memory_error_output(err: AgentMemoryError, quiet: bool) -> CliRunOutput {
    let exit_code = agent_memory_exit_code(&err);
    CliRunOutput {
        exit_code,
        stdout: String::new(),
        stderr: if quiet {
            String::new()
        } else {
            format!("{err}\n")
        },
    }
}

/// Execute one `agent-memory` invocation. Runs the synchronous library API
/// inside `spawn_blocking` so the async runtime is not stalled.
async fn dispatch_agent_memory(
    env: &CliRuntimeEnv,
    _tool_name: &str,
    invocation: AgentMemoryInvocation,
    stdin_override: Option<String>,
) -> CliRunOutput {
    let AgentMemoryInvocation {
        root_override,
        quiet,
        verb,
    } = invocation;

    let root = resolve_agent_memory_root(env, root_override);

    // `set` form B reads content from stdin BEFORE spawn_blocking so we can
    // surface the same async stdin path as the rest of the CLI.
    let resolved_verb = match verb {
        AgentMemoryVerb::Set {
            key,
            content,
            reason,
            entities,
            tags,
        } if content.is_none() => match read_stdin_content(stdin_override).await {
            Ok(content) => {
                if content.is_empty() {
                    return CliRunOutput {
                        exit_code: 1,
                        stdout: String::new(),
                        stderr: if quiet {
                            String::new()
                        } else {
                            "agent-memory: stdin produced 0 bytes; refusing empty content\n"
                                .to_string()
                        },
                    };
                }
                AgentMemoryVerb::Set {
                    key,
                    content: Some(content),
                    reason,
                    entities,
                    tags,
                }
            }
            Err(err) => {
                return CliRunOutput {
                    exit_code: 1,
                    stdout: String::new(),
                    stderr: if quiet {
                        String::new()
                    } else {
                        format!("{err}\n")
                    },
                }
            }
        },
        v => v,
    };

    let result =
        tokio::task::spawn_blocking(move || run_agent_memory_blocking(&root, resolved_verb))
            .await
            .unwrap_or_else(|join| {
                Err(AgentMemoryError::Invalid(format!(
                    "agent-memory worker panicked: {join}"
                )))
            });

    match result {
        Ok(stdout) => CliRunOutput {
            exit_code: 0,
            stdout,
            stderr: String::new(),
        },
        Err(err) => agent_memory_error_output(err, quiet),
    }
}

/// Stdin path for §4.2 form B. We honor `stdin_override` (used in tests) and
/// otherwise read all of stdin to EOF. Refusing TTY stdin is left to the
/// caller because the interactive notion is not meaningful in this harness.
async fn read_stdin_content(stdin_override: Option<String>) -> Result<String, AgentToolError> {
    if let Some(s) = stdin_override {
        return Ok(s);
    }
    let mut stdin = io::stdin();
    let mut buf = String::new();
    stdin
        .read_to_string(&mut buf)
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("read stdin failed: {err}")))?;
    Ok(buf)
}

/// Synchronous worker: opens the memory root and dispatches a single verb.
/// The returned `String` is the verb's stdout body per §5 (or empty for
/// verbs with no stdout output).
fn run_agent_memory_blocking(
    root: &Path,
    verb: AgentMemoryVerb,
) -> Result<String, AgentMemoryError> {
    let cfg = AgentMemoryConfig::new(root);
    let mem = AgentMemory::open(cfg)?;
    match verb {
        AgentMemoryVerb::Init => Ok(String::new()),
        AgentMemoryVerb::Set {
            key,
            content,
            reason,
            entities,
            tags,
        } => {
            let content = content.expect("stdin form resolved earlier");
            mem.set_free(FlatSetOp {
                key,
                content,
                reason,
                entities,
                tags,
                weight: None,
                confidence: None,
            })?;
            Ok(String::new())
        }
        AgentMemoryVerb::Remove { key, reason } => {
            mem.remove(&key, reason.as_deref())?;
            Ok(String::new())
        }
        AgentMemoryVerb::Get { kind, id } => match kind.as_deref() {
            None => mem.get(&id),
            Some("item") => mem.get_item_json(&id),
            Some("object") => mem.get_object_json(&id),
            Some("observation") => mem.get_observation_json(&id),
            Some(other) => Err(AgentMemoryError::Invalid(format!(
                "unsupported get kind `{other}`"
            ))),
        },
        AgentMemoryVerb::List { prefix } => {
            let keys = mem.list(prefix.as_deref())?;
            let mut out = keys.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            Ok(out)
        }
        AgentMemoryVerb::ListObjects { kind } => {
            let ids = mem.list_objects(kind.as_deref())?;
            let mut out = ids.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            Ok(out)
        }
        AgentMemoryVerb::OccasionAdd {
            occasion_type,
            summary,
            occurred_at,
            source_ref,
            tags,
        } => {
            let result = mem.add_occasion(OccasionAddInput {
                occasion_type,
                summary,
                occurred_at,
                source_ref,
                tags,
            })?;
            Ok(format!(
                "OCCASION {}\nSEQ {}\n",
                result.occasion_id, result.seq
            ))
        }
        AgentMemoryVerb::ObserveAdd { occasion_id, op } => {
            let wrapper = mem.observe(&occasion_id, op)?;
            Ok(format!("OCCASION {}\nSTATUS added\n", wrapper))
        }
        AgentMemoryVerb::ObjectUpsert { occasion_id, op } => {
            let wrapper = mem.upsert_object(&occasion_id, op)?;
            Ok(format!("OCCASION {}\nSTATUS updated\n", wrapper))
        }
        AgentMemoryVerb::ObjectReinforce { occasion_id, op } => {
            let wrapper = mem.reinforce_object(&occasion_id, op)?;
            Ok(format!("OCCASION {}\nSTATUS reinforced\n", wrapper))
        }
        AgentMemoryVerb::Relate { occasion_id, op } => {
            let wrapper = mem.relate(&occasion_id, op)?;
            Ok(format!("OCCASION {}\nSTATUS related\n", wrapper))
        }
        AgentMemoryVerb::SetStatus { occasion_id, op } => {
            let wrapper = mem.set_status(&occasion_id, op)?;
            Ok(format!("OCCASION {}\nSTATUS updated\n", wrapper))
        }
        AgentMemoryVerb::Load {
            tags,
            objects,
            aliases,
            max_records,
            max_bytes,
        } => {
            let mut opts = LoadOptions::default();
            opts.objects = objects;
            opts.aliases = aliases;
            if let Some(n) = max_records {
                opts.max_records = n;
            }
            if let Some(n) = max_bytes {
                opts.max_bytes = n;
            }
            let items = mem.load(&tags, opts)?;
            Ok(AgentMemory::format_load_items(&items))
        }
        AgentMemoryVerb::Verify { repair } => {
            let report = mem.verify(repair)?;
            Ok(format_verify_report(&report))
        }
        AgentMemoryVerb::Compact => {
            mem.compact()?;
            Ok(String::new())
        }
    }
}

fn format_verify_report(report: &agent_tool::VerifyReport) -> String {
    let mut out = String::new();
    out.push_str(&format!("OK_KEYS {}\n", report.ok_keys));
    out.push_str(&format!("ORPHAN_FILES {}\n", report.orphan_files.len()));
    for p in &report.orphan_files {
        out.push_str(&format!("  orphan {}\n", p.display()));
    }
    out.push_str(&format!(
        "TOMBSTONE_RESIDUE {}\n",
        report.tombstone_residue.len()
    ));
    for p in &report.tombstone_residue {
        out.push_str(&format!("  tombstone {}\n", p.display()));
    }
    out.push_str(&format!(
        "MISSING_CONTENT {}\n",
        report.missing_content.len()
    ));
    for k in &report.missing_content {
        out.push_str(&format!("  missing {}\n", k));
    }
    out.push_str(&format!(
        "DIGEST_MISMATCH {}\n",
        report.digest_mismatch.len()
    ));
    for k in &report.digest_mismatch {
        out.push_str(&format!("  mismatch {}\n", k));
    }
    if report.repaired_index {
        out.push_str("REPAIRED_INDEX 1\n");
    }
    out
}

// =================================================================
//  agent-notebook CLI (doc/opendan/Agent Notebook.md §9)
// =================================================================

const AGENT_NOTEBOOK_USAGE: &str = "agent-notebook [--root <path> | env AGENT_NOTEBOOK_ROOT] \
<list|read|append|status|promote|create-notebook|registry-context|\
system-context|hints|remarks> [...]";
const DEFAULT_AGENT_NOTEBOOK_ID: &str = "user/actions";

#[derive(Clone, Debug)]
struct AgentNotebookInvocation {
    root_override: Option<PathBuf>,
    verb: AgentNotebookVerb,
}

#[derive(Clone, Debug)]
enum AgentNotebookVerb {
    List {
        include_archived: bool,
    },
    Read {
        notebook_id: String,
        tags: Option<Vec<String>>,
        title: Option<String>,
        latest_n: Option<usize>,
        item_ids: Option<Vec<String>>,
        since_version: Option<String>,
        include_status: Option<Vec<NotebookItemStatus>>,
        include_superseded: bool,
        max_items: Option<usize>,
        max_bytes: Option<usize>,
        allow_unchanged: bool,
    },
    Append {
        notebook_id: String,
        title: String,
        /// `Some` → content from positional arg. `None` → read stdin.
        content: Option<String>,
        source_excerpt: Option<String>,
        write_reason: WriteReason,
        confidence: Option<Confidence>,
        valid_from: Option<String>,
        valid_until: Option<String>,
        tags: Vec<String>,
        detect_conflicts: bool,
    },
    Status {
        item_id: String,
        status: NotebookItemStatus,
        reason: String,
        superseded_by: Option<String>,
        expected_item_revision: Option<i64>,
    },
    Promote {
        item_id: String,
        reason: String,
        replace_item_id: Option<String>,
    },
    CreateNotebook {
        notebook_id: String,
        kind: Option<NotebookKind>,
        title: Option<String>,
        description: Option<String>,
    },
    RegistryContext {
        max_notebooks: Option<usize>,
    },
    SystemContext {
        max_items: Option<usize>,
    },
    Hints {
        topic_tags: Option<Vec<String>>,
        candidate_notebook_ids: Option<Vec<String>>,
        max_hints: Option<usize>,
    },
    RemarkList {
        item_id: String,
        remark_type: Option<String>,
    },
    RemarkAppend {
        item_id: String,
        remark_type: String,
        content: Option<String>,
    },
    RemarkRemove {
        item_id: String,
        remark_id: String,
    },
}

fn agent_notebook_invalid(message: impl Into<String>) -> AgentToolError {
    AgentToolError::InvalidArgs(format!(
        "{}\nUsage: {}",
        message.into(),
        AGENT_NOTEBOOK_USAGE
    ))
}

fn parse_agent_notebook_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    // Global flags ahead of the verb.
    let mut root_override: Option<PathBuf> = None;
    let mut idx = 0usize;

    while idx < tokens.len() {
        let token = &tokens[idx];
        match token.as_str() {
            "--root" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--root`"))?;
                root_override = Some(PathBuf::from(value));
            }
            v if v.starts_with("--root=") => {
                root_override = Some(PathBuf::from(&v["--root=".len()..]));
            }
            // First non-global token ends the global-flag region.
            _ => break,
        }
        idx += 1;
    }

    let verb_token = tokens
        .get(idx)
        .ok_or_else(|| agent_notebook_invalid("missing verb"))?
        .clone();
    let rest = &tokens[idx + 1..];

    let verb = match verb_token.as_str() {
        "list" => parse_agent_notebook_list(rest)?,
        "read" => parse_agent_notebook_read(rest)?,
        "append" => parse_agent_notebook_append(rest)?,
        "status" => parse_agent_notebook_status(rest)?,
        "promote" | "promote-to-system" => parse_agent_notebook_promote(rest)?,
        "create-notebook" | "create" => parse_agent_notebook_create(rest)?,
        "registry-context" => parse_agent_notebook_registry_context(rest)?,
        "system-context" => parse_agent_notebook_system_context(rest)?,
        "hints" => parse_agent_notebook_hints(rest)?,
        "remarks" | "remark" => parse_agent_notebook_remarks(rest)?,
        "remark-list" => parse_agent_notebook_remark_list(rest)?,
        "remark-append" => parse_agent_notebook_remark_append(rest)?,
        "remark-remove" => parse_agent_notebook_remark_remove(rest)?,
        other => return Err(agent_notebook_invalid(format!("unknown verb `{other}`"))),
    };

    Ok(ParsedCommand::AgentNotebook {
        tool_name,
        invocation: AgentNotebookInvocation {
            root_override,
            verb,
        },
    })
}

fn parse_agent_notebook_list(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut include_archived = false;
    for token in rest {
        match token.as_str() {
            "--include-archived" => include_archived = true,
            v => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported argument `{v}` for `list`"
                )))
            }
        }
    }
    Ok(AgentNotebookVerb::List { include_archived })
}

fn parse_agent_notebook_read(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut notebook_id: Option<String> = None;
    let mut tags: Option<Vec<String>> = None;
    let mut title: Option<String> = None;
    let mut latest_n: Option<usize> = None;
    let mut item_ids: Option<Vec<String>> = None;
    let mut since_version: Option<String> = None;
    let mut include_status: Option<Vec<NotebookItemStatus>> = None;
    let mut include_superseded = false;
    let mut max_items: Option<usize> = None;
    let mut max_bytes: Option<usize> = None;
    let mut allow_unchanged = true;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--bookid" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--bookid`"))?;
                notebook_id = Some(value.clone());
            }
            v if v.starts_with("--bookid=") => {
                notebook_id = Some(v["--bookid=".len()..].to_string());
            }
            "--tags" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--tags`"))?;
                tags = Some(split_csv(value));
            }
            v if v.starts_with("--tags=") => {
                tags = Some(split_csv(&v["--tags=".len()..]));
            }
            "--title" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--title`"))?;
                title = Some(value.clone());
            }
            v if v.starts_with("--title=") => {
                title = Some(v["--title=".len()..].to_string());
            }
            "--latest" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--latest`"))?;
                latest_n = Some(parse_usize(value, "latest")?);
            }
            v if v.starts_with("--latest=") => {
                latest_n = Some(parse_usize(&v["--latest=".len()..], "latest")?);
            }
            "--items" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--items`"))?;
                item_ids = Some(split_csv(value));
            }
            v if v.starts_with("--items=") => {
                item_ids = Some(split_csv(&v["--items=".len()..]));
            }
            "--since-version" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--since-version`"))?;
                since_version = Some(value.clone());
            }
            v if v.starts_with("--since-version=") => {
                since_version = Some(v["--since-version=".len()..].to_string());
            }
            "--include-status" => {
                idx += 1;
                let value = rest.get(idx).ok_or_else(|| {
                    agent_notebook_invalid("missing value for `--include-status`")
                })?;
                include_status = Some(parse_status_list(value)?);
            }
            v if v.starts_with("--include-status=") => {
                include_status = Some(parse_status_list(&v["--include-status=".len()..])?);
            }
            "--include-superseded" => include_superseded = true,
            "--max-items" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--max-items`"))?;
                max_items = Some(parse_usize(value, "max-items")?);
            }
            v if v.starts_with("--max-items=") => {
                max_items = Some(parse_usize(&v["--max-items=".len()..], "max-items")?);
            }
            "--max-bytes" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--max-bytes`"))?;
                max_bytes = Some(parse_usize(value, "max-bytes")?);
            }
            v if v.starts_with("--max-bytes=") => {
                max_bytes = Some(parse_usize(&v["--max-bytes=".len()..], "max-bytes")?);
            }
            "--no-unchanged" => allow_unchanged = false,
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `read`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if !positionals.is_empty() {
        return Err(agent_notebook_invalid(format!(
            "`read` does not accept positional notebook_id; use `--bookid <notebook_id>` (got {} positional arguments)",
            positionals.len()
        )));
    }
    Ok(AgentNotebookVerb::Read {
        notebook_id: notebook_id.unwrap_or_else(|| DEFAULT_AGENT_NOTEBOOK_ID.to_string()),
        tags,
        title,
        latest_n,
        item_ids,
        since_version,
        include_status,
        include_superseded,
        max_items,
        max_bytes,
        allow_unchanged,
    })
}

fn parse_agent_notebook_append(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut notebook_id: Option<String> = None;
    let mut use_stdin = false;
    let mut source_excerpt: Option<String> = None;
    let mut confidence: Option<Confidence> = None;
    let mut valid_from: Option<String> = None;
    let mut valid_until: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut detect_conflicts = true;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--bookid" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--bookid`"))?;
                notebook_id = Some(value.clone());
            }
            v if v.starts_with("--bookid=") => {
                notebook_id = Some(v["--bookid=".len()..].to_string());
            }
            "--stdin" => use_stdin = true,
            "--source-excerpt" => {
                idx += 1;
                let value = rest.get(idx).ok_or_else(|| {
                    agent_notebook_invalid("missing value for `--source-excerpt`")
                })?;
                source_excerpt = Some(value.clone());
            }
            v if v.starts_with("--source-excerpt=") => {
                source_excerpt = Some(v["--source-excerpt=".len()..].to_string());
            }
            "--confidence" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--confidence`"))?;
                confidence = Some(parse_confidence(value)?);
            }
            v if v.starts_with("--confidence=") => {
                confidence = Some(parse_confidence(&v["--confidence=".len()..])?);
            }
            "--valid-from" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--valid-from`"))?;
                valid_from = Some(value.clone());
            }
            v if v.starts_with("--valid-from=") => {
                valid_from = Some(v["--valid-from=".len()..].to_string());
            }
            "--valid-until" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--valid-until`"))?;
                valid_until = Some(value.clone());
            }
            v if v.starts_with("--valid-until=") => {
                valid_until = Some(v["--valid-until=".len()..].to_string());
            }
            "--tags" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--tags`"))?;
                tags = split_csv(value);
            }
            v if v.starts_with("--tags=") => {
                tags = split_csv(&v["--tags=".len()..]);
            }
            "--no-detect-conflicts" => detect_conflicts = false,
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `append`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    let (title, content) = match (use_stdin, positionals.len()) {
        (false, 2) => {
            let mut it = positionals.into_iter();
            (it.next().unwrap(), Some(it.next().unwrap()))
        }
        (true, 1) => {
            let mut it = positionals.into_iter();
            (it.next().unwrap(), None)
        }
        (false, 1) => {
            return Err(agent_notebook_invalid(
                "`append` expects positional `<content>` or `--stdin`",
            ));
        }
        (true, 2) => {
            return Err(agent_notebook_invalid(
                "`append --stdin` does not accept a positional `<content>`",
            ));
        }
        (_, n) => {
            return Err(agent_notebook_invalid(format!(
                "`append` expects 1-2 positional arguments (title[, content]); use `--bookid <notebook_id>` to select a notebook, got {n}"
            )));
        }
    };

    Ok(AgentNotebookVerb::Append {
        notebook_id: notebook_id.unwrap_or_else(|| DEFAULT_AGENT_NOTEBOOK_ID.to_string()),
        title,
        content,
        source_excerpt,
        write_reason: WriteReason::UserExplicit,
        confidence,
        valid_from,
        valid_until,
        tags,
        detect_conflicts,
    })
}

fn parse_agent_notebook_status(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut reason: Option<String> = None;
    let mut superseded_by: Option<String> = None;
    let mut expected_item_revision: Option<i64> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--reason" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--reason`"))?;
                reason = Some(value.clone());
            }
            v if v.starts_with("--reason=") => {
                reason = Some(v["--reason=".len()..].to_string());
            }
            "--superseded-by" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--superseded-by`"))?;
                superseded_by = Some(value.clone());
            }
            v if v.starts_with("--superseded-by=") => {
                superseded_by = Some(v["--superseded-by=".len()..].to_string());
            }
            "--expected-item-revision" => {
                idx += 1;
                let value = rest.get(idx).ok_or_else(|| {
                    agent_notebook_invalid("missing value for `--expected-item-revision`")
                })?;
                expected_item_revision = Some(parse_i64(value, "expected-item-revision")?);
            }
            v if v.starts_with("--expected-item-revision=") => {
                expected_item_revision = Some(parse_i64(
                    &v["--expected-item-revision=".len()..],
                    "expected-item-revision",
                )?);
            }
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `status`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 2 {
        return Err(agent_notebook_invalid(format!(
            "`status` expects 2 positional arguments (item_id, new_status), got {}",
            positionals.len()
        )));
    }
    let mut it = positionals.into_iter();
    let item_id = it.next().unwrap();
    let status = parse_item_status(&it.next().unwrap())?;
    let reason = reason.ok_or_else(|| agent_notebook_invalid("`status` requires `--reason`"))?;
    Ok(AgentNotebookVerb::Status {
        item_id,
        status,
        reason,
        superseded_by,
        expected_item_revision,
    })
}

fn parse_agent_notebook_promote(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut reason: Option<String> = None;
    let mut replace_item_id: Option<String> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--reason" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--reason`"))?;
                reason = Some(value.clone());
            }
            v if v.starts_with("--reason=") => {
                reason = Some(v["--reason=".len()..].to_string());
            }
            "--replace" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--replace`"))?;
                replace_item_id = Some(value.clone());
            }
            v if v.starts_with("--replace=") => {
                replace_item_id = Some(v["--replace=".len()..].to_string());
            }
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `promote`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_notebook_invalid(format!(
            "`promote` expects 1 positional argument (item_id), got {}",
            positionals.len()
        )));
    }
    let reason = reason.ok_or_else(|| agent_notebook_invalid("`promote` requires `--reason`"))?;
    Ok(AgentNotebookVerb::Promote {
        item_id: positionals.into_iter().next().unwrap(),
        reason,
        replace_item_id,
    })
}

fn parse_agent_notebook_create(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut notebook_id: Option<String> = None;
    let mut kind: Option<NotebookKind> = None;
    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--bookid" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--bookid`"))?;
                notebook_id = Some(value.clone());
            }
            v if v.starts_with("--bookid=") => {
                notebook_id = Some(v["--bookid=".len()..].to_string());
            }
            "--kind" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--kind`"))?;
                kind = Some(parse_notebook_kind(value)?);
            }
            v if v.starts_with("--kind=") => {
                kind = Some(parse_notebook_kind(&v["--kind=".len()..])?);
            }
            "--title" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--title`"))?;
                title = Some(value.clone());
            }
            v if v.starts_with("--title=") => {
                title = Some(v["--title=".len()..].to_string());
            }
            "--description" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--description`"))?;
                description = Some(value.clone());
            }
            v if v.starts_with("--description=") => {
                description = Some(v["--description=".len()..].to_string());
            }
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `create-notebook`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if !positionals.is_empty() {
        return Err(agent_notebook_invalid(format!(
            "`create-notebook` does not accept positional notebook_id; use `--bookid <notebook_id>` (got {} positional arguments)",
            positionals.len()
        )));
    }
    let notebook_id = notebook_id
        .ok_or_else(|| agent_notebook_invalid("`create-notebook` requires `--bookid`"))?;
    Ok(AgentNotebookVerb::CreateNotebook {
        notebook_id,
        kind,
        title,
        description,
    })
}

fn parse_agent_notebook_registry_context(
    rest: &[String],
) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut max_notebooks: Option<usize> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--max-notebooks" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--max-notebooks`"))?;
                max_notebooks = Some(parse_usize(value, "max-notebooks")?);
            }
            v if v.starts_with("--max-notebooks=") => {
                max_notebooks = Some(parse_usize(
                    &v["--max-notebooks=".len()..],
                    "max-notebooks",
                )?);
            }
            v => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported argument `{v}` for `registry-context`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentNotebookVerb::RegistryContext { max_notebooks })
}

fn parse_agent_notebook_system_context(
    rest: &[String],
) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut max_items: Option<usize> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--max-items" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--max-items`"))?;
                max_items = Some(parse_usize(value, "max-items")?);
            }
            v if v.starts_with("--max-items=") => {
                max_items = Some(parse_usize(&v["--max-items=".len()..], "max-items")?);
            }
            v => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported argument `{v}` for `system-context`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentNotebookVerb::SystemContext { max_items })
}

fn parse_agent_notebook_hints(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut topic_tags: Option<Vec<String>> = None;
    let mut candidate_notebook_ids: Option<Vec<String>> = None;
    let mut max_hints: Option<usize> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--topic-tags" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--topic-tags`"))?;
                topic_tags = Some(split_csv(value));
            }
            v if v.starts_with("--topic-tags=") => {
                topic_tags = Some(split_csv(&v["--topic-tags=".len()..]));
            }
            "--candidate-notebooks" => {
                idx += 1;
                let value = rest.get(idx).ok_or_else(|| {
                    agent_notebook_invalid("missing value for `--candidate-notebooks`")
                })?;
                candidate_notebook_ids = Some(split_csv(value));
            }
            v if v.starts_with("--candidate-notebooks=") => {
                candidate_notebook_ids = Some(split_csv(&v["--candidate-notebooks=".len()..]));
            }
            "--max-hints" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--max-hints`"))?;
                max_hints = Some(parse_usize(value, "max-hints")?);
            }
            v if v.starts_with("--max-hints=") => {
                max_hints = Some(parse_usize(&v["--max-hints=".len()..], "max-hints")?);
            }
            v => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported argument `{v}` for `hints`"
                )))
            }
        }
        idx += 1;
    }
    Ok(AgentNotebookVerb::Hints {
        topic_tags,
        candidate_notebook_ids,
        max_hints,
    })
}

fn parse_agent_notebook_remarks(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let sub = rest
        .first()
        .ok_or_else(|| agent_notebook_invalid("`remarks` requires list|append|remove"))?;
    match sub.as_str() {
        "list" => parse_agent_notebook_remark_list(&rest[1..]),
        "append" => parse_agent_notebook_remark_append(&rest[1..]),
        "remove" => parse_agent_notebook_remark_remove(&rest[1..]),
        other => Err(agent_notebook_invalid(format!(
            "unknown `remarks` subcommand `{other}`"
        ))),
    }
}

fn parse_agent_notebook_remark_list(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut remark_type: Option<String> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--type" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--type`"))?;
                remark_type = Some(value.clone());
            }
            v if v.starts_with("--type=") => {
                remark_type = Some(v["--type=".len()..].to_string());
            }
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `remarks list`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 1 {
        return Err(agent_notebook_invalid(format!(
            "`remarks list` expects 1 positional argument (item_id), got {}",
            positionals.len()
        )));
    }
    Ok(AgentNotebookVerb::RemarkList {
        item_id: positionals.into_iter().next().unwrap(),
        remark_type,
    })
}

fn parse_agent_notebook_remark_append(
    rest: &[String],
) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut use_stdin = false;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--stdin" => use_stdin = true,
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `remarks append`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    let (item_id, remark_type, content) = match (use_stdin, positionals.len()) {
        (false, 3) => {
            let mut it = positionals.into_iter();
            (
                it.next().unwrap(),
                it.next().unwrap(),
                Some(it.next().unwrap()),
            )
        }
        (true, 2) => {
            let mut it = positionals.into_iter();
            (it.next().unwrap(), it.next().unwrap(), None)
        }
        (false, n) => {
            return Err(agent_notebook_invalid(format!(
                "`remarks append` expects 3 positional arguments (item_id, type, content), got {n}"
            )))
        }
        (true, n) => {
            return Err(agent_notebook_invalid(format!(
                "`remarks append --stdin` expects 2 positional arguments (item_id, type), got {n}"
            )))
        }
    };
    Ok(AgentNotebookVerb::RemarkAppend {
        item_id,
        remark_type,
        content,
    })
}

fn parse_agent_notebook_remark_remove(
    rest: &[String],
) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            v if v.starts_with("--") => {
                return Err(agent_notebook_invalid(format!(
                    "unsupported flag `{v}` for `remarks remove`"
                )))
            }
            v => positionals.push(v.to_string()),
        }
        idx += 1;
    }
    if positionals.len() != 2 {
        return Err(agent_notebook_invalid(format!(
            "`remarks remove` expects 2 positional arguments (item_id, remark_id), got {}",
            positionals.len()
        )));
    }
    let mut it = positionals.into_iter();
    Ok(AgentNotebookVerb::RemarkRemove {
        item_id: it.next().unwrap(),
        remark_id: it.next().unwrap(),
    })
}

fn split_csv(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn parse_usize(raw: &str, name: &str) -> Result<usize, AgentToolError> {
    raw.trim()
        .parse::<usize>()
        .map_err(|_| agent_notebook_invalid(format!("invalid `--{name}` value `{raw}`")))
}

fn parse_i64(raw: &str, name: &str) -> Result<i64, AgentToolError> {
    raw.trim()
        .parse::<i64>()
        .map_err(|_| agent_notebook_invalid(format!("invalid `--{name}` value `{raw}`")))
}

fn parse_confidence(raw: &str) -> Result<Confidence, AgentToolError> {
    Ok(match raw.trim() {
        "low" => Confidence::Low,
        "medium" => Confidence::Medium,
        "high" => Confidence::High,
        other => {
            return Err(agent_notebook_invalid(format!(
                "invalid confidence `{other}` (expected low|medium|high)"
            )))
        }
    })
}

fn parse_item_status(raw: &str) -> Result<NotebookItemStatus, AgentToolError> {
    Ok(match raw.trim() {
        "active" => NotebookItemStatus::Active,
        "stale" => NotebookItemStatus::Stale,
        "superseded" => NotebookItemStatus::Superseded,
        "deleted" => NotebookItemStatus::Deleted,
        other => {
            return Err(agent_notebook_invalid(format!(
                "invalid item status `{other}` (expected active|stale|superseded|deleted)"
            )))
        }
    })
}

fn parse_status_list(raw: &str) -> Result<Vec<NotebookItemStatus>, AgentToolError> {
    let mut out = Vec::new();
    for piece in raw.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()) {
        out.push(parse_item_status(piece)?);
    }
    if out.is_empty() {
        return Err(agent_notebook_invalid(
            "`--include-status` must list at least one status",
        ));
    }
    Ok(out)
}

fn parse_notebook_kind(raw: &str) -> Result<NotebookKind, AgentToolError> {
    Ok(match raw.trim() {
        "normal" => NotebookKind::Normal,
        "project" => NotebookKind::Project,
        "system" => NotebookKind::System,
        "agent" => NotebookKind::Agent,
        other => {
            return Err(agent_notebook_invalid(format!(
                "invalid notebook kind `{other}` (expected normal|project|system|agent)"
            )))
        }
    })
}

fn agent_notebook_exit_code(err: &NotebookError) -> i32 {
    match err {
        NotebookError::NotFound(_) => 3,
        NotebookError::PermissionDenied(_) => 4,
        NotebookError::InvalidInput(_) | NotebookError::InvalidTag(_) => 2,
        NotebookError::VersionConflict(_) => 5,
        NotebookError::LimitExceeded(_) => 6,
        NotebookError::ItemSearchUnavailable(_) => 7,
        _ => 1,
    }
}

fn agent_notebook_error_output(err: NotebookError) -> CliRunOutput {
    let exit_code = agent_notebook_exit_code(&err);
    let payload = json!({
        "status": "error",
        "code": err.code(),
        "message": err.to_string(),
    });
    CliRunOutput {
        exit_code,
        stdout: format!("{payload}\n"),
        stderr: String::new(),
    }
}

fn agent_skills_error_output(err: SkillsMgrError) -> CliRunOutput {
    let exit_code = err.exit_code();
    let payload = json!({
        "status": "error",
        "code": err.code(),
        "message": err.to_string(),
    });
    CliRunOutput {
        exit_code,
        stdout: format!("{payload}\n"),
        stderr: String::new(),
    }
}

async fn dispatch_agent_skills(
    env: &CliRuntimeEnv,
    _tool_name: &str,
    invocation: AgentSkillsInvocation,
) -> CliRunOutput {
    let AgentSkillsInvocation {
        root_override,
        verb,
    } = invocation;
    let root = resolve_agent_skills_root(env, root_override);
    let default_actor = env.call_ctx.agent_name.clone();
    let default_session_id = env.runtime_context.session_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_agent_skills_blocking(&root, &default_actor, &default_session_id, verb)
    })
    .await
    .unwrap_or_else(|join| {
        Err(SkillsMgrError::Storage(format!(
            "agent-skills worker panicked: {join}"
        )))
    });
    match result {
        Ok(value) => {
            let mut stdout =
                serde_json::to_string(&value).unwrap_or_else(|_| "{\"status\":\"ok\"}".to_string());
            stdout.push('\n');
            CliRunOutput {
                exit_code: 0,
                stdout,
                stderr: String::new(),
            }
        }
        Err(err) => agent_skills_error_output(err),
    }
}

fn run_agent_skills_blocking(
    root: &Path,
    default_actor: &str,
    default_session_id: &str,
    verb: AgentSkillsVerb,
) -> Result<Json, SkillsMgrError> {
    let mgr = SkillsMgr::open(SkillsMgrConfig::new(root))?;
    match verb {
        AgentSkillsVerb::Init => Ok(json!({
            "status": "ok",
            "root": root.to_string_lossy().to_string(),
        })),
        AgentSkillsVerb::Install {
            path,
            id,
            name,
            title,
            description,
            skill_type,
            group_id,
            intent_tags,
            required_tools,
            risk_level,
            source_type,
            target_scope,
            trust_hint,
            installed_by,
            promote,
        } => {
            let installed_by = installed_by.unwrap_or_else(|| default_actor.to_string());
            let source_uri = path.to_string_lossy().to_string();
            let result = mgr.create_candidate(SkillCreateCandidateInput {
                source_uri,
                source_type,
                installed_by: installed_by.clone(),
                target_scope,
                trust_hint,
                package_path: Some(path),
                body: None,
                id,
                name,
                title,
                description,
                skill_type,
                group_id,
                intent_tags,
                required_tools,
                risk_level,
            })?;
            if promote {
                let promoted = mgr.promote(&result.candidate.skill_id, &installed_by)?;
                Ok(json!({
                    "status": "ok",
                    "install": result,
                    "promoted": promoted,
                }))
            } else {
                Ok(json!({
                    "status": "ok",
                    "install": result,
                }))
            }
        }
        AgentSkillsVerb::List { filter } => {
            let skills = mgr.list_skills(filter)?;
            Ok(json!({
                "status": "ok",
                "skills": skills,
            }))
        }
        AgentSkillsVerb::ExplorerHints { filter } => {
            let skills = mgr.skill_hints_for_explorer(filter)?;
            Ok(json!({
                "status": "ok",
                "skills": skills,
            }))
        }
        AgentSkillsVerb::View {
            skill_ref,
            package_path,
        } => {
            let value = mgr.skill_view(&skill_ref, package_path.as_deref())?;
            Ok(json!({
                "status": "ok",
                "skill": value,
            }))
        }
        AgentSkillsVerb::Promote { skill_ref, actor } => {
            let actor = actor.unwrap_or_else(|| default_actor.to_string());
            let skill = mgr.promote(&skill_ref, &actor)?;
            Ok(json!({
                "status": "ok",
                "skill": skill,
            }))
        }
        AgentSkillsVerb::Archive {
            skill_ref,
            reason,
            actor,
        } => {
            let actor = actor.unwrap_or_else(|| default_actor.to_string());
            let skill = mgr.archive(&skill_ref, &actor, &reason)?;
            Ok(json!({
                "status": "ok",
                "skill": skill,
            }))
        }
        AgentSkillsVerb::Restore {
            skill_ref,
            reason,
            actor,
        } => {
            let actor = actor.unwrap_or_else(|| default_actor.to_string());
            let skill = mgr.restore(&skill_ref, &actor, &reason)?;
            Ok(json!({
                "status": "ok",
                "skill": skill,
            }))
        }
        AgentSkillsVerb::Validate {
            requested,
            max_count,
            allowed_states,
            risk_budget,
        } => {
            let result = mgr.validate_selection(SkillValidateSelectionInput {
                requested,
                max_count,
                allowed_states,
                risk_budget,
            })?;
            Ok(json!({
                "status": "ok",
                "validation": result,
            }))
        }
        AgentSkillsVerb::SelectSession {
            session_id,
            requested,
            risk_budget,
            max_count,
        } => {
            let session_id = session_id.unwrap_or_else(|| default_session_id.to_string());
            let selected = mgr.select_for_session(
                &session_id,
                SkillValidateSelectionInput {
                    requested,
                    max_count,
                    allowed_states: vec![
                        SkillLifecycleState::Active,
                        SkillLifecycleState::Preferred,
                    ],
                    risk_budget,
                },
            )?;
            Ok(json!({
                "status": "ok",
                "selected": selected,
            }))
        }
        AgentSkillsVerb::SelectTodo {
            session_id,
            todo_id,
            requested,
            risk_budget,
            max_count,
        } => {
            let session_id = session_id.unwrap_or_else(|| default_session_id.to_string());
            let selected = mgr.select_for_todo(
                &session_id,
                &todo_id,
                SkillValidateSelectionInput {
                    requested,
                    max_count,
                    allowed_states: vec![
                        SkillLifecycleState::Active,
                        SkillLifecycleState::Preferred,
                    ],
                    risk_budget,
                },
            )?;
            Ok(json!({
                "status": "ok",
                "selected": selected,
            }))
        }
        AgentSkillsVerb::UnselectSession {
            session_id,
            skill_refs,
        } => {
            let session_id = session_id.unwrap_or_else(|| default_session_id.to_string());
            let selected = mgr.unselect_for_session(&session_id, &skill_refs)?;
            Ok(json!({
                "status": "ok",
                "selected": selected,
            }))
        }
        AgentSkillsVerb::UnselectTodo {
            session_id,
            todo_id,
            skill_refs,
        } => {
            let session_id = session_id.unwrap_or_else(|| default_session_id.to_string());
            let selected = mgr.unselect_for_todo(&session_id, &todo_id, &skill_refs)?;
            Ok(json!({
                "status": "ok",
                "selected": selected,
            }))
        }
        AgentSkillsVerb::SelectedList {
            session_id,
            todo_id,
        } => {
            let session_id = session_id.unwrap_or_else(|| default_session_id.to_string());
            let selected = mgr.selected_list(&session_id, todo_id.as_deref())?;
            Ok(json!({
                "status": "ok",
                "selected": selected,
            }))
        }
        AgentSkillsVerb::RenderSelected {
            session_id,
            todo_id,
            max_skills,
            token_budget,
            include_metadata,
            include_usage_instructions,
        } => {
            let session_id = session_id.unwrap_or_else(|| default_session_id.to_string());
            let fragment = mgr.render_selected(SkillRenderSelectedInput {
                session_id,
                todo_id,
                max_skills,
                token_budget,
                include_metadata,
                include_usage_instructions,
            })?;
            Ok(json!({
                "status": "ok",
                "fragment": fragment,
            }))
        }
        AgentSkillsVerb::RecordUsed {
            usage_id,
            mode,
            evidence,
        } => {
            let usage = mgr.record_used(&usage_id, mode, evidence)?;
            Ok(json!({
                "status": "ok",
                "usage": usage,
            }))
        }
        AgentSkillsVerb::RecordResult {
            usage_id,
            result,
            feedback,
            failure_reason,
            output_refs,
        } => {
            let usage =
                mgr.record_result(&usage_id, result, feedback, failure_reason, output_refs)?;
            Ok(json!({
                "status": "ok",
                "usage": usage,
            }))
        }
    }
}

async fn dispatch_agent_notebook(
    env: &CliRuntimeEnv,
    _tool_name: &str,
    invocation: AgentNotebookInvocation,
    stdin_override: Option<String>,
) -> CliRunOutput {
    let AgentNotebookInvocation {
        root_override,
        verb,
    } = invocation;

    let session_id = resolve_agent_notebook_session_id(env);

    let root = resolve_agent_notebook_root(env, root_override);

    // Stdin pickup for `append --stdin`.
    let resolved_verb = match verb {
        AgentNotebookVerb::Append {
            notebook_id,
            title,
            content,
            source_excerpt,
            write_reason,
            confidence,
            valid_from,
            valid_until,
            tags,
            detect_conflicts,
        } if content.is_none() => match read_stdin_content(stdin_override).await {
            Ok(body) => {
                if body.is_empty() {
                    return CliRunOutput {
                        exit_code: 2,
                        stdout: format!(
                            "{}\n",
                            json!({
                                "status": "error",
                                "code": "invalid_input",
                                "message": "stdin produced 0 bytes; refusing empty content",
                            })
                        ),
                        stderr: String::new(),
                    };
                }
                AgentNotebookVerb::Append {
                    notebook_id,
                    title,
                    content: Some(body),
                    source_excerpt,
                    write_reason,
                    confidence,
                    valid_from,
                    valid_until,
                    tags,
                    detect_conflicts,
                }
            }
            Err(err) => {
                return CliRunOutput {
                    exit_code: 1,
                    stdout: format!(
                        "{}\n",
                        json!({
                            "status": "error",
                            "code": "storage_error",
                            "message": err.to_string(),
                        })
                    ),
                    stderr: String::new(),
                };
            }
        },
        AgentNotebookVerb::RemarkAppend {
            item_id,
            remark_type,
            content,
        } if content.is_none() => match read_stdin_content(stdin_override).await {
            Ok(body) => {
                if body.is_empty() {
                    return CliRunOutput {
                        exit_code: 2,
                        stdout: format!(
                            "{}\n",
                            json!({
                                "status": "error",
                                "code": "invalid_input",
                                "message": "stdin produced 0 bytes; refusing empty remark content",
                            })
                        ),
                        stderr: String::new(),
                    };
                }
                AgentNotebookVerb::RemarkAppend {
                    item_id,
                    remark_type,
                    content: Some(body),
                }
            }
            Err(err) => {
                return CliRunOutput {
                    exit_code: 1,
                    stdout: format!(
                        "{}\n",
                        json!({
                            "status": "error",
                            "code": "storage_error",
                            "message": err.to_string(),
                        })
                    ),
                    stderr: String::new(),
                };
            }
        },
        v => v,
    };

    let session_arg = session_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_agent_notebook_blocking(&root, session_arg.as_deref(), resolved_verb)
    })
    .await
    .unwrap_or_else(|join| {
        Err(NotebookError::Storage(format!(
            "agent-notebook worker panicked: {join}"
        )))
    });

    match result {
        Ok(value) => {
            // Append a trailing newline so the JSON line plays nice with shell consumers.
            let mut stdout =
                serde_json::to_string(&value).unwrap_or_else(|_| "{\"status\":\"ok\"}".to_string());
            stdout.push('\n');
            CliRunOutput {
                exit_code: 0,
                stdout,
                stderr: String::new(),
            }
        }
        Err(err) => agent_notebook_error_output(err),
    }
}

fn run_agent_notebook_blocking(
    root: &Path,
    session_id: Option<&str>,
    verb: AgentNotebookVerb,
) -> nb::Result<Json> {
    let cfg = AgentNotebookConfig::new(root);
    let notebook = AgentNotebook::open(cfg)?;
    match verb {
        AgentNotebookVerb::List { include_archived } => {
            let entries = notebook.list_notebooks(ListNotebooksInput { include_archived })?;
            Ok(json!({
                "status": "ok",
                "notebooks": entries,
            }))
        }
        AgentNotebookVerb::Read {
            notebook_id,
            tags,
            title,
            latest_n,
            item_ids,
            since_version,
            include_status,
            include_superseded,
            max_items,
            max_bytes,
            allow_unchanged,
        } => {
            let result = notebook.read_notebook(ReadNotebookInput {
                session_id: session_id.map(|s| s.to_string()),
                notebook_id,
                tags,
                title,
                latest_n,
                item_ids,
                since_version,
                include_status,
                include_superseded,
                max_items,
                max_bytes,
                allow_unchanged,
            })?;
            Ok(serde_json::to_value(NotebookReadResultWire(result))?)
        }
        AgentNotebookVerb::Append {
            notebook_id,
            title,
            content,
            source_excerpt,
            write_reason,
            confidence,
            valid_from,
            valid_until,
            tags,
            detect_conflicts,
        } => {
            let result = notebook.append_note(AppendNoteInput {
                session_id: session_id.map(|s| s.to_string()),
                notebook_id,
                title,
                content: content.expect("stdin form resolved earlier"),
                source_excerpt,
                source_ref: None,
                source_session_id: session_id.map(|s| s.to_string()),
                write_reason,
                valid_from,
                valid_until,
                confidence,
                tags,
                detect_conflicts,
            })?;
            Ok(serde_json::to_value(result)?)
        }
        AgentNotebookVerb::Status {
            item_id,
            status,
            reason,
            superseded_by,
            expected_item_revision,
        } => {
            let result = notebook.mark_note_status(MarkNoteStatusInput {
                session_id: session_id.map(|s| s.to_string()),
                item_id,
                status,
                reason,
                superseded_by,
                expected_item_revision,
            })?;
            Ok(serde_json::to_value(result)?)
        }
        AgentNotebookVerb::Promote {
            item_id,
            reason,
            replace_item_id,
        } => {
            let result = notebook.promote_to_system_notebook(PromoteToSystemInput {
                item_id,
                reason,
                replace_item_id,
            })?;
            Ok(serde_json::to_value(PromoteResultWire(result))?)
        }
        AgentNotebookVerb::CreateNotebook {
            notebook_id,
            kind,
            title,
            description,
        } => {
            let result = notebook.create_or_update_notebook(CreateOrUpdateNotebookInput {
                notebook_id,
                kind,
                title,
                description,
            })?;
            Ok(json!({
                "status": "ok",
                "created": result.created,
                "notebook": result.notebook,
            }))
        }
        AgentNotebookVerb::RegistryContext { max_notebooks } => {
            let result = notebook
                .build_notebook_registry_context(BuildRegistryContextInput { max_notebooks })?;
            Ok(json!({
                "status": "ok",
                "registry": result,
            }))
        }
        AgentNotebookVerb::SystemContext { max_items } => {
            let result =
                notebook.build_system_notebook_context(BuildSystemContextInput { max_items })?;
            Ok(json!({
                "status": "ok",
                "system": result,
            }))
        }
        AgentNotebookVerb::Hints {
            topic_tags,
            candidate_notebook_ids,
            max_hints,
        } => {
            let session_id = session_id
                .map(|s| s.to_string())
                .ok_or_else(|| NotebookError::InvalidInput("current session id is empty".into()))?;
            let result = notebook.build_notebook_hints(BuildHintsInput {
                session_id,
                topic_tags,
                candidate_notebook_ids,
                max_hints,
            })?;
            Ok(json!({
                "status": "ok",
                "hints_block": result,
            }))
        }
        AgentNotebookVerb::RemarkList {
            item_id,
            remark_type,
        } => {
            let remarks = notebook.list_item_remarks(ListItemRemarksInput {
                item_id: item_id.clone(),
                remark_type,
            })?;
            Ok(json!({
                "status": "ok",
                "item_id": item_id,
                "remarks": remarks,
            }))
        }
        AgentNotebookVerb::RemarkAppend {
            item_id,
            remark_type,
            content,
        } => {
            let result = notebook.append_item_remark(AppendItemRemarkInput {
                session_id: session_id.map(|s| s.to_string()),
                item_id,
                remark_type,
                content: content.expect("stdin form resolved earlier"),
            })?;
            Ok(serde_json::to_value(result)?)
        }
        AgentNotebookVerb::RemarkRemove { item_id, remark_id } => {
            let result = notebook.remove_item_remark(RemoveItemRemarkInput {
                session_id: session_id.map(|s| s.to_string()),
                item_id,
                remark_id,
            })?;
            Ok(serde_json::to_value(result)?)
        }
    }
}

/// Thin wrapper so we can emit the tagged-enum directly without an extra
/// envelope (the enum already serializes a `status` discriminant per §5.3).
struct NotebookReadResultWire(NotebookReadResult);

impl Serialize for NotebookReadResultWire {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

struct PromoteResultWire(PromoteToSystemResult);

impl Serialize for PromoteResultWire {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

fn cli_state_root(env: &CliRuntimeEnv) -> PathBuf {
    if env.has_agent_env {
        env.agent_env_root.clone()
    } else {
        env.current_dir.join(".opendan-cli")
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CliLocalWorkspaceSessionBinding {
    session_id: String,
    bound_at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CliWorkspaceRecord {
    workspace_id: String,
    name: String,
    relative_path: Option<String>,
    created_by_session: Option<String>,
    created_at_ms: u64,
    updated_at_ms: u64,
    bound_sessions: Vec<CliLocalWorkspaceSessionBinding>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CliWorkspaceIndex {
    agent_did: String,
    workspaces: Vec<CliWorkspaceRecord>,
    updated_at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CliSessionWorkspaceBinding {
    session_id: String,
    local_workspace_id: String,
    workspace_path: String,
    workspace_rel_path: String,
    agent_env_root: String,
    bound_at_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct CliSessionBindingsFile {
    bindings: Vec<CliSessionWorkspaceBinding>,
}

#[derive(Clone)]
struct CliSessionBackend {
    state_root: PathBuf,
}

#[async_trait]
impl SessionViewBackend for CliSessionBackend {
    async fn session_view(&self, session_id: &str) -> Result<Json, AgentToolError> {
        let session = load_session_json(&self.state_root, session_id).await?;
        Ok(build_session_summary_view(&session))
    }
}

#[derive(Clone)]
struct CliWorkspaceBackend {
    state_root: PathBuf,
    agent_id: String,
}

struct CliReadSessionHistoryTool {
    agent_root: PathBuf,
}

struct CliCommitSessionHistoryImprovedTool {
    agent_root: PathBuf,
}

struct CliBeginAttentionSignalExtractionTool {
    agent_root: PathBuf,
    current_session_id: String,
    agent_id: String,
}

struct CliCompleteAttentionSignalExtractionTool {
    agent_root: PathBuf,
    current_session_id: String,
}

struct CliListPendingAttentionSignalsTool {
    store: Arc<AgentAttentionSignalStore>,
    agent_scope_id: String,
}

struct CliMarkAttentionSignalConsumedTool {
    store: Arc<AgentAttentionSignalStore>,
}

struct CliDiscoverEventTool {
    store: Arc<AgentAttentionSignalStore>,
    agent_root: PathBuf,
    current_session_id: String,
}

struct CliDiscoverObjectObservationTool {
    store: Arc<AgentAttentionSignalStore>,
    agent_root: PathBuf,
    current_session_id: String,
}

struct CliDiscoverRelationshipTool {
    store: Arc<AgentAttentionSignalStore>,
    agent_root: PathBuf,
    current_session_id: String,
}

struct CliDiscoverSkillCoverageGapTool {
    store: Arc<AgentAttentionSignalStore>,
    agent_root: PathBuf,
    current_session_id: String,
}

#[async_trait]
impl TypedTool for CliReadSessionHistoryTool {
    type Args = ReadSessionHistoryArgs;
    type Output = ReadSessionHistoryOutput;

    fn name(&self) -> &str {
        TOOL_READ_SESSION_HISTORY
    }

    fn description(&self) -> &str {
        "Read target Agent Session history from <agent_root>/sessions/<session_id>/round_history."
    }

    fn usage(&self) -> Option<String> {
        Some(
            "read_session_history '{\"session_id\":\"...\",\"from_already_improved\":true,\"token_limit\":40960}' | read_session_history session_id=<id> already_improved token_limit=40960".to_string(),
        )
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        parse_read_session_history_cli_args(tokens)
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!(
            "read {} message(s) from session {} ({})",
            output.returned, output.session_id, output.query
        )
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let session_id = args.session_id.trim();
        validate_session_id_arg(session_id)?;
        let session_dir = session_dir(&self.agent_root, session_id);
        if !session_dir.is_dir() {
            return Err(AgentToolError::ExecFailed(format!(
                "session `{session_id}` not found"
            )));
        }

        let token_limit = args.token_limit.unwrap_or(DEFAULT_HISTORY_TOKEN_LIMIT);
        let reader = SessionHistoryReader::open(&session_dir)
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        let already_improved = load_already_improved_state(&session_dir).await?;
        let (result, query_label) = if args.from_already_improved {
            let start_round_index = already_improved.committed_round_index.saturating_add(1);
            let result = reader
                .read_session_messages_from_round_index(
                    start_round_index,
                    SessionHistoryReadOptions { token_limit },
                )
                .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
            (
                result,
                format!("already_improved from_round={start_round_index}"),
            )
        } else {
            let (query, query_label) = build_history_query(&args)?;
            let result = reader
                .read_session_messages(query, SessionHistoryReadOptions { token_limit })
                .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
            (result, query_label)
        };
        let commit_round_index = args
            .from_already_improved
            .then_some(result.last_round_index)
            .flatten();
        let latest_round_index = result.latest_round_index;
        let messages = result
            .messages
            .into_iter()
            .map(|msg| {
                let ts_ms = msg.ts.timestamp_millis().max(0) as u64;
                SessionHistoryMessageOutput {
                    round_index: msg.round_index,
                    seq: msg.seq,
                    ts_ms,
                    ts: msg.ts.to_rfc3339(),
                    role: msg.role.as_str().to_string(),
                    text: msg.text,
                }
            })
            .collect::<Vec<_>>();
        Ok(ReadSessionHistoryOutput {
            session_id: session_id.to_string(),
            query: query_label,
            already_improved: already_improved_output(&already_improved),
            commit_round_index,
            latest_round_index,
            total_candidates: result.total_candidates,
            returned: messages.len(),
            truncated: result.truncated,
            messages,
        })
    }
}

#[async_trait]
impl TypedTool for CliCommitSessionHistoryImprovedTool {
    type Args = CommitSessionHistoryImprovedArgs;
    type Output = CommitSessionHistoryImprovedOutput;

    fn name(&self) -> &str {
        TOOL_COMMIT_SESSION_HISTORY_IMPROVED
    }

    fn description(&self) -> &str {
        "Commit self-improve processing progress into target session .meta/session.json."
    }

    fn usage(&self) -> Option<String> {
        Some(
            "commit_session_history_improved '{\"session_id\":\"...\",\"round_index\":3}' | commit_session_history_improved session_id=<id> round_index=3".to_string(),
        )
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!(
            "committed improved history for session {} through round {}",
            output.session_id, output.committed_round_index
        )
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let session_id = args.session_id.trim();
        validate_session_id_arg(session_id)?;
        let session_dir = session_dir(&self.agent_root, session_id);
        if !session_dir.is_dir() {
            return Err(AgentToolError::ExecFailed(format!(
                "session `{session_id}` not found"
            )));
        }
        let latest_round_index = SessionHistoryReader::open(&session_dir)
            .and_then(|reader| reader.latest_round_index())
            .map_err(|err| AgentToolError::ExecFailed(format!("{err:#}")))?;
        let target_round_index = latest_round_index
            .map(|latest| args.round_index.min(latest))
            .unwrap_or(0);
        let (previous, committed) =
            commit_already_improved_state(&session_dir, target_round_index).await?;
        Ok(CommitSessionHistoryImprovedOutput {
            session_id: session_id.to_string(),
            committed_round_index: committed.committed_round_index,
            previous_committed_round_index: previous.committed_round_index,
            latest_round_index,
        })
    }
}

#[async_trait]
impl TypedTool for CliBeginAttentionSignalExtractionTool {
    type Args = BeginAttentionSignalExtractionArgs;
    type Output = BeginAttentionSignalExtractionOutput;

    fn name(&self) -> &str {
        TOOL_BEGIN_ATTENTION_SIGNAL_EXTRACTION
    }

    fn description(&self) -> &str {
        "Open a persisted Stage1 extraction scope for subsequent Discover* CLI commands."
    }

    fn usage(&self) -> Option<String> {
        Some(
            "BeginAttentionSignalExtraction '{\"session_id\":\"...\",\"window_start\":\"...\",\"window_end\":\"...\"}'".to_string(),
        )
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!(
            "opened attention extraction window {} for session {}",
            output.extraction_window_id, output.session_id
        )
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let session_id = args.session_id.trim();
        validate_session_id_arg(session_id)?;
        if session_id == self.current_session_id {
            return Err(AgentToolError::InvalidArgs(
                "current self-improve session history cannot be used as self-improve input"
                    .to_string(),
            ));
        }
        let window_start = args.window_start.trim();
        let window_end = args.window_end.trim();
        if window_start.is_empty() || window_end.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "`window_start` and `window_end` must not be empty".to_string(),
            ));
        }

        let target_session_dir = session_dir(&self.agent_root, session_id);
        let meta = load_session_meta(&target_session_dir)
            .await?
            .ok_or_else(|| {
                AgentToolError::ExecFailed(format!("session `{session_id}` meta not found"))
            })?;
        if matches!(meta.kind, SessionKind::SelfImprove) {
            return Err(AgentToolError::InvalidArgs(
                "self-improve session history cannot be used as self-improve input".to_string(),
            ));
        }

        let owner_id = if meta.owner.trim().is_empty() {
            "system".to_string()
        } else {
            meta.owner.clone()
        };
        let user_id = owner_id.clone();
        let agent_scope_id = self.agent_id.clone();
        let store = open_attention_store(&self.agent_root)?;
        let extraction_window = store.create_extraction_window(CreateExtractionWindowInput {
            owner_id: owner_id.clone(),
            agent_id: self.agent_id.clone(),
            agent_scope_id: agent_scope_id.clone(),
            user_id: user_id.clone(),
            window_start: window_start.to_string(),
            window_end: window_end.to_string(),
        })?;
        let runtime = AttentionSignalToolRuntime {
            owner_id: owner_id.clone(),
            agent_id: self.agent_id.clone(),
            agent_scope_id: agent_scope_id.clone(),
            user_id: user_id.clone(),
            session_id: session_id.to_string(),
            window_start: window_start.to_string(),
            window_end: window_end.to_string(),
            extraction_window_id: extraction_window.id.clone(),
            extractor_version: None,
            prompt_version: None,
            model_name: None,
        };
        save_attention_runtime(&self.agent_root, &self.current_session_id, &runtime).await?;
        Ok(BeginAttentionSignalExtractionOutput {
            owner_id,
            agent_id: self.agent_id.clone(),
            agent_scope_id,
            user_id,
            session_id: session_id.to_string(),
            window_start: window_start.to_string(),
            window_end: window_end.to_string(),
            extraction_window_id: extraction_window.id,
        })
    }
}

#[async_trait]
impl TypedTool for CliCompleteAttentionSignalExtractionTool {
    type Args = CompleteAttentionSignalExtractionArgs;
    type Output = CompleteAttentionSignalExtractionOutput;

    fn name(&self) -> &str {
        TOOL_COMPLETE_ATTENTION_SIGNAL_EXTRACTION
    }

    fn description(&self) -> &str {
        "Complete the persisted Stage1 extraction scope for this session."
    }

    fn usage(&self) -> Option<String> {
        Some("CompleteAttentionSignalExtraction".to_string())
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!(
            "completed attention extraction window {}",
            output.extraction_window.id
        )
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        _args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let runtime = load_attention_runtime(&self.agent_root, &self.current_session_id).await?;
        let store = open_attention_store(&self.agent_root)?;
        let extraction_window = store.complete_extraction_window(&runtime.extraction_window_id)?;
        remove_attention_runtime(&self.agent_root, &self.current_session_id).await?;
        Ok(CompleteAttentionSignalExtractionOutput { extraction_window })
    }
}

#[async_trait]
impl TypedTool for CliListPendingAttentionSignalsTool {
    type Args = ListPendingAttentionSignalsArgs;
    type Output = ListPendingAttentionSignalsOutput;

    fn name(&self) -> &str {
        TOOL_LIST_PENDING_ATTENTION_SIGNALS
    }

    fn description(&self) -> &str {
        "List pending Stage2 attention signals from <agent_root>/attention_signals."
    }

    fn usage(&self) -> Option<String> {
        Some("ListPendingAttentionSignals [limit=100]".to_string())
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!("listed {} pending attention signal(s)", output.returned)
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let signals = self
            .store
            .list_pending_stage2(&self.agent_scope_id, args.limit)?;
        Ok(ListPendingAttentionSignalsOutput {
            agent_scope_id: self.agent_scope_id.clone(),
            returned: signals.len(),
            signals,
        })
    }
}

#[async_trait]
impl TypedTool for CliMarkAttentionSignalConsumedTool {
    type Args = MarkAttentionSignalConsumedArgs;
    type Output = MarkAttentionSignalConsumedOutput;

    fn name(&self) -> &str {
        TOOL_MARK_ATTENTION_SIGNAL_CONSUMED
    }

    fn description(&self) -> &str {
        "Mark one pending attention signal consumed in <agent_root>/attention_signals."
    }

    fn usage(&self) -> Option<String> {
        Some(
            "MarkAttentionSignalConsumed '{\"signal_id\":\"sig_...\"}' | MarkAttentionSignalConsumed signal_id=sig_...".to_string(),
        )
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!("marked attention signal {} consumed", output.signal.id)
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let signal = self
            .store
            .update_lifecycle_status(args.signal_id.trim(), SignalLifecycleStatus::Consumed)?;
        Ok(MarkAttentionSignalConsumedOutput { signal })
    }
}

#[async_trait]
impl TypedTool for CliDiscoverEventTool {
    type Args = DiscoverEventArgs;
    type Output = agent_tool::AttentionSignalWriteResult;

    fn name(&self) -> &str {
        agent_tool::TOOL_DISCOVER_EVENT
    }

    fn description(&self) -> &str {
        "Store a Stage1 event attention signal for the current extraction scope."
    }

    fn usage(&self) -> Option<String> {
        Some("DiscoverEvent '{\"title\":\"...\",\"phase\":\"active\",\"evidence\":[...],\"confidence\":0.9}'".to_string())
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let runtime = load_attention_runtime(&self.agent_root, &self.current_session_id).await?;
        DiscoverEventTool::new(self.store.clone(), runtime)
            .execute(ctx, args)
            .await
    }
}

#[async_trait]
impl TypedTool for CliDiscoverObjectObservationTool {
    type Args = DiscoverObjectObservationArgs;
    type Output = agent_tool::AttentionSignalWriteResult;

    fn name(&self) -> &str {
        agent_tool::TOOL_DISCOVER_OBJECT_OBSERVATION
    }

    fn description(&self) -> &str {
        "Store a Stage1 object-observation attention signal for the current extraction scope."
    }

    fn usage(&self) -> Option<String> {
        Some("DiscoverObjectObservation '{\"object\":{\"mention_text\":\"...\",\"entity_type\":\"project\"},\"observation\":\"...\",\"evidence\":[...],\"confidence\":0.9}'".to_string())
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let runtime = load_attention_runtime(&self.agent_root, &self.current_session_id).await?;
        DiscoverObjectObservationTool::new(self.store.clone(), runtime)
            .execute(ctx, args)
            .await
    }
}

#[async_trait]
impl TypedTool for CliDiscoverRelationshipTool {
    type Args = DiscoverRelationshipArgs;
    type Output = agent_tool::AttentionSignalWriteResult;

    fn name(&self) -> &str {
        agent_tool::TOOL_DISCOVER_RELATIONSHIP
    }

    fn description(&self) -> &str {
        "Store a Stage1 relationship attention signal for the current extraction scope."
    }

    fn usage(&self) -> Option<String> {
        Some("DiscoverRelationship '{\"subject\":{\"name\":\"...\"},\"predicate\":\"uses\",\"object\":{\"name\":\"...\"},\"evidence\":[...],\"confidence\":0.9}'".to_string())
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let runtime = load_attention_runtime(&self.agent_root, &self.current_session_id).await?;
        DiscoverRelationshipTool::new(self.store.clone(), runtime)
            .execute(ctx, args)
            .await
    }
}

#[async_trait]
impl TypedTool for CliDiscoverSkillCoverageGapTool {
    type Args = DiscoverSkillCoverageGapArgs;
    type Output = agent_tool::AttentionSignalWriteResult;

    fn name(&self) -> &str {
        agent_tool::TOOL_DISCOVER_SKILL_COVERAGE_GAP
    }

    fn description(&self) -> &str {
        "Store a Stage1 skill coverage gap attention signal for the current extraction scope."
    }

    fn usage(&self) -> Option<String> {
        Some("DiscoverSkillCoverageGap '{\"title\":\"...\",\"task_description\":\"...\",\"result_status\":\"success\",\"evidence\":[...],\"confidence\":0.8}'".to_string())
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let runtime = load_attention_runtime(&self.agent_root, &self.current_session_id).await?;
        DiscoverSkillCoverageGapTool::new(self.store.clone(), runtime)
            .execute(ctx, args)
            .await
    }
}

fn validate_session_id_arg(session_id: &str) -> Result<(), AgentToolError> {
    if session_id.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "`session_id` must not be empty".to_string(),
        ));
    }
    if session_id == "."
        || session_id == ".."
        || session_id.contains('/')
        || session_id.contains('\\')
    {
        return Err(AgentToolError::InvalidArgs(
            "`session_id` must be a plain session id, not a path".to_string(),
        ));
    }
    Ok(())
}

fn session_dir(agent_root: &Path, session_id: &str) -> PathBuf {
    agent_root.join("sessions").join(session_id)
}

fn session_meta_path(session_dir: &Path) -> PathBuf {
    session_dir.join(".meta").join("session.json")
}

async fn load_session_meta(session_dir: &Path) -> Result<Option<SessionMeta>, AgentToolError> {
    let path = session_meta_path(session_dir);
    match fs::read(&path).await {
        Ok(bytes) => serde_json::from_slice::<SessionMeta>(&bytes)
            .map(Some)
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("parse {} failed: {err}", path.display()))
            }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(AgentToolError::ExecFailed(format!(
            "read {} failed: {err}",
            path.display()
        ))),
    }
}

async fn write_session_meta(session_dir: &Path, meta: &SessionMeta) -> Result<(), AgentToolError> {
    let path = session_meta_path(session_dir);
    let dir = path.parent().ok_or_else(|| {
        AgentToolError::ExecFailed(format!("invalid session meta path {}", path.display()))
    })?;
    fs::create_dir_all(dir).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("mkdir {} failed: {err}", dir.display()))
    })?;
    let bytes = serde_json::to_vec_pretty(meta).map_err(|err| {
        AgentToolError::ExecFailed(format!("serialize session meta failed: {err}"))
    })?;
    let tmp = dir.join(format!(
        "session.json.{}.{}.tmp",
        std::process::id(),
        now_ms()
    ));
    fs::write(&tmp, &bytes).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("write {} failed: {err}", tmp.display()))
    })?;
    fs::rename(&tmp, &path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("rename to {} failed: {err}", path.display()))
    })?;
    Ok(())
}

async fn load_already_improved_state(
    session_dir: &Path,
) -> Result<AlreadyImprovedState, AgentToolError> {
    Ok(load_session_meta(session_dir)
        .await?
        .map(|meta| meta.already_improved)
        .unwrap_or_default())
}

async fn commit_already_improved_state(
    session_dir: &Path,
    round_index: u64,
) -> Result<(AlreadyImprovedState, AlreadyImprovedState), AgentToolError> {
    let mut meta = load_session_meta(session_dir).await?.ok_or_else(|| {
        AgentToolError::ExecFailed(format!(
            "session meta not found under {}",
            session_dir.display()
        ))
    })?;
    let previous = meta.already_improved.clone();
    if round_index > meta.already_improved.committed_round_index {
        meta.already_improved.committed_round_index = round_index;
        meta.already_improved.committed_at_ms = now_ms();
    }
    let committed = meta.already_improved.clone();
    write_session_meta(session_dir, &meta).await?;
    Ok((previous, committed))
}

fn already_improved_output(state: &AlreadyImprovedState) -> AlreadyImprovedOutput {
    AlreadyImprovedOutput {
        committed_round_index: state.committed_round_index,
        committed_at_ms: state.committed_at_ms,
    }
}

fn parse_read_session_history_cli_args(tokens: &[String]) -> Result<Json, AgentToolError> {
    if tokens.len() == 1 && tokens[0].trim().starts_with('{') {
        return agent_tool::parse_default_bash_exec_args(tokens);
    }
    let mut normalized = Vec::with_capacity(tokens.len());
    for token in tokens {
        match token.as_str() {
            "already_improved" | "from_already_improved" => {
                normalized.push("from_already_improved=true".to_string())
            }
            _ => normalized.push(token.clone()),
        }
    }
    agent_tool::parse_default_bash_exec_args(&normalized)
}

fn build_history_query(
    args: &ReadSessionHistoryArgs,
) -> Result<(SessionHistoryQuery, String), AgentToolError> {
    let exact_start = parse_optional_time(args.start_ms, args.start.as_deref(), "start")?;
    let exact_end = parse_optional_time(args.end_ms, args.end.as_deref(), "end")?;
    if exact_start.is_some() || exact_end.is_some() {
        let start = exact_start.ok_or_else(|| {
            AgentToolError::InvalidArgs(
                "`start`/`start_ms` is required with exact time range".to_string(),
            )
        })?;
        let end = exact_end.ok_or_else(|| {
            AgentToolError::InvalidArgs(
                "`end`/`end_ms` is required with exact time range".to_string(),
            )
        })?;
        if start > end {
            return Err(AgentToolError::InvalidArgs(
                "`start` must not be greater than `end`".to_string(),
            ));
        }
        return Ok((
            SessionHistoryQuery::TimeRange { start, end },
            format!("time_range {}..{}", start.to_rfc3339(), end.to_rfc3339()),
        ));
    }

    let at = parse_optional_time(args.at_ms, args.at.as_deref(), "at")?;
    if let Some(at) = at {
        let window_ms = args.window_ms.unwrap_or(DEFAULT_HISTORY_WINDOW_MS as u64) as i64;
        if window_ms <= 0 {
            return Err(AgentToolError::InvalidArgs(
                "`window_ms` must be greater than zero".to_string(),
            ));
        }
        let half = Duration::milliseconds(window_ms / 2);
        let start = at - half;
        let end = at + Duration::milliseconds(window_ms - window_ms / 2);
        return Ok((
            SessionHistoryQuery::TimeRange { start, end },
            format!("around {} window_ms={window_ms}", at.to_rfc3339()),
        ));
    }

    let page = args.page.unwrap_or(0);
    if page < -1 {
        return Err(AgentToolError::InvalidArgs(
            "`page` must be -1 or a non-negative integer".to_string(),
        ));
    }
    let page_size = args.page_size.unwrap_or(DEFAULT_HISTORY_PAGE_SIZE);
    if page_size == 0 {
        return Err(AgentToolError::InvalidArgs(
            "`page_size` must be greater than zero".to_string(),
        ));
    }
    let page_size = page_size.min(MAX_HISTORY_PAGE_SIZE);
    Ok((
        SessionHistoryQuery::Page { page, page_size },
        format!("page={page} page_size={page_size}"),
    ))
}

fn parse_optional_time(
    ms: Option<u64>,
    rfc3339: Option<&str>,
    name: &str,
) -> Result<Option<DateTime<Utc>>, AgentToolError> {
    match (ms, rfc3339.map(str::trim).filter(|s| !s.is_empty())) {
        (Some(ms), None) => {
            let ms = i64::try_from(ms)
                .map_err(|_| AgentToolError::InvalidArgs(format!("`{name}_ms` is out of range")))?;
            DateTime::<Utc>::from_timestamp_millis(ms)
                .map(Some)
                .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{name}_ms` is invalid")))
        }
        (None, Some(value)) => DateTime::parse_from_rfc3339(value)
            .map(|dt| Some(dt.with_timezone(&Utc)))
            .map_err(|err| AgentToolError::InvalidArgs(format!("invalid `{name}` time: {err}"))),
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err(AgentToolError::InvalidArgs(format!(
            "use either `{name}_ms` or `{name}`, not both"
        ))),
    }
}

fn open_attention_store(agent_root: &Path) -> Result<AgentAttentionSignalStore, AgentToolError> {
    AgentAttentionSignalStore::open(AttentionSignalStoreConfig::new(
        agent_root.join("attention_signals"),
    ))
    .map_err(AgentToolError::from)
}

fn attention_runtime_path(agent_root: &Path, current_session_id: &str) -> PathBuf {
    session_dir(agent_root, current_session_id).join(ATTENTION_EXTRACTION_RUNTIME_REL_PATH)
}

async fn save_attention_runtime(
    agent_root: &Path,
    current_session_id: &str,
    runtime: &AttentionSignalToolRuntime,
) -> Result<(), AgentToolError> {
    let path = attention_runtime_path(agent_root, current_session_id);
    let dir = path.parent().ok_or_else(|| {
        AgentToolError::ExecFailed(format!("invalid runtime path {}", path.display()))
    })?;
    fs::create_dir_all(dir).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("mkdir {} failed: {err}", dir.display()))
    })?;
    let bytes = serde_json::to_vec_pretty(runtime).map_err(|err| {
        AgentToolError::ExecFailed(format!("serialize extraction runtime failed: {err}"))
    })?;
    fs::write(&path, bytes).await.map_err(|err| {
        AgentToolError::ExecFailed(format!("write {} failed: {err}", path.display()))
    })
}

async fn load_attention_runtime(
    agent_root: &Path,
    current_session_id: &str,
) -> Result<AttentionSignalToolRuntime, AgentToolError> {
    let path = attention_runtime_path(agent_root, current_session_id);
    let bytes = fs::read(&path).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            AgentToolError::InvalidArgs(
                "call BeginAttentionSignalExtraction before Discover* or CompleteAttentionSignalExtraction".to_string(),
            )
        } else {
            AgentToolError::ExecFailed(format!("read {} failed: {err}", path.display()))
        }
    })?;
    serde_json::from_slice(&bytes).map_err(|err| {
        AgentToolError::ExecFailed(format!("parse {} failed: {err}", path.display()))
    })
}

async fn remove_attention_runtime(
    agent_root: &Path,
    current_session_id: &str,
) -> Result<(), AgentToolError> {
    let path = attention_runtime_path(agent_root, current_session_id);
    match fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(AgentToolError::ExecFailed(format!(
            "remove {} failed: {err}",
            path.display()
        ))),
    }
}

#[async_trait]
impl WorkspaceToolBackend for CliWorkspaceBackend {
    async fn create_workspace(
        &self,
        ctx: &SessionRuntimeContext,
        name: String,
        summary: String,
    ) -> Result<Json, AgentToolError> {
        let session_id = ctx.session_id.trim();
        if session_id.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "session_id is required".to_string(),
            ));
        }
        let session = load_session_json(&self.state_root, session_id).await?;
        if build_session_summary_view(&session)
            .get("local_workspace_id")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            return Err(AgentToolError::InvalidArgs(format!(
                "session `{session_id}` already bound local workspace"
            )));
        }

        let now = now_ms();
        let workspace_id = format!("ws-{now:x}-{:x}", std::process::id());
        let mut index = load_workspace_index(&self.state_root).await?;
        let workspace_dir_name =
            allocate_cli_workspace_dir_name(&self.state_root, &index, &name).await?;
        let workspace_rel_path = format!("workspaces/{workspace_dir_name}");
        let workspace_path = self.state_root.join(&workspace_rel_path);
        fs::create_dir_all(&workspace_path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create workspace dir `{}` failed: {err}",
                workspace_path.display()
            ))
        })?;
        let summary_path = workspace_path.join("SUMMARY.md");
        fs::write(&summary_path, format!("{}\n", summary.trim()))
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "write workspace summary failed: path={} err={err}",
                    summary_path.display()
                ))
            })?;

        let workspace = CliWorkspaceRecord {
            workspace_id: workspace_id.clone(),
            name: name.trim().to_string(),
            relative_path: Some(workspace_rel_path.clone()),
            created_by_session: Some(session_id.to_string()),
            created_at_ms: now,
            updated_at_ms: now,
            bound_sessions: vec![CliLocalWorkspaceSessionBinding {
                session_id: session_id.to_string(),
                bound_at_ms: now,
            }],
        };
        index.workspaces.push(workspace.clone());
        index.agent_did = self.agent_id.clone();
        index.updated_at_ms = now;
        save_workspace_index(&self.state_root, &index).await?;

        let binding = CliSessionWorkspaceBinding {
            session_id: session_id.to_string(),
            local_workspace_id: workspace_id.clone(),
            workspace_path: workspace_path.to_string_lossy().to_string(),
            workspace_rel_path,
            agent_env_root: self.state_root.to_string_lossy().to_string(),
            bound_at_ms: now,
        };
        save_session_binding(&self.state_root, &binding).await?;
        let session_updated = persist_session_workspace_binding(
            &self.state_root,
            session_id,
            &workspace_id,
            Some(workspace.name.as_str()),
            &binding,
        )
        .await?;

        Ok(json!({
            "ok": true,
            "workspace": workspace,
            "binding": binding,
            "summary_path": summary_path.to_string_lossy().to_string(),
            "session_id": session_id,
            "session_updated": session_updated
        }))
    }

    async fn resolve_workspace_id(
        &self,
        workspace_ref: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<String, AgentToolError> {
        let workspace_ref = workspace_ref.trim();
        if workspace_ref.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "workspace argument cannot be empty".to_string(),
            ));
        }

        let index = load_workspace_index(&self.state_root).await?;
        if let Some(found) = index
            .workspaces
            .iter()
            .find(|item| item.workspace_id == workspace_ref)
        {
            return Ok(found.workspace_id.clone());
        }

        let parsed = Path::new(workspace_ref);
        let candidate = if parsed.is_absolute() {
            parsed.to_path_buf()
        } else if let Some(cwd) = shell_cwd {
            cwd.join(parsed)
        } else {
            std::env::current_dir()
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("read current_dir failed: {err}"))
                })?
                .join(parsed)
        };
        let normalized_candidate = canonicalize_or_normalize(candidate, None);
        for item in index.workspaces {
            let workspace_path = workspace_root_for_record(&self.state_root, &item);
            if canonicalize_or_normalize(workspace_path, None) == normalized_candidate {
                return Ok(item.workspace_id);
            }
        }

        Err(AgentToolError::InvalidArgs(format!(
            "workspace not found: `{workspace_ref}`; expected workspace_id or workspace_path"
        )))
    }

    async fn bind_workspace(
        &self,
        _ctx: &SessionRuntimeContext,
        session_id: &str,
        workspace_id: &str,
    ) -> Result<Json, AgentToolError> {
        let session = load_session_json(&self.state_root, session_id).await?;
        if build_session_summary_view(&session)
            .get("local_workspace_id")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_some()
        {
            return Err(AgentToolError::InvalidArgs(format!(
                "session `{session_id}` already bound local workspace"
            )));
        }
        if load_session_binding(&self.state_root, session_id)
            .await?
            .is_some()
        {
            return Err(AgentToolError::InvalidArgs(format!(
                "session `{session_id}` already bound local workspace"
            )));
        }

        let mut index = load_workspace_index(&self.state_root).await?;
        let Some(workspace) = index
            .workspaces
            .iter_mut()
            .find(|item| item.workspace_id == workspace_id)
        else {
            return Err(AgentToolError::InvalidArgs(format!(
                "workspace not found: `{workspace_id}`"
            )));
        };

        let now = now_ms();
        workspace.updated_at_ms = now;
        workspace
            .bound_sessions
            .push(CliLocalWorkspaceSessionBinding {
                session_id: session_id.to_string(),
                bound_at_ms: now,
            });
        let workspace_snapshot = workspace.clone();
        index.updated_at_ms = now;
        save_workspace_index(&self.state_root, &index).await?;

        let workspace_path = workspace_root_for_record(&self.state_root, &workspace_snapshot);
        let binding = CliSessionWorkspaceBinding {
            session_id: session_id.to_string(),
            local_workspace_id: workspace_id.to_string(),
            workspace_path: workspace_path.to_string_lossy().to_string(),
            workspace_rel_path: workspace_snapshot
                .relative_path
                .clone()
                .unwrap_or_else(|| format!("workspaces/{workspace_id}")),
            agent_env_root: self.state_root.to_string_lossy().to_string(),
            bound_at_ms: now,
        };
        save_session_binding(&self.state_root, &binding).await?;
        let session_updated = persist_session_workspace_binding(
            &self.state_root,
            session_id,
            workspace_id,
            Some(workspace_snapshot.name.as_str()),
            &binding,
        )
        .await?;

        Ok(json!({
            "ok": true,
            "binding": binding,
            "session_id": session_id,
            "session_updated": session_updated
        }))
    }
}

/// Single registry-of-tools used by the CLI dispatcher. Replaces the
/// per-tool `build_xxx_tool` factories — adding a new tool here is a one
/// line `register_typed_tool` call instead of a new branch in
/// `execute_bash_tool`. Built per-process invocation because the CLI is
/// short-lived and tools depend on the resolved env.
async fn build_cli_tool_manager(env: &CliRuntimeEnv) -> Result<AgentToolManager, AgentToolError> {
    let mgr = AgentToolManager::new();
    let state_root = cli_state_root(env);
    let file_cfg = build_cli_file_tool_config(env);

    mgr.register_typed_tool(GetSessionTool::new(Arc::new(CliSessionBackend {
        state_root: state_root.clone(),
    })))?;

    let workspace_backend = Arc::new(CliWorkspaceBackend {
        state_root: state_root.clone(),
        agent_id: env.call_ctx.agent_name.clone(),
    });
    mgr.register_typed_tool(CreateWorkspaceTool::new(workspace_backend.clone()))?;
    mgr.register_typed_tool(BindWorkspaceTool::new(workspace_backend))?;

    // NOTE: agent-memory is no longer a TypedTool — it has its own
    // top-level CLI dispatch (see `dispatch_agent_memory`) so the agent
    // can invoke it directly via shell per the v2.8 contract.

    let audit = Arc::new(NoopFileWriteAudit);
    mgr.register_typed_tool(GlobTool::new(file_cfg.clone()))?;
    mgr.register_typed_tool(GrepTool::new(file_cfg.clone()))?;
    mgr.register_typed_tool(DcrontabTool::new())?;
    mgr.register_typed_tool(ReadFileTool::new(file_cfg.clone()))?;
    mgr.register_typed_tool(WriteFileTool::new(file_cfg.clone(), audit.clone()))?;
    mgr.register_typed_tool(EditFileTool::new(file_cfg, audit))?;
    mgr.register_typed_tool(TodoTool::new(TodoToolConfig::new(state_root)))?;
    mgr.register_typed_tool(CliReadSessionHistoryTool {
        agent_root: env.agent_env_root.clone(),
    })?;
    mgr.register_typed_tool(CliCommitSessionHistoryImprovedTool {
        agent_root: env.agent_env_root.clone(),
    })?;
    mgr.register_typed_tool(CliBeginAttentionSignalExtractionTool {
        agent_root: env.agent_env_root.clone(),
        current_session_id: env.call_ctx.session_id.clone(),
        agent_id: env.call_ctx.agent_name.clone(),
    })?;
    mgr.register_typed_tool(CliCompleteAttentionSignalExtractionTool {
        agent_root: env.agent_env_root.clone(),
        current_session_id: env.call_ctx.session_id.clone(),
    })?;
    let attention_store = Arc::new(open_attention_store(&env.agent_env_root)?);
    mgr.register_typed_tool(CliDiscoverEventTool {
        store: attention_store.clone(),
        agent_root: env.agent_env_root.clone(),
        current_session_id: env.call_ctx.session_id.clone(),
    })?;
    mgr.register_typed_tool(CliDiscoverObjectObservationTool {
        store: attention_store.clone(),
        agent_root: env.agent_env_root.clone(),
        current_session_id: env.call_ctx.session_id.clone(),
    })?;
    mgr.register_typed_tool(CliDiscoverRelationshipTool {
        store: attention_store.clone(),
        agent_root: env.agent_env_root.clone(),
        current_session_id: env.call_ctx.session_id.clone(),
    })?;
    mgr.register_typed_tool(CliDiscoverSkillCoverageGapTool {
        store: attention_store.clone(),
        agent_root: env.agent_env_root.clone(),
        current_session_id: env.call_ctx.session_id.clone(),
    })?;
    mgr.register_typed_tool(CliListPendingAttentionSignalsTool {
        store: attention_store.clone(),
        agent_scope_id: env.call_ctx.agent_name.clone(),
    })?;
    mgr.register_typed_tool(CliMarkAttentionSignalConsumedTool {
        store: attention_store,
    })?;

    Ok(mgr)
}

fn build_cli_file_tool_config(env: &CliRuntimeEnv) -> FileToolConfig {
    let mut cfg = FileToolConfig::new(env.current_dir.clone());
    cfg.allowed_read_roots.clear();
    if !env.has_agent_env {
        cfg.allowed_write_roots.clear();
    }
    cfg
}

fn success_result(tool_name: &str, result: AgentToolResult) -> AgentToolResult {
    cli_result_from_tool_result(tool_name, result)
}

fn render_plain_read_file_output(result: AgentToolResult) -> CliRunOutput {
    let stdout = result
        .details
        .get("content")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    CliRunOutput {
        exit_code: EXIT_SUCCESS,
        stdout,
        stderr: String::new(),
    }
}

fn render_plain_error_output(err: &AgentToolError) -> CliRunOutput {
    CliRunOutput {
        exit_code: cli_exit_code_for_error(err),
        stdout: String::new(),
        stderr: err.to_string(),
    }
}

/// Help text is built from each tool's own `usage()` rather than a
/// duplicated static table — the manager is the source of truth.
async fn build_help_result(env: &CliRuntimeEnv, tool_name: Option<&str>) -> AgentToolResult {
    let mgr = match build_cli_tool_manager(env).await {
        Ok(mgr) => mgr,
        Err(err) => return cli_error_result(tool_name.map(str::to_string).as_deref(), &err),
    };
    let tool_usage = |name: &str| -> String {
        if let Some(tool) = mgr.get_any_tool(name) {
            if let Some(usage) = tool.spec().usage {
                return usage;
            }
        }
        match name {
            TOOL_CHECK_TASK => "check_task <task_id>".to_string(),
            TOOL_CANCEL_TASK => "cancel_task <task_id> [--recursive]".to_string(),
            TOOL_FINISH_TASK => "finish_task <task_id> [failed] [--message <text>]".to_string(),
            TOOL_AGENT_SKILLS | TOOL_AGENT_SKILLS_SNAKE => AGENT_SKILLS_USAGE.to_string(),
            _ => format!("{name} ..."),
        }
    };
    match tool_name {
        Some(name) => cli_success_result(
            Some(name.to_string()),
            json!({ "tool": name, "usage": tool_usage(name) }),
            "show usage",
        ),
        None => cli_success_result(
            None,
            json!({
                "usage": generic_usage(),
                "tools": TOOL_NAMES.iter().map(|name| json!({
                    "name": name,
                    "usage": tool_usage(name),
                })).collect::<Vec<_>>(),
            }),
            "show usage",
        ),
    }
}

fn with_tool_usage(message: impl Into<String>, tool_name: &str) -> AgentToolError {
    let usage = match tool_name {
        TOOL_READ_OBJECT => {
            "read <object-or-path> [--content-only] [--offset <1-based-line>] [--limit <lines>] [--config <route.toml>]"
        }
        TOOL_X_CALL | TOOL_X_CALL_SNAKE => {
            "x-call <object> <action> [--params <json>] [key=value ...] [--key=value ...] [--config <route.toml>]"
        }
        TOOL_CHECK_TASK => "check_task <task_id>",
        TOOL_CANCEL_TASK => "cancel_task <task_id> [--recursive]",
        TOOL_FINISH_TASK => "finish_task <task_id> [failed] [--message <text>]",
        TOOL_AGENT_SKILLS | TOOL_AGENT_SKILLS_SNAKE => AGENT_SKILLS_USAGE,
        _ => "agent_tool <tool> ...",
    };
    AgentToolError::InvalidArgs(format!("{}\nUsage: {usage}", message.into()))
}

fn required_token(
    tokens: &[String],
    idx: usize,
    flag: &str,
    tool_name: &str,
) -> Result<String, AgentToolError> {
    tokens
        .get(idx)
        .cloned()
        .ok_or_else(|| with_tool_usage(format!("missing value for `{flag}`"), tool_name))
}

fn parse_object_usize(raw: &str, name: &str, tool_name: &str) -> Result<usize, AgentToolError> {
    raw.trim().parse::<usize>().map_err(|_| {
        with_tool_usage(
            format!("invalid `{name}` value `{raw}`, expected non-negative integer"),
            tool_name,
        )
    })
}

fn parse_object_bool(raw: &str, name: &str, tool_name: &str) -> Result<bool, AgentToolError> {
    match raw.trim() {
        "true" | "1" | "yes" => Ok(true),
        "false" | "0" | "no" => Ok(false),
        _ => Err(with_tool_usage(
            format!("invalid `{name}` value `{raw}`, expected bool"),
            tool_name,
        )),
    }
}

fn parse_json_arg(raw: &str, name: &str, tool_name: &str) -> Result<Json, AgentToolError> {
    serde_json::from_str(raw).map_err(|err| {
        with_tool_usage(
            format!("invalid `{name}` JSON value `{raw}`: {err}"),
            tool_name,
        )
    })
}

fn parse_scalar_json_value(raw: &str) -> Json {
    let value = raw.trim();
    match value {
        "true" => Json::Bool(true),
        "false" => Json::Bool(false),
        "null" => Json::Null,
        _ => value
            .parse::<i64>()
            .map(|n| json!(n))
            .or_else(|_| value.parse::<f64>().map(|n| json!(n)))
            .unwrap_or_else(|_| Json::String(raw.to_string())),
    }
}

fn canonical_object_url_for_cli(raw: &str, current_dir: &Path) -> Result<String, AgentToolError> {
    let raw = raw.trim();
    if raw.contains("://") {
        return Ok(raw.to_string());
    }
    let path = resolve_cli_path(raw, current_dir);
    Ok(format!("file://{}", percent_encode_file_path(&path)))
}

fn resolve_cli_path(raw: &str, current_dir: &Path) -> PathBuf {
    let path = PathBuf::from(raw);
    canonicalize_or_normalize(path, Some(current_dir))
}

fn percent_encode_file_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let mut out = String::new();
    for byte in raw.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'.' | b'_' | b'~' => {
                out.push(*byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn object_error_to_agent_tool_error(
    err: agent_did_object_lib::AgentDIDObjectError,
) -> AgentToolError {
    use agent_did_object_lib::AgentDIDObjectError as ObjErr;
    match err {
        ObjErr::InvalidConfig(message)
        | ObjErr::RouteNotFound(message)
        | ObjErr::UnsupportedObjectRef(message)
        | ObjErr::UnsupportedMethod(message)
        | ObjErr::DeclaredCapabilityNotFound(message)
        | ObjErr::SchemaError(message) => AgentToolError::InvalidArgs(message),
        ObjErr::AdapterNotFound(message)
        | ObjErr::AdapterUnavailable(message)
        | ObjErr::ResolveError(message)
        | ObjErr::HttpError(message)
        | ObjErr::ProtocolError(message)
        | ObjErr::KEventError(message)
        | ObjErr::EventBridgeError(message)
        | ObjErr::AdapterError(message) => AgentToolError::ExecFailed(message),
    }
}

fn generic_usage() -> String {
    format!("agent_tool <{}> [args...]", TOOL_NAMES.join("|"))
}

fn first_string_env(keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        env::var(key)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    })
}

fn first_path_env(keys: &[&str], current_dir: &Path) -> Option<PathBuf> {
    keys.iter().find_map(|key| env::var_os(key)).map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            canonicalize_or_normalize(path, None)
        } else {
            canonicalize_or_normalize(path, Some(current_dir))
        }
    })
}

fn is_tool_name(raw: &str) -> bool {
    TOOL_NAMES.iter().any(|tool_name| tool_name == &raw)
}

fn os_to_string(value: &OsString) -> Result<String, AgentToolError> {
    value.clone().into_string().map_err(|_| {
        AgentToolError::InvalidArgs("command line arguments must be valid UTF-8".to_string())
    })
}

fn session_file_path(state_root: &Path, session_id: &str) -> Result<PathBuf, AgentToolError> {
    session_record_path(
        &state_root.join("sessions"),
        session_id,
        SESSION_RECORD_FILE,
    )
}

async fn load_session_json(state_root: &Path, session_id: &str) -> Result<Json, AgentToolError> {
    let path = session_file_path(state_root, session_id)?;
    let raw = fs::read_to_string(&path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "read session file `{}` failed: {err}",
            path.display()
        ))
    })?;
    serde_json::from_str(&raw).map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "parse session file `{}` failed: {err}",
            path.display()
        ))
    })
}

async fn save_session_json(
    state_root: &Path,
    session_id: &str,
    session: &Json,
) -> Result<(), AgentToolError> {
    let path = session_file_path(state_root, session_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create session dir `{}` failed: {err}",
                parent.display()
            ))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(session)
        .map_err(|err| AgentToolError::ExecFailed(format!("serialize session failed: {err}")))?;
    fs::write(&path, bytes).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "write session file `{}` failed: {err}",
            path.display()
        ))
    })
}

fn build_session_summary_view(session: &Json) -> Json {
    let runtime_state = session
        .pointer("/meta/runtime_state")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let status = session
        .get("status")
        .and_then(Json::as_str)
        .unwrap_or("wait")
        .trim()
        .to_string();
    let state = runtime_state
        .get("state")
        .and_then(Json::as_str)
        .map(|value| value.to_ascii_uppercase())
        .unwrap_or_else(|| status.to_ascii_uppercase());
    json!({
        "session_id": session.get("session_id").cloned().unwrap_or_else(|| Json::String(String::new())),
        "status": status,
        "state": state,
        "title": session.get("title").cloned().unwrap_or(Json::Null),
        "summary": session.get("summary").cloned().unwrap_or(Json::Null),
        "current_behavior": runtime_state.get("current_behavior").cloned().unwrap_or(Json::Null),
        "default_remote": runtime_state.get("default_remote").cloned().unwrap_or(Json::Null),
        "step_index": runtime_state.get("step_index").cloned().unwrap_or_else(|| json!(0)),
        "updated_at_ms": session.get("updated_at_ms").cloned().unwrap_or_else(|| json!(0)),
        "last_activity_ms": session.get("last_activity_ms").cloned().unwrap_or_else(|| json!(0)),
        "new_msg_count": 0,
        "new_event_count": 0,
        "history_msg_count": 0,
        "history_event_count": 0,
        "new_link_count": 0,
        "workspace_info": runtime_state.get("workspace_info").cloned().unwrap_or(Json::Null),
        "local_workspace_id": runtime_state.get("local_workspace_id").cloned().unwrap_or(Json::Null),
        "meta": session.get("meta").cloned().unwrap_or_else(|| json!({})),
    })
}

async fn load_workspace_index(state_root: &Path) -> Result<CliWorkspaceIndex, AgentToolError> {
    let path = state_root.join(WORKSPACE_INDEX_FILE);
    match fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "parse workspace index `{}` failed: {err}",
                path.display()
            ))
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(CliWorkspaceIndex::default()),
        Err(err) => Err(AgentToolError::ExecFailed(format!(
            "read workspace index `{}` failed: {err}",
            path.display()
        ))),
    }
}

async fn save_workspace_index(
    state_root: &Path,
    index: &CliWorkspaceIndex,
) -> Result<(), AgentToolError> {
    let path = state_root.join(WORKSPACE_INDEX_FILE);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create workspace index dir `{}` failed: {err}",
                parent.display()
            ))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(index).map_err(|err| {
        AgentToolError::ExecFailed(format!("serialize workspace index failed: {err}"))
    })?;
    fs::write(&path, bytes).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "write workspace index `{}` failed: {err}",
            path.display()
        ))
    })
}

async fn load_session_bindings_file(
    state_root: &Path,
) -> Result<CliSessionBindingsFile, AgentToolError> {
    let path = state_root.join(SESSION_WORKSPACE_BINDINGS_REL_PATH);
    match fs::read_to_string(&path).await {
        Ok(raw) => serde_json::from_str(&raw).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "parse session bindings `{}` failed: {err}",
                path.display()
            ))
        }),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            Ok(CliSessionBindingsFile::default())
        }
        Err(err) => Err(AgentToolError::ExecFailed(format!(
            "read session bindings `{}` failed: {err}",
            path.display()
        ))),
    }
}

async fn save_session_bindings_file(
    state_root: &Path,
    file: &CliSessionBindingsFile,
) -> Result<(), AgentToolError> {
    let path = state_root.join(SESSION_WORKSPACE_BINDINGS_REL_PATH);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create session bindings dir `{}` failed: {err}",
                parent.display()
            ))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(file).map_err(|err| {
        AgentToolError::ExecFailed(format!("serialize session bindings failed: {err}"))
    })?;
    fs::write(&path, bytes).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "write session bindings `{}` failed: {err}",
            path.display()
        ))
    })
}

async fn load_session_binding(
    state_root: &Path,
    session_id: &str,
) -> Result<Option<CliSessionWorkspaceBinding>, AgentToolError> {
    let file = load_session_bindings_file(state_root).await?;
    Ok(file
        .bindings
        .into_iter()
        .find(|item| item.session_id.trim() == session_id))
}

async fn save_session_binding(
    state_root: &Path,
    binding: &CliSessionWorkspaceBinding,
) -> Result<(), AgentToolError> {
    let mut file = load_session_bindings_file(state_root).await?;
    file.bindings
        .retain(|item| item.session_id.trim() != binding.session_id.trim());
    file.bindings.push(binding.clone());
    save_session_bindings_file(state_root, &file).await
}

async fn persist_session_workspace_binding(
    state_root: &Path,
    session_id: &str,
    workspace_id: &str,
    workspace_name: Option<&str>,
    binding: &CliSessionWorkspaceBinding,
) -> Result<bool, AgentToolError> {
    let mut session = load_session_json(state_root, session_id).await?;
    let Some(root_map) = session.as_object_mut() else {
        return Err(AgentToolError::ExecFailed(
            "session record must be a json object".to_string(),
        ));
    };
    let meta = root_map
        .entry("meta".to_string())
        .or_insert_with(|| json!({}));
    if !meta.is_object() {
        *meta = json!({});
    }
    let meta_map = meta.as_object_mut().expect("meta object");
    if !meta_map.contains_key("runtime_state") {
        meta_map.insert("runtime_state".to_string(), json!({}));
    }
    let runtime_state = meta_map
        .get_mut("runtime_state")
        .expect("runtime_state present");
    if !runtime_state.is_object() {
        *runtime_state = json!({});
    }
    let workspace_info = json!({
        "workspace_id": workspace_id,
        "local_workspace_id": workspace_id,
        "workspace_name": workspace_name.unwrap_or(""),
        "workspace_type": "local",
        "binding": binding
    });
    let runtime_map = runtime_state.as_object_mut().expect("runtime_state object");
    runtime_map.insert(
        "local_workspace_id".to_string(),
        Json::String(workspace_id.to_string()),
    );
    runtime_map.insert("workspace_info".to_string(), workspace_info);
    let now = now_ms();
    root_map.insert("updated_at_ms".to_string(), json!(now));
    root_map.insert("last_activity_ms".to_string(), json!(now));
    save_session_json(state_root, session_id, &session).await?;
    Ok(true)
}

fn workspace_root_for_record(state_root: &Path, record: &CliWorkspaceRecord) -> PathBuf {
    record
        .relative_path
        .as_deref()
        .map(|rel| state_root.join(rel))
        .unwrap_or_else(|| state_root.join("workspaces").join(&record.workspace_id))
}

async fn allocate_cli_workspace_dir_name(
    state_root: &Path,
    index: &CliWorkspaceIndex,
    workspace_name: &str,
) -> Result<String, AgentToolError> {
    let base_name = sanitize_cli_workspace_dir_name(workspace_name);

    for suffix in 1u32.. {
        let candidate = if suffix == 1 {
            base_name.clone()
        } else {
            format!("{base_name}-{suffix}")
        };

        let already_indexed = index.workspaces.iter().any(|item| {
            item.relative_path
                .as_deref()
                .and_then(|rel| Path::new(rel).file_name())
                .and_then(|value| value.to_str())
                == Some(candidate.as_str())
        });
        if already_indexed {
            continue;
        }

        let candidate_path = state_root.join("workspaces").join(&candidate);
        if !fs::try_exists(&candidate_path).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "check workspace dir `{}` failed: {err}",
                candidate_path.display()
            ))
        })? {
            return Ok(candidate);
        }
    }

    unreachable!("workspace dir allocation should always find a candidate")
}

fn sanitize_cli_workspace_dir_name(workspace_name: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;

    for ch in workspace_name.trim().chars() {
        let is_forbidden =
            ch.is_control() || matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
        if is_forbidden {
            if !out.is_empty() {
                pending_dash = true;
            }
            continue;
        }

        if pending_dash && !out.ends_with('-') {
            out.push('-');
        }
        pending_dash = false;
        out.push(ch);
    }

    let sanitized = out.trim_matches([' ', '.']).trim();
    match sanitized {
        "" | "." | ".." => "workspace".to_string(),
        _ => sanitized.to_string(),
    }
}

async fn build_task_manager_client(
    env: &CliRuntimeEnv,
) -> Result<TaskManagerClient, AgentToolError> {
    if let Ok(runtime) = get_buckyos_api_runtime() {
        return runtime.get_task_mgr_client().await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "init task-manager client from runtime failed: {err}"
            ))
        });
    }

    if env.allow_dev_overrides() {
        if let Some(client) = resolve_dev_task_manager_client() {
            return Ok(client);
        }
    }

    require_runtime_token_for_rpc(env)?;
    let runtime = init_buckyos_api_runtime("opendan", None, BuckyOSRuntimeType::FrameService)
        .await
        .map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "init runtime for task-manager access failed: {err}"
            ))
        })?;
    runtime.get_task_mgr_client().await.map_err(|err| {
        AgentToolError::ExecFailed(format!("init task-manager client failed: {err}"))
    })
}

fn build_check_task_result(tool_name: &str, task: Task) -> AgentToolResult {
    let top_status = task_protocol_status(&task);
    let summary = task_summary(&task, top_status);
    let pending_reason = task_pending_reason(&task);
    let exec_bash_task = tool_exec_bash_task_data(&task);
    let is_exec_bash_task = exec_bash_task.is_some();
    let mut detail = if is_exec_bash_task {
        json!({})
    } else {
        normalized_task_detail(&task)
    };
    if !is_exec_bash_task {
        if let Some(map) = detail.as_object_mut() {
            map.insert("task".to_string(), json!(task.clone()));
        }
    }

    let cmd_line = if is_exec_bash_task {
        exec_bash_task
            .as_ref()
            .and_then(|data| data.command.clone())
    } else {
        Some(format!("{tool_name} {}", task.task_id))
    };
    let output = exec_bash_task.as_ref().and_then(|data| data.output.clone());
    let return_code = exec_bash_task
        .as_ref()
        .and_then(|data| data.return_code.or(data.exit_code));
    let estimated_wait = exec_bash_task
        .as_ref()
        .and_then(|data| data.estimated_wait.clone());
    let check_after = exec_bash_task
        .as_ref()
        .and_then(|data| data.check_after)
        .or_else(|| (top_status == AgentToolStatus::Pending).then_some(5));

    let mut result = AgentToolResult::from_details(detail)
        .with_status(top_status)
        .with_result(summary)
        .with_task_id(task.task_id.clone());
    if !is_exec_bash_task {
        result = result.with_tool(tool_name);
    }
    if let Some(cmd_line) = cmd_line.as_deref() {
        result = result.with_command_metadata_from_line(cmd_line);
    }
    if let Some(output) = output {
        result = result.with_output(output);
    }
    if let Some(rc) = return_code {
        result = result.with_return_code(rc);
    }
    if let Some(reason) = pending_reason {
        result = result.with_pending_reason(reason);
    }
    if let Some(wait) = estimated_wait {
        result = result.with_estimated_wait(wait);
    }
    if let Some(after) = check_after {
        result = result.with_check_after(after);
    }
    result
}

fn build_cancel_task_result(
    tool_name: &str,
    task: Task,
    recursive: bool,
    interrupt_error: Option<String>,
) -> AgentToolResult {
    let mut detail = normalized_task_detail(&task);
    if let Some(map) = detail.as_object_mut() {
        map.insert("task".to_string(), json!(task.clone()));
        map.insert("recursive".to_string(), Json::Bool(recursive));
        if let Some(err) = interrupt_error.as_ref() {
            map.insert("interrupt_error".to_string(), Json::String(err.clone()));
        }
    }

    let summary = match interrupt_error {
        Some(err) => format!("canceled task {} (interrupt failed: {err})", task.task_id),
        None => format!("canceled task {}", task.task_id),
    };

    AgentToolResult::from_details(detail)
        .with_status(AgentToolStatus::Success)
        .with_result(summary)
        .with_title(format!("{tool_name} {} => success", task.task_id))
        .with_tool(tool_name)
        .with_cmd_line(format!("{tool_name} {}", task.task_id))
        .with_task_id(task.task_id.clone())
}

fn build_finish_task_result(
    tool_name: &str,
    task: Task,
    outcome: FinishTaskOutcome,
) -> AgentToolResult {
    let mut detail = normalized_task_detail(&task);
    if let Some(map) = detail.as_object_mut() {
        map.insert("task".to_string(), json!(task.clone()));
        map.insert(
            "finish_outcome".to_string(),
            Json::String(
                match outcome {
                    FinishTaskOutcome::Completed => "completed",
                    FinishTaskOutcome::Failed => "failed",
                }
                .to_string(),
            ),
        );
    }
    let outcome_text = match outcome {
        FinishTaskOutcome::Completed => "finished",
        FinishTaskOutcome::Failed => "failed",
    };

    AgentToolResult::from_details(detail)
        .with_status(AgentToolStatus::Success)
        .with_result(format!("{outcome_text} task {}", task.task_id))
        .with_title(format!("{tool_name} {} {outcome_text} => success", task.task_id))
        .with_tool(tool_name)
        .with_cmd_line(match outcome {
            FinishTaskOutcome::Completed => format!("{tool_name} {}", task.task_id),
            FinishTaskOutcome::Failed => format!("{tool_name} {} failed", task.task_id),
        })
        .with_task_id(task.task_id.clone())
}

fn task_data_payload(task: &Task) -> Json {
    task.result
        .clone()
        .or_else(|| task.progress.clone())
        .unwrap_or_else(|| task.input.clone())
}

fn task_status_text(task: &Task) -> String {
    match task.outcome {
        Some(outcome) => outcome.to_string(),
        None => task.phase.to_string(),
    }
}

fn normalized_task_detail(task: &Task) -> Json {
    let payload = task_data_payload(task);
    let mut detail = if payload.is_object() {
        payload
    } else {
        json!({ "task_data": payload })
    };
    if let Some(map) = detail.as_object_mut() {
        map.entry("task_id".to_string())
            .or_insert_with(|| Json::String(task.task_id.clone()));
        map.entry("task_status".to_string())
            .or_insert_with(|| Json::String(task_status_text(task)));
        map.entry("task_name".to_string())
            .or_insert_with(|| Json::String(task.name.clone()));
        map.entry("task_type".to_string())
            .or_insert_with(|| Json::String(task.schema_id.clone()));
        map.entry("task_progress".to_string())
            .or_insert_with(|| task.progress.clone().unwrap_or(Json::Null));
        if let Some(message) = task.message.as_ref() {
            map.entry("task_message".to_string())
                .or_insert_with(|| Json::String(message.clone()));
        }
    }
    detail
}

fn task_protocol_status(task: &Task) -> AgentToolStatus {
    match (task.phase, task.outcome) {
        (TaskPhase::Terminal, Some(TaskOutcome::Succeeded)) => {
            match tool_exec_bash_task_data(task)
                .and_then(|data| data.status)
                .as_deref()
            {
                Some("error") => AgentToolStatus::Error,
                _ => AgentToolStatus::Success,
            }
        }
        (TaskPhase::Terminal, _) => AgentToolStatus::Error,
        _ => AgentToolStatus::Pending,
    }
}

fn task_summary(task: &Task, protocol_status: AgentToolStatus) -> String {
    let exec_summary = tool_exec_bash_task_data(task).and_then(|data| data.summary);
    exec_summary
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| task.message.as_ref().map(|value| value.trim().to_string()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| match (protocol_status, task.phase, task.outcome) {
            (AgentToolStatus::Pending, TaskPhase::Waiting, _) => {
                format!("task {} is waiting for approval", task.task_id)
            }
            (AgentToolStatus::Pending, _, _) => {
                format!("task {} is still running", task.task_id)
            }
            (AgentToolStatus::Success, _, _) => format!("task {} completed", task.task_id),
            (AgentToolStatus::Error, _, Some(TaskOutcome::Canceled)) => {
                format!("task {} was canceled", task.task_id)
            }
            (AgentToolStatus::Error, _, _) => format!("task {} failed", task.task_id),
        })
}

fn task_pending_reason(task: &Task) -> Option<AgentToolPendingReason> {
    let pending_reason = tool_exec_bash_task_data(task).and_then(|data| data.pending_reason);
    pending_reason
        .as_deref()
        .and_then(|value| match value {
            "user_approval" => Some(AgentToolPendingReason::UserApproval),
            "wait_for_install" | "external_callback" => {
                Some(AgentToolPendingReason::WaitForInstall)
            }
            "long_running" => Some(AgentToolPendingReason::LongRunning),
            _ => None,
        })
        .or_else(|| match task.phase {
            TaskPhase::Waiting => Some(AgentToolPendingReason::UserApproval),
            TaskPhase::Promised
            | TaskPhase::Accepted
            | TaskPhase::Running
            | TaskPhase::Paused => Some(AgentToolPendingReason::LongRunning),
            TaskPhase::Terminal => None,
        })
}

async fn interrupt_task_if_supported(task: &Task) -> Option<String> {
    let tmux_target = tool_exec_bash_task_data(task)?.tmux_target?;
    let tmux_target = tmux_target.trim();
    if tmux_target.is_empty() {
        return None;
    }

    let output = match Command::new("tmux")
        .args(["send-keys", "-t", tmux_target, "C-c"])
        .output()
        .await
    {
        Ok(output) => output,
        Err(err) => return Some(format!("tmux interrupt `{tmux_target}` failed: {err}")),
    };
    if output.status.success() {
        return None;
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Some(if stderr.is_empty() {
        format!("tmux interrupt `{tmux_target}` failed")
    } else {
        format!("tmux interrupt `{tmux_target}` failed: {stderr}")
    })
}

fn tool_exec_bash_task_data(task: &Task) -> Option<ToolExecBashTaskData> {
    match parse_typed_task_data(TaskDataType::ToolExecBash.as_str(), task_data_payload(task)).ok()? {
        TypedTaskData::ToolExecBash(data) if data.kind == TaskDataType::ToolExecBash.as_str() => {
            Some(data)
        }
        _ => None,
    }
}

fn canonicalize_or_normalize(path: PathBuf, base_dir: Option<&Path>) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        base_dir.map(|base| base.join(&path)).unwrap_or(path)
    };
    std::fs::canonicalize(&absolute).unwrap_or_else(|_| normalize_abs_path(&absolute))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Mutex;

    use agent_tool::RuntimeContextSource;
    use buckyos_api::{AiMessage, AiRole};
    use opendan::round_history::{ContextMode, RoundStatus, RoundTrigger, SessionHistoryWriter};
    use tempfile::tempdir;
    use tokio::fs;
    use tokio::io::AsyncWriteExt as _;
    use tokio::net::TcpListener;

    /// Env-mutating tests must hold this lock so they don't race with each
    /// other or with notebook tests that rely on `AGENT_NOTEBOOK_ROOT` /
    /// `OPENDAN_*` being unset. cargo runs tests on a thread pool, so any
    /// notebook CLI test acquires this lock — the cost is fully serializing
    /// six fast tests against each other, which beats flakiness.
    static ENV_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn nb_lock() -> std::sync::MutexGuard<'static, ()> {
        ENV_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn test_env(agent_env_root: PathBuf, current_dir: PathBuf) -> CliRuntimeEnv {
        let agent_env_root = canonicalize_or_normalize(agent_env_root, None);
        seed_agent_identity(&agent_env_root, "alice", "did:example:agent");
        let runtime_context = RuntimeContext::from_agent_root(
            agent_env_root.clone(),
            "session-test".to_string(),
            Some("test-token".to_string()),
            "trace-test".to_string(),
            RuntimeContextSource::StableEnv,
        )
        .expect("runtime context");
        CliRuntimeEnv {
            agent_env_root,
            has_agent_env: true,
            current_dir: canonicalize_or_normalize(current_dir, None),
            stdout_is_terminal: true,
            runtime_context,
            call_ctx: SessionRuntimeContext {
                trace_id: "trace-test".to_string(),
                agent_name: "did:example:agent".to_string(),
                behavior: "cli".to_string(),
                step_idx: 0,
                wakeup_id: "wakeup-test".to_string(),
                session_id: "session-test".to_string(),
                read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
            },
        }
    }

    async fn read_json_http_request(stream: &mut tokio::net::TcpStream) -> (String, Json) {
        let mut request = Vec::new();
        let mut chunk = [0_u8; 1024];
        loop {
            let n = stream.read(&mut chunk).await.expect("read request");
            assert!(n > 0, "request ended before its body was complete");
            request.extend_from_slice(&chunk[..n]);

            let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n")
            else {
                continue;
            };
            let headers = String::from_utf8_lossy(&request[..header_end]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .unwrap_or(0);
            let body_start = header_end + 4;
            if request.len() < body_start + content_length {
                continue;
            }

            let headers = headers.to_string();
            let body = serde_json::from_slice(&request[body_start..body_start + content_length])
                .expect("parse request body");
            return (headers, body);
        }
    }

    fn dev_test_env(agent_env_root: PathBuf, current_dir: PathBuf) -> CliRuntimeEnv {
        let agent_env_root = canonicalize_or_normalize(agent_env_root, None);
        let runtime_context = RuntimeContext::from_agent_root(
            agent_env_root.clone(),
            "session-test".to_string(),
            None,
            "trace-test".to_string(),
            RuntimeContextSource::DevFallback,
        )
        .expect("runtime context");
        CliRuntimeEnv {
            agent_env_root,
            has_agent_env: false,
            current_dir: canonicalize_or_normalize(current_dir, None),
            stdout_is_terminal: true,
            runtime_context,
            call_ctx: SessionRuntimeContext {
                trace_id: "trace-test".to_string(),
                agent_name: "did:example:agent".to_string(),
                behavior: "cli".to_string(),
                step_idx: 0,
                wakeup_id: "wakeup-test".to_string(),
                session_id: "session-test".to_string(),
                read_token_limit: DEFAULT_READ_TOKEN_LIMIT,
            },
        }
    }

    async fn seed_session(agent_env_root: &Path, session_id: &str, pwd: &Path) {
        let now = now_ms();
        let session = json!({
            "session_id": session_id,
            "owner_agent": "did:example:agent",
            "title": "CLI Session",
            "summary": "",
            "status": "wait",
            "created_at_ms": now,
            "updated_at_ms": now,
            "last_activity_ms": now,
            "meta": {
                "runtime_state": {
                    "state": "wait",
                    "current_behavior": "plan",
                    "step_index": 0,
                    "local_workspace_id": Json::Null,
                    "workspace_info": {
                        "workspace_path": pwd.to_string_lossy().to_string()
                    }
                }
            }
        });
        save_session_json(agent_env_root, session_id, &session)
            .await
            .expect("save session");
    }

    fn seed_agent_identity(agent_root: &Path, owner_user_id: &str, agent_id: &str) {
        std::fs::create_dir_all(agent_root).expect("create agent root");
        std::fs::write(
            agent_root.join("agent.toml"),
            format!(
                "[identity]\nowner_user_id = \"{}\"\nagent_id = \"{}\"\n",
                owner_user_id, agent_id
            ),
        )
        .expect("write agent identity");
    }

    async fn seed_opendan_session_meta(agent_root: &Path, session_id: &str, kind: SessionKind) {
        let session_dir = agent_root.join("sessions").join(session_id);
        fs::create_dir_all(session_dir.join(".meta"))
            .await
            .expect("create session meta dir");
        let meta = SessionMeta::new(
            session_id.to_string(),
            kind,
            "chat_route".to_string(),
            "alice".to_string(),
        );
        let bytes = serde_json::to_vec_pretty(&meta).expect("serialize session meta");
        fs::write(session_dir.join(".meta").join("session.json"), bytes)
            .await
            .expect("write session meta");
    }

    async fn seed_round_history(agent_root: &Path, session_id: &str, text: &str) {
        let session_dir = agent_root.join("sessions").join(session_id);
        let mut writer = SessionHistoryWriter::open(&session_dir)
            .await
            .expect("open history writer");
        writer
            .begin_round(
                RoundTrigger::UserMsg {
                    preview: text.to_string(),
                },
                Vec::new(),
                ContextMode::Chat,
            )
            .await
            .expect("begin round");
        writer
            .append_message(AiMessage::text(AiRole::User, text), None)
            .await
            .expect("append message");
        writer
            .finalize_round(RoundStatus::Completed)
            .await
            .expect("finalize round");
    }

    #[tokio::test]
    async fn read_file_alias_returns_structured_json() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        fs::write(cwd.join("demo.txt"), "line-1\nline-2\n")
            .await
            .expect("write demo file");

        let output = execute(
            vec![
                OsString::from("/tmp/read_file"),
                OsString::from("demo.txt"),
                OsString::from("1-1"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run read_file");

        assert_eq!(output.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["cmd_name"], "read_file");
        let cmd_args = payload["cmd_args"].as_str().expect("cmd_args string");
        assert!(cmd_args.ends_with("/demo.txt range=1-1"));
        assert_eq!(payload["detail"]["content"], "line-1\n");
    }

    #[tokio::test]
    async fn object_read_cli_uses_agent_did_runtime_for_bare_path() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        fs::write(cwd.join("demo.txt"), "line-1\nline-2\n")
            .await
            .expect("write demo file");

        let output = execute(
            vec![
                OsString::from("/tmp/agent_tool"),
                OsString::from("read"),
                OsString::from("demo.txt"),
                OsString::from("--content-only"),
                OsString::from("--offset"),
                OsString::from("2"),
                OsString::from("--limit"),
                OsString::from("1"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run object read");

        assert_eq!(output.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["tool"], "read");
        assert_eq!(payload["cmd_name"], "read");
        assert_eq!(payload["output"], "line-2");
    }

    #[tokio::test]
    async fn object_read_cli_returns_unchanged_for_same_session_id_env() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        fs::write(cwd.join("demo.txt"), "line-1\nline-2\n")
            .await
            .expect("write demo file");
        seed_agent_identity(&root, "alice", "did:example:agent");

        struct EnvGuard {
            key: &'static str,
            previous: Option<std::ffi::OsString>,
        }
        impl EnvGuard {
            fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
                let previous = std::env::var_os(key);
                std::env::set_var(key, value);
                Self { key, previous }
            }
        }
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                if let Some(previous) = self.previous.as_ref() {
                    std::env::set_var(self.key, previous);
                } else {
                    std::env::remove_var(self.key);
                }
            }
        }

        struct CwdGuard(std::path::PathBuf);
        impl CwdGuard {
            fn set(path: &Path) -> Self {
                let previous = std::env::current_dir().expect("read current dir");
                std::env::set_current_dir(path).expect("set current dir");
                Self(previous)
            }
        }
        impl Drop for CwdGuard {
            fn drop(&mut self) {
                let _ = std::env::set_current_dir(&self.0);
            }
        }

        let session_id = format!("unchanged-session-{}", now_ms());
        let _g1 = EnvGuard::set(agent_tool::OPENDAN_AGENT_ROOT_ENV, &root);
        let _g2 = EnvGuard::set(agent_tool::OPENDAN_SESSION_ID_ENV, &session_id);
        let _g3 = EnvGuard::set(agent_tool::OPENDAN_TRACE_ID_ENV, "trace-unchanged");
        let _cwd = CwdGuard::set(&cwd);

        let args = vec![
            OsString::from("/tmp/agent_tool"),
            OsString::from("read"),
            OsString::from("demo.txt"),
            OsString::from("--content-only"),
        ];
        let first = execute(args.clone(), CliRuntimeEnv::from_process().unwrap(), None)
            .await
            .expect("first read");
        assert_eq!(first.exit_code, EXIT_SUCCESS);
        let first_payload: Json = serde_json::from_str(&first.stdout).expect("parse first json");
        assert_eq!(first_payload["detail"]["unchanged"], false);
        assert_eq!(first_payload["output"], "line-1\nline-2\n");

        let second = execute(args, CliRuntimeEnv::from_process().unwrap(), None)
            .await
            .expect("second read");
        assert_eq!(second.exit_code, EXIT_SUCCESS);
        let second_payload: Json = serde_json::from_str(&second.stdout).expect("parse second json");
        assert_eq!(second_payload["detail"]["unchanged"], true);
        assert_eq!(second_payload["output"], "和上一次read相比没有变化");
    }

    #[tokio::test]
    async fn object_x_call_cli_uses_route_config_and_local_http_adapter() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind local adapter");
        let endpoint = format!("http://{}", listener.local_addr().expect("local addr"));
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept request");
            let (headers, request) = read_json_http_request(&mut stream).await;
            assert!(headers.starts_with("POST /adapter/x-call "));
            assert_eq!(request["object"], "obj://demo/item");
            assert_eq!(request["action"], "reserve");
            assert_eq!(request["params"]["qty"], 2);
            assert_eq!(request["params"]["dry_run"], true);
            let body = r#"{"agent_tool_protocol":"1","status":"success","detail":{"reserved":true},"output":"ok"}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream
                .write_all(response.as_bytes())
                .await
                .expect("write response");
        });

        let config_path = temp.path().join("routes.toml");
        fs::write(
            &config_path,
            format!(
                r#"
version = 1

[[adapters]]
id = "local"
type = "local_http"
endpoint = "{endpoint}"

[[routes]]
id = "obj-local"
match_type = "scheme"
pattern = "obj"
adapter = "local"
methods = ["x_call"]
"#
            ),
        )
        .await
        .expect("write route config");

        let output = execute(
            vec![
                OsString::from("/tmp/agent_tool"),
                OsString::from("x-call"),
                OsString::from("--config"),
                OsString::from(config_path),
                OsString::from("obj://demo/item"),
                OsString::from("reserve"),
                OsString::from("--qty=2"),
                OsString::from("--dry_run=true"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run object x-call");
        server.await.expect("server task");

        assert_eq!(output.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["tool"], "x-call");
        assert_eq!(payload["cmd_name"], "x_call");
        assert_eq!(payload["detail"]["reserved"], true);
        assert_eq!(payload["output"], "ok");
    }

    #[tokio::test]
    async fn write_and_edit_commands_update_file() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let write_output = execute(
            vec![
                OsString::from(MAIN_BINARY_NAME),
                OsString::from("write_file"),
                OsString::from("notes.txt"),
                OsString::from("--mode"),
                OsString::from("write"),
                OsString::from("--content-stdin"),
            ],
            test_env(root.clone(), cwd.clone()),
            Some("hello world\n".to_string()),
        )
        .await
        .expect("run write_file");
        assert_eq!(write_output.exit_code, EXIT_SUCCESS);

        let edit_output = execute(
            vec![
                OsString::from("/tmp/edit_file"),
                OsString::from("notes.txt"),
                OsString::from("--old-string"),
                OsString::from("world"),
                OsString::from("--new-string"),
                OsString::from("buckyos"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run edit_file");
        assert_eq!(edit_output.exit_code, EXIT_SUCCESS);

        let content = fs::read_to_string(cwd.join("notes.txt"))
            .await
            .expect("read updated file");
        assert_eq!(content, "hello buckyos\n");
    }

    #[tokio::test]
    async fn attention_stage1_cli_flow_reads_discovers_completes_and_commits() {
        let _guard = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd).await.expect("create cwd");
        let target_session = "target-session";
        seed_opendan_session_meta(&root, target_session, SessionKind::Ui).await;
        seed_round_history(
            &root,
            target_session,
            "Project Atlas is blocked on DNS verification.",
        )
        .await;

        let env = test_env(root.clone(), cwd.clone());
        let read_output = execute(
            vec![
                OsString::from("/tmp/read_session_history"),
                OsString::from(format!("session_id={target_session}")),
                OsString::from("already_improved"),
                OsString::from("token_limit=4096"),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("read session history");
        assert_eq!(read_output.exit_code, EXIT_SUCCESS);
        let read_payload: Json = serde_json::from_str(&read_output.stdout).expect("read json");
        assert_eq!(read_payload["detail"]["returned"], 1);
        assert_eq!(read_payload["detail"]["commit_round_index"], 1);
        let message_ts = read_payload["detail"]["messages"][0]["ts"]
            .as_str()
            .expect("message ts")
            .to_string();

        let begin_args = json!({
            "session_id": target_session,
            "window_start": message_ts,
            "window_end": message_ts,
        });
        let begin_output = execute(
            vec![
                OsString::from("/tmp/BeginAttentionSignalExtraction"),
                OsString::from(begin_args.to_string()),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("begin extraction");
        assert_eq!(begin_output.exit_code, EXIT_SUCCESS);

        let discover_args = json!({
            "object": {
                "mention_text": "Project Atlas",
                "entity_type": "project",
                "is_user_private_entity": true
            },
            "observation": "Project Atlas is blocked on DNS verification.",
            "observation_type": "status",
            "evidence": [{
                "round_index": 1,
                "entry_seq": 1,
                "entry_kind": "message",
                "role": "user",
                "text_excerpt": "Project Atlas is blocked on DNS verification."
            }],
            "confidence": 0.92
        });
        let discover_output = execute(
            vec![
                OsString::from("/tmp/DiscoverObjectObservation"),
                OsString::from(discover_args.to_string()),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("discover object observation");
        assert_eq!(discover_output.exit_code, EXIT_SUCCESS);
        let discover_payload: Json =
            serde_json::from_str(&discover_output.stdout).expect("discover json");
        let signal_id = discover_payload["detail"]["signal"]["id"]
            .as_str()
            .expect("signal id")
            .to_string();
        assert_eq!(
            discover_payload["detail"]["signal"]["source"]["session_id"],
            target_session
        );
        assert_eq!(
            discover_payload["detail"]["signal"]["extraction"]["extraction_window_id"],
            begin_output_json(&begin_output)["detail"]["extraction_window_id"]
        );

        let complete_output = execute(
            vec![OsString::from("/tmp/CompleteAttentionSignalExtraction")],
            env.clone(),
            None,
        )
        .await
        .expect("complete extraction");
        assert_eq!(complete_output.exit_code, EXIT_SUCCESS);

        let commit_output = execute(
            vec![
                OsString::from("/tmp/commit_session_history_improved"),
                OsString::from(format!("session_id={target_session}")),
                OsString::from("round_index=1"),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("commit history progress");
        assert_eq!(commit_output.exit_code, EXIT_SUCCESS);
        let committed_meta = load_session_meta(&root.join("sessions").join(target_session))
            .await
            .expect("load meta")
            .expect("meta exists");
        assert_eq!(committed_meta.already_improved.committed_round_index, 1);

        let list_output = execute(
            vec![
                OsString::from("/tmp/ListPendingAttentionSignals"),
                OsString::from("limit=10"),
            ],
            env,
            None,
        )
        .await
        .expect("list pending signals");
        assert_eq!(list_output.exit_code, EXIT_SUCCESS);
        let list_payload: Json = serde_json::from_str(&list_output.stdout).expect("list json");
        assert_eq!(list_payload["detail"]["returned"], 1);
        assert_eq!(list_payload["detail"]["signals"][0]["id"], signal_id);
    }

    #[tokio::test]
    async fn attention_stage2_cli_lists_and_marks_consumed() {
        let _guard = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd).await.expect("create cwd");
        let target_session = "target-session";
        seed_opendan_session_meta(&root, target_session, SessionKind::Ui).await;
        let env = test_env(root.clone(), cwd);

        let begin_args = json!({
            "session_id": target_session,
            "window_start": "2026-01-01T00:00:00Z",
            "window_end": "2026-01-01T00:01:00Z",
        });
        execute(
            vec![
                OsString::from("/tmp/BeginAttentionSignalExtraction"),
                OsString::from(begin_args.to_string()),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("begin extraction");
        let discover_args = json!({
            "title": "Project Atlas DNS verification blocked",
            "phase": "blocked",
            "evidence": [{
                "round_index": 1,
                "entry_seq": 1,
                "entry_kind": "message",
                "role": "user",
                "text_excerpt": "Project Atlas is blocked on DNS verification."
            }],
            "confidence": 0.9
        });
        let discover_output = execute(
            vec![
                OsString::from("/tmp/DiscoverEvent"),
                OsString::from(discover_args.to_string()),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("discover event");
        let signal_id = serde_json::from_str::<Json>(&discover_output.stdout)
            .expect("discover json")["detail"]["signal"]["id"]
            .as_str()
            .expect("signal id")
            .to_string();

        let list_output = execute(
            vec![OsString::from("/tmp/ListPendingAttentionSignals")],
            env.clone(),
            None,
        )
        .await
        .expect("list pending");
        let list_payload: Json = serde_json::from_str(&list_output.stdout).expect("list json");
        assert_eq!(list_payload["detail"]["returned"], 1);

        let mark_output = execute(
            vec![
                OsString::from("/tmp/MarkAttentionSignalConsumed"),
                OsString::from(format!("signal_id={signal_id}")),
            ],
            env.clone(),
            None,
        )
        .await
        .expect("mark consumed");
        assert_eq!(mark_output.exit_code, EXIT_SUCCESS);
        let mark_payload: Json = serde_json::from_str(&mark_output.stdout).expect("mark json");
        assert_eq!(
            mark_payload["detail"]["signal"]["lifecycle_status"],
            "consumed"
        );

        let after_output = execute(
            vec![OsString::from("/tmp/ListPendingAttentionSignals")],
            env,
            None,
        )
        .await
        .expect("list after consumed");
        let after_payload: Json = serde_json::from_str(&after_output.stdout).expect("after json");
        assert_eq!(after_payload["detail"]["returned"], 0);
    }

    fn begin_output_json(output: &CliRunOutput) -> Json {
        serde_json::from_str(&output.stdout).expect("begin json")
    }

    #[tokio::test]
    async fn generic_help_lists_all_cli_tools() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let output = execute(
            vec![OsString::from(MAIN_BINARY_NAME), OsString::from("--help")],
            test_env(root.clone(), root),
            None,
        )
        .await
        .expect("run help");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse help json");
        assert_eq!(payload["status"], "success");
        assert_eq!(
            payload["detail"]["tools"].as_array().map(|v| v.len()),
            Some(TOOL_NAMES.len())
        );
    }

    #[tokio::test]
    async fn todo_cli_writes_session_todos_under_agent_env() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let add_output = execute(
            vec![
                OsString::from("/tmp/todo"),
                OsString::from("add"),
                OsString::from("first task"),
                OsString::from("--content"),
                OsString::from("task body"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run todo add");
        assert_eq!(add_output.exit_code, EXIT_SUCCESS);

        let todos_path = root
            .join("sessions")
            .join("session-test")
            .join("todos.json");
        let todos: Json = serde_json::from_str(
            &fs::read_to_string(&todos_path)
                .await
                .expect("read todos.json"),
        )
        .expect("parse todos");
        assert_eq!(todos[0]["todo_id"], "T01");
        assert_eq!(todos[0]["session_id"], "session-test");
        assert_eq!(todos[0]["title"], "first task");

        let current_output = execute(
            vec![
                OsString::from(MAIN_BINARY_NAME),
                OsString::from("todo"),
                OsString::from("current"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run todo current");
        assert_eq!(current_output.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(&current_output.stdout).expect("parse json");
        assert_eq!(payload["detail"]["todo"]["todo_id"], "T01");
    }

    #[tokio::test]
    async fn agent_memory_set_get_remove_roundtrip() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        // set
        let set_output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("set"),
                OsString::from("/user/preference/style"),
                OsString::from("concise english"),
                OsString::from("--reason"),
                OsString::from("user conversation;c=1"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run agent-memory set");
        assert_eq!(set_output.exit_code, EXIT_SUCCESS);

        let item_dir = root.join("memory").join("item");
        assert!(
            fs::metadata(&item_dir).await.is_ok(),
            "free hints are materialized as canonical item JSON files"
        );

        // get echoes content directly (no envelope, per §4.5)
        let get_output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("get"),
                OsString::from("/user/preference/style"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run agent-memory get");
        assert_eq!(get_output.exit_code, EXIT_SUCCESS);
        assert_eq!(get_output.stdout, "concise english");

        // remove
        let remove_output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("remove"),
                OsString::from("/user/preference/style"),
                OsString::from("--reason"),
                OsString::from("user removed"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run agent-memory remove");
        assert_eq!(remove_output.exit_code, EXIT_SUCCESS);

        // get after remove → exit 1
        let get_after = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("get"),
                OsString::from("/user/preference/style"),
            ],
            test_env(root.clone(), cwd),
            None,
        )
        .await
        .expect("run agent-memory get-after-remove");
        assert_eq!(get_after.exit_code, 1);
    }

    #[tokio::test]
    async fn agent_memory_set_form_b_reads_stdin() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let body = "Importance: 3\nExpired-At: 2030-01-01T00:00:00Z\n\nbody text";
        let output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("set"),
                OsString::from("/user/note"),
                OsString::from("--reason"),
                OsString::from("user conversation;c=1"),
            ],
            test_env(root.clone(), cwd),
            Some(body.to_string()),
        )
        .await
        .expect("run agent-memory set form B");
        assert_eq!(output.exit_code, EXIT_SUCCESS);

        let get_output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("get"),
                OsString::from("/user/note"),
            ],
            test_env(root.clone(), root.join("workspace")),
            None,
        )
        .await
        .expect("get stored stdin content");
        assert_eq!(get_output.exit_code, EXIT_SUCCESS);
        assert_eq!(get_output.stdout, body);
    }

    #[tokio::test]
    async fn agent_memory_load_emits_size_prefixed_records() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("set"),
                OsString::from("/user/dental"),
                OsString::from("Dental followup at 10am"),
                OsString::from("--reason"),
                OsString::from("user conversation;c=1"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("seed");

        let load_output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("load"),
                OsString::from("dental"),
            ],
            test_env(root.clone(), cwd),
            None,
        )
        .await
        .expect("run agent-memory load");
        assert_eq!(load_output.exit_code, EXIT_SUCCESS);
        assert!(load_output.stdout.contains("ITEM item_"));
        assert!(load_output.stdout.contains("KIND free\n"));
        assert!(load_output.stdout.contains("---\n"));
        assert!(load_output.stdout.contains("\nEND\n"));
        assert!(load_output.stdout.contains("MATCHED tag:dental"));
    }

    #[tokio::test]
    async fn agent_memory_list_returns_keys_per_line() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        for k in ["/user/a", "/user/b", "/kb/c"] {
            execute(
                vec![
                    OsString::from("/tmp/agent-memory"),
                    OsString::from("set"),
                    OsString::from(k),
                    OsString::from("x"),
                    OsString::from("--reason"),
                    OsString::from("r"),
                ],
                test_env(root.clone(), cwd.clone()),
                None,
            )
            .await
            .expect("seed");
        }

        let output = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("list"),
                OsString::from("/user/"),
            ],
            test_env(root.clone(), cwd),
            None,
        )
        .await
        .expect("run agent-memory list");
        assert_eq!(output.exit_code, EXIT_SUCCESS);
        assert_eq!(output.stdout, "/user/a\n/user/b\n");
    }

    #[tokio::test]
    async fn agent_memory_set_missing_reason_returns_invalid_args() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let result = execute(
            vec![
                OsString::from("/tmp/agent-memory"),
                OsString::from("set"),
                OsString::from("/user/k"),
                OsString::from("v"),
            ],
            dev_test_env(root, cwd),
            None,
        )
        .await;
        let err = result.expect_err("set without --reason must fail at parse");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[test]
    fn agent_memory_load_parser_splits_tags_and_flags() {
        let parsed = parse_agent_memory_cli_command(
            "agent-memory".into(),
            &[
                "load".into(),
                "dental,phone case,reminder".into(),
                "--max-records".into(),
                "10".into(),
                "--max-bytes=4096".into(),
            ],
        )
        .expect("parse load");
        match parsed {
            ParsedCommand::AgentMemory {
                invocation:
                    AgentMemoryInvocation {
                        verb:
                            AgentMemoryVerb::Load {
                                tags,
                                max_records,
                                max_bytes,
                                ..
                            },
                        ..
                    },
                ..
            } => {
                assert_eq!(tags, vec!["dental", "phone case", "reminder"]);
                assert_eq!(max_records, Some(10));
                assert_eq!(max_bytes, Some(4096));
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn agent_memory_root_override_resolves_relative_to_cwd() {
        let parsed = parse_agent_memory_cli_command(
            "agent-memory".into(),
            &["--root".into(), "/tmp/custom-root".into(), "init".into()],
        )
        .expect("parse init with --root");
        match parsed {
            ParsedCommand::AgentMemory {
                invocation:
                    AgentMemoryInvocation {
                        root_override,
                        verb: AgentMemoryVerb::Init,
                        ..
                    },
                ..
            } => {
                assert_eq!(root_override, Some(PathBuf::from("/tmp/custom-root")));
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn parse_check_task_alias_accepts_positional_task_id() {
        let parsed = parse_command(
            &[OsString::from("/tmp/check_task"), OsString::from("42")],
            Path::new("/tmp"),
        )
        .expect("parse check_task");

        match parsed {
            ParsedCommand::CheckTask { tool_name, task_id } => {
                assert_eq!(tool_name, TOOL_CHECK_TASK);
                assert_eq!(task_id, "42");
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn parse_cancel_task_subcommand_accepts_recursive_flag() {
        let parsed = parse_command(
            &[
                OsString::from(MAIN_BINARY_NAME),
                OsString::from(TOOL_CANCEL_TASK),
                OsString::from("--recursive"),
                OsString::from("task_id=7"),
            ],
            Path::new("/tmp"),
        )
        .expect("parse cancel_task");

        match parsed {
            ParsedCommand::CancelTask {
                tool_name,
                task_id,
                recursive,
            } => {
                assert_eq!(tool_name, TOOL_CANCEL_TASK);
                assert_eq!(task_id, "7");
                assert!(recursive);
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn parse_finish_task_subcommand_accepts_task_id() {
        let parsed = parse_command(
            &[
                OsString::from(MAIN_BINARY_NAME),
                OsString::from(TOOL_FINISH_TASK),
                OsString::from("task_id=9"),
            ],
            Path::new("/tmp"),
        )
        .expect("parse finish_task");

        match parsed {
            ParsedCommand::FinishTask {
                tool_name,
                task_id,
                outcome,
                message,
            } => {
                assert_eq!(tool_name, TOOL_FINISH_TASK);
                assert_eq!(task_id, "9");
                assert_eq!(outcome, FinishTaskOutcome::Completed);
                assert_eq!(message, None);
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[test]
    fn parse_finish_task_failed_accepts_message() {
        let parsed = parse_command(
            &[
                OsString::from(MAIN_BINARY_NAME),
                OsString::from(TOOL_FINISH_TASK),
                OsString::from("9"),
                OsString::from("failed"),
                OsString::from("--message"),
                OsString::from("cannot route task"),
            ],
            Path::new("/tmp"),
        )
        .expect("parse finish_task failed");

        match parsed {
            ParsedCommand::FinishTask {
                tool_name,
                task_id,
                outcome,
                message,
            } => {
                assert_eq!(tool_name, TOOL_FINISH_TASK);
                assert_eq!(task_id, "9");
                assert_eq!(outcome, FinishTaskOutcome::Failed);
                assert_eq!(message.as_deref(), Some("cannot route task"));
            }
            other => panic!("unexpected parsed command: {other:?}"),
        }
    }

    #[tokio::test]
    async fn read_file_without_agent_env_has_no_scope_limit() {
        let temp = tempdir().expect("create tempdir");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside)
            .await
            .expect("create outside dir");
        fs::write(outside.join("demo.txt"), "free\n")
            .await
            .expect("write outside file");

        let output = execute(
            vec![
                OsString::from("/tmp/read_file"),
                OsString::from(outside.join("demo.txt")),
            ],
            dev_test_env(temp.path().join("cwd"), temp.path().join("cwd")),
            None,
        )
        .await
        .expect("run read_file");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["detail"]["content"], "free\n");
    }

    #[tokio::test]
    async fn read_file_with_agent_env_has_no_scope_limit() {
        let temp = tempdir().expect("create tempdir");
        let agent_root = temp.path().join("agent");
        let cwd = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&agent_root).await.expect("create agent");
        fs::create_dir_all(&cwd).await.expect("create cwd");
        fs::create_dir_all(&outside)
            .await
            .expect("create outside dir");
        fs::write(outside.join("demo.txt"), "free\n")
            .await
            .expect("write outside file");

        let output = execute(
            vec![
                OsString::from("/tmp/read_file"),
                OsString::from(outside.join("demo.txt")),
            ],
            test_env(agent_root, cwd),
            None,
        )
        .await
        .expect("run read_file");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["detail"]["content"], "free\n");
    }

    #[tokio::test]
    async fn glob_with_agent_env_has_no_scope_limit() {
        let temp = tempdir().expect("create tempdir");
        let agent_root = temp.path().join("agent");
        let cwd = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&agent_root).await.expect("create agent");
        fs::create_dir_all(&cwd).await.expect("create cwd");
        fs::create_dir_all(&outside)
            .await
            .expect("create outside dir");
        fs::write(outside.join("demo.txt"), "free\n")
            .await
            .expect("write outside file");

        let output = execute(
            vec![
                OsString::from("/tmp/Glob"),
                OsString::from("pattern=*.txt"),
                OsString::from(format!("path={}", outside.display())),
            ],
            test_env(agent_root, cwd),
            None,
        )
        .await
        .expect("run Glob");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["detail"]["numFiles"], 1);
    }

    #[tokio::test]
    async fn grep_with_agent_env_has_no_scope_limit() {
        let temp = tempdir().expect("create tempdir");
        let agent_root = temp.path().join("agent");
        let cwd = temp.path().join("workspace");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&agent_root).await.expect("create agent");
        fs::create_dir_all(&cwd).await.expect("create cwd");
        fs::create_dir_all(&outside)
            .await
            .expect("create outside dir");
        fs::write(outside.join("demo.txt"), "free\n")
            .await
            .expect("write outside file");

        let output = execute(
            vec![
                OsString::from("/tmp/Grep"),
                OsString::from("pattern=free"),
                OsString::from(format!("path={}", outside.display())),
            ],
            test_env(agent_root, cwd),
            None,
        )
        .await
        .expect("run Grep");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["detail"]["numFiles"], 1);
    }

    #[tokio::test]
    async fn read_file_without_agent_env_and_without_tty_returns_plain_text() {
        let temp = tempdir().expect("create tempdir");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&outside)
            .await
            .expect("create outside dir");
        fs::write(outside.join("demo.txt"), "free\n")
            .await
            .expect("write outside file");

        let output = execute(
            vec![
                OsString::from("/tmp/read_file"),
                OsString::from(outside.join("demo.txt")),
            ],
            {
                let mut env = dev_test_env(temp.path().join("cwd"), temp.path().join("cwd"));
                env.stdout_is_terminal = false;
                env
            },
            None,
        )
        .await
        .expect("run read_file");

        assert_eq!(output.exit_code, EXIT_SUCCESS);
        assert_eq!(output.stdout, "free\n");
        assert!(output.stderr.is_empty());
    }

    #[tokio::test]
    async fn command_not_found_proxy_returns_127_for_unknown_command() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");

        let output = execute(
            vec![
                OsString::from(MAIN_BINARY_NAME),
                OsString::from(COMMAND_NOT_FOUND_PROXY),
                OsString::from("missing_tool"),
            ],
            test_env(root.clone(), root),
            None,
        )
        .await
        .expect("run command_not_found proxy");

        // The dispatcher now delegates to `llm_tool_carft::run_subcommand`.
        // Until step 1 reads behavior cfg, every call falls through with
        // exit 127 + a structured AgentToolResult on stdout (stderr stays
        // empty — render_cli_output puts the envelope on stdout). The shell
        // hook's own `printf 'bash: %s: command not found\n'` is responsible
        // for re-emitting the canonical error to stderr, not this CLI.
        assert_eq!(output.exit_code, agent_tool::CLI_EXIT_COMMAND_NOT_FOUND);
        assert!(output.stderr.is_empty());
        assert!(output.stdout.contains("llm_tool_carft"));
        assert!(output.stdout.contains("missing_tool"));
        assert!(output.stdout.contains("skipped"));
    }

    #[tokio::test]
    async fn create_workspace_and_get_session_aliases_share_local_state() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        seed_session(&root, "session-test", &cwd).await;

        let create_output = execute(
            vec![
                OsString::from("/tmp/create_workspace"),
                OsString::from("demo"),
                OsString::from("workspace summary"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run create_workspace");
        let create_payload: Json =
            serde_json::from_str(&create_output.stdout).expect("parse create json");
        assert_eq!(create_payload["status"], "success");
        let workspace_id = create_payload["detail"]["workspace"]["workspace_id"]
            .as_str()
            .expect("workspace id");

        let session_output = execute(
            vec![OsString::from("/tmp/get_session")],
            test_env(root.clone(), cwd),
            None,
        )
        .await
        .expect("run get_session");
        let session_payload: Json =
            serde_json::from_str(&session_output.stdout).expect("parse session json");
        assert_eq!(session_payload["status"], "success");
        assert_eq!(
            session_payload["detail"]["session"]["local_workspace_id"],
            workspace_id
        );
    }

    #[tokio::test]
    async fn create_workspace_alias_uses_title_for_workspace_dir() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        seed_session(&root, "session-test", &cwd).await;

        let output = execute(
            vec![
                OsString::from("/tmp/create_workspace"),
                OsString::from("My Workspace"),
                OsString::from("workspace summary"),
            ],
            test_env(root.clone(), cwd),
            None,
        )
        .await
        .expect("run create_workspace");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse create json");
        assert_eq!(payload["status"], "success");
        assert_eq!(
            payload["detail"]["binding"]["workspace_rel_path"],
            "workspaces/My Workspace"
        );
        let workspace_path = payload["detail"]["binding"]["workspace_path"]
            .as_str()
            .expect("workspace path");
        assert!(workspace_path.ends_with("workspaces/My Workspace"));
        assert!(!workspace_path
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .starts_with("ws-"));
    }

    // ----------------------------- agent-notebook CLI tests

    #[tokio::test]
    async fn agent_notebook_append_then_list_and_read() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        // Append (auto-creates notebook).
        let append_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("concise replies"),
                OsString::from("user prefers terse output"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--confidence"),
                OsString::from("high"),
                OsString::from("--tags"),
                OsString::from("reply-style,tone"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run agent-notebook append");
        assert_eq!(append_output.exit_code, EXIT_SUCCESS);
        let append_payload: Json =
            serde_json::from_str(append_output.stdout.trim()).expect("parse append json");
        assert_eq!(append_payload["status"], "ok");
        assert_eq!(append_payload["notebook_id"], "user/preferences");
        let item_id = append_payload["item_id"]
            .as_str()
            .expect("item_id string")
            .to_string();

        // List should now show the notebook.
        let list_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("list"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run agent-notebook list");
        assert_eq!(list_output.exit_code, EXIT_SUCCESS);
        let list_payload: Json =
            serde_json::from_str(list_output.stdout.trim()).expect("parse list json");
        assert_eq!(list_payload["status"], "ok");
        let notebooks = list_payload["notebooks"]
            .as_array()
            .expect("notebooks array");
        assert_eq!(notebooks.len(), 1);
        assert_eq!(notebooks[0]["id"], "user/preferences");
        assert_eq!(notebooks[0]["active_entry_count"], 1);

        // Read by tags returns the item we just appended.
        let read_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("read"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("reply-style"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run agent-notebook read");
        assert_eq!(read_output.exit_code, EXIT_SUCCESS);
        let read_payload: Json =
            serde_json::from_str(read_output.stdout.trim()).expect("parse read json");
        assert_eq!(read_payload["status"], "ok");
        let entries = read_payload["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["item_id"], item_id);
        assert_eq!(entries[0]["title"], "concise replies");

        // Re-reading the same scope returns `unchanged`.
        let read_again = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("read"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("reply-style"),
            ],
            test_env(root.clone(), cwd),
            None,
        )
        .await
        .expect("run agent-notebook read again");
        assert_eq!(read_again.exit_code, EXIT_SUCCESS);
        let unchanged: Json =
            serde_json::from_str(read_again.stdout.trim()).expect("parse unchanged json");
        assert_eq!(unchanged["status"], "unchanged");
        assert!(unchanged.get("entries").is_none());
    }

    #[tokio::test]
    async fn agent_notebook_append_stdin_reads_content_from_stdin() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let body = "long body line 1\nlong body line 2\n";
        let output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("design notes"),
                OsString::from("--stdin"),
                OsString::from("--bookid"),
                OsString::from("projects/demo"),
                OsString::from("--tags"),
                OsString::from("design,notes"),
            ],
            test_env(root.clone(), cwd),
            Some(body.to_string()),
        )
        .await
        .expect("run agent-notebook append --stdin");
        assert_eq!(output.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(output.stdout.trim()).expect("parse append json");
        assert_eq!(payload["status"], "ok");
    }

    #[tokio::test]
    async fn agent_notebook_remarks_roundtrip() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let seed = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("fact"),
                OsString::from("body"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("fact"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("seed notebook item");
        assert_eq!(seed.exit_code, EXIT_SUCCESS);
        let seed_payload: Json = serde_json::from_str(seed.stdout.trim()).expect("parse seed json");
        let item_id = seed_payload["item_id"]
            .as_str()
            .expect("item_id string")
            .to_string();

        let append = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("remarks"),
                OsString::from("append"),
                OsString::from(item_id.clone()),
                OsString::from("red"),
                OsString::from("needs confirmation"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("append remark");
        assert_eq!(append.exit_code, EXIT_SUCCESS);
        let append_payload: Json =
            serde_json::from_str(append.stdout.trim()).expect("parse remark append json");
        assert_eq!(append_payload["status"], "ok");
        let remark_id = append_payload["remark_id"]
            .as_str()
            .expect("remark_id string")
            .to_string();

        let list = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("remarks"),
                OsString::from("list"),
                OsString::from(item_id.clone()),
                OsString::from("--type"),
                OsString::from("red"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("list remarks");
        assert_eq!(list.exit_code, EXIT_SUCCESS);
        let list_payload: Json =
            serde_json::from_str(list.stdout.trim()).expect("parse remark list json");
        let remarks = list_payload["remarks"].as_array().expect("remarks array");
        assert_eq!(remarks.len(), 1);
        assert_eq!(remarks[0]["remark_id"], remark_id);
        assert_eq!(remarks[0]["content"], "needs confirmation");

        let remove = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("remarks"),
                OsString::from("remove"),
                OsString::from(item_id.clone()),
                OsString::from(remark_id),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("remove remark");
        assert_eq!(remove.exit_code, EXIT_SUCCESS);

        let after = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("remarks"),
                OsString::from("list"),
                OsString::from(item_id),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("list remarks after remove");
        assert_eq!(after.exit_code, EXIT_SUCCESS);
        let after_payload: Json =
            serde_json::from_str(after.stdout.trim()).expect("parse remark list after remove");
        let remarks = after_payload["remarks"].as_array().expect("remarks array");
        assert!(remarks.is_empty());
    }

    #[tokio::test]
    async fn agent_notebook_read_and_append_default_to_owner_action_notebook() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let append_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("Tokyo lunch"),
                OsString::from("Lunch with Lucy in Tokyo."),
                OsString::from("--tags"),
                OsString::from("travel,appointment"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run default append");
        assert_eq!(append_output.exit_code, EXIT_SUCCESS);
        let append_payload: Json =
            serde_json::from_str(append_output.stdout.trim()).expect("parse append json");
        assert_eq!(append_payload["status"], "ok");
        assert_eq!(append_payload["notebook_id"], DEFAULT_AGENT_NOTEBOOK_ID);

        let read_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("read"),
                OsString::from("--title"),
                OsString::from("Tokyo lunch"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run default read");
        assert_eq!(read_output.exit_code, EXIT_SUCCESS);
        let read_payload: Json =
            serde_json::from_str(read_output.stdout.trim()).expect("parse read json");
        assert_eq!(read_payload["status"], "ok");
        assert_eq!(read_payload["notebook_id"], DEFAULT_AGENT_NOTEBOOK_ID);
        let entries = read_payload["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["title"], "Tokyo lunch");
    }

    #[tokio::test]
    async fn agent_notebook_create_notebook_requires_bookid_flag() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let missing_bookid = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("create-notebook"),
                OsString::from("--title"),
                OsString::from("Project Demo"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect_err("create-notebook without bookid should fail during parse");
        assert!(missing_bookid
            .to_string()
            .contains("`create-notebook` requires `--bookid`"));

        let old_id = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("create-notebook"),
                OsString::from("--id"),
                OsString::from("projects/demo"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect_err("old --id flag should fail during parse");
        assert!(old_id
            .to_string()
            .contains("unsupported flag `--id` for `create-notebook`"));

        let created = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("create-notebook"),
                OsString::from("--bookid"),
                OsString::from("projects/demo"),
                OsString::from("--kind"),
                OsString::from("project"),
                OsString::from("--title"),
                OsString::from("Project Demo"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run create-notebook with id");
        assert_eq!(created.exit_code, EXIT_SUCCESS);
        let created_payload: Json =
            serde_json::from_str(created.stdout.trim()).expect("parse created json");
        assert_eq!(created_payload["status"], "ok");
        assert_eq!(created_payload["notebook"]["id"], "projects/demo");
        assert_eq!(created_payload["created"], true);
    }

    #[tokio::test]
    async fn agent_notebook_owner_user_defaults_to_agent_appid() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        seed_agent_identity(&root, "alice", "did:opendan:test");

        let output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("list"),
            ],
            dev_test_env(root, cwd),
            None,
        )
        .await
        .expect("run agent-notebook list with agent identity");
        assert_eq!(output.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(output.stdout.trim()).expect("parse list json");
        assert_eq!(payload["status"], "ok");
        assert_eq!(
            payload["notebooks"]
                .as_array()
                .expect("notebooks array")
                .len(),
            0
        );
    }

    #[tokio::test]
    async fn agent_notebook_status_marks_item_stale() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        // Seed an item.
        let seed = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("old fact"),
                OsString::from("a stale fact"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("fact"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("seed item");
        assert_eq!(seed.exit_code, EXIT_SUCCESS);
        let seed_payload: Json = serde_json::from_str(seed.stdout.trim()).expect("parse seed json");
        let item_id = seed_payload["item_id"]
            .as_str()
            .expect("item_id")
            .to_string();

        // Mark stale.
        let status_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("status"),
                OsString::from(item_id.clone()),
                OsString::from("stale"),
                OsString::from("--reason"),
                OsString::from("no longer applies"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run status");
        assert_eq!(status_output.exit_code, EXIT_SUCCESS);
        let payload: Json =
            serde_json::from_str(status_output.stdout.trim()).expect("parse status json");
        assert_eq!(payload["status"], "ok");

        // Default read (active only) returns no entries.
        let read_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("read"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run read after stale");
        assert_eq!(read_output.exit_code, EXIT_SUCCESS);
        let read_payload: Json =
            serde_json::from_str(read_output.stdout.trim()).expect("parse read json");
        assert_eq!(read_payload["status"], "ok");
        let entries = read_payload["entries"].as_array().expect("entries");
        assert!(entries.is_empty());
    }

    #[tokio::test]
    async fn agent_notebook_invalid_tag_returns_structured_error() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        let output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("bad"),
                OsString::from("x"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("bad\"tag"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run agent-notebook append with bad tag");
        assert_eq!(output.exit_code, 2);
        let payload: Json = serde_json::from_str(output.stdout.trim()).expect("parse error json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "invalid_tag");
    }

    #[tokio::test]
    async fn agent_notebook_identity_and_root_override_resolve_context() {
        // Default notebook scope belongs to the agent appid; --root comes
        // from the dev override.
        // Env vars are process-global, so hold ENV_TEST_LOCK to keep other
        // notebook tests from seeing them.
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let nb_root = temp.path().join("nb-root");
        let agent_root = temp.path().join("agent-env");
        let cwd = temp.path().join("cwd");
        fs::create_dir_all(&cwd).await.expect("create cwd");
        seed_agent_identity(&agent_root, "alice", "did:opendan:test");

        struct EnvGuard(&'static str);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                std::env::remove_var(self.0);
            }
        }
        std::env::set_var(AGENT_NOTEBOOK_ROOT_ENV, &nb_root);
        let _g1 = EnvGuard(AGENT_NOTEBOOK_ROOT_ENV);

        // Append with zero CLI flags beyond verb-specific ones.
        let out = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("from env"),
                OsString::from("body via env-resolved scope"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("env-test"),
            ],
            dev_test_env(agent_root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run append via env");
        assert_eq!(out.exit_code, EXIT_SUCCESS, "stdout={:?}", out.stdout);
        assert!(nb_root.join("notebook.sqlite").exists());

        // Read also picks the identity/root up.
        let read = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("read"),
                OsString::from("--bookid"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("env-test"),
            ],
            dev_test_env(agent_root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run read via env");
        assert_eq!(read.exit_code, EXIT_SUCCESS);
        let payload: Json = serde_json::from_str(read.stdout.trim()).expect("parse env read json");
        assert_eq!(payload["status"], "ok");
        let entries = payload["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["title"], "from env");

        let list = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("list"),
            ],
            dev_test_env(agent_root, cwd),
            None,
        )
        .await
        .expect("run list via env");
        assert_eq!(list.exit_code, EXIT_SUCCESS);
        let list_payload: Json =
            serde_json::from_str(list.stdout.trim()).expect("parse env list json");
        let notebooks = list_payload["notebooks"]
            .as_array()
            .expect("notebooks array");
        assert_eq!(notebooks.len(), 1);
        assert_eq!(notebooks[0]["id"], "user/preferences");
    }

    #[tokio::test]
    async fn agent_notebook_registry_and_hints_smoke() {
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");

        // Seed two notebooks.
        for (nb_id, title) in [
            ("user/preferences", "tone preference"),
            ("projects/demo", "scope decision"),
        ] {
            let out = execute(
                vec![
                    OsString::from("/tmp/agent-notebook"),
                    OsString::from("append"),
                    OsString::from(title),
                    OsString::from("seed content"),
                    OsString::from("--bookid"),
                    OsString::from(nb_id),
                    OsString::from("--tags"),
                    OsString::from("tone,style"),
                ],
                test_env(root.clone(), cwd.clone()),
                None,
            )
            .await
            .expect("seed notebook");
            assert_eq!(out.exit_code, EXIT_SUCCESS);
        }

        // registry-context returns metadata only.
        let registry_out = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("registry-context"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run registry-context");
        assert_eq!(registry_out.exit_code, EXIT_SUCCESS);
        let payload: Json =
            serde_json::from_str(registry_out.stdout.trim()).expect("parse registry json");
        assert_eq!(payload["status"], "ok");
        let text = payload["registry"]["text"].as_str().expect("registry text");
        assert!(text.contains("user/preferences"));
        assert!(text.contains("projects/demo"));
        // Body content must not leak into registry.
        assert!(!text.contains("seed content"));

        // hints with topic_tags works.
        let hints_out = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("hints"),
                OsString::from("--topic-tags"),
                OsString::from("tone"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run hints");
        assert_eq!(hints_out.exit_code, EXIT_SUCCESS);
        let payload: Json =
            serde_json::from_str(hints_out.stdout.trim()).expect("parse hints json");
        assert_eq!(payload["status"], "ok");
        assert!(payload["hints_block"]["hints"].is_array());
    }
}
