//! Stage 1/2 refactoring scaffold.
//!
//! - `TypedTool` is the new strongly-typed trait that tools should target.
//!   Tools provide associated `Args`/`Output` types plus a small set of
//!   `const`s; the legacy `AgentTool` interface (JSON in, JSON out) is
//!   produced automatically by [`TypedToolHandle`].
//! - `CallingConventions` replaces the three `support_*` booleans on the
//!   legacy trait with a single bitflag value.
//! - `ToolHost` collapses the per-feature backend traits
//!   (`SessionViewBackend`, `WorkspaceRuntimeBackend`,
//!   `FileWriteAuditBackend`, ...) into one accessor trait so a tool
//!   only needs to hold `Arc<dyn ToolHost>` instead of one Arc per
//!   backend.
//! - `ToolCtx` is the runtime context handed to typed tools at execution
//!   time (session info + host + shell cwd).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value as Json};

use crate::file_tools::FileWriteAuditBackend;
use crate::workspace::WorkspaceRuntimeBackend;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult, ExternalWorkspaceBackend,
    MemoryLoadBackend, MemoryMutationBackend, SessionRuntimeContext, SessionViewBackend, ToolSpec,
    WorklogActionBackend, WorkspaceToolBackend,
};

/// Bitflag summary of how a tool may be invoked.
///
/// Replaces `AgentTool::support_bash`/`support_action`/`support_llm_tool_call`
/// with a single flag value carried as a const on the typed trait.
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
    fn memory_load(&self) -> Option<&dyn MemoryLoadBackend> {
        None
    }
    fn memory_mutation(&self) -> Option<&dyn MemoryMutationBackend> {
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
/// tool manager. Tools that ask for a service from a `NullToolHost`
/// receive `None` and must error or fall back to a stub behavior.
pub struct NullToolHost;
impl ToolHost for NullToolHost {}

fn null_host() -> Arc<dyn ToolHost> {
    Arc::new(NullToolHost)
}

/// Composes individual backend Arcs into a single `ToolHost` so callers
/// who already hold typed backends can promote them in one step.
#[derive(Default, Clone)]
pub struct BasicToolHost {
    pub session_view: Option<Arc<dyn SessionViewBackend>>,
    pub workspace_runtime: Option<Arc<dyn WorkspaceRuntimeBackend>>,
    pub workspace_tool: Option<Arc<dyn WorkspaceToolBackend>>,
    pub external_workspace: Option<Arc<dyn ExternalWorkspaceBackend>>,
    pub memory_load: Option<Arc<dyn MemoryLoadBackend>>,
    pub memory_mutation: Option<Arc<dyn MemoryMutationBackend>>,
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
    pub fn with_memory_load(mut self, backend: Arc<dyn MemoryLoadBackend>) -> Self {
        self.memory_load = Some(backend);
        self
    }
    pub fn with_memory_mutation(mut self, backend: Arc<dyn MemoryMutationBackend>) -> Self {
        self.memory_mutation = Some(backend);
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
    fn memory_load(&self) -> Option<&dyn MemoryLoadBackend> {
        self.memory_load.as_deref()
    }
    fn memory_mutation(&self) -> Option<&dyn MemoryMutationBackend> {
        self.memory_mutation.as_deref()
    }
    fn worklog_action(&self) -> Option<&dyn WorklogActionBackend> {
        self.worklog_action.as_deref()
    }
    fn file_write_audit(&self) -> Option<&dyn FileWriteAuditBackend> {
        self.file_write_audit.as_deref()
    }
}

/// Runtime context passed to typed tools.
///
/// Carries the per-call `SessionRuntimeContext`, a pointer to the
/// shared host services, and an optional shell cwd that bash-driven
/// invocations want to thread through.
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

/// New typed tool trait. Tools targeting this trait declare their input
/// and output types directly; argument deserialization, output
/// serialization and the legacy `AgentTool` plumbing are produced by
/// [`TypedToolHandle`] at registration time.
///
/// Stage 1 keeps `args_schema` / `output_schema` as plain JSON returned
/// by overrideable functions; later stages can swap the defaults to
/// schemars-derived schemas without changing this trait.
#[async_trait]
pub trait TypedTool: Send + Sync + 'static {
    type Args: DeserializeOwned + Send;
    type Output: Serialize + Send;

    const NAME: &'static str;
    const DESCRIPTION: &'static str;
    const CALLING: CallingConventions = CallingConventions::ALL;

    fn args_schema() -> Json {
        json!({"type": "object"})
    }

    fn output_schema() -> Json {
        json!({"type": "object"})
    }

    fn usage() -> Option<&'static str> {
        None
    }

    /// Optional hook to render a per-call command line into the
    /// AgentToolResult metadata. Tools can use the args to generate a
    /// human-readable invocation snippet; default returns `None`,
    /// meaning the bridge falls back to the tool name.
    fn build_cmd_line(_args: &Self::Args) -> Option<String> {
        None
    }

    /// Optional hook for the post-call summary string in the result.
    fn build_summary(_output: &Self::Output) -> String {
        "ok".to_string()
    }

    async fn execute(
        &self,
        ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError>;
}

/// Adapter wrapping a `TypedTool` so it satisfies the legacy
/// `AgentTool` trait. The host pointer is captured at construction
/// time and threaded into every `execute` call via `ToolCtx`.
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
            name: T::NAME.to_string(),
            description: T::DESCRIPTION.to_string(),
            args_schema: T::args_schema(),
            output_schema: T::output_schema(),
            usage: T::usage().map(|s| s.to_string()),
        }
    }

    fn support_bash(&self) -> bool {
        T::CALLING.supports_bash()
    }

    fn support_action(&self) -> bool {
        T::CALLING.supports_action()
    }

    fn support_llm_tool_call(&self) -> bool {
        T::CALLING.supports_llm_tool_call()
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let typed: T::Args = serde_json::from_value(args).map_err(|err| {
            AgentToolError::InvalidArgs(format!("invalid args for `{}`: {err}", T::NAME))
        })?;
        let cmd_line = T::build_cmd_line(&typed).unwrap_or_else(|| T::NAME.to_string());
        let tool_ctx = ToolCtx::new(ctx, self.host.as_ref());
        let output = self.inner.execute(&tool_ctx, typed).await?;
        let summary = T::build_summary(&output);
        let detail = serde_json::to_value(&output).map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "serialize output for `{}` failed: {err}",
                T::NAME
            ))
        })?;
        Ok(build_builtin_tool_result(detail, cmd_line, summary).with_tool(T::NAME.to_string()))
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        // Use the default tokenizer + key=value/JSON arg parser, then
        // route through `call`. Per-tool bash quirks are migrated as
        // tools opt into the typed trait (see Stage 3 in the plan).
        let _ = (ctx, shell_cwd);
        let tokens = crate::tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }
        let args = crate::parse_default_bash_exec_args(&tokens[1..])?;
        // Re-enter through `call` so the same arg-parse / context flow
        // applies; we cannot pass shell_cwd through the legacy bridge
        // yet — that arrives in stage 3 when typed tools own their
        // bash entry point too.
        AgentTool::call(self, ctx, args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calling_conventions_bitflags_round_trip() {
        let cc = CallingConventions::from_legacy(true, false, true);
        assert!(cc.supports_bash());
        assert!(!cc.supports_action());
        assert!(cc.supports_llm_tool_call());
        assert_eq!(cc.bits(), CallingConventions::BASH.bits() | CallingConventions::LLM.bits());

        let combined = CallingConventions::BASH | CallingConventions::LLM;
        assert!(combined.contains(CallingConventions::BASH));
        assert!(combined.contains(CallingConventions::LLM));
        assert!(!combined.contains(CallingConventions::ACTION));
        assert!(!CallingConventions::EMPTY.supports_bash());
    }

    #[derive(serde::Deserialize)]
    struct EchoArgs {
        message: String,
    }

    #[derive(serde::Serialize)]
    struct EchoOutput {
        echoed: String,
    }

    struct EchoTypedTool;

    #[async_trait]
    impl TypedTool for EchoTypedTool {
        type Args = EchoArgs;
        type Output = EchoOutput;

        const NAME: &'static str = "typed_echo";
        const DESCRIPTION: &'static str = "echo a message";
        const CALLING: CallingConventions = CallingConventions::ALL;

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
        assert!(result.is_agent_tool);
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
