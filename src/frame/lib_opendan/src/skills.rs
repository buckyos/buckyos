//! Skills of an agent on its AgentRoot. A skill is a directory `<kind dir>/<name>/` holding `SKILL.md` (front matter
//! and manual) and optional `references/` read on demand: work and UI manuals in `cmd_skills/` (with the commands they
//! bring in `tools/<name>/`), the methods a plan reads in `plan_skills/`, the methods a Goal uses in `goal_skills/`.
//! They come with the agent package or are installed by the owner (`skills.install`); `.meta/skills.json` records the
//! installed ones, so a newer agent package never replaces or removes them. `.meta/skills_state.json` holds what the
//! owner and the host decide about them: disabled skills, the UI set, usage counted at freeze, where the commands a
//! skill installs are.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;

/// Installed skills of the AgentRoot (`<kind>:<name>` → where they came from).
pub const SKILLS_RECORD: &str = ".meta/skills.json";
pub const SKILLS_RECORD_SCHEMA: &str = "opendan.skills/2";
pub const SKILLS_STATE: &str = ".meta/skills_state.json";
pub const SKILLS_STATE_SCHEMA: &str = "opendan.skills_state/1";
const SKILLS_STATE_LOCK: &str = ".meta/skills_state.lock";
/// Sync manifest of the agent package (written by the Loader): its keys are
/// the files the package brought.
const PACKAGE_MANIFEST: &str = ".meta/rootfs_sync.json";
/// Commands of a work skill: `tools/<name>/bin`, and what installs more (`tools/<name>/install.toml`).
/// A package directory skills never have: commands come from extension packages, not from skills.
const TOOLS_DIR: &str = "tools";
/// Skills waiting for the owner: `skills_drafts/<slug>/` laid out like a skill package, with `draft.json`.
pub const DRAFTS_DIR: &str = "skills_drafts";
pub const DRAFT_META: &str = "draft.json";
/// Earlier texts of a manual the agent rewrote: `<kind dir>/<name>/revisions/<at_ms>.md`.
pub const REVISIONS_DIR: &str = "revisions";
/// `usage` key of work created without any manual.
pub const USAGE_NONE: &str = "none";
const RECENT_SESSIONS: usize = 20;
const AUDIT_KEPT: usize = 200;
/// A UI manual larger than this is reported: every conversation carries it.
pub const UI_MANUAL_WARN_BYTES: u64 = 16 * 1024;
/// A plan skill larger than this is reported.
pub const PLAN_MANUAL_WARN_BYTES: u64 = 3 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillsRecord {
    #[serde(default)]
    pub schema: String,
    #[serde(default)]
    pub installed: BTreeMap<String, SkillInstallRecord>,
}

pub fn key(kind: SkillKind, name: &str) -> String {
    format!("{}:{name}", kind.as_str())
}

fn parse_key(key: &str) -> Option<(SkillKind, &str)> {
    let (kind, name) = key.split_once(':')?;
    Some((SkillKind::parse(kind)?, name))
}

pub fn load_record(root: &Path) -> Result<SkillsRecord> {
    let path = root.join(SKILLS_RECORD);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| OpenDanError::Other(format!("{} is damaged: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SkillsRecord::default()),
        Err(e) => Err(OpenDanError::io(&path, e)),
    }
}

fn save_record(root: &Path, record: &SkillsRecord) -> Result<()> {
    let mut r = record.clone();
    r.schema = SKILLS_RECORD_SCHEMA.into();
    crate::fsutil::atomic_replace_json(&root.join(SKILLS_RECORD), &r)
}

/// One change the host made or let the agent make to the skills, for the owner to read.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillAudit {
    pub at_ms: u64,
    pub by: String,
    /// `install | remove | enable | disable | ui_set | revise | draft | draft_discard | install_cmd |
    /// crystallize_on | crystallize_off`.
    pub action: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub skill: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
}

/// `.meta/skills_state.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsState {
    #[serde(default)]
    pub schema: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub disabled: BTreeMap<String, SkillDisabled>,
    /// The manuals of new UI sessions; absent: `agent.toml [session.ui] default_cmd_skills`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_cmd_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub usage: BTreeMap<String, SkillUsage>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub install: BTreeMap<String, SkillInstallState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audit: Vec<SkillAudit>,
    /// The owner's switch of the skill crystallization Goal; absent:
    /// `agent.toml [session.goal.crystallize] enabled`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crystallize: Option<bool>,
}

/// Switch the skill crystallization Goal on or off (the owner's).
pub fn set_crystallize(root: &Path, enabled: bool, who: &str) -> Result<SkillsState> {
    update_state(root, |state| {
        audit(state, who, if enabled { "crystallize_on" } else { "crystallize_off" }, "", "");
        state.crystallize = Some(enabled);
        Ok(state.clone())
    })
}

pub fn load_state(root: &Path) -> Result<SkillsState> {
    let path = root.join(SKILLS_STATE);
    match std::fs::read_to_string(&path) {
        Ok(raw) => serde_json::from_str(&raw)
            .map_err(|e| OpenDanError::Other(format!("{} is damaged: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SkillsState::default()),
        Err(e) => Err(OpenDanError::io(&path, e)),
    }
}

/// Change the state file under its lock: sessions are created by several processes at once.
pub fn update_state<T>(root: &Path, change: impl FnOnce(&mut SkillsState) -> Result<T>) -> Result<T> {
    let lock_path = root.join(SKILLS_STATE_LOCK);
    let mut waited = 0;
    let _lock = loop {
        if let Some(lock) = crate::lock::FileLock::try_acquire(&lock_path)? {
            break lock;
        }
        if waited >= 5_000 {
            return Err(OpenDanError::Other(format!("{} is held by another process", lock_path.display())));
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
        waited += 20;
    };
    let mut state = load_state(root)?;
    let out = change(&mut state)?;
    state.schema = SKILLS_STATE_SCHEMA.into();
    let extra = state.audit.len().saturating_sub(AUDIT_KEPT);
    state.audit.drain(..extra);
    crate::fsutil::atomic_replace_json(&root.join(SKILLS_STATE), &state)?;
    Ok(out)
}

fn audit(state: &mut SkillsState, by: &str, action: &str, skill: &str, note: &str) {
    state.audit.push(SkillAudit { at_ms: crate::now_ms(), by: by.to_string(), action: action.to_string(), skill: skill.to_string(), note: note.to_string() });
}

/// AgentRoot-relative paths (`/`-separated) of the installed skills' files:
/// the agent package's sync leaves them alone.
pub fn installed_paths(root: &Path) -> Result<Vec<String>> {
    let record = load_record(root)?;
    let mut out = Vec::new();
    for k in record.installed.keys() {
        let Some((kind, name)) = parse_key(k) else { continue };
        out.push(format!("{}/{name}/", kind.dir()));
    }
    Ok(out)
}

fn package_files(root: &Path) -> Vec<String> {
    std::fs::read_to_string(root.join(PACKAGE_MANIFEST))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("files").and_then(|f| f.as_object()).map(|f| f.keys().cloned().collect()))
        .unwrap_or_default()
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().join(SKILL_MANUAL).is_file())
                .filter_map(|e| e.file_name().to_str().map(str::to_string))
                .filter(|n| crate::ids::validate_session_id(n).is_ok())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

fn files_under(dir: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = e.path();
            if path.is_dir() {
                walk(base, &path, out);
            } else if let Ok(rel) = path.strip_prefix(base) {
                out.push(rel.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The manual of a skill (front matter included). `None`: the agent has no such skill.
pub fn manual_text(root: &Path, kind: SkillKind, name: &str) -> Result<Option<String>> {
    crate::ids::validate_session_id(name).map_err(|_| OpenDanError::InvalidArgument(format!("bad skill name `{name}`")))?;
    let path = root.join(kind.manual(name));
    match std::fs::read_to_string(&path) {
        Ok(text) => Ok(Some(text.trim().to_string())),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory) => Ok(None),
        Err(e) => Err(OpenDanError::io(&path, e)),
    }
}

/// The skills of `kind` (`None`: all) on the AgentRoot `root`, with what the state file says of each.
pub fn list(root: &Path, kind: Option<SkillKind>) -> Result<Vec<SkillMeta>> {
    let record = load_record(root)?;
    let state = load_state(root)?;
    let package = package_files(root);
    let mut out = Vec::new();
    for k in SkillKind::ALL {
        if kind.is_some_and(|want| want != k) {
            continue;
        }
        let dir = root.join(k.dir());
        for name in names_in(&dir) {
            let text = std::fs::read_to_string(root.join(k.manual(&name))).unwrap_or_default();
            let mut meta = skill_meta(k, &name, &text);
            let id = key(k, &name);
            meta.install = record.installed.get(&id).cloned();
            meta.origin = match &meta.install {
                Some(i) if i.imported_from.is_some() => "imported",
                Some(_) => "installed",
                None if package.contains(&k.manual(&name)) => "package",
                None => "local",
            }
            .into();
            meta.references = files_under(&dir.join(&name).join(SKILL_REFERENCES));
            meta.disabled = state.disabled.get(&id).cloned();
            meta.usage = state.usage.get(&id).cloned();
            meta.install_state = state.install.get(&id).cloned();
            meta.enabled = meta.disabled.is_none();
            out.push(meta);
        }
    }
    Ok(out)
}

fn find(root: &Path, kind: SkillKind, name: &str) -> Result<SkillMeta> {
    crate::ids::validate_session_id(name).map_err(|_| OpenDanError::InvalidArgument(format!("bad skill name `{name}`")))?;
    list(root, Some(kind))?
        .into_iter()
        .find(|m| m.name == name)
        .ok_or_else(|| OpenDanError::NotFound(format!("{} skill `{name}`", kind.as_str())))
}

/// Disable a skill of any origin, or enable it again. A disabled skill is taken by no new session; running sessions
/// keep what they froze.
pub fn set_enabled(root: &Path, kind: SkillKind, name: &str, enabled: bool, reason: Option<&str>, who: &str) -> Result<SkillMeta> {
    find(root, kind, name)?;
    let id = key(kind, name);
    update_state(root, |state| {
        if enabled {
            state.disabled.remove(&id);
        } else {
            state.disabled.insert(id.clone(), SkillDisabled { at_ms: crate::now_ms(), by: who.to_string(), reason: reason.unwrap_or_default().to_string() });
        }
        audit(state, who, if enabled { "enable" } else { "disable" }, &id, reason.unwrap_or_default());
        Ok(())
    })?;
    find(root, kind, name)
}

/// The manuals of new UI sessions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSkills {
    pub names: Vec<String>,
    /// `state`: the owner set them; `template`: `agent.toml [session.ui] default_cmd_skills`.
    pub source: String,
    /// Bytes of their manuals together: what every conversation carries.
    pub total_bytes: u64,
    /// Work skills the owner may add (`scope` includes `ui`, enabled, not in the set).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available: Vec<String>,
}

/// What the owner set for UI sessions (`None`: the template decides).
pub fn ui_override(root: &Path) -> Option<Vec<String>> {
    load_state(root).ok().and_then(|s| s.ui_cmd_skills)
}

pub fn ui_skills(root: &Path) -> Result<UiSkills> {
    let state = load_state(root)?;
    let (names, source) = match state.ui_cmd_skills {
        Some(names) => (names, "state"),
        None => (crate::template::SessionTemplate::load("ui", Some(root))?.default_cmd_skills, "template"),
    };
    let skills = list(root, Some(SkillKind::Work))?;
    Ok(UiSkills {
        total_bytes: skills.iter().filter(|s| names.contains(&s.name)).map(|s| s.size).sum(),
        available: skills.iter().filter(|s| s.scope.ui() && s.enabled && !names.contains(&s.name)).map(|s| s.name.clone()).collect(),
        names,
        source: source.to_string(),
    })
}

/// Set the manuals of new UI sessions (`None`: back to the template). Each is an enabled work skill whose `scope`
/// includes `ui`. Conversations that exist keep what they froze.
pub fn set_ui_skills(root: &Path, names: Option<Vec<String>>, who: &str) -> Result<UiSkills> {
    if let Some(names) = &names {
        crate::state::check_cmd_skill_names(names)?;
        let skills = list(root, Some(SkillKind::Work))?;
        for name in names {
            let Some(skill) = skills.iter().find(|s| &s.name == name) else {
                return Err(OpenDanError::InvalidArgument(format!("the agent has no work skill `{name}`")));
            };
            if !skill.scope.ui() {
                return Err(OpenDanError::InvalidArgument(format!("`{name}` is a work manual (scope: work): only a manual whose scope includes `ui` goes into conversations")));
            }
            if let Some(why) = skill.why_off() {
                return Err(OpenDanError::InvalidArgument(format!("`{name}` is off: {why}")));
            }
        }
    }
    update_state(root, |state| {
        audit(state, who, "ui_set", "", &names.as_ref().map(|n| n.join(", ")).unwrap_or_else(|| "template default".into()));
        state.ui_cmd_skills = names;
        Ok(())
    })?;
    ui_skills(root)
}

/// Count a session created with these manuals (`names` empty: a work created without any).
pub fn note_frozen(root: &Path, names: &[String], session_id: &str) -> Result<()> {
    let keys: Vec<String> = if names.is_empty() { vec![USAGE_NONE.to_string()] } else { names.iter().map(|n| key(SkillKind::Work, n)).collect() };
    update_state(root, |state| {
        for k in keys {
            let usage = state.usage.entry(k).or_default();
            usage.frozen += 1;
            usage.last_ms = crate::now_ms();
            usage.recent_sessions.retain(|s| s != session_id);
            usage.recent_sessions.push(session_id.to_string());
            let extra = usage.recent_sessions.len().saturating_sub(RECENT_SESSIONS);
            usage.recent_sessions.drain(..extra);
        }
        Ok(())
    })
}

/// Replace the body (and `description`) of a manual that is not the agent package's, keeping the text it had under
/// `revisions/`. What decides where and how the skill applies (`scope`, `commands`, `baseline`, `platforms`) and its
/// commands stay the owner's: a change there goes through a draft.
pub fn revise(root: &Path, kind: SkillKind, name: &str, text: &str, reason: &str, who: &str) -> Result<SkillMeta> {
    let meta = find(root, kind, name)?;
    if meta.origin == "package" || meta.origin == "installed" {
        return Err(OpenDanError::InvalidArgument(format!(
            "{} skill `{name}` comes from a package ({}): write a draft under {DRAFTS_DIR}/ for the owner instead",
            kind.as_str(),
            meta.origin
        )));
    }
    let path = root.join(kind.manual(name));
    let before = std::fs::read_to_string(&path).map_err(|e| OpenDanError::io(&path, e))?;
    let (old, _) = split_front_matter(&before);
    let (new, body) = split_front_matter(text);
    if body.trim().is_empty() {
        return Err(OpenDanError::InvalidArgument("the new manual has no body".into()));
    }
    check_front_matter(kind, &new).map_err(OpenDanError::InvalidArgument)?;
    for field in ["scope", "commands", "baseline", "platforms"] {
        let value = |f: &BTreeMap<String, String>| f.get(field).map(|v| v.split(',').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(",")).unwrap_or_default();
        if value(&old) != value(&new) {
            return Err(OpenDanError::InvalidArgument(format!(
                "`{field}` changes from `{}` to `{}`: that is the owner's to decide; write a draft under {DRAFTS_DIR}/ instead",
                value(&old),
                value(&new)
            )));
        }
    }
    let now = crate::now_ms();
    let backup = path.with_file_name(REVISIONS_DIR).join(format!("{now}.md"));
    crate::fsutil::atomic_replace(&backup, before.as_bytes())?;
    crate::fsutil::atomic_replace(&path, text.as_bytes())?;
    update_state(root, |state| {
        audit(state, who, "revise", &key(kind, name), &format!("{reason} (previous text: {}/{name}/{REVISIONS_DIR}/{now}.md)", kind.dir()));
        Ok(())
    })?;
    find(root, kind, name)
}

/// Of `commands`, the ones a check of this runtime found absent. A command no check has looked at is not known to be
/// missing, and a manual does not claim it is.
pub fn known_missing_commands(commands: &[String], caps: &RuntimeCaps) -> Vec<String> {
    let resolved = |c: &String| -> Option<bool> {
        let checks = [caps.baseline_check.as_ref().map(|b| &b.resolved), caps.tools_check.as_ref().map(|t| &t.resolved)];
        let asked = caps.baseline_check.as_ref().is_some_and(|b| b.error.is_none() && crate::baseline::checked_commands(false).contains(&c.as_str()));
        match checks.iter().flatten().find_map(|r| r.get(c)) {
            Some(found) => Some(!found.stub),
            None if asked => Some(false),
            None => None,
        }
    };
    commands.iter().filter(|c| resolved(c) == Some(false)).cloned().collect()
}

/// One command of a skill on one runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCommandState {
    pub present: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provided_by: Option<ProvidedBy>,
}

/// An extension package that would bring missing commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecommendedPackage {
    pub id: String,
    pub version: String,
    /// Packages it would bring along that are not enabled yet, dependencies first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires_chain: Vec<String>,
    /// The missing commands it provides.
    pub provides: Vec<String>,
}

/// `ready`: every command is there. `ready_system`: they are, some not from BuckyOS (the owner's own install, which counts).
/// `partial`: some are missing; the skill still works as far as it can. Nothing here ever blocks a skill.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SkillCommandsState {
    Ready,
    ReadySystem,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillCommandsStatus {
    pub state: SkillCommandsState,
    pub commands: BTreeMap<String, SkillCommandState>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub recommended_packages: Vec<RecommendedPackage>,
    /// Missing commands no package provides: the owner has to bring them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub no_package: Vec<String>,
}

/// A skill's `commands` against what a runtime has and what the Zone's extension packages provide. A command several
/// packages provide goes to the one needing the fewest other packages. Pure.
pub fn commands_status(
    commands: &[String],
    resolved: &BTreeMap<String, ResolvedCommand>,
    catalog: &[buckyos_api::ExtToolsCatalogEntry],
    desired: &BTreeMap<String, buckyos_api::ExtToolsDesired>,
) -> SkillCommandsStatus {
    let mut states = BTreeMap::new();
    let mut missing = Vec::new();
    let mut foreign = false;
    for command in commands {
        match resolved.get(command).filter(|r| !r.stub) {
            Some(found) => {
                foreign |= matches!(found.provided_by, ProvidedBy::System);
                states.insert(command.clone(), SkillCommandState { present: true, provided_by: Some(found.provided_by) });
            }
            None => {
                missing.push(command.clone());
                states.insert(command.clone(), SkillCommandState { present: false, provided_by: None });
            }
        }
    }
    let mut packages: BTreeMap<String, RecommendedPackage> = BTreeMap::new();
    let mut no_package = Vec::new();
    for command in &missing {
        let mut best: Option<(&buckyos_api::ExtToolsCatalogEntry, Vec<String>)> = None;
        let mut candidates: Vec<&buckyos_api::ExtToolsCatalogEntry> = catalog.iter().filter(|e| e.manifest.provides.iter().any(|p| &p.command == command)).collect();
        // The newest version of each package; packages in name order, so the answer does not depend on the Repo's listing.
        candidates.sort_by(|a, b| a.manifest.id.cmp(&b.manifest.id));
        candidates.dedup_by(|b, a| a.manifest.id == b.manifest.id);
        for candidate in candidates {
            let newest = {
                let mut versions: Vec<buckyos_api::ExtToolsCatalogEntry> = catalog.iter().filter(|e| e.manifest.id == candidate.manifest.id).cloned().collect();
                buckyos_api::ext_tools_sort_versions(&mut versions);
                versions.into_iter().next()
            };
            let Some(newest) = newest else { continue };
            let plan = buckyos_api::ext_tools_plan_enable(catalog, desired, &newest.manifest.id, Some(&newest.manifest.version));
            if !plan.errors.is_empty() || !newest.manifest.provides.iter().any(|p| &p.command == command) {
                continue;
            }
            let chain: Vec<String> = plan.enable.iter().map(|e| e.manifest.id.clone()).filter(|id| id != &newest.manifest.id).collect();
            if best.as_ref().is_none_or(|(_, b)| chain.len() < b.len()) {
                let entry = catalog.iter().find(|e| e.manifest.id == newest.manifest.id && e.manifest.version == newest.manifest.version);
                if let Some(entry) = entry {
                    best = Some((entry, chain));
                }
            }
        }
        match best {
            Some((entry, chain)) => packages
                .entry(entry.manifest.id.clone())
                .or_insert_with(|| RecommendedPackage { id: entry.manifest.id.clone(), version: entry.manifest.version.clone(), requires_chain: chain, provides: Vec::new() })
                .provides
                .push(command.clone()),
            None => no_package.push(command.clone()),
        }
    }
    let state = if !missing.is_empty() {
        SkillCommandsState::Partial
    } else if foreign {
        SkillCommandsState::ReadySystem
    } else {
        SkillCommandsState::Ready
    };
    SkillCommandsStatus { state, commands: states, recommended_packages: packages.into_values().collect(), no_package }
}

/// A skill package, unpacked: `cmd_skills/<name>/…`, `plan_skills/<name>/…`, `goal_skills/<name>/…`, each `SKILL.md`
/// and optional `references/`. Text only: a skill brings no command and no script.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkillPackage {
    /// Name of the skill package (the PIKG's App name).
    pub package: String,
    #[serde(default)]
    pub version: String,
    /// `sha256:<hex>` of the `.pikg` file it came from (none: a directory or a draft).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pikg_digest: Option<String>,
    /// A draft: what it was made from (`draft.json` `imported_from`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imported_from: Option<serde_json::Value>,
    /// Files by package-relative path (`/`-separated).
    pub files: BTreeMap<String, SkillFile>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFile {
    /// Base64 of the bytes.
    pub content: String,
}

impl SkillFile {
    pub fn bytes(&self) -> Result<Vec<u8>> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD
            .decode(&self.content)
            .map_err(|e| OpenDanError::InvalidArgument(format!("file content is not base64: {e}")))
    }

    pub fn of(bytes: &[u8]) -> Self {
        use base64::Engine;
        Self { content: base64::engine::general_purpose::STANDARD.encode(bytes) }
    }

    fn text(&self) -> Option<String> {
        String::from_utf8(self.bytes().ok()?).ok()
    }
}

/// `sha256:<hex>` over the sorted `(path, sha256(bytes))` lines
/// of a package: the same content gives the same digest wherever it was
/// packed.
pub fn content_digest(files: &BTreeMap<String, SkillFile>) -> Result<String> {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    for (path, f) in files {
        let bytes = f.bytes()?;
        h.update(format!("{path}|{}\n", hex::encode(Sha256::digest(&bytes))).as_bytes());
    }
    Ok(format!("sha256:{}", hex::encode(h.finalize())))
}

fn skill_dirs() -> [&'static str; 4] {
    [SkillKind::Work.dir(), SkillKind::Plan.dir(), SkillKind::Goal.dir(), TOOLS_DIR]
}

/// Read an unpacked skill package directory (its `cmd_skills/`, `plan_skills/`, `goal_skills/`; a `tools/` is read so
/// that the package is refused for it; everything else is left out).
pub fn read_package_dir(dir: &Path, package: &str, version: &str) -> Result<SkillPackage> {
    let mut files = BTreeMap::new();
    for top in skill_dirs() {
        for rel in files_under(&dir.join(top)) {
            let rel = format!("{top}/{rel}");
            if top != TOOLS_DIR && rel.split('/').nth(2) == Some(REVISIONS_DIR) {
                continue;
            }
            let p = dir.join(&rel);
            let bytes = std::fs::read(&p).map_err(|e| OpenDanError::io(&p, e))?;
            files.insert(rel, SkillFile::of(&bytes));
        }
    }
    Ok(SkillPackage { package: package.to_string(), version: version.to_string(), pikg_digest: None, imported_from: None, files })
}

/// What `skill install <path>` installs: a skill PIKG (`*.pikg`), or a
/// directory laid out like its `agent_skills` sub package (package name = directory name).
pub fn read_source(path: &Path) -> Result<SkillPackage> {
    if path.is_dir() {
        let name = path
            .canonicalize()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "local".into());
        return read_package_dir(path, &name, "");
    }
    read_pikg(path)
}

/// The sub package of a skill PIKG.
pub const SKILLS_SUB_PKG: &str = "agent_skills";

/// Read a skill PIKG: an App of type `agent_skills` whose `agent_skills`
/// sub package (a `tar.gz` checked against `PACKAGE_META.json`) holds the
/// skills. Package name and version are the App's.
pub fn read_pikg(path: &Path) -> Result<SkillPackage> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let bad = |m: String| OpenDanError::InvalidArgument(format!("{}: {m}", path.display()));
    let raw = std::fs::read(path).map_err(|e| OpenDanError::io(path, e))?;
    let pikg_digest = format!("sha256:{}", hex::encode(Sha256::digest(&raw)));
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(&raw))
        .map_err(|e| bad(format!("not a PIKG (zip): {e}")))?;
    let mut json_entry = |name: &str| -> Result<serde_json::Value> {
        let mut f = zip.by_name(name).map_err(|e| bad(format!("{name}: {e}")))?;
        let mut text = String::new();
        f.read_to_string(&mut text).map_err(|e| bad(format!("{name}: {e}")))?;
        serde_json::from_str(&text).map_err(|e| bad(format!("{name}: {e}")))
    };
    let app = json_entry("APPDOC.json")?;
    let meta = json_entry("PACKAGE_META.json")?;
    if app["app_type"].as_str() != Some(SKILLS_SUB_PKG) {
        return Err(bad(format!(
            "app_type is `{}`, a skill package has `{SKILLS_SUB_PKG}` (`buckyos pikg init --kind agent-skills`)",
            app["app_type"].as_str().unwrap_or_default()
        )));
    }
    let pkg_objid = app["pkg_list"][SKILLS_SUB_PKG]["pkg_objid"]
        .as_str()
        .ok_or_else(|| bad(format!("APPDOC.json has no pkg_list.{SKILLS_SUB_PKG}.pkg_objid")))?;
    let size = meta["package_objects"][pkg_objid]["size"]
        .as_u64()
        .ok_or_else(|| bad(format!("PACKAGE_META.json has no package object {pkg_objid}")))?;
    let (_, entry) = meta["content_index"]
        .as_object()
        .and_then(|m| m.iter().find(|(_, e)| e["sub_pkg_name"].as_str() == Some(SKILLS_SUB_PKG)))
        .ok_or_else(|| bad(format!("PACKAGE_META.json has no content of `{SKILLS_SUB_PKG}`")))?;
    let entry_path = entry["path"].as_str().unwrap_or_default().to_string();
    let digest = entry["digest"].as_str().unwrap_or_default().to_string();
    let mut payload = Vec::new();
    zip.by_name(&entry_path)
        .map_err(|e| bad(format!("{entry_path}: {e}")))?
        .read_to_end(&mut payload)
        .map_err(|e| bad(format!("{entry_path}: {e}")))?;
    let actual = format!("sha256:{}", hex::encode(Sha256::digest(&payload)));
    if actual != digest || payload.len() as u64 != size {
        return Err(bad(format!(
            "{entry_path} does not match PACKAGE_META.json ({actual}, {} bytes; expected {digest}, {size} bytes)",
            payload.len()
        )));
    }
    let mut files = BTreeMap::new();
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(payload.as_slice()));
    for e in tar.entries().map_err(|e| bad(format!("{entry_path}: {e}")))? {
        let mut e = e.map_err(|e| bad(format!("{entry_path}: {e}")))?;
        let kind = e.header().entry_type();
        let rel = e.path().map_err(|e| bad(e.to_string()))?.to_string_lossy().to_string();
        let rel = rel.trim_start_matches("./").trim_end_matches('/').to_string();
        if kind.is_dir() || rel.is_empty() {
            continue;
        }
        if !kind.is_file() || rel.starts_with('/') || rel.split('/').any(|p| p == "..") {
            return Err(bad(format!("{entry_path}: `{rel}` is not a plain relative file")));
        }
        let top = rel.split('/').next().unwrap_or_default();
        if !skill_dirs().contains(&top) {
            continue;
        }
        let mut bytes = Vec::new();
        e.read_to_end(&mut bytes).map_err(|e| bad(e.to_string()))?;
        files.insert(rel, SkillFile::of(&bytes));
    }
    Ok(SkillPackage {
        package: app["name"].as_str().unwrap_or_default().to_string(),
        version: app["version"].as_str().unwrap_or_default().to_string(),
        pikg_digest: Some(pikg_digest),
        imported_from: None,
        files,
    })
}

/// Where a package file lands, and the skill it belongs to.
fn place(path: &str) -> Result<(SkillKind, String)> {
    let bad = || {
        OpenDanError::InvalidArgument(format!(
            "`{path}` is not a skill file: a skill package holds <cmd_skills|plan_skills|goal_skills>/<name>/SKILL.md and their references/…"
        ))
    };
    let parts: Vec<&str> = path.split('/').collect();
    if parts.iter().any(|p| p.is_empty() || *p == "." || *p == "..") {
        return Err(bad());
    }
    if parts[0] == TOOLS_DIR {
        return Err(OpenDanError::InvalidArgument(format!(
            "`{path}`: a skill is text only and brings no command or install script; publish the commands as an extension package and name them in the manual's `commands`"
        )));
    }
    let (kind, name) = match parts.as_slice() {
        [dir, name, rest @ ..] => {
            let kind = SkillKind::ALL.into_iter().find(|k| k.dir() == *dir).ok_or_else(bad)?;
            match rest {
                [file] if *file == SKILL_MANUAL => {}
                [refs, _, ..] if *refs == SKILL_REFERENCES => {}
                _ => return Err(bad()),
            }
            (kind, *name)
        }
        _ => return Err(bad()),
    };
    crate::ids::validate_session_id(name).map_err(|_| bad())?;
    Ok((kind, name.to_string()))
}

/// One thing a check of a skill package found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillFinding {
    pub code: String,
    /// `error` (not installable) | `block` (not on this runtime) | `high` | `medium` | `low`.
    pub level: String,
    /// The skill or file it is about.
    #[serde(rename = "where")]
    pub place: String,
    pub note: String,
}

fn finding(code: &str, level: &str, place: &str, note: impl Into<String>) -> SkillFinding {
    SkillFinding { code: code.to_string(), level: level.to_string(), place: place.to_string(), note: note.into() }
}

/// Words at a command position that are the shell's own or part of every runtime.
const SHELL_WORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac", "in", "function", "select", "time",
    "cd", "echo", "printf", "export", "set", "unset", "read", "exit", "return", "source", "eval", "exec", "test", "true", "false", "local",
    "declare", "trap", "wait", "shift", "alias", "type", "command", "pwd", "kill", "umask", "ulimit", "break", "continue", "let", "readonly",
    "ls", "cat", "cp", "mv", "rm", "mkdir", "rmdir", "touch", "ln", "chmod", "chown", "head", "tail", "grep", "sed", "awk", "sort", "uniq",
    "wc", "cut", "tr", "tee", "find", "xargs", "basename", "dirname", "date", "sleep", "env", "which", "diff", "du", "df", "stat", "mktemp",
    "sh", "bash", "id", "whoami", "uname", "realpath", "readlink", "seq", "yes", "nohup", "timeout", "tar", "gzip", "gunzip", "sha256sum",
    "base64", "cmp", "file", "ps", "less", "more", "printenv", "hostname", "expr", "od", "paste", "comm", "nl", "tac", "rev", "fold", "column",
    "read", "shell",
];

/// The commands the shell code blocks of a manual run: the first word of each command, the shell's own words and
/// placeholders left out. A heuristic for telling the owner what a manual relies on, never a gate.
pub fn commands_in_manual(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut in_block = false;
    let mut shell = false;
    let mut heredoc: Option<String> = None;
    let mut continued = false;
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(lang) = line.strip_prefix("```") {
            in_block = !in_block;
            shell = in_block && matches!(lang.trim().to_ascii_lowercase().as_str(), "" | "sh" | "bash" | "shell" | "zsh" | "console");
            heredoc = None;
            continued = false;
            continue;
        }
        if !in_block || !shell {
            continue;
        }
        if let Some(end) = &heredoc {
            if line == end {
                heredoc = None;
            }
            continue;
        }
        let was_continued = continued;
        continued = line.ends_with('\\');
        if let Some(at) = line.find("<<") {
            let tag: String = line[at + 2..].trim_start_matches('-').trim().chars().take_while(|c| !c.is_whitespace()).collect();
            let tag = tag.trim_matches(['\'', '"']).to_string();
            if !tag.is_empty() && tag.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                heredoc = Some(tag);
            }
        }
        if was_continued || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("$ ").unwrap_or(line);
        let mut quote: Option<char> = None;
        let mut segments = vec![String::new()];
        let chars: Vec<char> = line.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            match quote {
                Some(q) if c == q => quote = None,
                Some(_) => {}
                None if c == '\'' || c == '"' => quote = Some(c),
                None if c == '#' && (i == 0 || chars[i - 1].is_whitespace()) => break,
                // `a|b|c` without spaces is a list of alternatives in a usage line, not a pipe.
                None if (c == '|' && (chars.get(i + 1).is_none_or(|n| n.is_whitespace() || *n == '|') || (i > 0 && chars[i - 1].is_whitespace())))
                    || c == ';'
                    || (c == '&' && chars.get(i + 1) == Some(&'&')) =>
                {
                    if chars.get(i + 1) == Some(&c) {
                        i += 1;
                    }
                    segments.push(String::new());
                    i += 1;
                    continue;
                }
                None if c == '$' && chars.get(i + 1) == Some(&'(') => {
                    segments.push(String::new());
                    i += 2;
                    continue;
                }
                None => {}
            }
            if let Some(last) = segments.last_mut() {
                last.push(c);
            }
            i += 1;
        }
        for segment in segments {
            let word = segment
                .split_whitespace()
                .find(|w| !(w.contains('=') && w.split('=').next().is_some_and(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'))) && *w != "sudo" && *w != "(" && *w != "{" && *w != "!");
            let Some(word) = word else { continue };
            let word = word.trim_start_matches('(');
            let plain = !word.is_empty()
                && word.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
                && word.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
                && word.chars().any(|c| c.is_ascii_lowercase());
            if plain && !SHELL_WORDS.contains(&word) {
                out.insert(word.to_string());
            }
        }
    }
    out
}

struct PackageSkill<'a> {
    kind: SkillKind,
    name: String,
    meta: SkillMeta,
    files: Vec<(&'a str, &'a SkillFile)>,
}

/// The skills of a package with what makes each installable checked: layout, front matter, a manual per skill, every
/// file text. `findings` takes what is worth telling without refusing.
fn package_skills<'a>(pkg: &'a SkillPackage, findings: &mut Vec<SkillFinding>) -> Result<Vec<PackageSkill<'a>>> {
    let invalid = |m: String| OpenDanError::InvalidArgument(m);
    let mut grouped: BTreeMap<(SkillKind, String), Vec<(&str, &SkillFile)>> = BTreeMap::new();
    for (path, f) in &pkg.files {
        let (kind, name) = place(path)?;
        grouped.entry((kind, name)).or_default().push((path.as_str(), f));
    }
    if let Some((path, _)) = pkg.files.iter().find(|(_, f)| f.text().is_none()) {
        return Err(invalid(format!("{path} is not UTF-8 text: a skill is text only")));
    }
    if grouped.is_empty() {
        return Err(invalid(format!("package {} holds no skill (<cmd_skills|plan_skills|goal_skills>/<name>/SKILL.md)", pkg.package)));
    }
    let mut out = Vec::new();
    for ((kind, name), files) in grouped {
        let manual = kind.manual(&name);
        let Some(file) = pkg.files.get(&manual) else {
            return Err(invalid(format!("{}/{name}/ has no {SKILL_MANUAL}", kind.dir())));
        };
        let text = file.text().ok_or_else(|| invalid(format!("{manual} is not UTF-8 text")))?;
        let (fields, body) = split_front_matter(&text);
        check_front_matter(kind, &fields).map_err(|e| invalid(format!("{manual}: {e}")))?;
        if fields.get("name").is_some_and(|n| n != &name) {
            return Err(invalid(format!("{manual}: front matter name `{}` differs from its directory", fields["name"])));
        }
        let mut meta = skill_meta(kind, &name, &text);
        meta.references = files.iter().filter_map(|(p, _)| p.strip_prefix(&format!("{}/{name}/{SKILL_REFERENCES}/", kind.dir()))).map(str::to_string).collect();
        if meta.description.trim().is_empty() {
            findings.push(finding("no_description", "medium", &manual, "no `description`: whoever chooses manuals only sees the name"));
        }
        let texts: Vec<String> = std::iter::once(body.to_string())
            .chain(files.iter().filter(|(p, _)| p.contains(&format!("/{SKILL_REFERENCES}/")) && p.ends_with(".md")).filter_map(|(_, f)| f.text()))
            .collect();
        match kind {
            SkillKind::Work => {
                if meta.scope.ui() && meta.size > UI_MANUAL_WARN_BYTES {
                    findings.push(finding("ui_manual_large", "low", &manual, format!("{} bytes: every conversation that takes it carries them (advised: at most {UI_MANUAL_WARN_BYTES})", meta.size)));
                }
                let used: BTreeSet<String> = texts.iter().flat_map(|t| commands_in_manual(t)).collect();
                let known = |c: &String| crate::baseline::STANDARD_TOOLS.iter().any(|t| t.name == c);
                let undeclared: Vec<String> = used.iter().filter(|c| !meta.commands.contains(c) && !known(c)).cloned().collect();
                if !undeclared.is_empty() {
                    findings.push(finding("undeclared_commands", "low", &manual, format!("the manual runs {} without naming them in `commands`", undeclared.join(", "))));
                }
                let lower: Vec<String> = texts.iter().map(|t| t.to_lowercase()).collect();
                let unused: Vec<String> = meta.commands.iter().filter(|c| !lower.iter().any(|t| t.contains(&c.to_lowercase()))).cloned().collect();
                if !unused.is_empty() {
                    findings.push(finding("unused_commands", "low", &manual, format!("`commands` names {} but the manual never mentions them", unused.join(", "))));
                }
            }
            SkillKind::Plan => {
                if meta.size > PLAN_MANUAL_WARN_BYTES {
                    findings.push(finding("plan_manual_large", "low", &manual, format!("{} bytes (advised: at most {PLAN_MANUAL_WARN_BYTES}): a plan reads it whole", meta.size)));
                }
                let hard = texts.iter().any(|t| t.contains("create-worksession") || t.contains("--cmd-skill") || t.lines().any(|l| l.contains("--skill ") && !l.contains("--skill <")));
                let commands: BTreeSet<String> = texts.iter().flat_map(|t| commands_in_manual(t)).collect();
                if hard || !commands.is_empty() {
                    findings.push(finding(
                        "plan_depends_on_commands",
                        "medium",
                        &manual,
                        "a plan skill says how to split, order and check work, not which commands or manuals to use: move commands into a work skill and write \"give this step a manual for X\"",
                    ));
                }
            }
            SkillKind::Goal => {}
        }
        out.push(PackageSkill { kind, name, meta, files });
    }
    Ok(out)
}

/// Check a skill package without an AgentRoot (`pikg doctor`, authors).
pub fn lint_package(pkg: &SkillPackage) -> Vec<SkillFinding> {
    let mut findings = Vec::new();
    if let Err(e) = package_skills(pkg, &mut findings) {
        findings.insert(0, finding("invalid_package", "error", &pkg.package, e.to_string()));
    }
    findings
}

/// One skill of an install plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedSkill {
    #[serde(flatten)]
    pub meta: SkillMeta,
    /// The skill of the same name it would replace (its origin).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaces: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallRisk {
    /// `low | medium | high`.
    pub level: String,
    pub findings: Vec<SkillFinding>,
}

/// What installing a package would do, before anything is written.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallPlan {
    pub package: String,
    pub version: String,
    /// Content digest of the package's files.
    pub digest: String,
    /// Covers package, version and content: `skills.install` is given it back and refuses another package.
    pub plan_digest: String,
    pub skills: Vec<PlannedSkill>,
    pub risk: InstallRisk,
}

fn plan_digest(pkg: &SkillPackage, digest: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(format!("{}\n{}\n{digest}\n", pkg.package, pkg.version).as_bytes())))
}

/// Plan the installation of `pkg` into `root`: what it holds, what it replaces and what a reader should know before
/// saying yes. Nothing is written.
pub fn plan(root: &Path, pkg: &SkillPackage) -> Result<InstallPlan> {
    if pkg.package.trim().is_empty() {
        return Err(OpenDanError::InvalidArgument("a skill package needs its name (pkg_name)".into()));
    }
    let digest = content_digest(&pkg.files)?;
    let mut findings = Vec::new();
    let skills = package_skills(pkg, &mut findings)?;
    let existing = list(root, None)?;
    let mut planned = Vec::new();
    for skill in &skills {
        let before = existing.iter().find(|m| m.kind == skill.kind && m.name == skill.name);
        let own = before.is_some_and(|m| m.install.as_ref().is_some_and(|i| i.package == pkg.package));
        let replaces = before.filter(|_| !own).map(|m| match &m.install {
            Some(i) => format!("{} ({})", m.origin, i.package),
            None => m.origin.clone(),
        });
        if let Some(origin) = &replaces {
            findings.push(finding("replaces_skill", "medium", &key(skill.kind, &skill.name), format!("takes the place of the {origin} skill of the same name")));
        }
        if skill.meta.scope.ui() {
            findings.push(finding("ui_scope", "medium", &key(skill.kind, &skill.name), "its scope includes `ui`: once the owner adds it to the UI set it goes into the prompt of every conversation"));
        }
        for (path, file) in &skill.files {
            let Some(text) = file.text() else { continue };
            let lower = text.to_ascii_lowercase();
            let piped = lower.lines().any(|l| (l.contains("curl ") || l.contains("wget ")) && (l.contains("| sh") || l.contains("|sh") || l.contains("| bash") || l.contains("|bash")));
            if piped {
                findings.push(finding("network_script", "high", path, "tells the agent to download a script and run it (`curl … | sh`)"));
            }
            let managers: Vec<&str> = ["apt-get ", "apt install", "brew install", "npm install -g", "npm i -g", "pip install", "pip3 install", "winget install", "choco install", "dnf install", "yum install", "apk add"]
                .into_iter()
                .filter(|m| lower.contains(m))
                .collect();
            if !managers.is_empty() {
                findings.push(finding("package_manager", "medium", path, format!("uses a package manager ({})", managers.join(", ").replace("  ", " "))));
            }
            if lower.lines().any(|l| l.split_whitespace().any(|w| w == "sudo")) {
                findings.push(finding("privilege", "high", path, "asks for root (`sudo`)"));
            }
            let secrets: BTreeSet<String> = text
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                .filter(|w| w.len() > 5 && w.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
                .filter(|w| ["TOKEN", "SECRET", "PASSWORD", "API_KEY", "APIKEY", "PRIVATE_KEY"].iter().any(|s| w.contains(s)))
                .map(str::to_string)
                .collect();
            if !secrets.is_empty() {
                findings.push(finding("secrets", "medium", path, format!("reads secrets from the environment ({})", secrets.into_iter().collect::<Vec<_>>().join(", "))));
            }
        }
        let mut meta = skill.meta.clone();
        meta.origin = if pkg.imported_from.is_some() { "imported" } else { "installed" }.to_string();
        planned.push(PlannedSkill { meta, replaces });
    }
    if pkg.imported_from.is_some() {
        findings.push(finding("imported_text", "medium", &pkg.package, "written by the agent from a text the owner gave it: nobody signed it"));
    } else if pkg.pikg_digest.is_none() {
        findings.push(finding("unpacked_source", "low", &pkg.package, "a directory, not a packed PIKG: there is no package digest to compare"));
    }
    let level = ["high", "medium"].into_iter().find(|l| findings.iter().any(|f| f.level == *l)).unwrap_or("low").to_string();
    Ok(InstallPlan {
        package: pkg.package.clone(),
        version: pkg.version.clone(),
        plan_digest: plan_digest(pkg, &digest),
        digest,
        skills: planned,
        risk: InstallRisk { level, findings },
    })
}

/// What an install did.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallReport {
    pub package: String,
    pub version: String,
    /// Content digest of the installed files ([`content_digest`]).
    pub digest: String,
    pub skills: Vec<SkillMeta>,
    /// Skills of the same names it replaced (`replace`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaced: Vec<String>,
}

/// What the owner accepted (`None`: an install without a plan, as tests and the Loader's own seeding do).
#[derive(Debug, Clone, Default)]
pub struct Accepted {
    pub level: String,
    pub plan_digest: String,
}

/// Install the files of `pkg` into the AgentRoot `root`. A skill whose
/// name is taken by a skill of the agent package, a local one, or one
/// installed from another package is refused unless `replace`; installing
/// a package again updates its own skills. Sessions created before keep
/// their frozen manuals. Installing is checking and copying: nothing runs.
pub fn install(root: &Path, pkg: &SkillPackage, replace: bool, who: &str, accepted: Option<&Accepted>) -> Result<InstallReport> {
    if pkg.package.trim().is_empty() {
        return Err(OpenDanError::InvalidArgument("a skill package needs its name (pkg_name)".into()));
    }
    let digest = content_digest(&pkg.files)?;
    let skills = package_skills(pkg, &mut Vec::new())?;
    let mut record = load_record(root)?;
    let mut replaced = Vec::new();
    for skill in &skills {
        let (kind, name) = (skill.kind, &skill.name);
        let taken = root.join(kind.dir()).join(name).exists();
        let own = record.installed.get(&key(kind, name)).is_some_and(|r| r.package == pkg.package);
        if taken && !own {
            if !replace {
                let by = record
                    .installed
                    .get(&key(kind, name))
                    .map(|r| format!("installed from {}", r.package))
                    .unwrap_or_else(|| "a skill of the agent package or a local one".into());
                return Err(OpenDanError::InvalidArgument(format!(
                    "{} skill `{name}` exists already ({by}); install with replace to take its place",
                    kind.as_str()
                )));
            }
            replaced.push(key(kind, name));
        }
    }
    let now = crate::now_ms();
    let mut report = InstallReport { package: pkg.package.clone(), version: pkg.version.clone(), digest: digest.clone(), replaced, ..Default::default() };
    let mut states: Vec<(String, Option<SkillInstallState>)> = Vec::new();
    for skill in &skills {
        let (kind, name) = (skill.kind, &skill.name);
        // What the skill had before goes; a manual the agent revised keeps its earlier texts.
        let manual_dir = root.join(kind.dir()).join(name);
        for entry in std::fs::read_dir(&manual_dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path == manual_dir.join(REVISIONS_DIR) {
                continue;
            }
            let removed = if path.is_dir() { std::fs::remove_dir_all(&path) } else { std::fs::remove_file(&path) };
            removed.map_err(|e| OpenDanError::io(&path, e))?;
        }
        for (path, f) in &skill.files {
            crate::fsutil::atomic_replace(&root.join(path), &f.bytes()?)?;
        }
        record.installed.insert(
            key(kind, name),
            SkillInstallRecord {
                package: pkg.package.clone(),
                version: pkg.version.clone(),
                digest: digest.clone(),
                pikg_digest: pkg.pikg_digest.clone(),
                installed_at_ms: now,
                installed_by: who.to_string(),
                imported_from: pkg.imported_from.clone(),
            },
        );
        let risk = accepted.map(|a| SkillRiskAccepted { level: a.level.clone(), accepted_by: who.to_string(), accepted_at_ms: now, plan_digest: a.plan_digest.clone() });
        let state = risk.map(|risk| SkillInstallState { risk: Some(risk) });
        states.push((key(kind, name), state));
    }
    save_record(root, &record)?;
    update_state(root, |state| {
        for (id, install) in states {
            match install {
                Some(install) => {
                    state.install.insert(id.clone(), install);
                }
                None => {
                    state.install.remove(&id);
                }
            }
            audit(state, who, "install", &id, &format!("{} {} ({digest})", pkg.package, pkg.version));
        }
        Ok(())
    })?;
    let all = list(root, None)?;
    report.skills = all.into_iter().filter(|m| skills.iter().any(|s| s.kind == m.kind && s.name == m.name)).collect();
    Ok(report)
}

/// Remove an installed skill (its directory).
/// A skill of the agent package is not removed here: the next package sync
/// would bring it back (disable it instead).
pub fn remove(root: &Path, kind: SkillKind, name: &str, who: &str) -> Result<SkillMeta> {
    let meta = find(root, kind, name)?;
    let mut record = load_record(root)?;
    if record.installed.remove(&key(kind, name)).is_none() {
        return Err(OpenDanError::InvalidArgument(format!(
            "{} skill `{name}` was not installed ({}): only installed skills are removed; `skill disable` turns any skill off",
            kind.as_str(),
            meta.origin
        )));
    }
    let dir = root.join(kind.dir()).join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
    }
    save_record(root, &record)?;
    let id = key(kind, name);
    update_state(root, |state| {
        state.install.remove(&id);
        state.disabled.remove(&id);
        if kind == SkillKind::Work {
            if let Some(ui) = state.ui_cmd_skills.as_mut() {
                ui.retain(|n| n != name);
            }
        }
        audit(state, who, "remove", &id, "");
        Ok(())
    })?;
    Ok(meta)
}

/// A draft waiting for the owner.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkillDraft {
    pub slug: String,
    /// `draft.json`: where it came from, how the text was split, what was checked, what a reader should know.
    pub draft: serde_json::Value,
    /// Its skills as they would be listed.
    pub skills: Vec<SkillMeta>,
    pub files: Vec<String>,
    pub updated_at_ms: u64,
    /// Why it cannot be installed as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn draft_dir(root: &Path, slug: &str) -> Result<PathBuf> {
    crate::ids::validate_session_id(slug).map_err(|_| OpenDanError::InvalidArgument(format!("bad draft name `{slug}`")))?;
    Ok(root.join(DRAFTS_DIR).join(slug))
}

/// The package a draft would install as.
pub fn read_draft(root: &Path, slug: &str) -> Result<SkillPackage> {
    let dir = draft_dir(root, slug)?;
    if !dir.is_dir() {
        return Err(OpenDanError::NotFound(format!("skill draft `{slug}`")));
    }
    let meta: serde_json::Value = crate::fsutil::read_json_opt(&dir.join(DRAFT_META))?.unwrap_or_else(|| serde_json::json!({}));
    let mut pkg = read_package_dir(&dir, &format!("imported-{slug}"), "")?;
    pkg.imported_from = Some(meta.get("imported_from").filter(|v| !v.is_null()).cloned().unwrap_or_else(|| serde_json::json!({ "draft": slug })));
    Ok(pkg)
}

pub fn list_drafts(root: &Path) -> Result<Vec<SkillDraft>> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(root.join(DRAFTS_DIR)).into_iter().flatten().flatten() {
        let path = entry.path();
        let Some(slug) = entry.file_name().to_str().map(str::to_string) else { continue };
        if !path.is_dir() || slug.starts_with('.') || crate::ids::validate_session_id(&slug).is_err() {
            continue;
        }
        let files = files_under(&path);
        let updated_at_ms = files
            .iter()
            .filter_map(|f| std::fs::metadata(path.join(f)).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .max()
            .unwrap_or(0);
        let draft = crate::fsutil::read_json_opt(&path.join(DRAFT_META)).ok().flatten().unwrap_or_else(|| serde_json::json!({}));
        let (skills, error) = match read_draft(root, &slug).and_then(|pkg| package_skills(&pkg, &mut Vec::new()).map(|s| s.into_iter().map(|s| s.meta).collect::<Vec<_>>())) {
            Ok(skills) => (skills, None),
            Err(e) => (Vec::new(), Some(e.to_string())),
        };
        out.push(SkillDraft { slug, draft, skills, files, updated_at_ms, error });
    }
    out.sort_by(|a, b| b.updated_at_ms.cmp(&a.updated_at_ms).then(a.slug.cmp(&b.slug)));
    Ok(out)
}

/// One text file of a draft, for the owner to read before installing.
pub fn read_draft_file(root: &Path, slug: &str, file: &str) -> Result<String> {
    let dir = draft_dir(root, slug)?;
    if file.starts_with('/') || file.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return Err(OpenDanError::InvalidArgument(format!("bad draft file `{file}`")));
    }
    let path = dir.join(file);
    std::fs::read_to_string(&path).map_err(|e| OpenDanError::io(&path, e))
}

/// Throw a draft away (the owner declined it, or it was installed).
pub fn discard_draft(root: &Path, slug: &str, who: &str, installed: bool) -> Result<()> {
    let dir = draft_dir(root, slug)?;
    if !dir.is_dir() {
        return Err(OpenDanError::NotFound(format!("skill draft `{slug}`")));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| OpenDanError::io(&dir, e))?;
    update_state(root, |state| {
        audit(state, who, if installed { "draft_installed" } else { "draft_discard" }, slug, "");
        Ok(())
    })
}

#[cfg(test)]
mod command_tests {
    use super::*;
    use buckyos_api::{ExtToolsCatalogEntry, ExtToolsCommand, ExtToolsManifest, ExtToolsRequire};

    fn entry(id: &str, version: &str, commands: &[&str], requires: &[&str]) -> ExtToolsCatalogEntry {
        ExtToolsCatalogEntry {
            manifest: ExtToolsManifest {
                schema: buckyos_api::EXT_TOOLS_MANIFEST_SCHEMA.to_string(),
                id: id.to_string(),
                version: version.to_string(),
                requires: requires.iter().map(|r| ExtToolsRequire { id: r.to_string(), version: "*".to_string() }).collect(),
                conflicts: vec![],
                provides: commands.iter().map(|c| ExtToolsCommand { command: c.to_string(), version_probe: format!("{c} -v"), smoke: None }).collect(),
                platforms: vec!["linux-x86_64".to_string()],
                size_bytes: BTreeMap::new(),
                license: String::new(),
                desc: String::new(),
            },
            pikg_digest: format!("sha256:{id}{version}"),
            content_id: format!("file:{id}{version}"),
        }
    }

    fn found(entries: &[(&str, ProvidedBy)]) -> BTreeMap<String, ResolvedCommand> {
        entries.iter().map(|(n, by)| (n.to_string(), ResolvedCommand { path: format!("/x/{n}"), version: None, provided_by: *by, stub: false })).collect()
    }

    fn commands(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn a_skill_is_ready_from_any_source_and_says_whose_it_is() {
        let none = BTreeMap::new();
        let status = commands_status(&commands(&["ffmpeg"]), &found(&[("ffmpeg", ProvidedBy::Exttools)]), &[], &none);
        assert_eq!(status.state, SkillCommandsState::Ready);
        assert_eq!(status.commands["ffmpeg"].provided_by, Some(ProvidedBy::Exttools));
        let status = commands_status(&commands(&["ffmpeg", "jq"]), &found(&[("ffmpeg", ProvidedBy::System), ("jq", ProvidedBy::Exttools)]), &[], &none);
        assert_eq!(status.state, SkillCommandsState::ReadySystem, "the owner's own ffmpeg counts");
        let status = commands_status(&commands(&["mine"]), &found(&[("mine", ProvidedBy::Agent)]), &[], &none);
        assert_eq!(status.state, SkillCommandsState::Ready);
        let mut stubbed = found(&[("ffmpeg", ProvidedBy::Exttools)]);
        stubbed.get_mut("ffmpeg").unwrap().stub = true;
        assert_eq!(commands_status(&commands(&["ffmpeg"]), &stubbed, &[], &none).state, SkillCommandsState::Partial, "a stub is not a command");
    }

    #[test]
    fn what_is_missing_points_at_the_package_needing_the_least() {
        let none = BTreeMap::new();
        let core = entry("ext-core", "1.0.0", &["python3"], &[]);
        let media_old = entry("ext-media", "1.0.0", &["ffmpeg"], &["ext-core"]);
        let media = entry("ext-media", "1.2.0", &["ffmpeg", "ffprobe"], &["ext-core"]);
        let standalone = entry("ext-ffmpeg", "3.0.0", &["ffmpeg"], &[]);
        let catalog = vec![core.clone(), media_old, media];
        let status = commands_status(&commands(&["ffmpeg", "ffprobe", "blender"]), &BTreeMap::new(), &catalog, &none);
        assert_eq!(status.state, SkillCommandsState::Partial);
        assert_eq!(status.recommended_packages.len(), 1);
        let package = &status.recommended_packages[0];
        assert_eq!((package.id.as_str(), package.version.as_str()), ("ext-media", "1.2.0"), "the newest version");
        assert_eq!(package.requires_chain, vec!["ext-core".to_string()]);
        assert_eq!(package.provides, commands(&["ffmpeg", "ffprobe"]));
        assert_eq!(status.no_package, commands(&["blender"]), "no package has it: the owner brings it");

        let mut with_standalone = catalog.clone();
        with_standalone.push(standalone);
        let status = commands_status(&commands(&["ffmpeg"]), &BTreeMap::new(), &with_standalone, &none);
        assert_eq!(status.recommended_packages[0].id, "ext-ffmpeg", "nothing else to enable");
    }

    #[test]
    fn a_manual_names_its_commands_in_its_front_matter() {
        let meta = skill_meta(SkillKind::Work, "x", "---\nname: x\ncommands: ffmpeg, ffprobe , ../evil, magick\n---\nbody\n");
        assert_eq!(meta.commands, commands(&["ffmpeg", "ffprobe", "magick"]));
        assert!(skill_meta(SkillKind::Work, "x", "---\nname: x\n---\nbody\n").commands.is_empty());
    }

    #[test]
    fn only_a_command_a_check_looked_for_is_known_to_be_missing() {
        let mut caps = RuntimeCaps::default();
        assert!(known_missing_commands(&commands(&["ffmpeg"]), &caps).is_empty(), "no check yet");
        caps.baseline_check = Some(BaselineCheck { checked_at_ms: 1, level: buckyos_api::RuntimeBaseline::Standard, ok: true, missing: vec![], unmanaged: vec![], resolved: found(&[("jq", ProvidedBy::System)]), ext: None, error: None });
        assert_eq!(known_missing_commands(&commands(&["ffmpeg", "jq", "blender"]), &caps), commands(&["ffmpeg"]), "ffmpeg was looked for, blender was not");
        caps.tools_check = Some(ToolsCheck { checked_at_ms: 1, present: vec![], missing: vec![], resolved: found(&[("blender", ProvidedBy::System)]), error: None });
        assert_eq!(known_missing_commands(&commands(&["blender"]), &caps), Vec::<String>::new());
    }

    #[test]
    fn a_skill_is_text_only() {
        let root = std::env::temp_dir().join(format!("skills-text-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut pkg = SkillPackage { package: "p".to_string(), version: "1".to_string(), ..Default::default() };
        pkg.files.insert("cmd_skills/s/SKILL.md".to_string(), SkillFile::of(b"---\nname: s\n---\nbody\n"));
        for path in ["tools/s/bin/mytool", "tools/s/install.toml"] {
            pkg.files.insert(path.to_string(), SkillFile::of(b"#!/bin/sh\n"));
            let error = install(&root, &pkg, false, "t", None).unwrap_err().to_string();
            assert!(error.contains("text only") && error.contains("extension package"), "{error}");
            assert_eq!(lint_package(&pkg)[0].code, "invalid_package");
            pkg.files.remove(path);
        }
        pkg.files.insert("cmd_skills/s/references/logo.bin".to_string(), SkillFile::of(&[0xff, 0xfe, 0x00]));
        assert!(install(&root, &pkg, false, "t", None).unwrap_err().to_string().contains("not UTF-8 text"));
        pkg.files.remove("cmd_skills/s/references/logo.bin");
        assert!(install(&root, &pkg, false, "t", None).is_ok());
        assert!(!root.join("tools").exists());
        let _ = std::fs::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(text: &str) -> SkillFile {
        SkillFile::of(text.as_bytes())
    }

    fn pkg(name: &str, files: &[(&str, &str)]) -> SkillPackage {
        SkillPackage {
            package: name.into(),
            version: "0.1.0".into(),
            pikg_digest: None,
            imported_from: None,
            files: files.iter().map(|(p, t)| (p.to_string(), file(t))).collect(),
        }
    }

    fn write(root: &Path, rel: &str, text: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn front_matter_is_metadata_not_manual() {
        let (fields, body) = split_front_matter("---\nname: x\ndescription: \"Do x\"\nrequires: y\n---\n# X\nbody\n");
        assert_eq!(fields.get("description").map(String::as_str), Some("Do x"));
        assert_eq!(fields.get("requires").map(String::as_str), Some("y"));
        assert_eq!(body, "# X\nbody");
        assert_eq!(split_front_matter("plain\n").1, "plain");
        assert_eq!(split_front_matter("---\nno end").1, "---\nno end");
    }

    #[test]
    fn install_list_remove() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "cmd_skills/ui_core/SKILL.md", "---\ndescription: chat\nscope: ui\n---\nbody");
        write(root, "cmd_skills/flat.md", "---\ndescription: the flat layout is gone\n---\nbody");
        let p = pkg(
            "skills-demo",
            &[
                ("cmd_skills/demo/SKILL.md", "---\ndescription: demo things\nrequires: nothing\n---\nRun `demo`. See references/more.md."),
                ("cmd_skills/demo/references/more.md", "more"),
                ("goal_skills/watch/SKILL.md", "---\ndescription: watching\n---\nWatch."),
                ("plan_skills/split/SKILL.md", "---\ndescription: splitting\n---\nSplit by risk."),
            ],
        );
        let r = install(root, &p, false, "did:bns:owner", None).unwrap();
        assert_eq!(r.skills.len(), 3);
        let work = list(root, Some(SkillKind::Work)).unwrap();
        assert!(work.iter().all(|m| m.name != "flat"), "cmd_skills/<name>.md is not a skill");
        let demo = work.iter().find(|m| m.name == "demo").unwrap();
        assert_eq!(demo.origin, "installed");
        assert_eq!(demo.description, "demo things");
        assert_eq!(demo.references, vec!["more.md".to_string()]);
        assert!(demo.enabled && demo.install_state.is_none(), "installed without a plan: nothing was accepted");
        let ui = work.iter().find(|m| m.name == "ui_core").unwrap();
        assert_eq!((ui.origin.as_str(), ui.scope), ("local", SkillScope::Ui));
        assert_eq!(list(root, Some(SkillKind::Plan)).unwrap()[0].name, "split");
        assert_eq!(serde_json::to_value(demo).unwrap()["kind"], "work", "the kind is called work outside");
        assert!(installed_paths(root).unwrap().contains(&"cmd_skills/demo/".to_string()));
        assert!(installed_paths(root).unwrap().contains(&"plan_skills/split/".to_string()));

        // A name taken by another skill is refused unless replaced.
        let other = pkg("other", &[("cmd_skills/ui_core/SKILL.md", "x")]);
        assert!(install(root, &other, false, "o", None).is_err());
        assert_eq!(install(root, &other, true, "o", None).unwrap().replaced, vec!["work:ui_core".to_string()]);
        // The same package again updates its own skills.
        install(root, &p, false, "o", None).unwrap();

        for (bad, why) in [
            (pkg("bad", &[("tools/zzz/bin/x", "x")]), "commands"),
            (pkg("bad", &[("cmd_skills/../role.md", "x")]), "traversal"),
            (pkg("bad", &[("cmd_skills/flat.md", "x")]), "the flat layout"),
            (pkg("bad", &[("cmd_skills/a/notes.md", "x"), ("cmd_skills/a/SKILL.md", "x")]), "a file that is neither the manual nor a reference"),
            (pkg("bad", &[("plan_skills/p/SKILL.md", "---\ncommands: jq\n---\nx")]), "a plan skill naming commands"),
            (pkg("bad", &[("cmd_skills/a/SKILL.md", "---\nscope: everywhere\n---\nx")]), "a scope that is no fixed word"),
        ] {
            assert!(install(root, &bad, false, "o", None).is_err(), "{why}");
        }

        remove(root, SkillKind::Work, "demo", "o").unwrap();
        assert!(!root.join("cmd_skills/demo").exists());
        assert!(list(root, Some(SkillKind::Work)).unwrap().iter().all(|m| m.name != "demo"));
        assert!(load_state(root).unwrap().install.get("work:demo").is_none());
        assert!(remove(root, SkillKind::Work, "nope", "o").is_err());
        write(root, "cmd_skills/local/SKILL.md", "x");
        assert!(remove(root, SkillKind::Work, "local", "o").is_err(), "only installed skills are removed");
    }

    #[test]
    fn the_owner_disables_any_skill_and_sets_the_ui_manuals() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "agent.toml", "[session.ui]\ndefault_cmd_skills = [\"ui_core\"]\n");
        write(root, "cmd_skills/ui_core/SKILL.md", "---\nscope: ui\n---\n12345");
        write(root, "cmd_skills/share/SKILL.md", "---\nscope: ui, work\n---\n123");
        write(root, "cmd_skills/video/SKILL.md", "---\ndescription: v\n---\nbody");
        let ui = ui_skills(root).unwrap();
        assert_eq!((ui.names.clone(), ui.source.as_str(), ui.total_bytes), (vec!["ui_core".to_string()], "template", 5));
        assert_eq!(ui.available, vec!["share".to_string()]);
        assert!(set_ui_skills(root, Some(vec!["ui_core".into(), "video".into()]), "owner").unwrap_err().to_string().contains("scope"));
        let ui = set_ui_skills(root, Some(vec!["ui_core".into(), "share".into()]), "owner").unwrap();
        assert_eq!((ui.source.as_str(), ui.total_bytes), ("state", 8));
        assert_eq!(crate::template::SessionTemplate::load("ui", Some(root)).unwrap().default_cmd_skills, ui.names, "new conversations take the owner's set");
        assert_eq!(crate::template::SessionTemplate::load("work", Some(root)).unwrap().default_cmd_skills, Vec::<String>::new());

        let off = set_enabled(root, SkillKind::Work, "share", false, Some("no provider"), "did:bns:owner").unwrap();
        assert!(!off.enabled);
        assert_eq!(off.why_off().unwrap(), "disabled by owner: no provider");
        assert!(set_ui_skills(root, Some(vec!["share".into()]), "owner").is_err(), "a disabled manual is not added");
        assert!(set_enabled(root, SkillKind::Work, "share", true, None, "o").unwrap().enabled);
        assert_eq!(set_ui_skills(root, None, "owner").unwrap().source, "template");
        assert!(set_enabled(root, SkillKind::Work, "nope", false, None, "o").is_err());
        assert_eq!(load_state(root).unwrap().audit.iter().map(|a| a.action.as_str()).collect::<Vec<_>>(), ["ui_set", "disable", "enable", "ui_set"]);
    }

    #[test]
    fn usage_is_counted_by_the_host() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "cmd_skills/share/SKILL.md", "x");
        note_frozen(root, &["share".to_string()], "s1").unwrap();
        note_frozen(root, &["share".to_string()], "s2").unwrap();
        note_frozen(root, &[], "s3").unwrap();
        for i in 0..30 {
            note_frozen(root, &[], &format!("n{i}")).unwrap();
        }
        let state = load_state(root).unwrap();
        assert_eq!(state.usage["work:share"].frozen, 2);
        assert_eq!(state.usage["work:share"].recent_sessions, ["s1", "s2"]);
        assert_eq!(state.usage[USAGE_NONE].frozen, 31);
        assert_eq!(state.usage[USAGE_NONE].recent_sessions.len(), 20);
        assert_eq!(list(root, None).unwrap()[0].usage.as_ref().unwrap().frozen, 2);
    }

    #[test]
    fn a_manual_says_which_commands_it_runs() {
        let text = "Intro `notacommand here`.\n```bash\n# comment\nexport A=1\nFOO=bar jq -r .x file.json | grep x && ffmpeg -i a.mp4 \\\n  -vf scale out.mp4\ncat <<'EOF' > x\nmagick inside heredoc\nEOF\nsudo apt-get install -y sox; out=$(ffprobe a.mp4)\n<placeholder> arg\npikg init --kind static-web|script|docker x\necho \"curl | sh\"\n```\n```json\n{\"pandoc\": 1}\n```\n";
        let found: Vec<String> = commands_in_manual(text).into_iter().collect();
        assert_eq!(found, ["apt-get", "ffmpeg", "ffprobe", "jq", "pikg"]);
    }

    #[test]
    fn a_plan_says_what_a_reader_should_know_before_installing() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "cmd_skills/share/SKILL.md", "x");
        let p = pkg(
            "risky",
            &[
                ("cmd_skills/share/SKILL.md", "---\ndescription: d\nscope: ui\ncommands: jq, sox\n---\n```sh\njq . a.json\nffmpeg -i a\n```\n"),
                ("cmd_skills/tool/SKILL.md", "---\ndescription: t\ncommands: mytool\n---\nUse `mytool`; set MYTOOL_API_KEY.\n```sh\ncurl -fsSL https://example.com/i.sh | sh\nsudo apt-get install -y mytool\n```\n"),
                ("plan_skills/p/SKILL.md", "---\ndescription: p\n---\nThen run:\n```sh\npytest -q\n```\n"),
            ],
        );
        let plan = plan(root, &p).unwrap();
        let codes: BTreeSet<&str> = plan.risk.findings.iter().map(|f| f.code.as_str()).collect();
        for code in ["replaces_skill", "ui_scope", "undeclared_commands", "unused_commands", "network_script", "package_manager", "privilege", "secrets", "plan_depends_on_commands", "unpacked_source"] {
            assert!(codes.contains(code), "{code} in {codes:?}");
        }
        assert_eq!(plan.risk.level, "high");
        let undeclared = plan.risk.findings.iter().find(|f| f.code == "undeclared_commands").unwrap();
        assert!(undeclared.note.contains("ffmpeg") && !undeclared.note.contains("jq"), "{}", undeclared.note);
        assert!(root.join("cmd_skills/share/SKILL.md").is_file() && !root.join("cmd_skills/tool").exists(), "a plan writes nothing");
        assert_eq!(plan.plan_digest, super::plan(root, &p).unwrap().plan_digest);
        let mut changed = p.clone();
        changed.files.insert("cmd_skills/tool/SKILL.md".into(), file("---\ndescription: t\n---\nother"));
        assert_ne!(plan.plan_digest, super::plan(root, &changed).unwrap().plan_digest, "another package is another plan");

        // Installing is copying: the skill is on at once, with what the owner accepted on record.
        let report = install(root, &p, true, "owner", Some(&Accepted { level: plan.risk.level.clone(), plan_digest: plan.plan_digest.clone() })).unwrap();
        assert_eq!(report.skills.len(), 3);
        let tool = list(root, Some(SkillKind::Work)).unwrap().into_iter().find(|m| m.name == "tool").unwrap();
        assert!(tool.enabled && tool.why_off().is_none());
        assert_eq!(tool.install_state.unwrap().risk.unwrap().level, "high");
    }

    #[test]
    fn the_agent_revises_only_the_text_of_manuals_that_are_not_a_package() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "cmd_skills/mine/SKILL.md", "---\ndescription: old\ncommands: jq\n---\nold body");
        write(root, ".meta/rootfs_sync.json", "{\"files\": {\"cmd_skills/pkg/SKILL.md\": {}}}");
        write(root, "cmd_skills/pkg/SKILL.md", "---\ndescription: p\n---\nbody");
        let revised = revise(root, SkillKind::Work, "mine", "---\ndescription: new\ncommands: jq\n---\nnew body", "the old steps failed twice", "session:s1").unwrap();
        assert_eq!(revised.description, "new");
        let kept: Vec<_> = std::fs::read_dir(root.join("cmd_skills/mine/revisions")).unwrap().flatten().collect();
        assert_eq!(kept.len(), 1);
        assert!(std::fs::read_to_string(kept[0].path()).unwrap().contains("old body"));
        assert!(revise(root, SkillKind::Work, "mine", "---\ndescription: new\ncommands: jq, curl\n---\nb", "r", "s").unwrap_err().to_string().contains("commands"));
        assert!(revise(root, SkillKind::Work, "mine", "---\ndescription: new\ncommands: jq\nscope: ui\n---\nb", "r", "s").is_err());
        assert!(revise(root, SkillKind::Work, "pkg", "---\ndescription: p\n---\nnew", "r", "s").unwrap_err().to_string().contains("draft"));
        assert_eq!(load_state(root).unwrap().audit.last().unwrap().action, "revise");
        assert_eq!(list(root, Some(SkillKind::Work)).unwrap().len(), 2, "revisions are not skills");
    }

    #[test]
    fn a_draft_is_installed_by_the_owner_as_imported() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        write(root, "skills_drafts/howto/cmd_skills/howto/SKILL.md", "---\ndescription: how to\nplatforms: linux\n---\nSteps.");
        write(root, "skills_drafts/howto/draft.json", "{\"imported_from\": {\"digest\": \"sha256:1\", \"runtime_id\": \"private\", \"platform\": \"linux\"}, \"risk\": \"reads files\"}");
        write(root, "skills_drafts/broken/cmd_skills/x.md", "flat");
        write(root, "skills_drafts/.inbox/1/source.md", "text");
        let drafts = list_drafts(root).unwrap();
        assert_eq!(drafts.len(), 2);
        let howto = drafts.iter().find(|d| d.slug == "howto").unwrap();
        assert_eq!(howto.skills[0].name, "howto");
        assert_eq!(howto.draft["risk"], "reads files");
        assert!(drafts.iter().find(|d| d.slug == "broken").unwrap().error.is_some());
        assert!(list(root, None).unwrap().is_empty(), "a draft is no skill");
        assert!(read_draft_file(root, "howto", "cmd_skills/howto/SKILL.md").unwrap().contains("Steps."));
        assert!(read_draft_file(root, "howto", "../../agent.toml").is_err());

        let pkg = read_draft(root, "howto").unwrap();
        assert_eq!(pkg.package, "imported-howto");
        let plan = plan(root, &pkg).unwrap();
        assert!(plan.risk.findings.iter().any(|f| f.code == "imported_text"));
        install(root, &pkg, false, "did:bns:owner", None).unwrap();
        discard_draft(root, "howto", "did:bns:owner", true).unwrap();
        let skill = &list(root, None).unwrap()[0];
        assert_eq!(skill.origin, "imported");
        assert_eq!(skill.install.as_ref().unwrap().imported_from.as_ref().unwrap()["platform"], "linux");
        assert_eq!(list_drafts(root).unwrap().len(), 1);
        assert!(discard_draft(root, "howto", "o", false).is_err());
    }
}
