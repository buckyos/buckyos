use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use tokio::fs;

use crate::{
    parse_default_bash_exec_args, resolve_path_from_root, rewrite_path_with_shell_cwd,
    AgentToolError, CallingConventions, CliInvocation, FileToolConfig, ToolCtx, TypedTool,
};

pub const TOOL_GLOB: &str = "Glob";

const DEFAULT_MAX_GLOB_RESULTS: usize = 100;

const DESCRIPTION: &str = r#"- Fast file pattern matching tool that works with any codebase size
- Supports glob patterns like "**/*.js" or "src/**/*.ts"
- Returns matching file paths sorted by modification time
- Use this tool when you need to find files by name patterns
- When you are doing an open ended search that may require multiple rounds of globbing and grepping, use the Agent tool instead"#;

#[derive(Clone, Debug)]
pub struct GlobTool {
    cfg: FileToolConfig,
    max_results: usize,
}

impl GlobTool {
    pub fn new(cfg: FileToolConfig) -> Self {
        Self {
            cfg,
            max_results: DEFAULT_MAX_GLOB_RESULTS,
        }
    }

    pub fn with_max_results(mut self, max_results: usize) -> Self {
        self.max_results = max_results.max(1);
        self
    }
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GlobArgs {
    pub pattern: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Serialize, JsonSchema)]
pub struct GlobOutput {
    #[serde(rename = "durationMs")]
    pub duration_ms: u64,
    #[serde(rename = "numFiles")]
    pub num_files: usize,
    pub filenames: Vec<String>,
    pub truncated: bool,
}

#[async_trait]
impl TypedTool for GlobTool {
    type Args = GlobArgs;
    type Output = GlobOutput;

    fn name(&self) -> &str {
        TOOL_GLOB
    }

    fn description(&self) -> &str {
        DESCRIPTION
    }

    fn calling(&self) -> CallingConventions {
        CallingConventions::from_legacy(true, false, true)
    }

    fn usage(&self) -> Option<String> {
        Some(
            "Glob <pattern> [path]\n\tpattern: glob such as \"**/*.rs\" or \"src/**/*.ts\""
                .to_string(),
        )
    }

    fn parse_bash_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<Json, AgentToolError> {
        parse_glob_args(tokens, shell_cwd)
    }

    fn parse_cli_args(
        &self,
        tokens: &[String],
        shell_cwd: Option<&Path>,
    ) -> Result<CliInvocation, AgentToolError> {
        Ok(CliInvocation::Json {
            args: parse_glob_args(tokens, shell_cwd)?,
            content_input: None,
        })
    }

    fn build_cmd_line(&self, args: &Self::Args) -> Option<String> {
        let mut cmd = format!("{} pattern={}", TOOL_GLOB, quote_arg(&args.pattern));
        if let Some(path) = args.path.as_deref() {
            cmd.push_str(" path=");
            cmd.push_str(&quote_arg(path));
        }
        Some(cmd)
    }

    fn build_summary(&self, output: &Self::Output) -> String {
        let mut summary = format!(
            "found {} files in {}ms",
            output.num_files, output.duration_ms
        );
        if output.truncated {
            summary.push_str(", truncated");
        }
        if !output.filenames.is_empty() {
            summary.push_str("\n```files\n");
            summary.push_str(&output.filenames.join("\n"));
            summary.push_str("\n```");
        }
        summary
    }

    fn build_title(&self, output: &Self::Output) -> Option<String> {
        Some(format!(
            "{} => found {} files{}",
            TOOL_GLOB,
            output.num_files,
            if output.truncated { " (truncated)" } else { "" }
        ))
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
        let mut search_dir = match args
            .path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            Some(path) => resolve_path_from_root(&self.cfg.root_dir, path)?,
            None => self.cfg.root_dir.clone(),
        };
        ensure_read_path_allowed(&self.cfg, &search_dir, args.path.as_deref().unwrap_or("."))?;

        let display_search_path = args
            .path
            .as_deref()
            .map(str::to_string)
            .unwrap_or_else(|| search_dir.to_string_lossy().to_string());
        let metadata = fs::metadata(&search_dir).await.map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                AgentToolError::InvalidArgs(format!(
                    "Directory does not exist: {}",
                    display_search_path
                ))
            } else {
                AgentToolError::ExecFailed(format!(
                    "stat directory `{}` failed: {err}",
                    search_dir.display()
                ))
            }
        })?;
        if !metadata.is_dir() {
            return Err(AgentToolError::InvalidArgs(format!(
                "Path is not a directory: {}",
                display_search_path
            )));
        }

        let mut search_pattern = pattern.clone();
        if Path::new(&pattern).is_absolute() {
            let (base_dir, relative_pattern) = extract_glob_base_directory(&pattern);
            if !base_dir.as_os_str().is_empty() {
                search_dir = base_dir;
                ensure_read_path_allowed(&self.cfg, &search_dir, &pattern)?;
                let metadata = fs::metadata(&search_dir).await.map_err(|err| {
                    if err.kind() == std::io::ErrorKind::NotFound {
                        AgentToolError::InvalidArgs(format!(
                            "Directory does not exist: {}",
                            search_dir.display()
                        ))
                    } else {
                        AgentToolError::ExecFailed(format!(
                            "stat directory `{}` failed: {err}",
                            search_dir.display()
                        ))
                    }
                })?;
                if !metadata.is_dir() {
                    return Err(AgentToolError::InvalidArgs(format!(
                        "Path is not a directory: {}",
                        search_dir.display()
                    )));
                }
                search_pattern = relative_pattern;
            }
        }

        let root_dir = self.cfg.root_dir.clone();
        let max_results = self.max_results;
        let matches = tokio::task::spawn_blocking(move || {
            collect_glob_matches(&search_dir, &search_pattern, &root_dir, max_results)
        })
        .await
        .map_err(|err| AgentToolError::ExecFailed(format!("glob task failed: {err}")))??;

        Ok(GlobOutput {
            duration_ms: start.elapsed().as_millis() as u64,
            num_files: matches.files.len(),
            filenames: matches.files,
            truncated: matches.truncated,
        })
    }
}

fn parse_glob_args(tokens: &[String], shell_cwd: Option<&Path>) -> Result<Json, AgentToolError> {
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
                "Glob args cannot mix positional args with key=value args".to_string(),
            ));
        }
        parse_default_bash_exec_args(tokens)?
    } else {
        if tokens.len() > 2 {
            return Err(AgentToolError::InvalidArgs(format!(
                "too many positional args for tool `{}`: got {}, max 2",
                TOOL_GLOB,
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
        AgentToolError::InvalidArgs("Glob args must be a json object".to_string())
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
                "Glob arg `path` must be a string".to_string(),
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

struct GlobMatches {
    files: Vec<String>,
    truncated: bool,
}

#[derive(Debug)]
struct Candidate {
    path: PathBuf,
    modified: SystemTime,
}

fn collect_glob_matches(
    search_dir: &Path,
    pattern: &str,
    output_root: &Path,
    limit: usize,
) -> Result<GlobMatches, AgentToolError> {
    let matcher = GlobMatcher::new(pattern)?;
    let mut candidates = Vec::new();
    visit_files(search_dir, search_dir, &matcher, &mut candidates)?;
    candidates.sort_by(|a, b| {
        a.modified
            .cmp(&b.modified)
            .then_with(|| path_to_slash_string(&a.path).cmp(&path_to_slash_string(&b.path)))
    });

    let truncated = candidates.len() > limit;
    let files = candidates
        .into_iter()
        .take(limit)
        .map(|candidate| display_path(&candidate.path, output_root))
        .collect();
    Ok(GlobMatches { files, truncated })
}

fn visit_files(
    dir: &Path,
    base: &Path,
    matcher: &GlobMatcher,
    out: &mut Vec<Candidate>,
) -> Result<(), AgentToolError> {
    let entries = std::fs::read_dir(dir).map_err(|err| {
        AgentToolError::ExecFailed(format!("read directory `{}` failed: {err}", dir.display()))
    })?;
    for entry in entries {
        let entry = entry.map_err(|err| {
            AgentToolError::ExecFailed(format!("read directory entry failed: {err}"))
        })?;
        let path = entry.path();
        let file_type = entry.file_type().map_err(|err| {
            AgentToolError::ExecFailed(format!("read file type `{}` failed: {err}", path.display()))
        })?;
        if file_type.is_dir() {
            visit_files(&path, base, matcher, out)?;
            continue;
        }

        let metadata = match std::fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_file() {
            continue;
        }
        let rel = path.strip_prefix(base).unwrap_or(path.as_path());
        if matcher.matches_path(rel) {
            out.push(Candidate {
                path,
                modified: metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    Ok(())
}

#[derive(Debug)]
struct GlobMatcher {
    segments: Vec<String>,
    basename_only: bool,
}

impl GlobMatcher {
    fn new(pattern: &str) -> Result<Self, AgentToolError> {
        let normalized = normalize_pattern(pattern);
        if normalized.trim().is_empty() {
            return Err(AgentToolError::InvalidArgs(
                "pattern cannot be empty".to_string(),
            ));
        }
        let basename_only = !normalized.contains('/');
        let segments = normalized
            .split('/')
            .filter(|segment| !segment.is_empty())
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        Ok(Self {
            segments,
            basename_only,
        })
    }

    fn matches_path(&self, path: &Path) -> bool {
        let rel = path_to_slash_string(path);
        if self.basename_only {
            return path
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| match_segment(&self.segments[0], name))
                .unwrap_or(false);
        }
        let path_segments = rel
            .split('/')
            .filter(|segment| !segment.is_empty())
            .collect::<Vec<_>>();
        match_segments(&self.segments, &path_segments)
    }
}

fn match_segments(patterns: &[String], path: &[&str]) -> bool {
    if patterns.is_empty() {
        return path.is_empty();
    }
    if patterns[0] == "**" {
        if match_segments(&patterns[1..], path) {
            return true;
        }
        return !path.is_empty() && match_segments(patterns, &path[1..]);
    }
    if path.is_empty() {
        return false;
    }
    match_segment(&patterns[0], path[0]) && match_segments(&patterns[1..], &path[1..])
}

fn match_segment(pattern: &str, text: &str) -> bool {
    let pattern = expand_braces(pattern);
    pattern.iter().any(|pat| match_segment_inner(pat, text))
}

fn match_segment_inner(pattern: &str, text: &str) -> bool {
    let p = pattern.chars().collect::<Vec<_>>();
    let t = text.chars().collect::<Vec<_>>();
    match_segment_from(&p, 0, &t, 0)
}

fn match_segment_from(pattern: &[char], pi: usize, text: &[char], ti: usize) -> bool {
    if pi == pattern.len() {
        return ti == text.len();
    }
    match pattern[pi] {
        '*' => {
            let mut next = ti;
            while next <= text.len() {
                if match_segment_from(pattern, pi + 1, text, next) {
                    return true;
                }
                next += 1;
            }
            false
        }
        '?' => ti < text.len() && match_segment_from(pattern, pi + 1, text, ti + 1),
        '[' => {
            if ti >= text.len() {
                return false;
            }
            if let Some((matched, next_pi)) = match_char_class(pattern, pi, text[ti]) {
                matched && match_segment_from(pattern, next_pi, text, ti + 1)
            } else {
                text[ti] == '[' && match_segment_from(pattern, pi + 1, text, ti + 1)
            }
        }
        '\\' => {
            if pi + 1 < pattern.len() {
                ti < text.len()
                    && pattern[pi + 1] == text[ti]
                    && match_segment_from(pattern, pi + 2, text, ti + 1)
            } else {
                ti < text.len()
                    && text[ti] == '\\'
                    && match_segment_from(pattern, pi + 1, text, ti + 1)
            }
        }
        ch => {
            ti < text.len() && ch == text[ti] && match_segment_from(pattern, pi + 1, text, ti + 1)
        }
    }
}

fn match_char_class(pattern: &[char], start: usize, ch: char) -> Option<(bool, usize)> {
    let mut idx = start + 1;
    if idx >= pattern.len() {
        return None;
    }
    let negated = pattern[idx] == '!' || pattern[idx] == '^';
    if negated {
        idx += 1;
    }
    let mut matched = false;
    let mut saw_any = false;
    while idx < pattern.len() {
        if pattern[idx] == ']' && saw_any {
            return Some((if negated { !matched } else { matched }, idx + 1));
        }
        let current = pattern[idx];
        if idx + 2 < pattern.len() && pattern[idx + 1] == '-' && pattern[idx + 2] != ']' {
            let end = pattern[idx + 2];
            if current <= ch && ch <= end {
                matched = true;
            }
            idx += 3;
        } else {
            if current == ch {
                matched = true;
            }
            idx += 1;
        }
        saw_any = true;
    }
    None
}

fn expand_braces(pattern: &str) -> Vec<String> {
    let Some(open) = pattern.find('{') else {
        return vec![pattern.to_string()];
    };
    let Some(close_rel) = pattern[open + 1..].find('}') else {
        return vec![pattern.to_string()];
    };
    let close = open + 1 + close_rel;
    let prefix = &pattern[..open];
    let suffix = &pattern[close + 1..];
    pattern[open + 1..close]
        .split(',')
        .flat_map(|part| expand_braces(&format!("{prefix}{part}{suffix}")))
        .collect()
}

fn extract_glob_base_directory(pattern: &str) -> (PathBuf, String) {
    let first_glob = pattern
        .char_indices()
        .find(|(_, ch)| matches!(ch, '*' | '?' | '[' | '{'))
        .map(|(idx, _)| idx);
    let Some(glob_idx) = first_glob else {
        let path = Path::new(pattern);
        let base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        let file = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(pattern)
            .to_string();
        return (base, file);
    };
    let static_prefix = &pattern[..glob_idx];
    let last_sep = static_prefix.rfind('/');
    match last_sep {
        Some(0) => (PathBuf::from("/"), pattern[1..].to_string()),
        Some(idx) => (
            PathBuf::from(&pattern[..idx]),
            pattern[idx + 1..].to_string(),
        ),
        None => (PathBuf::new(), pattern.to_string()),
    }
}

fn normalize_pattern(pattern: &str) -> String {
    pattern.trim().replace('\\', "/")
}

fn path_to_slash_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn display_path(path: &Path, output_root: &Path) -> String {
    path.strip_prefix(output_root)
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
    async fn glob_matches_recursive_pattern_and_truncates() {
        let dir = tempdir().expect("tempdir");
        std_fs::create_dir_all(dir.path().join("src/nested")).expect("mkdir");
        std_fs::write(dir.path().join("src/lib.rs"), "lib").expect("write");
        std_fs::write(dir.path().join("src/nested/mod.rs"), "mod").expect("write");
        std_fs::write(dir.path().join("src/readme.md"), "md").expect("write");

        let tool = TypedToolHandle::with_null_host(
            GlobTool::new(FileToolConfig::new(dir.path())).with_max_results(1),
        );
        let result = AgentTool::call(
            &tool,
            &ctx(),
            json!({
                "pattern": "src/**/*.rs"
            }),
        )
        .await
        .expect("call");

        assert_eq!(result.details["numFiles"], 1);
        assert_eq!(result.details["truncated"], true);
        assert!(result.details["filenames"][0]
            .as_str()
            .unwrap()
            .ends_with(".rs"));
    }

    #[test]
    fn glob_matcher_supports_basename_and_braces() {
        let matcher = GlobMatcher::new("*.{rs,ts}").expect("matcher");
        assert!(matcher.matches_path(Path::new("src/lib.rs")));
        assert!(matcher.matches_path(Path::new("web/app.ts")));
        assert!(!matcher.matches_path(Path::new("web/app.js")));
    }
}
