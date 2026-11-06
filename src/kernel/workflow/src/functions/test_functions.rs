//! `test.echo` / `test.sleep` / `test.fail`: the minimal native functions the
//! FunctionObject / thunk channel is verified with (TODO T1g).

use crate::error::WorkflowResult;
use crate::function_registry::{FunctionRegistry, RegistrationSpec};
use crate::native::{NativeContext, NativeFunction, NativeOutcome};
use async_trait::async_trait;
use buckyos_api::{
    error_codes, FunctionObject, FunctionResultType, FunctionType, ThunkExecutionError,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

pub const TEST_ECHO: &str = "test.echo";
pub const TEST_SLEEP: &str = "test.sleep";
pub const TEST_FAIL: &str = "test.fail";

fn function(content: &str, is_pure: bool, params_schema: Value, result_schema: Value) -> FunctionObject {
    FunctionObject {
        func_type: FunctionType::Native,
        content: content.to_string(),
        is_pure,
        params_type: Default::default(),
        params_schema,
        result_type: FunctionResultType::Fixed("object".to_string()),
        result_schema,
    }
}

/// Returns `{echo: <value>, calls: n}` where `n` counts real invocations; a
/// cache hit never increments it, which is what the cache tests observe.
#[derive(Default)]
pub struct Echo {
    calls: std::sync::atomic::AtomicU64,
}

impl Echo {
    #[cfg(test)]
    pub fn calls(&self) -> u64 {
        self.calls.load(std::sync::atomic::Ordering::SeqCst)
    }
}

#[async_trait]
impl NativeFunction for Echo {
    async fn invoke(&self, _ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let n = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        Ok(NativeOutcome::new(json!({
            "echo": params.get("value").cloned().unwrap_or(Value::Null),
            "calls": n,
        })))
    }
}

/// Sleeps `ms` milliseconds, honouring cancellation and the deadline.
pub struct Sleep;

#[async_trait]
impl NativeFunction for Sleep {
    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let ms = params.get("ms").and_then(Value::as_u64).unwrap_or(0);
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(ms)) => {}
            _ = ctx.cancel.cancelled() => {
                return Err(ThunkExecutionError::new(error_codes::CANCELLED, "sleep cancelled"));
            }
        }
        Ok(NativeOutcome::new(json!({"slept_ms": ms})))
    }
}

/// Always fails with the requested code (default `tool_failed`).
pub struct Fail;

#[async_trait]
impl NativeFunction for Fail {
    async fn invoke(&self, _ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let code = params
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or(error_codes::TOOL_FAILED)
            .to_string();
        let retryable = params.get("retryable").and_then(Value::as_bool).unwrap_or(false);
        let mut error = ThunkExecutionError::new(code, "test.fail requested failure")
            .with_details(json!({"params": params}));
        if retryable {
            error = error.retryable();
        }
        Err(error)
    }
}

pub fn register(registry: &mut FunctionRegistry) -> WorkflowResult<()> {
    register_with_echo(registry, Arc::new(Echo::default()))
}

pub fn register_with_echo(registry: &mut FunctionRegistry, echo: Arc<Echo>) -> WorkflowResult<()> {
    registry.register_native(
        RegistrationSpec::new(TEST_ECHO, "1")
            .description("Returns its `value` param; counts real invocations")
            .timeout(30),
        function(
            "test.echo/impl-1",
            true,
            json!({"type": "object", "properties": {"value": {}}}),
            json!({"type": "object", "required": ["echo"], "properties": {"echo": {}, "calls": {"type": "integer"}}}),
        ),
        echo,
    )?;
    registry.register_native(
        RegistrationSpec::new(TEST_SLEEP, "1")
            .description("Sleeps `ms` milliseconds")
            .timeout(60),
        function(
            "test.sleep/impl-1",
            false,
            json!({"type": "object", "properties": {"ms": {"type": "integer"}}}),
            json!({"type": "object", "required": ["slept_ms"]}),
        ),
        Arc::new(Sleep),
    )?;
    registry.register_native(
        RegistrationSpec::new(TEST_FAIL, "1")
            .description("Fails with `code`")
            .timeout(30),
        function(
            "test.fail/impl-1",
            true,
            json!({"type": "object", "properties": {"code": {"type": "string"}, "retryable": {"type": "boolean"}}}),
            json!({"type": "object"}),
        ),
        Arc::new(Fail),
    )?;
    Ok(())
}
