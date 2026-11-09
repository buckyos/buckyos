//! The standard environment baseline (primary-runtime TODO §2, T1): the
//! commands every session-runtime declared `standard` provides. One list
//! serves the image build (`xagent runtime baseline --check`), host
//! installation checks, the runtime check that records `baseline_check`
//! and the manuals (`agent-session runtime baseline`).
//!
//! An `extended` runtime has all of these plus [`EXTENDED_TOOLS`], the
//! `provides` of the core extension package (`publish/exttool/ext-core`, a
//! test keeps the two in step). Other extension packages do not change a
//! baseline: a cmd_skill that needs their commands names them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use buckyos_api::RuntimeBaseline;
use serde::Serialize;

use crate::error::OpenDanError;
use crate::protocol::{ExtCheck, ExtStatus, ProvidedBy, ResolvedCommand};

/// What a baseline command is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCategory {
    /// Capabilities of the runtime itself (sub LLM contexts).
    Runtime,
    /// The session's own state and the Agent State (through AgentService).
    AgentState,
    /// BuckyOS / AICC service calls under the Agent's identity.
    BuckyosService,
    /// Media input (understanding) and output (generation).
    Media,
}

#[derive(Debug, Clone, Serialize)]
pub struct BaselineTool {
    /// The command name on PATH.
    pub name: &'static str,
    pub category: ToolCategory,
    /// Where its implementation is.
    pub source: &'static str,
    /// In a runtime that shares the agent process's machine, the session's
    /// bin overlay provides this command from that executable instead of
    /// PATH.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_helper_of: Option<&'static str>,
}

const XAGENT_SOURCE: &str = "src/frame/lib_opendan (xagent_cli.rs)";
const AGENT_TOOL_SOURCE: &str = "src/frame/agent_tool_cli_dev";
const AGENT_TOOLS_SOURCE: &str = "src/tools/buckyos-agent/commands";

const fn tool(name: &'static str, category: ToolCategory, source: &'static str) -> BaselineTool {
    BaselineTool {
        name,
        category,
        source,
        session_helper_of: None,
    }
}

const fn helper(name: &'static str, category: ToolCategory, source: &'static str, of: &'static str) -> BaselineTool {
    BaselineTool {
        name,
        category,
        source,
        session_helper_of: Some(of),
    }
}

/// The commands of the standard environment. The media and service
/// commands of `buckyos-agent` are its `commands/*.ts` (a test keeps the
/// two in step).
pub const STANDARD_TOOLS: &[BaselineTool] = &[
    tool("agent_tool", ToolCategory::Runtime, AGENT_TOOL_SOURCE),
    helper("agent-session", ToolCategory::AgentState, XAGENT_SOURCE, "xagent"),
    tool("xagent", ToolCategory::AgentState, XAGENT_SOURCE),
    tool("buckyos", ToolCategory::BuckyosService, "buckyos-websdk cli/modules (launcher: publish/aios/buckyos)"),
    tool("aicc-tool", ToolCategory::BuckyosService, "src/tools/buckyos-agent/aicc-tool.ts"),
    tool("ai_provider", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("ai_provider_test", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("ai_quota", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("ask_user", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("import_office", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("import_table", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("materialize_resource", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    tool("resume_aicc_task", ToolCategory::BuckyosService, AGENT_TOOLS_SOURCE),
    helper("llm_understand_media", ToolCategory::Media, AGENT_TOOL_SOURCE, "agent_tool"),
    tool("caption_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("detect_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("edit_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("edit_video", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("enhance_audio", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("extend_video", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("gen_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("gen_music", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("gen_video", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("inpaint_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("ocr_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("remove_bg", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("segment_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("speech_to_text", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("text_to_speech", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("upscale_image", ToolCategory::Media, AGENT_TOOLS_SOURCE),
    tool("upscale_video", ToolCategory::Media, AGENT_TOOLS_SOURCE),
];

/// What an extended tool is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtToolCategory {
    AudioVideo,
    Image,
    Pdf,
    Ocr,
    Document,
    Python,
    JavaScript,
    Utility,
}

/// `Core` tools decide an `extended` baseline; a `Recommended` one is only
/// reported when it is missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtToolTier {
    Core,
    Recommended,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ExtCommand {
    pub name: &'static str,
    pub version_probe: &'static str,
    /// A no-network check, relative to the root of the package's payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub smoke: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct ExtendedTool {
    pub id: &'static str,
    pub category: ExtToolCategory,
    pub tier: ExtToolTier,
    pub commands: &'static [ExtCommand],
}

const fn ext_cmd(name: &'static str, version_probe: &'static str, smoke: Option<&'static str>) -> ExtCommand {
    ExtCommand { name, version_probe, smoke }
}

const fn core_tool(id: &'static str, category: ExtToolCategory, commands: &'static [ExtCommand]) -> ExtendedTool {
    ExtendedTool { id, category, tier: ExtToolTier::Core, commands }
}

/// The tools of an `extended` runtime: what `ext-core` provides.
pub const EXTENDED_TOOLS: &[ExtendedTool] = &[
    core_tool("ffmpeg", ExtToolCategory::AudioVideo, &[
        ext_cmd("ffmpeg", "ffmpeg -version", Some("smoke/ffmpeg.sh")),
        ext_cmd("ffprobe", "ffprobe -version", None),
    ]),
    core_tool("imagemagick", ExtToolCategory::Image, &[
        ext_cmd("magick", "magick -version", Some("smoke/imagemagick.sh")),
        ext_cmd("convert", "convert -version", None),
        ext_cmd("identify", "identify -version", None),
        ext_cmd("compare", "compare -version", None),
    ]),
    core_tool("poppler", ExtToolCategory::Pdf, &[
        ext_cmd("pdftotext", "pdftotext -v", Some("smoke/poppler.sh")),
        ext_cmd("pdftoppm", "pdftoppm -v", None),
        ext_cmd("pdfinfo", "pdfinfo -v", None),
    ]),
    core_tool("tesseract", ExtToolCategory::Ocr, &[ext_cmd("tesseract", "tesseract --version", Some("smoke/tesseract.sh"))]),
    core_tool("pandoc", ExtToolCategory::Document, &[ext_cmd("pandoc", "pandoc --version", Some("smoke/pandoc.sh"))]),
    core_tool("python", ExtToolCategory::Python, &[
        ext_cmd("python3", "python3 --version", Some("smoke/python3.sh")),
        ext_cmd("uv", "uv --version", None),
    ]),
    core_tool("node", ExtToolCategory::JavaScript, &[
        ext_cmd("node", "node --version", Some("smoke/node.sh")),
        ext_cmd("npm", "npm --version", None),
        ext_cmd("npx", "npx --version", None),
    ]),
    core_tool("deno", ExtToolCategory::JavaScript, &[ext_cmd("deno", "deno --version", Some("smoke/deno.sh"))]),
    core_tool("helpers", ExtToolCategory::Utility, &[
        ext_cmd("git", "git --version", Some("smoke/helpers.sh")),
        ext_cmd("curl", "curl --version", None),
        ext_cmd("jq", "jq --version", None),
        ext_cmd("tree", "tree --version", None),
        ext_cmd("unzip", "unzip -v", None),
        ext_cmd("xz", "xz --version", None),
    ]),
];

/// The names a runtime of `level` must have on PATH. `overlay`: the runtime
/// shares the agent process's machine, so session helpers come from the
/// session bin overlay and their providing executable is what must exist.
pub fn required_commands(level: RuntimeBaseline, overlay: bool) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for t in STANDARD_TOOLS {
        let name = match (overlay, t.session_helper_of) {
            (true, Some(of)) => of,
            _ => t.name,
        };
        if !out.contains(&name) {
            out.push(name);
        }
    }
    if level == RuntimeBaseline::Extended {
        out.extend(EXTENDED_TOOLS.iter().filter(|t| t.tier == ExtToolTier::Core).flat_map(|t| t.commands.iter().map(|c| c.name)));
    }
    out
}

/// Where the tools of an Agent keep what they download (ExtTools TODO §3.6): its own directory, so that nothing lands in the
/// account's global caches, npm prefix or interpreters. `HOME` stays; the Agent runs as the account it was given.
/// `llm_understand_media` unpacks long documents there too (doc/llm_context/LLM Understand Media.md §4.7).
pub fn tool_cache_env(cache: &Path) -> Vec<(String, String)> {
    [
        ("DENO_DIR", "deno-cache"),
        ("UV_CACHE_DIR", "uv-cache"),
        ("UV_PYTHON_INSTALL_DIR", "uv-python"),
        ("NPM_CONFIG_CACHE", "npm-cache"),
        ("NPM_CONFIG_PREFIX", "npm-prefix"),
        ("PIP_CACHE_DIR", "pip-cache"),
        ("LLM_UNDERSTAND_MEDIA_CATALOG_DIR", "media-catalog"),
    ]
    .into_iter()
    .map(|(name, dir)| (name.to_string(), cache.join(dir).display().to_string()))
    .collect()
}

/// Every extended command a check looks for.
pub fn extended_commands() -> Vec<&'static str> {
    EXTENDED_TOOLS.iter().flat_map(|t| t.commands.iter().map(|c| c.name)).collect()
}

/// The commands a check of any declaration looks for: the extended ones are
/// asked too, so a runtime declared `standard` still says what it has.
pub fn checked_commands(overlay: bool) -> Vec<&'static str> {
    let mut out = required_commands(RuntimeBaseline::Standard, overlay);
    out.extend(extended_commands());
    out
}

fn is_core_entry_ok(provider: ProvidedBy) -> bool {
    matches!(provider, ProvidedBy::BuckyosRoot | ProvidedBy::Dev)
}

/// What one check found against what `level` declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaselineFindings {
    /// Commands of `level` not there (a stub is not there).
    pub missing: Vec<String>,
    /// Standard commands found outside `$BUCKYOS_ROOT`: their dependencies
    /// are not the pinned ones, so the standard layer is not complete.
    pub unmanaged: Vec<String>,
    pub ext: ExtCheck,
}

impl BaselineFindings {
    pub fn ok(&self) -> bool {
        self.missing.is_empty() && self.unmanaged.is_empty()
    }
}

/// Judge what a check `resolved`. The standard layer is strict about where a
/// command comes from; the extended layer only asks that it is there, and
/// remembers who provides it.
pub fn evaluate(level: RuntimeBaseline, overlay: bool, resolved: &BTreeMap<String, ResolvedCommand>) -> BaselineFindings {
    let present = |name: &str| resolved.get(name).filter(|c| !c.stub);
    let mut missing = Vec::new();
    let mut unmanaged = Vec::new();
    for name in required_commands(RuntimeBaseline::Standard, overlay) {
        match present(name) {
            None => missing.push(name.to_string()),
            Some(c) if !is_core_entry_ok(c.provided_by) => unmanaged.push(name.to_string()),
            Some(_) => {}
        }
    }
    let mut ext_missing = Vec::new();
    let mut system = Vec::new();
    let mut found = 0usize;
    for tool in EXTENDED_TOOLS {
        for name in tool.commands.iter().map(|c| c.name) {
            match present(name) {
                Some(c) => {
                    found += 1;
                    if c.provided_by != ProvidedBy::Exttools {
                        system.push(name.to_string());
                    }
                }
                None => {
                    ext_missing.push(name.to_string());
                    if level == RuntimeBaseline::Extended && tool.tier == ExtToolTier::Core {
                        missing.push(name.to_string());
                    }
                }
            }
        }
    }
    let status = if found == 0 {
        ExtStatus::None
    } else if !ext_missing.is_empty() {
        ExtStatus::Partial
    } else if !system.is_empty() {
        ExtStatus::System
    } else {
        ExtStatus::Ok
    };
    BaselineFindings {
        missing,
        unmanaged,
        ext: ExtCheck { status, missing: ext_missing, system },
    }
}

/// Where a command found on a runtime comes from, by its path.
#[derive(Debug, Clone, Default)]
pub struct CommandOrigin {
    /// `$BUCKYOS_ROOT` of that machine; a machine without one has no
    /// managed tools.
    pub root: Option<String>,
    /// The Agent's own directory on this machine.
    pub agent_root: Option<String>,
    /// A development program's directory: helpers beside it are not installed
    /// but serve as the standard ones.
    pub beside: Option<String>,
    pub windows: bool,
}

impl CommandOrigin {
    fn under(&self, path: &str, dir: &str) -> bool {
        let norm = |p: &str| {
            let p = p.replace('\\', "/");
            let p = p.trim_end_matches('/');
            if self.windows { p.to_ascii_lowercase() } else { p.to_string() }
        };
        let (path, dir) = (norm(path), norm(dir));
        !dir.is_empty() && path.starts_with(&format!("{dir}/"))
    }

    pub fn of(&self, path: &str) -> ProvidedBy {
        let agent_tools = self.agent_root.as_ref().map(|r| format!("{}/{}", r.trim_end_matches(['/', '\\']), crate::skills::TOOLS_DIR));
        if agent_tools.as_deref().is_some_and(|d| self.under(path, d)) || path.replace('\\', "/").contains("/.opendan/") {
            return ProvidedBy::Agent;
        }
        if let Some(root) = &self.root {
            if self.under(path, &format!("{}/tools", root.trim_end_matches(['/', '\\']))) {
                return ProvidedBy::Exttools;
            }
            if self.under(path, root) {
                return ProvidedBy::BuckyosRoot;
            }
        }
        if self.beside.as_deref().is_some_and(|d| self.under(path, d)) {
            return ProvidedBy::Dev;
        }
        ProvidedBy::System
    }
}

pub(crate) fn powershell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// A command that prints each of `names` it cannot find on PATH, one per
/// line, and exits 0: POSIX `sh`, or PowerShell when `windows`.
pub fn missing_probe(names: &[&str], windows: bool) -> String {
    if windows {
        let list = names.iter().map(|n| powershell_quote(n)).collect::<Vec<_>>().join(",");
        format!(
            "foreach ($c in @({list})) {{ if (-not (Get-Command $c -ErrorAction SilentlyContinue)) {{ Write-Output $c }} }}"
        )
    } else {
        let list = names
            .iter()
            .map(|n| agent_tool::runtime::shell_quote(n))
            .collect::<Vec<_>>()
            .join(" ");
        format!("for c in {list}; do command -v \"$c\" >/dev/null 2>&1 || printf '%s\\n' \"$c\"; done; true")
    }
}

/// The missing commands a [`missing_probe`] printed, limited to `asked`.
pub fn parse_missing(stdout: &str, asked: &[&str]) -> Vec<String> {
    stdout
        .lines()
        .map(str::trim)
        .filter(|l| asked.contains(l))
        .map(str::to_string)
        .collect()
}

/// The version probe of a command: the extended tools' own, else `--version`.
pub fn version_probe_of(name: &str) -> String {
    EXTENDED_TOOLS
        .iter()
        .flat_map(|t| t.commands.iter())
        .find(|c| c.name == name)
        .map(|c| c.version_probe.to_string())
        .unwrap_or_else(|| {
            let quoted = if name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-')) {
                name.to_string()
            } else {
                agent_tool::runtime::shell_quote(name)
            };
            format!("{quoted} --version")
        })
}

/// One line per command of `names` found on PATH: `name<TAB>path`, and for
/// those of `versions` also the probe's exit code and first output line.
/// Exits 0: POSIX `sh`, or PowerShell when `windows`.
pub fn resolve_probe(names: &[&str], versions: &[&str], windows: bool) -> String {
    let probe = |n: &&str| if versions.contains(n) { version_probe_of(n) } else { String::new() };
    if windows {
        let calls = names
            .iter()
            .map(|n| format!("p {} {}", powershell_quote(n), powershell_quote(&probe(n))))
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "function p($c,$f) {{ $g = Get-Command $c -ErrorAction SilentlyContinue | Select-Object -First 1; if (-not $g) {{ return }}; if ($f) {{ $global:LASTEXITCODE = 0; $o = try {{ Invoke-Expression \"$f 2>&1\" }} catch {{ $global:LASTEXITCODE = 127; $_ }}; $v = ($o | Select-Object -First 1); Write-Output (\"{{0}}`t{{1}}`t{{2}}`t{{3}}\" -f $c,$g.Source,$LASTEXITCODE,$v) }} else {{ Write-Output (\"{{0}}`t{{1}}\" -f $c,$g.Source) }} }}; {calls}"
        )
    } else {
        let calls = names
            .iter()
            .map(|n| format!("p {} {}", agent_tool::runtime::shell_quote(n), agent_tool::runtime::shell_quote(&probe(n))))
            .collect::<Vec<_>>()
            .join("; ");
        format!(
            "t=; command -v timeout >/dev/null 2>&1 && t='timeout 10'; p() {{ w=\"$(command -v \"$1\" 2>/dev/null)\" || return 0; if [ -n \"$2\" ]; then v=\"$($t sh -c \"$2\" 2>&1 </dev/null)\"; r=$?; printf '%s\\t%s\\t%s\\t%s\\n' \"$1\" \"$w\" \"$r\" \"$(printf '%s\\n' \"$v\" | head -n 1 | tr '\\t' ' ')\"; else printf '%s\\t%s\\n' \"$1\" \"$w\"; fi; }}; {calls}; true"
        )
    }
}

/// A command a [`resolve_probe`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbedCommand {
    pub path: String,
    pub exit_code: Option<i32>,
    pub version: Option<String>,
}

/// What a [`resolve_probe`] printed, limited to `asked`.
pub fn parse_resolved(stdout: &str, asked: &[&str]) -> BTreeMap<String, ProbedCommand> {
    let mut found = BTreeMap::new();
    for line in stdout.lines() {
        let mut fields = line.trim_end_matches('\r').splitn(4, '\t');
        let (Some(name), Some(path)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !asked.contains(&name) || path.is_empty() {
            continue;
        }
        found.insert(
            name.to_string(),
            ProbedCommand {
                path: path.to_string(),
                exit_code: fields.next().and_then(|c| c.parse().ok()),
                version: fields.next().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string),
            },
        );
    }
    found
}

/// A command `exit_code` says does not run: not found or not executable by
/// the shell (a stub explaining how to get the real tool exits 127).
pub fn is_stub_exit(exit_code: Option<i32>) -> bool {
    matches!(exit_code, Some(126 | 127))
}

/// Keep what an earlier check learned of a command that has not moved: a
/// background check only records paths.
pub fn keep_versions(resolved: &mut BTreeMap<String, ResolvedCommand>, before: &BTreeMap<String, ResolvedCommand>) {
    for (name, command) in resolved.iter_mut() {
        if command.version.is_none() {
            if let Some(known) = before.get(name).filter(|b| b.path == command.path) {
                command.version = known.version.clone();
                command.stub = known.stub;
            }
        }
    }
}

fn executable_path_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat", ".ps1"]
            .iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .collect()
    } else {
        vec![dir.join(name)]
    };
    candidates.into_iter().find(|p| {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        }
        #[cfg(not(unix))]
        {
            p.is_file()
        }
    })
}

/// Where this process finds `names`, in the order of its PATH: `before`,
/// then `path` (`None`: its own `PATH`), then `after`.
pub fn resolve_locally(names: &[&str], before: &[PathBuf], path: Option<&std::ffi::OsStr>, after: &[PathBuf]) -> BTreeMap<String, String> {
    let path = path
        .map(|p| p.to_os_string())
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let dirs: Vec<PathBuf> = before.iter().cloned().chain(std::env::split_paths(&path)).chain(after.iter().cloned()).collect();
    names
        .iter()
        .filter_map(|n| {
            let found = dirs.iter().find_map(|d| executable_path_in(d, n))?;
            Some((n.to_string(), found.display().to_string()))
        })
        .collect()
}

/// The directories of this machine where an Agent's own commands are, in PATH
/// order: `tools/<skill>/bin` of every installed skill, then `tools`.
pub fn agent_tool_dirs(agent_root: &Path) -> Vec<PathBuf> {
    let tools = agent_root.join(crate::skills::TOOLS_DIR);
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&tools)
        .map(|entries| entries.flatten().map(|e| e.path().join("bin")).filter(|d| d.is_dir()).collect())
        .unwrap_or_default();
    dirs.sort();
    if tools.is_dir() {
        dirs.push(tools);
    }
    dirs
}

/// The directory of this program: where a development build's helpers are.
pub fn beside_current_exe() -> Option<PathBuf> {
    std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

// ---------------------------------------------------------------------------
// A BuckyOS host as a standard environment (§2.2): one multi-call program
// (`opendan`) answers to every standard command name, so a node's agent
// tool directory (`$BUCKYOS_ROOT/bin/agent-tools`) is only links to it —
// symbolic links where they are free, hard links (or copies) on Windows,
// never a script layer that re-parses arguments. The buckyos-agent
// commands run on the host Tool's Deno (`libexec/buckyos-tool/runtime`)
// from `libexec/buckyos-agent`.
// ---------------------------------------------------------------------------

/// Where a host install keeps the buckyos-agent commands' sources.
pub const AGENT_TOOLS_LIBEXEC: &str = "libexec/buckyos-agent";
/// The host's agent tool directory, below `$BUCKYOS_ROOT`.
pub const AGENT_TOOLS_BIN: &str = "bin/agent-tools";

/// Whether this program answers to `name` as a host tool entry: a
/// buckyos-agent command (run on Deno) or `buckyos` (the host's CLI).
pub fn is_host_entry(name: &str) -> bool {
    name == "buckyos" || is_deno_command(name)
}

/// Whether `name` is a buckyos-agent command (run on Deno).
pub fn is_deno_command(name: &str) -> bool {
    name == "aicc-tool"
        || STANDARD_TOOLS
            .iter()
            .any(|t| t.name == name && t.source == AGENT_TOOLS_SOURCE)
}

/// `$BUCKYOS_ROOT` as this program sees it: the environment, else two
/// levels above the program (`bin/<dir>/<program>`).
pub fn buckyos_root() -> Option<PathBuf> {
    if let Some(root) = std::env::var_os("BUCKYOS_ROOT").filter(|r| !r.is_empty()) {
        return Some(PathBuf::from(root));
    }
    let exe = std::env::args_os().next().map(PathBuf::from)?;
    let exe = if exe.is_absolute() { exe } else { std::env::current_exe().ok()? };
    exe.parent()?.parent()?.parent().map(Path::to_path_buf)
}

pub fn managed_tool_dirs() -> Vec<PathBuf> {
    buckyos_root().map(|root| vec![root.join("tools/bin"), root.join("bin/agent-tools")]).unwrap_or_default()
}

/// The agent's BuckyOS identity for a service tool run by a Session
/// process that has none in its environment (a node session-runtime: the
/// executor passes no `BUCKYOS_*`): asked of AgentService with the
/// session's token. Empty outside a session.
pub fn session_identity_env(name: &str) -> Vec<(String, String)> {
    if std::env::var_os("BUCKYOS_APPCLIENT_SESSION_TOKEN").is_some_and(|t| !t.is_empty()) {
        return Vec::new();
    }
    let Some(client) = crate::session_context::SessionClient::discover(None) else {
        return Vec::new();
    };
    let identity = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| OpenDanError::Other(e.to_string()))
        .and_then(|rt| rt.block_on(client.service_identity()));
    match identity {
        Ok(id) => id.env(),
        Err(e) => {
            eprintln!("{name}: the agent's identity is not available here ({e}); continuing without it");
            Vec::new()
        }
    }
}

/// Run a host tool entry (see [`is_host_entry`]) with `args` and return its
/// exit code; arguments are passed as they are, never through a shell.
pub fn run_host_entry(name: &str, args: &[std::ffi::OsString]) -> i32 {
    let Some(root) = buckyos_root() else {
        eprintln!("{name}: cannot tell $BUCKYOS_ROOT; set it");
        return 2;
    };
    let env = session_identity_env(name);
    if name == "buckyos" {
        let launcher = root.join("bin").join(if cfg!(windows) { "buckyos.cmd" } else { "buckyos" });
        let mut cmd = std::process::Command::new(&launcher);
        cmd.args(args).envs(env).env("BUCKYOS_ROOT", &root);
        return match buckyos_api::hide_child_console(&mut cmd).status() {
            Ok(status) => status.code().unwrap_or(1),
            Err(e) => {
                eprintln!("buckyos: start {}: {e}", launcher.display());
                1
            }
        };
    }
    run_deno_command(&root, name, args, env)
}

/// `<root>/libexec/buckyos-tool/runtime/deno run … aicc-tool.ts [name]
/// args…`.
fn run_deno_command(root: &Path, name: &str, args: &[std::ffi::OsString], env: Vec<(String, String)>) -> i32 {
    let tool = root.join(AGENT_TOOLS_LIBEXEC);
    let deno = root
        .join("libexec/buckyos-tool/runtime")
        .join(if cfg!(windows) { "deno.exe" } else { "deno" });
    for need in [tool.join("aicc-tool.ts"), deno.clone()] {
        if !need.is_file() {
            eprintln!("{name}: {} is missing: this host does not provide the standard AICC commands", need.display());
            return 2;
        }
    }
    let mut cmd = std::process::Command::new(&deno);
    buckyos_api::hide_child_console(&mut cmd);
    cmd.arg("run")
        .arg("--config")
        .arg(tool.join("deno.json"))
        .args(["--allow-net", "--allow-read", "--allow-write", "--allow-env", "--unsafely-ignore-certificate-errors"])
        .arg(tool.join("aicc-tool.ts"));
    if name != "aicc-tool" {
        cmd.arg(name);
    }
    cmd.args(args).envs(env).env("BUCKYOS_ROOT", root);
    match cmd.status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(e) => {
            eprintln!("{name}: start {}: {e}", deno.display());
            1
        }
    }
}

pub(crate) fn link_entry(program: &Path, entry: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(entry) {
        Ok(_) => std::fs::remove_file(entry)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(program, entry)
    }
    #[cfg(not(unix))]
    {
        std::fs::hard_link(program, entry).or_else(|_| std::fs::copy(program, entry).map(|_| ()))
    }
}

/// Lay out `dir` as the agent tool directory of this host: every standard
/// command an entry of `program` (the multi-call `opendan`; its `buckyos`
/// entry runs the host's own launcher with the session's identity).
/// Idempotent; returns the entries.
pub fn install_entries(dir: &Path, program: &Path) -> std::io::Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut made = Vec::new();
    for t in STANDARD_TOOLS {
        let entry = dir.join(format!("{}{}", t.name, std::env::consts::EXE_SUFFIX));
        link_entry(program, &entry)?;
        made.push(entry);
    }
    Ok(made)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_and_service_commands_follow_the_buckyos_agent_sources() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/buckyos-agent/commands");
        let mut sources: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().to_str()?.strip_suffix(".ts").map(str::to_string))
            .collect();
        sources.sort();
        let mut listed: Vec<String> = STANDARD_TOOLS
            .iter()
            .filter(|t| t.source == AGENT_TOOLS_SOURCE)
            .map(|t| t.name.to_string())
            .collect();
        listed.sort();
        assert_eq!(listed, sources, "STANDARD_TOOLS and buckyos-agent/commands differ");
    }

    #[test]
    fn caches_stay_in_the_agents_directory_and_home_is_left_alone() {
        let env = tool_cache_env(Path::new("/a/.cache"));
        assert!(env.iter().all(|(_, value)| value.starts_with("/a/.cache/")), "{env:?}");
        assert!(env.iter().all(|(name, _)| name != "HOME" && name != "USERPROFILE" && name != "PATH"));
        for wanted in ["DENO_DIR", "UV_CACHE_DIR", "UV_PYTHON_INSTALL_DIR", "NPM_CONFIG_CACHE", "NPM_CONFIG_PREFIX", "PIP_CACHE_DIR", "LLM_UNDERSTAND_MEDIA_CATALOG_DIR"] {
            assert!(env.iter().any(|(name, _)| name == wanted), "{wanted}");
        }
    }

    #[test]
    fn no_package_can_provide_a_standard_command() {
        let mut standard: Vec<&str> = STANDARD_TOOLS.iter().map(|t| t.name).collect();
        standard.sort();
        let mut reserved = buckyos_api::EXT_TOOLS_RESERVED_COMMANDS.to_vec();
        reserved.sort();
        assert_eq!(reserved, standard, "EXT_TOOLS_RESERVED_COMMANDS and STANDARD_TOOLS differ");
    }

    #[test]
    fn extended_tools_are_what_the_core_extension_package_provides() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../publish/exttool/ext-core/ext-manifest.json");
        let manifest = buckyos_api::ExtToolsManifest::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(manifest.id, "ext-core");
        let mut provided: Vec<(String, String, Option<String>)> = manifest
            .provides
            .into_iter()
            .map(|c| (c.command, c.version_probe, c.smoke))
            .collect();
        provided.sort();
        let mut listed: Vec<(String, String, Option<String>)> = EXTENDED_TOOLS
            .iter()
            .flat_map(|t| t.commands.iter())
            .map(|c| (c.name.to_string(), c.version_probe.to_string(), c.smoke.map(str::to_string)))
            .collect();
        listed.sort();
        assert_eq!(listed, provided, "EXTENDED_TOOLS and ext-core's ext-manifest.json differ");
        let smoke_dir = path.parent().unwrap();
        for (name, _, smoke) in &listed {
            if let Some(smoke) = smoke {
                assert!(smoke_dir.join(smoke).is_file(), "{name}: {smoke} is not in publish/exttool/ext-core");
            }
        }
        let standard = required_commands(RuntimeBaseline::Standard, false);
        assert!(!extended_commands().iter().any(|c| standard.contains(c)), "extended tools never shadow a standard command");
    }

    #[test]
    fn levels_ask_for_their_own_commands() {
        let standard = required_commands(RuntimeBaseline::Standard, false);
        let extended = required_commands(RuntimeBaseline::Extended, false);
        assert!(!standard.contains(&"ffmpeg") && extended.contains(&"ffmpeg"), "{extended:?}");
        assert!(standard.iter().all(|c| extended.contains(c)), "extended holds standard");
        assert!(extended.contains(&"magick"), "ImageMagick 7's own entry is what a package guarantees");
        assert!(checked_commands(false).contains(&"magick"));
    }

    fn found(entries: &[(&str, ProvidedBy)]) -> BTreeMap<String, ResolvedCommand> {
        entries
            .iter()
            .map(|(n, by)| {
                (
                    n.to_string(),
                    ResolvedCommand { path: format!("/x/{n}"), version: None, provided_by: *by, stub: false },
                )
            })
            .collect()
    }

    fn all_standard() -> Vec<(&'static str, ProvidedBy)> {
        required_commands(RuntimeBaseline::Standard, false).into_iter().map(|n| (n, ProvidedBy::BuckyosRoot)).collect()
    }

    fn all_extended(by: ProvidedBy) -> Vec<(&'static str, ProvidedBy)> {
        EXTENDED_TOOLS.iter().flat_map(|t| t.commands.iter()).map(|c| (c.name, by)).collect()
    }

    #[test]
    fn the_standard_layer_is_strict_about_where_a_command_comes_from() {
        let mut entries = all_standard();
        let complete = evaluate(RuntimeBaseline::Standard, false, &found(&entries));
        assert!(complete.ok(), "{complete:?}");
        assert_eq!(complete.ext.status, ExtStatus::None);

        // a helper beside a development program serves, and shows where it is
        entries[0].1 = ProvidedBy::Dev;
        assert!(evaluate(RuntimeBaseline::Standard, false, &found(&entries)).ok());

        // the user's own `xagent` on PATH is not the standard one
        let xagent = entries.iter().position(|(n, _)| *n == "xagent").unwrap();
        entries[xagent].1 = ProvidedBy::System;
        let shadowed = evaluate(RuntimeBaseline::Standard, false, &found(&entries));
        assert!(!shadowed.ok() && shadowed.missing.is_empty());
        assert_eq!(shadowed.unmanaged, vec!["xagent".to_string()]);
        // nor is an extension package's or an Agent's
        entries[xagent].1 = ProvidedBy::Exttools;
        assert!(!evaluate(RuntimeBaseline::Standard, false, &found(&entries)).ok());
        entries[xagent].1 = ProvidedBy::Agent;
        assert!(!evaluate(RuntimeBaseline::Standard, false, &found(&entries)).ok());
    }

    #[test]
    fn the_extended_layer_is_wide_and_says_who_provides_it() {
        let mut entries = all_standard();
        entries.extend(all_extended(ProvidedBy::Exttools));
        let managed = evaluate(RuntimeBaseline::Extended, false, &found(&entries));
        assert!(managed.ok(), "{managed:?}");
        assert_eq!(managed.ext.status, ExtStatus::Ok);
        assert_eq!(managed.ext.summary(), "ok");

        // the machine happens to have them: still extended, and it shows
        let mut entries = all_standard();
        entries.extend(all_extended(ProvidedBy::System));
        let system = evaluate(RuntimeBaseline::Extended, false, &found(&entries));
        assert!(system.ok());
        assert_eq!(system.ext.summary(), "system");

        // one core tool missing fails `extended` and never `standard`
        entries.retain(|(n, _)| *n != "pandoc");
        let partial = evaluate(RuntimeBaseline::Extended, false, &found(&entries));
        assert!(!partial.ok());
        assert_eq!(partial.missing, vec!["pandoc".to_string()]);
        assert_eq!(partial.ext.summary(), "partial(missing pandoc)");
        assert!(evaluate(RuntimeBaseline::Standard, false, &found(&entries)).ok());

        // a stub found on PATH is not the tool
        let mut stubbed = all_standard();
        stubbed.extend(all_extended(ProvidedBy::Exttools));
        let mut stubbed = found(&stubbed);
        stubbed.get_mut("ffmpeg").unwrap().stub = true;
        let stub = evaluate(RuntimeBaseline::Extended, false, &stubbed);
        assert_eq!(stub.missing, vec!["ffmpeg".to_string()]);
        assert!(stub.ext.summary().contains("missing ffmpeg"));

    }

    #[test]
    fn a_path_tells_who_provides_a_command() {
        let origin = CommandOrigin {
            root: Some("/opt/buckyos".into()),
            agent_root: Some("/opt/buckyos/data/home/o/agents/a".into()),
            beside: Some("/src/target/debug".into()),
            windows: false,
        };
        assert_eq!(origin.of("/opt/buckyos/bin/agent-tools/xagent"), ProvidedBy::BuckyosRoot);
        assert_eq!(origin.of("/opt/buckyos/bin/opendan/xagent"), ProvidedBy::BuckyosRoot);
        assert_eq!(origin.of("/opt/buckyos/tools/bin/ffmpeg"), ProvidedBy::Exttools);
        assert_eq!(origin.of("/opt/buckyos/data/home/o/agents/a/tools/lint/bin/x"), ProvidedBy::Agent);
        assert_eq!(origin.of("/home/o/.opendan/a/tools/x"), ProvidedBy::Agent);
        assert_eq!(origin.of("/src/target/debug/xagent"), ProvidedBy::Dev);
        assert_eq!(origin.of("/usr/bin/ffmpeg"), ProvidedBy::System);
        assert_eq!(origin.of("/opt/buckyos-other/bin/x"), ProvidedBy::System);
        let no_root = CommandOrigin::default();
        assert_eq!(no_root.of("/opt/buckyos/bin/agent-tools/xagent"), ProvidedBy::System);
        let windows = CommandOrigin { root: Some(r"C:\buckyos".into()), windows: true, ..Default::default() };
        assert_eq!(windows.of(r"c:\BuckyOS\tools\bin\ffmpeg.exe"), ProvidedBy::Exttools);
        assert_eq!(windows.of(r"C:\buckyos\bin\agent-tools\xagent.exe"), ProvidedBy::BuckyosRoot);
        assert_eq!(windows.of(r"C:\Windows\ffmpeg.exe"), ProvidedBy::System);
    }

    #[test]
    fn versions_survive_a_check_that_only_looks_at_paths() {
        let known = ResolvedCommand { path: "/x/ffmpeg".into(), version: Some("ffmpeg 7".into()), provided_by: ProvidedBy::System, stub: false };
        let before = BTreeMap::from([("ffmpeg".to_string(), known.clone())]);
        let mut now = BTreeMap::from([("ffmpeg".to_string(), ResolvedCommand { version: None, ..known.clone() })]);
        keep_versions(&mut now, &before);
        assert_eq!(now["ffmpeg"].version.as_deref(), Some("ffmpeg 7"));
        let mut moved = BTreeMap::from([("ffmpeg".to_string(), ResolvedCommand { path: "/y/ffmpeg".into(), version: None, ..known })]);
        keep_versions(&mut moved, &before);
        assert_eq!(moved["ffmpeg"].version, None, "another file is another version");
    }

    #[cfg(unix)]
    #[test]
    fn a_resolve_probe_reports_paths_versions_and_stubs() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = |name: &str, body: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        script("real", "echo 'real 1.2.3'; echo second line");
        script("stub", "echo 'not bundled' >&2; exit 127");
        script("odd", "echo 'odd	tabbed'; exit 3");
        let path = format!("{}:/usr/bin:/bin", dir.path().display());
        let asked = ["real", "stub", "odd", "absent-cmd"];
        let run = |versions: &[&str]| {
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(resolve_probe(&asked, versions, false))
                .env("PATH", &path)
                .output()
                .unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
            parse_resolved(&String::from_utf8_lossy(&out.stdout), &asked)
        };
        let paths = run(&[]);
        assert_eq!(paths.len(), 3);
        assert_eq!(paths["real"].path, dir.path().join("real").display().to_string());
        assert_eq!((paths["real"].exit_code, paths["real"].version.as_deref()), (None, None));
        let full = run(&["real", "stub", "odd", "absent-cmd"]);
        assert_eq!((full["real"].exit_code, full["real"].version.as_deref()), (Some(0), Some("real 1.2.3")));
        assert!(is_stub_exit(full["stub"].exit_code), "{:?}", full["stub"]);
        assert_eq!((full["odd"].exit_code, full["odd"].version.as_deref()), (Some(3), Some("odd tabbed")));
        assert!(!is_stub_exit(full["odd"].exit_code), "a command that runs but dislikes the flag is there");
        assert!(!full.contains_key("absent-cmd"));
    }

    #[test]
    fn a_windows_resolve_probe_is_one_powershell_line() {
        let ps = resolve_probe(&["it's", "ffmpeg"], &["ffmpeg"], true);
        assert!(ps.contains("Get-Command") && ps.contains("$g.Source"));
        assert!(ps.contains("p 'it''s' ''") && ps.contains("p 'ffmpeg' 'ffmpeg -version'"), "{ps}");
        assert_eq!(version_probe_of("custom-tool"), "custom-tool --version");
        assert_eq!(version_probe_of("pdftotext"), "pdftotext -v");
    }

    #[test]
    fn helpers_are_named_by_the_libopendan_overlay() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent_tool"), "").unwrap();
        let helpers = crate::runtime::bin_overlay::standard_helpers(Some(dir.path()));
        for h in &helpers {
            assert!(STANDARD_TOOLS.iter().any(|t| t.name == h.name && t.session_helper_of == Some("agent_tool")));
        }
        let overlay = required_commands(RuntimeBaseline::Standard, true);
        assert!(overlay.contains(&"xagent") && !overlay.contains(&"agent-session"));
        assert!(required_commands(RuntimeBaseline::Standard, false).contains(&"agent-session"));
    }

    #[cfg(unix)]
    #[test]
    fn a_host_tool_directory_is_links_to_one_program() {
        let root = tempfile::tempdir().unwrap();
        let program = root.path().join("opendan");
        std::fs::write(&program, "#!/bin/sh\necho \"$(basename \"$0\")\" \"$@\"\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
        let dir = root.path().join("bin").join("agent-tools");
        let made = install_entries(&dir, &program).unwrap();
        assert_eq!(made.len(), STANDARD_TOOLS.len());
        assert_eq!(install_entries(&dir, &program).unwrap(), made, "idempotent");
        let names = required_commands(RuntimeBaseline::Standard, false);
        assert_eq!(resolve_locally(&names, &[], Some(dir.as_os_str()), &[]).len(), names.len());
        let out = std::process::Command::new(dir.join("agent-session")).args(["a b", "中文"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "agent-session a b 中文\n");
        assert!(is_host_entry("gen_image") && is_host_entry("aicc-tool") && is_host_entry("buckyos"));
        assert!(!is_host_entry("xagent") && !is_host_entry("agent-session"));
    }

    #[test]
    fn probes_report_what_is_missing() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present");
        std::fs::write(&present, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&present, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let found = resolve_locally(&["present", "absent-cmd"], &[], Some(dir.path().as_os_str()), &[]);
        assert_eq!(found.keys().cloned().collect::<Vec<_>>(), vec!["present".to_string()]);
        #[cfg(unix)]
        {
            let script = missing_probe(&["present", "absent-cmd"], false);
            let out = std::process::Command::new("/bin/sh")
                .arg("-c")
                .arg(&script)
                .env("PATH", dir.path())
                .output()
                .unwrap();
            assert!(out.status.success());
            assert_eq!(
                parse_missing(&String::from_utf8_lossy(&out.stdout), &["present", "absent-cmd"]),
                vec!["absent-cmd".to_string()]
            );
        }
        let ps = missing_probe(&["it's"], true);
        assert!(ps.contains("@('it''s')") && ps.contains("Get-Command"));
    }
}
