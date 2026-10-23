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
import { imageSize } from '@drawio/bytes'
import { looksLikeDrawio } from '@drawio/file'
import { svgShapeFromDocument } from '@drawio/svg'
import { Group, Lock, LockOpen, MessageSquarePlus, Sparkles, Ungroup } from 'lucide-react'
import { describeError } from '../../api/session'
import { randomId } from '../../api/ids'
import type { CellPayload, EntityEnvelope, Json, KeyedContent, Operation, Placement } from '../../api/types'
import { aiwsT } from '../../i18n'
import { useAiwsText } from '../../useAiwsText'
import { useReadOnlyReason, useStore, useUserState, useWorkspaceUi } from '../../state/hooks'
import type { WorkspaceStore } from '../../state/store'
import { BlockBoundary } from '../blocks/BlockHost'
import { setCellConfig } from '../blocks/ops'
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
import { ViewToolbar } from '../shell/ViewToolbar'
import { CANVAS_SIDE_TABS, useCanvasMode, useShell, type SideTab } from '../shell/shellContext'
import { SidePanel } from '../shell/SidePanel'
import { StatusDetail, StatusDock } from '../shell/StatusSummary'
import { OpenWishContext } from '../wish/wishBlock'
import { GuideOverlay } from '../../presentation/GuideOverlay'
import { PathEditor } from '../../presentation/PathEditor'
import { setLive } from '../../presentation/pathOps'
import { WishPanel } from '../wish/WishPanel'
import { entryId, entrySize, registryEntries, type CatalogEntry, type InsertRequest } from './catalog'
import { canvasClipboard, copyToClipboard, pasteOperations } from './clipboard'
import { anchorWorld, axisOf, boxOf, facingAnchor } from './connectors/geometry'
import { formOf, hasBoundEnd, isConnector } from './connectors/layout'
import { connectorChangeOps, createConnectorOp, freezeOps, type ConnectorChange, type NewConnector } from './connectors/ops'
import { flowOrder, moveInFlow } from './flowOps'
import { FlowSurface } from './FlowSurface'
import { exportSurfaceDrawio, saveTextFile } from './drawioExport'
import { classifyFile, importDrawio, importReports } from './drawioImport'
import { ImportReportDialog } from './ImportReportDialog'
import { MetadataForm } from './MetadataForm'
import { libraryShapeConfig, SHAPE_DRAG_TYPE, stencilPack } from './stencilLibrary'
import { InsertCatalog, type CatalogTab } from './InsertCatalog'
import { importedAssetOps } from '../sources/dataOps'
import { boundsOf, groupAt, movedPlacement, placementOf, relativeTo, surfaceBounds, topLevel, type Laid } from './layout'
import { ObjectToolbar, type PointerTool } from './ObjectToolbar'
import { Camera } from './render/camera'
import { RenderHost, type LayoutChange } from './render/RenderHost'
import { useSurfaceConfig } from './surfaceManage'
import { createSurfaceOps, surfacesOf } from './surfaceOps'
import { useSurfaceLayout } from './useSurfaceLayout'
import { BlockInspector, ContextMenu, NearToolbar, type NearAction } from './tools'

export interface CanvasFocus { surfaceId: string; blockId: string | null; nonce: number }

interface Viewport { x: number; y: number; zoom: number }

/** Screen space the floating toolbars and the status area cover (fit, centring and the near toolbar avoid it);
 * narrow windows stack the two top toolbars (a phone has only one). */
/** What a line drawn to blank space offers to create at its end (Miro's "next object"). */
const NEXT_OBJECTS = ['note', 'richtext', 'shape', 'wish']
const TOP_INSET = 68
const TOP_INSET_NARROW = 124
const LEFT_INSET = 68
/** The path editor's panel (第三期规划 §7.1) takes the left side of the canvas. */
const PATH_EDITOR_INSET = 324
const BOTTOM_INSET = 52

/** The order key right after `afterId` among the children of `parent`: first when null, last when undefined (or gone). */
function siblingKey(store: WorkspaceStore, parent: string, afterId?: string | null): string {
  const children = store.outline.childrenOf(parent)
  const index = afterId === undefined ? -2 : afterId === null ? -1 : children.findIndex((child) => child.entity_id === afterId)
  if (index === -2 || (afterId && index < 0)) return store.core.order_key_between(children.at(-1)?.order_key ?? undefined, undefined)
  return store.core.order_key_between(children[index]?.order_key ?? undefined, children[index + 1]?.order_key ?? undefined)
}

export function CanvasView({ focus }: { focus: CanvasFocus | null }) {
  const shell = useShell()
  const mode = useCanvasMode()
  const active = shell.activeSurface
  const report = useSyncExternalStore(importReports.subscribe, importReports.snapshot)
  return (
    <>
      {active ? <SurfaceView key={active.entity_id} surface={active} mode={mode} focus={focus} /> : <EmptyCanvas mode={mode} />}
      {report && <ImportReportDialog outcome={report} onClose={() => importReports.show(null)} />}
    </>
  )
}

function EmptyCanvas({ mode }: { mode: CanvasMode }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const canStructure = store.session.info().capabilities.includes('structure') && !shell.phone
  const create = async (layout: 'free' | 'flow') => {
    const { ops, surfaceId } = createSurfaceOps(store, layout === 'free' ? t('aiworkspace.canvas.empty.firstCanvas') : t('aiworkspace.canvas.empty.firstPage'), layout)
    const outcome = await store.submit({ editId: 'surface:new', label: t('aiworkspace.canvas.empty.createLabel'), operations: ops })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') shell.selectSurface(surfaceId)
  }
  return (
    <div className="aiws-canvas-view" data-testid="aiws-canvas-view" data-mode={mode}>
      <div className="aiws-canvas-body">
        <div className="aiws-canvas-main is-empty">
          <div className="aiws-empty" data-testid="aiws-no-surface">
            <p>{t('aiworkspace.canvas.empty.noCanvas')}</p>
            {canStructure ? (
              <div className="aiws-dialog-actions">
                <button type="button" className="is-primary" data-testid="aiws-create-first-surface" onClick={() => { void create('free') }}>{t('aiworkspace.canvas.empty.newFree')}</button>
                <button type="button" data-testid="aiws-create-first-flow" onClick={() => { void create('flow') }}>{t('aiworkspace.canvas.empty.newFlow')}</button>
              </div>
            ) : <p className="aiws-muted">{t(shell.phone ? 'aiworkspace.canvas.empty.phoneViewOnly' : 'aiworkspace.canvas.empty.noPermission')}</p>}
          </div>
          <div className="aiws-chrome-top">
            {shell.phone ? <ViewToolbar camera={null} hasSelection={false} onFitAll={() => undefined} onFitSelection={() => undefined} annotate={null} /> : <MainToolbar canvas={null} />}
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
  const { t, list } = useAiwsText()
  const readOnlyReason = useReadOnlyReason()
  const readOnlyNow = readOnlyReason !== null
  const overlayOpen = useOverlayOpen()
  const clip = useSyncExternalStore(canvasClipboard.subscribe, canvasClipboard.snapshot)
  const surfaceId = surface.entity_id
  const policy = modePolicy(mode)
  const [selection, setSelectionState] = useState<Set<string>>(new Set())
  const [editing, setEditingState] = useState<string | null>(null)
  /** What was typed to open an editor, while it mounts. */
  const [typeAhead] = useState(() => new Map<'typed', { id: string; text: string }>())
  const [menu, setMenu] = useState<MenuState | null>(null)
  const [picker, setPicker] = useState<{ kind: 'add-view' | 'move-surface' } | null>(null)
  const [catalog, setCatalog] = useState<{ tab: CatalogTab; key?: string; world: { x: number; y: number } | null; parentId?: string; afterId?: string | null } | null>(null)
  const [tool, setTool] = useState<PointerTool>('select')
  const [placing, setPlacing] = useState<InsertRequest | null>(null)
  const [annotatePick, setAnnotatePick] = useState(false)
  /** A line just drawn to blank space offers to create the next object at its end (标准对象的交互改进 §6.1, R11). */
  const [nextObject, setNextObject] = useState<{ connectorId: string; at: { x: number; y: number }; from: { x: number; y: number }; end: { x: number; y: number } } | null>(null)
  const [collapsedChoice, setCollapsed] = useState<boolean | null>(null)
  const collapsed = collapsedChoice ?? shell.size === 'narrow'
  const [camera] = useState(() => new Camera())
  const [budget] = useState(() => createBudget(surface.layout?.mode === 'free' ? undefined : { editors: 200, html: 8 }))
  const isFree = surface.layout?.mode === 'free'
  const { laid, index, shapeBook } = useSurfaceLayout(surfaceId)
  /** The page settings of the Surface (draw.io 导入 TODO §2.5): its background colour. */
  const surfaceLook = useSurfaceConfig(surfaceId)
  // the Editor of the Block being edited hands its tools to the near toolbar (标准对象的交互改进 §5.4)
  const [toolbarSink] = useState(() => new ToolbarSink())
  useSyncExternalStore(toolbarSink.subscribe, toolbarSink.snapshot)
  /** The wish whose flow is open in the right panel (R6): its Block stays a card. */
  const [wishOpen, setWishOpen] = useState<{ wishId: string; cellId: string } | null>(null)
  const phone = shell.phone
  const showObjectToolbar = shell.prefs.objectToolbar && mode !== 'presentation_edit' && !phone
  const topInset = shell.size === 'narrow' && !phone ? TOP_INSET_NARROW : TOP_INSET
  const leftInset = mode === 'presentation_edit' && isFree && !phone ? PATH_EDITOR_INSET : showObjectToolbar && !collapsed ? LEFT_INSET : 12
  const insets = { top: topInset, right: 12, bottom: BOTTOM_INSET, left: leftInset }
  useEffect(() => { camera.setInsets({ top: topInset, right: 12, bottom: BOTTOM_INSET, left: leftInset }) }, [camera, topInset, leftInset])

  const canComment = store.session.info().capabilities.includes('comment')
  /** One block picked while "add annotation" waits for a target. */
  const pickForAnnotation = (next: Set<string>) => {
    if (!annotatePick || next.size !== 1) return
    const picked = store.outline.get([...next][0])
    const source = picked?.source_id ? store.outline.get(picked.source_id) : undefined
    if (!source) { store.notify('info', t('aiworkspace.canvas.annotate.noData')); return }
    setAnnotatePick(false)
    ui.annotate?.({ target: { entity_id: source.entity_id }, label: source.title ?? source.name ?? source.entity_id })
  }
  // selection survives mode switches when the policy still allows it (§10.2); editors are released
  const setSelection = (next: Set<string>) => { setSelectionState(next); pickForAnnotation(next) }
  // activating an editor brings its Block into view (§8.2: the active Block is never half off-screen);
  // on a free canvas a wish opens in the right panel instead (its Block stays a card)
  const { setSide } = shell
  const setEditing = useCallback((id: string | null) => {
    typeAhead.delete('typed')
    // a flow page has no editing state (流式页交互改进 §2.1): every editor is live, the caret goes where an intent puts it,
    // and the Block being typed in is not shown as selected
    if (!isFree) { setEditingState(null); if (id) setSelectionState(new Set()); return }
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
    // a group's backing (a swimlane's frame) is edited with its group selected
    const owner = l?.backing ? l.parentId : id
    if (id && owner) setSelectionState((current) => (current.size === 1 && (current.has(id) || current.has(owner)) ? current : new Set([owner])))
    if (l) camera.ensureVisible(l.bounds)
  }, [laid, camera, isFree, setSide, store, typeAhead])
  const policyKey = `${policy.select}:${policy.editContent}:${policy.insert}`
  const [policySeen, setPolicySeen] = useState(policyKey)
  if (policySeen !== policyKey) {
    setPolicySeen(policyKey)
    if (!policy.select) setSelectionState(new Set())
    if (!policy.editContent) setEditingState(null)
    if (!policy.insert) { setPlacing(null); setCatalog(null); setTool((t) => (t === 'connector' ? 'select' : t)) }
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
    // a Block moved into another parent goes on top of its new siblings
    const lastKey = new Map<string, string | undefined>()
    const ops: Operation[] = changes.map((change) => {
      if (!change.parentId) {
        store.noteLayoutIntent(change.id, change.placement)
        return { op: 'tree.place', entity_id: change.id, placement: change.placement }
      }
      const parent = change.parentId
      const key = store.core.order_key_between(lastKey.has(parent) ? lastKey.get(parent) : store.outline.childrenOf(parent).at(-1)?.order_key ?? undefined, undefined)
      lastKey.set(parent, key)
      store.noteLayoutIntent(change.id, change.placement, key, parent)
      return { op: 'tree.move', entity_id: change.id, new_parent_id: parent, order_key: key, placement: change.placement }
    })
    // a group's backing (its frame or shape) keeps covering it (draw.io 导入 TODO §2.2)
    for (const change of changes) {
      const l = laid.get(change.id)
      if (!l?.backingId || (Math.abs(l.rect.w - change.placement.w) < 0.5 && Math.abs(l.rect.h - change.placement.h) < 0.5)) continue
      const placement = { x: 0, y: 0, w: change.placement.w, h: change.placement.h }
      ops.push({ op: 'tree.place', entity_id: l.backingId, placement })
      store.outline.patchLocal(l.backingId, { placement })
    }
    // optimistic: the frames already moved; the stream confirms (or another client's later move overrides, D1)
    for (const change of changes) store.outline.patchLocal(change.id, { placement: change.placement, ...(change.parentId ? { parent_id: change.parentId } : {}) })
    const moved = changes.filter((c) => c.parentId).length
    const label = moved > 0 ? (moved === 1 ? aiwsT(changes[0].parentId === surfaceId ? 'aiworkspace.canvas.layout.moveIntoCanvas' : 'aiworkspace.canvas.layout.moveIntoGroup') : aiwsT('aiworkspace.canvas.layout.moveRegrouped', { count: changes.length, moved }))
      : changes.length === 1 ? aiwsT('aiworkspace.canvas.moveNamedLabel', { name: laid.get(changes[0].id)?.entity.title ?? changes[0].id }) : aiwsT('aiworkspace.canvas.layout.moveBlocks', { count: changes.length })
    void store.submit({ editId: `layout:${surfaceId}:${randomId().slice(0, 8)}`, label, operations: ops })
  }, [store, laid, policy.layout, surfaceId])

  const canLayout = policy.layout && surface.capabilities.includes('structure') && !readOnlyNow
  /** A Block this user may move, resize and remove: anything on a canvas it may restructure, and — with `comment`
   * alone (a public link) — the notes it wrote, whose envelopes carry `structure` (公开链接便签). */
  const canLayoutBlock = useCallback((id: string) => canLayout || (policy.layout && !readOnlyNow && Boolean(store.outline.get(id)?.capabilities.includes('structure'))),
    [canLayout, policy.layout, readOnlyNow, store])
  const insertReason = !policy.insert ? t(mode === 'view' ? 'aiworkspace.canvas.reason.viewModeInsert' : 'aiworkspace.canvas.reason.presentationInsert')
    : !surface.capabilities.includes('structure') ? t('aiworkspace.canvas.reason.noInsertPermission')
      : readOnlyReason
  /** A note is an annotation: `comment` is enough (a public link's guest), and it can be added wherever annotating is —
   * view mode and phones too, from the annotate menu and the canvas menu (a long press on a phone). */
  const noteReason = !policy.annotate ? insertReason
    : !surface.capabilities.includes('structure') && !surface.capabilities.includes('comment') ? t('aiworkspace.canvas.reason.noNotePermission')
      : readOnlyReason
  /** What the creation tools (edit mode only) offer for notes. */
  const toolNoteReason = policy.insert ? noteReason : insertReason
  const insertReasonOf = (type: string) => (type === 'note' ? noteReason : insertReason)

  /** World point at the centre of the unobstructed visible area (top-left of a Block of `size` centred there). */
  const centreSpot = (size: { w: number; h: number }) => {
    const c = camera.center
    const w = camera.toWorld(c.x, c.y)
    const spot = { x: Math.round(w.x - size.w / 2), y: Math.round(w.y - size.h / 2) }
    while ([...laid.values()].some((l) => !l.isGroup && Math.abs(l.rect.x - spot.x) < 4 && Math.abs(l.rect.y - spot.y) < 4)) { spot.x += 24; spot.y += 24 }
    return spot
  }

  // ---- insertion (§4.2, §8.1; UI improvement §7.2): no title prompt; text and notes open their editor at once
  const insert = useCallback(async (request: InsertRequest, world: { x: number; y: number } | null, parentId?: string,
    /** Operations committed together with the insertion, given the new Block and its world rect (e.g. a line's end). */
    also?: (cellId: string, rect: { x: number; y: number; w: number; h: number }) => Promise<Operation[]>,
    /** A flow page: right after this sibling (null: first); appended when absent. */
    afterId?: string | null) => {
    const definition = request.entry.definition
    if (!definition.create || insertReasonOf(definition.type)) return false
    // a flow page shows no connectors (连接线实现方案 §8.3)
    if (definition.type === 'connector' && !isFree) { store.notify('info', aiwsT('aiworkspace.canvas.flowNoConnectors')); return false }
    let config = request.entry.config || request.config ? { ...(request.entry.config ?? {}), ...(request.config ?? {}) } : undefined
    if (request.file) {
      try {
        const uploaded = await store.session.uploadAsset(request.file, request.file.name)
        config = { ...(config ?? {}), file: { object_id: uploaded.object_id, file_name: request.file.name } }
      } catch (error) {
        store.notify('error', aiwsT('aiworkspace.canvas.uploadFailed', { name: request.file.name, reason: describeError(error) }))
        return false
      }
    }
    const contentFolderId = surface.content_folder_id ?? ''
    const size = entrySize(request.entry)
    const spot = world ?? centreSpot(size)
    // placed inside a lane or another group, it joins it (draw.io 导入 TODO §2.2)
    const parent = parentId ?? (isFree && world ? groupAt(laid, { x: spot.x + size.w / 2, y: spot.y + size.h / 2 }, surfaceId) : surfaceId)
    const placement: Placement = isFree ? relativeTo(laid, parent, { ...spot, ...size }) : { x: 0, y: 0, ...size }
    const cellId = randomId('c')
    const dataId = randomId('d')
    const orderKey = siblingKey(store, parent, afterId)
    const dataOrderKey = store.core.order_key_between(store.outline.childrenOf(contentFolderId).at(-1)?.order_key ?? undefined, undefined)
    let operations: Operation[]
    try {
      operations = definition.create({ store, surfaceId, contentFolderId, cellId, dataId, parentId: parent, orderKey, dataOrderKey, placement, title: request.title, existingSourceId: request.sourceId, config })
    } catch (error) {
      store.notify('error', aiwsT('aiworkspace.canvas.insertFailed', { name: request.entry.title, reason: describeError(error) }))
      return false
    }
    if (!isFree) for (const op of operations) if (op.op === 'entity.create' && op.entity_id === cellId) delete (op as Record<string, unknown>).placement
    if (also) {
      try { operations = [...operations, ...await also(cellId, { ...spot, ...size })] } catch (error) { store.notify('error', aiwsT('aiworkspace.canvas.insertFailed', { name: request.entry.title, reason: describeError(error) })); return false }
    }
    const outcome = await store.submit({ editId: `insert:${cellId}`, label: aiwsT('aiworkspace.canvas.insertNamed', { name: request.entry.title }), operations })
    if (outcome.status !== 'accepted' && outcome.status !== 'saved_locally') return false
    if (isFree) setSelectionState(new Set([cellId]))
    if (request.entry.catalog.editAfterInsert && !request.sourceId && policy.editContent) {
      // a new wish opens its task in the right panel (it is not laid out yet when this runs)
      if (isFree && definition.type === 'wish') { setWishOpen({ wishId: dataId, cellId }); setSide('wish') } else { requestIntent(cellId, 'end'); setEditing(cellId) }
    }
    return true
    // eslint-disable-next-line react-hooks/exhaustive-deps -- centreSpot reads the camera at call time
  }, [store, surface.content_folder_id, surfaceId, laid, isFree, insertReason, noteReason, policy.editContent, setEditing, setSide])

  /** Add a note at `world` (null: the centre of the view) on a free canvas: written in place where content is edited,
   * else (view mode, a phone) written first in the annotations panel and placed when saved. */
  const [noteDraft, setNoteDraft] = useState<{ world: { x: number; y: number } } | null>(null)
  const noteEntry = registryEntries().find((entry) => entry.definition.type === 'note' && entry.catalog.standard)
  const addNote = (world: { x: number; y: number } | null, parentId?: string) => {
    if (!noteEntry || noteReason) { store.notify('info', noteReason ?? t('aiworkspace.canvas.noNoteEntry')); return }
    if (policy.editContent || !isFree) { void insert({ entry: noteEntry }, world, parentId); return }
    setNoteDraft({ world: world ?? centreSpot(noteEntry.definition.defaultSize) })
    setAnnotatePick(false)
    setSide('annotations')
  }
  const saveNoteDraft = async (body: string) => {
    if (!noteEntry || !noteDraft) return false
    const saved = await insert({ entry: noteEntry, title: body }, noteDraft.world)
    if (saved) {
      setNoteDraft(null)
      // the drawer covers a phone's canvas: show where the note went
      if (phone) setSide(null)
    }
    return saved
  }

  /** The toolbar's pick: a one-shot placement on a free Surface, an append on a flow page. */
  const pick = (request: InsertRequest) => {
    const reason = insertReasonOf(request.entry.definition.type)
    if (reason) { store.notify('info', reason); return }
    // a line is drawn, not placed (标准对象的交互改进 §6.1)
    if (request.entry.definition.type === 'connector' && isFree) { setPlacing(null); setTool('connector'); return }
    if (isFree) { setPlacing(request); setTool('select') } else void insert(request, null)
  }
  const pickEntry = (entry: CatalogEntry) => {
    if (entry.definition.type === 'wish' && single?.view_type === 'wish' && policy.editContent) { setEditing(single.entity_id); return }
    // the connector entry is a tool: press and drag on the canvas draws a line (标准对象的交互改进 §6.1)
    if (entry.definition.type === 'connector') {
      if (insertReason) { store.notify('info', insertReason); return }
      if (!isFree) { store.notify('info', t('aiworkspace.canvas.flowNoConnectors')); return }
      setPlacing(null)
      setTool((current) => (current === 'connector' ? 'select' : 'connector'))
      return
    }
    pick({ entry })
  }
  const openCatalog = (tab: CatalogTab, key?: string, world: { x: number; y: number } | null = null, parentId?: string, afterId?: string | null) => setCatalog({ tab, key, world, parentId, afterId })

  // ---- a flow page inserts where the user is (流式页交互改进 §2.3): after a Block, a file asked for first when needed
  const insertGroup = (parent: string, afterId?: string | null) => {
    const groupId = randomId('g')
    void store.submit({ editId: `insert:${groupId}`, label: t('aiworkspace.canvas.insertGroup'), operations: [{ op: 'entity.create', entity_id: groupId, type_id: 'buckyos.container', parent_id: parent, order_key: siblingKey(store, parent, afterId), payload: { kind: 'group', layout: { mode: 'flow' }, title: t('aiworkspace.canvas.group') } }] })
  }
  const insertFlow = (entry: CatalogEntry, parent: string, afterId: string | null) => {
    const reason = insertReasonOf(entry.definition.type)
    if (reason) { store.notify('info', reason); return }
    if (entry.catalog.needs === 'file') {
      const input = document.createElement('input')
      input.type = 'file'
      if (entry.definition.type === 'asset') input.accept = 'image/*'
      input.onchange = () => { const file = input.files?.[0]; if (file) void insert({ entry, file }, null, parent, undefined, afterId) }
      input.click()
    } else if (entry.catalog.needs !== 'none') openCatalog(entry.group === 'extension' ? 'extension' : 'all', entry.key, null, parent, afterId)
    else void insert({ entry }, null, parent, undefined, afterId)
  }

  // ---- actions on the selection
  const selectedEntities = [...selection].map((id) => store.outline.get(id)).filter((e): e is EntityEnvelope => Boolean(e))
  const single = selectedEntities.length === 1 ? selectedEntities[0] : null
  /** What the selection's tools, inspector and editing act on: a group with a backing (a swimlane, a container) is
   * edited through its frame or shape (draw.io 导入 TODO §2.2). */
  const backingId = single?.kind === 'group' ? laid.get(single.entity_id)?.backingId : undefined
  const subject = (backingId ? store.outline.get(backingId) : undefined) ?? single
  /** The selected Blocks this user may move and remove (all of them with `structure` on the canvas). */
  const layoutable = topLevel(laid, selection).filter(canLayoutBlock)
  const selectedBlock = useBlockContext({
    cellId: subject?.type_id === 'buckyos.cell' ? subject.entity_id : single?.entity_id ?? null, mode, view: 'canvas', selected: true,
    editorActive: Boolean(subject && editing === subject.entity_id), zoom: camera.zoom,
    onActivate: () => { if (subject) setEditing(subject.entity_id) }, onDeactivate: () => setEditing(null),
  })
  /** Locked objects stay where they are (§7.2): skipped by delete, moves and grouping, with a note. */
  const unlocked = (ids: string[], what: 'delete' | 'group' | 'move') => {
    const kept = ids.filter((id) => !laid.get(id)?.locked)
    const note = { delete: 'aiworkspace.canvas.locked.notDeleted', group: 'aiworkspace.canvas.locked.notGrouped', move: 'aiworkspace.canvas.locked.notMoved' }[what]
    if (kept.length < ids.length) store.notify('info', t(note, { count: ids.length - kept.length }))
    return kept
  }
  const deleteSelection = async () => {
    const ops: Operation[] = []
    const doomed = new Set<string>()
    const targets = unlocked(topLevel(laid, selection).filter(canLayoutBlock), 'delete')
    if (targets.length === 0) return
    for (const id of targets) {
      if (store.outline.get(id)?.capabilities.includes('delete')) for (const e of [id, ...store.outline.descendants(id).map((d) => d.entity_id)]) doomed.add(e)
    }
    // the lines of deleted Blocks stay where they are now, still bound (连接线实现方案 §6.2, R3)
    try { ops.push(...await freezeOps(store, laid, doomed)) } catch { /* the lines fall back to their stored spot */ }
    for (const id of targets) {
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
        } else if (inContent && otherBlocks) store.notify('info', t('aiworkspace.canvas.delete.stillUsed', { name: source.title ?? source.name ?? source.entity_id }))
      }
    }
    if (ops.length === 0) return
    if (!ops.some((op) => op.op === 'entity.delete')) return
    const outcome = await store.submit({ editId: `delete:${randomId().slice(0, 8)}`, label: t('aiworkspace.canvas.delete.label', { count: ops.filter((o) => o.op === 'entity.delete').length }), operations: ops })
    if (outcome.status === 'rejected' && outcome.code === 'REFERENCE_BROKEN') {
      const data = outcome.errors?.[0]?.data as { referrers?: { entity_id: string }[]; hidden_referrers?: boolean } | undefined
      store.notify('error', t(data?.hidden_referrers ? 'aiworkspace.canvas.delete.referencedHidden' : 'aiworkspace.canvas.delete.referenced', { list: list((data?.referrers ?? []).map((r) => store.outline.get(r.entity_id)?.title ?? r.entity_id)) }))
    } else if (outcome.status === 'rejected') {
      store.notify('error', t('aiworkspace.canvas.delete.failed', { reason: outcome.detail ? t('aiworkspace.canvas.codeWithDetail', { code: outcome.code, detail: outcome.detail }) : outcome.code }))
    } else if (outcome.status === 'accepted' || outcome.status === 'saved_locally') {
      // only what was deleted leaves the selection (something picked meanwhile stays picked)
      setSelectionState((current) => { const next = new Set([...current].filter((id) => !doomed.has(id))); return next.size === current.size ? current : next })
    }
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
    if (ops.length) void store.submit({ editId: `lock:${randomId().slice(0, 8)}`, label: t(locked ? 'aiworkspace.canvas.lock.label' : 'aiworkspace.canvas.lock.unlockLabel', { count: ops.length }), operations: ops })
  }
  const reorder = (direction: 'front' | 'back') => {
    const ops: Operation[] = []
    for (const id of topLevel(laid, selection).filter(canLayoutBlock)) {
      const l = laid.get(id)
      if (!l) continue
      const siblings = store.outline.childrenOf(l.parentId)
      const key = direction === 'front' ? store.core.order_key_between(siblings.at(-1)?.order_key ?? undefined, undefined) : store.core.order_key_between(undefined, siblings[0]?.order_key ?? undefined)
      ops.push({ op: 'tree.place', entity_id: id, order_key: key })
    }
    if (ops.length) void store.submit({ editId: `order:${randomId().slice(0, 8)}`, label: t(direction === 'front' ? 'aiworkspace.canvas.order.front' : 'aiworkspace.canvas.order.back'), operations: ops })
  }
  const group = () => {
    const ids = unlocked(topLevel(laid, selection), 'group')
    if (ids.length < 2) return
    const bounds = boundsOf(laid, ids)
    if (!bounds) return
    const parentId = laid.get(ids[0])?.parentId ?? surfaceId
    const groupId = randomId('g')
    const rect = { x: bounds.x - 16, y: bounds.y - 40, w: bounds.w + 32, h: bounds.h + 56 }
    const ops: Operation[] = [{ op: 'entity.create', entity_id: groupId, type_id: 'buckyos.container', parent_id: parentId, order_key: store.core.order_key_between(store.outline.childrenOf(parentId).at(-1)?.order_key ?? undefined, undefined), placement: relativeTo(laid, parentId, rect), payload: { kind: 'group', layout: { mode: 'free' }, title: t('aiworkspace.canvas.group') } }]
    let key: string | undefined
    for (const id of ids) {
      const l = laid.get(id)!
      key = store.core.order_key_between(key, undefined)
      ops.push({ op: 'tree.move', entity_id: id, new_parent_id: groupId, order_key: key, placement: { x: Math.round(l.rect.x - rect.x), y: Math.round(l.rect.y - rect.y), w: l.rect.w, h: l.rect.h, ...(l.rotation ? { rotation: l.rotation } : {}) } })
    }
    void store.submit({ editId: `group:${groupId}`, label: t('aiworkspace.canvas.groupLabel', { count: ids.length }), operations: ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set([groupId])) })
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
      ops.push({ op: 'tree.move', entity_id: child.entity_id, new_parent_id: l.parentId, order_key: key, placement: placementOf(laid, cl, l.parentId) })
      moved.push(child.entity_id)
    }
    ops.push({ op: 'entity.delete', entity_id: single.entity_id, expect: { rev: single.life_rev } })
    void store.submit({ editId: `ungroup:${single.entity_id}`, label: t('aiworkspace.canvas.ungroup'), operations: ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set(moved)) })
  }
  const moveToSurface = (targetId: string) => {
    const target = store.outline.get(targetId)
    if (!target) return
    const ops: Operation[] = []
    let key = store.outline.childrenOf(targetId).at(-1)?.order_key
    const toFlow = target.layout?.mode !== 'free'
    const carried = (l: Laid) => [l.entity, ...store.outline.descendants(l.entity.entity_id)].some((e) => isConnector(e))
    for (const id of unlocked(topLevel(laid, selection), 'move')) {
      const l = laid.get(id)
      if (!l) continue
      if (toFlow && carried(l)) { store.notify('info', t('aiworkspace.canvas.flowConnectorsNotMoved')); continue }
      key = store.core.order_key_between(key, undefined)
      const placement = movedPlacement(laid, l, 0, 0, '')
      store.noteLayoutIntent(id, placement, key, targetId)
      ops.push({ op: 'tree.move', entity_id: id, new_parent_id: targetId, order_key: key, placement })
    }
    if (ops.length) void store.submit({ editId: `move-surface:${randomId().slice(0, 8)}`, label: t('aiworkspace.canvas.moveToCanvasLabel', { name: target.title ?? target.name ?? '' }), operations: ops }).then((outcome) => { if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set()) })
    setPicker(null)
  }
  // ---- connectors (连接线实现方案 §7): drawing one is one commit; an edit of one is one commit
  const createConnector = (spec: NewConnector, release: { blank: boolean; screen: { x: number; y: number } }) => {
    setTool('select')
    if (insertReason) { store.notify('info', insertReason); return }
    const { op, id } = createConnectorOp(store, laid, surfaceId, spec)
    void store.submit({ editId: `insert:${id}`, label: t('aiworkspace.canvas.insertConnector'), operations: [op] }).then((outcome) => {
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') {
        setSelectionState(new Set([id]))
        if (release.blank && policy.insert) setNextObject({ connectorId: id, at: release.screen, from: spec.start.point, end: spec.end.point })
      } else if (outcome.status === 'rejected') store.notify('error', t('aiworkspace.canvas.connectorCreateFailed', { reason: outcome.detail ? t('aiworkspace.canvas.codeWithDetail', { code: outcome.code, detail: outcome.detail }) : outcome.code }))
    })
  }
  const changeConnector = (id: string, change: ConnectorChange) => {
    const l = laid.get(id)
    if (!l) return
    void connectorChangeOps(store, laid, l, change).then(async (write) => {
      if (!write) return
      if (write.placement) store.noteLayoutIntent(id, write.placement)
      // the line shows its new shape at once; a refused commit puts the old one back
      store.outline.patchLocal(id, write.patch)
      const outcome = await store.submit({ editId: `line:${id}:${randomId().slice(0, 6)}`, label: write.label, operations: write.operations })
      if (outcome.status === 'rejected' || outcome.status === 'conflict') store.outline.patchLocal(id, write.before)
    }, (error: unknown) => store.notify('error', t('aiworkspace.canvas.connectorChangeFailed', { reason: describeError(error) })))
  }
  /** The "next object" picked at the end of a line drawn to blank space: created there, the line's end bound to it. */
  const insertAtLineEnd = (entry: CatalogEntry) => {
    const next = nextObject
    setNextObject(null)
    if (!next) return
    const size = entrySize(entry)
    const dir = axisOf(next.end.x - next.from.x, next.end.y - next.from.y)
    const spot = dir.x > 0 ? { x: next.end.x, y: next.end.y - size.h / 2 } : dir.x < 0 ? { x: next.end.x - size.w, y: next.end.y - size.h / 2 }
      : dir.y > 0 ? { x: next.end.x - size.w / 2, y: next.end.y } : { x: next.end.x - size.w / 2, y: next.end.y - size.h }
    void insert({ entry }, { x: Math.round(spot.x), y: Math.round(spot.y) }, undefined, async (cellId, rect) => {
      const line = store.outline.get(next.connectorId)
      if (!line || line.deleted) return []
      const read = await store.session.read<KeyedContent<CellPayload>>(next.connectorId)
      const revs = read.content.key_revs ?? {}
      // the end takes the new object's anchor facing the start (the side it was placed on)
      const form = formOf(entry.definition)
      const target = { rect, rotation: 0, shape: form.shape, anchors: form.anchors }
      const anchor = facingAnchor(target, next.from)
      if (!anchor) return []
      const end = anchorWorld(target, anchor, next.from)
      const box = boxOf(next.from, end)
      const flipped = Boolean(box.flip.h) !== Boolean(read.content.payload.flip?.h) || Boolean(box.flip.v) !== Boolean(read.content.payload.flip?.v)
      const placement = relativeTo(laid, surfaceId, box.rect, 0, 0)
      store.noteLayoutIntent(next.connectorId, placement)
      return [
        { op: 'entity.set_keys', entity_id: next.connectorId, keys: [
          { key: 'end', value: { entity_id: cellId, anchor }, expect: { rev: revs.end ?? 0 } },
          ...(flipped ? [{ key: 'flip', value: { ...(box.flip.h ? { h: true } : {}), ...(box.flip.v ? { v: true } : {}) }, expect: { rev: revs.flip ?? 0 } }] : []),
        ] },
        { op: 'tree.place', entity_id: next.connectorId, placement },
      ]
    })
  }

  const annotateSelection = () => {
    if (!single?.source_id) return
    const source = store.outline.get(single.source_id)
    if (!source) return
    ui.annotate?.({ target: { entity_id: source.entity_id }, label: source.title ?? source.name ?? source.entity_id })
  }

  // ---- draw.io import, dropped and pasted files (draw.io 导入 TODO §2.3, §2.4, §3)
  const importBusy = useRef(false)
  const importInput = useRef<HTMLInputElement>(null)
  const canCreateSurfaces = store.session.info().capabilities.includes('structure')
  const importReason = !isFree && !canCreateSurfaces ? t('aiworkspace.canvas.reason.noCreateCanvas') : insertReason
  const runImport = async (input: { name: string; bytes?: Uint8Array; text?: string }, at: { x: number; y: number } | null) => {
    if (importReason) { store.notify('info', importReason); return }
    if (importBusy.current) { store.notify('info', t('aiworkspace.canvas.import.busy')); return }
    importBusy.current = true
    const progress = store.notify('info', t('aiworkspace.canvas.import.progress', { name: input.name }))
    const c = camera.center
    try {
      // an import never lands on what is already there: it goes right of the canvas's objects instead
      const clear = (rect: { x: number; y: number; w: number; h: number }) => {
        const all = surfaceBounds(laid)
        const hit = [...laid.values()].some((l) => !l.isGroup && l.bounds.x < rect.x + rect.w && l.bounds.x + l.bounds.w > rect.x && l.bounds.y < rect.y + rect.h && l.bounds.y + l.bounds.h > rect.y)
        return hit && all ? { x: Math.round(all.x + all.w + 80), y: Math.round(all.y) } : { x: rect.x, y: rect.y }
      }
      const outcome = await importDrawio(store, input, { surfaceId, isFree, at, center: camera.toWorld(c.x, c.y), clear })
      if (outcome.surfaceId && outcome.surfaceId !== surfaceId) shell.selectSurface(outcome.surfaceId)
      else if (outcome.created.length) { setSelectionState(new Set(outcome.created)); if (outcome.bounds) camera.fit(outcome.bounds, 60) }
      const { report } = outcome
      if (outcome.error || report.notes.size > 0) importReports.show(outcome)
      else store.notify('info', t('aiworkspace.canvas.import.done', { name: outcome.name, pages: t('aiworkspace.drawio.count.pages', { count: report.pages }), objects: t('aiworkspace.drawio.count.objects', { count: report.objects }), lines: t('aiworkspace.drawio.count.lines', { count: report.lines }) }))
    } catch (error) {
      store.notify('error', t('aiworkspace.canvas.import.failed', { reason: describeError(error) }))
    } finally {
      importBusy.current = false
      store.dismissNotice(progress)
    }
  }
  const importCommand: Command = { reason: importReason, run: () => importInput.current?.click() }
  const exportCommand: Command = {
    reason: !isFree ? t('aiworkspace.canvas.export.onlyFree') : null,
    run: () => {
      void exportSurfaceDrawio(store, surface).then(({ xml, skipped }) => {
        const name = (surface.title ?? surface.name ?? t('aiworkspace.canvas.defaultName')).replace(/[\\/:*?"<>|]/g, '_')
        saveTextFile(`${name}.drawio`, xml)
        const lost = [...skipped.values()].reduce((a, b) => a + b, 0)
        store.notify('info', lost ? t('aiworkspace.canvas.export.doneLossy', { file: `${name}.drawio`, count: lost }) : t('aiworkspace.canvas.export.done', { file: `${name}.drawio` }))
      }, (error: unknown) => store.notify('error', t('aiworkspace.canvas.export.failed', { reason: describeError(error) })))
    },
  }
  /** A picture as an image Block (its natural size, at most 480 wide or high), centred on `at` or the view. */
  const insertImage = (file: File, bytes: Uint8Array, at: { x: number; y: number } | null) => {
    const entry = registryEntries().find((e) => e.definition.type === 'asset' && !e.variant)
    if (!entry) return
    const natural = imageSize(bytes, file.type) ?? entry.definition.defaultSize
    const nw = 'width' in natural ? natural.width : natural.w
    const nh = 'height' in natural ? natural.height : natural.h
    const k = Math.min(1, 480 / Math.max(nw, nh, 1))
    const size = { w: Math.max(24, Math.round(nw * k)), h: Math.max(24, Math.round(nh * k)) }
    void insert({ entry: { ...entry, size }, file }, at ? { x: Math.round(at.x - size.w / 2), y: Math.round(at.y - size.h / 2) } : null)
  }
  /** An SVG document as a recolourable SVG shape (§2.4); too large for a Block's configuration, it stays a picture. */
  const insertSvg = (text: string, at: { x: number; y: number } | null, file?: File) => {
    const parsed = svgShapeFromDocument(text)
    const asPicture = () => { const f = file ?? new File([text], 'image.svg', { type: 'image/svg+xml' }); insertImage(f, new TextEncoder().encode(text), at) }
    if (!parsed) { if (file) asPicture(); else store.notify('info', t('aiworkspace.canvas.svg.unreadable')); return }
    if (parsed.svg.body.length > 48 * 1024) { store.notify('info', t('aiworkspace.canvas.svg.tooLarge')); asPicture(); return }
    const entry = registryEntries().find((e) => e.definition.type === 'shape' && !e.variant)
    if (!entry) return
    const k = Math.min(1, 320 / Math.max(parsed.w, parsed.h, 1))
    const size = { w: Math.max(24, Math.round(parsed.w * k)), h: Math.max(24, Math.round(parsed.h * k)) }
    const config: Record<string, Json> = { shape: 'svg', svg: parsed.svg as unknown as Json, fill: parsed.fill, stroke: parsed.stroke ?? 'none', stroke_width: 1 }
    void insert({ entry: { ...entry, size, config } }, at ? { x: Math.round(at.x - size.w / 2), y: Math.round(at.y - size.h / 2) } : null)
  }
  const handleFiles = async (files: File[], at: { x: number; y: number } | null) => {
    let spot = at
    for (const file of files) {
      const { kind, bytes, text } = await classifyFile(file)
      if (kind === 'drawio') await runImport({ name: file.name, bytes }, spot)
      else if (insertReason) { store.notify('info', insertReason); return }
      else if (kind === 'svg' && text) insertSvg(text, spot, file)
      else if (kind === 'image') insertImage(file, bytes, spot)
      else store.notify('info', t('aiworkspace.canvas.unsupportedFile', { name: file.name }))
      spot = spot ? { x: spot.x + 24, y: spot.y + 24 } : null
    }
  }
  /** A library shape dragged from the shape library onto the canvas (§2.4). */
  const dropLibraryShape = async (data: string, at: { x: number; y: number } | null) => {
    const entry = registryEntries().find((e) => e.definition.type === 'shape' && !e.variant)
    let ref: { lib?: string; name?: string } = {}
    try { ref = JSON.parse(data) as typeof ref } catch { return }
    const shape = ref.lib ? (await stencilPack(ref.lib))?.shapes.find((s) => s.name === ref.name) : undefined
    if (!entry || !shape) return
    const { size, config } = libraryShapeConfig(shape)
    void insert({ entry: { ...entry, title: shape.title, size, config } }, at ? { x: Math.round(at.x - size.w / 2), y: Math.round(at.y - size.h / 2) } : null)
  }
  /** The canvas clipboard leaves a marker in the system clipboard: a later paste of it is ours, anything else is a
   * file, a picture, an SVG or a draw.io model from outside. */
  const copyMarker = useRef<string | null>(null)
  const pendingCopy = useRef<string | null>(null)
  const pendingPaste = useRef(false)

  // ---- clipboard (UI improvement §6.4)
  const copy: Command = {
    reason: !policy.select ? t('aiworkspace.canvas.reason.cannotSelect') : selection.size === 0 ? t('aiworkspace.canvas.reason.noSelection') : null,
    run: () => {
      void copyToClipboard(store, surfaceId, laid, selection, 'copy').then(
        (n) => { if (n > 0) store.notify('info', t('aiworkspace.canvas.clipboard.copied', { count: n })) },
        (error: unknown) => store.notify('error', t('aiworkspace.canvas.clipboard.copyFailed', { reason: describeError(error) })))
    },
  }
  const cut: Command = {
    reason: !canLayout ? t(policy.layout ? 'aiworkspace.canvas.reason.noLayoutPermission' : 'aiworkspace.canvas.reason.cannotMove') : selection.size === 0 ? t('aiworkspace.canvas.reason.noSelection') : null,
    run: () => { void copyToClipboard(store, surfaceId, laid, selection, 'cut').then((n) => { if (n > 0) store.notify('info', t('aiworkspace.canvas.clipboard.cut', { count: n })) }) },
  }
  const pasteAt = (at: { x: number; y: number } | null) => {
    const current = canvasClipboard.snapshot()
    if (!current) return
    if (current.workspaceId !== store.session.workspaceId) { store.notify('info', t('aiworkspace.canvas.clipboard.crossWorkspace')); return }
    const c = camera.center
    const { operations, newIds, label, refused } = pasteOperations(store, current, { surfaceId, isFree, at, center: camera.toWorld(c.x, c.y) })
    if (refused) { store.notify('info', refused); return }
    if (operations.length === 0) { store.notify('info', t('aiworkspace.canvas.clipboard.cutGone')); canvasClipboard.set(null); return }
    void store.submit({ editId: `paste:${randomId().slice(0, 8)}`, label, operations }).then((outcome) => {
      if (outcome.status === 'accepted' || outcome.status === 'saved_locally') {
        setSelectionState(new Set(newIds))
        if (current.kind === 'cut') canvasClipboard.set(null)
        else canvasClipboard.notePaste()
      } else if (outcome.status === 'rejected' || outcome.status === 'conflict') {
        store.notify('error', t(current.kind === 'cut' ? 'aiworkspace.canvas.clipboard.pasteFailedCut' : 'aiworkspace.canvas.clipboard.pasteFailed', { code: outcome.code }))
      }
    })
  }
  const paste: Command = {
    reason: !clip ? t('aiworkspace.canvas.clipboard.empty') : clip.workspaceId !== store.session.workspaceId ? t('aiworkspace.canvas.clipboard.otherWorkspace') : insertReason ?? (canLayout ? null : t('aiworkspace.canvas.reason.noLayoutPermission')),
    run: () => pasteAt(null),
  }
  const remove: Command = { reason: !policy.layout ? t('aiworkspace.canvas.reason.cannotDelete') : selection.size === 0 ? t('aiworkspace.canvas.reason.noSelection') : layoutable.length === 0 ? t('aiworkspace.canvas.reason.noLayoutPermission') : null, run: () => { void deleteSelection() } }

  const fitAll = () => { const b = surfaceBounds(laid); if (b) camera.fit(b); else camera.set({ x: -camera.clearArea.x, y: -camera.clearArea.y, zoom: 1 }) }
  const fitSelection = () => { const b = boundsOf(laid, selection); if (b) camera.fit(b, 80) }

  // ---- keyboard (§8.2; UI improvement §7.2, §11): Esc exits layer by layer; shortcuts need the canvas focus and never take text input
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      // a guide owns the keys while it runs (第三期规划 §10.4)
      if (shell.guide) return
      if (event.isComposing || target?.closest('input, textarea, select, [contenteditable="true"], [data-role="editor"]')) return
      // a placement or target pick started from a toolbar button is cancelled by Esc even while that button has the focus
      if (event.key === 'Escape' && (placing || annotatePick || tool === 'connector') && !target?.closest('.aiws-popover, .aiws-dialog, .aiws-menu')) { setPlacing(null); setAnnotatePick(false); setTool((t) => (t === 'connector' ? 'select' : t)); return }
      if (target?.closest('.aiws-panel, .aiws-popover, .aiws-dialog, .aiws-menu, .aiws-side, .aiws-near')) return
      if (!hostRef.current?.contains(target) && target !== document.body) return
      const mod = event.ctrlKey || event.metaKey
      if (event.key === 'Escape') {
        if (nextObject) { setNextObject(null); return }
        if (tool === 'connector') { setTool('select'); return }
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
      const typable = subject && !editing && policy.editContent && (subject.view_type === 'richtext' || subject.view_type === 'note' || subject.view_type === 'shape') && !laid.get(subject.entity_id)?.locked
      // keys typed before the editor that typing opened has mounted (its Block still loading) are passed on to it
      const typed = typeAhead.get('typed')
      if (editing && typed?.id === editing && !mod && !event.altKey && event.key.length === 1) {
        event.preventDefault()
        typeAhead.set('typed', { id: editing, text: typed.text + event.key })
        requestIntent(editing, `type:${typed.text}${event.key}`)
        return
      }
      if (typable && !mod && !event.altKey && event.key.length === 1 && event.key !== ' ') {
        event.preventDefault()
        requestIntent(subject.entity_id, `type:${event.key}`)
        setEditing(subject.entity_id)
        typeAhead.set('typed', { id: subject.entity_id, text: event.key })
        return
      }
      if (!mod && !event.altKey && isFree && (event.key === 'v' || event.key === 'V')) { setTool('select'); setPlacing(null); return }
      if (!mod && !event.altKey && isFree && (event.key === 'h' || event.key === 'H')) { setTool('hand'); setPlacing(null); return }
      if (!mod && !event.altKey && isFree && (event.key === 'l' || event.key === 'L') && insertReason === null) { setTool('connector'); setPlacing(null); return }
      if (mod && !event.altKey && !event.shiftKey) {
        const key = event.key.toLowerCase()
        // copy and cut let the browser's copy event run (it writes the marker); a paste waits for the paste event,
        // which brings outside content, and falls back to the canvas clipboard when there is none
        if (key === 'c' && copy.reason === null) { pendingCopy.current = `aiws-clipboard:${randomId()}`; copy.run(); return }
        if (key === 'x' && cut.reason === null) { pendingCopy.current = `aiws-clipboard:${randomId()}`; cut.run(); return }
        if (key === 'v') {
          pendingPaste.current = paste.reason === null
          window.setTimeout(() => { if (pendingPaste.current) { pendingPaste.current = false; paste.run() } }, 0)
          return
        }
      }
      if (!policy.select) return
      if ((event.key === 'Delete' || event.key === 'Backspace') && remove.reason === null) { event.preventDefault(); remove.run(); return }
      if (event.key === 'Enter' && subject && policy.editContent && !editing) { event.preventDefault(); requestIntent(subject.entity_id, 'end'); setEditing(subject.entity_id); return }
      // a flow page (流式页交互改进 §2.4): ↑ ↓ walk the Blocks, Ctrl+Shift+↑ ↓ move the selected one
      if (!isFree && (event.key === 'ArrowUp' || event.key === 'ArrowDown') && single) {
        event.preventDefault()
        const direction = event.key === 'ArrowUp' ? -1 : 1
        if (mod && event.shiftKey) { if (canLayoutBlock(single.entity_id)) moveInFlow(store, single, direction); return }
        const order = flowOrder(store, surfaceId)
        const next = order[order.indexOf(single.entity_id) + direction]
        if (next) setSelectionState(new Set([next]))
        return
      }
      if (event.key.startsWith('Arrow') && layoutable.length > 0 && isFree) {
        event.preventDefault()
        const step = event.shiftKey ? 10 : 1
        const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0
        const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0
        // a line with a bound end moves only by its ends (连接线实现方案 §7)
        commitLayout(layoutable.flatMap((id) => { const l = laid.get(id); return l && !l.locked && !hasBoundEnd(l) ? [{ id, placement: movedPlacement(laid, l, dx, dy) }] : [] }))
        return
      }
      if (mod && (event.key === ']' || event.key === '[') && layoutable.length > 0) { event.preventDefault(); reorder(event.key === ']' ? 'front' : 'back'); return }
      if (mod && event.key.toLowerCase() === 'g' && canLayout) { event.preventDefault(); if (event.shiftKey) ungroup(); else group(); return }
      if (event.key === 'F2' && single && policy.layout) { event.preventDefault(); shell.setSide('inspector') }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })
  useEffect(() => {
    const outside = (target: EventTarget | null) => {
      const el = target instanceof Element ? target : null
      if (el?.closest('input, textarea, select, [contenteditable="true"], [contenteditable="plaintext-only"], [data-role="editor"], .aiws-panel, .aiws-popover, .aiws-dialog, .aiws-menu, .aiws-side, .aiws-near')) return true
      return !hostRef.current?.contains(el) && el !== document.body
    }
    const onCopy = (event: ClipboardEvent) => {
      const marker = pendingCopy.current
      pendingCopy.current = null
      if (!marker || outside(event.target) || !event.clipboardData) return
      event.clipboardData.setData('text/plain', marker)
      event.preventDefault()
      copyMarker.current = marker
    }
    const onPaste = (event: ClipboardEvent) => {
      if (outside(event.target) || !event.clipboardData) return
      const data = event.clipboardData
      const files = [...data.files]
      const text = data.getData('text/plain')
      const svg = /^\s*(<\?xml[^>]*>\s*)?(<!--[\s\S]*?-->\s*)*<svg[\s>]/i.test(text)
      const foreign = files.length > 0 || (text !== '' && text !== copyMarker.current && (looksLikeDrawio(text) || svg))
      if (!foreign) {
        if (pendingPaste.current) { pendingPaste.current = false; event.preventDefault(); paste.run() }
        return
      }
      event.preventDefault()
      pendingPaste.current = false
      if (files.length > 0) void handleFiles(files, null)
      else if (looksLikeDrawio(text)) void runImport({ name: t('aiworkspace.canvas.import.pastedName'), text }, null)
      else if (insertReason) store.notify('info', insertReason)
      else insertSvg(text, null)
    }
    window.addEventListener('copy', onCopy)
    window.addEventListener('cut', onCopy)
    window.addEventListener('paste', onPaste)
    return () => { window.removeEventListener('copy', onCopy); window.removeEventListener('cut', onCopy); window.removeEventListener('paste', onPaste) }
  })

  // ---- near toolbar (§9.2; 标准对象的交互改进 §5): the Editor's tools while editing → the type's tools → common
  // tools (annotate, lock, AI) → "more" (the same list as the context menu)
  const moreActions: NearAction[] = []
  const typeItems: ToolbarItem[] = []
  const commonItems: ToolbarItem[] = []
  const topIds = topLevel(laid, selection)
  const lockable = topIds.length > 0 && topIds.every((id) => canLayoutBlock(id) && store.outline.get(id)?.capabilities.includes('update'))
  const allLocked = topIds.length > 0 && topIds.every((id) => laid.get(id)?.locked)
  const lockedHere = topIds.every((id) => store.outline.get(id)?.locked)
  const singleLocked = single ? Boolean(laid.get(single.entity_id)?.locked) : false
  const wishEntry = registryEntries().find((entry) => entry.definition.type === 'wish' && entry.catalog.standard)
  if (single && policy.select) {
    const context = selectedBlock.context
    const definition = selectedBlock.resolution?.ok ? context?.definition : undefined
    const source = subject?.source_id ? store.outline.get(subject.source_id) : undefined
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
          const run = () => { void Promise.resolve().then(() => action.run(context, store)).catch((error: unknown) => store.notify('error', t('aiworkspace.canvas.extensionActionFailed', { reason: describeError(error) }))) }
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
    if (single.kind === 'group' && canLayout && !singleLocked) typeItems.push({ kind: 'button', id: 'ungroup', icon: Ungroup, label: t('aiworkspace.canvas.ungroup'), key: 'Ctrl+Shift+G', run: ungroup })
    if (definition?.Inspector || definition?.configFields || single.kind === 'group') moreActions.push({ id: 'inspector', label: t('aiworkspace.canvas.inspector.title'), key: 'F2', run: () => shell.setSide('inspector') })
    if (policy.annotate && source && ui.annotate) commonItems.push({ kind: 'button', id: 'annotate', icon: MessageSquarePlus, label: t('aiworkspace.canvas.annotate.label'), run: annotateSelection })
    if (source) moreActions.push({ id: 'relations', label: t('aiworkspace.canvas.relations'), run: () => shell.setSide('relations') })
    if (source && canLayout && !singleLocked) moreActions.push({ id: 'add-view', label: t('aiworkspace.canvas.addView'), run: () => setPicker({ kind: 'add-view' }) })
    if (single.kind === 'group' && canLayout && !singleLocked) moreActions.push({ id: 'ungroup', label: t('aiworkspace.canvas.ungroup'), key: 'Ctrl+Shift+G', run: ungroup })
  }
  if (selection.size > 1 && canLayout && mode === 'edit') typeItems.push({ kind: 'button', id: 'group', icon: Group, label: t('aiworkspace.canvas.group'), key: 'Ctrl+G', run: group })
  if (selection.size > 1 && canLayout) moreActions.push({ id: 'group', label: t('aiworkspace.canvas.group'), key: 'Ctrl+G', run: group })
  if (lockable && mode === 'edit') commonItems.push(allLocked
    ? { kind: 'button', id: 'unlock', icon: LockOpen, label: t('aiworkspace.canvas.lock.unlock'), disabled: lockedHere ? false : t('aiworkspace.canvas.lock.groupLocked'), run: () => { void setLocked(topIds, false) } }
    : { kind: 'button', id: 'lock', icon: Lock, label: t('aiworkspace.canvas.lock.lock'), run: () => { void setLocked(topIds, true) } })
  if (wishEntry && selection.size > 0 && policy.insert && insertReason === null && !allLocked) commonItems.push({ kind: 'button', id: 'ai', icon: Sparkles, label: t(single?.view_type === 'wish' ? 'aiworkspace.canvas.wish.open' : 'aiworkspace.canvas.wish.forSelection'), ai: true, run: () => pickEntry(wishEntry) })
  if (selection.size > 0 && policy.select && !phone) {
    moreActions.push({ id: 'copy', label: t('aiworkspace.canvas.clipboard.copy'), key: 'Ctrl+C', run: copy.run })
    if (cut.reason === null && !allLocked) moreActions.push({ id: 'cut', label: t('aiworkspace.canvas.clipboard.cutAction'), key: 'Ctrl+X', run: cut.run })
  }
  if (layoutable.length > 0 && !allLocked) {
    moreActions.push({ id: 'front', label: t('aiworkspace.canvas.order.front'), key: 'Ctrl+]', run: () => reorder('front') }, { id: 'back', label: t('aiworkspace.canvas.order.back'), key: 'Ctrl+[', run: () => reorder('back') })
    if (canLayout && surfacesOf(store).length > 1) moreActions.push({ id: 'move-surface', label: t('aiworkspace.canvas.moveToCanvasMenu'), run: () => setPicker({ kind: 'move-surface' }) })
    moreActions.push({ id: 'delete', label: t('aiworkspace.common.delete'), key: 'Delete', run: () => { void deleteSelection() } })
  }
  // a locked object offers "unlock" and reading only (§4.3): no type tools, nothing that moves or removes it
  const editorItems = subject && editing === subject.entity_id ? toolbarSink.items() : []
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
    ...(insertReason === null ? quickEntries.map((entry) => ({ id: `insert-${entryId(entry)}`, label: t('aiworkspace.canvas.insertNamed', { name: entry.title }), run: () => { if (entry.definition.type === 'note') addNote(menu.world, menu.parentId); else void insert({ entry }, menu.world, menu.parentId) } })) : []),
    ...(insertReason !== null && noteReason === null && (isFree || policy.insert) ? [{ id: 'insert-note', label: t(policy.insert ? 'aiworkspace.canvas.menu.insertNote' : 'aiworkspace.canvas.menu.addNote'), run: () => addNote(menu.world, menu.parentId) }] : []),
    ...(insertReason === null && !isFree ? [{ id: 'insert-group', label: t('aiworkspace.canvas.insertGroup'), run: () => insertGroup(menu.parentId ?? surfaceId) }] : []),
    ...(insertReason === null ? [
      { id: 'catalog', label: t('aiworkspace.canvas.menu.insertObject'), run: () => openCatalog('all', undefined, menu.world, menu.parentId) },
      { id: 'add-data', label: t('aiworkspace.canvas.menu.addExisting'), run: () => openCatalog('existing', undefined, menu.world, menu.parentId) },
    ] : []),
    ...(paste.reason === null ? [{ id: 'paste', label: t('aiworkspace.canvas.clipboard.paste'), run: () => pasteAt(menu.world) }] : []),
    ...(isFree ? [{ id: 'sep', label: '', run: () => undefined, separator: true }, { id: 'fit', label: t('aiworkspace.canvas.menu.fitAll'), run: fitAll }] : []),
  ].filter((item, i) => !(i === 0 && 'separator' in item))) : []

  const commands: CanvasCommands = {
    isFree, insertReason, noteReason: toolNoteReason,
    insert: (entry) => {
      if (entry.definition.type === 'connector' && isFree) pickEntry(entry)
      else if (entry.catalog.needs === 'none') void insert({ entry }, null)
      else openCatalog(entry.group === 'extension' ? 'extension' : 'all', entry.key)
    },
    openCatalog: (tab, key) => openCatalog(tab, key),
    copy, cut, paste, remove,
    importDrawio: importCommand,
    exportDrawio: exportCommand,
    view: isFree ? { camera, fitAll, fitSelection, hasSelection: selection.size > 0 } : null,
    canvasTabs: CANVAS_SIDE_TABS,
  }
  const annotateCommand = {
    reason: !canComment ? t('aiworkspace.canvas.annotate.noPermission') : !policy.annotate ? t('aiworkspace.canvas.annotate.notInMode') : null,
    active: annotatePick,
    run: () => {
      if (single?.source_id) { annotateSelection(); return }
      setAnnotatePick((value) => !value)
      shell.setSide('annotations')
    },
    // a free canvas also takes notes from here (they are annotations)
    note: isFree ? { reason: noteReason, run: () => addNote(null) } : null,
  }
  const sideContent = (tab: SideTab) => {
    switch (tab) {
      case 'inspector': return subject ? <SelectionInspector cell={subject} mode={mode} context={selectedBlock.context} resolved={Boolean(selectedBlock.resolution?.ok)} registryVersion={selectedBlock.registryVersion} /> : <div className="aiws-muted">{t('aiworkspace.canvas.side.noSelection')}</div>
      case 'relations': return single?.source_id ? <RelationsPanel entityId={single.source_id} /> : <div className="aiws-muted">{t('aiworkspace.canvas.side.noDataSelection')}</div>
      case 'annotations': return <AnnotationsPanel parentId={surface.content_folder_id ?? null} noteDraft={noteDraft ? { save: saveNoteDraft, cancel: () => setNoteDraft(null) } : null} />
      case 'collab': return <PermissionsPanel />
      case 'edits': return <StatusDetail />
      case 'wish': return wishOpen && store.outline.get(wishOpen.wishId)
        ? <WishPanel key={wishOpen.wishId} wishId={wishOpen.wishId} cellId={wishOpen.cellId} readOnly={readOnlyNow || !policy.writes} canvasSelection={[...selection].filter((id) => id !== wishOpen.cellId)} />
        : <div className="aiws-muted">{t('aiworkspace.canvas.side.noWish')}</div>
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
          <div ref={mainRef} className={`aiws-canvas-main${shell.prefs.grid ? '' : ' no-grid'}${showObjectToolbar && !collapsed ? ' has-left-tools' : ''}`}
            onDragOver={(event) => {
              const types = event.dataTransfer.types
              if ((types.includes('Files') && (insertReason === null || importReason === null)) || (types.includes(SHAPE_DRAG_TYPE) && insertReason === null)) { event.preventDefault(); event.dataTransfer.dropEffect = 'copy' }
            }}
            onDrop={(event) => {
              const r = mainRef.current?.getBoundingClientRect()
              const at = isFree && r ? camera.toWorld(event.clientX - r.left, event.clientY - r.top) : null
              const libraryDrop = event.dataTransfer.getData(SHAPE_DRAG_TYPE)
              if (libraryDrop) { event.preventDefault(); void dropLibraryShape(libraryDrop, at); return }
              const files = [...event.dataTransfer.files]
              if (files.length === 0) return
              event.preventDefault()
              void handleFiles(files, at)
            }}>
            {isFree ? (
              <EditorToolbarContext.Provider value={toolbarSink}>
              <OpenWishContext.Provider value={shell.side === 'wish' ? wishOpen?.cellId ?? null : null}>
              <RenderHost surfaceId={surfaceId} mode={mode} camera={camera} laid={laid} index={index} selection={selection} onSelectionChange={setSelection} editing={editing} onEditingChange={setEditing}
                canLayout={canLayoutBlock} onCommitLayout={commitLayout} onOpenEntity={ui.openEntity} gesturesPaused={Boolean(menu || picker || catalog || overlayOpen)}
                tool={tool} placing={placing ? { ...entrySize(placing.entry), label: placingTitle } : null}
                onPlace={(world) => {
                  const request = placing
                  setPlacing(null)
                  // the tool button keeps no focus: typing goes to the placed object (标准对象的交互改进 §4.4)
                  if (document.activeElement instanceof HTMLElement && document.activeElement.closest('.aiws-panel')) document.activeElement.blur()
                  if (request) void insert(request, world)
                }}
                onContextMenu={(point, blockId) => { if (!policy.select && !policy.insert) return; setMenu({ at: { x: point.screenX, y: point.screenY }, world: { x: point.worldX, y: point.worldY }, blockId }) }}
                cutIds={cutIds} renderNear={(bbox) => <NearToolbar bbox={bbox} items={nearItems} more={moreActions} viewport={viewportSize} insets={insets} />}
                background={surfaceLook.background} formOf={(id) => shapeBook.get(store.outline.get(id))} onConnectorCreate={policy.insert && insertReason === null ? createConnector : undefined} onConnectorChange={changeConnector} />
              </OpenWishContext.Provider>
              </EditorToolbarContext.Provider>
            ) : (
              <div className="aiws-flow-host" data-testid="aiws-flow-host">
                <FlowSurface surfaceId={surfaceId} mode={mode} selected={selection} onSelect={setSelection}
                  onInsertMenu={(parentId, at) => setMenu({ at, world: null, blockId: null, parentId })} insertAt={insertFlow} insertGroup={insertGroup}
                  onInspect={(id) => { setSelection(new Set([id])); shell.setSide('inspector') }} />
              </div>
            )}
            {mode === 'presentation_edit' && (isFree
              ? <PathEditor surface={surface} camera={camera} laid={laid} />
              : (
                <div className="aiws-presentation-placeholder" role="status" data-testid="aiws-presentation-placeholder">
                  <b>{t('aiworkspace.canvas.presentation.flowNoPath')}</b>
                  <div>{t('aiworkspace.canvas.presentation.flowNoPathNote')}</div>
                </div>
              ))}
            {shell.guide && <GuideOverlay key={shell.guide.pathId} camera={camera} surfaceId={surfaceId} />}
            <div className="aiws-chrome-top">
              {!phone && <MainToolbar canvas={commands} />}
              {(shell.prefs.viewToolbar || phone) && <ViewToolbar camera={isFree ? camera : null} hasSelection={selection.size > 0} onFitAll={fitAll} onFitSelection={fitSelection} annotate={annotateCommand} />}
            </div>
            {showObjectToolbar && (
              <div className="aiws-chrome-left">
                <ObjectToolbar isFree={isFree} tool={tool} onTool={(next) => { setTool(next); setPlacing(null) }} placingKey={placing?.entry.key ?? (tool === 'connector' ? 'block:connector' : null)} insertReason={insertReason} noteReason={toolNoteReason}
                  onPick={pickEntry} onPickFile={(entry, file) => pick({ entry, file })} onOpenCatalog={(tab, key) => openCatalog(tab, key)} collapsed={collapsed} onCollapsed={setCollapsed} />
              </div>
            )}
            {(placing || annotatePick || tool === 'connector') && (
              <div className="aiws-hint" role="status" data-testid="aiws-tool-hint">
                {placing ? t('aiworkspace.canvas.hint.place', { name: placingTitle }) : tool === 'connector' ? t('aiworkspace.canvas.hint.connector')
                  : t(phone ? 'aiworkspace.canvas.hint.annotatePhone' : 'aiworkspace.canvas.hint.annotate')}
              </div>
            )}
            {nextObject && <ContextMenu at={nextObject.at} onClose={() => setNextObject(null)}
              items={NEXT_OBJECTS.flatMap((type) => { const entry = registryEntries().find((e) => e.definition.type === type && !e.variant); return entry ? [{ id: `next-${type}`, label: entry.title, run: () => insertAtLineEnd(entry) }] : [] })} />}
            <StatusDock />
            <input ref={importInput} type="file" hidden aria-hidden="true" tabIndex={-1} accept=".drawio,.xml,.png,.svg,application/xml,text/xml,image/png,image/svg+xml" data-testid="aiws-import-drawio-file"
              onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void file.arrayBuffer().then((buffer) => runImport({ name: file.name, bytes: new Uint8Array(buffer) }, null)) }} />
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
              <InsertCatalog initialTab={catalog.tab} initialKey={catalog.key} canPlace={isFree} onClose={() => setCatalog(null)} onImport={importReason === null ? () => { setCatalog(null); importCommand.run() } : undefined}
                onImportAssets={insertReason === null ? async (files, folder, signal) => {
                  try {
                    const items = await store.session.importAssets(files, folder, signal)
                    signal.throwIfAborted()
                    const spot = catalog.world ?? centreSpot({ w: 320, h: 240 })
                    const operations = importedAssetOps(store, surface.content_folder_id!, items, { parentId: catalog.parentId ?? surfaceId, free: isFree, ...spot })
                    const result = await store.submit({ editId: randomId('import'), label: t('aiworkspace.data.tree.importFiles'), operations })
                    if (result.status === 'accepted' || result.status === 'saved_locally') setCatalog(null)
                  } catch (error) {
                    if (!signal.aborted) store.notify('error', t('aiworkspace.data.tree.uploadFailed', { error: describeError(error) }))
                  }
                } : undefined} onInsert={(request, how) => {
                const at = catalog.world
                const parentId = catalog.parentId
                setCatalog(null)
                if (how === 'place') pick(request)
                else void insert(request, at, parentId, undefined, catalog.afterId)
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
  const { t, tNode } = useAiwsText()
  const live = context?.cell ?? cell
  const definition = resolved ? context?.definition : undefined
  const source = context?.source
  const payload = context?.payload
  const keyRevs = context?.keyRevs ?? {}
  const readOnly = !context || context.readOnlyReason !== null
  const setConfig = (key: string, value: Json) => {
    if (!payload || readOnly) return
    const config = { ...(payload.config ?? {}), [key]: value }
    void store.submit({ editId: `config:${cell.entity_id}`, label: t('aiworkspace.canvas.inspector.configLabel', { key }), operations: [{ op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'config', value: config as Json, expect: { rev: keyRevs.config ?? 0 } }] }] })
  }
  const setTitle = (title: string) => {
    if (!payload || readOnly || title === (payload.title ?? '')) return
    void store.submit({ editId: `key:${cell.entity_id}:title`, label: title ? t('aiworkspace.canvas.blockTitleLabel', { title }) : t('aiworkspace.canvas.blockTitleClearedLabel'), operations: [title ? { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: title, expect: { rev: keyRevs.title ?? 0 } }] } : { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev: keyRevs.title ?? 0 } }] }] })
  }
  const Inspector = definition?.Inspector
  const operable = cell.view_type !== 'frame' && cell.view_type !== 'connector' && cell.type_id === 'buckyos.cell'
  return (
    <BlockInspector cellId={cell.entity_id} mode={mode}>
      {operable && payload && (
        <label className="aiws-inline-form" title={t('aiworkspace.canvas.inspector.liveTip')}>
          <input type="checkbox" data-testid="aiws-inspector-live" checked={payload.presentation?.live === true} disabled={readOnly}
            onChange={(event) => { void setLive(store, cell.entity_id, event.target.checked) }} />{t('aiworkspace.canvas.inspector.live')}
        </label>
      )}
      {payload && (
        <label className="aiws-inline-form">{t('aiworkspace.canvas.inspector.titleField')} <input aria-label={t('aiworkspace.canvas.blockTitle')} defaultValue={payload.title ?? ''} disabled={readOnly} onBlur={(event) => setTitle(event.target.value.trim())} /></label>
      )}
      {source && <div className="aiws-muted">{tNode('aiworkspace.canvas.inspector.source', { name: <button type="button" className="aiws-link" onClick={() => ui.openEntity(source.entity_id)}>{source.title ?? source.name ?? source.entity_id}</button>, type: source.type_id.replace('buckyos.', '') })}</div>}
      {definition?.configFields && payload && definition.configFields.map((field) => (
        <label key={field.key} className="aiws-inline-form">{field.label}
          {field.kind === 'select'
            ? <select value={String(payload.config?.[field.key] ?? field.options?.[0]?.value ?? '')} disabled={readOnly} onChange={(event) => setConfig(field.key, event.target.value)}>{(field.options ?? []).map((o) => <option key={o.value} value={o.value}>{o.label}</option>)}</select>
            : field.kind === 'boolean'
              ? <input type="checkbox" checked={Boolean(payload.config?.[field.key])} disabled={readOnly} onChange={(event) => setConfig(field.key, event.target.checked)} />
              : <input type={field.kind === 'color' ? 'color' : field.kind === 'number' ? 'number' : 'text'} value={String(payload.config?.[field.key] ?? (field.kind === 'color' ? '#4f8df7' : ''))} disabled={readOnly} onChange={(event) => setConfig(field.key, field.kind === 'number' ? Number(event.target.value) : event.target.value)} />}
        </label>
      ))}
      {payload && cell.type_id === 'buckyos.cell' && (
        <MetadataForm config={payload.config ?? {}} readOnly={readOnly} onChange={(patch, label) => { if (!readOnly && context) setCellConfig(context, store, patch, label) }} />
      )}
      {Inspector && context && (
        <BlockBoundary resetKey={`${cell.entity_id}:${mode}:${live.content_rev}:${registryVersion}`} fallback={(error, reset) => (
          <div className="aiws-block-fallback" data-testid="aiws-inspector-error"><div className="aiws-error" role="alert">{t('aiworkspace.canvas.inspector.error', { reason: describeError(error) })}</div><button type="button" onClick={reset}>{t('aiworkspace.common.retry')}</button></div>
        )}>
          <Inspector {...context} />
        </BlockBoundary>
      )}
      <details className="aiws-details">
        <summary>{t('aiworkspace.canvas.inspector.details')}</summary>
        {live.placement && <div className="aiws-muted">{t('aiworkspace.canvas.inspector.placement', { x: live.placement.x, y: live.placement.y, w: live.placement.w, h: live.placement.h, order: live.order_key ?? '' })}</div>}
      </details>
    </BlockInspector>
  )
}

function Picker({ kind, selection, surfaceId, onClose, onPick }: { kind: 'add-view' | 'move-surface'; selection: EntityEnvelope[]; surfaceId: string; onClose: () => void; onPick: (id: string) => void }) {
  const store = useStore()
  const { t } = useAiwsText()
  const [chosen, setChosen] = useState('')
  const single = selection[0]
  const source = single?.source_id ? store.outline.get(single.source_id) : undefined
  const renderers = kind === 'add-view' && source ? blockRegistry.forSource(source.type_id).filter((def) => def.create && def.catalog && def.catalog.needs !== 'definition') : []
  const targets = surfacesOf(store).filter((s) => s.entity_id !== surfaceId)
  return (
    <div className="aiws-modal-backdrop" onPointerDown={(event) => { if (event.target === event.currentTarget) onClose() }}>
      <div className="aiws-dialog" role="dialog" aria-modal="true" aria-label={t(kind === 'add-view' ? 'aiworkspace.canvas.picker.addView' : 'aiworkspace.canvas.picker.moveToCanvas')} data-testid={`aiws-picker-${kind}`} onKeyDown={(event) => { if (event.key === 'Escape') onClose() }}>
        {kind === 'add-view' && (
          <>
            <b>{t('aiworkspace.canvas.picker.addViewTitle', { name: source?.title ?? source?.name ?? '' })}</b>
            <div className="aiws-muted">{t('aiworkspace.canvas.picker.addViewNote')}</div>
            <select aria-label={t('aiworkspace.canvas.catalog.renderer')} data-testid="aiws-picker-renderer" value={chosen} onChange={(event) => setChosen(event.target.value)}><option value="">{t('aiworkspace.canvas.picker.chooseRenderer')}</option>{renderers.map((d) => <option key={d.type} value={d.type}>{d.title}</option>)}</select>
            <div className="aiws-dialog-actions"><button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!chosen} onClick={() => onPick(chosen)}>{t('aiworkspace.canvas.picker.add')}</button><button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button></div>
          </>
        )}
        {kind === 'move-surface' && (
          <>
            <b>{t('aiworkspace.canvas.picker.moveTitle', { count: selection.length })}</b>
            <div className="aiws-muted">{t('aiworkspace.canvas.picker.moveNote')}</div>
            <select aria-label={t('aiworkspace.canvas.picker.targetCanvas')} data-testid="aiws-picker-surface" value={chosen} onChange={(event) => setChosen(event.target.value)}><option value="">{t('aiworkspace.canvas.picker.chooseCanvas')}</option>{targets.map((s) => <option key={s.entity_id} value={s.entity_id}>{s.title ?? s.name ?? s.entity_id}</option>)}</select>
            <div className="aiws-dialog-actions"><button type="button" className="is-primary" data-testid="aiws-picker-confirm" disabled={!chosen} onClick={() => onPick(chosen)}>{t('aiworkspace.canvas.picker.move')}</button><button type="button" onClick={onClose}>{t('aiworkspace.common.cancel')}</button></div>
          </>
        )}
      </div>
    </div>
  )
}
