/* Touch gestures of the free canvas (UI improvement §16), after Miro's mobile canvas:
 *
 *   one finger      a drag pans, and a flick glides on; on the selected Block it scrolls that Block's own
 *                   scrollable content; in edit mode a drag that starts on a Block moves it (the mouse path)
 *   two fingers     pinch zooms about the midpoint while the midpoint pans; lifting one finger keeps panning
 *   tap             selects the Block under the finger, or clears the selection
 *   double tap      zooms to the Block (edit mode: edits it), or zooms in ×2 at a blank spot
 *   two-finger tap  zooms out ×2
 *   long press      the context menu of the Block or of the blank spot
 *
 * The browser does none of this itself: the canvas and everything in it is `touch-action: none`. This
 * class only recognises gestures and moves the camera; what a tap or a press means is the RenderHost's.
 * Moves and lifts arrive from window listeners, so a touch whose target left the DOM still ends. */

import type { Camera } from './camera'

export interface Point { x: number; y: number }

/** What a one-finger drag does, decided when the finger comes down. */
export type OneFingerDrag = { kind: 'pan' } | { kind: 'scroll'; element: HTMLElement } | { kind: 'delegate' }

export interface TouchActions {
  /** The canvas-relative point of a client point. */
  local: (clientX: number, clientY: number) => Point
  /** A first finger came down at `p`: what a drag from here does; null leaves the touch to the browser. */
  begin: (event: PointerEvent, p: Point) => OneFingerDrag | null
  /** A delegated one-finger gesture runs on the mouse path. */
  delegateMove: (event: PointerEvent) => void
  delegateEnd: (cancel: boolean) => void
  /** A pan or pinch is under way (the near toolbar hides meanwhile). */
  moving: (on: boolean) => void
  tap: (p: Point, target: Element | null) => void
  doubleTap: (p: Point) => void
  twoFingerTap: (p: Point) => void
  longPress: (p: Point) => void
}

/** How far a finger may move and still tap (px). */
const TAP_SLOP = 8
/** A two-finger tap lifts both fingers within this time (ms). */
const TWO_FINGER_TAP_MS = 300
/** The second tap of a double tap ends within this time of the first, near it. */
const DOUBLE_TAP_MS = 350
const DOUBLE_TAP_SLOP = 30
const LONG_PRESS_MS = 500
/** A flick faster than this (px/ms), measured over the last moves, glides on. */
const FLICK_MIN_SPEED = 0.3
const VELOCITY_WINDOW_MS = 100
/** A finger that rested this long before lifting does not flick. */
const FLICK_REST_MS = 60

interface Sample { x: number; y: number; t: number }

type Gesture =
  | {
    kind: 'one'; id: number; drag: OneFingerDrag; start: Point; last: Point; at: number; moved: boolean; pressed: boolean
    target: Element | null; samples: Sample[]
    /** Left over from a two-finger gesture that did not move: lifting this finger soon is a two-finger tap there. */
    twoTap: Point | null
  }
  | { kind: 'two'; ids: [number, number]; at: number; moved: boolean; tapEligible: boolean; startDist: number; startMid: Point; lastDist: number; lastMid: Point }

function distance(a: Point, b: Point): number {
  return Math.hypot(a.x - b.x, a.y - b.y)
}

function midpoint(a: Point, b: Point): Point {
  return { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 }
}

export class TouchGestures {
  private actions: TouchActions | null = null
  /** Client coordinates of the fingers this gesture follows. */
  private readonly points = new Map<number, Point>()
  private gesture: Gesture | null = null
  private frame = 0
  private pressTimer = 0
  private movingOn = false
  private lastTap: { at: number; p: Point } | null = null
  private readonly camera: Camera

  constructor(camera: Camera) {
    this.camera = camera
  }

  /** Called by the RenderHost after every render: the callbacks see its current props. */
  setActions(actions: TouchActions) {
    this.actions = actions
  }

  down(event: PointerEvent) {
    const actions = this.actions
    if (!actions) return
    const client = { x: event.clientX, y: event.clientY }
    if (this.points.size === 0) {
      this.camera.stopAnimation()
      const p = actions.local(client.x, client.y)
      const drag = actions.begin(event, p)
      if (!drag) return
      this.points.set(event.pointerId, client)
      const target = event.target instanceof Element ? event.target : null
      this.gesture = { kind: 'one', id: event.pointerId, drag, start: p, last: p, at: performance.now(), moved: false, pressed: false, target, samples: [{ ...p, t: performance.now() }], twoTap: null }
      if (drag.kind !== 'delegate') this.pressTimer = window.setTimeout(() => this.press(), LONG_PRESS_MS)
      return
    }
    // a third finger is ignored; a second one turns any one-finger gesture into a pinch
    if (this.points.size >= 2 || !this.gesture) return
    this.points.set(event.pointerId, client)
    this.clearPress()
    const previous = this.gesture
    if (previous.kind === 'one' && previous.drag.kind === 'delegate') actions.delegateEnd(true)
    const ids = [...this.points.keys()] as [number, number]
    const a = this.localOf(ids[0])
    const b = this.localOf(ids[1])
    if (!a || !b) return
    const dist = Math.max(1, distance(a, b))
    const mid = midpoint(a, b)
    this.gesture = { kind: 'two', ids, at: performance.now(), moved: false, tapEligible: previous.kind === 'one' && !previous.moved && !previous.pressed, startDist: dist, startMid: mid, lastDist: dist, lastMid: mid }
    this.setMoving(true)
  }

  move(event: PointerEvent) {
    if (!this.points.has(event.pointerId)) return
    this.points.set(event.pointerId, { x: event.clientX, y: event.clientY })
    const g = this.gesture
    if (g?.kind === 'one' && g.drag.kind === 'delegate') { this.actions?.delegateMove(event); return }
    if (!this.frame) this.frame = requestAnimationFrame(() => this.flush())
  }

  up(event: PointerEvent, cancel: boolean) {
    if (!this.points.has(event.pointerId)) return
    // the last position counts: a quick gesture must not lose its final move
    if (this.frame) { cancelAnimationFrame(this.frame); this.frame = 0; this.flush() }
    this.points.delete(event.pointerId)
    const g = this.gesture
    const actions = this.actions
    if (!g || !actions) { if (this.points.size === 0) this.reset(); return }
    if (g.kind === 'two') {
      const rest = g.ids.find((id) => this.points.has(id))
      if (rest !== undefined && !cancel) {
        // one finger stays: it pans on (a pinch that did not move may still become a two-finger tap)
        const p = this.localOf(rest) ?? g.lastMid
        this.gesture = { kind: 'one', id: rest, drag: { kind: 'pan' }, start: p, last: p, at: g.at, moved: g.moved, pressed: false, target: null, samples: [{ ...p, t: performance.now() }], twoTap: g.moved || !g.tapEligible ? null : g.lastMid }
        return
      }
      if (!cancel && !g.moved && g.tapEligible && performance.now() - g.at < TWO_FINGER_TAP_MS) actions.twoFingerTap(g.lastMid)
      this.end()
      return
    }
    if (g.id !== event.pointerId) return
    this.clearPress()
    if (g.drag.kind === 'delegate') { this.gesture = null; this.points.clear(); actions.delegateEnd(cancel); return }
    if (cancel || g.pressed) { this.end(); return }
    if (g.moved) {
      if (g.drag.kind === 'pan') this.flick(g.samples)
      this.end()
      return
    }
    if (g.twoTap) {
      if (performance.now() - g.at < TWO_FINGER_TAP_MS) actions.twoFingerTap(g.twoTap)
      this.end()
      return
    }
    // a tap; a second one soon after at the same spot makes it a double tap
    const now = performance.now()
    const last = this.lastTap
    this.end()
    if (last && now - last.at < DOUBLE_TAP_MS && distance(last.p, g.start) < DOUBLE_TAP_SLOP) {
      this.lastTap = null
      actions.doubleTap(g.start)
    } else {
      this.lastTap = { at: now, p: g.start }
      actions.tap(g.start, g.target)
    }
  }

  /** Forget everything (the canvas unmounts or refuses gestures). */
  reset() {
    this.clearPress()
    if (this.frame) cancelAnimationFrame(this.frame)
    this.frame = 0
    this.points.clear()
    this.gesture = null
    this.setMoving(false)
  }

  private end() {
    this.gesture = null
    if (this.points.size === 0) this.setMoving(false)
  }

  private setMoving(on: boolean) {
    if (this.movingOn === on) return
    this.movingOn = on
    this.actions?.moving(on)
  }

  private clearPress() {
    if (this.pressTimer) window.clearTimeout(this.pressTimer)
    this.pressTimer = 0
  }

  private press() {
    this.pressTimer = 0
    const g = this.gesture
    if (!g || g.kind !== 'one' || g.moved || g.drag.kind === 'delegate' || this.points.size !== 1) return
    g.pressed = true
    this.actions?.longPress(g.start)
  }

  private localOf(id: number): Point | null {
    const client = this.points.get(id)
    return client && this.actions ? this.actions.local(client.x, client.y) : null
  }

  /** Apply the fingers' latest positions (once per frame). */
  private flush() {
    this.frame = 0
    const g = this.gesture
    if (!g) return
    if (g.kind === 'one') {
      const p = this.localOf(g.id)
      if (!p) return
      if (!g.moved) {
        if (distance(p, g.start) < TAP_SLOP) return
        g.moved = true
        this.clearPress()
        this.setMoving(true)
      }
      const dx = p.x - g.last.x
      const dy = p.y - g.last.y
      g.last = p
      const now = performance.now()
      g.samples.push({ ...p, t: now })
      while (g.samples.length > 2 && now - g.samples[0].t > VELOCITY_WINDOW_MS) g.samples.shift()
      if (g.drag.kind === 'scroll') { g.drag.element.scrollLeft -= dx; g.drag.element.scrollTop -= dy } else this.camera.panBy(dx, dy)
      return
    }
    const a = this.localOf(g.ids[0])
    const b = this.localOf(g.ids[1])
    if (!a || !b) return
    const dist = Math.max(1, distance(a, b))
    const mid = midpoint(a, b)
    if (!g.moved) {
      if (Math.abs(dist - g.startDist) < TAP_SLOP && distance(mid, g.startMid) < TAP_SLOP) return
      g.moved = true
    }
    // pan first (the world point under the old midpoint goes under the new one), then zoom about it
    this.camera.panBy(mid.x - g.lastMid.x, mid.y - g.lastMid.y)
    this.camera.zoomAt(mid.x, mid.y, dist / g.lastDist)
    g.lastDist = dist
    g.lastMid = mid
  }

  private flick(samples: Sample[]) {
    const now = performance.now()
    const last = samples[samples.length - 1]
    const first = samples.find((s) => now - s.t <= VELOCITY_WINDOW_MS + FLICK_REST_MS) ?? samples[0]
    if (!last || !first || now - last.t > FLICK_REST_MS || last.t - first.t <= 0) return
    const vx = (last.x - first.x) / (last.t - first.t)
    const vy = (last.y - first.y) / (last.t - first.t)
    if (Math.hypot(vx, vy) >= FLICK_MIN_SPEED) this.camera.glide(vx, vy)
  }
}
