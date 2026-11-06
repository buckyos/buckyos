use super::{function, CAPABILITIES};
use crate::native::{ExecutionLane, NativeContext, NativeFunction, NativeOutcome};
use async_trait::async_trait;
use buckyos_api::{FunctionObject, ThunkExecutionError};
use serde_json::{json, Value};

pub fn function_object() -> FunctionObject {
    function(
        CAPABILITIES,
        false,
        &[],
        json!({"type": "object", "additionalProperties": false, "properties": {}}),
        "media.capabilities",
        json!({
            "type": "object",
            "required": ["revision", "tools", "missing"],
            "properties": {
                "revision": {"type": "string"},
                "tools": {"type": "object"},
                "missing": {"type": "array", "items": {"type": "string"}}
            }
        }),
    )
}

pub struct Capabilities;

#[async_trait]
impl NativeFunction for Capabilities {
    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Light
    }

    async fn invoke(&self, ctx: &NativeContext, _params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let tools = ctx
            .env
            .tools
            .iter()
            .map(|(name, info)| (name.clone(), json!({"version": info.version, "path": info.path})))
            .collect::<serde_json::Map<_, _>>();
        Ok(NativeOutcome::new(json!({
            "revision": ctx.env.revision,
            "tools": tools,
            "missing": ctx.env.missing,
        }))
        .not_cacheable())
    }
}
