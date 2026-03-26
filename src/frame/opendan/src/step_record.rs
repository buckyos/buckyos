use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use buckyos_api::{
    AiPayload, Capability, CompleteRequest, ModelSpec, MsgRecordWithObject, Requirements,
};
use chrono::{DateTime, Utc};
use log::warn;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

use crate::agent_tool::{AgentHistoryShowLevel, AgentToolError, AgentToolResult};
use crate::behavior::{BehaviorLLMResult, Tokenizer};

const DEFAULT_STEP_RECORD_FILE: &str = "llm_step_record.jsonl";

/*
Step history 的 HistoryShow 只保留 4 档：
- Min: title + new_msg + conclusion
- Mini: Min + reply_msg
- Medium: Mini + action_result(medium)
- Full: Mini + action_result(full)

完整渲染不属于 HistoryShow，只用于 render_last_step_text：
- Complete: title + new_msg + conclusion + thinking + reply_msg + action_result(uncompressed)

预算控制：
- 先给最近的 step 做种子分配：2 个 Full + 1 个 Medium + 1 个 Mini
- 然后在预算内按“最近优先”逐级升级剩余 step
- action_result 的 4 档渲染复用 agent_tool 的 AgentHistoryShowLevel

输出格式:
<steps_summary>
<step behavior="plan" step_num=0 step_time="2026-03-23 10:00:00">
<recive_msg from="Alic">msg content</recive_msg>
<conclusion>初始搜索确定 6 个候选方案：TrueNAS SCALE、OMV、Unraid、CasaOS、Rockstor、XigmaNAS。后两者社区较小，暂列备选。</conclusion>
<thinking>分析了 6 个候选方案的优缺点，选择了 TrueNAS SCALE 作为首选方案。</thinking>
<reply_msg to="Alic">TrueNAS SCALE 的优点是：xxxx</reply_msg>
<action>
- write xxxx => success
- tool-2 => success
```output
step-2-line-1
step-2-line-2
step-2-line-3
step-2-line-4
step-2-line-5
step-2-line-6
step-2-line-7
step-2-line-8
... [TRUNCATED: showing first 8 lines only] ...
```
</action>
</step>
</steps_summary>

*/

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LLMStepRecord {
    pub session_id: String,
    pub step_num: u32,
    pub step_index: u32,
    pub behavior_name: String,
    pub started_at_ms: u64,
    pub llm_completed_at_ms: u64,
    pub action_completed_at_ms: u64,
    pub new_msg: Vec<MsgRecordWithObject>,
    //pub new_event : Vec<EventRecord>,
    pub input: String,
    #[serde(default = "empty_complete_request")]
    pub llm_prompt: CompleteRequest,
    pub llm_result: BehaviorLLMResult,
    pub action_result: HashMap<String, AgentToolResult>,
    pub error: Option<String>,
}

impl Default for LLMStepRecord {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            step_num: 0,
            step_index: 0,
            behavior_name: String::new(),
            started_at_ms: 0,
            llm_completed_at_ms: 0,
            action_completed_at_ms: 0,
            new_msg: vec![],
            input: String::new(),
            llm_prompt: empty_complete_request(),
            llm_result: BehaviorLLMResult::default(),
            action_result: HashMap::new(),
            error: None,
        }
    }
}

impl LLMStepRecord {
    pub fn behavior_step_label(&self) -> String {
        let behavior_name = self.behavior_name.trim();
        if behavior_name.is_empty() {
            return format!("step:{}", self.step_index);
        }
        format!("{behavior_name}:{}", self.step_index)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RenderCompressionLevel {
    Min,
    Mini,
    Medium,
    Full,
    Complete,
}

impl RenderCompressionLevel {
    fn next_history_level(self) -> Option<Self> {
        match self {
            Self::Min => Some(Self::Mini),
            Self::Mini => Some(Self::Medium),
            Self::Medium => Some(Self::Full),
            Self::Full | Self::Complete => None,
        }
    }

    fn action_result_level(self) -> Option<AgentHistoryShowLevel> {
        match self {
            Self::Min | Self::Mini => None,
            Self::Medium => Some(AgentHistoryShowLevel::Medium),
            Self::Full | Self::Complete => Some(AgentHistoryShowLevel::Full),
        }
    }

    fn shows_reply(self) -> bool {
        matches!(
            self,
            Self::Mini | Self::Medium | Self::Full | Self::Complete
        )
    }

    fn shows_thinking(self) -> bool {
        matches!(self, Self::Complete)
    }

    fn history_index(self) -> usize {
        match self {
            Self::Min => 0,
            Self::Mini => 1,
            Self::Medium => 2,
            Self::Full => 3,
            Self::Complete => 4,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LLMStepPromptRenderOptions {
    pub max_render_steps: usize,
    pub recent_detail_steps: usize,
    pub max_render_tokens: u32,
    pub max_render_chars: usize,
    pub max_conclusion_chars: usize,
    pub max_thinking_chars: usize,
    pub max_next_action_chars: usize,
    pub max_action_result_chars: usize,
    pub max_new_msg_chars: usize,
}

impl Default for LLMStepPromptRenderOptions {
    fn default() -> Self {
        Self {
            max_render_steps: 24,
            recent_detail_steps: 4,
            max_render_tokens: 0,
            max_render_chars: 0,
            max_conclusion_chars: 280,
            max_thinking_chars: 400,
            max_next_action_chars: 280,
            max_action_result_chars: 1200,
            max_new_msg_chars: 240,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct LLMStepRecordLog {
    session_id: String,
    session_root_dir: PathBuf,
    loaded: bool,
    records: Vec<LLMStepRecord>,
}

impl LLMStepRecordLog {
    pub fn bind_session(&mut self, session_id: &str, session_root_dir: &Path) {
        let normalized_session_id = session_id.trim().to_string();
        let normalized_root = session_root_dir.to_path_buf();
        let changed =
            self.session_id != normalized_session_id || self.session_root_dir != normalized_root;
        self.session_id = normalized_session_id;
        self.session_root_dir = normalized_root;
        if changed {
            self.loaded = false;
            self.records.clear();
        }
    }

    pub fn record_file_path(&self) -> Option<PathBuf> {
        let session_id = self.session_id.trim();
        if session_id.is_empty() || self.session_root_dir.as_os_str().is_empty() {
            return None;
        }
        Some(
            self.session_root_dir
                .join(session_id)
                .join(DEFAULT_STEP_RECORD_FILE),
        )
    }

    pub async fn append(&mut self, record: LLMStepRecord) -> Result<(), AgentToolError> {
        self.ensure_loaded().await?;

        if let Some(path) = self.record_file_path() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).await.map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "create llm step record dir `{}` failed: {err}",
                        parent.display()
                    ))
                })?;
            }
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .await
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!(
                        "open llm step record file `{}` failed: {err}",
                        path.display()
                    ))
                })?;
            let line = serde_json::to_string(&record).map_err(|err| {
                AgentToolError::ExecFailed(format!("serialize llm step record failed: {err}"))
            })?;
            file.write_all(line.as_bytes()).await.map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "write llm step record file `{}` failed: {err}",
                    path.display()
                ))
            })?;
            file.write_all(b"\n").await.map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "finalize llm step record file `{}` failed: {err}",
                    path.display()
                ))
            })?;
        }

        self.records.push(record);
        Ok(())
    }

    pub async fn render_prompt_text(
        &mut self,
        options: Option<&LLMStepPromptRenderOptions>,
    ) -> Result<String, AgentToolError> {
        self.ensure_loaded().await?;
        let options = options.cloned().unwrap_or_default();
        Ok(render_prompt_text_from_records(
            self.records.as_slice(),
            &options,
        ))
    }

    pub async fn render_last_step_text(&mut self) -> Result<Option<String>, AgentToolError> {
        self.ensure_loaded().await?;
        let Some(record) = self.records.last() else {
            return Ok(None);
        };
        let record = enrich_record_new_msgs(record).await;
        Ok(Some(render_single_step(
            &record,
            RenderCompressionLevel::Complete,
            &last_step_render_options(),
        )))
    }

    pub async fn ensure_loaded(&mut self) -> Result<(), AgentToolError> {
        if self.loaded {
            return Ok(());
        }
        self.loaded = true;

        let Some(path) = self.record_file_path() else {
            return Ok(());
        };

        self.records = load_records_from_path(&path).await?;
        Ok(())
    }
}

pub async fn load_records_from_path(path: &Path) -> Result<Vec<LLMStepRecord>, AgentToolError> {
    let payload = match fs::read_to_string(path).await {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(err) => {
            return Err(AgentToolError::ExecFailed(format!(
                "read llm step record file `{}` failed: {err}",
                path.display()
            )))
        }
    };

    let mut loaded = Vec::<LLMStepRecord>::new();
    for (line_no, raw_line) in payload.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() {
            continue;
        }
        match serde_json::from_str::<LLMStepRecord>(line) {
            Ok(record) => loaded.push(record),
            Err(err) => {
                warn!(
                    "step_record.load skip invalid json: path={} line={} err={}",
                    path.display(),
                    line_no + 1,
                    err
                );
            }
        }
    }
    Ok(loaded)
}

pub fn render_prompt_text_from_records(
    records: &[LLMStepRecord],
    options: &LLMStepPromptRenderOptions,
) -> String {
    render_prompt_text_from_records_impl(records, options, None)
}

pub fn render_prompt_text_from_records_with_tokenizer(
    records: &[LLMStepRecord],
    options: &LLMStepPromptRenderOptions,
    tokenizer: &dyn Tokenizer,
) -> String {
    render_prompt_text_from_records_impl(records, options, Some(tokenizer))
}

fn empty_complete_request() -> CompleteRequest {
    CompleteRequest::new(
        Capability::LlmRouter,
        ModelSpec::new(String::new(), None),
        Requirements::new(vec![], None, None, None),
        AiPayload::default(),
        None,
    )
}

fn last_step_render_options() -> LLMStepPromptRenderOptions {
    LLMStepPromptRenderOptions {
        max_render_steps: 1,
        recent_detail_steps: 1,
        max_render_tokens: 0,
        max_render_chars: 0,
        max_conclusion_chars: usize::MAX,
        max_thinking_chars: usize::MAX,
        max_next_action_chars: usize::MAX,
        max_action_result_chars: usize::MAX,
        max_new_msg_chars: usize::MAX,
    }
}

fn render_prompt_text_from_records_impl(
    records: &[LLMStepRecord],
    options: &LLMStepPromptRenderOptions,
    tokenizer: Option<&dyn Tokenizer>,
) -> String {
    if records.is_empty() || options.max_render_steps == 0 {
        return String::new();
    }

    let start = records.len().saturating_sub(options.max_render_steps);
    let visible = &records[start..];
    if visible.is_empty() {
        return String::new();
    }

    let levels = allocate_render_levels(visible, options, tokenizer);
    let blocks = visible
        .iter()
        .zip(levels.iter().copied())
        .map(|(record, level)| render_single_step(record, level, options))
        .collect::<Vec<_>>();

    wrap_steps_summary(blocks.as_slice())
}

fn allocate_render_levels(
    records: &[LLMStepRecord],
    options: &LLMStepPromptRenderOptions,
    tokenizer: Option<&dyn Tokenizer>,
) -> Vec<RenderCompressionLevel> {
    let mut levels = vec![RenderCompressionLevel::Min; records.len()];
    if records.is_empty() {
        return levels;
    }

    let cache = records
        .iter()
        .map(|record| build_render_cache(record, options))
        .collect::<Vec<_>>();
    let budget = render_budget(options, tokenizer);
    let mut total_cost = total_render_cost(cache.as_slice(), levels.as_slice(), tokenizer);

    let seed_levels = [
        RenderCompressionLevel::Full,
        RenderCompressionLevel::Full,
        RenderCompressionLevel::Medium,
        RenderCompressionLevel::Mini,
    ];
    for (offset, target) in seed_levels
        .iter()
        .copied()
        .enumerate()
        .take(options.recent_detail_steps.min(seed_levels.len()))
    {
        let Some(idx) = records.len().checked_sub(offset + 1) else {
            break;
        };
        promote_step_to_target(
            idx,
            target,
            levels.as_mut_slice(),
            cache.as_slice(),
            budget,
            &mut total_cost,
            tokenizer,
        );
    }

    if budget.is_none() {
        return levels;
    }

    loop {
        let mut upgraded = false;
        for idx in (0..records.len()).rev() {
            let Some(next_level) = levels[idx].next_history_level() else {
                continue;
            };
            if try_upgrade_step(
                idx,
                next_level,
                levels.as_mut_slice(),
                cache.as_slice(),
                budget,
                &mut total_cost,
                tokenizer,
            ) {
                upgraded = true;
                break;
            }
        }
        if !upgraded {
            break;
        }
    }

    levels
}

struct StepRenderCache {
    renders: Vec<String>,
}

fn build_render_cache(
    record: &LLMStepRecord,
    options: &LLMStepPromptRenderOptions,
) -> StepRenderCache {
    StepRenderCache {
        renders: vec![
            render_single_step(record, RenderCompressionLevel::Min, options),
            render_single_step(record, RenderCompressionLevel::Mini, options),
            render_single_step(record, RenderCompressionLevel::Medium, options),
            render_single_step(record, RenderCompressionLevel::Full, options),
            render_single_step(record, RenderCompressionLevel::Complete, options),
        ],
    }
}

fn promote_step_to_target(
    idx: usize,
    target: RenderCompressionLevel,
    levels: &mut [RenderCompressionLevel],
    cache: &[StepRenderCache],
    budget: Option<usize>,
    total_cost: &mut usize,
    tokenizer: Option<&dyn Tokenizer>,
) {
    while levels[idx] != target {
        let Some(next_level) = levels[idx].next_history_level() else {
            break;
        };
        if !try_upgrade_step(
            idx, next_level, levels, cache, budget, total_cost, tokenizer,
        ) {
            break;
        }
    }
}

fn try_upgrade_step(
    idx: usize,
    next_level: RenderCompressionLevel,
    levels: &mut [RenderCompressionLevel],
    cache: &[StepRenderCache],
    budget: Option<usize>,
    total_cost: &mut usize,
    tokenizer: Option<&dyn Tokenizer>,
) -> bool {
    let current_level = levels[idx];
    levels[idx] = next_level;
    let next_total = total_render_cost(cache, levels, tokenizer);
    if let Some(budget) = budget {
        if next_total > budget {
            levels[idx] = current_level;
            return false;
        }
    }
    *total_cost = next_total;
    true
}

fn render_budget(
    options: &LLMStepPromptRenderOptions,
    tokenizer: Option<&dyn Tokenizer>,
) -> Option<usize> {
    if tokenizer.is_some() && options.max_render_tokens > 0 {
        Some(options.max_render_tokens as usize)
    } else if options.max_render_chars > 0 {
        Some(options.max_render_chars)
    } else {
        None
    }
}

fn total_render_cost(
    cache: &[StepRenderCache],
    levels: &[RenderCompressionLevel],
    tokenizer: Option<&dyn Tokenizer>,
) -> usize {
    if levels.is_empty() {
        return 0;
    }

    let rendered = assemble_rendered_history(cache, levels);
    match tokenizer {
        Some(tokenizer) => tokenizer.count_tokens(rendered.as_str()) as usize,
        None => rendered.len(),
    }
}

fn assemble_rendered_history(
    cache: &[StepRenderCache],
    levels: &[RenderCompressionLevel],
) -> String {
    let blocks = levels
        .iter()
        .enumerate()
        .map(|(idx, level)| cache[idx].renders[level.history_index()].clone())
        .collect::<Vec<_>>();
    wrap_steps_summary(blocks.as_slice())
}

fn wrap_steps_summary(blocks: &[String]) -> String {
    let blocks = blocks
        .iter()
        .map(|block| block.trim())
        .filter(|block| !block.is_empty())
        .collect::<Vec<_>>();
    if blocks.is_empty() {
        return String::new();
    }
    format!("<steps_summary>\n{}\n</steps_summary>", blocks.join("\n"))
}

fn render_single_step(
    record: &LLMStepRecord,
    level: RenderCompressionLevel,
    options: &LLMStepPromptRenderOptions,
) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "<step behavior=\"{}\" step_num={} step_time=\"{}\">",
        record.behavior_name.trim(),
        record.step_num,
        format_step_timestamp(record.started_at_ms)
    );

    let new_msg = match level {
        RenderCompressionLevel::Complete => {
            render_new_msgs_for_last_step(record.new_msg.as_slice())
        }
        _ => render_new_msgs_for_prompt(record.new_msg.as_slice(), options.max_new_msg_chars),
    };
    if !new_msg.is_empty() {
        let _ = write!(out, "\n{new_msg}");
    }

    let error = truncate_text(
        record.error.as_deref().unwrap_or_default().trim(),
        options.max_next_action_chars,
    );
    if !error.is_empty() {
        let _ = write!(out, "\n<error>{error}</error>");
        out.push_str("\n</step>");
        return out;
    }

    let conclusion = truncate_text(
        record
            .llm_result
            .conclusion
            .as_deref()
            .unwrap_or_default()
            .trim(),
        options.max_conclusion_chars,
    );
    if !conclusion.is_empty() {
        let _ = write!(out, "\n<conclusion>{conclusion}</conclusion>");
    }

    if level.shows_thinking() {
        let thinking = truncate_text(
            record
                .llm_result
                .thinking
                .as_deref()
                .unwrap_or_default()
                .trim(),
            options.max_thinking_chars,
        );
        if !thinking.is_empty() {
            let _ = write!(out, "\n<thinking>{thinking}</thinking>");
        }
    }

    if level.shows_reply() {
        let reply_msg = truncate_text(
            record.llm_result.reply.as_deref().unwrap_or_default(),
            options.max_next_action_chars,
        );
        if !reply_msg.is_empty() {
            let reply_to = infer_reply_target(record);
            if reply_to.is_empty() {
                let _ = write!(out, "\n<reply_msg>{reply_msg}</reply_msg>");
            } else {
                let _ = write!(
                    out,
                    "\n<reply_msg to=\"{reply_to}\">{reply_msg}</reply_msg>"
                );
            }
        }
    }

    let action_result = match level {
        RenderCompressionLevel::Complete => truncate_text(
            render_action_results_for_last_step(&record.action_result).as_str(),
            options.max_action_result_chars,
        ),
        _ => level
            .action_result_level()
            .map(|action_level| {
                truncate_text(
                    render_action_results_for_prompt(&record.action_result, action_level).as_str(),
                    options.max_action_result_chars,
                )
            })
            .unwrap_or_default(),
    };
    if !action_result.is_empty() {
        let _ = write!(out, "\n<action>{action_result}</action>");
    }

    out.push_str("\n</step>");
    out
}

fn render_action_results_for_prompt(
    results: &HashMap<String, AgentToolResult>,
    level: AgentHistoryShowLevel,
) -> String {
    let mut keys = results.keys().cloned().collect::<Vec<_>>();
    keys.sort();

    let mut lines = Vec::<String>::new();

    for key in keys {
        if key.starts_with("__") {
            continue;
        }
        let Some(result) = results.get(&key) else {
            continue;
        };
        let prompt = result.render_for_level(level);
        if prompt.trim().is_empty() {
            continue;
        }
        let mut prompt_lines = prompt.lines();
        if let Some(first) = prompt_lines.next() {
            lines.push(format!("- {}", first));
            for line in prompt_lines {
                lines.push(line.to_string());
            }
        }
    }

    lines.join("\n")
}

fn render_action_results_for_last_step(results: &HashMap<String, AgentToolResult>) -> String {
    let mut keys = results.keys().cloned().collect::<Vec<_>>();
    keys.sort();

    let mut lines = Vec::<String>::new();

    for key in keys {
        if key.starts_with("__") {
            continue;
        }
        let Some(result) = results.get(&key) else {
            continue;
        };
        let prompt = result.render_for_last_step();
        if prompt.trim().is_empty() {
            continue;
        }
        let mut prompt_lines = prompt.lines();
        if let Some(first) = prompt_lines.next() {
            lines.push(format!("- {}", first));
            for line in prompt_lines {
                lines.push(line.to_string());
            }
        }
    }

    lines.join("\n")
}

fn render_new_msgs_for_prompt(messages: &[MsgRecordWithObject], max_chars: usize) -> String {
    let mut rendered = messages
        .iter()
        .take(2)
        .map(|record| render_new_msg_summary(record, max_chars))
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>();
    if messages.len() > 2 {
        rendered.push(format!(
            "<recive_msg from=\"summary\">+{} more</recive_msg>",
            messages.len() - 2
        ));
    }
    rendered.join("\n")
}

fn render_new_msgs_for_last_step(messages: &[MsgRecordWithObject]) -> String {
    messages
        .iter()
        .map(render_new_msg_original)
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_new_msg_summary(record: &MsgRecordWithObject, max_chars: usize) -> String {
    let sender = render_msg_sender(record);
    let content = record
        .msg
        .as_ref()
        .map(|msg| msg.content.content.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| collapse_inline_whitespace(value, max_chars))
        .unwrap_or_else(|| {
            format!(
                "msg_id={} state={:?}",
                record.record.msg_id, record.record.state
            )
        });
    format!("<recive_msg from=\"{sender}\">{content}</recive_msg>")
}

fn render_new_msg_original(record: &MsgRecordWithObject) -> String {
    let sender = render_msg_sender(record);
    let time = format_step_timestamp(resolve_msg_timestamp_ms(record));
    let content = record
        .msg
        .as_ref()
        .map(|msg| msg.content.content.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(normalize_step_msg_multiline_text)
        .unwrap_or_else(|| {
            format!(
                "msg_id={} state={:?}",
                record.record.msg_id, record.record.state
            )
        });
    if content.is_empty() {
        return String::new();
    }
    if content.contains('\n') {
        format!("<recive_msg from=\"{sender}\" time=\"{time}\">\n{content}\n</recive_msg>")
    } else {
        format!("<recive_msg from=\"{sender}\" time=\"{time}\">{content}</recive_msg>")
    }
}

fn render_msg_sender(record: &MsgRecordWithObject) -> String {
    record
        .record
        .from_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .or_else(|| {
            record
                .msg
                .as_ref()
                .map(|msg| msg.from.to_raw_host_name())
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or_else(|| record.record.from.to_string())
}

fn infer_reply_target(record: &LLMStepRecord) -> String {
    record
        .new_msg
        .first()
        .map(render_msg_sender)
        .unwrap_or_default()
}

fn format_step_timestamp(timestamp_ms: u64) -> String {
    let secs = (timestamp_ms / 1000) as i64;
    let nanos = ((timestamp_ms % 1000) * 1_000_000) as u32;
    let dt = DateTime::<Utc>::from_timestamp(secs, nanos).unwrap_or_else(Utc::now);
    dt.format("%Y-%m-%d %H:%M:%S").to_string()
}

fn resolve_msg_timestamp_ms(record: &MsgRecordWithObject) -> u64 {
    let msg_ts = record
        .msg
        .as_ref()
        .map(|msg| msg.created_at_ms)
        .unwrap_or(0);
    record
        .record
        .updated_at_ms
        .max(record.record.created_at_ms)
        .max(msg_ts)
}

fn normalize_step_msg_multiline_text(input: &str) -> String {
    let mut lines = Vec::<String>::new();
    let mut last_blank = false;
    for raw_line in input.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if last_blank {
                continue;
            }
            lines.push(String::new());
            last_blank = true;
            continue;
        }
        lines.push(line.to_string());
        last_blank = false;
    }
    let merged = lines.join("\n");
    if merged.trim().is_empty() {
        String::new()
    } else {
        merged.trim().to_string()
    }
}

async fn enrich_record_new_msgs(record: &LLMStepRecord) -> LLMStepRecord {
    let mut enriched = record.clone();
    for msg in &mut enriched.new_msg {
        if msg.msg.is_some() {
            continue;
        }
        if let Ok(msg_obj) = msg.get_msg().await {
            msg.msg = Some(msg_obj);
        }
    }
    enriched
}

fn collapse_inline_whitespace(input: &str, max_chars: usize) -> String {
    let collapsed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_text(collapsed.as_str(), max_chars)
}

fn truncate_text(input: &str, max_chars: usize) -> String {
    let normalized = input.trim();
    if normalized.is_empty() || max_chars == 0 {
        return String::new();
    }

    let mut chars = normalized.chars();
    let collected = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_none() {
        return collected;
    }

    let trimmed = collected.trim_end();
    if trimmed.is_empty() {
        "...".to_string()
    } else {
        format!("{trimmed}...")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct CountingTokenizer;

    impl Tokenizer for CountingTokenizer {
        fn count_tokens(&self, text: &str) -> u32 {
            text.chars().count() as u32
        }
    }

    fn sample_record(step_num: u32, step_index: u32) -> LLMStepRecord {
        LLMStepRecord {
            session_id: "work-1".to_string(),
            step_num,
            step_index,
            behavior_name: "plan".to_string(),
            started_at_ms: 1,
            llm_completed_at_ms: 2,
            action_completed_at_ms: 3,
            new_msg: vec![],
            input: format!("input-{step_num}"),
            llm_prompt: CompleteRequest::new(
                Capability::LlmRouter,
                ModelSpec::new("llm.default".to_string(), None),
                Requirements::new(vec![], None, None, None),
                AiPayload::new(
                    None,
                    vec![buckyos_api::AiMessage::new(
                        "user".to_string(),
                        format!("prompt-{step_num}"),
                    )],
                    vec![],
                    vec![],
                    None,
                    None,
                ),
                Some(format!("step-{step_num}")),
            ),
            llm_result: BehaviorLLMResult {
                conclusion: Some(format!("conclusion-{step_num}")),
                thinking: Some(format!("thinking-{step_num}")),
                reply: Some(format!("reply-{step_num}")),
                ..Default::default()
            },
            action_result: HashMap::from([(
                "#0".to_string(),
                AgentToolResult::from_details(serde_json::json!({}))
                    .with_cmd_line(format!("tool-{step_num}"))
                    .with_result(format!("action-{step_num}"))
                    .with_output(
                        (1..=9)
                            .map(|line| format!("step-{step_num}-line-{line}"))
                            .collect::<Vec<_>>()
                            .join("\n"),
                    ),
            )]),
            error: None,
        }
    }

    fn sample_new_msg(content: &str) -> MsgRecordWithObject {
        serde_json::from_value(serde_json::json!({
            "record": {
                "record_id": "record-1",
                "box_kind": "INBOX",
                "msg_id": "sha256:00000000000000000000000000000001",
                "msg_kind": "chat",
                "state": "UNREAD",
                "from": "did:web:alice.example.com",
                "from_name": "alice.example.com",
                "to": "did:web:agent.example.com",
                "created_at_ms": 1000,
                "updated_at_ms": 1000,
                "sort_key": 1000,
                "tags": []
            },
            "msg": {
                "from": "did:web:alice.example.com",
                "to": ["did:web:agent.example.com"],
                "kind": "chat",
                "created_at_ms": 1000,
                "content": {
                    "content": content
                }
            }
        }))
        .expect("sample msg should parse")
    }

    #[test]
    fn render_prompt_text_uses_four_level_history_compression() {
        let records = vec![
            sample_record(0, 0),
            sample_record(1, 1),
            sample_record(2, 2),
            sample_record(3, 3),
            sample_record(4, 4),
        ];
        let rendered = render_prompt_text_from_records(
            &records,
            &LLMStepPromptRenderOptions {
                max_render_steps: 5,
                recent_detail_steps: 4,
                ..Default::default()
            },
        );

        println!("rendered: {}", rendered);

        assert!(rendered.contains("<steps_summary>"));
        assert!(rendered.contains("<step behavior=\"plan\" step_num=0 step_time=\""));
        assert!(rendered.contains("<conclusion>conclusion-0</conclusion>"));
        assert!(!rendered.contains("thinking-0"));
        assert!(!rendered.contains("step-0-line-1"));

        assert!(rendered.contains("<step behavior=\"plan\" step_num=1 step_time=\""));
        assert!(rendered.contains("<reply_msg>reply-1</reply_msg>"));
        assert!(!rendered.contains("<action>- tool-1 => success"));
        assert!(!rendered.contains("step-1-line-1"));

        assert!(rendered.contains("<step behavior=\"plan\" step_num=2 step_time=\""));
        assert!(rendered.contains("<reply_msg>reply-2</reply_msg>"));
        assert!(rendered.contains("<action>- tool-2 => success"));
        assert!(!rendered.contains("step-2-line-9"));

        assert!(rendered.contains("<step behavior=\"plan\" step_num=4 step_time=\""));
        assert!(rendered.contains("<reply_msg>reply-4</reply_msg>"));
        assert!(rendered.contains("<action>- tool-4 => success"));
        assert!(rendered.contains("step-4-line-9"));
        assert!(!rendered.contains("thinking-4"));
        assert!(rendered.contains("</steps_summary>"));
    }

    #[test]
    fn wrap_steps_summary_skips_empty_sections() {
        let rendered = wrap_steps_summary(&[String::new(), "   ".to_string()]);

        assert!(rendered.is_empty());
    }

    #[test]
    fn render_prompt_text_respects_render_token_budget() {
        let records = vec![
            sample_record(0, 0),
            sample_record(1, 1),
            sample_record(2, 2),
        ];
        let tokenizer = CountingTokenizer;
        let base_options = LLMStepPromptRenderOptions {
            max_render_steps: 3,
            recent_detail_steps: 4,
            ..Default::default()
        };
        let min_plus_latest_mini = wrap_steps_summary(
            vec![
                render_single_step(&records[0], RenderCompressionLevel::Min, &base_options),
                render_single_step(&records[1], RenderCompressionLevel::Min, &base_options),
                render_single_step(&records[2], RenderCompressionLevel::Mini, &base_options),
            ]
            .as_slice(),
        );
        let rendered = render_prompt_text_from_records_with_tokenizer(
            &records,
            &LLMStepPromptRenderOptions {
                max_render_steps: 3,
                recent_detail_steps: 4,
                max_render_tokens: min_plus_latest_mini.len() as u32,
                ..Default::default()
            },
            &tokenizer,
        );

        assert_eq!(rendered.len(), min_plus_latest_mini.len());
        assert!(rendered.contains("<reply_msg>reply-2</reply_msg>"));
        assert!(!rendered.contains("<action>- tool-2 => success"));
        assert!(!rendered.contains("step-2-line-1"));
    }

    #[test]
    fn render_complete_step_includes_thinking() {
        let rendered = render_single_step(
            &sample_record(9, 9),
            RenderCompressionLevel::Complete,
            &LLMStepPromptRenderOptions::default(),
        );

        assert!(rendered.contains("<reply_msg>reply-9</reply_msg>"));
        assert!(rendered.contains("<thinking>thinking-9</thinking>"));
        assert!(rendered.contains("step-9-line-9"));
    }

    #[test]
    fn render_complete_step_orders_sections_as_conclusion_thinking_reply_action() {
        let rendered = render_single_step(
            &sample_record(10, 10),
            RenderCompressionLevel::Complete,
            &LLMStepPromptRenderOptions::default(),
        );

        let conclusion_idx = rendered
            .find("<conclusion>")
            .expect("conclusion should be rendered");
        let thinking_idx = rendered
            .find("<thinking>")
            .expect("thinking should be rendered");
        let reply_idx = rendered
            .find("<reply_msg>")
            .expect("reply should be rendered");
        let action_idx = rendered
            .find("<action>")
            .expect("action should be rendered");

        assert!(conclusion_idx < thinking_idx);
        assert!(thinking_idx < reply_idx);
        assert!(reply_idx < action_idx);
    }

    #[test]
    fn render_complete_step_keeps_multiline_new_msg_content() {
        let mut record = sample_record(11, 11);
        record.new_msg = vec![sample_new_msg("line 1\n\nline 2\nline 3")];

        let rendered = render_single_step(
            &record,
            RenderCompressionLevel::Complete,
            &last_step_render_options(),
        );

        assert!(rendered.contains(&format!(
            "<recive_msg from=\"alice.example.com\" time=\"{}\">",
            format_step_timestamp(1000)
        )));
        assert!(rendered.contains("line 1\n\nline 2\nline 3"));
        assert!(!rendered.contains("line 1 line 2 line 3"));
    }

    #[test]
    fn render_step_includes_error_when_present() {
        let mut record = sample_record(5, 5);
        record.error = Some("run_step failed: invalid behavior llm xml".to_string());

        let rendered = render_single_step(
            &record,
            RenderCompressionLevel::Complete,
            &LLMStepPromptRenderOptions::default(),
        );

        assert!(rendered.contains("<error>run_step failed: invalid behavior llm xml</error>"));
        assert!(!rendered.contains("<conclusion>"));
        assert!(!rendered.contains("<reply_msg>"));
        assert!(!rendered.contains("<action>"));
        assert!(!rendered.contains("<thinking>"));
    }

    #[tokio::test]
    async fn append_persists_and_reload_works() {
        let root = tempfile::tempdir().expect("create tempdir");
        let mut log = LLMStepRecordLog::default();
        log.bind_session("work-1", root.path());
        log.append(sample_record(7, 3))
            .await
            .expect("append record");

        let mut reloaded = LLMStepRecordLog::default();
        reloaded.bind_session("work-1", root.path());
        let rendered = reloaded
            .render_prompt_text(None)
            .await
            .expect("render from file");

        assert!(rendered.contains("<step behavior=\"plan\" step_num=7 step_time=\""));
        assert!(reloaded.record_file_path().expect("record path").is_file());
    }

    #[tokio::test]
    async fn render_last_step_text_keeps_full_action_output() {
        let root = tempfile::tempdir().expect("create tempdir");
        let mut log = LLMStepRecordLog::default();
        log.bind_session("work-1", root.path());

        let mut record = sample_record(8, 8);
        let output = (1..=40)
            .map(|line| format!("step-8-line-{line}"))
            .collect::<Vec<_>>()
            .join("\n");
        record.action_result = HashMap::from([(
            "#0".to_string(),
            AgentToolResult::from_details(serde_json::json!({}))
                .with_cmd_line("sed -n '1,40p' build.log")
                .with_output(output),
        )]);

        log.append(record).await.expect("append record");

        let rendered = log
            .render_last_step_text()
            .await
            .expect("render last step")
            .expect("last step text");

        assert!(rendered.contains("step-8-line-1"));
        assert!(rendered.contains("step-8-line-40"));
        assert!(!rendered.contains("TRUNCATED"));
    }
}
