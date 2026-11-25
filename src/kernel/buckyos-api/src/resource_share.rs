use crate::{has_permission, invite_secret_digest, AuthTarget, PermissionItem};
use serde::{Deserialize, Serialize};

pub const RESOURCE_SHARES_PREFIX: &str = "services/control_panel/resource_shares";
pub const RESOURCE_SHARE_ACCEPTANCES_PREFIX: &str =
    "services/control_panel/resource_share_acceptances";
pub const SHARE_DIRECTED_PERMISSION: &str = "sso/share/directed";
pub const SHARE_OPEN_PERMISSION: &str = "sso/share/open";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ShareLevel {
    #[default]
    Disabled,
    Directed,
    Open,
}

impl ShareLevel {
    pub fn from_permissions(permissions: &[PermissionItem], now: u64) -> Self {
        let has = |name| has_permission(permissions, name, now);
        if !has(SHARE_DIRECTED_PERMISSION) {
            return Self::Disabled;
        }
        if has(SHARE_OPEN_PERMISSION) {
            Self::Open
        } else {
            Self::Directed
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ShareAudience {
    Directed { did: String },
    Open,
}

impl ShareAudience {
    pub fn level(&self) -> ShareLevel {
        match self {
            Self::Directed { .. } => ShareLevel::Directed,
            Self::Open => ShareLevel::Open,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceShare {
    pub invite_id: String,
    pub issuer: String,
    pub target: AuthTarget,
    pub target_url: String,
    pub resource_id: String,
    pub sharer: String,
    pub title: String,
    pub operations: Vec<String>,
    pub methods: crate::EntryMethods,
    pub audience: ShareAudience,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ResourceShareAcceptance {
    pub invite_id: String,
    pub did: String,
    pub target: AuthTarget,
    pub accepted_at: u64,
}

pub fn valid_resource_share_id(id: &str) -> bool {
    id.strip_prefix("inv-").is_some_and(|s| {
        s.len() == 32
            && s.bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    })
}

pub fn resource_share_key(id: &str) -> String {
    format!("{RESOURCE_SHARES_PREFIX}/{id}")
}

pub fn resource_share_acceptance_key(id: &str, did: &str) -> String {
    format!(
        "{RESOURCE_SHARE_ACCEPTANCES_PREFIX}/{id}/{}",
        invite_secret_digest(did)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn permission(scope: &str, exp: Option<u32>) -> PermissionItem {
        PermissionItem {
            scope_path: scope.into(),
            actions: vec![],
            required: false,
            exp,
        }
    }
    #[test]
    fn current_grant_and_declaration_both_bound_invitations() {
        let directed = permission(SHARE_DIRECTED_PERMISSION, None);
        let open = permission(SHARE_OPEN_PERMISSION, None);
        assert_eq!(
            ShareLevel::from_permissions(&[open.clone()], 20),
            ShareLevel::Disabled
        );
        assert_eq!(
            ShareLevel::from_permissions(&[directed.clone()], 20),
            ShareLevel::Directed
        );
        assert_eq!(
            ShareLevel::from_permissions(&[directed.clone(), open], 20),
            ShareLevel::Open
        );
        assert_eq!(
            ShareLevel::from_permissions(
                &[directed, permission(SHARE_OPEN_PERMISSION, Some(20))],
                20
            ),
            ShareLevel::Directed
        );
    }
}
