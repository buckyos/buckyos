//! Owner-scoped session semantics of the MessageCenter:
//!
//! * caller identity from the verified session token and the viewer → owner
//!   authorization rules (`Message Center.md` §5.6);
//! * owner-local session lifecycle: registration of empty sessions, archive /
//!   restore, delete watermark (`Message Center.md` §5.5 / §5.8);
//! * the session list / timeline projection that applies those rules on top
//!   of the mailbox index (`UI_DATAMODEL.md` §1.4a activity ordering).

use crate::msg_center::MessageCenter;
use crate::owner_session_db::{DeleteWatermark, VisibleSessionIndexEntry};
use async_trait::async_trait;
use buckyos_api::{
    get_buckyos_api_runtime, validate_verify_hub_token_claims, MailboxRecord,
    MsgCenterCreateSessionReq, OwnerSessionState, SessionLifecycle, SessionListLifecycleFilter,
    SessionListOrder, SessionMessagePage, SessionSummary, SessionSummaryPage, TokenPrincipalKind,
    TokenUse, UiSessionStateEntry, UserPrivateProfile,
};
use kRPC::{RPCContext, RPCErrors, RPCSessionToken};
use log::warn;
use name_lib::{AgentDocument, DID};
use ndn_lib::MsgObjKind;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

const DEFAULT_SESSION_LIST_LIMIT: usize = 50;
const MAX_SESSION_LIST_LIMIT: usize = 200;
const DEFAULT_LIST_LIMIT: usize = 50;
const MAX_LIST_LIMIT: usize = 500;
const MAX_SESSION_TITLE_CHARS: usize = 64;
const MAX_SESSION_ID_CHARS: usize = 200;

/// Verifies a raw session token. Production uses the runtime trust keys;
/// tests inject a static key.
#[async_trait]
pub trait SessionTokenVerifier: Send + Sync {
    async fn verify(&self, token: &str) -> std::result::Result<RPCSessionToken, RPCErrors>;
    async fn resolve_user_did(&self, user_id: &str) -> std::result::Result<DID, RPCErrors>;
    async fn is_zone_agent(&self, did: &DID) -> std::result::Result<bool, RPCErrors>;
}

pub struct RuntimeSessionTokenVerifier;

#[async_trait]
impl SessionTokenVerifier for RuntimeSessionTokenVerifier {
    async fn verify(&self, token: &str) -> std::result::Result<RPCSessionToken, RPCErrors> {
        get_buckyos_api_runtime()?
            .verify_trusted_session_token(token)
            .await
    }

    async fn resolve_user_did(&self, user_id: &str) -> std::result::Result<DID, RPCErrors> {
        let client = get_buckyos_api_runtime()?
            .get_system_config_client()
            .await?;
        let value = client
            .get(&format!("users/{}/profile", user_id))
            .await
            .map_err(|error| permission_denied(error.to_string()))?;
        let profile: UserPrivateProfile = serde_json::from_str(&value.value)
            .map_err(|error| permission_denied(format!("invalid user profile: {}", error)))?;
        Ok(profile.did)
    }

    async fn is_zone_agent(&self, did: &DID) -> std::result::Result<bool, RPCErrors> {
        let client = get_buckyos_api_runtime()?
            .get_system_config_client()
            .await?;
        for agent_id in client
            .list("agents")
            .await
            .map_err(|error| permission_denied(error.to_string()))?
        {
            let value = client
                .get(&format!("agents/{}/doc", agent_id))
                .await
                .map_err(|error| permission_denied(error.to_string()))?;
            let doc: AgentDocument = serde_json::from_str(&value.value)
                .map_err(|error| permission_denied(format!("invalid agent document: {}", error)))?;
            if &doc.id == did {
                let settings = client
                    .get(&format!("agents/{}/settings", agent_id))
                    .await
                    .map_err(|error| permission_denied(error.to_string()))?;
                let settings: Value = serde_json::from_str(&settings.value).map_err(|error| {
                    permission_denied(format!("invalid agent settings: {}", error))
                })?;
                return Ok(settings.get("state").and_then(Value::as_str) != Some("deleted"));
            }
        }
        Ok(false)
    }
}

/// Holder so `MessageCenter` can stay `Clone + Debug` while carrying a
/// swappable verifier.
pub struct TokenVerifierSlot(RwLock<Arc<dyn SessionTokenVerifier>>);

impl TokenVerifierSlot {
    pub fn new(verifier: Arc<dyn SessionTokenVerifier>) -> Self {
        Self(RwLock::new(verifier))
    }

    fn get(&self) -> Arc<dyn SessionTokenVerifier> {
        self.0.read().unwrap().clone()
    }

    #[allow(dead_code)]
    fn set(&self, verifier: Arc<dyn SessionTokenVerifier>) {
        *self.0.write().unwrap() = verifier;
    }
}

impl std::fmt::Debug for TokenVerifierSlot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TokenVerifierSlot")
    }
}

impl Default for TokenVerifierSlot {
    fn default() -> Self {
        Self::new(Arc::new(RuntimeSessionTokenVerifier))
    }
}

/// Identity of the caller as established from the verified session token.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallerIdentity {
    pub user_id: String,
    pub principal_kind: TokenPrincipalKind,
    pub user_did: Option<DID>,
}

fn permission_denied(reason: impl Into<String>) -> RPCErrors {
    RPCErrors::NoPermission(reason.into())
}

fn is_activity_kind(kind: MsgObjKind) -> bool {
    matches!(
        kind,
        MsgObjKind::Chat | MsgObjKind::GroupMsg | MsgObjKind::Deliver
    )
}

fn normalize_session_id(raw: &str) -> std::result::Result<String, RPCErrors> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(RPCErrors::ParseRequestError(
            "session_id cannot be empty".to_string(),
        ));
    }
    if trimmed.chars().count() > MAX_SESSION_ID_CHARS {
        return Err(RPCErrors::ParseRequestError(
            "session_id is too long".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_title(raw: Option<String>) -> std::result::Result<Option<String>, RPCErrors> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > MAX_SESSION_TITLE_CHARS {
        return Err(RPCErrors::ParseRequestError(format!(
            "title exceeds {} characters",
            MAX_SESSION_TITLE_CHARS
        )));
    }
    Ok(Some(trimmed.to_string()))
}

impl MessageCenter {
    #[allow(dead_code)]
    pub fn set_token_verifier(&self, verifier: Arc<dyn SessionTokenVerifier>) {
        self.token_verifier.set(verifier);
    }

    /// Resolve the caller from `ctx.token`.
    ///
    /// * `Ok(None)`: no token at all. Kept for in-process callers and legacy
    ///   trusted transports; such calls are not restricted (pre-existing
    ///   behaviour).
    /// * `Ok(Some(_))`: a verified verify-hub session token.
    /// * `Err(_)`: a token was supplied but is invalid / expired / not trusted.
    pub(crate) async fn caller_identity(
        &self,
        ctx: &RPCContext,
    ) -> std::result::Result<Option<CallerIdentity>, RPCErrors> {
        let Some(token) = ctx
            .token
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
        else {
            return Ok(None);
        };
        let verified = self
            .token_verifier
            .get()
            .verify(token)
            .await
            .map_err(|error| {
                warn!("msg-center rejected session token: {}", error);
                permission_denied(format!("invalid session token: {}", error))
            })?;
        let claims = validate_verify_hub_token_claims(&verified, TokenUse::Session)
            .map_err(|error| permission_denied(format!("invalid session claims: {}", error)))?;
        let user_id = verified
            .sub
            .clone()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| permission_denied("session token has no subject"))?;
        let user_did = if claims.principal_kind == TokenPrincipalKind::User {
            Some(
                self.token_verifier
                    .get()
                    .resolve_user_did(&user_id)
                    .await
                    .map_err(|error| {
                        permission_denied(format!("cannot resolve user identity: {}", error))
                    })?,
            )
        } else {
            None
        };
        Ok(Some(CallerIdentity {
            user_id,
            principal_kind: claims.principal_kind,
            user_did,
        }))
    }

    /// A zone user may observe a mailbox owner other than themselves only when
    /// the owner is a zone-hosted non-user identity (an agent such as
    /// `did:web:jarvis.<zone>`). Other users' mailboxes are never readable.
    async fn user_may_observe(&self, owner: &DID) -> std::result::Result<bool, RPCErrors> {
        if !self.is_local_recipient(owner) {
            return Ok(false);
        }
        self.token_verifier.get().is_zone_agent(owner).await
    }

    /// Read access of the caller to `owner`'s mailbox / sessions.
    pub(crate) async fn authorize_owner_read(
        &self,
        ctx: &RPCContext,
        owner: &DID,
    ) -> std::result::Result<(), RPCErrors> {
        match self.caller_identity(ctx).await? {
            None => Ok(()),
            Some(caller) => match caller.principal_kind {
                TokenPrincipalKind::User => {
                    if caller.user_did.as_ref() == Some(owner)
                        || self.user_may_observe(owner).await?
                    {
                        Ok(())
                    } else {
                        Err(permission_denied(format!(
                            "user {} may not read mailbox of {}",
                            caller.user_id,
                            owner.to_string()
                        )))
                    }
                }
                _ => Ok(()),
            },
        }
    }

    /// Write access: a zone user may only act as themselves. Agent observation
    /// is read-only by design (`Message Center.md` §5.6).
    pub(crate) async fn authorize_owner_write(
        &self,
        ctx: &RPCContext,
        owner: &DID,
    ) -> std::result::Result<(), RPCErrors> {
        match self.caller_identity(ctx).await? {
            None => Ok(()),
            Some(caller) => match caller.principal_kind {
                TokenPrincipalKind::User => {
                    if caller.user_did.as_ref() == Some(owner) {
                        Ok(())
                    } else {
                        Err(permission_denied(format!(
                            "user {} may not write as {}",
                            caller.user_id,
                            owner.to_string()
                        )))
                    }
                }
                _ => Ok(()),
            },
        }
    }

    // ------------------------------------------------------------------
    // Lifecycle
    // ------------------------------------------------------------------

    fn new_owner_session_state(owner: DID, session_id: String, now_ms: u64) -> OwnerSessionState {
        OwnerSessionState {
            owner,
            session_id,
            lifecycle: SessionLifecycle::Active,
            registered: false,
            origin: None,
            peer_did: None,
            binding: None,
            title: None,
            archived_at_ms: None,
            delete_watermark_sort_key: None,
            delete_watermark_record_id: None,
            deleted_at_ms: None,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        }
    }

    pub(crate) async fn create_session_internal(
        &self,
        req: MsgCenterCreateSessionReq,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        let now_ms = Self::now_ms();
        let session_id = match req.session_id.as_deref() {
            Some(raw) => normalize_session_id(raw)?,
            None => uuid::Uuid::new_v4().to_string(),
        };
        let title = normalize_title(req.title)?;
        if let Some(existing) = self
            .msg_box_db
            .get_owner_session(&req.owner, &session_id)
            .await?
        {
            if existing.registered && existing.peer_did.as_ref() == Some(&req.peer_did) {
                // Idempotent retry with a caller-chosen id.
                return Ok(existing);
            }
            if existing.registered {
                return Err(RPCErrors::ReasonError(format!(
                    "session {} already exists for another peer",
                    session_id
                )));
            }
            // A previously deleted or lifecycle-only row: re-register while
            // keeping its delete watermark so old history stays hidden.
            let state = OwnerSessionState {
                registered: true,
                lifecycle: SessionLifecycle::Active,
                origin: Some(req.origin.unwrap_or_else(|| "manual".to_string())),
                peer_did: Some(req.peer_did),
                binding: req.binding,
                title,
                archived_at_ms: None,
                updated_at_ms: now_ms,
                ..existing
            };
            self.msg_box_db.upsert_owner_session(&state).await?;
            return Ok(state);
        }
        let mut state = Self::new_owner_session_state(req.owner, session_id, now_ms);
        state.registered = true;
        state.origin = Some(req.origin.unwrap_or_else(|| "manual".to_string()));
        state.peer_did = Some(req.peer_did);
        state.binding = req.binding;
        state.title = title;
        self.msg_box_db.upsert_owner_session(&state).await?;
        Ok(state)
    }

    async fn load_or_default_state(
        &self,
        owner: &DID,
        session_id: &str,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        if let Some(state) = self.msg_box_db.get_owner_session(owner, session_id).await? {
            return Ok(state);
        }
        // Lifecycle rows are created lazily; a session only known from the
        // mailbox index must have at least one visible record.
        let has_records = !self
            .msg_box_db
            .list_visible_session_records(owner, session_id, 1, None, None, true, None)
            .await?
            .is_empty();
        if !has_records {
            return Err(RPCErrors::ReasonError(format!(
                "session {} not found for owner {}",
                session_id,
                owner.to_string()
            )));
        }
        Ok(Self::new_owner_session_state(
            owner.clone(),
            session_id.to_string(),
            Self::now_ms(),
        ))
    }

    pub(crate) async fn set_session_lifecycle_internal(
        &self,
        owner: DID,
        session_id: String,
        lifecycle: SessionLifecycle,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let mut state = self.load_or_default_state(&owner, &session_id).await?;
        if state.lifecycle == lifecycle {
            return Ok(state);
        }
        let now_ms = Self::now_ms();
        state.lifecycle = lifecycle;
        state.archived_at_ms = match lifecycle {
            SessionLifecycle::Archived => Some(now_ms),
            SessionLifecycle::Active => None,
        };
        state.updated_at_ms = now_ms;
        self.msg_box_db.upsert_owner_session(&state).await?;
        Ok(state)
    }

    /// Delete for this owner only: move the visibility watermark past every
    /// record that exists now, drop registration and the owner's UI state.
    /// Per-record `RecipientState`, other owners' references and the shared
    /// `MsgObject`s are untouched.
    pub(crate) async fn delete_session_internal(
        &self,
        owner: DID,
        session_id: String,
    ) -> std::result::Result<OwnerSessionState, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let now_ms = Self::now_ms();
        let mut state = match self
            .msg_box_db
            .get_owner_session(&owner, &session_id)
            .await?
        {
            Some(state) => state,
            None => Self::new_owner_session_state(owner.clone(), session_id.clone(), now_ms),
        };
        let (max_sort_key, max_record_id) = self
            .msg_box_db
            .session_max_record_key(&owner, &session_id)
            .await?
            .unwrap_or((0, String::new()));
        // Records are keyed by the message creation time; a replay of an old
        // message carries the same sort_key and therefore stays hidden.
        let watermark_sort_key = max_sort_key.max(now_ms);
        let watermark_record_id = if watermark_sort_key == max_sort_key {
            max_record_id
        } else {
            String::new()
        };
        state.registered = false;
        state.lifecycle = SessionLifecycle::Active;
        state.archived_at_ms = None;
        state.delete_watermark_sort_key = Some(watermark_sort_key);
        state.delete_watermark_record_id = Some(watermark_record_id);
        state.deleted_at_ms = Some(now_ms);
        state.title = None;
        state.updated_at_ms = now_ms;
        self.msg_box_db.upsert_owner_session(&state).await?;
        self.msg_box_db
            .delete_owner_ui_session_states(&owner, &session_id)
            .await?;
        Ok(state)
    }

    pub(crate) async fn get_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
    ) -> std::result::Result<Option<OwnerSessionState>, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        self.msg_box_db.get_owner_session(&owner, &session_id).await
    }

    /// Called after mailbox records are committed: an effective new message
    /// (chat / group message / deliver) newer than the delete watermark
    /// re-activates an archived session. Runtime state and action logs never
    /// reach this path with an activity kind, so they cannot un-archive.
    pub(crate) async fn note_records_committed(&self, records: &[MailboxRecord]) {
        for record in records {
            if !is_activity_kind(record.msg_kind) {
                continue;
            }
            let Some(session_id) = record.session_id.as_deref() else {
                continue;
            };
            let state = match self
                .msg_box_db
                .get_owner_session(&record.owner, session_id)
                .await
            {
                Ok(Some(state)) => state,
                Ok(None) => continue,
                Err(error) => {
                    warn!(
                        "msg-center lifecycle lookup failed for {}/{}: {}",
                        record.owner.to_string(),
                        session_id,
                        error
                    );
                    continue;
                }
            };
            if state.lifecycle != SessionLifecycle::Archived {
                continue;
            }
            if let Some(watermark) = DeleteWatermark::from_state(&state) {
                if !watermark.allows(record) {
                    continue;
                }
            }
            let now_ms = Self::now_ms();
            let next = OwnerSessionState {
                lifecycle: SessionLifecycle::Active,
                archived_at_ms: None,
                updated_at_ms: now_ms,
                ..state
            };
            if let Err(error) = self.msg_box_db.upsert_owner_session(&next).await {
                warn!(
                    "msg-center failed to re-activate archived session {}/{}: {}",
                    record.owner.to_string(),
                    session_id,
                    error
                );
            }
        }
    }

    // ------------------------------------------------------------------
    // Projection
    // ------------------------------------------------------------------

    fn clamp_limit_value(limit: Option<usize>, default: usize, max: usize) -> usize {
        limit.unwrap_or(default).clamp(1, max)
    }

    pub(crate) async fn list_sessions_scoped(
        &self,
        owner: DID,
        limit: Option<usize>,
        cursor_value: Option<u64>,
        cursor_session_id: Option<String>,
        with_object: Option<bool>,
        lifecycle: Option<SessionListLifecycleFilter>,
        order_by: Option<SessionListOrder>,
    ) -> std::result::Result<SessionSummaryPage, RPCErrors> {
        let limit =
            Self::clamp_limit_value(limit, DEFAULT_SESSION_LIST_LIMIT, MAX_SESSION_LIST_LIMIT);
        let lifecycle = lifecycle.unwrap_or_default();
        let order_by = order_by.unwrap_or_default();

        let states: HashMap<String, OwnerSessionState> = self
            .msg_box_db
            .list_owner_sessions(&owner)
            .await?
            .into_iter()
            .map(|state| (state.session_id.clone(), state))
            .collect();
        let mut entries: HashMap<String, VisibleSessionIndexEntry> = self
            .msg_box_db
            .list_visible_session_index(&owner)
            .await?
            .into_iter()
            .map(|entry| (entry.session_id.clone(), entry))
            .collect();
        // Registered empty sessions are listed with their registration time
        // as activity so the UI can show "created, no messages yet".
        for state in states.values() {
            if state.registered && !entries.contains_key(&state.session_id) {
                entries.insert(
                    state.session_id.clone(),
                    VisibleSessionIndexEntry {
                        session_id: state.session_id.clone(),
                        updated_at_ms: state.updated_at_ms,
                        last_activity_ms: state.created_at_ms,
                        unread_count: 0,
                        request_count: 0,
                    },
                );
            }
        }

        let mut rows: Vec<VisibleSessionIndexEntry> = entries
            .into_values()
            .filter(|entry| {
                let session_lifecycle = states
                    .get(&entry.session_id)
                    .map(|state| state.lifecycle)
                    .unwrap_or_default();
                match lifecycle {
                    SessionListLifecycleFilter::Active => {
                        session_lifecycle == SessionLifecycle::Active
                    }
                    SessionListLifecycleFilter::Archived => {
                        session_lifecycle == SessionLifecycle::Archived
                    }
                    SessionListLifecycleFilter::All => true,
                }
            })
            .collect();
        let sort_value = |entry: &VisibleSessionIndexEntry| match order_by {
            SessionListOrder::Updated => entry.updated_at_ms,
            SessionListOrder::Activity => entry.last_activity_ms,
        };
        rows.sort_by(|a, b| {
            sort_value(b)
                .cmp(&sort_value(a))
                .then_with(|| b.session_id.cmp(&a.session_id))
        });
        if let Some(cursor_value) = cursor_value {
            let cursor_session_id = cursor_session_id.unwrap_or_default();
            rows.retain(|entry| {
                let value = sort_value(entry);
                value < cursor_value
                    || (value == cursor_value && entry.session_id < cursor_session_id)
            });
        }
        let has_more = rows.len() > limit;
        let page_entries = rows.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_entries.len());
        for entry in page_entries {
            let state = states.get(&entry.session_id).cloned();
            let watermark = state.as_ref().and_then(DeleteWatermark::from_state);
            let last_record = self
                .msg_box_db
                .list_visible_session_records(
                    &owner,
                    &entry.session_id,
                    1,
                    None,
                    None,
                    true,
                    watermark.as_ref(),
                )
                .await?
                .into_iter()
                .next();
            let last_record = match last_record {
                Some(record) => {
                    Some(Self::build_record_view(record, Some(with_object.unwrap_or(false))).await?)
                }
                None => None,
            };
            items.push(SessionSummary {
                session_id: entry.session_id,
                last_record,
                unread_count: entry.unread_count,
                updated_at_ms: entry.updated_at_ms,
                last_activity_ms: entry.last_activity_ms,
                request_count: entry.request_count,
                lifecycle: state.as_ref().map(|s| s.lifecycle).unwrap_or_default(),
                state,
            });
        }

        let (next_cursor_updated_at_ms, next_cursor_session_id) = if has_more {
            items
                .last()
                .map(|item| {
                    let value = match order_by {
                        SessionListOrder::Updated => item.updated_at_ms,
                        SessionListOrder::Activity => item.last_activity_ms,
                    };
                    (Some(value), Some(item.session_id.clone()))
                })
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Ok(SessionSummaryPage {
            items,
            next_cursor_updated_at_ms,
            next_cursor_session_id,
        })
    }

    pub(crate) async fn list_session_scoped(
        &self,
        owner: DID,
        session_id: String,
        limit: Option<usize>,
        cursor_sort_key: Option<u64>,
        cursor_record_id: Option<String>,
        descending: Option<bool>,
        with_object: Option<bool>,
    ) -> std::result::Result<SessionMessagePage, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let descending = descending.unwrap_or(true);
        let limit = Self::clamp_limit_value(limit, DEFAULT_LIST_LIMIT, MAX_LIST_LIMIT);
        let watermark = self
            .msg_box_db
            .get_owner_session(&owner, &session_id)
            .await?
            .as_ref()
            .and_then(DeleteWatermark::from_state);
        let records = self
            .msg_box_db
            .list_visible_session_records(
                &owner,
                &session_id,
                limit + 1,
                cursor_sort_key,
                cursor_record_id.as_deref(),
                descending,
                watermark.as_ref(),
            )
            .await?;
        let has_more = records.len() > limit;
        let page_records = records.into_iter().take(limit).collect::<Vec<_>>();

        let mut items = Vec::with_capacity(page_records.len());
        for record in page_records {
            items.push(
                self.build_session_item(record, with_object.unwrap_or(false))
                    .await?,
            );
        }

        let (next_cursor_sort_key, next_cursor_record_id) = if has_more {
            items
                .last()
                .map(|item| (Some(item.sort_key), Some(item.record_id.clone())))
                .unwrap_or((None, None))
        } else {
            (None, None)
        };

        Ok(SessionMessagePage {
            items,
            next_cursor_sort_key,
            next_cursor_record_id,
        })
    }

    // ------------------------------------------------------------------
    // Owner-scoped UI state
    // ------------------------------------------------------------------

    pub(crate) async fn update_owner_ui_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
        key: String,
        value: Value,
    ) -> std::result::Result<UiSessionStateEntry, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let key = normalize_session_id(&key)?;
        self.msg_box_db
            .upsert_owner_ui_session_state(&owner, &session_id, &key, &value, Self::now_ms())
            .await
    }

    pub(crate) async fn get_owner_ui_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
        key: String,
    ) -> std::result::Result<Option<UiSessionStateEntry>, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        let key = normalize_session_id(&key)?;
        self.msg_box_db
            .get_owner_ui_session_state(&owner, &session_id, &key)
            .await
    }

    pub(crate) async fn list_owner_ui_session_state_internal(
        &self,
        owner: DID,
        session_id: String,
    ) -> std::result::Result<Vec<UiSessionStateEntry>, RPCErrors> {
        let session_id = normalize_session_id(&session_id)?;
        self.msg_box_db
            .list_owner_ui_session_state(&owner, &session_id)
            .await
    }

    /// Owner of a mailbox record id (`owner|box|msg|variant`).
    pub(crate) fn record_owner(record_id: &str) -> std::result::Result<DID, RPCErrors> {
        Self::owner_from_record_id(record_id)
    }
}
