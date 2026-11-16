import { createContext, useContext, useEffect, useSyncExternalStore } from 'react'

/** The time relative labels ("5m") are drawn against; a data source may keep its own (the mock store does). */
export interface ConversationClock {
  subscribe(listener: () => void): () => void
  now(): number
  tick(): void
}

function localClock(): ConversationClock {
  let now = Date.now()
  const listeners = new Set<() => void>()
  return {
    subscribe: listener => { listeners.add(listener); return () => { listeners.delete(listener) } },
    now: () => now,
    tick: () => { now = Date.now(); listeners.forEach(listener => listener()) },
  }
}

export const ConversationClockContext = createContext<ConversationClock>(localClock())

/** The conversation's clock, advanced every minute while the caller is mounted. */
export function useConversationClock(): number {
  const clock = useContext(ConversationClockContext)
  const now = useSyncExternalStore(clock.subscribe, clock.now)
  useEffect(() => { const timer = setInterval(clock.tick, 60_000); return () => clearInterval(timer) }, [clock])
  return now
}
