use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Serialize;
use serde_json::{json, Value as Json};
use tokio::io::{self, AsyncReadExt};

use crate::agent_environment::AgentEnvironment;
use crate::agent_session::{AgentSessionMgr, GetSessionTool};
use crate::agent_tool::{
    AgentTool, AgentToolError, AgentToolManager, AgentToolResult, TOOL_BIND_WORKSPACE,
    TOOL_CREATE_WORKSPACE, TOOL_EDIT_FILE, TOOL_GET_SESSION, TOOL_READ_FILE, TOOL_WRITE_FILE,
};
use crate::behavior::SessionRuntimeContext;
use crate::buildin_tool::{
    normalize_abs_path, parse_read_file_bash_args, rewrite_read_file_path_with_shell_cwd,
    EditFileTool, ReadFileTool, WorkshopWriteAudit, WriteFileTool,
};
use crate::worklog::WorklogToolConfig;
use crate::workspace::workshop::{AgentWorkshopConfig, WorkshopToolConfig};

const TOOL_TODO: &str = "todo";
const TOOL_NAMES: [&str; 7] = [
    TOOL_READ_FILE,
    TOOL_WRITE_FILE,
    TOOL_EDIT_FILE,
    TOOL_GET_SESSION,
    TOOL_TODO,
    TOOL_CREATE_WORKSPACE,
    TOOL_BIND_WORKSPACE,
];
const EXIT_SUCCESS: i32 = 0;
const EXIT_ERROR: i32 = 1;
const EXIT_USAGE: i32 = 2;
const DEFAULT_AGENT_NAME: &str = "did:opendan:cli";
const DEFAULT_TRACE_ID: &str = "cli-trace";
const DEFAULT_SESSION_ID: &str = "cli-session";
const DEFAULT_WAKEUP_ID: &str = "cli-wakeup";
const DEFAULT_BEHAVIOR: &str = "cli";

#[derive(Clone, Debug)]
struct CliRuntimeEnv {
    agent_env_root: PathBuf,
    has_agent_env: bool,
    current_dir: PathBuf,
    call_ctx: SessionRuntimeContext,
}

impl CliRuntimeEnv {
    fn from_process() -> Result<Self, AgentToolError> {
        let current_dir = env::current_dir()
            .map(|path| canonicalize_or_normalize(path, None))
            .map_err(|err| {
                AgentToolError::ExecFailed(format!("resolve current dir failed: {err}"))
            })?;
        let agent_env_root = first_path_env(
            &["OPENDAN_AGENT_ENV", "AGENT_ENV", "AGENT_ROOT"],
            &current_dir,
        );
        let has_agent_env = agent_env_root.is_some();
        let agent_env_root = agent_env_root.unwrap_or_else(|| current_dir.clone());
        let step_idx = first_string_env(&["OPENDAN_STEP_IDX", "STEP_IDX"])
            .and_then(|raw| raw.parse::<u32>().ok())
            .unwrap_or(0);

        Ok(Self {
            agent_env_root,
            has_agent_env,
            current_dir,
            call_ctx: SessionRuntimeContext {
                trace_id: first_string_env(&["OPENDAN_TRACE_ID", "TRACE_ID"])
                    .unwrap_or_else(|| DEFAULT_TRACE_ID.to_string()),
                agent_name: first_string_env(&["OPENDAN_AGENT_ID", "AGENT_ID"])
                    .unwrap_or_else(|| DEFAULT_AGENT_NAME.to_string()),
                behavior: first_string_env(&["OPENDAN_BEHAVIOR", "BEHAVIOR"])
                    .unwrap_or_else(|| DEFAULT_BEHAVIOR.to_string()),
                step_idx,
                wakeup_id: first_string_env(&["OPENDAN_WAKEUP_ID", "WAKEUP_ID"])
                    .unwrap_or_else(|| DEFAULT_WAKEUP_ID.to_string()),
                session_id: first_string_env(&["OPENDAN_SESSION_ID", "SESSION_ID"])
                    .unwrap_or_else(|| DEFAULT_SESSION_ID.to_string()),
            },
        })
    }
}

#[derive(Debug)]
pub struct CliRunOutput {
    pub exit_code: i32,
    pub stdout: String,
}

#[derive(Clone, Debug)]
enum ContentInput {
    Inline(String),
    Stdin,
}

#[derive(Clone, Debug)]
enum ParsedCommand {
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
}

#[derive(Clone, Debug, Serialize)]
struct CliResultEnvelope {
    status: &'static str,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cmd_line: Option<String>,
    detail: Json,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stderr: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pending_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    estimated_wait: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    check_after: Option<u64>,
}

pub async fn run_process() -> CliRunOutput {
    let args = env::args_os().collect::<Vec<_>>();
    let env = match CliRuntimeEnv::from_process() {
        Ok(env) => env,
        Err(err) => {
            let exit_code = exit_code_for_error(&err);
            return render_output(error_envelope(None, err), exit_code);
        }
    };

    match execute(args, env, None).await {
        Ok(output) => output,
        Err(err) => {
            let exit_code = exit_code_for_error(&err);
            render_output(error_envelope(None, err), exit_code)
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
        ParsedCommand::Help { tool_name } => Ok(render_output(
            build_help_envelope(&env, tool_name.as_deref()).await,
            EXIT_SUCCESS,
        )),
        ParsedCommand::BashTool { tool_name, line } => {
            let tool_mgr = build_runtime_tool_manager(&env).await?;
            let result = tool_mgr
                .call_tool_from_bash_line_with_cwd(
                    &env.call_ctx,
                    &line,
                    Some(env.current_dir.as_path()),
                )
                .await?
                .ok_or_else(|| AgentToolError::NotFound(tool_name.clone()))?;
            Ok(render_output(
                success_envelope(&tool_name, result),
                EXIT_SUCCESS,
            ))
        }
        ParsedCommand::ReadFile { tool_name, args } => {
            let tool = build_read_file_tool(&env)?;
            let result = tool.call(&env.call_ctx, args).await?;
            Ok(render_output(
                success_envelope(&tool_name, result),
                EXIT_SUCCESS,
            ))
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
            let tool = build_write_file_tool(&env)?;
            let result = tool.call(&env.call_ctx, args).await?;
            Ok(render_output(
                success_envelope(&tool_name, result),
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
            let tool = build_edit_file_tool(&env)?;
            let result = tool.call(&env.call_ctx, args).await?;
            Ok(render_output(
                success_envelope(&tool_name, result),
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
        .unwrap_or("agent-tools");
    let rest = args
        .iter()
        .skip(1)
        .map(os_to_string)
        .collect::<Result<Vec<_>, _>>()?;

    if is_tool_name(argv0) {
        return parse_tool_command(argv0.to_string(), &rest, current_dir);
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
        _ => Err(AgentToolError::InvalidArgs(format!(
            "unsupported tool `{tool_name}`"
        ))),
    }
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

fn build_read_file_tool(env: &CliRuntimeEnv) -> Result<ReadFileTool, AgentToolError> {
    ReadFileTool::from_tool_config(
        &cli_workshop_config(env),
        &WorkshopToolConfig::enabled(TOOL_READ_FILE),
    )
}

fn build_write_file_tool(env: &CliRuntimeEnv) -> Result<WriteFileTool, AgentToolError> {
    let cfg = cli_workshop_config(env);
    let audit = WorkshopWriteAudit::new(WorklogToolConfig::with_db_path(
        cli_state_root(env).join("worklog").join("worklog.db"),
    ));
    WriteFileTool::from_tool_config(&cfg, &WorkshopToolConfig::enabled(TOOL_WRITE_FILE), audit)
}

fn build_edit_file_tool(env: &CliRuntimeEnv) -> Result<EditFileTool, AgentToolError> {
    let cfg = cli_workshop_config(env);
    let audit = WorkshopWriteAudit::new(WorklogToolConfig::with_db_path(
        cli_state_root(env).join("worklog").join("worklog.db"),
    ));
    EditFileTool::from_tool_config(&cfg, &WorkshopToolConfig::enabled(TOOL_EDIT_FILE), audit)
}

fn cli_workshop_config(env: &CliRuntimeEnv) -> AgentWorkshopConfig {
    let mut cfg = AgentWorkshopConfig::new(&env.agent_env_root);
    cfg.agent_did = env.call_ctx.agent_name.clone();
    cfg
}

fn cli_state_root(env: &CliRuntimeEnv) -> PathBuf {
    if env.has_agent_env {
        env.agent_env_root.clone()
    } else {
        env.current_dir.join(".opendan-cli")
    }
}

fn success_envelope(tool_name: &str, result: AgentToolResult) -> CliResultEnvelope {
    let detail = result.details.clone();
    let status = if detail.get("status").and_then(Json::as_str) == Some("pending") {
        "pending"
    } else {
        "success"
    };
    CliResultEnvelope {
        status,
        summary: result
            .result
            .clone()
            .unwrap_or_else(|| "completed".to_string()),
        tool: Some(tool_name.to_string()),
        cmd_line: (!result.cmd_line.trim().is_empty()).then_some(result.cmd_line),
        detail,
        stdout: result.stdout,
        stderr: result.stderr,
        pending_reason: result
            .details
            .get("pending_reason")
            .and_then(Json::as_str)
            .map(|value| value.to_string()),
        task_id: result
            .details
            .get("task_id")
            .and_then(Json::as_str)
            .map(|value| value.to_string()),
        estimated_wait: None,
        check_after: result.details.get("check_after").and_then(Json::as_u64),
    }
}

fn error_envelope(tool_name: Option<&str>, err: AgentToolError) -> CliResultEnvelope {
    let message = err.to_string();
    CliResultEnvelope {
        status: "error",
        summary: message.clone(),
        tool: tool_name.map(|value| value.to_string()),
        cmd_line: None,
        detail: json!({
            "error_type": error_kind(&err),
            "message": message,
        }),
        stdout: None,
        stderr: None,
        pending_reason: None,
        task_id: None,
        estimated_wait: None,
        check_after: None,
    }
}

async fn build_help_envelope(env: &CliRuntimeEnv, tool_name: Option<&str>) -> CliResultEnvelope {
    match load_help_specs(env).await {
        Ok(specs) => help_envelope_from_specs(tool_name, &specs),
        Err(_) => static_help_envelope(tool_name),
    }
}

async fn load_help_specs(env: &CliRuntimeEnv) -> Result<Vec<(String, String)>, AgentToolError> {
    let tool_mgr = build_runtime_tool_manager(env).await?;
    let specs = TOOL_NAMES
        .iter()
        .map(|tool_name| {
            let usage = tool_mgr
                .get_bash_cmd(tool_name)
                .map(|tool| tool.spec())
                .or_else(|| tool_mgr.get_tool(tool_name).map(|tool| tool.spec()))
                .and_then(|spec| spec.usage)
                .unwrap_or_else(|| static_tool_usage(tool_name).to_string());
            ((*tool_name).to_string(), usage)
        })
        .collect::<Vec<_>>();
    Ok(specs)
}

fn help_envelope_from_specs(
    tool_name: Option<&str>,
    specs: &[(String, String)],
) -> CliResultEnvelope {
    match tool_name {
        Some(tool_name) => CliResultEnvelope {
            status: "success",
            summary: "show usage".to_string(),
            tool: Some(tool_name.to_string()),
            cmd_line: None,
            detail: json!({
                "tool": tool_name,
                "usage": specs
                    .iter()
                    .find(|(name, _)| name == tool_name)
                    .map(|(_, usage)| usage.as_str())
                    .unwrap_or_else(|| static_tool_usage(tool_name)),
            }),
            stdout: None,
            stderr: None,
            pending_reason: None,
            task_id: None,
            estimated_wait: None,
            check_after: None,
        },
        None => CliResultEnvelope {
            status: "success",
            summary: "show usage".to_string(),
            tool: None,
            cmd_line: None,
            detail: json!({
                "usage": generic_usage(),
                "tools": specs.iter().map(|(tool_name, usage)| {
                    json!({
                        "name": tool_name,
                        "usage": usage,
                    })
                }).collect::<Vec<_>>(),
            }),
            stdout: None,
            stderr: None,
            pending_reason: None,
            task_id: None,
            estimated_wait: None,
            check_after: None,
        },
    }
}

fn static_help_envelope(tool_name: Option<&str>) -> CliResultEnvelope {
    match tool_name {
        Some(tool_name) => CliResultEnvelope {
            status: "success",
            summary: "show usage".to_string(),
            tool: Some(tool_name.to_string()),
            cmd_line: None,
            detail: json!({
                "tool": tool_name,
                "usage": static_tool_usage(tool_name),
            }),
            stdout: None,
            stderr: None,
            pending_reason: None,
            task_id: None,
            estimated_wait: None,
            check_after: None,
        },
        None => CliResultEnvelope {
            status: "success",
            summary: "show usage".to_string(),
            tool: None,
            cmd_line: None,
            detail: json!({
                "usage": generic_usage(),
                "tools": TOOL_NAMES.iter().map(|tool_name| {
                    json!({
                        "name": tool_name,
                        "usage": static_tool_usage(tool_name),
                    })
                }).collect::<Vec<_>>(),
            }),
            stdout: None,
            stderr: None,
            pending_reason: None,
            task_id: None,
            estimated_wait: None,
            check_after: None,
        },
    }
}

fn render_output(payload: CliResultEnvelope, exit_code: i32) -> CliRunOutput {
    let stdout = serde_json::to_string(&payload).unwrap_or_else(|_| {
        "{\"status\":\"error\",\"summary\":\"serialize cli result failed\",\"detail\":{}}"
            .to_string()
    });
    CliRunOutput { exit_code, stdout }
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
        _ => "agent-tools <tool> ...",
    }
}

fn generic_usage() -> &'static str {
    "agent-tools <read_file|write_file|edit_file|get_session|todo|create_workspace|bind_workspace> [args...]"
}

fn error_kind(err: &AgentToolError) -> &'static str {
    match err {
        AgentToolError::NotFound(_) => "not_found",
        AgentToolError::AlreadyExists(_) => "already_exists",
        AgentToolError::InvalidArgs(_) => "invalid_args",
        AgentToolError::ExecFailed(_) => "exec_failed",
        AgentToolError::Timeout => "timeout",
    }
}

pub fn exit_code_for_error(err: &AgentToolError) -> i32 {
    match err {
        AgentToolError::InvalidArgs(_) | AgentToolError::NotFound(_) => EXIT_USAGE,
        AgentToolError::AlreadyExists(_)
        | AgentToolError::ExecFailed(_)
        | AgentToolError::Timeout => EXIT_ERROR,
    }
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

async fn build_runtime_tool_manager(
    env: &CliRuntimeEnv,
) -> Result<AgentToolManager, AgentToolError> {
    let tool_mgr = AgentToolManager::new();
    let session_store = Arc::new(
        AgentSessionMgr::new(
            env.call_ctx.agent_name.clone(),
            env.agent_env_root.join("sessions"),
            env.call_ctx.behavior.clone(),
        )
        .await?,
    );
    let environment = AgentEnvironment::new(&env.agent_env_root).await?;
    environment.register_workshop_tools(&tool_mgr, session_store.clone())?;
    tool_mgr.register_tool(GetSessionTool::new(session_store))?;
    Ok(tool_mgr)
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

    use crate::agent_session::AgentSessionMgr;

    use tempfile::tempdir;
    use tokio::fs;

    fn test_env(agent_env_root: PathBuf, current_dir: PathBuf) -> CliRuntimeEnv {
        CliRuntimeEnv {
            agent_env_root: canonicalize_or_normalize(agent_env_root, None),
            has_agent_env: true,
            current_dir: canonicalize_or_normalize(current_dir, None),
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
        let store = AgentSessionMgr::new(
            "did:example:agent",
            agent_env_root.join("sessions"),
            "plan".to_string(),
        )
        .await
        .expect("create session store");
        let session = store
            .ensure_session(session_id, Some("CLI Session".to_string()), None, None)
            .await
            .expect("ensure session");
        {
            let mut guard = session.lock().await;
            guard.pwd = pwd.to_path_buf();
        }
        store.save_session(session_id).await.expect("save session");
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
        assert_eq!(payload["tool"], TOOL_READ_FILE);
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
                OsString::from("agent-tools"),
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
    async fn generic_help_lists_all_m1_tools() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().join("agent");
        let output = execute(
            vec![OsString::from("agent-tools"), OsString::from("--help")],
            test_env(root.clone(), root),
            None,
        )
        .await
        .expect("run help");

        let payload: Json = serde_json::from_str(&output.stdout).expect("parse help json");
        assert_eq!(payload["status"], "success");
        assert_eq!(
            payload["detail"]["tools"].as_array().map(|v| v.len()),
            Some(7)
        );
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
