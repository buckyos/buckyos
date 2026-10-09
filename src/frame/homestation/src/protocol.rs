//! Cross-node objects of the HomeStation architecture (v0.6 §5, §16.5, E01–E17) and the
//! rules every receiver applies before trusting them. Wire names follow the architecture
//! examples; implementation choices for the items left open in §21 are listed in
//! `doc/homestation/HomeStation 协议与实现.md`.

use ndn_lib::{build_named_object_by_json, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

pub const OBJ_TYPE_FEED: &str = "cyfeed";
pub const OBJ_TYPE_HEAD: &str = "cyfhead";
pub const OBJ_TYPE_FOLLOW: &str = "cyfollow";
pub const OBJ_TYPE_EVALUATION: &str = "cyfeval";
pub const OBJ_TYPE_CONSUMPTION: &str = "cyfproof";
pub const OBJ_TYPE_FILE: &str = ndn_lib::OBJ_TYPE_FILE;

/// Largest dispatched object body (the CYFS named-inbox default).
pub const MAX_OBJECT_BYTES: usize = 65536;
/// Largest inline text of a self-contained Feed Object (§5.5); longer bodies are wrapped files.
pub const MAX_INLINE_TEXT: usize = 8000;
pub const MAX_MEDIA_PARTS: usize = 9;
pub const MAX_TAGS: usize = 16;

pub const NS_FEED: &str = "feed";
pub const NS_REACTIONS: &str = "reactions";
pub const NS_FOLLOWS: &str = "follows";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum FeedKind {
    Post,
    Comment,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum CommentType {
    Text,
    Like,
    Dislike,
    Bookmark,
    Repost,
    Quote,
}

impl CommentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Like => "like",
            Self::Dislike => "dislike",
            Self::Bookmark => "bookmark",
            Self::Repost => "repost",
            Self::Quote => "quote",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "text" => Self::Text,
            "like" => Self::Like,
            "dislike" => Self::Dislike,
            "bookmark" => Self::Bookmark,
            "repost" => Self::Repost,
            "quote" => Self::Quote,
            _ => return None,
        })
    }
    /// Switch-like interactions: one logical state per (publisher, target, type) (§12.4).
    pub fn is_toggle(self) -> bool {
        matches!(self, Self::Like | Self::Dislike | Self::Bookmark | Self::Repost)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentType {
    Text,
    Image,
    Video,
    Audio,
    Article,
    Link,
    Product,
}

impl ContentType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Article => "article",
            Self::Link => "link",
            Self::Product => "product",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaPart {
    pub object: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alt: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedContent {
    #[serde(rename = "type")]
    pub content_type: ContentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cover: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MediaPart>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedReference {
    pub relation: String,
    pub object_id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedSource {
    pub kind: String,
    pub original_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub original_author: Option<String>,
    pub captured_at_ms: u64,
}

/// The unified, immutable publication unit (§5). Its ObjId is computed from the claims.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedObject {
    pub kind: FeedKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment_type: Option<CommentType>,
    pub publisher: String,
    pub iat: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<FeedContent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wraps: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<FeedReference>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<FeedSource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_category: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_on: Option<String>,
}

impl FeedObject {
    /// The object a comment or interaction is about: the wrapped object for reposts and
    /// quotes, otherwise the `comment_on` reference (§5.9).
    pub fn comment_target(&self) -> Option<String> {
        match self.comment_type {
            Some(CommentType::Repost) | Some(CommentType::Quote) => self.wraps.as_deref().map(normalize_obj_id),
            Some(_) => self
                .references
                .iter()
                .find(|r| r.relation == "comment_on")
                .map(|r| normalize_obj_id(&r.object_id)),
            None => None,
        }
    }

    /// Every ObjId this object fixes as its content: wrapped object, cover, media parts.
    pub fn content_parts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(wraps) = &self.wraps {
            parts.push(normalize_obj_id(wraps));
        }
        if let Some(content) = &self.content {
            if let Some(cover) = &content.cover {
                parts.push(normalize_obj_id(cover));
            }
            for media in &content.media {
                parts.push(normalize_obj_id(&media.object));
            }
        }
        parts
    }

    pub fn search_text(&self) -> String {
        let mut text = String::new();
        if let Some(content) = &self.content {
            for value in [&content.title, &content.summary, &content.text].into_iter().flatten() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(value);
            }
        }
        text
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).expect("feed object serializes")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeadState {
    Active,
    Withdrawn,
}

impl HeadState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Withdrawn => "withdrawn",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "active" => Some(Self::Active),
            "withdrawn" => Some(Self::Withdrawn),
            _ => None,
        }
    }
}

/// Entry state (§5.4, §16.5): an independent signed object, not a `PathObject`.
/// `publisher` names the entry controller whose namespace the entry lies in.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedHead {
    pub kind: String,
    pub publisher: String,
    pub entry: String,
    pub seq: u64,
    pub state: HeadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current: Option<String>,
    pub updated_at_ms: u64,
}

pub const HEAD_KIND: &str = "feed_head";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowTarget {
    pub publisher: String,
    pub stream: String,
}

/// Follow declaration (E17): signed by the follower, written to its own stream with the
/// followed person as the only audience; validity is the Head of its entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FollowDeclaration {
    pub kind: String,
    pub publisher: String,
    pub iat: u64,
    pub entry: String,
    pub target: FollowTarget,
}

pub const FOLLOW_KIND: &str = "follow";

/// Proof of consumption (§11.3): a voluntary signed statement about one concrete version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConsumptionProof {
    pub kind: String,
    pub publisher: String,
    pub iat: u64,
    pub target: String,
    pub action: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    pub receiver: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agreement: Option<String>,
    pub nonce: String,
}

pub const CONSUMPTION_KIND: &str = "consumption_proof";

/// `type:hex` form used inside objects and as storage key; ObjIds may arrive in base32.
pub fn normalize_obj_id(value: &str) -> String {
    match ObjId::new(value) {
        Ok(id) => id.to_string(),
        Err(_) => value.to_string(),
    }
}

pub fn parse_obj_id(value: &str) -> Result<ObjId, String> {
    let id = ObjId::new(value).map_err(|e| format!("invalid ObjId {value:?}: {e}"))?;
    if id.obj_type.is_empty() || id.obj_hash.len() != 32 {
        return Err(format!("invalid ObjId {value:?}"));
    }
    Ok(id)
}

pub fn obj_id_of(obj_type: &str, claims: &Value) -> Result<(String, String), String> {
    let jcs = serde_jcs_string(claims)?;
    let (id, _) = build_named_object_by_json(obj_type, claims);
    Ok((id.to_string(), jcs))
}

/// The locked ndn-lib silently hashes `{}` when canonicalization fails; refuse such values.
fn serde_jcs_string(value: &Value) -> Result<String, String> {
    fn check(value: &Value) -> Result<(), String> {
        match value {
            Value::Number(n) if n.as_f64().is_some_and(|f| !f.is_finite()) => Err("non-finite number".into()),
            Value::Number(n) if n.is_f64() => Err("floating point numbers are not allowed in objects".into()),
            Value::Array(items) => items.iter().try_for_each(check),
            Value::Object(map) => map.values().try_for_each(check),
            _ => Ok(()),
        }
    }
    check(value)?;
    let (_, jcs) = build_named_object_by_json("x", value);
    if jcs == "{}" && value.as_object().is_none_or(|m| !m.is_empty()) {
        return Err("object cannot be canonicalized".into());
    }
    Ok(jcs)
}

pub fn obj_type_of(id: &str) -> Option<String> {
    ObjId::new(id).ok().map(|id| id.obj_type)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryNamespace {
    Feed,
    Reactions,
    Follows,
}

impl EntryNamespace {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Feed => NS_FEED,
            Self::Reactions => NS_REACTIONS,
            Self::Follows => NS_FOLLOWS,
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            NS_FEED => Some(Self::Feed),
            NS_REACTIONS => Some(Self::Reactions),
            NS_FOLLOWS => Some(Self::Follows),
            _ => None,
        }
    }
}

/// A mutable entry: a zone path under the publisher's stream, or a content DID (E08).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryRef {
    Path { zone: String, namespace: EntryNamespace, key: String },
    Did(String),
}

impl EntryRef {
    pub fn parse(entry: &str) -> Result<Self, String> {
        if entry.starts_with("did:") {
            let parts: Vec<&str> = entry.splitn(3, ':').collect();
            if parts.len() != 3 || parts[1].is_empty() || parts[2].is_empty() {
                return Err(format!("invalid entry DID {entry:?}"));
            }
            return Ok(Self::Did(entry.to_string()));
        }
        let rest = entry.strip_prefix("cyfs://").ok_or_else(|| format!("entry must be a cyfs URL or DID: {entry:?}"))?;
        let (zone, path) = rest.split_once('/').ok_or_else(|| format!("entry has no path: {entry:?}"))?;
        let zone = zone.to_ascii_lowercase();
        if zone.is_empty() || zone.contains(':') {
            return Err(format!("invalid entry zone {entry:?}"));
        }
        let mut segments = path.split('/');
        let (home, ns, at, key) = (segments.next(), segments.next(), segments.next(), segments.next());
        if home != Some("home") || at != Some("@") || segments.next().is_some() {
            return Err(format!("entry must look like cyfs://<zone>/home/<namespace>/@/<key>: {entry:?}"));
        }
        let namespace = ns.and_then(EntryNamespace::parse).ok_or_else(|| format!("unknown entry namespace in {entry:?}"))?;
        let key = key.unwrap_or_default();
        if !valid_entry_key(key) {
            return Err(format!("invalid entry key in {entry:?}"));
        }
        Ok(Self::Path { zone, namespace, key: key.to_string() })
    }

    pub fn zone(&self) -> Option<&str> {
        match self {
            Self::Path { zone, .. } => Some(zone),
            Self::Did(_) => None,
        }
    }
}

pub fn valid_entry_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 128
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && key != "."
        && key != ".."
}

pub fn entry_url(zone: &str, namespace: EntryNamespace, key: &str) -> String {
    format!("cyfs://{}/home/{}/@/{}", zone, namespace.as_str(), key)
}

pub fn stream_url(zone: &str) -> String {
    format!("cyfs://{}/home/feed", zone)
}

pub fn inbox_target(zone: &str) -> String {
    format!("cyfs://{}/home/inbox", zone)
}

pub fn digest32(parts: &[&str]) -> String {
    let mut hasher = Sha256::new();
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            hasher.update(b"\n");
        }
        hasher.update(part.as_bytes());
    }
    hex::encode(&hasher.finalize()[..16])
}

/// Interaction-key digest (§12.4): deterministic from (interactor DID, target version, type),
/// so the same interaction always lands on the same entry of the interactor's stream.
pub fn reaction_key(publisher: &str, target: &str, comment_type: CommentType) -> String {
    let target = normalize_obj_id(target);
    format!(
        "{}-{}",
        comment_type.as_str(),
        digest32(&["homestation/reaction/v1", publisher, &target, comment_type.as_str()])
    )
}

pub fn follow_key(publisher: &str, followed: &str) -> String {
    format!("f-{}", digest32(&["homestation/follow/v1", publisher, followed]))
}

pub fn is_http_url(value: &str) -> bool {
    match url::Url::parse(value) {
        Ok(url) => matches!(url.scheme(), "http" | "https") && url.host_str().is_some(),
        Err(_) => false,
    }
}

fn check_obj_ref(field: &str, value: &str) -> Result<(), String> {
    if value.contains("://") {
        return Err(format!("{field} must be an ObjId, not a URL (§5.5)"));
    }
    parse_obj_id(value).map(|_| ()).map_err(|e| format!("{field}: {e}"))
}

/// Structural rules a receiver checks before anything else (A48, §5.5, §12). Namespace
/// ownership of the entry needs a directory and is checked by the verifier.
pub fn validate_feed_object(obj: &FeedObject) -> Result<(), String> {
    if !obj.publisher.starts_with("did:") {
        return Err("publisher must be a DID".into());
    }
    match (obj.kind, obj.comment_type) {
        (FeedKind::Post, Some(_)) => return Err("a post has no comment_type".into()),
        (FeedKind::Comment, None) => return Err("a comment needs comment_type".into()),
        _ => {}
    }
    if let Some(entry) = &obj.entry {
        EntryRef::parse(entry)?;
    }
    if let Some(wraps) = &obj.wraps {
        check_obj_ref("wraps", wraps)?;
    }
    if let Some(base_on) = &obj.base_on {
        check_obj_ref("base_on", base_on)?;
    }
    for reference in &obj.references {
        if reference.relation.is_empty() {
            return Err("reference relation is empty".into());
        }
        check_obj_ref("references.object_id", &reference.object_id)?;
    }
    if let Some(content) = &obj.content {
        if let Some(cover) = &content.cover {
            check_obj_ref("content.cover", cover)?;
        }
        if content.media.len() > MAX_MEDIA_PARTS {
            return Err(format!("at most {MAX_MEDIA_PARTS} media parts"));
        }
        for media in &content.media {
            check_obj_ref("content.media.object", &media.object)?;
        }
        let inline = [&content.text, &content.title, &content.summary]
            .into_iter()
            .flatten()
            .map(|s| s.chars().count())
            .sum::<usize>();
        if inline > MAX_INLINE_TEXT {
            return Err(format!("inline content exceeds {MAX_INLINE_TEXT} characters; wrap a file instead"));
        }
    }
    if let Some(link) = &obj.link {
        if !is_http_url(link) {
            return Err("link must be an http(s) URL".into());
        }
    }
    if let Some(source) = &obj.source {
        if !is_http_url(&source.original_url) {
            return Err("source.original_url must be an http(s) URL".into());
        }
    }
    if obj.tags.len() > MAX_TAGS {
        return Err(format!("at most {MAX_TAGS} tags"));
    }
    let comment_on = obj.references.iter().filter(|r| r.relation == "comment_on").count();
    match obj.comment_type {
        Some(CommentType::Text) => {
            if comment_on != 1 {
                return Err("a text comment references exactly one comment_on target".into());
            }
            if obj.content.as_ref().and_then(|c| c.text.as_ref()).is_none_or(|t| t.trim().is_empty()) {
                return Err("a text comment needs text".into());
            }
        }
        Some(CommentType::Like) | Some(CommentType::Dislike) | Some(CommentType::Bookmark) => {
            if comment_on != 1 || obj.wraps.is_some() {
                return Err("a like/dislike/bookmark references exactly one comment_on target".into());
            }
        }
        Some(CommentType::Repost) => {
            if obj.wraps.is_none() {
                return Err("a repost wraps its target".into());
            }
        }
        Some(CommentType::Quote) => {
            if obj.wraps.is_none() || obj.content.as_ref().and_then(|c| c.text.as_ref()).is_none_or(|t| t.trim().is_empty()) {
                return Err("a quote wraps its target and carries text".into());
            }
        }
        None => {
            if obj.content.is_none() && obj.wraps.is_none() {
                return Err("a post is self-contained or wraps an object".into());
            }
        }
    }
    let size = serde_json::to_vec(obj).map(|v| v.len()).unwrap_or(usize::MAX);
    if size > MAX_OBJECT_BYTES {
        return Err("object too large".into());
    }
    Ok(())
}

pub fn validate_head(head: &FeedHead) -> Result<(), String> {
    if head.kind != HEAD_KIND {
        return Err("not a feed_head".into());
    }
    if !head.publisher.starts_with("did:") {
        return Err("head publisher must be a DID".into());
    }
    EntryRef::parse(&head.entry)?;
    if head.seq == 0 {
        return Err("head seq starts at 1".into());
    }
    match (head.state, &head.current) {
        (HeadState::Active, None) => Err("an active head names its current version".into()),
        (HeadState::Withdrawn, Some(_)) => Err("a withdrawn head points at no version".into()),
        (_, Some(current)) => check_obj_ref("current", current),
        _ => Ok(()),
    }
}

pub fn validate_follow(follow: &FollowDeclaration) -> Result<(), String> {
    if follow.kind != FOLLOW_KIND {
        return Err("not a follow declaration".into());
    }
    if !follow.publisher.starts_with("did:") || !follow.target.publisher.starts_with("did:") {
        return Err("follow publisher and target must be DIDs".into());
    }
    match EntryRef::parse(&follow.entry)? {
        EntryRef::Path { namespace: EntryNamespace::Follows, .. } => Ok(()),
        _ => Err("a follow declaration lives in the follows namespace".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn objid(n: u8) -> String {
        ObjId::new_by_raw("cyfile".into(), vec![n; 32]).to_string()
    }

    #[test]
    fn entry_urls_round_trip() {
        let url = entry_url("alice.example", EntryNamespace::Feed, "garden-0001");
        assert_eq!(url, "cyfs://alice.example/home/feed/@/garden-0001");
        let parsed = EntryRef::parse(&url).unwrap();
        assert_eq!(parsed.zone(), Some("alice.example"));
        assert!(EntryRef::parse("cyfs://alice.example/home/feed/garden").is_err());
        assert!(EntryRef::parse("https://alice.example/home/feed/@/x").is_err());
        assert!(EntryRef::parse("cyfs://alice.example/home/feed/@/a/b").is_err());
        assert!(matches!(EntryRef::parse("did:bns:plant-stand").unwrap(), EntryRef::Did(_)));
    }

    #[test]
    fn reaction_key_is_deterministic_and_typed() {
        let a = reaction_key("did:bns:bob", &objid(1), CommentType::Like);
        let b = reaction_key("did:bns:bob", &ObjId::new(&objid(1)).unwrap().to_base32(), CommentType::Like);
        assert_eq!(a, b);
        assert!(a.starts_with("like-"));
        assert_ne!(a, reaction_key("did:bns:bob", &objid(1), CommentType::Repost));
        assert_ne!(a, reaction_key("did:bns:carol", &objid(1), CommentType::Like));
        assert!(valid_entry_key(&a));
    }

    #[test]
    fn urls_are_not_content_references() {
        let mut obj: FeedObject = serde_json::from_value(json!({
            "kind": "post", "publisher": "did:bns:alice", "iat": 1,
            "content": { "type": "image", "media": [{ "object": "https://cdn.example/a.png" }] }
        }))
        .unwrap();
        assert!(validate_feed_object(&obj).unwrap_err().contains("not a URL"));
        obj.content.as_mut().unwrap().media[0].object = objid(2);
        validate_feed_object(&obj).unwrap();
        obj.link = Some("ftp://example.org".into());
        assert!(validate_feed_object(&obj).is_err());
    }

    #[test]
    fn comment_shapes() {
        let like: FeedObject = serde_json::from_value(json!({
            "kind": "comment", "comment_type": "like", "publisher": "did:bns:bob", "iat": 1,
            "references": [{ "relation": "comment_on", "object_id": objid(3) }]
        }))
        .unwrap();
        validate_feed_object(&like).unwrap();
        assert_eq!(like.comment_target(), Some(objid(3)));
        let repost: FeedObject = serde_json::from_value(json!({
            "kind": "comment", "comment_type": "repost", "publisher": "did:bns:bob", "iat": 1, "wraps": objid(3)
        }))
        .unwrap();
        validate_feed_object(&repost).unwrap();
        assert_eq!(repost.comment_target(), Some(objid(3)));
        let bad_quote: FeedObject = serde_json::from_value(json!({
            "kind": "comment", "comment_type": "quote", "publisher": "did:bns:bob", "iat": 1, "wraps": objid(3)
        }))
        .unwrap();
        assert!(validate_feed_object(&bad_quote).is_err());
    }

    #[test]
    fn head_shapes() {
        let mut head = FeedHead {
            kind: HEAD_KIND.into(),
            publisher: "did:bns:alice".into(),
            entry: entry_url("alice.example", EntryNamespace::Feed, "p1"),
            seq: 1,
            state: HeadState::Active,
            current: Some(objid(4)),
            updated_at_ms: 1,
        };
        validate_head(&head).unwrap();
        head.state = HeadState::Withdrawn;
        assert!(validate_head(&head).is_err());
        head.current = None;
        validate_head(&head).unwrap();
    }

    #[test]
    fn object_ids_are_stable_and_reject_floats() {
        let (a, _) = obj_id_of(OBJ_TYPE_FEED, &json!({ "b": 1, "a": "x" })).unwrap();
        let (b, _) = obj_id_of(OBJ_TYPE_FEED, &json!({ "a": "x", "b": 1 })).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("cyfeed:"));
        assert!(obj_id_of(OBJ_TYPE_FEED, &json!({ "a": 0.5 })).is_err());
    }
}
