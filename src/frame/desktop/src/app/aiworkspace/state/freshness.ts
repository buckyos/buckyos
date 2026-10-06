/* FreshnessService (phase two §7.5 rule 8): the front end subscribes to the freshness the core
 * computed (`doc.freshness`, online or in the replica) and displays it; it never derives it. Change
 * events invalidate the cache; subscribed entities are re-read in one batched call. */

import type { WorkspaceSession } from '../api/session'
import type { FreshnessInfo } from '../api/types'
import { Emitter } from './emitter'

const REFRESH_DELAY_MS = 120

export class FreshnessService {
  private readonly cache = new Map<string, FreshnessInfo>()
  private readonly wanted = new Map<string, number>()
  private readonly emitter = new Emitter()
  private readonly keyListeners = new Map<string, Set<() => void>>()
  private timer: number | null = null
  private pending = new Set<string>()
  private readonly session: WorkspaceSession
  private disposed = false
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = (): number => this.version

  constructor(session: WorkspaceSession) {
    this.session = session
  }

  get(id: string): FreshnessInfo | undefined { return this.cache.get(id) }

  /** A view that shows `id` keeps it fetched while mounted. */
  watch(id: string, listener: () => void): () => void {
    this.wanted.set(id, (this.wanted.get(id) ?? 0) + 1)
    let set = this.keyListeners.get(id)
    if (!set) { set = new Set(); this.keyListeners.set(id, set) }
    set.add(listener)
    if (!this.cache.has(id)) this.schedule([id])
    return () => {
      set?.delete(listener)
      const n = (this.wanted.get(id) ?? 1) - 1
      if (n <= 0) this.wanted.delete(id); else this.wanted.set(id, n)
    }
  }

  private schedule(ids: Iterable<string>) {
    for (const id of ids) this.pending.add(id)
    if (this.timer !== null) return
    this.timer = window.setTimeout(() => { this.timer = null; void this.fetch() }, REFRESH_DELAY_MS)
  }

  private async fetch() {
    if (this.disposed) return
    const ids = [...this.pending].filter((id) => this.wanted.has(id))
    this.pending.clear()
    if (ids.length === 0) return
    let items: FreshnessInfo[]
    try { items = await this.session.freshness(ids) } catch (error) {
      items = ids.map((id) => ({ entity_id: id, status: 'unknown', error: { code: 'UNAVAILABLE', detail: error instanceof Error ? error.message : String(error) } }))
    }
    this.version += 1
    for (const item of items) {
      this.cache.set(item.entity_id, item)
      const set = this.keyListeners.get(item.entity_id)
      if (set) for (const listener of [...set]) listener()
    }
    this.emitter.emit()
  }

  /** Something changed in the document: everything watched is re-read (dependencies may cross any entity). */
  invalidate() {
    if (this.wanted.size === 0) { this.cache.clear(); return }
    this.schedule(this.wanted.keys())
  }

  dispose() {
    this.disposed = true
    if (this.timer !== null) window.clearTimeout(this.timer)
  }
}
