//! AICC (AI Compute Center) workflow adapter。
//!
//! 把 aicc 服务的方法接入 workflow 编排器侧的直执行通道。DSL 里写
//! `executor: "service::aicc.<method>"` 即可调用对应能力，例如：
//!
//! ```text
//! service::aicc.llm.chat
//! service::aicc.embedding.text
//! service::aicc.image.txt2img
//! service::aicc.cancel
//! ```
//!
//! ## schema 范围
//!
//! buckyos-api 里的 `AiMethodRequest` / `AiMethodResponse` 是完整协议，包含
//! 路由策略、配额、provider hint 等大量字段。workflow 作者通常不会用到
//! 这么多东西，所以这里只暴露一个 **面向 workflow 的扁平子集**：
//!
//! - 输入：把请求 envelope（`model` / `requirements` / `policy`）和
//!   `payload` 字段平铺到一层 JSON object，方便 DSL 的 `${...}` 引用。
//! - 输出：把 `AiMethodResponse` 顶层字段（`task_id` / `status` / `event_ref`）
//!   和 `result: AiResponseSummary` 里的字段平铺到一层（`text` / `tool_calls`
//!   / `artifacts` / `usage` / `cost` / `finish_reason` / `extra`）。
//!
//! 复杂的协议字段（task_options、provider 内部 extra）一期不暴露，需要的时候
//! 再扩展 schema。
//!
//! ## 不在 buckyos-api
//!
//! 这些 schema 是给 workflow 引擎和 DSL 作者用的，不是协议本身。所以放在
//! `workflow` crate 内。buckyos-api 继续维护 `AiMethodRequest` 的稳定协议
//! schema；workflow 这边只持有 workflow 视角的子集。

use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_adapter::ExecutorAdapter;
use crate::types::ExecutorRef;
use async_trait::async_trait;
use buckyos_api::{
    ai_methods, AiCost, AiMessage, AiMethodRequest, AiMethodResponse, AiMethodStatus, AiPayload,
    AiResponseSummary, AiToolCall, AiToolSpec, AiUsage, AiccClient, Capability, ModelSpec,
    RespFormat, Requirements, ResourceRef, RoutePolicy,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 编排器在 `executor` 字段里识别 aicc 服务的命名空间前缀。
pub const AICC_EXECUTOR_PREFIX: &str = "service::aicc.";

/// 一条 aicc 方法的 workflow 视角 schema。
#[derive(Debug, Clone)]
pub struct AiccMethodSchema {
    /// `service::aicc.<method>` 中的 `<method>` 部分。
    pub method: &'static str,
    /// 能力分类，决定底层 `AiMethodRequest.capability`；控制方法返回 `None`。
    pub capability: Option<Capability>,
    /// workflow 视角推荐的默认 model alias（仅 AI 方法使用）。可被 input.model 覆盖。
    pub default_alias: Option<&'static str>,
    /// workflow 视角的输入 JSON Schema（draft-07 子集）。
    pub input_schema: Value,
    /// workflow 视角的输出 JSON Schema。
    pub output_schema: Value,
    /// 是否默认幂等。供 DSL 作者参考；最终是否启用结果缓存仍由 Step `idempotent` 字段决定。
    pub idempotent: bool,
    /// 单行说明，给 registry 列表展示用。
    pub description: &'static str,
}

/// 返回 workflow 一期支持的全部 aicc 方法 schema。
pub fn aicc_method_schemas() -> Vec<AiccMethodSchema> {
    use ai_methods::*;

    let mut out = Vec::new();

    // ---- LLM ----
    out.push(llm_schema(
        LLM_CHAT,
        "llm.chat",
        "Chat completion with optional tool calls.",
        true,
    ));
    out.push(llm_schema(
        LLM_COMPLETION,
        "llm.completion",
        "Plain text completion / instruction following.",
        true,
    ));

    // ---- Embedding ----
    out.push(embedding_schema(EMBEDDING_TEXT, "embedding.text", false));
    out.push(embedding_schema(
        EMBEDDING_MULTIMODAL,
        "embedding.multimodal",
        true,
    ));

    // ---- Rerank ----
    out.push(AiccMethodSchema {
        method: RERANK,
        capability: Some(Capability::Rerank),
        default_alias: Some("rerank"),
        input_schema: json!({
            "type": "object",
            "description": "Rerank a list of documents against a query.",
            "properties": {
                "input_json": {
                    "type": "object",
                    "description": "Provider-shaped payload, typically `{ \"query\": str, \"documents\": [str] }`.",
                    "properties": {
                        "query": { "type": "string" },
                        "documents": { "type": "array", "items": { "type": "string" } },
                        "top_k": { "type": "integer" }
                    },
                    "required": ["query", "documents"]
                },
                "model": { "type": "string" },
                "model_hint": { "type": "string" },
                "options": { "type": "object" },
                "policy": route_policy_schema(),
                "idempotency_key": { "type": "string" },
                "must_features": features_schema(),
                "max_latency_ms": { "type": "integer", "minimum": 0 },
                "max_cost_usd": { "type": "number", "minimum": 0.0 }
            },
            "required": ["input_json"]
        }),
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description: "Rerank candidate documents against a query.",
    });

    // ---- Image generation ----
    out.push(image_schema(
        IMAGE_TXT2IMG,
        "image.txt2img",
        "Generate an image from a text prompt.",
        false,
        false,
    ));
    out.push(image_schema(
        IMAGE_IMG2IMG,
        "image.img2img",
        "Transform an input image guided by a prompt.",
        true,
        false,
    ));
    out.push(image_schema(
        IMAGE_INPAINT,
        "image.inpaint",
        "Inpaint masked regions of an image.",
        true,
        true,
    ));
    out.push(image_schema(
        IMAGE_UPSCALE,
        "image.upscale",
        "Upscale an image.",
        true,
        false,
    ));
    out.push(image_schema(
        IMAGE_BG_REMOVE,
        "image.bg_remove",
        "Remove the background of an image.",
        true,
        false,
    ));

    // ---- Vision ----
    out.push(vision_schema(
        VISION_OCR,
        "vision.ocr",
        "Run OCR on an image; returns recognized text.",
    ));
    out.push(vision_schema(
        VISION_CAPTION,
        "vision.caption",
        "Generate a caption describing an image.",
    ));
    out.push(vision_schema(
        VISION_DETECT,
        "vision.detect",
        "Detect objects in an image; results in `extra`.",
    ));
    out.push(vision_schema(
        VISION_SEGMENT,
        "vision.segment",
        "Run image segmentation; masks in `artifacts`.",
    ));

    // ---- Audio ----
    out.push(AiccMethodSchema {
        method: AUDIO_TTS,
        capability: Some(Capability::Audio),
        default_alias: Some("audio.tts"),
        input_schema: tts_input_schema(),
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description: "Text-to-speech; audio in `artifacts`.",
    });
    out.push(AiccMethodSchema {
        method: AUDIO_ASR,
        capability: Some(Capability::Audio),
        default_alias: Some("audio.asr"),
        input_schema: single_resource_schema(
            "audio",
            "Speech audio to transcribe (one of `kind: url|base64|named_object`).",
        ),
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description: "Automatic speech recognition; transcript in `text`.",
    });
    out.push(AiccMethodSchema {
        method: AUDIO_MUSIC,
        capability: Some(Capability::Audio),
        default_alias: Some("audio.music"),
        input_schema: prompt_input_schema("Music generation prompt."),
        output_schema: ai_response_output_schema(),
        idempotent: false,
        description: "Generate music from a prompt.",
    });
    out.push(AiccMethodSchema {
        method: AUDIO_ENHANCE,
        capability: Some(Capability::Audio),
        default_alias: Some("audio.enhance"),
        input_schema: single_resource_schema("audio", "Audio resource to enhance."),
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description: "Audio enhancement (denoise / restore).",
    });

    // ---- Video ----
    out.push(video_schema(
        VIDEO_TXT2VIDEO,
        "video.txt2video",
        "Text-to-video.",
        false,
    ));
    out.push(video_schema(
        VIDEO_IMG2VIDEO,
        "video.img2video",
        "Image-to-video.",
        true,
    ));
    out.push(video_schema(
        VIDEO_VIDEO2VIDEO,
        "video.video2video",
        "Video-to-video transformation.",
        true,
    ));
    out.push(video_schema(
        VIDEO_EXTEND,
        "video.extend",
        "Extend an input video.",
        true,
    ));
    out.push(video_schema(
        VIDEO_UPSCALE,
        "video.upscale",
        "Upscale a video.",
        true,
    ));

    // ---- Agent ----
    out.push(AiccMethodSchema {
        method: AGENT_COMPUTER_USE,
        capability: Some(Capability::Agent),
        default_alias: Some("agent.computer_use"),
        input_schema: json!({
            "type": "object",
            "description": "Computer-use agent step. Provide goal in `text` and current screenshot in `resources`.",
            "properties": {
                "text": { "type": "string", "description": "Task / goal description." },
                "messages": messages_schema(),
                "tool_specs": tool_specs_schema(),
                "resources": resources_schema(),
                "options": { "type": "object" },
                "input_json": { "type": "object" },
                "model": { "type": "string" },
                "model_hint": { "type": "string" },
                "policy": route_policy_schema(),
                "idempotency_key": { "type": "string" },
                "must_features": features_schema(),
                "max_latency_ms": { "type": "integer", "minimum": 0 },
                "max_cost_usd": { "type": "number", "minimum": 0.0 }
            }
        }),
        output_schema: ai_response_output_schema(),
        idempotent: false,
        description: "Computer-use agent step; tool_calls in output.",
    });

    // ---- Control ----
    out.push(AiccMethodSchema {
        method: ai_methods::CANCEL,
        capability: None,
        default_alias: None,
        input_schema: json!({
            "type": "object",
            "description": "Cancel a running aicc task.",
            "properties": {
                "task_id": { "type": "string" }
            },
            "required": ["task_id"]
        }),
        output_schema: json!({
            "type": "object",
            "properties": {
                "task_id": { "type": "string" },
                "accepted": { "type": "boolean" }
            },
            "required": ["task_id", "accepted"]
        }),
        idempotent: true,
        description: "Cancel a previously issued aicc task.",
    });

    out
}

/// 在 schema 表中查找指定方法的定义。
pub fn aicc_method_schema(method: &str) -> Option<AiccMethodSchema> {
    aicc_method_schemas()
        .into_iter()
        .find(|schema| schema.method == method)
}

// ---------- adapter ----------

/// 调用 aicc 服务的编排器侧 adapter。匹配所有 `service::aicc.<method>` executor。
pub struct AiccAdapter {
    client: Arc<AiccClient>,
}

impl AiccAdapter {
    pub fn new(client: Arc<AiccClient>) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ExecutorAdapter for AiccAdapter {
    fn supports(&self, executor: &ExecutorRef) -> bool {
        match executor {
            ExecutorRef::Actual(value) => method_from_executor(value).is_some(),
            ExecutorRef::SemanticPath(_) => false,
        }
    }

    async fn invoke(&self, executor: &ExecutorRef, input: &Value) -> WorkflowResult<Value> {
        let executor_str = executor.as_str();
        let method = method_from_executor(executor_str).ok_or_else(|| {
            WorkflowError::Dispatcher(format!("aicc adapter cannot handle `{}`", executor_str))
        })?;

        if method == ai_methods::CANCEL {
            return self.invoke_cancel(input).await;
        }

        let schema = aicc_method_schema(method).ok_or_else(|| {
            WorkflowError::Dispatcher(format!(
                "aicc method `{}` is not registered in workflow schema table",
                method
            ))
        })?;
        let request = build_ai_request(&schema, input)?;
        let response = self
            .client
            .call_method(method, request)
            .await
            .map_err(|err| WorkflowError::Dispatcher(format!("aicc {} failed: {}", method, err)))?;
        if response.status == AiMethodStatus::Failed {
            // 把 provider 侧失败抛回 orchestrator，让 retry/human-fallback 生效；
            // 否则会把没有 `text` 的伪成功结果写进 node_outputs 并污染缓存，
            // 下游 `${node.output.text}` 引用马上就会爆 ReferenceResolution。
            let event_ref = response.event_ref.as_deref().unwrap_or("");
            return Err(WorkflowError::Dispatcher(format!(
                "aicc {} returned failed status: task_id={}, event_ref={}",
                method, response.task_id, event_ref,
            )));
        }
        Ok(flatten_ai_response(response))
    }
}

impl AiccAdapter {
    async fn invoke_cancel(&self, input: &Value) -> WorkflowResult<Value> {
        let task_id = input
            .get("task_id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                WorkflowError::Dispatcher(
                    "aicc cancel: missing required field `task_id`".to_string(),
                )
            })?;
        let resp = self
            .client
            .cancel(task_id)
            .await
            .map_err(|err| WorkflowError::Dispatcher(format!("aicc cancel failed: {}", err)))?;
        Ok(json!({
            "task_id": resp.task_id,
            "accepted": resp.accepted,
        }))
    }
}

fn method_from_executor(value: &str) -> Option<&str> {
    let method = value.strip_prefix(AICC_EXECUTOR_PREFIX)?;
    if method == ai_methods::CANCEL || ai_methods::is_ai_method(method) {
        Some(method)
    } else {
        None
    }
}

fn build_ai_request(schema: &AiccMethodSchema, input: &Value) -> WorkflowResult<AiMethodRequest> {
    let capability = schema.capability.clone().ok_or_else(|| {
        WorkflowError::Dispatcher(format!(
            "aicc method `{}` has no capability mapping",
            schema.method
        ))
    })?;

    let alias = input
        .get("model")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| schema.default_alias.map(str::to_owned))
        .ok_or_else(|| {
            WorkflowError::Dispatcher(format!(
                "aicc {}: missing `model` and no default alias configured",
                schema.method
            ))
        })?;
    let model_hint = input
        .get("model_hint")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let must_features = input
        .get("must_features")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let max_latency_ms = input.get("max_latency_ms").and_then(Value::as_u64);
    let max_cost_usd = input.get("max_cost_usd").and_then(Value::as_f64);
    let resp_format = match input.get("resp_format").and_then(Value::as_str) {
        Some(value) if value.eq_ignore_ascii_case("json") => RespFormat::Json,
        _ => RespFormat::Text,
    };
    let extra = input.get("extra").cloned();

    let requirements = Requirements {
        must_features,
        max_latency_ms,
        max_cost_usd,
        resp_format,
        extra,
    };

    let policy = match input.get("policy") {
        Some(value) if !value.is_null() => Some(serde_json::from_value::<RoutePolicy>(
            value.clone(),
        )
        .map_err(|err| {
            WorkflowError::Dispatcher(format!(
                "aicc {}: invalid `policy`: {}",
                schema.method, err
            ))
        })?),
        _ => None,
    };

    let idempotency_key = input
        .get("idempotency_key")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let payload = build_ai_payload(schema, input)?;

    Ok(AiMethodRequest {
        capability,
        model: ModelSpec::new(alias, model_hint),
        requirements,
        payload,
        policy,
        idempotency_key,
        task_options: None,
    })
}

fn build_ai_payload(schema: &AiccMethodSchema, input: &Value) -> WorkflowResult<AiPayload> {
    let text = input
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let messages: Vec<AiMessage> = match input.get("messages") {
        Some(value) if !value.is_null() => serde_json::from_value(value.clone()).map_err(|err| {
            WorkflowError::Dispatcher(format!(
                "aicc {}: invalid `messages`: {}",
                schema.method, err
            ))
        })?,
        _ => Vec::new(),
    };
    let tool_specs: Vec<AiToolSpec> = match input.get("tool_specs") {
        Some(value) if !value.is_null() => serde_json::from_value(value.clone()).map_err(|err| {
            WorkflowError::Dispatcher(format!(
                "aicc {}: invalid `tool_specs`: {}",
                schema.method, err
            ))
        })?,
        _ => Vec::new(),
    };
    let resources: Vec<ResourceRef> = match input.get("resources") {
        Some(value) if !value.is_null() => serde_json::from_value(value.clone()).map_err(|err| {
            WorkflowError::Dispatcher(format!(
                "aicc {}: invalid `resources`: {}",
                schema.method, err
            ))
        })?,
        _ => Vec::new(),
    };
    let options = input.get("options").cloned();
    let input_json = input.get("input_json").cloned();

    Ok(AiPayload::new(
        text,
        messages,
        tool_specs,
        resources,
        input_json,
        options,
    ))
}

fn flatten_ai_response(response: AiMethodResponse) -> Value {
    let AiMethodResponse {
        task_id,
        status,
        result,
        event_ref,
    } = response;

    let mut out = serde_json::Map::new();
    out.insert("task_id".to_string(), Value::String(task_id));
    out.insert("status".to_string(), ai_status_to_value(status));
    if let Some(event_ref) = event_ref {
        out.insert("event_ref".to_string(), Value::String(event_ref));
    }

    if let Some(summary) = result {
        merge_summary_fields(&mut out, summary);
    }

    Value::Object(out)
}

fn merge_summary_fields(out: &mut serde_json::Map<String, Value>, summary: AiResponseSummary) {
    let AiResponseSummary {
        text,
        tool_calls,
        artifacts,
        usage,
        cost,
        finish_reason,
        provider_task_ref,
        extra,
    } = summary;

    if let Some(text) = text {
        out.insert("text".to_string(), Value::String(text));
    }
    if !tool_calls.is_empty() {
        out.insert(
            "tool_calls".to_string(),
            tool_calls_to_value(tool_calls),
        );
    }
    if !artifacts.is_empty() {
        out.insert(
            "artifacts".to_string(),
            serde_json::to_value(&artifacts).unwrap_or(Value::Null),
        );
    }
    if let Some(usage) = usage {
        out.insert("usage".to_string(), usage_to_value(usage));
    }
    if let Some(cost) = cost {
        out.insert("cost".to_string(), cost_to_value(cost));
    }
    if let Some(finish_reason) = finish_reason {
        out.insert("finish_reason".to_string(), Value::String(finish_reason));
    }
    if let Some(provider_task_ref) = provider_task_ref {
        out.insert(
            "provider_task_ref".to_string(),
            Value::String(provider_task_ref),
        );
    }
    if let Some(extra) = extra {
        out.insert("extra".to_string(), extra);
    }
}

fn ai_status_to_value(status: AiMethodStatus) -> Value {
    match status {
        AiMethodStatus::Succeeded => Value::String("succeeded".into()),
        AiMethodStatus::Running => Value::String("running".into()),
        AiMethodStatus::Failed => Value::String("failed".into()),
    }
}

fn tool_calls_to_value(calls: Vec<AiToolCall>) -> Value {
    Value::Array(
        calls
            .into_iter()
            .map(|call| {
                json!({
                    "name": call.name,
                    "args": call.args,
                    "call_id": call.call_id,
                })
            })
            .collect(),
    )
}

fn usage_to_value(usage: AiUsage) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(v) = usage.input_tokens {
        map.insert("input_tokens".into(), json!(v));
    }
    if let Some(v) = usage.output_tokens {
        map.insert("output_tokens".into(), json!(v));
    }
    if let Some(v) = usage.total_tokens {
        map.insert("total_tokens".into(), json!(v));
    }
    Value::Object(map)
}

fn cost_to_value(cost: AiCost) -> Value {
    json!({
        "amount": cost.amount,
        "currency": cost.currency,
    })
}

// ---------- schema helpers ----------

fn ai_response_output_schema() -> Value {
    json!({
        "type": "object",
        "description": "Flattened AiMethodResponse + AiResponseSummary.",
        "properties": {
            "task_id": { "type": "string" },
            "status": { "type": "string", "enum": ["succeeded", "running", "failed"] },
            "event_ref": { "type": "string" },
            "text": { "type": "string" },
            "tool_calls": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "args": { "type": "object" },
                        "call_id": { "type": "string" }
                    },
                    "required": ["name", "args", "call_id"]
                }
            },
            "artifacts": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "name": { "type": "string" },
                        "resource": resource_ref_schema(),
                        "mime": { "type": "string" },
                        "metadata": { "type": "object" }
                    },
                    "required": ["name", "resource"]
                }
            },
            "usage": {
                "type": "object",
                "properties": {
                    "input_tokens": { "type": "integer" },
                    "output_tokens": { "type": "integer" },
                    "total_tokens": { "type": "integer" }
                }
            },
            "cost": {
                "type": "object",
                "properties": {
                    "amount": { "type": "number" },
                    "currency": { "type": "string" }
                },
                "required": ["amount", "currency"]
            },
            "finish_reason": { "type": "string" },
            "provider_task_ref": { "type": "string" },
            "extra": {}
        },
        "required": ["task_id", "status"]
    })
}

fn resource_ref_schema() -> Value {
    json!({
        "type": "object",
        "description": "Reference to a media/data resource. `kind` selects the variant.",
        "oneOf": [
            {
                "properties": {
                    "kind": { "const": "url" },
                    "url": { "type": "string" },
                    "mime_hint": { "type": "string" }
                },
                "required": ["kind", "url"]
            },
            {
                "properties": {
                    "kind": { "const": "base64" },
                    "mime": { "type": "string" },
                    "data_base64": { "type": "string" }
                },
                "required": ["kind", "mime", "data_base64"]
            },
            {
                "properties": {
                    "kind": { "const": "named_object" },
                    "obj_id": { "type": "string" }
                },
                "required": ["kind", "obj_id"]
            }
        ]
    })
}

fn resources_schema() -> Value {
    json!({
        "type": "array",
        "description": "Attached resources (images, audio, named objects).",
        "items": resource_ref_schema()
    })
}

fn messages_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "role": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["role", "content"]
        }
    })
}

fn tool_specs_schema() -> Value {
    json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "name": { "type": "string" },
                "description": { "type": "string" },
                "args_schema": { "type": "object" },
                "output_schema": {}
            },
            "required": ["name", "description", "args_schema", "output_schema"]
        }
    })
}

fn route_policy_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "profile": {
                "type": "string",
                "enum": ["cheap", "fast", "balanced", "quality"]
            },
            "allow_fallback": { "type": "boolean" },
            "runtime_failover": { "type": "boolean" },
            "explain": { "type": "boolean" }
        }
    })
}

fn features_schema() -> Value {
    json!({ "type": "array", "items": { "type": "string" } })
}

fn llm_schema(
    method: &'static str,
    default_alias: &'static str,
    description: &'static str,
    needs_messages_or_text: bool,
) -> AiccMethodSchema {
    let mut schema = json!({
        "type": "object",
        "description": description,
        "properties": {
            "text": { "type": "string", "description": "Single-turn prompt; alternative to `messages`." },
            "messages": messages_schema(),
            "tool_specs": tool_specs_schema(),
            "resources": resources_schema(),
            "options": {
                "type": "object",
                "description": "Provider runtime options (temperature, max_tokens, etc.)."
            },
            "input_json": { "type": "object", "description": "Raw escape hatch when text/messages are insufficient." },
            "model": { "type": "string" },
            "model_hint": { "type": "string" },
            "policy": route_policy_schema(),
            "idempotency_key": { "type": "string" },
            "must_features": features_schema(),
            "max_latency_ms": { "type": "integer", "minimum": 0 },
            "max_cost_usd": { "type": "number", "minimum": 0.0 },
            "resp_format": { "type": "string", "enum": ["text", "json"] }
        }
    });
    if needs_messages_or_text {
        if let Some(map) = schema.as_object_mut() {
            map.insert(
                "anyOf".to_string(),
                json!([
                    { "required": ["messages"] },
                    { "required": ["text"] },
                    { "required": ["input_json"] }
                ]),
            );
        }
    }
    AiccMethodSchema {
        method,
        capability: Some(Capability::Llm),
        default_alias: Some(default_alias),
        input_schema: schema,
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description,
    }
}

fn embedding_schema(
    method: &'static str,
    default_alias: &'static str,
    require_resources: bool,
) -> AiccMethodSchema {
    let description = if require_resources {
        "Multimodal embedding; provide `text` and/or `resources`."
    } else {
        "Text embedding. Provide `text` (single) or `input_json.texts` (batch)."
    };
    let mut schema = json!({
        "type": "object",
        "description": description,
        "properties": {
            "text": { "type": "string" },
            "input_json": {
                "type": "object",
                "properties": {
                    "texts": { "type": "array", "items": { "type": "string" } }
                }
            },
            "resources": resources_schema(),
            "options": { "type": "object" },
            "model": { "type": "string" },
            "model_hint": { "type": "string" },
            "policy": route_policy_schema(),
            "idempotency_key": { "type": "string" },
            "must_features": features_schema(),
            "max_latency_ms": { "type": "integer", "minimum": 0 },
            "max_cost_usd": { "type": "number", "minimum": 0.0 }
        }
    });
    if let Some(map) = schema.as_object_mut() {
        let mut clauses: Vec<Value> = vec![
            json!({ "required": ["text"] }),
            json!({ "required": ["input_json"] }),
        ];
        if require_resources {
            clauses.push(json!({ "required": ["resources"] }));
        }
        map.insert("anyOf".to_string(), Value::Array(clauses));
    }
    AiccMethodSchema {
        method,
        capability: Some(Capability::Embedding),
        default_alias: Some(default_alias),
        input_schema: schema,
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description,
    }
}

fn image_schema(
    method: &'static str,
    default_alias: &'static str,
    description: &'static str,
    require_image: bool,
    require_mask: bool,
) -> AiccMethodSchema {
    let mut props = serde_json::Map::new();
    props.insert(
        "text".to_string(),
        json!({ "type": "string", "description": "Prompt." }),
    );
    props.insert(
        "input_json".to_string(),
        json!({
            "type": "object",
            "description": "Optional per-provider knobs (negative_prompt, width, height, num_images, etc.)."
        }),
    );
    props.insert("resources".to_string(), resources_schema());
    props.insert("options".to_string(), json!({ "type": "object" }));
    props.insert("model".to_string(), json!({ "type": "string" }));
    props.insert("model_hint".to_string(), json!({ "type": "string" }));
    props.insert("policy".to_string(), route_policy_schema());
    props.insert("idempotency_key".to_string(), json!({ "type": "string" }));
    props.insert("must_features".to_string(), features_schema());
    props.insert(
        "max_latency_ms".to_string(),
        json!({ "type": "integer", "minimum": 0 }),
    );
    props.insert(
        "max_cost_usd".to_string(),
        json!({ "type": "number", "minimum": 0.0 }),
    );

    let mut schema = json!({
        "type": "object",
        "description": description,
        "properties": Value::Object(props),
    });
    if require_image || require_mask {
        let mut required = Vec::new();
        if require_image || require_mask {
            required.push(json!("resources"));
        }
        if let Some(map) = schema.as_object_mut() {
            map.insert("required".to_string(), Value::Array(required));
        }
    }
    AiccMethodSchema {
        method,
        capability: Some(Capability::Image),
        default_alias: Some(default_alias),
        input_schema: schema,
        output_schema: ai_response_output_schema(),
        idempotent: false,
        description,
    }
}

fn vision_schema(
    method: &'static str,
    default_alias: &'static str,
    description: &'static str,
) -> AiccMethodSchema {
    AiccMethodSchema {
        method,
        capability: Some(Capability::Vision),
        default_alias: Some(default_alias),
        input_schema: single_resource_schema("image", "Image to analyze."),
        output_schema: ai_response_output_schema(),
        idempotent: true,
        description,
    }
}

fn video_schema(
    method: &'static str,
    default_alias: &'static str,
    description: &'static str,
    require_video: bool,
) -> AiccMethodSchema {
    let mut schema = json!({
        "type": "object",
        "description": description,
        "properties": {
            "text": { "type": "string", "description": "Prompt." },
            "resources": resources_schema(),
            "input_json": { "type": "object" },
            "options": { "type": "object" },
            "model": { "type": "string" },
            "model_hint": { "type": "string" },
            "policy": route_policy_schema(),
            "idempotency_key": { "type": "string" },
            "must_features": features_schema(),
            "max_latency_ms": { "type": "integer", "minimum": 0 },
            "max_cost_usd": { "type": "number", "minimum": 0.0 }
        }
    });
    if require_video {
        if let Some(map) = schema.as_object_mut() {
            map.insert("required".to_string(), json!(["resources"]));
        }
    }
    AiccMethodSchema {
        method,
        capability: Some(Capability::Video),
        default_alias: Some(default_alias),
        input_schema: schema,
        output_schema: ai_response_output_schema(),
        idempotent: false,
        description,
    }
}

fn single_resource_schema(label: &str, description: &str) -> Value {
    json!({
        "type": "object",
        "description": format!(
            "Provide the {label} resource as the first item in `resources`. {description}"
        ),
        "properties": {
            "resources": resources_schema(),
            "options": { "type": "object" },
            "input_json": { "type": "object" },
            "model": { "type": "string" },
            "model_hint": { "type": "string" },
            "policy": route_policy_schema(),
            "idempotency_key": { "type": "string" },
            "must_features": features_schema(),
            "max_latency_ms": { "type": "integer", "minimum": 0 },
            "max_cost_usd": { "type": "number", "minimum": 0.0 }
        },
        "required": ["resources"]
    })
}

fn prompt_input_schema(description: &str) -> Value {
    json!({
        "type": "object",
        "description": description,
        "properties": {
            "text": { "type": "string" },
            "options": { "type": "object" },
            "input_json": { "type": "object" },
            "model": { "type": "string" },
            "model_hint": { "type": "string" },
            "policy": route_policy_schema(),
            "idempotency_key": { "type": "string" },
            "must_features": features_schema(),
            "max_latency_ms": { "type": "integer", "minimum": 0 },
            "max_cost_usd": { "type": "number", "minimum": 0.0 }
        },
        "required": ["text"]
    })
}

fn tts_input_schema() -> Value {
    json!({
        "type": "object",
        "description": "Text-to-speech. `text` is the spoken content.",
        "properties": {
            "text": { "type": "string" },
            "options": {
                "type": "object",
                "description": "Provider knobs (voice, speed, language, format)."
            },
            "input_json": { "type": "object" },
            "model": { "type": "string" },
            "model_hint": { "type": "string" },
            "policy": route_policy_schema(),
            "idempotency_key": { "type": "string" },
            "must_features": features_schema(),
            "max_latency_ms": { "type": "integer", "minimum": 0 },
            "max_cost_usd": { "type": "number", "minimum": 0.0 }
        },
        "required": ["text"]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::AiccHandler;
    use kRPC::{RPCContext, RPCErrors};
    use std::sync::Mutex;

    #[test]
    fn schema_table_covers_all_known_methods() {
        let schemas = aicc_method_schemas();
        let methods: Vec<&str> = schemas.iter().map(|s| s.method).collect();

        for expected in [
            ai_methods::LLM_CHAT,
            ai_methods::LLM_COMPLETION,
            ai_methods::EMBEDDING_TEXT,
            ai_methods::EMBEDDING_MULTIMODAL,
            ai_methods::RERANK,
            ai_methods::IMAGE_TXT2IMG,
            ai_methods::IMAGE_IMG2IMG,
            ai_methods::IMAGE_INPAINT,
            ai_methods::IMAGE_UPSCALE,
            ai_methods::IMAGE_BG_REMOVE,
            ai_methods::VISION_OCR,
            ai_methods::VISION_CAPTION,
            ai_methods::VISION_DETECT,
            ai_methods::VISION_SEGMENT,
            ai_methods::AUDIO_TTS,
            ai_methods::AUDIO_ASR,
            ai_methods::AUDIO_MUSIC,
            ai_methods::AUDIO_ENHANCE,
            ai_methods::VIDEO_TXT2VIDEO,
            ai_methods::VIDEO_IMG2VIDEO,
            ai_methods::VIDEO_VIDEO2VIDEO,
            ai_methods::VIDEO_EXTEND,
            ai_methods::VIDEO_UPSCALE,
            ai_methods::AGENT_COMPUTER_USE,
            ai_methods::CANCEL,
        ] {
            assert!(
                methods.contains(&expected),
                "schema table missing aicc method `{}`",
                expected
            );
        }
    }

    #[test]
    fn supports_only_aicc_executor() {
        let adapter = AiccAdapter::new(Arc::new(AiccClient::new_in_process(Box::new(
            EchoHandler::default(),
        ))));
        assert!(adapter.supports(&ExecutorRef::parse("service::aicc.llm.chat").unwrap()));
        assert!(adapter.supports(&ExecutorRef::parse("service::aicc.cancel").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("service::aicc.unknown").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("service::msg_center.notify").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("/agent/mia").unwrap()));
    }

    #[derive(Default)]
    struct EchoHandler {
        last_method: Mutex<Option<String>>,
        last_request: Mutex<Option<AiMethodRequest>>,
    }

    #[async_trait]
    impl AiccHandler for EchoHandler {
        async fn handle_method(
            &self,
            method: &str,
            request: AiMethodRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<AiMethodResponse, RPCErrors> {
            *self.last_method.lock().unwrap() = Some(method.to_string());
            *self.last_request.lock().unwrap() = Some(request);
            Ok(AiMethodResponse::new(
                "task-1".to_string(),
                AiMethodStatus::Succeeded,
                Some(AiResponseSummary {
                    text: Some("hello".to_string()),
                    tool_calls: vec![],
                    artifacts: vec![],
                    usage: Some(AiUsage {
                        input_tokens: Some(3),
                        output_tokens: Some(5),
                        total_tokens: Some(8),
                    }),
                    cost: Some(AiCost {
                        amount: 0.0001,
                        currency: "USD".to_string(),
                    }),
                    finish_reason: Some("stop".to_string()),
                    provider_task_ref: None,
                    extra: None,
                }),
                Some("task://task-1/events".to_string()),
            ))
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<buckyos_api::CancelResponse, RPCErrors> {
            Ok(buckyos_api::CancelResponse::new(task_id.to_string(), true))
        }
    }

    #[tokio::test]
    async fn llm_chat_round_trip() {
        let handler = Arc::new(EchoHandler::default());
        let client = Arc::new(AiccClient::new_in_process(
            Box::new(SharedHandler(handler.clone())),
        ));
        let adapter = AiccAdapter::new(client);
        let executor = ExecutorRef::parse("service::aicc.llm.chat").unwrap();
        let input = json!({
            "messages": [{"role": "user", "content": "hi"}],
            "options": {"temperature": 0.2},
            "model": "llm.chat.test",
            "must_features": ["plan"],
            "max_cost_usd": 0.01
        });
        let output = adapter.invoke(&executor, &input).await.unwrap();
        assert_eq!(output["task_id"], "task-1");
        assert_eq!(output["status"], "succeeded");
        assert_eq!(output["text"], "hello");
        assert_eq!(output["usage"]["total_tokens"], 8);
        assert_eq!(output["cost"]["currency"], "USD");
        assert_eq!(output["event_ref"], "task://task-1/events");

        let last_method = handler.last_method.lock().unwrap().clone();
        assert_eq!(last_method.as_deref(), Some(ai_methods::LLM_CHAT));
        let request = handler.last_request.lock().unwrap().clone().unwrap();
        assert_eq!(request.model.alias, "llm.chat.test");
        assert_eq!(request.requirements.must_features, vec!["plan".to_string()]);
        assert_eq!(request.requirements.max_cost_usd, Some(0.01));
        assert_eq!(request.payload.messages.len(), 1);
        assert_eq!(request.payload.messages[0].content, "hi");
    }

    #[tokio::test]
    async fn cancel_round_trip() {
        let handler = Arc::new(EchoHandler::default());
        let client = Arc::new(AiccClient::new_in_process(Box::new(SharedHandler(
            handler.clone(),
        ))));
        let adapter = AiccAdapter::new(client);
        let executor = ExecutorRef::parse("service::aicc.cancel").unwrap();
        let output = adapter
            .invoke(&executor, &json!({ "task_id": "task-1" }))
            .await
            .unwrap();
        assert_eq!(output["task_id"], "task-1");
        assert_eq!(output["accepted"], true);
    }

    #[tokio::test]
    async fn missing_required_field_errors() {
        let handler = Arc::new(EchoHandler::default());
        let client = Arc::new(AiccClient::new_in_process(Box::new(SharedHandler(
            handler.clone(),
        ))));
        let adapter = AiccAdapter::new(client);
        let executor = ExecutorRef::parse("service::aicc.cancel").unwrap();
        let err = adapter.invoke(&executor, &json!({})).await.unwrap_err();
        assert!(matches!(err, WorkflowError::Dispatcher(_)));
    }

    #[tokio::test]
    async fn failed_status_surfaces_as_error() {
        let client = Arc::new(AiccClient::new_in_process(Box::new(FailedHandler)));
        let adapter = AiccAdapter::new(client);
        let executor = ExecutorRef::parse("service::aicc.llm.chat").unwrap();
        let err = adapter
            .invoke(
                &executor,
                &json!({
                    "messages": [{"role": "user", "content": "hi"}],
                    "model": "llm.chat.test"
                }),
            )
            .await
            .unwrap_err();
        match err {
            WorkflowError::Dispatcher(msg) => {
                assert!(msg.contains("returned failed status"));
                assert!(msg.contains("task-bad"));
            }
            other => panic!("expected Dispatcher err, got {:?}", other),
        }
    }

    struct FailedHandler;

    #[async_trait]
    impl AiccHandler for FailedHandler {
        async fn handle_method(
            &self,
            _method: &str,
            _request: AiMethodRequest,
            _ctx: RPCContext,
        ) -> std::result::Result<AiMethodResponse, RPCErrors> {
            Ok(AiMethodResponse::new(
                "task-bad".to_string(),
                AiMethodStatus::Failed,
                None,
                Some("task://task-bad/events".to_string()),
            ))
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> std::result::Result<buckyos_api::CancelResponse, RPCErrors> {
            Ok(buckyos_api::CancelResponse::new(task_id.to_string(), true))
        }
    }

    struct SharedHandler(Arc<EchoHandler>);

    #[async_trait]
    impl AiccHandler for SharedHandler {
        async fn handle_method(
            &self,
            method: &str,
            request: AiMethodRequest,
            ctx: RPCContext,
        ) -> std::result::Result<AiMethodResponse, RPCErrors> {
            self.0.handle_method(method, request, ctx).await
        }

        async fn handle_cancel(
            &self,
            task_id: &str,
            ctx: RPCContext,
        ) -> std::result::Result<buckyos_api::CancelResponse, RPCErrors> {
            self.0.handle_cancel(task_id, ctx).await
        }
    }
}
