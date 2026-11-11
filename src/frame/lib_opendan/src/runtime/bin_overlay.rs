//! `<sid>/.runtime/bin` — the Session Bin layer, first on PATH (§7.1).
//!
//! Content: tool-plan tombstones (stubs that shadow blocked tools) and
//! session helpers (e.g. the `agent-session` CLI used to declare activity /
//! perceptions from inside `exec`). Agent tools stay a separate PATH layer
//! (`<agent_root>/tools`), tombstones shadow them.
//!
//! A helper is a script that runs its program on Unix; on Windows it is the
//! program itself under the helper's name (`<name>.exe`, a hard link or a
//! copy), since a script is run only by its own shell and every other
//! caller (`cmd`, `Start-Process`, CreateProcess) either misses it or hands
//! it to its file association. Tombstones are `.cmd` scripts there.
//!
//! Preparation is idempotent and repairable: the expected set is computed,
//! every entry is (re)written through an atomic replace, stale entries are
//! removed, and a manifest (`.manifest.json`, written last) records the
//! expected content hashes (a program's length and modification time).
//! `verify` checks the manifest against the files; a half-prepared
//! directory (crash in the middle) fails verification and the next
//! `prepare` repairs it.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{OpenDanError, Result};
use crate::fsutil;

pub const MANIFEST: &str = ".manifest.json";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanMode {
    #[default]
    Deny,
    Allow,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolPlanEntry {
    pub name: String,
    #[serde(default)]
    pub reason: String,
}

/// `<agent_root>/tool_plans/<name>.toml` (same schema as opendan).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolPlan {
    pub mode: PlanMode,
    pub deny: Vec<ToolPlanEntry>,
    pub allow: Vec<ToolPlanEntry>,
}

impl ToolPlan {
    pub fn load(agent_root: &Path, name: &str) -> Result<Option<ToolPlan>> {
        let path = agent_root.join("tool_plans").join(format!("{name}.toml"));
        match std::fs::read_to_string(&path) {
            Ok(s) => toml::from_str(&s)
                .map(Some)
                .map_err(|e| OpenDanError::json(&path, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(OpenDanError::io(&path, e)),
        }
    }
}

/// What the Session Bin must contain.
#[derive(Debug, Clone, Default)]
pub struct BinPlan {
    pub tool_plan_name: Option<String>,
    pub tool_plan: Option<ToolPlan>,
    /// Executable providing the `agent-session` helper CLI (wrapped).
    pub session_cli: Option<PathBuf>,
    /// Further helper commands of the host, wrapped the same way.
    pub helpers: Vec<SessionHelper>,
    /// Extra fixed scripts `name → content`.
    pub scripts: BTreeMap<String, String>,
    /// Directories whose executables form the tool universe for tombstones.
    pub universe_dirs: Vec<PathBuf>,
}

/// An entry of the Session Bin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Entry {
    /// A script with this text.
    Script(String),
    /// This program under the entry's name (Windows).
    Program(PathBuf),
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BinManifest {
    /// name → sha256 of a script's content.
    pub entries: BTreeMap<String, String>,
    /// name → [`agent_tool::xllm::session_program_stamp`] of a program.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub programs: BTreeMap<String, String>,
    #[serde(default)]
    pub tool_plan: Option<String>,
    #[serde(default)]
    pub prepared_at_ms: u64,
}

fn sha(content: &[u8]) -> String {
    hex::encode(Sha256::digest(content))
}

fn stamp(path: &Path) -> Result<String> {
    agent_tool::xllm::session_program_stamp(path).map_err(|e| OpenDanError::io(path, e))
}

fn is_executable(meta: &std::fs::Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        true
    }
}

/// The command a file on PATH answers to: on Windows its name without an
/// executable extension (an extensionless file is found as it is).
fn command_of(file_name: &str) -> &str {
    if cfg!(windows) {
        if let Some((stem, ext)) = file_name.rsplit_once('.') {
            if !stem.is_empty() && ["exe", "com", "bat", "cmd", "ps1"].iter().any(|x| ext.eq_ignore_ascii_case(x)) {
                return stem;
            }
        }
    }
    file_name
}

fn scan_universe(dirs: &[PathBuf]) -> HashSet<String> {
    let mut out = HashSet::new();
    for d in dirs {
        let Ok(rd) = std::fs::read_dir(d) else {
            continue;
        };
        for e in rd.flatten() {
            if let Ok(meta) = e.metadata() {
                if meta.is_file() && is_executable(&meta) {
                    if let Some(n) = e.file_name().to_str() {
                        out.insert(command_of(n).to_string());
                    }
                }
            }
        }
    }
    out
}

#[cfg(not(windows))]
pub(crate) fn tombstone_script(tool: &str, reason: &str, plan: &str) -> String {
    let esc = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"").replace('\'', "");
    format!(
        "#!/bin/sh\n# generated by libopendan (tool plan tombstone)\n\
         echo '{{\"blocked_by\":\"tool_plan\",\"tool\":\"{t}\",\"reason\":\"{r}\",\"plan\":\"{p}\"}}' >&2\n\
         echo '{t} is blocked by tool plan: {r}' >&2\n\
         exit 127\n",
        t = esc(tool),
        r = esc(reason),
        p = esc(plan)
    )
}

/// A `.cmd` tombstone: every caret-escaped character stays literal, so the
/// text never opens a quote, a redirection or a second command.
#[cfg(windows)]
pub(crate) fn tombstone_script(tool: &str, reason: &str, plan: &str) -> String {
    let json = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let line = |s: &str| {
        let mut out = String::new();
        for c in s.chars() {
            match c {
                '%' => out.push_str("%%"),
                '^' | '&' | '|' | '<' | '>' | '"' | '(' | ')' => {
                    out.push('^');
                    out.push(c);
                }
                '\r' | '\n' => out.push(' '),
                _ => out.push(c),
            }
        }
        out
    };
    let blocked = format!(
        "{{\"blocked_by\":\"tool_plan\",\"tool\":\"{}\",\"reason\":\"{}\",\"plan\":\"{}\"}}",
        json(tool),
        json(reason),
        json(plan)
    );
    format!(
        "@echo off\r\nrem generated by libopendan (tool plan tombstone)\r\n\
         echo {} 1>&2\r\n\
         echo {} 1>&2\r\n\
         exit /b 127\r\n",
        line(&blocked),
        line(&format!("{tool} is blocked by tool plan: {reason}"))
    )
}

/// File name of the tombstone shadowing `command`.
pub fn tombstone_name(command: &str) -> String {
    if cfg!(windows) {
        format!("{command}.cmd")
    } else {
        command.to_string()
    }
}

/// A command the host puts on every session's PATH: `name` runs
/// `exe args… "$@"` (e.g. `llm_understand_media` → `agent_tool
/// llm_understand_media`). On Windows the entry is `exe` itself named
/// `name`, so `exe` answers to that name as it does to `exe args…`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHelper {
    pub name: String,
    pub exe: PathBuf,
    pub args: Vec<String>,
}

impl SessionHelper {
    pub fn new(name: &str, exe: PathBuf, args: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            exe,
            args: args.iter().map(|a| a.to_string()).collect(),
        }
    }

    fn entry(&self) -> Entry {
        helper_entry("session helper command", &self.exe, &self.args)
    }
}

/// File name of the Session Bin helper answering to `command`.
pub fn entry_name(command: &str) -> String {
    format!("{command}{}", std::env::consts::EXE_SUFFIX)
}

#[cfg(not(windows))]
fn helper_entry(kind: &str, exe: &Path, args: &[String]) -> Entry {
    let quote = |s: &str| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    let mut line = format!("exec {}", quote(&exe.display().to_string()));
    for a in args {
        line.push(' ');
        line.push_str(&quote(a));
    }
    Entry::Script(format!("#!/bin/sh\n# generated by libopendan: {kind}\n{line} \"$@\"\n"))
}

#[cfg(windows)]
fn helper_entry(_kind: &str, exe: &Path, _args: &[String]) -> Entry {
    Entry::Program(exe.to_path_buf())
}

/// The helper commands every session gets from the binaries found in
/// `dir` (the host's own directory): `llm_understand_media` when the
/// `agent_tool` CLI is there.
pub fn standard_helpers(dir: Option<&Path>) -> Vec<SessionHelper> {
    let Some(dir) = dir else {
        return Vec::new();
    };
    let agent_tool = dir.join(if cfg!(windows) { "agent_tool.exe" } else { "agent_tool" });
    if !agent_tool.is_file() {
        return Vec::new();
    }
    vec![SessionHelper::new(
        "llm_understand_media",
        agent_tool,
        &["llm_understand_media"],
    )]
}

/// Compute the expected entries of the Session Bin.
pub fn expected_entries(plan: &BinPlan, path_env: Option<&str>) -> BTreeMap<String, Entry> {
    let mut out = BTreeMap::new();
    if let Some(cli) = &plan.session_cli {
        out.insert(entry_name("agent-session"), helper_entry("session helper CLI", cli, &[]));
    }
    for h in &plan.helpers {
        out.insert(entry_name(&h.name), h.entry());
    }
    for (n, c) in &plan.scripts {
        out.insert(n.clone(), Entry::Script(c.clone()));
    }
    let shadow = |out: &mut BTreeMap<String, Entry>, n: &str, reason: &str, plan: &str| {
        out.remove(&entry_name(n));
        out.insert(tombstone_name(n), Entry::Script(tombstone_script(n, reason, plan)));
    };
    if let (Some(name), Some(tp)) = (&plan.tool_plan_name, &plan.tool_plan) {
        let universe = scan_universe(&plan.universe_dirs);
        match tp.mode {
            PlanMode::Deny => {
                // Deny lists shadow the name wherever it would be found on
                // PATH (agent / runtime layers and the system PATH).
                let mut on_path = universe.clone();
                if let Some(p) = path_env {
                    let dirs: Vec<PathBuf> = std::env::split_paths(p).collect();
                    on_path.extend(scan_universe(&dirs));
                }
                for e in &tp.deny {
                    let n = e.name.trim();
                    if n.is_empty() || !on_path.contains(n) {
                        continue;
                    }
                    let reason = if e.reason.trim().is_empty() {
                        format!("blocked by tool plan `{name}`")
                    } else {
                        e.reason.clone()
                    };
                    shadow(&mut out, n, &reason, name);
                }
            }
            PlanMode::Allow => {
                let allowed: HashSet<&str> =
                    tp.allow.iter().map(|e| e.name.as_str()).collect();
                for n in universe {
                    if allowed.contains(n.as_str()) {
                        continue;
                    }
                    let reason = format!("not in allow list of tool plan `{name}`");
                    shadow(&mut out, &n, &reason, name);
                }
            }
        }
    }
    out
}

#[cfg(unix)]
fn set_exec(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut p = std::fs::metadata(path)
        .map_err(|e| OpenDanError::io(path, e))?
        .permissions();
    p.set_mode(0o755);
    std::fs::set_permissions(path, p).map_err(|e| OpenDanError::io(path, e))
}

#[cfg(not(unix))]
fn set_exec(_path: &Path) -> Result<()> {
    Ok(())
}

/// Idempotently (re)build the Session Bin; the manifest is written last.
pub fn prepare(bin_dir: &Path, plan: &BinPlan) -> Result<BinManifest> {
    std::fs::create_dir_all(bin_dir).map_err(|e| OpenDanError::io(bin_dir, e))?;
    let path_env = std::env::var("PATH").ok();
    let expected = expected_entries(plan, path_env.as_deref());
    // Invalidate the manifest first: a crash mid-way must fail verification.
    let manifest_path = bin_dir.join(MANIFEST);
    let _ = std::fs::remove_file(&manifest_path);
    let mut manifest = BinManifest {
        tool_plan: plan.tool_plan_name.clone(),
        ..Default::default()
    };
    for (name, entry) in &expected {
        let path = bin_dir.join(name);
        let tmp = bin_dir.join(format!(".{name}.tmp-{}", uuid::Uuid::new_v4().simple()));
        match entry {
            Entry::Script(content) => {
                std::fs::write(&tmp, content.as_bytes()).map_err(|e| OpenDanError::io(&tmp, e))?;
                set_exec(&tmp)?;
                std::fs::rename(&tmp, &path).map_err(|e| OpenDanError::io(&path, e))?;
                manifest.entries.insert(name.clone(), sha(content.as_bytes()));
            }
            Entry::Program(program) => {
                let want = stamp(program)?;
                if stamp(&path).ok().as_ref() != Some(&want) {
                    crate::baseline::link_entry(program, &tmp).map_err(|e| OpenDanError::io(&tmp, e))?;
                    // A running program cannot be replaced, only moved away.
                    if std::fs::symlink_metadata(&path).is_ok() {
                        let old = bin_dir.join(format!(".{name}.old-{}", uuid::Uuid::new_v4().simple()));
                        std::fs::rename(&path, &old).map_err(|e| OpenDanError::io(&path, e))?;
                    }
                    std::fs::rename(&tmp, &path).map_err(|e| OpenDanError::io(&path, e))?;
                }
                manifest.programs.insert(name.clone(), want);
            }
        }
    }
    // Remove entries we generated earlier that are no longer expected, and
    // leftovers of interrupted writes.
    if let Ok(rd) = std::fs::read_dir(bin_dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n == MANIFEST {
                continue;
            }
            if n.starts_with('.') || !expected.contains_key(&n) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
    manifest.prepared_at_ms = crate::now_ms();
    fsutil::atomic_replace_json(&manifest_path, &manifest)?;
    Ok(manifest)
}

/// Verify the Session Bin against its manifest and the expected plan.
pub fn verify(bin_dir: &Path, plan: &BinPlan) -> Result<()> {
    let manifest: BinManifest = fsutil::read_json_opt(&bin_dir.join(MANIFEST))?
        .ok_or_else(|| OpenDanError::Bind("session bin is not prepared (no manifest)".into()))?;
    let path_env = std::env::var("PATH").ok();
    let expected = expected_entries(plan, path_env.as_deref());
    for (name, entry) in &expected {
        let recorded = match entry {
            Entry::Script(content) => manifest.entries.get(name) == Some(&sha(content.as_bytes())),
            Entry::Program(program) => manifest.programs.get(name) == Some(&stamp(program)?),
        };
        if !recorded {
            return Err(OpenDanError::Bind(format!(
                "session bin entry `{name}` is missing from the manifest or outdated"
            )));
        }
    }
    for (name, recorded) in &manifest.programs {
        let found = agent_tool::xllm::session_program_stamp(&bin_dir.join(name))
            .map_err(|_| OpenDanError::Bind(format!("session bin entry `{name}` is missing")))?;
        if &found != recorded {
            return Err(OpenDanError::Bind(format!(
                "session bin entry `{name}` does not match the manifest"
            )));
        }
    }
    for (name, digest) in &manifest.entries {
        let path = bin_dir.join(name);
        let data = std::fs::read(&path)
            .map_err(|_| OpenDanError::Bind(format!("session bin entry `{name}` is missing")))?;
        if &sha(&data) != digest {
            return Err(OpenDanError::Bind(format!(
                "session bin entry `{name}` content does not match the manifest"
            )));
        }
        let meta = std::fs::metadata(&path).map_err(|e| OpenDanError::io(&path, e))?;
        if !is_executable(&meta) {
            return Err(OpenDanError::Bind(format!(
                "session bin entry `{name}` is not executable"
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers_are_wrapped_next_to_agent_session() {
        let dir = tempfile::tempdir().unwrap();
        assert!(standard_helpers(Some(dir.path())).is_empty(), "no agent_tool, no helper");
        let tool = dir.path().join(format!("agent_tool{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&tool, "").unwrap();
        let helpers = standard_helpers(Some(dir.path()));
        assert_eq!(
            helpers,
            vec![SessionHelper::new("llm_understand_media", tool.clone(), &["llm_understand_media"])]
        );
        let plan = BinPlan {
            session_cli: Some(dir.path().join("xagent")),
            helpers,
            ..Default::default()
        };
        let entries = expected_entries(&plan, None);
        #[cfg(not(windows))]
        assert_eq!(
            entries["llm_understand_media"],
            Entry::Script(format!(
                "#!/bin/sh\n# generated by libopendan: session helper command\nexec \"{}\" \"llm_understand_media\" \"$@\"\n",
                tool.display()
            ))
        );
        #[cfg(windows)]
        {
            assert_eq!(entries["agent-session.exe"], Entry::Program(dir.path().join("xagent")));
            assert_eq!(entries["llm_understand_media.exe"], Entry::Program(tool));
        }
    }

    #[cfg(windows)]
    fn powershell(bin: &Path, body: &str) -> std::process::Output {
        let script = format!(
            "$ErrorActionPreference = 'Stop'\n$env:PATH = '{};' + $env:PATH\n\
             $OutputEncoding = [Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)\n{body}",
            bin.display()
        );
        std::process::Command::new("powershell.exe")
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", &script])
            .output()
            .unwrap()
    }

    /// `sort.exe` stands for `xagent`, `cmd.exe` for a helper; both copied
    /// into a host directory so that the session bin links them.
    #[cfg(windows)]
    fn windows_host(dir: &Path) -> BinPlan {
        let host = dir.join("host");
        std::fs::create_dir_all(&host).unwrap();
        std::fs::copy(r"C:\Windows\System32\sort.exe", host.join("xagent.exe")).unwrap();
        std::fs::copy(r"C:\Windows\System32\cmd.exe", host.join("echo_args.exe")).unwrap();
        BinPlan {
            session_cli: Some(host.join("xagent.exe")),
            helpers: vec![SessionHelper::new("echo_args", host.join("echo_args.exe"), &[])],
            ..Default::default()
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_helpers_are_programs_every_caller_runs() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let plan = windows_host(dir.path());
        let manifest = prepare(&bin, &plan).unwrap();
        verify(&bin, &plan).unwrap();
        assert!(manifest.entries.is_empty(), "{manifest:?}");
        let mut names: Vec<String> = std::fs::read_dir(&bin)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, [MANIFEST, "agent-session.exe", "echo_args.exe"]);

        let out = powershell(
            &bin,
            "$c = Get-Command agent-session\n\
             if ($c.CommandType -ne 'Application' -or -not $c.Source.EndsWith('agent-session.exe')) { throw \"resolved to $($c.Source)\" }\n\
             'needle' | agent-session\n\
             echo_args /d /c echo \"it's\" 中文\n\
             $p = Start-Process agent-session -ArgumentList '/?' -WindowStyle Hidden -Wait -PassThru\n\
             'start-process exit ' + $p.ExitCode\n",
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(out.status.success(), "{stdout}\n{}", String::from_utf8_lossy(&out.stderr));
        assert!(stdout.contains("needle") && stdout.contains("it's"), "{stdout}");
        assert!(stdout.contains("start-process exit 0"), "{stdout}");

        let path = format!("{};{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        let out = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "echo needle| agent-session"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("needle"), "{out:?}");
        let out = std::process::Command::new("echo_args")
            .args(["/d", "/c", "echo direct"])
            .env("PATH", &path)
            .output()
            .unwrap();
        assert!(out.status.success() && String::from_utf8_lossy(&out.stdout).contains("direct"), "{out:?}");
    }

    #[cfg(windows)]
    #[test]
    fn windows_programs_follow_the_host_even_while_one_runs() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let plan = windows_host(dir.path());
        std::fs::create_dir_all(&bin).unwrap();
        // An older program still running under the entry's name.
        std::fs::copy(r"C:\Windows\System32\findstr.exe", bin.join("agent-session.exe")).unwrap();
        std::fs::write(bin.join("agent-session.ps1"), "'stale'").unwrap();
        let mut running = std::process::Command::new(bin.join("agent-session.exe"))
            .arg("x")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        prepare(&bin, &plan).unwrap();
        verify(&bin, &plan).unwrap();
        assert!(!bin.join("agent-session.ps1").exists());
        assert_eq!(
            std::fs::read(bin.join("agent-session.exe")).unwrap(),
            std::fs::read(plan.session_cli.as_ref().unwrap()).unwrap()
        );
        running.stdin.take().unwrap().write_all(b"x\n").unwrap();
        running.wait().unwrap();
        prepare(&bin, &plan).unwrap();
        assert!(std::fs::read_dir(&bin).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains(".old-")));

        // A new host program: the entry is outdated until the next prepare.
        let xagent = plan.session_cli.clone().unwrap();
        std::fs::remove_file(&xagent).unwrap();
        std::fs::copy(r"C:\Windows\System32\findstr.exe", &xagent).unwrap();
        assert!(verify(&bin, &plan).is_err());
        prepare(&bin, &plan).unwrap();
        verify(&bin, &plan).unwrap();
        assert_eq!(
            std::fs::read(bin.join("agent-session.exe")).unwrap(),
            std::fs::read(&xagent).unwrap()
        );

        // Replaced in place: no longer what the manifest says.
        std::fs::remove_file(bin.join("echo_args.exe")).unwrap();
        std::fs::copy(r"C:\Windows\System32\sort.exe", bin.join("echo_args.exe")).unwrap();
        assert!(verify(&bin, &plan).is_err());
    }

    #[cfg(windows)]
    #[test]
    fn windows_tombstones_are_cmd_scripts_that_shadow_any_extension() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        std::fs::write(tools.join("rm-all.cmd"), "@echo boom\r\n").unwrap();
        std::fs::write(tools.join("wipe"), "#!/bin/sh\n").unwrap();
        let bin = dir.path().join("bin");
        let reason = r#"a & b | "c" 100% <x> ^y (z)"#;
        let plan = BinPlan {
            tool_plan_name: Some("safe".into()),
            tool_plan: Some(ToolPlan {
                mode: PlanMode::Deny,
                deny: vec![
                    ToolPlanEntry { name: "rm-all".into(), reason: reason.into() },
                    ToolPlanEntry { name: "wipe".into(), reason: String::new() },
                ],
                allow: Vec::new(),
            }),
            universe_dirs: vec![tools.clone()],
            ..Default::default()
        };
        prepare(&bin, &plan).unwrap();
        verify(&bin, &plan).unwrap();
        assert!(bin.join(tombstone_name("rm-all")).is_file() && bin.join("wipe.cmd").is_file());
        assert!(!bin.join("rm-all").exists() && !bin.join("wipe").exists());

        let path = format!("{};{};{}", bin.display(), std::env::var("PATH").unwrap_or_default(), tools.display());
        let out = powershell(
            &bin,
            &format!(
                "$env:PATH = '{path}'\n$ErrorActionPreference = 'Continue'\n\
                 rm-all 2>$null\n'rm-all ' + $LASTEXITCODE\nwipe 2>$null\n'wipe ' + $LASTEXITCODE\n"
            ),
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(!stdout.contains("boom") && stdout.contains("rm-all 127") && stdout.contains("wipe 127"), "{stdout}");

        let run = |command: &str| {
            use std::os::windows::process::CommandExt;
            let out = std::process::Command::new("cmd.exe")
                .raw_arg(format!("/d /c {command}"))
                .env("PATH", &path)
                .output()
                .unwrap();
            assert_eq!(out.status.code(), Some(127), "{out:?}");
            assert!(out.stdout.is_empty(), "{out:?}");
            String::from_utf8_lossy(&out.stderr).into_owned()
        };
        let stderr = run("rm-all");
        assert!(stderr.contains(&format!("rm-all is blocked by tool plan: {reason}")), "{stderr}");
        assert!(stderr.contains(r#""reason":"a & b | \"c\" 100% <x> ^y (z)""#), "{stderr}");
        assert!(run("wipe").contains("wipe is blocked by tool plan: blocked by tool plan `safe`"));
    }
}
