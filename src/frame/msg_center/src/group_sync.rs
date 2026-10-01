use crate::group_store::db_error;
use crate::group_types::*;
use crate::msg_center::MessageCenter;
use buckyos_api::{MailboxAddress, MailboxKind, RecipientState};
use name_lib::DID;
use ndn_lib::{CyfsNamedObjectEncoding, MsgObject, NamedObject, ObjId, CYFS_HEADER_ORIGINAL_USER};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::BTreeMap;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JoinedGroupRoute {
    pub owner_did: DID,
    pub group_did: DID,
    pub host: String,
    pub upstream: String,
    pub authorization: String,
    pub proof_ids: Vec<String>,
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
    pub fn validate(&self) -> Result<()> {
        let url = reqwest::Url::parse(&self.upstream).map_err(invalid)?;
        if !matches!(url.scheme(), "http" | "https")
            || url.host_str().is_none()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
            || !self.authorization.starts_with("Bearer ")
            || http::HeaderValue::from_str(&self.authorization).is_err()
            || self.proof_ids.is_empty()
        {
            return Err(invalid("invalid-joined-group-route"));
        }
        if ndn_lib::normalize_cyfs_dispatch_target(&self.host, "/inbox").is_err() {
            return Err(invalid("invalid-group-host"));
        }
        for proof in &self.proof_ids {
            ObjId::new(proof).map_err(invalid)?;
        }
        Ok(())
    }
    pub fn base_path(&self) -> String {
        format!("/{}", self.group_did.to_string())
    }
    async fn fetch(&self, path: &str, query: &[(&str, String)]) -> Result<Value> {
        let mut url = reqwest::Url::parse(&self.upstream).map_err(invalid)?;
        url.set_path(path);
        url.query_pairs_mut()
            .extend_pairs(query.iter().map(|(k, v)| (*k, v.as_str())));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(db_error)?;
        let response = client
            .get(url)
            .header("host", &self.host)
            .header("authorization", &self.authorization)
            .header(CYFS_HEADER_ORIGINAL_USER, self.owner_did.to_string())
            .header(
                "cyfs-proofs",
                serde_json::to_string(&self.proof_ids).unwrap(),
            )
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(db_error)?;
        if !response.status().is_success() {
            return Err(kRPC::RPCErrors::ReasonError(format!(
                "joined-group-host:{}",
                response.status()
            )));
        }
        if response.content_length().is_some_and(|n| n > 1024 * 1024) {
            return Err(invalid("group-response-too-large"));
        }
        let mut response = response;
        let mut bytes = vec![];
        while let Some(chunk) = response.chunk().await.map_err(db_error)? {
            if chunk.len() > (1024_usize * 1024).saturating_sub(bytes.len()) {
                return Err(invalid("group-response-too-large"));
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(invalid)
    }
    async fn object(&self, id: &ObjId) -> Result<(MsgObject, Option<String>)> {
        let mut url = reqwest::Url::parse(&self.upstream).map_err(invalid)?;
        url.set_path(&format!("{}/objects/{}", self.base_path(), id.to_string()));
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(db_error)?;
        let response = client
            .get(url)
            .header("host", &self.host)
            .header("authorization", &self.authorization)
            .header(CYFS_HEADER_ORIGINAL_USER, self.owner_did.to_string())
            .header(
                "cyfs-proofs",
                serde_json::to_string(&self.proof_ids).unwrap(),
            )
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(db_error)?
            .error_for_status()
            .map_err(db_error)?;
        let encoding = response
            .headers()
            .get("content-type")
            .and_then(|h| h.to_str().ok())
            .and_then(CyfsNamedObjectEncoding::from_content_type)
            .ok_or_else(|| invalid("invalid-group-object-encoding"))?;
        if response.content_length().is_some_and(|n| n > 65_536) {
            return Err(invalid("group-object-too-large"));
        }
        let raw = response.text().await.map_err(db_error)?;
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
                let signed = crate::cyfs_dispatch::verify_signed_message(&raw)
                    .await
                    .map_err(|(_, r)| denied(r))?;
                (signed.msg, Some(raw))
            }
        };
        if msg.gen_obj_id().0 != *id || msg.to != [self.group_did.clone()] {
            return Err(invalid("joined-object-identity-mismatch"));
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
    pub session_seqs: BTreeMap<String, u64>,
    pub last_read_seq: BTreeMap<String, u64>,
    pub reported_read_seq: BTreeMap<String, u64>,
    pub stopped: bool,
    #[serde(default)]
    pub proof_ids: Vec<String>,
    #[serde(default)]
    pub doc_cache: Option<Value>,
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
    pub(crate) async fn sync_joined_group(&self, route: &JoinedGroupRoute) -> Result<Value> {
        route.validate()?;
        let _guard = self.groups.sync_lock.lock().await;
        let mut state = self
            .joined_groups(&route.owner_did)
            .await?
            .into_iter()
            .find(|s| s.group_did == route.group_did)
            .unwrap_or(JoinedGroupState {
                schema_version: 1,
                owner_did: route.owner_did.clone(),
                group_did: route.group_did.clone(),
                session_cache: vec![],
                changes_token: None,
                session_seqs: BTreeMap::new(),
                last_read_seq: BTreeMap::new(),
                reported_read_seq: BTreeMap::new(),
                stopped: false,
                proof_ids: route.proof_ids.clone(),
                doc_cache: None,
            });
        if state.proof_ids != route.proof_ids {
            state.proof_ids = route.proof_ids.clone();
            state.changes_token = None;
            state.stopped = false;
        }
        if state.stopped {
            return Ok(json!({"stopped":true}));
        }
        let mut query = vec![("limit", "100".to_string())];
        if let Some(t) = &state.changes_token {
            query.push(("since", t.clone()));
        }
        let changes = route
            .fetch(&format!("{}/changes", route.base_path()), &query)
            .await?;
        let mut pending = vec![];
        let mut lost_session = false;
        for change in changes["items"]
            .as_array()
            .ok_or_else(|| invalid("invalid-group-changes"))?
        {
            if change["action"] == "session.deleted"
                || (change["subject_did"] == json!(route.owner_did)
                    && ["session.member_left", "session.member_removed"]
                        .contains(&change["action"].as_str().unwrap_or("")))
            {
                state
                    .session_cache
                    .retain(|s| s["session_id"] != change["session_id"]);
                lost_session = true;
            }
            if change["action"] == "entity.group_deleted"
                || (change["session_id"].is_null()
                    && (change["subject_did"] == json!(route.owner_did)
                        || change["subject_did"].is_null())
                    && ["entity.member_left", "entity.member_removed"]
                        .contains(&change["action"].as_str().unwrap_or("")))
            {
                state.stopped = true;
            }
            if let Some(message) = change.get("message") {
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
                .fetch(&format!("{}/sessions", route.base_path()), &[])
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
                            &inbox,
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
                Some(route.object(&id).await?)
            };
            if object
                .as_ref()
                .is_some_and(|(msg, _)| msg.to_session != sid)
            {
                return Err(invalid("joined-message-session-mismatch"));
            }
            fetched.push((key, seq, id, redacted, object, accepted_at));
        }
        if !state.stopped {
            for (session, last_read) in state.last_read_seq.clone() {
                if state.reported_read_seq.get(&session).copied().unwrap_or(0) >= last_read {
                    continue;
                }
                let mailbox = MailboxAddress::try_from(session.clone()).map_err(invalid)?;
                if route
                    .report_read(mailbox.session_id(), last_read)
                    .await
                    .is_ok()
                {
                    state.reported_read_seq.insert(session, last_read);
                }
            }
        }
        let mut tx = self.msg_box_db.pool().begin().await.map_err(db_error)?;
        let mut purges = std::collections::BTreeSet::new();
        for (_, _, id, redacted, object, _) in &fetched {
            if *redacted {
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
        for (_, _, id, _, object, _) in &fetched {
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
        for (key, seq, id, redacted, object, accepted_at) in &fetched {
            let sql=self.msg_box_db.render_sql("INSERT INTO group_objects(obj_id,group_did,body_json,jwt) VALUES(?,?,?,?) ON CONFLICT(obj_id) DO UPDATE SET body_json=CASE WHEN group_objects.body_json IS NULL THEN NULL ELSE excluded.body_json END,jwt=CASE WHEN group_objects.body_json IS NULL THEN NULL ELSE COALESCE(group_objects.jwt,excluded.jwt) END");
            sqlx::query(&sql)
                .bind(id.to_string())
                .bind(route.group_did.to_string())
                .bind(object.as_ref().map(|(m, _)| m.gen_obj_id().1))
                .bind(object.as_ref().and_then(|(_, j)| j.as_deref()))
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
            if let Some((msg, _)) = object {
                let mut record = Self::build_mailbox_record(
                    route.owner_did.clone(),
                    MailboxKind::Inbox,
                    msg,
                    RecipientState::Unread,
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
            } else if *redacted {
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
                    state: RecipientState::Unread,
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
            for statement in ["UPDATE group_objects SET body_json=NULL,jwt=NULL WHERE obj_id=?","DELETE FROM msg_jwt_originals WHERE msg_id=?","INSERT INTO group_object_purges(obj_id,schema_version) VALUES(?,1) ON CONFLICT(obj_id) DO NOTHING"] {
                let sql=self.msg_box_db.render_sql(statement);sqlx::query(&sql).bind(&id).execute(&mut *tx).await.map_err(db_error)?;
            }
        }
        for session in &state.session_cache {
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
        state.changes_token = changes["next_token"]
            .as_str()
            .map(str::to_owned)
            .or(state.changes_token);
        let sql=self.msg_box_db.render_sql("INSERT INTO joined_groups(owner,group_did,state_json) VALUES(?,?,?) ON CONFLICT(owner,group_did) DO UPDATE SET state_json=excluded.state_json");
        sqlx::query(&sql)
            .bind(route.owner_did.to_string())
            .bind(route.group_did.to_string())
            .bind(serde_json::to_string(&state).map_err(db_error)?)
            .execute(&mut *tx)
            .await
            .map_err(db_error)?;
        tx.commit().await.map_err(db_error)?;
        Ok(json!({"synced":fetched.len(),"stopped":state.stopped}))
    }
    pub(crate) fn start_group_sync(&self) {
        let center = self.clone();
        tokio::spawn(async move {
            loop {
                if let Ok(groups) = center.groups.list().await {
                    for group in groups {
                        if let Err(e) = center.expire_group_invitations(&group.group_did).await {
                            log::warn!("group invitation expiration failed: {e}");
                        }
                    }
                }
                if let Err(e) = center.publish_group_documents().await {
                    log::warn!("group DID publication failed: {e}");
                }
                if let Err(e) = center.purge_group_objects().await {
                    log::warn!("group object purge failed: {e}");
                }
                let routes = center.cyfs_dispatch.read().unwrap().joined_groups.clone();
                for route in routes {
                    if let Err(e) = center.sync_joined_group(&route).await {
                        log::warn!(
                            "joined group {} sync failed: {}",
                            route.group_did.to_string(),
                            e
                        );
                    }
                }
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        });
    }
}

impl MessageCenter {
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
        let _guard = self.groups.sync_lock.lock().await;
        if let Some(mut state) = self
            .joined_groups(&record.owner)
            .await?
            .into_iter()
            .find(|g| &g.group_did == group)
        {
            state
                .last_read_seq
                .entry(session.clone())
                .and_modify(|s| *s = (*s).max(seq))
                .or_insert(seq);
            let sql = self
                .msg_box_db
                .render_sql("UPDATE joined_groups SET state_json=? WHERE owner=? AND group_did=?");
            sqlx::query(&sql)
                .bind(serde_json::to_string(&state).map_err(db_error)?)
                .bind(record.owner.to_string())
                .bind(group.to_string())
                .execute(self.msg_box_db.pool())
                .await
                .map_err(db_error)?;
        }
        Ok(())
    }
}

impl JoinedGroupRoute {
    async fn report_read(&self, session: Option<&str>, last_read: u64) -> Result<()> {
        let mut url = reqwest::Url::parse(&self.upstream).map_err(invalid)?;
        url.set_path(&format!("{}/read_markers", self.base_path()));
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(db_error)?
            .put(url)
            .header("host", &self.host)
            .header("authorization", &self.authorization)
            .header(CYFS_HEADER_ORIGINAL_USER, self.owner_did.to_string())
            .header(
                "cyfs-proofs",
                serde_json::to_string(&self.proof_ids).unwrap(),
            )
            .json(&json!({"session_id":session,"last_read_seq":last_read}))
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await
            .map_err(db_error)?;
        response.error_for_status().map_err(db_error)?;
        Ok(())
    }
}
impl MessageCenter {
    async fn purge_group_objects(&self) -> Result<()> {
        let Ok(runtime) = buckyos_api::get_buckyos_api_runtime() else {
            return Ok(());
        };
        let store = runtime.get_named_store().await?;
        for row in sqlx::query("SELECT obj_id FROM group_object_purges LIMIT 100")
            .fetch_all(self.msg_box_db.pool())
            .await
            .map_err(db_error)?
        {
            let raw: String = row.try_get("obj_id").map_err(db_error)?;
            let id = ObjId::new(&raw).map_err(db_error)?;
            match store.remove_object(&id).await {
                Ok(_) => {}
                Err(e)
                    if e.to_string().to_lowercase().contains("notfound")
                        || e.to_string().to_lowercase().contains("not found") => {}
                Err(e) => return Err(db_error(e)),
            }
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
