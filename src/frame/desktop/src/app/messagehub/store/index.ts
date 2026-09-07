import { useEffect, useState, useSyncExternalStore } from 'react'
import { isMockRuntime } from '../../../runtime'
import { MessageHubMockStore } from '../mock/store'
import { MessageHubApiStore } from '../api/store'
import type { MessageHubStore } from './types'

export type { ConnectionChoice, EntityAdmission, ManageAction, MessageHubStore, OutgoingPayload, OwnerStatus } from './types'

const mockModeValues = new Set(['1', 'true', 'yes', 'mock'])

/** `VITE_MESSAGEHUB_USE_MOCK` overrides the desktop-wide mock flag. */
export function isMessageHubMock(): boolean {
  const value = import.meta.env.VITE_MESSAGEHUB_USE_MOCK
  if (value !== undefined && String(value).trim() !== '') {
    return mockModeValues.has(String(value).trim().toLowerCase())
  }
  return isMockRuntime()
}

let instance: MessageHubStore | null = null

/**
 * The store is created lazily so the mock seed (and its IndexedDB) is never
 * touched in real mode and the msg-center client is never touched in mock
 * mode.
 */
export function getMessageHubStore(): MessageHubStore {
  if (instance) return instance
  instance = isMessageHubMock() ? new MessageHubMockStore() : new MessageHubApiStore()
  if (import.meta.env.DEV && typeof window !== 'undefined') Object.assign(window, { __messageHubStore: instance })
  return instance
}

export function useMessageHubStore(): MessageHubStore {
  const store = getMessageHubStore()
  useSyncExternalStore(store.subscribe, store.getSnapshot)
  return store
}

export function useMessageHubClock() {
  const store = getMessageHubStore()
  const now = useSyncExternalStore(store.subscribeTime, store.getTime)
  useEffect(() => { const timer = setInterval(store.tick, 60_000); return () => clearInterval(timer) }, [store])
  return now
}

export function useMessageHubRuntime() {
  const store = getMessageHubStore()
  useSyncExternalStore(store.subscribeRuntime, store.getRuntimeVersion)
  useEffect(() => { const timer = setInterval(store.tick, 1000); return () => clearInterval(timer) }, [store])
}

export function useMessageHubReady() {
  const store = getMessageHubStore()
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const [attempt, setAttempt] = useState(0)
  useEffect(() => {
    let cancelled = false
    store.initialize().then(() => { if (!cancelled) setStatus('ready') }, () => { if (!cancelled) setStatus('error') })
    return () => { cancelled = true }
  }, [store, attempt])
  const retry = () => { setStatus('loading'); setAttempt(value => value + 1) }
  return { status, retry }
}
