import { buckyos } from 'buckyos'
import type { DID, MsgObject } from '../protocol/msgobj'

/**
 * Thin client for the msg-center Session projection API — the only read
 * surface the MessageHub UI uses for conversations (`msg.list_sessions` /
 * `msg.list_session`), plus `msg.post_send` for the write path.
 *
 * Wire shapes mirror the Rust structs in
 * src/kernel/buckyos-api/src/msg_center_client.rs.
 */

const MSG_CENTER_SERVICE = 'msg-center'

/* ── Wire types ── */

export type MailboxKind = 'INBOX' | 'SENT' | 'GROUP_INBOX' | 'REQUEST_BOX'

export type RecipientState = 'UNREAD' | 'READING' | 'READ' | 'ARCHIVED' | 'DELETED'

export type DeliveryState = 'WAIT' | 'SENDING' | 'SENT' | 'FAILED' | 'DEAD'

export type SessionMessageDirection = 'in' | 'out'

export type SessionDeliveryOverall =
  | 'sending'
  | 'delivered'
  | 'partial_failed'
  | 'failed'

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
  created_at_ms: number
  updated_at_ms: number
}

export interface MailboxRecordWithObject {
  record: MailboxRecord
  msg?: MsgObject | null
}

export interface SessionSummary {
  session_id: string
  last_record?: MailboxRecordWithObject
  unread_count: number
  updated_at_ms: number
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
  /** Only present on inbound entries. */
  recipient_state?: RecipientState
  /** Only present on outbound entries. */
  delivery?: SessionDeliveryView
  /** Full message object when `with_object` was requested. */
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

export interface PostSendRequest {
  msg: MsgObject
  idempotency_key?: string
}

/* ── RPC calls ── */

const SESSION_LIST_PAGE_SIZE = 200

function getMsgCenterRpcClient() {
  return buckyos.getServiceRpcClient(MSG_CENTER_SERVICE)
}

/** Resolve the current account's owner DID (`did:bns:{user_id}`). */
export async function fetchOwnerDid(): Promise<string | null> {
  const accountInfo = await buckyos.getAccountInfo()
  const userId = accountInfo?.user_id
  return userId ? `did:bns:${userId}` : null
}

export async function listSessions(
  request: ListSessionsRequest,
): Promise<SessionSummaryPage> {
  const rpcClient = getMsgCenterRpcClient()
  const page = await rpcClient.call<SessionSummaryPage, ListSessionsRequest>(
    'msg.list_sessions',
    request,
  )
  return page ?? {}
}

export async function listSessionMessages(
  request: ListSessionRequest,
): Promise<SessionMessagePage> {
  const rpcClient = getMsgCenterRpcClient()
  const page = await rpcClient.call<SessionMessagePage, ListSessionRequest>(
    'msg.list_session',
    request,
  )
  return page ?? {}
}

/** Fetch every session summary of `owner`, following the paging cursor. */
export async function listAllSessions(owner: DID): Promise<SessionSummary[]> {
  const sessions: SessionSummary[] = []
  let cursorUpdatedAtMs: number | undefined
  let cursorSessionId: string | undefined

  for (;;) {
    const page = await listSessions({
      owner,
      limit: SESSION_LIST_PAGE_SIZE,
      cursor_updated_at_ms: cursorUpdatedAtMs,
      cursor_session_id: cursorSessionId,
      with_object: true,
    })
    sessions.push(...(page.items ?? []))

    if (
      page.next_cursor_updated_at_ms === undefined
      || page.next_cursor_session_id === undefined
    ) {
      return sessions
    }

    cursorUpdatedAtMs = page.next_cursor_updated_at_ms
    cursorSessionId = page.next_cursor_session_id
  }
}

/** Send one message. `msg.to` must contain determinate DIDs. */
export async function postSendMessage(
  msg: MsgObject,
  idempotencyKey?: string,
): Promise<void> {
  const rpcClient = getMsgCenterRpcClient()
  const request: PostSendRequest = { msg }
  if (idempotencyKey) {
    request.idempotency_key = idempotencyKey
  }
  await rpcClient.call<unknown, PostSendRequest>('msg.post_send', request)
}
