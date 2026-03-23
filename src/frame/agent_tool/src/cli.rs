use std::env;
use std::ffi::OsString;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

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

use crate::{
    cli_exit_code_for_error, normalize_abs_path, parse_read_file_bash_args, render_cli_output,
    rewrite_read_file_path_with_shell_cwd, session_record_path, AgentTool, AgentToolError,
    AgentToolResult, BindWorkspaceTool, CliPendingReason, CliResultEnvelope, CliRunOutput,
    CliStatus, CreateWorkspaceTool, EditFileTool, FileToolConfig, GetSessionTool,
    NoopFileWriteAudit, ReadFileTool, SessionRuntimeContext, SessionViewBackend, TodoTool,
    TodoToolConfig, WorkspaceToolBackend, WriteFileTool, TOOL_BIND_WORKSPACE,
    TOOL_CREATE_WORKSPACE, TOOL_GET_SESSION,
};

const TOOL_TODO: &str = "todo";
const TOOL_READ_FILE: &str = "read_file";
const TOOL_WRITE_FILE: &str = "write_file";
const TOOL_EDIT_FILE: &str = "edit_file";
const TOOL_CHECK_TASK: &str = "check_task";
const TOOL_CANCEL_TASK: &str = "cancel_task";
const TOOL_NAMES: [&str; 9] = [
    TOOL_READ_FILE,
    TOOL_WRITE_FILE,
    TOOL_EDIT_FILE,
    TOOL_GET_SESSION,
    TOOL_TODO,
    TOOL_CREATE_WORKSPACE,
    TOOL_BIND_WORKSPACE,
    TOOL_CHECK_TASK,
    TOOL_CANCEL_TASK,
];
const EXIT_SUCCESS: i32 = crate::CLI_EXIT_SUCCESS;
const EXIT_COMMAND_NOT_FOUND: i32 = crate::CLI_EXIT_COMMAND_NOT_FOUND;
const COMMAND_NOT_FOUND_PROXY: &str = "__command_not_found__";
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

#[derive(Clone, Debug)]
enum ContentInput {
    Inline(String),
    Stdin,
}

#[derive(Clone, Debug)]
enum ParsedCommand {
    CommandNotFound {
        command: Option<String>,
        argv: Vec<String>,
    },
    Help {
        tool_name: Option<String>,
    },
    BashTool {
        tool_name: String,
        line: String,
    },
    ReadFile {
        tool_name: String,
        args: Json,
    },
    WriteFile {
        tool_name: String,
        args: Json,
        content: ContentInput,
    },
    EditFile {
        tool_name: String,
        args: Json,
        new_content: ContentInput,
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
}

pub async fn run_process() -> CliRunOutput {
    let args = env::args_os().collect::<Vec<_>>();
    let env = match CliRuntimeEnv::from_process() {
        Ok(env) => env,
        Err(err) => {
            let exit_code = cli_exit_code_for_error(&err);
            return render_cli_output(&CliResultEnvelope::error(None, &err), exit_code);
        }
    };

    match execute(args, env, None).await {
        Ok(output) => output,
        Err(err) => {
            let exit_code = cli_exit_code_for_error(&err);
            render_cli_output(&CliResultEnvelope::error(None, &err), exit_code)
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
        ParsedCommand::CommandNotFound { command, argv } => Ok(CliRunOutput {
            exit_code: EXIT_COMMAND_NOT_FOUND,
            stdout: String::new(),
            stderr: render_command_not_found_log(command.as_deref(), &argv),
        }),
        ParsedCommand::Help { tool_name } => Ok(render_cli_output(
            &build_help_envelope(&env, tool_name.as_deref()).await,
            EXIT_SUCCESS,
        )),
        ParsedCommand::BashTool { tool_name, line } => {
            let result = execute_bash_tool(&env, &tool_name, &line).await?;
            Ok(render_cli_output(
                &success_envelope(&tool_name, result),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::ReadFile { tool_name, args } => {
            if env.use_plain_text_read_output() {
                match run_read_file(&env, args).await {
                    Ok(result) => Ok(render_plain_read_file_output(result)),
                    Err(err) => Ok(render_plain_error_output(&err)),
                }
            } else {
                let result = run_read_file(&env, args).await?;
                Ok(render_cli_output(
                    &success_envelope(&tool_name, result),
                    EXIT_SUCCESS,
                ))
            }
        }
        ParsedCommand::WriteFile {
            tool_name,
            mut args,
            content,
        } => {
            let content = resolve_content_input(content, stdin_override).await?;
            let map = args.as_object_mut().ok_or_else(|| {
                AgentToolError::InvalidArgs("write_file args must be object".to_string())
            })?;
            map.insert("content".to_string(), Json::String(content));
            let result = run_write_file(&env, args).await?;
            Ok(render_cli_output(
                &success_envelope(&tool_name, result),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::EditFile {
            tool_name,
            mut args,
            new_content,
        } => {
            let new_content = resolve_content_input(new_content, stdin_override).await?;
            let map = args.as_object_mut().ok_or_else(|| {
                AgentToolError::InvalidArgs("edit_file args must be object".to_string())
            })?;
            map.insert("new_content".to_string(), Json::String(new_content));
            let result = run_edit_file(&env, args).await?;
            Ok(render_cli_output(
                &success_envelope(&tool_name, result),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::CheckTask { tool_name, task_id } => {
            let task_mgr = build_task_manager_client(&env).await?;
            let task = task_mgr.get_task(task_id).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("get task `{task_id}` failed: {err}"))
            })?;
            Ok(render_cli_output(
                &build_check_task_envelope(&tool_name, task),
                EXIT_SUCCESS,
            ))
        }
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
                &build_cancel_task_envelope(&tool_name, after, recursive, interrupt_error),
                EXIT_SUCCESS,
            ))
        }
    }
}

async fn resolve_content_input(
    input: ContentInput,
    stdin_override: Option<String>,
) -> Result<String, AgentToolError> {
    match input {
        ContentInput::Inline(value) => Ok(value),
        ContentInput::Stdin => {
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
        TOOL_READ_FILE => Ok(ParsedCommand::ReadFile {
            tool_name,
            args: parse_read_file_cli_args(tokens, current_dir)?,
        }),
        TOOL_WRITE_FILE => parse_write_file_cli_args(tool_name, tokens, current_dir),
        TOOL_EDIT_FILE => parse_edit_file_cli_args(tool_name, tokens, current_dir),
        TOOL_GET_SESSION => parse_get_session_cli_command(tool_name, tokens),
        TOOL_TODO | TOOL_CREATE_WORKSPACE | TOOL_BIND_WORKSPACE => {
            Ok(parse_passthrough_bash_command(tool_name, tokens))
        }
        TOOL_CHECK_TASK => parse_check_task_cli_command(tool_name, tokens),
        TOOL_CANCEL_TASK => parse_cancel_task_cli_command(tool_name, tokens),
        _ => Err(AgentToolError::InvalidArgs(format!(
            "unsupported tool `{tool_name}`"
        ))),
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

fn parse_get_session_cli_command(
    tool_name: String,
    tokens: &[String],
) -> Result<ParsedCommand, AgentToolError> {
    if tokens.is_empty() {
        return Ok(ParsedCommand::BashTool {
            tool_name: tool_name.clone(),
            line: tool_name,
        });
    }

    let mut session_id: Option<String> = None;
    let mut idx = 0usize;
    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "--session-id" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage("missing value for `--session-id`", TOOL_GET_SESSION)
                })?;
                session_id = Some(value.clone());
            }
            token if token.starts_with("--") => {
                return Err(with_tool_usage(
                    format!("unsupported flag `{token}`"),
                    TOOL_GET_SESSION,
                ));
            }
            token if token.contains('=') => {
                let (key, value) = token
                    .split_once('=')
                    .ok_or_else(|| with_tool_usage("invalid key=value arg", TOOL_GET_SESSION))?;
                match key {
                    "session_id" | "session" => {
                        session_id = Some(value.to_string());
                    }
                    _ => {
                        return Err(with_tool_usage(
                            format!("unsupported arg `{key}`"),
                            TOOL_GET_SESSION,
                        ));
                    }
                }
            }
            value => {
                if session_id.is_some() {
                    return Err(with_tool_usage(
                        format!("unexpected positional arg `{value}`"),
                        TOOL_GET_SESSION,
                    ));
                }
                session_id = Some(value.to_string());
            }
        }
        idx += 1;
    }

    let mut forwarded = Vec::new();
    if let Some(session_id) = session_id {
        forwarded.push(format!("session_id={session_id}"));
    }
    Ok(ParsedCommand::BashTool {
        tool_name: tool_name.clone(),
        line: build_bash_cli_line(&tool_name, &forwarded),
    })
}

fn parse_passthrough_bash_command(tool_name: String, tokens: &[String]) -> ParsedCommand {
    ParsedCommand::BashTool {
        tool_name: tool_name.clone(),
        line: build_bash_cli_line(&tool_name, tokens),
    }
}

fn parse_read_file_cli_args(tokens: &[String], current_dir: &Path) -> Result<Json, AgentToolError> {
    let mut args = parse_read_file_bash_args(tokens)?;
    rewrite_read_file_path_with_shell_cwd(&mut args, current_dir);
    Ok(args)
}

fn parse_write_file_cli_args(
    tool_name: String,
    tokens: &[String],
    current_dir: &Path,
) -> Result<ParsedCommand, AgentToolError> {
    if tokens.is_empty() {
        return Err(with_tool_usage(
            "missing required arg `path`",
            TOOL_WRITE_FILE,
        ));
    }

    let mut args = serde_json::Map::<String, Json>::new();
    let mut path: Option<String> = None;
    let mut content: Option<ContentInput> = None;
    let mut mode: Option<String> = None;
    let mut idx = 0usize;

    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "--mode" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage("missing value for `--mode`", TOOL_WRITE_FILE)
                })?;
                mode = Some(value.clone());
            }
            "--content" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage("missing value for `--content`", TOOL_WRITE_FILE)
                })?;
                content = Some(ContentInput::Inline(value.clone()));
            }
            "--content-stdin" => {
                content = Some(ContentInput::Stdin);
            }
            token if token.starts_with("--") => {
                return Err(with_tool_usage(
                    format!("unsupported flag `{token}`"),
                    TOOL_WRITE_FILE,
                ));
            }
            token if token.contains('=') => {
                let (key, value) = token
                    .split_once('=')
                    .ok_or_else(|| with_tool_usage("invalid key=value arg", TOOL_WRITE_FILE))?;
                match key {
                    "path" => path = Some(value.to_string()),
                    "mode" => mode = Some(value.to_string()),
                    "content" => content = Some(ContentInput::Inline(value.to_string())),
                    "content_stdin" => {
                        if value == "true" {
                            content = Some(ContentInput::Stdin);
                        } else {
                            return Err(with_tool_usage(
                                "content_stdin must be `true` when provided",
                                TOOL_WRITE_FILE,
                            ));
                        }
                    }
                    _ => {
                        return Err(with_tool_usage(
                            format!("unsupported arg `{key}`"),
                            TOOL_WRITE_FILE,
                        ));
                    }
                }
            }
            value => {
                if path.is_none() {
                    path = Some(value.to_string());
                } else {
                    return Err(with_tool_usage(
                        format!("unexpected positional arg `{value}`"),
                        TOOL_WRITE_FILE,
                    ));
                }
            }
        }
        idx += 1;
    }

    let path =
        path.ok_or_else(|| with_tool_usage("missing required arg `path`", TOOL_WRITE_FILE))?;
    let content = content.ok_or_else(|| {
        with_tool_usage(
            "one of `--content` or `--content-stdin` is required",
            TOOL_WRITE_FILE,
        )
    })?;
    args.insert(
        "path".to_string(),
        Json::String(rewrite_path_with_shell_cwd(path, current_dir)),
    );
    if let Some(mode) = mode {
        args.insert("mode".to_string(), Json::String(mode));
    }

    Ok(ParsedCommand::WriteFile {
        tool_name,
        args: Json::Object(args),
        content,
    })
}

fn parse_edit_file_cli_args(
    tool_name: String,
    tokens: &[String],
    current_dir: &Path,
) -> Result<ParsedCommand, AgentToolError> {
    if tokens.is_empty() {
        return Err(with_tool_usage(
            "missing required arg `path`",
            TOOL_EDIT_FILE,
        ));
    }

    let mut args = serde_json::Map::<String, Json>::new();
    let mut path: Option<String> = None;
    let mut pos_chunk: Option<String> = None;
    let mut new_content: Option<ContentInput> = None;
    let mut mode: Option<String> = None;
    let mut idx = 0usize;

    while idx < tokens.len() {
        match tokens[idx].as_str() {
            "--mode" => {
                idx += 1;
                let value = tokens
                    .get(idx)
                    .ok_or_else(|| with_tool_usage("missing value for `--mode`", TOOL_EDIT_FILE))?;
                mode = Some(value.clone());
            }
            "--pos-chunk" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage("missing value for `--pos-chunk`", TOOL_EDIT_FILE)
                })?;
                pos_chunk = Some(value.clone());
            }
            "--new-content" => {
                idx += 1;
                let value = tokens.get(idx).ok_or_else(|| {
                    with_tool_usage("missing value for `--new-content`", TOOL_EDIT_FILE)
                })?;
                new_content = Some(ContentInput::Inline(value.clone()));
            }
            "--new-content-stdin" => {
                new_content = Some(ContentInput::Stdin);
            }
            token if token.starts_with("--") => {
                return Err(with_tool_usage(
                    format!("unsupported flag `{token}`"),
                    TOOL_EDIT_FILE,
                ));
            }
            token if token.contains('=') => {
                let (key, value) = token
                    .split_once('=')
                    .ok_or_else(|| with_tool_usage("invalid key=value arg", TOOL_EDIT_FILE))?;
                match key {
                    "path" => path = Some(value.to_string()),
                    "mode" => mode = Some(value.to_string()),
                    "pos_chunk" => pos_chunk = Some(value.to_string()),
                    "new_content" => new_content = Some(ContentInput::Inline(value.to_string())),
                    "new_content_stdin" => {
                        if value == "true" {
                            new_content = Some(ContentInput::Stdin);
                        } else {
                            return Err(with_tool_usage(
                                "new_content_stdin must be `true` when provided",
                                TOOL_EDIT_FILE,
                            ));
                        }
                    }
                    _ => {
                        return Err(with_tool_usage(
                            format!("unsupported arg `{key}`"),
                            TOOL_EDIT_FILE,
                        ));
                    }
                }
            }
            value => {
                if path.is_none() {
                    path = Some(value.to_string());
                } else {
                    return Err(with_tool_usage(
                        format!("unexpected positional arg `{value}`"),
                        TOOL_EDIT_FILE,
                    ));
                }
            }
        }
        idx += 1;
    }

    let path =
        path.ok_or_else(|| with_tool_usage("missing required arg `path`", TOOL_EDIT_FILE))?;
    let pos_chunk = pos_chunk
        .ok_or_else(|| with_tool_usage("missing required arg `--pos-chunk`", TOOL_EDIT_FILE))?;
    let new_content = new_content.ok_or_else(|| {
        with_tool_usage(
            "one of `--new-content` or `--new-content-stdin` is required",
            TOOL_EDIT_FILE,
        )
    })?;

    args.insert(
        "path".to_string(),
        Json::String(rewrite_path_with_shell_cwd(path, current_dir)),
    );
    args.insert("pos_chunk".to_string(), Json::String(pos_chunk));
    if let Some(mode) = mode {
        args.insert("mode".to_string(), Json::String(mode));
    }

    Ok(ParsedCommand::EditFile {
        tool_name,
        args: Json::Object(args),
        new_content,
    })
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
        let workspace_rel_path = format!("workspaces/{workspace_id}");
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

        let mut index = load_workspace_index(&self.state_root).await?;
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

async fn execute_bash_tool(
    env: &CliRuntimeEnv,
    tool_name: &str,
    line: &str,
) -> Result<AgentToolResult, AgentToolError> {
    match tool_name {
        TOOL_GET_SESSION => {
            build_get_session_tool(env)?
                .exec(&env.call_ctx, line, Some(env.current_dir.as_path()))
                .await
        }
        TOOL_TODO => {
            build_todo_tool(env)?
                .exec(&env.call_ctx, line, Some(env.current_dir.as_path()))
                .await
        }
        TOOL_CREATE_WORKSPACE => {
            build_create_workspace_tool(env)?
                .exec(&env.call_ctx, line, Some(env.current_dir.as_path()))
                .await
        }
        TOOL_BIND_WORKSPACE => {
            build_bind_workspace_tool(env)?
                .exec(&env.call_ctx, line, Some(env.current_dir.as_path()))
                .await
        }
        _ => Err(AgentToolError::NotFound(tool_name.to_string())),
    }
}

fn build_get_session_tool(env: &CliRuntimeEnv) -> Result<GetSessionTool, AgentToolError> {
    Ok(GetSessionTool::new(Arc::new(CliSessionBackend {
        state_root: cli_state_root(env),
    })))
}

fn build_todo_tool(env: &CliRuntimeEnv) -> Result<TodoTool, AgentToolError> {
    TodoTool::new(TodoToolConfig::with_db_path(
        cli_state_root(env).join("todo").join("todo.db"),
    ))
}

fn build_create_workspace_tool(env: &CliRuntimeEnv) -> Result<CreateWorkspaceTool, AgentToolError> {
    Ok(CreateWorkspaceTool::new(Arc::new(CliWorkspaceBackend {
        state_root: cli_state_root(env),
        agent_id: env.call_ctx.agent_name.clone(),
    })))
}

fn build_bind_workspace_tool(env: &CliRuntimeEnv) -> Result<BindWorkspaceTool, AgentToolError> {
    Ok(BindWorkspaceTool::new(Arc::new(CliWorkspaceBackend {
        state_root: cli_state_root(env),
        agent_id: env.call_ctx.agent_name.clone(),
    })))
}

fn build_cli_file_tool_config(env: &CliRuntimeEnv) -> FileToolConfig {
    let mut cfg = FileToolConfig::new(env.current_dir.clone());
    if !env.has_agent_env {
        cfg.allowed_read_roots.clear();
        cfg.allowed_write_roots.clear();
    }
    cfg
}

fn build_read_file_tool(env: &CliRuntimeEnv) -> ReadFileTool {
    ReadFileTool::new(build_cli_file_tool_config(env))
}

fn build_write_file_tool(env: &CliRuntimeEnv) -> WriteFileTool {
    WriteFileTool::new(
        build_cli_file_tool_config(env),
        Arc::new(NoopFileWriteAudit),
    )
}

fn build_edit_file_tool(env: &CliRuntimeEnv) -> EditFileTool {
    EditFileTool::new(
        build_cli_file_tool_config(env),
        Arc::new(NoopFileWriteAudit),
    )
}

async fn run_read_file(env: &CliRuntimeEnv, args: Json) -> Result<AgentToolResult, AgentToolError> {
    build_read_file_tool(env).call(&env.call_ctx, args).await
}

async fn run_write_file(
    env: &CliRuntimeEnv,
    args: Json,
) -> Result<AgentToolResult, AgentToolError> {
    build_write_file_tool(env).call(&env.call_ctx, args).await
}

async fn run_edit_file(env: &CliRuntimeEnv, args: Json) -> Result<AgentToolResult, AgentToolError> {
    build_edit_file_tool(env).call(&env.call_ctx, args).await
}

fn success_envelope(tool_name: &str, result: AgentToolResult) -> CliResultEnvelope {
    CliResultEnvelope::from_tool_result(tool_name, result)
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

async fn build_help_envelope(_env: &CliRuntimeEnv, tool_name: Option<&str>) -> CliResultEnvelope {
    static_help_envelope(tool_name)
}

fn static_help_envelope(tool_name: Option<&str>) -> CliResultEnvelope {
    match tool_name {
        Some(tool_name) => CliResultEnvelope::success(
            Some(tool_name.to_string()),
            json!({
                "tool": tool_name,
                "usage": static_tool_usage(tool_name),
            }),
            "show usage",
        ),
        None => CliResultEnvelope::success(
            None,
            json!({
                "usage": generic_usage(),
                "tools": TOOL_NAMES.iter().map(|tool_name| {
                    json!({
                        "name": tool_name,
                        "usage": static_tool_usage(tool_name),
                    })
                }).collect::<Vec<_>>(),
            }),
            "show usage",
        ),
    }
}

fn render_command_not_found_log(command: Option<&str>, argv: &[String]) -> String {
    let command = command.unwrap_or("").trim();
    let argv_text = if argv.is_empty() {
        String::new()
    } else {
        format!(" argv={}", argv.join(" "))
    };
    if command.is_empty() {
        "agent_tool auto-implement placeholder: command_not_found with empty command".to_string()
    } else {
        format!("agent_tool auto-implement placeholder: missing command `{command}`{argv_text}")
    }
}

fn with_tool_usage(message: impl Into<String>, tool_name: &str) -> AgentToolError {
    let message = message.into();
    AgentToolError::InvalidArgs(format!(
        "{message}\nUsage: {}",
        static_tool_usage(tool_name)
    ))
}

fn static_tool_usage(tool_name: &str) -> &'static str {
    match tool_name {
        TOOL_READ_FILE => {
            "read_file <path> [range] [first_chunk]\n\trange: 1-based; supports negative/$/+N, and applies within first_chunk slice"
        }
        TOOL_WRITE_FILE => {
            "write_file <path> [--mode new|append|write] (--content <text> | --content-stdin)"
        }
        TOOL_EDIT_FILE => {
            "edit_file <path> --pos-chunk <text> [--mode replace|after|before] (--new-content <text> | --new-content-stdin)"
        }
        TOOL_GET_SESSION => "get_session [session_id]",
        TOOL_TODO => "todo <command> [args...]",
        TOOL_CREATE_WORKSPACE => "create_workspace <name> <summary>",
        TOOL_BIND_WORKSPACE => "bind_workspace <workspace_id|workspace_path>",
        TOOL_CHECK_TASK => "check_task <task_id>",
        TOOL_CANCEL_TASK => "cancel_task <task_id> [--recursive]",
        _ => "agent_tool <tool> ...",
    }
}

fn generic_usage() -> &'static str {
    "agent_tool <read_file|write_file|edit_file|get_session|todo|create_workspace|bind_workspace|check_task|cancel_task> [args...]"
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

fn rewrite_path_with_shell_cwd(raw_path: String, current_dir: &Path) -> String {
    let path = Path::new(raw_path.trim());
    if path.is_absolute() {
        return canonicalize_or_normalize(path.to_path_buf(), None)
            .to_string_lossy()
            .to_string();
    }
    canonicalize_or_normalize(path.to_path_buf(), Some(current_dir))
        .to_string_lossy()
        .to_string()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
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

fn build_check_task_envelope(tool_name: &str, task: Task) -> CliResultEnvelope {
    let top_status = task_protocol_status(&task);
    let summary = task_summary(&task, top_status);
    let pending_reason = task_pending_reason(&task);
    let is_agent_tool = task
        .data
        .get("is_agent_tool")
        .and_then(Json::as_bool)
        .unwrap_or(true);
    let mut detail = if is_agent_tool {
        normalized_task_detail(&task)
    } else {
        json!({})
    };
    if is_agent_tool {
        if let Some(map) = detail.as_object_mut() {
            map.insert("task".to_string(), json!(task.clone()));
        }
    }

    CliResultEnvelope {
        is_agent_tool,
        status: top_status,
        summary,
        tool: is_agent_tool.then_some(tool_name.to_string()),
        cmd_line: if is_agent_tool {
            Some(format!("{tool_name} {}", task.id))
        } else {
            task.data
                .get("command")
                .and_then(Json::as_str)
                .map(|value| value.to_string())
        },
        detail,
        output: task
            .data
            .get("output")
            .and_then(Json::as_str)
            .map(|value| value.to_string()),
        return_code: task
            .data
            .get("return_code")
            .or_else(|| task.data.get("exit_code"))
            .and_then(Json::as_i64)
            .and_then(|value| i32::try_from(value).ok()),
        pending_reason,
        task_id: Some(task.id.to_string()),
        estimated_wait: task
            .data
            .get("estimated_wait")
            .and_then(Json::as_str)
            .map(|value| value.to_string()),
        check_after: task
            .data
            .get("check_after")
            .and_then(Json::as_u64)
            .or_else(|| (top_status == CliStatus::Pending).then_some(5)),
    }
}

fn build_cancel_task_envelope(
    tool_name: &str,
    task: Task,
    recursive: bool,
    interrupt_error: Option<String>,
) -> CliResultEnvelope {
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

    CliResultEnvelope {
        is_agent_tool: true,
        status: CliStatus::Success,
        summary,
        tool: Some(tool_name.to_string()),
        cmd_line: Some(format!("{tool_name} {}", task.id)),
        detail,
        output: None,
        return_code: None,
        pending_reason: None,
        task_id: Some(task.id.to_string()),
        estimated_wait: None,
        check_after: None,
    }
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

fn task_protocol_status(task: &Task) -> CliStatus {
    match task.status {
        TaskStatus::Completed => match task.data.get("status").and_then(Json::as_str) {
            Some("error") => CliStatus::Error,
            _ => CliStatus::Success,
        },
        TaskStatus::Failed | TaskStatus::Canceled => CliStatus::Error,
        TaskStatus::Pending
        | TaskStatus::Running
        | TaskStatus::Paused
        | TaskStatus::WaitingForApproval => CliStatus::Pending,
    }
}

fn task_summary(task: &Task, protocol_status: CliStatus) -> String {
    task.data
        .get("summary")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .or_else(|| task.message.as_ref().map(|value| value.trim().to_string()))
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| match (protocol_status, task.status) {
            (CliStatus::Pending, TaskStatus::WaitingForApproval) => {
                format!("task {} is waiting for approval", task.id)
            }
            (CliStatus::Pending, _) => format!("task {} is still running", task.id),
            (CliStatus::Success, _) => format!("task {} completed", task.id),
            (CliStatus::Error, TaskStatus::Canceled) => format!("task {} was canceled", task.id),
            (CliStatus::Error, _) => format!("task {} failed", task.id),
        })
}

fn task_pending_reason(task: &Task) -> Option<CliPendingReason> {
    task.data
        .get("pending_reason")
        .and_then(Json::as_str)
        .and_then(|value| match value {
            "user_approval" => Some(CliPendingReason::UserApproval),
            "wait_for_install" | "external_callback" => Some(CliPendingReason::WaitForInstall),
            "long_running" => Some(CliPendingReason::LongRunning),
            _ => None,
        })
        .or_else(|| match task.status {
            TaskStatus::WaitingForApproval => Some(CliPendingReason::UserApproval),
            TaskStatus::Pending | TaskStatus::Running | TaskStatus::Paused => {
                Some(CliPendingReason::LongRunning)
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

fn build_bash_cli_line(tool_name: &str, tokens: &[String]) -> String {
    let mut line = String::from(tool_name);
    for token in tokens {
        line.push(' ');
        line.push_str(&shell_quote_token(token));
    }
    line
}

fn shell_quote_token(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
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

    use tempfile::tempdir;
    use tokio::fs;

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
        assert_eq!(payload["cmd_name"], TOOL_READ_FILE);
        assert_eq!(payload["detail"]["content"], "line-1");
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
            Some(9)
        );
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

        assert_eq!(output.exit_code, EXIT_COMMAND_NOT_FOUND);
        assert!(output.stdout.is_empty());
        assert!(output.stderr.contains("auto-implement placeholder"));
        assert!(output.stderr.contains("missing_tool"));
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
    async fn todo_alias_uses_bound_workspace_without_rpc() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let cwd = root.join("workspace");
        fs::create_dir_all(&cwd)
            .await
            .expect("create workspace dir");
        seed_session(&root, "session-test", &cwd).await;

        let _ = execute(
            vec![
                OsString::from("/tmp/create_workspace"),
                OsString::from("demo"),
                OsString::from("workspace summary"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("create workspace");

        let add_output = execute(
            vec![
                OsString::from("/tmp/todo"),
                OsString::from("add"),
                OsString::from("Task A"),
            ],
            test_env(root.clone(), cwd.clone()),
            None,
        )
        .await
        .expect("run todo add");
        let add_payload: Json = serde_json::from_str(&add_output.stdout).expect("parse add json");
        assert_eq!(add_payload["status"], "success");
        assert_eq!(add_payload["detail"]["created_items"][0]["title"], "Task A");

        let next_output = execute(
            vec![OsString::from("/tmp/todo"), OsString::from("next")],
            test_env(root, cwd),
            None,
        )
        .await
        .expect("run todo next");
        let next_payload: Json =
            serde_json::from_str(&next_output.stdout).expect("parse next json");
        assert_eq!(next_payload["status"], "success");
        assert_eq!(next_payload["detail"]["item"]["title"], "Task A");
    }
}
