/* Layout maths of a free Surface (phase two §8.2): child placements are relative to their container
 * (Surface or UI group); this turns the BlockTree into world rectangles, finds free spots for new
 * Blocks, and converts between parents when grouping or moving across containers. */

import type { EntityEnvelope, Placement } from '../../api/types'
import type { OutlineModel } from '../../state/outline'
import { normalizeRotation, rotatedBounds } from './geometry'
import { union, type Rect } from './render/camera'

/** `paint` is the stacking rank: BlockTree pre-order (a parent before its children, siblings by `order_key`),
 * higher is drawn on top. Painting and hit testing both follow it. `rect` is the unrotated layout rect and
 * `rotation` turns it about its centre; `bounds` is the axis-aligned box around the result (culling, marquee,
 * fit). Groups do not rotate yet (P2): their stored rotation is not drawn. `locked`: the Block or an
 * enclosing group is locked (标准对象的交互改进 §7.2). */
export interface Laid { entity: EntityEnvelope; rect: Rect; rotation: number; bounds: Rect; depth: number; parentId: string; isGroup: boolean; paint: number; locked: boolean }

export const DEFAULT_PLACEMENT: Placement = { x: 0, y: 0, w: 320, h: 200 }

/** World rectangles of every Block and group of `surfaceId`, in paint order (parents before children). */
export function layoutSurface(outline: OutlineModel, surfaceId: string): Map<string, Laid> {
  const out = new Map<string, Laid>()
  let paint = 0
  const walk = (parentId: string, origin: { x: number; y: number }, depth: number, lockedAbove: boolean) => {
    let autoY = origin.y
    for (const entity of outline.childrenOf(parentId)) {
      if (entity.type_id !== 'buckyos.cell' && entity.kind !== 'group') continue
      const p = entity.placement ?? { ...DEFAULT_PLACEMENT, y: autoY - origin.y }
      autoY += p.h + 24
      const rect = { x: origin.x + p.x, y: origin.y + p.y, w: p.w, h: p.h }
      const isGroup = entity.type_id === 'buckyos.container'
      const rotation = isGroup ? 0 : normalizeRotation(p.rotation ?? 0)
      const locked = lockedAbove || entity.locked === true
      out.set(entity.entity_id, { entity, rect, rotation, bounds: rotatedBounds(rect, rotation), depth, parentId, isGroup, paint: paint++, locked })
      if (isGroup) walk(entity.entity_id, { x: rect.x, y: rect.y }, depth + 1, locked)
    }
  }
  walk(surfaceId, { x: 0, y: 0 }, 0, false)
  return out
}

/** A placement relative to `parentId` for the world rect `rect` (turned by `rotation`, kept only when not 0). */
export function relativeTo(laid: Map<string, Laid>, parentId: string, rect: Rect, rotation = 0): Placement {
  const parent = laid.get(parentId)
  const ox = parent ? parent.rect.x : 0
  const oy = parent ? parent.rect.y : 0
  const r = normalizeRotation(rotation)
  return { x: Math.round(rect.x - ox), y: Math.round(rect.y - oy), w: Math.max(1, Math.round(rect.w)), h: Math.max(1, Math.round(rect.h)), ...(r ? { rotation: r } : {}) }
}

/** The placement of a laid-out Block moved by (dx, dy), under `parentId` (its own parent by default). */
export function movedPlacement(laid: Map<string, Laid>, l: Laid, dx: number, dy: number, parentId = l.parentId): Placement {
  return relativeTo(laid, parentId, { ...l.rect, x: l.rect.x + dx, y: l.rect.y + dy }, l.rotation)
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
