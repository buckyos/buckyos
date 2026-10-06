/* Annotation anchor adapters (design §3.7). An adapter owns one range kind: it turns a view's
 * selection into an anchor (`capture`) and finds the range in that view again (`locate`). The
 * built-in kinds register here exactly like an application's `<app>/<name>` kinds; the backend
 * keeps application ranges verbatim and reports them as `unchecked`, so locating them is the
 * application's job. */

import type { AnnotationContext, AnnotationRange, Reference } from '../api/types'
import type { AnnotationMark } from '../state/hooks'

/** What a new annotation is about, captured from a selection. */
export interface CapturedAnchor {
  target: Reference
  range?: AnnotationRange
  context?: AnnotationContext
  /** Shown while the annotation is being written. */
  label: string
}

export interface AnchorAdapter<Host, Hit> {
  /** The range kind this adapter owns: a built-in kind or `<app>/<name>`. */
  kind: string
  /** An anchor for the host's current selection, or null when the selection is not this adapter's. */
  capture?(host: Host): CapturedAnchor | null
  /** Where the range of `mark` is in the host now, or null when it cannot be found. */
  locate(host: Host, mark: AnnotationMark): Hit | null
}

export class AnchorRegistry<Host, Hit> {
  private readonly adapters = new Map<string, AnchorAdapter<Host, Hit>>()
  private readonly listeners = new Set<() => void>()

  /** Returns the unregister function. Registering again under the same kind replaces the adapter. */
  register(adapter: AnchorAdapter<Host, Hit>): () => void {
    this.adapters.delete(adapter.kind)
    this.adapters.set(adapter.kind, adapter)
    this.emit()
    return () => {
      if (this.adapters.get(adapter.kind) !== adapter) return
      this.adapters.delete(adapter.kind)
      this.emit()
    }
  }

  get(kind: string): AnchorAdapter<Host, Hit> | undefined {
    return this.adapters.get(kind)
  }

  /** Capturing adapters, the most recently registered first: an application sees the selection before the built-in kinds. */
  capturers(): AnchorAdapter<Host, Hit>[] {
    return [...this.adapters.values()].filter((adapter) => adapter.capture).reverse()
  }

  subscribe(listener: () => void): () => void {
    this.listeners.add(listener)
    return () => { this.listeners.delete(listener) }
  }

  private emit() {
    for (const listener of [...this.listeners]) listener()
  }
}
