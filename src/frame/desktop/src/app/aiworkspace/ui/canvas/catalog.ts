/* The insert catalog's entries (UI improvement §6.3, §7.1): built from the BlockRegistry (definitions
 * that carry `catalog` information) and from the workspace's readable `buckyos.block-def` entities. The
 * menus, the object toolbar, the context menu and the catalog dialog all read this one list. */

import type { EntityEnvelope, Json } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { blockRegistry, type BlockDefinition, type CatalogGroup, type CatalogInfo } from '../blocks/registry'
import { MOCK_WISH_DEF_ID } from '../wish/mockWishDef'

export interface CatalogEntry {
  /** `block:<type>` for a registered definition, `def:<entity id>` for a workspace definition. */
  key: string
  definition: BlockDefinition
  catalog: CatalogInfo
  title: string
  group: CatalogGroup
  /** The workspace definition entity of an extension entry. */
  defEntity?: EntityEnvelope
}

export const STANDARD_ORDER = ['richtext', 'note', 'shape', 'frame', 'table', 'asset', 'wish']

/** Registered definitions offered for insertion; the HTML / declarative hosts are reached through workspace definitions. */
export function registryEntries(): CatalogEntry[] {
  const out: CatalogEntry[] = []
  for (const definition of blockRegistry.list()) {
    const catalog = definition.catalog
    if (!catalog || !definition.create || catalog.needs === 'definition') continue
    if (out.some((entry) => entry.definition.type === definition.type)) continue
    const latest = blockRegistry.get(definition.type) ?? definition
    out.push({ key: `block:${latest.type}`, definition: latest, catalog, title: latest.title, group: catalog.group })
  }
  const rank = (entry: CatalogEntry) => { const i = STANDARD_ORDER.indexOf(entry.definition.type); return i < 0 ? 100 : i }
  return out.sort((a, b) => rank(a) - rank(b) || a.title.localeCompare(b.title))
}

/** The workspace's Block definitions (extensions), as insertable entries. */
export function extensionEntries(store: WorkspaceStore): CatalogEntry[] {
  const out: CatalogEntry[] = []
  for (const entity of store.outline.all()) {
    // the Mock wish executor's definition drives wishes; it is not a view to place on a canvas
    if (entity.deleted || entity.type_id !== 'buckyos.block-def' || entity.entity_id === MOCK_WISH_DEF_ID) continue
    const host = entity.def_kind ? blockRegistry.get(entity.def_kind) : undefined
    if (!host?.catalog || !host.create) continue
    out.push({ key: `def:${entity.entity_id}`, definition: host, catalog: host.catalog, title: entity.title ?? entity.name ?? entity.entity_id, group: 'extension', defEntity: entity })
  }
  return out.sort((a, b) => a.title.localeCompare(b.title))
}

export function allEntries(store: WorkspaceStore): CatalogEntry[] {
  return [...registryEntries(), ...extensionEntries(store)]
}

/** What one insertion carries besides the definition. */
export interface InsertRequest {
  entry: CatalogEntry
  title?: string
  /** Existing data the new Block shows (needs `data`, or an extension that binds data). */
  sourceId?: string
  /** The file to upload first (needs `file`). */
  file?: File
  config?: Record<string, Json>
}

/** Data entities of the data tree a definition can show (for "needs data" and "add existing data"). */
export function dataFor(store: WorkspaceStore, accepts: string[] | null): EntityEnvelope[] {
  return store.outline.all().filter((e) => !e.deleted && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.container' && e.type_id !== 'buckyos.block-def'
    && (!accepts || accepts.includes(e.type_id)) && store.outline.ancestors(e.entity_id).includes('data'))
}

/** Why an entry cannot be inserted without a further step (its needs), or null when it can be inserted at once. */
export function pendingNeed(entry: CatalogEntry): 'data' | 'file' | null {
  if (entry.catalog.needs === 'data') return 'data'
  if (entry.catalog.needs === 'file') return 'file'
  return null
}
