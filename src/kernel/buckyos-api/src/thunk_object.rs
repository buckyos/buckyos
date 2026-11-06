//! FunctionObject / ThunkObject contract.
//!
//! Object hashes only cover the fields that decide "same input, same result"
//! (doc/workflow/wokflow engine.md, notepads/llm-media-pipeline-workflow-todo.md §2.3):
//!
//! - [`FunctionObject`] (`func:<hex>`): how the result is computed and the call
//!   contract. Descriptive and scheduling fields live in [`FunctionRegistration`],
//!   which is not a named object and never enters a hash.
//! - [`ThunkObject`] (`thunk:<hex>`): `fun_id` + canonical params. The same
//!   computation has the same thunk id across runs and retries.
//! - [`ThunkExecution`]: one execution attempt of a thunk inside a run. Every
//!   receipt is correlated by `execution_id`, not by thunk id.

use std::collections::BTreeMap;

use ndn_lib::{NamedObject, NdnResult, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::validate_json_schema;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FunctionType {
    ExecPkg,
    Script(String),
    OPTask(String),
    Operator,
    /// Implemented inside a runner process (workflow `LocalThunkRunner` in P0).
    Native,
}

impl FunctionType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::ExecPkg => "exec_pkg",
            Self::Script(_) => "script",
            Self::OPTask(_) => "op_task",
            Self::Operator => "operator",
            Self::Native => "native",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FunctionParamType {
    Fixed(String),
    ObjId(String),
    CheckByRunner(String),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ResourceRequirements {
    #[serde(default)]
    pub max_tokens: Option<u64>,
    #[serde(default)]
    pub max_duration: Option<String>,
    #[serde(default)]
    pub gpu_required: bool,
    #[serde(default)]
    pub max_cost_usdb: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FunctionResultType {
    Fixed(String),
    Object(String),
    Stream(String),
    Iterator {
        element_schema: String,
        seekable: bool,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum AffinityType {
    Input,
    Result,
    Custom(Vec<String>),
}

impl Default for AffinityType {
    fn default() -> Self {
        Self::Input
    }
}

impl AffinityType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Result => "result",
            Self::Custom(_) => "custom",
        }
    }
}

/// Schema keywords that describe rather than constrain; they must not change a
/// function's identity, so they are rejected inside hashed schemas.
pub const SCHEMA_DESCRIPTIVE_KEYWORDS: [&str; 4] = ["description", "title", "examples", "default"];

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionObject {
    pub func_type: FunctionType,
    /// Native: `<registration name>/impl-<revision>`; other types: package /
    /// script content reference.
    pub content: String,
    pub is_pure: bool,
    #[serde(default)]
    pub params_type: BTreeMap<String, FunctionParamType>,
    pub params_schema: Value,
    pub result_type: FunctionResultType,
    pub result_schema: Value,
}

impl NamedObject for FunctionObject {
    fn get_obj_type() -> &'static str {
        "func"
    }
}

impl FunctionObject {
    pub fn calc_obj_id(&self) -> NdnResult<(ObjId, String)> {
        let value = serde_json::to_value(self)
            .map_err(|e| ndn_lib::NdnError::InvalidData(format!("function object: {e}")))?;
        ndn_lib::try_build_named_object_by_json(Self::get_obj_type(), &value)
    }

    /// Registration-time checks: schemas carry no descriptive keywords, every
    /// `params_type` entry names a property of `params_schema`.
    pub fn validate(&self) -> Result<(), String> {
        reject_descriptive_keywords(&self.params_schema, "params_schema")?;
        reject_descriptive_keywords(&self.result_schema, "result_schema")?;
        let properties = self
            .params_schema
            .get("properties")
            .and_then(Value::as_object);
        for name in self.params_type.keys() {
            let known = properties.map(|p| p.contains_key(name)).unwrap_or(false);
            if !known {
                return Err(format!(
                    "params_type `{name}` is not declared in params_schema.properties"
                ));
            }
        }
        Ok(())
    }

    pub fn validate_params(&self, params: &Value) -> Result<(), String> {
        validate_json_schema(&self.params_schema, params)
    }

    pub fn validate_result(&self, result: &Value) -> Result<(), String> {
        validate_json_schema(&self.result_schema, result)
    }
}

fn reject_descriptive_keywords(schema: &Value, path: &str) -> Result<(), String> {
    match schema {
        Value::Object(map) => {
            for (key, value) in map {
                if SCHEMA_DESCRIPTIVE_KEYWORDS.contains(&key.as_str()) {
                    return Err(format!("{path}: descriptive keyword `{key}` is not allowed in a hashed schema"));
                }
                reject_descriptive_keywords(value, &format!("{path}.{key}"))?;
            }
            Ok(())
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                reject_descriptive_keywords(item, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Lookup entry and admission data of a published function. Changing any field
/// here leaves `fun_id` and existing caches untouched.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FunctionRegistration {
    pub name: String,
    pub version: String,
    pub fun_id: ObjId,
    #[serde(default)]
    pub description: String,
    /// Seconds.
    #[serde(default)]
    pub timeout: Option<u64>,
    #[serde(default)]
    pub requirements: BTreeMap<String, u64>,
    #[serde(default)]
    pub best_run_weight: BTreeMap<String, u64>,
    #[serde(default)]
    pub affinity_type: AffinityType,
}

impl FunctionRegistration {
    pub fn semantic_ref(&self) -> String {
        format!("/tool/{}@{}", self.name, self.version)
    }
}

/// Parses `/tool/<name>@<version>` into `(name, version)`.
pub fn parse_tool_ref(raw: &str) -> Option<(&str, &str)> {
    let rest = raw.strip_prefix("/tool/")?;
    let (name, version) = rest.rsplit_once('@')?;
    if name.is_empty() || version.is_empty() {
        return None;
    }
    Some((name, version))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThunkObject {
    pub fun_id: ObjId,
    pub params: BTreeMap<String, Value>,
}

impl NamedObject for ThunkObject {
    fn get_obj_type() -> &'static str {
        "thunk"
    }
}

impl ThunkObject {
    pub fn new(fun_id: ObjId, params: BTreeMap<String, Value>) -> Self {
        Self { fun_id, params }
    }

    /// Canonical (JCS) id: key order of `params` never matters.
    pub fn calc_obj_id(&self) -> NdnResult<(ObjId, String)> {
        let value = serde_json::to_value(self)
            .map_err(|e| ndn_lib::NdnError::InvalidData(format!("thunk object: {e}")))?;
        ndn_lib::try_build_named_object_by_json(Self::get_obj_type(), &value)
    }
}

/// Builds `<run_id>/<node_id>/<attempt>`; for_each shards already carry the
/// index inside `node_id` (`loop[3]`).
pub fn execution_id(run_id: &str, node_id: &str, attempt: u32) -> String {
    format!("{run_id}/{node_id}/{attempt}")
}

/// Run meta of one execution attempt. Nothing here enters a content hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThunkExecution {
    pub execution_id: String,
    pub thunk_obj_id: ObjId,
    pub run_id: String,
    pub node_id: String,
    pub attempt: u32,
    #[serde(default)]
    pub execution_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_obj_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_by: Option<String>,
    /// Unix seconds; the runner must stop the work at this instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline_at: Option<i64>,
    /// Snapshot of the registration the run bound to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration: Option<FunctionRegistration>,
    /// Execution fingerprint the step cache key was computed with; the runner
    /// refuses to run under a different environment (`environment_changed`).
    #[serde(default)]
    pub fingerprint: BTreeMap<String, String>,
    /// Step cache key when the result is reusable (`idempotent && is_pure`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_key: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ThunkExecutionStatus {
    Waiting,
    Dispatched,
    Success,
    Failed,
    Cancelled,
}

/// Structured execution error carried unchanged from the function implementation
/// through the receipt, the task tree and the RPC surface.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ThunkExecutionError {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub details: Value,
    #[serde(default)]
    pub retryable: bool,
}

impl ThunkExecutionError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            details: Value::Null,
            retryable: false,
        }
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
    }

    pub fn retryable(mut self) -> Self {
        self.retryable = true;
        self
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

impl std::fmt::Display for ThunkExecutionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

pub mod error_codes {
    pub const TOOLS_MISSING: &str = "tools_missing";
    pub const TIMEOUT: &str = "timeout";
    pub const CANCELLED: &str = "cancelled";
    pub const INTERRUPTED: &str = "interrupted";
    pub const PERMISSION_DENIED: &str = "permission_denied";
    pub const RESOURCE_LIMIT: &str = "resource_limit";
    pub const ENVIRONMENT_CHANGED: &str = "environment_changed";
    pub const INVALID_PARAMS: &str = "invalid_params";
    pub const INVALID_RESULT: &str = "invalid_result";
    pub const INPUT_UNAVAILABLE: &str = "input_unavailable";
    pub const TOOL_FAILED: &str = "tool_failed";
    pub const FUNCTION_NOT_FOUND: &str = "function_not_found";
    pub const QUEUE_FULL: &str = "queue_full";
    pub const INTERNAL: &str = "internal";
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThunkExecutionResult {
    pub execution_id: String,
    pub thunk_obj_id: ObjId,
    #[serde(default)]
    pub task_id: String,
    pub status: ThunkExecutionStatus,
    #[serde(default)]
    pub result_obj_id: Option<ObjId>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub result_url: Option<String>,
    #[serde(default)]
    pub error: Option<ThunkExecutionError>,
    /// Execution facts that never enter result objects or caches.
    #[serde(default)]
    pub metrics: Value,
    /// Tool versions / implementation environment the result actually
    /// depended on; part of the step cache key.
    #[serde(default)]
    pub fingerprint: BTreeMap<String, String>,
    /// Named objects (artifact FileObjects) the result references.
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    /// `false` when the result is partial / truncated and must not be cached.
    #[serde(default = "default_true")]
    pub cacheable: bool,
}

fn default_true() -> bool {
    true
}

impl ThunkExecutionResult {
    pub fn success(execution: &ThunkExecution, result: Value) -> Self {
        Self {
            execution_id: execution.execution_id.clone(),
            thunk_obj_id: execution.thunk_obj_id.clone(),
            task_id: String::new(),
            status: ThunkExecutionStatus::Success,
            result_obj_id: None,
            result: Some(result),
            result_url: None,
            error: None,
            metrics: Value::Null,
            fingerprint: BTreeMap::new(),
            artifact_refs: Vec::new(),
            cacheable: true,
        }
    }

    pub fn failed(execution: &ThunkExecution, error: ThunkExecutionError) -> Self {
        let status = if error.code == error_codes::CANCELLED {
            ThunkExecutionStatus::Cancelled
        } else {
            ThunkExecutionStatus::Failed
        };
        Self {
            execution_id: execution.execution_id.clone(),
            thunk_obj_id: execution.thunk_obj_id.clone(),
            task_id: String::new(),
            status,
            result_obj_id: None,
            result: None,
            result_url: None,
            error: Some(error),
            metrics: Value::Null,
            fingerprint: BTreeMap::new(),
            artifact_refs: Vec::new(),
            cacheable: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_function() -> FunctionObject {
        FunctionObject {
            func_type: FunctionType::Native,
            content: "media.render_pdf_pages/impl-1".to_string(),
            is_pure: true,
            params_type: BTreeMap::from([(
                "content_id".to_string(),
                FunctionParamType::CheckByRunner("file_content".to_string()),
            )]),
            params_schema: json!({
                "type": "object",
                "required": ["content_id", "dpi"],
                "additionalProperties": false,
                "properties": {
                    "content_id": {"type": "string"},
                    "dpi": {"type": "integer"}
                }
            }),
            result_type: FunctionResultType::Object("media.page_images".to_string()),
            result_schema: json!({"type": "object", "required": ["pages"], "properties": {"pages": {"type": "array"}}}),
        }
    }

    #[test]
    fn function_id_ignores_registration_fields() {
        let function = sample_function();
        let (id1, _) = function.calc_obj_id().unwrap();
        let (id2, _) = function.calc_obj_id().unwrap();
        assert_eq!(id1, id2);
        assert_eq!(id1.obj_type, "func");
        let mut changed = function.clone();
        changed.content = "media.render_pdf_pages/impl-2".to_string();
        assert_ne!(changed.calc_obj_id().unwrap().0, id1);
    }

    #[test]
    fn descriptive_keywords_are_rejected() {
        let mut function = sample_function();
        function.params_schema["properties"]["dpi"]["description"] = json!("dots per inch");
        assert!(function.validate().is_err());
        let mut function = sample_function();
        function
            .params_type
            .insert("unknown".to_string(), FunctionParamType::Fixed("string".into()));
        assert!(function.validate().is_err());
        assert!(sample_function().validate().is_ok());
    }

    #[test]
    fn thunk_id_is_order_independent() {
        let fun_id = sample_function().calc_obj_id().unwrap().0;
        let a = ThunkObject::new(
            fun_id.clone(),
            BTreeMap::from([("dpi".to_string(), json!(144)), ("content_id".to_string(), json!("sha256:ab"))]),
        );
        let b = ThunkObject::new(
            fun_id,
            BTreeMap::from([("content_id".to_string(), json!("sha256:ab")), ("dpi".to_string(), json!(144))]),
        );
        assert_eq!(a.calc_obj_id().unwrap().0, b.calc_obj_id().unwrap().0);
        assert_eq!(a.calc_obj_id().unwrap().0.obj_type, "thunk");
    }

    #[test]
    fn tool_ref_parsing() {
        assert_eq!(parse_tool_ref("/tool/media.fit_image@1"), Some(("media.fit_image", "1")));
        assert_eq!(parse_tool_ref("/tool/media.fit_image"), None);
        assert_eq!(parse_tool_ref("/skill/x@1"), None);
        assert_eq!(execution_id("run-1", "loop[3]", 2), "run-1/loop[3]/2");
    }
}
