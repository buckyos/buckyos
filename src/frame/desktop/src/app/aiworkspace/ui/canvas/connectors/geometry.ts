/* Connector geometry (连接线实现方案 §4.3–§4.5, §9.1): pure functions, the same on every client — part of
 * `connector@1`, so a change that alters existing drawings needs a new `view.version`.
 *
 *   stored ends   the placement box plus `flip` give the two stored endpoints S and E (world)
 *   connection    a bound end sits on its target's anchor of that id, leaving by its side (or by the one of its
 *                 sides facing the other end); an id the target does not have floats: where the ray from the
 *                 target's centre towards the other end leaves its outline (rect or ellipse); rotated targets
 *                 turn both
 *   controls      `{u, v, dx, dy}` in the endpoint frame: between the ends a ratio, beyond them an offset from
 *                 the nearer end, decomposed per axis — so right angles survive any move of the ends
 *   routes        straight (through the controls), elbow (orthogonal; automatic route adapted from React
 *                 Flow's smoothstep, MIT), curve (cubic Hermite through the points, ends along the exit
 *                 directions)
 *   measures      flattened polyline, arc length, label point, nearest point, distance, bounds */

import { centerOf, rotatePoint, type Point } from '../geometry'
import type { Rect } from '../render/camera'
import { defaultAnchors, findAnchor, SIDE_VEC, sidesOf, type AnchorDef, type Shape, type Side } from './anchors'
import type { Anchor, Binding, Control, ConnectorData, Flip, Route } from './model'

export type { Shape } from './anchors'
/** A bound end's target: its layout rect, rotation, outline and anchors (the outline's default set if absent). */
export interface TargetGeom { rect: Rect; rotation: number; shape: Shape; anchors?: readonly AnchorDef[] }
export type BrokenReason = 'missing' | 'off_surface' | 'invalid'
/** What the layout knows of a bound end's target. */
export type TargetLookup = (id: string) => TargetGeom | { broken: BrokenReason }

export interface ResolvedEnd {
  point: Point
  /** Unit direction the line leaves this end with (outward from a bound target). */
  dir: Point
  state: 'free' | 'bound' | 'broken'
  reason?: BrokenReason
  targetId?: string
}

/** One piece of the path: a line, or a cubic when `c1` / `c2` are set. */
export interface Seg { a: Point; b: Point; c1?: Point; c2?: Point }

export interface ConnectorGeometry {
  start: ResolvedEnd
  end: ResolvedEnd
  route: Route
  /** Straight: S, the bends, E. Elbow: S, every corner, E. Curve: S, the points passed through, E. */
  pts: Point[]
  /** No stored controls: the route is the automatic one. */
  auto: boolean
  segs: Seg[]
  /** The path as a polyline (cubics sampled), with cumulative lengths and the piece each sample belongs to. */
  flat: Point[]
  cum: number[]
  segOf: number[]
  length: number
  label: { point: Point; box: Rect | null }
  /** The path, its label and `pad` around it. */
  bounds: Rect
}

/** Distance from a bound target's outline to where the line (or its cap) ends, in world units. */
export const END_GAP = 4
/** First leg of an automatic elbow route out of a connection point. */
export const ELBOW_STUB = 20
const EPS = 0.01
const CURVE_SAMPLES = 24

// ---- small vector helpers

const sub = (a: Point, b: Point): Point => ({ x: a.x - b.x, y: a.y - b.y })
const add = (a: Point, b: Point): Point => ({ x: a.x + b.x, y: a.y + b.y })
const mul = (a: Point, k: number): Point => ({ x: a.x * k, y: a.y * k })
const len = (a: Point) => Math.hypot(a.x, a.y)
export const dist = (a: Point, b: Point) => Math.hypot(a.x - b.x, a.y - b.y)
function unit(a: Point, fallback: Point = { x: 1, y: 0 }): Point {
  const l = len(a)
  return l < 1e-9 ? fallback : { x: a.x / l, y: a.y / l }
}
/** The dominant axis of a vector as a unit direction (ties go horizontal; zero goes right). */
export function axisOf(dx: number, dy: number): Point {
  if (Math.abs(dx) < 1e-9 && Math.abs(dy) < 1e-9) return { x: 1, y: 0 }
  return Math.abs(dx) >= Math.abs(dy) ? { x: Math.sign(dx), y: 0 } : { x: 0, y: Math.sign(dy) }
}

// ---- stored ends (placement box + flip)

/** S and E of a stored box: S is the top-left corner unless flipped (OOXML `xfrm` + `flipH` / `flipV`). */
export function storedEnds(rect: Rect, flip: Flip): [Point, Point] {
  const x1 = rect.x, y1 = rect.y, x2 = rect.x + rect.w, y2 = rect.y + rect.h
  return [{ x: flip.h ? x2 : x1, y: flip.v ? y2 : y1 }, { x: flip.h ? x1 : x2, y: flip.v ? y1 : y2 }]
}

/** The stored box and flip of two endpoints (rounded like every placement). */
export function boxOf(s: Point, e: Point): { rect: Rect; flip: Flip } {
  const sx = Math.round(s.x), sy = Math.round(s.y), ex = Math.round(e.x), ey = Math.round(e.y)
  return { rect: { x: Math.min(sx, ex), y: Math.min(sy, ey), w: Math.abs(ex - sx), h: Math.abs(ey - sy) }, flip: { h: sx > ex, v: sy > ey } }
}

// ---- connection points

function toLocal(t: TargetGeom, p: Point): Point { return rotatePoint(p, centerOf(t.rect), -t.rotation) }
function toWorld(t: TargetGeom, p: Point): Point { return rotatePoint(p, centerOf(t.rect), t.rotation) }

export function anchorsOf(t: TargetGeom): readonly AnchorDef[] {
  return t.anchors ?? defaultAnchors(t.shape)
}

/** The target's anchor of that id (none: the target does not have it). */
function namedDef(t: TargetGeom, anchor: Anchor): AnchorDef | undefined {
  return findAnchor(anchorsOf(t), anchor.id)
}

function defLocal(t: TargetGeom, a: AnchorDef): Point {
  const r = t.rect
  return { x: r.x + a.x * r.w + (a.dx ?? 0), y: r.y + a.y * r.h + (a.dy ?? 0) }
}

/** Where the ray from the target's centre towards `toward` leaves its outline (own coordinates). */
function floating(t: TargetGeom, toward: Point): Point {
  const r = t.rect
  const c = centerOf(r)
  const q = toLocal(t, toward)
  let dx = q.x - c.x
  let dy = q.y - c.y
  if (Math.abs(dx) < 1e-9 && Math.abs(dy) < 1e-9) { dx = 1; dy = 0 }
  const hw = Math.max(r.w / 2, 1e-9)
  const hh = Math.max(r.h / 2, 1e-9)
  const k = t.shape === 'ellipse' ? 1 / Math.hypot(dx / hw, dy / hh) : 1 / Math.max(Math.abs(dx) / hw, Math.abs(dy) / hh)
  return { x: c.x + dx * k, y: c.y + dy * k }
}

/** The connection point in the target's own (unrotated) coordinates. */
export function anchorLocal(t: TargetGeom, anchor: Anchor, toward: Point): Point {
  const def = namedDef(t, anchor)
  return def ? defLocal(t, def) : floating(t, toward)
}

/** The world position of a connection point. */
export function anchorWorld(t: TargetGeom, anchor: Anchor, toward: Point): Point {
  return toWorld(t, anchorLocal(t, anchor, toward))
}

/** The side a floating connection point is on, as a turned unit direction (the centre takes the way to `toward`). */
function outward(t: TargetGeom, local: Point, toward: Point): Point {
  const r = t.rect
  const c = centerOf(r)
  const nx = (local.x - c.x) / Math.max(r.w / 2, 1e-9)
  const ny = (local.y - c.y) / Math.max(r.h / 2, 1e-9)
  let d: Point
  if (Math.abs(nx) < 1e-6 && Math.abs(ny) < 1e-6) { const q = toLocal(t, toward); d = axisOf(q.x - c.x, q.y - c.y) }
  else d = Math.abs(nx) >= Math.abs(ny) ? { x: Math.sign(nx), y: 0 } : { x: 0, y: Math.sign(ny) }
  return rotatePoint(d, { x: 0, y: 0 }, t.rotation)
}

/** The side a named anchor leaves by: its only side, or of its sides the one facing `toward` (ties: the first). */
function sideToward(t: TargetGeom, def: AnchorDef, local: Point, toward: Point): Point {
  const q = toLocal(t, toward)
  const v = { x: q.x - local.x, y: q.y - local.y }
  let best = SIDE_VEC[sidesOf(def)[0]]
  let bestDot = -Infinity
  for (const side of sidesOf(def)) {
    const d = SIDE_VEC[side]
    const dot = d.x * v.x + d.y * v.y
    if (dot > bestDot + 1e-9) { best = d; bestDot = dot }
  }
  return rotatePoint(best, { x: 0, y: 0 }, t.rotation)
}

/** Minor anchors are offered when every one of them is at least this far (screen px) from its neighbours. */
export const MINOR_GAP_PX = 32

export interface AnchorSpot { def: AnchorDef; anchor: Anchor; point: Point }

/** The anchors a dragged end can snap to at `zoom`, with their world points: the major ones always, the minor
 * ones too when the target is large enough on screen (8 → 16 on a rectangle). */
export function visibleAnchors(t: TargetGeom, zoom: number): AnchorSpot[] {
  const all = anchorsOf(t).map((def): AnchorSpot => ({ def, anchor: { kind: 'named', id: def.id }, point: toWorld(t, defLocal(t, def)) }))
  const minor = all.filter((a) => a.def.minor)
  if (!minor.length) return all
  const gap = MINOR_GAP_PX / Math.max(zoom, 1e-9)
  const roomy = minor.every((m) => all.every((o) => o === m || dist(o.point, m.point) >= gap))
  return roomy ? all : all.filter((a) => !a.def.minor)
}

/** The anchor an end dropped inside the target (not on an anchor) takes: on the side facing `toward` (in the
 * target's own axes), the major anchor that leaves only by that side and is nearest to where the ray from the
 * centre leaves the outline; without one, any major anchor leaving by that side, then any major anchor. None
 * when the target declares no anchors (it takes no lines). */
export function facingAnchor(t: TargetGeom, toward: Point): Anchor | null {
  const defs = anchorsOf(t)
  if (!defs.length) return null
  const c = centerOf(t.rect)
  const q = toLocal(t, toward)
  const side = axisOf(q.x - c.x, q.y - c.y)
  const hit = floating(t, toward)
  const majors = defs.filter((d) => !d.minor)
  const pool = majors.length ? majors : defs
  const faces = (d: AnchorDef, only: boolean) => {
    const sides = sidesOf(d)
    return (!only || sides.length === 1) && sides.some((s) => SIDE_VEC[s].x === side.x && SIDE_VEC[s].y === side.y)
  }
  const only = pool.filter((d) => faces(d, true))
  const some = only.length ? only : pool.filter((d) => faces(d, false))
  const candidates = some.length ? some : pool
  let best = candidates[0]
  let bestD = Infinity
  for (const d of candidates) {
    const dd = dist(defLocal(t, d), hit)
    if (dd < bestD - 1e-9) { best = d; bestD = dd }
  }
  return { kind: 'named', id: best.id }
}

/** The anchor a connection handle on `side` starts from: the target's anchor of that compass id when it leaves by
 * that side, else the major anchor leaving by that side nearest the side's midpoint; none without one. */
export function sideAnchor(t: TargetGeom, side: Side): AnchorSpot | null {
  const defs = anchorsOf(t)
  const leaves = (d: AnchorDef) => sidesOf(d).includes(side)
  const own = findAnchor(defs, side)
  let def = own && leaves(own) ? own : undefined
  if (!def) {
    const r = t.rect
    const mid = { x: r.x + (0.5 + SIDE_VEC[side].x / 2) * r.w, y: r.y + (0.5 + SIDE_VEC[side].y / 2) * r.h }
    let bestD = Infinity
    for (const d of defs) {
      if (d.minor || !leaves(d)) continue
      const dd = dist(defLocal(t, d), mid)
      if (dd < bestD) { def = d; bestD = dd }
    }
  }
  return def ? { def, anchor: { kind: 'named', id: def.id }, point: toWorld(t, defLocal(t, def)) } : null
}

/** Where both ends are now (§4.3): bound ends on their targets, coordinate and broken ends at their stored spot. */
export function resolveEnds(data: Pick<ConnectorData, 'start' | 'end'>, stored: [Point, Point], lookup: TargetLookup): [ResolvedEnd, ResolvedEnd] {
  const geomOf = (b: Binding | null) => {
    if (!b) return { b, g: null, broken: null }
    const t = lookup(b.entity_id)
    return 'broken' in t ? { b, g: null, broken: t.broken } : { b, g: t, broken: null }
  }
  const s = geomOf(data.start)
  const e = geomOf(data.end)
  // what an end's sides face (and a floating end aims at): the other end's anchor, or (the other end floats too)
  // the other target's centre
  const fixed = (o: typeof s) => Boolean(o.b && o.g && namedDef(o.g, o.b.anchor))
  const reference = (o: typeof s, stored: Point): Point => (!o.b || !o.g ? stored : fixed(o) ? anchorWorld(o.g, o.b.anchor, stored) : centerOf(o.g.rect))
  const one = (x: typeof s, storedAt: Point, toward: Point): ResolvedEnd => {
    if (!x.b) return { point: storedAt, dir: { x: 0, y: 0 }, state: 'free' }
    if (!x.g) return { point: storedAt, dir: { x: 0, y: 0 }, state: 'broken', reason: x.broken ?? 'missing', targetId: x.b.entity_id }
    const def = namedDef(x.g, x.b.anchor)
    const local = def ? defLocal(x.g, def) : anchorLocal(x.g, x.b.anchor, toward)
    return { point: toWorld(x.g, local), dir: def ? sideToward(x.g, def, local, toward) : outward(x.g, local, toward), state: 'bound', targetId: x.b.entity_id }
  }
  const start = one(s, stored[0], reference(e, stored[1]))
  const end = one(e, stored[1], reference(s, stored[0]))
  // a free end leaves towards the other end, along the dominant axis
  if (start.state !== 'bound') start.dir = axisOf(end.point.x - start.point.x, end.point.y - start.point.y)
  if (end.state !== 'bound') end.dir = axisOf(start.point.x - end.point.x, start.point.y - end.point.y)
  return [start, end]
}

// ---- control points in the endpoint frame (§4.4)

function axisPart(p: number, s: number, e: number): { r: number; d: number } {
  const span = e - s
  if (Math.abs(span) >= 1 && (p - s) * (p - e) <= 0) return { r: Math.round(((p - s) / span) * 1e6) / 1e6, d: 0 }
  return Math.abs(p - s) <= Math.abs(p - e) ? { r: 0, d: Math.round((p - s) * 100) / 100 } : { r: 1, d: Math.round((p - e) * 100) / 100 }
}

/** The canonical decomposition of a world point against the resolved ends. */
export function decompose(p: Point, s: Point, e: Point): Control {
  const x = axisPart(p.x, s.x, e.x)
  const y = axisPart(p.y, s.y, e.y)
  return { u: x.r, v: y.r, ...(x.d ? { dx: x.d } : {}), ...(y.d ? { dy: y.d } : {}) }
}

export function compose(c: Control, s: Point, e: Point): Point {
  return { x: s.x + c.u * (e.x - s.x) + (c.dx ?? 0), y: s.y + c.v * (e.y - s.y) + (c.dy ?? 0) }
}

// ---- routes

function samePoint(a: Point, b: Point) { return Math.abs(a.x - b.x) < EPS && Math.abs(a.y - b.y) < EPS }

/** Drop repeated points and corners that are not corners (collinear, same direction). */
export function simplify(points: Point[]): Point[] {
  const out: Point[] = []
  for (const p of points) if (!out.length || !samePoint(out[out.length - 1], p)) out.push(p)
  for (let i = 1; i < out.length - 1;) {
    const a = out[i - 1], b = out[i], c = out[i + 1]
    const cross = (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x)
    const dot = (b.x - a.x) * (c.x - b.x) + (b.y - a.y) * (c.y - b.y)
    if (Math.abs(cross) < 1e-6 && dot >= 0) out.splice(i, 1)
    else i += 1
  }
  return out
}

type Axis = 'x' | 'y'

/** The automatic orthogonal route between two ends leaving along `ds` / `de` (adapted from React Flow's smoothstep). */
export function elbowAuto(s: Point, ds: Point, e: Point, de: Point): Point[] {
  const sg = add(s, mul(ds, ELBOW_STUB))
  const tg = add(e, mul(de, ELBOW_STUB))
  const acc: Axis = ds.x !== 0 ? 'x' : 'y'
  const curr = acc === 'x' ? (sg.x < tg.x ? 1 : -1) : (sg.y < tg.y ? 1 : -1)
  const cx = (s.x + e.x) / 2
  const cy = (s.y + e.y) / 2
  if (ds[acc] * de[acc] === -1) {
    const verticalSplit = [{ x: cx, y: sg.y }, { x: cx, y: tg.y }]
    const horizontalSplit = [{ x: sg.x, y: cy }, { x: tg.x, y: cy }]
    const mid = ds[acc] === curr ? (acc === 'x' ? verticalSplit : horizontalSplit) : (acc === 'x' ? horizontalSplit : verticalSplit)
    return simplify([s, sg, ...mid, tg, e])
  }
  const sourceTarget = [{ x: sg.x, y: tg.y }]
  const targetSource = [{ x: tg.x, y: sg.y }]
  let mid = acc === 'x' ? (ds.x === curr ? targetSource : sourceTarget) : (ds.y === curr ? sourceTarget : targetSource)
  const sOff = { x: 0, y: 0 }
  const tOff = { x: 0, y: 0 }
  if (ds.x === de.x && ds.y === de.y) {
    // both ends leave the same way and are close on that axis: shorten one stub so it does not overshoot
    const diff = Math.abs(s[acc] - e[acc])
    if (diff <= ELBOW_STUB) {
      const gap = Math.min(ELBOW_STUB - 1, ELBOW_STUB - diff)
      if (ds[acc] === curr) sOff[acc] = (sg[acc] > s[acc] ? -1 : 1) * gap
      else tOff[acc] = (tg[acc] > e[acc] ? -1 : 1) * gap
    }
  } else {
    const opp: Axis = acc === 'x' ? 'y' : 'x'
    const sameDir = ds[acc] === de[opp]
    const sGt = sg[opp] > tg[opp]
    const sLt = sg[opp] < tg[opp]
    const swap = (ds[acc] === 1 && ((!sameDir && sGt) || (sameDir && sLt))) || (ds[acc] !== 1 && ((!sameDir && sLt) || (sameDir && sGt)))
    if (swap) mid = acc === 'x' ? sourceTarget : targetSource
  }
  return simplify([s, add(sg, sOff), ...mid, add(tg, tOff), e])
}

/** An orthogonal route through the given points: a corner is added between two points not on one axis
 * (the first leg along the exit direction, then alternating; the last leg along the entry direction). */
export function elbowThrough(s: Point, ds: Point, through: Point[], e: Point, de: Point): Point[] {
  const pts = [s, ...through, e]
  const out: Point[] = [s]
  const perp = (a: Axis): Axis => (a === 'x' ? 'y' : 'x')
  let prev: Axis = ds.x !== 0 ? 'y' : 'x'
  for (let i = 1; i < pts.length; i++) {
    const p = out[out.length - 1]
    const q = pts[i]
    const apartX = Math.abs(p.x - q.x) > EPS
    const apartY = Math.abs(p.y - q.y) > EPS
    if (apartX && apartY) {
      const first: Axis = i === pts.length - 1 ? perp(de.x !== 0 ? 'x' : 'y') : perp(prev)
      out.push(first === 'x' ? { x: q.x, y: p.y } : { x: p.x, y: q.y })
      prev = perp(first)
    } else if (apartX || apartY) prev = apartX ? 'x' : 'y'
    out.push(q)
  }
  return simplify(out)
}

/** The automatic curve: one cubic leaving and entering along the exit directions. */
export function curveAuto(s: Point, ds: Point, e: Point, de: Point): Seg {
  const k = Math.min(400, Math.max(12, dist(s, e) * 0.4))
  return { a: s, c1: add(s, mul(ds, k)), c2: add(e, mul(de, k)), b: e }
}

/** A smooth curve through the points: Hermite pieces with Catmull-Rom tangents inside and the exit
 * directions at the ends, each tangent scaled by a third of its piece. */
export function curveThrough(pts: Point[], ds: Point, de: Point): Seg[] {
  const n = pts.length
  const tangents = pts.map((p, i) => {
    if (i === 0) return unit(ds)
    if (i === n - 1) return mul(unit(de), -1)
    return unit(sub(pts[i + 1], pts[i - 1]), unit(sub(pts[i + 1], p)))
  })
  const segs: Seg[] = []
  for (let i = 0; i < n - 1; i++) {
    const a = pts[i], b = pts[i + 1]
    const l = dist(a, b) / 3
    segs.push({ a, b, c1: add(a, mul(tangents[i], l)), c2: sub(b, mul(tangents[i + 1], l)) })
  }
  return segs
}

export function cubicAt(sg: Seg, t: number): Point {
  if (!sg.c1 || !sg.c2) return { x: sg.a.x + (sg.b.x - sg.a.x) * t, y: sg.a.y + (sg.b.y - sg.a.y) * t }
  const u = 1 - t
  const k0 = u * u * u, k1 = 3 * u * u * t, k2 = 3 * u * t * t, k3 = t * t * t
  return { x: k0 * sg.a.x + k1 * sg.c1.x + k2 * sg.c2.x + k3 * sg.b.x, y: k0 * sg.a.y + k1 * sg.c1.y + k2 * sg.c2.y + k3 * sg.b.y }
}

function polylineSegs(pts: Point[]): Seg[] {
  const segs: Seg[] = []
  for (let i = 0; i < pts.length - 1; i++) segs.push({ a: pts[i], b: pts[i + 1] })
  return segs
}

// ---- measures

function flatten(segs: Seg[]): { flat: Point[]; cum: number[]; segOf: number[] } {
  const flat: Point[] = []
  const segOf: number[] = []
  segs.forEach((sg, i) => {
    if (i === 0) { flat.push(sg.a); segOf.push(0) }
    if (sg.c1 && sg.c2) for (let k = 1; k <= CURVE_SAMPLES; k++) { flat.push(cubicAt(sg, k / CURVE_SAMPLES)); segOf.push(i) }
    else { flat.push(sg.b); segOf.push(i) }
  })
  const cum = [0]
  for (let i = 1; i < flat.length; i++) cum.push(cum[i - 1] + dist(flat[i - 1], flat[i]))
  return { flat, cum, segOf }
}

/** The point at arc-length ratio `t` and the unit direction of travel there. */
export function pointAt(g: Pick<ConnectorGeometry, 'flat' | 'cum' | 'length'>, t: number): { point: Point; tangent: Point } {
  const { flat, cum } = g
  if (flat.length < 2) return { point: flat[0] ?? { x: 0, y: 0 }, tangent: { x: 1, y: 0 } }
  const target = Math.min(1, Math.max(0, t)) * g.length
  let i = 1
  while (i < flat.length - 1 && cum[i] < target) i += 1
  const a = flat[i - 1], b = flat[i]
  const piece = cum[i] - cum[i - 1]
  const f = piece > 1e-9 ? (target - cum[i - 1]) / piece : 0
  return { point: { x: a.x + (b.x - a.x) * f, y: a.y + (b.y - a.y) * f }, tangent: unit(sub(b, a)) }
}

/** The left-hand normal of a direction of travel (screen coordinates: y grows downwards). */
export function leftOf(tangent: Point): Point { return { x: tangent.y, y: -tangent.x } }

/** The nearest point of the path to `p`: its arc-length ratio, distance, and the route piece it is on. */
export function nearest(g: Pick<ConnectorGeometry, 'flat' | 'cum' | 'length' | 'segOf'>, p: Point): { t: number; point: Point; distance: number; seg: number; tangent: Point } {
  const { flat, cum } = g
  let best = { t: 0, point: flat[0] ?? p, distance: Infinity, seg: 0, tangent: { x: 1, y: 0 } }
  for (let i = 1; i < flat.length; i++) {
    const a = flat[i - 1], b = flat[i]
    const ab = sub(b, a)
    const l2 = ab.x * ab.x + ab.y * ab.y
    const f = l2 > 1e-12 ? Math.min(1, Math.max(0, ((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / l2)) : 0
    const q = { x: a.x + ab.x * f, y: a.y + ab.y * f }
    const d = dist(p, q)
    if (d < best.distance) best = { t: g.length > 0 ? (cum[i - 1] + (cum[i] - cum[i - 1]) * f) / g.length : 0, point: q, distance: d, seg: g.segOf[i], tangent: unit(ab) }
  }
  return best
}

// ---- labels

const LABEL_MAX_W = 240
const LINE_HEIGHT = 1.35
let measureCtx: CanvasRenderingContext2D | null | undefined
let measureFont = ''
function textWidth(text: string, size: number): number {
  if (measureCtx === undefined) {
    try {
      measureCtx = typeof document !== 'undefined' ? document.createElement('canvas').getContext('2d') : null
      measureFont = typeof document !== 'undefined' ? getComputedStyle(document.body).fontFamily || 'sans-serif' : 'sans-serif'
    } catch { measureCtx = null }
  }
  if (measureCtx) { measureCtx.font = `${size}px ${measureFont}`; return measureCtx.measureText(text).width }
  let w = 0
  for (const ch of text) w += /[⺀-￿]/.test(ch) ? size : size * 0.58
  return w
}

/** The label's box (world units) for a title at `font` size, centred on `at`: horizontal text, wrapped at 240. */
export function labelBox(title: string, font: number, at: Point): Rect {
  let lines = 0
  let widest = 0
  for (const line of title.split('\n')) {
    const w = textWidth(line, font)
    lines += Math.max(1, Math.ceil(w / LABEL_MAX_W))
    widest = Math.max(widest, Math.min(LABEL_MAX_W, w))
  }
  const w = widest + 8
  const h = lines * font * LINE_HEIGHT + 4
  return { x: at.x - w / 2, y: at.y - h / 2, w, h }
}

export function labelPoint(g: Pick<ConnectorGeometry, 'flat' | 'cum' | 'length'>, t: number, offset: number): Point {
  const { point, tangent } = pointAt(g, t)
  const n = leftOf(tangent)
  return { x: point.x + n.x * offset, y: point.y + n.y * offset }
}

// ---- the whole connector

export interface RouteOptions {
  /** The label text (Cell title) and its font size; no box without text. */
  title?: string | null
  font?: number
  /** World units around the path in `bounds` (line width, caps, hit slop). */
  pad: number
}

/** Resolve, route and measure a connector whose stored ends are `stored`. */
export function routeConnector(data: ConnectorData, stored: [Point, Point], lookup: TargetLookup, options: RouteOptions): ConnectorGeometry {
  const [start, end] = resolveEnds(data, stored, lookup)
  return routeBetween(data, start, end, options)
}

/** Route and measure between already resolved ends. */
export function routeBetween(data: Pick<ConnectorData, 'route' | 'controls' | 'label'>, start: ResolvedEnd, end: ResolvedEnd, options: RouteOptions): ConnectorGeometry {
  const s = start.point
  const e = end.point
  const through = data.controls.map((c) => compose(c, s, e))
  const auto = through.length === 0
  let pts: Point[]
  let segs: Seg[]
  if (data.route === 'elbow') {
    pts = auto ? elbowAuto(s, axisOf(start.dir.x, start.dir.y), e, axisOf(end.dir.x, end.dir.y)) : elbowThrough(s, axisOf(start.dir.x, start.dir.y), through, e, axisOf(end.dir.x, end.dir.y))
    if (pts.length < 2) pts = [s, e]
    segs = polylineSegs(pts)
  } else if (data.route === 'curve') {
    pts = [s, ...through, e]
    segs = auto ? [curveAuto(s, start.dir, e, end.dir)] : curveThrough(pts, start.dir, end.dir)
  } else {
    pts = [s, ...through, e]
    segs = polylineSegs(pts)
  }
  const { flat, cum, segOf } = flatten(segs)
  const length = cum[cum.length - 1] ?? 0
  const measured = { flat, cum, length }
  const labelAt = labelPoint(measured, data.label.t, data.label.offset)
  const box = options.title ? labelBox(options.title, options.font ?? 14, labelAt) : null
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity
  for (const p of flat) { x1 = Math.min(x1, p.x); y1 = Math.min(y1, p.y); x2 = Math.max(x2, p.x); y2 = Math.max(y2, p.y) }
  if (box) { x1 = Math.min(x1, box.x); y1 = Math.min(y1, box.y); x2 = Math.max(x2, box.x + box.w); y2 = Math.max(y2, box.y + box.h) }
  const pad = options.pad
  return {
    start, end, route: data.route, pts, auto, segs, flat, cum, segOf, length,
    label: { point: labelAt, box },
    bounds: { x: x1 - pad, y: y1 - pad, w: x2 - x1 + pad * 2, h: y2 - y1 + pad * 2 },
  }
}

/** The automatic route as editable points (the first drag of a line or curve fixes it, §4.4). */
export function materialize(g: ConnectorGeometry): Point[] {
  // an elbow keeps every corner (its segments are what gets dragged); a curve without points gets its middle
  if (g.route === 'curve' && g.auto) return [cubicAt(g.segs[0], 0.5)]
  return g.pts.slice(1, -1)
}

/** Distance from `p` to the drawn path (label box counts as on it). */
export function distanceTo(g: ConnectorGeometry, p: Point): number {
  const box = g.label.box
  if (box && p.x >= box.x && p.x <= box.x + box.w && p.y >= box.y && p.y <= box.y + box.h) return 0
  return nearest(g, p).distance
}

/** The route pieces, shortened at the ends by `trimStart` / `trimEnd` (the end gap and a cap's inset). */
export function trimmed(segs: Seg[], trimStart: number, trimEnd: number): Seg[] {
  if (segs.length === 0) return segs
  const out = segs.map((sg) => ({ ...sg }))
  const cut = (sg: Seg, fromStart: boolean, amount: number) => {
    if (amount <= 0) return
    const tip = fromStart ? sg.a : sg.b
    const toward = fromStart ? (sg.c1 ?? sg.b) : (sg.c2 ?? sg.a)
    const span = dist(tip, toward)
    const room = sg.c1 ? span : dist(sg.a, sg.b)
    const k = Math.min(amount, Math.max(0, room - 0.01))
    if (k <= 0 || span < 1e-9) return
    const moved = add(tip, mul(unit(sub(toward, tip)), k))
    if (fromStart) sg.a = moved
    else sg.b = moved
  }
  cut(out[0], true, trimStart)
  cut(out[out.length - 1], false, trimEnd)
  return out
}

/** The unit direction of travel at the start and at the end of the route. */
export function endTangents(segs: Seg[]): [Point, Point] {
  const first = segs[0]
  const last = segs[segs.length - 1]
  if (!first || !last) return [{ x: 1, y: 0 }, { x: 1, y: 0 }]
  const s = unit(sub(first.c1 && !samePoint(first.c1, first.a) ? first.c1 : first.b, first.a))
  const e = unit(sub(last.b, last.c2 && !samePoint(last.c2, last.b) ? last.c2 : last.a))
  return [s, e]
}

/** The SVG path data of the route; elbow corners rounded with `radius` (clamped to half the shorter leg). */
export function pathData(segs: Seg[], radius = 0): string {
  if (segs.length === 0) return ''
  const f = (n: number) => Math.round(n * 100) / 100
  let d = `M${f(segs[0].a.x)} ${f(segs[0].a.y)}`
  for (let i = 0; i < segs.length; i++) {
    const sg = segs[i]
    if (sg.c1 && sg.c2) { d += `C${f(sg.c1.x)} ${f(sg.c1.y)} ${f(sg.c2.x)} ${f(sg.c2.y)} ${f(sg.b.x)} ${f(sg.b.y)}`; continue }
    const next = segs[i + 1]
    if (radius > 0 && next && !next.c1) {
      const inLen = dist(sg.a, sg.b)
      const outLen = dist(next.a, next.b)
      const r = Math.min(radius, inLen / 2, outLen / 2)
      if (r > 0.5) {
        const before = sub(sg.b, mul(unit(sub(sg.b, sg.a)), r))
        const after = add(next.a, mul(unit(sub(next.b, next.a)), r))
        d += `L${f(before.x)} ${f(before.y)}Q${f(sg.b.x)} ${f(sg.b.y)} ${f(after.x)} ${f(after.y)}`
        continue
      }
    }
    d += `L${f(sg.b.x)} ${f(sg.b.y)}`
  }
  return d
}
