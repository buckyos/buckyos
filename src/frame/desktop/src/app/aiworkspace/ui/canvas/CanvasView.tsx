/* CanvasView (phase two §5, §8, §10.2; UI improvement §3, §7–§9): the canvas fills the application
 * area and the tools float above it in screen space — the main toolbar top left, the presenter toolbar
 * (annotation, zoom and view navigation, identity, share) top right, the vertical object toolbar on the
 * left — with the right panel taking layout width (a drawer in narrow windows). This view owns the
 * selection, the pointer tool, one-shot placement, insertion, the object clipboard, grouping,
 * cross-Surface moves and the per-Surface viewport; the RenderHost owns rendering and gestures. Menus,
 * buttons, context menus and shortcuts call the same actions. A phone (§16) gets the canvas in view mode
 * with one toolbar (canvas switcher, annotation, zoom, identity, share) and touch gestures, and keeps a
 * viewport of its own. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { Group, Lock, LockOpen, MessageSquarePlus, Sparkles, Ungroup } from 'lucide-react'
import { describeError } from '../../api/session'
import { randomId } from '../../api/ids'
import type { EntityEnvelope, Json, Operation, Placement } from '../../api/types'
import { useDirectReadOnly, useOutlineVersion, useStore, useUserState, useWorkspaceUi } from '../../state/hooks'
import { BlockBoundary } from '../blocks/BlockHost'
import { useBlockContext } from '../blocks/useBlockContext'
import { BudgetContext, createBudget } from '../blocks/budget'
import { EditorToolbarContext, requestIntent, ToolbarSink } from '../blocks/editorToolbar'
import { blockRegistry, modePolicy, type CanvasMode, type RenderContext, type ToolbarItem } from '../blocks/registry'
import { PermissionsPanel } from '../sources/PermissionsPanel'
import { RelationsPanel } from '../sources/RelationsPanel'
import { AnnotationsPanel } from '../shell/AnnotationsPanel'
import type { CanvasCommands, Command } from '../shell/MainMenu'
import { MainToolbar } from '../shell/MainToolbar'
import { useOverlayOpen } from '../shell/popover'
import { PresenterToolbar } from '../shell/PresenterToolbar'
import { CANVAS_SIDE_TABS, useCanvasMode, useShell, type SideTab } from '../shell/shellContext'
import { SidePanel } from '../shell/SidePanel'
import { StatusDetail, StatusDock } from '../shell/StatusSummary'
import { OpenWishContext } from '../wish/wishBlock'
import { WishPanel } from '../wish/WishPanel'
import { registryEntries, type CatalogEntry, type InsertRequest } from './catalog'
import { canvasClipboard, copyToClipboard, pasteOperations } from './clipboard'
import { FlowSurface } from './FlowSurface'
import { InsertCatalog, type CatalogTab } from './InsertCatalog'
import { boundsOf, layoutSurface, movedPlacement, relativeTo, surfaceBounds, topLevel } from './layout'
import { ObjectToolbar, type PointerTool } from './ObjectToolbar'
import { Camera } from './render/camera'
import { RenderHost, type LayoutChange } from './render/RenderHost'
import { SpatialIndex } from './render/spatialIndex'
import { createSurfaceOps, surfacesOf } from './surfaceOps'
import { BlockInspector, ContextMenu, NearToolbar, type NearAction } from './tools'

export interface CanvasFocus { surfaceId: string; blockId: string | null; nonce: number }

interface Viewport { x: number; y: number; zoom: number }

/** Screen space the floating toolbars and the status area cover (fit, centring and the near toolbar avoid it);
 * narrow windows stack the two top toolbars (a phone has only one). */
const TOP_INSET = 68
const TOP_INSET_NARROW = 124
const LEFT_INSET = 68
const BOTTOM_INSET = 52

export function CanvasView({ focus }: { focus: CanvasFocus | null }) {
  const shell = useShell()
  const mode = useCanvasMode()
  const active = shell.activeSurface
  if (!active) return <EmptyCanvas mode={mode} />
  return <SurfaceView key={active.entity_id} surface={active} mode={mode} focus={focus} />
}

function EmptyCanvas({ mode }: { mode: CanvasMode }) {
  const store = useStore()
  const shell = useShell()
  const canStructure = store.session.info().capabilities.includes('structure') && !shell.phone
  const create = async (layout: 'free' | 'flow') => {
    const { ops, surfaceId } = createSurfaceOps(store, layout === 'free' ? '画布 1' : '页 1', layout)
    const outcome = await store.submit({ editId: 'surface:new', label: '创建画布', operations: ops })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') shell.selectSurface(surfaceId)
  }
  return (
    <div className="aiws-canvas-view" data-testid="aiws-canvas-view" data-mode={mode}>
      <div className="aiws-canvas-body">
        <div className="aiws-canvas-main is-empty">
          <div className="aiws-empty" data-testid="aiws-no-surface">
            <p>这个工作区还没有画布。</p>
            {canStructure ? (
              <div className="aiws-dialog-actions">
                <button type="button" className="is-primary" data-testid="aiws-create-first-surface" onClick={() => { void create('free') }}>新建自由画布</button>
                <button type="button" data-testid="aiws-create-first-flow" onClick={() => { void create('flow') }}>新建流式页</button>
              </div>
            ) : <p className="aiws-muted">{shell.phone ? '手机上只能查看；新建画布请在电脑上打开这个工作区。' : '你没有新建画布的权限；数据源中的数据仍可查看。'}</p>}
          </div>
          <div className="aiws-chrome-top">
            {shell.phone ? <PresenterToolbar camera={null} hasSelection={false} onFitAll={() => undefined} onFitSelection={() => undefined} annotate={null} /> : <MainToolbar canvas={null} />}
          </div>
          <StatusDock />
        </div>
        <SidePanel tabs={['collab', 'edits']} render={(tab) => (tab === 'collab' ? <PermissionsPanel /> : <StatusDetail />)} />
      </div>
    </div>
  )
}

type MenuState = { at: { x: number; y: number }; world: { x: number; y: number } | null; blockId: string | null; parentId?: string }

function SurfaceView({ surface, mode, focus }: { surface: EntityEnvelope; mode: CanvasMode; focus: CanvasFocus | null }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const shell = useShell()
  const outlineVersion = useOutlineVersion()
  const readOnlyNow = useDirectReadOnly()
  const overlayOpen = useOverlayOpen()
  const clip = useSyncExternalStore(canvasClipboard.subscribe, canvasClipboard.snapshot)
  const surfaceId = surface.entity_id
  const policy = modePolicy(mode)
  const [selection, setSelectionState] = useState<Set<string>>(new Set())
  const [editing, setEditingState] = useState<string | null>(null)
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [picker, setPicker] = useState<{ kind: 'add-view' | 'move-surface' } | null>(null)
  const [catalog, setCatalog] = useState<{ tab: CatalogTab; key?: string; world: { x: number; y: number } | null; parentId?: string } | null>(null)
  const [tool, setTool] = useState<PointerTool>('select')
  const [placing, setPlacing] = useState<InsertRequest | null>(null)
  const [annotatePick, setAnnotatePick] = useState(false)
  const [collapsedChoice, setCollapsed] = useState<boolean | null>(null)
  const collapsed = collapsedChoice ?? shell.size === 'narrow'
  const [camera] = useState(() => new Camera())
  const [budget] = useState(() => createBudget(surface.layout?.mode === 'free' ? undefined : { editors: 200, html: 8 }))
  const isFree = surface.layout?.mode === 'free'
  // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal of the outline model
  const laid = useMemo(() => layoutSurface(store.outline, surfaceId), [store, surfaceId, outlineVersion])
  const index = useMemo(() => {
    const idx = new SpatialIndex()
    for (const [id, l] of laid) idx.insert({ id, rect: l.bounds, paint: l.paint, ...(l.rotation ? { turned: { rect: l.rect, rotation: l.rotation } } : {}) })
    return idx
  }, [laid])
  // the Editor of the Block being edited hands its tools to the near toolbar (标准对象的交互改进 §5.4)
  const [toolbarSink] = useState(() => new ToolbarSink())
  useSyncExternalStore(toolbarSink.subscribe, toolbarSink.snapshot)
  /** The wish whose flow is open in the right panel (R6): its Block stays a card. */
  const [wishOpen, setWishOpen] = useState<{ wishId: string; cellId: string } | null>(null)
  const phone = shell.phone
  const showObjectToolbar = shell.prefs.objectToolbar && mode !== 'presentation_edit' && !phone
  const topInset = shell.size === 'narrow' && !phone ? TOP_INSET_NARROW : TOP_INSET
  const leftInset = showObjectToolbar && !collapsed ? LEFT_INSET : 12
  const insets = { top: topInset, right: 12, bottom: BOTTOM_INSET, left: leftInset }
  useEffect(() => { camera.setInsets({ top: topInset, right: 12, bottom: BOTTOM_INSET, left: leftInset }) }, [camera, topInset, leftInset])

  const canComment = store.session.info().capabilities.includes('comment')
  /** One block picked while "add annotation" waits for a target. */
  const pickForAnnotation = (next: Set<string>) => {
    if (!annotatePick || next.size !== 1) return
    const picked = store.outline.get([...next][0])
    const source = picked?.source_id ? store.outline.get(picked.source_id) : undefined
    if (!source) { store.notify('info', '这个对象没有数据，暂不支持批注：请点选表格、富文本、记录等数据对象。'); return }
    setAnnotatePick(false)
    ui.annotate?.({ target: { entity_id: source.entity_id }, label: source.title ?? source.name ?? source.entity_id })
  }
  // selection survives mode switches when the policy still allows it (§10.2); editors are released
  const setSelection = (next: Set<string>) => { setSelectionState(next); pickForAnnotation(next) }
  // activating an editor brings its Block into view (§8.2: the active Block is never half off-screen);
  // on a free canvas a wish opens in the right panel instead (its Block stays a card)
  const { setSide } = shell
  const setEditing = useCallback((id: string | null) => {
    const l = id ? laid.get(id) : undefined
    // a Block inserted a moment ago may not be laid out yet: the outline knows what it is
    const entity = l?.entity ?? (id ? store.outline.get(id) : undefined)
    if (id && isFree && entity?.view_type === 'wish' && entity.source_id) {
      setWishOpen({ wishId: entity.source_id, cellId: id })
      setSelectionState(new Set([id]))
      setSide('wish')
      return
    }
    setEditingState(id)
    // editing is "selected + typing" (§4.4): an editor opened from an affordance selects its Block too
    if (id) setSelectionState((current) => (current.size === 1 && current.has(id) ? current : new Set([id])))
    if (l) camera.ensureVisible(l.bounds)
  }, [laid, camera, isFree, setSide, store])
  const policyKey = `${policy.select}:${policy.editContent}:${policy.insert}`
  const [policySeen, setPolicySeen] = useState(policyKey)
  if (policySeen !== policyKey) {
    setPolicySeen(policyKey)
    if (!policy.select) setSelectionState(new Set())
    if (!policy.editContent) setEditingState(null)
    if (!policy.insert) { setPlacing(null); setCatalog(null) }
    if (!policy.annotate) setAnnotatePick(false)
  }
  const [laidSeen, setLaidSeen] = useState(laid)
  if (laidSeen !== laid) {
    setLaidSeen(laid)
    const kept = [...selection].filter((id) => laid.has(id))
    if (kept.length !== selection.size) setSelectionState(new Set(kept))
    if (editing && !laid.has(editing)) setEditingState(null)
  }
  const [blockFocusSeen, setBlockFocusSeen] = useState(0)
  if (focus && focus.surfaceId === surfaceId && focus.nonce !== blockFocusSeen) {
    setBlockFocusSeen(focus.nonce)
    if (focus.blockId && laid.has(focus.blockId)) setSelectionState(new Set([focus.blockId]))
  }
  // per-Surface viewport in the user work state (§4.4); a first visit puts the world origin at the top left of the clear area.
  // A phone keeps its own viewport (a phone's zoom would be odd on the desktop and back) and first sees the whole canvas.
  const viewportKey = `${phone ? 'phone-viewport' : 'viewport'}:${surfaceId}`
  const savedViewport = useUserState<Json>(viewportKey) as Viewport | undefined
  const restored = useRef(false)
  useEffect(() => {
    if (restored.current) return
    const bounds = phone && !savedViewport ? surfaceBounds(laid) : null
    if (bounds) { camera.fit(bounds); if (camera.zoom > 1) camera.zoomTo(1) } else camera.set(savedViewport ?? { x: -camera.clearArea.x, y: -camera.clearArea.y, zoom: 1 })
    restored.current = true
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [savedViewport])
  useEffect(() => camera.onSettle(() => { if (restored.current) store.userState.set(viewportKey, { x: Math.round(camera.x), y: Math.round(camera.y), zoom: Number(camera.zoom.toFixed(3)) }) }), [camera, store, viewportKey])
  // the task location of a wish run (许愿格 §5.2): this Surface, the last selection that was not only a wish, the viewport
  useEffect(() => {
    const ids = [...selection]
    store.setCanvasFocus(ids.length > 0 && !ids.every((id) => laid.get(id)?.entity.view_type === 'wish') ? { surfaceId, selection: ids } : { surfaceId })
  }, [store, surfaceId, selection, laid])
  useEffect(() => camera.onSettle(() => {
    const r = camera.visibleRect
    store.setCanvasFocus({ viewport: { x: Math.round(r.x), y: Math.round(r.y), w: Math.round(r.w), h: Math.round(r.h) } })
  }), [camera, store])
  useEffect(() => {
    if (!focus || focus.surfaceId !== surfaceId || !focus.blockId) return
    const rect = laid.get(focus.blockId)?.bounds
    if (rect) camera.fit({ x: rect.x - 80, y: rect.y - 80, w: rect.w + 160, h: rect.h + 160 })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the camera moves once per focus request, not per layout change
  }, [focus?.nonce, surfaceId, camera])

  const [viewportSize, setViewportSize] = useState({ w: 800, h: 600 })
  const hostRef = useRef<HTMLDivElement>(null)
  const mainRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = mainRef.current
    if (!el) return
    // the observer reports once on observe, which is the initial measurement
    const observer = new ResizeObserver(() => setViewportSize({ w: el.clientWidth, h: el.clientHeight }))
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  // ---- layout commits: one gesture, one commit (§8.2)
  const commitLayout = useCallback((changes: LayoutChange[]) => {
    if (!policy.layout) return
    const ops: Operation[] = changes.map((change) => {
      store.noteLayoutIntent(change.id, change.placement, undefined, change.parentId)
      return change.parentId
        ? { op: 'tree.move', entity_id: change.id, new_parent_id: change.parentId, order_key: laid.get(change.id)?.entity.order_key ?? store.core.order_key_between(undefined, undefined), placement: change.placement }
        : { op: 'tree.place', entity_id: change.id, placement: change.placement }
    })
    // optimistic: the frames already moved; the stream confirms (or another client's later move overrides, D1)
    for (const change of changes) store.outline.patchLocal(change.id, { placement: change.placement, ...(change.parentId ? { parent_id: change.parentId } : {}) })
    const label = changes.length === 1 ? `移动 ${laid.get(changes[0].id)?.entity.title ?? changes[0].id}` : `移动 ${changes.length} 个 Block`
    void store.submit({ editId: `layout:${surfaceId}:${randomId().slice(0, 8)}`, label, operations: ops })
  }, [store, laid, policy.layout, surfaceId])

  const canLayout = policy.layout && surface.capabilities.includes('structure') && !readOnlyNow
  const insertReason = !policy.insert ? (mode === 'view' ? '查看模式不能插入' : '播放编辑占位中不能插入')
    : !surface.capabilities.includes('structure') ? '没有在这张画布上添加对象的权限'
      : readOnlyNow ? '后台不可达且此窗口未启用离线：当前只读' : null

  /** World point at the centre of the unobstructed visible area (top-left of a Block of `size` centred there). */
  const centreSpot = (size: { w: number; h: number }) => {
    const c = camera.center
    const w = camera.toWorld(c.x, c.y)
    const spot = { x: Math.round(w.x - size.w / 2), y: Math.round(w.y - size.h / 2) }
    while ([...laid.values()].some((l) => !l.isGroup && Math.abs(l.rect.x - spot.x) < 4 && Math.abs(l.rect.y - spot.y) < 4)) { spot.x += 24; spot.y += 24 }
    return spot
  }

  // ---- insertion (§4.2, §8.1; UI improvement §7.2): no title prompt; text and notes open their editor at once
  const insert = useCallback(async (request: InsertRequest, world: { x: number; y: number } | null, parentId?: string) => {
    const definition = request.entry.definition
    if (!definition.create || insertReason) return
    let config = request.config
    if (request.file) {
      try {
        const uploaded = await store.session.uploadAsset(request.file, request.file.name)
        config = { ...(config ?? {}), file: { object_id: uploaded.object_id, file_name: request.file.name } }
      } catch (error) {
        store.notify('error', `上传“${request.file.name}”失败：${describeError(error)}。没有插入任何对象。`)
        return
      }
    }
    const contentFolderId = surface.content_folder_id ?? ''
    const parent = parentId ?? surfaceId
    const size = definition.defaultSize
    const spot = world ?? centreSpot(size)
    const placement: Placement = isFree ? relativeTo(laid, parent, { ...spot, ...size }) : { x: 0, y: 0, ...size }
    const cellId = randomId('c')
    const dataId = randomId('d')
    const orderKey = store.core.order_key_between(store.outline.childrenOf(parent).at(-1)?.order_key ?? undefined, undefined)
    const dataOrderKey = store.core.order_key_between(store.outline.childrenOf(contentFolderId).at(-1)?.order_key ?? undefined, undefined)
    let operations: Operation[]
    try {
      operations = definition.create({ store, surfaceId, contentFolderId, cellId, dataId, parentId: parent, orderKey, dataOrderKey, placement, title: request.title, existingSourceId: request.sourceId, config })
    } catch (error) {
      store.notify('error', `无法插入${request.entry.title}：${describeError(error)}`)
      return
    }
    if (!isFree) for (const op of operations) if (op.op === 'entity.create' && op.entity_id === cellId) delete (op as Record<string, unknown>).placement
    const outcome = await store.submit({ editId: `insert:${cellId}`, label: `插入${request.entry.title}`, operations })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') {
      setSelectionState(new Set([cellId]))
      if (request.entry.catalog.editAfterInsert && !request.sourceId && policy.editContent) {
        // a new wish opens its task in the right panel (it is not laid out yet when this runs)
        if (isFree && definition.type === 'wish') { setWishOpen({ wishId: dataId, cellId }); setSide('wish') } else { requestIntent(cellId, 'end'); setEditing(cellId) }
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- centreSpot reads the camera at call time
  }, [store, surface.content_folder_id, surfaceId, laid, isFree, insertReason, policy.editContent, setEditing, setSide])

  /** The toolbar's pick: a one-shot placement on a free Surface, an append on a flow page. */
  const pick = (request: InsertRequest) => {
    if (insertReason) { store.notify('info', insertReason); return }
    if (isFree) { setPlacing(request); setTool('select') } else void insert(request, null)
  }
  const pickEntry = (entry: CatalogEntry) => {
    if (entry.definition.type === 'wish' && single?.view_type === 'wish' && policy.editContent) { setEditing(single.entity_id); return }
    pick({ entry })
  }
  const openCatalog = (tab: CatalogTab, key?: string, world: { x: number; y: number } | null = null, parentId?: string) => setCatalog({ tab, key, world, parentId })

  // ---- actions on the selection
  const selectedEntities = [...selection].map((id) => store.outline.get(id)).filter((e): e is EntityEnvelope => Boolean(e))
  const single = selectedEntities.length === 1 ? selectedEntities[0] : null
  const selectedBlock = useBlockContext({
    cellId: single?.entity_id ?? null, mode, view: 'canvas', selected: true,
    editorActive: Boolean(single && editing === single.entity_id), zoom: camera.zoom,
    onActivate: () => { if (single) setEditing(single.entity_id) }, onDeactivate: () => setEditing(null),
  })
  /** Locked objects stay where they are (§7.2): skipped by delete, moves and grouping, with a note. */
  const unlocked = (ids: string[], what: string) => {
    const kept = ids.filter((id) => !laid.get(id)?.locked)
    if (kept.length < ids.length) store.notify('info', `${ids.length - kept.length} 个已锁定的对象没有${what}；先解锁。`)
    return kept
  }
  const deleteSelection = async () => {
    if (!canLayout) return
    const ops: Operation[] = []
    for (const id of unlocked(topLevel(laid, selection), '删除')) {
      const entity = store.outline.get(id)
      if (!entity || !entity.capabilities.includes('delete')) continue
      const descendants = store.outline.descendants(id)
      ops.push({ op: 'entity.delete', entity_id: id, ...(descendants.length ? { subtree: { delete: descendants.map((d) => d.entity_id) } } : {}), expect: { rev: entity.life_rev } })
      // canvas content data that only this Block showed goes with it (§4.3, pending product confirmation §14-2)
      for (const cell of [entity, ...descendants].filter((e) => e.type_id === 'buckyos.cell')) {
        const source = cell.source_id ? store.outline.get(cell.source_id) : undefined
        if (!source || source.deleted || !source.capabilities.includes('delete')) continue
        const inContent = store.outline.ancestors(source.entity_id).includes('canvas-content')
        const otherBlocks = store.outline.all().some((e) => e.type_id === 'buckyos.cell' && e.source_id === source.entity_id && e.entity_id !== cell.entity_id && !e.deleted && !descendants.some((d) => d.entity_id === e.entity_id))
        if (inContent && !otherBlocks && !ops.some((op) => op.entity_id === source.entity_id)) {
          const kids = store.outline.descendants(source.entity_id)
          ops.push({ op: 'entity.delete', entity_id: source.entity_id, ...(kids.length ? { subtree: { delete: kids.map((d) => d.entity_id) } } : {}), expect: { rev: source.life_rev } })
        } else if (inContent && otherBlocks) store.notify('info', `「${source.title ?? source.name ?? source.entity_id}」仍被其他 Block 使用，只删除了这个 Block。`)
      }
    }
    if (ops.length === 0) return
    const outcome = await store.submit({ editId: `delete:${randomId().slice(0, 8)}`, label: `删除 ${ops.filter((o) => o.op === 'entity.delete').length} 项`, operations: ops })
    if (outcome.status === 'rejected' && outcome.code === 'REFERENCE_BROKEN') {
      const data = outcome.errors?.[0]?.data as { referrers?: { entity_id: string }[]; hidden_referrers?: boolean } | undefined
      store.notify('error', `数据仍被引用，没有删除：${(data?.referrers ?? []).map((r) => store.outline.get(r.entity_id)?.title ?? r.entity_id).join('、')}${data?.hidden_referrers ? '（另有无权查看的引用）' : ''}。先删除引用它的 Block，或只删除 Block。`)
    } else if (outcome.status === 'rejected') {
      store.notify('error', `没有删除：${outcome.code}${outcome.detail ? `（${outcome.detail}）` : ''}`)
    } else if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set())
  }
  /** Lock or unlock the selection: the shared `locked` key of each Block or group (written with `update`). */
  const setLocked = async (ids: string[], locked: boolean) => {
    const ops: Operation[] = []
    for (const id of ids) {
      const entity = store.outline.get(id)
      if (!entity || !entity.capabilities.includes('update') || Boolean(entity.locked) === locked) continue
      const read = await store.session.read<{ key_revs: Record<string, number> }>(id)
      const expect = { rev: read.content.key_revs.locked ?? 0 }
      ops.push(locked ? { op: 'entity.set_keys', entity_id: id, keys: [{ key: 'locked', value: true, expect }] } : { op: 'entity.unset_keys', entity_id: id, keys: [{ key: 'locked', expect }] })
    }
    if (ops.length) void store.submit({ editId: `lock:${randomId().slice(0, 8)}`, label: locked ? `锁定 ${ops.length} 个对象` : `解锁 ${ops.length} 个对象`, operations: ops })
  }
  const reorder = (direction: 'front' | 'back') => {
    const ops: Operation[] = []
    for (const id of topLevel(laid, selection)) {
      const l = laid.get(id)
      if (!l) continue
      const siblings = store.outline.childrenOf(l.parentId)
      const key = direction === 'front' ? store.core.order_key_between(siblings.at(-1)?.order_key ?? undefined, undefined) : store.core.order_key_between(undefined, siblings[0]?.order_key ?? undefined)
      ops.push({ op: 'tree.place', entity_id: id, order_key: key })
    }
    if (ops.length) void store.submit({ editId: `order:${randomId().slice(0, 8)}`, label: direction === 'front' ? '置顶' : '置底', operations: ops })
  }
  const group = () => {
    const ids = unlocked(topLevel(laid, selection), '加入分组')
    if (ids.length < 2) return
    const bounds = boundsOf(laid, ids)
    if (!bounds) return
    const parentId = laid.get(ids[0])?.parentId ?? surfaceId
    const groupId = randomId('g')
    const rect = { x: bounds.x - 16, y: bounds.y - 40, w: bounds.w + 32, h: bounds.h + 56 }
    const ops: Operation[] = [{ op: 'entity.create', entity_id: groupId, type_id: 'buckyos.container', parent_id: parentId, order_key: store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined), placement: relativeTo(laid, parentId, rect), payload: { kind: 'group', layout: { mode: 'free' }, title: '分组' } }]
    let key: string | undefined
    for (const id of ids) {
      const l = laid.get(id)!
      key = store.core.order_key_between(key, undefined)
      ops.push({ op: 'tree.move', entity_id: id, new_parent_id: groupId, order_key: key, placement: { x: Math.round(l.rect.x - rect.x), y: Math.round(l.rect.y - rect.y), w: l.rect.w, h: l.rect.h, ...(l.rotation ? { rotation: l.rotation } : {}) } })
    }
    void store.submit({ editId: `group:${groupId}`, label: `分组 ${ids.length} 个 Block`, operations: ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set([groupId])) })
  }
  const ungroup = () => {
    if (!single || single.kind !== 'group') return
    const l = laid.get(single.entity_id)
    if (!l) return
    const ops: Operation[] = []
    let key = store.outline.childrenOf(l.parentId).at(-1)?.order_key
    const moved: string[] = []
    for (const child of store.outline.childrenOf(single.entity_id)) {
      const cl = laid.get(child.entity_id)
      if (!cl) continue
      key = store.core.order_key_between(key, undefined)
      ops.push({ op: 'tree.move', entity_id: child.entity_id, new_parent_id: l.parentId, order_key: key, placement: relativeTo(laid, l.parentId, cl.rect, cl.rotation) })
      moved.push(child.entity_id)
    }
    ops.push({ op: 'entity.delete', entity_id: single.entity_id, expect: { rev: single.life_rev } })
    void store.submit({ editId: `ungroup:${single.entity_id}`, label: '解组', operations: ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set(moved)) })
  }
  const moveToSurface = (targetId: string) => {
    const target = store.outline.get(targetId)
    if (!target) return
    const ops: Operation[] = []
    let key = store.outline.childrenOf(targetId).at(-1)?.order_key
    for (const id of unlocked(topLevel(laid, selection), '移动')) {
      const l = laid.get(id)
      if (!l) continue
      key = store.core.order_key_between(key, undefined)
      const placement = movedPlacement(laid, l, 0, 0, '')
      store.noteLayoutIntent(id, placement, key, targetId)
      ops.push({ op: 'tree.move', entity_id: id, new_parent_id: targetId, order_key: key, placement })
    }
    if (ops.length) void store.submit({ editId: `move-surface:${randomId().slice(0, 8)}`, label: `移动到画布 ${target.title ?? target.name ?? ''}`, operations: ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set()) })
    setPicker(null)
  }
  const annotateSelection = () => {
    if (!single?.source_id) return
    const source = store.outline.get(single.source_id)
    if (!source) return
    ui.annotate?.({ target: { entity_id: source.entity_id }, label: source.title ?? source.name ?? source.entity_id })
  }

  // ---- clipboard (UI improvement §6.4)
  const copy: Command = {
    reason: !policy.select ? '当前模式不能选择对象' : selection.size === 0 ? '没有选中的对象' : null,
    run: () => {
      void copyToClipboard(store, surfaceId, laid, selection, 'copy').then(
        (n) => { if (n > 0) store.notify('info', `已复制 ${n} 个对象。粘贴得到新的视图，数据与原对象共享。`) },
        (error: unknown) => store.notify('error', `复制失败：${describeError(error)}`))
    },
  }
  const cut: Command = {
    reason: !canLayout ? (policy.layout ? '没有调整这张画布的权限' : '当前模式不能移动对象') : selection.size === 0 ? '没有选中的对象' : null,
    run: () => { void copyToClipboard(store, surfaceId, laid, selection, 'cut').then((n) => { if (n > 0) store.notify('info', `已剪切 ${n} 个对象：粘贴时移动到目标位置，取消或粘贴失败时原对象保留。`) }) },
  }
  const pasteAt = (at: { x: number; y: number } | null) => {
    const current = canvasClipboard.snapshot()
    if (!current) return
    if (current.workspaceId !== store.session.workspaceId) { store.notify('info', '暂不支持跨工作区粘贴：需要完整导入与引用重映射。'); return }
    const c = camera.center
    const { operations, newIds, label } = pasteOperations(store, current, { surfaceId, isFree, at, center: camera.toWorld(c.x, c.y) })
    if (operations.length === 0) { store.notify('info', '剪切的对象已不存在。'); canvasClipboard.set(null); return }
    void store.submit({ editId: `paste:${randomId().slice(0, 8)}`, label, operations }).then((outcome) => {
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') {
        setSelectionState(new Set(newIds))
        if (current.kind === 'cut') canvasClipboard.set(null)
        else canvasClipboard.notePaste()
      } else if (outcome.status === 'rejected' || outcome.status === 'conflict') {
        store.notify('error', `粘贴没有完成（${outcome.code}）${current.kind === 'cut' ? '：原对象保留在原处。' : '。'}`)
      }
    })
  }
  const paste: Command = {
    reason: !clip ? '剪贴板是空的' : clip.workspaceId !== store.session.workspaceId ? '剪贴板中的对象来自另一个工作区' : insertReason ?? (canLayout ? null : '没有调整这张画布的权限'),
    run: () => pasteAt(null),
  }
  const remove: Command = { reason: !canLayout ? (policy.layout ? '没有调整这张画布的权限' : '当前模式不能删除对象') : selection.size === 0 ? '没有选中的对象' : null, run: () => { void deleteSelection() } }

  const fitAll = () => { const b = surfaceBounds(laid); if (b) camera.fit(b); else camera.set({ x: -camera.clearArea.x, y: -camera.clearArea.y, zoom: 1 }) }
  const fitSelection = () => { const b = boundsOf(laid, selection); if (b) camera.fit(b, 80) }

  // ---- keyboard (§8.2; UI improvement §7.2, §11): Esc exits layer by layer; shortcuts need the canvas focus and never take text input
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      if (event.isComposing || target?.closest('input, textarea, select, [contenteditable="true"], [data-role="editor"]')) return
      // a placement or target pick started from a toolbar button is cancelled by Esc even while that button has the focus
      if (event.key === 'Escape' && (placing || annotatePick) && !target?.closest('.aiws-popover, .aiws-dialog, .aiws-menu')) { setPlacing(null); setAnnotatePick(false); return }
      if (target?.closest('.aiws-panel, .aiws-popover, .aiws-dialog, .aiws-menu, .aiws-side, .aiws-near')) return
      if (!hostRef.current?.contains(target) && target !== document.body) return
      const mod = event.ctrlKey || event.metaKey
      if (event.key === 'Escape') {
        if (menu) { setMenu(null); return }
        if (picker) { setPicker(null); return }
        if (placing) { setPlacing(null); return }
        if (annotatePick) { setAnnotatePick(false); return }
        if (editing) { setEditing(null); return }
        if (selection.size > 0) { setSelectionState(new Set()); return }
        if (clip?.kind === 'cut') { canvasClipboard.set(null); return }
        return
      }
      if (placing && event.key === 'Enter') { event.preventDefault(); void insert(placing, null); setPlacing(null); return }
      // a selected text or note takes typing at once (§4.4): the key starts the editor (single-letter shortcuts yield)
      const typable = single && !editing && policy.editContent && (single.view_type === 'richtext' || single.view_type === 'note') && !laid.get(single.entity_id)?.locked
      if (typable && !mod && !event.altKey && event.key.length === 1 && event.key !== ' ') { event.preventDefault(); requestIntent(single.entity_id, `type:${event.key}`); setEditing(single.entity_id); return }
      if (!mod && !event.altKey && isFree && (event.key === 'v' || event.key === 'V')) { setTool('select'); setPlacing(null); return }
      if (!mod && !event.altKey && isFree && (event.key === 'h' || event.key === 'H')) { setTool('hand'); setPlacing(null); return }
      if (mod && !event.altKey && !event.shiftKey) {
        const key = event.key.toLowerCase()
        if (key === 'c' && copy.reason === null) { event.preventDefault(); copy.run(); return }
        if (key === 'x' && cut.reason === null) { event.preventDefault(); cut.run(); return }
        if (key === 'v' && paste.reason === null) { event.preventDefault(); paste.run(); return }
      }
      if (!policy.select) return
      if ((event.key === 'Delete' || event.key === 'Backspace') && remove.reason === null) { event.preventDefault(); remove.run(); return }
      if (event.key === 'Enter' && single && policy.editContent && !editing) { event.preventDefault(); requestIntent(single.entity_id, 'end'); setEditing(single.entity_id); return }
      if (event.key.startsWith('Arrow') && selection.size > 0 && canLayout) {
        event.preventDefault()
        const step = event.shiftKey ? 10 : 1
        const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0
        const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0
        commitLayout(topLevel(laid, selection).flatMap((id) => { const l = laid.get(id); return l && !l.locked ? [{ id, placement: movedPlacement(laid, l, dx, dy) }] : [] }))
        return
      }
      if (mod && (event.key === ']' || event.key === '[') && selection.size > 0 && canLayout) { event.preventDefault(); reorder(event.key === ']' ? 'front' : 'back'); return }
      if (mod && event.key.toLowerCase() === 'g' && canLayout) { event.preventDefault(); if (event.shiftKey) ungroup(); else group(); return }
      if (event.key === 'F2' && single && policy.layout) { event.preventDefault(); shell.setSide('inspector') }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  // ---- near toolbar (§9.2; 标准对象的交互改进 §5): the Editor's tools while editing → the type's tools → common
  // tools (annotate, lock, AI) → "more" (the same list as the context menu)
  const moreActions: NearAction[] = []
  const typeItems: ToolbarItem[] = []
  const commonItems: ToolbarItem[] = []
  const topIds = topLevel(laid, selection)
  const lockable = canLayout && topIds.length > 0 && topIds.every((id) => store.outline.get(id)?.capabilities.includes('update'))
  const allLocked = topIds.length > 0 && topIds.every((id) => laid.get(id)?.locked)
  const lockedHere = topIds.every((id) => store.outline.get(id)?.locked)
  const singleLocked = single ? Boolean(laid.get(single.entity_id)?.locked) : false
  const wishEntry = registryEntries().find((entry) => entry.definition.type === 'wish' && entry.catalog.standard)
  if (single && policy.select) {
    const context = selectedBlock.context
    const definition = selectedBlock.resolution?.ok ? context?.definition : undefined
    const source = single.source_id ? store.outline.get(single.source_id) : undefined
    if (definition?.actions && context && !singleLocked) {
      for (const action of definition.actions) {
        if (!action.modes.includes(mode) || (action.views && !action.views.includes('canvas'))) continue
        const needs = action.needs ?? []
        const target = action.onSource ? source : single
        if (needs.some((cap) => !(target?.capabilities ?? []).includes(cap))) continue
        const writes = needs.some((cap) => ['update', 'append', 'structure', 'delete', 'manage'].includes(cap))
        if (writes && context.readOnlyReason) continue
        // the actions of a definition that declares no tools of its own (an extension, until S5) stay on the toolbar as words
        const asTool = !definition.toolbar
        try {
          if (action.when && !action.when(context)) continue
          const run = () => { void Promise.resolve().then(() => action.run(context, store)).catch((error: unknown) => store.notify('error', `扩展动作失败：${describeError(error)}`)) }
          if (asTool) typeItems.push({ kind: 'button', id: action.id, label: action.label, key: action.key, run })
          else moreActions.push({ id: action.id, label: action.label, key: action.key, run })
        } catch (error) {
          if (asTool) typeItems.push({ kind: 'button', id: action.id, label: action.label, disabled: describeError(error), run: () => undefined })
          else moreActions.push({ id: action.id, label: action.label, disabled: true, title: describeError(error), run: () => undefined })
        }
      }
    }
    // the type's own tools: edit mode only, never on a locked object (§5.3)
    if (definition?.toolbar && context && mode === 'edit' && !singleLocked) {
      try { typeItems.push(...definition.toolbar(context, store)) } catch (error) { console.error('[aiworkspace] block toolbar failed', error) }
    }
    if (single.kind === 'group' && canLayout && !singleLocked) typeItems.push({ kind: 'button', id: 'ungroup', icon: Ungroup, label: '解组', key: 'Ctrl+Shift+G', run: ungroup })
    if (definition?.Inspector || definition?.configFields || single.kind === 'group') moreActions.push({ id: 'inspector', label: '属性', key: 'F2', run: () => shell.setSide('inspector') })
    if (policy.annotate && source && ui.annotate) commonItems.push({ kind: 'button', id: 'annotate', icon: MessageSquarePlus, label: '批注', run: annotateSelection })
    if (source) moreActions.push({ id: 'relations', label: '查看数据与依赖', run: () => shell.setSide('relations') })
    if (source && canLayout && !singleLocked) moreActions.push({ id: 'add-view', label: '增加视图…', run: () => setPicker({ kind: 'add-view' }) })
    if (single.kind === 'group' && canLayout && !singleLocked) moreActions.push({ id: 'ungroup', label: '解组', key: 'Ctrl+Shift+G', run: ungroup })
  }
  if (selection.size > 1 && canLayout && mode === 'edit') typeItems.push({ kind: 'button', id: 'group', icon: Group, label: '分组', key: 'Ctrl+G', run: group })
  if (selection.size > 1 && canLayout) moreActions.push({ id: 'group', label: '分组', key: 'Ctrl+G', run: group })
  if (lockable && mode === 'edit') commonItems.push(allLocked
    ? { kind: 'button', id: 'unlock', icon: LockOpen, label: '解锁', disabled: lockedHere ? false : '所在分组已锁定：先解锁分组', run: () => { void setLocked(topIds, false) } }
    : { kind: 'button', id: 'lock', icon: Lock, label: '锁定（不能移动、缩放或删除）', run: () => { void setLocked(topIds, true) } })
  if (wishEntry && selection.size > 0 && policy.insert && insertReason === null && !allLocked) commonItems.push({ kind: 'button', id: 'ai', icon: Sparkles, label: single?.view_type === 'wish' ? '打开许愿格' : '对选中的对象许愿', ai: true, run: () => pickEntry(wishEntry) })
  if (selection.size > 0 && policy.select && !phone) {
    moreActions.push({ id: 'copy', label: '复制', key: 'Ctrl+C', run: copy.run })
    if (cut.reason === null && !allLocked) moreActions.push({ id: 'cut', label: '剪切', key: 'Ctrl+X', run: cut.run })
  }
  if (selection.size > 0 && canLayout && !allLocked) {
    moreActions.push({ id: 'front', label: '置顶', key: 'Ctrl+]', run: () => reorder('front') }, { id: 'back', label: '置底', key: 'Ctrl+[', run: () => reorder('back') })
    if (surfacesOf(store).length > 1) moreActions.push({ id: 'move-surface', label: '移动到画布…', run: () => setPicker({ kind: 'move-surface' }) })
    moreActions.push({ id: 'delete', label: '删除', key: 'Delete', run: () => { void deleteSelection() } })
  }
  // a locked object offers "unlock" and reading only (§4.3): no type tools, nothing that moves or removes it
  const editorItems = single && editing === single.entity_id ? toolbarSink.items() : []
  const nearItems: ToolbarItem[] = []
  for (const part of [editorItems, typeItems, commonItems]) {
    if (part.length === 0) continue
    if (nearItems.length > 0) nearItems.push({ kind: 'separator', id: `sep-${nearItems.length}` })
    nearItems.push(...part)
  }

  // ---- context menu items
  const quickEntries = registryEntries().filter((entry) => entry.catalog.standard && entry.catalog.needs === 'none')
  const blockMenu = [
    ...commonItems.flatMap((item) => (item.kind === 'button' && item.id !== 'ai' ? [{ id: item.id, label: item.label, run: item.run, disabled: Boolean(item.disabled) }] : [])),
    ...moreActions.map((a) => ({ id: a.id, label: a.label, run: a.run, disabled: a.disabled })),
  ]
  const menuItems = menu ? (menu.blockId ? blockMenu : [
    ...(insertReason === null ? quickEntries.map((entry) => ({ id: `insert-${entry.definition.type}`, label: `插入${entry.title}`, run: () => { void insert({ entry }, menu.world, menu.parentId) } })) : []),
    ...(insertReason === null && !isFree ? [{ id: 'insert-group', label: '插入分组', run: () => {
      const parent = menu.parentId ?? surfaceId
      const groupId = randomId('g')
      void store.submit({ editId: `insert:${groupId}`, label: '插入分组', operations: [{ op: 'entity.create', entity_id: groupId, type_id: 'buckyos.container', parent_id: parent, order_key: store.core.order_key_between(store.outline.childrenOf(parent).at(-1)?.order_key ?? undefined, undefined), payload: { kind: 'group', layout: { mode: 'flow' }, title: '分组' } }] })
    } }] : []),
    ...(insertReason === null ? [
      { id: 'catalog', label: '插入对象…', run: () => openCatalog('all', undefined, menu.world, menu.parentId) },
      { id: 'add-data', label: '添加已有数据…', run: () => openCatalog('existing', undefined, menu.world, menu.parentId) },
    ] : []),
    ...(paste.reason === null ? [{ id: 'paste', label: '粘贴', run: () => pasteAt(menu.world) }] : []),
    ...(isFree ? [{ id: 'sep', label: '', run: () => undefined, separator: true }, { id: 'fit', label: '适应全部', run: fitAll }] : []),
  ].filter((item, i) => !(i === 0 && 'separator' in item))) : []

  const commands: CanvasCommands = {
    isFree, insertReason,
    insert: (entry) => {
      if (entry.catalog.needs === 'none') void insert({ entry }, null)
      else openCatalog(entry.group === 'extension' ? 'extension' : 'all', entry.key)
    },
    openCatalog: (tab, key) => openCatalog(tab, key),
    copy, cut, paste, remove,
    view: isFree ? { camera, fitAll, fitSelection, hasSelection: selection.size > 0 } : null,
    canvasTabs: CANVAS_SIDE_TABS,
  }
  const annotateCommand = {
    reason: !canComment ? '没有批注权限' : !policy.annotate ? '当前模式不能批注' : null,
    active: annotatePick,
    run: () => {
      if (single?.source_id) { annotateSelection(); return }
      setAnnotatePick((value) => !value)
      shell.setSide('annotations')
    },
  }
  const sideContent = (tab: SideTab) => {
    switch (tab) {
      case 'inspector': return single ? <SelectionInspector cell={single} mode={mode} context={selectedBlock.context} resolved={Boolean(selectedBlock.resolution?.ok)} registryVersion={selectedBlock.registryVersion} /> : <div className="aiws-muted">选中一个对象查看属性。</div>
      case 'relations': return single?.source_id ? <RelationsPanel entityId={single.source_id} /> : <div className="aiws-muted">选中一个数据对象查看它的引用与依赖。</div>
      case 'annotations': return <AnnotationsPanel parentId={surface.content_folder_id ?? null} />
      case 'collab': return <PermissionsPanel />
      case 'edits': return <StatusDetail />
      case 'wish': return wishOpen && store.outline.get(wishOpen.wishId)
        ? <WishPanel key={wishOpen.wishId} wishId={wishOpen.wishId} cellId={wishOpen.cellId} readOnly={readOnlyNow || !policy.writes} canvasSelection={[...selection].filter((id) => id !== wishOpen.cellId)} />
        : <div className="aiws-muted">在画布上双击许愿格，或点它的“打开”，在这里编辑和运行。</div>
    }
  }
  const sideTabs = wishOpen ? [...CANVAS_SIDE_TABS, 'wish' as const] : CANVAS_SIDE_TABS
  const clipIds = clip?.kind === 'cut' && clip.surfaceId === surfaceId ? clip.ids.join(',') : ''
  const cutIds = useMemo(() => (clipIds ? new Set(clipIds.split(',')) : undefined), [clipIds])

  const placingTitle = placing?.entry.title ?? ''
  return (
    <BudgetContext.Provider value={budget}>
      <div className="aiws-canvas-view" data-testid="aiws-canvas-view" data-mode={mode}>
        <div className="aiws-canvas-body" ref={hostRef} data-testid="aiws-canvas-body">
          <div ref={mainRef} className={`aiws-canvas-main${mode === 'presentation_edit' ? ' is-placeholder' : ''}${shell.prefs.grid ? '' : ' no-grid'}${showObjectToolbar && !collapsed ? ' has-left-tools' : ''}`}>
            {isFree ? (
              <EditorToolbarContext.Provider value={toolbarSink}>
              <OpenWishContext.Provider value={shell.side === 'wish' ? wishOpen?.cellId ?? null : null}>
              <RenderHost surfaceId={surfaceId} mode={mode} camera={camera} laid={laid} index={index} selection={selection} onSelectionChange={setSelection} editing={editing} onEditingChange={setEditing}
                canLayout={canLayout} onCommitLayout={commitLayout} onOpenEntity={ui.openEntity} gesturesPaused={Boolean(menu || picker || catalog || overlayOpen)}
                tool={tool} placing={placing ? { ...placing.entry.definition.defaultSize, label: placingTitle } : null}
                onPlace={(world) => { const request = placing; setPlacing(null); if (request) void insert(request, world) }}
                onContextMenu={(point, blockId) => { if (!policy.select && !policy.insert) return; setMenu({ at: { x: point.screenX, y: point.screenY }, world: { x: point.worldX, y: point.worldY }, blockId }) }}
                cutIds={cutIds} renderNear={(bbox) => <NearToolbar bbox={bbox} items={nearItems} more={moreActions} viewport={viewportSize} insets={insets} />} />
              </OpenWishContext.Provider>
              </EditorToolbarContext.Provider>
            ) : (
              <div className="aiws-flow-host" data-testid="aiws-flow-host"><FlowSurface surfaceId={surfaceId} mode={mode} selected={selection} onSelect={setSelection} editing={editing} onEditingChange={setEditing} onInsert={(parentId) => setMenu({ at: { x: 200, y: 120 }, world: null, blockId: null, parentId })} /></div>
            )}
            {mode === 'presentation_edit' && (
              <div className="aiws-presentation-placeholder" role="status" data-testid="aiws-presentation-placeholder">
                <b>播放编辑尚未实现</b>
                <div>这里将来编排演示路径、镜头、备注和互动。现在只是入口：画布保持静态显示，不选中、不写文档。</div>
              </div>
            )}
            <div className="aiws-chrome-top">
              {!phone && <MainToolbar canvas={commands} />}
              {(shell.prefs.presenterToolbar || phone) && <PresenterToolbar camera={isFree ? camera : null} hasSelection={selection.size > 0} onFitAll={fitAll} onFitSelection={fitSelection} annotate={annotateCommand} />}
            </div>
            {showObjectToolbar && (
              <div className="aiws-chrome-left">
                <ObjectToolbar isFree={isFree} tool={tool} onTool={(next) => { setTool(next); setPlacing(null) }} placingKey={placing?.entry.key ?? null} insertReason={insertReason}
                  onPick={pickEntry} onPickFile={(entry, file) => pick({ entry, file })} onOpenCatalog={(tab, key) => openCatalog(tab, key)} collapsed={collapsed} onCollapsed={setCollapsed} />
              </div>
            )}
            {(placing || annotatePick) && (
              <div className="aiws-hint" role="status" data-testid="aiws-tool-hint">
                {placing ? `点击画布放置${placingTitle}；Enter 放到视图中央，Esc 取消` : phone ? '点按要批注的对象；再点一次“批注”取消' : '点选要批注的对象；Esc 取消'}
              </div>
            )}
            <StatusDock />
            {menu && <ContextMenu at={menu.at} items={menuItems} onClose={() => setMenu(null)} />}
            {picker && <Picker kind={picker.kind} selection={selectedEntities} surfaceId={surfaceId} onClose={() => setPicker(null)} onPick={(choice) => {
              if (picker.kind === 'move-surface') moveToSurface(choice)
              else if (picker.kind === 'add-view' && single?.source_id) {
                const def = blockRegistry.get(choice)
                if (def?.catalog) void insert({ entry: { key: `block:${def.type}`, definition: def, catalog: def.catalog, title: def.title, group: def.catalog.group }, sourceId: single.source_id }, null)
                setPicker(null)
              }
            }} />}
            {catalog && (
              <InsertCatalog initialTab={catalog.tab} initialKey={catalog.key} canPlace={isFree} onClose={() => setCatalog(null)} onInsert={(request, how) => {
                const at = catalog.world
                const parentId = catalog.parentId
                setCatalog(null)
                if (how === 'place') pick(request)
                else void insert(request, at, parentId)
              }} />
            )}
          </div>
          <SidePanel tabs={sideTabs} render={sideContent} />
        </div>
      </div>
    </BudgetContext.Provider>
  )
}

function SelectionInspector({ cell, mode, context, resolved, registryVersion }: { cell: EntityEnvelope; mode: CanvasMode; context: RenderContext | null; resolved: boolean; registryVersion: number }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const live = context?.cell ?? cell
  const definition = resolved ? context?.definition : undefined
  const source = context?.source
  const payload = context?.payload
  const keyRevs = context?.keyRevs ?? {}
  const readOnly = !context || context.readOnlyReason !== null
  const setConfig = (key: string, value: Json) => {
    if (!payload || readOnly) return
    const config = { ...(payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${cell.entity_id}`, label: `属性 ${key}`, operations: [{ op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: keyRevs.config ?? 0 } }] }] })
  }
  const setTitle = (title: string) => {
    if (!payload || readOnly || title === (payload.title ?? '')) return
    void store.submit({ editId: `key:${cell.entity_id}:title`, label: `Block 标题 → ${title || '（清除）'}`, operations: [title ? { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: title, expect: { rev: keyRevs.title ?? 0 } }] } : { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev: keyRevs.title ?? 0 } }] }] })
  }
  const Inspector = definition?.Inspector
  return (
    <BlockInspector cellId={cell.entity_id} mode={mode}>
      {payload && (
        <label className="aiws-inline-form">标题 <input aria-label="Block 标题" defaultValue={payload.title ?? ''} disabled={readOnly} onBlur={(event) => setTitle(event.target.value.trim())} /></label>
      )}
      {source && <div className="aiws-muted">数据：<button type="button" className="aiws-link" onClick={() => ui.openEntity(source.entity_id)}>{source.title ?? source.name ?? source.entity_id}</button>（{source.type_id.replace('buckyos.', '')}）</div>}
      {definition?.configFields && payload && definition.configFields.map((field) => (
        <label key={field.key} className="aiws-inline-form">{field.label}
          {field.kind === 'select'
            ? <select value={String(payload.config?.[field.key] ?? field.options?.[0]?.value ?? '')} disabled={readOnly} onChange={(event) => setConfig(field.key, event.target.value)}>{(field.options ?? []).map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}</select>
            : field.kind === 'boolean'
              ? <input type="checkbox" checked={Boolean(payload.config?.[field.key])} disabled={readOnly} onChange={(event) => setConfig(field.key, event.target.checked)} />
              : <input type={field.kind === 'color' ? 'color' : field.kind === 'number' ? 'number' : 'text'} value={String(payload.config?.[field.key] ?? (field.kind === 'color' ? '#4f8df7' : ''))} disabled={readOnly} onChange={(event) => setConfig(field.key, field.kind === 'number' ? Number(event.target.value) : event.target.value)} />}
        </label>
      ))}
      {Inspector && context && (
        <BlockBoundary resetKey={`${cell.entity_id}:${mode}:${live.content_rev}:${registryVersion}`} fallback={(error, reset) => (
          <div className="aiws-block-fallback" data-testid="aiws-inspector-error"><div className="aiws-error" role="alert">属性面板出错：{describeError(error)}</div><button type="button" onClick={reset}>重试</button></div>
        )}>
          <Inspector {...context} />
        </BlockBoundary>
      )}
      <details className="aiws-details">
        <summary>详情</summary>
        {live.placement && <div className="aiws-muted">位置 {live.placement.x}, {live.placement.y} · 尺寸 {live.placement.w} × {live.placement.h} · 顺序 {live.order_key}</div>}
      </details>
    </BlockInspector>
  )
}

function Picker({ kind, selection, surfaceId, onClose, onPick }: { kind: 'add-view' | 'move-surface'; selection: EntityEnvelope[]; surfaceId: string; onClose: () => void; onPick: (id: string) => void }) {
  const store = useStore()
  const [chosen, setChosen] = useState('')
  const single = selection[0]
  const source = single?.source_id ? store.outline.get(single.source_id) : undefined
  const renderers = kind === 'add-view' && source ? blockRegistry.forSource(source.type_id).filter((def) => def.create && def.catalog && def.catalog.needs !== 'definition') : []
  const targets = surfacesOf(store).filter((s) => s.entity_id !== surfaceId)
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog" role="dialog" aria-modal="true" aria-label={kind === 'add-view' ? '增加视图' : '移动到画布'} data-testid={`aiws-picker-${kind}`} onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
        {kind === 'add-view' && (
          <>
            <b>为「{source?.title ?? source?.name ?? ''}」增加视图</b>
            <div className="aiws-muted">新 Block 与原视图共享数据，视图配置独立。</div>
            <select aria-label="展现方式" data-testid="aiws-picker-renderer" value={chosen} onChange={(event) => setChosen(event.target.value)}><option value="">选择展现…</option>{renderers.map((d) => <option key={d.type} value={d.type}>{d.title}</option>)}</select>
            <div className="aiws-dialog-actions"><button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!chosen} onClick={() => onPick(chosen)}>增加</button><button type="button" onClick={onClose}>取消</button></div>
          </>
        )}
        {kind === 'move-surface' && (
          <>
            <b>移动 {selection.length} 个对象到另一张画布</b>
            <div className="aiws-muted">对象的身份、数据绑定和依赖记录不变；数据留在原画布内容区。</div>
            <select aria-label="目标画布" data-testid="aiws-picker-surface" value={chosen} onChange={(event) => setChosen(event.target.value)}><option value="">选择画布…</option>{targets.map((s) => <option key={s.entity_id} value={s.entity_id}>{s.title ?? s.name ?? s.entity_id}</option>)}</select>
            <div className="aiws-dialog-actions"><button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!chosen} onClick={() => onPick(chosen)}>移动</button><button type="button" onClick={onClose}>取消</button></div>
          </>
        )}
      </div>
    </div>
  )
}
