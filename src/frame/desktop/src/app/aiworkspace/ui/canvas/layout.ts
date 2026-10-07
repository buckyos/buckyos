/* Layout maths of a free Surface (phase two §8.2): child placements are relative to their container
 * (Surface or UI group); this turns the BlockTree into world rectangles, finds free spots for new
 * Blocks, and converts between parents when grouping or moving across containers. */

import type { EntityEnvelope, Placement } from '../../api/types'
import type { OutlineModel } from '../../state/outline'
import { union, type Rect } from './render/camera'

/** `paint` is the stacking rank: BlockTree pre-order (a parent before its children, siblings by `order_key`),
 * higher is drawn on top. Painting and hit testing both follow it. */
export interface Laid { entity: EntityEnvelope; rect: Rect; depth: number; parentId: string; isGroup: boolean; paint: number }

export const DEFAULT_PLACEMENT: Placement = { x: 0, y: 0, w: 320, h: 200 }

/** World rectangles of every Block and group of `surfaceId`, in paint order (parents before children). */
export function layoutSurface(outline: OutlineModel, surfaceId: string): Map<string, Laid> {
  const out = new Map<string, Laid>()
  let paint = 0
  const walk = (parentId: string, origin: { x: number; y: number }, depth: number) => {
    let autoY = origin.y
    for (const entity of outline.childrenOf(parentId)) {
      if (entity.type_id !== 'buckyos.cell' && entity.kind !== 'group') continue
      const p = entity.placement ?? { ...DEFAULT_PLACEMENT, y: autoY - origin.y }
      autoY += p.h + 24
      const rect = { x: origin.x + p.x, y: origin.y + p.y, w: p.w, h: p.h }
      const isGroup = entity.type_id === 'buckyos.container'
      out.set(entity.entity_id, { entity, rect, depth, parentId, isGroup, paint: paint++ })
      if (isGroup) walk(entity.entity_id, { x: rect.x, y: rect.y }, depth + 1)
    }
  }
  walk(surfaceId, { x: 0, y: 0 }, 0)
  return out
}

/** A placement relative to `parentId` for the world rect `rect`. */
export function relativeTo(laid: Map<string, Laid>, parentId: string, rect: Rect): Placement {
  const parent = laid.get(parentId)
  const ox = parent ? parent.rect.x : 0
  const oy = parent ? parent.rect.y : 0
  return { x: Math.round(rect.x - ox), y: Math.round(rect.y - oy), w: Math.max(1, Math.round(rect.w)), h: Math.max(1, Math.round(rect.h)) }
}

/** A free spot of `size` near `near` (right of it, then below), avoiding existing rects. */
export function freeSpot(laid: Map<string, Laid>, size: { w: number; h: number }, near: Rect | null): Rect {
  const rects = [...laid.values()].filter((l) => !l.isGroup).map((l) => l.rect)
  const start = near ? { x: near.x + near.w + 40, y: near.y } : { x: 40, y: 40 }
  const overlaps = (r: Rect) => rects.some((o) => r.x < o.x + o.w + 16 && r.x + r.w + 16 > o.x && r.y < o.y + o.h + 16 && r.y + r.h + 16 > o.y)
  let candidate: Rect = { ...start, ...size }
  for (let i = 0; i < 200 && overlaps(candidate); i++) candidate = { ...candidate, y: candidate.y + 40 * (i % 2 === 0 ? 1 : 1), x: candidate.x + (i % 5 === 4 ? 60 : 0) }
  return candidate
}

export function boundsOf(laid: Map<string, Laid>, ids: Iterable<string>): Rect | null {
  const rects: Rect[] = []
  for (const id of ids) { const l = laid.get(id); if (l) rects.push(l.rect) }
  return union(rects)
}

/** The whole Surface's extent (for "fit all"). */
export function surfaceBounds(laid: Map<string, Laid>): Rect | null {
  return union([...laid.values()].map((l) => l.rect))
}

/** Top-level selection: drop ids whose ancestor group is also selected (moving the group moves them). */
export function topLevel(laid: Map<string, Laid>, ids: Set<string>): string[] {
  return [...ids].filter((id) => {
    let cur = laid.get(id)?.parentId
    while (cur && laid.has(cur)) { if (ids.has(cur)) return false; cur = laid.get(cur)?.parentId }
    return true
  })
}
