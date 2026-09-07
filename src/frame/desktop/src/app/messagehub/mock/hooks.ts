import { useEffect, useState, useSyncExternalStore } from 'react'
import { messageHubStore } from './store'

export function useMessageHubStore() {
  useSyncExternalStore(messageHubStore.subscribe, messageHubStore.getSnapshot)
  return messageHubStore
}
export function useMessageHubClock() {
  const now = useSyncExternalStore(messageHubStore.subscribeTime, messageHubStore.getTime)
  useEffect(() => { const timer = setInterval(messageHubStore.tick, 60_000); return () => clearInterval(timer) }, [])
  return now
}
export function useMessageHubRuntime() {
  useSyncExternalStore(messageHubStore.subscribeRuntime, messageHubStore.getRuntimeVersion)
  useEffect(() => { const timer = setInterval(messageHubStore.tick, 1000); return () => clearInterval(timer) }, [])
}
export function useMockReady() {
  const [status, setStatus] = useState<'loading' | 'ready' | 'error'>('loading')
  const load = () => { void messageHubStore.initialize().then(() => setStatus('ready'), () => setStatus('error')) }
  useEffect(load, [])
  return { status, retry: load }
}
