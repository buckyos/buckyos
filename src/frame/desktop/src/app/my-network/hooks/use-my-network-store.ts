import { createContext, useContext, useSyncExternalStore } from 'react'
import type { MyNetworkStore } from '../datamodel/store'
import type {
  Collection,
  MyNetworkEntity,
  MyNetworkSnapshot,
} from '../datamodel/types'

export const MyNetworkStoreContext = createContext<MyNetworkStore>(null!)

export function useMyNetworkStore(): MyNetworkStore {
  return useContext(MyNetworkStoreContext)
}

export function useMyNetworkSnapshot(): MyNetworkSnapshot {
  const store = useMyNetworkStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}

export function useCollections() {
  return useMyNetworkSnapshot().collections
}

export function useEntity(id: string): MyNetworkEntity | undefined {
  const snap = useMyNetworkSnapshot()
  return (
    snap.contacts.find((contact) => contact.id === id) ??
    snap.entityGroups.find((group) => group.id === id)
  )
}

export function useCollection(id: string): Collection | undefined {
  return useCollections().find((collection) => collection.id === id)
}

export function useCollectionEntities(collectionId: string): MyNetworkEntity[] {
  const snap = useMyNetworkSnapshot()
  const collection = snap.collections.find((item) => item.id === collectionId)
  if (!collection) return []

  const all: MyNetworkEntity[] = [
    ...snap.contacts,
    ...snap.entityGroups,
  ]
  const lookup = new Map(all.map((entity) => [entity.id, entity]))
  return collection.entityIds.map((id) => lookup.get(id)).filter(Boolean) as MyNetworkEntity[]
}
