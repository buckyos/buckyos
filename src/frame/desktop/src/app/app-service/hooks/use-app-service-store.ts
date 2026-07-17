import { createContext, useContext, useSyncExternalStore } from 'react'
import type { AppServiceMockStore } from '../mock/store'

export const AppServiceStoreContext = createContext<AppServiceMockStore | null>(null)

export function useAppServiceStore() {
  const store = useContext(AppServiceStoreContext)
  if (!store) throw new Error('useAppServiceStore must be used within AppServiceStoreContext.Provider')
  useSyncExternalStore(store.subscribe, store.getRevision, store.getRevision)
  return store
}
