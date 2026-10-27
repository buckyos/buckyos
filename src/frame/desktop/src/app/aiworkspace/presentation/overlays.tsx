/* The presenter's marks on stage (第三期规划 §10.3): a laser pointer whose trail fades, and ink — pen, eraser, undo,
 * clear. Both live in this show's memory only; nothing is written to the document. Ink strokes are kept in world
 * coordinates of their Surface (so they scale with the canvas and stay when the page turns) with time offsets and the
 * pen pressure. Sampling, the owning pointer, cancelling and drawing are the canvas pen's (ink/, 手写笔 TODO D2):
 * only what a finished stroke becomes differs — a board mark here, a Block there. */

import { useEffect, useLayoutEffect, useRef, useState } from 'react'
import { randomId } from '../api/ids'
import { InkInput, type InkHost } from '../ink/input'
import { settleChannels, simplify, SIMPLIFY_PX, touches, widePoints, type InkPoint, type InkStyle } from '../ink/model'
import { InkCanvas } from '../ink/paint'
import type { Camera } from '../ui/canvas/render/camera'

export interface InkStroke { id: string; surfaceId: string; color: string; width: number; points: InkPoint[] }
export type MarkTool = 'none' | 'pointer' | 'pen' | 'eraser'

const TRAIL_MS = 700

const styleOf = (stroke: { color: string; width: number }): InkStyle => ({ color: stroke.color, width: stroke.width, pressure: true })

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
  const tailRef = useRef<HTMLCanvasElement>(null)
  const [painter] = useState(() => new InkCanvas({ toScreen: (x, y) => ({ x, y }), zoom: () => 1 }))
  const [host] = useState<InkHost>(() => ({
    toWorld: (x, y) => ({ x, y }), zoom: () => 1, tool: () => 'pen', style: () => ({ color, width, pressure: true }), capture: () => undefined, release: () => undefined, interrupt: () => undefined,
    begin: () => undefined, draw: () => undefined, stroke: () => undefined, dropped: () => undefined, erase: () => undefined, eraseEnd: () => undefined,
  }))
  const [input] = useState(() => new InkInput(host, { fingers: true }))
  const active = (tool === 'pen' || tool === 'eraser') && camera !== null && surfaceId !== null

  useLayoutEffect(() => {
    if (camera) painter.setView({ toScreen: (x, y) => camera.toScreen(x, y), zoom: () => camera.zoom })
  }, [camera, painter])
  useLayoutEffect(() => {
    let style: InkStyle = { color, width, pressure: true }
    Object.assign(host, {
      toWorld: (clientX: number, clientY: number) => {
        const r = canvasRef.current?.getBoundingClientRect()
        return camera ? camera.toWorld(clientX - (r?.left ?? 0), clientY - (r?.top ?? 0)) : { x: clientX, y: clientY }
      },
      zoom: () => camera?.zoom ?? 1,
      tool: () => (tool === 'eraser' ? 'eraser' : 'pen'),
      style: () => ({ color, width, pressure: true }),
      capture: (id: number) => { try { canvasRef.current?.setPointerCapture(id) } catch { /* gone */ } },
      release: (id: number) => { try { if (canvasRef.current?.hasPointerCapture(id)) canvasRef.current.releasePointerCapture(id) } catch { /* released */ } },
      interrupt: () => undefined,
      begin: (next: InkStyle) => { style = next; painter.begin(next.color) },
      draw: (points, predicted) => painter.add(widePoints(points, style), widePoints(predicted, style)),
      stroke: (points, used, zoom) => {
        if (surfaceId) onStrokes([...strokes, { id: randomId('k'), surfaceId, color: used.color, width: used.width, points: simplify(settleChannels(points), SIMPLIFY_PX / zoom, used) }])
        painter.end()
      },
      dropped: () => painter.end(),
      // the board's eraser takes a stroke the moment it touches its line
      erase: (a, b, r) => {
        const kept = strokes.filter((s) => s.surfaceId !== surfaceId || !touches(widePoints(s.points, styleOf(s)), a, b, r))
        if (kept.length !== strokes.length) onStrokes(kept)
      },
      eraseEnd: () => undefined,
    } satisfies InkHost)
  })
  useEffect(() => {
    painter.attach(canvasRef.current, tailRef.current)
    return () => painter.attach(null, null)
  }, [painter])
  useEffect(() => {
    painter.setWaiting(strokes.filter((s) => s.surfaceId === surfaceId).map((s) => ({ points: widePoints(s.points, styleOf(s)), color: s.color })))
  }, [painter, strokes, surfaceId])
  useEffect(() => {
    const off = camera?.onChange(() => painter.redraw())
    const observer = new ResizeObserver(() => painter.redraw())
    if (canvasRef.current) observer.observe(canvasRef.current)
    painter.redraw()
    return () => { off?.(); observer.disconnect() }
  }, [camera, painter])
  // another tool or another page drops the stroke under way; the strokes drawn stay
  useEffect(() => () => input.cancel(), [input, tool, surfaceId])

  return (
    <>
      <canvas
        ref={canvasRef}
        className={`aiws-stage-ink aiws-stage-ui-pass${active ? ' is-active' : ''}`}
        data-testid="aiws-stage-ink"
        data-strokes={strokes.filter((s) => s.surfaceId === surfaceId).length}
        onPointerDown={(event) => {
          if (!active) return
          if (input.down(event.nativeEvent)) event.preventDefault()
        }}
        onPointerMove={(event) => { if (input.owns(event.pointerId)) input.move(event.nativeEvent) }}
        onPointerUp={(event) => { if (input.owns(event.pointerId)) input.up(event.nativeEvent) }}
        onPointerCancel={(event) => input.cancelPointer(event.pointerId)}
        onLostPointerCapture={(event) => input.lost(event.pointerId)}
        onContextMenu={(event) => { if (active) event.preventDefault() }}
      />
      <canvas ref={tailRef} className="aiws-stage-ink is-tail" aria-hidden="true" />
    </>
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
