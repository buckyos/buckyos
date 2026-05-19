//! v2 `read` Action — protocol-aware reader.
//!
//! The v2 Behavior protocol (see `doc/opendan/Agent Actions.md` §1.4) uses
//! `<read uri="..."/>` as a placeholder for a generic "read by URI scheme"
//! action. v1 first cut implements filesystem reads: an explicit `file://`
//! URI is accepted, and a target without a `://` protocol header is treated
//! as a file path. Other schemes return `InvalidArgs`. New schemes (`kv://`,
//! `event://`, `http://`, `mcp://...`) get added by extending the dispatch
//! table in [`ReadTool::call`].
//!
//! Compared with the legacy CLI-oriented `read_file`, this tool is
//! byte-oriented and intentionally does *not* truncate within the requested
//! window — its existence reason is "bypass the `exec_bash` `max_output_bytes`
//! clipping when the model genuinely needs to read a large file in chunks."

use std::io::{Read, Seek, SeekFrom};

use async_trait::async_trait;
use serde_json::{json, Value as Json};

use crate::file_tools::FileToolConfig;
use crate::path_utils::{resolve_path_under_root, to_abs_path};
use crate::tool::CallingConventions;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult, AgentToolStatus,
    SessionRuntimeContext, ToolSpec,
};

pub const TOOL_READ: &str = "read";

/// Default `limit` when caller omits it. Picked to be slightly larger than
/// the typical exec_bash output clip (256 KiB) so the tool is useful as a
/// "give me the next chunk" loop, but bounded so a single call can't blow
/// the context.
const DEFAULT_LIMIT_BYTES: u64 = 64 * 1024;

/// Hard cap on a single read regardless of `limit`. 4 MiB is enough for any
/// realistic single-shot file slice; anything bigger should paginate.
const MAX_LIMIT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ReadTool {
    cfg: FileToolConfig,
}

impl ReadTool {
    pub fn new(cfg: FileToolConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl AgentTool for ReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_READ.to_string(),
            description:
                "Read everything by uri."
                    .to_string(),
            args_schema: json!({
                "properties": {
                    "uri": {
                        "type": "string",
                        "description": "Target to read. Bare paths default to file reads; "
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Byte offset to start reading at; defaults to 0."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Max bytes to read. Capped at 4 MiB; default 64 KiB."
                    }
                },
                "required": ["uri"]
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "uri": {"type": "string"},
                    "scheme": {"type": "string"},
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                    "offset": {"type": "integer"},
                    "bytes_read": {"type": "integer"},
                    "total_bytes": {"type": "integer"},
                    "eof": {"type": "boolean"}
                }
            }),
            usage: Some(format!(
                "{TOOL_READ} uri=\"<path-or-uri>\" [offset=<bytes>] [limit=<bytes>]"
            )),
        }
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
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

        let uri = map
            .get("uri")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("`uri` is required".to_string()))?
            .to_string();

        let offset = parse_u64_arg(map.get("offset"), "offset")?.unwrap_or(0);
        let limit = parse_u64_arg(map.get("limit"), "limit")?
            .unwrap_or(DEFAULT_LIMIT_BYTES)
            .clamp(1, MAX_LIMIT_BYTES);

        let target = parse_read_target(&uri)?;
        match target.scheme.as_str() {
            "file" => {
                self.read_file_path(&target.uri, &target.path, offset, limit)
                    .await
            }
            other => Err(AgentToolError::InvalidArgs(format!(
                "unsupported read scheme `{other}` (v1 first cut only supports `file`)"
            ))),
        }
    }
}

impl ReadTool {
    async fn read_file_path(
        &self,
        uri: &str,
        path_str: &str,
        offset: u64,
        limit: u64,
    ) -> Result<AgentToolResult, AgentToolError> {
        let workspace = to_abs_path(&self.cfg.root_dir)?;
        let resolved = resolve_path_under_root(&workspace, &path_str)?;
        if !resolved.exists() {
            return Err(AgentToolError::InvalidArgs(format!(
                "file not found: {}",
                resolved.display()
            )));
        }
        let metadata = std::fs::metadata(&resolved)
            .map_err(|e| AgentToolError::ExecFailed(format!("stat failed: {e}")))?;
        if !metadata.is_file() {
            return Err(AgentToolError::InvalidArgs(format!(
                "not a regular file: {}",
                resolved.display()
            )));
        }
        let total = metadata.len();
        let read_start = offset.min(total);
        let want = limit.min(total.saturating_sub(read_start));
        let bytes_to_read = usize::try_from(want).map_err(|_| {
            AgentToolError::InvalidArgs(format!(
                "requested read size exceeds usize on this platform: {want} bytes"
            ))
        })?;

        let mut buf = vec![0u8; bytes_to_read];
        if bytes_to_read > 0 {
            let mut file = std::fs::File::open(&resolved)
                .map_err(|e| AgentToolError::ExecFailed(format!("open failed: {e}")))?;
            file.seek(SeekFrom::Start(read_start))
                .map_err(|e| AgentToolError::ExecFailed(format!("seek failed: {e}")))?;
            file.read_exact(&mut buf)
                .map_err(|e| AgentToolError::ExecFailed(format!("read failed: {e}")))?;
        }
        let content = String::from_utf8_lossy(&buf).into_owned();
        let actual_bytes = buf.len() as u64;
        let eof = read_start + actual_bytes >= total;

        let cmd_line = if offset == 0 && limit == DEFAULT_LIMIT_BYTES {
            format!("read {uri}")
        } else {
            format!("read {uri} offset={offset} limit={limit}")
        };
        let summary = format!(
            "read {actual_bytes} bytes at offset {read_start} of {total}{}",
            if eof { " (EOF)" } else { "" }
        );
        let details = json!({
            "uri": uri,
            "scheme": "file",
            "path": resolved.to_string_lossy().to_string(),
            "content": content,
            "offset": read_start,
            "bytes_read": actual_bytes,
            "total_bytes": total,
            "eof": eof,
        });

        let mut result = build_builtin_tool_result(details, cmd_line, summary)
            .with_tool(TOOL_READ)
            .with_status(AgentToolStatus::Success);
        if !content.is_empty() {
            result = result.with_output(content);
        }
        Ok(result)
    }
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
struct ReadTarget {
    scheme: String,
    uri: String,
    path: String,
}

fn parse_read_target(raw: &str) -> Result<ReadTarget, AgentToolError> {
    if let Some((scheme, _)) = raw.split_once("://") {
        let scheme = scheme.trim();
        if scheme.is_empty() {
            return Err(AgentToolError::InvalidArgs(format!(
                "read uri has empty scheme: {raw}"
            )));
        }
        let path = if scheme == "file" {
            parse_file_uri_path(raw)?
        } else {
            String::new()
        };
        return Ok(ReadTarget {
            scheme: scheme.to_string(),
            uri: raw.to_string(),
            path,
        });
    }

    Ok(ReadTarget {
        scheme: "file".to_string(),
        uri: raw.to_string(),
        path: raw.to_string(),
    })
}

/// Parse `file://[host]/abs/path` → `/abs/path`. The host portion is
/// accepted but ignored (`localhost` and empty both valid, others rejected
/// to surface typos).
fn parse_file_uri_path(uri: &str) -> Result<String, AgentToolError> {
    let rest = uri
        .strip_prefix("file://")
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("malformed file uri: {uri}")))?;
    // After `file://`, expect either:
    //   - `/abs/path`        (no host)
    //   - `localhost/abs/path`
    //   - `host/abs/path`    (rejected for safety)
    if let Some(stripped) = rest.strip_prefix("localhost/") {
        return Ok(format!("/{stripped}"));
    }
    if rest.starts_with('/') {
        return Ok(rest.to_string());
    }
    // host without recognized name — reject so typos like `file://workspace/x`
    // don't silently get treated as host="workspace", path="/x".
    Err(AgentToolError::InvalidArgs(format!(
        "file:// uri must be of the form `file:///abs/path` or `file://localhost/abs/path`; got `{uri}`"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use tempfile::tempdir;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            step_idx: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
        }
    }

    fn ws_with_file(name: &str, content: &[u8]) -> (tempfile::TempDir, PathBuf) {
        let dir = tempdir().expect("tempdir");
        let ws = dir.path().join("workspace");
        fs::create_dir_all(&ws).expect("mkws");
        fs::write(ws.join(name), content).expect("write file");
        (dir, ws)
    }

    #[tokio::test]
    async fn reads_whole_small_file_with_default_limit() {
        let (_dir, ws) = ws_with_file("hello.txt", b"hello world\n");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        let res = tool
            .call(
                &ctx(),
                json!({ "uri": format!("file://{}/hello.txt", ws.to_string_lossy()) }),
            )
            .await
            .expect("call");
        assert_eq!(res.status, AgentToolStatus::Success);
        assert_eq!(res.details["content"], "hello world\n");
        assert_eq!(res.details["bytes_read"], 12);
        assert_eq!(res.details["eof"], true);
    }

    #[tokio::test]
    async fn bare_path_defaults_to_file_scheme() {
        let (_dir, ws) = ws_with_file("hello.txt", b"hello world\n");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        let res = tool
            .call(&ctx(), json!({ "uri": "hello.txt" }))
            .await
            .expect("call");
        assert_eq!(res.details["scheme"], "file");
        assert_eq!(res.details["content"], "hello world\n");
    }

    #[tokio::test]
    async fn offset_and_limit_paginate() {
        let (_dir, ws) = ws_with_file("big.txt", b"abcdefghijklmnop");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));

        // First chunk: bytes 0..4
        let res = tool
            .call(
                &ctx(),
                json!({
                    "uri": format!("file://{}/big.txt", ws.to_string_lossy()),
                    "offset": 0,
                    "limit": 4
                }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "abcd");
        assert_eq!(res.details["eof"], false);

        // Second chunk: bytes 4..8
        let res = tool
            .call(
                &ctx(),
                json!({
                    "uri": format!("file://{}/big.txt", ws.to_string_lossy()),
                    "offset": 4,
                    "limit": 4
                }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "efgh");

        // Past-EOF read returns empty content (not error).
        let res = tool
            .call(
                &ctx(),
                json!({
                    "uri": format!("file://{}/big.txt", ws.to_string_lossy()),
                    "offset": 100,
                    "limit": 4
                }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "");
        assert_eq!(res.details["bytes_read"], 0);
        assert_eq!(res.details["eof"], true);
    }

    #[tokio::test]
    async fn string_offset_limit_from_xml_attrs_works() {
        // The v2 XML parser supplies attribute values as JSON strings.
        let (_dir, ws) = ws_with_file("s.txt", b"abcdef");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        let res = tool
            .call(
                &ctx(),
                json!({
                    "uri": format!("file://{}/s.txt", ws.to_string_lossy()),
                    "offset": "1",
                    "limit": "3"
                }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "bcd");
    }

    #[tokio::test]
    async fn missing_uri_is_invalid_args() {
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(FileToolConfig::new(dir.path().join("workspace")));
        let err = tool.call(&ctx(), json!({})).await.expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn non_file_scheme_rejected() {
        let dir = tempdir().unwrap();
        let tool = ReadTool::new(FileToolConfig::new(dir.path().join("workspace")));
        let err = tool
            .call(&ctx(), json!({ "uri": "kv://agent/foo" }))
            .await
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn missing_file_returns_invalid_args() {
        let (_dir, ws) = ws_with_file("present.txt", b"x");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        let err = tool
            .call(
                &ctx(),
                json!({
                    "uri": format!("file://{}/absent.txt", ws.to_string_lossy())
                }),
            )
            .await
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[tokio::test]
    async fn path_escape_attempt_rejected() {
        let (_dir, ws) = ws_with_file("x.txt", b"x");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        // Try to escape workspace via `..` — resolve_path_under_root rejects.
        let err = tool
            .call(
                &ctx(),
                json!({
                    "uri": format!("file://{}/../etc/passwd", ws.to_string_lossy())
                }),
            )
            .await
            .expect_err("must reject");
        assert!(matches!(err, AgentToolError::InvalidArgs(_)));
    }

    #[test]
    fn file_uri_path_parser_accepts_canonical_forms() {
        assert_eq!(
            parse_file_uri_path("file:///abs/path").unwrap(),
            "/abs/path"
        );
        assert_eq!(
            parse_file_uri_path("file://localhost/abs/path").unwrap(),
            "/abs/path"
        );
        // Bare hosts rejected.
        assert!(parse_file_uri_path("file://workspace/x").is_err());
        // Non-file uri rejected.
        assert!(parse_file_uri_path("kv:///x").is_err());
    }

    #[test]
    fn read_target_defaults_paths_without_protocol_header_to_file() {
        assert_eq!(
            parse_read_target("src/main.rs").unwrap(),
            ReadTarget {
                scheme: "file".to_string(),
                uri: "src/main.rs".to_string(),
                path: "src/main.rs".to_string(),
            }
        );
        assert_eq!(
            parse_read_target("/abs/path").unwrap(),
            ReadTarget {
                scheme: "file".to_string(),
                uri: "/abs/path".to_string(),
                path: "/abs/path".to_string(),
            }
        );
        assert_eq!(
            parse_read_target("file:///abs/path").unwrap(),
            ReadTarget {
                scheme: "file".to_string(),
                uri: "file:///abs/path".to_string(),
                path: "/abs/path".to_string(),
            }
        );
        assert_eq!(
            parse_read_target("http://example.com").unwrap().scheme,
            "http"
        );
    }
}
