use crate::{external_command, ControlPanelServer};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use base64::{engine::general_purpose, Engine as _};
use bytes::Bytes;
use chrono::{DateTime, Datelike, NaiveDateTime, TimeZone, Utc};
use cyfs_gateway_lib::*;
use http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_TYPE};
use http_body_util::{combinators::BoxBody, BodyExt, Full};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use tokio::task;
use uuid::Uuid;
use zip::write::FileOptions;
use zip::CompressionMethod;

pub(crate) const LOG_ROOT_DIR: &str = "/opt/buckyos/logs";
pub(crate) const LOG_DOWNLOAD_TTL_SECS: u64 = 600;
const DEFAULT_LOG_LIMIT: usize = 200;
const MAX_LOG_LIMIT: usize = 1000;

#[derive(Clone, Serialize, Deserialize)]
struct LogQueryCursor {
    service: String,
    file: String,
    line_index: u64,
    direction: String,
}

#[derive(Clone, Serialize, Deserialize)]
struct LogTailCursor {
    file: String,
    offset: u64,
}

pub(crate) struct LogDownloadEntry {
    pub(crate) path: PathBuf,
    pub(crate) filename: String,
    pub(crate) expires_at: std::time::SystemTime,
}

pub(crate) struct LogFileRef {
    pub(crate) service: String,
    pub(crate) name: String,
    pub(crate) path: PathBuf,
    pub(crate) modified: std::time::SystemTime,
}

type LogCursorContext = (String, String);
type ParsedLogEntry = (String, String, String, Option<LogCursorContext>);

impl ControlPanelServer {
    pub(crate) fn encode_cursor<T: Serialize>(value: &T) -> String {
        let payload = serde_json::to_vec(value).unwrap_or_default();
        general_purpose::STANDARD.encode(payload)
    }

    pub(crate) fn decode_cursor<T: DeserializeOwned>(value: &str) -> Option<T> {
        let decoded = general_purpose::STANDARD.decode(value).ok()?;
        serde_json::from_slice(&decoded).ok()
    }

    fn normalize_log_level(value: &str) -> String {
        match value.to_uppercase().as_str() {
            "INFO" => "info".to_string(),
            "WARN" | "WARNING" => "warning".to_string(),
            "ERROR" => "error".to_string(),
            other => other.to_lowercase(),
        }
    }

    pub(crate) fn split_log_line(line: &str) -> (String, String, String) {
        let trimmed = line.trim_start().trim_end();
        if let Some(bracket_start) = trimmed.find('[') {
            if let Some(bracket_end) = trimmed[bracket_start + 1..].find(']') {
                let ts_candidate = trimmed[..bracket_start].trim_end();
                let level = trimmed[bracket_start + 1..bracket_start + 1 + bracket_end].trim();
                let message = trimmed[bracket_start + 1 + bracket_end + 1..].trim_start();
                if !ts_candidate.is_empty() {
                    return (
                        ts_candidate.to_string(),
                        Self::normalize_log_level(level),
                        message.to_string(),
                    );
                }
            }
        }
        ("".to_string(), "unknown".to_string(), trimmed.to_string())
    }

    fn extract_log_entry(raw: &str, context: Option<&LogCursorContext>) -> Option<ParsedLogEntry> {
        let trimmed = raw.trim_end();
        if trimmed.is_empty() {
            return None;
        }
        let (ts, level, message) = Self::split_log_line(trimmed);
        if !ts.is_empty() {
            let normalized_level = if level == "unknown" {
                "info".to_string()
            } else {
                level
            };
            let msg = if message.is_empty() {
                trimmed.to_string()
            } else {
                message
            };
            return Some((
                ts.clone(),
                normalized_level.clone(),
                msg,
                Some((ts, normalized_level)),
            ));
        }
        if let Some((ctx_ts, ctx_level)) = context {
            return Some((
                ctx_ts.clone(),
                ctx_level.clone(),
                trimmed.trim().to_string(),
                None,
            ));
        }
        None
    }

    pub(crate) fn parse_log_timestamp(value: &str) -> Option<DateTime<Utc>> {
        if value.is_empty() {
            return None;
        }
        let year = Utc::now().year();
        let with_year = format!("{}-{}", year, value);
        let parsed = NaiveDateTime::parse_from_str(&with_year, "%Y-%m-%d %H:%M:%S%.3f").ok()?;
        Some(Utc.from_utc_datetime(&parsed))
    }

    fn parse_filter_time(value: &str) -> Option<DateTime<Utc>> {
        if let Ok(parsed) = DateTime::parse_from_rfc3339(value) {
            return Some(parsed.with_timezone(&Utc));
        }
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S%.3f") {
            return Some(Utc.from_utc_datetime(&parsed));
        }
        if let Ok(parsed) = NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S") {
            return Some(Utc.from_utc_datetime(&parsed));
        }
        if NaiveDateTime::parse_from_str(value, "%m-%d %H:%M:%S%.3f").is_ok() {
            let year = Utc::now().year();
            let with_year = format!("{}-{}", year, value);
            let parsed = NaiveDateTime::parse_from_str(&with_year, "%Y-%m-%d %H:%M:%S%.3f").ok()?;
            return Some(Utc.from_utc_datetime(&parsed));
        }
        None
    }

    fn format_log_filter_key(value: &DateTime<Utc>) -> String {
        value.format("%m-%d %H:%M:%S%.3f").to_string()
    }

    fn rg_available() -> bool {
        external_command("rg")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false)
    }

    fn rg_search_lines(path: &Path, keyword: &str) -> Result<Vec<(u64, String)>, RPCErrors> {
        let output = external_command("rg")
            .arg("--line-number")
            .arg("--fixed-strings")
            .arg("--no-heading")
            .arg("--no-filename")
            .arg("--color")
            .arg("never")
            .arg("-i")
            .arg(keyword)
            .arg(path)
            .output()
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to run rg: {}", err)))?;

        if !output.status.success() {
            if output.status.code() == Some(1) {
                return Ok(Vec::new());
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(RPCErrors::ReasonError(format!(
                "rg failed for {}: {}",
                path.display(),
                stderr
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut results = Vec::new();
        for line in stdout.lines() {
            let mut parts = line.splitn(2, ':');
            let line_no = parts.next().unwrap_or("");
            let content = parts.next().unwrap_or("").to_string();
            if let Ok(number) = line_no.parse::<u64>() {
                let line_index = number.saturating_sub(1);
                results.push((line_index, content));
            }
        }
        Ok(results)
    }

    pub(crate) fn list_log_service_ids(&self) -> Result<Vec<String>, RPCErrors> {
        let mut services = Vec::new();
        let entries = std::fs::read_dir(LOG_ROOT_DIR)
            .map_err(|err| RPCErrors::ReasonError(format!("Failed to read log root: {}", err)))?;
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type() {
                if file_type.is_dir() {
                    if let Some(name) = entry.file_name().to_str() {
                        services.push(name.to_string());
                    }
                }
            }
        }
        services.sort();
        Ok(services)
    }

    pub(crate) fn format_log_service_label(name: &str) -> String {
        name.split(['_', '-'])
            .filter(|part| !part.is_empty())
            .map(|part| {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                    None => "".to_string(),
                }
            })
            .collect::<Vec<String>>()
            .join(" ")
    }

    pub(crate) fn collect_log_files(
        &self,
        service: &str,
        file_filter: Option<&str>,
    ) -> Result<Vec<LogFileRef>, RPCErrors> {
        let mut files = Vec::new();
        let dir_path = Path::new(LOG_ROOT_DIR).join(service);
        let entries = std::fs::read_dir(&dir_path).map_err(|err| {
            RPCErrors::ReasonError(format!("Failed to read log dir {}: {}", service, err))
        })?;

        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(file_type) = entry.file_type() {
                if !file_type.is_file() {
                    continue;
                }
            }
            let name = match path.file_name().and_then(|value| value.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };
            if let Some(filter) = file_filter {
                if name != filter {
                    continue;
                }
            }
            let modified = std::fs::metadata(&path)
                .and_then(|meta| meta.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            files.push(LogFileRef {
                service: service.to_string(),
                name,
                path,
                modified,
            });
        }

        files.sort_by(|a, b| b.modified.cmp(&a.modified));
        Ok(files)
    }

    pub(crate) async fn cleanup_log_downloads(&self) {
        let mut downloads = self.log_downloads.lock().await;
        let now = std::time::SystemTime::now();
        let mut expired: Vec<PathBuf> = Vec::new();
        downloads.retain(|_, entry| {
            if entry.expires_at <= now {
                expired.push(entry.path.clone());
                false
            } else {
                true
            }
        });
        for path in expired {
            let _ = std::fs::remove_file(path);
        }
    }

    pub(crate) async fn handle_system_logs_list(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let services = self.list_log_service_ids()?;
        let items: Vec<Value> = services
            .iter()
            .map(|service| {
                json!({
                    "id": service,
                    "label": Self::format_log_service_label(service),
                    "path": format!("{}/{}", LOG_ROOT_DIR, service),
                })
            })
            .collect();

        Ok(RPCResponse::new(
            RPCResult::Success(json!({ "services": items })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_system_logs_query(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let mut services: Vec<String> = req
            .params
            .get("services")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        if services.is_empty() {
            if let Some(service) = Self::param_str(&req, "service") {
                services.push(service);
            }
        }
        if services.is_empty() {
            return Err(RPCErrors::ParseRequestError("Missing service".to_string()));
        }

        let available = self.list_log_service_ids()?;
        for service in services.iter() {
            if !available.contains(service) {
                return Err(RPCErrors::ReasonError(format!(
                    "Unknown log service: {}",
                    service
                )));
            }
        }

        let file_filter = Self::param_str(&req, "file");
        let direction = Self::param_str(&req, "direction").unwrap_or_else(|| "forward".to_string());
        let direction = if direction == "backward" {
            "backward".to_string()
        } else {
            "forward".to_string()
        };
        let level_filter = Self::param_str(&req, "level").map(|value| value.to_lowercase());
        let keyword_raw = Self::param_str(&req, "keyword");
        let keyword_filter = keyword_raw.as_ref().map(|value| value.to_lowercase());
        let since_filter =
            Self::param_str(&req, "since").and_then(|value| Self::parse_filter_time(&value));
        let until_filter =
            Self::param_str(&req, "until").and_then(|value| Self::parse_filter_time(&value));
        let since_key = since_filter.as_ref().map(Self::format_log_filter_key);
        let until_key = until_filter.as_ref().map(Self::format_log_filter_key);
        let limit = req
            .params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_LOG_LIMIT as u64)
            .clamp(1, MAX_LOG_LIMIT as u64) as usize;
        let cursor = Self::param_str(&req, "cursor")
            .and_then(|value| Self::decode_cursor::<LogQueryCursor>(&value));

        let mut files: Vec<LogFileRef> = Vec::new();
        for service in services.iter() {
            files.extend(self.collect_log_files(service, file_filter.as_deref())?);
        }
        files.sort_by(|a, b| b.modified.cmp(&a.modified));

        let cursor = cursor.and_then(|value| {
            if value.direction != direction {
                return None;
            }
            if files
                .iter()
                .any(|file| file.service == value.service && file.name == value.file)
            {
                Some(value)
            } else {
                None
            }
        });

        if direction == "backward" {
            let mut collected: Vec<Value> = Vec::new();
            let mut has_more = false;
            let mut next_cursor: Option<LogQueryCursor> = None;
            let use_rg = keyword_raw.as_ref().is_some() && Self::rg_available();
            let cursor_index = cursor.as_ref().and_then(|value| {
                files
                    .iter()
                    .position(|file| file.service == value.service && file.name == value.file)
            });

            for (file_index, file) in files.iter().enumerate() {
                if let Some(cursor_index) = cursor_index {
                    if file_index < cursor_index {
                        continue;
                    }
                }

                let mut candidates: Vec<(u64, String, String, String, String)> = Vec::new();
                let mut rg_used = false;
                if use_rg {
                    let keyword = keyword_raw.as_ref().unwrap();
                    match Self::rg_search_lines(&file.path, keyword) {
                        Ok(matched_lines) => {
                            rg_used = true;
                            for (line_index, raw) in matched_lines.into_iter() {
                                let (ts, level, message) = Self::split_log_line(&raw);
                                if ts.is_empty() {
                                    continue;
                                }
                                if let Some(filter) = level_filter.as_ref() {
                                    if &level != filter {
                                        continue;
                                    }
                                }
                                if since_key.is_some() || until_key.is_some() {
                                    if ts.is_empty() {
                                        continue;
                                    }
                                    if let Some(since) = since_key.as_ref() {
                                        if ts < *since {
                                            continue;
                                        }
                                    }
                                    if let Some(until) = until_key.as_ref() {
                                        if ts > *until {
                                            continue;
                                        }
                                    }
                                }
                                candidates.push((line_index, ts, level, message, raw));
                            }
                        }
                        Err(err) => {
                            log::warn!("rg failed for {}: {}", file.name, err);
                        }
                    }
                }

                if !rg_used {
                    let mut last_context: Option<(String, String)> = None;
                    let file_handle = std::fs::File::open(&file.path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to open log file {}: {}",
                            file.name, err
                        ))
                    })?;
                    let reader = BufReader::new(file_handle);
                    for (index, line) in reader.lines().enumerate() {
                        let raw = match line {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let maybe_entry = Self::extract_log_entry(&raw, last_context.as_ref());
                        let (ts, level, message, raw_line) = match maybe_entry {
                            Some((ts, level, message, next_context)) => {
                                if let Some(context) = next_context {
                                    last_context = Some(context);
                                }
                                (ts, level, message, raw.trim_end().to_string())
                            }
                            None => continue,
                        };
                        if let Some(filter) = level_filter.as_ref() {
                            if &level != filter {
                                continue;
                            }
                        }
                        if let Some(filter) = keyword_filter.as_ref() {
                            if !raw_line.to_lowercase().contains(filter) {
                                continue;
                            }
                        }
                        if since_key.is_some() || until_key.is_some() {
                            if ts.is_empty() {
                                continue;
                            }
                            if let Some(since) = since_key.as_ref() {
                                if ts < *since {
                                    continue;
                                }
                            }
                            if let Some(until) = until_key.as_ref() {
                                if ts > *until {
                                    continue;
                                }
                            }
                        }
                        candidates.push((index as u64, ts, level, message, raw_line));
                    }
                }

                if let Some(cursor) = cursor.as_ref() {
                    if cursor.service == file.service && cursor.file == file.name {
                        candidates
                            .retain(|(line_index, _, _, _, _)| *line_index < cursor.line_index);
                    }
                }

                for (line_index, ts, level, message, raw) in candidates.into_iter().rev() {
                    collected.push(json!({
                        "timestamp": ts,
                        "level": level,
                        "message": message,
                        "raw": raw,
                        "service": file.service.clone(),
                        "file": file.name.clone(),
                        "line": line_index,
                    }));

                    if collected.len() >= limit {
                        has_more = true;
                        next_cursor = Some(LogQueryCursor {
                            service: file.service.clone(),
                            file: file.name.clone(),
                            line_index,
                            direction: direction.clone(),
                        });
                        break;
                    }
                }

                if has_more {
                    break;
                }
            }

            collected.reverse();
            Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "entries": collected,
                    "hasMore": has_more,
                    "nextCursor": next_cursor.map(|value| Self::encode_cursor(&value)),
                })),
                req.seq,
            ))
        } else {
            let mut entries: Vec<Value> = Vec::new();
            let mut has_more = false;
            let mut next_cursor: Option<LogQueryCursor> = None;
            let mut reached_cursor = cursor.is_none();
            let use_rg = keyword_raw.as_ref().is_some() && Self::rg_available();

            for file in files.iter() {
                let mut rg_used = false;
                if use_rg {
                    let keyword = keyword_raw.as_ref().unwrap();
                    match Self::rg_search_lines(&file.path, keyword) {
                        Ok(matched_lines) => {
                            rg_used = true;
                            for (line_index, raw) in matched_lines.into_iter() {
                                if !reached_cursor {
                                    if let Some(cursor) = cursor.as_ref() {
                                        if cursor.service == file.service
                                            && cursor.file == file.name
                                        {
                                            if line_index <= cursor.line_index {
                                                continue;
                                            }
                                            reached_cursor = true;
                                        } else {
                                            continue;
                                        }
                                    }
                                }

                                let (ts, level, message) = Self::split_log_line(&raw);
                                if ts.is_empty() {
                                    continue;
                                }
                                if let Some(filter) = level_filter.as_ref() {
                                    if &level != filter {
                                        continue;
                                    }
                                }
                                if since_key.is_some() || until_key.is_some() {
                                    if ts.is_empty() {
                                        continue;
                                    }
                                    if let Some(since) = since_key.as_ref() {
                                        if ts < *since {
                                            continue;
                                        }
                                    }
                                    if let Some(until) = until_key.as_ref() {
                                        if ts > *until {
                                            continue;
                                        }
                                    }
                                }

                                entries.push(json!({
                                    "timestamp": ts,
                                    "level": level,
                                    "message": message,
                                    "raw": raw,
                                    "service": file.service.clone(),
                                    "file": file.name.clone(),
                                    "line": line_index,
                                }));

                                if entries.len() >= limit {
                                    has_more = true;
                                    next_cursor = Some(LogQueryCursor {
                                        service: file.service.clone(),
                                        file: file.name.clone(),
                                        line_index,
                                        direction: direction.clone(),
                                    });
                                    break;
                                }
                            }
                        }
                        Err(err) => {
                            log::warn!("rg failed for {}: {}", file.name, err);
                        }
                    }
                }

                if !rg_used {
                    let mut last_context: Option<(String, String)> = None;
                    let file_handle = std::fs::File::open(&file.path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to open log file {}: {}",
                            file.name, err
                        ))
                    })?;
                    let reader = BufReader::new(file_handle);
                    for (index, line) in reader.lines().enumerate() {
                        let line_index = index as u64;
                        let raw = match line {
                            Ok(value) => value,
                            Err(_) => continue,
                        };
                        let maybe_entry = Self::extract_log_entry(&raw, last_context.as_ref());
                        let (ts, level, message, raw_line) = match maybe_entry {
                            Some((ts, level, message, next_context)) => {
                                if let Some(context) = next_context {
                                    last_context = Some(context);
                                }
                                (ts, level, message, raw.trim_end().to_string())
                            }
                            None => continue,
                        };

                        if !reached_cursor {
                            if let Some(cursor) = cursor.as_ref() {
                                if cursor.service == file.service && cursor.file == file.name {
                                    if line_index <= cursor.line_index {
                                        continue;
                                    }
                                    reached_cursor = true;
                                } else {
                                    continue;
                                }
                            }
                        }
                        if let Some(filter) = level_filter.as_ref() {
                            if &level != filter {
                                continue;
                            }
                        }
                        if let Some(filter) = keyword_filter.as_ref() {
                            if !raw_line.to_lowercase().contains(filter) {
                                continue;
                            }
                        }
                        if since_key.is_some() || until_key.is_some() {
                            if ts.is_empty() {
                                continue;
                            }
                            if let Some(since) = since_key.as_ref() {
                                if ts < *since {
                                    continue;
                                }
                            }
                            if let Some(until) = until_key.as_ref() {
                                if ts > *until {
                                    continue;
                                }
                            }
                        }

                        entries.push(json!({
                            "timestamp": ts,
                            "level": level,
                            "message": message,
                            "raw": raw_line,
                            "service": file.service.clone(),
                            "file": file.name.clone(),
                            "line": line_index,
                        }));

                        if entries.len() >= limit {
                            has_more = true;
                            next_cursor = Some(LogQueryCursor {
                                service: file.service.clone(),
                                file: file.name.clone(),
                                line_index,
                                direction: direction.clone(),
                            });
                            break;
                        }
                    }
                }

                if !reached_cursor {
                    if let Some(cursor) = cursor.as_ref() {
                        if cursor.service == file.service && cursor.file == file.name {
                            reached_cursor = true;
                        }
                    }
                }

                if has_more {
                    break;
                }
            }

            Ok(RPCResponse::new(
                RPCResult::Success(json!({
                    "entries": entries,
                    "hasMore": has_more,
                    "nextCursor": next_cursor.map(|value| Self::encode_cursor(&value)),
                })),
                req.seq,
            ))
        }
    }

    pub(crate) async fn handle_system_logs_tail(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let mut services: Vec<String> = req
            .params
            .get("services")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if services.is_empty() {
            if let Some(service) = Self::param_str(&req, "service") {
                services.push(service);
            }
        }
        if services.len() != 1 {
            return Err(RPCErrors::ReasonError(
                "Tail requires exactly one service".to_string(),
            ));
        }
        let service = services[0].clone();

        let available = self.list_log_service_ids()?;
        if !available.contains(&service) {
            return Err(RPCErrors::ReasonError(format!(
                "Unknown log service: {}",
                service
            )));
        }

        let file_param = Self::param_str(&req, "file");
        let level_filter = Self::param_str(&req, "level").map(|value| value.to_lowercase());
        let keyword_filter = Self::param_str(&req, "keyword").map(|value| value.to_lowercase());
        let limit = req
            .params
            .get("limit")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_LOG_LIMIT as u64)
            .clamp(1, MAX_LOG_LIMIT as u64) as usize;
        let from = Self::param_str(&req, "from").unwrap_or_else(|| "end".to_string());
        let cursor = Self::param_str(&req, "cursor")
            .and_then(|value| Self::decode_cursor::<LogTailCursor>(&value));

        let mut files = self.collect_log_files(&service, None)?;
        if let Some(file) = file_param.as_deref() {
            files.retain(|entry| entry.name == file);
        }
        let file = files
            .first()
            .ok_or_else(|| RPCErrors::ReasonError(format!("No log files found for {}", service)))?;

        let mut start_offset = 0u64;
        let mut read_from_end = false;
        if let Some(cursor) = cursor.as_ref() {
            if cursor.file == file.name {
                start_offset = cursor.offset;
            } else {
                read_from_end = from != "start";
            }
        } else if from != "start" {
            read_from_end = true;
        }

        let path = file.path.clone();
        let file_name = file.name.clone();
        let read_result = task::spawn_blocking(move || -> Result<(Vec<String>, u64), RPCErrors> {
            let mut file = std::fs::File::open(&path).map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to open log file: {}", err))
            })?;
            let metadata = file.metadata().map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to read log metadata: {}", err))
            })?;
            let file_len = metadata.len();
            if read_from_end {
                let mut buffer = String::new();
                file.read_to_string(&mut buffer).map_err(|err| {
                    RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                })?;
                let lines = buffer.lines().map(|line| line.to_string()).collect();
                return Ok((lines, file_len));
            }
            let offset = start_offset.min(file_len);
            file.seek(SeekFrom::Start(offset)).map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to seek log file: {}", err))
            })?;
            let mut buffer = String::new();
            file.read_to_string(&mut buffer).map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
            })?;
            let lines = buffer.lines().map(|line| line.to_string()).collect();
            Ok((lines, file_len))
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("Log tail task failed: {}", err)))??;

        let mut lines = read_result.0;
        let new_offset = read_result.1;
        if read_from_end && lines.len() > limit {
            lines = lines.split_off(lines.len() - limit);
        }

        let mut entries: Vec<Value> = Vec::new();
        let mut last_context: Option<(String, String)> = None;
        for raw in lines.into_iter() {
            let maybe_entry = Self::extract_log_entry(&raw, last_context.as_ref());
            let (ts, level, message, raw_line) = match maybe_entry {
                Some((ts, level, message, next_context)) => {
                    if let Some(context) = next_context {
                        last_context = Some(context);
                    }
                    (ts, level, message, raw.trim_end().to_string())
                }
                None => continue,
            };
            if let Some(filter) = level_filter.as_ref() {
                if &level != filter {
                    continue;
                }
            }
            if let Some(filter) = keyword_filter.as_ref() {
                if !raw_line.to_lowercase().contains(filter) {
                    continue;
                }
            }
            entries.push(json!({
                "timestamp": ts,
                "level": level,
                "message": message,
                "raw": raw_line,
                "service": service.clone(),
                "file": file_name.clone(),
            }));
        }

        let next_cursor = LogTailCursor {
            file: file_name,
            offset: new_offset,
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "entries": entries,
                "nextCursor": Self::encode_cursor(&next_cursor),
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_system_logs_download(
        &self,
        req: RPCRequest,
    ) -> Result<RPCResponse, RPCErrors> {
        let mut services: Vec<String> = req
            .params
            .get("services")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|value| value.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if services.is_empty() {
            if let Some(service) = Self::param_str(&req, "service") {
                services.push(service);
            }
        }
        if services.is_empty() {
            return Err(RPCErrors::ParseRequestError("Missing service".to_string()));
        }

        let available = self.list_log_service_ids()?;
        for service in services.iter() {
            if !available.contains(service) {
                return Err(RPCErrors::ReasonError(format!(
                    "Unknown log service: {}",
                    service
                )));
            }
        }

        let mode = Self::param_str(&req, "mode").unwrap_or_else(|| "filtered".to_string());
        let level_filter = Self::param_str(&req, "level").map(|value| value.to_lowercase());
        let keyword_filter = Self::param_str(&req, "keyword").map(|value| value.to_lowercase());
        let since_filter =
            Self::param_str(&req, "since").and_then(|value| Self::parse_filter_time(&value));
        let until_filter =
            Self::param_str(&req, "until").and_then(|value| Self::parse_filter_time(&value));

        let token = Uuid::new_v4().to_string();
        let file_name = format!("buckyos-logs-{}.zip", token);
        let zip_path = std::env::temp_dir().join(&file_name);
        let zip_path_clone = zip_path.clone();

        let services_clone = services.clone();
        let mode_clone = mode.clone();
        let file_filter = Self::param_str(&req, "file");

        task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let file = std::fs::File::create(&zip_path_clone)
                .map_err(|err| RPCErrors::ReasonError(format!("Failed to create zip: {}", err)))?;
            let mut zip = zip::ZipWriter::new(file);
            let options =
                FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);

            for service in services_clone.iter() {
                let dir_path = Path::new(LOG_ROOT_DIR).join(service);
                if mode_clone == "full" {
                    let entries = std::fs::read_dir(&dir_path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to read log dir {}: {}",
                            service, err
                        ))
                    })?;
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_file() {
                            continue;
                        }
                        let name = match path.file_name().and_then(|value| value.to_str()) {
                            Some(name) => name.to_string(),
                            None => continue,
                        };
                        if let Some(filter) = file_filter.as_deref() {
                            if name != filter {
                                continue;
                            }
                        }
                        let entry_name = format!("{}/{}", service, name);
                        zip.start_file(entry_name, options)
                            .map_err(|err| RPCErrors::ReasonError(format!("Zip error: {}", err)))?;
                        let mut file_reader = std::fs::File::open(&path).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                        })?;
                        let mut buffer = Vec::new();
                        file_reader.read_to_end(&mut buffer).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                        })?;
                        zip.write_all(&buffer).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to write zip: {}", err))
                        })?;
                    }
                } else {
                    let mut filtered = String::new();
                    let entries = std::fs::read_dir(&dir_path).map_err(|err| {
                        RPCErrors::ReasonError(format!(
                            "Failed to read log dir {}: {}",
                            service, err
                        ))
                    })?;
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if !path.is_file() {
                            continue;
                        }
                        let name = match path.file_name().and_then(|value| value.to_str()) {
                            Some(name) => name.to_string(),
                            None => continue,
                        };
                        if let Some(filter) = file_filter.as_deref() {
                            if name != filter {
                                continue;
                            }
                        }
                        let file_handle = std::fs::File::open(&path).map_err(|err| {
                            RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                        })?;
                        let reader = BufReader::new(file_handle);
                        for line in reader.lines().map_while(Result::ok) {
                            let (ts, level, message) = ControlPanelServer::split_log_line(&line);
                            if let Some(filter) = level_filter.as_ref() {
                                if &level != filter {
                                    continue;
                                }
                            }
                            if let Some(filter) = keyword_filter.as_ref() {
                                if !line.to_lowercase().contains(filter) {
                                    continue;
                                }
                            }
                            if since_filter.is_some() || until_filter.is_some() {
                                let ts_value = match ControlPanelServer::parse_log_timestamp(&ts) {
                                    Some(value) => value,
                                    None => continue,
                                };
                                if let Some(since) = since_filter.as_ref() {
                                    if &ts_value < since {
                                        continue;
                                    }
                                }
                                if let Some(until) = until_filter.as_ref() {
                                    if &ts_value > until {
                                        continue;
                                    }
                                }
                            }
                            filtered.push_str(&format!(
                                "{} {} {}\n",
                                ts,
                                level.to_uppercase(),
                                message
                            ));
                        }
                    }
                    let entry_name = format!("{}/filtered.log", service);
                    zip.start_file(entry_name, options)
                        .map_err(|err| RPCErrors::ReasonError(format!("Zip error: {}", err)))?;
                    zip.write_all(filtered.as_bytes()).map_err(|err| {
                        RPCErrors::ReasonError(format!("Failed to write zip: {}", err))
                    })?;
                }
            }

            zip.finish()
                .map_err(|err| RPCErrors::ReasonError(format!("Failed to finish zip: {}", err)))?;
            Ok(())
        })
        .await
        .map_err(|err| RPCErrors::ReasonError(format!("Zip task failed: {}", err)))??;

        self.cleanup_log_downloads().await;
        let mut downloads = self.log_downloads.lock().await;
        downloads.insert(
            token.clone(),
            LogDownloadEntry {
                path: zip_path,
                filename: file_name.clone(),
                expires_at: std::time::SystemTime::now()
                    + std::time::Duration::from_secs(LOG_DOWNLOAD_TTL_SECS),
            },
        );

        Ok(RPCResponse::new(
            RPCResult::Success(json!({
                "url": format!("/kapi/control-panel/logs/download/{}", token),
                "expiresInSec": LOG_DOWNLOAD_TTL_SECS,
                "filename": file_name,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_logs_download_http(
        &self,
        token: &str,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        self.cleanup_log_downloads().await;
        let (path, filename) = {
            let downloads = self.log_downloads.lock().await;
            match downloads.get(token) {
                Some(entry) => (entry.path.clone(), entry.filename.clone()),
                None => {
                    return Err(server_err!(
                        ServerErrorCode::BadRequest,
                        "Invalid download token"
                    ))
                }
            }
        };

        let content = tokio::fs::read(&path)
            .await
            .map_err(|err| server_err!(ServerErrorCode::InvalidData, "Read zip error: {}", err))?;
        let body = BoxBody::new(
            Full::new(Bytes::from(content))
                .map_err(|never: std::convert::Infallible| match never {})
                .boxed(),
        );

        http::Response::builder()
            .header(CONTENT_TYPE, "application/zip")
            .header(
                CONTENT_DISPOSITION,
                format!("attachment; filename=\"{}\"", filename),
            )
            .header(CACHE_CONTROL, "no-store")
            .body(body)
            .map_err(|err| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build download response: {}",
                    err
                )
            })
    }
}
