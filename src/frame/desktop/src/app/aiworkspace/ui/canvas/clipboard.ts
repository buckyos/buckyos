/* Canvas object clipboard (UI improvement §6.4). First version: Blocks and groups within one Workspace.
 *
 *   copy → paste   new Block / group identities, the layout and view configuration copied, data Blocks keep
 *                  referencing the same data ("copy the view, share the data"); one commit, one undo step
 *   cut  → paste   cut only marks the objects; paste moves them in one commit, identities and data
 *                  references unchanged; a refused paste leaves them where they were
 *
 * The clipboard is per window and in memory; it never touches the system clipboard, annotations, run
 * history or data snapshots. */

import { randomId } from '../../api/ids'
import type { Json, Operation, Placement } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { Emitter } from '../../state/emitter'
import { boundsOf, topLevel, type Laid } from './layout'
import type { Rect } from './render/camera'

interface ClipItem { id: string; isGroup: boolean; placement: Placement; payload: Record<string, Json>; children: ClipItem[] }

export interface Clip {
  workspaceId: string
  surfaceId: string
  kind: 'copy' | 'cut'
  ids: string[]
  items: ClipItem[]
  bounds: Rect
  pastes: number
}

class CanvasClipboard {
  private clip: Clip | null = null
  private readonly emitter = new Emitter()
  readonly subscribe = this.emitter.subscribe
  snapshot = (): Clip | null => this.clip

  set(clip: Clip | null) {
    this.clip = clip
    this.emitter.emit()
  }

  notePaste() {
    if (this.clip) this.clip = { ...this.clip, pastes: this.clip.pastes + 1 }
    this.emitter.emit()
  }
}

export const canvasClipboard = new CanvasClipboard()

async function snapshotItem(store: WorkspaceStore, id: string, placement: Placement): Promise<ClipItem | null> {
  const entity = store.outline.get(id)
  if (!entity || entity.deleted) return null
  const read = await store.session.read<{ payload: Record<string, Json> }>(id)
  const isGroup = entity.type_id === 'buckyos.container'
  const children: ClipItem[] = []
  if (isGroup) {
    for (const child of store.outline.childrenOf(id)) {
      if (child.type_id !== 'buckyos.cell' && child.kind !== 'group') continue
      const item = await snapshotItem(store, child.entity_id, child.placement ?? { x: 0, y: 0, w: 320, h: 200 })
      if (item) children.push(item)
    }
  }
  return { id, isGroup, placement, payload: { ...read.content.payload }, children }
}

/** Put the top-level selection on the clipboard. Copy snapshots the view payloads now. */
export async function copyToClipboard(store: WorkspaceStore, surfaceId: string, laid: Map<string, Laid>, selection: ReadonlySet<string>, kind: 'copy' | 'cut'): Promise<number> {
  const ids = topLevel(laid, new Set(selection))
  const bounds = boundsOf(laid, ids)
  if (ids.length === 0 || !bounds) return 0
  const items: ClipItem[] = []
  if (kind === 'copy') {
    for (const id of ids) {
      const l = laid.get(id)
      if (!l) continue
      const item = await snapshotItem(store, id, { x: Math.round(l.rect.x), y: Math.round(l.rect.y), w: Math.round(l.rect.w), h: Math.round(l.rect.h) })
      if (item) items.push(item)
    }
  } else {
    for (const id of ids) {
      const l = laid.get(id)
      if (l) items.push({ id, isGroup: l.isGroup, placement: { x: Math.round(l.rect.x), y: Math.round(l.rect.y), w: Math.round(l.rect.w), h: Math.round(l.rect.h) }, payload: {}, children: [] })
    }
  }
  canvasClipboard.set({ workspaceId: store.session.workspaceId, surfaceId, kind, ids, items, bounds, pastes: 0 })
  return items.length
}

export interface PasteTarget {
  surfaceId: string
  isFree: boolean
  /** World point where the clip's top-left goes (right-click paste), or null for the visible centre. */
  at: { x: number; y: number } | null
  /** Centre of the unobstructed visible area, in world coordinates. */
  center: { x: number; y: number }
}

/** The operations of one paste (one commit); `newIds` are the top-level identities to select afterwards. */
export function pasteOperations(store: WorkspaceStore, clip: Clip, target: PasteTarget): { operations: Operation[]; newIds: string[]; label: string } {
  const step = clip.kind === 'copy' ? 24 * (clip.pastes + 1) : 0
  const anchor = target.at ?? { x: target.center.x - clip.bounds.w / 2 + step, y: target.center.y - clip.bounds.h / 2 + step }
  const dx = Math.round(anchor.x - clip.bounds.x)
  const dy = Math.round(anchor.y - clip.bounds.y)
  const operations: Operation[] = []
  const newIds: string[] = []
  let key = store.outline.childrenOf(target.surfaceId).at(-1)?.order_key ?? undefined
  const nextKey = () => { key = store.core.order_key_between(key, undefined); return key }
  if (clip.kind === 'cut') {
    for (const item of clip.items) {
      const entity = store.outline.get(item.id)
      if (!entity || entity.deleted) continue
      const placement = { ...item.placement, x: item.placement.x + dx, y: item.placement.y + dy }
      const sameParent = entity.parent_id === target.surfaceId
      if (target.isFree) store.noteLayoutIntent(item.id, placement, sameParent ? undefined : key ?? undefined, sameParent ? undefined : target.surfaceId)
      operations.push(sameParent
        ? { op: 'tree.place', entity_id: item.id, ...(target.isFree ? { placement } : {}) }
        : { op: 'tree.move', entity_id: item.id, new_parent_id: target.surfaceId, order_key: nextKey(), ...(target.isFree ? { placement } : {}) })
      newIds.push(item.id)
    }
    return { operations, newIds, label: `移动 ${newIds.length} 个对象` }
  }
  const create = (item: ClipItem, parentId: string, orderKey: string, placement: Placement | null): string => {
    const id = randomId(item.isGroup ? 'g' : 'c')
    operations.push({ op: 'entity.create', entity_id: id, type_id: item.isGroup ? 'buckyos.container' : 'buckyos.cell', parent_id: parentId, order_key: orderKey, ...(placement ? { placement } : {}), payload: item.payload })
    let childKey: string | undefined
    for (const child of item.children) {
      childKey = store.core.order_key_between(childKey, undefined)
      create(child, id, childKey, child.placement)
    }
    return id
  }
  for (const item of clip.items) {
    const placement = { ...item.placement, x: item.placement.x + dx, y: item.placement.y + dy }
    newIds.push(create(item, target.surfaceId, nextKey(), target.isFree ? placement : null))
  }
  return { operations, newIds, label: `粘贴 ${newIds.length} 个对象` }
}
