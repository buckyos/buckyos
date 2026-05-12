//! 使用 buckyos 的 aicc 服务，来驱动 local llm。
//!
//! 这是 `agent_tool` 二进制的一个 dev/test 子命令，可以通过命令行来指定
//! local llm dir，以及关键的 Input 的构造。主要用作 DV Test 环境下
//! `llm_context::LocalLLMContext` 的端到端测试驱动。
//!
//! ## 用法
//!
//! ```text
//!   agent_tool run_local_llm \
//!     --dir <local-llm-dir> \
//!     [--model <alias>]     \              # AICC model alias（除非 --append，否则必填）
//!     [--objective <text>]  \              # 任务目标（写进 worklog，不进 prompt）
//!     [--system <text>]     \              # 追加一条 system message
//!     [--user <text>]       \              # 追加一条 user message
//!     [--input-file <path>] \              # 读取 JSON 数组（Vec<AiMessage>）作为初始历史
//!     [--input-stdin]       \              # 把 stdin 当作一条 user message
//!     [--append <text>]     \              # 把 text 当作 user message 追加到上一轮 Completed
//!                                          #   run 之后并起新一轮（与其它输入 flag / --new 互斥）
//!     [--temperature <f>]   \              # 采样温度
//!     [--max-tokens <n>]    \              # max_completion_tokens
//!     [--max-rounds <n>]    \              # ToolPolicy.max_rounds（默认 8）
//!     [--no-tools]          \              # ToolPolicy.mode = None（默认 All）
//!     [--json]              \              # 强制 JSON 输出
//!     [--new]               \              # 强制新 run（默认 resume_or_new）
//!     [--output <path>]                    # 把 final outcome 写到文件（不写则只打印）
//! ```
//!
//! 至少要提供 `--user` / `--system` / `--input-file` / `--input-stdin` /
//! `--append` 中的一项；前四个 flag 互相可以叠加构成初始 input，`--append`
//! 是"接着上一轮跑"的独立路径，跟那四个互斥。
//!
//! ## 设计要点
//!
//! 1. **LlmClient 适配**：通过 `AiccLlmClient` 把 waist 侧的
//!    `LlmInferenceRequest` 翻译成 AICC 的 `AiMethodRequest`（capability =
//!    Llm，method = `llm.chat`），返回的 `AiResponseSummary` 直接 forward
//!    给 waist。Running 状态本工具不做轮询（DV test 用的是同步模型），
//!    遇到时直接报错让 caller 排查。
//!
//! 2. **Compressor**：使用最简单的 `KeepTailCompressor` —— 保留 `system`
//!    消息加上最后 N 条非 system 消息。够测，不引入二级 LLM 调用。
//!
//! 3. **runtime 注入**：通过 `buckyos_api::init_buckyos_api_runtime`
//!    （`FrameService` 类型）初始化运行时，复用 `get_aicc_client()`。
//!    DV test 环境会注入合适的 zone / token，让 kRPC 能拨到 aicc。

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use buckyos_api::{
    ai_methods, get_buckyos_api_runtime, init_buckyos_api_runtime, set_buckyos_api_runtime,
    value_to_object_map, AiMessage, AiMethodRequest, AiMethodStatus, AiPayload, AiResponseSummary,
    AiToolSpec, AiccClient, BuckyOSRuntimeType, Capability, ModelSpec, Requirements, RespFormat,
};
use llm_context::{
    LLMComputeError, LLMContextOutcome, LlmClient, LlmInferenceRequest, ToolMode, ToolPolicy,
};

use crate::local_llm_context::{Compressor, LocalLLMContextError};
use crate::{LocalLLMContext, OneShotRequest};
use serde_json::{json, Value};
use tokio::fs;
use tokio::io::AsyncReadExt;

// =========================================================================
// 子命令入口
// =========================================================================

/// Dispatch entry, called by `lib::run_process` when argv[1] == "run_local_llm".
/// `args` 是去掉 `agent_tool run_local_llm` 之后的剩余参数。直接 println /
/// eprintln 到 stdout / stderr，返回 process exit code。
pub async fn run_subcommand(args: Vec<String>) -> i32 {
    let opts = match CliOpts::parse(&args) {
        Ok(opts) => opts,
        Err(ParseError::Help) => {
            print!("{}", USAGE);
            return 0;
        }
        Err(ParseError::Bad(msg)) => {
            eprintln!("error: {msg}\n\n{}", USAGE);
            return 2;
        }
    };

    match run(opts).await {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("run_local_llm failed: {err}");
            1
        }
    }
}

async fn run(opts: CliOpts) -> Result<(), Box<dyn std::error::Error>> {
    // 1. 构造 OneShotRequest —— 走 --append 还是常规 input flag 是两条路。
    let mut request = if let Some(text) = opts.append.as_ref() {
        if opts.system.is_some()
            || opts.user.is_some()
            || opts.input_file.is_some()
            || opts.input_stdin
            || opts.force_new
        {
            return Err(
                "--append is mutually exclusive with --system / --user / \
                        --input-file / --input-stdin / --new"
                    .into(),
            );
        }
        // 从上一轮 Completed run 继承 objective / policies / 累积历史，再 push
        // 这一条新 user 消息。CLI 后面的 tuning override 还会覆盖一遍。
        LocalLLMContext::prepare_followup_request(
            &opts.dir,
            AiMessage::new("user".into(), text.clone()),
        )?
    } else {
        let input = build_input_messages(&opts).await?;
        if input.is_empty() {
            return Err(
                "no input messages — provide at least one of --system / --user / \
                        --input-file / --input-stdin / --append"
                    .into(),
            );
        }
        OneShotRequest::new(
            opts.objective
                .clone()
                .unwrap_or_else(|| "run_local_llm dev test".to_string()),
            input,
        )
    };

    // 2. CLI tuning overrides
    //
    // --model 没给只在 --append 路径下合法（CLI 解析器已经强制过），此时让
    // model_policy 沿用 prior request.json 的值。其它 tuning flag 是 CLI 的
    // 既有行为：始终用 CLI 值覆盖（含默认值）。
    if let Some(m) = opts.model.as_ref() {
        request.model_policy = Some(llm_context::ModelPolicy {
            preferred: m.clone(),
            fallbacks: Vec::new(),
            temperature: opts.temperature,
            max_completion_tokens: opts.max_tokens,
            provider_options: None,
        });
    }
    request.tool_policy = Some(ToolPolicy {
        mode: if opts.no_tools {
            ToolMode::None
        } else {
            ToolMode::All
        },
        max_rounds: opts.max_rounds.unwrap_or(8),
        ..ToolPolicy::default()
    });
    if opts.force_json {
        request.output = Some(llm_context::OutputSpec::Json {
            schema: None,
            strict: false,
        });
    }
    if let Some(obj) = opts.objective.as_ref() {
        // 显式给了就覆盖 inherited objective；没给就保留（无论 inherited 还是默认串）。
        request.objective = obj.clone();
    }

    // 3. 初始化运行时 → 取 AICC client → 包装成 LlmClient
    ensure_buckyos_runtime().await?;
    let aicc = acquire_aicc_client().await?;
    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new(aicc));

    // 4. 启动 LocalLLMContext
    //
    // --append 永远走 new_run：每一轮对话独立 run_id，审计链清晰；
    // semantic_hash 也不会因为 input 多了一条而跟旧 run 冲突。
    let mut ctx = if opts.force_new || opts.append.is_some() {
        LocalLLMContext::new_run(opts.dir.clone(), request, llm)?
    } else {
        LocalLLMContext::resume_or_new(opts.dir.clone(), request, llm)?
    };
    eprintln!(
        "run_local_llm: dir={} run_id={}",
        opts.dir.display(),
        ctx.run_id()
    );

    // 5. 跑到终态 / 挂起
    let compressor = KeepTailCompressor { tail: 8 };
    let outcome = ctx.drive_to_terminal(&compressor).await?;

    // 6. 输出
    let pretty = serde_json::to_string_pretty(&outcome)?;
    if let Some(path) = opts.output.as_ref() {
        fs::write(path, pretty.as_bytes()).await?;
        eprintln!("outcome written to {}", path.display());
    } else {
        println!("{pretty}");
    }

    // 终态非 Done 视作"业务失败"——返回非零退出码方便脚本判断
    match outcome {
        LLMContextOutcome::Done { .. } => Ok(()),
        other => Err(format!("non-done outcome: {}", outcome_tag(&other)).into()),
    }
}

fn outcome_tag(o: &LLMContextOutcome) -> &'static str {
    match o {
        LLMContextOutcome::Done { .. } => "done",
        LLMContextOutcome::WaitInput { .. } => "wait_input",
        LLMContextOutcome::PendingTool { .. } => "pending_tool",
        LLMContextOutcome::BudgetExhausted { .. } => "budget_exhausted",
        LLMContextOutcome::Error { .. } => "error",
        LLMContextOutcome::ContextLimitReached { .. } => "context_limit_reached",
    }
}

// =========================================================================
// CLI 参数解析
// =========================================================================

const USAGE: &str = r#"Usage: run_local_llm --dir <path> [--model <alias>] [options]

Required:
  --dir <path>           Local LLM context working directory
  --model <alias>        AICC model alias (e.g. "gpt-4o", "default-llm")
                         Required unless --append is used (then inherited from
                         the prior run unless overridden).

Input (at least one required):
  --system <text>        Prepend a system message
  --user <text>          Append a user message
  --input-file <path>    Load Vec<AiMessage> from JSON file
  --input-stdin          Read stdin as a single user message
  --append <text>        Continue the dir's latest Completed run by appending
                         this as a new user message. Inherits objective/policies
                         from the prior run.json (CLI tuning flags still
                         override). Mutually exclusive with --system/--user/
                         --input-file/--input-stdin/--new.

Tuning:
  --objective <text>     Free-form objective (worklog only, not in prompt)
  --temperature <f>      Sampling temperature
  --max-tokens <n>       max_completion_tokens
  --max-rounds <n>       ToolPolicy.max_rounds (default 8)
  --no-tools             Disable tool loop (ToolPolicy.mode = None)
  --json                 Force JSON output

Resume:
  --new                  Force a fresh run (default: resume_or_new)

Output:
  --output <path>        Write final outcome JSON to file
  -h, --help             Show this help
"#;

#[derive(Debug)]
struct CliOpts {
    dir: PathBuf,
    /// `None` 只在 `--append` 模式下合法——会从 prior run 的 request.json
    /// 继承 model_policy。否则解析阶段就会报错。
    model: Option<String>,

    objective: Option<String>,
    system: Option<String>,
    user: Option<String>,
    input_file: Option<PathBuf>,
    input_stdin: bool,
    /// `--append <text>` 的值。Some 时走 follow-up run 路径,与其它 input
    /// flag / `--new` 互斥(在 `run()` 里 enforce)。
    append: Option<String>,

    temperature: Option<f32>,
    max_tokens: Option<u32>,
    max_rounds: Option<u32>,
    no_tools: bool,
    force_json: bool,

    force_new: bool,
    output: Option<PathBuf>,
}

enum ParseError {
    Help,
    Bad(String),
}

impl CliOpts {
    fn parse(args: &[String]) -> Result<Self, ParseError> {
        let mut dir: Option<PathBuf> = None;
        let mut model: Option<String> = None;
        let mut objective = None;
        let mut system = None;
        let mut user = None;
        let mut input_file = None;
        let mut input_stdin = false;
        let mut append: Option<String> = None;
        let mut temperature = None;
        let mut max_tokens = None;
        let mut max_rounds = None;
        let mut no_tools = false;
        let mut force_json = false;
        let mut force_new = false;
        let mut output = None;

        let mut idx = 0;
        while idx < args.len() {
            let tok = args[idx].as_str();
            match tok {
                "-h" | "--help" => return Err(ParseError::Help),
                "--dir" => {
                    dir = Some(PathBuf::from(next_value(args, &mut idx, "--dir")?));
                }
                "--model" => model = Some(next_value(args, &mut idx, "--model")?),
                "--objective" => objective = Some(next_value(args, &mut idx, "--objective")?),
                "--system" => system = Some(next_value(args, &mut idx, "--system")?),
                "--user" => user = Some(next_value(args, &mut idx, "--user")?),
                "--input-file" => {
                    input_file = Some(PathBuf::from(next_value(args, &mut idx, "--input-file")?));
                }
                "--input-stdin" => input_stdin = true,
                "--append" => append = Some(next_value(args, &mut idx, "--append")?),
                "--temperature" => {
                    let v = next_value(args, &mut idx, "--temperature")?;
                    temperature = Some(
                        v.parse::<f32>()
                            .map_err(|e| ParseError::Bad(format!("invalid --temperature: {e}")))?,
                    );
                }
                "--max-tokens" => {
                    let v = next_value(args, &mut idx, "--max-tokens")?;
                    max_tokens = Some(
                        v.parse::<u32>()
                            .map_err(|e| ParseError::Bad(format!("invalid --max-tokens: {e}")))?,
                    );
                }
                "--max-rounds" => {
                    let v = next_value(args, &mut idx, "--max-rounds")?;
                    max_rounds = Some(
                        v.parse::<u32>()
                            .map_err(|e| ParseError::Bad(format!("invalid --max-rounds: {e}")))?,
                    );
                }
                "--no-tools" => no_tools = true,
                "--json" => force_json = true,
                "--new" => force_new = true,
                "--output" => {
                    output = Some(PathBuf::from(next_value(args, &mut idx, "--output")?));
                }
                other => {
                    return Err(ParseError::Bad(format!("unknown flag `{other}`")));
                }
            }
            idx += 1;
        }

        let dir = dir.ok_or_else(|| ParseError::Bad("missing --dir".into()))?;
        // --model 在 --append 模式下可省略(从 prior run 继承);其它路径下必填。
        if model.is_none() && append.is_none() {
            return Err(ParseError::Bad(
                "missing --model (required unless --append is set)".into(),
            ));
        }

        Ok(Self {
            dir,
            model,
            objective,
            system,
            user,
            input_file,
            input_stdin,
            append,
            temperature,
            max_tokens,
            max_rounds,
            no_tools,
            force_json,
            force_new,
            output,
        })
    }
}

fn next_value(args: &[String], idx: &mut usize, flag: &str) -> Result<String, ParseError> {
    *idx += 1;
    args.get(*idx)
        .cloned()
        .ok_or_else(|| ParseError::Bad(format!("missing value for {flag}")))
}

async fn build_input_messages(
    opts: &CliOpts,
) -> Result<Vec<AiMessage>, Box<dyn std::error::Error>> {
    let mut msgs: Vec<AiMessage> = Vec::new();

    if let Some(sys) = opts.system.as_ref() {
        msgs.push(AiMessage::new("system".into(), sys.clone()));
    }

    if let Some(path) = opts.input_file.as_ref() {
        let bytes = fs::read(path).await?;
        let loaded: Vec<AiMessage> =
            serde_json::from_slice(&bytes).map_err(|e| format!("parse {}: {e}", path.display()))?;
        msgs.extend(loaded);
    }

    if let Some(u) = opts.user.as_ref() {
        msgs.push(AiMessage::new("user".into(), u.clone()));
    }

    if opts.input_stdin {
        let mut buf = String::new();
        tokio::io::stdin().read_to_string(&mut buf).await?;
        if !buf.is_empty() {
            msgs.push(AiMessage::new("user".into(), buf));
        }
    }

    Ok(msgs)
}

// =========================================================================
// AICC runtime 接入
// =========================================================================

pub(crate) async fn ensure_buckyos_runtime() -> Result<(), Box<dyn std::error::Error>> {
    // 优先复用已经初始化的 runtime（被外层 harness 注入的场景），否则按
    // AppClient 类型初始化一个 —— 这是 DV Test 容器里 buckyos 进程的
    // 通用约定（参见 agent_tool_cli_dev::build_task_manager_client）。
    if get_buckyos_api_runtime().is_ok() {
        return Ok(());
    }

    let runtime =
        init_buckyos_api_runtime("buckycli", None, BuckyOSRuntimeType::AppClient).await?;
    set_buckyos_api_runtime(runtime)?;
    Ok(())
}

pub(crate) async fn acquire_aicc_client() -> Result<Arc<AiccClient>, Box<dyn std::error::Error>> {
    let runtime = get_buckyos_api_runtime()?;
    let client = runtime.get_aicc_client().await?;
    Ok(Arc::new(client))
}

// =========================================================================
// AICC → LlmClient 适配
// =========================================================================

pub(crate) struct AiccLlmClient {
    client: Arc<AiccClient>,
}

impl AiccLlmClient {
    pub(crate) fn new(client: Arc<AiccClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl LlmClient for AiccLlmClient {
    async fn infer(&self, req: LlmInferenceRequest) -> Result<AiResponseSummary, LLMComputeError> {
        let LlmInferenceRequest {
            messages,
            model_alias,
            fallbacks: _,
            temperature,
            max_completion_tokens,
            force_json,
            json_schema: _,
            provider_options,
            tool_specs,
            allow_tool_calls,
        } = req;

        // tool specs：waist ToolSpecLite → AICC AiToolSpec
        let aicc_tool_specs: Vec<AiToolSpec> = if allow_tool_calls {
            tool_specs
                .into_iter()
                .map(|spec| AiToolSpec {
                    name: spec.name,
                    description: spec.description,
                    args_schema: value_to_object_map(spec.args_schema),
                    output_schema: json!({}),
                })
                .collect()
        } else {
            Vec::new()
        };

        // payload.options：把 temperature / max_tokens 透传给底层 provider
        let mut options = serde_json::Map::new();
        if let Some(t) = temperature {
            options.insert("temperature".into(), json!(t));
        }
        if let Some(n) = max_completion_tokens {
            options.insert("max_tokens".into(), json!(n));
        }
        let options_value = if options.is_empty() {
            Some(json!({}))
        } else {
            Some(Value::Object(options))
        };

        let payload = AiPayload {
            text: None,
            messages,
            tool_specs: aicc_tool_specs,
            resources: Vec::new(),
            input_json: None,
            options: options_value,
        };

        let mut must_features = Vec::new();
        if allow_tool_calls && !payload.tool_specs.is_empty() {
            must_features.push("tool_calling".to_string());
        }
        if force_json {
            must_features.push("json_output".to_string());
        }

        let requirements = Requirements {
            must_features,
            max_latency_ms: None,
            max_cost_usd: None,
            resp_format: if force_json {
                RespFormat::Json
            } else {
                RespFormat::Text
            },
            extra: provider_options,
        };

        let request = AiMethodRequest::new(
            Capability::Llm,
            ModelSpec::new(model_alias.clone(), None),
            requirements,
            payload,
            None,
        );

        let response = self
            .client
            .call_method(ai_methods::LLM_CHAT, request)
            .await
            .map_err(|e| LLMComputeError::Provider(format!("aicc llm.chat failed: {e}")))?;

        match response.status {
            AiMethodStatus::Succeeded => response.result.ok_or_else(|| {
                LLMComputeError::Provider("aicc llm.chat succeeded but result is empty".to_string())
            }),
            AiMethodStatus::Failed => Err(LLMComputeError::Provider(format!(
                "aicc llm.chat failed: task_id={}, event_ref={}",
                response.task_id,
                response.event_ref.as_deref().unwrap_or("")
            ))),
            AiMethodStatus::Running => Err(LLMComputeError::Provider(format!(
                "aicc llm.chat returned async task `{}`; run_local_llm dev tool does \
                 not poll async tasks — use a synchronous-capable model",
                response.task_id
            ))),
        }
    }
}

// =========================================================================
// Compressor：保留 system + 最后 N 条
// =========================================================================

pub(crate) struct KeepTailCompressor {
    tail: usize,
}

impl KeepTailCompressor {
    pub(crate) fn new(tail: usize) -> Self {
        Self { tail }
    }
}

#[async_trait]
impl Compressor for KeepTailCompressor {
    async fn compress(
        &self,
        accumulated: Vec<AiMessage>,
        _dir: &std::path::Path,
    ) -> Result<Vec<AiMessage>, LocalLLMContextError> {
        let (sys, rest): (Vec<_>, Vec<_>) =
            accumulated.into_iter().partition(|m| m.role == "system");
        let kept_tail = if rest.len() > self.tail {
            rest[rest.len() - self.tail..].to_vec()
        } else {
            rest
        };
        let mut out = sys;
        out.extend(kept_tail);
        Ok(out)
    }
}
