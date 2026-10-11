import { isMessageActivity } from '../../sessionModel'
import { agentAskTarget, messageAgentAsk, messageObjId, type AgentAsk, type MessageObject, type MsgMentions, type MsgRelation } from '../../protocol/msgobj'
import { mentionsViewer, messageRelation } from '../history/relations'

/** The composer is locked to answering `question` (see `conversationInputState`). */
export interface AnswerInputState {
  mode: 'answer'
  question: MessageObject
  ask: AgentAsk
  /** What every message sent in this state relates to: `thread` → the anchor, or → the question without one. */
  relatesTo: MsgRelation
  /** In a group: the asker, whose group bridge only takes messages that mention it. */
  mentions?: MsgMentions
}

/**
 * The input state of a conversation: the composer's style and target follow
 * from the conversation, not only from what the user picked.
 */
export type ConversationInputState = { mode: 'normal' } | AnswerInputState

const NORMAL: ConversationInputState = { mode: 'normal' }

/** `/stop` is a control command of the Agent, never an answer. */
export function isStopCommand(text: string): boolean {
  return /^\/stop(\s|$)/.test(text.trim())
}

/**
 * Derived from the timeline only (oldest first, unfolded), never from task
 * state, so every device of the viewer ends up in the same state: when the
 * newest message of the conversation is a question (`agent_ask`) someone
 * else asked, the composer answers it; anything newer — the viewer's answer
 * above all — ends it. Not counted as messages: relation messages (edits,
 * redactions, reactions), events, status pills, sends that failed locally and
 * redacted messages. A question is also over once its anchor was edited
 * after it (the Turn's final reply arrived). In a group only the viewer the
 * question mentions answers it.
 */
export function conversationInputState(timeline: readonly MessageObject[], viewerDid: string, options: { group?: boolean } = {}): ConversationInputState {
  const redacted = new Set<string>()
  /** `target \n author` of every edit newer than the message looked at. */
  const edited = new Set<string>()
  for (let index = timeline.length - 1; index >= 0; index--) {
    const message = timeline[index]
    const relation = messageRelation(message)
    if (relation?.rel === 'redact') { redacted.add(relation.target); continue }
    if (relation?.rel === 'edit') { edited.add(`${relation.target}\n${message.from}`); continue }
    if (!isMessageActivity(message) || message.ui_delivery_status === 'failed') continue
    const id = messageObjId(message)
    if (id && redacted.has(id)) continue
    const ask = messageAgentAsk(message)
    if (!ask || message.from === viewerDid) return NORMAL
    if (ask.anchor && edited.has(`${ask.anchor}\n${message.from}`)) return NORMAL
    if (options.group && !mentionsViewer(message, viewerDid)) return NORMAL
    const target = agentAskTarget(message, ask)
    if (!target) return NORMAL
    return { mode: 'answer', question: message, ask, relatesTo: { rel: 'thread', target }, ...(options.group ? { mentions: { dids: [message.from] } } : {}) }
  }
  return NORMAL
}

/** Structured mentions of both, or `undefined` when neither mentions anyone. */
export function mergeMentions(a: MsgMentions | undefined, b: MsgMentions | undefined): MsgMentions | undefined {
  const dids = [...new Set([...(a?.dids ?? []), ...(b?.dids ?? [])])]
  const all = a?.all === true || b?.all === true
  if (!all && dids.length === 0) return undefined
  return { ...(dids.length ? { dids } : {}), ...(all ? { all: true } : {}) }
}
