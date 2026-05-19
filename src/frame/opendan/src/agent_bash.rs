//! §9.5 of NewOpenDANRuntime — UI-session default tool wiring.
//!
//! Registers the built-in tool catalogue described in §2 / §3 of the notepad
//! onto an `AgentToolManager`:
//!   - `exec_bash` (with overlay PATH stacked across the §2 4-layer model:
//!     Session > Agent > Runtime > System) — backed by a per-session tmux
//!     runner so each AgentSession owns one long-lived `od_<sid>` tmux
//!     session, and `exec_bash` calls land in that pane
//!   - `read` / `write_file` / `edit_file`
//!   - `glob` / `grep`
//!
//! 4-layer overlay (§2 of NewOpenDANRuntime.md, "渲染规则 2026-05-14 修订"):
//! System / Runtime / Agent are rendered-free — each is just a bin directory
//! prepended to PATH in priority order. Session Exec Bin is the only
//! rendered layer: at session boot we hard-link Agent tools into it and
//! write tombstone stub scripts for tools blocked by the behavior's tool
//! plan; every `exec_bash` call runs an opportunistic mtime resync so live
//! edits to `<agent_root>/tools/` show up on the next LLM step.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use log::warn;
use tokio::fs;
use tokio::process::Command;
use tokio::time::{sleep, Duration};

use agent_tool::{
    AgentToolError, AgentToolManager, BashRunOutput, BashRunRequest, BashRunner, BashTarget,
    BinOverlayConfig, EditFileTool, ExecBashTool, FileToolConfig, LlmBashConfig,
    NoopFileWriteAudit, ReadTool, SessionRuntimeContext, WriteFileTool,
};

use crate::agent_config::FilesystemPolicy;
use crate::paths;
use crate::tool_plan::SessionBinRenderer;

/// Tmux session name prefix. Used both to namespace per-agent-session panes and
/// to identify our sessions during GC sweeps (`tmux list-sessions`).
const TMUX_SESSION_PREFIX: &str = "od_";
/// Polling interval while waiting for the per-run exit-code file. Short enough
/// to feel snappy on small commands, large enough to avoid burning a core.
const TMUX_POLL_MS: u64 = 120;
/// How many lines of scrollback `capture-pane` should pull back when we read
/// pane output for fallback / partial-output paths.
const TMUX_CAPTURE_SCROLLBACK: &str = "-6000";
/// Trigger GC of stale sessions once we observe at least this many sessions on
/// the server (cheap check before each `new-session`).
const TMUX_GC_TRIGGER_COUNT: usize = 16;
/// Sessions whose last activity is older than this are considered stale and
/// reaped during GC. 24h is long enough to span an LLM run but short enough
/// that crashed agents don't leak forever.
const TMUX_GC_IDLE_SECS: u64 = 24 * 60 * 60;

/// Env var that gates the bash `command_not_found_handle` proxy injected by
/// [`build_exec_script`]. When set on the exec_bash request env, its value
/// must be the absolute path to the `agent_tool` CLI binary; an unknown
/// command in the user's script is then proxied through
/// `"$OPENDAN_AGENT_TOOL" __command_not_found__ <argv...>` so the intent
/// engine bypass (see `agent_tool::llm_tool_carft`) gets a chance to
/// synthesise a tool before bash falls back to the native
/// `command not found` error. Unset ⇒ no hook installed, bash behaves
/// normally. The session-level intent-bypass toggle is responsible for
/// populating this var when wired up.
pub const OPENDAN_AGENT_TOOL_ENV: &str = "OPENDAN_AGENT_TOOL";

static EXEC_RUN_SEQ: AtomicU64 = AtomicU64::new(0);

/// Layout for the 4 PATH layers as described in §2 of the notepad. All four
/// paths are resolved up-front against `<buckyos_root>` + `agent_id` +
/// `session_id`; the §2 PATH precedence is encoded by [`Self::to_overlay`]
/// returning a 4-element [`BinOverlayConfig`] in Session > Agent > Runtime >
/// System order.
#[derive(Debug, Clone)]
pub struct SessionBinLayout {
    /// `<buckyos_root>/tools/store/` — host-shared, read-only.
    pub system_bin: PathBuf,
    /// `<buckyos_root>/tools/bin/` — App-scoped, read-only. Empty-dir
    /// placeholder until the ExtTool Volume / Crafter integration lands.
    pub runtime_bin: PathBuf,
    /// `<agent_root>/tools/` — Agent-owned, persistent.
    pub agent_bin: PathBuf,
    /// `<buckyos_root>/tools/<agent_id>/<session_id>/` — rendered fresh per
    /// session. Holds hard-linked Agent tools + tool-plan tombstones.
    pub session_bin: PathBuf,
}

impl SessionBinLayout {
    /// Compute the 4-layer layout for `(agent_id, session_id)`. The agent
    /// root is needed for the Agent Bin layer; the other three layers
    /// resolve from [`paths::buckyos_root`].
    pub fn compute(agent_id: &str, session_id: &str, agent_root: &Path) -> Self {
        Self {
            system_bin: paths::system_bin_dir(),
            runtime_bin: paths::runtime_bin_dir(),
            agent_bin: agent_root.join("tools"),
            session_bin: paths::session_exec_bin_dir(agent_id, session_id),
        }
    }

    /// Create the bin directories the runtime is allowed to own. We skip
    /// `system_bin` (pre-provisioned by the BuckyOS image; creating it
    /// would need root in production) and idempotently mkdir the other
    /// three so a missing Agent root / first-boot scenario still boots.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.runtime_bin)?;
        std::fs::create_dir_all(&self.agent_bin)?;
        std::fs::create_dir_all(&self.session_bin)?;
        Ok(())
    }

    /// Convert into the upstream multi-layer [`BinOverlayConfig`]. Layer
    /// order matches §2: Session > Agent > Runtime > System.
    pub fn to_overlay(&self) -> BinOverlayConfig {
        BinOverlayConfig::layered([
            self.session_bin.clone(),
            self.agent_bin.clone(),
            self.runtime_bin.clone(),
            self.system_bin.clone(),
        ])
    }
}

/// Bundle of paths used to configure the file tools' read/write roots.
#[derive(Debug, Clone)]
pub struct FsRoots {
    /// Workspace root — both read and write allowed.
    pub workspace_root: PathBuf,
    /// Additional read-only roots (agent root, etc.). Granted read but not write.
    pub extra_read_roots: Vec<PathBuf>,
    pub filesystem_policy: FilesystemPolicy,
}

impl FsRoots {
    pub fn workspace_only(workspace_root: impl Into<PathBuf>) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            extra_read_roots: Vec::new(),
            filesystem_policy: FilesystemPolicy::default(),
        }
    }

    pub fn with_extra_read(mut self, root: impl Into<PathBuf>) -> Self {
        self.extra_read_roots.push(root.into());
        self
    }

    pub fn with_filesystem_policy(mut self, policy: FilesystemPolicy) -> Self {
        self.filesystem_policy = policy;
        self
    }

    fn to_file_tool_config(&self) -> FileToolConfig {
        let mut cfg = FileToolConfig::new(self.workspace_root.clone());
        match self.filesystem_policy {
            FilesystemPolicy::Workspace => {
                cfg.allowed_read_roots.extend(self.extra_read_roots.clone());
            }
            FilesystemPolicy::Unrestricted => {
                cfg.allowed_read_roots.clear();
            }
        }
        cfg
    }
}

/// `BashRunner` implementation that routes every `exec_bash` call through a
/// per-agent-session tmux session (one `od_<sid>` window per AgentSession).
///
/// Rationale: keeping a long-lived pane preserves shell state (env exports,
/// bash history, etc.) and matches the "agent has a terminal" mental model
/// that the legacy implementation depended on. Each call still runs against
/// a fresh wrapper script with a unique run-id, so we get reliable exit code
/// + stdout/stderr capture via marker files instead of trying to scrape the
/// pane (which is lossy under wrapping / colors).
pub struct TmuxBashRunner {
    /// Per-session scratch directory for wrapper scripts + stdout/stderr/exit
    /// log files. Created lazily on the first call.
    runtime_dir: PathBuf,
    /// Optional Session Exec Bin renderer. When present the runner triggers
    /// a cheap mtime-driven resync at the head of every `run()` so Agent
    /// tool edits + tool-plan tombstone refreshes propagate without a
    /// session restart. `None` in unit tests that don't need the layer.
    bin_renderer: Option<Arc<SessionBinRenderer>>,
}

impl TmuxBashRunner {
    pub fn new(runtime_dir: impl Into<PathBuf>) -> Self {
        Self {
            runtime_dir: runtime_dir.into(),
            bin_renderer: None,
        }
    }

    pub fn with_bin_renderer(mut self, renderer: Arc<SessionBinRenderer>) -> Self {
        self.bin_renderer = Some(renderer);
        self
    }
}

#[async_trait]
impl BashRunner for TmuxBashRunner {
    async fn run(
        &self,
        ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<BashRunOutput, AgentToolError> {
        // Targets other than Local don't go through tmux either; they're
        // explicitly rejected so the LLM gets a clear error instead of a
        // silent fallback.
        if let BashTarget::Unsupported(value) = &req.target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported exec_bash target `{value}` (only local is supported)"
            )));
        }

        // Hot-path Agent tools resync per §9.2 design point #2: cheap
        // mtime walk picks up any new/edited tools the operator dropped
        // into `<agent_root>/tools/` since the last run. Failure is
        // logged but doesn't block the command — stale tool surface is
        // better than a refused exec_bash.
        if let Some(renderer) = self.bin_renderer.as_ref() {
            if let Err(err) = renderer.maybe_resync() {
                warn!("opendan.agent_bash: Session Exec Bin resync failed: {err}");
            }
        }

        let started = Instant::now();
        let tmux_session = build_tmux_session_name(&ctx.session_id);
        let tmux_target = format!("{tmux_session}:0.0");
        ensure_tmux_session(&tmux_session, &req.cwd).await?;

        fs::create_dir_all(&self.runtime_dir).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create exec runtime dir `{}` failed: {err}",
                self.runtime_dir.display()
            ))
        })?;

        let run_id = format!(
            "{}-{}-{}-{}",
            now_ms(),
            sanitize_for_id(&ctx.behavior),
            ctx.step_idx,
            EXEC_RUN_SEQ.fetch_add(1, Ordering::Relaxed)
        );
        let stdout_path = self.runtime_dir.join(format!("{run_id}.stdout.log"));
        let stderr_path = self.runtime_dir.join(format!("{run_id}.stderr.log"));
        let exit_code_path = self.runtime_dir.join(format!("{run_id}.exit.code"));
        let script_path = self.runtime_dir.join(format!("{run_id}.exec.sh"));

        let script = build_exec_script(
            &run_id,
            &stdout_path,
            &stderr_path,
            &exit_code_path,
            &req.cwd,
            &req.command,
            &req.env,
        );
        fs::write(&script_path, script).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "write exec script `{}` failed: {err}",
                script_path.display()
            ))
        })?;

        // Audit trail: print a human-readable banner before invoking the
        // wrapper, so an operator attaching with `tmux attach -t od_<sid>`
        // sees the actual command (and its run id) rather than an opaque
        // `. /tmp/.../<run>.exec.sh` line. We deliberately do NOT clear
        // pane history — the exit-code marker is namespaced by run_id, so
        // scrollback residue can't fool the fallback parser, and keeping
        // history makes after-the-fact audit possible.
        let banner = format!(
            "printf '\\n\\033[1;36m# exec_bash[%s]\\033[0m %s\\n' {run} {cmd}",
            run = shell_quote(&run_id),
            cmd = shell_quote(&req.command),
        );
        send_keys(&tmux_target, &banner).await?;

        let invoke = format!(". {}", shell_quote(script_path.to_string_lossy().as_ref()));
        send_keys(&tmux_target, &invoke).await?;

        let exit_code = match wait_exit_code(&exit_code_path, &tmux_target, &run_id, req.timeout_ms)
            .await?
        {
            Some(code) => code,
            None => {
                // Best-effort interrupt the pane so a runaway command stops
                // chewing CPU; we still return Timeout to the caller.
                let _ = interrupt_pane(&tmux_target).await;
                cleanup_run_files(&script_path, &stdout_path, &stderr_path, &exit_code_path).await;
                return Err(AgentToolError::Timeout);
            }
        };

        // Brief settle so any in-flight `tee` flush lands before we read the
        // log files (cheaper than fsync'ing from the script side).
        sleep(Duration::from_millis(30)).await;
        let stdout_bytes = fs::read(&stdout_path).await.unwrap_or_default();
        let stderr_bytes = fs::read(&stderr_path).await.unwrap_or_default();
        cleanup_run_files(&script_path, &stdout_path, &stderr_path, &exit_code_path).await;

        let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
        let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
        let (output, output_truncated) =
            assemble_output(&stdout_bytes, &stderr_bytes, req.max_output_bytes);

        Ok(BashRunOutput {
            exit_code,
            stdout,
            stderr,
            output,
            output_truncated,
            duration_ms: started.elapsed().as_millis() as u64,
            engine: "tmux".to_string(),
            cwd: req.cwd,
        })
    }
}

/// Inputs for [`build_session_tools`]. Bundled into a struct because the
/// 4-layer overlay model needs more context than the original
/// `(workspace_root, session_dir)` MVP — `agent_id` + `session_id` drive
/// the Session Exec Bin path, `agent_root` anchors the Agent Bin layer,
/// and `bin_renderer` (when present) wires Agent tool sync + tombstones
/// into `exec_bash`.
pub struct SessionToolsBuild {
    pub workspace_root: PathBuf,
    pub session_dir: PathBuf,
    pub agent_root: PathBuf,
    pub agent_id: String,
    pub session_id: String,
    pub filesystem_policy: FilesystemPolicy,
    /// Pre-resolved Session Exec Bin renderer (per the behavior's tool
    /// plan). `None` ⇒ no tombstones and no Agent tool sync — useful in
    /// tests that just want the file tools wired.
    pub bin_renderer: Option<Arc<SessionBinRenderer>>,
}

/// Build a tool manager pre-populated with the §3 UI-session defaults.
/// `layout` defines the 4-layer PATH overlay; `bash_runtime_dir` is the
/// per-session scratch dir for tmux wrapper scripts + exit-code marker
/// files.
pub fn build_default_tool_manager(
    fs_roots: FsRoots,
    layout: &SessionBinLayout,
    bash_runtime_dir: &Path,
    bin_renderer: Option<Arc<SessionBinRenderer>>,
) -> Arc<AgentToolManager> {
    let manager = AgentToolManager::new();

    let bash_cfg =
        LlmBashConfig::local_workspace(&fs_roots.workspace_root).with_overlay(layout.to_overlay());
    let mut runner = TmuxBashRunner::new(bash_runtime_dir);
    if let Some(renderer) = bin_renderer {
        runner = runner.with_bin_renderer(renderer);
    }
    let runner: Arc<dyn BashRunner> = Arc::new(runner);
    let _ = manager.register_tool(ExecBashTool::with_runner(bash_cfg, runner));

    let file_cfg = fs_roots.to_file_tool_config();
    let audit = Arc::new(NoopFileWriteAudit);
    // v2 Action set (see doc/opendan/Agent Actions.md §1):
    // - `read` replaces v1 `read_file`
    // - `glob` / `grep` removed (LLM uses find/grep/rg via exec_bash)
    // - `write_file` / `edit_file` unchanged
    let _ = manager.register_tool(ReadTool::new(file_cfg.clone()));
    let _ = manager.register_typed_tool(WriteFileTool::new(file_cfg.clone(), audit.clone()));
    let _ = manager.register_typed_tool(EditFileTool::new(file_cfg, audit));

    Arc::new(manager)
}

/// Higher-level convenience used by `AIAgent` on session create — bundles
/// the 4-layer layout computation, directory mkdir, and tool manager
/// bootstrap. The tmux scratch dir lives under the session dir so it gets
/// reaped with it.
pub fn build_session_tools(build: SessionToolsBuild) -> std::io::Result<Arc<AgentToolManager>> {
    let layout = SessionBinLayout::compute(&build.agent_id, &build.session_id, &build.agent_root);
    layout.ensure_dirs()?;
    let bash_runtime_dir = build.session_dir.join(".runtime").join("exec_bash");
    std::fs::create_dir_all(&bash_runtime_dir)?;

    // If a renderer is supplied, do the initial Agent tools link + tombstone
    // render now so the very first `exec_bash` finds the layer populated.
    // We propagate render errors so the agent boots cleanly or fails loudly
    // — a half-rendered Session Exec Bin would be hard to diagnose later.
    if let Some(renderer) = build.bin_renderer.as_ref() {
        renderer.render_initial(&build.session_dir)?;
    }

    let manager = build_default_tool_manager(
        FsRoots::workspace_only(&build.workspace_root)
            .with_extra_read(&build.agent_root)
            .with_filesystem_policy(build.filesystem_policy),
        &layout,
        &bash_runtime_dir,
        build.bin_renderer.clone(),
    );
    Ok(manager)
}

// ---------------------------------------------------------------------------
// tmux helpers (private)
// ---------------------------------------------------------------------------

fn build_tmux_session_name(session_id: &str) -> String {
    format!("{TMUX_SESSION_PREFIX}{}", sanitize_for_id(session_id))
}

/// Restrict id characters to `[A-Za-z0-9_]`; tmux session names are otherwise
/// permissive but we want predictable, shell-safe names.
fn sanitize_for_id(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push('_');
    }
    out
}

fn shell_quote(raw: &str) -> String {
    if raw.is_empty() {
        return "''".to_string();
    }
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

async fn ensure_tmux_session(name: &str, cwd: &Path) -> Result<(), AgentToolError> {
    let probe = Command::new("tmux")
        .arg("-V")
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux unavailable: {err}")))?;
    if !probe.status.success() {
        return Err(AgentToolError::ExecFailed(
            "tmux command exists but version probe failed".to_string(),
        ));
    }

    let has = Command::new("tmux")
        .args(["has-session", "-t", name])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux has-session failed: {err}")))?;
    if has.status.success() {
        return Ok(());
    }

    maybe_gc_stale_sessions().await;
    let created = Command::new("tmux")
        .args(["new-session", "-d", "-s", name, "-c"])
        .arg(cwd)
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux new-session failed: {err}")))?;
    if created.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "create tmux session `{name}` failed: {}",
        String::from_utf8_lossy(&created.stderr)
    )))
}

async fn maybe_gc_stale_sessions() {
    let sessions = match Command::new("tmux")
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\t#{session_activity}",
        ])
        .output()
        .await
    {
        Ok(out) if out.status.success() => out,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.to_ascii_lowercase().contains("no server running") {
                warn!("tmux list-sessions failed before gc: {}", stderr.trim());
            }
            return;
        }
        Err(err) => {
            warn!("tmux list-sessions exec failed before gc: {err}");
            return;
        }
    };

    let raw = String::from_utf8_lossy(&sessions.stdout);
    let mut entries: Vec<(String, Option<u64>)> = Vec::new();
    for line in raw.lines() {
        let mut parts = line.splitn(2, '\t');
        let Some(name) = parts.next().map(str::trim).filter(|s| !s.is_empty()) else {
            continue;
        };
        let activity = parts
            .next()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .and_then(|v| v.parse::<u64>().ok());
        entries.push((name.to_string(), activity));
    }
    if entries.len() < TMUX_GC_TRIGGER_COUNT {
        return;
    }

    let now = now_unix_secs();
    let mut reaped = 0usize;
    for (name, activity) in entries {
        if !name.starts_with(TMUX_SESSION_PREFIX) {
            continue;
        }
        let Some(last) = activity else { continue };
        if now.saturating_sub(last) < TMUX_GC_IDLE_SECS {
            continue;
        }
        match Command::new("tmux")
            .args(["kill-session", "-t", &name])
            .output()
            .await
        {
            Ok(out) if out.status.success() => {
                reaped += 1;
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                if !stderr.to_ascii_lowercase().contains("can't find session") {
                    warn!("tmux gc kill-session `{name}` failed: {}", stderr.trim());
                }
            }
            Err(err) => warn!("tmux gc kill-session `{name}` exec failed: {err}"),
        }
    }
    if reaped > 0 {
        warn!("tmux gc reclaimed {reaped} stale sessions");
    }
}

async fn send_keys(target: &str, command: &str) -> Result<(), AgentToolError> {
    let out = Command::new("tmux")
        .args(["send-keys", "-t", target, "--", command, "C-m"])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux send-keys failed: {err}")))?;
    if out.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux send-keys `{target}` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    )))
}

async fn interrupt_pane(target: &str) -> Result<(), AgentToolError> {
    let out = Command::new("tmux")
        .args(["send-keys", "-t", target, "C-c"])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux interrupt failed: {err}")))?;
    if out.status.success() {
        return Ok(());
    }
    Err(AgentToolError::ExecFailed(format!(
        "tmux interrupt `{target}` failed: {}",
        String::from_utf8_lossy(&out.stderr)
    )))
}

async fn capture_pane(target: &str) -> Result<String, AgentToolError> {
    let out = Command::new("tmux")
        .args([
            "capture-pane",
            "-p",
            "-J",
            "-S",
            TMUX_CAPTURE_SCROLLBACK,
            "-t",
            target,
        ])
        .output()
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("tmux capture-pane failed: {err}")))?;
    if !out.status.success() {
        return Err(AgentToolError::ExecFailed(format!(
            "tmux capture-pane `{target}` failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Wait for the exit-code file written by the wrapper script. Two readers,
/// in order: the marker file (authoritative) and a regex over the pane
/// scrollback (fallback in case the file write races with our poll).
async fn wait_exit_code(
    exit_code_path: &Path,
    tmux_target: &str,
    run_id: &str,
    timeout_ms: u64,
) -> Result<Option<i32>, AgentToolError> {
    let started = Instant::now();
    let deadline = Duration::from_millis(timeout_ms);
    let pane_marker = format!("__OD_EXIT__{run_id}:");
    loop {
        if started.elapsed() >= deadline {
            return Ok(None);
        }
        if let Some(code) = read_exit_code_file(exit_code_path).await? {
            return Ok(Some(code));
        }
        if let Ok(pane) = capture_pane(tmux_target).await {
            if let Some(code) = parse_exit_code_from_pane(&pane, &pane_marker)? {
                return Ok(Some(code));
            }
        }
        sleep(Duration::from_millis(TMUX_POLL_MS)).await;
    }
}

async fn read_exit_code_file(path: &Path) -> Result<Option<i32>, AgentToolError> {
    let raw = match fs::read_to_string(path).await {
        Ok(raw) => raw,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(AgentToolError::ExecFailed(format!(
                "read exit code file `{}` failed: {err}",
                path.display()
            )));
        }
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    trimmed.parse::<i32>().map(Some).map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "invalid exit code file `{}` content `{trimmed}`: {err}",
            path.display()
        ))
    })
}

fn parse_exit_code_from_pane(pane: &str, marker: &str) -> Result<Option<i32>, AgentToolError> {
    for line in pane.lines().rev() {
        let Some(pos) = line.find(marker) else {
            continue;
        };
        let tail = &line[pos + marker.len()..];
        let mut buf = String::new();
        for ch in tail.chars() {
            if (ch == '-' && buf.is_empty()) || ch.is_ascii_digit() {
                buf.push(ch);
            } else if !buf.is_empty() {
                break;
            }
        }
        if buf.is_empty() || buf == "-" {
            return Err(AgentToolError::ExecFailed(format!(
                "invalid tmux exit code payload after marker: `{tail}`"
            )));
        }
        return buf
            .parse::<i32>()
            .map(Some)
            .map_err(|err| AgentToolError::ExecFailed(format!("parse exit code `{buf}`: {err}")));
    }
    Ok(None)
}

async fn cleanup_run_files(script: &Path, stdout: &Path, stderr: &Path, exit_code: &Path) {
    let _ = fs::remove_file(script).await;
    let _ = fs::remove_file(stdout).await;
    let _ = fs::remove_file(stderr).await;
    let _ = fs::remove_file(exit_code).await;
}

/// Wrapper script body: cd's into the request cwd, exports the resolved env,
/// runs the user command with stdout/stderr `tee`'d to log files, writes the
/// exit code to a marker file, and prints a pane marker so the fallback
/// parser can recover the exit code if the marker file read races.
fn build_exec_script(
    run_id: &str,
    stdout_path: &Path,
    stderr_path: &Path,
    exit_code_path: &Path,
    cwd: &Path,
    command: &str,
    env_vars: &[(String, String)],
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("__od_run_id={}", shell_quote(run_id)));
    lines.push(format!(
        "__od_stdout={}",
        shell_quote(stdout_path.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_stderr={}",
        shell_quote(stderr_path.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_exit_code={}",
        shell_quote(exit_code_path.to_string_lossy().as_ref())
    ));
    lines.push(format!(
        "__od_cwd={}",
        shell_quote(cwd.to_string_lossy().as_ref())
    ));
    lines.push(": > \"$__od_stdout\"".to_string());
    lines.push(": > \"$__od_stderr\"".to_string());
    lines.push("rm -f \"$__od_exit_code\"".to_string());
    // Outer `{ ... }` group carries the tee redirections; the user command
    // runs inside a `( ... )` subshell so a literal `exit N` only kills the
    // subshell — if it killed the wrapper itself, we'd never write the exit
    // code file and the run would time out instead of returning N.
    lines.push("{".to_string());
    lines.push("  cd \"$__od_cwd\" || { echo \"cd failed: $__od_cwd\" >&2; false; }".to_string());
    for (key, value) in env_vars {
        lines.push(format!("  export {}={}", key, shell_quote(value.as_str())));
    }
    // Intent-engine bypass hook: when the session sets OPENDAN_AGENT_TOOL,
    // install bash's command_not_found_handle so unknown commands are
    // proxied to `agent_tool __command_not_found__` instead of dying with
    // 127 immediately. The handler still falls back to bash's native error
    // message when the proxy itself reports 127, so a fully-failed bypass
    // looks the same to the LLM as no bypass. Functions defined here are
    // inherited by the `( ... )` subshell below.
    lines.push(format!(
        "  if [ -n \"${{{env}:-}}\" ]; then",
        env = OPENDAN_AGENT_TOOL_ENV
    ));
    lines.push("    command_not_found_handle() {".to_string());
    lines.push(format!(
        "      local __od_tool=\"${{{env}:-}}\"",
        env = OPENDAN_AGENT_TOOL_ENV
    ));
    lines.push(format!(
        "      \"$__od_tool\" {} \"$@\"",
        shell_quote(agent_tool::CLI_COMMAND_NOT_FOUND_SUBCOMMAND)
    ));
    lines.push("      local __od_cnf_ec=$?".to_string());
    lines.push("      if [ \"$__od_cnf_ec\" -eq 127 ]; then".to_string());
    lines.push("        printf 'bash: %s: command not found\\n' \"$1\" >&2".to_string());
    lines.push("      fi".to_string());
    lines.push("      return \"$__od_cnf_ec\"".to_string());
    lines.push("    }".to_string());
    lines.push("  fi".to_string());
    lines.push("  (".to_string());
    lines.push(command.to_string());
    lines.push("  )".to_string());
    lines.push("} > >(tee \"$__od_stdout\") 2> >(tee \"$__od_stderr\" >&2)".to_string());
    lines.push("__od_ec=$?".to_string());
    lines.push("printf \"%s\\n\" \"$__od_ec\" > \"$__od_exit_code\"".to_string());
    lines.push("printf \"__OD_EXIT__%s:%s\\n\" \"$__od_run_id\" \"$__od_ec\"".to_string());
    lines.join("\n")
}

/// Merge stdout + stderr into the single `output` field the LLM sees, with
/// a `max_output_bytes` cap that's applied to the merged blob (so a chatty
/// stderr can't crowd out a useful stdout).
fn assemble_output(stdout: &[u8], stderr: &[u8], max_bytes: usize) -> (String, bool) {
    let mut combined: Vec<u8> = Vec::with_capacity(stdout.len() + stderr.len() + 1);
    combined.extend_from_slice(stdout);
    if !stdout.is_empty() && !stderr.is_empty() {
        combined.push(b'\n');
    }
    combined.extend_from_slice(stderr);
    if combined.len() <= max_bytes {
        return (String::from_utf8_lossy(&combined).to_string(), false);
    }
    (
        String::from_utf8_lossy(&combined[..max_bytes]).to_string(),
        true,
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn now_unix_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn ctx_for(sid: &str) -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            step_idx: 0,
            wakeup_id: "w".into(),
            session_id: sid.to_string(),
        }
    }

    #[test]
    fn registers_default_tools() {
        let dir = tempdir().unwrap();
        // BUCKYOS_ROOT is consulted at layout-compute time; pin it under the
        // tempdir so multiple parallel tests don't fight over /opt/buckyos.
        let _bg = ScopedBuckyosRoot::set(dir.path());
        let manager = build_session_tools(SessionToolsBuild {
            workspace_root: dir.path().to_path_buf(),
            session_dir: dir.path().join("session"),
            agent_root: dir.path().join("agent_root"),
            agent_id: "test-agent".to_string(),
            session_id: format!("s_{}", now_ms()),
            filesystem_policy: FilesystemPolicy::Workspace,
            bin_renderer: None,
        })
        .expect("build tools");
        for name in ["exec_bash", "read"] {
            assert!(manager.has_tool(name), "tool {name} not registered");
        }
        for action_only in ["write_file", "edit_file"] {
            assert!(
                manager.get_any_tool(action_only).is_some(),
                "tool {action_only} not registered"
            );
        }
        // v2 dropped Glob/Grep/read_file from the action registry.
        for dropped in ["Glob", "Grep", "read_file"] {
            assert!(
                !manager.has_tool(dropped),
                "tool {dropped} should be unregistered in v2"
            );
        }
    }

    #[test]
    fn filesystem_policy_controls_read_roots() {
        let dir = tempdir().unwrap();
        let ws = dir.path().join("workspace");
        let agent_root = dir.path().join("agent");

        let workspace_cfg = FsRoots::workspace_only(&ws)
            .with_extra_read(&agent_root)
            .with_filesystem_policy(FilesystemPolicy::Workspace)
            .to_file_tool_config();
        assert_eq!(workspace_cfg.allowed_read_roots.len(), 2);

        let unrestricted_cfg = FsRoots::workspace_only(&ws)
            .with_extra_read(&agent_root)
            .with_filesystem_policy(FilesystemPolicy::Unrestricted)
            .to_file_tool_config();
        assert!(unrestricted_cfg.allowed_read_roots.is_empty());
        assert_eq!(unrestricted_cfg.allowed_write_roots, vec![ws]);
    }

    #[test]
    fn session_bin_layout_overlay_has_four_layers() {
        let dir = tempdir().unwrap();
        let _bg = ScopedBuckyosRoot::set(dir.path());
        let layout = SessionBinLayout::compute("agent-1", "ses-1", &dir.path().join("agent_root"));
        layout.ensure_dirs().unwrap();
        assert!(layout.runtime_bin.exists());
        assert!(layout.agent_bin.exists());
        assert!(layout.session_bin.exists());
        let overlay = layout.to_overlay();
        assert!(overlay.enabled);
        assert_eq!(overlay.layers.len(), 4);
        assert_eq!(overlay.layers[0], layout.session_bin);
        assert_eq!(overlay.layers[1], layout.agent_bin);
        assert_eq!(overlay.layers[2], layout.runtime_bin);
        assert_eq!(overlay.layers[3], layout.system_bin);
    }

    /// Test guard: pins `BUCKYOS_ROOT` for the lifetime of the scope so the
    /// layout helpers point at a unique tempdir, and restores the previous
    /// value on drop. Use as `let _g = ScopedBuckyosRoot::set(...)`.
    struct ScopedBuckyosRoot {
        prev: Option<String>,
    }

    impl ScopedBuckyosRoot {
        fn set(path: &Path) -> Self {
            let prev = std::env::var("BUCKYOS_ROOT").ok();
            std::env::set_var("BUCKYOS_ROOT", path);
            Self { prev }
        }
    }

    impl Drop for ScopedBuckyosRoot {
        fn drop(&mut self) {
            match &self.prev {
                Some(p) => std::env::set_var("BUCKYOS_ROOT", p),
                None => std::env::remove_var("BUCKYOS_ROOT"),
            }
        }
    }

    #[test]
    fn tmux_session_name_is_namespaced_and_sanitized() {
        let raw = "sess/with-dashes:and.colons";
        let name = build_tmux_session_name(raw);
        assert!(name.starts_with(TMUX_SESSION_PREFIX));
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "name `{name}` must be shell-safe"
        );
    }

    #[test]
    fn exec_script_carries_cwd_env_and_markers() {
        let dir = tempdir().unwrap();
        let stdout = dir.path().join("out");
        let stderr = dir.path().join("err");
        let exit_code = dir.path().join("ec");
        let cwd = dir.path().join("work");
        let script = build_exec_script(
            "rid",
            &stdout,
            &stderr,
            &exit_code,
            &cwd,
            "echo hi",
            &[("FOO".to_string(), "bar".to_string())],
        );
        assert!(script.contains("__OD_EXIT__"));
        assert!(script.contains("export FOO='bar'"));
        assert!(script.contains(&format!("cd \"$__od_cwd\"")));
        assert!(script.contains("echo hi"));
    }

    #[test]
    fn exec_script_installs_command_not_found_proxy() {
        let dir = tempdir().unwrap();
        let script = build_exec_script(
            "rid",
            &dir.path().join("out"),
            &dir.path().join("err"),
            &dir.path().join("ec"),
            &dir.path().join("work"),
            "missing-cmd",
            &[],
        );
        // Hook is gated on OPENDAN_AGENT_TOOL being set on the request env.
        assert!(script.contains(&format!("if [ -n \"${{{}:-}}\" ]", OPENDAN_AGENT_TOOL_ENV)));
        // Proxies to the shared `__command_not_found__` subcommand…
        assert!(script.contains("command_not_found_handle()"));
        assert!(script.contains(agent_tool::CLI_COMMAND_NOT_FOUND_SUBCOMMAND));
        // …and falls back to bash's native error when the proxy returns 127.
        assert!(script.contains("command not found"));
        // Handler is defined inside the outer `{ ... }` group (before the
        // user-command subshell) so the `( ... )` below inherits it.
        let handler_idx = script.find("command_not_found_handle()").unwrap();
        let subshell_idx = script.find("\n  (\n").unwrap();
        assert!(
            handler_idx < subshell_idx,
            "handler must be defined before the user-command subshell so it's inherited"
        );
    }

    #[test]
    fn parse_exit_code_handles_negative_and_extra_text() {
        let pane = "some prefix __OD_EXIT__rid:-1 trailing\n";
        let code = parse_exit_code_from_pane(pane, "__OD_EXIT__rid:")
            .unwrap()
            .unwrap();
        assert_eq!(code, -1);
        let pane2 = "__OD_EXIT__rid:7\n";
        let code2 = parse_exit_code_from_pane(pane2, "__OD_EXIT__rid:")
            .unwrap()
            .unwrap();
        assert_eq!(code2, 7);
        assert!(
            parse_exit_code_from_pane("no marker here", "__OD_EXIT__rid:")
                .unwrap()
                .is_none()
        );
    }

    /// End-to-end: requires `tmux` on PATH. Verifies that `exec_bash` runs
    /// through the tmux runner and returns the script's stdout + exit code.
    /// Cleans up its own tmux session on success.
    #[tokio::test]
    async fn tmux_runner_executes_simple_command() {
        if Command::new("tmux").arg("-V").output().await.is_err() {
            eprintln!("tmux not available; skipping integration test");
            return;
        }
        let dir = tempdir().unwrap();
        let runtime = dir.path().join("rt");
        std::fs::create_dir_all(&runtime).unwrap();
        let runner = TmuxBashRunner::new(&runtime);

        let sid = format!("test_{}", now_ms());
        let ctx = ctx_for(&sid);
        let req = BashRunRequest {
            command: "echo hello-tmux".to_string(),
            cwd: dir.path().to_path_buf(),
            timeout_ms: 5_000,
            max_output_bytes: 64 * 1024,
            env: Vec::new(),
            target: BashTarget::Local,
        };
        let output = runner.run(&ctx, req).await.expect("run ok");
        assert_eq!(output.exit_code, 0);
        assert!(
            output.stdout.contains("hello-tmux"),
            "stdout was `{}`",
            output.stdout
        );
        assert_eq!(output.engine, "tmux");

        // Tear down the per-session tmux session so we don't leak between
        // test runs. Best-effort; ignore errors.
        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &build_tmux_session_name(&sid)])
            .output()
            .await;
    }

    #[tokio::test]
    async fn tmux_runner_reports_nonzero_exit() {
        if Command::new("tmux").arg("-V").output().await.is_err() {
            eprintln!("tmux not available; skipping integration test");
            return;
        }
        let dir = tempdir().unwrap();
        let runtime = dir.path().join("rt");
        std::fs::create_dir_all(&runtime).unwrap();
        let runner = TmuxBashRunner::new(&runtime);

        let sid = format!("test_exit_{}", now_ms());
        let ctx = ctx_for(&sid);
        let req = BashRunRequest {
            command: "exit 9".to_string(),
            cwd: dir.path().to_path_buf(),
            timeout_ms: 5_000,
            max_output_bytes: 4_096,
            env: Vec::new(),
            target: BashTarget::Local,
        };
        let output = runner.run(&ctx, req).await.expect("run ok");
        assert_eq!(output.exit_code, 9);

        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &build_tmux_session_name(&sid)])
            .output()
            .await;
    }

    #[tokio::test]
    async fn tmux_runner_honors_timeout() {
        if Command::new("tmux").arg("-V").output().await.is_err() {
            eprintln!("tmux not available; skipping integration test");
            return;
        }
        let dir = tempdir().unwrap();
        let runtime = dir.path().join("rt");
        std::fs::create_dir_all(&runtime).unwrap();
        let runner = TmuxBashRunner::new(&runtime);

        let sid = format!("test_to_{}", now_ms());
        let ctx = ctx_for(&sid);
        let req = BashRunRequest {
            command: "sleep 5".to_string(),
            cwd: dir.path().to_path_buf(),
            timeout_ms: 400,
            max_output_bytes: 1_024,
            env: Vec::new(),
            target: BashTarget::Local,
        };
        let err = runner.run(&ctx, req).await.expect_err("should time out");
        assert!(matches!(err, AgentToolError::Timeout), "got {err:?}");

        let _ = Command::new("tmux")
            .args(["kill-session", "-t", &build_tmux_session_name(&sid)])
            .output()
            .await;
    }
}
