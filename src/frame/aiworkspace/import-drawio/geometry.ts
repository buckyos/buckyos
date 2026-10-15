/* Plane geometry shared by the canvas and the importer: points, rectangles, rotation about a centre. */

export interface Point { x: number; y: number }
export interface Rect { x: number; y: number; w: number; h: number }

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

/** A rotation as stored: `[0, 360)`, two decimals. */
export function normalizeRotation(deg: number): number {
  const r = Math.round((((deg % 360) + 360) % 360) * 100) / 100
  return r >= 360 ? 0 : r
}

export function unionRects(rects: Rect[]): Rect | null {
  if (rects.length === 0) return null
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity
  for (const r of rects) { x1 = Math.min(x1, r.x); y1 = Math.min(y1, r.y); x2 = Math.max(x2, r.x + r.w); y2 = Math.max(y2, r.y + r.h) }
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 }
}

/** The axis-aligned box around `rect` turned by `deg` about its centre. */
export function rotatedBounds(rect: Rect, deg: number): Rect {
  if (!deg) return rect
  const c = centerOf(rect)
  const pts = [{ x: rect.x, y: rect.y }, { x: rect.x + rect.w, y: rect.y }, { x: rect.x + rect.w, y: rect.y + rect.h }, { x: rect.x, y: rect.y + rect.h }].map((p) => rotatePoint(p, c, deg))
  const xs = pts.map((p) => p.x)
  const ys = pts.map((p) => p.y)
  const x = Math.min(...xs)
  const y = Math.min(...ys)
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y }
}
