/* The `tree` layout of a mind map (脑图方案 §5.3–§5.6): a pure function from the logical tree (order, sizes already
 * resolved, pins, sides, collapsed nodes) and the control's config to the rectangle of every shown node, relative to
 * the control's origin (the first root's top-left). One algorithm for the four directions (R13): children are stacked
 * along the cross axis, each subtree taking the extent it needs, centred on their parent; siblings' main-axis edges
 * line up. A pinned node (a free topic, R16) is left out of its siblings' stacking — they keep no room for it — and
 * its own subtree is laid out around it. A collapsed node counts as a leaf. Not versioned (R18): the snapshot tests
 * guard against unintended changes. */

export type Direction = 'right' | 'left' | 'both' | 'down'
export type EdgeStyle = 'curve' | 'elbow' | 'straight'
/** The way a node grows from its parent. */
export type Grow = 'right' | 'left' | 'down'

export interface MindmapConfig {
  direction: Direction
  edge: EdgeStyle
  spacing: { sibling: number; level: number }
  branch_colors?: string[]
}

export const DEFAULT_CONFIG: MindmapConfig = { direction: 'right', edge: 'curve', spacing: { sibling: 12, level: 48 } }
/** Room around the outermost nodes inside the control's box. */
export const BOX_PADDING = 16

/** A control's stored config with defaults filled in (an invalid value falls back like a missing one). */
export function mindmapConfig(raw: unknown): MindmapConfig {
  const c = raw && typeof raw === 'object' ? raw as Record<string, unknown> : {}
  const direction = (['right', 'left', 'both', 'down'] as const).find((d) => d === c.direction) ?? DEFAULT_CONFIG.direction
  const edge = (['curve', 'elbow', 'straight'] as const).find((e) => e === c.edge) ?? DEFAULT_CONFIG.edge
  const s = c.spacing && typeof c.spacing === 'object' ? c.spacing as Record<string, unknown> : {}
  const n = (v: unknown, d: number) => (typeof v === 'number' && Number.isFinite(v) && v >= 0 ? v : d)
  const colors = Array.isArray(c.branch_colors) ? c.branch_colors.filter((x): x is string => typeof x === 'string') : undefined
  return { direction, edge, spacing: { sibling: n(s.sibling, DEFAULT_CONFIG.spacing.sibling), level: n(s.level, DEFAULT_CONFIG.spacing.level) }, ...(colors?.length ? { branch_colors: colors } : {}) }
}

export interface TreeNode {
  id: string
  /** Resolved size (a number the user set, or what the node's own content gives). */
  w: number
  h: number
  /** Pinned position components, relative to the parent's origin (a root's: the control's origin). */
  x?: number
  y?: number
  /** Where the node's rectangle starts relative to its own origin: a nested control's box around its first root. */
  offset?: { x: number; y: number }
  /** Under a two-sided root: the side a first-level node goes to (`auto`: balanced). */
  side?: 'left' | 'right'
  /** Children are not laid out (the reader folded them, a `user_state`, R8). */
  collapsed?: boolean
  children: TreeNode[]
}

export interface Rect { x: number; y: number; w: number; h: number }

export interface LaidNode extends Rect {
  id: string
  parent: string | null
  depth: number
  grow: Grow
  pinned: boolean
  /** Has children (shown or folded): the fold handle is drawn. */
  hasChildren: boolean
  collapsed: boolean
}

export interface TreeLayout {
  /** Every shown node, parents before children (pre-order, siblings in order). */
  nodes: Map<string, LaidNode>
  /** The control's box relative to its origin: the nodes' extent plus padding. */
  bounds: Rect
}

const pinnedOf = (n: TreeNode) => n.x !== undefined || n.y !== undefined

export function layoutTree(roots: TreeNode[], config: MindmapConfig = DEFAULT_CONFIG): TreeLayout {
  const { sibling, level } = config.spacing
  const nodes = new Map<string, LaidNode>()
  const extents = new Map<string, number>()
  const cross = (n: TreeNode, grow: Grow) => (grow === 'down' ? n.w : n.h)
  const shown = (n: TreeNode) => (n.collapsed ? [] : n.children)
  // the extent a subtree needs across the growth axis (pinned children take none)
  const extent = (n: TreeNode, grow: Grow): number => {
    const key = `${grow}\u0000${n.id}`
    const hit = extents.get(key)
    if (hit !== undefined) return hit
    const kids = shown(n).filter((c) => !pinnedOf(c))
    const sum = kids.reduce((s, c) => s + extent(c, grow), 0) + sibling * Math.max(0, kids.length - 1)
    const e = Math.max(cross(n, grow), sum)
    extents.set(key, e)
    return e
  }
  const put = (n: TreeNode, x: number, y: number, parent: string | null, depth: number, grow: Grow, pinned: boolean) => {
    nodes.set(n.id, { id: n.id, x, y, w: n.w, h: n.h, parent, depth, grow, pinned, hasChildren: n.children.length > 0, collapsed: !!n.collapsed && n.children.length > 0 })
  }
  // where a pinned node goes: its stored components relative to the parent's origin, the rest as if it were alone
  const pinnedAt = (n: TreeNode, parent: LaidNode, grow: Grow) => {
    const off = n.offset ?? { x: 0, y: 0 }
    const def = grow === 'down'
      ? { x: parent.x + (parent.w - n.w) / 2, y: parent.y + parent.h + level }
      : { x: grow === 'right' ? parent.x + parent.w + level : parent.x - level - n.w, y: parent.y + (parent.h - n.h) / 2 }
    return { x: n.x !== undefined ? parent.x + n.x + off.x : def.x, y: n.y !== undefined ? parent.y + n.y + off.y : def.y }
  }
  const placeKids = (n: TreeNode, kids: TreeNode[], grow: Grow, depth: number) => {
    const p = nodes.get(n.id)!
    const stacked = kids.filter((c) => !pinnedOf(c))
    const total = stacked.reduce((s, c) => s + extent(c, grow), 0) + sibling * Math.max(0, stacked.length - 1)
    let at = (grow === 'down' ? p.x + p.w / 2 : p.y + p.h / 2) - total / 2
    for (const c of kids) {
      if (pinnedOf(c)) {
        const q = pinnedAt(c, p, grow)
        // a free topic of a two-sided root grows on the side it sits on
        const g: Grow = grow === 'down' ? 'down' : q.x + c.w / 2 < p.x + p.w / 2 ? 'left' : 'right'
        place(c, q.x, q.y, n.id, depth, g, true)
        continue
      }
      const e = extent(c, grow)
      if (grow === 'down') place(c, at + (e - c.w) / 2, p.y + p.h + level, n.id, depth, grow, false)
      else place(c, grow === 'right' ? p.x + p.w + level : p.x - level - c.w, at + (e - c.h) / 2, n.id, depth, grow, false)
      at += e + sibling
    }
  }
  const place = (n: TreeNode, x: number, y: number, parent: string | null, depth: number, grow: Grow, pinned: boolean) => {
    put(n, x, y, parent, depth, grow, pinned)
    placeKids(n, shown(n), grow, depth + 1)
  }
  // a two-sided root: explicit sides kept, the others balanced by extent in order
  const placeBoth = (root: TreeNode) => {
    const left: TreeNode[] = []
    const right: TreeNode[] = []
    let sumL = 0
    let sumR = 0
    for (const c of shown(root)) {
      if (pinnedOf(c)) { right.push(c); continue }
      const side = c.side ?? (sumR <= sumL ? 'right' : 'left')
      if (side === 'left') { left.push(c); sumL += extent(c, side) + sibling }
      else { right.push(c); sumR += extent(c, side) + sibling }
    }
    // pinned children go with the right side and choose their own by where they sit
    placeKids(root, right, 'right', 1)
    placeKids(root, left, 'left', 1)
  }
  const rootGrow: Grow = config.direction === 'left' ? 'left' : config.direction === 'down' ? 'down' : 'right'
  let cursor: number | null = null
  for (const root of roots) {
    const before = new Set(nodes.keys())
    if (pinnedOf(root)) {
      const off = root.offset ?? { x: 0, y: 0 }
      put(root, (root.x ?? 0) + off.x, (root.y ?? 0) + off.y, null, 0, rootGrow, true)
      if (config.direction === 'both') placeBoth(root)
      else placeKids(root, shown(root), rootGrow, 1)
      continue
    }
    put(root, 0, 0, null, 0, rootGrow, false)
    if (config.direction === 'both') placeBoth(root)
    else placeKids(root, shown(root), rootGrow, 1)
    // further roots line up after the extent of the previous trees
    const mine = [...nodes.values()].filter((l) => !before.has(l.id))
    const box = extentOf(mine)
    if (cursor === null) { cursor = config.direction === 'down' ? box.x + box.w : box.y + box.h; continue }
    const at: number = cursor
    const shift: { x: number; y: number } = config.direction === 'down' ? { x: at + level - box.x, y: 0 } : { x: 0, y: at + level - box.y }
    for (const l of mine) { l.x += shift.x; l.y += shift.y }
    cursor = config.direction === 'down' ? box.x + shift.x + box.w : box.y + shift.y + box.h
  }
  const all = extentOf([...nodes.values()])
  const bounds = nodes.size === 0 ? { x: 0, y: 0, w: 160, h: 64 } : { x: all.x - BOX_PADDING, y: all.y - BOX_PADDING, w: all.w + BOX_PADDING * 2, h: all.h + BOX_PADDING * 2 }
  return { nodes: ordered(roots, nodes), bounds }
}

function extentOf(rects: Rect[]): Rect {
  if (rects.length === 0) return { x: 0, y: 0, w: 0, h: 0 }
  let x0 = Infinity, y0 = Infinity, x1 = -Infinity, y1 = -Infinity
  for (const r of rects) { x0 = Math.min(x0, r.x); y0 = Math.min(y0, r.y); x1 = Math.max(x1, r.x + r.w); y1 = Math.max(y1, r.y + r.h) }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }
}

/** Pre-order (parents before children, siblings in order) whatever the side split placed first. */
function ordered(roots: TreeNode[], laid: Map<string, LaidNode>): Map<string, LaidNode> {
  const out = new Map<string, LaidNode>()
  const walk = (n: TreeNode) => {
    const l = laid.get(n.id)
    if (!l) return
    out.set(n.id, l)
    if (!n.collapsed) for (const c of n.children) walk(c)
  }
  for (const r of roots) walk(r)
  return out
}

/** Every node id of `roots` (folded subtrees included). */
export function allIds(roots: TreeNode[]): string[] {
  const out: string[] = []
  const walk = (n: TreeNode) => { out.push(n.id); for (const c of n.children) walk(c) }
  for (const r of roots) walk(r)
  return out
}
