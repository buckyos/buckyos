use super::{ProtocolError, ProtocolResultValue};
use buckyos_api::{AiArtifact, AiMethodStatus, AiUsage, ApiType};
use futures_util::Stream;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::pin::Pin;
use std::time::Duration;

const PROVIDER_ARTIFACT_ID_KEY: &str = "aicc_provider_artifact_id";
const PROVIDER_ARTIFACT_EXPIRES_AT_MS_KEY: &str = "aicc_provider_artifact_expires_at_ms";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ProviderArtifactRef {
    pub id: String,
    pub expires_at_ms: Option<i64>,
}

impl ProviderArtifactRef {
    pub(crate) fn new(id: impl Into<String>, expires_at_ms: Option<i64>) -> Self {
        Self {
            id: id.into(),
            expires_at_ms,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ProtocolOutput {
    pub value: Value,
    pub usage: Option<AiUsage>,
    pub artifacts: Vec<AiArtifact>,
}

impl ProtocolOutput {
    pub(crate) fn new(value: Value) -> Self {
        Self {
            value,
            usage: None,
            artifacts: Vec::new(),
        }
    }

    pub(crate) fn bind_provider_artifact_refs(
        &mut self,
        artifact_refs: &BTreeMap<String, ProviderArtifactRef>,
    ) {
        for (artifact_name, artifact_ref) in artifact_refs {
            let Some(artifact) = self
                .artifacts
                .iter_mut()
                .find(|artifact| artifact.name == *artifact_name)
            else {
                continue;
            };
            let metadata = artifact
                .metadata
                .get_or_insert_with(|| Value::Object(Map::new()));
            if let Some(metadata) = metadata.as_object_mut() {
                metadata.insert(
                    PROVIDER_ARTIFACT_ID_KEY.to_string(),
                    Value::String(artifact_ref.id.clone()),
                );
                if let Some(expires_at_ms) = artifact_ref.expires_at_ms {
                    metadata.insert(
                        PROVIDER_ARTIFACT_EXPIRES_AT_MS_KEY.to_string(),
                        Value::Number(expires_at_ms.into()),
                    );
                }
            }
        }
    }

    pub(crate) fn validate_canonical_type(&self, api_type: ApiType) -> ProtocolResultValue<()> {
        let object = self.value.as_object().ok_or_else(|| {
            ProtocolError::invalid_response("canonical provider output must be an object")
        })?;
        let mut response = object.clone();
        response.insert("task_id".to_owned(), Value::String("validation".to_owned()));
        response.insert(
            "status".to_owned(),
            serde_json::to_value(AiMethodStatus::Succeeded).expect("AiMethodStatus must serialize"),
        );
        let response = Value::Object(response);
        macro_rules! validate {
            ($response_type:ty) => {
                validate_response::<$response_type>(api_type, response)
            };
        }
        match api_type {
            ApiType::Llm => validate!(buckyos_api::LlmChatInvokeResponse),
            ApiType::EmbeddingText => validate!(buckyos_api::EmbeddingTextResponse),
            ApiType::EmbeddingMultimodal => validate!(buckyos_api::EmbeddingMultimodalResponse),
            ApiType::Rerank => validate!(buckyos_api::RerankResponse),
            ApiType::ImageTextToImage => validate!(buckyos_api::TextToImageInvokeResponse),
            ApiType::ImageImageToImage => validate!(buckyos_api::ImageToImageResponse),
            ApiType::ImageInpaint => validate!(buckyos_api::ImageInpaintResponse),
            ApiType::ImageUpscale => validate!(buckyos_api::ImageUpscaleResponse),
            ApiType::ImageBackgroundRemove => {
                validate!(buckyos_api::ImageBackgroundRemoveResponse)
            }
            ApiType::VisionOcr => validate!(buckyos_api::VisionOcrResponse),
            ApiType::VisionCaption => validate!(buckyos_api::VisionCaptionResponse),
            ApiType::VisionDetect => validate!(buckyos_api::VisionDetectResponse),
            ApiType::VisionSegment => validate!(buckyos_api::VisionSegmentResponse),
            ApiType::AudioTextToSpeech => validate!(buckyos_api::AudioTextToSpeechResponse),
            ApiType::AudioSpeechRecognition => {
                validate!(buckyos_api::AudioSpeechRecognitionResponse)
            }
            ApiType::AudioMusic => validate!(buckyos_api::AudioMusicResponse),
            ApiType::AudioEnhance => validate!(buckyos_api::AudioEnhanceResponse),
            ApiType::VideoTextToVideo => validate!(buckyos_api::VideoTextToVideoResponse),
            ApiType::VideoImageToVideo => validate!(buckyos_api::VideoImageToVideoResponse),
            ApiType::VideoToVideo => validate!(buckyos_api::VideoToVideoResponse),
            ApiType::VideoExtend => validate!(buckyos_api::VideoExtendResponse),
            ApiType::VideoUpscale => validate!(buckyos_api::VideoUpscaleResponse),
            ApiType::AgentComputerUse => validate!(buckyos_api::ComputerUseResponse),
        }
    }

    pub(crate) fn take_provider_artifact_refs(&mut self) -> BTreeMap<String, ProviderArtifactRef> {
        self.artifacts
            .iter_mut()
            .filter_map(|artifact| {
                let metadata = artifact.metadata.as_mut().and_then(Value::as_object_mut)?;
                let artifact_id = metadata
                    .remove(PROVIDER_ARTIFACT_ID_KEY)?
                    .as_str()?
                    .to_owned();
                let expires_at_ms = metadata
                    .remove(PROVIDER_ARTIFACT_EXPIRES_AT_MS_KEY)
                    .and_then(|value| value.as_i64());
                Some((
                    artifact.name.clone(),
                    ProviderArtifactRef::new(artifact_id, expires_at_ms),
                ))
            })
            .collect()
    }
}

fn validate_response<T: DeserializeOwned>(
    api_type: ApiType,
    response: Value,
) -> ProtocolResultValue<()> {
    serde_json::from_value::<T>(response)
        .map(|_| ())
        .map_err(|error| {
            ProtocolError::invalid_response(format!(
                "canonical output does not match {} response: {error}",
                api_type.typed_method()
            ))
        })
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProtocolEvent {
    Delta(Value),
    Progress(Value),
    Final(ProtocolOutput),
}

pub(crate) type ProtocolEventStream =
    Pin<Box<dyn Stream<Item = ProtocolResultValue<ProtocolEvent>> + Send + 'static>>;

pub(crate) struct ProtocolStream {
    pub events: ProtocolEventStream,
}

impl std::fmt::Debug for ProtocolStream {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ProtocolStream { events: <stream> }")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeTaskState {
    Submitted,
    Queued,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl NativeTaskState {
    pub(crate) fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeTaskHandle {
    pub remote_task_id: String,
    pub state: NativeTaskState,
    pub poll_after: Option<Duration>,
    pub cancel_supported: bool,
    pub webhook_supported: bool,
    pub result_artifacts: BTreeMap<String, ProviderArtifactRef>,
}

impl NativeTaskHandle {
    pub(crate) fn new(remote_task_id: impl Into<String>) -> ProtocolResultValue<Self> {
        let remote_task_id = remote_task_id.into();
        if remote_task_id.trim().is_empty() {
            return Err(ProtocolError::invalid_response(
                "native task ID must not be empty",
            ));
        }
        Ok(Self {
            remote_task_id,
            state: NativeTaskState::Submitted,
            poll_after: None,
            cancel_supported: false,
            webhook_supported: false,
            result_artifacts: BTreeMap::new(),
        })
    }
}

#[derive(Debug)]
pub(crate) enum ProtocolExecution {
    Immediate(ProtocolOutput),
    Stream(ProtocolStream),
    NativeTask(NativeTaskHandle),
}

impl ProtocolExecution {
    pub(crate) fn mode(&self) -> super::ExecutionMode {
        match self {
            Self::Immediate(_) => super::ExecutionMode::Immediate,
            Self::Stream(_) => super::ExecutionMode::Stream,
            Self::NativeTask(_) => super::ExecutionMode::NativeTask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::ResourceRef;
    use serde_json::json;

    #[test]
    fn unifies_immediate_and_native_task_modes() {
        assert_eq!(
            ProtocolExecution::Immediate(ProtocolOutput::new(json!({"ok": true}))).mode(),
            super::super::ExecutionMode::Immediate
        );
        assert_eq!(
            ProtocolExecution::NativeTask(NativeTaskHandle::new("remote-1").unwrap()).mode(),
            super::super::ExecutionMode::NativeTask
        );
    }

    #[test]
    fn provider_artifact_ids_bind_by_artifact_name_and_are_consumed() {
        let mut output = ProtocolOutput {
            value: Value::Null,
            usage: None,
            artifacts: vec![AiArtifact {
                name: "video".to_string(),
                resource: ResourceRef::base64("video/mp4".to_string(), "dmlkZW8=".to_string()),
                mime: Some("video/mp4".to_string()),
                metadata: Some(json!({"digest":"sha256:abc"})),
            }],
        };
        output.bind_provider_artifact_refs(&BTreeMap::from([
            (
                "video".to_string(),
                ProviderArtifactRef::new("provider-video-1", Some(123)),
            ),
            (
                "missing".to_string(),
                ProviderArtifactRef::new("ignored", None),
            ),
        ]));
        assert_eq!(
            output.take_provider_artifact_refs(),
            BTreeMap::from([(
                "video".to_string(),
                ProviderArtifactRef::new("provider-video-1", Some(123)),
            )])
        );
        assert_eq!(
            output.artifacts[0].metadata.as_ref().unwrap()["digest"],
            "sha256:abc"
        );
        assert!(output.take_provider_artifact_refs().is_empty());
    }

    #[test]
    fn canonical_output_is_checked_against_the_selected_api_type() {
        let valid = ProtocolOutput::new(serde_json::json!({"images": []}));
        valid
            .validate_canonical_type(ApiType::ImageTextToImage)
            .unwrap();
        let invalid = ProtocolOutput::new(serde_json::json!({"images": "not-an-array"}));
        assert!(invalid
            .validate_canonical_type(ApiType::ImageTextToImage)
            .is_err());
    }
}
