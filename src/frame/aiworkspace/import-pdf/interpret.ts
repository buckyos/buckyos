/* One walk over a page's operator list (PDF 导入方案 §4.1 第 5 条, §4.2, §4.3): the graphics state pdf.js's canvas
 * keeps (matrix, colours, alpha, line width, dash, clips, text state, forms, groups, annotations, optional content)
 * and what each painting operator draws, in world coordinates of the page (top-left origin, y down, `scale` world
 * units per point). Nothing is decided here: `convert.ts` turns the marks into objects or leaves them to the raster.
 *
 *   paths   every filled or stroked path, a `sh` (shading fill) as its clip filled with the shading
 *   images  a picture drawn into the unit square: its four corners
 *   runs    every `showText`: where it starts and ends, its size, font, fill and rendering mode
 *   other   painting operators the converter never takes (masks, repeated pictures): always in the raster
 *
 * `complex` names why a mark cannot become an object (it is drawn with a soft mask, a blend mode, a tiling pattern,
 * a mesh shading, in a knockout group or an annotation); such marks stay in the raster. */

import { DRAW, OPS, type Json, type PdfPage } from './data.ts'
import type { Point, Rect } from '../import-drawio/geometry.ts'

export type Mat = [number, number, number, number, number, number]

export const IDENTITY: Mat = [1, 0, 0, 1, 0, 0]

/** `a · b`: apply `b`, then `a` (pdf.js `Util.transform`). */
export function mul(a: Mat, b: readonly number[]): Mat {
  return [
    a[0] * b[0] + a[2] * b[1], a[1] * b[0] + a[3] * b[1],
    a[0] * b[2] + a[2] * b[3], a[1] * b[2] + a[3] * b[3],
    a[0] * b[4] + a[2] * b[5] + a[4], a[1] * b[4] + a[3] * b[5] + a[5],
  ]
}

export function apply(m: Mat, x: number, y: number): Point {
  return { x: m[0] * x + m[2] * y + m[4], y: m[1] * x + m[3] * y + m[5] }
}

/** The mean length scale of a matrix (line widths). */
export const scaleOf = (m: Mat) => Math.sqrt(Math.abs(m[0] * m[3] - m[1] * m[2]))

export type Paint = { kind: 'color'; color: string } | { kind: 'shading'; id: string; matrix: Mat } | { kind: 'none' } | { kind: 'other'; why: string }

export type Seg = { c: 'M'; p: Point } | { c: 'L'; p: Point } | { c: 'C'; p1: Point; p2: Point; p: Point } | { c: 'Z' }

export interface Clip { segs: Seg[]; box: Rect; rect: boolean; evenOdd: boolean }

export interface PathMark {
  index: number
  segs: Seg[]
  box: Rect
  fill: Paint | null
  stroke: Paint | null
  evenOdd: boolean
  fillAlpha: number
  strokeAlpha: number
  lineWidth: number
  dash: number[]
  clips: Clip[]
  form: number
  /** A `sh` operator: the area of the clip filled with a shading. */
  shading: boolean
  complex: string | null
}

export interface ImageMark { index: number; ref: string; corners: [Point, Point, Point, Point]; clips: Clip[]; alpha: number; form: number; mcid: number | null; complex: string | null }

export interface TextRun {
  index: number
  start: Point
  end: Point
  /** World units per text space unit across the baseline: the font size as drawn. */
  size: number
  font: string
  fill: Paint | null
  alpha: number
  mode: number
  mcid: number | null
  complex: string | null
  /** Only spaces: draws nothing. */
  blank: boolean
}

export interface Interpreted { paths: PathMark[]; images: ImageMark[]; runs: TextRun[]; other: number[] }

interface State {
  ctm: Mat
  base: Mat
  fill: Paint
  stroke: Paint
  fillAlpha: number
  strokeAlpha: number
  lineWidth: number
  dash: number[]
  blend: string
  smask: boolean
  clips: Clip[]
  form: number
  complex: string | null
  font: string
  fontSize: number
  charSpacing: number
  wordSpacing: number
  hScale: number
  leading: number
  rise: number
  mode: number
  textMatrix: Mat
  x: number
  y: number
  lineX: number
  lineY: number
}

const BLACK: Paint = { kind: 'color', color: '#000000' }

function boxOfSegs(segs: Seg[]): Rect {
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity
  const add = (p: Point) => { x1 = Math.min(x1, p.x); y1 = Math.min(y1, p.y); x2 = Math.max(x2, p.x); y2 = Math.max(y2, p.y) }
  for (const s of segs) {
    if (s.c === 'Z') continue
    add(s.p)
    if (s.c === 'C') { add(s.p1); add(s.p2) }
  }
  if (!Number.isFinite(x1)) return { x: 0, y: 0, w: 0, h: 0 }
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 }
}

/** An axis-aligned rectangle (one closed subpath of four corners), or null. */
export function rectOf(segs: Seg[], eps = 0.05): Rect | null {
  const pts: Point[] = []
  let moves = 0
  for (const s of segs) {
    if (s.c === 'C') return null
    if (s.c === 'M') { moves += 1; if (moves > 1) return null }
    if (s.c !== 'Z') pts.push(s.p)
  }
  if (pts.length === 5 && Math.abs(pts[4].x - pts[0].x) < eps && Math.abs(pts[4].y - pts[0].y) < eps) pts.pop()
  if (pts.length !== 4) return null
  for (let i = 0; i < 4; i++) {
    const a = pts[i], b = pts[(i + 1) % 4]
    if (Math.abs(a.x - b.x) > eps && Math.abs(a.y - b.y) > eps) return null
  }
  const box = boxOfSegs(segs)
  return box.w > eps && box.h > eps ? box : null
}

function readPath(data: number[] | null, m: Mat): Seg[] {
  const segs: Seg[] = []
  if (!data) return segs
  for (let k = 0; k < data.length;) {
    const op = data[k++]
    if (op === DRAW.moveTo) { segs.push({ c: 'M', p: apply(m, data[k], data[k + 1]) }); k += 2 }
    else if (op === DRAW.lineTo) { segs.push({ c: 'L', p: apply(m, data[k], data[k + 1]) }); k += 2 }
    else if (op === DRAW.curveTo) { segs.push({ c: 'C', p1: apply(m, data[k], data[k + 1]), p2: apply(m, data[k + 2], data[k + 3]), p: apply(m, data[k + 4], data[k + 5]) }); k += 6 }
    else if (op === DRAW.quadraticCurveTo) {
      // as a cubic, from the current point
      const last = [...segs].reverse().find((s) => s.c !== 'Z') as Exclude<Seg, { c: 'Z' }> | undefined
      const p0 = last?.p ?? apply(m, data[k], data[k + 1])
      const q = apply(m, data[k], data[k + 1]), p = apply(m, data[k + 2], data[k + 3])
      segs.push({ c: 'C', p1: { x: p0.x + (2 / 3) * (q.x - p0.x), y: p0.y + (2 / 3) * (q.y - p0.y) }, p2: { x: p.x + (2 / 3) * (q.x - p.x), y: p.y + (2 / 3) * (q.y - p.y) }, p })
      k += 4
    } else if (op === DRAW.closePath) segs.push({ c: 'Z' })
    else break
  }
  return segs
}

const FILLS = new Set<number>([OPS.fill, OPS.eoFill, OPS.fillStroke, OPS.eoFillStroke, OPS.closeFillStroke, OPS.closeEOFillStroke, OPS.rawFillPath])
const STROKES = new Set<number>([OPS.stroke, OPS.closeStroke, OPS.fillStroke, OPS.eoFillStroke, OPS.closeFillStroke, OPS.closeEOFillStroke])
const EVEN_ODD = new Set<number>([OPS.eoFill, OPS.eoFillStroke, OPS.closeEOFillStroke])
const OTHER_PAINT = new Set<number>([OPS.paintImageMaskXObject, OPS.paintImageMaskXObjectGroup, OPS.paintImageMaskXObjectRepeat, OPS.paintImageXObjectRepeat, OPS.paintInlineImageXObjectGroup, OPS.paintSolidColorImageMask])

/** Walk the page's operators. `world` maps user space to world units (the viewport transform, scaled). */
export function interpret(page: PdfPage, world: Mat): Interpreted {
  const out: Interpreted = { paths: [], images: [], runs: [], other: [] }
  let st: State = {
    ctm: IDENTITY, base: IDENTITY, fill: BLACK, stroke: BLACK, fillAlpha: 1, strokeAlpha: 1, lineWidth: 1, dash: [], blend: 'source-over', smask: false,
    clips: [], form: 0, complex: null, font: '', fontSize: 0, charSpacing: 0, wordSpacing: 0, hScale: 1, leading: 0, rise: 0, mode: 0,
    textMatrix: IDENTITY, x: 0, y: 0, lineX: 0, lineY: 0,
  }
  const stack: State[] = []
  const marked: { mcid: number | null; hidden: boolean }[] = []
  let pendingClip: 'nonzero' | 'evenodd' | null = null
  let forms = 0
  const hidden = () => marked.some((m) => m.hidden)
  const mcid = () => { for (let i = marked.length - 1; i >= 0; i--) if (marked[i].mcid !== null) return marked[i].mcid; return null }
  const complexOf = (paint: Paint | null): string | null => {
    if (st.complex) return st.complex
    if (st.smask) return 'smask'
    if (st.blend !== 'source-over' && st.blend !== 'normal') return 'blend'
    if (paint?.kind === 'other') return paint.why
    return null
  }
  const paintOfShading = (id: string, matrix: Mat): Paint => {
    const ir = page.patterns[id] as Json[] | undefined
    return ir?.[0] === 'RadialAxial' && (ir[1] === 'axial' || ir[1] === 'radial') ? { kind: 'shading', id, matrix } : { kind: 'other', why: 'mesh' }
  }
  const save = () => { stack.push(st); st = { ...st } }
  const restore = () => { const prev = stack.pop(); if (prev) st = prev }
  const toWorld = () => mul(world, st.ctm)
  const moveText = (x: number, y: number) => { st.lineX += x; st.lineY += y; st.x = st.lineX; st.y = st.lineY }
  for (let i = 0; i < page.fnArray.length; i++) {
    const fn = page.fnArray[i]
    const args = (page.argsArray[i] ?? []) as Json[]
    switch (fn) {
      case OPS.save: save(); break
      case OPS.restore: restore(); break
      case OPS.transform: st.ctm = mul(st.ctm, args as number[]); break
      case OPS.setLineWidth: st.lineWidth = Number(args[0]) || 0; break
      case OPS.setDash: st.dash = ((args[0] as number[]) ?? []).map(Number); break
      case OPS.setGState:
        for (const [key, value] of (args[0] as [string, Json][]) ?? []) {
          if (key === 'LW') st.lineWidth = Number(value) || 0
          else if (key === 'D') st.dash = (((value as Json[])?.[0] as number[]) ?? []).map(Number)
          else if (key === 'CA') st.strokeAlpha = Number(value ?? 1)
          else if (key === 'ca') st.fillAlpha = Number(value ?? 1)
          else if (key === 'BM') st.blend = String(value ?? 'source-over')
          else if (key === 'SMask') st.smask = value === true
          else if (key === 'Font') { st.font = String((value as Json[])[0]); st.fontSize = Number((value as Json[])[1]) || 0 }
        }
        break
      case OPS.setFillRGBColor: st.fill = { kind: 'color', color: String(args[0]).toLowerCase() }; break
      case OPS.setStrokeRGBColor: st.stroke = { kind: 'color', color: String(args[0]).toLowerCase() }; break
      case OPS.setFillTransparent: st.fill = { kind: 'none' }; break
      case OPS.setStrokeTransparent: st.stroke = { kind: 'none' }; break
      case OPS.setFillColorN: case OPS.setStrokeColorN: {
        const paint: Paint = args[0] === 'Shading' ? paintOfShading(String(args[1]), mul(mul(world, st.base), (args[2] as number[] | null) ?? IDENTITY)) : args[0] === 'TilingPattern' ? { kind: 'other', why: 'pattern' } : { kind: 'other', why: 'color' }
        if (fn === OPS.setFillColorN) st.fill = paint
        else st.stroke = paint
        break
      }
      case OPS.clip: pendingClip = 'nonzero'; break
      case OPS.eoClip: pendingClip = 'evenodd'; break
      case OPS.constructPath: {
        const op = Number(args[0])
        const segs = readPath(args[1] as number[] | null, toWorld())
        if (segs.length && !hidden() && (FILLS.has(op) || STROKES.has(op))) {
          const fill = FILLS.has(op) ? st.fill : null
          const stroke = STROKES.has(op) ? st.stroke : null
          const t = toWorld()
          out.paths.push({
            index: i, segs, box: boxOfSegs(segs), fill, stroke, evenOdd: EVEN_ODD.has(op), fillAlpha: st.fillAlpha, strokeAlpha: st.strokeAlpha,
            lineWidth: Math.max(st.lineWidth, 0) * scaleOf(t), dash: st.dash.map((d) => d * scaleOf(t)), clips: st.clips, form: st.form, shading: false,
            complex: complexOf(fill?.kind === 'other' ? fill : stroke?.kind === 'other' ? stroke : null),
          })
        }
        if (pendingClip) {
          const clip: Clip = { segs, box: boxOfSegs(segs), rect: rectOf(segs) !== null, evenOdd: pendingClip === 'evenodd' }
          st.clips = [...st.clips, clip]
          pendingClip = null
        }
        break
      }
      case OPS.shadingFill: {
        if (hidden()) break
        const paint = paintOfShading(String(args[0]), toWorld())
        // the area is the innermost clip (pdf.js fills the whole canvas through the clip)
        const clip = st.clips.at(-1)
        const area: Seg[] = clip?.segs ?? [{ c: 'M', p: apply(world, 0, 0) }]
        out.paths.push({ index: i, segs: clip ? area : [], box: clip?.box ?? { x: 0, y: 0, w: 0, h: 0 }, fill: paint, stroke: null, evenOdd: clip?.evenOdd ?? false, fillAlpha: st.fillAlpha, strokeAlpha: 1, lineWidth: 0, dash: [], clips: clip ? st.clips.slice(0, -1) : st.clips, form: st.form, shading: true, complex: complexOf(paint) ?? (clip ? null : 'page_shading') })
        break
      }
      case OPS.paintFormXObjectBegin: {
        save()
        if (args[0]) st.ctm = mul(st.ctm, args[0] as number[])
        st.base = st.ctm
        forms += 1
        st.form = forms
        const bbox = args[1] as number[] | null
        if (bbox) {
          const m = toWorld()
          const segs: Seg[] = [{ c: 'M', p: apply(m, bbox[0], bbox[1]) }, { c: 'L', p: apply(m, bbox[2], bbox[1]) }, { c: 'L', p: apply(m, bbox[2], bbox[3]) }, { c: 'L', p: apply(m, bbox[0], bbox[3]) }, { c: 'Z' }]
          st.clips = [...st.clips, { segs, box: boxOfSegs(segs), rect: rectOf(segs) !== null, evenOdd: false }]
        }
        break
      }
      case OPS.paintFormXObjectEnd: restore(); break
      case OPS.beginGroup: {
        save()
        const g = (args[0] ?? {}) as { knockout?: boolean; smask?: boolean }
        if (g.smask) st.complex = 'smask'
        else if (g.knockout) st.complex = 'knockout'
        break
      }
      case OPS.endGroup: restore(); break
      case OPS.beginAnnotation: save(); st.complex = 'annotation'; break
      case OPS.endAnnotation: restore(); break
      case OPS.beginMarkedContentProps:
        marked.push(args[0] === 'OC' ? { mcid: null, hidden: args[1] === false } : { mcid: typeof args[1] === 'number' ? args[1] : null, hidden: false })
        break
      case OPS.beginMarkedContent: marked.push({ mcid: null, hidden: false }); break
      case OPS.endMarkedContent: marked.pop(); break
      case OPS.beginText: st.textMatrix = IDENTITY; st.x = st.lineX = 0; st.y = st.lineY = 0; break
      case OPS.setTextMatrix: st.textMatrix = (args as number[]).slice(0, 6) as Mat; st.x = st.lineX = 0; st.y = st.lineY = 0; break
      case OPS.moveText: moveText(Number(args[0]), Number(args[1])); break
      case OPS.setLeadingMoveText: st.leading = Number(args[1]); moveText(Number(args[0]), Number(args[1])); break
      case OPS.nextLine: moveText(0, st.leading); break
      case OPS.setLeading: st.leading = -Number(args[0]); break
      case OPS.setCharSpacing: st.charSpacing = Number(args[0]) || 0; break
      case OPS.setWordSpacing: st.wordSpacing = Number(args[0]) || 0; break
      case OPS.setHScale: st.hScale = (Number(args[0]) || 100) / 100; break
      case OPS.setTextRise: st.rise = Number(args[0]) || 0; break
      case OPS.setTextRenderingMode: st.mode = Number(args[0]) || 0; break
      case OPS.setFont: st.font = String(args[0]); st.fontSize = Number(args[1]) || 0; break
      case OPS.showText: {
        const font = page.fonts[st.font]
        const direction = st.fontSize < 0 ? -1 : 1
        const size = Math.abs(st.fontSize)
        const advanceScale = size * (font?.unit ?? 0.001)
        let x = 0
        let blank = true
        for (const g of (args[0] as Json[]) ?? []) {
          if (typeof g === 'number') { x += (font?.vertical ? 1 : -1) * g * size / 1000; continue }
          const [width, space, unicode] = g as [number, number, string]
          if (!space && String(unicode).trim()) blank = false
          x += Number(width) * advanceScale + ((space ? st.wordSpacing : 0) + st.charSpacing) * direction
        }
        const m = mul(mul(world, st.ctm), st.textMatrix)
        const start = apply(m, st.x, st.y + st.rise)
        const end = font?.vertical ? apply(m, st.x, st.y + st.rise - x) : apply(m, st.x + x * st.hScale * direction, st.y + st.rise)
        if (!hidden()) {
          out.runs.push({
            index: i, start, end, size: size * Math.hypot(m[2], m[3]), font: st.font, fill: st.fill, alpha: st.fillAlpha, mode: st.mode, mcid: mcid(),
            complex: complexOf(st.fill.kind === 'other' ? st.fill : null) ?? (st.mode >= 4 ? 'text_clip' : null), blank,
          })
        }
        if (font?.vertical) st.y -= x
        else st.x += x * st.hScale * direction
        break
      }
      case OPS.paintImageXObject: case OPS.paintInlineImageXObject: {
        if (hidden()) break
        const m = toWorld()
        out.images.push({ index: i, ref: String(args[0]), corners: [apply(m, 0, 0), apply(m, 1, 0), apply(m, 1, 1), apply(m, 0, 1)], clips: st.clips, alpha: st.fillAlpha, form: st.form, mcid: mcid(), complex: complexOf(null) })
        break
      }
      default:
        if (OTHER_PAINT.has(fn) && !hidden()) out.other.push(i)
    }
  }
  return out
}
