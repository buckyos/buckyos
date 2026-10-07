/* Canvas object clipboard (UI improvement §6.4). First version: Blocks and groups within one Workspace.
 *
 *   copy → paste   new Block / group identities, the layout and view configuration copied, data Blocks keep
 *                  referencing the same data ("copy the view, share the data"); one commit, one undo step
 *   cut  → paste   cut only marks the objects; paste moves them in one commit, identities and data
 *                  references unchanged; a refused paste leaves them where they were
 *   connectors     (连接线实现方案 §8.1) a copied line keeps only the bindings to objects copied with it, rebound to
 *                  their copies; its other ends become coordinates where they were when copied. Objects are created
 *                  before lines. A flow page takes no lines.
 *
 * The clipboard is per window and in memory; it never touches the system clipboard, annotations, run
 * history or data snapshots. */

import { randomId } from '../../api/ids'
import type { Json, Operation, Placement } from '../../api/types'
import type { WorkspaceStore } from '../../state/store'
import { Emitter } from '../../state/emitter'
import { boxOf } from './connectors/geometry'
import { isConnector } from './connectors/layout'
import type { Point } from './geometry'
import { boundsOf, topLevel, type Laid } from './layout'
import type { Rect } from './render/camera'

/** `ends`: a line's two ends (world) when it was copied. */
interface ClipItem { id: string; isGroup: boolean; placement: Placement; payload: Record<string, Json>; children: ClipItem[]; ends?: [Point, Point] }

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

async function snapshotItem(store: WorkspaceStore, laid: Map<string, Laid>, id: string, placement: Placement): Promise<ClipItem | null> {
  const entity = store.outline.get(id)
  if (!entity || entity.deleted) return null
  const read = await store.session.read<{ payload: Record<string, Json> }>(id)
  const isGroup = entity.type_id === 'buckyos.container'
  const children: ClipItem[] = []
  if (isGroup) {
    for (const child of store.outline.childrenOf(id)) {
      if (child.type_id !== 'buckyos.cell' && child.kind !== 'group') continue
      const item = await snapshotItem(store, laid, child.entity_id, child.placement ?? { x: 0, y: 0, w: 320, h: 200 })
      if (item) children.push(item)
    }
  }
  // a copy is never locked (标准对象的交互改进 §7.2)
  const { locked: _locked, ...payload } = read.content.payload
  void _locked
  const geom = laid.get(id)?.connector?.geom
  return { id, isGroup, placement, payload, children, ...(geom ? { ends: [geom.start.point, geom.end.point] as [Point, Point] } : {}) }
}

const hasLines = (items: ClipItem[]): boolean => items.some((item) => item.ends !== undefined || hasLines(item.children))

/** The world placement of a laid-out object, rotation included. */
function worldPlacement(l: Laid): Placement {
  return { x: Math.round(l.rect.x), y: Math.round(l.rect.y), w: Math.round(l.rect.w), h: Math.round(l.rect.h), ...(l.rotation ? { rotation: l.rotation } : {}) }
}

/** Put the top-level selection on the clipboard. Copy snapshots the view payloads now. */
export async function copyToClipboard(store: WorkspaceStore, surfaceId: string, laid: Map<string, Laid>, selection: ReadonlySet<string>, kind: 'copy' | 'cut'): Promise<number> {
  // a cut moves the objects: locked ones stay out of it
  const ids = topLevel(laid, new Set(selection)).filter((id) => kind === 'copy' || !laid.get(id)?.locked)
  const bounds = boundsOf(laid, ids)
  if (ids.length === 0 || !bounds) return 0
  const items: ClipItem[] = []
  if (kind === 'copy') {
    for (const id of ids) {
      const l = laid.get(id)
      if (!l) continue
      const item = await snapshotItem(store, laid, id, worldPlacement(l))
      if (item) items.push(item)
    }
  } else {
    for (const id of ids) {
      const l = laid.get(id)
      // a cut remembers whether it carries lines (a flow page refuses them)
      const lines = l && (isConnector(l.entity) || store.outline.descendants(id).some((d) => isConnector(d)))
      if (l) items.push({ id, isGroup: l.isGroup, placement: worldPlacement(l), payload: {}, children: [], ...(lines ? { ends: [{ x: 0, y: 0 }, { x: 0, y: 0 }] as [Point, Point] } : {}) })
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

/** The operations of one paste (one commit); `newIds` are the top-level identities to select afterwards;
 * `refused` says why nothing is pasted. */
export function pasteOperations(store: WorkspaceStore, clip: Clip, target: PasteTarget): { operations: Operation[]; newIds: string[]; label: string; refused?: string } {
  if (!target.isFree && hasLines(clip.items)) return { operations: [], newIds: [], label: '', refused: '流式页不显示连接线：剪贴板中有连接线（或含连接线的分组），没有粘贴。' }
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
  // every identity first, so a line can be rebound to the copy of an object it was copied with
  const fresh = new Map<string, string>()
  const allocate = (item: ClipItem) => { fresh.set(item.id, randomId(item.isGroup ? 'g' : 'c')); item.children.forEach(allocate) }
  clip.items.forEach(allocate)
  const lines: Operation[] = []
  /** `origin`: the parent's world position after the paste (lines are placed from their world ends). */
  const create = (item: ClipItem, parentId: string, orderKey: string, placement: Placement | null, origin: Point) => {
    const id = fresh.get(item.id) ?? randomId('c')
    if (item.ends) {
      const rebind = (end: Json | undefined): Json => {
        const b = end as { entity_id?: string; anchor?: Json } | null | undefined
        const copy = b?.entity_id ? fresh.get(b.entity_id) : undefined
        return copy ? { entity_id: copy, anchor: b?.anchor ?? { kind: 'auto' } } : null
      }
      const { rect, flip } = boxOf({ x: item.ends[0].x + dx, y: item.ends[0].y + dy }, { x: item.ends[1].x + dx, y: item.ends[1].y + dy })
      const { flip: _flip, ...rest } = item.payload
      void _flip
      const payload: Record<string, Json> = { ...rest, start: rebind(item.payload.start), end: rebind(item.payload.end), ...(flip.h || flip.v ? { flip: { ...(flip.h ? { h: true } : {}), ...(flip.v ? { v: true } : {}) } } : {}) }
      lines.push({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, order_key: orderKey, placement: { x: rect.x - origin.x, y: rect.y - origin.y, w: rect.w, h: rect.h }, payload })
      return id
    }
    operations.push({ op: 'entity.create', entity_id: id, type_id: item.isGroup ? 'buckyos.container' : 'buckyos.cell', parent_id: parentId, order_key: orderKey, ...(placement ? { placement } : {}), payload: item.payload })
    const here = placement ? { x: origin.x + placement.x, y: origin.y + placement.y } : origin
    let childKey: string | undefined
    for (const child of item.children) {
      childKey = store.core.order_key_between(childKey, undefined)
      create(child, id, childKey, child.placement, here)
    }
    return id
  }
  for (const item of clip.items) {
    const placement = { ...item.placement, x: item.placement.x + dx, y: item.placement.y + dy }
    newIds.push(create(item, target.surfaceId, nextKey(), target.isFree ? placement : null, { x: 0, y: 0 }))
  }
  // lines after every object they may be bound to (§7: a binding needs its target to exist)
  operations.push(...lines)
  return { operations, newIds, label: `粘贴 ${newIds.length} 个对象` }
}
