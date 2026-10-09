//! Social Delivery (§4.3, §7.4, §13.5): Push to other HomeStations' inboxes by CYFS dispatch.
//! The sender keeps the retry responsibility until `accepted`; `cached` and no response are
//! not delivery (A73). Push only speeds up convergence, Pull stays the source of truth.

use crate::error::{HsError, HsResult};
use crate::objects::get_object;
use crate::{now_ms, Station};
use ndn_lib::{parse_cyfs_dispatch_result, CyfsDispatchStatus, ObjId};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;
use serde_json::Value;
use std::time::Duration;

pub const MAX_ATTEMPTS: i64 = 12;
pub const HEADER_AUDIENCE: &str = "hs-audience";

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum DispatchOutcome {
    Accepted { admission: Option<String> },
    Cached,
    Rejected { reason: String, retryable: bool },
    NoResponse { detail: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryProgress {
    pub state: String,
    pub delivered: i64,
    pub total: i64,
}

fn backoff(attempts: i64) -> i64 {
    let secs = 5i64.saturating_mul(1i64 << attempts.clamp(0, 10));
    secs.min(3600) * 1000
}

impl Station {
    /// Queue objects for recipients; the same object is queued once per recipient (§13.5).
    pub async fn enqueue_delivery(
        &self,
        entry: Option<&str>,
        obj_ids: &[String],
        recipients: &[String],
        task_key: Option<&str>,
        restricted: bool,
    ) -> HsResult<()> {
        let owner = self.cfg.owner.clone();
        let mut rows = Vec::new();
        for recipient in recipients {
            if recipient == &owner || self.directory.known_without_home(recipient).await {
                continue;
            }
            for (i, obj_id) in obj_ids.iter().enumerate() {
                let purpose = match crate::protocol::obj_type_of(obj_id).as_deref() {
                    Some(crate::protocol::OBJ_TYPE_HEAD) => "head",
                    Some(crate::protocol::OBJ_TYPE_FOLLOW) => "follow",
                    Some(crate::protocol::OBJ_TYPE_CONSUMPTION) => "proof",
                    _ if i == 0 => "object",
                    _ => "object",
                };
                rows.push((obj_id.clone(), recipient.clone(), purpose));
            }
        }
        if rows.is_empty() {
            return Ok(());
        }
        let entry = entry.map(str::to_string);
        let task_key = task_key.map(str::to_string);
        let now = now_ms();
        self.db
            .call(move |c| {
                let tx = c.transaction()?;
                for (obj_id, recipient, purpose) in rows {
                    tx.execute(
                        "INSERT INTO outbox(obj_id, recipient, target, entry, task_key, purpose, restricted, state, attempts, next_at, created_at, updated_at)
                         VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, 'pending', 0, ?7, ?7, ?7)
                         ON CONFLICT(obj_id, recipient) DO NOTHING",
                        params![obj_id, recipient, entry, task_key, purpose, restricted as i64, now],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.wake.delivery.notify_one();
        Ok(())
    }

    pub(crate) async fn delivery_loop(self: std::sync::Arc<Self>) {
        loop {
            match self.process_due_deliveries().await {
                Ok(n) if n > 0 => continue,
                Ok(_) => {}
                Err(e) => log::warn!("delivery pass failed: {e}"),
            }
            let _ = tokio::time::timeout(self.cfg.delivery_interval, self.wake.delivery.notified()).await;
        }
    }

    /// One pass over due outbox rows; returns how many were attempted.
    pub async fn process_due_deliveries(&self) -> HsResult<usize> {
        let now = now_ms();
        let due: Vec<(i64, String, String, Option<String>, i64)> = self
            .db
            .call(move |c| {
                let mut stmt = c.prepare(
                    "SELECT o.id, o.obj_id, o.recipient,
                            CASE WHEN o.restricted=1 THEN COALESCE(p.audience, '{\"kind\":\"dids\"}') END, o.attempts
                     FROM outbox o LEFT JOIN published p ON p.entry=o.entry
                     WHERE o.state='pending' AND o.next_at<=?1 ORDER BY o.id LIMIT 32",
                )?;
                let rows = stmt
                    .query_map([now], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await?;
        let count = due.len();
        let mut settled = 0;
        for (id, obj_id, recipient, audience, attempts) in due {
            let tier = audience
                .and_then(|a| serde_json::from_str::<crate::audience::AudienceSpec>(&a).ok())
                .map(|a| a.tier().to_string());
            let (target, outcome) = match self.dispatch_to(&obj_id, &recipient, tier).await {
                Ok(v) => v,
                // The recipient definitely has no HomeStation (an agent, an unknown user): retrying cannot help.
                Err(HsError::NotFound(detail)) => (None, DispatchOutcome::Rejected { reason: format!("no-homestation: {detail}"), retryable: false }),
                Err(e) => (None, DispatchOutcome::NoResponse { detail: e.to_string() }),
            };
            let result_json = serde_json::to_string(&outcome).unwrap_or_default();
            let now = now_ms();
            let (state, admission, next_at) = match &outcome {
                DispatchOutcome::Accepted { admission } => ("accepted", admission.clone(), now),
                DispatchOutcome::Rejected { retryable: false, .. } => ("rejected", None, now),
                _ if attempts + 1 >= MAX_ATTEMPTS => ("failed", None, now),
                _ => ("pending", None, now + backoff(attempts)),
            };
            self.db
                .call(move |c| {
                    c.execute(
                        "UPDATE outbox SET state=?2, admission=?3, attempts=attempts+1, next_at=?4, last_result=?5, target=COALESCE(?6, target), updated_at=?7 WHERE id=?1",
                        params![id, state, admission, next_at, result_json, target, now],
                    )?;
                    Ok(())
                })
                .await?;
            if state != "pending" {
                settled += 1;
            }
            if let DispatchOutcome::Accepted { .. } = outcome {
                self.on_delivered(&obj_id, &recipient).await?;
            }
        }
        if settled > 0 {
            self.bump(&["published", "sources", "saved"]);
        }
        Ok(count)
    }

    async fn on_delivered(&self, obj_id: &str, recipient: &str) -> HsResult<()> {
        if crate::protocol::obj_type_of(obj_id).as_deref() == Some(crate::protocol::OBJ_TYPE_FOLLOW) {
            let recipient = recipient.to_string();
            self.db
                .call(move |c| {
                    c.execute("UPDATE sources SET notify='acknowledged' WHERE did=?1 AND notify!='unsupported'", [recipient])?;
                    Ok(())
                })
                .await?;
        }
        Ok(())
    }

    /// PUT one object to the recipient's inbox `cyfs://<zone>/home/<user>/inbox`.
    pub async fn dispatch_to(&self, obj_id: &str, recipient: &str, tier: Option<String>) -> HsResult<(Option<String>, DispatchOutcome)> {
        let id = obj_id.to_string();
        let obj = self
            .db
            .call(move |c| get_object(c, &id))
            .await?
            .ok_or_else(|| HsError::Internal(format!("object {obj_id} vanished")))?;
        let (home, origin) = self.origin_for_did(recipient).await?;
        let target = home.inbox();
        let (body, content_type) = obj.wire();
        let outcome = self.dispatch_raw(&origin, &home, obj_id, body, content_type, tier.as_deref()).await;
        Ok((Some(target), outcome))
    }

    pub async fn dispatch_raw(
        &self,
        origin: &str,
        home: &crate::protocol::HomeRef,
        obj_id: &str,
        body: String,
        content_type: &str,
        tier: Option<&str>,
    ) -> DispatchOutcome {
        let expected = match ObjId::new(obj_id) {
            Ok(id) => id,
            Err(e) => return DispatchOutcome::Rejected { reason: format!("invalid object id: {e}"), retryable: false },
        };
        let target = home.inbox();
        let target = target.as_str();
        let url = format!("{}{}", origin.trim_end_matches('/'), home.path("inbox"));
        let mut request = self
            .http
            .put(&url)
            .header("host", &home.zone)
            .header("content-type", content_type)
            .header("cyfs-obj-id", expected.to_base32())
            .header("cyfs-original-user", self.cfg.owner.clone())
            .timeout(Duration::from_secs(15))
            .body(body);
        if let Some(tier) = tier.filter(|t| *t != "public") {
            request = request.header(HEADER_AUDIENCE, tier);
        }
        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => return DispatchOutcome::NoResponse { detail: e.to_string() },
        };
        let status = response.status().as_u16();
        let headers = response.headers().clone();
        let bytes = match response.bytes().await {
            Ok(b) => b,
            Err(e) => return DispatchOutcome::NoResponse { detail: e.to_string() },
        };
        if bytes.len() > 65536 {
            return DispatchOutcome::NoResponse { detail: "oversized dispatch response".into() };
        }
        if headers.get(ndn_lib::CYFS_HEADER_DISPATCH_ERROR).is_some() {
            return DispatchOutcome::NoResponse { detail: String::from_utf8_lossy(&bytes).to_string() };
        }
        match parse_cyfs_dispatch_result(status, &headers, &bytes, &expected, target, false) {
            Ok(result) => match result.status {
                CyfsDispatchStatus::Accepted => {
                    let admission = serde_json::from_slice::<Value>(&bytes)
                        .ok()
                        .and_then(|v| v.get("admission").and_then(Value::as_str).map(str::to_string));
                    DispatchOutcome::Accepted { admission }
                }
                CyfsDispatchStatus::Cached => DispatchOutcome::Cached,
                CyfsDispatchStatus::Rejected => DispatchOutcome::Rejected {
                    reason: result.reason.unwrap_or_default(),
                    retryable: result.retryable.unwrap_or(false),
                },
            },
            Err(e) => DispatchOutcome::NoResponse { detail: format!("invalid dispatch response ({status}): {e}") },
        }
    }

    /// Delivery state of an entry's object (not its Heads) for the publish UI (§17.4).
    pub async fn entry_delivery(&self, entry: &str) -> HsResult<Option<DeliveryProgress>> {
        let entry = entry.to_string();
        self.db
            .call(move |c| {
                let row: Option<(i64, i64, i64, i64)> = c
                    .query_row(
                        "SELECT COUNT(DISTINCT recipient),
                                COUNT(DISTINCT CASE WHEN state='accepted' THEN recipient END),
                                COUNT(DISTINCT CASE WHEN state='pending' THEN recipient END),
                                COUNT(DISTINCT CASE WHEN state IN ('rejected','failed') THEN recipient END)
                         FROM outbox WHERE entry=?1 AND purpose IN ('object','follow')",
                        [entry],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                Ok(row.and_then(|(total, delivered, pending, failed)| {
                    if total == 0 {
                        return None;
                    }
                    let state = if pending > 0 {
                        "delivering"
                    } else if failed > 0 {
                        "partially_failed"
                    } else {
                        "delivered"
                    };
                    Some(DeliveryProgress { state: state.into(), delivered, total })
                }))
            })
            .await
    }

    /// Give failed and rejected-but-retryable deliveries of an entry another round.
    pub async fn retry_delivery(&self, entry: &str) -> HsResult<()> {
        let entry = entry.to_string();
        let now = now_ms();
        self.db
            .call(move |c| {
                c.execute(
                    "UPDATE outbox SET state='pending', attempts=0, next_at=?2 WHERE entry=?1 AND state IN ('failed','rejected')",
                    params![entry, now],
                )?;
                Ok(())
            })
            .await?;
        self.wake.delivery.notify_one();
        self.bump(&["published"]);
        Ok(())
    }
}
