import type { MessageHubStore } from '../../app/messagehub/store/types'
import type { MessageHubContext } from '../../app/messagehub/types'
import { unreadConversations, type UnreadConversations } from '../../shared/activity/messages.ts'

/** The unread conversations of the MessageHub store. */
export function messageRows(store: MessageHubStore, context: MessageHubContext): UnreadConversations {
  const sessions = store.sessions(context)
  const byId = new Map(sessions.map((session) => [session.id, session]))
  return unreadConversations({
    sessions,
    entity: (entityId) => store.findEntity(context, entityId),
    muted: (sessionId) => store.preferences(context, sessionId).muted,
    title: (sessionId) => store.title(context, byId.get(sessionId)!),
    sessionCount: (entityId) => store.sessions(context, entityId).length,
  })
}
