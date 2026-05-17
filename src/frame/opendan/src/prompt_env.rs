//! Phase-1 integration of `llm_context::PromptRenderEngine` into AgentSession
//! egress. See `doc/opendan/Agent Enviroment.md` §15.1 for the variable
//! contract.
//!
//! Surfaces the minimal Phase-1 variable set (session / behavior / workspace
//! / paths / input / runtime) as both static `RenderVars.vars` (for upon
//! `{{ session.id }}` placeholders) and a `ValueLoader` (for explicit
//! `__VAR(name, $session)__` / `__ENV($session.id)__` lookups). Aggregate
//! objects carry sibling `has_*` booleans so templates can branch on
//! presence without relying on engine-specific string-truthy semantics.
//!
//! All OpenDAN behavior templates use the engine's upon syntax — the
//! single-brace `{name}` form is no longer supported.

use std::path::PathBuf;

use async_trait::async_trait;
use llm_context::{
    EngineConfig, PromptRenderEngine, RenderError, RenderVars, ValueLoader,
};
use serde_json::{json, Value as Json};

use crate::session_model::SessionKind;

/// Phase-1 snapshot of the variables the loader / `RenderVars` can serve.
/// Built once per turn at the egress boundary so the value set is stable
/// for the whole render even if `meta` mutates under a concurrent inbound.
///
/// String fields that drive presence checks (`session_title`,
/// `recent_activity`) are stored already-trimmed so the matching
/// `has_*` booleans line up with the historical `compose_environment_message`
/// rules byte-for-byte.
#[derive(Debug, Clone)]
pub struct AgentSessionEnv {
    pub session_id: String,
    pub session_kind: &'static str,
    pub session_title: String,
    pub session_objective: String,
    pub session_owner: String,

    pub behavior_name: String,
    pub behavior_objective: String,
    pub behavior_mode: &'static str,

    pub workspace_id: Option<String>,
    pub workspace_root: Option<PathBuf>,

    pub agent_root: PathBuf,
    pub session_root: PathBuf,

    pub input_text: String,
    pub input_has_user_text: bool,
    pub input_has_events: bool,

    pub recent_activity: String,
    pub clock_unix_ms: u64,
}

impl AgentSessionEnv {
    /// Normalize a raw `SessionKind` to the stable string used in templates.
    pub fn kind_str(kind: SessionKind) -> &'static str {
        match kind {
            SessionKind::Ui => "ui",
            SessionKind::Work => "work",
        }
    }

    fn has_title(&self) -> bool {
        !self.session_title.is_empty()
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
        .with_var("runtime", runtime_object(env))
}

/// `EngineConfig` for Phase 1.
///
/// - `agent_root` and `session_root` always whitelisted for `__INCLUDE__`.
/// - `workspace_root` added when the session is bound to a workspace.
/// - `__EXEC__` stays disabled (engine default).
/// - memory / notepads / skills / tools roots are intentionally NOT added.
pub fn build_engine_config(env: &AgentSessionEnv) -> EngineConfig {
    let mut cfg = EngineConfig::default();
    cfg.include_roots.push(env.agent_root.clone());
    cfg.include_roots.push(env.session_root.clone());
    if let Some(root) = &env.workspace_root {
        cfg.include_roots.push(root.clone());
    }
    cfg
}

/// Loader that resolves Phase-1 `$session.*` / `$behavior.*` / `$workspace.*`
/// / `$paths.*` / `$input.*` / `$runtime.*` expressions. Aggregate names
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
        "session.has_title" => Some(Json::Bool(env.has_title())),

        "behavior" => Some(behavior_object(env)),
        "behavior.name" => Some(Json::String(env.behavior_name.clone())),
        "behavior.objective" => Some(Json::String(env.behavior_objective.clone())),
        "behavior.mode" => Some(Json::String(env.behavior_mode.to_string())),

        "workspace" => Some(workspace_object(env)),
        "workspace.id" => Some(Json::String(
            env.workspace_id.clone().unwrap_or_default(),
        )),
        "workspace.root" => Some(Json::String(env.workspace_root_display())),
        "workspace.has_id" => Some(Json::Bool(env.has_workspace_id())),

        "paths" => Some(paths_object(env)),
        "paths.agent_root" => Some(Json::String(env.agent_root.display().to_string())),
        "paths.session_root" => Some(Json::String(env.session_root.display().to_string())),
        "paths.workspace_root" => Some(Json::String(env.workspace_root_display())),

        "input" => Some(input_object(env)),
        "input.text" => Some(Json::String(env.input_text.clone())),
        "input.has_user_text" => Some(Json::Bool(env.input_has_user_text)),
        "input.has_events" => Some(Json::Bool(env.input_has_events)),

        "runtime" => Some(runtime_object(env)),
        "runtime.clock_unix_ms" => Some(Json::from(env.clock_unix_ms)),
        "runtime.recent_activity" => Some(Json::String(env.recent_activity.clone())),
        "runtime.has_activity" => Some(Json::Bool(env.has_recent_activity())),

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
        "has_title": env.has_title(),
    })
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
        "text": env.input_text,
        "has_user_text": env.input_has_user_text,
        "has_events": env.input_has_events,
    })
}

fn runtime_object(env: &AgentSessionEnv) -> Json {
    json!({
        "clock_unix_ms": env.clock_unix_ms,
        "recent_activity": env.recent_activity,
        "has_activity": env.has_recent_activity(),
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

/// Phase-1 environment-block template. Mirrors the historical
/// `compose_environment_message` output line-for-line:
///
/// ```text
/// behavior: `<name>`
/// session: `<id>`[ ("<title>")]
/// [workspace: `<id>`]
/// [recent activity: <activity>]
/// clock: unix_ms=<ms>
/// ```
///
/// The `{% if %}` guards key off the explicit `has_*` booleans seeded into
/// the aggregate objects, so empty-string distinction is exact.
pub const ENVIRONMENT_BLOCK_TEMPLATE: &str = "\
behavior: `{{ behavior.name }}`
session: `{{ session.id }}`{% if session.has_title %} (\"{{ session.title }}\"){% endif %}\
{% if workspace.has_id %}
workspace: `{{ workspace.id }}`{% endif %}\
{% if runtime.has_activity %}
recent activity: {{ runtime.recent_activity }}{% endif %}
clock: unix_ms={{ runtime.clock_unix_ms }}";

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
            behavior_name: "chat_route".into(),
            behavior_objective: "route".into(),
            behavior_mode: "behavior",
            workspace_id: Some("ws1".into()),
            workspace_root: Some(PathBuf::from("/tmp/ws1")),
            agent_root: PathBuf::from("/tmp/agent"),
            session_root: PathBuf::from("/tmp/agent/sessions/s-1"),
            input_text: "hi".into(),
            input_has_user_text: true,
            input_has_events: false,
            recent_activity: "running tool".into(),
            clock_unix_ms: 123,
        }
    }

    fn minimal_env() -> AgentSessionEnv {
        AgentSessionEnv {
            session_id: "s-2".into(),
            session_kind: "ui",
            session_title: String::new(),
            session_objective: String::new(),
            session_owner: String::new(),
            behavior_name: "chat_route".into(),
            behavior_objective: String::new(),
            behavior_mode: "behavior",
            workspace_id: None,
            workspace_root: None,
            agent_root: PathBuf::from("/tmp/agent"),
            session_root: PathBuf::from("/tmp/agent/sessions/s-2"),
            input_text: String::new(),
            input_has_user_text: false,
            input_has_events: false,
            recent_activity: String::new(),
            clock_unix_ms: 999,
        }
    }

    #[tokio::test]
    async fn loader_resolves_phase1_keys() {
        let env = sample_env();
        let loader = AgentSessionValueLoader::new(env.clone());
        assert_eq!(
            loader.load("$session.id").await.unwrap(),
            Some(Json::String("s-1".into()))
        );
        assert_eq!(
            loader.load("$behavior.name").await.unwrap(),
            Some(Json::String("chat_route".into()))
        );
        assert_eq!(
            loader.load("$workspace.has_id").await.unwrap(),
            Some(Json::Bool(true))
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
        let out = render_template("id={{ session.id }}", &env, &[])
            .await
            .unwrap();
        assert_eq!(out, "id=s-1");
    }

    #[tokio::test]
    async fn environment_block_matches_full_layout() {
        let env = sample_env();
        let out = render_template(ENVIRONMENT_BLOCK_TEMPLATE, &env, &[])
            .await
            .unwrap();
        let expected = "\
behavior: `chat_route`
session: `s-1` (\"hello\")
workspace: `ws1`
recent activity: running tool
clock: unix_ms=123";
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn environment_block_minimal_layout() {
        let env = minimal_env();
        let out = render_template(ENVIRONMENT_BLOCK_TEMPLATE, &env, &[])
            .await
            .unwrap();
        let expected = "\
behavior: `chat_route`
session: `s-2`
clock: unix_ms=999";
        assert_eq!(out, expected);
    }

    #[tokio::test]
    async fn environment_block_partial_title_only() {
        let mut env = sample_env();
        env.workspace_id = None;
        env.workspace_root = None;
        env.recent_activity = String::new();
        let out = render_template(ENVIRONMENT_BLOCK_TEMPLATE, &env, &[])
            .await
            .unwrap();
        let expected = "\
behavior: `chat_route`
session: `s-1` (\"hello\")
clock: unix_ms=123";
        assert_eq!(out, expected);
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
    async fn extras_and_phase1_vars_compose() {
        let env = sample_env();
        let extras = vec![("role_md", Json::String("ROLE".into()))];
        let template =
            "agent={{ behavior.name }}\nsession={{ session.id }}\n---\n{{ role_md }}";
        let out = render_template(template, &env, &extras).await.unwrap();
        assert_eq!(out, "agent=chat_route\nsession=s-1\n---\nROLE");
    }

    #[tokio::test]
    async fn engine_config_seeds_phase1_include_roots() {
        let env = sample_env();
        let cfg = build_engine_config(&env);
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
