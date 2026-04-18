use crate::{ControlPanelServer, RpcAuthPrincipal};
use ::kRPC::{RPCErrors, RPCRequest, RPCResponse, RPCResult};
use buckyos_api::{
    get_buckyos_api_runtime, AccessGroupLevel, BoxKind, Contact, ContactQuery, Event, KEventClient,
    MsgCenterClient, MsgRecordWithObject, MsgState, SendContext, UserType,
    CONTROL_PANEL_SERVICE_NAME,
};
use bytes::Bytes;
use cyfs_gateway_lib::{server_err, ServerError, ServerErrorCode, ServerResult};
use futures::{stream, TryStreamExt};
use http::header::{CACHE_CONTROL, CONTENT_TYPE};
use http::StatusCode;
use http_body_util::{combinators::BoxBody, BodyExt, StreamBody};
use hyper::body::Frame;
use name_lib::DID;
use ndn_lib::{MsgContent, MsgContentFormat, MsgObjKind, MsgObject};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use tokio::sync::mpsc;
use uuid::Uuid;

const DEFAULT_CHAT_CONTACT_LIMIT: usize = 100;
const DEFAULT_CHAT_MESSAGE_LIMIT: usize = 60;
const MAX_CHAT_MESSAGE_LIMIT: usize = 100;
const MAX_CHAT_SCAN_LIMIT: usize = 240;
const DEFAULT_CHAT_STREAM_KEEPALIVE_MS: u64 = 15_000;
const MIN_CHAT_STREAM_KEEPALIVE_MS: u64 = 5_000;
const MAX_CHAT_STREAM_KEEPALIVE_MS: u64 = 60_000;

#[derive(Clone, Serialize)]
pub(crate) struct ChatScopeInfo {
    username: String,
    owner_did: String,
    access_mode: &'static str,
}

#[derive(Clone, Serialize)]
struct ChatCapabilityInfo {
    contact_list: bool,
    message_list: bool,
    message_send: bool,
    thread_id_send: bool,
    realtime_events: bool,
    standalone_chat_app_link: bool,
    opendan_channel_ready: bool,
}

#[derive(Clone, Serialize)]
struct ChatBootstrapResponse {
    scope: ChatScopeInfo,
    capabilities: ChatCapabilityInfo,
    notes: Vec<String>,
}

#[derive(Clone, Serialize)]
struct ChatBindingView {
    platform: String,
    account_id: String,
    display_id: String,
    tunnel_id: String,
    last_active_at: u64,
    meta: HashMap<String, String>,
}

#[derive(Clone, Serialize)]
struct ChatContactView {
    did: String,
    name: String,
    avatar: Option<String>,
    note: Option<String>,
    access_level: &'static str,
    is_verified: bool,
    groups: Vec<String>,
    tags: Vec<String>,
    created_at: u64,
    updated_at: u64,
    bindings: Vec<ChatBindingView>,
}

#[derive(Clone, Serialize)]
struct ChatContactListResponse {
    scope: ChatScopeInfo,
    items: Vec<ChatContactView>,
}

#[derive(Clone, Serialize)]
pub(crate) struct ChatMessageView {
    pub(crate) record_id: String,
    pub(crate) msg_id: String,
    pub(crate) direction: &'static str,
    pub(crate) peer_did: String,
    pub(crate) peer_name: Option<String>,
    pub(crate) state: &'static str,
    pub(crate) created_at_ms: u64,
    pub(crate) updated_at_ms: u64,
    pub(crate) sort_key: u64,
    pub(crate) thread_id: Option<String>,
    pub(crate) content: String,
    pub(crate) content_format: Option<String>,
}

#[derive(Clone, Serialize)]
struct ChatMessageListResponse {
    scope: ChatScopeInfo,
    peer_did: String,
    peer_name: Option<String>,
    items: Vec<ChatMessageView>,
}

#[derive(Clone, Serialize)]
struct ChatSendMessageResponse {
    scope: ChatScopeInfo,
    target_did: String,
    delivery_count: usize,
    message: ChatMessageView,
}

#[derive(Clone, Deserialize)]
struct ChatStreamHttpRequest {
    #[serde(default)]
    session_token: Option<String>,
    peer_did: String,
    #[serde(default)]
    thread_id: Option<String>,
    #[serde(default)]
    keepalive_ms: Option<u64>,
}

#[derive(Clone, Deserialize)]
struct MsgCenterBoxChangedEvent {
    #[serde(default)]
    operation: Option<String>,
    #[serde(default)]
    record_id: Option<String>,
}

impl ControlPanelServer {
    pub(crate) fn require_chat_principal(
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<&RpcAuthPrincipal, RPCErrors> {
        principal
            .ok_or_else(|| RPCErrors::InvalidToken("missing authenticated principal".to_string()))
    }

    pub(crate) fn parse_chat_owner_did(principal: &RpcAuthPrincipal) -> Result<DID, RPCErrors> {
        DID::from_str(principal.owner_did.as_str()).map_err(|error| {
            RPCErrors::ReasonError(format!(
                "invalid chat owner DID `{}`: {}",
                principal.owner_did, error
            ))
        })
    }

    fn chat_scope_info(principal: &RpcAuthPrincipal) -> ChatScopeInfo {
        ChatScopeInfo {
            username: principal.username.clone(),
            owner_did: principal.owner_did.clone(),
            access_mode: match principal.user_type {
                UserType::Root | UserType::Admin => "full_access",
                UserType::User | UserType::Limited | UserType::Guest => "read_only",
            },
        }
    }

    pub(crate) async fn get_msg_center_client(&self) -> Result<MsgCenterClient, RPCErrors> {
        let runtime = get_buckyos_api_runtime()?;
        runtime.get_msg_center_client().await.map_err(|error| {
            RPCErrors::ReasonError(format!("get msg-center client failed: {}", error))
        })
    }

    fn get_chat_kevent_client() -> KEventClient {
        static CHAT_KEVENT_CLIENT: OnceLock<KEventClient> = OnceLock::new();
        CHAT_KEVENT_CLIENT
            .get_or_init(|| KEventClient::new_full(CONTROL_PANEL_SERVICE_NAME, None))
            .clone()
    }

    fn normalize_chat_stream_keepalive_ms(keepalive_ms: Option<u64>) -> u64 {
        keepalive_ms
            .unwrap_or(DEFAULT_CHAT_STREAM_KEEPALIVE_MS)
            .clamp(MIN_CHAT_STREAM_KEEPALIVE_MS, MAX_CHAT_STREAM_KEEPALIVE_MS)
    }

    fn build_chat_stream_response(
        receiver: mpsc::Receiver<std::result::Result<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let stream = stream::unfold(receiver, |mut receiver| async move {
            receiver.recv().await.map(|item| (item, receiver))
        });
        let body = StreamBody::new(stream.map_ok(Frame::data));

        http::Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, "application/x-ndjson")
            .header(CACHE_CONTROL, "no-store")
            .header("X-Accel-Buffering", "no")
            .body(BodyExt::map_err(body, |error| error).boxed())
            .map_err(|error| {
                server_err!(
                    ServerErrorCode::InvalidData,
                    "Failed to build chat stream response: {}",
                    error
                )
            })
    }

    async fn send_chat_stream_json<T: Serialize>(
        sender: &mpsc::Sender<std::result::Result<Bytes, ServerError>>,
        payload: &T,
    ) -> bool {
        let mut body = match serde_json::to_vec(payload) {
            Ok(body) => body,
            Err(error) => {
                let _ = sender
                    .send(Err(server_err!(
                        ServerErrorCode::EncodeError,
                        "Failed to serialize chat stream payload: {}",
                        error
                    )))
                    .await;
                return false;
            }
        };
        body.push(b'\n');
        sender.send(Ok(Bytes::from(body))).await.is_ok()
    }

    async fn send_chat_stream_error(
        sender: &mpsc::Sender<std::result::Result<Bytes, ServerError>>,
        message: String,
    ) -> bool {
        Self::send_chat_stream_json(
            sender,
            &json!({
                "type": "error",
                "message": message,
                "at_ms": Self::current_time_ms(),
            }),
        )
        .await
    }

    fn chat_access_level_label(level: &AccessGroupLevel) -> &'static str {
        match level {
            AccessGroupLevel::Block => "block",
            AccessGroupLevel::Stranger => "stranger",
            AccessGroupLevel::Temporary => "temporary",
            AccessGroupLevel::Friend => "friend",
        }
    }

    fn chat_msg_state_label(state: &MsgState) -> &'static str {
        match state {
            MsgState::Unread => "unread",
            MsgState::Reading => "reading",
            MsgState::Readed => "readed",
            MsgState::Wait => "wait",
            MsgState::Sending => "sending",
            MsgState::Sent => "sent",
            MsgState::Failed => "failed",
            MsgState::Dead => "dead",
            MsgState::Deleted => "deleted",
            MsgState::Archived => "archived",
        }
    }

    fn normalize_chat_contact_limit(limit: Option<usize>) -> usize {
        limit
            .unwrap_or(DEFAULT_CHAT_CONTACT_LIMIT)
            .clamp(1, DEFAULT_CHAT_CONTACT_LIMIT)
    }

    fn normalize_chat_message_limit(limit: Option<usize>) -> usize {
        limit
            .unwrap_or(DEFAULT_CHAT_MESSAGE_LIMIT)
            .clamp(1, MAX_CHAT_MESSAGE_LIMIT)
    }

    fn chat_scan_limit(message_limit: usize) -> usize {
        message_limit
            .saturating_mul(4)
            .clamp(40, MAX_CHAT_SCAN_LIMIT)
    }

    fn current_time_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64
    }

    fn map_chat_contact(contact: Contact) -> ChatContactView {
        ChatContactView {
            did: contact.did.to_string(),
            name: contact.name,
            avatar: contact.avatar,
            note: contact.note,
            access_level: Self::chat_access_level_label(&contact.access_level),
            is_verified: contact.is_verified,
            groups: contact.groups,
            tags: contact.tags,
            created_at: contact.created_at,
            updated_at: contact.updated_at,
            bindings: contact
                .bindings
                .into_iter()
                .map(|binding| ChatBindingView {
                    platform: binding.platform,
                    account_id: binding.account_id,
                    display_id: binding.display_id,
                    tunnel_id: binding.tunnel_id,
                    last_active_at: binding.last_active_at,
                    meta: binding.meta,
                })
                .collect(),
        }
    }

    fn chat_message_thread_id(record: &MsgRecordWithObject) -> Option<String> {
        record
            .record
            .ui_session_id
            .clone()
            .or_else(|| record.msg.as_ref().and_then(|msg| msg.thread.topic.clone()))
            .or_else(|| {
                record
                    .msg
                    .as_ref()
                    .and_then(|msg| msg.thread.correlation_id.clone())
            })
            .or_else(|| {
                record.msg.as_ref().and_then(|msg| {
                    msg.meta
                        .get("session_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                })
            })
            .or_else(|| {
                record.msg.as_ref().and_then(|msg| {
                    msg.meta
                        .get("owner_session_id")
                        .and_then(|value| value.as_str())
                        .map(|value| value.trim().to_string())
                        .filter(|value| !value.is_empty())
                })
            })
    }

    pub(crate) fn chat_record_matches_peer(
        record: &MsgRecordWithObject,
        owner_did: &DID,
        peer_did: &DID,
    ) -> bool {
        if record.record.msg_kind != MsgObjKind::Chat {
            return false;
        }

        if record.record.from == *owner_did {
            record.record.to == *peer_did
        } else {
            record.record.from == *peer_did
        }
    }

    fn chat_record_matches_stream(
        record: &MsgRecordWithObject,
        owner_did: &DID,
        peer_did: &DID,
        thread_id: Option<&str>,
    ) -> bool {
        if !Self::chat_record_matches_peer(record, owner_did, peer_did) {
            return false;
        }

        match thread_id {
            Some(thread_id) => Self::chat_message_thread_id(record).as_deref() == Some(thread_id),
            None => true,
        }
    }

    fn chat_record_id_from_event(event: &Event) -> Option<String> {
        serde_json::from_value::<MsgCenterBoxChangedEvent>(event.data.clone())
            .ok()
            .and_then(|payload| payload.record_id)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    }

    fn chat_event_operation(event: &Event) -> String {
        serde_json::from_value::<MsgCenterBoxChangedEvent>(event.data.clone())
            .ok()
            .and_then(|payload| payload.operation)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "changed".to_string())
    }

    pub(crate) fn map_chat_message_record(
        record: &MsgRecordWithObject,
        owner_did: &DID,
        peer_name: Option<String>,
    ) -> ChatMessageView {
        let direction = if record.record.from == *owner_did {
            "outbound"
        } else {
            "inbound"
        };
        let peer_name = peer_name.or_else(|| {
            if direction == "inbound" {
                record.record.from_name.clone()
            } else {
                None
            }
        });
        let peer_did = if direction == "outbound" {
            record.record.to.to_string()
        } else {
            record.record.from.to_string()
        };
        let (content, content_format) = match record.msg.as_ref() {
            Some(msg) => (
                msg.content.content.clone(),
                msg.content
                    .format
                    .as_ref()
                    .map(|format| format!("{:?}", format)),
            ),
            None => (String::new(), None),
        };

        ChatMessageView {
            record_id: record.record.record_id.clone(),
            msg_id: record.record.msg_id.to_string(),
            direction,
            peer_did,
            peer_name,
            state: Self::chat_msg_state_label(&record.record.state),
            created_at_ms: record.record.created_at_ms,
            updated_at_ms: record.record.updated_at_ms,
            sort_key: record.record.sort_key,
            thread_id: Self::chat_message_thread_id(record),
            content,
            content_format,
        }
    }

    pub(crate) async fn handle_chat_bootstrap(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let can_send = matches!(principal.user_type, UserType::Root | UserType::Admin);
        let response = ChatBootstrapResponse {
            scope: Self::chat_scope_info(principal),
            capabilities: ChatCapabilityInfo {
                contact_list: true,
                message_list: true,
                message_send: can_send,
                thread_id_send: can_send,
                realtime_events: false,
                standalone_chat_app_link: true,
                opendan_channel_ready: false,
            },
            notes: vec![
                "Message Hub currently uses a browser-safe wrapper over msg-center."
                    .to_string(),
                "The current standalone route is /message-hub/chat while the backend adapter remains in transition."
                    .to_string(),
                "Future email, calendar, notification, TODO, and agent record views remain follow-up work."
                    .to_string(),
            ],
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(response)),
            req.seq,
        ))
    }

    pub(crate) async fn handle_chat_contact_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let msg_center = self.get_msg_center_client().await?;
        let limit = Self::normalize_chat_contact_limit(Self::param_usize(&req, "limit"));
        let keyword = Self::param_str(&req, "keyword")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let query = ContactQuery {
            keyword,
            limit: Some(limit),
            offset: Self::param_u64(&req, "offset"),
            ..Default::default()
        };

        let mut items = msg_center
            .list_contacts(query, Some(owner_did))
            .await?
            .into_iter()
            .map(Self::map_chat_contact)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| {
            right
                .updated_at
                .cmp(&left.updated_at)
                .then_with(|| left.name.cmp(&right.name))
        });

        Ok(RPCResponse::new(
            RPCResult::Success(json!(ChatContactListResponse {
                scope: Self::chat_scope_info(principal),
                items,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_chat_message_list(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let peer_did_raw = Self::require_param_str(&req, "peer_did")?;
        let peer_did = DID::from_str(peer_did_raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!("Invalid peer_did `{}`: {}", peer_did_raw, error))
        })?;
        let limit = Self::normalize_chat_message_limit(Self::param_usize(&req, "limit"));
        let scan_limit = Self::chat_scan_limit(limit);
        let msg_center = self.get_msg_center_client().await?;

        let peer_name = match msg_center
            .get_contact(peer_did.clone(), Some(owner_did.clone()))
            .await
        {
            Ok(Some(contact)) => Some(contact.name),
            Ok(None) => None,
            Err(error) => {
                log::warn!(
                    "chat.message.list get_contact failed: peer={:?} owner={:?} err={}",
                    peer_did,
                    owner_did,
                    error
                );
                None
            }
        };

        let inbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Inbox,
                None,
                Some(scan_limit),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;
        let outbox = msg_center
            .list_box_by_time(
                owner_did.clone(),
                BoxKind::Outbox,
                None,
                Some(scan_limit),
                None,
                None,
                Some(true),
                Some(true),
            )
            .await?;

        let mut records = inbox
            .items
            .into_iter()
            .chain(outbox.items.into_iter())
            .filter(|record| Self::chat_record_matches_peer(record, &owner_did, &peer_did))
            .collect::<Vec<_>>();
        records.sort_by(|left, right| {
            right
                .record
                .sort_key
                .cmp(&left.record.sort_key)
                .then_with(|| right.record.updated_at_ms.cmp(&left.record.updated_at_ms))
                .then_with(|| right.record.record_id.cmp(&left.record.record_id))
        });
        records.truncate(limit);

        let items = records
            .iter()
            .map(|record| Self::map_chat_message_record(record, &owner_did, peer_name.clone()))
            .collect::<Vec<_>>();

        Ok(RPCResponse::new(
            RPCResult::Success(json!(ChatMessageListResponse {
                scope: Self::chat_scope_info(principal),
                peer_did: peer_did.to_string(),
                peer_name,
                items,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_chat_message_send(
        &self,
        req: RPCRequest,
        principal: Option<&RpcAuthPrincipal>,
    ) -> Result<RPCResponse, RPCErrors> {
        let principal = Self::require_chat_principal(principal)?;
        let owner_did = Self::parse_chat_owner_did(principal)?;
        let target_did_raw = Self::require_param_str(&req, "target_did")?;
        let target_did = DID::from_str(target_did_raw.trim()).map_err(|error| {
            RPCErrors::ParseRequestError(format!(
                "Invalid target_did `{}`: {}",
                target_did_raw, error
            ))
        })?;
        let content = Self::require_param_str(&req, "content")?.trim().to_string();
        if content.is_empty() {
            return Err(RPCErrors::ParseRequestError(
                "content cannot be empty".to_string(),
            ));
        }
        let thread_id = Self::param_str(&req, "thread_id")
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let msg_center = self.get_msg_center_client().await?;

        let peer_name = match msg_center
            .get_contact(target_did.clone(), Some(owner_did.clone()))
            .await
        {
            Ok(Some(contact)) => Some(contact.name),
            Ok(None) => None,
            Err(error) => {
                log::warn!(
                    "chat.message.send get_contact failed: peer={:?} owner={:?} err={}",
                    target_did,
                    owner_did,
                    error
                );
                None
            }
        };

        let mut message = MsgObject {
            from: owner_did.clone(),
            to: vec![target_did.clone()],
            kind: MsgObjKind::Chat,
            created_at_ms: Self::current_time_ms(),
            content: MsgContent {
                format: Some(MsgContentFormat::TextPlain),
                content: content.clone(),
                ..Default::default()
            },
            ..Default::default()
        };
        if let Some(thread_id) = thread_id.as_ref() {
            message.thread.topic = Some(thread_id.clone());
            message.thread.correlation_id = Some(thread_id.clone());
            message
                .meta
                .insert("session_id".to_string(), Value::String(thread_id.clone()));
            message.meta.insert(
                "owner_session_id".to_string(),
                Value::String(thread_id.clone()),
            );
        }

        let result = msg_center
            .post_send(
                message.clone(),
                Some(SendContext {
                    contact_mgr_owner: Some(owner_did.clone()),
                    ..Default::default()
                }),
                None,
            )
            .await?;
        if !result.ok {
            return Err(RPCErrors::ReasonError(result.reason.unwrap_or_else(|| {
                "msg-center rejected the chat send request".to_string()
            })));
        }

        let first_delivery = result.deliveries.first().ok_or_else(|| {
            RPCErrors::ReasonError("msg-center returned no delivery record".to_string())
        })?;
        let stored_record = msg_center
            .get_record(first_delivery.record_id.clone(), Some(true))
            .await?;
        let mapped_message = if let Some(record) = stored_record.as_ref() {
            Self::map_chat_message_record(record, &owner_did, peer_name.clone())
        } else {
            ChatMessageView {
                record_id: first_delivery.record_id.clone(),
                msg_id: result.msg_id.to_string(),
                direction: "outbound",
                peer_did: target_did.to_string(),
                peer_name,
                state: "sent",
                created_at_ms: message.created_at_ms,
                updated_at_ms: message.created_at_ms,
                sort_key: message.created_at_ms,
                thread_id,
                content,
                content_format: Some("TextPlain".to_string()),
            }
        };

        Ok(RPCResponse::new(
            RPCResult::Success(json!(ChatSendMessageResponse {
                scope: Self::chat_scope_info(principal),
                target_did: target_did.to_string(),
                delivery_count: result.deliveries.len(),
                message: mapped_message,
            })),
            req.seq,
        ))
    }

    pub(crate) async fn handle_chat_stream_http(
        &self,
        req: http::Request<BoxBody<Bytes, ServerError>>,
    ) -> ServerResult<http::Response<BoxBody<Bytes, ServerError>>> {
        let fallback_token = Self::extract_http_session_token(&req);
        let collected = req.into_body().collect().await.map_err(|error| {
            server_err!(
                ServerErrorCode::BadRequest,
                "Failed to read chat stream request body: {}",
                error
            )
        })?;
        let body = collected.to_bytes();
        let stream_req = match serde_json::from_slice::<ChatStreamHttpRequest>(&body) {
            Ok(value) => value,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error": format!("Invalid chat stream request: {}", error),
                    }),
                );
            }
        };

        let peer_did_raw = stream_req.peer_did.trim().to_string();
        if peer_did_raw.is_empty() {
            return Self::build_http_json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "peer_did is required" }),
            );
        }
        let peer_did = match DID::from_str(peer_did_raw.as_str()) {
            Ok(value) => value,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error": format!("Invalid peer_did `{}`: {}", peer_did_raw, error),
                    }),
                );
            }
        };

        let thread_id = stream_req
            .thread_id
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty());
        let keepalive_ms = Self::normalize_chat_stream_keepalive_ms(stream_req.keepalive_ms);
        let principal = match self
            .authenticate_session_token_for_method(
                "chat.stream",
                stream_req.session_token.or(fallback_token),
            )
            .await
        {
            Ok(Some(principal)) => principal,
            Ok(None) => {
                return Self::build_http_json_response(
                    StatusCode::UNAUTHORIZED,
                    json!({ "error": "chat stream requires an authenticated session" }),
                );
            }
            Err(error) => {
                let status = match error {
                    RPCErrors::InvalidToken(_) => StatusCode::UNAUTHORIZED,
                    RPCErrors::NoPermission(_) => StatusCode::FORBIDDEN,
                    _ => StatusCode::BAD_REQUEST,
                };
                return Self::build_http_json_response(
                    status,
                    json!({ "error": error.to_string() }),
                );
            }
        };
        let owner_did = match Self::parse_chat_owner_did(&principal) {
            Ok(value) => value,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": error.to_string() }),
                );
            }
        };

        let owner_token = owner_did.to_raw_host_name();
        let patterns = vec![
            format!("/msg_center/{}/box/in/**", owner_token),
            format!("/msg_center/{}/box/out/**", owner_token),
        ];
        let event_reader = match Self::get_chat_kevent_client()
            .create_event_reader(patterns)
            .await
        {
            Ok(reader) => reader,
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({
                        "error": format!("Failed to create chat event reader: {}", error),
                    }),
                );
            }
        };

        let (sender, receiver) = mpsc::channel::<std::result::Result<Bytes, ServerError>>(32);
        let scope = Self::chat_scope_info(&principal);
        let peer_name = match self.get_msg_center_client().await {
            Ok(msg_center) => match msg_center
                .get_contact(peer_did.clone(), Some(owner_did.clone()))
                .await
            {
                Ok(Some(contact)) => Some(contact.name),
                Ok(None) => None,
                Err(error) => {
                    log::warn!(
                        "chat.stream get_contact failed: peer={:?} owner={:?} err={}",
                        peer_did,
                        owner_did,
                        error
                    );
                    None
                }
            },
            Err(error) => {
                return Self::build_http_json_response(
                    StatusCode::BAD_REQUEST,
                    json!({ "error": error.to_string() }),
                );
            }
        };

        if !Self::send_chat_stream_json(
            &sender,
            &json!({
                "type": "ack",
                "connection_id": Uuid::new_v4().to_string(),
                "scope": scope,
                "peer_did": peer_did.to_string(),
                "thread_id": thread_id.clone(),
                "keepalive_ms": keepalive_ms,
                "at_ms": Self::current_time_ms(),
            }),
        )
        .await
        {
            return Self::build_http_json_response(
                StatusCode::BAD_REQUEST,
                json!({ "error": "Failed to initialize chat stream" }),
            );
        }

        let server = self.clone();
        tokio::spawn(async move {
            let msg_center = match server.get_msg_center_client().await {
                Ok(client) => client,
                Err(error) => {
                    let _ = Self::send_chat_stream_error(&sender, error.to_string()).await;
                    return;
                }
            };

            loop {
                let event = match event_reader.pull_event(Some(keepalive_ms)).await {
                    Ok(Some(event)) => event,
                    Ok(None) => {
                        if !Self::send_chat_stream_json(
                            &sender,
                            &json!({
                                "type": "keepalive",
                                "at_ms": Self::current_time_ms(),
                            }),
                        )
                        .await
                        {
                            return;
                        }
                        continue;
                    }
                    Err(error) => {
                        let _ = Self::send_chat_stream_error(
                            &sender,
                            format!("chat event reader failed: {}", error),
                        )
                        .await;
                        return;
                    }
                };

                let record_id = match Self::chat_record_id_from_event(&event) {
                    Some(record_id) => record_id,
                    None => {
                        if !Self::send_chat_stream_json(
                            &sender,
                            &json!({
                                "type": "resync",
                                "reason": "missing_record_id",
                                "peer_did": peer_did.to_string(),
                                "thread_id": thread_id.clone(),
                                "at_ms": Self::current_time_ms(),
                            }),
                        )
                        .await
                        {
                            return;
                        }
                        continue;
                    }
                };

                let record = match msg_center.get_record(record_id.clone(), Some(true)).await {
                    Ok(Some(record)) => record,
                    Ok(None) => {
                        if !Self::send_chat_stream_json(
                            &sender,
                            &json!({
                                "type": "resync",
                                "reason": "record_not_found",
                                "record_id": record_id,
                                "peer_did": peer_did.to_string(),
                                "thread_id": thread_id.clone(),
                                "at_ms": Self::current_time_ms(),
                            }),
                        )
                        .await
                        {
                            return;
                        }
                        continue;
                    }
                    Err(error) => {
                        let _ = Self::send_chat_stream_error(
                            &sender,
                            format!("Failed to load chat record {}: {}", record_id, error),
                        )
                        .await;
                        return;
                    }
                };

                if !Self::chat_record_matches_stream(
                    &record,
                    &owner_did,
                    &peer_did,
                    thread_id.as_deref(),
                ) {
                    continue;
                }

                let message = Self::map_chat_message_record(&record, &owner_did, peer_name.clone());
                if !Self::send_chat_stream_json(
                    &sender,
                    &json!({
                        "type": "message",
                        "operation": Self::chat_event_operation(&event),
                        "record_id": record.record.record_id,
                        "message": message,
                        "at_ms": Self::current_time_ms(),
                    }),
                )
                .await
                {
                    return;
                }
            }
        });

        Self::build_chat_stream_response(receiver)
    }
}
