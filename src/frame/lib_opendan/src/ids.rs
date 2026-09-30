//! Global session ids (§4.2) and naming helpers.
//!
//! A session id is globally unique across agents, apps and owners. Random ids
//! keep the full UUID; idempotent / singleton ids hash a fixed-order JSON
//! array (`H`) and keep the full SHA-256 hex.

use sha2::{Digest, Sha256};

use crate::error::{OpenDanError, Result};
use crate::protocol::SessionKind;

pub const MAX_SID_LEN: usize = 200;

/// `H(args)`: SHA-256 of the compact JSON encoding of the argument array,
/// lowercase hex, never truncated.
pub fn h(args: &[&str]) -> String {
    let json = serde_json::to_string(args).expect("string array serializes");
    let digest = Sha256::digest(json.as_bytes());
    hex::encode(digest)
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string()
}

fn random_part() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

/// Derive a session id following §4.2.
///
/// - `ui`: `ui-H("ui", agent_did, route_key)`
/// - `work`: random, or `work-H("work", agent_did, creator, key)`
/// - `self_improve`: random, or `si-H("self_improve", agent_did, creator, key)`
/// - `self_check`: `sc-H("self_check", agent_did)`
pub fn derive_session_id(
    kind: SessionKind,
    agent_did: &str,
    creator_principal: &str,
    idempotency_key: Option<&str>,
    route_key: Option<&str>,
) -> Result<String> {
    let prefix = kind.id_prefix();
    Ok(match kind {
        SessionKind::Ui => {
            let route = route_key.ok_or_else(|| {
                OpenDanError::InvalidArgument("ui session requires a route_key".into())
            })?;
            format!("{prefix}-{}", h(&["ui", agent_did, route]))
        }
        SessionKind::SelfCheck => format!("{prefix}-{}", h(&["self_check", agent_did])),
        SessionKind::Work | SessionKind::SelfImprove => match idempotency_key {
            Some(key) => format!(
                "{prefix}-{}",
                h(&[kind.as_str(), agent_did, creator_principal, key])
            ),
            None => format!("{prefix}-{}-{}", stamp(), random_part()),
        },
    })
}

/// Characters allowed in ids that become kevent segments / kmsg names.
pub fn is_id_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.')
}

/// Validate an explicit session id: charset, length, no leading dot.
pub fn validate_session_id(sid: &str) -> Result<()> {
    if sid.is_empty() || sid.len() > MAX_SID_LEN {
        return Err(OpenDanError::InvalidArgument(format!(
            "session id must be 1..={MAX_SID_LEN} characters"
        )));
    }
    if sid.starts_with('.') || !sid.chars().all(is_id_char) {
        return Err(OpenDanError::InvalidArgument(format!(
            "session id `{sid}` may only use letters, digits and `_ - .` and must not start with `.`"
        )));
    }
    Ok(())
}

/// Sanitize any string into one id segment (letters, digits, `_ - .`).
pub fn sanitize_segment(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| if is_id_char(c) { c } else { '_' })
        .collect();
    let out = out.trim_matches('.').to_string();
    if out.is_empty() {
        "_".to_string()
    } else {
        out
    }
}

/// Agent id used in kevent / kmsg names, derived from the agent DID when the
/// caller does not configure one: `did:bns:jarvis.alice` → `jarvis.alice`.
pub fn agent_id_from_did(did: &str) -> String {
    let last = did.rsplit(':').next().unwrap_or(did);
    sanitize_segment(last)
}

/// Split `app:<appid>@<owner>` (A9). Returns `(appid, owner)`.
pub fn parse_app_principal(p: &str) -> Option<(String, String)> {
    let rest = p.strip_prefix("app:")?;
    let (app, owner) = rest.rsplit_once('@')?;
    if app.is_empty() || owner.is_empty() {
        return None;
    }
    Some((app.to_string(), owner.to_string()))
}

pub fn app_principal(appid: &str, owner: &str) -> String {
    format!("app:{appid}@{owner}")
}

/// kmsg queue name of a session (`opendan.session.<sid>`).
pub fn queue_name(sid: &str) -> String {
    format!("opendan.session.{sid}")
}

/// kmsg subscriber id (sub ids share one namespace across queues and apps).
pub fn subscriber_id(agent_id: &str, sid: &str) -> String {
    format!("opendan.{}.{sid}", sanitize_segment(agent_id))
}

/// kevent id published after posting to the session queue.
pub fn wake_event(agent_id: &str, sid: &str) -> String {
    format!("/opendan/{}/session/{sid}/input", sanitize_segment(agent_id))
}

/// Unique id of a runner instance.
pub fn new_runner_id() -> String {
    format!("rn-{}", &random_part()[..16])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_ids_keep_full_uuid() {
        let a = derive_session_id(SessionKind::Work, "did:bns:a", "app:x@o", None, None).unwrap();
        let b = derive_session_id(SessionKind::Work, "did:bns:a", "app:x@o", None, None).unwrap();
        assert_ne!(a, b);
        let parts: Vec<&str> = a.split('-').collect();
        assert_eq!(parts[0], "work");
        assert_eq!(parts[1].len(), 15); // yyyymmddThhmmss
        assert_eq!(parts[2].len(), 32);
        validate_session_id(&a).unwrap();
    }

    #[test]
    fn idempotent_ids_are_namespaced() {
        let k = Some("req-1");
        let a1 = derive_session_id(SessionKind::Work, "did:bns:a", "app:x@o", k, None).unwrap();
        let a2 = derive_session_id(SessionKind::Work, "did:bns:a", "app:x@o", k, None).unwrap();
        assert_eq!(a1, a2);
        let other_agent =
            derive_session_id(SessionKind::Work, "did:bns:b", "app:x@o", k, None).unwrap();
        let other_creator =
            derive_session_id(SessionKind::Work, "did:bns:a", "app:y@o", k, None).unwrap();
        let other_owner =
            derive_session_id(SessionKind::Work, "did:bns:a", "app:x@o2", k, None).unwrap();
        assert_ne!(a1, other_agent);
        assert_ne!(a1, other_creator);
        assert_ne!(a1, other_owner);
        assert_eq!(a1.len(), "work-".len() + 64);
        let si = derive_session_id(SessionKind::SelfImprove, "did:bns:a", "app:x@o", k, None)
            .unwrap();
        assert!(si.starts_with("si-"));
        assert_ne!(si[3..], a1[5..]);
    }

    #[test]
    fn deterministic_ids_include_agent() {
        let u1 = derive_session_id(SessionKind::Ui, "did:bns:a", "", None, Some("tg:1")).unwrap();
        let u2 = derive_session_id(SessionKind::Ui, "did:bns:b", "", None, Some("tg:1")).unwrap();
        assert_ne!(u1, u2);
        let c1 = derive_session_id(SessionKind::SelfCheck, "did:bns:a", "", None, None).unwrap();
        let c2 = derive_session_id(SessionKind::SelfCheck, "did:bns:b", "", None, None).unwrap();
        assert_ne!(c1, c2);
        // Same agent name under two owners → different DIDs → different ids.
        let o1 =
            derive_session_id(SessionKind::SelfCheck, "did:bns:jarvis.alice", "", None, None)
                .unwrap();
        let o2 = derive_session_id(SessionKind::SelfCheck, "did:bns:jarvis.bob", "", None, None)
            .unwrap();
        assert_ne!(o1, o2);
    }

    #[test]
    fn h_uses_json_array_encoding() {
        // No bare concatenation: ("ab","c") and ("a","bc") differ.
        assert_ne!(h(&["ab", "c"]), h(&["a", "bc"]));
        assert_eq!(h(&["x"]).len(), 64);
    }

    #[test]
    fn validation_and_names() {
        assert!(validate_session_id("work-1.2_3").is_ok());
        assert!(validate_session_id("bad/sid").is_err());
        assert!(validate_session_id(".hidden").is_err());
        assert_eq!(agent_id_from_did("did:bns:jarvis.alice"), "jarvis.alice");
        assert_eq!(
            parse_app_principal("app:app2@alice"),
            Some(("app2".into(), "alice".into()))
        );
        assert_eq!(wake_event("jarvis", "work-x"), "/opendan/jarvis/session/work-x/input");
    }
}
