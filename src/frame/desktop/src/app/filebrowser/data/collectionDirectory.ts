/**
 * Collection directory — the sidebar-facing "which collections exist"
 * projection plus creation, decoupled from any concrete store. The mock
 * registers a wrapper around its in-memory store; the NFSP adapter registers
 * a server-backed directory (create_collection / open_collection). Membership
 * operations stay on CollectionReader — this is only list/create/lookup.
 */

import { useCallback, useSyncExternalStore } from 'react'
import type { CollectionId, CollectionSummary } from '../types'

export interface CollectionDirectory {
  /** Change notification for useSyncExternalStore. */
  subscribe(listener: () => void): () => void
  /** Monotonic snapshot token. */
  snapshot(): number
  /** Current summaries (cached projection; may refresh asynchronously). */
  list(): CollectionSummary[]
  get(id: CollectionId): { title: string; description?: string } | undefined
  /** Server owns the id (UI_DATAMODEL.md §3): resolves with the new id. */
  create(title: string): Promise<CollectionId>
}

const emptyDirectory: CollectionDirectory = {
  subscribe: () => () => {},
  snapshot: () => 0,
  list: () => [],
  get: () => undefined,
  create: () => Promise.reject(new Error('filebrowser: no collection directory registered')),
}

let active: CollectionDirectory = emptyDirectory

export function registerCollectionDirectory(directory: CollectionDirectory): () => void {
  active = directory
  return () => {
    if (active === directory) active = emptyDirectory
  }
}

export function collectionDirectory(): CollectionDirectory {
  return active
}

/** Live collection summaries for the sidebar and "Add to Collection" menus. */
export function useCollections(): CollectionSummary[] {
  const subscribe = useCallback((listener: () => void) => active.subscribe(listener), [])
  useSyncExternalStore(subscribe, () => active.snapshot())
  return active.list()
}
