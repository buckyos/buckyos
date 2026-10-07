/* User work state (phase two §4.4, D5): top-level mode, canvas sub-mode, active Surface, per-Surface
 * viewports, panel widths, table view configuration without a Block… Saved on the server per user and
 * Workspace, and in this browser's IndexedDB so it is there offline and at once on the next open.
 * Never a document Commit: no history, no change-stream event, no undo, not in exports.
 *
 * Open: the local copy is applied immediately, then corrected by the server's entries (an entry this
 * window changed since is not overwritten). Writes: local at once, server throttled per key (high
 * frequency values such as viewports) and queued while offline; conflicts are last writer wins per entry.
 * A local entry the server does not have is dropped only when the server had confirmed it before (another
 * device removed it); one it never saw — written just before a reload or a close — is uploaded instead.
 * Closing flushes what is still waiting (best effort). */

import type { Json } from '../api/types'
import type { WorkspaceSession } from '../api/session'
import { Emitter } from './emitter'

const DB_NAME = 'aiworkspace-user-state'
const STORE = 'entries'
/** Keys written this often (viewport moves) are sent to the server at most every so often. */
const THROTTLE_MS = 800

function openDb(): Promise<IDBDatabase | null> {
  return new Promise((resolve) => {
    try {
      if (typeof indexedDB === 'undefined') { resolve(null); return }
      const request = indexedDB.open(DB_NAME, 1)
      request.onupgradeneeded = () => { request.result.createObjectStore(STORE) }
      request.onsuccess = () => resolve(request.result)
      request.onerror = () => resolve(null)
      request.onblocked = () => resolve(null)
    } catch {
      resolve(null)
    }
  })
}

function idbGetAll(db: IDBDatabase, prefix: string): Promise<Record<string, Json>> {
  return new Promise((resolve) => {
    try {
      const tx = db.transaction(STORE, 'readonly')
      const store = tx.objectStore(STORE)
      const out: Record<string, Json> = {}
      const request = store.openCursor(IDBKeyRange.bound(prefix, `${prefix}￿`))
      request.onsuccess = () => {
        const cursor = request.result
        if (!cursor) { resolve(out); return }
        out[String(cursor.key).slice(prefix.length)] = cursor.value as Json
        cursor.continue()
      }
      request.onerror = () => resolve(out)
    } catch {
      resolve({})
    }
  })
}

function idbWrite(db: IDBDatabase, entries: [string, Json | null][]): Promise<void> {
  return new Promise((resolve) => {
    try {
      const tx = db.transaction(STORE, 'readwrite')
      const store = tx.objectStore(STORE)
      for (const [key, value] of entries) { if (value === null) store.delete(key); else store.put(value, key) }
      tx.oncomplete = () => resolve()
      tx.onerror = () => resolve()
      tx.onabort = () => resolve()
    } catch {
      resolve()
    }
  })
}

export class UserWorkState {
  private entries = new Map<string, Json>()
  private readonly emitter = new Emitter()
  private readonly keyListeners = new Map<string, Set<() => void>>()
  private readonly prefix: string
  private db: IDBDatabase | null = null
  /** Entries changed locally and not yet accepted by the server. */
  private readonly dirty = new Map<string, Json | null>()
  /** Keys the server is known to hold (seen in a pull or accepted by a flush). */
  private readonly onServer = new Set<string>()
  private flushTimer: number | null = null
  private readonly session: WorkspaceSession
  private readonly offStatus: () => void
  private disposed = false
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = (): number => this.version
  /** `local` until the server answered once; `synced` afterwards; `offline` when writes wait. */
  syncState: 'local' | 'synced' | 'offline' = 'local'

  constructor(session: WorkspaceSession) {
    this.session = session
    this.prefix = `${session.workspaceId}|${session.principal ?? ''}|`
    this.offStatus = session.subscribeStatus(() => { if (session.status().kind === 'live') void this.pull().then(() => this.flush()) })
  }

  private bump(keys: Iterable<string>) {
    this.version += 1
    for (const key of keys) { const set = this.keyListeners.get(key); if (set) for (const listener of [...set]) listener() }
    this.emitter.emit()
  }

  subscribeKey(key: string, listener: () => void): () => void {
    let set = this.keyListeners.get(key)
    if (!set) { set = new Set(); this.keyListeners.set(key, set) }
    set.add(listener)
    return () => { set?.delete(listener) }
  }

  get<T extends Json = Json>(key: string): T | undefined {
    return this.entries.get(key) as T | undefined
  }

  /** Load the local copy (fast), then the server's (authoritative for what this window did not change). */
  async init(): Promise<void> {
    this.db = await openDb()
    if (this.db) {
      const local = await idbGetAll(this.db, this.prefix)
      for (const [key, value] of Object.entries(local)) this.entries.set(key, value)
      this.bump(Object.keys(local))
    }
    if (this.session.status().kind === 'live') await this.pull()
  }

  private async pull() {
    if (this.disposed) return
    let server: Record<string, Json>
    try { server = await this.session.getUserState() } catch { this.syncState = this.dirty.size > 0 ? 'offline' : this.syncState; return }
    const changed: string[] = []
    for (const [key, value] of Object.entries(server)) {
      this.onServer.add(key)
      if (this.dirty.has(key)) continue
      if (JSON.stringify(this.entries.get(key)) !== JSON.stringify(value)) { this.entries.set(key, value); changed.push(key) }
    }
    for (const key of [...this.entries.keys()]) {
      if (key in server || this.dirty.has(key)) continue
      // removed by another device: dropped here too; never seen by the server: sent to it
      if (this.onServer.has(key)) { this.onServer.delete(key); this.entries.delete(key); changed.push(key) } else this.dirty.set(key, this.entries.get(key) ?? null)
    }
    if (this.dirty.size > 0 && this.flushTimer === null) this.flushTimer = window.setTimeout(() => { this.flushTimer = null; void this.flush() }, THROTTLE_MS)
    this.syncState = this.dirty.size > 0 ? 'offline' : 'synced'
    if (this.db) void idbWrite(this.db, changed.map((key) => [this.prefix + key, this.entries.get(key) ?? null]))
    this.bump(changed)
    if (changed.length === 0) this.emitter.emit()
  }

  /** Write one entry: locally at once, to the server soon (throttled). `null` removes it. */
  set(key: string, value: Json | null) {
    if (value === null) this.entries.delete(key)
    else this.entries.set(key, value)
    this.dirty.set(key, value)
    if (this.db) void idbWrite(this.db, [[this.prefix + key, value]])
    this.bump([key])
    if (this.flushTimer === null) this.flushTimer = window.setTimeout(() => { this.flushTimer = null; void this.flush() }, THROTTLE_MS)
  }

  private async flush() {
    if (this.disposed || this.dirty.size === 0) return
    if (this.session.status().kind !== 'live') { this.syncState = 'offline'; this.emitter.emit(); return }
    const batch = Object.fromEntries(this.dirty)
    this.dirty.clear()
    try {
      await this.session.setUserState(batch)
      for (const [key, value] of Object.entries(batch)) { if (value === null) this.onServer.delete(key); else this.onServer.add(key) }
      this.syncState = this.dirty.size > 0 ? 'offline' : 'synced'
    } catch {
      // keep them for the next attempt unless a newer write replaced them meanwhile
      for (const [key, value] of Object.entries(batch)) if (!this.dirty.has(key)) this.dirty.set(key, value)
      this.syncState = 'offline'
    }
    this.emitter.emit()
  }

  /** Drop entries of Surfaces that no longer exist or are unreadable (phase two §4.4; UI improvement §12.1):
   * per-Surface keys (`viewport:<id>`, the phone's `phone-viewport:<id>`, `selection:<id>`) and the active Surface pointing at one of them.
   * Workspace-wide layout preferences live under `ui:` and are cleared only by "restore default layout". */
  pruneSurfaces(existing: ReadonlySet<string>) {
    const gone: string[] = []
    for (const key of this.entries.keys()) {
      const match = /^(viewport|phone-viewport|selection):(.+)$/.exec(key)
      if (match && !existing.has(match[2])) gone.push(key)
    }
    const active = this.entries.get('surface:active')
    if (typeof active === 'string' && !existing.has(active)) gone.push('surface:active')
    for (const key of gone) this.set(key, null)
  }

  dispose() {
    if (this.flushTimer !== null) { window.clearTimeout(this.flushTimer); this.flushTimer = null }
    // what is still waiting goes out now (best effort): a close right after a change keeps it
    if (this.dirty.size > 0 && this.session.status().kind === 'live') void this.session.setUserState(Object.fromEntries(this.dirty)).catch(() => undefined)
    this.disposed = true
    this.offStatus()
    this.db?.close()
  }
}
