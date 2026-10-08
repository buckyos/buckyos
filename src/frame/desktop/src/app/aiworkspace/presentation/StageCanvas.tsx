/* The stage's picture (第三期规划 §4.2, §5.3, §9): the Surfaces of the current and the next step rendered through the
 * canvas RenderHost in `show` mode (at most two mounted, §9.4), a camera per Surface driven here, and a mask that
 * letterboxes the stage and crops a Frame step to its Frame. Transitions are written here and nowhere else:
 *
 *   cut    the camera jumps
 *   fade   a veil of the background covers the stage (150 ms), the camera (and Surface) switches while it is covered,
 *          the target's Blocks get up to 300 ms to be ready, the veil lifts (150 ms)
 *   fly    van Wijk–Nuij smooth zoom on the same Surface; the crop fades out over the first 150 ms and in over the last
 *
 * A new navigation during a transition wins: a flight starts again from where the camera is now, a fade completes at
 * once and the new one starts (a token voids the old callbacks). The camera moves through CSS transforms only. A camera
 * change that did not come from here is the presenter browsing by hand (`onFree`). */

import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { testHooks } from '../api/testHooks'
import type { StageSize } from '../api/types'
import type { Laid } from '../ui/canvas/layout'
import { Camera } from '../ui/canvas/render/camera'
import { RenderHost } from '../ui/canvas/render/RenderHost'
import { useSurfaceLayout } from '../ui/canvas/useSurfaceLayout'
import type { Transition } from './controller'
import { DEFAULT_BACKGROUND, eyeOf, FADE_MS, fitStage, flyPath, stageView, viewOfEye, type Rect, type ResolvedStep, type View } from './model'

const READY_MAX_MS = 300

export interface StageCanvasProps {
  stage: StageSize
  /** The path background: letterbox and the background of Viewport steps. */
  background?: string
  current: ResolvedStep | null
  /** Its Blocks are mounted ahead (§9.4: pre-read the next step, never run its HTML). */
  next: ResolvedStep | null
  /** Background of the current Frame step (`presentation.background`). */
  pageBackground?: string | null
  transition: Transition
  /** Every navigation has its own number: a new number animates once. */
  nonce: number
  free: boolean
  black: boolean
  onFree?: () => void
  editing?: string | null
  onEditingChange?: (id: string | null) => void
  /** The camera of the Surface on stage (for world-space overlays) and that Surface. */
  onCamera?: (camera: Camera, surfaceId: string) => void
  /** Drawn above the Blocks and below the mask (ink), and above everything (the laser pointer). */
  underMask?: ReactNode
  overMask?: ReactNode
  /** The prompter's background picture: cuts only, no gestures, no editing. */
  passive?: boolean
}

function frameIds(laid: Map<string, Laid>): string[] {
  const out: string[] = []
  for (const [id, l] of laid) if (l.entity.view_type === 'frame') out.push(id)
  return out
}

function SurfaceLayer({ surfaceId, camera, active, focus, hide, editing, onEditingChange, onMoved, passive }: {
  surfaceId: string
  camera: Camera
  active: boolean
  focus: Rect[]
  hide: string[]
  editing: string | null
  onEditingChange: (id: string | null) => void
  /** Every change of this layer's camera (the stage tells its own from the presenter's). */
  onMoved: (surfaceId: string, camera: Camera) => void
  passive: boolean
}) {
  const { laid, index } = useSurfaceLayout(surfaceId)
  useEffect(() => camera.onChange(() => onMoved(surfaceId, camera)), [camera, surfaceId, onMoved])
  const focusKey = focus.map((r) => `${Math.round(r.x)},${Math.round(r.y)},${Math.round(r.w)},${Math.round(r.h)}`).join(';')
  // the target's and the next step's Blocks are mounted at full detail before the camera gets there
  const prefetch = useMemo(() => {
    const ids = new Set<string>()
    for (const part of focusKey ? focusKey.split(';') : []) {
      const [x, y, w, h] = part.split(',').map(Number)
      for (const item of index.query({ x, y, w, h })) ids.add(item.id)
    }
    return ids
  }, [index, focusKey])
  const hideKey = hide.join(',')
  // Frames are editing aids: their border and title never show on stage (§4.2)
  const hidden = useMemo(() => new Set([...frameIds(laid), ...(hideKey ? hideKey.split(',') : [])]), [laid, hideKey])
  const [noSelection] = useState(() => new Set<string>())
  return (
    <div className={`aiws-stage-layer${active ? ' is-active' : ''}`} data-testid={`aiws-stage-layer-${surfaceId}`} data-active={active ? 'true' : undefined} aria-hidden={!active}>
      <RenderHost surfaceId={surfaceId} mode="show" camera={camera} laid={laid} index={index} selection={noSelection} onSelectionChange={() => undefined}
        editing={active && !passive ? editing : null} onEditingChange={(id) => { if (!passive) onEditingChange(id) }} canLayout={false} onCommitLayout={() => undefined}
        onContextMenu={() => undefined} onOpenEntity={() => undefined} gesturesPaused={passive} prefetch={prefetch} hidden={hidden} />
    </div>
  )
}

interface Mask { hole: Rect | null; opacity: number }

function hooksFor(passive: boolean) {
  const hooks = passive ? null : testHooks()
  if (!hooks) return null
  hooks.stage ??= { view: () => ({ x: 0, y: 0, zoom: 1, surfaceId: null }), transitions: [] }
  return hooks.stage
}

function same(a: View, b: View): boolean {
  return Math.abs(a.x - b.x) < 1e-6 && Math.abs(a.y - b.y) < 1e-6 && Math.abs(a.zoom - b.zoom) < 1e-9
}

export function StageCanvas(props: StageCanvasProps) {
  const { stage, current, next, transition, nonce, free, black, passive = false } = props
  const rootRef = useRef<HTMLDivElement>(null)
  const veilRef = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState({ w: 0, h: 0 })
  const [displayed, setDisplayed] = useState<string | null>(null)
  const [mask, setMask] = useState<Mask>({ hole: null, opacity: 1 })
  // what callbacks need of the latest render
  const live = useRef({ current, free, onFree: props.onFree, onCamera: props.onCamera, stage, displayed: null as string | null })
  useLayoutEffect(() => { Object.assign(live.current, { current, free, onFree: props.onFree, onCamera: props.onCamera, stage }) })
  const token = useRef(0)
  const timer = useRef(0)
  const raf = useRef(0)
  const running = useRef<(() => void) | null>(null)
  const driving = useRef(false)
  const lastViews = useRef(new Map<string, View>())
  const [cameras] = useState(() => new Map<string, Camera>())
  const cameraFor = (surfaceId: string): Camera => {
    let camera = cameras.get(surfaceId)
    if (!camera) { camera = new Camera(); cameras.set(surfaceId, camera) }
    return camera
  }
  // a change this component did not make is the presenter browsing by hand
  const onMoved = useCallback((surfaceId: string, camera: Camera) => {
    const view = camera.viewport
    const last = lastViews.current.get(surfaceId)
    if (driving.current || (last && same(view, last))) return
    lastViews.current.set(surfaceId, view)
    if (!passive && last) live.current.onFree?.()
  }, [passive])

  const drive = (surfaceId: string, view: View) => {
    const camera = cameraFor(surfaceId)
    driving.current = true
    try { camera.set(view) } finally { driving.current = false }
    lastViews.current.set(surfaceId, camera.viewport)
    return camera
  }

  const stageRect = (): Rect => fitStage({ x: 0, y: 0, w: rootRef.current?.clientWidth ?? 800, h: rootRef.current?.clientHeight ?? 600 }, live.current.stage)

  /** Put `step` on stage at once. */
  const jump = (step: ResolvedStep) => {
    const { view, hole } = stageView(step.rect!, stageRect())
    const camera = drive(step.surfaceId!, view)
    live.current.displayed = step.surfaceId
    setDisplayed(step.surfaceId)
    setMask({ hole, opacity: 1 })
    live.current.onCamera?.(camera, step.surfaceId!)
  }

  const setVeil = (opacity: number, ms: number) => {
    const veil = veilRef.current
    if (!veil) return
    veil.style.transition = ms ? `opacity ${ms}ms ease` : 'none'
    veil.style.opacity = String(opacity)
  }

  /** Wait until the Blocks on stage stopped loading (at most READY_MAX_MS). */
  const whenReady = (surfaceId: string, run: number) => new Promise<void>((resolve) => {
    const start = performance.now()
    const check = () => {
      if (token.current !== run) { resolve(); return }
      const layer = rootRef.current?.querySelector(`[data-testid="aiws-stage-layer-${CSS.escape(surfaceId)}"]`)
      if (!layer?.querySelector('[data-testid^="aiws-block-loading-"]') || performance.now() - start > READY_MAX_MS) resolve()
      else requestAnimationFrame(check)
    }
    requestAnimationFrame(check)
  })

  // ---- one navigation, one transition
  useLayoutEffect(() => {
    const step = current
    if (!step?.rect || !step.surfaceId) return
    // a transition still running completes at once; its callbacks are void from here on
    running.current?.()
    running.current = null
    window.clearTimeout(timer.current)
    cancelAnimationFrame(raf.current)
    const run = ++token.current
    const first = live.current.displayed === null
    const kind: Transition = first ? 'fade' : passive ? 'cut' : transition === 'fly' && live.current.displayed !== step.surfaceId ? 'fade' : transition
    // the e2e probe reads what ran: kind, frame times, when the target was on stage (§15 P3-04, P3-19)
    const record = hooksFor(passive)?.transitions
    const entry = record ? { kind, step: step.step.id, startedAt: performance.now(), arrivedAt: null as number | null, frames: [] as number[] } : null
    if (record && entry) { record.push(entry); if (record.length > 50) record.shift() }
    const arrived = () => { if (entry) entry.arrivedAt = performance.now() }
    if (kind === 'cut') {
      raf.current = requestAnimationFrame(() => { if (token.current === run) { setVeil(0, 0); jump(step); arrived() } })
      return
    }
    if (kind === 'fade') {
      setVeil(1, first ? 0 : FADE_MS)
      running.current = () => { setVeil(0, 0); jump(step) }
      timer.current = window.setTimeout(() => {
        if (token.current !== run) return
        jump(step)
        void whenReady(step.surfaceId!, run).then(() => {
          if (token.current !== run) return
          running.current = null
          arrived()
          setVeil(0, FADE_MS)
        })
      }, first ? 0 : FADE_MS)
      return
    }
    // fly: from where the camera is now (a flight interrupted, free browsing) to the step
    const s = stageRect()
    const { view, hole } = stageView(step.rect, s)
    const camera = cameraFor(step.surfaceId)
    const path = flyPath(eyeOf(camera.viewport, s), eyeOf(view, s))
    running.current = () => { drive(step.surfaceId!, view); setMask({ hole, opacity: 1 }) }
    setVeil(0, 0)
    const t0 = performance.now()
    let phase: 'start' | 'flying' | 'revealed' = 'start'
    const frame = (now: number) => {
      if (token.current !== run) return
      entry?.frames.push(now)
      if (phase === 'start') { phase = 'flying'; setMask((m) => ({ hole: m.hole, opacity: 0 })) }
      const t = Math.min(1, (now - t0) / path.duration)
      drive(step.surfaceId!, viewOfEye(path.at(t), s))
      // the target's crop and background come in over the last 150 ms
      if (phase === 'flying' && now - t0 >= path.duration - FADE_MS) { phase = 'revealed'; setMask({ hole, opacity: 1 }) }
      if (t < 1) { raf.current = requestAnimationFrame(frame); return }
      drive(step.surfaceId!, view)
      running.current = null
      arrived()
      live.current.onCamera?.(camera, step.surfaceId!)
    }
    raf.current = requestAnimationFrame(frame)
    // eslint-disable-next-line react-hooks/exhaustive-deps -- one transition per navigation number
  }, [nonce])

  // a new window size: the same content, re-fitted at once (§9.5)
  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    const observer = new ResizeObserver(() => {
      setSize({ w: el.clientWidth, h: el.clientHeight })
      const step = live.current.current
      if (step?.rect && !running.current && !live.current.free && live.current.displayed) jump(step)
    })
    observer.observe(el)
    return () => observer.disconnect()
    // eslint-disable-next-line react-hooks/exhaustive-deps -- reads the latest render through `live`
  }, [])

  useEffect(() => () => { token.current += 1; window.clearTimeout(timer.current); cancelAnimationFrame(raf.current) }, [])
  useEffect(() => {
    const hooks = hooksFor(passive)
    if (!hooks) return
    hooks.view = () => {
      const surfaceId = live.current.displayed
      const v = surfaceId ? cameras.get(surfaceId)?.viewport : undefined
      return { x: v?.x ?? 0, y: v?.y ?? 0, zoom: v?.zoom ?? 1, surfaceId }
    }
  }, [passive, cameras])

  // the Surfaces kept mounted: the one on stage, the current step's and the next step's (at most two)
  const layers: string[] = []
  for (const id of [displayed, current?.surfaceId ?? null, next?.surfaceId ?? null]) if (id && !layers.includes(id) && layers.length < 2) layers.push(id)
  const hole = mask.hole
  const background = props.background ?? DEFAULT_BACKGROUND
  const page = current?.kind === 'frame' ? props.pageBackground ?? background : background
  return (
    <div ref={rootRef} className={`aiws-stage-canvas${passive ? ' is-passive' : ''}`} data-testid="aiws-stage-canvas" data-step={current?.step.id} data-surface={displayed ?? undefined} data-free={free ? 'true' : undefined}
      style={{ ['--aiws-stage-bg' as string]: page, ['--aiws-stage-letterbox' as string]: background }}>
      {layers.map((surfaceId) => {
        const focus = [current, next].flatMap((s) => (s?.surfaceId === surfaceId && s.rect ? [s.rect] : []))
        return (
          <SurfaceLayer key={surfaceId} surfaceId={surfaceId} camera={cameraFor(surfaceId)} active={surfaceId === displayed} focus={focus}
            hide={current?.surfaceId === surfaceId ? current.step.hide ?? [] : []} editing={props.editing ?? null} onEditingChange={props.onEditingChange ?? (() => undefined)} onMoved={onMoved} passive={passive} />
        )
      })}
      {props.underMask}
      {hole && size.w > 0 && (
        <svg className="aiws-stage-mask" data-testid="aiws-stage-mask" data-hole={`${Math.round(hole.x)},${Math.round(hole.y)},${Math.round(hole.w)},${Math.round(hole.h)}`}
          width={size.w} height={size.h} style={{ opacity: free ? 0 : mask.opacity }}>
          <path fillRule="evenodd" d={`M0 0H${size.w}V${size.h}H0Z M${hole.x} ${hole.y}H${hole.x + hole.w}V${hole.y + hole.h}H${hole.x}Z`} />
        </svg>
      )}
      <div ref={veilRef} className="aiws-stage-veil" data-testid="aiws-stage-veil" />
      {black && <div className="aiws-stage-black" data-testid="aiws-stage-black" />}
      {props.overMask}
    </div>
  )
}
