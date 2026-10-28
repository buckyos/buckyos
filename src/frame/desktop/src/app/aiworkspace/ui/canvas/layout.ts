/* Layout maths of a free Surface (phase two §8.2): child placements are relative to their container
 * (Surface or UI group); this turns the BlockTree into world rectangles, finds free spots for new
 * Blocks, and converts between parents when grouping or moving across containers.
 *
 * A Block-control (脑图方案 §5, §6.2) lays out its own Blocks: its named layout fills every "auto" component from the
 * outline alone (nested controls first, bottom-up, R24), its box wraps them, and its stored x / y name its origin (the
 * first root's top-left), not the box. Folded nodes (the reader's own state) and their subtrees are not laid out. A
 * container kind this client does not know is drawn as a group (R12). Connectors are routed last (R7). */

import { isControlKind, isTextView } from '@controls/contract'
import { num, type StoredPlacement } from '@controls/placement'
import { branchColors } from '@controls/edges'
import { placeControl, type PlacedControl, type PlacedNode } from '@controls/solve'
import type { Grow } from '@controls/tree'
import type { EntityEnvelope, Placement } from '../../api/types'
import type { OutlineModel } from '../../state/outline'
import { blockRegistry } from '../blocks/registry'
import { nodeMeasurer } from '../controls/measure'
import { controlRegistry, isCanvasContainer } from '../controls/registry'
import { isConnector, RECT_FORM, resolveConnectors, type LaidConnector, type TargetForm } from './connectors/layout'
import { normalizeRotation, rotatedBounds } from './geometry'
import { union, type Rect } from './render/camera'

/** A Block laid out by a control (脑图方案 §5): where it sits in the control's tree. */
export interface ControlNode {
  /** The control laying it out. */
  control: string
  depth: number
  grow: Grow
  /** Its x / y are numbers: a free topic (R16). */
  pinned: boolean
  hasChildren: boolean
  collapsed: boolean
  /** Its branch colour (none for a root). */
  color: string | null
  /** Its parent in the control's tree (null for a root). */
  parent: string | null
}

/** A control container: its kind, its origin (what its stored x / y name) and its solved layout. */
export interface ControlBox {
  kind: string
  origin: { x: number; y: number }
  placed: PlacedControl
}

/** `paint` is the stacking rank: BlockTree pre-order (a parent before its children, siblings by `order_key`),
 * higher is drawn on top. Painting and hit testing both follow it. `rect` is the unrotated layout rect and
 * `rotation` turns it about its centre; `bounds` is the axis-aligned box around the result (culling, marquee,
 * fit). Groups do not rotate yet (P2): their stored rotation is not drawn. `locked`: the Block or an
 * enclosing group is locked (标准对象的交互改进 §7.2). A connector (连接线实现方案 §9.2) has its stored box as
 * `rect` (what placement writes use; zero width or height allowed) and its routed path in `connector`, with
 * `bounds` around the path and label. */
export interface Laid { entity: EntityEnvelope; rect: Rect; rotation: number; bounds: Rect; depth: number; parentId: string; isGroup: boolean; paint: number; locked: boolean; connector?: LaidConnector
  /** A group's first child when it is a frame or shape covering the group (its look: a swimlane, a container,
   * draw.io 导入 TODO §2.2): pressing it selects and moves the group, resizing the group resizes it. */
  backingId?: string
  /** This Block is its group's backing. */
  backing?: boolean
  /** A control container (its `rect` is its box). */
  control?: ControlBox
  /** A Block laid out by a control. */
  node?: ControlNode
  /** A container kind this client does not know: drawn as a group (R12). */
  unknownKind?: string
  /** Where this Block's children are placed from, when not `rect`'s top-left (a control's origin). */
  origin?: { x: number; y: number } }

export const DEFAULT_PLACEMENT: Placement = { x: 0, y: 0, w: 320, h: 200 }

/** A stored placement with numbers everywhere: `"auto"` (only a control fills it) and missing parts take `fallback`. */
export function numericPlacement(p: Partial<StoredPlacement> | null | undefined, fallback: Placement = DEFAULT_PLACEMENT): Placement {
  return { x: num(p?.x) ?? fallback.x, y: num(p?.y) ?? fallback.y, w: num(p?.w) ?? fallback.w, h: num(p?.h) ?? fallback.h, ...(p?.rotation ? { rotation: p.rotation } : {}) }
}

/** The size a Block without a numeric one is laid out with inside a control: its definition's default. */
function fallbackSize(e: { view_type?: string | null }): { w: number; h: number } {
  const def = e.view_type ? blockRegistry.get(e.view_type) : undefined
  return def?.defaultSize ?? { w: 160, h: 90 }
}

/** World rectangles of every Block and group of `surfaceId`, in paint order (parents before children), then
 * the connectors routed between them (`forms`: the outline and anchors each target declares; `folded`: the nodes
 * the reader folded in a control). */
export function layoutSurface(outline: OutlineModel, surfaceId: string, forms: (id: string) => TargetForm = () => RECT_FORM,
  folded: (containerId: string) => ReadonlySet<string> | undefined = () => undefined): Map<string, Laid> {
  const out = new Map<string, Laid>()
  let paint = 0
  const add = (entity: EntityEnvelope, rect: Rect, rotation: number, depth: number, parentId: string, lockedAbove: boolean, extra: Partial<Laid> = {}): Laid => {
    const isGroup = entity.type_id === 'buckyos.container'
    const locked = lockedAbove || entity.locked === true
    const laid: Laid = { entity, rect, rotation, bounds: rotatedBounds(rect, rotation), depth, parentId, isGroup, paint: paint++, locked, ...extra }
    out.set(entity.entity_id, laid)
    return laid
  }
  const palettes = new Map<string, Map<string, string>>()
  const colorOf = (c: PlacedControl, id: string) => {
    let colors = palettes.get(c.solved.id)
    if (!colors) { colors = branchColors(c.solved.layout.nodes, c.solved.config, (n) => outline.get(n)?.branch_color); palettes.set(c.solved.id, colors) }
    return colors.get(id) ?? null
  }
  /** A control placed (with its nested controls) in `placed`: its box, then its Blocks in BlockTree pre-order. */
  const control = (entity: EntityEnvelope, placed: { nodes: Map<string, PlacedNode>; controls: Map<string, PlacedControl> }, depth: number, parentId: string, lockedAbove: boolean, extra: Partial<Laid> = {}) => {
    const box = placed.controls.get(entity.entity_id)!
    const me = add(entity, box.box, 0, depth, parentId, lockedAbove, { ...extra, control: { kind: entity.kind ?? '', origin: box.origin, placed: box }, origin: box.origin })
    const nodes = (parent: string, nodeDepth: number, locked: boolean) => {
      for (const child of outline.childrenOf(parent)) {
        const p = placed.nodes.get(child.entity_id)
        if (!p || (child.type_id !== 'buckyos.cell' && !isCanvasContainer(child))) continue
        const node: ControlNode = { control: p.control, depth: p.depth, grow: p.grow, pinned: p.pinned, hasChildren: p.hasChildren, collapsed: p.collapsed, parent: p.parent,
          color: colorOf(placed.controls.get(p.control)!, child.entity_id) }
        if (placed.controls.has(child.entity_id)) { control(child, placed, nodeDepth, parent, locked, { node }); continue }
        const rect = { x: p.x, y: p.y, w: p.w, h: p.h }
        const laid = add(child, rect, 0, nodeDepth, parent, locked, { node })
        if (child.type_id === 'buckyos.container') walk(child.entity_id, { x: rect.x, y: rect.y }, nodeDepth + 1, laid.locked)
        else if (!p.collapsed) nodes(child.entity_id, nodeDepth + 1, laid.locked)
      }
    }
    nodes(entity.entity_id, depth + 1, me.locked)
  }
  const walk = (parentId: string, origin: { x: number; y: number }, depth: number, lockedAbove: boolean) => {
    let autoY = origin.y
    for (const entity of outline.childrenOf(parentId)) {
      if (entity.type_id !== 'buckyos.cell' && !isCanvasContainer(entity)) continue
      const line = isConnector(entity)
      const def = isControlKind(entity.kind) ? controlRegistry.get(entity.kind) : undefined
      if (def) {
        // a control: solved from the outline (nested ones first), placed by its origin
        const solved = def.layout.solve(outline, entity.entity_id, { measurer: nodeMeasurer, collapsed: folded, fallbackSize })
        const at = { x: origin.x + (num(entity.placement?.x) ?? 0), y: entity.placement ? origin.y + (num(entity.placement.y) ?? 0) : autoY - solved.layout.bounds.y }
        control(entity, placeControl(solved, at), depth, parentId, lockedAbove)
        autoY = at.y + solved.layout.bounds.y + solved.layout.bounds.h + 24
        continue
      }
      const stored = entity.placement
      const p = stored ? numericPlacement(stored, isTextView(entity.view_type) ? { ...DEFAULT_PLACEMENT, y: autoY - origin.y } : DEFAULT_PLACEMENT)
        : line ? { x: 0, y: autoY - origin.y, w: 0, h: 0 } : { ...DEFAULT_PLACEMENT, y: autoY - origin.y }
      autoY += p.h + 24
      const rect = { x: origin.x + p.x, y: origin.y + p.y, w: p.w, h: p.h }
      const isGroup = entity.type_id === 'buckyos.container'
      const rotation = isGroup || line ? 0 : normalizeRotation(p.rotation ?? 0)
      const laid = add(entity, rect, rotation, depth, parentId, lockedAbove, isGroup && entity.kind !== 'group' ? { unknownKind: entity.kind ?? '' } : {})
      const parent = out.get(parentId)
      // the first child of a group, a frame or shape covering it, is the group's look
      if (parent?.isGroup && !parent.control && parent.backingId === undefined && !rotation && (entity.view_type === 'frame' || entity.view_type === 'shape')
        && Math.abs(rect.x - parent.rect.x) <= 1 && Math.abs(rect.y - parent.rect.y) <= 1 && Math.abs(rect.w - parent.rect.w) <= 1 && Math.abs(rect.h - parent.rect.h) <= 1
        && outline.childrenOf(parentId).find((c) => c.type_id === 'buckyos.cell' || isCanvasContainer(c))?.entity_id === entity.entity_id) {
        parent.backingId = entity.entity_id
        laid.backing = true
      }
      if (isGroup) walk(entity.entity_id, { x: rect.x, y: rect.y }, depth + 1, laid.locked)
    }
  }
  walk(surfaceId, { x: 0, y: 0 }, 0, false)
  resolveConnectors(out, outline, forms)
  return out
}

/** Where the children of `parentId` are placed from: a control's origin, else its rectangle's top-left. */
export function originOf(laid: Map<string, Laid>, parentId: string): { x: number; y: number } {
  const parent = laid.get(parentId)
  return parent ? parent.origin ?? { x: parent.rect.x, y: parent.rect.y } : { x: 0, y: 0 }
}

/** A placement relative to `parentId` for the world rect `rect` (turned by `rotation`, kept only when not 0).
 * Sizes are at least `minSize` (a connector's box may be flat: 0). */
export function relativeTo(laid: Map<string, Laid>, parentId: string, rect: Rect, rotation = 0, minSize = 1): Placement {
  const { x: ox, y: oy } = originOf(laid, parentId)
  const r = normalizeRotation(rotation)
  return { x: Math.round(rect.x - ox), y: Math.round(rect.y - oy), w: Math.max(minSize, Math.round(rect.w)), h: Math.max(minSize, Math.round(rect.h)), ...(r ? { rotation: r } : {}) }
}

/** The placement of a laid-out Block at the world rect `rect` (its own by default) under `parentId`. A control keeps
 * its `"auto"` size and is placed by its origin; a Block inside a control keeps the size components it stores. */
export function placementOf(laid: Map<string, Laid>, l: Laid, parentId = l.parentId, rect = l.rect, rotation = l.rotation): StoredPlacement {
  if (l.control) {
    const { x: ox, y: oy } = originOf(laid, parentId)
    return { x: Math.round(l.control.origin.x + rect.x - l.rect.x - ox), y: Math.round(l.control.origin.y + rect.y - l.rect.y - oy), w: 'auto', h: 'auto' }
  }
  if (l.node) {
    const stored = l.entity.placement
    const p = relativeTo(laid, parentId, rect, 0)
    return { x: p.x, y: p.y, w: stored?.w ?? p.w, h: stored?.h ?? p.h }
  }
  return l.connector ? relativeTo(laid, parentId, rect, 0, 0) : relativeTo(laid, parentId, rect, rotation)
}

/** The placement of a laid-out Block moved by (dx, dy), under `parentId` (its own parent by default). */
export function movedPlacement(laid: Map<string, Laid>, l: Laid, dx: number, dy: number, parentId = l.parentId): StoredPlacement {
  return placementOf(laid, l, parentId, { ...l.rect, x: l.rect.x + dx, y: l.rect.y + dy })
}

/** The group an object centred on `p` belongs in (draw.io 导入 TODO §2.2): the top-most group under it that is not
 * locked and not `excluded` (the moving objects themselves), else the Surface. A control and what it lays out are
 * never dropped into (R9: its Blocks come by copy and paste). */
export function groupAt(laid: Map<string, Laid>, p: { x: number; y: number }, surfaceId: string, excluded: ReadonlySet<string> = new Set()): string {
  let best: Laid | null = null
  for (const g of laid.values()) {
    if (!g.isGroup || g.control || g.node || excluded.has(g.entity.entity_id) || g.locked) continue
    if (p.x < g.rect.x || p.x > g.rect.x + g.rect.w || p.y < g.rect.y || p.y > g.rect.y + g.rect.h) continue
    if (!best || g.paint > best.paint) best = g
  }
  return best ? best.entity.entity_id : surfaceId
}

export function boundsOf(laid: Map<string, Laid>, ids: Iterable<string>): Rect | null {
  const rects: Rect[] = []
  for (const id of ids) { const l = laid.get(id); if (l) rects.push(l.bounds) }
  return union(rects)
}

/** The whole Surface's extent (for "fit all"). */
export function surfaceBounds(laid: Map<string, Laid>): Rect | null {
  return union([...laid.values()].map((l) => l.bounds))
}

/** Top-level selection: drop ids whose ancestor group is also selected (moving the group moves them). */
export function topLevel(laid: Map<string, Laid>, ids: Set<string>): string[] {
  return [...ids].filter((id) => {
    let cur = laid.get(id)?.parentId
    while (cur && laid.has(cur)) { if (ids.has(cur)) return false; cur = laid.get(cur)?.parentId }
    return true
  })
}
