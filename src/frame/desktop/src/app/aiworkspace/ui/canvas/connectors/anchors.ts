/* Anchors (连接线实现方案 §4.3): the named spots on a Block where a line end attaches. A bound end stores the
 * anchor's id (`{ kind: "named", id }`); where that id is on the Block is the Block's business, so a line keeps
 * its anchor when the Block is resized, turned, or changes its outline (a shape going from rectangle to ellipse).
 *
 *   default set   16 anchors named like a 16-point compass rose, clockwise from `n`: the 8 major ones (corners
 *                 and side midpoints of a rectangle; every 45° of an ellipse) and 8 minor ones between them
 *                 (the quarter points of a rectangle's sides; the 22.5° points of an ellipse)
 *   own set       a Block definition may declare its own anchors (`BlockDefinition.anchors`), e.g. a port per row
 *   subdivision   minor anchors are shown and snapped to only when the Block is large enough on screen
 *   sides         each anchor leaves by one side (a side point), two (a corner: the one facing the other end
 *                 wins) or any (an inner point) */

import type { Point } from '../geometry'

export type Shape = 'rect' | 'ellipse'
export type Side = 'n' | 'e' | 's' | 'w'

/** An anchor a Block offers. */
export interface AnchorDef {
  /** What a line stores: stable for the Block's life (≤ 64 of `A–Z a–z 0–9 _ . : -`). */
  id: string
  /** Position in the Block's layout rect, `[0, 1]` each, in its own (unturned) axes. */
  x: number
  y: number
  /** Plus this many world units (a port at a fixed distance from an edge). */
  dx?: number
  dy?: number
  /** The sides a line may leave by; by default from the position (see `sidesOf`). */
  sides?: Side[]
  /** A subdivision: offered only when the Block has room for it on screen. */
  minor?: boolean
  /** Shown in tooltips; compass ids have their own. */
  label?: string
}

export const SIDE_VEC: Record<Side, Point> = { n: { x: 0, y: -1 }, e: { x: 1, y: 0 }, s: { x: 0, y: 1 }, w: { x: -1, y: 0 } }

/** The 16 compass ids, clockwise from the top. Even positions are major. */
export const COMPASS = ['n', 'nne', 'ne', 'ene', 'e', 'ese', 'se', 'sse', 's', 'ssw', 'sw', 'wsw', 'w', 'wnw', 'nw', 'nnw'] as const

const COMPASS_LABELS: Record<string, string> = {
  n: '上', nne: '上偏右', ne: '右上', ene: '右偏上', e: '右', ese: '右偏下', se: '右下', sse: '下偏右',
  s: '下', ssw: '下偏左', sw: '左下', wsw: '左偏下', w: '左', wnw: '左偏上', nw: '左上', nnw: '上偏左',
}

/** Rectangle: corners, side midpoints and quarter points, walking the outline clockwise from the top midpoint. */
const RECT_SPOTS: [number, number][] = [
  [0.5, 0], [0.75, 0], [1, 0], [1, 0.25], [1, 0.5], [1, 0.75], [1, 1], [0.75, 1],
  [0.5, 1], [0.25, 1], [0, 1], [0, 0.75], [0, 0.5], [0, 0.25], [0, 0], [0.25, 0],
]

const round6 = (v: number) => Math.round(v * 1e6) / 1e6

export const RECT_ANCHORS: readonly AnchorDef[] = COMPASS.map((id, i) => ({ id, x: RECT_SPOTS[i][0], y: RECT_SPOTS[i][1], ...(i % 2 ? { minor: true } : {}) }))

/** Ellipse: every 22.5° of the outline (`n` at the top). */
export const ELLIPSE_ANCHORS: readonly AnchorDef[] = COMPASS.map((id, i) => {
  const a = ((-90 + i * 22.5) * Math.PI) / 180
  return { id, x: round6(0.5 + 0.5 * Math.cos(a)), y: round6(0.5 + 0.5 * Math.sin(a)), ...(i % 2 ? { minor: true } : {}) }
})

export function defaultAnchors(shape: Shape): readonly AnchorDef[] {
  return shape === 'ellipse' ? ELLIPSE_ANCHORS : RECT_ANCHORS
}

export const ANCHOR_ID = /^[A-Za-z0-9_.:-]{1,64}$/

/** A definition's anchors, read wide: bad entries are dropped, the first of a repeated id wins. */
export function cleanAnchors(list: unknown): AnchorDef[] | null {
  if (!Array.isArray(list)) return null
  const seen = new Set<string>()
  const out: AnchorDef[] = []
  const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)
  for (const a of list as Partial<AnchorDef>[]) {
    if (!a || typeof a.id !== 'string' || !ANCHOR_ID.test(a.id) || seen.has(a.id) || !finite(a.x) || !finite(a.y)) continue
    seen.add(a.id)
    const sides = Array.isArray(a.sides) ? a.sides.filter((s): s is Side => s === 'n' || s === 'e' || s === 's' || s === 'w') : []
    out.push({
      id: a.id, x: Math.min(1, Math.max(0, a.x)), y: Math.min(1, Math.max(0, a.y)),
      ...(finite(a.dx) && a.dx ? { dx: a.dx } : {}), ...(finite(a.dy) && a.dy ? { dy: a.dy } : {}),
      ...(sides.length ? { sides } : {}), ...(a.minor ? { minor: true } : {}), ...(typeof a.label === 'string' && a.label ? { label: a.label } : {}),
    })
  }
  return out
}

const byId = new WeakMap<readonly AnchorDef[], Map<string, AnchorDef>>()

export function findAnchor(anchors: readonly AnchorDef[], id: string): AnchorDef | undefined {
  let map = byId.get(anchors)
  if (!map) {
    map = new Map()
    for (const a of anchors) if (!map.has(a.id)) map.set(a.id, a)
    byId.set(anchors, map)
  }
  return map.get(id)
}

/** The sides an anchor leaves by: declared, else from where it sits — a side point its side, a point on a
 * diagonal (a corner) both sides there (horizontal first), the centre any side. */
export function sidesOf(a: AnchorDef): Side[] {
  if (a.sides?.length) return a.sides
  const nx = 2 * a.x - 1
  const ny = 2 * a.y - 1
  const h: Side = nx >= 0 ? 'e' : 'w'
  const v: Side = ny >= 0 ? 's' : 'n'
  if (Math.abs(nx) < 1e-6 && Math.abs(ny) < 1e-6) return ['e', 's', 'w', 'n']
  if (Math.abs(Math.abs(nx) - Math.abs(ny)) < 1e-6) return [h, v]
  return Math.abs(nx) > Math.abs(ny) ? [h] : [v]
}

export function anchorLabel(a: AnchorDef | undefined, id: string): string {
  return a?.label ?? COMPASS_LABELS[id] ?? id
}
