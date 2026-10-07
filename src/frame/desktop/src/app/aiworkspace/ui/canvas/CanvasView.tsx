/* CanvasView (phase two §5, §8, §10.2): Surface navigation, the canvas sub-mode controller
 * (edit / view / presentation-edit placeholder), selection, keyboard, near tools, insertion,
 * grouping, cross-Surface moves, and the side panels (inspector, relations, annotations). It owns
 * the camera and the user's per-Surface viewport; the RenderHost owns rendering and gestures. */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { describeError } from '../../api/session'
import { randomId } from '../../api/ids'
import type { EntityEnvelope, Json, Operation, Placement } from '../../api/types'
import { useOutlineVersion, useStore, useUserState, useWorkspaceUi } from '../../state/hooks'
import { BlockBoundary } from '../blocks/BlockHost'
import { useBlockContext } from '../blocks/useBlockContext'
import { BudgetContext, createBudget } from '../blocks/budget'
import { defaultRendererFor } from '../blocks/ops'
import { blockRegistry, CANVAS_MODE_LABEL, CANVAS_MODES, modePolicy, type BlockDefinition, type CanvasMode, type RenderContext } from '../blocks/registry'
import { RelationsPanel } from '../sources/RelationsPanel'
import { AnnotationsPanel } from '../shell/AnnotationsPanel'
import { FlowSurface } from './FlowSurface'
import { boundsOf, freeSpot, layoutSurface, relativeTo, surfaceBounds, topLevel } from './layout'
import { Camera } from './render/camera'
import { RenderHost, type LayoutChange } from './render/RenderHost'
import { SpatialIndex } from './render/spatialIndex'
import { BlockInspector, ContextMenu, NearToolbar, SurfaceNav, type NearAction } from './tools'
import { createSurfaceOps, insertChoices, surfacesOf } from './surfaceOps'

export interface CanvasFocus { surfaceId: string; blockId: string | null; nonce: number }

interface Viewport { x: number; y: number; zoom: number }

export function CanvasView({ focus }: { focus: CanvasFocus | null }) {
  const store = useStore()
  useOutlineVersion()
  const surfaces = surfacesOf(store)
  const remembered = useUserState<string>('surface:active')
  const [activeId, setActiveId] = useState<string | null>(null)
  // a focus request (open a Block from the data-source view) selects its Surface: derived during render
  const [focusSeen, setFocusSeen] = useState<CanvasFocus | null>(null)
  if (focus !== focusSeen) {
    setFocusSeen(focus)
    if (focus) setActiveId(focus.surfaceId)
  }
  const active = surfaces.find((s) => s.entity_id === (activeId ?? remembered)) ?? surfaces[0] ?? null
  const selectSurface = (id: string) => setActiveId(id)
  useEffect(() => { if (activeId) store.userState.set('surface:active', activeId) }, [activeId, store])
  const modeState = useUserState<CanvasMode>('canvas:mode')
  const mode: CanvasMode = modeState && CANVAS_MODES.includes(modeState) ? modeState : 'edit'
  const setMode = (next: CanvasMode) => store.userState.set('canvas:mode', next)
  const canStructure = store.session.info().capabilities.includes('structure')
  return (
    <div className="aiws-canvas-view" data-testid="aiws-canvas-view" data-mode={mode}>
      <div className="aiws-canvas-top">
        <SurfaceNav active={active?.entity_id ?? null} onSelect={selectSurface} />
        <div className="aiws-mode-tabs" role="tablist" aria-label="画布子模式" data-testid="aiws-mode-tabs">
          {CANVAS_MODES.map((m) => (
            <button key={m} type="button" role="tab" aria-selected={mode === m} data-testid={`aiws-mode-${m}`} onClick={() => setMode(m)}>{CANVAS_MODE_LABEL[m]}</button>
          ))}
        </div>
        <span className="aiws-grow" />
        <span className="aiws-muted" data-testid="aiws-mode-note">{mode === 'view' ? '查看模式：除批注外不修改文档' : mode === 'presentation_edit' ? '播放编辑：尚未实现，只读占位' : '编辑模式'}</span>
      </div>
      {!active ? (
        <div className="aiws-empty" data-testid="aiws-no-surface">
          <p>这个工作区还没有画布。</p>
          {canStructure && <button type="button" data-testid="aiws-create-first-surface" onClick={() => { void createFirstSurface(store, selectSurface, 'free') }}>创建第一张自由画布</button>}
          {canStructure && <button type="button" data-testid="aiws-create-first-flow" onClick={() => { void createFirstSurface(store, selectSurface, 'flow') }}>创建第一张流式页</button>}
        </div>
      ) : (
        <SurfaceView key={active.entity_id} surface={active} mode={mode} focus={focus} />
      )}
    </div>
  )
}

async function createFirstSurface(store: ReturnType<typeof useStore>, select: (id: string) => void, layout: 'free' | 'flow') {
  const { ops, surfaceId } = createSurfaceOps(store, layout === 'free' ? '画布 1' : '页 1', layout)
  const outcome = await store.submit({ editId: 'surface:new', label: '创建画布', operations: ops })
  if (outcome.status === 'accepted' || outcome.status === 'saved_locally') select(surfaceId)
}

function SurfaceView({ surface, mode, focus }: { surface: EntityEnvelope; mode: CanvasMode; focus: CanvasFocus | null }) {
  const store = useStore()
  const ui = useWorkspaceUi()
  const outlineVersion = useOutlineVersion()
  const surfaceId = surface.entity_id
  const policy = modePolicy(mode)
  const [selection, setSelectionState] = useState<Set<string>>(new Set())
  const [editing, setEditingState] = useState<string | null>(null)
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; world: { x: number; y: number }; blockId: string | null; parentId?: string } | null>(null)
  const [side, setSide] = useState<'inspector' | 'relations' | 'annotations' | null>(null)
  const [picker, setPicker] = useState<{ kind: 'add-data' | 'add-view' | 'move-surface'; world: { x: number; y: number } } | null>(null)
  const [camera] = useState(() => new Camera())
  const [budget] = useState(() => createBudget(surface.layout?.mode === 'free' ? undefined : { editors: 200, html: 8 }))
  const isFree = surface.layout?.mode === 'free'
  // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal of the outline model
  const laid = useMemo(() => layoutSurface(store.outline, surfaceId), [store, surfaceId, outlineVersion])
  const index = useMemo(() => {
    const idx = new SpatialIndex()
    for (const [id, l] of laid) idx.insert({ id, rect: l.rect, order: l.order, depth: l.depth })
    return idx
  }, [laid])
  // selection survives mode switches when the policy still allows it (§10.2); editors are released
  const setSelection = useCallback((next: Set<string>) => { setSelectionState(next) }, [])
  // activating an editor brings its Block into view (§8.2: the active Block is never half off-screen)
  const setEditing = useCallback((id: string | null) => { setEditingState(id); if (id) { const l = laid.get(id); if (l) camera.ensureVisible(l.rect) } }, [laid, camera])
  const policyKey = `${policy.select}:${policy.editContent}`
  const [policySeen, setPolicySeen] = useState(policyKey)
  if (policySeen !== policyKey) {
    setPolicySeen(policyKey)
    if (!policy.select) setSelectionState(new Set())
    if (!policy.editContent) setEditingState(null)
  }
  const [laidSeen, setLaidSeen] = useState(laid)
  if (laidSeen !== laid) {
    setLaidSeen(laid)
    const kept = [...selection].filter((id) => laid.has(id))
    if (kept.length !== selection.size) setSelectionState(new Set(kept))
    if (editing && !laid.has(editing)) setEditingState(null)
  }
  // an annotation draft (from a table cell, a rich text selection or the near tool) opens the annotations side
  const uiDraft = (ui as unknown as { draft?: unknown }).draft ?? null
  const [draftSeen, setDraftSeen] = useState<unknown>(null)
  if (uiDraft !== draftSeen) {
    setDraftSeen(uiDraft)
    if (uiDraft) setSide('annotations')
  }
  const [blockFocusSeen, setBlockFocusSeen] = useState(0)
  if (focus && focus.surfaceId === surfaceId && focus.nonce !== blockFocusSeen) {
    setBlockFocusSeen(focus.nonce)
    if (focus.blockId && laid.has(focus.blockId)) setSelectionState(new Set([focus.blockId]))
  }
  // per-Surface viewport in the user work state (§4.4)
  const viewportKey = `viewport:${surfaceId}`
  const savedViewport = useUserState<Json>(viewportKey) as Viewport | undefined
  const restored = useRef(false)
  useEffect(() => {
    if (restored.current) return
    if (savedViewport) camera.set(savedViewport)
    restored.current = true
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [savedViewport])
  useEffect(() => camera.onSettle(() => { if (restored.current) store.userState.set(viewportKey, { x: Math.round(camera.x), y: Math.round(camera.y), zoom: Number(camera.zoom.toFixed(3)) }) }), [camera, store, viewportKey])
  useEffect(() => {
    if (!focus || focus.surfaceId !== surfaceId || !focus.blockId) return
    const rect = laid.get(focus.blockId)?.rect
    if (rect) camera.fit({ x: rect.x - 80, y: rect.y - 80, w: rect.w + 160, h: rect.h + 160 })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the camera moves once per focus request, not per layout change
  }, [focus?.nonce, surfaceId, camera])

  const [viewportSize, setViewportSize] = useState({ w: 800, h: 600 })
  const hostRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const el = hostRef.current
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

  const canLayout = policy.layout && surface.capabilities.includes('structure')

  // ---- insertion (§4.2, §8.1)
  const insert = useCallback(async (definition: BlockDefinition, world: { x: number; y: number } | null, existingSourceId?: string, config?: Record<string, Json>, parentId?: string) => {
    if (!definition.create) return
    const contentFolderId = surface.content_folder_id ?? ''
    const parent = parentId ?? surfaceId
    const size = definition.defaultSize
    const spot = world ? { x: world.x, y: world.y, ...size } : freeSpot(laid, size, boundsOf(laid, selection))
    const placement: Placement = isFree ? relativeTo(laid, parent, spot) : { x: 0, y: 0, ...size }
    const cellId = randomId('c')
    const dataId = randomId('d')
    const orderKey = store.core.order_key_between(store.outline.childrenOf(parent).at(-1)?.order_key ?? undefined, undefined)
    const dataOrderKey = store.core.order_key_between(store.outline.childrenOf(contentFolderId).at(-1)?.order_key ?? undefined, undefined)
    const title = existingSourceId ? undefined : prompt(`${definition.title}的标题（可留空）`) ?? undefined
    if (title === undefined && !existingSourceId && definition.type !== 'frame' && definition.type !== 'shape') return
    const operations = definition.create({ store, surfaceId, contentFolderId, cellId, dataId, parentId: parent, orderKey, dataOrderKey, placement: isFree ? placement : { x: 0, y: 0, ...size }, title: title ?? undefined, existingSourceId, config })
    if (!isFree) for (const op of operations) if (op.op === 'entity.create' && op.entity_id === cellId) delete (op as Record<string, unknown>).placement
    const outcome = await store.submit({ editId: `insert:${cellId}`, label: `插入${definition.title}`, operations })
    if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set([cellId]))
  }, [store, surface.content_folder_id, surfaceId, laid, selection, isFree])

  // ---- actions on the selection
  const selectedEntities = [...selection].map((id) => store.outline.get(id)).filter((e): e is EntityEnvelope => Boolean(e))
  const single = selectedEntities.length === 1 ? selectedEntities[0] : null
  const selectedBlock = useBlockContext({
    cellId: single?.entity_id ?? null, mode, view: 'canvas', selected: true,
    editorActive: Boolean(single && editing === single.entity_id), zoom: camera.zoom,
    onActivate: () => { if (single) setEditing(single.entity_id) }, onDeactivate: () => setEditing(null),
  })
  const deleteSelection = async () => {
    if (!policy.layout) return
    const ops: Operation[] = []
    for (const id of topLevel(laid, selection)) {
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
    } else if (outcome.status === 'accepted' || outcome.status === 'saved_locally') setSelectionState(new Set())
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
    const ids = topLevel(laid, selection)
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
      ops.push({ op: 'tree.move', entity_id: id, new_parent_id: groupId, order_key: key, placement: { x: Math.round(l.rect.x - rect.x), y: Math.round(l.rect.y - rect.y), w: l.rect.w, h: l.rect.h } })
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
      ops.push({ op: 'tree.move', entity_id: child.entity_id, new_parent_id: l.parentId, order_key: key, placement: relativeTo(laid, l.parentId, cl.rect) })
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
    for (const id of topLevel(laid, selection)) {
      const l = laid.get(id)
      if (!l) continue
      key = store.core.order_key_between(key, undefined)
      const placement = { x: Math.round(l.rect.x), y: Math.round(l.rect.y), w: l.rect.w, h: l.rect.h }
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
    setSide('annotations')
  }

  // ---- keyboard (§8.2): Esc exits layer by layer; core actions do not depend on hover
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const target = event.target as HTMLElement | null
      if (target?.closest('input, textarea, select, [contenteditable="true"], [data-role="editor"]')) return
      if (!hostRef.current?.contains(target) && target !== document.body) return
      if (event.key === 'Escape') {
        if (menu) { setMenu(null); return }
        if (picker) { setPicker(null); return }
        if (editing) { setEditing(null); return }
        if (selection.size > 0) { setSelectionState(new Set()); return }
        return
      }
      if (!policy.select) return
      if ((event.key === 'Delete' || event.key === 'Backspace') && policy.layout && selection.size > 0) { event.preventDefault(); void deleteSelection(); return }
      if (event.key === 'Enter' && single && policy.editContent && !editing) { event.preventDefault(); setEditing(single.entity_id); return }
      if (event.key.startsWith('Arrow') && policy.layout && selection.size > 0 && canLayout) {
        event.preventDefault()
        const step = event.shiftKey ? 10 : 1
        const dx = event.key === 'ArrowLeft' ? -step : event.key === 'ArrowRight' ? step : 0
        const dy = event.key === 'ArrowUp' ? -step : event.key === 'ArrowDown' ? step : 0
        commitLayout(topLevel(laid, selection).flatMap((id) => { const l = laid.get(id); return l ? [{ id, placement: relativeTo(laid, l.parentId, { ...l.rect, x: l.rect.x + dx, y: l.rect.y + dy }) }] : [] }))
        return
      }
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 'g' && policy.layout) { event.preventDefault(); if (event.shiftKey) ungroup(); else group(); return }
      if (event.key === 'F2' && single && policy.layout) { event.preventDefault(); setSide('inspector') }
    }
    window.addEventListener('keydown', onKey)
    return () => window.removeEventListener('keydown', onKey)
  })

  // ---- near toolbar actions
  const nearActions: NearAction[] = []
  if (single && policy.select) {
    const context = selectedBlock.context
    const definition = selectedBlock.resolution?.ok ? context?.definition : undefined
    const source = single.source_id ? store.outline.get(single.source_id) : undefined
    if (definition?.actions && context) {
      for (const action of definition.actions) {
        if (!action.modes.includes(mode) || (action.views && !action.views.includes('canvas'))) continue
        const needs = action.needs ?? []
        const target = action.onSource ? source : single
        if (needs.some((cap) => !(target?.capabilities ?? []).includes(cap))) continue
        const writes = needs.some((cap) => ['update', 'append', 'structure', 'delete', 'manage'].includes(cap))
        if (writes && context.readOnlyReason) continue
        try {
          if (action.when && !action.when(context)) continue
          nearActions.push({ id: action.id, label: action.label, key: action.key, run: () => {
            void Promise.resolve().then(() => action.run(context, store)).catch((error: unknown) => store.notify('error', `扩展动作失败：${describeError(error)}`))
          } })
        } catch (error) {
          nearActions.push({ id: action.id, label: action.label, disabled: true, title: describeError(error), run: () => undefined })
        }
      }
    }
    if (source) nearActions.push({ id: 'relations', label: '查看依赖', run: () => setSide('relations') })
    if (source && policy.layout) nearActions.push({ id: 'add-view', label: '增加视图', run: () => setPicker({ kind: 'add-view', world: { x: 0, y: 0 } }) })
    if (definition?.Inspector || definition?.configFields) nearActions.push({ id: 'inspector', label: '属性', key: 'F2', run: () => setSide('inspector') })
    if (single.kind === 'group' && policy.layout) nearActions.push({ id: 'ungroup', label: '解组', key: 'Ctrl+Shift+G', run: ungroup })
    if (policy.annotate && source && ui.annotate) nearActions.push({ id: 'annotate', label: '批注', run: annotateSelection })
  }
  if (selection.size > 1 && policy.layout) nearActions.push({ id: 'group', label: '分组', key: 'Ctrl+G', run: group })
  if (selection.size > 0 && policy.layout && canLayout) {
    nearActions.push({ id: 'front', label: '置顶', run: () => reorder('front') }, { id: 'back', label: '置底', run: () => reorder('back') })
    if (surfacesOf(store).length > 1) nearActions.push({ id: 'move-surface', label: '移动到画布…', run: () => setPicker({ kind: 'move-surface', world: { x: 0, y: 0 } }) })
    nearActions.push({ id: 'delete', label: '删除', key: 'Delete', run: () => { void deleteSelection() } })
  }

  // ---- context menu items
  const menuItems = menu ? (menu.blockId ? nearActions.map((a) => ({ id: a.id, label: a.label, run: a.run, disabled: a.disabled })) : [
    ...(policy.insert && canLayout ? insertChoices().map((choice) => ({ id: `insert-${choice.definition.type}`, label: `插入${choice.label}`, run: () => { void insert(choice.definition, isFree ? menu.world : null, undefined, undefined, menu.parentId) } })) : []),
    ...(policy.insert && canLayout && !isFree ? [{ id: 'insert-group', label: '插入分组', run: () => {
      const parent = menu.parentId ?? surfaceId
      const title = window.prompt('分组标题（可留空）')
      if (title === null) return
      const groupId = randomId('g')
      void store.submit({ editId: `insert:${groupId}`, label: '插入分组', operations: [{ op: 'entity.create', entity_id: groupId, type_id: 'buckyos.container', parent_id: parent, order_key: store.core.order_key_between(store.outline.childrenOf(parent).at(-1)?.order_key ?? undefined, undefined), payload: { kind: 'group', layout: { mode: 'flow' }, title: title || '分组' } }] })
    } }] : []),
    ...(policy.insert && canLayout ? [{ id: 'sep', label: '', run: () => undefined, separator: true }, { id: 'add-data', label: '添加已有数据…', run: () => setPicker({ kind: 'add-data', world: menu.world }) }] : []),
    { id: 'fit', label: '适应全部', run: () => { const b = surfaceBounds(laid); if (b) camera.fit(b) } },
  ]) : []

  const canvasModeStyle = mode === 'presentation_edit' ? ' is-placeholder' : ''
  return (
    <BudgetContext.Provider value={budget}>
      <div className="aiws-canvas-body" ref={hostRef} data-testid="aiws-canvas-body">
        <div className={`aiws-canvas-main${canvasModeStyle}`}>
          {isFree ? (
            <RenderHost surfaceId={surfaceId} mode={mode} camera={camera} laid={laid} index={index} selection={selection} onSelectionChange={setSelection} editing={editing} onEditingChange={setEditing}
              canLayout={canLayout} onCommitLayout={commitLayout} onOpenEntity={ui.openEntity} gesturesPaused={Boolean(menu || picker)}
              onContextMenu={(point, blockId) => { if (!policy.select && !policy.insert) return; setMenu({ at: { x: point.screenX, y: point.screenY }, world: { x: point.worldX, y: point.worldY }, blockId }) }}
              renderNear={(bbox) => <NearToolbar bbox={bbox} actions={nearActions} viewport={viewportSize} />} />
          ) : (
            <div className="aiws-flow-host" data-testid="aiws-flow-host"><FlowSurface surfaceId={surfaceId} mode={mode} selected={selection} onSelect={setSelection} editing={editing} onEditingChange={setEditing} onInsert={(parentId) => setMenu({ at: { x: 200, y: 120 }, world: { x: 0, y: 0 }, blockId: null, parentId })} /></div>
          )}
          {mode === 'presentation_edit' && (
            <div className="aiws-presentation-placeholder" role="status" data-testid="aiws-presentation-placeholder">
              <b>播放编辑尚未实现</b>
              <div>这里将来编排演示路径、时间轴与镜头。本期只提供入口：画布保持静态显示，不选中、不写文档。</div>
            </div>
          )}
          {isFree && (
            <div className="aiws-canvas-tools" data-testid="aiws-canvas-tools">
              {policy.insert && canLayout && <button type="button" data-testid="aiws-insert-open" onClick={(event) => { const r = (event.currentTarget.parentElement as HTMLElement).getBoundingClientRect(); const host = hostRef.current?.getBoundingClientRect(); setMenu({ at: { x: r.left - (host?.left ?? 0), y: r.top - (host?.top ?? 0) + 28 }, world: camera.toWorld(viewportSize.w / 2 - 160, viewportSize.h / 2 - 100), blockId: null }) }}>插入 ▾</button>}
              <button type="button" data-testid="aiws-fit-all" onClick={() => { const b = surfaceBounds(laid); if (b) camera.fit(b); else camera.set({ x: 0, y: 0, zoom: 1 }) }}>适应全部</button>
              <button type="button" data-testid="aiws-fit-selection" disabled={selection.size === 0} onClick={() => { const b = boundsOf(laid, selection); if (b) camera.fit(b, 80) }}>适应选区</button>
              <button type="button" onClick={() => camera.zoomAt(viewportSize.w / 2, viewportSize.h / 2, 1 / 1.25)}>−</button>
              <span className="aiws-muted aiws-zoom" data-testid="aiws-zoom">{Math.round(camera.zoom * 100)}%</span>
              <button type="button" onClick={() => camera.zoomAt(viewportSize.w / 2, viewportSize.h / 2, 1.25)}>+</button>
              <span className="aiws-muted">{laid.size} 个对象</span>
              <button type="button" aria-pressed={side === 'inspector'} onClick={() => setSide(side === 'inspector' ? null : 'inspector')}>属性</button>
              <button type="button" aria-pressed={side === 'relations'} onClick={() => setSide(side === 'relations' ? null : 'relations')}>依赖</button>
              <button type="button" aria-pressed={side === 'annotations'} data-testid="aiws-side-annotations" onClick={() => setSide(side === 'annotations' ? null : 'annotations')}>批注</button>
            </div>
          )}
          {!isFree && mode !== 'presentation_edit' && (
            <div className="aiws-canvas-tools" data-testid="aiws-canvas-tools">
              <span className="aiws-muted">{laid.size} 个对象</span>
              <button type="button" aria-pressed={side === 'inspector'} onClick={() => setSide(side === 'inspector' ? null : 'inspector')}>属性</button>
              <button type="button" aria-pressed={side === 'relations'} onClick={() => setSide(side === 'relations' ? null : 'relations')}>依赖</button>
              <button type="button" aria-pressed={side === 'annotations'} data-testid="aiws-side-annotations" onClick={() => setSide(side === 'annotations' ? null : 'annotations')}>批注</button>
            </div>
          )}
          {menu && <ContextMenu at={menu.at} items={menuItems} onClose={() => setMenu(null)} />}
          {picker && <Picker kind={picker.kind} selection={selectedEntities} surfaceId={surfaceId} onClose={() => setPicker(null)} onPick={(choice) => {
            if (picker.kind === 'move-surface') moveToSurface(choice.id)
            else if (picker.kind === 'add-view' && single?.source_id) { const def = blockRegistry.get(choice.id); if (def) void insert(def, null, single.source_id); setPicker(null) }
            else if (picker.kind === 'add-data') { const source = store.outline.get(choice.id); const def = source ? blockRegistry.get(choice.renderer ?? defaultRendererFor(source.type_id) ?? '') : undefined; if (def && source) void insert(def, picker.world, source.entity_id); setPicker(null) }
          }} />}
        </div>
        {side && (
          <aside className="aiws-canvas-side" data-testid="aiws-canvas-side">
            <div className="aiws-inline-form"><b>{side === 'inspector' ? '属性' : side === 'relations' ? '引用与依赖' : '批注'}</b><span className="aiws-grow" /><button type="button" className="aiws-link" onClick={() => setSide(null)}>关闭</button></div>
            {side === 'inspector' && (single ? <SelectionInspector cell={single} mode={mode} context={selectedBlock.context} resolved={Boolean(selectedBlock.resolution?.ok)} registryVersion={selectedBlock.registryVersion} /> : <div className="aiws-muted">选中一个 Block 查看属性。</div>)}
            {side === 'relations' && (single?.source_id ? <RelationsPanel entityId={single.source_id} /> : <div className="aiws-muted">选中一个数据 Block 查看它的引用与依赖。</div>)}
            {side === 'annotations' && <AnnotationsPanel parentId={surface.content_folder_id ?? null} />}
          </aside>
        )}
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
      {live.placement && <div className="aiws-muted">位置 {live.placement.x}, {live.placement.y} · 尺寸 {live.placement.w} × {live.placement.h} · 顺序 {live.order_key}</div>}
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
    </BlockInspector>
  )
}

function Picker({ kind, selection, surfaceId, onClose, onPick }: { kind: 'add-data' | 'add-view' | 'move-surface'; selection: EntityEnvelope[]; surfaceId: string; onClose: () => void; onPick: (choice: { id: string; renderer?: string }) => void }) {
  const store = useStore()
  const [renderer, setRenderer] = useState('')
  const [chosen, setChosen] = useState('')
  const single = selection[0]
  const source = single?.source_id ? store.outline.get(single.source_id) : undefined
  const data = store.outline.all().filter((e) => !e.deleted && e.type_id !== 'buckyos.cell' && e.type_id !== 'buckyos.container' && e.type_id !== 'buckyos.block-def' && store.outline.ancestors(e.entity_id).includes('data'))
  const chosenEntity = store.outline.get(chosen)
  const renderers = kind === 'add-view' && source ? blockRegistry.forSource(source.type_id) : chosenEntity ? blockRegistry.forSource(chosenEntity.type_id) : []
  const targets = surfacesOf(store).filter((s) => s.entity_id !== surfaceId)
  return (
    <div className="aiws-dialog" role="dialog" aria-label={kind === 'add-data' ? '添加已有数据' : kind === 'add-view' ? '增加视图' : '移动到画布'} data-testid={`aiws-picker-${kind}`}>
      {kind === 'add-data' && (
        <>
          <b>添加已有数据到画布</b>
          <div className="aiws-muted">只创建引用该数据的 Block，不复制数据（§4.3）。</div>
          <select aria-label="数据" data-testid="aiws-picker-data" value={chosen} onChange={(event) => { setChosen(event.target.value); setRenderer('') }}>
            <option value="">选择数据…</option>
            {data.map((e) => <option key={e.entity_id} value={e.entity_id}>{e.title ?? e.name ?? e.entity_id}（{e.type_id.replace('buckyos.', '')}）</option>)}
          </select>
          {renderers.length > 1 && <select aria-label="展现方式" data-testid="aiws-picker-renderer" value={renderer} onChange={(event) => setRenderer(event.target.value)}><option value="">默认展现</option>{renderers.map((d) => <option key={d.type} value={d.type}>{d.title}</option>)}</select>}
          <div className="aiws-inline-form"><button type="button" data-testid="aiws-picker-confirm" disabled={!chosen} onClick={() => onPick({ id: chosen, renderer: renderer || undefined })}>添加</button><button type="button" onClick={onClose}>取消</button></div>
        </>
      )}
      {kind === 'add-view' && (
        <>
          <b>为「{source?.title ?? source?.name ?? ''}」增加视图</b>
          <div className="aiws-muted">新 Block 复用同一数据，视图配置独立。</div>
          <select aria-label="展现方式" data-testid="aiws-picker-renderer" value={renderer} onChange={(event) => setRenderer(event.target.value)}><option value="">选择展现…</option>{renderers.map((d) => <option key={d.type} value={d.type}>{d.title}</option>)}</select>
          <div className="aiws-inline-form"><button type="button" data-testid="aiws-picker-confirm" disabled={!renderer} onClick={() => onPick({ id: renderer })}>增加</button><button type="button" onClick={onClose}>取消</button></div>
        </>
      )}
      {kind === 'move-surface' && (
        <>
          <b>移动 {selection.length} 个 Block 到另一张画布</b>
          <div className="aiws-muted">Block 的身份、数据绑定和依赖记录不变；数据留在原画布内容区（§4.3）。</div>
          <select aria-label="目标画布" data-testid="aiws-picker-surface" value={chosen} onChange={(event) => setChosen(event.target.value)}><option value="">选择画布…</option>{targets.map((s) => <option key={s.entity_id} value={s.entity_id}>{s.title ?? s.name ?? s.entity_id}</option>)}</select>
          <div className="aiws-inline-form"><button type="button" data-testid="aiws-picker-confirm" disabled={!chosen} onClick={() => onPick({ id: chosen })}>移动</button><button type="button" onClick={onClose}>取消</button></div>
        </>
      )}
    </div>
  )
}
