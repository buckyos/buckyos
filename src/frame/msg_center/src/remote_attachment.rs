//! Attachments of messages from other Zones (issue #689, bucket design §21 N18).
//!
//! A remote sender's message is received without binding anything it references
//! (`Arrival::Remote`): a signature proves who wrote the message, not that its
//! author holds what it names, and finding the object here is no proof either.
//! The binding appears only after the bytes came from a side entitled to them and
//! verified:
//!
//! 1. a reader of the message (already authorized on it) misses the binding;
//! 2. this Zone signs in to the source Zone and asks for a read token for that key
//!    (`msg.attachment_ticket`); a token never travels in a message;
//! 3. `/ndn/<file>?hint={"token":...}` is pulled, every part verified against its
//!    id, local copies notwithstanding;
//! 4. the key is bound and the read proceeds.
//!
//! The source and whom this Zone asks as depend on the message:
//!
//! - a private message: the sender's Zone, as the reader;
//! - a group this Zone hosts: the author's Zone, as the group (its document's owner
//!   places it here), right after the message is accepted and again on a read;
//! - a group joined from here: the host, as the reader, whom the host judges by the
//!   group's rules. The host fetches what it does not have yet, so no member's Zone
//!   ever looks for the author.
//!
//! A description (`describe`) needs only the verified FileObject: the source's copy,
//! never a local one (a message naming someone's private object must not learn its
//! name), and nothing is bound. Only `content` pulls.
//!
//! A message and its key are all the state needed: whatever a crash interrupted
//! is picked up by the next read.

use crate::buckets::{attachment_ids, context_msg_id, display_context, display_prefix, zone_bucket, Buckets};
use crate::group_types::{missing, session_key, GroupActor};
use crate::msg_center::MessageCenter;
use buckyos_api::bucket_client::{Action, PutRequest, TokenRequest};
use buckyos_api::{
    canonical_zone_of, CrossZoneError, MailboxAddress, MailboxKind, Principal, METHOD_MSG_ATTACHMENT_TICKET,
    MSG_CENTER_ATTACHMENTS_BUCKET, MSG_CENTER_SERVICE_NAME,
};
use kRPC::{RPCContext, RPCErrorDetail, RPCErrors};
use name_lib::DID;
use ndn_lib::{MsgObject, NdnError, NdnHint, ObjId};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::watch;

type Result<T> = std::result::Result<T, RPCErrors>;

/// One pull may take many requests; the token outlives it comfortably.
const TICKET_TTL_MS: i64 = 3600 * 1000;
/// What a read can hand over: anything bigger is not fetched only to end in 413.
const MAX_REMOTE_ATTACHMENT_BYTES: u64 = crate::object_access::MAX_CONTENT_BYTES;
const PULL_TIMEOUT: Duration = Duration::from_secs(1800);
/// How long a read waits for the pull before answering "try again".
const READ_WAIT: Duration = Duration::from_secs(20);
/// How long a ticket request waits for the host's own pull: the Zone asking is on its own clock.
const TICKET_WAIT: Duration = Duration::from_secs(3);
/// After a failure, reads do not hammer the source.
const RETRY_AFTER_FAILURE: Duration = Duration::from_secs(30);
const RETRY_WHILE_FETCHING_SECS: u64 = 5;

/// `msg.attachment_ticket` said "not yet": the source is fetching it itself.
const TICKET_PENDING: &str = "attachment_pending";
/// `msg.attachment_ticket` said "not now": the source could not answer or reach its own source.
const TICKET_UNAVAILABLE: &str = "attachment_unavailable";

/// What a read gets from [`MessageCenter::ensure_remote_attachment`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RemoteAttachment {
    /// The key holds the attachment now.
    Bound,
    /// Not an attachment fetched from elsewhere: the plain "not found" stands.
    NotRemote,
    /// More than a read can hand over; no bytes were fetched.
    TooLarge,
    /// A directory: not pulled across Zones yet.
    Directory,
    /// The reader has not accepted the sender: nothing went out.
    AwaitingAcceptance,
    /// The source refused, or does not have it.
    Refused,
    /// What came from the source does not match its id or declared size.
    Corrupt,
    /// On its way; the reader may retry soon.
    Fetching,
    /// The source did not answer, or the pull could not be kept; the reader may retry later.
    Unreachable,
}

impl RemoteAttachment {
    /// When a reader should ask again; `None` when waiting changes nothing.
    pub(crate) fn retry_after_secs(self) -> Option<u64> {
        match self {
            Self::Fetching => Some(RETRY_WHILE_FETCHING_SECS),
            Self::Unreachable => Some(RETRY_AFTER_FAILURE.as_secs()),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(crate) struct RemoteFetches {
    running: Mutex<HashMap<String, watch::Receiver<Option<RemoteAttachment>>>>,
    failed: Mutex<HashMap<String, (Instant, RemoteAttachment)>>,
}

impl std::fmt::Debug for RemoteFetches {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RemoteFetches")
    }
}

impl RemoteFetches {
    /// Tests do not sit out the back-off.
    #[cfg(test)]
    pub(crate) fn forget_failures(&self) {
        self.failed.lock().unwrap().clear();
    }

    fn recent_failure(&self, key: &str) -> Option<RemoteAttachment> {
        let failed = self.failed.lock().unwrap();
        failed.get(key).filter(|(at, _)| at.elapsed() < RETRY_AFTER_FAILURE).map(|(_, outcome)| *outcome)
    }

    /// Keeps a failure for its back-off; what is still on its way is not one.
    fn record(&self, key: &str, outcome: RemoteAttachment) {
        let mut failed = self.failed.lock().unwrap();
        if matches!(outcome, RemoteAttachment::Bound | RemoteAttachment::Fetching) {
            failed.remove(key);
        } else {
            failed.retain(|_, (at, _)| at.elapsed() < RETRY_AFTER_FAILURE);
            failed.insert(key.to_string(), (Instant::now(), outcome));
        }
    }
}

#[derive(Debug, Clone)]
enum Upstream {
    Zone(DID),
    Host(String),
}

/// One attachment to get: where from, as whom, and the key that keeps it.
#[derive(Debug, Clone)]
pub(crate) struct Fetch {
    id: ObjId,
    pub(crate) key: String,
    /// The message whose recall ends it; a group's display file has none.
    msg_id: Option<ObjId>,
    principal: DID,
    upstream: Upstream,
    ticket_params: Value,
}

#[derive(Deserialize)]
struct AttachmentTicketReq {
    obj_id: ObjId,
    #[serde(default)]
    msg_id: Option<ObjId>,
    /// `<session address>/<msg_id>` or `<session address>/_display`: a Zone asks the
    /// group's host for one of its members, whom the group's rules judge.
    #[serde(default)]
    context_path: Option<String>,
}

fn ticket_not_ready(code: &str) -> RPCErrors {
    RPCErrors::Detailed(RPCErrorDetail::new(code, code, None))
}

/// The attachment cannot be used yet; the same request may succeed later.
pub(crate) fn attachment_not_ready() -> RPCErrors {
    ticket_not_ready(TICKET_PENDING)
}

fn ticket_failure(error: &CrossZoneError) -> RemoteAttachment {
    if let CrossZoneError::Target(RPCErrors::Detailed(detail)) = error {
        match detail.code.as_str() {
            TICKET_PENDING => return RemoteAttachment::Fetching,
            TICKET_UNAVAILABLE => return RemoteAttachment::Unreachable,
            _ => {}
        }
    }
    if error.is_retryable() {
        RemoteAttachment::Unreachable
    } else {
        RemoteAttachment::Refused
    }
}

fn pull_failure(error: &NdnError) -> RemoteAttachment {
    match error {
        NdnError::TooLarge(_) => RemoteAttachment::TooLarge,
        NdnError::NotFound(_) => RemoteAttachment::Refused,
        NdnError::VerifyError(_)
        | NdnError::InvalidData(_)
        | NdnError::InvalidId(_)
        | NdnError::InvalidObjType(_)
        | NdnError::InvalidLink(_)
        | NdnError::DecodeError(_) => RemoteAttachment::Corrupt,
        _ => RemoteAttachment::Unreachable,
    }
}

impl MessageCenter {
    /// `msg.attachment_ticket`, asked by a Zone: a short read token for one attachment key.
    /// A recipient of a private message, or the group a message was addressed to, asks by
    /// the message; a member's Zone asks the group's host by `context_path`. Anyone else
    /// learns nothing, not even that the message exists.
    pub(crate) async fn attachment_ticket(&self, params: Value, ctx: &RPCContext) -> Result<Value> {
        let req: AttachmentTicketReq = serde_json::from_value(params).map_err(|e| RPCErrors::ParseRequestError(e.to_string()))?;
        let caller = self.caller_identity(ctx).await?.ok_or_else(missing)?;
        let requester = caller.user_did.ok_or_else(missing)?;
        let buckets = self.open_buckets().await?.ok_or_else(|| ticket_not_ready(TICKET_UNAVAILABLE))?;
        let key = match req.context_path.as_deref() {
            Some(path) => self.member_ticket_key(&buckets, ctx, &req.obj_id, path).await?,
            None => {
                let msg_id = req.msg_id.as_ref().ok_or_else(missing)?;
                self.recipient_ticket_key(&buckets, &requester, msg_id, &req.obj_id).await?
            }
        };
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
        log::info!("attachment ticket of {} for {}", req.obj_id.to_string(), requester.to_string());
        Ok(json!({"token": issued.token, "expires_at": issued.expires_at}))
    }

    /// The requester is in `msg.to` and the message's binding holds the file. The sender
    /// may have blocked a recipient since: that reads the same as no such message.
    async fn recipient_ticket_key(&self, buckets: &Buckets, requester: &DID, msg_id: &ObjId, id: &ObjId) -> Result<String> {
        let msg = self.load_message(msg_id).await.map_err(|_| missing())?;
        if !msg.to.contains(requester) || !id.is_file_object() {
            return Err(missing());
        }
        if !Self::is_group_message(&msg) && self.is_contact_blocked(requester, Some(msg.from.clone())).await.unwrap_or(true) {
            return Err(missing());
        }
        self.bound_key(buckets, msg_id, id).await?.ok_or_else(missing)
    }

    /// The host's decision for a member of another Zone: what it would decide for a read here.
    /// What it does not hold yet it fetches now, and the member's Zone asks again.
    async fn member_ticket_key(&self, buckets: &Buckets, ctx: &RPCContext, id: &ObjId, path: &str) -> Result<String> {
        let actor = self.federated_group_actor(ctx).await.map_err(|_| missing())?;
        if let Some(session) = display_context(path) {
            let key = self.authorize_display_file(&actor, id, session).await.map_err(|_| missing())?;
            return match Self::key_holds(buckets, &key, id).await {
                Ok(true) => Ok(key),
                Ok(false) => Err(missing()),
                Err(_) => Err(ticket_not_ready(TICKET_UNAVAILABLE)),
            };
        }
        self.authorize_group_attachment(&actor, id, path).await.map_err(|_| missing())?;
        if !id.is_file_object() {
            return Err(missing());
        }
        let msg_id = context_msg_id(path)?;
        if let Some(key) = self.bound_key(buckets, &msg_id, id).await? {
            return Ok(key);
        }
        match self.ensure_within(buckets, &msg_id, id, None, TICKET_WAIT).await {
            RemoteAttachment::Bound => self.bound_key(buckets, &msg_id, id).await?.ok_or_else(missing),
            RemoteAttachment::Fetching => Err(ticket_not_ready(TICKET_PENDING)),
            RemoteAttachment::Unreachable => Err(ticket_not_ready(TICKET_UNAVAILABLE)),
            _ => Err(missing()),
        }
    }

    async fn bound_key(&self, buckets: &Buckets, msg_id: &ObjId, id: &ObjId) -> Result<Option<String>> {
        let found = self.bound_attachment(buckets, msg_id, id).await.map_err(|_| ticket_not_ready(TICKET_UNAVAILABLE))?;
        Ok(found.map(|(key, _, _)| key))
    }

    /// The caller may read `msg_id` (as `reader`, the mailbox it reads it through) and the
    /// message's binding does not hold `id` yet.
    pub(crate) async fn ensure_remote_attachment(&self, buckets: &Buckets, msg_id: &ObjId, id: &ObjId, reader: Option<&DID>) -> RemoteAttachment {
        self.ensure_within(buckets, msg_id, id, reader, READ_WAIT).await
    }

    async fn ensure_within(&self, buckets: &Buckets, msg_id: &ObjId, id: &ObjId, reader: Option<&DID>, wait: Duration) -> RemoteAttachment {
        match self.attachment_fetch(msg_id, id, reader).await {
            Ok(fetch) => self.run_fetch(buckets, fetch, wait).await,
            Err(outcome) => outcome,
        }
    }

    /// A description of `id` from the source of message `msg_id`: its verified object, kept
    /// nowhere. The same gates as a pull, and no chunk moves.
    pub(crate) async fn describe_remote_attachment(&self, msg_id: &ObjId, id: &ObjId, reader: Option<&DID>) -> std::result::Result<String, RemoteAttachment> {
        let fetch = self.attachment_fetch(msg_id, id, reader).await?;
        self.describe_fetch(fetch).await
    }

    /// A remote author's attachments are fetched as soon as the message is accepted: an author
    /// who goes offline before anyone reads must not cost the group its copy. A failure leaves
    /// the message as it is, and the next read tries again.
    pub(crate) fn fetch_accepted_attachments(&self, msg_id: &ObjId, msg: &MsgObject) {
        if self.cross_zone.get().is_none() {
            return;
        }
        for id in attachment_ids(msg).into_iter().filter(ObjId::is_file_object) {
            let center = self.clone();
            let msg_id = msg_id.clone();
            tokio::spawn(async move {
                match center.open_buckets().await {
                    Ok(Some(buckets)) => {
                        center.ensure_within(&buckets, &msg_id, &id, None, Duration::ZERO).await;
                    }
                    Ok(None) => {}
                    Err(e) => log::warn!("attachment {} of accepted {}: {e}", id.to_string(), msg_id.to_string()),
                }
            });
        }
    }

    /// The mailbox owner a caller reads `msg_id` as: whom the sender's policy and the group's
    /// host judge, the caller's Agent included.
    pub(crate) async fn message_reader(&self, ctx: &RPCContext, msg_id: &ObjId) -> Option<DID> {
        for record in self.msg_box_db.list_message_records(msg_id).await.ok()? {
            if record.box_kind != MailboxKind::GroupInbox && self.authorize_mailbox(ctx, &record.mailbox, record.box_kind, "read").await.is_ok() {
                return Some(record.owner);
            }
        }
        None
    }

    async fn attachment_fetch(&self, msg_id: &ObjId, id: &ObjId, reader: Option<&DID>) -> std::result::Result<Fetch, RemoteAttachment> {
        if self.cross_zone.get().is_none() {
            return Err(RemoteAttachment::NotRemote);
        }
        let msg = self.load_message(msg_id).await.map_err(|_| RemoteAttachment::NotRemote)?;
        let n = attachment_ids(&msg).iter().position(|x| x == id).ok_or(RemoteAttachment::NotRemote)?;
        let prefix = Self::attachment_prefix(msg_id, &msg);
        let (principal, upstream, context_path) = if Self::is_group_message(&msg) {
            self.group_source(&msg, reader, prefix.trim_end_matches('/')).await?
        } else {
            self.private_source(&msg, reader).await?
        };
        if id.is_dir_object() {
            return Err(RemoteAttachment::Directory);
        }
        if !id.is_file_object() {
            return Err(RemoteAttachment::NotRemote);
        }
        let ticket_params = match context_path {
            Some(path) => json!({"obj_id": id, "context_path": path}),
            None => json!({"obj_id": id, "msg_id": msg_id}),
        };
        Ok(Fetch { id: id.clone(), key: format!("{prefix}{n}"), msg_id: Some(msg_id.clone()), principal, upstream, ticket_params })
    }

    /// The sender's Zone, as the reader, once the reader takes messages from that sender:
    /// otherwise a card rendering would tell a stranger when and from where it is read.
    /// Which box the message landed in does not count: accepting moves no record.
    async fn private_source(&self, msg: &MsgObject, reader: Option<&DID>) -> std::result::Result<(DID, Upstream, Option<String>), RemoteAttachment> {
        if self.is_hosted_identity(&msg.from) {
            return Err(RemoteAttachment::NotRemote);
        }
        let reader = reader.ok_or(RemoteAttachment::NotRemote)?;
        let principal = msg.to.iter().find(|to| *to == reader).or_else(|| msg.to.iter().find(|to| self.is_local_recipient(to))).ok_or(RemoteAttachment::NotRemote)?;
        match self.decide_inbox_kind(&msg.from, None, principal, None).await {
            Ok(Some(MailboxKind::Inbox)) => {}
            Ok(_) => return Err(RemoteAttachment::AwaitingAcceptance),
            Err(e) => {
                log::warn!("access policy of {} towards {}: {e}", principal.to_string(), msg.from.to_string());
                return Err(RemoteAttachment::Unreachable);
            }
        }
        Ok((principal.clone(), Upstream::Zone(self.zone_of(&msg.from).await?), None))
    }

    /// A hosted group asks the author's Zone as the group; `groups.load` says whether it is
    /// hosted, registration does not survive a restart. A joined one asks the host as the reader.
    async fn group_source(&self, msg: &MsgObject, reader: Option<&DID>, context_path: &str) -> std::result::Result<(DID, Upstream, Option<String>), RemoteAttachment> {
        let group = msg.to.first().ok_or(RemoteAttachment::NotRemote)?;
        match self.groups.load(group).await {
            Ok(Some(_)) => {
                if self.is_hosted_identity(&msg.from) {
                    return Err(RemoteAttachment::NotRemote);
                }
                Ok((group.clone(), Upstream::Zone(self.zone_of(&msg.from).await?), None))
            }
            Ok(None) => {
                let reader = reader.ok_or(RemoteAttachment::NotRemote)?;
                let route = self.joined_group_route(reader, group).await.map_err(|_| RemoteAttachment::NotRemote)?.ok_or(RemoteAttachment::NotRemote)?;
                Ok((reader.clone(), Upstream::Host(route.host), Some(context_path.to_string())))
            }
            Err(e) => {
                log::warn!("group {} not loaded: {e}", group.to_string());
                Err(RemoteAttachment::Unreachable)
            }
        }
    }

    async fn zone_of(&self, did: &DID) -> std::result::Result<DID, RemoteAttachment> {
        match canonical_zone_of(self.custody.get().as_ref(), did, None).await {
            Ok(Some(zone)) => Ok(zone),
            Ok(None) => Err(RemoteAttachment::Refused),
            Err(e) => {
                log::warn!("zone of {}: {e}", did.to_string());
                Err(RemoteAttachment::Unreachable)
            }
        }
    }

    /// One pull per key, whoever else is waiting for it.
    async fn run_fetch(&self, buckets: &Buckets, fetch: Fetch, wait: Duration) -> RemoteAttachment {
        let mut progress = {
            let mut running = self.remote_fetches.running.lock().unwrap();
            match running.get(&fetch.key) {
                Some(progress) => progress.clone(),
                None => {
                    if let Some(outcome) = self.remote_fetches.recent_failure(&fetch.key) {
                        return outcome;
                    }
                    let (done, progress) = watch::channel(None);
                    running.insert(fetch.key.clone(), progress.clone());
                    self.spawn_fetch(buckets.clone(), fetch, done);
                    progress
                }
            }
        };
        let finished = tokio::time::timeout(wait, progress.wait_for(Option::is_some)).await;
        match finished {
            Ok(Ok(outcome)) => outcome.unwrap_or(RemoteAttachment::Fetching),
            _ => RemoteAttachment::Fetching,
        }
    }

    /// The pull client's futures are not Send; a blocking slot runs them to the end,
    /// whether or not the read that started it is still waiting.
    fn spawn_fetch(&self, buckets: Buckets, fetch: Fetch, done: watch::Sender<Option<RemoteAttachment>>) {
        let center = self.clone();
        let runtime = tokio::runtime::Handle::current();
        tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let outcome = center.pull_attachment(&buckets, &fetch).await;
                log::info!("attachment {} at {}: {outcome:?}", fetch.id.to_string(), fetch.key);
                center.remote_fetches.record(&fetch.key, outcome);
                center.remote_fetches.running.lock().unwrap().remove(&fetch.key);
                let _ = done.send(Some(outcome));
            })
        });
    }

    async fn pull_attachment(&self, buckets: &Buckets, fetch: &Fetch) -> RemoteAttachment {
        let url = match self.ticketed_url(fetch).await {
            Ok(url) => url,
            Err(outcome) => return outcome,
        };
        let ndn = match ndn_toolkit::CyfsNdnClient::builder().timeout(PULL_TIMEOUT).build() {
            Ok(ndn) => ndn,
            Err(e) => {
                log::warn!("attachment {}: {e}", fetch.id.to_string());
                return RemoteAttachment::Unreachable;
            }
        };
        if let Err(e) = ndn.get(&url).max_bytes(MAX_REMOTE_ATTACHMENT_BYTES).pull_to_named_store(&buckets.store).await {
            log::warn!("attachment {} from {:?}: {e}", fetch.id.to_string(), fetch.upstream);
            return pull_failure(&e);
        }
        // A recall that came during the pull must not be undone by binding what it removed.
        if self.is_withdrawn(fetch).await {
            return RemoteAttachment::Refused;
        }
        let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
        let put = PutRequest { key: fetch.key.clone(), obj_id: fetch.id.to_string(), ..Default::default() };
        if let Err(e) = buckets.client.put(&bucket, &put).await {
            // Out of quota, among others: not this attachment's fault, and not for good.
            log::warn!("attachment {} not bound at {}: {e}", fetch.id.to_string(), fetch.key);
            return RemoteAttachment::Unreachable;
        }
        if self.is_withdrawn(fetch).await {
            if let Err(e) = buckets.client.delete(&bucket, &fetch.key, None).await {
                log::warn!("{} of a recalled message not unbound: {e}", fetch.key);
            }
            return RemoteAttachment::Refused;
        }
        RemoteAttachment::Bound
    }

    async fn is_withdrawn(&self, fetch: &Fetch) -> bool {
        match &fetch.msg_id {
            Some(msg_id) => matches!(self.groups.object(msg_id).await, Ok(Some((_, None, _)))),
            None => false,
        }
    }

    /// The source's verified object, through a blocking slot as a pull is.
    pub(crate) async fn describe_fetch(&self, fetch: Fetch) -> std::result::Result<String, RemoteAttachment> {
        if let Some(outcome) = self.remote_fetches.recent_failure(&fetch.key) {
            return Err(outcome);
        }
        let center = self.clone();
        let runtime = tokio::runtime::Handle::current();
        let described = tokio::task::spawn_blocking(move || {
            runtime.block_on(async move {
                let described = center.described_object(&fetch).await;
                if let Err(outcome) = described {
                    center.remote_fetches.record(&fetch.key, outcome);
                }
                described
            })
        });
        match tokio::time::timeout(READ_WAIT, described).await {
            Ok(Ok(described)) => described,
            _ => Err(RemoteAttachment::Unreachable),
        }
    }

    async fn described_object(&self, fetch: &Fetch) -> std::result::Result<String, RemoteAttachment> {
        let url = self.ticketed_url(fetch).await?;
        let ndn = ndn_toolkit::CyfsNdnClient::builder().timeout(READ_WAIT).build().map_err(|_| RemoteAttachment::Unreachable)?;
        let failed = |e: NdnError| {
            log::warn!("description of {} from {:?}: {e}", fetch.id.to_string(), fetch.upstream);
            pull_failure(&e)
        };
        let (announced, body) = ndn.get(&url).raw().send().await.map_err(failed)?.object_string().await.map_err(failed)?;
        if announced != fetch.id {
            return Err(RemoteAttachment::Corrupt);
        }
        Ok(body)
    }

    /// Asks the source for a ticket; the URL it opens carries the token, so it is never logged.
    async fn ticketed_url(&self, fetch: &Fetch) -> std::result::Result<String, RemoteAttachment> {
        let client = self.cross_zone.get().ok_or(RemoteAttachment::NotRemote)?;
        let principal = Principal::new(fetch.principal.clone());
        let params = fetch.ticket_params.clone();
        let failed = |e: CrossZoneError| {
            log::warn!("ticket of {} from {:?}: {e}", fetch.id.to_string(), fetch.upstream);
            ticket_failure(&e)
        };
        let (host, ticket) = match &fetch.upstream {
            Upstream::Zone(zone) => {
                let host = client.zone_host(zone).await.map_err(failed)?;
                (host, client.call(zone, MSG_CENTER_SERVICE_NAME, &principal, METHOD_MSG_ATTACHMENT_TICKET, params).await)
            }
            Upstream::Host(host) => (host.clone(), client.call_host(host, MSG_CENTER_SERVICE_NAME, &principal, METHOD_MSG_ATTACHMENT_TICKET, params).await),
        };
        let token = ticket.map_err(failed)?["token"].as_str().ok_or(RemoteAttachment::Unreachable)?.to_string();
        let origin = self.cyfs_dispatch.read().unwrap().origin_of_zone(&host);
        let hint = NdnHint { path: None, token: Some(token) };
        Ok(buckyos_api::ndn_read_url(&origin, &fetch.id.to_string(), Some(&hint)))
    }

    /// A file a joined group's display shows, as `actor` reads it from here: the member is
    /// active and in that Session by what the last sync said, and the host will judge it.
    /// `None`: no such group here for this member.
    pub(crate) async fn joined_display_file(&self, actor: &GroupActor, id: &ObjId, session: &str) -> Option<Fetch> {
        let address = MailboxAddress::try_from(session.to_string()).ok()?;
        let group = address.owner();
        self.cross_zone.get()?;
        if !id.is_file_object() || session_key(group, address.session_id()).ok()? != session {
            return None;
        }
        let route = self.joined_group_route(&actor.did, group).await.ok()??;
        let state = self.joined_groups(&actor.did).await.ok()?.into_iter().find(|state| state.group_did == *group)?;
        if !state.session_cache.iter().any(|item| item["session_id"] == json!(address.session_id())) {
            return None;
        }
        Some(Fetch {
            id: id.clone(),
            key: format!("{}{}", display_prefix(group).ok()?, id.to_string()),
            msg_id: None,
            principal: actor.did.clone(),
            upstream: Upstream::Host(route.host),
            ticket_params: json!({"obj_id": id, "context_path": format!("{session}/_display")}),
        })
    }

    pub(crate) async fn ensure_display_file(&self, buckets: &Buckets, fetch: Fetch) -> RemoteAttachment {
        self.run_fetch(buckets, fetch, READ_WAIT).await
    }
}
