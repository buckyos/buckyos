/* Resolving a control's Blocks from the outline (脑图方案 §5.3–§5.4, R7, R24): sizes first (a number the user set, a
 * text node's title, a nested control's own box — inner controls are solved first, bottom-up), then the control's
 * layout. Only the outline is read: kind, view, title, placement, side, branch colour, a control's config. Shared by
 * the canvas, the static SVG and the exports, so every reader sees the same map. */

import { isControlKind, isTextView } from './contract.ts'
import { NODE_PAD, NodeMeasurer } from './measure.ts'
import { num, type StoredPlacement } from './placement.ts'
import { layoutTree, mindmapConfig, type LaidNode, type MindmapConfig, type Rect, type TreeLayout, type TreeNode } from './tree.ts'

/** The outline fields a control reads (an `EntityEnvelope` has them). */
export interface ControlEntity {
  entity_id: string
  type_id: string
  kind?: string | null
  view_type?: string | null
  title?: string | null
  placement?: Partial<StoredPlacement> | null
  side?: string | null
  branch_color?: string | null
  config?: unknown
}

export interface ControlSource {
  get(id: string): ControlEntity | undefined
  childrenOf(id: string): ControlEntity[]
}

export interface SolveOptions {
  measurer: NodeMeasurer
  /** The nodes the reader folded in this control (none: all expanded, as published pages, exports and Agents see). */
  collapsed?: (containerId: string) => ReadonlySet<string> | undefined
  /** The size of a Block without a numeric one (a non-text view the core would refuse, an old document). */
  fallbackSize?: (e: ControlEntity) => { w: number; h: number }
}

export interface SolvedControl {
  id: string
  config: MindmapConfig
  layout: TreeLayout
  /** Nested controls among the nodes, solved first. */
  nested: Map<string, SolvedControl>
}

/** Is `e` a Block that belongs to the tree of a control (a Cell, or a container drawn on a canvas)? */
const isBlock = (e: ControlEntity) => e.type_id === 'buckyos.cell' || (e.type_id === 'buckyos.container' && e.kind !== 'surface' && e.kind !== 'folder')

export function solveControl(src: ControlSource, containerId: string, opts: SolveOptions): SolvedControl {
  const container = src.get(containerId)
  const config = mindmapConfig(container?.config)
  const folded = opts.collapsed?.(containerId)
  const nested = new Map<string, SolvedControl>()
  const fallback = opts.fallbackSize ?? (() => ({ w: 160, h: 90 }))
  const node = (e: ControlEntity, depth: number): TreeNode => {
    const p = e.placement ?? {}
    const pin = { ...(num(p.x) !== undefined ? { x: num(p.x) } : {}), ...(num(p.y) !== undefined ? { y: num(p.y) } : {}) }
    const side: { side?: 'left' | 'right' } = e.side === 'left' || e.side === 'right' ? { side: e.side } : {}
    if (e.type_id === 'buckyos.container') {
      if (isControlKind(e.kind)) {
        // a control inside a control (R24): its own box, already solved, is this node's size
        const inner = solveControl(src, e.entity_id, opts)
        nested.set(e.entity_id, inner)
        const b = inner.layout.bounds
        return { id: e.entity_id, w: b.w, h: b.h, offset: { x: b.x, y: b.y }, ...pin, ...side, children: [] }
      }
      const f = fallback(e)
      return { id: e.entity_id, w: num(p.w) ?? f.w, h: num(p.h) ?? f.h, ...pin, ...side, children: [] }
    }
    let w = num(p.w)
    let h = num(p.h)
    if ((w === undefined || h === undefined) && isTextView(e.view_type)) {
      const m = opts.measurer.measure(e.title, depth, w !== undefined ? Math.max(1, w - NODE_PAD.x * 2) : undefined)
      w ??= m.w
      h ??= m.h
    }
    if (w === undefined || h === undefined) {
      const f = fallback(e)
      w ??= f.w
      h ??= f.h
    }
    const kids = src.childrenOf(e.entity_id).filter(isBlock)
    return { id: e.entity_id, w, h, ...pin, ...side, collapsed: !!folded?.has(e.entity_id) && kids.length > 0, children: kids.map((c) => node(c, depth + 1)) }
  }
  const roots = src.childrenOf(containerId).filter(isBlock).map((e) => node(e, 0))
  return { id: containerId, config, layout: layoutTree(roots, config), nested }
}

export interface PlacedNode extends LaidNode {
  /** The control laying this node out. */
  control: string
}

export interface PlacedControl {
  /** The control's box in the caller's coordinates. */
  box: Rect
  /** Its origin (the first root's top-left): what its stored x / y name. */
  origin: { x: number; y: number }
  solved: SolvedControl
}

/** World rectangles of a solved control placed with its origin at `origin`, nested controls included (each node
 * once, in pre-order). `out` collects nodes, `controls` every control's box. */
export function placeControl(solved: SolvedControl, origin: { x: number; y: number }, out = new Map<string, PlacedNode>(), controls = new Map<string, PlacedControl>()): { nodes: Map<string, PlacedNode>; controls: Map<string, PlacedControl> } {
  const b = solved.layout.bounds
  controls.set(solved.id, { box: { x: origin.x + b.x, y: origin.y + b.y, w: b.w, h: b.h }, origin, solved })
  for (const l of solved.layout.nodes.values()) {
    out.set(l.id, { ...l, x: origin.x + l.x, y: origin.y + l.y, control: solved.id })
    const inner = solved.nested.get(l.id)
    if (inner) placeControl(inner, { x: origin.x + l.x - inner.layout.bounds.x, y: origin.y + l.y - inner.layout.bounds.y }, out, controls)
  }
  return { nodes: out, controls }
}
