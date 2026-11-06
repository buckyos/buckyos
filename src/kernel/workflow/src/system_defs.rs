//! System built-in definitions (TODO T1a / T3): the four `media.*`
//! pipelines ship with the service, are seeded at startup, carry
//! `source: system`, are read-only, and bind every `/tool/` link to a real
//! FunctionObject before the service accepts requests.

use crate::analysis::{analyze_workflow, trigger_input_schema};
use crate::compiler::compile_workflow_with;
use crate::error::{WorkflowError, WorkflowResult};
use crate::function_registry::FunctionRegistry;
use crate::state::{DefinitionRecord, DefinitionSource, DefinitionStore, Owner};
use buckyos_api::WorkflowDefinition;
use serde_json::Value;
use std::sync::Arc;

pub const MEDIA_IMAGE_JSON: &str = include_str!("../defs/media.image.json");
pub const MEDIA_PDF_JSON: &str = include_str!("../defs/media.pdf.json");
pub const MEDIA_VIDEO_JSON: &str = include_str!("../defs/media.video.json");
pub const MEDIA_DOCUMENT_JSON: &str = include_str!("../defs/media.document.json");

pub const SYSTEM_DEFINITIONS: [&str; 4] = [
    MEDIA_IMAGE_JSON,
    MEDIA_PDF_JSON,
    MEDIA_VIDEO_JSON,
    MEDIA_DOCUMENT_JSON,
];

/// Media definitions must satisfy the static constraints of TODO T1e before
/// they are seeded: explicit `idempotent`, `max_attempts = 1`,
/// `fallback = abort`, every executor a `/tool/media.*` link, and a trigger
/// input schema that never lets callers set `content_id`.
pub fn validate_media_definition(raw: &Value) -> Result<(), String> {
    let id = raw.get("id").and_then(Value::as_str).unwrap_or_default();
    if !id.starts_with("media.") {
        return Err(format!("id `{id}` must start with `media.`"));
    }
    if raw.get("name").and_then(Value::as_str) != Some(id) {
        return Err("name must equal id".to_string());
    }
    let trigger_schema = raw
        .pointer("/trigger/input_schema")
        .ok_or("trigger.input_schema is required")?;
    if trigger_schema
        .pointer("/properties/content_id")
        .is_some()
    {
        return Err("trigger.input_schema must not expose content_id".to_string());
    }
    if trigger_schema.get("additionalProperties") != Some(&Value::Bool(false)) {
        return Err("trigger.input_schema must set additionalProperties=false".to_string());
    }
    let steps = raw
        .get("steps")
        .and_then(Value::as_array)
        .ok_or("steps missing")?;
    for step in steps {
        let step_id = step.get("id").and_then(Value::as_str).unwrap_or("?");
        if step.get("idempotent").and_then(Value::as_bool).is_none() {
            return Err(format!("step `{step_id}` must declare idempotent explicitly"));
        }
        let executor = step.get("executor").and_then(Value::as_str).unwrap_or("");
        if !executor.starts_with("/tool/media.") {
            return Err(format!("step `{step_id}` executor `{executor}` must be a /tool/media.* link"));
        }
        let retry = step
            .pointer("/guards/retry")
            .ok_or_else(|| format!("step `{step_id}` must declare guards.retry"))?;
        if retry.get("max_attempts").and_then(Value::as_u64) != Some(1) {
            return Err(format!("step `{step_id}` must set guards.retry.max_attempts = 1"));
        }
        if retry.get("fallback").and_then(Value::as_str) != Some("abort") {
            return Err(format!("step `{step_id}` must set guards.retry.fallback = abort"));
        }
    }
    Ok(())
}

/// Compiles and stores every built-in definition. Any failure aborts service
/// startup: a system pipeline that does not compile is a release defect.
pub async fn seed_system_definitions(
    definitions: &DefinitionStore,
    registry: &FunctionRegistry,
) -> WorkflowResult<Vec<Arc<DefinitionRecord>>> {
    let mut out = Vec::new();
    for raw in SYSTEM_DEFINITIONS {
        out.push(seed_one(definitions, registry, raw).await?);
    }
    Ok(out)
}

pub async fn seed_one(
    definitions: &DefinitionStore,
    registry: &FunctionRegistry,
    raw: &str,
) -> WorkflowResult<Arc<DefinitionRecord>> {
    let value: Value = serde_json::from_str(raw)
        .map_err(|err| WorkflowError::Serialization(format!("system definition json: {err}")))?;
    let id = value
        .get("id")
        .and_then(Value::as_str)
        .unwrap_or("<missing id>")
        .to_string();
    validate_media_definition(&value).map_err(|reason| WorkflowError::InvalidSystemDefinition {
        id: id.clone(),
        reason,
    })?;
    let definition: WorkflowDefinition = serde_json::from_value(value).map_err(|err| {
        WorkflowError::InvalidSystemDefinition {
            id: id.clone(),
            reason: err.to_string(),
        }
    })?;
    if trigger_input_schema(&definition).is_none() {
        return Err(WorkflowError::InvalidSystemDefinition {
            id,
            reason: "trigger.input_schema missing".to_string(),
        });
    }
    let (report, _) = analyze_workflow(&definition);
    if report.has_errors() {
        return Err(WorkflowError::InvalidSystemDefinition {
            id,
            reason: format!(
                "analysis errors: {}",
                report
                    .errors
                    .iter()
                    .map(|e| format!("{}: {}", e.code, e.message))
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        });
    }
    let compiled = compile_workflow_with(definition.clone(), registry)?.workflow;
    for step in &definition.steps {
        if !compiled.resolved_functions.contains_key(&step.id) {
            return Err(WorkflowError::InvalidSystemDefinition {
                id,
                reason: format!("step `{}` did not bind to a FunctionObject", step.id),
            });
        }
    }
    Ok(definitions
        .upsert_with_source(
            Owner::system(),
            DefinitionSource::System,
            Some(definition.id.clone()),
            definition,
            compiled,
            report,
            vec!["system".to_string(), "media".to_string()],
        )
        .await)
}
