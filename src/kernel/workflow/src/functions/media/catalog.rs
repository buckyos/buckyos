use super::{function, BUILD_CATALOG};
use crate::native::{param_str, ExecutionLane, NativeContext, NativeFunction, NativeOutcome};
use async_trait::async_trait;
use buckyos_api::{
    FunctionObject, MediaEntry, MediaEntryError, MediaEntryKind, MediaEntryStatus, MediaManifest,
    MediaSourceRef, MediaWhere, ThunkExecutionError, MEDIA_MANIFEST_OBJ_TYPE,
};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub fn function_object() -> FunctionObject {
    function(
        BUILD_CATALOG,
        true,
        &[],
        json!({
            "type": "object",
            "required": ["family", "source_content_id", "probe"],
            "additionalProperties": false,
            "properties": {
                "family": {"type": "string", "enum": ["image", "pdf", "video", "document"]},
                "source_content_id": {"type": "string"},
                "derived_content_id": {"type": ["string", "null"]},
                "probe": {"type": "object"},
                "text": {"type": "object"},
                "images": {"type": "object"},
                "frames": {"type": "object"},
                "audio": {"type": "object"}
            }
        }),
        "media.catalog",
        json!({
            "type": "object",
            "required": ["manifest_obj_id", "summary", "complete"],
            "properties": {
                "manifest_obj_id": {"type": "string"},
                "summary": {"type": "object"},
                "complete": {"type": "boolean"}
            }
        }),
    )
}

pub struct BuildCatalog;

fn entry_error(value: Option<&Value>) -> Option<MediaEntryError> {
    let value = value?;
    Some(MediaEntryError {
        code: value.get("code").and_then(Value::as_str).unwrap_or("unknown").to_string(),
        message: value.get("message").and_then(Value::as_str).unwrap_or("").to_string(),
        retryable: value.get("retryable").and_then(Value::as_bool).unwrap_or(false),
    })
}

fn obj_id(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Builds the page entries of a document family from the text and image
/// step results: one entry per page in `1..=total`, merging both sources.
pub fn page_entries(text: Option<&Value>, images: Option<&Value>, source: MediaSourceRef) -> (Option<u64>, Vec<u64>, Vec<MediaEntry>) {
    #[derive(Default)]
    struct Page {
        text_obj_id: Option<String>,
        image_obj_id: Option<String>,
        no_text: bool,
        requested: bool,
        error: Option<MediaEntryError>,
    }
    let mut pages: BTreeMap<u64, Page> = BTreeMap::new();
    let mut total = None;
    for (result, is_text) in [(text, true), (images, false)] {
        let Some(result) = result else { continue };
        if let Some(t) = result.get("total_pages").and_then(Value::as_u64) {
            total = Some(total.map_or(t, |cur: u64| cur.max(t)));
        }
        for entry in result.get("pages").and_then(Value::as_array).into_iter().flatten() {
            let Some(page) = entry.get("page").and_then(Value::as_u64) else { continue };
            let slot = pages.entry(page).or_default();
            slot.requested = true;
            match entry.get("status").and_then(Value::as_str).unwrap_or("") {
                "ok" => {
                    if is_text {
                        slot.text_obj_id = obj_id(entry, "text_obj_id");
                    } else {
                        slot.image_obj_id = obj_id(entry, "image_obj_id");
                    }
                }
                "no_text" => slot.no_text = true,
                "failed" => {
                    let err = entry_error(entry.get("error"));
                    if slot.error.as_ref().map(|e| !e.retryable).unwrap_or(true) {
                        slot.error = err.or(slot.error.take());
                    }
                }
                _ => {}
            }
        }
    }
    let requested: Vec<u64> = pages.iter().filter(|(_, p)| p.requested).map(|(n, _)| *n).collect();
    let mut entries = Vec::new();
    let last = total.unwrap_or_else(|| pages.keys().next_back().copied().unwrap_or(0));
    for page in 1..=last {
        let slot = pages.get(&page);
        let (status, error) = match slot {
            None => (MediaEntryStatus::Unprocessed, None),
            Some(p) if p.error.is_some() && p.text_obj_id.is_none() && p.image_obj_id.is_none() => (MediaEntryStatus::Failed, p.error.clone()),
            Some(p) if p.error.is_some() => (MediaEntryStatus::Failed, p.error.clone()),
            Some(p) if p.text_obj_id.is_none() && p.no_text => (MediaEntryStatus::NoText, None),
            Some(_) => (MediaEntryStatus::Ok, None),
        };
        entries.push(MediaEntry {
            kind: MediaEntryKind::Page,
            location: MediaWhere { source, page: Some(page as u32), time_ms: None, index: None },
            text_obj_id: slot.and_then(|p| p.text_obj_id.clone()),
            image_obj_id: slot.and_then(|p| p.image_obj_id.clone()),
            audio_obj_id: None,
            status,
            error,
        });
    }
    (total, requested, entries)
}

pub fn video_entries(frames: Option<&Value>, audio: Option<&Value>) -> (Option<u64>, Vec<u64>, Vec<MediaEntry>) {
    let mut entries = Vec::new();
    let mut requested = Vec::new();
    let mut total = None;
    if let Some(frames) = frames {
        total = frames.get("duration_ms").and_then(Value::as_u64);
        for frame in frames.get("frames").and_then(Value::as_array).into_iter().flatten() {
            let time_ms = frame.get("time_ms").and_then(Value::as_u64).unwrap_or(0);
            let index = frame.get("index").and_then(Value::as_u64).map(|i| i as u32);
            requested.push(time_ms);
            let failed = frame.get("status").and_then(Value::as_str) == Some("failed");
            entries.push(MediaEntry {
                kind: MediaEntryKind::Frame,
                location: MediaWhere { source: MediaSourceRef::Original, page: None, time_ms: Some(time_ms), index },
                text_obj_id: None,
                image_obj_id: obj_id(frame, "image_obj_id"),
                audio_obj_id: None,
                status: if failed { MediaEntryStatus::Failed } else { MediaEntryStatus::Ok },
                error: if failed { entry_error(frame.get("error")) } else { None },
            });
        }
    }
    if let Some(audio) = audio {
        if total.is_none() {
            total = audio.get("duration_ms").and_then(Value::as_u64);
        }
        let absent = audio.get("status").and_then(Value::as_str) == Some("absent");
        entries.push(MediaEntry {
            kind: MediaEntryKind::Audio,
            location: MediaWhere { source: MediaSourceRef::Original, page: None, time_ms: Some(0), index: None },
            text_obj_id: None,
            image_obj_id: None,
            audio_obj_id: obj_id(audio, "audio_obj_id"),
            status: if absent { MediaEntryStatus::Absent } else { MediaEntryStatus::Ok },
            error: None,
        });
    }
    (total, requested, entries)
}

pub fn build_manifest(params: &Value) -> Result<MediaManifest, ThunkExecutionError> {
    let family = param_str(params, "family")?;
    let source = param_str(params, "source_content_id")?;
    let derived = params.get("derived_content_id").and_then(Value::as_str).filter(|s| !s.is_empty());
    let mut manifest = MediaManifest::new(family, source);
    manifest.derived_content_id = derived.map(str::to_string);
    manifest.probe = params.get("probe").cloned().unwrap_or(Value::Null);
    let (total, requested, entries) = match family {
        "video" => video_entries(params.get("frames"), params.get("audio")),
        "image" => (None, Vec::new(), Vec::new()),
        _ => page_entries(
            params.get("text"),
            params.get("images"),
            if derived.is_some() { MediaSourceRef::Derived } else { MediaSourceRef::Original },
        ),
    };
    manifest.range.total = total;
    manifest.range.requested = requested;
    manifest.entries = entries;
    manifest.recompute_coverage();
    Ok(manifest)
}

#[async_trait]
impl NativeFunction for BuildCatalog {
    fn lane(&self) -> ExecutionLane {
        ExecutionLane::Light
    }

    async fn invoke(&self, ctx: &NativeContext, params: &Value) -> Result<NativeOutcome, ThunkExecutionError> {
        let manifest = build_manifest(params)?;
        let value = serde_json::to_value(&manifest)
            .map_err(|err| ThunkExecutionError::new(buckyos_api::error_codes::INTERNAL, err.to_string()))?;
        let manifest_obj_id = ctx.io.put_object(MEDIA_MANIFEST_OBJ_TYPE, &value).await?;
        let mut refs = manifest.artifact_refs();
        refs.push(manifest_obj_id.clone());
        let outcome = NativeOutcome::new(json!({
            "manifest_obj_id": manifest_obj_id,
            "summary": manifest.summary(),
            "complete": manifest.complete,
        }))
        .with_artifacts(refs);
        Ok(if manifest.has_retryable_failure() { outcome.not_cacheable() } else { outcome })
    }
}
