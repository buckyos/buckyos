//! Explicit migration of a schema 2.10 Memory root to 3.0 (TD-12, TD-24).
//!
//! The 2.10 envelopes are verified with their original serialization, then
//! converted field by field; every lossy mapping is listed in the report.
//! The old logs, meta and snapshot are moved aside, never deleted.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::model::*;
use super::{AgentMemoryError, Result};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacySourceRef {
    r#type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    notebook_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyOccasion {
    schema_version: String,
    occasion_id: String,
    seq: u64,
    occurred_at: String,
    noticed_at: String,
    occasion_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    actor_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_ref: Option<LegacySourceRef>,
    summary: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default)]
    operations: Vec<LegacyOp>,
    digest: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum LegacyOp {
    UpsertObject(LegacyUpsertObject),
    AddObservation(LegacyAddObservation),
    ReinforceObjectWeight(LegacyReinforce),
    UpsertRelation(LegacyRelation),
    SetStatus(LegacySetStatus),
    SetFree(LegacySetFree),
    RemoveFree(LegacyRemoveFree),
    PutItem(LegacyPutItem),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyUpsertObject {
    #[serde(skip_serializing_if = "Option::is_none")]
    object_id: Option<String>,
    kind: String,
    canonical_name: String,
    #[serde(default)]
    aliases: Vec<LegacyAlias>,
    #[serde(default)]
    evidence: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weight: Option<f64>,
    confidence: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    merge_into: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyAlias {
    alias: String,
    alias_type: String,
    confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyAddObservation {
    #[serde(skip_serializing_if = "Option::is_none")]
    observation_id: Option<String>,
    kind: String,
    #[serde(default)]
    entities: Vec<String>,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_excerpt: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_ref: Option<LegacySourceRef>,
    confidence: f64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyReinforce {
    object_id: String,
    delta: f64,
    reason: String,
    #[serde(default)]
    evidence: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyRelation {
    subject: String,
    predicate: String,
    object: String,
    weight: f64,
    confidence: f64,
    #[serde(default)]
    evidence: Vec<String>,
    write_reason: String,
    #[serde(default)]
    replaces: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacySetStatus {
    target_kind: String,
    target_id: String,
    status: String,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    replaced_by: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacySetFree {
    key: String,
    content: String,
    reason: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    weight: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    confidence: Option<f64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyRemoveFree {
    key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LegacyPutItem {
    #[serde(skip_serializing_if = "Option::is_none")]
    item_id: Option<String>,
    kind: String,
    #[serde(default)]
    entities: Vec<String>,
    claim: Value,
    weight: f64,
    confidence: f64,
    #[serde(default)]
    evidence: Vec<String>,
    write_reason: String,
    #[serde(default)]
    replaces: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MigrationReport {
    pub from: String,
    pub to: String,
    pub occasions: usize,
    /// Every lossy or defaulted mapping, one line each.
    pub notes: Vec<String>,
    pub legacy_dir: String,
}

fn legacy_digest(o: &LegacyOccasion) -> Result<String> {
    let mut c = o.clone();
    c.digest.clear();
    Ok(format!(
        "blake3:{}",
        blake3::hash(&serde_json::to_vec(&c)?).to_hex()
    ))
}

fn map_enum<T: std::str::FromStr>(
    raw: &str,
    fallback: T,
    what: &str,
    at: &str,
    notes: &mut Vec<String>,
) -> T {
    match raw.parse::<T>() {
        Ok(v) => v,
        Err(_) => {
            notes.push(format!("{at}: {what} `{raw}` mapped to the fallback value"));
            fallback
        }
    }
}

fn map_source(s: LegacySourceRef, at: &str, notes: &mut Vec<String>) -> SourceRef {
    let r#type = match s.r#type.as_str() {
        "session_message" | "session" => {
            notes.push(format!(
                "{at}: source type `{}` mapped to session_event",
                s.r#type
            ));
            SourceType::SessionEvent
        }
        other => map_enum(other, SourceType::Manual, "source type", at, notes),
    };
    let uri = match (s.notebook_id, s.item_id, s.uri) {
        (_, _, Some(u)) => Some(u),
        (Some(n), i, None) => {
            notes.push(format!("{at}: notebook source kept as uri"));
            Some(match i {
                Some(i) => format!("notebook:{n}/{i}"),
                None => format!("notebook:{n}"),
            })
        }
        (None, Some(i), None) => Some(format!("notebook-item:{i}")),
        (None, None, None) => None,
    };
    SourceRef {
        r#type,
        session_id: s.session_id,
        event_ref: None,
        message_id: s.message_id,
        tool_call_id: s.tool_call_id,
        task_ref: None,
        goal_ref: None,
        uri,
        digest: s.digest,
        actor_kind: None,
    }
}

fn map_item_kind(raw: &str, at: &str, notes: &mut Vec<String>) -> ItemKind {
    match raw {
        "event" => {
            notes.push(format!("{at}: item kind `event` mapped to event_effect"));
            ItemKind::EventEffect
        }
        other => map_enum(other, ItemKind::Free, "item kind", at, notes),
    }
}

fn map_op(op: LegacyOp, at: &str, notes: &mut Vec<String>) -> GraphOperation {
    match op {
        LegacyOp::UpsertObject(o) => GraphOperation::UpsertObject(UpsertObjectOp {
            object_id: o.object_id,
            kind: map_enum(&o.kind, ObjectKind::Custom, "object kind", at, notes),
            canonical_name: o.canonical_name,
            aliases: o
                .aliases
                .into_iter()
                .map(|a| ObjectAliasInput {
                    alias_type: map_enum(&a.alias_type, AliasType::Custom, "alias type", at, notes),
                    alias: a.alias,
                    confidence: a.confidence,
                })
                .collect(),
            evidence: o.evidence,
            weight: o.weight,
            confidence: o.confidence,
            merge_into: o.merge_into,
        }),
        LegacyOp::AddObservation(o) => GraphOperation::AddObservation(AddObservationOp {
            observation_id: o.observation_id,
            kind: map_enum(
                &o.kind,
                ObservationKind::CuratorNote,
                "observation kind",
                at,
                notes,
            ),
            entities: o.entities,
            content: o.content,
            source_excerpt: o.source_excerpt,
            source_ref: o.source_ref.map(|s| map_source(s, at, notes)),
            occurred_at: None,
            scope: None,
            confidence: o.confidence,
        }),
        LegacyOp::ReinforceObjectWeight(o) => {
            GraphOperation::ReinforceObjectWeight(ReinforceObjectWeightOp {
                object_id: o.object_id,
                delta: o.delta,
                reason: o.reason,
                evidence: o.evidence,
            })
        }
        LegacyOp::UpsertRelation(o) => GraphOperation::UpsertRelation(UpsertRelationOp {
            item_id: None,
            subject: o.subject,
            predicate: o.predicate,
            object: o.object,
            weight: o.weight,
            confidence: o.confidence,
            evidence: o.evidence,
            write_reason: o.write_reason,
            replaces: o.replaces,
            expected_revision: None,
            meta: ItemMeta::default(),
        }),
        LegacyOp::SetStatus(o) => GraphOperation::SetStatus(SetStatusOp {
            target_kind: map_enum(&o.target_kind, TargetKind::Item, "target kind", at, notes),
            target_id: o.target_id,
            status: o.status,
            reason: o.reason,
            replaced_by: o.replaced_by,
            expected_revision: None,
            evidence: Vec::new(),
        }),
        LegacyOp::SetFree(o) => GraphOperation::SetFree(FlatSetOp {
            key: o.key,
            content: o.content,
            reason: o.reason,
            entities: o.entities,
            weight: o.weight,
            confidence: o.confidence,
            evidence: Vec::new(),
            expected_revision: None,
            meta: ItemMeta {
                tags: o.tags,
                ..ItemMeta::default()
            },
        }),
        LegacyOp::RemoveFree(o) => GraphOperation::RemoveFree(FlatRemoveOp {
            key: o.key,
            reason: o.reason,
        }),
        LegacyOp::PutItem(o) => GraphOperation::PutItem(PutItemOp {
            item_id: o.item_id,
            kind: map_item_kind(&o.kind, at, notes),
            entities: o.entities,
            claim: o.claim,
            weight: o.weight,
            confidence: o.confidence,
            evidence: o.evidence,
            write_reason: o.write_reason,
            replaces: o.replaces,
            expected_revision: None,
            meta: ItemMeta::default(),
        }),
    }
}

/// Read and verify every 2.10 envelope (archives, then the live log) and
/// convert it to the 3.0 form (digest recomputed by the caller).
pub(super) fn convert_legacy_logs(
    meta_dir: &Path,
    notes: &mut Vec<String>,
) -> Result<Vec<MemoryOccasion>> {
    let mut paths = Vec::new();
    let archive = meta_dir.join("archive");
    if let Ok(rd) = fs::read_dir(&archive) {
        let mut a: Vec<_> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("jsonl"))
            .collect();
        a.sort();
        paths.extend(a);
    }
    paths.push(meta_dir.join("occasions.jsonl"));
    let mut legacy = Vec::new();
    for p in &paths {
        let text = match fs::read_to_string(p) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let o: LegacyOccasion = serde_json::from_str(line).map_err(|e| {
                AgentMemoryError::Corrupted(format!("{}:{}: {e}", p.display(), n + 1))
            })?;
            if o.digest != legacy_digest(&o)? {
                return Err(AgentMemoryError::Corrupted(format!(
                    "{}:{}: 2.10 digest mismatch for {}",
                    p.display(),
                    n + 1,
                    o.occasion_id
                )));
            }
            legacy.push(o);
        }
    }
    legacy.sort_by_key(|o| o.seq);
    let mut out = Vec::new();
    for o in legacy {
        let at = o.occasion_id.clone();
        let operations = o
            .operations
            .into_iter()
            .map(|op| map_op(op, &at, notes))
            .collect();
        out.push(MemoryOccasion {
            schema_version: super::SCHEMA_VERSION.to_string(),
            occasion_id: o.occasion_id,
            seq: o.seq,
            occurred_at: o.occurred_at,
            noticed_at: o.noticed_at,
            occasion_type: o.occasion_type,
            actor_session_id: o.actor_session_id,
            lease_epoch: None,
            idempotency_key: None,
            plan_digest: None,
            produced_by: None,
            parent_occasion: None,
            source_ref: o.source_ref.map(|s| map_source(s, &at, notes)),
            summary: o.summary,
            tags: o.tags,
            operations,
            dispositions: Vec::new(),
            digest: String::new(),
        });
    }
    Ok(out)
}

/// Write a small 2.10 root (meta + one log) for migration tests.
#[cfg(test)]
pub(super) fn write_legacy_root(root: &Path) -> Result<()> {
    let meta_dir = root.join(super::META_DIR);
    fs::create_dir_all(&meta_dir)?;
    fs::write(
        meta_dir.join("meta.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "schema_version": "2.10",
            "primary_language": "en",
            "graph": { "time_model": "dual:occurred_at+noticed_at" },
            "compaction_strategy": "snapshot"
        }))?,
    )?;
    let mk = |seq: u64,
              ty: &str,
              ops: Vec<LegacyOp>,
              source: Option<LegacySourceRef>|
     -> Result<String> {
        let mut o = LegacyOccasion {
            schema_version: "2.10".into(),
            occasion_id: format!("occ_{seq:016}"),
            seq,
            occurred_at: "2026-05-01T00:00:00Z".into(),
            noticed_at: "2026-05-01T00:00:00Z".into(),
            occasion_type: ty.into(),
            actor_session_id: Some("memory.write".into()),
            source_ref: source,
            summary: "legacy".into(),
            tags: Vec::new(),
            operations: ops,
            digest: String::new(),
        };
        o.digest = legacy_digest(&o)?;
        Ok(serde_json::to_string(&o)?)
    };
    let lines = [
        mk(
            1,
            "memory.write",
            vec![LegacyOp::SetFree(LegacySetFree {
                key: "/user/style".into(),
                content: "concise".into(),
                reason: "r".into(),
                entities: Vec::new(),
                tags: vec!["style".into()],
                weight: None,
                confidence: None,
            })],
            None,
        )?,
        mk(
            2,
            "session.turn",
            vec![
                LegacyOp::AddObservation(LegacyAddObservation {
                    observation_id: Some("obs_a".into()),
                    kind: "relationship".into(),
                    entities: Vec::new(),
                    content: "User works on BuckyOS".into(),
                    source_excerpt: None,
                    source_ref: None,
                    confidence: 0.8,
                }),
                LegacyOp::UpsertObject(LegacyUpsertObject {
                    object_id: Some("obj_user".into()),
                    kind: "user".into(),
                    canonical_name: "User".into(),
                    aliases: vec![LegacyAlias {
                        alias: "user".into(),
                        alias_type: "name".into(),
                        confidence: 0.9,
                    }],
                    evidence: vec!["obs_a".into()],
                    weight: Some(0.8),
                    confidence: 0.9,
                    merge_into: None,
                }),
            ],
            Some(LegacySourceRef {
                r#type: "session_message".into(),
                session_id: Some("s1".into()),
                message_id: None,
                tool_call_id: None,
                notebook_id: Some("user/actions".into()),
                item_id: None,
                uri: None,
                digest: None,
            }),
        )?,
    ];
    fs::write(meta_dir.join("occasions.jsonl"), lines.join("\n") + "\n")?;
    Ok(())
}
