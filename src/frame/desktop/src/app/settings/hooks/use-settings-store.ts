import { createContext, useContext, useEffect, useSyncExternalStore } from 'react'
import { globalSettingsStore, type ResourceKey, type SettingsSnapshot, type SettingsStore } from '../data/store'

export const SettingsStoreContext = createContext<SettingsStore>(globalSettingsStore)

export function useSettingsStore(): SettingsStore {
  return useContext(SettingsStoreContext)
}

export function useSettingsSnapshot(): SettingsSnapshot {
  const store = useSettingsStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}

/** A store resource, loaded the first time a page asks for it. */
export function useSettingsResource<K extends ResourceKey>(key: K): SettingsSnapshot[K] {
  const store = useSettingsStore()
  const snapshot = useSettingsSnapshot()
  useEffect(() => {
    void store.load(key)
  }, [store, key])
  return snapshot[key]
}
