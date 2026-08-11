//! §9.2 — Tool plan loader + Session Exec Bin renderer.
//!
//! A *tool plan* is a per-Agent TOML file under
//! `<agent_root>/tool_plans/<name>.toml` that describes which lower-layer
//! tools should be visible inside a session. Behavior configs reference
//! a plan by name via `BehaviorCfg.tool_plan = "<name>"`; the renderer
//! materializes the plan into the Session Exec Bin layer as tombstone stub
//! files (shebang scripts that print a reason and `exit 127`).
//!
//! The renderer also performs the *Agent tools sync*: it hard-links (or
//! copies as a fallback) every executable from `<agent_root>/tools/` into
//! the Session Exec Bin so that the AgentSession's tmux pane sees the same
//! tool surface the agent author maintains, without exposing the persistent
//! Agent tools dir to mutation. A cheap mtime walk runs at the head of
//! every `exec_bash` call so live edits to `<agent_root>/tools/` show up
//! in the next LLM step.

use std::collections::{BTreeMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::behavior_cfg::BehaviorCfgError;

/// Plan mode: `Deny` (default) blocks the explicitly listed tools;
/// `Allow` blocks every lower-layer tool that's not in the allow list.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    #[default]
    Deny,
    Allow,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolPlanEntry {
    pub name: String,
    #[serde(default)]
    pub reason: String,
}

/// On-disk tool plan. Schema matches §9.2's "已决设计要点 #1".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPlanToml {
    pub mode: PlanMode,
    pub deny: Vec<ToolPlanEntry>,
    pub allow: Vec<ToolPlanEntry>,
}

impl ToolPlanToml {
    pub fn from_str(s: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(s)
    }

    pub fn load_from_file(path: &Path) -> Result<Self, BehaviorCfgError> {
        let bytes = std::fs::read_to_string(path).map_err(|err| BehaviorCfgError::Io {
            path: path.display().to_string(),
            err,
        })?;
        toml::from_str(&bytes).map_err(|err| BehaviorCfgError::Parse {
            path: path.display().to_string(),
            err,
        })
    }
}

/// Resolved view of a plan after expanding `Allow` mode against the
/// observed lower-layer tool set. Serialized to
/// `<session_dir>/tool_plan.resolved.toml` for operator audit.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ResolvedToolPlan {
    pub plan_name: String,
    pub mode: String,
    pub tombstones: Vec<ResolvedTombstone>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ResolvedTombstone {
    pub tool: String,
    pub reason: String,
}

impl ResolvedToolPlan {
    /// Resolve a plan into the final tombstone set against the observed
    /// universe of lower-layer tool names.
    ///
    /// `universe` is the union of executable basenames found in the
    /// Agent / Runtime / System bin layers — the names the user would
    /// otherwise see on PATH. Tombstones are *only* written for names
    /// in the universe; we don't fabricate stubs for names that nothing
    /// could provide in the first place (keeps the Session Exec Bin
    /// uncluttered).
    pub fn resolve(plan_name: &str, plan: &ToolPlanToml, universe: &HashSet<String>) -> Self {
        let mut tombstones = match plan.mode {
            PlanMode::Deny => plan
                .deny
                .iter()
                .filter(|e| !e.name.trim().is_empty())
                .filter(|e| universe.contains(e.name.as_str()))
                .map(|e| ResolvedTombstone {
                    tool: e.name.clone(),
                    reason: if e.reason.trim().is_empty() {
                        format!("blocked by tool plan `{plan_name}`")
                    } else {
                        e.reason.clone()
                    },
                })
                .collect::<Vec<_>>(),
            PlanMode::Allow => {
                let allowed: HashSet<&str> = plan
                    .allow
                    .iter()
                    .map(|e| e.name.as_str())
                    .filter(|s| !s.is_empty())
                    .collect();
                let mut out = Vec::new();
                for name in universe {
                    if allowed.contains(name.as_str()) {
                        continue;
                    }
                    out.push(ResolvedTombstone {
                        tool: name.clone(),
                        reason: format!("not in allow list of tool plan `{plan_name}`"),
                    });
                }
                out
            }
        };
        tombstones.sort_by(|a, b| a.tool.cmp(&b.tool));
        tombstones.dedup_by(|a, b| a.tool == b.tool);
        Self {
            plan_name: plan_name.to_string(),
            mode: match plan.mode {
                PlanMode::Deny => "deny",
                PlanMode::Allow => "allow",
            }
            .to_string(),
            tombstones,
        }
    }

    /// Names of tools that need a tombstone stub.
    pub fn tombstone_names(&self) -> impl Iterator<Item = &str> {
        self.tombstones.iter().map(|t| t.tool.as_str())
    }
}

/// Scan the given bin directories (top-level + one subdirectory level, per
/// the §9.2 hot-path convention) and return the union of executable
/// basenames found. Used to build the `universe` argument to
/// [`ResolvedToolPlan::resolve`].
pub fn scan_bin_universe<I, P>(dirs: I) -> HashSet<String>
where
    I: IntoIterator<Item = P>,
    P: AsRef<Path>,
{
    let mut out = HashSet::new();
    for dir in dirs {
        let dir = dir.as_ref();
        if !dir.is_dir() {
            continue;
        }
        collect_executables(dir, &mut out);
        if let Ok(rd) = std::fs::read_dir(dir) {
            for ent in rd.flatten() {
                let path = ent.path();
                if path.is_dir() {
                    collect_executables(&path, &mut out);
                }
            }
        }
    }
    out
}

fn collect_executables(dir: &Path, out: &mut HashSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for ent in rd.flatten() {
        let path = ent.path();
        let Ok(meta) = ent.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        if !is_probably_executable(&meta, &path) {
            continue;
        }
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            out.insert(name.to_string());
        }
    }
}

/// Write a tombstone stub at `<session_bin>/<tool>` per §9.2 design point #3.
/// Shebang script, dual-line stderr (JSON + human), `exit 127`.
pub fn write_tombstone(
    session_bin: &Path,
    plan_name: &str,
    tombstone: &ResolvedTombstone,
) -> io::Result<()> {
    let path = session_bin.join(&tombstone.tool);
    let escaped_reason = tombstone.reason.replace('"', "\\\"");
    let escaped_plan = plan_name.replace('"', "\\\"");
    let escaped_tool = tombstone.tool.replace('"', "\\\"");
    let script = format!(
        "#!/bin/sh\n# auto-generated by opendan tool plan renderer\n\
         echo '{{\"blocked_by\":\"tool_plan\",\"tool\":\"{tool}\",\"reason\":\"{reason}\",\"plan\":\"{plan}\"}}' >&2\n\
         echo '{tool} is blocked by tool plan: {reason}' >&2\n\
         exit 127\n",
        tool = escaped_tool,
        reason = escaped_reason,
        plan = escaped_plan,
    );
    // Remove any pre-existing entry first so we don't try to overwrite a
    // hard-linked Agent tool with a tombstone (the stub must win on PATH).
    let _ = std::fs::remove_file(&path);
    std::fs::write(&path, script)?;
    set_executable(&path)?;
    Ok(())
}

#[cfg(unix)]
fn set_executable(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Snapshot of the Agent tools tree taken during a sync pass.
#[derive(Debug, Default, Clone)]
struct AgentToolSnapshot {
    /// Map of basename → source path under `<agent_root>/tools/`.
    entries: BTreeMap<String, PathBuf>,
    /// Max mtime (ns since UNIX epoch) observed during the walk. Used as the
    /// cheap dirty bit for the next sync pass.
    max_mtime_ns: u64,
}

/// Walk `agent_tools` (top level + one subdirectory level, per §9.2 design
/// point #2 "hot path" convention) and return the discovered executable
/// entries + max mtime.
fn snapshot_agent_tools(agent_tools: &Path) -> AgentToolSnapshot {
    let mut snap = AgentToolSnapshot::default();
    if !agent_tools.is_dir() {
        return snap;
    }
    let mut max_mtime_ns: u64 = 0;
    let walk = |dir: &Path, snap: &mut AgentToolSnapshot, max: &mut u64| {
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for ent in rd.flatten() {
            let path = ent.path();
            let Ok(meta) = ent.metadata() else { continue };
            if let Ok(mtime) = meta.modified() {
                let ns = mtime
                    .duration_since(SystemTime::UNIX_EPOCH)
                    .map(|d| d.as_nanos() as u64)
                    .unwrap_or(0);
                if ns > *max {
                    *max = ns;
                }
            }
            if meta.is_file() && is_probably_executable(&meta, &path) {
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                snap.entries.insert(name.to_string(), path.clone());
            }
        }
    };
    walk(agent_tools, &mut snap, &mut max_mtime_ns);
    // One subdir level: pick up `<agent_root>/tools/<bucket>/*` so authors
    // can organize without forcing flat layout. Subdir name is dropped on
    // the way down — collisions are last-writer-wins (BTreeMap insertion).
    if let Ok(rd) = std::fs::read_dir(agent_tools) {
        for ent in rd.flatten() {
            let path = ent.path();
            let Ok(meta) = ent.metadata() else { continue };
            if meta.is_dir() {
                walk(&path, &mut snap, &mut max_mtime_ns);
            }
        }
    }
    snap.max_mtime_ns = max_mtime_ns;
    snap
}

#[cfg(unix)]
fn is_probably_executable(meta: &std::fs::Metadata, _path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn is_probably_executable(_meta: &std::fs::Metadata, path: &Path) -> bool {
    path.extension()
        .and_then(|s| s.to_str())
        .map(|ext| matches!(ext, "exe" | "bat" | "cmd" | "ps1"))
        .unwrap_or(false)
}

/// Try `std::fs::hard_link(src, dst)`; fall back to `copy` when the link
/// fails (cross-filesystem, non-Unix, etc.). §9.2 design point #2 "拷贝形式".
fn link_or_copy(src: &Path, dst: &Path) -> io::Result<()> {
    let _ = std::fs::remove_file(dst);
    match std::fs::hard_link(src, dst) {
        Ok(()) => Ok(()),
        Err(_) => {
            std::fs::copy(src, dst).map(|_| ())?;
            set_executable(dst)?;
            Ok(())
        }
    }
}

/// Stateful renderer driving the Session Exec Bin layer.
///
/// Constructed once per AgentSession (at session boot or restore), held by
/// the tmux runner, and consulted on every `exec_bash` call to keep Agent
/// tools in sync. The renderer owns the *list of currently-linked Agent
/// tool basenames* so it can also remove stale entries when tools are
/// deleted from the Agent tools directory.
pub struct SessionBinRenderer {
    session_bin: PathBuf,
    agent_tools: PathBuf,
    plan_name: String,
    resolved: ResolvedToolPlan,
    /// Highest mtime observed during the last sync pass; the next call
    /// short-circuits when nothing under `agent_tools` is newer.
    last_sync_mtime_ns: AtomicU64,
    /// Basenames currently linked from `agent_tools` into `session_bin`.
    /// Used to remove stale entries when an Agent tool is deleted.
    linked: Mutex<HashSet<String>>,
}

impl SessionBinRenderer {
    pub fn new(
        session_bin: impl Into<PathBuf>,
        agent_tools: impl Into<PathBuf>,
        plan_name: String,
        resolved: ResolvedToolPlan,
    ) -> Self {
        Self {
            session_bin: session_bin.into(),
            agent_tools: agent_tools.into(),
            plan_name,
            resolved,
            last_sync_mtime_ns: AtomicU64::new(0),
            linked: Mutex::new(HashSet::new()),
        }
    }

    pub fn plan_name(&self) -> &str {
        &self.plan_name
    }

    pub fn resolved(&self) -> &ResolvedToolPlan {
        &self.resolved
    }

    /// Initial render run at session boot. Creates the session bin dir,
    /// hard-links Agent tools into it, writes tombstones, and dumps the
    /// resolved plan to `<session_dir>/tool_plan.resolved.toml`.
    pub fn render_initial(&self, session_dir: &Path) -> io::Result<()> {
        std::fs::create_dir_all(&self.session_bin)?;
        // Force an Agent tools sync regardless of cached mtime so the
        // session starts in a clean state.
        self.last_sync_mtime_ns.store(0, Ordering::Relaxed);
        self.sync_agent_tools(true)?;
        self.write_tombstones()?;
        self.dump_resolved_plan(session_dir)?;
        Ok(())
    }

    /// Per-`exec_bash` opportunistic sync. Cheap mtime walk; only re-links
    /// when something under `agent_tools` changed since the last pass.
    /// Tombstones are re-applied on top in case a stale Agent tool with
    /// the same name was just written (last-writer-wins on the session
    /// bin slot).
    pub fn maybe_resync(&self) -> io::Result<()> {
        let snap = snapshot_agent_tools(&self.agent_tools);
        let prev = self.last_sync_mtime_ns.load(Ordering::Relaxed);
        if snap.max_mtime_ns <= prev && !snap.entries.is_empty() {
            return Ok(());
        }
        self.apply_snapshot(snap)?;
        self.write_tombstones()?;
        Ok(())
    }

    fn sync_agent_tools(&self, force: bool) -> io::Result<()> {
        let snap = snapshot_agent_tools(&self.agent_tools);
        if !force {
            let prev = self.last_sync_mtime_ns.load(Ordering::Relaxed);
            if snap.max_mtime_ns <= prev {
                return Ok(());
            }
        }
        self.apply_snapshot(snap)
    }

    fn apply_snapshot(&self, snap: AgentToolSnapshot) -> io::Result<()> {
        let tombstone_names: HashSet<&str> = self.resolved.tombstone_names().collect();
        let mut next_linked: HashSet<String> = HashSet::with_capacity(snap.entries.len());
        for (name, src) in &snap.entries {
            // Don't waste an inode on a name we're about to overwrite with a
            // tombstone — the stub wins anyway and `write_tombstones` will
            // recreate it after this loop.
            if tombstone_names.contains(name.as_str()) {
                continue;
            }
            let dst = self.session_bin.join(name);
            if let Err(err) = link_or_copy(src, &dst) {
                // Don't bail on a single bad tool — log and continue so a
                // session-wide bash run can still succeed.
                log::warn!(
                    "opendan.tool_plan: link `{}` -> `{}` failed: {err}",
                    src.display(),
                    dst.display(),
                );
                continue;
            }
            next_linked.insert(name.clone());
        }
        // Remove entries that disappeared from `agent_tools` since the
        // previous sync (and aren't tombstones / unknown files).
        let mut linked = self.linked.lock().expect("linked mutex poisoned");
        for name in linked.iter() {
            if next_linked.contains(name) || tombstone_names.contains(name.as_str()) {
                continue;
            }
            let _ = std::fs::remove_file(self.session_bin.join(name));
        }
        *linked = next_linked;
        self.last_sync_mtime_ns
            .store(snap.max_mtime_ns, Ordering::Relaxed);
        Ok(())
    }

    fn write_tombstones(&self) -> io::Result<()> {
        for stone in &self.resolved.tombstones {
            write_tombstone(&self.session_bin, &self.plan_name, stone)?;
        }
        Ok(())
    }

    fn dump_resolved_plan(&self, session_dir: &Path) -> io::Result<()> {
        let path = session_dir.join("tool_plan.resolved.toml");
        let toml = toml::to_string_pretty(&self.resolved).map_err(|err| {
            io::Error::new(
                io::ErrorKind::Other,
                format!("serialize resolved tool plan: {err}"),
            )
        })?;
        std::fs::write(path, toml)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn universe(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plan_deny_mode_picks_only_universe_intersect() {
        let plan = ToolPlanToml::from_str(
            r#"
mode = "deny"
[[deny]]
name = "rm"
reason = "use trash-cli"
[[deny]]
name = "ghost"
            "#,
        )
        .unwrap();
        let u = universe(&["rm", "cat", "ls"]);
        let resolved = ResolvedToolPlan::resolve("safe", &plan, &u);
        assert_eq!(resolved.tombstones.len(), 1);
        assert_eq!(resolved.tombstones[0].tool, "rm");
        assert_eq!(resolved.tombstones[0].reason, "use trash-cli");
    }

    #[test]
    fn plan_allow_mode_tombstones_everything_else() {
        let plan = ToolPlanToml::from_str(
            r#"
mode = "allow"
[[allow]]
name = "ls"
[[allow]]
name = "cat"
            "#,
        )
        .unwrap();
        let u = universe(&["rm", "cat", "ls", "ffmpeg"]);
        let resolved = ResolvedToolPlan::resolve("worker_safe", &plan, &u);
        let names: Vec<_> = resolved
            .tombstones
            .iter()
            .map(|t| t.tool.as_str())
            .collect();
        assert_eq!(names, vec!["ffmpeg", "rm"]);
    }

    #[test]
    fn write_tombstone_creates_executable_script() {
        let dir = tempdir().unwrap();
        let bin = dir.path().join("session_bin");
        std::fs::create_dir_all(&bin).unwrap();
        let stone = ResolvedTombstone {
            tool: "rm".to_string(),
            reason: "use trash-cli".to_string(),
        };
        write_tombstone(&bin, "safe", &stone).unwrap();
        let path = bin.join("rm");
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.starts_with("#!/bin/sh"));
        assert!(body.contains("exit 127"));
        assert!(body.contains("\"plan\":\"safe\""));
        assert!(body.contains("\"tool\":\"rm\""));
    }

    #[test]
    fn render_initial_links_agent_tools_and_writes_resolved_plan() {
        let dir = tempdir().unwrap();
        let agent_tools = dir.path().join("agent_tools");
        std::fs::create_dir_all(&agent_tools).unwrap();
        // Drop an "executable" tool into agent tools.
        let agent_tool = agent_tools.join("hello");
        std::fs::write(&agent_tool, "#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&agent_tool).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&agent_tool, perms).unwrap();
        }

        let session_dir = dir.path().join("session");
        std::fs::create_dir_all(&session_dir).unwrap();
        let session_bin = session_dir.join("bin");

        let plan = ToolPlanToml::from_str(
            r#"
mode = "deny"
[[deny]]
name = "rm"
            "#,
        )
        .unwrap();
        let resolved = ResolvedToolPlan::resolve("safe", &plan, &universe(&["rm", "hello"]));
        let renderer = SessionBinRenderer::new(&session_bin, &agent_tools, "safe".into(), resolved);
        renderer.render_initial(&session_dir).unwrap();

        // Linked agent tool present.
        assert!(session_bin.join("hello").exists());
        // Tombstone present (and is not the same inode as the agent tool —
        // it's a freshly written script with the rm name).
        let rm_body = std::fs::read_to_string(session_bin.join("rm")).unwrap();
        assert!(rm_body.contains("exit 127"));
        // Resolved plan dumped for audit.
        let dump = std::fs::read_to_string(session_dir.join("tool_plan.resolved.toml")).unwrap();
        assert!(dump.contains("plan_name = \"safe\""));
        assert!(dump.contains("tool = \"rm\""));
    }
}
