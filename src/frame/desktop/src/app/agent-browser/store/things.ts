/* A thing (D15): one MessageHub session with the Agent plus the group of tabs that belongs to it. */

import type { Session } from '../../messagehub/types'
import { DEFAULT_GROUP } from '../api/types'

export interface Thing {
  groupId: string
  /** The MessageHub session; the default thing has none until one is bound to it (the first time its conversation is used). */
  session: Session | null
  /** Empty for the default thing: the shell shows its own name for it. */
  title: string
  lastActiveAt: number
}

/**
 * Protocol §1: the session the default group is bound to (`default_session_id` of `browser.status`), a `dm:*` session
 * and the one named after the entity are the group `default`; any other session is the group of its own id.
 */
export function groupIdOfSession(session: Pick<Session, 'id'>, entityId: string, defaultSessionId: string | null): string {
  return session.id === defaultSessionId || session.id.startsWith('dm:') || session.id === entityId ? DEFAULT_GROUP : session.id
}

/** The conversation of the default thing among the active sessions: the bound one while it is there, else a `dm:*` or entity-named one, else none. */
export function defaultThingSession(sessions: Session[], entityId: string, defaultSessionId: string | null): Session | null {
  const bound = defaultSessionId ? sessions.find((session) => session.id === defaultSessionId) : undefined
  if (bound) return bound
  return sessions.filter((session) => session.id.startsWith('dm:') || session.id === entityId).sort((a, b) => b.lastActiveAt - a.lastActiveAt)[0] ?? null
}

/** The default thing first (it always exists), then the listed sessions in the order MessageHub gives them. */
export function buildThings(sessions: Session[], entityId: string, defaultSessionId: string | null, titleOf: (session: Session) => string, listed: (session: Session) => boolean): Thing[] {
  const bound = defaultThingSession(sessions, entityId, defaultSessionId)
  const things: Thing[] = [{ groupId: DEFAULT_GROUP, session: bound, title: '', lastActiveAt: bound?.lastActiveAt ?? 0 }]
  for (const session of sessions) {
    if (groupIdOfSession(session, entityId, defaultSessionId) === DEFAULT_GROUP || !listed(session)) continue
    things.push({ groupId: session.id, session, title: titleOf(session), lastActiveAt: session.lastActiveAt })
  }
  return things
}

/** The name a thing is shown under: the default thing has the App's own name for it. */
export function thingTitle(t: (key: string, fallback?: string) => string, thing: Pick<Thing, 'groupId' | 'title'>): string {
  if (thing.groupId === DEFAULT_GROUP) return t('agentBrowser.thing.default', 'General browsing')
  return thing.title || t('agentBrowser.thing.untitled', 'Untitled task')
}
