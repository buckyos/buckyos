use name_lib::DID;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::{fmt, str::FromStr};

fn validate_dns_hostname(value: &str) -> Result<(), String> {
    if value.is_empty() || value.len() > 253 || value.contains('@') {
        return Err("hostname must be 1..=253 bytes and must not contain `@`".to_string());
    }
    if value != value.to_ascii_lowercase() || !value.is_ascii() {
        return Err("hostname must be canonical lowercase ASCII".to_string());
    }
    for label in value.split('.') {
        if label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(format!("invalid DNS label `{label}`"));
        }
    }
    Ok(())
}

fn canonical_hostname_did(did: &DID, identity_kind: &str) -> Result<String, String> {
    let canonical = did.to_string();
    if canonical != format!("did:{}:{}", did.method, did.id)
        || did.method.is_empty()
        || did.id.is_empty()
        || did.method != did.method.to_ascii_lowercase()
        || did.id != did.id.to_ascii_lowercase()
        || did.id.contains(':')
        || did.id.contains('#')
        || did.id.contains('%')
    {
        return Err(format!(
            "{identity_kind} must be a canonical lowercase hostname-form DID without path, port, encoding, or fragment"
        ));
    }

    let raw = did.to_raw_host_name();
    validate_dns_hostname(&raw)?;
    if did.method == "web" && raw.ends_with(".did") {
        return Err(format!(
            "{identity_kind} did:web hostnames ending in `.did` are reserved"
        ));
    }

    let round_trip = parse_raw_hostname(&raw)?;
    if round_trip != *did {
        return Err(format!(
            "{identity_kind} raw hostname does not round-trip: {canonical} -> {raw} -> {}",
            round_trip.to_string()
        ));
    }
    Ok(raw)
}

fn parse_raw_hostname(value: &str) -> Result<DID, String> {
    validate_dns_hostname(value)?;
    let parsed = DID::from_str(value).map_err(|error| error.to_string())?;
    if value.ends_with(".did") {
        let labels: Vec<&str> = value.split('.').collect();
        if labels.len() < 3 {
            return Err("reserved DID hostname must contain id, method, and `did` labels".into());
        }
        let method = labels[labels.len() - 2];
        let id = labels[..labels.len() - 2].join(".");
        let expected = DID::new(method, &id);
        if parsed != expected {
            return Ok(expected);
        }
    }
    Ok(parsed)
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppId(String);

impl AppId {
    pub fn from_app_did(app_did: &DID) -> Result<Self, String> {
        canonical_hostname_did(app_did, "AppDID").map(Self)
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let value = value.as_ref().trim();
        let did = parse_raw_hostname(value)?;
        let canonical = Self::from_app_did(&did)?;
        if canonical.as_str() != value {
            return Err("AppId is not a canonical AppDID raw hostname".to_string());
        }
        Ok(canonical)
    }

    pub fn app_did(&self) -> DID {
        parse_raw_hostname(&self.0).expect("validated AppId must remain reversible")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AppId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AppId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for AppId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AppId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AppInstanceId {
    app_id: AppId,
    owner_user_id: String,
}

impl AppInstanceId {
    pub fn from_app_did(app_did: &DID, owner_user_id: impl Into<String>) -> Result<Self, String> {
        Self::new(AppId::from_app_did(app_did)?, owner_user_id)
    }

    pub fn new(app_id: AppId, owner_user_id: impl Into<String>) -> Result<Self, String> {
        let owner_user_id = owner_user_id.into();
        validate_owner_user_id(&owner_user_id)?;
        Ok(Self {
            app_id,
            owner_user_id,
        })
    }

    pub fn app_id(&self) -> &AppId {
        &self.app_id
    }

    pub fn owner_user_id(&self) -> &str {
        &self.owner_user_id
    }

    pub fn runtime_key(&self) -> String {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        let digest = Sha256::digest(self.to_string().as_bytes());
        let mut encoded = String::with_capacity(64);
        for byte in digest {
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
        encoded
    }
}

fn validate_owner_user_id(value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 64
        || value.contains('@')
        || !value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
    {
        return Err("owner_user_id must be lowercase ASCII and contain only [a-z0-9._-]".into());
    }
    Ok(())
}

impl fmt::Display for AppInstanceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.app_id, self.owner_user_id)
    }
}

impl FromStr for AppInstanceId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let (app_id, owner_user_id) = value
            .trim()
            .rsplit_once('@')
            .ok_or_else(|| "AppInstanceId must be `{app_id}@{owner_user_id}`".to_string())?;
        Self::new(AppId::parse(app_id)?, owner_user_id)
    }
}

impl Serialize for AppInstanceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for AppInstanceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::from_str(&value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentId(String);

impl AgentId {
    pub fn from_agent_did(agent_did: &DID) -> Result<Self, String> {
        canonical_hostname_did(agent_did, "AgentDID").map(Self)
    }

    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let value = value.as_ref().trim();
        let did = parse_raw_hostname(value)?;
        let id = Self::from_agent_did(&did)?;
        if id.as_str() != value {
            return Err("AgentId is not a canonical AgentDID raw hostname".into());
        }
        Ok(id)
    }

    pub fn agent_did(&self) -> DID {
        parse_raw_hostname(&self.0).expect("validated AgentId must remain reversible")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for AgentId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for AgentId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for AgentId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SystemServiceId(String);

impl SystemServiceId {
    pub fn parse(value: impl AsRef<str>) -> Result<Self, String> {
        let value = value.as_ref();
        if value.starts_with("did:")
            || value.is_empty()
            || value.len() > 128
            || value.contains('@')
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(
                "SystemServiceId must contain only lowercase [a-z0-9-] and not start with `did:`"
                    .into(),
            );
        }
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SystemServiceId {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl Serialize for SystemServiceId {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SystemServiceId {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for SystemServiceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ServiceIdentity {
    App { app_id: AppId },
    System { service_id: SystemServiceId },
}

impl ServiceIdentity {
    pub fn parse(value: &str) -> Result<Self, String> {
        if value.starts_with("did:") {
            let did = DID::from_str(value).map_err(|error| error.to_string())?;
            Ok(Self::App {
                app_id: AppId::from_app_did(&did)?,
            })
        } else {
            Ok(Self::System {
                service_id: SystemServiceId::parse(value.to_string())?,
            })
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthTarget {
    App { app_instance_id: AppInstanceId },
    System { service_id: SystemServiceId },
}

impl AuthTarget {
    pub fn app(app_instance_id: AppInstanceId) -> Self {
        Self::App { app_instance_id }
    }

    pub fn system(service_id: SystemServiceId) -> Self {
        Self::System { service_id }
    }

    pub fn from_canonical_key(value: &str) -> Result<Self, String> {
        if let Some(app_instance_id) = value.strip_prefix("app:") {
            return Ok(Self::app(app_instance_id.parse()?));
        }
        if let Some(service_id) = value.strip_prefix("system:") {
            return Ok(Self::system(service_id.parse()?));
        }
        Err("AuthTarget key must be `app:<app_instance_id>` or `system:<service_id>`".into())
    }

    pub fn appid_claim(&self) -> &str {
        match self {
            Self::App { app_instance_id } => app_instance_id.app_id().as_str(),
            Self::System { service_id } => service_id.as_str(),
        }
    }

    pub fn canonical_id(&self) -> String {
        match self {
            Self::App { app_instance_id } => app_instance_id.to_string(),
            Self::System { service_id } => service_id.to_string(),
        }
    }

    pub fn canonical_key(&self) -> String {
        match self {
            Self::App { app_instance_id } => format!("app:{app_instance_id}"),
            Self::System { service_id } => format!("system:{service_id}"),
        }
    }

    pub fn authorization_identity(&self) -> ServiceIdentity {
        match self {
            Self::App { app_instance_id } => ServiceIdentity::App {
                app_id: app_instance_id.app_id().clone(),
            },
            Self::System { service_id } => ServiceIdentity::System {
                service_id: service_id.clone(),
            },
        }
    }

    pub fn authorization_key(&self) -> String {
        match self.authorization_identity() {
            ServiceIdentity::App { app_id } => format!("app:{app_id}"),
            ServiceIdentity::System { service_id } => format!("system:{service_id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_did_and_app_id_round_trip() {
        let web = DID::from_str("did:web:filebrowser.buckyos.ai").unwrap();
        let web_id = AppId::from_app_did(&web).unwrap();
        assert_eq!(web_id.as_str(), "filebrowser.buckyos.ai");
        assert_eq!(web_id.app_did(), web);

        let bns = DID::from_str("did:bns:filebrowser.buckyos").unwrap();
        let bns_id = AppId::from_app_did(&bns).unwrap();
        assert_eq!(bns_id.as_str(), "filebrowser.buckyos.bns.did");
        assert_eq!(bns_id.app_did(), bns);
    }

    #[test]
    fn app_did_profile_rejects_non_hostname_forms() {
        for value in [
            "did:web:Example.com",
            "did:web:example.com:path",
            "did:web:example.com%3A443",
            "did:web:example.did",
            "did:bns:bad_name",
        ] {
            let did = DID::from_str(value).unwrap();
            assert!(AppId::from_app_did(&did).is_err(), "accepted {value}");
        }
    }

    #[test]
    fn app_instance_id_is_canonical_and_owner_scoped() {
        let app_id = AppId::parse("filebrowser.buckyos.ai").unwrap();
        let alice = AppInstanceId::new(app_id.clone(), "alice").unwrap();
        let bob = AppInstanceId::new(app_id, "bob").unwrap();
        assert_eq!(alice.to_string(), "filebrowser.buckyos.ai@alice");
        assert_eq!(alice, alice.to_string().parse().unwrap());
        assert_ne!(alice, bob);
        assert_eq!(alice.runtime_key().len(), 64);
    }

    #[test]
    fn service_identity_classifies_before_did_parsing() {
        assert!(matches!(
            ServiceIdentity::parse("did:web:filebrowser.buckyos.ai").unwrap(),
            ServiceIdentity::App { .. }
        ));
        assert!(matches!(
            ServiceIdentity::parse("control-panel").unwrap(),
            ServiceIdentity::System { .. }
        ));
    }

    #[test]
    fn auth_target_round_trips_and_uses_kind_aware_keys() {
        let app = AuthTarget::app("filebrowser.buckyos.ai@alice".parse().unwrap());
        let app_json = serde_json::to_value(&app).unwrap();
        assert_eq!(serde_json::from_value::<AuthTarget>(app_json).unwrap(), app);
        assert_eq!(app.canonical_key(), "app:filebrowser.buckyos.ai@alice");
        assert_eq!(app.authorization_key(), "app:filebrowser.buckyos.ai");
        assert_eq!(
            AuthTarget::from_canonical_key(&app.canonical_key()).unwrap(),
            app
        );

        let system = AuthTarget::system("control-panel".parse().unwrap());
        let system_json = serde_json::to_value(&system).unwrap();
        assert_eq!(
            serde_json::from_value::<AuthTarget>(system_json).unwrap(),
            system
        );
        assert_eq!(system.canonical_key(), "system:control-panel");
        assert_eq!(system.authorization_key(), "system:control-panel");
        assert_eq!(
            AuthTarget::from_canonical_key(&system.canonical_key()).unwrap(),
            system
        );
    }

    #[test]
    fn auth_target_canonical_key_rejects_untyped_ids() {
        assert!(AuthTarget::from_canonical_key("control-panel").is_err());
        assert!(AuthTarget::from_canonical_key("app:control-panel").is_err());
        assert!(AuthTarget::from_canonical_key("system:filebrowser@alice").is_err());
    }

    #[test]
    fn auth_target_deserialization_rejects_unknown_or_mixed_fields() {
        for value in [
            serde_json::json!({"kind": "unknown", "service_id": "control-panel"}),
            serde_json::json!({"kind": "system", "service_id": ""}),
            serde_json::json!({"kind": "system", "service_id": "Control-Panel"}),
            serde_json::json!({"kind": "system", "service_id": "control-panel", "app_instance_id": "filebrowser.buckyos.ai@alice"}),
            serde_json::json!({"kind": "app", "app_instance_id": "filebrowser.buckyos.ai"}),
            serde_json::json!({"kind": "app", "app_instance_id": "filebrowser.buckyos.ai@alice", "service_id": "control-panel"}),
        ] {
            assert!(serde_json::from_value::<AuthTarget>(value).is_err());
        }
    }

    #[test]
    fn canonical_system_service_ids_parse() {
        assert_eq!(
            SystemServiceId::parse("control-panel").unwrap().as_str(),
            "control-panel"
        );
        assert_eq!(SystemServiceId::parse("kmsg").unwrap().as_str(), "kmsg");
    }
}
