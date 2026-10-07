/* Camera (phase two §9.2, §9.3 rule 1): the viewport of a free Surface. It lives outside React:
 * panning and zooming change one CSS transform on the world layer and notify listeners; nothing
 * re-renders per frame. `onSettle` fires once the gesture stopped (LOD, culling, persistence).
 * Touch gestures (UI improvement §16) also animate it: a glide after a flick and a smooth move to a
 * target; any other camera change stops them. */

import type { Placement } from '../../../api/types'

export interface Viewport { x: number; y: number; zoom: number }
export interface Rect { x: number; y: number; w: number; h: number }

export const MIN_ZOOM = 0.05
export const MAX_ZOOM = 4
const SETTLE_MS = 120
/** A glide loses this share of its speed per 16 ms frame and stops below the minimum speed (px/ms). */
const GLIDE_DECAY = 0.95
const GLIDE_MIN_SPEED = 0.02

function clampZoom(zoom: number): number {
  return Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom))
}

export class Camera {
  x = 0
  y = 0
  zoom = 1
  private world: HTMLElement | null = null
  private readonly changeListeners = new Set<() => void>()
  private readonly settleListeners = new Set<() => void>()
  private settleTimer: number | null = null
  private animation = 0
  private viewportSize = { w: 800, h: 600 }
  /** Screen margins covered by the floating toolbars (UI improvement §3.1): fitting and centring use the rest. */
  private insets = { top: 0, right: 0, bottom: 0, left: 0 }

  attach(world: HTMLElement | null) {
    this.world = world
    this.apply()
  }

  setViewportSize(w: number, h: number) {
    this.viewportSize = { w, h }
  }

  setInsets(insets: { top: number; right: number; bottom: number; left: number }) {
    this.insets = insets
  }

  /** The unobstructed part of the viewport, in screen coordinates. */
  get clearArea(): Rect {
    const { w, h } = this.viewportSize
    const { top, right, bottom, left } = this.insets
    const cw = Math.max(80, w - left - right)
    const ch = Math.max(80, h - top - bottom)
    return { x: Math.min(left, w - cw), y: Math.min(top, h - ch), w: cw, h: ch }
  }

  /** Centre of the unobstructed area (button zoom anchor, insertion point). */
  get center(): { x: number; y: number } {
    const area = this.clearArea
    return { x: area.x + area.w / 2, y: area.y + area.h / 2 }
  }

  /** Zoom to an absolute value around the centre of the unobstructed area. */
  zoomTo(zoom: number) {
    const c = this.center
    this.zoomAt(c.x, c.y, Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, zoom)) / this.zoom)
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
    this.stopAnimation()
    if (viewport.x !== undefined) this.x = viewport.x
    if (viewport.y !== undefined) this.y = viewport.y
    if (viewport.zoom !== undefined) this.zoom = clampZoom(viewport.zoom)
    this.apply()
  }

  panBy(dxScreen: number, dyScreen: number) {
    this.stopAnimation()
    this.x -= dxScreen / this.zoom
    this.y -= dyScreen / this.zoom
    this.apply()
  }

  /** Zoom keeping the world point under (sx, sy) fixed. */
  zoomAt(sx: number, sy: number, factor: number) {
    this.stopAnimation()
    const next = this.zoomedAt(sx, sy, factor)
    this.zoom = next.zoom
    this.x = next.x
    this.y = next.y
    this.apply()
  }

  /** The viewport `zoomAt` would give. */
  zoomedAt(sx: number, sy: number, factor: number): Viewport {
    const zoom = clampZoom(this.zoom * factor)
    const wx = this.x + sx / this.zoom
    const wy = this.y + sy / this.zoom
    return { x: wx - sx / zoom, y: wy - sy / zoom, zoom }
  }

  /** Stop a running glide or animated move (the camera stays where it is). */
  stopAnimation() {
    if (this.animation) cancelAnimationFrame(this.animation)
    this.animation = 0
  }

  /** Move smoothly to `target`. A zoom change scales about the one screen point both viewports agree on, so a
   * zoom at a point keeps that point still all the way. */
  animateTo(target: Viewport, duration = 240) {
    this.stopAnimation()
    const from = this.viewport
    const zoom = clampZoom(target.zoom)
    const scaling = Math.abs(zoom - from.zoom) > 1e-9
    const k = 1 / from.zoom - 1 / zoom
    const anchor = scaling ? { x: (target.x - from.x) / k, y: (target.y - from.y) / k } : { x: 0, y: 0 }
    const world = { x: from.x + anchor.x / from.zoom, y: from.y + anchor.y / from.zoom }
    const start = performance.now()
    const step = (now: number) => {
      const t = Math.min(1, (now - start) / duration)
      const e = 1 - (1 - t) ** 3
      if (scaling) {
        this.zoom = from.zoom * (zoom / from.zoom) ** e
        this.x = world.x - anchor.x / this.zoom
        this.y = world.y - anchor.y / this.zoom
      } else {
        this.x = from.x + (target.x - from.x) * e
        this.y = from.y + (target.y - from.y) * e
      }
      this.animation = t < 1 ? requestAnimationFrame(step) : 0
      this.apply()
    }
    this.animation = requestAnimationFrame(step)
  }

  /** Keep panning after a flick at `vx`, `vy` screen px/ms, slowing down until it stops. */
  glide(vx: number, vy: number) {
    this.stopAnimation()
    let last = performance.now()
    let speed = { x: vx, y: vy }
    const step = (now: number) => {
      const dt = Math.min(64, now - last)
      last = now
      this.x -= (speed.x * dt) / this.zoom
      this.y -= (speed.y * dt) / this.zoom
      const decay = GLIDE_DECAY ** (dt / 16)
      speed = { x: speed.x * decay, y: speed.y * decay }
      this.animation = Math.hypot(speed.x, speed.y) > GLIDE_MIN_SPEED ? requestAnimationFrame(step) : 0
      this.apply()
    }
    this.animation = requestAnimationFrame(step)
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

  /** The smallest pan that brings `rect` (world) fully into the unobstructed area; a rect larger than it is fitted. */
  ensureVisible(rect: Rect, margin = 24) {
    const area = this.clearArea
    const r = this.rectToScreen(rect)
    if (r.w > area.w - 2 * margin || r.h > area.h - 2 * margin) { this.fit(rect); return }
    let dx = 0
    let dy = 0
    if (r.x < area.x + margin) dx = area.x + margin - r.x
    else if (r.x + r.w > area.x + area.w - margin) dx = area.x + area.w - margin - (r.x + r.w)
    if (r.y < area.y + margin) dy = area.y + margin - r.y
    else if (r.y + r.h > area.y + area.h - margin) dy = area.y + area.h - margin - (r.y + r.h)
    if (dx !== 0 || dy !== 0) this.panBy(dx, dy)
  }

  /** Fit `rect` into the unobstructed area with padding. */
  fit(rect: Rect, padding = 40) {
    this.stopAnimation()
    const next = this.fitted(rect, padding)
    this.zoom = next.zoom
    this.x = next.x
    this.y = next.y
    this.apply()
  }

  /** The viewport `fit` would give. */
  fitted(rect: Rect, padding = 40): Viewport {
    const area = this.clearArea
    const zoom = clampZoom(Math.min((area.w - 2 * padding) / Math.max(1, rect.w), (area.h - 2 * padding) / Math.max(1, rect.h)))
    return { x: rect.x - (area.x + area.w / 2) / zoom + rect.w / 2, y: rect.y - (area.y + area.h / 2) / zoom + rect.h / 2, zoom }
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
