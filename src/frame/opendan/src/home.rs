//! What the WebUI's home page needs beyond the Agent State: the agent's
//! profile card, token usage by model and where a UI session's conversation
//! lives in the message system.
//!
//! - `agent.profile` / `agent.profile_set {display_name?, avatar?, bio?}`:
//!   nickname, avatar and a short introduction, the agent's `profile`
//!   record in the zone (the same one the control panel edits). Without a
//!   nickname the agent is called by its user name.
//! - `usage.models`: tokens by model over the last hour, the last 24 hours
//!   and in total, summed from the sessions' `usage.jsonl`.
//! - `ui.bindings`: the conversation each UI session is bound to.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use buckyos_api::{AgentId, AgentProfile};
use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::{fsutil, OpenDanError, SessionDir};
use serde::Serialize;
use serde_json::{json, Value};

use crate::records::{self, AgentRecords};

const MAX_NAME_CHARS: usize = 64;
const MAX_BIO_CHARS: usize = 500;
const MAX_AVATAR_BYTES: usize = 128 * 1024;
const HOUR_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Tokens {
    input: u64,
    output: u64,
    total: u64,
}

impl Tokens {
    fn add(&mut self, r: &UsageRecord) {
        self.input += r.input_tokens;
        self.output += r.output_tokens;
        self.total += r.total_tokens;
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Windows {
    hour: Tokens,
    day: Tokens,
    all: Tokens,
}

struct UsageCache {
    len: u64,
    records: Vec<UsageRecord>,
}

pub struct Home {
    records: Arc<dyn AgentRecords>,
    /// `None` outside a zone, like `desktop_url` (the zone desktop, where
    /// MessageHub is): there is no message system to link to.
    owner_did: Option<String>,
    desktop_url: Option<String>,
    /// Parsed `usage.jsonl` by session, re-read when the file has grown.
    usage: Mutex<HashMap<String, UsageCache>>,
}

fn text_param(params: &Value, name: &str) -> Result<Option<String>, OpenDanError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(OpenDanError::InvalidArgument(format!("param `{name}` is a string"))),
    }
}

fn store_err(e: String) -> OpenDanError {
    OpenDanError::Other(format!("agent profile: {e}"))
}

impl Home {
    pub fn new(records: Arc<dyn AgentRecords>, owner_did: Option<String>, desktop_url: Option<String>) -> Self {
        Self {
            records,
            owner_did,
            desktop_url,
            usage: Mutex::new(HashMap::new()),
        }
    }

    /// The profile card.
    pub async fn profile(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let profile = records::profile(self.records.as_ref()).await.map_err(store_err)?;
        let display_name = parse_did(agent.agent_did())
            .ok()
            .and_then(|did| AgentId::from_agent_did(&did).ok())
            .map(|id| profile.resolved_display_name(&id))
            .unwrap_or_else(|| agent.agent_id().to_string());
        Ok(json!({
            "agent_did": agent.agent_did(),
            "agent_id": agent.agent_id(),
            "display_name": display_name,
            "avatar": profile.avatar,
            "bio": profile.bio.unwrap_or_default(),
            "owner_did": self.owner_did,
            "desktop_url": self.desktop_url,
        }))
    }

    /// Change the fields that are present; an empty string clears one (the
    /// name falls back to the agent's user name).
    pub async fn set_profile(&self, params: &Value) -> Result<(), OpenDanError> {
        let mut profile = records::profile(self.records.as_ref()).await.map_err(store_err)?;
        apply_profile(&mut profile, params)?;
        records::set_profile(self.records.as_ref(), &profile)
            .await
            .map_err(store_err)
    }

    fn session_usage(&self, entry: &RegistryEntry) -> Vec<UsageRecord> {
        let Ok(sd) = SessionDir::open(&entry.location) else {
            return Vec::new();
        };
        let len = fsutil::file_len(&sd.file(USAGE_FILE)).unwrap_or(0);
        let mut cache = self.usage.lock().expect("usage cache");
        match cache.get(&entry.session_id) {
            Some(c) if c.len == len => c.records.clone(),
            _ => {
                let records = sd.usage().unwrap_or_default();
                cache.insert(
                    entry.session_id.clone(),
                    UsageCache {
                        len,
                        records: records.clone(),
                    },
                );
                records
            }
        }
    }

    /// Tokens by model, most used first. Only Rounds recorded in
    /// `usage.jsonl` count: sessions older than that file have none.
    pub async fn usage_models(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let now = libopendan::now_ms();
        let mut models: BTreeMap<String, Windows> = BTreeMap::new();
        let mut since_ms: Option<u64> = None;
        for entry in agent.sessions().query(&RegistryQuery::default()).await? {
            for r in self.session_usage(&entry) {
                let w = models.entry(r.model.clone()).or_default();
                w.all.add(&r);
                if r.at_ms + 24 * HOUR_MS >= now {
                    w.day.add(&r);
                }
                if r.at_ms + HOUR_MS >= now {
                    w.hour.add(&r);
                }
                since_ms = Some(since_ms.map_or(r.at_ms, |s| s.min(r.at_ms)));
            }
        }
        let mut models: Vec<(String, Windows)> = models.into_iter().collect();
        models.sort_by(|a, b| b.1.all.total.cmp(&a.1.all.total).then(a.0.cmp(&b.0)));
        Ok(json!({
            "now_ms": now,
            "since_ms": since_ms,
            "models": models
                .into_iter()
                .map(|(model, w)| json!({ "model": model, "hour": w.hour, "day": w.day, "all": w.all }))
                .collect::<Vec<_>>(),
        }))
    }

    /// The conversation of every UI session: the peer (or group) its
    /// replies go to and the conversation's session there.
    pub async fn ui_bindings(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let query = RegistryQuery {
            kind: Some(SessionKind::Ui),
            ..Default::default()
        };
        let mut out = Vec::new();
        for entry in agent.sessions().query(&query).await? {
            let outbound = SessionDir::open(&entry.location)
                .and_then(|sd| sd.config())
                .ok()
                .and_then(|cfg| cfg.channels.outbound);
            if let Some(o) = outbound {
                out.push(json!({
                    "session_id": entry.session_id,
                    "to": o.to,
                    "to_session": o.to_session,
                    "kind": o.kind,
                }));
            }
        }
        Ok(json!(out))
    }

    /// The runtimes assigned to this agent (`AgentSettings.runtimes`) and the
    /// owner's default: the one in effect (`default_runtime_id`, the private
    /// runtime while the configured one is switched off) and the one the
    /// owner configured.
    pub async fn runtime_settings(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let settings = records::settings(self.records.as_ref()).await.map_err(store_err)?;
        let default = agent.runtimes().lookup(settings.effective_default_runtime()).await?
            .map(|r| r.runtime_id).unwrap_or_else(|| settings.effective_default_runtime().into());
        Ok(json!({
            "runtimes": settings.runtimes,
            "default_runtime_id": default,
            "configured_default_runtime_id": settings.default_runtime_id,
        }))
    }

    /// The settings key a runtime id names (`None`: the private runtime,
    /// which has no entry) and whether it is the private runtime under one
    /// of its names (the owner's machine, when the service runs natively).
    async fn settings_key(agent: &dyn AgentStateClient, runtime_id: &str) -> Result<(Option<String>, bool), OpenDanError> {
        if is_private_runtime(runtime_id) {
            return Ok((None, true));
        }
        Ok(match agent.runtimes().lookup(runtime_id).await?.map(|r| r.connector) {
            Some(RuntimeConnector::Native { aliases, .. }) => (aliases.first().cloned(), true),
            _ => (Some(runtime_id.to_string()), false),
        })
    }

    /// `runtimes.impact {runtime_id}`: what switching a runtime off or
    /// removing it touches. `blocking`: something is in progress on it
    /// (sessions that are not finished, Todos waiting to run there).
    pub async fn runtime_impact(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<Value, OpenDanError> {
        let asked = required(params, "runtime_id")?;
        let record = agent.runtimes().lookup(&asked).await?.ok_or_else(|| OpenDanError::NotFound(format!("runtime {asked}")))?;
        let id = record.runtime_id.clone();
        let open: Vec<RegistryEntry> = agent
            .sessions()
            .query(&RegistryQuery::default())
            .await?
            .into_iter()
            .filter(|e| e.status.run_state != RunState::Finished)
            .collect();
        let sessions: Vec<Value> = open
            .iter()
            .filter(|e| e.runtime_id.as_deref().is_some_and(|r| record.names(r)))
            .map(|e| json!({
                "session_id": e.session_id,
                "kind": e.kind,
                "class": e.class,
                "title": e.title.clone().unwrap_or_else(|| e.objective.clone()),
                "run_state": e.status.run_state,
            }))
            .collect();
        let mut todos = Vec::new();
        for e in &open {
            let Ok(sd) = SessionDir::open(&e.location) else { continue };
            let Ok(list) = libopendan::todos::store_of(&sd).load() else { continue };
            for t in list.todos.iter().filter(|t| {
                !t.status.is_terminal() && t.running().is_none() && t.runtime_id.as_deref().is_some_and(|r| record.names(r))
            }) {
                todos.push(json!({ "session_id": e.session_id, "todo_id": t.todo_id, "title": t.title, "status": t.status }));
            }
        }
        let workspaces: Vec<Value> = agent
            .workspaces()
            .query(&WorkspaceQuery::default())
            .await?
            .into_iter()
            .filter(|w| record.names(&w.location.runtime_id))
            .map(|w| json!({ "workspace_id": w.workspace_id, "name": w.name, "directory": w.location.directory, "lifecycle": w.lifecycle }))
            .collect();
        Ok(json!({
            "runtime_id": id,
            "owner_default": record.default,
            "blocking": !sessions.is_empty() || !todos.is_empty(),
            "active_sessions": sessions,
            "pending_todos": todos,
            "workspaces": workspaces,
            "usage": record.usage,
        }))
    }

    /// What is in progress on a runtime must be seen before it is taken
    /// away: without `confirm: true` the change is refused while there is
    /// any (`all`: also for workspaces bound to it).
    async fn confirmed(&self, agent: &dyn AgentStateClient, runtime_id: &str, params: &Value, verb: &str, all: bool) -> Result<(), OpenDanError> {
        if params.get("confirm").and_then(Value::as_bool).unwrap_or(false) {
            return Ok(());
        }
        let Ok(impact) = self.runtime_impact(agent, &json!({ "runtime_id": runtime_id })).await else {
            return Ok(());
        };
        let count = |key: &str| impact[key].as_array().map(Vec::len).unwrap_or(0);
        let (sessions, todos, workspaces) = (count("active_sessions"), count("pending_todos"), count("workspaces"));
        if sessions + todos == 0 && (!all || workspaces == 0) {
            return Ok(());
        }
        Err(OpenDanError::InvalidArgument(format!(
            "confirm_required: runtime `{runtime_id}` has {sessions} session(s) in progress, {todos} Todo(s) waiting to run on it and {workspaces} workspace(s); they are not moved. Show them (`runtimes.impact`) and {verb} with `confirm: true`"
        )))
    }

    /// `runtimes.settings_set {runtime_id, enabled?, default?, confirm?}`:
    /// whether this agent uses a runtime it was assigned, and the owner's
    /// default. What a runtime allows is its own configuration and is not
    /// changed here; assigning a runtime is the control panel's
    /// (`agent.update`). A default that is switched off keeps its place.
    /// Returns the new settings.
    pub async fn set_runtime_settings(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<buckyos_api::AgentSettings, OpenDanError> {
        let invalid = |m: String| OpenDanError::InvalidArgument(m);
        let mut settings = records::settings(self.records.as_ref()).await.map_err(store_err)?;
        let asked = required(params, "runtime_id")?;
        let (key, private) = Self::settings_key(agent, &asked).await?;
        if let Some(enabled) = params.get("enabled").and_then(Value::as_bool) {
            if private {
                if !enabled {
                    return Err(invalid(format!("`{asked}` is the agent's private runtime: it is always on")));
                }
            } else {
                let key = key.clone().unwrap_or_default();
                let entry = settings.runtimes.get(&key).cloned().ok_or_else(|| {
                    invalid(format!("runtime `{key}` is not assigned to this agent; the owner assigns runtimes in the runtime directory (control panel `agent.update`)"))
                })?;
                if entry.enabled && !enabled {
                    self.confirmed(agent, &key, params, "switch it off", false).await?;
                }
                if !entry.enabled && enabled && matches!(entry.target, buckyos_api::AgentRuntimeTarget::Executor { .. }) {
                    // A device is used by one agent at a time: only the
                    // control panel sees the owner's other agents.
                    enable_through_control_panel(&key).await?;
                    settings = records::settings(self.records.as_ref()).await.map_err(store_err)?;
                } else if let Some(entry) = settings.runtimes.get_mut(&key) {
                    entry.enabled = enabled;
                }
            }
        }
        if let Some(default) = params.get("default").and_then(Value::as_bool) {
            let key = key.clone().unwrap_or_else(|| buckyos_api::CONTAINER_RUNTIME_ID.to_string());
            if default {
                settings.default_runtime_id = Some(key).filter(|k| k != buckyos_api::CONTAINER_RUNTIME_ID);
            } else if settings.default_runtime_id.as_deref() == Some(&key) {
                settings.default_runtime_id = None;
            }
        }
        settings.validate_runtimes().map_err(invalid)?;
        records::set_settings(self.records.as_ref(), &settings)
            .await
            .map_err(store_err)?;
        Ok(settings)
    }

    /// `runtimes.remove {runtime_id, confirm?}`: take an assigned runtime
    /// away from this agent. Its record is kept as `retired` for its
    /// history; the runtime itself stays in the owner's directory and can
    /// be assigned again. Returns the new settings.
    pub async fn remove_runtime(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<buckyos_api::AgentSettings, OpenDanError> {
        let asked = required(params, "runtime_id")?;
        let (key, private) = Self::settings_key(agent, &asked).await?;
        if private {
            return Err(OpenDanError::InvalidArgument(format!("`{asked}` is the agent's private runtime: it cannot be removed")));
        }
        let key = key.unwrap_or_default();
        let mut settings = records::settings(self.records.as_ref()).await.map_err(store_err)?;
        if !settings.runtimes.contains_key(&key) {
            return Err(OpenDanError::InvalidArgument(format!("runtime `{key}` is not assigned to this agent")));
        }
        self.confirmed(agent, &key, params, "remove it", true).await?;
        settings.runtimes.remove(&key);
        if settings.default_runtime_id.as_deref() == Some(&key) {
            settings.default_runtime_id = None;
        }
        settings.validate_runtimes().map_err(OpenDanError::InvalidArgument)?;
        records::set_settings(self.records.as_ref(), &settings)
            .await
            .map_err(store_err)?;
        Ok(settings)
    }

    /// `skills.status`: every work skill against one runtime — whether a session there would take it (`unmet`: its
    /// baseline or platform), the commands its manual relies on and which extension package would bring what is missing.
    /// The check runs now, so a package that just went active or away
    /// shows at once. The package catalog is the Zone's: an Agent that cannot reach it still knows what is missing, only
    /// not what would bring it.
    pub async fn skills_status(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<Value, OpenDanError> {
        let runtime_id = text_param(params, "runtime_id")?.filter(|id| !id.is_empty()).unwrap_or_else(|| "private".to_string());
        let skills = agent.behaviors().skills(Some(SkillKind::Work)).await?;
        let mut wanted: Vec<String> = skills.iter().flat_map(|s| s.commands.iter().cloned()).collect();
        wanted.sort();
        wanted.dedup();
        let record = agent.runtimes().check(&runtime_id, &wanted, false).await?;
        let resolved = resolved_of(&record);
        let (catalog, desired) = zone_ext_tools().await;
        let reports: Vec<Value> = skills
            .iter()
            .map(|s| {
                json!({
                    "name": s.name,
                    "scope": s.scope,
                    "enabled": s.enabled,
                    "off": s.why_off(),
                    "requires": s.requires,
                    "unmet": s.unmet_on(record.caps.baseline, &record.caps.platform.os),
                    "commands": libopendan::skills::commands_status(&s.commands, &resolved, &catalog, &desired),
                })
            })
            .collect();
        Ok(json!({"runtime_id": record.runtime_id, "baseline": record.caps.baseline, "platform": record.caps.platform.os, "skills": reports}))
    }

    /// The package of a call: sent whole (`package`), or a draft on the AgentRoot (`draft`).
    fn skill_package(agent: &dyn AgentStateClient, params: &Value) -> Result<(libopendan::skills::SkillPackage, Option<String>), OpenDanError> {
        if let Some(slug) = text_param(params, "draft")?.filter(|s| !s.is_empty()) {
            return Ok((libopendan::skills::read_draft(&skills_root(agent)?, &slug)?, Some(slug)));
        }
        let package = serde_json::from_value(params.get("package").cloned().unwrap_or(Value::Null))
            .map_err(|e| OpenDanError::InvalidArgument(format!("param `package`: {e}")))?;
        Ok((package, None))
    }

    async fn plan_of(
        agent: &dyn AgentStateClient,
        package: &libopendan::skills::SkillPackage,
        draft: Option<&str>,
    ) -> Result<libopendan::skills::InstallPlan, OpenDanError> {
        let root = skills_root(agent)?;
        let mut plan = libopendan::skills::plan(&root, package)?;
        // What the agent wrote about its own draft is part of what the owner reads before saying yes.
        if let Some(slug) = draft {
            let meta: Value = fsutil::read_json_opt(&root.join(libopendan::skills::DRAFTS_DIR).join(slug).join(libopendan::skills::DRAFT_META))?.unwrap_or(Value::Null);
            if let Some(risk) = meta.get("risk").and_then(Value::as_str).filter(|r| !r.trim().is_empty()) {
                plan.risk.findings.push(libopendan::skills::SkillFinding {
                    code: "author_note".into(),
                    level: "low".into(),
                    place: slug.to_string(),
                    note: risk.trim().to_string(),
                });
            }
        }
        Ok(plan)
    }

    /// `skills.install_plan`: what installing a package (or a draft) would do, what a reader should know before saying
    /// yes, and what its manuals rely on against a runtime (`runtime_id`, else the private one). Nothing is written.
    pub async fn skills_install_plan(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<Value, OpenDanError> {
        let (package, draft) = Self::skill_package(agent, params)?;
        let runtime_id = text_param(params, "runtime_id")?.filter(|id| !id.is_empty());
        let plan = Self::plan_of(agent, &package, draft.as_deref()).await?;
        let against = runtime_id.unwrap_or_else(|| "private".to_string());
        let record = agent.runtimes().lookup(&against).await?;
        let resolved = record.as_ref().map(resolved_of).unwrap_or_default();
        let (catalog, desired) = zone_ext_tools().await;
        let command_status: Vec<Value> = plan
            .skills
            .iter()
            .filter(|s| s.meta.kind == SkillKind::Work)
            .map(|s| json!({"name": s.meta.name, "commands": s.meta.commands, "status": libopendan::skills::commands_status(&s.meta.commands, &resolved, &catalog, &desired)}))
            .collect();
        let mut out = serde_json::to_value(&plan).map_err(|e| OpenDanError::Other(e.to_string()))?;
        out["command_status"] = json!(command_status);
        out["checked_runtime_id"] = json!(record.map(|r| r.runtime_id));
        Ok(out)
    }

    /// `skills.install` (owner): install what the owner saw the plan of. Installing is copying the package's text
    /// files onto the AgentRoot; new sessions take the skills from then on. A draft is gone once it is installed.
    pub async fn skills_install(&self, agent: &dyn AgentStateClient, params: &Value, who: &str) -> Result<Value, OpenDanError> {
        let (package, draft) = Self::skill_package(agent, params)?;
        let root = skills_root(agent)?;
        let plan = Self::plan_of(agent, &package, draft.as_deref()).await?;
        let given = text_param(params, "plan_digest")?.unwrap_or_default();
        if given != plan.plan_digest {
            return Err(OpenDanError::InvalidArgument(format!(
                "plan_digest `{given}` is not the plan of this package ({}): ask for the plan again (`skills.install_plan`) and confirm what it says",
                plan.plan_digest
            )));
        }
        let replace = params.get("replace").and_then(Value::as_bool).unwrap_or(false);
        let accepted = libopendan::skills::Accepted { level: plan.risk.level.clone(), plan_digest: plan.plan_digest.clone() };
        let report = libopendan::skills::install(&root, &package, replace, who, Some(&accepted))?;
        log::info!(
            "skills: {} {} ({}) installed by {who}, risk {}: {}",
            report.package,
            report.version,
            report.digest,
            plan.risk.level,
            report.skills.iter().map(|s| format!("{}:{}", s.kind.as_str(), s.name)).collect::<Vec<_>>().join(", ")
        );
        if let Some(slug) = &draft {
            libopendan::skills::discard_draft(&root, slug, who, true)?;
        }
        let mut out = serde_json::to_value(&report).map_err(|e| OpenDanError::Other(e.to_string()))?;
        out["plan"] = json!(plan);
        Ok(out)
    }

    /// `skills.remove` (owner).
    pub async fn skills_remove(&self, agent: &dyn AgentStateClient, params: &Value, who: &str) -> Result<Value, OpenDanError> {
        let (kind, name) = skill_ref(params)?;
        let removed = libopendan::skills::remove(&skills_root(agent)?, kind, &name, who)?;
        log::info!("skills: {}:{name} removed by {who}", kind.as_str());
        Ok(json!(removed))
    }

    /// The owner's other skill methods, all on the AgentRoot files.
    pub async fn skills_call(&self, agent: &dyn AgentStateClient, method: &str, params: &Value, who: &str) -> Result<Value, OpenDanError> {
        use libopendan::skills;
        let root = skills_root(agent)?;
        let out = |v: Value| Ok(v);
        match method {
            "skills.state" => out(json!(skills::load_state(&root)?)),
            "skills.ui_get" => out(json!(skills::ui_skills(&root)?)),
            "skills.ui_set" => {
                let names: Option<Vec<String>> = serde_json::from_value(params.get("names").cloned().unwrap_or(Value::Null))
                    .map_err(|e| OpenDanError::InvalidArgument(format!("param `names`: {e}")))?;
                out(json!(skills::set_ui_skills(&root, names, who)?))
            }
            "skills.set_enabled" => {
                let (kind, name) = skill_ref(params)?;
                let enabled = params.get("enabled").and_then(Value::as_bool).ok_or_else(|| OpenDanError::InvalidArgument("param `enabled` is true | false".into()))?;
                let reason = text_param(params, "reason")?;
                out(json!(skills::set_enabled(&root, kind, &name, enabled, reason.as_deref(), who)?))
            }
            "skills.drafts" => out(json!(skills::list_drafts(&root)?)),
            "skills.draft_file" => {
                let slug = required(params, "slug")?;
                let file = required(params, "file")?;
                out(json!({ "text": skills::read_draft_file(&root, &slug, &file)? }))
            }
            "skills.draft_discard" => {
                let slug = required(params, "slug")?;
                skills::discard_draft(&root, &slug, who, false)?;
                out(json!({ "discarded": slug }))
            }
            other => Err(OpenDanError::InvalidArgument(format!("unknown method `{other}`"))),
        }
    }

    /// `skills.import` (owner): keep the text where a work session can read it. The session that turns it into a draft
    /// is the caller's to start.
    pub fn skills_import_inbox(&self, agent: &dyn AgentStateClient, params: &Value) -> Result<SkillImport, OpenDanError> {
        const MAX_SOURCE_BYTES: usize = 512 * 1024;
        let text = params.get("text").and_then(Value::as_str).unwrap_or_default();
        if text.trim().is_empty() {
            return Err(OpenDanError::InvalidArgument("param `text` is the text to turn into skills".into()));
        }
        if text.len() > MAX_SOURCE_BYTES {
            return Err(OpenDanError::InvalidArgument(format!("the text is larger than {} KiB: split it", MAX_SOURCE_BYTES / 1024)));
        }
        let name = text_param(params, "name")?.filter(|n| !n.is_empty());
        if let Some(name) = &name {
            libopendan::ids::validate_session_id(name).map_err(|_| OpenDanError::InvalidArgument(format!("bad skill name `{name}`")))?;
        }
        let scope = text_param(params, "scope")?.filter(|s| !s.is_empty());
        if let Some(scope) = &scope {
            if !["work", "ui", "plan"].contains(&scope.as_str()) {
                return Err(OpenDanError::InvalidArgument(format!("scope `{scope}` is not work | ui | plan")));
            }
        }
        let id = libopendan::ids::new_runner_id();
        let inbox = format!("{}/.inbox/{id}/source.md", libopendan::skills::DRAFTS_DIR);
        fsutil::atomic_replace(&skills_root(agent)?.join(&inbox), text.as_bytes())?;
        Ok(SkillImport {
            inbox,
            name,
            scope,
            source: text_param(params, "source")?.filter(|s| !s.is_empty()),
            runtime_id: text_param(params, "runtime_id")?.filter(|s| !s.is_empty()),
        })
    }

    pub async fn workspaces(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let records = agent.workspaces().query(&WorkspaceQuery::default()).await?;
        Ok(json!(records.into_iter().map(|record| json!({
            "version": record.version,
            "runtime_host": record.runtime_host,
            "workspace_id": record.workspace_id,
            "name": record.name,
            "description": record.description,
            "url": record.url,
            "location": record.location,
            "usage": record.usage,
            "lifecycle": record.lifecycle,
            "availability": record.availability,
            "revision": record.revision,
            "location_revision": record.location_revision,
            "updated_at_ms": record.updated_at_ms,
            "checked_at_ms": record.checked_at_ms,
            "source_session": record.source_session,
            "conflict": record.conflict,
            "last_error": record.last_error,
        })).collect::<Vec<_>>()))
    }
}

/// A text the owner handed in to be made into skills.
pub struct SkillImport {
    /// AgentRoot-relative path of the text.
    pub inbox: String,
    pub name: Option<String>,
    pub scope: Option<String>,
    pub source: Option<String>,
    pub runtime_id: Option<String>,
}

impl SkillImport {
    /// The first message of the work that writes the draft.
    pub fn message(&self) -> String {
        let mut text = format!("Turn the text at $OPENDAN_AGENT_ROOT/{} into skill drafts for the owner to install.", self.inbox);
        if let Some(source) = &self.source {
            text.push_str(&format!("\nIt came from: {source}."));
        }
        if let Some(name) = &self.name {
            text.push_str(&format!("\nThe owner asks for the name `{name}` (the draft slug, and the skill's name when it becomes one skill)."));
        }
        if let Some(scope) = &self.scope {
            text.push_str(&format!("\nThe owner asks for a {scope} skill; say so in draft.json if the text does not fit that."));
        }
        if let Some(runtime_id) = &self.runtime_id {
            text.push_str(&format!("\nVerify its commands on runtime `{runtime_id}` (`agent-session run --runtime {runtime_id} -- command -v <cmd>`): the skill is for that machine."));
        }
        text
    }
}

fn skills_root(agent: &dyn AgentStateClient) -> Result<std::path::PathBuf, OpenDanError> {
    agent
        .agent_root()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| OpenDanError::InvalidArgument("skills are managed where the AgentRoot is".into()))
}

fn required(params: &Value, name: &str) -> Result<String, OpenDanError> {
    text_param(params, name)?.filter(|v| !v.is_empty()).ok_or_else(|| OpenDanError::InvalidArgument(format!("param `{name}` is required")))
}

fn skill_ref(params: &Value) -> Result<(SkillKind, String), OpenDanError> {
    let kind = match params.get("kind") {
        None | Some(Value::Null) => SkillKind::Work,
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| OpenDanError::InvalidArgument("param `kind` is work | plan | goal".into()))?,
    };
    Ok((kind, required(params, "name")?))
}

fn resolved_of(record: &RuntimeRecord) -> BTreeMap<String, ResolvedCommand> {
    let mut resolved = record.caps.baseline_check.as_ref().map(|b| b.resolved.clone()).unwrap_or_default();
    if let Some(tools) = record.caps.tools_check.as_ref() {
        resolved.extend(tools.resolved.clone());
    }
    resolved
}

/// The Zone's extension package catalog and desired state, from control-panel. Empty outside a Zone or when it does not
/// answer: advice goes missing, nothing else.
async fn zone_ext_tools() -> (Vec<buckyos_api::ExtToolsCatalogEntry>, std::collections::BTreeMap<String, buckyos_api::ExtToolsDesired>) {
    let answer = async {
        let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| e.to_string())?;
        let client = runtime.get_zone_service_krpc_client("control-panel").await.map_err(|e| e.to_string())?;
        client.call("ext_tools.status", json!({})).await.map_err(|e| e.to_string())
    }
    .await;
    let status: Value = match answer {
        Ok(status) => status,
        Err(error) => {
            log::warn!("extension package catalog unavailable: {error}");
            return Default::default();
        }
    };
    let mut catalog = Vec::new();
    let mut desired = std::collections::BTreeMap::new();
    for package in status["packages"].as_array().into_iter().flatten() {
        if let Ok(entry) = serde_json::from_value(package["latest"].clone()) {
            catalog.push(entry);
        }
        if let Ok(state) = serde_json::from_value(package["desired"].clone()) {
            desired.insert(package["id"].as_str().unwrap_or_default().to_string(), state);
        }
    }
    (catalog, desired)
}

/// Switch a device runtime on for this agent through the control panel,
/// which refuses while another agent of the owner has it on.
async fn enable_through_control_panel(runtime_id: &str) -> Result<(), OpenDanError> {
    let failed = |e: String| OpenDanError::Bind(format!("runtime `{runtime_id}` was not switched on: {e}"));
    let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| failed(e.to_string()))?;
    let client = runtime.get_zone_service_krpc_client("control-panel").await.map_err(|e| failed(e.to_string()))?;
    client
        .call("agent.runtime_enable", json!({ "runtime_id": runtime_id }))
        .await
        .map(|_| ())
        .map_err(|e| failed(e.to_string()))
}

/// Where the home page reads its agent root from (for re-projection).
pub fn agent_root_of(agent: &dyn AgentStateClient) -> Option<std::path::PathBuf> {
    agent.agent_root().map(|p| p.to_path_buf())
}

fn apply_profile(profile: &mut AgentProfile, params: &Value) -> Result<(), OpenDanError> {
    let invalid = |m: String| Err(OpenDanError::InvalidArgument(m));
    let name = text_param(params, "display_name")?;
    let bio = text_param(params, "bio")?;
    let avatar = text_param(params, "avatar")?;
    if name.as_ref().is_some_and(|n| n.chars().count() > MAX_NAME_CHARS) {
        return invalid(format!("display_name is longer than {MAX_NAME_CHARS} characters"));
    }
    if bio.as_ref().is_some_and(|b| b.chars().count() > MAX_BIO_CHARS) {
        return invalid(format!("bio is longer than {MAX_BIO_CHARS} characters"));
    }
    if let Some(avatar) = &avatar {
        if avatar.len() > MAX_AVATAR_BYTES {
            return invalid(format!("avatar is larger than {} KiB", MAX_AVATAR_BYTES / 1024));
        }
        if !avatar.is_empty() && !avatar.starts_with("data:image/") {
            return invalid("avatar is an image data URL".to_string());
        }
    }
    for (field, value) in [
        (&mut profile.display_name, name),
        (&mut profile.bio, bio),
        (&mut profile.avatar, avatar),
    ] {
        if let Some(v) = value {
            *field = Some(v).filter(|s| !s.is_empty());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::LocalRecords;

    #[tokio::test]
    async fn profile_edits_are_checked_and_kept() {
        let records = Arc::new(LocalRecords::memory());
        let home = Home::new(records.clone(), Some("did:bns:bob".into()), None);
        home.set_profile(&json!({ "display_name": " J ", "bio": "hello", "avatar": "data:image/png;base64,AA==" }))
            .await
            .unwrap();
        let stored = records::profile(records.as_ref()).await.unwrap();
        assert_eq!(stored.display_name.as_deref(), Some("J"));
        assert_eq!(stored.bio.as_deref(), Some("hello"));

        // Absent fields stay; an empty string clears one.
        home.set_profile(&json!({ "display_name": "" })).await.unwrap();
        let stored = records::profile(records.as_ref()).await.unwrap();
        assert!(stored.display_name.is_none() && stored.avatar.is_some());

        for bad in [
            json!({ "avatar": "javascript:alert(1)" }),
            json!({ "avatar": "https://example.com/a.png" }),
            json!({ "display_name": "x".repeat(65) }),
            json!({ "bio": 1 }),
            json!({ "avatar": format!("data:image/png;base64,{}", "A".repeat(MAX_AVATAR_BYTES)) }),
        ] {
            assert!(matches!(home.set_profile(&bad).await, Err(OpenDanError::InvalidArgument(_))), "{bad}");
        }
        assert_eq!(records::profile(records.as_ref()).await.unwrap(), stored, "a refused edit changes nothing");
    }
}
