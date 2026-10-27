/* Pen ink (手写笔 TODO D0, D2): the stroke format, the width curve, sampling, simplification, segments within the
 * config budget, decoding and the geometry shared by the live layer, the Block renderer, hit tests and the stage's
 * board marks. Pure: no DOM, no React.
 *
 *   a stroke      the points of one pen-down … pen-up in Surface world coordinates, `t` in ms since the pen came
 *                 down, `p` the pen's pressure (0..1) when it reports one, tilt only when the pen gives it
 *   a segment     one `buckyos.cell` with `view { type: ink, version: 1 }`; its config holds its points in its own
 *                 frame (`0..frame.w`, `0..frame.h`) and the placement box shows that frame (a resize scales it)
 *   a long stroke several segments with one `stroke` id, consecutive ones sharing their boundary point; they are
 *                 created in one commit and selected, erased and copied as one */

import { centerOf, rotatePoint, type Point } from '../ui/canvas/geometry'
import type { Rect } from '../ui/canvas/render/camera'

export interface InkPoint { x: number; y: number; t: number; p?: number; tx?: number; ty?: number }
export interface InkStyle { color: string; width: number; pressure: boolean }

export interface InkConfig {
  format: 1
  stroke: string
  seg: number
  segs: number
  color: string
  width: number
  pressure: boolean
  frame: { w: number; h: number }
  points: { x: number[]; y: number[]; t: number[]; p?: number[]; tilt_x?: number[]; tilt_y?: number[] }
}

/** A decoded segment: its points in its own frame. */
export interface InkSegment { stroke: string; seg: number; segs: number; style: InkStyle; frame: { w: number; h: number }; points: InkPoint[] }

export const INK_VIEW = 'ink'
export const INK_FORMAT = 1
/** The backend's limit of a Cell's config (canonical JSON bytes, core `MAX_CONFIG_BYTES`). */
export const MAX_CONFIG_BYTES = 64 * 1024
/** A segment is cut before its config reaches this size: the rest of the limit is the margin of the style and keys. */
export const SEGMENT_BUDGET = 60 * 1024
/** Segments of one logical stroke (one commit); a pen still down past them goes on as a new stroke. */
export const MAX_SEGMENTS = 8
/** The raw samples one stroke may gather before it is saved and continued as a new one (simplification only shrinks). */
export const RAW_BUDGET = Math.floor(SEGMENT_BUDGET * MAX_SEGMENTS * 0.9)
/** Bounds of the core's validation (`validate_ink`). */
export const MAX_SEGMENT_POINTS = 16_384
export const MAX_WIDTH = 256
/** Simplification tolerance in screen px at the zoom the stroke was written at. */
export const SIMPLIFY_PX = 0.25
/** Simplification runs over windows of this many points (a bounded cost on pathological input). */
const SIMPLIFY_WINDOW = 2048
const COLOR = /^#[0-9a-fA-F]{6}$/

/** The drawn width at pressure `p` (0..1): thin at no pressure, the base width at the half pressure a pen without
 * pressure reports, wider when pressed hard. Without pressure (a mouse, pressure off) every point has the base width. */
export function widthAt(style: InkStyle, p: number | undefined): number {
  if (!style.pressure || p === undefined) return style.width
  return style.width * (0.3 + 1.4 * Math.min(1, Math.max(0, p)))
}

/** The widest a stroke of this style is drawn. */
export function maxWidth(style: InkStyle): number {
  return style.pressure ? style.width * 1.7 : style.width
}

// ---- sampling

/** What sampling reads of a pointer event (a DOM PointerEvent fits; tests pass plain objects). */
export interface PointerSample {
  pointerId: number
  pointerType: string
  clientX: number
  clientY: number
  pressure: number
  tiltX?: number
  tiltY?: number
  timeStamp: number
  getCoalescedEvents?: () => PointerSample[]
  getPredictedEvents?: () => PointerSample[]
}

/** The samples a move stands for, oldest first: its coalesced events (they include the dispatched one) when the
 * browser gives any, else the event itself — never both. */
export function samplesOf(event: PointerSample): PointerSample[] {
  const coalesced = typeof event.getCoalescedEvents === 'function' ? event.getCoalescedEvents() : null
  const list = coalesced && coalesced.length > 0 ? coalesced : [event]
  return [...list].sort((a, b) => a.timeStamp - b.timeStamp)
}

/** The predicted next samples (a temporary pen tip only; never stored). */
export function predictionsOf(event: PointerSample): PointerSample[] {
  const predicted = typeof event.getPredictedEvents === 'function' ? event.getPredictedEvents() : null
  return predicted ? [...predicted] : []
}

/** One stored point from a sample at world `w`: pressure from a pen only; tilt only when the pen reports one. */
export function pointOf(sample: PointerSample, w: Point, start: number, lastT: number): InkPoint {
  const t = Math.max(lastT, Math.round(sample.timeStamp - start))
  const point: InkPoint = { x: w.x, y: w.y, t }
  if (sample.pointerType === 'pen') {
    point.p = Math.min(1, Math.max(0, sample.pressure))
    if (sample.tiltX || sample.tiltY) { point.tx = Math.round(sample.tiltX ?? 0); point.ty = Math.round(sample.tiltY ?? 0) }
  }
  return point
}

/** Is `next` the same sample as `last` (position, time, pressure and tilt)? Duplicates only; a pen resting while its
 * pressure or time changes is kept. */
export function sameSample(last: InkPoint | undefined, next: InkPoint): boolean {
  return Boolean(last && last.x === next.x && last.y === next.y && last.t === next.t && last.p === next.p && last.tx === next.tx && last.ty === next.ty)
}

/** What a stroke keeps of its samples' channels: no pressure when the pen never reported anything but the 0.5 of a pen
 * without pressure (judged on the whole stroke, not one event); no tilt when it was always flat. */
export function settleChannels(points: InkPoint[]): InkPoint[] {
  const flatPressure = points.every((q) => q.p === undefined || q.p === 0.5)
  const noTilt = points.every((q) => !q.tx && !q.ty)
  if (!flatPressure && !noTilt) return points
  return points.map((q) => {
    const out: InkPoint = { x: q.x, y: q.y, t: q.t }
    if (!flatPressure && q.p !== undefined) out.p = q.p
    if (!noTilt && (q.tx !== undefined || q.ty !== undefined)) { out.tx = q.tx ?? 0; out.ty = q.ty ?? 0 }
    return out
  })
}

// ---- simplification

function distance3(q: InkPoint, a: InkPoint, b: InkPoint, k: number): number {
  const za = (a.p ?? 0) * k
  const zb = (b.p ?? 0) * k
  const zq = (q.p ?? 0) * k
  const dx = b.x - a.x
  const dy = b.y - a.y
  const dz = zb - za
  const len2 = dx * dx + dy * dy + dz * dz
  let u = len2 > 0 ? ((q.x - a.x) * dx + (q.y - a.y) * dy + (zq - za) * dz) / len2 : 0
  u = Math.max(0, Math.min(1, u))
  return Math.hypot(q.x - (a.x + u * dx), q.y - (a.y + u * dy), zq - (za + u * dz))
}

/** Ramer–Douglas–Peucker in (x, y, pressure as edge displacement): the ends, clear turns and pressure changes stay.
 * Windows of SIMPLIFY_WINDOW points keep the worst case bounded; their boundaries stay too. */
export function simplify(points: InkPoint[], tolerance: number, style: InkStyle): InkPoint[] {
  const n = points.length
  if (n <= 2 || tolerance <= 0) return points.slice()
  // a pressure change moves the edge of the drawn line by half the width it changes
  const k = style.pressure ? style.width * 0.7 : 0
  const keep = new Uint8Array(n)
  for (let from = 0; from < n - 1; from += SIMPLIFY_WINDOW) {
    const to = Math.min(n - 1, from + SIMPLIFY_WINDOW)
    keep[from] = 1
    keep[to] = 1
    const stack: [number, number][] = [[from, to]]
    while (stack.length > 0) {
      const [a, b] = stack.pop()!
      let best = -1
      let bestD = tolerance
      for (let i = a + 1; i < b; i++) {
        const d = distance3(points[i], points[a], points[b], k)
        if (d > bestD) { bestD = d; best = i }
      }
      if (best >= 0) { keep[best] = 1; stack.push([a, best], [best, b]) }
    }
  }
  const out: InkPoint[] = []
  for (let i = 0; i < n; i++) if (keep[i]) out.push(points[i])
  return out
}

// ---- encoding

const r2 = (v: number) => Math.round(v * 100) / 100
const r3 = (v: number) => Math.round(v * 1000) / 1000
const chars = (v: number) => String(v).length

/** The canonical byte size of a config (JCS: same numbers as JSON.stringify, sorted keys, no spaces; ASCII here). */
export function configBytes(config: InkConfig): number {
  return new TextEncoder().encode(JSON.stringify(config)).length
}

/** Bytes a point adds to a segment's config (an estimate in world coordinates; its frame's are no longer). */
function pointBytes(q: InkPoint): number {
  let n = chars(r2(q.x)) + chars(r2(q.y)) + chars(Math.round(q.t)) + 3
  if (q.p !== undefined) n += chars(r3(q.p)) + 1
  if (q.tx !== undefined) n += chars(q.tx) + chars(q.ty ?? 0) + 2
  return n
}

/** Raw bytes of the samples gathered so far (the input splits a stroke past RAW_BUDGET). */
export function rawBytes(points: InkPoint[]): number {
  let n = 0
  for (const q of points) n += pointBytes(q)
  return n
}

export interface EncodedSegment { config: InkConfig; /** World box of the segment's frame (whole px). */ rect: Rect }

function buildSegment(points: InkPoint[], style: InkStyle, stroke: string, seg: number, segs: number): EncodedSegment {
  const pad = maxWidth(style) / 2 + 1
  let minX = Infinity, minY = Infinity, maxX = -Infinity, maxY = -Infinity
  for (const q of points) { minX = Math.min(minX, q.x); minY = Math.min(minY, q.y); maxX = Math.max(maxX, q.x); maxY = Math.max(maxY, q.y) }
  const x = Math.floor(minX - pad)
  const y = Math.floor(minY - pad)
  const w = Math.max(1, Math.ceil(maxX + pad) - x)
  const h = Math.max(1, Math.ceil(maxY + pad) - y)
  const channels: InkConfig['points'] = { x: [], y: [], t: [] }
  const withP = points.some((q) => q.p !== undefined)
  const withTilt = points.some((q) => q.tx !== undefined)
  if (withP) channels.p = []
  if (withTilt) { channels.tilt_x = []; channels.tilt_y = [] }
  for (const q of points) {
    channels.x.push(Math.min(w, Math.max(0, r2(q.x - x))))
    channels.y.push(Math.min(h, Math.max(0, r2(q.y - y))))
    channels.t.push(Math.round(q.t))
    if (withP) channels.p!.push(r3(q.p ?? 0.5))
    if (withTilt) { channels.tilt_x!.push(Math.round(q.tx ?? 0)); channels.tilt_y!.push(Math.round(q.ty ?? 0)) }
  }
  const config: InkConfig = { format: INK_FORMAT, stroke, seg, segs, color: style.color, width: style.width, pressure: style.pressure, frame: { w, h }, points: channels }
  return { config, rect: { x, y, w, h } }
}

/** A stroke (world points, already simplified) as segments whose configs stay within SEGMENT_BUDGET, consecutive
 * segments sharing their boundary point. The sizes are measured, not guessed: a segment over the budget is cut again. */
export function encodeStroke(points: InkPoint[], style: InkStyle, stroke: string): EncodedSegment[] {
  if (points.length === 0) return []
  const overhead = 512 + stroke.length + style.color.length
  const chunks: InkPoint[][] = []
  let start = 0
  for (;;) {
    let bytes = overhead
    let end = start
    while (end < points.length && end - start < MAX_SEGMENT_POINTS && bytes + pointBytes(points[end]) <= SEGMENT_BUDGET) { bytes += pointBytes(points[end]); end++ }
    if (end === start) end = start + 1
    chunks.push(points.slice(start, end))
    if (end >= points.length) break
    start = end - 1
  }
  for (;;) {
    const segments = chunks.map((chunk, i) => buildSegment(chunk, style, stroke, i, chunks.length))
    const over = segments.findIndex((s) => configBytes(s.config) > SEGMENT_BUDGET)
    if (over < 0) return segments
    const chunk = chunks[over]
    const mid = Math.floor(chunk.length / 2)
    chunks.splice(over, 1, chunk.slice(0, mid + 1), chunk.slice(mid))
  }
}

// ---- decoding

const isNum = (v: unknown): v is number => typeof v === 'number' && Number.isFinite(v)

/** A segment's config as written by `encodeStroke` (and checked by the core), or null when it is not one. */
export function readInk(config: unknown): InkSegment | null {
  if (!config || typeof config !== 'object') return null
  const c = config as Partial<InkConfig>
  if (c.format !== INK_FORMAT || typeof c.stroke !== 'string' || !isNum(c.seg) || !isNum(c.segs) || c.seg < 0 || c.seg >= c.segs) return null
  if (typeof c.color !== 'string' || !COLOR.test(c.color) || !isNum(c.width) || c.width <= 0 || c.width > MAX_WIDTH || typeof c.pressure !== 'boolean') return null
  const frame = c.frame
  if (!frame || !isNum(frame.w) || !isNum(frame.h) || frame.w <= 0 || frame.h <= 0) return null
  const pts = c.points
  if (!pts || !Array.isArray(pts.x) || !Array.isArray(pts.y) || !Array.isArray(pts.t)) return null
  const n = pts.x.length
  if (n === 0 || pts.y.length !== n || pts.t.length !== n) return null
  const optional = (list: unknown) => (list === undefined ? null : Array.isArray(list) && list.length === n && list.every(isNum) ? list as number[] : undefined)
  const p = optional(pts.p)
  const tx = optional(pts.tilt_x)
  const ty = optional(pts.tilt_y)
  if (p === undefined || tx === undefined || ty === undefined) return null
  const points: InkPoint[] = new Array(n)
  for (let i = 0; i < n; i++) {
    const x = pts.x[i], y = pts.y[i], t = pts.t[i]
    if (!isNum(x) || !isNum(y) || !isNum(t)) return null
    const q: InkPoint = { x, y, t }
    if (p) q.p = p[i]
    if (tx && ty) { q.tx = tx[i]; q.ty = ty[i] }
    points[i] = q
  }
  return { stroke: c.stroke, seg: c.seg, segs: c.segs, style: { color: c.color, width: c.width, pressure: c.pressure }, frame: { w: frame.w, h: frame.h }, points }
}

// ---- geometry

/** How a segment's frame maps onto its box: a resized box scales the points, and the width by the mean scale. */
export function frameScale(frame: { w: number; h: number }, size: { w: number; h: number }): { sx: number; sy: number; sw: number } {
  const sx = size.w / frame.w
  const sy = size.h / frame.h
  return { sx, sy, sw: Math.sqrt(Math.abs(sx * sy)) }
}

/** A point with the width it is drawn at. */
export interface WidePoint { x: number; y: number; w: number }

/** The points of a segment in its box of `size` (its own drawing coordinates) with their widths. */
export function boxPoints(ink: InkSegment, size: { w: number; h: number }): WidePoint[] {
  const { sx, sy, sw } = frameScale(ink.frame, size)
  return ink.points.map((q) => ({ x: q.x * sx, y: q.y * sy, w: widthAt(ink.style, q.p) * sw }))
}

/** The points of a segment in world coordinates: its box `rect` turned by `rotation` about its centre. */
export function worldPoints(ink: InkSegment, rect: Rect, rotation: number): WidePoint[] {
  const c = centerOf(rect)
  return boxPoints(ink, rect).map((q) => {
    const p = rotatePoint({ x: rect.x + q.x, y: rect.y + q.y }, c, rotation)
    return { x: p.x, y: p.y, w: q.w }
  })
}

/** The points as drawn from world points of a stroke being written (or waiting for its Block). */
export function widePoints(points: InkPoint[], style: InkStyle): WidePoint[] {
  return points.map((q) => ({ x: q.x, y: q.y, w: widthAt(style, q.p) }))
}

/** One piece of the smoothed line: a quadratic from `a` to `b` bent towards `c` (a straight piece has no `c`), at the
 * width of the point it belongs to. The line runs p0 → m0, then m(i-1) ~p(i)~ m(i), then m(n-2) → p(n-1), where m are
 * the midpoints: slow curves come out round, and every piece is known once the point after it is. */
export interface Piece { a: Point; b: Point; c?: Point; w: number }

const mid = (a: Point, b: Point): Point => ({ x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 })

/** Piece `i` of `pts` (0 … n-1); a single point is a dot (`a` = `b`). */
export function pieceAt(pts: readonly WidePoint[], i: number): Piece {
  const n = pts.length
  if (n === 1) return { a: pts[0], b: pts[0], w: pts[0].w }
  if (i === 0) return { a: pts[0], b: mid(pts[0], pts[1]), w: pts[0].w }
  if (i === n - 1) return { a: mid(pts[n - 2], pts[n - 1]), b: pts[n - 1], w: pts[n - 1].w }
  return { a: mid(pts[i - 1], pts[i]), c: pts[i], b: mid(pts[i], pts[i + 1]), w: pts[i].w }
}

export function pieces(pts: readonly WidePoint[]): Piece[] {
  const out: Piece[] = []
  for (let i = 0; i < pts.length; i++) out.push(pieceAt(pts, i))
  return out
}

/** Consecutive pieces drawn as one path: widths within 8 % share one (a stroke without pressure is one path, one with
 * pressure a path per step of its width, not one per point). `w` is the run's width. */
export interface Run { pieces: Piece[]; w: number }

export function runs(list: readonly Piece[]): Run[] {
  const out: Run[] = []
  for (const piece of list) {
    const quantum = Math.max(0.1, piece.w * 0.08)
    const w = Math.round(piece.w / quantum) * quantum
    const last = out.at(-1)
    if (last && Math.abs(last.w - w) < 1e-9) last.pieces.push(piece)
    else out.push({ pieces: [piece], w })
  }
  return out
}

const f = (v: number) => String(Math.round(v * 100) / 100)

/** SVG path data of a run (its pieces are contiguous). */
export function runPath(run: Run): string {
  const first = run.pieces[0]
  let d = `M${f(first.a.x)} ${f(first.a.y)}`
  // a dot: a zero-length line, which round caps draw as a disc
  if (run.pieces.length === 1 && first.a.x === first.b.x && first.a.y === first.b.y) return `${d}l0 0`
  for (const piece of run.pieces) d += piece.c ? `Q${f(piece.c.x)} ${f(piece.c.y)} ${f(piece.b.x)} ${f(piece.b.y)}` : `L${f(piece.b.x)} ${f(piece.b.y)}`
  return d
}

/** Display points of a long stroke when the canvas is zoomed out: fewer, still within half a screen pixel. */
export function displayPoints(pts: WidePoint[], zoom: number): WidePoint[] {
  if (zoom >= 1 || pts.length <= 8) return pts
  const tolerance = 0.5 / zoom
  const asInk = pts.map((q, i) => ({ x: q.x, y: q.y, t: i, p: q.w }))
  const kept = simplify(asInk, tolerance, { color: '#000000', width: 1 / 0.7, pressure: true })
  return kept.map((q) => ({ x: q.x, y: q.y, w: q.p ?? 0 }))
}

// ---- hit tests

/** The distance between segments ab and cd. */
export function segmentDistance(a: Point, b: Point, c: Point, d: Point): number {
  const cross = (o: Point, p: Point, q: Point) => (p.x - o.x) * (q.y - o.y) - (p.y - o.y) * (q.x - o.x)
  const d1 = cross(c, d, a), d2 = cross(c, d, b), d3 = cross(a, b, c), d4 = cross(a, b, d)
  if (((d1 > 0 && d2 < 0) || (d1 < 0 && d2 > 0)) && ((d3 > 0 && d4 < 0) || (d3 < 0 && d4 > 0))) return 0
  return Math.min(pointSegment(a, c, d), pointSegment(b, c, d), pointSegment(c, a, b), pointSegment(d, a, b))
}

export function pointSegment(p: Point, a: Point, b: Point): number {
  const dx = b.x - a.x
  const dy = b.y - a.y
  const len2 = dx * dx + dy * dy
  const u = len2 > 0 ? Math.max(0, Math.min(1, ((p.x - a.x) * dx + (p.y - a.y) * dy) / len2)) : 0
  return Math.hypot(p.x - (a.x + u * dx), p.y - (a.y + u * dy))
}

/** Does the eraser's path from `a` to `b` (radius `r`) touch the drawn line (the segments between the points, each
 * as wide as its wider end) — not just its sample points or its box? */
export function touches(pts: readonly WidePoint[], a: Point, b: Point, r: number): boolean {
  if (pts.length === 1) return segmentDistance(a, b, pts[0], pts[0]) <= r + pts[0].w / 2
  for (let i = 1; i < pts.length; i++) {
    if (segmentDistance(a, b, pts[i - 1], pts[i]) <= r + Math.max(pts[i - 1].w, pts[i].w) / 2) return true
  }
  return false
}

/** The distance from `p` to the drawn line's edge (0 inside it). */
export function distanceToInk(pts: readonly WidePoint[], p: Point): number {
  let best = Infinity
  if (pts.length === 1) return Math.max(0, Math.hypot(p.x - pts[0].x, p.y - pts[0].y) - pts[0].w / 2)
  for (let i = 1; i < pts.length; i++) best = Math.min(best, pointSegment(p, pts[i - 1], pts[i]) - Math.max(pts[i - 1].w, pts[i].w) / 2)
  return Math.max(0, best)
}
