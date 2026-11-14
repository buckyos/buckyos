/* The canvas tree (UI 框架 PRD §4.1.1, Q5, K1): canvas folders are real folders under `surfaces`, at any depth, and a
 * canvas's place is its entity path `/surfaces/<folder>/…/<canvas>`. The tree is shared document structure: creating,
 * renaming, moving and deleting folders are ordinary undoable Commits that need `structure`. What one person sees of
 * it (collapsed folders, the order by name, the recent canvases) is their own work state. */

import { randomId } from '../../api/ids'
import type { EntityEnvelope, Operation } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'

export const SURFACES_ROOT = 'surfaces'

/** User work state of the picker (按工作区的用户状态, PRD §12). */
export const RECENT_KEY = 'ui:surface-recent'
export const COLLAPSED_KEY = 'ui:surface-collapsed'
export const SORT_KEY = 'ui:surface-sort'
export type SortMode = 'manual' | 'name'

/** A canvas or a canvas folder (what the `surfaces` tree holds). */
export function isCanvasEntry(e: EntityEnvelope): boolean {
  return e.type_id === 'buckyos.container' && (e.kind === 'surface' || e.kind === 'folder')
}

export function isCanvasFolder(store: WorkspaceStore, e: EntityEnvelope | undefined): boolean {
  return Boolean(e && e.type_id === 'buckyos.container' && e.kind === 'folder' && store.outline.ancestors(e.entity_id).includes(SURFACES_ROOT))
}

export function entryLabel(e: EntityEnvelope): string {
  return e.title ?? e.name ?? e.entity_id
}

const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' })

/** The canvases and folders directly under `parentId`: in the shared order, or by name with folders first. */
export function entriesOf(store: WorkspaceStore, parentId: string, byName = false): EntityEnvelope[] {
  const list = store.outline.childrenOf(parentId).filter(isCanvasEntry)
  if (!byName) return list
  return list.sort((a, b) => (a.kind === b.kind ? collator.compare(entryLabel(a), entryLabel(b)) : a.kind === 'folder' ? -1 : 1))
}

export interface TreeRow { kind: 'folder' | 'surface'; entity: EntityEnvelope; depth: number; collapsed: boolean; childCount: number }

/** The tree as rows, depth first; a collapsed folder hides what is in it. */
export function treeRows(store: WorkspaceStore, collapsed: ReadonlySet<string>, byName: boolean): TreeRow[] {
  const out: TreeRow[] = []
  const walk = (parentId: string, depth: number) => {
    for (const e of entriesOf(store, parentId, byName)) {
      if (e.kind === 'folder') {
        const shut = collapsed.has(e.entity_id)
        out.push({ kind: 'folder', entity: e, depth, collapsed: shut, childCount: entriesOf(store, e.entity_id).length })
        if (!shut) walk(e.entity_id, depth + 1)
      } else out.push({ kind: 'surface', entity: e, depth, collapsed: false, childCount: 0 })
    }
  }
  walk(SURFACES_ROOT, 0)
  return out
}

/** Every canvas folder in tree order with its depth (move targets, the new canvas dialog). */
export function allFolders(store: WorkspaceStore): { entity: EntityEnvelope; depth: number }[] {
  const out: { entity: EntityEnvelope; depth: number }[] = []
  const walk = (parentId: string, depth: number) => {
    for (const e of entriesOf(store, parentId)) if (e.kind === 'folder') { out.push({ entity: e, depth }); walk(e.entity_id, depth + 1) }
  }
  walk(SURFACES_ROOT, 0)
  return out
}

/** The folders from the top down to the one holding `id` (not `id` itself). */
export function folderTrail(store: WorkspaceStore, id: string): EntityEnvelope[] {
  const ids = store.outline.ancestors(id).slice(1)
  const cut = ids.indexOf(SURFACES_ROOT)
  if (cut < 0) return []
  return ids.slice(0, cut).reverse().map((x) => store.outline.get(x)).filter((e): e is EntityEnvelope => Boolean(e))
}

/** `/surfaces/<name>/…/<name>`: the path the core API, Agents and permission rules use (`~<id>` for an unnamed step). */
export function entityPath(store: WorkspaceStore, id: string): string {
  const ids = store.outline.ancestors(id)
  const cut = ids.indexOf(SURFACES_ROOT)
  const steps = (cut < 0 ? ids : ids.slice(0, cut)).reverse().map((x) => store.outline.get(x)?.name ?? `~${x}`)
  return `/${SURFACES_ROOT}/${steps.join('/')}`
}

export interface SearchHit { entity: EntityEnvelope; trail: string }

/** Canvases and folders whose title or name contains `query` (no case), across the whole tree whatever is collapsed:
 * flat, canvases first (Enter opens the first one), each with the folders it is in. */
export function searchEntries(store: WorkspaceStore, query: string, byName: boolean): SearchHit[] {
  const q = query.trim().toLowerCase()
  if (!q) return []
  const surfaces: SearchHit[] = []
  const folders: SearchHit[] = []
  const walk = (parentId: string, trail: string[]) => {
    for (const e of entriesOf(store, parentId, byName)) {
      const hit = entryLabel(e).toLowerCase().includes(q) || (e.name ?? '').toLowerCase().includes(q)
      if (hit) (e.kind === 'folder' ? folders : surfaces).push({ entity: e, trail: trail.join(' / ') })
      if (e.kind === 'folder') walk(e.entity_id, [...trail, entryLabel(e)])
    }
  }
  walk(SURFACES_ROOT, [])
  return [...surfaces, ...folders]
}

/** A new order key under `parentId`: at the end, or just before / after a sibling. */
function orderKeyIn(store: WorkspaceStore, parentId: string, movingId: string | null, at: { before?: string; after?: string } | null): string {
  const siblings = store.outline.childrenOf(parentId).filter((e) => e.entity_id !== movingId)
  const index = at?.before ? siblings.findIndex((e) => e.entity_id === at.before) : at?.after ? siblings.findIndex((e) => e.entity_id === at.after) + 1 : -1
  if (index < 0 || index >= siblings.length) return store.core.order_key_between(siblings.at(-1)?.order_key ?? undefined, undefined)
  return store.core.order_key_between(siblings[index - 1]?.order_key ?? undefined, siblings[index].order_key ?? undefined)
}

/** Move a canvas or a folder into `parentId` (the end, or next to a sibling there). */
export function moveEntryOps(store: WorkspaceStore, id: string, parentId: string, at: { before?: string; after?: string } | null = null): Operation[] {
  return [{ op: 'tree.move', entity_id: id, new_parent_id: parentId, order_key: orderKeyIn(store, parentId, id, at) }]
}

/** A new canvas folder at the end of `parentId`; its name follows the title (the core gives `-2`, `-3` on a clash). */
export function createFolderOps(store: WorkspaceStore, title: string, parentId: string): { ops: Operation[]; folderId: string } {
  const folderId = randomId('sd')
  return { folderId, ops: [{ op: 'entity.create', entity_id: folderId, type_id: 'buckyos.container', parent_id: parentId, order_key: orderKeyIn(store, parentId, null, null), payload: { kind: 'folder', title } }] }
}

/** The order key of a new entry right after `afterId` in its folder (a canvas's copy goes next to it). */
export function orderKeyAfter(store: WorkspaceStore, parentId: string, afterId: string | null): string {
  return orderKeyIn(store, parentId, null, afterId ? { after: afterId } : null)
}

/** May `id` (a canvas or a folder) move into `parentId`: not into itself or below itself. */
export function canMoveInto(store: WorkspaceStore, id: string, parentId: string): boolean {
  return parentId !== id && !store.outline.ancestors(parentId).includes(id)
}

/** Asks the shell to open the permission view of one entity (按实体的权限视图, PRD §8.4): a cancelable window event
 * `aiws:entity-permissions` with `{ entityId }`; true when a listener took it (called `preventDefault`). */
export const ENTITY_PERMISSIONS_EVENT = 'aiws:entity-permissions'

export function requestEntityPermissions(entityId: string): boolean {
  return !window.dispatchEvent(new CustomEvent(ENTITY_PERMISSIONS_EVENT, { detail: { entityId }, cancelable: true }))
}
