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
    if name.starts_with(".nfs_import_") || name.starts_with(".nfs_tmp_") {
        return Err(invalid("reserved staging name"));
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
    #[cfg(windows)]
    if root.as_os_str().is_empty() {
        return windows_drive_path(rel).map(PathBuf::from).unwrap_or_default();
    }
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

pub fn is_drive_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

pub fn windows_drive_path(rel: &str) -> Option<String> {
    let mut parts = rel.split('/').filter(|s| !s.is_empty());
    let drive = parts.next().filter(|d| is_drive_name(d))?;
    let rest: Vec<&str> = parts.collect();
    Some(format!("{}\\{}", drive.to_ascii_uppercase(), rest.join("\\")))
}

pub fn host_join(root: &Path, rel: &str) -> Option<PathBuf> {
    if root.as_os_str().is_empty() {
        return if rel.is_empty() { Some(PathBuf::new()) } else { windows_drive_path(rel).map(PathBuf::from) };
    }
    Some(join_root(root, rel))
}

pub fn windows_drives() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    (b'A'..=b'Z')
        .map(|d| format!("{}:", d as char))
        .filter(|d| Path::new(&format!("{}\\", d)).is_dir())
        .collect()
}

pub fn file_uri(rel: &str) -> String {
    const SET: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ').add(b'"').add(b'#').add(b'%').add(b'<').add(b'>').add(b'?')
        .add(b'[').add(b'\\').add(b']').add(b'^').add(b'`').add(b'{').add(b'|').add(b'}').add(b'~');
    let segs: Vec<String> = rel
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| percent_encoding::utf8_percent_encode(s, SET).to_string())
        .collect();
    let mut out = format!("file:///{}", segs.join("/"));
    if segs.len() == 1 && is_drive_name(&segs[0]) {
        out.push('/');
    }
    out
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
    pub birth: Option<std::time::SystemTime>,
    pub link: bool,
    pub special: bool,
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
    let ft = meta.file_type();
    let special = !ft.is_dir() && !ft.is_file() && !ft.is_symlink();
    NodeMeta { kind, size: meta.len(), mtime, ctime, id: file_id(meta), birth: meta.created().ok(), link: false, special }
}

/// lstat: does not follow a final symlink (list semantics).
pub fn lstat(path: &Path) -> NfsResult<NodeMeta> {
    Ok(node_meta(&std::fs::symlink_metadata(path)?))
}

pub fn host_meta(path: &Path) -> NfsResult<NodeMeta> {
    if path.as_os_str().is_empty() {
        return Ok(NodeMeta { kind: "dir", size: 0, mtime: 0, ctime: 0, id: FileId { ino: 0, dev: 0 }, birth: None, link: false, special: false });
    }
    let own = std::fs::symlink_metadata(path)?;
    if !own.file_type().is_symlink() {
        return Ok(node_meta(&own));
    }
    match std::fs::metadata(path) {
        Ok(target) => Ok(NodeMeta { link: true, ..node_meta(&target) }),
        Err(_) => Ok(node_meta(&own)),
    }
}

#[cfg(unix)]
pub fn os_readable(path: &Path, dir: bool) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    let mode = if dir { libc::R_OK | libc::X_OK } else { libc::R_OK };
    unsafe { libc::access(c.as_ptr(), mode) == 0 }
}

#[cfg(not(unix))]
pub fn os_readable(_path: &Path, _dir: bool) -> bool {
    true
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
        Some("heif") => "image/heif",
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
    fn host_paths() {
        assert_eq!(windows_drive_path("C:/Users/alice").as_deref(), Some("C:\\Users\\alice"));
        assert_eq!(windows_drive_path("d:").as_deref(), Some("D:\\"));
        assert_eq!(windows_drive_path(""), None);
        assert_eq!(windows_drive_path("Users/alice"), None);
        assert_eq!(file_uri(""), "file:///");
        assert_eq!(file_uri("Users/alice/My Doc.txt"), "file:///Users/alice/My%20Doc.txt");
        assert_eq!(file_uri("C:/Users/alice/a.txt"), "file:///C:/Users/alice/a.txt");
        assert_eq!(file_uri("C:"), "file:///C:/");
        assert_eq!(file_uri("a/中文/x#y?z%w.txt"), "file:///a/%E4%B8%AD%E6%96%87/x%23y%3Fz%25w.txt");
        assert_eq!(file_uri("a/^`{}|[]<>\"';=&+$,!~*()@:"), "file:///a/%5E%60%7B%7D%7C%5B%5D%3C%3E%22';=&+$,!%7E*()@:");
        assert_eq!(file_uri("tmp/\u{7f}\u{1}é"), "file:///tmp/%7F%01%C3%A9");
        assert!(normalize_rel("/Users/../etc").is_err());
        assert!(normalize_rel("C:/..").is_err());
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
