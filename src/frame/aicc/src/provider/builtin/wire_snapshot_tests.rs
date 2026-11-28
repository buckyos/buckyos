//! Wire snapshots of every built-in Provider.
//!
//! Each entry is keyed by `provider | model sample | API type | case` and holds
//! what AICC sends upstream (method, URL, headers, body) or what it decodes
//! from a canned response. Protocol client IDs are deliberately absent from
//! both keys and contents, so a change in how bindings are assembled must not
//! change a single entry. The committed golden file stores one digest per
//! entry; `AICC_WIRE_SNAPSHOT_DUMP=<file>` writes the full contents for
//! diffing and `AICC_WIRE_SNAPSHOT_UPDATE=1` rewrites the golden file.

use super::registry::{BuiltinProviderRegistry, CUSTOM_PROVIDER_PROFILE_ID};
use super::*;
use crate::call::{CallResolver, ProviderCallTarget};
use crate::catalog::{CatalogBuildOptions, CatalogSnapshot};
use crate::protocol::*;
use crate::provider::inventory::ProviderConnectionInput;
use crate::provider::*;
use crate::routing::policy::*;
use crate::routing::{CandidateRuntimeState, Router, RoutingRequest};
use crate::settings::{load_builtin_metadata, MetadataSources};
use base64::Engine as _;
use buckyos_api::{
    AiContent, AiMessage, AiRole, AiccCall, AiccExecutionMode, ApiType, LlmChatInvokeRequest,
    ResourceRef,
};
use bytes::Bytes;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE};
use reqwest::StatusCode;
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::Duration;

const INSTANCE: &str = "snap";
const SECRET_REF: &str = "secret://snapshot";
const SECRET: &str = "snapid.snapsecret";
const GOLDEN_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/provider/builtin/wire_snapshots.golden"
);

/// Documented bindings no model in the built-in catalog can reach: the fal
/// pattern maps API types none of its catalog models has, no Veo model is in
/// the Gemini catalog, and SN excludes `wan-3.0`.
const UNREACHABLE_BINDINGS: &[&str] = &[
    "fal|audio.asr|queue.submit",
    "fal|audio.music|queue.submit",
    "fal|audio.tts|queue.submit",
    "fal|image.img2img|queue.submit",
    "fal|image.inpaint|queue.submit",
    "fal|image.txt2img|queue.submit",
    "gemini|video.extend|models.predictLongRunning",
    "gemini|video.generate|models.predictLongRunning",
    "sn|video.edit|videos.create",
    "sn|video.extend|videos.create",
    "sn|video.generate|videos.create",
];

struct Quota;

impl QuotaSource for Quota {
    fn query(&self, _: &QuotaLookup) -> Result<QuotaSnapshot, QuotaSourceError> {
        Ok(QuotaSnapshot {
            state: Some(buckyos_api::QuotaState::Normal),
            remaining_request_units: None,
            remaining_cost: None,
            reset_at: None,
        })
    }
}

#[derive(Default)]
struct Snapshot {
    entries: BTreeMap<String, Value>,
    coverage: BTreeSet<String>,
}

impl Snapshot {
    fn insert(&mut self, key: String, value: Value) {
        assert!(
            self.entries.insert(key.clone(), value).is_none(),
            "duplicate wire snapshot key {key}"
        );
    }
}

fn catalog() -> Arc<CatalogSnapshot> {
    MetadataSources {
        builtin: load_builtin_metadata().unwrap(),
        ..Default::default()
    }
    .build_snapshot(
        crate::settings::BUILTIN_CATALOG_REVISION_SEQ,
        &CatalogBuildOptions::default(),
    )
    .unwrap()
}

fn discovered(id: &str) -> DiscoveredModel {
    DiscoveredModel {
        provider_model_id: id.into(),
        api_types: None,
        supported_features: None,
        unsupported_features: BTreeSet::new(),
        remote_methods: None,
        availability: ModelAvailability::Available,
        deprecated: false,
        pricing: None,
        video: None,
    }
}

/// Model IDs an endpoint of this Provider could list: every ID its rules
/// name, plus the origin IDs of the model driver that shares its name.
fn sample_model_ids(catalog: &CatalogSnapshot, profile_id: &str, rules_id: &str) -> Vec<String> {
    let mut ids = BTreeSet::new();
    if let Some(rules) = catalog.provider_rules(rules_id) {
        ids.extend(rules.models.iter().map(|rule| rule.id.clone()));
        ids.extend(rules.static_inventory_models.iter().cloned());
        ids.extend(rules.model_driver_overrides.keys().cloned());
    }
    let driver_id = match profile_id {
        "doubao" | "doubao-agent-plan" => "doubao",
        "bailian" => "qwen",
        other => other,
    };
    for driver in catalog.model_drivers() {
        if driver.model_driver_id == driver_id {
            ids.extend(driver.models.iter().map(|model| model.id.clone()));
        }
        if profile_id == "openrouter" {
            let vendor = match driver.model_driver_id.as_str() {
                "claude" => "anthropic",
                "gemini" => "google",
                "kimi" => "moonshotai",
                "glm" => "z-ai",
                "xai" => "x-ai",
                "mistral" => "mistralai",
                other => other,
            };
            ids.extend(
                driver
                    .models
                    .iter()
                    .map(|model| format!("{vendor}/{}", model.id)),
            );
        }
    }
    ids.into_iter().collect()
}

fn resource(kind: &str) -> Value {
    match kind {
        "image" => {
            json!({"kind": "base64", "mime": "image/png", "data_base64": base64::engine::general_purpose::STANDARD.encode(b"png-bytes")})
        }
        "mask" => {
            json!({"kind": "base64", "mime": "image/png", "data_base64": base64::engine::general_purpose::STANDARD.encode(b"mask-bytes")})
        }
        "audio" => {
            json!({"kind": "url", "url": "https://media.example.com/input.mp3", "mime_hint": "audio/mpeg"})
        }
        "video" => {
            json!({"kind": "url", "url": "https://media.example.com/input.mp4", "mime_hint": "video/mp4"})
        }
        "document" => {
            json!({"kind": "url", "url": "https://media.example.com/scan.png", "mime_hint": "image/png"})
        }
        _ => unreachable!(),
    }
}

fn base_request(api_type: ApiType, exact: &str) -> AiccCall {
    let (method, params) = match api_type {
        ApiType::Llm => {
            let mut request =
                LlmChatInvokeRequest::new(exact, vec![AiMessage::text(AiRole::User, "hello")]);
            request.max_output_tokens = Some(256);
            return AiccCall::ChatCompletionsCreate(request);
        }
        ApiType::EmbeddingText => (
            "embedding.text",
            json!({"items": [{"type": "text", "id": "a", "text": "hello"}]}),
        ),
        ApiType::EmbeddingMultimodal => (
            "embedding.multimodal",
            json!({"items": [{"id": "a", "text": "hello", "image": resource("image")}]}),
        ),
        ApiType::Decision => (
            "decision.evaluate",
            json!({"state": {"text": "hello"}, "questions": [{"type": "boolean", "id": "q1", "instructions": "Is it a greeting?"}]}),
        ),
        ApiType::Rerank => (
            "rerank",
            json!({"query": "hello", "documents": [{"id": "d1", "text": "hello world"}, {"id": "d2", "text": "bye"}], "n": 1}),
        ),
        ApiType::ImageTextToImage => ("images.generate", json!({"prompt": "A blue square"})),
        ApiType::ImageImageToImage => (
            "image.img2img",
            json!({"images": [resource("image")], "prompt": "Make it red"}),
        ),
        ApiType::ImageInpaint => (
            "image.inpaint",
            json!({"image": resource("image"), "mask": resource("mask"), "prompt": "Fill it"}),
        ),
        ApiType::ImageUpscale => (
            "image.upscale",
            json!({"image": resource("image"), "scale": 2}),
        ),
        ApiType::ImageBackgroundRemove => ("image.bg_remove", json!({"image": resource("image")})),
        ApiType::VisionOcr => ("vision.ocr", json!({"document": resource("document")})),
        ApiType::VisionCaption => ("vision.caption", json!({"image": resource("image")})),
        ApiType::VisionDetect => (
            "vision.detect",
            json!({"image": resource("image"), "classes": ["cat"]}),
        ),
        ApiType::VisionSegment => (
            "vision.segment",
            json!({"image": resource("image"), "prompt": {"type": "text", "text": "cat"}}),
        ),
        ApiType::AudioTextToSpeech => (
            "audio.tts",
            json!({"text": "hello", "voice": {"language": "en"}}),
        ),
        ApiType::AudioSpeechRecognition => ("audio.asr", json!({"audio": resource("audio")})),
        ApiType::AudioMusic => ("audio.music", json!({"prompt": "calm piano"})),
        ApiType::AudioEnhance => (
            "audio.enhance",
            json!({"audio": resource("audio"), "task": "denoise"}),
        ),
        ApiType::VideoGenerate => ("video.generate", json!({"prompt": "A paper plane"})),
        ApiType::VideoEdit => (
            "video.edit",
            json!({"video": resource("video"), "prompt": "Make it night"}),
        ),
        ApiType::VideoExtend => (
            "video.extend",
            json!({"video": resource("video"), "prompt": "Keep flying"}),
        ),
        ApiType::VideoUpscale => (
            "video.upscale",
            json!({"video": resource("video"), "target_resolution": "1080p"}),
        ),
        ApiType::AgentComputerUse => (
            "agent.computer_use",
            json!({"task": "open the menu", "environment": {"environment_id": "env-1", "session_id": "session-1", "screenshot": resource("image"), "viewport": {"width": 1280, "height": 720}}, "allowed_actions": ["left_click"]}),
        ),
    };
    let mut params = params;
    params["exact_model"] = Value::String(exact.into());
    let method = AiccCall::from_method_and_params(method, params.clone())
        .map(|call| call.method())
        .unwrap_or(method);
    AiccCall::from_method_and_params(method, params)
        .unwrap_or_else(|error| panic!("{api_type:?} sample request is invalid: {error}"))
}

fn history_request(exact: &str, coordinate: &buckyos_api::ProviderStateCoordinate) -> AiccCall {
    let mut request = LlmChatInvokeRequest::new(
        exact,
        vec![
            AiMessage::text(AiRole::User, "hello"),
            AiMessage::new(
                AiRole::Assistant,
                vec![
                    AiContent::Thinking {
                        source: coordinate.clone(),
                        summary: None,
                        text: Some("plan".into()),
                        provider_metadata: Some(json!({"signature": "sig-1"})),
                    },
                    AiContent::Thinking {
                        source: buckyos_api::ProviderStateCoordinate::unbound(),
                        summary: Some("foreign".into()),
                        text: Some("foreign".into()),
                        provider_metadata: None,
                    },
                    AiContent::Text { text: "hi".into() },
                ],
            ),
            AiMessage::text(AiRole::User, "again"),
        ],
    );
    request.max_output_tokens = Some(256);
    AiccCall::ChatCompletionsCreate(request)
}

fn redact_header(name: &str, value: &str) -> String {
    if matches!(name, "x-api-request-id" | "x-request-id")
        && value.len() == 36
        && value.bytes().filter(|byte| *byte == b'-').count() == 4
    {
        return "<uuid>".into();
    }
    if value.contains(SECRET) || value.contains("snapsecret") {
        return value.replace(SECRET, "***");
    }
    if name.eq_ignore_ascii_case("authorization") {
        let scheme = value.split_whitespace().next().unwrap_or_default();
        return format!("{scheme} <token>");
    }
    value.to_owned()
}

fn request_json(request: &HttpRequest) -> Value {
    let headers = request
        .headers
        .iter()
        .map(|(name, value)| {
            let value = value.to_str().unwrap_or("<binary>");
            (
                name.as_str().to_owned(),
                Value::String(redact_header(name.as_str(), value)),
            )
        })
        .collect::<Map<_, _>>();
    let body = match &request.body {
        HttpBody::Empty => Value::Null,
        HttpBody::Json(value) => value.clone(),
        HttpBody::Multipart(body) => Value::Array(
            body.parts()
                .iter()
                .map(|part| {
                    json!({
                        "name": part.name,
                        "file_name": part.file_name,
                        "mime": part.mime,
                        "bytes": String::from_utf8(part.bytes.to_vec())
                            .unwrap_or_else(|_| format!("<{} bytes>", part.bytes.len())),
                    })
                })
                .collect(),
        ),
    };
    json!({
        "method": request.method.as_str(),
        "url": request.url,
        "headers": headers,
        "body": body,
        "timeout": request.timeout.map(|value| value.as_secs()),
    })
}

fn materialize(call: &mut crate::call::ResolvedProviderCall, codecs: &CodecRegistry) {
    let pointers = call
        .resource_requirements
        .iter()
        .map(|requirement| requirement.request_pointer.as_str())
        .collect::<Vec<_>>();
    let form = codecs.resource_input_form(
        &call.protocol_adapter_id,
        &call.operation,
        call.api_type,
        call.execution_mode,
        &pointers,
    );
    for requirement in &call.resource_requirements {
        let key = crate::resource::ResourceKey::from_ref(&requirement.resource);
        let materialized = match &requirement.resource {
            ResourceRef::Url { url, mime_hint } if form.accepts_url() => {
                MaterializedResource::from_url(
                    url.clone(),
                    mime_hint
                        .clone()
                        .unwrap_or_else(|| "application/octet-stream".into()),
                    None,
                )
                .unwrap()
            }
            ResourceRef::Url { mime_hint, .. } => MaterializedResource::new(
                Bytes::from_static(b"downloaded-bytes"),
                mime_hint
                    .clone()
                    .unwrap_or_else(|| "application/octet-stream".into()),
                Some("input".into()),
            )
            .unwrap(),
            ResourceRef::Base64 { mime, data_base64 } => MaterializedResource::new(
                base64::engine::general_purpose::STANDARD
                    .decode(data_base64)
                    .unwrap(),
                mime.clone(),
                Some("input.png".into()),
            )
            .unwrap(),
            ResourceRef::NamedObject { .. } => unreachable!(),
        };
        call.context
            .resources
            .insert(key.as_str().to_owned(), materialized);
    }
}

/// The post-processing the execution layer applies to every decoded value.
fn finish_decoded(
    value: &mut Value,
    call: &crate::call::ResolvedProviderCall,
    codecs: &CodecRegistry,
) {
    finish_provider_state(
        value,
        StateNamespaceRename::for_call(codecs, &call.protocol_adapter_id, &call.context).as_ref(),
        &call.context.state_coordinate,
    );
}

/// The source coordinate names the protocol client; the snapshot does not.
fn without_client_ids(value: &mut Value) {
    match value {
        Value::Array(values) => values.iter_mut().for_each(without_client_ids),
        Value::Object(object) => {
            if let Some(Value::Object(source)) = object.get_mut("source") {
                if source.contains_key("adapter_type") {
                    source.insert("adapter_type".into(), Value::String("<client>".into()));
                }
            }
            object.values_mut().for_each(without_client_ids);
        }
        _ => {}
    }
}

fn output_json(output: &ProtocolOutput) -> Value {
    json!({
        "value": output.value,
        "usage": output.usage,
        "artifacts": output.artifacts.len(),
    })
}

fn http_response(body: Value) -> HttpResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    HttpResponse {
        status: StatusCode::OK,
        headers,
        body: Bytes::from(serde_json::to_vec(&body).unwrap()),
        request_id: "snapshot-request".into(),
        retry_after: None,
    }
}

fn sse_response(frames: &[String]) -> StreamingHttpResponse {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/event-stream"));
    let chunks = frames
        .iter()
        .map(|frame| Ok(Bytes::from(frame.clone())))
        .collect::<Vec<_>>();
    StreamingHttpResponse {
        status: StatusCode::OK,
        headers,
        body: Box::pin(futures_util::stream::iter(chunks)),
        request_id: "snapshot-request".into(),
        retry_after: None,
    }
}

fn data_frame(value: Value) -> String {
    format!("data: {value}\n\n")
}

fn event_frame(event: &str, value: Value) -> String {
    format!("event: {event}\ndata: {value}\n\n")
}

/// Canned upstream responses, chosen by the shape of the request URL so the
/// choice never depends on which protocol client produced the request.
fn decode_fixtures(url: &str) -> (Vec<(&'static str, Value)>, Vec<(&'static str, Vec<String>)>) {
    let path = reqwest::Url::parse(url).unwrap().path().to_owned();
    if path.ends_with("/chat/completions") {
        let message = json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "upstream-model",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "reasoning_content": "plan", "content": "hi",
                    "tool_calls": [{"id": "call-1", "type": "function", "function": {"name": "shell", "arguments": "{\"command\":\"ls\"}"}}]},
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14, "cached_tokens": 6,
                "completion_tokens_details": {"reasoning_tokens": 2}}
        });
        let mut without_object = message.clone();
        without_object.as_object_mut().unwrap().remove("object");
        let mut with_details = message.clone();
        with_details["usage"] = json!({"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14,
            "prompt_tokens_details": {"cached_tokens": 3}});
        with_details["choices"][0]["message"]
            .as_object_mut()
            .unwrap()
            .remove("reasoning_content");
        with_details["choices"][0]["message"]["reasoning"] = json!("plan-b");
        let chunk = |delta: Value, finish: Value| {
            data_frame(
                json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "model": "upstream-model",
                "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]}),
            )
        };
        let stream = vec![
            chunk(
                json!({"role": "assistant", "reasoning_content": "pl"}),
                Value::Null,
            ),
            chunk(json!({"reasoning_content": "an"}), Value::Null),
            chunk(json!({"content": "h"}), Value::Null),
            chunk(
                json!({"tool_calls": [{"index": 0, "id": "call-1", "type": "function", "function": {"name": "shell", "arguments": "{\"command\""}}]}),
                Value::Null,
            ),
            chunk(
                json!({"tool_calls": [{"index": 0, "function": {"arguments": ":\"ls\"}"}}]}),
                json!("tool_calls"),
            ),
            data_frame(
                json!({"id": "chatcmpl-1", "object": "chat.completion.chunk", "model": "upstream-model", "choices": [],
                "usage": {"prompt_tokens": 10, "completion_tokens": 4, "total_tokens": 14, "cached_tokens": 6}}),
            ),
            "data: [DONE]\n\n".to_owned(),
        ];
        return (
            vec![
                ("chat", message),
                ("chat_without_object", without_object),
                ("chat_reasoning_field", with_details),
            ],
            vec![("chat_stream", stream)],
        );
    }
    if path.ends_with("/responses") {
        let response = json!({
            "id": "resp_1", "object": "response", "status": "completed", "model": "upstream-model",
            "output": [
                {"type": "reasoning", "id": "rs_1", "summary": [{"type": "summary_text", "text": "plan"}], "encrypted_content": "enc"},
                {"type": "message", "id": "msg_1", "role": "assistant", "content": [{"type": "output_text", "text": "hi", "annotations": []}]},
                {"type": "function_call", "id": "fc_1", "call_id": "call-1", "name": "shell", "arguments": "{\"command\":\"ls\"}"}
            ],
            "usage": {"input_tokens": 10, "output_tokens": 4, "total_tokens": 14,
                "input_tokens_details": {"cached_tokens": 6}, "output_tokens_details": {"reasoning_tokens": 2}}
        });
        let stream = vec![
            event_frame(
                "response.reasoning_summary_text.delta",
                json!({"type": "response.reasoning_summary_text.delta", "item_id": "rs_1", "output_index": 0, "summary_index": 0, "delta": "plan"}),
            ),
            event_frame(
                "response.output_text.delta",
                json!({"type": "response.output_text.delta", "item_id": "msg_1", "output_index": 1, "content_index": 0, "delta": "hi"}),
            ),
            event_frame(
                "response.output_item.done",
                json!({"type": "response.output_item.done", "output_index": 2, "item": {"type": "function_call", "id": "fc_1", "call_id": "call-1", "name": "shell", "arguments": "{}"}}),
            ),
            event_frame(
                "response.completed",
                json!({"type": "response.completed", "response": response}),
            ),
        ];
        return (
            vec![("responses", response)],
            vec![("responses_stream", stream)],
        );
    }
    if path.ends_with("/messages") {
        let message = json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "upstream-model",
            "content": [
                {"type": "thinking", "thinking": "plan", "signature": "sig-1"},
                {"type": "text", "text": "hi"},
                {"type": "tool_use", "id": "toolu_1", "name": "shell", "input": {"command": "ls"}}
            ],
            "stop_reason": "tool_use", "stop_sequence": null,
            "usage": {"input_tokens": 10, "output_tokens": 4, "cache_read_input_tokens": 6, "cache_creation_input_tokens": 1}
        });
        let mut base_resp = message.clone();
        base_resp["base_resp"] = json!({"status_code": 1008, "status_msg": "insufficient balance"});
        let stream = vec![
            event_frame(
                "message_start",
                json!({"type": "message_start", "message": {"id": "msg_1", "type": "message", "role": "assistant", "model": "upstream-model", "content": [], "stop_reason": null, "usage": {"input_tokens": 10, "output_tokens": 0, "cache_read_input_tokens": 6}}}),
            ),
            event_frame(
                "content_block_start",
                json!({"type": "content_block_start", "index": 0, "content_block": {"type": "thinking", "thinking": ""}}),
            ),
            event_frame(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "thinking_delta", "thinking": "plan"}}),
            ),
            event_frame(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0, "delta": {"type": "signature_delta", "signature": "sig-1"}}),
            ),
            event_frame(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 0}),
            ),
            event_frame(
                "content_block_start",
                json!({"type": "content_block_start", "index": 1, "content_block": {"type": "text", "text": ""}}),
            ),
            event_frame(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 1, "delta": {"type": "text_delta", "text": "hi"}}),
            ),
            event_frame(
                "content_block_stop",
                json!({"type": "content_block_stop", "index": 1}),
            ),
            event_frame(
                "message_delta",
                json!({"type": "message_delta", "delta": {"stop_reason": "end_turn", "stop_sequence": null}, "usage": {"output_tokens": 4}}),
            ),
            event_frame("message_stop", json!({"type": "message_stop"})),
        ];
        return (
            vec![("messages", message), ("messages_base_resp", base_resp)],
            vec![("messages_stream", stream)],
        );
    }
    if path.contains("interactions") {
        let interaction = json!({
            "id": "interaction-1", "object": "interaction", "status": "completed", "model": "upstream-model",
            "outputs": [{"type": "thought", "summary": [{"type": "text", "text": "plan"}], "signature": "sig-1"}, {"type": "text", "text": "hi"}],
            "usage": {"total_input_tokens": 10, "total_output_tokens": 4, "total_tokens": 14, "total_cached_tokens": 6}
        });
        let stream = vec![
            event_frame(
                "content.delta",
                json!({"event_type": "content.delta", "delta": {"type": "text", "text": "hi"}}),
            ),
            event_frame(
                "interaction.completed",
                json!({"event_type": "interaction.completed", "interaction": interaction}),
            ),
        ];
        return (
            vec![("interactions", interaction)],
            vec![("interactions_stream", stream)],
        );
    }
    (Vec::new(), Vec::new())
}

struct ProfileRun<'a> {
    catalog: &'a CatalogSnapshot,
    codecs: &'a CodecRegistry,
    label: String,
    profile: ProviderProfile,
    instance: ProviderInstanceConfig,
    inventory: ProviderInventorySnapshot,
    models: crate::model::ModelRegistry,
    runtime: BTreeMap<String, CandidateRuntimeState>,
}

impl ProfileRun<'_> {
    async fn lower(
        &self,
        exact: &str,
        call: &AiccCall,
    ) -> Result<crate::call::ResolvedProviderCall, String> {
        let api_type = call.api_type().unwrap();
        let caller = CallerIdentity {
            tenant_id: "tenant".into(),
            user_id: "user".into(),
            app_id: None,
        };
        let policy =
            PolicyEngine::new(EffectiveRoutingPolicy::merge_ordered([]).unwrap(), Quota).unwrap();
        let decision = Router::new(&self.models, &policy, &self.runtime)
            .route(&RoutingRequest::new(
                "trace", "request", exact, api_type, caller,
            ))
            .map_err(|error| format!("route: {error}"))?;
        let selected = &decision.selected;
        let credential = StaticCredentialResolver::new(BTreeMap::from([(
            SECRET_REF.to_owned(),
            SECRET.to_owned(),
        )]))
        .resolve(
            &self
                .profile
                .credential_for_client(
                    &selected.protocol_adapter_id,
                    self.instance.credential_kind,
                    self.codecs,
                )
                .map_err(|error| format!("credential: {error}"))?,
            &self.instance.credential,
        )
        .await
        .map_err(|error| format!("credential: {error}"))?;
        let credential_header_name = self
            .profile
            .credential_for_client(
                &selected.protocol_adapter_id,
                self.instance.credential_kind,
                self.codecs,
            )
            .ok()
            .and_then(|credential| credential.header_name);
        let inventory_model = self.inventory.models.iter().find(|model| {
            model.provider_model_id == selected.provider_model_id
                && model.model_driver_id == selected.model_driver_id
        });
        let mut match_dimensions = BTreeMap::new();
        for (name, value) in [
            ("region", self.instance.region.as_ref()),
            ("workspace", self.instance.workspace.as_ref()),
            ("account", self.instance.account.as_ref()),
            (
                "policy_region",
                self.instance
                    .instance_rules
                    .as_ref()
                    .and_then(|rules| rules.policy_region.as_ref()),
            ),
        ] {
            if let Some(value) = value {
                match_dimensions.insert(name.to_string(), Value::String(value.clone()));
            }
        }
        let target = ProviderCallTarget {
            provider_rules_id: self.instance.provider_rules_id.clone(),
            protocol_clients: self.profile.protocol_clients(),
            state_namespace: self.profile.state_namespace.clone(),
            use_server_defaults: crate::model::ExactModelName::parse(&selected.exact_model)
                .is_ok_and(|model| model.variant().is_none())
                && inventory_model.is_some_and(|model| !model.server_default_routes.is_empty()),
            base_url: self.instance.base_url.clone(),
            operation_base_urls: self.instance.operation_base_urls.clone(),
            credential,
            credential_reference: self.instance.credential.reference.clone(),
            credential_header_name,
            limits: CodecLimits {
                request_timeout: Duration::from_secs(120),
                max_request_bytes: 32 * 1024 * 1024,
                max_response_bytes: 32 * 1024 * 1024,
            },
            pricing: None,
            match_dimensions,
            video_contract: inventory_model.and_then(|model| {
                crate::video::VideoGenerationContract::from_capabilities(&model.capabilities)
            }),
            video_facts: Default::default(),
        };
        let mut lowered = CallResolver::new(self.catalog, self.codecs)
            .lower(&decision, call, target)
            .map_err(|error| format!("lower: {error}"))?;
        materialize(&mut lowered, self.codecs);
        Ok(lowered)
    }

    fn encode(&self, call: &crate::call::ResolvedProviderCall) -> Result<HttpRequest, String> {
        match call.execution_mode {
            ExecutionMode::NativeTask => self.codecs.encode_native(
                &call.protocol_adapter_id,
                &call.operation,
                call.api_type,
                &NativeTaskInput {
                    operation: NativeTaskOperation::Submit,
                    remote_task_id: None,
                    codec_input: Some(&call.input),
                    resolved_parameters: &call.input.resolved_parameters,
                    context: &call.context,
                },
            ),
            ExecutionMode::Immediate | ExecutionMode::Stream => self.codecs.encode(
                &call.protocol_adapter_id,
                &call.operation,
                call.api_type,
                &call.input,
                &call.context,
            ),
        }
        .map_err(|error| format!("encode: {error}"))
    }

    async fn record_request(
        &self,
        snapshot: &mut Snapshot,
        key: String,
        exact: &str,
        call: &AiccCall,
    ) -> Option<crate::call::ResolvedProviderCall> {
        let lowered = match self.lower(exact, call).await {
            Ok(lowered) => lowered,
            Err(error) => {
                snapshot.insert(key, json!({"error": error}));
                return None;
            }
        };
        snapshot.coverage.insert(format!(
            "{}|{}|{}",
            self.profile.provider_profile_id,
            api_type_label(lowered.api_type),
            lowered.operation
        ));
        let entry = match self.encode(&lowered) {
            Ok(request) => {
                let mut entry = request_json(&request);
                entry["execution_mode"] = json!(format!("{:?}", lowered.execution_mode));
                entry
            }
            Err(error) => json!({"error": error}),
        };
        snapshot.insert(key, entry);
        Some(lowered)
    }

    async fn record_decodes(
        &self,
        snapshot: &mut Snapshot,
        key: &str,
        call: &crate::call::ResolvedProviderCall,
        stream_call: Option<&crate::call::ResolvedProviderCall>,
    ) {
        let Ok(request) = self.encode(call) else {
            return;
        };
        let (immediate, streams) = decode_fixtures(&request.url);
        for (name, body) in immediate {
            let entry = match self
                .codecs
                .decode_with_input(
                    &call.protocol_adapter_id,
                    &call.operation,
                    call.api_type,
                    http_response(body),
                    &call.input,
                )
                .await
            {
                Ok(ProtocolExecution::Immediate(mut output)) => {
                    finish_decoded(&mut output.value, call, self.codecs);
                    output_json(&output)
                }
                Ok(ProtocolExecution::Stream(_)) => json!({"error": "unexpected stream"}),
                Err(error) => json!({"error": error.to_string()}),
            };
            snapshot.insert(format!("{key}|decode:{name}"), entry);
        }
        let Some(stream_call) = stream_call else {
            return;
        };
        for (name, frames) in streams {
            let entry = match self
                .codecs
                .decode_stream(
                    &stream_call.protocol_adapter_id,
                    &stream_call.operation,
                    stream_call.api_type,
                    sse_response(&frames),
                )
                .await
            {
                Ok(stream) => {
                    let events = stream.events.collect::<Vec<_>>().await;
                    Value::Array(
                        events
                            .into_iter()
                            .map(|event| match event {
                                Ok(ProtocolEvent::Delta(mut value)) => {
                                    finish_decoded(&mut value, stream_call, self.codecs);
                                    json!({"delta": value})
                                }
                                Ok(ProtocolEvent::Progress(mut value)) => {
                                    finish_decoded(&mut value, stream_call, self.codecs);
                                    json!({"progress": value})
                                }
                                Ok(ProtocolEvent::Final(mut output)) => {
                                    finish_decoded(&mut output.value, stream_call, self.codecs);
                                    json!({"final": output_json(&output)})
                                }
                                Err(error) => json!({"error": error.to_string()}),
                            })
                            .collect(),
                    )
                }
                Err(error) => json!({"error": error.to_string()}),
            };
            snapshot.insert(format!("{key}|decode:{name}"), entry);
        }
    }

    async fn record(&self, snapshot: &mut Snapshot) {
        let mut decoded_operations = BTreeSet::new();
        for model in &self.inventory.models {
            if model.api_types.is_empty() {
                continue;
            }
            let exact = format!("{}@{INSTANCE}", model.provider_model_id);
            for api_type in &model.api_types {
                let key = format!(
                    "{}|{}|{}",
                    self.label,
                    model.provider_model_id,
                    api_type_label(*api_type)
                );
                let call = base_request(*api_type, &exact);
                let Some(lowered) = self
                    .record_request(snapshot, format!("{key}|request"), &exact, &call)
                    .await
                else {
                    continue;
                };
                if *api_type != ApiType::Llm {
                    continue;
                }
                let AiccCall::ChatCompletionsCreate(mut stream_request) = call.clone() else {
                    unreachable!()
                };
                stream_request.execution_mode = AiccExecutionMode::Stream;
                let stream_lowered = self
                    .record_request(
                        snapshot,
                        format!("{key}|stream"),
                        &exact,
                        &AiccCall::ChatCompletionsCreate(stream_request),
                    )
                    .await;
                self.record_request(
                    snapshot,
                    format!("{key}|history"),
                    &exact,
                    &history_request(&exact, &lowered.context.state_coordinate),
                )
                .await;
                for variant in &model.variants {
                    let variant_exact =
                        format!("{}:{}@{INSTANCE}", model.provider_model_id, variant.name);
                    self.record_request(
                        snapshot,
                        format!("{key}|variant:{}", variant.name),
                        &variant_exact,
                        &base_request(ApiType::Llm, &variant_exact),
                    )
                    .await;
                }
                if decoded_operations.insert(lowered.operation.clone()) {
                    self.record_decodes(snapshot, &key, &lowered, stream_lowered.as_ref())
                        .await;
                }
            }
        }
    }
}

fn api_type_label(api_type: ApiType) -> String {
    serde_json::to_value(api_type)
        .unwrap()
        .as_str()
        .unwrap()
        .to_owned()
}

async fn profile_run<'a>(
    catalog: &'a CatalogSnapshot,
    registry: &BuiltinProviderRegistry,
    codecs: &'a CodecRegistry,
    profile_id: &str,
    credential_kind: Option<CredentialKind>,
) -> ProfileRun<'a> {
    let default_adapter = registry
        .profiles()
        .find(|profile| profile.provider_profile_id == profile_id)
        .unwrap()
        .default_protocol_adapter_id
        .clone();
    let binding = registry
        .resolve(BuiltinProviderRequest {
            provider_profile_id: profile_id,
            protocol_adapter_id: &default_adapter,
            auth_mode: ProviderAuthMode::ApiKey,
            credential_kind,
            configured_inventory: None,
        })
        .unwrap();
    let connection = binding
        .connection
        .resolve(ProviderConnectionInput::default())
        .unwrap();
    let configuration = catalog.resolve_provider_configuration(profile_id).unwrap();
    let mut instance_rules = binding.instance_rules.clone();
    if let Some(schema) = &binding.connection.policy_region {
        let policy_region = schema.resolve("policy_region", None).unwrap();
        instance_rules
            .get_or_insert_with(Default::default)
            .policy_region = policy_region;
    }
    let instance = ProviderInstanceConfig {
        provider_instance_name: INSTANCE.into(),
        provider_profile_id: profile_id.into(),
        protocol_adapter_id: default_adapter,
        base_url: connection.base_url,
        operation_base_urls: connection.operation_base_urls,
        credential: CredentialReference {
            reference: SECRET_REF.into(),
        },
        credential_kind,
        provider_rules_id: Some(configuration.provider_rules_id.clone()),
        region: connection.region,
        workspace: connection.workspace,
        account: connection.account,
        request_timeout: Duration::from_secs(120),
        auto_sync_models: true,
        instance_rules,
    };
    let build = |discovery: ProviderDiscoverySnapshot| {
        InventoryBuilder::build_with_matcher(
            &binding.profile,
            &instance,
            discovery,
            catalog,
            codecs,
            Some(binding.discovery.as_ref()),
        )
    };
    let samples = |models: Vec<DiscoveredModel>| ProviderDiscoverySnapshot {
        revision: None,
        discovered_at_ms: 10,
        health: ProviderHealthState::Healthy,
        models,
    };
    let discovery = match &binding.profile.default_inventory {
        Some(inventory) if binding.profile.discovery_mode == DiscoveryMode::CatalogOnly => {
            inventory.clone()
        }
        _ => samples(
            sample_model_ids(catalog, profile_id, &configuration.provider_rules_id)
                .iter()
                .map(|id| discovered(id))
                .filter(|model| match build(samples(vec![model.clone()])) {
                    Ok(_) => true,
                    Err(error) => {
                        if std::env::var("AICC_WIRE_SNAPSHOT_DEBUG").is_ok() {
                            eprintln!("skip {profile_id}/{}: {error}", model.provider_model_id);
                        }
                        false
                    }
                })
                .collect(),
        ),
    };
    let inventory =
        build(discovery).unwrap_or_else(|error| panic!("{profile_id} inventory: {error}"));
    if std::env::var("AICC_WIRE_SNAPSHOT_DEBUG").is_ok() {
        for model in &inventory.unmatched_models {
            eprintln!(
                "unmatched {profile_id}/{}: {:?}",
                model.provider_model_id, model.reason
            );
        }
        for item in &inventory.filtered_capabilities {
            eprintln!(
                "filtered {profile_id}/{}: {} {:?} {}",
                item.provider_model_id, item.api_type, item.operation_id, item.reason
            );
        }
    }
    let models =
        crate::service::builtin_registry_for_test(catalog, &[inventory.as_model_inventory()]);
    let runtime = models
        .model_views()
        .into_iter()
        .map(|model| {
            (
                model.exact_model,
                CandidateRuntimeState {
                    enabled: true,
                    credential_available: true,
                    provider_privacy: ProviderPrivacy::PublicCloud,
                    trust: Some(ProviderTrustView {
                        provider_type: ProviderType::CloudApi,
                        provider_type_revision: "snapshot".into(),
                        asserted_at_ms: 1,
                        trust_level: ProviderTrustLevel::Verified,
                    }),
                    credential_scope: CredentialScope::Tenant {
                        tenant_id: "tenant".into(),
                    },
                    estimated_cost: None,
                    p95_latency_ms: None,
                    cache_hit_probability: None,
                },
            )
        })
        .collect();
    ProfileRun {
        catalog,
        codecs,
        label: match credential_kind {
            Some(kind) => format!("{profile_id}#{}", kind.as_str()),
            None => profile_id.to_owned(),
        },
        profile: binding.profile,
        instance,
        inventory,
        models,
        runtime,
    }
}

fn documented_bindings() -> BTreeSet<String> {
    let document = include_str!("../../../../../../doc/aicc/provider_operation_bindings.md");
    document
        .split("<!-- BEGIN GENERATED BINDINGS -->")
        .nth(1)
        .and_then(|section| section.split("<!-- END GENERATED BINDINGS -->").next())
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let parts = line.split('|').collect::<Vec<_>>();
            format!("{}|{}|{}", parts[0], parts[2], parts[3])
        })
        .collect()
}

fn digest(value: &Value) -> String {
    format!("{:x}", Sha256::digest(serde_json::to_vec(value).unwrap()))
}

#[tokio::test]
async fn builtin_provider_wire_snapshots_are_stable() {
    let catalog = catalog();
    let registry = builtin_provider_registry(&catalog).unwrap();
    let codecs = registry.codecs();
    let mut profiles = registry
        .profiles()
        .map(|profile| profile.provider_profile_id.clone())
        .filter(|id| id != CUSTOM_PROVIDER_PROFILE_ID)
        .collect::<Vec<_>>();
    profiles.sort();
    let mut snapshot = Snapshot::default();
    for profile_id in &profiles {
        let mut credential_kinds = vec![None];
        if profile_id == "glm" {
            credential_kinds.push(Some(CredentialKind::GlmJwt));
        }
        for credential_kind in credential_kinds {
            profile_run(&catalog, &registry, &codecs, profile_id, credential_kind)
                .await
                .record(&mut snapshot)
                .await;
        }
    }

    let missing = documented_bindings()
        .difference(&snapshot.coverage)
        .filter(|binding| !UNREACHABLE_BINDINGS.contains(&binding.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "wire snapshot samples miss documented bindings: {missing:#?}"
    );

    for value in snapshot.entries.values_mut() {
        without_client_ids(value);
    }
    if let Ok(path) = std::env::var("AICC_WIRE_SNAPSHOT_DUMP") {
        let mut dump = String::new();
        for (key, value) in &snapshot.entries {
            dump.push_str(key);
            dump.push('\n');
            dump.push_str(&serde_json::to_string_pretty(value).unwrap());
            dump.push_str("\n\n");
        }
        std::fs::write(path, dump).unwrap();
    }
    let actual = snapshot
        .entries
        .iter()
        .map(|(key, value)| format!("{key}\t{}\n", digest(value)))
        .collect::<String>();
    if std::env::var("AICC_WIRE_SNAPSHOT_UPDATE").is_ok_and(|value| value == "1") {
        std::fs::write(GOLDEN_FILE, &actual).unwrap();
        return;
    }
    let golden = std::fs::read_to_string(GOLDEN_FILE).unwrap_or_default();
    let expected = golden
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect::<BTreeMap<_, _>>();
    let actual = actual
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .collect::<BTreeMap<_, _>>();
    let changed = actual
        .iter()
        .filter(|(key, digest)| expected.get(*key) != Some(*digest))
        .map(|(key, _)| format!("changed or added: {key}"))
        .chain(
            expected
                .keys()
                .filter(|key| !actual.contains_key(*key))
                .map(|key| format!("removed: {key}")),
        )
        .collect::<Vec<_>>();
    assert!(
        changed.is_empty(),
        "{} wire snapshot entries differ (dump with AICC_WIRE_SNAPSHOT_DUMP=<file>):\n{}",
        changed.len(),
        changed
            .iter()
            .take(80)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
