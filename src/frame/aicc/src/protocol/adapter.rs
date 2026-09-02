use super::{HttpRequest, HttpResponse, ProtocolError, ProtocolExecution, ProtocolResultValue};
use async_trait::async_trait;
use buckyos_api::{AiccCall, ApiType, Capability};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

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
pub(crate) struct OperationDescriptor {
    pub operation_id: String,
    pub api_type: ApiType,
    pub capability: Capability,
    pub supported_features: BTreeSet<String>,
    pub execution_modes: BTreeSet<ExecutionMode>,
    pub supports_cancel: bool,
    pub supports_webhook: bool,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

impl OperationDescriptor {
    pub(crate) fn validate(&self) -> ProtocolResultValue<()> {
        validate_id("operation", &self.operation_id)?;
        if self.capability != self.api_type.capability() {
            return Err(ProtocolError::invalid_configuration(
                "operation capability does not match its API type",
            ));
        }
        if self.execution_modes.is_empty() {
            return Err(ProtocolError::invalid_configuration(
                "operation must support at least one execution mode",
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
        if self.max_request_bytes == 0 || self.max_response_bytes == 0 {
            return Err(ProtocolError::invalid_configuration(
                "operation body limits must be greater than zero",
            ));
        }
        if self.supports_cancel && !self.execution_modes.contains(&ExecutionMode::NativeTask) {
            return Err(ProtocolError::invalid_configuration(
                "cancel support requires native task execution",
            ));
        }
        if self.supports_webhook && !self.execution_modes.contains(&ExecutionMode::NativeTask) {
            return Err(ProtocolError::invalid_configuration(
                "webhook support requires native task execution",
            ));
        }
        Ok(())
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
    pub(crate) fn validate_for(&self, descriptor: &OperationDescriptor) -> ProtocolResultValue<()> {
        if self.canonical_request.method() != descriptor.api_type.typed_method() {
            return Err(ProtocolError::invalid_request(
                "canonical request method does not match codec API type",
            ));
        }
        Ok(())
    }
}

#[async_trait]
pub(crate) trait OperationCodec: Send + Sync {
    fn descriptor(&self) -> &OperationDescriptor;
    fn encode(&self, input: &CodecInput) -> ProtocolResultValue<HttpRequest>;
    async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution>;
}

#[derive(Clone)]
struct RegisteredOperation {
    descriptor: OperationDescriptor,
    codec: Arc<dyn OperationCodec>,
}

#[derive(Default)]
pub(crate) struct CodecRegistry {
    adapters: BTreeMap<String, AdapterDescriptor>,
    operations: BTreeMap<(String, String), RegisteredOperation>,
}

impl std::fmt::Debug for CodecRegistry {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodecRegistry")
            .field("adapters", &self.adapters.keys().collect::<Vec<_>>())
            .field("operation_count", &self.operations.len())
            .finish()
    }
}

impl CodecRegistry {
    pub(crate) fn register(
        &mut self,
        descriptor: AdapterDescriptor,
        codecs: Vec<Arc<dyn OperationCodec>>,
    ) -> ProtocolResultValue<()> {
        descriptor.validate()?;
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
        if codecs.len() != descriptor.operations.len() {
            return Err(ProtocolError::invalid_configuration(
                "codec set does not cover exactly the declared operations",
            ));
        }
        let mut pending = BTreeMap::new();
        for codec in codecs {
            let operation = codec.descriptor().clone();
            operation.validate()?;
            let declared = descriptor
                .operations
                .get(&operation.operation_id)
                .ok_or_else(|| {
                    ProtocolError::invalid_configuration(
                        "codec operation is not declared by its adapter",
                    )
                })?;
            if declared != &operation {
                return Err(ProtocolError::invalid_configuration(
                    "codec descriptor differs from adapter operation descriptor",
                ));
            }
            if pending
                .insert(
                    operation.operation_id.clone(),
                    RegisteredOperation {
                        descriptor: operation,
                        codec,
                    },
                )
                .is_some()
            {
                return Err(ProtocolError::invalid_configuration(
                    "adapter contains duplicate operation codecs",
                ));
            }
        }
        let adapter_id = descriptor.protocol_adapter_id.clone();
        for (operation_id, operation) in pending {
            self.operations
                .insert((adapter_id.clone(), operation_id), operation);
        }
        self.adapters.insert(adapter_id, descriptor);
        Ok(())
    }

    pub(crate) fn adapter(&self, adapter_id: &str) -> Option<&AdapterDescriptor> {
        self.adapters.get(adapter_id)
    }

    pub(crate) fn operation_descriptor(
        &self,
        adapter_id: &str,
        operation_id: &str,
    ) -> ProtocolResultValue<&OperationDescriptor> {
        self.operations
            .get(&(adapter_id.to_string(), operation_id.to_string()))
            .map(|registered| &registered.descriptor)
            .ok_or_else(|| {
                let kind = if self.adapters.contains_key(adapter_id) {
                    super::ProtocolErrorKind::UnsupportedOperation
                } else {
                    super::ProtocolErrorKind::UnknownAdapter
                };
                ProtocolError::new(kind, "protocol codec is not registered")
            })
    }

    pub(crate) fn codec(
        &self,
        adapter_id: &str,
        operation_id: &str,
    ) -> ProtocolResultValue<Arc<dyn OperationCodec>> {
        self.operations
            .get(&(adapter_id.to_string(), operation_id.to_string()))
            .map(|registered| Arc::clone(&registered.codec))
            .ok_or_else(|| {
                let kind = if self.adapters.contains_key(adapter_id) {
                    super::ProtocolErrorKind::UnsupportedOperation
                } else {
                    super::ProtocolErrorKind::UnknownAdapter
                };
                ProtocolError::new(kind, "protocol codec is not registered")
            })
    }

    pub(crate) fn encode(
        &self,
        adapter_id: &str,
        operation_id: &str,
        input: &CodecInput,
    ) -> ProtocolResultValue<HttpRequest> {
        let registered = self
            .operations
            .get(&(adapter_id.to_string(), operation_id.to_string()))
            .ok_or_else(|| {
                let kind = if self.adapters.contains_key(adapter_id) {
                    super::ProtocolErrorKind::UnsupportedOperation
                } else {
                    super::ProtocolErrorKind::UnknownAdapter
                };
                ProtocolError::new(kind, "protocol codec is not registered")
            })?;
        input.validate_for(&registered.descriptor)?;
        registered.codec.encode(input)
    }

    pub(crate) async fn decode(
        &self,
        adapter_id: &str,
        operation_id: &str,
        response: HttpResponse,
    ) -> ProtocolResultValue<ProtocolExecution> {
        self.codec(adapter_id, operation_id)?.decode(response).await
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ProtocolOutput;
    use buckyos_api::{LlmChatHelperRequest, LlmChatInvokeRequest};
    use reqwest::Method;

    struct FakeCodec(OperationDescriptor);

    #[async_trait]
    impl OperationCodec for FakeCodec {
        fn descriptor(&self) -> &OperationDescriptor {
            &self.0
        }

        fn encode(&self, _input: &CodecInput) -> ProtocolResultValue<HttpRequest> {
            Ok(HttpRequest::new(
                Method::POST,
                "https://example.invalid/operation",
            ))
        }

        async fn decode(&self, response: HttpResponse) -> ProtocolResultValue<ProtocolExecution> {
            Ok(ProtocolExecution::Immediate(ProtocolOutput::new(
                serde_json::from_slice(&response.body).unwrap(),
            )))
        }
    }

    fn operation(id: &str) -> OperationDescriptor {
        OperationDescriptor {
            operation_id: id.to_string(),
            api_type: ApiType::Llm,
            capability: Capability::Llm,
            supported_features: BTreeSet::new(),
            execution_modes: BTreeSet::from([ExecutionMode::Immediate]),
            supports_cancel: false,
            supports_webhook: false,
            max_request_bytes: 1024,
            max_response_bytes: 1024,
        }
    }

    fn adapter(id: &str, base: Option<&str>, operation: OperationDescriptor) -> AdapterDescriptor {
        AdapterDescriptor {
            protocol_family_id: "openai".to_string(),
            protocol_adapter_id: id.to_string(),
            interface_generation: "responses-v1".to_string(),
            base_adapter_id: base.map(str::to_string),
            status: AdapterStatus::Stable,
            operations: BTreeMap::from([(operation.operation_id.clone(), operation)]),
        }
    }

    #[test]
    fn registry_is_explicit_and_transactional() {
        let mut registry = CodecRegistry::default();
        let descriptor = operation("responses.create");
        registry
            .register(
                adapter("openai-responses", None, descriptor.clone()),
                vec![Arc::new(FakeCodec(descriptor.clone()))],
            )
            .unwrap();
        assert_eq!(
            registry
                .operation_descriptor("openai-responses", "responses.create")
                .unwrap(),
            &descriptor
        );
        assert!(registry
            .operation_descriptor("some-provider", "responses.create")
            .is_err());

        let mismatched = operation("other");
        assert!(registry
            .register(
                adapter("broken", None, descriptor),
                vec![Arc::new(FakeCodec(mismatched))],
            )
            .is_err());
        assert!(registry.adapter("broken").is_none());
    }

    #[test]
    fn derived_adapter_requires_registered_base_in_same_family() {
        let mut registry = CodecRegistry::default();
        let descriptor = operation("responses.create");
        assert!(registry
            .register(
                adapter("sn-openai", Some("openai-responses"), descriptor.clone()),
                vec![Arc::new(FakeCodec(descriptor.clone()))],
            )
            .is_err());
        registry
            .register(
                adapter("openai-responses", None, descriptor.clone()),
                vec![Arc::new(FakeCodec(descriptor.clone()))],
            )
            .unwrap();
        registry
            .register(
                adapter("sn-openai", Some("openai-responses"), descriptor.clone()),
                vec![Arc::new(FakeCodec(descriptor))],
            )
            .unwrap();
    }

    #[test]
    fn codec_entry_rejects_helper_before_provider_wire_encoding() {
        let mut registry = CodecRegistry::default();
        let descriptor = operation("responses.create");
        registry
            .register(
                adapter("openai-responses", None, descriptor.clone()),
                vec![Arc::new(FakeCodec(descriptor))],
            )
            .unwrap();
        let exact = CodecInput {
            canonical_request: AiccCall::ChatCompletionsCreate(LlmChatInvokeRequest::new(
                "model@instance",
                Vec::new(),
            )),
            resolved_parameters: BTreeMap::new(),
        };
        registry
            .encode("openai-responses", "responses.create", &exact)
            .unwrap();

        let helper = CodecInput {
            canonical_request: AiccCall::HelperLlmChat(LlmChatHelperRequest::new(
                "llm.chat",
                Vec::new(),
            )),
            resolved_parameters: BTreeMap::new(),
        };
        assert_eq!(
            registry
                .encode("openai-responses", "responses.create", &helper)
                .unwrap_err()
                .kind,
            super::super::ProtocolErrorKind::InvalidRequest
        );
    }
}
