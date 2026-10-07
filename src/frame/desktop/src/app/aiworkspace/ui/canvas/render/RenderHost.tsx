/* RenderHost (phase two §9): the free canvas's world layer, culling, LOD, overlay and gestures.
 *
 *   content layer   one absolutely positioned frame per mounted Block, inside a world layer whose single
 *                   CSS transform is the camera (no per-Block repositioning, no React render per frame)
 *   low detail      Blocks that are small on screen get a placeholder drawn here, not by the Renderer
 *   overlay         hover outline, selection boxes, handles, marquee — screen coordinates, SVG
 *   editor          the BlockHost mounts the Editor of at most the activated Block(s)
 *
 * Rules kept here: the camera is not React state; culling has three levels with hysteresis
 * (mounted / hidden / unmounted) and never culls selected, editing or hovered Blocks; pointermove
 * is coalesced per animation frame and moves DOM transforms, never state or the network; a gesture
 * ends in exactly one commit (or none, on Esc). Hit testing is geometric through the spatial index.
 * Touch has its own gestures (touch.ts, UI improvement §16): pan, pinch, taps, long press. */

import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent, type ReactNode, type WheelEvent as ReactWheelEvent } from 'react'
import type { Placement } from '../../../api/types'
import { testHooks } from '../../../api/testHooks'
import { useStore } from '../../../state/hooks'
import { BlockHost, type Lod } from '../../blocks/BlockHost'
import { BudgetContext } from '../../blocks/budget'
import { modePolicy, type CanvasMode } from '../../blocks/registry'
import { relativeTo, topLevel, type Laid } from '../layout'
import { Camera, intersects, type Rect } from './camera'
import { SpatialIndex } from './spatialIndex'
import { TouchGestures, type OneFingerDrag, type Point, type TouchActions } from './touch'

/** Blocks within this many viewports of the visible area stay mounted; within the next band they stay in the DOM but hidden. */
const MOUNT_MARGIN = 0.5
const HIDE_MARGIN = 2
/** At most this many content Blocks carry a mounted Renderer at once; the rest show placeholders. */
const MAX_MOUNTED = 400
const PLACEHOLDER_PX = 48
const SIMPLIFIED_ZOOM = 0.4
const HANDLE = 8
const MIN_SIZE = 40
/** Above this many moved frames a drag shows outlines only (one hide, one restore) instead of per-frame transforms. */
const GHOST_DRAG_LIMIT = 24

export interface LayoutChange { id: string; placement: Placement; parentId?: string }

export interface RenderHostProps {
  surfaceId: string
  mode: CanvasMode
  camera: Camera
  laid: Map<string, Laid>
  index: SpatialIndex
  selection: ReadonlySet<string>
  onSelectionChange: (next: Set<string>) => void
  editing: string | null
  onEditingChange: (id: string | null) => void
  canLayout: boolean
  onCommitLayout: (changes: LayoutChange[]) => void
  onContextMenu: (point: { screenX: number; screenY: number; worldX: number; worldY: number }, blockId: string | null) => void
  onOpenEntity: (entityId: string) => void
  /** Rendered in screen space above the selection (the near toolbar). */
  renderNear?: (bbox: Rect) => ReactNode
  /** Something outside the canvas wants the pointer (e.g. an open menu): gestures are refused. */
  gesturesPaused?: boolean
  /** The pointer tool (UI improvement §7.2): `hand` pans with the primary button and never selects. */
  tool?: 'select' | 'hand'
  /** A one-shot placement: the pointer carries a preview of this size; a primary click places it. */
  placing?: { w: number; h: number; label: string } | null
  onPlace?: (world: { x: number; y: number }) => void
}

type Drag =
  | { kind: 'pan'; lastX: number; lastY: number; moved: boolean }
  | { kind: 'move'; ids: string[]; affected: string[]; startX: number; startY: number; dx: number; dy: number; moved: boolean; clicked: string; additive: boolean; wasSelected: boolean; ghost?: boolean }
  | { kind: 'resize'; id: string; handle: string; startX: number; startY: number; start: Rect; current: Rect }
  | { kind: 'marquee'; startX: number; startY: number; current: Rect | null; additive: boolean; moved: boolean }

type MountState = 'mounted' | 'hidden'

/** What starting a mouse-path gesture needs: a React pointer event, or a native one handed over by a touch. */
type PointerStart = Pick<PointerEvent, 'pointerId' | 'button' | 'clientX' | 'clientY' | 'shiftKey' | 'metaKey' | 'ctrlKey' | 'target' | 'preventDefault'>

/** Controls inside a Block that take a tap themselves (their click must survive). */
const TAPPABLE = 'button, a[href], summary, label'
/** Touches here stay with the browser: editors, fields, the near toolbar and menus. */
const TOUCH_EXEMPT = '[data-role="editor"], input, textarea, select, [contenteditable="true"], .aiws-near, .aiws-menu'

/** The nearest element between `target` and `frame` whose own content scrolls. */
function scrollerWithin(target: Element | null, frame: HTMLElement | undefined): HTMLElement | null {
  for (let el = target; el && frame && el !== frame && frame.contains(el); el = el.parentElement) {
    if (!(el instanceof HTMLElement)) continue
    const style = getComputedStyle(el)
    if ((el.scrollHeight > el.clientHeight + 1 && /auto|scroll/.test(style.overflowY)) || (el.scrollWidth > el.clientWidth + 1 && /auto|scroll/.test(style.overflowX))) return el
  }
  return null
}

/** Three culling levels with hysteresis (§9.3 rule 3): mounted near the viewport, hidden in the DOM further out, unmounted beyond. */
function computeMounts(prev: Map<string, MountState>, camera: Camera, laid: Map<string, Laid>, index: SpatialIndex, pinned: Set<string>): Map<string, MountState> {
  const near = camera.visibleWorld(MOUNT_MARGIN)
  const far = camera.visibleWorld(HIDE_MARGIN)
  const center = { x: near.x + near.w / 2, y: near.y + near.h / 2 }
  const next = new Map<string, MountState>()
  const candidates: { id: string; dist: number }[] = []
  for (const item of index.query(far)) {
    if (intersects(item.rect, near)) candidates.push({ id: item.id, dist: Math.hypot(item.rect.x + item.rect.w / 2 - center.x, item.rect.y + item.rect.h / 2 - center.y) })
    else next.set(item.id, 'hidden')
  }
  // previously mounted Blocks that are now only in the hidden band stay in the DOM (hysteresis)
  for (const [id, state] of prev) if (state === 'mounted' && !next.has(id) && !candidates.some((c) => c.id === id)) { const l = laid.get(id); if (l && intersects(l.rect, far)) next.set(id, 'hidden') }
  candidates.sort((a, b) => a.dist - b.dist)
  candidates.forEach((c, i) => next.set(c.id, i < MAX_MOUNTED ? 'mounted' : 'hidden'))
  for (const id of pinned) if (laid.has(id)) next.set(id, 'mounted')
  let same = next.size === prev.size
  if (same) for (const [id, state] of next) if (prev.get(id) !== state) { same = false; break }
  return same ? prev : next
}

function lodFor(rect: Rect, zoom: number, pinned: boolean): Lod {
  if (pinned) return 'full'
  const px = Math.min(rect.w, rect.h) * zoom
  if (px < PLACEHOLDER_PX) return 'placeholder'
  if (zoom < SIMPLIFIED_ZOOM) return 'simplified'
  return 'full'
}

export function RenderHost(props: RenderHostProps) {
  const { camera, laid, index, selection, mode, editing, canLayout } = props
  const store = useStore()
  const budget = useContext(BudgetContext)
  const rootRef = useRef<HTMLDivElement>(null)
  const worldRef = useRef<HTMLDivElement>(null)
  const frames = useRef(new Map<string, HTMLDivElement>())
  const dragRef = useRef<Drag | null>(null)
  const rafRef = useRef<number>(0)
  const pendingMove = useRef<{ x: number; y: number } | null>(null)
  const spaceHeld = useRef(false)
  const policy = modePolicy(mode)
  const [marquee, setMarquee] = useState<Rect | null>(null)
  const [dragging, setDragging] = useState(false)
  const [hover, setHover] = useState<string | null>(null)
  const [resizePreview, setResizePreview] = useState<{ id: string; rect: Rect } | null>(null)
  const [, forceOverlay] = useState(0)
  const [settled, setSettled] = useState(0)
  const pointerTypeRef = useRef('mouse')
  const [touch] = useState(() => new TouchGestures(camera))

  // ---- camera attach and overlay subscriptions
  useLayoutEffect(() => {
    camera.attach(worldRef.current)
    const root = rootRef.current
    if (root) camera.setViewportSize(root.clientWidth, root.clientHeight)
    const observer = root ? new ResizeObserver(() => { camera.setViewportSize(root.clientWidth, root.clientHeight); setSettled((n) => n + 1) }) : null
    if (root && observer) observer.observe(root)
    const offChange = camera.onChange(() => forceOverlay((n) => n + 1))
    return () => { offChange(); observer?.disconnect(); camera.attach(null) }
  }, [camera])

  // ---- culling with hysteresis (recomputed when the camera settles or the layout changes)
  const pinned = useMemo(() => {
    const set = new Set<string>(selection)
    if (editing) set.add(editing)
    if (hover) set.add(hover)
    return set
  }, [selection, editing, hover])
  const [mounts, setMounts] = useState<Map<string, MountState>>(() => computeMounts(new Map(), camera, laid, index, pinned))
  // the camera settling is an event: recompute from the listener (never inside an effect)
  useEffect(() => camera.onSettle(() => { setSettled((n) => n + 1); setMounts((prev) => computeMounts(prev, camera, laid, index, pinned)) }), [camera, laid, index, pinned])
  // a new layout or a new pinned set recomputes during render (derived state)
  const [inputsSeen, setInputsSeen] = useState({ laid, index, pinned })
  if (inputsSeen.laid !== laid || inputsSeen.index !== index || inputsSeen.pinned !== pinned) {
    setInputsSeen({ laid, index, pinned })
    setMounts((prev) => computeMounts(prev, camera, laid, index, pinned))
  }

  // ---- stats for the render probe (dev override only)
  useEffect(() => {
    const hooks = testHooks()
    if (!hooks) return
    let mounted = 0, hidden = 0, placeholders = 0
    const zoom = camera.settledZoom
    for (const [id, state] of mounts) {
      if (state === 'hidden') { hidden += 1; continue }
      const l = laid.get(id)
      if (l && !l.isGroup && lodFor(l.rect, zoom, pinned.has(id)) === 'placeholder') placeholders += 1
      else mounted += 1
    }
    hooks.canvas = { surfaceId: props.surfaceId, blocks: laid.size, mounted, hidden, placeholders, editors: budget.used('editors'), html: budget.used('html'), zoom, mode }
  })

  // ---- helpers
  const screenPoint = (event: { clientX: number; clientY: number }) => {
    const r = rootRef.current?.getBoundingClientRect()
    return { x: event.clientX - (r?.left ?? 0), y: event.clientY - (r?.top ?? 0) }
  }
  const descendantsOf = useCallback((id: string): string[] => {
    const out: string[] = []
    for (const [cid, l] of laid) { let cur: string | undefined = l.parentId; while (cur) { if (cur === id) { out.push(cid); break } cur = laid.get(cur)?.parentId } }
    return out
  }, [laid])
  const setTransform = (ids: Iterable<string>, dx: number, dy: number) => {
    for (const id of ids) { const el = frames.current.get(id); if (el) el.style.transform = dx === 0 && dy === 0 ? '' : `translate(${dx}px, ${dy}px)` }
  }
  /** Leaving an editor from the canvas ends its input first (its blur saves), before the editor unmounts. */
  const endEditing = () => {
    const active = document.activeElement
    if (active instanceof HTMLElement && rootRef.current?.contains(active)) active.blur()
    props.onEditingChange(null)
  }
  const insideEditor = (target: EventTarget | null) => {
    const el = target as HTMLElement | null
    return Boolean(el?.closest?.('[data-role="editor"], input, textarea, select, [contenteditable="true"], .aiws-near, .aiws-menu, button'))
  }

  const flushMoveRef = useRef<(() => void) | null>(null)
  const selectionLayerRef = useRef<SVGGElement>(null)
  // per-Block props that must keep their identity so a memoised BlockHost skips pans (§9.3)
  const { onEditingChange } = props
  const [blockHandlers] = useState(() => new Map<string, { key: typeof onEditingChange; onActivate: () => void; onDeactivate: () => void }>())
  const handlersFor = (id: string) => {
    let h = blockHandlers.get(id)
    if (!h || h.key !== onEditingChange) { h = { key: onEditingChange, onActivate: () => onEditingChange(id), onDeactivate: () => onEditingChange(null) }; blockHandlers.set(id, h) }
    return h
  }
  const [blockSizes] = useState(() => new Map<string, { w: number; h: number }>())
  const sizeFor = (id: string, w: number, h: number) => {
    const s = blockSizes.get(id)
    if (s && s.w === w && s.h === h) return s
    const next = { w, h }
    blockSizes.set(id, next)
    return next
  }
  const finishDrag = useCallback((cancel: boolean) => {
    // a move that is still waiting for its frame counts: a fast gesture must not end as a click
    if (rafRef.current) { cancelAnimationFrame(rafRef.current); rafRef.current = 0; flushMoveRef.current?.() }
    const drag = dragRef.current
    dragRef.current = null
    pendingMove.current = null
    setDragging(false)
    worldRef.current?.classList.remove('is-moving')
    if (!drag) return
    if (drag.kind === 'move') {
      if (drag.ghost) { for (const gid of drag.affected) { const el = frames.current.get(gid); if (el) el.style.visibility = '' } } else setTransform(drag.affected, 0, 0)
      selectionLayerRef.current?.removeAttribute('transform')
      if (!cancel && drag.moved) {
        const changes: LayoutChange[] = []
        for (const id of drag.ids) {
          const l = laid.get(id)
          if (!l) continue
          changes.push({ id, placement: relativeTo(laid, l.parentId, { ...l.rect, x: l.rect.x + drag.dx, y: l.rect.y + drag.dy }) })
        }
        if (changes.length > 0) props.onCommitLayout(changes)
      } else if (!drag.moved && !cancel) {
        // a click: select (shift toggles)
        const next = new Set(drag.additive ? selection : [])
        if (drag.additive && drag.wasSelected) next.delete(drag.clicked)
        else next.add(drag.clicked)
        props.onSelectionChange(next)
      }
    } else if (drag.kind === 'resize') {
      setResizePreview(null)
      const el = frames.current.get(drag.id)
      const l = laid.get(drag.id)
      if (el && l) { el.style.left = `${l.rect.x}px`; el.style.top = `${l.rect.y}px`; el.style.width = `${l.rect.w}px`; el.style.height = `${l.rect.h}px` }
      if (!cancel && l && (drag.current.w !== drag.start.w || drag.current.h !== drag.start.h || drag.current.x !== drag.start.x || drag.current.y !== drag.start.y)) {
        props.onCommitLayout([{ id: drag.id, placement: relativeTo(laid, l.parentId, drag.current) }])
      }
    } else if (drag.kind === 'marquee') {
      setMarquee(null)
      if (!cancel && drag.current && drag.moved) {
        const world = { ...camera.toWorld(drag.current.x, drag.current.y), w: drag.current.w / camera.zoom, h: drag.current.h / camera.zoom }
        const hit = index.within(world).map((item) => item.id)
        const next = new Set(drag.additive ? selection : [])
        for (const id of hit) next.add(id)
        props.onSelectionChange(next)
      } else if (!cancel && !drag.moved && !drag.additive) {
        props.onSelectionChange(new Set())
      }
    }
  }, [laid, selection, props, camera, index])

  // ---- pointer gestures
  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    pointerTypeRef.current = event.pointerType
    if (event.pointerType === 'touch') { touch.down(event.nativeEvent); return }
    if (props.gesturesPaused || dragRef.current) return
    if (insideEditor(event.target)) return
    beginPointer(event)
  }

  /** The mouse path: place, pan, move, or marquee. */
  const beginPointer = (event: PointerStart) => {
    const p = screenPoint(event)
    const w = camera.toWorld(p.x, p.y)
    if (props.placing && event.button === 0 && !spaceHeld.current) {
      event.preventDefault()
      props.onPlace?.(w)
      return
    }
    const hand = props.tool === 'hand'
    const hit = policy.select && !hand ? index.hit(w.x, w.y) : null
    const pan = event.button === 1 || event.button === 2 || spaceHeld.current || hand || (!policy.select && event.button === 0) || (mode === 'view' && !hit && event.button === 0)
    if (event.button === 2 && !spaceHeld.current) return // context menu
    rootRef.current?.setPointerCapture(event.pointerId)
    window.getSelection()?.removeAllRanges()
    setDragging(true)
    if (pan) { dragRef.current = { kind: 'pan', lastX: event.clientX, lastY: event.clientY, moved: false }; worldRef.current?.classList.add('is-moving'); return }
    if (event.button !== 0) { setDragging(false); return }
    if (hit) {
      const id = hit.id
      const wasSelected = selection.has(id)
      const additive = event.shiftKey || event.metaKey || event.ctrlKey
      const ids = policy.layout && canLayout ? topLevel(laid, wasSelected && !additive ? new Set(selection) : new Set([id])) : []
      const affected = ids.flatMap((moved) => [moved, ...descendantsOf(moved)])
      if (!wasSelected && !additive) props.onSelectionChange(new Set([id]))
      dragRef.current = { kind: 'move', ids, affected, startX: event.clientX, startY: event.clientY, dx: 0, dy: 0, moved: false, clicked: id, additive, wasSelected }
      if (editing && editing !== id) endEditing()
      return
    }
    if (editing) endEditing()
    dragRef.current = { kind: 'marquee', startX: p.x, startY: p.y, current: null, additive: event.shiftKey, moved: false }
  }

  const ghostRef = useRef<HTMLDivElement>(null)
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === 'touch') return // window listeners follow touches
    const drag = dragRef.current
    if (!drag && props.placing) {
      // the placement preview follows the pointer in the DOM: no React work per move
      const ghost = ghostRef.current
      if (ghost) {
        const p = screenPoint(event)
        ghost.style.transform = `translate(${p.x}px, ${p.y}px)`
        ghost.style.width = `${props.placing.w * camera.zoom}px`
        ghost.style.height = `${props.placing.h * camera.zoom}px`
        ghost.style.visibility = 'visible'
      }
      return
    }
    if (!drag) {
      if (!policy.select || props.tool === 'hand') return
      const p = screenPoint(event)
      const w = camera.toWorld(p.x, p.y)
      const hit = insideEditor(event.target) ? null : index.hit(w.x, w.y)
      const id = hit?.id ?? null
      if (id !== hover) setHover(id)
      return
    }
    trackPointer(event.clientX, event.clientY)
  }

  /** The running mouse-path gesture follows this position at the next frame. */
  const trackPointer = (clientX: number, clientY: number) => {
    pendingMove.current = { x: clientX, y: clientY }
    if (rafRef.current) return
    rafRef.current = requestAnimationFrame(flushMove)
  }

  /** Apply the last pointer position to the running gesture (once per frame; also before a gesture ends). */
  const flushMove = () => {
      rafRef.current = 0
      const current = dragRef.current
      const point = pendingMove.current
      if (!current || !point) return
      if (current.kind === 'pan') {
        camera.panBy(point.x - current.lastX, point.y - current.lastY)
        current.lastX = point.x; current.lastY = point.y; current.moved = true
        return
      }
      if (current.kind === 'move') {
        const dx = (point.x - current.startX) / camera.zoom
        const dy = (point.y - current.startY) / camera.zoom
        if (!current.moved && Math.hypot(point.x - current.startX, point.y - current.startY) < 3) return
        if (current.ids.length === 0) return
        current.moved = true
        current.dx = dx; current.dy = dy
        worldRef.current?.classList.add('is-moving')
        // a large selection moves as a ghost: the frames are hidden once and only the outlines follow (§9.3)
        if (current.affected.length > GHOST_DRAG_LIMIT) {
          if (!current.ghost) { current.ghost = true; for (const gid of current.affected) { const el = frames.current.get(gid); if (el) el.style.visibility = 'hidden' } }
        } else setTransform(current.affected, dx, dy)
        // the selection outlines follow in the DOM: no React work per frame (§9.3)
        selectionLayerRef.current?.setAttribute('transform', `translate(${dx * camera.zoom}, ${dy * camera.zoom})`)
        return
      }
      if (current.kind === 'resize') {
        const dx = (point.x - current.startX) / camera.zoom
        const dy = (point.y - current.startY) / camera.zoom
        const s = current.start
        let { x, y, w, h } = s
        if (current.handle.includes('e')) w = Math.max(MIN_SIZE, s.w + dx)
        if (current.handle.includes('s')) h = Math.max(MIN_SIZE, s.h + dy)
        if (current.handle.includes('w')) { w = Math.max(MIN_SIZE, s.w - dx); x = s.x + s.w - w }
        if (current.handle.includes('n')) { h = Math.max(MIN_SIZE, s.h - dy); y = s.y + s.h - h }
        current.current = { x, y, w, h }
        const el = frames.current.get(current.id)
        if (el) { el.style.left = `${x}px`; el.style.top = `${y}px`; el.style.width = `${w}px`; el.style.height = `${h}px` }
        setResizePreview({ id: current.id, rect: current.current })
        return
      }
      if (current.kind === 'marquee') {
        const p = screenPoint({ clientX: point.x, clientY: point.y })
        const rect = { x: Math.min(current.startX, p.x), y: Math.min(current.startY, p.y), w: Math.abs(p.x - current.startX), h: Math.abs(p.y - current.startY) }
        if (rect.w > 2 || rect.h > 2) current.moved = true
        current.current = rect
        setMarquee(rect)
      }
  }

  useLayoutEffect(() => { flushMoveRef.current = flushMove })

  const onPointerUp = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === 'touch' || !dragRef.current) return
    try { event.currentTarget.releasePointerCapture(event.pointerId) } catch { /* already released */ }
    finishDrag(false)
  }

  const onWheel = (event: ReactWheelEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement
    // a scrollable editor keeps its own wheel
    const scrollable = target.closest<HTMLElement>('.aiws-table-scroll, .aiws-prose-host, textarea, .aiws-html-host')
    if (scrollable && scrollable.scrollHeight > scrollable.clientHeight && !event.ctrlKey && !event.metaKey) return
    event.preventDefault()
    const p = screenPoint(event)
    if (event.ctrlKey || event.metaKey) camera.zoomAt(p.x, p.y, Math.exp(-event.deltaY * 0.0015))
    else if (event.shiftKey) camera.panBy(-event.deltaY, 0)
    else camera.panBy(-event.deltaX, -event.deltaY)
  }

  useEffect(() => {
    const down = (event: KeyboardEvent) => {
      if (event.code === 'Space' && !insideEditor(event.target)) { spaceHeld.current = true }
      if (event.key === 'Escape' && dragRef.current) { event.preventDefault(); finishDrag(true) }
    }
    const up = (event: KeyboardEvent) => { if (event.code === 'Space') spaceHeld.current = false }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => { window.removeEventListener('keydown', down); window.removeEventListener('keyup', up) }
  }, [finishDrag])


  /** The context menu of the Block at `p` (selecting it first) or of the blank spot. */
  const openMenuAt = (p: Point) => {
    const w = camera.toWorld(p.x, p.y)
    const hit = policy.select ? index.hit(w.x, w.y) : null
    if (hit && !selection.has(hit.id)) props.onSelectionChange(new Set([hit.id]))
    props.onContextMenu({ screenX: p.x, screenY: p.y, worldX: w.x, worldY: w.y }, hit?.id ?? null)
  }
  const onContextMenu = (event: React.MouseEvent<HTMLDivElement>) => {
    if (insideEditor(event.target)) return
    event.preventDefault()
    // a long touch opens the menu itself (touch.ts); the browser's own long-press menu is dropped
    if (pointerTypeRef.current === 'touch') return
    openMenuAt(screenPoint(event))
  }

  // ---- touch (UI improvement §16): what a touch gesture means here; touch.ts recognises the gestures
  const beginTouch = (event: PointerEvent, p: Point): OneFingerDrag | null => {
    const target = event.target instanceof Element ? event.target : null
    if (props.gesturesPaused || dragRef.current || target?.closest(TOUCH_EXEMPT)) return null
    const w = camera.toWorld(p.x, p.y)
    if (props.placing && !spaceHeld.current) { event.preventDefault(); props.onPlace?.(w); return null }
    const tappable = Boolean(target?.closest(TAPPABLE))
    const hit = policy.select && props.tool !== 'hand' ? index.hit(w.x, w.y) : null
    // edit mode: a drag that starts on a Block moves it, as with the mouse
    if (hit && !tappable && policy.layout && canLayout) { beginPointer(event); return { kind: 'delegate' } }
    // the selected Block's own scrollable content scrolls under the finger
    const scroller = hit && selection.has(hit.id) ? scrollerWithin(target, frames.current.get(hit.id)) : null
    return scroller ? { kind: 'scroll', element: scroller } : { kind: 'pan' }
  }
  const tapAt = (p: Point, target: Element | null) => {
    if (props.tool === 'hand' || (target?.closest(TAPPABLE) && target.closest('.aiws-frame-block'))) return // a control in a Block took the tap
    const w = camera.toWorld(p.x, p.y)
    const hit = policy.select ? index.hit(w.x, w.y) : null
    if (editing && editing !== hit?.id) endEditing()
    if (hit) props.onSelectionChange(new Set([hit.id]))
    else if (selection.size > 0) props.onSelectionChange(new Set())
  }
  const doubleTapAt = (p: Point) => {
    const w = camera.toWorld(p.x, p.y)
    const hit = policy.select && props.tool !== 'hand' ? index.hit(w.x, w.y) : null
    const l = hit ? laid.get(hit.id) : undefined
    if (hit && l && !l.isGroup && policy.editContent) { props.onEditingChange(hit.id); props.onSelectionChange(new Set([hit.id])); return }
    camera.animateTo(l ? camera.fitted(l.rect, 24) : camera.zoomedAt(p.x, p.y, 2))
  }
  useLayoutEffect(() => {
    const actions: TouchActions = {
      local: (clientX, clientY) => screenPoint({ clientX, clientY }),
      begin: beginTouch,
      delegateMove: (event) => trackPointer(event.clientX, event.clientY),
      delegateEnd: finishDrag,
      moving: (on) => { setDragging(on); worldRef.current?.classList.toggle('is-moving', on) },
      tap: tapAt,
      doubleTap: doubleTapAt,
      twoFingerTap: (p) => camera.animateTo(camera.zoomedAt(p.x, p.y, 0.5)),
      longPress: openMenuAt,
    }
    touch.setActions(actions)
  })
  // touches are followed from the window: a finger whose target left the DOM still moves and ends the gesture
  useEffect(() => {
    const move = (event: PointerEvent) => { if (event.pointerType === 'touch') touch.move(event) }
    const up = (event: PointerEvent) => { if (event.pointerType === 'touch') touch.up(event, false) }
    const cancel = (event: PointerEvent) => { if (event.pointerType === 'touch') touch.up(event, true) }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', up)
    window.addEventListener('pointercancel', cancel)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', up)
      window.removeEventListener('pointercancel', cancel)
      touch.reset()
    }
  }, [touch])

  const beginResize = (event: ReactPointerEvent<SVGRectElement>, id: string, handle: string) => {
    if (!policy.layout || !canLayout || dragRef.current) return
    const l = laid.get(id)
    if (!l) return
    event.stopPropagation()
    event.preventDefault()
    rootRef.current?.setPointerCapture(event.pointerId)
    setDragging(true)
    dragRef.current = { kind: 'resize', id, handle, startX: event.clientX, startY: event.clientY, start: { ...l.rect }, current: { ...l.rect } }
    worldRef.current?.classList.add('is-moving')
  }

  // ---- render
  useSyncExternalStore(store.outline.subscribe, store.outline.snapshot)
  const zoom = camera.settledZoom
  const frameList: ReactNode[] = []
  for (const [id, state] of mounts) {
    const l = laid.get(id)
    if (!l) continue
    const isPinned = pinned.has(id)
    const lod = l.isGroup ? 'full' : lodFor(l.rect, zoom, isPinned)
    frameList.push(
      <div
        key={id}
        ref={(el) => { if (el) frames.current.set(id, el); else frames.current.delete(id) }}
        className={`aiws-frame-block${l.isGroup ? ' is-group' : ''}${selection.has(id) ? ' is-selected' : ''}${hover === id ? ' is-hovered' : ''}${editing === id ? ' is-editing' : ''}`}
        data-block-id={id}
        data-testid={`aiws-canvas-block-${id}`}
        data-lod={lod}
        data-mount={state}
        style={{ left: l.rect.x, top: l.rect.y, width: l.rect.w, height: l.rect.h, display: state === 'hidden' ? 'none' : undefined, zIndex: l.depth }}
      >
        {l.isGroup ? (
          <div className="aiws-group-chrome"><span className="aiws-group-title">{l.entity.title ?? l.entity.name ?? '分组'}</span></div>
        ) : lod === 'placeholder' ? (
          <div className="aiws-block-placeholder" data-testid={`aiws-placeholder-${id}`}>{l.entity.title ?? l.entity.view_type ?? ''}</div>
        ) : (
          <BlockHost cellId={id} mode={mode} view="canvas" selected={selection.has(id)} hovered={hover === id} editorActive={editing === id}
            onActivate={handlersFor(id).onActivate} onDeactivate={handlersFor(id).onDeactivate} size={sizeFor(id, l.rect.w, l.rect.h)} zoom={zoom} lod={lod} />
        )}
      </div>,
    )
  }
  const screenRects = [...selection].flatMap((id) => { const l = laid.get(id); return l ? [{ id, rect: camera.rectToScreen(resizePreview?.id === id ? resizePreview.rect : l.rect) }] : [] })
  const bbox = screenRects.length > 0 ? screenRects.reduce((acc, { rect }) => ({ x: Math.min(acc.x, rect.x), y: Math.min(acc.y, rect.y), x2: Math.max(acc.x2, rect.x + rect.w), y2: Math.max(acc.y2, rect.y + rect.h) }), { x: Infinity, y: Infinity, x2: -Infinity, y2: -Infinity }) : null
  const hoverRect = hover && !selection.has(hover) ? laid.get(hover) : undefined
  const single = screenRects.length === 1 && policy.layout && canLayout ? screenRects[0] : null
  const handles = single ? ['nw', 'n', 'ne', 'e', 'se', 's', 'sw', 'w'].map((h) => {
    const r = single.rect
    const cx = h.includes('w') ? r.x : h.includes('e') ? r.x + r.w : r.x + r.w / 2
    const cy = h.includes('n') ? r.y : h.includes('s') ? r.y + r.h : r.y + r.h / 2
    return { h, cx, cy }
  }) : []
  return (
    <div
      ref={rootRef}
      className={`aiws-canvas aiws-canvas-mode-${mode}${props.tool === 'hand' ? ' is-hand' : ''}${props.placing ? ' is-placing' : ''}`}
      data-testid="aiws-canvas"
      data-surface-id={props.surfaceId}
      data-mode={mode}
      data-tool={props.placing ? 'place' : props.tool ?? 'select'}
      data-zoom={zoom.toFixed(2)}
      data-settled={settled}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={(event) => { if (event.pointerType !== 'touch') finishDrag(true) }}
      onDragStart={(event) => { if (!insideEditor(event.target)) event.preventDefault() }}
      onWheel={onWheel}
      onContextMenu={onContextMenu}
      onPointerLeave={() => {
        if (!dragRef.current && hover) setHover(null)
        if (ghostRef.current) ghostRef.current.style.visibility = 'hidden'
      }}
      onDoubleClick={(event) => {
        // pointer capture retargets clicks to the root: the double-click is resolved geometrically here
        if (!policy.editContent || insideEditor(event.target)) return
        const p = screenPoint(event)
        const w = camera.toWorld(p.x, p.y)
        const hit = index.hit(w.x, w.y)
        if (!hit || laid.get(hit.id)?.isGroup) return
        props.onEditingChange(hit.id)
        props.onSelectionChange(new Set([hit.id]))
      }}
    >
      <div ref={worldRef} className="aiws-world" data-testid="aiws-world">{frameList}</div>
      <svg className="aiws-overlay" data-testid="aiws-overlay">
        {hoverRect && policy.select && (() => { const r = camera.rectToScreen(hoverRect.rect); return <rect className="aiws-hover-outline" x={r.x} y={r.y} width={r.w} height={r.h} /> })()}
        <g ref={selectionLayerRef}>
          {screenRects.map(({ id, rect }) => <rect key={id} className="aiws-selection-box" data-testid={`aiws-selection-${id}`} x={rect.x} y={rect.y} width={rect.w} height={rect.h} />)}
          {handles.map(({ h, cx, cy }) => (
            <rect key={h} className="aiws-handle" data-testid={`aiws-handle-${h}`} x={cx - HANDLE / 2} y={cy - HANDLE / 2} width={HANDLE} height={HANDLE} style={{ cursor: `${h}-resize` }} onPointerDown={(event) => beginResize(event, single!.id, h)} />
          ))}
        </g>
        {marquee && <rect className="aiws-marquee" data-testid="aiws-marquee" x={marquee.x} y={marquee.y} width={marquee.w} height={marquee.h} />}
      </svg>
      {props.placing && <div ref={ghostRef} className="aiws-place-ghost" data-testid="aiws-place-ghost" style={{ visibility: 'hidden' }}><span>{props.placing.label}</span></div>}
      {bbox && props.renderNear && !resizePreview && !dragging && props.renderNear({ x: bbox.x, y: bbox.y, w: bbox.x2 - bbox.x, h: bbox.y2 - bbox.y })}
    </div>
  )
}
