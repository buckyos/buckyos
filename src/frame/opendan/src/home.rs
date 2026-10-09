//! What the WebUI's home page needs beyond the Agent State: the agent's
//! profile card, token usage by model and where a UI session's conversation
//! lives in the message system.
//!
//! - `agent.profile` / `agent.profile_set {display_name?, avatar?, bio?}`:
//!   nickname, avatar and a short introduction. Defaults come from
//!   `agent.toml [identity]`; what the owner edits is kept in
//!   `<agent_root>/.meta/profile.json`.
//! - `usage.models`: tokens by model over the last hour, the last 24 hours
//!   and in total, summed from the sessions' `usage.jsonl`.
//! - `ui.bindings`: the conversation each UI session is bound to.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::{fsutil, OpenDanError, SessionDir};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const PROFILE_FILE: &str = ".meta/profile.json";
const MAX_NAME_CHARS: usize = 64;
const MAX_BIO_CHARS: usize = 500;
const MAX_AVATAR_BYTES: usize = 128 * 1024;
const HOUR_MS: u64 = 3_600_000;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct StoredProfile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    avatar: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bio: Option<String>,
    #[serde(default)]
    updated_at_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct IdentitySection {
    #[serde(default)]
    display_name: Option<String>,
    #[serde(default)]
    avatar: Option<String>,
    #[serde(default, alias = "description")]
    bio: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct IdentityToml {
    #[serde(default)]
    identity: IdentitySection,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Tokens {
    input: u64,
    output: u64,
    total: u64,
}

impl Tokens {
    fn add(&mut self, r: &UsageRecord) {
        self.input += r.input_tokens;
        self.output += r.output_tokens;
        self.total += r.total_tokens;
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
struct Windows {
    hour: Tokens,
    day: Tokens,
    all: Tokens,
}

struct UsageCache {
    len: u64,
    records: Vec<UsageRecord>,
}

pub struct Home {
    agent_root: PathBuf,
    /// Parsed `usage.jsonl` by session, re-read when the file has grown.
    usage: Mutex<HashMap<String, UsageCache>>,
}

fn text_param(params: &Value, name: &str) -> Result<Option<String>, OpenDanError> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.trim().to_string())),
        Some(_) => Err(OpenDanError::InvalidArgument(format!("param `{name}` is a string"))),
    }
}

impl Home {
    pub fn new(agent_root: &Path) -> Self {
        Self {
            agent_root: agent_root.to_path_buf(),
            usage: Mutex::new(HashMap::new()),
        }
    }

    fn stored(&self) -> StoredProfile {
        fsutil::read_json_opt(&self.agent_root.join(PROFILE_FILE))
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    fn package_identity(&self) -> IdentitySection {
        std::fs::read_to_string(self.agent_root.join("agent.toml"))
            .ok()
            .and_then(|t| toml::from_str::<IdentityToml>(&t).ok())
            .unwrap_or_default()
            .identity
    }

    /// The profile card. `owner_did` / `desktop_url` are `null` outside a
    /// zone: there is no message system to link to.
    pub fn profile(
        &self,
        agent: &dyn AgentStateClient,
        owner_did: Option<String>,
        desktop_url: Option<String>,
    ) -> Value {
        let stored = self.stored();
        let package = self.package_identity();
        let pick = |own: Option<String>, base: Option<String>| own.or(base).filter(|s| !s.is_empty());
        json!({
            "agent_did": agent.agent_did(),
            "agent_id": agent.agent_id(),
            "display_name": pick(stored.display_name, package.display_name)
                .unwrap_or_else(|| agent.agent_id().to_string()),
            "avatar": pick(stored.avatar, package.avatar),
            "bio": pick(stored.bio, package.bio).unwrap_or_default(),
            "owner_did": owner_did,
            "desktop_url": desktop_url,
            "updated_at_ms": stored.updated_at_ms,
        })
    }

    /// Change the fields that are present; an empty string goes back to the
    /// package's default.
    pub fn set_profile(&self, params: &Value) -> Result<(), OpenDanError> {
        let invalid = |m: String| Err(OpenDanError::InvalidArgument(m));
        let mut stored = self.stored();
        if let Some(name) = text_param(params, "display_name")? {
            if name.chars().count() > MAX_NAME_CHARS {
                return invalid(format!("display_name is longer than {MAX_NAME_CHARS} characters"));
            }
            stored.display_name = Some(name).filter(|s| !s.is_empty());
        }
        if let Some(bio) = text_param(params, "bio")? {
            if bio.chars().count() > MAX_BIO_CHARS {
                return invalid(format!("bio is longer than {MAX_BIO_CHARS} characters"));
            }
            stored.bio = Some(bio).filter(|s| !s.is_empty());
        }
        if let Some(avatar) = text_param(params, "avatar")? {
            if avatar.len() > MAX_AVATAR_BYTES {
                return invalid(format!("avatar is larger than {} KiB", MAX_AVATAR_BYTES / 1024));
            }
            let known = ["data:image/", "https://", "http://"];
            if !avatar.is_empty() && !known.iter().any(|p| avatar.starts_with(p)) {
                return invalid("avatar is an image data URL or an http(s) URL".to_string());
            }
            stored.avatar = Some(avatar).filter(|s| !s.is_empty());
        }
        stored.updated_at_ms = libopendan::now_ms();
        fsutil::atomic_replace_json(&self.agent_root.join(PROFILE_FILE), &stored)
    }

    fn session_usage(&self, entry: &RegistryEntry) -> Vec<UsageRecord> {
        let Ok(sd) = SessionDir::open(&entry.location) else {
            return Vec::new();
        };
        let len = fsutil::file_len(&sd.file(USAGE_FILE)).unwrap_or(0);
        let mut cache = self.usage.lock().expect("usage cache");
        match cache.get(&entry.session_id) {
            Some(c) if c.len == len => c.records.clone(),
            _ => {
                let records = sd.usage().unwrap_or_default();
                cache.insert(
                    entry.session_id.clone(),
                    UsageCache {
                        len,
                        records: records.clone(),
                    },
                );
                records
            }
        }
    }

    /// Tokens by model, most used first. Only Rounds recorded in
    /// `usage.jsonl` count: sessions older than that file have none.
    pub async fn usage_models(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let now = libopendan::now_ms();
        let mut models: BTreeMap<String, Windows> = BTreeMap::new();
        let mut since_ms: Option<u64> = None;
        for entry in agent.sessions().query(&RegistryQuery::default()).await? {
            for r in self.session_usage(&entry) {
                let w = models.entry(r.model.clone()).or_default();
                w.all.add(&r);
                if r.at_ms + 24 * HOUR_MS >= now {
                    w.day.add(&r);
                }
                if r.at_ms + HOUR_MS >= now {
                    w.hour.add(&r);
                }
                since_ms = Some(since_ms.map_or(r.at_ms, |s| s.min(r.at_ms)));
            }
        }
        let mut models: Vec<(String, Windows)> = models.into_iter().collect();
        models.sort_by(|a, b| b.1.all.total.cmp(&a.1.all.total).then(a.0.cmp(&b.0)));
        Ok(json!({
            "now_ms": now,
            "since_ms": since_ms,
            "models": models
                .into_iter()
                .map(|(model, w)| json!({ "model": model, "hour": w.hour, "day": w.day, "all": w.all }))
                .collect::<Vec<_>>(),
        }))
    }

    /// The conversation of every UI session: the peer (or group) its
    /// replies go to and the conversation's session there.
    pub async fn ui_bindings(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let query = RegistryQuery {
            kind: Some(SessionKind::Ui),
            ..Default::default()
        };
        let mut out = Vec::new();
        for entry in agent.sessions().query(&query).await? {
            let outbound = SessionDir::open(&entry.location)
                .and_then(|sd| sd.config())
                .ok()
                .and_then(|cfg| cfg.channels.outbound);
            if let Some(o) = outbound {
                out.push(json!({
                    "session_id": entry.session_id,
                    "to": o.to,
                    "to_session": o.to_session,
                    "kind": o.kind,
                }));
            }
        }
        Ok(json!(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_edits_are_checked_and_kept() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".meta")).unwrap();
        std::fs::write(dir.path().join("agent.toml"), "[identity]\ndisplay_name = \"Jarvis\"\n").unwrap();
        let home = Home::new(dir.path());
        assert_eq!(home.package_identity().display_name.as_deref(), Some("Jarvis"));

        home.set_profile(&json!({ "display_name": " J ", "bio": "hello", "avatar": "data:image/png;base64,AA==" }))
            .unwrap();
        let stored = home.stored();
        assert_eq!(stored.display_name.as_deref(), Some("J"));
        assert_eq!(stored.bio.as_deref(), Some("hello"));

        // Absent fields stay; an empty string goes back to the default.
        home.set_profile(&json!({ "display_name": "" })).unwrap();
        let stored = home.stored();
        assert!(stored.display_name.is_none() && stored.avatar.is_some());

        for bad in [
            json!({ "avatar": "javascript:alert(1)" }),
            json!({ "display_name": "x".repeat(65) }),
            json!({ "bio": 1 }),
            json!({ "avatar": format!("data:image/png;base64,{}", "A".repeat(MAX_AVATAR_BYTES)) }),
        ] {
            assert!(matches!(home.set_profile(&bad), Err(OpenDanError::InvalidArgument(_))), "{bad}");
        }
    }
}
