//! System user groups and contact projection (product/users_and_agents/UserType.md).
//!
//! A system user group is a label admins maintain in Users & Agents. It carries
//! no system RBAC policy; App availability rules, resource grants and contact
//! projection rules refer to it. Group definitions live at
//! `system/user_groups/<id>`, the projection rules at
//! `system/user_groups/_projections`, and membership in the user's settings
//! (`UserSettings.groups`). Scheduler adds `g, <user>, group:<id>` for every
//! defined group a user belongs to.
//!
//! A [`UserGroupRef`] names either a UserType group (`type:admin`,
//! `type:user`, `type:limited`) or a system user group (`group:<id>`).

use crate::{UserSettings, UserType};
use ::kRPC::RPCErrors;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::str::FromStr;

pub const USER_GROUPS_CONFIG_PREFIX: &str = "system/user_groups";
pub const CONTACT_PROJECTIONS_CONFIG_KEY: &str = "system/user_groups/_projections";
pub const USER_GROUP_REF_PREFIX: &str = "group:";
pub const USER_TYPE_GROUP_REF_PREFIX: &str = "type:";
pub const USER_GROUP_ID_MAX_CHARS: usize = 32;
pub const USER_GROUP_NAME_MAX_CHARS: usize = 64;

pub fn user_group_config_key(group_id: &str) -> String {
    format!("{USER_GROUPS_CONFIG_PREFIX}/{group_id}")
}

/// `[a-z][a-z0-9_-]*`, at most [`USER_GROUP_ID_MAX_CHARS`] characters.
pub fn validate_user_group_id(group_id: &str) -> Result<(), RPCErrors> {
    let valid = !group_id.is_empty()
        && group_id.chars().count() <= USER_GROUP_ID_MAX_CHARS
        && group_id.starts_with(|c: char| c.is_ascii_lowercase())
        && group_id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-');
    if valid {
        Ok(())
    } else {
        Err(RPCErrors::ParseRequestError(format!(
            "invalid user group id `{group_id}` (allowed: a-z first, then a-z, 0-9, _, -; at most {USER_GROUP_ID_MAX_CHARS})"
        )))
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct UserGroup {
    pub group_id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub updated_by: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum UserGroupRef {
    Type(UserType),
    Group(String),
}

impl UserGroupRef {
    pub fn group_id(&self) -> Option<&str> {
        match self {
            Self::Group(group_id) => Some(group_id),
            Self::Type(_) => None,
        }
    }
}

impl fmt::Display for UserGroupRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Type(user_type) => write!(f, "{USER_TYPE_GROUP_REF_PREFIX}{user_type}"),
            Self::Group(group_id) => write!(f, "{USER_GROUP_REF_PREFIX}{group_id}"),
        }
    }
}

impl FromStr for UserGroupRef {
    type Err = RPCErrors;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some(group_id) = value.strip_prefix(USER_GROUP_REF_PREFIX) {
            validate_user_group_id(group_id)?;
            return Ok(Self::Group(group_id.to_string()));
        }
        if let Some(user_type) = value
            .strip_prefix(USER_TYPE_GROUP_REF_PREFIX)
            .and_then(UserType::parse)
            .filter(|user_type| user_type.is_assignable())
        {
            return Ok(Self::Type(user_type));
        }
        Err(RPCErrors::ParseRequestError(format!(
            "invalid user group reference `{value}` (expected type:admin|user|limited or group:<id>)"
        )))
    }
}

impl Serialize for UserGroupRef {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for UserGroupRef {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

/// Members of `source` appear, read-only, in the address book of members of
/// `target`. One-way; mutual visibility takes two rules.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(deny_unknown_fields)]
pub struct ContactProjectionRule {
    pub source: UserGroupRef,
    pub target: UserGroupRef,
}

impl ContactProjectionRule {
    pub fn new(source: UserGroupRef, target: UserGroupRef) -> Self {
        Self { source, target }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Default)]
pub struct ContactProjectionConfig {
    pub rules: Vec<ContactProjectionRule>,
    #[serde(default)]
    pub updated_by: String,
    #[serde(default)]
    pub updated_at: u64,
}

/// Used while `system/user_groups/_projections` does not exist: Admin and User
/// accounts see each other, Limited accounts are seen by nobody.
pub fn default_contact_projection_rules() -> Vec<ContactProjectionRule> {
    let admin = UserGroupRef::Type(UserType::Admin);
    let user = UserGroupRef::Type(UserType::User);
    vec![
        ContactProjectionRule::new(admin.clone(), admin.clone()),
        ContactProjectionRule::new(admin.clone(), user.clone()),
        ContactProjectionRule::new(user.clone(), admin),
        ContactProjectionRule::new(user.clone(), user),
    ]
}

/// Rules may only name defined system groups and must not repeat.
pub fn validate_contact_projection_rules(
    rules: &[ContactProjectionRule],
    defined_groups: &HashSet<String>,
) -> Result<(), RPCErrors> {
    let mut seen = HashSet::new();
    for rule in rules {
        for group in [&rule.source, &rule.target] {
            if let Some(group_id) = group.group_id() {
                if !defined_groups.contains(group_id) {
                    return Err(RPCErrors::ParseRequestError(format!(
                        "projection rule refers to undefined group `{group}`"
                    )));
                }
            }
        }
        if !seen.insert(rule) {
            return Err(RPCErrors::ParseRequestError(format!(
                "duplicate projection rule {} -> {}",
                rule.source, rule.target
            )));
        }
    }
    Ok(())
}

/// The source groups whose members a viewer in `viewer_groups` sees.
pub fn projected_source_groups(
    rules: &[ContactProjectionRule],
    viewer_groups: &[UserGroupRef],
) -> BTreeSet<UserGroupRef> {
    rules
        .iter()
        .filter(|rule| viewer_groups.contains(&rule.target))
        .map(|rule| rule.source.clone())
        .collect()
}

/// For one viewer: every other active account it sees through the rules, with
/// the source groups that make it visible. Settings name groups by id; only
/// groups in `defined_groups` count.
pub fn project_contacts<'a>(
    rules: &[ContactProjectionRule],
    defined_groups: &HashSet<String>,
    viewer: &UserSettings,
    users: impl IntoIterator<Item = &'a UserSettings>,
) -> BTreeMap<String, BTreeSet<UserGroupRef>> {
    let effective_groups = |settings: &UserSettings| -> Vec<UserGroupRef> {
        settings
            .group_refs()
            .into_iter()
            .filter(|group| {
                group
                    .group_id()
                    .map_or(true, |group_id| defined_groups.contains(group_id))
            })
            .collect()
    };
    let mut projected = BTreeMap::new();
    if !viewer.is_active() {
        return projected;
    }
    let sources = projected_source_groups(rules, &effective_groups(viewer));
    if sources.is_empty() {
        return projected;
    }
    for user in users {
        if user.user_id == viewer.user_id || !user.is_active() {
            continue;
        }
        let via: BTreeSet<UserGroupRef> = effective_groups(user)
            .into_iter()
            .filter(|group| sources.contains(group))
            .collect();
        if !via.is_empty() {
            projected.insert(user.user_id.clone(), via);
        }
    }
    projected
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UserState;

    fn account(user_id: &str, user_type: UserType, groups: &[&str]) -> UserSettings {
        UserSettings {
            user_id: user_id.to_string(),
            user_type,
            password: "hash".to_string(),
            state: UserState::Active,
            res_pool_id: "default".to_string(),
            is_local: true,
            allow_password_change: None,
            allow_contact_edit: None,
            groups: groups.iter().map(|group| group.to_string()).collect(),
            password_state: crate::PasswordState::Set,
        }
    }

    #[test]
    fn group_refs_round_trip_and_reject_root_and_bad_ids() {
        for raw in [
            "type:admin",
            "type:user",
            "type:limited",
            "group:customer-1",
        ] {
            let parsed: UserGroupRef = raw.parse().unwrap();
            assert_eq!(parsed.to_string(), raw);
        }
        for raw in [
            "type:root",
            "type:guest",
            "group:",
            "group:Bad",
            "customer",
            "group:1x",
        ] {
            assert!(raw.parse::<UserGroupRef>().is_err(), "{raw}");
        }
        let rule: ContactProjectionRule =
            serde_json::from_str(r#"{"source":"group:customer","target":"group:service"}"#)
                .unwrap();
        assert_eq!(rule.source, UserGroupRef::Group("customer".into()));
    }

    #[test]
    fn default_rules_let_admins_and_users_see_each_other_but_not_limited() {
        let rules = default_contact_projection_rules();
        let groups = HashSet::new();
        let users = [
            account("alice", UserType::Admin, &[]),
            account("bob", UserType::User, &[]),
            account("carol", UserType::Limited, &[]),
        ];
        let seen = |viewer: &UserSettings| -> Vec<String> {
            project_contacts(&rules, &groups, viewer, users.iter())
                .into_keys()
                .collect()
        };
        assert_eq!(seen(&users[0]), vec!["bob"]);
        assert_eq!(seen(&users[1]), vec!["alice"]);
        assert!(seen(&users[2]).is_empty());
    }

    #[test]
    fn customer_to_service_rule_is_one_way_and_needs_defined_groups() {
        let rules = vec![ContactProjectionRule::new(
            UserGroupRef::Group("customer".into()),
            UserGroupRef::Group("service".into()),
        )];
        let users = [
            account("svc", UserType::User, &["service"]),
            account("cust", UserType::Limited, &["customer"]),
        ];
        let defined: HashSet<String> = ["customer", "service"].map(String::from).into();
        let service_view = project_contacts(&rules, &defined, &users[0], users.iter());
        assert_eq!(
            service_view.get("cust").unwrap().iter().collect::<Vec<_>>(),
            vec![&UserGroupRef::Group("customer".into())]
        );
        assert!(project_contacts(&rules, &defined, &users[1], users.iter()).is_empty());
        assert!(project_contacts(&rules, &HashSet::new(), &users[0], users.iter()).is_empty());
        assert!(validate_contact_projection_rules(&rules, &HashSet::new()).is_err());
        assert!(validate_contact_projection_rules(&rules, &defined).is_ok());
        let doubled = [rules[0].clone(), rules[0].clone()];
        assert!(validate_contact_projection_rules(&doubled, &defined).is_err());
    }

    #[test]
    fn four_dimension_defaults_follow_the_user_type() {
        let limited = account("carol", UserType::Limited, &[]);
        assert!(
            limited.is_active() && !limited.can_change_password() && !limited.can_edit_contacts()
        );
        let mut opened = limited.clone();
        opened.allow_password_change = Some(true);
        opened.allow_contact_edit = Some(true);
        assert!(opened.can_change_password() && opened.can_edit_contacts());
        let user = account("bob", UserType::User, &[]);
        assert!(user.can_change_password() && user.can_edit_contacts());
        let mut root = account("root", UserType::Root, &["customer"]);
        assert!(!root.is_active() && root.group_refs().is_empty());
        root.user_type = UserType::Admin;
        assert!(root.is_active());
        assert!(!UserType::Limited.can_federate_out() && UserType::User.can_federate_out());
    }
}
