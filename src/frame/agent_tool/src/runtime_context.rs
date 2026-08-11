use std::env;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::{normalize_abs_path, AgentToolError};

pub const OPENDAN_AGENT_ROOT_ENV: &str = "OPENDAN_AGENT_ROOT";
pub const OPENDAN_SESSION_ID_ENV: &str = "OPENDAN_SESSION_ID";
pub const BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV: &str = "BUCKYOS_APPCLIENT_SESSION_TOKEN";
pub const OPENDAN_TRACE_ID_ENV: &str = "OPENDAN_TRACE_ID";

pub const DEFAULT_TRACE_ID: &str = "cli-trace";
pub const DEFAULT_SESSION_ID: &str = "cli-session";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeContextSource {
    StableEnv,
    DevFallback,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentIdentity {
    pub owner_user_id: String,
    pub agent_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeContext {
    pub agent_root: PathBuf,
    pub session_id: String,
    pub session_root: PathBuf,
    pub appclient_session_token: Option<String>,
    pub trace_id: String,
    pub identity: Option<AgentIdentity>,
    pub source: RuntimeContextSource,
}

impl RuntimeContext {
    pub fn from_process_env(
        current_dir: &Path,
        allow_dev_fallback: bool,
    ) -> Result<Self, AgentToolError> {
        if let Some(agent_root) = path_env(OPENDAN_AGENT_ROOT_ENV, current_dir) {
            return Self::from_agent_root(
                agent_root,
                session_id_from_env(allow_dev_fallback)?,
                string_env(BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV),
                trace_id_from_env(),
                RuntimeContextSource::StableEnv,
            );
        }

        if allow_dev_fallback {
            return Self::from_agent_root(
                canonicalize_or_normalize(current_dir.to_path_buf(), None),
                DEFAULT_SESSION_ID.to_string(),
                string_env(BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV),
                trace_id_from_env(),
                RuntimeContextSource::DevFallback,
            );
        }

        Err(AgentToolError::ExecFailed(format!(
            "missing required ${OPENDAN_AGENT_ROOT_ENV}"
        )))
    }

    pub fn from_agent_root(
        agent_root: PathBuf,
        session_id: String,
        appclient_session_token: Option<String>,
        trace_id: String,
        source: RuntimeContextSource,
    ) -> Result<Self, AgentToolError> {
        let agent_root = canonicalize_or_normalize(agent_root, None);
        let session_id = normalize_required_env_value(OPENDAN_SESSION_ID_ENV, session_id)?;
        let session_root = agent_root.join("sessions").join(&session_id);
        let identity = resolve_agent_identity(&agent_root);
        Ok(Self {
            agent_root,
            session_id,
            session_root,
            appclient_session_token,
            trace_id,
            identity,
            source,
        })
    }

    pub fn is_dev_fallback(&self) -> bool {
        matches!(self.source, RuntimeContextSource::DevFallback)
    }

    pub fn require_identity(&self) -> Result<&AgentIdentity, AgentToolError> {
        self.identity.as_ref().ok_or_else(|| {
            AgentToolError::ExecFailed(format!(
                "missing Agent RootFS identity metadata under {}; expected canonical $BUCKYOS_ROOT/data/home/<owner>/.local/share/<agent_id>, agent.toml owner_user_id+agent_id, or .meta/agent_identity.json",
                self.agent_root.display()
            ))
        })
    }

    pub fn require_appclient_session_token(&self) -> Result<&str, AgentToolError> {
        self.appclient_session_token
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| {
                AgentToolError::ExecFailed(format!(
                    "missing required ${BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV} for BuckyOS runtime access"
                ))
            })
    }
}

fn session_id_from_env(allow_dev_fallback: bool) -> Result<String, AgentToolError> {
    if let Some(value) = string_env(OPENDAN_SESSION_ID_ENV) {
        return Ok(value);
    }
    if allow_dev_fallback {
        return Ok(DEFAULT_SESSION_ID.to_string());
    }
    Err(AgentToolError::ExecFailed(format!(
        "missing required ${OPENDAN_SESSION_ID_ENV}"
    )))
}

fn trace_id_from_env() -> String {
    string_env(OPENDAN_TRACE_ID_ENV).unwrap_or_else(|| DEFAULT_TRACE_ID.to_string())
}

fn normalize_required_env_value(key: &str, value: String) -> Result<String, AgentToolError> {
    let value = value.trim().to_string();
    if value.is_empty() {
        return Err(AgentToolError::ExecFailed(format!(
            "missing required ${key}"
        )));
    }
    Ok(value)
}

fn string_env(key: &str) -> Option<String> {
    env::var(key)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn path_env(key: &str, current_dir: &Path) -> Option<PathBuf> {
    env::var_os(key).map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            canonicalize_or_normalize(path, None)
        } else {
            canonicalize_or_normalize(path, Some(current_dir))
        }
    })
}

fn canonicalize_or_normalize(path: PathBuf, base_dir: Option<&Path>) -> PathBuf {
    let absolute = if path.is_absolute() {
        path
    } else {
        base_dir.map(|base| base.join(&path)).unwrap_or(path)
    };
    std::fs::canonicalize(&absolute).unwrap_or_else(|_| normalize_abs_path(&absolute))
}

fn resolve_agent_identity(agent_root: &Path) -> Option<AgentIdentity> {
    read_json_identity(agent_root)
        .or_else(|| read_agent_toml_identity(agent_root))
        .or_else(|| canonical_path_identity(agent_root))
}

#[derive(Deserialize)]
struct JsonIdentity {
    owner_user_id: Option<String>,
    agent_id: Option<String>,
}

fn read_json_identity(agent_root: &Path) -> Option<AgentIdentity> {
    let raw = std::fs::read_to_string(agent_root.join(".meta/agent_identity.json")).ok()?;
    let parsed: JsonIdentity = serde_json::from_str(&raw).ok()?;
    build_identity(parsed.owner_user_id, parsed.agent_id)
}

fn read_agent_toml_identity(agent_root: &Path) -> Option<AgentIdentity> {
    let raw = std::fs::read_to_string(agent_root.join("agent.toml")).ok()?;
    let mut owner_user_id = None;
    let mut agent_id = None;
    let mut section = String::new();
    for line in raw.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line.trim_matches(['[', ']']).trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = parse_toml_string_value(value.trim())?;
        if key == "owner_user_id" && (section.is_empty() || section == "identity") {
            owner_user_id = Some(value);
        } else if (key == "agent_id" || key == "app_id")
            && (section.is_empty() || section == "identity")
        {
            agent_id = Some(value);
        }
    }
    build_identity(owner_user_id, agent_id)
}

fn parse_toml_string_value(raw: &str) -> Option<String> {
    let value = raw.trim().trim_matches(',');
    if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_string());
    }
    if value.starts_with('\'') && value.ends_with('\'') && value.len() >= 2 {
        return Some(value[1..value.len() - 1].to_string());
    }
    if value.contains(['[', '{']) {
        return None;
    }
    Some(value.to_string())
}

fn canonical_path_identity(agent_root: &Path) -> Option<AgentIdentity> {
    let parts = agent_root
        .components()
        .map(|component| component.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>();
    for window in parts.windows(6) {
        if window[0] == "data"
            && window[1] == "home"
            && window[3] == ".local"
            && window[4] == "share"
        {
            return build_identity(Some(window[2].clone()), Some(window[5].clone()));
        }
    }
    None
}

fn build_identity(
    owner_user_id: Option<String>,
    agent_id: Option<String>,
) -> Option<AgentIdentity> {
    let owner_user_id = owner_user_id?.trim().to_string();
    let agent_id = agent_id?.trim().to_string();
    if owner_user_id.is_empty() || agent_id.is_empty() {
        return None;
    }
    Some(AgentIdentity {
        owner_user_id,
        agent_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn resolves_identity_from_json_metadata() {
        let dir = tempdir().unwrap();
        let meta = dir.path().join(".meta");
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(
            meta.join("agent_identity.json"),
            r#"{"owner_user_id":"alice","agent_id":"buckyos_jarvis"}"#,
        )
        .unwrap();
        let identity = resolve_agent_identity(dir.path()).unwrap();
        assert_eq!(identity.owner_user_id, "alice");
        assert_eq!(identity.agent_id, "buckyos_jarvis");
    }

    #[test]
    fn resolves_identity_from_agent_toml() {
        let dir = tempdir().unwrap();
        std::fs::write(
            dir.path().join("agent.toml"),
            "[identity]\nowner_user_id = \"alice\"\nagent_id = \"buckyos_jarvis\"\n",
        )
        .unwrap();
        let identity = resolve_agent_identity(dir.path()).unwrap();
        assert_eq!(identity.owner_user_id, "alice");
        assert_eq!(identity.agent_id, "buckyos_jarvis");
    }

    #[test]
    fn resolves_identity_from_canonical_path() {
        let root = PathBuf::from("/opt/buckyos/data/home/alice/.local/share/buckyos_jarvis");
        let identity = resolve_agent_identity(&root).unwrap();
        assert_eq!(identity.owner_user_id, "alice");
        assert_eq!(identity.agent_id, "buckyos_jarvis");
    }

    #[test]
    fn arbitrary_root_without_metadata_has_no_identity() {
        let dir = tempdir().unwrap();
        assert!(resolve_agent_identity(dir.path()).is_none());
    }

    #[test]
    fn minimal_contract_builds_complete_context() {
        let dir = tempdir().unwrap();
        let meta = dir.path().join(".meta");
        std::fs::create_dir_all(&meta).unwrap();
        std::fs::write(
            meta.join("agent_identity.json"),
            r#"{"owner_user_id":"alice","agent_id":"buckyos_jarvis"}"#,
        )
        .unwrap();

        let ctx = RuntimeContext::from_agent_root(
            dir.path().to_path_buf(),
            "sess-1".to_string(),
            Some("token-1".to_string()),
            "trace-1".to_string(),
            RuntimeContextSource::StableEnv,
        )
        .unwrap();

        assert_eq!(ctx.session_id, "sess-1");
        assert_eq!(
            ctx.session_root,
            ctx.agent_root.join("sessions").join("sess-1")
        );
        assert_eq!(ctx.trace_id, "trace-1");
        assert!(!ctx.is_dev_fallback());
        assert_eq!(ctx.require_identity().unwrap().agent_id, "buckyos_jarvis");
        assert_eq!(ctx.require_appclient_session_token().unwrap(), "token-1");
    }

    #[test]
    fn empty_session_id_is_rejected() {
        let dir = tempdir().unwrap();
        let err = RuntimeContext::from_agent_root(
            dir.path().to_path_buf(),
            "   ".to_string(),
            None,
            DEFAULT_TRACE_ID.to_string(),
            RuntimeContextSource::StableEnv,
        )
        .unwrap_err();
        assert!(format!("{err}").contains(OPENDAN_SESSION_ID_ENV));
    }

    #[test]
    fn rpc_token_missing_yields_clear_error() {
        let dir = tempdir().unwrap();
        // No token, and an empty/whitespace token both count as missing.
        for token in [None, Some("   ".to_string())] {
            let ctx = RuntimeContext::from_agent_root(
                dir.path().to_path_buf(),
                "sess-1".to_string(),
                token,
                DEFAULT_TRACE_ID.to_string(),
                RuntimeContextSource::StableEnv,
            )
            .unwrap();
            let err = ctx.require_appclient_session_token().unwrap_err();
            assert!(
                format!("{err}").contains(BUCKYOS_APPCLIENT_SESSION_TOKEN_ENV),
                "error should name the required env var"
            );
        }
    }
}
