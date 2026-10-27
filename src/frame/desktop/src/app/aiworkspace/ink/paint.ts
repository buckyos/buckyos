/* The live ink layer (手写笔 TODO D2): two screen-space canvases over a canvas or a stage.
 *
 *   base   the strokes waiting for their Blocks and the finished pieces of the stroke being written, drawn once each
 *          (a new sample draws only the pieces it completed); redrawn whole only when the camera, the size, the
 *          device pixel ratio or the set of waiting strokes changes
 *   tail   cleared and drawn per sample: the last, still open piece up to the pen and the predicted points ahead
 *          of it (replaced by the real ones as they come; never stored), and the eraser's ring
 *
 * Pieces are those of `model.ts`, at the same widths, so a finished stroke looks the same when its Block takes over. */

import { pieceAt, type Piece, type WidePoint } from './model'
import type { Point } from '../ui/canvas/geometry'

/** World → screen (the layer's own pixels) and the scale of widths. */
export interface InkView { toScreen: (x: number, y: number) => Point; zoom: () => number }

export interface DrawnStroke { points: readonly WidePoint[]; color: string }

function fit(canvas: HTMLCanvasElement): CanvasRenderingContext2D | null {
  const dpr = window.devicePixelRatio || 1
  const w = canvas.clientWidth
  const h = canvas.clientHeight
  if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(h * dpr)) {
    canvas.width = Math.round(w * dpr)
    canvas.height = Math.round(h * dpr)
  }
  const ctx = canvas.getContext('2d')
  ctx?.setTransform(dpr, 0, 0, dpr, 0, 0)
  ctx?.clearRect(0, 0, w, h)
  return ctx
}

/** Draw one piece in screen space. */
export function paintPiece(ctx: CanvasRenderingContext2D, piece: Piece, view: InkView, color: string) {
  const zoom = view.zoom()
  const a = view.toScreen(piece.a.x, piece.a.y)
  const b = view.toScreen(piece.b.x, piece.b.y)
  const width = piece.w * zoom
  if (!piece.c && a.x === b.x && a.y === b.y) {
    ctx.fillStyle = color
    ctx.beginPath()
    ctx.arc(a.x, a.y, width / 2, 0, Math.PI * 2)
    ctx.fill()
    return
  }
  ctx.strokeStyle = color
  ctx.lineWidth = width
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  ctx.beginPath()
  ctx.moveTo(a.x, a.y)
  if (piece.c) { const c = view.toScreen(piece.c.x, piece.c.y); ctx.quadraticCurveTo(c.x, c.y, b.x, b.y) } else ctx.lineTo(b.x, b.y)
  ctx.stroke()
}

/** A whole stroke (a waiting one, or a board mark on stage). */
export function paintStroke(ctx: CanvasRenderingContext2D, stroke: DrawnStroke, view: InkView) {
  for (let i = 0; i < stroke.points.length; i++) paintPiece(ctx, pieceAt(stroke.points, i), view, stroke.color)
}

export class InkCanvas {
  private base: HTMLCanvasElement | null = null
  private tail: HTMLCanvasElement | null = null
  private view: InkView
  /** The stroke being written: its points so far and how many of its pieces are on the base canvas. */
  private active: { points: WidePoint[]; color: string; drawn: number } | null = null
  private waiting: readonly DrawnStroke[] = []
  private ring: { at: Point; r: number } | null = null
  private predicted: WidePoint[] = []
  /** Whether each canvas may show something (a pan with nothing drawn touches neither). */
  private baseShown = true
  private tailShown = true

  constructor(view: InkView) {
    this.view = view
  }

  /** Another camera (the stage's page changed). */
  setView(view: InkView) {
    this.view = view
    this.redraw()
  }

  attach(base: HTMLCanvasElement | null, tail: HTMLCanvasElement | null) {
    this.base = base
    this.tail = tail
    this.redraw()
  }

  /** The strokes waiting for their Blocks. */
  setWaiting(strokes: readonly DrawnStroke[]) {
    this.waiting = strokes
    this.redraw()
  }

  begin(color: string) {
    this.active = { points: [], color, drawn: 0 }
    this.predicted = []
    this.redraw()
  }

  /** New samples of the stroke: the pieces they complete go on the base canvas, the open end on the tail. */
  add(points: readonly WidePoint[], predicted: readonly WidePoint[] = []) {
    const active = this.active
    if (!active) return
    active.points.push(...points)
    this.predicted = [...predicted]
    const ctx = this.base?.getContext('2d')
    if (ctx) {
      this.baseShown = true
      for (; active.drawn < active.points.length - 1; active.drawn++) paintPiece(ctx, pieceAt(active.points, active.drawn), this.view, active.color)
    }
    this.paintTail()
  }

  /** The stroke ends (saved or cancelled): what stays is up to the waiting list. */
  end() {
    this.active = null
    this.predicted = []
    this.redraw()
  }

  /** The eraser's ring at a screen point (null hides it). */
  setRing(ring: { at: Point; r: number } | null) {
    this.ring = ring
    this.paintTail()
  }

  /** Everything again (camera, size or pixel ratio changed). */
  redraw() {
    const base = this.base
    if (base && (this.baseShown || this.waiting.length > 0 || this.active)) {
      this.baseShown = this.waiting.length > 0 || this.active !== null
      const ctx = fit(base)
      if (ctx) {
        for (const stroke of this.waiting) paintStroke(ctx, stroke, this.view)
        const active = this.active
        if (active) for (let i = 0; i < active.points.length - 1; i++) paintPiece(ctx, pieceAt(active.points, i), this.view, active.color)
      }
    }
    this.paintTail()
  }

  private paintTail() {
    const tail = this.tail
    if (!tail || (!this.tailShown && !this.active && !this.ring)) return
    this.tailShown = this.active !== null || this.ring !== null
    const ctx = fit(tail)
    if (!ctx) return
    const active = this.active
    if (active && active.points.length > 0) {
      const n = active.points.length
      paintPiece(ctx, pieceAt(active.points, n - 1), this.view, active.color)
      // the predicted path ahead of the pen, drawn from the last real point
      let prev = active.points[n - 1]
      for (const q of this.predicted) { paintPiece(ctx, { a: prev, b: q, w: q.w }, this.view, active.color); prev = q }
    }
    if (this.ring) {
      ctx.strokeStyle = 'rgba(100, 116, 139, 0.9)'
      ctx.lineWidth = 1.5
      ctx.beginPath()
      ctx.arc(this.ring.at.x, this.ring.at.y, this.ring.r, 0, Math.PI * 2)
      ctx.stroke()
    }
  }
}
