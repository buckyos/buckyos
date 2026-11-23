use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct SmbUserItem {
    pub user: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Clone)]
pub struct SmbItem {
    pub smb_name: String,
    pub path: String,
    pub allow_users: Vec<String>,
}

/// The persisted user preference at `users/<uid>/samba/settings`.
#[derive(Serialize, Deserialize)]
#[serde(tag = "schema", deny_unknown_fields)]
pub enum UserSambaInfo {
    #[serde(rename = "buckyos.samba.settings.v1")]
    V1 { password: String, is_enable: bool },
}
