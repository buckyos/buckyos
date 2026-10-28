//! `read`: read a file or a data object and return LLM-readable text.
//!
//! A superset of `read_file` (main TODO of the UI prompt, D3): a path
//! (relative to the working directory or absolute within it) or a
//! `file://` URI is read by `read_file`'s own implementation — same
//! working-directory policy, same `range` / `first_chunk` window, same
//! result — so the two never drift apart. On top of that:
//!
//! - an object id (`cyfile:…`, `chunk:…`: an attachment or a report
//!   artifact) is read from the object store, or a message attachment the
//!   store refuses through msg-center ([`ObjectReader`]), without being
//!   copied to disk first;
//! - `offset` / `limit` (0-based line paging) are accepted as another
//!   spelling of `range`.
//!
//! Other URI schemes are refused.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use ndn_lib::{FileObject, NdnError, ObjId};
use serde_json::{json, Value as Json};

use crate::file_tools::{FileToolConfig, ReadFileTool};
use crate::path_utils::expand_home;
use crate::runtime::files::{FileBackend, PathType};
use crate::tool::{CallingConventions, TypedToolHandle};
use crate::{AgentTool, AgentToolError, AgentToolResult, SessionRuntimeContext, ToolSpec};

pub use crate::TOOL_READ;

/// Content of data objects (`cyfile:…`) for `read`.
#[async_trait]
pub trait ObjectReader: Send + Sync + std::fmt::Debug {
    async fn read_object(&self, obj_id: &str) -> Result<Vec<u8>, AgentToolError>;
}

/// The zone's NamedStore (through the process's BuckyOS runtime).
#[derive(Debug, Default)]
pub struct NamedStoreObjectReader;

#[async_trait]
impl ObjectReader for NamedStoreObjectReader {
    async fn read_object(&self, obj_id: &str) -> Result<Vec<u8>, AgentToolError> {
        let id = ObjId::new(obj_id)
            .map_err(|e| AgentToolError::InvalidArgs(format!("bad object id {obj_id}: {e}")))?;
        read_named_object(&id)
            .await
            .map(|content| content.bytes)
            .map_err(AgentToolError::ExecFailed)
    }
}

/// A data object's content and the media type its FileObject or its server
/// gives, when either does.
pub struct ObjectContent {
    pub bytes: Vec<u8>,
    pub mime: Option<String>,
}

/// Reads `obj_id` with the process's identity: from the zone's NamedStore, or,
/// when the store refuses that identity (an attachment of a message, which
/// only msg-center reads for the members of its conversation), through
/// msg-center, which finds the message among the caller's own mailboxes.
pub async fn read_named_object(obj_id: &ObjId) -> Result<ObjectContent, String> {
    use tokio::io::AsyncReadExt;
    let runtime = buckyos_api::get_buckyos_api_runtime()
        .map_err(|e| format!("no object store here: {e}"))?;
    let store = runtime
        .get_named_store()
        .await
        .map_err(|e| format!("object store: {e}"))?;
    let mime = match store.get_object(obj_id).await {
        Ok(body) => file_object_mime(&body),
        Err(NdnError::PermissionDenied(_)) => return read_message_attachment(runtime, obj_id).await,
        Err(_) => None,
    };
    let (mut reader, _) = match store.open_reader(obj_id, None).await {
        Ok(opened) => opened,
        Err(NdnError::PermissionDenied(_)) => return read_message_attachment(runtime, obj_id).await,
        Err(e) => return Err(format!("open {obj_id}: {e}")),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| format!("read {obj_id}: {e}"))?;
    Ok(ObjectContent { bytes, mime })
}

fn file_object_mime(body: &str) -> Option<String> {
    let file: FileObject = serde_json::from_str(body).ok()?;
    file.meta
        .get("mime_type")
        .or_else(|| file.meta.get("mime"))
        .and_then(Json::as_str)
        .map(str::to_string)
}

/// `GET <msg-center>/objects/<obj_id>/content`.
async fn read_message_attachment(
    runtime: &buckyos_api::BuckyOSRuntime,
    obj_id: &ObjId,
) -> Result<ObjectContent, String> {
    let base = runtime
        .get_zone_service_url(buckyos_api::MSG_CENTER_SERVICE_NAME, runtime.force_https)
        .await
        .map_err(|e| format!("msg-center: {e}"))?;
    runtime
        .renew_token_from_verify_hub()
        .await
        .map_err(|e| format!("msg-center authentication: {e}"))?;
    let response = reqwest::Client::new()
        .get(format!("{base}/objects/{obj_id}/content"))
        .bearer_auth(runtime.get_session_token().await)
        .send()
        .await
        .map_err(|e| format!("msg-center: {e}"))?;
    let status = response.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Err(format!(
            "{obj_id} is not readable here: the object store refuses this identity, and no message it received references it"
        ));
    }
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("msg-center refused {obj_id}: HTTP {status}: {body}"));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read {obj_id} from msg-center: {e}"))?;
    Ok(ObjectContent { bytes: bytes.to_vec(), mime })
}

/// A file backend over one object: `read_file`'s window logic applied to
/// the object's content. Object ids are not paths: nothing is resolved
/// against the working directory.
#[derive(Debug)]
struct ObjectBackend {
    reader: Arc<dyn ObjectReader>,
}

#[async_trait]
impl FileBackend for ObjectBackend {
    async fn resolve(
        &self,
        _root: &Path,
        raw: &str,
        _allowed: &[PathBuf],
    ) -> Result<PathBuf, AgentToolError> {
        Ok(PathBuf::from(raw))
    }
    async fn exists(&self, _path: &Path) -> Result<bool, AgentToolError> {
        Ok(true)
    }
    async fn read(&self, path: &Path) -> Result<Vec<u8>, AgentToolError> {
        let id = path.to_string_lossy();
        let bytes = self.reader.read_object(&id).await?;
        let head = &bytes[..bytes.len().min(8192)];
        if head.contains(&0) || std::str::from_utf8(&bytes).is_err() {
            return Err(AgentToolError::InvalidArgs(format!(
                "{id} is not text; use llm_understand_media to look at media objects"
            )));
        }
        Ok(bytes)
    }
    async fn write(&self, path: &Path, _bytes: &[u8]) -> Result<(), AgentToolError> {
        Err(AgentToolError::InvalidArgs(format!(
            "{} is an object and cannot be written",
            path.display()
        )))
    }
    async fn path_type(&self, _path: &Path) -> Result<PathType, AgentToolError> {
        Ok(PathType::File)
    }
    async fn mkdir(&self, path: &Path) -> Result<(), AgentToolError> {
        Err(AgentToolError::InvalidArgs(format!(
            "{} is an object store path, not a directory",
            path.display()
        )))
    }
}

#[derive(Clone, Debug)]
pub struct ReadTool {
    cfg: FileToolConfig,
    objects: Arc<dyn ObjectReader>,
}

impl ReadTool {
    pub fn new(cfg: FileToolConfig) -> Self {
        Self {
            cfg,
            objects: Arc::new(NamedStoreObjectReader),
        }
    }

    pub fn with_objects(mut self, objects: Arc<dyn ObjectReader>) -> Self {
        self.objects = objects;
        self
    }
}

#[async_trait]
impl AgentTool for ReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_READ.to_string(),
            description: "Read a file or an object and return LLM-readable text. uri is a path (relative to the working directory or absolute within it), file:///abs/path, or an object id such as cyfile:… (an attachment, a report artifact). range selects 1-based lines: \"120-200\", \"120:$\" (to the end), \"120:+20\" (20 lines from 120), negative numbers count from the end (\"-50:$\" = the last 50); first_chunk starts the read at the first occurrence of that text (range then counts from there). offset (0-based) and limit page by lines instead of range.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "uri": {
                        "type": "string",
                        "description": "Path, file:// URI or object id (cyfile:…)."
                    },
                    "range": {
                        "type": "string",
                        "description": "1-based line range, e.g. \"120-200\", \"120:+20\", \"-50:$\"."
                    },
                    "first_chunk": {
                        "type": "string",
                        "description": "Start at the first occurrence of this text."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "0-based line to start at (instead of range)."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Lines to read from offset (instead of range)."
                    }
                },
                "required": ["uri"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "content": {"type": "string"},
                    "matched": {"type": "boolean"},
                    "line_range": {"type": "string"},
                    "bytes": {"type": "integer"},
                    "line_count": {"type": "integer"},
                    "start_line": {"type": ["integer", "null"]},
                    "end_line": {"type": ["integer", "null"]}
                }
            }),
            usage: Some(format!(
                "{TOOL_READ} uri=\"<path | file:///abs | cyfile:…>\" [range=\"120-200\"] [first_chunk=\"<text>\"]"
            )),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let map = match args {
            Json::Object(m) => m,
            Json::Null => serde_json::Map::new(),
            other => {
                return Err(AgentToolError::InvalidArgs(format!(
                    "read args must be a json object, got {other}"
                )))
            }
        };
        // `path` is read_file's name for it.
        let uri = map
            .get("uri")
            .or_else(|| map.get("path"))
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("`uri` is required".to_string()))?
            .to_string();
        let range = window_range(&map)?;
        let mut read_args = serde_json::Map::new();
        if let Some(r) = range {
            read_args.insert("range".into(), r);
        }
        if let Some(chunk) = map.get("first_chunk").filter(|v| !v.is_null()) {
            read_args.insert("first_chunk".into(), chunk.clone());
        }
        let (cfg, path) = match parse_read_target(&uri)? {
            ReadTarget::File { path } => (self.cfg.clone(), path),
            ReadTarget::Object { obj_id } => {
                let mut cfg = self.cfg.clone();
                cfg.backend = Arc::new(ObjectBackend {
                    reader: self.objects.clone(),
                });
                cfg.allowed_read_roots.clear();
                (cfg, obj_id)
            }
        };
        read_args.insert("path".into(), Json::String(path));
        TypedToolHandle::with_null_host(ReadFileTool::new(cfg))
            .call(ctx, Json::Object(read_args))
            .await
    }
}

/// `range` as given, or the `offset` / `limit` page spelled as a range.
fn window_range(map: &serde_json::Map<String, Json>) -> Result<Option<Json>, AgentToolError> {
    let offset = parse_u64_arg(map.get("offset"), "offset")?;
    let limit = parse_u64_arg(map.get("limit"), "limit")?;
    let range = map.get("range").filter(|v| !v.is_null()).cloned();
    if range.is_some() && (offset.is_some() || limit.is_some()) {
        return Err(AgentToolError::InvalidArgs(
            "give either range or offset/limit, not both".to_string(),
        ));
    }
    if let Some(0) = limit {
        return Err(AgentToolError::InvalidArgs("`limit` must be at least 1".to_string()));
    }
    Ok(match (range, offset, limit) {
        (Some(r), _, _) => Some(r),
        (None, None, None) => None,
        (None, offset, Some(limit)) => {
            let start = offset.unwrap_or(0) + 1;
            Some(Json::String(format!("{start}-{}", start + limit - 1)))
        }
        (None, Some(offset), None) => Some(Json::String(format!("{}:$", offset + 1))),
    })
}

/// Pull a non-negative integer out of args. Accepts either a JSON number or
/// a string-of-digits (the XML parser supplies attribute values as strings).
fn parse_u64_arg(v: Option<&Json>, name: &str) -> Result<Option<u64>, AgentToolError> {
    let Some(v) = v else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    if let Some(n) = v.as_u64() {
        return Ok(Some(n));
    }
    if let Some(s) = v.as_str() {
        let s = s.trim();
        if s.is_empty() {
            return Ok(None);
        }
        return s
            .parse::<u64>()
            .map(Some)
            .map_err(|e| AgentToolError::InvalidArgs(format!("`{name}` not a u64: {e}")));
    }
    Err(AgentToolError::InvalidArgs(format!(
        "`{name}` must be a non-negative integer (got {v})"
    )))
}

#[derive(Debug, PartialEq, Eq)]
enum ReadTarget {
    File { path: String },
    Object { obj_id: String },
}

/// An object id has a type prefix, no path separator and no whitespace
/// (`cyfile:…`, `chunk:…`).
fn object_id(raw: &str) -> Option<String> {
    if raw.contains(['/', '\\']) || raw.chars().any(char::is_whitespace) || !raw.contains(':') {
        return None;
    }
    ObjId::new(raw).ok().map(|id| id.to_string())
}

fn parse_read_target(raw: &str) -> Result<ReadTarget, AgentToolError> {
    if let Some(obj_id) = object_id(raw) {
        return Ok(ReadTarget::Object { obj_id });
    }
    if let Some((scheme, _)) = raw.split_once("://") {
        let scheme = scheme.trim();
        if scheme == "file" {
            return Ok(ReadTarget::File {
                path: parse_file_uri_path(raw)?,
            });
        }
        return Err(AgentToolError::InvalidArgs(format!(
            "unsupported read uri `{raw}`: use a path, file:///abs/path or an object id (cyfile:…)"
        )));
    }
    Ok(ReadTarget::File {
        path: expand_home(raw),
    })
}

/// Parse `file://[host]/abs/path` → `/abs/path`. The host portion is
/// accepted but ignored (`localhost` and empty both valid, others rejected
/// to surface typos).
fn parse_file_uri_path(uri: &str) -> Result<String, AgentToolError> {
    let rest = uri
        .strip_prefix("file://")
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("malformed file uri: {uri}")))?;
    if let Some(stripped) = rest.strip_prefix("localhost/") {
        return Ok(format!("/{stripped}"));
    }
    if rest.starts_with('/') {
        return Ok(rest.to_string());
    }
    // A host other than localhost is a typo (`file://workspace/x`), not a
    // host named `workspace`.
    Err(AgentToolError::InvalidArgs(format!(
        "file:// uri must be of the form `file:///abs/path` or `file://localhost/abs/path`; got `{uri}`"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use tempfile::tempdir;

    use crate::file_tools::ReadFileTool;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            tool_call_index: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    fn ws_with_file(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let ws = dir.path().join("workspace");
        fs::create_dir_all(&ws).unwrap();
        fs::write(ws.join(name), content).unwrap();
        (dir, ws)
    }

    const TEXT: &str = "alpha\nbeta\ngamma\ndelta\nepsilon\n";

    async fn read_file(ws: &Path, args: Json) -> AgentToolResult {
        TypedToolHandle::with_null_host(ReadFileTool::new(FileToolConfig::new(ws.to_path_buf())))
            .call(&ctx(), args)
            .await
            .unwrap()
    }

    async fn read(ws: &Path, args: Json) -> Result<AgentToolResult, AgentToolError> {
        ReadTool::new(FileToolConfig::new(ws.to_path_buf()))
            .call(&ctx(), args)
            .await
    }

    fn same(a: &AgentToolResult, b: &AgentToolResult) {
        assert_eq!(
            serde_json::to_string(a).unwrap(),
            serde_json::to_string(b).unwrap()
        );
    }

    #[tokio::test]
    async fn a_file_reads_exactly_like_read_file() {
        let (_d, ws) = ws_with_file("notes.txt", TEXT);
        let abs = ws.join("notes.txt").canonicalize().unwrap();
        for (read_args, file_args) in [
            (json!({"uri": "notes.txt"}), json!({"path": "notes.txt"})),
            (
                json!({"uri": format!("file://{}", abs.display())}),
                json!({"path": abs.display().to_string()}),
            ),
            (
                json!({"uri": "notes.txt", "range": "2-3"}),
                json!({"path": "notes.txt", "range": "2-3"}),
            ),
            (
                json!({"uri": "notes.txt", "first_chunk": "gamma", "range": "1-2"}),
                json!({"path": "notes.txt", "first_chunk": "gamma", "range": "1-2"}),
            ),
            (json!({"path": "notes.txt", "range": "-2:$"}), json!({"path": "notes.txt", "range": "-2:$"})),
        ] {
            same(&read(&ws, read_args).await.unwrap(), &read_file(&ws, file_args).await);
        }
    }

    #[tokio::test]
    async fn offset_and_limit_are_another_spelling_of_range() {
        let (_d, ws) = ws_with_file("notes.txt", TEXT);
        same(
            &read(&ws, json!({"uri": "notes.txt", "offset": 1, "limit": 2})).await.unwrap(),
            &read_file(&ws, json!({"path": "notes.txt", "range": "2-3"})).await,
        );
        same(
            &read(&ws, json!({"uri": "notes.txt", "offset": "3"})).await.unwrap(),
            &read_file(&ws, json!({"path": "notes.txt", "range": "4:$"})).await,
        );
        assert!(read(&ws, json!({"uri": "notes.txt", "offset": 1, "range": "1-2"}))
            .await
            .is_err());
        assert!(read(&ws, json!({"uri": "notes.txt", "limit": 0})).await.is_err());
    }

    #[tokio::test]
    async fn the_working_directory_policy_is_read_files() {
        let (dir, ws) = ws_with_file("notes.txt", TEXT);
        fs::write(dir.path().join("outside.txt"), "secret").unwrap();
        for uri in ["../outside.txt", &dir.path().join("outside.txt").display().to_string()] {
            assert!(
                matches!(read(&ws, json!({"uri": uri})).await, Err(AgentToolError::InvalidArgs(_))),
                "{uri}"
            );
        }
        assert!(read(&ws, json!({})).await.is_err());
        assert!(read(&ws, json!({"uri": "http://example.com/x"})).await.is_err());
        assert!(read(&ws, json!({"uri": "file://workspace/notes.txt"})).await.is_err());
    }

    #[derive(Debug)]
    struct Objects;

    #[async_trait]
    impl ObjectReader for Objects {
        async fn read_object(&self, obj_id: &str) -> Result<Vec<u8>, AgentToolError> {
            if obj_id.ends_with(&"bb".repeat(32)) {
                return Ok(vec![0x89, b'P', b'N', b'G', 0, 1, 2]);
            }
            Ok(TEXT.as_bytes().to_vec())
        }
    }

    #[tokio::test]
    async fn an_object_is_read_from_the_object_store() {
        let (_d, ws) = ws_with_file("notes.txt", TEXT);
        let tool = ReadTool::new(FileToolConfig::new(ws.clone())).with_objects(Arc::new(Objects));
        let id = format!("cyfile:{}", "aa".repeat(32));
        let r = tool
            .call(&ctx(), json!({"uri": id, "range": "2-3"}))
            .await
            .unwrap();
        assert_eq!(r.details["content"], json!("beta\ngamma\n"));
        let png = format!("cyfile:{}", "bb".repeat(32));
        let err = tool.call(&ctx(), json!({"uri": png})).await.unwrap_err();
        assert!(err.to_string().contains("llm_understand_media"), "{err}");
    }

    #[test]
    fn targets_are_told_apart() {
        assert_eq!(
            parse_read_target("notes/a.txt").unwrap(),
            ReadTarget::File { path: "notes/a.txt".into() }
        );
        assert_eq!(
            parse_read_target("file:///tmp/a").unwrap(),
            ReadTarget::File { path: "/tmp/a".into() }
        );
        assert_eq!(
            parse_read_target("file://localhost/tmp/a").unwrap(),
            ReadTarget::File { path: "/tmp/a".into() }
        );
        let id = format!("cyfile:{}", "aa".repeat(32));
        assert_eq!(
            parse_read_target(&id).unwrap(),
            ReadTarget::Object { obj_id: id.clone() }
        );
        // A file name with a colon is still a file.
        assert!(matches!(
            parse_read_target("notes:draft.txt").unwrap(),
            ReadTarget::File { .. }
        ));
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(
                parse_read_target("~/x").unwrap(),
                ReadTarget::File {
                    path: PathBuf::from(home).join("x").display().to_string()
                }
            );
        }
    }
}
