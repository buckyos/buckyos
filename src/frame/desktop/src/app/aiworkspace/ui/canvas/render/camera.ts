/* Camera (phase two §9.2, §9.3 rule 1): the viewport of a free Surface. It lives outside React:
 * panning and zooming change one CSS transform on the world layer and notify listeners; nothing
 * re-renders per frame. `onSettle` fires once the gesture stopped (LOD, culling, persistence). */

import type { Placement } from '../../../api/types'

export interface Viewport { x: number; y: number; zoom: number }
export interface Rect { x: number; y: number; w: number; h: number }

export const MIN_ZOOM = 0.05
export const MAX_ZOOM = 4
const SETTLE_MS = 120

export class Camera {
  x = 0
  y = 0
  zoom = 1
  private world: HTMLElement | null = null
  private readonly changeListeners = new Set<() => void>()
  private readonly settleListeners = new Set<() => void>()
  private settleTimer: number | null = null
  private viewportSize = { w: 800, h: 600 }

  attach(world: HTMLElement | null) {
    this.world = world
    this.apply()
  }

  setViewportSize(w: number, h: number) {
    this.viewportSize = { w, h }
  }

  get viewport(): Viewport { return { x: this.x, y: this.y, zoom: this.zoom } }

  /** The world rectangle the viewport shows. */
  get visibleRect(): Rect { return { x: this.x, y: this.y, w: this.viewportSize.w / this.zoom, h: this.viewportSize.h / this.zoom } }

  /** Stable zoom used for LOD decisions (the value at the last settle). */
  settledZoom = 1

  onChange(listener: () => void): () => void {
    this.changeListeners.add(listener)
    return () => { this.changeListeners.delete(listener) }
  }

  onSettle(listener: () => void): () => void {
    this.settleListeners.add(listener)
    return () => { this.settleListeners.delete(listener) }
  }

  private apply() {
    if (this.world) this.world.style.transform = `translate(${-this.x * this.zoom}px, ${-this.y * this.zoom}px) scale(${this.zoom})`
    for (const listener of [...this.changeListeners]) listener()
    if (this.settleTimer !== null) window.clearTimeout(this.settleTimer)
    this.settleTimer = window.setTimeout(() => {
      this.settleTimer = null
      this.settledZoom = this.zoom
      for (const listener of [...this.settleListeners]) listener()
    }, SETTLE_MS)
  }

  set(viewport: Partial<Viewport>) {
    if (viewport.x !== undefined) this.x = viewport.x
    if (viewport.y !== undefined) this.y = viewport.y
    if (viewport.zoom !== undefined) this.zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, viewport.zoom))
    this.apply()
  }

  panBy(dxScreen: number, dyScreen: number) {
    this.x -= dxScreen / this.zoom
    this.y -= dyScreen / this.zoom
    this.apply()
  }

  /** Zoom keeping the world point under (sx, sy) fixed. */
  zoomAt(sx: number, sy: number, factor: number) {
    const next = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, this.zoom * factor))
    const wx = this.x + sx / this.zoom
    const wy = this.y + sy / this.zoom
    this.zoom = next
    this.x = wx - sx / next
    this.y = wy - sy / next
    this.apply()
  }

  toWorld(sx: number, sy: number): { x: number; y: number } {
    return { x: this.x + sx / this.zoom, y: this.y + sy / this.zoom }
  }

  toScreen(wx: number, wy: number): { x: number; y: number } {
    return { x: (wx - this.x) * this.zoom, y: (wy - this.y) * this.zoom }
  }

  rectToScreen(r: Rect): Rect {
    const p = this.toScreen(r.x, r.y)
    return { x: p.x, y: p.y, w: r.w * this.zoom, h: r.h * this.zoom }
  }

  /** The world rectangle visible now, expanded by `margin` viewports on each side. */
  visibleWorld(margin = 0): Rect {
    const { w, h } = this.viewportSize
    const ww = w / this.zoom
    const wh = h / this.zoom
    return { x: this.x - ww * margin, y: this.y - wh * margin, w: ww * (1 + 2 * margin), h: wh * (1 + 2 * margin) }
  }

  /** Fit `rect` into the viewport with padding. */
  /** The smallest pan that brings `rect` (world) fully into view; a rect larger than the viewport is fitted. */
  ensureVisible(rect: Rect, margin = 24) {
    const { w, h } = this.viewportSize
    const r = this.rectToScreen(rect)
    if (r.w > w - 2 * margin || r.h > h - 2 * margin) { this.fit(rect); return }
    let dx = 0
    let dy = 0
    if (r.x < margin) dx = margin - r.x
    else if (r.x + r.w > w - margin) dx = w - margin - (r.x + r.w)
    if (r.y < margin) dy = margin - r.y
    else if (r.y + r.h > h - margin) dy = h - margin - (r.y + r.h)
    if (dx !== 0 || dy !== 0) this.panBy(dx, dy)
  }

  fit(rect: Rect, padding = 40) {
    const { w, h } = this.viewportSize
    const zoom = Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, Math.min((w - 2 * padding) / Math.max(1, rect.w), (h - 2 * padding) / Math.max(1, rect.h))))
    this.zoom = zoom
    this.x = rect.x - (w / zoom - rect.w) / 2
    this.y = rect.y - (h / zoom - rect.h) / 2
    this.apply()
  }
}

export function union(rects: Rect[]): Rect | null {
  if (rects.length === 0) return null
  let x1 = Infinity, y1 = Infinity, x2 = -Infinity, y2 = -Infinity
  for (const r of rects) { x1 = Math.min(x1, r.x); y1 = Math.min(y1, r.y); x2 = Math.max(x2, r.x + r.w); y2 = Math.max(y2, r.y + r.h) }
  return { x: x1, y: y1, w: x2 - x1, h: y2 - y1 }
}

export function intersects(a: Rect, b: Rect): boolean {
  return a.x < b.x + b.w && a.x + a.w > b.x && a.y < b.y + b.h && a.y + a.h > b.y
}

export function contains(outer: Rect, inner: Rect): boolean {
  return inner.x >= outer.x && inner.y >= outer.y && inner.x + inner.w <= outer.x + outer.w && inner.y + inner.h <= outer.y + outer.h
}

export function placementOf(r: Rect): Placement {
  return { x: Math.round(r.x), y: Math.round(r.y), w: Math.max(1, Math.round(r.w)), h: Math.max(1, Math.round(r.h)) }
}
