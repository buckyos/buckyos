use std::collections::HashMap;
use std::sync::Arc;

use buckyos_api::CompleteRequest;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio::sync::Mutex;
use xmltree::{Element, XMLNode};

use crate::agent_session::AgentSession;
use crate::agent_tool::{ActionCall, DoAction, DoActions};
use crate::behavior::{BehaviorConfig, LLMComputeError};

pub type InboxPack = Json;
pub type MemoryPack = Json;

pub use ::agent_tool::SessionRuntimeContext;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvKV {
    pub key: String,
    pub value: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct StepLimits {
    pub max_prompt_tokens: u32,
    pub max_completion_tokens: u32,
    pub max_tool_rounds: u8,
    pub max_tool_calls_per_round: u16,
    pub max_observation_bytes: usize,
    pub deadline_ms: u64,
}

impl Default for StepLimits {
    fn default() -> Self {
        Self {
            max_prompt_tokens: 200_000,
            max_completion_tokens: 200_000,
            max_tool_rounds: 1,
            max_tool_calls_per_round: 8,
            max_observation_bytes: 32 * 1024,
            deadline_ms: 30_000,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BehaviorExecInput {
    pub session_id: String,
    pub trace: SessionRuntimeContext,

    pub input_prompt: String,
    pub last_step_prompt: String,
    pub role_md: String,
    pub self_md: String,
    pub behavior_prompt: String,
    pub limits: StepLimits,
    pub behavior_cfg: BehaviorConfig,
    /// Session for template rendering ({{key}} from session values).
    #[serde(skip)]
    pub session: Option<Arc<Mutex<AgentSession>>>,
}

impl std::fmt::Debug for BehaviorExecInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BehaviorExecInput")
            .field("session_id", &self.session_id)
            .field("trace", &self.trace)
            .field("input_prompt", &self.input_prompt)
            .field("last_step_prompt", &self.last_step_prompt)
            .field("role_md", &self.role_md)
            .field("self_md", &self.self_md)
            .field("behavior_prompt", &self.behavior_prompt)
            .field("limits", &self.limits)
            .field("behavior_cfg", &self.behavior_cfg)
            .field("session", &self.session.as_ref().map(|_| "Some(_)"))
            .finish()
    }
}

impl PartialEq for BehaviorExecInput {
    fn eq(&self, other: &Self) -> bool {
        self.session_id == other.session_id
            && self.trace == other.trace
            && self.input_prompt == other.input_prompt
            && self.last_step_prompt == other.last_step_prompt
            && self.role_md == other.role_md
            && self.self_md == other.self_md
            && self.behavior_prompt == other.behavior_prompt
            && self.limits == other.limits
            && self.behavior_cfg == other.behavior_cfg
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TokenUsage {
    pub prompt: u32,
    pub completion: u32,
    pub total: u32,
}

impl TokenUsage {
    pub fn add(self, other: TokenUsage) -> TokenUsage {
        TokenUsage {
            prompt: self.prompt.saturating_add(other.prompt),
            completion: self.completion.saturating_add(other.completion),
            total: self.total.saturating_add(other.total),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum LLMOutput {
    Json(Json),
    Text(String),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct ExecutorReply {
    pub audience: String,
    pub format: String,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct BehaviorLLMResult {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conclusion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub topic_tags: Vec<String>,

    //---下面的字段都是next_action-----
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
    #[serde(default, skip_serializing_if = "DoActions::is_empty")]
    pub actions: DoActions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shell_commands: Vec<String>,

    // #[serde(default, skip_serializing_if = "Vec::is_empty")]
    // pub todo: Vec<Json>,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub set_memory: HashMap<String, String>,
    //#[serde(default, skip_serializing_if = "Vec::is_empty")]
    //pub load_skills: Vec<String>,
    //#[serde(default, skip_serializing_if = "Vec::is_empty")]
    //pub enable_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new_session: Option<(String, String)>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_behavior: Option<String>,
}
impl BehaviorLLMResult {
    pub fn is_sleep(&self) -> bool {
        self.next_behavior.as_deref() == Some("END")
    }

    pub fn from_xml_str(input: &str) -> Result<Self, LLMComputeError> {
        let normalized = input.trim();
        let direct_err = match parse_behavior_llm_result_xml(normalized) {
            Ok(parsed) => return Ok(parsed),
            Err(err) => err,
        };

        if let Some(xml_block) = extract_xml_from_markdown_fence(normalized) {
            if let Ok(parsed) = parse_behavior_llm_result_xml(xml_block.as_str()) {
                return Ok(parsed);
            }
        }

        if let Some(xml_doc) = extract_embedded_xml(normalized) {
            if xml_doc != normalized {
                if let Ok(parsed) = parse_behavior_llm_result_xml(xml_doc.as_str()) {
                    return Ok(parsed);
                }
            }
        }

        let err = format!("invalid behavior llm xml: {direct_err}");
        warn!("failed to parse BehaviorLLMResult output: {}", err);
        Err(LLMComputeError::Internal(err))
    }
}

fn parse_behavior_llm_result_xml(input: &str) -> Result<BehaviorLLMResult, String> {
    let root = Element::parse(input.as_bytes()).map_err(|err| err.to_string())?;
    if !root.name.eq_ignore_ascii_case("response") {
        return Err("root tag must be <response>".to_string());
    }
    let mut result = BehaviorLLMResult::default();

    result.next_behavior = child_text(&root, "next_behavior");
    result.thinking = child_text(&root, "thinking");
    result.reply = child_text(&root, "reply");
    result.route_session_id = child_text(&root, "route_session_id");
    result.new_session = parse_new_session(&root)?;
    result.topic_tags = parse_text_list(&root, "topic_tags", &["tag"]);
    result.shell_commands = parse_shell_commands(&root);
    result.set_memory = parse_set_memory(&root)?;
    if let Some(actions_node) = first_child_named(&root, "actions") {
        result.actions = parse_actions(actions_node)?;
    }

    Ok(result)
}

fn extract_xml_from_markdown_fence(input: &str) -> Option<String> {
    if !input.contains("```") {
        return None;
    }
    let mut parts = input.split("```");
    let _ = parts.next();
    while let Some(block) = parts.next() {
        let mut candidate = block.trim();
        if candidate.is_empty() {
            continue;
        }

        if let Some(stripped) = candidate.strip_prefix("xml") {
            candidate = stripped.trim_start();
        } else if let Some(stripped) = candidate.strip_prefix("XML") {
            candidate = stripped.trim_start();
        }

        if candidate.starts_with('<') {
            return Some(candidate.to_string());
        }
    }
    None
}

fn extract_embedded_xml(input: &str) -> Option<String> {
    let start = input.find('<')?;
    let end = input.rfind('>')?;
    if end < start {
        return None;
    }
    let candidate = input[start..=end].trim();
    if candidate.is_empty() {
        return None;
    }
    Some(candidate.to_string())
}

fn first_child_named<'a>(element: &'a Element, name: &str) -> Option<&'a Element> {
    element.children.iter().find_map(|node| match node {
        XMLNode::Element(child) if child.name.eq_ignore_ascii_case(name) => Some(child),
        _ => None,
    })
}

fn child_text(element: &Element, name: &str) -> Option<String> {
    first_child_named(element, name).and_then(element_text)
}

fn parse_new_session(root: &Element) -> Result<Option<(String, String)>, String> {
    let Some(node) = first_child_named(root, "new_session") else {
        return Ok(None);
    };
    let title = child_text(node, "title");
    let summary = child_text(node, "summary");
    match (title, summary) {
        (Some(title), Some(summary)) => Ok(Some((title, summary))),
        (None, None) => Ok(None),
        _ => Err("new_session requires both <title> and <summary>".to_string()),
    }
}

fn parse_text_list(root: &Element, field_name: &str, item_names: &[&str]) -> Vec<String> {
    let Some(list_node) = first_child_named(root, field_name) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for node in &list_node.children {
        let XMLNode::Element(item) = node else {
            continue;
        };
        if !item_names
            .iter()
            .any(|name| item.name.eq_ignore_ascii_case(name))
        {
            continue;
        }
        if let Some(value) = element_text(item) {
            out.push(value);
        }
    }

    if out.is_empty() {
        if let Some(value) = element_direct_text(list_node) {
            out.push(value);
        }
    }
    out
}

fn parse_shell_commands(root: &Element) -> Vec<String> {
    let Some(shell_node) = first_child_named(root, "shell_commands") else {
        return Vec::new();
    };

    element_direct_text(shell_node)
        .map(|block| split_shell_command_lines(block.as_str()))
        .unwrap_or_default()
}

fn split_shell_command_lines(raw: &str) -> Vec<String> {
    let mut commands = Vec::new();
    let mut current = Vec::new();
    let mut heredoc = None;

    for raw_line in raw.lines() {
        let line = raw_line.trim_end_matches('\r');
        if let Some(state) = heredoc.as_ref() {
            let normalized = normalize_heredoc_line(line, state);
            if normalized.trim() == state.delimiter {
                current.push(state.delimiter.clone());
                commands.push(current.join("\n"));
                current.clear();
                heredoc = None;
            } else {
                current.push(normalized);
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        current.push(trimmed.to_string());
        if let Some(state) = parse_heredoc_state(line) {
            heredoc = Some(state);
        } else {
            commands.push(current.join("\n"));
            current.clear();
        }
    }

    if !current.is_empty() {
        commands.push(current.join("\n"));
    }

    commands
}

#[derive(Clone, Debug)]
struct HeredocState {
    delimiter: String,
    strip_tabs: bool,
    indent_prefix: String,
}

fn parse_heredoc_state(line: &str) -> Option<HeredocState> {
    let trimmed = line.trim();
    for (idx, _) in trimmed.match_indices("<<") {
        let mut rest = &trimmed[idx + 2..];
        if rest.starts_with('<') {
            continue;
        }

        let strip_tabs = rest.starts_with('-');
        if strip_tabs {
            rest = &rest[1..];
        }

        let rest = rest.trim_start();
        if rest.is_empty() {
            continue;
        }

        let delimiter = parse_heredoc_delimiter(rest)?;
        return Some(HeredocState {
            delimiter,
            strip_tabs,
            indent_prefix: leading_whitespace_prefix(line).to_string(),
        });
    }
    None
}

fn parse_heredoc_delimiter(raw: &str) -> Option<String> {
    let mut chars = raw.chars();
    let first = chars.next()?;
    if first == '\'' || first == '"' {
        let end = raw[1..].find(first)?;
        let delimiter = &raw[1..1 + end];
        if delimiter.is_empty() {
            return None;
        }
        return Some(delimiter.to_string());
    }

    let delimiter = raw.split_whitespace().next()?.trim();
    if delimiter.is_empty() {
        None
    } else {
        Some(delimiter.to_string())
    }
}

fn leading_whitespace_prefix(line: &str) -> &str {
    let indent_len = line
        .find(|ch: char| !ch.is_whitespace())
        .unwrap_or(line.len());
    &line[..indent_len]
}

fn normalize_heredoc_line(line: &str, state: &HeredocState) -> String {
    let mut normalized = if line.starts_with(state.indent_prefix.as_str()) {
        &line[state.indent_prefix.len()..]
    } else {
        line
    };

    if state.strip_tabs {
        normalized = normalized.trim_start_matches('\t');
    }

    normalized.to_string()
}

fn parse_set_memory(root: &Element) -> Result<HashMap<String, String>, String> {
    let mut out = HashMap::new();
    let Some(set_memory_node) = first_child_named(root, "set_memory") else {
        return Ok(out);
    };

    for node in &set_memory_node.children {
        let XMLNode::Element(item) = node else {
            continue;
        };
        if !item.name.eq_ignore_ascii_case("item") {
            continue;
        }

        let key = item
            .attributes
            .get("key")
            .cloned()
            .ok_or_else(|| "set_memory item missing key".to_string())?;

        let value = element_direct_text(item).unwrap_or_default();

        out.insert(key, value);
    }
    Ok(out)
}

fn parse_actions(actions_node: &Element) -> Result<DoActions, String> {
    let mut actions = DoActions::default();
    if let Some(mode) = actions_node
        .attributes
        .get("mode")
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
    {
        actions.mode = mode.to_string();
    }

    for node in &actions_node.children {
        let XMLNode::Element(child) = node else {
            continue;
        };
        if child.name.eq_ignore_ascii_case("command") {
            if let Some(command) = element_text(child) {
                actions.cmds.push(DoAction::Exec(command));
            }
            continue;
        }
        if child.name.eq_ignore_ascii_case("exec") {
            actions
                .cmds
                .push(DoAction::Call(parse_action_exec_as_call(child)?));
            continue;
        }
        return Err(format!(
            "unsupported actions child tag `<{}>`, only <command> and <exec> are allowed",
            child.name
        ));
    }

    Ok(actions)
}

fn parse_action_exec_as_call(exec_node: &Element) -> Result<ActionCall, String> {
    let action_name = exec_node
        .attributes
        .get("name")
        .cloned()
        .or_else(|| child_text(exec_node, "name"))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "actions.exec missing name".to_string())?;

    let mut params = serde_json::Map::new();
    for (key, value) in &exec_node.attributes {
        if key.eq_ignore_ascii_case("name") {
            continue;
        }
        params.insert(key.to_string(), parse_xml_value(value));
    }

    if let Some(body) = element_direct_text(exec_node).filter(|value| !value.trim().is_empty()) {
        if action_name.eq_ignore_ascii_case("edit_file") {
            if !params.contains_key("new_content") {
                params.insert("new_content".to_string(), Json::String(body));
            }
        } else if !params.contains_key("content") {
            params.insert("content".to_string(), Json::String(body));
        }
    }

    Ok(ActionCall {
        call_action_name: action_name,
        call_params: Json::Object(params),
    })
}

fn parse_xml_value(raw: &str) -> Json {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Json::String(String::new());
    }
    if looks_like_json_literal(trimmed) {
        if let Ok(value) = serde_json::from_str::<Json>(trimmed) {
            return value;
        }
    }
    Json::String(trimmed.to_string())
}

fn looks_like_json_literal(raw: &str) -> bool {
    raw.starts_with('{')
        || raw.starts_with('[')
        || raw.starts_with('"')
        || raw.eq_ignore_ascii_case("true")
        || raw.eq_ignore_ascii_case("false")
        || raw.eq_ignore_ascii_case("null")
        || raw.parse::<f64>().is_ok()
}

fn element_text(element: &Element) -> Option<String> {
    let mut out = String::new();
    collect_text_recursive(element, &mut out);
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn element_direct_text(element: &Element) -> Option<String> {
    let mut out = String::new();
    for node in &element.children {
        match node {
            XMLNode::Text(value) | XMLNode::CData(value) => out.push_str(value),
            _ => {}
        }
    }
    let trimmed = out.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn collect_text_recursive(element: &Element, out: &mut String) {
    for node in &element.children {
        match node {
            XMLNode::Text(value) | XMLNode::CData(value) => out.push_str(value),
            XMLNode::Element(child) => collect_text_recursive(child, out),
            _ => {}
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ObservationSource {
    Tool,
    Action,
    System,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Observation {
    pub source: ObservationSource,
    pub name: String,
    pub content: Json,
    pub ok: bool,
    pub truncated: bool,
    pub bytes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolExecRecord {
    pub tool_name: String,
    pub call_id: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackInfo {
    pub trace_id: String,
    pub model: String,
    pub provider: String,
    pub latency_ms: u64,
    pub llm_task_ids: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LLMTrackingInfo {
    pub token_usage: TokenUsage,
    pub track: TrackInfo,
    pub tool_trace: Vec<ToolExecRecord>,
    pub raw_output: LLMOutput,
    pub prompt_request: CompleteRequest,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LLMBehaviorConfig {
    pub process_name: String,
    pub model_policy: ModelPolicy,
    pub response_schema: Option<Json>,
    pub force_json: bool,
    pub output_mode: String,
    pub output_protocol: String,
}

impl Default for LLMBehaviorConfig {
    fn default() -> Self {
        Self {
            process_name: "opendan-llm-behavior".to_string(),
            model_policy: ModelPolicy::default(),
            response_schema: None,
            force_json: true,
            output_mode: "auto".to_string(),
            output_protocol: String::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ModelPolicy {
    pub preferred: String,
    pub fallback: Vec<String>,
    pub temperature: f32,
}

impl Default for ModelPolicy {
    fn default() -> Self {
        Self {
            preferred: "llm.default".to_string(),
            fallback: vec![],
            temperature: 0.2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_tool::DoAction;

    #[test]
    fn behavior_result_accepts_basic_xml() {
        let raw = r#"<response>
  <actions mode="all"></actions>
  <next_behavior>DO:1</next_behavior>
  <thinking>done</thinking>
</response>"#;
        let parsed = BehaviorLLMResult::from_xml_str(raw).expect("xml should be parsed");

        assert_eq!(parsed.actions.mode, "all");
        assert_eq!(parsed.actions.cmds.len(), 0);
        assert_eq!(parsed.next_behavior.as_deref(), Some("DO:1"));
        assert_eq!(parsed.thinking.as_deref(), Some("done"));
    }

    #[test]
    fn behavior_result_accepts_markdown_wrapped_xml() {
        let raw = r#"```xml
<response>
  <next_behavior>END</next_behavior>
  <thinking>ok</thinking>
</response>
```"#;
        let parsed = BehaviorLLMResult::from_xml_str(raw).expect("markdown wrapped xml");
        assert_eq!(parsed.next_behavior.as_deref(), Some("END"));
        assert_eq!(parsed.thinking.as_deref(), Some("ok"));
    }

    #[test]
    fn behavior_result_accepts_embedded_xml_payload() {
        let raw = r#"model output:
<response>
  <thinking>switch</thinking>
  <next_behavior>DO:todo=T01</next_behavior>
</response>
end"#;
        let parsed = BehaviorLLMResult::from_xml_str(raw).expect("embedded xml should parse");
        assert_eq!(parsed.next_behavior.as_deref(), Some("DO:todo=T01"));
        assert_eq!(parsed.thinking.as_deref(), Some("switch"));
    }

    #[test]
    fn behavior_result_accepts_shell_commands_and_structured_actions() {
        let raw = r#"<response>
  <thinking>Need to inspect scripts and then patch index.html.</thinking>
  <reply>先执行检查命令，再执行结构化编辑动作。</reply>
  <shell_commands>
    <![CDATA[
      sed -n '1,240p' workspaces/llk-frontend-game-1772767149633-0/index.html
      sed -n '240,520p' workspaces/llk-frontend-game-1772767149633-0/index.html
    ]]>
  </shell_commands>
  <actions mode="all">
    <command>echo hello</command>
    <exec name="write_file" path="workspaces/llk-frontend-game-1772767149633-0/index.html" mode="write">
      <![CDATA[
        hello from cdata
      ]]>
    </exec>
  </actions>
</response>"#;
        let parsed =
            BehaviorLLMResult::from_xml_str(raw).expect("shell_commands + structured xml actions");
        assert_eq!(parsed.shell_commands.len(), 2);
        assert_eq!(parsed.actions.mode, "all");
        assert_eq!(parsed.actions.cmds.len(), 2);
        match &parsed.actions.cmds[1] {
            DoAction::Call(call) => {
                assert_eq!(call.call_action_name, "write_file");
                assert_eq!(
                    call.call_params
                        .get("path")
                        .and_then(Json::as_str)
                        .unwrap_or_default(),
                    "workspaces/llk-frontend-game-1772767149633-0/index.html"
                );
                assert_eq!(
                    call.call_params
                        .get("content")
                        .and_then(Json::as_str)
                        .unwrap_or_default(),
                    "hello from cdata"
                );
            }
            other => panic!("expected structured call action, got {other:?}"),
        }
    }

    #[test]
    fn behavior_result_accepts_shell_commands_with_heredoc_payload() {
        let raw = r#"<response>
  <thinking>Need to perform a minimal self-check command since claiming completion; will run a small python validation to ensure key files/strings exist, then record result note.</thinking>
  <reply>Running a quick self-check to validate required files/keywords exist and documenting the result for traceability.</reply>
  <shell_commands>
<![CDATA[
        cd /opt/buckyos/data/home/devtest/.local/share/jarvis/workspaces/ws-19d12b01a2b-1352f
        python - <<'PY'
from pathlib import Path
req = ["index.html","script.js","style.css"]
for f in req:
    assert Path(f).exists(), f"missing {f}"
data = Path("script.js").read_text(encoding="utf-8")
for key in ["canConnect","generateBoard","renderBoard","restartBtn"]:
    assert key in data, f"missing {key}"
print("self-check ok")
PY
]]>
  </shell_commands>
  <actions mode="all">
    <command>todo note T001 "Self-check: basic file/keyword validation passed (python asserts for index.html/script.js/style.css, canConnect/generateBoard/renderBoard/restartBtn)." --kind=result</command>
  </actions>
</response>"#;

        let parsed = BehaviorLLMResult::from_xml_str(raw).expect("heredoc shell_commands xml");
        assert_eq!(parsed.shell_commands.len(), 2);
        assert_eq!(
            parsed.shell_commands[0],
            "cd /opt/buckyos/data/home/devtest/.local/share/jarvis/workspaces/ws-19d12b01a2b-1352f"
        );
        assert_eq!(
            parsed.shell_commands[1],
            r#"python - <<'PY'
from pathlib import Path
req = ["index.html","script.js","style.css"]
for f in req:
    assert Path(f).exists(), f"missing {f}"
data = Path("script.js").read_text(encoding="utf-8")
for key in ["canConnect","generateBoard","renderBoard","restartBtn"]:
    assert key in data, f"missing {key}"
print("self-check ok")
PY"#
        );
        assert_eq!(parsed.actions.mode, "all");
        assert_eq!(parsed.actions.cmds.len(), 1);
        match &parsed.actions.cmds[0] {
            DoAction::Exec(cmd) => assert!(cmd.contains("todo note T001")),
            other => panic!("expected exec action, got {other:?}"),
        }
    }

    #[test]
    fn behavior_result_accepts_shell_commands_with_heredoc_variants() {
        let raw = "<response>\r\n  <shell_commands><![CDATA[\r\n\tpython - <<-'PY'\r\n\tprint('line-1')\r\n\t\r\n\tprint('line-2')\r\n\tPY\r\n\techo done\r\n  ]]></shell_commands>\r\n</response>";

        let parsed = BehaviorLLMResult::from_xml_str(raw).expect("heredoc variants should parse");
        assert_eq!(parsed.shell_commands.len(), 2);
        assert_eq!(
            parsed.shell_commands[0],
            "python - <<-'PY'\nprint('line-1')\n\nprint('line-2')\nPY"
        );
        assert_eq!(parsed.shell_commands[1], "echo done");
    }
}
