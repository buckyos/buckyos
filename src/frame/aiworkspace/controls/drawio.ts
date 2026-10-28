/* A control for draw.io (脑图 TODO M4): draw.io has no mind map, so the control is materialised — every control
 * (nested ones too) becomes a group at its box, every node a vertex at its laid-out rectangle (all expanded: an
 * export has no reader), every implicit edge a real edge bound to the two nodes' facing sides. Importing that file
 * back gives ordinary shapes and lines. */

import { isTextView } from './contract.ts'
import { branchColors, edgeEnds, edgeWidth } from './edges.ts'
import { typography, type NodeMeasurer } from './measure.ts'
import { placeControl, solveControl, type ControlSource } from './solve.ts'
import type { EdgeStyle, Rect } from './tree.ts'

export interface ExportedControl {
  /** Each control's box, relative to its parent's box (the outer one: to the parent's origin). */
  groups: { id: string; parent: string | null; placement: Rect; title?: string }[]
  /** Each node at its rectangle relative to its control's box; text nodes carry the look of their depth. */
  nodes: { id: string; control: string; placement: Rect; text: boolean; depth: number; color: string | null }[]
  /** Each parent → child edge, its ends on the facing sides; the box of its two ends relative to the control's box. */
  edges: { id: string; control: string; from: string; to: string; fromAnchor: string; toAnchor: string; route: EdgeStyle; color: string; width: number; placement: Rect; flip: { h?: boolean; v?: boolean } }[]
}

const SIDE: Record<string, [string, string]> = { right: ['e', 'w'], left: ['w', 'e'], down: ['s', 'n'], up: ['n', 's'] }

/** The control `containerId` laid out (all expanded) with its origin (its stored x / y) at `origin` under `parent`. */
export function exportControl(src: ControlSource, containerId: string, measurer: NodeMeasurer, parent: string | null, origin: { x: number; y: number }): ExportedControl {
  const solved = solveControl(src, containerId, { measurer })
  const b = solved.layout.bounds
  const placed = placeControl(solved, { x: -b.x, y: -b.y })
  const out: ExportedControl = { groups: [], nodes: [], edges: [] }
  const boxOf = new Map<string, Rect>()
  for (const [id, c] of placed.controls) boxOf.set(id, c.box)
  for (const [id, c] of placed.controls) {
    const node = placed.nodes.get(id)
    const parentBox = node ? boxOf.get(node.control)! : null
    const title = src.get(id)?.title ?? undefined
    out.groups.push({ id, parent: node ? node.control : parent, placement: parentBox ? { x: c.box.x - parentBox.x, y: c.box.y - parentBox.y, w: c.box.w, h: c.box.h } : { x: origin.x + b.x, y: origin.y + b.y, w: c.box.w, h: c.box.h }, ...(title ? { title } : {}) })
    const colors = branchColors(c.solved.layout.nodes, c.solved.config, (n) => src.get(n)?.branch_color)
    for (const l of c.solved.layout.nodes.values()) {
      if (l.parent === null) continue
      const from = placed.nodes.get(l.parent)!
      const to = placed.nodes.get(l.id)!
      const ends = edgeEnds(from, to, l.grow)
      const way = ends.vertical ? (ends.e.y >= ends.s.y ? 'down' : 'up') : ends.e.x >= ends.s.x ? 'right' : 'left'
      const [fromAnchor, toAnchor] = SIDE[way]
      const x0 = Math.min(ends.s.x, ends.e.x), y0 = Math.min(ends.s.y, ends.e.y)
      out.edges.push({ id: `${l.parent}~${l.id}`, control: id, from: l.parent, to: l.id, fromAnchor, toAnchor, route: c.solved.config.edge, color: colors.get(l.id) ?? '#4f8df7', width: edgeWidth(l.depth),
        placement: { x: x0 - c.box.x, y: y0 - c.box.y, w: Math.abs(ends.e.x - ends.s.x), h: Math.abs(ends.e.y - ends.s.y) },
        flip: { ...(ends.e.x < ends.s.x ? { h: true } : {}), ...(ends.e.y < ends.s.y ? { v: true } : {}) } })
    }
  }
  for (const n of placed.nodes.values()) {
    if (placed.controls.has(n.id)) continue
    const box = boxOf.get(n.control)!
    const e = src.get(n.id)
    const colors = branchColors(placed.controls.get(n.control)!.solved.layout.nodes, placed.controls.get(n.control)!.solved.config, (id) => src.get(id)?.branch_color)
    out.nodes.push({ id: n.id, control: n.control, placement: { x: n.x - box.x, y: n.y - box.y, w: n.w, h: n.h }, text: e?.type_id === 'buckyos.cell' && isTextView(e.view_type), depth: n.depth, color: colors.get(n.id) ?? null })
  }
  return out
}

/** The `shape` Block config a text node is drawn with in draw.io: a rounded box in its depth's type. */
export function nodeShapeConfig(depth: number, color: string | null): Record<string, unknown> {
  const t = typography(depth)
  if (depth === 0) return { shape: 'rect', radius: 10, fill: '#1f2937', stroke: 'none', font_size: t.size, bold: true, font_color: '#ffffff', wrap: true }
  return { shape: 'rect', radius: 8, fill: '#ffffff', stroke: depth === 1 && color ? color : '#d0d5dd', stroke_width: depth === 1 ? 2 : 1, font_size: t.size, ...(t.weight >= 600 ? { bold: true } : {}), wrap: true }
}
