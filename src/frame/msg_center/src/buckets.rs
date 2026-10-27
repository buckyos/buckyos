//! What msg-center writes to the named store is retained by its buckets
//! (`doc/message_hub/Message Center Durable Data Schema.md`, Storage
//! Strategy). A stored message is bound in `zone/msg-center/_/messages`
//! (`<msg_id>`) and each of its file attachments in
//! `zone/msg-center/_/attachments` (`<session>/<msg_id>/<n>`), in one call,
//! before its records commit; an error fails the accept. A group redaction
//! unbinds both (`purge_message`); a private message has no server-side
//! delete, so it stays bound. An attachment is served only while the binding
//! of the message it is read through holds it, and a user collects one into
//! `user/msg-center/<user>/collected`. The files a hosted group's displays
//! show (EXT-04) are bound under `<group address>/_display/<obj_id>` while a
//! display shows them, and read through `<session address>/_display`.

use crate::group_types::{missing, session_key, GroupState};
use crate::msg_center::MessageCenter;
use buckyos_api::bucket_client::{BucketId, Conditions, ListRequest, Origin, PutRequest};
use buckyos_api::{
    bucket_error_code, get_buckyos_api_runtime, is_no_such_key, is_precondition_failed,
    BucketClient, MailboxAddress, MailboxKind, TokenPrincipalKind, MSG_CENTER_ATTACHMENTS_BUCKET,
    MSG_CENTER_COLLECTED_BUCKET, MSG_CENTER_MESSAGES_BUCKET, MSG_CENTER_SERVICE_NAME,
};
use kRPC::{RPCContext, RPCErrors};
use log::warn;
use named_store::NamedDataMgr;
use ndn_lib::{MsgObject, NdnError, ObjId, RefTarget};
use serde::Deserialize;
use serde_json::{json, Value};
use name_lib::DID;
use sqlx::Row;
use std::collections::BTreeSet;
use std::sync::{Arc, RwLock};

type Result<T> = std::result::Result<T, RPCErrors>;

/// The bucket service and the named store one operation binds through.
#[derive(Clone)]
pub struct Buckets {
    pub client: Arc<BucketClient>,
    pub store: NamedDataMgr,
}

/// The Zone's bucket service (nfs-server) and node store, or fixed ones.
pub enum BucketHost {
    Runtime,
    #[allow(dead_code)]
    Fixed(Buckets),
}

impl BucketHost {
    pub async fn open(&self) -> Result<Buckets> {
        match self {
            Self::Runtime => {
                let runtime = get_buckyos_api_runtime()?;
                Ok(Buckets {
                    client: Arc::new(runtime.get_bucket_client().await?),
                    store: runtime.get_local_named_store().await?,
                })
            }
            Self::Fixed(buckets) => Ok(buckets.clone()),
        }
    }

    pub async fn store(&self) -> Result<NamedDataMgr> {
        match self {
            Self::Runtime => get_buckyos_api_runtime()?.get_local_named_store().await,
            Self::Fixed(buckets) => Ok(buckets.store.clone()),
        }
    }
}

/// Unset in unit tests that store nothing; the service sets the runtime host
/// at startup.
#[derive(Default)]
pub struct BucketSlot(RwLock<Option<Arc<BucketHost>>>);

impl BucketSlot {
    pub fn get(&self) -> Option<Arc<BucketHost>> {
        self.0.read().unwrap().clone()
    }

    pub fn set(&self, host: BucketHost) {
        *self.0.write().unwrap() = Some(Arc::new(host));
    }
}

impl std::fmt::Debug for BucketSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("BucketSlot")
    }
}

/// Where a message comes from. A remote sender's attachments may exist only
/// in its own Zone; a local message's must be here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arrival {
    Local,
    Remote,
}

pub(crate) fn zone_bucket(name: &str) -> BucketId {
    BucketId::zone(MSG_CENTER_SERVICE_NAME, name).expect("msg-center zone bucket id")
}

/// Objects a message references (`DataObj` refs), each once, in order; the
/// position is the `<n>` of the attachment key.
pub fn attachment_ids(msg: &MsgObject) -> Vec<ObjId> {
    let mut ids: Vec<ObjId> = Vec::new();
    for r in &msg.content.refs {
        if let RefTarget::DataObj { obj_id, .. } = &r.target {
            if !ids.contains(obj_id) {
                ids.push(obj_id.clone());
            }
        }
    }
    ids
}

/// A data bucket binds files and directories; other referenced objects are
/// not attachments msg-center keeps.
fn kept(id: &ObjId) -> bool {
    id.is_file_object() || id.is_dir_object()
}

fn bucket_error(e: impl std::fmt::Display) -> RPCErrors {
    RPCErrors::ReasonError(e.to_string())
}

/// The name a collected attachment gets: the label its message gave it.
fn collected_name(msg: &MsgObject, id: &ObjId) -> String {
    let label = msg
        .content
        .refs
        .iter()
        .find_map(|r| match &r.target {
            RefTarget::DataObj { obj_id, .. } if obj_id == id => r.label.clone(),
            _ => None,
        })
        .unwrap_or_default();
    let name: String = label
        .trim()
        .chars()
        .take(200)
        .map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c })
        .collect();
    if name.is_empty() || name == "." || name == ".." {
        "attachment".to_string()
    } else {
        name
    }
}

#[derive(Deserialize)]
struct CollectAttachmentReq {
    obj_id: ObjId,
    context_path: String,
}

impl MessageCenter {
    pub fn set_bucket_host(&self, host: BucketHost) {
        self.buckets.set(host);
    }

    pub(crate) fn bucket_host(&self) -> Option<Arc<BucketHost>> {
        self.buckets.get()
    }

    pub(crate) async fn open_buckets(&self) -> Result<Option<Buckets>> {
        match self.bucket_host() {
            Some(host) => Ok(Some(host.open().await?)),
            None => Ok(None),
        }
    }

    /// The node named store.
    pub(crate) async fn named_store(&self) -> Result<NamedDataMgr> {
        match self.bucket_host() {
            Some(host) => host.store().await,
            None => get_buckyos_api_runtime()?.get_local_named_store().await,
        }
    }

    /// `<session>/<msg_id>/`: the session is the message's own address, its
    /// group Session or its author's Session (the SENT mailbox).
    pub(crate) fn attachment_prefix(msg_id: &ObjId, msg: &MsgObject) -> String {
        let session = if Self::is_group_message(msg) {
            msg.to
                .first()
                .and_then(|group| session_key(group, msg.to_session.as_deref()).ok())
        } else {
            MailboxAddress::new(
                msg.from.clone(),
                Self::derive_session_id(&MailboxKind::Sent, msg),
            )
            .ok()
            .map(|address| address.to_string())
        };
        format!(
            "{}/{}/",
            session.unwrap_or_else(|| msg.from.to_string()),
            msg_id.to_string()
        )
    }

    /// Stores a message and binds it and its attachments; called before its
    /// records commit, an error fails the accept.
    pub(crate) async fn store_message(
        &self,
        msg_id: &ObjId,
        msg_json: &str,
        msg: &MsgObject,
        arrival: Arrival,
    ) -> Result<()> {
        let Some(buckets) = self.open_buckets().await? else {
            return Ok(());
        };
        Self::bind_attachments_with(&buckets, msg_id, msg, arrival).await?;
        buckets
            .client
            .put_object_str(
                &buckets.store,
                &zone_bucket(MSG_CENTER_MESSAGES_BUCKET),
                &msg_id.to_string(),
                msg_id,
                msg_json,
                Conditions::default(),
            )
            .await?;
        Ok(())
    }

    /// Binds the attachments of a message whose body lives elsewhere (a
    /// hosted group keeps it in RDB).
    pub(crate) async fn bind_attachments(
        &self,
        msg_id: &ObjId,
        msg: &MsgObject,
        arrival: Arrival,
    ) -> Result<()> {
        match self.open_buckets().await? {
            Some(buckets) => Self::bind_attachments_with(&buckets, msg_id, msg, arrival).await,
            None => Ok(()),
        }
    }

    async fn bind_attachments_with(
        buckets: &Buckets,
        msg_id: &ObjId,
        msg: &MsgObject,
        arrival: Arrival,
    ) -> Result<()> {
        let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
        let prefix = Self::attachment_prefix(msg_id, msg);
        for (n, id) in attachment_ids(msg).iter().enumerate() {
            if !kept(id) {
                continue;
            }
            let req = PutRequest {
                key: format!("{prefix}{n}"),
                obj_id: id.to_string(),
                ..Default::default()
            };
            match buckets.client.put(&bucket, &req).await {
                Ok(_) => {}
                Err(e)
                    if arrival == Arrival::Remote
                        && bucket_error_code(&e) == Some("content_not_available") =>
                {
                    warn!(
                        "attachment {} of remote message {} is not in this zone; nothing to keep",
                        id.to_string(),
                        msg_id.to_string()
                    );
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }

    /// A message that was not accepted gives up the attachment keys it took.
    pub(crate) async fn unbind_attachments(&self, msg_id: &ObjId, msg: &MsgObject) {
        let buckets = match self.open_buckets().await {
            Ok(Some(buckets)) => buckets,
            Ok(None) => return,
            Err(e) => {
                warn!("unbind attachments of rejected {}: {}", msg_id.to_string(), e);
                return;
            }
        };
        let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
        let prefix = Self::attachment_prefix(msg_id, msg);
        for (n, id) in attachment_ids(msg).iter().enumerate() {
            if !kept(id) {
                continue;
            }
            if let Err(e) = buckets.client.delete(&bucket, &format!("{prefix}{n}"), None).await {
                warn!("unbind attachment {prefix}{n} of rejected message: {e}");
            }
        }
    }

    /// Unbinds a redacted message and its attachments (keys under `prefix`,
    /// else derived from the stored message), then removes the message from
    /// the named store.
    pub(crate) async fn purge_message(
        &self,
        buckets: &Buckets,
        msg_id: &ObjId,
        prefix: Option<String>,
    ) -> Result<()> {
        let prefix = match prefix {
            Some(prefix) => Some(prefix),
            None => match buckets.store.get_object(msg_id).await {
                Ok(body) => serde_json::from_str::<MsgObject>(&body)
                    .ok()
                    .map(|msg| Self::attachment_prefix(msg_id, &msg)),
                Err(NdnError::NotFound(_)) => None,
                Err(e) => return Err(bucket_error(e)),
            },
        };
        if let Some(prefix) = prefix {
            let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
            let listed = buckets
                .client
                .list_objects(&bucket, &ListRequest { prefix, ..Default::default() })
                .await?;
            for item in listed.items {
                buckets.client.delete(&bucket, &item.key, None).await?;
            }
        }
        buckets
            .client
            .delete(&zone_bucket(MSG_CENTER_MESSAGES_BUCKET), &msg_id.to_string(), None)
            .await?;
        match buckets.store.remove_object(msg_id).await {
            Ok(_) | Err(NdnError::NotFound(_)) => Ok(()),
            Err(e) => Err(bucket_error(e)),
        }
    }

    /// The `attachments` key under which message `msg_id` keeps `id`, with the
    /// message and the position, when that binding holds `id`.
    pub(crate) async fn bound_attachment(
        &self,
        buckets: &Buckets,
        msg_id: &ObjId,
        id: &ObjId,
    ) -> Result<Option<(String, MsgObject, usize)>> {
        let msg = self.load_message(msg_id).await?;
        let Some(n) = attachment_ids(&msg).iter().position(|x| x == id) else {
            return Ok(None);
        };
        let key = format!("{}{n}", Self::attachment_prefix(msg_id, &msg));
        Ok(Self::key_holds(buckets, &key, id).await?.then_some((key, msg, n)))
    }

    /// The message `context_path` (`[<session address>/]<msg_id>`) names,
    /// once the caller may read `id` through it: a message of a hosted group
    /// Session that references `id`, or a message of the caller's mailboxes.
    pub(crate) async fn authorize_attachment(
        &self,
        ctx: &RPCContext,
        id: &ObjId,
        context_path: &str,
    ) -> Result<ObjId> {
        let msg_id = context_msg_id(context_path)?;
        if context_path.contains('/') {
            self.authorize_group_attachment(&self.group_actor(ctx).await?, id, context_path)
                .await?;
        } else {
            self.authorize_message(ctx, &msg_id).await?;
        }
        Ok(msg_id)
    }

    /// `msg.collect_attachment`: copies an attachment the calling user reads
    /// into that user's `collected` bucket, recording its sender and message
    /// as origin; collecting it again returns the same key.
    pub(crate) async fn collect_attachment(&self, params: Value, ctx: &RPCContext) -> Result<Value> {
        let req: CollectAttachmentReq = serde_json::from_value(params)
            .map_err(|e| RPCErrors::ParseRequestError(e.to_string()))?;
        let caller = self
            .caller_identity(ctx)
            .await?
            .filter(|caller| caller.principal_kind == TokenPrincipalKind::User && !caller.external)
            .ok_or_else(|| RPCErrors::NoPermission("collect-requires-a-zone-user".into()))?;
        let msg_id = self.authorize_attachment(ctx, &req.obj_id, &req.context_path).await?;
        let buckets = self
            .open_buckets()
            .await?
            .ok_or_else(|| RPCErrors::ReasonError("bucket service unavailable".into()))?;
        let (src_key, msg, n) = self
            .bound_attachment(&buckets, &msg_id, &req.obj_id)
            .await?
            .ok_or_else(missing)?;
        let dst = BucketId::user(MSG_CENTER_SERVICE_NAME, &caller.user_id, MSG_CENTER_COLLECTED_BUCKET)
            .map_err(bucket_error)?;
        let dst_key = format!("{}/{}/{}", msg_id.to_string(), n, collected_name(&msg, &req.obj_id));
        let origin = Origin {
            author: msg.from.to_string(),
            source: format!("cyfs://{}", msg_id.to_string()),
            collected_at: Self::now_ms() as i64,
            proof: None,
        };
        match buckets
            .client
            .copy_to(
                (&zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET), &src_key),
                (&dst, &dst_key),
                Some(origin),
                Conditions::create_only(),
            )
            .await
        {
            Ok(_) => {}
            Err(e) if is_precondition_failed(&e) => {}
            Err(e) => return Err(e),
        }
        Ok(json!({"bucket_id": dst.to_string(), "key": dst_key}))
    }

    /// Authorize user-originated references before anything is stored or
    /// bound. Signed remote ingress and trusted in-process sends use their
    /// existing path.
    pub(crate) async fn authorize_outgoing_attachments(&self, ctx: &RPCContext, msg: &MsgObject) -> Result<()> {
        let ids = attachment_ids(msg);
        if ids.is_empty() {
            return Ok(());
        }
        let Some(caller) = self.caller_identity(ctx).await? else { return Ok(()) };
        Self::refuse_guest_attachments(Some(&caller), msg)?;
        let token = ctx.token.as_deref().ok_or_else(|| RPCErrors::NoPermission("attachment token required".into()))?;
        for id in ids {
            if matches!(self.mailbox_references(ctx, &id).await, Ok(true)) {
                continue;
            }
            // A group reference may be reused only while its original message
            // remains readable to this caller (membership, history and recall).
            let sql = self.msg_box_db.render_sql("SELECT obj_id,body_json FROM group_objects WHERE body_json LIKE ?");
            let rows = sqlx::query(&sql)
                .bind(format!("%{}%", id.to_string()))
                .fetch_all(self.msg_box_db.pool())
                .await
                .map_err(crate::group_store::db_error)?;
            let mut readable = false;
            for row in rows {
                let body: String = row.try_get("body_json").map_err(crate::group_store::db_error)?;
                let source: MsgObject = serde_json::from_str(&body).map_err(crate::group_store::db_error)?;
                if !Self::message_references(&source, &id) {
                    continue;
                }
                let raw: String = row.try_get("obj_id").map_err(crate::group_store::db_error)?;
                let source_id = ObjId::new(&raw).map_err(crate::group_store::db_error)?;
                if self.authorize_group_message(&self.group_actor(ctx).await?, &source_id).await.is_ok() {
                    readable = true;
                    break;
                }
            }
            if readable {
                continue;
            }
            if !self.token_verifier.get().authorize_imported_attachment(token, &id).await? {
                return Err(RPCErrors::NoPermission("attachment is not readable by this caller".into()));
            }
        }
        Ok(())
    }

    /// A message of the caller's mailboxes that the caller may read references `id`.
    async fn mailbox_references(&self, ctx: &RPCContext, id: &ObjId) -> Result<bool> {
        let actor = self.group_actor(ctx).await?;
        let sql = self.msg_box_db.render_sql("SELECT msg_id FROM msg_refs WHERE owner=?");
        for row in sqlx::query(&sql)
            .bind(actor.did.to_string())
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(crate::group_store::db_error)?
        {
            let raw: String = row.try_get("msg_id").map_err(crate::group_store::db_error)?;
            let msg_id = ObjId::new(&raw).map_err(crate::group_store::db_error)?;
            if self.authorize_message(ctx, &msg_id).await.is_err() {
                continue;
            }
            if let Ok(message) = self.load_message(&msg_id).await {
                if Self::message_references(&message, id) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }
}

/// The segment of display keys and read contexts; `_`-prefixed Session ids
/// are reserved, so no Session address collides with it.
const DISPLAY_SEGMENT: &str = "_display";

/// `<group address>/_display/`: where a group's display files are bound.
pub(crate) fn display_prefix(group: &DID) -> Result<String> {
    Ok(format!("{}/{DISPLAY_SEGMENT}/", session_key(group, None)?))
}

/// The Session address of a display read context `<session address>/_display`.
pub(crate) fn display_context(context_path: &str) -> Option<&str> {
    context_path
        .strip_suffix(DISPLAY_SEGMENT)
        .and_then(|session| session.strip_suffix('/'))
}

/// Files the displays of `g` show to the readers of its Sessions.
pub(crate) fn display_files(g: &GroupState) -> Result<BTreeSet<String>> {
    let mut files = BTreeSet::new();
    if g.lifecycle == "deleted" {
        return Ok(files);
    }
    let group = g.config()?.display;
    let live = g
        .sessions
        .iter()
        .filter(|(_, r)| r.lifecycle != "deleted")
        .map(|(sid, _)| Some(sid.as_str()));
    for sid in std::iter::once(None).chain(live) {
        let Ok(rules) = g.rules(sid) else { continue };
        for file in rules.display.over(&group).files {
            if kept(&file.obj_id) {
                files.insert(file.obj_id.to_string());
            }
        }
    }
    Ok(files)
}

impl MessageCenter {
    /// The objects bound under `<group address>/_display/`.
    async fn bound_display_files(buckets: &Buckets, prefix: &str) -> Result<BTreeSet<String>> {
        let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
        let mut bound = BTreeSet::new();
        let mut start_after = None;
        loop {
            let req = ListRequest { prefix: prefix.to_string(), start_after, ..Default::default() };
            let page = buckets.client.list_objects(&bucket, &req).await?;
            bound.extend(page.items.iter().filter_map(|b| b.key.strip_prefix(prefix).map(str::to_owned)));
            match page.next_start_after {
                Some(next) => start_after = Some(next),
                None => return Ok(bound),
            }
        }
    }

    /// Binds the display files of `group` not bound yet; before the change
    /// that shows them commits, an error fails it.
    pub(crate) async fn bind_display_files(&self, group: &DID, files: &BTreeSet<String>) -> Result<()> {
        let Some(buckets) = self.open_buckets().await? else {
            return Ok(());
        };
        let prefix = display_prefix(group)?;
        let bound = Self::bound_display_files(&buckets, &prefix).await?;
        let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
        for id in files.difference(&bound) {
            let req = PutRequest { key: format!("{prefix}{id}"), obj_id: id.clone(), ..Default::default() };
            buckets.client.put(&bucket, &req).await?;
        }
        Ok(())
    }

    /// Unbinds the display files of `group` no display shows any more; after
    /// the change commits, a failure is retried by the next change.
    pub(crate) async fn unbind_display_files(&self, group: &DID, files: &BTreeSet<String>) {
        let result: Result<()> = async {
            let Some(buckets) = self.open_buckets().await? else {
                return Ok(());
            };
            let prefix = display_prefix(group)?;
            let bucket = zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET);
            for id in Self::bound_display_files(&buckets, &prefix).await?.difference(files) {
                buckets.client.delete(&bucket, &format!("{prefix}{id}"), None).await?;
            }
            Ok(())
        }
        .await;
        if let Err(e) = result {
            warn!("unbind display files of {}: {}", group.to_string(), e);
        }
    }

    /// Whether `key` of `attachments` is bound to `id`.
    pub(crate) async fn key_holds(buckets: &Buckets, key: &str, id: &ObjId) -> Result<bool> {
        match buckets
            .client
            .get(&zone_bucket(MSG_CENTER_ATTACHMENTS_BUCKET), key, None)
            .await
        {
            Ok(binding) => Ok(binding.obj_id == Some(id.to_string())),
            Err(e) if is_no_such_key(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }
}

/// The message of a `context_path`: its last segment.
pub(crate) fn context_msg_id(context_path: &str) -> Result<ObjId> {
    let raw = context_path.rsplit('/').next().unwrap_or(context_path);
    ObjId::new(raw).map_err(|_| missing())
}
