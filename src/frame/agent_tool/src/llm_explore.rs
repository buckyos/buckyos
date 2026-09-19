//! `agent_tool llm_explore` —— 基于 LLM 的代码库探索工具。
//!
//! ## 设计
//!
//! 调用方给两段自然语言:
//! - `--description` (objective): 写进 worklog,不进 prompt。
//! - `--prompt`     (user content): 实际给 LLM 看的任务说明。
//!
//! 我们在一个本地目录上起一个 xllm Run（`local_llm_context`）,预装好 bash 工具组 /
//! Grep / exec_bash 等只读 / 读写工具,把这套 system prompt 钉在第一条
//! 消息上,然后 `drive_to_terminal`,把最终的助手输出整理成
//! `AgentToolResult` 写到 stdout。
//!
//! ## CLI 参数
//!
//! ```text
//!   agent_tool llm_explore \
//!     --description <text>     # 必填,探索任务描述(进 worklog,不进 prompt)
//!     --prompt <text>          # 必填,真正喂给 LLM 的用户消息
//!     [--root-dir <path>]      # 选填,探索根目录,默认 PWD
//!     [--work-dir <path>]      # 选填,LocalLLMContext 工作目录,
//!                              #   默认 $TMPDIR/llm_explore-<ts>-<desc>
//!     [--model <alias>]        # 选填,LLM 模型 alias,默认 llm.summary
//! ```
//!
//! ## 输出契约
//!
//! 不管成功或失败都写一份 `AgentToolResult` JSON 到 stdout,`details` 里
//! 一定带 `work_dir`(让用户能去看持久化的 snapshots / worklog)。
//!
//! ## 与 `run_local_llm` 的区别
//!
//! `run_local_llm` 是低层 dev 驱动,行为完全由 CLI flag 决定;`llm_explore`
//! 是一个具体业务工具——它**固定**了一套 system prompt(file search
//! specialist),只暴露给用户两段自然语言。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use buckyos_api::{AiMessage, AiRole};
use llm_context::LlmClient;
use serde_json::{json, Value};

use crate::local_llm_context::{
    ensure_buckyos_runtime, AiccLlmClient, LoopModel, RunOutcome, TaskInput, TaskOverrides,
    XllmDeps, XllmRun, XllmTask,
};
use crate::{
    cli_error_result, render_cli_output, AgentToolError, AgentToolPendingReason, AgentToolResult,
    AgentToolStatus, AGENT_TOOL_PROTOCOL_VERSION, CLI_EXIT_ERROR, CLI_EXIT_SUCCESS, CLI_EXIT_USAGE,
};

const TOOL_NAME: &str = "llm_explore";
const DEFAULT_MODEL_ALIAS: &str = "llm.summary";
const DEFAULT_MAX_ROUNDS: u32 = 16;

/// 钉在每个 llm_explore run 上的 system prompt。
const SYSTEM_PROMPT: &str = "\
You are a file search specialist. You excel at thoroughly navigating and exploring codebases.

Your strengths:
- Rapidly finding files using glob patterns
- Searching code and text with powerful regex patterns
- Reading and analyzing file contents

Guidelines:
- Use `exec` with find / grep / ls / git for broad discovery
- Use `read_file` when you know the specific file path you need to read
- Use `exec` ONLY for read-only operations: ls, find, grep, cat, head, tail, git status, git log, git diff
- NEVER run mkdir, touch, rm, cp, mv, git add, git commit, npm install, pip install, or modify files
- Adapt your search approach based on the thoroughness level specified by the caller
- Communicate your final report directly as a regular message - do NOT attempt to create files

NOTE: You are meant to be a fast agent that returns output as quickly as possible.";

// =========================================================================
// 入口
// =========================================================================

/// Dispatch entry, called by `lib::run_process` when argv[1] == "llm_explore".
/// `args` 是去掉 `agent_tool llm_explore` 之后的剩余参数。
pub async fn run_subcommand(args: Vec<String>) -> i32 {
    let opts = match CliOpts::parse(&args) {
        Ok(opts) => opts,
        Err(ParseError::Help) => {
            print!("{}", USAGE);
            return CLI_EXIT_SUCCESS;
        }
        Err(ParseError::Bad(msg)) => {
            eprintln!("error: {msg}\n\n{}", USAGE);
            emit_result(&cli_error_result(
                Some(TOOL_NAME),
                &AgentToolError::InvalidArgs(msg),
            ));
            return CLI_EXIT_USAGE;
        }
    };

    let (result, exit_code) = run(opts).await;
    emit_result(&result);
    exit_code
}

fn emit_result(result: &AgentToolResult) {
    let rendered = render_cli_output(result, 0);
    println!("{}", rendered.stdout);
}

async fn run(opts: CliOpts) -> (AgentToolResult, i32) {
    // 1) 工作目录（探索根）与 Runs 目录。
    let root_dir = match opts.root_dir.clone() {
        Some(p) => p,
        None => match std::env::current_dir() {
            Ok(p) => p,
            Err(err) => {
                return (
                    build_error_result(None, &opts, &format!("cannot resolve cwd: {err}")),
                    CLI_EXIT_ERROR,
                )
            }
        },
    };
    let work_dir = opts
        .work_dir
        .clone()
        .unwrap_or_else(|| default_work_dir(&opts.description));

    // 2) BuckyOS runtime + AICC client.
    if let Err(err) = ensure_buckyos_runtime().await {
        return (
            build_error_result(
                Some(&work_dir),
                &opts,
                &format!("init buckyos runtime failed: {err}"),
            ),
            CLI_EXIT_ERROR,
        );
    }
    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new());
    let deps = XllmDeps::default().with_llm(llm);

    // 3) 结构化输入：system prompt + user prompt；工具 = bash 组（function_call）。
    let input = TaskInput::structured(vec![
        AiMessage::text(AiRole::System, SYSTEM_PROMPT),
        AiMessage::text(AiRole::User, opts.prompt.clone()),
    ]);
    let overrides = TaskOverrides {
        model: Some(
            opts.model
                .clone()
                .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string()),
        ),
        loop_model: Some(LoopModel::FunctionCall),
        tools: Some(true),
        max_rounds: Some(DEFAULT_MAX_ROUNDS),
        runs_dir: Some(work_dir.clone()),
        ..Default::default()
    };
    let prepared = match XllmTask::prepare(&root_dir, input, overrides, &deps).await {
        Ok(p) => p,
        Err(err) => {
            return (
                build_error_result(Some(&work_dir), &opts, &format!("prepare failed: {err}")),
                CLI_EXIT_ERROR,
            )
        }
    };
    let mut run = match XllmRun::start(prepared, deps).await {
        Ok(r) => r,
        Err(err) => {
            return (
                build_error_result(Some(&work_dir), &opts, &format!("start run failed: {err}")),
                CLI_EXIT_ERROR,
            )
        }
    };
    let run_id = run.run_id().to_string();
    eprintln!(
        "llm_explore: work_dir={} run_id={}",
        work_dir.display(),
        run_id
    );

    // 4) 执行到本次停止点。
    let outcome = match run.execute().await {
        Ok(o) => o,
        Err(err) => {
            return (
                build_error_result(Some(&work_dir), &opts, &format!("execute failed: {err}")),
                CLI_EXIT_ERROR,
            )
        }
    };

    // 5) 把 outcome 翻译成 AgentToolResult。
    build_outcome_result(&work_dir, &run_id, &opts, outcome)
}

fn default_work_dir(description: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut safe = String::with_capacity(description.len());
    for ch in description.chars().take(32) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            safe.push(ch);
        } else if ch.is_whitespace() {
            safe.push('_');
        }
    }
    if safe.is_empty() {
        safe.push_str("explore");
    }
    std::env::temp_dir().join(format!("llm_explore-{ts}-{safe}"))
}

// =========================================================================
// outcome → AgentToolResult
// =========================================================================

fn build_outcome_result(
    work_dir: &Path,
    run_id: &str,
    opts: &CliOpts,
    outcome: RunOutcome,
) -> (AgentToolResult, i32) {
    let work_dir_str = work_dir.display().to_string();
    let record = outcome.record().clone();
    let usage = record.usage.total();
    match outcome {
        RunOutcome::Completed(_) => {
            let content = record
                .result
                .as_ref()
                .and_then(|r| r.extracted.as_ref().map(|e| e.to_output_text()))
                .unwrap_or_default();
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "done",
                "content": content,
                "usage": usage,
                "latency_ms": record.updated_at_ms.saturating_sub(record.created_at_ms),
                "artifacts": record.artifacts,
            });
            let summary = if content.trim().is_empty() {
                format!("done (run_id={run_id})")
            } else {
                truncate_for_summary(&content, 200)
            };
            (
                AgentToolResult {
                    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                    tool: Some(TOOL_NAME.to_string()),
                    cmd_name: None,
                    status: AgentToolStatus::Success,
                    task_id: None,
                    pending_reason: None,
                    check_after: None,
                    estimated_wait: None,
                    title: format!("{TOOL_NAME} => done"),
                    summary,
                    details,
                    cmd_args: None,
                    return_code: Some(0),
                    partial_output: None,
                    output: Some(content),
                },
                CLI_EXIT_SUCCESS,
            )
        }
        RunOutcome::Paused(_) | RunOutcome::Interrupted(_) => {
            let reason = record
                .last_error
                .as_ref()
                .map(|e| e.message.clone())
                .or_else(|| record.interrupt_reason.clone())
                .unwrap_or_else(|| "paused".to_string());
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": record.status.as_str(),
                "reason": reason,
                "usage": usage,
                "resume": record.resume_command(),
            });
            (
                AgentToolResult {
                    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                    tool: Some(TOOL_NAME.to_string()),
                    cmd_name: None,
                    status: AgentToolStatus::Pending,
                    task_id: Some(run_id.to_string()),
                    pending_reason: Some(AgentToolPendingReason::LongRunning),
                    check_after: None,
                    estimated_wait: None,
                    title: format!("{TOOL_NAME} => {}", record.status.as_str()),
                    summary: format!("run {}: {reason}", record.status.label()),
                    details,
                    cmd_args: None,
                    return_code: None,
                    partial_output: None,
                    output: None,
                },
                CLI_EXIT_SUCCESS,
            )
        }
        RunOutcome::LimitReached(_) | RunOutcome::Failed(_) => {
            let reason = record
                .limit_reason
                .clone()
                .or_else(|| record.last_error.as_ref().map(|e| e.message.clone()))
                .unwrap_or_else(|| record.status.label().to_string());
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": record.status.as_str(),
                "error": reason,
                "usage": usage,
            });
            (
                AgentToolResult {
                    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                    tool: Some(TOOL_NAME.to_string()),
                    cmd_name: None,
                    status: AgentToolStatus::Error,
                    task_id: None,
                    pending_reason: None,
                    check_after: None,
                    estimated_wait: None,
                    title: format!("{TOOL_NAME} => {}", record.status.as_str()),
                    summary: format!("{}: {reason}", record.status.label()),
                    details,
                    cmd_args: None,
                    return_code: None,
                    partial_output: None,
                    output: None,
                },
                CLI_EXIT_ERROR,
            )
        }
    }
}

fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push_str("...");
    out
}

fn build_error_result(work_dir: Option<&Path>, opts: &CliOpts, message: &str) -> AgentToolResult {
    let mut details = serde_json::Map::new();
    if let Some(dir) = work_dir {
        details.insert("work_dir".into(), Value::String(dir.display().to_string()));
    }
    details.insert(
        "description".into(),
        Value::String(opts.description.clone()),
    );
    details.insert("error".into(), Value::String(message.to_string()));
    AgentToolResult {
        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
        tool: Some(TOOL_NAME.to_string()),
        cmd_name: None,
        status: AgentToolStatus::Error,
        task_id: None,
        pending_reason: None,
        check_after: None,
        estimated_wait: None,
        title: format!("{TOOL_NAME} => error"),
        summary: message.to_string(),
        details: Value::Object(details),
        cmd_args: None,
        return_code: None,
        partial_output: None,
        output: None,
    }
}

// =========================================================================
// CLI 参数解析
// =========================================================================

const USAGE: &str = r#"Usage: agent_tool llm_explore --description <text> --prompt <text> [options]

Required:
  --description <text>   Free-form task description (worklog only, not in prompt)
  --prompt <text>        User instruction handed to the LLM as the user message

Options:
  --root-dir <path>      Exploration root directory (default: PWD). The LLM's
                         tools (read_file/write_file/edit_file/exec) work here.
  --work-dir <path>      Runs directory for the xllm run record and snapshots.
                         Default: $TMPDIR/llm_explore-<ts>-<sanitized-description>.
  --model <alias>        AICC model alias (default: llm.summary).
  -h, --help             Show this help.
"#;

#[derive(Debug)]
struct CliOpts {
    description: String,
    prompt: String,
    root_dir: Option<PathBuf>,
    work_dir: Option<PathBuf>,
    model: Option<String>,
}

enum ParseError {
    Help,
    Bad(String),
}

impl CliOpts {
    fn parse(args: &[String]) -> Result<Self, ParseError> {
        let mut description: Option<String> = None;
        let mut prompt: Option<String> = None;
        let mut root_dir: Option<PathBuf> = None;
        let mut work_dir: Option<PathBuf> = None;
        let mut model: Option<String> = None;

        let mut idx = 0;
        while idx < args.len() {
            let tok = args[idx].as_str();
            match tok {
                "-h" | "--help" => return Err(ParseError::Help),
                "--description" => {
                    description = Some(next_value(args, &mut idx, "--description")?);
                }
                "--prompt" => prompt = Some(next_value(args, &mut idx, "--prompt")?),
                "--root-dir" => {
                    root_dir = Some(PathBuf::from(next_value(args, &mut idx, "--root-dir")?));
                }
                "--work-dir" => {
                    work_dir = Some(PathBuf::from(next_value(args, &mut idx, "--work-dir")?));
                }
                "--model" => model = Some(next_value(args, &mut idx, "--model")?),
                other => {
                    return Err(ParseError::Bad(format!("unknown flag `{other}`")));
                }
            }
            idx += 1;
        }

        let description =
            description.ok_or_else(|| ParseError::Bad("missing --description".into()))?;
        let prompt = prompt.ok_or_else(|| ParseError::Bad("missing --prompt".into()))?;
        if description.trim().is_empty() {
            return Err(ParseError::Bad("--description must not be empty".into()));
        }
        if prompt.trim().is_empty() {
            return Err(ParseError::Bad("--prompt must not be empty".into()));
        }

        Ok(Self {
            description,
            prompt,
            root_dir,
            work_dir,
            model,
        })
    }
}

fn next_value(args: &[String], idx: &mut usize, flag: &str) -> Result<String, ParseError> {
    *idx += 1;
    args.get(*idx)
        .cloned()
        .ok_or_else(|| ParseError::Bad(format!("missing value for {flag}")))
}
