/* What a Block on a session's stage knows of the session (第四期规划 §5.3): who looks at it, the channel to call
 * `session.*` through, and the participant's own feeds. Outside a session (the courseware) there is none, and the
 * classroom Blocks show their design view. */

import { createContext, useCallback, useContext, useEffect, useState, useSyncExternalStore } from 'react'
import type { EntityEnvelope } from '../api/types'
import type { WorkspaceStore } from '../state/store'
import type { ChannelStatus, SessionChannel } from './channel'
import type { RoomModel } from './room'
import type { MediaState, SessionInfo, SessionRole } from './types'

export interface SessionUi {
  sessionId: string
  role: SessionRole
  principal: string
  level: 'verified' | 'guest'
  info: SessionInfo
  channel: SessionChannel
  /** `{ <courseware sample id>: <the participant's own table> }` (`my_responses`). */
  myFeeds: Record<string, string>
  /** Answering is open to this caller (a guest in a room without guest interaction only watches, Q03). */
  mayInteract: boolean
  /** Replay renders the recorded state only (§7.2): no calls, no writes. */
  replay: boolean
  /** The room's confirmed state (media playback included); null in a replay, which drives its own. */
  room: RoomModel | null
  /** Whether this terminal follows the presenter right now (media follow the presenter only then, Q24). */
  following: () => boolean
  /** The demonstration time now (replay: the playhead). */
  tRel: () => number
  /** The presenter's playback of each media Block (§7.3.2): live from the room, in a replay from the time line. */
  media: { get(entityId: string): MediaState | undefined; subscribe(listener: () => void): () => void }
}

export const SessionContext = createContext<SessionUi | null>(null)

export function useSession(): SessionUi | null {
  return useContext(SessionContext)
}

/** The feed entity a Block binding names, as this caller reads it in a session: a participant's own `my_responses`
 * instead of the courseware's (shared, empty) sample. */
export function feedTarget(session: SessionUi | null, store: WorkspaceStore, entityId: string | undefined): string | undefined {
  if (!entityId) return undefined
  if (!session) return entityId
  const mine = session.myFeeds[entityId]
  if (mine && store.outline.get(mine)) return mine
  const e = store.outline.get(entityId)
  if (e?.source?.kind === 'session' && e.source.feed === 'my_responses') {
    const own = store.outline.all().find((x: EntityEnvelope) => x.source?.kind === 'session' && x.source.feed === 'my_responses' && x.scope === 'participant')
    if (own) return own.entity_id
  }
  return entityId
}

/** Re-render every `ms` (clocks of a live room). */
export function useTick(ms: number): number {
  const [n, setN] = useState(0)
  useEffect(() => {
    const t = window.setInterval(() => setN((x) => x + 1), ms)
    return () => window.clearInterval(t)
  }, [ms])
  return n
}

/** The channel's state, re-read when it changes (a room shows it and names it in `data-channel`). */
export function useChannelStatus(channel: SessionChannel): ChannelStatus {
  const subscribe = useCallback((listener: () => void) => channel.onStatus(listener), [channel])
  return useSyncExternalStore(subscribe, () => channel.status)
}
