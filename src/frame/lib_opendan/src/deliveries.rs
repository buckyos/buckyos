//! The delivery index (Agent UI upgrade TODO, AR1): which Session handed
//! which file to the owner, and when.
//!
//! `<agent_root>/state/deliveries.jsonl`, append only, one
//! [`DeliveryRecord`] per line in delivery order. A line is written when a
//! report's or a reply's artifact has become an object of the host's object
//! store; the same object delivered again by the same Session adds nothing.
//! The file is an index: the objects live in the Agent's `artifacts`
//! bucket, the Sessions in the registry. Lines are never rewritten; a line
//! that cannot be parsed is skipped by readers.

use std::path::{Path, PathBuf};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{OpenDanError, Result};
use crate::fsutil;

/// Entries of one page, at most.
pub const MAX_PAGE: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliverySource {
    /// An artifact of a report.
    Report,
    /// An attachment of a reply.
    Reply,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliveryRecord {
    pub at_ms: u64,
    /// The name the file was delivered under.
    pub name: String,
    pub obj_id: String,
    /// The `obj://bucket/...` binding that retains the object (absent: an
    /// object passed on by its id, kept by no bucket of this Agent).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    /// The Session that delivered it.
    pub session_id: String,
    /// The UI Session at the top of that Session's origin chain (itself,
    /// when a UI Session delivered).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ui_session_id: Option<String>,
    /// The conversation of that UI Session: the same through every
    /// generation of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route_key: Option<String>,
    pub source: DeliverySource,
}

/// `deliveries.recent`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DeliveryPage {
    /// Newest first.
    pub entries: Vec<DeliveryRecord>,
    /// Hand it back as `before` for the entries older than these; absent
    /// when there are none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_before: Option<u64>,
}

pub fn index_path(agent_root: &Path) -> PathBuf {
    agent_root.join("state").join("deliveries.jsonl")
}

/// A content type by the name's extension (`None`: not a common one).
pub fn mime_of(name: &str) -> Option<String> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    let mime = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "mp4" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "ogg" => "audio/ogg",
        "m4a" => "audio/mp4",
        "pdf" => "application/pdf",
        "zip" => "application/zip",
        "json" => "application/json",
        "txt" | "log" => "text/plain",
        "md" => "text/markdown",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "ppt" => "application/vnd.ms-powerpoint",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        _ => return None,
    };
    Some(mime.to_string())
}

fn read_all(agent_root: &Path) -> Result<Vec<DeliveryRecord>> {
    let path = index_path(agent_root);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(OpenDanError::io(&path, e)),
    };
    Ok(text
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect())
}

/// Append `record` unless its Session already delivered that object.
/// `true`: a line was written.
pub fn record(agent_root: &Path, record: &DeliveryRecord) -> Result<bool> {
    if read_all(agent_root)?
        .iter()
        .any(|r| r.obj_id == record.obj_id && r.session_id == record.session_id)
    {
        return Ok(false);
    }
    fsutil::append_batch(&index_path(agent_root), &fsutil::to_json_lines(&[record])?)?;
    Ok(true)
}

/// Deliveries newest first: at most `limit`, older than position `before`
/// (a `next_before` of an earlier page), of one conversation when
/// `route_key` is given.
pub fn recent(
    agent_root: &Path,
    limit: usize,
    before: Option<u64>,
    route_key: Option<&str>,
) -> Result<DeliveryPage> {
    let all = read_all(agent_root)?;
    let limit = limit.clamp(1, MAX_PAGE);
    let mut older = all
        .into_iter()
        .enumerate()
        .rev()
        .filter(|(i, _)| before.is_none_or(|b| (*i as u64) < b))
        .filter(|(_, r)| route_key.is_none_or(|k| r.route_key.as_deref() == Some(k)));
    let page: Vec<(usize, DeliveryRecord)> = older.by_ref().take(limit).collect();
    let next_before = older.next().and(page.last()).map(|(i, _)| *i as u64);
    Ok(DeliveryPage {
        entries: page.into_iter().map(|(_, r)| r).collect(),
        next_before,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delivery(at_ms: u64, obj: &str, sid: &str, route: Option<&str>) -> DeliveryRecord {
        DeliveryRecord {
            at_ms,
            name: format!("{obj}.png"),
            obj_id: obj.to_string(),
            url: Some(format!("obj://bucket/b/{obj}")),
            mime: mime_of("a.png"),
            size: Some(3),
            session_id: sid.to_string(),
            ui_session_id: route.map(|_| "ui-1".to_string()),
            route_key: route.map(str::to_string),
            source: DeliverySource::Report,
        }
    }

    #[test]
    fn deliveries_are_appended_once_and_paged_newest_first() {
        let tmp = tempfile::TempDir::new().unwrap();
        let root = tmp.path();
        assert_eq!(recent(root, 10, None, None).unwrap(), DeliveryPage::default());
        for (i, route) in [Some("a"), Some("b"), Some("a"), None, Some("a")].into_iter().enumerate() {
            assert!(record(root, &delivery(i as u64, &format!("o{i}"), "work-1", route)).unwrap());
        }
        // The same object by the same session adds nothing; by another session it is another delivery.
        assert!(!record(root, &delivery(9, "o0", "work-1", Some("a"))).unwrap());
        assert!(record(root, &delivery(9, "o0", "work-2", Some("b"))).unwrap());

        let names = |p: &DeliveryPage| p.entries.iter().map(|e| (e.obj_id.clone(), e.session_id.clone())).collect::<Vec<_>>();
        let first = recent(root, 4, None, None).unwrap();
        assert_eq!(
            names(&first),
            [("o0", "work-2"), ("o4", "work-1"), ("o3", "work-1"), ("o2", "work-1")]
                .map(|(o, s)| (o.to_string(), s.to_string()))
        );
        let rest = recent(root, 4, first.next_before, None).unwrap();
        assert_eq!(names(&rest), [("o1", "work-1"), ("o0", "work-1")].map(|(o, s)| (o.to_string(), s.to_string())));
        assert_eq!(rest.next_before, None);

        let a = recent(root, 2, None, Some("a")).unwrap();
        assert_eq!(a.entries.iter().map(|e| e.obj_id.as_str()).collect::<Vec<_>>(), ["o4", "o2"]);
        let a = recent(root, 2, a.next_before, Some("a")).unwrap();
        assert_eq!(a.entries.iter().map(|e| e.obj_id.as_str()).collect::<Vec<_>>(), ["o0"]);
        assert_eq!(a.next_before, None);

        // A broken line is skipped, not an error.
        let path = index_path(root);
        let mut text = std::fs::read_to_string(&path).unwrap();
        text.push_str("{not json\n");
        std::fs::write(&path, text).unwrap();
        assert_eq!(recent(root, 100, None, None).unwrap().entries.len(), 6);
    }
}
