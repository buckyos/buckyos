import { buckyos } from 'buckyos'
import { fetchUserDetail } from '../../../api/user_mgr'
import type { DID, MsgObject } from '../protocol/msgobj'

/**
 * Thin client for the msg-center RPC surface used by MessageHub. Wire shapes
 * mirror the Rust structs in src/kernel/buckyos-api/src/msg_center_client.rs
 * (`msg.*`, `ui_session.*`, `contact.*`, `group.*`).
 */

const MSG_CENTER_SERVICE = 'msg-center'

/* ── Wire types ── */

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

/** A mailbox owner's reference to one immutable MsgObject. */
export interface MailboxRecord {
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
  msg?: MsgObject | null
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
  archived_at_ms?: number
  delete_watermark_sort_key?: number
  delete_watermark_record_id?: string
  deleted_at_ms?: number
  created_at_ms: number
  updated_at_ms: number
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
  msg?: MsgObject | null
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

export type AccessGroupLevel = 'block' | 'stranger' | 'temporary' | 'friend'

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
  groups?: string[]
  tags?: string[]
  created_at: number
  updated_at: number
}

export interface GroupSummary {
  group_did: DID
  name: string
  avatar?: string
  host_zone: DID
  owner: DID
  purpose: string
  entity_kind: string
  member_count: number
  is_hosted_by_self: boolean
  can_message: boolean
  updated_at_ms: number
}

export interface GroupAccessDecision {
  action: string
  allowed: boolean
  reason?: string
  effective_role?: string
}

export interface MailboxRecordPage {
  items?: MailboxRecordWithObject[]
  next_cursor_sort_key?: number
  next_cursor_record_id?: string
}

/* ── Errors ── */

export class MessageHubApiError extends Error {
  readonly kind: 'permission_denied' | 'not_found' | 'invalid' | 'unavailable' | 'unknown'

  constructor(message: string, kind: MessageHubApiError['kind'] = 'unknown') {
    super(message)
    this.name = 'MessageHubApiError'
    this.kind = kind
  }
}

function classifyError(error: unknown): MessageHubApiError {
  if (error instanceof MessageHubApiError) return error
  const message = error instanceof Error ? error.message : String(error)
  const lower = message.toLowerCase()
  if (lower.includes('no permission') || lower.includes('permission')) return new MessageHubApiError(message, 'permission_denied')
  if (lower.includes('not found')) return new MessageHubApiError(message, 'not_found')
  if (lower.includes('invalid') || lower.includes('parse')) return new MessageHubApiError(message, 'invalid')
  if (lower.includes('failed to fetch') || lower.includes('network') || lower.includes('timeout')) return new MessageHubApiError(message, 'unavailable')
  return new MessageHubApiError(message)
}

/* ── RPC plumbing ── */

function rpc() {
  return buckyos.getServiceRpcClient(MSG_CENTER_SERVICE)
}

async function call<TResult>(method: string, params: Record<string, unknown>): Promise<TResult> {
  try {
    return await rpc().call<TResult, Record<string, unknown>>(method, params)
  } catch (error) {
    throw classifyError(error)
  }
}

export async function fetchOwnerDid(): Promise<string | null> {
  const accountInfo = await buckyos.getAccountInfo()
  const userId = accountInfo?.user_id
  if (!userId) return null
  const { data, error } = await fetchUserDetail({ userId })
  if (error) throw classifyError(error)
  const did = data?.local_profile?.did ?? data?.profile?.did
  if (typeof did !== 'string' || !did.startsWith('did:')) {
    throw new MessageHubApiError('user profile has no owner DID', 'permission_denied')
  }
  return did
}

export async function currentSessionToken(): Promise<string | null> {
  const token = rpc().getSessionToken()
  if (token) return token
  const accountInfo = await buckyos.getAccountInfo()
  return accountInfo?.session_token ?? null
}

/** Base URL of the msg-center service (`https://zone/kapi/msg-center`). */
export function msgCenterServiceUrl(): string {
  return buckyos.getZoneServiceURL(MSG_CENTER_SERVICE)
}

/* ── Session projection ── */

export const listSessions = (request: ListSessionsRequest) => call<SessionSummaryPage | null>('msg.list_sessions', request as unknown as Record<string, unknown>).then(page => page ?? {})
export const listSessionMessages = (request: ListSessionRequest) => call<SessionMessagePage | null>('msg.list_session', request as unknown as Record<string, unknown>).then(page => page ?? {})
export const getSessionState = (owner: DID, sessionId: string) => call<OwnerSessionState | null>('msg.get_session_state', { owner, session_id: sessionId })
export const createSession = (input: { owner: DID; peer_did: DID; title?: string; binding?: unknown; session_id?: string; origin?: string }) => call<OwnerSessionState>('msg.create_session', input as unknown as Record<string, unknown>)
export const archiveSession = (owner: DID, sessionId: string) => call<OwnerSessionState>('msg.archive_session', { owner, session_id: sessionId })
export const restoreSession = (owner: DID, sessionId: string) => call<OwnerSessionState>('msg.restore_session', { owner, session_id: sessionId })
export const deleteSession = (owner: DID, sessionId: string) => call<OwnerSessionState>('msg.delete_session', { owner, session_id: sessionId })

/* ── Records ── */

export const updateRecordState = (recordId: string, newState: RecipientState) => call<MailboxRecord>('msg.update_record_state', { record_id: recordId, new_state: newState })
export const getRecord = (recordId: string, withObject = true) => call<MailboxRecordWithObject | null>('msg.get_record', { record_id: recordId, with_object: withObject })
export const listBoxByTime = (input: { owner: DID; box_kind: MailboxKind; limit?: number; cursor_sort_key?: number; cursor_record_id?: string; descending?: boolean; with_object?: boolean }) => call<MailboxRecordPage | null>('msg.list_box_by_time', input as unknown as Record<string, unknown>).then(page => page ?? {})

/* ── Send ── */

export async function postSendMessage(msg: MsgObject, idempotencyKey?: string): Promise<PostSendResult> {
  const request: Record<string, unknown> = { msg }
  if (idempotencyKey) request.idempotency_key = idempotencyKey
  const result = await call<PostSendResult | null>('msg.post_send', request)
  if (!result || typeof result.ok !== 'boolean') throw new MessageHubApiError('post_send returned no result', 'unknown')
  return result
}

/* ── UI session state (legacy session-only KV and owner-scoped KV) ── */

export const listUiSessionState = (sessionId: string, owner?: DID) => call<UiSessionStateEntry[] | null>('ui_session.list_state', owner ? { session_id: sessionId, owner } : { session_id: sessionId }).then(list => list ?? [])
export const updateUiSessionState = (sessionId: string, key: string, value: unknown, owner: DID) => call<UiSessionStateEntry>('ui_session.update_state', { session_id: sessionId, key, value, owner })

/* ── Contacts and groups ── */

export const listContacts = (contactMgrOwner?: DID) => call<Contact[] | null>('contact.list_contacts', contactMgrOwner ? { query: {}, contact_mgr_owner: contactMgrOwner } : { query: {} }).then(list => list ?? [])
export const getContact = (did: DID, contactMgrOwner?: DID) => call<Contact | null>('contact.get_contact', contactMgrOwner ? { did, contact_mgr_owner: contactMgrOwner } : { did })
export const updateContact = (did: DID, patch: Record<string, unknown>, contactMgrOwner?: DID) => call<Contact>('contact.update_contact', contactMgrOwner ? { did, patch, contact_mgr_owner: contactMgrOwner } : { did, patch })
export const blockContact = (did: DID, contactMgrOwner?: DID, reason?: string) => call<unknown>('contact.block_contact', { did, ...(reason ? { reason } : {}), ...(contactMgrOwner ? { contact_mgr_owner: contactMgrOwner } : {}) })
export const listGroupsByMember = (memberDid: DID) => call<GroupSummary[] | null>('group.list_by_member', { member_did: memberDid, host_owner: memberDid }).then(list => list ?? [])
export const checkGroupAccess = (groupDid: DID, actorDid: DID, action: string) => call<GroupAccessDecision>('group.check_access', { group_did: groupDid, actor_did: actorDid, action, host_owner: actorDid })
