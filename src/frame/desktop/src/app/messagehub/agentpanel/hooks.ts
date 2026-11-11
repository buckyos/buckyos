import { useCallback, useEffect, useRef, useState } from 'react'
import { getTaskWatchSource } from '../conversation/tasks'

export interface PanelResource<T> {
  /** `unavailable`: no read has succeeded (not the owner, the Agent is offline). */
  status: 'loading' | 'ready' | 'unavailable'
  value?: T
  /** A read failed after a good one: `value` may be outdated. */
  stale: boolean
  refresh: () => void
}

const POLL_MS = 15_000
const HINT_DEBOUNCE_MS = 1_000

/**
 * A read of the Agent's service kept current while the caller is mounted
 * (a closed panel reads nothing): at once, again on a TaskMgr tree hint of
 * `hintTaskId` (debounced), whenever `signal` changes, and every 15 seconds
 * while `live(value)` holds. `key: null` reads nothing.
 */
export function usePanelResource<T>(key: string | null, read: () => Promise<T>, options: { live?: (value: T) => boolean; hintTaskId?: string; signal?: unknown } = {}): PanelResource<T> {
  const [state, setState] = useState<{ key: string | null; status: PanelResource<T>['status']; value?: T; stale: boolean }>({ key, status: 'loading', stale: false })
  const reader = useRef(read)
  const live = useRef(options.live)
  const load = useRef<() => void>(() => undefined)
  useEffect(() => { reader.current = read; live.current = options.live })
  useEffect(() => {
    if (key === null) return
    let cancelled = false, reading = false, again = false
    let timer: ReturnType<typeof setTimeout> | undefined
    const run = async () => {
      if (reading) { again = true; return }
      reading = true
      if (timer !== undefined) { clearTimeout(timer); timer = undefined }
      let next: T | undefined
      try {
        next = await reader.current()
        if (!cancelled) setState({ key, status: 'ready', value: next, stale: false })
      } catch {
        if (!cancelled) setState(previous => previous.key === key && previous.value !== undefined ? { ...previous, stale: true } : { key, status: 'unavailable', stale: false })
      }
      reading = false
      if (cancelled) return
      if (again) { again = false; void run(); return }
      if (next === undefined || (live.current?.(next) ?? true)) timer = setTimeout(() => { timer = undefined; void run() }, POLL_MS)
    }
    load.current = () => { void run() }
    // Deferred, so a mount that is undone at once (React's development double mount) reads nothing.
    timer = setTimeout(() => { timer = undefined; void run() }, 0)
    return () => { cancelled = true; load.current = () => undefined; if (timer !== undefined) clearTimeout(timer) }
  }, [key])
  const { hintTaskId, signal } = options
  useEffect(() => {
    if (key === null || !hintTaskId) return
    let timer: ReturnType<typeof setTimeout> | undefined
    const close = getTaskWatchSource().subscribe('tree', hintTaskId, () => {
      if (timer !== undefined) return
      timer = setTimeout(() => { timer = undefined; load.current() }, HINT_DEBOUNCE_MS)
    })
    return () => { close(); if (timer !== undefined) clearTimeout(timer) }
  }, [key, hintTaskId])
  const seen = useRef(signal)
  useEffect(() => {
    if (Object.is(seen.current, signal)) return
    seen.current = signal
    load.current()
  }, [signal])
  const refresh = useCallback(() => load.current(), [])
  if (state.key !== key) return { status: 'loading', stale: false, refresh }
  return { status: state.status, value: state.value, stale: state.stale, refresh }
}

/** One operation at a time; `error` holds the last failure's message until the next run. */
export function usePanelAction() {
  const [pending, setPending] = useState<string | null>(null)
  const [error, setError] = useState('')
  const busy = useRef(false)
  const run = useCallback(async (name: string, operation: () => Promise<unknown>) => {
    if (busy.current) return false
    busy.current = true; setPending(name); setError('')
    try { await operation(); return true } catch (failure) { setError(failure instanceof Error ? failure.message : String(failure)); return false } finally { busy.current = false; setPending(null) }
  }, [])
  return { pending, error, run }
}
