//! The Session context of a Session process (primary-runtime TODO §6.1).
//!
//! A command a session runs on its session-runtime — the shell, its scripts,
//! `agent-session` and the other helpers — is a Session process. On the
//! machine of the agent's own process (AgentService) it finds its session
//! through the environment (`OPENDAN_*`) and the AgentRoot. On another
//! machine (a node runtime) no `OPENDAN_*` variable arrives (executors pass
//! none) and the AgentRoot is not there: the session's working directory
//! holds `.opendan/session.json` and a token AgentService accepts for this
//! session only, and a helper reaches AgentService over kRPC
//! (`session.helper`, `session.task`, `session.token_refresh`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::error::{OpenDanError, Result};
use crate::protocol::*;
use crate::session::SessionDir;
use crate::state::AgentStateClient;

/// Directory of the context in a session's working directory.
pub const CONTEXT_DIR: &str = ".opendan";
pub const CONTEXT_FILE: &str = "session.json";
/// The token, next to the context (rewritten when it is renewed).
pub const TOKEN_FILE: &str = "session.token";
/// How long a session token is valid; every drive writes a new one, a helper
/// renews it within [`TOKEN_RENEW_BEFORE_MS`] of its end.
pub const TOKEN_TTL_MS: u64 = 24 * 60 * 60 * 1000;
pub const TOKEN_RENEW_BEFORE_MS: u64 = 60 * 60 * 1000;
/// AgentService's address as a node session-runtime reaches it, when not
/// the node's own gateway (set in AgentService's environment).
pub const ENV_NODE_STATE_URL: &str = "OPENDAN_NODE_STATE_URL";
/// Longest a forwarded helper command may run (`wait` waits up to 30 min).
pub const HELPER_TIMEOUT_MS: u64 = 35 * 60 * 1000;
/// Output of a forwarded helper kept per stream.
pub const HELPER_MAX_OUTPUT: usize = 1024 * 1024;

const TOKEN_PREFIX: &str = "odsess1";
const KEY_FILE: &str = "session_token.key";

/// `<workdir>/.opendan/session.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionContext {
    pub agent_did: String,
    pub agent_id: String,
    pub session_id: String,
    pub trace_id: String,
    /// The session-runtime.
    pub runtime_id: String,
    /// The session's working directory on it.
    pub workdir: String,
    /// AgentService as this runtime reaches it.
    pub state_url: String,
    /// The token's file name, next to this file.
    pub token_file: String,
    pub expires_at_ms: u64,
    /// The session's workspace (`runtime::workspace_view`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Value>,
}

/// What a session token says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionClaims {
    pub session_id: String,
    pub expires_at_ms: u64,
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |byte: u8| block.iter().map(|b| b ^ byte).collect::<Vec<u8>>();
    let inner = Sha256::new().chain_update(pad(0x36)).chain_update(msg).finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

fn invalid_token(why: &str) -> OpenDanError {
    OpenDanError::InvalidArgument(format!("session token: {why}"))
}

/// Whether `token` is a session token (not a BuckyOS session token).
pub fn is_session_token(token: &str) -> bool {
    token.starts_with(&format!("{TOKEN_PREFIX}."))
}

/// When a session token ends, as it says (not verified).
pub fn token_expiry(token: &str) -> Option<u64> {
    token.rsplitn(3, '.').nth(1)?.parse().ok()
}

/// The key session tokens of an agent are signed with
/// (`<agent_root>/state/session_token.key`): whoever drives the agent's
/// sessions with its AgentRoot signs, AgentService verifies.
pub struct TokenKey {
    key: Vec<u8>,
}

impl TokenKey {
    pub fn load_or_create(agent_root: &Path) -> Result<Self> {
        let path = agent_root.join("state").join(KEY_FILE);
        match std::fs::read(&path) {
            Ok(key) if key.len() >= 32 => return Ok(Self { key }),
            Ok(_) => return Err(OpenDanError::Other(format!("{}: key too short", path.display()))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(OpenDanError::io(&path, e)),
        }
        let mut key = Vec::with_capacity(32);
        key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        key.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| OpenDanError::io(dir, e))?;
        }
        // Two signers racing: the first key published wins.
        if !crate::fsutil::publish_noreplace(&path, &key)? {
            return Self::load_or_create(agent_root);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
        }
        Ok(Self { key })
    }

    fn mac(&self, agent_id: &str, sid: &str, expires_at_ms: u64) -> String {
        hex::encode(hmac_sha256(
            &self.key,
            format!("{TOKEN_PREFIX}|{agent_id}|{sid}|{expires_at_ms}").as_bytes(),
        ))
    }

    /// A token for session `sid` of `agent_id`, valid until `expires_at_ms`.
    pub fn sign(&self, agent_id: &str, sid: &str, expires_at_ms: u64) -> String {
        format!("{TOKEN_PREFIX}.{sid}.{expires_at_ms}.{}", self.mac(agent_id, sid, expires_at_ms))
    }

    pub fn verify(&self, agent_id: &str, token: &str, now_ms: u64) -> Result<SessionClaims> {
        let mut parts = token.rsplitn(3, '.');
        let (Some(mac), Some(exp), Some(head)) = (parts.next(), parts.next(), parts.next()) else {
            return Err(invalid_token("malformed"));
        };
        let sid = head
            .strip_prefix(TOKEN_PREFIX)
            .and_then(|s| s.strip_prefix('.'))
            .ok_or_else(|| invalid_token("malformed"))?;
        let expires_at_ms: u64 = exp.parse().map_err(|_| invalid_token("malformed"))?;
        let expected = self.mac(agent_id, sid, expires_at_ms);
        let same = expected.len() == mac.len()
            && expected.bytes().zip(mac.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;
        if !same {
            return Err(invalid_token("not issued for this agent"));
        }
        if expires_at_ms <= now_ms {
            return Err(invalid_token("expired"));
        }
        Ok(SessionClaims {
            session_id: sid.to_string(),
            expires_at_ms,
        })
    }
}

/// AgentService as a node session-runtime reaches it: the node's own
/// gateway, which forwards `/kapi/<agent_id>` to the agent's service.
pub fn node_state_url(agent_id: &str) -> String {
    std::env::var(ENV_NODE_STATE_URL)
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| {
            format!(
                "http://127.0.0.1:{}/kapi/{agent_id}",
                buckyos_api::DEFAULT_NODE_GATEWAY_PORT
            )
        })
}

/// The context of `cfg`'s session working in `workdir` on its runtime,
/// with a fresh token.
pub fn new_context(
    key: &TokenKey,
    agent_id: &str,
    cfg: &SessionConfig,
    workdir: &str,
    state_url: String,
) -> (SessionContext, String) {
    let expires_at_ms = crate::now_ms() + TOKEN_TTL_MS;
    let sid = cfg.session.session_id.clone();
    let token = key.sign(agent_id, &sid, expires_at_ms);
    (
        SessionContext {
            agent_did: cfg.session.agent_did.clone(),
            agent_id: agent_id.to_string(),
            trace_id: sid.clone(),
            session_id: sid,
            runtime_id: cfg.runtime.runtime_id().to_string(),
            workdir: workdir.to_string(),
            state_url,
            token_file: TOKEN_FILE.to_string(),
            expires_at_ms,
            workspace: crate::runtime::workspace_view(cfg),
        },
        token,
    )
}

/// Write a context and its token into `workdir` on a runtime, through that
/// runtime's file primitives.
pub async fn write_context(
    files: &dyn agent_tool::runtime::files::RuntimeFiles,
    ctx: &SessionContext,
    token: &str,
) -> Result<()> {
    use agent_tool::runtime::files::{WholeWriteMode, WriteChunk};
    let body = serde_json::to_vec_pretty(ctx).map_err(|e| OpenDanError::Other(e.to_string()))?;
    for (name, data) in [(TOKEN_FILE, token.as_bytes()), (CONTEXT_FILE, body.as_slice())] {
        let path = crate::runtime::target_join(&ctx.workdir, &[CONTEXT_DIR, name]);
        files
            .write_chunk(
                &path,
                WriteChunk {
                    mode: WholeWriteMode::Replace,
                    expected_size: None,
                    expected_hash: None,
                    data,
                },
            )
            .await
            .map_err(|e| OpenDanError::Bind(format!("write {path}: {}", e.message)))?;
    }
    Ok(())
}

/// The context a Session process works in: `explicit`, else
/// `.opendan/session.json` in `start` or the nearest parent that has one.
pub fn discover(explicit: Option<&Path>, start: &Path) -> Option<(PathBuf, SessionContext)> {
    let read = |p: &Path| -> Option<SessionContext> { serde_json::from_slice(&std::fs::read(p).ok()?).ok() };
    if let Some(p) = explicit {
        return read(p).map(|c| (p.to_path_buf(), c));
    }
    let mut dir = Some(start);
    while let Some(d) = dir {
        let p = d.join(CONTEXT_DIR).join(CONTEXT_FILE);
        if let Some(c) = read(&p) {
            return Some((p, c));
        }
        dir = d.parent();
    }
    None
}

/// The token file of the context at `context_path`.
pub fn token_path(context_path: &Path, ctx: &SessionContext) -> PathBuf {
    context_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&ctx.token_file)
}

/// What a helper forwards to AgentService (`session.helper`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HelperRequest {
    /// The `agent-session` arguments, without the program name.
    pub argv: Vec<String>,
    /// What the command reads from stdin (`--content -`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdin: Option<String>,
    /// The tool call the command belongs to (`EXECUTOR_CALL_ID` there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HelperOutput {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
}

fn capped(bytes: Vec<u8>) -> String {
    let mut s = String::from_utf8_lossy(&bytes).into_owned();
    if s.len() > HELPER_MAX_OUTPUT {
        let mut cut = HELPER_MAX_OUTPUT;
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        s.truncate(cut);
        s.push_str("\n[output truncated]\n");
    }
    s
}

/// Run an `agent-session` command for session `sid` on the control side,
/// where the session directory and the AgentRoot are: the session's
/// environment (as a local session's shell has it), its directory as the
/// working directory, `session_cli` as the program.
pub async fn run_helper(
    agent: &dyn AgentStateClient,
    session_cli: &Path,
    sid: &str,
    req: &HelperRequest,
) -> Result<HelperOutput> {
    let entry = agent
        .sessions()
        .lookup(sid)
        .await?
        .ok_or_else(|| OpenDanError::NotFound(format!("session {sid}")))?;
    let sd = SessionDir::open(&entry.location)?;
    let cfg = sd.config()?;
    let ctx = crate::runtime::SessionEnvCtx {
        session_id: sid.to_string(),
        session_dir: sd.path().to_path_buf(),
        agent_did: cfg.session.agent_did.clone(),
        agent_root: agent.agent_root().map(Path::to_path_buf),
        input_queue: cfg.channels.kmsg().map(|(_, q, _)| q.to_string()),
        trace_id: sid.to_string(),
        extra_env: cfg.runtime.env.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
        cmd_skills: crate::runtime::frozen_cmd_skills(&cfg),
        workspace: cfg.workspace_binding.clone(),
    };
    let mut env = crate::runtime::session_env_vars(&ctx, cfg.runtime.runtime_id()).await?;
    if let Some(call) = req.call_id.as_ref().filter(|c| !c.is_empty()) {
        env.push((agent_tool::llm_bash::ENV_CALL_ID.to_string(), call.clone()));
    }
    let mut cmd = tokio::process::Command::new(session_cli);
    cmd.args(&req.argv)
        .current_dir(sd.path())
        .envs(env)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd
        .spawn()
        .map_err(|e| OpenDanError::Other(format!("start {}: {e}", session_cli.display())))?;
    if let Some(mut stdin) = child.stdin.take() {
        if let Some(text) = &req.stdin {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(text.as_bytes()).await;
        }
    }
    let out = tokio::time::timeout(
        std::time::Duration::from_millis(HELPER_TIMEOUT_MS),
        child.wait_with_output(),
    )
    .await
    .map_err(|_| OpenDanError::Other(format!("agent-session {} did not finish in time", req.argv.join(" "))))?
    .map_err(|e| OpenDanError::Other(e.to_string()))?;
    Ok(HelperOutput {
        exit_code: out.status.code().unwrap_or(1),
        stdout: capped(out.stdout),
        stderr: capped(out.stderr),
    })
}

/// Methods a session token may call: the session's own operations (a copy
/// between runtimes is `agent-session copy`, forwarded as a helper).
pub fn is_session_method(method: &str) -> bool {
    matches!(
        method,
        "session.helper" | "session.task" | "session.token_refresh" | "session.service_token"
    )
}

/// The BuckyOS identity a Session process's service tools act with: the
/// agent's own (`session.service_token`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceIdentity {
    pub token: String,
    pub app_id: String,
    pub owner_user_id: String,
}

impl ServiceIdentity {
    /// The environment the buckyos-agent commands and the `buckyos` CLI
    /// read an injected identity from, reaching the zone through this
    /// machine's node gateway.
    pub fn env(&self) -> Vec<(String, String)> {
        vec![
            ("BUCKYOS_APPCLIENT_SESSION_TOKEN".into(), self.token.clone()),
            ("BUCKYOS_APP_ID".into(), self.app_id.clone()),
            ("BUCKYOS_OWNER_USER_ID".into(), self.owner_user_id.clone()),
            ("BUCKYOS_HOST_GATEWAY".into(), "127.0.0.1".into()),
            ("BUCKYOS_NODE_GATEWAY_PORT".into(), buckyos_api::DEFAULT_NODE_GATEWAY_PORT.to_string()),
        ]
    }
}

/// A Session process's connection to AgentService: the context it found
/// and the session token next to it, renewed when it nears its end.
pub struct SessionClient {
    pub path: PathBuf,
    pub ctx: SessionContext,
}

impl SessionClient {
    /// The context of the current directory (or `explicit`).
    pub fn discover(explicit: Option<&Path>) -> Option<Self> {
        let cwd = std::env::current_dir().ok()?;
        discover(explicit, &cwd).map(|(path, ctx)| Self { path, ctx })
    }

    async fn raw_call(&self, token: &str, method: &str, params: Value) -> Result<Value> {
        let timeout = std::time::Duration::from_millis(HELPER_TIMEOUT_MS + 60_000);
        let client = ::kRPC::kRPC::new_with_timeout(&self.ctx.state_url, Some(token.to_string()), timeout);
        client.call(method, params).await.map_err(|e| match e {
            ::kRPC::RPCErrors::ReasonError(text) => crate::state::krpc::error_from_wire(&text),
            other => OpenDanError::Forbidden(format!("AgentService at {} ({method}): {other}", self.ctx.state_url)),
        })
    }

    /// The session's token, renewed when it is close to its end.
    pub async fn token(&self) -> Result<String> {
        let path = token_path(&self.path, &self.ctx);
        let token = std::fs::read_to_string(&path)
            .map_err(|e| OpenDanError::io(&path, e))?
            .trim()
            .to_string();
        let now = crate::now_ms();
        let expires = token_expiry(&token).unwrap_or(0);
        if expires > now + TOKEN_RENEW_BEFORE_MS {
            return Ok(token);
        }
        if expires <= now {
            return Err(OpenDanError::Forbidden(
                "the session token expired: the next drive of the session writes a new one".into(),
            ));
        }
        let v = self
            .raw_call(&token, "session.token_refresh", serde_json::json!({ "sid": self.ctx.session_id }))
            .await?;
        let renewed = v["token"]
            .as_str()
            .ok_or_else(|| OpenDanError::Other("session.token_refresh returned no token".into()))?;
        let tmp = path.with_extension("token.new");
        std::fs::write(&tmp, renewed).map_err(|e| OpenDanError::io(&tmp, e))?;
        std::fs::rename(&tmp, &path).map_err(|e| OpenDanError::io(&path, e))?;
        Ok(renewed.to_string())
    }

    pub async fn call(&self, method: &str, mut params: Value) -> Result<Value> {
        let token = self.token().await?;
        if let Some(o) = params.as_object_mut() {
            o.entry("sid").or_insert_with(|| Value::String(self.ctx.session_id.clone()));
        }
        self.raw_call(&token, method, params).await
    }

    pub async fn service_identity(&self) -> Result<ServiceIdentity> {
        let v = self.call("session.service_token", serde_json::json!({})).await?;
        serde_json::from_value(v).map_err(|e| OpenDanError::Other(format!("session.service_token: {e}")))
    }
}

/// AgentService's side of the Session context: session tokens and the
/// methods a Session process calls with one.
pub struct SessionService {
    key: TokenKey,
    /// The `agent-session` program forwarded helpers run as.
    session_cli: Option<PathBuf>,
}

impl SessionService {
    pub fn new(agent_root: &Path, session_cli: Option<PathBuf>) -> Result<Self> {
        Ok(Self {
            key: TokenKey::load_or_create(agent_root)?,
            session_cli,
        })
    }

    pub fn authenticate(&self, agent_id: &str, token: &str) -> Result<SessionClaims> {
        self.key.verify(agent_id, token, crate::now_ms())
    }

    /// A session method called with `claims`: only for that session.
    pub async fn call(
        &self,
        agent: &dyn AgentStateClient,
        claims: &SessionClaims,
        method: &str,
        params: &Value,
    ) -> Result<Value> {
        let sid = claims.session_id.as_str();
        if params.get("sid").and_then(Value::as_str).is_some_and(|asked| asked != sid) {
            return Err(OpenDanError::InvalidArgument(format!(
                "the session token is for session {sid}"
            )));
        }
        if agent.sessions().lookup(sid).await?.is_none() {
            return Err(OpenDanError::NotFound(format!("session {sid}")));
        }
        match method {
            "session.token_refresh" => {
                let expires_at_ms = crate::now_ms() + TOKEN_TTL_MS;
                Ok(serde_json::json!({
                    "token": self.key.sign(agent.agent_id(), sid, expires_at_ms),
                    "expires_at_ms": expires_at_ms,
                }))
            }
            "session.task" => {
                let req: agent_tool::task_bridge::TaskBridgeRequest = param(params, "request")?;
                match crate::runner::answer_session_task(sid, &req).await {
                    Some(r) => to_json(&r),
                    None => to_json(&agent_tool::task_bridge::TaskBridgeResponse {
                        state: "unknown".into(),
                        text: format!("session {sid} has no run in progress on this host: its tasks are answered while it runs"),
                    }),
                }
            }
            "session.service_token" => {
                let runtime = buckyos_api::get_buckyos_api_runtime().map_err(|e| {
                    OpenDanError::Other(format!("this host has no BuckyOS identity to lend: {e}"))
                })?;
                Box::pin(runtime.renew_token_from_verify_hub())
                    .await
                    .map_err(|e| OpenDanError::Other(format!("renew the agent's session token: {e}")))?;
                to_json(&ServiceIdentity {
                    token: runtime.get_session_token().await,
                    app_id: runtime.get_app_id(),
                    owner_user_id: runtime.get_owner_user_id().unwrap_or_default(),
                })
            }
            "session.helper" => {
                let req: HelperRequest = param(params, "request")?;
                let cli = self.session_cli.as_deref().ok_or_else(|| {
                    OpenDanError::Other("this host has no agent-session program to run helpers with".into())
                })?;
                to_json(&run_helper(agent, cli, sid, &req).await?)
            }
            other => Err(OpenDanError::InvalidArgument(format!("`{other}` is not a session method"))),
        }
    }
}

fn param<T: serde::de::DeserializeOwned>(params: &Value, name: &str) -> Result<T> {
    serde_json::from_value(params.get(name).cloned().unwrap_or(Value::Null))
        .map_err(|e| OpenDanError::InvalidArgument(format!("param `{name}`: {e}")))
}

fn to_json<T: Serialize>(v: &T) -> Result<Value> {
    serde_json::to_value(v).map_err(|e| OpenDanError::Other(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_rfc4231_case_2() {
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            hex::encode(mac),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn tokens_bind_agent_session_and_time() {
        let root = tempfile::tempdir().unwrap();
        let key = TokenKey::load_or_create(root.path()).unwrap();
        let again = TokenKey::load_or_create(root.path()).unwrap();
        let token = key.sign("jarvis.test", "s-1.a", 2_000);
        assert!(is_session_token(&token));
        assert_eq!(token_expiry(&token), Some(2_000));
        assert_eq!(
            again.verify("jarvis.test", &token, 1_000).unwrap(),
            SessionClaims { session_id: "s-1.a".into(), expires_at_ms: 2_000 }
        );
        assert!(key.verify("other.test", &token, 1_000).is_err());
        assert!(key.verify("jarvis.test", &token, 2_000).is_err());
        let forged = token.replace("s-1.a", "s-2");
        assert!(key.verify("jarvis.test", &forged, 1_000).is_err());
        let other = TokenKey::load_or_create(tempfile::tempdir().unwrap().path()).unwrap();
        assert!(other.verify("jarvis.test", &token, 1_000).is_err());
    }

    #[test]
    fn context_is_found_from_below_the_workdir() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = SessionContext {
            agent_did: "did:web:a".into(),
            agent_id: "a".into(),
            session_id: "s".into(),
            trace_id: "s".into(),
            runtime_id: "host".into(),
            workdir: dir.path().display().to_string(),
            state_url: "http://127.0.0.1:3180/kapi/a".into(),
            token_file: TOKEN_FILE.into(),
            expires_at_ms: 1,
            workspace: None,
        };
        std::fs::create_dir_all(dir.path().join(CONTEXT_DIR)).unwrap();
        std::fs::write(dir.path().join(CONTEXT_DIR).join(CONTEXT_FILE), serde_json::to_vec(&ctx).unwrap()).unwrap();
        let deep = dir.path().join("x").join("y");
        std::fs::create_dir_all(&deep).unwrap();
        let (path, found) = discover(None, &deep).unwrap();
        assert_eq!(found, ctx);
        assert_eq!(token_path(&path, &found), dir.path().join(CONTEXT_DIR).join(TOKEN_FILE));
        assert!(discover(None, tempfile::tempdir().unwrap().path()).is_none());
    }
}
