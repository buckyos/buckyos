use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use sha2::{Digest, Sha256};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::agent_tool::{AgentTool, ToolError, ToolManager, ToolSpec};
use crate::behavior::TraceCtx;

pub const TOOL_LOAD_MEMORY: &str = "load_memory";
pub const TOOL_SET_MEMORY: &str = "set_memory";
pub const TOOL_LOAD_THINGS: &str = "load_things";

const DEFAULT_MEMORY_DIR_NAME: &str = "memory";
const DEFAULT_LOG_FILE_NAME: &str = "log.jsonl";
const DEFAULT_STATE_FILE_NAME: &str = "state.jsonl";
const DEFAULT_INDEX_DIR_NAME: &str = "index";
const DEFAULT_MAX_JSON_CONTENT_BYTES: usize = 256 * 1024;
const DEFAULT_TOKEN_LIMIT: u32 = 1200;
const MAX_INDEX_SEGMENT_BYTES: usize = 72;

#[derive(Clone, Debug)]
pub struct AgentMemoryConfig {
    pub agent_root: PathBuf,
    pub memory_dir_name: String,
    pub log_file_name: String,
    pub state_file_name: String,
    pub index_dir_name: String,
    pub max_json_content_bytes: usize,
    pub default_token_limit: u32,
}

impl AgentMemoryConfig {
    pub fn new(agent_root: impl Into<PathBuf>) -> Self {
        Self {
            agent_root: agent_root.into(),
            memory_dir_name: DEFAULT_MEMORY_DIR_NAME.to_string(),
            log_file_name: DEFAULT_LOG_FILE_NAME.to_string(),
            state_file_name: DEFAULT_STATE_FILE_NAME.to_string(),
            index_dir_name: DEFAULT_INDEX_DIR_NAME.to_string(),
            max_json_content_bytes: DEFAULT_MAX_JSON_CONTENT_BYTES,
            default_token_limit: DEFAULT_TOKEN_LIMIT,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentMemory {
    inner: Arc<AgentMemoryInner>,
}

#[derive(Debug)]
struct AgentMemoryInner {
    cfg: AgentMemoryConfig,
    memory_dir: PathBuf,
    log_path: PathBuf,
    state_path: PathBuf,
    index_root: PathBuf,
    write_lock: Mutex<()>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MemoryEnvelope {
    key: String,
    ts: String,
    valid: bool,
    source: Json,
    content: Json,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MemoryIndexDocument {
    key: String,
    ts: String,
    source: Json,
    content: Json,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LoadMemoryRequest {
    token_limit: u32,
    tags: Vec<String>,
    current_time: DateTime<Utc>,
}

impl AgentMemory {
    pub async fn new(mut cfg: AgentMemoryConfig) -> Result<Self, ToolError> {
        if cfg.max_json_content_bytes == 0 {
            cfg.max_json_content_bytes = DEFAULT_MAX_JSON_CONTENT_BYTES;
        }
        if cfg.default_token_limit == 0 {
            cfg.default_token_limit = DEFAULT_TOKEN_LIMIT;
        }

        let memory_dir = cfg.agent_root.join(&cfg.memory_dir_name);
        let log_path = memory_dir.join(&cfg.log_file_name);
        let state_path = memory_dir.join(&cfg.state_file_name);
        let index_root = memory_dir.join(&cfg.index_dir_name);

        fs::create_dir_all(&index_root).await.map_err(|err| {
            ToolError::ExecFailed(format!("create memory index dir failed: {err}"))
        })?;
        touch_file(&log_path).await?;
        touch_file(&state_path).await?;

        let instance = Self {
            inner: Arc::new(AgentMemoryInner {
                cfg,
                memory_dir,
                log_path,
                state_path,
                index_root,
                write_lock: Mutex::new(()),
            }),
        };
        instance.bootstrap_if_needed().await?;
        Ok(instance)
    }

    pub fn memory_dir(&self) -> &Path {
        &self.inner.memory_dir
    }

    pub fn register_tools(&self, tool_mgr: &ToolManager) -> Result<(), ToolError> {
        if !tool_mgr.has_tool(TOOL_SET_MEMORY) {
            tool_mgr.register_tool(SetMemoryTool::new(self.clone()))?;
        }
        if !tool_mgr.has_tool(TOOL_LOAD_MEMORY) {
            tool_mgr.register_tool(LoadMemoryTool::new(self.clone()))?;
        }
        Ok(())
    }

    pub async fn set_memory(
        &self,
        key: &str,
        json_content: Json,
        source: Json,
    ) -> Result<Json, ToolError> {
        let normalized_key = normalize_key(key)?;
        validate_source(&normalized_key, &source)?;

        if !json_content.is_null() {
            let payload_size = serde_json::to_vec(&json_content)
                .map_err(|err| {
                    ToolError::ExecFailed(format!("serialize json_content failed: {err}"))
                })?
                .len();
            if payload_size > self.inner.cfg.max_json_content_bytes {
                return Err(ToolError::InvalidArgs(format!(
                    "json_content too large: {} bytes > {} bytes",
                    payload_size, self.inner.cfg.max_json_content_bytes
                )));
            }
        }

        let now = Utc::now();
        let envelope = MemoryEnvelope {
            key: normalized_key.clone(),
            ts: now.to_rfc3339(),
            valid: !json_content.is_null(),
            source,
            content: json_content,
        };

        let index_path = self.index_path_for_key(&normalized_key);
        {
            let _guard = self.inner.write_lock.lock().await;
            self.append_log_line(&envelope).await?;

            let mut current = self.read_state_map().await?;
            if envelope.valid {
                current.insert(normalized_key.clone(), envelope.clone());
                self.write_index_doc(&index_path, &envelope).await?;
            } else {
                current.remove(&normalized_key);
                if let Err(err) = fs::remove_file(&index_path).await {
                    if err.kind() != std::io::ErrorKind::NotFound {
                        return Err(ToolError::ExecFailed(format!(
                            "remove memory index file failed: path={}, err={err}",
                            index_path.display()
                        )));
                    }
                }
            }

            self.write_state_map_atomic(&current).await?;
        }
        info!(
            "agent_memory.set_memory: key={} valid={} index={}",
            normalized_key,
            envelope.valid,
            index_path.display()
        );

        Ok(json!({
            "ok": true,
            "key": normalized_key,
            "valid": envelope.valid,
            "ts": envelope.ts,
            "index_path": index_path.to_string_lossy(),
        }))
    }

    pub async fn load_memory(
        &self,
        token_limit: Option<u32>,
        tags: Vec<String>,
        current_time: Option<DateTime<Utc>>,
    ) -> Result<Json, ToolError> {
        let request = LoadMemoryRequest {
            token_limit: token_limit
                .unwrap_or(self.inner.cfg.default_token_limit)
                .max(1),
            tags,
            current_time: current_time.unwrap_or_else(Utc::now),
        };
        self.load_memory_by_request(request).await
    }

    pub async fn compact(&self) -> Result<Json, ToolError> {
        let _guard = self.inner.write_lock.lock().await;
        self.compact_locked().await
    }

    async fn bootstrap_if_needed(&self) -> Result<(), ToolError> {
        let state_len = file_len_or_zero(&self.inner.state_path).await;
        if state_len > 0 {
            return Ok(());
        }
        let log_len = file_len_or_zero(&self.inner.log_path).await;
        if log_len == 0 {
            return Ok(());
        }
        info!(
            "agent_memory.bootstrap: rebuilding state from log: path={}",
            self.inner.log_path.display()
        );
        let _guard = self.inner.write_lock.lock().await;
        self.compact_locked().await?;
        Ok(())
    }

    async fn compact_locked(&self) -> Result<Json, ToolError> {
        let records = self.read_latest_from_log().await?;
        let now = Utc::now();
        let mut active = HashMap::<String, MemoryEnvelope>::new();
        let mut expired = 0_usize;
        let mut tombstone = 0_usize;

        for record in records.values() {
            if !record.valid {
                tombstone += 1;
                continue;
            }
            if is_expired_at(record.content.get("expired_at"), &now) {
                expired += 1;
                continue;
            }
            active.insert(record.key.clone(), record.clone());
        }

        self.write_state_map_atomic(&active).await?;
        self.rebuild_index_from_active(&active).await?;

        Ok(json!({
            "ok": true,
            "total_keys": records.len(),
            "active_keys": active.len(),
            "expired_keys": expired,
            "tombstone_keys": tombstone
        }))
    }

    async fn load_memory_by_request(&self, req: LoadMemoryRequest) -> Result<Json, ToolError> {
        let mut records = self.read_state_map().await?;
        if records.is_empty() {
            records = self.read_latest_from_log().await?;
        }

        let requested_tags: HashSet<String> = req
            .tags
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();

        let mut candidates = Vec::<MemoryRankItem>::new();
        for record in records.values() {
            if !record.valid {
                continue;
            }
            if is_expired_at(record.content.get("expired_at"), &req.current_time) {
                continue;
            }

            let importance = extract_importance(&record.content);
            let type_name = extract_type_name(&record.content);
            let tags = extract_tags(&record.content);
            let tag_score = tags
                .iter()
                .filter(|tag| requested_tags.contains(tag.as_str()))
                .count() as u32;
            let summary = extract_summary_text(&record.content);

            candidates.push(MemoryRankItem {
                key: record.key.clone(),
                ts: record.ts.clone(),
                source: record.source.clone(),
                content: record.content.clone(),
                type_name,
                summary,
                importance,
                tag_score,
                ts_unix_ms: parse_rfc3339_to_ms(&record.ts).unwrap_or(0),
            });
        }

        candidates.sort_by(rank_candidates);

        let mut lines = vec!["[Agent Memory]".to_string()];
        let mut selected_items = Vec::<Json>::new();
        let mut used_tokens = estimate_token_count(lines[0].as_str());
        let mut truncated = false;

        for item in &candidates {
            let line = render_memory_line(item);
            let line_tokens = estimate_token_count(&line);
            if used_tokens.saturating_add(line_tokens) > req.token_limit as usize {
                truncated = true;
                break;
            }
            used_tokens += line_tokens;
            lines.push(line);
            selected_items.push(json!({
                "key": item.key,
                "type": item.type_name,
                "summary": item.summary,
                "ts": item.ts,
                "source": item.source,
                "content": item.content,
                "importance": item.importance,
                "tag_score": item.tag_score
            }));
        }

        if lines.len() == 1 {
            lines.push("- (empty)".to_string());
        }

        debug!(
            "agent_memory.load_memory: token_limit={} selected={} total={} tags={}",
            req.token_limit,
            selected_items.len(),
            candidates.len(),
            req.tags.join(",")
        );

        Ok(json!({
            "memory_text": lines.join("\n"),
            "items": selected_items,
            "selected": selected_items.len(),
            "total_candidates": candidates.len(),
            "token_limit": req.token_limit,
            "token_estimate": used_tokens,
            "truncated": truncated,
            "generated_at": Utc::now().to_rfc3339(),
        }))
    }

    async fn append_log_line(&self, envelope: &MemoryEnvelope) -> Result<(), ToolError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.inner.log_path)
            .await
            .map_err(|err| {
                ToolError::ExecFailed(format!(
                    "open memory log for append failed: path={}, err={err}",
                    self.inner.log_path.display()
                ))
            })?;

        let line = serde_json::to_string(envelope).map_err(|err| {
            ToolError::ExecFailed(format!("serialize memory envelope failed: {err}"))
        })?;
        file.write_all(line.as_bytes()).await.map_err(|err| {
            ToolError::ExecFailed(format!("append memory log line failed: {err}"))
        })?;
        file.write_all(b"\n").await.map_err(|err| {
            ToolError::ExecFailed(format!("append memory log newline failed: {err}"))
        })?;
        file.flush()
            .await
            .map_err(|err| ToolError::ExecFailed(format!("flush memory log failed: {err}")))?;
        Ok(())
    }

    async fn read_state_map(&self) -> Result<HashMap<String, MemoryEnvelope>, ToolError> {
        self.read_jsonl_map(&self.inner.state_path).await
    }

    async fn read_latest_from_log(&self) -> Result<HashMap<String, MemoryEnvelope>, ToolError> {
        self.read_jsonl_map(&self.inner.log_path).await
    }

    async fn read_jsonl_map(
        &self,
        file_path: &Path,
    ) -> Result<HashMap<String, MemoryEnvelope>, ToolError> {
        let payload = match fs::read_to_string(file_path).await {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(HashMap::new()),
            Err(err) => {
                return Err(ToolError::ExecFailed(format!(
                    "read memory file failed: path={}, err={err}",
                    file_path.display()
                )))
            }
        };

        let mut result = HashMap::<String, MemoryEnvelope>::new();
        for (line_idx, raw_line) in payload.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<MemoryEnvelope>(line) {
                Ok(record) => {
                    result.insert(record.key.clone(), record);
                }
                Err(err) => {
                    warn!(
                        "agent_memory.invalid_jsonl_line: path={} line={} err={}",
                        file_path.display(),
                        line_idx + 1,
                        err
                    );
                }
            }
        }
        Ok(result)
    }

    async fn write_state_map_atomic(
        &self,
        state_map: &HashMap<String, MemoryEnvelope>,
    ) -> Result<(), ToolError> {
        let mut keys: Vec<&String> = state_map.keys().collect();
        keys.sort();

        let mut body = String::new();
        for key in keys {
            let Some(record) = state_map.get(key) else {
                continue;
            };
            if !record.valid {
                continue;
            }
            let line = serde_json::to_string(record).map_err(|err| {
                ToolError::ExecFailed(format!("serialize state line failed: {err}"))
            })?;
            body.push_str(&line);
            body.push('\n');
        }

        write_atomic_text(&self.inner.state_path, &body).await
    }

    async fn rebuild_index_from_active(
        &self,
        active: &HashMap<String, MemoryEnvelope>,
    ) -> Result<(), ToolError> {
        if let Err(err) = fs::remove_dir_all(&self.inner.index_root).await {
            if err.kind() != std::io::ErrorKind::NotFound {
                return Err(ToolError::ExecFailed(format!(
                    "cleanup memory index dir failed: path={}, err={err}",
                    self.inner.index_root.display()
                )));
            }
        }
        fs::create_dir_all(&self.inner.index_root)
            .await
            .map_err(|err| {
                ToolError::ExecFailed(format!("recreate memory index dir failed: {err}"))
            })?;

        let mut keys: Vec<&String> = active.keys().collect();
        keys.sort();
        for key in keys {
            let Some(record) = active.get(key) else {
                continue;
            };
            let index_path = self.index_path_for_key(&record.key);
            self.write_index_doc(&index_path, record).await?;
        }
        Ok(())
    }

    async fn write_index_doc(
        &self,
        index_path: &Path,
        record: &MemoryEnvelope,
    ) -> Result<(), ToolError> {
        if let Some(parent) = index_path.parent() {
            fs::create_dir_all(parent).await.map_err(|err| {
                ToolError::ExecFailed(format!(
                    "create index parent dir failed: path={}, err={err}",
                    parent.display()
                ))
            })?;
        }
        let doc = MemoryIndexDocument {
            key: record.key.clone(),
            ts: record.ts.clone(),
            source: record.source.clone(),
            content: record.content.clone(),
        };
        let payload = serde_json::to_string_pretty(&doc).map_err(|err| {
            ToolError::ExecFailed(format!("serialize memory index doc failed: {err}"))
        })?;
        write_atomic_text(index_path, &payload).await
    }

    fn index_path_for_key(&self, key: &str) -> PathBuf {
        let segments: Vec<&str> = key.trim_start_matches('/').split('/').collect();
        let mut path = self.inner.index_root.clone();
        if segments.is_empty() {
            return path.join("_root@00000000.json");
        }
        if segments.len() == 1 {
            let file_segment = build_index_file_segment(segments[0]);
            return path.join(file_segment);
        }

        for segment in &segments[..segments.len() - 1] {
            path = path.join(build_index_dir_segment(segment));
        }
        path.join(build_index_file_segment(segments[segments.len() - 1]))
    }
}

struct SetMemoryTool {
    memory: AgentMemory,
}

impl SetMemoryTool {
    fn new(memory: AgentMemory) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl AgentTool for SetMemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_SET_MEMORY.to_string(),
            description: "Set or invalidate an agent memory entry using key/json_content/source."
                .to_string(),
            args_schema: json!({
                "type": "object",
                "required": ["key", "json_content", "source"],
                "properties": {
                    "key": { "type": "string" },
                    "json_content": {},
                    "source": {
                        "anyOf": [
                            { "type": "string" },
                            { "type": "object" }
                        ]
                    }
                }
            }),
            output_schema: json!({
                "type": "object",
                "properties": {
                    "ok": {"type":"boolean"},
                    "key": {"type":"string"},
                    "valid": {"type":"boolean"},
                    "ts": {"type":"string"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let key = require_arg_string(&args, "key")?;
        let json_content = args
            .get("json_content")
            .cloned()
            .ok_or_else(|| ToolError::InvalidArgs("missing `json_content`".to_string()))?;
        let source = args
            .get("source")
            .cloned()
            .ok_or_else(|| ToolError::InvalidArgs("missing `source`".to_string()))?;
        self.memory.set_memory(&key, json_content, source).await
    }
}

struct LoadMemoryTool {
    memory: AgentMemory,
}

impl LoadMemoryTool {
    fn new(memory: AgentMemory) -> Self {
        Self { memory }
    }
}

#[async_trait]
impl AgentTool for LoadMemoryTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_LOAD_MEMORY.to_string(),
            description: "Read memory summary using default retrieval strategy.".to_string(),
            args_schema: json!({
                "type": "object",
                "properties": {
                    "token_limit": {"type":"number"},
                    "tags": {
                        "type":"array",
                        "items": {"type":"string"}
                    },
                    "current_time": {"type":"string"}
                }
            }),
            output_schema: json!({
                "type":"object",
                "properties": {
                    "memory_text": {"type":"string"},
                    "items": {"type":"array"}
                }
            }),
        }
    }

    async fn call(&self, _ctx: &TraceCtx, args: Json) -> Result<Json, ToolError> {
        let token_limit = args
            .get("token_limit")
            .and_then(|v| v.as_u64())
            .map(|n| n.min(u32::MAX as u64) as u32);
        let tags = args
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                    .filter(|s| !s.is_empty())
                    .collect::<Vec<String>>()
            })
            .unwrap_or_default();
        let current_time = args
            .get("current_time")
            .and_then(|v| v.as_str())
            .and_then(|raw| parse_rfc3339_to_utc(raw).ok());

        self.memory
            .load_memory(token_limit, tags, current_time)
            .await
    }
}

#[derive(Clone, Debug)]
struct MemoryRankItem {
    key: String,
    ts: String,
    source: Json,
    content: Json,
    type_name: String,
    summary: String,
    importance: i64,
    tag_score: u32,
    ts_unix_ms: i64,
}

fn rank_candidates(a: &MemoryRankItem, b: &MemoryRankItem) -> Ordering {
    b.ts_unix_ms
        .cmp(&a.ts_unix_ms)
        .then_with(|| b.importance.cmp(&a.importance))
        .then_with(|| b.tag_score.cmp(&a.tag_score))
        .then_with(|| a.key.cmp(&b.key))
}

fn render_memory_line(item: &MemoryRankItem) -> String {
    let key_part = item.key.trim_start_matches('/');
    let type_part = if item.type_name.is_empty() {
        String::new()
    } else {
        format!(" {}", item.type_name)
    };
    format!(
        "- {}{} {}",
        key_part,
        type_part,
        truncate_chars(item.summary.as_str(), 120)
    )
}

fn extract_importance(content: &Json) -> i64 {
    match content.get("importance") {
        Some(Json::Number(n)) => n
            .as_i64()
            .or_else(|| n.as_f64().map(|f| f as i64))
            .unwrap_or(0),
        Some(Json::String(s)) => s.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn extract_type_name(content: &Json) -> String {
    content
        .get("type")
        .and_then(|v| v.as_str())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
}

fn extract_tags(content: &Json) -> Vec<String> {
    content
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn extract_summary_text(content: &Json) -> String {
    if let Some(summary) = content
        .get("summary")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return summary.to_string();
    }
    if let Some(text) = content
        .get("text")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return text.to_string();
    }
    if let Some(text) = content.as_str().map(str::trim).filter(|v| !v.is_empty()) {
        return text.to_string();
    }
    let serialized = serde_json::to_string(content).unwrap_or_else(|_| "{}".to_string());
    truncate_chars(serialized.as_str(), 140)
}

fn parse_rfc3339_to_ms(raw: &str) -> Option<i64> {
    parse_rfc3339_to_utc(raw)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

fn parse_rfc3339_to_utc(raw: &str) -> Result<DateTime<Utc>, chrono::ParseError> {
    DateTime::parse_from_rfc3339(raw).map(|dt| dt.with_timezone(&Utc))
}

fn is_expired_at(raw_expired_at: Option<&Json>, now: &DateTime<Utc>) -> bool {
    let Some(raw) = raw_expired_at else {
        return false;
    };
    let Some(text) = raw.as_str() else {
        return false;
    };
    parse_rfc3339_to_utc(text)
        .map(|expired| now > &expired)
        .unwrap_or(false)
}

fn require_arg_string(args: &Json, field: &str) -> Result<String, ToolError> {
    args.get(field)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
        .ok_or_else(|| ToolError::InvalidArgs(format!("missing or invalid `{field}`")))
}

fn normalize_key(raw_key: &str) -> Result<String, ToolError> {
    let key = raw_key.trim();
    if key.is_empty() {
        return Err(ToolError::InvalidArgs(
            "memory key cannot be empty".to_string(),
        ));
    }
    if !key.starts_with('/') {
        return Err(ToolError::InvalidArgs(
            "memory key must start with `/`".to_string(),
        ));
    }
    if key.contains('\0') || key.contains('\n') || key.contains('\r') {
        return Err(ToolError::InvalidArgs(
            "memory key contains forbidden control characters".to_string(),
        ));
    }

    let mut segments = Vec::<&str>::new();
    for segment in key.split('/') {
        let seg = segment.trim();
        if seg.is_empty() {
            continue;
        }
        if seg == "." || seg == ".." {
            return Err(ToolError::InvalidArgs(
                "memory key cannot contain `.` or `..` segments".to_string(),
            ));
        }
        segments.push(seg);
    }

    if segments.is_empty() {
        return Err(ToolError::InvalidArgs(
            "memory key must include at least one segment".to_string(),
        ));
    }
    Ok(format!("/{}", segments.join("/")))
}

fn validate_source(key: &str, source: &Json) -> Result<(), ToolError> {
    if source.is_null() {
        return Err(ToolError::InvalidArgs(
            "source is required and cannot be null".to_string(),
        ));
    }

    if let Some(text) = source.as_str() {
        if text.trim().is_empty() {
            return Err(ToolError::InvalidArgs(
                "source string cannot be empty".to_string(),
            ));
        }
        if key.starts_with("/kb/") || key == "/kb" {
            return Err(ToolError::InvalidArgs(
                "kb namespace requires object provenance source".to_string(),
            ));
        }
        return Ok(());
    }

    let Some(obj) = source.as_object() else {
        return Err(ToolError::InvalidArgs(
            "source must be object or string".to_string(),
        ));
    };

    let kind = obj.get("kind").and_then(|v| v.as_str()).unwrap_or_default();
    let should_require_provenance =
        key.starts_with("/kb/") || key == "/kb" || matches!(kind, "web" | "tool" | "file");

    if should_require_provenance {
        let name = obj.get("name").and_then(|v| v.as_str()).unwrap_or_default();
        let retrieved_at = obj
            .get("retrieved_at")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let locator = obj.get("locator");

        if kind.trim().is_empty()
            || name.trim().is_empty()
            || retrieved_at.trim().is_empty()
            || locator.is_none()
        {
            return Err(ToolError::InvalidArgs(
                "source missing required provenance fields: kind/name/retrieved_at/locator"
                    .to_string(),
            ));
        }
        parse_rfc3339_to_utc(retrieved_at).map_err(|err| {
            ToolError::InvalidArgs(format!("source.retrieved_at must be RFC3339: {err}"))
        })?;
    }

    Ok(())
}

fn build_index_dir_segment(raw_segment: &str) -> String {
    sanitize_index_segment(raw_segment, false)
}

fn build_index_file_segment(raw_segment: &str) -> String {
    format!(
        "{}@{}.json",
        sanitize_index_segment(raw_segment, true),
        short_hash_hex(raw_segment.as_bytes())
    )
}

fn sanitize_index_segment(raw_segment: &str, is_file_name: bool) -> String {
    let mut encoded = String::new();
    for byte in raw_segment.as_bytes() {
        let c = *byte as char;
        if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' {
            encoded.push(c);
        } else {
            encoded.push('%');
            encoded.push_str(format!("{:02X}", byte).as_str());
        }
    }

    if encoded.is_empty() {
        encoded.push('_');
    }

    if encoded.len() > MAX_INDEX_SEGMENT_BYTES {
        let keep_len = MAX_INDEX_SEGMENT_BYTES.min(encoded.len());
        let prefix = encoded.chars().take(keep_len).collect::<String>();
        let digest = short_hash_hex(raw_segment.as_bytes());
        encoded = format!("{prefix}@{digest}");
    }

    if is_file_name && encoded.ends_with('.') {
        encoded.push('_');
    }
    encoded
}

fn short_hash_hex(data: &[u8]) -> String {
    let digest = Sha256::digest(data);
    hex::encode(&digest[..4])
}

fn truncate_chars(input: &str, max_chars: usize) -> String {
    if input.chars().count() <= max_chars {
        return input.to_string();
    }
    let mut out = input.chars().take(max_chars).collect::<String>();
    out.push_str("...");
    out
}

fn estimate_token_count(text: &str) -> usize {
    text.split_whitespace().count().max(1)
}

async fn touch_file(path: &Path) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "create parent directory failed: path={}, err={err}",
                parent.display()
            ))
        })?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
        .map_err(|err| {
            ToolError::ExecFailed(format!(
                "touch file failed: path={}, err={err}",
                path.display()
            ))
        })?;
    Ok(())
}

async fn file_len_or_zero(path: &Path) -> u64 {
    fs::metadata(path).await.map(|meta| meta.len()).unwrap_or(0)
}

async fn write_atomic_text(path: &Path, body: &str) -> Result<(), ToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            ToolError::ExecFailed(format!(
                "create parent dir failed for atomic write: path={}, err={err}",
                parent.display()
            ))
        })?;
    }

    let tmp_name = format!(
        ".{}.tmp.{}",
        path.file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("memory"),
        Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let tmp_path = path
        .parent()
        .map(|p| p.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(tmp_name));

    fs::write(&tmp_path, body).await.map_err(|err| {
        ToolError::ExecFailed(format!(
            "write temporary file failed: path={}, err={err}",
            tmp_path.display()
        ))
    })?;
    fs::rename(&tmp_path, path).await.map_err(|err| {
        ToolError::ExecFailed(format!(
            "atomic rename failed: from={} to={} err={err}",
            tmp_path.display(),
            path.display()
        ))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn test_trace_ctx() -> TraceCtx {
        TraceCtx {
            trace_id: "trace-memory".to_string(),
            agent_did: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-memory".to_string(),
        }
    }

    #[tokio::test]
    async fn set_memory_then_load_memory_returns_recent_item() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        let _ = memory
            .set_memory(
                "/user/preference/style",
                json!({
                    "type":"preference",
                    "summary":"用户偏好简洁回复",
                    "importance": 7,
                    "tags": ["style"]
                }),
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-02-22T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m1"}
                }),
            )
            .await
            .expect("set memory");

        let loaded = memory
            .load_memory(Some(200), vec!["style".to_string()], None)
            .await
            .expect("load memory");
        let memory_text = loaded
            .get("memory_text")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        assert!(memory_text.contains("user/preference/style"));
        assert!(memory_text.contains("用户偏好简洁回复"));
    }

    #[tokio::test]
    async fn tombstone_removes_memory_from_default_read() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        memory
            .set_memory(
                "/user/calendar/meeting",
                json!({
                    "type":"reminder",
                    "summary":"下午 3 点会议"
                }),
                json!({"kind":"user","name":"chat","retrieved_at":"2026-02-22T10:00:00Z","locator":{"message_id":"m2"}}),
            )
            .await
            .expect("set reminder");
        memory
            .set_memory(
                "/user/calendar/meeting",
                Json::Null,
                json!({"kind":"agent","name":"cleanup","retrieved_at":"2026-02-22T11:00:00Z","locator":{"reason":"done"}}),
            )
            .await
            .expect("tombstone reminder");

        let loaded = memory
            .load_memory(Some(200), vec![], None)
            .await
            .expect("load memory");
        let memory_text = loaded
            .get("memory_text")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        assert!(!memory_text.contains("user/calendar/meeting"));
    }

    #[tokio::test]
    async fn tools_register_and_set_memory_tool_is_callable() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        let tool_mgr = ToolManager::new();
        memory
            .register_tools(&tool_mgr)
            .expect("register memory tools");

        let result = tool_mgr
            .call_tool(
                &test_trace_ctx(),
                crate::agent_tool::ToolCall {
                    name: TOOL_SET_MEMORY.to_string(),
                    args: json!({
                        "key": "/agent/status/current",
                        "json_content": {
                            "type":"status",
                            "summary":"ready"
                        },
                        "source": {
                            "kind":"agent",
                            "name":"self",
                            "retrieved_at":"2026-02-22T12:00:00Z",
                            "locator":{"step":"boot"}
                        }
                    }),
                    call_id: "call-set-memory-1".to_string(),
                },
            )
            .await
            .expect("call set_memory tool");
        assert_eq!(result.get("ok").and_then(|v| v.as_bool()), Some(true));
    }
}
