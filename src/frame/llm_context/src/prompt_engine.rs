//! Pure text template engine. See `notepads/prompt_render_engine.md` §2/§3.
//!
//! Four directives + `{name}` placeholder. Zero agent / session vocabulary.
//! All dynamic values flow through a caller-provided `ValueLoader` trait so
//! the engine itself has no knowledge of session, workspace, database, etc.

use std::collections::{HashMap, HashSet};
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use log::warn;
use serde_json::{Map, Value as Json};
use tokio::fs;
use tokio::process::Command;
use tokio::time::timeout;
use upon::Engine;

const ESCAPED_OPEN_SENTINEL: &str = "\u{001f}ESCAPED_OPEN_BRACE\u{001f}";
const ESCAPED_CLOSE_SENTINEL: &str = "\u{001f}ESCAPED_CLOSE_BRACE\u{001f}";
const MAX_PREPROCESS_PASSES: usize = 32;

/// Static render variables. `env` feeds `__ENV__`; `vars` feeds `{name}` and
/// `__VAR__`'s dynamic context overlay.
#[derive(Default, Clone, Debug)]
pub struct RenderVars {
    pub env: HashMap<String, Json>,
    pub vars: HashMap<String, Json>,
}

impl RenderVars {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<Json>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn with_var(mut self, key: impl Into<String>, value: impl Into<Json>) -> Self {
        self.vars.insert(key.into(), value.into());
        self
    }

    /// Merge `other` on top of `self`. `other` wins on collision.
    pub fn merged(mut self, other: &RenderVars) -> Self {
        for (k, v) in &other.env {
            self.env.insert(k.clone(), v.clone());
        }
        for (k, v) in &other.vars {
            self.vars.insert(k.clone(), v.clone());
        }
        self
    }
}

/// Caller-provided async resolver for `__VAR__`'s `$expr` and `__ENV__`'s
/// `$expr`. Returning `Ok(None)` means "I don't know this name" — the engine
/// treats it as a soft miss (counted, not fatal).
#[async_trait]
pub trait ValueLoader: Send + Sync {
    async fn load(&self, expr: &str) -> Result<Option<Json>, RenderError>;
}

/// Loader that returns `Ok(None)` for everything. Useful when the template
/// only relies on static `RenderVars`.
pub struct NullValueLoader;

#[async_trait]
impl ValueLoader for NullValueLoader {
    async fn load(&self, _expr: &str) -> Result<Option<Json>, RenderError> {
        Ok(None)
    }
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// Per-`__INCLUDE__` byte cap. Default 64 KiB.
    pub max_include_bytes: usize,
    /// Final rendered output cap. Default 256 KiB; over-cap triggers `truncated=true`.
    pub max_total_bytes: usize,
    /// `__EXEC__` per-command timeout. Default 10s.
    pub exec_timeout: Duration,
    /// `__EXEC__` master gate. Default `false` (sandbox-friendly).
    pub allow_exec: bool,
    /// Virtual root for `__INCLUDE__` paths that start with `/`.
    /// When set, `/foo.md` resolves to `<include_root>/foo.md`.
    pub include_root: Option<PathBuf>,
    /// Directory of the root template. Relative `__INCLUDE__` paths resolve
    /// against this directory; nested includes resolve against the included
    /// file's directory.
    pub template_dir: Option<PathBuf>,
    /// `__INCLUDE__` root whitelist. Empty ⇒ file include is fully disabled.
    pub include_roots: Vec<PathBuf>,
    /// Max recursion depth for `__INCLUDE__`-nested templates. Default 8.
    pub max_recursion_depth: u8,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_include_bytes: 64 * 1024,
            max_total_bytes: 256 * 1024,
            exec_timeout: Duration::from_secs(10),
            allow_exec: false,
            include_root: None,
            template_dir: None,
            include_roots: Vec::new(),
            max_recursion_depth: 8,
        }
    }
}

#[derive(Default, Debug, Clone)]
pub struct RenderStats {
    pub env_expanded: u32,
    pub env_not_found: u32,
    pub content_loaded: u32,
    pub content_failed: u32,
    pub exec_run: u32,
    pub exec_failed: u32,
    pub var_registered: u32,
    pub var_failed: u32,
}

#[derive(Debug)]
pub struct RenderResult {
    pub rendered: String,
    pub stats: RenderStats,
    /// `__VAR__` registrations + whether resolution succeeded.
    pub resolved_vars: HashMap<String, bool>,
    /// Whether the output hit `max_total_bytes` and was truncated.
    pub truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("template syntax error: {0}")]
    Syntax(String),
    #[error("recursion depth exceeded ({0})")]
    RecursionTooDeep(u8),
    #[error("loader failure: {0}")]
    Loader(String),
}

pub struct PromptRenderEngine {
    config: EngineConfig,
}

impl PromptRenderEngine {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    pub fn with_defaults() -> Self {
        Self::new(EngineConfig::default())
    }

    pub fn config(&self) -> &EngineConfig {
        &self.config
    }

    /// Render a template. See module doc for syntax rules.
    pub async fn render<L>(
        &self,
        template: &str,
        vars: &RenderVars,
        loader: &L,
    ) -> Result<RenderResult, RenderError>
    where
        L: ValueLoader + ?Sized,
    {
        let mut stats = RenderStats::default();
        let mut resolved_vars: HashMap<String, bool> = HashMap::new();
        let mut render_ctx: Map<String, Json> = Map::new();
        // Seed render_ctx with static vars so `{name}` works without an
        // explicit `__VAR__` declaration when the value is known up front.
        for (k, v) in &vars.vars {
            render_ctx.insert(k.clone(), v.clone());
        }

        let prepared = prepare_prompt_template(template);
        let preprocessed = self
            .preprocess_with_depth(
                prepared.as_str(),
                vars,
                loader,
                &mut stats,
                &mut resolved_vars,
                &mut render_ctx,
                self.config.template_dir.as_deref(),
                0,
            )
            .await?;

        if preprocessed.contains("__OPENDAN_") || contains_unhandled_directive(&preprocessed) {
            return Err(RenderError::Syntax(format!(
                "unresolved directive remains after preprocessing: {}",
                truncate_chars(&preprocessed, 160)
            )));
        }

        let escaped = escape_template_literals(&preprocessed);
        let mut engine = Engine::new();
        engine
            .add_template("text_template", &escaped)
            .map_err(|err| RenderError::Syntax(format!("add text template failed: {err}")))?;

        let rendered_raw = engine
            .template("text_template")
            .render(&render_ctx)
            .to_string()
            .map_err(|err| RenderError::Syntax(format!("render failed: {err}")))?;
        let unescaped = unescape_template_literals(&rendered_raw);
        let truncated = unescaped.len() > self.config.max_total_bytes;
        let rendered = truncate_utf8(&unescaped, self.config.max_total_bytes);

        Ok(RenderResult {
            rendered,
            stats,
            resolved_vars,
            truncated,
        })
    }

    fn preprocess_with_depth<'a, L>(
        &'a self,
        input: &'a str,
        vars: &'a RenderVars,
        loader: &'a L,
        stats: &'a mut RenderStats,
        resolved_vars: &'a mut HashMap<String, bool>,
        render_ctx: &'a mut Map<String, Json>,
        current_dir: Option<&'a Path>,
        depth: u8,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, RenderError>> + Send + 'a>>
    where
        L: ValueLoader + ?Sized,
    {
        Box::pin(async move {
            if depth > self.config.max_recursion_depth {
                return Err(RenderError::RecursionTooDeep(
                    self.config.max_recursion_depth,
                ));
            }

            let mut preprocessed = input.to_string();
            for _ in 0..MAX_PREPROCESS_PASSES {
                let (next, changed) = self
                    .preprocess_one_pass(
                        preprocessed.as_str(),
                        vars,
                        loader,
                        stats,
                        resolved_vars,
                        render_ctx,
                        current_dir,
                        depth,
                    )
                    .await?;
                preprocessed = next;
                if !changed {
                    break;
                }
            }
            Ok(preprocessed)
        })
    }

    async fn preprocess_one_pass<L>(
        &self,
        input: &str,
        vars: &RenderVars,
        loader: &L,
        stats: &mut RenderStats,
        resolved_vars: &mut HashMap<String, bool>,
        render_ctx: &mut Map<String, Json>,
        current_dir: Option<&Path>,
        depth: u8,
    ) -> Result<(String, bool), RenderError>
    where
        L: ValueLoader + ?Sized,
    {
        const ENV_TOKEN: &str = "__ENV(";
        const INCLUDE_TOKEN: &str = "__INCLUDE(";
        const EXEC_TOKEN: &str = "__EXEC(";
        const VAR_TOKEN: &str = "__VAR(";
        const TOKEN_PREFIX: &str = "__";

        let mut output = String::with_capacity(input.len());
        let mut cursor = 0usize;
        let mut changed = false;

        while let Some(rel) = input[cursor..].find(TOKEN_PREFIX) {
            let start = cursor + rel;
            output.push_str(&input[cursor..start]);
            let tail = &input[start..];

            // Recognised directives all end with `)__`.
            let directive = if tail.starts_with(ENV_TOKEN) {
                Some((ENV_TOKEN.len(), "env"))
            } else if tail.starts_with(INCLUDE_TOKEN) {
                Some((INCLUDE_TOKEN.len(), "include"))
            } else if tail.starts_with(EXEC_TOKEN) {
                Some((EXEC_TOKEN.len(), "exec"))
            } else if tail.starts_with(VAR_TOKEN) {
                Some((VAR_TOKEN.len(), "var"))
            } else {
                None
            };

            let Some((open_len, kind)) = directive else {
                // Not a directive — keep the literal `__` and advance.
                output.push_str(&input[start..start + 2]);
                cursor = start + 2;
                continue;
            };

            let arg_start = start + open_len;
            let Some(rel_end) = input[arg_start..].find(")__") else {
                return Err(RenderError::Syntax(format!(
                    "malformed __{}__ directive near `{}`",
                    kind.to_uppercase(),
                    truncate_chars(&input[start..], 64)
                )));
            };
            let arg_end = arg_start + rel_end;
            let raw_arg = input[arg_start..arg_end].trim();
            let after = arg_end + 3; // skip `)__`

            match kind {
                "env" => {
                    let value = resolve_env_or_dynamic(raw_arg, vars, loader).await?;
                    if value.is_some() {
                        stats.env_expanded = stats.env_expanded.saturating_add(1);
                    } else {
                        stats.env_not_found = stats.env_not_found.saturating_add(1);
                    }
                    output.push_str(
                        &value
                            .as_ref()
                            .and_then(json_value_to_compact_text)
                            .unwrap_or_else(|| failed_marker("env", "not found")),
                    );
                }
                "include" => {
                    match self
                        .load_include(
                            raw_arg,
                            vars,
                            loader,
                            stats,
                            resolved_vars,
                            render_ctx,
                            current_dir,
                            depth,
                        )
                        .await
                    {
                        Ok(content) => {
                            stats.content_loaded = stats.content_loaded.saturating_add(1);
                            output.push_str(content.as_str());
                        }
                        Err(reason) => {
                            stats.content_failed = stats.content_failed.saturating_add(1);
                            warn!("prompt_engine __INCLUDE__ failed: {reason}");
                            output.push_str(&failed_marker("include", reason.as_str()));
                        }
                    }
                }
                "exec" => {
                    if !self.config.allow_exec {
                        stats.exec_failed = stats.exec_failed.saturating_add(1);
                        output.push_str(&failed_marker("exec", "disabled by config"));
                    } else {
                        match self.run_exec(raw_arg, vars, loader).await {
                            Ok(text) => {
                                stats.exec_run = stats.exec_run.saturating_add(1);
                                output.push_str(text.as_str());
                            }
                            Err(reason) => {
                                stats.exec_failed = stats.exec_failed.saturating_add(1);
                                warn!("prompt_engine __EXEC__ failed: {reason}");
                                output.push_str(&failed_marker("exec", reason.as_str()));
                            }
                        }
                    }
                }
                "var" => {
                    let Some((name_raw, expr_raw)) = raw_arg.split_once(',') else {
                        return Err(RenderError::Syntax(
                            "__VAR__ requires `var_name, $expr`".to_string(),
                        ));
                    };
                    let name = name_raw.trim();
                    if !is_variable_name(name) {
                        return Err(RenderError::Syntax(format!(
                            "invalid __VAR__ name `{name}`"
                        )));
                    }
                    let expr = expr_raw.trim();
                    let value = resolve_env_or_dynamic(expr, vars, loader).await?;
                    let resolved_entry = resolved_vars.entry(name.to_string()).or_insert(false);
                    if let Some(v) = value {
                        render_ctx.insert(name.to_string(), v);
                        stats.var_registered = stats.var_registered.saturating_add(1);
                        *resolved_entry = true;
                    } else {
                        render_ctx
                            .entry(name.to_string())
                            .or_insert(Json::String(String::new()));
                        stats.var_failed = stats.var_failed.saturating_add(1);
                    }
                }
                _ => unreachable!(),
            }

            cursor = after;
            changed = true;
        }

        if cursor < input.len() {
            output.push_str(&input[cursor..]);
        }

        Ok((output, changed))
    }

    async fn load_include<L>(
        &self,
        path_arg: &str,
        vars: &RenderVars,
        loader: &L,
        stats: &mut RenderStats,
        resolved_vars: &mut HashMap<String, bool>,
        render_ctx: &mut Map<String, Json>,
        current_dir: Option<&Path>,
        depth: u8,
    ) -> Result<String, String>
    where
        L: ValueLoader + ?Sized,
    {
        if self.config.include_roots.is_empty() {
            return Err("no include_roots configured".to_string());
        }

        let raw_path = if path_arg.starts_with('$') {
            resolve_env_or_dynamic(path_arg, vars, loader)
                .await
                .map_err(|err| format!("resolve $expr failed: {err}"))?
                .as_ref()
                .and_then(json_value_to_compact_text)
                .ok_or_else(|| format!("path expression `{path_arg}` resolved to empty"))?
        } else {
            path_arg.trim().to_string()
        };

        let expanded = expand_path_env_vars(raw_path.as_str());
        let path = resolve_include_path(
            expanded.trim(),
            self.config.include_root.as_deref(),
            current_dir,
        )?;
        if !path_within_any_root(&path, &self.config.include_roots) {
            return Err(format!(
                "path `{}` not under any include_roots whitelist",
                path.display()
            ));
        }

        let bytes = match fs::read(&path).await {
            Ok(b) => b,
            Err(err) if err.kind() == ErrorKind::NotFound => {
                return Err(format!("file not found: {}", path.display()));
            }
            Err(err) => return Err(format!("read `{}` failed: {err}", path.display())),
        };
        let content = String::from_utf8(bytes)
            .map_err(|err| format!("decode `{}` failed: {err}", path.display()))?;
        let bounded = truncate_utf8(&content, self.config.max_include_bytes);

        // Recurse: included content may itself contain directives.
        let nested = self
            .preprocess_with_depth(
                bounded.as_str(),
                vars,
                loader,
                stats,
                resolved_vars,
                render_ctx,
                path.parent(),
                depth + 1,
            )
            .await
            .map_err(|err| format!("nested render failed: {err}"))?;
        Ok(nested)
    }

    async fn run_exec<L>(
        &self,
        command_arg: &str,
        vars: &RenderVars,
        loader: &L,
    ) -> Result<String, String>
    where
        L: ValueLoader + ?Sized,
    {
        let command = parse_exec_command_arg(command_arg)?;
        if command.is_empty() {
            return Err("command is empty".to_string());
        }

        let expanded = expand_exec_command_dynamic_values(command, vars, loader)
            .await
            .map_err(|err| format!("expand $expr failed: {err}"))?;
        let result = timeout(
            self.config.exec_timeout,
            Command::new("sh")
                .arg("-lc")
                .arg(expanded.as_str())
                .output(),
        )
        .await
        .map_err(|_| {
            format!(
                "timed out after {}ms: `{}`",
                self.config.exec_timeout.as_millis(),
                truncate_chars(expanded.as_str(), 160)
            )
        })?
        .map_err(|err| format!("spawn failed: {err}"))?;

        if !result.status.success() {
            let stderr = String::from_utf8_lossy(&result.stderr);
            let stderr = truncate_chars(stderr.trim(), 200);
            let exit_code = result.status.code().unwrap_or_default();
            return Err(format!(
                "exit_code={} stderr={}",
                exit_code,
                if stderr.is_empty() {
                    "<empty>"
                } else {
                    stderr.as_str()
                }
            ));
        }
        let stdout = String::from_utf8_lossy(&result.stdout);
        Ok(truncate_utf8(
            stdout.as_ref(),
            self.config.max_include_bytes,
        ))
    }
}

fn contains_unhandled_directive(text: &str) -> bool {
    // After preprocessing, no recognised directive should remain. Catch
    // accidental partial matches (e.g. `__ENV(` without closing `)__`).
    for token in ["__ENV(", "__INCLUDE(", "__EXEC(", "__VAR("] {
        if text.contains(token) {
            return true;
        }
    }
    false
}

fn failed_marker(directive: &str, reason: &str) -> String {
    let reason = truncate_chars(reason, 160);
    format!(
        "<!-- __{}__ failed: {} -->",
        directive.to_uppercase(),
        reason
    )
}

async fn resolve_env_or_dynamic<L>(
    expr: &str,
    vars: &RenderVars,
    loader: &L,
) -> Result<Option<Json>, RenderError>
where
    L: ValueLoader + ?Sized,
{
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    // Form: `$name` or `$dotted.path`. Bare `name` (no `$`) is rejected to
    // keep the dynamic-vs-static distinction visible at the call site.
    let Some(key) = trimmed.strip_prefix('$').map(str::trim) else {
        return Err(RenderError::Syntax(format!(
            "dynamic expression must start with `$`: `{trimmed}`"
        )));
    };
    if key.is_empty() {
        return Ok(None);
    }

    if let Some(value) = resolve_dotted_path(&vars.env, key) {
        return Ok(Some(value.clone()));
    }
    if let Some(value) = resolve_dotted_path(&vars.vars, key) {
        return Ok(Some(value.clone()));
    }

    match loader.load(key).await? {
        Some(v) => Ok(Some(v)),
        None => Ok(None),
    }
}

fn resolve_dotted_path<'a>(map: &'a HashMap<String, Json>, key: &str) -> Option<&'a Json> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(v) = map.get(trimmed) {
        return Some(v);
    }

    let segments: Vec<&str> = trimmed
        .split('.')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if segments.is_empty() {
        return None;
    }

    for split in (1..=segments.len()).rev() {
        let prefix = segments[..split].join(".");
        let Some(mut current) = map.get(prefix.as_str()) else {
            continue;
        };
        if split == segments.len() {
            return Some(current);
        }
        let mut ok = true;
        for seg in &segments[split..] {
            current = match current {
                Json::Object(m) => {
                    let Some(v) = m.get(*seg) else {
                        ok = false;
                        break;
                    };
                    v
                }
                Json::Array(items) => {
                    let Ok(idx) = seg.parse::<usize>() else {
                        ok = false;
                        break;
                    };
                    let Some(v) = items.get(idx) else {
                        ok = false;
                        break;
                    };
                    v
                }
                _ => {
                    ok = false;
                    break;
                }
            };
        }
        if ok {
            return Some(current);
        }
    }
    None
}

fn parse_exec_command_arg(command_arg: &str) -> Result<&str, String> {
    let trimmed = command_arg.trim();
    if trimmed.is_empty() {
        return Ok(trimmed);
    }
    let first = trimmed.chars().next().unwrap_or_default();
    let last = trimmed.chars().last().unwrap_or_default();
    if (first == '"' || first == '\'') && first != last {
        return Err(format!(
            "malformed command quoting: `{}`",
            truncate_chars(trimmed, 160)
        ));
    }
    if (first == '"' || first == '\'') && trimmed.len() >= 2 {
        return Ok(&trimmed[1..trimmed.len() - 1]);
    }
    Ok(trimmed)
}

async fn expand_exec_command_dynamic_values<L>(
    command: &str,
    vars: &RenderVars,
    loader: &L,
) -> Result<String, RenderError>
where
    L: ValueLoader + ?Sized,
{
    let mut output = String::with_capacity(command.len());
    let mut cursor = 0usize;
    while cursor < command.len() {
        let next_dollar = command[cursor..]
            .find('$')
            .map(|i| cursor + i)
            .unwrap_or(command.len());
        output.push_str(&command[cursor..next_dollar]);
        if next_dollar >= command.len() {
            break;
        }
        let prev_char = command[..next_dollar].chars().last();
        if matches!(prev_char, Some('\\')) {
            output.push('$');
            cursor = next_dollar + 1;
            continue;
        }
        let token_len = command[next_dollar..]
            .chars()
            .take_while(|c| {
                *c == '$' || c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '-')
            })
            .map(char::len_utf8)
            .sum::<usize>();
        if token_len <= 1 {
            output.push('$');
            cursor = next_dollar + 1;
            continue;
        }
        let expr = &command[next_dollar..next_dollar + token_len];
        let resolved = resolve_env_or_dynamic(expr, vars, loader).await?;
        if let Some(text) = resolved.as_ref().and_then(json_value_to_compact_text) {
            output.push_str(text.as_str());
        } else {
            output.push_str(expr);
        }
        cursor = next_dollar + token_len;
    }
    Ok(output)
}

fn resolve_include_path(
    path: &str,
    include_root: Option<&Path>,
    current_dir: Option<&Path>,
) -> Result<PathBuf, String> {
    let requested = PathBuf::from(path);
    if requested.is_absolute() {
        if let Some(root) = include_root {
            return Ok(root.join(strip_virtual_root(&requested)?));
        }
        return Ok(requested);
    }
    if requested
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(format!(
            "relative include path cannot contain `..`: `{path}`"
        ));
    }

    let Some(dir) = current_dir else {
        return Err(format!(
            "relative include path requires current template directory: `{path}`"
        ));
    };
    Ok(dir.join(requested))
}

fn strip_virtual_root(path: &Path) -> Result<PathBuf, String> {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => {}
            Component::CurDir => {}
            Component::Normal(part) => out.push(part),
            Component::ParentDir => {
                return Err(format!(
                    "absolute include path cannot contain `..`: `{}`",
                    path.display()
                ));
            }
            Component::Prefix(_) => {
                return Err(format!(
                    "absolute include path cannot contain platform prefix: `{}`",
                    path.display()
                ));
            }
        }
    }
    Ok(out)
}

fn path_within_any_root(path: &Path, roots: &[PathBuf]) -> bool {
    // Reject `..` components defensively; canonicalisation may be unavailable
    // when the file does not yet exist.
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        return false;
    }
    roots.iter().any(|root| path.starts_with(root))
}

fn expand_path_env_vars(input: &str) -> String {
    // Supports `$HOME`, `${HOME}`, and a leading `~` (alone or `~/...`).
    let mut intermediate = String::new();
    if let Some(rest) = input.strip_prefix("~/") {
        if let Ok(home) = std::env::var("HOME") {
            intermediate.push_str(home.as_str());
            intermediate.push('/');
            intermediate.push_str(rest);
        } else {
            intermediate.push('~');
            intermediate.push('/');
            intermediate.push_str(rest);
        }
    } else if input == "~" {
        if let Ok(home) = std::env::var("HOME") {
            intermediate.push_str(home.as_str());
        } else {
            intermediate.push('~');
        }
    } else {
        intermediate.push_str(input);
    }

    let chars: Vec<char> = intermediate.chars().collect();
    let mut output = String::with_capacity(intermediate.len());
    let mut idx = 0usize;
    while idx < chars.len() {
        if chars[idx] != '$' {
            output.push(chars[idx]);
            idx += 1;
            continue;
        }
        if idx + 1 >= chars.len() {
            output.push('$');
            idx += 1;
            continue;
        }
        if chars[idx + 1] == '{' {
            let mut end = idx + 2;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            if end >= chars.len() {
                output.push('$');
                idx += 1;
                continue;
            }
            let name: String = chars[idx + 2..end].iter().collect();
            output.push_str(std::env::var(name.as_str()).unwrap_or_default().as_str());
            idx = end + 1;
            continue;
        }
        let mut end = idx + 1;
        while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
            end += 1;
        }
        if end == idx + 1 {
            output.push('$');
            idx += 1;
            continue;
        }
        let name: String = chars[idx + 1..end].iter().collect();
        output.push_str(std::env::var(name.as_str()).unwrap_or_default().as_str());
        idx = end;
    }
    output
}

fn escape_template_literals(template: &str) -> String {
    template
        .replace(r"\{{", ESCAPED_OPEN_SENTINEL)
        .replace(r"\}}", ESCAPED_CLOSE_SENTINEL)
}

fn unescape_template_literals(rendered: &str) -> String {
    rendered
        .replace(ESCAPED_OPEN_SENTINEL, "{{")
        .replace(ESCAPED_CLOSE_SENTINEL, "}}")
}

fn json_value_to_compact_text(value: &Json) -> Option<String> {
    match value {
        Json::Null => None,
        Json::String(v) => {
            let trimmed = v.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        }
        _ => serde_json::to_string(value)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
    }
}

fn prepare_prompt_template(template: &str) -> String {
    if template.trim().is_empty() {
        return String::new();
    }

    let escaped = escape_template_literals(template);
    let declared = collect_declared_prompt_vars(escaped.as_str());
    let placeholders = collect_plain_placeholder_vars(escaped.as_str());

    let mut auto_vars: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for name in placeholders {
        if !declared.contains(name.as_str()) && seen.insert(name.clone()) {
            auto_vars.push(name);
        }
    }

    if auto_vars.is_empty() {
        return template.to_string();
    }

    let prologue = auto_vars
        .into_iter()
        .map(|n| format!("__VAR({0}, ${0})__", n))
        .collect::<Vec<_>>()
        .join("");
    format!("{prologue}{template}")
}

fn collect_declared_prompt_vars(template: &str) -> HashSet<String> {
    const VAR_TOKEN_OPEN: &str = "__VAR(";
    let mut declared: HashSet<String> = HashSet::new();
    let mut cursor = 0usize;
    while let Some(start) = template[cursor..].find(VAR_TOKEN_OPEN).map(|i| cursor + i) {
        let arg_start = start + VAR_TOKEN_OPEN.len();
        let Some(arg_end) = template[arg_start..].find(')').map(|i| arg_start + i) else {
            break;
        };
        let raw_args = template[arg_start..arg_end].trim();
        if let Some((name_raw, _)) = raw_args.split_once(',') {
            let name = name_raw.trim();
            if is_variable_name(name) {
                declared.insert(name.to_string());
            }
        }
        cursor = arg_end.saturating_add(1);
    }
    declared
}

fn collect_plain_placeholder_vars(template: &str) -> Vec<String> {
    let mut vars: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut cursor = 0usize;
    while let Some(open) = template[cursor..].find("{{").map(|i| cursor + i) {
        let content_start = open + 2;
        let Some(close) = template[content_start..]
            .find("}}")
            .map(|i| content_start + i)
        else {
            break;
        };
        let placeholder = template[content_start..close].trim();
        if is_variable_name(placeholder) {
            let head = placeholder
                .split('.')
                .next()
                .unwrap_or(placeholder)
                .to_string();
            if seen.insert(head.clone()) {
                vars.push(head);
            }
        }
        cursor = close + 2;
    }
    vars
}

fn is_variable_name(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
}

fn truncate_utf8(text: &str, max_bytes: usize) -> String {
    if text.len() <= max_bytes {
        return text.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut end = max_bytes;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_string()
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut out = String::new();
    for (idx, ch) in text.chars().enumerate() {
        if idx >= max_chars {
            out.push('…');
            break;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;
    use tempfile::tempdir;

    struct StaticLoader {
        values: HashMap<String, Json>,
        calls: Mutex<Vec<String>>,
    }

    impl StaticLoader {
        fn new(values: HashMap<String, Json>) -> Self {
            Self {
                values,
                calls: Mutex::new(Vec::new()),
            }
        }
    }

    #[async_trait]
    impl ValueLoader for StaticLoader {
        async fn load(&self, expr: &str) -> Result<Option<Json>, RenderError> {
            self.calls.lock().unwrap().push(expr.to_string());
            Ok(self.values.get(expr).cloned())
        }
    }

    #[tokio::test]
    async fn renders_plain_placeholder() {
        let engine = PromptRenderEngine::with_defaults();
        let mut vars = RenderVars::new();
        vars.vars
            .insert("name".to_string(), Json::String("alice".to_string()));
        let result = engine
            .render("hi {{ name }}", &vars, &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "hi alice");
        assert!(!result.truncated);
    }

    #[tokio::test]
    async fn escapes_literal_braces() {
        let engine = PromptRenderEngine::with_defaults();
        let result = engine
            .render(
                r"keep \{{ raw \}} pass",
                &RenderVars::new(),
                &NullValueLoader,
            )
            .await
            .unwrap();
        assert_eq!(result.rendered, "keep {{ raw }} pass");
    }

    #[tokio::test]
    async fn env_directive_hits_static_then_loader() {
        let engine = PromptRenderEngine::with_defaults();
        let mut vars = RenderVars::new();
        vars.env
            .insert("session_id".to_string(), Json::String("s-1".to_string()));
        let loader = StaticLoader::new(HashMap::new());
        let result = engine
            .render("id=__ENV($session_id)__", &vars, &loader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "id=s-1");
        assert_eq!(result.stats.env_expanded, 1);
        assert_eq!(result.stats.env_not_found, 0);
    }

    #[tokio::test]
    async fn env_directive_dotted_path() {
        let engine = PromptRenderEngine::with_defaults();
        let mut vars = RenderVars::new();
        vars.env.insert(
            "owner".to_string(),
            json!({ "name": "bob", "did": "did:test:1" }),
        );
        let result = engine
            .render("name=__ENV($owner.name)__", &vars, &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "name=bob");
    }

    #[tokio::test]
    async fn env_directive_miss_counted() {
        let engine = PromptRenderEngine::with_defaults();
        let result = engine
            .render("v=__ENV($missing)__!", &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.stats.env_not_found, 1);
        assert!(result.rendered.starts_with("v=<!-- __ENV__"));
        assert!(result.rendered.ends_with("-->!"));
    }

    #[tokio::test]
    async fn var_directive_resolves_and_substitutes() {
        let engine = PromptRenderEngine::with_defaults();
        let mut values = HashMap::new();
        values.insert(
            "session_title".to_string(),
            Json::String("hello world".to_string()),
        );
        let loader = StaticLoader::new(values);
        let template = "__VAR(title, $session_title)__title={{ title }}";
        let result = engine
            .render(template, &RenderVars::new(), &loader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "title=hello world");
        assert_eq!(result.stats.var_registered, 1);
        assert_eq!(result.resolved_vars.get("title"), Some(&true));
    }

    #[tokio::test]
    async fn auto_var_for_plain_placeholder() {
        let engine = PromptRenderEngine::with_defaults();
        let mut values = HashMap::new();
        values.insert("user".to_string(), Json::String("zoe".to_string()));
        let loader = StaticLoader::new(values);
        let result = engine
            .render("hi {{ user }}", &RenderVars::new(), &loader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "hi zoe");
    }

    #[tokio::test]
    async fn var_failure_counted_not_fatal() {
        let engine = PromptRenderEngine::with_defaults();
        let loader = StaticLoader::new(HashMap::new());
        let template = "__VAR(t, $nope)__[{{ t }}]";
        let result = engine
            .render(template, &RenderVars::new(), &loader)
            .await
            .unwrap();
        assert_eq!(result.stats.var_failed, 1);
        assert_eq!(result.resolved_vars.get("t"), Some(&false));
        assert_eq!(result.rendered, "[]");
    }

    #[tokio::test]
    async fn include_disabled_when_no_roots() {
        let engine = PromptRenderEngine::with_defaults();
        let result = engine
            .render(
                "before __INCLUDE(/tmp/foo)__ after",
                &RenderVars::new(),
                &NullValueLoader,
            )
            .await
            .unwrap();
        assert_eq!(result.stats.content_failed, 1);
        assert!(result.rendered.contains("__INCLUDE__ failed"));
    }

    #[tokio::test]
    async fn include_loads_and_recurses() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("inc.txt");
        std::fs::write(&path, "INNER {{ name }}").unwrap();

        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let mut vars = RenderVars::new();
        vars.vars
            .insert("name".to_string(), Json::String("x".to_string()));
        let template = format!("[__INCLUDE({})__]", path.display());
        let result = engine
            .render(&template, &vars, &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "[INNER x]");
        assert_eq!(result.stats.content_loaded, 1);
    }

    #[tokio::test]
    async fn include_resolves_relative_path_under_roots() {
        let dir = tempdir().unwrap();
        std::fs::write(dir.path().join("role.md"), "ROLE").unwrap();
        let snippets = dir.path().join("behaviors");
        std::fs::create_dir(&snippets).unwrap();
        std::fs::write(snippets.join("rule.inc"), "RULE __INCLUDE(./nested.inc)__").unwrap();
        std::fs::write(snippets.join("nested.inc"), "NESTED").unwrap();

        let cfg = EngineConfig {
            include_root: Some(dir.path().to_path_buf()),
            template_dir: Some(snippets),
            include_roots: vec![dir.path().to_path_buf()],
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let result = engine
            .render(
                "__INCLUDE(/role.md)__\n__INCLUDE(./rule.inc)__",
                &RenderVars::new(),
                &NullValueLoader,
            )
            .await
            .unwrap();
        assert_eq!(result.rendered, "ROLE\nRULE NESTED");
        assert_eq!(result.stats.content_loaded, 3);
    }

    #[tokio::test]
    async fn include_rejects_relative_parent_escape() {
        let dir = tempdir().unwrap();
        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let result = engine
            .render(
                "__INCLUDE(../secret.txt)__",
                &RenderVars::new(),
                &NullValueLoader,
            )
            .await
            .unwrap();
        assert_eq!(result.stats.content_failed, 1);
        assert!(result.rendered.contains("cannot contain `..`"));
    }

    #[tokio::test]
    async fn include_rejects_relative_path_without_template_dir() {
        let dir = tempdir().unwrap();
        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let result = engine
            .render("__INCLUDE(role.md)__", &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.stats.content_failed, 1);
        assert!(result.rendered.contains("current template directory"));
    }

    #[tokio::test]
    async fn include_rejects_outside_root() {
        let dir = tempdir().unwrap();
        let outside = tempdir().unwrap();
        let path = outside.path().join("escape.txt");
        std::fs::write(&path, "nope").unwrap();
        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let template = format!("__INCLUDE({})__", path.display());
        let result = engine
            .render(&template, &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.stats.content_failed, 1);
        assert!(result.rendered.contains("not under any include_roots"));
    }

    #[tokio::test]
    async fn include_byte_cap_truncates() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, "x".repeat(1024)).unwrap();
        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            max_include_bytes: 16,
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let template = format!("__INCLUDE({})__", path.display());
        let result = engine
            .render(&template, &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.rendered.len(), 16);
        assert_eq!(result.stats.content_loaded, 1);
    }

    #[tokio::test]
    async fn include_recursion_depth_limit() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("loop.txt");
        std::fs::write(&path, format!("__INCLUDE({})__", path.display())).unwrap();
        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            max_recursion_depth: 2,
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let template = format!("__INCLUDE({})__", path.display());
        let result = engine
            .render(&template, &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        // Recursion eventually trips the depth gate; the failing include is
        // converted to a soft marker per the "失败可恢复" design rule.
        assert!(result.stats.content_failed >= 1);
        assert!(result.rendered.contains("recursion depth exceeded"));
    }

    #[tokio::test]
    async fn exec_disabled_by_default() {
        let engine = PromptRenderEngine::with_defaults();
        let result = engine
            .render("[__EXEC(echo hi)__]", &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.stats.exec_failed, 1);
        assert!(result.rendered.contains("disabled by config"));
    }

    #[tokio::test]
    async fn exec_runs_when_enabled() {
        let cfg = EngineConfig {
            allow_exec: true,
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let result = engine
            .render(
                "v=__EXEC(printf hi)__!",
                &RenderVars::new(),
                &NullValueLoader,
            )
            .await
            .unwrap();
        assert_eq!(result.rendered, "v=hi!");
        assert_eq!(result.stats.exec_run, 1);
    }

    #[tokio::test]
    async fn exec_timeout_failure_counted() {
        let cfg = EngineConfig {
            allow_exec: true,
            exec_timeout: Duration::from_millis(50),
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let result = engine
            .render("v=__EXEC(sleep 1)__", &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.stats.exec_failed, 1);
        assert!(result.rendered.contains("timed out"));
    }

    #[tokio::test]
    async fn nested_include_directive_runs_var() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("inc.txt");
        std::fs::write(&path, "name=__ENV($name)__").unwrap();
        let cfg = EngineConfig {
            include_roots: vec![dir.path().to_path_buf()],
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let mut vars = RenderVars::new();
        vars.env.insert("name".into(), Json::String("ada".into()));
        let template = format!("[__INCLUDE({})__]", path.display());
        let result = engine
            .render(&template, &vars, &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.rendered, "[name=ada]");
    }

    #[tokio::test]
    async fn truncated_when_over_total_cap() {
        let cfg = EngineConfig {
            max_total_bytes: 4,
            ..EngineConfig::default()
        };
        let engine = PromptRenderEngine::new(cfg);
        let result = engine
            .render("abcdefghij", &RenderVars::new(), &NullValueLoader)
            .await
            .unwrap();
        assert_eq!(result.rendered.len(), 4);
        assert!(result.truncated);
    }

    #[test]
    fn dotted_path_resolves_against_object() {
        let mut map = HashMap::new();
        map.insert("owner".into(), json!({ "name": "x" }));
        let v = resolve_dotted_path(&map, "owner.name").unwrap();
        assert_eq!(v, &Json::String("x".into()));
    }
}
