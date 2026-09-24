//! AICC (AI Compute Center) workflow adapter。
//!
//! 把 aicc 服务的方法接入 workflow 编排器侧的直执行通道。DSL 里写
//! `executor: "service::aicc.<method>"` 即可调用对应能力，例如：
//!
//! ```text
//! service::aicc.helper.llm_chat
//! service::aicc.embedding.text
//! service::aicc.helper.text_to_image
//! service::aicc.cancel
//! ```
//!
//! ## schema 范围
//!
//! Workflow 输入直接使用 buckyos-api 定义的 canonical typed request；输出保留
//! 对应 typed response，并为 chat message 补充便于 DSL 引用的 `text` 和
//! `tool_calls` 派生字段。
//!
//! ## 不在 buckyos-api
//!
//! 这些 schema 是给 workflow 引擎和 DSL 作者用的，不是协议本身。所以放在
//! `workflow` crate 内。buckyos-api 是 typed request/response 的唯一 owner。

use crate::error::{WorkflowError, WorkflowResult};
use crate::executor_adapter::ExecutorAdapter;
use crate::types::ExecutorRef;
use async_trait::async_trait;
use buckyos_api::{
    ai_methods, get_buckyos_api_runtime, AiccCall, AiccClient, CancelRequest, Capability,
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
    /// 粗粒度能力分类；控制方法返回 `None`。
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

type AiccMethodDefinition = (
    &'static str,
    Option<Capability>,
    Option<&'static str>,
    bool,
    &'static str,
);

/// 返回 workflow 一期支持的全部 aicc 方法 schema。
pub fn aicc_method_schemas() -> Vec<AiccMethodSchema> {
    use ai_methods::*;

    let methods: &[AiccMethodDefinition] = &[
        (
            ROUTE_RESOLVE,
            None,
            None,
            true,
            "Resolve a logical model to one exact model.",
        ),
        (
            CHAT_COMPLETIONS_CREATE,
            Some(Capability::Llm),
            None,
            false,
            "Run typed chat inference.",
        ),
        (
            IMAGES_GENERATE,
            Some(Capability::Image),
            None,
            false,
            "Generate images from text.",
        ),
        (
            HELPER_LLM_CHAT,
            Some(Capability::Llm),
            Some("llm.chat"),
            false,
            "Resolve and run chat inference.",
        ),
        (
            HELPER_TEXT_TO_IMAGE,
            Some(Capability::Image),
            Some("image.txt2img"),
            false,
            "Resolve and generate images.",
        ),
        (
            EMBEDDING_TEXT,
            Some(Capability::Embedding),
            None,
            true,
            "Embed text or document resources.",
        ),
        (
            EMBEDDING_MULTIMODAL,
            Some(Capability::Embedding),
            None,
            true,
            "Embed multimodal items.",
        ),
        (
            RERANK,
            Some(Capability::Rerank),
            None,
            true,
            "Rerank documents.",
        ),
        (
            IMAGE_IMG2IMG,
            Some(Capability::Image),
            None,
            false,
            "Transform images.",
        ),
        (
            IMAGE_INPAINT,
            Some(Capability::Image),
            None,
            false,
            "Inpaint an image.",
        ),
        (
            IMAGE_UPSCALE,
            Some(Capability::Image),
            None,
            true,
            "Upscale an image.",
        ),
        (
            IMAGE_BG_REMOVE,
            Some(Capability::Image),
            None,
            true,
            "Remove an image background.",
        ),
        (VISION_OCR, Some(Capability::Vision), None, true, "Run OCR."),
        (
            VISION_CAPTION,
            Some(Capability::Vision),
            None,
            true,
            "Caption an image.",
        ),
        (
            VISION_DETECT,
            Some(Capability::Vision),
            None,
            true,
            "Detect image objects.",
        ),
        (
            VISION_SEGMENT,
            Some(Capability::Vision),
            None,
            true,
            "Segment an image.",
        ),
        (
            AUDIO_TTS,
            Some(Capability::Audio),
            None,
            true,
            "Synthesize speech.",
        ),
        (
            AUDIO_ASR,
            Some(Capability::Audio),
            None,
            true,
            "Transcribe speech.",
        ),
        (
            AUDIO_MUSIC,
            Some(Capability::Audio),
            None,
            false,
            "Generate music.",
        ),
        (
            AUDIO_ENHANCE,
            Some(Capability::Audio),
            None,
            true,
            "Enhance audio.",
        ),
        (
            VIDEO_TXT2VIDEO,
            Some(Capability::Video),
            None,
            false,
            "Generate video from text.",
        ),
        (
            VIDEO_IMG2VIDEO,
            Some(Capability::Video),
            None,
            false,
            "Generate video from an image.",
        ),
        (
            VIDEO_VIDEO2VIDEO,
            Some(Capability::Video),
            None,
            false,
            "Transform video.",
        ),
        (
            VIDEO_EXTEND,
            Some(Capability::Video),
            None,
            false,
            "Extend video.",
        ),
        (
            VIDEO_UPSCALE,
            Some(Capability::Video),
            None,
            true,
            "Upscale video.",
        ),
        (
            AGENT_COMPUTER_USE,
            Some(Capability::Agent),
            None,
            false,
            "Run a computer-use step.",
        ),
        (CANCEL, None, None, true, "Cancel an AICC task."),
    ];

    methods
        .iter()
        .map(
            |(method, capability, default_alias, idempotent, description)| AiccMethodSchema {
                method,
                capability: capability.clone(),
                default_alias: *default_alias,
                input_schema: canonical_input_schema(method),
                output_schema: canonical_output_schema(method),
                idempotent: *idempotent,
                description,
            },
        )
        .collect()
}

fn canonical_output_schema(method: &str) -> Value {
    if method == ai_methods::CANCEL {
        return json!({
            "type": "object",
            "properties": {
                "task_id": {"type": "string"},
                "accepted": {"type": "boolean"}
            },
            "required": ["task_id", "accepted"]
        });
    }
    if method == ai_methods::ROUTE_RESOLVE {
        return json!({
            "type": "object",
            "properties": {
                "selected_exact_model": {"type": "string"},
                "selected_model_uid": {"type": "string"},
                "provider_instance_name": {"type": "string"},
                "route_trace": {"type": "object"}
            },
            "required": [
                "selected_exact_model",
                "selected_model_uid",
                "provider_instance_name",
                "route_trace"
            ]
        });
    }
    json!({
        "type": "object",
        "properties": {
            "task_id": {"type": "string"},
            "status": {"type": "string", "enum": ["succeeded", "running", "failed"]},
            "event_ref": {"type": "string"},
            "error": {"type": "object"}
        },
        "required": ["task_id", "status"]
    })
}

fn canonical_input_schema(method: &str) -> Value {
    use ai_methods::*;

    let required: &[&str] = match method {
        ROUTE_RESOLVE => &["api_type", "logical_model"],
        CANCEL => &["task_id"],
        CHAT_COMPLETIONS_CREATE => &["exact_model", "messages"],
        IMAGES_GENERATE => &["exact_model", "prompt"],
        HELPER_LLM_CHAT => &["logical_model", "messages"],
        HELPER_TEXT_TO_IMAGE => &["logical_model", "prompt"],
        EMBEDDING_TEXT | EMBEDDING_MULTIMODAL => &["exact_model", "items"],
        RERANK => &["exact_model", "query", "documents"],
        IMAGE_IMG2IMG => &["exact_model", "images", "prompt"],
        IMAGE_INPAINT => &["exact_model", "image", "mask", "prompt"],
        IMAGE_UPSCALE | IMAGE_BG_REMOVE => &["exact_model", "image"],
        VISION_OCR => &["exact_model", "document"],
        VISION_CAPTION | VISION_DETECT => &["exact_model", "image"],
        VISION_SEGMENT => &["exact_model", "image", "prompt"],
        AUDIO_TTS => &["exact_model", "text", "voice"],
        AUDIO_ASR => &["exact_model", "audio"],
        AUDIO_MUSIC => &["exact_model", "prompt"],
        AUDIO_ENHANCE => &["exact_model", "audio", "task"],
        VIDEO_TXT2VIDEO => &["exact_model", "prompt"],
        VIDEO_IMG2VIDEO => &["exact_model", "image", "prompt"],
        VIDEO_VIDEO2VIDEO | VIDEO_EXTEND => &["exact_model", "video", "prompt"],
        VIDEO_UPSCALE => &["exact_model", "video", "target_resolution"],
        AGENT_COMPUTER_USE => &["exact_model", "task", "environment", "allowed_actions"],
        _ => &[],
    };
    let mut properties = serde_json::Map::new();
    for field in required {
        properties.insert((*field).to_string(), json!({}));
    }
    if properties.contains_key("exact_model") {
        properties.insert(
            "exact_model".to_string(),
            json!({"type": "string", "pattern": "^[^@]+@[^@]+$"}),
        );
    }
    if properties.contains_key("logical_model") {
        properties.insert(
            "logical_model".to_string(),
            json!({"type": "string", "pattern": "^[^@]+$"}),
        );
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "description": "Canonical AICC typed request; buckyos-api performs strict serde validation."
    })
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
    client: Option<Arc<AiccClient>>,
}

impl AiccAdapter {
    pub fn new(client: Arc<AiccClient>) -> Self {
        Self {
            client: Some(client),
        }
    }

    pub fn from_runtime() -> Self {
        Self { client: None }
    }

    async fn client(&self) -> WorkflowResult<Arc<AiccClient>> {
        if let Some(client) = self.client.as_ref() {
            return Ok(client.clone());
        }
        let runtime =
            get_buckyos_api_runtime().map_err(|err| WorkflowError::Dispatcher(err.to_string()))?;
        Box::pin(runtime.get_aicc_client())
            .await
            .map(Arc::new)
            .map_err(|err| WorkflowError::Dispatcher(err.to_string()))
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

        let call = AiccCall::from_method_and_params(method, input.clone())
            .map_err(|err| WorkflowError::Dispatcher(err.to_string()))?;
        let response = self
            .client()
            .await?
            .invoke(call)
            .await
            .map_err(|err| WorkflowError::Dispatcher(format!("aicc {method} failed: {err}")))?;
        if response.get("status").and_then(Value::as_str) == Some("failed") {
            // 把 provider 侧失败抛回 orchestrator，让 retry/human-fallback 生效；
            // 否则会把没有 `text` 的伪成功结果写进 node_outputs 并污染缓存，
            // 下游 `${node.output.text}` 引用马上就会爆 ReferenceResolution。
            let task_id = response
                .get("task_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let event_ref = response
                .get("event_ref")
                .and_then(Value::as_str)
                .unwrap_or("");
            return Err(WorkflowError::Dispatcher(format!(
                "aicc {} returned failed status: task_id={}, event_ref={}",
                method, task_id, event_ref,
            )));
        }
        Ok(flatten_typed_response(response))
    }
}

impl AiccAdapter {
    async fn invoke_cancel(&self, input: &Value) -> WorkflowResult<Value> {
        let request = CancelRequest::from_json(input.clone())
            .map_err(|err| WorkflowError::Dispatcher(err.to_string()))?;
        let resp = self
            .client()
            .await?
            .cancel(&request.task_id)
            .await
            .map_err(|err| WorkflowError::Dispatcher(format!("aicc cancel failed: {}", err)))?;
        serde_json::to_value(resp).map_err(|err| WorkflowError::Dispatcher(err.to_string()))
    }
}

fn method_from_executor(value: &str) -> Option<&str> {
    let method = value.strip_prefix(AICC_EXECUTOR_PREFIX)?;
    if aicc_method_schema(method).is_some() {
        Some(method)
    } else {
        None
    }
}

fn flatten_typed_response(mut response: Value) -> Value {
    let Some(out) = response.as_object_mut() else {
        return response;
    };
    if let Some(message) = out.get("message").cloned() {
        if let Ok(message) = serde_json::from_value::<buckyos_api::AiMessage>(message) {
            let text = message.text_content();
            if !text.is_empty() {
                out.insert("text".to_string(), Value::String(text));
            }
            let tool_calls = message.tool_calls();
            if !tool_calls.is_empty() {
                out.insert(
                    "tool_calls".to_string(),
                    serde_json::to_value(tool_calls).unwrap_or(Value::Null),
                );
            }
        }
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::{
        AiContent, AiMessage, AiMethodStatus, AiRole, AiccHandler, CancelResponse,
        LlmChatHelperRequest, LlmChatInvokeRequest, LlmChatInvokeResponse,
        TextToImageHelperRequest, TextToImageInvokeResponse,
    };
    use kRPC::{RPCContext, RPCErrors};
    use std::sync::Mutex;

    #[derive(Debug, Clone, PartialEq)]
    enum SeenCall {
        TypedChat(LlmChatInvokeRequest),
        HelperChat(LlmChatHelperRequest),
        HelperImage(TextToImageHelperRequest),
        Cancel(String),
    }

    #[derive(Default)]
    struct RecordingHandler {
        calls: Arc<Mutex<Vec<SeenCall>>>,
    }

    #[async_trait]
    impl AiccHandler for RecordingHandler {
        async fn handle_cancel(
            &self,
            task_id: &str,
            _ctx: RPCContext,
        ) -> Result<CancelResponse, RPCErrors> {
            self.calls
                .lock()
                .unwrap()
                .push(SeenCall::Cancel(task_id.to_string()));
            Ok(CancelResponse::new(task_id.to_string(), true))
        }

        async fn handle_chat_completions_create(
            &self,
            request: LlmChatInvokeRequest,
            _ctx: RPCContext,
        ) -> Result<LlmChatInvokeResponse, RPCErrors> {
            self.calls
                .lock()
                .unwrap()
                .push(SeenCall::TypedChat(request));
            let mut response = LlmChatInvokeResponse {
                task_id: "chat-task".to_string(),
                status: AiMethodStatus::Succeeded,
                message: Some(AiMessage::new(
                    AiRole::Assistant,
                    vec![
                        AiContent::text("hello"),
                        AiContent::tool_use("call-1", "lookup", Default::default()),
                    ],
                )),
                tool_calls: Vec::new(),
                usage: None,
                cost: None,
                finish_reason: Some("tool_use".to_string()),
                provider_task_ref: None,
                route_trace: None,
                event_ref: Some("task://chat-task/events".to_string()),
                error: None,
            };
            response.tool_calls = response.message.as_ref().unwrap().tool_calls();
            Ok(response)
        }

        async fn handle_helper_llm_chat(
            &self,
            request: LlmChatHelperRequest,
            _ctx: RPCContext,
        ) -> Result<LlmChatInvokeResponse, RPCErrors> {
            self.calls
                .lock()
                .unwrap()
                .push(SeenCall::HelperChat(request));
            Ok(LlmChatInvokeResponse {
                task_id: "helper-task".to_string(),
                status: AiMethodStatus::Succeeded,
                message: Some(AiMessage::text(AiRole::Assistant, "helper result")),
                tool_calls: Vec::new(),
                usage: None,
                cost: None,
                finish_reason: Some("stop".to_string()),
                provider_task_ref: None,
                route_trace: None,
                event_ref: Some("task://helper-task/events".to_string()),
                error: None,
            })
        }

        async fn handle_helper_text_to_image(
            &self,
            request: TextToImageHelperRequest,
            _ctx: RPCContext,
        ) -> Result<TextToImageInvokeResponse, RPCErrors> {
            self.calls
                .lock()
                .unwrap()
                .push(SeenCall::HelperImage(request));
            Ok(TextToImageInvokeResponse {
                task_id: "image-task".to_string(),
                status: AiMethodStatus::Running,
                images: Vec::new(),
                provider_states: Vec::new(),
                usage: None,
                cost: None,
                finish_reason: None,
                provider_task_ref: Some("provider-job-1".to_string()),
                route_trace: None,
                event_ref: Some("task://image-task/events".to_string()),
                error: None,
            })
        }
    }

    fn adapter() -> (AiccAdapter, Arc<Mutex<Vec<SeenCall>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let handler = RecordingHandler {
            calls: calls.clone(),
        };
        (
            AiccAdapter::new(Arc::new(AiccClient::new_in_process(Box::new(handler)))),
            calls,
        )
    }

    #[test]
    fn exposes_only_canonical_core_and_cancel_methods() {
        let schemas = aicc_method_schemas();
        assert_eq!(schemas.len(), 27);
        assert!(schemas
            .iter()
            .all(|schema| ai_methods::is_aicc_core_method(schema.method)
                || schema.method == ai_methods::CANCEL));
        assert!(aicc_method_schema(ai_methods::CHAT_COMPLETIONS_CREATE).is_some());
        assert!(aicc_method_schema(ai_methods::IMAGES_GENERATE).is_some());
        assert!(aicc_method_schema("llm.chat").is_none());
        assert!(aicc_method_schema("image.txt2image").is_none());
        assert!(aicc_method_schema(ai_methods::SERVICE_RELOAD_SETTINGS).is_none());
    }

    #[test]
    fn supports_only_registered_aicc_executors() {
        let (adapter, _) = adapter();
        assert!(
            adapter.supports(&ExecutorRef::parse("service::aicc.chat.completions.create").unwrap())
        );
        assert!(adapter.supports(&ExecutorRef::parse("service::aicc.cancel").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("service::aicc.llm.chat").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("service::aicc.provider.list").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("service::msg-center.send").unwrap()));
        assert!(!adapter.supports(&ExecutorRef::parse("/agent/jarvis").unwrap()));
    }

    #[tokio::test]
    async fn typed_chat_uses_public_request_and_response_contracts() {
        let (adapter, calls) = adapter();
        let output = adapter
            .invoke(
                &ExecutorRef::parse("service::aicc.chat.completions.create").unwrap(),
                &json!({
                    "exact_model": "gpt-5@openai-main",
                    "messages": [{
                        "role": "user",
                        "content": [{"type": "text", "text": "hi"}]
                    }],
                    "temperature": 0.2
                }),
            )
            .await
            .unwrap();

        assert_eq!(output["task_id"], "chat-task");
        assert_eq!(output["status"], "succeeded");
        assert_eq!(output["text"], "hello");
        assert_eq!(output["tool_calls"][0]["name"], "lookup");
        assert_eq!(output["event_ref"], "task://chat-task/events");
        let calls = calls.lock().unwrap();
        let SeenCall::TypedChat(request) = &calls[0] else {
            panic!("expected typed chat call");
        };
        assert_eq!(request.exact_model, "gpt-5@openai-main");
        assert_eq!(request.messages[0].text_content(), "hi");
        assert_eq!(request.temperature, Some(0.2));
    }

    #[tokio::test]
    async fn helpers_keep_logical_models_and_running_task_envelopes() {
        let (adapter, calls) = adapter();
        let chat = adapter
            .invoke(
                &ExecutorRef::parse("service::aicc.helper.llm_chat").unwrap(),
                &json!({
                    "logical_model": "llm.chat",
                    "messages": [{
                        "role": "user",
                        "content": [{"type": "text", "text": "plan"}]
                    }]
                }),
            )
            .await
            .unwrap();
        let image = adapter
            .invoke(
                &ExecutorRef::parse("service::aicc.helper.text_to_image").unwrap(),
                &json!({"logical_model": "image.txt2img", "prompt": "a lighthouse"}),
            )
            .await
            .unwrap();

        assert_eq!(chat["text"], "helper result");
        assert_eq!(image["task_id"], "image-task");
        assert_eq!(image["status"], "running");
        assert_eq!(image["event_ref"], "task://image-task/events");
        let calls = calls.lock().unwrap();
        assert!(matches!(
            &calls[0],
            SeenCall::HelperChat(request) if request.logical_model == "llm.chat"
        ));
        assert!(matches!(
            &calls[1],
            SeenCall::HelperImage(request) if request.logical_model == "image.txt2img"
        ));
    }

    #[tokio::test]
    async fn cancel_uses_strict_public_request_and_response_contracts() {
        let (adapter, calls) = adapter();
        let executor = ExecutorRef::parse("service::aicc.cancel").unwrap();
        let output = adapter
            .invoke(&executor, &json!({"task_id": "image-task"}))
            .await
            .unwrap();
        assert_eq!(output, json!({"task_id": "image-task", "accepted": true}));
        assert_eq!(
            calls.lock().unwrap().as_slice(),
            &[SeenCall::Cancel("image-task".to_string())]
        );

        let error = adapter
            .invoke(
                &executor,
                &json!({"task_id": "image-task", "legacy_force": true}),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, WorkflowError::Dispatcher(_)));
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn typed_request_rejects_all_in_one_payload_fields() {
        let (adapter, calls) = adapter();
        let error = adapter
            .invoke(
                &ExecutorRef::parse("service::aicc.chat.completions.create").unwrap(),
                &json!({
                    "exact_model": "gpt-5@openai-main",
                    "messages": [],
                    "payload": {"text": "legacy"}
                }),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, WorkflowError::Dispatcher(_)));
        assert!(calls.lock().unwrap().is_empty());
    }
}
