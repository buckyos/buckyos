//! Zone-hosted Agents as msg-center sees them, read from
//! `users/<owner>/agents/<agent_id>/{spec,settings,profile}`. An Agent is
//! recognised only once its `spec` is written: creation still in progress has
//! no spec and cannot be talked to.

use anyhow::Result;
use async_trait::async_trait;
use buckyos_api::{
    agent_profile_key, agent_settings_key, agent_spec_key, AgentId, AgentProfile, AgentSettings,
    AgentSpec, SystemConfigClient, SystemConfigError,
};
use log::warn;
use name_lib::{AgentDocument, DID};
use serde::de::DeserializeOwned;

pub const TELEGRAM_PLATFORM: &str = "telegram";

#[async_trait]
pub trait ConfigSource: Send + Sync {
    async fn list(&self, key: &str) -> Result<Vec<String>>;
    async fn get(&self, key: &str) -> Result<Option<String>>;
}

#[async_trait]
impl ConfigSource for SystemConfigClient {
    async fn list(&self, key: &str) -> Result<Vec<String>> {
        Ok(SystemConfigClient::list(self, key).await?)
    }

    async fn get(&self, key: &str) -> Result<Option<String>> {
        match SystemConfigClient::get(self, key).await {
            Ok(value) => Ok(Some(value.value)),
            Err(SystemConfigError::KeyNotFound(_)) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ZoneAgent {
    pub agent_id: AgentId,
    pub doc: AgentDocument,
    pub settings: AgentSettings,
    pub profile: AgentProfile,
}

impl ZoneAgent {
    pub fn did(&self) -> &DID {
        &self.doc.id
    }

    pub fn owner(&self) -> &DID {
        &self.doc.owner
    }

    pub fn display_name(&self) -> String {
        self.profile.resolved_display_name(&self.agent_id)
    }

    /// The Agent's own Telegram bot (`settings.msg_tunnels`, at most one).
    pub fn telegram_tunnel(&self) -> Option<(&str, Option<&str>)> {
        let tunnel = self.settings.msg_tunnels.iter().find(|tunnel| {
            tunnel
                .platform
                .trim()
                .eq_ignore_ascii_case(TELEGRAM_PLATFORM)
        })?;
        let token = tunnel.bot_token.trim();
        if token.is_empty() {
            warn!(
                "skip telegram tunnel without bot_token for agent {}",
                self.agent_id
            );
            return None;
        }
        Some((token, tunnel.bot_account_id.as_deref()))
    }
}

pub fn parse_agent_spec(value: &str, agent_id: &AgentId) -> Result<AgentSpec> {
    let spec: AgentSpec = serde_json::from_str(value)?;
    spec.validate().map_err(|error| anyhow::anyhow!(error))?;
    anyhow::ensure!(
        &spec.agent_id == agent_id,
        "AgentSpec key does not match its identity"
    );
    Ok(spec)
}

async fn get_or_default<T: DeserializeOwned + Default>(
    source: &dyn ConfigSource,
    key: &str,
) -> Result<T> {
    let Some(value) = source.get(key).await? else {
        return Ok(T::default());
    };
    Ok(serde_json::from_str(&value).unwrap_or_else(|error| {
        warn!("invalid {}, use defaults: {}", key, error);
        T::default()
    }))
}

/// `Ok(None)` while the Agent has no (valid) spec. Storage errors are
/// returned so a failed read never looks like a removed Agent.
pub async fn load_zone_agent(
    source: &dyn ConfigSource,
    owner_user_id: &str,
    agent_id: &AgentId,
) -> Result<Option<ZoneAgent>> {
    let spec_key = agent_spec_key(owner_user_id, agent_id);
    let Some(spec) = source.get(&spec_key).await? else {
        return Ok(None);
    };
    let spec = match parse_agent_spec(&spec, agent_id) {
        Ok(spec) => spec,
        Err(error) => {
            warn!("skip agent with invalid {}: {}", spec_key, error);
            return Ok(None);
        }
    };
    Ok(Some(ZoneAgent {
        agent_id: agent_id.clone(),
        doc: spec.agent_doc,
        settings: get_or_default(source, &agent_settings_key(owner_user_id, agent_id)).await?,
        profile: get_or_default(source, &agent_profile_key(owner_user_id, agent_id)).await?,
    }))
}

pub async fn load_zone_agents(source: &dyn ConfigSource) -> Result<Vec<ZoneAgent>> {
    let mut agents = Vec::new();
    for user_id in source.list("users").await? {
        for name in source.list(&format!("users/{user_id}/agents")).await? {
            let agent_id = match AgentId::parse(&name) {
                Ok(agent_id) => agent_id,
                Err(error) => {
                    warn!("skip users/{}/agents/{}: {}", user_id, name, error);
                    continue;
                }
            };
            if let Some(agent) = load_zone_agent(source, &user_id, &agent_id).await? {
                agents.push(agent);
            }
        }
    }
    Ok(agents)
}

pub async fn find_zone_agent(source: &dyn ConfigSource, did: &DID) -> Result<Option<ZoneAgent>> {
    let Ok(agent_id) = AgentId::from_agent_did(did) else {
        return Ok(None);
    };
    for user_id in source.list("users").await? {
        if let Some(agent) = load_zone_agent(source, &user_id, &agent_id).await? {
            if agent.did() == did {
                return Ok(Some(agent));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
pub mod test_support {
    use super::*;
    use serde_json::{json, Value};
    use std::collections::BTreeMap;

    #[async_trait]
    impl ConfigSource for BTreeMap<String, String> {
        async fn list(&self, key: &str) -> Result<Vec<String>> {
            let prefix = format!("{key}/");
            let mut children: Vec<String> = self
                .keys()
                .filter_map(|path| path.strip_prefix(&prefix))
                .filter_map(|rest| rest.split('/').next())
                .map(str::to_string)
                .collect();
            children.dedup();
            Ok(children)
        }

        async fn get(&self, key: &str) -> Result<Option<String>> {
            Ok(BTreeMap::get(self, key).cloned())
        }
    }

    pub fn agent_document(agent_did: &DID, owner: &DID) -> AgentDocument {
        let (_, public_key) = name_lib::generate_ed25519_key_pair();
        AgentDocument::new(
            agent_did.clone(),
            owner.clone(),
            serde_json::from_value(public_key).unwrap(),
        )
    }

    pub fn agent_spec(agent: &AgentDocument, owner_user_id: &str) -> Value {
        let doc = serde_json::to_value(agent).unwrap();
        let object_id = ndn_lib::build_named_object_by_json("agentdoc", &doc).0;
        let agent_id = AgentId::from_agent_did(&agent.id).unwrap();
        json!({
            "schema_version": buckyos_api::AGENT_SPEC_SCHEMA_VERSION,
            "agent_id": agent_id,
            "agent_did": agent.id,
            "agent_doc_object_id": object_id,
            "agent_doc": doc,
            "binding": {
                "schema_version": buckyos_api::AGENT_SPEC_SCHEMA_VERSION,
                "agent_did": agent.id,
                "agent_doc_object_id": object_id,
                "target_app_instance_id": format!("{agent_id}@{owner_user_id}"),
                "service_name": "www",
                "generation": 1
            },
            "generation": 1
        })
    }

    /// Writes the records of an Agent: `spec` only when `with_spec`.
    pub fn put_agent(
        config: &mut BTreeMap<String, String>,
        owner_user_id: &str,
        agent: &AgentDocument,
        with_spec: bool,
        settings: Value,
        profile: Value,
    ) {
        let agent_id = AgentId::from_agent_did(&agent.id).unwrap();
        if with_spec {
            config.insert(
                agent_spec_key(owner_user_id, &agent_id),
                agent_spec(agent, owner_user_id).to_string(),
            );
        }
        config.insert(
            agent_settings_key(owner_user_id, &agent_id),
            settings.to_string(),
        );
        config.insert(
            agent_profile_key(owner_user_id, &agent_id),
            profile.to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn only_agents_with_a_spec_are_recognised() {
        let alice = DID::new("web", "alice.test.buckyos.io");
        let ready = agent_document(&DID::new("web", "xiaobai.test.buckyos.io"), &alice);
        let creating = agent_document(&DID::new("web", "xiaohei.test.buckyos.io"), &alice);
        let mut config = BTreeMap::new();
        put_agent(
            &mut config,
            "alice",
            &ready,
            true,
            json!({"allow_group": true}),
            json!({"display_name": "小白"}),
        );
        put_agent(&mut config, "alice", &creating, false, json!({}), json!({}));
        config.insert("users/alice/settings".into(), "{}".into());

        let agents = load_zone_agents(&config).await.unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].did(), &ready.id);
        assert_eq!(agents[0].owner(), &alice);
        assert!(agents[0].settings.allow_group);
        assert_eq!(agents[0].display_name(), "小白");

        let found = find_zone_agent(&config, &ready.id).await.unwrap().unwrap();
        assert_eq!(found, agents[0]);
        assert!(find_zone_agent(&config, &creating.id)
            .await
            .unwrap()
            .is_none());
        assert!(find_zone_agent(&config, &alice).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn missing_settings_and_profile_fall_back_to_defaults() {
        let alice = DID::new("web", "alice.test.buckyos.io");
        let agent = agent_document(&DID::new("web", "xiaobai.test.buckyos.io"), &alice);
        let agent_id = AgentId::from_agent_did(&agent.id).unwrap();
        let mut config = BTreeMap::new();
        config.insert(
            agent_spec_key("alice", &agent_id),
            agent_spec(&agent, "alice").to_string(),
        );

        let loaded = load_zone_agent(&config, "alice", &agent_id)
            .await
            .unwrap()
            .unwrap();
        assert!(!loaded.settings.allow_group);
        assert!(loaded.telegram_tunnel().is_none());
        assert_eq!(loaded.display_name(), "xiaobai");

        assert!(load_zone_agent(&config, "bob", &agent_id)
            .await
            .unwrap()
            .is_none());
        let other = AgentId::parse("xiaohei.test.buckyos.io").unwrap();
        config.insert(
            agent_spec_key("alice", &other),
            agent_spec(&agent, "alice").to_string(),
        );
        assert!(load_zone_agent(&config, "alice", &other)
            .await
            .unwrap()
            .is_none());
    }

    #[test]
    fn telegram_tunnel_needs_a_token() {
        let agent = |settings: serde_json::Value| ZoneAgent {
            agent_id: AgentId::parse("xiaobai.test.buckyos.io").unwrap(),
            doc: agent_document(
                &DID::new("web", "xiaobai.test.buckyos.io"),
                &DID::new("web", "alice.test.buckyos.io"),
            ),
            settings: serde_json::from_value(settings).unwrap(),
            profile: AgentProfile::default(),
        };
        assert_eq!(
            agent(json!({"msg_tunnels": [{"platform": "telegram", "bot_token": " 1:a ", "bot_account_id": "@b"}]}))
                .telegram_tunnel(),
            Some(("1:a", Some("@b")))
        );
        assert!(
            agent(json!({"msg_tunnels": [{"platform": "telegram", "bot_token": " "}]}))
                .telegram_tunnel()
                .is_none()
        );
        assert!(
            agent(json!({"msg_tunnels": [{"platform": "lark", "bot_token": "x"}]}))
                .telegram_tunnel()
                .is_none()
        );
    }
}
