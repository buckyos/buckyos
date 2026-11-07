/**
 * What MessageHub adds to the standard MsgObject (`ndn.MsgObject` of the SDK,
 * `CYFS 标准对象` §16): the extension fields OpenDAN and MessageHub agree on,
 * the readers that clean untrusted notification data, and the `ui_*` hints of
 * a timeline item. The protocol types themselves come from `buckyos`.
 */

import { NOTIFY_INTENT, type HumanPromptControl, type HumanTaskPrompt, type NotifyCategory, type NotifyLevel, type NotifyMessageData, type NotifyStatus, type ndn } from 'buckyos'
import { didDisplayName } from '../../../presentation/model'

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
  anchor?: string
  call_id: string
  turn?: number
  input: AgentAskInput
}

/** `content.machine.intent` of an answer that picked an option; `data = { call_id, option }`. */
export const AGENT_ASK_ANSWER_INTENT = 'agent_ask.answer'

/**
 * `voice`: this message is a voice note, the audio file of its only
 * `data_obj` ref. `waveform` holds up to `VOICE_WAVEFORM_SAMPLES` levels of
 * 0–`VOICE_WAVEFORM_MAX` over the whole clip, for drawing it before the audio
 * is loaded. Other clients see an ordinary audio attachment.
 */
export interface VoiceNote {
  duration_ms: number
  waveform: number[]
}

export const VOICE_WAVEFORM_SAMPLES = 64
export const VOICE_WAVEFORM_MAX = 31

/**
 * A MsgObject as MessageHub reads it. Rust flattens `meta` into the top level,
 * so `ui_*` hints also live there; these three are read only through
 * `messageAgentTaskId`, `messageAgentAsk` and `messageVoice`.
 */
export type MessageObject = ndn.MsgObject & {
  agent_task?: AgentTaskRef
  agent_ask?: AgentAsk
  voice?: VoiceNote
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

/**
 * The `voice` of a voice note, or `undefined` when there is none or it is
 * malformed. An edit never carries one; out-of-range levels are clamped and
 * a missing waveform reads as a flat line.
 */
export function messageVoice(message: MessageObject): VoiceNote | undefined {
  if (message.relates_to?.rel === 'edit') return undefined
  const value: unknown = message.voice
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined
  const { duration_ms: duration, waveform } = value as { duration_ms?: unknown; waveform?: unknown }
  if (typeof duration !== 'number' || !Number.isFinite(duration) || duration <= 0) return undefined
  const levels = Array.isArray(waveform) ? waveform.slice(0, VOICE_WAVEFORM_SAMPLES).map(level => (typeof level === 'number' && Number.isFinite(level) ? Math.min(VOICE_WAVEFORM_MAX, Math.max(0, Math.round(level))) : 0)) : []
  return { duration_ms: Math.round(duration), waveform: levels }
}

/** The option an answer picked (`content.machine` with `agent_ask.answer`), when it names one. */
export function messageAgentAskAnswer(message: MessageObject): { call_id: string; option: string } | undefined {
  const machine = message.content?.machine
  if (machine?.intent !== AGENT_ASK_ANSWER_INTENT) return undefined
  const { call_id: callId, option } = machine.data ?? {}
  return typeof callId === 'string' && typeof option === 'string' ? { call_id: callId, option } : undefined
}

/** What an answer to `question` replies to: the anchor, or the question itself when it has none. */
export function agentAskTarget(question: MessageObject, ask: AgentAsk): string | undefined {
  return ask.anchor ?? messageObjId(question)
}

/**
 * The parts of an answer to `question`: its text (the typed text, or the
 * picked option's label), `relates_to` (`thread` to the anchor, or to the
 * question itself when it has none) and, for a picked option,
 * `content.machine`. `undefined` when the option is unknown or the question
 * has no id to reply to.
 */
export function buildAgentAskAnswer(question: MessageObject, ask: AgentAsk, answer: { text: string } | { option: string }): { text: string; relates_to: ndn.MsgRelation; machine?: ndn.MachineContent } | undefined {
  const target = agentAskTarget(question, ask)
  if (!target) return undefined
  const relates_to: ndn.MsgRelation = { rel: 'thread', target }
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

/** By importance to the user: the order of the category sessions. */
export const NOTIFY_CATEGORIES: readonly NotifyCategory[] = ['todo', 'agent', 'app']
export const NOTIFY_LEVELS: readonly NotifyLevel[] = ['quiet', 'normal', 'timely', 'urgent']
export const NOTIFY_STATUSES: readonly NotifyStatus[] = ['open', 'read', 'answered', 'dismissed', 'expired', 'canceled']

function isOneOf<T extends string>(values: readonly T[], value: unknown): value is T {
  return typeof value === 'string' && (values as readonly string[]).includes(value)
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value) ? value as Record<string, unknown> : undefined
}

function promptOf(value: unknown): HumanTaskPrompt {
  const raw = asObject(value) ?? {}
  const controls: HumanPromptControl[] = []
  for (const item of Array.isArray(raw.controls) ? raw.controls : []) {
    const control = asObject(item)
    if (!control || typeof control.id !== 'string') continue
    if (control.kind === 'choice') {
      const options = (Array.isArray(control.options) ? control.options : []).map(asObject).filter((option): option is Record<string, unknown> => !!option && typeof option.value === 'string' && typeof option.label === 'string')
        .map(option => ({ value: option.value as string, label: option.label as string, ...(typeof option.description === 'string' ? { description: option.description } : {}), ...(isOneOf(['default', 'primary', 'danger'] as const, option.style) ? { style: option.style } : {}) }))
      controls.push({ kind: 'choice', id: control.id, ...(typeof control.label === 'string' ? { label: control.label } : {}), options, multiple: control.multiple === true, required: control.required === true })
    } else if (control.kind === 'text_input') {
      controls.push({ kind: 'text_input', id: control.id, ...(typeof control.label === 'string' ? { label: control.label } : {}), ...(typeof control.placeholder === 'string' ? { placeholder: control.placeholder } : {}), ...(typeof control.default_value === 'string' ? { default_value: control.default_value } : {}), multiline: control.multiline === true, required: control.required === true })
    }
  }
  return { ...(typeof raw.title === 'string' ? { title: raw.title } : {}), ...(typeof raw.message === 'string' ? { message: raw.message } : {}), controls }
}

/** Reads a `HumanTaskPrompt` from untrusted JSON, dropping controls it does not understand. */
export function parseHumanTaskPrompt(value: unknown): HumanTaskPrompt {
  return promptOf(value)
}

/**
 * The notification data of a `kind = notify` message (`intent =
 * buckyos.notify`), or `undefined` when it is none or malformed. An edit
 * never carries one.
 */
export function messageNotify(message: MessageObject): NotifyMessageData | undefined {
  if (message.kind !== 'notify' || message.relates_to?.rel === 'edit') return undefined
  const machine = message.content?.machine
  if (machine?.intent !== NOTIFY_INTENT) return undefined
  const data = asObject(machine.data)
  if (!data || typeof data.notify_id !== 'string' || !data.notify_id.trim() || !isOneOf(['notice', 'input'] as const, data.kind) || !isOneOf(NOTIFY_CATEGORIES, data.category)) return undefined
  const source = asObject(data.source) ?? {}
  const text = (key: string) => typeof source[key] === 'string' && (source[key] as string).trim() ? { [key]: source[key] as string } : {}
  const conversation = asObject(source.conversation)
  return {
    version: typeof data.version === 'number' ? data.version : 1,
    notify_id: data.notify_id,
    kind: data.kind,
    category: data.category,
    level: isOneOf(NOTIFY_LEVELS, data.level) ? data.level : 'normal',
    home: data.home === true,
    source: {
      ...text('app_id'), ...text('agent_did'), ...text('session_id'), ...text('call_id'), ...text('goal_id'), ...text('operation'), ...text('link'),
      ...(typeof source.turn === 'number' ? { turn: source.turn } : {}),
      ...(conversation && typeof conversation.to === 'string' ? { conversation: { to: conversation.to, ...(typeof conversation.to_session === 'string' ? { to_session: conversation.to_session } : {}) } } : {}),
    },
    prompt: promptOf(data.prompt),
    ...(typeof data.task_id === 'string' && data.task_id ? { task_id: data.task_id } : {}),
    url: typeof data.url === 'string' ? data.url : `/messagehub?notify=${encodeURIComponent(data.notify_id)}`,
    ...(typeof data.expires_at_ms === 'number' ? { expires_at_ms: data.expires_at_ms } : {}),
  }
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

/** A status pill: a transient `notify` or a UI status item. A System Notify message is a card, never a pill. */
export function isStatusMessageObject(message: MessageObject): boolean {
  return (
    (message.kind === 'notify' && !messageNotify(message))
    || getMessageMetaString(message, 'ui_item_kind') === 'status'
  )
}
