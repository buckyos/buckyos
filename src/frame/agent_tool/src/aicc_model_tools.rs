use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{AgentToolError, ToolCtx, TypedTool};

pub const TOOL_MAKE_EXACT_MODEL: &str = "make_exact_model";
pub const TOOL_PARSE_EXACT_MODEL: &str = "parse_exact_model";

#[derive(Clone, Debug, Default)]
pub struct MakeExactModelTool;

impl MakeExactModelTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Clone, Debug, Default)]
pub struct ParseExactModelTool;

impl ParseExactModelTool {
    pub fn new() -> Self {
        Self
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct MakeExactModelArgs {
    #[schemars(description = "Provider-side model ID, for example gemini-3.1-pro-preview.")]
    pub provider_model_id: String,
    #[schemars(description = "AICC provider instance name, for example google-gemini-main.")]
    pub provider_instance_name: String,
    #[serde(default)]
    #[schemars(description = "Optional AICC model variant, for example reasoning-high.")]
    pub variant: Option<String>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct MakeExactModelOutput {
    pub exact_model: String,
    pub provider_model_id: String,
    pub provider_instance_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[derive(Clone, Debug, Deserialize, JsonSchema)]
pub struct ParseExactModelArgs {
    #[schemars(
        description = "AICC exact model in <provider_model_id>[:variant]@<provider_instance_name> form."
    )]
    pub exact_model: String,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ParseExactModelOutput {
    pub exact_model: String,
    pub provider_model_id: String,
    pub provider_instance_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant: Option<String>,
}

#[async_trait]
impl TypedTool for MakeExactModelTool {
    type Args = MakeExactModelArgs;
    type Output = MakeExactModelOutput;

    fn name(&self) -> &str {
        TOOL_MAKE_EXACT_MODEL
    }

    fn description(&self) -> &str {
        "Build an AICC exact_model from provider_model_id, provider_instance_name, and optional variant."
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        output.exact_model.clone()
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let provider_model_id = require_part("provider_model_id", &args.provider_model_id)?;
        let provider_instance_name =
            require_part("provider_instance_name", &args.provider_instance_name)?;
        let variant = args
            .variant
            .as_deref()
            .map(|value| require_part("variant", value))
            .transpose()?;
        let exact_model = match variant.as_deref() {
            Some(variant) => format!("{provider_model_id}:{variant}@{provider_instance_name}"),
            None => format!("{provider_model_id}@{provider_instance_name}"),
        };
        Ok(MakeExactModelOutput {
            exact_model,
            provider_model_id,
            provider_instance_name,
            variant,
        })
    }
}

#[async_trait]
impl TypedTool for ParseExactModelTool {
    type Args = ParseExactModelArgs;
    type Output = ParseExactModelOutput;

    fn name(&self) -> &str {
        TOOL_PARSE_EXACT_MODEL
    }

    fn description(&self) -> &str {
        "Parse an AICC exact_model into provider_model_id, provider_instance_name, and optional variant."
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        format!(
            "{} via {}",
            output.provider_model_id, output.provider_instance_name
        )
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let exact_model = args.exact_model.trim();
        let (model_part, provider_instance_name) =
            exact_model.split_once('@').ok_or_else(|| {
                AgentToolError::InvalidArgs(
                    "exact_model must be `<provider_model_id>[:variant]@<provider_instance_name>`"
                        .to_string(),
                )
            })?;
        if provider_instance_name.contains('@') {
            return Err(AgentToolError::InvalidArgs(
                "exact_model must contain exactly one `@`".to_string(),
            ));
        }
        let provider_instance_name =
            require_part("provider_instance_name", provider_instance_name)?;
        let (provider_model_id, variant) = match model_part.split_once(':') {
            Some((model, variant)) => (
                require_part("provider_model_id", model)?,
                Some(require_part("variant", variant)?),
            ),
            None => (require_part("provider_model_id", model_part)?, None),
        };
        Ok(ParseExactModelOutput {
            exact_model: exact_model.to_string(),
            provider_model_id,
            provider_instance_name,
            variant,
        })
    }
}

fn require_part(field: &str, value: &str) -> Result<String, AgentToolError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "{field} must be non-empty"
        )));
    }
    if value.contains('@') {
        return Err(AgentToolError::InvalidArgs(format!(
            "{field} must not contain `@`"
        )));
    }
    Ok(value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SessionRuntimeContext, TypedToolHandle};

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace".into(),
            agent_name: "agent".into(),
            behavior: "behavior".into(),
            step_idx: 0,
            wakeup_id: "wakeup".into(),
            session_id: "session".into(),
            read_token_limit: 0,
        }
    }

    #[tokio::test]
    async fn make_exact_model_requires_provider_instance_name() {
        let tool = TypedToolHandle::with_null_host(MakeExactModelTool::new());
        let err = crate::AgentTool::call(
            &tool,
            &ctx(),
            serde_json::json!({
                "provider_model_id": "gemini-3.1-pro-preview",
                "provider_instance_name": ""
            }),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn make_and_parse_exact_model_with_variant() {
        let make = TypedToolHandle::with_null_host(MakeExactModelTool::new());
        let made = crate::AgentTool::call(
            &make,
            &ctx(),
            serde_json::json!({
                "provider_model_id": "gemini-3.1-pro-preview",
                "provider_instance_name": "google-gemini-main",
                "variant": "reasoning-high"
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            made.details["exact_model"],
            "gemini-3.1-pro-preview:reasoning-high@google-gemini-main"
        );

        let parse = TypedToolHandle::with_null_host(ParseExactModelTool::new());
        let parsed = crate::AgentTool::call(
            &parse,
            &ctx(),
            serde_json::json!({
                "exact_model": made.details["exact_model"]
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            parsed.details["provider_model_id"],
            "gemini-3.1-pro-preview"
        );
        assert_eq!(
            parsed.details["provider_instance_name"],
            "google-gemini-main"
        );
        assert_eq!(parsed.details["variant"], "reasoning-high");
    }
}
