use super::files::{check_allowed, FileBackend};
use super::{shell_quote, ExecDirFacts, RuntimeInfo, SshConfig};
use crate::llm_bash::{
    sanitize_call_id, BashRunOutput, BashRunRequest, BashRunner, BashTarget, CommandHandle,
    CommandProgress, RunBindingSlot, OUTPUT_TAIL_BYTES, TIMEOUT_EXIT_CODE,
};
use crate::xllm::XllmError;
use crate::{AgentToolError, SessionRuntimeContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

fn new_transfer_id() -> String {
    format!("{}-{}", std::process::id(), crate::tasks::next_local_seq())
}

type ToolResult<T> = Result<T, AgentToolError>;
fn transport(message: impl Into<String>, effect_unknown: bool) -> AgentToolError {
    AgentToolError::Transport {
        message: message.into(),
        effect_unknown,
    }
}
fn capability(e: impl std::fmt::Display) -> XllmError {
    XllmError::Capability(e.to_string())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub machine: String,
    pub hostname: String,
    pub uid: String,
}
#[derive(Debug)]
pub struct Probe {
    pub identity: Identity,
    pub info: RuntimeInfo,
    pub path: String,
}

const PROBE: &str = r#"set -e
[ "$(uname -s)" = Linux ]
command -v bash >/dev/null
command -v realpath >/dev/null
command -v base64 >/dev/null
printf '%s\0' "$(cat /etc/machine-id)" "$(hostname)" "$(id -u)" "$(uname -s | tr '[:upper:]' '[:lower:]')" "$(uname -m)" "$(command -v bash)" "$(pwd -P)" "${PATH}" "$(date -Iseconds)" "$(date +%Z%z)"
for tool in bash realpath base64; do printf '%s\0' "$tool"; done
"#;

fn probe_script(cwd: &str, env: &BTreeMap<String, String>) -> String {
    let mut s = format!("cd {} || exit 126\n", shell_quote(cwd));
    for (k, v) in env {
        s.push_str(&format!("export {k}={}\n", shell_quote(v)));
    }
    s.push_str(PROBE);
    s
}
pub(super) fn parse_probe(bytes: &[u8]) -> Result<Probe, XllmError> {
    let p = bytes
        .strip_suffix(&[0])
        .unwrap_or(bytes)
        .split(|b| *b == 0)
        .map(|p| String::from_utf8_lossy(p).to_string())
        .collect::<Vec<_>>();
    if p.len() < 10 || p[..10].iter().any(String::is_empty) {
        return Err(capability(
            "execution environment probe returned incomplete data",
        ));
    }
    Ok(Probe {
        identity: Identity {
            machine: p[0].clone(),
            hostname: p[1].clone(),
            uid: p[2].clone(),
        },
        info: RuntimeInfo {
            os: p[3].clone(),
            arch: p[4].clone(),
            hostname: p[1].clone(),
            shell: p[5].clone(),
            cwd: p[6].clone(),
            current_time: p[8].clone(),
            timezone: p[9].clone(),
            tools: p[10..].iter().take(32).cloned().collect(),
            ..Default::default()
        },
        path: p[7].clone(),
    })
}

pub(super) const LOCAL_PROBE: &str = r#"set -e
command -v bash >/dev/null
machine_id="$(cat /etc/machine-id 2>/dev/null || true)"
[ -n "$machine_id" ] || machine_id="$(hostname)"
printf '%s\0' "$machine_id" "$(hostname)" "$(id -u)" "$(uname -s | tr '[:upper:]' '[:lower:]')" "$(uname -m)" "$(command -v bash)" "$(pwd -P)" "${PATH}" "$(date +%Y-%m-%dT%H:%M:%S%z)" "$(date +%Z%z)"
for tool in bash realpath base64; do if command -v "$tool" >/dev/null; then printf '%s\0' "$tool"; fi; done
"#;

#[cfg(windows)]
const POWERSHELL_PROBE: &str = r#"$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$hostname = [Environment]::MachineName
$machine = (Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Cryptography' -Name MachineGuid -ErrorAction SilentlyContinue).MachineGuid
if (-not $machine) { $machine = $hostname }
$identity = [System.Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$arch = $env:PROCESSOR_ARCHITECTURE.ToLowerInvariant()
if ($arch -eq 'amd64') { $arch = 'x86_64' }
$shell = (Get-Process -Id $PID).Path
$fields = @($machine, $hostname, $identity, 'windows', $arch, $shell, (Get-Location).ProviderPath, $env:PATH, [DateTimeOffset]::Now.ToString('o'), [TimeZoneInfo]::Local.Id, [System.IO.Path]::GetFileNameWithoutExtension($shell), 'Get-Content', 'Set-Content', 'Get-ChildItem', 'Select-String', 'Start-Process')
[Console]::Write(($fields -join [char]0) + [char]0)
"#;

pub async fn local_probe(cwd: &Path, env: &BTreeMap<String, String>) -> Result<Probe, XllmError> {
    #[cfg(windows)]
    let script = POWERSHELL_PROBE;
    #[cfg(not(windows))]
    let script = LOCAL_PROBE;
    let mut command = crate::llm_bash::native_shell_command(script);
    command.current_dir(cwd).envs(env).kill_on_drop(true);
    let o = tokio::time::timeout(Duration::from_secs(10), command.output())
        .await
        .map_err(capability)?
        .map_err(capability)?;
    if !o.status.success() {
        return Err(capability(format!(
            "native capability probe failed: {}",
            String::from_utf8_lossy(&o.stderr)
        )));
    }
    let probe = parse_probe(&o.stdout)?;
    #[cfg(windows)]
    let probe = {
        let mut probe = probe;
        probe.info.cwd = cwd
            .canonicalize()
            .map_err(capability)?
            .display()
            .to_string();
        probe
    };
    Ok(probe)
}

#[cfg(all(test, unix))]
mod probe_tests {
    use super::*;

    #[tokio::test]
    async fn local_probe_uses_hostname_when_machine_id_is_empty() {
        let output = Command::new("bash")
            .arg("-c")
            .arg(format!("cat() {{ return 0; }}\n{LOCAL_PROBE}"))
            .output()
            .await
            .unwrap();
        assert!(output.status.success());
        let probe = parse_probe(&output.stdout).unwrap();
        assert_eq!(probe.identity.machine, probe.identity.hostname);
        assert_eq!(
            probe.info.cwd,
            std::env::current_dir().unwrap().to_str().unwrap()
        );
        assert!(probe.info.shell.ends_with("/bash"));
    }

    #[test]
    fn probe_rejects_empty_fields_without_shifting_positions() {
        let fields = [
            "machine",
            "host",
            "1000",
            "linux",
            "x86_64",
            "/bin/bash",
            "/tmp/work",
            "/usr/bin:/bin",
            "2026-10-03T00:00:00+0000",
            "UTC+0000",
            "bash",
            "realpath",
            "base64",
        ];
        for index in 0..10 {
            let mut incomplete = fields;
            incomplete[index] = "";
            assert!(parse_probe(format!("{}\0", incomplete.join("\0")).as_bytes()).is_err());
        }
        let probe = parse_probe(format!("{}\0", fields.join("\0")).as_bytes()).unwrap();
        assert_eq!(probe.info.cwd, "/tmp/work");
        assert_eq!(probe.path, "/usr/bin:/bin");
    }
}

#[derive(Debug, Clone)]
pub struct SshTransport {
    config: SshConfig,
    resolved: BTreeMap<String, String>,
    identity: Option<Identity>,
}

impl SshTransport {
    fn args(config: &SshConfig, sftp: bool) -> Vec<String> {
        let mut args = vec![
            "-oBatchMode=yes".into(),
            "-oConnectTimeout=5".into(),
            "-oConnectionAttempts=1".into(),
            "-oServerAliveInterval=2".into(),
            "-oServerAliveCountMax=2".into(),
            "-oForwardAgent=no".into(),
            "-oClearAllForwardings=yes".into(),
        ];
        if let Some(user) = &config.user {
            args.push(format!("-oUser={user}"));
        }
        if let Some(port) = config.port {
            args.extend([if sftp { "-P" } else { "-p" }.into(), port.to_string()]);
        }
        if let Some(file) = &config.identity_file {
            args.extend(["-i".into(), file.clone()]);
        }
        args
    }
    fn connection_args(&self, sftp: bool) -> Vec<String> {
        let mut args = Self::args(&self.config, sftp);
        if let Some(host) = self.resolved.get("hostname") {
            args.push(format!("-oHostName={host}"));
        }
        if self.config.user.is_none() {
            if let Some(user) = self.resolved.get("user") {
                args.push(format!("-oUser={user}"));
            }
        }
        if self.config.port.is_none() {
            if let Some(port) = self.resolved.get("port") {
                args.push(format!("-oPort={port}"));
            }
        }
        args
    }
    pub async fn connect(config: &SshConfig) -> Result<Self, XllmError> {
        let out = Command::new("ssh")
            .args(Self::args(config, false))
            .arg("-G")
            .arg(config.host.as_ref().unwrap())
            .kill_on_drop(true)
            .output()
            .await
            .map_err(capability)?;
        if !out.status.success() {
            return Err(capability(format!(
                "ssh config: {}",
                String::from_utf8_lossy(&out.stderr)
            )));
        }
        let resolved = String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|l| l.split_once(' '))
            .filter(|(k, _)| matches!(*k, "hostname" | "user" | "port"))
            .map(|(k, v)| (k.to_string(), v.trim().to_string()))
            .collect();
        Ok(Self {
            config: config.clone(),
            resolved,
            identity: None,
        })
    }
    pub fn with_identity(&self, identity: Identity) -> Self {
        Self {
            identity: Some(identity),
            ..self.clone()
        }
    }
    pub fn target(&self, p: &Probe) -> Value {
        json!({"connection": self.resolved, "host": format!("ssh:{}:{}:{}", p.identity.hostname, p.identity.machine, p.identity.uid), "machine_id": p.identity.machine, "uid": p.identity.uid})
    }
    pub fn verify_identity(&self, i: &Identity) -> ToolResult<()> {
        if self.identity.as_ref().is_some_and(|old| old != i) {
            return Err(transport("SSH target identity changed", false));
        }
        Ok(())
    }
    fn identity_gate(&self) -> String {
        match &self.identity {
            Some(i) => format!("[ \"$(cat /etc/machine-id)\" = {} ] && [ \"$(id -u)\" = {} ] && [ \"$(hostname)\" = {} ] || exit 199\n", shell_quote(&i.machine), shell_quote(&i.uid), shell_quote(&i.hostname)),
            None => String::new(),
        }
    }
    pub async fn script(&self, script: &str, effect: bool) -> ToolResult<Vec<u8>> {
        let mut c = Command::new("ssh");
        c.args(self.connection_args(false))
            .arg(self.config.host.as_ref().unwrap())
            .arg("bash --noprofile --norc -s");
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = c
            .spawn()
            .map_err(|e| transport(format!("ssh unavailable: {e}"), false))?;
        let data = format!("{}{}", self.identity_gate(), script);
        let mut stdin = child.stdin.take().unwrap();
        let writer = tokio::spawn(async move {
            stdin.write_all(data.as_bytes()).await?;
            stdin.shutdown().await
        });
        let out = tokio::time::timeout(Duration::from_secs(12), child.wait_with_output())
            .await
            .map_err(|_| transport("SSH transport timed out", effect))?
            .map_err(|e| transport(e.to_string(), effect))?;
        let _ = writer.await;
        if out.status.code() == Some(199) {
            return Err(transport("SSH target identity changed", effect));
        }
        if !out.status.success() {
            let message = format!(
                "SSH operation failed ({}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
            if out.status.code() == Some(255) || out.status.code().is_none() {
                return Err(transport(message, effect));
            }
            return Err(AgentToolError::ExecFailed(message));
        }
        Ok(out.stdout)
    }
    pub async fn probe(
        &self,
        cwd: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Probe, XllmError> {
        let p = parse_probe(
            &self
                .script(&probe_script(cwd, env), false)
                .await
                .map_err(capability)?,
        )?;
        self.verify_identity(&p.identity).map_err(capability)?;
        Ok(p)
    }
    async fn sftp(&self, batch: &str, effect: bool) -> ToolResult<()> {
        let mut c = Command::new("sftp");
        c.args(self.connection_args(true))
            .args(["-q", "-b", "-"])
            .arg(self.config.host.as_ref().unwrap());
        c.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = c
            .spawn()
            .map_err(|e| transport(format!("sftp unavailable: {e}"), false))?;
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(batch.as_bytes())
            .await
            .map_err(|e| transport(e.to_string(), effect))?;
        drop(stdin);
        let out = tokio::time::timeout(Duration::from_secs(12), child.wait_with_output())
            .await
            .map_err(|_| transport("SFTP transport timed out", effect))?
            .map_err(|e| transport(e.to_string(), effect))?;
        if !out.status.success() {
            return Err(transport(
                format!("SFTP failed: {}", String::from_utf8_lossy(&out.stderr)),
                effect,
            ));
        }
        Ok(())
    }
    pub async fn verify_sftp(&self) -> Result<(), XllmError> {
        let machine = self
            .read(Path::new("/etc/machine-id"))
            .await
            .map_err(capability)?;
        if String::from_utf8_lossy(&machine).trim() != self.identity.as_ref().unwrap().machine {
            return Err(capability("SSH and SFTP do not expose the same filesystem"));
        }
        Ok(())
    }
    /// `/tmp/llm-runtime-<uid>/<run_id>/<call_id>`: the remote execution
    /// directory of one `shell` call.
    pub fn remote_exec_dir(&self, run_id: &str, call_id: &str) -> String {
        let uid = self
            .identity
            .as_ref()
            .map(|i| i.uid.clone())
            .unwrap_or_else(|| "unknown".into());
        format!(
            "/tmp/llm-runtime-{uid}/{}/{}",
            sanitize_call_id(run_id),
            sanitize_call_id(call_id)
        )
    }

    /// Exit code and output tails of a remote execution directory, `None`
    /// when the host or the directory cannot be read.
    pub async fn read_exec_dir(&self, dir: &str) -> Option<ExecDirFacts> {
        let q = shell_quote(dir);
        let script = format!(
            "if [ -d {q} ]; then echo DIR; else echo NODIR; exit 0; fi\nif [ -f {q}/exit ]; then cat -- {q}/exit; fi\necho '\x01'\nif [ -f {q}/stdout ]; then tail -c {n} -- {q}/stdout; fi\necho '\x01'\nif [ -f {q}/stderr ]; then tail -c {n} -- {q}/stderr; fi\n",
            n = OUTPUT_TAIL_BYTES
        );
        let bytes = self.script(&script, false).await.ok()?;
        let text = String::from_utf8_lossy(&bytes).to_string();
        if text.starts_with("NODIR") {
            return Some(ExecDirFacts {
                exit_code: None,
                stdout_tail: String::new(),
                stderr_tail: String::new(),
                exists: false,
            });
        }
        let rest = text.strip_prefix("DIR\n").unwrap_or(&text);
        let mut parts = rest.split('\x01');
        let exit_code = parts
            .next()
            .and_then(|p| p.trim().parse::<i32>().ok());
        let stdout_tail = parts.next().unwrap_or("").trim_start_matches('\n').to_string();
        let stderr_tail = parts.next().unwrap_or("").trim_start_matches('\n').to_string();
        Some(ExecDirFacts {
            exit_code,
            stdout_tail,
            stderr_tail,
            exists: true,
        })
    }
}

struct TempFile(PathBuf);
impl TempFile {
    async fn new(bytes: &[u8]) -> ToolResult<Self> {
        let p = std::env::temp_dir().join(format!("llm-transfer-{}", new_transfer_id()));
        let mut f = tokio::fs::OpenOptions::new();
        f.write(true).create_new(true);
        #[cfg(unix)]
        f.mode(0o600);
        let mut file = f
            .open(&p)
            .await
            .map_err(|e| transport(e.to_string(), false))?;
        file.write_all(bytes)
            .await
            .map_err(|e| transport(e.to_string(), false))?;
        Ok(Self(p))
    }
}
impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn sftp_quote(s: &str) -> ToolResult<String> {
    if s.chars().any(|c| c.is_control()) {
        return Err(AgentToolError::InvalidArgs(
            "SFTP paths cannot contain control characters".into(),
        ));
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        if matches!(c, '\\' | '"' | '*' | '?' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

#[async_trait]
impl FileBackend for SshTransport {
    async fn resolve(&self, root: &Path, raw: &str, allowed: &[PathBuf]) -> ToolResult<PathBuf> {
        let lexical = crate::path_utils::resolve_path_from_root(root, raw)?;
        check_allowed(&lexical, allowed)?;
        let candidate = if raw.starts_with('/') {
            raw.to_string()
        } else {
            format!("{}/{raw}", root.display())
        };
        sftp_quote(&candidate)?;
        let mut script = format!("set -e\nrealpath -m -z -- {}\n", shell_quote(&candidate));
        for r in allowed {
            script.push_str(&format!(
                "realpath -m -z -- {}\n",
                shell_quote(&r.display().to_string())
            ));
        }
        let bytes = self.script(&script, false).await?;
        let s = String::from_utf8(bytes).map_err(|e| transport(e.to_string(), false))?;
        let paths = s
            .strip_suffix('\0')
            .ok_or_else(|| transport("realpath returned unterminated paths", false))?
            .split('\0')
            .collect::<Vec<_>>();
        if paths.len() != allowed.len() + 1 || paths.iter().any(|p| p.is_empty()) {
            return Err(transport("realpath returned incomplete paths", false));
        }
        for p in &paths {
            sftp_quote(p)?;
        }
        let path = PathBuf::from(paths[0]);
        let roots = paths[1..].iter().map(PathBuf::from).collect::<Vec<_>>();
        check_allowed(&path, &roots)?;
        Ok(path)
    }
    async fn exists(&self, path: &Path) -> ToolResult<bool> {
        let b = self
            .script(
                &format!(
                    "if [ -e {} ]; then printf yes; else printf no; fi\n",
                    shell_quote(&path.display().to_string())
                ),
                false,
            )
            .await?;
        Ok(b == b"yes")
    }
    async fn read(&self, path: &Path) -> ToolResult<Vec<u8>> {
        if !self.exists(path).await? {
            return Err(AgentToolError::ExecFailed(format!(
                "read file failed: {} does not exist",
                path.display()
            )));
        }
        self.script(
            &format!(
                "test -f {p} && test -r {p}\n",
                p = shell_quote(&path.display().to_string())
            ),
            false,
        )
        .await?;
        let temp = TempFile::new(b"").await?;
        self.sftp(
            &format!(
                "get {} {}\n",
                sftp_quote(&path.display().to_string())?,
                sftp_quote(&temp.0.display().to_string())?
            ),
            false,
        )
        .await?;
        tokio::fs::read(&temp.0)
            .await
            .map_err(|e| transport(e.to_string(), false))
    }
    async fn write(&self, path: &Path, bytes: &[u8]) -> ToolResult<()> {
        let parent = path
            .parent()
            .ok_or_else(|| AgentToolError::InvalidArgs("file requires parent directory".into()))?;
        let remote = parent.join(format!(".llm-write-{}", new_transfer_id()));
        self.script(
            &format!(
                "mkdir -p -- {}\n",
                shell_quote(&parent.display().to_string())
            ),
            true,
        )
        .await?;
        let temp = TempFile::new(bytes).await?;
        let result = self
            .sftp(
                &format!(
                    "put {} {}\n",
                    sftp_quote(&temp.0.display().to_string())?,
                    sftp_quote(&remote.display().to_string())?
                ),
                true,
            )
            .await;
        if let Err(e) = result {
            let _ = self
                .script(
                    &format!("rm -f -- {}\n", shell_quote(&remote.display().to_string())),
                    false,
                )
                .await;
            return Err(e);
        }
        self.script(
            &format!(
                "set -e\nif [ -e {p} ]; then chmod --reference={p} -- {t}; fi\nmv -f -- {t} {p}\n",
                p = shell_quote(&path.display().to_string()),
                t = shell_quote(&remote.display().to_string())
            ),
            true,
        )
        .await?;
        Ok(())
    }
}

pub struct SshBashRunner {
    transport: Arc<SshTransport>,
    env: BTreeMap<String, String>,
    run: RunBindingSlot,
}

impl SshBashRunner {
    pub fn new(transport: Arc<SshTransport>, env: BTreeMap<String, String>, run: RunBindingSlot) -> Self {
        Self {
            transport,
            env,
            run,
        }
    }

    fn remote_dir(&self, call_id: Option<&str>) -> String {
        let run_id = self
            .run
            .lock()
            .expect("run binding")
            .as_ref()
            .map(|b| b.run_id.clone())
            .unwrap_or_else(|| "unbound".into());
        let call = call_id
            .map(str::to_string)
            .unwrap_or_else(|| format!("anon-{}", new_transfer_id()));
        self.transport.remote_exec_dir(&run_id, &call)
    }
}

/// Remote wrapper, started in the background by one SSH session and polled
/// by later ones: output to files, pid of the command's bash to `pid`
/// (`kill` reads it), exit code to `exit`.
const WRAPPER: &str = r#"llm_dir=$1
bash --noprofile --norc "$llm_dir/command" </dev/null >"$llm_dir/stdout" 2>"$llm_dir/stderr" &
llm_child=$!
printf '%s\n' "$llm_child" > "$llm_dir/pid"
wait "$llm_child"
llm_ec=$?
printf '%s\n' "$llm_ec" > "$llm_dir/exit.tmp"
mv -f "$llm_dir/exit.tmp" "$llm_dir/exit"
"#;

#[async_trait]
impl BashRunner for SshBashRunner {
    fn engine(&self) -> &str {
        "remote_ssh"
    }

    async fn resolve_cwd(
        &self,
        root: &Path,
        raw: Option<&str>,
        restricted: bool,
    ) -> ToolResult<PathBuf> {
        let allowed = if restricted {
            vec![root.to_path_buf()]
        } else {
            Vec::new()
        };
        let cwd = self
            .transport
            .resolve(root, raw.unwrap_or("."), &allowed)
            .await?;
        self.transport
            .script(
                &format!("test -d {}\n", shell_quote(&cwd.display().to_string())),
                false,
            )
            .await?;
        Ok(cwd)
    }

    async fn start(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> ToolResult<Box<dyn CommandHandle>> {
        if let BashTarget::Unsupported(t) = &req.target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported shell target {t}"
            )));
        }
        let dir = self.remote_dir(req.call_id.as_deref());
        let qdir = shell_quote(&dir);
        let base = shell_quote(
            Path::new(&dir)
                .parent()
                .and_then(Path::parent)
                .unwrap()
                .to_str()
                .unwrap(),
        );
        self.transport.script(&format!("set -e\numask 077\nif [ ! -e {base} ]; then mkdir -m 700 -- {base}; fi\n[ -d {base} ] && [ ! -L {base} ] && [ -O {base} ] && [ \"$(stat -c %a -- {base})\" = 700 ]\nmkdir -p -- {qdir}\nrm -f -- {qdir}/exit {qdir}/exit.tmp {qdir}/pid\n"),false).await?;
        let mut env = self.env.clone();
        env.extend(req.env.iter().cloned());
        let mut script = format!(
            "cd {} || exit 126\n",
            shell_quote(&req.cwd.display().to_string())
        );
        for (k, v) in &env {
            script.push_str(&format!("export {k}={}\n", shell_quote(v)));
        }
        script.push_str(&req.command);
        script.push('\n');
        self.transport
            .write(Path::new(&format!("{dir}/command")), script.as_bytes())
            .await?;
        let launch = format!(
            "nohup bash -c {} -- {qdir} </dev/null >/dev/null 2>&1 &\nfor ((i=0; i<100; i++)); do [ -f {qdir}/pid ] && break; sleep .02; done\nif [ -f {qdir}/pid ]; then cat -- {qdir}/pid; fi\n",
            shell_quote(WRAPPER)
        );
        let pid = self.transport.script(&launch, true).await?;
        let pid = String::from_utf8_lossy(&pid).trim().to_string();
        if pid.is_empty() {
            return Err(transport("remote command did not start", true));
        }
        Ok(Box::new(SshCommand {
            transport: self.transport.clone(),
            dir,
            started: Instant::now(),
            max_output: req.max_output_bytes,
            cwd: req.cwd,
            finished: None,
        }))
    }
}

pub struct SshCommand {
    transport: Arc<SshTransport>,
    dir: String,
    started: Instant,
    max_output: usize,
    cwd: PathBuf,
    finished: Option<BashRunOutput>,
}

impl SshCommand {
    async fn poll_exit(&self) -> ToolResult<Option<i32>> {
        let qdir = shell_quote(&self.dir);
        let poll = self
            .transport
            .script(
                &format!("if [ -f {qdir}/exit ]; then cat -- {qdir}/exit; fi\n"),
                true,
            )
            .await?;
        Ok(if poll.is_empty() {
            None
        } else {
            String::from_utf8_lossy(&poll).trim().parse::<i32>().ok()
        })
    }

    async fn collect(&self, exit_code: i32, timed_out: bool) -> ToolResult<BashRunOutput> {
        let qdir = shell_quote(&self.dir);
        let stdout = self
            .transport
            .script(
                &format!(
                    "if [ -f {qdir}/stdout ]; then head -c {} -- {qdir}/stdout; fi\n",
                    self.max_output.saturating_add(1)
                ),
                true,
            )
            .await?;
        let stderr = self
            .transport
            .script(
                &format!(
                    "if [ -f {qdir}/stderr ]; then head -c {} -- {qdir}/stderr; fi\n",
                    self.max_output.saturating_add(1)
                ),
                true,
            )
            .await?;
        Ok(super::output(
            exit_code,
            &stdout,
            &stderr,
            self.max_output,
            timed_out,
            self.started.elapsed(),
            "remote_ssh",
            self.cwd.clone(),
        ))
    }
}

#[async_trait]
impl CommandHandle for SshCommand {
    async fn wait(&mut self, timeout: Duration) -> ToolResult<Option<BashRunOutput>> {
        if let Some(done) = &self.finished {
            return Ok(Some(done.clone()));
        }
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(code) = self.poll_exit().await? {
                let out = self.collect(code, false).await?;
                self.finished = Some(out.clone());
                return Ok(Some(out));
            }
            if Instant::now() >= deadline {
                return Ok(None);
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    async fn progress(&self) -> CommandProgress {
        let qdir = shell_quote(&self.dir);
        let tail = self
            .transport
            .script(
                &format!(
                    "if [ -f {qdir}/stdout ]; then tail -c {} -- {qdir}/stdout; fi\n",
                    OUTPUT_TAIL_BYTES
                ),
                false,
            )
            .await
            .map(|b| String::from_utf8_lossy(&b).to_string())
            .unwrap_or_default();
        CommandProgress {
            elapsed_ms: self.started.elapsed().as_millis() as u64,
            output_tail: tail,
        }
    }

    /// Kills the command's bash (the pid the wrapper recorded) with `kill`.
    /// Children that detached from it are not chased.
    async fn kill(&mut self) -> ToolResult<BashRunOutput> {
        if let Some(done) = &self.finished {
            return Ok(done.clone());
        }
        let qdir = shell_quote(&self.dir);
        self.transport
            .script(
                &format!("if [ -f {qdir}/pid ]; then kill -KILL -- \"$(cat -- {qdir}/pid)\" 2>/dev/null || true; fi\n"),
                true,
            )
            .await
            .map_err(|e| transport(e.to_string(), true))?;
        let code = self.poll_exit().await?.unwrap_or(TIMEOUT_EXIT_CODE);
        let out = self.collect(code, true).await?;
        self.finished = Some(out.clone());
        Ok(out)
    }

    fn detach(&mut self) {}

    fn cancellable(&self) -> bool {
        true
    }

    fn locator(&self) -> String {
        format!("remote execution directory {}", self.dir)
    }
}
