mod adapter;
mod auth;
mod binding;
#[cfg(test)]
mod client_catalog_tests;
#[cfg(test)]
mod claude_messages_legacy_tests;
mod chat_completions_dialects;
mod claude_messages;
#[cfg(test)]
mod contract;
mod derived_responses;
mod doubao_media;
mod doubao_speech;
mod fal_queue;
mod gemini;
mod gemini_generate_content;
mod glm_video;
mod minimax_media;
mod openai_chat_completions;
mod openai_responses;
pub(crate) mod openrouter_decisions;
mod openrouter_media;
mod openrouter_videos;
mod provider_state;
mod qwen_media;
mod result;
#[cfg(test)]
mod review_tests;
mod sse;
mod task;
mod transport;
pub(crate) mod video;
pub(crate) mod typesafe;

pub(crate) use openrouter_videos::OPENROUTER_VIDEOS_OPERATION_ID;
pub(crate) use video::finish_video_output;

pub(crate) use minimax_media::minimax_media_adapter;

pub(crate) use crate::error::{
    openai_error_kind, protocol_error_kind_from_http_status, ProtocolError, ProtocolErrorKind,
    ProtocolResultValue,
};
pub(crate) use binding::{
    resolve_binding, BindingQuery, BindingRef, BindingResolveError,
    ProtocolClientSet,
};
pub(crate) use adapter::{
    AdapterCredentialContract, AdapterDescriptor, AdapterStatus, ArtifactUrlReader, CodecCall, CodecContext, CodecInput, CodecLimits,
    CodecRegistration, CodecRegistry, ExecutionMode, MaterializedResource, NativeTaskCodec,
    NativeTaskInput, NativeTaskOperation, NativeTaskOutput, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolAdapterPlugin,
};
#[cfg(test)]
pub(crate) use auth::AnonymousCredentialRef;
pub(crate) use auth::{CredentialAudit, CredentialKind, ResolvedCredential};
pub(crate) use chat_completions_dialects::{
    legacy_openai_chat_completions_adapter, openrouter_native_adapter,
    openrouter_responses_adapter, rerank_adapter,
    OPENROUTER_RERANK_OPERATION_ID,
};
#[cfg(test)]
pub(crate) use claude_messages::CLAUDE_MESSAGES_OPERATION_ID;
pub(crate) use claude_messages::{
    claude_messages_adapter, legacy_claude_messages_adapter, LEGACY_CLAUDE_MESSAGES_ADAPTER_ID,
};
#[cfg(test)]
pub(crate) use contract::{GoldenBody, ProtocolContractHarness};
#[cfg(test)]
pub(crate) use derived_responses::ResponsesDialectKind;
pub(crate) use derived_responses::{
    openai_responses_compatible_adapters,
    OPENROUTER_RESPONSES_ADAPTER_ID,
};
pub(crate) use doubao_media::doubao_media_adapter;
pub(crate) use doubao_speech::doubao_speech_adapter;
pub(crate) use fal_queue::fal_queue_adapter;
#[cfg(test)]
pub(crate) use fal_queue::{FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID};
#[cfg(test)]
pub(crate) use gemini::{
    gemini_api_key, GEMINI_EMBED_CONTENT_OPERATION_ID, GEMINI_INTERACTIONS_OPERATION_ID,
    GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
};
pub(crate) use gemini::gemini_interactions_adapter;
pub(crate) use gemini_generate_content::gemini_generate_content_adapter;
pub(crate) use glm_video::glm_video_adapter;
pub(crate) use openai_chat_completions::{
    openai_chat_completions_adapter, openai_chat_completions_operation_descriptor,
    ChatCompletionsImmediateExtensions, ChatCompletionsStreamExtensions,
    ChatCompletionsTokenLimitParameter, OpenAiChatCompletionsCodec, OpenAiChatCompletionsDialect,
    OPENAI_CHAT_COMPLETIONS_ADAPTER_ID, OPENAI_CHAT_COMPLETIONS_OPERATION_ID,
    OPENAI_PROTOCOL_FAMILY_ID,
};
#[cfg(test)]
pub(crate) use openai_responses::OPENAI_VIDEOS_OPERATION_ID;
pub(crate) use openai_responses::{
    openai_audio_adapter, openai_embeddings_adapter, openai_images_adapter,
    openai_responses_adapter, openai_videos_adapter,
    OPENAI_AUDIO_SPEECH_OPERATION_ID,
    OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID, OPENAI_EMBEDDINGS_OPERATION_ID, OPENAI_RESPONSES_ADAPTER_ID,
    OPENAI_RESPONSES_OPERATION_ID,
};
pub(crate) use openrouter_media::{OPENROUTER_IMAGES_OPERATION_ID, OPENROUTER_MUSIC_OPERATION_ID};
pub(crate) use provider_state::{
    drop_foreign_thinking, finish_provider_state,
    foreign_provider_state_text, StateNamespaceRename,
    provider_state_is_native,
};
pub(crate) use qwen_media::qwen_media_adapter;
#[cfg(test)]
pub(crate) use self::{
    claude_messages::CLAUDE_MESSAGES_ADAPTER_ID,
    chat_completions_dialects::LEGACY_OPENAI_CHAT_COMPLETIONS_ADAPTER_ID,
    gemini::GEMINI_ADAPTER_ID,
    gemini_generate_content::GEMINI_GENERATE_CONTENT_ADAPTER_ID,
    minimax_media::MINIMAX_MEDIA_ADAPTER_ID, provider_state::bind_provider_state_source,
};
pub(crate) use result::{
    NativeTaskHandle, NativeTaskState, ProtocolEvent, ProtocolExecution, ProtocolOutput,
    ProtocolStream, ProviderArtifactRef,
};
#[cfg(test)]
pub(crate) use sse::SseEvent;
pub(crate) use sse::{
    sse_frame_stream, SseConfig, SseFrame, SseFrameStream, SseFramer, SseStreamEnd,
};
pub(crate) use task::{cancellation_pair, CancelHandle, Cancellation};
#[cfg(test)]
pub(crate) use transport::parse_retry_after;
pub(crate) use transport::{
    HttpBody, HttpByteStream, HttpRequest, HttpResponse, HttpTransport, HttpTransportConfig,
    MultipartBody, MultipartPart, StreamingHttpResponse,
};

include!(concat!(env!("OUT_DIR"), "/protocol_plugins.rs"));

/// Every built-in protocol client, as the production registry registers them.
#[cfg(test)]
pub(crate) fn builtin_codecs_for_test() -> CodecRegistry {
    let mut registry = CodecRegistry::default();
    register_builtin_adapter_plugins(&mut registry).expect("built-in adapters register");
    registry
}
