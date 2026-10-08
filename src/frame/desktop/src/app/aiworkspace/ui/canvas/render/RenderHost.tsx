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
 * Touch has its own gestures (touch.ts, UI improvement §16): pan, pinch, taps, long press.
 *
 * Stacking: painting and hit testing follow one order, the BlockTree pre-order (`Laid.paint`: a parent
 * before its children, siblings by `order_key`), and the Block being edited above all. The order is a
 * z-index; the frames keep a stable DOM order (by id) so that culling or a reorder never moves a frame,
 * which would reload an embedded document or drop an editor's focus.
 *
 * Object states (标准对象的交互改进 §4): a frame draws nothing of its own; hover, selection, editing and
 * dragging live in the overlay only — a solid hover outline with the Block's affordances after a short
 * delay, a selection box with round corner handles, edge resize zones and a rotation handle, a group box
 * for several Blocks, a lock badge, the cut marker and a size / angle hint while a gesture runs. Handle
 * sizes follow the pointer type. A rotated Block turns its frame about the centre; hit tests and resizing
 * work in its own coordinates.
 *
 * Connectors (连接线实现方案 §9; 标准对象的交互改进 §6): a line is a frame in the same paint order, hit within a
 * few pixels of its path. A selected Block offers four connection handles to drag a line out of; the connector
 * tool draws one anywhere; a dragged end snaps to a side midpoint (`point`) or to the Block under it (`auto`).
 * A selected line shows its end, bend, segment and label handles and a halo instead of a box. While Blocks
 * move, resize or turn, the lines bound to them are re-routed and repainted per frame without React; dragging
 * near the edge of the canvas pans it. */

import { useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type PointerEvent as ReactPointerEvent, type ReactNode, type WheelEvent as ReactWheelEvent } from 'react'
import { Lock, RotateCw } from 'lucide-react'
import type { Placement } from '../../../api/types'
import { ConnectorFrame } from '../connectors/ConnectorFrame'
import { LineRegistry } from '../connectors/registry'
import { anchorLabel, findAnchor, SIDE_VEC } from '../connectors/anchors'
import { anchorsOf, anchorWorld, cubicAt, decompose, dist, ELBOW_STUB, facingAnchor, leftOf, materialize, nearest, pathData, routeBetween, routeConnector, sideAnchor, visibleAnchors, type ConnectorGeometry, type Seg, type TargetGeom } from '../connectors/geometry'
import { CONNECTOR_PAD, connectorAdjacency, hasBoundEnd, lookupIn, RECT_FORM, routeLaid, type Override, type TargetForm } from '../connectors/layout'
import { LABEL_FONT, sameAnchor, type Anchor, type ConnectorData, type Route } from '../connectors/model'
import type { ConnectorChange, EndSpec, NewConnector } from '../connectors/ops'
import { paintConnector } from '../connectors/paint'
import { describeError } from '../../../api/session'
import { testHooks } from '../../../api/testHooks'
import { useStore } from '../../../state/hooks'
import { BlockHost, type Lod } from '../../blocks/BlockHost'
import { BudgetContext } from '../../blocks/budget'
import { BlockMetaContext, BlockMetaSink, requestIntent, type BlockMeta } from '../../blocks/editorToolbar'
import { blockRegistry, modePolicy, type CanvasMode, type HoverAffordance } from '../../blocks/registry'
import { usePointerType } from '../../blocks/useBlockContext'
import { FreshnessBadge } from '../../sources/FreshnessBadge'
import { angleFrom, centerOf, containsPoint, corners, normalizeRotation, resizeRotated, rotatePoint } from '../geometry'
import { movedPlacement, relativeTo, topLevel, type Laid } from '../layout'
import { Camera, intersects, type Rect } from './camera'
import { SpatialIndex } from './spatialIndex'
import { TouchGestures, type OneFingerDrag, type Point, type TouchActions } from './touch'

/** Blocks within this many viewports of the visible area stay mounted; within the next band they stay in the DOM but hidden. */
const MOUNT_MARGIN = 0.5
const HIDE_MARGIN = 2
/** At most this many content Blocks carry a mounted Renderer at once; the rest stay hidden. Lines are not counted. */
const MAX_MOUNTED = 400
const PLACEHOLDER_PX = 48
const SIMPLIFIED_ZOOM = 0.4
const MIN_SIZE = 40
/** Handle sizes in screen px by pointer type (§4.3): what is drawn and what takes the pointer. */
const HANDLES = {
  mouse: { corner: 10, cornerHit: 14, edge: 10, rotate: 22, rotateOffset: 18 },
  touch: { corner: 14, cornerHit: 44, edge: 24, rotate: 36, rotateOffset: 26 },
}
/** Hover affordances wait this long, and need this much Block on screen for their buttons (§4.2). */
const AFFORD_DELAY = 150
const AFFORD_BUTTON_MIN = 64
const ROTATE_SNAP = 15
/** Above this many moved frames a drag shows outlines only (one hide, one restore) instead of per-frame transforms. */
const GHOST_DRAG_LIMIT = 24
/** A line takes the pointer this close to its path (screen px); a dragged end snaps to a side midpoint this close. */
const LINE_HIT_PX = 6
const SNAP_PX = 12
/** Connection handles sit this far outside the side midpoints (§4.3); drawn and hit radius. */
const CONNECT_OFFSET = 14
const CONNECT_DOT = 4
/** Dragging within this distance of the canvas edge pans it, this fast (px per ms). */
const EDGE_PX = 24
const EDGE_SPEED = 0.6
/** Two presses on a handle within this time are a double click (pointer capture keeps dblclick away from it). */
const DOUBLE_PRESS_MS = 400

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
  /** The pointer tool (UI improvement §7.2): `hand` pans with the primary button and never selects; `connector` draws a line. */
  tool?: 'select' | 'hand' | 'connector'
  /** A one-shot placement: the pointer carries a preview of this size; a primary click places it. */
  placing?: { w: number; h: number; label: string } | null
  onPlace?: (world: { x: number; y: number }) => void
  /** Objects marked by "cut" (dashed until pasted or cancelled). */
  cutIds?: ReadonlySet<string>
  /** The outline and anchors each Block declares: connector ends meet them. */
  formOf?: (id: string) => TargetForm
  /** A line was drawn (from a connection handle or with the connector tool); `blank`: its end is on no object. */
  onConnectorCreate?: (spec: NewConnector, release: { blank: boolean; screen: Point }) => void
  /** One gesture's edit of a line (an end, its bends, its label). */
  onConnectorChange?: (id: string, change: ConnectorChange) => void
}

/** Where a dragged line end would attach: a Block, the anchor on it, the point, and whether the pointer is on that
 * anchor (`exact`) or only inside the Block (the anchor facing the other end). */
interface Snap { id: string; anchor: Anchor; point: Point; target: TargetGeom; exact: boolean }

type Drag =
  | { kind: 'pan'; lastX: number; lastY: number; moved: boolean }
  /** `lines`: lines re-routed per frame (bound to a moved Block, or moved with a bound end). */
  | { kind: 'move'; ids: string[]; affected: string[]; lines: string[]; shifted: ReadonlySet<string>; startX: number; startY: number; dx: number; dy: number; moved: boolean; clicked: string; additive: boolean; wasSelected: boolean; ghost?: boolean }
  | { kind: 'resize'; id: string; handle: string; startX: number; startY: number; start: Rect; rotation: number; aspect: 'free' | 'locked'; current: Rect; lines: string[] }
  | { kind: 'rotate'; id: string; center: Point; startAngle: number; start: number; current: number; lines: string[] }
  | { kind: 'marquee'; startX: number; startY: number; current: Rect | null; additive: boolean; moved: boolean }
  /** Drawing a new line from `from`. */
  /** `loose`: the start was pressed inside a Block, not on an anchor — it takes the anchor facing the end. */
  | { kind: 'connect'; from: EndSpec; loose: boolean; startX: number; startY: number; end: EndSpec | null; endExact: boolean; route: Route; screen: Point; moved: boolean }
  /** Dragging one end of a line. */
  | { kind: 'endpoint'; id: string; which: 'start' | 'end'; startX: number; startY: number; end: EndSpec | null; endExact: boolean; route: Route | null; moved: boolean }
  /** Dragging a bend (`move` point `index` of `base`) or an elbow segment (`segment` `index` of the corner list `base`);
   * a press on the line's body that does not move is a click (`clicked`). */
  | { kind: 'bend'; id: string; mode: 'move' | 'segment'; base: Point[]; index: number; press: Point; working: Point[] | null; startX: number; startY: number; moved: boolean; clicked: boolean; additive: boolean; wasSelected: boolean }
  | { kind: 'label'; id: string; startX: number; startY: number; label: { t: number; offset: number } | null; moved: boolean }

/** What a gesture shows next to the pointer: the size while resizing, the angle while rotating. */
interface Hint { x: number; y: number; text: string }

/** A cursor for a resize zone, turned with the Block. */
function resizeCursor(handle: string, rotation: number): string {
  const base: Record<string, number> = { n: 0, ne: 45, e: 90, se: 135, s: 180, sw: 225, w: 270, nw: 315 }
  const names = ['ns-resize', 'nesw-resize', 'ew-resize', 'nwse-resize']
  const angle = ((base[handle] ?? 0) + rotation + 360 + 22.5) % 180
  return names[Math.floor(angle / 45) % 4]
}

/** An elbow segment `k` of the corner list `pts` (S … E) dragged across its axis: its two corners move; a first or
 * last segment first gets a short stub at its end, so the line still leaves and enters the way it did. */
function dragSegment(pts: Point[], k: number, dx: number, dy: number): Point[] {
  const n = pts.length - 1
  const a = pts[k]
  const b = pts[k + 1]
  if (!a || !b) return pts.slice(1, -1)
  const horizontal = Math.abs(a.y - b.y) < Math.abs(a.x - b.x)
  const shift = (p: Point): Point => (horizontal ? { x: p.x, y: p.y + dy } : { x: p.x + dx, y: p.y })
  const stub = (from: Point, to: Point): Point => {
    const l = dist(from, to)
    const k2 = l > 1e-9 ? Math.min(ELBOW_STUB, l / 2) / l : 0
    return { x: from.x + (to.x - from.x) * k2, y: from.y + (to.y - from.y) * k2 }
  }
  if (n === 1) { const s1 = stub(pts[0], pts[1]); const e1 = stub(pts[1], pts[0]); return [s1, shift(s1), shift(e1), e1] }
  if (k === 0) { const s1 = stub(pts[0], pts[1]); return [s1, shift(s1), shift(pts[1]), ...pts.slice(2, n)] }
  if (k === n - 1) { const e1 = stub(pts[n], pts[n - 1]); return [...pts.slice(1, n - 1), shift(pts[n - 1]), shift(e1), e1] }
  return pts.slice(1, n).map((p, i) => (i + 1 === k || i + 1 === k + 1 ? shift(p) : p))
}

type MountState = 'mounted' | 'hidden'

/** What starting a mouse-path gesture needs: a React pointer event, or a native one handed over by a touch. */
type PointerStart = Pick<PointerEvent, 'pointerId' | 'button' | 'clientX' | 'clientY' | 'shiftKey' | 'metaKey' | 'ctrlKey' | 'target' | 'preventDefault' | 'timeStamp'>

/** Controls inside a Block that take a tap themselves (their click must survive). */
const TAPPABLE = 'button:not(:disabled), a[href], summary, label'
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
    // a line has no Renderer to budget: it stays drawn wherever it crosses the view (连接线方案 §9.4)
    if (item.line && intersects(item.rect, near)) next.set(item.id, 'mounted')
    else if (intersects(item.rect, near)) candidates.push({ id: item.id, dist: Math.hypot(item.rect.x + item.rect.w / 2 - center.x, item.rect.y + item.rect.h / 2 - center.y) })
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
  const shiftHeld = useRef(false)
  const policy = modePolicy(mode)
  const [marquee, setMarquee] = useState<Rect | null>(null)
  const [dragging, setDragging] = useState(false)
  const [hover, setHover] = useState<string | null>(null)
  /** The hovered Block whose affordances show (after AFFORD_DELAY). */
  const [shownHover, setShownHover] = useState<string | null>(null)
  const hoverTimer = useRef(0)
  const [resizePreview, setResizePreview] = useState<{ id: string; rect: Rect } | null>(null)
  const [rotatePreview, setRotatePreview] = useState<{ id: string; rotation: number } | null>(null)
  const [hint, setHint] = useState<Hint | null>(null)
  const [, forceOverlay] = useState(0)
  const [settled, setSettled] = useState(0)
  const pointerTypeRef = useRef('mouse')
  const pointer = usePointerType()
  const sizes = HANDLES[pointer]
  const [touch] = useState(() => new TouchGestures(camera))
  const [metaSink] = useState(() => new BlockMetaSink())
  useSyncExternalStore(metaSink.subscribe, metaSink.snapshot)
  // ---- connectors: mounted line frames (for repainting during gestures), which lines end on which Block
  const [lines] = useState(() => new LineRegistry())
  const adjacency = useMemo(() => connectorAdjacency(laid), [laid])
  /** A gesture's live line: its geometry, and (dragging an end) the Block the end would attach to. */
  const [linePreview, setLinePreview] = useState<{ id: string | null; geom: ConnectorGeometry; snap: Snap | null } | null>(null)
  const formOf = props.formOf ?? (() => RECT_FORM)
  const lookupWith = (override?: Override) => lookupIn(laid, store.outline, formOf, override)
  /** A Block as a line end's target (not a line). */
  const targetOf = (id: string): TargetGeom | null => {
    const l = laid.get(id)
    if (!l || l.connector) return null
    const form = formOf(id)
    return { rect: l.rect, rotation: l.rotation, shape: form.shape, anchors: form.anchors }
  }
  const routeOptions = (id: string | null) => ({ title: id ? laid.get(id)?.entity.title ?? null : null, font: LABEL_FONT.m, pad: CONNECTOR_PAD })
  const repaint = (id: string, g: ConnectorGeometry) => { const e = lines.get(id); if (e) paintConnector(e.el, g, e.style, e.options) }
  /** After a gesture: the frames show the committed layout again (React repaints when it changes). */
  const restoreLines = (ids: Iterable<string>) => { for (const id of ids) { const l = laid.get(id); if (l?.connector) repaint(id, l.connector.geom) } }
  /** The lines a gesture on `affected` re-routes: those ending on one of them, and moved ones with a bound end. */
  const linesTouching = (affected: Iterable<string>): string[] => {
    const out = new Set<string>()
    for (const id of affected) {
      for (const line of adjacency.get(id) ?? []) out.add(line)
      if (hasBoundEnd(laid.get(id))) out.add(id)
    }
    return [...out]
  }
  /** Re-route and repaint `ids` with Blocks moved by `override` (and lines themselves moved by `offsetOf`). */
  const rerouteLines = (ids: string[], override: Override, offsetOf: (id: string) => Point | undefined = () => undefined) => {
    if (ids.length === 0) return
    const lookup = lookupWith(override)
    for (const id of ids) { const l = laid.get(id); if (l?.connector) repaint(id, routeLaid(l, l.connector.data, lookup, offsetOf(id))) }
  }
  const frameEl = (id: string) => frames.current.get(id) ?? lines.get(id)?.el
  /** Can the user change this line's ends, bends and label here (mouse; touch only selects lines)? */
  const lineEditable = (id: string) => policy.layout && canLayout && pointer === 'mouse' && !laid.get(id)?.locked && Boolean(store.outline.get(id)?.capabilities.includes('update'))
  /** Hover changes at once (the outline); affordances follow after a short delay and leave at once. */
  const changeHover = (id: string | null) => {
    if (id === hover) return
    setHover(id)
    window.clearTimeout(hoverTimer.current)
    setShownHover(null)
    if (id) hoverTimer.current = window.setTimeout(() => setShownHover(id), AFFORD_DELAY)
  }
  useEffect(() => () => window.clearTimeout(hoverTimer.current), [])

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
  // frames in a stable DOM order (by id): mounting, culling or a reorder inserts and removes, never moves
  const frameOrder = useMemo(() => [...mounts.keys()].sort(), [mounts])

  // ---- stats for the render probe (dev override only)
  useEffect(() => {
    const hooks = testHooks()
    if (!hooks) return
    let mounted = 0, hidden = 0, placeholders = 0
    const zoom = camera.settledZoom
    for (const [id, state] of mounts) {
      if (state === 'hidden') { hidden += 1; continue }
      const l = laid.get(id)
      if (l && !l.isGroup && !l.connector && lodFor(l.rect, zoom, pinned.has(id)) === 'placeholder') placeholders += 1
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
  // `translate` composes with a Block's own `rotate` (it is applied after it), unlike `transform`
  const setTransform = (ids: Iterable<string>, dx: number, dy: number) => {
    for (const id of ids) { const el = frameEl(id); if (el) el.style.translate = dx === 0 && dy === 0 ? '' : `${dx}px ${dy}px` }
  }
  /** Leaving an editor from the canvas ends its input first (its blur saves), before the editor unmounts. */
  const endEditing = () => {
    const active = document.activeElement
    if (active instanceof HTMLElement && rootRef.current?.contains(active)) active.blur()
    props.onEditingChange(null)
  }
  const insideEditor = (target: EventTarget | null) => {
    const el = target as HTMLElement | null
    // a disabled button (a read-only table cell) is content, not a control: the canvas takes the gesture
    return Boolean(el?.closest?.('[data-role="editor"], input, textarea, select, [contenteditable="true"], .aiws-near, .aiws-menu, button:not(:disabled)'))
  }
  /** The top-most Block at a world point, as painted: the Block being edited first, then the paint order;
   * a line within a few screen pixels of its path. */
  const hitAt = (x: number, y: number) => {
    const top = editing ? index.get(editing) : undefined
    if (top && !top.line && (top.turned ? containsPoint(top.turned.rect, top.turned.rotation, x, y) : x >= top.rect.x && x <= top.rect.x + top.rect.w && y >= top.rect.y && y <= top.rect.y + top.rect.h)) return top
    return index.hit(x, y, { slop: LINE_HIT_PX / camera.zoom, lineTolerance: (item) => lines.halfWidth(item.id) })
  }
  /** Where a dragged line end at screen point `p` attaches (§4.3): an anchor within SNAP_PX of the pointer (the
   * top-most Block's, then the nearest), else — inside a Block — its anchor facing `toward`, else nothing. Lines,
   * unreadable and excluded Blocks never attach; minor anchors count only while they are shown. */
  const snapAt = (p: Point, toward: Point, exclude?: string): Snap | null => {
    const w = camera.toWorld(p.x, p.y)
    const r = SNAP_PX / camera.zoom
    const usable = (id: string) => (id === exclude ? null : targetOf(id))
    let best: Snap | null = null
    let bestPaint = -1
    let bestD = Infinity
    for (const item of index.query({ x: w.x - r, y: w.y - r, w: r * 2, h: r * 2 })) {
      const target = item.line ? null : usable(item.id)
      if (!target) continue
      for (const spot of visibleAnchors(target, camera.zoom)) {
        const s = camera.toScreen(spot.point.x, spot.point.y)
        const d = Math.hypot(s.x - p.x, s.y - p.y)
        if (d > SNAP_PX || item.paint < bestPaint || (item.paint === bestPaint && d >= bestD)) continue
        best = { id: item.id, anchor: spot.anchor, point: spot.point, target, exact: true }
        bestPaint = item.paint
        bestD = d
      }
    }
    if (best) return best
    const hit = index.hit(w.x, w.y, { accept: (item) => !item.line && usable(item.id) !== null })
    const target = hit ? usable(hit.id) : null
    if (!hit || !target) return null
    const anchor = facingAnchor(target, toward)
    return anchor ? { id: hit.id, anchor, point: anchorWorld(target, anchor, toward), target, exact: false } : null
  }
  /** What the connector tool would start from at the pointer (its anchors are shown before the press). */
  const [toolSnap, setToolSnap] = useState<Snap | null>(null)
  const followToolSnap = (p: Point | null) => {
    const next = p ? snapAt(p, camera.toWorld(p.x, p.y)) : null
    setToolSnap((before) => (before?.id === next?.id && before?.exact === next?.exact && (!before || !next || sameAnchor(before.anchor, next.anchor)) && before?.target.rect === next?.target.rect ? before : next))
  }

  // ---- edge panning: a drag near the canvas edge moves the camera; what follows the pointer is shifted with it
  const edgeRef = useRef<{ raf: number; vx: number; vy: number; last: number; clientX: number; clientY: number } | null>(null)
  const stopEdgePan = () => {
    if (edgeRef.current) cancelAnimationFrame(edgeRef.current.raf)
    edgeRef.current = null
  }
  const followEdge = (clientX: number, clientY: number) => {
    const drag = dragRef.current
    const root = rootRef.current
    if (!drag || drag.kind === 'pan' || drag.kind === 'rotate' || !root) { stopEdgePan(); return }
    const r = root.getBoundingClientRect()
    const vx = clientX < r.left + EDGE_PX ? EDGE_SPEED : clientX > r.right - EDGE_PX ? -EDGE_SPEED : 0
    const vy = clientY < r.top + EDGE_PX ? EDGE_SPEED : clientY > r.bottom - EDGE_PX ? -EDGE_SPEED : 0
    if (!vx && !vy) { stopEdgePan(); return }
    if (edgeRef.current) { Object.assign(edgeRef.current, { vx, vy, clientX, clientY }); return }
    const state = { raf: 0, vx, vy, last: -1, clientX, clientY }
    edgeRef.current = state
    const step = (t: number) => {
      const d = dragRef.current
      if (edgeRef.current !== state || !d) return
      const dt = state.last < 0 ? 16 : Math.min(48, Math.max(0, t - state.last))
      state.last = t
      const px = state.vx * dt
      const py = state.vy * dt
      camera.panBy(px, py)
      if (d.kind === 'move' || d.kind === 'resize' || d.kind === 'marquee') { d.startX += px; d.startY += py }
      trackPointer(state.clientX, state.clientY)
      state.raf = requestAnimationFrame(step)
    }
    state.raf = requestAnimationFrame(step)
  }
  useEffect(() => () => stopEdgePan(), [])

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
    stopEdgePan()
    setDragging(false)
    worldRef.current?.classList.remove('is-moving')
    if (!drag) return
    /** A press that did not move: select (shift toggles). */
    const click = (id: string, additive: boolean, wasSelected: boolean) => {
      const next = new Set(additive ? selection : [])
      if (additive && wasSelected) next.delete(id)
      else next.add(id)
      props.onSelectionChange(next)
    }
    if (drag.kind === 'move') {
      if (drag.ghost) { for (const gid of [...drag.affected, ...drag.lines]) { const el = frameEl(gid); if (el) el.style.visibility = '' } } else setTransform(drag.affected, 0, 0)
      restoreLines(drag.lines)
      selectionLayerRef.current?.removeAttribute('transform')
      if (!cancel && drag.moved) {
        const changes: LayoutChange[] = []
        for (const id of drag.ids) {
          const l = laid.get(id)
          if (!l) continue
          changes.push({ id, placement: movedPlacement(laid, l, drag.dx, drag.dy) })
        }
        if (changes.length > 0) props.onCommitLayout(changes)
      } else if (!drag.moved && !cancel) click(drag.clicked, drag.additive, drag.wasSelected)
    } else if (drag.kind === 'resize') {
      setResizePreview(null)
      setHint(null)
      restoreLines(drag.lines)
      const el = frames.current.get(drag.id)
      const l = laid.get(drag.id)
      if (el && l) { el.style.left = `${l.rect.x}px`; el.style.top = `${l.rect.y}px`; el.style.width = `${l.rect.w}px`; el.style.height = `${l.rect.h}px` }
      if (!cancel && l && (drag.current.w !== drag.start.w || drag.current.h !== drag.start.h || drag.current.x !== drag.start.x || drag.current.y !== drag.start.y)) {
        props.onCommitLayout([{ id: drag.id, placement: relativeTo(laid, l.parentId, drag.current, l.rotation) }])
      }
    } else if (drag.kind === 'rotate') {
      setRotatePreview(null)
      setHint(null)
      restoreLines(drag.lines)
      const el = frames.current.get(drag.id)
      const l = laid.get(drag.id)
      if (el && l) el.style.rotate = l.rotation ? `${l.rotation}deg` : ''
      if (!cancel && l && drag.current !== l.rotation) props.onCommitLayout([{ id: drag.id, placement: relativeTo(laid, l.parentId, l.rect, drag.current) }])
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
    } else if (drag.kind === 'connect') {
      setLinePreview(null)
      if (cancel || !drag.moved || !drag.end) return
      const from = drag.from.binding
      const to = drag.end.binding
      // a loop needs two different anchors the pointer was on (§4.3)
      if (from && to && from.entity_id === to.entity_id && (drag.loose || !drag.endExact || sameAnchor(from.anchor, to.anchor))) {
        store.notify('info', '首尾连在同一个对象上时，两端要放在它两个不同的锚点上。')
        return
      }
      // the stored corners are where the ends are now (an `auto` start depends on where the end went)
      const g = routeConnector({ start: from, end: to, flip: {}, route: drag.route, controls: [], label: { t: 0.5, offset: 0 } }, [drag.from.point, drag.end.point], lookupWith(), routeOptions(null))
      if (!from && !to && dist(g.start.point, g.end.point) * camera.zoom < 4) return
      props.onConnectorCreate?.({ start: { binding: from, point: g.start.point }, end: { binding: to, point: g.end.point }, route: drag.route }, { blank: !to, screen: drag.screen })
    } else if (drag.kind === 'endpoint') {
      setLinePreview(null)
      restoreLines([drag.id])
      const l = laid.get(drag.id)
      if (cancel || !drag.moved || !drag.end || !l?.connector) return
      const data: ConnectorData = { ...l.connector.data, [drag.which]: drag.end.binding, ...(drag.route ? { route: drag.route } : {}) }
      const other = drag.which === 'start' ? data.end : data.start
      const mine = drag.end.binding
      if (mine && other && mine.entity_id === other.entity_id && (!drag.endExact || sameAnchor(mine.anchor, other.anchor))) {
        store.notify('info', '首尾连在同一个对象上时，两端要放在它两个不同的锚点上。')
        return
      }
      const geom = l.connector.geom
      const stored: [Point, Point] = drag.which === 'start' ? [drag.end.point, geom.end.point] : [geom.start.point, drag.end.point]
      const g = routeConnector(data, stored, lookupWith(), routeOptions(drag.id))
      props.onConnectorChange?.(drag.id, { kind: 'end', which: drag.which, spec: { binding: mine, point: drag.which === 'start' ? g.start.point : g.end.point }, ...(drag.route ? { route: drag.route } : {}) })
    } else if (drag.kind === 'bend') {
      setLinePreview(null)
      restoreLines([drag.id])
      if (!cancel && drag.moved && drag.working) props.onConnectorChange?.(drag.id, { kind: 'controls', points: drag.working })
      else if (!cancel && !drag.moved && drag.clicked) click(drag.id, drag.additive, drag.wasSelected)
    } else if (drag.kind === 'label') {
      setLinePreview(null)
      restoreLines([drag.id])
      if (!cancel && drag.moved && drag.label) props.onConnectorChange?.(drag.id, { kind: 'label', label: drag.label })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the helpers it calls are rebuilt with `laid` / `props`, which are listed
  }, [laid, selection, props, camera, index])

  // ---- pointer gestures
  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    pointerTypeRef.current = event.pointerType
    if (event.pointerType === 'touch') { touch.down(event.nativeEvent); return }
    if (props.gesturesPaused || dragRef.current) return
    if (insideEditor(event.target)) return
    beginPointer(event)
  }

  /** The mouse path: place, draw a line, pan, move, or marquee. */
  const beginPointer = (event: PointerStart) => {
    const p = screenPoint(event)
    const w = camera.toWorld(p.x, p.y)
    if (props.placing && event.button === 0 && !spaceHeld.current) {
      event.preventDefault()
      props.onPlace?.(w)
      return
    }
    // the connector tool: a press on an anchor starts there, inside a Block the start takes the anchor facing
    // the end, a press on blank space starts a free line
    if (props.tool === 'connector' && event.button === 0 && !spaceHeld.current && props.onConnectorCreate) {
      const snap = snapAt(p, w)
      setToolSnap(null)
      beginConnect(event, snap ? { binding: { entity_id: snap.id, anchor: snap.anchor }, point: snap.point } : { binding: null, point: w }, Boolean(snap && !snap.exact))
      return
    }
    const hand = props.tool === 'hand'
    const hit = policy.select && !hand ? hitAt(w.x, w.y) : null
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
      if (editing && editing !== id) endEditing()
      // a line with a bound end does not move as a whole: dragging its body bends it (§9.6), a press selects it
      if (hasBoundEnd(laid.get(id))) {
        if (!wasSelected && !additive) props.onSelectionChange(new Set([id]))
        // a second press soon after the first edits the label (the first one shows handles that would take a dblclick)
        if (lineEditable(id) && !additive && secondPress(`line:${id}`, event)) { setDragging(false); props.onEditingChange(id); return }
        if (lineEditable(id)) beginBend(event, id, null, w, { clicked: true, additive, wasSelected })
        else dragRef.current = { kind: 'move', ids: [], affected: [], lines: [], shifted: new Set(), startX: event.clientX, startY: event.clientY, dx: 0, dy: 0, moved: false, clicked: id, additive, wasSelected }
        return
      }
      // locked Blocks are selected but never moved, also as part of a selection (§7.2)
      const ids = policy.layout && canLayout ? topLevel(laid, wasSelected && !additive ? new Set(selection) : new Set([id])).filter((moved) => !laid.get(moved)?.locked) : []
      const affected = ids.flatMap((moved) => [moved, ...descendantsOf(moved)])
      const touching = linesTouching(affected)
      if (!wasSelected && !additive) props.onSelectionChange(new Set([id]))
      // re-routed lines are repainted, not translated
      dragRef.current = { kind: 'move', ids, affected: affected.filter((a) => !touching.includes(a)), lines: touching, shifted: new Set(affected), startX: event.clientX, startY: event.clientY, dx: 0, dy: 0, moved: false, clicked: id, additive, wasSelected }
      return
    }
    if (editing) endEditing()
    dragRef.current = { kind: 'marquee', startX: p.x, startY: p.y, current: null, additive: event.shiftKey, moved: false }
  }

  /** Start drawing a line from `from` (a connection handle, or the connector tool's press). */
  const beginConnect = (event: PointerStart, from: EndSpec, loose = false) => {
    event.preventDefault()
    rootRef.current?.setPointerCapture(event.pointerId)
    window.getSelection()?.removeAllRanges()
    setDragging(true)
    if (editing) endEditing()
    dragRef.current = { kind: 'connect', from, loose, startX: event.clientX, startY: event.clientY, end: null, endExact: false, route: 'straight', screen: screenPoint(event), moved: false }
  }
  /** Start dragging a bend: a stored or materialised point (`index`), an elbow segment, or — `index` null — the
   * piece of the line under the pointer (a new bend there; on an elbow, that segment). */
  const beginBend = (event: PointerStart, id: string, index: number | null, press: Point, click?: { clicked: boolean; additive: boolean; wasSelected: boolean }, insert = false) => {
    const g = laid.get(id)?.connector?.geom
    if (!g) return
    // no preventDefault: the press may be the first half of a double-click that edits the label
    rootRef.current?.setPointerCapture(event.pointerId)
    window.getSelection()?.removeAllRanges()
    setDragging(true)
    const common = { id, press, working: null, startX: event.clientX, startY: event.clientY, moved: false, clicked: click?.clicked ?? false, additive: click?.additive ?? false, wasSelected: click?.wasSelected ?? true }
    if (g.route === 'elbow') {
      const seg = index ?? nearest(g, press).seg
      dragRef.current = { kind: 'bend', mode: 'segment', base: g.pts, index: seg, ...common }
      return
    }
    const through = materialize(g)
    if (index !== null && !insert) { dragRef.current = { kind: 'bend', mode: 'move', base: through, index, ...common }; return }
    // a new bend between two points the line passes through (an automatic curve has none yet)
    const at = index ?? nearest(g, press).seg
    const base = g.auto ? [] : through
    const slot = Math.min(at, base.length)
    dragRef.current = { kind: 'bend', mode: 'move', base: [...base.slice(0, slot), press, ...base.slice(slot)], index: slot, ...common }
  }
  /** Double clicks on handles: a press soon after the previous one on the same handle, at the same spot. */
  const lastHandlePress = useRef<{ key: string; at: number; x: number; y: number }>({ key: '', at: -1000, x: 0, y: 0 })
  const secondPress = (key: string, event: { timeStamp: number; clientX: number; clientY: number }) => {
    const last = lastHandlePress.current
    const again = last.key === key && event.timeStamp - last.at < DOUBLE_PRESS_MS && Math.hypot(event.clientX - last.x, event.clientY - last.y) < 8
    lastHandlePress.current = again ? { key: '', at: -1000, x: 0, y: 0 } : { key, at: event.timeStamp, x: event.clientX, y: event.clientY }
    handleDownAt.current = event.timeStamp
    return again
  }
  const beginLineEnd = (event: ReactPointerEvent<Element>, id: string, which: 'start' | 'end') => {
    if (dragRef.current || !lineEditable(id) && !(policy.layout && canLayout)) return
    event.stopPropagation()
    event.preventDefault()
    if (secondPress(`end:${id}:${which}`, event)) {
      const l = laid.get(id)
      if ((which === 'start' ? l?.connector?.geom.start : l?.connector?.geom.end)?.state === 'broken') store.notify('info', '这一端连接的对象已不在这张画布上或无法访问。拖动这个端点到一个对象上即可重新连接，拖到空白处则改为普通端点。')
      return
    }
    rootRef.current?.setPointerCapture(event.pointerId)
    setDragging(true)
    dragRef.current = { kind: 'endpoint', id, which, startX: event.clientX, startY: event.clientY, end: null, endExact: false, route: null, moved: false }
  }
  const beginLineHandle = (event: ReactPointerEvent<Element>, id: string, index: number, insert: boolean) => {
    if (dragRef.current || !lineEditable(id)) return
    event.stopPropagation()
    const l = laid.get(id)
    const g = l?.connector?.geom
    if (!g || !l?.connector) return
    // a double click on a bend removes it
    if (!insert && g.route !== 'elbow' && secondPress(`bend:${id}:${index}`, event)) {
      event.preventDefault()
      const rest = materialize(g).filter((_, i) => i !== index)
      props.onConnectorChange?.(id, { kind: 'controls', points: rest.length ? rest : null })
      return
    }
    // a double click on the middle of a piece edits the label (the handle sits where people double-click)
    if (insert && secondPress(`line:${id}`, event)) { event.preventDefault(); props.onEditingChange(id); return }
    if (!insert) handleDownAt.current = event.timeStamp
    const p = screenPoint(event)
    beginBend(event, id, index, camera.toWorld(p.x, p.y), undefined, insert)
  }
  const beginLabelDrag = (event: ReactPointerEvent<Element>, id: string) => {
    if (dragRef.current || !lineEditable(id)) return
    event.stopPropagation()
    event.preventDefault()
    if (secondPress(`label:${id}`, event)) { props.onEditingChange(id); return }
    rootRef.current?.setPointerCapture(event.pointerId)
    setDragging(true)
    dragRef.current = { kind: 'label', id, startX: event.clientX, startY: event.clientY, label: null, moved: false }
  }
  const beginConnectFrom = (event: ReactPointerEvent<Element>, id: string, anchor: Anchor) => {
    const l = laid.get(id)
    if (dragRef.current || !l) return
    event.stopPropagation()
    handleDownAt.current = event.timeStamp
    const target = targetOf(id)
    if (!target) return
    beginConnect(event, { binding: { entity_id: id, anchor }, point: anchorWorld(target, anchor, centerOf(l.rect)) })
  }


  const ghostRef = useRef<HTMLDivElement>(null)
  const onPointerMove = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.pointerType === 'touch') return // window listeners follow touches
    shiftHeld.current = event.shiftKey
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
      // the connector tool shows the anchors under the pointer instead of the hover state
      if (props.tool === 'connector' && props.onConnectorCreate) { changeHover(null); followToolSnap(screenPoint(event)); return }
      if (!policy.select || props.tool === 'hand') return
      // the pointer on an affordance keeps its Block hovered
      if ((event.target as Element | null)?.closest?.('.aiws-afford')) return
      // a Block's own buttons (table cells, links) keep it hovered; an editor, a field or a menu does not
      const typing = Boolean((event.target as Element | null)?.closest?.('[data-role="editor"], input, textarea, select, [contenteditable="true"], .aiws-near, .aiws-menu, .aiws-popover'))
      const p = screenPoint(event)
      const w = camera.toWorld(p.x, p.y)
      const hit = typing ? null : hitAt(w.x, w.y)
      changeHover(hit?.id ?? null)
      return
    }
    trackPointer(event.clientX, event.clientY)
    followEdge(event.clientX, event.clientY)
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
        if (current.affected.length + current.lines.length > GHOST_DRAG_LIMIT) {
          if (!current.ghost) { current.ghost = true; for (const gid of [...current.affected, ...current.lines]) { const el = frameEl(gid); if (el) el.style.visibility = 'hidden' } }
        } else {
          setTransform(current.affected, dx, dy)
          // lines ending on moved Blocks follow them (§9.5)
          const moved = current.shifted
          const shift = (id: string) => { const l = laid.get(id); return l ? { rect: { ...l.rect, x: l.rect.x + dx, y: l.rect.y + dy }, rotation: l.rotation } : undefined }
          rerouteLines(current.lines, (id) => (moved.has(id) ? shift(id) : undefined), (id) => (moved.has(id) ? { x: dx, y: dy } : undefined))
        }
        // the selection outlines follow in the DOM: no React work per frame (§9.3)
        selectionLayerRef.current?.setAttribute('transform', `translate(${dx * camera.zoom}, ${dy * camera.zoom})`)
        return
      }
      if (current.kind === 'resize') {
        const delta = { x: (point.x - current.startX) / camera.zoom, y: (point.y - current.startY) / camera.zoom }
        // corner handles of an image or note keep the proportions; Shift switches the rule either way
        const aspect = (current.aspect === 'locked') !== shiftHeld.current
        const r = resizeRotated(current.start, current.rotation, current.handle, delta, { min: MIN_SIZE, aspect })
        const { x, y, w, h } = { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.w), h: Math.round(r.h) }
        current.current = { x, y, w, h }
        const el = frames.current.get(current.id)
        if (el) { el.style.left = `${x}px`; el.style.top = `${y}px`; el.style.width = `${w}px`; el.style.height = `${h}px` }
        const resized = current.current
        rerouteLines(current.lines, (id) => (id === current.id ? { rect: resized, rotation: current.rotation } : undefined))
        setResizePreview({ id: current.id, rect: current.current })
        const p = screenPoint({ clientX: point.x, clientY: point.y })
        setHint({ x: p.x, y: p.y, text: `${w} × ${h}` })
        return
      }
      if (current.kind === 'rotate') {
        const p = screenPoint({ clientX: point.x, clientY: point.y })
        const w = camera.toWorld(p.x, p.y)
        let deg = current.start + angleFrom(current.center, w) - current.startAngle
        if (shiftHeld.current) deg = Math.round(deg / ROTATE_SNAP) * ROTATE_SNAP
        current.current = normalizeRotation(deg)
        const el = frames.current.get(current.id)
        if (el) el.style.rotate = current.current ? `${current.current}deg` : ''
        const turnedTo = current.current
        rerouteLines(current.lines, (id) => { const l = id === current.id ? laid.get(id) : undefined; return l ? { rect: l.rect, rotation: turnedTo } : undefined })
        setRotatePreview({ id: current.id, rotation: current.current })
        setHint({ x: p.x, y: p.y, text: `${Math.round(current.current)}°` })
        return
      }
      if (current.kind === 'marquee') {
        const p = screenPoint({ clientX: point.x, clientY: point.y })
        const rect = { x: Math.min(current.startX, p.x), y: Math.min(current.startY, p.y), w: Math.abs(p.x - current.startX), h: Math.abs(p.y - current.startY) }
        if (rect.w > 2 || rect.h > 2) current.moved = true
        current.current = rect
        setMarquee(rect)
        return
      }
      if (!current.moved && Math.hypot(point.x - current.startX, point.y - current.startY) < 3) return
      current.moved = true
      const p = screenPoint({ clientX: point.x, clientY: point.y })
      const w = camera.toWorld(p.x, p.y)
      if (current.kind === 'connect') {
        // the end attaches to what is under the pointer; a loop on one Block routes as an elbow
        const fromTarget = current.loose && current.from.binding ? targetOf(current.from.binding.entity_id) : null
        const snap = snapAt(p, fromTarget ? centerOf(fromTarget.rect) : current.from.point)
        current.end = snap ? { binding: { entity_id: snap.id, anchor: snap.anchor }, point: snap.point } : { binding: null, point: w }
        current.endExact = snap?.exact ?? false
        // a start pressed inside its Block faces the end
        const facing = fromTarget && facingAnchor(fromTarget, current.end.point)
        if (fromTarget && facing && current.from.binding) current.from = { binding: { entity_id: current.from.binding.entity_id, anchor: facing }, point: anchorWorld(fromTarget, facing, current.end.point) }
        current.route = snap && snap.id === current.from.binding?.entity_id ? 'elbow' : 'straight'
        current.screen = p
        const g = routeConnector({ start: current.from.binding, end: current.end.binding, flip: {}, route: current.route, controls: [], label: { t: 0.5, offset: 0 } }, [current.from.point, current.end.point], lookupWith(), routeOptions(null))
        setLinePreview({ id: null, geom: g, snap })
        return
      }
      const l = laid.get(current.id)
      const line = l?.connector
      if (!l || !line) return
      if (current.kind === 'endpoint') {
        const other = current.which === 'start' ? line.geom.end : line.geom.start
        const otherBinding = current.which === 'start' ? line.data.end : line.data.start
        // binding needs `update` on the line; without it the end only moves
        const snap = lineEditable(current.id) ? snapAt(p, other.point, current.id) : null
        current.end = snap ? { binding: { entity_id: snap.id, anchor: snap.anchor }, point: snap.point } : { binding: null, point: w }
        current.endExact = snap?.exact ?? false
        current.route = snap && otherBinding?.entity_id === snap.id && line.data.route === 'straight' ? 'elbow' : null
        const data: ConnectorData = { ...line.data, [current.which]: current.end.binding, ...(current.route ? { route: current.route } : {}) }
        const stored: [Point, Point] = current.which === 'start' ? [current.end.point, other.point] : [other.point, current.end.point]
        const g = routeConnector(data, stored, lookupWith(), routeOptions(current.id))
        repaint(current.id, g)
        setLinePreview({ id: current.id, geom: g, snap })
        return
      }
      if (current.kind === 'bend') {
        const s0 = line.geom.start.point
        const e0 = line.geom.end.point
        let working: Point[]
        if (current.mode === 'move') {
          const origin = current.base[current.index]
          let next = { x: origin.x + w.x - current.press.x, y: origin.y + w.y - current.press.y }
          if (shiftHeld.current) {
            // Shift: the piece to the previous point becomes horizontal or vertical
            const prev = current.index === 0 ? s0 : current.base[current.index - 1]
            next = Math.abs(next.x - prev.x) < Math.abs(next.y - prev.y) ? { x: prev.x, y: next.y } : { x: next.x, y: prev.y }
          }
          working = current.base.map((pt, i) => (i === current.index ? next : pt))
        } else {
          working = dragSegment(current.base, current.index, w.x - current.press.x, w.y - current.press.y)
        }
        current.working = working
        const g = routeBetween({ ...line.data, controls: working.map((pt) => decompose(pt, s0, e0)) }, line.geom.start, line.geom.end, routeOptions(current.id))
        repaint(current.id, g)
        setLinePreview({ id: current.id, geom: g, snap: null })
        return
      }
      if (current.kind === 'label') {
        const near = nearest(line.geom, w)
        const n = leftOf(near.tangent)
        let offset = (w.x - near.point.x) * n.x + (w.y - near.point.y) * n.y
        // close to the path it snaps back onto it
        if (Math.abs(offset) * camera.zoom <= 6) offset = 0
        current.label = { t: near.t, offset }
        const g = routeBetween({ ...line.data, label: current.label }, line.geom.start, line.geom.end, routeOptions(current.id))
        repaint(current.id, g)
        setLinePreview({ id: current.id, geom: g, snap: null })
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
      shiftHeld.current = event.shiftKey
      if (event.code === 'Space' && !insideEditor(event.target)) { spaceHeld.current = true }
      if (event.key === 'Escape' && dragRef.current) { event.preventDefault(); finishDrag(true) }
    }
    const up = (event: KeyboardEvent) => { shiftHeld.current = event.shiftKey; if (event.code === 'Space') spaceHeld.current = false }
    window.addEventListener('keydown', down)
    window.addEventListener('keyup', up)
    return () => { window.removeEventListener('keydown', down); window.removeEventListener('keyup', up) }
  }, [finishDrag])


  /** The context menu of the Block at `p` (selecting it first) or of the blank spot. */
  const openMenuAt = (p: Point) => {
    const w = camera.toWorld(p.x, p.y)
    const hit = policy.select ? hitAt(w.x, w.y) : null
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
    const hit = policy.select && props.tool !== 'hand' ? hitAt(w.x, w.y) : null
    // edit mode: a drag that starts on a Block moves it, as with the mouse (a locked one is only selected; a line
    // is only selected — touch edits of lines come with touch editing, §6.5)
    if (hit && !tappable && policy.layout && canLayout && !laid.get(hit.id)?.locked && !laid.get(hit.id)?.connector) { beginPointer(event); return { kind: 'delegate' } }
    // the selected Block's own scrollable content scrolls under the finger
    const scroller = hit && selection.has(hit.id) ? scrollerWithin(target, frames.current.get(hit.id)) : null
    return scroller ? { kind: 'scroll', element: scroller } : { kind: 'pan' }
  }
  const tapAt = (p: Point, target: Element | null) => {
    if (props.tool === 'hand' || (target?.closest(TAPPABLE) && target.closest('.aiws-frame-block'))) return // a control in a Block took the tap
    const w = camera.toWorld(p.x, p.y)
    const hit = policy.select ? hitAt(w.x, w.y) : null
    if (editing && editing !== hit?.id) endEditing()
    if (hit) props.onSelectionChange(new Set([hit.id]))
    else if (selection.size > 0) props.onSelectionChange(new Set())
  }
  const doubleTapAt = (p: Point) => {
    const w = camera.toWorld(p.x, p.y)
    const hit = policy.select && props.tool !== 'hand' ? hitAt(w.x, w.y) : null
    const l = hit ? laid.get(hit.id) : undefined
    if (hit && l && !l.isGroup && !l.connector && policy.editContent) { props.onEditingChange(hit.id); props.onSelectionChange(new Set([hit.id])); return }
    camera.animateTo(l ? camera.fitted(l.bounds, 24) : camera.zoomedAt(p.x, p.y, 2))
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

  /** Double-clicking the rotation handle turns the Block back upright. */
  const resetRotation = (id: string) => {
    const l = laid.get(id)
    if (l && l.rotation && !l.locked && policy.layout && canLayout) props.onCommitLayout([{ id, placement: relativeTo(laid, l.parentId, l.rect, 0) }])
  }
  /** When a handle was last pressed: the root's double-click (retargeted there by pointer capture) is not an "edit". */
  const handleDownAt = useRef(0)
  const rotateDownAt = useRef(-1000)
  const beginResize = (event: ReactPointerEvent<Element>, id: string, handle: string) => {
    if (!policy.layout || !canLayout || dragRef.current) return
    const l = laid.get(id)
    if (!l || l.locked) return
    handleDownAt.current = event.timeStamp
    event.stopPropagation()
    event.preventDefault()
    rootRef.current?.setPointerCapture(event.pointerId)
    setDragging(true)
    shiftHeld.current = event.shiftKey
    dragRef.current = { kind: 'resize', id, handle, startX: event.clientX, startY: event.clientY, start: { ...l.rect }, rotation: l.rotation, aspect: metaSink.get(id)?.aspect ?? 'free', current: { ...l.rect }, lines: adjacency.get(id) ?? [] }
    worldRef.current?.classList.add('is-moving')
  }
  const beginRotate = (event: ReactPointerEvent<Element>, id: string) => {
    if (!policy.layout || !canLayout || dragRef.current) return
    const l = laid.get(id)
    if (!l || l.locked || l.isGroup) return
    event.stopPropagation()
    event.preventDefault()
    // a second press soon after the first is the double-click that turns the Block upright
    const again = event.timeStamp - rotateDownAt.current < 400
    handleDownAt.current = event.timeStamp
    rotateDownAt.current = again ? -1000 : event.timeStamp
    if (again) { resetRotation(id); return }
    rootRef.current?.setPointerCapture(event.pointerId)
    setDragging(true)
    const p = screenPoint(event)
    const center = centerOf(l.rect)
    dragRef.current = { kind: 'rotate', id, center, startAngle: angleFrom(center, camera.toWorld(p.x, p.y)), start: l.rotation, current: l.rotation, lines: adjacency.get(id) ?? [] }
  }
  /** A hover affordance was pressed: its own `run`, or the Block action it names. */
  const runAffordance = (meta: BlockMeta, affordance: HoverAffordance) => {
    try {
      if (affordance.run) { affordance.run(meta.context); return }
      const action = meta.context.definition.actions?.find((candidate) => candidate.id === affordance.action)
      if (action) void Promise.resolve(action.run(meta.context, store)).catch((error: unknown) => store.notify('error', `动作失败：${describeError(error)}`))
    } catch (error) {
      store.notify('error', `动作失败：${describeError(error)}`)
    }
  }

  // ---- render
  useSyncExternalStore(store.outline.subscribe, store.outline.snapshot)
  // a definition registered later changes its frames' `data-chrome`
  useSyncExternalStore(blockRegistry.subscribe, blockRegistry.snapshot)
  const zoom = camera.settledZoom
  // lines depend on the zoom only through screen-sized details (1 px minimum width, markers): steps of 25 % re-render them
  const lineZoom = Math.pow(1.25, Math.round(Math.log(zoom) / Math.log(1.25)))
  const frameList: ReactNode[] = []
  for (const id of frameOrder) {
    const state = mounts.get(id)
    const l = laid.get(id)
    if (!l || !state) continue
    if (l.connector) {
      frameList.push(
        <ConnectorFrame key={id} id={id} geom={l.connector.geom} geomKey={l.connector.key} title={l.entity.title ?? null} zIndex={editing === id ? laid.size + 1 : l.paint + 1}
          hidden={state === 'hidden'} zoom={lineZoom} mode={mode} editing={editing === id} registry={lines} onDone={handlersFor(id).onDeactivate} />,
      )
      continue
    }
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
        data-rotation={l.rotation || undefined}
        data-locked={l.locked ? 'true' : undefined}
        data-view={l.entity.view_type ?? undefined}
        data-chrome={l.isGroup ? undefined : blockRegistry.get(l.entity.view_type ?? '', l.entity.view_version ?? undefined)?.chrome ?? 'clip'}
        style={{ left: l.rect.x, top: l.rect.y, width: l.rect.w, height: l.rect.h, rotate: l.rotation ? `${l.rotation}deg` : undefined, display: state === 'hidden' ? 'none' : undefined, zIndex: editing === id ? laid.size + 1 : l.paint + 1 }}
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

  // ---- overlay geometry (screen space): every state is drawn around the same (turned) rectangle
  const shapeFor = (id: string): { rect: Rect; rotation: number } | null => {
    const l = laid.get(id)
    if (!l) return null
    // a line is shown by its path; its box is the path's bounds
    if (l.connector) return { rect: camera.rectToScreen(linePreview?.id === id ? linePreview.geom.bounds : l.bounds), rotation: 0 }
    const rect = camera.rectToScreen(resizePreview?.id === id ? resizePreview.rect : l.rect)
    return { rect, rotation: rotatePreview?.id === id ? rotatePreview.rotation : l.rotation }
  }
  const turned = (rect: Rect, rotation: number) => (rotation ? `rotate(${rotation} ${rect.x + rect.w / 2} ${rect.y + rect.h / 2})` : undefined)
  const selected = [...selection].flatMap((id) => { const s = shapeFor(id); return s ? [{ id, ...s, locked: laid.get(id)?.locked ?? false, line: Boolean(laid.get(id)?.connector) }] : [] })
  const screenBounds = (s: { rect: Rect; rotation: number }): Rect => {
    if (!s.rotation) return s.rect
    const pts = corners(s.rect, s.rotation)
    const x = Math.min(...pts.map((p) => p.x)), y = Math.min(...pts.map((p) => p.y))
    return { x, y, w: Math.max(...pts.map((p) => p.x)) - x, h: Math.max(...pts.map((p) => p.y)) - y }
  }
  const bbox = selected.length > 0 ? selected.map(screenBounds).reduce((acc, r) => ({ x: Math.min(acc.x, r.x), y: Math.min(acc.y, r.y), x2: Math.max(acc.x2, r.x + r.w), y2: Math.max(acc.y2, r.y + r.h) }), { x: Infinity, y: Infinity, x2: -Infinity, y2: -Infinity }) : null
  const hoverLaid = hover && !selection.has(hover) ? laid.get(hover) : undefined
  const hoverShape = hoverLaid ? shapeFor(hoverLaid.entity.entity_id) : null
  const hoverMeta = hover ? metaSink.get(hover) : undefined
  const single = selected.length === 1 && policy.layout && canLayout && !selected[0].locked && !selected[0].line && editing !== selected[0].id && !dragging ? selected[0] : null
  const singleLaid = single ? laid.get(single.id) : undefined
  const handles = single ? (['nw', 'ne', 'se', 'sw'] as const).map((h, i) => ({ h, ...corners(single.rect, single.rotation)[i] })) : []
  const edges = single ? (['n', 'e', 's', 'w'] as const).map((h, i) => {
    const pts = corners(single.rect, single.rotation)
    return { h, a: pts[i], b: pts[(i + 1) % 4] }
  }) : []
  const rotateAt = single && singleLaid && !singleLaid.isGroup ? (() => {
    const sw = corners(single.rect, single.rotation)[3]
    return rotatePoint({ x: sw.x - sizes.rotateOffset * 0.72, y: sw.y + sizes.rotateOffset * 0.72 }, sw, single.rotation)
  })() : null
  // affordances (§4.2): the hovered Block after the delay; on touch, the selected Block (no hover there)
  const affordId = pointer === 'touch' ? (selected.length === 1 ? selected[0].id : null) : shownHover
  const affordMeta = affordId && affordId === (pointer === 'touch' ? affordId : hover) && editing !== affordId && !dragging && !props.placing ? metaSink.get(affordId) : undefined
  const affordShape = affordMeta && affordId ? shapeFor(affordId) : null
  const affordances = (() => {
    if (!affordMeta || !affordShape) return []
    const box = screenBounds(affordShape)
    const buttons = Math.min(box.w, box.h) >= AFFORD_BUTTON_MIN
    const shown = affordMeta.affordances.filter((a) => (a.modes ?? ['edit']).includes(mode) && (a.kind === 'label' || buttons))
    const labels = shown.filter((a) => a.kind === 'label').slice(0, 1)
    const actions = shown.filter((a) => a.kind === 'button').slice(0, 2)
    if (import.meta.env.DEV && shown.length > labels.length + actions.length) console.warn('[aiworkspace] at most one label and two buttons per Block', affordMeta.context.definition.type)
    return [...labels, ...actions].map((a) => {
      const at = a.at === 'top-left-out' ? { left: box.x, top: box.y - 6 } : a.at === 'top-right-in' ? { left: box.x + box.w - 6, top: box.y + 6 }
        : a.at === 'bottom-out' ? { left: box.x + box.w / 2, top: box.y + box.h + 6 } : { left: box.x + box.w - 6, top: box.y + box.h - 6 }
      return { a, at }
    })
  })()
  const lockBadge = selected.length === 1 && selected[0].locked ? screenBounds(selected[0]) : null

  // ---- connectors in the overlay: halos, handles, connection handles, the end's snap target, a new line's preview
  const toScreenPt = (p: Point) => camera.toScreen(p.x, p.y)
  const screenSegs = (segs: Seg[]): Seg[] => segs.map((sg) => ({ a: toScreenPt(sg.a), b: toScreenPt(sg.b), ...(sg.c1 && sg.c2 ? { c1: toScreenPt(sg.c1), c2: toScreenPt(sg.c2) } : {}) }))
  const lineGeom = (id: string) => (linePreview?.id === id ? linePreview.geom : laid.get(id)?.connector?.geom)
  const linePath = (id: string) => { const g = lineGeom(id); return g ? pathData(screenSegs(g.segs)) : '' }
  const haloWidth = (id: string, extra: number) => lines.halfWidth(id) * 2 * camera.zoom + extra
  // a Block move re-routes lines without React: their halos would lag, so they hide until it ends
  const showHalos = !dragging || linePreview !== null
  const lineId = selected.length === 1 && selected[0].line && mode === 'edit' && policy.layout && canLayout && !selected[0].locked && editing !== selected[0].id && !dragging ? selected[0].id : null
  const lineG = lineId ? lineGeom(lineId) : undefined
  const canBend = lineId ? lineEditable(lineId) : false
  const lineEnds = lineG && lineId ? (['start', 'end'] as const).map((which) => ({ which, end: lineG[which], at: toScreenPt(lineG[which].point), binding: laid.get(lineId)?.connector?.data[which] ?? null })) : []
  /** Which anchor of which Block a bound end is on, in words. */
  const endPlace = (binding: { entity_id: string; anchor: Anchor } | null): string | null => {
    if (!binding) return null
    const name = store.outline.get(binding.entity_id)?.title || '对象'
    const id = binding.anchor.id
    const target = targetOf(binding.entity_id)
    return `连接在「${name}」的${anchorLabel(target ? findAnchor(anchorsOf(target), id) : undefined, id)}锚点`
  }
  const lineControls = lineG && canBend && lineG.route !== 'elbow' && !lineG.auto ? lineG.pts.slice(1, -1).map(toScreenPt) : []
  const lineMids = lineG && canBend ? (lineG.route === 'curve'
    ? lineG.segs.map((sg) => ({ at: toScreenPt(cubicAt(sg, 0.5)), long: dist(toScreenPt(sg.a), toScreenPt(sg.b)) >= 24 }))
    : lineG.pts.slice(0, -1).map((a, i) => ({ at: toScreenPt({ x: (a.x + lineG.pts[i + 1].x) / 2, y: (a.y + lineG.pts[i + 1].y) / 2 }), long: dist(a, lineG.pts[i + 1]) * camera.zoom >= 24 }))) : []
  const lineLabel = lineG && canBend && lineG.label.box ? camera.rectToScreen(lineG.label.box) : null
  // the label's handle wins over a bend handle under it (a bend is still added by dragging the line elsewhere)
  const underLabel = (p: Point) => Boolean(lineLabel && p.x >= lineLabel.x - 6 && p.x <= lineLabel.x + lineLabel.w + 6 && p.y >= lineLabel.y - 6 && p.y <= lineLabel.y + lineLabel.h + 6)
  // connection handles: just outside the anchor each side starts from — n, e, s, w unless the Block declares its
  // own (mouse only, §4.3)
  const connectHandles = single && singleLaid && !singleLaid.isGroup && pointer === 'mouse' && props.onConnectorCreate && props.tool !== 'connector' ? (() => {
    const target = targetOf(single.id)
    if (!target) return []
    return (['n', 'e', 's', 'w'] as const).flatMap((side) => {
      const spot = sideAnchor(target, side)
      if (!spot) return []
      const at = toScreenPt(spot.point)
      const out = rotatePoint(SIDE_VEC[side], { x: 0, y: 0 }, target.rotation)
      return [{ side, anchor: spot.anchor, x: at.x + out.x * CONNECT_OFFSET, y: at.y + out.y * CONNECT_OFFSET }]
    })
  })() : []
  // the anchors of the Block a dragged end (or the connector tool) is over; the chosen one stands out
  const snap = linePreview?.snap ?? (props.tool === 'connector' && !dragging ? toolSnap : null)
  const snapShape = snap ? { rect: camera.rectToScreen(snap.target.rect), rotation: snap.target.rotation, ellipse: snap.target.shape === 'ellipse' } : null
  const snapSpots = snap ? visibleAnchors(snap.target, camera.zoom) : []
  const snapOn = snap && (snap.exact || linePreview) ? snap.anchor.id : null

  /** What the near toolbar keeps clear of: the selection and, on a turned Block, its rotation handle (and the connection handles). */
  const nearBox = (b: { x: number; y: number; x2: number; y2: number }): Rect => {
    let { x, y, x2, y2 } = b
    const keep = (px: number, py: number, r: number) => { x = Math.min(x, px - r); y = Math.min(y, py - r); x2 = Math.max(x2, px + r); y2 = Math.max(y2, py + r) }
    if (rotateAt) keep(rotateAt.x, rotateAt.y, sizes.rotate / 2 + 2)
    for (const h of connectHandles) keep(h.x, h.y, CONNECT_DOT + 4)
    return { x, y, w: x2 - x, h: y2 - y }
  }
  const cutShapes = props.cutIds ? [...props.cutIds].flatMap((id) => { const s = shapeFor(id); return s ? [{ id, ...s, line: Boolean(laid.get(id)?.connector) }] : [] }) : []
  return (
    <div
      ref={rootRef}
      className={`aiws-canvas aiws-canvas-mode-${mode}${props.tool === 'hand' ? ' is-hand' : ''}${props.placing ? ' is-placing' : ''}${hover && laid.get(hover)?.connector && !dragging && policy.select ? ' is-line-hover' : ''}`}
      data-testid="aiws-canvas"
      data-surface-id={props.surfaceId}
      data-mode={mode}
      data-tool={props.placing ? 'place' : props.tool ?? 'select'}
      data-zoom={zoom.toFixed(2)}
      data-settled={settled}
      data-pointer={pointer}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={onPointerUp}
      onPointerCancel={(event) => { if (event.pointerType !== 'touch') finishDrag(true) }}
      onDragStart={(event) => { if (!insideEditor(event.target)) event.preventDefault() }}
      onWheel={onWheel}
      onContextMenu={onContextMenu}
      onPointerLeave={() => {
        if (!dragRef.current) { changeHover(null); setToolSnap(null) }
        if (ghostRef.current) ghostRef.current.style.visibility = 'hidden'
      }}
      onDoubleClick={(event) => {
        // pointer capture retargets clicks to the root: the double-click is resolved geometrically here
        if (!policy.editContent || insideEditor(event.target) || (event.target as Element).closest('.aiws-overlay-html') || event.timeStamp - handleDownAt.current < 600) return
        const p = screenPoint(event)
        const w = camera.toWorld(p.x, p.y)
        const hit = hitAt(w.x, w.y)
        const l = hit ? laid.get(hit.id) : undefined
        if (!hit || !l || l.isGroup) return
        // a line's label is typed in place (标准对象的交互改进 §4.4)
        if (l.connector && !lineEditable(hit.id)) return
        // rich text opens with the caret where the double-click was (§4.4)
        if (l.entity.view_type === 'richtext') requestIntent(hit.id, `caret:${event.clientX},${event.clientY}`)
        props.onEditingChange(hit.id)
        props.onSelectionChange(new Set([hit.id]))
      }}
    >
      <BlockMetaContext.Provider value={metaSink}>
        <div ref={worldRef} className="aiws-world" data-testid="aiws-world">{frameList}</div>
      </BlockMetaContext.Provider>
      <svg className="aiws-overlay" data-testid="aiws-overlay">
        {cutShapes.map(({ id, rect, rotation, line }) => line
          ? <path key={`cut:${id}`} className="aiws-cut-outline" data-testid={`aiws-cut-${id}`} d={linePath(id)} strokeWidth={haloWidth(id, 6)} />
          : <rect key={`cut:${id}`} className="aiws-cut-outline" data-testid={`aiws-cut-${id}`} x={rect.x} y={rect.y} width={rect.w} height={rect.h} transform={turned(rect, rotation)} />)}
        {hoverShape && hoverLaid?.connector && policy.select && !dragging && <path className="aiws-line-hover" data-testid="aiws-hover-outline" d={linePath(hoverLaid.entity.entity_id)} strokeWidth={haloWidth(hoverLaid.entity.entity_id, 4)} />}
        {showHalos && selected.filter((x) => x.line).map(({ id, locked }) => <path key={`halo:${id}`} className={`aiws-line-halo${locked ? ' is-locked' : ''}`} data-testid={`aiws-selection-${id}`} d={linePath(id)} strokeWidth={haloWidth(id, 6)} />)}
        {hoverShape && hoverLaid && !hoverLaid.connector && policy.select && !dragging && (hoverMeta?.shape === 'ellipse'
          ? <ellipse className="aiws-hover-outline" data-testid="aiws-hover-outline" cx={hoverShape.rect.x + hoverShape.rect.w / 2} cy={hoverShape.rect.y + hoverShape.rect.h / 2} rx={hoverShape.rect.w / 2} ry={hoverShape.rect.h / 2} transform={turned(hoverShape.rect, hoverShape.rotation)} />
          : <rect className={`aiws-hover-outline${hoverLaid.isGroup ? ' is-group' : ''}`} data-testid="aiws-hover-outline" x={hoverShape.rect.x} y={hoverShape.rect.y} width={hoverShape.rect.w} height={hoverShape.rect.h} transform={turned(hoverShape.rect, hoverShape.rotation)} />)}
        <g ref={selectionLayerRef}>
          {selected.filter((x) => !x.line).map(({ id, rect, rotation, locked }) => (
            <rect key={id} className={`aiws-selection-box${selected.length > 1 ? ' is-member' : ''}${locked ? ' is-locked' : ''}${laid.get(id)?.isGroup ? ' is-group' : ''}`} data-testid={`aiws-selection-${id}`} x={rect.x} y={rect.y} width={rect.w} height={rect.h} transform={turned(rect, rotation)} />
          ))}
          {selected.length > 1 && bbox && <rect className="aiws-selection-group" data-testid="aiws-multi-selection" x={bbox.x} y={bbox.y} width={bbox.x2 - bbox.x} height={bbox.y2 - bbox.y} />}
          {edges.map(({ h, a, b }) => (
            <line key={h} className="aiws-edge" data-testid={`aiws-edge-${h}`} x1={a.x} y1={a.y} x2={b.x} y2={b.y} strokeWidth={sizes.edge} style={{ cursor: resizeCursor(h, single?.rotation ?? 0) }} onPointerDown={(event) => beginResize(event, single!.id, h)} />
          ))}
          {handles.map(({ h, x, y }) => (
            <g key={h} className="aiws-handle-group" style={{ cursor: resizeCursor(h, single?.rotation ?? 0) }} onPointerDown={(event) => beginResize(event, single!.id, h)}>
              <circle className="aiws-handle-hit" cx={x} cy={y} r={sizes.cornerHit / 2} />
              <circle className="aiws-handle" data-testid={`aiws-handle-${h}`} cx={x} cy={y} r={sizes.corner / 2} />
            </g>
          ))}
        </g>
        {marquee && <rect className="aiws-marquee" data-testid="aiws-marquee" x={marquee.x} y={marquee.y} width={marquee.w} height={marquee.h} />}
        {snapShape && snap && (
          <g className="aiws-snap" data-testid={`aiws-snap-${snap.id}`} data-anchor={snap.anchor.id} data-exact={snap.exact ? 'true' : 'false'}>
            {snapShape.ellipse
              ? <ellipse className="aiws-snap-outline" cx={snapShape.rect.x + snapShape.rect.w / 2} cy={snapShape.rect.y + snapShape.rect.h / 2} rx={snapShape.rect.w / 2} ry={snapShape.rect.h / 2} transform={turned(snapShape.rect, snapShape.rotation)} />
              : <rect className="aiws-snap-outline" x={snapShape.rect.x} y={snapShape.rect.y} width={snapShape.rect.w} height={snapShape.rect.h} transform={turned(snapShape.rect, snapShape.rotation)} />}
            {snapSpots.map(({ def, point }) => {
              const at = toScreenPt(point)
              const on = def.id === snapOn
              return <circle key={def.id} className={`aiws-snap-point${def.minor ? ' is-minor' : ''}${on ? ' is-on' : ''}`} data-testid={`aiws-snap-anchor-${def.id}`} cx={at.x} cy={at.y} r={on ? 5 : def.minor ? 2.75 : 3.5} />
            })}
          </g>
        )}
        {linePreview && linePreview.id === null && <path className="aiws-line-preview" data-testid="aiws-line-preview" d={pathData(screenSegs(linePreview.geom.segs))} />}
        {lineId && lineG && (
          <g className="aiws-line-handles" data-testid={`aiws-line-handles-${lineId}`}>
            {lineLabel && <rect className="aiws-line-label-handle" data-testid="aiws-line-label-handle" x={lineLabel.x - 3} y={lineLabel.y - 2} width={lineLabel.w + 6} height={lineLabel.h + 4} rx={4} onPointerDown={(event) => beginLabelDrag(event, lineId)}><title>拖动标签；双击编辑</title></rect>}
            {lineMids.map(({ at, long }, i) => long && !underLabel(at) && (
              <g key={`mid:${i}`} className="aiws-line-handle aiws-line-mid" data-testid={`aiws-line-mid-${i}`} onPointerDown={(event) => beginLineHandle(event, lineId, i, true)}>
                <circle className="aiws-line-handle-hit" cx={at.x} cy={at.y} r={8} /><circle className="aiws-line-dot" cx={at.x} cy={at.y} r={4} />
                <title>{lineG.route === 'elbow' ? '拖动这一段' : '拖动添加折点'}</title>
              </g>
            ))}
            {lineControls.map((at, i) => (
              <g key={`ctl:${i}`} className="aiws-line-handle aiws-line-control" data-testid={`aiws-line-control-${i}`} onPointerDown={(event) => beginLineHandle(event, lineId, i, false)}>
                <circle className="aiws-line-handle-hit" cx={at.x} cy={at.y} r={8} /><circle className="aiws-line-dot" cx={at.x} cy={at.y} r={4} />
                <title>拖动折点（Shift 水平 / 垂直）；双击删除</title>
              </g>
            ))}
            {lineEnds.map(({ which, end, at, binding }) => (
              <g key={which} className={`aiws-line-handle aiws-line-end is-${end.state}`} data-testid={`aiws-line-end-${which}`} data-state={end.state} data-anchor={binding?.anchor.id} onPointerDown={(event) => beginLineEnd(event, lineId, which)}>
                <circle className="aiws-line-handle-hit" cx={at.x} cy={at.y} r={9} /><circle className="aiws-line-dot" cx={at.x} cy={at.y} r={5} />
                {end.state === 'broken' && <text className="aiws-line-bang" x={at.x} y={at.y}>!</text>}
                <title>{end.state === 'broken' ? '端点不可用：拖到对象上重新连接（双击查看说明）' : `${end.state === 'bound' ? `${endPlace(binding)}。` : ''}拖动重新连接；拖到空白处断开`}</title>
              </g>
            ))}
          </g>
        )}
        {connectHandles.map(({ side, anchor, x, y }) => (
          <g key={`connect:${side}`} className="aiws-connect-handle" data-testid={`aiws-connect-${side}`} onPointerDown={(event) => beginConnectFrom(event, single!.id, anchor)}>
            <circle className="aiws-connect-hit" cx={x} cy={y} r={CONNECT_DOT + 6} /><circle className="aiws-connect-dot" cx={x} cy={y} r={CONNECT_DOT} />
            <title>拖出连接线</title>
          </g>
        ))}
      </svg>
      <div className="aiws-overlay-html">
        {rotateAt && single && (
          <div className="aiws-rotate-handle" role="button" aria-label="旋转（Shift 吸附 15°，双击归零）" title="旋转（Shift 吸附 15°，双击归零）" data-testid="aiws-rotate-handle"
            style={{ left: rotateAt.x, top: rotateAt.y, width: sizes.rotate, height: sizes.rotate }}
            onPointerDown={(event) => beginRotate(event, single.id)}>
            <RotateCw size={pointer === 'touch' ? 18 : 14} aria-hidden="true" />
          </div>
        )}
        {lockBadge && <div className="aiws-lock-badge" data-testid="aiws-lock-badge" title="已锁定：不能移动、缩放或删除" style={{ left: lockBadge.x, top: lockBadge.y }}><Lock size={12} aria-hidden="true" /></div>}
        {affordMeta && affordances.map(({ a, at }) => a.kind === 'label' ? (
          <div key={a.id} className={`aiws-afford aiws-afford-label at-${a.at}`} data-testid={`aiws-afford-${a.id}`} style={at}>
            {a.icon && <a.icon size={12} aria-hidden="true" />}<span className="aiws-afford-text">{a.label}</span>
            {a.freshness && <FreshnessBadge entityId={a.freshness} showManual={false} />}
          </div>
        ) : (
          <button key={a.id} type="button" className={`aiws-afford aiws-afford-button at-${a.at}`} data-testid={`aiws-afford-${a.id}`} aria-label={a.label} title={a.label} style={at}
            onPointerDown={(event) => event.stopPropagation()} onClick={() => runAffordance(affordMeta, a)}>
            {a.icon ? <a.icon size={14} aria-hidden="true" /> : a.label}
          </button>
        ))}
        {hint && <div className="aiws-gesture-hint" data-testid="aiws-gesture-hint" style={{ left: hint.x + 14, top: hint.y + 14 }}>{hint.text}</div>}
      </div>
      {props.placing && <div ref={ghostRef} className="aiws-place-ghost" data-testid="aiws-place-ghost" style={{ visibility: 'hidden' }}><span>{props.placing.label}</span></div>}
      {bbox && props.renderNear && !resizePreview && !rotatePreview && !dragging && props.renderNear(nearBox(bbox))}
    </div>
  )
}
