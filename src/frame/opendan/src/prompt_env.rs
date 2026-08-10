//! Phase-1 integration of `llm_context::PromptRenderEngine` into AgentSession
//! egress. See `doc/opendan/Agent Enviroment.md` §15.1 for the variable
//! contract.
//!
//! Surfaces the minimal Phase-1 variable set (session / behavior / workspace
//! / paths / input / runtime / result_protocol) as both static `RenderVars.vars` (for upon
//! `{{ session.id }}` placeholders) and a `ValueLoader` (for explicit
//! `__VAR(name, $session)__` / `__ENV($session.id)__` lookups). Aggregate
//! objects carry sibling `has_*` booleans so templates can branch on
//! presence without relying on engine-specific string-truthy semantics.
//!
//! All OpenDAN behavior templates use the engine's upon syntax — the
//! single-brace `{name}` form is no longer supported.

use std::path::PathBuf;

use async_trait::async_trait;
use buckyos_api::{AiContent, AiMessage, ResourceRef};
use chrono::{Local, TimeZone};
use llm_context::{
    behavior_loop::StepRecord, EngineConfig, PromptRenderEngine, RenderError, RenderVars,
    ValueLoader, XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT,
};
use serde_json::{json, Value as Json};

use crate::session_model::{
    AgentTaskBinding, BackgroundHint, BgEventSnapshot, EventRef, PendingInput, SessionKind,
};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
#[serde(default)]
pub struct LlmContextEnv {
    pub msgs: Vec<Json>,
    pub events: Vec<EventRef>,
    pub bg_events: Vec<BgEventSnapshot>,
    pub background_hints: Vec<BackgroundHint>,
    pub default_changed_background_hint_text: String,
    pub last_step: Option<Json>,
    pub last_report: Option<String>,
    pub behavior_history: Vec<Json>,
    pub agent_global_state: Json,
}

impl Default for LlmContextEnv {
    fn default() -> Self {
        Self {
            msgs: Vec::new(),
            events: Vec::new(),
            bg_events: Vec::new(),
            background_hints: Vec::new(),
            default_changed_background_hint_text: String::new(),
            last_step: None,
            last_report: None,
            behavior_history: Vec::new(),
            agent_global_state: Json::Null,
        }
    }
}

/// Phase-1 snapshot of the variables the loader / `RenderVars` can serve.
/// Built once per turn at the egress boundary so the value set is stable
/// for the whole render even if `meta` mutates under a concurrent inbound.
///
/// String fields that drive presence checks (`session_title`,
/// `recent_activity`) are stored already-trimmed so the matching
/// `has_*` booleans are stable across behavior-template renders.
#[derive(Debug, Clone)]
pub struct AgentSessionEnv {
    pub session_id: String,
    pub session_kind: &'static str,
    pub session_title: String,
    pub session_objective: String,
    pub session_owner: String,
    pub session_current_todo: Json,
    pub session_current_todo_list: String,
    pub session_background_hints: Vec<BackgroundHint>,
    pub session_default_changed_background_hint_text: String,

    pub behavior_name: String,
    pub behavior_objective: String,
    pub behavior_mode: &'static str,
    pub behavior_template_dir: Option<PathBuf>,

    pub workspace_id: Option<String>,
    pub workspace_root: Option<PathBuf>,

    pub agent_root: PathBuf,
    pub session_root: PathBuf,

    pub input_text: String,
    pub input_has_user_text: bool,
    pub input_has_events: bool,

    pub recent_activity: String,
    pub clock_unix_ms: u64,
    pub runtime_workspace_list_text: String,
    pub runtime_last_schedule_task_list_text: String,
    pub notebook_list_text: String,
    pub notebook_last_items_text: String,
    pub llm_context: LlmContextEnv,
    pub task: Option<TaskEnv>,
}

#[derive(Debug, Clone)]
pub struct TaskEnv {
    pub task_id: String,
    pub root_id: String,
    pub task_type: String,
    pub runner: String,
    pub task_name: String,
    pub user_id: String,
    pub app_id: String,
    pub parent_id: Option<String>,
}

impl From<AgentTaskBinding> for TaskEnv {
    fn from(binding: AgentTaskBinding) -> Self {
        Self {
            task_id: binding.task_id,
            root_id: binding.root_id,
            task_type: binding.task_type,
            runner: binding.runner,
            task_name: binding.task_name,
            user_id: binding.user_id,
            app_id: binding.app_id,
            parent_id: binding.parent_id,
        }
    }
}

impl AgentSessionEnv {
    /// Normalize a raw `SessionKind` to the stable string used in templates.
    pub fn kind_str(kind: SessionKind) -> &'static str {
        kind.as_str()
    }

    fn has_title(&self) -> bool {
        !self.session_title.is_empty()
    }

    fn has_current_todo(&self) -> bool {
        !self.session_current_todo.is_null()
    }

    fn background_hint_changed(&self) -> bool {
        !self.session_background_hints.is_empty()
    }

    fn has_workspace_id(&self) -> bool {
        self.workspace_id
            .as_deref()
            .map(|s| !s.is_empty())
            .unwrap_or(false)
    }

    fn has_recent_activity(&self) -> bool {
        !self.recent_activity.is_empty()
    }

    fn workspace_root_display(&self) -> String {
        self.workspace_root
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    }
}

/// `RenderVars` seeded with the Phase-1 aggregate objects. Templates can
/// reference `{{ session.id }}`, `{{ behavior.name }}`, etc. directly — the
/// engine's prepare pass auto-injects `__VAR__` declarations for plain
/// placeholders that match a seeded var name.
pub fn build_render_vars(env: &AgentSessionEnv) -> RenderVars {
    RenderVars::new()
        .with_var("session", session_object(env))
        .with_var("behavior", behavior_object(env))
        .with_var("workspace", workspace_object(env))
        .with_var("paths", paths_object(env))
        .with_var("input", input_object(env))
        .with_var("current_context", current_context_object(env))
        .with_var("task", task_object(env))
        .with_var("task_executor", task_executor_object(env))
        .with_var("runtime", runtime_object(env))
        .with_var("notebook", notebook_object(env))
        .with_var("llm_context", llm_context_object(env))
        .with_var("msgs", msgs_array(env))
        .with_var("events", events_array(env))
        .with_var("bg_events", bg_events_array(env))
        .with_var(
            "last_step",
            env.llm_context.last_step.clone().unwrap_or(Json::Null),
        )
        .with_var(
            "behavior_history",
            Json::Array(env.llm_context.behavior_history.clone()),
        )
        .with_var(
            "step_history",
            Json::Array(env.llm_context.behavior_history.clone()),
        )
        .with_var(
            "agent_global_state",
            env.llm_context.agent_global_state.clone(),
        )
        .with_var(
            "result_protocol",
            Json::String(XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT.to_string()),
        )
        .with_var(
            "xml_behavior_result_protocol",
            Json::String(XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT.to_string()),
        )
}

/// `EngineConfig` for Phase 1.
///
/// - `__INCLUDE__` paths starting with `/` are resolved from `agent_root`.
/// - Relative `__INCLUDE__` paths are resolved from the behavior template dir.
/// - `agent_root` and `session_root` are whitelisted for `__INCLUDE__`.
/// - `workspace_root` added when the session is bound to a workspace.
/// - `__EXEC__` stays disabled (engine default).
/// - memory / notepads / skills / tools roots are intentionally NOT added.
pub fn build_engine_config(env: &AgentSessionEnv) -> EngineConfig {
    let mut cfg = EngineConfig::default();
    cfg.include_root = Some(env.agent_root.clone());
    cfg.template_dir = env.behavior_template_dir.clone();
    cfg.include_roots.push(env.agent_root.clone());
    cfg.include_roots.push(env.session_root.clone());
    if let Some(root) = &env.workspace_root {
        cfg.include_roots.push(root.clone());
    }
    cfg
}

/// Loader that resolves Phase-1 `$session.*` / `$behavior.*` / `$workspace.*`
/// / `$paths.*` / `$input.*` / `$runtime.*` / `$result_protocol` expressions. Aggregate names
/// without a trailing path return the matching JSON object so
/// `__VAR(session, $session)__` works.
pub struct AgentSessionValueLoader {
    env: AgentSessionEnv,
}

impl AgentSessionValueLoader {
    pub fn new(env: AgentSessionEnv) -> Self {
        Self { env }
    }
}

#[async_trait]
impl ValueLoader for AgentSessionValueLoader {
    async fn load(&self, expr: &str) -> Result<Option<Json>, RenderError> {
        Ok(resolve_phase1(&self.env, expr))
    }
}

fn resolve_phase1(env: &AgentSessionEnv, expr: &str) -> Option<Json> {
    let key = expr.strip_prefix('$').unwrap_or(expr);
    match key {
        "session" => Some(session_object(env)),
        "session.id" => Some(Json::String(env.session_id.clone())),
        "session.kind" => Some(Json::String(env.session_kind.to_string())),
        "session.title" => Some(Json::String(env.session_title.clone())),
        "session.objective" => Some(Json::String(env.session_objective.clone())),
        "session.owner" => Some(Json::String(env.session_owner.clone())),
        "session.current_behavior" => Some(Json::String(env.behavior_name.clone())),
        "session.task_id" => Some(task_number(env, |task| task.task_id.clone())),
        "session.current_todo" => Some(env.session_current_todo.clone()),
        "session.current_todo_list" => Some(Json::String(env.session_current_todo_list.clone())),
        "session.background_hint_changed" => Some(Json::Bool(env.background_hint_changed())),
        "session.default_changed_background_hint_text" => Some(Json::String(
            env.session_default_changed_background_hint_text.clone(),
        )),
        "session.default_changed_backgrand_hint_text" => Some(Json::String(
            env.session_default_changed_background_hint_text.clone(),
        )),
        "session.background_hints" => Some(background_hints_array(env)),
        "session.has_title" => Some(Json::Bool(env.has_title())),
        "session.has_current_todo" => Some(Json::Bool(env.has_current_todo())),

        _ if key.starts_with("session.current_todo.") => {
            let path = key.trim_start_matches("session.current_todo.");
            resolve_json_path(&env.session_current_todo, path)
        }

        "behavior" => Some(behavior_object(env)),
        "behavior.name" => Some(Json::String(env.behavior_name.clone())),
        "behavior.objective" => Some(Json::String(env.behavior_objective.clone())),
        "behavior.mode" => Some(Json::String(env.behavior_mode.to_string())),

        "workspace" => Some(workspace_object(env)),
        "workspace.id" => Some(Json::String(env.workspace_id.clone().unwrap_or_default())),
        "workspace.root" => Some(Json::String(env.workspace_root_display())),
        "workspace.has_id" => Some(Json::Bool(env.has_workspace_id())),

        "paths" => Some(paths_object(env)),
        "paths.agent_root" => Some(Json::String(env.agent_root.display().to_string())),
        "paths.session_root" => Some(Json::String(env.session_root.display().to_string())),
        "paths.workspace_root" => Some(Json::String(env.workspace_root_display())),

        "input" => Some(input_object(env)),
        "input.text" => Some(Json::String(input_text(env))),
        "input.msg" => Some(first_or_null(&env.llm_context.msgs)),
        "input.msgs" | "msgs" => Some(msgs_array(env)),
        "input.event" => first_event_or_null(&env.llm_context.events),
        "input.events" | "llm_context.events" | "events" => Some(events_array(env)),
        "input.bg_events" | "llm_context.bg_events" | "bg_events" => Some(bg_events_array(env)),
        "input.timer_events" => Some(filtered_events_array(env, is_timer_event)),
        "input.reminder_events" => Some(filtered_events_array(env, |event| {
            event.event_id == "timer.reminder_check"
        })),
        "input.hard_barrier_events" => Some(filtered_events_array(env, |event| {
            event.event_id == "timer.hard_barrier"
        })),
        "input.scheduled_task_events" => Some(filtered_events_array(env, |event| {
            event.event_id == "timer.scheduled_task_check"
        })),
        "input.worksession_reports" => Some(filtered_events_array(env, |event| {
            event.event_id == "worksession_report"
        })),
        "input.has_user_text" => Some(Json::Bool(has_user_text(env))),
        "input.has_msgs" => Some(Json::Bool(!env.llm_context.msgs.is_empty())),
        "input.has_events" => Some(Json::Bool(!env.llm_context.events.is_empty())),
        "input.has_bg_events" => Some(Json::Bool(!env.llm_context.bg_events.is_empty())),
        _ if key.starts_with("input.") => {
            let path = key.trim_start_matches("input.");
            resolve_json_path(&input_object(env), path)
        }

        "runtime" => Some(runtime_object(env)),
        "runtime.clock_unix_ms" => Some(Json::from(env.clock_unix_ms)),
        "runtime.clock_text" => Some(Json::String(clock_text_from_unix_ms(env.clock_unix_ms))),
        "runtime.recent_activity" => Some(Json::String(env.recent_activity.clone())),
        "runtime.has_activity" => Some(Json::Bool(env.has_recent_activity())),
        "runtime.workspace_list_text" => {
            Some(Json::String(env.runtime_workspace_list_text.clone()))
        }
        "runtime.last_schedule_task_list_text" => Some(Json::String(
            env.runtime_last_schedule_task_list_text.clone(),
        )),

        "task" => Some(task_object(env)),
        "task.id" | "task.task_id" => Some(task_number(env, |task| task.task_id.clone())),
        "task.root_id" => Some(task_string(env, |task| task.root_id.as_str())),
        "task.type" | "task.task_type" => Some(task_string(env, |task| task.task_type.as_str())),
        "task.runner" => Some(task_string(env, |task| task.runner.as_str())),
        "task.name" | "task.task_name" => Some(task_string(env, |task| task.task_name.as_str())),
        "task.user_id" => Some(task_string(env, |task| task.user_id.as_str())),
        "task.app_id" => Some(task_string(env, |task| task.app_id.as_str())),
        "task.parent_id" => Some(
            match env.task.as_ref().and_then(|task| task.parent_id.clone()) {
                Some(id) => Json::from(id),
                None => Json::Null,
            },
        ),

        "task_executor" => Some(task_executor_object(env)),
        "task_executor.has_task" => Some(Json::Bool(env.task.is_some())),
        "task_executor.task_id" => Some(task_number(env, |task| task.task_id.clone())),

        "notebook" => Some(notebook_object(env)),
        "notebook.list_text" => Some(Json::String(env.notebook_list_text.clone())),
        "notebook.last_items_text" => Some(Json::String(env.notebook_last_items_text.clone())),

        "current_context" => Some(current_context_object(env)),
        "current_context.behavior_name" => Some(Json::String(env.behavior_name.clone())),
        "current_context.last_step" | "last_step" => {
            Some(env.llm_context.last_step.clone().unwrap_or(Json::Null))
        }
        "current_context.last_report" => Some(match &env.llm_context.last_report {
            Some(report) => Json::String(report.clone()),
            None => Json::Null,
        }),
        "current_context.step_history"
        | "step_history"
        | "llm_context.behavior_history"
        | "behavior_history" => Some(Json::Array(env.llm_context.behavior_history.clone())),
        _ if key.starts_with("current_context.") => {
            let path = key.trim_start_matches("current_context.");
            resolve_json_path(&current_context_object(env), path)
        }

        "llm_context" => Some(llm_context_object(env)),
        "llm_context.last_step" => Some(env.llm_context.last_step.clone().unwrap_or(Json::Null)),
        "llm_context.last_report" => Some(match &env.llm_context.last_report {
            Some(report) => Json::String(report.clone()),
            None => Json::Null,
        }),
        "llm_context.agent_global_state" | "agent_global_state" => {
            Some(env.llm_context.agent_global_state.clone())
        }

        "result_protocol" | "xml_behavior_result_protocol" => Some(Json::String(
            XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT.to_string(),
        )),

        _ => None,
    }
}

fn session_object(env: &AgentSessionEnv) -> Json {
    json!({
        "id": env.session_id,
        "kind": env.session_kind,
        "title": env.session_title,
        "objective": env.session_objective,
        "owner": env.session_owner,
        "current_behavior": env.behavior_name,
        "task_id": env.task.as_ref().map(|task| task.task_id.clone()),
        "current_todo": env.session_current_todo.clone(),
        "current_todo_list": env.session_current_todo_list,
        "background_hint_changed": env.background_hint_changed(),
        "default_changed_background_hint_text": env.session_default_changed_background_hint_text.clone(),
        "default_changed_backgrand_hint_text": env.session_default_changed_background_hint_text.clone(),
        "background_hints": background_hints_array(env),
        "has_title": env.has_title(),
        "has_current_todo": env.has_current_todo(),
    })
}

fn task_object(env: &AgentSessionEnv) -> Json {
    match &env.task {
        Some(task) => json!({
            "id": task.task_id,
            "task_id": task.task_id.clone(),
            "root_id": task.root_id,
            "type": task.task_type,
            "task_type": task.task_type,
            "runner": task.runner,
            "name": task.task_name,
            "task_name": task.task_name,
            "user_id": task.user_id,
            "app_id": task.app_id,
            "parent_id": task.parent_id,
        }),
        None => Json::Null,
    }
}

fn task_executor_object(env: &AgentSessionEnv) -> Json {
    json!({
        "has_task": env.task.is_some(),
        "task": task_object(env),
        "task_id": env.task.as_ref().map(|task| task.task_id.clone()),
    })
}

fn task_number(env: &AgentSessionEnv, f: impl Fn(&TaskEnv) -> String) -> Json {
    env.task
        .as_ref()
        .map(f)
        .map(Json::from)
        .unwrap_or(Json::Null)
}

fn task_string<'a>(env: &'a AgentSessionEnv, f: impl Fn(&'a TaskEnv) -> &'a str) -> Json {
    env.task
        .as_ref()
        .map(|task| Json::String(f(task).to_string()))
        .unwrap_or(Json::Null)
}

fn resolve_json_path(value: &Json, path: &str) -> Option<Json> {
    let mut current = value;
    for segment in path
        .split('.')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
    {
        current = match current {
            Json::Object(map) => map.get(segment)?,
            Json::Array(items) => items.get(segment.parse::<usize>().ok()?)?,
            _ => return None,
        };
    }
    Some(current.clone())
}

fn behavior_object(env: &AgentSessionEnv) -> Json {
    json!({
        "name": env.behavior_name,
        "objective": env.behavior_objective,
        "mode": env.behavior_mode,
    })
}

fn workspace_object(env: &AgentSessionEnv) -> Json {
    json!({
        "id": env.workspace_id.clone().unwrap_or_default(),
        "root": env.workspace_root_display(),
        "has_id": env.has_workspace_id(),
    })
}

fn paths_object(env: &AgentSessionEnv) -> Json {
    json!({
        "agent_root": env.agent_root.display().to_string(),
        "session_root": env.session_root.display().to_string(),
        "workspace_root": env.workspace_root_display(),
    })
}

fn input_object(env: &AgentSessionEnv) -> Json {
    json!({
        "text": input_text(env),
        "msg": first_or_null(&env.llm_context.msgs),
        "msgs": msgs_array(env),
        "event": first_event_or_null(&env.llm_context.events).unwrap_or(Json::Null),
        "events": events_array(env),
        "bg_events": bg_events_array(env),
        "timer_events": filtered_events_array(env, is_timer_event),
        "reminder_events": filtered_events_array(env, |event| {
            event.event_id == "timer.reminder_check"
        }),
        "hard_barrier_events": filtered_events_array(env, |event| {
            event.event_id == "timer.hard_barrier"
        }),
        "scheduled_task_events": filtered_events_array(env, |event| {
            event.event_id == "timer.scheduled_task_check"
        }),
        "worksession_reports": filtered_events_array(env, |event| {
            event.event_id == "worksession_report"
        }),
        "has_user_text": has_user_text(env),
        "has_msgs": !env.llm_context.msgs.is_empty(),
        "has_events": !env.llm_context.events.is_empty(),
        "has_bg_events": !env.llm_context.bg_events.is_empty(),
    })
}

fn runtime_object(env: &AgentSessionEnv) -> Json {
    json!({
        "clock_unix_ms": env.clock_unix_ms,
        "clock_text": clock_text_from_unix_ms(env.clock_unix_ms),
        "recent_activity": env.recent_activity,
        "has_activity": env.has_recent_activity(),
        "workspace_list_text": env.runtime_workspace_list_text,
        "last_schedule_task_list_text": env.runtime_last_schedule_task_list_text,
    })
}

fn notebook_object(env: &AgentSessionEnv) -> Json {
    json!({
        "list_text": env.notebook_list_text,
        "last_items_text": env.notebook_last_items_text,
    })
}

fn clock_text_from_unix_ms(clock_unix_ms: u64) -> String {
    let Ok(ms) = i64::try_from(clock_unix_ms) else {
        return String::new();
    };

    Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S %Z").to_string())
        .unwrap_or_default()
}

fn llm_context_object(env: &AgentSessionEnv) -> Json {
    json!({
        "msgs": msgs_array(env),
        "events": events_array(env),
        "bg_events": bg_events_array(env),
        "last_step": env.llm_context.last_step.clone().unwrap_or(Json::Null),
        "last_report": env.llm_context.last_report.clone(),
        "behavior_history": env.llm_context.behavior_history.clone(),
        "step_history": env.llm_context.behavior_history.clone(),
        "current_context": current_context_object(env),
        "agent_global_state": env.llm_context.agent_global_state.clone(),
    })
}

fn current_context_object(env: &AgentSessionEnv) -> Json {
    json!({
        "behavior_name": env.behavior_name,
        "last_step": env.llm_context.last_step.clone().unwrap_or(Json::Null),
        "last_report": env.llm_context.last_report.clone(),
        "step_history": env.llm_context.behavior_history.clone(),
    })
}

fn msgs_array(env: &AgentSessionEnv) -> Json {
    Json::Array(env.llm_context.msgs.clone())
}

fn events_array(env: &AgentSessionEnv) -> Json {
    Json::Array(
        env.llm_context
            .events
            .iter()
            .map(event_prompt_value)
            .collect(),
    )
}

fn bg_events_array(env: &AgentSessionEnv) -> Json {
    Json::Array(
        env.llm_context
            .bg_events
            .iter()
            .map(bg_event_prompt_value)
            .collect(),
    )
}

fn background_hints_array(env: &AgentSessionEnv) -> Json {
    serde_json::to_value(&env.session_background_hints).unwrap_or(Json::Array(Vec::new()))
}

fn first_or_null(values: &[Json]) -> Json {
    values.first().cloned().unwrap_or(Json::Null)
}

fn first_event_or_null(events: &[EventRef]) -> Option<Json> {
    events.first().map(event_prompt_value).or(Some(Json::Null))
}

fn is_timer_event(event: &EventRef) -> bool {
    event.event_id == "timer"
        || event.event_id.starts_with("timer.")
        || event.event_id.starts_with("timer/")
}

fn filtered_events_array(env: &AgentSessionEnv, predicate: impl Fn(&EventRef) -> bool) -> Json {
    Json::Array(
        env.llm_context
            .events
            .iter()
            .filter(|event| predicate(event))
            .map(event_prompt_value)
            .collect(),
    )
}

fn event_prompt_value(event: &EventRef) -> Json {
    let mut value = serde_json::to_value(event).unwrap_or(Json::Null);
    add_event_data_text(&mut value, &event.data);
    value
}

fn bg_event_prompt_value(event: &BgEventSnapshot) -> Json {
    let mut value = serde_json::to_value(event).unwrap_or(Json::Null);
    add_event_data_text(&mut value, &event.data);
    value
}

fn add_event_data_text(value: &mut Json, data: &Json) {
    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "data_text".to_string(),
            Json::String(json_template_text(data)),
        );
    }
}

fn json_template_text(value: &Json) -> String {
    if value.is_null() {
        String::new()
    } else {
        serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
    }
}

fn input_text(env: &AgentSessionEnv) -> String {
    if env.llm_context.msgs.is_empty() {
        return env.input_text.clone();
    }
    env.llm_context
        .msgs
        .iter()
        .filter_map(|msg| msg.get("text").and_then(|value| value.as_str()))
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn has_user_text(env: &AgentSessionEnv) -> bool {
    env.input_has_user_text || !input_text(env).trim().is_empty()
}

pub fn step_record_prompt_value(step: &StepRecord) -> Json {
    json!({
        "step_index": step.meta.step_index,
        "behavior_name": step.meta.behavior_name,
        "observation": step.observation,
        "thinking": step.thought,
        "actions": step.actions,
        "action_results": step.action_results,
        "messages_sent": step.messages_sent,
        "report": step.self_report,
        "next_behavior": step.next_behavior,
    })
}

pub fn context_snapshot_prompt_value(snapshot: &llm_context::state::LLMContextSnapshot) -> Json {
    let step_history: Vec<Json> = snapshot
        .state
        .steps
        .iter()
        .map(step_record_prompt_value)
        .collect();
    json!({
        "behavior_name": snapshot.request.behavior_name,
        "last_step": snapshot
            .state
            .last_step
            .as_ref()
            .map(step_record_prompt_value)
            .unwrap_or(Json::Null),
        "last_report": snapshot.state.last_report,
        "step_history": step_history,
    })
}

pub fn context_snapshot_prompt_value_from_env(env: &AgentSessionEnv) -> Json {
    current_context_object(env)
}

pub fn msg_ref_from_pending(input: &PendingInput, received_at_ms: u64) -> Option<Json> {
    let PendingInput::Msg {
        record_id,
        from,
        from_did,
        tunnel_did,
        text,
        ai_message,
        ..
    } = input
    else {
        return None;
    };
    let (content, attachments, default_text) = render_msg_content(ai_message);
    Some(json!({
        "record_id": record_id,
        "from": from,
        "from_did": from_did,
        "tunnel_did": tunnel_did,
        "created_at_ms": Json::Null,
        "received_at_ms": received_at_ms,
        "raw_text": if text.trim().is_empty() {
            Json::Null
        } else {
            Json::String(text.clone())
        },
        "text": default_text,
        "content": content,
        "attachments": attachments,
    }))
}

fn render_msg_content(message: &AiMessage) -> (Vec<Json>, Vec<Json>, String) {
    let mut content = Vec::new();
    let mut attachments = Vec::new();
    let mut text_parts = Vec::new();
    for block in &message.content {
        match block {
            AiContent::Text { text } => {
                content.push(json!({
                    "type": "text",
                    "text": text,
                    "attachment": Json::Null,
                    "machine": Json::Null,
                }));
                if !text.trim().is_empty() {
                    text_parts.push(text.clone());
                }
            }
            AiContent::Image { source } => {
                let attachment = attachment_ref("image", source, None);
                text_parts.push(
                    attachment
                        .get("text_marker")
                        .and_then(|value| value.as_str())
                        .unwrap_or("[image]")
                        .to_string(),
                );
                content.push(json!({
                    "type": "image",
                    "text": Json::Null,
                    "attachment": attachment.clone(),
                    "machine": Json::Null,
                }));
                attachments.push(attachment);
            }
            AiContent::Document { source, title } => {
                let attachment = attachment_ref("document", source, title.as_deref());
                text_parts.push(
                    attachment
                        .get("text_marker")
                        .and_then(|value| value.as_str())
                        .unwrap_or("[document]")
                        .to_string(),
                );
                content.push(json!({
                    "type": "document",
                    "text": Json::Null,
                    "attachment": attachment.clone(),
                    "machine": Json::Null,
                }));
                attachments.push(attachment);
            }
            AiContent::ToolUse { .. }
            | AiContent::ToolResult { .. }
            | AiContent::Thinking { .. }
            | AiContent::ProviderState { .. } => {
                content.push(json!({
                    "type": "machine",
                    "text": Json::Null,
                    "attachment": Json::Null,
                    "machine": serde_json::to_value(block).unwrap_or(Json::Null),
                }));
            }
        }
    }
    (content, attachments, text_parts.join("\n"))
}

fn attachment_ref(kind: &str, source: &ResourceRef, title: Option<&str>) -> Json {
    let (source_json, mime, fallback_title) = match source {
        ResourceRef::NamedObject { obj_id } => (
            json!({
                "type": "named_object",
                "obj_id": obj_id.to_string(),
                "url": Json::Null,
            }),
            None,
            Some(obj_id.to_string()),
        ),
        ResourceRef::Url { url, mime_hint } => (
            json!({
                "type": "url",
                "obj_id": Json::Null,
                "url": url,
            }),
            mime_hint.clone(),
            url.rsplit('/').next().map(str::to_string),
        ),
        ResourceRef::Base64 { mime, .. } => (
            json!({
                "type": "base64",
                "obj_id": Json::Null,
                "url": Json::Null,
            }),
            Some(mime.clone()),
            None,
        ),
    };
    let label = title
        .map(str::to_string)
        .or(fallback_title)
        .filter(|value| !value.trim().is_empty());
    let marker = match &label {
        Some(label) => format!("[{kind}: {label}]"),
        None => format!("[{kind}]"),
    };
    json!({
        "kind": kind,
        "source": source_json,
        "mime": mime,
        "title": label,
        "label": label,
        "text_marker": marker,
    })
}

/// Render `template` through `PromptRenderEngine` with the Phase-1 variable
/// contract.
///
/// `extra_vars` are seeded into `RenderVars.vars` on top of the Phase-1 set,
/// overriding any name collision. Use for call-site-specific values that
/// don't belong in the stable variable contract — e.g. the `render_system_
/// messages` injection of pre-read `role_md` / `self_md` markdown content
/// (which will move to `__INCLUDE__` directives once behavior templates
/// migrate). Pass an empty slice when no overlay is needed.
pub async fn render_template(
    template: &str,
    env: &AgentSessionEnv,
    extra_vars: &[(&str, Json)],
) -> Result<String, RenderError> {
    let mut vars = build_render_vars(env);
    for (key, value) in extra_vars {
        vars = vars.with_var(*key, value.clone());
    }
    let engine = PromptRenderEngine::new(build_engine_config(env));
    let loader = AgentSessionValueLoader::new(env.clone());
    let result = engine.render(template, &vars, &loader).await?;
    Ok(result.rendered)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_env() -> AgentSessionEnv {
        AgentSessionEnv {
            session_id: "s-1".into(),
            session_kind: "ui",
            session_title: "hello".into(),
            session_objective: "do thing".into(),
            session_owner: "alice".into(),
            session_current_todo: json!({
                "todo_id": "T01",
                "status": "pending",
                "title": "do thing",
                "content": "do thing details",
                "skills": ["docs"],
            }),
            session_current_todo_list: "T01 pending current - do thing".into(),
            session_background_hints: Vec::new(),
            session_default_changed_background_hint_text: String::new(),
            behavior_name: "chat_route".into(),
            behavior_objective: "route".into(),
            behavior_mode: "behavior",
            behavior_template_dir: Some(PathBuf::from("/tmp/agent/behaviors")),
            workspace_id: Some("ws1".into()),
            workspace_root: Some(PathBuf::from("/tmp/ws1")),
            agent_root: PathBuf::from("/tmp/agent"),
            session_root: PathBuf::from("/tmp/agent/sessions/s-1"),
            input_text: "hi".into(),
            input_has_user_text: true,
            input_has_events: false,
            recent_activity: "running tool".into(),
            clock_unix_ms: 123,
            runtime_workspace_list_text: "- `ws-a` (ready) — Alpha\n".into(),
            runtime_last_schedule_task_list_text:
                "- created task_id=task-1 task_title=\"daily mail check\" note=\"create from Notebook Item notebook=user/actions item=item-9\"\n".into(),
            notebook_list_text: "Available notebooks: 1 total.\n- user/preferences: preferences, 2 records, last modified 2026-05-24T10:00:00Z\n".into(),
            notebook_last_items_text: "Recent notebook items: latest 1.\nitem-1 active 2026-05-24T10:00:00Z sess-1\nUse a concise tone.\n".into(),
            llm_context: LlmContextEnv {
                msgs: vec![json!({
                    "record_id": "msg-1",
                    "from": "alice",
                    "from_did": Json::Null,
                    "tunnel_did": Json::Null,
                    "created_at_ms": Json::Null,
                    "received_at_ms": 120,
                    "raw_text": "hi",
                    "text": "hi",
                    "content": [{"type": "text", "text": "hi", "attachment": Json::Null, "machine": Json::Null}],
                    "attachments": [],
                })],
                events: vec![EventRef {
                    event_id: "timer.reminder_check".into(),
                    data: json!({"target_id": "r1"}),
                    reason: Some("reminder".into()),
                    observed_at_ms: 121,
                }],
                bg_events: vec![BgEventSnapshot {
                    event_id: "presence.changed".into(),
                    data: json!({"online": true}),
                    reason: None,
                    observed_at_ms: 122,
                }],
                background_hints: Vec::new(),
                default_changed_background_hint_text: String::new(),
                last_step: Some(json!({"step_index": 7, "behavior_name": "chat_route"})),
                last_report: Some("latest report".into()),
                behavior_history: vec![json!({"step_index": 6, "behavior_name": "chat_route"})],
                agent_global_state: json!({"mood": "steady"}),
            },
            task: None,
        }
    }

    fn minimal_env() -> AgentSessionEnv {
        AgentSessionEnv {
            session_id: "s-2".into(),
            session_kind: "ui",
            session_title: String::new(),
            session_objective: String::new(),
            session_owner: String::new(),
            session_current_todo: Json::Null,
            session_current_todo_list: String::new(),
            session_background_hints: Vec::new(),
            session_default_changed_background_hint_text: String::new(),
            behavior_name: "chat_route".into(),
            behavior_objective: String::new(),
            behavior_mode: "behavior",
            behavior_template_dir: Some(PathBuf::from("/tmp/agent/behaviors")),
            workspace_id: None,
            workspace_root: None,
            agent_root: PathBuf::from("/tmp/agent"),
            session_root: PathBuf::from("/tmp/agent/sessions/s-2"),
            input_text: String::new(),
            input_has_user_text: false,
            input_has_events: false,
            recent_activity: String::new(),
            clock_unix_ms: 999,
            runtime_workspace_list_text: String::new(),
            runtime_last_schedule_task_list_text: String::new(),
            notebook_list_text: String::new(),
            notebook_last_items_text: String::new(),
            llm_context: LlmContextEnv::default(),
            task: None,
        }
    }

    #[test]
    fn clock_text_includes_year_seconds_and_timezone() {
        let timestamp_ms = 1_779_729_908_000;
        let expected = Local
            .timestamp_millis_opt(timestamp_ms)
            .single()
            .unwrap()
            .format("%Y-%m-%d %H:%M:%S %Z")
            .to_string();

        assert_eq!(clock_text_from_unix_ms(timestamp_ms as u64), expected);
    }

    #[tokio::test]
    async fn loader_resolves_phase1_keys() {
        let mut env = sample_env();
        env.task = Some(TaskEnv {
            task_id: "t-42".to_string(),
            root_id: "t-40".to_string(),
            task_type: "agent.delegate".to_string(),
            runner: "jarvis".to_string(),
            task_name: "delegate".to_string(),
            user_id: "user".to_string(),
            app_id: "opendan".to_string(),
            parent_id: None,
        });
        let loader = AgentSessionValueLoader::new(env.clone());
        assert_eq!(
            loader.load("$session.id").await.unwrap(),
            Some(Json::String("s-1".into()))
        );
        assert_eq!(
            loader.load("$session.current_todo_list").await.unwrap(),
            Some(Json::String("T01 pending current - do thing".into()))
        );
        assert_eq!(
            loader.load("$session.current_todo").await.unwrap(),
            Some(json!({
                "todo_id": "T01",
                "status": "pending",
                "title": "do thing",
                "content": "do thing details",
                "skills": ["docs"],
            }))
        );
        assert_eq!(
            loader.load("$session.current_todo.title").await.unwrap(),
            Some(Json::String("do thing".into()))
        );
        assert_eq!(
            loader.load("$session.current_todo.content").await.unwrap(),
            Some(Json::String("do thing details".into()))
        );
        assert_eq!(
            loader.load("$task.id").await.unwrap(),
            Some(Json::from("t-42"))
        );
        assert_eq!(
            loader.load("$task_executor.has_task").await.unwrap(),
            Some(Json::Bool(true))
        );
        assert_eq!(
            loader.load("$session.has_current_todo").await.unwrap(),
            Some(Json::Bool(true))
        );
        assert_eq!(
            loader.load("$behavior.name").await.unwrap(),
            Some(Json::String("chat_route".into()))
        );
        assert_eq!(
            loader.load("$workspace.has_id").await.unwrap(),
            Some(Json::Bool(true))
        );
        assert_eq!(
            loader.load("$runtime.clock_text").await.unwrap(),
            Some(Json::String(clock_text_from_unix_ms(env.clock_unix_ms)))
        );
        assert_eq!(
            loader.load("$runtime.workspace_list_text").await.unwrap(),
            Some(Json::String(env.runtime_workspace_list_text.clone()))
        );
        assert_eq!(
            loader
                .load("$runtime.last_schedule_task_list_text")
                .await
                .unwrap(),
            Some(Json::String(
                env.runtime_last_schedule_task_list_text.clone()
            ))
        );
        assert_eq!(
            loader.load("$notebook.list_text").await.unwrap(),
            Some(Json::String(env.notebook_list_text.clone()))
        );
        assert_eq!(
            loader.load("$notebook.last_items_text").await.unwrap(),
            Some(Json::String(env.notebook_last_items_text.clone()))
        );
        assert_eq!(
            loader.load("$result_protocol").await.unwrap(),
            Some(Json::String(
                XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT.to_string()
            ))
        );
        assert_eq!(
            loader.load("$llm_context.events").await.unwrap(),
            Some(json!([{
                "event_id": "timer.reminder_check",
                "data": {"target_id": "r1"},
                "data_text": "{\"target_id\":\"r1\"}",
                "reason": "reminder",
                "observed_at_ms": 121
            }]))
        );
        assert_eq!(
            loader.load("$input.msg.text").await.unwrap(),
            Some(Json::String("hi".into()))
        );
        assert_eq!(
            loader.load("$current_context.last_report").await.unwrap(),
            Some(Json::String("latest report".into()))
        );
        assert_eq!(
            loader.load("$agent_global_state").await.unwrap(),
            Some(json!({"mood": "steady"}))
        );
        assert_eq!(loader.load("$unknown.path").await.unwrap(), None);
    }

    #[tokio::test]
    async fn loader_aggregate_object_returned() {
        let env = sample_env();
        let loader = AgentSessionValueLoader::new(env);
        let val = loader.load("$session").await.unwrap().unwrap();
        assert_eq!(val["id"], Json::String("s-1".into()));
        assert_eq!(val["kind"], Json::String("ui".into()));
        assert_eq!(val["has_title"], Json::Bool(true));
    }

    #[tokio::test]
    async fn engine_substitutes_aggregate_dotted_path() {
        let env = sample_env();
        let out = render_template(
            "id={{ session.id }} todo={{ session.current_todo.title }}",
            &env,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(out, "id=s-1 todo=do thing");
    }

    #[tokio::test]
    async fn engine_substitutes_xml_behavior_result_protocol() {
        let env = sample_env();
        let out = render_template("{{ result_protocol }}", &env, &[])
            .await
            .unwrap();
        assert_eq!(out, XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT);

        let alias = render_template("{{ xml_behavior_result_protocol }}", &env, &[])
            .await
            .unwrap();
        assert_eq!(alias, XML_BEHAVIOR_RESULT_PROTOCOL_PROMPT);
    }

    #[tokio::test]
    async fn event_data_text_renders_map_payload() {
        let env = sample_env();
        let out = render_template(
            "{% for event in input.events %}{{ event.event_id }} {{ event.data_text }}{% endfor %}",
            &env,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(out, "timer.reminder_check {\"target_id\":\"r1\"}");
    }

    #[tokio::test]
    async fn extra_vars_seed_overlay() {
        let env = sample_env();
        let extras = vec![
            ("role_md", Json::String("ROLE".into())),
            ("self_md", Json::String("SELF".into())),
        ];
        let template = "{{ role_md }}\n\n{{ self_md }}";
        let out = render_template(template, &env, &extras).await.unwrap();
        assert_eq!(out, "ROLE\n\nSELF");
    }

    #[tokio::test]
    async fn extra_vars_support_on_behavior_switch_from_context_report() {
        let env = sample_env();
        let extras = vec![(
            "from_context",
            json!({
                "report": "finished todo T01",
            }),
        )];
        let out = render_template("{{ from_context.report }}", &env, &extras)
            .await
            .unwrap();
        assert_eq!(out, "finished todo T01");
    }

    #[tokio::test]
    async fn extras_and_phase1_vars_compose() {
        let env = sample_env();
        let extras = vec![("role_md", Json::String("ROLE".into()))];
        let template = "agent={{ behavior.name }}\nsession={{ session.id }}\n---\n{{ role_md }}";
        let out = render_template(template, &env, &extras).await.unwrap();
        assert_eq!(out, "agent=chat_route\nsession=s-1\n---\nROLE");
    }

    #[tokio::test]
    async fn contract_renders_main_variables_and_control_flow() {
        let mut env = sample_env();
        let msg = PendingInput::Msg {
            record_id: "msg-image".into(),
            from: "alice".into(),
            from_did: Some("did:example:alice".into()),
            from_name: Some("Alice".into()),
            tunnel_did: Some("did:example:tunnel".into()),
            text: "look".into(),
            ai_message: AiMessage::new(
                buckyos_api::AiRole::User,
                vec![
                    AiContent::text("look"),
                    AiContent::Image {
                        source: ResourceRef::url(
                            "https://example.test/screenshot.png".into(),
                            Some("image/png".into()),
                        ),
                    },
                ],
            ),
        };
        env.input_text = String::new();
        env.input_has_user_text = false;
        env.input_has_events = false;
        env.llm_context = LlmContextEnv {
            msgs: vec![msg_ref_from_pending(&msg, 42).unwrap()],
            events: vec![
                EventRef {
                    event_id: "timer.reminder_check".into(),
                    data: json!({
                        "trigger_type": "precise_trigger",
                        "target_type": "reminder",
                        "target_id": "r1",
                        "expected_trigger_time": "2026-05-24T10:00:00-07:00",
                        "reason": "drink water",
                    }),
                    reason: Some("reminder subscription".into()),
                    observed_at_ms: 101,
                },
                EventRef {
                    event_id: "timer.hard_barrier".into(),
                    data: json!({
                        "trigger_type": "hard_barrier",
                        "target_type": "other",
                        "target_id": "all",
                        "expected_trigger_time": "2026-05-24T11:00:00-07:00",
                        "reason": "daily scan",
                    }),
                    reason: Some("barrier subscription".into()),
                    observed_at_ms: 102,
                },
                EventRef {
                    event_id: "timer.scheduled_task_check".into(),
                    data: json!({
                        "trigger_type": "precise_trigger",
                        "target_type": "scheduled_task",
                        "target_id": "task-1",
                        "expected_trigger_time": "2026-05-24T12:00:00-07:00",
                        "reason": "run task",
                    }),
                    reason: Some("task subscription".into()),
                    observed_at_ms: 103,
                },
                EventRef {
                    event_id: "worksession_report".into(),
                    data: json!({
                        "report_id": "report-1",
                        "source_session_id": "work-1",
                        "target_session_id": "s-1",
                        "title": "Build",
                        "objective": "Ship",
                        "workspace_id": "ws1",
                        "phase": "final",
                        "report": "done",
                        "is_final": true,
                    }),
                    reason: Some("work report".into()),
                    observed_at_ms: 104,
                },
            ],
            bg_events: vec![BgEventSnapshot {
                event_id: "presence.changed".into(),
                data: json!({"online": true}),
                reason: Some("presence subscription".into()),
                observed_at_ms: 105,
            }],
            background_hints: Vec::new(),
            default_changed_background_hint_text: String::new(),
            last_step: Some(json!({
                "step_index": 8,
                "behavior_name": "chat_route",
                "observation": "observed",
                "thinking": "thought",
                "report": "step report",
                "next_behavior": "do",
            })),
            last_report: Some("latest report".into()),
            behavior_history: vec![
                json!({
                    "step_index": 6,
                    "behavior_name": "plan",
                    "report": "plan report",
                    "next_behavior": "do",
                }),
                json!({
                    "step_index": 7,
                    "behavior_name": "do",
                    "observation": "done",
                    "report": "do report",
                    "next_behavior": "",
                }),
            ],
            agent_global_state: json!({
                "mood": "steady",
                "driver": {
                    "hook_point": "on_behavior_switch",
                    "pulled_msg_count": 1,
                    "pulled_event_count": 4,
                },
            }),
        };
        env.session_background_hints = vec![BackgroundHint {
            path: "memory/user/preference/style".into(),
            kind: "memory".into(),
            text: "Memory may be relevant: /user/preference/style".into(),
            fingerprint: "fp-memory".into(),
            data: json!({"key": "/user/preference/style"}),
        }];
        env.session_default_changed_background_hint_text =
            "- Memory may be relevant: /user/preference/style".into();

        let extras = vec![
            (
                "switch",
                json!({
                    "from": "plan",
                    "to": "do",
                    "from_context": {
                        "behavior_name": "plan",
                        "last_report": "parent report",
                    },
                    "to_context": {
                        "behavior_name": "do",
                        "last_report": "child report",
                    },
                }),
            ),
            ("from_behavior", Json::String("plan".into())),
            (
                "from_context",
                json!({
                    "behavior_name": "plan",
                    "last_report": "parent report",
                }),
            ),
            (
                "to_context",
                json!({
                    "behavior_name": "do",
                    "last_report": "child report",
                }),
            ),
        ];
        let template = r#"
session={{ session.id }}|{{ session.kind }}|{{ session.title }}|{{ session.objective }}|{{ session.owner }}|{{ session.current_behavior }}|{{ session.current_todo.todo_id }}|{{ session.current_todo_list }}
behavior={{ behavior.name }}|{{ behavior.objective }}|{{ behavior.mode }}
workspace={{ workspace.id }}|{{ workspace.root }}|{{ workspace.has_id }}
paths={{ paths.agent_root }}|{{ paths.session_root }}|{{ paths.workspace_root }}
runtime={{ runtime.clock_unix_ms }}|{{ runtime.clock_text }}|{{ runtime.recent_activity }}|{{ runtime.has_activity }}
runtime_workspace={{ runtime.workspace_list_text }}
runtime_schedule={{ runtime.last_schedule_task_list_text }}
notebook={{ notebook.list_text }}|{{ notebook.last_items_text }}
{% if session.has_title %}if_session_title={{ session.title }}{% endif %}
{% if session.background_hint_changed %}if_background_hint={{ session.default_changed_background_hint_text }}
{% endif %}
{% for hint in session.background_hints %}hint={{ hint.path }}|{{ hint.kind }}|{{ hint.text }}
{% endfor %}
{% if input.has_user_text %}if_input_text={{ input.text }}{% endif %}
{% if input.has_msgs %}if_msgs=yes{% endif %}
{% if input.has_events %}if_events=yes{% endif %}
{% if input.has_bg_events %}if_bg=yes{% endif %}
input_msg={{ input.msg.record_id }}|{{ input.msg.from }}|{{ input.msg.from_did }}|{{ input.msg.tunnel_did }}|{{ input.msg.received_at_ms }}|{{ input.msg.raw_text }}|{{ input.msg.text }}|{{ input.msg.content.0.text }}|{{ input.msg.content.1.attachment.text_marker }}
{% for msg in input.msgs %}msg={{ msg.record_id }}|{{ msg.attachments.0.kind }}|{{ msg.attachments.0.source.type }}|{{ msg.attachments.0.source.url }}|{{ msg.attachments.0.mime }}|{{ msg.attachments.0.title }}|{{ msg.attachments.0.text_marker }}
{% endfor %}
input_event={{ input.event.event_id }}|{{ input.event.reason }}|{{ input.event.observed_at_ms }}|{{ input.event.data.target_id }}|{{ input.event.data_text }}
{% for event in input.events %}event={{ event.event_id }}|{{ event.reason }}|{{ event.observed_at_ms }}|{{ event.data_text }}
{% endfor %}
{% for event in input.bg_events %}bg={{ event.event_id }}|{{ event.reason }}|{{ event.observed_at_ms }}|{{ event.data.online }}|{{ event.data_text }}
{% endfor %}
{% for event in input.timer_events %}timer={{ event.event_id }}|{{ event.data.trigger_type }}
{% endfor %}
{% for event in input.reminder_events %}reminder={{ event.data.target_id }}|{{ event.data.reason }}
{% endfor %}
{% for event in input.hard_barrier_events %}hard={{ event.data.target_id }}|{{ event.data.reason }}
{% endfor %}
{% for event in input.scheduled_task_events %}scheduled={{ event.data.target_id }}|{{ event.data.reason }}
{% endfor %}
{% for event in input.worksession_reports %}workreport={{ event.data.report_id }}|{{ event.data.source_session_id }}|{{ event.data.target_session_id }}|{{ event.data.title }}|{{ event.data.objective }}|{{ event.data.workspace_id }}|{{ event.data.phase }}|{{ event.data.report }}|{{ event.data.is_final }}
{% endfor %}
current={{ current_context.behavior_name }}|{{ current_context.last_step.step_index }}|{{ current_context.last_step.report }}|{{ current_context.last_report }}
{% for step in current_context.step_history %}step={{ step.step_index }}|{{ step.behavior_name }}|{{ step.report }}|{{ step.next_behavior }}
{% endfor %}
legacy={{ last_step.step_index }}|{{ llm_context.last_step.step_index }}|{{ llm_context.last_report }}|{{ llm_context.agent_global_state.mood }}|{{ agent_global_state.mood }}
{% for step in behavior_history %}legacy_step={{ step.step_index }}|{{ step.behavior_name }}
{% endfor %}
{% for step in step_history %}alias_step={{ step.step_index }}|{{ step.behavior_name }}
{% endfor %}
{% for event in events %}legacy_event={{ event.event_id }}
{% endfor %}
{% for event in bg_events %}legacy_bg={{ event.event_id }}
{% endfor %}
switch={{ switch.from }}|{{ switch.to }}|{{ from_behavior }}|{{ switch.from_context.last_report }}|{{ switch.to_context.behavior_name }}|{{ from_context.last_report }}|{{ to_context.last_report }}
{% if result_protocol %}result_protocol=yes{% endif %}
{% if xml_behavior_result_protocol %}xml_protocol=yes{% endif %}
"#;
        let out = render_template(template, &env, &extras).await.unwrap();
        let expected_runtime = format!(
            "runtime=123|{}|running tool|true",
            clock_text_from_unix_ms(env.clock_unix_ms)
        );

        for expected in [
            "session=s-1|ui|hello|do thing|alice|chat_route|T01|T01 pending current - do thing",
            "behavior=chat_route|route|behavior",
            "workspace=ws1|/tmp/ws1|true",
            "paths=/tmp/agent|/tmp/agent/sessions/s-1|/tmp/ws1",
            expected_runtime.as_str(),
            "runtime_workspace=- `ws-a` (ready) — Alpha",
            "runtime_schedule=- created task_id=task-1 task_title=\"daily mail check\" note=\"create from Notebook Item notebook=user/actions item=item-9\"",
            "notebook=Available notebooks: 1 total.",
            "- user/preferences: preferences, 2 records, last modified 2026-05-24T10:00:00Z",
            "if_session_title=hello",
            "if_background_hint=- Memory may be relevant: /user/preference/style",
            "hint=memory/user/preference/style|memory|Memory may be relevant: /user/preference/style",
            "if_input_text=look\n[image: screenshot.png]",
            "if_msgs=yes",
            "if_events=yes",
            "if_bg=yes",
            "input_msg=msg-image|alice|did:example:alice|did:example:tunnel|42|look|look\n[image: screenshot.png]|look|[image: screenshot.png]",
            "msg=msg-image|image|url|https://example.test/screenshot.png|image/png|screenshot.png|[image: screenshot.png]",
            "input_event=timer.reminder_check|reminder subscription|101|r1|{",
            "event=timer.hard_barrier|barrier subscription|102|{",
            "bg=presence.changed|presence subscription|105|true|{\"online\":true}",
            "timer=timer.scheduled_task_check|precise_trigger",
            "reminder=r1|drink water",
            "hard=all|daily scan",
            "scheduled=task-1|run task",
            "workreport=report-1|work-1|s-1|Build|Ship|ws1|final|done|true",
            "current=chat_route|8|step report|latest report",
            "step=6|plan|plan report|do",
            "step=7|do|do report|",
            "legacy=8|8|latest report|steady|steady",
            "legacy_step=6|plan",
            "alias_step=7|do",
            "legacy_event=worksession_report",
            "legacy_bg=presence.changed",
            "switch=plan|do|plan|parent report|do|parent report|child report",
            "result_protocol=yes",
            "xml_protocol=yes",
        ] {
            assert!(
                out.contains(expected),
                "rendered contract output missing `{expected}`\n--- output ---\n{out}"
            );
        }
    }

    #[tokio::test]
    async fn msg_ref_renders_structured_attachments() {
        let input = PendingInput::Msg {
            record_id: "msg-image".into(),
            from: "alice".into(),
            from_did: Some("did:example:alice".into()),
            from_name: None,
            tunnel_did: Some("did:example:tunnel".into()),
            text: "look".into(),
            ai_message: AiMessage::new(
                buckyos_api::AiRole::User,
                vec![
                    AiContent::text("look"),
                    AiContent::Image {
                        source: ResourceRef::url(
                            "https://example.test/screenshot.png".into(),
                            Some("image/png".into()),
                        ),
                    },
                ],
            ),
        };
        let msg = msg_ref_from_pending(&input, 42).unwrap();
        assert_eq!(
            msg["text"],
            Json::String("look\n[image: screenshot.png]".into())
        );
        assert_eq!(msg["attachments"][0]["kind"], Json::String("image".into()));
        assert_eq!(
            msg["attachments"][0]["source"]["type"],
            Json::String("url".into())
        );
        assert_eq!(
            msg["content"][1]["attachment"]["text_marker"],
            Json::String("[image: screenshot.png]".into())
        );
    }

    #[tokio::test]
    async fn engine_config_seeds_phase1_include_roots() {
        let env = sample_env();
        let cfg = build_engine_config(&env);
        assert_eq!(cfg.include_root, Some(PathBuf::from("/tmp/agent")));
        assert_eq!(
            cfg.template_dir,
            Some(PathBuf::from("/tmp/agent/behaviors"))
        );
        assert_eq!(cfg.include_roots.len(), 3);
        assert!(cfg.include_roots.contains(&PathBuf::from("/tmp/agent")));
        assert!(cfg
            .include_roots
            .contains(&PathBuf::from("/tmp/agent/sessions/s-1")));
        assert!(cfg.include_roots.contains(&PathBuf::from("/tmp/ws1")));
        assert!(!cfg.allow_exec);
    }

    #[tokio::test]
    async fn engine_config_omits_workspace_when_unbound() {
        let env = minimal_env();
        let cfg = build_engine_config(&env);
        assert_eq!(cfg.include_roots.len(), 2);
    }
}
