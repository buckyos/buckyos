/* The wish lines of a free canvas (许愿格交互升级 TODO §3.9, M6): light dashed arrows from each input's Block into its wish
 * (blue) and from the wish to each Block of its last run's results (orange), and the ghosts of inputs that have no Block
 * here. They are a decoration the canvas works out from the wishes' data, not entities: nothing is stored, they cannot be
 * selected, they are not in the outline and no export reads them (draw.io export reads the BlockTree; print hides them).
 *
 * They are drawn in the RenderHost's world layer, under every Block (z-index 0; frames start at 1), so the camera's one
 * transform moves them; lines take no pointer, ghosts are buttons (the canvas leaves a press on a button alone) that open
 * their input. Only edit and view mode draw them; a flow page and a show's stage never mount this. Within a wish its Block
 * says whether to draw (`config.show_links: false` turns them off). Lines are routed and painted with the connectors' own
 * geometry and markup, culled to the view (with a margin, recomputed when the camera settles) and repainted in place while
 * a gesture moves their Blocks (live.ts). */

import { createElement, memo, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { createPortal } from 'react-dom'
import { ChevronUp, TriangleAlert } from 'lucide-react'
import type { CellPayload, KeyedContent, WishPayloadRead } from '../../../api/types'
import { useLoad, useStore, useVersion, useWorkspaceUi } from '../../../state/hooks'
import { useAiwsText } from '../../../useAiwsText'
import type { CanvasMode } from '../../blocks/registry'
import { iconOf, nameOf } from '../../datamode/model'
import type { TargetGeom } from '../connectors/geometry'
import { LINE_SIMPLIFIED_ZOOM } from '../connectors/registry'
import type { Laid } from '../layout'
import type { Camera, Rect } from '../render/camera'
import { FrameWatch, type FrameRead } from './live'
import { blocksBySource, GHOST_SIZE, ghostRect, isGhostKey, planWishLinks, type Ghost, type PlannedLink, type WishLinkPlan } from './plan'
import { intersects, paintLink, routeLink, type PaintedLink } from './route'
import './wishLinks.css'

/** Lines and ghosts are kept within this many viewports around the view. */
const VIEW_MARGIN = 0.5
const EMPTY: WishLinkPlan = { links: [], ghosts: [] }

interface View { zoom: number; rect: Rect }

/** Mounted right after the canvas's RenderHost: draws into its world layer. */
export function WishLinks({ mode, camera, laid }: { mode: CanvasMode; camera: Camera; laid: Map<string, Laid> }) {
  const [world, setWorld] = useState<HTMLElement | null>(null)
  const anchor = useCallback((el: HTMLSpanElement | null) => {
    setWorld(el?.previousElementSibling?.querySelector<HTMLElement>(':scope > .aiws-world') ?? null)
  }, [])
  return (
    <>
      <span ref={anchor} hidden />
      {(mode === 'edit' || mode === 'view') && world && createPortal(<Layer camera={camera} laid={laid} world={world} />, world)}
    </>
  )
}

function Layer({ camera, laid, world }: { camera: Camera; laid: Map<string, Laid>; world: HTMLElement }) {
  const [watch] = useState(() => new FrameWatch())
  useEffect(() => { watch.attach(world); return () => watch.dispose() }, [watch, world])
  const [view, setView] = useState<View>(() => ({ zoom: camera.zoom, rect: camera.visibleWorld(VIEW_MARGIN) }))
  useEffect(() => camera.onSettle(() => setView({ zoom: camera.zoom, rect: camera.visibleWorld(VIEW_MARGIN) })), [camera])
  const bySource = useMemo(() => blocksBySource(laid), [laid])
  const wishes = useMemo(() => [...laid.values()].filter((l) => l.entity.view_type === 'wish' && l.entity.source_id && !l.connector), [laid])
  return (
    <div className="aiws-wishlinks" data-testid="aiws-wishlinks" data-decoration="wish-links">
      {wishes.map((l) => <WishBlockLinks key={l.entity.entity_id} cellId={l.entity.entity_id} wishId={l.entity.source_id!} laid={laid} bySource={bySource} view={view} watch={watch} />)}
    </div>
  )
}

type RectOf = (id: string) => { rect: Rect; rotation: number } | undefined

/** Where a line's end is: a Block's (or group's) rect, or a ghost's square beside the wish. */
function endGeom(id: string, plan: WishLinkPlan, cellId: string, rectOf: RectOf): TargetGeom | null {
  if (isGhostKey(id)) {
    const ghost = plan.ghosts.find((g) => g.key === id)
    const wish = rectOf(cellId)
    return ghost && wish ? { rect: ghostRect(wish.rect, ghost.slot), rotation: 0, shape: 'rect' } : null
  }
  const r = rectOf(id)
  return r ? { rect: r.rect, rotation: r.rotation, shape: 'rect' } : null
}

function paintOne(link: PlannedLink, plan: WishLinkPlan, cellId: string, rectOf: RectOf, zoom: number): PaintedLink | null {
  const from = endGeom(link.from, plan, cellId, rectOf)
  const to = endGeom(link.to, plan, cellId, rectOf)
  return from && to ? paintLink(routeLink(from, to, link.paired), link.kind, zoom, `${cellId}-${link.key}`) : null
}

/** The far end of a line in its test id: the Block, `ghost-<entity>`, or `more`. */
function farEnd(link: PlannedLink): string {
  const far = link.kind === 'input' ? link.from : link.to
  return far.startsWith('@ghost:') ? `ghost-${far.slice(7)}` : far.startsWith('@') ? far.slice(1) : far
}

function place(el: SVGSVGElement, painted: PaintedLink) {
  const { box } = painted
  el.style.left = `${box.x}px`
  el.style.top = `${box.y}px`
  el.style.width = `${box.w}px`
  el.style.height = `${box.h}px`
  el.setAttribute('viewBox', `${box.x} ${box.y} ${box.w} ${box.h}`)
  el.innerHTML = painted.markup
}

interface WishProps { cellId: string; wishId: string; laid: Map<string, Laid>; bySource: Map<string, string[]>; view: View; watch: FrameWatch }

const WishBlockLinks = memo(function WishBlockLinks({ cellId, wishId, laid, bySource, view, watch }: WishProps) {
  const store = useStore()
  const wishVersion = useVersion(`e:${wishId}`)
  const cellVersion = useVersion(`e:${cellId}`)
  const loadWish = useCallback(() => store.readBatched<WishPayloadRead>(wishId), [store, wishId])
  const loadCell = useCallback(() => store.readBatched<KeyedContent<CellPayload>>(cellId), [store, cellId])
  const wish = useLoad(loadWish, wishVersion)
  const cell = useLoad(loadCell, cellVersion)
  const [expanded, setExpanded] = useState(false)
  const payload = wish.data?.entity_id === wishId ? wish.data.content.payload : undefined
  const cellPayload = cell.data?.entity_id === cellId ? cell.data.content.payload : undefined
  const shown = Boolean(payload && cellPayload && cellPayload.config?.show_links !== false)
  const plan = useMemo(() => (shown && payload
    ? planWishLinks({ cellId, inputs: payload.inputs ?? [], results: Object.values(payload.last_run?.result_bindings?.results ?? {}) }, laid, bySource, (id) => store.outline.get(id), expanded)
    : EMPTY), [shown, payload, cellId, laid, bySource, store, expanded])
  const painted = useMemo(() => {
    const rectOf: RectOf = (id) => laid.get(id)
    return plan.links.flatMap((link) => {
      const p = paintOne(link, plan, cellId, rectOf, view.zoom)
      return p && intersects(p.box, view.rect) ? [{ link, ...p }] : []
    })
  }, [plan, laid, cellId, view])

  // a gesture moving an end repaints in place (no React work per frame)
  const [svgs] = useState(() => new Map<string, SVGSVGElement>())
  const [ghostEls] = useState(() => new Map<string, HTMLElement>())
  const latest = useRef({ plan, laid, zoom: view.zoom })
  useLayoutEffect(() => { latest.current = { plan, laid, zoom: view.zoom } })
  useEffect(() => watch.subscribe((moved: ReadonlySet<string>, frame: FrameRead) => {
    const { plan: now, laid: layout, zoom } = latest.current
    const wishMoved = moved.has(cellId)
    if (!now.links.some((l) => wishMoved || moved.has(l.from) || moved.has(l.to))) return
    const rectOf: RectOf = (id) => frame(id) ?? layout.get(id)
    for (const link of now.links) {
      if (!wishMoved && !moved.has(link.from) && !moved.has(link.to)) continue
      const el = svgs.get(link.key)
      const p = el ? paintOne(link, now, cellId, rectOf, zoom) : null
      if (el && p) place(el, p)
    }
    const at = wishMoved ? rectOf(cellId) : undefined
    if (at) for (const g of now.ghosts) {
      const el = ghostEls.get(g.key)
      if (!el) continue
      const r = ghostRect(at.rect, g.slot)
      el.style.left = `${r.x}px`
      el.style.top = `${r.y}px`
    }
  }), [watch, cellId, svgs, ghostEls])

  const wishRect = laid.get(cellId)?.rect
  if (!shown || !wishRect) return null
  return (
    <div className="aiws-wishlinks-wish" data-testid={`aiws-wishlinks-${cellId}`} data-wish-id={wishId} data-links={plan.links.length} data-ghosts={plan.ghosts.length}>
      {painted.map(({ link, box, markup }) => (
        <svg key={link.key} ref={(el) => { if (el) svgs.set(link.key, el); else svgs.delete(link.key) }} className="aiws-wishlink" aria-hidden="true"
          data-testid={`aiws-wishlink-${cellId}-${link.kind}-${farEnd(link)}`} data-kind={link.kind} data-from={link.from} data-to={link.to}
          data-entity={link.entityId ?? undefined} data-paired={link.paired ? 'true' : undefined} data-count={link.count}
          style={{ left: box.x, top: box.y, width: box.w, height: box.h }} viewBox={`${box.x} ${box.y} ${box.w} ${box.h}`} dangerouslySetInnerHTML={{ __html: markup }} />
      ))}
      {plan.ghosts.map((g) => {
        const r = ghostRect(wishRect, g.slot)
        if (!intersects(r, view.rect)) return null
        return <GhostSquare key={g.key} ghost={g} cellId={cellId} rect={r} zoom={view.zoom} onToggle={setExpanded}
          register={(el) => { if (el) ghostEls.set(g.key, el); else ghostEls.delete(g.key) }} />
      })}
    </div>
  )
})

/** An input with no Block here: its type and name (a warning only, when it is gone or unreadable); "+N"; "fewer". */
function GhostSquare({ ghost, cellId, rect, zoom, onToggle, register }: { ghost: Ghost; cellId: string; rect: Rect; zoom: number; onToggle: (expanded: boolean) => void; register: (el: HTMLButtonElement | null) => void }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const { t, list } = useAiwsText()
  const detailed = zoom >= LINE_SIMPLIFIED_ZOOM
  const style = { left: rect.x, top: rect.y, width: GHOST_SIZE, height: GHOST_SIZE }
  const common = { ref: register, type: 'button' as const, className: 'aiws-wishlink-ghost', 'data-state': ghost.state, style }
  if (ghost.state === 'more') {
    const folded = ghost.folded ?? []
    const names = folded.flatMap((g) => (g.entity ? [nameOf(store, g.entity, g.entity.entity_id)] : []))
    const label = t('aiworkspace.wishlinks.ghost.more', { count: folded.length })
    return (
      <button {...common} data-testid={`aiws-wishlink-more-${cellId}`} data-count={folded.length} title={names.length ? `${label}\n${list(names)}` : label} aria-label={label} onClick={() => onToggle(true)}>
        <span className="aiws-wishlink-count">+{folded.length}</span>
      </button>
    )
  }
  if (ghost.state === 'less') {
    const label = t('aiworkspace.wishlinks.ghost.less')
    return <button {...common} data-testid={`aiws-wishlink-less-${cellId}`} title={label} aria-label={label} onClick={() => onToggle(false)}><ChevronUp size={18} aria-hidden="true" /></button>
  }
  const testId = `aiws-wishlink-ghost-${cellId}-${ghost.entityId}`
  if (ghost.state === 'unavailable' || !ghost.entity || !ghost.entityId) {
    const label = t('aiworkspace.wishlinks.ghost.unavailable')
    return (
      <button {...common} data-testid={testId} title={label} aria-label={label} onClick={() => store.notify('info', label)}>
        <TriangleAlert size={20} aria-hidden="true" />
      </button>
    )
  }
  const name = nameOf(store, ghost.entity, ghost.entityId)
  const entityId = ghost.entityId
  const label = t('aiworkspace.wishlinks.ghost.open', { name })
  return (
    <button {...common} data-testid={testId} data-type={ghost.entity.type_id} title={label} aria-label={label} onClick={() => ui.openEntity(entityId)}>
      {createElement(iconOf(ghost.entity), { size: 20, 'aria-hidden': true })}
      {detailed && <span className="aiws-wishlink-name">{name}</span>}
    </button>
  )
}
