/**
 * Thin TypeScript mirror of:
 * /Users/liuzhicong/project/cyfs-ndn/src/ndn-lib/src/msgobj.rs
 *
 * Keep the field names aligned with Rust serde output so the UI can consume
 * protocol objects directly instead of mapping them into a separate UI DTO.
 */

export type DID = string
export type ObjId = string
export type Uri = string

export type JsonValue =
  | null
  | boolean
  | number
  | string
  | JsonValue[]
  | { [key: string]: JsonValue }

export type MsgObjKind =
  | 'chat'
  | 'group_msg'
  | 'deliver'
  | 'notify'
  | 'event'
  | 'operation'

export type MsgContentFormat =
  | 'text/plain'
  | 'text/markdown'
  | 'text/html'
  | 'text/css'
  | 'text/xml'
  | 'image/png'
  | 'image/jpeg'
  | 'image/gif'
  | 'image/webp'
  | 'image/svg+xml'
  | 'image/bmp'
  | 'video/mp4'
  | 'video/webm'
  | 'video/ogg'
  | 'video/quicktime'
  | 'video/x-msvideo'
  | 'audio/mpeg'
  | 'audio/wav'
  | 'audio/ogg'
  | 'audio/webm'
  | 'audio/aac'
  | 'audio/flac'
  | 'application/json'
  | 'application/xml'
  | 'application/pdf'
  | 'application/zip'
  | 'application/octet-stream'
  | string

/**
 * Semantic hints only. `topic` may help to label a conversation but is never
 * a routing key: the target session is `MsgObject.to_session`.
 */
export interface TopicThread {
  topic?: string
  reply_to?: ObjId
  correlation_id?: string
}

/**
 * Relation kind of a relation message (`CYFS 标准对象` §16.3). Unknown values
 * are kept verbatim; a receiver shows such a message as a plain one.
 */
export type MsgRelType =
  | 'edit'
  | 'redact'
  | 'reaction'
  | 'thread'
  | string

/**
 * `relates_to`: this message edits / redacts / reacts to / joins the thread
 * of another `cymsg`. The original message is never modified.
 */
export interface MsgRelation {
  rel: MsgRelType
  /** The related message, always a `cymsg` ObjId. */
  target: ObjId
  /** Only for `reaction`: the reaction content (1-64 bytes), e.g. one emoji. */
  key?: string
}

/**
 * Structured mentions; notification semantics come only from this field,
 * never from `@` text in the body. Omit the whole field when it is empty:
 * `{}` is not a canonical value and is rejected.
 */
export interface MsgMentions {
  dids?: DID[]
  /** Mentions every participant of the target session. */
  all?: boolean
}

export type CanonValue =
  | null
  | boolean
  | number
  | string
  | number[]
  | CanonValue[]
  | { [key: string]: CanonValue }

export interface MachineContent {
  intent?: string
  data?: Record<string, CanonValue>
}

export type RefTarget =
  | {
      type: 'data_obj'
      obj_id: ObjId
      uri_hint?: Uri
    }
  | {
      type: 'service_did'
      did: DID
    }

export type RefRole =
  | 'context'
  | 'input'
  | 'output'
  | 'evidence'
  | 'control'

export interface RefItem {
  role: RefRole
  target: RefTarget
  label?: string
}

export interface MsgContent {
  title?: string
  format?: MsgContentFormat
  content?: string
  machine?: MachineContent
  refs?: RefItem[]
}

/**
 * `agent_task`: the TaskMgr task of the Agent Turn that produced this reply.
 * Only MessageHub reads it, and only on the anchor message (never on an
 * `edit`); everything else passes it through as an opaque extension field.
 */
export interface AgentTaskRef {
  task_id: string
}

/**
 * MsgObject v2 (`CYFS 标准对象` §16).
 *
 * Rust flattens `meta` into the top-level object, so unknown keys are allowed
 * here on purpose. UI-specific hints should also live there. `proof` is a
 * reserved key since v2 (a signed message travels in the JWT form instead)
 * and must never be set.
 */
export interface MsgObject {
  from: DID
  to: DID[]
  kind: MsgObjKind
  /**
   * Named session under the single target entity (`to[0]/to_session`).
   * Omitted means the default session. Only valid when `to` has exactly one
   * DID; the value follows `isValidMsgSessionId`.
   */
  to_session?: string
  thread?: TopicThread
  relates_to?: MsgRelation
  /** Omitted when empty, never `{}`. */
  mentions?: MsgMentions
  workspace?: DID
  /** Sender-declared creation time, for display only. */
  created_at_ms: number
  expires_at_ms?: number
  /** Random per message (`randomMsgNonce`) so identical messages keep distinct ObjIds. */
  nonce?: number
  content: MsgContent
  /** Flattened `meta` extension, see `AgentTaskRef`. */
  agent_task?: AgentTaskRef
  [key: string]: unknown
}

export type MessageObject = MsgObject

/** Max char length of `to_session`, same as the MailboxAddress session part. */
export const MSG_SESSION_ID_MAX_CHARS = 200

/**
 * `to_session` value rule (`msgobj.rs::validate_msg_session_id`): 1-200
 * chars, no surrounding whitespace, no control chars, not `.` / `..`.
 */
export function isValidMsgSessionId(value: string): boolean {
  if (!value || value === '.' || value === '..' || value.trim() !== value) return false
  let chars = 0
  for (const char of value) {
    const code = char.codePointAt(0) ?? 0
    if (code <= 0x1f || (code >= 0x7f && code <= 0x9f)) return false
    chars += 1
  }
  return chars <= MSG_SESSION_ID_MAX_CHARS
}

/**
 * Random `nonce` for one outgoing message (§16.6): deduplication is by ObjId,
 * so two identical messages must not collapse into one. Kept below 2^53 so it
 * stays a safe integer in JS and round-trips through JSON unchanged.
 */
export function randomMsgNonce(): number {
  const [high, low] = crypto.getRandomValues(new Uint32Array(2))
  return (high & 0x1f_ffff) * 0x1_0000_0000 + low
}
/** Task id of an anchor message; an `agent_task` carried by an edit is ignored. */
export function messageAgentTaskId(message: MessageObject): string | undefined {
  if (message.relates_to?.rel === 'edit') return undefined
  const value: unknown = message.agent_task
  const taskId = value && typeof value === 'object' ? (value as { task_id?: unknown }).task_id : undefined
  return typeof taskId === 'string' && taskId.trim() ? taskId : undefined
}

export type MessageDeliveryStatus =
  | 'sending'
  | 'sent'
  | 'delivered'
  | 'read'
  | 'failed'

export type ConversationStatusType =
  | 'typing'
  | 'processing'
  | 'disconnected'
  | 'info'

export function getMessageMetaString(
  message: MessageObject,
  key: string,
): string | undefined {
  const value = message[key]
  return typeof value === 'string' ? value : undefined
}

export function getMessageSenderName(message: MessageObject): string {
  return getMessageMetaString(message, 'ui_sender_name') ?? message.from
}

export function getMessageDeliveryStatus(
  message: MessageObject,
): MessageDeliveryStatus | undefined {
  const status = getMessageMetaString(message, 'ui_delivery_status')
  if (
    status === 'sending'
    || status === 'sent'
    || status === 'delivered'
    || status === 'read'
    || status === 'failed'
  ) {
    return status
  }

  return undefined
}

export function getMessageStatusType(
  message: MessageObject,
): ConversationStatusType | undefined {
  const status = getMessageMetaString(message, 'ui_status_type')
  if (
    status === 'typing'
    || status === 'processing'
    || status === 'disconnected'
    || status === 'info'
  ) {
    return status
  }

  return undefined
}

export function getMessageStableId(
  message: MessageObject,
  indexHint: number,
): string {
  return (
    getMessageMetaString(message, 'ui_message_id')
    ?? `${message.from}:${message.created_at_ms}:${indexHint}`
  )
}

export function isStatusMessageObject(message: MessageObject): boolean {
  return (
    message.kind === 'notify'
    || getMessageMetaString(message, 'ui_item_kind') === 'status'
  )
}
