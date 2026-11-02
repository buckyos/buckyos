/* Canvas object clipboard (UI improvement §6.4). First version: Blocks and groups within one Workspace.
 *
 *   copy → paste   new Block / group identities, the layout and view configuration copied; a Block's own data
 *                  (created with it on the canvas, not declared shared) is cloned through `entity.clone` when
 *                  pasted on the same canvas and shared when pasted elsewhere (内核架构改进 D05); shared data
 *                  of the data tree is always shared; one commit, one undo step
 *   cut  → paste   cut only marks the objects; paste moves them in one commit, identities and data
 *                  references unchanged; a refused paste leaves them where they were
 *   connectors     (连接线实现方案 §8.1) a copied line keeps only the bindings to objects copied with it, rebound to
 *                  their copies; its other ends become coordinates where they were when copied. Objects are created
 *                  before lines. A flow page takes no lines.
 *   mind maps      (脑图方案 §7.2, R9) a node is copied with its subtree and its "auto" placement. Pasted while a node is
 *                  selected it hangs under that node (a cut moves it there); pasted onto the canvas, nodes become the
 *                  roots of a new mind map (Blocks never live loose with children); a whole control is placed by its
 *                  origin. A Block pasted under a node becomes a node, laid out by the control (a non-text one keeps its
 *                  size). Lines stay on the canvas.
 *   composites     (扩展控件 P1) a copied instance is pasted with `tree.clone` while its source exists in this Workspace
 *                  (the core copies the subtree and the own data, the definition stays shared); its Blocks pasted onto
 *                  the canvas are ordinary Blocks (no mind map around them); a Block pasted into a composite takes the
 *                  slot of the Block it follows, else the first slot that accepts it (none: refused).
 *
 * The clipboard is per window and in memory; it never touches the system clipboard, annotations, run
 * history or data snapshots. */

import { isControlKind, isTextView } from '@controls/contract'
import { num } from '@controls/placement'
import { randomId } from '../../api/ids'
import type { Json, Operation, StoredPlacement } from '../../api/types'
import { aiwsT } from '../../i18n'
import type { WorkspaceStore } from '../../state/store'
import { Emitter } from '../../state/emitter'
import { boxOf } from './connectors/geometry'
import { isConnector } from './connectors/layout'
import type { Point } from './geometry'
import { isComposite, slotForDrop } from '../controls/composite/model'
import { isCanvasContainer } from '../controls/registry'
import { boundsOf, topLevel, type Laid } from './layout'
import type { Rect } from './render/camera'

/** `ends`: a line's two ends (world) when it was copied. `node`: it was laid out by a control (its placement keeps the
 * stored "auto" components; `controlConfig`: that control's config, for a new map around it). `kind`: a container's. */
interface ClipItem { id: string; isGroup: boolean; kind?: string; placement: StoredPlacement; payload: Record<string, Json>; children: ClipItem[]; ends?: [Point, Point]; node?: boolean; controlConfig?: Record<string, Json>
  /** The kind of the control laying it out (a tree control wraps pasted nodes in a new map; a composite does not). */
  controlKind?: string
  /** A top-level item's placement as stored (its "auto" size components). */
  stored?: StoredPlacement }

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

async function snapshotItem(store: WorkspaceStore, laid: Map<string, Laid>, id: string, placement: StoredPlacement): Promise<ClipItem | null> {
  const entity = store.outline.get(id)
  if (!entity || entity.deleted) return null
  const read = await store.session.read<{ payload: Record<string, Json> }>(id)
  const isGroup = entity.type_id === 'buckyos.container'
  const children: ClipItem[] = []
  // a container's Blocks, and a node's children inside a control
  for (const child of store.outline.childrenOf(id)) {
    if (child.type_id !== 'buckyos.cell' && !isCanvasContainer(child)) continue
    const item = await snapshotItem(store, laid, child.entity_id, child.placement ?? { x: 0, y: 0, w: 320, h: 200 })
    if (item) children.push(item)
  }
  // a copy is never locked (标准对象的交互改进 §7.2)
  const { locked: _locked, ...payload } = read.content.payload
  void _locked
  const l = laid.get(id)
  const geom = l?.connector?.geom
  const control = l?.node ? store.outline.get(l.node.control) : undefined
  return { id, isGroup, ...(isGroup ? { kind: entity.kind } : {}), placement, payload, children, ...(geom ? { ends: [geom.start.point, geom.end.point] as [Point, Point] } : {}),
    ...(l?.node ? { node: true, controlKind: control?.kind ?? undefined, controlConfig: (control?.config ?? {}) as Record<string, Json> } : {}) }
}

const hasLines = (items: ClipItem[]): boolean => items.some((item) => item.ends !== undefined || hasLines(item.children))
const isInk = (payload: Record<string, Json>) => (payload.view as { type?: string } | undefined)?.type === 'ink'
const hasInk = (items: ClipItem[]): boolean => items.some((item) => isInk(item.payload) || hasInk(item.children))

/** The world placement of a laid-out object, rotation included; a control by its origin (its size stays "auto"). */
function worldPlacement(l: Laid): StoredPlacement {
  if (l.control) return { x: Math.round(l.control.origin.x), y: Math.round(l.control.origin.y), w: 'auto', h: 'auto' }
  return { x: Math.round(l.rect.x), y: Math.round(l.rect.y), w: Math.round(l.rect.w), h: Math.round(l.rect.h), ...(l.rotation ? { rotation: l.rotation } : {}) }
}

/** The clip of the top-level selection, without putting it on the clipboard (Ctrl+D duplicates through it). */
export async function makeClip(store: WorkspaceStore, surfaceId: string, laid: Map<string, Laid>, selection: ReadonlySet<string>, kind: 'copy' | 'cut'): Promise<Clip | null> {
  // a cut moves the objects: locked ones stay out of it
  const ids = topLevel(laid, new Set(selection)).filter((id) => kind === 'copy' || !laid.get(id)?.locked)
  const bounds = boundsOf(laid, ids)
  if (ids.length === 0 || !bounds) return null
  const items: ClipItem[] = []
  if (kind === 'copy') {
    for (const id of ids) {
      const l = laid.get(id)
      if (!l) continue
      const item = await snapshotItem(store, laid, id, worldPlacement(l))
      if (item) items.push({ ...item, ...(l.entity.placement ? { stored: l.entity.placement } : {}) })
    }
  } else {
    for (const id of ids) {
      const l = laid.get(id)
      // a cut remembers whether it carries lines (a flow page refuses them)
      const lines = l && (isConnector(l.entity) || store.outline.descendants(id).some((d) => isConnector(d)))
      const control = l?.node ? store.outline.get(l.node.control) : undefined
      if (l) items.push({ id, isGroup: l.isGroup, ...(l.isGroup ? { kind: l.entity.kind } : {}), placement: worldPlacement(l), payload: {}, children: [], ...(lines ? { ends: [{ x: 0, y: 0 }, { x: 0, y: 0 }] as [Point, Point] } : {}),
        ...(l.node ? { node: true, controlKind: control?.kind ?? undefined, controlConfig: (control?.config ?? {}) as Record<string, Json> } : {}) })
    }
  }
  return { workspaceId: store.session.workspaceId, surfaceId, kind, ids, items, bounds, pastes: 0 }
}

/** Put the top-level selection on the clipboard. Copy snapshots the view payloads now. */
export async function copyToClipboard(store: WorkspaceStore, surfaceId: string, laid: Map<string, Laid>, selection: ReadonlySet<string>, kind: 'copy' | 'cut'): Promise<number> {
  const clip = await makeClip(store, surfaceId, laid, selection, kind)
  if (!clip) return 0
  canvasClipboard.set(clip)
  return clip.items.length
}

/** Does a Block size itself inside a control (a text view, a control)? Its pasted size then stays "auto". */
const sizesItself = (item: ClipItem) => (item.isGroup ? isControlKind(item.kind) : isTextView((item.payload.view as { type?: string } | undefined)?.type))
/** Where a Block goes inside a control: laid out ("auto" x / y), its size its own or the one it had. */
function inControl(item: ClipItem, stored?: StoredPlacement): StoredPlacement {
  if (sizesItself(item) && (stored === undefined || stored.w === 'auto' || stored.h === 'auto')) return { x: 'auto', y: 'auto', w: 'auto', h: 'auto' }
  return { x: 'auto', y: 'auto', w: num(stored?.w) ?? num(item.placement.w) ?? 160, h: num(stored?.h) ?? num(item.placement.h) ?? 90 }
}

export interface PasteTarget {
  surfaceId: string
  isFree: boolean
  /** World point where the clip's top-left goes (right-click paste), or null for the visible centre. */
  at: { x: number; y: number } | null
  /** Centre of the unobstructed visible area, in world coordinates. */
  center: { x: number; y: number }
  /** Inside a control (脑图方案 §7.2): the node (or control) the clip hangs under, after its child `after` (end when absent). */
  into?: { parentId: string; after?: string | null }
}

/** The operations of one paste (one commit); `newIds` are the top-level identities to select afterwards;
 * `refused` says why nothing is pasted. */
export function pasteOperations(store: WorkspaceStore, clip: Clip, target: PasteTarget): { operations: Operation[]; newIds: string[]; label: string; refused?: string } {
  if (!target.isFree && hasLines(clip.items)) return { operations: [], newIds: [], label: '', refused: aiwsT('aiworkspace.canvas.clipboard.linesOnFlowPage') }
  // ink is written on free canvases (a flow page's handwriting is a later extension)
  if (!target.isFree && (clip.kind === 'copy' ? hasInk(clip.items) : clip.items.some((item) => store.outline.get(item.id)?.view_type === 'ink' || store.outline.descendants(item.id).some((d) => d.view_type === 'ink')))) {
    return { operations: [], newIds: [], label: '', refused: aiwsT('aiworkspace.canvas.clipboard.inkOnFlowPage') }
  }
  const into = target.into
  // a control lays out Blocks, not lines (连接线方案 R7: lines are drawn on the canvas)
  if (into && hasLines(clip.items)) return { operations: [], newIds: [], label: '', refused: aiwsT('aiworkspace.mindmap.noLinesInside') }
  if (into && clip.kind === 'cut' && clip.items.some((item) => item.id === into.parentId || store.outline.ancestors(into.parentId).includes(item.id))) {
    return { operations: [], newIds: [], label: '', refused: aiwsT('aiworkspace.mindmap.notIntoItself') }
  }
  const step = clip.kind === 'copy' ? 24 * (clip.pastes + 1) : 0
  const anchor = target.at ?? { x: target.center.x - clip.bounds.w / 2 + step, y: target.center.y - clip.bounds.h / 2 + step }
  const dx = Math.round(anchor.x - clip.bounds.x)
  const dy = Math.round(anchor.y - clip.bounds.y)
  const shifted = (p: StoredPlacement): StoredPlacement => ({ ...p, x: (num(p.x) ?? 0) + dx, y: (num(p.y) ?? 0) + dy })
  const operations: Operation[] = []
  const newIds: string[] = []
  let key = store.outline.childrenOf(target.surfaceId).at(-1)?.order_key ?? undefined
  const nextKey = () => { key = store.core.order_key_between(key, undefined); return key }
  // inside a control: keys right after `after` (or at the end) among the parent's children
  const intoKeys: string[] = []
  if (into) {
    const siblings = store.outline.childrenOf(into.parentId).filter((c) => !clip.items.some((item) => item.id === c.entity_id))
    const at = into.after ? siblings.findIndex((c) => c.entity_id === into.after) : siblings.length - 1
    let low = siblings[at]?.order_key ?? undefined
    const high = siblings[at + 1]?.order_key ?? undefined
    for (let i = 0; i < clip.items.length; i++) { low = store.core.order_key_between(low, high); intoKeys.push(low) }
  }
  // a Block pasted into a composite fills a slot: the one of the Block it follows, else the first that takes it
  const slots: (string | null)[] = []
  if (into && isComposite(store.outline.get(into.parentId))) {
    const moving = new Set(clip.kind === 'cut' ? clip.items.map((item) => item.id) : [])
    for (const item of clip.items) {
      const view = (item.payload.view as { type?: string } | undefined)?.type ?? store.outline.get(item.id)?.view_type ?? null
      const block = { type_id: item.isGroup ? 'buckyos.container' : 'buckyos.cell', view_type: view, def_id: (item.payload.def_ref as { entity_id?: string } | undefined)?.entity_id ?? store.outline.get(item.id)?.def_id ?? null }
      const slot = slotForDrop(store, into.parentId, block, into.after ?? null, moving)
      if (!slot) return { operations: [], newIds: [], label: '', refused: aiwsT('aiworkspace.ext.composite.slotRefuses') }
      slots.push(slot)
    }
  }
  // nodes of a tree control pasted onto the canvas become the roots of a new mind map (a Block holding Blocks lives
  // only in such a control); a composite's Blocks are ordinary Blocks on their own
  const wrap = !into && clip.items.some((item) => item.node && item.controlKind === 'mindmap')
  const mapId = wrap ? randomId('m') : null
  let mapKey: string | undefined
  if (wrap && mapId) {
    const first = clip.items.find((item) => item.node && item.controlKind === 'mindmap')!
    const origin = shifted(first.placement)
    operations.push({ op: 'entity.create', entity_id: mapId, type_id: 'buckyos.container', parent_id: target.surfaceId, order_key: nextKey(),
      ...(target.isFree ? { placement: { x: origin.x, y: origin.y, w: 'auto', h: 'auto' } } : {}),
      payload: { kind: 'mindmap', layout: { mode: 'tree' }, ...(typeof first.payload.title === 'string' ? { title: first.payload.title } : {}), config: first.controlConfig ?? {} } })
  }
  /** Where a top-level item goes: under the node pasted into, into the new map, or onto the canvas. */
  const destination = (item: ClipItem, index: number, stored: StoredPlacement | undefined): { parent: string; key: string; placement: StoredPlacement | null } => {
    if (into) return { parent: into.parentId, key: intoKeys[index], placement: inControl(item, stored) }
    if (mapId && item.node && item.controlKind === 'mindmap') { mapKey = store.core.order_key_between(mapKey, undefined); return { parent: mapId, key: mapKey, placement: inControl(item, stored) } }
    return { parent: target.surfaceId, key: nextKey(), placement: target.isFree ? shifted(item.placement) : null }
  }
  if (clip.kind === 'cut') {
    clip.items.forEach((item, index) => {
      const entity = store.outline.get(item.id)
      if (!entity || entity.deleted) return
      const d = destination(item, index, entity.placement)
      const sameParent = entity.parent_id === d.parent
      if (target.isFree && d.placement) store.noteLayoutIntent(item.id, d.placement, sameParent ? undefined : d.key, sameParent ? undefined : d.parent)
      // into a composite a Block takes its slot; out of one into anything else it drops it
      const slot = slots.length > 0 ? slots[index] : entity.slot && isComposite(store.outline.get(entity.parent_id ?? '')) && !into ? null : undefined
      operations.push(sameParent && !into && slot === undefined
        ? { op: 'tree.place', entity_id: item.id, ...(d.placement ? { placement: d.placement } : {}) }
        : { op: 'tree.move', entity_id: item.id, new_parent_id: d.parent, order_key: d.key, ...(d.placement ? { placement: d.placement } : {}), ...(slot !== undefined ? { slot } : {}) })
      newIds.push(item.id)
    })
    return { operations, newIds, label: aiwsT('aiworkspace.canvas.clipboard.moveLabel', { count: newIds.length }) }
  }
  // every identity first, so a line can be rebound to the copy of an object it was copied with
  const fresh = new Map<string, string>()
  const allocate = (item: ClipItem) => { fresh.set(item.id, randomId(item.isGroup ? 'g' : 'c')); item.children.forEach(allocate) }
  clip.items.forEach(allocate)
  const lines: Operation[] = []
  // a copied stroke is a stroke of its own: its segments get one new logical id, so erasing one never takes the other
  const strokes = new Map<string, string>()
  const freshInk = (payload: Record<string, Json>): Record<string, Json> => {
    const config = payload.config as Record<string, Json> | undefined
    const stroke = typeof config?.stroke === 'string' ? config.stroke : null
    if (!isInk(payload) || !config || !stroke) return payload
    if (!strokes.has(stroke)) strokes.set(stroke, randomId('k'))
    return { ...payload, config: { ...config, stroke: strokes.get(stroke)! } }
  }
  // own data of a Block copied onto the same canvas is cloned (D05); the planner then makes the copy own its clone
  const sameCanvas = clip.workspaceId === store.session.workspaceId && clip.surfaceId === target.surfaceId
  const cloned = new Map<string, string>()
  const ownData = (payload: Record<string, Json>): string | null => {
    if (!sameCanvas || payload.data_ownership === 'shared') return null
    const id = (payload.source_ref as { entity_id?: string } | undefined)?.entity_id
    const source = id ? store.outline.get(id) : undefined
    if (!source || source.deleted || !source.parent_id || !store.outline.ancestors(source.entity_id).includes('canvas-content')) return null
    if (cloned.has(source.entity_id)) return cloned.get(source.entity_id) ?? null
    const copy = randomId('d')
    operations.push({ op: 'entity.clone', entity_id: source.entity_id, new_entity_id: copy, parent_id: source.parent_id, order_key: store.core.order_key_between(store.outline.childrenOf(source.parent_id).at(-1)?.order_key, undefined), ...(source.name ? { name: `${source.name} ${clip.pastes + 2}` } : {}) })
    cloned.set(source.entity_id, copy)
    return copy
  }
  /** `origin`: the parent's world position after the paste (lines are placed from their world ends). */
  const create = (item: ClipItem, parentId: string, orderKey: string, placement: StoredPlacement | null, origin: Point, top = false) => {
    const id = fresh.get(item.id) ?? randomId('c')
    if (item.ends) {
      const rebind = (end: Json | undefined): Json => {
        const b = end as { entity_id?: string; anchor?: Json } | null | undefined
        const copy = b?.entity_id ? fresh.get(b.entity_id) : undefined
        return copy && b?.anchor ? { entity_id: copy, anchor: b.anchor } : null
      }
      const { rect, flip } = boxOf({ x: item.ends[0].x + dx, y: item.ends[0].y + dy }, { x: item.ends[1].x + dx, y: item.ends[1].y + dy })
      const { flip: _flip, ...rest } = item.payload
      void _flip
      const payload: Record<string, Json> = { ...rest, start: rebind(item.payload.start), end: rebind(item.payload.end), ...(flip.h || flip.v ? { flip: { ...(flip.h ? { h: true } : {}), ...(flip.v ? { v: true } : {}) } } : {}) }
      lines.push({ op: 'entity.create', entity_id: id, type_id: 'buckyos.cell', parent_id: parentId, order_key: orderKey, placement: { x: rect.x - origin.x, y: rect.y - origin.y, w: rect.w, h: rect.h }, payload })
      return id
    }
    const own = item.isGroup ? null : ownData(item.payload)
    // a slot is a role among siblings (R22): kept inside a copied parent (a new namespace), dropped by the copy itself
    const { slot: _slot, ...base } = item.payload
    void _slot
    const kept = top ? base : item.payload
    const payload = own ? { ...kept, source_ref: { ...(kept.source_ref as Record<string, Json>), entity_id: own } } : freshInk(kept)
    operations.push({ op: 'entity.create', entity_id: id, type_id: item.isGroup ? 'buckyos.container' : 'buckyos.cell', parent_id: parentId, order_key: orderKey, ...(placement ? { placement } : {}), payload })
    const here = placement ? { x: origin.x + (num(placement.x) ?? 0), y: origin.y + (num(placement.y) ?? 0) } : origin
    let childKey: string | undefined
    for (const child of item.children) {
      childKey = store.core.order_key_between(childKey, undefined)
      create(child, id, childKey, child.placement, here)
    }
    return id
  }
  const seed = randomId('s')
  clip.items.forEach((item, index) => {
    const d = destination(item, index, item.stored)
    // a composite instance whose source is still here: the core copies it (Q17), sharing its definition
    const source = store.outline.get(item.id)
    if (item.isGroup && item.kind === 'composite' && clip.workspaceId === store.session.workspaceId && source && !source.deleted) {
      operations.push({ op: 'tree.clone', source_id: item.id, parent_id: d.parent, order_key: d.key, id_seed: seed, ...(d.placement ? { placement: d.placement } : {}), ...(slots.length > 0 ? { slot: slots[index] } : {}) })
      newIds.push(store.core.clone_id(seed, item.id))
      return
    }
    const id = create(item, d.parent, d.key, d.placement, { x: 0, y: 0 }, true)
    if (slots.length > 0) {
      const op = operations.find((o) => o.op === 'entity.create' && o.entity_id === id)
      if (op) op.payload = { ...(op.payload as Record<string, Json>), slot: slots[index] }
    }
    newIds.push(id)
  })
  // lines after every object they may be bound to (§7: a binding needs its target to exist)
  operations.push(...lines)
  return { operations, newIds, label: aiwsT('aiworkspace.canvas.clipboard.pasteLabel', { count: newIds.length }) }
}
