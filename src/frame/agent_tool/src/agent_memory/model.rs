//! Data model of the Memory Graph (Memory requirements, appendix A.3).

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AgentMemoryError, Result};

macro_rules! string_enum {
    ($(#[$m:meta])* $name:ident { $($var:ident => $s:literal),+ $(,)? }) => {
        $(#[$m])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
        pub enum $name {
            $(#[serde(rename = $s)] $var),+
        }

        impl $name {
            pub const ALL: &'static [$name] = &[$($name::$var),+];

            pub fn as_str(&self) -> &'static str {
                match self {
                    $($name::$var => $s),+
                }
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }

        impl FromStr for $name {
            type Err = AgentMemoryError;

            fn from_str(s: &str) -> Result<Self> {
                match s.trim() {
                    $($s => Ok($name::$var),)+
                    other => Err(AgentMemoryError::Invalid(format!(
                        "invalid {} `{}`; expected one of: {}",
                        stringify!($name),
                        other,
                        [$($s),+].join(", ")
                    ))),
                }
            }
        }
    };
}

string_enum!(
    /// `SourceRef.type` (A.3.1).
    SourceType {
        SessionEvent => "session_event",
        ToolResult => "tool_result",
        File => "file",
        Url => "url",
        Manual => "manual",
        System => "system",
    }
);

string_enum!(
    /// Who produced the source event; `runtime` marks material the Runtime
    /// attached (e.g. a Memory observation block), never a new fact source.
    ActorKind {
        User => "user",
        Agent => "agent",
        Tool => "tool",
        Runtime => "runtime",
        System => "system",
        ThirdParty => "third_party",
    }
);

string_enum!(
    ObjectKind {
        User => "user",
        Person => "person",
        Agent => "agent",
        Project => "project",
        Repo => "repo",
        File => "file",
        Service => "service",
        Concept => "concept",
        Organization => "organization",
        Place => "place",
        Custom => "custom",
    }
);

string_enum!(
    AliasType {
        Name => "name",
        Nickname => "nickname",
        Username => "username",
        Email => "email",
        Did => "did",
        Path => "path",
        Url => "url",
        Repo => "repo",
        Custom => "custom",
    }
);

string_enum!(
    ObjectStatus {
        Active => "active",
        Merged => "merged",
        Deprecated => "deprecated",
        Deleted => "deleted",
    }
);

string_enum!(
    AliasStatus {
        Active => "active",
        Deprecated => "deprecated",
        Merged => "merged",
        Deleted => "deleted",
    }
);

string_enum!(
    ObservationKind {
        Mention => "mention",
        ExplicitStatement => "explicit_statement",
        BehaviorSignal => "behavior_signal",
        ToolEvidence => "tool_evidence",
        CuratorNote => "curator_note",
    }
);

string_enum!(
    ObservationStatus {
        Active => "active",
        Superseded => "superseded",
        Disputed => "disputed",
        Deleted => "deleted",
    }
);

string_enum!(
    ItemKind {
        Object => "object",
        Attribute => "attribute",
        Relation => "relation",
        EventEffect => "event_effect",
        Salience => "salience",
        ObservationInference => "observation_inference",
        Free => "free",
    }
);

string_enum!(
    ItemStatus {
        Active => "active",
        Superseded => "superseded",
        Disputed => "disputed",
        Stale => "stale",
        Deleted => "deleted",
    }
);

string_enum!(
    /// Evidence character of a cognition (M-06); independent of weight and
    /// confidence.
    Basis {
        UserStatement => "user_statement",
        ToolObservation => "tool_observation",
        ThirdPartyClaim => "third_party_claim",
        Inference => "inference",
    }
);

string_enum!(
    DispositionOutcome {
        Absorbed => "absorbed",
        Duplicate => "duplicate",
        Discarded => "discarded",
        Deferred => "deferred",
    }
);

string_enum!(
    TargetKind {
        Item => "item",
        Object => "object",
        Observation => "observation",
        Alias => "alias",
    }
);

impl ItemStatus {
    /// Takes part in ordinary recall (A.3.5).
    pub fn recallable(self) -> bool {
        matches!(self, ItemStatus::Active | ItemStatus::Disputed)
    }

    /// Legal transitions (A.3.5). Reviving a superseded or stale item needs
    /// explicit evidence; `deleted` is terminal.
    pub fn can_become(self, to: ItemStatus, has_evidence: bool) -> bool {
        use ItemStatus::*;
        match (self, to) {
            (a, b) if a == b => false,
            (Deleted, _) => false,
            (Active, _) => true,
            (Disputed, _) => true,
            (Stale, Active) | (Superseded, Active) | (Superseded, Disputed) => has_evidence,
            (Stale, _) => true,
            (Superseded, Deleted) => true,
            (Superseded, _) => false,
        }
    }
}

impl ObjectStatus {
    pub fn can_become(self, to: ObjectStatus) -> bool {
        use ObjectStatus::*;
        match (self, to) {
            (a, b) if a == b => false,
            (Deleted, _) | (Merged, _) => false,
            (Active, _) => true,
            (Deprecated, _) => true,
        }
    }
}

impl AliasStatus {
    pub fn can_become(self, to: AliasStatus) -> bool {
        use AliasStatus::*;
        match (self, to) {
            (a, b) if a == b => false,
            (Deleted, _) | (Merged, _) => false,
            (Active, _) => true,
            (Deprecated, _) => true,
        }
    }
}

impl ObservationStatus {
    pub fn can_become(self, to: ObservationStatus) -> bool {
        use ObservationStatus::*;
        match (self, to) {
            (a, b) if a == b => false,
            (Deleted, _) => false,
            (Active, _) | (Disputed, _) => true,
            (Superseded, Deleted) => true,
            (Superseded, _) => false,
        }
    }
}

impl DispositionOutcome {
    /// Absorbed / duplicate / discarded end the material's life; its body is
    /// cleaned after the commit.
    pub fn is_terminal(self) -> bool {
        !matches!(self, DispositionOutcome::Deferred)
    }
}

/// Source reference (A.3.1). Integrated sources point back to the original
/// Session event so the evidence stays traceable after the perception body
/// is cleaned.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SourceRef {
    pub r#type: SourceType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_kind: Option<ActorKind>,
}

impl SourceRef {
    pub fn event(r#type: SourceType, event_ref: impl Into<String>) -> Self {
        Self {
            r#type,
            session_id: None,
            event_ref: Some(event_ref.into()),
            message_id: None,
            tool_call_id: None,
            task_ref: None,
            goal_ref: None,
            uri: None,
            digest: None,
            actor_kind: None,
        }
    }

    /// Points somewhere a reader can go back to.
    pub fn is_traceable(&self) -> bool {
        self.event_ref
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
            || self.uri.as_deref().is_some_and(|s| !s.trim().is_empty())
    }

    pub fn validate(&self) -> Result<()> {
        for (name, v) in [
            ("session_id", &self.session_id),
            ("event_ref", &self.event_ref),
            ("message_id", &self.message_id),
            ("tool_call_id", &self.tool_call_id),
            ("task_ref", &self.task_ref),
            ("goal_ref", &self.goal_ref),
            ("uri", &self.uri),
            ("digest", &self.digest),
        ] {
            if let Some(v) = v {
                if v.trim().is_empty() || v.chars().any(char::is_control) {
                    return Err(AgentMemoryError::Invalid(format!(
                        "source_ref.{name} must be a non-empty single-line string"
                    )));
                }
            }
        }
        Ok(())
    }
}

/// Principals a Session may read for (A.3.7): its user, groups and the Agent
/// itself. Bound by the host, never by the model.
pub type Grants = BTreeSet<String>;

/// Applicability and visibility range (A.3.7).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Scope {
    /// Subject references (user, group, the Agent itself); at least one.
    pub subjects: Vec<String>,
    /// Object paths split by `/`; empty = not limited to objects.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    /// Object paths the scope explicitly does not cover.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exceptions: Vec<String>,
}

/// The range a query or a topic looks at.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct QueryScope {
    #[serde(default)]
    pub subjects: Vec<String>,
    #[serde(default)]
    pub objects: Vec<String>,
}

pub const MAX_SCOPE_ENTRIES: usize = 32;
pub const MAX_OBJECT_PATH_BYTES: usize = 512;

pub fn validate_principal(p: &str) -> Result<()> {
    if p.trim().is_empty() || p.len() > MAX_OBJECT_PATH_BYTES || p.chars().any(char::is_control) {
        return Err(AgentMemoryError::Invalid(format!(
            "invalid principal `{p}`"
        )));
    }
    Ok(())
}

pub fn validate_object_path(p: &str) -> Result<()> {
    if p.is_empty() || p.len() > MAX_OBJECT_PATH_BYTES || p.chars().any(char::is_control) {
        return Err(AgentMemoryError::Invalid(format!(
            "invalid object path `{p}`"
        )));
    }
    for seg in p.split('/') {
        if seg.trim().is_empty() || seg == "." || seg == ".." {
            return Err(AgentMemoryError::Invalid(format!(
                "object path `{p}` has an empty or relative segment"
            )));
        }
    }
    Ok(())
}

/// Two object paths overlap when one is a segment prefix of the other
/// (equal included): `a/b` overlaps `a` and `a/b/c`, `a2` does not overlap `a`.
pub fn object_paths_overlap(a: &str, b: &str) -> bool {
    path_is_under(a, b) || path_is_under(b, a)
}

/// `path` equals `base` or lies below it.
pub fn path_is_under(path: &str, base: &str) -> bool {
    let mut p = path.split('/');
    for seg in base.split('/') {
        if p.next() != Some(seg) {
            return false;
        }
    }
    true
}

fn sorted_dedup(v: &[String]) -> Vec<String> {
    let mut out: Vec<String> = v
        .iter()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
        .collect();
    out.sort();
    out.dedup();
    out
}

impl Scope {
    pub fn new(subjects: &[&str], objects: &[&str]) -> Self {
        Scope {
            subjects: subjects.iter().map(|s| s.to_string()).collect(),
            objects: objects.iter().map(|s| s.to_string()).collect(),
            exceptions: Vec::new(),
        }
    }

    pub fn with_exceptions(mut self, exceptions: &[&str]) -> Self {
        self.exceptions = exceptions.iter().map(|s| s.to_string()).collect();
        self
    }

    pub fn normalized(&self) -> Scope {
        Scope {
            subjects: sorted_dedup(&self.subjects),
            objects: sorted_dedup(&self.objects),
            exceptions: sorted_dedup(&self.exceptions),
        }
    }

    pub fn validate(&self) -> Result<()> {
        if self.subjects.is_empty() {
            return Err(AgentMemoryError::Invalid(
                "scope.subjects needs at least one subject".into(),
            ));
        }
        if self.subjects.len() + self.objects.len() + self.exceptions.len() > MAX_SCOPE_ENTRIES {
            return Err(AgentMemoryError::Invalid(format!(
                "scope has more than {MAX_SCOPE_ENTRIES} entries"
            )));
        }
        for s in &self.subjects {
            validate_principal(s)?;
        }
        for o in self.objects.iter().chain(&self.exceptions) {
            validate_object_path(o)?;
        }
        Ok(())
    }

    /// Canonical identity of the scope (relation triples are revised in place
    /// only within the same scope).
    pub fn key(&self) -> String {
        let n = self.normalized();
        format!(
            "s={};o={};x={}",
            n.subjects.join(","),
            n.objects.join(","),
            n.exceptions.join(",")
        )
    }

    /// Every subject is among the reader's grants (A.3.7). An empty subject
    /// list (records written without a scope) is visible to every Session of
    /// the Agent.
    pub fn visible_to(&self, grants: &Grants) -> bool {
        self.subjects.iter().all(|s| grants.contains(s))
    }

    /// Range match (A.3.7): shared subject, overlapping objects (an omitted
    /// side overlaps anything), query objects not under an exception.
    pub fn matches(&self, q: &QueryScope) -> bool {
        if !self.subjects.iter().any(|s| q.subjects.contains(s)) {
            return false;
        }
        if q.objects.is_empty() {
            return true;
        }
        q.objects.iter().any(|qo| {
            let overlaps =
                self.objects.is_empty() || self.objects.iter().any(|o| object_paths_overlap(o, qo));
            overlaps && !self.exceptions.iter().any(|x| path_is_under(qo, x))
        })
    }

    /// Object pairs that overlap when both sides name objects (a structural
    /// entry, not only a filter).
    pub fn object_hits(&self, q: &QueryScope) -> Vec<String> {
        let mut out = Vec::new();
        for o in &self.objects {
            for qo in &q.objects {
                if object_paths_overlap(o, qo)
                    && !self.exceptions.iter().any(|x| path_is_under(qo, x))
                {
                    out.push(o.clone());
                    break;
                }
            }
        }
        out
    }

    /// Same subject and overlapping objects in both directions (used to tie a
    /// correction to the cognitions it may affect, §4.3).
    pub fn overlaps(&self, other: &Scope) -> bool {
        self.matches(&QueryScope {
            subjects: other.subjects.clone(),
            objects: other.objects.clone(),
        })
    }
}

/// Integration version that produced a cognition (M-05).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProducedBy {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal_run: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryObject {
    pub object_id: String,
    pub kind: ObjectKind,
    pub canonical_name: String,
    #[serde(default)]
    pub aliases: Vec<ObjectAlias>,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub source_occasion: String,
    pub last_occasion: String,
    pub noticed_at: String,
    pub status: ObjectStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged_into: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectAlias {
    pub alias: String,
    pub alias_type: AliasType,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub source_occasion: String,
    pub noticed_at: String,
    pub status: AliasStatus,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryObservation {
    pub observation_id: String,
    pub kind: ObservationKind,
    pub source_occasion: String,
    #[serde(default)]
    pub entities: Vec<String>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    pub confidence: f64,
    pub noticed_at: String,
    pub status: ObservationStatus,
}

/// A cognition (A.3.4). `item_id` is the logical id and stays stable across
/// revisions; every revision is kept in the replayed history.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryItem {
    pub item_id: String,
    pub revision: u64,
    pub kind: ItemKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_kind: Option<String>,
    #[serde(default)]
    pub entities: Vec<String>,
    pub claim: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    /// Occasion of the current revision.
    pub source_occasion: String,
    /// Occasion that created the logical item.
    pub first_occasion: String,
    pub noticed_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_when: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produced_by: Option<ProducedBy>,
    pub write_reason: String,
    pub status: ItemStatus,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_key: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

impl MemoryItem {
    /// `item_x@3`.
    pub fn reference(&self) -> String {
        format!("{}@{}", self.item_id, self.revision)
    }

    pub fn statement(&self) -> String {
        claim_summary(&self.claim)
    }

    /// Recallable now: status and validity window (M-05, M-20).
    pub fn is_current(&self, now: chrono::DateTime<chrono::Utc>) -> bool {
        if !self.status.recallable() || self.kind == ItemKind::Salience {
            return false;
        }
        match &self.valid_until {
            Some(t) => chrono::DateTime::parse_from_rfc3339(t)
                .map(|t| t.with_timezone(&chrono::Utc) > now)
                .unwrap_or(false),
            None => true,
        }
    }

    pub fn subjects(&self) -> &[String] {
        self.scope
            .as_ref()
            .map(|s| s.subjects.as_slice())
            .unwrap_or(&[])
    }

    pub fn visible_to(&self, grants: &Grants) -> bool {
        self.scope
            .as_ref()
            .map(|s| s.visible_to(grants))
            .unwrap_or(true)
    }
}

/// Fields shared by every cognition-writing operation.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ItemMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub semantic_kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<Basis>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub valid_until: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub review_when: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Graph operations (A.4.2).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum GraphOperation {
    UpsertObject(UpsertObjectOp),
    AddObservation(AddObservationOp),
    ReinforceObjectWeight(ReinforceObjectWeightOp),
    UpsertRelation(UpsertRelationOp),
    SetStatus(SetStatusOp),
    SetFree(FlatSetOp),
    RemoveFree(FlatRemoveOp),
    PutItem(PutItemOp),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpsertObjectOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub object_id: Option<String>,
    pub kind: ObjectKind,
    pub canonical_name: String,
    #[serde(default)]
    pub aliases: Vec<ObjectAliasInput>,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    pub confidence: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge_into: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ObjectAliasInput {
    pub alias: String,
    pub alias_type: AliasType,
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AddObservationOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub observation_id: Option<String>,
    pub kind: ObservationKind,
    #[serde(default)]
    pub entities: Vec<String>,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_excerpt: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<Scope>,
    pub confidence: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ReinforceObjectWeightOp {
    pub object_id: String,
    pub delta: f64,
    pub reason: String,
    #[serde(default)]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UpsertRelationOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub subject: String,
    pub predicate: String,
    pub object: String,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub write_reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(flatten)]
    pub meta: ItemMeta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SetStatusOp {
    pub target_kind: TargetKind,
    /// `item_…`, `obj_…`, `obs_…`, or `<object_id>:<alias>` for an alias.
    pub target_id: String,
    pub status: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replaced_by: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    /// Required to revive a superseded or stale item.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlatSetOp {
    pub key: String,
    pub content: String,
    pub reason: String,
    #[serde(default)]
    pub entities: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(flatten)]
    pub meta: ItemMeta,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct FlatRemoveOp {
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PutItemOp {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    pub kind: ItemKind,
    #[serde(default)]
    pub entities: Vec<String>,
    pub claim: Value,
    pub weight: f64,
    pub confidence: f64,
    #[serde(default)]
    pub evidence: Vec<String>,
    pub write_reason: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replaces: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<u64>,
    #[serde(flatten)]
    pub meta: ItemMeta,
}

/// Per-material outcome of a consolidation (A.4.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Disposition {
    /// `<sid>:<seq>`.
    pub perception_ref: String,
    pub outcome: DispositionOutcome,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cognition_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reevaluate_when: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    /// A low-priority question a related UI may ask when the material waits
    /// for a user confirmation (§5.9).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clarify: Option<String>,
}

/// One line of `.meta/occasions.jsonl`: the commit unit and replay truth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MemoryOccasion {
    pub schema_version: String,
    pub occasion_id: String,
    pub seq: u64,
    pub occurred_at: String,
    pub noticed_at: String,
    pub occasion_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lease_epoch: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub produced_by: Option<ProducedBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_occasion: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_ref: Option<SourceRef>,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub operations: Vec<GraphOperation>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dispositions: Vec<Disposition>,
    pub digest: String,
}

pub type Envelope = MemoryOccasion;

/// Human-readable form of a claim. Any claim may carry an optional
/// `statement`, which wins over the structural rendering.
pub fn claim_summary(claim: &Value) -> String {
    let s = |k: &str| claim.get(k).and_then(Value::as_str).unwrap_or("");
    if !s("statement").is_empty() {
        return s("statement").to_string();
    }
    match s("type") {
        "relation" => format!("{} {} {}", s("subject"), s("predicate"), s("object")),
        "attribute" => format!("{} {} {}", s("subject"), s("attribute"), s("value")),
        "event_effect" => s("effect").to_string(),
        "salience" => s("reason").to_string(),
        _ => serde_json::to_string(claim).unwrap_or_default(),
    }
}

/// Shape check of a claim against its item kind.
pub fn validate_claim(kind: ItemKind, claim: &Value) -> Result<()> {
    let obj = claim
        .as_object()
        .ok_or_else(|| AgentMemoryError::Invalid("claim must be a JSON object".into()))?;
    let ty = obj.get("type").and_then(Value::as_str).unwrap_or("");
    if ty != kind.as_str() {
        return Err(AgentMemoryError::Invalid(format!(
            "claim.type `{ty}` does not match item kind `{kind}`"
        )));
    }
    let need = |fields: &[&str]| -> Result<()> {
        for f in fields {
            if obj
                .get(*f)
                .and_then(Value::as_str)
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err(AgentMemoryError::Invalid(format!(
                    "{kind} claim needs a non-empty `{f}`"
                )));
            }
        }
        Ok(())
    };
    match kind {
        ItemKind::Object => need(&["object_id", "statement"]),
        ItemKind::Attribute => need(&["subject", "attribute", "value"]),
        ItemKind::Relation => need(&["subject", "predicate", "object"]),
        ItemKind::EventEffect => need(&["effect"]),
        ItemKind::Salience => need(&["object_id", "reason"]),
        ItemKind::ObservationInference | ItemKind::Free => need(&["statement"]),
    }
}
