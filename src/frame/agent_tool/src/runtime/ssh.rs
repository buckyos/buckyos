use super::files::{check_allowed, FileBackend};
use super::{shell_quote, RuntimeInfo, SshConfig};
use crate::exec_tracking::{new_execution_id, ExecutionRecord, ExecutionRegistrar, EXECUTION_ENV};
use crate::llm_bash::{BashRunOutput, BashRunRequest, BashRunner, BashTarget};
use crate::xllm::XllmError;
use crate::{AgentToolError, SessionRuntimeContext};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

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
command -v setsid >/dev/null
command -v realpath >/dev/null
command -v base64 >/dev/null
[ -r /proc/self/stat ]
printf '%s\0' "$(cat /etc/machine-id)" "$(hostname)" "$(id -u)" "$(uname -s | tr '[:upper:]' '[:lower:]')" "$(uname -m)" "$(command -v bash)" "$(pwd -P)" "${PATH}" "$(date -Iseconds)" "$(date +%Z%z)"
for tool in bash setsid realpath base64; do printf '%s\0' "$tool"; done
"#;

fn probe_script(cwd: &str, env: &BTreeMap<String, String>) -> String {
    let mut s = format!("cd {} || exit 126\n", shell_quote(cwd));
    for (k, v) in env {
        s.push_str(&format!("export {k}={}\n", shell_quote(v)));
    }
    s.push_str(PROBE);
    s
}
fn parse_probe(bytes: &[u8]) -> Result<Probe, XllmError> {
    let p = bytes
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).to_string())
        .collect::<Vec<_>>();
    if p.len() < 10 {
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

pub async fn local_probe(cwd: &Path, env: &BTreeMap<String, String>) -> Result<Probe, XllmError> {
    let mut command = Command::new("bash");
    command.arg("-c").arg(r#"set -e
command -v bash >/dev/null
printf '%s\0' "$(cat /etc/machine-id 2>/dev/null || hostname)" "$(hostname)" "$(id -u)" "$(uname -s | tr '[:upper:]' '[:lower:]')" "$(uname -m)" "$(command -v bash)" "$(pwd -P)" "${PATH}" "$(date +%Y-%m-%dT%H:%M:%S%z)" "$(date +%Z%z)"
for tool in bash setsid realpath base64; do if command -v "$tool" >/dev/null; then printf '%s\0' "$tool"; fi; done
"#).current_dir(cwd).envs(env).kill_on_drop(true);
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
    parse_probe(&o.stdout)
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
    fn execution_dir(&self, id: &str) -> ToolResult<String> {
        if !id.starts_with("ex-") || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            return Err(transport("invalid execution id", false));
        }
        Ok(format!(
            "/tmp/llm-runtime-{}/{id}",
            self.identity
                .as_ref()
                .ok_or_else(|| transport("SSH not opened", false))?
                .uid
        ))
    }
    async fn inspect_execution(&self, rec: &ExecutionRecord, terminate: bool) -> ToolResult<bool> {
        let current = self
            .probe("/", &BTreeMap::new())
            .await
            .map_err(|e| transport(e.to_string(), false))?;
        if rec.kind != "remote_ssh"
            || rec.host.as_deref() != self.target(&current).get("host").and_then(Value::as_str)
        {
            return Err(transport(
                "RecoveryBlocked: execution target does not match",
                false,
            ));
        }
        let boot = rec
            .boot_id
            .as_deref()
            .ok_or_else(|| transport("RecoveryBlocked: execution has no boot id", false))?;
        let pgid = rec
            .pgid
            .ok_or_else(|| transport("RecoveryBlocked: execution has no process group", false))?;
        let ticks = rec
            .leader_start_ticks
            .ok_or_else(|| transport("RecoveryBlocked: execution has no leader identity", false))?;
        let script = format!(
            "llm_kill={}\nllm_exid={}\nllm_boot={}\nllm_pgid={pgid}\nllm_ticks={ticks}\n{}",
            if terminate { 1 } else { 0 },
            shell_quote(&rec.execution_id),
            shell_quote(boot),
            STOP
        );
        let stopped = self.script(&script, false).await? == b"STOPPED\n";
        if stopped {
            let dir = self.execution_dir(&rec.execution_id)?;
            self.script(&format!("rm -rf -- {}\n", shell_quote(&dir)), false)
                .await?;
        }
        Ok(stopped)
    }
    pub async fn stop(&self, rec: &ExecutionRecord) -> ToolResult<()> {
        if self.inspect_execution(rec, true).await? {
            Ok(())
        } else {
            Err(transport(
                "RecoveryBlocked: remote execution not stopped",
                false,
            ))
        }
    }
}

const STOP: &str = r#"set -e
[ "$(cat /proc/sys/kernel/random/boot_id)" = "$llm_boot" ] || { echo STOPPED; exit 0; }
for ((llm_round=0; llm_round<50; llm_round++)); do
  llm_alive=0
  llm_reused=0
  if [ -r /proc/$llm_pgid/stat ]; then
    IFS= read -r llm_stat < /proc/$llm_pgid/stat || true
    llm_rest=${llm_stat##*) }
    read -ra llm_fields <<< "$llm_rest"
    [ "${llm_fields[19]}" = "$llm_ticks" ] || llm_reused=1
  fi
  for llm_proc in /proc/[0-9]*; do
    [ -r "$llm_proc/stat" ] || continue
    IFS= read -r llm_stat < "$llm_proc/stat" || continue
    llm_rest=${llm_stat##*) }
    read -ra llm_fields <<< "$llm_rest"
    [[ ${llm_fields[0]} != Z && ${llm_fields[0]} != X ]] || continue
    if (tr '\0' '\n' < "$llm_proc/environ" 2>/dev/null | grep -Fxq "OPENDAN_EXECUTION_ID=$llm_exid"); then
      llm_alive=1
      IFS= read -r llm_check < "$llm_proc/stat" || continue
      llm_check=${llm_check##*) }
      read -ra llm_verify <<< "$llm_check"
      if [[ $llm_kill = 1 && "${llm_verify[19]}" = "${llm_fields[19]}" ]]; then kill -KILL "${llm_proc##*/}" 2>/dev/null || true; fi
    elif [[ $llm_reused = 0 && ${llm_fields[2]} = "$llm_pgid" && ${llm_fields[19]} -ge $llm_ticks ]]; then
      echo 'RecoveryBlocked: process in execution group cannot be verified' >&2
      exit 197
    fi
  done
  if [ "$llm_alive" = 0 ]; then echo STOPPED; exit 0; fi
  if [ "$llm_kill" = 0 ]; then echo ALIVE; exit 0; fi
  sleep .1
done
echo 'RecoveryBlocked: execution still alive' >&2
exit 198
"#;

struct TempFile(PathBuf);
impl TempFile {
    async fn new(bytes: &[u8]) -> ToolResult<Self> {
        let p = std::env::temp_dir().join(format!("llm-transfer-{}", new_execution_id()));
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
        let mut script = format!("realpath -m -- {}\n", shell_quote(&candidate));
        for r in allowed {
            script.push_str(&format!(
                "realpath -m -- {}\n",
                shell_quote(&r.display().to_string())
            ));
        }
        let bytes = self.script(&script, false).await?;
        let s = String::from_utf8(bytes).map_err(|e| transport(e.to_string(), false))?;
        let mut lines = s.lines();
        let path = PathBuf::from(
            lines
                .next()
                .ok_or_else(|| transport("realpath returned no path", false))?,
        );
        let roots = lines.map(PathBuf::from).collect::<Vec<_>>();
        check_allowed(&path, &roots)?;
        sftp_quote(&path.display().to_string())?;
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
        let remote = parent.join(format!(".llm-write-{}", new_execution_id()));
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
    runtime_id: String,
    registrar: Arc<dyn ExecutionRegistrar>,
    env: BTreeMap<String, String>,
    active: Arc<Mutex<BTreeMap<String, ExecutionRecord>>>,
}
impl SshBashRunner {
    pub fn new(
        transport: Arc<SshTransport>,
        id: &str,
        registrar: Arc<dyn ExecutionRegistrar>,
        env: BTreeMap<String, String>,
    ) -> Self {
        Self {
            transport,
            runtime_id: id.into(),
            registrar,
            env,
            active: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }
}
struct RemoteGuard {
    transport: Arc<SshTransport>,
    rec: Option<ExecutionRecord>,
    registrar: Arc<dyn ExecutionRegistrar>,
}
impl Drop for RemoteGuard {
    fn drop(&mut self) {
        if let Some(rec) = self.rec.take() {
            let ssh = self.transport.clone();
            let registrar = self.registrar.clone();
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    if ssh.stop(&rec).await.is_ok() {
                        registrar.completed(&rec.execution_id).await;
                    }
                });
            }
        }
    }
}

const WRAPPER: &str = r#"set -e
llm_dir=$1
llm_pid=$BASHPID
IFS= read -r llm_stat < /proc/$llm_pid/stat
llm_rest=${llm_stat##*) }
read -ra llm_fields <<< "$llm_rest"
printf '%s\n' "$(cat /proc/sys/kernel/random/boot_id)" "$llm_pid" "${llm_fields[19]}" > "$llm_dir/identity"
for ((llm_wait=0; llm_wait<300; llm_wait++)); do
  [ -e "$llm_dir/go" ] && break
  sleep .1
done
[ -e "$llm_dir/go" ] || exit 125
set +e
bash --noprofile --norc "$llm_dir/command" </dev/null >"$llm_dir/stdout" 2>"$llm_dir/stderr"
llm_ec=$?
printf '%s\n' "$llm_ec" > "$llm_dir/exit.tmp"
mv "$llm_dir/exit.tmp" "$llm_dir/exit"
"#;

#[async_trait]
impl BashRunner for SshBashRunner {
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
    async fn run(
        &self,
        _ctx: &SessionRuntimeContext,
        req: BashRunRequest,
    ) -> ToolResult<BashRunOutput> {
        if let BashTarget::Unsupported(t) = &req.target {
            return Err(AgentToolError::InvalidArgs(format!(
                "unsupported exec target {t}"
            )));
        }
        let old = self
            .active
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for rec in old {
            if self.transport.inspect_execution(&rec, false).await? {
                self.active.lock().unwrap().remove(&rec.execution_id);
                self.registrar.completed(&rec.execution_id).await;
            }
        }
        let started = Instant::now();
        let id = new_execution_id();
        let dir = self.transport.execution_dir(&id)?;
        let qdir = shell_quote(&dir);
        let base = shell_quote(Path::new(&dir).parent().unwrap().to_str().unwrap());
        self.transport.script(&format!("set -e\numask 077\nif [ ! -e {base} ]; then mkdir -m 700 -- {base}; fi\n[ -d {base} ] && [ ! -L {base} ] && [ -O {base} ] && [ \"$(stat -c %a -- {base})\" = 700 ]\nmkdir -- {qdir}\n"),false).await?;
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
        self.transport
            .write(Path::new(&format!("{dir}/command")), script.as_bytes())
            .await?;
        let launch = format!("setsid env {EXECUTION_ENV}={} bash -c {} -- {qdir} </dev/null >/dev/null 2>&1 &\nfor ((i=0; i<100; i++)); do [ -f {qdir}/identity ] && break; sleep .02; done\ncat -- {qdir}/identity\n", shell_quote(&id), shell_quote(WRAPPER));
        let ident = self.transport.script(&launch, false).await?;
        let ident = String::from_utf8_lossy(&ident);
        let p = ident.lines().collect::<Vec<_>>();
        if p.len() != 3 {
            return Err(transport("invalid SSH execution handshake", false));
        }
        let identity = self.transport.identity.as_ref().unwrap();
        let rec = ExecutionRecord {
            execution_id: id.clone(),
            call_id: None,
            kind: "remote_ssh".into(),
            runtime_id: Some(self.runtime_id.clone()),
            host: Some(format!(
                "ssh:{}:{}:{}",
                identity.hostname, identity.machine, identity.uid
            )),
            boot_id: Some(p[0].into()),
            pgid: p[1].parse().ok(),
            leader_start_ticks: p[2].parse().ok(),
            command: req.command.chars().take(512).collect(),
            started_at_ms: crate::exec_tracking::now_ms(),
        };
        let mut guard = RemoteGuard {
            transport: self.transport.clone(),
            rec: Some(rec.clone()),
            registrar: self.registrar.clone(),
        };
        self.registrar.register(&rec).await.map_err(|e| {
            transport(
                format!("execution registration failed; command not started: {e}"),
                false,
            )
        })?;
        self.active.lock().unwrap().insert(id.clone(), rec.clone());
        self.transport
            .script(&format!("touch -- {qdir}/go\n"), true)
            .await?;
        let mut exit = None;
        while started.elapsed() < Duration::from_millis(req.timeout_ms.max(1)) {
            let poll = self
                .transport
                .script(
                    &format!("if [ -f {qdir}/exit ]; then cat -- {qdir}/exit; fi\n"),
                    true,
                )
                .await?;
            if !poll.is_empty() {
                exit = String::from_utf8_lossy(&poll).trim().parse::<i32>().ok();
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let timed_out = exit.is_none();
        let stdout = self
            .transport
            .script(
                &format!(
                    "if [ -f {qdir}/stdout ]; then head -c {} -- {qdir}/stdout; fi\n",
                    req.max_output_bytes.saturating_add(1)
                ),
                true,
            )
            .await?;
        let stderr = self
            .transport
            .script(
                &format!(
                    "if [ -f {qdir}/stderr ]; then head -c {} -- {qdir}/stderr; fi\n",
                    req.max_output_bytes.saturating_add(1)
                ),
                true,
            )
            .await?;
        let stopped = self
            .transport
            .inspect_execution(&rec, timed_out)
            .await
            .map_err(|e| transport(e.to_string(), true))?;
        guard.rec = None;
        if stopped {
            self.active.lock().unwrap().remove(&id);
            self.registrar.completed(&id).await;
        }
        Ok(super::output(
            exit.unwrap_or(124),
            &stdout,
            &stderr,
            req.max_output_bytes,
            timed_out,
            started.elapsed(),
            "remote_ssh",
            req.cwd,
        ))
    }
    async fn cancel(&self) -> ToolResult<()> {
        let active = self
            .active
            .lock()
            .unwrap()
            .values()
            .cloned()
            .collect::<Vec<_>>();
        for rec in active {
            self.transport
                .stop(&rec)
                .await
                .map_err(|e| transport(e.to_string(), true))?;
            self.registrar.completed(&rec.execution_id).await;
            self.active.lock().unwrap().remove(&rec.execution_id);
        }
        Ok(())
    }
}
