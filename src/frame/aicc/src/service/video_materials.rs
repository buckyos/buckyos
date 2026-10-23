//! Measures and prepares the media a `video.generate` request references,
//! after the policy pre-route and before the final route.
//!
//! Reference video and audio are always read: their real duration and audio
//! track decide which models can take them. A URL is downloaded through the
//! usual URL checks and then submitted as AICC's own copy, so the bytes that
//! were measured are the bytes the model receives. A reference video with
//! `include_audio=false` loses its audio tracks here.

use super::*;
use crate::media::{probe, strip_audio_tracks, MediaContainer, MediaFacts};
use crate::video::{ConditionFacts, DerivedMaterial, MaterialTransform, VideoMaterialFacts};
use buckyos_api::{video_error, VideoConditionKind, VideoGenerateRequest};

pub(super) struct PreparedVideoRequest {
    pub request: VideoGenerateRequest,
    pub facts: VideoMaterialFacts,
}

fn needs_bytes(
    request: &VideoGenerateRequest,
    kind: VideoConditionKind,
    resource: &ResourceRef,
) -> bool {
    match kind {
        VideoConditionKind::ReferenceVideo | VideoConditionKind::ReferenceAudio => true,
        // A keyframe ratio can only be checked against the image itself.
        VideoConditionKind::Keyframe
            if request
                .output
                .as_ref()
                .is_some_and(|output| output.aspect_ratio.is_some()) =>
        {
            true
        }
        _ => !matches!(resource, ResourceRef::Url { .. }),
    }
}

fn kind_accepts(kind: VideoConditionKind, facts: &MediaFacts) -> bool {
    match kind {
        VideoConditionKind::ReferenceVideo => facts.has_video,
        VideoConditionKind::ReferenceAudio => facts.has_audio && !facts.has_video,
        VideoConditionKind::ReferenceImage | VideoConditionKind::Keyframe => {
            facts.container.is_image()
        }
    }
}

fn condition_error(
    code: AiccErrorCode,
    message: impl Into<String>,
    index: usize,
    id: &str,
) -> RPCErrors {
    video_error(
        code,
        message,
        &format!("/conditions/{index}/resource"),
        Some(id),
        None,
        None,
    )
    .to_krpc_error()
}

impl RuntimeInferencePort {
    pub(super) async fn prepare_video_materials(
        &self,
        caller: &AuthorizedCaller,
        request_id: &str,
        selected_exact_model: &str,
        request: &VideoGenerateRequest,
    ) -> Result<PreparedVideoRequest, RPCErrors> {
        let mut prepared = request.clone();
        let mut facts = VideoMaterialFacts::new();
        let context = ResourceAccessContext::new(
            caller.tenant_id.clone(),
            caller.user_id.clone(),
            request_id.to_string(),
        )
        .map_err(resource_rpc_error)?;
        let manager = ResourceManager::new(
            Arc::new(AuthenticatedResourceAuthorizer {
                tenant_id: caller.tenant_id.clone(),
                caller_id: caller.user_id.clone(),
                storage: self.storage.clone(),
            }),
            self.resource_store.clone(),
            self.url_fetcher.clone(),
            ResourceLimits::default(),
        )
        .map_err(resource_rpc_error)?
        .with_input_form(ResourceInputForm::BytesOnly);
        for (index, condition) in prepared.conditions.iter_mut().enumerate() {
            let kind = condition.kind();
            let id = condition.id().to_string();
            if !needs_bytes(request, kind, condition.resource()) {
                continue;
            }
            let source = condition.resource().clone();
            let inspected = manager
                .inspect(&context, std::slice::from_ref(&source))
                .await
                .map_err(resource_rpc_error)?;
            let mut materialized = manager
                .materialize_after_provider_selected(&context, selected_exact_model, inspected)
                .await
                .map_err(resource_rpc_error)?;
            let resource = materialized.pop().ok_or_else(|| {
                condition_error(
                    AiccErrorCode::ResourceInvalid,
                    "resource could not be read",
                    index,
                    &id,
                )
            })?;
            let bytes = resource.bytes().to_vec();
            let measured = match probe(&bytes) {
                Ok(measured) => measured,
                // Image facts only refine checks; unreadable clips cannot be checked at all.
                Err(_)
                    if matches!(
                        kind,
                        VideoConditionKind::ReferenceImage | VideoConditionKind::Keyframe
                    ) && request
                        .output
                        .as_ref()
                        .is_none_or(|output| output.aspect_ratio.is_none()) =>
                {
                    continue;
                }
                Err(error) => {
                    return Err(condition_error(
                        AiccErrorCode::ResourceInvalid,
                        format!("media facts could not be read: {error}"),
                        index,
                        &id,
                    ));
                }
            };
            if !kind_accepts(kind, &measured) {
                return Err(condition_error(
                    AiccErrorCode::ResourceInvalid,
                    format!(
                        "{} condition received {} content",
                        kind.as_str(),
                        measured.container.mime()
                    ),
                    index,
                    &id,
                ));
            }
            let source_digest = format!("sha256:{:x}", Sha256::digest(&bytes));
            let mut transform = None;
            let mut output = bytes;
            let mut output_facts = measured.clone();
            if kind == VideoConditionKind::ReferenceVideo
                && !condition.include_audio()
                && measured.has_audio
            {
                output = strip_audio_tracks(&output).map_err(|error| {
                    condition_error(
                        AiccErrorCode::ResourceInvalid,
                        format!(
                            "the reference video's audio cannot be removed ({error}); supply a video without audio or set include_audio=true"
                        ),
                        index,
                        &id,
                    )
                })?;
                output_facts = probe(&output).map_err(|error| {
                    condition_error(
                        AiccErrorCode::InternalError,
                        format!("the audio-free video is unreadable: {error}"),
                        index,
                        &id,
                    )
                })?;
                transform = Some(MaterialTransform::StripAudio);
            } else if matches!(source, ResourceRef::Url { .. }) {
                transform = Some(MaterialTransform::Copy);
            }
            let derived = match transform {
                Some(transform) => {
                    let mime = output_facts.container.mime();
                    let extension = match output_facts.container {
                        MediaContainer::Mp4 => "mp4",
                        MediaContainer::Mov => "mov",
                        MediaContainer::Wav => "wav",
                        MediaContainer::Mp3 => "mp3",
                        MediaContainer::Png => "png",
                        MediaContainer::Jpeg => "jpg",
                        MediaContainer::Webp => "webp",
                        MediaContainer::Gif => "gif",
                    };
                    let artifact = manager
                        .write_artifact(
                            &context,
                            &output,
                            ArtifactSpec {
                                name: format!("video-condition-{id}.{extension}"),
                                mime: mime.to_string(),
                                attributes: Map::from_iter([
                                    ("source_digest".to_string(), json!(source_digest)),
                                    ("condition_id".to_string(), json!(id)),
                                    ("transform".to_string(), json!(transform)),
                                ]),
                                embedding: None,
                            },
                        )
                        .await
                        .map_err(resource_rpc_error)?;
                    let ResourceRef::NamedObject { obj_id } = &artifact.resource else {
                        return Err(inference_error(
                            AiccErrorCode::InternalError,
                            "prepared material did not produce a Named Object",
                        ));
                    };
                    self.storage
                        .remember_artifact_scope(
                            &obj_id.to_string(),
                            &caller.tenant_id,
                            &caller.user_id,
                            caller.app_id.as_deref(),
                            now_ms() as i64,
                        )
                        .await
                        .map_err(|error| {
                            inference_error(AiccErrorCode::InternalError, error.to_string())
                        })?;
                    log::info!(
                        "video_material_prepared: condition_id={id} transform={transform:?} source_digest={source_digest} obj_id={obj_id}"
                    );
                    *condition.resource_mut() = artifact.resource.clone();
                    Some(DerivedMaterial {
                        transform,
                        obj_id: obj_id.to_string(),
                        source_digest,
                    })
                }
                None => None,
            };
            facts.insert(
                id,
                ConditionFacts {
                    media: Some(output_facts),
                    derived,
                },
            );
        }
        Ok(PreparedVideoRequest {
            request: prepared,
            facts,
        })
    }
}

fn resource_rpc_error(error: ResourceError) -> RPCErrors {
    error.to_aicc_error().to_krpc_error()
}
