import {
  createContext,
  createElement,
  useContext,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react'
import { AppServiceMockStore } from '../mock/store'

export const AppServiceStoreContext = createContext<AppServiceMockStore | null>(null)

let sharedStore: AppServiceMockStore | null = null

function getSharedStore() {
  sharedStore ??= new AppServiceMockStore()
  return sharedStore
}

export function AppServiceStoreProvider({ children }: { children: ReactNode }) {
  const [store] = useState(getSharedStore)

  return createElement(AppServiceStoreContext.Provider, { value: store }, children)
}

export function useAppServiceStore() {
  const store = useContext(AppServiceStoreContext)
  if (!store) throw new Error('useAppServiceStore must be used within AppServiceStoreContext.Provider')
  useSyncExternalStore(store.subscribe, store.getRevision, store.getRevision)
  return store
}

export function useSharedAppServiceStore() {
  const [store] = useState(getSharedStore)
  useSyncExternalStore(store.subscribe, store.getRevision, store.getRevision)
  return store
}
