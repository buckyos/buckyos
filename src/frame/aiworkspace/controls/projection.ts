/* Projected Blocks (控件机制 TODO G2, Q19): a data control declares a template — a small tree of label, shape (svg)
 * and asset (media) nodes whose values are `{{item.<field>}}` placeholders — and the host instantiates it once per
 * data item (a record, an occurrence, a directory member, a day). Projected Blocks are never stored: their identity
 * is derived from the control, the template node and the item, so it survives refreshes and re-layouts, and
 * selection, annotations, links and Agents address the record behind them. Shared by the Desktop, Deno and the
 * declarative composite opening (G13). */

export interface Rect { x: number; y: number; w: number; h: number }

export type ProjectedView = 'label' | 'shape' | 'asset'

/** A length inside the item's box: pixels, or `"<n>%"` of the box. */
export type Dim = number | string

/** Where a node sits in its item's box: `x` / `y` from the left / top, `w` / `h` its size, `r` / `b` insets from the
 * right / bottom (instead of `w` / `h`). Absent: the whole box. Values may be placeholders too (`{{item.pad}}`). */
export interface TemplateBox { x?: Dim; y?: Dim; w?: Dim; h?: Dim; r?: Dim; b?: Dim }

export interface TemplateNode {
  /** Stable id within the template (part of the projected Block's identity). */
  id: string
  view: ProjectedView
  box?: TemplateBox
  /** The Block's config; strings may hold `{{item.<field>}}`, `{{config.<key>}}`, `{{viewer.<key>}}`. */
  config?: Record<string, unknown>
  /** Shown only when this placeholder expression is truthy (`!` in front negates it). */
  when?: string
  /** What a press on this node means to the control's gestures (`body`, `check`, `resize-end`, …). */
  role?: string
  /** A label edited in place writes this field of the item's record. */
  edit?: string
}

export interface Template {
  nodes: TemplateNode[]
}

export interface ProjectedNode {
  /** `<control>~<node>~<item>`: stable across refreshes and re-layouts. */
  id: string
  /** The item (record id, or `<record>@<original>` for an occurrence). */
  item: string
  node: string
  view: ProjectedView
  rect: Rect
  config: Record<string, unknown>
  role?: string
  edit?: string
}

export const projectedId = (control: string, node: string, item: string) => `${control}~${node}~${item}`

/** The control, template node and item a projected id names. */
export function parseProjectedId(id: string): { control: string; node: string; item: string } | null {
  const a = id.indexOf('~')
  const b = a < 0 ? -1 : id.indexOf('~', a + 1)
  if (a <= 0 || b < 0) return null
  return { control: id.slice(0, a), node: id.slice(a + 1, b), item: id.slice(b + 1) }
}

const PLACEHOLDER = /\{\{\s*([a-z]+)\.([A-Za-z0-9_.-]+)\s*\}\}/g
const WHOLE = /^\{\{\s*([a-z]+)\.([A-Za-z0-9_.-]+)\s*\}\}$/

export type Scope = Record<string, unknown>

function lookup(scope: Scope, root: string, path: string): unknown {
  let cur: unknown = scope[root]
  for (const part of path.split('.')) {
    if (cur === null || typeof cur !== 'object') return undefined
    cur = (cur as Record<string, unknown>)[part]
  }
  return cur
}

/** Substitute placeholders: a string that is exactly one placeholder takes the value as it is (its type kept); a
 * placeholder inside text becomes text (absent values: empty). Objects and arrays are walked. */
export function resolveValue(value: unknown, scope: Scope): unknown {
  if (typeof value === 'string') {
    const whole = WHOLE.exec(value)
    if (whole) return lookup(scope, whole[1], whole[2])
    if (!value.includes('{{')) return value
    return value.replace(PLACEHOLDER, (_m, root: string, path: string) => {
      const v = lookup(scope, root, path)
      return v === undefined || v === null ? '' : typeof v === 'object' ? JSON.stringify(v) : String(v)
    })
  }
  if (Array.isArray(value)) return value.map((v) => resolveValue(v, scope))
  if (value && typeof value === 'object') return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, resolveValue(v, scope)]))
  return value
}

const truthy = (v: unknown) => !(v === undefined || v === null || v === false || v === '' || v === 0 || (Array.isArray(v) && v.length === 0))

export function holds(when: string | undefined, scope: Scope): boolean {
  if (!when) return true
  const negate = when.startsWith('!')
  const v = resolveValue(negate ? when.slice(1) : when, scope)
  return negate ? !truthy(v) : truthy(v)
}

function dim(d: Dim | undefined, size: number): number | undefined {
  if (d === undefined) return undefined
  if (typeof d === 'number') return d
  const m = /^(-?\d+(?:\.\d+)?)%$/.exec(d)
  return m ? (Number(m[1]) / 100) * size : Number(d) || 0
}

/** A node's rectangle inside an item's box (a right / bottom inset with a size anchors the node to that side). */
export function nodeRect(box: TemplateBox | undefined, item: Rect): Rect {
  if (!box) return { ...item }
  const axis = (start: Dim | undefined, size: Dim | undefined, inset: Dim | undefined, total: number): [number, number] => {
    const s = dim(start, total), z = dim(size, total), e = dim(inset, total)
    if (s === undefined && z !== undefined && e !== undefined) return [total - e - z, z]
    const from = s ?? 0
    return [from, z ?? Math.max(0, total - from - (e ?? 0))]
  }
  const [x, w] = axis(box.x, box.w, box.r, item.w)
  const [y, h] = axis(box.y, box.h, box.b, item.h)
  return { x: item.x + x, y: item.y + y, w, h }
}

/** One item's projected Blocks, in template order (later nodes are drawn on top). */
export function instantiate(control: string, template: Template, item: { key: string; scope: Scope }, box: Rect): ProjectedNode[] {
  const out: ProjectedNode[] = []
  for (const node of template.nodes) {
    if (!holds(node.when, item.scope)) continue
    out.push({
      id: projectedId(control, node.id, item.key), item: item.key, node: node.id, view: node.view,
      rect: nodeRect(node.box ? resolveValue(node.box, item.scope) as TemplateBox : undefined, box), config: (resolveValue(node.config ?? {}, item.scope) ?? {}) as Record<string, unknown>,
      ...(node.role ? { role: node.role } : {}), ...(node.edit ? { edit: node.edit } : {}),
    })
  }
  return out
}

/** Every item's projected Blocks, items visible in `view` only (G7: what is off screen is not instantiated). */
export function project(control: string, template: Template, items: { key: string; scope: Scope; rect: Rect }[], view?: Rect): ProjectedNode[] {
  const out: ProjectedNode[] = []
  for (const item of items) {
    if (view && (item.rect.x > view.x + view.w || item.rect.y > view.y + view.h || item.rect.x + item.rect.w < view.x || item.rect.y + item.rect.h < view.y)) continue
    out.push(...instantiate(control, template, item, item.rect))
  }
  return out
}
