//! `contact.lookup`: find people and groups for whatever the user typed. There
//! is no network directory; results come from the owner's own relations, this
//! Zone, and the name service resolving the input itself (`Contact Mgr.md`).

use crate::group_types::{GroupRole, GroupState, MemberStatus};
use crate::msg_center::MessageCenter;
use crate::network_directory::DirectoryError;
use buckyos_api::{
    contact_tag_kind, Contact, ContactLookupReq, ContactQuery, ContactTagKind, LookupCandidate,
    LookupError, LookupGroupInfo, LookupInputKind, LookupKind, LookupRelation, LookupResult,
    LookupSection, Reachability, ReachabilityState, CONTACT_TAG_ZONE_AGENT,
};
use kRPC::RPCErrors;
use name_lib::DID;
use serde_json::Value;

type Result<T> = std::result::Result<T, RPCErrors>;

const DEFAULT_LOOKUP_LIMIT: usize = 20;
const MAX_LOOKUP_LIMIT: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ParsedInput {
    Empty,
    Text,
    Did(DID),
    Name(DID),
    Invite(DID, String),
}

fn lookup_error(code: &str, message: impl Into<String>) -> LookupError {
    LookupError {
        code: code.to_string(),
        message: Some(message.into()),
    }
}

fn is_name_like(input: &str) -> bool {
    !input.starts_with('.')
        && !input.ends_with('.')
        && input
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
}

/// `<group_did>?invite=<token>`, a URL carrying `group`/`group_did` and
/// `invite`, a DID, a BNS or domain name, or free text.
pub(crate) fn parse_lookup_input(raw: &str) -> std::result::Result<ParsedInput, LookupError> {
    let input = raw.trim();
    if input.is_empty() {
        return Ok(ParsedInput::Empty);
    }
    if let Some((head, query)) = input.split_once('?') {
        let params: Vec<(String, String)> = reqwest::Url::parse(&format!("http://q/?{query}"))
            .map(|url| url.query_pairs().into_owned().collect())
            .unwrap_or_default();
        let param = |key: &str| {
            params
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let group = param("group_did")
            .or_else(|| param("group"))
            .or_else(|| head.starts_with("did:").then(|| head.to_string()));
        return match (group, param("invite")) {
            (Some(group), Some(invite)) => DID::from_friendly_name(&group)
                .map(|did| ParsedInput::Invite(did, invite))
                .map_err(|_| lookup_error("invalid-input", "invalid group DID")),
            _ => Err(lookup_error("invalid-input", "not an invite link")),
        };
    }
    if input.starts_with("did:") {
        return DID::from_friendly_name(input)
            .map(ParsedInput::Did)
            .map_err(|error| lookup_error("invalid-input", error.to_string()));
    }
    if is_name_like(input) {
        if let Ok(did) = DID::from_friendly_name(input) {
            return Ok(ParsedInput::Name(did));
        }
    }
    Ok(ParsedInput::Text)
}

fn member_status_name(status: &MemberStatus) -> Option<String> {
    serde_json::to_value(status)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

fn role_name(role: &GroupRole) -> Option<String> {
    serde_json::to_value(role)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
}

fn profile_text(profile: &Value, key: &str) -> Option<String> {
    profile
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

impl MessageCenter {
    fn contact_kind(contact: &Contact) -> LookupKind {
        if contact.did.method == "msgtunnel" {
            LookupKind::Msgtunnel
        } else if contact.tags.iter().any(|t| t == CONTACT_TAG_ZONE_AGENT) {
            LookupKind::Agent
        } else if contact.did.method == "dev" {
            LookupKind::Device
        } else {
            LookupKind::User
        }
    }

    fn cheap_reachability(&self, did: &DID) -> Reachability {
        let state = if did.method == "msgtunnel" {
            ReachabilityState::Tunnel
        } else if self.is_local_recipient(did) {
            ReachabilityState::Local
        } else {
            ReachabilityState::Unknown
        };
        Reachability {
            state,
            zone: None,
            reason: None,
        }
    }

    /// Where native delivery to `did` would go.
    pub(crate) async fn reachability(&self, did: &DID) -> Reachability {
        let cheap = self.cheap_reachability(did);
        if cheap.state != ReachabilityState::Unknown {
            return cheap;
        }
        match self.directory.get().zone_of(did).await {
            Ok(Some(zone)) => Reachability {
                state: ReachabilityState::Routable,
                zone: Some(zone),
                reason: None,
            },
            Ok(None) => Reachability {
                state: ReachabilityState::Unreachable,
                zone: None,
                reason: Some("no-zone".into()),
            },
            Err(DirectoryError::NotFound(_)) => Reachability {
                state: ReachabilityState::Unreachable,
                zone: None,
                reason: Some("not-found".into()),
            },
            Err(DirectoryError::Unavailable(_)) => Reachability {
                state: ReachabilityState::Unreachable,
                zone: None,
                reason: Some("resolve-failed".into()),
            },
        }
    }

    async fn relation_of(&self, owner: &DID, contact: Option<&Contact>, did: &DID) -> Result<LookupRelation> {
        let now = Self::now_ms();
        let pending = self.pending_friend_request(owner, did).await?;
        Ok(match contact {
            Some(contact) => LookupRelation {
                is_contact: true,
                friend_state: contact.friend_state,
                access_level: Some(contact.access_level.clone()),
                temporary_expires_at_ms: contact
                    .temp_grants
                    .iter()
                    .filter(|grant| grant.expires_at > now)
                    .map(|grant| grant.expires_at)
                    .max(),
                tags: contact.tags.clone(),
                pending_request_id: pending.map(|r| r.request_id),
                ..Default::default()
            },
            None => LookupRelation {
                pending_request_id: pending.map(|r| r.request_id),
                ..Default::default()
            },
        })
    }

    async fn contact_candidate(&self, owner: &DID, contact: &Contact) -> Result<LookupCandidate> {
        let section = if contact
            .tags
            .iter()
            .any(|tag| contact_tag_kind(tag) == ContactTagKind::System)
        {
            LookupSection::Zone
        } else {
            LookupSection::Relation
        };
        Ok(LookupCandidate {
            did: contact.did.clone(),
            kind: Self::contact_kind(contact),
            section,
            name: Some(contact.name.clone()).filter(|n| !n.trim().is_empty()),
            avatar: contact.avatar.clone(),
            note: contact.note.clone(),
            reachability: self.cheap_reachability(&contact.did),
            relation: self.relation_of(owner, Some(contact), &contact.did).await?,
            group: None,
        })
    }

    fn hosted_group_candidate(
        owner: &DID,
        group: &GroupState,
        section: LookupSection,
        invite: Option<String>,
    ) -> LookupCandidate {
        let config = group.config().ok();
        let profile = config.as_ref().map(|c| c.profile.clone()).unwrap_or_default();
        let member = group.members.get(&owner.to_string());
        let active = |m: &&crate::group_types::MemberRecord| m.state == MemberStatus::Active;
        let visible = config.as_ref().is_some_and(|c| {
            c.membership.member_list_visibility == "public"
                || (c.membership.member_list_visibility == "members" && member.filter(active).is_some())
        }) || group.owner == *owner;
        LookupCandidate {
            did: group.group_did.clone(),
            kind: LookupKind::Group,
            section,
            name: profile_text(&profile, "name"),
            avatar: profile_text(&profile, "avatar"),
            note: profile_text(&profile, "description"),
            reachability: Reachability {
                state: ReachabilityState::Local,
                zone: None,
                reason: None,
            },
            relation: LookupRelation {
                group_state: member.and_then(|m| member_status_name(&m.state)),
                group_role: member.filter(active).and_then(|m| role_name(&m.role)),
                ..Default::default()
            },
            group: Some(LookupGroupInfo {
                join_policy: config.as_ref().map(|c| c.membership.join_policy.clone()),
                invite,
                hosted_here: true,
                host: Some(group.host.to_string()),
                owner: Some(group.owner.clone()),
                member_count: visible.then(|| {
                    group
                        .members
                        .values()
                        .filter(|m| m.state == MemberStatus::Active)
                        .count() as u64
                }),
            }),
        }
    }

    /// Groups the owner belongs to: hosted here, or joined in another Zone.
    async fn relation_groups(&self, owner: &DID, keyword: Option<&str>) -> Result<Vec<LookupCandidate>> {
        let keyword = keyword.map(str::to_lowercase);
        let matches = |did: &DID, name: Option<&str>| {
            keyword.as_ref().is_none_or(|k| {
                did.to_string().to_lowercase().contains(k)
                    || name.is_some_and(|n| n.to_lowercase().contains(k))
            })
        };
        let mut out = Vec::new();
        for group in self.groups.list().await? {
            if group.lifecycle == "deleted" {
                continue;
            }
            let Some(member) = group.members.get(&owner.to_string()) else {
                continue;
            };
            if !matches!(
                member.state,
                MemberStatus::Active | MemberStatus::Invited | MemberStatus::PendingAdminApproval
            ) {
                continue;
            }
            let candidate = Self::hosted_group_candidate(owner, &group, LookupSection::Relation, None);
            if matches(&candidate.did, candidate.name.as_deref()) {
                out.push(candidate);
            }
        }
        for joined in self.joined_groups(owner).await? {
            if out.iter().any(|c| c.did == joined.group_did) || !matches(&joined.group_did, None) {
                continue;
            }
            out.push(LookupCandidate {
                did: joined.group_did.clone(),
                kind: LookupKind::Group,
                section: LookupSection::Relation,
                name: None,
                avatar: None,
                note: None,
                reachability: Reachability {
                    state: ReachabilityState::Routable,
                    zone: None,
                    reason: None,
                },
                relation: LookupRelation {
                    group_state: Some("active".into()),
                    ..Default::default()
                },
                group: Some(LookupGroupInfo {
                    hosted_here: false,
                    ..Default::default()
                }),
            });
        }
        Ok(out)
    }

    /// The input itself as one object: a contact, a group hosted here, a DID
    /// of this Zone, or whatever the name service says.
    async fn resolve_input(
        &self,
        owner: &DID,
        did: &DID,
        invite: Option<String>,
    ) -> Result<std::result::Result<LookupCandidate, LookupError>> {
        if let Some(group) = self.groups.load(did).await?.filter(|g| g.lifecycle != "deleted") {
            return Ok(Ok(Self::hosted_group_candidate(
                owner,
                &group,
                LookupSection::Resolved,
                invite,
            )));
        }
        if let Some(contact) = self
            .contact_mgr
            .get_contact(
                self.contact_mgr
                    .resolve_canonical_did(did.clone(), Some(owner.clone()))
                    .await?,
                Some(owner.clone()),
            )
            .await?
        {
            let mut candidate = self.contact_candidate(owner, &contact).await?;
            if candidate.reachability.state == ReachabilityState::Unknown {
                candidate.reachability = self.reachability(&contact.did).await;
            }
            return Ok(Ok(candidate));
        }
        let profile = self.directory.get().describe(did).await;
        if self.is_local_recipient(did) {
            let agent = self
                .token_verifier
                .get()
                .is_zone_agent(did)
                .await
                .unwrap_or(false);
            let profile = profile.ok();
            return Ok(Ok(LookupCandidate {
                did: did.clone(),
                kind: if agent { LookupKind::Agent } else { profile.as_ref().map(|p| p.kind).unwrap_or(LookupKind::User) },
                section: LookupSection::Resolved,
                name: profile.as_ref().and_then(|p| p.name.clone()),
                avatar: profile.as_ref().and_then(|p| p.avatar.clone()),
                note: None,
                reachability: self.cheap_reachability(did),
                relation: self.relation_of(owner, None, did).await?,
                group: None,
            }));
        }
        let profile = match profile {
            Ok(profile) => profile,
            Err(DirectoryError::NotFound(reason)) => return Ok(Err(lookup_error("not-found", reason))),
            Err(DirectoryError::Unavailable(reason)) => {
                return Ok(Err(lookup_error("unreachable", reason)))
            }
        };
        let reachability = match profile.zone.clone() {
            Some(zone) => Reachability {
                state: ReachabilityState::Routable,
                zone: Some(zone),
                reason: None,
            },
            None => Reachability {
                state: ReachabilityState::Unreachable,
                zone: None,
                reason: Some("no-zone".into()),
            },
        };
        Ok(Ok(LookupCandidate {
            did: did.clone(),
            kind: profile.kind,
            section: LookupSection::Resolved,
            name: profile.name,
            avatar: profile.avatar,
            note: None,
            reachability: reachability.clone(),
            relation: self.relation_of(owner, None, did).await?,
            group: (profile.kind == LookupKind::Group).then(|| LookupGroupInfo {
                invite,
                hosted_here: false,
                host: reachability.zone,
                ..Default::default()
            }),
        }))
    }

    pub(crate) async fn contact_lookup(&self, owner: &DID, req: ContactLookupReq) -> Result<LookupResult> {
        let limit = req
            .limit
            .unwrap_or(DEFAULT_LOOKUP_LIMIT)
            .clamp(1, MAX_LOOKUP_LIMIT);
        let input = req.input.trim().to_string();
        let parsed = parse_lookup_input(&input);
        let input_kind = match &parsed {
            Ok(ParsedInput::Empty) => LookupInputKind::Empty,
            Ok(ParsedInput::Text) | Err(_) => LookupInputKind::Text,
            Ok(ParsedInput::Did(_)) => LookupInputKind::Did,
            Ok(ParsedInput::Name(_)) => LookupInputKind::Name,
            Ok(ParsedInput::Invite(..)) => LookupInputKind::InviteLink,
        };
        let mut candidates: Vec<LookupCandidate> = Vec::new();
        let mut error = None;

        let keyword = match &parsed {
            Ok(ParsedInput::Empty) => Some(None),
            Ok(ParsedInput::Text) | Ok(ParsedInput::Name(_)) => Some(Some(input.clone())),
            _ => None,
        };
        if let Some(keyword) = keyword {
            let contacts = self
                .contact_mgr
                .list_contacts(
                    ContactQuery {
                        keyword: keyword.clone(),
                        limit: Some(MAX_LOOKUP_LIMIT),
                        ..Default::default()
                    },
                    Some(owner.clone()),
                )
                .await?;
            for contact in contacts.iter().filter(|c| &c.did != owner) {
                candidates.push(self.contact_candidate(owner, contact).await?);
            }
            candidates.extend(self.relation_groups(owner, keyword.as_deref()).await?);
        }

        // A bare name that already names someone the owner knows is that
        // person; only a full `did:` asks the name service regardless.
        let known_name = |candidates: &[LookupCandidate]| {
            let wanted = input.to_lowercase();
            candidates.iter().any(|c| {
                c.name.as_deref().is_some_and(|n| n.to_lowercase() == wanted)
                    || c.did.id.to_lowercase() == wanted
                    || c.did.id.to_lowercase().starts_with(&format!("{wanted}."))
            })
        };
        let target = match &parsed {
            Ok(ParsedInput::Name(_)) if known_name(&candidates) => None,
            Ok(ParsedInput::Did(did)) | Ok(ParsedInput::Name(did)) => Some((did.clone(), None)),
            Ok(ParsedInput::Invite(did, invite)) => Some((did.clone(), Some(invite.clone()))),
            Err(e) => {
                error = Some(e.clone());
                None
            }
            _ => None,
        };
        if let Some((did, invite)) = target {
            match self.resolve_input(owner, &did, invite).await? {
                Ok(resolved) => {
                    candidates.retain(|c| c.did != resolved.did);
                    candidates.insert(0, resolved);
                }
                Err(e) => {
                    // A name that also matched local people is reported only
                    // when nothing else was found.
                    let named = matches!(parsed, Ok(ParsedInput::Name(_)));
                    if !(named && !candidates.is_empty()) {
                        error = Some(e);
                    }
                }
            }
        }

        if let Some(kinds) = req.kinds.as_ref().filter(|k| !k.is_empty()) {
            candidates.retain(|c| kinds.contains(&c.kind));
        }
        let rank = |section: LookupSection| match section {
            LookupSection::Resolved => 0,
            LookupSection::Relation => 1,
            LookupSection::Zone => 2,
        };
        candidates.sort_by_key(|c| rank(c.section));
        let mut seen = std::collections::HashSet::new();
        candidates.retain(|c| seen.insert(c.did.to_string()));
        candidates.truncate(limit);
        Ok(LookupResult {
            input,
            input_kind,
            candidates,
            error,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inputs_are_classified() {
        assert_eq!(parse_lookup_input("  ").unwrap(), ParsedInput::Empty);
        assert_eq!(parse_lookup_input("Bob Chen").unwrap(), ParsedInput::Text);
        assert_eq!(
            parse_lookup_input("bob").unwrap(),
            ParsedInput::Name(DID::new("bns", "bob"))
        );
        assert_eq!(
            parse_lookup_input("alice.example.com").unwrap(),
            ParsedInput::Name(DID::new("web", "alice.example.com"))
        );
        assert_eq!(
            parse_lookup_input("did:web:g.example.com?invite=t1").unwrap(),
            ParsedInput::Invite(DID::new("web", "g.example.com"), "t1".into())
        );
        assert_eq!(
            parse_lookup_input("https://zone.example/messagehub?group=did:web:g.zone.example&invite=t2")
                .unwrap(),
            ParsedInput::Invite(DID::new("web", "g.zone.example"), "t2".into())
        );
        assert_eq!(parse_lookup_input("did:x").unwrap_err().code, "invalid-input");
        assert_eq!(parse_lookup_input("a?b=1").unwrap_err().code, "invalid-input");
    }
}
