/**
 * Timeline reader backed by `msg.list_session`: newest page first, older
 * pages prepended on demand, and record-level upsert / remove so delivery
 * and read-state changes reach the projection without a full reload.
 */
import type { ConversationMessageReader } from '../conversation/history/types'
import type { MailboxKind, RecipientState, SessionDeliveryOverall, SessionDeliveryView, SessionMessageDirection, SessionMessageItem } from '../datamodel/sessionApi'
import type { MessageDeliveryStatus, MessageObject } from '../protocol/msgobj'

/** Local record context attached to every projected message (`ui_record`). */
export interface MessageRecordMeta {
  ownerDid: string
  recordId: string
  msgId: string
  sessionId: string
  direction: SessionMessageDirection
  boxKind: MailboxKind
  sortKey: number
  recipientState?: RecipientState
  delivery?: SessionDeliveryView
}

export interface SessionHistory {
  readonly messages: readonly MessageObject[]
  readonly hasOlder: boolean
  readonly oldestCursor?: { sortKey: number; recordId: string }
  readonly revision: number
  readonly loaded: boolean
  readonly error?: string
}

export const emptyHistory: SessionHistory = { messages: [], hasOlder: false, revision: 0, loaded: false }

export function mapDeliveryOverall(overall: SessionDeliveryOverall | undefined): MessageDeliveryStatus | undefined {
  switch (overall) {
    case 'sending': return 'sending'
    case 'delivered': return 'delivered'
    case 'partial_failed':
    case 'failed': return 'failed'
    default: return undefined
  }
}

export function recordMeta(message: MessageObject): MessageRecordMeta | undefined {
  const value = message.ui_record
  return value && typeof value === 'object' ? value as MessageRecordMeta : undefined
}

/**
 * Merge one timeline item into the UI message shape. The protocol object is
 * kept verbatim; the owner's record context rides along as `ui_record`.
 * Items without an object become an explicit "unavailable" placeholder
 * instead of being dropped.
 */
export function itemToMessage(item: SessionMessageItem, ownerDid: string, sessionId: string, senderName: string | undefined, unavailableLabel: string): MessageObject {
  const meta: MessageRecordMeta = {
    ownerDid,
    recordId: item.record_id,
    msgId: item.msg_id,
    sessionId,
    direction: item.direction,
    boxKind: item.box_kind,
    sortKey: item.sort_key,
    recipientState: item.recipient_state,
    delivery: item.delivery,
  }
  const base: MessageObject = item.msg
    ? { ...item.msg }
    : { from: item.from, to: [item.to], kind: 'notify', created_at_ms: item.sort_key, content: { format: 'text/plain', content: unavailableLabel }, ui_unavailable: true }
  base.ui_message_id = item.record_id
  base.ui_session_id = sessionId
  base.ui_record = meta
  if (senderName) base.ui_sender_name = senderName
  if (item.direction === 'out') {
    const status = mapDeliveryOverall(item.delivery?.overall)
    if (status) base.ui_delivery_status = status
  }
  return base
}

export function messageId(message: MessageObject): string {
  return typeof message.ui_message_id === 'string' ? message.ui_message_id : `${message.from}:${message.created_at_ms}`
}

function orderKey(message: MessageObject): [number, string] {
  const meta = recordMeta(message)
  return meta ? [meta.sortKey, meta.recordId] : [message.created_at_ms, `~${messageId(message)}`]
}

function compare(a: MessageObject, b: MessageObject): number {
  const [ka, ra] = orderKey(a)
  const [kb, rb] = orderKey(b)
  return ka - kb || (ra < rb ? -1 : ra > rb ? 1 : 0)
}

export function upsertMessages(history: SessionHistory, incoming: readonly MessageObject[], patch: Partial<Pick<SessionHistory, 'hasOlder' | 'oldestCursor' | 'loaded' | 'error'>> = {}): SessionHistory {
  const byId = new Map<string, MessageObject>()
  for (const message of history.messages) byId.set(messageId(message), message)
  let changed = false
  for (const message of incoming) {
    const id = messageId(message)
    const previous = byId.get(id)
    if (!previous || JSON.stringify(previous) !== JSON.stringify(message)) { byId.set(id, message); changed = true }
  }
  const patched = Object.keys(patch).some(key => (patch as Record<string, unknown>)[key] !== (history as unknown as Record<string, unknown>)[key])
  if (!changed && !patched) return history
  const messages = [...byId.values()].sort(compare)
  return { ...history, ...patch, messages, revision: history.revision + 1 }
}

export function removeMessage(history: SessionHistory, id: string): SessionHistory {
  if (!history.messages.some(message => message.ui_message_id === id)) return history
  return { ...history, messages: history.messages.filter(message => message.ui_message_id !== id), revision: history.revision + 1 }
}

export class SessionApiReader implements ConversationMessageReader {
  readonly readerKey: string
  readonly totalCount: number
  readonly revision: number
  private readonly messages: readonly MessageObject[]

  constructor(readerKey: string, history: SessionHistory) {
    this.readerKey = readerKey
    this.messages = history.messages
    this.totalCount = history.messages.length
    this.revision = history.revision
  }

  async readRange(startIndex: number, count: number) {
    if (count <= 0 || startIndex >= this.totalCount) return []
    const safeStart = Math.max(0, startIndex)
    return this.messages.slice(safeStart, safeStart + count)
  }
}
