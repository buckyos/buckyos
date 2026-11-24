/* ── Conversations with unread messages ──
 *
 * The one place that says which conversations are listed and what a row shows. The Shell's tray feeds it from the
 * MessageHub store, `conversations.ts` from msg-center's answers.
 */

import type { Entity, Session } from '../messagehub/types.ts'
import type { ActivityMessageItem } from './model.ts'

export interface ConversationSource {
  sessions: readonly Pick<Session, 'id' | 'entityId' | 'unreadCount' | 'lastMessage' | 'lastActiveAt'>[]
  entity(entityId: string): Pick<Entity, 'name' | 'avatar' | 'type' | 'systemNotify'> | undefined
  muted(sessionId: string): boolean
  /** The session's own title. */
  title(sessionId: string): string
  /** How many sessions the entity has. */
  sessionCount(entityId: string): number
}

export interface UnreadConversations {
  /** Unread messages of every conversation that counts. */
  unread: number
  /** Newest first. */
  items: ActivityMessageItem[]
}

export function unreadConversations(source: ConversationSource): UnreadConversations {
  const items: ActivityMessageItem[] = []
  const counts = new Map<string, number>()
  let unread = 0
  for (const session of source.sessions) {
    if (session.unreadCount <= 0) continue
    const entity = source.entity(session.entityId)
    if (entity?.systemNotify || source.muted(session.id)) continue
    unread += session.unreadCount
    const title = source.title(session.id)
    const name = entity?.name ?? title
    let count = counts.get(session.entityId)
    if (count === undefined) {
      count = source.sessionCount(session.entityId)
      counts.set(session.entityId, count)
    }
    items.push({
      sessionId: session.id,
      entityId: session.entityId,
      title: count > 1 ? `${name} · ${title}` : name,
      avatar: entity?.avatar,
      isGroup: entity?.type === 'group',
      senderName: session.lastMessage?.senderName,
      text: session.lastMessage?.text ?? '',
      unreadCount: session.unreadCount,
      timestamp: session.lastMessage?.timestamp ?? session.lastActiveAt,
    })
  }
  items.sort((a, b) => b.timestamp - a.timestamp)
  return { unread, items }
}
