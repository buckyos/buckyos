/**
 * NFSP-backed CollectionReader (UI_DATAMODEL.md §8.2/§8.3).
 *
 * The name-based `collection://<id>/<group…>` URL stays the UI locator, but
 * identity is entry_ref underneath (§2.2/§9 item 8): each group segment is
 * resolved by walking the parent listing and capturing the group entry's
 * `entry_ref` + target Ref — slash-joined names are never sent as an id.
 * Member operations go through `collection_patch` with entry refs and the
 * last seen revision as CAS; a mismatch surfaces as a conflict (§4.6).
 */

import type { CollectionPatchOp, Listing, LocatorLike, WireRef } from '../../../../api/nfsp_client'
import type { CollectionReader } from '../CollectionModel'
import type { FileItem, LocationCapabilities, LocationMeta } from '../FolderReader'
import { registerReaderProvider } from '../readerRegistry'
import { collectionUrl, dfsPathOf, parseCollectionUrl } from '../urls'
import { ensureSession, nfspClient } from './client'
import { nfspToUiError } from './errors'
import { mapEntryToItem, refIdOf, unixToIso, NFSP_SORT_DIRS } from './mapping'
import { NfspContainerReader } from './readers'
import { refreshNfspCollectionDirectory } from './collectionDirectory'

const PROVISIONAL_COLLECTION: LocationCapabilities = {
  kind: 'collection',
  acceptsContent: false,
  acceptsReferences: true,
  removal: 'remove-ref',
  canReorder: true,
  sortKeys: ['manual', 'name', 'size', 'modified'],
  sortDirs: NFSP_SORT_DIRS,
  defaultSortKey: 'manual',
}

interface ResolvedContainer {
  /** Ref of the listed container (collection root or walked group). */
  ref: WireRef
  /** Ref of the collection root — collection_patch is addressed here. */
  rootRef: WireRef
  /** entry_ref of the deepest group (add_ref parent), undefined at the root. */
  groupEntryRef?: string
  title: string
  description?: string
}

class NfspCollectionReader extends NfspContainerReader implements CollectionReader {
  private readonly collectionId: string
  private readonly groupPath: string[]
  private resolved: ResolvedContainer | null = null
  private lastRevision: string | undefined

  constructor(url: string) {
    super(url, PROVISIONAL_COLLECTION)
    const parts = parseCollectionUrl(url)
    if (!parts) throw new Error(`bad collection url: ${url}`)
    this.collectionId = parts.collectionId
    this.groupPath = parts.groupPath
  }

  protected cacheMode(): 'no-cache' | undefined {
    // Group listings bypass the cache — invalidation is root-addressed.
    return this.groupPath.length > 0 ? 'no-cache' : undefined
  }

  /** Walk `groupPath` by name, capturing entry_ref-backed identity per level. */
  private async ensureContainer(): Promise<ResolvedContainer> {
    if (this.resolved) return this.resolved
    await ensureSession()
    const client = nfspClient()
    const rootInfo = await client.resolve({ uri: `collection://${this.collectionId}` }, ['base'])
    if (!rootInfo) throw nfspToUiError({ code: 'NOT_FOUND', message: 'collection gone' })
    let ref = rootInfo.ref
    let groupEntryRef: string | undefined
    for (const name of this.groupPath) {
      const found = await this.findGroupEntry(ref, name)
      ref = found.ref
      groupEntryRef = found.entryRef
    }
    this.resolved = {
      ref,
      rootRef: rootInfo.ref,
      groupEntryRef,
      title: rootInfo.title ?? this.collectionId,
    }
    this.containerRefIds.add(refIdOf(rootInfo.ref))
    this.containerRefIds.add(refIdOf(ref))
    return this.resolved
  }

  private async findGroupEntry(
    parent: WireRef,
    name: string,
  ): Promise<{ ref: WireRef; entryRef: string }> {
    const client = nfspClient()
    let cursor: string | undefined
    for (;;) {
      const listing = await client.list(
        parent,
        cursor ? { cursor, order: 'manual' } : { order: 'manual' },
        undefined,
        { cache: 'no-cache' },
      )
      if (!listing) break
      const hit = listing.entries.find(
        (entry) => entry.target.kind === 'group' && entry.name === name,
      )
      if (hit) {
        if (!hit.entry_ref) {
          // Missing Collection entry_ref is a protocol error, never permission
          // to fall back to the target id (§8.2).
          throw nfspToUiError({ code: 'INTERNAL', message: 'group without entry_ref' })
        }
        return { ref: hit.target.ref, entryRef: hit.entry_ref }
      }
      if (!listing.next_cursor) break
      cursor = listing.next_cursor
    }
    throw nfspToUiError({ code: 'NOT_FOUND', message: `group '${name}' not found` })
  }

  protected async locator(): Promise<LocatorLike> {
    const container = await this.ensureContainer()
    return { ref: container.ref }
  }

  protected refineCapabilities(mapped: LocationCapabilities): LocationCapabilities {
    // The §2.5 collection column is authoritative for the whole subtree.
    return { ...mapped, kind: 'collection', removal: 'remove-ref' }
  }

  protected locationMetaOf(listing: Listing): LocationMeta | undefined {
    this.lastRevision = listing.container.revision
    const title = this.resolved?.title ?? this.collectionId
    return {
      title: this.groupPath.length ? `${title} › ${this.groupPath.join(' › ')}` : title,
      description: this.resolved?.description,
    }
  }

  protected mapListing(listing: Listing, offset: number): FileItem[] {
    return listing.entries.map((entry, index): FileItem => {
      const orderIndex = offset + index
      if (entry.target.kind === 'group') {
        const path = collectionUrl(this.collectionId, [...this.groupPath, entry.name])
        return {
          key: entry.entry_ref ?? refIdOf(entry.target.ref),
          entry: {
            id: refIdOf(entry.target.ref),
            name: entry.name,
            kind: 'folder',
            // Groups are collection structure, not real folders: their path
            // *is* the collection URL, so "open" naturally navigates there.
            path,
            modifiedAt: unixToIso(entry.target.attrs?.mtime),
          },
          ref: { collectionUrl: this.url, refPath: path, orderIndex },
        }
      }
      const item = mapEntryToItem(entry, {})
      const broken = entry.target.target_state !== undefined && entry.target.target_state !== 'ok'
      return {
        ...item,
        ref: {
          collectionUrl: this.url,
          refPath: `${this.url}/${item.entry.name}`,
          orderIndex,
          broken,
        },
      }
    })
  }

  // ─── Member management (§8.3) ───

  private async patch(ops: CollectionPatchOp[]): Promise<void> {
    const container = await this.ensureContainer()
    try {
      await nfspClient().collectionPatch(container.rootRef, ops, {
        expectedRevision: this.lastRevision,
      })
    } catch (err) {
      throw nfspToUiError(err)
    }
    this.notifyLocal()
    refreshNfspCollectionDirectory()
  }

  async addReferences(targets: string[], position?: number): Promise<void> {
    const container = await this.ensureContainer()
    const client = nfspClient()
    const ops: CollectionPatchOp[] = []
    for (const [index, target] of targets.entries()) {
      // Locators resolve to a stable target Ref before binding (§8.3). A
      // canonical dfs:// URL becomes a path locator; other schemes pass as uri.
      const path = dfsPathOf(target)
      const info = await client.resolve(path ?? { uri: target }).catch((err: unknown) => {
        throw nfspToUiError(err)
      })
      if (!info) throw nfspToUiError({ code: 'NOT_FOUND', message: 'target gone' })
      ops.push({
        add_ref: {
          target_ref: info.ref,
          position: position !== undefined ? position + index : undefined,
          parent_entry_ref: container.groupEntryRef,
        },
      })
    }
    await this.patch(ops)
  }

  async removeItems(itemKeys: string[]): Promise<void> {
    await this.patch(itemKeys.map((entry_ref) => ({ remove_entry: { entry_ref } })))
  }

  async reorder(itemKeys: string[], toIndex: number): Promise<void> {
    await this.patch([{ move_entries: { entry_refs: itemKeys, to_index: toIndex } }])
  }

  async createGroup(name: string, position?: number): Promise<void> {
    if (this.groupPath.length > 0) {
      // v1 collection_patch creates groups at the collection root only.
      throw nfspToUiError({ code: 'UNSUPPORTED', message: 'nested groups not supported' })
    }
    await this.patch([{ create_group: { name, position } }])
  }

  async renameGroup(itemKey: string, name: string): Promise<void> {
    await this.patch([{ rename_group: { entry_ref: itemKey, name } }])
  }
}

export function registerNfspCollectionReader() {
  registerReaderProvider({
    scheme: 'collection',
    create: (url) => new NfspCollectionReader(url),
  })
}
