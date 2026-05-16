//! `agent_tool llm_explore` —— 基于 LLM 的代码库探索工具。
//!
//! ## 设计
//!
//! 调用方给两段自然语言:
//! - `--description` (objective): 写进 worklog,不进 prompt。
//! - `--prompt`     (user content): 实际给 LLM 看的任务说明。
//!
//! 我们在一个本地目录上起一个 [`LocalLLMContext`],预装好 Read / Glob /
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
use llm_context::{
    ContextOutput, LLMContextOutcome, LlmClient, ModelPolicy, ToolMode, ToolPolicy,
};
use serde_json::{json, Value};

use crate::run_local_llm::{
    acquire_aicc_client, ensure_buckyos_runtime, AiccLlmClient, KeepTailCompressor,
};
use crate::{
    cli_error_result, render_cli_output, AgentToolError, AgentToolPendingReason,
    AgentToolResult, AgentToolStatus, LocalLLMContext, OneShotRequest,
    AGENT_TOOL_PROTOCOL_VERSION, CLI_EXIT_ERROR, CLI_EXIT_SUCCESS, CLI_EXIT_USAGE,
};

const TOOL_NAME: &str = "llm_explore";
const DEFAULT_MODEL_ALIAS: &str = "llm.summary";
const DEFAULT_MAX_ROUNDS: u32 = 16;

/// 钉在每个 llm_explore run 上的 system prompt。来自 agent_tool 仓库内
/// `agent_tool/src/llm_explore.rs` 的设计注释。
const SYSTEM_PROMPT: &str = "\
You are a file search specialist. You excel at thoroughly navigating and exploring codebases.

Your strengths:
- Rapidly finding files using glob patterns
- Searching code and text with powerful regex patterns
- Reading and analyzing file contents

Guidelines:
- Use Glob/find for broad file pattern matching
- Use Grep/grep for searching file contents with regex
- Use Read when you know the specific file path you need to read
- Use exec_bash ONLY for read-only operations: ls, git status, git log, git diff, find, cat, head, tail
- NEVER use exec_bash for mkdir, touch, rm, cp, mv, git add, git commit, npm install, pip install, or file modifications
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
            let err = AgentToolError::InvalidArgs(msg);
            emit_result(&cli_error_result(Some(TOOL_NAME), &err));
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
    let work_dir = match prepare_work_dir(&opts) {
        Ok(dir) => dir,
        Err(err) => {
            return (
                build_error_result(None, &opts, &format!("prepare work dir failed: {err}")),
                CLI_EXIT_ERROR,
            );
        }
    };

    // 1) BuckyOS runtime + AICC client.
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
    let aicc = match acquire_aicc_client().await {
        Ok(c) => c,
        Err(err) => {
            return (
                build_error_result(
                    Some(&work_dir),
                    &opts,
                    &format!("acquire aicc client failed: {err}"),
                ),
                CLI_EXIT_ERROR,
            );
        }
    };
    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new(aicc));

    // 2) OneShotRequest:system prompt + 用户 prompt + model + tools=All。
    let request = build_request(&opts);

    // 3) 起 / resume LocalLLMContext。同一个 work_dir 多次跑同一组 args 时
    //    会自动 resume(基于 semantic_hash);第二次给不一样的 prompt 会
    //    被拒绝,这是上层 `resume_or_new` 的设计意图,不在本工具里 hack 掉。
    let mut ctx = match LocalLLMContext::resume_or_new(work_dir.clone(), request, llm) {
        Ok(c) => c,
        Err(err) => {
            return (
                build_error_result(
                    Some(&work_dir),
                    &opts,
                    &format!("LocalLLMContext init failed: {err}"),
                ),
                CLI_EXIT_ERROR,
            );
        }
    };
    let run_id = ctx.run_id().to_string();
    eprintln!(
        "llm_explore: work_dir={} run_id={}",
        work_dir.display(),
        run_id
    );

    // 4) drive_to_terminal。压缩策略复用 run_local_llm 里那个简单的
    //    "保留 system + 最近 N 条"——够用,不引二级 LLM 调用。
    let compressor = KeepTailCompressor::new(8);
    let outcome = match ctx.drive_to_terminal(&compressor).await {
        Ok(o) => o,
        Err(err) => {
            return (
                build_error_result(
                    Some(&work_dir),
                    &opts,
                    &format!("drive_to_terminal failed: {err}"),
                ),
                CLI_EXIT_ERROR,
            );
        }
    };

    // 5) 把 outcome 翻译成 AgentToolResult。
    build_outcome_result(&work_dir, &run_id, &opts, outcome)
}

// =========================================================================
// work_dir 处理
// =========================================================================

/// 准备 work_dir。如果调用方给了 `--root-dir`,在 `work_dir/workspace`
/// 处创建一个指向 root_dir 的符号链接,让 LocalLLMContext 的工具集
/// (Read/Glob/Grep/exec_bash)直接 sandbox 在那个目录下。
///
/// 选符号链接而不是改 LocalLLMContext 的接口:LocalLLMContext 的工作目录
/// 布局有自己的 invariants(state.json / snapshots / .lock 都靠固定的
/// `<dir>/workspace` 子路径),从外面 inject 一个 workspace 路径破坏面太大;
/// `ensure_dir_layout` 调 `create_dir_all`,对一个已经存在并指向目录的
/// symlink 会返回 Ok,所以预先建好符号链接就能"借壳"完成接管。
fn prepare_work_dir(opts: &CliOpts) -> std::io::Result<PathBuf> {
    let work_dir = match opts.work_dir.as_ref() {
        Some(p) => p.clone(),
        None => default_work_dir(&opts.description),
    };
    std::fs::create_dir_all(&work_dir)?;

    let workspace = work_dir.join("workspace");
    let root = match opts.root_dir.as_ref() {
        Some(p) => p.clone(),
        None => std::env::current_dir()?,
    };
    let root = root.canonicalize().unwrap_or(root);

    // 如果 workspace 已经存在且指向预期目录,不动它(resume 场景)。
    let workspace_meta = std::fs::symlink_metadata(&workspace).ok();
    match workspace_meta {
        None => {
            #[cfg(unix)]
            {
                std::os::unix::fs::symlink(&root, &workspace)?;
            }
            #[cfg(not(unix))]
            {
                // Windows fallback:不建 symlink,把 root 直接当 workspace。
                // 由于 LocalLLMContext 自己会 create_dir_all,这里就让它建空目录,
                // 用户会感知到"工具看到的是空 workspace"——比静默失败好。
                let _ = &root;
                std::fs::create_dir_all(&workspace)?;
            }
        }
        Some(meta) if meta.file_type().is_symlink() => {
            // 已经是 symlink,假定之前一次的 run 留下来的,沿用。
        }
        Some(_) => {
            // 已经是真实目录(可能是上一次没传 --root-dir 起的 run),沿用。
        }
    }
    Ok(work_dir)
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
// OneShotRequest 构造
// =========================================================================

fn build_request(opts: &CliOpts) -> OneShotRequest {
    let input = vec![
        AiMessage::text(AiRole::System, SYSTEM_PROMPT),
        AiMessage::text(AiRole::User, opts.prompt.clone()),
    ];
    let mut req = OneShotRequest::new(opts.description.clone(), input);
    req.model_policy = Some(ModelPolicy {
        preferred: opts
            .model
            .clone()
            .unwrap_or_else(|| DEFAULT_MODEL_ALIAS.to_string()),
        fallbacks: Vec::new(),
        temperature: None,
        max_completion_tokens: None,
        provider_options: None,
    });
    req.tool_policy = Some(ToolPolicy {
        mode: ToolMode::All,
        max_rounds: DEFAULT_MAX_ROUNDS,
        ..ToolPolicy::default()
    });
    req
}

// =========================================================================
// outcome → AgentToolResult
// =========================================================================

fn build_outcome_result(
    work_dir: &Path,
    run_id: &str,
    opts: &CliOpts,
    outcome: LLMContextOutcome,
) -> (AgentToolResult, i32) {
    let work_dir_str = work_dir.display().to_string();
    match outcome {
        LLMContextOutcome::Done {
            output,
            response,
            trace,
            usage,
            ..
        } => {
            let content = output_to_text(&output);
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "done",
                "content": content,
                "usage": usage,
                "latency_ms": trace.latency_ms,
                "llm_task_ids": trace.llm_task_ids,
                "response": response,
            });
            let summary = if content.trim().is_empty() {
                format!("done (run_id={run_id})")
            } else {
                truncate_for_summary(&content, 200)
            };
            let result = AgentToolResult {
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
            };
            (result, CLI_EXIT_SUCCESS)
        }
        LLMContextOutcome::PendingTool { pending, .. } => {
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "pending_tool",
                "pending": pending,
            });
            let result = AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_NAME.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Pending,
                task_id: Some(run_id.to_string()),
                pending_reason: Some(AgentToolPendingReason::LongRunning),
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_NAME} => pending_tool"),
                summary: format!("pending {} tool call(s)", pending.len()),
                details,
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            };
            (result, CLI_EXIT_SUCCESS)
        }
        LLMContextOutcome::BudgetExhausted {
            which,
            partial,
            usage,
        } => {
            let partial_text = partial.as_ref().map(output_to_text);
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "budget_exhausted",
                "which": which,
                "usage": usage,
                "partial": partial_text,
            });
            let result = AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_NAME.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Error,
                task_id: None,
                pending_reason: None,
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_NAME} => budget_exhausted"),
                summary: format!("budget exhausted ({:?})", which),
                details,
                cmd_args: None,
                return_code: None,
                partial_output: partial_text,
                output: None,
            };
            (result, CLI_EXIT_ERROR)
        }
        LLMContextOutcome::Error { error, usage } => {
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "error",
                "error": format!("{error}"),
                "usage": usage,
            });
            let result = AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_NAME.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Error,
                task_id: None,
                pending_reason: None,
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_NAME} => error"),
                summary: format!("llm error: {error}"),
                details,
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            };
            (result, CLI_EXIT_ERROR)
        }
        LLMContextOutcome::ContextLimitReached { which, .. } => {
            // drive_to_terminal 内部应当已经消化掉 ContextLimitReached;
            // 跑到这里说明 compressor 链路坏了或被 caller 用 step() 显式 surface。
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "context_limit_reached",
                "which": format!("{:?}", which),
            });
            let result = AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_NAME.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Error,
                task_id: None,
                pending_reason: None,
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_NAME} => context_limit_reached"),
                summary: format!("context limit surfaced unexpectedly: {:?}", which),
                details,
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            };
            (result, CLI_EXIT_ERROR)
        }
        LLMContextOutcome::Interrupted {
            reason,
            usage,
            abort,
            ..
        } => {
            // §3.13:run 被外部 interrupt handle 抢占。run id 仍有效 —— caller
            // 可以重新打开 LocalLLMContext 走 ResumeFromMidRun 继续推进。
            // CLI 层把它表达成 pending,任务可由外部重启。
            let details = json!({
                "work_dir": work_dir_str,
                "run_id": run_id,
                "description": opts.description,
                "outcome": "interrupted",
                "reason": reason,
                "usage": usage,
                "abort": abort,
            });
            let result = AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_NAME.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Pending,
                task_id: Some(run_id.to_string()),
                pending_reason: Some(AgentToolPendingReason::LongRunning),
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_NAME} => interrupted"),
                summary: format!("inference interrupted: {reason}"),
                details,
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            };
            (result, CLI_EXIT_SUCCESS)
        }
    }
}

fn output_to_text(output: &ContextOutput) -> String {
    match output {
        ContextOutput::Text { content } => content.clone(),
        ContextOutput::Json { content } => serde_json::to_string_pretty(content)
            .unwrap_or_else(|_| content.to_string()),
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
    details.insert("description".into(), Value::String(opts.description.clone()));
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
  --root-dir <path>      Exploration root directory (default: PWD).
                         Symlinked into <work_dir>/workspace so the LLM's tools
                         (Read/Glob/Grep/exec_bash) sandbox in this tree.
  --work-dir <path>      LocalLLMContext working directory; persists snapshots,
                         worklog, and run state. Default: $TMPDIR/llm_explore-
                         <ts>-<sanitized-description>.
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

        let description = description
            .ok_or_else(|| ParseError::Bad("missing --description".into()))?;
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
