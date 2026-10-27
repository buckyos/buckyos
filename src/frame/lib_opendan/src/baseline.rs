//! The standard environment baseline (primary-runtime TODO §2, T1): the
//! commands every session-runtime declared `standard` provides. One list
//! serves the image build (`xagent runtime baseline --check`), host
//! installation checks, the runtime check that records `baseline_check`
//! and the manuals (`agent-session runtime baseline`).
//!
//! An `extended` runtime has all of these plus whatever its owner prepared
//! (build tool chains, ext-tools); those are not listed: a cmd_skill that
//! needs them declares `baseline: extended` and names them in its text.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::OpenDanError;

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

/// The names a runtime must have on PATH. `overlay`: the runtime shares the
/// agent process's machine, so session helpers come from the session bin
/// overlay and their providing executable is what must exist.
pub fn required_commands(overlay: bool) -> Vec<&'static str> {
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
    out
}

fn powershell_quote(s: &str) -> String {
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

fn executable_in(dir: &Path, name: &str) -> bool {
    let candidates: Vec<PathBuf> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat", ".ps1"]
            .iter()
            .map(|ext| dir.join(format!("{name}{ext}")))
            .collect()
    } else {
        vec![dir.join(name)]
    };
    candidates.iter().any(|p| {
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

/// The commands of `names` this process cannot find on `path` (`None`: its
/// own `PATH`) nor in `extra` directories.
pub fn missing_locally(names: &[&str], path: Option<&std::ffi::OsStr>, extra: &[PathBuf]) -> Vec<String> {
    let path = path
        .map(|p| p.to_os_string())
        .or_else(|| std::env::var_os("PATH"))
        .unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::env::split_paths(&path).collect();
    dirs.extend(extra.iter().cloned());
    names
        .iter()
        .filter(|n| !dirs.iter().any(|d| executable_in(d, n)))
        .map(|n| n.to_string())
        .collect()
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

fn link_entry(program: &Path, entry: &Path) -> std::io::Result<()> {
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
    fn helpers_are_named_by_the_libopendan_overlay() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent_tool"), "").unwrap();
        let helpers = crate::runtime::bin_overlay::standard_helpers(Some(dir.path()));
        for h in &helpers {
            assert!(STANDARD_TOOLS.iter().any(|t| t.name == h.name && t.session_helper_of == Some("agent_tool")));
        }
        let overlay = required_commands(true);
        assert!(overlay.contains(&"xagent") && !overlay.contains(&"agent-session"));
        assert!(required_commands(false).contains(&"agent-session"));
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
        assert!(missing_locally(&required_commands(false), Some(dir.as_os_str()), &[]).is_empty());
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
        let missing = missing_locally(&["present", "absent-cmd"], Some(dir.path().as_os_str()), &[]);
        assert_eq!(missing, vec!["absent-cmd".to_string()]);
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
