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
