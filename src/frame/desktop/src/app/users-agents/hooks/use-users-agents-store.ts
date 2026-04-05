/* ── Users & Agents – store hooks ── */

import { createContext, useContext, useSyncExternalStore } from 'react'
import type { UsersAgentsMockStore } from '../mock/store'
import type {
  UsersAgentsSnapshot,
  AnyEntity,
  Collection,
} from '../mock/types'

export const UsersAgentsStoreContext = createContext<UsersAgentsMockStore>(null!)

export function useUsersAgentsStore(): UsersAgentsMockStore {
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

export function useLocalUsers() {
  return useUsersAgentsSnapshot().localUsers
}

export function useContacts() {
  return useUsersAgentsSnapshot().contacts
}

export function useEntityGroups() {
  return useUsersAgentsSnapshot().entityGroups
}

export function useCollections() {
  return useUsersAgentsSnapshot().collections
}

export function useEntity(id: string): AnyEntity | undefined {
  const snap = useUsersAgentsSnapshot()
  if (snap.self.id === id) return snap.self
  if (snap.agent.id === id) return snap.agent
  return (
    snap.localUsers.find((u) => u.id === id) ??
    snap.contacts.find((c) => c.id === id) ??
    snap.entityGroups.find((g) => g.id === id)
  )
}

export function useCollection(id: string): Collection | undefined {
  return useCollections().find((c) => c.id === id)
}

/** Resolve entity IDs in a collection to actual entities */
export function useCollectionEntities(collectionId: string): AnyEntity[] {
  const snap = useUsersAgentsSnapshot()
  const col = snap.collections.find((c) => c.id === collectionId)
  if (!col) return []

  const all: AnyEntity[] = [
    snap.self,
    snap.agent,
    ...snap.localUsers,
    ...snap.contacts,
    ...snap.entityGroups,
  ]
  const lookup = new Map(all.map((e) => [e.id, e]))
  return col.entityIds.map((id) => lookup.get(id)).filter(Boolean) as AnyEntity[]
}
