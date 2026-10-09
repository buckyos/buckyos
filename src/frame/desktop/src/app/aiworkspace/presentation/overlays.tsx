/* The presenter's marks on stage (第三期规划 §10.3): a laser pointer whose trail fades, and ink — pen, eraser, undo,
 * clear. Both live in this show's memory only; nothing is written to the document. Ink strokes are kept in world
 * coordinates of their Surface (so they scale with the canvas and stay when the page turns) with time offsets and the
 * pen pressure: the structure the fourth phase persists. */

import { useEffect, useRef, type PointerEvent as ReactPointerEvent } from 'react'
import type { Camera } from '../ui/canvas/render/camera'

export interface InkPoint { x: number; y: number; t: number; p?: number }
export interface InkStroke { id: number; surfaceId: string; color: string; width: number; points: InkPoint[] }
export type MarkTool = 'none' | 'pointer' | 'pen' | 'eraser'

const TRAIL_MS = 700
const ERASE_PX = 10

function drawStroke(ctx: CanvasRenderingContext2D, stroke: InkStroke, camera: Camera) {
  const pts = stroke.points
  if (pts.length === 0) return
  ctx.strokeStyle = stroke.color
  ctx.fillStyle = stroke.color
  ctx.lineCap = 'round'
  ctx.lineJoin = 'round'
  if (pts.length === 1) {
    const p = camera.toScreen(pts[0].x, pts[0].y)
    ctx.beginPath()
    ctx.arc(p.x, p.y, (stroke.width * camera.zoom) / 2, 0, Math.PI * 2)
    ctx.fill()
    return
  }
  for (let i = 1; i < pts.length; i++) {
    const a = camera.toScreen(pts[i - 1].x, pts[i - 1].y)
    const b = camera.toScreen(pts[i].x, pts[i].y)
    ctx.lineWidth = Math.max(1, stroke.width * camera.zoom * (pts[i].p !== undefined ? 0.5 + (pts[i].p ?? 0) : 1))
    ctx.beginPath()
    ctx.moveTo(a.x, a.y)
    ctx.lineTo(b.x, b.y)
    ctx.stroke()
  }
}

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

/** Ink of the Surface on stage; takes the pointer while the pen or the eraser is the tool. */
export function InkLayer({ camera, surfaceId, tool, color, width, strokes, onStrokes }: {
  camera: Camera | null
  surfaceId: string | null
  tool: MarkTool
  color: string
  width: number
  strokes: InkStroke[]
  onStrokes: (next: InkStroke[]) => void
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const drawing = useRef<InkStroke | null>(null)
  const started = useRef(0)
  const latest = useRef(strokes)
  useEffect(() => { latest.current = strokes })

  useEffect(() => {
    const paint = () => {
      const canvas = canvasRef.current
      if (!canvas || !camera) return
      const ctx = fit(canvas)
      if (!ctx) return
      for (const stroke of latest.current) if (stroke.surfaceId === surfaceId) drawStroke(ctx, stroke, camera)
      if (drawing.current) drawStroke(ctx, drawing.current, camera)
    }
    paint()
    const off = camera?.onChange(paint)
    const observer = new ResizeObserver(paint)
    if (canvasRef.current) observer.observe(canvasRef.current)
    return () => { off?.(); observer.disconnect() }
  }, [camera, surfaceId, strokes])

  const world = (event: ReactPointerEvent) => {
    const r = canvasRef.current!.getBoundingClientRect()
    return camera!.toWorld(event.clientX - r.left, event.clientY - r.top)
  }
  const repaint = () => {
    const canvas = canvasRef.current
    if (!canvas || !camera) return
    const ctx = fit(canvas)
    if (!ctx) return
    for (const stroke of latest.current) if (stroke.surfaceId === surfaceId) drawStroke(ctx, stroke, camera)
    if (drawing.current) drawStroke(ctx, drawing.current, camera)
  }
  const erase = (event: ReactPointerEvent) => {
    if (!camera) return
    const p = world(event)
    const r = ERASE_PX / camera.zoom
    const kept = latest.current.filter((s) => s.surfaceId !== surfaceId || !s.points.some((q) => Math.hypot(q.x - p.x, q.y - p.y) <= r + s.width / 2))
    if (kept.length !== latest.current.length) onStrokes(kept)
  }
  const active = (tool === 'pen' || tool === 'eraser') && camera !== null && surfaceId !== null
  return (
    <canvas
      ref={canvasRef}
      className={`aiws-stage-ink aiws-stage-ui-pass${active ? ' is-active' : ''}`}
      data-testid="aiws-stage-ink"
      data-strokes={strokes.filter((s) => s.surfaceId === surfaceId).length}
      onPointerDown={(event) => {
        if (!active || event.button !== 0) return
        event.preventDefault()
        event.currentTarget.setPointerCapture(event.pointerId)
        if (tool === 'eraser') { erase(event); return }
        started.current = performance.now()
        const p = world(event)
        drawing.current = { id: Date.now(), surfaceId: surfaceId!, color, width, points: [{ x: p.x, y: p.y, t: 0, ...(event.pointerType === 'pen' ? { p: event.pressure } : {}) }] }
        repaint()
      }}
      onPointerMove={(event) => {
        if (!active || !(event.buttons & 1)) return
        if (tool === 'eraser') { erase(event); return }
        const stroke = drawing.current
        if (!stroke) return
        const p = world(event)
        stroke.points.push({ x: p.x, y: p.y, t: Math.round(performance.now() - started.current), ...(event.pointerType === 'pen' ? { p: event.pressure } : {}) })
        repaint()
      }}
      onPointerUp={() => {
        const stroke = drawing.current
        drawing.current = null
        if (stroke) onStrokes([...latest.current, stroke])
      }}
    />
  )
}

/** The laser pointer: a dot with a fading trail; `onMove` reports it in screen px of this layer (null when it leaves). */
export function PointerLayer({ active, onMove }: { active: boolean; onMove: (p: { x: number; y: number } | null) => void }) {
  const canvasRef = useRef<HTMLCanvasElement>(null)
  const trail = useRef<{ x: number; y: number; at: number }[]>([])
  const raf = useRef(0)
  useEffect(() => {
    if (!active) return
    const paint = () => {
      const canvas = canvasRef.current
      if (!canvas) return
      const ctx = fit(canvas)
      const now = performance.now()
      trail.current = trail.current.filter((p) => now - p.at < TRAIL_MS)
      if (ctx) {
        const pts = trail.current
        for (let i = 1; i < pts.length; i++) {
          const fade = 1 - (now - pts[i].at) / TRAIL_MS
          ctx.strokeStyle = `rgba(239, 68, 68, ${0.55 * fade})`
          ctx.lineWidth = 6 * fade + 1
          ctx.lineCap = 'round'
          ctx.beginPath()
          ctx.moveTo(pts[i - 1].x, pts[i - 1].y)
          ctx.lineTo(pts[i].x, pts[i].y)
          ctx.stroke()
        }
        const head = pts.at(-1)
        if (head && now - head.at < TRAIL_MS) {
          ctx.fillStyle = 'rgba(239, 68, 68, 0.95)'
          ctx.shadowColor = 'rgba(239, 68, 68, 0.8)'
          ctx.shadowBlur = 12
          ctx.beginPath()
          ctx.arc(head.x, head.y, 7, 0, Math.PI * 2)
          ctx.fill()
          ctx.shadowBlur = 0
        }
      }
      raf.current = requestAnimationFrame(paint)
    }
    raf.current = requestAnimationFrame(paint)
    return () => cancelAnimationFrame(raf.current)
  }, [active])
  return (
    <canvas
      ref={canvasRef}
      className={`aiws-stage-pointer aiws-stage-ui-pass${active ? ' is-active' : ''}`}
      data-testid="aiws-stage-pointer"
      onPointerMove={(event) => {
        if (!active) return
        const r = event.currentTarget.getBoundingClientRect()
        const p = { x: event.clientX - r.left, y: event.clientY - r.top }
        trail.current.push({ ...p, at: performance.now() })
        onMove(p)
      }}
      onPointerLeave={() => { if (active) onMove(null) }}
    />
  )
}
