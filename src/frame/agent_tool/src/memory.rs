use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::path::Component;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::{
    AgentToolError, AgentToolManager, LoadMemoryTool, MemoryLoadBackend, MemoryLoadPreview,
    MemoryMutationBackend, RemoveMemoryTool, SetMemoryTool, TOOL_LOAD_MEMORY, TOOL_REMOVE_MEMORY,
    TOOL_SET_MEMORY,
};

const DEFAULT_MEMORY_DIR_NAME: &str = "memory";
const DEFAULT_LOG_FILE_NAME: &str = "log.jsonl";
const DEFAULT_STATE_FILE_NAME: &str = "state.jsonl";
const DEFAULT_MAX_JSON_CONTENT_BYTES: usize = 256 * 1024;
const DEFAULT_TOKEN_LIMIT: u32 = 1200;

#[derive(Clone, Debug)]
pub struct AgentMemoryConfig {
    pub agent_root: PathBuf,
    pub memory_dir_name: String,
    pub log_file_name: String,
    pub state_file_name: String,
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
struct LoadMemoryRequest {
    token_limit: u32,
    tags: Vec<String>,
    current_time: DateTime<Utc>,
}

impl AgentMemory {
    pub async fn new(mut cfg: AgentMemoryConfig) -> Result<Self, AgentToolError> {
        if cfg.max_json_content_bytes == 0 {
            cfg.max_json_content_bytes = DEFAULT_MAX_JSON_CONTENT_BYTES;
        }
        if cfg.default_token_limit == 0 {
            cfg.default_token_limit = DEFAULT_TOKEN_LIMIT;
        }

        let memory_dir = cfg.agent_root.join(&cfg.memory_dir_name);
        let log_path = memory_dir.join(&cfg.log_file_name);
        let state_path = memory_dir.join(&cfg.state_file_name);
        if fs::metadata(&log_path).await.is_err() {
            info!(
                "opendan.persist_entity_prepare: kind=memory_log_file path={}",
                log_path.display()
            );
        }
        touch_file(&log_path).await?;
        if fs::metadata(&state_path).await.is_err() {
            info!(
                "opendan.persist_entity_prepare: kind=memory_state_file path={}",
                state_path.display()
            );
        }
        touch_file(&state_path).await?;

        let instance = Self {
            inner: Arc::new(AgentMemoryInner {
                cfg,
                memory_dir,
                log_path,
                state_path,
                write_lock: Mutex::new(()),
            }),
        };
        instance.bootstrap_if_needed().await?;
        Ok(instance)
    }

    pub fn memory_dir(&self) -> &Path {
        &self.inner.memory_dir
    }

    pub fn register_tools(&self, tool_mgr: &AgentToolManager) -> Result<(), AgentToolError> {
        if !tool_mgr.has_tool(TOOL_LOAD_MEMORY) {
            tool_mgr.register_tool(LoadMemoryTool::new(Arc::new(self.clone())))?;
        }
        if !tool_mgr.has_tool(TOOL_SET_MEMORY) {
            tool_mgr.register_tool(SetMemoryTool::new(Arc::new(self.clone())))?;
        }
        if !tool_mgr.has_tool(TOOL_REMOVE_MEMORY) {
            tool_mgr.register_tool(RemoveMemoryTool::new(Arc::new(self.clone())))?;
        }
        Ok(())
    }

    pub async fn remove_memory(&self, key: &str, source: Json) -> Result<Json, AgentToolError> {
        self.set_memory(key, "null", source).await
    }

    pub async fn set_memory(
        &self,
        key: &str,
        content: &str,
        source: Json,
    ) -> Result<Json, AgentToolError> {
        let normalized_key = normalize_key(key)?;
        validate_source(&normalized_key, &source)?;
        let json_content = parse_content_value(content);

        if !json_content.is_null() {
            let payload_size = serde_json::to_vec(&json_content)
                .map_err(|err| {
                    AgentToolError::ExecFailed(format!("serialize json_content failed: {err}"))
                })?
                .len();
            if payload_size > self.inner.cfg.max_json_content_bytes {
                return Err(AgentToolError::InvalidArgs(format!(
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

        let memory_path = self.memory_path_for_key(&normalized_key);
        {
            let _guard = self.inner.write_lock.lock().await;
            self.append_log_line(&envelope).await?;

            let mut current = self.read_state_map().await?;
            if envelope.valid {
                current.insert(normalized_key.clone(), envelope.clone());
                self.write_memory_content(&memory_path, content).await?;
            } else {
                current.remove(&normalized_key);
                self.remove_memory_content(&memory_path).await?;
            }

            self.write_state_map_atomic(&current).await?;
        }
        info!(
            "agent_memory.set_memory: key={} valid={} path={}",
            normalized_key,
            envelope.valid,
            memory_path.display()
        );

        Ok(json!({
            "ok": true,
            "key": normalized_key,
            "valid": envelope.valid,
            "ts": envelope.ts,
            "memory_path": memory_path.to_string_lossy(),
        }))
    }

    pub async fn load_memory(
        &self,
        token_limit: Option<u32>,
        tags: Vec<String>,
        current_time: Option<DateTime<Utc>>,
    ) -> Result<Vec<MemoryRankItem>, AgentToolError> {
        let request = LoadMemoryRequest {
            token_limit: token_limit
                .unwrap_or(self.inner.cfg.default_token_limit)
                .max(1),
            tags,
            current_time: current_time.unwrap_or_else(Utc::now),
        };
        self.load_memory_by_request(request).await
    }

    pub fn render_memory_items(items: &[MemoryRankItem]) -> String {
        items
            .iter()
            .map(render_memory_line)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub async fn compact(&self) -> Result<Json, AgentToolError> {
        let _guard = self.inner.write_lock.lock().await;
        self.compact_locked().await
    }

    async fn bootstrap_if_needed(&self) -> Result<(), AgentToolError> {
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
        let state_map = self.rebuild_state_from_log().await?;
        self.write_state_map_atomic(&state_map).await?;
        Ok(())
    }

    async fn rebuild_state_from_log(
        &self,
    ) -> Result<HashMap<String, MemoryEnvelope>, AgentToolError> {
        let content = fs::read_to_string(&self.inner.log_path)
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "read memory log failed: path={} err={err}",
                    self.inner.log_path.display()
                ))
            })?;

        let mut state_map = HashMap::<String, MemoryEnvelope>::new();
        for (idx, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<MemoryEnvelope>(trimmed) {
                Ok(envelope) => {
                    if envelope.valid {
                        let memory_path = self.memory_path_for_key(&envelope.key);
                        let raw_content = serialize_memory_content(&envelope.content)?;
                        self.write_memory_content(&memory_path, &raw_content)
                            .await?;
                        state_map.insert(envelope.key.clone(), envelope);
                    } else {
                        let memory_path = self.memory_path_for_key(&envelope.key);
                        self.remove_memory_content(&memory_path).await?;
                        state_map.remove(&envelope.key);
                    }
                }
                Err(err) => {
                    warn!(
                        "agent_memory.invalid_jsonl_line: path={} line={} err={}",
                        self.inner.log_path.display(),
                        idx + 1,
                        err
                    );
                }
            }
        }
        Ok(state_map)
    }

    async fn load_memory_by_request(
        &self,
        request: LoadMemoryRequest,
    ) -> Result<Vec<MemoryRankItem>, AgentToolError> {
        let state_map = self.read_state_map().await?;
        let tag_filters = request
            .tags
            .iter()
            .map(|tag| tag.trim().to_ascii_lowercase())
            .filter(|tag| !tag.is_empty())
            .collect::<HashSet<_>>();

        let mut candidates = Vec::<MemoryRankItem>::new();
        for envelope in state_map.into_values() {
            if !envelope.valid {
                continue;
            }
            let tags = extract_tags(&envelope.content);
            if !tag_filters.is_empty()
                && tags
                    .iter()
                    .all(|tag| !tag_filters.contains(&tag.to_ascii_lowercase()))
            {
                continue;
            }

            let recency_hours = parse_rfc3339(&envelope.ts)
                .map(|ts| {
                    request
                        .current_time
                        .signed_duration_since(ts)
                        .num_hours()
                        .max(0)
                })
                .unwrap_or(24);

            let importance = extract_importance(&envelope.content);
            let type_name = extract_type_name(&envelope.content);
            let summary = extract_summary_text(&envelope.content);
            let tag_score = tags
                .iter()
                .filter(|tag| tag_filters.contains(&tag.to_ascii_lowercase()))
                .count() as u32;
            let ts_unix_ms = parse_rfc3339(&envelope.ts)
                .map(|ts| ts.timestamp_millis())
                .unwrap_or_default();
            let token_estimate = estimate_token_count(
                render_memory_line(&MemoryRankItem {
                    key: envelope.key.clone(),
                    ts: envelope.ts.clone(),
                    source: envelope.source.clone(),
                    content: envelope.content.clone(),
                    importance,
                    recency_hours,
                    token_estimate: 0,
                    tags: tags.clone(),
                    type_name: type_name.clone(),
                    summary: summary.clone(),
                    tag_score,
                    ts_unix_ms,
                })
                .as_str(),
            );
            candidates.push(MemoryRankItem {
                key: envelope.key,
                ts: envelope.ts,
                source: envelope.source,
                content: envelope.content,
                importance,
                recency_hours,
                token_estimate,
                tags,
                type_name,
                summary,
                tag_score,
                ts_unix_ms,
            });
        }

        candidates.sort_by(rank_candidates);
        let mut selected = Vec::<MemoryRankItem>::new();
        let mut used_tokens = 0_usize;
        let mut truncated = false;
        for item in candidates.into_iter() {
            let next_total = used_tokens.saturating_add(item.token_estimate);
            if next_total > request.token_limit as usize {
                truncated = true;
                break;
            }
            used_tokens = next_total;
            selected.push(item);
        }

        debug!(
            "agent_memory.load_memory: token_limit={} selected={} total={} tags={}",
            request.token_limit,
            selected.len(),
            used_tokens,
            request.tags.join(",")
        );
        if truncated {
            debug!(
                "agent_memory.load_memory truncated: selected={} total={} token_estimate={}",
                selected.len(),
                used_tokens,
                request.token_limit,
            );
        }
        Ok(selected)
    }

    async fn compact_locked(&self) -> Result<Json, AgentToolError> {
        let state_map = self.read_state_map().await?;
        let mut values = state_map.into_values().collect::<Vec<_>>();
        values.sort_by(|a, b| a.key.cmp(&b.key).then(a.ts.cmp(&b.ts)));

        let mut body = String::new();
        for value in &values {
            let line = serde_json::to_string(value).map_err(|err| {
                AgentToolError::ExecFailed(format!("serialize compacted memory line failed: {err}"))
            })?;
            body.push_str(&line);
            body.push('\n');
        }

        atomic_write(&self.inner.log_path, body.as_bytes()).await?;
        self.write_state_map_atomic(
            &values
                .into_iter()
                .map(|item| (item.key.clone(), item))
                .collect::<HashMap<_, _>>(),
        )
        .await?;

        Ok(json!({
            "ok": true,
            "entries": self.read_state_map().await?.len(),
        }))
    }

    async fn append_log_line(&self, envelope: &MemoryEnvelope) -> Result<(), AgentToolError> {
        let line = serde_json::to_string(envelope).map_err(|err| {
            AgentToolError::ExecFailed(format!("serialize memory log line failed: {err}"))
        })?;
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.inner.log_path)
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "open memory log failed: path={} err={err}",
                    self.inner.log_path.display()
                ))
            })?;
        file.write_all(line.as_bytes()).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "append memory log failed: path={} err={err}",
                self.inner.log_path.display()
            ))
        })?;
        file.write_all(b"\n").await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "append memory log newline failed: path={} err={err}",
                self.inner.log_path.display()
            ))
        })?;
        file.flush().await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "flush memory log failed: path={} err={err}",
                self.inner.log_path.display()
            ))
        })?;
        Ok(())
    }

    async fn read_state_map(&self) -> Result<HashMap<String, MemoryEnvelope>, AgentToolError> {
        let content = fs::read_to_string(&self.inner.state_path)
            .await
            .map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "read memory state failed: path={} err={err}",
                    self.inner.state_path.display()
                ))
            })?;
        let mut out = HashMap::<String, MemoryEnvelope>::new();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let envelope = serde_json::from_str::<MemoryEnvelope>(trimmed).map_err(|err| {
                AgentToolError::ExecFailed(format!(
                    "parse memory state failed: path={} err={err}",
                    self.inner.state_path.display()
                ))
            })?;
            out.insert(envelope.key.clone(), envelope);
        }
        Ok(out)
    }

    async fn write_state_map_atomic(
        &self,
        map: &HashMap<String, MemoryEnvelope>,
    ) -> Result<(), AgentToolError> {
        let mut values = map.values().cloned().collect::<Vec<_>>();
        values.sort_by(|a, b| a.key.cmp(&b.key).then(a.ts.cmp(&b.ts)));

        let mut body = String::new();
        for value in values {
            let line = serde_json::to_string(&value).map_err(|err| {
                AgentToolError::ExecFailed(format!("serialize memory state line failed: {err}"))
            })?;
            body.push_str(&line);
            body.push('\n');
        }
        atomic_write(&self.inner.state_path, body.as_bytes()).await
    }

    async fn write_memory_content(&self, path: &Path, content: &str) -> Result<(), AgentToolError> {
        atomic_write(path, content.as_bytes()).await
    }

    async fn remove_memory_content(&self, path: &Path) -> Result<(), AgentToolError> {
        match fs::remove_file(path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(AgentToolError::ExecFailed(format!(
                "remove memory content file failed: path={} err={err}",
                path.display()
            ))),
        }
    }

    fn memory_path_for_key(&self, key: &str) -> PathBuf {
        let raw = key.trim().trim_start_matches('/');
        match parse_memory_relative_path(
            raw,
            [
                self.inner.cfg.log_file_name.as_str(),
                self.inner.cfg.state_file_name.as_str(),
            ],
        ) {
            Some(path) => self.inner.memory_dir.join(path),
            None => self.inner.memory_dir.join(flat_memory_file_name(raw)),
        }
    }
}

#[async_trait]
impl MemoryLoadBackend for AgentMemory {
    async fn load_memory_preview(
        &self,
        token_limit: Option<u32>,
        tags: Vec<String>,
        current_time: Option<String>,
    ) -> Result<MemoryLoadPreview, AgentToolError> {
        let current_time = current_time.as_deref().map(parse_rfc3339).transpose()?;
        let items = self.load_memory(token_limit, tags, current_time).await?;
        Ok(MemoryLoadPreview {
            rendered: Self::render_memory_items(&items),
            item_count: items.len(),
        })
    }
}

#[async_trait]
impl MemoryMutationBackend for AgentMemory {
    async fn set_memory(
        &self,
        key: String,
        content: String,
        source: Json,
    ) -> Result<Json, AgentToolError> {
        self.set_memory(key.as_str(), content.as_str(), source)
            .await
    }

    async fn remove_memory(&self, key: String, source: Json) -> Result<Json, AgentToolError> {
        self.set_memory(key.as_str(), "null", source).await
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct MemoryRankItem {
    pub key: String,
    pub ts: String,
    pub source: Json,
    pub content: Json,
    pub importance: i64,
    pub recency_hours: i64,
    pub token_estimate: usize,
    pub tags: Vec<String>,
    pub type_name: String,
    pub summary: String,
    pub tag_score: u32,
    pub ts_unix_ms: i64,
}

fn rank_candidates(a: &MemoryRankItem, b: &MemoryRankItem) -> Ordering {
    b.ts_unix_ms
        .cmp(&a.ts_unix_ms)
        .then_with(|| b.importance.cmp(&a.importance))
        .then_with(|| b.tag_score.cmp(&a.tag_score))
        .then(a.key.cmp(&b.key))
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
    content
        .get("importance")
        .and_then(Json::as_i64)
        .unwrap_or(0)
}

fn extract_tags(content: &Json) -> Vec<String> {
    content
        .get("tags")
        .and_then(Json::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Json::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn extract_type_name(content: &Json) -> String {
    content
        .get("type")
        .and_then(Json::as_str)
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_default()
}

fn extract_summary_text(content: &Json) -> String {
    if let Some(summary) = content
        .get("summary")
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        return summary.to_string();
    }
    if let Some(text) = content
        .get("text")
        .and_then(Json::as_str)
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

fn parse_rfc3339(raw: &str) -> Result<DateTime<Utc>, AgentToolError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| AgentToolError::InvalidArgs(format!("invalid RFC3339 timestamp: {err}")))
}

fn parse_content_value(content: &str) -> Json {
    serde_json::from_str::<Json>(content).unwrap_or_else(|_| Json::String(content.to_string()))
}

fn serialize_memory_content(content: &Json) -> Result<String, AgentToolError> {
    match content {
        Json::String(text) => Ok(text.clone()),
        other => serde_json::to_string(other).map_err(|err| {
            AgentToolError::ExecFailed(format!("serialize memory content failed: {err}"))
        }),
    }
}

fn estimate_token_count(text: &str) -> usize {
    text.chars().count().div_ceil(4).max(1)
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut chars = text.chars();
    for _ in 0..max_chars {
        let Some(ch) = chars.next() else {
            return text.to_string();
        };
        out.push(ch);
    }
    if chars.next().is_some() {
        out.push_str("...");
    }
    out
}

fn validate_source(key: &str, source: &Json) -> Result<(), AgentToolError> {
    let Some(map) = source.as_object() else {
        return Err(AgentToolError::InvalidArgs(format!(
            "memory source for `{key}` must be object"
        )));
    };
    for required in ["kind", "name", "retrieved_at", "locator"] {
        if !map.contains_key(required) {
            return Err(AgentToolError::InvalidArgs(format!(
                "memory source for `{key}` missing required field `{required}`"
            )));
        }
    }
    Ok(())
}

fn normalize_key(key: &str) -> Result<String, AgentToolError> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "memory key cannot be empty".to_string(),
        ));
    }
    let normalized = if trimmed.starts_with('/') {
        trimmed.to_string()
    } else {
        format!("/{trimmed}")
    };
    Ok(normalized)
}

fn parse_memory_relative_path<'a, I>(raw: &str, reserved_roots: I) -> Option<PathBuf>
where
    I: IntoIterator<Item = &'a str>,
{
    if raw.is_empty() {
        return None;
    }

    let reserved_roots = reserved_roots.into_iter().collect::<HashSet<_>>();
    let segments = raw.split('/').collect::<Vec<_>>();
    if segments.iter().any(|segment| segment.is_empty()) {
        return None;
    }
    if reserved_roots.contains(segments[0]) {
        return None;
    }

    let mut path = PathBuf::new();
    for segment in segments {
        let segment_path = Path::new(segment);
        let mut components = segment_path.components();
        match components.next() {
            Some(Component::Normal(_)) if components.next().is_none() => path.push(segment),
            _ => return None,
        }
    }
    Some(path)
}

fn flat_memory_file_name(raw: &str) -> String {
    format!(".memory-key-{}", hex::encode(raw.as_bytes()))
}

async fn touch_file(path: &Path) -> Result<(), AgentToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create parent dir failed: path={} err={err}",
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
            AgentToolError::ExecFailed(format!(
                "touch file failed: path={} err={err}",
                path.display()
            ))
        })?;
    Ok(())
}

async fn file_len_or_zero(path: &Path) -> u64 {
    fs::metadata(path).await.map(|meta| meta.len()).unwrap_or(0)
}

async fn atomic_write(path: &Path, body: &[u8]) -> Result<(), AgentToolError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await.map_err(|err| {
            AgentToolError::ExecFailed(format!(
                "create parent dir failed: path={} err={err}",
                parent.display()
            ))
        })?;
    }

    let tmp_name = format!(
        ".{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("memory")
    );
    let tmp_path = path
        .parent()
        .map(|p| p.join(&tmp_name))
        .unwrap_or_else(|| PathBuf::from(tmp_name));

    fs::write(&tmp_path, body).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
            "write temporary file failed: path={}, err={err}",
            tmp_path.display()
        ))
    })?;
    fs::rename(&tmp_path, path).await.map_err(|err| {
        AgentToolError::ExecFailed(format!(
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
    use crate::SessionRuntimeContext;
    use buckyos_api::{value_to_object_map, AiToolCall};
    use tempfile::tempdir;

    fn test_trace_ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "trace-memory".to_string(),
            agent_name: "did:example:agent".to_string(),
            behavior: "on_wakeup".to_string(),
            step_idx: 0,
            wakeup_id: "wakeup-memory".to_string(),
            session_id: "session-memory".to_string(),
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
                r#"{"type":"preference","summary":"用户偏好简洁回复","importance":7,"tags":["style"]}"#,
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
        let memory_text = AgentMemory::render_memory_items(&loaded);
        assert!(memory_text.contains("user/preference/style"));
        assert!(memory_text.contains("用户偏好简洁回复"));
    }

    #[tokio::test]
    async fn set_memory_without_leading_slash_is_normalized() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        let result = memory
            .set_memory(
                "user_profile/location",
                r#"{"type":"profile","summary":"用户居住在 Cupertino"}"#,
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-02-22T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m2"}
                }),
            )
            .await
            .expect("set memory");

        assert_eq!(result["key"], "/user_profile/location");

        let loaded = memory
            .load_memory(Some(200), vec![], None)
            .await
            .expect("load memory");
        let memory_text = AgentMemory::render_memory_items(&loaded);
        assert!(memory_text.contains("user_profile/location"));
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
                r#"{"type":"reminder","summary":"下午 3 点会议"}"#,
                json!({"kind":"user","name":"chat","retrieved_at":"2026-02-22T10:00:00Z","locator":{"message_id":"m2"}}),
            )
            .await
            .expect("set reminder");
        memory
            .set_memory(
                "/user/calendar/meeting",
                "null",
                json!({"kind":"agent","name":"cleanup","retrieved_at":"2026-02-22T11:00:00Z","locator":{"reason":"done"}}),
            )
            .await
            .expect("tombstone reminder");

        let loaded = memory
            .load_memory(Some(200), vec![], None)
            .await
            .expect("load memory");
        let memory_text = AgentMemory::render_memory_items(&loaded);

        assert!(!memory_text.contains("user/calendar/meeting"));
        assert!(
            fs::metadata(memory.memory_path_for_key("/user/calendar/meeting"))
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn remove_memory_delegates_to_set_memory_and_deletes_file() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        memory
            .set_memory(
                "/user/calendar/meeting",
                "weekly meeting",
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-03-23T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m5"}
                }),
            )
            .await
            .expect("set memory");
        let memory_path = memory.memory_path_for_key("/user/calendar/meeting");
        assert!(fs::metadata(&memory_path).await.is_ok());

        let result = memory
            .remove_memory(
                "/user/calendar/meeting",
                json!({
                    "kind":"tool",
                    "name":"remove_memory",
                    "retrieved_at":"2026-03-23T10:01:00Z",
                    "locator":{"call_id":"rm-1"}
                }),
            )
            .await
            .expect("remove memory");

        assert_eq!(result["valid"], false);
        assert!(fs::metadata(&memory_path).await.is_err());
    }

    #[tokio::test]
    async fn set_memory_creates_nested_directories_from_key() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        memory
            .set_memory(
                "remind/bob/20260323",
                "weekly meeting",
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-03-23T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m3"}
                }),
            )
            .await
            .expect("set nested memory");

        let memory_path = memory.memory_path_for_key("/remind/bob/20260323");
        let written = fs::read_to_string(&memory_path)
            .await
            .expect("read nested memory file");
        assert_eq!(written, "weekly meeting");
        assert!(memory_path.ends_with("remind/bob/20260323"));
    }

    #[tokio::test]
    async fn invalid_relative_path_key_falls_back_to_single_file() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        memory
            .set_memory(
                "remind/../20260323",
                "weekly meeting",
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-03-23T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m4"}
                }),
            )
            .await
            .expect("set fallback memory");

        let memory_path = memory.memory_path_for_key("/remind/../20260323");
        let written = fs::read_to_string(&memory_path)
            .await
            .expect("read fallback memory file");
        assert_eq!(written, "weekly meeting");
        assert_eq!(
            memory_path.parent().expect("fallback file parent"),
            memory.memory_dir()
        );
        assert!(memory_path
            .file_name()
            .is_some_and(|name| { name.to_string_lossy().starts_with(".memory-key-") }));
    }

    #[tokio::test]
    async fn index_prefix_is_now_available_as_regular_memory_path() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        memory
            .set_memory(
                "index/demo",
                "usable path",
                json!({
                    "kind":"user",
                    "name":"chat",
                    "retrieved_at":"2026-03-23T10:00:00Z",
                    "locator":{"conversation_id":"c1","message_id":"m6"}
                }),
            )
            .await
            .expect("set memory under former index prefix");

        let memory_path = memory.memory_path_for_key("/index/demo");
        let written = fs::read_to_string(&memory_path)
            .await
            .expect("read former index-prefix memory file");
        assert_eq!(written, "usable path");
        assert!(memory_path.ends_with("index/demo"));
    }

    #[tokio::test]
    async fn tools_register_and_load_memory_tool_is_callable() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");
        memory
            .set_memory(
                "/agent/status/current",
                r#"{"type":"status","summary":"ready"}"#,
                json!({
                    "kind":"agent",
                    "name":"self",
                    "retrieved_at":"2026-02-22T12:00:00Z",
                    "locator":{"step":"boot"}
                }),
            )
            .await
            .expect("set memory");

        let tool_mgr = AgentToolManager::new();
        memory
            .register_tools(&tool_mgr)
            .expect("register memory tools");

        let result = tool_mgr
            .call_tool(
                &test_trace_ctx(),
                AiToolCall {
                    name: TOOL_LOAD_MEMORY.to_string(),
                    args: value_to_object_map(json!({
                        "token_limit": 200
                    })),
                    call_id: "call-load-memory-1".to_string(),
                },
            )
            .await
            .expect("call load_memory tool");
        assert!(result.is_agent_tool);
        let memory_text = result.as_str().expect("load_memory returns string");
        assert!(memory_text.contains("agent/status/current"));
    }

    #[tokio::test]
    async fn remove_memory_tool_is_callable_and_deletes_file() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");
        memory
            .set_memory(
                "/agent/status/current",
                "ready",
                json!({
                    "kind":"agent",
                    "name":"self",
                    "retrieved_at":"2026-02-22T12:00:00Z",
                    "locator":{"step":"boot"}
                }),
            )
            .await
            .expect("set memory");

        let memory_path = memory.memory_path_for_key("/agent/status/current");
        assert!(fs::metadata(&memory_path).await.is_ok());

        let tool_mgr = AgentToolManager::new();
        memory
            .register_tools(&tool_mgr)
            .expect("register memory tools");

        let result = tool_mgr
            .call_tool(
                &test_trace_ctx(),
                AiToolCall {
                    name: TOOL_REMOVE_MEMORY.to_string(),
                    args: value_to_object_map(json!({
                        "key": "/agent/status/current"
                    })),
                    call_id: "call-remove-memory-1".to_string(),
                },
            )
            .await
            .expect("call remove_memory tool");
        assert!(result.is_agent_tool);
        assert_eq!(result.details["valid"], false);
        assert!(fs::metadata(&memory_path).await.is_err());
    }

    #[tokio::test]
    async fn load_memory_truncates_when_token_limit_is_small() {
        let temp = tempdir().expect("create tempdir");
        let root = temp.path().to_path_buf();
        let memory = AgentMemory::new(AgentMemoryConfig::new(&root))
            .await
            .expect("create memory");

        for i in 0..60_u32 {
            let source = json!({
                "kind":"user",
                "name":"chat",
                "retrieved_at":"2026-02-22T10:00:00Z",
                "locator":{"conversation_id":"c1","message_id":format!("m{i}")}
            });
            let content = json!({
                "type":"note",
                "summary": format!("条目{i:02}摘要"),
                "importance": (100 - i) as i64,
                "tags": ["bulk", "trim"]
            })
            .to_string();
            memory
                .set_memory(
                    format!("/user/bulk/item_{i:02}").as_str(),
                    content.as_str(),
                    source,
                )
                .await
                .expect("set bulk memory");
        }

        let all = memory
            .load_memory(Some(20_000), vec!["trim".to_string()], None)
            .await
            .expect("load all memory with huge token limit");
        let all = AgentMemory::render_memory_items(&all);

        let all_lines = all
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        assert!(
            all_lines.len() >= 40,
            "need at least 40 lines for this test, got {}",
            all_lines.len()
        );

        let token_limit_40 = all_lines
            .iter()
            .take(40)
            .map(|line| estimate_token_count(line))
            .sum::<usize>() as u32;
        let token_limit_20 = all_lines
            .iter()
            .take(20)
            .map(|line| estimate_token_count(line))
            .sum::<usize>() as u32;

        let forty = memory
            .load_memory(Some(token_limit_40), vec!["trim".to_string()], None)
            .await
            .expect("load memory with token limit for 40 lines");
        let forty = AgentMemory::render_memory_items(&forty);
        let twenty = memory
            .load_memory(Some(token_limit_20), vec!["trim".to_string()], None)
            .await
            .expect("load memory with token limit for 20 lines");
        let twenty = AgentMemory::render_memory_items(&twenty);

        let forty_lines = forty
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();
        let twenty_lines = twenty
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>();

        assert!(
            forty_lines.len() == 40,
            "expected 40 lines, got {} (token_limit={})",
            forty_lines.len(),
            token_limit_40
        );
        assert!(
            twenty_lines.len() == 20,
            "expected 20 lines, got {} (token_limit={})",
            twenty_lines.len(),
            token_limit_20
        );
        assert!(
            twenty_lines.iter().all(|line| forty_lines.contains(line)),
            "20-line output should be subset of 40-line output"
        );
    }
}
