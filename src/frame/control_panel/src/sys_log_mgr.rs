use crate::redaction::redact_text;
use crate::{external_command, ControlPanelServer};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use base64::{engine::general_purpose, Engine as _};
use buckyos_http_server::*;
use bytes::Bytes;
use chrono::{DateTime, Datelike, Local, NaiveDateTime, TimeZone, Utc};
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
const MAX_LOG_LIMIT: usize = 500;

#[derive(Clone, Serialize, Deserialize)]
struct LogQueryCursor {
    service: String,
    file: String,
    line_index: u64,
    direction: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_file_selector_rejects_host_paths() {
        assert!(ControlPanelServer::validate_log_file_name(Some("scheduler.log")).is_ok());
        assert!(ControlPanelServer::validate_log_file_name(Some("../scheduler.log")).is_err());
        assert!(ControlPanelServer::validate_log_file_name(Some("/etc/passwd")).is_err());
    }

    #[test]
    fn log_line_matches_current_logger_format() {
        for (level, expected_level) in [
            ("INFO", "info"),
            ("WARN", "warning"),
            ("ERROR", "error"),
            ("DEBUG", "debug"),
            ("TRACE", "trace"),
        ] {
            let line = format!("01-15 12:00:00.123 {level:<5} [main.rs:42] 日志 [details]");
            let (timestamp, parsed_level, message) = ControlPanelServer::split_log_line(&line);
            assert_eq!(timestamp, "01-15 12:00:00.123");
            assert_eq!(parsed_level, expected_level);
            assert_eq!(message, "[main.rs:42] 日志 [details]");
            assert!(ControlPanelServer::parse_log_timestamp(&timestamp).is_some());
        }
    }

    #[test]
    fn log_continuations_do_not_replace_header_context() {
        let context = ("01-15 12:00:00.123".to_string(), "error".to_string());
        for line in [
            "  请求失败 [details]",
            "  {\"items\": [1, 2]}",
            "99-99 99:99:99.999 INFO [invalid timestamp]",
        ] {
            let (timestamp, level, _, next_context) =
                ControlPanelServer::extract_log_entry(line, Some(&context)).unwrap();
            assert_eq!(timestamp, context.0);
            assert_eq!(level, context.1);
            assert!(next_context.is_none());
        }
    }

    #[test]
    fn log_timestamp_and_query_key_use_service_local_time() {
        let local = Local
            .with_ymd_and_hms(Local::now().year(), 1, 15, 12, 0, 0)
            .single()
            .unwrap();
        let utc = local.with_timezone(&Utc);
        assert_eq!(
            ControlPanelServer::parse_log_timestamp("01-15 12:00:00.000"),
            Some(utc)
        );
        assert_eq!(
            ControlPanelServer::format_log_filter_key(&utc),
            "01-15 12:00:00.000"
        );
        assert_eq!(
            ControlPanelServer::parse_filter_time(&utc.to_rfc3339()),
            Some(utc)
        );
    }

    #[test]
    fn filtered_download_keeps_matching_records_and_continuations() {
        let input = concat!(
            "orphan continuation\n",
            "01-15 11:59:59.999 INFO  [main.rs:1] too early\n",
            "  excluded earlier detail\n",
            "01-15 12:00:00.000 INFO  [main.rs:2] first\n",
            "  请求内容 [1, 2]\n",
            "01-15 12:01:00.000 WARN  [main.rs:3] last\n",
            "  last detail\n",
            "01-15 12:01:00.001 ERROR [main.rs:4] too late\n",
            "  excluded later detail\n",
        );
        let since = Local
            .with_ymd_and_hms(Local::now().year(), 1, 15, 12, 0, 0)
            .single()
            .unwrap()
            .with_timezone(&Utc);
        let until = since + chrono::Duration::minutes(1);
        let output = ControlPanelServer::filter_log_lines(
            input.as_bytes(),
            None,
            None,
            Some(&since),
            Some(&until),
        )
        .unwrap();
        assert_eq!(
            output,
            concat!(
                "01-15 12:00:00.000 INFO  [main.rs:2] first\n",
                "  请求内容 [1, 2]\n",
                "01-15 12:01:00.000 WARN  [main.rs:3] last\n",
                "  last detail\n",
            )
        );
        assert!(ControlPanelServer::filter_log_lines(
            input.as_bytes(),
            None,
            None,
            Some(&(until + chrono::Duration::hours(1))),
            None,
        )
        .unwrap()
        .is_empty());
    }

    #[test]
    fn filtered_download_applies_level_keyword_and_redaction() {
        let input = concat!(
            "01-15 12:00:00.000 WARN  [main.rs:1] warning\n",
            "  MATCH password=hidden-value\n",
            "01-15 12:00:00.001 INFO  [main.rs:2] other\n",
            "  MATCH excluded\n",
        );
        let output = ControlPanelServer::filter_log_lines(
            input.as_bytes(),
            Some("warning"),
            Some("match"),
            None,
            None,
        )
        .unwrap();
        assert!(output.starts_with("  MATCH password="));
        assert!(output.contains("REDACTED"));
        assert!(!output.contains("hidden-value"));
        assert!(!output.contains("excluded"));
    }

    #[test]
    fn filtered_download_reports_read_errors() {
        let input = b"01-15 12:00:00.000 INFO  [main.rs:1] first\n\xff\n";
        assert!(ControlPanelServer::filter_log_lines(&input[..], None, None, None, None).is_err());
    }

    #[test]
    fn filtered_download_redacts_multiline_secrets_before_keyword_filtering() {
        let input = concat!(
            "01-15 12:00:00.000 INFO  [main.rs:1] key:\n",
            "-----BEGIN PRIVATE KEY-----\n",
            "sensitive-key-material\n",
            "-----END PRIVATE KEY-----\n",
            "  sensitive operation completed\n",
        );
        let output = ControlPanelServer::filter_log_lines(
            input.as_bytes(),
            None,
            Some("sensitive"),
            None,
            None,
        )
        .unwrap();
        assert_eq!(output, "  sensitive operation completed\n");
    }
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
        let mut parts = trimmed.splitn(3, ' ');
        if let (Some(date), Some(time), Some(rest)) = (parts.next(), parts.next(), parts.next()) {
            if let Some((level, message)) = rest.trim_start().split_once(char::is_whitespace) {
                let timestamp = format!("{} {}", date, time);
                if matches!(level, "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE")
                    && Self::parse_log_timestamp(&timestamp).is_some()
                {
                    return (
                        timestamp,
                        Self::normalize_log_level(level),
                        message.trim_start().to_string(),
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
        let year = Local::now().year();
        let with_year = format!("{}-{}", year, value);
        let parsed = NaiveDateTime::parse_from_str(&with_year, "%Y-%m-%d %H:%M:%S%.3f").ok()?;
        Local
            .from_local_datetime(&parsed)
            .earliest()
            .map(|timestamp| timestamp.with_timezone(&Utc))
    }

    pub(crate) fn parse_filter_time(value: &str) -> Option<DateTime<Utc>> {
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

    fn parse_request_filter_time(
        req: &RPCRequest,
        name: &str,
    ) -> Result<Option<DateTime<Utc>>, RPCErrors> {
        Self::param_str(req, name)
            .map(|value| {
                Self::parse_filter_time(&value).ok_or_else(|| {
                    RPCErrors::ParseRequestError(format!("{} must be RFC 3339", name))
                })
            })
            .transpose()
    }

    fn format_log_filter_key(value: &DateTime<Utc>) -> String {
        value
            .with_timezone(&Local)
            .format("%m-%d %H:%M:%S%.3f")
            .to_string()
    }

    fn filter_log_lines(
        reader: impl BufRead,
        level_filter: Option<&str>,
        keyword_filter: Option<&str>,
        since_filter: Option<&DateTime<Utc>>,
        until_filter: Option<&DateTime<Utc>>,
    ) -> Result<String, RPCErrors> {
        let mut filtered = String::new();
        let mut context = None;
        for line in reader.lines() {
            let line = line.map_err(|err| {
                RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
            })?;
            let Some((timestamp, level, _, next_context)) =
                Self::extract_log_entry(&line, context.as_ref())
            else {
                continue;
            };
            if let Some(next_context) = next_context {
                context = Some(next_context);
            }
            if level_filter.is_some_and(|filter| level != filter) {
                continue;
            }
            if since_filter.is_some() || until_filter.is_some() {
                let Some(timestamp) = Self::parse_log_timestamp(&timestamp) else {
                    continue;
                };
                if since_filter.is_some_and(|start| &timestamp < start)
                    || until_filter.is_some_and(|end| &timestamp > end)
                {
                    continue;
                }
            }
            filtered.push_str(&line);
            filtered.push('\n');
        }
        let redacted = redact_text(&filtered);
        if let Some(keyword) = keyword_filter {
            filtered.clear();
            for line in redacted.lines() {
                if line.to_lowercase().contains(keyword) {
                    filtered.push_str(line);
                    filtered.push('\n');
                }
            }
            Ok(filtered)
        } else {
            Ok(redacted)
        }
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

    fn validate_log_file_name(value: Option<&str>) -> Result<(), RPCErrors> {
        if let Some(value) = value {
            let valid = !matches!(value, "." | "..")
                && Path::new(value).file_name().and_then(|name| name.to_str()) == Some(value);
            if !valid {
                return Err(RPCErrors::ParseRequestError(
                    "log file must be an exact filename".to_string(),
                ));
            }
        }
        Ok(())
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
        Self::validate_log_file_name(file_filter.as_deref())?;
        let direction = Self::param_str(&req, "direction").unwrap_or_else(|| "forward".to_string());
        let direction = if direction == "backward" {
            "backward".to_string()
        } else {
            "forward".to_string()
        };
        let level_filter = Self::param_str(&req, "level").map(|value| Self::normalize_log_level(&value));
        let keyword_raw = Self::param_str(&req, "keyword");
        let keyword_filter = keyword_raw.as_ref().map(|value| value.to_lowercase());
        let since_filter = Self::parse_request_filter_time(&req, "since")?;
        let until_filter = Self::parse_request_filter_time(&req, "until")?;
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
                        "message": redact_text(&message),
                        "raw": redact_text(&raw),
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
                                    "message": redact_text(&message),
                                    "raw": redact_text(&raw),
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
                            "message": redact_text(&message),
                            "raw": redact_text(&raw_line),
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
        Self::validate_log_file_name(file_param.as_deref())?;
        let level_filter = Self::param_str(&req, "level").map(|value| Self::normalize_log_level(&value));
        let keyword_filter = Self::param_str(&req, "keyword").map(|value| value.to_lowercase());
        let since_filter = Self::parse_request_filter_time(&req, "since")?;
        let until_filter = Self::parse_request_filter_time(&req, "until")?;
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
                return Ok((Vec::new(), file_len));
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
            if since_filter.is_some() || until_filter.is_some() {
                let timestamp = match Self::parse_log_timestamp(&ts) {
                    Some(timestamp) => timestamp,
                    None => continue,
                };
                if since_filter
                    .as_ref()
                    .is_some_and(|start| &timestamp < start)
                    || until_filter.as_ref().is_some_and(|end| &timestamp > end)
                {
                    continue;
                }
            }
            entries.push(json!({
                "timestamp": ts,
                "level": level,
                "message": redact_text(&message),
                "raw": redact_text(&raw_line),
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
        if mode != "filtered" {
            return Err(RPCErrors::ParseRequestError(
                "only filtered log downloads are supported".to_string(),
            ));
        }
        let level_filter = Self::param_str(&req, "level").map(|value| Self::normalize_log_level(&value));
        let keyword_filter = Self::param_str(&req, "keyword").map(|value| value.to_lowercase());
        let since_filter = Self::parse_request_filter_time(&req, "since")?;
        let until_filter = Self::parse_request_filter_time(&req, "until")?;
        if since_filter.is_none() && until_filter.is_none() {
            return Err(RPCErrors::ParseRequestError(
                "log download requires since or until".to_string(),
            ));
        }

        let token = Uuid::new_v4().to_string();
        let file_name = format!("buckyos-logs-{}.zip", token);
        let zip_path = std::env::temp_dir().join(&file_name);
        let zip_path_clone = zip_path.clone();

        let services_clone = services.clone();
        let file_filter = Self::param_str(&req, "file");
        Self::validate_log_file_name(file_filter.as_deref())?;

        task::spawn_blocking(move || -> Result<(), RPCErrors> {
            let file = std::fs::File::create(&zip_path_clone)
                .map_err(|err| RPCErrors::ReasonError(format!("Failed to create zip: {}", err)))?;
            let mut zip = zip::ZipWriter::new(file);
            let options =
                FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);

            for service in services_clone.iter() {
                let dir_path = Path::new(LOG_ROOT_DIR).join(service);
                let mut filtered = String::new();
                let entries = std::fs::read_dir(&dir_path).map_err(|err| {
                    RPCErrors::ReasonError(format!("Failed to read log dir {}: {}", service, err))
                })?;
                for entry in entries.flatten() {
                    let path = entry.path();
                    if !path.is_file() {
                        continue;
                    }
                    let name = match path.file_name().and_then(|value| value.to_str()) {
                        Some(name) => name,
                        None => continue,
                    };
                    if file_filter.as_deref().is_some_and(|filter| name != filter) {
                        continue;
                    }
                    let file_handle = std::fs::File::open(&path).map_err(|err| {
                        RPCErrors::ReasonError(format!("Failed to read log file: {}", err))
                    })?;
                    filtered.push_str(&ControlPanelServer::filter_log_lines(
                        BufReader::new(file_handle),
                        level_filter.as_deref(),
                        keyword_filter.as_deref(),
                        since_filter.as_ref(),
                        until_filter.as_ref(),
                    )?);
                }
                let entry_name = format!("{}/filtered.log", service);
                zip.start_file(entry_name, options)
                    .map_err(|err| RPCErrors::ReasonError(format!("Zip error: {}", err)))?;
                zip.write_all(filtered.as_bytes()).map_err(|err| {
                    RPCErrors::ReasonError(format!("Failed to write zip: {}", err))
                })?;
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
