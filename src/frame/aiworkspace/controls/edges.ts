/* The implicit edges of a mind map (脑图方案 §6.1, R4): drawn by the control from the parent → child relation, never
 * stored. Routing is the control's `edge` (curve / elbow / straight); the colour is the first-level node's branch
 * colour, inherited down and overridable per node (`config.branch_color`); the width thins out by depth. Edges of the
 * same colour and width are merged into one path (a thousand nodes, one path per branch). Shared by the canvas, the
 * static SVG of a published page and the draw.io export. */

import type { EdgeStyle, LaidNode, MindmapConfig, Rect } from './tree.ts'

/** Branch colours in turn when the control names none. */
export const BRANCH_PALETTE = ['#4f8df7', '#e8590c', '#2f9e44', '#ae3ec9', '#f08c00', '#0c8599', '#e64980', '#5c7cfa']

export function edgeWidth(depth: number): number {
  return depth <= 1 ? 2.5 : depth === 2 ? 2 : 1.5
}

/** The two ends of the edge into `to`: the sides facing each other along the growth axis. */
export function edgeEnds(from: Rect, to: Rect, grow: LaidNode['grow']): { s: { x: number; y: number }; e: { x: number; y: number }; vertical: boolean } {
  if (grow === 'down') {
    const below = to.y + to.h / 2 >= from.y + from.h / 2
    return { s: { x: from.x + from.w / 2, y: below ? from.y + from.h : from.y }, e: { x: to.x + to.w / 2, y: below ? to.y : to.y + to.h }, vertical: true }
  }
  const right = to.x + to.w / 2 >= from.x + from.w / 2
  return { s: { x: right ? from.x + from.w : from.x, y: from.y + from.h / 2 }, e: { x: right ? to.x : to.x + to.w, y: to.y + to.h / 2 }, vertical: false }
}

const r = (n: number) => Math.round(n * 10) / 10

/** SVG path data of one edge. */
export function edgePath(from: Rect, to: Rect, grow: LaidNode['grow'], style: EdgeStyle): string {
  const { s, e, vertical } = edgeEnds(from, to, grow)
  if (style === 'straight') return `M${r(s.x)} ${r(s.y)}L${r(e.x)} ${r(e.y)}`
  if (style === 'elbow') {
    if (vertical) { const m = (s.y + e.y) / 2; return `M${r(s.x)} ${r(s.y)}V${r(m)}H${r(e.x)}V${r(e.y)}` }
    const m = (s.x + e.x) / 2
    return `M${r(s.x)} ${r(s.y)}H${r(m)}V${r(e.y)}H${r(e.x)}`
  }
  if (vertical) { const d = (e.y - s.y) / 2; return `M${r(s.x)} ${r(s.y)}C${r(s.x)} ${r(s.y + d)} ${r(e.x)} ${r(e.y - d)} ${r(e.x)} ${r(e.y)}` }
  const d = (e.x - s.x) / 2
  return `M${r(s.x)} ${r(s.y)}C${r(s.x + d)} ${r(s.y)} ${r(e.x - d)} ${r(e.y)} ${r(e.x)} ${r(e.y)}`
}

/** The branch colour of every laid node (roots have none): `own` gives a node's own `branch_color`. */
export function branchColors(nodes: Map<string, LaidNode>, config: MindmapConfig, own: (id: string) => string | null | undefined): Map<string, string> {
  const palette = config.branch_colors?.length ? config.branch_colors : BRANCH_PALETTE
  const out = new Map<string, string>()
  const turn = new Map<string, number>()
  for (const n of nodes.values()) {
    if (n.parent === null) continue
    const parent = nodes.get(n.parent)
    let color = own(n.id) ?? undefined
    if (!color) {
      if (parent && parent.parent === null) {
        const i = turn.get(parent.id) ?? 0
        turn.set(parent.id, i + 1)
        color = palette[i % palette.length]
      } else color = (parent && out.get(parent.id)) ?? palette[0]
    } else if (parent && parent.parent === null) turn.set(parent.id, (turn.get(parent.id) ?? 0) + 1)
    out.set(n.id, color)
  }
  return out
}

export interface EdgeStroke { d: string; color: string; width: number }

/** Every edge between shown nodes, merged per colour and width. `offset` moves them (world coordinates). */
export function edgeStrokes(nodes: Map<string, LaidNode>, config: MindmapConfig, colors: Map<string, string>, offset = { x: 0, y: 0 }): EdgeStroke[] {
  const groups = new Map<string, EdgeStroke>()
  for (const n of nodes.values()) {
    const parent = n.parent === null ? undefined : nodes.get(n.parent)
    if (!parent) continue
    const color = colors.get(n.id) ?? BRANCH_PALETTE[0]
    const width = edgeWidth(n.depth)
    const shift = (q: Rect) => ({ x: q.x + offset.x, y: q.y + offset.y, w: q.w, h: q.h })
    const d = edgePath(shift(parent), shift(n), n.grow, config.edge)
    const key = `${color}|${width}`
    const g = groups.get(key)
    if (g) g.d += d
    else groups.set(key, { d, color, width })
  }
  return [...groups.values()]
}
