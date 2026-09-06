//! MessageHub — the native delivery executor for shareable DIDs.
//!
//! MessageHub is *not* a message tunnel (see `Message Tunnel Design.md` §2):
//! it carries `MsgObject`s losslessly between zones instead of adapting them
//! to an external platform. It shares the `DeliveryExecutor` interface, the
//! delivery state machine and the idempotency key with tunnels.
//!
use crate::msg_center::MessageCenter;
use crate::msg_tunnel::DeliveryExecutor;
use anyhow::{anyhow, Result as AnyResult};
use async_trait::async_trait;
use buckyos_api::{DeliveryRecordWithObject, DeliveryReportResult};
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
            let settings = self.center.cyfs_dispatch.read().unwrap().clone();
            let mut paths: Vec<_> = settings
                .accepted_paths
                .iter()
                .filter(|(_, did)| *did == &envelope.target_did)
                .map(|(path, _)| path)
                .collect();
            paths.sort();
            let configured_target = settings
                .target_zone
                .as_ref()
                .zip(paths.first())
                .and_then(|(zone, path)| ndn_lib::normalize_cyfs_dispatch_target(zone, path).ok());
            let snapshot = envelope.address.as_ref().and_then(|a| a.address.as_ref());
            if snapshot.is_some() && snapshot != configured_target.as_ref() {
                return Ok(DeliveryReportResult {
                    ok: false,
                    error_code: Some("native-route-changed".into()),
                    error_message: Some("Local receiver differs from the delivery snapshot".into()),
                    retryable: Some(false),
                    ..Default::default()
                });
            }
            let target = configured_target.unwrap_or_else(|| {
                format!("cyfs://local/{}/inbox", envelope.target_did.to_string())
            });
            let dispatch = self
                .center
                .dispatch_to_receiver(msg, envelope.target_did.clone(), target)
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
            let route = self
                .center
                .cyfs_dispatch
                .read()
                .unwrap()
                .outgoing
                .get(&envelope.target_did.to_string())
                .cloned();
            if let Some(route) = route {
                let snapshot = envelope.address.as_ref().and_then(|a| a.address.as_deref());
                if snapshot != Some(route.target.as_str()) {
                    return Ok(DeliveryReportResult {
                        ok: false,
                        error_code: Some("native-route-changed".into()),
                        error_message: Some(
                            "Configured logical receiver differs from the delivery snapshot".into(),
                        ),
                        retryable: Some(false),
                        ..Default::default()
                    });
                }
                return crate::cyfs_dispatch::send(&route, &msg, &envelope.msg_id).await;
            }
            warn!(
                "message hub has no configured native route: delivery_id={} target={}",
                record.record.delivery_id,
                envelope.target_did.to_string()
            );
            Ok(DeliveryReportResult {
                ok: false,
                error_code: Some("native-route-not-configured".to_string()),
                error_message: Some(format!(
                    "target {} has no configured CYFS delivery route",
                    envelope.target_did.to_string()
                )),
                retryable: Some(false),
                ..Default::default()
            })
        }
    }
}
