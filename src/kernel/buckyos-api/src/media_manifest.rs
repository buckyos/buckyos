//! Media representation manifest produced by the system `media.*` pipelines
//! (notepads/llm-media-pipeline-workflow-todo.md §3.1).
//!
//! The manifest is a pure, cacheable named object: it names content ids and
//! artifact object ids only. Credentials, expiry, run ids, workflow ids and
//! file names stay in the run output / delivery layer.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MEDIA_MANIFEST_SCHEMA_VERSION: u32 = 1;
pub const MEDIA_MANIFEST_OBJ_TYPE: &str = "mmanifest";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaEntryKind {
    Page,
    Frame,
    Audio,
    Image,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaEntryStatus {
    /// Representation produced.
    Ok,
    /// Page exists but carries no extractable text (scanned / blank page).
    NoText,
    /// The source has no such track (video without audio).
    Absent,
    /// Outside the requested range: present in the source, not processed.
    Unprocessed,
    /// Processing failed; `error` says why.
    Failed,
}

/// Which source the `page` / `time_ms` coordinates belong to.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaSourceRef {
    Original,
    Derived,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaWhere {
    pub source: MediaSourceRef,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaEntry {
    pub kind: MediaEntryKind,
    #[serde(rename = "where")]
    pub location: MediaWhere,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text_obj_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_obj_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio_obj_id: Option<String>,
    pub status: MediaEntryStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<MediaEntryError>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaEntryError {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub retryable: bool,
}

/// Pages or seconds: what the source has, what was asked for, what was done.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaRange {
    /// Total pages (documents) or duration in ms (video); `None` when unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    /// Requested page numbers (1-based) or frame timestamps in ms.
    #[serde(default)]
    pub requested: Vec<u64>,
    /// Successfully processed page numbers / timestamps.
    #[serde(default)]
    pub processed: Vec<u64>,
    /// Requested but failed page numbers / timestamps.
    #[serde(default)]
    pub failed: Vec<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaManifest {
    pub schema_version: u32,
    /// `image` / `pdf` / `video` / `document`.
    pub family: String,
    /// Content id (chunk / chunklist) of the caller's original object.
    pub source_content_id: String,
    /// Content id of the derived source (document -> PDF); coordinates of
    /// `Derived` entries refer to it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub derived_content_id: Option<String>,
    #[serde(default)]
    pub probe: Value,
    pub range: MediaRange,
    pub entries: Vec<MediaEntry>,
    /// `true` when every requested item has an `Ok` / `NoText` / `Absent` entry.
    pub complete: bool,
}

impl MediaManifest {
    pub fn new(family: &str, source_content_id: &str) -> Self {
        Self {
            schema_version: MEDIA_MANIFEST_SCHEMA_VERSION,
            family: family.to_string(),
            source_content_id: source_content_id.to_string(),
            derived_content_id: None,
            probe: Value::Null,
            range: MediaRange::default(),
            entries: Vec::new(),
            complete: true,
        }
    }

    pub fn recompute_coverage(&mut self) {
        self.range.processed = self
            .entries
            .iter()
            .filter(|e| {
                matches!(
                    e.status,
                    MediaEntryStatus::Ok | MediaEntryStatus::NoText | MediaEntryStatus::Absent
                )
            })
            .filter_map(|e| entry_coordinate(e))
            .collect();
        self.range.failed = self
            .entries
            .iter()
            .filter(|e| e.status == MediaEntryStatus::Failed)
            .filter_map(|e| entry_coordinate(e))
            .collect();
        self.range.processed.sort_unstable();
        self.range.processed.dedup();
        self.range.failed.sort_unstable();
        self.range.failed.dedup();
        self.complete = self
            .entries
            .iter()
            .all(|e| !matches!(e.status, MediaEntryStatus::Failed | MediaEntryStatus::Unprocessed))
            && self
                .range
                .requested
                .iter()
                .all(|r| self.range.processed.contains(r));
    }

    /// `true` when any failed entry is retryable: the result is deliverable
    /// but must not be written to a reusable cache.
    pub fn has_retryable_failure(&self) -> bool {
        self.entries
            .iter()
            .any(|e| e.error.as_ref().map(|err| err.retryable).unwrap_or(false))
    }

    pub fn artifact_refs(&self) -> Vec<String> {
        let mut refs = Vec::new();
        for entry in &self.entries {
            for id in [&entry.text_obj_id, &entry.image_obj_id, &entry.audio_obj_id]
                .into_iter()
                .flatten()
            {
                refs.push(id.clone());
            }
        }
        refs.sort();
        refs.dedup();
        refs
    }

    pub fn summary(&self) -> MediaManifestSummary {
        let mut summary = MediaManifestSummary {
            family: self.family.clone(),
            total: self.range.total,
            requested: self.range.requested.len() as u64,
            processed: self.range.processed.len() as u64,
            failed: self.range.failed.len() as u64,
            complete: self.complete,
            ..Default::default()
        };
        for entry in &self.entries {
            match entry.status {
                MediaEntryStatus::Ok => summary.ok += 1,
                MediaEntryStatus::NoText => summary.no_text += 1,
                MediaEntryStatus::Absent => summary.absent += 1,
                MediaEntryStatus::Unprocessed => summary.unprocessed += 1,
                MediaEntryStatus::Failed => {}
            }
            if entry.text_obj_id.is_some() {
                summary.text_entries += 1;
            }
            if entry.image_obj_id.is_some() {
                summary.image_entries += 1;
            }
            if entry.audio_obj_id.is_some() {
                summary.audio_entries += 1;
            }
        }
        summary
    }
}

fn entry_coordinate(entry: &MediaEntry) -> Option<u64> {
    entry
        .location
        .page
        .map(u64::from)
        .or(entry.location.time_ms)
        .or(entry.location.index.map(u64::from))
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaManifestSummary {
    pub family: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub total: Option<u64>,
    pub requested: u64,
    pub processed: u64,
    pub failed: u64,
    pub ok: u64,
    pub no_text: u64,
    pub absent: u64,
    pub unprocessed: u64,
    pub text_entries: u64,
    pub image_entries: u64,
    pub audio_entries: u64,
    pub complete: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(page: u32, status: MediaEntryStatus, retryable: bool) -> MediaEntry {
        MediaEntry {
            kind: MediaEntryKind::Page,
            location: MediaWhere { source: MediaSourceRef::Original, page: Some(page), time_ms: None, index: None },
            text_obj_id: (status == MediaEntryStatus::Ok).then(|| format!("cyfile:{page}")),
            image_obj_id: None,
            audio_obj_id: None,
            status,
            error: (status == MediaEntryStatus::Failed).then(|| MediaEntryError {
                code: "tool_failed".into(),
                message: "render failed".into(),
                retryable,
            }),
        }
    }

    #[test]
    fn coverage_and_cacheability() {
        let mut manifest = MediaManifest::new("pdf", "sha256:abc");
        manifest.range.total = Some(3);
        manifest.range.requested = vec![1, 2, 3];
        manifest.entries = vec![
            page(1, MediaEntryStatus::Ok, false),
            page(2, MediaEntryStatus::NoText, false),
            page(3, MediaEntryStatus::Failed, true),
        ];
        manifest.recompute_coverage();
        assert_eq!(manifest.range.processed, vec![1, 2]);
        assert_eq!(manifest.range.failed, vec![3]);
        assert!(!manifest.complete);
        assert!(manifest.has_retryable_failure());
        assert_eq!(manifest.artifact_refs(), vec!["cyfile:1".to_string()]);
        let summary = manifest.summary();
        assert_eq!((summary.ok, summary.no_text, summary.failed), (1, 1, 1));

        manifest.entries[2] = page(3, MediaEntryStatus::NoText, false);
        manifest.recompute_coverage();
        assert!(manifest.complete);
        assert!(!manifest.has_retryable_failure());
        let json = serde_json::to_value(&manifest).unwrap();
        assert_eq!(json["entries"][0]["where"]["page"], 1);
        let back: MediaManifest = serde_json::from_value(json).unwrap();
        assert_eq!(back, manifest);
    }
}
