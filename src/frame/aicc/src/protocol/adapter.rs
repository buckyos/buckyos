use super::{
    HttpRequest, HttpResponse, NativeTaskHandle, NativeTaskState, ProtocolError, ProtocolExecution,
    ProtocolOutput, ProtocolResultValue, ProtocolStream, ResolvedCredential, StreamingHttpResponse,
};
use async_trait::async_trait;
use buckyos_api::{AiccCall, ApiType, Capability};
use bytes::Bytes;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum ExecutionMode {
    Immediate,
    Stream,
    NativeTask,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AdapterStatus {
    Stable,
    Preview,
    Deprecated,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationBinding {
    pub api_type: ApiType,
    pub capability: Capability,
    pub supported_features: BTreeSet<String>,
    pub execution_modes: BTreeSet<ExecutionMode>,
}

impl OperationBinding {
    pub(crate) fn new(
        api_type: ApiType,
        execution_modes: impl IntoIterator<Item = ExecutionMode>,
    ) -> Self {
        Self {
            api_type,
            capability: api_type.capability(),
            supported_features: BTreeSet::new(),
            execution_modes: execution_modes.into_iter().collect(),
        }
    }

    fn validate(&self) -> ProtocolResultValue<()> {
        if self.capability != self.api_type.capability() {
            return Err(ProtocolError::invalid_configuration(
                "operation binding capability does not match its API type",
            ));
        }
        if self.execution_modes.is_empty() {
            return Err(ProtocolError::invalid_configuration(
                "operation binding must support at least one execution mode",
            ));
        }
        if self
            .supported_features
            .iter()
            .any(|feature| feature.trim().is_empty())
        {
            return Err(ProtocolError::invalid_configuration(
                "operation feature names must not be empty",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OperationDescriptor {
    pub operation_id: String,
    pub bindings: Vec<OperationBinding>,
    pub supports_cancel: bool,
    pub supports_webhook: bool,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

impl OperationDescriptor {
    pub(crate) fn validate(&self) -> ProtocolResultValue<()> {
        validate_operation_id(&self.operation_id)?;
        if self.bindings.is_empty() {
            return Err(ProtocolError::invalid_configuration(
                "operation must declare at least one API type binding",
            ));
        }
        let mut api_types = BTreeSet::new();
        for binding in &self.bindings {
            binding.validate()?;
            if !api_types.insert(binding.api_type.typed_method()) {
                return Err(ProtocolError::invalid_configuration(
                    "operation contains a duplicate API type binding",
                ));
            }
        }
        if self.max_request_bytes == 0 || self.max_response_bytes == 0 {
            return Err(ProtocolError::invalid_configuration(
                "operation body limits must be greater than zero",
            ));
        }
        let has_native_task = self
            .bindings
            .iter()
            .any(|binding| binding.execution_modes.contains(&ExecutionMode::NativeTask));
        if (self.supports_cancel || self.supports_webhook) && !has_native_task {
            return Err(ProtocolError::invalid_configuration(
                "cancel and webhook support require native task execution",
            ));
        }
        Ok(())
    }

    pub(crate) fn binding(&self, api_type: ApiType) -> ProtocolResultValue<&OperationBinding> {
        self.bindings
            .iter()
            .find(|binding| binding.api_type == api_type)
            .ok_or_else(|| {
                ProtocolError::new(
                    super::ProtocolErrorKind::UnsupportedOperation,
                    "operation does not support the selected API type",
                )
            })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AdapterDescriptor {
    pub protocol_family_id: String,
    pub protocol_adapter_id: String,
    pub interface_generation: String,
    pub base_adapter_id: Option<String>,
    pub status: AdapterStatus,
    pub operations: BTreeMap<String, OperationDescriptor>,
}

impl AdapterDescriptor {
    pub(crate) fn validate(&self) -> ProtocolResultValue<()> {
        validate_id("protocol family", &self.protocol_family_id)?;
        validate_id("protocol adapter", &self.protocol_adapter_id)?;
        validate_id("interface generation", &self.interface_generation)?;
        if let Some(base_adapter_id) = &self.base_adapter_id {
            validate_id("base adapter", base_adapter_id)?;
            if base_adapter_id == &self.protocol_adapter_id {
                return Err(ProtocolError::invalid_configuration(
                    "adapter cannot use itself as its base",
                ));
            }
        }
        if self.operations.is_empty() {
            return Err(ProtocolError::invalid_configuration(
                "adapter must declare at least one operation",
            ));
        }
        for (operation_id, descriptor) in &self.operations {
            descriptor.validate()?;
            if operation_id != &descriptor.operation_id {
                return Err(ProtocolError::invalid_configuration(
                    "adapter operation key does not match descriptor ID",
                ));
            }
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq)]
pub(crate) struct CodecInput {
    pub canonical_request: AiccCall,
    pub resolved_parameters: BTreeMap<String, Value>,
}

impl std::fmt::Debug for CodecInput {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecInput")
            .field("method", &self.canonical_request.method())
            .field(
                "resolved_parameter_names",
                &self.resolved_parameters.keys().collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl CodecInput {
    pub(crate) fn validate_for(&self, binding: &OperationBinding) -> ProtocolResultValue<()> {
        if self.canonical_request.method() != binding.api_type.typed_method() {
            return Err(ProtocolError::invalid_request(
                "canonical request method does not match selected API type",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq)]
pub(crate) struct MaterializedResource {
    pub bytes: Bytes,
    pub mime: String,
    pub file_name: Option<String>,
}

impl std::fmt::Debug for MaterializedResource {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MaterializedResource")
            .field("byte_len", &self.bytes.len())
            .field("mime", &self.mime)
            .field("file_name", &self.file_name)
            .finish()
    }
}

impl MaterializedResource {
    pub(crate) fn new(
        bytes: impl Into<Bytes>,
        mime: impl Into<String>,
        file_name: Option<String>,
    ) -> ProtocolResultValue<Self> {
        let mime = mime.into();
        if mime.trim().is_empty() {
            return Err(ProtocolError::invalid_request(
                "materialized resource MIME type must not be empty",
            ));
        }
        Ok(Self {
            bytes: bytes.into(),
            mime,
            file_name,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CodecLimits {
    pub request_timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

impl CodecLimits {
    fn validate(&self) -> ProtocolResultValue<()> {
        if self.request_timeout.is_zero()
            || self.max_request_bytes == 0
            || self.max_response_bytes == 0
        {
            return Err(ProtocolError::invalid_configuration(
                "codec call timeout and body limits must be greater than zero",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct CodecContext {
    pub base_url: String,
    pub credential: Option<ResolvedCredential>,
    pub resources: BTreeMap<String, MaterializedResource>,
    pub limits: CodecLimits,
}

impl std::fmt::Debug for CodecContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecContext")
            .field("base_url", &self.base_url)
            .field(
                "credential",
                &self.credential.as_ref().map(|_| "[REDACTED]"),
            )
            .field("resource_names", &self.resources.keys().collect::<Vec<_>>())
            .field("limits", &self.limits)
            .finish()
    }
}

impl CodecContext {
    pub(crate) fn validate(&self) -> ProtocolResultValue<()> {
        let parsed = reqwest::Url::parse(&self.base_url)
            .map_err(|_| ProtocolError::invalid_configuration("codec base URL is invalid"))?;
        if !matches!(parsed.scheme(), "http" | "https")
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
            || parsed.query().is_some()
            || parsed.fragment().is_some()
        {
            return Err(ProtocolError::invalid_configuration(
                "codec base URL must be an absolute HTTP URL without credentials, query, or fragment",
            ));
        }
        self.limits.validate()
    }
}

pub(crate) struct CodecCall<'a> {
    pub api_type: ApiType,
    pub input: &'a CodecInput,
    pub context: &'a CodecContext,
}

impl std::fmt::Debug for CodecCall<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecCall")
            .field("api_type", &self.api_type)
            .field("input", &self.input)
            .field("context", &self.context)
            .finish()
    }
}

#[async_trait]
pub(crate) trait OperationCodec: Send + Sync {
    fn descriptor(&self) -> &OperationDescriptor;
    fn api_type(&self) -> ApiType;
    fn execution_modes(&self) -> BTreeSet<ExecutionMode>;
    fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest>;
    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution>;

    async fn decode_stream(
        &self,
        _response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        Err(ProtocolError::new(
            super::ProtocolErrorKind::UnsupportedOperation,
            "streaming decoder is not implemented for this codec",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum NativeTaskOperation {
    Submit,
    Status,
    Result,
    Cancel,
}

#[derive(Clone)]
pub(crate) struct NativeTaskInput<'a> {
    pub operation: NativeTaskOperation,
    pub remote_task_id: Option<&'a str>,
    pub resolved_parameters: &'a BTreeMap<String, Value>,
    pub context: &'a CodecContext,
}

impl std::fmt::Debug for NativeTaskInput<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("NativeTaskInput")
            .field("operation", &self.operation)
            .field("remote_task_id", &self.remote_task_id)
            .field(
                "resolved_parameter_names",
                &self.resolved_parameters.keys().collect::<Vec<_>>(),
            )
            .field("context", &self.context)
            .finish()
    }
}

#[derive(Debug)]
pub(crate) enum NativeTaskOutput {
    Submitted(NativeTaskHandle),
    Status {
        state: NativeTaskState,
        retry_after: Option<Duration>,
    },
    Result(ProtocolOutput),
    Cancelled {
        accepted: bool,
    },
}

#[async_trait]
pub(crate) trait NativeTaskCodec: Send + Sync {
    fn descriptor(&self) -> &OperationDescriptor;
    fn api_type(&self) -> ApiType;
    fn operations(&self) -> BTreeSet<NativeTaskOperation>;
    fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest>;
    async fn decode_native(
        &self,
        operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput>;
}

#[derive(Default)]
pub(crate) struct CodecRegistration {
    pub operation_codecs: Vec<Arc<dyn OperationCodec>>,
    pub native_task_codecs: Vec<Arc<dyn NativeTaskCodec>>,
}

#[derive(Clone)]
struct RegisteredOperation {
    descriptor: OperationDescriptor,
    binding: OperationBinding,
    codec: Option<Arc<dyn OperationCodec>>,
    native_task_codec: Option<Arc<dyn NativeTaskCodec>>,
}

#[derive(Default)]
pub(crate) struct CodecRegistry {
    adapters: BTreeMap<String, AdapterDescriptor>,
    operations: HashMap<(String, String, ApiType), RegisteredOperation>,
}

impl std::fmt::Debug for CodecRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecRegistry")
            .field("adapters", &self.adapters.keys().collect::<Vec<_>>())
            .field("binding_count", &self.operations.len())
            .finish()
    }
}

impl CodecRegistry {
    pub(crate) fn register(
        &mut self,
        descriptor: AdapterDescriptor,
        codecs: Vec<Arc<dyn OperationCodec>>,
    ) -> ProtocolResultValue<()> {
        self.register_codecs(
            descriptor,
            CodecRegistration {
                operation_codecs: codecs,
                native_task_codecs: Vec::new(),
            },
        )
    }

    pub(crate) fn register_codecs(
        &mut self,
        descriptor: AdapterDescriptor,
        codecs: CodecRegistration,
    ) -> ProtocolResultValue<()> {
        descriptor.validate()?;
        self.validate_adapter_identity(&descriptor)?;

        let mut operation_codecs = HashMap::new();
        for codec in codecs.operation_codecs {
            let operation = codec.descriptor();
            operation.validate()?;
            let key = (operation.operation_id.clone(), codec.api_type());
            let declared_modes = operation
                .binding(codec.api_type())
                .map_err(|_| {
                    ProtocolError::invalid_configuration(
                        "operation codec API type is not declared by its descriptor",
                    )
                })?
                .execution_modes
                .iter()
                .copied()
                .filter(|mode| matches!(mode, ExecutionMode::Immediate | ExecutionMode::Stream))
                .collect::<BTreeSet<_>>();
            if declared_modes != codec.execution_modes()
                || operation_codecs.insert(key, codec).is_some()
            {
                return Err(ProtocolError::invalid_configuration(
                    "operation codec has undeclared modes or a duplicate API type binding",
                ));
            }
        }
        let mut native_task_codecs = HashMap::new();
        for codec in codecs.native_task_codecs {
            let operation = codec.descriptor();
            operation.validate()?;
            let key = (operation.operation_id.clone(), codec.api_type());
            let mut required_lifecycle = BTreeSet::from([
                NativeTaskOperation::Submit,
                NativeTaskOperation::Status,
                NativeTaskOperation::Result,
            ]);
            if operation.supports_cancel {
                required_lifecycle.insert(NativeTaskOperation::Cancel);
            }
            if operation.binding(codec.api_type()).is_err()
                || codec.operations() != required_lifecycle
                || native_task_codecs.insert(key, codec).is_some()
            {
                return Err(ProtocolError::invalid_configuration(
                    "native task codec must implement one declared binding and all lifecycle operations",
                ));
            }
        }

        let mut pending = HashMap::new();
        for operation in descriptor.operations.values() {
            for binding in &operation.bindings {
                let key = (operation.operation_id.clone(), binding.api_type);
                let operation_codec = operation_codecs.remove(&key);
                let native_task_codec = native_task_codecs.remove(&key);
                let needs_operation_codec = binding
                    .execution_modes
                    .iter()
                    .any(|mode| matches!(mode, ExecutionMode::Immediate | ExecutionMode::Stream));
                let needs_native_task_codec =
                    binding.execution_modes.contains(&ExecutionMode::NativeTask);
                if needs_operation_codec != operation_codec.is_some()
                    || needs_native_task_codec != native_task_codec.is_some()
                {
                    return Err(ProtocolError::invalid_configuration(
                        "codec set does not exactly cover declared execution modes",
                    ));
                }
                if operation_codec
                    .as_ref()
                    .is_some_and(|codec| codec.descriptor() != operation)
                    || native_task_codec
                        .as_ref()
                        .is_some_and(|codec| codec.descriptor() != operation)
                {
                    return Err(ProtocolError::invalid_configuration(
                        "codec descriptor differs from adapter operation descriptor",
                    ));
                }
                pending.insert(
                    (
                        descriptor.protocol_adapter_id.clone(),
                        operation.operation_id.clone(),
                        binding.api_type,
                    ),
                    RegisteredOperation {
                        descriptor: operation.clone(),
                        binding: binding.clone(),
                        codec: operation_codec,
                        native_task_codec,
                    },
                );
            }
        }
        if !operation_codecs.is_empty() || !native_task_codecs.is_empty() {
            return Err(ProtocolError::invalid_configuration(
                "codec set contains bindings not declared by the adapter",
            ));
        }
        self.operations.extend(pending);
        self.adapters
            .insert(descriptor.protocol_adapter_id.clone(), descriptor);
        Ok(())
    }

    fn validate_adapter_identity(&self, descriptor: &AdapterDescriptor) -> ProtocolResultValue<()> {
        if self.adapters.contains_key(&descriptor.protocol_adapter_id) {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::DuplicateAdapter,
                "protocol adapter is already registered",
            ));
        }
        if let Some(base_adapter_id) = &descriptor.base_adapter_id {
            let base = self.adapters.get(base_adapter_id).ok_or_else(|| {
                ProtocolError::new(
                    super::ProtocolErrorKind::UnknownAdapter,
                    "base protocol adapter is not registered",
                )
            })?;
            if base.protocol_family_id != descriptor.protocol_family_id {
                return Err(ProtocolError::invalid_configuration(
                    "derived and base adapters must belong to the same protocol family",
                ));
            }
        }
        Ok(())
    }

    pub(crate) fn adapter(&self, adapter_id: &str) -> Option<&AdapterDescriptor> {
        self.adapters.get(adapter_id)
    }

    fn registered(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
    ) -> ProtocolResultValue<&RegisteredOperation> {
        self.operations
            .get(&(adapter_id.to_string(), operation_id.to_string(), api_type))
            .ok_or_else(|| {
                let kind = if self.adapters.contains_key(adapter_id) {
                    super::ProtocolErrorKind::UnsupportedOperation
                } else {
                    super::ProtocolErrorKind::UnknownAdapter
                };
                ProtocolError::new(kind, "protocol codec binding is not registered")
            })
    }

    pub(crate) fn operation_descriptor(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
    ) -> ProtocolResultValue<&OperationDescriptor> {
        Ok(&self
            .registered(adapter_id, operation_id, api_type)?
            .descriptor)
    }

    pub(crate) fn codec(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
    ) -> ProtocolResultValue<Arc<dyn OperationCodec>> {
        self.registered(adapter_id, operation_id, api_type)?
            .codec
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| {
                ProtocolError::new(
                    super::ProtocolErrorKind::UnsupportedOperation,
                    "buffered/streaming codec is not registered for this binding",
                )
            })
    }

    pub(crate) fn native_task_codec(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
    ) -> ProtocolResultValue<Arc<dyn NativeTaskCodec>> {
        self.registered(adapter_id, operation_id, api_type)?
            .native_task_codec
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| {
                ProtocolError::new(
                    super::ProtocolErrorKind::UnsupportedOperation,
                    "native task codec is not registered for this binding",
                )
            })
    }

    pub(crate) fn encode(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
        input: &CodecInput,
        context: &CodecContext,
    ) -> ProtocolResultValue<HttpRequest> {
        let registered = self.registered(adapter_id, operation_id, api_type)?;
        if !registered
            .binding
            .execution_modes
            .iter()
            .any(|mode| matches!(mode, ExecutionMode::Immediate | ExecutionMode::Stream))
        {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::UnsupportedOperation,
                "selected binding does not support direct execution",
            ));
        }
        input.validate_for(&registered.binding)?;
        context.validate()?;
        registered
            .codec
            .as_ref()
            .expect("registry validated operation codec")
            .encode(&CodecCall {
                api_type,
                input,
                context,
            })
    }

    pub(crate) async fn decode(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
        response: HttpResponse,
    ) -> ProtocolResultValue<ProtocolExecution> {
        let registered = self.registered(adapter_id, operation_id, api_type)?;
        if !registered
            .binding
            .execution_modes
            .contains(&ExecutionMode::Immediate)
        {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::UnsupportedOperation,
                "selected binding does not declare buffered execution",
            ));
        }
        registered
            .codec
            .as_ref()
            .expect("registry validated operation codec")
            .decode(response)
            .await
    }

    pub(crate) async fn decode_stream(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
        response: StreamingHttpResponse,
    ) -> ProtocolResultValue<ProtocolStream> {
        let registered = self.registered(adapter_id, operation_id, api_type)?;
        if !registered
            .binding
            .execution_modes
            .contains(&ExecutionMode::Stream)
        {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::UnsupportedOperation,
                "selected binding does not declare streaming execution",
            ));
        }
        registered
            .codec
            .as_ref()
            .expect("registry validated operation codec")
            .decode_stream(response)
            .await
    }

    pub(crate) fn encode_native(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
        input: &NativeTaskInput<'_>,
    ) -> ProtocolResultValue<HttpRequest> {
        let registered = self.registered(adapter_id, operation_id, api_type)?;
        if !registered
            .binding
            .execution_modes
            .contains(&ExecutionMode::NativeTask)
        {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::UnsupportedOperation,
                "selected binding does not declare native task execution",
            ));
        }
        input.context.validate()?;
        if input.operation != NativeTaskOperation::Submit
            && input.remote_task_id.is_none_or(|id| id.trim().is_empty())
        {
            return Err(ProtocolError::invalid_request(
                "native lifecycle operation requires a remote task ID",
            ));
        }
        let codec = self.native_task_codec(adapter_id, operation_id, api_type)?;
        if !codec.operations().contains(&input.operation) {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::UnsupportedOperation,
                "native lifecycle operation is not supported",
            ));
        }
        codec.encode_native(input)
    }

    pub(crate) async fn decode_native(
        &self,
        adapter_id: &str,
        operation_id: &str,
        api_type: ApiType,
        lifecycle_operation: NativeTaskOperation,
        response: HttpResponse,
    ) -> ProtocolResultValue<NativeTaskOutput> {
        let codec = self.native_task_codec(adapter_id, operation_id, api_type)?;
        if !codec.operations().contains(&lifecycle_operation) {
            return Err(ProtocolError::new(
                super::ProtocolErrorKind::UnsupportedOperation,
                "native lifecycle operation is not supported",
            ));
        }
        codec.decode_native(lifecycle_operation, response).await
    }
}

fn validate_id(label: &str, value: &str) -> ProtocolResultValue<()> {
    if value.is_empty()
        || value.len() > 128
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err(ProtocolError::invalid_configuration(format!(
            "{label} ID is invalid"
        )));
    }
    Ok(())
}

fn validate_operation_id(value: &str) -> ProtocolResultValue<()> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(ProtocolError::invalid_configuration(
            "operation ID is invalid",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{
        cancellation_pair, poll_until_terminal, HttpBody, PollOutcome, PollPolicy, ProtocolEvent,
        SseConfig, SseFrame,
    };
    use buckyos_api::{EmbeddingTextItem, EmbeddingTextRequest, LlmChatInvokeRequest};
    use futures_util::{stream, StreamExt};
    use reqwest::{header::HeaderMap, Method, StatusCode};
    use serde_json::json;
    use std::sync::atomic::{AtomicU32, Ordering};
    use tokio::time::Instant;

    struct FakeCodec {
        descriptor: OperationDescriptor,
        api_type: ApiType,
    }

    #[async_trait]
    impl OperationCodec for FakeCodec {
        fn descriptor(&self) -> &OperationDescriptor {
            &self.descriptor
        }

        fn api_type(&self) -> ApiType {
            self.api_type
        }

        fn execution_modes(&self) -> BTreeSet<ExecutionMode> {
            self.descriptor
                .binding(self.api_type)
                .unwrap()
                .execution_modes
                .iter()
                .copied()
                .filter(|mode| matches!(mode, ExecutionMode::Immediate | ExecutionMode::Stream))
                .collect()
        }

        fn encode(&self, call: &CodecCall<'_>) -> ProtocolResultValue<HttpRequest> {
            Ok(HttpRequest::new(
                Method::POST,
                format!("{}/{}", call.context.base_url, call.api_type.typed_method()),
            ))
        }

        async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
            Ok(ProtocolExecution::Immediate(ProtocolOutput::new(
                serde_json::from_slice(&response.body).unwrap(),
            )))
        }

        async fn decode_stream(
            &self,
            response: StreamingHttpResponse,
        ) -> ProtocolResultValue<ProtocolStream> {
            let frames =
                crate::protocol::sse_frame_stream(response, SseConfig::default(), 1024).await?;
            let events = frames.filter_map(|frame| async move {
                match frame {
                    Ok(SseFrame::Event(event)) => {
                        Some(Ok(ProtocolEvent::Delta(json!({"data": event.data}))))
                    }
                    Ok(SseFrame::Terminated { .. } | SseFrame::StreamEnd(_)) => None,
                    Err(error) => Some(Err(error)),
                }
            });
            Ok(ProtocolStream {
                events: Box::pin(events),
            })
        }
    }

    struct FakeNativeCodec(OperationDescriptor);

    #[async_trait]
    impl NativeTaskCodec for FakeNativeCodec {
        fn descriptor(&self) -> &OperationDescriptor {
            &self.0
        }

        fn api_type(&self) -> ApiType {
            ApiType::VideoTextToVideo
        }

        fn operations(&self) -> BTreeSet<NativeTaskOperation> {
            BTreeSet::from([
                NativeTaskOperation::Submit,
                NativeTaskOperation::Status,
                NativeTaskOperation::Result,
                NativeTaskOperation::Cancel,
            ])
        }

        fn encode_native(&self, input: &NativeTaskInput<'_>) -> ProtocolResultValue<HttpRequest> {
            Ok(HttpRequest::new(
                Method::POST,
                format!("{}/native/{:?}", input.context.base_url, input.operation),
            ))
        }

        async fn decode_native(
            &self,
            operation: NativeTaskOperation,
            response: HttpResponse,
        ) -> ProtocolResultValue<NativeTaskOutput> {
            let retry_after = response.retry_after;
            let value: Value = response.json(1024)?;
            match operation {
                NativeTaskOperation::Submit => Ok(NativeTaskOutput::Submitted(
                    NativeTaskHandle::new(value["id"].as_str().unwrap())?,
                )),
                NativeTaskOperation::Status => {
                    let state = match value["state"].as_str().unwrap() {
                        "queued" => NativeTaskState::Queued,
                        "running" => NativeTaskState::Running,
                        "succeeded" => NativeTaskState::Succeeded,
                        "failed" => NativeTaskState::Failed,
                        "cancelled" => NativeTaskState::Cancelled,
                        _ => return Err(ProtocolError::invalid_response("unknown task state")),
                    };
                    Ok(NativeTaskOutput::Status { state, retry_after })
                }
                NativeTaskOperation::Result => Ok(NativeTaskOutput::Result(ProtocolOutput::new(
                    value["result"].clone(),
                ))),
                NativeTaskOperation::Cancel => Ok(NativeTaskOutput::Cancelled {
                    accepted: value["cancelled"].as_bool().unwrap_or(false),
                }),
            }
        }
    }

    fn operation(id: &str, bindings: Vec<OperationBinding>) -> OperationDescriptor {
        OperationDescriptor {
            operation_id: id.to_string(),
            bindings,
            supports_cancel: false,
            supports_webhook: false,
            max_request_bytes: 1024,
            max_response_bytes: 1024,
        }
    }

    fn adapter(id: &str, operation: OperationDescriptor) -> AdapterDescriptor {
        AdapterDescriptor {
            protocol_family_id: "test".to_string(),
            protocol_adapter_id: id.to_string(),
            interface_generation: "v1".to_string(),
            base_adapter_id: None,
            status: AdapterStatus::Stable,
            operations: BTreeMap::from([(operation.operation_id.clone(), operation)]),
        }
    }

    fn context(base_url: &str, secret: &str) -> CodecContext {
        CodecContext {
            base_url: base_url.to_string(),
            credential: Some(ResolvedCredential::bearer("ref:test", secret).unwrap()),
            resources: BTreeMap::new(),
            limits: CodecLimits {
                request_timeout: Duration::from_secs(10),
                max_request_bytes: 1024,
                max_response_bytes: 1024,
            },
        }
    }

    fn response(body: Value) -> HttpResponse {
        HttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Bytes::from(serde_json::to_vec(&body).unwrap()),
            request_id: "req-1".to_string(),
            retry_after: None,
        }
    }

    #[test]
    fn one_official_operation_binds_and_invokes_multiple_api_types() {
        let descriptor = operation(
            "interactions.create",
            vec![
                OperationBinding::new(ApiType::Llm, [ExecutionMode::Immediate]),
                OperationBinding::new(ApiType::EmbeddingText, [ExecutionMode::Immediate]),
            ],
        );
        let mut registry = CodecRegistry::default();
        registry
            .register(
                adapter("gemini", descriptor.clone()),
                vec![
                    Arc::new(FakeCodec {
                        descriptor: descriptor.clone(),
                        api_type: ApiType::Llm,
                    }),
                    Arc::new(FakeCodec {
                        descriptor,
                        api_type: ApiType::EmbeddingText,
                    }),
                ],
            )
            .unwrap();
        let llm = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "model@instance",
                Vec::new(),
            )),
            resolved_parameters: BTreeMap::new(),
        };
        let embedding = CodecInput {
            canonical_request: AiccCall::EmbeddingText(EmbeddingTextRequest::new(
                "model@instance",
                vec![EmbeddingTextItem::Text {
                    text: "hello".to_string(),
                    id: None,
                }],
            )),
            resolved_parameters: BTreeMap::new(),
        };
        let call_context = context("https://one.example", "first-secret");
        assert!(registry
            .encode(
                "gemini",
                "interactions.create",
                ApiType::Llm,
                &llm,
                &call_context,
            )
            .unwrap()
            .url
            .ends_with("chat.completions.create"));
        assert!(registry
            .encode(
                "gemini",
                "interactions.create",
                ApiType::EmbeddingText,
                &embedding,
                &call_context,
            )
            .is_ok());
        let unsupported = registry
            .codec("gemini", "interactions.create", ApiType::VisionOcr)
            .err()
            .unwrap();
        assert_eq!(
            unsupported.kind,
            super::super::ProtocolErrorKind::UnsupportedOperation
        );
    }

    #[test]
    fn registration_is_transactional_and_rejects_duplicate_bindings() {
        let descriptor = operation(
            "interactions.create",
            vec![
                OperationBinding::new(ApiType::Llm, [ExecutionMode::Immediate]),
                OperationBinding::new(ApiType::VisionOcr, [ExecutionMode::Stream]),
            ],
        );
        let mut registry = CodecRegistry::default();
        assert!(registry
            .register(
                adapter("broken", descriptor.clone()),
                vec![Arc::new(FakeCodec {
                    descriptor,
                    api_type: ApiType::Llm,
                })],
            )
            .is_err());
        assert!(registry.adapter("broken").is_none());
        assert!(registry
            .codec("broken", "interactions.create", ApiType::Llm)
            .is_err());

        let duplicate = operation(
            "official.operation",
            vec![
                OperationBinding::new(ApiType::Llm, [ExecutionMode::Immediate]),
                OperationBinding::new(ApiType::Llm, [ExecutionMode::Stream]),
            ],
        );
        assert!(duplicate.validate().is_err());
    }

    #[test]
    fn selected_api_type_rejects_mismatched_canonical_request() {
        let descriptor = operation(
            "models.embedContent",
            vec![OperationBinding::new(
                ApiType::EmbeddingText,
                [ExecutionMode::Immediate],
            )],
        );
        let mut registry = CodecRegistry::default();
        registry
            .register(
                adapter("gemini", descriptor.clone()),
                vec![Arc::new(FakeCodec {
                    descriptor,
                    api_type: ApiType::EmbeddingText,
                })],
            )
            .unwrap();
        let wrong = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "model@instance",
                Vec::new(),
            )),
            resolved_parameters: BTreeMap::new(),
        };
        assert_eq!(
            registry
                .encode(
                    "gemini",
                    "models.embedContent",
                    ApiType::EmbeddingText,
                    &wrong,
                    &context("https://one.example", "secret"),
                )
                .unwrap_err()
                .kind,
            super::super::ProtocolErrorKind::InvalidRequest
        );
    }

    #[test]
    fn context_is_reusable_and_never_debugs_credentials() {
        let first = context("https://one.example", "first-secret");
        let second = context("https://two.example", "second-secret");
        assert_ne!(first.base_url, second.base_url);
        for rendered in [format!("{first:?}"), format!("{second:?}")] {
            assert!(!rendered.contains("first-secret"));
            assert!(!rendered.contains("second-secret"));
        }
    }

    #[test]
    fn materialized_resource_round_trips_through_multipart() {
        let mut call_context = context("https://upload.example", "secret");
        call_context.resources.insert(
            "source".to_string(),
            MaterializedResource::new(
                Bytes::from_static(b"image-bytes"),
                "image/png",
                Some("input.png".to_string()),
            )
            .unwrap(),
        );
        let resource = &call_context.resources["source"];
        let mut multipart = crate::protocol::MultipartBody::new(2, 1024).unwrap();
        multipart
            .push(crate::protocol::MultipartPart::file(
                "file",
                resource.bytes.clone(),
                resource.file_name.clone().unwrap(),
                resource.mime.clone(),
            ))
            .unwrap();
        let HttpBody::Multipart(round_trip) = HttpBody::Multipart(multipart) else {
            unreachable!()
        };
        assert_eq!(
            round_trip.parts()[0].bytes,
            Bytes::from_static(b"image-bytes")
        );
        assert_eq!(round_trip.parts()[0].mime.as_deref(), Some("image/png"));
        assert_eq!(
            round_trip.parts()[0].file_name.as_deref(),
            Some("input.png")
        );
    }

    #[tokio::test]
    async fn registry_dispatches_streaming_and_buffered_codecs() {
        let descriptor = operation(
            "responses.create",
            vec![OperationBinding::new(
                ApiType::Llm,
                [ExecutionMode::Immediate, ExecutionMode::Stream],
            )],
        );
        let mut registry = CodecRegistry::default();
        registry
            .register(
                adapter("responses", descriptor.clone()),
                vec![Arc::new(FakeCodec {
                    descriptor,
                    api_type: ApiType::Llm,
                })],
            )
            .unwrap();
        let decoded = registry
            .decode(
                "responses",
                "responses.create",
                ApiType::Llm,
                response(json!({"ok":true})),
            )
            .await
            .unwrap();
        assert!(matches!(decoded, ProtocolExecution::Immediate(_)));

        let wire = StreamingHttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Box::pin(stream::iter(vec![
                Ok(Bytes::from_static(b"data: {\"n\":")),
                Ok(Bytes::from_static(b"1}\n\ndata: [DONE]\n\n")),
            ])),
            request_id: "req-stream".to_string(),
            retry_after: None,
        };
        let mut decoded = registry
            .decode_stream("responses", "responses.create", ApiType::Llm, wire)
            .await
            .unwrap();
        assert_eq!(
            decoded.events.next().await.unwrap().unwrap(),
            ProtocolEvent::Delta(json!({"data":"{\"n\":1}"}))
        );
        assert!(decoded.events.next().await.is_none());
    }

    #[tokio::test]
    async fn registry_rejects_streaming_when_mode_is_not_declared() {
        let descriptor = operation(
            "chat.completions",
            vec![OperationBinding::new(
                ApiType::Llm,
                [ExecutionMode::Immediate],
            )],
        );
        let mut registry = CodecRegistry::default();
        registry
            .register(
                adapter("chat", descriptor.clone()),
                vec![Arc::new(FakeCodec {
                    descriptor,
                    api_type: ApiType::Llm,
                })],
            )
            .unwrap();
        let wire = StreamingHttpResponse {
            status: StatusCode::OK,
            headers: HeaderMap::new(),
            body: Box::pin(stream::empty()),
            request_id: "req-stream".to_string(),
            retry_after: None,
        };
        assert_eq!(
            registry
                .decode_stream("chat", "chat.completions", ApiType::Llm, wire)
                .await
                .unwrap_err()
                .kind,
            super::super::ProtocolErrorKind::UnsupportedOperation
        );
    }

    #[tokio::test]
    async fn native_lifecycle_has_no_aicc_call_and_reuses_task_primitives() {
        let mut descriptor = operation(
            "videos",
            vec![OperationBinding::new(
                ApiType::VideoTextToVideo,
                [ExecutionMode::NativeTask],
            )],
        );
        descriptor.supports_cancel = true;
        let mut registry = CodecRegistry::default();
        registry
            .register_codecs(
                adapter("videos", descriptor.clone()),
                CodecRegistration {
                    operation_codecs: Vec::new(),
                    native_task_codecs: vec![Arc::new(FakeNativeCodec(descriptor))],
                },
            )
            .unwrap();
        let parameters = BTreeMap::from([("prompt".to_string(), json!("ocean"))]);
        let call_context = context("https://video.example", "secret");
        let submit = NativeTaskInput {
            operation: NativeTaskOperation::Submit,
            remote_task_id: None,
            resolved_parameters: &parameters,
            context: &call_context,
        };
        registry
            .encode_native("videos", "videos", ApiType::VideoTextToVideo, &submit)
            .unwrap();
        let NativeTaskOutput::Submitted(handle) = registry
            .decode_native(
                "videos",
                "videos",
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Submit,
                response(json!({"id":"video-1"})),
            )
            .await
            .unwrap()
        else {
            panic!("expected submitted task")
        };
        assert_eq!(handle.remote_task_id, "video-1");

        let attempts = Arc::new(AtomicU32::new(0));
        let observed = Arc::clone(&attempts);
        let (_cancel_handle, cancellation) = cancellation_pair();
        let final_state = poll_until_terminal(
            &PollPolicy {
                initial_delay: Duration::from_millis(1),
                maximum_delay: Duration::from_millis(1),
                multiplier: 1,
                maximum_attempts: Some(2),
            },
            Instant::now() + Duration::from_secs(1),
            &cancellation,
            move |_| {
                let attempt = observed.fetch_add(1, Ordering::SeqCst);
                async move {
                    if attempt == 0 {
                        Ok(PollOutcome::Pending {
                            state: NativeTaskState::Running,
                            retry_after: Some(Duration::from_millis(1)),
                        })
                    } else {
                        Ok(PollOutcome::Complete(NativeTaskState::Succeeded))
                    }
                }
            },
        )
        .await
        .unwrap();
        assert_eq!(final_state, NativeTaskState::Succeeded);

        for (wire_state, expected) in [
            ("failed", NativeTaskState::Failed),
            ("cancelled", NativeTaskState::Cancelled),
        ] {
            let NativeTaskOutput::Status { state, .. } = registry
                .decode_native(
                    "videos",
                    "videos",
                    ApiType::VideoTextToVideo,
                    NativeTaskOperation::Status,
                    response(json!({"state":wire_state})),
                )
                .await
                .unwrap()
            else {
                panic!("expected task status")
            };
            assert_eq!(state, expected);
        }

        let mut retry_response = response(json!({"state":"running"}));
        retry_response.retry_after = Some(Duration::from_secs(4));
        let NativeTaskOutput::Status { retry_after, .. } = registry
            .decode_native(
                "videos",
                "videos",
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Status,
                retry_response,
            )
            .await
            .unwrap()
        else {
            panic!("expected task status")
        };
        assert_eq!(retry_after, Some(Duration::from_secs(4)));

        let NativeTaskOutput::Result(output) = registry
            .decode_native(
                "videos",
                "videos",
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Result,
                response(json!({"result":{"url":"artifact"}})),
            )
            .await
            .unwrap()
        else {
            panic!("expected task result")
        };
        assert_eq!(output.value, json!({"url":"artifact"}));

        let NativeTaskOutput::Cancelled { accepted } = registry
            .decode_native(
                "videos",
                "videos",
                ApiType::VideoTextToVideo,
                NativeTaskOperation::Cancel,
                response(json!({"cancelled":true})),
            )
            .await
            .unwrap()
        else {
            panic!("expected cancellation result")
        };
        assert!(accepted);
    }

    #[tokio::test]
    async fn task_primitives_cover_timeout_and_cancellation() {
        let (_handle, cancellation) = cancellation_pair();
        let timeout = poll_until_terminal(
            &PollPolicy::default(),
            Instant::now(),
            &cancellation,
            |_| async { Ok::<_, ProtocolError>(PollOutcome::<()>::Complete(())) },
        )
        .await
        .unwrap_err();
        assert_eq!(
            timeout.kind,
            super::super::ProtocolErrorKind::DeadlineExceeded
        );

        let (handle, cancellation) = cancellation_pair();
        handle.cancel();
        let cancelled = poll_until_terminal(
            &PollPolicy::default(),
            Instant::now() + Duration::from_secs(1),
            &cancellation,
            |_| async { Ok::<_, ProtocolError>(PollOutcome::<()>::Complete(())) },
        )
        .await
        .unwrap_err();
        assert_eq!(cancelled.kind, super::super::ProtocolErrorKind::Cancelled);
    }
}
