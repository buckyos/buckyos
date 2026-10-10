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

/** The logged-in zone username (`user_id` of the account info), or '' when unknown. */
export async function fetchAccountUsername(): Promise<string> {
  const accountInfo = await buckyos.getAccountInfo()
  const userId = accountInfo?.user_id
  return typeof userId === 'string' ? userId.trim() : ''
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
export const listBoxByTime = (input: { mailbox: string; box_kind: MailboxKind; limit?: number; cursor_sort_key?: number; cursor_record_id?: string; descending?: boolean; with_object?: boolean }) => call<MailboxRecordPage | null>('msg.list_box_by_time', input as unknown as Record<string, unknown>).then(page => page ?? {})

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

/* ── Self-host Group v2 (msg_center/doc/group-v2-backend.md) ── */

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

/** Shared state of a Group Session (`group.get_shared_state`); text fields are absent when unset. */
export interface GroupSharedState {
  revision: string
  updated_at_ms?: number
  title?: string
  description?: string
  announcement?: string
}

export interface GroupSessionRules {
  receipts?: string
  edit?: { edit_window_ms?: number | null; recall_window_ms?: number | null }
  allow_guests?: boolean
}

export interface GroupSessionItem {
  session_id: string | null
  session: string
  shared_state?: Partial<GroupSharedState> | null
  rules?: GroupSessionRules | null
  lifecycle: string
  revision: string
  has_guests: boolean
  participant_count?: number
}

/** `group.get_config`: the configuration document (`GroupConfiguration`). */
export interface GroupConfig {
  revision: string
  profile?: { name?: unknown; description?: unknown } | null
  default_session?: GroupSessionRules | null
}

export interface GroupInviteLink {
  token: string
  expires_at_ms?: number | null
  max_uses?: number | null
  require_approval: boolean
}

export interface GroupReadMarkers {
  last_read_seq: number
  visibility?: 'hidden'
  count?: number
  readers?: DID[]
}

const key = (idempotencyKey: string) => ({ idempotency_key: idempotencyKey })
const session = (sessionId: string | undefined) => sessionId !== undefined ? { session_id: sessionId } : {}

export const listGroupsByMember = () => call<{ items?: GroupDocEnvelope[]; joined?: JoinedGroupSummary[] } | null>('group.list_by_member', {}).then(result => result ?? {})
export const getGroupDoc = (groupDid: DID) => call<GroupDocEnvelope>('group.get_doc', { group_did: groupDid })
export const getGroupConfig = (groupDid: DID) => call<GroupConfig>('group.get_config', { group_did: groupDid })
/** Members of a hosted group plus the owner transfer still waiting for its target (members only; the public doc never carries it). */
export const listGroupMembers = (groupDid: DID) => call<{ items?: GroupMemberRecord[]; pending_owner_transfer?: GroupOwnerTransfer | null } | null>('group.list_members', { group_did: groupDid }).then(result => ({ items: result?.items ?? [], pendingTransfer: result?.pending_owner_transfer ?? null }))
export interface GroupOwnerTransfer { member_did: DID; transfer_id: string; expires_at_ms: number }
export const listGroupSessions = (groupDid: DID) => call<{ items?: GroupSessionItem[] } | null>('group.list_sessions', { group_did: groupDid }).then(result => result?.items ?? [])
/** One participant of a Group Session (`group.list_session_members`); `invited` guests are listed only for callers who may invite guests. */
export interface GroupSessionMemberRecord {
  member_did: DID
  kind: 'group_member' | 'guest'
  role?: GroupMemberRecord['role'] | null
  entity_kind?: string | null
  state: 'included' | 'invited'
}
/** `complete` is false for a guest, or a member the member list is hidden from: only explicit participants and those who posted are listed. */
export const listGroupSessionMembers = (groupDid: DID, sessionId?: string) => call<{ items?: GroupSessionMemberRecord[]; complete?: boolean; revision?: string } | null>('group.list_session_members', { group_did: groupDid, ...session(sessionId) }).then(result => ({ items: result?.items ?? [], complete: result?.complete === true, revision: result?.revision ?? '' }))
export const checkGroupAccess = (groupDid: DID, action: string, sessionId?: string) => call<{ allowed: boolean; reason?: string | null }>('group.check_access', { group_did: groupDid, action, ...session(sessionId) })
export const createGroup = (input: { idempotency_key: string; profile: { name: string }; invitations: Array<{ member_did: DID }> }) => call<{ group_did: DID; revision: string }>('group.create', input as unknown as Record<string, unknown>)
/** `patch` is deep-merged into the configuration (`{ profile: { name, description } }` edits the profile). */
export const applyGroupConfig = (groupDid: DID, expectedRevision: string, patch: Record<string, unknown>, idempotencyKey: string) => call<{ revision: string }>('group.apply_config', { group_did: groupDid, expected_revision: expectedRevision, patch, ...key(idempotencyKey) })
export const inviteGroupMember = (groupDid: DID, memberDid: DID, idempotencyKey: string) => call<{ invite_id: string; member_did: DID; expires_at_ms: number; state: 'invited' | 'active' | 'pending_admin_approval' }>('group.invite_member', { group_did: groupDid, member_did: memberDid, ...key(idempotencyKey) })
export const removeGroupMember = (groupDid: DID, memberDid: DID, idempotencyKey: string) => call<unknown>('group.remove_member', { group_did: groupDid, member_did: memberDid, ...key(idempotencyKey) })
export const approveGroupMember = (groupDid: DID, memberDid: DID, idempotencyKey: string) => call<GroupMemberRecord>('group.approve_member', { group_did: groupDid, member_did: memberDid, ...key(idempotencyKey) })
export const rejectGroupMember = (groupDid: DID, memberDid: DID, idempotencyKey: string) => call<GroupMemberRecord>('group.reject_member', { group_did: groupDid, member_did: memberDid, ...key(idempotencyKey) })
export const updateGroupMemberRole = (groupDid: DID, memberDid: DID, role: 'admin' | 'member', idempotencyKey: string) => call<{ role: string }>('group.update_member_role', { group_did: groupDid, member_did: memberDid, role, ...key(idempotencyKey) })
export const moderateGroupMember = (groupDid: DID, memberDid: DID, patch: { blocked?: boolean; muted_until_ms?: number | null }, idempotencyKey: string) => call<unknown>('group.moderate', { group_did: groupDid, member_did: memberDid, ...patch, ...key(idempotencyKey) })
/** Starts a two-step transfer; the target accepts with `acceptGroupOwnerTransfer`. */
export const transferGroupOwner = (groupDid: DID, memberDid: DID, idempotencyKey: string) => call<{ transfer_id: string; member_did: DID; expires_at_ms: number }>('group.transfer_owner', { group_did: groupDid, member_did: memberDid, ...key(idempotencyKey) })
export const acceptGroupOwnerTransfer = (groupDid: DID, transferId: string) => call<{ owner: DID }>('group.accept_owner_transfer', { group_did: groupDid, transfer_id: transferId })
export const cancelGroupOwnerTransfer = (groupDid: DID, idempotencyKey: string) => call<unknown>('group.cancel_owner_transfer', { group_did: groupDid, ...key(idempotencyKey) })
export const leaveGroup = (groupDid: DID, idempotencyKey: string) => call<unknown>('group.leave', { group_did: groupDid, ...key(idempotencyKey) })
export const deleteGroup = (groupDid: DID, idempotencyKey: string) => call<{ lifecycle: string }>('group.delete', { group_did: groupDid, ...key(idempotencyKey) })
export const createGroupInviteLink = (groupDid: DID, options: { expires_at_ms?: number; max_uses?: number; require_approval?: boolean }, idempotencyKey: string) => call<GroupInviteLink>('group.create_invite_link', { group_did: groupDid, ...options, ...key(idempotencyKey) })
export const revokeGroupInviteLink = (groupDid: DID, token: string, idempotencyKey: string) => call<unknown>('group.revoke_invite_link', { group_did: groupDid, token, ...key(idempotencyKey) })
/** Accepts a direct invitation; `memberDid` only when the viewer accepts on behalf of its agent. */
export const acceptGroupInvitation = (groupDid: DID, invitationId: string, memberDid?: DID) => call<GroupMemberRecord>('group.accept_invitation', { group_did: groupDid, invitation_id: invitationId, ...(memberDid ? { member_did: memberDid } : {}) })
/** Asks to join (open / request_and_approve policy) or joins through an invite link token. */
export const requestGroupJoin = (groupDid: DID, invite?: string) => call<GroupMemberRecord>('group.request_join', { group_did: groupDid, ...(invite ? { invite } : {}) })
export const createGroupSession = (groupDid: DID, input: { title?: string; idempotency_key: string }) => call<{ group_did: DID; session_id: string; session: string; revision: string }>('group.create_session', { group_did: groupDid, ...input })
export const updateGroupSession = (groupDid: DID, sessionId: string, expectedRevision: string, input: { add_members?: DID[]; membership?: unknown }, idempotencyKey: string) => call<{ revision: string }>('group.update_session', { group_did: groupDid, session_id: sessionId, expected_revision: expectedRevision, ...input, ...key(idempotencyKey) })
export const archiveGroupSession = (groupDid: DID, sessionId: string, expectedRevision: string, idempotencyKey: string) => call<{ revision: string; lifecycle: string }>('group.archive_session', { group_did: groupDid, session_id: sessionId, expected_revision: expectedRevision, ...key(idempotencyKey) })
export const deleteGroupSession = (groupDid: DID, sessionId: string, expectedRevision: string, idempotencyKey: string) => call<{ revision: string; lifecycle: string }>('group.delete_session', { group_did: groupDid, session_id: sessionId, expected_revision: expectedRevision, ...key(idempotencyKey) })
export const removeGroupSessionMember = (groupDid: DID, sessionId: string, memberDid: DID, expectedRevision: string, idempotencyKey: string) => call<{ revision: string }>('group.remove_session_member', { group_did: groupDid, session_id: sessionId, member_did: memberDid, expected_revision: expectedRevision, ...key(idempotencyKey) })
export const leaveGroupSession = (groupDid: DID, sessionId: string, idempotencyKey: string) => call<{ revision: string }>('group.leave_session', { group_did: groupDid, session_id: sessionId, ...key(idempotencyKey) })
export const inviteGroupSessionGuest = (groupDid: DID, sessionId: string, memberDid: DID, idempotencyKey: string) => call<unknown>('group.invite_session_guest', { group_did: groupDid, session_id: sessionId, member_did: memberDid, ...key(idempotencyKey) })
export const acceptGroupSessionInvitation = (groupDid: DID, sessionId: string) => call<unknown>('group.accept_session_invitation', { group_did: groupDid, session_id: sessionId })
/** Message sequence numbers of a session (`seq` ↔ `obj_id`), which read markers are expressed in. */
export const listGroupMessages = (groupDid: DID, sessionId: string | undefined, afterSeq: number, limit: number) => call<{ items?: Array<{ seq: number; obj_id: string; redacted?: boolean }>; next_after_seq?: number; limited?: boolean } | null>('group.list_messages', { group_did: groupDid, ...session(sessionId), after_seq: afterSeq, limit }).then(result => result ?? {})
export const getGroupReadMarkers = (groupDid: DID, sessionId: string | undefined, sessionSeq: number) => call<GroupReadMarkers>('group.get_read_markers', { group_did: groupDid, ...session(sessionId), session_seq: sessionSeq })
export const updateGroupReadMarker = (groupDid: DID, sessionId: string | undefined, lastReadSeq: number) => call<{ last_read_seq: number }>('group.update_read_marker', { group_did: groupDid, ...session(sessionId), last_read_seq: lastReadSeq })
export const getGroupSharedState = (groupDid: DID, sessionId?: string) => call<GroupSharedState>('group.get_shared_state', { group_did: groupDid, ...session(sessionId) })
export const updateGroupSharedState = (groupDid: DID, sessionId: string | undefined, expectedRevision: string, set: Record<string, string>, unset: string[], idempotencyKey: string) => call<GroupSharedState>('group.update_shared_state', { group_did: groupDid, ...session(sessionId), expected_revision: expectedRevision, set, unset, ...key(idempotencyKey) })
export const getGroupMemberState = (groupDid: DID, sessionId?: string) => call<GroupSharedState & { nickname?: string }>('group.get_member_state', { group_did: groupDid, ...session(sessionId) })
export const updateGroupMemberState = (groupDid: DID, sessionId: string | undefined, expectedRevision: string, set: Record<string, string>, unset: string[], idempotencyKey: string) => call<GroupSharedState & { nickname?: string }>('group.update_member_state', { group_did: groupDid, ...session(sessionId), expected_revision: expectedRevision, set, unset, ...key(idempotencyKey) })

/** The host's reason code (`capability-denied`, `invitation-mismatch`, …) carried in an RPC error. */
export function groupErrorReason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error)
  // msg-center refuses to add an Agent whose `allow_group` is off (NoPermission … agent_group_disabled).
  if (message.includes('agent_group_disabled')) return 'agent-group-disabled'
  return message.match(/[a-z0-9]+(?:-[a-z0-9]+)+/)?.[0] ?? 'unknown'
}
