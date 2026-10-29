/* Vector graphics (PDF 导入方案 §4.3): a filled or stroked path becomes a standard shape where it is one — an
 * axis-aligned rectangle, a two-point line, a circle or ellipse — and an SVG shape otherwise; neighbouring paths of
 * one drawing (an icon, a chart) share one SVG shape. Clips that cut a path become `clipPath`s, axial and radial
 * shadings `linearGradient` / `radialGradient`; patterns, mesh shadings, soft masks and blend modes stay in the
 * raster (`interpret.ts` marks them).
 *
 *   rect      fill / stroke / width / dash / opacity; a fill-only one cut by rectangular clips is cut here
 *   line      a box as long as the segment, turned by its angle
 *   svg       paths in world units relative to the box; the most used fill and stroke colours follow the shape's
 *             fill and stroke (CSS variables), so the drawing can be recoloured as one
 *   joining   paths join the open SVG shape while they are of one form, at most 4 pt apart and the body stays under
 *             48 KiB; a large standard shape, a picture or text in between closes it; small standard shapes (an
 *             icon's parts) join too, thin rules (table lines, underlines) never do */

import type { Json } from './data.ts'
import { rectOf, type Clip, type Mat, type Paint, type PathMark, type Seg } from './interpret.ts'
import type { Rect } from '../import-drawio/geometry.ts'

export const SVG_BODY_LIMIT = 48 * 1024

export type PrimKind = 'rect' | 'line' | 'ellipse'

export interface Candidate {
  mark: PathMark
  /** The standard shape it can be, or null (SVG only). */
  prim: PrimKind | null
  /** World box of what is drawn (a cut rectangle: the cut). */
  box: Rect
  /** Clips that cut it (those containing it are dropped). */
  clips: Clip[]
  small: boolean
}

export interface ShapeObject { order: number; ops: number[]; rect: Rect; rotation: number; config: Record<string, Json>; kind: PrimKind | 'svg' }

const r2 = (v: number) => Math.round(v * 100) / 100
const inside = (a: Rect, b: Rect, tol: number) => a.x >= b.x - tol && a.y >= b.y - tol && a.x + a.w <= b.x + b.w + tol && a.y + a.h <= b.y + b.h + tol
export function intersect(a: Rect, b: Rect): Rect | null {
  const x = Math.max(a.x, b.x), y = Math.max(a.y, b.y)
  const x2 = Math.min(a.x + a.w, b.x + b.w), y2 = Math.min(a.y + a.h, b.y + b.h)
  return x2 > x && y2 > y ? { x, y, w: x2 - x, h: y2 - y } : null
}
const union = (a: Rect, b: Rect): Rect => {
  const x = Math.min(a.x, b.x), y = Math.min(a.y, b.y)
  return { x, y, w: Math.max(a.x + a.w, b.x + b.w) - x, h: Math.max(a.y + a.h, b.y + b.h) - y }
}
const gapBetween = (a: Rect, b: Rect) => Math.max(0, Math.max(a.x, b.x) - Math.min(a.x + a.w, b.x + b.w), Math.max(a.y, b.y) - Math.min(a.y + a.h, b.y + b.h))

const color = (p: Paint | null) => (p?.kind === 'color' ? p.color : null)
const painted = (p: Paint | null) => p !== null && p.kind !== 'none'

/** Four cubic quarters meeting the box's side midpoints with circle handles (~0.552). */
function isEllipse(segs: Seg[], box: Rect): boolean {
  const curves = segs.filter((s) => s.c === 'C')
  if (curves.length !== 4 || segs.filter((s) => s.c === 'M').length !== 1 || segs.some((s) => s.c === 'L')) return false
  if (box.w < 1 || box.h < 1) return false
  const cx = box.x + box.w / 2, cy = box.y + box.h / 2
  const tol = Math.max(box.w, box.h) * 0.02 + 0.1
  const onMid = (x: number, y: number) => (Math.abs(x - cx) < tol && (Math.abs(y - box.y) < tol || Math.abs(y - box.y - box.h) < tol)) || (Math.abs(y - cy) < tol && (Math.abs(x - box.x) < tol || Math.abs(x - box.x - box.w) < tol))
  return curves.every((s) => s.c === 'C' && onMid(s.p.x, s.p.y) && Math.abs(s.p1.x - s.p2.x) > tol / 2 && Math.abs(s.p1.y - s.p2.y) > tol / 2)
}

function lineOf(segs: Seg[]): [number, number, number, number] | null {
  const pts = segs.filter((s) => s.c !== 'Z')
  if (pts.length !== 2 || pts[0].c !== 'M' || pts[1].c !== 'L' || segs.some((s) => s.c === 'Z')) return null
  return [pts[0].p.x, pts[0].p.y, pts[1].p.x, pts[1].p.y]
}

/** A path as a candidate: which standard shape it can be, what cuts it. `unit` is world units per point. */
export function candidateOf(mark: PathMark, unit: number): Candidate | null {
  if (!painted(mark.fill) && !painted(mark.stroke)) return null
  const pad = mark.stroke && painted(mark.stroke) ? mark.lineWidth / 2 : 0
  let box: Rect = { x: mark.box.x - pad, y: mark.box.y - pad, w: mark.box.w + 2 * pad, h: mark.box.h + 2 * pad }
  // a rectangular clip around the whole path cuts nothing
  const clips = mark.clips.filter((c) => !(c.rect && inside(box, c.box, 0.5)))
  let prim: PrimKind | null = null
  const solid = (!painted(mark.fill) || mark.fill?.kind === 'color') && (!painted(mark.stroke) || mark.stroke?.kind === 'color')
  const rect = rectOf(mark.segs)
  if (solid && !mark.shading && rect) {
    if (clips.length === 0) prim = 'rect'
    else if (!painted(mark.stroke) && clips.every((c) => c.rect)) {
      let cut: Rect | null = box
      for (const c of clips) cut = cut && intersect(cut, c.box)
      if (!cut) return null
      box = cut
      prim = 'rect'
    }
  } else if (solid && !mark.shading && clips.length === 0 && !painted(mark.fill) && lineOf(mark.segs)) prim = 'line'
  else if (solid && !mark.shading && clips.length === 0 && isEllipse(mark.segs, mark.box)) prim = 'ellipse'
  const thin = Math.min(box.w, box.h) < 2.5 * unit && Math.max(box.w, box.h) > 8 * unit
  const small = !thin && Math.max(box.w, box.h) < 48 * unit
  return { mark, prim, box, clips: prim ? [] : clips, small }
}

function dashKey(mark: PathMark): Record<string, Json> {
  if (!mark.dash.length || mark.dash.every((d) => d === 0)) return {}
  return { dash: mark.dash[0] <= Math.max(1.5 * mark.lineWidth, 0.5) ? 'dotted' : 'dashed' }
}

function opacityKey(mark: PathMark): Record<string, Json> {
  const a = painted(mark.fill) && painted(mark.stroke) ? Math.min(mark.fillAlpha, mark.strokeAlpha) : painted(mark.fill) ? mark.fillAlpha : mark.strokeAlpha
  return a < 0.995 ? { opacity: r2(Math.max(0, a)) } : {}
}

/** A candidate drawn as its standard shape. */
export function primitiveObject(c: Candidate, source: Record<string, Json>): ShapeObject {
  const m = c.mark
  const fill = painted(m.fill) ? color(m.fill)! : 'none'
  const stroke = painted(m.stroke) ? color(m.stroke)! : 'none'
  const width = r2(Math.max(0.1, Math.min(48, m.lineWidth)))
  const look: Record<string, Json> = { stroke, stroke_width: stroke === 'none' ? 0 : width, ...dashKey(m), ...opacityKey(m) }
  if (c.prim === 'line') {
    const [x1, y1, x2, y2] = lineOf(m.segs)!
    const len = Math.hypot(x2 - x1, y2 - y1)
    const h = Math.max(1, width)
    // a line has no direction: its angle in [0, 180)
    let rotation = r2(((((Math.atan2(y2 - y1, x2 - x1) * 180) / Math.PI) % 180) + 180) % 180)
    if (rotation >= 180) rotation = 0
    return { order: m.index, ops: [m.index], kind: 'line', rect: { x: (x1 + x2) / 2 - len / 2, y: (y1 + y2) / 2 - h / 2, w: Math.max(1, len), h }, rotation, config: { shape: 'line', fill: 'none', ...look, pdf_source: { ...source, kind: 'line' } } }
  }
  // a stroke is centred on the outline: the box is the path's own
  const rect = c.prim === 'rect' && !painted(m.stroke) ? c.box : m.box
  return { order: m.index, ops: [m.index], kind: c.prim!, rect, rotation: 0, config: { shape: c.prim!, fill, ...look, pdf_source: { ...source, kind: c.prim! } } }
}

// ---- SVG shapes

const num = (v: number, digits: number) => {
  const s = v.toFixed(digits)
  return s.includes('.') ? s.replace(/0+$/, '').replace(/\.$/, '') : s
}

function pathData(segs: Seg[], ox: number, oy: number, digits: number): string {
  const p = (x: number, y: number) => `${num(x - ox, digits)} ${num(y - oy, digits)}`
  return segs.map((s) => (s.c === 'M' ? `M${p(s.p.x, s.p.y)}` : s.c === 'L' ? `L${p(s.p.x, s.p.y)}` : s.c === 'C' ? `C${p(s.p1.x, s.p1.y)} ${p(s.p2.x, s.p2.y)} ${p(s.p.x, s.p.y)}` : 'Z')).join('')
}

export interface SvgGroup { members: Candidate[]; box: Rect; form: number; size: number }

const estimate = (c: Candidate) => c.mark.segs.length * 26 + 120 + c.clips.reduce((n, k) => n + k.segs.length * 22 + 60, 0)

/** May `c` join the open SVG shape `g`? */
export function canJoin(g: SvgGroup, c: Candidate, unit: number): boolean {
  if (c.prim && !c.small) return false
  if (g.members.some((m) => m.prim && !m.small)) return false
  if (c.mark.form !== g.form) return false
  if (g.size + estimate(c) > SVG_BODY_LIMIT * 0.9) return false
  const reach = (c.small && c.box.w * c.box.h < 4 * unit * unit ? 8 : 4) * unit
  return gapBetween(g.box, c.box) <= reach
}

export function openGroup(c: Candidate): SvgGroup {
  return { members: [c], box: c.box, form: c.mark.form, size: estimate(c) }
}

export function joinGroup(g: SvgGroup, c: Candidate) {
  g.members.push(c)
  g.box = union(g.box, c.box)
  g.size += estimate(c)
}

const fmtMatrix = (m: Mat, ox: number, oy: number) => `matrix(${[m[0], m[1], m[2], m[3], m[4] - ox, m[5] - oy].map((v) => num(v, 4)).join(' ')})`

/** An SVG group as one `shape: svg` object, or null when its body would not fit. `patterns` resolves shadings;
 * `ids` makes element ids unique in the page that shows many shapes. */
export function svgObject(g: SvgGroup, patterns: Record<string, Json>, ids: string, source: Record<string, Json>): ShapeObject | null {
  const box = g.box
  const w = Math.max(1, box.w), h = Math.max(1, box.h)
  const digits = Math.max(w, h) > 400 ? 1 : 2
  // the most used colours follow the shape's fill and stroke
  const count = (key: 'fill' | 'stroke') => {
    const n = new Map<string, number>()
    for (const c of g.members) { const v = color(c.mark[key]); if (v) n.set(v, (n.get(v) ?? 0) + 1) }
    return [...n].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null
  }
  const mainFill = count('fill')
  const mainStroke = count('stroke')
  const defs: string[] = []
  const clipIds = new Map<Clip, string>()
  let next = 0
  const paint = (p: Paint | null, main: string | null, variable: string): string => {
    if (!p || p.kind === 'none') return 'none'
    if (p.kind === 'color') return p.color === main ? `var(${variable})` : p.color
    if (p.kind === 'shading') {
      const ir = patterns[p.id] as Json[] | undefined
      if (!ir || ir[0] !== 'RadialAxial') return 'none'
      const id = `${ids}g${next++}`
      const stops = ((ir[3] as [number, string][]) ?? []).map(([t, c]) => `<stop offset="${num(Math.min(1, Math.max(0, t)), 4)}" stop-color="${/^#[0-9a-f]{6}$/i.test(c) ? c : '#000000'}"/>`).join('')
      const p0 = (ir[4] as number[]) ?? [0, 0], p1 = (ir[5] as number[]) ?? [0, 0]
      const t = fmtMatrix(p.matrix, box.x, box.y)
      if (ir[1] === 'axial') defs.push(`<linearGradient id="${id}" gradientUnits="userSpaceOnUse" x1="${num(p0[0], 3)}" y1="${num(p0[1], 3)}" x2="${num(p1[0], 3)}" y2="${num(p1[1], 3)}" gradientTransform="${t}">${stops}</linearGradient>`)
      else defs.push(`<radialGradient id="${id}" gradientUnits="userSpaceOnUse" fx="${num(p0[0], 3)}" fy="${num(p0[1], 3)}" fr="${num(Number(ir[6] ?? 0), 3)}" cx="${num(p1[0], 3)}" cy="${num(p1[1], 3)}" r="${num(Number(ir[7] ?? 0), 3)}" gradientTransform="${t}">${stops}</radialGradient>`)
      return `url(#${id})`
    }
    return 'none'
  }
  const parts: string[] = []
  for (const c of g.members) {
    const m = c.mark
    const attrs = [`d="${pathData(m.segs, box.x, box.y, digits)}"`]
    attrs.push(`fill="${paint(m.fill, mainFill, '--aiws-shape-fill')}"`)
    if (m.evenOdd && painted(m.fill)) attrs.push('fill-rule="evenodd"')
    if (painted(m.fill) && m.fillAlpha < 0.995) attrs.push(`fill-opacity="${num(m.fillAlpha, 3)}"`)
    if (painted(m.stroke)) {
      attrs.push(`stroke="${paint(m.stroke, mainStroke, '--aiws-shape-stroke')}"`, `stroke-width="${num(Math.max(0.1, m.lineWidth), 2)}"`)
      if (m.dash.length && m.dash.some((d) => d > 0)) attrs.push(`stroke-dasharray="${m.dash.map((d) => num(d, 2)).join(' ')}"`)
      if (m.strokeAlpha < 0.995) attrs.push(`stroke-opacity="${num(m.strokeAlpha, 3)}"`)
    }
    let el = `<path ${attrs.join(' ')}/>`
    for (const clip of [...c.clips].reverse()) {
      let id = clipIds.get(clip)
      if (!id) {
        id = `${ids}c${next++}`
        clipIds.set(clip, id)
        defs.push(`<clipPath id="${id}"><path d="${pathData(clip.segs, box.x, box.y, digits)}"${clip.evenOdd ? ' clip-rule="evenodd"' : ''}/></clipPath>`)
      }
      el = `<g clip-path="url(#${id})">${el}</g>`
    }
    parts.push(el)
  }
  const body = `${defs.length ? `<defs>${defs.join('')}</defs>` : ''}${parts.join('')}`
  if (body.length > SVG_BODY_LIMIT) return null
  const ops = g.members.map((c) => c.mark.index)
  return {
    order: Math.min(...ops), ops, kind: 'svg', rect: box, rotation: 0,
    config: { shape: 'svg', svg: { viewBox: `0 0 ${num(w, 2)} ${num(h, 2)}`, body, aspect: 'stretch' }, fill: mainFill ?? 'none', stroke: mainStroke ?? 'none', stroke_width: 1, pdf_source: { ...source, kind: 'svg' } },
  }
}
