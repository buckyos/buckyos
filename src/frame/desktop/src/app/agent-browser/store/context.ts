import { createContext, useContext, useSyncExternalStore } from 'react'
import type { AgentBrowserSnapshot, AgentBrowserStore } from './store'

export const AgentBrowserStoreContext = createContext<AgentBrowserStore | null>(null)

/** The store itself: its methods, without following its state. */
export function useAgentBrowserStore(): AgentBrowserStore {
  const store = useContext(AgentBrowserStoreContext)
  if (!store) throw new Error('useAgentBrowserStore must be used inside the Agent browser')
  return store
}

/** The part of the state a component shows; it re-renders when that part changes. */
export function useAgentBrowser<T>(select: (snapshot: AgentBrowserSnapshot) => T): T {
  const store = useAgentBrowserStore()
  return useSyncExternalStore(store.subscribe, () => select(store.getSnapshot()))
}
