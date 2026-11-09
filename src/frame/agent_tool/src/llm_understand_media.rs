use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use base64::{engine::general_purpose, Engine as _};
use buckyos_api::{
    get_buckyos_api_runtime, AiContent, AiMessage, AiMethodStatus, AiRole, ApiType,
    AudioSpeechRecognitionRequest, ResourceRef, RouteResolveRequest, ThunkExecutionError,
};
use llm_context::deps::{LLMContextDeps, ToolManager};
use llm_context::{ContextOutput, LlmClient};
use ndn_lib::ObjId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

mod catalog;
mod pipeline;
mod terms;
mod zone_file;

use catalog::{CatalogFile, CatalogSource, CatalogTarget};
use pipeline::{
    audio_entry, choose_text, document_format_from_mime, document_format_from_name,
    frame_entries, freeze_reader, goal_pages, goal_terms, image_dimensions, normalize_text,
    object_head, parse_pages_arg, plan_pages, progress,
    protocol_accepts_documents, read_manifest, read_object_bytes, run_pipeline,
    window_done_line, Coverage, DocumentLimits, ImageLimits, PageText, PipelineOutcome,
    PipelineRequest, PipelineTrace, PlanRequest, VideoLimits, WindowWatch,
    CATALOG_WINDOW_PAGES, FAMILY_DOCUMENT, FAMILY_IMAGE, FAMILY_PDF, FAMILY_VIDEO,
    MAX_INPUT_BYTES,
};

use crate::xllm::{
    ensure_buckyos_runtime, AiccLlmClient, LoopModel, RunOutcome, TaskInput, TaskOverrides,
    XllmDeps, XllmRun, XllmTask,
};
use crate::{
    cli_error_result, llm_compress, render_cli_output, AgentTool, AgentToolError,
    AgentToolResult, AgentToolStatus, CallingConventions,
    SessionRuntimeContext, ToolSpec, AGENT_TOOL_PROTOCOL_VERSION, CLI_EXIT_ERROR, CLI_EXIT_SUCCESS,
    CLI_EXIT_USAGE,
};

pub const TOOL_LLM_UNDERSTAND_MEDIA: &str = "llm_understand_media";

const DEFAULT_MODEL_ALIAS: &str = "llm.vision";
const DEFAULT_AUDIO_ANALYSIS_MODEL_ALIAS: &str = "llm.chat";
const DEFAULT_AUDIO_ASR_MODEL_ALIAS: &str = "audio.asr";
const DEFAULT_SUMMARY_MODEL_ALIAS: &str = llm_compress::SUMMARIZE_MODEL_ALIAS;
const DEFAULT_TARGET_TOKENS: u32 = 24_000;
const DEFAULT_MAX_COMPLETION_TOKENS: u32 = 2_048;
const RAW_OUTPUT_LOG_PREVIEW_CHARS: usize = 2_000;
const PROBE_HEAD_BYTES: usize = 4096;
const DEFAULT_TEXT_CHAR_BUDGET: usize = 60_000;
const DEFAULT_MAX_IMAGES: usize = 16;
const MAX_INLINE_TEXT_BYTES: u64 = 2 * 1024 * 1024;
/// AICC's `ResourceLimits::max_inline_bytes`: more than this must be an object.
const MAX_MODEL_INLINE_BYTES: u64 = 4 * 1024 * 1024;
const TEXT_READ_CONCURRENCY: usize = 8;

const SYSTEM_PROMPT: &str = r#"You are OpenDAN's controlled media-understanding side context.

You must inspect the target media resource and answer the requested goal as a JSON object with exactly these fields:
- observations: array of objects with id, description and where.
- reasoning: string.
- conclusion: string.
- confidence: one of "Observed", "Inferred", "Uncertain".

Rules:
1. Produce observations first in causal order. Observations are objective facts observable in the media resource. Each observation must have a stable id such as "obs-1".
2. Reasoning must come after observations and must only cite facts that trace to observation ids. If a step needs information not in observations, mark it as speculation.
3. Conclusions that cannot be derived only from observations must be marked in reasoning as speculation and reflected by confidence "Inferred" or "Uncertain".
4. Do not invent media details to support a likely answer.
5. For audio, distinguish clearly intelligible speech from a sound that merely resembles speech. An exact transcription is "Observed" only when the words are clearly audible and supported by an observation that explicitly states the speech is unambiguous. If the clip is short, noisy, ambiguous, or could instead be a non-speech sound, mark any proposed transcription "Uncertain" and present it only as a candidate, not as an observed fact.
6. A document may arrive as a catalog: an index of its pages, then the text and page images of the pages selected for this goal. Read the index, then the text, then the images. The index is not the full text: do not claim to have read pages you were not shown, and say so when the answer may be on one of them.
7. When the media has pages or a timeline, give each observation a "where" such as "page 12" or "00:01:05"; otherwise set it to null.
8. Return only JSON. Do not call tools."#;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ObservationItem {
    pub id: String,
    pub description: String,
    /// Page or timestamp the observation comes from.
    #[serde(rename = "where", default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
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
            description: "Understand a media resource through a controlled LLM side context. Images, PDFs, office documents (docx/pptx/odt/...) and videos the model cannot take directly are converted by the system media pipeline first; the call waits for the conversion. Archives must be extracted first. media is either a stored object ({kind:\"named_object\", obj_id:\"cyfile:…\"}), a Zone file ({url:\"cyfs:///home/<owner>/…\"}), a URL, or a local file ({kind:\"local_file\", path:\"/abs/path\"}); use the local-file form for artifacts an earlier shell produced.".to_string(),
            args_schema: json!({
                "type": "object",
                "additionalProperties": false,
                "required": ["media", "goal", "max_completion_tokens"],
                "properties": {
                    "media": {
                        "type": "object",
                        "description": "Media locator. {kind:\"named_object\", obj_id:\"cyfile:…\"} for a stored object, {kind:\"local_file\", path:\"/abs/path\"} for a file on this host (for example a frame written by a previous shell), or {url:\"…\"} for a Zone file (cyfs:///<path>, or the path Files shows, /home/<owner>/…), a cyfs://<objid>/ or https URL. A bare path, object id or url string is also accepted. mime_hint is optional."
                    },
                    "goal": {
                        "type": "string",
                        "description": "What to understand or answer from the media."
                    },
                    "max_completion_tokens": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Token budget for the media model to complete this task."
                    },
                    "pages": {
                        "type": "string",
                        "description": "Pages of a document to read, e.g. \"14,50,70\" or \"2-33\"; also taken from the goal (\"page 12\")."
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
        ctx: &SessionRuntimeContext,
        args: Value,
    ) -> Result<AgentToolResult, AgentToolError> {
        let mut opts = RunOpts::from_tool_args(args)?;
        opts.session_id = Some(ctx.session_id.clone());
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

    pipeline::enable_progress();
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
    /// Session whose workspace relative local paths resolve against. Set by
    /// `AgentTool::call`; the CLI leaves it unset.
    session_id: Option<String>,
    goal: String,
    parent_history: Vec<AiMessage>,
    work_dir: Option<PathBuf>,
    model: Option<String>,
    summary_model: String,
    target_tokens: u32,
    max_completion_tokens: u32,
    /// `--pages`: pages of a document to read, with those the goal names.
    pages: BTreeSet<u64>,
}

impl RunOpts {
    /// Pages to read: `--pages` and the pages the goal names.
    fn named_pages(&self) -> BTreeSet<u64> {
        let mut pages = goal_pages(&self.goal);
        pages.extend(self.pages.iter().copied());
        pages
    }

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
        let pages = match map.get("pages") {
            None | Some(Value::Null) => BTreeSet::new(),
            Some(Value::String(raw)) if raw.trim().is_empty() => BTreeSet::new(),
            Some(Value::String(raw)) => parse_pages_arg(raw).map_err(AgentToolError::InvalidArgs)?,
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_u64().filter(|p| *p > 0))
                .collect::<Option<BTreeSet<u64>>>()
                .ok_or_else(|| AgentToolError::InvalidArgs("pages must be page numbers from 1".to_string()))?,
            Some(other) => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "pages must be a string such as \"1,5,10-20\", got {}",
                    json_type_name(other)
                )))
            }
        };
        Ok(Self {
            media_value,
            session_id: None,
            goal,
            parent_history,
            work_dir,
            model,
            summary_model,
            target_tokens,
            max_completion_tokens,
            pages,
        })
    }
}

async fn run(opts: RunOpts) -> (AgentToolResult, i32) {
    let media = match parse_media_arg(&opts.media_value) {
        Ok(media) => media,
        Err(err) => return (build_error_result(&opts, err), CLI_EXIT_USAGE),
    };
    let input_source_kind = media.source.kind();
    log::debug!(
        "llm_understand_media: input media {} ({input_source_kind})",
        media.source.display_id()
    );

    if let Err(err) = ensure_buckyos_runtime().await {
        return (
            build_error_result(&opts, format!("init buckyos runtime failed: {err}")),
            CLI_EXIT_ERROR,
        );
    }

    if let MediaSource::LocalFile(path) = &media.source {
        let root = catalog::catalog_root();
        if let Some(target) = catalog::recognize(path, &root) {
            let changed = matches!(target, CatalogTarget::Page { .. }) && catalog::page_image_changed(target.dir(), path);
            if !changed {
                return run_catalog(&opts, target).await;
            }
        }
    }

    let (resolved_media, zone_file) = match resolve_media(&media, opts.session_id.as_deref()).await {
        Ok(resolved) => resolved,
        Err(err) => return (build_error_result(&opts, err), CLI_EXIT_ERROR),
    };
    let mime = resolved_media.mime.clone();
    let media_id = resolved_media.label();

    let model_alias = match opts.model.clone().or_else(|| route_model(&mime)) {
        Some(model) => model,
        None => {
            return (
                build_error_result(&opts, format!("no model route for media mime `{mime}`")),
                CLI_EXIT_ERROR,
            );
        }
    };

    let mut prepared = match prepare_media_content(&resolved_media, &opts, &model_alias).await {
        Ok(prepared) => prepared,
        Err(err) => {
            let mut result = build_error_result(&opts, err.message);
            attach_pipeline_details(&mut result, Some(&err.decision), err.trace.as_ref(), None, None, None, None);
            attach_zone_file(&mut result, zone_file.as_ref());
            return (result, CLI_EXIT_ERROR);
        }
    };

    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new());
    let deps = LLMContextDeps::new(llm.clone(), Arc::new(NoopToolManager));
    let purified = purify_history(&opts.parent_history);
    let history = match llm_compress::compress(
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

    let work_dir = opts
        .work_dir
        .clone()
        .unwrap_or_else(|| default_work_dir(&opts.goal));
    let analysis = Analysis {
        opts: &opts,
        history: &history,
        llm: &llm,
        model_alias: &model_alias,
        work_dir: &work_dir,
        mime: &mime,
        media_id: &media_id,
    };
    if let Some(reading) = &prepared.reading {
        progress(format!("reading {reading} with {model_alias}"));
    }
    let job = prepared.catalog_job.take();
    let unpack = async {
        match job {
            Some(job) => Some(catalog::write(&job.root, &job.file, &job.texts).await),
            None => None,
        }
    };
    let ((mut result, mut code), unpacked) = tokio::join!(analysis.run(&prepared), unpack);
    match unpacked {
        Some(Ok(dir)) => progress(format!("catalog written to {}", dir.display())),
        Some(Err(err)) => {
            log::warn!("llm_understand_media: unpacking the catalog failed: {err}");
            progress(format!("the catalog could not be unpacked: {err}"));
            prepared.catalog_dir = None;
        }
        None => {}
    }
    if prepared.convert_if_refused && code == CLI_EXIT_ERROR && result.details.get("outcome").is_some() {
        log::warn!(
            "llm_understand_media: the model refused the document directly ({}); media_id={media_id}",
            result.summary
        );
        let reason = format!("sent directly and failed: {}", result.summary);
        match prepare_document(&resolved_media, &opts, None, reason).await {
            Ok(converted) => {
                prepared = converted;
                if let Some(job) = prepared.catalog_job.take() {
                    match catalog::write(&job.root, &job.file, &job.texts).await {
                        Ok(dir) => progress(format!("catalog written to {}", dir.display())),
                        Err(err) => {
                            log::warn!("llm_understand_media: unpacking the catalog failed: {err}");
                            prepared.catalog_dir = None;
                        }
                    }
                }
                (result, code) = analysis.run(&prepared).await;
            }
            Err(err) => log::warn!("llm_understand_media: conversion after the refusal failed: {}", err.message),
        }
    }
    attach_pipeline_details(
        &mut result,
        Some(&prepared.decision),
        prepared.trace.as_ref(),
        prepared.coverage.as_ref(),
        prepared.preprocess.as_ref(),
        prepared.caller_note.as_deref(),
        prepared.catalog_dir.as_deref(),
    );
    attach_extra_details(&mut result, &prepared);
    attach_zone_file(&mut result, zone_file.as_ref());
    if opts.work_dir.is_none() && result.status == AgentToolStatus::Success {
        if let Err(err) = std::fs::remove_dir_all(&work_dir) {
            log::warn!("llm_understand_media: remove {} failed: {err}", work_dir.display());
        }
    }
    (result, code)
}

/// One model analysis of prepared media: a run in `work_dir` that dies with it.
struct Analysis<'a> {
    opts: &'a RunOpts,
    history: &'a [AiMessage],
    llm: &'a Arc<dyn LlmClient>,
    model_alias: &'a str,
    work_dir: &'a PathBuf,
    mime: &'a str,
    media_id: &'a str,
}

impl Analysis<'_> {
    async fn run(&self, prepared: &PreparedMedia) -> (AgentToolResult, i32) {
        let opts = self.opts;
        log::info!(
            "llm_understand_media: media decision; media_id={} mime={} decision={} reason={} run_id={:?}",
            self.media_id,
            self.mime,
            prepared.decision.kind,
            prepared.decision.reason,
            prepared.trace.as_ref().map(|t| t.run_id.as_str())
        );
        let input = build_input(opts, prepared.content.clone(), self.history.to_vec());
        let overrides = media_overrides(opts, self.model_alias.to_string(), self.work_dir.clone());
        let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
        let xllm_deps = XllmDeps::default().with_llm(self.llm.clone());
        let task = match XllmTask::prepare(&cwd, TaskInput::structured(input), overrides, &xllm_deps).await {
            Ok(task) => task,
            Err(err) => {
                return (
                    build_error_result(opts, format!("prepare task failed: {err}")),
                    CLI_EXIT_ERROR,
                );
            }
        };
        let mut run = match XllmRun::start(task, xllm_deps).await {
            Ok(run) => run,
            Err(err) => {
                return (
                    build_error_result(opts, format!("start run failed: {err}")),
                    CLI_EXIT_ERROR,
                );
            }
        };
        let run_id = run.run_id().to_string();
        eprintln!(
            "llm_understand_media: work_dir={} run_id={}",
            self.work_dir.display(),
            run_id
        );
        log::info!(
            "llm_understand_media: started; work_dir={} run_id={} mime={} media_id={} model={} goal={}",
            self.work_dir.display(),
            run_id,
            self.mime,
            self.media_id,
            self.model_alias,
            opts.goal
        );
        match run.execute().await {
            Ok(outcome) => build_outcome_result(outcome, self.mime, self.work_dir, &run_id, &opts.goal, self.media_id),
            Err(err) => {
                let message = format!("execute failed: {err}");
                log::error!(
                    "llm_understand_media: {}; work_dir={} run_id={} goal={}",
                    message,
                    self.work_dir.display(),
                    run_id,
                    opts.goal
                );
                let mut result = build_error_result(opts, message);
                add_run_context(&mut result, self.work_dir, &run_id, Some(self.mime));
                if let Value::Object(map) = &mut result.details {
                    map.insert("outcome".to_string(), Value::String("failed".to_string()));
                }
                (result, CLI_EXIT_ERROR)
            }
        }
    }
}

/// `details.media_decision` / `pipeline_trace` / `coverage` /
/// `media_preprocess` (doc/llm_context/LLM Understand Media.md §4.7); a
/// downscale or uncovered pages are also appended to the output for the
/// calling agent.
fn attach_pipeline_details(
    result: &mut AgentToolResult,
    decision: Option<&MediaDecision>,
    trace: Option<&PipelineTrace>,
    coverage: Option<&Coverage>,
    preprocess: Option<&Value>,
    caller_note: Option<&str>,
    catalog_dir: Option<&Path>,
) {
    if let Value::Object(map) = &mut result.details {
        if let Some(decision) = decision {
            map.insert("media_decision".to_string(), decision.to_value());
        }
        if let Some(trace) = trace {
            map.insert(
                "pipeline_trace".to_string(),
                serde_json::to_value(trace).unwrap_or(Value::Null),
            );
        }
        if let Some(coverage) = coverage {
            map.insert(
                "coverage".to_string(),
                serde_json::to_value(coverage).unwrap_or(Value::Null),
            );
        }
        if let Some(preprocess) = preprocess {
            map.insert("media_preprocess".to_string(), preprocess.clone());
        }
    }
    let mut notes = Vec::new();
    if let Some(note) = caller_note {
        notes.push(note.to_string());
    }
    match coverage.and_then(|c| c.uncovered_note(catalog_dir)) {
        Some(note) => notes.push(note),
        None => {
            if let Some(dir) = catalog_dir {
                notes.push(format!(
                    "The document is unpacked in {} (index.tsv lists every page; pages/ holds each page's text and image).",
                    dir.display()
                ));
            }
        }
    }
    if let (Some(dir), Value::Object(map)) = (catalog_dir, &mut result.details) {
        map.insert("catalog".to_string(), json!({"dir": dir.display().to_string()}));
    }
    if let (Some(output), false) = (result.output.as_mut(), notes.is_empty()) {
        for note in notes {
            output.push('\n');
            output.push_str(&note);
        }
    }
}

/// `details.retrieval` (how pages were searched) and `details.media_source`
/// of a catalog page.
fn attach_extra_details(result: &mut AgentToolResult, prepared: &PreparedMedia) {
    if let Value::Object(map) = &mut result.details {
        if let Some(retrieval) = &prepared.retrieval {
            map.insert("retrieval".to_string(), retrieval.clone());
        }
        if let Some(source) = &prepared.media_source {
            map.insert("media_source".to_string(), source.clone());
        }
    }
}

/// `details.media_source`: the Zone file read, so the caller can name it; a
/// reading other than the path as written is also told in the output.
fn attach_zone_file(result: &mut AgentToolResult, found: Option<&zone_file::ZoneFile>) {
    let Some(found) = found else {
        return;
    };
    if let Value::Object(map) = &mut result.details {
        map.insert(
            "media_source".to_string(),
            json!({ "url": found.url, "path": found.path.display().to_string() }),
        );
    }
    if let (Some(note), Some(output)) = (found.note.as_deref(), result.output.as_mut()) {
        output.push('\n');
        output.push_str(note);
    }
}

/// 媒体侧 context 的执行配置：无工具、JSON 报告、关闭 web_search。
fn media_overrides(opts: &RunOpts, model_alias: String, work_dir: PathBuf) -> TaskOverrides {
    TaskOverrides {
        model: Some(model_alias),
        loop_model: Some(LoopModel::FunctionCall),
        tools: Some(false),
        max_tokens: Some(opts.max_completion_tokens),
        runs_dir: Some(work_dir),
        json: true,
        json_schema: Some(report_schema()),
        disable_capabilities: vec!["web_search".to_string()],
        ..Default::default()
    }
}

fn build_input(
    opts: &RunOpts,
    media_content: Vec<AiContent>,
    parent_history: Vec<AiMessage>,
) -> Vec<AiMessage> {
    let mut input = Vec::with_capacity(parent_history.len() + 2);
    input.push(AiMessage::text(AiRole::System, SYSTEM_PROMPT));
    input.extend(parent_history);
    let mut user_content = media_content;
    user_content.push(AiContent::text(format!("Goal: {}", opts.goal)));
    input.push(AiMessage::new(AiRole::User, user_content));
    input
}

fn build_outcome_result(
    outcome: RunOutcome,
    mime: &str,
    work_dir: &PathBuf,
    run_id: &str,
    goal: &str,
    media_id: &str,
) -> (AgentToolResult, i32) {
    let record = outcome.record().clone();
    let usage = record.usage.total();
    let latency_ms = record.updated_at_ms.saturating_sub(record.created_at_ms);
    match outcome {
        RunOutcome::Completed(_) => {
            let raw = record
                .result
                .as_ref()
                .map(|r| r.raw.clone())
                .unwrap_or_default();
            let output = match serde_json::from_str::<Value>(raw.trim()) {
                Ok(content) => ContextOutput::Json { content },
                Err(_) => ContextOutput::Text { content: raw },
            };
            match parse_report_output(&output) {
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
                            hints: Vec::new(),
                            dedup: None,
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
                            images: Vec::new(),
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
                            hints: Vec::new(),
                            dedup: None,
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
                                "latency_ms": latency_ms,
                            }),
                            cmd_args: None,
                            return_code: None,
                            partial_output: None,
                            output: None,
                            images: Vec::new(),
                        },
                        CLI_EXIT_ERROR,
                    )
                }
            }
        }
        RunOutcome::Paused(_) | RunOutcome::Interrupted(_) => {
            let reason = record
                .last_error
                .as_ref()
                .map(|e| e.message.clone())
                .or_else(|| record.interrupt_reason.clone())
                .unwrap_or_else(|| record.status.label().to_string());
            (
                AgentToolResult {
                    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                    hints: Vec::new(),
                    dedup: None,
                    tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                    cmd_name: None,
                    status: AgentToolStatus::Error,
                    task_id: None,
                    pending_reason: None,
                    check_after: None,
                    estimated_wait: None,
                    title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => {}", record.status.as_str()),
                    summary: format!(
                        "sub run {run_id} {}: {reason}. It did not finish; take it over with `{}` or start again.",
                        record.status.label(),
                        record.resume_command()
                    ),
                    details: json!({
                        "outcome": record.status.as_str(),
                        "reason": reason,
                        "usage": usage,
                        "work_dir": work_dir.display().to_string(),
                        "run_id": run_id,
                        "resume": record.resume_command(),
                    }),
                    cmd_args: None,
                    return_code: None,
                    partial_output: None,
                    output: None,
                    images: Vec::new(),
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
            log::error!(
                "llm_understand_media: llm outcome {}: {}; work_dir={} run_id={} goal={}",
                record.status.as_str(),
                reason,
                work_dir.display(),
                run_id,
                goal
            );
            (
                AgentToolResult {
                    agent_tool_protocol: AGENT_TOOL_PROTOCOL_VERSION.to_string(),
                    hints: Vec::new(),
                    dedup: None,
                    tool: Some(TOOL_LLM_UNDERSTAND_MEDIA.to_string()),
                    cmd_name: None,
                    status: AgentToolStatus::Error,
                    task_id: None,
                    pending_reason: None,
                    check_after: None,
                    estimated_wait: None,
                    title: format!("{TOOL_LLM_UNDERSTAND_MEDIA} => {}", record.status.as_str()),
                    summary: format!("{}: {reason}", record.status.label()),
                    details: json!({
                        "outcome": record.status.as_str(),
                        "error": reason,
                        "error_detail": record.last_error,
                        "mime": mime,
                        "work_dir": work_dir.display().to_string(),
                        "run_id": run_id,
                        "usage": usage,
                    }),
                    cmd_args: None,
                    return_code: None,
                    partial_output: None,
                    output: None,
                    images: Vec::new(),
                },
                CLI_EXIT_ERROR,
            )
        }
    }
}

#[derive(Clone, Debug)]
struct MediaArg {
    source: MediaSource,
    mime_hint: Option<String>,
}

/// Where `llm_understand_media` reads the media from.
///
/// `ResourceRef` — the wire type — has no local-file variant, but an agent that
/// has just produced a frame or a clip with `shell` naturally holds a
/// filesystem path. Accepting that path here keeps a multi-step media task
/// inside one turn; the alternative is that the model wraps the path in a
/// `named_object` (the only shape it is offered), the path fails to decode as
/// an object id, and the turn is spent on a cryptic error.
#[derive(Clone, Debug)]
enum MediaSource {
    Resource(ResourceRef),
    LocalFile(PathBuf),
    /// A file of the Zone's DFS (`cyfs:///<path>`), as decoded path segments;
    /// `zone` is the host a `cyfs://<zone>/<path>` names.
    ZoneFile {
        zone: Option<String>,
        segments: Vec<String>,
    },
}

impl MediaSource {
    /// Stable label for logs and the tool result.
    fn kind(&self) -> &'static str {
        match self {
            MediaSource::Resource(source) => resource_source_kind(source),
            MediaSource::LocalFile(_) => "local_file",
            MediaSource::ZoneFile { .. } => "zone_file",
        }
    }

    /// Identifier safe to echo back to the model.
    fn display_id(&self) -> String {
        match self {
            MediaSource::Resource(source) => masked_resource_id(source),
            MediaSource::LocalFile(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_else(|| "<local file>".to_string()),
            MediaSource::ZoneFile { segments, .. } => segments
                .last()
                .cloned()
                .unwrap_or_else(|| "<zone file>".to_string()),
        }
    }
}

/// Where the media's bytes are.
#[derive(Debug)]
enum MediaPayload {
    /// A FileObject this identity reads through the Named Store.
    Object(ObjId),
    /// A local file, or a download.
    File(PathBuf),
    /// Inline media, a chunk, or an attachment only msg-center reads for this identity.
    Bytes(Vec<u8>),
}

/// A downloaded file, removed with the media.
#[derive(Debug)]
struct Download(PathBuf);

impl Drop for Download {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// A media input and what a light probe says about it. Nothing is uploaded or
/// read whole until the chosen path needs it.
#[derive(Debug)]
struct ResolvedMedia {
    payload: MediaPayload,
    mime: String,
    size: u64,
    name: Option<String>,
    head: Vec<u8>,
    _download: Option<Download>,
}

impl ResolvedMedia {
    fn object_id(&self) -> Option<String> {
        match &self.payload {
            MediaPayload::Object(id) => Some(id.to_string()),
            _ => None,
        }
    }

    /// Identifier for logs: never a payload.
    fn label(&self) -> String {
        self.object_id()
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| "<inline media>".to_string())
    }

    async fn full_bytes(&self) -> Result<Vec<u8>, String> {
        match &self.payload {
            MediaPayload::Object(id) => read_object_bytes(&id.to_string()).await,
            MediaPayload::File(path) => tokio::fs::read(path)
                .await
                .map_err(|err| format!("read media `{}` failed: {err}", path.display())),
            MediaPayload::Bytes(bytes) => Ok(bytes.clone()),
        }
    }

    /// The object a pipeline run takes as input: media without one is stored now.
    async fn frozen_id(&self) -> Result<ObjId, String> {
        let name = self.name.as_deref().unwrap_or("media");
        let stored = match &self.payload {
            MediaPayload::Object(id) => return Ok(id.clone()),
            MediaPayload::File(path) => {
                let file = tokio::fs::File::open(path)
                    .await
                    .map_err(|err| format!("open media `{}` failed: {err}", path.display()))?;
                freeze_reader(name, &self.mime, self.size, file).await
            }
            MediaPayload::Bytes(bytes) => {
                freeze_reader(name, &self.mime, self.size, bytes.as_slice()).await
            }
        };
        stored.map_err(|err| format!("store media for conversion failed: {err}"))
    }

    /// How the model stage refers to the media: AICC reads an object itself;
    /// anything else goes inline while it may, and is stored when it may not.
    async fn model_source(&self) -> Result<ResourceRef, String> {
        match &self.payload {
            MediaPayload::Object(id) => Ok(ResourceRef::NamedObject { obj_id: id.clone() }),
            _ if self.size <= MAX_MODEL_INLINE_BYTES => Ok(ResourceRef::Base64 {
                mime: self.mime.clone(),
                data_base64: general_purpose::STANDARD.encode(self.full_bytes().await?),
            }),
            _ => Ok(ResourceRef::NamedObject {
                obj_id: self.frozen_id().await?,
            }),
        }
    }
}

/// `details.media_decision`.
#[derive(Clone, Debug)]
struct MediaDecision {
    kind: &'static str,
    reason: String,
    family: Option<&'static str>,
    obj_id: Option<String>,
    mime: String,
}

impl MediaDecision {
    fn direct(media: &ResolvedMedia, reason: impl Into<String>) -> Self {
        Self {
            kind: "direct",
            reason: reason.into(),
            family: None,
            obj_id: media.object_id(),
            mime: media.mime.clone(),
        }
    }

    fn converted(media: &ResolvedMedia, family: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind: "converted",
            reason: reason.into(),
            family: Some(family),
            obj_id: media.object_id(),
            mime: media.mime.clone(),
        }
    }

    fn with_kind(mut self, kind: &'static str) -> Self {
        self.kind = kind;
        self
    }

    fn to_value(&self) -> Value {
        json!({
            "kind": self.kind,
            "reason": self.reason,
            "family": self.family,
            "obj_id": self.obj_id,
            "mime": self.mime,
        })
    }
}

/// Parse the `media` argument.
///
/// Accepted shapes, in order of preference:
/// * `{"kind":"named_object","obj_id":"cyfile:…"}` — a stored object;
/// * a system URL (`cyfs:///<path>`, `cyfs://<objid>/`, `file://…`), bare or
///   in any of the fields below — the ways the system names a file;
/// * `{"url":"https://…"}` / a bare `http(s):` / `data:` string;
/// * a bare path or `{"kind":"local_file","path":"…"}` — a file on this host,
///   which covers artifacts an earlier `shell` produced, or the Zone path
///   Files shows (`/home/<owner>/…`);
/// * `{"kind":"named_object","obj_id":"<path>"}` — a model that reached for
///   the only shape it knows; coerced to a local file when the path exists,
///   rejected with an actionable message when it does not.
fn parse_media_arg(value: &Value) -> Result<MediaArg, String> {
    let mime_hint = value
        .get("mime_hint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            value
                .get("mime")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        });

    let source = match value {
        // Bare string: URL, typed object id, or local path.
        Value::String(raw) => parse_media_locator(raw)?,
        Value::Object(map) => {
            let kind = map
                .get("kind")
                .and_then(Value::as_str)
                .map(|kind| kind.trim().to_ascii_lowercase());

            if let Some(url) = ["url", "uri"].iter().find_map(|key| {
                map.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            }) {
                // Documented shorthand — `{"url": "…"}`, with or without the
                // `kind` tag the wire type insists on.
                parse_media_locator(url)?
            } else if let Some(url) = ["path", "file", "local_path", "obj_id"].iter().find_map(|key| {
                map.get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| value.contains("://"))
            }) {
                // A system URL in whichever field the model chose for it.
                parse_media_locator(url)?
            } else if let Some(path) = local_path_from_object(map, kind.as_deref())? {
                MediaSource::LocalFile(path)
            } else {
                MediaSource::Resource(parse_resource_ref(value)?)
            }
        }
        other => {
            return Err(format!(
                "media must be an object or a string, got {}",
                json_type_name(other)
            ))
        }
    };

    Ok(MediaArg { source, mime_hint })
}

/// Decode a media object as a `ResourceRef`.
///
/// `mime_hint` / `mime` are documented as media-level fields, but the wire type
/// rejects unknown fields, so the documented shape fails for variants that do
/// not declare one. Retry with the hints removed — the value is already kept in
/// `MediaArg::mime_hint` — and only then report the failure.
fn parse_resource_ref(value: &Value) -> Result<ResourceRef, String> {
    let err = match serde_json::from_value::<ResourceRef>(value.clone()) {
        Ok(source) => return Ok(source),
        Err(err) => err,
    };

    if let Some(map) = value.as_object() {
        let mut stripped = map.clone();
        stripped.remove("mime_hint");
        stripped.remove("mime");
        if stripped.len() != map.len() {
            if let Ok(source) = serde_json::from_value::<ResourceRef>(Value::Object(stripped)) {
                return Ok(source);
            }
        }
    }

    Err(format!(
        "invalid media argument: {err}. Pass one of \
         {{\"kind\":\"named_object\",\"obj_id\":\"cyfile:…\"}}, \
         {{\"kind\":\"local_file\",\"path\":\"/abs/path\"}}, a file path, or a url."
    ))
}

/// Resolve a bare locator string to a media source.
fn parse_media_locator(raw: &str) -> Result<MediaSource, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("media string is empty".to_string());
    }
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(MediaSource::Resource(ResourceRef::Url {
            url: trimmed.to_string(),
            mime_hint: None,
        }));
    }
    if let Some(rest) = trimmed.strip_prefix("data:") {
        return parse_data_url(rest).map(MediaSource::Resource);
    }
    if let Some(source) = zone_file::parse_system_url(trimmed)? {
        return Ok(source);
    }
    if looks_like_object_id(trimmed) {
        // Let the wire type do the decoding so the error message stays
        // authoritative about what a valid object id is.
        return serde_json::from_value::<ResourceRef>(serde_json::json!({
            "kind": "named_object",
            "obj_id": trimmed,
        }))
        .map(MediaSource::Resource)
        .map_err(|err| format!("invalid object id `{trimmed}`: {err}"));
    }
    let path = PathBuf::from(trimmed);
    // A path is resolved later, where the session workspace and the Zone's
    // files are tried and every miss is reported.
    if path.exists() || looks_like_path(trimmed) {
        return Ok(MediaSource::LocalFile(path));
    }
    Err(format!(
        "media `{trimmed}` is neither a URL (`cyfs:///<path>`, https://…), a typed object id \
         (`cyfile:…` / `chunk:…`), nor a file path"
    ))
}

/// Detect `{"kind":"local_file","path":…}` and friends, plus the very common
/// mistake of passing a filesystem path as `obj_id`.
fn local_path_from_object(
    map: &serde_json::Map<String, Value>,
    kind: Option<&str>,
) -> Result<Option<PathBuf>, String> {
    let explicit_path = map
        .get("path")
        .or_else(|| map.get("file"))
        .or_else(|| map.get("local_path"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if let Some(path) = explicit_path {
        return Ok(Some(PathBuf::from(path)));
    }

    if matches!(kind, Some("local_file") | Some("file") | Some("path")) {
        return Err(format!(
            "media kind `{}` requires a `path`",
            kind.unwrap_or("local_file")
        ));
    }

    if matches!(kind, Some("named_object") | None) {
        if let Some(obj_id) = map.get("obj_id").and_then(Value::as_str) {
            let obj_id = obj_id.trim();
            // A path handed over as an object id: the tool can serve it, so
            // serve it rather than failing on base32 decoding.
            if !looks_like_object_id(obj_id) && looks_like_path(obj_id) {
                // A path handed over as an object id. It cannot decode as an
                // id, but it is exactly the shape a model produces after
                // `shell` wrote an artifact — treat it as a local file and
                // let resolution report a precise error if it is missing.
                log::warn!(
                    "llm_understand_media: media.obj_id `{obj_id}` is not a typed object id; \
                     treating it as a local file path"
                );
                return Ok(Some(PathBuf::from(obj_id)));
            }
        }
    }

    Ok(None)
}

fn looks_like_object_id(value: &str) -> bool {
    // Object ids in this system are typed (`cyfile:<…>`, a chunk id such as
    // `mix256:<…>` / `sha256:<…>`): whatever ndn-lib parses as an `ObjId`,
    // with no path separator or whitespace.
    if value.contains(['/', '\\']) || value.chars().any(char::is_whitespace) {
        return false;
    }
    matches!(
        value.split_once(':').map(|(kind, _)| kind),
        Some("cyfile") | Some("chunk") | Some("objid") | Some("obj")
    ) || ndn_lib::ObjId::new(value).is_ok()
}

fn looks_like_path(value: &str) -> bool {
    value.contains('/') || value.contains('\\') || value.contains('.') || value.starts_with('~')
}

fn parse_data_url(rest: &str) -> Result<ResourceRef, String> {
    let (meta, data) = rest
        .split_once(',')
        .ok_or_else(|| "malformed data: URL".to_string())?;
    if !meta.ends_with(";base64") {
        return Err("only base64 `data:` URLs are supported for media".to_string());
    }
    let mime = normalize_mime(meta.trim_end_matches(";base64"))
        .ok_or_else(|| "data: URL has no mime type".to_string())?;
    if data.trim().is_empty() {
        return Err("data: URL carries no payload".to_string());
    }
    Ok(ResourceRef::Base64 {
        mime,
        data_base64: data.trim().to_string(),
    })
}

fn json_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Find the file a caller meant by a local path.
///
/// `shell` runs inside the session workspace — `<agent_root>/sessions/<id>`,
/// which is also where session media inputs live — while the agent process
/// itself starts in the install root. A model that has just written
/// `frame008.png` therefore names it relative to a directory that is *not* the
/// process CWD. Try the process-relative path first, then the session
/// workspace, and on failure report every path that was tried.
fn resolve_local_file_path(raw: &Path, session_id: Option<&str>) -> Result<PathBuf, String> {
    let candidates = local_path_candidates(raw, session_id);
    for (index, candidate) in candidates.iter().enumerate() {
        if candidate.is_file() {
            if index > 0 {
                log::info!(
                    "llm_understand_media: resolved local media `{}` to `{}`",
                    raw.display(),
                    candidate.display()
                );
            }
            return Ok(candidate.clone());
        }
    }
    Err(format!(
        "local media `{}` does not exist (tried: {}); pass an absolute path",
        raw.display(),
        candidates
            .iter()
            .map(|candidate| candidate.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn local_path_candidates(raw: &Path, session_id: Option<&str>) -> Vec<PathBuf> {
    let mut agent_roots = Vec::new();
    if let Some(agent_root) = env_path(crate::runtime_context::OPENDAN_AGENT_ROOT_ENV) {
        agent_roots.push(agent_root);
    }
    // The agent process does not always carry `OPENDAN_AGENT_ROOT`; recover the
    // agent root from the deployment layout instead.
    if let Some(data_dir) = env_path("BUCKYOS_DATA_DIR") {
        agent_roots.push(data_dir);
    }
    local_path_candidates_under(raw, session_id, &agent_roots)
}

/// Pure half of [`local_path_candidates`]: each entry of `agent_roots`
/// contributes `<root>/sessions/<session_id>/<raw>`.
fn local_path_candidates_under(
    raw: &Path,
    session_id: Option<&str>,
    agent_roots: &[PathBuf],
) -> Vec<PathBuf> {
    let mut candidates = vec![raw.to_path_buf()];
    if raw.is_absolute() {
        return candidates;
    }
    let Some(session_id) = session_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return candidates;
    };
    for root in agent_roots {
        candidates.push(root.join("sessions").join(session_id).join(raw));
    }
    candidates
}

fn env_path(key: &str) -> Option<PathBuf> {
    std::env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// The media and, when it is a Zone file, which one was read.
async fn resolve_media(
    media: &MediaArg,
    session_id: Option<&str>,
) -> Result<(ResolvedMedia, Option<zone_file::ZoneFile>), String> {
    let hint = media.mime_hint.as_deref().and_then(normalize_mime);
    match &media.source {
        MediaSource::LocalFile(raw) => {
            let (path, found) = match resolve_local_file_path(raw, session_id) {
                Ok(path) => (path, None),
                // Not a file here: the path Files shows for a Zone file.
                Err(local_err) => {
                    let zone = zone_file::ZoneView::from_runtime();
                    let Some(segments) = zone.bare_path_segments(raw) else {
                        return Err(local_err);
                    };
                    let found = zone.locate(None, &segments).map_err(|zone_err| {
                        log::debug!("llm_understand_media: {local_err}");
                        format!("`{}` is not a file here; as a Zone path: {zone_err}", raw.display())
                    })?;
                    (found.path.clone(), Some(found))
                }
            };
            Ok((local_media(path, hint).await?, found))
        }
        MediaSource::ZoneFile { zone, segments } => {
            let found = zone_file::ZoneView::from_runtime().locate(zone.as_deref(), segments)?;
            Ok((local_media(found.path.clone(), hint).await?, Some(found)))
        }
        MediaSource::Resource(source) => Ok((resolve_resource(source, hint).await?, None)),
    }
}

/// A file of this runtime, probed.
async fn local_media(path: PathBuf, hint: Option<String>) -> Result<ResolvedMedia, String> {
    let (size, head) = probe_file(&path).await?;
    let mime = hint
        .or_else(|| mime_from_path_extension(&path))
        .or_else(|| sniff_mime(&head))
        .ok_or_else(|| {
            format!(
                "cannot determine MIME for local media `{}`; pass mime_hint",
                path.display()
            )
        })?;
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "media".to_string());
    Ok(ResolvedMedia {
        payload: MediaPayload::File(path),
        mime,
        size,
        name: Some(name),
        head,
        _download: None,
    })
}

async fn resolve_resource(
    source: &ResourceRef,
    hint: Option<String>,
) -> Result<ResolvedMedia, String> {
    let inline = |mime: String, name: Option<String>, bytes: Vec<u8>| {
        let head = bytes[..bytes.len().min(PROBE_HEAD_BYTES)].to_vec();
        ResolvedMedia {
            size: bytes.len() as u64,
            payload: MediaPayload::Bytes(bytes),
            mime,
            name,
            head,
            _download: None,
        }
    };
    match source {
        ResourceRef::Base64 { mime, data_base64 } => {
            let mime =
                normalize_mime(mime).ok_or_else(|| "base64 media has empty mime".to_string())?;
            if data_base64.trim().is_empty() {
                return Err("base64 media has empty data_base64".to_string());
            }
            let bytes = general_purpose::STANDARD
                .decode(data_base64.trim())
                .map_err(|err| format!("decode base64 media failed: {err}"))?;
            let name = format!("inline.{}", mime.rsplit('/').next().unwrap_or("bin"));
            Ok(inline(mime, Some(name), bytes))
        }
        ResourceRef::Url { url, mime_hint } => {
            let download = download_url(url).await?;
            let (size, head) = probe_file(&download.0).await?;
            let mime = mime_hint
                .as_deref()
                .and_then(normalize_mime)
                .or_else(|| hint.clone())
                .or_else(|| sniff_mime(&head))
                .or_else(|| url_file_name(url).and_then(|n| mime_from_path_extension(Path::new(&n))))
                .ok_or_else(|| "cannot determine MIME for URL media; pass mime_hint".to_string())?;
            Ok(ResolvedMedia {
                payload: MediaPayload::File(download.0.clone()),
                mime,
                size,
                name: Some(url_file_name(url).unwrap_or_else(|| "download".to_string())),
                head,
                _download: Some(download),
            })
        }
        ResourceRef::NamedObject { obj_id } => {
            let head = if obj_id.is_file_object() {
                object_head(obj_id, PROBE_HEAD_BYTES).await?
            } else {
                None
            };
            let Some(head) = head else {
                let content = crate::read_tool::read_named_object(obj_id).await?;
                if content.bytes.is_empty() {
                    return Err(format!("named_object {obj_id} has empty content"));
                }
                let declared = content.mime.as_deref().and_then(normalize_mime);
                let mime = named_media_mime(&content.bytes, None, declared, hint.clone())
                    .ok_or_else(|| format!("cannot determine MIME for named_object {obj_id}"))?;
                return Ok(inline(mime, None, content.bytes));
            };
            if head.size == 0 {
                return Err(format!("named_object {obj_id} has empty content"));
            }
            let mime = named_media_mime(
                &head.head,
                head.name.as_deref(),
                head.mime.as_deref().and_then(normalize_mime),
                hint.clone(),
            )
            .ok_or_else(|| format!("cannot determine MIME for named_object {obj_id}"))?;
            Ok(ResolvedMedia {
                payload: MediaPayload::Object(obj_id.clone()),
                mime,
                size: head.size,
                name: head.name,
                head: head.head,
                _download: None,
            })
        }
    }
}

/// What the content says, unless it only says "zip" and the name says more
/// (office documents); then the name, the declared type and the caller's hint.
fn named_media_mime(
    head: &[u8],
    name: Option<&str>,
    declared: Option<String>,
    hint: Option<String>,
) -> Option<String> {
    let by_name = name.and_then(|n| mime_from_path_extension(Path::new(n)));
    sniff_mime(head)
        .filter(|m| !is_archive_mime(m) || by_name.is_none())
        .or(by_name)
        .or_else(|| declared.clone().filter(|m| m != "application/octet-stream"))
        .or(hint)
        .or(declared)
}

/// Size and first bytes of a local file, within what a pipeline takes as input.
async fn probe_file(path: &Path) -> Result<(u64, Vec<u8>), String> {
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(path)
        .await
        .map_err(|err| format!("read local media `{}` failed: {err}", path.display()))?;
    let size = file
        .metadata()
        .await
        .map_err(|err| format!("read local media `{}` failed: {err}", path.display()))?
        .len();
    if size == 0 {
        return Err(format!("local media `{}` is empty", path.display()));
    }
    if size > MAX_INPUT_BYTES {
        return Err(format!(
            "local media `{}` is {size} bytes, above the {MAX_INPUT_BYTES} byte limit",
            path.display()
        ));
    }
    let mut head = vec![0u8; PROBE_HEAD_BYTES.min(size as usize)];
    file.read_exact(&mut head)
        .await
        .map_err(|err| format!("read local media `{}` failed: {err}", path.display()))?;
    Ok((size, head))
}

fn sniff_mime(bytes: &[u8]) -> Option<String> {
    sniff_archive_mime(bytes)
        .or_else(|| sniff_image_mime(bytes))
        .or_else(|| sniff_video_mime(bytes))
        .or_else(|| sniff_document_mime(bytes))
        .map(str::to_string)
}

fn url_file_name(url: &str) -> Option<String> {
    let path = url.split(['?', '#']).next()?;
    let name = path.rsplit('/').next()?.trim();
    (!name.is_empty() && name.contains('.')).then(|| name.to_string())
}

/// Streams `url` to a temporary file, within what a pipeline takes as input.
async fn download_url(url: &str) -> Result<Download, String> {
    use tokio::io::AsyncWriteExt;
    let mut response = reqwest::Client::new()
        .get(url)
        .send()
        .await
        .map_err(|err| format!("download media failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("download media failed: {err}"))?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let download = Download(std::env::temp_dir().join(format!(
        "llm_understand_media-download-{}-{nanos}",
        std::process::id()
    )));
    let mut file = tokio::fs::File::create(&download.0)
        .await
        .map_err(|err| format!("create {} failed: {err}", download.0.display()))?;
    let mut total = 0u64;
    while let Some(piece) = response
        .chunk()
        .await
        .map_err(|err| format!("read downloaded media failed: {err}"))?
    {
        total += piece.len() as u64;
        if total > MAX_INPUT_BYTES {
            return Err(format!("downloaded media exceeds the {MAX_INPUT_BYTES} byte limit"));
        }
        file.write_all(&piece)
            .await
            .map_err(|err| format!("write {} failed: {err}", download.0.display()))?;
    }
    file.flush()
        .await
        .map_err(|err| format!("write {} failed: {err}", download.0.display()))?;
    if total == 0 {
        return Err("downloaded media is empty".to_string());
    }
    Ok(download)
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

fn normalize_mime(value: &str) -> Option<String> {
    value
        .split(';')
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase())
}

/// MIME guess from a file extension, checked before content sniffing so local
/// artifacts that sniffing does not recognize (audio, for instance) still route
/// to the right model.
fn mime_from_path_extension(path: &Path) -> Option<String> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let mime = match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "svg" => "image/svg+xml",
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        "wav" => "audio/wav",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "odp" => "application/vnd.oasis.opendocument.presentation",
        "doc" => "application/msword",
        "ppt" => "application/vnd.ms-powerpoint",
        "rtf" => "application/rtf",
        "json" => "application/json",
        "txt" | "md" | "log" | "csv" | "tsv" => "text/plain",
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        "rar" => "application/vnd.rar",
        "bz2" => "application/x-bzip2",
        "xz" => "application/x-xz",
        "zst" => "application/zstd",
        _ => return None,
    };
    Some(mime.to_string())
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

fn is_audio_mime(mime: &str) -> bool {
    mime.starts_with("audio/")
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
    } else if is_audio_mime(mime) {
        configured_model("LLM_UNDERSTAND_MEDIA_AUDIO_MODEL")
    } else if is_archive_mime(mime) {
        return None;
    } else {
        None
    };
    specific
        .or_else(|| configured_model("LLM_UNDERSTAND_MEDIA_MODEL"))
        .or_else(|| Some(default_model_alias(mime).to_string()))
}

fn default_model_alias(mime: &str) -> &'static str {
    if is_audio_mime(mime) {
        DEFAULT_AUDIO_ANALYSIS_MODEL_ALIAS
    } else {
        DEFAULT_MODEL_ALIAS
    }
}

struct PreparedMedia {
    content: Vec<AiContent>,
    decision: MediaDecision,
    trace: Option<PipelineTrace>,
    coverage: Option<Coverage>,
    preprocess: Option<Value>,
    caller_note: Option<String>,
    /// Sent directly; a pipeline can still represent it if the model refuses.
    convert_if_refused: bool,
    /// What the model is given, for the progress line (`16 page images and 56 pages of text`).
    reading: Option<String>,
    /// The catalog to unpack while the model works, and where it goes.
    catalog_job: Option<CatalogJob>,
    catalog_dir: Option<PathBuf>,
    retrieval: Option<Value>,
    media_source: Option<Value>,
}

struct CatalogJob {
    root: PathBuf,
    file: CatalogFile,
    texts: BTreeMap<u64, PageText>,
}

impl PreparedMedia {
    fn new(content: Vec<AiContent>, decision: MediaDecision) -> Self {
        Self {
            content,
            decision,
            trace: None,
            coverage: None,
            preprocess: None,
            caller_note: None,
            convert_if_refused: false,
            reading: None,
            catalog_job: None,
            catalog_dir: None,
            retrieval: None,
            media_source: None,
        }
    }
}

struct PrepareError {
    message: String,
    decision: MediaDecision,
    trace: Option<PipelineTrace>,
}

impl PrepareError {
    fn new(decision: MediaDecision, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            decision,
            trace: None,
        }
    }

    fn pipeline(decision: MediaDecision, error: &ThunkExecutionError, trace: PipelineTrace) -> Self {
        let deployment = matches!(
            error.code.as_str(),
            "tools_missing" | "environment_changed" | "function_not_found" | "interrupted"
        );
        let message = format!(
            "media pipeline {} (run {}) failed{}: {}: {}{}",
            trace.workflow_id,
            trace.run_id,
            if deployment { " (deployment / kernel diagnosis)" } else { "" },
            error.code,
            error.message,
            if error.details.is_null() { String::new() } else { format!(" details={}", error.details) }
        );
        Self {
            message,
            decision: decision.with_kind("failed"),
            trace: Some(trace),
        }
    }
}

fn text_char_budget() -> usize {
    std::env::var("LLM_UNDERSTAND_MEDIA_TEXT_CHARS")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_TEXT_CHAR_BUDGET)
}

fn max_images() -> usize {
    std::env::var("LLM_UNDERSTAND_MEDIA_MAX_IMAGES")
        .ok()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_IMAGES)
}

const PASSTHROUGH_IMAGE_MIMES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Picks between direct pass-through and the system pipeline: direct when the
/// model takes the format within budget, otherwise a `media.<family>` run whose
/// products become the model input.
async fn prepare_media_content(
    media: &ResolvedMedia,
    opts: &RunOpts,
    model_alias: &str,
) -> Result<PreparedMedia, PrepareError> {
    let session_id = opts.session_id.as_deref();
    let failed = |decision: &MediaDecision, message: String| {
        PrepareError::new(decision.clone().with_kind("failed"), message)
    };
    if is_archive_mime(&media.mime) {
        return Err(PrepareError::new(
            MediaDecision::direct(media, "archive"),
            format!("archive media mime `{}` must be extracted first", media.mime),
        ));
    }
    if is_audio_mime(&media.mime) {
        let decision = MediaDecision::direct(media, "audio goes through AICC speech recognition");
        let source = media.model_source().await.map_err(|e| failed(&decision, e))?;
        let transcript = transcribe_audio(source, session_id)
            .await
            .map_err(|e| failed(&decision, e))?;
        let mut prepared = PreparedMedia::new(audio_transcript_content(&transcript), decision);
        prepared.coverage = Some(Coverage {
            family: "audio".to_string(),
            audio: Some("transcribed".to_string()),
            complete: true,
            ..Default::default()
        });
        return Ok(prepared);
    }
    if is_image_mime(&media.mime) {
        return prepare_image(media, opts).await;
    }
    if is_video_mime(&media.mime) {
        return prepare_video(media, opts).await;
    }
    let office_format = document_format_from_mime(&media.mime)
        .or_else(|| media.name.as_deref().and_then(document_format_from_name));
    if let Some(format) = office_format {
        return prepare_document(
            media,
            opts,
            Some(format),
            format!("{format} is converted to a catalog of PDF pages and text"),
        )
        .await;
    }
    if media.mime == "application/pdf" {
        return prepare_pdf(media, opts, model_alias).await;
    }
    if media.mime.starts_with("text/") || media.mime == "application/json" {
        let decision = MediaDecision::direct(media, "text content is inlined");
        if media.size > MAX_INLINE_TEXT_BYTES {
            return Err(PrepareError::new(
                decision,
                format!("text media of {} bytes exceeds the {} byte inline limit; read it with `read` instead", media.size, MAX_INLINE_TEXT_BYTES),
            ));
        }
        let bytes = media.full_bytes().await.map_err(|e| failed(&decision, e))?;
        return Ok(PreparedMedia::new(
            vec![AiContent::text(format!(
                "Text media input ({}):\n{}",
                media.mime,
                String::from_utf8_lossy(&bytes)
            ))],
            decision,
        ));
    }
    let protocol = resolve_protocol(model_alias, session_id).await;
    if protocol.as_deref().map(protocol_accepts_documents).unwrap_or(false)
        && media.size <= DocumentLimits::from_env().direct_max_bytes
    {
        let decision = MediaDecision::direct(media, "protocol accepts documents");
        return direct_document(media, decision).await;
    }
    Err(PrepareError::new(
        MediaDecision::direct(media, "unsupported"),
        format!(
            "media mime `{}` is not accepted by model protocol `{}` and no system pipeline converts it",
            media.mime,
            protocol.unwrap_or_else(|| "unknown".to_string())
        ),
    ))
}

async fn direct_document(
    media: &ResolvedMedia,
    decision: MediaDecision,
) -> Result<PreparedMedia, PrepareError> {
    let source = media
        .model_source()
        .await
        .map_err(|e| PrepareError::new(decision.clone().with_kind("failed"), e))?;
    Ok(PreparedMedia::new(
        vec![AiContent::Document {
            source,
            title: media.name.clone().or_else(|| Some("media input".to_string())),
        }],
        decision,
    ))
}

/// A PDF goes direct only when the protocol takes documents, the file is small
/// and it is not long; everything else becomes a catalog.
async fn prepare_pdf(
    media: &ResolvedMedia,
    opts: &RunOpts,
    model_alias: &str,
) -> Result<PreparedMedia, PrepareError> {
    let limits = DocumentLimits::from_env();
    let protocol = resolve_protocol(model_alias, opts.session_id.as_deref())
        .await
        .unwrap_or_else(|| "unknown".to_string());
    if !protocol_accepts_documents(&protocol) {
        let reason = format!("protocol `{protocol}` does not take documents");
        return prepare_document(media, opts, None, reason).await;
    }
    if media.size > limits.direct_max_bytes {
        let reason = format!("{} bytes is above the direct limit", media.size);
        return prepare_document(media, opts, None, reason).await;
    }
    let decision = MediaDecision::converted(media, FAMILY_PDF, "counting the pages of the PDF");
    let obj_id = frozen_input(media, &decision).await?;
    let input = limits.pipeline_input(&obj_id.to_string(), None, &[1], 1, pipeline::OCR_LANG_AUTO);
    let (output, trace) = drive(FAMILY_PDF, &obj_id, input, opts, &decision).await?;
    let pages = output
        .get("build_catalog")
        .and_then(|c| c.get("summary"))
        .and_then(|s| s.get("total"))
        .and_then(Value::as_u64)
        .ok_or_else(|| PrepareError::new(decision.clone().with_kind("failed"), "the PDF has no page count"))?;
    if pages > u64::from(limits.long_doc_pages) {
        let reason = format!(
            "{pages} pages is a long document (more than {})",
            limits.long_doc_pages
        );
        return prepare_document(media, opts, None, reason).await;
    }
    let reason = format!(
        "protocol `{protocol}` takes documents; {pages} pages, {} bytes",
        media.size
    );
    let mut prepared = direct_document(media, MediaDecision::direct(media, reason)).await?;
    prepared.trace = Some(trace);
    prepared.convert_if_refused = true;
    Ok(prepared)
}

/// The protocol adapter the chosen model routes to (`claude-messages`,
/// `openai-chat-completions`, ...); `None` when routing is unavailable.
async fn resolve_protocol(model_alias: &str, session_id: Option<&str>) -> Option<String> {
    let runtime = get_buckyos_api_runtime().ok()?;
    let client = runtime.get_aicc_client().await.ok()?;
    let mut request = RouteResolveRequest::new(ApiType::Llm, model_alias);
    request.session_id = session_id.map(str::to_string);
    client
        .route_resolve(request)
        .await
        .ok()
        .map(|route| route.protocol_adapter_id)
}

async fn frozen_input(media: &ResolvedMedia, decision: &MediaDecision) -> Result<ObjId, PrepareError> {
    media
        .frozen_id()
        .await
        .map_err(|e| PrepareError::new(decision.clone().with_kind("failed"), e))
}

async fn drive(
    family: &'static str,
    obj_id: &ObjId,
    input: Value,
    opts: &RunOpts,
    decision: &MediaDecision,
) -> Result<(BTreeMap<String, Value>, PipelineTrace), PrepareError> {
    drive_watched(family, obj_id, input, opts, decision, None).await
}

async fn drive_watched(
    family: &'static str,
    obj_id: &ObjId,
    input: Value,
    opts: &RunOpts,
    decision: &MediaDecision,
    watch: Option<WindowWatch>,
) -> Result<(BTreeMap<String, Value>, PipelineTrace), PrepareError> {
    let mut request = PipelineRequest::new(family, &obj_id.to_string(), input);
    request.session_id = opts.session_id.clone();
    request.watch = watch;
    match run_pipeline(request).await {
        Ok(PipelineOutcome::Completed { output, trace }) => Ok((output, trace)),
        Ok(PipelineOutcome::Failed { error, trace }) => {
            Err(PrepareError::pipeline(decision.clone(), &error, trace))
        }
        Err(message) => Err(PrepareError::new(decision.clone().with_kind("failed"), message)),
    }
}

fn named_object(id: &str) -> Result<ResourceRef, String> {
    Ok(ResourceRef::NamedObject {
        obj_id: ObjId::new(id).map_err(|e| format!("invalid artifact id `{id}`: {e}"))?,
    })
}

async fn prepare_image(media: &ResolvedMedia, opts: &RunOpts) -> Result<PreparedMedia, PrepareError> {
    let limits = ImageLimits::from_env();
    let dims = image_dimensions(&media.head);
    let passthrough = PASSTHROUGH_IMAGE_MIMES.contains(&media.mime.as_str());
    if let (true, Some((width, height))) = (passthrough, dims) {
        if limits.allows(width, height, media.size) {
            let decision = MediaDecision::direct(
                media,
                format!("{}x{} {} within the vision limits", width, height, media.mime),
            );
            let source = media
                .model_source()
                .await
                .map_err(|e| PrepareError::new(decision.clone().with_kind("failed"), e))?;
            let info = json!({"mime": media.mime, "width": width, "height": height, "bytes": media.size});
            let mut prepared = PreparedMedia::new(vec![AiContent::image(source)], decision);
            prepared.preprocess = Some(json!({"original": info, "sent": info, "actions": []}));
            return Ok(prepared);
        }
    }
    let reason = match dims {
        Some((w, h)) if passthrough => format!("{w}x{h} / {} bytes exceeds the vision limits", media.size),
        _ => format!("`{}` must be decoded and re-encoded for the vision model", media.mime),
    };
    let mut decision = MediaDecision::converted(media, FAMILY_IMAGE, reason);
    let obj_id = frozen_input(media, &decision).await?;
    decision.obj_id = Some(obj_id.to_string());
    let input = limits.pipeline_input(&obj_id.to_string());
    let (output, trace) = drive(FAMILY_IMAGE, &obj_id, input, opts, &decision).await?;
    let fit = output
        .get("fit_image")
        .cloned()
        .ok_or_else(|| PrepareError::new(decision.clone(), "media.image run has no fit_image output"))?;
    let model_obj = fit
        .get("model_media_obj_id")
        .and_then(Value::as_str)
        .ok_or_else(|| PrepareError::new(decision.clone(), "fit_image output has no model_media_obj_id"))?
        .to_string();
    let source = named_object(&model_obj).map_err(|e| PrepareError::new(decision.clone(), e))?;
    let downscaled = fit
        .get("actions")
        .and_then(Value::as_array)
        .map(|a| a.iter().any(|v| v == "downscaled"))
        .unwrap_or(false);
    let dims_of = |key: &str| {
        let v = fit.get(key);
        (
            v.and_then(|v| v.get("width")).and_then(Value::as_u64).unwrap_or(0),
            v.and_then(|v| v.get("height")).and_then(Value::as_u64).unwrap_or(0),
        )
    };
    let (ow, oh) = dims_of("original");
    let (sw, sh) = dims_of("sent");
    let mut content = Vec::with_capacity(2);
    let mut caller_note = None;
    if downscaled {
        content.push(AiContent::text(format!(
            "Image preprocessing note: this image was downscaled from {ow}x{oh} to {sw}x{sh} pixels before analysis, so very small text or fine detail may be lost and any pixel coordinates refer to the downscaled image."
        )));
        caller_note = Some(format!(
            "Note: analyzed a downscaled copy ({ow}x{oh} -> {sw}x{sh}); for fine detail, crop the region of interest and analyze the crop."
        ));
    }
    content.push(AiContent::image(source));
    let mut prepared = PreparedMedia::new(content, decision);
    prepared.trace = Some(trace);
    prepared.preprocess = Some(json!({
        "original": fit.get("original"),
        "sent": fit.get("sent"),
        "actions": fit.get("actions"),
        "model_media_obj_id": model_obj,
    }));
    prepared.caller_note = caller_note;
    Ok(prepared)
}

fn manifest_id_of(output: &BTreeMap<String, Value>) -> Result<String, String> {
    output
        .get("build_catalog")
        .and_then(|c| c.get("manifest_obj_id"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| "run output has no manifest_obj_id".to_string())
}

/// The whole catalog of a document: the first run names the page count, later
/// page windows are separate runs. A later window that fails leaves its pages
/// `unprocessed`; what was built is still delivered.
async fn build_document_catalog(
    family: &'static str,
    obj_id: &ObjId,
    format: Option<&'static str>,
    opts: &RunOpts,
    decision: &MediaDecision,
) -> Result<(buckyos_api::MediaManifest, Vec<String>, PipelineTrace, Option<String>), PrepareError> {
    let limits = DocumentLimits::from_env();
    let id = obj_id.to_string();
    let as_error = |e: String| PrepareError::new(decision.clone().with_kind("failed"), e);
    let input = limits.pipeline_input(&id, format, &[], limits.first_window_pages(), pipeline::OCR_LANG_AUTO);
    let watch = |label: String, first: bool| Some(WindowWatch { label, first, max_pages: limits.max_pages });
    let started = std::time::Instant::now();
    let (output, mut trace) =
        drive_watched(family, obj_id, input, opts, decision, watch("the first pages".to_string(), true)).await?;
    let first_id = manifest_id_of(&output).map_err(as_error)?;
    let mut manifest = read_manifest(&first_id).await.map_err(as_error)?;
    let total = manifest.range.total.unwrap_or(0);
    let first_last = total.min(u64::from(limits.first_window_pages()));
    progress(window_done_line(&format!("pages 1-{first_last}"), &trace, started.elapsed()));
    // Every window is read in the language the first one found.
    let ocr_lang = manifest
        .ocr
        .as_ref()
        .and_then(|o| o.lang.clone())
        .unwrap_or_else(|| pipeline::OCR_LANG_AUTO.to_string());
    let mut manifest_ids = vec![first_id];
    let mut incomplete = None;
    for window in limits.later_windows(total) {
        let input = limits.pipeline_input(&id, format, &window, CATALOG_WINDOW_PAGES, &ocr_lang);
        let label = format!("pages {}-{}", window.first().copied().unwrap_or(0), window.last().copied().unwrap_or(0));
        let started = std::time::Instant::now();
        let next = match drive_watched(family, obj_id, input, opts, decision, watch(label.clone(), false)).await {
            Ok((output, window_trace)) => {
                progress(window_done_line(&label, &window_trace, started.elapsed()));
                trace.more_runs.push(window_trace);
                match manifest_id_of(&output) {
                    Ok(next_id) => read_manifest(&next_id).await.map(|m| (next_id, m)),
                    Err(err) => Err(err),
                }
            }
            Err(err) => {
                if let Some(window_trace) = err.trace {
                    trace.more_runs.push(window_trace);
                }
                Err(err.message)
            }
        };
        match next {
            Ok((next_id, next)) => {
                manifest.merge_range(next);
                manifest_ids.push(next_id);
            }
            Err(err) => {
                log::warn!("llm_understand_media: page window failed: {err}");
                incomplete = Some(format!(
                    "Note: the catalog stops before page {}: {err}",
                    window.first().copied().unwrap_or(0)
                ));
                break;
            }
        }
    }
    Ok((manifest, manifest_ids, trace, incomplete))
}

/// The text each page is read by (text layer or OCR, normalized), and why
/// the others could not be read.
async fn read_page_texts(
    manifest: &buckyos_api::MediaManifest,
) -> (BTreeMap<u64, PageText>, BTreeMap<u64, String>) {
    let ids: Vec<(u64, String, pipeline::TextSource)> = manifest
        .entries
        .iter()
        .filter_map(|e| {
            let (id, source) = choose_text(e)?;
            Some((u64::from(e.location.page?), id, source))
        })
        .collect();
    let mut texts = BTreeMap::new();
    let mut unreadable = BTreeMap::new();
    for batch in ids.chunks(TEXT_READ_CONCURRENCY) {
        let mut reads = tokio::task::JoinSet::new();
        for (page, id, source) in batch.iter().cloned() {
            reads.spawn(async move { (page, source, read_object_bytes(&id).await) });
        }
        while let Some(read) = reads.join_next().await {
            match read {
                Ok((page, source, Ok(bytes))) => {
                    let text = normalize_text(&String::from_utf8_lossy(&bytes));
                    if !text.is_empty() {
                        texts.insert(page, PageText { text, source });
                    }
                }
                Ok((page, _, Err(err))) => {
                    log::warn!("llm_understand_media: read page {page} text failed: {err}");
                    unreadable.insert(page, err);
                }
                Err(err) => log::warn!("llm_understand_media: page text read aborted: {err}"),
            }
        }
    }
    (texts, unreadable)
}

async fn prepare_document(
    media: &ResolvedMedia,
    opts: &RunOpts,
    format: Option<&'static str>,
    reason: String,
) -> Result<PreparedMedia, PrepareError> {
    let family = if format.is_some() { FAMILY_DOCUMENT } else { FAMILY_PDF };
    let mut decision = MediaDecision::converted(media, family, reason);
    let obj_id = frozen_input(media, &decision).await?;
    decision.obj_id = Some(obj_id.to_string());
    let (manifest, manifest_ids, trace, incomplete) =
        build_document_catalog(family, &obj_id, format, opts, &decision).await?;
    let (texts, unreadable) = read_page_texts(&manifest).await;
    let given_as = match &media.payload {
        MediaPayload::File(path) if media._download.is_none() => Some(path.display().to_string()),
        _ => None,
    };
    let file = CatalogFile {
        source: CatalogSource {
            obj_id: obj_id.to_string(),
            name: media.name.clone(),
            mime: media.mime.clone(),
            family: family.to_string(),
            format: format.map(str::to_string),
            given_as,
        },
        manifest_ids,
        manifest,
    };
    let mut prepared = document_content(opts, &file, &texts, &unreadable, decision, &BTreeSet::new()).await?;
    prepared.trace = Some(trace);
    prepared.caller_note = incomplete;
    // A long document, or one the model does not see whole, is unpacked for the Agent.
    let long = file.manifest.range.total.unwrap_or(0) > u64::from(DocumentLimits::from_env().long_doc_pages);
    let partial = prepared.coverage.as_ref().is_some_and(|c| !c.uncovered.is_empty());
    if long || partial {
        let root = catalog::catalog_root();
        prepared.catalog_dir = Some(catalog::dir_of(&root, &file.source.obj_id));
        prepared.catalog_job = Some(CatalogJob { root, file, texts });
    }
    Ok(prepared)
}

/// The model input of a document catalog: the pages chosen for the goal (doc
/// §4.7). `only`: pages to read, whatever the goal says; empty for the goal's
/// own choice.
async fn document_content(
    opts: &RunOpts,
    file: &CatalogFile,
    texts: &BTreeMap<u64, PageText>,
    unreadable: &BTreeMap<u64, String>,
    decision: MediaDecision,
    only: &BTreeSet<u64>,
) -> Result<PreparedMedia, PrepareError> {
    let manifest = &file.manifest;
    let named = if only.is_empty() { opts.named_pages() } else { only.clone() };
    let budget = text_char_budget();
    let own_terms = goal_terms(&opts.goal);
    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new());
    let (terms, retrieval) =
        terms::search_terms(&llm, &opts.goal, &own_terms, &named, manifest, texts, budget).await;
    let request = PlanRequest { terms: &terms, named: &named, text_char_budget: budget, max_images: max_images() };
    let plan = plan_pages(&request, manifest, texts, unreadable);
    let derived = manifest.derived_content_id.is_some();
    let mut content = Vec::with_capacity(plan.texts.len() + plan.images.len() * 2 + 2);
    content.push(AiContent::text(format!(
        "Document catalog from the media pipeline ({}{}): {} pages in the source{}; page texts shown for pages {}; page images shown for pages {}. Page numbers refer to {}.{}",
        manifest.family,
        file.source.name.as_deref().map(|n| format!(", \u{ab}{n}\u{bb}")).unwrap_or_default(),
        manifest.range.total.map(|t| t.to_string()).unwrap_or_else(|| "?".to_string()),
        if derived { " (converted to PDF first)" } else { "" },
        pipeline::format_ranges(&plan.texts.iter().map(|(p, _)| *p).collect::<Vec<_>>()),
        pipeline::format_ranges(&plan.images.iter().map(|(p, _)| *p).collect::<Vec<_>>()),
        if derived { "the derived PDF" } else { "the original document" },
        if plan.coverage.text_ocr.is_empty() {
            String::new()
        } else {
            format!(
                " The text of pages {} was recognized from the page image (OCR) and may contain recognition errors.",
                pipeline::format_ranges(&plan.coverage.text_ocr)
            )
        },
    )));
    if let Some(index) = &plan.index {
        content.push(AiContent::text(index.clone()));
    }
    for (page, text) in &plan.texts {
        content.push(AiContent::text(format!("Page {page} text:\n{text}")));
    }
    for (page, image_id) in &plan.images {
        let source = named_object(image_id).map_err(|e| PrepareError::new(decision.clone(), e))?;
        content.push(AiContent::text(format!("Page {page} image:")));
        content.push(AiContent::image(source));
    }
    let reading = format!(
        "{} page image{} and {} page{} of text",
        plan.images.len(),
        if plan.images.len() == 1 { "" } else { "s" },
        plan.texts.len(),
        if plan.texts.len() == 1 { "" } else { "s" },
    );
    let mut prepared = PreparedMedia::new(content, decision);
    prepared.coverage = Some(plan.coverage);
    prepared.preprocess = Some(json!({
        "manifest_obj_ids": file.manifest_ids,
        "summary": manifest.summary(),
        "ocr": manifest.ocr.as_ref().map(|o| json!({"status": o.status, "lang": o.lang, "script": o.script, "garbled_layer": o.garbled_layer})),
    }));
    prepared.reading = Some(reading);
    prepared.retrieval = retrieval.and_then(|r| serde_json::to_value(r).ok());
    Ok(prepared)
}

/// A catalog directory as found, or rebuilt from the conversion cache when it
/// is gone (the run's steps all come from the step cache).
async fn open_catalog(
    opts: &RunOpts,
    target: &CatalogTarget,
) -> Result<(CatalogFile, BTreeMap<u64, PageText>, Option<PipelineTrace>), PrepareError> {
    let dir = target.dir();
    if let Some(file) = catalog::load(dir) {
        let texts = catalog::load_texts(dir, &file);
        return Ok((file, texts, None));
    }
    progress(format!("the catalog {} is gone; rebuilding it from the conversion cache", dir.display()));
    let obj_id = target.obj_id();
    let failed = |message: String| {
        PrepareError::new(
            MediaDecision {
                kind: "failed",
                reason: "rebuilding a catalog".to_string(),
                family: None,
                obj_id: Some(obj_id.to_string()),
                mime: String::new(),
            },
            message,
        )
    };
    let id = ObjId::new(obj_id).map_err(|e| failed(format!("`{obj_id}` is not an object id: {e}")))?;
    let head = object_head(&id, PROBE_HEAD_BYTES)
        .await
        .map_err(&failed)?
        .ok_or_else(|| failed(format!("the source object {obj_id} of this catalog cannot be read any more; call llm_understand_media with the original file")))?;
    let mime = head
        .mime
        .clone()
        .or_else(|| sniff_mime(&head.head))
        .unwrap_or_else(|| "application/pdf".to_string());
    let format = document_format_from_mime(&mime).or_else(|| head.name.as_deref().and_then(document_format_from_name));
    let family = if format.is_some() { FAMILY_DOCUMENT } else { FAMILY_PDF };
    let decision = MediaDecision {
        kind: "converted",
        reason: "rebuilding the catalog from the conversion cache".to_string(),
        family: Some(family),
        obj_id: Some(obj_id.to_string()),
        mime: mime.clone(),
    };
    let (manifest, manifest_ids, trace, _) = build_document_catalog(family, &id, format, opts, &decision).await?;
    let (texts, _) = read_page_texts(&manifest).await;
    let file = CatalogFile {
        source: CatalogSource {
            obj_id: obj_id.to_string(),
            name: head.name,
            mime,
            family: family.to_string(),
            format: format.map(str::to_string),
            given_as: None,
        },
        manifest_ids,
        manifest,
    };
    let root = dir.parent().map(Path::to_path_buf).unwrap_or_else(catalog::catalog_root);
    let written = catalog::write(&root, &file, &texts).await.map_err(&failed)?;
    progress(format!("catalog written to {}", written.display()));
    Ok((file, texts, Some(trace)))
}

/// `llm_understand_media <catalog>/pages/0101.png` (one page: its image as
/// converted, with its text) or `llm_understand_media <catalog> --pages …`:
/// the conversion is read from the catalog, nothing is stored again.
async fn run_catalog(opts: &RunOpts, target: CatalogTarget) -> (AgentToolResult, i32) {
    let (file, texts, trace) = match open_catalog(opts, &target).await {
        Ok(opened) => opened,
        Err(err) => {
            let mut result = build_error_result(opts, err.message);
            attach_pipeline_details(&mut result, Some(&err.decision), err.trace.as_ref(), None, None, None, None);
            return (result, CLI_EXIT_ERROR);
        }
    };
    let dir = target.dir().to_path_buf();
    let total = file.manifest.range.total.unwrap_or(0);
    let name = file.source.name.clone().unwrap_or_else(|| file.source.obj_id.clone());
    let decision = MediaDecision {
        kind: "catalog",
        reason: format!("read from the catalog {}", dir.display()),
        family: Some(if file.source.family == FAMILY_DOCUMENT { FAMILY_DOCUMENT } else { FAMILY_PDF }),
        obj_id: Some(file.source.obj_id.clone()),
        mime: file.source.mime.clone(),
    };
    let prepared = match &target {
        CatalogTarget::Page { page, .. } => {
            let entry = file.manifest.entries.iter().find(|e| e.location.page.map(u64::from) == Some(*page));
            let Some(image_id) = entry.and_then(|e| e.image_obj_id.clone()) else {
                let message = if *page > total {
                    format!("{name} has {total} pages; there is no page {page}")
                } else {
                    format!("page {page} of {name} has no page image")
                };
                return (build_error_result(opts, message), CLI_EXIT_USAGE);
            };
            let source = match named_object(&image_id) {
                Ok(source) => source,
                Err(err) => return (build_error_result(opts, err), CLI_EXIT_ERROR),
            };
            let mut content = vec![AiContent::text(format!(
                "This is page {page} of \u{ab}{name}\u{bb} ({total} pages){}.",
                if file.manifest.derived_content_id.is_some() { ", numbered as in the PDF it was converted to" } else { "" }
            ))];
            if let Some(text) = texts.get(page) {
                content.push(AiContent::text(format!(
                    "Page {page} text{}:\n{}",
                    if text.source == pipeline::TextSource::Ocr { " (recognized from the page image, may contain errors)" } else { "" },
                    text.text
                )));
            }
            content.push(AiContent::text(format!("Page {page} image:")));
            content.push(AiContent::image(source));
            let mut prepared = PreparedMedia::new(content, decision);
            prepared.coverage = Some(Coverage {
                family: file.manifest.family.clone(),
                total: Some(total),
                requested: vec![*page],
                processed: vec![*page],
                text_delivered: texts.contains_key(page).then_some(*page).into_iter().collect(),
                text_ocr: texts.get(page).filter(|t| t.source == pipeline::TextSource::Ocr).map(|_| *page).into_iter().collect(),
                image_delivered: vec![*page],
                complete: file.manifest.complete,
                ..Default::default()
            });
            prepared.reading = Some(format!("page {page} of {name}"));
            prepared.media_source = Some(json!({"catalog": file.source.obj_id, "page": page, "dir": dir.display().to_string()}));
            prepared
        }
        CatalogTarget::Dir { .. } => {
            let only = opts.named_pages();
            if !only.is_empty() && only.iter().all(|p| *p == 0 || *p > total) {
                return (build_error_result(opts, format!("{name} has {total} pages; pages {} are not in it", pipeline::format_ranges(&only.iter().copied().collect::<Vec<_>>()))), CLI_EXIT_USAGE);
            }
            match document_content(opts, &file, &texts, &BTreeMap::new(), decision, &only).await {
                Ok(mut prepared) => {
                    prepared.media_source = Some(json!({"catalog": file.source.obj_id, "pages": only, "dir": dir.display().to_string()}));
                    prepared
                }
                Err(err) => return (build_error_result(opts, err.message), CLI_EXIT_ERROR),
            }
        }
    };
    let mut prepared = prepared;
    prepared.trace = trace;
    prepared.catalog_dir = Some(dir);
    let mime = if matches!(target, CatalogTarget::Page { .. }) { "image/png" } else { file.source.mime.as_str() };
    let Some(model_alias) = opts.model.clone().or_else(|| route_model(mime)) else {
        return (build_error_result(opts, format!("no model route for media mime `{mime}`")), CLI_EXIT_ERROR);
    };
    if let Some(reading) = &prepared.reading {
        progress(format!("reading {reading} with {model_alias}"));
    }
    let llm: Arc<dyn LlmClient> = Arc::new(AiccLlmClient::new());
    let work_dir = opts.work_dir.clone().unwrap_or_else(|| default_work_dir(&opts.goal));
    let media_id = file.source.obj_id.clone();
    let analysis = Analysis {
        opts,
        history: &[],
        llm: &llm,
        model_alias: &model_alias,
        work_dir: &work_dir,
        mime,
        media_id: &media_id,
    };
    let (mut result, code) = analysis.run(&prepared).await;
    // The caller is in the catalog already: name it again only to say what was left out.
    let partial = prepared.coverage.as_ref().is_some_and(|c| !c.uncovered.is_empty());
    attach_pipeline_details(
        &mut result,
        Some(&prepared.decision),
        prepared.trace.as_ref(),
        prepared.coverage.as_ref(),
        prepared.preprocess.as_ref(),
        None,
        prepared.catalog_dir.as_deref().filter(|_| partial),
    );
    attach_extra_details(&mut result, &prepared);
    if opts.work_dir.is_none() && result.status == AgentToolStatus::Success {
        let _ = std::fs::remove_dir_all(&work_dir);
    }
    (result, code)
}

async fn prepare_video(media: &ResolvedMedia, opts: &RunOpts) -> Result<PreparedMedia, PrepareError> {
    let mut decision = MediaDecision::converted(
        media,
        FAMILY_VIDEO,
        "video frames are sampled and the audio track transcribed",
    );
    let obj_id = frozen_input(media, &decision).await?;
    decision.obj_id = Some(obj_id.to_string());
    let input = VideoLimits::from_env().pipeline_input(&obj_id.to_string());
    let (output, trace) = drive(FAMILY_VIDEO, &obj_id, input, opts, &decision).await?;
    let manifest_id = manifest_id_of(&output).map_err(|e| PrepareError::new(decision.clone(), e))?;
    let manifest = read_manifest(&manifest_id)
        .await
        .map_err(|e| PrepareError::new(decision.clone(), e))?;
    let frames = frame_entries(&manifest);
    let mut coverage = Coverage {
        family: manifest.family.clone(),
        total: manifest.range.total,
        requested: manifest.range.requested.clone(),
        processed: manifest.range.processed.clone(),
        failed: manifest.range.failed.clone(),
        complete: manifest.complete,
        ..Default::default()
    };
    let limit = max_images();
    let usable = frames.iter().filter(|e| e.image_obj_id.is_some()).count();
    let shown_positions: std::collections::BTreeSet<usize> = (0..limit.min(usable))
        .map(|i| i * usable / limit.min(usable).max(1))
        .collect();
    let mut content = Vec::with_capacity(frames.len() * 2 + 2);
    let mut position = 0usize;
    for entry in &frames {
        let coord = entry.location.time_ms.or(entry.location.index.map(u64::from)).unwrap_or(0);
        let Some(image_id) = entry.image_obj_id.as_ref() else {
            coverage.uncovered.push(pipeline::Uncovered {
                location: coord,
                reason: entry
                    .error
                    .as_ref()
                    .map(|e| format!("{}: {}", e.code, e.message))
                    .unwrap_or_else(|| "no frame produced".to_string()),
            });
            continue;
        };
        let shown = shown_positions.contains(&position);
        position += 1;
        if !shown {
            coverage.uncovered.push(pipeline::Uncovered { location: coord, reason: "omitted by the image budget".to_string() });
            continue;
        }
        let source = named_object(image_id).map_err(|e| PrepareError::new(decision.clone(), e))?;
        content.push(AiContent::text(format!("Frame at {:.3} seconds:", coord as f64 / 1000.0)));
        content.push(AiContent::image(source));
        coverage.image_delivered.push(coord);
    }
    content.insert(
        0,
        AiContent::text(format!(
            "The following {} images are uniformly sampled video frames in chronological order (video duration {} ms).",
            coverage.image_delivered.len(),
            manifest.range.total.map(|t| t.to_string()).unwrap_or_else(|| "?".to_string())
        )),
    );
    let mut caller_note = None;
    match audio_entry(&manifest) {
        Some(entry) if entry.status == buckyos_api::MediaEntryStatus::Absent => {
            coverage.audio = Some("absent".to_string());
            content.push(AiContent::text("The video has no audio track.".to_string()));
        }
        Some(entry) => match entry.audio_obj_id.as_ref() {
            Some(audio_id) => {
                let result = match named_object(audio_id) {
                    Ok(source) => transcribe_audio(source, opts.session_id.as_deref()).await,
                    Err(err) => Err(err),
                };
                match result {
                    Ok(transcript) => {
                        coverage.audio = Some("transcribed".to_string());
                        content.extend(audio_transcript_content(&transcript));
                    }
                    Err(err) => {
                        log::warn!("llm_understand_media: video audio transcription failed: {err}");
                        coverage.audio = Some(format!("failed: {err}"));
                        content.push(AiContent::text(format!(
                            "The audio track could not be transcribed ({err}); the analysis covers the frames only."
                        )));
                        caller_note = Some(
                            "Note: the audio track could not be transcribed, so this covers the frames only.".to_string(),
                        );
                    }
                }
            }
            None => {
                coverage.audio = Some(format!(
                    "failed: {}",
                    entry.error.as_ref().map(|e| e.message.clone()).unwrap_or_default()
                ));
            }
        },
        None => coverage.audio = Some("not_requested".to_string()),
    }
    let mut prepared = PreparedMedia::new(content, decision);
    prepared.trace = Some(trace);
    prepared.coverage = Some(coverage);
    prepared.preprocess = Some(json!({"manifest_obj_id": manifest_id}));
    prepared.caller_note = caller_note;
    Ok(prepared)
}

async fn transcribe_audio(source: ResourceRef, session_id: Option<&str>) -> Result<String, String> {
    let runtime = get_buckyos_api_runtime()
        .map_err(|err| format!("get buckyos runtime for audio transcription failed: {err}"))?;
    let client = runtime
        .get_aicc_client()
        .await
        .map_err(|err| format!("get aicc client for audio transcription failed: {err}"))?;

    let mut route_request = RouteResolveRequest::new(
        ApiType::AudioSpeechRecognition,
        DEFAULT_AUDIO_ASR_MODEL_ALIAS,
    );
    route_request.session_id = session_id.map(str::to_string);
    let route = client
        .route_resolve(route_request)
        .await
        .map_err(|err| format!("resolve audio.asr model failed: {err}"))?;

    let mut request = AudioSpeechRecognitionRequest::new(route.selected_exact_model, source);
    request.session_id = session_id.map(str::to_string);
    let response = client
        .audio_speech_recognition(request)
        .await
        .map_err(|err| format!("aicc audio.asr failed: {err}"))?;

    match response.status {
        AiMethodStatus::Succeeded => {
            let text = response
                .text
                .filter(|text| !text.trim().is_empty())
                .or_else(|| {
                    let text = response
                        .segments
                        .iter()
                        .map(|segment| segment.text.trim())
                        .filter(|text| !text.is_empty())
                        .collect::<Vec<_>>()
                        .join("\n");
                    (!text.is_empty()).then_some(text)
                })
                .ok_or_else(|| "audio.asr succeeded but returned an empty transcript".to_string())?;
            Ok(text)
        }
        AiMethodStatus::Failed => Err(format!(
            "audio.asr failed: task_id={}, event_ref={}, error={}",
            response.task_id,
            response.event_ref.as_deref().unwrap_or(""),
            response
                .error
                .as_ref()
                .and_then(|error| serde_json::to_string(error).ok())
                .unwrap_or_else(|| "<none>".to_string())
        )),
        AiMethodStatus::Running => Err(format!(
            "audio.asr returned async task `{}`; llm_understand_media requires an immediate transcription",
            response.task_id
        )),
    }
}

fn audio_transcript_content(transcript: &str) -> Vec<AiContent> {
    vec![AiContent::text(format!(
        "Audio preprocessing result from automatic speech recognition. This evidence covers intelligible speech only; it does not identify music, ambient sounds, emotion, or the scene unless those are stated in the transcript.\n\nTranscript:\n{}",
        transcript.trim()
    ))]
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
        ContextOutput::Text { content } | ContextOutput::Invalid { content, .. } => {
            parse_jsonish(content)?
        }
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
        if let Some(location) = obs.location.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            out.push_str(" (");
            out.push_str(location);
            out.push(')');
        }
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
        ContextOutput::Text { content } | ContextOutput::Invalid { content, .. } => {
            content.clone()
        }
        ContextOutput::Json { content } => {
            serde_json::to_string_pretty(content).unwrap_or_else(|_| content.to_string())
        }
    }
}

fn context_output_kind(output: &ContextOutput) -> &'static str {
    match output {
        ContextOutput::Text { .. } => "text",
        ContextOutput::Json { .. } => "json",
        ContextOutput::Invalid { .. } => "invalid",
    }
}

fn run_final_outcome_path(work_dir: &Path, run_id: &str) -> PathBuf {
    work_dir.join(run_id).join("run.json")
}

fn write_parse_error_raw_output(
    work_dir: &Path,
    run_id: &str,
    raw_output: &str,
) -> std::io::Result<PathBuf> {
    let outcomes_dir = work_dir.join(run_id);
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
                    "required": ["id", "description", "where"],
                    "additionalProperties": false,
                    "properties": {
                        "id": { "type": "string" },
                        "description": { "type": "string" },
                        "where": { "type": ["string", "null"] }
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
        hints: Vec::new(),
        dedup: None,
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
        images: Vec::new(),
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
    std::env::temp_dir().join(format!(
        "llm_understand_media-{ts}-{}-{safe}",
        std::process::id()
    ))
}

struct NoopToolManager;

#[async_trait]
impl ToolManager for NoopToolManager {
    async fn call_tool(
        &self,
        call: buckyos_api::AiToolCall,
        _ctx: llm_context::ToolCallCtx,
    ) -> Result<llm_context::Observation, llm_context::ToolDispatchError> {
        Ok(llm_context::Observation::Error {
            call_id: call.call_id,
            message: "tools are disabled in llm_understand_media".to_string(),
            host: None,
        })
    }
}

const USAGE: &str = r#"Usage: llm_understand_media <file | object id | url> [--goal <text>] [options]
       agent_tool llm_understand_media --media <json | locator> [--goal <text>] [options]

Media (one of):
  <locator>               A local file (relative to the current directory or absolute), a Zone file
                          (cyfs:///home/<owner>/…, or the path Files shows: /home/<owner>/…), an object id
                          (cyfile:…, a chunk id such as mix256:…, cyfs://<objid>/; e.g. an attachment of a
                          message), an http(s) URL, or a catalog directory / page image (see below).
  --media <json>          The same as a JSON ResourceRef, e.g. {"kind":"named_object","obj_id":"cyfile:…","mime_hint":"image/png"},
                          {"kind":"local_file","path":"/abs/frame.png"}, or {"url":"https://…"}.

Options:
  --goal <text>           What to find out (default: describe the content in detail).
  --pages <list>          Pages of a document to read, e.g. 14,50,70 or 2-33 (pages the goal names count too).
  --history-file <path>   JSON Vec<AiMessage> parent history snapshot.
  --work-dir <path>       LocalLLMContext working directory.
  --model <alias>         AICC logical model alias; default image route is llm.vision.
  --max-completion-tokens <n>  Positive output budget; default 2048, rounded up to 2048/4096/8192 tiers.
  -h, --help              Show this help.

Long documents (PDF, docx, pptx, ...):
  The first call converts the document: every page gets its text and a page image (text recognized by OCR
  where the PDF has none or a garbled one). That takes minutes for hundreds of pages; progress lines go to
  stderr meanwhile. Do not render or extract pages yourself.
  The conversion is kept: asking again with another goal, or with --pages, converts nothing and costs one
  more model call.
  A long document is also unpacked into a local directory, named in the output:
    <dir>/index.tsv          one row a page: page, status, text_source (layer|ocr), chars, image size, first line
    <dir>/pages/0001.txt     the page's text        <dir>/pages/0001.png   the page image
  Search it with grep, then look at the pages that matter:
    llm_understand_media <dir>/pages/0101.png --goal "…"     one page: its image and its text
    llm_understand_media <dir> --pages 14,50,70 --goal "…"   several pages together
  The directory may be deleted; any path into it rebuilds it from the kept conversion.
"#;

/// The goal of a CLI call without `--goal`.
const DEFAULT_CLI_GOAL: &str = "Describe this content in detail: what it shows or says, any text in it (verbatim), its structure, and anything notable, so that someone who cannot see it can work with it.";

#[derive(Debug)]
struct CliOpts {
    media: Value,
    goal: String,
    history_file: Option<PathBuf>,
    work_dir: Option<PathBuf>,
    model: Option<String>,
    max_completion_tokens: Option<u32>,
    pages: BTreeSet<u64>,
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
            session_id: None,
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
            pages: self.pages,
        })
    }

    fn parse(args: &[String]) -> Result<Self, ParseError> {
        let mut media: Option<Value> = None;
        let mut goal: Option<String> = None;
        let mut history_file: Option<PathBuf> = None;
        let mut work_dir: Option<PathBuf> = None;
        let mut model: Option<String> = None;
        let mut max_completion_tokens = None;
        let mut pages = BTreeSet::new();

        let mut idx = 0;
        fn set_media(value: Value, media: &mut Option<Value>) -> Result<(), ParseError> {
            if media.replace(value).is_some() {
                return Err(ParseError::Bad("give one media only".to_string()));
            }
            Ok(())
        }
        while idx < args.len() {
            match args[idx].as_str() {
                "-h" | "--help" => return Err(ParseError::Help),
                "--media" => {
                    let raw = next_value(args, &mut idx, "--media")?;
                    // A JSON ResourceRef, or a bare locator.
                    let value = match serde_json::from_str::<Value>(&raw) {
                        Ok(v @ Value::Object(_)) => v,
                        _ => Value::String(raw),
                    };
                    set_media(value, &mut media)?;
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
                "--pages" => {
                    pages.extend(parse_pages_arg(&next_value(args, &mut idx, "--pages")?).map_err(ParseError::Bad)?);
                }
                "--max-completion-tokens" => {
                    max_completion_tokens = Some(parse_positive_u32(
                        &next_value(args, &mut idx, "--max-completion-tokens")?,
                        "--max-completion-tokens",
                    )?);
                }
                other if other.starts_with("--") => {
                    return Err(ParseError::Bad(format!("unknown flag `{other}`")))
                }
                // The locator itself (`llm_understand_media photo.png`).
                locator => set_media(Value::String(locator.to_string()), &mut media)?,
            }
            idx += 1;
        }

        Ok(Self {
            media: media.ok_or_else(|| {
                ParseError::Bad("missing media: a file, a Zone file (cyfs:///…), an object id (cyfile:…) or a url".to_string())
            })?,
            goal: goal
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| DEFAULT_CLI_GOAL.to_string()),
            history_file,
            work_dir,
            model,
            max_completion_tokens,
            pages,
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
    use buckyos_api::{AiToolResultContent, AiUsage};

    fn cli(args: &[&str]) -> Result<CliOpts, ParseError> {
        CliOpts::parse(&args.iter().map(|a| a.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn chunk_ids_are_object_ids() {
        let chunk = "mix256:8d11eaa912debbdd30ccbbebbdae7ac4950b321afaaf740dd98ad76af3d7cfdc19c9";
        assert!(looks_like_object_id(chunk));
        assert!(matches!(
            parse_media_locator(chunk),
            Ok(MediaSource::Resource(ResourceRef::NamedObject { .. }))
        ));
        assert!(!looks_like_object_id("notes/a.png"));
        assert!(!looks_like_object_id("a.png"));
    }

    #[test]
    fn the_cli_takes_a_bare_locator_and_a_default_goal() {
        let o = cli(&["photo.png"]).ok().unwrap();
        assert_eq!(o.media, Value::String("photo.png".into()));
        assert_eq!(o.goal, DEFAULT_CLI_GOAL);
        let o = cli(&["cyfile:abc", "--goal", "what error is shown?"]).ok().unwrap();
        assert_eq!(o.media, Value::String("cyfile:abc".into()));
        assert_eq!(o.goal, "what error is shown?");
        // `--media` takes JSON or a locator.
        let o = cli(&["--media", r#"{"kind":"local_file","path":"/a.png"}"#]).ok().unwrap();
        assert_eq!(o.media["path"], "/a.png");
        let o = cli(&["--media", "https://example.com/a.png"]).ok().unwrap();
        assert_eq!(o.media, Value::String("https://example.com/a.png".into()));
        assert!(matches!(cli(&[]), Err(ParseError::Bad(_))));
        assert!(matches!(cli(&["a.png", "b.png"]), Err(ParseError::Bad(_))));
        assert!(matches!(cli(&["a.png", "--nope"]), Err(ParseError::Bad(_))));
        assert!(matches!(cli(&["--help"]), Err(ParseError::Help)));
    }

    #[test]
    fn a_zone_file_parses_bare_or_in_any_media_field() {
        let book = "cyfs://home/English for Everyone (Audio).pdf";
        let expected = vec!["home".to_string(), "English for Everyone (Audio).pdf".to_string()];
        for value in [
            json!(book),
            json!({ "url": book }),
            json!({ "uri": book }),
            json!({ "kind": "local_file", "path": book }),
            json!({ "kind": "named_object", "obj_id": book }),
        ] {
            match parse_media_arg(&value).unwrap().source {
                MediaSource::ZoneFile { zone: None, segments } => assert_eq!(segments, expected, "{value}"),
                other => panic!("{value}: {other:?}"),
            }
        }
        let id = ObjId::new(OBJECT).unwrap();
        let in_url = json!({ "kind": "named_object", "obj_id": format!("cyfs://{}/", id.to_base32()) });
        assert!(matches!(
            parse_media_arg(&in_url).unwrap().source,
            MediaSource::Resource(ResourceRef::NamedObject { obj_id }) if obj_id == id
        ));
        // A path that is not a file in the process's directory is still a path:
        // resolution tries the session workspace and the Zone.
        assert!(matches!(
            parse_media_arg(&json!("/home/alice/Documents/report.pdf")).unwrap().source,
            MediaSource::LocalFile(_)
        ));
        assert!(matches!(
            parse_media_arg(&json!("frame008.png")).unwrap().source,
            MediaSource::LocalFile(_)
        ));
        assert!(parse_media_arg(&json!("not-a-locator")).is_err());
    }

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
                location: Some("page 3".to_string()),
            }],
            reasoning: "obs-1 indicates an error state.".to_string(),
            conclusion: "The screen shows an error.".to_string(),
            confidence: Confidence::Observed,
        };
        let text = render_report(&report);
        assert!(text.contains("Observations:"));
        assert!(text.contains("- obs-1 (page 3): A red error banner"));
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
            session_id: None,
            goal: "describe image".to_string(),
            parent_history: Vec::new(),
            work_dir: None,
            model: None,
            summary_model: DEFAULT_SUMMARY_MODEL_ALIAS.to_string(),
            target_tokens: DEFAULT_TARGET_TOKENS,
            max_completion_tokens: DEFAULT_MAX_COMPLETION_TOKENS,
            pages: BTreeSet::new(),
        };

        let input = build_input(
            &opts,
            vec![AiContent::image(ResourceRef::Base64 {
                mime: "image/png".to_string(),
                data_base64: "AAAA".to_string(),
            })],
            Vec::new(),
        );
        assert_eq!(input.len(), 2);
        assert_eq!(input[0].role, AiRole::System);
        let overrides = media_overrides(
            &opts,
            DEFAULT_MODEL_ALIAS.to_string(),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(overrides.max_tokens, Some(2_048));
        assert_eq!(overrides.tools, Some(false));
        assert!(overrides.json);
        assert!(overrides
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
        let overrides = media_overrides(
            &opts,
            DEFAULT_MODEL_ALIAS.to_string(),
            PathBuf::from("/tmp/x"),
        );
        assert_eq!(overrides.max_tokens, Some(16_384));
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
    fn non_archive_media_mime_routes_to_model_and_sniffs_common_containers() {
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
        assert_eq!(default_model_alias("audio/mpeg"), "llm.chat");
        assert_eq!(default_model_alias("image/png"), "llm.vision");
    }

    const OBJECT: &str = "cyfile:d05ec9f1d9ff3713fda374c2ad1e1a9ff1b168c941772df29769935fbbb49fdd";

    fn resolved(mime: &str, size: u64, head: &[u8], name: Option<&str>) -> ResolvedMedia {
        ResolvedMedia {
            payload: MediaPayload::Object(ObjId::new(OBJECT).unwrap()),
            mime: mime.to_string(),
            size,
            name: name.map(str::to_string),
            head: head.to_vec(),
            _download: None,
        }
    }

    #[test]
    fn media_decisions_name_the_family_and_reason() {
        let media = resolved("application/pdf", 10, b"%PDF-1.7", Some("a.pdf"));
        let converted = MediaDecision::converted(&media, FAMILY_PDF, "no document support");
        let value = converted.to_value();
        assert_eq!(value["kind"], "converted");
        assert_eq!(value["family"], "media.pdf");
        assert_eq!(value["obj_id"], media.object_id().unwrap());
        let direct = MediaDecision::direct(&media, "within budget").to_value();
        assert_eq!(direct["family"], Value::Null);

        let docx = resolved("application/zip", 10, b"PK\x03\x04", Some("slides.pptx"));
        assert_eq!(
            document_format_from_mime(&docx.mime).or_else(|| docx.name.as_deref().and_then(document_format_from_name)),
            Some("pptx")
        );
    }

    #[tokio::test]
    async fn the_model_reads_objects_itself_and_gets_small_local_media_inline() {
        let object = resolved("image/png", 10, b"", Some("a.png"));
        assert!(matches!(
            object.model_source().await.unwrap(),
            ResourceRef::NamedObject { .. }
        ));
        let inline = ResolvedMedia {
            payload: MediaPayload::Bytes(b"abc".to_vec()),
            mime: "image/png".to_string(),
            size: 3,
            name: None,
            head: Vec::new(),
            _download: None,
        };
        assert!(inline.object_id().is_none());
        assert_eq!(inline.label(), "<inline media>");
        let ResourceRef::Base64 { mime, data_base64 } = inline.model_source().await.unwrap() else {
            panic!("small local media stays inline");
        };
        assert_eq!((mime.as_str(), data_base64.as_str()), ("image/png", "YWJj"));
    }

    #[test]
    fn named_media_trusts_content_unless_a_name_says_more_than_zip() {
        let zip = b"PK\x03\x04";
        assert_eq!(
            named_media_mime(zip, Some("a.pptx"), None, None).as_deref(),
            Some("application/vnd.openxmlformats-officedocument.presentationml.presentation")
        );
        assert_eq!(named_media_mime(zip, None, None, None).as_deref(), Some("application/zip"));
        assert_eq!(
            named_media_mime(b"%PDF-1.7", Some("a.bin"), Some("text/plain".to_string()), None).as_deref(),
            Some("application/pdf")
        );
        assert_eq!(
            named_media_mime(b"??", None, Some("application/octet-stream".to_string()), Some("audio/mpeg".to_string())).as_deref(),
            Some("audio/mpeg")
        );
    }

    #[tokio::test]
    async fn a_local_file_is_only_probed_and_bounded() {
        let dir = std::env::temp_dir().join(format!("llm_understand_media-probe-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("a.png");
        std::fs::write(&path, vec![7u8; PROBE_HEAD_BYTES + 10]).unwrap();
        let (size, head) = probe_file(&path).await.unwrap();
        assert_eq!(size as usize, PROBE_HEAD_BYTES + 10);
        assert_eq!(head.len(), PROBE_HEAD_BYTES);
        let empty = dir.join("empty.png");
        std::fs::write(&empty, b"").unwrap();
        assert!(probe_file(&empty).await.unwrap_err().contains("empty"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn image_pass_through_needs_known_dimensions_within_limits() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&4096u32.to_be_bytes());
        png.extend_from_slice(&1024u32.to_be_bytes());
        let limits = ImageLimits::default();
        let (w, h) = image_dimensions(&png).unwrap();
        assert!(!limits.allows(w, h, 1000), "4096x1024 exceeds the long edge");
        let mut small = png.clone();
        small[16..20].copy_from_slice(&32u32.to_be_bytes());
        small[20..24].copy_from_slice(&32u32.to_be_bytes());
        let (w, h) = image_dimensions(&small).unwrap();
        assert!(limits.allows(w, h, 1000));
        assert!(!limits.allows(w, h, limits.max_bytes + 1));
        assert!(PASSTHROUGH_IMAGE_MIMES.contains(&"image/png"));
        assert!(!PASSTHROUGH_IMAGE_MIMES.contains(&"image/tiff"));
        assert_eq!(image_dimensions(b"II*\x00"), None);
        let input = limits.pipeline_input("cyfile:ab");
        assert_eq!(input["max_long_edge"], 2048);
        assert_eq!(input["obj_id"], "cyfile:ab");
    }

    #[test]
    fn results_carry_decision_trace_and_coverage() {
        let opts = RunOpts::from_tool_args(json!({
            "media": { "kind": "named_object", "obj_id": OBJECT },
            "goal": "describe",
            "max_completion_tokens": 100
        }))
        .unwrap();
        let media = resolved("application/pdf", 10, b"%PDF-1.7", Some("a.pdf"));
        let decision = MediaDecision::converted(&media, FAMILY_PDF, "convert");
        let trace = PipelineTrace {
            workflow_id: "media.pdf".to_string(),
            run_id: "run-9".to_string(),
            revision: "rev".to_string(),
            status: "completed".to_string(),
            ..Default::default()
        };
        let mut result = build_error_result(&opts, "boom");
        result.output = Some("Conclusion:\nok".to_string());
        let coverage = Coverage {
            family: "pdf".to_string(),
            requested: vec![1, 2, 3],
            uncovered: vec![pipeline::Uncovered { location: 3, reason: "tool_failed: render".to_string() }],
            ..Default::default()
        };
        attach_pipeline_details(
            &mut result,
            Some(&decision),
            Some(&trace),
            Some(&coverage),
            Some(&json!({"original": {"width": 4096}, "sent": {"width": 2048}})),
            Some("Note: analyzed a downscaled copy (4096x1024 -> 2048x512); for fine detail, crop the region of interest and analyze the crop."),
            None,
        );
        assert_eq!(result.details["media_decision"]["family"], "media.pdf");
        assert_eq!(result.details["pipeline_trace"]["revision"], "rev");
        assert_eq!(result.details["coverage"]["uncovered"][0]["where"], 3);
        assert_eq!(result.details["media_preprocess"]["sent"]["width"], 2048);
        let output = result.output.unwrap();
        assert!(output.contains("analyze the crop."));
        assert!(output.contains("were not shown to the model"));
        assert!(!serde_json::to_string(&result.details).unwrap().contains("data_base64"));
    }

    #[test]
    fn audio_transcript_is_forwarded_as_text_with_scope_limit() {
        let content = audio_transcript_content(" hello world ");
        assert_eq!(content.len(), 1);
        let AiContent::Text { text } = &content[0] else {
            panic!("audio transcript must become text content");
        };
        assert!(text.contains("speech only"));
        assert!(text.ends_with("Transcript:\nhello world"));
    }

    #[test]
    fn build_request_preserves_timestamped_video_frames() {
        let opts = RunOpts {
            media_value: json!({}),
            session_id: None,
            goal: "find the action time".to_string(),
            parent_history: Vec::new(),
            work_dir: None,
            model: None,
            summary_model: DEFAULT_SUMMARY_MODEL_ALIAS.to_string(),
            target_tokens: DEFAULT_TARGET_TOKENS,
            max_completion_tokens: DEFAULT_MAX_COMPLETION_TOKENS,
            pages: BTreeSet::new(),
        };
        let media_content = vec![
            AiContent::text("Frame at 1.250 seconds:"),
            AiContent::image(ResourceRef::Base64 {
                mime: "image/jpeg".to_string(),
                data_base64: "AAAA".to_string(),
            }),
        ];
        let input = build_input(&opts, media_content, Vec::new());
        let user = input.last().expect("user message");
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
        let mut record = crate::xllm::RunRecord::synthetic(
            run_id,
            &work_dir,
            crate::xllm::RunStatus::Completed,
        );
        record.result = Some(crate::xllm::RunResultRecord {
            raw: raw.to_string(),
            extracted: None,
            extract_error: None,
            json_valid: None,
            json_error: None,
            response_model: None,
        });
        record.usage.main = Some(AiUsage {
            input_tokens: Some(1),
            output_tokens: Some(2),
            total_tokens: Some(3),
            ..Default::default()
        });
        let outcome = RunOutcome::Completed(record);

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
            .ends_with("run.json"));

        let _ = std::fs::remove_dir_all(work_dir);
    }

    #[test]
    fn parse_media_arg_keeps_typed_object_ids() {
        let arg = parse_media_arg(&json!({
            "kind": "named_object",
            "obj_id": "cyfile:f155c4725272667e44761ec037d8247aae48848c5a33d86e88303d5841228b9f",
            "mime_hint": "video/mp4",
        }))
        .expect("typed object id must stay a named_object");
        assert_eq!(arg.mime_hint.as_deref(), Some("video/mp4"));
        assert!(matches!(
            arg.source,
            MediaSource::Resource(ResourceRef::NamedObject { .. })
        ));
    }

    #[test]
    fn parse_media_arg_accepts_url_and_data_url() {
        let url = parse_media_arg(&json!({ "url": "https://example.test/a.mp4" })).unwrap();
        assert!(matches!(
            url.source,
            MediaSource::Resource(ResourceRef::Url { .. })
        ));

        let data = parse_media_arg(&json!("data:image/png;base64,AAAA")).unwrap();
        match data.source {
            MediaSource::Resource(ResourceRef::Base64 { mime, data_base64 }) => {
                assert_eq!(mime, "image/png");
                assert_eq!(data_base64, "AAAA");
            }
            other => panic!("expected base64 media, got {other:?}"),
        }
    }

    #[test]
    fn parse_media_arg_accepts_local_files_including_paths_mistaken_for_ids() {
        let dir = std::env::temp_dir().join(format!("lum-media-arg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("frame008.png");
        std::fs::write(&file, b"png-bytes").unwrap();
        let file_str = file.to_string_lossy().to_string();

        let explicit =
            parse_media_arg(&json!({ "kind": "local_file", "path": file_str.clone() })).unwrap();
        assert!(matches!(explicit.source, MediaSource::LocalFile(_)));

        let bare = parse_media_arg(&json!(file_str.clone())).unwrap();
        assert!(matches!(bare.source, MediaSource::LocalFile(_)));

        // The mistake that motivated this: a filesystem path passed as an
        // object id must become a local file, not a base32 decode failure.
        let coerced =
            parse_media_arg(&json!({ "kind": "named_object", "obj_id": file_str })).unwrap();
        match coerced.source {
            MediaSource::LocalFile(path) => {
                assert_eq!(path.file_name().unwrap(), "frame008.png")
            }
            other => panic!("expected a local file, got {other:?}"),
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_local_file_reports_every_candidate_path() {
        let missing = "definitely-missing-frame-9f3a.png";
        let err = resolve_local_file_path(Path::new(missing), Some("ui-test")).unwrap_err();
        assert!(err.contains("does not exist"), "{err}");
        assert!(err.contains("absolute path"), "{err}");
    }

    #[test]
    fn media_finds_sessions_directly_in_the_agents_data_directory() {
        if let Some(root) = env_path("BUCKYOS_MEDIA_ROOT_TEST") {
            let candidates = local_path_candidates(Path::new("frame.png"), Some("ui-test"));
            assert!(candidates.contains(&root.join("sessions/ui-test/frame.png")));
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "llm_understand_media::tests::media_finds_sessions_directly_in_the_agents_data_directory"])
            .env_remove(crate::runtime_context::OPENDAN_AGENT_ROOT_ENV)
            .env("BUCKYOS_DATA_DIR", root.path())
            .env("BUCKYOS_MEDIA_ROOT_TEST", root.path())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        assert!(String::from_utf8_lossy(&out.stdout).contains("1 passed"));
    }

    #[test]
    fn relative_paths_also_consider_the_session_workspace() {
        let agent_root = PathBuf::from("/opt/agent");
        let candidates =
            local_path_candidates_under(Path::new("frame008.png"), Some("ui-abc"), &[agent_root]);
        assert_eq!(candidates[0], PathBuf::from("frame008.png"));
        assert_eq!(
            candidates[1],
            PathBuf::from("/opt/agent/sessions/ui-abc/frame008.png")
        );

        // Absolute paths are taken verbatim, and an unknown session adds
        // nothing.
        assert_eq!(
            local_path_candidates_under(Path::new("/tmp/x.png"), Some("ui-abc"), &[]),
            vec![PathBuf::from("/tmp/x.png")]
        );
        assert_eq!(
            local_path_candidates_under(Path::new("a.png"), None, &[PathBuf::from("/opt/agent")]),
            vec![PathBuf::from("a.png")]
        );
    }
}
