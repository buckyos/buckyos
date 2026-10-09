/* Layout maths of a free Surface (phase two §8.2): child placements are relative to their container
 * (Surface or UI group); this turns the BlockTree into world rectangles, finds free spots for new
 * Blocks, and converts between parents when grouping or moving across containers. */

import type { EntityEnvelope, Placement } from '../../api/types'
import type { OutlineModel } from '../../state/outline'
import { isConnector, RECT_FORM, resolveConnectors, type LaidConnector, type TargetForm } from './connectors/layout'
import { normalizeRotation, rotatedBounds } from './geometry'
import { union, type Rect } from './render/camera'

/** `paint` is the stacking rank: BlockTree pre-order (a parent before its children, siblings by `order_key`),
 * higher is drawn on top. Painting and hit testing both follow it. `rect` is the unrotated layout rect and
 * `rotation` turns it about its centre; `bounds` is the axis-aligned box around the result (culling, marquee,
 * fit). Groups do not rotate yet (P2): their stored rotation is not drawn. `locked`: the Block or an
 * enclosing group is locked (标准对象的交互改进 §7.2). A connector (连接线实现方案 §9.2) has its stored box as
 * `rect` (what placement writes use; zero width or height allowed) and its routed path in `connector`, with
 * `bounds` around the path and label. */
export interface Laid { entity: EntityEnvelope; rect: Rect; rotation: number; bounds: Rect; depth: number; parentId: string; isGroup: boolean; paint: number; locked: boolean; connector?: LaidConnector }

export const DEFAULT_PLACEMENT: Placement = { x: 0, y: 0, w: 320, h: 200 }

/** World rectangles of every Block and group of `surfaceId`, in paint order (parents before children), then
 * the connectors routed between them (`forms`: the outline and anchors each target declares). */
export function layoutSurface(outline: OutlineModel, surfaceId: string, forms: (id: string) => TargetForm = () => RECT_FORM): Map<string, Laid> {
  const out = new Map<string, Laid>()
  let paint = 0
  const walk = (parentId: string, origin: { x: number; y: number }, depth: number, lockedAbove: boolean) => {
    let autoY = origin.y
    for (const entity of outline.childrenOf(parentId)) {
      if (entity.type_id !== 'buckyos.cell' && entity.kind !== 'group') continue
      const line = isConnector(entity)
      const p = entity.placement ?? (line ? { x: 0, y: autoY - origin.y, w: 0, h: 0 } : { ...DEFAULT_PLACEMENT, y: autoY - origin.y })
      autoY += p.h + 24
      const rect = { x: origin.x + p.x, y: origin.y + p.y, w: p.w, h: p.h }
      const isGroup = entity.type_id === 'buckyos.container'
      const rotation = isGroup || line ? 0 : normalizeRotation(p.rotation ?? 0)
      const locked = lockedAbove || entity.locked === true
      out.set(entity.entity_id, { entity, rect, rotation, bounds: rotatedBounds(rect, rotation), depth, parentId, isGroup, paint: paint++, locked })
      if (isGroup) walk(entity.entity_id, { x: rect.x, y: rect.y }, depth + 1, locked)
    }
  }
  walk(surfaceId, { x: 0, y: 0 }, 0, false)
  resolveConnectors(out, outline, forms)
  return out
}

/** A placement relative to `parentId` for the world rect `rect` (turned by `rotation`, kept only when not 0).
 * Sizes are at least `minSize` (a connector's box may be flat: 0). */
export function relativeTo(laid: Map<string, Laid>, parentId: string, rect: Rect, rotation = 0, minSize = 1): Placement {
  const parent = laid.get(parentId)
  const ox = parent ? parent.rect.x : 0
  const oy = parent ? parent.rect.y : 0
  const r = normalizeRotation(rotation)
  return { x: Math.round(rect.x - ox), y: Math.round(rect.y - oy), w: Math.max(minSize, Math.round(rect.w)), h: Math.max(minSize, Math.round(rect.h)), ...(r ? { rotation: r } : {}) }
}

/** The placement of a laid-out Block at the world rect `rect` (its own by default) under `parentId`. */
export function placementOf(laid: Map<string, Laid>, l: Laid, parentId = l.parentId, rect = l.rect, rotation = l.rotation): Placement {
  return l.connector ? relativeTo(laid, parentId, rect, 0, 0) : relativeTo(laid, parentId, rect, rotation)
}

/** The placement of a laid-out Block moved by (dx, dy), under `parentId` (its own parent by default). */
export function movedPlacement(laid: Map<string, Laid>, l: Laid, dx: number, dy: number, parentId = l.parentId): Placement {
  return placementOf(laid, l, parentId, { ...l.rect, x: l.rect.x + dx, y: l.rect.y + dy })
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
