/* Composite controls (扩展控件 P1, K2–K4): a `buckyos.container { kind: "composite", def_ref }` lays out the Blocks of
 * its slots with an algorithm the host ships — `stack` (one after another, column or row) or `fixed` (each keeps its
 * x / y) — from its definition's `composition` (projected into the outline with the definition's config defaults).
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
}

/** A composite's `composition.json` (README “Composite controls”): layout, slots, chrome. */
export interface Composition {
  layout: { algorithm: 'stack' | 'fixed'; direction?: 'column' | 'row'; spacing?: number | string; padding?: number | string | { t?: number | string; r?: number | string; b?: number | string; l?: number | string } }
  slots: Record<string, CompositionSlot>
  chrome?: { background?: string; radius?: number | string; border?: string; title?: { text: string; slot?: string }; empty?: string }
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
  const by = (p: SlotPlacement) => Object.entries(composition.slots ?? {}).filter(([, s]) => (s.placement ?? (composition.layout.algorithm === 'fixed' ? 'free' : 'stack')) === p).map(([n]) => n)
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
  const fixed = composition?.layout.algorithm === 'fixed'
  const row = !fixed && composition?.layout.direction === 'row'
  const spacing = dimension(composition?.layout.spacing, config, DEFAULT_SPACING)
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
  const place = (e: ControlEntity, x: number, y: number, s: { w: number; h: number }, pinned: boolean) =>
    nodes.set(e.entity_id, { id: e.entity_id, x, y, w: s.w, h: s.h, parent: null, depth: 0, grow: row ? 'down' : 'right', pinned, hasChildren: false, collapsed: false })
  // the stacked slots in order, then what sits in no stacked slot (a free slot, an unknown one)
  const order = composition ? stackedSlots(composition) : []
  const stacked = fixed ? [] : order.flatMap((slot) => children.filter((c) => c.slot === slot))
  const rest = children.filter((c) => !stacked.includes(c))
  let cursor = row ? pad.l : top
  for (const e of stacked) {
    const s = size(e)
    const p = e.placement ?? {}
    const px = num(p.x)
    const py = num(p.y)
    if (px !== undefined && py !== undefined) { place(e, px, py, s, true); continue }
    if (row) { place(e, px ?? cursor, py ?? top, s, false); cursor += s.w + spacing }
    else { place(e, px ?? pad.l, py ?? cursor, s, false); cursor += s.h + spacing }
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
  const all = [...nodes.values()]
  const extent = all.reduce((b, n) => ({ x: Math.max(b.x, n.x + n.w), y: Math.max(b.y, n.y + n.h) }), { x: 0, y: 0 })
  const minW = all.length === 0 ? EMPTY_BOX.w : 0
  const minH = all.length === 0 ? EMPTY_BOX.h : 0
  const bounds: Rect = { x: 0, y: 0, w: Math.max(minW, extent.x + pad.r, pad.l + pad.r), h: Math.max(minH, extent.y + pad.b, top + pad.b) }
  return { id: containerId, config: mindmapConfig(undefined), layout: { nodes, bounds }, nested }
}

/** The `composite` host kind for Agents (the catalogue): which control an instance is, is its definition's. */
export const COMPOSITE_CONTROL: ControlDescriptor = {
  kind: 'composite',
  title: 'Composite control',
  summary: 'The host of every composite extension control: a container whose definition (def_ref) lays out the Blocks of its slots and draws a frame around them.',
  when: 'To place an instance of a composite control a Workspace defines (data/extensions, kind composite); it is never a wish result kind.',
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
  ].join('\n'),
}
