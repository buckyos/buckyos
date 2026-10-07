/* Rotated rectangles (标准对象的交互改进 §7.1): a placement is an axis-aligned rect plus a rotation in degrees,
 * clockwise about its centre. Culling and the spatial index use the axis-aligned bounds; hit testing,
 * handles and resizing work in the rect's own (unrotated) coordinates. */

import type { Rect } from './render/camera'

export interface Point { x: number; y: number }

/** A rotation as stored: `[0, 360)`, two decimals. */
export function normalizeRotation(deg: number): number {
  const r = Math.round((((deg % 360) + 360) % 360) * 100) / 100
  return r >= 360 ? 0 : r
}

export function centerOf(rect: Rect): Point {
  return { x: rect.x + rect.w / 2, y: rect.y + rect.h / 2 }
}

/** `p` turned by `deg` (clockwise on screen) about `about`. */
export function rotatePoint(p: Point, about: Point, deg: number): Point {
  if (!deg) return p
  const a = (deg * Math.PI) / 180
  const cos = Math.cos(a)
  const sin = Math.sin(a)
  const dx = p.x - about.x
  const dy = p.y - about.y
  return { x: about.x + dx * cos - dy * sin, y: about.y + dx * sin + dy * cos }
}

/** The four corners nw, ne, se, sw of the rotated rect. */
export function corners(rect: Rect, deg: number): Point[] {
  const c = centerOf(rect)
  return [
    { x: rect.x, y: rect.y }, { x: rect.x + rect.w, y: rect.y }, { x: rect.x + rect.w, y: rect.y + rect.h }, { x: rect.x, y: rect.y + rect.h },
  ].map((p) => rotatePoint(p, c, deg))
}

/** The axis-aligned box around the rotated rect. */
export function rotatedBounds(rect: Rect, deg: number): Rect {
  if (!deg) return rect
  const pts = corners(rect, deg)
  const xs = pts.map((p) => p.x)
  const ys = pts.map((p) => p.y)
  const x = Math.min(...xs)
  const y = Math.min(...ys)
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y }
}

/** Is the point inside the rotated rect (edges included, widened by `slop`)? */
export function containsPoint(rect: Rect, deg: number, x: number, y: number, slop = 0): boolean {
  const p = rotatePoint({ x, y }, centerOf(rect), -deg)
  return p.x >= rect.x - slop && p.x <= rect.x + rect.w + slop && p.y >= rect.y - slop && p.y <= rect.y + rect.h + slop
}

/** A resize handle's new rect: the opposite side stays where it is on screen, also when rotated.
 * `handle` is a compass id (n, ne, e, …); `delta` is the pointer movement in world units. */
export function resizeRotated(start: Rect, deg: number, handle: string, delta: Point, options: { min: number; aspect: boolean }): Rect {
  const local = rotatePoint(delta, { x: 0, y: 0 }, -deg)
  const sx = handle.includes('e') ? 1 : handle.includes('w') ? -1 : 0
  const sy = handle.includes('s') ? 1 : handle.includes('n') ? -1 : 0
  let w = sx ? Math.max(options.min, start.w + sx * local.x) : start.w
  let h = sy ? Math.max(options.min, start.h + sy * local.y) : start.h
  if (options.aspect && sx && sy) {
    const scale = Math.max(w / start.w, h / start.h, options.min / Math.min(start.w, start.h))
    w = start.w * scale
    h = start.h * scale
  }
  const shift = rotatePoint({ x: (sx * (w - start.w)) / 2, y: (sy * (h - start.h)) / 2 }, { x: 0, y: 0 }, deg)
  const c = centerOf(start)
  return { x: c.x + shift.x - w / 2, y: c.y + shift.y - h / 2, w, h }
}

/** The angle of `p` seen from `c`, in degrees clockwise from "up". */
export function angleFrom(c: Point, p: Point): number {
  return (Math.atan2(p.y - c.y, p.x - c.x) * 180) / Math.PI + 90
}
