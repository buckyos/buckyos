/* ── Users & Agents – store hooks ── */

import { createContext, useContext, useSyncExternalStore } from 'react'
import type { UsersAgentsStore } from '../datamodel/store'
import type {
  UsersAgentsSnapshot,
  AnyEntity,
} from '../datamodel/types'

export const UsersAgentsStoreContext = createContext<UsersAgentsStore>(null!)

export function useUsersAgentsStore(): UsersAgentsStore {
  return useContext(UsersAgentsStoreContext)
}

export function useUsersAgentsSnapshot(): UsersAgentsSnapshot {
  const store = useUsersAgentsStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}

export function useSelf() {
  return useUsersAgentsSnapshot().self
}

export function useAgent() {
  return useUsersAgentsSnapshot().agent
}

export function useAgents() {
  return useUsersAgentsSnapshot().agents
}

export function useLocalUsers() {
  return useUsersAgentsSnapshot().localUsers
}

export function useEntityGroups() {
  return useUsersAgentsSnapshot().entityGroups
}

export function useEntity(id: string): AnyEntity | undefined {
  const snap = useUsersAgentsSnapshot()
  if (snap.self.id === id) return snap.self
  const agent = snap.agents.find((item) => item.id === id)
  if (agent) return agent
  return (
    snap.localUsers.find((u) => u.id === id) ??
    snap.entityGroups.find((g) => g.id === id)
  )
}
