/* Composite controls (扩展控件 P1, K2–K4): a `buckyos.container { kind: "composite", def_ref }` lays out the Blocks of
 * its slots with an algorithm the host ships — `stack` (one after another, column or row), `grid` (P3: cells row by
 * row) or `fixed` (each keeps its x / y) — from its definition's `composition` (projected into the outline with the
 * definition's config defaults). An algorithm this host does not know lays the children out as a group (Q33).
 * Pure and single-direction like the tree: only `"auto"` components are filled, user Blocks are never created or
 * removed here. Shared by the canvas, the static exports and tests. */

import { isTextView } from './contract.ts'
import { NODE_PAD } from './measure.ts'
import { num } from './placement.ts'
import { solveControl, type ControlEntity, type ControlSource, type SolvedControl, type SolveOptions } from './solve.ts'
import { mindmapConfig, type LaidNode, type Rect } from './tree.ts'
import type { ControlDescriptor } from './catalog.ts'

export type SlotPlacement = 'stack' | 'top' | 'bottom' | 'free'

export interface CompositionSlot {
  /** `cell:<author_did>#<name>` (an instance of that control), a view type, or `container`; absent: any Block. */
  accepts?: string[]
  min?: number
  max?: number
  placement?: SlotPlacement
  title?: string
  /** (manifest 4, 控件机制 TODO G19) shown to each viewer for a while from their first opening: a presentation, not access
   * control — design mode, static views, prints and exports show it always, marked. */
  visible?: { from: 'first_open'; for_ms: number | string }
}

/** (manifest 4, 控件机制 TODO G2, G13, G21) Blocks projected from data — never stored: a template of label, shape and asset
 * nodes instantiated per item, laid out as a list or a grid in a region of the composite (placed like a slot). */
export interface CompositionProjection {
  source:
    /** The records of a bound table (`sort`: a field id, `-` in front descends). */
    | { kind: 'records'; binding: string; sort?: string; limit?: number }
    /** The members of a bound directory (`buckyos.dir`): `name`, `object_id`, `media_type`. */
    | { kind: 'members'; binding: string; accept?: 'image' | 'audio' | 'video' | 'any' }
    /** The elements of a list field of a bound table, counted (G21): `key`, `count`, most first; `of` joins the items of
     * another projection by key (a counted photo shows as that photo). */
    | { kind: 'counts'; binding: string; field: string; top?: number; of?: string }
    /** One item: the viewer (`principal`, `first_open`, `remaining_ms`, `expired`). */
    | { kind: 'viewer' }
  /** `item.<as>` is true when the viewer's own record (the submitter's, G20) of `binding` lists the item's key in `field`. */
  mark?: { binding: string; field: string; as: string }
  template: { nodes: unknown[] }
  layout: { algorithm: 'list' | 'grid'; columns?: number | string; gap?: number | string; item: { w?: number | string; h: number | string } }
  placement?: 'top' | 'stack' | 'bottom'
  /** At most this many items are shown (default 200). */
  max_items?: number
  /** A press on a node of this role raises the composite's event that names it (`events.<e>.from = { projection, role }`). */
}

/** (manifest 4, G17) The media Block of a slot plays as the instance's config says: `{ pos_ms, at_ms, playing }` in
 * `config.<anchor>` (position `pos_ms` at server time `at_ms`), looping when `config.<loop>` is true. */
export interface CompositionMedia { slot: string; anchor: string; loop?: string }

/** A composite's `composition.json` (README “Composite controls”): layout, slots, chrome. */
/** The layout algorithms this host ships (K2); a composition naming another one is laid out as a group. */
export const KNOWN_ALGORITHMS: readonly string[] = ['stack', 'grid', 'fixed']

export interface Composition {
  layout: {
    algorithm: 'stack' | 'grid' | 'fixed'
    direction?: 'column' | 'row'
    spacing?: number | string
    padding?: number | string | { t?: number | string; r?: number | string; b?: number | string; l?: number | string }
    /** `grid`: 1…12 columns, `gap` px between cells (default the spacing). */
    columns?: number | string
    gap?: number | string
  }
  slots: Record<string, CompositionSlot>
  chrome?: { background?: string; radius?: number | string; border?: string; title?: { text: string; slot?: string }; empty?: string }
  projections?: Record<string, CompositionProjection>
  media?: CompositionMedia
  events?: Record<string, { from?: { slot?: string; event?: string; projection?: string; role?: string }; title?: string; description?: string }>
}

const PROJECTION_MAX = 200

/** A projection's region and its items' rectangles, `count` items laid out from `origin` within `width`. */
export function projectionLayout(p: CompositionProjection, config: Record<string, unknown>, count: number, origin: { x: number; y: number }, width: number): { region: Rect; items: Rect[] } {
  const n = Math.max(0, Math.min(count, p.max_items ?? PROJECTION_MAX))
  const gap = dimension(p.layout?.gap, config, DEFAULT_SPACING)
  const h = dimension(p.layout?.item?.h, config, 40) || 40
  if (p.layout?.algorithm === 'grid') {
    const columns = Math.max(1, Math.min(12, Math.round(dimension(p.layout.columns, config, 3)) || 1))
    const w = dimension(p.layout.item?.w, config, 0) || Math.max(24, (width - gap * (columns - 1)) / columns)
    const items = Array.from({ length: n }, (_, i) => ({ x: origin.x + (i % columns) * (w + gap), y: origin.y + Math.floor(i / columns) * (h + gap), w, h }))
    // no items, no room (an empty cover takes no band)
    const rows = Math.ceil(n / columns)
    return { region: { x: origin.x, y: origin.y, w: columns * w + (columns - 1) * gap, h: rows ? rows * h + (rows - 1) * gap : 0 }, items }
  }
  const w = dimension(p.layout?.item?.w, config, 0) || width
  const items = Array.from({ length: n }, (_, i) => ({ x: origin.x, y: origin.y + i * (h + gap), w, h }))
  return { region: { x: origin.x, y: origin.y, w, h: n ? n * h + (n - 1) * gap : 0 }, items }
}

export const DEFAULT_SPACING = 8
export const DEFAULT_PADDING = 12
/** Height of the title band the Chrome draws at the top of a composite. */
export const TITLE_BAND = 28
/** The box of a composite with nothing in it (room for its `empty` text). */
export const EMPTY_BOX = { w: 240, h: 120 }

/** A composite's definition as the outline projects it, and the instance's effective config (instance over defaults). */
export function compositeOf(src: ControlSource, containerId: string): { composition: Composition | null; config: Record<string, unknown>; defId: string | null } {
  const container = src.get(containerId)
  const defId = container?.def_id ?? null
  const def = defId ? src.get(defId) : undefined
  const own = container?.config && typeof container.config === 'object' ? container.config as Record<string, unknown> : {}
  const defaults = def?.config_defaults && typeof def.config_defaults === 'object' ? def.config_defaults as Record<string, unknown> : {}
  const composition = def?.composition && typeof def.composition === 'object' ? def.composition as Composition : null
  return { composition, config: { ...defaults, ...own }, defId }
}

/** The algorithm of a composition when this host does not know it (null: known or no composition). */
export function unknownAlgorithm(composition: Composition | null): string | null {
  const algorithm = composition?.layout?.algorithm
  return algorithm && !KNOWN_ALGORITHMS.includes(algorithm) ? String(algorithm) : null
}

const WHOLE = /^\s*\{\{\s*config\.([A-Za-z0-9_]+)\s*\}\}\s*$/

/** A value of the composition with its `{{config.<key>}}` placeholders filled: a whole-placeholder string answers the
 * config value as it is (a number stays a number), other strings get the values interpolated. */
export function templated(value: unknown, config: Record<string, unknown>): unknown {
  if (typeof value !== 'string') return value
  const whole = WHOLE.exec(value)
  if (whole) return config[whole[1]]
  return value.replace(/\{\{\s*config\.([A-Za-z0-9_]+)\s*\}\}/g, (_, key: string) => (config[key] === undefined || config[key] === null ? '' : String(config[key])))
}

/** A dimension of the composition (`spacing`, `padding`, `radius`) as a number. */
export function dimension(value: unknown, config: Record<string, unknown>, fallback: number): number {
  const v = templated(value, config)
  const n = typeof v === 'number' ? v : typeof v === 'string' && v.trim() !== '' ? Number(v) : NaN
  return Number.isFinite(n) && n >= 0 ? Math.min(n, 2000) : fallback
}

export function paddingOf(composition: Composition | null, config: Record<string, unknown>): { t: number; r: number; b: number; l: number } {
  const p = composition?.layout.padding
  if (p && typeof p === 'object') {
    const side = (k: 't' | 'r' | 'b' | 'l') => dimension(p[k], config, DEFAULT_PADDING)
    return { t: side('t'), r: side('r'), b: side('b'), l: side('l') }
  }
  const all = dimension(p, config, DEFAULT_PADDING)
  return { t: all, r: all, b: all, l: all }
}

/** The slots in the order a stack lays them out: `top` ones, then `stack` ones, then `bottom` ones (each group in
 * the composition's order); `free` slots are not stacked. */
export function stackedSlots(composition: Composition): string[] {
  return stackedRegions(composition).filter((r) => r.kind === 'slot').map((r) => r.name)
}

/** Slots and projections in stacking order (a projection is placed like a slot; `#` marks it in the regions map). */
export function stackedRegions(composition: Composition): { kind: 'slot' | 'projection'; name: string }[] {
  const free = composition.layout.algorithm === 'fixed' || unknownAlgorithm(composition) !== null
  const slots = Object.entries(composition.slots ?? {}).map(([n, s]) => ({ kind: 'slot' as const, name: n, placement: s.placement ?? (free ? 'free' : 'stack') }))
  const projections = Object.entries(composition.projections ?? {}).map(([n, p]) => ({ kind: 'projection' as const, name: n, placement: p.placement ?? 'stack' }))
  const all = [...slots, ...projections]
  const by = (p: string) => all.filter((r) => r.placement === p).map(({ kind, name }) => ({ kind, name }))
  return [...by('top'), ...by('stack'), ...by('bottom')]
}

/** Is the title band drawn: a title, unless it belongs to a slot that holds something. */
export function showsTitle(composition: Composition | null, children: ControlEntity[]): boolean {
  const title = composition?.chrome?.title
  if (!title?.text) return false
  return !title.slot || !children.some((c) => c.slot === title.slot)
}

/** What a slot may take (README “Composite controls”): no `accepts` — any Block; `container` — a group or a control;
 * `cell:<ident>` — an instance of that control; else a view type. `identOf` names a definition's identity. */
export function slotAccepts(slot: CompositionSlot | undefined, block: { type_id: string; view_type?: string | null; def_id?: string | null }, identOf: (defId: string) => string | null | undefined): boolean {
  if (!slot) return false
  if (!slot.accepts || slot.accepts.length === 0) return true
  return slot.accepts.some((a) => {
    if (a === 'container') return block.type_id === 'buckyos.container'
    if (block.type_id !== 'buckyos.cell') return false
    if (a.startsWith('cell:')) return !!block.def_id && identOf(block.def_id) === a.slice(5)
    return block.view_type === a
  })
}

/** The first slot of a composition that takes `block` (and still has room, given `count` Blocks per slot). */
export function slotFor(composition: Composition, block: { type_id: string; view_type?: string | null; def_id?: string | null }, identOf: (defId: string) => string | null | undefined, count: (slot: string) => number = () => 0): string | null {
  for (const [name, slot] of Object.entries(composition.slots ?? {})) {
    if (slot.max !== undefined && count(name) >= slot.max) continue
    if (slotAccepts(slot, block, identOf)) return name
  }
  return null
}

const isBlock = (e: ControlEntity) => e.type_id === 'buckyos.cell' || (e.type_id === 'buckyos.container' && e.kind !== 'surface' && e.kind !== 'folder')

/** Lay out a composite: its children by slot, relative to its origin (the box's top-left). Children with numeric x / y
 * keep them (pinned); in a `fixed` layout every child is placed so (an `"auto"` one falls into a column below). */
export function solveComposite(src: ControlSource, containerId: string, opts: SolveOptions): SolvedControl {
  const { composition, config } = compositeOf(src, containerId)
  const fallback = opts.fallbackSize ?? (() => ({ w: 160, h: 90 }))
  const nested = new Map<string, SolvedControl>()
  const children = src.childrenOf(containerId).filter(isBlock)
  const pad = paddingOf(composition, config)
  const top = pad.t + (showsTitle(composition, children) ? TITLE_BAND : 0)
  // an algorithm this host does not know keeps every child where it is, as a group would (Q33)
  const fixed = composition?.layout.algorithm === 'fixed' || unknownAlgorithm(composition) !== null
  const grid = !fixed && composition?.layout.algorithm === 'grid'
  const row = !fixed && !grid && composition?.layout.direction === 'row'
  const spacing = dimension(composition?.layout.spacing, config, DEFAULT_SPACING)
  const columns = grid ? Math.max(1, Math.min(12, Math.round(dimension(composition?.layout.columns, config, 2)) || 1)) : 1
  const gap = grid ? dimension(composition?.layout.gap, config, spacing) : spacing
  const size = (e: ControlEntity): { w: number; h: number } => {
    const p = e.placement ?? {}
    if (e.type_id === 'buckyos.container' && (e.kind === 'composite' || e.kind === 'mindmap')) {
      const inner = e.kind === 'composite' ? solveComposite(src, e.entity_id, opts) : solveControl(src, e.entity_id, opts)
      nested.set(e.entity_id, inner)
      return { w: inner.layout.bounds.w, h: inner.layout.bounds.h }
    }
    let w = num(p.w)
    let h = num(p.h)
    if ((w === undefined || h === undefined) && e.type_id === 'buckyos.cell' && isTextView(e.view_type)) {
      const m = opts.measurer.measure(e.title, 1, w !== undefined ? Math.max(1, w - NODE_PAD.x * 2) : undefined)
      w ??= m.w
      h ??= m.h
    }
    const f = fallback(e)
    return { w: w ?? f.w, h: h ?? f.h }
  }
  const nodes = new Map<string, LaidNode>()
  const regions = new Map<string, Rect>()
  const place = (e: ControlEntity, x: number, y: number, s: { w: number; h: number }, pinned: boolean) =>
    nodes.set(e.entity_id, { id: e.entity_id, x, y, w: s.w, h: s.h, parent: null, depth: 0, grow: row ? 'down' : 'right', pinned, hasChildren: false, collapsed: false })
  // the stacked slots in order, then what sits in no stacked slot (a free slot, an unknown one)
  const order = composition ? stackedSlots(composition) : []
  const stacked = fixed ? [] : order.flatMap((slot) => children.filter((c) => c.slot === slot))
  const rest = children.filter((c) => !stacked.includes(c))
  if (grid) {
    // cells row by row in slot order: a column is as wide as its widest Block, a row as tall as its tallest
    const flowing = stacked.filter((e) => num(e.placement?.x) === undefined || num(e.placement?.y) === undefined)
    for (const e of stacked.filter((c) => !flowing.includes(c))) place(e, num(e.placement?.x)!, num(e.placement?.y)!, size(e), true)
    const sizes = flowing.map(size)
    const widths = Array.from({ length: columns }, (_, c) => Math.max(0, ...sizes.filter((_, i) => i % columns === c).map((s) => s.w)))
    const heights = Array.from({ length: Math.ceil(flowing.length / columns) }, (_, r) => Math.max(0, ...sizes.slice(r * columns, r * columns + columns).map((s) => s.h)))
    flowing.forEach((e, i) => {
      const c = i % columns
      const r = Math.floor(i / columns)
      const x = pad.l + widths.slice(0, c).reduce((a, w) => a + w + gap, 0)
      const y = top + heights.slice(0, r).reduce((a, h) => a + h + gap, 0)
      place(e, x, y, sizes[i], false)
    })
  } else {
    let cursor = row ? pad.l : top
    // the width projections fill: the composite's widest stacked Block, or a default
    const fill = Math.max(240, ...stacked.map((e) => size(e).w))
    const regionsInOrder = composition ? stackedRegions(composition) : []
    for (const r of regionsInOrder) {
      if (r.kind === 'projection') {
        const proj = composition!.projections![r.name]
        const count = opts.projectionCount?.(containerId, r.name) ?? 1
        const laid = projectionLayout(proj, config, count, row ? { x: cursor, y: top } : { x: pad.l, y: cursor }, fill)
        regions.set(r.name, laid.region)
        const extent = row ? laid.region.w : laid.region.h
        if (extent > 0) cursor += extent + spacing
        continue
      }
      for (const e of stacked.filter((c) => c.slot === r.name)) {
        const s = size(e)
        const p = e.placement ?? {}
        const px = num(p.x)
        const py = num(p.y)
        if (px !== undefined && py !== undefined) { place(e, px, py, s, true); continue }
        if (row) { place(e, px ?? cursor, py ?? top, s, false); cursor += s.w + spacing }
        else { place(e, px ?? pad.l, py ?? cursor, s, false); cursor += s.h + spacing }
      }
    }
  }
  // free children keep their place; one without a numeric x / y (e.g. written for a stack) drops into a column below
  const pinned = (e: ControlEntity) => num(e.placement?.x) !== undefined && num(e.placement?.y) !== undefined
  for (const e of rest.filter(pinned)) place(e, num(e.placement?.x)!, num(e.placement?.y)!, size(e), true)
  let below = Math.max(top, ...[...nodes.values()].map((n) => n.y + n.h + spacing))
  for (const e of rest.filter((c) => !pinned(c))) {
    const s = size(e)
    place(e, num(e.placement?.x) ?? pad.l, num(e.placement?.y) ?? below, s, false)
    below += s.h + spacing
  }
  const all = [...nodes.values(), ...regions.values()]
  const extent = all.reduce((b, n) => ({ x: Math.max(b.x, n.x + n.w), y: Math.max(b.y, n.y + n.h) }), { x: 0, y: 0 })
  const minW = all.length === 0 ? EMPTY_BOX.w : 0
  const minH = all.length === 0 ? EMPTY_BOX.h : 0
  const bounds: Rect = { x: 0, y: 0, w: Math.max(minW, extent.x + pad.r, pad.l + pad.r), h: Math.max(minH, extent.y + pad.b, top + pad.b) }
  return { id: containerId, config: mindmapConfig(undefined), layout: { nodes, bounds }, nested, regions }
}

/** The `composite` host kind for Agents (the catalogue): which control an instance is, is its definition's. */
export const COMPOSITE_CONTROL: ControlDescriptor = {
  kind: 'composite',
  title: 'Composite control',
  summary: 'The host of every composite extension control: a container whose definition (def_ref) lays out the Blocks of its slots and draws a frame around them.',
  when: 'To place an instance of a composite control a Workspace defines (data/extensions, kind composite); as a wish result it is a control result naming the definition (def) and filling its slots, never this kind.',
  formats: [],
  agent: [
    '# Composite controls',
    '',
    'A composite instance is `buckyos.container { kind: "composite", def_ref: { entity_id: <definition> }, config?, bindings?, layout: { mode: "def" } }`.',
    'Its definition (a `buckyos.block-def` with `kind: "composite"`) carries `composition { layout, slots, chrome }` and an AGENT.md',
    'self-description: read it first (`doc.read` the definition). Every child of the instance is an ordinary Block with `slot` set to a',
    'slot of the composition; several Blocks may fill one slot (the definition\'s `max` and `accepts` say how many and what).',
    '',
    'Placement: in a `stack` layout write `{ x: "auto", y: "auto", w, h }` on the children and `{ x, y, w: "auto", h: "auto" }` on the',
    'instance — the host computes the rest; a `fixed` layout needs numeric x / y. Never compute coordinates the control computes.',
    '',
    'Example (doc.commit): `{ op: "entity.create", entity_id: "b1", type_id: "buckyos.container", parent_id: <surface>, order_key: "m",',
    'placement: { x: 40, y: 40, w: "auto", h: "auto" }, payload: { kind: "composite", def_ref: { entity_id: <def> }, config: { title: "Today" } } }`,',
    'then `{ op: "entity.create", entity_id: "c1", type_id: "buckyos.cell", parent_id: "b1", order_key: "a", placement: { x: "auto", y: "auto", w: 240, h: 120 },',
    'payload: { view: { type: "html" }, def_ref: { entity_id: <card def> }, slot: "cards" } }`. Copy an instance with `tree.clone`',
    '(its children and own data come along, the definition is shared). Moving a Block in or out: `tree.move` with `slot`.',
    '`layout.algorithm` `grid` (columns, gap) fills cells row by row: children get `{ x: "auto", y: "auto", w, h }` as in a stack.',
    '',
    'Events: the definition\'s `events` (outline `def_events`) re-raise events of the slots\' controls (`from: { slot, event }`). An instance binds',
    'handlers in `on: { <event>: handler | [handlers] }` — `{ kind: "write", target: <binding>, what: recordSet | upsert | setMarkdown, args }`,',
    '`{ kind: "wish", wish_id }`, `{ kind: "open", target? }`, `{ kind: "notify", text }`, strings with `{{event.<p>}}` / `{{config.<k>}}`;',
    'they run when a person clicks in run mode. Change `config` / `bindings` / `on` one property at a time with `entity.set_props`.',
    '',
    'As a wish result: `aiws.result.control(name, { def: "<author_did>#<name>", config?, slots: { <slot>: ["result:<n>" | "input:<n>" |',
    '{ view: "richtext", markdown }] }, under?: { input, slot } })` — the other results\' Blocks, copies of input Blocks and new texts fill the slots.',
  ].join('\n'),
}
