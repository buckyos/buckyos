/* WorkspaceShell (phase two §5; UI improvement §3, §4, §12): the top-level views (data source / canvas),
 * the active Surface, the right panel, dialogs, layout preferences, persistent alerts and the single undo
 * stack. Switching a view never destroys the session, the undo coordinator or the pending queue — both
 * views live under one store. The canvas draws its own floating toolbars; the data-source view gets the
 * same main toolbar as a bar above it. */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CapturedAnchor } from '../../anchors/registry'
import type { AiwsClient } from '../../api/client'
import type { EntityEnvelope } from '../../api/types'
import { WorkspaceUiContext, useEdits, useLoad, useOutlineVersion, useStore, useUserState, useVersion, type AnnotationMark, type WorkspaceUi } from '../../state/hooks'
import { BlockHost } from '../blocks/BlockHost'
import { CanvasView, type CanvasFocus } from '../canvas/CanvasView'
import { surfacesOf } from '../canvas/surfaceOps'
import { DataSourceView } from '../sources/DataSourceView'
import { PermissionsPanel } from '../sources/PermissionsPanel'
import type { OfflineActions } from '../WorkspaceView'
import { ExportDialog, HelpDialog, ImportDialog, MockDialog, NewDialog } from './dialogs'
import { MainToolbar } from './MainToolbar'
import { LAYOUT_KEYS, PREF_KEYS, ShellContext, SOURCES_SIDE_TABS, type DialogRequest, type LayoutPrefs, type ShellApi, type SideTab, type SizeClass, type TopMode } from './shellContext'
import { SidePanel } from './SidePanel'
import { StatusDetail, StatusDock } from './StatusSummary'

export type { TopMode } from './shellContext'

/** Where a link or an explicit open asked to land (UI improvement §4 rule 1). */
export interface LaunchTarget { surfaceId?: string | null; blockId?: string | null }

/** Annotations in scope: under `parentId` or anchored to anything under it (plus the sources of Blocks there). */
function useAnnotations(parentId: string | null, entities: EntityEnvelope[]): AnnotationMark[] {
  const store = useStore()
  const any = useVersion('any')
  const key = useMemo(() => {
    if (!parentId) return ''
    const shown = new Set<string>()
    for (const e of store.outline.descendants(parentId)) { shown.add(e.entity_id); if (e.source_id) shown.add(e.source_id) }
    // a Surface's content folder: also what its Blocks show from the first-level data
    const surface = entities.find((e) => e.kind === 'surface' && e.content_folder_id === parentId)
    if (surface) for (const e of store.outline.descendants(surface.entity_id)) if (e.source_id) shown.add(e.source_id)
    return [...shown].sort().join(',')
  }, [parentId, entities, store])
  const load = useCallback(async (): Promise<AnnotationMark[]> => {
    if (!parentId) return []
    const annotations = await store.session.listAnnotations({ target_ids: key === '' ? [] : key.split(','), parent_id: parentId })
    return annotations.map((read) => ({ entityId: read.entity_id, payload: read.content.payload, anchor: read.content.anchor, envelope: read }))
  }, [store, key, parentId])
  return useLoad(load, any).data ?? []
}

function sizeOf(width: number): SizeClass {
  return width >= 1100 ? 'wide' : width >= 760 ? 'medium' : 'narrow'
}

export interface ShellProps {
  client: AiwsClient
  onClose: () => void
  onOpenWorkspace: (workspaceId: string) => void
  offline: OfflineActions
  identity: { principal: string | null; dev: boolean }
  onLogout: () => void
  devTools: boolean
  target?: LaunchTarget | null
}

export function WorkspaceShell({ client, onClose, onOpenWorkspace, offline, identity, onLogout, devTools, target }: ShellProps) {
  const store = useStore()
  const outlineVersion = useOutlineVersion()
  const [loadError, setLoadError] = useState<string | null>(null)
  useEffect(() => {
    let live = true
    store.outline.reload().catch((error: unknown) => { if (live) setLoadError(error instanceof Error ? error.message : String(error)) })
    return () => { live = false }
  }, [store])
  const loaded = store.outline.isLoaded()
  // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal of the outline model
  const entities = useMemo(() => store.outline.all(), [store, outlineVersion])
  // eslint-disable-next-line react-hooks/exhaustive-deps -- outlineVersion is the invalidation signal of the outline model
  const surfaces = useMemo(() => surfacesOf(store), [store, outlineVersion])

  // ---- top-level view and the active Surface (§4 rules 2–4: the remembered Surface, else the first readable one)
  const modeState = useUserState<TopMode>('mode')
  const mode: TopMode = modeState === 'sources' ? 'sources' : 'canvas'
  const setTopMode = useCallback((next: TopMode) => store.userState.set('mode', next), [store])
  const remembered = useUserState<string>('surface:active')
  const activeSurface = surfaces.find((s) => s.entity_id === remembered) ?? surfaces[0] ?? null
  const selectSurface = useCallback((surfaceId: string) => store.userState.set('surface:active', surfaceId), [store])

  // ---- right panel and layout preferences (user work state, `ui:*`)
  const sideState = useUserState<SideTab>('ui:side')
  const side = sideState ?? null
  const setSide = useCallback((tab: SideTab | null) => store.userState.set('ui:side', tab), [store])
  const objectToolbar = useUserState<boolean>(PREF_KEYS.objectToolbar)
  const presenterToolbar = useUserState<boolean>(PREF_KEYS.presenterToolbar)
  const grid = useUserState<boolean>(PREF_KEYS.grid)
  const prefs: LayoutPrefs = { objectToolbar: objectToolbar ?? true, presenterToolbar: presenterToolbar ?? true, grid: grid ?? true }
  const setPref = useCallback((key: keyof LayoutPrefs, value: boolean) => store.userState.set(PREF_KEYS[key], value), [store])
  const resetLayout = useCallback(() => { for (const key of LAYOUT_KEYS) store.userState.set(key, null) }, [store])

  const [selected, setSelected] = useState<string | null>(null)
  // a link's target is the first focus request (the Surface becomes active below, once the outline says it exists)
  const [focus, setFocus] = useState<CanvasFocus | null>(() => (target?.surfaceId ? { surfaceId: target.surfaceId, blockId: target.blockId ?? null, nonce: 1 } : null))
  const [draft, setDraft] = useState<CapturedAnchor | null>(null)
  const [activeAnnotation, setActiveAnnotation] = useState<string | null>(null)
  const [dialog, setDialog] = useState<DialogRequest | null>(null)
  const [offlineBusy, setOfflineBusy] = useState<string | null>(null)
  const [offlineError, setOfflineError] = useState<string | null>(null)
  const canComment = store.session.info().capabilities.includes('comment')
  const scopeParent = mode === 'canvas' ? (activeSurface?.content_folder_id ?? null) : (selected ? (store.outline.get(selected)?.type_id === 'buckyos.container' ? selected : store.outline.get(selected)?.parent_id ?? 'data') : 'data')
  const annotations = useAnnotations(scopeParent, entities)

  const lockRequired = entities.some((entity) => entity.write_policy === 'lock_required')
  useEffect(() => { store.setLockPolling(lockRequired); return () => store.setLockPolling(false) }, [store, lockRequired])
  // stale user state of Surfaces that are gone (§4.4; UI improvement §12.1)
  useEffect(() => { if (loaded) store.userState.pruneSurfaces(new Set(surfaces.map((e) => e.entity_id))) }, [store, surfaces, loaded])
  // a workspace never reopens into the presentation-edit placeholder (§4 rule 3)
  useEffect(() => {
    let live = true
    void store.userReady.then(() => {
      if (!live || store.userState.get('canvas:mode') !== 'presentation_edit') return
      store.userState.set('canvas:mode', 'view')
      store.notify('info', '上次停在“播放编辑”占位（尚未实现），已切换到查看模式。')
    })
    return () => { live = false }
  }, [store])
  // an explicit target (a link) wins over the remembered Surface (§4 rule 1); a target that is gone falls back and says so
  const targetHandled = useRef(false)
  useEffect(() => {
    if (!loaded || !target || targetHandled.current) return
    targetHandled.current = true
    const surface = target.surfaceId ? surfaces.find((s) => s.entity_id === target.surfaceId) : undefined
    if (surface) {
      store.userState.set('surface:active', surface.entity_id)
      store.userState.set('mode', 'canvas')
    } else if (target.surfaceId) {
      store.notify('info', surfaces.length > 0 ? '链接指向的画布已不存在或你无权查看，已打开这个工作区的第一张画布。' : '链接指向的画布已不存在或你无权查看。')
    }
  }, [loaded, target, surfaces, store])

  // a new problem that needs a decision opens the save-state panel: problems stay visible (§10)
  const edits = useEdits()
  let attention = 0
  for (const entry of edits.values()) if (entry.state === 'needs_attention') attention += 1
  const attentionSeen = useRef(0)
  useEffect(() => {
    if (attention > attentionSeen.current) store.userState.set('ui:side', 'edits')
    attentionSeen.current = attention
  }, [attention, store])

  // ---- window size class of the application container (§11)
  const rootRef = useRef<HTMLDivElement>(null)
  const [size, setSize] = useState<SizeClass>('wide')
  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    const observer = new ResizeObserver(() => setSize(sizeOf(el.clientWidth)))
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  /** Show an entity: a Block focuses its Surface on the canvas; data opens in the data-source view. */
  const openEntity = useCallback((entityId: string) => {
    const entity = store.outline.get(entityId)
    if (!entity) return
    if (entity.type_id === 'buckyos.cell' || entity.kind === 'group') {
      const surfaceId = store.outline.ancestors(entityId).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')?.entity_id
      if (surfaceId) { setFocus({ surfaceId, blockId: entityId, nonce: Date.now() }); store.userState.set('surface:active', surfaceId); setTopMode('canvas') }
      return
    }
    if (entity.kind === 'surface') { setFocus({ surfaceId: entityId, blockId: null, nonce: Date.now() }); store.userState.set('surface:active', entityId); setTopMode('canvas'); return }
    setSelected(entityId)
    setTopMode('sources')
  }, [store, setTopMode])
  const annotate = useCallback((anchor: CapturedAnchor) => {
    setDraft(anchor)
    if (store.userState.get('mode') !== 'sources') store.userState.set('ui:side', 'annotations')
  }, [store])
  const renderCell = useCallback((cellId: string, depth: number) => {
    const embedded = store.outline.get(cellId)
    if (!embedded || embedded.deleted) return <div className="aiws-warning">嵌入的 Block 不存在或已删除（{cellId}）</div>
    if (embedded.type_id !== 'buckyos.cell') return <div className="aiws-warning">嵌入目标不是 Block（{cellId}）</div>
    return (
      <div className="aiws-embedded" data-testid={`aiws-embed-${cellId}`}>
        <div className="aiws-embedded-title">嵌入 · {embedded.title ?? embedded.name ?? cellId}（只读）</div>
        <BlockHost cellId={cellId} mode="view" view="source" depth={depth} />
      </div>
    )
  }, [store])
  const ui = useMemo<WorkspaceUi & { draft: CapturedAnchor | null; clearDraft: () => void }>(() => ({
    entities, byId: new Map(entities.map((entity) => [entity.entity_id, entity])), annotations, openEntity,
    annotate: canComment ? annotate : null, activeAnnotation, setActiveAnnotation, renderCell, draft, clearDraft: () => setDraft(null),
  }), [entities, annotations, openEntity, annotate, canComment, activeAnnotation, renderCell, draft])

  const runOffline = useCallback((label: string, work: () => Promise<void>) => {
    setOfflineBusy(label)
    setOfflineError(null)
    work().catch((error: unknown) => setOfflineError(offline.describe(error))).finally(() => setOfflineBusy(null))
  }, [offline])

  const shell: ShellApi = {
    client, close: onClose, openWorkspace: onOpenWorkspace, offline, topMode: mode, setTopMode, surfaces, activeSurface, selectSurface,
    side, setSide, openDialog: setDialog, prefs, setPref, resetLayout, size, devTools, identity, logout: onLogout,
    runOffline, offlineBusy, offlineError, clearOfflineError: () => setOfflineError(null),
  }

  // Ctrl/Cmd+Z goes to the UndoCoordinator: exactly one step per key press (design §2.7)
  useEffect(() => {
    let active = true
    const onPointer = (event: PointerEvent) => { active = Boolean(rootRef.current?.contains(event.target as Node)) }
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.defaultPrevented || event.isComposing) return
      const key = event.key.toLowerCase()
      if (key !== 'z' && key !== 'y') return
      const root = rootRef.current
      const keyTarget = event.target as HTMLElement | null
      if (!root || !keyTarget) return
      const inside = root.contains(keyTarget)
      if (!inside && !(active && (keyTarget === document.body || keyTarget === document.documentElement))) return
      if (inside && keyTarget.closest('input, textarea, select')) return
      event.preventDefault()
      if (key === 'y' || event.shiftKey) void store.undo.redo()
      else void store.undo.undo()
    }
    window.addEventListener('pointerdown', onPointer, true)
    window.addEventListener('keydown', onKey)
    return () => { window.removeEventListener('pointerdown', onPointer, true); window.removeEventListener('keydown', onKey) }
  }, [store])

  const workspaceInfo = store.session.info()
  return (
    <div ref={rootRef} className="aiws-workspace" data-testid="aiws-workspace" data-workspace-id={store.session.workspaceId} data-session-id={store.session.sessionId} data-top-mode={mode} data-size={size}>
      <ShellContext.Provider value={shell}>
        {loadError && !loaded && <div className="aiws-error" role="alert">无法读取工作区：{loadError}</div>}
        {loaded && (
          <WorkspaceUiContext.Provider value={ui}>
            <div className="aiws-stage">
              {mode === 'sources' ? (
                <div className="aiws-sources-stage">
                  <div className="aiws-sources-bar"><MainToolbar canvas={null} /></div>
                  <div className="aiws-sources-row">
                    <div className="aiws-sources-work">
                      <DataSourceView selected={selected} onSelect={setSelected} />
                      <StatusDock />
                    </div>
                    <SidePanel tabs={SOURCES_SIDE_TABS} render={(tab) => (tab === 'collab' ? <PermissionsPanel /> : <StatusDetail />)} />
                  </div>
                </div>
              ) : <CanvasView focus={focus} />}
            </div>
            {dialog?.kind === 'new' && <NewDialog client={client} store={store} initialTab={dialog.tab} onClose={() => setDialog(null)} onCreatedSurface={(id) => { selectSurface(id); setTopMode('canvas') }} onOpenWorkspace={onOpenWorkspace} />}
            {dialog?.kind === 'export' && <ExportDialog client={client} workspace={{ workspace_id: workspaceInfo.workspace_id, title: workspaceInfo.title }} onClose={() => setDialog(null)} />}
            {dialog?.kind === 'import' && <ImportDialog client={client} onClose={() => setDialog(null)} onOpenWorkspace={onOpenWorkspace} />}
            {dialog?.kind === 'help' && <HelpDialog onClose={() => setDialog(null)} />}
            {dialog?.kind === 'mock' && <MockDialog onClose={() => setDialog(null)} />}
          </WorkspaceUiContext.Provider>
        )}
      </ShellContext.Provider>
    </div>
  )
}

