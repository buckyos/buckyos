mod adapter;
mod auth;
mod chat_completions_dialects;
mod claude_messages;
#[cfg(test)]
mod contract;
mod derived_responses;
mod fal_queue;
mod gemini;
mod glm_media;
mod minimax_media;
mod minimax_messages;
mod openai_chat_completions;
mod openai_responses;
mod provider_state;
mod result;
mod sse;
mod task;
mod transport;

use minimax_media::minimax_media_registration;

pub(crate) use crate::error::{
    protocol_error_kind_from_http_status, ProtocolError, ProtocolErrorKind, ProtocolResultValue,
};
#[allow(unused_imports)]
pub(crate) use adapter::{
    normalize_provider_base_url, AdapterCredentialContract, AdapterDescriptor, AdapterStatus,
    ArtifactDownloadProtocol, ArtifactUrlReader, CodecCall, CodecContext, CodecInput, CodecLimits,
    CodecRegistration, CodecRegistry, ExecutionMode, MaterializedResource, NativeTaskCodec,
    NativeTaskInput, NativeTaskOperation, NativeTaskOutput, OperationBinding, OperationCodec,
    OperationDescriptor, ProtocolAdapterPlugin,
};
#[cfg(test)]
pub(crate) use auth::AnonymousCredentialRef;
pub(crate) use auth::{CredentialAudit, CredentialKind, ResolvedCredential};
#[cfg(test)]
pub(crate) use chat_completions_dialects::GLM_CHAT_ADAPTER_ID;
pub(crate) use chat_completions_dialects::{
    glm_chat_adapter, kimi_chat_adapter, openrouter_responses_adapter, KIMI_CHAT_ADAPTER_ID,
    OPENROUTER_RERANK_OPERATION_ID,
};
#[cfg(test)]
pub(crate) use claude_messages::CLAUDE_MESSAGES_OPERATION_ID;
pub(crate) use claude_messages::{ClaudeMessagesCodec, CLAUDE_MESSAGES_ADAPTER_ID};
#[cfg(test)]
pub(crate) use contract::{GoldenBody, ProtocolContractHarness};
#[cfg(test)]
pub(crate) use derived_responses::ResponsesDialectKind;
pub(crate) use derived_responses::{
    openai_responses_compatible_adapters, DEEPSEEK_RESPONSES_ADAPTER_ID,
    OPENROUTER_RESPONSES_ADAPTER_ID,
};
pub(crate) use fal_queue::fal_queue_adapter;
#[cfg(test)]
pub(crate) use fal_queue::{FAL_QUEUE_ADAPTER_ID, FAL_QUEUE_OPERATION_ID};
#[cfg(test)]
pub(crate) use gemini::{
    gemini_api_key, GEMINI_EMBED_CONTENT_OPERATION_ID, GEMINI_INTERACTIONS_OPERATION_ID,
    GEMINI_PREDICT_LONG_RUNNING_OPERATION_ID,
};
pub(crate) use gemini::{gemini_interactions_adapter, GEMINI_ADAPTER_ID};
pub(crate) use minimax_messages::minimax_messages_adapter;
#[cfg(test)]
pub(crate) use minimax_messages::{minimax_messages_dialect_contract, MINIMAX_MESSAGES_ADAPTER_ID};
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
    openai_responses_adapter, OPENAI_AUDIO_SPEECH_OPERATION_ID,
    OPENAI_AUDIO_TRANSCRIPTIONS_OPERATION_ID, OPENAI_EMBEDDINGS_OPERATION_ID,
    OPENAI_IMAGES_GENERATE_OPERATION_ID, OPENAI_RESPONSES_ADAPTER_ID,
    OPENAI_RESPONSES_OPERATION_ID,
};
pub(crate) use provider_state::{
    bind_provider_state_source, foreign_provider_state_text, provider_state_is_native,
};
pub(crate) use result::{
    NativeTaskHandle, NativeTaskState, ProtocolEvent, ProtocolExecution, ProtocolOutput,
    ProtocolStream,
};
#[cfg(test)]
pub(crate) use sse::SseEvent;
pub(crate) use sse::{
    sse_frame_stream, SseConfig, SseFrame, SseFrameStream, SseFramer, SseStreamEnd,
};
pub(crate) use task::{cancellation_pair, CancelHandle, Cancellation};
#[cfg(test)]
pub(crate) use task::{poll_until_terminal, PollOutcome, PollPolicy};
#[cfg(test)]
pub(crate) use transport::parse_retry_after;
pub(crate) use transport::{
    HttpBody, HttpByteStream, HttpRequest, HttpResponse, HttpTransport, HttpTransportConfig,
    MultipartBody, MultipartPart, StreamingHttpResponse,
};

include!(concat!(env!("OUT_DIR"), "/protocol_plugins.rs"));
