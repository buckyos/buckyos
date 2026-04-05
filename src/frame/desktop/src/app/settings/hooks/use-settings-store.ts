import { createContext, useContext, useSyncExternalStore } from 'react'
import { SettingsMockStore } from '../mock/store'
import type { SettingsStoreSnapshot } from '../mock/types'

export const SettingsStoreContext = createContext<SettingsMockStore>(null!)

export function useSettingsStore(): SettingsMockStore {
  return useContext(SettingsStoreContext)
}

export function useSettingsSnapshot(): SettingsStoreSnapshot {
  const store = useSettingsStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}
