//! What the WebUI's home page needs beyond the Agent State: the agent's
//! profile card, token usage by model and where a UI session's conversation
//! lives in the message system.
//!
//! - `agent.profile` / `agent.profile_set {display_name?, avatar?, bio?}`:
//!   nickname, avatar and a short introduction, the agent's `profile`
//!   record in the zone (the same one the control panel edits). Without a
//!   nickname the agent is called by its user name.
//! - `usage.models`: tokens by model over the last hour, the last 24 hours
//!   and in total, summed from the sessions' `usage.jsonl`.
//! - `ui.bindings`: the conversation each UI session is bound to.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use buckyos_api::{AgentId, AgentProfile};
use libopendan::protocol::*;
use libopendan::state::AgentStateClient;
use libopendan::{fsutil, OpenDanError, SessionDir};
use serde::Serialize;
use serde_json::{json, Value};

use crate::records::{self, AgentRecords};

const MAX_NAME_CHARS: usize = 64;
const MAX_BIO_CHARS: usize = 500;
const MAX_AVATAR_BYTES: usize = 128 * 1024;
const HOUR_MS: u64 = 3_600_000;

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
    records: Arc<dyn AgentRecords>,
    /// `None` outside a zone, like `desktop_url` (the zone desktop, where
    /// MessageHub is): there is no message system to link to.
    owner_did: Option<String>,
    desktop_url: Option<String>,
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

fn store_err(e: String) -> OpenDanError {
    OpenDanError::Other(format!("agent profile: {e}"))
}

impl Home {
    pub fn new(records: Arc<dyn AgentRecords>, owner_did: Option<String>, desktop_url: Option<String>) -> Self {
        Self {
            records,
            owner_did,
            desktop_url,
            usage: Mutex::new(HashMap::new()),
        }
    }

    /// The profile card.
    pub async fn profile(&self, agent: &dyn AgentStateClient) -> Result<Value, OpenDanError> {
        let profile = records::profile(self.records.as_ref()).await.map_err(store_err)?;
        let display_name = parse_did(agent.agent_did())
            .ok()
            .and_then(|did| AgentId::from_agent_did(&did).ok())
            .map(|id| profile.resolved_display_name(&id))
            .unwrap_or_else(|| agent.agent_id().to_string());
        Ok(json!({
            "agent_did": agent.agent_did(),
            "agent_id": agent.agent_id(),
            "display_name": display_name,
            "avatar": profile.avatar,
            "bio": profile.bio.unwrap_or_default(),
            "owner_did": self.owner_did,
            "desktop_url": self.desktop_url,
        }))
    }

    /// Change the fields that are present; an empty string clears one (the
    /// name falls back to the agent's user name).
    pub async fn set_profile(&self, params: &Value) -> Result<(), OpenDanError> {
        let mut profile = records::profile(self.records.as_ref()).await.map_err(store_err)?;
        apply_profile(&mut profile, params)?;
        records::set_profile(self.records.as_ref(), &profile)
            .await
            .map_err(store_err)
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

fn apply_profile(profile: &mut AgentProfile, params: &Value) -> Result<(), OpenDanError> {
    let invalid = |m: String| Err(OpenDanError::InvalidArgument(m));
    let name = text_param(params, "display_name")?;
    let bio = text_param(params, "bio")?;
    let avatar = text_param(params, "avatar")?;
    if name.as_ref().is_some_and(|n| n.chars().count() > MAX_NAME_CHARS) {
        return invalid(format!("display_name is longer than {MAX_NAME_CHARS} characters"));
    }
    if bio.as_ref().is_some_and(|b| b.chars().count() > MAX_BIO_CHARS) {
        return invalid(format!("bio is longer than {MAX_BIO_CHARS} characters"));
    }
    if let Some(avatar) = &avatar {
        if avatar.len() > MAX_AVATAR_BYTES {
            return invalid(format!("avatar is larger than {} KiB", MAX_AVATAR_BYTES / 1024));
        }
        if !avatar.is_empty() && !avatar.starts_with("data:image/") {
            return invalid("avatar is an image data URL".to_string());
        }
    }
    for (field, value) in [
        (&mut profile.display_name, name),
        (&mut profile.bio, bio),
        (&mut profile.avatar, avatar),
    ] {
        if let Some(v) = value {
            *field = Some(v).filter(|s| !s.is_empty());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::records::LocalRecords;

    #[tokio::test]
    async fn profile_edits_are_checked_and_kept() {
        let records = Arc::new(LocalRecords::memory());
        let home = Home::new(records.clone(), Some("did:bns:bob".into()), None);
        home.set_profile(&json!({ "display_name": " J ", "bio": "hello", "avatar": "data:image/png;base64,AA==" }))
            .await
            .unwrap();
        let stored = records::profile(records.as_ref()).await.unwrap();
        assert_eq!(stored.display_name.as_deref(), Some("J"));
        assert_eq!(stored.bio.as_deref(), Some("hello"));

        // Absent fields stay; an empty string clears one.
        home.set_profile(&json!({ "display_name": "" })).await.unwrap();
        let stored = records::profile(records.as_ref()).await.unwrap();
        assert!(stored.display_name.is_none() && stored.avatar.is_some());

        for bad in [
            json!({ "avatar": "javascript:alert(1)" }),
            json!({ "avatar": "https://example.com/a.png" }),
            json!({ "display_name": "x".repeat(65) }),
            json!({ "bio": 1 }),
            json!({ "avatar": format!("data:image/png;base64,{}", "A".repeat(MAX_AVATAR_BYTES)) }),
        ] {
            assert!(matches!(home.set_profile(&bad).await, Err(OpenDanError::InvalidArgument(_))), "{bad}");
        }
        assert_eq!(records::profile(records.as_ref()).await.unwrap(), stored, "a refused edit changes nothing");
    }
}
