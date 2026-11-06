//! Attachments of messages from other Zones (issue #689, bucket design §21 N18).
//!
//! A remote sender's message is received without binding anything it references
//! (`Arrival::Remote`): a signature proves who wrote the message, not that its
//! author holds what it names, and finding the object here is no proof either.
//! The binding appears only after the bytes came from the sender's Zone and verified:
//!
//! 1. a reader of the message (already authorized on it) misses the binding;
//! 2. this Zone signs in to the sender's Zone as the recipient, and its msg-center
//!    checks the recipient relation and issues a read token for that key
//!    (`msg.attachment_ticket`); a token never travels in a message;
//! 3. `/ndn/<file>?hint={"token":...}` is pulled, every part verified against its id,
//!    local copies notwithstanding;
//! 4. the key is bound and the read proceeds.
//!
//! A message and its key are all the state needed: whatever a crash interrupted
//! is picked up by the next read. Private messages only for now: a group's
//! attachments have a host to authorize them (#689 P2) and stay 404 until then.

use crate::buckets::{attachment_ids, zone_bucket, Buckets};
use crate::group_types::missing;
use crate::msg_center::MessageCenter;
use buckyos_api::bucket_client::{Action, PutRequest, TokenRequest};
use buckyos_api::{
    canonical_zone_of, Principal, METHOD_MSG_ATTACHMENT_TICKET, MSG_CENTER_ATTACHMENTS_BUCKET, MSG_CENTER_SERVICE_NAME,
};
use kRPC::{RPCContext, RPCErrors};
use ndn_lib::{MsgObject, NdnHint, ObjId};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, RPCErrors>;

/// One pull may take many requests; the token outlives it comfortably.
const TICKET_TTL_MS: i64 = 3600 * 1000;
/// A single attachment this Zone agrees to fetch on a reader's behalf.
const MAX_REMOTE_ATTACHMENT_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const PULL_TIMEOUT: Duration = Duration::from_secs(1800);
/// How long a read waits for the pull before answering "try again".
const READ_WAIT: Duration = Duration::from_secs(20);
/// After a failure, reads do not hammer the sender's Zone.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(30);

/// What a read gets from [`MessageCenter::ensure_remote_attachment`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteAttachment {
    /// The key holds the attachment now.
    Bound,
    /// Not a remote attachment: the plain "not found" stands.
    NotRemote,
    /// Being fetched, or the sender's Zone did not answer; the reader may retry.
    Unavailable,
}

#[derive(Default)]
pub(crate) struct RemoteFetches {
    running: Mutex<HashMap<String, watch::Receiver<Option<bool>>>>,
    failed: Mutex<HashMap<String, Instant>>,
}

impl std::fmt::Debug for RemoteFetches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteFetches")
    }
}

#[derive(Deserialize)]
struct AttachmentTicketReq {
    msg_id: ObjId,
    obj_id: ObjId,
}

impl MessageCenter {
    /// `msg.attachment_ticket`, asked by a Zone for one of its users: a recipient of
    /// this private message gets a short read token for that one attachment key.
    /// Anyone else learns nothing, not even that the message exists.
    pub(crate) async fn attachment_ticket(&self, params: Value, ctx: &RPCContext) -> Result<Value> {
        let req: AttachmentTicketReq = serde_json::from_value(params).map_err(|e| RPCErrors::ParseRequestError(e.to_string()))?;
        let caller = self.caller_identity(ctx).await?.ok_or_else(missing)?;
        let recipient = caller.user_did.ok_or_else(missing)?;
        let buckets = self.open_buckets().await?.ok_or_else(|| RPCErrors::ReasonError("bucket service unavailable".into()))?;
        let msg = self.load_message(&req.msg_id).await.map_err(|_| missing())?;
        if Self::is_group_message(&msg) || !msg.to.contains(&recipient) || !req.obj_id.is_file_object() {
            return Err(missing());
        }
        let (key, _, _) = self.bound_attachment(&buckets, &req.msg_id, &req.obj_id).await?.ok_or_else(missing)?;
        let token = TokenRequest {
            prefix: None,
            key: Some(key),
            actions: [Action::Read].into_iter().collect(),
            expires_at: buckyos_api::bucket_client::now_ms() + TICKET_TTL_MS,
            max_uses: None,
            bind_user_id: None,
            bind_app_id: None,
        };
        let issued = buckets.client.create_access_token(&zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET), &token).await?;
        log::info!("attachment ticket of {} for {}", req.msg_id.to_string(), recipient.to_string());
        Ok(json!({"token": issued.token, "expires_at": issued.expires_at}))
    }

    /// The caller may read `msg_id` and its binding does not hold `id` yet.
    pub(crate) async fn ensure_remote_attachment(&self, buckets: &Buckets, msg_id: &ObjId, id: &ObjId) -> RemoteAttachment {
        let Ok(msg) = self.load_message(msg_id).await else { return RemoteAttachment::NotRemote };
        if Self::is_group_message(&msg) || !id.is_file_object() || self.is_local_recipient(&msg.from) || self.cross_zone.get().is_none() {
            return RemoteAttachment::NotRemote;
        }
        let Some(n) = attachment_ids(&msg).iter().position(|x| x == id) else { return RemoteAttachment::NotRemote };
        let key = format!("{}{n}", Self::attachment_prefix(msg_id, &msg));
        let mut progress = {
            let mut running = self.remote_fetches.running.lock().unwrap();
            match running.get(&key) {
                Some(progress) => progress.clone(),
                None => {
                    if self.remote_fetches.failed.lock().unwrap().get(&key).is_some_and(|at| at.elapsed() < RETRY_AFTER_FAILURE) {
                        return RemoteAttachment::Unavailable;
                    }
                    let (done, progress) = watch::channel(None);
                    running.insert(key.clone(), progress.clone());
                    self.spawn_fetch(buckets.clone(), msg, id.clone(), key.clone(), done);
                    progress
                }
            }
        };
        let finished = tokio::time::timeout(READ_WAIT, progress.wait_for(Option::is_some)).await;
        match finished {
            Ok(Ok(outcome)) if *outcome == Some(true) => RemoteAttachment::Bound,
            _ => RemoteAttachment::Unavailable,
        }
    }

    /// The pull client's futures are not Send; a blocking slot runs them to the end,
    /// whether or not the read that started it is still waiting.
    fn spawn_fetch(&self, buckets: Buckets, msg: MsgObject, id: ObjId, key: String, done: watch::Sender<Option<bool>>) {
        let center = self.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let outcome = center.pull_attachment(&buckets, &msg, &id, &key).await;
                if let Err(e) = &outcome {
                    log::warn!("attachment {} from {}: {e}", id.to_string(), msg.from.to_string());
                    center.remote_fetches.failed.lock().unwrap().insert(key.clone(), Instant::now());
                } else {
                    log::info!("attachment {} from {} verified and bound at {key}", id.to_string(), msg.from.to_string());
                    center.remote_fetches.failed.lock().unwrap().remove(&key);
                }
                center.remote_fetches.running.lock().unwrap().remove(&key);
                let _ = done.send(Some(outcome.is_ok()));
            })
        });
    }

    async fn pull_attachment(&self, buckets: &Buckets, msg: &MsgObject, id: &ObjId, key: &str) -> std::result::Result<(), String> {
        let client = self.cross_zone.get().ok_or("no cross-zone client")?;
        let requester = msg.to.iter().find(|to| self.is_local_recipient(to)).ok_or("no recipient of this message lives here")?;
        let zone = canonical_zone_of(self.custody.get().as_ref(), &msg.from, None)
            .await
            .map_err(|e| format!("zone of the sender: {e}"))?
            .ok_or("the sender has no zone")?;
        let host = client.zone_host(&zone).await.map_err(|e| format!("host of the sender's zone: {e}"))?;
        let (msg_id, _) = ndn_lib::NamedObject::gen_obj_id(msg);
        let params = json!({"msg_id": msg_id.to_string(), "obj_id": id.to_string()});
        let principal = Principal::new(requester.clone());
        let ticket = client
            .call(&zone, MSG_CENTER_SERVICE_NAME, &principal, METHOD_MSG_ATTACHMENT_TICKET, params)
            .await
            .map_err(|e| format!("ticket: {e}"))?;
        let token = ticket["token"].as_str().ok_or("the ticket has no token")?.to_string();
        let origin = self.cyfs_dispatch.read().unwrap().origin_of_zone(&host);
        let hint = NdnHint { path: None, token: Some(token) };
        let url = buckyos_api::ndn_read_url(&origin, &id.to_string(), Some(&hint));
        let ndn = ndn_toolkit::CyfsNdnClient::builder().timeout(PULL_TIMEOUT).build().map_err(|e| e.to_string())?;
        let (announced, body) = ndn.get(&url).raw().send().await.map_err(|e| e.to_string())?.object().await.map_err(|e| e.to_string())?;
        if announced != *id {
            return Err(format!("the source served {}", announced.to_string()));
        }
        let size = body["size"].as_u64().ok_or("the file object has no size")?;
        if size > MAX_REMOTE_ATTACHMENT_BYTES {
            return Err(format!("{size} bytes is more than this zone fetches for a reader"));
        }
        ndn.get(&url).pull_to_named_store(&buckets.store).await.map_err(|e| e.to_string())?;
        let put = PutRequest { key: key.to_string(), obj_id: id.to_string(), ..Default::default() };
        buckets.client.put(&zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET), &put).await.map_err(|e| e.to_string())?;
        Ok(())
    }
}
