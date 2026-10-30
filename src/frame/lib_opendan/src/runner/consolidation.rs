//! The host side of the Memory consolidation of the `memory_maintenance`
//! Goal (Jarvis default Goal TODO §4, §4.1), and the perception entry of
//! every session (Memory §4.2).
//!
//! - **Capabilities**: a Goal Session's Goal carries
//!   `extensions.opendan.capabilities`; `memory_consolidation` binds the
//!   drive to the Agent-level consolidation lease and opens the host tools
//!   `memory_consolidation_begin` / `memory_consolidation_commit`. Any other
//!   session calling them gets "unknown tool".
//! - **Receipts**: a batch is read (`begin`) into a receipt persisted in the
//!   Goal State (`maintenance.receipts`) before the model sees it; the plan
//!   is persisted before the Memory commit; every later stage (Memory
//!   commit → publish → cleanup) is re-entrant. Memory, Goal State and
//!   workflow are not one transaction: a crash resumes from the stage the
//!   receipt reached, the Memory commit replays by its idempotency key.
//! - **Recovery and mechanical work** ([`maintain`]): run at the start of
//!   every drive that holds the lease, before any input is read, without a
//!   model: unfinished receipts are finished, read-but-uncommitted batches
//!   are dropped (their material stays), expired deferrals are swept,
//!   cleanup is retried.

use std::sync::Arc;

use agent_tool::{
    AgentTool, AgentToolError, AgentToolResult, CallingConventions, SessionRuntimeContext, ToolSpec,
};
use async_trait::async_trait;
use buckyos_api::AiToolCall;
use llm_context::observation::Observation;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{OpenDanError, Result};
use crate::lock::Lease;
use crate::memory::{
    iso_ms, ActorKind, Batch, BatchOptions, Caller, ConsolidationCommit, Disposition, GraphOperation,
    Material, MaterialState, Memory, MemoryConfig, MemoryError, PendingWork, PerceptionInput,
    PerceptionReceipt, ProducedBy, Scope, SourceRef, SourceType, CONSOLIDATION_LEASE,
};
use crate::protocol::*;
use crate::session::runs::RunHandle;
use crate::session::Session;

use super::shared::{commit_and_report, Shared};

pub const TOOL_BEGIN: &str = "memory_consolidation_begin";
pub const TOOL_COMMIT: &str = "memory_consolidation_commit";

/// Deferrals this long past their deadline are discarded by the sweep.
pub const SWEEP_GRACE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
/// Related cognitions shown with a batch, at most.
const RELATED_LIMIT: usize = 24;

/// What the Goal of a Goal Session allows its session (read once per drive
/// from `GoalDefinition.extensions.opendan`; an ordinary session has the
/// defaults).
#[derive(Clone, Debug)]
pub struct GoalCaps {
    pub consolidation: bool,
    pub records_perception: bool,
    pub builtin: Option<String>,
    /// The Goal's owner: the subject of the session's perceptions.
    pub owner: Option<String>,
}

impl Default for GoalCaps {
    fn default() -> Self {
        Self {
            consolidation: false,
            records_perception: true,
            builtin: None,
            owner: None,
        }
    }
}

/// Read the capabilities of the session's Goal (none for other sessions)
/// and keep them for this drive.
pub(super) async fn load_caps(sh: &Shared) -> Result<GoalCaps> {
    let binding = super::goal::binding(&sh.session.lock().await.config).cloned();
    let caps = match binding {
        Some(b) => match sh.agent().goals().get(&b.goal_id).await? {
            Some(goal) => {
                let ext = goal.opendan();
                GoalCaps {
                    consolidation: ext
                        .as_ref()
                        .is_some_and(|e| e.has_capability(CAP_MEMORY_CONSOLIDATION)),
                    records_perception: ext.as_ref().is_none_or(|e| e.records_perception),
                    builtin: ext.and_then(|e| e.builtin),
                    owner: Some(goal.owner),
                }
            }
            None => GoalCaps::default(),
        },
        None => GoalCaps::default(),
    };
    *sh.goal_caps.lock().expect("goal caps") = Some(caps.clone());
    Ok(caps)
}

pub(super) fn caps(sh: &Shared) -> GoalCaps {
    sh.goal_caps
        .lock()
        .expect("goal caps")
        .clone()
        .unwrap_or_default()
}

/// The Memory component of the agent this session belongs to (one per
/// drive).
pub(super) fn memory(sh: &Shared) -> Result<Arc<Memory>> {
    let mut m = sh.memory.lock().expect("memory");
    if let Some(m) = m.as_ref() {
        return Ok(m.clone());
    }
    let root = sh
        .agent_root
        .as_ref()
        .ok_or_else(|| OpenDanError::Other("memory needs the AgentRoot (remote drivers have none)".into()))?;
    let mem = Arc::new(Memory::open(root, MemoryConfig::default()));
    *m = Some(mem.clone());
    Ok(mem)
}

fn consolidation_lease(sh: &Shared) -> Result<Arc<Lease>> {
    sh.kind_lease
        .lock()
        .expect("kind lease")
        .clone()
        .filter(|l| l.resource() == CONSOLIDATION_LEASE)
        .ok_or_else(|| OpenDanError::Other("this drive does not hold the consolidation lease".into()))
}

async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> std::result::Result<T, MemoryError> + Send + 'static,
) -> std::result::Result<T, MemoryError> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| MemoryError::Unavailable(e.to_string()))?
}

// ---------------------------------------------------------------------------
// Tool stubs (the specs the toolset shows; the runner executes them)
// ---------------------------------------------------------------------------

pub(super) struct BeginTool;
pub(super) struct CommitTool;

fn host_only() -> std::result::Result<AgentToolResult, AgentToolError> {
    Err(AgentToolError::ExecFailed(
        "memory consolidation tools require the Session host executor".into(),
    ))
}

#[async_trait]
impl AgentTool for BeginTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_BEGIN.into(),
            description: "Read the next bounded batch of unconsolidated perceptions (new and due material, whole expressions, with related deferred material as context and the cognitions already known in the same scope). The batch is persisted as a receipt (`operation_id`); dispose of it with memory_consolidation_commit before reading another. `empty: true` means there is nothing to consolidate now.".into(),
            args_schema: json!({"type":"object","additionalProperties":false,"properties":{
                "limit":{"type":"integer","minimum":1,"maximum":40,"description":"soft size of the batch (default 20)"}
            }}),
            output_schema: json!({"type":"object"}),
            usage: Some("memory_consolidation_begin({})".into()),
        }
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }
    async fn call(&self, _: &SessionRuntimeContext, _: Value) -> std::result::Result<AgentToolResult, AgentToolError> {
        host_only()
    }
}

#[async_trait]
impl AgentTool for CommitTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: TOOL_COMMIT.into(),
            description: "Commit the consolidation of the batch `operation_id` as one Memory envelope: `operations` (graph operations: add_observation, upsert_object, upsert_relation, put_item, set_status, reinforce_object_weight) and `dispositions` (one per material you read: absorbed | duplicate | discarded | deferred with a deadline). A new cognition needs scope, basis, last_evidence_at and evidence; every disposed material must be in this batch. The commit is idempotent by operation_id; cleanup of the disposed bodies follows. `abandon: true` closes the batch without disposing anything (the material comes back next time).".into(),
            args_schema: commit_args_schema(),
            output_schema: json!({"type":"object"}),
            usage: Some("memory_consolidation_commit({operation_id, summary, operations: [...], dispositions: [...]})".into()),
        }
    }
    fn calling(&self) -> CallingConventions {
        CallingConventions::ALL
    }
    async fn call(&self, _: &SessionRuntimeContext, _: Value) -> std::result::Result<AgentToolResult, AgentToolError> {
        host_only()
    }
}

/// The exact shape of a plan: field names and closed enums, so the first
/// attempt is right (every refused attempt costs a whole round).
fn commit_args_schema() -> Value {
    serde_json::from_str(r#"{"type":"object","additionalProperties":false,"properties":{
        "operation_id":{"type":"string","description":"the `operation_id` the begin call returned"},
        "summary":{"type":"string","description":"what this consolidation did, one or two sentences (1..500 chars)"},
        "operations":{"type":"array","description":"graph operations, applied in order; observations before the cognitions that cite them","items":{"type":"object","additionalProperties":true,"properties":{
            "op":{"type":"string","enum":["add_observation","upsert_object","upsert_relation","put_item","set_status","reinforce_object_weight","set_free","remove_free"]},
            "observation_id":{"type":"string","description":"add_observation: `obs_<name>` (ASCII letters, digits, `_ - .`)"},
            "kind":{"type":"string","description":"add_observation: mention | explicit_statement | behavior_signal | tool_evidence | curator_note. put_item: attribute | observation_inference | free | event_effect. upsert_object: user | person | agent | project | repo | file | service | concept | organization | place | custom"},
            "content":{"type":"string","description":"add_observation: what was observed, close to the source"},
            "source_ref":{"type":"object","description":"add_observation: the material's source, as an object","properties":{"type":{"type":"string","enum":["session_event","tool_result","file","url","manual","system"]},"event_ref":{"type":"string","description":"the material's source.event_ref, or its `ref` when there is none"},"session_id":{"type":"string"}},"required":["type","event_ref"]},
            "scope":{"type":"object","description":"subjects: the agent DID, the user principal or `workspace:<id>` from the material; objects: optional object paths, or the canonical URL of the object (obj://…, cyfs://…) so it can be read later","properties":{"subjects":{"type":"array","items":{"type":"string"}},"objects":{"type":"array","items":{"type":"string"}}},"required":["subjects"]},
            "confidence":{"type":"number","minimum":0,"maximum":1},
            "weight":{"type":"number","minimum":0,"maximum":1,"description":"recall importance"},
            "item_id":{"type":"string","description":"put_item / upsert_relation: `item_<name>`; a known cognition's id with `expected_revision` revises it"},
            "expected_revision":{"type":"integer","description":"required when revising a known cognition"},
            "claim":{"type":"object","description":"put_item: {type: <the same value as kind>, statement: <sentence>, ...}; attribute: + subject, attribute, value; event_effect: + effect. `semantic_kind` (preference, experience, …) is a separate field, not claim.type"},
            "entities":{"type":"array","items":{"type":"string"},"description":"object ids (`obj_…`) the cognition is about"},
            "evidence":{"type":"array","items":{"type":"string"},"description":"observation ids (`obs_…`) added in this commit or already known; at least one for a cognition"},
            "write_reason":{"type":"string"},
            "basis":{"type":"string","enum":["user_statement","tool_observation","third_party_claim","inference"]},
            "explicit":{"type":"boolean","description":"true only when the user asked to remember"},
            "stability":{"type":"string","enum":["stable","contextual","volatile"]},
            "last_evidence_at":{"type":"string","description":"RFC 3339: when the evidence was observed (the material's at / occurred_at), never now"},
            "valid_until":{"type":"string","description":"RFC 3339"},
            "review_at":{"type":"string","description":"RFC 3339"},
            "review_when":{"type":"array","items":{"type":"string"}},
            "semantic_kind":{"type":"string","description":"preference | experience | correction | fact | …"},
            "tags":{"type":"array","items":{"type":"string"},"description":"2..32 bytes each, letters digits spaces -"},
            "subject":{"type":"string","description":"upsert_relation: `obj_…`"},
            "predicate":{"type":"string","description":"upsert_relation: lowercase snake_case"},
            "object":{"type":"string","description":"upsert_relation: `obj_…`"},
            "object_id":{"type":"string","description":"upsert_object / reinforce_object_weight: `obj_…`"},
            "canonical_name":{"type":"string"},
            "aliases":{"type":"array","items":{"type":"object","properties":{"alias":{"type":"string"},"alias_type":{"type":"string","enum":["name","nickname","username","email","did","path","url","repo","custom"]},"confidence":{"type":"number"}},"required":["alias","alias_type","confidence"]}},
            "target_kind":{"type":"string","enum":["item","object","observation","alias"]},
            "target_id":{"type":"string"},
            "status":{"type":"string","description":"set_status: active | superseded | disputed | stale | deleted"},
            "reason":{"type":"string"},
            "replaced_by":{"type":"string"},
            "replaces":{"type":"array","items":{"type":"string"}},
            "delta":{"type":"number"},
            "key":{"type":"string","description":"set_free / remove_free"}
        },"required":["op"]}},
        "dispositions":{"type":"array","description":"one per material you read","items":{"type":"object","additionalProperties":false,"properties":{
            "perception_ref":{"type":"string","description":"the material's `ref` (`<sid>:<seq>`)"},
            "outcome":{"type":"string","enum":["absorbed","duplicate","discarded","deferred"]},
            "cognition_refs":{"type":"array","items":{"type":"string"},"description":"absorbed / duplicate: the cognition ids (`item_…`) it went into"},
            "reason":{"type":"string"},
            "deadline":{"type":"string","description":"deferred: RFC 3339, inside the deferral window"},
            "reevaluate_when":{"type":"array","items":{"type":"string"}},
            "clarify":{"type":"string","description":"deferred: a low-priority question a UI may ask"}
        },"required":["perception_ref","outcome"]}},
        "tags":{"type":"array","items":{"type":"string"}},
        "abandon":{"type":"boolean","description":"close the batch without disposing anything"},
        "reason":{"type":"string","description":"why the batch is abandoned"}
    },"required":["operation_id"]}"#).expect("commit tool schema is valid JSON")
}

// ---------------------------------------------------------------------------
// Receipt persistence
// ---------------------------------------------------------------------------

struct Context {
    sid: String,
    goal_id: String,
    epoch: u64,
    activation: Option<u64>,
    behavior: Option<String>,
}

async fn context(sh: &Shared) -> Result<Context> {
    let s = sh.session.lock().await;
    let g = s
        .state
        .goal
        .as_ref()
        .ok_or_else(|| OpenDanError::Other("goal session without its goal state".into()))?;
    Ok(Context {
        sid: s.sid().to_string(),
        goal_id: g.goal_id.clone(),
        epoch: g.epoch,
        activation: g.activation.as_ref().map(|a| a.seq),
        behavior: s.state.current_behavior.clone().or_else(|| s.config.prompt.behavior.clone()),
    })
}

async fn maintenance(sh: &Shared) -> MaintenanceState {
    sh.session
        .lock()
        .await
        .state
        .goal
        .as_ref()
        .and_then(|g| g.state.maintenance.clone())
        .unwrap_or_default()
}

/// Change the maintenance state inside a session commit, then publish the
/// Goal State (the recovery pointer of the next epoch).
async fn persist(sh: &Shared, f: impl FnOnce(&mut GoalState)) -> Result<()> {
    {
        let mut s = sh.session.lock().await;
        let g = s
            .state
            .goal
            .as_mut()
            .ok_or_else(|| OpenDanError::Other("goal session without its goal state".into()))?;
        f(&mut g.state);
        g.state.maintenance_mut().trim();
        g.state.rev += 1;
        g.state.updated_at_ms = crate::now_ms();
        commit_and_report(sh, &mut s).await?;
    }
    super::goal::flush(sh).await;
    Ok(())
}

fn invalid(m: impl Into<String>) -> OpenDanError {
    OpenDanError::InvalidArgument(m.into())
}

// ---------------------------------------------------------------------------
// begin
// ---------------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BeginArgs {
    #[serde(default)]
    limit: Option<usize>,
}

fn material_view(m: &Material) -> Value {
    let r = &m.record;
    let mut v = json!({
        "ref": m.reference,
        "at": iso_ms(r.at_ms),
        "kind": r.kind,
        "content": r.summary,
        "state": match m.state {
            MaterialState::New => "new",
            MaterialState::Deferred { due: true } => "deferred_due",
            MaterialState::Deferred { due: false } => "deferred_waiting",
        },
    });
    if !r.payload.is_null() {
        v["attributes"] = r.payload.clone();
    }
    if !r.tags.is_empty() {
        v["tags"] = json!(r.tags);
    }
    if let Some(s) = &r.scope {
        v["scope"] = json!({ "subjects": s.subjects, "objects": s.objects, "exceptions": s.exceptions });
    } else if !r.objects.is_empty() {
        v["objects"] = json!(r.objects);
    }
    if let Some(k) = &r.suggested_kind {
        v["suggested_kind"] = json!(k);
    }
    if let Some(i) = &r.memory_intent {
        v["memory_intent"] = json!(i);
    }
    if let Some(t) = &r.occurred_at {
        v["occurred_at"] = json!(t);
    }
    if let Some(a) = &r.anchors {
        v["anchors"] = serde_json::to_value(a).unwrap_or(Value::Null);
    }
    if let Some(s) = &r.source_ref {
        v["source"] = json!({
            "type": s.r#type,
            "event_ref": s.event_ref,
            "session_id": s.session_id,
            "actor_kind": s.actor_kind,
            "goal_ref": s.goal_ref,
        });
    }
    if r.is_runtime() {
        v["runtime_record"] = json!(true);
    }
    if r.backfilled {
        v["backfilled"] = json!(true);
    }
    if !m.cited.is_empty() {
        v["cites"] = json!(m
            .cited
            .iter()
            .map(|c| json!({ "cite": c.cite, "current_revision": c.current_revision, "status": c.status }))
            .collect::<Vec<_>>());
    }
    if !r.mentions.is_empty() {
        v["mentions"] = serde_json::to_value(&r.mentions).unwrap_or(Value::Null);
    }
    if let Some(d) = &m.disposition {
        v["deferred"] = json!({
            "reason": d.reason,
            "reevaluate_when": d.reevaluate_when,
            "deadline": d.deadline,
            "clarify": d.clarify,
        });
    }
    v
}

/// Cognitions already known in the scope of the batch (the model revises
/// them instead of writing them again), with the objects they name.
fn related_view(
    view: &agent_tool::agent_memory::GraphView,
    batch: &Batch,
    agent_did: &str,
) -> (Vec<Value>, Vec<Value>) {
    // Material without a scope (the runtime's own records) is about the
    // agent itself: what is known about the agent is shown with it.
    let agent_scope = Scope::new(&[agent_did], &[]);
    let scopes: Vec<&Scope> = batch
        .materials
        .iter()
        .chain(batch.context.iter())
        .map(|m| m.record.scope.as_ref().unwrap_or(&agent_scope))
        .collect();
    let mut items = Vec::new();
    let mut objects = std::collections::BTreeSet::new();
    for item in view.items() {
        if items.len() >= RELATED_LIMIT {
            break;
        }
        if !item.status.recallable() {
            continue;
        }
        let hit = match &item.scope {
            Some(s) => scopes.iter().any(|q| s.overlaps(q)),
            None => false,
        };
        if !hit {
            continue;
        }
        for e in &item.entities {
            objects.insert(e.clone());
        }
        items.push(json!({
            "ref": item.reference(),
            "kind": item.kind,
            "semantic_kind": item.semantic_kind,
            "statement": item.statement(),
            "claim": item.claim,
            "scope": item.scope,
            "basis": item.basis,
            "explicit": item.explicit,
            "stability": item.stability,
            "weight": item.weight,
            "confidence": item.confidence,
            "created_at": view.item_created_at(item),
            "last_evidence_at": item.last_evidence_at,
            "valid_until": item.valid_until,
            "review_at": item.review_at,
            "review_when": item.review_when,
            "status": item.status,
            "evidence": item.evidence,
        }));
    }
    let objects = objects
        .iter()
        .filter_map(|id| view.object(id))
        .map(|o| json!({ "object_id": o.object_id, "kind": o.kind, "name": o.canonical_name, "status": o.status }))
        .collect();
    (items, objects)
}

async fn begin(sh: &Arc<Shared>, args: BeginArgs) -> Result<Value> {
    let lease = consolidation_lease(sh)?;
    let ctx = context(sh).await?;
    let activation = ctx
        .activation
        .ok_or_else(|| invalid("a batch belongs to an activation; none is open"))?;
    let m = maintenance(sh).await;
    if let Some(open) = m.open_receipts().next() {
        return Err(invalid(format!(
            "batch {} (operation_id {}) is still open: commit it (or abandon it) before reading another",
            open.batch_id, open.operation_id
        )));
    }
    let in_activation = m
        .receipts
        .iter()
        .filter(|r| r.epoch == ctx.epoch && r.activation == activation)
        .count();
    if in_activation >= MAINTENANCE_BATCHES_PER_ACTIVATION {
        return Err(invalid(format!(
            "this activation already read {MAINTENANCE_BATCHES_PER_ACTIVATION} batches; end it with report (the rest waits for the next opportunity)"
        )));
    }
    let memory = memory(sh)?;
    let sid = ctx.sid.clone();
    let agent_did = sh.session.lock().await.config.session.agent_did.clone();
    let limit = args.limit.unwrap_or(0).min(40);
    let (batch, pending, related, objects) = blocking({
        let memory = memory.clone();
        let lease = lease.clone();
        move || {
            let c = memory.consolidator(&lease, &sid)?;
            let batch = c.begin(&BatchOptions {
                limit,
                max_context: 0,
            })?;
            let pending = memory.pending_work()?;
            let view = c.graph()?;
            let (related, objects) = related_view(&view, &batch, &agent_did);
            Ok((batch, pending, related, objects))
        }
    })
    .await
    .map_err(OpenDanError::from)?;
    if batch.materials.is_empty() {
        return Ok(json!({
            "empty": true,
            "pending": pending_view(&pending),
            "note": "nothing to consolidate now; end the activation with decision no_action",
        }));
    }
    let now = crate::now_ms();
    let operation_id = format!(
        "mc-{}",
        &crate::ids::h(&[
            &ctx.goal_id,
            &ctx.epoch.to_string(),
            &activation.to_string(),
            &batch.id,
            &in_activation.to_string(),
        ])[..24]
    );
    let receipt = ConsolidationReceipt {
        operation_id: operation_id.clone(),
        epoch: ctx.epoch,
        activation,
        batch_id: batch.id.clone(),
        graph_seq: batch.graph_seq,
        refs: batch.refs().into_iter().collect(),
        batch: serde_json::to_value(&batch).map_err(|e| OpenDanError::Other(e.to_string()))?,
        plan: None,
        stage: ReceiptStage::Prepared,
        memory_occasion: None,
        operations: 0,
        dispositions: 0,
        cleanup_pending: false,
        created_at_ms: now,
        updated_at_ms: now,
        last_error: None,
    };
    persist(sh, |st| st.maintenance_mut().receipts.push(receipt)).await?;
    crate::fault::point("consolidation:after_begin");
    let window_hours = memory.config().deferral_window.as_secs() / 3600;
    Ok(json!({
        "operation_id": operation_id,
        "batch_id": batch.id,
        "graph_seq": batch.graph_seq,
        "truncated": batch.truncated,
        "materials": batch.materials.iter().map(material_view).collect::<Vec<_>>(),
        "context": batch.context.iter().map(material_view).collect::<Vec<_>>(),
        "known_cognitions": related,
        "known_objects": objects,
        "pending": pending_view(&pending),
        "rules": {
            "deferral_window_hours": window_hours,
            "dispose": "every material in `materials` (and any `context` you read) gets one disposition {perception_ref: <ref>, outcome: absorbed|duplicate|discarded|deferred, cognition_refs?, reason?, deadline?}; context you did not judge is left alone",
            "subjects": "a cognition's scope.subjects are the Agent, its owner (the user) or a workspace; anything else is a world observation: contextual or volatile with valid_until/review_at, or hung on one of those three",
            "new_cognition": "put_item {op, item_id: item_<name>, kind: observation_inference|attribute|free|event_effect, claim: {type: <kind>, statement}, weight, confidence, evidence: [obs_…], write_reason, scope, basis: user_statement|tool_observation|third_party_claim|inference, stability: stable|contextual|volatile, last_evidence_at: <the material's at>, explicit?, valid_until?, review_at?, semantic_kind?, tags?}",
            "new_observation": "add_observation {op, observation_id: obs_<name>, kind: explicit_statement|behavior_signal|tool_evidence|mention|curator_note, content, source_ref: {type: session_event, event_ref: <material source.event_ref or ref>, session_id}, scope, confidence}",
            "revision": "a known cognition is revised with its item_id and expected_revision; a correction supersedes it (set_status superseded) or replaces it (`replaces`)",
        },
    }))
}

fn pending_view(p: &PendingWork) -> Value {
    json!({
        "new": p.new,
        "due": p.due,
        "waiting": p.waiting,
        "retry_cleanup": p.retry_cleanup,
        "next_deadline": p.next_deadline,
        "oldest_at": p.oldest_at_ms.map(iso_ms),
    })
}

// ---------------------------------------------------------------------------
// commit
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitArgs {
    operation_id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    operations: Vec<GraphOperation>,
    #[serde(default)]
    dispositions: Vec<Disposition>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    abandon: bool,
    #[serde(default)]
    reason: Option<String>,
}

async fn commit(sh: &Arc<Shared>, args: CommitArgs) -> Result<Value> {
    consolidation_lease(sh)?;
    let ctx = context(sh).await?;
    let m = maintenance(sh).await;
    let receipt = m
        .receipts
        .iter()
        .find(|r| r.operation_id == args.operation_id)
        .cloned()
        .ok_or_else(|| invalid(format!("unknown operation_id {}", args.operation_id)))?;
    match receipt.stage {
        ReceiptStage::Failed => {
            return Err(invalid(format!(
                "batch {} failed for good: {}",
                receipt.batch_id,
                receipt.last_error.unwrap_or_default()
            )))
        }
        ReceiptStage::Completed => return Ok(receipt_view(&receipt, "already_completed")),
        ReceiptStage::Prepared if receipt.plan.is_none() => {}
        _ => return advance_receipt(sh, &args.operation_id).await,
    }
    if args.abandon {
        let reason = args.reason.unwrap_or_else(|| "abandoned by the model".into());
        let id = args.operation_id.clone();
        persist(sh, |st| {
            let m = st.maintenance_mut();
            m.abandoned_batches += 1;
            if let Some(r) = m.receipt_mut(&id) {
                r.stage = ReceiptStage::Completed;
                r.last_error = Some(format!("abandoned: {reason}"));
                r.updated_at_ms = crate::now_ms();
            }
        })
        .await?;
        return Ok(json!({ "abandoned": true, "operation_id": args.operation_id }));
    }
    let summary = args
        .summary
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| invalid("a commit needs a summary of what it did (or abandon: true)"))?;
    if args.operations.is_empty() && args.dispositions.is_empty() {
        return Err(invalid(
            "a commit changes or disposes something; use abandon: true to close the batch untouched",
        ));
    }
    for d in &args.dispositions {
        if !receipt.refs.iter().any(|r| r == &d.perception_ref) {
            return Err(invalid(format!(
                "{} is not in batch {}; only material read in this batch can be disposed",
                d.perception_ref, receipt.batch_id
            )));
        }
    }
    let plan = ConsolidationCommit {
        idempotency_key: args.operation_id.clone(),
        summary: summary.chars().take(500).collect(),
        produced_by: ProducedBy {
            goal_run: Some(format!("goal:{}:{}:{}", ctx.goal_id, ctx.epoch, receipt.activation)),
            prompt_version: ctx.behavior.clone(),
            model: None,
        },
        operations: args.operations,
        dispositions: args.dispositions,
        tags: args.tags,
    };
    let plan_value = serde_json::to_value(&plan).map_err(|e| OpenDanError::Other(e.to_string()))?;
    let id = args.operation_id.clone();
    persist(sh, |st| {
        if let Some(r) = st.maintenance_mut().receipt_mut(&id) {
            r.plan = Some(plan_value);
            r.updated_at_ms = crate::now_ms();
        }
    })
    .await?;
    crate::fault::point("consolidation:after_plan");
    advance_receipt(sh, &args.operation_id).await
}

fn receipt_view(r: &ConsolidationReceipt, status: &str) -> Value {
    json!({
        "operation_id": r.operation_id,
        "batch_id": r.batch_id,
        "status": status,
        "stage": r.stage,
        "memory_occasion": r.memory_occasion,
        "operations": r.operations,
        "dispositions": r.dispositions,
        "cleanup_pending": r.cleanup_pending,
        "error": r.last_error,
    })
}

/// Take the receipt through its remaining stages (re-entrant; used by the
/// commit tool and by recovery).
async fn advance_receipt(sh: &Arc<Shared>, operation_id: &str) -> Result<Value> {
    let lease = consolidation_lease(sh)?;
    let memory = memory(sh)?;
    let ctx = context(sh).await?;
    let mut receipt = maintenance(sh)
        .await
        .receipts
        .into_iter()
        .find(|r| r.operation_id == operation_id)
        .ok_or_else(|| invalid(format!("unknown operation_id {operation_id}")))?;
    let id = operation_id.to_string();
    if receipt.stage == ReceiptStage::Prepared {
        let plan: ConsolidationCommit = receipt
            .plan
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|e| OpenDanError::Other(format!("receipt {id}: plan unreadable: {e}")))?
            .ok_or_else(|| invalid(format!("batch {} has no plan yet", receipt.batch_id)))?;
        let batch: Batch = serde_json::from_value(receipt.batch.clone())
            .map_err(|e| OpenDanError::Other(format!("receipt {id}: batch unreadable: {e}")))?;
        let ops = plan.operations.len() as u64;
        let disp = plan.dispositions.len() as u64;
        let sid = ctx.sid.clone();
        let result = blocking({
            let memory = memory.clone();
            let lease = lease.clone();
            move || {
                let c = memory.consolidator(&lease, &sid)?;
                c.commit(&batch, plan)
            }
        })
        .await;
        match result {
            Ok(r) => {
                let occ = r.occasion_id.clone();
                let rid = id.clone();
                persist(sh, move |st| {
                    let m = st.maintenance_mut();
                    m.operations += ops;
                    m.dispositions += disp;
                    if let Some(x) = m.receipt_mut(&rid) {
                        x.stage = ReceiptStage::MemoryCommitted;
                        x.memory_occasion = Some(occ);
                        x.operations = ops;
                        x.dispositions = disp;
                        x.last_error = None;
                        x.updated_at_ms = crate::now_ms();
                    }
                })
                .await?;
                crate::fault::point("consolidation:after_memory_commit");
            }
            Err(MemoryError::StaleLease(m)) => {
                let message = format!(
                    "stale consolidation lease: {m}; the lock file {} needs repair by hand (no automatic epoch bump)",
                    CONSOLIDATION_LEASE
                );
                let rid = id.clone();
                let msg = message.clone();
                persist(sh, move |st| {
                    st.last_error = Some(json!({ "kind": "stale_lease", "message": msg }));
                    let m = st.maintenance_mut();
                    m.last_error = Some(msg.clone());
                    if let Some(x) = m.receipt_mut(&rid) {
                        x.stage = ReceiptStage::Failed;
                        x.last_error = Some(msg);
                        x.updated_at_ms = crate::now_ms();
                    }
                })
                .await?;
                return Err(OpenDanError::Other(message));
            }
            Err(e @ (MemoryError::Invalid(_) | MemoryError::Conflict(_) | MemoryError::NotFound(_))) => {
                // The plan is wrong: back to a read batch, the model corrects it.
                let rid = id.clone();
                let msg = e.to_string();
                persist(sh, move |st| {
                    if let Some(x) = st.maintenance_mut().receipt_mut(&rid) {
                        x.plan = None;
                        x.last_error = Some(msg);
                        x.updated_at_ms = crate::now_ms();
                    }
                })
                .await?;
                return Err(invalid(format!("the plan was refused: {e}")));
            }
            Err(e) => return Err(OpenDanError::Other(format!("memory commit: {e}"))),
        }
        receipt.stage = ReceiptStage::MemoryCommitted;
        receipt.operations = ops;
        receipt.dispositions = disp;
    }
    if receipt.stage == ReceiptStage::MemoryCommitted {
        // P1 publishes the candidates of the plan here (none in P0).
        let rid = id.clone();
        persist(sh, move |st| {
            if let Some(x) = st.maintenance_mut().receipt_mut(&rid) {
                x.stage = ReceiptStage::Published;
                x.updated_at_ms = crate::now_ms();
            }
        })
        .await?;
        receipt.stage = ReceiptStage::Published;
    }
    let mut cleanup = json!(null);
    if receipt.stage == ReceiptStage::Published {
        let sid = ctx.sid.clone();
        let report = blocking({
            let memory = memory.clone();
            let lease = lease.clone();
            move || memory.consolidator(&lease, &sid)?.cleanup()
        })
        .await;
        let (pending, view) = match report {
            Ok(r) => {
                let pending = !r.failed.is_empty();
                (
                    pending,
                    json!({ "cleared": r.cleared.len(), "failed": r.failed, "remaining": r.remaining }),
                )
            }
            Err(e) => (true, json!({ "error": e.to_string() })),
        };
        cleanup = view;
        let rid = id.clone();
        let changed = receipt.operations > 0 || receipt.dispositions > 0;
        persist(sh, move |st| {
            let now = crate::now_ms();
            let m = st.maintenance_mut();
            m.consolidations += 1;
            if changed {
                m.last_successful_consolidation_at_ms = Some(now);
            }
            if let Some(x) = m.receipt_mut(&rid) {
                x.stage = ReceiptStage::Completed;
                x.cleanup_pending = pending;
                x.updated_at_ms = now;
            }
        })
        .await?;
        crate::fault::point("consolidation:after_cleanup");
    }
    let final_receipt = maintenance(sh)
        .await
        .receipts
        .into_iter()
        .find(|r| r.operation_id == operation_id)
        .unwrap_or(receipt);
    let mut v = receipt_view(&final_receipt, "committed");
    v["cleanup"] = cleanup;
    Ok(v)
}

// ---------------------------------------------------------------------------
// Tool dispatch and the final-report check
// ---------------------------------------------------------------------------

pub(super) fn is_tool(name: &str) -> bool {
    name == TOOL_BEGIN || name == TOOL_COMMIT
}

pub(super) async fn call(sh: &Arc<Shared>, _run: &RunHandle, call: &AiToolCall) -> Result<Observation> {
    let args = super::flush::canonical_args(&call.args);
    let result = if call.name == TOOL_BEGIN {
        match serde_json::from_value::<BeginArgs>(args) {
            Ok(a) => begin(sh, a).await,
            Err(e) => Err(invalid(e.to_string())),
        }
    } else {
        match serde_json::from_value::<CommitArgs>(args) {
            Ok(a) => commit(sh, a).await,
            Err(e) => Err(invalid(format!(
                "{e}; operations are {{op: add_observation | upsert_object | upsert_relation | put_item | set_status | reinforce_object_weight | set_free | remove_free, ...}}, dispositions are {{perception_ref, outcome: absorbed | duplicate | discarded | deferred, cognition_refs?, reason?, deadline?, reevaluate_when?, clarify?}}"
            ))),
        }
    };
    match result {
        Ok(content) => Ok(Observation::Success {
            call_id: call.call_id.clone(),
            bytes: content.to_string().len(),
            content,
            truncated: false,
            tool_result: None,
        }),
        Err(OpenDanError::InvalidArgument(message)) => Ok(Observation::Error {
            call_id: call.call_id.clone(),
            message,
            tool_result: None,
        }),
        Err(OpenDanError::Other(message)) => Ok(Observation::Error {
            call_id: call.call_id.clone(),
            message,
            tool_result: None,
        }),
        Err(e) => Err(e),
    }
}

/// A final report of the consolidation Goal: a batch that was read and
/// neither committed nor abandoned cannot be reported as acted upon.
pub(super) async fn check_final_report(sh: &Shared, result: &Value) -> Result<()> {
    if !caps(sh).consolidation {
        return Ok(());
    }
    let m = maintenance(sh).await;
    let Some(open) = m.open_receipts().next() else {
        return Ok(());
    };
    let decision = result.get("decision").and_then(Value::as_str).unwrap_or("");
    if decision == "acted" || decision == "created_work" {
        return Err(invalid(format!(
            "batch {} (operation_id {}) was read but not committed: commit it, abandon it, or report no_action / needs_info with the reason",
            open.batch_id, open.operation_id
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Mechanical maintenance at the start of a drive
// ---------------------------------------------------------------------------

/// Recovery and model-free work of the consolidation Goal: unfinished
/// receipts, batches read but never committed, the sweep of expired
/// deferrals, the retry of cleanups. Never opens an Activation.
pub(super) async fn maintain(sh: &Arc<Shared>) -> Result<()> {
    if !caps(sh).consolidation {
        return Ok(());
    }
    let lease = consolidation_lease(sh)?;
    let memory = memory(sh)?;
    if sh.session.lock().await.state.goal.is_none() {
        return Ok(());
    }
    let ctx = context(sh).await?;
    let open: Vec<ConsolidationReceipt> = maintenance(sh).await.open_receipts().cloned().collect();
    for r in open {
        match (r.stage, r.plan.is_some()) {
            (ReceiptStage::Prepared, false) => {
                let id = r.operation_id.clone();
                persist(sh, move |st| {
                    let m = st.maintenance_mut();
                    m.abandoned_batches += 1;
                    m.receipts.retain(|x| x.operation_id != id);
                })
                .await?;
                log::info!(
                    "session {}: consolidation batch {} was read but never committed; dropped",
                    ctx.sid,
                    r.batch_id
                );
            }
            _ => {
                if let Err(e) = advance_receipt(sh, &r.operation_id).await {
                    log::warn!("session {}: receipt {} not finished: {e}", ctx.sid, r.operation_id);
                }
            }
        }
    }
    let sid = ctx.sid.clone();
    let swept = blocking({
        let memory = memory.clone();
        let lease = lease.clone();
        move || {
            let c = memory.consolidator(&lease, &sid)?;
            let swept = c.sweep_expired(SWEEP_GRACE)?;
            let cleanup = c.cleanup()?;
            Ok((swept, cleanup))
        }
    })
    .await;
    match swept {
        Ok((swept, cleanup)) => {
            if swept.is_some() || !cleanup.cleared.is_empty() {
                let discarded = swept.map(|(_, n)| n as u64).unwrap_or(0);
                persist(sh, move |st| {
                    let m = st.maintenance_mut();
                    m.dispositions += discarded;
                    if discarded > 0 {
                        m.last_successful_consolidation_at_ms = Some(crate::now_ms());
                    }
                    for r in m.receipts.iter_mut().filter(|r| r.cleanup_pending) {
                        r.cleanup_pending = false;
                    }
                })
                .await?;
            }
            if !cleanup.failed.is_empty() {
                log::warn!("session {}: perception cleanup still failing: {:?}", ctx.sid, cleanup.failed);
            }
        }
        Err(e) => {
            let msg = format!("maintenance sweep/cleanup: {e}");
            persist(sh, move |st| st.maintenance_mut().last_error = Some(msg)).await?;
        }
    }
    Ok(())
}

/// The `<memory_maintenance>` block of an Activation header: the state of
/// the material and of this Goal's own bookkeeping (errors shown, never
/// turned into "nothing to do").
pub(super) async fn header_block(sh: &Shared) -> String {
    if !caps(sh).consolidation {
        return String::new();
    }
    let m = maintenance(sh).await;
    let memory = match memory(sh) {
        Ok(m) => m,
        Err(e) => return format!("<memory_maintenance error=\"{}\"/>\n", llm_context::escape_xml_attr(&e.to_string())),
    };
    let probe = blocking(move || Ok((memory.pending_work()?, memory.due_reviews()?))).await;
    let mut s = String::new();
    match probe {
        Ok((p, (reviews, next))) => {
            s.push_str(&format!(
                "<memory_maintenance new=\"{}\" due=\"{}\" waiting=\"{}\" retry_cleanup=\"{}\" due_reviews=\"{}\"{}{}{}>\n",
                p.new,
                p.due,
                p.waiting,
                p.retry_cleanup,
                reviews.len(),
                p.oldest_at_ms.map(|t| format!(" oldest=\"{}\"", iso_ms(t))).unwrap_or_default(),
                p.next_deadline.as_ref().map(|d| format!(" next_deadline=\"{}\"", llm_context::escape_xml_attr(d))).unwrap_or_default(),
                next.map(|n| format!(" next_review=\"{}\"", llm_context::escape_xml_attr(&n))).unwrap_or_default(),
            ));
            for r in reviews.iter().take(16) {
                s.push_str(&format!(
                    "<review_due ref=\"{}\" review_at=\"{}\">{}</review_due>\n",
                    llm_context::escape_xml_attr(&r.reference),
                    llm_context::escape_xml_attr(&r.review_at),
                    llm_context::escape_xml_text(&r.statement)
                ));
            }
        }
        Err(e) => s.push_str(&format!(
            "<memory_maintenance error=\"{}\">\n",
            llm_context::escape_xml_attr(&e.to_string())
        )),
    }
    s.push_str(&format!(
        "<bookkeeping consolidations=\"{}\" operations=\"{}\" dispositions=\"{}\" abandoned_batches=\"{}\" open_receipts=\"{}\"{}{}/>\n",
        m.consolidations,
        m.operations,
        m.dispositions,
        m.abandoned_batches,
        m.open_receipts().count(),
        m.last_successful_consolidation_at_ms
            .map(|t| format!(" last_consolidation=\"{}\"", iso_ms(t)))
            .unwrap_or_default(),
        m.last_error
            .as_ref()
            .map(|e| format!(" last_error=\"{}\"", llm_context::escape_xml_attr(e)))
            .unwrap_or_default(),
    ));
    s.push_str("</memory_maintenance>\n");
    s
}

// ---------------------------------------------------------------------------
// Perception entry of every session (`ctl perceive`)
// ---------------------------------------------------------------------------

/// The arguments of `ControlCommand::Perceive`, by reference.
pub(super) struct PerceiveArgs<'a> {
    pub kind: &'a str,
    pub summary: &'a str,
    pub tags: &'a [String],
    pub objects: &'a [String],
    pub subjects: &'a [String],
    pub cites: &'a [String],
    pub source: Option<&'a str>,
    pub occurred_at: Option<&'a str>,
    pub suggested_kind: Option<&'a str>,
    pub memory_intent: Option<&'a str>,
    pub mentions: &'a [String],
}

/// The Memory identity of a session, bound by the host (T01 rule): a UI
/// session speaks for its peer, a Work for the agent's owner, a Goal Session
/// for its Goal's owner; the Agent itself is always among the grants, and a
/// bound workspace is a subject too.
pub(super) fn memory_caller(cfg: &SessionConfig, caps: &GoalCaps) -> Caller {
    let agent = cfg.session.agent_did.clone();
    let subject = match cfg.session.kind {
        SessionKind::Ui => cfg
            .channels
            .outbound
            .as_ref()
            .map(|o| o.to.clone())
            .or_else(|| cfg.session.owner.clone()),
        SessionKind::Goal => caps.owner.clone().or_else(|| cfg.session.owner.clone()),
        _ => cfg.session.owner.clone(),
    };
    let mut subjects: Vec<String> = subject.into_iter().collect();
    if let Some(w) = &cfg.workspace_binding {
        subjects.push(format!("workspace:{}", w.workspace_id));
    }
    if subjects.is_empty() {
        subjects.push(agent.clone());
    }
    let mut grants: Vec<&str> = subjects.iter().map(String::as_str).collect();
    grants.push(agent.as_str());
    Caller::new(
        cfg.session_id(),
        &grants,
        &subjects.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

/// Record a perception of this session through the Memory facade: the
/// driver allocates the seq, binds subjects, source and anchors, and the
/// control input's key is the idempotency key (a re-applied input is a
/// replay).
pub(super) async fn perceive(
    sh: &Shared,
    s: &mut Session,
    input_key: &str,
    input_id: &str,
    args: PerceiveArgs<'_>,
) -> Result<PerceptionReceipt> {
    let caps = caps(sh);
    if !caps.records_perception {
        return Err(invalid(format!(
            "the session of goal {} does not record perceptions: the consolidation goal only reads them",
            caps.builtin.as_deref().unwrap_or("?")
        )));
    }
    let memory = memory(sh)?;
    let caller = memory_caller(&s.config, &caps);
    let mut source = SourceRef::event(
        SourceType::SessionEvent,
        args.source.unwrap_or(input_id).to_string(),
    );
    source.session_id = Some(s.sid().to_string());
    source.actor_kind = if args.source.is_some() {
        None
    } else {
        Some(ActorKind::Agent)
    };
    if let Some(b) = super::goal::binding(&s.config) {
        source.goal_ref = Some(format!("goal:{}", b.goal_id));
    }
    let seq = s.state.perception_seq + 1;
    let input = PerceptionInput {
        content: args.summary.to_string(),
        attributes: Value::Null,
        subjects: args.subjects.to_vec(),
        objects: args.objects.to_vec(),
        exceptions: Vec::new(),
        tags: args.tags.to_vec(),
        suggested_kind: args.suggested_kind.map(str::to_string),
        memory_intent: args.memory_intent.map(str::to_string),
        cites: args.cites.to_vec(),
        occurred_at: args.occurred_at.map(str::to_string),
        anchors: s.config.session.timezone.as_ref().map(|tz| PerceptionAnchors {
            timezone: Some(tz.clone()),
            ..Default::default()
        }),
        mentions: args.mentions.to_vec(),
        source: Some(source),
        idempotency_key: Some(input_key.to_string()),
        kind: Some(args.kind.to_string()),
        seq: Some(seq),
    };
    let lease = sh.lease.clone();
    let receipt = blocking(move || memory.record_perception(&caller, &lease, input))
        .await
        .map_err(OpenDanError::from)?;
    if receipt.seq > s.state.perception_seq {
        s.state.perception_seq = receipt.seq;
    }
    Ok(receipt)
}
