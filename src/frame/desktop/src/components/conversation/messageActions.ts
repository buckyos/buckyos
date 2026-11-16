import type { DID } from 'buckyos'
import type { ConversationMessageActions } from './history/actions'
import { messageObjId, messageSummaryText, ownReactionId } from './history/relations'
import type { ComposerRelation } from './input/ConversationComposer'
import { turnInputMessage } from './input/foregroundState'
import { withinWindow } from './model/groupNotice'
import type { MessageObject } from './model/msgobj'
import type { ConversationSource } from './source'
import type { WorklogDestination } from './worklog/model'

/** What the host does with a message; an action whose callback the host leaves out is not offered. */
export interface ConversationHostActions {
  /** The details of a message; `worklog`: on that session's work log (a work session card). */
  onOpenMessage?: (message: MessageObject, worklog?: WorklogDestination) => void
  /** The target picker that posts a copy of the message to another conversation. */
  onForward?: (message: MessageObject) => void
  onOpenEntity?: (did: DID) => void
  /** The profile of a participant whose picture was clicked. */
  onOpenProfile?: (did: DID, name: string) => void
}

/**
 * The per-message actions of a conversation: what the source can do and the host handles, within what the viewer
 * may do right now (`canSend`; `answering`: while a question is open nothing but its answer is sent).
 */
export function buildMessageActions(source: ConversationSource, host: ConversationHostActions, state: { canSend: boolean; answering: boolean; you: string; setRelation: (relation: ComposerRelation) => void }): ConversationMessageActions {
  const { sessionId, selfDid, canAct, group } = source
  const { canSend, answering } = state
  const isGroup = !!group
  const pinnedId = source.preferences.pinned?.id
  const setPinned = source.preferences.setPinned
  const sendRelation = (target: string | undefined, rel: 'redact' | 'reaction', key?: string) => {
    if (!target || !canSend) return Promise.reject(Error('permission_denied'))
    return source.send({ attachments: [], content: key ?? '', relatesTo: { rel, target, ...(key ? { key } : {}) } })
  }
  const timeline = source.reader.timeline ?? []
  return {
    ...source.messageActions,
    resend: canSend ? source.resend : undefined,
    displayName: (did: DID) => did === selfDid ? state.you : source.displayName(did),
    openEntity: host.onOpenEntity,
    senderAvatar: group?.senderAvatar,
    openProfile: host.onOpenProfile,
    // Relations are plain messages with `relates_to`, so a direct conversation offers reactions, replies and delete
    // requests too; edits follow the group's rules.
    relations: canAct && sessionId ? {
      capabilities: (message: MessageObject) => {
        const own = message.from === selfDid, now = source.now?.() ?? Date.now()
        const edit = isGroup && canSend && own && withinWindow(group?.editWindowMs, message.created_at_ms, now)
        // Outside a group a redact is a request the peer's client honours by default; nothing enforces it, so
        // either side may ask for any message.
        const redact = canSend && (!isGroup || (own ? withinWindow(group?.recallWindowMs, message.created_at_ms, now) : group?.redactAny ?? false))
        return { reply: canSend && !answering, react: canSend, edit: edit && !answering, redact, recall: redact && isGroup && own }
      },
      reply: (message: MessageObject) => state.setRelation({ kind: 'reply', message }),
      edit: (message: MessageObject) => state.setRelation({ kind: 'edit', message }),
      redact: (message: MessageObject) => sendRelation(messageObjId(message), 'redact'),
      react: (message: MessageObject, key: string) => sendRelation(messageObjId(message), 'reaction', key),
      unreact: (message: MessageObject, key: string) => sendRelation(ownReactionId(message, selfDid, key), 'redact'),
    } : undefined,
    openDetails: sessionId ? host.onOpenMessage : undefined,
    forward: canAct && sessionId ? host.onForward : undefined,
    remove: canAct && sessionId ? source.remove : undefined,
    pin: canAct && sessionId && setPinned ? {
      isPinned: (message: MessageObject) => !!pinnedId && messageObjId(message) === pinnedId,
      toggle: (message: MessageObject) => {
        const id = messageObjId(message)
        if (!id) return Promise.reject(Error('not-found'))
        return setPinned(id === pinnedId ? null : { id, text: messageSummaryText(message).slice(0, 2000), senderDid: message.from, createdAt: message.created_at_ms })
      },
    } : undefined,
    readReceipt: sessionId ? source.readReceipt : undefined,
    agentTurn: source.agent && sessionId ? {
      inputOf: (anchor: MessageObject, task: { input?: unknown } | undefined) => {
        const keys = (task?.input as { input_keys?: unknown } | undefined)?.input_keys
        return canSend ? turnInputMessage(timeline, anchor, selfDid, Array.isArray(keys) ? keys.filter((key): key is string => typeof key === 'string') : []) : null
      },
      retract: (input: MessageObject) => sendRelation(messageObjId(input), 'redact'),
    } : undefined,
  }
}
