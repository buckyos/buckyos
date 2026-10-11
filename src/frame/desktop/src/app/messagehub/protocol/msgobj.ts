/**
 * Thin TypeScript mirror of:
 * /Users/liuzhicong/project/cyfs-ndn/src/ndn-lib/src/msgobj.rs
 *
 * Keep the field names aligned with Rust serde output so the UI can consume
 * protocol objects directly instead of mapping them into a separate UI DTO.
 */

import { didDisplayName } from '../../../presentation/model'

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

export interface AgentAskOption {
  id: string
  label: string
}

/** How a question is answered: free text, or one of `options` (plus free text when `allow_text`). */
export interface AgentAskInput {
  kind: 'text' | 'choice'
  options: AgentAskOption[]
  allow_text: boolean
}

/**
 * `agent_ask`: this message is a question an Agent asks within a Turn and
 * waits for the answer to. `anchor` is the Turn's placeholder (the answer
 * joins its `thread`); it is empty on channels that cannot edit, where the
 * answer replies to the question itself. A choice question also lists its
 * options in the body for clients that do not read `agent_ask`.
 */
export interface AgentAsk {
  anchor?: ObjId
  call_id: string
  turn?: number
  input: AgentAskInput
}

/** `content.machine.intent` of an answer that picked an option; `data = { call_id, option }`. */
export const AGENT_ASK_ANSWER_INTENT = 'agent_ask.answer'

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
  /** Flattened `meta` extension, read only through `messageAgentAsk`. */
  agent_ask?: AgentAsk
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

/** ObjId of a timeline message: the stored record's `msg_id`, else the accepted or local id. */
export function messageObjId(message: MessageObject): string | undefined {
  const record = message.ui_record
  const msgId = record && typeof record === 'object' ? (record as { msgId?: unknown }).msgId : undefined
  if (typeof msgId === 'string') return msgId
  if (typeof message.ui_sent_msg_id === 'string') return message.ui_sent_msg_id
  return typeof message.ui_message_id === 'string' ? message.ui_message_id : undefined
}

/** Task id of an anchor message; an `agent_task` carried by an edit is ignored. */
export function messageAgentTaskId(message: MessageObject): string | undefined {
  if (message.relates_to?.rel === 'edit') return undefined
  const value: unknown = message.agent_task
  const taskId = value && typeof value === 'object' ? (value as { task_id?: unknown }).task_id : undefined
  return typeof taskId === 'string' && taskId.trim() ? taskId : undefined
}

function askOptions(value: unknown): AgentAskOption[] {
  if (!Array.isArray(value)) return []
  const options: AgentAskOption[] = []
  for (const item of value) {
    const { id, label } = item && typeof item === 'object' ? item as { id?: unknown; label?: unknown } : {}
    if (typeof id !== 'string' || !id.trim() || typeof label !== 'string' || !label.trim() || options.some(option => option.id === id)) continue
    options.push({ id, label })
  }
  return options
}

/**
 * The `agent_ask` of a question, or `undefined` when there is none or it is
 * malformed (no `call_id`). An `agent_ask` carried by an edit is ignored. A
 * choice without a usable option, or an unknown kind, is read as a text question.
 */
export function messageAgentAsk(message: MessageObject): AgentAsk | undefined {
  if (message.relates_to?.rel === 'edit') return undefined
  const value: unknown = message.agent_ask
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const { anchor, call_id: callId, turn, input } = value as { anchor?: unknown; call_id?: unknown; turn?: unknown; input?: unknown }
  if (typeof callId !== 'string' || !callId.trim()) return undefined
  const raw = input && typeof input === 'object' ? input as { kind?: unknown; options?: unknown; allow_text?: unknown } : {}
  const options = raw.kind === 'choice' ? askOptions(raw.options) : []
  return {
    ...(typeof anchor === 'string' && anchor.trim() ? { anchor } : {}),
    call_id: callId,
    ...(typeof turn === 'number' && Number.isFinite(turn) ? { turn } : {}),
    input: options.length > 0 ? { kind: 'choice', options, allow_text: raw.allow_text === true } : { kind: 'text', options: [], allow_text: true },
  }
}

/** The option an answer picked (`content.machine` with `agent_ask.answer`), when it names one. */
export function messageAgentAskAnswer(message: MessageObject): { call_id: string; option: string } | undefined {
  const machine = message.content?.machine
  if (machine?.intent !== AGENT_ASK_ANSWER_INTENT) return undefined
  const { call_id: callId, option } = machine.data ?? {}
  return typeof callId === 'string' && typeof option === 'string' ? { call_id: callId, option } : undefined
}

/** What an answer to `question` replies to: the anchor, or the question itself when it has none. */
export function agentAskTarget(question: MessageObject, ask: AgentAsk): ObjId | undefined {
  return ask.anchor ?? messageObjId(question)
}

/**
 * The parts of an answer to `question`: its text (the typed text, or the
 * picked option's label), `relates_to` (`thread` to the anchor, or to the
 * question itself when it has none) and, for a picked option,
 * `content.machine`. `undefined` when the option is unknown or the question
 * has no id to reply to.
 */
export function buildAgentAskAnswer(question: MessageObject, ask: AgentAsk, answer: { text: string } | { option: string }): { text: string; relates_to: MsgRelation; machine?: MachineContent } | undefined {
  const target = agentAskTarget(question, ask)
  if (!target) return undefined
  const relates_to: MsgRelation = { rel: 'thread', target }
  if ('text' in answer) return { text: answer.text, relates_to }
  const option = ask.input.options.find(item => item.id === answer.option)
  if (!option) return undefined
  return { text: option.label, relates_to, machine: { intent: AGENT_ASK_ANSWER_INTENT, data: { call_id: ask.call_id, option: option.id } } }
}

/**
 * The question of a choice `agent_ask` without the option list its body
 * repeats for other clients: trailing lines that name the options, in order,
 * are dropped. Any other body is returned as it is.
 */
export function agentAskQuestionText(text: string, ask: AgentAsk): string {
  const options = ask.input.options
  if (options.length === 0) return text.trim()
  const lines = text.trimEnd().split('\n')
  let end = lines.length
  for (let index = options.length - 1; index >= 0; index--) {
    while (end > 0 && !lines[end - 1].trim()) end--
    if (end === 0 || !lines[end - 1].includes(options[index].label)) return text.trim()
    end--
  }
  const question = lines.slice(0, end).join('\n').trim()
  return question || text.trim()
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
  return getMessageMetaString(message, 'ui_sender_name') ?? didDisplayName(message.from)
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
