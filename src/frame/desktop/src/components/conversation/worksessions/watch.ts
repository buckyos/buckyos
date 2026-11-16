import type { WorkCards } from './model'
import type { WorkCardsRequest, WorkCardsSource } from './source'

/**
 * `unavailable`: no read has succeeded (no permission, an Agent that does
 * not know `session.work_cards`, unreachable) — the bubble keeps its task
 * area. `stale`: a read failed after a good one, so `cards` may be outdated.
 * `settled`: nothing more is read (the cards are complete, the Turn ended
 * without work sessions, or reading kept failing).
 */
export interface WorkCardsSnapshot {
  status: 'loading' | 'ready' | 'unavailable'
  cards?: WorkCards
  stale: boolean
  settled: boolean
}

/** `/obj/taskmgr/<root_id>/tree` hints of the UI Turn's task tree (the TaskMgr watch source). */
export interface TreeHints { subscribe(scope: 'tree', id: string, onHint: () => void): () => void }

export interface WorkCardsWatchOptions {
  /** A tree hint is read after this delay (later hints within it join the same read). */
  hintDebounceMs?: number
  /** Two reads of one bubble are at least this far apart. */
  minIntervalMs?: number
  /** Fallback poll while some card still changes. */
  pollMs?: number
  /** Reads that never succeeded are given up after this many failures. */
  maxFailures?: number
  /** A read that never succeeded is tried again after this delay, doubled per failure. */
  retryMs?: number
  /** Polling of stale cards stops after this many failures in a row (hints still read). */
  maxStaleFailures?: number
  readTimeoutMs?: number
  maxCached?: number
}

interface Entry {
  request: WorkCardsRequest
  refs: number
  rootTerminal: boolean
  failures: number
  lastReadAt: number
  reading: boolean
  again: boolean
  hintTimer?: ReturnType<typeof setTimeout>
  pollTimer?: ReturnType<typeof setTimeout>
  close?: () => void
}

export function workCardsKey({ agentDid, sessionId, taskId }: WorkCardsRequest): string {
  return `${agentDid}\n${sessionId}\n${taskId}`
}

/**
 * Work session cards of the mounted bubbles, one entry per (Agent, UI
 * session, UI Turn task) shared by every view. A watched entry is read at
 * once, then again on a tree hint of the UI Turn's task (debounced) and by a
 * slow poll while its cards are not complete; once `complete` (or the Turn
 * ended without work sessions) nothing is subscribed or polled and later
 * mounts use the cache. Unmounting the last viewer releases everything.
 */
export class WorkCardsWatch {
  private readonly source: WorkCardsSource
  private readonly hints: TreeHints
  private readonly hintDebounceMs: number
  private readonly minIntervalMs: number
  private readonly pollMs: number
  private readonly maxFailures: number
  private readonly retryMs: number
  private readonly maxStaleFailures: number
  private readonly readTimeoutMs: number
  private readonly maxCached: number
  private snapshots = new Map<string, WorkCardsSnapshot>()
  private entries = new Map<string, Entry>()
  private listeners = new Set<() => void>()

  constructor(source: WorkCardsSource, hints: TreeHints, options: WorkCardsWatchOptions = {}) {
    this.source = source
    this.hints = hints
    this.hintDebounceMs = options.hintDebounceMs ?? 1_000
    this.minIntervalMs = options.minIntervalMs ?? 2_000
    this.pollMs = options.pollMs ?? 15_000
    this.maxFailures = options.maxFailures ?? 4
    this.retryMs = options.retryMs ?? 3_000
    this.maxStaleFailures = options.maxStaleFailures ?? 6
    this.readTimeoutMs = options.readTimeoutMs ?? 10_000
    this.maxCached = options.maxCached ?? 200
  }

  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener) } }
  snapshot(request: WorkCardsRequest): WorkCardsSnapshot | undefined { return this.snapshots.get(workCardsKey(request)) }
  /** Whether the entry is still subscribed or polled. */
  isLive(request: WorkCardsRequest): boolean { const entry = this.entries.get(workCardsKey(request)); return !!entry && (!!entry.close || entry.pollTimer !== undefined || entry.hintTimer !== undefined || entry.reading) }

  /** Keeps the cards of a bubble current until released. `rootTerminal`: the UI Turn's task ended. */
  watch(request: WorkCardsRequest, rootTerminal = false): () => void {
    const key = workCardsKey(request)
    let entry = this.entries.get(key)
    if (!entry) { entry = { request, refs: 0, rootTerminal, failures: 0, lastReadAt: 0, reading: false, again: false }; this.entries.set(key, entry) }
    const current = entry
    current.refs++
    if (current.refs === 1) {
      const cached = this.snapshots.get(key)
      if (!cached) this.setSnapshot(key, { status: 'loading', stale: false, settled: false })
      if (!cached?.settled) {
        current.close = this.hints.subscribe('tree', request.taskId, () => this.hint(current))
        void this.read(current)
      }
    }
    let released = false
    return () => {
      if (released) return
      released = true
      current.refs--
      if (current.refs > 0) return
      this.stop(current)
      this.entries.delete(key)
      this.evict()
    }
  }

  /** The UI Turn's task ended: a Turn that created no work session is not followed any longer. */
  setRootTerminal(request: WorkCardsRequest, terminal: boolean) {
    const entry = this.entries.get(workCardsKey(request))
    if (!entry || entry.rootTerminal === terminal) return
    entry.rootTerminal = terminal
    if (!entry.reading) this.settle(entry)
  }

  /** Catch-up after a reconnect or the page becoming visible: every followed entry is read again now. */
  resync() {
    for (const entry of this.entries.values()) if (!this.snapshots.get(workCardsKey(entry.request))?.settled && entry.refs > 0) void this.read(entry)
  }

  dispose() {
    for (const entry of this.entries.values()) this.stop(entry)
    this.entries.clear()
    this.listeners.clear()
  }

  private hint(entry: Entry) {
    if (entry.refs === 0 || entry.hintTimer !== undefined) return
    if (entry.reading) { entry.again = true; return }
    const delay = Math.max(this.hintDebounceMs, entry.lastReadAt + this.minIntervalMs - Date.now())
    entry.hintTimer = setTimeout(() => { entry.hintTimer = undefined; void this.read(entry) }, delay)
  }

  private async read(entry: Entry): Promise<void> {
    if (entry.reading) { entry.again = true; return }
    entry.reading = true
    this.clearTimers(entry)
    entry.lastReadAt = Date.now()
    const key = workCardsKey(entry.request)
    try {
      const cards = await this.timed(this.source(entry.request))
      entry.failures = 0
      this.setSnapshot(key, { status: 'ready', cards, stale: false, settled: this.isSettled(entry, cards) })
    } catch {
      entry.failures++
      const previous = this.snapshots.get(key)
      if (previous?.cards) this.setSnapshot(key, { ...previous, stale: true })
      else this.setSnapshot(key, { status: 'unavailable', stale: false, settled: entry.failures >= this.maxFailures })
    } finally {
      entry.reading = false
      if (entry.again && entry.refs > 0 && !this.snapshots.get(key)?.settled) {
        entry.again = false
        this.hint(entry)
      } else {
        entry.again = false
        this.settle(entry)
      }
    }
  }

  private isSettled(entry: Entry, cards: WorkCards): boolean {
    return cards.complete || (cards.sessions.length === 0 && entry.rootTerminal)
  }

  /** Stops what a settled entry no longer needs, or schedules the next poll. */
  private settle(entry: Entry) {
    const key = workCardsKey(entry.request)
    const snapshot = this.snapshots.get(key)
    if (snapshot?.status === 'ready' && snapshot.cards && !snapshot.settled && !snapshot.stale && this.isSettled(entry, snapshot.cards)) this.setSnapshot(key, { ...snapshot, settled: true })
    const current = this.snapshots.get(key)
    if (current?.settled) { this.stop(entry); return }
    if (entry.refs === 0 || entry.pollTimer !== undefined) return
    // A read that never succeeded is tried again (a finished Turn sends no hints).
    if (current?.status === 'unavailable') {
      entry.pollTimer = setTimeout(() => { entry.pollTimer = undefined; void this.read(entry) }, this.retryMs * 2 ** Math.max(0, entry.failures - 1))
      return
    }
    // Only cards still changing are polled; a Turn without work sessions so far waits for tree hints.
    const changing = !!current?.cards && current.cards.sessions.length > 0
    if (!changing || (current?.stale && entry.failures >= this.maxStaleFailures)) return
    entry.pollTimer = setTimeout(() => { entry.pollTimer = undefined; void this.read(entry) }, this.pollMs)
  }

  private clearTimers(entry: Entry) {
    if (entry.hintTimer !== undefined) { clearTimeout(entry.hintTimer); entry.hintTimer = undefined }
    if (entry.pollTimer !== undefined) { clearTimeout(entry.pollTimer); entry.pollTimer = undefined }
  }

  private stop(entry: Entry) {
    this.clearTimers(entry)
    entry.close?.()
    entry.close = undefined
  }

  private setSnapshot(key: string, snapshot: WorkCardsSnapshot) {
    const previous = this.snapshots.get(key)
    if (previous && JSON.stringify(previous) === JSON.stringify(snapshot)) return
    this.snapshots.delete(key)
    this.snapshots.set(key, snapshot)
    this.notify()
  }

  private notify() { this.listeners.forEach(listener => listener()) }

  private evict() {
    for (const key of this.snapshots.keys()) {
      if (this.snapshots.size <= this.maxCached) return
      if (!this.entries.has(key)) this.snapshots.delete(key)
    }
  }

  private timed<T>(promise: Promise<T>): Promise<T> {
    return new Promise<T>((resolve, reject) => {
      const timer = setTimeout(() => reject(Error('timeout')), this.readTimeoutMs)
      promise.then(value => { clearTimeout(timer); resolve(value) }, error => { clearTimeout(timer); reject(error) })
    })
  }
}
