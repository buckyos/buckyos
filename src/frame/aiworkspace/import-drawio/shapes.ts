/* The `shape` Block's geometry (draw.io 导入 TODO §2.1): the basic outlines, their look keys, and what line ends
 * meet. Shared by the canvas renderer, the importer and the exporter, so a shape draws, anchors and round-trips
 * the same everywhere.
 *
 *   kinds       rect … text: draw.io's built-in geometries (`rhombus` → diamond, `singleArrow` → arrow, …);
 *               `svg`: a vector body of its own (§2.4)
 *   frame       every outline is built pointing east in its own box, then turned by `direction` (s, w, n) and
 *               mirrored by `flip_h` / `flip_v`, as draw.io does; `shape_size` is draw.io's `size` (a fraction or
 *               world units, per kind)
 *   anchors     rect and ellipse keep their compass tables; any other outline gets the 16 compass ids where the
 *               ray from the centre towards the rectangle's spot leaves it; `config.anchors` adds a Block's own
 *   paths       cubic and quadratic pieces only (no arcs), so a turn or a mirror is a point map */

import { cleanAnchors, defaultAnchors, type AnchorDef, type Shape } from './anchors.ts'
import type { Point } from './geometry.ts'

export type ShapeKind =
  | 'rect' | 'ellipse' | 'diamond' | 'triangle' | 'hexagon' | 'parallelogram' | 'trapezoid' | 'cylinder' | 'cloud'
  | 'document' | 'process' | 'card' | 'note' | 'callout' | 'cross' | 'step' | 'arrow' | 'double_arrow' | 'line' | 'text' | 'svg'

export type Direction = 'e' | 's' | 'w' | 'n'
export type Dash = 'solid' | 'dashed' | 'dotted'
export type Align = 'left' | 'center' | 'right'
export type VAlign = 'top' | 'middle' | 'bottom'
export type LabelPosition = 'inside' | 'top' | 'bottom' | 'left' | 'right'

export interface KindInfo {
  kind: ShapeKind
  label: string
  group: 'basic' | 'flow' | 'arrow' | 'text'
  /** draw.io's default `size` for the kind, and whether it is a fraction of the length along the direction. */
  size?: { value: number; relative: boolean }
}

export const SHAPE_KINDS: readonly KindInfo[] = [
  { kind: 'rect', label: '矩形', group: 'basic' },
  { kind: 'ellipse', label: '椭圆', group: 'basic' },
  { kind: 'diamond', label: '菱形', group: 'basic' },
  { kind: 'triangle', label: '三角形', group: 'basic' },
  { kind: 'hexagon', label: '六边形', group: 'basic', size: { value: 0.25, relative: true } },
  { kind: 'parallelogram', label: '平行四边形', group: 'basic', size: { value: 0.2, relative: true } },
  { kind: 'trapezoid', label: '梯形', group: 'basic', size: { value: 0.2, relative: true } },
  { kind: 'cross', label: '十字', group: 'basic', size: { value: 0.2, relative: true } },
  { kind: 'cloud', label: '云', group: 'basic' },
  { kind: 'cylinder', label: '圆柱（数据库）', group: 'flow', size: { value: 15, relative: false } },
  { kind: 'document', label: '文档', group: 'flow', size: { value: 0.3, relative: true } },
  { kind: 'process', label: '预定义过程', group: 'flow', size: { value: 0.1, relative: true } },
  { kind: 'card', label: '卡片', group: 'flow', size: { value: 18, relative: false } },
  { kind: 'note', label: '折角便笺', group: 'flow', size: { value: 30, relative: false } },
  { kind: 'callout', label: '标注气泡', group: 'flow', size: { value: 30, relative: false } },
  { kind: 'step', label: '步骤（V 形）', group: 'arrow', size: { value: 0.2, relative: true } },
  { kind: 'arrow', label: '箭头', group: 'arrow' },
  { kind: 'double_arrow', label: '双向箭头', group: 'arrow' },
  { kind: 'line', label: '线条', group: 'arrow' },
  { kind: 'text', label: '文字', group: 'text' },
]

export const KIND_GROUPS: { group: KindInfo['group']; label: string }[] = [
  { group: 'basic', label: '基本图形' }, { group: 'flow', label: '流程图' }, { group: 'arrow', label: '箭头与线' }, { group: 'text', label: '文字' },
]

const KINDS = new Set<string>([...SHAPE_KINDS.map((k) => k.kind), 'svg'])
export const kindInfo = (kind: ShapeKind): KindInfo | undefined => SHAPE_KINDS.find((k) => k.kind === kind)

/** Defaults of a shape inserted by hand; an imported one writes its keys explicitly. */
export const DEFAULT_FILL = '#e8f0fe'
export const DEFAULT_STROKE = '#4f8df7'
export const DEFAULT_STROKE_WIDTH = 2
export const DEFAULT_FONT_SIZE = 14

export interface SvgBody { viewBox: string; body: string; aspect: 'stretch' | 'fixed' }

/** A shape's look, read wide from `config` (unknown or broken values fall back to the defaults). Colours are
 * `#rrggbb`, `none`, or `auto` (the theme's surface / ink: draw.io's `default`). */
export interface ShapeLook {
  kind: ShapeKind
  fill: string
  stroke: string
  strokeWidth: number
  dash: Dash
  opacity: number
  radius: number
  direction: Direction
  flipH: boolean
  flipV: boolean
  size: number | null
  text: string
  fontSize: number
  fontColor: string
  bold: boolean
  italic: boolean
  underline: boolean
  align: Align
  valign: VAlign
  labelPosition: LabelPosition
  wrap: boolean
  labelFill: string | null
  svg: SvgBody | null
}

type Config = Record<string, unknown> | null | undefined

const HEX = /^#[0-9a-f]{6}$/i
const finite = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)

function colorOf(v: unknown, fallback: string, allowNone = true): string {
  if (typeof v !== 'string') return fallback
  if (HEX.test(v)) return v.toLowerCase()
  if (v === 'auto' || (allowNone && v === 'none')) return v
  return fallback
}

export function readSvg(v: unknown): SvgBody | null {
  if (!v || typeof v !== 'object') return null
  const s = v as { viewBox?: unknown; body?: unknown; aspect?: unknown }
  if (typeof s.viewBox !== 'string' || !/^\s*-?[\d.]+(?:[\s,]+-?[\d.]+){3}\s*$/.test(s.viewBox) || typeof s.body !== 'string') return null
  return { viewBox: s.viewBox.trim(), body: s.body, aspect: s.aspect === 'fixed' ? 'fixed' : 'stretch' }
}

export function readLook(config: Config, title?: string | null): ShapeLook {
  const c = config ?? {}
  const kind = (typeof c.shape === 'string' && KINDS.has(c.shape) ? c.shape : 'rect') as ShapeKind
  const isText = kind === 'text'
  return {
    kind,
    fill: colorOf(c.fill, isText ? 'none' : DEFAULT_FILL),
    stroke: colorOf(c.stroke, isText ? 'none' : DEFAULT_STROKE),
    strokeWidth: finite(c.stroke_width) ? Math.min(48, Math.max(0, c.stroke_width)) : DEFAULT_STROKE_WIDTH,
    dash: c.dash === 'dashed' || c.dash === 'dotted' ? c.dash : 'solid',
    opacity: finite(c.opacity) ? Math.min(1, Math.max(0, c.opacity)) : 1,
    radius: finite(c.radius) ? Math.max(0, c.radius) : 0,
    direction: c.direction === 's' || c.direction === 'w' || c.direction === 'n' ? c.direction : 'e',
    flipH: c.flip_h === true,
    flipV: c.flip_v === true,
    size: finite(c.shape_size) ? Math.max(0, c.shape_size) : null,
    text: typeof c.text === 'string' ? c.text : (title ?? ''),
    fontSize: finite(c.font_size) ? Math.min(400, Math.max(4, c.font_size)) : DEFAULT_FONT_SIZE,
    fontColor: colorOf(c.font_color, '', false),
    bold: c.bold === true,
    italic: c.italic === true,
    underline: c.underline === true,
    align: c.align === 'left' || c.align === 'right' ? c.align : isText && c.align === undefined ? 'left' : 'center',
    valign: c.valign === 'top' || c.valign === 'bottom' ? c.valign : 'middle',
    labelPosition: c.label_position === 'top' || c.label_position === 'bottom' || c.label_position === 'left' || c.label_position === 'right' ? c.label_position : 'inside',
    wrap: c.wrap !== false,
    labelFill: typeof c.label_fill === 'string' && (HEX.test(c.label_fill) || c.label_fill === 'auto') ? c.label_fill : null,
    svg: kind === 'svg' ? readSvg(c.svg) : null,
  }
}

// ---- paths: built in the east frame (W × H), then mapped into the box (w × h)

type Cmd = { c: 'M'; p: Point } | { c: 'L'; p: Point } | { c: 'Q'; p1: Point; p: Point } | { c: 'C'; p1: Point; p2: Point; p: Point } | { c: 'Z' }
interface Built { body: Cmd[]; details: Cmd[][]; /** The body is an open stroke (a line), not a filled outline. */ open?: boolean }

const P = (x: number, y: number): Point => ({ x, y })

/** A circle-ish quarter is drawn with this cubic handle ratio. */
const K = 0.5522847498

function ellipseCmds(cx: number, cy: number, rx: number, ry: number): Cmd[] {
  return [
    { c: 'M', p: P(cx + rx, cy) },
    { c: 'C', p1: P(cx + rx, cy + ry * K), p2: P(cx + rx * K, cy + ry), p: P(cx, cy + ry) },
    { c: 'C', p1: P(cx - rx * K, cy + ry), p2: P(cx - rx, cy + ry * K), p: P(cx - rx, cy) },
    { c: 'C', p1: P(cx - rx, cy - ry * K), p2: P(cx - rx * K, cy - ry), p: P(cx, cy - ry) },
    { c: 'C', p1: P(cx + rx * K, cy - ry), p2: P(cx + rx, cy - ry * K), p: P(cx + rx, cy) },
    { c: 'Z' },
  ]
}

/** A closed polygon, its corners rounded by `r` (quadratic through each corner, never more than half a side). */
function polygon(points: Point[], r: number): Cmd[] {
  const n = points.length
  if (r <= 0 || n < 3) return [{ c: 'M', p: points[0] }, ...points.slice(1).map((p): Cmd => ({ c: 'L', p })), { c: 'Z' }]
  const cut = (from: Point, to: Point, k: number): Point => {
    const d = Math.hypot(to.x - from.x, to.y - from.y)
    const t = d > 1e-9 ? Math.min(k, d / 2) / d : 0
    return P(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t)
  }
  const out: Cmd[] = []
  for (let i = 0; i < n; i++) {
    const prev = points[(i + n - 1) % n]
    const cur = points[i]
    const next = points[(i + 1) % n]
    const a = cut(cur, prev, r)
    const b = cut(cur, next, r)
    out.push(i === 0 ? { c: 'M', p: a } : { c: 'L', p: a })
    out.push({ c: 'Q', p1: cur, p: b })
  }
  out.push({ c: 'Z' })
  return out
}

function sizeOf(look: Pick<ShapeLook, 'kind' | 'size'>): number {
  return look.size ?? kindInfo(look.kind)?.size?.value ?? 0
}

/** The outline and its extra strokes in the east frame. */
function build(look: Pick<ShapeLook, 'kind' | 'size' | 'radius'>, W: number, H: number): Built {
  const r = look.radius
  const s = sizeOf(look)
  const rel = (v: number, len: number) => Math.min(len / 2, Math.max(0, v <= 1 ? v * len : v))
  switch (look.kind) {
    case 'ellipse': return { body: ellipseCmds(W / 2, H / 2, W / 2, H / 2), details: [] }
    case 'diamond': return { body: polygon([P(W / 2, 0), P(W, H / 2), P(W / 2, H), P(0, H / 2)], r), details: [] }
    case 'triangle': return { body: polygon([P(0, 0), P(W, H / 2), P(0, H)], r), details: [] }
    case 'hexagon': { const d = rel(s, W); return { body: polygon([P(d, 0), P(W - d, 0), P(W, H / 2), P(W - d, H), P(d, H), P(0, H / 2)], r), details: [] } }
    case 'parallelogram': { const d = rel(s, W); return { body: polygon([P(d, 0), P(W, 0), P(W - d, H), P(0, H)], r), details: [] } }
    case 'trapezoid': { const d = rel(s, W); return { body: polygon([P(d, 0), P(W - d, 0), P(W, H), P(0, H)], r), details: [] } }
    case 'step': { const d = rel(s, W); return { body: polygon([P(0, 0), P(W - d, 0), P(W, H / 2), P(W - d, H), P(0, H), P(d, H / 2)], r), details: [] } }
    case 'cross': {
      const t = Math.min(W, H) * Math.min(0.5, Math.max(0, s <= 1 ? s : s / Math.min(W, H)))
      const x1 = (W - t) / 2, x2 = (W + t) / 2, y1 = (H - t) / 2, y2 = (H + t) / 2
      return { body: polygon([P(x1, 0), P(x2, 0), P(x2, y1), P(W, y1), P(W, y2), P(x2, y2), P(x2, H), P(x1, H), P(x1, y2), P(0, y2), P(0, y1), P(x1, y1)], r), details: [] }
    }
    case 'arrow': case 'double_arrow': {
      // draw.io singleArrow / doubleArrow: arrowWidth 0.3 of H for the shaft, arrowSize 0.2 of W for each head
      const shaft = H * 0.3
      const head = Math.min(W / 2, W * 0.2)
      const y1 = (H - shaft) / 2, y2 = (H + shaft) / 2
      const pts = look.kind === 'arrow'
        ? [P(0, y1), P(W - head, y1), P(W - head, 0), P(W, H / 2), P(W - head, H), P(W - head, y2), P(0, y2)]
        : [P(0, H / 2), P(head, 0), P(head, y1), P(W - head, y1), P(W - head, 0), P(W, H / 2), P(W - head, H), P(W - head, y2), P(head, y2), P(head, H)]
      return { body: polygon(pts, r), details: [] }
    }
    case 'cylinder': {
      const dy = Math.min(H / 2, s <= 1 ? s * H : s)
      const body: Cmd[] = [
        { c: 'M', p: P(0, dy) },
        { c: 'C', p1: P(0, -dy / 3), p2: P(W, -dy / 3), p: P(W, dy) },
        { c: 'L', p: P(W, H - dy) },
        { c: 'C', p1: P(W, H + dy / 3), p2: P(0, H + dy / 3), p: P(0, H - dy) },
        { c: 'Z' },
      ]
      return { body, details: [[{ c: 'M', p: P(0, dy) }, { c: 'C', p1: P(0, 2 * dy), p2: P(W, 2 * dy), p: P(W, dy) }]] }
    }
    case 'cloud': {
      // draw.io's cloud (mxCloud): one closed bezier path in a 1 × 1 box
      const x = (v: number) => v * W
      const y = (v: number) => v * H
      const body: Cmd[] = [
        { c: 'M', p: P(x(0.25), y(0.25)) },
        { c: 'C', p1: P(x(0.05), y(0.25)), p2: P(x(0), y(0.5)), p: P(x(0.16), y(0.55)) },
        { c: 'C', p1: P(x(0), y(0.66)), p2: P(x(0.18), y(0.9)), p: P(x(0.31), y(0.8)) },
        { c: 'C', p1: P(x(0.4), y(1)), p2: P(x(0.7), y(1)), p: P(x(0.8), y(0.8)) },
        { c: 'C', p1: P(x(1), y(0.8)), p2: P(x(1), y(0.6)), p: P(x(0.875), y(0.5)) },
        { c: 'C', p1: P(x(1), y(0.3)), p2: P(x(0.8), y(0.1)), p: P(x(0.625), y(0.2)) },
        { c: 'C', p1: P(x(0.5), y(0.05)), p2: P(x(0.3), y(0.05)), p: P(x(0.25), y(0.25)) },
        { c: 'Z' },
      ]
      return { body, details: [] }
    }
    case 'document': {
      const dy = H * Math.min(1, Math.max(0, s))
      const fy = 1.4
      const body: Cmd[] = [
        { c: 'M', p: P(0, 0) }, { c: 'L', p: P(W, 0) }, { c: 'L', p: P(W, H - dy / 2) },
        { c: 'Q', p1: P((W * 3) / 4, H - dy * fy), p: P(W / 2, H - dy / 2) },
        { c: 'Q', p1: P(W / 4, H - dy * (1 - fy)), p: P(0, H - dy / 2) },
        { c: 'Z' },
      ]
      return { body, details: [] }
    }
    case 'process': {
      const d = W * Math.min(0.5, Math.max(0, s <= 1 ? s : s / W))
      return { body: polygon([P(0, 0), P(W, 0), P(W, H), P(0, H)], r), details: [[{ c: 'M', p: P(d, 0) }, { c: 'L', p: P(d, H) }], [{ c: 'M', p: P(W - d, 0) }, { c: 'L', p: P(W - d, H) }]] }
    }
    case 'card': {
      const d = Math.min(W, H, s <= 1 ? s * Math.min(W, H) : s)
      return { body: polygon([P(d, 0), P(W, 0), P(W, H), P(0, H), P(0, d)], r), details: [] }
    }
    case 'note': {
      const d = Math.min(W, H, s <= 1 ? s * Math.min(W, H) : s)
      return { body: polygon([P(0, 0), P(W - d, 0), P(W, d), P(W, H), P(0, H)], r), details: [[{ c: 'M', p: P(W - d, 0) }, { c: 'L', p: P(W - d, d) }, { c: 'L', p: P(W, d) }]] }
    }
    case 'callout': {
      // draw.io callout: a box with a pointer of height `size` below, at `position` (0.5), base 20
      const dy = Math.min(H / 2, s <= 1 ? s * H : s)
      const base = Math.min(W / 2, 20)
      const px = W * 0.5
      const bottom = H - dy
      return { body: polygon([P(0, 0), P(W, 0), P(W, bottom), P(px + base / 2, bottom), P(px, H), P(px - base / 2, bottom), P(0, bottom)], r), details: [] }
    }
    case 'line': return { body: [{ c: 'M', p: P(0, H / 2) }, { c: 'L', p: P(W, H / 2) }], details: [], open: true }
    default: return { body: polygon([P(0, 0), P(W, 0), P(W, H), P(0, H)], Math.min(r, W / 2, H / 2)), details: [] }
  }
}

/** Point map from the east frame (W × H) into the box (w × h): turned by the direction, then mirrored. */
function mapper(look: Pick<ShapeLook, 'direction' | 'flipH' | 'flipV'>, w: number, h: number): { W: number; H: number; map: (p: Point) => Point } {
  const turned = look.direction === 's' || look.direction === 'n'
  const W = turned ? h : w
  const H = turned ? w : h
  const turn = (p: Point): Point => {
    switch (look.direction) {
      case 's': return P(H - p.y, p.x)
      case 'w': return P(W - p.x, H - p.y)
      case 'n': return P(p.y, W - p.x)
      default: return p
    }
  }
  return { W, H, map: (p) => { const q = turn(p); return P(look.flipH ? w - q.x : q.x, look.flipV ? h - q.y : q.y) } }
}

const fmt = (n: number) => String(Math.round(n * 100) / 100)

function serialize(cmds: Cmd[], map: (p: Point) => Point): string {
  let d = ''
  for (const cmd of cmds) {
    if (cmd.c === 'Z') { d += 'Z'; continue }
    const p = map(cmd.p)
    if (cmd.c === 'M' || cmd.c === 'L') d += `${cmd.c}${fmt(p.x)} ${fmt(p.y)}`
    else if (cmd.c === 'Q') { const a = map(cmd.p1); d += `Q${fmt(a.x)} ${fmt(a.y)} ${fmt(p.x)} ${fmt(p.y)}` }
    else { const a = map(cmd.p1); const b = map(cmd.p2); d += `C${fmt(a.x)} ${fmt(a.y)} ${fmt(b.x)} ${fmt(b.y)} ${fmt(p.x)} ${fmt(p.y)}` }
  }
  return d
}

export interface ShapePaths {
  /** The outline (filled and stroked; only stroked when `open`). */
  body: string
  /** Extra strokes drawn over it (a cylinder's front rim, a process's bars, a note's fold). */
  details: string[]
  open: boolean
}

/** The SVG path data of a shape drawn in a `w × h` box (world units). `svg` and `text` have no outline here. */
export function shapePaths(look: Pick<ShapeLook, 'kind' | 'size' | 'radius' | 'direction' | 'flipH' | 'flipV'>, w: number, h: number): ShapePaths {
  const { W, H, map } = mapper(look, w, h)
  const built = build(look, W, H)
  return { body: serialize(built.body, map), details: built.details.map((d) => serialize(d, map)), open: built.open === true }
}

/** The outline flattened to a polygon in the unit box (cubics and quadratics sampled). */
function flatten(cmds: Cmd[], map: (p: Point) => Point, w: number, h: number): Point[] {
  const out: Point[] = []
  let cur = P(0, 0)
  const push = (p: Point) => { const q = map(p); out.push(P(Math.round((q.x / w) * 1e6) / 1e6, Math.round((q.y / h) * 1e6) / 1e6)) }
  for (const cmd of cmds) {
    if (cmd.c === 'Z') continue
    if (cmd.c === 'M' || cmd.c === 'L') { push(cmd.p); cur = cmd.p; continue }
    const steps = 8
    for (let k = 1; k <= steps; k++) {
      const t = k / steps
      const u = 1 - t
      if (cmd.c === 'Q') push(P(u * u * cur.x + 2 * u * t * cmd.p1.x + t * t * cmd.p.x, u * u * cur.y + 2 * u * t * cmd.p1.y + t * t * cmd.p.y))
      else push(P(u * u * u * cur.x + 3 * u * u * t * cmd.p1.x + 3 * u * t * t * cmd.p2.x + t * t * t * cmd.p.x, u * u * u * cur.y + 3 * u * u * t * cmd.p1.y + 3 * u * t * t * cmd.p2.y + t * t * t * cmd.p.y))
    }
    cur = cmd.p
  }
  return out
}

/** Outlines with absolute parts (a note's fold, a cylinder's rim) are measured at this size for the anchors. */
const NOMINAL = { w: 160, h: 120 }

const outlineCache = new Map<string, Shape>()

/** The outline line ends meet: rect and ellipse as such, every other kind as its polygon. */
export function shapeOutline(look: Pick<ShapeLook, 'kind' | 'size' | 'radius' | 'direction' | 'flipH' | 'flipV'>): Shape {
  if (look.kind === 'ellipse' || look.kind === 'cloud') return 'ellipse'
  if (look.kind === 'rect' || look.kind === 'text' || look.kind === 'line' || look.kind === 'svg' || look.kind === 'process' || look.kind === 'cylinder') return 'rect'
  const key = `${look.kind}|${look.size}|${look.direction}|${look.flipH}|${look.flipV}`
  let shape = outlineCache.get(key)
  if (!shape) {
    const { W, H, map } = mapper(look, NOMINAL.w, NOMINAL.h)
    shape = { poly: flatten(build({ ...look, radius: 0 }, W, H).body, map, NOMINAL.w, NOMINAL.h) }
    outlineCache.set(key, shape)
  }
  return shape
}

/** The anchors of a shape: its outline's compass set plus its own (`config.anchors`; an own id replaces a compass one). */
export function shapeAnchors(config: Config, outline: Shape): readonly AnchorDef[] {
  const base = defaultAnchors(outline)
  const own = cleanAnchors((config ?? {}).anchors)
  if (!own || own.length === 0) return base
  const ids = new Set(own.map((a) => a.id))
  return [...own, ...base.filter((a) => !ids.has(a.id))]
}

/** Where the text goes: the whole box inside, or a box of the same size on one side outside it. */
export function labelBox(look: Pick<ShapeLook, 'labelPosition'>, w: number, h: number): { x: number; y: number; w: number; h: number } {
  switch (look.labelPosition) {
    case 'top': return { x: 0, y: -h, w, h }
    case 'bottom': return { x: 0, y: h, w, h }
    case 'left': return { x: -w, y: 0, w, h }
    case 'right': return { x: w, y: 0, w, h }
    default: return { x: 0, y: 0, w, h }
  }
}

/** Dash pattern of a stroke of width `width` (world units). */
export function dashArray(dash: Dash, width: number): string | null {
  const w = Math.max(width, 1)
  if (dash === 'dashed') return `${fmt(w * 3)} ${fmt(w * 3)}`
  if (dash === 'dotted') return `${fmt(w)} ${fmt(w * 2)}`
  return null
}

/** Dark text on a light fill, light text on a dark one; `null` when the fill does not decide it. */
export function inkOn(fill: string): string | null {
  const m = /^#([0-9a-f]{6})$/i.exec(fill)
  if (!m) return null
  const n = parseInt(m[1], 16)
  const luminance = (0.299 * (n >> 16) + 0.587 * ((n >> 8) & 255) + 0.114 * (n & 255)) / 255
  return luminance > 0.6 ? '#1f2328' : '#ffffff'
}
