/* Folded nodes (脑图方案 §7.3, R8): each reader's own state, never the document. Stored in the user work state under
 * `mindmap:collapsed:<container>` (the ids of folded nodes; ids of deleted nodes are ignored), so it follows the
 * account, works offline and stays out of commits, undo and exports. The book keeps the canvas layout's cache key:
 * its version moves only when a fold set it was asked about changes, not on every viewport save. */

import type { Json } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { Emitter } from '../../state/emitter'

export const foldKey = (containerId: string) => `mindmap:collapsed:${containerId}`

export class FoldBook {
  private readonly emitter = new Emitter()
  private readonly asked = new Map<string, string>()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.version

  private readonly store: WorkspaceStore

  constructor(store: WorkspaceStore) {
    this.store = store
    store.userState.subscribe(() => {
      let changed = false
      for (const [id, seen] of this.asked) {
        const now = JSON.stringify(store.userState.get(foldKey(id)) ?? null)
        if (now !== seen) { this.asked.set(id, now); changed = true }
      }
      if (changed) { this.version += 1; this.emitter.emit() }
    })
  }

  /** The folded nodes of `containerId`. */
  folded(containerId: string): ReadonlySet<string> {
    const value = this.store.userState.get<Json>(foldKey(containerId))
    this.asked.set(containerId, JSON.stringify(value ?? null))
    return new Set(Array.isArray(value) ? value.filter((v): v is string => typeof v === 'string') : [])
  }

  isFolded(containerId: string, nodeId: string): boolean {
    return this.folded(containerId).has(nodeId)
  }

  /** Fold or unfold `nodeIds` (no commit, no undo step: a reader's view). */
  set(containerId: string, nodeIds: string[], folded: boolean) {
    const next = new Set(this.folded(containerId))
    for (const id of nodeIds) { if (folded) next.add(id); else next.delete(id) }
    // only nodes that still exist are kept
    const alive = [...next].filter((id) => this.store.outline.get(id) && !this.store.outline.get(id)?.deleted)
    this.store.userState.set(foldKey(containerId), alive.length ? alive : null)
  }

  toggle(containerId: string, nodeId: string) {
    this.set(containerId, [nodeId], !this.isFolded(containerId, nodeId))
  }
}

const books = new WeakMap<WorkspaceStore, FoldBook>()
export function foldBook(store: WorkspaceStore): FoldBook {
  let book = books.get(store)
  if (!book) { book = new FoldBook(store); books.set(store, book) }
  return book
}
