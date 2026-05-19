use std::env;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, init_buckyos_api_runtime, BuckyOSRuntimeType, Task, TaskManagerClient,
    TaskStatus,
};
use kRPC::kRPC;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs;
use tokio::io::{self, AsyncReadExt};
use tokio::process::Command;

use agent_tool::agent_memory::{AgentMemory, AgentMemoryConfig, AgentMemoryError, LoadOptions};
use agent_tool::agent_notebook::{
    self as nb, ActorKind, AgentNotebook, AgentNotebookConfig, AppendNoteInput,
    BuildHintsInput, BuildRegistryContextInput, BuildSystemContextInput, Confidence,
    CreateOrUpdateNotebookInput, ListNotebooksInput, MarkNoteStatusInput, NotebookError,
    NotebookItemStatus, NotebookKind, NotebookReadResult, OwnerScope, PromoteToSystemInput,
    PromoteToSystemResult, ReadNotebookInput, WriteReason,
};
use agent_tool::llm_tool_carft::{self, CommandNotFoundRequest};
use agent_tool::{
    cli_error_result, cli_exit_code_for_error, cli_result_from_tool_result, cli_success_result,
    normalize_abs_path, now_ms, render_cli_output, session_record_path, AgentToolError,
    AgentToolManager, AgentToolPendingReason, AgentToolResult, AgentToolStatus, BindWorkspaceTool,
    CliRunOutput, CreateWorkspaceTool, EditFileTool, FileToolConfig, GetSessionTool, GlobTool,
    GrepTool, NoopFileWriteAudit, ReadFileTool, SessionRuntimeContext, SessionViewBackend,
    WorkspaceToolBackend, WriteFileTool,
};
use agent_tool::{llm_explore, run_local_llm};

const TOOL_CHECK_TASK: &str = "check_task";
const TOOL_CANCEL_TASK: &str = "cancel_task";
const TOOL_AGENT_MEMORY: &str = "agent-memory";
const TOOL_AGENT_MEMORY_SNAKE: &str = "agent_memory";
const TOOL_AGENT_NOTEBOOK: &str = "agent-notebook";
const TOOL_AGENT_NOTEBOOK_SNAKE: &str = "agent_notebook";
const TOOL_NAMES: [&str; 14] = [
    "Glob",
    "Grep",
    "read_file",
    "write_file",
    "edit_file",
    "get_session",
    "create_workspace",
    "bind_workspace",
    TOOL_AGENT_MEMORY,
    TOOL_AGENT_MEMORY_SNAKE,
    TOOL_AGENT_NOTEBOOK,
    TOOL_AGENT_NOTEBOOK_SNAKE,
    TOOL_CHECK_TASK,
    TOOL_CANCEL_TASK,
];
const AGENT_MEMORY_ROOT_ENV: &str = "AGENT_MEMORY_ROOT";
const AGENT_MEMORY_DIR_NAME: &str = "memory";
const AGENT_NOTEBOOK_ROOT_ENV: &str = "AGENT_NOTEBOOK_ROOT";
const AGENT_NOTEBOOK_DIR_NAME: &str = "notebook";
/// Fallback env when `--owner-user` is not passed. New name; no existing
/// buckyos env for "the user this agent serves".
const DEFAULT_OWNER_USER_ENV: &str = "OPENDAN_OWNER_USER_ID";
/// Fallback env when `--owner-agent` is not passed. Reuses the existing
/// harness env (the agent's own DID == the owner_agent for its own
/// notebooks). Kept aligned with `CliRuntimeEnv::call_ctx.agent_name`.
const DEFAULT_OWNER_AGENT_ENV: &str = "OPENDAN_AGENT_ID";
/// Fallback env when `--session` is not passed. The harness already sets
/// this for in-session invocations; ad-hoc shells can export it manually.
const DEFAULT_SESSION_ENV: &str = "OPENDAN_SESSION_ID";
const EXIT_SUCCESS: i32 = agent_tool::CLI_EXIT_SUCCESS;
const COMMAND_NOT_FOUND_PROXY: &str = agent_tool::CLI_COMMAND_NOT_FOUND_SUBCOMMAND;
const MAIN_BINARY_NAME: &str = "agent_tool";
const DEFAULT_AGENT_NAME: &str = "did:opendan:cli";
const DEFAULT_TRACE_ID: &str = "cli-trace";
const DEFAULT_SESSION_ID: &str = "cli-session";
const DEFAULT_WAKEUP_ID: &str = "cli-wakeup";
const DEFAULT_BEHAVIOR: &str = "cli";
const SESSION_RECORD_FILE: &str = "session.json";
const SESSION_WORKSPACE_BINDINGS_REL_PATH: &str = "workspaces/session_workspace_bindings.json";
const WORKSPACE_INDEX_FILE: &str = "index.json";

#[derive(Clone, Debug)]
struct CliRuntimeEnv {
    agent_env_root: PathBuf,
    has_agent_env: bool,
    current_dir: PathBuf,
    stdout_is_terminal: bool,
    call_ctx: SessionRuntimeContext,
}

impl CliRuntimeEnv {
    fn from_process() -> Result<Self, AgentToolError> {
        let current_dir = env::current_dir()
            .map(|path| canonicalize_or_normalize(path, None))
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("resolve current dir failed: {err}"))
            })?;
        let agent_env_root = first_path_env(&["OPENDAN_AGENT_ENV"], &current_dir);
        let has_agent_env = agent_env_root.is_some();
        let agent_env_root = agent_env_root.unwrap_or_else(|| current_dir.clone());
        let step_idx = first_string_env(&["OPENDAN_STEP_IDX"])
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(0);

        Ok(Self {
            agent_env_root,
            has_agent_env,
            current_dir,
            stdout_is_terminal: std::io::stdout().is_terminal(),
            call_ctx: SessionRuntimeContext {
                trace_id: first_string_env(&["OPENDAN_TRACE_ID"])
                    .unwrap_or_else(|| DEFAULT_TRACE_ID.to_string()),
                agent_name: first_string_env(&["OPENDAN_AGENT_ID"])
                    .unwrap_or_else(|| DEFAULT_AGENT_NAME.to_string()),
                behavior: first_string_env(&["OPENDAN_BEHAVIOR"])
                    .unwrap_or_else(|| DEFAULT_BEHAVIOR.to_string()),
                step_idx,
                wakeup_id: first_string_env(&["OPENDAN_WAKEUP_ID"])
                    .unwrap_or_else(|| DEFAULT_WAKEUP_ID.to_string()),
                session_id: first_string_env(&["OPENDAN_SESSION_ID"])
                    .unwrap_or_else(|| DEFAULT_SESSION_ID.to_string()),
            },
        })
    }

    fn use_plain_text_read_output(&self) -> bool {
        !self.has_agent_env && !self.stdout_is_terminal
    }
}

/// What the parser produced. The dispatcher resolves the tool against
/// the registry and asks it to parse its own argv via
/// `AgentTool::parse_cli_args`. Pseudo-tools (`check_task`/`cancel_task`)
/// stay as variants because they don't live in the tool registry.
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
    CheckTask {
        tool_name: String,
        task_id: i64,
    },
    CancelTask {
        tool_name: String,
        task_id: i64,
        recursive: bool,
    },
    AgentMemory {
        tool_name: String,
        invocation: AgentMemoryInvocation,
    },
    AgentNotebook {
        tool_name: String,
        invocation: AgentNotebookInvocation,
    },
}

/// Parsed `agent-memory` command before execution. Mirrors §3.1/§4.x of the
/// v2.8 contract. `root_override` is the resolved `--root` / env / default.
#[derive(Clone, Debug)]
struct AgentMemoryInvocation {
    root_override: Option<PathBuf>,
    quiet: bool,
    verb: AgentMemoryVerb,
}

#[derive(Clone, Debug)]
enum AgentMemoryVerb {
    Init,
    Set {
        key: String,
        /// `Some` → content was passed as positional argv (form A).
        /// `None` → content must come from stdin (form B).
        content: Option<String>,
        reason: String,
    },
    Remove {
        key: String,
        reason: Option<String>,
    },
    Get {
        key: String,
    },
    List {
        prefix: Option<String>,
    },
    Load {
        tags: Vec<String>,
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
        ParsedCommand::CheckTask { tool_name, task_id } => {
            let task_mgr = build_task_manager_client(&env).await?;
            let task = task_mgr.get_task(task_id).await.map_err(|err| {
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
        ParsedCommand::CancelTask {
            tool_name,
            task_id,
            recursive,
        } => {
            let task_mgr = build_task_manager_client(&env).await?;
            let before = task_mgr.get_task(task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("get task `{task_id}` failed: {err}"))
            })?;
            task_mgr
                .cancel_task(task_id, recursive)
                .await
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("cancel task `{task_id}` failed: {err}"))
                })?;
            let interrupt_error = interrupt_task_if_supported(&before).await;
            let after = task_mgr.get_task(task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("reload task `{task_id}` failed: {err}"))
            })?;
            Ok(render_cli_output(
                &build_cancel_task_result(&tool_name, after, recursive, interrupt_error),
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
        TOOL_CHECK_TASK => parse_check_task_cli_command(tool_name, tokens),
        TOOL_CANCEL_TASK => parse_cancel_task_cli_command(tool_name, tokens),
        TOOL_AGENT_MEMORY | TOOL_AGENT_MEMORY_SNAKE => {
            parse_agent_memory_cli_command(tool_name, tokens)
        }
        TOOL_AGENT_NOTEBOOK | TOOL_AGENT_NOTEBOOK_SNAKE => {
            parse_agent_notebook_cli_command(tool_name, tokens)
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

fn parse_task_id_arg(tokens: &[String], tool_name: &str) -> Result<i64, AgentToolError> {
    if tokens.is_empty() {
        return Err(with_tool_usage("missing required arg `task_id`", tool_name));
    }

    let mut task_id: Option<i64> = None;
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

fn parse_task_id_value(raw: &str, tool_name: &str) -> Result<i64, AgentToolError> {
    raw.trim()
        .parse::<i64>()
        .map_err(|_| with_tool_usage(format!("invalid task_id `{}`", raw.trim()), tool_name))
}

// =================================================================
//  agent-memory CLI
// =================================================================

const AGENT_MEMORY_USAGE: &str = "agent-memory [--root <path>] [--quiet] \
<init|set|remove|get|list|load|verify|compact> [...]";

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

fn parse_agent_memory_set(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
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
            })
        }
        1 => {
            let key = positionals.into_iter().next().unwrap();
            Ok(AgentMemoryVerb::Set {
                key,
                content: None,
                reason,
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
    if rest.len() != 1 {
        return Err(agent_memory_invalid(format!(
            "`get` expects exactly 1 positional argument (key), got {}",
            rest.len()
        )));
    }
    Ok(AgentMemoryVerb::Get {
        key: rest[0].clone(),
    })
}

fn parse_agent_memory_list(rest: &[String]) -> Result<AgentMemoryVerb, AgentToolError> {
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
        max_records,
        max_bytes,
    })
}

fn parse_load_count(raw: &str, name: &str) -> Result<usize, AgentToolError> {
    raw.trim()
        .parse::<usize>()
        .map_err(|_| agent_memory_invalid(format!("invalid `--{name}` value `{raw}`")))
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

/// Resolve `--root` → env var `AGENT_MEMORY_ROOT` → `<state_root>/memory`.
fn resolve_agent_memory_root(env: &CliRuntimeEnv, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path {
        return canonicalize_or_normalize(p, Some(env.current_dir.as_path()));
    }
    if let Some(value) = first_path_env(&[AGENT_MEMORY_ROOT_ENV], &env.current_dir) {
        return value;
    }
    cli_state_root(env).join(AGENT_MEMORY_DIR_NAME)
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
        } => {
            let content = content.expect("stdin form resolved earlier");
            mem.set(&key, &content, &reason)?;
            Ok(String::new())
        }
        AgentMemoryVerb::Remove { key, reason } => {
            mem.remove(&key, reason.as_deref())?;
            Ok(String::new())
        }
        AgentMemoryVerb::Get { key } => mem.get(&key),
        AgentMemoryVerb::List { prefix } => {
            let keys = mem.list(prefix.as_deref())?;
            let mut out = keys.join("\n");
            if !out.is_empty() {
                out.push('\n');
            }
            Ok(out)
        }
        AgentMemoryVerb::Load {
            tags,
            max_records,
            max_bytes,
        } => {
            let mut opts = LoadOptions::default();
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

const AGENT_NOTEBOOK_USAGE: &str =
    "agent-notebook [--root <path> | env AGENT_NOTEBOOK_ROOT] \
[--owner-user <user_id> | env OPENDAN_OWNER_USER_ID] \
[--owner-agent <agent> | env OPENDAN_AGENT_ID] \
[--session <id> | env OPENDAN_SESSION_ID] \
<list|read|append|status|promote|create-notebook|registry-context|\
system-context|hints> [...]";

#[derive(Clone, Debug)]
struct AgentNotebookInvocation {
    root_override: Option<PathBuf>,
    owner_user_id: Option<String>,
    owner_agent_id: Option<String>,
    session_id: Option<String>,
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
        actor_kind: ActorKind,
        actor_id: Option<String>,
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
        actor_kind: ActorKind,
        actor_id: Option<String>,
    },
    Promote {
        item_id: String,
        reason: String,
        actor_kind: ActorKind,
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
    let mut owner_user_id: Option<String> = None;
    let mut owner_agent_id: Option<String> = None;
    let mut session_id: Option<String> = None;
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
            "--owner-user" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--owner-user`"))?;
                owner_user_id = Some(value.clone());
            }
            v if v.starts_with("--owner-user=") => {
                owner_user_id = Some(v["--owner-user=".len()..].to_string());
            }
            "--owner-agent" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--owner-agent`"))?;
                owner_agent_id = Some(value.clone());
            }
            v if v.starts_with("--owner-agent=") => {
                owner_agent_id = Some(v["--owner-agent=".len()..].to_string());
            }
            "--session" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--session`"))?;
                session_id = Some(value.clone());
            }
            v if v.starts_with("--session=") => {
                session_id = Some(v["--session=".len()..].to_string());
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
        other => return Err(agent_notebook_invalid(format!("unknown verb `{other}`"))),
    };

    Ok(ParsedCommand::AgentNotebook {
        tool_name,
        invocation: AgentNotebookInvocation {
            root_override,
            owner_user_id,
            owner_agent_id,
            session_id,
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
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--include-status`"))?;
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
    if positionals.len() != 1 {
        return Err(agent_notebook_invalid(format!(
            "`read` expects exactly 1 positional argument (notebook_id), got {}",
            positionals.len()
        )));
    }
    Ok(AgentNotebookVerb::Read {
        notebook_id: positionals.into_iter().next().unwrap(),
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
    let mut use_stdin = false;
    let mut source_excerpt: Option<String> = None;
    let mut actor_kind: Option<ActorKind> = None;
    let mut actor_id: Option<String> = None;
    let mut write_reason: Option<WriteReason> = None;
    let mut confidence: Option<Confidence> = None;
    let mut valid_from: Option<String> = None;
    let mut valid_until: Option<String> = None;
    let mut tags: Vec<String> = Vec::new();
    let mut detect_conflicts = true;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
            "--stdin" => use_stdin = true,
            "--source-excerpt" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--source-excerpt`"))?;
                source_excerpt = Some(value.clone());
            }
            v if v.starts_with("--source-excerpt=") => {
                source_excerpt = Some(v["--source-excerpt=".len()..].to_string());
            }
            "--actor-kind" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--actor-kind`"))?;
                actor_kind = Some(parse_actor_kind(value)?);
            }
            v if v.starts_with("--actor-kind=") => {
                actor_kind = Some(parse_actor_kind(&v["--actor-kind=".len()..])?);
            }
            "--actor-id" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--actor-id`"))?;
                actor_id = Some(value.clone());
            }
            v if v.starts_with("--actor-id=") => {
                actor_id = Some(v["--actor-id=".len()..].to_string());
            }
            "--write-reason" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--write-reason`"))?;
                write_reason = Some(parse_write_reason(value)?);
            }
            v if v.starts_with("--write-reason=") => {
                write_reason = Some(parse_write_reason(&v["--write-reason=".len()..])?);
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
    let actor_kind = actor_kind
        .ok_or_else(|| agent_notebook_invalid("`append` requires `--actor-kind`"))?;
    let write_reason = write_reason
        .ok_or_else(|| agent_notebook_invalid("`append` requires `--write-reason`"))?;

    let (notebook_id, title, content) = match (use_stdin, positionals.len()) {
        (false, 3) => {
            let mut it = positionals.into_iter();
            (it.next().unwrap(), it.next().unwrap(), Some(it.next().unwrap()))
        }
        (true, 2) => {
            let mut it = positionals.into_iter();
            (it.next().unwrap(), it.next().unwrap(), None)
        }
        (false, 2) => {
            return Err(agent_notebook_invalid(
                "`append` expects positional `<content>` or `--stdin`",
            ));
        }
        (true, 3) => {
            return Err(agent_notebook_invalid(
                "`append --stdin` does not accept a positional `<content>`",
            ));
        }
        (_, n) => {
            return Err(agent_notebook_invalid(format!(
                "`append` expects 2-3 positional arguments (notebook_id, title[, content]), got {n}"
            )));
        }
    };

    Ok(AgentNotebookVerb::Append {
        notebook_id,
        title,
        content,
        source_excerpt,
        actor_kind,
        actor_id,
        write_reason,
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
    let mut actor_kind: Option<ActorKind> = None;
    let mut actor_id: Option<String> = None;
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
            "--actor-kind" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--actor-kind`"))?;
                actor_kind = Some(parse_actor_kind(value)?);
            }
            v if v.starts_with("--actor-kind=") => {
                actor_kind = Some(parse_actor_kind(&v["--actor-kind=".len()..])?);
            }
            "--actor-id" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--actor-id`"))?;
                actor_id = Some(value.clone());
            }
            v if v.starts_with("--actor-id=") => {
                actor_id = Some(v["--actor-id=".len()..].to_string());
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
    let actor_kind =
        actor_kind.ok_or_else(|| agent_notebook_invalid("`status` requires `--actor-kind`"))?;
    Ok(AgentNotebookVerb::Status {
        item_id,
        status,
        reason,
        superseded_by,
        expected_item_revision,
        actor_kind,
        actor_id,
    })
}

fn parse_agent_notebook_promote(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut reason: Option<String> = None;
    let mut actor_kind: Option<ActorKind> = None;
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
            "--actor-kind" => {
                idx += 1;
                let value = rest
                    .get(idx)
                    .ok_or_else(|| agent_notebook_invalid("missing value for `--actor-kind`"))?;
                actor_kind = Some(parse_actor_kind(value)?);
            }
            v if v.starts_with("--actor-kind=") => {
                actor_kind = Some(parse_actor_kind(&v["--actor-kind=".len()..])?);
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
    let reason =
        reason.ok_or_else(|| agent_notebook_invalid("`promote` requires `--reason`"))?;
    let actor_kind = actor_kind
        .ok_or_else(|| agent_notebook_invalid("`promote` requires `--actor-kind`"))?;
    Ok(AgentNotebookVerb::Promote {
        item_id: positionals.into_iter().next().unwrap(),
        reason,
        actor_kind,
        replace_item_id,
    })
}

fn parse_agent_notebook_create(rest: &[String]) -> Result<AgentNotebookVerb, AgentToolError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut kind: Option<NotebookKind> = None;
    let mut title: Option<String> = None;
    let mut description: Option<String> = None;
    let mut idx = 0usize;
    while idx < rest.len() {
        let token = &rest[idx];
        match token.as_str() {
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
    if positionals.len() != 1 {
        return Err(agent_notebook_invalid(format!(
            "`create-notebook` expects 1 positional argument (notebook_id), got {}",
            positionals.len()
        )));
    }
    Ok(AgentNotebookVerb::CreateNotebook {
        notebook_id: positionals.into_iter().next().unwrap(),
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
                max_notebooks = Some(parse_usize(&v["--max-notebooks=".len()..], "max-notebooks")?);
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

fn parse_actor_kind(raw: &str) -> Result<ActorKind, AgentToolError> {
    Ok(match raw.trim() {
        "user" => ActorKind::User,
        "online_agent" => ActorKind::OnlineAgent,
        "curator" => ActorKind::Curator,
        "system" => ActorKind::System,
        "admin" => ActorKind::Admin,
        other => {
            return Err(agent_notebook_invalid(format!(
                "invalid actor_kind `{other}` (expected user|online_agent|curator|system|admin)"
            )))
        }
    })
}

fn parse_write_reason(raw: &str) -> Result<WriteReason, AgentToolError> {
    Ok(match raw.trim() {
        "user_explicit" => WriteReason::UserExplicit,
        "strong_rule" => WriteReason::StrongRule,
        "project_state" => WriteReason::ProjectState,
        "curator_extracted" => WriteReason::CuratorExtracted,
        "curator_cleanup" => WriteReason::CuratorCleanup,
        "manual_admin" => WriteReason::ManualAdmin,
        other => {
            return Err(agent_notebook_invalid(format!(
                "invalid write_reason `{other}`"
            )))
        }
    })
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

fn resolve_agent_notebook_root(env: &CliRuntimeEnv, override_path: Option<PathBuf>) -> PathBuf {
    if let Some(p) = override_path {
        return canonicalize_or_normalize(p, Some(env.current_dir.as_path()));
    }
    if let Some(value) = first_path_env(&[AGENT_NOTEBOOK_ROOT_ENV], &env.current_dir) {
        return value;
    }
    cli_state_root(env).join(AGENT_NOTEBOOK_DIR_NAME)
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

async fn dispatch_agent_notebook(
    env: &CliRuntimeEnv,
    _tool_name: &str,
    invocation: AgentNotebookInvocation,
    stdin_override: Option<String>,
) -> CliRunOutput {
    let AgentNotebookInvocation {
        root_override,
        owner_user_id,
        owner_agent_id,
        session_id,
        verb,
    } = invocation;

    let owner_user_id = match owner_user_id
        .or_else(|| first_string_env(&[DEFAULT_OWNER_USER_ENV]))
    {
        Some(v) if !v.trim().is_empty() => v,
        _ => {
            return CliRunOutput {
                exit_code: 2,
                stdout: format!(
                    "{}\n",
                    json!({
                        "status": "error",
                        "code": "invalid_input",
                        "message": format!(
                            "missing --owner-user (or set ${})", DEFAULT_OWNER_USER_ENV
                        ),
                    })
                ),
                stderr: String::new(),
            };
        }
    };
    let owner_agent_id = owner_agent_id
        .or_else(|| first_string_env(&[DEFAULT_OWNER_AGENT_ENV]))
        .filter(|v| !v.trim().is_empty());
    let session_id = session_id
        .or_else(|| first_string_env(&[DEFAULT_SESSION_ENV]))
        .filter(|v| !v.trim().is_empty());

    let root = resolve_agent_notebook_root(env, root_override);

    // Stdin pickup for `append --stdin`.
    let resolved_verb = match verb {
        AgentNotebookVerb::Append {
            notebook_id,
            title,
            content,
            source_excerpt,
            actor_kind,
            actor_id,
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
                    actor_kind,
                    actor_id,
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
        v => v,
    };

    let session_arg = session_id.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_agent_notebook_blocking(
            &root,
            &owner_user_id,
            owner_agent_id.as_deref(),
            session_arg.as_deref(),
            resolved_verb,
        )
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
            let mut stdout = serde_json::to_string(&value)
                .unwrap_or_else(|_| "{\"status\":\"ok\"}".to_string());
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

fn build_owner_scope(owner_user_id: &str, owner_agent_id: Option<&str>) -> OwnerScope {
    let mut scope = OwnerScope::new(owner_user_id.to_string());
    if let Some(agent) = owner_agent_id {
        scope = scope.with_agent(agent.to_string());
    }
    scope
}

fn run_agent_notebook_blocking(
    root: &Path,
    owner_user_id: &str,
    owner_agent_id: Option<&str>,
    session_id: Option<&str>,
    verb: AgentNotebookVerb,
) -> nb::Result<Json> {
    let cfg = AgentNotebookConfig::new(root);
    let notebook = AgentNotebook::open(cfg)?;
    let scope = build_owner_scope(owner_user_id, owner_agent_id);
    match verb {
        AgentNotebookVerb::List { include_archived } => {
            let entries = notebook.list_notebooks(ListNotebooksInput {
                scope,
                include_archived,
            })?;
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
                scope,
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
            actor_kind,
            actor_id,
            write_reason,
            confidence,
            valid_from,
            valid_until,
            tags,
            detect_conflicts,
        } => {
            let result = notebook.append_note(AppendNoteInput {
                scope,
                session_id: session_id.map(|s| s.to_string()),
                notebook_id,
                title,
                content: content.expect("stdin form resolved earlier"),
                source_excerpt,
                source_ref: None,
                source_session_id: session_id.map(|s| s.to_string()),
                actor_kind,
                actor_id,
                write_reason,
                valid_from,
                valid_until,
                confidence,
                tags,
                metadata: None,
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
            actor_kind,
            actor_id,
        } => {
            let result = notebook.mark_note_status(MarkNoteStatusInput {
                scope,
                session_id: session_id.map(|s| s.to_string()),
                item_id,
                status,
                reason,
                superseded_by,
                expected_item_revision,
                actor_kind,
                actor_id,
            })?;
            Ok(serde_json::to_value(result)?)
        }
        AgentNotebookVerb::Promote {
            item_id,
            reason,
            actor_kind,
            replace_item_id,
        } => {
            let result = notebook.promote_to_system_notebook(PromoteToSystemInput {
                scope,
                item_id,
                reason,
                actor_kind,
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
                scope,
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
            let result = notebook.build_notebook_registry_context(BuildRegistryContextInput {
                scope,
                max_notebooks,
            })?;
            Ok(json!({
                "status": "ok",
                "registry": result,
            }))
        }
        AgentNotebookVerb::SystemContext { max_items } => {
            let result = notebook.build_system_notebook_context(BuildSystemContextInput {
                scope,
                max_items,
            })?;
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
                .ok_or_else(|| NotebookError::InvalidInput("`hints` requires --session".into()))?;
            let result = notebook.build_notebook_hints(BuildHintsInput {
                scope,
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
    }
}

/// Thin wrapper so we can emit the tagged-enum directly without an extra
/// envelope (the enum already serializes a `status` discriminant per §5.3).
struct NotebookReadResultWire(NotebookReadResult);

impl Serialize for NotebookReadResultWire {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

struct PromoteResultWire(PromoteToSystemResult);

impl Serialize for PromoteResultWire {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error> {
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
    mgr.register_typed_tool(ReadFileTool::new(file_cfg.clone()))?;
    mgr.register_typed_tool(WriteFileTool::new(file_cfg.clone(), audit.clone()))?;
    mgr.register_typed_tool(EditFileTool::new(file_cfg, audit))?;

    Ok(mgr)
}

fn build_cli_file_tool_config(env: &CliRuntimeEnv) -> FileToolConfig {
    let mut cfg = FileToolConfig::new(env.current_dir.clone());
    if !env.has_agent_env {
        cfg.allowed_read_roots.clear();
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
        TOOL_CHECK_TASK => "check_task <task_id>",
        TOOL_CANCEL_TASK => "cancel_task <task_id> [--recursive]",
        _ => "agent_tool <tool> ...",
    };
    AgentToolError::InvalidArgs(format!("{}\nUsage: {usage}", message.into()))
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
    _env: &CliRuntimeEnv,
) -> Result<TaskManagerClient, AgentToolError> {
    if let Ok(runtime) = get_buckyos_api_runtime() {
        return runtime.get_task_mgr_client().await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "init task-manager client from runtime failed: {err}"
            ))
        });
    }

    if let Some(url) = first_string_env(&[
        "OPENDAN_TASK_MANAGER_URL",
        "OPENDAN_TASK_MANAGER_RPC",
        "TASK_MANAGER_URL",
        "TASK_MANAGER_RPC",
    ]) {
        let session_token = first_string_env(&["OPENDAN_SESSION_TOKEN", "SESSION_TOKEN"]);
        return Ok(TaskManagerClient::new(kRPC::new(
            url.as_str(),
            session_token,
        )));
    }

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
    let is_exec_bash_task = task.data.get("kind").and_then(Json::as_str) == Some("tool.exec_bash");
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
        task.data
            .get("command")
            .and_then(Json::as_str)
            .map(|value| value.to_string())
    } else {
        Some(format!("{tool_name} {}", task.id))
    };
    let output = task
        .data
        .get("output")
        .and_then(Json::as_str)
        .map(|value| value.to_string());
    let return_code = task
        .data
        .get("return_code")
        .or_else(|| task.data.get("exit_code"))
        .and_then(Json::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let estimated_wait = task
        .data
        .get("estimated_wait")
        .and_then(Json::as_str)
        .map(|value| value.to_string());
    let check_after = task
        .data
        .get("check_after")
        .and_then(Json::as_u64)
        .or_else(|| (top_status == AgentToolStatus::Pending).then_some(5));

    let mut result = AgentToolResult::from_details(detail)
        .with_status(top_status)
        .with_result(summary)
        .with_task_id(task.id.to_string());
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
        Some(err) => format!("canceled task {} (interrupt failed: {err})", task.id),
        None => format!("canceled task {}", task.id),
    };

    AgentToolResult::from_details(detail)
        .with_status(AgentToolStatus::Success)
        .with_result(summary)
        .with_title(format!("{tool_name} {} => success", task.id))
        .with_tool(tool_name)
        .with_cmd_line(format!("{tool_name} {}", task.id))
        .with_task_id(task.id.to_string())
}

fn normalized_task_detail(task: &Task) -> Json {
    let mut detail = if task.data.is_object() {
        task.data.clone()
    } else {
        json!({ "task_data": task.data.clone() })
    };
    if let Some(map) = detail.as_object_mut() {
        map.entry("task_id".to_string())
            .or_insert_with(|| Json::String(task.id.to_string()));
        map.entry("task_status".to_string())
            .or_insert_with(|| Json::String(task.status.to_string()));
        map.entry("task_name".to_string())
            .or_insert_with(|| Json::String(task.name.clone()));
        map.entry("task_type".to_string())
            .or_insert_with(|| Json::String(task.task_type.clone()));
        map.entry("task_progress".to_string())
            .or_insert_with(|| json!(task.progress));
        if let Some(message) = task.message.as_ref() {
            map.entry("task_message".to_string())
                .or_insert_with(|| Json::String(message.clone()));
        }
    }
    detail
}

fn task_protocol_status(task: &Task) -> AgentToolStatus {
    match task.status {
        TaskStatus::Completed => match task.data.get("status").and_then(Json::as_str) {
            Some("error") => AgentToolStatus::Error,
            _ => AgentToolStatus::Success,
        },
        TaskStatus::Failed | TaskStatus::Canceled => AgentToolStatus::Error,
        TaskStatus::Pending
        | TaskStatus::Running
        | TaskStatus::Paused
        | TaskStatus::WaitingForApproval => AgentToolStatus::Pending,
    }
}

fn task_summary(task: &Task, protocol_status: AgentToolStatus) -> String {
    task.data
        .get("summary")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| task.message.as_ref().map(|value| value.trim().to_string()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| match (protocol_status, task.status) {
            (AgentToolStatus::Pending, TaskStatus::WaitingForApproval) => {
                format!("task {} is waiting for approval", task.id)
            }
            (AgentToolStatus::Pending, _) => format!("task {} is still running", task.id),
            (AgentToolStatus::Success, _) => format!("task {} completed", task.id),
            (AgentToolStatus::Error, TaskStatus::Canceled) => {
                format!("task {} was canceled", task.id)
            }
            (AgentToolStatus::Error, _) => format!("task {} failed", task.id),
        })
}

fn task_pending_reason(task: &Task) -> Option<AgentToolPendingReason> {
    task.data
        .get("pending_reason")
        .and_then(Json::as_str)
        .and_then(|value| match value {
            "user_approval" => Some(AgentToolPendingReason::UserApproval),
            "wait_for_install" | "external_callback" => {
                Some(AgentToolPendingReason::WaitForInstall)
            }
            "long_running" => Some(AgentToolPendingReason::LongRunning),
            _ => None,
        })
        .or_else(|| match task.status {
            TaskStatus::WaitingForApproval => Some(AgentToolPendingReason::UserApproval),
            TaskStatus::Pending | TaskStatus::Running | TaskStatus::Paused => {
                Some(AgentToolPendingReason::LongRunning)
            }
            _ => None,
        })
}

async fn interrupt_task_if_supported(task: &Task) -> Option<String> {
    if task.data.get("kind").and_then(Json::as_str) != Some("tool.exec_bash") {
        return None;
    }
    let tmux_target = task
        .data
        .get("tmux_target")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())?;

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

    use tempfile::tempdir;
    use tokio::fs;

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
        CliRuntimeEnv {
            agent_env_root: canonicalize_or_normalize(agent_env_root, None),
            has_agent_env: true,
            current_dir: canonicalize_or_normalize(current_dir, None),
            stdout_is_terminal: true,
            call_ctx: SessionRuntimeContext {
                trace_id: "trace-test".to_string(),
                agent_name: "did:example:agent".to_string(),
                behavior: "cli".to_string(),
                step_idx: 0,
                wakeup_id: "wakeup-test".to_string(),
                session_id: "session-test".to_string(),
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
                OsString::from("--pos-chunk"),
                OsString::from("world"),
                OsString::from("--new-content"),
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
            Some(14)
        );
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

        let memory_path = root
            .join("memory")
            .join("user")
            .join("preference")
            .join("style");
        let content = fs::read_to_string(&memory_path)
            .await
            .expect("read memory file");
        assert_eq!(content, "concise english");

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
        assert!(fs::metadata(&memory_path).await.is_err());

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

        let stored = fs::read_to_string(root.join("memory").join("user").join("note"))
            .await
            .expect("read stored content");
        assert_eq!(stored, body);
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
        assert!(load_output.stdout.contains("KEY /user/dental\n"));
        assert!(load_output.stdout.contains("---\n"));
        assert!(load_output.stdout.contains("\nEND\n"));
        assert!(load_output.stdout.contains("MATCHED dental"));
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
            test_env(root, cwd),
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
                assert_eq!(task_id, 42);
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
                assert_eq!(task_id, 7);
                assert!(recursive);
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
            CliRuntimeEnv {
                agent_env_root: canonicalize_or_normalize(temp.path().join("cwd"), None),
                has_agent_env: false,
                current_dir: canonicalize_or_normalize(temp.path().join("cwd"), None),
                stdout_is_terminal: true,
                call_ctx: SessionRuntimeContext {
                    trace_id: "trace-test".to_string(),
                    agent_name: "did:example:agent".to_string(),
                    behavior: "cli".to_string(),
                    step_idx: 0,
                    wakeup_id: "wakeup-test".to_string(),
                    session_id: "session-test".to_string(),
                },
            },
            None,
        )
        .await
        .expect("run read_file");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse json");
        assert_eq!(payload["status"], "success");
        assert_eq!(payload["detail"]["content"], "free\n");
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
            CliRuntimeEnv {
                agent_env_root: canonicalize_or_normalize(temp.path().join("cwd"), None),
                has_agent_env: false,
                current_dir: canonicalize_or_normalize(temp.path().join("cwd"), None),
                stdout_is_terminal: false,
                call_ctx: SessionRuntimeContext {
                    trace_id: "trace-test".to_string(),
                    agent_name: "did:example:agent".to_string(),
                    behavior: "cli".to_string(),
                    step_idx: 0,
                    wakeup_id: "wakeup-test".to_string(),
                    session_id: "session-test".to_string(),
                },
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("--session"),
                OsString::from("s1"),
                OsString::from("append"),
                OsString::from("user/preferences"),
                OsString::from("concise replies"),
                OsString::from("user prefers terse output"),
                OsString::from("--actor-kind"),
                OsString::from("online_agent"),
                OsString::from("--write-reason"),
                OsString::from("user_explicit"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("--session"),
                OsString::from("s1"),
                OsString::from("read"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("--session"),
                OsString::from("s1"),
                OsString::from("read"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("--session"),
                OsString::from("s1"),
                OsString::from("append"),
                OsString::from("projects/demo"),
                OsString::from("design notes"),
                OsString::from("--stdin"),
                OsString::from("--actor-kind"),
                OsString::from("curator"),
                OsString::from("--write-reason"),
                OsString::from("curator_extracted"),
                OsString::from("--tags"),
                OsString::from("design,notes"),
            ],
            test_env(root.clone(), cwd),
            Some(body.to_string()),
        )
        .await
        .expect("run agent-notebook append --stdin");
        assert_eq!(output.exit_code, EXIT_SUCCESS);
        let payload: Json =
            serde_json::from_str(output.stdout.trim()).expect("parse append json");
        assert_eq!(payload["status"], "ok");
    }

    #[tokio::test]
    async fn agent_notebook_missing_owner_user_returns_error_json() {
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
                OsString::from("list"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run agent-notebook list without owner");
        assert_eq!(output.exit_code, 2);
        let payload: Json =
            serde_json::from_str(output.stdout.trim()).expect("parse error json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "invalid_input");
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("append"),
                OsString::from("user/preferences"),
                OsString::from("old fact"),
                OsString::from("a stale fact"),
                OsString::from("--actor-kind"),
                OsString::from("online_agent"),
                OsString::from("--write-reason"),
                OsString::from("user_explicit"),
                OsString::from("--tags"),
                OsString::from("fact"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("seed item");
        assert_eq!(seed.exit_code, EXIT_SUCCESS);
        let seed_payload: Json =
            serde_json::from_str(seed.stdout.trim()).expect("parse seed json");
        let item_id = seed_payload["item_id"].as_str().expect("item_id").to_string();

        // Mark stale.
        let status_output = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("status"),
                OsString::from(item_id.clone()),
                OsString::from("stale"),
                OsString::from("--reason"),
                OsString::from("no longer applies"),
                OsString::from("--actor-kind"),
                OsString::from("curator"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("--session"),
                OsString::from("s1"),
                OsString::from("read"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("append"),
                OsString::from("user/preferences"),
                OsString::from("bad"),
                OsString::from("x"),
                OsString::from("--actor-kind"),
                OsString::from("online_agent"),
                OsString::from("--write-reason"),
                OsString::from("user_explicit"),
                OsString::from("--tags"),
                OsString::from("bad\"tag"),
            ],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run agent-notebook append with bad tag");
        assert_eq!(output.exit_code, 2);
        let payload: Json =
            serde_json::from_str(output.stdout.trim()).expect("parse error json");
        assert_eq!(payload["status"], "error");
        assert_eq!(payload["code"], "invalid_tag");
    }

    #[tokio::test]
    async fn agent_notebook_env_fallbacks_replace_cli_flags() {
        // --owner-user / --owner-agent / --session / --root all come from env.
        // Env vars are process-global, so hold ENV_TEST_LOCK to keep other
        // notebook tests from seeing them.
        let _lock = nb_lock();
        let temp = tempdir().expect("create tempdir");
        let nb_root = temp.path().join("nb-root");
        let cwd = temp.path().join("cwd");
        fs::create_dir_all(&cwd).await.expect("create cwd");

        struct EnvGuard(&'static str);
        impl Drop for EnvGuard {
            fn drop(&mut self) {
                std::env::remove_var(self.0);
            }
        }
        std::env::set_var(AGENT_NOTEBOOK_ROOT_ENV, &nb_root);
        std::env::set_var(DEFAULT_OWNER_USER_ENV, "alice");
        std::env::set_var(DEFAULT_OWNER_AGENT_ENV, "did:opendan:test");
        std::env::set_var(DEFAULT_SESSION_ENV, "env-session");
        let _g1 = EnvGuard(AGENT_NOTEBOOK_ROOT_ENV);
        let _g2 = EnvGuard(DEFAULT_OWNER_USER_ENV);
        let _g3 = EnvGuard(DEFAULT_OWNER_AGENT_ENV);
        let _g4 = EnvGuard(DEFAULT_SESSION_ENV);

        // Append with zero CLI flags beyond verb-specific ones.
        let out = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("append"),
                OsString::from("user/preferences"),
                OsString::from("from env"),
                OsString::from("body via env-resolved scope"),
                OsString::from("--actor-kind"),
                OsString::from("online_agent"),
                OsString::from("--write-reason"),
                OsString::from("user_explicit"),
                OsString::from("--tags"),
                OsString::from("env-test"),
            ],
            test_env(temp.path().join("agent-env"), cwd.clone()),
            None,
        )
        .await
        .expect("run append via env");
        assert_eq!(
            out.exit_code, EXIT_SUCCESS,
            "stdout={:?}",
            out.stdout
        );
        assert!(nb_root.join("notebook.sqlite").exists());

        // Read also picks env session / owner up.
        let read = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("read"),
                OsString::from("user/preferences"),
                OsString::from("--tags"),
                OsString::from("env-test"),
            ],
            test_env(temp.path().join("agent-env"), cwd),
            None,
        )
        .await
        .expect("run read via env");
        assert_eq!(read.exit_code, EXIT_SUCCESS);
        let payload: Json =
            serde_json::from_str(read.stdout.trim()).expect("parse env read json");
        assert_eq!(payload["status"], "ok");
        let entries = payload["entries"].as_array().expect("entries array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["title"], "from env");
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
                    OsString::from("--owner-user"),
                    OsString::from("alice"),
                    OsString::from("append"),
                    OsString::from(nb_id),
                    OsString::from(title),
                    OsString::from("seed content"),
                    OsString::from("--actor-kind"),
                    OsString::from("online_agent"),
                    OsString::from("--write-reason"),
                    OsString::from("user_explicit"),
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
                OsString::from("--owner-user"),
                OsString::from("alice"),
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
        let text = payload["registry"]["text"]
            .as_str()
            .expect("registry text");
        assert!(text.contains("user/preferences"));
        assert!(text.contains("projects/demo"));
        // Body content must not leak into registry.
        assert!(!text.contains("seed content"));

        // hints with topic_tags works.
        let hints_out = execute(
            vec![
                OsString::from("/tmp/agent-notebook"),
                OsString::from("--owner-user"),
                OsString::from("alice"),
                OsString::from("--session"),
                OsString::from("session-test"),
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
