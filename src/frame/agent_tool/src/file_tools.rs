use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use log::warn;
use serde_json::{json, Value as Json};
use tokio::fs;

use crate::{
    tokenize_bash_command_line, AgentTool, AgentToolError, AgentToolResult, SessionRuntimeContext,
    ToolSpec,
};

pub const TOOL_EDIT_FILE: &str = "edit_file";
pub const TOOL_WRITE_FILE: &str = "write_file";
pub const TOOL_READ_FILE: &str = "read_file";

const CMD_PARAM_PREVIEW_KEEP_CHARS: usize = 32;

#[derive(Clone, Debug)]
pub struct FileToolConfig {
    pub root_dir: PathBuf,
}

impl FileToolConfig {
    pub fn new(root_dir: impl Into<PathBuf>) -> Self {
        Self {
            root_dir: root_dir.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct FileWriteAuditRecord {
    pub file_path: String,
    pub operation: String,
    pub created: bool,
    pub changed: bool,
    pub bytes_before: usize,
    pub bytes_after: usize,
    pub diff: String,
    pub diff_truncated: bool,
}

#[async_trait]
pub trait FileWriteAuditBackend: Send + Sync {
    async fn record_file_write(
        &self,
        ctx: &SessionRuntimeContext,
        args: &Json,
        record: &FileWriteAuditRecord,
    ) -> Result<(), AgentToolError>;
}

#[derive(Clone, Debug, Default)]
pub struct NoopFileWriteAudit;

#[async_trait]
impl FileWriteAuditBackend for NoopFileWriteAudit {
    async fn record_file_write(
        &self,
        _ctx: &SessionRuntimeContext,
        _args: &Json,
        _record: &FileWriteAuditRecord,
    ) -> Result<(), AgentToolError> {
        Ok(())
    }
}

#[derive(Clone)]
pub struct EditFileTool {
    cfg: FileToolConfig,
    write_audit: Arc<dyn FileWriteAuditBackend>,
}

impl EditFileTool {
    pub fn new(cfg: FileToolConfig, write_audit: Arc<dyn FileWriteAuditBackend>) -> Self {
        Self { cfg, write_audit }
    }
}

#[async_trait]
impl AgentTool for EditFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_EDIT_FILE.to_string(),
            description: "Edit file.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "pos_chunk": { "type": "string" },
                    "new_content": { "type": "string" },
                    "mode": { "type": "string", "enum": ["replace", "after", "before"] }
                },
                "required": ["path", "pos_chunk", "new_content"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
            usage: Some(
                "edit_file <path> --pos-chunk <text> [--mode replace|after|before] (--new-content <text> | --new-content-stdin)"
                    .to_string(),
            ),
        }
    }

    fn support_bash(&self) -> bool {
        false
    }

    fn support_action(&self) -> bool {
        true
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let file_path = require_string(&args, "path")?;
        let abs_path = resolve_path_from_root(&self.cfg.root_dir, &file_path)?;

        let exists = fs::metadata(&abs_path).await.is_ok();
        let original_content = if exists {
            read_text_file_lossy(&abs_path).await?
        } else {
            String::new()
        };

        let pos_chunk = require_string(&args, "pos_chunk")?;
        let new_content = require_string(&args, "new_content")?;
        let mode = parse_edit_mode(&args)?;
        let (operation, updated_content, matched) =
            if let Some(anchor_pos) = original_content.find(&pos_chunk) {
                let updated = match mode {
                    "replace" => {
                        let mut out = original_content.clone();
                        let end = anchor_pos + pos_chunk.len();
                        out.replace_range(anchor_pos..end, &new_content);
                        out
                    }
                    "after" => {
                        let mut out = original_content.clone();
                        let insert_at = anchor_pos + pos_chunk.len();
                        out.insert_str(insert_at, &new_content);
                        out
                    }
                    "before" => {
                        let mut out = original_content.clone();
                        out.insert_str(anchor_pos, &new_content);
                        out
                    }
                    _ => unreachable!("validated by parse_edit_mode"),
                };
                (mode.to_string(), updated, true)
            } else {
                (mode.to_string(), original_content.clone(), false)
            };

        let changed = original_content != updated_content;
        let created = changed && !exists;
        if changed {
            if let Some(parent) = abs_path.parent() {
                fs::create_dir_all(parent).await.map_err(|err| {
                    AgentToolError::ExecFailed(format!("create parent dir failed: {err}"))
                })?;
            }
            fs::write(&abs_path, updated_content.as_bytes())
                .await
                .map_err(|err| AgentToolError::ExecFailed(format!("write file failed: {err}")))?;
        }

        let (diff, diff_truncated) =
            build_simple_diff(&file_path, &original_content, &updated_content);
        if changed {
            if let Err(err) = self
                .write_audit
                .record_file_write(
                    ctx,
                    &args,
                    &FileWriteAuditRecord {
                        file_path: file_path.clone(),
                        operation: operation.clone(),
                        created,
                        changed,
                        bytes_before: original_content.len(),
                        bytes_after: updated_content.len(),
                        diff: diff.clone(),
                        diff_truncated,
                    },
                )
                .await
            {
                warn!(
                    "file_tool.edit_file_audit_failed: path={} operation={} err={}",
                    file_path, operation, err
                );
            }
        }

        let details = json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "operation": operation,
            "matched": matched,
            "created": created,
            "changed": changed,
            "bytes_before": original_content.len(),
            "bytes_after": updated_content.len(),
            "diff": diff,
            "diff_truncated": diff_truncated
        });
        let summary = if changed {
            format!(
                "{} {} -> {} bytes",
                operation,
                original_content.len(),
                updated_content.len()
            )
        } else if !matched {
            "anchor not found, no change".to_string()
        } else {
            "no change".to_string()
        };
        Ok(AgentToolResult::from_details(details)
            .with_cmd_line(format!(
                "{} {} mode={} pos_chunk=\"{}\" new_content=\"{}\"",
                TOOL_EDIT_FILE,
                file_path,
                operation,
                compact_cmd_param_preview(&pos_chunk),
                compact_cmd_param_preview(&new_content),
            ))
            .with_result(summary))
    }
}

#[derive(Clone)]
pub struct WriteFileTool {
    cfg: FileToolConfig,
    write_audit: Arc<dyn FileWriteAuditBackend>,
}

impl WriteFileTool {
    pub fn new(cfg: FileToolConfig, write_audit: Arc<dyn FileWriteAuditBackend>) -> Self {
        Self { cfg, write_audit }
    }
}

#[async_trait]
impl AgentTool for WriteFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_WRITE_FILE.to_string(),
            description: "Write file.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" },
                    "mode": { "type": "string", "enum": ["new", "append", "write"] }
                },
                "required": ["path", "content"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
            usage: Some(
                "write_file <path> [--mode new|append|write] (--content <text> | --content-stdin)"
                    .to_string(),
            ),
        }
    }

    fn support_bash(&self) -> bool {
        false
    }

    fn support_action(&self) -> bool {
        true
    }

    fn support_llm_tool_call(&self) -> bool {
        false
    }

    async fn call(
        &self,
        ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let file_path = require_string(&args, "path")?;
        let content = require_string(&args, "content")?;
        let abs_path = resolve_path_from_root(&self.cfg.root_dir, &file_path)?;

        let mode = parse_write_mode(&args)?;
        let exists = fs::metadata(&abs_path).await.is_ok();
        if mode == "new" && exists {
            return Err(AgentToolError::InvalidArgs(format!(
                "write mode `new` requires target file not exist: {file_path}"
            )));
        }

        let original_content = if exists {
            read_text_file_lossy(&abs_path).await?
        } else {
            String::new()
        };
        let operation = if mode == "append" {
            "append"
        } else if mode == "new" {
            "new"
        } else if exists {
            "write"
        } else {
            "create"
        }
        .to_string();
        let updated_content = if mode == "append" {
            format!("{original_content}{content}")
        } else {
            content.clone()
        };

        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                AgentToolError::ExecFailed(format!("create parent dir failed: {err}"))
            })?;
        }
        fs::write(&abs_path, updated_content.as_bytes())
            .await
            .map_err(|err| AgentToolError::ExecFailed(format!("write file failed: {err}")))?;

        let changed = original_content != updated_content;
        let (diff, diff_truncated) =
            build_simple_diff(&file_path, &original_content, &updated_content);
        if let Err(err) = self
            .write_audit
            .record_file_write(
                ctx,
                &args,
                &FileWriteAuditRecord {
                    file_path: file_path.clone(),
                    operation: operation.clone(),
                    created: !exists,
                    changed,
                    bytes_before: original_content.len(),
                    bytes_after: updated_content.len(),
                    diff: diff.clone(),
                    diff_truncated,
                },
            )
            .await
        {
            warn!(
                "file_tool.write_file_audit_failed: path={} operation={} err={}",
                file_path, operation, err
            );
        }

        let details = json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "operation": operation,
            "mode": mode,
            "created": !exists,
            "changed": changed,
            "bytes_before": original_content.len(),
            "bytes_after": updated_content.len(),
            "diff": diff,
            "diff_truncated": diff_truncated
        });
        let summary = format!(
            "{} {} -> {} bytes",
            operation,
            original_content.len(),
            updated_content.len()
        );
        Ok(AgentToolResult::from_details(details)
            .with_cmd_line(format!(
                "{} {} mode={} content=\"{}\"",
                TOOL_WRITE_FILE,
                file_path,
                mode,
                compact_cmd_param_preview(&content),
            ))
            .with_result(summary))
    }
}

#[derive(Clone, Debug)]
pub struct ReadFileTool {
    cfg: FileToolConfig,
}

impl ReadFileTool {
    pub fn new(cfg: FileToolConfig) -> Self {
        Self { cfg }
    }
}

#[async_trait]
impl AgentTool for ReadFileTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_READ_FILE.to_string(),
            description: "Read file.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "range": {},
                    "first_chunk": { "type": "string" }
                },
                "required": ["path"],
                "additionalProperties": true
            }),
            output_schema: json!({"type": "object"}),
            usage: Some(
                "read_file <path> [range] [first_chunk]\n\trange: 1-based; supports negative/$/+N, and applies within first_chunk slice"
                    .to_string(),
            ),
        }
    }

    fn support_bash(&self) -> bool {
        true
    }

    fn support_action(&self) -> bool {
        false
    }

    fn support_llm_tool_call(&self) -> bool {
        true
    }

    async fn call(
        &self,
        _ctx: &SessionRuntimeContext,
        args: Json,
    ) -> Result<AgentToolResult, AgentToolError> {
        let file_path = require_string(&args, "path")?;
        let abs_path = resolve_path_from_root(&self.cfg.root_dir, &file_path)?;

        let full_content = read_text_file_lossy(&abs_path).await?;
        let first_chunk = optional_string(&args, "first_chunk")?;
        let (selected_content, matched) = if let Some(first_chunk) = first_chunk.as_deref() {
            if let Some(pos) = full_content.find(first_chunk) {
                (full_content[pos..].to_string(), true)
            } else {
                (String::new(), false)
            }
        } else {
            (full_content.clone(), true)
        };

        let (selected_content, line_range_label) = {
            let lines = selected_content.lines().collect::<Vec<_>>();
            if let Some((start, end)) = parse_line_range(&args, lines.len())? {
                let start_idx = start.saturating_sub(1).min(lines.len());
                let end_idx = end.min(lines.len());
                let slice = if start_idx < end_idx {
                    lines[start_idx..end_idx].join("\n")
                } else {
                    String::new()
                };
                (slice, format!("{start}-{end}"))
            } else {
                (selected_content, String::new())
            }
        };

        let content = selected_content;
        let details = json!({
            "ok": true,
            "path": file_path,
            "abs_path": abs_path.to_string_lossy().to_string(),
            "content": content,
            "matched": matched,
            "line_range": line_range_label,
            "bytes": full_content.len(),
            "truncated": false,
            "pwd": self.cfg.root_dir.to_string_lossy().to_string(),
        });
        let stdout_payload = (!content.trim().is_empty()).then_some(content.clone());
        Ok(AgentToolResult::from_details(details)
            .with_cmd_line(build_read_file_cmd_line(
                &file_path,
                args.get("range"),
                first_chunk.as_deref(),
            ))
            .with_result(format!("read {} bytes", full_content.len()))
            .with_stdout(stdout_payload))
    }

    async fn exec(
        &self,
        ctx: &SessionRuntimeContext,
        line: &str,
        shell_cwd: Option<&Path>,
    ) -> Result<AgentToolResult, AgentToolError> {
        let tokens = tokenize_bash_command_line(line)?;
        if tokens.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "empty bash command line".to_string(),
            ));
        }

        let mut args = parse_read_file_bash_args(&tokens[1..])?;
        let shell_cwd_for_details = shell_cwd.map(|cwd| cwd.to_path_buf());
        if let Some(shell_cwd) = shell_cwd {
            rewrite_read_file_path_with_shell_cwd(&mut args, shell_cwd);
        }
        let mut result = self.call(ctx, args).await?;
        result.cmd_line = line.trim().to_string();
        if let Some(cwd) = shell_cwd_for_details {
            if let Some(map) = result.details.as_object_mut() {
                map.insert(
                    "pwd".to_string(),
                    Json::String(cwd.to_string_lossy().to_string()),
                );
            }
        }
        Ok(result)
    }
}

pub fn parse_read_file_bash_args(tokens: &[String]) -> Result<Json, AgentToolError> {
    let mut out = serde_json::Map::<String, Json>::new();
    if tokens.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "missing required arg `path`".to_string(),
        ));
    }

    let has_key_value = tokens.iter().any(|token| token.contains('='));
    if has_key_value {
        if tokens.iter().any(|token| !token.contains('=')) {
            return Err(AgentToolError::InvalidArgs(
                "read_file bash args cannot mix positional args with key=value args".to_string(),
            ));
        }
        for token in tokens {
            let (raw_key, raw_value) = token.split_once('=').ok_or_else(|| {
                AgentToolError::InvalidArgs("invalid key=value token".to_string())
            })?;
            let key = raw_key.trim();
            if key.is_empty() {
                return Err(AgentToolError::InvalidArgs(
                    "arg key cannot be empty".to_string(),
                ));
            }
            if key != "path" && key != "range" && key != "first_chunk" {
                return Err(AgentToolError::InvalidArgs(format!(
                    "unsupported read_file arg `{key}`"
                )));
            }
            out.insert(key.to_string(), Json::String(raw_value.trim().to_string()));
        }
    } else {
        if tokens.len() > 3 {
            return Err(AgentToolError::InvalidArgs(format!(
                "too many positional args for tool `{}`: got {}, max 3",
                TOOL_READ_FILE,
                tokens.len()
            )));
        }
        if let Some(path) = tokens.first() {
            out.insert("path".to_string(), Json::String(path.trim().to_string()));
        }
        if let Some(range) = tokens.get(1) {
            out.insert("range".to_string(), Json::String(range.trim().to_string()));
        }
        if let Some(first_chunk) = tokens.get(2) {
            out.insert(
                "first_chunk".to_string(),
                Json::String(first_chunk.trim().to_string()),
            );
        }
    }

    if out
        .get("path")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .is_none()
    {
        return Err(AgentToolError::InvalidArgs(
            "missing required arg `path`".to_string(),
        ));
    }

    Ok(Json::Object(out))
}

pub fn rewrite_read_file_path_with_shell_cwd(args: &mut Json, shell_cwd: &Path) {
    let Some(map) = args.as_object_mut() else {
        return;
    };
    let Some(raw_path) = map.get("path").and_then(|value| value.as_str()) else {
        return;
    };

    let trimmed = raw_path.trim();
    if trimmed.is_empty() {
        return;
    }
    let parsed = Path::new(trimmed);
    if parsed.is_absolute() {
        return;
    }

    let joined = shell_cwd.join(parsed);
    map.insert(
        "path".to_string(),
        Json::String(joined.to_string_lossy().to_string()),
    );
}

pub fn normalize_abs_path(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                let _ = normalized.pop();
            }
            Component::Normal(seg) => normalized.push(seg),
        }
    }
    normalized
}

fn require_string(args: &Json, key: &str) -> Result<String, AgentToolError> {
    let value = args
        .get(key)
        .and_then(|v| v.as_str())
        .map(|v| v.to_string())
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("missing or invalid `{key}`")))?;
    if value.is_empty() {
        return Err(AgentToolError::InvalidArgs(format!(
            "`{key}` cannot be empty"
        )));
    }
    Ok(value)
}

fn optional_string(args: &Json, key: &str) -> Result<Option<String>, AgentToolError> {
    let Some(value) = args.get(key) else {
        return Ok(None);
    };
    let raw = value
        .as_str()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("`{key}` must be a string")))?;
    Ok(Some(raw.to_string()))
}

fn parse_edit_mode(args: &Json) -> Result<&'static str, AgentToolError> {
    let raw = args
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("replace");
    match raw.trim().to_ascii_lowercase().as_str() {
        "" | "replace" => Ok("replace"),
        "after" => Ok("after"),
        "before" => Ok("before"),
        other => Err(AgentToolError::InvalidArgs(format!(
            "invalid edit mode `{other}`, expected replace/after/before"
        ))),
    }
}

fn parse_write_mode(args: &Json) -> Result<&'static str, AgentToolError> {
    let raw = args.get("mode").and_then(|v| v.as_str()).unwrap_or("write");
    match raw.trim().to_ascii_lowercase().as_str() {
        "new" | "create" => Ok("new"),
        "append" => Ok("append"),
        "" | "write" | "overwrite" => Ok("write"),
        other => Err(AgentToolError::InvalidArgs(format!(
            "invalid write mode `{other}`, expected new/append/write"
        ))),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LineMarker {
    Index(i64),
    End,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RangeDefaultEnd {
    SameAsStart,
    End,
    TailIfStartNegative,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct LineRangeSpec {
    start: Option<LineMarker>,
    end: Option<LineMarker>,
    count: Option<usize>,
    default_end: RangeDefaultEnd,
}

fn parse_line_range(
    args: &Json,
    total_lines: usize,
) -> Result<Option<(usize, usize)>, AgentToolError> {
    let Some(spec) = parse_line_range_spec(args)? else {
        return Ok(None);
    };
    let (start, end) = resolve_line_range(spec, total_lines)?;
    Ok(Some((start, end)))
}

fn parse_line_range_spec(args: &Json) -> Result<Option<LineRangeSpec>, AgentToolError> {
    let Some(raw) = args.get("range") else {
        return Ok(None);
    };

    if let Some(obj) = raw.as_object() {
        let start = obj
            .get("start")
            .map(|v| parse_line_marker_json(v, "range.start"))
            .transpose()?;
        let end = obj
            .get("end")
            .map(|v| parse_line_marker_json(v, "range.end"))
            .transpose()?;
        let count = obj
            .get("count")
            .map(|v| parse_line_count_json(v, "range.count"))
            .transpose()?;
        if count.is_some() && end.is_some() {
            return Err(AgentToolError::InvalidArgs(
                "range cannot set both `end` and `count`".to_string(),
            ));
        }
        if count.is_some() && start.is_none() {
            return Err(AgentToolError::InvalidArgs(
                "range.count requires range.start".to_string(),
            ));
        }
        if start.is_none() && end.is_none() && count.is_none() {
            return Err(AgentToolError::InvalidArgs(
                "range object must contain at least one of start/end/count".to_string(),
            ));
        }
        return Ok(Some(LineRangeSpec {
            start,
            end,
            count,
            default_end: RangeDefaultEnd::TailIfStartNegative,
        }));
    } else if let Some(arr) = raw.as_array() {
        if arr.is_empty() || arr.len() > 2 {
            return Err(AgentToolError::InvalidArgs(
                "range array must be [start] or [start,end]".to_string(),
            ));
        }
        let start = parse_line_marker_json(&arr[0], "range[0]")?;
        let end = if arr.len() == 2 {
            Some(parse_line_marker_json(&arr[1], "range[1]")?)
        } else {
            None
        };
        return Ok(Some(LineRangeSpec {
            start: Some(start),
            end,
            count: None,
            default_end: RangeDefaultEnd::SameAsStart,
        }));
    } else if raw.is_i64() || raw.is_u64() {
        let marker = parse_line_marker_json(raw, "range")?;
        return Ok(Some(LineRangeSpec {
            start: Some(marker),
            end: None,
            count: None,
            default_end: RangeDefaultEnd::SameAsStart,
        }));
    } else if let Some(text) = raw.as_str() {
        return parse_line_range_text(text);
    }

    Err(AgentToolError::InvalidArgs(
        "range must be string/number/array/object".to_string(),
    ))
}

fn parse_line_range_text(raw: &str) -> Result<Option<LineRangeSpec>, AgentToolError> {
    let text = raw.trim();
    if text.is_empty() || text == "-" {
        return Ok(None);
    }
    if text.contains(',') || text.contains(':') {
        let has_comma = text.contains(',');
        let has_colon = text.contains(':');
        if has_comma && has_colon {
            return Err(AgentToolError::InvalidArgs(format!(
                "invalid range text `{raw}`"
            )));
        }
        let delim = if has_comma { ',' } else { ':' };
        let parts = text.split(delim).collect::<Vec<_>>();
        if parts.len() != 2 {
            return Err(AgentToolError::InvalidArgs(format!(
                "invalid range text `{raw}`"
            )));
        }
        let start = if parts[0].trim().is_empty() {
            None
        } else {
            Some(parse_line_marker_token(parts[0].trim(), raw)?)
        };
        let end_text = parts[1].trim();
        if end_text.starts_with('+') {
            if start.is_none() {
                return Err(AgentToolError::InvalidArgs(format!(
                    "range `{raw}` with `+count` requires start"
                )));
            }
            let count = parse_line_count_text(end_text, raw)?;
            return Ok(Some(LineRangeSpec {
                start,
                end: None,
                count: Some(count),
                default_end: RangeDefaultEnd::SameAsStart,
            }));
        }
        let end = if end_text.is_empty() {
            None
        } else {
            Some(parse_line_marker_token(end_text, raw)?)
        };
        return Ok(Some(LineRangeSpec {
            start,
            end,
            count: None,
            default_end: RangeDefaultEnd::End,
        }));
    }

    if let Some((left, right)) = text.split_once('-') {
        let left_trim = left.trim();
        let right_trim = right.trim();
        if !left_trim.is_empty()
            && !right_trim.is_empty()
            && left_trim.chars().all(|ch| ch.is_ascii_digit())
            && right_trim.chars().all(|ch| ch.is_ascii_digit())
        {
            let start = parse_line_marker_token(left_trim, raw)?;
            let end = parse_line_marker_token(right_trim, raw)?;
            return Ok(Some(LineRangeSpec {
                start: Some(start),
                end: Some(end),
                count: None,
                default_end: RangeDefaultEnd::SameAsStart,
            }));
        }
    }

    let marker = parse_line_marker_token(text, raw)?;
    Ok(Some(LineRangeSpec {
        start: Some(marker),
        end: None,
        count: None,
        default_end: RangeDefaultEnd::SameAsStart,
    }))
}

fn parse_line_marker_token(raw: &str, range_text: &str) -> Result<LineMarker, AgentToolError> {
    let token = raw.trim();
    if token == "$" {
        return Ok(LineMarker::End);
    }
    if token.starts_with('+') {
        return Err(AgentToolError::InvalidArgs(format!(
            "invalid range text `{range_text}`"
        )));
    }
    let value = token
        .parse::<i64>()
        .map_err(|_| AgentToolError::InvalidArgs(format!("invalid range text `{range_text}`")))?;
    if value == 0 {
        return Err(AgentToolError::InvalidArgs(
            "range must be 1-based, zero is invalid".to_string(),
        ));
    }
    Ok(LineMarker::Index(value))
}

fn parse_line_count_text(raw: &str, range_text: &str) -> Result<usize, AgentToolError> {
    let token = raw.trim();
    let Some(count_text) = token.strip_prefix('+') else {
        return Err(AgentToolError::InvalidArgs(format!(
            "invalid range text `{range_text}`"
        )));
    };
    let count_u64 = count_text
        .trim()
        .parse::<u64>()
        .map_err(|_| AgentToolError::InvalidArgs(format!("invalid range text `{range_text}`")))?;
    if count_u64 == 0 {
        return Err(AgentToolError::InvalidArgs(
            "range count must be positive integer".to_string(),
        ));
    }
    u64_to_usize(count_u64)
}

fn parse_line_marker_json(value: &Json, name: &str) -> Result<LineMarker, AgentToolError> {
    if let Some(text) = value.as_str() {
        return parse_line_marker_token(text, name);
    }
    if let Some(v) = value.as_i64() {
        if v == 0 {
            return Err(AgentToolError::InvalidArgs(format!(
                "{name} must be non-zero integer"
            )));
        }
        return Ok(LineMarker::Index(v));
    }
    if let Some(v) = value.as_u64() {
        let as_i64 = i64::try_from(v)
            .map_err(|_| AgentToolError::InvalidArgs(format!("{name} is too large")))?;
        return Ok(LineMarker::Index(as_i64));
    }
    Err(AgentToolError::InvalidArgs(format!(
        "{name} must be integer or `$`"
    )))
}

fn parse_line_count_json(value: &Json, name: &str) -> Result<usize, AgentToolError> {
    let count_u64 = value
        .as_u64()
        .ok_or_else(|| AgentToolError::InvalidArgs(format!("{name} must be positive integer")))?;
    if count_u64 == 0 {
        return Err(AgentToolError::InvalidArgs(format!(
            "{name} must be positive integer"
        )));
    }
    u64_to_usize(count_u64)
}

fn resolve_line_range(
    spec: LineRangeSpec,
    total_lines: usize,
) -> Result<(usize, usize), AgentToolError> {
    let total_i64 = i64::try_from(total_lines)
        .map_err(|_| AgentToolError::InvalidArgs("file is too large".to_string()))?;
    let start_marker = spec.start.unwrap_or(LineMarker::Index(1));
    let start_raw = resolve_line_marker(start_marker, total_i64);
    let end_raw = if let Some(count) = spec.count {
        let count_i64 = i64::try_from(count)
            .map_err(|_| AgentToolError::InvalidArgs("range count is too large".to_string()))?;
        start_raw
            .checked_add(count_i64 - 1)
            .ok_or_else(|| AgentToolError::InvalidArgs("range end overflow".to_string()))?
    } else if let Some(end_marker) = spec.end {
        resolve_line_marker(end_marker, total_i64)
    } else {
        match spec.default_end {
            RangeDefaultEnd::SameAsStart => start_raw,
            RangeDefaultEnd::End => total_i64,
            RangeDefaultEnd::TailIfStartNegative => {
                if matches!(start_marker, LineMarker::Index(v) if v < 0) {
                    total_i64
                } else {
                    start_raw
                }
            }
        }
    };
    let start_clamped = start_raw.clamp(1, total_i64.saturating_add(1));
    let end_clamped = end_raw.clamp(0, total_i64);
    let start = usize::try_from(start_clamped)
        .map_err(|_| AgentToolError::InvalidArgs("range start out of bounds".to_string()))?;
    let end = usize::try_from(end_clamped)
        .map_err(|_| AgentToolError::InvalidArgs("range end out of bounds".to_string()))?;
    Ok((start, end))
}

fn resolve_line_marker(marker: LineMarker, total_lines: i64) -> i64 {
    match marker {
        LineMarker::End => total_lines,
        LineMarker::Index(v) if v > 0 => v,
        LineMarker::Index(v) => total_lines.saturating_add(v).saturating_add(1),
    }
}

fn resolve_path_from_root(root: &Path, raw_path: &str) -> Result<PathBuf, AgentToolError> {
    if raw_path.trim().is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "path cannot be empty".to_string(),
        ));
    }
    let user_path = Path::new(raw_path);
    let candidate = if user_path.is_absolute() {
        user_path.to_path_buf()
    } else {
        root.join(user_path)
    };
    Ok(normalize_abs_path(&candidate))
}

async fn read_text_file_lossy(path: &Path) -> Result<String, AgentToolError> {
    let bytes = fs::read(path)
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("read file failed: {err}")))?;
    Ok(String::from_utf8_lossy(&bytes).to_string())
}

fn compact_cmd_param_preview(raw: &str) -> String {
    let escaped = raw
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "\\r")
        .replace('\n', "\\n");
    let total_lines = if raw.is_empty() {
        0
    } else {
        raw.lines().count().max(1)
    };
    let total_chars = escaped.chars().count();
    if total_chars <= CMD_PARAM_PREVIEW_KEEP_CHARS * 2 {
        return escaped;
    }
    let head: String = escaped.chars().take(CMD_PARAM_PREVIEW_KEEP_CHARS).collect();
    let tail: String = escaped
        .chars()
        .rev()
        .take(CMD_PARAM_PREVIEW_KEEP_CHARS)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}...(total {total_lines} lines)...{tail}")
}

fn build_read_file_cmd_line(path: &str, range: Option<&Json>, first_chunk: Option<&str>) -> String {
    let mut cmd = format!("{TOOL_READ_FILE} {path}");
    if let Some(range) = range {
        let range_text = range
            .as_str()
            .map(|value| value.to_string())
            .unwrap_or_else(|| serde_json::to_string(range).unwrap_or_else(|_| "null".to_string()));
        if !range_text.trim().is_empty() {
            cmd.push_str(format!(" range={range_text}").as_str());
        }
    }
    if let Some(first_chunk) = first_chunk {
        cmd.push_str(
            format!(
                " first_chunk=\"{}\"",
                compact_cmd_param_preview(first_chunk)
            )
            .as_str(),
        );
    }
    cmd
}

fn build_simple_diff(display_path: &str, before: &str, after: &str) -> (String, bool) {
    let before_lines = before.lines().collect::<Vec<_>>();
    let after_lines = after.lines().collect::<Vec<_>>();
    let mut body = Vec::new();
    let max_len = before_lines.len().max(after_lines.len());
    for idx in 0..max_len {
        let old = before_lines.get(idx).copied();
        let new = after_lines.get(idx).copied();
        match (old, new) {
            (Some(a), Some(b)) if a == b => body.push(format!(" {a}")),
            (Some(a), Some(b)) => {
                body.push(format!("-{a}"));
                body.push(format!("+{b}"));
            }
            (Some(a), None) => body.push(format!("-{a}")),
            (None, Some(b)) => body.push(format!("+{b}")),
            (None, None) => {}
        }
    }
    let mut diff = Vec::new();
    diff.push(format!("--- a/{display_path}"));
    diff.push(format!("+++ b/{display_path}"));
    diff.push(format!(
        "@@ -1,{} +1,{} @@",
        before_lines.len(),
        after_lines.len()
    ));
    diff.extend(body);
    (diff.join("\n"), false)
}

fn u64_to_usize(v: u64) -> Result<usize, AgentToolError> {
    usize::try_from(v)
        .map_err(|_| AgentToolError::InvalidArgs("value is too large for current platform".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_line_range_text_formats() {
        assert_eq!(
            parse_line_range(&json!({"range": "10,20"}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10:20"}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10"}), 100).expect("parse"),
            Some((10, 10))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10,+5"}), 100).expect("parse"),
            Some((10, 14))
        );
        assert_eq!(
            parse_line_range(&json!({"range": ",30"}), 100).expect("parse"),
            Some((1, 30))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-30,"}), 100).expect("parse"),
            Some((71, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-30,$"}), 100).expect("parse"),
            Some((71, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-1"}), 100).expect("parse"),
            Some((100, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-5,-1"}), 100).expect("parse"),
            Some((96, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "10,$"}), 100).expect("parse"),
            Some((10, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "$"}), 100).expect("parse"),
            Some((100, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "1-2"}), 100).expect("parse"),
            Some((1, 2))
        );
        assert_eq!(
            parse_line_range(&json!({"range": "-"}), 100).expect("parse"),
            None
        );
    }

    #[test]
    fn compact_cmd_param_preview_truncates_with_line_hint() {
        let mut raw = String::new();
        for i in 0..120 {
            raw.push_str(format!("line-{i:04}\n").as_str());
        }
        let preview = compact_cmd_param_preview(&raw);
        assert!(preview.contains("...(total 120 lines)..."));
        assert!(preview.contains("line-0000\\nline-0001\\n"));
        assert!(preview.contains("line-0118\\nline-0119\\n"));
    }

    #[test]
    fn compact_cmd_param_preview_keeps_short_text() {
        let preview = compact_cmd_param_preview("hello\nworld");
        assert_eq!(preview, "hello\\nworld");
    }

    #[test]
    fn build_read_file_cmd_line_uses_compact_preview_for_first_chunk() {
        let mut chunk = String::new();
        for _ in 0..80 {
            chunk.push_str("line-x\n");
        }
        let cmd = build_read_file_cmd_line("demo.txt", Some(&json!("1-5")), Some(&chunk));
        assert!(cmd.contains("read_file demo.txt range=1-5 first_chunk=\""));
        assert!(cmd.contains("...(total 80 lines)..."));
    }

    #[test]
    fn parse_line_range_json_formats() {
        assert_eq!(
            parse_line_range(&json!({"range": {"start": 10, "end": 20}}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": {"start": 10}}), 100).expect("parse"),
            Some((10, 10))
        );
        assert_eq!(
            parse_line_range(&json!({"range": {"start": -5}}), 100).expect("parse"),
            Some((96, 100))
        );
        assert_eq!(
            parse_line_range(&json!({"range": {"start": 10, "count": 5}}), 100).expect("parse"),
            Some((10, 14))
        );
        assert_eq!(
            parse_line_range(&json!({"range": [10, 20]}), 100).expect("parse"),
            Some((10, 20))
        );
        assert_eq!(
            parse_line_range(&json!({"range": [10]}), 100).expect("parse"),
            Some((10, 10))
        );
        assert_eq!(
            parse_line_range(&json!({"range": 10}), 100).expect("parse"),
            Some((10, 10))
        );
    }

    #[test]
    fn parse_line_range_errors_on_invalid_forms() {
        assert!(parse_line_range(&json!({"range": "0"}), 10).is_err());
        assert!(parse_line_range(&json!({"range": "10,+0"}), 10).is_err());
        assert!(
            parse_line_range(&json!({"range": {"start": 1, "end": 2, "count": 3}}), 10).is_err()
        );
        assert!(parse_line_range(&json!({"range": {"count": 3}}), 10).is_err());
        assert!(parse_line_range(&json!({"range": [1, 2, 3]}), 10).is_err());
    }
}
