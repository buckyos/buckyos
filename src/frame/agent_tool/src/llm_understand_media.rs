use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use buckyos_api::{get_buckyos_api_runtime, AiContent, AiMessage, AiRole, ResourceRef};
use llm_context::deps::{LLMContextDeps, ToolManager};
use llm_context::{
    ContextOutput, LLMContextOutcome, LlmClient, ModelPolicy, OutputSpec, ToolMode, ToolPolicy,
};
use ndn_lib::FileObject;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::run_local_llm::{ensure_buckyos_runtime, AiccLlmClient};
use crate::{
    cli_error_result, llm_compress, render_cli_output, AgentTool, AgentToolError,
    AgentToolPendingReason, AgentToolResult, AgentToolStatus, CallingConventions, LocalLLMContext,
    OneShotRequest, SessionRuntimeContext, ToolSpec, AGENT_TOOL_PROTOCOL_VERSION, CLI_EXIT_ERROR,
    CLI_EXIT_SUCCESS, CLI_EXIT_USAGE,
};

pub const TOOL_LLM_UNDERSTAND_MEDIA: &str = "llm_understand_media";

const DEFAULT_MODEL_ALIAS: &str = "llm.vision";
const DEFAULT_SUMMARY_MODEL_ALIAS: &str = "llm.summary";
const DEFAULT_TARGET_TOKENS: u32 = 24_000;
const DEFAULT_MAX_COMPLETION_TOKENS: u32 = 2_048;
const RAW_OUTPUT_LOG_PREVIEW_CHARS: usize = 2_000;
const DEFAULT_VIDEO_FRAME_COUNT: usize = 8;
const MAX_VIDEO_FRAME_COUNT: usize = 16;

const SYSTEM_PROMPT: &str = r#"You are OpenDAN's controlled attachment-understanding side context.

You must inspect the target attachment and answer the user's goal as a JSON object with exactly these fields:
- observations: array of objects with id and description.
- reasoning: string.
- conclusion: string.
- confidence: one of "Observed", "Inferred", "Uncertain".

Rules:
1. Produce observations first in causal order. Observations are objective facts observable in the attachment. Each observation must have a stable id such as "obs-1".
2. Reasoning must come after observations and must only cite facts that trace to observation ids. If a step needs information not in observations, mark it as speculation.
3. Conclusions that cannot be derived only from observations must be marked in reasoning as speculation and reflected by confidence "Inferred" or "Uncertain".
4. Do not invent attachment details to support a likely answer.
5. For audio, distinguish clearly intelligible speech from a sound that merely resembles speech. An exact transcription is "Observed" only when the words are clearly audible and supported by an observation that explicitly states the speech is unambiguous. If the clip is short, noisy, ambiguous, or could instead be a non-speech sound, mark any proposed transcription "Uncertain" and present it only as a candidate, not as an observed fact.
6. Return only JSON. Do not call tools."#;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservationItem {
    pub id: String,
    pub description: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum Confidence {
    Observed,
    Inferred,
    Uncertain,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnderstandingReport {
    pub observations: Vec<ObservationItem>,
    pub reasoning: String,
    pub conclusion: String,
    pub confidence: Confidence,
}

pub struct LlmUnderstandMediaTool;

impl LlmUnderstandMediaTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for LlmUnderstandMediaTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AgentTool for LlmUnderstandMediaTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LLM_UNDERSTAND_MEDIA.to_string(),
            description: "Understand an attachment through a controlled LLM side context. Archives must be extracted first; other formats are forwarded to the selected model and fail if it does not support them. Accepts media, goal, and max_completion_tokens only; media should be a named_object ResourceRef.".to_string(),
            args_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["media", "goal", "max_completion_tokens"],
                "properties": {
                    "media": {
                        "type": "object",
                        "description": "ResourceRef-shaped media argument. Prefer {kind:\"named_object\", obj_id:\"...\"}; url is accepted. mime_hint is optional."
                    },
                    "goal": {
                        "type": "string",
                        "description": "What to understand or answer from the media."
                    },
                    "max_completion_tokens": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Token budget for the media model to complete this task."
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "required": ["observations", "reasoning", "conclusion", "confidence"],
                "properties": {
                    "observations": { "type": "array" },
                    "reasoning": { "type": "string" },
                    "conclusion": { "type": "string" },
                    "confidence": { "type": "string" }
                }
            }),
            usage: None,
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::LLM
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let opts = RunOpts::from_tool_args(args)?;
        let (result, _) = run(opts).await;
        Ok(result)
    }
}

pub async fn run_subcommand(args: Vec<String>) -> i32 {
    let opts = match CliOpts::parse(&args) {
        Ok(opts) => match opts.into_run_opts().await {
            Ok(opts) => opts,
            Err(err) => {
                emit_result(&cli_error_result(
                    Some(TOOL_LLM_UNDERSTAND_MEDIA),
                    &AgentToolError::InvalidArgs(err),
                ));
                return CLI_EXIT_USAGE;
            }
        },
        Err(ParseError::Help) => {
            print!("{}", USAGE);
            return CLI_EXIT_SUCCESS;
        }
        Err(ParseError::Bad(msg)) => {
            eprintln!("error: {msg}\n\n{}", USAGE);
            emit_result(&cli_error_result(
                Some(TOOL_LLM_UNDERSTAND_MEDIA),
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

#[derive(Clone, Debug)]
struct RunOpts {
    media_value: Value,
    goal: String,
    parent_history: Vec<AiMessage>,
    work_dir: Option<PathBuf>,
    model: Option<String>,
    summary_model: String,
    target_tokens: u32,
    max_completion_tokens: u32,
}

impl RunOpts {
    fn from_tool_args(args: Value) -> Result<Self, AgentToolError> {
        let map = args.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("llm_understand_media args must be object".to_string())
        })?;
        let media_value = map
            .get("media")
            .cloned()
            .ok_or_else(|| AgentToolError::InvalidArgs("missing `media`".to_string()))?;
        let goal = map
            .get("goal")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .ok_or_else(|| AgentToolError::InvalidArgs("missing non-empty `goal`".to_string()))?;
        let parent_history = map
            .get("parent_history")
            .or_else(|| map.get("history"))
            .map(|value| serde_json::from_value::<Vec<AiMessage>>(value.clone()))
            .transpose()
            .map_err(|err| AgentToolError::InvalidArgs(format!("invalid parent_history: {err}")))?
            .unwrap_or_default();
        let work_dir = map
            .get("work_dir")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        let model = map
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string);
        let summary_model = map
            .get("summary_model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or(DEFAULT_SUMMARY_MODEL_ALIAS)
            .to_string();
        let target_tokens = map
            .get("target_tokens")
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .unwrap_or(DEFAULT_TARGET_TOKENS);
        let max_completion_tokens = match map.get("max_completion_tokens") {
            Some(value) => value
                .as_u64()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| {
                    AgentToolError::InvalidArgs(
                        "max_completion_tokens must be an integer".to_string(),
                    )
                })?,
            None => DEFAULT_MAX_COMPLETION_TOKENS,
        };
        if max_completion_tokens == 0 {
            return Err(AgentToolError::InvalidArgs(
                "max_completion_tokens must be greater than 0".to_string(),
            ));
        }
        let max_completion_tokens = normalize_completion_tokens(max_completion_tokens);
        Ok(Self {
            media_value,
            goal,
            parent_history,
            work_dir,
            model,
            summary_model,
            target_tokens,
            max_completion_tokens,
        })
    }
}

async fn run(opts: RunOpts) -> (AgentToolResult, i32) {
    let media = match parse_media_arg(&opts.media_value) {
        Ok(media) => media,
        Err(err) => return (build_error_result(&opts, err), CLI_EXIT_USAGE),
    };
    let input_source_kind = resource_source_kind(&media.source);
    let media_id = masked_resource_id(&media.source);

    if let Err(err) = ensure_buckyos_runtime().await {
        return (
            build_error_result(&opts, format!("init buckyos runtime failed: {err}")),
            CLI_EXIT_ERROR,
        );
    }

    let resolved_media = match resolve_media(&media).await {
        Ok(media) => media,
        Err(err) => return (build_error_result(&opts, err), CLI_EXIT_ERROR),
    };
    let mime = resolved_media.mime.clone();
    let resolved_source_kind = resource_source_kind(&resolved_media.source);
    let media_content = match prepare_media_content(resolved_media).await {
        Ok(content) => content,
        Err(err) => return (build_error_result(&opts, err), CLI_EXIT_ERROR),
    };

    let model_alias = match opts.model.clone().or_else(|| route_model(&mime)) {
        Some(model) => model,
        None => {
            return (
                build_error_result(&opts, format!("no model route for media mime `{mime}`")),
                CLI_EXIT_ERROR,
            );
        }
    };

    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new());
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(NoopToolManager));

    let purified = purify_history(&opts.parent_history);
    let compressed = match llm_compress::compress(
        &purified,
        &deps,
        opts.target_tokens,
        opts.summary_model.as_str(),
        Some("Preserve parent-history facts relevant to the media-understanding goal and discard unrelated execution detail."),
    )
    .await
    {
        Ok(messages) => messages,
        Err(err) => {
            return (
                build_error_result(&opts, format!("compress parent history failed: {err}")),
                CLI_EXIT_ERROR,
            );
        }
    };

    let parent_history_count = purified.len();
    let compressed_history_count = compressed.len();
    let request = build_request(&opts, media_content, model_alias.clone(), compressed);
    let work_dir = opts
        .work_dir
        .clone()
        .unwrap_or_else(|| default_work_dir(&opts.goal));
    let mut ctx = match LocalLLMContext::resume_or_new(work_dir.clone(), request, llm) {
        Ok(ctx) => ctx,
        Err(err) => {
            return (
                build_error_result(&opts, format!("LocalLLMContext init failed: {err}")),
                CLI_EXIT_ERROR,
            );
        }
    };
    let run_id = ctx.run_id().to_string();
    eprintln!(
        "llm_understand_media: work_dir={} run_id={}",
        work_dir.display(),
        run_id
    );
    log::info!(
        "llm_understand_media: started; work_dir={} run_id={} mime={} input_source_kind={} resolved_source_kind={} media_id={} model={} parent_history_count={} compressed_history_count={} goal={}",
        work_dir.display(),
        run_id,
        mime,
        input_source_kind,
        resolved_source_kind,
        media_id,
        model_alias,
        parent_history_count,
        compressed_history_count,
        opts.goal
    );

    let compressor =
        crate::LlmSummarizeCompressor::new(deps, opts.summary_model.clone(), opts.target_tokens);
    let outcome = match ctx.drive_to_terminal(&compressor).await {
        Ok(outcome) => outcome,
        Err(err) => {
            let message = format!("drive_to_terminal failed: {err}");
            log::error!(
                "llm_understand_media: {}; work_dir={} run_id={} goal={}",
                message,
                work_dir.display(),
                run_id,
                opts.goal
            );
            let mut result = build_error_result(&opts, message);
            add_run_context(&mut result, &work_dir, &run_id, Some(&mime));
            return (result, CLI_EXIT_ERROR);
        }
    };

    build_outcome_result(outcome, &mime, &work_dir, &run_id, &opts.goal, &media_id)
}

fn build_request(
    opts: &RunOpts,
    media_content: Vec<AiContent>,
    model_alias: String,
    parent_history: Vec<AiMessage>,
) -> OneShotRequest {
    let mut input = Vec::with_capacity(parent_history.len() + 2);
    input.push(AiMessage::text(AiRole::System, SYSTEM_PROMPT));
    input.extend(parent_history);
    let mut user_content = media_content;
    user_content.push(AiContent::text(format!("Goal: {}", opts.goal)));
    input.push(AiMessage::new(AiRole::User, user_content));

    let mut req = OneShotRequest::new(opts.goal.clone(), input);
    req.model_policy = Some(ModelPolicy {
        preferred: model_alias,
        fallbacks: Vec::new(),
        temperature: Some(0.0),
        max_completion_tokens: Some(opts.max_completion_tokens),
        provider_options: None,
    });
    req.tool_policy = Some(ToolPolicy {
        mode: ToolMode::None,
        action_mode: ToolMode::None,
        max_rounds: 0,
        max_calls_per_round: 0,
        disable_capabilities: vec!["web_search".to_string()],
        allow_deferred: false,
        ..ToolPolicy::default()
    });
    req.output = Some(OutputSpec::Json {
        schema: Some(report_schema()),
        strict: false,
    });
    req.budget = Some(llm_context::request::BudgetSpec {
        max_total_tokens: Some(
            opts.target_tokens
                .saturating_add(opts.max_completion_tokens),
        ),
        max_completion_tokens: Some(opts.max_completion_tokens),
        ..Default::default()
    });
    req
}

fn build_outcome_result(
    outcome: LLMContextOutcome,
    mime: &str,
    work_dir: &PathBuf,
    run_id: &str,
    goal: &str,
    media_id: &str,
) -> (AgentToolResult, i32) {
    match outcome {
        LLMContextOutcome::Done {
            output,
            trace,
            usage,
            ..
        } => match parse_report_output(&output) {
            Ok(report) => {
                log::info!(
                    "llm_understand_media: completed; work_dir={} run_id={} mime={} media_id={} confidence={:?} conclusion={}",
                    work_dir.display(),
                    run_id,
                    mime,
                    media_id,
                    report.confidence,
                    truncate_for_summary(&report.conclusion, 200)
                );
                let rendered = render_report(&report);
                let summary = truncate_for_summary(&report.conclusion, 200);
                (
                    AgentToolResult {
                        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                        tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                        cmd_name: None,
                        status: AgentToolStatus::Success,
                        task_id: None,
                        pending_reason: None,
                        check_after: None,
                        estimated_wait: None,
                        title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => done"),
                        summary,
                        details: serde_json::to_value(&report).unwrap_or_else(|_| json!({})),
                        cmd_args: None,
                        return_code: Some(0),
                        partial_output: None,
                        output: Some(rendered),
                    },
                    CLI_EXIT_SUCCESS,
                )
            }
            Err(err) => {
                let raw_output = output_to_text(&output);
                let raw_output_chars = raw_output.chars().count();
                let raw_output_preview =
                    truncate_for_summary(&raw_output, RAW_OUTPUT_LOG_PREVIEW_CHARS);
                let output_kind = context_output_kind(&output);
                let final_outcome_path = run_final_outcome_path(work_dir, run_id);
                let raw_output_path = match write_parse_error_raw_output(
                    work_dir,
                    run_id,
                    &raw_output,
                ) {
                    Ok(path) => Some(path),
                    Err(write_err) => {
                        log::warn!(
                            "llm_understand_media: write parse error raw output failed: {}; work_dir={} run_id={}",
                            write_err,
                            work_dir.display(),
                            run_id
                        );
                        None
                    }
                };
                log::error!(
                    "llm_understand_media: parse understanding report failed: {}; work_dir={} run_id={} mime={} output_kind={} raw_output_chars={} raw_output_preview={:?} final_outcome_path={} raw_output_path={} goal={}",
                    err,
                    work_dir.display(),
                    run_id,
                    mime,
                    output_kind,
                    raw_output_chars,
                    raw_output_preview,
                    final_outcome_path.display(),
                    raw_output_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "<unwritten>".to_string()),
                    goal
                );
                (
                    AgentToolResult {
                        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                        tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                        cmd_name: None,
                        status: AgentToolStatus::Error,
                        task_id: None,
                        pending_reason: None,
                        check_after: None,
                        estimated_wait: None,
                        title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => parse_error"),
                        summary: format!("parse understanding report failed: {err}"),
                        details: json!({
                            "error": err,
                            "mime": mime,
                            "work_dir": work_dir.display().to_string(),
                            "run_id": run_id,
                            "output_kind": output_kind,
                            "raw_output": raw_output,
                            "raw_output_chars": raw_output_chars,
                            "raw_output_preview": raw_output_preview,
                            "raw_output_path": raw_output_path
                                .as_ref()
                                .map(|path| path.display().to_string()),
                            "final_outcome_path": final_outcome_path.display().to_string(),
                            "usage": usage,
                            "latency_ms": trace.latency_ms,
                        }),
                        cmd_args: None,
                        return_code: None,
                        partial_output: None,
                        output: None,
                    },
                    CLI_EXIT_ERROR,
                )
            }
        },
        LLMContextOutcome::PendingTool { pending, .. } => (
            AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Pending,
                task_id: None,
                pending_reason: Some(AgentToolPendingReason::LongRunning),
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => pending_tool"),
                summary: format!("pending {} tool call(s)", pending.len()),
                details: json!({ "pending": pending }),
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            },
            CLI_EXIT_SUCCESS,
        ),
        LLMContextOutcome::BudgetExhausted {
            which,
            partial,
            usage,
        } => (
            AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Error,
                task_id: None,
                pending_reason: None,
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => budget_exhausted"),
                summary: format!("budget exhausted ({which:?})"),
                details: json!({
                    "outcome": "budget_exhausted",
                    "which": which,
                    "usage": usage,
                }),
                cmd_args: None,
                return_code: None,
                partial_output: partial.as_ref().map(output_to_text),
                output: None,
            },
            CLI_EXIT_ERROR,
        ),
        LLMContextOutcome::Error { error, usage } => {
            log::error!(
                "llm_understand_media: llm outcome error: {}; work_dir={} run_id={} goal={}",
                error,
                work_dir.display(),
                run_id,
                goal
            );
            (
                AgentToolResult {
                    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                    tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                    cmd_name: None,
                    status: AgentToolStatus::Error,
                    task_id: None,
                    pending_reason: None,
                    check_after: None,
                    estimated_wait: None,
                    title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => error"),
                    summary: format!("llm error: {error}"),
                    details: json!({
                        "error": format!("{error}"),
                        "error_detail": serde_json::to_value(&error)
                            .unwrap_or_else(|_| json!({ "message": format!("{error}") })),
                        "mime": mime,
                        "work_dir": work_dir.display().to_string(),
                        "run_id": run_id,
                        "usage": usage,
                    }),
                    cmd_args: None,
                    return_code: None,
                    partial_output: None,
                    output: None,
                },
                CLI_EXIT_ERROR,
            )
        }
        LLMContextOutcome::ContextLimitReached { which, .. } => (
            AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Error,
                task_id: None,
                pending_reason: None,
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => context_limit_reached"),
                summary: format!("context limit surfaced unexpectedly: {which:?}"),
                details: json!({ "which": format!("{which:?}") }),
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            },
            CLI_EXIT_ERROR,
        ),
        LLMContextOutcome::Interrupted { reason, usage, .. } => (
            AgentToolResult {
                agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                cmd_name: None,
                status: AgentToolStatus::Pending,
                task_id: None,
                pending_reason: Some(AgentToolPendingReason::LongRunning),
                check_after: None,
                estimated_wait: None,
                title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => interrupted"),
                summary: format!("inference interrupted: {reason}"),
                details: json!({ "reason": reason, "usage": usage }),
                cmd_args: None,
                return_code: None,
                partial_output: None,
                output: None,
            },
            CLI_EXIT_SUCCESS,
        ),
    }
}

#[derive(Clone, Debug)]
struct MediaArg {
    source: ResourceRef,
    mime_hint: Option<String>,
}

#[derive(Clone, Debug)]
struct ResolvedMedia {
    source: ResourceRef,
    mime: String,
}

fn parse_media_arg(value: &Value) -> Result<MediaArg, String> {
    let source = serde_json::from_value::<ResourceRef>(value.clone())
        .map_err(|err| format!("invalid media ResourceRef: {err}"))?;
    let mime_hint = value
        .get("mime_hint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    Ok(MediaArg { source, mime_hint })
}

async fn resolve_media(media: &MediaArg) -> Result<ResolvedMedia, String> {
    match &media.source {
        ResourceRef::Base64 { mime, data_base64 } => {
            let mime =
                normalize_mime(mime).ok_or_else(|| "base64 media has empty mime".to_string())?;
            if data_base64.trim().is_empty() {
                return Err("base64 media has empty data_base64".to_string());
            }
            Ok(ResolvedMedia {
                source: media.source.clone(),
                mime,
            })
        }
        ResourceRef::Url { url, mime_hint } => {
            if let Some(mime) = mime_hint.as_deref().and_then(normalize_mime) {
                return Ok(ResolvedMedia {
                    source: media.source.clone(),
                    mime,
                });
            }
            if let Some(mime) = media.mime_hint.as_deref().and_then(normalize_mime) {
                return Ok(ResolvedMedia {
                    source: media.source.clone(),
                    mime,
                });
            }
            Ok(ResolvedMedia {
                source: media.source.clone(),
                mime: resolve_url_mime(url).await?,
            })
        }
        ResourceRef::NamedObject { obj_id } => {
            let masked_obj_id = masked_resource_id(&media.source);
            let runtime = get_buckyos_api_runtime()
                .map_err(|err| format!("get buckyos runtime failed: {err}"))?;
            let named_store = runtime
                .get_named_store()
                .await
                .map_err(|err| format!("get named_store failed: {err}"))?;

            let object_mime = match named_store.get_object(obj_id).await {
                Ok(object_json) => file_object_mime_from_json(&object_json),
                Err(err) => {
                    log::warn!(
                        "llm_understand_media: load named_object metadata failed: {}; obj_id={}",
                        err,
                        masked_obj_id
                    );
                    None
                }
            };
            let (mut reader, _total_size) = named_store
                .open_reader(obj_id, None)
                .await
                .map_err(|err| format!("open named_object {obj_id} reader failed: {err}"))?;
            let mut bytes = Vec::new();
            reader
                .read_to_end(&mut bytes)
                .await
                .map_err(|err| format!("read named_object {obj_id} bytes failed: {err}"))?;
            if bytes.is_empty() {
                return Err(format!("named_object {obj_id} has empty content"));
            }

            let mime = object_mime
                .clone()
                .filter(|mime| mime != "application/octet-stream")
                .or_else(|| sniff_archive_mime(&bytes).map(str::to_string))
                .or_else(|| sniff_image_mime(&bytes).map(str::to_string))
                .or_else(|| sniff_video_mime(&bytes).map(str::to_string))
                .or_else(|| sniff_document_mime(&bytes).map(str::to_string))
                .or(object_mime)
                .or_else(|| media.mime_hint.as_deref().and_then(normalize_mime))
                .ok_or_else(|| format!("cannot determine MIME for named_object {obj_id}"))?;
            let data_base64 = general_purpose::STANDARD.encode(bytes);
            Ok(ResolvedMedia {
                source: ResourceRef::Base64 {
                    mime: mime.clone(),
                    data_base64,
                },
                mime,
            })
        }
    }
}

fn file_object_mime_from_json(object_json: &str) -> Option<String> {
    let file_obj: FileObject = serde_json::from_str(object_json).ok()?;
    file_obj
        .meta
        .get("mime_type")
        .or_else(|| file_obj.meta.get("mime"))
        .and_then(Value::as_str)
        .and_then(normalize_mime)
}

fn resource_source_kind(source: &ResourceRef) -> &'static str {
    match source {
        ResourceRef::Base64 { .. } => "base64",
        ResourceRef::Url { .. } => "url",
        ResourceRef::NamedObject { .. } => "named_object",
    }
}

fn masked_resource_id(source: &ResourceRef) -> String {
    let ResourceRef::NamedObject { obj_id } = source else {
        return "<none>".to_string();
    };
    let value = obj_id.to_string();
    let (kind, payload) = value
        .split_once(':')
        .map(|(kind, payload)| (Some(kind), payload))
        .unwrap_or((None, value.as_str()));
    let chars = payload.chars().collect::<Vec<_>>();
    let masked_payload = if chars.len() <= 16 {
        payload.to_string()
    } else {
        format!(
            "{}…{}",
            chars[..8].iter().collect::<String>(),
            chars[chars.len() - 8..].iter().collect::<String>()
        )
    };
    match kind {
        Some(kind) => format!("{kind}:{masked_payload}"),
        None => masked_payload,
    }
}

async fn resolve_url_mime(url: &str) -> Result<String, String> {
    let resp = reqwest::Client::new()
        .head(url)
        .send()
        .await
        .map_err(|err| format!("fetch URL headers failed: {err}"))?;
    resp.headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(normalize_mime)
        .ok_or_else(|| "URL media has no usable Content-Type; pass mime_hint".to_string())
}

fn normalize_mime(value: &str) -> Option<String> {
    value
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

fn sniff_image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return Some("image/webp");
    }
    None
}

fn sniff_video_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.get(4..8) == Some(b"ftyp") {
        return Some("video/mp4");
    }
    if bytes.starts_with(&[0x1a, 0x45, 0xdf, 0xa3]) {
        return Some("video/webm");
    }
    None
}

fn sniff_document_mime(bytes: &[u8]) -> Option<&'static str> {
    bytes.starts_with(b"%PDF-").then_some("application/pdf")
}

fn sniff_archive_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"PK\x03\x04")
        || bytes.starts_with(b"PK\x05\x06")
        || bytes.starts_with(b"PK\x07\x08")
    {
        return Some("application/zip");
    }
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return Some("application/gzip");
    }
    if bytes.starts_with(&[0x37, 0x7a, 0xbc, 0xaf, 0x27, 0x1c]) {
        return Some("application/x-7z-compressed");
    }
    if bytes.starts_with(b"Rar!\x1a\x07") {
        return Some("application/vnd.rar");
    }
    if bytes.starts_with(b"BZh") {
        return Some("application/x-bzip2");
    }
    if bytes.starts_with(&[0xfd, 0x37, 0x7a, 0x58, 0x5a, 0x00]) {
        return Some("application/x-xz");
    }
    if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return Some("application/zstd");
    }
    (bytes.get(257..262) == Some(b"ustar")).then_some("application/x-tar")
}

fn is_image_mime(mime: &str) -> bool {
    mime.starts_with("image/")
}

fn is_video_mime(mime: &str) -> bool {
    mime.starts_with("video/")
}

fn is_archive_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/zip"
            | "application/x-7z-compressed"
            | "application/x-rar-compressed"
            | "application/vnd.rar"
            | "application/gzip"
            | "application/x-gzip"
            | "application/x-tar"
            | "application/x-compressed-tar"
            | "application/zstd"
            | "application/x-bzip2"
            | "application/x-xz"
    )
}

fn configured_model(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn route_model(mime: &str) -> Option<String> {
    let specific = if is_video_mime(mime) {
        configured_model("LLM_UNDERSTAND_MEDIA_VIDEO_MODEL")
    } else if is_image_mime(mime) {
        configured_model("LLM_UNDERSTAND_MEDIA_IMAGE_MODEL")
    } else if is_archive_mime(mime) {
        return None;
    } else {
        None
    };
    specific
        .or_else(|| configured_model("LLM_UNDERSTAND_MEDIA_MODEL"))
        .or_else(|| Some(DEFAULT_MODEL_ALIAS.to_string()))
}

async fn prepare_media_content(media: ResolvedMedia) -> Result<Vec<AiContent>, String> {
    if is_image_mime(&media.mime) {
        return Ok(vec![AiContent::image(media.source)]);
    }
    if is_archive_mime(&media.mime) {
        return Err(format!(
            "archive attachment mime `{}` must be extracted first",
            media.mime
        ));
    }
    if !is_video_mime(&media.mime) {
        return Ok(vec![AiContent::Document {
            source: media.source,
            title: Some("attachment input".to_string()),
        }]);
    }

    let frames = extract_video_frames(&media).await?;
    let mut content = Vec::with_capacity(frames.len() * 2 + 1);
    content.push(AiContent::text(format!(
        "The following {} images are representative video frames in chronological order.",
        frames.len()
    )));
    for (timestamp, frame) in frames {
        content.push(AiContent::text(format!("Frame at {timestamp:.3} seconds:")));
        content.push(AiContent::image(frame));
    }
    Ok(content)
}

async fn extract_video_frames(media: &ResolvedMedia) -> Result<Vec<(f64, ResourceRef)>, String> {
    let bytes = match &media.source {
        ResourceRef::Base64 { data_base64, .. } => general_purpose::STANDARD
            .decode(data_base64)
            .map_err(|err| format!("decode video base64 failed: {err}"))?,
        ResourceRef::Url { url, .. } => reqwest::Client::new()
            .get(url)
            .send()
            .await
            .map_err(|err| format!("download video failed: {err}"))?
            .error_for_status()
            .map_err(|err| format!("download video failed: {err}"))?
            .bytes()
            .await
            .map_err(|err| format!("read downloaded video failed: {err}"))?
            .to_vec(),
        ResourceRef::NamedObject { .. } => {
            return Err("named_object video was not resolved to bytes".to_string())
        }
    };
    if bytes.is_empty() {
        return Err("video has empty content".to_string());
    }

    let temp_dir = std::env::temp_dir().join(format!(
        "llm-understand-video-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    tokio::fs::create_dir_all(&temp_dir)
        .await
        .map_err(|err| format!("create video frame temp dir failed: {err}"))?;
    let input_path = temp_dir.join(if media.mime == "video/webm" {
        "input.webm"
    } else {
        "input.mp4"
    });
    let result = async {
        tokio::fs::write(&input_path, bytes)
            .await
            .map_err(|err| format!("write temporary video failed: {err}"))?;
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&input_path)
            .output()
            .await
            .map_err(|err| format!("start ffprobe failed: {err}"))?;
        if !probe.status.success() {
            return Err(format!(
                "ffprobe failed: {}",
                String::from_utf8_lossy(&probe.stderr).trim()
            ));
        }
        let duration = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse::<f64>()
            .map_err(|err| format!("parse video duration failed: {err}"))?;
        if !duration.is_finite() || duration <= 0.0 {
            return Err(format!("invalid video duration `{duration}`"));
        }
        let configured_count = std::env::var("LLM_UNDERSTAND_MEDIA_VIDEO_FRAMES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(DEFAULT_VIDEO_FRAME_COUNT)
            .clamp(1, MAX_VIDEO_FRAME_COUNT);
        let frame_count = configured_count
            .min((duration * 2.0).ceil() as usize)
            .max(1);
        let mut frames = Vec::with_capacity(frame_count);
        for index in 0..frame_count {
            let timestamp = duration * (index as f64 + 0.5) / frame_count as f64;
            let frame_path = temp_dir.join(format!("frame-{index:02}.jpg"));
            let output = Command::new("ffmpeg")
                .args(["-v", "error", "-y", "-ss", &format!("{timestamp:.6}")])
                .arg("-i")
                .arg(&input_path)
                .args(["-frames:v", "1", "-q:v", "2"])
                .arg(&frame_path)
                .output()
                .await
                .map_err(|err| format!("start ffmpeg failed: {err}"))?;
            if !output.status.success() {
                return Err(format!(
                    "extract video frame at {timestamp:.3}s failed: {}",
                    String::from_utf8_lossy(&output.stderr).trim()
                ));
            }
            let frame_bytes = tokio::fs::read(&frame_path)
                .await
                .map_err(|err| format!("read extracted video frame failed: {err}"))?;
            frames.push((
                timestamp,
                ResourceRef::Base64 {
                    mime: "image/jpeg".to_string(),
                    data_base64: general_purpose::STANDARD.encode(frame_bytes),
                },
            ));
        }
        Ok(frames)
    }
    .await;
    let _ = tokio::fs::remove_dir_all(&temp_dir).await;
    result
}

fn purify_history(history: &[AiMessage]) -> Vec<AiMessage> {
    history
        .iter()
        .map(|msg| AiMessage::new(msg.role, msg.content.iter().map(purify_content).collect()))
        .collect()
}

fn purify_content(content: &AiContent) -> AiContent {
    match content {
        AiContent::Image { source } => AiContent::text(media_placeholder("image", source, None)),
        AiContent::Document { source, title } => {
            AiContent::text(media_placeholder("document", source, title.as_deref()))
        }
        AiContent::ToolUse {
            call_id,
            name,
            args,
        } => AiContent::ToolUse {
            call_id: call_id.clone(),
            name: name.clone(),
            args: args
                .iter()
                .map(|(key, value)| (key.clone(), scrub_value(value)))
                .collect(),
        },
        AiContent::ToolResult {
            call_id,
            content,
            is_error,
        } => AiContent::ToolResult {
            call_id: call_id.clone(),
            content: content
                .iter()
                .map(|item| match item {
                    buckyos_api::AiToolResultContent::Text { text } => {
                        buckyos_api::AiToolResultContent::text(text.clone())
                    }
                    buckyos_api::AiToolResultContent::Image { source } => {
                        buckyos_api::AiToolResultContent::text(media_placeholder(
                            "image", source, None,
                        ))
                    }
                    buckyos_api::AiToolResultContent::Document { source, title } => {
                        buckyos_api::AiToolResultContent::text(media_placeholder(
                            "document",
                            source,
                            title.as_deref(),
                        ))
                    }
                })
                .collect(),
            is_error: *is_error,
        },
        other => other.clone(),
    }
}

fn media_placeholder(kind: &str, source: &ResourceRef, title: Option<&str>) -> String {
    let mut label = match source {
        ResourceRef::NamedObject { obj_id } => {
            format!("[media omitted: kind={kind}, obj_id={obj_id}]")
        }
        ResourceRef::Url { url, mime_hint } => {
            let mime = mime_hint.as_deref().unwrap_or("unknown");
            format!("[media omitted: kind={kind}, url={url}, mime={mime}]")
        }
        ResourceRef::Base64 { mime, data_base64 } => {
            format!(
                "[media omitted: kind={kind}, inline_base64_mime={mime}, bytes_redacted_chars={}]",
                data_base64.len()
            )
        }
    };
    if let Some(title) = title.map(str::trim).filter(|value| !value.is_empty()) {
        label.push_str(" title=");
        label.push_str(title);
    }
    label
}

fn scrub_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (key, value) in map {
                if key == "data_base64" {
                    out.insert(key.clone(), Value::String("[base64 omitted]".to_string()));
                } else {
                    out.insert(key.clone(), scrub_value(value));
                }
            }
            Value::Object(out)
        }
        Value::Array(items) => Value::Array(items.iter().map(scrub_value).collect()),
        other => other.clone(),
    }
}

fn parse_report_output(output: &ContextOutput) -> Result<UnderstandingReport, String> {
    let value = match output {
        ContextOutput::Json { content } => content.clone(),
        ContextOutput::Text { content } => parse_jsonish(content)?,
    };
    let mut report: UnderstandingReport =
        serde_json::from_value(value).map_err(|err| err.to_string())?;
    normalize_report(&mut report)?;
    Ok(report)
}

fn parse_jsonish(text: &str) -> Result<Value, String> {
    if let Ok(value) = serde_json::from_str::<Value>(text.trim()) {
        return Ok(value);
    }
    let start = text
        .find('{')
        .ok_or_else(|| "no JSON object found".to_string())?;
    let end = text
        .rfind('}')
        .ok_or_else(|| "no JSON object end found".to_string())?;
    serde_json::from_str::<Value>(&text[start..=end]).map_err(|err| err.to_string())
}

fn normalize_report(report: &mut UnderstandingReport) -> Result<(), String> {
    if report.conclusion.trim().is_empty() {
        return Err("report conclusion is empty".to_string());
    }
    if report.reasoning.trim().is_empty() {
        return Err("report reasoning is empty".to_string());
    }
    for (idx, obs) in report.observations.iter_mut().enumerate() {
        if obs.id.trim().is_empty() {
            obs.id = format!("obs-{}", idx + 1);
        }
        if obs.description.trim().is_empty() {
            return Err(format!("observation `{}` has empty description", obs.id));
        }
    }
    Ok(())
}

fn render_report(report: &UnderstandingReport) -> String {
    let mut out = String::new();
    out.push_str("Observations:\n");
    for obs in &report.observations {
        out.push_str("- ");
        out.push_str(obs.id.trim());
        out.push_str(": ");
        out.push_str(obs.description.trim());
        out.push('\n');
    }
    out.push_str("Reasoning:\n");
    out.push_str(report.reasoning.trim());
    out.push('\n');
    out.push_str("Conclusion:\n");
    out.push_str(report.conclusion.trim());
    out.push('\n');
    out.push_str("Confidence: ");
    out.push_str(match report.confidence {
        Confidence::Observed => "Observed",
        Confidence::Inferred => "Inferred",
        Confidence::Uncertain => "Uncertain",
    });
    out
}

fn output_to_text(output: &ContextOutput) -> String {
    match output {
        ContextOutput::Text { content } => content.clone(),
        ContextOutput::Json { content } => {
            serde_json::to_string_pretty(content).unwrap_or_else(|_| content.to_string())
        }
    }
}

fn context_output_kind(output: &ContextOutput) -> &'static str {
    match output {
        ContextOutput::Text { .. } => "text",
        ContextOutput::Json { .. } => "json",
    }
}

fn run_final_outcome_path(work_dir: &Path, run_id: &str) -> PathBuf {
    work_dir
        .join("runs")
        .join(run_id)
        .join("outcomes")
        .join("final.json")
}

fn write_parse_error_raw_output(
    work_dir: &Path,
    run_id: &str,
    raw_output: &str,
) -> std::io::Result<PathBuf> {
    let outcomes_dir = work_dir.join("runs").join(run_id).join("outcomes");
    std::fs::create_dir_all(&outcomes_dir)?;
    let path = outcomes_dir.join("parse_error_raw_output.txt");
    std::fs::write(&path, raw_output)?;
    Ok(path)
}

fn report_schema() -> Value {
    json!({
        "type": "object",
        "required": ["observations", "reasoning", "conclusion", "confidence"],
        "additionalProperties": false,
        "properties": {
            "observations": {
                "type": "array",
                "items": {
                    "type": "object",
                    "required": ["id", "description"],
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "string" },
                        "description": { "type": "string" }
                    }
                }
            },
            "reasoning": { "type": "string" },
            "conclusion": { "type": "string" },
            "confidence": { "type": "string", "enum": ["Observed", "Inferred", "Uncertain"] }
        }
    })
}

fn build_error_result(opts: &RunOpts, message: impl Into<String>) -> AgentToolResult {
    let message = message.into();
    AgentToolResult {
        agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
        tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
        cmd_name: None,
        status: AgentToolStatus::Error,
        task_id: None,
        pending_reason: None,
        check_after: None,
        estimated_wait: None,
        title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => error"),
        summary: message.clone(),
        details: json!({
            "error": message,
            "goal": opts.goal,
        }),
        cmd_args: None,
        return_code: None,
        partial_output: None,
        output: None,
    }
}

fn add_run_context(
    result: &mut AgentToolResult,
    work_dir: &PathBuf,
    run_id: &str,
    mime: Option<&str>,
) {
    if let Value::Object(map) = &mut result.details {
        map.insert(
            "work_dir".to_string(),
            Value::String(work_dir.display().to_string()),
        );
        map.insert("run_id".to_string(), Value::String(run_id.to_string()));
        if let Some(mime) = mime {
            map.insert("mime".to_string(), Value::String(mime.to_string()));
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

fn default_work_dir(goal: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let mut safe = String::new();
    for ch in goal.chars().take(32) {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            safe.push(ch);
        } else if ch.is_whitespace() {
            safe.push('_');
        }
    }
    if safe.is_empty() {
        safe.push_str("media");
    }
    std::env::temp_dir().join(format!("llm_understand_media-{ts}-{safe}"))
}

struct NoopToolManager;

#[async_trait]
impl ToolManager for NoopToolManager {
    async fn call_tool(&self, call: buckyos_api::AiToolCall) -> llm_context::Observation {
        llm_context::Observation::Error {
            call_id: call.call_id,
            message: "tools are disabled in llm_understand_media".to_string(),
            tool_result: None,
        }
    }
}

const USAGE: &str = r#"Usage: agent_tool llm_understand_media --media <json> --goal <text> [options]

Required:
  --media <json>          ResourceRef JSON, e.g. {"kind":"named_object","obj_id":"...","mime_hint":"image/png"}
  --goal <text>           Understanding goal.

Options:
  --history-file <path>   JSON Vec<AiMessage> parent history snapshot.
  --work-dir <path>       LocalLLMContext working directory.
  --model <alias>         AICC logical model alias; default image route is llm.vision.
  --max-completion-tokens <n>  Positive output budget; default 2048, rounded up to 2048/4096/8192 tiers.
  -h, --help              Show this help.
"#;

#[derive(Debug)]
struct CliOpts {
    media: Value,
    goal: String,
    history_file: Option<PathBuf>,
    work_dir: Option<PathBuf>,
    model: Option<String>,
    max_completion_tokens: Option<u32>,
}

impl CliOpts {
    async fn into_run_opts(self) -> Result<RunOpts, String> {
        let parent_history = match self.history_file.as_ref() {
            Some(path) => {
                let content = tokio::fs::read_to_string(path).await.map_err(|err| {
                    format!("read history file `{}` failed: {err}", path.display())
                })?;
                serde_json::from_str::<Vec<AiMessage>>(&content).map_err(|err| {
                    format!("parse history file `{}` failed: {err}", path.display())
                })?
            }
            None => Vec::new(),
        };
        Ok(RunOpts {
            media_value: self.media,
            goal: self.goal,
            parent_history,
            work_dir: self.work_dir,
            model: self.model,
            summary_model: DEFAULT_SUMMARY_MODEL_ALIAS.to_string(),
            target_tokens: DEFAULT_TARGET_TOKENS,
            max_completion_tokens: normalize_completion_tokens(
                self.max_completion_tokens
                    .unwrap_or(DEFAULT_MAX_COMPLETION_TOKENS),
            ),
        })
    }

    fn parse(args: &[String]) -> Result<Self, ParseError> {
        let mut media: Option<Value> = None;
        let mut goal: Option<String> = None;
        let mut history_file: Option<PathBuf> = None;
        let mut work_dir: Option<PathBuf> = None;
        let mut model: Option<String> = None;
        let mut max_completion_tokens = None;

        let mut idx = 0;
        while idx < args.len() {
            match args[idx].as_str() {
                "-h" | "--help" => return Err(ParseError::Help),
                "--media" => {
                    let raw = next_value(args, &mut idx, "--media")?;
                    media = Some(serde_json::from_str(&raw).map_err(|err| {
                        ParseError::Bad(format!("--media must be JSON ResourceRef: {err}"))
                    })?);
                }
                "--goal" => goal = Some(next_value(args, &mut idx, "--goal")?),
                "--history-file" => {
                    history_file =
                        Some(PathBuf::from(next_value(args, &mut idx, "--history-file")?));
                }
                "--work-dir" => {
                    work_dir = Some(PathBuf::from(next_value(args, &mut idx, "--work-dir")?));
                }
                "--model" => model = Some(next_value(args, &mut idx, "--model")?),
                "--max-completion-tokens" => {
                    max_completion_tokens = Some(parse_positive_u32(
                        &next_value(args, &mut idx, "--max-completion-tokens")?,
                        "--max-completion-tokens",
                    )?);
                }
                other => return Err(ParseError::Bad(format!("unknown flag `{other}`"))),
            }
            idx += 1;
        }

        Ok(Self {
            media: media.ok_or_else(|| ParseError::Bad("missing --media".to_string()))?,
            goal: goal
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| ParseError::Bad("missing non-empty --goal".to_string()))?,
            history_file,
            work_dir,
            model,
            max_completion_tokens,
        })
    }
}

enum ParseError {
    Help,
    Bad(String),
}

fn next_value(args: &[String], idx: &mut usize, flag: &str) -> Result<String, ParseError> {
    *idx += 1;
    args.get(*idx)
        .cloned()
        .ok_or_else(|| ParseError::Bad(format!("{flag} requires a value")))
}

fn parse_positive_u32(raw: &str, flag: &str) -> Result<u32, ParseError> {
    let value = raw
        .parse::<u32>()
        .map_err(|_| ParseError::Bad(format!("{flag} must be an integer")))?;
    if value == 0 {
        return Err(ParseError::Bad(format!("{flag} must be greater than 0")));
    }
    Ok(value)
}

fn normalize_completion_tokens(value: u32) -> u32 {
    match value {
        1..=2_048 => 2_048,
        2_049..=4_096 => 4_096,
        4_097..=8_192 => 8_192,
        _ => value,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{AiResponse, AiToolResultContent, AiUsage};

    #[test]
    fn purify_history_omits_media_payloads() {
        let history = vec![AiMessage::new(
            AiRole::User,
            vec![
                AiContent::text("see this"),
                AiContent::image(ResourceRef::Base64 {
                    mime: "image/png".to_string(),
                    data_base64: "AAAA".repeat(20),
                }),
            ],
        )];

        let out = purify_history(&history);
        let text = serde_json::to_string(&out).unwrap();
        assert!(text.contains("media omitted"));
        assert!(!text.contains("AAAA"));
        assert!(!matches!(out[0].content[1], AiContent::Image { .. }));
    }

    #[test]
    fn purify_tool_result_media_to_text() {
        let history = vec![AiMessage::new(
            AiRole::Tool,
            vec![AiContent::ToolResult {
                call_id: "c1".to_string(),
                content: vec![AiToolResultContent::Image {
                    source: ResourceRef::Url {
                        url: "https://example.test/a.png".to_string(),
                        mime_hint: Some("image/png".to_string()),
                    },
                }],
                is_error: false,
            }],
        )];

        let out = purify_history(&history);
        let AiContent::ToolResult { content, .. } = &out[0].content[0] else {
            panic!("expected tool result");
        };
        assert!(matches!(content[0], AiToolResultContent::Text { .. }));
    }

    #[test]
    fn render_report_is_compact_text() {
        let report = UnderstandingReport {
            observations: vec![ObservationItem {
                id: "obs-1".to_string(),
                description: "A red error banner is visible.".to_string(),
            }],
            reasoning: "obs-1 indicates an error state.".to_string(),
            conclusion: "The screen shows an error.".to_string(),
            confidence: Confidence::Observed,
        };
        let text = render_report(&report);
        assert!(text.contains("Observations:"));
        assert!(text.contains("Confidence: Observed"));
    }

    #[test]
    fn masked_resource_id_preserves_type_and_masks_payload() {
        let source: ResourceRef = serde_json::from_value(json!({
            "kind": "named_object",
            "obj_id": "cyfile:d05ec9f1d9ff3713fda374c2ad1e1a9ff1b168c941772df29769935fbbb49fdd"
        }))
        .unwrap();
        assert_eq!(masked_resource_id(&source), "cyfile:d05ec9f1…bbb49fdd");
    }

    #[test]
    fn build_request_disables_web_search_for_media_side_context() {
        let opts = RunOpts {
            media_value: json!({}),
            goal: "describe image".to_string(),
            parent_history: Vec::new(),
            work_dir: None,
            model: None,
            summary_model: DEFAULT_SUMMARY_MODEL_ALIAS.to_string(),
            target_tokens: DEFAULT_TARGET_TOKENS,
            max_completion_tokens: DEFAULT_MAX_COMPLETION_TOKENS,
        };

        let request = build_request(
            &opts,
            vec![AiContent::image(ResourceRef::Base64 {
                mime: "image/png".to_string(),
                data_base64: "AAAA".to_string(),
            })],
            DEFAULT_MODEL_ALIAS.to_string(),
            Vec::new(),
        );

        let model_policy = request.model_policy.expect("model policy");
        assert_eq!(model_policy.max_completion_tokens, Some(2_048));
        assert_eq!(model_policy.provider_options, None);
        let tool_policy = request.tool_policy.expect("tool policy");
        assert_eq!(tool_policy.mode, ToolMode::None);
        assert_eq!(tool_policy.action_mode, ToolMode::None);
        assert!(tool_policy
            .disable_capabilities
            .contains(&"web_search".to_string()));
    }

    #[test]
    fn caller_can_raise_media_output_budget() {
        let opts = RunOpts::from_tool_args(json!({
            "media": { "kind": "url", "url": "https://example.test/video.mp4" },
            "goal": "produce a detailed timeline",
            "max_completion_tokens": 16384
        }))
        .expect("caller-selected media budget should be accepted");
        let request = build_request(
            &opts,
            vec![AiContent::image(ResourceRef::Base64 {
                mime: "image/jpeg".to_string(),
                data_base64: "AAAA".to_string(),
            })],
            DEFAULT_MODEL_ALIAS.to_string(),
            Vec::new(),
        );
        assert_eq!(
            request.model_policy.unwrap().max_completion_tokens,
            Some(16_384)
        );
        assert_eq!(
            request.budget.unwrap().max_total_tokens,
            Some(DEFAULT_TARGET_TOKENS + 16_384)
        );
    }

    #[test]
    fn omitted_media_output_budget_uses_default() {
        let opts = RunOpts::from_tool_args(json!({
            "media": { "kind": "url", "url": "https://example.test/image.png" },
            "goal": "describe the image"
        }))
        .expect("omitted media budget should use the default");
        assert_eq!(opts.max_completion_tokens, DEFAULT_MAX_COMPLETION_TOKENS);
    }

    #[test]
    fn media_tool_schema_requires_caller_budget() {
        let spec = LlmUnderstandMediaTool::new().spec();
        let required = spec.args_schema["required"]
            .as_array()
            .expect("required array");
        assert!(required.contains(&json!("media")));
        assert!(required.contains(&json!("goal")));
        assert!(required.contains(&json!("max_completion_tokens")));
        assert_eq!(spec.args_schema["additionalProperties"], json!(false));
    }

    #[test]
    fn media_output_budget_rounds_up_to_staged_floor() {
        assert_eq!(normalize_completion_tokens(1), 2_048);
        assert_eq!(normalize_completion_tokens(2_047), 2_048);
        assert_eq!(normalize_completion_tokens(2_048), 2_048);
        assert_eq!(normalize_completion_tokens(2_049), 4_096);
        assert_eq!(normalize_completion_tokens(4_096), 4_096);
        assert_eq!(normalize_completion_tokens(4_097), 8_192);
        assert_eq!(normalize_completion_tokens(8_192), 8_192);
        assert_eq!(normalize_completion_tokens(8_193), 8_193);
        assert_eq!(normalize_completion_tokens(16_384), 16_384);
    }

    #[test]
    fn non_archive_attachment_mime_routes_to_model_and_sniffs_common_containers() {
        assert!(route_model("video/mp4").is_some());
        assert!(route_model("audio/mpeg").is_some());
        assert!(route_model("application/pdf").is_some());
        assert!(route_model("text/plain").is_some());
        assert!(route_model("application/octet-stream").is_some());
        assert!(route_model("application/zip").is_none());
        assert_eq!(sniff_video_mime(b"\0\0\0\x18ftypisom"), Some("video/mp4"));
        assert_eq!(
            sniff_video_mime(&[0x1a, 0x45, 0xdf, 0xa3, 0x01]),
            Some("video/webm")
        );
        assert_eq!(sniff_document_mime(b"%PDF-1.7"), Some("application/pdf"));
        assert_eq!(
            sniff_archive_mime(b"PK\x03\x04archive"),
            Some("application/zip")
        );
    }

    #[tokio::test]
    async fn non_archive_attachments_are_forwarded_inline() {
        for mime in [
            "audio/mpeg",
            "application/pdf",
            "text/plain",
            "application/octet-stream",
        ] {
            let content = prepare_media_content(ResolvedMedia {
                source: ResourceRef::Base64 {
                    mime: mime.to_string(),
                    data_base64: "AAAA".to_string(),
                },
                mime: mime.to_string(),
            })
            .await
            .expect("non-archive attachment should be forwarded");
            assert!(matches!(
                &content[0],
                AiContent::Document {
                    source: ResourceRef::Base64 { mime: actual, .. },
                    ..
                } if actual == mime
            ));
        }

        let err = prepare_media_content(ResolvedMedia {
            source: ResourceRef::Base64 {
                mime: "application/zip".to_string(),
                data_base64: "AAAA".to_string(),
            },
            mime: "application/zip".to_string(),
        })
        .await
        .expect_err("archives must be extracted before understanding");
        assert!(err.contains("must be extracted first"));
    }

    #[test]
    fn build_request_preserves_timestamped_video_frames() {
        let opts = RunOpts {
            media_value: json!({}),
            goal: "find the action time".to_string(),
            parent_history: Vec::new(),
            work_dir: None,
            model: None,
            summary_model: DEFAULT_SUMMARY_MODEL_ALIAS.to_string(),
            target_tokens: DEFAULT_TARGET_TOKENS,
            max_completion_tokens: DEFAULT_MAX_COMPLETION_TOKENS,
        };
        let media_content = vec![
            AiContent::text("Frame at 1.250 seconds:"),
            AiContent::image(ResourceRef::Base64 {
                mime: "image/jpeg".to_string(),
                data_base64: "AAAA".to_string(),
            }),
        ];
        let request = build_request(
            &opts,
            media_content,
            DEFAULT_MODEL_ALIAS.to_string(),
            Vec::new(),
        );
        let user = request.input.last().expect("user message");
        assert!(matches!(
            &user.content[0],
            AiContent::Text { text } if text.contains("1.250 seconds")
        ));
        assert!(matches!(&user.content[1], AiContent::Image { .. }));
    }

    #[test]
    fn parse_error_result_persists_raw_output_diagnostics() {
        let work_dir = std::env::temp_dir().join(format!(
            "llm_understand_media-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let run_id = "20260524-234831-test";
        let raw = "{\n  \"observations\": [";
        let outcome = LLMContextOutcome::Done {
            reason: None,
            output: ContextOutput::Text {
                content: raw.to_string(),
            },
            usage: AiUsage {
                input_tokens: Some(1),
                output_tokens: Some(2),
                total_tokens: Some(3),
                request_units: None,
            },
            response: AiResponse::text(raw),
            trace: llm_context::ContextRunTrace {
                trace_id: run_id.to_string(),
                latency_ms: 12,
                tool_trace: Vec::new(),
                llm_task_ids: Vec::new(),
            },
            behavior_result: None,
        };

        let (result, exit_code) = build_outcome_result(
            outcome,
            "image/png",
            &work_dir,
            run_id,
            "describe image",
            "cyfile:12345678…90abcdef",
        );

        assert_eq!(exit_code, CLI_EXIT_ERROR);
        assert_eq!(result.status, AgentToolStatus::Error);
        assert_eq!(result.details["output_kind"], "text");
        assert_eq!(result.details["raw_output"], raw);
        let raw_output_path = result.details["raw_output_path"].as_str().unwrap();
        assert_eq!(std::fs::read_to_string(raw_output_path).unwrap(), raw);
        assert!(result.details["final_outcome_path"]
            .as_str()
            .unwrap()
            .ends_with("outcomes/final.json"));

        let _ = std::fs::remove_dir_all(work_dir);
    }
}
