import {
  createContext,
  createElement,
  useContext,
  useEffect,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from 'react'
import { AppServiceMockStore } from '../mock/store'
import { isMockRuntime } from '../../../runtime'

export const AppServiceStoreContext = createContext<AppServiceMockStore | null>(null)

let sharedStore: AppServiceMockStore | null = null

export function getSharedAppServiceStore() {
  sharedStore ??= new AppServiceMockStore()
  return sharedStore
}

export function AppServiceStoreProvider({ children }: { children: ReactNode }) {
  const [store] = useState(getSharedAppServiceStore)
  useEffect(() => {
    if (isMockRuntime()) return
    void store.refreshRealApps()
    const timer = window.setInterval(() => { void store.refreshRealApps() }, 15_000)
    return () => window.clearInterval(timer)
  }, [store])

  return createElement(AppServiceStoreContext.Provider, { value: store }, children)
}

export function useAppServiceStore() {
  const store = useContext(AppServiceStoreContext)
  if (!store) throw new Error('useAppServiceStore must be used within AppServiceStoreContext.Provider')
  useSyncExternalStore(store.subscribe, store.getRevision, store.getRevision)
  return store
}

export function useSharedAppServiceStore() {
  const [store] = useState(getSharedAppServiceStore)
  useSyncExternalStore(store.subscribe, store.getRevision, store.getRevision)
  return store
}
