/* The insert catalog's entries (UI improvement §6.3, §7.1): built from the BlockRegistry (definitions
 * that carry `catalog` information) and from the workspace's readable `buckyos.block-def` entities. The
 * menus, the object toolbar, the context menu and the catalog dialog all read this one list. */

import type { EntityEnvelope, Json } from '../../api/types'
import { aiwsText } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { blockRegistry, type BlockDefinition, type CatalogGroup, type CatalogInfo, type CatalogVariant } from '../blocks/registry'
import { MOCK_WISH_DEF_ID } from '../wish/mockWishDef'
import type { MediaFacts } from '../media/poster'

export interface CatalogEntry {
  /** `block:<type>` for a registered definition (`block:<type>:<variant>` for a variant), `def:<entity id>` for a workspace definition. */
  key: string
  /** A variant's id (its preset `config` and `size` below). */
  variant?: string
  config?: Record<string, Json>
  size?: { w: number; h: number }
  options?: CatalogVariant['options']
  definition: BlockDefinition
  catalog: CatalogInfo
  title: string
  group: CatalogGroup
  /** The workspace definition entity of an extension entry. */
  defEntity?: EntityEnvelope
}

export const STANDARD_ORDER = ['richtext', 'shape:text', 'note', 'shape', 'connector', 'frame', 'table', 'asset', 'wish']

/** An entry's id in test ids and menu ids: the type, or `type-variant`. */
export const entryId = (entry: CatalogEntry) => (entry.variant ? `${entry.definition.type}-${entry.variant}` : entry.definition.type)
/** The size an insertion of the entry takes. */
export const entrySize = (entry: CatalogEntry) => entry.size ?? entry.definition.defaultSize

/** Registered definitions offered for insertion; the HTML / declarative hosts are reached through workspace definitions. */
export function registryEntries(): CatalogEntry[] {
  const out: CatalogEntry[] = []
  for (const definition of blockRegistry.list()) {
    const catalog = definition.catalog
    if (!catalog || !definition.create || catalog.needs === 'definition') continue
    if (out.some((entry) => entry.definition.type === definition.type)) continue
    const latest = blockRegistry.get(definition.type) ?? definition
    out.push({ key: `block:${latest.type}`, definition: latest, catalog, title: latest.title, group: catalog.group })
    for (const v of catalog.variants ?? []) {
      out.push({
        key: `block:${latest.type}:${v.id}`, variant: v.id, config: v.config, size: v.size, options: v.options, definition: latest, title: v.title, group: v.group,
        catalog: { group: v.group, description: v.description, needs: 'none', editAfterInsert: v.editAfterInsert, standard: v.standard },
      })
    }
  }
  const rank = (entry: CatalogEntry) => { const i = STANDARD_ORDER.indexOf(entry.variant ? `${entry.definition.type}:${entry.variant}` : entry.definition.type); return i < 0 ? 100 : i }
  const { compare } = aiwsText()
  return out.sort((a, b) => rank(a) - rank(b) || compare(a.title, b.title))
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
  const { compare } = aiwsText()
  return out.sort((a, b) => compare(a.title, b.title))
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
  /** What is already known of `file` (its poster, size, length; 多媒体对象 §3), else worked out on insertion. */
  facts?: MediaFacts
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
