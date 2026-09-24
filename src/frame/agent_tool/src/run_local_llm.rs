//! `xllm` 命令行入口（`agent_tool xllm ...`；`run_local_llm` 为兼容别名）。
//!
//! 这是 `product/xllm/PRD.md` 命令面的 Rust 参考实现：所有任务语义都由
//! [`crate::local_llm_context`] 提供，这里只做 argv / stdin → SDK 请求，
//! SDK 结果 → stdout / stderr / 退出码 的映射。
//!
//! ## 退出码
//!
//! | 码 | 含义 |
//! | --- | --- |
//! | 0 | 任务正常完成且结果已交付；查询命令成功读取记录 |
//! | 1 | 任务终态失败（不可恢复错误 / 达到限制），或查询目标不存在、记录损坏 |
//! | 2 | 参数、配置、输入预检错误；尚未建立 Run |
//! | 3 | 可恢复错误：本次命令失败，任务已暂停，可 resume |
//! | 4 | 用户中断，进度已保存 |
//! | 5 | 任务已完成但 `--output` 写入失败（可用 `xllm result` 重新导出） |
//! | 6 | 任务已完成但结果提取 / `--json` 校验失败（原文已保存） |

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use buckyos_api::{LlmResponseFormat, TaskError};
use serde_json::Value;

use crate::local_llm_context::{
    build_result_view, export_result, list_runs, load_run, ExtractedValue, LoopModel, ProviderKind,
    ResultFormat, ResumeLimits, ResumeStart, RunEvent, RunLogLevel, RunObserver, RunOutcome,
    RunPhase, RunRecord, RunStatus, RunStore, RunSummary, TaskInput, TaskOverrides, XllmDeps,
    XllmError, XllmRun, XllmTask, DEFAULT_RUNS_DIR,
};
pub use crate::local_llm_context::{ensure_buckyos_runtime, AiccLlmClient};

pub const EXIT_OK: i32 = 0;
pub const EXIT_TASK_FAILED: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_PAUSED: i32 = 3;
pub const EXIT_INTERRUPTED: i32 = 4;
pub const EXIT_OUTPUT_FAILED: i32 = 5;
pub const EXIT_RESULT_INVALID: i32 = 6;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = r#"xllm — one-shot LLM task runner (AICC ↔ Agent boundary)

Usage:
  xllm "question" [options]               run a new task with a clean context
  xllm --select <group> ["question"]      run a prompt group's default task
  xllm "task one" | xllm "task two"       chain: stdin becomes material / request
  xllm --image ./photo.png "what is it?"  images (repeatable), --file for text
  xllm --tools "index the docs dir"       enable tools (read/write/edit/exec)
  xllm list [--limit N]                   recent runs of the working directory
  xllm status [--run <id>]                run status, effective config, resumability
  xllm result [--run <id>]                re-export a saved result (no model call)
  xllm --resume [--run <id>]              continue an interrupted / paused run

Input:
  "question" | --user <text>   task request (one of them, once)
  --system <text>              full custom business prompt (runtime protocol kept)
  --select <name>              choose a prompt group from .llm_context
  --file <path>                text material (repeatable, order kept)
  --image <path|url>           image material (png/jpeg/webp; repeatable)
  --input-file <path>          structured input: JSON array of messages
  --dir <path>                 working directory (default: cwd)
  --runs-dir <path>            runs directory (default: .llm_context or ~/.xllm/runs)

Model:
  --provider buckyos|openai    --model <name>    --file-model <name>
  --loop-model function_call|behavior
  --tools | --no-tools         override every file-level tool switch

Limits:
  --max-tokens <n>  --max-rounds <n>  --timeout <secs>  --llm-timeout <secs>

Output:
  --result-format raw|result.<path>   extract from the final response
  --json                              require the extracted result to be JSON
  --format text|json                  plain answer or structured CLI result
  --output <path>                     save the output instead of printing it
  --run-logs debug|info|warn|result   stderr verbosity

Other:
  --run <id>  (with --resume / status / result)   --limit <n> (list)
  --help, --version

Exit codes: 0 done · 1 failed/limit · 2 usage/config · 3 paused (resumable) ·
4 interrupted · 5 output write failed · 6 result invalid
"#;

const SHORT_USAGE: &str = r#"usage: xllm "question" [options]
       xllm --select <group> | xllm list | xllm status | xllm result | xllm --resume
No task request: pass a question, --user, a group with default_user, or pipe input.
Run `xllm --help` for all options."#;

// =========================================================================
// 入口
// =========================================================================

pub async fn run_subcommand(args: Vec<String>) -> i32 {
    let opts = match CliOpts::parse(&args) {
        Ok(opts) => opts,
        Err(ParseError::Help) => {
            print!("{USAGE}");
            return EXIT_OK;
        }
        Err(ParseError::Version) => {
            println!("xllm {VERSION}");
            return EXIT_OK;
        }
        Err(ParseError::Bad(msg)) => {
            eprintln!("xllm: error: {msg}\n\n{SHORT_USAGE}");
            return EXIT_USAGE;
        }
    };
    match opts.command {
        Command::List => run_list(&opts).await,
        Command::Status => run_status(&opts).await,
        Command::Result => run_result(&opts).await,
        Command::Resume => run_resume(opts).await,
        Command::New => run_new(opts).await,
    }
}

// =========================================================================
// 参数
// =========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Command {
    New,
    Resume,
    List,
    Status,
    Result,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CliFormat {
    Text,
    Json,
}

#[derive(Debug, Clone)]
struct CliOpts {
    command: Command,
    question: Option<String>,
    user: Option<String>,
    system: Option<String>,
    select: Option<String>,
    files: Vec<PathBuf>,
    images: Vec<String>,
    attachments: Vec<crate::local_llm_context::Attachment>,
    input_file: Option<PathBuf>,
    dir: Option<PathBuf>,
    runs_dir: Option<PathBuf>,
    provider: Option<ProviderKind>,
    model: Option<String>,
    file_model: Option<String>,
    loop_model: Option<LoopModel>,
    tools: Option<bool>,
    run: Option<String>,
    max_tokens: Option<u32>,
    max_rounds: Option<u32>,
    timeout: Option<u64>,
    llm_timeout: Option<u64>,
    run_logs: Option<RunLogLevel>,
    result_format: Option<ResultFormat>,
    json: bool,
    format: CliFormat,
    output: Option<PathBuf>,
    limit: usize,
}

enum ParseError {
    Help,
    Version,
    Bad(String),
}

fn next_value(args: &[String], idx: &mut usize, flag: &str) -> Result<String, ParseError> {
    *idx += 1;
    args.get(*idx)
        .cloned()
        .ok_or_else(|| ParseError::Bad(format!("missing value for {flag}")))
}

fn parse_num<T: std::str::FromStr>(v: &str, flag: &str) -> Result<T, ParseError> {
    v.parse::<T>()
        .map_err(|_| ParseError::Bad(format!("invalid value for {flag}: `{v}`")))
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), ParseError> {
    if slot.is_some() {
        return Err(ParseError::Bad(format!("{flag} may be given only once")));
    }
    *slot = Some(value);
    Ok(())
}

impl CliOpts {
    fn parse(args: &[String]) -> Result<Self, ParseError> {
        use crate::local_llm_context::Attachment;
        let mut o = CliOpts {
            command: Command::New,
            question: None,
            user: None,
            system: None,
            select: None,
            files: Vec::new(),
            images: Vec::new(),
            attachments: Vec::new(),
            input_file: None,
            dir: None,
            runs_dir: None,
            provider: None,
            model: None,
            file_model: None,
            loop_model: None,
            tools: None,
            run: None,
            max_tokens: None,
            max_rounds: None,
            timeout: None,
            llm_timeout: None,
            run_logs: None,
            result_format: None,
            json: false,
            format: CliFormat::Text,
            output: None,
            limit: 20,
        };
        let mut resume = false;
        let mut idx = 0;
        let mut positional_seen = false;
        while idx < args.len() {
            let tok = args[idx].as_str();
            match tok {
                "-h" | "--help" => return Err(ParseError::Help),
                "--version" | "-V" => return Err(ParseError::Version),
                "list" | "status" | "result" if idx == 0 => {
                    o.command = match tok {
                        "list" => Command::List,
                        "status" => Command::Status,
                        _ => Command::Result,
                    };
                }
                "--user" => {
                    let v = next_value(args, &mut idx, "--user")?;
                    set_once(&mut o.user, v, "--user")?;
                }
                "--system" => {
                    let v = next_value(args, &mut idx, "--system")?;
                    set_once(&mut o.system, v, "--system")?;
                }
                "--select" => {
                    let v = next_value(args, &mut idx, "--select")?;
                    set_once(&mut o.select, v, "--select")?;
                }
                "--file" => {
                    let v = PathBuf::from(next_value(args, &mut idx, "--file")?);
                    o.files.push(v.clone());
                    o.attachments.push(Attachment::File { path: v });
                }
                "--image" => {
                    let v = next_value(args, &mut idx, "--image")?;
                    o.images.push(v.clone());
                    o.attachments.push(Attachment::Image { source: v });
                }
                "--input-file" => {
                    let v = PathBuf::from(next_value(args, &mut idx, "--input-file")?);
                    set_once(&mut o.input_file, v, "--input-file")?;
                }
                "--dir" => {
                    let v = PathBuf::from(next_value(args, &mut idx, "--dir")?);
                    set_once(&mut o.dir, v, "--dir")?;
                }
                "--runs-dir" => {
                    let v = PathBuf::from(next_value(args, &mut idx, "--runs-dir")?);
                    set_once(&mut o.runs_dir, v, "--runs-dir")?;
                }
                "--provider" => {
                    let v = next_value(args, &mut idx, "--provider")?;
                    let k = ProviderKind::parse(&v).ok_or_else(|| {
                        ParseError::Bad(format!("--provider must be buckyos or openai, got `{v}`"))
                    })?;
                    set_once(&mut o.provider, k, "--provider")?;
                }
                "--model" => {
                    let v = next_value(args, &mut idx, "--model")?;
                    set_once(&mut o.model, v, "--model")?;
                }
                "--file-model" => {
                    let v = next_value(args, &mut idx, "--file-model")?;
                    set_once(&mut o.file_model, v, "--file-model")?;
                }
                "--loop-model" => {
                    let v = next_value(args, &mut idx, "--loop-model")?;
                    let l = LoopModel::parse(&v).ok_or_else(|| {
                        ParseError::Bad(format!(
                            "--loop-model must be function_call or behavior, got `{v}`"
                        ))
                    })?;
                    set_once(&mut o.loop_model, l, "--loop-model")?;
                }
                "--tools" => {
                    if o.tools == Some(false) {
                        return Err(ParseError::Bad(
                            "--tools and --no-tools are mutually exclusive".into(),
                        ));
                    }
                    o.tools = Some(true);
                }
                "--no-tools" => {
                    if o.tools == Some(true) {
                        return Err(ParseError::Bad(
                            "--tools and --no-tools are mutually exclusive".into(),
                        ));
                    }
                    o.tools = Some(false);
                }
                "--resume" => resume = true,
                "--run" => {
                    let v = next_value(args, &mut idx, "--run")?;
                    set_once(&mut o.run, v, "--run")?;
                }
                "--max-tokens" => {
                    let v = next_value(args, &mut idx, "--max-tokens")?;
                    set_once(
                        &mut o.max_tokens,
                        parse_num(&v, "--max-tokens")?,
                        "--max-tokens",
                    )?;
                }
                "--max-rounds" => {
                    let v = next_value(args, &mut idx, "--max-rounds")?;
                    set_once(
                        &mut o.max_rounds,
                        parse_num(&v, "--max-rounds")?,
                        "--max-rounds",
                    )?;
                }
                "--timeout" => {
                    let v = next_value(args, &mut idx, "--timeout")?;
                    set_once(&mut o.timeout, parse_num(&v, "--timeout")?, "--timeout")?;
                }
                "--llm-timeout" => {
                    let v = next_value(args, &mut idx, "--llm-timeout")?;
                    set_once(
                        &mut o.llm_timeout,
                        parse_num(&v, "--llm-timeout")?,
                        "--llm-timeout",
                    )?;
                }
                "--run-logs" => {
                    let v = next_value(args, &mut idx, "--run-logs")?;
                    let l = RunLogLevel::parse(&v).ok_or_else(|| {
                        ParseError::Bad(format!(
                            "--run-logs must be debug, info, warn or result, got `{v}`"
                        ))
                    })?;
                    set_once(&mut o.run_logs, l, "--run-logs")?;
                }
                "--result-format" => {
                    let v = next_value(args, &mut idx, "--result-format")?;
                    let f = ResultFormat::parse(&v).map_err(|e| ParseError::Bad(e.to_string()))?;
                    set_once(&mut o.result_format, f, "--result-format")?;
                }
                "--json" => o.json = true,
                "--format" => {
                    let v = next_value(args, &mut idx, "--format")?;
                    o.format = match v.as_str() {
                        "text" => CliFormat::Text,
                        "json" => CliFormat::Json,
                        other => {
                            return Err(ParseError::Bad(format!(
                                "--format must be text or json, got `{other}`"
                            )))
                        }
                    };
                }
                "--output" => {
                    let v = PathBuf::from(next_value(args, &mut idx, "--output")?);
                    set_once(&mut o.output, v, "--output")?;
                }
                "--limit" => {
                    let v = next_value(args, &mut idx, "--limit")?;
                    o.limit = parse_num(&v, "--limit")?;
                }
                other if other.starts_with('-') && other.len() > 1 => {
                    return Err(ParseError::Bad(format!("unknown flag `{other}`")));
                }
                positional => {
                    if positional_seen {
                        return Err(ParseError::Bad(format!(
                            "unexpected extra argument `{positional}`; quote the question as one argument"
                        )));
                    }
                    positional_seen = true;
                    o.question = Some(positional.to_string());
                }
            }
            idx += 1;
        }
        if resume {
            o.command = Command::Resume;
        }
        if o.question.is_some() && o.user.is_some() {
            return Err(ParseError::Bad(
                "a positional question and --user are mutually exclusive".into(),
            ));
        }
        if o.select.is_some() && o.system.is_some() {
            return Err(ParseError::Bad(
                "--select and --system are mutually exclusive".into(),
            ));
        }
        match o.command {
            Command::New => {
                if o.run.is_some() {
                    return Err(ParseError::Bad(
                        "--run only applies to --resume / status / result; a new task gets its own run id".into(),
                    ));
                }
            }
            Command::Resume => {
                if o.question.is_some()
                    || o.user.is_some()
                    || o.system.is_some()
                    || o.select.is_some()
                    || !o.attachments.is_empty()
                    || o.input_file.is_some()
                {
                    return Err(ParseError::Bad(
                        "--resume continues the original task; it does not accept a question, --user, --system, --select, --input-file or attachments".into(),
                    ));
                }
            }
            Command::List | Command::Status | Command::Result => {
                if o.question.is_some() || o.user.is_some() {
                    return Err(ParseError::Bad(format!(
                        "`{}` is a subcommand; to ask that as a question use --user",
                        match o.command {
                            Command::List => "list",
                            Command::Status => "status",
                            _ => "result",
                        }
                    )));
                }
            }
        }
        Ok(o)
    }

    fn workdir(&self) -> Result<PathBuf, String> {
        match &self.dir {
            Some(d) => {
                let base = std::env::current_dir().map_err(|e| e.to_string())?;
                let p = if d.is_absolute() {
                    d.clone()
                } else {
                    base.join(d)
                };
                p.canonicalize()
                    .map_err(|e| format!("--dir {}: {e}", d.display()))
            }
            None => std::env::current_dir().map_err(|e| format!("cannot resolve cwd: {e}")),
        }
    }

    fn overrides(&self) -> TaskOverrides {
        TaskOverrides {
            provider: self.provider,
            model: self.model.clone(),
            file_model: self.file_model.clone(),
            loop_model: self.loop_model,
            tools: self.tools,
            select: self.select.clone(),
            system: self.system.clone(),
            max_tokens: self.max_tokens,
            max_rounds: self.max_rounds,
            timeout_secs: self.timeout,
            llm_timeout_secs: self.llm_timeout,
            run_logs: self.run_logs,
            result_format: self.result_format.clone(),
            runs_dir: self.runs_dir.clone(),
            json: self.json,
            json_schema: None,
            disable_capabilities: Vec::new(),
        }
    }

    fn resume_limits(&self) -> ResumeLimits {
        ResumeLimits {
            max_tokens: self.max_tokens,
            max_rounds: self.max_rounds,
            timeout_secs: self.timeout,
            llm_timeout_secs: self.llm_timeout,
        }
    }

    fn log_level(&self) -> RunLogLevel {
        self.run_logs.unwrap_or(RunLogLevel::Info)
    }
}

// =========================================================================
// stderr 观察者（F09）
// =========================================================================

struct StderrObserver {
    level: RunLogLevel,
}

impl RunObserver for StderrObserver {
    fn on_event(&self, run_id: &str, event: RunEvent) {
        let (min, line) = match event {
            RunEvent::Phase { phase, detail } => (
                // 等待模型的阶段由 LlmStarted 事件单独播报，避免重复。
                if phase == RunPhase::WaitingModel {
                    RunLogLevel::Debug
                } else {
                    RunLogLevel::Info
                },
                if detail.is_empty() {
                    format!("[{run_id}] {}", phase.label())
                } else {
                    format!("[{run_id}] {}: {detail}", phase.label())
                },
            ),
            RunEvent::LlmStarted { model } => {
                (RunLogLevel::Info, format!("[{run_id}] 等待模型 {model} …"))
            }
            RunEvent::LlmFinished { ok, elapsed_ms } => (
                RunLogLevel::Info,
                format!(
                    "[{run_id}] 模型返回 ({}, {:.1}s)",
                    if ok { "ok" } else { "failed" },
                    elapsed_ms as f64 / 1000.0
                ),
            ),
            RunEvent::ToolStarted { name, call_id } => (
                RunLogLevel::Info,
                format!("[{run_id}] 执行工具 {name} ({call_id})"),
            ),
            RunEvent::ToolFinished {
                name,
                call_id,
                ok,
                duration_ms,
            } => (
                RunLogLevel::Info,
                format!(
                    "[{run_id}] 工具 {name} ({call_id}) {} ({duration_ms} ms)",
                    if ok { "完成" } else { "失败" }
                ),
            ),
            RunEvent::Warning(msg) => (RunLogLevel::Warn, format!("[{run_id}] warning: {msg}")),
            RunEvent::Debug(msg) => (RunLogLevel::Debug, format!("[{run_id}] debug: {msg}")),
        };
        // level 顺序：Debug < Info < Warn < Result；事件的 min 级别小于等于配置才显示。
        let show = match self.level {
            RunLogLevel::Debug => true,
            RunLogLevel::Info => min >= RunLogLevel::Info,
            RunLogLevel::Warn => min >= RunLogLevel::Warn,
            RunLogLevel::Result => false,
        };
        if show {
            eprintln!("{line}");
        }
    }
}

fn diag(level: RunLogLevel, msg: &str) {
    if level != RunLogLevel::Result {
        eprintln!("xllm: {msg}");
    }
}

// =========================================================================
// 新任务
// =========================================================================

/// 只有真正的管道 / 文件重定向才自动读到 EOF；终端、socket、`/dev/null`
/// 等一律视为“没有管道输入”，避免在没有上游的情况下挂起等待。
fn read_piped_stdin() -> Result<Option<String>, String> {
    let stdin = std::io::stdin();
    if stdin.is_terminal() {
        return Ok(None);
    }
    #[cfg(unix)]
    let (is_fifo, is_file) = {
        use std::os::unix::fs::FileTypeExt;
        match std::fs::metadata("/dev/stdin") {
            Ok(md) => (md.file_type().is_fifo(), md.file_type().is_file()),
            Err(_) => (false, false),
        }
    };
    #[cfg(not(unix))]
    let (is_fifo, is_file) = (true, false);
    if !is_fifo && !is_file {
        return Ok(None);
    }
    let mut buf = String::new();
    stdin
        .lock()
        .read_to_string(&mut buf)
        .map_err(|e| format!("cannot read stdin: {e}"))?;
    if buf.trim().is_empty() {
        if is_fifo {
            return Err(
                "stdin pipe delivered no usable input; check the upstream command and its exit status"
                    .into(),
            );
        }
        return Ok(None);
    }
    Ok(Some(buf))
}

fn read_structured_input(path: &Path) -> Result<Vec<buckyos_api::AiMessage>, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("--input-file {}: {e}", path.display()))?;
    serde_json::from_slice::<Vec<buckyos_api::AiMessage>>(&bytes).map_err(|e| {
        format!(
            "--input-file {}: expected a JSON array of messages: {e}",
            path.display()
        )
    })
}

fn exit_code_for_error(err: &XllmError) -> i32 {
    match err {
        XllmError::Config { .. }
        | XllmError::Input(_)
        | XllmError::Capability(_)
        | XllmError::Tools(_)
        | XllmError::Template { .. } => EXIT_USAGE,
        XllmError::RunNotFound { .. }
        | XllmError::RunTerminal { .. }
        | XllmError::NotResumable { .. }
        | XllmError::CorruptedRun { .. } => EXIT_TASK_FAILED,
        XllmError::RunBusy { .. } | XllmError::WorkdirBusy { .. } => EXIT_USAGE,
        XllmError::Storage(_) | XllmError::Io(_) => EXIT_TASK_FAILED,
        XllmError::Extract(_) => EXIT_RESULT_INVALID,
        XllmError::Compressor(_) | XllmError::Other(_) => EXIT_TASK_FAILED,
    }
}

fn store_for(opts: &CliOpts, workdir: &Path) -> RunStore {
    if let Some(p) = &opts.runs_dir {
        let base = std::env::current_dir().unwrap_or_else(|_| workdir.to_path_buf());
        return RunStore::disk(crate::local_llm_context::resolve_config_path(
            &p.display().to_string(),
            &base,
        ));
    }
    // 只读命令沿用目录配置里的 runs_dir，否则默认位置。
    let layers = crate::local_llm_context::load_config_layers(workdir).unwrap_or_default();
    let merged = crate::local_llm_context::merge_config_layers(&layers).unwrap_or_default();
    match merged.runs_dir {
        Some(crate::local_llm_context::RunsDirSetting::Path { path }) => RunStore::disk(path),
        Some(crate::local_llm_context::RunsDirSetting::Disabled) => RunStore::memory(),
        None => RunStore::disk(crate::local_llm_context::resolve_config_path(
            DEFAULT_RUNS_DIR,
            workdir,
        )),
    }
}

async fn run_new(opts: CliOpts) -> i32 {
    let level = opts.log_level();
    let workdir = match opts.workdir() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("xllm: error: {e}");
            return EXIT_USAGE;
        }
    };
    let mut input = TaskInput {
        user: opts.question.clone().or_else(|| opts.user.clone()),
        attachments: opts.attachments.clone(),
        stdin: None,
        structured: None,
        base_dir: std::env::current_dir().ok(),
    };
    if let Some(p) = &opts.input_file {
        match read_structured_input(p) {
            Ok(msgs) => input.structured = Some(msgs),
            Err(e) => {
                eprintln!("xllm: error: {e}");
                return EXIT_USAGE;
            }
        }
    } else {
        match read_piped_stdin() {
            Ok(s) => input.stdin = s,
            Err(e) => {
                eprintln!("xllm: error: {e}");
                return EXIT_USAGE;
            }
        }
    }
    let nothing_given = input.user.is_none()
        && input.stdin.is_none()
        && input.structured.is_none()
        && opts.select.is_none()
        && input.attachments.is_empty();

    let deps = XllmDeps::default().with_observer(Arc::new(StderrObserver { level }));
    let prepared = match XllmTask::prepare(&workdir, input, opts.overrides(), &deps).await {
        Ok(p) => p,
        Err(err) => {
            if nothing_given && matches!(err, XllmError::Input(_)) {
                eprintln!("{SHORT_USAGE}");
                return EXIT_USAGE;
            }
            eprintln!("xllm: error: {err}\n(no run was created)");
            return exit_code_for_error(&err);
        }
    };
    let mut run = match XllmRun::start(prepared, deps).await {
        Ok(r) => r,
        Err(err) => {
            eprintln!("xllm: error: {err}\n(no run was created)");
            return exit_code_for_error(&err);
        }
    };
    diag(
        level,
        &format!(
            "run {} started (workdir {}, runs {})",
            run.run_id(),
            workdir.display(),
            run.store()
                .runs_dir()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(memory)".into())
        ),
    );
    execute_and_deliver(&opts, &mut run).await
}

async fn execute_and_deliver(opts: &CliOpts, run: &mut XllmRun) -> i32 {
    let level = opts.log_level();
    let interrupter = run.interrupter();
    let run_id = run.run_id().to_string();
    let ctrl_c = tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            eprintln!("xllm: interrupt requested; saving progress of run {run_id} …");
            interrupter.interrupt("user interrupt (Ctrl-C)");
            if tokio::signal::ctrl_c().await.is_ok() {
                eprintln!("xllm: second interrupt; exiting without waiting");
                std::process::exit(EXIT_INTERRUPTED);
            }
        }
    });
    let outcome = run.execute().await;
    ctrl_c.abort();
    let outcome = match outcome {
        Ok(o) => o,
        Err(err) => {
            eprintln!(
                "xllm: error: {err}\nrun {} may still be resumable: {}",
                run.run_id(),
                run.record().resume_command()
            );
            return EXIT_TASK_FAILED;
        }
    };
    let store = run.store().clone();
    deliver_outcome(opts, &store, outcome, level)
}

fn deliver_outcome(
    opts: &CliOpts,
    store: &RunStore,
    outcome: RunOutcome,
    level: RunLogLevel,
) -> i32 {
    let record = outcome.record().clone();
    let summary = store.summarize(&record);
    match &outcome {
        RunOutcome::Completed(_) => {
            if let Some(u) = record.usage.total() {
                diag(
                    level,
                    &format!(
                        "run {} completed; tokens in={} out={} total={}",
                        record.run_id,
                        u.input_tokens
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".into()),
                        u.output_tokens
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".into()),
                        u.total_tokens
                            .map(|v| v.to_string())
                            .unwrap_or_else(|| "?".into())
                    ),
                );
            } else {
                diag(level, &format!("run {} completed", record.run_id));
            }
            if !record.artifacts.is_empty() {
                diag(
                    level,
                    &format!("artifacts: {}", record.artifacts.join(", ")),
                );
            }
            deliver_completed(opts, &record, &summary, None)
        }
        RunOutcome::Paused(_) => {
            let err = record.last_error.clone();
            eprintln!(
                "xllm: run {} paused ({}): {}",
                record.run_id,
                err.as_ref()
                    .map(|e| e.kind.as_str())
                    .unwrap_or("recoverable error"),
                err.as_ref().map(|e| e.message.as_str()).unwrap_or("")
            );
            if let Some(c) = err.as_ref().and_then(|e| e.condition.clone()) {
                eprintln!("xllm: to continue: {c}");
            }
            eprintln!("xllm: resume with: {}", record.resume_command());
            emit_json_if_requested(opts, &record, &summary);
            EXIT_PAUSED
        }
        RunOutcome::Interrupted(_) => {
            eprintln!(
                "xllm: run {} interrupted ({}); progress saved under {}",
                record.run_id,
                record.interrupt_reason.as_deref().unwrap_or("interrupted"),
                record.runs_dir.as_deref().unwrap_or("(memory)")
            );
            eprintln!("xllm: resume with: {}", record.resume_command());
            emit_json_if_requested(opts, &record, &summary);
            EXIT_INTERRUPTED
        }
        RunOutcome::Failed(_) => {
            eprintln!(
                "xllm: run {} failed: {}",
                record.run_id,
                record
                    .last_error
                    .as_ref()
                    .map(|e| e.message.as_str())
                    .unwrap_or("unrecoverable error")
            );
            emit_json_if_requested(opts, &record, &summary);
            EXIT_TASK_FAILED
        }
        RunOutcome::LimitReached(_) => {
            eprintln!(
                "xllm: run {} stopped: {}",
                record.run_id,
                record.limit_reason.as_deref().unwrap_or("limit reached")
            );
            if !record.artifacts.is_empty() {
                eprintln!("xllm: artifacts so far: {}", record.artifacts.join(", "));
            }
            emit_json_if_requested(opts, &record, &summary);
            EXIT_TASK_FAILED
        }
    }
}

fn emit_json_if_requested(opts: &CliOpts, record: &RunRecord, summary: &RunSummary) {
    if opts.format == CliFormat::Json {
        let view = build_result_view(record, summary, opts.result_format.as_ref());
        println!(
            "{}",
            serde_json::to_string_pretty(&view).unwrap_or_else(|_| "{}".into())
        );
    }
}

/// 已完成任务的交付：提取 → --json 校验 → --format 包装 → stdout / --output。
fn deliver_completed(
    opts: &CliOpts,
    record: &RunRecord,
    summary: &RunSummary,
    fmt_override: Option<&ResultFormat>,
) -> i32 {
    let fmt = fmt_override.or(opts.result_format.as_ref());
    let extracted: Result<ExtractedValue, XllmError> = match fmt {
        Some(f) => export_result(record, Some(f)),
        None => match &record.result {
            Some(r) => match (&r.extracted, &r.extract_error) {
                (Some(v), _) => Ok(v.clone()),
                (None, Some(e)) => Err(XllmError::Extract(e.clone())),
                (None, None) => Err(XllmError::Extract("no extracted result".into())),
            },
            None => Err(XllmError::Other("run has no final response".into())),
        },
    };
    let json_required = opts.json || (fmt_override.is_none() && record.config.json);
    let (payload_text, code) = match extracted {
        Ok(v) => {
            if json_required {
                match v.as_json() {
                    Ok(jv) => (
                        serde_json::to_string_pretty(&jv).unwrap_or_else(|_| jv.to_string()),
                        EXIT_OK,
                    ),
                    Err(e) => {
                        eprintln!(
                            "xllm: run {} completed but the result is not valid JSON: {e}\n(raw response saved; re-export with `xllm result --run {}`)",
                            record.run_id, record.run_id
                        );
                        (String::new(), EXIT_RESULT_INVALID)
                    }
                }
            } else {
                (v.to_output_text(), EXIT_OK)
            }
        }
        Err(e) => {
            eprintln!(
                "xllm: run {} completed but result extraction failed: {e}\n(raw response saved; re-export with `xllm result --run {} --result-format raw`)",
                record.run_id, record.run_id
            );
            (String::new(), EXIT_RESULT_INVALID)
        }
    };
    let output_text = match opts.format {
        CliFormat::Json => {
            let view = build_result_view(record, summary, fmt);
            serde_json::to_string_pretty(&view).unwrap_or_else(|_| "{}".into())
        }
        CliFormat::Text => {
            if code != EXIT_OK {
                return code;
            }
            payload_text
        }
    };
    match &opts.output {
        Some(path) => {
            if code != EXIT_OK && opts.format == CliFormat::Text {
                return code;
            }
            if let Err(e) = std::fs::write(path, output_text.as_bytes()) {
                eprintln!(
                    "xllm: task {} completed, but saving to {} failed: {e}\n(re-export with `xllm result --run {} --output <path>`)",
                    record.run_id,
                    path.display(),
                    record.run_id
                );
                return EXIT_OUTPUT_FAILED;
            }
            diag(
                opts.log_level(),
                &format!("output saved to {}", path.display()),
            );
            code
        }
        None => {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(output_text.as_bytes());
            if !output_text.ends_with('\n') {
                let _ = out.write_all(b"\n");
            }
            let _ = out.flush();
            code
        }
    }
}

async fn load_task_error(
    runtime: &buckyos_api::BuckyOSRuntime,
    task_id: &str,
) -> Option<TaskError> {
    let client = match runtime.get_task_mgr_client().await {
        Ok(client) => client,
        Err(error) => {
            log::warn!(
                "aicc helper.llm_chat failed: load task error skipped; get task-manager client failed; task_id={task_id}; error={error}"
            );
            return None;
        }
    };

    let task = match client.get_task(task_id).await {
        Ok(task) => task,
        Err(error) => {
            log::warn!(
                "aicc helper.llm_chat failed: get task failed; task_id={task_id}; error={error}"
            );
            return None;
        }
    };

    if task.error.is_none() {
        log::warn!(
            "aicc helper.llm_chat failed: task has no error; task_id={task_id}; phase={:?}",
            task.phase
        );
    }

    task.error
}

fn format_aicc_failed_message(
    task_id: &str,
    event_ref: Option<&str>,
    task_error: Option<&TaskError>,
) -> String {
    let mut message = format!(
        "aicc helper.llm_chat failed: task_id={}, event_ref={}",
        task_id,
        event_ref.unwrap_or("")
    );
    if let Some(error) = task_error {
        message.push_str(", error=");
        message.push_str(&format_task_error(error));
    }
    message
}

fn format_task_error(error: &TaskError) -> String {
    let mut message = format!("{}: {}", error.code, error.message);
    if let Some(detail) = error.detail.as_ref() {
        if let Ok(detail_json) = serde_json::to_string(detail) {
            message.push_str(", detail=");
            message.push_str(&detail_json);
        }
    }
    message
}

fn aicc_response_format(force_json: bool, json_schema: Option<Value>) -> Option<LlmResponseFormat> {
    force_json.then(|| match json_schema {
        Some(schema) => {
            LlmResponseFormat::json_schema(Some("llm_response".to_string()), schema, None)
        }
        None => LlmResponseFormat::json_object(),
    })
}

// =========================================================================
// resume / list / status / result
// =========================================================================

fn fmt_time(ms: u64) -> String {
    let t = UNIX_EPOCH + Duration::from_millis(ms);
    chrono::DateTime::<chrono::Local>::from(t)
        .format("%Y-%m-%d %H:%M:%S")
        .to_string()
}

async fn run_resume(opts: CliOpts) -> i32 {
    let level = opts.log_level();
    let workdir = match opts.workdir() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("xllm: error: {e}");
            return EXIT_USAGE;
        }
    };
    let store = store_for(&opts, &workdir);
    let deps = XllmDeps::default().with_observer(Arc::new(StderrObserver { level }));
    let start = XllmRun::resume(
        &store,
        opts.run.as_deref(),
        Some(&workdir),
        opts.resume_limits(),
        deps,
    )
    .await;
    match start {
        Ok(ResumeStart::Terminal(record)) => {
            let summary = store.summarize(&record);
            diag(
                level,
                &format!(
                    "run {} is already {}; showing the saved result",
                    record.run_id,
                    record.status.label()
                ),
            );
            match record.status {
                RunStatus::Completed => deliver_completed(&opts, &record, &summary, None),
                _ => {
                    print_status_text(&record, &summary);
                    emit_json_if_requested(&opts, &record, &summary);
                    EXIT_OK
                }
            }
        }
        Ok(ResumeStart::Run(mut run)) => {
            let rec = run.record();
            diag(
                level,
                &format!(
                    "resuming run {} (saved {}, workdir {}); next: {}",
                    rec.run_id,
                    fmt_time(rec.updated_at_ms),
                    rec.workdir,
                    if rec.latest_snapshot_idx.is_some() {
                        "retry the unfinished model request"
                    } else {
                        "analyze attachments / first model request"
                    }
                ),
            );
            execute_and_deliver(&opts, &mut run).await
        }
        Err(err) => {
            eprintln!(
                "xllm: error: {err} (runs directory: {})",
                store
                    .runs_dir()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "(memory)".into())
            );
            exit_code_for_error(&err)
        }
    }
}

async fn run_list(opts: &CliOpts) -> i32 {
    let workdir = match opts.workdir() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("xllm: error: {e}");
            return EXIT_USAGE;
        }
    };
    let store = store_for(opts, &workdir);
    match list_runs(&store, Some(&workdir), opts.limit) {
        Ok(items) => {
            if opts.format == CliFormat::Json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&items).unwrap_or_else(|_| "[]".into())
                );
                return EXIT_OK;
            }
            if items.is_empty() {
                eprintln!(
                    "xllm: no runs for {} under {}",
                    workdir.display(),
                    store
                        .runs_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(memory)".into())
                );
                return EXIT_OK;
            }
            for it in items {
                println!(
                    "{}  {:<20}  {}  {}  {}{}",
                    it.run_id,
                    it.status_label,
                    fmt_time(it.updated_at_ms),
                    if it.resumable { "resumable" } else { "-" },
                    it.summary,
                    if it.stale_running {
                        "  (process exited)"
                    } else {
                        ""
                    }
                );
            }
            EXIT_OK
        }
        Err(err) => {
            eprintln!("xllm: error: {err}");
            exit_code_for_error(&err)
        }
    }
}

fn select_record(opts: &CliOpts, store: &RunStore, workdir: &Path) -> Result<RunRecord, XllmError> {
    match &opts.run {
        Some(id) => store.read_record(id),
        None => {
            crate::local_llm_context::latest_run(store, Some(workdir), false)?.ok_or_else(|| {
                XllmError::RunNotFound {
                    run_id: "(latest)".into(),
                    runs_dir: store
                        .runs_dir()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(memory)".into()),
                }
            })
        }
    }
}

fn print_status_text(record: &RunRecord, summary: &RunSummary) {
    println!("run:            {}", record.run_id);
    println!(
        "status:         {}{}{}",
        summary.status_label,
        if summary.is_terminal {
            " (terminal)"
        } else {
            ""
        },
        if summary.stale_running {
            " — process exited"
        } else {
            ""
        }
    );
    println!("summary:        {}", record.summary);
    println!("workdir:        {}", record.workdir);
    println!(
        "runs dir:       {}",
        record.runs_dir.as_deref().unwrap_or("(memory)")
    );
    println!("created:        {}", fmt_time(record.created_at_ms));
    println!("updated:        {}", fmt_time(record.updated_at_ms));
    let c = &record.config;
    println!(
        "provider/model: {} / {}{}",
        c.provider.effective_kind().as_str(),
        c.model,
        c.file_model
            .as_ref()
            .map(|f| format!(" (file_model {f})"))
            .unwrap_or_default()
    );
    println!(
        "loop/tools:     {} / {}{}",
        c.loop_model.as_str(),
        if c.tools.enabled {
            let names = c.tools.all_names();
            if names.is_empty() {
                "enabled (no tools)".to_string()
            } else {
                names.join(", ")
            }
        } else {
            "disabled".to_string()
        },
        if c.tools.tools2actions {
            " (tools2actions)"
        } else {
            ""
        }
    );
    println!(
        "limits:         max_rounds={} timeout={}s llm_timeout={}s max_tokens={}",
        c.limits.max_rounds,
        c.limits.timeout_secs,
        c.limits.llm_timeout_secs,
        c.limits
            .max_tokens
            .map(|v| v.to_string())
            .unwrap_or_else(|| "model default".into())
    );
    println!("result_format:  {}", c.result_format.as_string());
    if !c.config_files.is_empty() {
        println!("config files:   {}", c.config_files.join(" → "));
    }
    let mut src: Vec<String> = c
        .sources
        .iter()
        .filter(|(k, _)| !k.contains("session_token") && !k.contains("api_key"))
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    src.sort();
    if !src.is_empty() {
        println!("sources:        {}", src.join("; "));
    }
    if let Some(e) = &record.last_error {
        println!(
            "last error:     [{}] {}{}",
            e.phase,
            e.message,
            if e.recoverable { " (recoverable)" } else { "" }
        );
        if let Some(c) = &e.condition {
            println!("to continue:    {c}");
        }
    }
    if let Some(r) = &record.limit_reason {
        println!("limit:          {r}");
    }
    if let Some(r) = &record.interrupt_reason {
        println!("interrupted:    {r}");
    }
    if !record.artifacts.is_empty() {
        println!("artifacts:      {}", record.artifacts.join(", "));
    }
    if let Some(u) = record.usage.total() {
        println!(
            "usage:          in={} out={} total={} requests={}",
            u.input_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            u.output_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            u.total_tokens
                .map(|v| v.to_string())
                .unwrap_or_else(|| "?".into()),
            record.usage.llm_requests
        );
    }
    println!(
        "resumable:      {}{}",
        if summary.resumable { "yes" } else { "no" },
        if summary.resumable {
            format!(" — {}", record.resume_command())
        } else {
            String::new()
        }
    );
}

async fn run_status(opts: &CliOpts) -> i32 {
    let workdir = match opts.workdir() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("xllm: error: {e}");
            return EXIT_USAGE;
        }
    };
    let store = store_for(opts, &workdir);
    let record = match select_record(opts, &store, &workdir) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("xllm: error: {err}");
            return exit_code_for_error(&err);
        }
    };
    let (record, summary) = match load_run(&store, &record.run_id) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("xllm: error: {err}");
            return exit_code_for_error(&err);
        }
    };
    if opts.format == CliFormat::Json {
        let view = build_result_view(&record, &summary, None);
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "result": view,
                "input": record.input,
                "config": record.config,
                "prompt": record.prompt,
            }))
            .unwrap_or_else(|_| "{}".into())
        );
        return EXIT_OK;
    }
    print_status_text(&record, &summary);
    EXIT_OK
}

async fn run_result(opts: &CliOpts) -> i32 {
    let workdir = match opts.workdir() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("xllm: error: {e}");
            return EXIT_USAGE;
        }
    };
    let store = store_for(opts, &workdir);
    let record = match select_record(opts, &store, &workdir) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("xllm: error: {err}");
            return exit_code_for_error(&err);
        }
    };
    let summary = store.summarize(&record);
    if record.result.is_none() {
        eprintln!(
            "xllm: run {} has no final response yet (status: {})",
            record.run_id, summary.status_label
        );
        emit_json_if_requested(opts, &record, &summary);
        return EXIT_TASK_FAILED;
    }
    deliver_completed(opts, &record, &summary, opts.result_format.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse(args: &[&str]) -> Result<CliOpts, String> {
        CliOpts::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).map_err(|e| match e
        {
            ParseError::Bad(m) => m,
            ParseError::Help => "help".into(),
            ParseError::Version => "version".into(),
        })
    }

    #[test]
    fn positional_question_and_user_are_exclusive() {
        let err = parse(&["hello", "--user", "x"]).unwrap_err();
        assert!(err.contains("mutually exclusive"));
        let ok = parse(&["hello world"]).unwrap();
        assert_eq!(ok.question.as_deref(), Some("hello world"));
        assert_eq!(ok.command, Command::New);
    }

    #[test]
    fn subcommands_and_resume_rules() {
        assert_eq!(parse(&["list", "--limit", "3"]).unwrap().limit, 3);
        assert_eq!(
            parse(&["status", "--run", "abc"]).unwrap().command,
            Command::Status
        );
        assert!(parse(&["--resume", "question"]).is_err());
        assert!(parse(&["--run", "abc", "question"]).is_err());
        assert!(parse(&["--tools", "--no-tools"]).is_err());
        assert!(parse(&["--select", "a", "--system", "b"]).is_err());
        assert!(parse(&["--user", "a", "--user", "b"]).is_err());
        assert_eq!(parse(&["--resume"]).unwrap().command, Command::Resume);
    }

    #[test]
    fn attachments_keep_command_order() {
        use crate::local_llm_context::Attachment;
        let o = parse(&[
            "--image", "a.png", "--file", "b.txt", "--image", "c.png", "q",
        ])
        .unwrap();
        assert_eq!(o.attachments.len(), 3);
        assert!(matches!(&o.attachments[1], Attachment::File { path } if path.ends_with("b.txt")));
    }

    #[test]
    fn aicc_failed_message_includes_task_error_detail() {
        let error = TaskError {
            code: "provider_error".to_string(),
            message: "Gemini http_error: model unavailable".to_string(),
            detail: Some(json!({
                "provider_code": "404",
                "message": "use models/gemini-3.1-pro-preview"
            })),
        };

        let message = format_aicc_failed_message("t-1", Some("/task_mgr/t-1"), Some(&error));

        assert!(message.contains("task_id=t-1"));
        assert!(message.contains("provider_error"));
        assert!(message.contains("Gemini http_error: model unavailable"));
        assert!(message.contains("gemini-3.1-pro-preview"));
        assert!(message.contains("\"provider_code\":\"404\""));
    }
}
