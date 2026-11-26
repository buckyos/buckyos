use crate::group_push::{GroupReaderAck, GroupReaderUpdate, GroupUpdate, GroupUpdateAck};
use crate::group_store::db_error;
use crate::group_types::*;
use crate::msg_center::MessageCenter;
use buckyos_api::{MailboxAddress, MailboxKind, RecipientState};
use name_lib::DID;
use buckyos_api::{CrossZoneRequest, Principal};
use ndn_lib::{CyfsNamedObjectEncoding, MsgObject, NamedObject, ObjId};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

type BindingKey = (String, String);
const STALE_JOINED_STATE: &str = "joined-group-state-changed";

#[derive(Clone, Debug, Default)]
pub(crate) struct KeyedLocks<K>(Arc<Mutex<HashMap<K, Weak<AsyncMutex<()>>>>>);
pub(crate) type BindingLocks = KeyedLocks<BindingKey>;

impl BindingLocks {
    pub(crate) async fn lock(&self, owner: &DID, group: &DID) -> OwnedMutexGuard<()> {
        self.lock_key((owner.to_string(), group.to_string())).await
    }
}

impl<K: Eq + std::hash::Hash> KeyedLocks<K> {
    pub(crate) async fn lock_key(&self, key: K) -> OwnedMutexGuard<()> {
        let lock = {
            let mut locks = self.0.lock().unwrap();
            match locks.get(&key).and_then(Weak::upgrade) {
                Some(lock) => lock,
                None => {
                    locks.retain(|_, lock| lock.strong_count() != 0);
                    let lock = Arc::new(AsyncMutex::new(()));
                    locks.insert(key, Arc::downgrade(&lock));
                    lock
                }
            }
        };
        lock.lock_owned().await
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PendingPulls {
    pending: Arc<Mutex<HashMap<BindingKey, bool>>>,
    pub(crate) fast_commits: Arc<AtomicU64>,
    pub(crate) catchup_runs: Arc<AtomicU64>,
}

struct FetchedMessage {
    key: String,
    seq: u64,
    id: ObjId,
    object: Option<(MsgObject, Option<String>)>,
    accepted_at: u64,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinedGroupRoute {
    pub owner_did: DID,
    pub group_did: DID,
    pub host: String,
    pub upstream: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authorization: Option<String>,
}
impl std::fmt::Debug for JoinedGroupRoute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JoinedGroupRoute")
            .field("owner_did", &self.owner_did)
            .field("group_did", &self.group_did)
            .field("host", &self.host)
            .finish()
    }
}
impl JoinedGroupRoute {
    pub(crate) fn message_route(&self, msg: &MsgObject) -> anyhow::Result<crate::cyfs_dispatch::NativeDispatchRoute> {
        let mailbox = MailboxAddress::new(self.group_did.clone(), msg.to_session.clone())
            .map_err(|error| anyhow::anyhow!(error))?;
        let path = match msg.to_session.as_ref() {
            None => format!("{}/inbox", self.base_path()),
            Some(_) => format!("{}/sessions/{}/inbox", self.base_path(), mailbox.to_string().split_once('/').unwrap().1),
        };
        Ok(crate::cyfs_dispatch::NativeDispatchRoute {
            target: crate::group_http::group_target(&self.host, &path).map_err(|error| anyhow::anyhow!(error.to_string()))?,
            upstream: self.upstream.clone(), authorization: self.authorization.clone(), timeout_ms: 15_000, zone: None,
        })
    }
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.upstream).map_err(invalid)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || self.authorization.as_ref().is_some_and(|value| {
                !value.starts_with("Bearer ") || http::HeaderValue::from_str(value).is_err()
            })
        {
            return Err(invalid("invalid-joined-group-route"));
        }
        if ndn_lib::normalize_cyfs_dispatch_target(&self.host, "/inbox").is_err() {
            return Err(invalid("invalid-group-host"));
        }
        Ok(())
    }
    pub fn base_path(&self) -> String {
        if self.authorization.is_some() {
            format!("/{}", self.group_did.to_string())
        } else {
            format!("/kapi/msg-center/groups/{}", self.group_did.to_string())
        }
    }
    /// A request to the group host as the member: a configured route's own Bearer, else a
    /// session of the host Zone obtained by the member's Zone (`CrossZoneClient`).
    async fn send(&self, center: &MessageCenter, method: reqwest::Method, path_and_query: &str, body: Vec<u8>) -> Result<(http::HeaderMap, Vec<u8>)> {
        self.validate()?;
        let unavailable = |_| kRPC::RPCErrors::ReasonError("remote-group-unavailable".into());
        let timeout = std::time::Duration::from_secs(15);
        let response = match self.authorization.as_deref() {
            Some(authorization) => {
                let url = reqwest::Url::parse(&self.upstream).map_err(invalid)?.join(path_and_query).map_err(invalid)?;
                let client = reqwest::Client::builder().no_proxy().redirect(reqwest::redirect::Policy::none())
                    .build().map_err(db_error)?;
                let request = client.request(method, url).header("host", &self.host)
                    .header("authorization", authorization).timeout(timeout);
                let request = if body.is_empty() { request } else { request.header("content-type", "application/json").body(body) };
                request.send().await.map_err(unavailable)?
            }
            None => {
                let client = center.cross_zone.get().ok_or_else(|| kRPC::RPCErrors::ReasonError("remote-group-unavailable".into()))?;
                let mut request = CrossZoneRequest::new(method, path_and_query).timeout(timeout);
                if !body.is_empty() {
                    request = request.header("content-type", "application/json").body(body);
                }
                let principal = Principal::new(self.owner_did.clone());
                match client.request_host(&self.host, crate::cyfs_dispatch::MSG_CENTER_SERVICE, &principal, request).await {
                    Ok(response) => response,
                    Err(error) if error.is_retryable() => return Err(kRPC::RPCErrors::ReasonError("remote-group-unavailable".into())),
                    Err(error) => {
                        log::warn!("group host {} refused {}: {}", self.host, self.owner_did.to_string(), error);
                        return Err(denied("remote-group-auth-failed"));
                    }
                }
            }
        };
        Self::response(response).await
    }
    async fn response(mut response: reqwest::Response) -> Result<(http::HeaderMap, Vec<u8>)> {
        let unavailable = |_| kRPC::RPCErrors::ReasonError("remote-group-unavailable".into());
        let status = response.status();
        let headers = response.headers().clone();
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(unavailable)? {
            if chunk.len() > (1024_usize * 1024).saturating_sub(bytes.len()) {
                return Err(invalid("group-response-too-large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if !status.is_success() {
            if status.is_server_error() || status == http::StatusCode::TOO_MANY_REQUESTS {
                return Err(kRPC::RPCErrors::ReasonError("remote-group-unavailable".into()));
            }
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            let reason = value["error"].as_str().unwrap_or(match status {
                http::StatusCode::NOT_FOUND => "remote-group-not-found",
                http::StatusCode::UNAUTHORIZED => "remote-group-auth-required",
                _ => "remote-group-auth-failed",
            });
            return Err(denied(reason));
        }
        Ok((headers, bytes))
    }
    pub(crate) async fn put(&self, center: &MessageCenter, path: &str, body: &Value) -> Result<Value> {
        self.put_query(center, path, &[], body).await
    }
    pub(crate) async fn put_query(&self, center: &MessageCenter, path: &str, query: &[(&str, String)], body: &Value) -> Result<Value> {
        let path_and_query = if query.is_empty() { path.to_owned() } else { Self::path_and_query(path, query)? };
        let (_, bytes) = self.send(center, reqwest::Method::PUT, &path_and_query, serde_json::to_vec(body).map_err(invalid)?).await?;
        serde_json::from_slice(&bytes).map_err(|_| invalid("invalid-group-response"))
    }
    pub(crate) async fn fetch(&self, center: &MessageCenter, path: &str, query: &[(&str, String)]) -> Result<Value> {
        let path_and_query = Self::path_and_query(path, query)?;
        let (_, bytes) = self.send(center, reqwest::Method::GET, &path_and_query, vec![]).await?;
        serde_json::from_slice(&bytes).map_err(invalid)
    }
    fn path_and_query(path: &str, query: &[(&str, String)]) -> Result<String> {
        let mut url = reqwest::Url::parse("http://group.invalid").map_err(invalid)?;
        url.set_path(path);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        }
        Ok(format!("{}{}", url.path(), url.query().map(|q| format!("?{q}")).unwrap_or_default()))
    }
    async fn object(
        &self,
        center: &MessageCenter,
        id: &ObjId,
        session: Option<&str>,
    ) -> Result<(MsgObject, Option<String>)> {
        let path = format!("{}/objects/{}", self.base_path(), id.to_string());
        let (headers, bytes) = self.send(center, reqwest::Method::GET, &path, vec![]).await?;
        let encoding = headers.get("content-type")
            .and_then(|h| h.to_str().ok())
            .and_then(CyfsNamedObjectEncoding::from_content_type)
            .ok_or_else(|| invalid("invalid-group-object-encoding"))?;
        self.decode_object(
            center,
            id,
            session,
            encoding,
            String::from_utf8(bytes).map_err(invalid)?,
        )
        .await
    }
    async fn decode_object(
        &self,
        center: &MessageCenter,
        id: &ObjId,
        session: Option<&str>,
        encoding: CyfsNamedObjectEncoding,
        raw: String,
    ) -> Result<(MsgObject, Option<String>)> {
        if raw.len() > 65_536 {
            return Err(invalid("group-object-too-large"));
        }
        let (msg, jwt) = match encoding {
            CyfsNamedObjectEncoding::Json => (
                MsgObject::from_json_value_checked(serde_json::from_str(&raw).map_err(invalid)?)
                    .map_err(invalid)?
                    .0,
                None,
            ),
            CyfsNamedObjectEncoding::Jwt => {
                let signed = crate::cyfs_dispatch::verify_signed_message_with(&raw, center)
                    .await
                    .map_err(|(_, r)| denied(r))?;
                (signed.msg, Some(raw))
            }
        };
        if msg.gen_obj_id().0 != *id || msg.to != [self.group_did.clone()] {
            return Err(invalid("joined-object-identity-mismatch"));
        }
        if msg.to_session.as_deref() != session {
            return Err(invalid("joined-message-session-mismatch"));
        }
        Ok((msg, jwt))
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JoinedGroupState {
    pub schema_version: u32,
    pub owner_did: DID,
    pub group_did: DID,
    pub session_cache: Vec<Value>,
    pub changes_token: Option<String>,
    #[serde(default)]
    pub member_epoch: Option<u32>,
    #[serde(default, skip_serializing_if = "VecDeque::is_empty")]
    pub(crate) recent_push_tokens: VecDeque<String>,
    pub session_seqs: BTreeMap<String, u64>,
    pub last_read_seq: BTreeMap<String, u64>,
    pub reported_read_seq: BTreeMap<String, u64>,
    pub stopped: bool,
    #[serde(default)]
    pub doc_cache: Option<Value>,
}
impl JoinedGroupState {
    fn new(route: &JoinedGroupRoute) -> Self {
        Self {
            schema_version: 1,
            owner_did: route.owner_did.clone(),
            group_did: route.group_did.clone(),
            session_cache: vec![],
            changes_token: None,
            member_epoch: None,
            recent_push_tokens: VecDeque::new(),
            session_seqs: BTreeMap::new(),
            last_read_seq: BTreeMap::new(),
            reported_read_seq: BTreeMap::new(),
            stopped: false,
            doc_cache: None,
        }
    }
}
impl MessageCenter {
    pub(crate) async fn joined_groups(&self, owner: &DID) -> Result<Vec<JoinedGroupState>> {
        let sql = self
            .msg_box_db
            .render_sql("SELECT state_json FROM joined_groups WHERE owner=? ORDER BY group_did");
        sqlx::query(&sql)
            .bind(owner.to_string())
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
            .into_iter()
            .map(|r| {
                let s: String = r.try_get("state_json").map_err(db_error)?;
                serde_json::from_str(&s).map_err(db_error)
            })
            .collect()
    }
    pub(crate) async fn joined_state(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Result<Option<JoinedGroupState>> {
        Ok(self
            .joined_snapshot(owner, group)
            .await?
            .map(|(_, state)| state))
    }

    async fn joined_snapshot(
        &self,
        owner: &DID,
        group: &DID,
    ) -> Result<Option<(String, JoinedGroupState)>> {
        let sql = self
            .msg_box_db
            .render_sql("SELECT state_json FROM joined_groups WHERE owner=? AND group_did=?");
        sqlx::query(&sql)
            .bind(owner.to_string())
            .bind(group.to_string())
            .fetch_optional(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
            .map(|row| {
                let raw: String = row.try_get("state_json").map_err(db_error)?;
                let state = serde_json::from_str(&raw).map_err(db_error)?;
                Ok((raw, state))
            })
            .transpose()
    }

    pub(crate) async fn sync_joined_group(&self, route: &JoinedGroupRoute) -> Result<Value> {
        let outcome = self.pull_joined_group(route, false).await?;
        self.report_joined_reads(route, false).await?;
        Ok(outcome)
    }

    async fn check_joined_route(
        &self,
        route: &JoinedGroupRoute,
        require_binding: bool,
    ) -> Result<()> {
        let valid = if let Some(binding) = self
            .load_group_binding(&route.owner_did, &route.group_did)
            .await?
        {
            binding.active() && binding.route(self) == *route
        } else {
            !require_binding
                || self
                    .configured_group_route(&route.owner_did, &route.group_did)
                    .as_ref()
                    == Some(route)
        };
        if !valid {
            return Err(denied("joined-group-binding-changed"));
        }
        Ok(())
    }

    async fn pull_joined_group(
        &self,
        route: &JoinedGroupRoute,
        require_binding: bool,
    ) -> Result<Value> {
        route.validate()?;
        let mut synced = 0;
        loop {
            let guard = self
                .groups
                .sync_locks
                .lock(&route.owner_did, &route.group_did)
                .await;
            self.check_joined_route(route, require_binding).await?;
            let page = self.pull_joined_page(route).await;
            if matches!(&page, Err(kRPC::RPCErrors::ReasonError(reason)) if reason == STALE_JOINED_STATE)
            {
                continue;
            }
            let (count, stopped, more) = page?;
            synced += count;
            drop(guard);
            if stopped || !more {
                return Ok(json!({"synced":synced, "stopped":stopped}));
            }
            tokio::task::yield_now().await;
        }
    }

    async fn pull_joined_page(&self, route: &JoinedGroupRoute) -> Result<(usize, bool, bool)> {
        let (previous, state) = self
            .joined_snapshot(&route.owner_did, &route.group_did)
            .await?
            .unzip();
        let mut state = state.unwrap_or_else(|| JoinedGroupState::new(route));
        if state.stopped {
            return Ok((0, true, false));
        }
        let mut query = vec![("limit", "100".to_string())];
        if let Some(t) = &state.changes_token {
            query.push(("since", t.clone()));
        }
        let changes = route
            .fetch(self, &format!("{}/changes", route.base_path()), &query)
            .await?;
        let mut pending = vec![];
        let mut lost_session = false;
        let items = changes["items"]
            .as_array()
            .ok_or_else(|| invalid("invalid-group-changes"))?;
        let next = changes["next_token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| invalid("invalid-group-changes-token"))?;
        if items.len() >= 100 && state.changes_token.as_deref() == Some(next) {
            return Err(invalid("non-advancing-group-changes-token"));
        }
        let epoch = changes["member"]["epoch"]
            .as_u64()
            .and_then(|v| u32::try_from(v).ok());
        if state.member_epoch != epoch {
            state.recent_push_tokens.clear();
            state.member_epoch = epoch;
        }
        for change in items {
            let management = change["kind"] != "message";
            if management
                && (change["action"] == "session.deleted"
                    || (change["subject_did"] == json!(route.owner_did)
                        && ["session.member_left", "session.member_removed"]
                            .contains(&change["action"].as_str().unwrap_or(""))))
            {
                state
                    .session_cache
                    .retain(|s| s["session_id"] != change["session_id"]);
                lost_session = true;
            }
            if management
                && (change["action"] == "entity.group_deleted"
                    || (change["session_id"].is_null()
                        && changes["member"]["state"] != "active"
                        && (change["subject_did"] == json!(route.owner_did)
                            || change["subject_did"].is_null())
                        && ["entity.member_left", "entity.member_removed"]
                            .contains(&change["action"].as_str().unwrap_or(""))))
            {
                state.stopped = true;
            }
            if let Some(message) = change
                .get("message")
                .filter(|message| !message["obj_id"].is_null())
            {
                let id = ObjId::new(
                    message["obj_id"]
                        .as_str()
                        .ok_or_else(|| invalid("invalid-group-object-id"))?,
                )
                .map_err(invalid)?;
                let sid = change["session_id"].as_str().map(str::to_owned);
                let key = session_key(&route.group_did, sid.as_deref())?;
                let seq = message["seq"]
                    .as_u64()
                    .ok_or_else(|| invalid("invalid-session-seq"))?;
                if seq > *state.session_seqs.get(&key).unwrap_or(&0) {
                    pending.push((
                        sid,
                        key,
                        seq,
                        id,
                        message["redacted"] == true,
                        message["accepted_at_ms"].as_u64().unwrap_or(seq),
                    ));
                }
            }
        }
        if lost_session
            && state.doc_cache.is_none()
            && state.session_cache.is_empty()
            && state.changes_token.is_some()
        {
            state.stopped = true;
        }
        if state.stopped {
            pending.clear();
            state.session_cache.clear();
        } else {
            let sessions = route
                .fetch(self, &format!("{}/sessions", route.base_path()), &[])
                .await?;
            state.session_cache = sessions["items"]
                .as_array()
                .ok_or_else(|| invalid("invalid-group-session-list"))?
                .clone();
            state.doc_cache = sessions.get("group_doc").filter(|v| !v.is_null()).cloned();
            pending.retain(|(sid, _, _, _, _, _)| {
                state
                    .session_cache
                    .iter()
                    .any(|s| s["session_id"] == json!(sid))
            });
        }
        if !state.stopped && (changes["limited"] == true || state.changes_token.is_none()) {
            for session in &state.session_cache {
                let sid = session["session_id"].as_str().map(str::to_owned);
                let key = session_key(&route.group_did, sid.as_deref())?;
                let inbox = match sid.as_deref() {
                    None => format!("{}/inbox", route.base_path()),
                    Some(_) => format!(
                        "{}/sessions/{}/inbox",
                        route.base_path(),
                        MailboxAddress::new(route.group_did.clone(), sid.clone())
                            .map_err(invalid)?
                            .to_string()
                            .split_once('/')
                            .unwrap()
                            .1
                    ),
                };
                let mut after = *state.session_seqs.get(&key).unwrap_or(&0);
                loop {
                    let page = route
                        .fetch(
                            self, &inbox,
                            &[("after_seq", after.to_string()), ("limit", "100".into())],
                        )
                        .await?;
                    let items = page["items"]
                        .as_array()
                        .ok_or_else(|| invalid("invalid-group-inbox"))?;
                    for item in items {
                        let id = ObjId::new(
                            item["obj_id"]
                                .as_str()
                                .ok_or_else(|| invalid("invalid-object-id"))?,
                        )
                        .map_err(invalid)?;
                        let seq = item["seq"]
                            .as_u64()
                            .ok_or_else(|| invalid("invalid-session-seq"))?;
                        pending.push((
                            sid.clone(),
                            key.clone(),
                            seq,
                            id,
                            item["redacted"] == true,
                            item["accepted_at_ms"].as_u64().unwrap_or(seq),
                        ));
                    }
                    let next = page["next_after_seq"].as_u64().unwrap_or(after);
                    if page["limited"] != true || next <= after {
                        break;
                    }
                    after = next;
                }
            }
        }
        pending.sort_by(|a, b| (&a.1, a.2).cmp(&(&b.1, b.2)));
        pending.dedup_by(|a, b| a.1 == b.1 && a.2 == b.2);
        let mut fetched = vec![];
        for (sid, key, seq, id, redacted, accepted_at) in pending {
            let object = if redacted {
                None
            } else {
                Some(route.object(self, &id, sid.as_deref()).await?)
            };
            fetched.push(FetchedMessage {
                key,
                seq,
                id,
                object,
                accepted_at,
            });
        }
        state.changes_token = Some(next.to_owned());
        self.project_joined_page(route, &mut state, previous.as_deref(), &fetched, true)
            .await?;
        Ok((fetched.len(), state.stopped, items.len() >= 100))
    }

    async fn project_joined_page(
        &self,
        route: &JoinedGroupRoute,
        state: &mut JoinedGroupState,
        previous: Option<&str>,
        fetched: &[FetchedMessage],
        refresh_sessions: bool,
    ) -> Result<()> {
        let mut tx = self.msg_box_db.pool().begin().await.map_err(db_error)?;
        let mut projections = vec![];
        let mut purges = std::collections::BTreeSet::new();
        for FetchedMessage { id, object, .. } in fetched {
            if object.is_none() {
                purges.insert(id.to_string());
            }
            if let Some((msg, _)) = object {
                if let Some(relation) = msg
                    .relates_to
                    .as_ref()
                    .filter(|r| r.rel == ndn_lib::MsgRelType::Redact)
                {
                    purges.insert(relation.target.to_string());
                }
            }
        }
        for FetchedMessage { id, object, .. } in fetched {
            if object.as_ref().is_some_and(|(msg, _)| {
                msg.relates_to.as_ref().is_some_and(|r| {
                    matches!(
                        r.rel,
                        ndn_lib::MsgRelType::Edit | ndn_lib::MsgRelType::Reaction
                    ) && purges.contains(&r.target.to_string())
                })
            }) {
                purges.insert(id.to_string());
            }
        }
        if !purges.is_empty() {
            let sql=self.msg_box_db.render_sql("SELECT obj_id,body_json FROM group_objects WHERE group_did=? AND body_json IS NOT NULL");
            for row in sqlx::query(&sql)
                .bind(route.group_did.to_string())
                .fetch_all(&mut *tx)
                .await
                .map_err(db_error)?
            {
                let body: String = row.try_get("body_json").map_err(db_error)?;
                if let Ok(msg) = serde_json::from_str::<MsgObject>(&body) {
                    if msg.relates_to.as_ref().is_some_and(|r| {
                        matches!(
                            r.rel,
                            ndn_lib::MsgRelType::Edit | ndn_lib::MsgRelType::Reaction
                        ) && purges.contains(&r.target.to_string())
                    }) {
                        purges.insert(row.try_get::<String, _>("obj_id").map_err(db_error)?);
                    }
                }
            }
        }
        for FetchedMessage {
            key,
            seq,
            id,
            object,
            accepted_at,
        } in fetched
        {
            if let Some(epoch) = state.member_epoch {
                // This reader may have only a SENT record for their own message.
                // Keep the host sequence and object identity with the reader's commit.
                let sql = self.msg_box_db.render_sql(
                    "INSERT INTO joined_group_projections(owner,group_did,member_epoch,session_key,session_seq,obj_id) VALUES(?,?,?,?,?,?) ON CONFLICT(owner,group_did,member_epoch,session_key,session_seq) DO NOTHING"
                );
                sqlx::query(&sql)
                    .bind(route.owner_did.to_string())
                    .bind(route.group_did.to_string())
                    .bind(i64::from(epoch))
                    .bind(key)
                    .bind(i64::try_from(*seq).map_err(invalid)?)
                    .bind(id.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db_error)?;
            }
            let sql=self.msg_box_db.render_sql("INSERT INTO group_objects(obj_id,group_did,body_json,jwt) VALUES(?,?,?,?) ON CONFLICT(obj_id) DO UPDATE SET body_json=CASE WHEN group_objects.body_json IS NULL THEN NULL ELSE excluded.body_json END,jwt=CASE WHEN group_objects.body_json IS NULL THEN NULL ELSE COALESCE(group_objects.jwt,excluded.jwt) END");
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(route.group_did.to_string())
                .bind(object.as_ref().map(|(m, _)| m.gen_obj_id().1))
                .bind(object.as_ref().and_then(|(_, j)| j.as_deref()))
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            // The member's own messages already exist as SENT records; only
            // other participants' messages are projected into the INBOX.
            if let Some((msg, _)) = object.as_ref().filter(|(m, _)| m.from != route.owner_did) {
                let mut record = Self::build_mailbox_record(
                    route.owner_did.clone(),
                    MailboxKind::Inbox,
                    msg,
                    Self::inbound_initial_state(msg),
                    None,
                    vec![
                        format!("group:{}", route.group_did.to_string()),
                        format!("session_seq:{seq}"),
                    ],
                    "group-projection",
                )?;
                record.session_id = Some(key.clone());
                record.mailbox = MailboxAddress::new(route.owner_did.clone(), Some(key.clone()))
                    .map_err(invalid)?;
                record.to = route.group_did.clone();
                record.sort_key = *accepted_at;
                self.msg_box_db
                    .upsert_record_with_msg_tx(&mut tx, &record, Some(msg))
                    .await?;
                projections.push(record);
            } else if object.is_none() {
                let now = Self::now_ms();
                let record = buckyos_api::MailboxRecord {
                    mailbox: MailboxAddress::new(route.owner_did.clone(), Some(key.clone()))
                        .map_err(invalid)?,
                    record_id: format!(
                        "{}|INBOX|{}|group-projection",
                        route.owner_did.to_string(),
                        id.to_string()
                    ),
                    owner: route.owner_did.clone(),
                    box_kind: MailboxKind::Inbox,
                    msg_id: id.clone(),
                    msg_kind: ndn_lib::MsgObjKind::GroupMsg,
                    // Already withdrawn before this member ever saw it: nothing to read.
                    state: RecipientState::Read,
                    from: route.group_did.clone(),
                    from_name: None,
                    to: route.group_did.clone(),
                    session_id: Some(key.clone()),
                    sort_key: *accepted_at,
                    tags: vec![
                        format!("group:{}", route.group_did.to_string()),
                        format!("session_seq:{seq}"),
                        "redacted".into(),
                    ],
                    ingress: None,
                    created_at_ms: now,
                    updated_at_ms: now,
                };
                self.msg_box_db
                    .upsert_record_with_msg_tx(&mut tx, &record, None)
                    .await?;
                projections.push(record);
                let sql = self
                    .msg_box_db
                    .render_sql("DELETE FROM msg_jwt_originals WHERE msg_id=?");
                sqlx::query(&sql)
                    .bind(id.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(db_error)?;
            }
            state
                .session_seqs
                .entry(key.clone())
                .and_modify(|s| *s = (*s).max(*seq))
                .or_insert(*seq);
        }
        for id in purges {
            crate::group_store::queue_purge(&self.msg_box_db, &mut tx, &ObjId::new(&id).map_err(invalid)?).await?;
            for statement in ["UPDATE group_objects SET body_json=NULL,jwt=NULL WHERE obj_id=?","DELETE FROM msg_jwt_originals WHERE msg_id=?"] {
                let sql=self.msg_box_db.render_sql(statement);sqlx::query(&sql).bind(&id).execute(&mut *tx).await.map_err(db_error)?;
            }
        }
        let sessions = if refresh_sessions {
            state.session_cache.as_slice()
        } else {
            &[]
        };
        for session in sessions {
            let sid = session["session_id"].as_str();
            let key = session_key(&route.group_did, sid)?;
            let now = Self::now_ms();
            let sql=self.msg_box_db.render_sql("INSERT INTO owner_sessions(owner,session_id,lifecycle,registered,origin,peer_did,binding_json,title,created_at_ms,updated_at_ms) VALUES(?,?,?,?,?,?,?,?,?,?) ON CONFLICT(owner,session_id) DO UPDATE SET binding_json=excluded.binding_json,title=excluded.title");
            let binding = json!({"authority_did":route.group_did,"session_key":key});
            sqlx::query(&sql)
                .bind(route.owner_did.to_string())
                .bind(key)
                .bind("active")
                .bind(1_i64)
                .bind("group")
                .bind(route.group_did.to_string())
                .bind(binding.to_string())
                .bind(session["shared_state"]["title"].as_str())
                .bind(now as i64)
                .bind(now as i64)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
        self.store_joined_state(&mut tx, state, previous).await?;
        tx.commit().await.map_err(db_error)?;
        for record in projections {
            Self::publish_box_changed_event(&record, "upsert");
        }
        Ok(())
    }
    pub(crate) async fn receive_group_update(
        &self,
        group: &DID,
        verified_host: &str,
        update: GroupUpdate,
    ) -> Result<GroupUpdateAck> {
        let mut entries = tokio::task::JoinSet::new();
        for entry in update.entries {
            let center = self.clone();
            let group = group.clone();
            let host = verified_host.to_owned();
            entries.spawn(async move { center.receive_reader_update(&group, &host, entry).await });
        }
        let mut committed = vec![];
        while let Some(result) = entries.join_next().await {
            match result {
                Ok(Ok(Some(ack))) => committed.push(ack),
                Ok(Ok(None)) => (),
                Ok(Err(error)) => {
                    log::debug!("rejected group {} update: {error}", group.to_string())
                }
                Err(error) => {
                    log::warn!("joined group {} update failed: {error}", group.to_string())
                }
            }
        }
        Ok(GroupUpdateAck { committed })
    }

    async fn receive_reader_update(
        &self,
        group: &DID,
        verified_host: &str,
        entry: GroupReaderUpdate,
    ) -> Result<Option<GroupReaderAck>> {
        let owner = &entry.reader_did;
        let _guard = self.groups.sync_locks.lock(owner, group).await;
        let binding = self.load_group_binding(owner, group).await?;
        let route = match &binding {
            Some(binding) if binding.active() => binding.route(self),
            Some(_) => return Ok(None),
            None => match self.configured_group_route(owner, group) {
                Some(route) => route,
                None => return Ok(None),
            },
        };
        if !route.host.eq_ignore_ascii_case(verified_host) {
            return Ok(None);
        }
        let (previous, state) = self.joined_snapshot(owner, group).await?.unzip();
        if state.as_ref().is_some_and(|s| s.stopped) {
            return Ok(None);
        }
        let binding_epoch = binding.and_then(|binding| binding.epoch());
        let epoch = binding_epoch.or_else(|| state.as_ref().and_then(|s| s.member_epoch));
        if epoch.zip(entry.member_epoch).is_some_and(|(a, b)| a != b) {
            return Ok(None);
        }
        let Some(delta) = entry.delta else {
            self.schedule_joined_pull(&route);
            return Ok(None);
        };
        let Some(entry_epoch) = entry.member_epoch else {
            return Ok(None);
        };
        if delta.next_token.len() > 256
            || delta.previous_token.len() > 256
            || delta.next_token.is_empty()
            || delta.previous_token.is_empty()
            || delta.previous_token == delta.next_token
        {
            return Err(invalid("invalid-push-cursors"));
        }
        let change = &delta.change;
        let session = match &change["session_id"] {
            Value::Null => None,
            Value::String(s) => Some(s.as_str()),
            _ => return Err(invalid("invalid-push-session")),
        };
        let key = session_key(group, session)?;
        let message = &change["message"];
        if change["kind"] != "message" || change["session"] != key || message["redacted"] != false {
            return Err(invalid("invalid-push-change"));
        }
        let seq = message["seq"]
            .as_u64()
            .filter(|s| *s > 0)
            .ok_or_else(|| invalid("invalid-session-seq"))?;
        let accepted_at = message["accepted_at_ms"]
            .as_u64()
            .ok_or_else(|| invalid("missing-accepted-at"))?;
        let id = ObjId::new(
            message["obj_id"]
                .as_str()
                .ok_or_else(|| invalid("invalid-object-id"))?,
        )
        .map_err(invalid)?;
        let encoding = CyfsNamedObjectEncoding::from_content_type(&delta.content_type)
            .ok_or_else(|| invalid("invalid-group-object-encoding"))?;
        let object = route
            .decode_object(self, &id, session, encoding, delta.object)
            .await?;
        if object.0.kind != ndn_lib::MsgObjKind::GroupMsg {
            return Err(invalid("invalid-push-object-kind"));
        }
        if object.0.relates_to.is_some() {
            self.schedule_joined_pull(&route);
            return Ok(None);
        }

        let Some(mut state) = state else {
            self.schedule_joined_pull(&route);
            return Ok(None);
        };
        if state.member_epoch != entry.member_epoch {
            if state.member_epoch.is_none() {
                self.schedule_joined_pull(&route);
            }
            return Ok(None);
        }
        if state.changes_token.as_ref() != Some(&delta.next_token) {
            if state.recent_push_tokens.contains(&delta.next_token) {
                return Ok(None);
            }
            let already_seen_seq = state.session_seqs.get(&key).copied().unwrap_or_default() >= seq;
            if already_seen_seq {
                let sql = self.msg_box_db.render_sql(
                    "SELECT 1 FROM joined_group_projections WHERE owner=? AND group_did=? AND member_epoch=? AND session_key=? AND session_seq=? AND obj_id=?"
                );
                let evidence = sqlx::query(&sql)
                    .bind(owner.to_string())
                    .bind(group.to_string())
                    .bind(i64::from(entry_epoch))
                    .bind(&key)
                    .bind(i64::try_from(seq).map_err(invalid)?)
                    .bind(id.to_string())
                    .fetch_optional(self.msg_box_db.pool())
                    .await
                    .map_err(db_error)?;
                if evidence.is_some() {
                    return Ok(None);
                }
            }
            if state.changes_token.as_ref() != Some(&delta.previous_token)
                || !state
                    .session_cache
                    .iter()
                    .any(|s| s["session_id"] == json!(session))
                || already_seen_seq
            {
                self.schedule_joined_pull(&route);
                return Ok(None);
            }
            state.changes_token = Some(delta.next_token.clone());
            if state.recent_push_tokens.len() >= 32 {
                state.recent_push_tokens.pop_front();
            }
            state.recent_push_tokens.push_back(delta.next_token.clone());
            let msg = FetchedMessage {
                key,
                seq,
                id,
                object: Some(object),
                accepted_at,
            };
            let result = self
                .project_joined_page(&route, &mut state, previous.as_deref(), &[msg], false)
                .await;
            if matches!(&result, Err(kRPC::RPCErrors::ReasonError(reason)) if reason == STALE_JOINED_STATE)
            {
                self.schedule_joined_pull(&route);
                return Ok(None);
            }
            result?;
            let count = self
                .groups
                .pulls
                .fast_commits
                .fetch_add(1, Ordering::Relaxed)
                + 1;
            log::debug!("joined group {} fast_commits={count}", group.to_string());
        }
        Ok(Some(GroupReaderAck {
            reader_did: entry.reader_did,
            next_token: delta.next_token,
        }))
    }

    fn schedule_joined_pull(&self, route: &JoinedGroupRoute) {
        let key = (route.owner_did.to_string(), route.group_did.to_string());
        {
            let mut pending = self.groups.pulls.pending.lock().unwrap();
            if let Some(again) = pending.get_mut(&key) {
                *again = true;
                return;
            }
            pending.insert(key.clone(), false);
        }
        let center = self.clone();
        let route = route.clone();
        tokio::spawn(async move {
            loop {
                let count = center
                    .groups
                    .pulls
                    .catchup_runs
                    .fetch_add(1, Ordering::Relaxed)
                    + 1;
                log::debug!(
                    "joined group {} catchup_runs={count}",
                    route.group_did.to_string()
                );
                if let Err(error) = center.pull_joined_group(&route, true).await {
                    log::warn!(
                        "joined group {} catchup failed: {error}",
                        route.group_did.to_string()
                    );
                }
                let mut pending = center.groups.pulls.pending.lock().unwrap();
                if pending.get_mut(&key).is_some_and(std::mem::take) {
                    continue;
                }
                pending.remove(&key);
                break;
            }
        });
    }

    pub(crate) async fn store_joined_state(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Any>,
        state: &JoinedGroupState,
        previous: Option<&str>,
    ) -> Result<()> {
        let statement = if previous.is_some() {
            "UPDATE joined_groups SET state_json=? WHERE owner=? AND group_did=? AND state_json=?"
        } else {
            "INSERT INTO joined_groups(state_json,owner,group_did) VALUES(?,?,?) ON CONFLICT(owner,group_did) DO NOTHING"
        };
        let sql = self.msg_box_db.render_sql(statement);
        let mut query = sqlx::query(&sql)
            .bind(serde_json::to_string(state).map_err(db_error)?)
            .bind(state.owner_did.to_string())
            .bind(state.group_did.to_string());
        if let Some(raw) = previous {
            query = query.bind(raw);
        }
        if query
            .execute(&mut **tx)
            .await
            .map_err(db_error)?
            .rows_affected()
            == 0
        {
            return Err(kRPC::RPCErrors::ReasonError(STALE_JOINED_STATE.into()));
        }
        Ok(())
    }

    async fn save_joined_state(&self, state: &JoinedGroupState, previous: &str) -> Result<()> {
        let mut tx = self.msg_box_db.pool().begin().await.map_err(db_error)?;
        self.store_joined_state(&mut tx, state, Some(previous))
            .await?;
        tx.commit().await.map_err(db_error)
    }

    async fn report_joined_reads(
        &self,
        route: &JoinedGroupRoute,
        require_binding: bool,
    ) -> Result<()> {
        let _guard = self
            .groups
            .sync_locks
            .lock(&route.owner_did, &route.group_did)
            .await;
        let Some((previous, mut state)) = self
            .joined_snapshot(&route.owner_did, &route.group_did)
            .await?
        else {
            return Ok(());
        };
        if state.stopped
            || !state.last_read_seq.iter().any(|(session, last_read)| {
                state
                    .reported_read_seq
                    .get(session)
                    .copied()
                    .unwrap_or_default()
                    < *last_read
            })
        {
            return Ok(());
        }
        self.check_joined_route(route, require_binding).await?;
        let mut changed = false;
        for (session, last_read) in &state.last_read_seq {
            if state.reported_read_seq.get(session).copied().unwrap_or(0) >= *last_read {
                continue;
            }
            let mailbox = MailboxAddress::try_from(session.clone()).map_err(invalid)?;
            if route
                .report_read(self, mailbox.session_id(), *last_read)
                .await
                .is_ok()
            {
                state.reported_read_seq.insert(session.clone(), *last_read);
                changed = true;
            }
        }
        if changed {
            self.save_joined_state(&state, &previous).await?;
        }
        Ok(())
    }

    pub(crate) fn start_group_sync(&self) {
        self.start_group_sync_with_interval(Duration::from_secs(15 * 60));
    }

    pub(crate) fn start_group_sync_with_interval(&self, period: Duration) {
        self.start_group_sync_with_clock(period, tokio::time::sleep);
    }

    pub(crate) fn start_group_sync_with_clock<S, F>(&self, period: Duration, sleep: S)
    where
        S: Fn(Duration) -> F + Clone + Send + Sync + 'static,
        F: Future<Output = ()> + Send,
    {
        let center = self.clone();
        tokio::spawn(async move { center.monitor_joined_routes(period, sleep).await });
        let center = self.clone();
        tokio::spawn(async move {
            loop {
                if let Err(error) = center.recover_remote_joins(false).await {
                    log::warn!("joined group recovery failed: {error}");
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
        let center = self.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(groups) = center.groups.list().await {
                    for group in groups {
                        if let Err(error) = center.expire_group_invitations(&group.group_did).await
                        {
                            log::warn!("group invitation expiration failed: {error}");
                        }
                    }
                }
                if let Err(error) = center.publish_group_documents().await {
                    log::warn!("group DID publication failed: {error}");
                }
                if let Err(error) = center.purge_group_objects().await {
                    log::warn!("group object purge failed: {error}");
                }
                tokio::time::sleep(Duration::from_secs(30)).await;
            }
        });
    }

    async fn monitor_joined_routes<S, F>(&self, period: Duration, sleep: S)
    where
        S: Fn(Duration) -> F + Clone + Send + Sync + 'static,
        F: Future<Output = ()> + Send,
    {
        let mut workers =
            BTreeMap::<BindingKey, (JoinedGroupRoute, tokio::task::JoinHandle<()>)>::new();
        let discovery = (period / 10).clamp(Duration::from_millis(10), Duration::from_secs(30));
        loop {
            match self.scheduled_group_routes().await {
                Ok(routes) => {
                    let mut previous = std::mem::take(&mut workers);
                    for route in routes {
                        let key = (route.owner_did.to_string(), route.group_did.to_string());
                        if let Some((prior, handle)) = previous.remove(&key) {
                            if prior.host == route.host
                                && prior.upstream == route.upstream
                                && prior.authorization == route.authorization
                                && !handle.is_finished()
                            {
                                workers.insert(key, (prior, handle));
                                continue;
                            }
                            handle.abort();
                        }
                        let center = self.clone();
                        let worker_route = route.clone();
                        let sleep = sleep.clone();
                        let handle = tokio::spawn(async move {
                            tokio::join!(
                                center.poll_joined_changes(&worker_route, period, sleep),
                                async {
                                    loop {
                                        tokio::time::sleep(Duration::from_secs(30)).await;
                                        if let Err(error) =
                                            center.report_joined_reads(&worker_route, true).await
                                        {
                                            log::warn!("joined read report failed: {error}");
                                        }
                                    }
                                }
                            );
                        });
                        workers.insert(key, (route, handle));
                    }
                    for (_, (_, handle)) in previous {
                        handle.abort();
                    }
                }
                Err(error) => log::warn!("joined group route discovery failed: {error}"),
            }
            tokio::time::sleep(discovery).await;
        }
    }

    async fn poll_joined_changes<F>(
        &self,
        route: &JoinedGroupRoute,
        period: Duration,
        sleep: impl Fn(Duration) -> F,
    ) where
        F: Future<Output = ()>,
    {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hash = DefaultHasher::new();
        (route.owner_did.to_string(), route.group_did.to_string()).hash(&mut hash);
        let staggered = period - period.mul_f64((hash.finish() % 101) as f64 / 1000.);
        if self
            .joined_state(&route.owner_did, &route.group_did)
            .await
            .ok()
            .flatten()
            .is_some_and(|state| state.changes_token.is_some())
        {
            sleep(staggered).await;
        }
        let mut backoff = Duration::from_secs(30).min(staggered);
        loop {
            match self.pull_joined_group(route, true).await {
                Ok(_) => {
                    backoff = Duration::from_secs(30).min(staggered);
                    sleep(staggered).await;
                }
                Err(error) => {
                    log::warn!(
                        "joined group {} compensation failed: {error}",
                        route.group_did.to_string()
                    );
                    sleep(backoff).await;
                    backoff = (backoff * 2).min(staggered);
                }
            }
        }
    }

    pub(crate) async fn advance_joined_read_from_record(
        &self,
        record: &buckyos_api::MailboxRecord,
        group: &DID,
    ) -> Result<()> {
        let Some(seq) = record.tags.iter().find_map(|t| {
            t.strip_prefix("session_seq:")
                .and_then(|s| s.parse::<u64>().ok())
        }) else {
            return Ok(());
        };
        let Some(session) = record.session_id.as_ref() else {
            return Ok(());
        };
        let _guard = self.groups.sync_locks.lock(&record.owner, group).await;
        while let Some((previous, mut state)) = self.joined_snapshot(&record.owner, group).await? {
            state
                .last_read_seq
                .entry(session.clone())
                .and_modify(|s| *s = (*s).max(seq))
                .or_insert(seq);
            let result = self.save_joined_state(&state, &previous).await;
            if matches!(&result, Err(kRPC::RPCErrors::ReasonError(reason)) if reason == STALE_JOINED_STATE)
            {
                continue;
            }
            return result;
        }
        Ok(())
    }
}

impl JoinedGroupRoute {
    async fn report_read(&self, center: &MessageCenter, session: Option<&str>, last_read: u64) -> Result<()> {
        self.put(center, &format!("{}/read_markers", self.base_path()),
            &json!({"session_id":session,"last_read_seq":last_read})).await?;
        Ok(())
    }
}
impl MessageCenter {
    /// Unbinds and removes the messages redactions queued (`queue_purge`).
    pub(crate) async fn purge_group_objects(&self) -> Result<()> {
        let Some(buckets) = self.open_buckets().await? else {
            return Ok(());
        };
        for row in sqlx::query("SELECT obj_id,attachment_prefix FROM group_object_purges LIMIT 100")
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
        {
            let raw: String = row.try_get("obj_id").map_err(db_error)?;
            let prefix: Option<String> = row.try_get("attachment_prefix").map_err(db_error)?;
            let id = ObjId::new(&raw).map_err(db_error)?;
            self.purge_message(&buckets, &id, prefix).await?;
            let sql = self
                .msg_box_db
                .render_sql("DELETE FROM group_object_purges WHERE obj_id=?");
            sqlx::query(&sql)
                .bind(raw)
                .execute(self.msg_box_db.pool())
                .await
                .map_err(db_error)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msg_box_db::MsgBoxDbMgr;

    #[tokio::test]
    async fn stale_projection_from_a_second_instance_rolls_back_objects_mail_and_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let url = format!(
            "sqlite://{}?mode=rwc",
            dir.path().join("messages.sqlite").display()
        );
        let open = || async {
            let db = MsgBoxDbMgr::open_default_sqlite(&url).await.unwrap();
            MessageCenter::open_with_db(db).await.unwrap()
        };
        let writer = open().await;
        let delayed = open().await;
        let route = JoinedGroupRoute {
            owner_did: DID::new("web", "bob.member.test"),
            group_did: DID::new("web", "room.host.test"),
            host: "host.test".into(),
            upstream: "https://host.test".into(),
            authorization: None,
        };
        let mut initial = JoinedGroupState::new(&route);
        initial.changes_token = Some("A".into());
        writer
            .project_joined_page(&route, &mut initial, None, &[], false)
            .await
            .unwrap();
        let (snapshot, mut old) = delayed
            .joined_snapshot(&route.owner_did, &route.group_did)
            .await
            .unwrap()
            .unwrap();
        let mut current = old.clone();
        current.changes_token = Some("C".into());
        writer.save_joined_state(&current, &snapshot).await.unwrap();

        let msg = MsgObject {
            from: DID::new("web", "alice.host.test"),
            to: vec![route.group_did.clone()],
            kind: ndn_lib::MsgObjKind::GroupMsg,
            ..Default::default()
        };
        let id = msg.gen_obj_id().0;
        let late = FetchedMessage {
            key: route.group_did.to_string(),
            seq: 1,
            id: id.clone(),
            object: Some((msg, None)),
            accepted_at: 1000,
        };
        old.changes_token = Some("B".into());
        let result = delayed
            .project_joined_page(&route, &mut old, Some(&snapshot), &[late], false)
            .await;
        assert!(
            matches!(result, Err(kRPC::RPCErrors::ReasonError(reason)) if reason == STALE_JOINED_STATE)
        );
        assert!(writer.groups.object(&id).await.unwrap().is_none());
        assert!(writer
            .msg_box_db
            .list_message_records(&id)
            .await
            .unwrap()
            .is_empty());
        let saved = writer
            .joined_state(&route.owner_did, &route.group_did)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.changes_token.as_deref(), Some("C"));
        assert!(saved.session_seqs.is_empty());
    }
}
