//! Publication audience (§4.5): an access policy of the entry, never written into objects.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AudienceSpec {
    Public,
    Followers,
    Friends,
    Group {
        #[serde(rename = "groupId")]
        group_id: String,
    },
    Dids {
        dids: Vec<String>,
    },
}

impl AudienceSpec {
    pub fn is_public(&self) -> bool {
        matches!(self, Self::Public)
    }

    pub fn only(did: &str) -> Self {
        Self::Dids { dids: vec![did.to_string()] }
    }

    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::Group { group_id } if group_id.trim().is_empty() => Err("group audience needs a group".into()),
            Self::Dids { dids } if dids.is_empty() || dids.len() > 20 => Err("1 to 20 DIDs".into()),
            Self::Dids { dids } if dids.iter().any(|d| !d.starts_with("did:")) => Err("audience DIDs must be DIDs".into()),
            _ => Ok(()),
        }
    }

    /// The level shown to others: restricted entries only reveal the tier (§4.5).
    pub fn tier(&self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Followers => "followers",
            Self::Friends => "friends",
            Self::Group { .. } => "group",
            Self::Dids { .. } => "dids",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reader {
    Owner,
    Anonymous,
    Did(String),
}

impl Reader {
    pub fn did(&self) -> Option<&str> {
        match self {
            Self::Did(did) => Some(did),
            _ => None,
        }
    }
}

/// What the publisher knows about a reader at read time.
#[derive(Debug, Clone, Default)]
pub struct ReaderRelations {
    pub follower: bool,
    pub friend: bool,
    pub groups: Vec<String>,
}

pub fn allows(spec: &AudienceSpec, reader: &Reader, relations: &ReaderRelations) -> bool {
    match reader {
        Reader::Owner => true,
        Reader::Anonymous => spec.is_public(),
        Reader::Did(did) => match spec {
            AudienceSpec::Public => true,
            AudienceSpec::Followers => relations.follower || relations.friend,
            AudienceSpec::Friends => relations.friend,
            AudienceSpec::Group { group_id } => relations.groups.iter().any(|g| g == group_id),
            AudienceSpec::Dids { dids } => dids.iter().any(|d| d == did),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audience_matrix() {
        let bob = Reader::Did("did:bns:bob".into());
        let stranger = ReaderRelations::default();
        let friend = ReaderRelations { friend: true, groups: vec!["hiking".into()], ..Default::default() };
        assert!(allows(&AudienceSpec::Public, &Reader::Anonymous, &stranger));
        assert!(!allows(&AudienceSpec::Followers, &Reader::Anonymous, &stranger));
        assert!(!allows(&AudienceSpec::Friends, &bob, &stranger));
        assert!(allows(&AudienceSpec::Friends, &bob, &friend));
        assert!(allows(&AudienceSpec::Followers, &bob, &friend));
        assert!(allows(&AudienceSpec::Group { group_id: "hiking".into() }, &bob, &friend));
        assert!(!allows(&AudienceSpec::Group { group_id: "work".into() }, &bob, &friend));
        assert!(allows(&AudienceSpec::only("did:bns:bob"), &bob, &stranger));
        assert!(allows(&AudienceSpec::Friends, &Reader::Owner, &stranger));
        let json = serde_json::to_value(AudienceSpec::Group { group_id: "g".into() }).unwrap();
        assert_eq!(json, serde_json::json!({ "kind": "group", "groupId": "g" }));
    }
}
