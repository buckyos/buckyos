/**
 * Collection data model + member-management API.
 *
 * A collection is an ordered tree of nodes: references (canonical target
 * URLs, scheme-extensible) and groups (nesting inside the collection — not
 * real folders). The interface shape here is the future backend RPC shape;
 * the mock store in `../mock/collections.ts` implements it in memory.
 */

import type { FolderReader } from './FolderReader'

export type CollectionNode =
  | { type: 'ref'; key: string; targetUrl: string }
  //  targetUrl is any canonical URL: the reference type is decided by its
  //  scheme (`dfs://` today) and resolved through the target-resolver
  //  registry — new reference types register a scheme, the model is unchanged.
  | { type: 'group'; key: string; name: string; children: CollectionNode[] }

/** Narrow a resolved reader to the collection interface (capability + shape check). */
export function asCollectionReader(reader: FolderReader): CollectionReader | null {
  if (reader.capabilities.kind !== 'collection') return null
  return 'addReferences' in reader ? (reader as CollectionReader) : null
}

export interface CollectionReader extends FolderReader {
  /** Append (or insert at `position`) references to existing items by canonical URL. */
  addReferences(targets: string[], position?: number): Promise<void>
  removeItems(itemKeys: string[]): Promise<void>
  /** Manual ordering: move `itemKeys` as a block to `toIndex` (drives the default Preview order). */
  reorder(itemKeys: string[], toIndex: number): Promise<void>
  createGroup(name: string, position?: number): Promise<void>
  renameGroup(itemKey: string, name: string): Promise<void>
}
