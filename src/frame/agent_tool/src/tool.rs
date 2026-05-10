//! Typed tool trait and runtime context.
//!
//! - [`TypedTool`] is the user-facing trait for typed implementations.
//!   [`TypedToolHandle`] provides the runtime-erased `AgentTool`
//!   interface used by the manager.
//! - [`CallingConventions`] is a bitflag value that replaces the three
//!   `support_bash`/`support_action`/`support_llm_tool_call` booleans.
//! - [`ToolHost`] collapses the per-feature backend traits into one
//!   accessor surface so a tool only holds `Arc<dyn ToolHost>`.
//! - [`ToolCtx`] is the runtime context passed at execution time
//!   (session info + host + shell cwd).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use schemars::{schema_for, JsonSchema};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value as Json;

use crate::file_tools::FileWriteAuditBackend;
use crate::workspace::WorkspaceRuntimeBackend;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult,
    ExternalWorkspaceBackend, SessionRuntimeContext, SessionViewBackend, ToolSpec,
    WorklogActionBackend, WorkspaceToolBackend,
};

/// Bitflag summary of how a tool may be invoked.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallingConventions(u8);

impl CallingConventions {
    pub const EMPTY: Self = Self(0);
    pub const BASH: Self = Self(0b001);
    pub const ACTION: Self = Self(0b010);
    pub const LLM: Self = Self(0b100);
    pub const ALL: Self = Self(0b111);

    pub const fn from_legacy(bash: bool, action: bool, llm: bool) -> Self {
        let mut bits = 0u8;
        if bash {
            bits |= Self::BASH.0;
        }
        if action {
            bits |= Self::ACTION.0;
        }
        if llm {
            bits |= Self::LLM.0;
        }
        Self(bits)
    }

    pub const fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0 && other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn supports_bash(self) -> bool {
        (self.0 & Self::BASH.0) != 0
    }

    pub const fn supports_action(self) -> bool {
        (self.0 & Self::ACTION.0) != 0
    }

    pub const fn supports_llm_tool_call(self) -> bool {
        (self.0 & Self::LLM.0) != 0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }
}

impl std::ops::BitOr for CallingConventions {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for CallingConventions {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

/// Unified accessor surface for runtime services that tools depend on.
///
/// Each method returns `None` by default so concrete hosts only override
/// the slots they actually wire up. Tools call `ctx.host().memory_load()`
/// and degrade gracefully when the slot is empty.
pub trait ToolHost: Send + Sync {
    fn session_view(&self) -> Option<&dyn SessionViewBackend> {
        None
    }
    fn workspace_runtime(&self) -> Option<&dyn WorkspaceRuntimeBackend> {
        None
    }
    fn workspace_tool(&self) -> Option<&dyn WorkspaceToolBackend> {
        None
    }
    fn external_workspace(&self) -> Option<&dyn ExternalWorkspaceBackend> {
        None
    }
    fn worklog_action(&self) -> Option<&dyn WorklogActionBackend> {
        None
    }
    fn file_write_audit(&self) -> Option<&dyn FileWriteAuditBackend> {
        None
    }
}

/// Empty host. Used as the default when nobody has set a host on the
/// tool manager.
pub struct NullToolHost;
impl ToolHost for NullToolHost {}

fn null_host() -> Arc<dyn ToolHost> {
    Arc::new(NullToolHost)
}

/// Composes individual backend Arcs into a single `ToolHost`.
#[derive(Default, Clone)]
pub struct BasicToolHost {
    pub session_view: Option<Arc<dyn SessionViewBackend>>,
    pub workspace_runtime: Option<Arc<dyn WorkspaceRuntimeBackend>>,
    pub workspace_tool: Option<Arc<dyn WorkspaceToolBackend>>,
    pub external_workspace: Option<Arc<dyn ExternalWorkspaceBackend>>,
    pub worklog_action: Option<Arc<dyn WorklogActionBackend>>,
    pub file_write_audit: Option<Arc<dyn FileWriteAuditBackend>>,
}

impl BasicToolHost {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_session_view(mut self, backend: Arc<dyn SessionViewBackend>) -> Self {
        self.session_view = Some(backend);
        self
    }
    pub fn with_workspace_runtime(mut self, backend: Arc<dyn WorkspaceRuntimeBackend>) -> Self {
        self.workspace_runtime = Some(backend);
        self
    }
    pub fn with_workspace_tool(mut self, backend: Arc<dyn WorkspaceToolBackend>) -> Self {
        self.workspace_tool = Some(backend);
        self
    }
    pub fn with_external_workspace(mut self, backend: Arc<dyn ExternalWorkspaceBackend>) -> Self {
        self.external_workspace = Some(backend);
        self
    }
    pub fn with_worklog_action(mut self, backend: Arc<dyn WorklogActionBackend>) -> Self {
        self.worklog_action = Some(backend);
        self
    }
    pub fn with_file_write_audit(mut self, backend: Arc<dyn FileWriteAuditBackend>) -> Self {
        self.file_write_audit = Some(backend);
        self
    }
}

impl ToolHost for BasicToolHost {
    fn session_view(&self) -> Option<&dyn SessionViewBackend> {
        self.session_view.as_deref()
    }
    fn workspace_runtime(&self) -> Option<&dyn WorkspaceRuntimeBackend> {
        self.workspace_runtime.as_deref()
    }
    fn workspace_tool(&self) -> Option<&dyn WorkspaceToolBackend> {
        self.workspace_tool.as_deref()
    }
    fn external_workspace(&self) -> Option<&dyn ExternalWorkspaceBackend> {
        self.external_workspace.as_deref()
    }
    fn worklog_action(&self) -> Option<&dyn WorklogActionBackend> {
        self.worklog_action.as_deref()
    }
    fn file_write_audit(&self) -> Option<&dyn FileWriteAuditBackend> {
        self.file_write_audit.as_deref()
    }
}

/// Where the value for a CLI content field comes from. `Inline` is the
/// raw flag value; `Stdin` defers reading until dispatch.
#[derive(Clone, Debug)]
pub enum ContentInput {
    Inline(String),
    Stdin,
}

/// What the CLI dispatcher should do once a tool has parsed its argv.
/// `Bash` reuses the tool's `exec` path; `Json` calls `call` after the
/// dispatcher resolves any optional stdin into the named field.
#[derive(Clone, Debug)]
pub enum CliInvocation {
    Bash {
        line: String,
    },
    Json {
        args: Json,
        content_input: Option<(String, ContentInput)>,
    },
}

/// Runtime context passed to typed tools.
pub struct ToolCtx<'a> {
    session: &'a SessionRuntimeContext,
    host: &'a dyn ToolHost,
    shell_cwd: Option<&'a Path>,
}

impl<'a> ToolCtx<'a> {
    pub fn new(session: &'a SessionRuntimeContext, host: &'a dyn ToolHost) -> Self {
        Self {
            session,
            host,
            shell_cwd: None,
        }
    }

    pub fn with_shell_cwd(mut self, cwd: Option<&'a Path>) -> Self {
        self.shell_cwd = cwd;
        self
    }

    pub fn session(&self) -> &SessionRuntimeContext {
        self.session
    }

    pub fn host(&self) -> &dyn ToolHost {
        self.host
    }

    pub fn shell_cwd(&self) -> Option<&Path> {
        self.shell_cwd
    }
}

/// Typed tool trait. The high-level trait for built-in and
/// dynamically-named tools alike. The runtime-erased `AgentTool`
/// interface is produced via [`TypedToolHandle`].
///
/// All metadata is exposed through `&self` methods so static tools
/// can return `&'static str` literals while dynamic tools (MCP) can
/// return values stored on the instance.
#[async_trait]
pub trait TypedTool: Send + Sync + 'static {
    type Args: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize + JsonSchema + Send;

    fn name(&self) -> &str;

    fn description(&self) -> &str {
        ""
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    /// Default impl derives the schema from `Self::Args` via `schemars`.
    /// Tools whose args are a runtime-defined `serde_json::Value` (MCP,
    /// Worklog) override this to supply a richer hand-written schema.
    fn args_schema(&self) -> Json {
        serde_json::to_value(schema_for!(Self::Args))
            .unwrap_or_else(|_| Json::Object(Default::default()))
    }

    fn output_schema(&self) -> Json {
        serde_json::to_value(schema_for!(Self::Output))
            .unwrap_or_else(|_| Json::Object(Default::default()))
    }

    fn usage(&self) -> Option<String> {
        None
    }

    /// Optional hook to render a per-call command line into the
    /// `AgentToolResult` metadata. Default returns `None`, meaning the
    /// bridge falls back to the tool name.
    fn build_cmd_line(&self, _args: &Self::Args) -> Option<String> {
        None
    }

    /// Optional hook for the post-call summary string.
    fn build_summary(&self, _output: &Self::Output) -> String {
        "ok".to_string()
    }

    /// Optional title hook. Returning `Some` overrides the default
    /// `derive_default_title` rendering.
    fn build_title(&self, _output: &Self::Output) -> Option<String> {
        None
    }

    /// Parse bash tokens (after the tool name) into a JSON arg object
    /// that `Self::Args` can deserialize from. Default produces the
    /// same key=value/JSON parsing as `parse_default_bash_exec_args`.
    fn parse_bash_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        crate::parse_default_bash_exec_args(tokens)
    }

    /// Parse a CLI argv (after the tool name) into either a bash-style
    /// invocation or a JSON args object with optional stdin pickup.
    /// Default just stitches tokens back into a bash line; tools whose
    /// CLI uses `--flag value` syntax override this.
    fn parse_cli_args(
        &self,
        tokens: &[String],
        _shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        Ok(CliInvocation::Bash {
            line: crate::build_bash_cli_line(self.name(), tokens),
        })
    }

    /// Returns true if the CLI should pipe non-JSON `stdout` for this
    /// tool, stripping the envelope. Currently only `read_file` opts in,
    /// when the caller is a non-interactive shell.
    fn cli_plain_text_stdout(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError>;
}

/// Adapter wrapping a `TypedTool` so it satisfies the `AgentTool`
/// trait used for type-erased dispatch by the manager.
pub struct TypedToolHandle<T: TypedTool> {
    inner: T,
    host: Arc<dyn ToolHost>,
}

impl<T: TypedTool> TypedToolHandle<T> {
    pub fn new(inner: T, host: Arc<dyn ToolHost>) -> Self {
        Self { inner, host }
    }

    pub fn with_null_host(inner: T) -> Self {
        Self {
            inner,
            host: null_host(),
        }
    }

    pub fn inner(&self) -> &T {
        &self.inner
    }
}

#[async_trait]
impl<T: TypedTool> AgentTool for TypedToolHandle<T> {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: self.inner.name().to_string(),
            description: self.inner.description().to_string(),
            args_schema: self.inner.args_schema(),
            output_schema: self.inner.output_schema(),
            usage: self.inner.usage(),
        }
    }

    fn calling(&self) -> CallingConventions {
        self.inner.calling()
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let typed: T::Args = serde_json::from_value(args).map_err(|err| {
            AgentToolError::InvalidArgs(format!("invalid args for `{}`: {err}", self.inner.name()))
        })?;
        let cmd_line = self
            .inner
            .build_cmd_line(&typed)
            .unwrap_or_else(|| self.inner.name().to_string());
        let tool_ctx = ToolCtx::new(ctx, self.host.as_ref());
        let output = self.inner.execute(&tool_ctx, typed).await?;
        finalize_typed_result(&self.inner, output, cmd_line)
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = crate::tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }
        let args = self.inner.parse_bash_args(&tokens[1..], shell_cwd)?;
        let typed: T::Args = serde_json::from_value(args).map_err(|err| {
            AgentToolError::InvalidArgs(format!("invalid args for `{}`: {err}", self.inner.name()))
        })?;
        let cmd_line = self
            .inner
            .build_cmd_line(&typed)
            .unwrap_or_else(|| line.trim().to_string());
        let tool_ctx = ToolCtx::new(ctx, self.host.as_ref()).with_shell_cwd(shell_cwd);
        let output = self.inner.execute(&tool_ctx, typed).await?;
        finalize_typed_result(&self.inner, output, cmd_line)
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        self.inner.parse_cli_args(tokens, shell_cwd)
    }

    fn cli_plain_text_stdout(&self) -> bool {
        self.inner.cli_plain_text_stdout()
    }
}

fn finalize_typed_result<T: TypedTool>(
    tool: &T,
    output: T::Output,
    cmd_line: String,
) -> Result<AgentToolResult, AgentToolError> {
    let summary = tool.build_summary(&output);
    let title = tool.build_title(&output);
    let detail = serde_json::to_value(&output).map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "serialize output for `{}` failed: {err}",
            tool.name()
        ))
    })?;
    let mut result = build_builtin_tool_result(detail, cmd_line, summary).with_tool(tool.name());
    if let Some(custom_title) = title {
        result.title = custom_title;
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn calling_conventions_bitflags_round_trip() {
        let cc = CallingConventions::from_legacy(true, false, true);
        assert!(cc.supports_bash());
        assert!(!cc.supports_action());
        assert!(cc.supports_llm_tool_call());
        assert_eq!(
            cc.bits(),
            CallingConventions::BASH.bits() | CallingConventions::LLM.bits()
        );

        let combined = CallingConventions::BASH | CallingConventions::LLM;
        assert!(combined.contains(CallingConventions::BASH));
        assert!(combined.contains(CallingConventions::LLM));
        assert!(!combined.contains(CallingConventions::ACTION));
        assert!(!CallingConventions::EMPTY.supports_bash());
    }

    #[derive(serde::Deserialize, JsonSchema)]
    struct EchoArgs {
        message: String,
    }

    #[derive(serde::Serialize, JsonSchema)]
    struct EchoOutput {
        echoed: String,
    }

    struct EchoTypedTool;

    #[async_trait]
    impl TypedTool for EchoTypedTool {
        type Args = EchoArgs;
        type Output = EchoOutput;

        fn name(&self) -> &str {
            "typed_echo"
        }

        fn description(&self) -> &str {
            "echo a message"
        }

        async fn execute(
            &self,
            _ctx: &ToolCtx<'_>,
            args: Self::Args,
        ) -> Result<Self::Output, AgentToolError> {
            Ok(EchoOutput {
                echoed: args.message,
            })
        }
    }

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

    #[tokio::test]
    async fn typed_tool_handle_bridges_to_agent_tool() {
        let handle = TypedToolHandle::with_null_host(EchoTypedTool);
        let result = AgentTool::call(&handle, &ctx(), json!({ "message": "hi" }))
            .await
            .expect("call");
        assert_eq!(result.details["echoed"], "hi");
        assert_eq!(result.summary, "ok");
        assert_eq!(result.agent_tool_protocol, "1");
    }

    #[tokio::test]
    async fn typed_tool_handle_invalid_args() {
        let handle = TypedToolHandle::with_null_host(EchoTypedTool);
        let err = AgentTool::call(&handle, &ctx(), json!({}))
            .await
            .expect_err("must fail");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }
}
