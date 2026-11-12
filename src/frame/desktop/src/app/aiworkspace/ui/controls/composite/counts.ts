/* How many items each composite projection has now (控件机制 TODO G2, G13): the projection's data is the reader's, read
 * after the outline, so the canvas layout asks this book for the counts and re-lays out when one changes. */

import { Emitter } from '../../../state/emitter'
import type { WorkspaceStore } from '../../../state/store'

export class ProjectionCounts {
  private readonly counts = new Map<string, number>()
  private readonly emitter = new Emitter()
  private version = 0
  readonly subscribe = this.emitter.subscribe
  snapshot = () => this.version

  get(containerId: string, projection: string): number | undefined {
    return this.counts.get(`${containerId}\n${projection}`)
  }

  set(containerId: string, projection: string, count: number) {
    const key = `${containerId}\n${projection}`
    if (this.counts.get(key) === count) return
    this.counts.set(key, count)
    this.version += 1
    this.emitter.emit()
  }
}

const books = new WeakMap<WorkspaceStore, ProjectionCounts>()
export function projectionCounts(store: WorkspaceStore): ProjectionCounts {
  let book = books.get(store)
  if (!book) { book = new ProjectionCounts(); books.set(store, book) }
  return book
}
