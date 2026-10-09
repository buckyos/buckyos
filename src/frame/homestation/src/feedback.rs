//! Local feedback (§10) and voluntary proof of consumption (§11.2–§11.4). Behaviour events
//! stay local and expire; a proof is only sent for an agreement the user joined, and carries
//! only what the agreement needs.

use crate::error::{bad, HsError, HsResult};
use crate::objects::get_feed;
use crate::protocol::*;
use crate::{new_id, now_ms, now_s, Station};
use rusqlite::params;
use serde_json::{json, Value};
use std::collections::HashMap;

pub const EVENTS: &[&str] = &["impression", "open", "expand", "dwell", "play", "complete", "click", "dislike", "less_like"];

impl Station {
    pub async fn record_event(&self, obj_id: &str, event: &str, value: Option<i64>) -> HsResult<()> {
        if !EVENTS.contains(&event) {
            return Err(bad(format!("event is one of {}", EVENTS.join(", "))));
        }
        let (obj_id, event) = (normalize_obj_id(obj_id), event.to_string());
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute("INSERT INTO behavior_events(obj_id, event, value, at) VALUES (?1, ?2, ?3, ?4)", params![obj_id, event, value, now])?;
                Ok(())
            })
            .await
    }

    pub async fn purge_behavior(&self) -> HsResult<usize> {
        let before = now_ms() - self.cfg.behavior_retention_days * 86_400_000;
        self.db.call(move |c| Ok(c.execute("DELETE FROM behavior_events WHERE at<?1", [before])?)).await
    }

    /// Per-publisher bonus from recent engagement; a heuristic, never a protocol fact (§10.2).
    pub async fn publisher_affinity(&self) -> HsResult<HashMap<String, f64>> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT f.publisher, e.event, COUNT(*) FROM behavior_events e JOIN feed_index f ON f.obj_id=e.obj_id GROUP BY f.publisher, e.event",
                )?;
                let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))?.collect::<Result<Vec<_>, _>>()?;
                let mut out: HashMap<String, f64> = HashMap::new();
                for (publisher, event, n) in rows {
                    let w = match event.as_str() {
                        "open" | "expand" | "play" | "complete" | "click" => 0.1,
                        "dislike" | "less_like" => -0.5,
                        _ => 0.0,
                    };
                    *out.entry(publisher).or_default() += w * n as f64;
                }
                for v in out.values_mut() {
                    *v = v.clamp(-2.0, 1.0);
                }
                Ok(out)
            })
            .await
    }

    /// Join (or leave) a consumption agreement for one concrete version (§11.2, A19).
    pub async fn set_agreement(&self, target: &str, receiver: &str, action: &str, terms: Option<String>, joined: bool) -> HsResult<String> {
        let target = normalize_obj_id(target);
        parse_obj_id(&target).map_err(bad)?;
        if !receiver.starts_with("did:") {
            return Err(bad("receiver must be a DID"));
        }
        let id = format!("agr-{}", &digest32(&["homestation/agreement/v1", receiver, &target, action])[..16]);
        let (id2, receiver, action) = (id.clone(), receiver.to_string(), action.to_string());
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute(
                    "INSERT INTO agreements(id, target, receiver, action, terms, joined, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)
                     ON CONFLICT(id) DO UPDATE SET joined=excluded.joined, action=excluded.action, terms=excluded.terms, updated_at=excluded.updated_at",
                    params![id2, target, receiver, action, terms, joined as i64, now],
                )?;
                Ok(())
            })
            .await?;
        Ok(id)
    }

    /// Build, sign and send a proof of consumption — only for a joined agreement and only with
    /// the agreed action and result (A20). Signing proves who stated it, not attention (§11.3).
    pub async fn report_consumption(&self, agreement_id: &str, result: Option<Value>) -> HsResult<String> {
        let id = agreement_id.to_string();
        let row: Option<(String, String, String, i64)> = self
            .db
            .call(move |c| {
                Ok(rusqlite::OptionalExtension::optional(c.query_row(
                    "SELECT target, receiver, action, joined FROM agreements WHERE id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                ))?)
            })
            .await?;
        let Some((target, receiver, action, joined)) = row else { return Err(HsError::NotFound("no such agreement".into())) };
        if joined == 0 {
            return Err(HsError::Forbidden("the user has not joined this agreement".into()));
        }
        let t = target.clone();
        if self.db.call(move |c| get_feed(c, &t)).await?.is_none() {
            return Err(HsError::NotFound("unknown target version".into()));
        }
        let proof = ConsumptionProof {
            kind: CONSUMPTION_KIND.into(),
            publisher: self.cfg.owner.clone(),
            iat: now_s(),
            target: target.clone(),
            action,
            result: result.filter(|r| !r.is_null()),
            receiver: receiver.clone(),
            agreement: Some(agreement_id.to_string()),
            nonce: new_id("n"),
        };
        let obj = self.sign_object(OBJ_TYPE_CONSUMPTION, &serde_json::to_value(&proof)?)?;
        let stored = obj.clone();
        let (agreement, target2, receiver2) = (agreement_id.to_string(), target.clone(), receiver.clone());
        let now = now_ms();
        self.db
            .call(move |c| {
                crate::objects::put_object(c, &stored, Some("self"), now)?;
                c.execute(
                    "INSERT INTO consumption_proofs(obj_id, agreement_id, target, receiver, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![stored.obj_id, agreement, target2, receiver2, now],
                )?;
                Ok(())
            })
            .await?;
        self.enqueue_delivery(None, std::slice::from_ref(&obj.obj_id), &[receiver], None, true).await?;
        Ok(obj.obj_id)
    }

    pub async fn list_agreements(&self) -> HsResult<Value> {
        self.db
            .call(|c| {
                let mut stmt = c.prepare(
                    "SELECT a.id, a.target, a.receiver, a.action, a.joined, a.updated_at,
                            (SELECT COUNT(*) FROM consumption_proofs p WHERE p.agreement_id=a.id)
                     FROM agreements a ORDER BY a.updated_at DESC",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(json!({
                            "id": r.get::<_, String>(0)?, "target": r.get::<_, String>(1)?, "receiver": r.get::<_, String>(2)?,
                            "action": r.get::<_, String>(3)?, "joined": r.get::<_, i64>(4)? != 0, "updatedAt": r.get::<_, i64>(5)?,
                            "proofs": r.get::<_, i64>(6)?,
                        }))
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(json!(rows))
            })
            .await
    }
}
