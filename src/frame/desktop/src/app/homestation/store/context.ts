import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useSyncExternalStore } from 'react'
import type { ReaderIdentity } from '../datamodel/types'
import { useHsNav } from '../navContext'
import type { HomeStationStore, StoreDomain } from './types'

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
  const store = useHomeStationStore()
  const reader = useHsNav().reader
  const readerKey = JSON.stringify(reader)
  useEffect(() => store.watchCard(objId, JSON.parse(readerKey) as ReaderIdentity), [store, objId, readerKey])
  const select = useCallback((current: HomeStationStore) => current.peekCard(objId, reader), [objId, reader])
  return useStoreSelector(select)
}

export type PreviewReaderKey = 'anonymous' | 'follower' | 'friend'

const ANONYMOUS_READER: ReaderIdentity = { kind: 'anonymous' }
const selectPreviewFollower = (store: HomeStationStore) => store.peekPreviewReaders().follower
const selectPreviewFriend = (store: HomeStationStore) => store.peekPreviewReaders().friend

export function usePreviewReaders(): { anonymous: ReaderIdentity } & Partial<Record<PreviewReaderKey, ReaderIdentity>> {
  const follower = useStoreSelector(selectPreviewFollower)
  const friend = useStoreSelector(selectPreviewFriend)
  return useMemo(() => ({
    anonymous: ANONYMOUS_READER,
    ...(follower ? { follower: { kind: 'did' as const, did: follower } } : {}),
    ...(friend ? { friend: { kind: 'did' as const, did: friend } } : {}),
  }), [follower, friend])
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
