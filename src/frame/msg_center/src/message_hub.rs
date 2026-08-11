//! MessageHub — the native delivery executor for shareable DIDs.
//!
//! MessageHub is *not* a message tunnel (see `Message Tunnel Design.md` §2):
//! it carries `MsgObject`s losslessly between zones instead of adapting them
//! to an external platform. It shares the `DeliveryExecutor` interface, the
//! delivery state machine and the idempotency key with tunnels.
//!
//! Current scope: targets that live in this zone (zone users, agents, hosted
//! groups) are delivered natively by dispatching into their mailboxes.
//! Cross-zone delivery (resolve DID → target zone → POST MsgObject) is the
//! designed follow-up; until it lands, a non-local target fails the delivery
//! deterministically (DEAD, diagnosable, manually re-queueable) instead of
//! falling back to anything.

use crate::msg_center::MessageCenter;
use crate::msg_tunnel::DeliveryExecutor;
use anyhow::{anyhow, Result as AnyResult};
use async_trait::async_trait;
use buckyos_api::{DeliveryRecordWithObject, DeliveryReportResult, MsgCenterHandler};
use kRPC::RPCContext;
use log::{info, warn};
use name_lib::DID;

pub const MESSAGE_HUB_PLATFORM: &str = "messagehub";

pub struct MessageHubExecutor {
    transport_did: DID,
    center: MessageCenter,
    name: String,
}

impl MessageHubExecutor {
    pub fn new(transport_did: DID, center: MessageCenter) -> Self {
        Self {
            transport_did,
            center,
            name: "message-hub".to_string(),
        }
    }
}

#[async_trait]
impl DeliveryExecutor for MessageHubExecutor {
    fn transport_did(&self) -> DID {
        self.transport_did.clone()
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn platform(&self) -> &str {
        MESSAGE_HUB_PLATFORM
    }

    fn supports_ingress(&self) -> bool {
        // Native ingress arrives through the msg-center `dispatch` RPC surface,
        // not through a poller owned by this executor.
        false
    }

    fn supports_egress(&self) -> bool {
        true
    }

    async fn start(&self) -> AnyResult<()> {
        Ok(())
    }

    async fn stop(&self) -> AnyResult<()> {
        Ok(())
    }

    async fn execute_delivery(
        &self,
        record: DeliveryRecordWithObject,
    ) -> AnyResult<DeliveryReportResult> {
        let envelope = record.record.envelope.clone();
        if envelope.transport_did != self.transport_did {
            return Err(anyhow!(
                "delivery {} belongs to executor {}, not message hub {}",
                record.record.delivery_id,
                envelope.transport_did.to_string(),
                self.transport_did.to_string()
            ));
        }
        // Shadow endpoint DIDs are local-only and must never reach the hub.
        if envelope.target_did.method == "msgtunnel" {
            return Ok(DeliveryReportResult {
                ok: false,
                error_code: Some("shadow_did_rejected".to_string()),
                error_message: Some(format!(
                    "message hub rejects local shadow endpoint target {}",
                    envelope.target_did.to_string()
                )),
                retryable: Some(false),
                ..Default::default()
            });
        }

        let msg = record
            .get_msg()
            .await
            .map_err(|error| anyhow!("load message for hub delivery failed: {}", error))?;

        if self.center.is_local_recipient(&envelope.target_did) {
            // Native local delivery: the destination zone is this zone, so the
            // "POST MsgObject to the target zone" hop degenerates into a local
            // dispatch that creates the recipient mailbox records.
            let idempotency_key = format!("hub:{}", record.record.delivery_id);
            let dispatch = self
                .center
                .handle_dispatch(msg, None, Some(idempotency_key), RPCContext::default())
                .await
                .map_err(|error| anyhow!("hub local dispatch failed: {}", error))?;
            // Judge the outcome for *this* target: a dispatch can succeed
            // overall while dropping individual recipients (ACL/no mailbox).
            let target_delivered = dispatch.ok
                && (dispatch.delivered_recipients.contains(&envelope.target_did)
                    || dispatch.delivered_group.as_ref() == Some(&envelope.target_did));
            if target_delivered {
                info!(
                    "message hub delivered locally: delivery_id={} target={}",
                    record.record.delivery_id,
                    envelope.target_did.to_string()
                );
                Ok(DeliveryReportResult {
                    ok: true,
                    external_msg_id: Some(dispatch.msg_id.to_string()),
                    ..Default::default()
                })
            } else {
                // Rejected by local policy (blocked sender, no mailbox). Not
                // retryable: the outcome is deterministic.
                Ok(DeliveryReportResult {
                    ok: false,
                    error_code: Some("local_dispatch_rejected".to_string()),
                    error_message: Some(dispatch.reason.unwrap_or_else(|| {
                        format!(
                            "local dispatch dropped recipient {}",
                            envelope.target_did.to_string()
                        )
                    })),
                    retryable: Some(false),
                    ..Default::default()
                })
            }
        } else {
            warn!(
                "message hub cross-zone delivery not implemented yet: delivery_id={} target={}",
                record.record.delivery_id,
                envelope.target_did.to_string()
            );
            Ok(DeliveryReportResult {
                ok: false,
                error_code: Some("remote_zone_delivery_unimplemented".to_string()),
                error_message: Some(format!(
                    "target {} is not hosted in this zone and cross-zone hub delivery is not implemented yet",
                    envelope.target_did.to_string()
                )),
                retryable: Some(false),
                ..Default::default()
            })
        }
    }
}
