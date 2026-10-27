/* Pen and mouse input while a writing tool is on (手写笔 TODO D1): one stroke at a time, owned by one pointer.
 *
 *   down     a pen tip (or its eraser end), or the mouse's primary button, starts a stroke or an erase and takes
 *            the pointer (capture: it goes on outside the canvas); everything else that runs stops first
 *   move     only the owner's: its coalesced samples (or the event itself), in world coordinates of that moment,
 *            duplicates dropped; predicted samples only draw a temporary tip
 *   up       only the owner's: the stroke is saved once (its last point keeps the pressure it had — a pen's up
 *            reports 0); the capture's release that follows changes nothing
 *   cancel   pointercancel, a capture lost before the up, Esc, another tool or canvas, lost permission, a hidden
 *            page, unmounting: the stroke is dropped (an erase put back); saved strokes stay
 *
 * A second pen, another pointer's events and touches never touch the stroke. A stroke that reaches its budget is
 * saved and goes on as a new stroke from the same point (the host says so). Pure: tests drive it with plain events. */

import { pointOf, predictionsOf, rawBytes, RAW_BUDGET, sameSample, samplesOf, type InkPoint, type InkStyle, type PointerSample } from './model'
import type { Point } from '../ui/canvas/geometry'

export type InkTool = 'pen' | 'eraser'

export interface InkPointerEvent extends PointerSample {
  button: number
  buttons: number
}

/** What the canvas does for the input. */
export interface InkHost {
  toWorld: (clientX: number, clientY: number) => Point
  zoom: () => number
  tool: () => InkTool
  style: () => InkStyle
  capture: (pointerId: number) => void
  release: (pointerId: number) => void
  /** A stroke starts: touch gestures, object drags, camera animation and pending timers stop. */
  interrupt: () => void
  /** Live drawing of the stroke being written (world points, the predicted ones apart). */
  begin: (style: InkStyle) => void
  draw: (points: InkPoint[], predicted: InkPoint[]) => void
  /** A finished stroke (world points as sampled), written at `zoom`; `split`: it reached its budget and goes on. */
  stroke: (points: InkPoint[], style: InkStyle, zoom: number, split: boolean) => void
  /** The stroke was dropped: nothing is saved. */
  dropped: () => void
  /** The eraser moved from `a` to `b` (world), `radius` in world units. */
  erase: (a: Point, b: Point, radius: number) => void
  eraseEnd: (cancel: boolean) => void
}

/** The eraser's reach on screen (px). */
export const ERASER_PX = 10

interface Active {
  pointerId: number
  pointerType: string
  tool: InkTool
  style: InkStyle
  zoom: number
  start: number
  points: InkPoint[]
  bytes: number
  last: InkPoint
}

/** A pen's eraser end (button 5) or eraser button held. */
const eraserEnd = (event: InkPointerEvent) => event.button === 5 || (event.buttons & 32) !== 0

export class InkInput {
  private active: Active | null = null
  private readonly host: InkHost
  /** A finger writes too (the stage's board, which has no touch navigation); never on the canvas. */
  private readonly fingers: boolean

  constructor(host: InkHost, options: { fingers?: boolean } = {}) {
    this.host = host
    this.fingers = options.fingers ?? false
  }

  /** Is a stroke or an erase under way? */
  busy(): boolean { return this.active !== null }

  /** Does this pointer own the stroke under way? */
  owns(pointerId: number): boolean { return this.active?.pointerId === pointerId }

  /** A pointer came down on the writing area; true when it started a stroke or an erase. */
  down(event: InkPointerEvent): boolean {
    if (this.active) return false
    const finger = event.pointerType === 'touch' && this.fingers
    if (event.pointerType === 'mouse' || finger ? event.button !== 0 : event.pointerType !== 'pen' || (event.button !== 0 && !eraserEnd(event))) return false
    const tool: InkTool = event.pointerType === 'pen' && eraserEnd(event) ? 'eraser' : this.host.tool()
    this.host.interrupt()
    this.host.capture(event.pointerId)
    this.begin(event, tool)
    return true
  }

  private begin(event: PointerSample, tool: InkTool, from?: InkPoint) {
    const style = this.host.style()
    const first = from ? { ...from, t: 0 } : pointOf(event, this.host.toWorld(event.clientX, event.clientY), event.timeStamp, 0)
    this.active = { pointerId: event.pointerId, pointerType: event.pointerType, tool, style, zoom: this.host.zoom(), start: event.timeStamp, points: [first], bytes: rawBytes([first]), last: first }
    if (tool === 'pen') {
      this.host.begin(style)
      this.host.draw([first], [])
    } else {
      this.host.erase(first, first, this.radius())
    }
  }

  private radius() { return ERASER_PX / (this.active?.zoom ?? this.host.zoom()) }

  move(event: InkPointerEvent) {
    if (!this.active || event.pointerId !== this.active.pointerId) return
    let added: InkPoint[] = []
    for (const sample of samplesOf(event)) {
      const a = this.active
      // a sample after the tip left the surface (a pen reports its release as a move with no button) is not part of it
      if ((sample as Partial<InkPointerEvent>).buttons === 0) continue
      const point = pointOf(sample, this.host.toWorld(sample.clientX, sample.clientY), a.start, a.last.t)
      if (sameSample(a.last, point)) continue
      if (a.tool === 'eraser') { this.host.erase(a.last, point, this.radius()); a.last = point; continue }
      a.points.push(point)
      a.bytes += rawBytes([point])
      a.last = point
      added.push(point)
      if (a.bytes > RAW_BUDGET) {
        // saved now and continued from here as a new stroke: nothing the pen draws is cut off
        this.host.draw(added, [])
        added = []
        this.host.stroke(a.points, a.style, a.zoom, true)
        this.begin(sample, 'pen', point)
      }
    }
    const a = this.active
    if (a.tool === 'pen') {
      const predicted = predictionsOf(event).map((sample) => pointOf(sample, this.host.toWorld(sample.clientX, sample.clientY), a.start, a.last.t))
      if (added.length > 0 || predicted.length > 0) this.host.draw(added, predicted)
    }
  }

  up(event: InkPointerEvent) {
    const a = this.active
    if (!a || event.pointerId !== a.pointerId) return
    this.active = null
    const at = this.host.toWorld(event.clientX, event.clientY)
    if (a.tool === 'eraser') {
      if (at.x !== a.last.x || at.y !== a.last.y) this.host.erase(a.last, at, this.radius())
      this.host.release(a.pointerId)
      this.host.eraseEnd(false)
      return
    }
    // the up's own position counts, at the pressure the pen had (its 0 is the lift, not a lighter touch)
    if (at.x !== a.last.x || at.y !== a.last.y) {
      const last: InkPoint = { ...a.last, x: at.x, y: at.y, t: Math.max(a.last.t, Math.round(event.timeStamp - a.start)) }
      a.points.push(last)
      this.host.draw([last], [])
    }
    this.host.release(a.pointerId)
    this.host.stroke(a.points, a.style, a.zoom, false)
  }

  /** pointercancel of the owner (other pointers' cancels change nothing). */
  cancelPointer(pointerId: number) {
    if (this.owns(pointerId)) this.cancel()
  }

  /** The capture was lost while the stroke was still running (after an up there is nothing to lose). */
  lost(pointerId: number) {
    if (this.owns(pointerId)) this.cancel()
  }

  /** Drop the stroke under way (any reason); saved strokes are not affected. */
  cancel() {
    const a = this.active
    if (!a) return
    this.active = null
    this.host.release(a.pointerId)
    if (a.tool === 'eraser') this.host.eraseEnd(true)
    else this.host.dropped()
  }
}

/** What the canvas view asks of the writing area (Esc): is a stroke under way, and drop it. */
export class InkControl {
  private input: InkInput | null = null
  attach(input: InkInput | null) { this.input = input }
  busy(): boolean { return this.input?.busy() ?? false }
  cancel() { this.input?.cancel() }
}
