use std::collections::HashMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use buckyos_api::{
    AiPayload, Capability, CompleteRequest, ModelSpec, MsgRecordWithObject, Requirements,
};
use log::warn;
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;

use crate::agent_tool::{AgentHistoryShowLevel, AgentToolError, AgentToolResult, DoAction};
use crate::behavior::BehaviorLLMResult;

const DEFAULT_STEP_RECORD_FILE: &str = "llm_step_record.jsonl";

/*
Step Record压缩渲染的思路

最小：只显示Step.tilte + Step.new_msg/new_event +Step.conclusion
mini: 显示Step.tilte + Step.new_msg/new_event +Step.conclusion + Step.next_action 最小压缩
中等: 显示Step.tilte + Step.new_msg/new_event + Step.conclusion + Step.next_action 中等压缩
大： 显示Step.title + Step.new_msg/new_event + step.conclusion + step.next_action 略微损失压缩（包含完整错误输出）
完整: 显示Step.title + Step.new_msg/new_event + step.conclusion + step.thinking + step.action 不压缩


自动决定每个step的压缩级别的思路：
目的：能显示所有的step，越后的step越完成


算法：首先有1个完整的+1个大+1个中等+一个mini+剩下的都是最小的进行首次填充
根据还剩多少token（剩余总数)，逐步膨胀一个最近的step，直到达到预算
如果剩余总数>预算的50%，则增加一个完整的step
如果剩余总数>预算的40%，则增加一个大的step
如果剩余总数>预算的30%,增增加一个中等step
如果剩余总数>预算的20%,增增加一个mini step

agent_tool的action_result的压缩级别（从高到底)
- cmd_name (此时已经包含了最小参数) => result (成功/失败/pending+基本原因) 
- summary (已经在构造的时候包含了所有信息)
- summary (已经在构造的时候包含了所有信息)
- cmd_name + details（如有)

注意: Reply是一个标准action_result

一般的bash_exec会得到非标准action_result
非标准action_result的压缩级别（从高到底)也分 4个级别

- cmd_name 部分参数 => result (成功/失败/pending+基本原因)
- cmd_name 部分参数 => result (成功/失败/pending+基本原因) 
  如果失败的话，会显示output的最后n行
- cmd完整显示  => result (成功/失败/pending+基本原因) 
  如果失败的话，会显示output的最后n行
  如果成功，会显示output的前n行
- cmd完整显示  => result (成功/失败/pending+基本原因) 
  如果失败的话，会显示output的最后K行
  如果成功，会显示output的头部K行，K是多少取决于content limit
   


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
    Full,
    Summary,
    ConclusionOnly,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LLMStepPromptRenderOptions {
    pub max_render_steps: usize,
    pub recent_detail_steps: usize,
    pub max_conclusion_chars: usize,
    pub max_thinking_chars: usize,
    pub max_next_action_chars: usize,
    pub max_action_result_chars: usize,
}

impl Default for LLMStepPromptRenderOptions {
    fn default() -> Self {
        Self {
            max_render_steps: 24,
            recent_detail_steps: 4,
            max_conclusion_chars: 280,
            max_thinking_chars: 400,
            max_next_action_chars: 280,
            max_action_result_chars: 1200,
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
        Ok(self.records.last().map(|record| {
            render_single_step(
                record,
                RenderCompressionLevel::Full,
                &LLMStepPromptRenderOptions::default(),
            )
        }))
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
    render_prompt_text_from_records_impl(records, options)
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

fn render_prompt_text_from_records_impl(
    records: &[LLMStepRecord],
    options: &LLMStepPromptRenderOptions,
) -> String {
    if records.is_empty() || options.max_render_steps == 0 {
        return String::new();
    }

    let start = records.len().saturating_sub(options.max_render_steps);
    let visible = &records[start..];
    if visible.is_empty() {
        return String::new();
    }

    let mut blocks = Vec::<String>::new();
    let detail_cutoff = visible
        .len()
        .saturating_sub(options.recent_detail_steps.saturating_add(1));

    for (idx, record) in visible.iter().enumerate() {
        let level = if idx + 1 == visible.len() {
            RenderCompressionLevel::Full
        } else if idx >= detail_cutoff {
            RenderCompressionLevel::Summary
        } else {
            RenderCompressionLevel::ConclusionOnly
        };
        blocks.push(render_single_step(record, level, options));
    }

    format!("## Step Records\n{}", blocks.join("\n\n"))
}

fn render_single_step(
    record: &LLMStepRecord,
    level: RenderCompressionLevel,
    options: &LLMStepPromptRenderOptions,
) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        "### Step {} [{}]",
        record.step_num,
        record.behavior_step_label()
    );

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
        let _ = write!(out, "\n- conclusion: {conclusion}");
    }

    let action_result = if matches!(level, RenderCompressionLevel::Full) {
        truncate_text(
            render_action_results_for_prompt(&record.action_result, AgentHistoryShowLevel::Full)
                .as_str(),
            options.max_action_result_chars,
        )
    } else {
        String::new()
    };

    if matches!(
        level,
        RenderCompressionLevel::Summary | RenderCompressionLevel::Full
    ) && action_result.is_empty()
    {
        let next_action = truncate_text(
            render_llm_next_action(&record.llm_result).as_str(),
            options.max_next_action_chars,
        );
        if !next_action.is_empty() {
            let _ = write!(out, "\n- next_action: {next_action}");
        }
    }

    if matches!(level, RenderCompressionLevel::Full) {
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
            let _ = write!(out, "\n- thinking: {thinking}");
        }
        if !action_result.is_empty() {
            let _ = write!(out, "\n- action:\n```text\n{action_result}\n```");
        }
    }

    out
}

fn render_llm_next_action(llm_result: &BehaviorLLMResult) -> String {
    let mut lines = Vec::<String>::new();

    if let Some(reply) = llm_result
        .reply
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("reply: {reply}"));
    }

    for command in &llm_result.shell_commands {
        let command = command.trim();
        if !command.is_empty() {
            lines.push(format!("exec: {command}"));
        }
    }

    for action in &llm_result.actions.cmds {
        match action {
            DoAction::Exec(command) => {
                let command = command.trim();
                if !command.is_empty() {
                    lines.push(format!("exec: {command}"));
                }
            }
            DoAction::Call(call) => {
                let action_name = call.call_action_name.trim();
                if action_name.is_empty() {
                    continue;
                }
                let params =
                    serde_json::to_string(&call.call_params).unwrap_or_else(|_| "{}".to_string());
                lines.push(format!("call: {action_name} {params}"));
            }
        }
    }

    if let Some(route_session_id) = llm_result
        .route_session_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("route_session_id: {route_session_id}"));
    }

    if let Some((session_title, behavior_name)) = llm_result.new_session.as_ref() {
        let session_title = session_title.trim();
        let behavior_name = behavior_name.trim();
        if !session_title.is_empty() || !behavior_name.is_empty() {
            lines.push(format!(
                "new_session: title={} behavior={}",
                session_title, behavior_name
            ));
        }
    }

    if let Some(next_behavior) = llm_result
        .next_behavior
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        lines.push(format!("next_behavior: {next_behavior}"));
    }

    lines.join("\n")
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
                shell_commands: vec![format!("next-{step_num}")],
                ..Default::default()
            },
            action_result: HashMap::from([(
                "#0".to_string(),
                AgentToolResult::from_details(serde_json::json!({}))
                    .with_cmd_line(format!("tool-{step_num}"))
                    .with_result(format!("action-{step_num}")),
            )]),
        }
    }

    #[test]
    fn render_prompt_text_uses_three_level_compression() {
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
                recent_detail_steps: 2,
                ..Default::default()
            },
        );

        println!("rendered: {}", rendered);

        assert!(rendered.contains("### Step 0 [plan:0]"));
        assert!(rendered.contains("- conclusion: conclusion-0"));
        assert!(!rendered.contains("thinking-0"));
        assert!(!rendered.contains("action-0"));

        assert!(rendered.contains("### Step 2 [plan:2]"));
        assert!(rendered.contains("- next_action: exec: next-2"));
        assert!(!rendered.contains("thinking-2"));

        assert!(rendered.contains("### Step 4 [plan:4]"));
        assert!(!rendered.contains("- next_action: exec: next-4"));
        assert!(rendered.contains("- thinking: thinking-4"));
        assert!(rendered.contains("- action:\n```text\n- tool-4 => action-4"));
        assert!(rendered.contains("- tool-4 => action-4"));
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

        assert!(rendered.contains("### Step 7 [plan:3]"));
        assert!(reloaded.record_file_path().expect("record path").is_file());
    }
}
