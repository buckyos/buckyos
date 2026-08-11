use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio::fs;
use tokio::process::Command;

use crate::{
    parse_default_bash_exec_args, resolve_path_from_root, rewrite_path_with_shell_cwd,
    AgentToolError, CallingConventions, CliInvocation, FileToolConfig, ToolCtx, TypedTool,
};

pub const TOOL_GREP: &str = "Grep";

const DEFAULT_HEAD_LIMIT: usize = 250;
const MAX_COLUMNS: &str = "500";

const VCS_DIRECTORIES_TO_EXCLUDE: [&str; 6] = [".git", ".svn", ".hg", ".bzr", ".jj", ".sl"];

const DESCRIPTION: &str = r#"A powerful search tool built on ripgrep

Usage:
- Search file contents with regular expressions.
- Supports full ripgrep regex syntax, glob filtering, file type filtering, line numbers, context lines, pagination, and multiline matching.
- Output modes: "content" shows matching lines, "files_with_matches" shows only file paths (default), "count" shows match counts.
- Use this tool for code/content search instead of invoking grep or rg manually."#;

#[derive(Clone, Debug)]
pub struct GrepTool {
    cfg: FileToolConfig,
}

impl GrepTool {
    pub fn new(cfg: FileToolConfig) -> Self {
        Self { cfg }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GrepOutputMode {
    Content,
    FilesWithMatches,
    Count,
}

fn default_output_mode() -> GrepOutputMode {
    GrepOutputMode::FilesWithMatches
}

fn default_show_line_numbers() -> bool {
    true
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub glob: Option<String>,
    #[serde(default = "default_output_mode")]
    pub output_mode: GrepOutputMode,
    #[serde(default, rename = "-B")]
    pub before: Option<usize>,
    #[serde(default, rename = "-A")]
    pub after: Option<usize>,
    #[serde(default, rename = "-C")]
    pub context_flag: Option<usize>,
    #[serde(default)]
    pub context: Option<usize>,
    #[serde(default = "default_show_line_numbers", rename = "-n")]
    pub show_line_numbers: bool,
    #[serde(default, rename = "-i")]
    pub case_insensitive: bool,
    #[serde(default)]
    pub r#type: Option<String>,
    #[serde(default)]
    pub head_limit: Option<usize>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default)]
    pub multiline: bool,
}

#[derive(Serialize, JsonSchema)]
pub struct GrepOutput {
    pub mode: GrepOutputMode,
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    #[serde(rename = "numFiles")]
    pub num_files: usize,
    pub filenames: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(rename = "numLines", skip_serializing_if = "Option::is_none")]
    pub num_lines: Option<usize>,
    #[serde(rename = "numMatches", skip_serializing_if = "Option::is_none")]
    pub num_matches: Option<usize>,
    #[serde(rename = "appliedLimit", skip_serializing_if = "Option::is_none")]
    pub applied_limit: Option<usize>,
    #[serde(rename = "appliedOffset", skip_serializing_if = "Option::is_none")]
    pub applied_offset: Option<usize>,
}

#[async_trait]
impl TypedTool for GrepTool {
    type Args = GrepArgs;
    type Output = GrepOutput;

    fn name(&self) -> &str {
        TOOL_GREP
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::from_legacy(true, false, true)
    }

    fn usage(&self) -> Option<String> {
        Some(
            "Grep <pattern> [path]\n\tpattern: ripgrep regex; optional key=value args include glob, type, output_mode, head_limit, offset".to_string(),
        )
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        parse_grep_args(tokens, shell_cwd)
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        Ok(CliInvocation::Json {
            args: parse_grep_args(tokens, shell_cwd)?,
            content_input: None,
        })
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let mut cmd = format!("{} pattern={}", TOOL_GREP, quote_arg(&args.pattern));
        if let Some(path) = args.path.as_deref() {
            cmd.push_str(" path=");
            cmd.push_str(&quote_arg(path));
        }
        if args.output_mode != GrepOutputMode::FilesWithMatches {
            cmd.push_str(" output_mode=");
            cmd.push_str(match args.output_mode {
                GrepOutputMode::Content => "content",
                GrepOutputMode::FilesWithMatches => "files_with_matches",
                GrepOutputMode::Count => "count",
            });
        }
        Some(cmd)
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        match output.mode {
            GrepOutputMode::Content => {
                let mut summary = format!(
                    "found {} lines in {}ms",
                    output.num_lines.unwrap_or(0),
                    output.duration_ms
                );
                append_pagination_summary(&mut summary, output);
                if let Some(content) = output.content.as_deref().filter(|value| !value.is_empty()) {
                    summary.push_str("\n```grep\n");
                    summary.push_str(content);
                    summary.push_str("\n```");
                }
                summary
            }
            GrepOutputMode::Count => {
                let mut summary = format!(
                    "found {} occurrences across {} files in {}ms",
                    output.num_matches.unwrap_or(0),
                    output.num_files,
                    output.duration_ms
                );
                append_pagination_summary(&mut summary, output);
                if let Some(content) = output.content.as_deref().filter(|value| !value.is_empty()) {
                    summary.push_str("\n```counts\n");
                    summary.push_str(content);
                    summary.push_str("\n```");
                }
                summary
            }
            GrepOutputMode::FilesWithMatches => {
                let mut summary = format!(
                    "found {} files in {}ms",
                    output.num_files, output.duration_ms
                );
                append_pagination_summary(&mut summary, output);
                if !output.filenames.is_empty() {
                    summary.push_str("\n```files\n");
                    summary.push_str(&output.filenames.join("\n"));
                    summary.push_str("\n```");
                }
                summary
            }
        }
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        Some(match output.mode {
            GrepOutputMode::Content => format!(
                "{} => found {} lines{}",
                TOOL_GREP,
                output.num_lines.unwrap_or(0),
                title_pagination_suffix(output)
            ),
            GrepOutputMode::Count => format!(
                "{} => found {} occurrences in {} files{}",
                TOOL_GREP,
                output.num_matches.unwrap_or(0),
                output.num_files,
                title_pagination_suffix(output)
            ),
            GrepOutputMode::FilesWithMatches => format!(
                "{} => found {} files{}",
                TOOL_GREP,
                output.num_files,
                title_pagination_suffix(output)
            ),
        })
    }

    async fn execute(
        &self,
        _ctx: &ToolCtx<'_>,
        args: Self::Args,
    ) -> Result<Self::Output, AgentToolError> {
        let pattern = args.pattern.trim().to_string();
        if pattern.is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "missing required arg `pattern`".to_string(),
            ));
        }
        if pattern.contains('\0') {
            return Err(AgentToolError::InvalidArgs(
                "pattern cannot contain null bytes".to_string(),
            ));
        }

        let start = Instant::now();
        let raw_search_path = args.path.as_deref().unwrap_or(".");
        let search_path = match args
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            Some(path) => resolve_path_from_root(&self.cfg.root_dir, path)?,
            None => self.cfg.root_dir.clone(),
        };
        ensure_read_path_allowed(&self.cfg, &search_path, raw_search_path)?;

        let metadata = fs::metadata(&search_path).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                AgentToolError::InvalidArgs(format!("Path does not exist: {raw_search_path}"))
            } else {
                AgentToolError::ExecFailed(format!(
                    "stat path `{}` failed: {err}",
                    search_path.display()
                ))
            }
        })?;
        if !(metadata.is_dir() || metadata.is_file()) {
            return Err(AgentToolError::InvalidArgs(format!(
                "Path is not a file or directory: {raw_search_path}"
            )));
        }

        let rg_lines = run_ripgrep(&self.cfg.root_dir, &search_path, &args, &pattern).await?;
        let output = match args.output_mode {
            GrepOutputMode::Content => build_content_output(
                start.elapsed().as_millis() as u64,
                rg_lines,
                &self.cfg.root_dir,
                &args,
            ),
            GrepOutputMode::Count => build_count_output(
                start.elapsed().as_millis() as u64,
                rg_lines,
                &self.cfg.root_dir,
                &args,
            ),
            GrepOutputMode::FilesWithMatches => {
                build_files_output(
                    start.elapsed().as_millis() as u64,
                    rg_lines,
                    &self.cfg.root_dir,
                    &args,
                )
                .await
            }
        };

        Ok(output)
    }
}

fn parse_grep_args(tokens: &[String], shell_cwd: Option<&Path>) -> Result<Json, AgentToolError> {
    if tokens.is_empty() {
        return Err(AgentToolError::InvalidArgs(
            "missing required arg `pattern`".to_string(),
        ));
    }

    let mut args = if tokens.len() == 1 && tokens[0].trim_start().starts_with('{') {
        parse_default_bash_exec_args(tokens)?
    } else if tokens.iter().any(|token| token.contains('=')) {
        if tokens.iter().any(|token| !token.contains('=')) {
            return Err(AgentToolError::InvalidArgs(
                "Grep args cannot mix positional args with key=value args".to_string(),
            ));
        }
        parse_default_bash_exec_args(tokens)?
    } else {
        if tokens.len() > 2 {
            return Err(AgentToolError::InvalidArgs(format!(
                "too many positional args for tool `{}`: got {}, max 2",
                TOOL_GREP,
                tokens.len()
            )));
        }
        let mut map = serde_json::Map::new();
        map.insert(
            "pattern".to_string(),
            Json::String(tokens[0].trim().to_string()),
        );
        if let Some(path) = tokens.get(1) {
            map.insert("path".to_string(), Json::String(path.trim().to_string()));
        }
        Json::Object(map)
    };

    let map = args.as_object_mut().ok_or_else(|| {
        AgentToolError::InvalidArgs("Grep args must be a json object".to_string())
    })?;
    let pattern = map
        .get("pattern")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AgentToolError::InvalidArgs("missing required arg `pattern`".to_string()))?
        .to_string();
    map.insert("pattern".to_string(), Json::String(pattern));

    if let Some(path_value) = map.get("path") {
        if !path_value.is_string() {
            return Err(AgentToolError::InvalidArgs(
                "Grep arg `path` must be a string".to_string(),
            ));
        }
    }
    if let Some(cwd) = shell_cwd {
        if let Some(raw_path) = map
            .get("path")
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            if !Path::new(raw_path).is_absolute() {
                map.insert(
                    "path".to_string(),
                    Json::String(rewrite_path_with_shell_cwd(raw_path.to_string(), cwd)),
                );
            }
        }
    }
    Ok(args)
}

async fn run_ripgrep(
    root_dir: &Path,
    search_path: &Path,
    args: &GrepArgs,
    pattern: &str,
) -> Result<Vec<String>, AgentToolError> {
    let mut rg_args = Vec::<String>::new();
    rg_args.push("--hidden".to_string());
    rg_args.push("--color".to_string());
    rg_args.push("never".to_string());
    rg_args.push("--with-filename".to_string());
    rg_args.push("--max-columns".to_string());
    rg_args.push(MAX_COLUMNS.to_string());

    for dir in VCS_DIRECTORIES_TO_EXCLUDE {
        rg_args.push("--glob".to_string());
        rg_args.push(format!("!{dir}"));
    }

    if args.multiline {
        rg_args.push("-U".to_string());
        rg_args.push("--multiline-dotall".to_string());
    }
    if args.case_insensitive {
        rg_args.push("-i".to_string());
    }

    match args.output_mode {
        GrepOutputMode::FilesWithMatches => rg_args.push("-l".to_string()),
        GrepOutputMode::Count => rg_args.push("-c".to_string()),
        GrepOutputMode::Content => {
            if args.show_line_numbers {
                rg_args.push("-n".to_string());
            }
            if let Some(context) = args.context.or(args.context_flag) {
                rg_args.push("-C".to_string());
                rg_args.push(context.to_string());
            } else {
                if let Some(before) = args.before {
                    rg_args.push("-B".to_string());
                    rg_args.push(before.to_string());
                }
                if let Some(after) = args.after {
                    rg_args.push("-A".to_string());
                    rg_args.push(after.to_string());
                }
            }
        }
    }

    if let Some(file_type) = args
        .r#type
        .as_deref()
        .map(str::trim)
        .filter(|v| !v.is_empty())
    {
        rg_args.push("--type".to_string());
        rg_args.push(file_type.to_string());
    }

    if let Some(glob) = args.glob.as_deref() {
        for glob_pattern in split_glob_patterns(glob) {
            rg_args.push("--glob".to_string());
            rg_args.push(glob_pattern);
        }
    }

    if pattern.starts_with('-') {
        rg_args.push("-e".to_string());
        rg_args.push(pattern.to_string());
    } else {
        rg_args.push(pattern.to_string());
    }

    rg_args.push(display_search_arg(root_dir, search_path));

    let output = Command::new("rg")
        .current_dir(root_dir)
        .args(&rg_args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                AgentToolError::ExecFailed("ripgrep executable `rg` was not found".to_string())
            } else {
                AgentToolError::ExecFailed(format!("run ripgrep failed: {err}"))
            }
        })?;

    match output.status.code() {
        Some(0) | Some(1) => {}
        Some(code) => {
            return Err(AgentToolError::ExecFailed(format!(
                "ripgrep exited with code {code}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        None => {
            return Err(AgentToolError::ExecFailed(
                "ripgrep terminated by signal".to_string(),
            ));
        }
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(|line| line.trim_end_matches('\r').to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

fn build_content_output(
    duration_ms: u64,
    lines: Vec<String>,
    root_dir: &Path,
    args: &GrepArgs,
) -> GrepOutput {
    let (limited, applied_limit) = apply_head_limit(lines, args.head_limit, args.offset);
    let final_lines = limited
        .into_iter()
        .map(|line| relativize_rg_line(&line, root_dir, false))
        .collect::<Vec<_>>();

    GrepOutput {
        mode: GrepOutputMode::Content,
        duration_ms,
        num_files: 0,
        filenames: Vec::new(),
        content: Some(final_lines.join("\n")),
        num_lines: Some(final_lines.len()),
        num_matches: None,
        applied_limit,
        applied_offset: applied_offset(args.offset),
    }
}

fn build_count_output(
    duration_ms: u64,
    lines: Vec<String>,
    root_dir: &Path,
    args: &GrepArgs,
) -> GrepOutput {
    let (limited, applied_limit) = apply_head_limit(lines, args.head_limit, args.offset);
    let final_lines = limited
        .into_iter()
        .map(|line| relativize_rg_line(&line, root_dir, true))
        .collect::<Vec<_>>();

    let mut total_matches = 0usize;
    let mut file_count = 0usize;
    for line in &final_lines {
        if let Some((_, count)) = line.rsplit_once(':') {
            if let Ok(count) = count.parse::<usize>() {
                total_matches += count;
                file_count += 1;
            }
        }
    }

    GrepOutput {
        mode: GrepOutputMode::Count,
        duration_ms,
        num_files: file_count,
        filenames: Vec::new(),
        content: Some(final_lines.join("\n")),
        num_lines: None,
        num_matches: Some(total_matches),
        applied_limit,
        applied_offset: applied_offset(args.offset),
    }
}

async fn build_files_output(
    duration_ms: u64,
    lines: Vec<String>,
    root_dir: &Path,
    args: &GrepArgs,
) -> GrepOutput {
    let mut candidates = Vec::with_capacity(lines.len());
    for line in lines {
        let abs_path = if Path::new(&line).is_absolute() {
            PathBuf::from(&line)
        } else {
            root_dir.join(&line)
        };
        let modified = fs::metadata(&abs_path)
            .await
            .ok()
            .and_then(|metadata| metadata.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        candidates.push((line, modified));
    }
    candidates.sort_by(|a, b| {
        b.1.cmp(&a.1)
            .then_with(|| a.0.replace('\\', "/").cmp(&b.0.replace('\\', "/")))
    });

    let sorted = candidates
        .into_iter()
        .map(|(path, _)| display_path(Path::new(&path), root_dir))
        .collect::<Vec<_>>();
    let (filenames, applied_limit) = apply_head_limit(sorted, args.head_limit, args.offset);

    GrepOutput {
        mode: GrepOutputMode::FilesWithMatches,
        duration_ms,
        num_files: filenames.len(),
        filenames,
        content: None,
        num_lines: None,
        num_matches: None,
        applied_limit,
        applied_offset: applied_offset(args.offset),
    }
}

fn apply_head_limit<T>(
    items: Vec<T>,
    limit: Option<usize>,
    offset: usize,
) -> (Vec<T>, Option<usize>) {
    if limit == Some(0) {
        return (items.into_iter().skip(offset).collect(), None);
    }
    let effective_limit = limit.unwrap_or(DEFAULT_HEAD_LIMIT);
    let was_truncated = items.len().saturating_sub(offset) > effective_limit;
    let selected = items
        .into_iter()
        .skip(offset)
        .take(effective_limit)
        .collect();
    (selected, was_truncated.then_some(effective_limit))
}

fn applied_offset(offset: usize) -> Option<usize> {
    if offset > 0 {
        Some(offset)
    } else {
        None
    }
}

fn append_pagination_summary(summary: &mut String, output: &GrepOutput) {
    let mut parts = Vec::new();
    if let Some(limit) = output.applied_limit {
        parts.push(format!("limit: {limit}"));
    }
    if let Some(offset) = output.applied_offset {
        parts.push(format!("offset: {offset}"));
    }
    if !parts.is_empty() {
        summary.push_str(" (");
        summary.push_str(&parts.join(", "));
        summary.push(')');
    }
}

fn title_pagination_suffix(output: &GrepOutput) -> String {
    if output.applied_limit.is_some() || output.applied_offset.is_some() {
        " (paginated)".to_string()
    } else {
        String::new()
    }
}

fn split_glob_patterns(glob: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in glob.split_whitespace() {
        if raw.contains('{') && raw.contains('}') {
            out.push(raw.to_string());
        } else {
            out.extend(
                raw.split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string),
            );
        }
    }
    out
}

fn display_search_arg(root_dir: &Path, search_path: &Path) -> String {
    match search_path.strip_prefix(root_dir) {
        Ok(rel) if rel.as_os_str().is_empty() => ".".to_string(),
        Ok(rel) => path_to_slash_string(rel),
        Err(_) => path_to_slash_string(search_path),
    }
}

fn relativize_rg_line(line: &str, root_dir: &Path, split_last_colon: bool) -> String {
    let split = if split_last_colon {
        line.rsplit_once(':')
    } else {
        line.split_once(':')
    };
    let Some((file_path, rest)) = split else {
        return line.to_string();
    };
    let display = display_path(Path::new(file_path), root_dir);
    format!("{display}:{rest}")
}

fn path_to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn display_path(path: &Path, output_root: &Path) -> String {
    let abs_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        output_root.join(path)
    };
    abs_path
        .strip_prefix(output_root)
        .ok()
        .filter(|rel| !rel.as_os_str().is_empty())
        .map(path_to_slash_string)
        .unwrap_or_else(|| path_to_slash_string(path))
}

fn ensure_read_path_allowed(
    cfg: &FileToolConfig,
    abs_path: &Path,
    raw_path: &str,
) -> Result<(), AgentToolError> {
    if cfg.allowed_read_roots.is_empty()
        || cfg
            .allowed_read_roots
            .iter()
            .any(|root| abs_path.starts_with(root))
    {
        return Ok(());
    }
    Err(AgentToolError::InvalidArgs(format!(
        "read path not allowed by policy: {raw_path}"
    )))
}

fn quote_arg(raw: &str) -> String {
    format!("\"{}\"", raw.replace('\\', "\\\\").replace('"', "\\\""))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AgentTool, SessionRuntimeContext, TypedToolHandle};
    use serde_json::json;
    use std::fs as std_fs;
    use tempfile::tempdir;

    fn ctx() -> SessionRuntimeContext {
        SessionRuntimeContext {
            trace_id: "t".into(),
            agent_name: "a".into(),
            behavior: "b".into(),
            step_idx: 0,
            wakeup_id: "w".into(),
            session_id: "s".into(),
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        }
    }

    #[tokio::test]
    async fn grep_defaults_to_files_with_matches_and_filters_by_glob() {
        let dir = tempdir().expect("tempdir");
        std_fs::create_dir_all(dir.path().join("src")).expect("mkdir");
        std_fs::write(dir.path().join("src/lib.rs"), "fn main() {}\n").expect("write");
        std_fs::write(dir.path().join("src/app.ts"), "function main() {}\n").expect("write");

        let tool = TypedToolHandle::with_null_host(GrepTool::new(FileToolConfig::new(dir.path())));
        let result = AgentTool::call(
            &tool,
            &ctx(),
            json!({
                "pattern": "fn|function",
                "glob": "*.rs"
            }),
        )
        .await
        .expect("call");

        assert_eq!(result.details["mode"], "files_with_matches");
        assert_eq!(result.details["numFiles"], 1);
        assert_eq!(result.details["filenames"][0], "src/lib.rs");
    }

    #[tokio::test]
    async fn grep_content_mode_supports_context_and_line_numbers() {
        let dir = tempdir().expect("tempdir");
        std_fs::write(dir.path().join("notes.txt"), "alpha\nbeta\ngamma\n").expect("write");

        let tool = TypedToolHandle::with_null_host(GrepTool::new(FileToolConfig::new(dir.path())));
        let result = AgentTool::call(
            &tool,
            &ctx(),
            json!({
                "pattern": "beta",
                "output_mode": "content",
                "-C": 1
            }),
        )
        .await
        .expect("call");

        let content = result.details["content"].as_str().expect("content");
        assert!(content.contains("notes.txt-1-alpha"));
        assert!(content.contains("notes.txt:2:beta"));
        assert!(content.contains("notes.txt-3-gamma"));
    }

    #[tokio::test]
    async fn grep_count_mode_reports_paginated_counts() {
        let dir = tempdir().expect("tempdir");
        std_fs::write(dir.path().join("one.txt"), "hit\nhit\n").expect("write");
        std_fs::write(dir.path().join("two.txt"), "hit\n").expect("write");

        let tool = TypedToolHandle::with_null_host(GrepTool::new(FileToolConfig::new(dir.path())));
        let result = AgentTool::call(
            &tool,
            &ctx(),
            json!({
                "pattern": "hit",
                "output_mode": "count",
                "head_limit": 1
            }),
        )
        .await
        .expect("call");

        assert_eq!(result.details["numFiles"], 1);
        assert!(result.details["numMatches"].as_u64().unwrap() >= 1);
        assert_eq!(result.details["appliedLimit"], 1);
    }
}
