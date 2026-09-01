//! Local-filesystem passthrough helpers: safe relative paths, native ids,
//! metadata snapshots and content-type guessing.

use crate::error::{invalid, NfsError, NfsResult};
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

pub const MAX_NAME_BYTES: usize = 255;
pub const MAX_DEPTH: usize = 128;

/// Validates a single path segment (an entry name).
pub fn validate_name(name: &str) -> NfsResult<()> {
    if name.is_empty() || name == "." || name == ".." {
        return Err(invalid(format!("invalid name '{}'", name)));
    }
    if name.len() > MAX_NAME_BYTES {
        return Err(invalid("name too long"));
    }
    if name.bytes().any(|b| b == b'/' || b == 0) || name.contains('\\') {
        return Err(invalid(format!("name '{}' contains forbidden characters", name)));
    }
    Ok(())
}

/// Normalizes a client-supplied absolute-within-realm path ("/a/b/c" or "a/b/c")
/// into a clean relative path with validated segments. "" means the realm root.
pub fn normalize_rel(path: &str) -> NfsResult<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => continue,
            ".." => return Err(invalid("path may not contain '..'")),
            s => {
                validate_name(s)?;
                out.push(s);
            }
        }
    }
    if out.len() > MAX_DEPTH {
        return Err(invalid("path too deep"));
    }
    Ok(out.join("/"))
}

/// Joins a normalized relative path onto an export root.
pub fn join_root(root: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        root.to_path_buf()
    } else {
        let mut p = root.to_path_buf();
        for seg in rel.split('/') {
            p.push(seg);
        }
        p
    }
}

pub fn rel_join(rel: &str, name: &str) -> String {
    if rel.is_empty() {
        name.to_string()
    } else {
        format!("{}/{}", rel, name)
    }
}

/// Splits "a/b/c" into ("a/b", "c"); "" has no parent.
pub fn rel_split(rel: &str) -> Option<(String, String)> {
    if rel.is_empty() {
        return None;
    }
    match rel.rsplit_once('/') {
        Some((p, n)) => Some((p.to_string(), n.to_string())),
        None => Some((String::new(), rel.to_string())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FileId {
    pub ino: u64,
    pub dev: u64,
}

#[cfg(unix)]
pub fn file_id(meta: &Metadata) -> FileId {
    use std::os::unix::fs::MetadataExt;
    FileId { ino: meta.ino(), dev: meta.dev() }
}

#[cfg(not(unix))]
pub fn file_id(_meta: &Metadata) -> FileId {
    // Degraded on non-unix: handles cannot validate the native id (documented).
    FileId { ino: 0, dev: 0 }
}

#[derive(Debug, Clone)]
pub struct NodeMeta {
    pub kind: &'static str, // "file" | "dir" | "symlink"
    pub size: u64,
    pub mtime: i64,
    pub ctime: i64,
    pub id: FileId,
}

pub fn node_meta(meta: &Metadata) -> NodeMeta {
    let kind = if meta.is_dir() {
        "dir"
    } else if meta.file_type().is_symlink() {
        "symlink"
    } else {
        "file"
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let ctime = meta
        .created()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(mtime);
    NodeMeta { kind, size: meta.len(), mtime, ctime, id: file_id(meta) }
}

/// lstat: does not follow a final symlink (list semantics).
pub fn lstat(path: &Path) -> NfsResult<NodeMeta> {
    Ok(node_meta(&std::fs::symlink_metadata(path)?))
}

/// stat: follows symlinks (resolve default per NFSP D2).
pub fn stat_follow(path: &Path) -> NfsResult<NodeMeta> {
    Ok(node_meta(&std::fs::metadata(path)?))
}

pub fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Minimal extension → content-type map for the data plane.
pub fn content_type_for(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("txt") | Some("md") | Some("log") => "text/plain; charset=utf-8",
        Some("html") | Some("htm") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        Some("js") | Some("mjs") => "text/javascript",
        Some("json") => "application/json",
        Some("xml") => "application/xml",
        Some("pdf") => "application/pdf",
        Some("zip") => "application/zip",
        Some("png") => "image/png",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("gif") => "image/gif",
        Some("webp") => "image/webp",
        Some("svg") => "image/svg+xml",
        Some("heic") => "image/heic",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
        Some("mov") => "video/quicktime",
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("wav") => "audio/wav",
        _ => "application/octet-stream",
    }
}

/// Simple glob matcher supporting `*` and `?` (for list name_glob filters).
pub fn glob_match(pattern: &str, name: &str) -> bool {
    fn inner(p: &[u8], n: &[u8]) -> bool {
        if p.is_empty() {
            return n.is_empty();
        }
        match p[0] {
            b'*' => {
                // Collapse consecutive '*'
                let rest = &p[1..];
                (0..=n.len()).any(|i| inner(rest, &n[i..]))
            }
            b'?' => !n.is_empty() && inner(&p[1..], &n[1..]),
            c => !n.is_empty() && n[0] == c && inner(&p[1..], &n[1..]),
        }
    }
    inner(pattern.as_bytes(), name.as_bytes())
}

impl NfsError {
    /// Attaches path context to IO-derived errors (helper).
    pub fn ctx(mut self, what: &str) -> NfsError {
        self.message = format!("{}: {}", what, self.message);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_rejects_escape() {
        assert_eq!(normalize_rel("/a/b/").unwrap(), "a/b");
        assert_eq!(normalize_rel("a//b/./c").unwrap(), "a/b/c");
        assert_eq!(normalize_rel("/").unwrap(), "");
        assert!(normalize_rel("../x").is_err());
        assert!(normalize_rel("a/../../b").is_err());
        assert!(normalize_rel("a/b\\c").is_err());
    }

    #[test]
    fn split_and_join() {
        assert_eq!(rel_split("a/b/c"), Some(("a/b".into(), "c".into())));
        assert_eq!(rel_split("a"), Some(("".into(), "a".into())));
        assert_eq!(rel_split(""), None);
        assert_eq!(rel_join("", "x"), "x");
        assert_eq!(rel_join("a", "x"), "a/x");
    }

    #[test]
    fn globs() {
        assert!(glob_match("*.jpg", "a.jpg"));
        assert!(glob_match("a?c", "abc"));
        assert!(!glob_match("*.jpg", "a.png"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("**", "x"));
        assert!(glob_match("", ""));
    }

    #[test]
    fn content_types() {
        assert_eq!(content_type_for("x.PNG"), "image/png");
        assert_eq!(content_type_for("noext"), "application/octet-stream");
    }
}
