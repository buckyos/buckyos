/* The table a data control shows (控件机制 TODO G2): its standard table (`source.schemaId`). Inserted or found unbound, the
 * control takes the workspace's only such table, asks which one when there are several (or a new one), and creates one
 * when there is none — in the canvas's content folder, with the binding in the same Commit. */

import { standardTableOp } from 'buckyos'
import { uniqueName } from '@table/operations'
import type { EntityEnvelope, Operation } from '../../../api/types'
import { currentAiwsLocale } from '../../../i18n'
import type { WorkspaceStore } from '../../../state/store'
import type { DataSource } from '../registry'

/** The workspace's tables of `schemaId`, in tree order. */
export function sourceTables(store: WorkspaceStore, schemaId: string): EntityEnvelope[] {
  return store.outline.all().filter((e) => !e.deleted && e.type_id === 'buckyos.table-source' && e.schema_id === schemaId)
}

/** Where a table lives, for people: its canvas, else its folder. */
export function placeOf(store: WorkspaceStore, table: EntityEnvelope): string {
  const parent = table.parent_id ? store.outline.get(table.parent_id) : undefined
  if (!parent) return ''
  const surface = parent.system === 'surface_content' && parent.surface_id ? store.outline.get(parent.surface_id) : undefined
  return (surface ?? parent).title ?? (surface ?? parent).name ?? ''
}

/** A table name free in `folderId` (names are unique among siblings): `base`, else `base (2)` … */
export function freeTableName(store: WorkspaceStore, folderId: string, base: string): string {
  return uniqueName(base, new Set(store.outline.childrenOf(folderId).filter((e) => !e.deleted).map((e) => e.name).filter((n): n is string => Boolean(n))))
}

/** A new standard table for `source` in `folderId`, after the folder's last child. */
export function newSourceOps(store: WorkspaceStore, source: DataSource, folderId: string, tableId: string, name: string): Operation[] {
  const orderKey = store.core.order_key_between(store.outline.childrenOf(folderId).at(-1)?.order_key ?? undefined, undefined)
  return [standardTableOp(source.table, { entity_id: tableId, parent_id: folderId, order_key: orderKey, name: freeTableName(store, folderId, name), locale: currentAiwsLocale().startsWith('zh') ? 'zh' : 'en' }) as unknown as Operation]
}

/** The content folder of the canvas a Block or container is on. */
export function surfaceFolderOf(store: WorkspaceStore, id: string): string | null {
  let at = store.outline.get(id)
  for (let i = 0; at && at.kind !== 'surface' && i < 64; i++) at = at.parent_id ? store.outline.get(at.parent_id) : undefined
  return at?.content_folder_id ?? null
}

/** Bind (an id) or unbind (null) named bindings of a control: one `entity.set_props` on its `bindings`. */
export function bindOps(containerId: string, bindings: Record<string, string | null>): Operation[] {
  const props = Object.fromEntries(Object.entries(bindings).map(([name, id]) => [name, id === null ? null : { entity_id: id }]))
  return [{ op: 'entity.set_props', entity_id: containerId, key: 'bindings', props } as unknown as Operation]
}

/** A binding name not yet used: `base` when free, else `<base>_2`, `<base>_3` … */
export function freeBindingName(bindings: Record<string, string>, base: string): string {
  if (!(base in bindings)) return base
  for (let n = 2; ; n++) if (!(`${base}_${n}` in bindings)) return `${base}_${n}`
}
