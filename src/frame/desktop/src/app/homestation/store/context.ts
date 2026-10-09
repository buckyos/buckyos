import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from 'react'
import type { HomeStationStore, StoreDomain } from '../mock/store'
import { useHsNav } from '../navContext'

export const HomeStationStoreContext = createContext<HomeStationStore | null>(null)

export function useHomeStationStore(): HomeStationStore {
  const store = useContext(HomeStationStoreContext)
  if (!store) throw new Error('useHomeStationStore must be used inside HomeStationStoreContext')
  return store
}

export function useStoreSelector<T>(selector: (store: HomeStationStore, version: number) => T): T {
  const store = useHomeStationStore()
  const version = useSyncExternalStore(store.subscribe, store.getVersion)
  return useMemo(() => selector(store, version), [selector, store, version])
}

export function useCardView(objId: string) {
  const reader = useHsNav().reader
  const select = useCallback((store: HomeStationStore) => store.peekCard(objId, reader), [objId, reader])
  return useStoreSelector(select)
}

export function useStoreRevalidate(domains: StoreDomain[], revalidate: () => void) {
  const store = useHomeStationStore()
  const key = domains.join(',')
  const latest = useRef(revalidate)
  useEffect(() => {
    latest.current = revalidate
  })
  useEffect(() => store.onDomains(key.split(',') as StoreDomain[], () => latest.current()), [store, key])
}

let clockNow = Date.now()
const clockListeners = new Set<() => void>()
let clockTimer = 0

function subscribeClock(listener: () => void) {
  clockListeners.add(listener)
  if (!clockTimer) {
    clockTimer = window.setInterval(() => {
      clockNow = Date.now()
      for (const entry of clockListeners) entry()
    }, 30_000)
  }
  return () => {
    clockListeners.delete(listener)
    if (clockListeners.size === 0) {
      window.clearInterval(clockTimer)
      clockTimer = 0
    }
  }
}

export function useNow() {
  return useSyncExternalStore(subscribeClock, () => clockNow)
}
