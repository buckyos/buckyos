/**
 * The msg-center RPC shapes MessageHub and Activity read. They mirror the Rust structs in
 * src/kernel/buckyos-api/src/msg_center_client.rs (`msg.*`, `ui_session.*`, `contact.*`, `group.*`).
 */
import type { DID, ndn } from 'buckyos'
import type { ConversationMemoryMode } from 'buckyos/app-frame'

export type MailboxKind = 'INBOX' | 'SENT' | 'GROUP_INBOX' | 'REQUEST_BOX'
export type RecipientState = 'UNREAD' | 'READING' | 'READ' | 'ARCHIVED' | 'DELETED'
export type DeliveryState = 'WAIT' | 'SENDING' | 'SENT' | 'FAILED' | 'DEAD'
export type SessionMessageDirection = 'in' | 'out'
export type SessionDeliveryOverall = 'sending' | 'delivered' | 'partial_failed' | 'failed'
export type SessionLifecycle = 'active' | 'archived'
export type SessionListLifecycleFilter = 'active' | 'archived' | 'all'
export type SessionListOrder = 'updated' | 'activity'

export interface DeliveryError {
  error_code?: string
  message: string
  retryable: boolean
  duplicate_risk: boolean
}

export interface SessionDeliveryTarget {
  target_did: DID
  state: DeliveryState
  attempts: number
  external_msg_id?: string
  last_error?: DeliveryError
}

/** Aggregated delivery view of one outbound message. */
export interface SessionDeliveryView {
  overall: SessionDeliveryOverall
  per_target?: SessionDeliveryTarget[]
}

export interface IngressContext {
  transport_did?: DID
  platform?: string
  chat_id?: string
  source_account_id?: string
  context_id?: string
  contact_mgr_owner?: DID
}

/** A mailbox owner's reference to one immutable ndn.MsgObject. */
export interface MailboxRecord {
  mailbox: string
  record_id: string
  owner: DID
  box_kind: MailboxKind
  msg_id: string
  msg_kind?: string
  state: RecipientState
  from: DID
  from_name?: string
  to: DID
  session_id?: string
  sort_key: number
  tags?: string[]
  ingress?: IngressContext
  created_at_ms: number
  updated_at_ms: number
}

export interface MailboxRecordWithObject {
  record: MailboxRecord
  msg?: ndn.MsgObject | null
}

/** Owner-scoped session metadata (`msg.get_session_state`). */
export interface OwnerSessionState {
  owner: DID
  session_id: string
  lifecycle: SessionLifecycle
  registered: boolean
  origin?: string
  peer_did?: DID
  binding?: unknown
  title?: string
  /** Absent: normal. */
  memory_mode?: ConversationMemoryMode
  archived_at_ms?: number
  delete_watermark_sort_key?: number
  delete_watermark_record_id?: string
  deleted_at_ms?: number
  created_at_ms: number
  updated_at_ms: number
}

/** A conversation of the owner with an Agent, from the Agent's mailbox (`msg.list_conversations`). */
export interface PeerConversation {
  /** The session in the Agent's mailbox. */
  session_id: string
  /** `native`, or the platform of the channel. */
  platform: string
  /** The owner's endpoint the Agent replies to, and the session there. */
  to: DID
  to_session?: string
  last_active_at_ms: number
}

/** The default session of the owner with an Agent. */
export interface DefaultSession {
  /** `native`: the owner's own session; otherwise the session in the Agent's mailbox. */
  session_id: string
  platform: string
  to: DID
  to_session?: string
  set_at_ms: number
}

export interface PeerConversations {
  /** The stored one; absent: none is stored yet, or it was deleted. */
  default?: DefaultSession
  conversations: PeerConversation[]
}

export interface SessionSummary {
  session_id: string
  last_record?: MailboxRecordWithObject
  unread_count: number
  updated_at_ms: number
  /** Last effective message activity; registration time for empty sessions. */
  last_activity_ms: number
  request_count: number
  lifecycle: SessionLifecycle
  state?: OwnerSessionState
}

export interface SessionSummaryPage {
  items?: SessionSummary[]
  next_cursor_updated_at_ms?: number
  next_cursor_session_id?: string
}

/** One timeline entry of `msg.list_session`. */
export interface SessionMessageItem {
  record_id: string
  msg_id: string
  direction: SessionMessageDirection
  box_kind: MailboxKind
  sort_key: number
  from: DID
  to: DID
  recipient_state?: RecipientState
  delivery?: SessionDeliveryView
  msg?: ndn.MsgObject | null
}

export interface SessionMessagePage {
  items?: SessionMessageItem[]
  next_cursor_sort_key?: number
  next_cursor_record_id?: string
}

export interface ListSessionsRequest {
  owner: DID
  limit?: number
  cursor_updated_at_ms?: number
  cursor_session_id?: string
  with_object?: boolean
  lifecycle?: SessionListLifecycleFilter
  order_by?: SessionListOrder
}

export interface ListSessionRequest {
  owner: DID
  session_id: string
  limit?: number
  cursor_sort_key?: number
  cursor_record_id?: string
  descending?: boolean
  with_object?: boolean
}

export interface PostSendDelivery {
  delivery_id: string
  transport_did: DID
  target_did: DID
  transport: { kind: 'native' } | { kind: 'tunnel'; platform: string; tunnel_instance_id: string }
}

export interface PostSendResult {
  ok: boolean
  msg_id: string
  deliveries?: PostSendDelivery[]
  reason?: string
}

export interface UiSessionStateEntry {
  session_id: string
  key: string
  value: unknown
  updated_at_ms: number
}

export interface AccountBinding {
  platform: string
  account_id: string
  display_id: string
  tunnel_instance_id: string
  account_type?: string
  endpoint_did?: DID
  last_active_at: number
  meta?: Record<string, string>
}

export type AccessGroupLevel = 'block' | 'stranger' | 'temporary' | 'allow'

export interface Contact {
  did: DID
  name: string
  avatar?: string
  note?: string
  source: string
  is_verified: boolean
  bindings?: AccountBinding[]
  access_level: AccessGroupLevel
  temp_grants?: Array<{ context_id: string; granted_at: number; expires_at: number }>
  friend_state?: 'none' | 'outgoing_pending' | 'incoming_pending' | 'friend'
  tags?: string[]
  created_at: number
  updated_at: number
}

export interface MailboxRecordPage {
  items?: MailboxRecordWithObject[]
  next_cursor_sort_key?: number
  next_cursor_record_id?: string
}

/* ── Groups the caller participates in (`group.list_by_member`) ── */

export interface GroupDoc {
  id: DID
  owner: DID
  controller: DID
  host: DID
  profile?: { name?: unknown; description?: unknown } | null
  join_policy?: string
  revision: string
  lifecycle: 'active' | 'archived' | 'deleted'
}

export interface GroupDocEnvelope {
  obj_id: string
  doc: GroupDoc
}

export interface JoinedGroupSummary {
  group_did: DID
  stopped?: boolean
  doc_cache?: GroupDocEnvelope | GroupDoc | null
}

export interface GroupMemberRecord {
  member_did: DID
  role: 'owner' | 'admin' | 'member'
  state: 'invited' | 'pending_admin_approval' | 'active' | 'left' | 'removed' | 'rejected' | 'expired' | 'revoked'
  entity_kind: string
  invitation_id?: string | null
  invited_by?: DID | null
  expires_at_ms?: number | null
}

/** A Session invitation waiting for the caller's acceptance (msg-center `list_by_member`). */
export interface PendingSessionInvitation {
  group_did: DID
  group_name?: string | null
  session_id: string
  title?: string | null
  invited_by?: DID
}

export interface GroupsByMember {
  items?: GroupDocEnvelope[]
  joined?: JoinedGroupSummary[]
  invitations?: Array<{ group_did?: DID; doc?: GroupDoc; member?: Partial<GroupMemberRecord> }>
  session_invitations?: PendingSessionInvitation[]
}
