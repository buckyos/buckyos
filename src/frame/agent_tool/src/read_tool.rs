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
//! Compared with the legacy CLI-oriented `read_file`, this tool reads text
//! content with line-based pagination so the model can page through structured
//! data deterministically.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use log::warn;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

use crate::file_tools::FileToolConfig;
use crate::path_utils::{expand_home, normalize_root_path, resolve_path_from_root};
use crate::tool::CallingConventions;
use crate::{
    build_builtin_tool_result, AgentTool, AgentToolError, AgentToolResult, AgentToolStatus,
    SessionRuntimeContext, ToolSpec,
};

pub const TOOL_READ: &str = "read";

const READ_UNCHANGED_MESSAGE: &str = "和上一次read相比没有变化";

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
            description: "Read structured data by uri and return LLM-readable text.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "uri": {
                        "type": "string",
                        "description": "Structured data target to read. Bare paths default to file reads."
                    },
                    "offset": {
                        "type": "integer",
                        "minimum": 0,
                        "description": "Line offset to start reading at; defaults to 0."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Max lines to read."
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
                    "limit": {"type": ["integer", "null"]},
                    "lines_read": {"type": "integer"},
                    "total_lines": {"type": "integer"},
                    "eof": {"type": "boolean"},
                    "unchanged": {"type": "boolean"},
                    "token_limit": {"type": "integer"},
                    "token_truncated": {"type": "boolean"}
                }
            }),
            usage: Some(format!(
                "{TOOL_READ} uri=\"<path-or-uri>\" [offset=<line>] [limit=<lines>]"
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

        let uri = map
            .get("uri")
            .and_then(Json::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AgentToolError::InvalidArgs("`uri` is required".to_string()))?
            .to_string();

        let offset = parse_u64_arg(map.get("offset"), "offset")?.unwrap_or(0);
        let limit = parse_u64_arg(map.get("limit"), "limit")?;

        let target = parse_read_target(&uri)?;
        match target.scheme.as_str() {
            "file" => {
                self.read_file_path(ctx, &target.uri, &target.path, offset, limit)
                    .await
            }
            other => Err(AgentToolError::InvalidArgs(format!(
                "unsupported read scheme `{other}` (v1 first cut only supports `file`)"
            ))),
        }
    }
}

impl ReadTool {
    fn resolve_file_read_path(&self, path_str: &str) -> Result<PathBuf, AgentToolError> {
        if Path::new(path_str)
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(AgentToolError::InvalidArgs(format!(
                "read path not allowed by policy: {path_str}"
            )));
        }
        let root = normalize_root_path(&self.cfg.root_dir)?;
        let resolved = resolve_path_from_root(&root, path_str)?;
        if self.cfg.allowed_read_roots.is_empty() {
            return Ok(resolved);
        }
        for allowed_root in &self.cfg.allowed_read_roots {
            let allowed_root = normalize_root_path(allowed_root)?;
            if resolved.starts_with(&allowed_root) {
                return Ok(resolved);
            }
        }
        Err(AgentToolError::InvalidArgs(format!(
            "read path not allowed by policy: {path_str}"
        )))
    }

    async fn read_file_path(
        &self,
        ctx: &SessionRuntimeContext,
        uri: &str,
        path_str: &str,
        offset: u64,
        limit: Option<u64>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let resolved = self.resolve_file_read_path(path_str)?;
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
        let content_hash = hash_file_content(&resolved)?;
        let content = std::fs::read_to_string(&resolved).map_err(|e| {
            AgentToolError::InvalidArgs(format!(
                "read target must be UTF-8 text for LLM consumption: {} ({e})",
                resolved.display()
            ))
        })?;
        let line_window = build_line_window(&content, offset, limit);
        let read_key = ReadStateKey {
            path: resolved.to_string_lossy().to_string(),
            offset: line_window.offset,
            limit,
        };
        let unchanged = detect_and_update_read_state(
            ctx.session_id.as_str(),
            &read_key,
            line_window.total_lines,
            &content_hash,
        )
        .unwrap_or_else(|err| {
            warn!(
                "read_tool.state_update_failed: session_id={} path={} err={}",
                ctx.session_id,
                resolved.display(),
                err
            );
            false
        });
        let token_limit = ctx.effective_read_token_limit();
        if unchanged {
            let cmd_line = build_read_cmd_line(uri, offset, limit);
            let details = json!({
                "uri": uri,
                "scheme": "file",
                "path": resolved.to_string_lossy().to_string(),
                "content": READ_UNCHANGED_MESSAGE,
                "offset": line_window.offset,
                "limit": limit,
                "lines_read": 0u64,
                "total_lines": line_window.total_lines,
                "eof": line_window.eof,
                "unchanged": true,
                "token_limit": token_limit,
                "token_truncated": false,
            });
            return Ok(
                build_builtin_tool_result(details, cmd_line, READ_UNCHANGED_MESSAGE)
                    .with_tool(TOOL_READ)
                    .with_status(AgentToolStatus::Success)
                    .with_title(format!(
                        "read {uri} => read 0 lines{}",
                        if line_window.eof { " (EOF)" } else { "" }
                    ))
                    .with_output(READ_UNCHANGED_MESSAGE),
            );
        }
        let limited = apply_token_limit(&line_window.content, token_limit);

        let cmd_line = build_read_cmd_line(uri, offset, limit);
        let summary = format!(
            "read {} lines at offset {} of {}{}{}",
            line_window.lines_read,
            line_window.offset,
            line_window.total_lines,
            if line_window.eof { " (EOF)" } else { "" },
            if limited.truncated {
                " (token truncated)"
            } else {
                ""
            }
        );
        let title = format!(
            "read {uri} => read {} lines{}{}",
            line_window.lines_read,
            if line_window.eof { " (EOF)" } else { "" },
            if limited.truncated {
                " (token truncated)"
            } else {
                ""
            }
        );
        let details = json!({
            "uri": uri,
            "scheme": "file",
            "path": resolved.to_string_lossy().to_string(),
            "content": limited.content,
            "offset": line_window.offset,
            "limit": limit,
            "lines_read": line_window.lines_read,
            "total_lines": line_window.total_lines,
            "eof": line_window.eof,
            "unchanged": false,
            "token_limit": token_limit,
            "token_truncated": limited.truncated,
        });

        let mut result = build_builtin_tool_result(details, cmd_line, summary)
            .with_tool(TOOL_READ)
            .with_status(AgentToolStatus::Success)
            .with_title(title);
        if !limited.content.is_empty() {
            result = result.with_output(limited.content);
        }
        Ok(result)
    }
}

fn build_read_cmd_line(uri: &str, offset: u64, limit: Option<u64>) -> String {
    if offset == 0 && limit.is_none() {
        format!("read {uri}")
    } else if let Some(limit) = limit {
        format!("read {uri} offset={offset} limit={limit}")
    } else {
        format!("read {uri} offset={offset}")
    }
}

#[derive(Debug)]
struct ReadStateKey {
    path: String,
    offset: u64,
    limit: Option<u64>,
}

#[derive(Debug, Deserialize, Serialize)]
struct ReadStateRecord {
    path: String,
    offset: u64,
    limit: Option<u64>,
    total_lines: u64,
    content_hash: String,
}

fn hash_file_content(path: &Path) -> Result<String, AgentToolError> {
    let mut file = std::fs::File::open(path)
        .map_err(|e| AgentToolError::ExecFailed(format!("open failed: {e}")))?;
    let mut hasher = blake3::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| AgentToolError::ExecFailed(format!("read failed: {e}")))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().to_hex().to_string())
}

fn detect_and_update_read_state(
    session_id: &str,
    key: &ReadStateKey,
    total_lines: u64,
    content_hash: &str,
) -> Result<bool, AgentToolError> {
    let session_id = session_id.trim();
    if session_id.is_empty() {
        return Ok(false);
    }
    let state_path = read_state_path(session_id, key);
    let previous = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ReadStateRecord>(&bytes).ok());
    let unchanged = previous.as_ref().is_some_and(|record| {
        record.path == key.path
            && record.offset == key.offset
            && record.limit == key.limit
            && record.total_lines == total_lines
            && record.content_hash == content_hash
    });
    let record = ReadStateRecord {
        path: key.path.clone(),
        offset: key.offset,
        limit: key.limit,
        total_lines,
        content_hash: content_hash.to_string(),
    };
    if let Some(parent) = state_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            AgentToolError::ExecFailed(format!("create read state dir failed: {e}"))
        })?;
    }
    let data = serde_json::to_vec(&record)
        .map_err(|e| AgentToolError::ExecFailed(format!("serialize read state failed: {e}")))?;
    let tmp_path = state_path.with_extension("tmp");
    std::fs::write(&tmp_path, data)
        .map_err(|e| AgentToolError::ExecFailed(format!("write read state failed: {e}")))?;
    std::fs::rename(&tmp_path, &state_path)
        .map_err(|e| AgentToolError::ExecFailed(format!("rename read state failed: {e}")))?;
    Ok(unchanged)
}

fn read_state_path(session_id: &str, key: &ReadStateKey) -> PathBuf {
    let session_hash = blake3::hash(session_id.as_bytes()).to_hex().to_string();
    let key_hash = blake3::hash(format!("{}:{}:{:?}", key.path, key.offset, key.limit).as_bytes())
        .to_hex()
        .to_string();
    std::env::temp_dir()
        .join("buckyos_agent_tool")
        .join("read_state")
        .join(session_hash)
        .join(format!("{key_hash}.json"))
}

struct LineWindow {
    content: String,
    offset: u64,
    lines_read: u64,
    total_lines: u64,
    eof: bool,
}

fn build_line_window(content: &str, offset: u64, limit: Option<u64>) -> LineWindow {
    let lines: Vec<&str> = content.split_inclusive('\n').collect();
    let total_lines = lines.len() as u64;
    let start = offset.min(total_lines);
    let remaining = total_lines.saturating_sub(start);
    let lines_read = limit.unwrap_or(remaining).min(remaining);
    let end = start.saturating_add(lines_read);
    let mut window = String::new();
    for line in &lines[start as usize..end as usize] {
        window.push_str(line);
    }
    LineWindow {
        content: window,
        offset: start,
        lines_read,
        total_lines,
        eof: end >= total_lines,
    }
}

struct TokenLimitedText {
    content: String,
    truncated: bool,
}

fn apply_token_limit(content: &str, token_limit: u32) -> TokenLimitedText {
    let token_limit = token_limit.max(1);
    let mut used = 0u32;
    let mut out = String::new();
    let mut truncated = false;

    for line in content.split_inclusive('\n') {
        let line_tokens = estimate_text_tokens(line);
        if used.saturating_add(line_tokens) > token_limit {
            truncated = true;
            if out.is_empty() {
                let max_chars = token_limit.saturating_mul(4) as usize;
                out = line.chars().take(max_chars).collect();
            }
            break;
        }
        used = used.saturating_add(line_tokens);
        out.push_str(line);
    }

    TokenLimitedText {
        content: out,
        truncated,
    }
}

fn estimate_text_tokens(text: &str) -> u32 {
    ((text.chars().count() as u32).saturating_add(3) / 4).max(1)
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
        ctx_with_session("s")
    }

    fn ctx_with_session(session_id: &str) -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            step_idx: 0,
            wakeup_id: "w".into(),
            session_id: session_id.into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    fn ctx_with_read_token_limit(read_token_limit: u32) -> SessionRuntimeContext {
        SessionRuntimeContext {
            read_token_limit,
            ..ctx_with_session("token-limited-read-session")
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
        assert_eq!(res.details["lines_read"], 1);
        assert_eq!(res.details["total_lines"], 1);
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
    async fn repeated_same_session_same_read_returns_unchanged_message() {
        let (_dir, ws) = ws_with_file("hello.txt", b"hello world\n");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        let args = json!({ "uri": "hello.txt" });
        let session = ctx_with_session("repeat-read-session");

        let first = tool.call(&session, args.clone()).await.expect("first read");
        assert_eq!(first.details["content"], "hello world\n");
        assert_eq!(first.details["unchanged"], false);

        let second = tool
            .call(&session, args.clone())
            .await
            .expect("second read");
        assert_eq!(second.details["content"], READ_UNCHANGED_MESSAGE);
        assert_eq!(second.details["unchanged"], true);
        assert_eq!(second.output.as_deref(), Some(READ_UNCHANGED_MESSAGE));
        assert_eq!(second.summary, READ_UNCHANGED_MESSAGE);

        let other_session = tool
            .call(&ctx_with_session("repeat-read-other-session"), args.clone())
            .await
            .expect("other session read");
        assert_eq!(other_session.details["content"], "hello world\n");
        assert_eq!(other_session.details["unchanged"], false);

        fs::write(ws.join("hello.txt"), b"changed\n").expect("rewrite file");
        let changed = tool.call(&session, args).await.expect("changed read");
        assert_eq!(changed.details["content"], "changed\n");
        assert_eq!(changed.details["unchanged"], false);
    }

    #[tokio::test]
    async fn offset_and_limit_paginate() {
        let (_dir, ws) = ws_with_file("big.txt", b"a\nb\nc\nd\ne\nf\ng\nh\n");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));

        // First chunk: lines 0..4
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
        assert_eq!(res.details["content"], "a\nb\nc\nd\n");
        assert_eq!(res.details["lines_read"], 4);
        assert_eq!(res.details["eof"], false);

        // Second chunk: lines 4..8
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
        assert_eq!(res.details["content"], "e\nf\ng\nh\n");

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
        assert_eq!(res.details["lines_read"], 0);
        assert_eq!(res.details["eof"], true);
    }

    #[tokio::test]
    async fn string_offset_limit_from_xml_attrs_works() {
        // The v2 XML parser supplies attribute values as JSON strings.
        let (_dir, ws) = ws_with_file("s.txt", b"a\nb\nc\nd\ne\n");
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
        assert_eq!(res.details["content"], "b\nc\nd\n");
    }

    #[tokio::test]
    async fn session_read_token_limit_truncates_text_output() {
        let (_dir, ws) = ws_with_file("s.txt", b"abcdefgh\nijklmnop\nqrstuvwx\n");
        let tool = ReadTool::new(FileToolConfig::new(ws.clone()));
        let res = tool
            .call(
                &ctx_with_read_token_limit(3),
                json!({
                    "uri": format!("file://{}/s.txt", ws.to_string_lossy()),
                }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "abcdefgh\n");
        assert_eq!(res.details["token_limit"], 3);
        assert_eq!(res.details["token_truncated"], true);
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

    #[tokio::test]
    async fn extra_read_root_allows_file_outside_workspace() {
        let (_dir, ws) = ws_with_file("x.txt", b"x");
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("log.txt"), b"external log").unwrap();
        let mut cfg = FileToolConfig::new(ws.clone());
        cfg.allowed_read_roots.push(outside.path().to_path_buf());
        let tool = ReadTool::new(cfg);

        let res = tool
            .call(
                &ctx(),
                json!({ "uri": outside.path().join("log.txt").to_string_lossy().to_string() }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "external log");
    }

    #[tokio::test]
    async fn empty_read_roots_allows_unrestricted_file_read() {
        let (_dir, ws) = ws_with_file("x.txt", b"x");
        let outside = tempdir().unwrap();
        fs::write(outside.path().join("log.txt"), b"external log").unwrap();
        let mut cfg = FileToolConfig::new(ws.clone());
        cfg.allowed_read_roots.clear();
        let tool = ReadTool::new(cfg);

        let res = tool
            .call(
                &ctx(),
                json!({ "uri": outside.path().join("log.txt").to_string_lossy().to_string() }),
            )
            .await
            .expect("call");
        assert_eq!(res.details["content"], "external log");
    }

    #[tokio::test]
    async fn unrestricted_read_still_rejects_dot_dot_traversal() {
        let (_dir, ws) = ws_with_file("x.txt", b"x");
        let mut cfg = FileToolConfig::new(ws.clone());
        cfg.allowed_read_roots.clear();
        let tool = ReadTool::new(cfg);

        let err = tool
            .call(&ctx(), json!({ "uri": "../outside.txt" }))
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
    fn read_target_expands_leading_tilde_against_home() {
        let prev = std::env::var_os("HOME");
        std::env::set_var("HOME", "/home/agent");
        let target = parse_read_target("~/project/README.md").unwrap();
        assert_eq!(target.scheme, "file");
        assert_eq!(target.uri, "~/project/README.md");
        assert_eq!(target.path, "/home/agent/project/README.md");

        let target = parse_read_target("~").unwrap();
        assert_eq!(target.path, "/home/agent");

        // `~user/...` is not expanded — would need passwd lookup.
        let target = parse_read_target("~root/foo").unwrap();
        assert_eq!(target.path, "~root/foo");

        match prev {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
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
