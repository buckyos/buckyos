/* ── Whether the Zone can be reached ──
 *
 * Judged by the page's kevent streams, never by `navigator.onLine`: a computer without any network still reaches the
 * Zone it runs itself. A drop counts only after GRACE_MS without an answer, so loading, a short interruption and a
 * change of network do not show. Any HTTP answer, even a refusal, means the Zone was reached.
 */
import { buckyos } from 'buckyos'
import { isMockRuntime } from '../runtime.ts'
import { openMockEventReader } from './zone_link_mock.ts'

export type ZoneEventReader = Pick<Awaited<ReturnType<typeof buckyos.createEventReader>>, 'pullEvent' | 'close'>
export type ZoneEvent = NonNullable<Awaited<ReturnType<ZoneEventReader['pullEvent']>>>

interface Stream {
  patterns: string[]
  onEvent: (event: ZoneEvent) => void
  stopped: boolean
  reader: ZoneEventReader | null
  attempt: AbortController | null
}

const RECONNECT_MS = 1_000
const OPEN_TIMEOUT_MS = 10_000
const GRACE_MS = 5_000
/** What the link's own stream listens to while the page follows nothing else. */
const LINK_PATTERNS = ['/obj/control-panel/zone-link']

const streams = new Set<Stream>()
const reachableListeners = new Set<() => void>()
const recoveredListeners = new Set<() => void>()
let watchers = 0
let linkStream: Stream | null = null
let reachable = true
let live = 0
/** Since when nothing got through; null while something does. */
let lostAt: number | null = null
let missedEvents = false

class OpenTimeout extends Error {}

function setReachable(value: boolean) {
  if (reachable === value) return
  reachable = value
  reachableListeners.forEach((listener) => listener())
}

function answered(acknowledged: boolean) {
  const recovered = lostAt !== null || (acknowledged && missedEvents)
  lostAt = null
  if (acknowledged) missedEvents = false
  setReachable(true)
  if (recovered) recoveredListeners.forEach((listener) => listener())
}

function unanswered(since: number) {
  if (live > 0) return
  lostAt ??= since
  const visible = typeof document === 'undefined' || document.visibilityState !== 'hidden'
  if (visible && Date.now() - lostAt >= GRACE_MS) setReachable(false)
}

function dropped() {
  missedEvents = true
  if (live === 0) lostAt ??= Date.now()
}

if (typeof document !== 'undefined') {
  // A page that was hidden may have been frozen: what it lost meanwhile counts from now.
  document.addEventListener('visibilitychange', () => { if (reachable && lostAt !== null) lostAt = Date.now() })
}

async function open(stream: Stream): Promise<ZoneEventReader> {
  if (isMockRuntime()) return openMockEventReader()
  const attempt = new AbortController()
  stream.attempt = attempt
  let timer: ReturnType<typeof setTimeout> | undefined
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(() => { reject(new OpenTimeout('the Zone did not answer')); attempt.abort() }, OPEN_TIMEOUT_MS)
  })
  try {
    return await Promise.race([buckyos.createEventReader(stream.patterns, { signal: attempt.signal }), expired])
  } finally {
    clearTimeout(timer)
  }
}

async function follow(stream: Stream) {
  while (!stream.stopped) {
    const startedAt = Date.now()
    try {
      const reader = await open(stream)
      stream.reader = reader
      if (stream.stopped) break
      live += 1
      try {
        answered(true)
        for (let event = await reader.pullEvent(); event && !stream.stopped; event = await reader.pullEvent()) {
          try { stream.onEvent(event) } catch (error) { console.error('zone event handler failed', error) }
        }
      } finally {
        live -= 1
      }
      if (!stream.stopped) dropped()
    } catch (error) {
      if (stream.stopped) break
      if (error instanceof TypeError || error instanceof OpenTimeout) unanswered(startedAt)
      else answered(false)
    } finally {
      void stream.reader?.close()
      stream.reader = null
    }
    await new Promise((resolve) => setTimeout(resolve, RECONNECT_MS))
  }
}

function start(patterns: string[], onEvent: (event: ZoneEvent) => void): Stream {
  const stream: Stream = { patterns, onEvent, stopped: false, reader: null, attempt: null }
  streams.add(stream)
  void follow(stream)
  return stream
}

function stop(stream: Stream) {
  stream.stopped = true
  streams.delete(stream)
  stream.attempt?.abort()
  void stream.reader?.close()
}

function balance() {
  const followed = streams.size - (linkStream ? 1 : 0)
  if (linkStream && (watchers === 0 || followed > 0)) {
    stop(linkStream)
    linkStream = null
  } else if (!linkStream && watchers > 0 && followed === 0) {
    linkStream = start(LINK_PATTERNS, () => undefined)
  }
  if (streams.size > 0) return
  lostAt = null
  missedEvents = false
  setReachable(true)
}

/** Follows kevents until the returned function is called, reconnecting on its own; what was missed meanwhile is announced by `onZoneRecovered`. */
export function subscribeZoneEvents(patterns: string[], onEvent: (event: ZoneEvent) => void): () => void {
  const stream = start(patterns, onEvent)
  balance()
  return () => {
    if (stream.stopped) return
    stop(stream)
    balance()
  }
}

/** Keeps the state current (with a stream of its own if the page follows nothing) until the returned function is called. */
export function watchZoneLink(): () => void {
  watchers += 1
  balance()
  let released = false
  return () => {
    if (released) return
    released = true
    watchers -= 1
    balance()
  }
}

export const isZoneReachable = () => reachable

export function subscribeZoneReachable(listener: () => void): () => void {
  reachableListeners.add(listener)
  return () => { reachableListeners.delete(listener) }
}

/** Time to read again: the Zone answers after it did not, events were missed, or the system reports a network. */
export function onZoneRecovered(listener: () => void): () => void {
  recoveredListeners.add(listener)
  window.addEventListener('online', listener)
  return () => {
    recoveredListeners.delete(listener)
    window.removeEventListener('online', listener)
  }
}
