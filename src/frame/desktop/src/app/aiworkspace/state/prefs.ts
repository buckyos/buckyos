/* The person's preferences across Workspaces (UI 框架 PRD §4.6, §12; K4): the tool zones' personal layout, where the
 * properties panel is pinned, whether the status bar is expanded, the home page's tab and filters. The service keeps
 * them (`user.get_prefs` / `user.set_prefs`), because every Workspace has its own origin: the browser's storage of one
 * Workspace page is not seen by another, nor by the home page, nor by another device.
 *
 * One store per Transport (the Desktop window has one AiwsClient for the home page and the open Workspace). Writes are
 * optimistic and merge per top-level key on the service (last writer wins per key). Offline behaviour: the last copy
 * this origin received is kept in localStorage (per identity) and used while the service cannot be reached; writes made
 * meanwhile are kept there too and sent with the next successful load or write. A session that keeps no preferences (a
 * guest, a link's visitor: PERMISSION_DENIED) keeps them in this page only — `state: 'unavailable'`. Another window's
 * change arrives when this one regains focus. */

import { useEffect, useSyncExternalStore } from 'react'
import { ServiceFailure, unwrap } from '../api/client'
import type { Transport } from '../api/transport'
import type { Json, Result, UserPrefsRead } from '../api/types'
import { describeError } from '../api/session'
import { identitySlot } from './recent'

const CACHE_KEY = 'aiworkspace.prefs'

export type PrefsState = 'loading' | 'synced' | 'local' | 'unavailable'

export interface UserPrefsSnapshot {
  /** The preferences as this page sees them: the service's copy with this page's unsent writes on top. */
  prefs: Readonly<Record<string, Json>>
  /** The service's revision of the copy last received (0: none yet). */
  revision: number
  /** `synced`: the service's copy; `local`: the service cannot be reached, the cached copy (writes wait); `unavailable`:
   * this session keeps no preferences (kept in this page only); `loading`: before the first answer. */
  state: PrefsState
  error: string | null
}

export interface UserPrefs extends UserPrefsSnapshot {
  /** One key's value, or `fallback` when unset. */
  get<T extends Json>(key: string, fallback: T): T
  /** Set (null: remove) top-level keys; resolves once the service has them (or they were kept for later). */
  set(changes: Record<string, Json | null>): Promise<void>
  /** Read the service's copy again. */
  reload(): Promise<void>
}

interface Cached { prefs: Record<string, Json>; revision: number; pending: Record<string, Json | null> }

function readCache(slot: string): Cached | null {
  try {
    const all = JSON.parse(window.localStorage.getItem(CACHE_KEY) ?? '{}') as Record<string, Cached>
    const c = all[slot]
    return c && typeof c === 'object' && c.prefs && typeof c.prefs === 'object' ? { prefs: c.prefs, revision: Number(c.revision) || 0, pending: c.pending ?? {} } : null
  } catch {
    return null
  }
}

function writeCache(slot: string, c: Cached) {
  try {
    const all = JSON.parse(window.localStorage.getItem(CACHE_KEY) ?? '{}') as Record<string, Cached>
    window.localStorage.setItem(CACHE_KEY, JSON.stringify({ ...all, [slot]: c }))
  } catch { /* a convenience: without storage the service's copy is all there is */ }
}

function applyChanges(base: Record<string, Json>, changes: Record<string, Json | null>): Record<string, Json> {
  const next = { ...base }
  for (const [k, v] of Object.entries(changes)) {
    if (v === null) delete next[k]
    else next[k] = v
  }
  return next
}

/** A refusal of the session kind (a guest, a link): nothing will be kept on the service, retrying does not help. */
function keepsNone(error: unknown): boolean {
  return error instanceof ServiceFailure && error.code === 'PERMISSION_DENIED'
}

export class UserPrefsStore {
  private readonly transport: Transport
  private server: Record<string, Json> = {}
  private pending: Record<string, Json | null> = {}
  private snap: UserPrefsSnapshot = { prefs: {}, revision: 0, state: 'loading', error: null }
  private readonly listeners = new Set<() => void>()
  private slot: Promise<string | null>
  private loading: Promise<void> | null = null

  constructor(transport: Transport) {
    this.transport = transport
    this.slot = identitySlot(transport).catch(() => null)
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    if (this.snap.state === 'loading' && !this.loading) void this.reload()
    return () => { this.listeners.delete(listener) }
  }

  snapshot = (): UserPrefsSnapshot => this.snap

  private publish(patch: Partial<UserPrefsSnapshot>) {
    this.snap = { ...this.snap, ...patch, prefs: applyChanges(this.server, this.pending) }
    for (const l of this.listeners) l()
  }

  private async keep(revision: number) {
    const slot = await this.slot
    if (slot) writeCache(slot, { prefs: this.server, revision, pending: this.pending })
  }

  reload(): Promise<void> {
    if (this.loading) return this.loading
    this.loading = (async () => {
      const slot = await this.slot
      if (this.snap.state === 'loading' && slot) {
        const cached = readCache(slot)
        if (cached) { this.server = cached.prefs; this.pending = { ...cached.pending, ...this.pending }; this.publish({ revision: cached.revision }) }
      }
      try {
        const r = unwrap(await this.transport.call<Result<UserPrefsRead>>('user.get_prefs', {}))
        this.server = r.prefs
        this.publish({ revision: r.revision, state: 'synced', error: null })
        await this.keep(r.revision)
        if (Object.keys(this.pending).length) await this.flush()
      } catch (error) {
        this.publish({ state: keepsNone(error) ? 'unavailable' : 'local', error: describeError(error) })
      }
    })().finally(() => { this.loading = null })
    return this.loading
  }

  private async flush(): Promise<void> {
    const sent = this.pending
    if (!Object.keys(sent).length) return
    try {
      const r = unwrap(await this.transport.call<Result<UserPrefsRead>>('user.set_prefs', { merge: sent }))
      // what was written meanwhile stays pending on top of the answer
      for (const [k, v] of Object.entries(sent)) if (this.pending[k] === v) delete this.pending[k]
      this.server = r.prefs
      this.publish({ revision: r.revision, state: 'synced', error: null })
      await this.keep(r.revision)
    } catch (error) {
      this.publish({ state: keepsNone(error) ? 'unavailable' : 'local', error: describeError(error) })
      await this.keep(this.snap.revision)
    }
  }

  async set(changes: Record<string, Json | null>): Promise<void> {
    this.pending = { ...this.pending, ...changes }
    this.publish({})
    if (this.snap.state === 'unavailable') return
    await this.flush()
  }
}

const stores = new WeakMap<Transport, UserPrefsStore>()

/** The preferences store of this window's Transport (created on first use). */
export function userPrefsStore(transport: Transport): UserPrefsStore {
  let s = stores.get(transport)
  if (!s) { s = new UserPrefsStore(transport); stores.set(transport, s) }
  return s
}

const NOBODY: UserPrefsSnapshot = { prefs: {}, revision: 0, state: 'unavailable', error: null }
const noop = () => () => undefined

/** The person's preferences across Workspaces (see the file comment); `transport` null: none (no service). */
export function useUserPrefs(transport: Transport | null): UserPrefs {
  const store = transport ? userPrefsStore(transport) : null
  const snap = useSyncExternalStore(store ? store.subscribe : noop, store ? store.snapshot : () => NOBODY)
  // another window (another Workspace, the home page) may have written: read again when this one is shown again
  useEffect(() => {
    if (!store) return
    const again = () => { if (document.visibilityState === 'visible') void store.reload() }
    document.addEventListener('visibilitychange', again)
    window.addEventListener('focus', again)
    return () => { document.removeEventListener('visibilitychange', again); window.removeEventListener('focus', again) }
  }, [store])
  return {
    ...snap,
    get: <T extends Json>(key: string, fallback: T): T => (key in snap.prefs ? snap.prefs[key] as T : fallback),
    set: (changes) => (store ? store.set(changes) : Promise.resolve()),
    reload: () => (store ? store.reload() : Promise.resolve()),
  }
}
