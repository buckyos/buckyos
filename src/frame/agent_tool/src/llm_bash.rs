//! Composable `exec_bash` tool for the agent_tool crate.
//!
//! Provides the building blocks (`BashRunner`, `LocalProcessBashRunner`,
//! `ExecBashTool`, `LlmBashConfig`, `BinOverlayConfig`) so any
//! `ToolManager`-shaped consumer (e.g. `LocalLLMContext`) can register a
//! local one-shot `exec_bash` without depending on OpenDAN-specific
//! session/task plumbing.
//!
//! Target/runner/overlay structures are kept extensible: the local
//! one-shot path is the only implemented backend in this stage, but
//! `BashTarget::Unsupported` and `BashRunner` leave room for tmux /
//! node / container runners later.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use serde_json::{json, Map as JsonMap, Value as Json};
use tokio::time::{timeout as tokio_timeout, Duration};

use crate::path_utils::{normalize_abs_path, resolve_path_under_root, to_abs_path};
use crate::tool::CallingConventions;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult, AgentToolStatus,
    SessionRuntimeContext, ToolSpec,
};

pub const TOOL_EXEC_BASH: &str = "exec_bash";

const DEFAULT_TIMEOUT_MS: u64 = 60_000;
const DEFAULT_MAX_TIMEOUT_MS: u64 = 10 * 60_000;
const DEFAULT_MAX_OUTPUT_BYTES: usize = 256 * 1024;
const LOCAL_ENGINE: &str = "local";

/// User-facing target field. `None` / empty string means [`BashTarget::Local`];
/// other values are passed through [`BashTargetSpec::parse`] before reaching
/// the runner.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BashTargetSpec {
    #[default]
    Local,
    Raw(String),
}

impl BashTargetSpec {
    /// Parse a user-provided string. Empty / "local" / "localhost" / "."
    /// resolve to [`BashTarget::Local`]; anything else becomes
    /// [`BashTarget::Unsupported`] so callers can surface a clear error.
    pub fn parse(raw: Option<&str>) -> BashTarget {
        let Some(value) = raw else {
            return BashTarget::Local;
        };
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return BashTarget::Local;
        }
        match trimmed.to_ascii_lowercase().as_str() {
            "local" | "localhost" | "." => BashTarget::Local,
            _ => BashTarget::Unsupported(trimmed.to_string()),
        }
    }

    pub fn resolve(&self) -> BashTarget {
        match self {
            BashTargetSpec::Local => BashTarget::Local,
            BashTargetSpec::Raw(raw) => Self::parse(Some(raw.as_str())),
        }
    }
}

/// Internal, structured execution target. Only `Local` is implemented in
/// this stage; unknown targets are explicitly rejected by [`ExecBashTool`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BashTarget {
    Local,
    Unsupported(String),
}

impl BashTarget {
    pub fn label(&self) -> &str {
        match self {
            BashTarget::Local => "local",
            BashTarget::Unsupported(value) => value.as_str(),
        }
    }
}

/// PATH overlay: an ordered list of bin directories prepended to `PATH`.
///
/// `layers[0]` has the highest precedence (entries earlier in the vector
/// win on PATH lookup). One slot is sufficient for the legacy single-bin
/// callers (`BinOverlayConfig::local`); the multi-layer form is used by
/// the §2 4-layer overlay (Session > Agent > Runtime > System).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BinOverlayConfig {
    pub layers: Vec<PathBuf>,
    pub enabled: bool,
}

impl BinOverlayConfig {
    pub fn disabled() -> Self {
        Self {
            layers: Vec::new(),
            enabled: false,
        }
    }

    pub fn local(bin_dir: impl Into<PathBuf>) -> Self {
        Self {
            layers: vec![bin_dir.into()],
            enabled: true,
        }
    }

    /// Stacked overlay: layer at index 0 has the highest priority, layer at
    /// the end has the lowest (callers pass `[session, agent, runtime, system]`
    /// for the §2 four-layer model).
    pub fn layered<I, P>(layers: I) -> Self
    where
        I: IntoIterator<Item = P>,
        P: Into<PathBuf>,
    {
        let layers: Vec<PathBuf> = layers.into_iter().map(Into::into).collect();
        Self {
            enabled: !layers.is_empty(),
            layers,
        }
    }

    fn active_layers(&self) -> &[PathBuf] {
        if !self.enabled {
            return &[];
        }
        &self.layers
    }
}

#[derive(Clone, Debug)]
pub struct LlmBashConfig {
    pub workspace: PathBuf,
    pub default_timeout_ms: u64,
    pub max_timeout_ms: u64,
    pub max_output_bytes: usize,
    pub allow_env: bool,
    pub target: BashTargetSpec,
    pub overlay: BinOverlayConfig,
    pub tool_name: String,
}

impl LlmBashConfig {
    pub fn local_workspace(workspace: impl Into<PathBuf>) -> Self {
        Self {
            workspace: workspace.into(),
            default_timeout_ms: DEFAULT_TIMEOUT_MS,
            max_timeout_ms: DEFAULT_MAX_TIMEOUT_MS,
            max_output_bytes: DEFAULT_MAX_OUTPUT_BYTES,
            allow_env: true,
            target: BashTargetSpec::Local,
            overlay: BinOverlayConfig::disabled(),
            tool_name: TOOL_EXEC_BASH.to_string(),
        }
    }

    pub fn with_tool_name(mut self, name: impl Into<String>) -> Self {
        self.tool_name = name.into();
        self
    }

    pub fn with_overlay(mut self, overlay: BinOverlayConfig) -> Self {
        self.overlay = overlay;
        self
    }

    pub fn with_default_timeout_ms(mut self, ms: u64) -> Self {
        self.default_timeout_ms = ms;
        self
    }

    pub fn with_max_timeout_ms(mut self, ms: u64) -> Self {
        self.max_timeout_ms = ms;
        self
    }

    pub fn with_max_output_bytes(mut self, bytes: usize) -> Self {
        self.max_output_bytes = bytes;
        self
    }

    pub fn with_allow_env(mut self, allow: bool) -> Self {
        self.allow_env = allow;
        self
    }
}

/// One-shot run request handed to a [`BashRunner`].
///
/// `cwd` is already validated to live under the configured workspace;
/// `env` is already filtered against `allow_env` and key validation;
/// `target` is the resolved structured target.
#[derive(Clone, Debug)]
pub struct BashRunRequest {
    pub command: String,
    pub cwd: PathBuf,
    pub timeout_ms: u64,
    pub max_output_bytes: usize,
    pub env: Vec<(String, String)>,
    pub target: BashTarget,
}

#[derive(Clone, Debug, Default)]
pub struct BashRunOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub output: String,
    pub output_truncated: bool,
    pub duration_ms: u64,
    pub engine: String,
    pub cwd: PathBuf,
}

#[async_trait]
pub trait BashRunner: Send + Sync {
    async fn run(
        &self,
        ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<BashRunOutput, AgentToolError>;
}

/// Build the env list applied to the spawned shell. Prepends each overlay
/// layer to `PATH` in order so `layers[0]` ends up at the very front;
/// user-supplied env vars are merged on top. Split into its own helper so
/// the day-2 overlay refactor only touches this function.
pub fn prepare_overlay_env(
    overlay: &BinOverlayConfig,
    user_env: &[(String, String)],
) -> Vec<(String, String)> {
    let mut merged = BTreeMap::<String, String>::new();
    for (key, value) in user_env {
        merged.insert(key.clone(), value.clone());
    }

    let active = overlay.active_layers();
    if !active.is_empty() {
        let base_path = merged
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        // Walk layers from lowest precedence (end) to highest (front) so each
        // `prepend_path_entry` call leaves the higher-priority layer at the
        // very front of the resulting PATH string.
        let mut path = base_path;
        for layer in active.iter().rev() {
            let entry = layer.to_string_lossy().to_string();
            path = prepend_path_entry(&entry, &path);
        }
        merged.insert("PATH".to_string(), path);
    }

    merged.into_iter().collect()
}

fn prepend_path_entry(entry: &str, base_path: &str) -> String {
    let entry = entry.trim();
    if entry.is_empty() {
        return base_path.to_string();
    }
    if base_path.is_empty() {
        return entry.to_string();
    }
    if base_path.split(':').any(|item| item == entry) {
        return base_path.to_string();
    }
    format!("{entry}:{base_path}")
}

fn truncate_bytes(input: &[u8], max_bytes: usize) -> (String, bool) {
    if input.len() <= max_bytes {
        return (String::from_utf8_lossy(input).to_string(), false);
    }
    (
        String::from_utf8_lossy(&input[..max_bytes]).to_string(),
        true,
    )
}

/// Validate a shell env key: ASCII letter/underscore followed by
/// letters/digits/underscores. Mirrors `[A-Za-z_][A-Za-z0-9_]*`.
fn is_valid_shell_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

/// Default `BashRunner`: spawns `/bin/bash -lc <command>` via
/// `tokio::process`, captures stdout/stderr, enforces timeout, and
/// truncates the merged output to `max_output_bytes`.
#[derive(Clone, Debug, Default)]
pub struct LocalProcessBashRunner;

impl LocalProcessBashRunner {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl BashRunner for LocalProcessBashRunner {
    async fn run(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> Result<BashRunOutput, AgentToolError> {
        match req.target {
            BashTarget::Local => {}
            BashTarget::Unsupported(value) => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported exec_bash target `{value}` (only local is supported)"
                )));
            }
        }

        // Use `-c` rather than `-lc`: a login shell sources profile files
        // (on macOS `path_helper` resets PATH from /etc/paths) which would
        // demote the overlay bin_dir below /usr/bin and break the
        // "bin_dir : 原 PATH" precedence the caller is promised.
        let mut cmd = tokio::process::Command::new("/bin/bash");
        cmd.arg("-c").arg(&req.command);
        cmd.current_dir(&req.cwd);
        cmd.stdin(std::process::Stdio::null());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);
        for (k, v) in &req.env {
            cmd.env(k, v);
        }

        let child = cmd
            .spawn()
            .map_err(|err| AgentToolError::ExecFailed(format!("spawn bash failed: {err}")))?;

        let started = Instant::now();
        let duration = Duration::from_millis(req.timeout_ms);
        let output = match tokio_timeout(duration, child.wait_with_output()).await {
            Err(_) => return Err(AgentToolError::Timeout),
            Ok(Err(err)) => {
                return Err(AgentToolError::ExecFailed(format!(
                    "wait bash output failed: {err}"
                )));
            }
            Ok(Ok(output)) => output,
        };
        let elapsed = started.elapsed();

        let stdout_str = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr_str = String::from_utf8_lossy(&output.stderr).to_string();
        let mut combined = Vec::with_capacity(output.stdout.len() + output.stderr.len());
        combined.extend_from_slice(&output.stdout);
        if !output.stdout.is_empty() && !output.stderr.is_empty() {
            combined.push(b'\n');
        }
        combined.extend_from_slice(&output.stderr);
        let (combined_str, output_truncated) = truncate_bytes(&combined, req.max_output_bytes);

        let exit_code = output.status.code().unwrap_or_else(|| {
            #[cfg(unix)]
            {
                use std::os::unix::process::ExitStatusExt;
                output.status.signal().map(|sig| 128 + sig).unwrap_or(-1)
            }
            #[cfg(not(unix))]
            {
                -1
            }
        });

        Ok(BashRunOutput {
            exit_code,
            stdout: stdout_str,
            stderr: stderr_str,
            output: combined_str,
            output_truncated,
            duration_ms: elapsed.as_millis() as u64,
            engine: LOCAL_ENGINE.to_string(),
            cwd: req.cwd,
        })
    }
}

/// `AgentTool` implementation backed by an arbitrary [`BashRunner`].
pub struct ExecBashTool {
    config: LlmBashConfig,
    runner: Arc<dyn BashRunner>,
}

impl ExecBashTool {
    pub fn new(config: LlmBashConfig) -> Self {
        Self {
            config,
            runner: Arc::new(LocalProcessBashRunner::new()),
        }
    }

    pub fn with_runner(config: LlmBashConfig, runner: Arc<dyn BashRunner>) -> Self {
        Self { config, runner }
    }

    pub fn local_workspace(workspace: impl Into<PathBuf>) -> Self {
        Self::new(LlmBashConfig::local_workspace(workspace))
    }

    pub fn config(&self) -> &LlmBashConfig {
        &self.config
    }

    fn tool_name(&self) -> &str {
        self.config.tool_name.as_str()
    }

    fn resolve_cwd(&self, raw: Option<&str>) -> Result<PathBuf, AgentToolError> {
        let workspace = to_abs_path(&self.config.workspace)?;
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            Some(value) => {
                let resolved = resolve_path_under_root(&workspace, value)?;
                if !resolved.exists() {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "cwd does not exist: {}",
                        resolved.display()
                    )));
                }
                Ok(resolved)
            }
            None => Ok(normalize_abs_path(&workspace)),
        }
    }

    fn resolve_timeout(&self, raw: Option<u64>) -> u64 {
        let max = self.config.max_timeout_ms.max(1);
        let candidate = raw.unwrap_or(self.config.default_timeout_ms);
        candidate.clamp(1, max)
    }

    fn parse_env(&self, raw: Option<&Json>) -> Result<Vec<(String, String)>, AgentToolError> {
        let Some(value) = raw else {
            return Ok(Vec::new());
        };
        if value.is_null() {
            return Ok(Vec::new());
        }
        let map = value.as_object().ok_or_else(|| {
            AgentToolError::InvalidArgs("`env` must be an object of string=>string".to_string())
        })?;
        if map.is_empty() {
            return Ok(Vec::new());
        }
        if !self.config.allow_env {
            return Err(AgentToolError::InvalidArgs(
                "env passing is disabled for this exec_bash tool".to_string(),
            ));
        }
        let mut out = Vec::with_capacity(map.len());
        for (key, val) in map {
            if !is_valid_shell_env_key(key) {
                return Err(AgentToolError::InvalidArgs(format!(
                    "invalid env key `{key}` (must match [A-Za-z_][A-Za-z0-9_]*)"
                )));
            }
            let value_str = match val {
                Json::String(s) => s.clone(),
                Json::Number(n) => n.to_string(),
                Json::Bool(b) => b.to_string(),
                Json::Null => String::new(),
                other => {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "env value for `{key}` must be string/number/bool, got {}",
                        other
                    )));
                }
            };
            out.push((key.clone(), value_str));
        }
        Ok(out)
    }

    fn build_details(
        &self,
        command: &str,
        target: &BashTarget,
        output: &BashRunOutput,
    ) -> Json {
        json!({
            "command": command,
            "target": target.label(),
            "cwd": output.cwd.to_string_lossy().to_string(),
            "exit_code": output.exit_code,
            "stdout": output.stdout,
            "stderr": output.stderr,
            "output": output.output,
            "output_truncated": output.output_truncated,
            "duration_ms": output.duration_ms,
            "engine": output.engine,
        })
    }
}

#[async_trait]
impl AgentTool for ExecBashTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.tool_name().to_string(),
            description: "Run a shell command in the workspace via /bin/bash -lc.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "shell command to execute"
                    },
                    "timeout_ms": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "per-call timeout in ms; clamped to the tool's max_timeout_ms"
                    },
                    "cwd": {
                        "type": "string",
                        "description": "working directory; must live under the configured workspace"
                    },
                    "env": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "extra env vars; keys must match [A-Za-z_][A-Za-z0-9_]*"
                    },
                    "target": {
                        "type": "string",
                        "description": "execution target; only `local` is supported in this stage"
                    }
                },
                "required": ["command"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string"},
                    "target": {"type": "string"},
                    "cwd": {"type": "string"},
                    "exit_code": {"type": "integer"},
                    "stdout": {"type": "string"},
                    "stderr": {"type": "string"},
                    "output": {"type": "string"},
                    "output_truncated": {"type": "boolean"},
                    "duration_ms": {"type": "integer"},
                    "engine": {"type": "string"}
                }
            }),
            usage: Some(format!(
                "{name} command='<shell>' [cwd=<path>] [timeout_ms=<ms>] [env={{\"K\":\"V\"}}] [target=local]",
                name = self.tool_name()
            )),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let map = match args {
            Json::Object(map) => map,
            Json::Null => JsonMap::new(),
            other => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "exec_bash args must be a json object, got {}",
                    other
                )));
            }
        };

        let command = map
            .get("command")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                AgentToolError::InvalidArgs("`command` is required and must be non-empty".into())
            })?
            .to_string();

        let raw_cwd = map.get("cwd").and_then(Json::as_str);
        let cwd = self.resolve_cwd(raw_cwd)?;

        let timeout_ms = self.resolve_timeout(map.get("timeout_ms").and_then(Json::as_u64));

        let user_env = self.parse_env(map.get("env"))?;

        let target = match map.get("target") {
            Some(Json::Null) | None => self.config.target.resolve(),
            Some(Json::String(s)) => BashTargetSpec::parse(Some(s.as_str())),
            Some(other) => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "`target` must be a string, got {}",
                    other
                )));
            }
        };
        if let BashTarget::Unsupported(value) = &target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported exec_bash target `{value}` (only local is supported)"
            )));
        }

        let env = prepare_overlay_env(&self.config.overlay, &user_env);

        let request = BashRunRequest {
            command: command.clone(),
            cwd,
            timeout_ms,
            max_output_bytes: self.config.max_output_bytes,
            env,
            target: target.clone(),
        };

        let output = self.runner.run(ctx, request).await?;

        let summary = if output.exit_code == 0 {
            format!("exit=0 in {}ms", output.duration_ms)
        } else {
            format!(
                "exit={} in {}ms",
                output.exit_code, output.duration_ms
            )
        };

        let details = self.build_details(&command, &target, &output);
        let mut result = build_builtin_tool_result(details, command.clone(), summary)
            .with_tool(self.tool_name())
            .with_status(AgentToolStatus::Success)
            .with_return_code(output.exit_code);
        if !output.output.is_empty() {
            result = result.with_output(output.output.clone());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::Path;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use tempfile::tempdir;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            step_idx: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
        }
    }

    fn ws() -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let workspace = dir.path().join("workspace");
        fs::create_dir_all(&workspace).expect("mkdir workspace");
        (dir, workspace)
    }

    #[tokio::test]
    async fn local_pwd_runs_inside_workspace() {
        let (_dir, workspace) = ws();
        let tool = ExecBashTool::local_workspace(workspace.clone());

        let result = tool
            .call(&ctx(), json!({ "command": "pwd" }))
            .await
            .expect("call ok");

        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(result.details["exit_code"], 0);
        let stdout = result.details["stdout"].as_str().unwrap().trim();
        let canonical_ws = fs::canonicalize(&workspace).expect("canonicalize");
        let canonical_stdout = fs::canonicalize(Path::new(stdout)).expect("canonicalize stdout");
        assert_eq!(canonical_stdout, canonical_ws);
        assert_eq!(result.details["target"], "local");
        assert_eq!(result.details["engine"], LOCAL_ENGINE);
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn bin_overlay_shadows_system_path() {
        let (dir, workspace) = ws();
        let bin_dir = dir.path().join("bin");
        fs::create_dir_all(&bin_dir).expect("mkdir bin");

        // 1) A unique shim name nothing on PATH could provide — proves overlay
        //    actually gets used.
        let unique_path = bin_dir.join("llm_bash_overlay_probe");
        fs::write(&unique_path, "#!/bin/sh\necho UNIQUE_HIT\n").expect("write unique shim");
        let mut perms = fs::metadata(&unique_path).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&unique_path, perms).expect("chmod");

        // 2) A shim that shadows `cat` — proves the overlay wins over the
        //    system PATH when both have the binary.
        let cat_shim = bin_dir.join("cat");
        fs::write(&cat_shim, "#!/bin/sh\necho SHIM_CAT_WINS\n").expect("write cat shim");
        let mut perms = fs::metadata(&cat_shim).expect("meta").permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&cat_shim, perms).expect("chmod");

        let cfg = LlmBashConfig::local_workspace(workspace)
            .with_overlay(BinOverlayConfig::local(bin_dir));
        let tool = ExecBashTool::new(cfg);

        let unique_result = tool
            .call(&ctx(), json!({ "command": "llm_bash_overlay_probe" }))
            .await
            .expect("call ok");
        let unique_stdout = unique_result.details["stdout"].as_str().unwrap();
        assert!(
            unique_stdout.contains("UNIQUE_HIT"),
            "overlay shim not found, got: {unique_stdout}"
        );

        let shadow_result = tool
            .call(&ctx(), json!({ "command": "cat /dev/null" }))
            .await
            .expect("call ok");
        let shadow_stdout = shadow_result.details["stdout"].as_str().unwrap();
        assert!(
            shadow_stdout.contains("SHIM_CAT_WINS"),
            "overlay should win over system cat, got: {shadow_stdout}"
        );
    }

    #[tokio::test]
    async fn cwd_outside_workspace_rejected() {
        let (dir, workspace) = ws();
        let outside = dir.path().join("outside");
        fs::create_dir_all(&outside).expect("mkdir outside");
        let tool = ExecBashTool::local_workspace(workspace);

        let err = tool
            .call(
                &ctx(),
                json!({
                    "command": "pwd",
                    "cwd": outside.to_string_lossy().to_string()
                }),
            )
            .await
            .expect_err("should reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn invalid_env_key_rejected() {
        let (_dir, workspace) = ws();
        let tool = ExecBashTool::local_workspace(workspace);

        let err = tool
            .call(
                &ctx(),
                json!({
                    "command": "true",
                    "env": { "1BAD": "value" }
                }),
            )
            .await
            .expect_err("should reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn allow_env_false_rejects_env_arg() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace).with_allow_env(false);
        let tool = ExecBashTool::new(cfg);

        let err = tool
            .call(
                &ctx(),
                json!({
                    "command": "true",
                    "env": { "FOO": "bar" }
                }),
            )
            .await
            .expect_err("should reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn non_zero_exit_is_success_with_exit_code() {
        let (_dir, workspace) = ws();
        let tool = ExecBashTool::local_workspace(workspace);

        let result = tool
            .call(&ctx(), json!({ "command": "exit 7" }))
            .await
            .expect("call ok despite non-zero exit");
        assert_eq!(result.status, AgentToolStatus::Success);
        assert_eq!(result.details["exit_code"], 7);
        assert_eq!(result.return_code, Some(7));
    }

    #[tokio::test]
    async fn timeout_returns_error() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace)
            .with_default_timeout_ms(120)
            .with_max_timeout_ms(200);
        let tool = ExecBashTool::new(cfg);

        let err = tool
            .call(&ctx(), json!({ "command": "sleep 5" }))
            .await
            .expect_err("must time out");
        assert!(matches!(err, AgentToolError::Timeout), "got {err:?}");
    }

    #[tokio::test]
    async fn output_truncated_flag_is_set() {
        let (_dir, workspace) = ws();
        let cfg = LlmBashConfig::local_workspace(workspace).with_max_output_bytes(32);
        let tool = ExecBashTool::new(cfg);

        let result = tool
            .call(
                &ctx(),
                json!({ "command": "for i in $(seq 1 200); do echo -n abcdefghij; done" }),
            )
            .await
            .expect("call ok");
        assert_eq!(result.details["output_truncated"], true);
        let output = result.details["output"].as_str().unwrap();
        assert!(output.len() <= 32, "output should be truncated, got {} bytes", output.len());
    }

    #[tokio::test]
    async fn unsupported_target_rejected() {
        let (_dir, workspace) = ws();
        let tool = ExecBashTool::local_workspace(workspace);

        let err = tool
            .call(&ctx(), json!({ "command": "true", "target": "tmux" }))
            .await
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[test]
    fn parse_target_spec() {
        assert_eq!(BashTargetSpec::parse(None), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some("")), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some("local")), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some("LocalHost")), BashTarget::Local);
        assert_eq!(BashTargetSpec::parse(Some(".")), BashTarget::Local);
        assert!(matches!(
            BashTargetSpec::parse(Some("node-1")),
            BashTarget::Unsupported(_)
        ));
    }

    #[test]
    fn overlay_env_prepends_bin_dir() {
        let overlay = BinOverlayConfig::local("/tmp/llm_bash_overlay");
        let env = prepare_overlay_env(
            &overlay,
            &[("PATH".to_string(), "/usr/bin:/bin".to_string())],
        );
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.as_str())
            .unwrap_or_default();
        assert!(path.starts_with("/tmp/llm_bash_overlay:"), "got PATH={path}");

        let env_disabled =
            prepare_overlay_env(&BinOverlayConfig::disabled(), &[("PATH".into(), "/p".into())]);
        let path2 = env_disabled
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(path2, "/p");
    }

    #[test]
    fn overlay_env_stacks_multiple_layers_in_priority_order() {
        let overlay = BinOverlayConfig::layered(["/a/session", "/a/agent", "/a/runtime", "/a/system"]);
        let env = prepare_overlay_env(
            &overlay,
            &[("PATH".to_string(), "/usr/bin:/bin".to_string())],
        );
        let path = env
            .iter()
            .find(|(k, _)| k == "PATH")
            .map(|(_, v)| v.clone())
            .unwrap();
        assert_eq!(path, "/a/session:/a/agent:/a/runtime:/a/system:/usr/bin:/bin");
    }
}
