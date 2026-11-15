/* What a free canvas draws for a wish (许愿格交互升级 TODO §3.9): computed from the wish's data and the layout, never
 * stored. An input is drawn from its Block on this canvas — the one nearest the wish; a table view input from that view —
 * or, when it has none here (another canvas, only in the data tree, a folder), from a ghost: a fixed square in a column
 * left of the wish, at most four (the fourth becomes "+N"). An input the reader cannot read, or that is gone, is always a
 * ghost with a warning and no name. A result is drawn to each of its Blocks here (none for a result elsewhere), or to their
 * group when the group holds more than six of them. A derived column (a result written into an input table) gets both
 * lines between the same two Blocks, set apart (`paired`). Pure: the view routes and paints what this returns. */

import type { EntityEnvelope, WishInput, WishResultBinding } from '../../../api/types'
import type { Laid } from '../layout'
import type { Rect } from '../render/camera'

/** A ghost's side, the space between ghosts and between the column and the wish (room for the arrow), world units. */
export const GHOST_SIZE = 72
export const GHOST_GAP = 12
export const GHOST_OFFSET = 56
/** Ghost squares in the column; more inputs than that fold the last ones into "+N". */
export const GHOSTS_SHOWN = 4
/** A group holding more result Blocks than this gets one line. */
export const GROUP_MERGE = 6

/** What the plan reads of a wish: its Block, its inputs, and the result bindings of its last run. */
export interface WishLinkData { cellId: string; inputs: readonly WishInput[]; results: readonly WishResultBinding[] }

export type LinkKind = 'input' | 'result'

export interface PlannedLink {
  /** Unique within the wish: the kind and the Block (or ghost) at the far end. */
  key: string
  kind: LinkKind
  /** A Block or group id of this canvas, or a ghost's key. */
  from: string
  to: string
  /** The input or result entity; null for "+N" and for a line standing for a group. */
  entityId: string | null
  /** A derived column: a line of the other kind joins the same two Blocks. */
  paired: boolean
  /** A line standing for a group: how many result Blocks it holds. */
  count?: number
}

export type GhostState = 'ok' | 'unavailable' | 'more' | 'less'

export interface Ghost {
  /** `@ghost:<entity>`, `@more`, `@less`: never an entity id (those start with a letter or digit). */
  key: string
  slot: number
  state: GhostState
  entityId: string | null
  /** The input, when the reader can read it. */
  entity?: EntityEnvelope
  /** "+N": the inputs it folds. */
  folded?: Ghost[]
}

export interface WishLinkPlan { links: PlannedLink[]; ghosts: Ghost[] }

export const ghostKey = (entityId: string) => `@ghost:${entityId}`
export const isGhostKey = (id: string) => id.startsWith('@')

/** A ghost's square: in the column left of the wish, from its top down. */
export function ghostRect(wish: Rect, slot: number): Rect {
  return { x: wish.x - GHOST_OFFSET - GHOST_SIZE, y: wish.y + slot * (GHOST_SIZE + GHOST_GAP), w: GHOST_SIZE, h: GHOST_SIZE }
}

/** The Blocks of this canvas by the entity they show (lines and groups show none). */
export function blocksBySource(laid: ReadonlyMap<string, Laid>): Map<string, string[]> {
  const out = new Map<string, string[]>()
  for (const [id, l] of laid) {
    const source = l.entity.source_id
    if (!source || l.isGroup || l.connector) continue
    const list = out.get(source)
    if (list) list.push(id)
    else out.set(source, [id])
  }
  return out
}

/** Edge-to-edge distance of two rectangles (0 when they touch or overlap). */
export function rectGap(a: Rect, b: Rect): number {
  const dx = Math.max(0, a.x - (b.x + b.w), b.x - (a.x + a.w))
  const dy = Math.max(0, a.y - (b.y + b.h), b.y - (a.y + a.h))
  return Math.hypot(dx, dy)
}

const centreGap = (a: Rect, b: Rect) => Math.hypot(a.x + a.w / 2 - b.x - b.w / 2, a.y + a.h / 2 - b.y - b.h / 2)

/** Of `ids`, the Block nearest `to`: edge to edge, then centre to centre. */
export function nearestBlock(laid: ReadonlyMap<string, Laid>, ids: readonly string[], to: Rect, skip: string): string | null {
  let best: string | null = null
  let bestGap = Infinity
  let bestCentre = Infinity
  for (const id of ids) {
    const l = id === skip ? undefined : laid.get(id)
    if (!l) continue
    const gap = rectGap(l.rect, to)
    const centre = centreGap(l.rect, to)
    if (gap < bestGap - 1e-6 || (Math.abs(gap - bestGap) <= 1e-6 && centre < bestCentre)) { best = id; bestGap = gap; bestCentre = centre }
  }
  return best
}

/** The lines and ghosts of one wish Block. `entityOf`: the outline's readable entity (undefined: unknown or unreadable). */
export function planWishLinks(data: WishLinkData, laid: ReadonlyMap<string, Laid>, bySource: ReadonlyMap<string, readonly string[]>,
  entityOf: (id: string) => EntityEnvelope | undefined, expanded: boolean): WishLinkPlan {
  const wish = laid.get(data.cellId)
  if (!wish) return { links: [], ghosts: [] }
  const links: PlannedLink[] = []
  const keys = new Set<string>()
  const add = (link: Omit<PlannedLink, 'key' | 'paired'>) => {
    const key = `${link.kind}:${link.kind === 'input' ? link.from : link.to}`
    if (keys.has(key)) return
    keys.add(key)
    links.push({ ...link, key, paired: false })
  }

  // inputs: a Block here, else a ghost (one per entity, in the order of the inputs)
  const linked = new Set<string>()
  const pending: Ghost[] = []
  for (const input of data.inputs) {
    const id = input.entity_id
    const entity = entityOf(id)
    const readable = Boolean(entity && !entity.deleted)
    let block: string | null = null
    if (readable) {
      const view = input.selector?.kind === 'table_view' ? (input.selector as { cell_id?: unknown }).cell_id : undefined
      block = typeof view === 'string' ? (view !== data.cellId && laid.has(view) ? view : null) : nearestBlock(laid, bySource.get(id) ?? [], wish.rect, data.cellId)
    }
    if (block) { add({ kind: 'input', from: block, to: data.cellId, entityId: id }); linked.add(id) }
    else if (!pending.some((g) => g.entityId === id)) pending.push({ key: ghostKey(id), slot: 0, state: readable ? 'ok' : 'unavailable', entityId: id, ...(readable ? { entity } : {}) })
  }

  // results: each Block here, a crowded group as one line
  const targets: { block: string; entityId: string }[] = []
  for (const binding of data.results) {
    let cells = (binding.cells ?? []).filter((c) => c !== data.cellId && laid.has(c))
    // a control result is its own container; derived columns are drawn to the Block their table is read from
    if (!cells.length && binding.type === 'control' && binding.entity_id !== data.cellId && laid.has(binding.entity_id)) cells = [binding.entity_id]
    if (!cells.length && binding.type === 'table_columns') {
      const b = links.find((l) => l.kind === 'input' && l.entityId === binding.entity_id)?.from ?? nearestBlock(laid, bySource.get(binding.entity_id) ?? [], wish.rect, data.cellId)
      if (b) cells = [b]
    }
    for (const block of cells) if (!targets.some((t) => t.block === block)) targets.push({ block, entityId: binding.entity_id })
  }
  const groupOf = (block: string) => { const parent = laid.get(block)?.parentId; return parent && laid.get(parent)?.isGroup ? parent : null }
  const perGroup = new Map<string, number>()
  for (const t of targets) { const g = groupOf(t.block); if (g) perGroup.set(g, (perGroup.get(g) ?? 0) + 1) }
  for (const t of targets) {
    const g = groupOf(t.block)
    const n = g ? perGroup.get(g) ?? 0 : 0
    if (g && n > GROUP_MERGE) add({ kind: 'result', from: data.cellId, to: g, entityId: null, count: n })
    else add({ kind: 'result', from: data.cellId, to: t.block, entityId: t.entityId })
  }

  // derived columns: an input Block that is also a result Block
  const inputBlocks = new Set(links.filter((l) => l.kind === 'input').map((l) => l.from))
  const resultBlocks = new Set(links.filter((l) => l.kind === 'result').map((l) => l.to))
  for (const l of links) l.paired = l.kind === 'input' ? resultBlocks.has(l.from) : inputBlocks.has(l.to)

  // the ghost column
  const missing = pending.filter((g) => !g.entityId || !linked.has(g.entityId))
  let column: Ghost[]
  if (missing.length > GHOSTS_SHOWN && !expanded) column = [...missing.slice(0, GHOSTS_SHOWN - 1), { key: '@more', slot: 0, state: 'more', entityId: null, folded: missing.slice(GHOSTS_SHOWN - 1) }]
  else if (missing.length > GHOSTS_SHOWN) column = [...missing, { key: '@less', slot: 0, state: 'less', entityId: null }]
  else column = missing
  const ghosts = column.map((g, slot) => ({ ...g, slot }))
  for (const g of ghosts) if (g.state !== 'less') add({ kind: 'input', from: g.key, to: data.cellId, entityId: g.entityId })
  return { links, ghosts }
}
