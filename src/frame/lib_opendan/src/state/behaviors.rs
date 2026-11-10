//! Behavior catalog of an agent (xAgent §6.2) and freezing it into a
//! session (§6.3).
//!
//! File layout: `<agent_root>/behaviors/<name>.toml` (`__INCLUDE(path)__`
//! expanded relative to the including file), `role.md`, `self.md`,
//! `i18n/<lang>.md`, and the owner's supplement to the role
//! (`.meta/role_supplement.md`, written by the host).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

/// The agent's behaviors and identity text.
#[async_trait]
pub trait BehaviorCatalog: Send + Sync {
    async fn identity(&self) -> Result<IdentityText>;
    /// Parsed, includes expanded. `None`: the agent has no such behavior.
    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>>;
    async fn list(&self) -> Result<Vec<BehaviorMeta>>;
    /// Version of the catalog as a whole (audit of a freeze).
    async fn revision(&self) -> Result<String>;
    /// The manual of a work skill (`cmd_skills/<name>/SKILL.md`, front matter
    /// included). `None`: the agent has no such work skill.
    async fn cmd_skill(&self, name: &str) -> Result<Option<String>> {
        let _ = name;
        Ok(None)
    }
    /// The agent's skills of `kind` (`None`: all), by name, with whether new sessions may take each.
    async fn skills(&self, kind: Option<SkillKind>) -> Result<Vec<SkillMeta>> {
        let _ = kind;
        Ok(Vec::new())
    }
    /// A session was created with these manuals (none: a work without any): the host counts it (`skills.note_frozen`).
    async fn note_skills_frozen(&self, names: &[String], session_id: &str) -> Result<()> {
        let _ = (names, session_id);
        Ok(())
    }
    /// Replace the text of a manual that is not a package's, keeping what it was (`skills.revise`).
    async fn revise_skill(&self, kind: SkillKind, name: &str, text: &str, reason: &str, who: &str) -> Result<SkillMeta> {
        let _ = (kind, name, text, reason, who);
        Err(OpenDanError::InvalidArgument("this agent state does not revise skills".into()))
    }
    /// The commands each cmd_skill relies on, against `runtime_id` (`None`: the private runtime), and which extension
    /// package would bring what is missing (`skills.status`).
    async fn skills_status(&self, runtime_id: Option<&str>) -> Result<serde_json::Value> {
        let _ = runtime_id;
        Err(crate::error::OpenDanError::InvalidArgument("skills.status needs the Agent Service".to_string()))
    }
}

const MAX_INCLUDE_DEPTH: usize = 8;

/// What the owner adds to the package's role, appended to it.
pub const ROLE_SUPPLEMENT_FILE: &str = ".meta/role_supplement.md";

/// `<agent_root>/cmd_skills/<name>/SKILL.md`: the manuals a session takes
/// when it is created (`prompt.cmd_skills`).
pub const CMD_SKILLS_DIR: &str = "cmd_skills";

fn expand_includes(text: &str, base: &Path, depth: usize) -> Result<String> {
    const OPEN: &str = "__INCLUDE(";
    const CLOSE: &str = ")__";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(end) = after.find(CLOSE) else {
            break;
        };
        out.push_str(&rest[..start]);
        if depth >= MAX_INCLUDE_DEPTH {
            return Err(OpenDanError::InvalidArgument(format!(
                "behavior includes are nested deeper than {MAX_INCLUDE_DEPTH} at {}",
                base.display()
            )));
        }
        let path = base.join(after[..end].trim());
        let included = std::fs::read_to_string(&path).map_err(|e| OpenDanError::io(&path, e))?;
        out.push_str(&expand_includes(
            &included,
            path.parent().unwrap_or(base),
            depth + 1,
        )?);
        rest = &after[end + CLOSE.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

/// Parse one behavior file (its includes expanded).
pub fn parse_behavior(name: &str, path: &Path) -> Result<BehaviorConfig> {
    let raw = std::fs::read_to_string(path).map_err(|e| OpenDanError::io(path, e))?;
    let mut cfg: BehaviorConfig = toml::from_str(&raw)
        .map_err(|e| OpenDanError::InvalidArgument(format!("{}: {e}", path.display())))?;
    let base = path.parent().unwrap_or(Path::new("."));
    for t in [
        &mut cfg.prompt.system,
        &mut cfg.prompt.on_init,
        &mut cfg.prompt.on_input,
        &mut cfg.prompt.on_context_switch,
    ] {
        if let Some(text) = t {
            *text = expand_includes(text, base, 0)?;
        }
    }
    // `[reply] schema` may be written as JSON text (`schema = '''{…}'''`).
    if let Some(reply) = cfg.reply.as_mut() {
        if let serde_json::Value::String(text) = &reply.schema {
            reply.schema = serde_json::from_str(text).map_err(|e| {
                OpenDanError::InvalidArgument(format!("{}: [reply] schema: {e}", path.display()))
            })?;
        }
        reply
            .validate(name)
            .map_err(|e| OpenDanError::InvalidArgument(format!("{}: {e}", path.display())))?;
    }
    if cfg.meta.name.is_empty() {
        cfg.meta.name = name.to_string();
    } else if cfg.meta.name != name {
        return Err(OpenDanError::InvalidArgument(format!(
            "{}: meta.name `{}` differs from the file name",
            path.display(),
            cfg.meta.name
        )));
    }
    Ok(cfg)
}

/// Catalog on the AgentRoot files.
pub struct FsBehaviorCatalog {
    root: PathBuf,
}

impl FsBehaviorCatalog {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            root: agent_root.into(),
        }
    }

    fn dir(&self) -> PathBuf {
        self.root.join("behaviors")
    }

    fn names(&self) -> Result<Vec<String>> {
        let dir = self.dir();
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(OpenDanError::io(&dir, e)),
        };
        let mut names: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                e.file_name()
                    .to_str()
                    .and_then(|n| n.strip_suffix(".toml"))
                    .map(str::to_string)
            })
            .collect();
        names.sort();
        Ok(names)
    }
}

fn read_trimmed(path: &Path) -> String {
    std::fs::read_to_string(path)
        .map(|t| t.trim().to_string())
        .unwrap_or_default()
}

#[async_trait]
impl BehaviorCatalog for FsBehaviorCatalog {
    async fn identity(&self) -> Result<IdentityText> {
        let mut i18n = BTreeMap::new();
        if let Ok(rd) = std::fs::read_dir(self.root.join("i18n")) {
            for e in rd.flatten() {
                if let Some(lang) = e.file_name().to_str().and_then(|n| n.strip_suffix(".md")) {
                    i18n.insert(lang.to_string(), read_trimmed(&e.path()));
                }
            }
        }
        let role = [
            read_trimmed(&self.root.join("role.md")),
            read_trimmed(&self.root.join(ROLE_SUPPLEMENT_FILE)),
        ]
        .into_iter()
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
        Ok(IdentityText {
            role,
            self_text: read_trimmed(&self.root.join("self.md")),
            i18n,
        })
    }

    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>> {
        crate::ids::validate_session_id(name)
            .map_err(|_| OpenDanError::InvalidArgument(format!("bad behavior name `{name}`")))?;
        let path = self.dir().join(format!("{name}.toml"));
        if !path.is_file() {
            return Ok(None);
        }
        parse_behavior(name, &path).map(Some)
    }

    async fn list(&self) -> Result<Vec<BehaviorMeta>> {
        let mut out = Vec::new();
        for n in self.names()? {
            match parse_behavior(&n, &self.dir().join(format!("{n}.toml"))) {
                Ok(c) => out.push(c.meta),
                Err(e) => log::warn!("behavior {n}: {e}"),
            }
        }
        Ok(out)
    }

    async fn cmd_skill(&self, name: &str) -> Result<Option<String>> {
        crate::skills::manual_text(&self.root, SkillKind::Work, name)
    }

    async fn skills(&self, kind: Option<SkillKind>) -> Result<Vec<SkillMeta>> {
        crate::skills::list(&self.root, kind)
    }

    async fn note_skills_frozen(&self, names: &[String], session_id: &str) -> Result<()> {
        crate::skills::note_frozen(&self.root, names, session_id)
    }

    async fn revise_skill(&self, kind: SkillKind, name: &str, text: &str, reason: &str, who: &str) -> Result<SkillMeta> {
        let m = crate::skills::revise(&self.root, kind, name, text, reason, who)?;
        log::info!("skills: {}:{name} revised by {who}: {reason}", kind.as_str());
        Ok(m)
    }

    async fn revision(&self) -> Result<String> {
        let mut h = Sha256::new();
        let mut files: Vec<PathBuf> = vec![
            self.root.join("role.md"),
            self.root.join(ROLE_SUPPLEMENT_FILE),
            self.root.join("self.md"),
        ];
        for d in [self.dir(), self.root.join("i18n")] {
            if let Ok(rd) = std::fs::read_dir(d) {
                files.extend(rd.flatten().map(|e| e.path()));
            }
        }
        if let Ok(rd) = std::fs::read_dir(self.root.join(CMD_SKILLS_DIR)) {
            files.extend(rd.flatten().map(|e| e.path().join(SKILL_MANUAL)));
        }
        files.sort();
        for f in files {
            if let Ok(m) = std::fs::metadata(&f) {
                let mtime = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                h.update(format!("{}|{}|{mtime}\n", f.display(), m.len()).as_bytes());
            }
        }
        Ok(format!("sha256:{}", hex::encode(h.finalize())))
    }
}

/// Catalog held in memory (embedding hosts, tests).
#[derive(Default)]
pub struct MemBehaviorCatalog {
    identity: Mutex<IdentityText>,
    behaviors: Mutex<BTreeMap<String, BehaviorConfig>>,
    cmd_skills: Mutex<BTreeMap<String, String>>,
    rev: Mutex<u64>,
}

impl MemBehaviorCatalog {
    pub fn new(identity: IdentityText) -> Self {
        Self {
            identity: Mutex::new(identity),
            ..Default::default()
        }
    }

    pub fn put(&self, name: &str, mut cfg: BehaviorConfig) {
        cfg.meta.name = name.to_string();
        self.behaviors
            .lock()
            .expect("behaviors")
            .insert(name.to_string(), cfg);
        *self.rev.lock().expect("rev") += 1;
    }
}

#[async_trait]
impl BehaviorCatalog for MemBehaviorCatalog {
    async fn identity(&self) -> Result<IdentityText> {
        Ok(self.identity.lock().expect("identity").clone())
    }

    async fn get(&self, name: &str) -> Result<Option<BehaviorConfig>> {
        Ok(self.behaviors.lock().expect("behaviors").get(name).cloned())
    }

    async fn list(&self) -> Result<Vec<BehaviorMeta>> {
        Ok(self
            .behaviors
            .lock()
            .expect("behaviors")
            .values()
            .map(|b| b.meta.clone())
            .collect())
    }

    async fn revision(&self) -> Result<String> {
        Ok(format!("mem:{}", self.rev.lock().expect("rev")))
    }

    async fn cmd_skill(&self, name: &str) -> Result<Option<String>> {
        Ok(self.cmd_skills.lock().expect("cmd skills").get(name).cloned())
    }

    async fn skills(&self, kind: Option<SkillKind>) -> Result<Vec<SkillMeta>> {
        if kind.is_some_and(|k| k != SkillKind::Work) {
            return Ok(Vec::new());
        }
        Ok(self
            .cmd_skills
            .lock()
            .expect("cmd skills")
            .iter()
            .map(|(n, t)| SkillMeta {
                origin: "package".into(),
                ..skill_meta(SkillKind::Work, n, t)
            })
            .collect())
    }
}

fn check_llm_context(cfg: &SessionConfig) -> Result<()> {
    // `prepare_hosted` ignores prompt sections of the `.llm_context`: a
    // frozen session's system comes from its behaviors only.
    for key in ["prompt", "groups", "runs_dir"] {
        if cfg.prompt.llm_context.get(key).is_some() {
            return Err(OpenDanError::InvalidArgument(format!(
                "prompt.llm_context.{key} has no effect in a session; the system text comes from the frozen behavior"
            )));
        }
    }
    Ok(())
}

/// Freeze the identity, the entry behavior and the closure it declares
/// (`meta.next`) into `cfg` (`prompt.frozen` and the runner's entry table).
/// Entry modes are validated here; nothing is frozen on an error.
pub async fn freeze_config(
    cfg: &mut SessionConfig,
    catalog: &dyn BehaviorCatalog,
    who: &str,
) -> Result<()> {
    check_llm_context(cfg)?;
    let mut frozen = FrozenPrompt {
        catalog_rev: catalog.revision().await?,
        frozen_at_ms: crate::now_ms(),
        frozen_by: who.to_string(),
        identity: catalog.identity().await?,
        behaviors: BTreeMap::new(),
        cmd_skills: Vec::new(),
        unavailable_cmd_skills: Vec::new(),
        skill_candidates: Vec::new(),
    };
    // The session's manuals are frozen with its behaviors: one the agent does not have is a configuration error;
    // one that is off (disabled by the owner, commands not installed) is left out without a word; one its
    // session-runtime does not meet (declared baseline, platform) is left out and reported, never injected. A session
    // whose entry behavior only chooses manuals for others (a plan) freezes none: it sees their names as candidates.
    check_cmd_skill_names(&cfg.prompt.cmd_skills)?;
    let candidates_only = match &cfg.prompt.behavior {
        Some(entry) => catalog.get(entry).await?.is_some_and(|b| b.capabilities.cmd_skills == CmdSkillsUse::Candidates),
        None => false,
    };
    let known = if cfg.prompt.cmd_skills.is_empty() { Vec::new() } else { catalog.skills(Some(SkillKind::Work)).await? };
    let ui = cfg.session.kind == SessionKind::Ui;
    let runtime = cfg
        .runtime
        .frozen
        .as_ref()
        .map(|f| (f.baseline(), f.caps.platform.os.clone(), f.runtime_id.clone()));
    for name in &cfg.prompt.cmd_skills {
        let meta = known.iter().find(|s| &s.name == name);
        let Some(text) = catalog.cmd_skill(name).await? else {
            let other = catalog.skills(None).await.unwrap_or_default().into_iter().find(|s| &s.name == name && s.kind != SkillKind::Work);
            return Err(OpenDanError::InvalidArgument(match other {
                Some(other) => format!(
                    "`{name}` is a {} skill: it is read when needed (`{}`), never given to a session as a manual",
                    other.kind.as_str(),
                    other.kind.manual(name)
                ),
                None => format!(
                    "the agent has no work skill `{name}` (installed: {}; see `agent-session skill list`)",
                    if known.is_empty() { "none".to_string() } else { known.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", ") }
                ),
            }));
        };
        let parsed = skill_meta(SkillKind::Work, name, &text);
        let meta = meta.unwrap_or(&parsed);
        if !meta.enabled {
            continue;
        }
        if candidates_only {
            frozen.skill_candidates.push(SkillCandidate { name: name.clone(), description: meta.description.clone() });
            continue;
        }
        if ui && !meta.scope.ui() {
            log::warn!("session {}: `{name}` is a work manual (scope: work), left out of the conversation", cfg.session.session_id);
            continue;
        }
        if !ui && !meta.scope.work() {
            return Err(OpenDanError::InvalidArgument(format!(
                "`{name}` is a conversation manual (scope: ui): work takes manuals whose scope includes `work`"
            )));
        }
        if let Some((baseline, platform, runtime_id)) = &runtime {
            if let Some(reason) = parsed.unmet_on(*baseline, platform) {
                frozen.unavailable_cmd_skills.push(UnavailableCmdSkill {
                    name: name.clone(),
                    reason: format!("{reason} (runtime `{runtime_id}`)"),
                });
                continue;
            }
        }
        let missing_commands = cfg
            .runtime
            .frozen
            .as_ref()
            .map(|f| crate::skills::known_missing_commands(&parsed.commands, &f.caps))
            .unwrap_or_default();
        // Commands the skill installed itself are on the runtimes recorded for it, not everywhere.
        let installed_on = match (&meta.install_state, &runtime) {
            (Some(install), Some((_, _, runtime_id))) if !install.ok_runtimes().contains(&runtime_id.as_str()) => {
                install.ok_runtimes().into_iter().map(str::to_string).collect()
            }
            _ => Vec::new(),
        };
        frozen.cmd_skills.push(CmdSkill {
            name: name.clone(),
            text: split_front_matter(&text).1.to_string(),
            missing_commands,
            installed_on,
        });
    }
    let mut next = cfg.clone();
    let mut queue: Vec<String> = cfg.prompt.behavior.iter().cloned().collect();
    while let Some(name) = queue.pop() {
        if frozen.behaviors.contains_key(&name) {
            continue;
        }
        let b = catalog.get(&name).await?.ok_or_else(|| {
            OpenDanError::InvalidArgument(format!("the agent has no behavior `{name}`"))
        })?;
        next.set_frozen_entry(&name, &b)
            .map_err(OpenDanError::InvalidArgument)?;
        queue.extend(b.meta.next.iter().cloned());
        frozen.behaviors.insert(name, b);
    }
    if candidates_only {
        next.prompt.cmd_skills.clear();
    }
    next.prompt.frozen = Some(frozen);
    next.behaviors().map_err(OpenDanError::InvalidArgument)?;
    *cfg = next;
    Ok(())
}

/// Manuals a creator names for a work (`--cmd-skill`, a Todo's `--skill`): each is a work skill the agent has, on, and
/// for work. What its runtime does not meet is found when the session is frozen.
pub async fn check_named_skills(catalog: &dyn BehaviorCatalog, names: &[String]) -> Result<()> {
    if names.is_empty() {
        return Ok(());
    }
    check_cmd_skill_names(names)?;
    let all = catalog.skills(None).await?;
    for name in names {
        let Some(skill) = all.iter().find(|s| &s.name == name && s.kind == SkillKind::Work) else {
            return Err(OpenDanError::InvalidArgument(match all.iter().find(|s| &s.name == name) {
                Some(other) => format!(
                    "`{name}` is a {} skill: it is read when needed (`{}`), never given to a session as a manual",
                    other.kind.as_str(),
                    other.kind.manual(name)
                ),
                None => format!(
                    "the agent has no work skill `{name}` (see `agent-session skill list --kind work --scope work`)"
                ),
            }));
        };
        if let Some(why) = skill.why_off() {
            return Err(OpenDanError::InvalidArgument(format!("work skill `{name}` is off: {why}")));
        }
        if !skill.scope.work() {
            return Err(OpenDanError::InvalidArgument(format!(
                "`{name}` is a conversation manual (scope: ui): work takes manuals whose scope includes `work`"
            )));
        }
    }
    Ok(())
}

/// The cmd_skill names a session may take: at most [`MAX_CMD_SKILLS`], each
/// once, each a valid name.
pub fn check_cmd_skill_names(names: &[String]) -> Result<()> {
    if names.len() > MAX_CMD_SKILLS {
        return Err(OpenDanError::InvalidArgument(format!(
            "a session takes at most {MAX_CMD_SKILLS} cmd_skills (got {}): choose the ones the task needs",
            names.len()
        )));
    }
    let mut seen = std::collections::BTreeSet::new();
    for n in names {
        crate::ids::validate_session_id(n)
            .map_err(|_| OpenDanError::InvalidArgument(format!("bad cmd_skill name `{n}`")))?;
        if !seen.insert(n.as_str()) {
            return Err(OpenDanError::InvalidArgument(format!(
                "cmd_skill `{n}` is named twice"
            )));
        }
    }
    Ok(())
}

/// Add a behavior used for the first time after the freeze. `false`: it is
/// frozen already.
pub async fn freeze_behavior(
    cfg: &mut SessionConfig,
    catalog: &dyn BehaviorCatalog,
    name: &str,
) -> Result<bool> {
    let Some(frozen) = &cfg.prompt.frozen else {
        return Err(OpenDanError::blocked(
            "the session has no frozen behaviors to add to",
            None,
        ));
    };
    if frozen.behaviors.contains_key(name) {
        return Ok(false);
    }
    let b = catalog.get(name).await?.ok_or_else(|| {
        OpenDanError::InvalidArgument(format!("the agent has no behavior `{name}`"))
    })?;
    let mut next = cfg.clone();
    next.set_frozen_entry(name, &b)
        .map_err(OpenDanError::InvalidArgument)?;
    if let Some(f) = next.prompt.frozen.as_mut() {
        f.behaviors.insert(name.to_string(), b);
    }
    *cfg = next;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_role_supplement_follows_the_role() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".meta")).unwrap();
        std::fs::write(dir.path().join("role.md"), "You are Jarvis.\n").unwrap();
        std::fs::write(dir.path().join("self.md"), "I keep notes.").unwrap();
        let catalog = FsBehaviorCatalog::new(dir.path());
        let plain = catalog.identity().await.unwrap();
        assert_eq!(plain.role, "You are Jarvis.");
        let before = catalog.revision().await.unwrap();

        std::fs::write(dir.path().join(ROLE_SUPPLEMENT_FILE), "Answer in Japanese.\n").unwrap();
        let with = catalog.identity().await.unwrap();
        assert_eq!(with.role, "You are Jarvis.\n\nAnswer in Japanese.");
        assert_eq!(with.self_text, "I keep notes.");
        let after = catalog.revision().await.unwrap();
        assert_ne!(before, after, "the supplement is part of the catalog revision");

        std::fs::remove_file(dir.path().join(ROLE_SUPPLEMENT_FILE)).unwrap();
        assert_eq!(catalog.identity().await.unwrap(), plain);
        assert_eq!(catalog.revision().await.unwrap(), before);
    }
}
