/* WorkspaceShell (phase two §5; UI improvement §3, §4, §12): the top-level views (data source / canvas),
 * the active Surface, the right panel, dialogs, layout preferences, persistent alerts and the single undo
 * stack. Switching a view never destroys the session, the undo coordinator or the pending queue — both
 * views live under one store. The canvas draws its own floating toolbars; the data-source view gets the
 * same main toolbar as a bar above it. */

import { ExtensionsDialog } from '../extensions/ExtensionsDialog'
import { SaveTemplateDialog } from '../extensions/SaveTemplateDialog'
import { isCanvasContainer } from '../controls/registry'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import type { CapturedAnchor } from '../../anchors/registry'
import type { AiwsClient } from '../../api/client'
import type { EntityEnvelope } from '../../api/types'
import { WorkspaceUiContext, useEdits, useLoad, useOutlineVersion, useStore, useUserState, useVersion, type AnnotationMark, type WorkspaceUi } from '../../state/hooks'
import { useAiwsText } from '../../useAiwsText'
import { BlockHost } from '../blocks/BlockHost'
import { CanvasView, type CanvasFocus } from '../canvas/CanvasView'
import { SurfaceDeleteDialog } from '../canvas/surfaceManage'
import { surfacesOf } from '../canvas/surfaceOps'
import { DataModeStage } from '../datamode/DataModeView'
import { dataTabsOf, onDataEditorRequest, openDataEditor, PERMISSIONS_TAB } from '../datamode/tabs'
import type { OfflineActions } from '../WorkspaceView'
import type { Json } from '../../api/types'
import { pathsOf } from '../../presentation/model'
import { openShow, resumeShow } from '../../presentation/showSession'
import { StartShowDialog } from '../../presentation/StartShowDialog'
import { StageView, type ShowSession } from '../../presentation/StageView'
import { ExportDialog, HelpDialog, ImportDialog, MockDialog, NewDialog } from './dialogs'
import { ShareDialog } from './ShareDialog'
import { LAYOUT_KEYS, PREF_KEYS, ShellContext, usePhone, type DialogRequest, type GuideState, type LayoutPrefs, type ShellApi, type SideTab, type SizeClass, type TopMode } from './shellContext'
import { TableImportHost } from '../import/TableImportDialog'
import { OfficeImportHost } from '../import/ImportOfficeDialog'
import { BindDialogHost } from '../controls/data/BindDialogs'
import { PermissionsHost } from '../permissions'
import { FrameworkHost } from '../framework/FrameworkHost'
import { expandStatus, personalZoneShown, resetPersonalLayout, setPersonalZone } from '../framework/personal'
import { useUserPrefs } from '../../state/prefs'
import { isDataControl } from '@controls/contract'
import { ControlPage } from '../controls/data/ControlPage'
import { onControlPage, requestRecordFocus } from '../controls/data/focus'
import type { RecordTarget } from '../../links'

export type { TopMode } from './shellContext'

/** Where a link or an explicit open asked to land (UI improvement §4 rule 1). */
export interface LaunchTarget extends RecordTarget { surfaceId?: string | null; blockId?: string | null }

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
  identity: { principal: string | null; dev: boolean; guest: boolean; restricted: boolean }
  onLogout: () => void
  onLogin: () => void
  onHome: (() => void) | null
  devTools: boolean
  target?: LaunchTarget | null
}

export function WorkspaceShell({ client, onClose, onOpenWorkspace, offline, identity, onLogout, onLogin, onHome, devTools, target }: ShellProps) {
  const store = useStore()
  const { t } = useAiwsText()
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

  // ---- top-level view and the active Surface (§4 rules 2–4: the remembered Surface, else the first readable one);
  // a phone keeps its view in this window and leaves the stored one alone: the canvas first, the data mode when asked
  // (UI 框架 PRD §5.6: the main menu's "数据模式", a canvas's "展开", data opened from a comment or a property)
  const phone = usePhone()
  const modeState = useUserState<TopMode>('mode')
  const [phoneMode, setPhoneMode] = useState<TopMode>('canvas')
  const mode: TopMode = phone ? phoneMode : modeState === 'sources' ? 'sources' : 'canvas'
  const setTopMode = useCallback((next: TopMode) => { if (phone) setPhoneMode(next === 'sources' ? 'sources' : 'canvas'); else store.userState.set('mode', next) }, [store, phone])
  const remembered = useUserState<string>('surface:active')
  const activeSurface = surfaces.find((s) => s.entity_id === remembered) ?? surfaces[0] ?? null
  const selectSurface = useCallback((surfaceId: string) => store.userState.set('surface:active', surfaceId), [store])

  // ---- right panel and layout preferences (user work state, `ui:*`)
  const sideState = useUserState<SideTab>('ui:side')
  const side = sideState ?? null
  const setSide = useCallback((tab: SideTab | null) => store.userState.set('ui:side', tab), [store])
  // the tool zones' switches are the person's across Workspaces (UI 框架 PRD §4.6, §12); the grid is this Workspace's
  const personal = useUserPrefs(client.transport)
  const grid = useUserState<boolean>(PREF_KEYS.grid)
  const prefs: LayoutPrefs = { objectToolbar: personalZoneShown(personal, 'tools'), viewToolbar: personalZoneShown(personal, 'collab'), grid: grid ?? true }
  const setPref = (key: keyof LayoutPrefs, value: boolean) => {
    if (key === 'grid') store.userState.set(PREF_KEYS.grid, value)
    else void setPersonalZone(personal, key === 'objectToolbar' ? 'tools' : 'collab', !value)
  }
  const resetLayout = () => { for (const key of LAYOUT_KEYS) store.userState.set(key, null); void resetPersonalLayout(personal) }

  const [selected, setSelected] = useState<string | null>(null)
  // a link's target is the first focus request (the Surface becomes active below, once the outline says it exists)
  const [focus, setFocus] = useState<CanvasFocus | null>(() => (target?.surfaceId ? { surfaceId: target.surfaceId, blockId: target.blockId ?? null, nonce: 1 } : null))
  // a link to a record of a data control (G12): the control turns to it once it is drawn
  useEffect(() => { if (target?.blockId && target.record) requestRecordFocus(target.blockId, target.record, target.occurrence) }, [target])
  // a data control as its own page (G14): asked by the link (`page=control`), by a record link on a phone, or by its menu
  const [pageChoice, setPageChoice] = useState<string | null | undefined>(undefined)
  useEffect(() => onControlPage((id) => setPageChoice(id)), [])
  const linkPage = target?.blockId && (target.page === 'control' || (phone && target.record && isDataControl(store.outline.get(target.blockId)?.kind))) ? target.blockId : null
  const pageControl = pageChoice === undefined ? linkPage : pageChoice
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
  // ---- shows and guides (第三期规划 §7.2, §7.3): a show takes the window over until it ends; a guide runs on the canvas
  const [showSession, setShowSession] = useState<ShowSession | null>(null)
  const [startRequest, setStartRequest] = useState<{ pathId: string | null; stepId: string | null } | null>(null)
  const [guide, setGuideState] = useState<GuideState | null>(null)
  const offerNotice = useRef<number | null>(null)
  const setGuide = useCallback((next: GuideState | null) => {
    setGuideState(next)
    if (!next) return
    store.userState.set('mode', 'canvas')
    // a guide that runs needs no more offering
    if (offerNotice.current !== null) { store.dismissNotice(offerNotice.current); offerNotice.current = null }
  }, [store])
  const endShow = useCallback((reason?: string) => {
    setShowSession((current) => {
      if (current && current.store !== store) void current.store.dispose()
      return null
    })
    // the lock was this show's: writing is open again (the follower would only learn it at its next poll)
    store.session.noteShowLock(null)
    if (reason) store.notify('info', reason)
  }, [store])
  // a reloaded stage takes its show back while the show is alive (§11.3)
  useEffect(() => {
    let live = true
    void resumeShow(client, store).then((session) => { if (live && session) setShowSession(session); else if (session && session.store !== store) void session.store.dispose() })
    return () => { live = false }
  }, [client, store])
  // a workspace with a guide offers it once (the offer is remembered in the user state, §7.3)
  useEffect(() => {
    let live = true
    void store.userReady.then(() => {
      if (!live) return
      const offer = pathsOf(store.outline, 'guide').find((g) => store.userState.get(`guide:${g.entity_id}`) === undefined)
      if (!offer) return
      store.userState.set(`guide:${offer.entity_id}`, { index: 0, prompted: true } satisfies Json)
      offerNotice.current = store.notify('info', t('aiworkspace.shell.workspace.guideOffer', { name: offer.title ?? offer.name ?? '' }), { label: t('aiworkspace.shell.workspace.startGuide'), run: () => setGuide({ pathId: offer.entity_id, index: 0 }) })
    })
    return () => { live = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per opened workspace, when the outline is there
  }, [store, loaded])
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
      store.notify('info', surfaces.length > 0 ? t('aiworkspace.shell.workspace.targetGoneFallback') : t('aiworkspace.shell.workspace.targetGone'))
    }
  }, [loaded, target, surfaces, store, t])

  // a new problem that needs a decision opens the status panel (UI 框架 PRD §4.5): problems stay visible (§10)
  const edits = useEdits()
  let attention = 0
  for (const entry of edits.values()) if (entry.state === 'needs_attention') attention += 1
  const attentionSeen = useRef(0)
  useEffect(() => {
    if (attention > attentionSeen.current) expandStatus(client.transport)
    attentionSeen.current = attention
  }, [attention, client])

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

  /** Show an entity: a Block focuses its Surface on the canvas; data opens its standard editor in a tab of its own in
   * the data mode — on a phone too, full screen (UI 框架 PRD §5.3, §5.6). */
  const openEntity = useCallback((entityId: string) => {
    const entity = store.outline.get(entityId)
    if (!entity) return
    const surfaceOf = (id: string) => store.outline.ancestors(id).map((a) => store.outline.get(a)).find((e) => e?.kind === 'surface')?.entity_id
    const focusBlock = (blockId: string) => {
      const surfaceId = surfaceOf(blockId)
      if (!surfaceId) return
      setFocus({ surfaceId, blockId, nonce: Date.now() })
      store.userState.set('surface:active', surfaceId)
      setTopMode('canvas')
    }
    if (entity.type_id === 'buckyos.cell' || isCanvasContainer(entity)) { focusBlock(entityId); return }
    if (entity.kind === 'surface') { setFocus({ surfaceId: entityId, blockId: null, nonce: Date.now() }); store.userState.set('surface:active', entityId); setTopMode('canvas'); return }
    openDataEditor(entityId)
  }, [store, setTopMode])
  // "open this object's editor in the data mode" from anywhere (openDataEditor: the canvas's "查看数据", a phone's "展开",
  // "成员与权限…"): the data mode, with the object's tab current
  useEffect(() => onDataEditorRequest(({ entityId, ...options }) => {
    dataTabsOf(store).open(entityId, options)
    setTopMode('sources')
  }), [store, setTopMode])
  const annotate = useCallback((anchor: CapturedAnchor) => {
    setDraft(anchor)
    if (store.userState.get('mode') !== 'sources') store.userState.set('ui:side', 'annotations')
  }, [store])
  const [annotationFilter, setAnnotationFilter] = useState<string | null>(null)
  const showAnnotations = useCallback((targetId: string | null) => {
    setAnnotationFilter(targetId)
    if (targetId && store.userState.get('mode') !== 'sources') store.userState.set('ui:side', 'annotations')
  }, [store])
  const renderCell = useCallback((cellId: string, depth: number) => {
    const embedded = store.outline.get(cellId)
    if (!embedded || embedded.deleted) return <div className="aiws-warning">{t('aiworkspace.shell.workspace.embedMissing', { id: cellId })}</div>
    if (embedded.type_id !== 'buckyos.cell') return <div className="aiws-warning">{t('aiworkspace.shell.workspace.embedNotBlock', { id: cellId })}</div>
    return (
      <div className="aiws-embedded" data-testid={`aiws-embed-${cellId}`}>
        <div className="aiws-embedded-title" title={t('aiworkspace.shell.workspace.embedTitle')}>↗ {embedded.title ?? embedded.name ?? cellId}</div>
        <BlockHost cellId={cellId} mode="view" view="source" depth={depth} />
      </div>
    )
  }, [store, t])
  const ui = useMemo<WorkspaceUi & { draft: CapturedAnchor | null; clearDraft: () => void }>(() => ({
    entities, byId: new Map(entities.map((entity) => [entity.entity_id, entity])), annotations, openEntity,
    annotate: canComment ? annotate : null, activeAnnotation, setActiveAnnotation, renderCell, draft, clearDraft: () => setDraft(null),
    annotationFilter, showAnnotations,
  }), [entities, annotations, openEntity, annotate, canComment, activeAnnotation, renderCell, draft, annotationFilter, showAnnotations])

  const runOffline = useCallback((label: string, work: () => Promise<void>) => {
    setOfflineBusy(label)
    setOfflineError(null)
    work().catch((error: unknown) => setOfflineError(offline.describe(error))).finally(() => setOfflineBusy(null))
  }, [offline])

  const shell: ShellApi = {
    client, close: onClose, openWorkspace: onOpenWorkspace, offline, topMode: showSession ? 'show' : mode, setTopMode, surfaces, activeSurface, selectSurface,
    side, setSide, openDialog: setDialog, openPermissions: () => openDataEditor(PERMISSIONS_TAB), prefs, setPref, resetLayout, size, phone, devTools, identity, logout: onLogout, login: onLogin, home: onHome,
    runOffline, offlineBusy, offlineError, clearOfflineError: () => setOfflineError(null),
    startShow: (request) => setStartRequest({ pathId: request?.pathId ?? null, stepId: request?.stepId ?? null }),
    guide, setGuide,
  }

  // Ctrl/Cmd+Z goes to the UndoCoordinator: exactly one step per key press (design §2.7)
  useEffect(() => {
    let active = true
    const onPointer = (event: PointerEvent) => { active = Boolean(rootRef.current?.contains(event.target as Node)) }
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.defaultPrevented || event.isComposing) return
      // the stage of a show never undoes the workspace's edits
      if (rootRef.current?.querySelector('[data-testid="aiws-show"]')) return
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
    <div ref={rootRef} className="aiws-workspace" data-testid="aiws-workspace" data-workspace-id={store.session.workspaceId} data-session-id={store.session.sessionId} data-top-mode={mode} data-size={size} data-phone={phone ? 'true' : undefined}>
      <ShellContext.Provider value={shell}>
        {loadError && !loaded && <div className="aiws-error" role="alert">{t('aiworkspace.shell.workspace.loadError', { error: loadError })}</div>}
        {loaded && (
          <WorkspaceUiContext.Provider value={ui}>
            <div className="aiws-stage">
              {showSession ? <StageView session={showSession} client={client} onExit={endShow} /> : mode === 'sources' ? (
                <DataModeStage onActive={setSelected} />
              ) : pageControl && store.outline.get(pageControl) ? <ControlPage containerId={pageControl} onBack={() => setPageChoice(null)} /> : <CanvasView focus={focus} />}
            </div>
            {startRequest && !showSession && (
              <StartShowDialog initialPathId={startRequest.pathId} initialStepId={startRequest.stepId} onClose={() => setStartRequest(null)} transport={client.transport}
                onStart={async (pathId, stepId) => {
                  const session = await openShow(client, store, pathId, stepId)
                  setGuideState(null)
                  setStartRequest(null)
                  setShowSession(session)
                }} />
            )}
            {dialog?.kind === 'new' && <NewDialog client={client} store={store} initialTab={dialog.tab} folderId={dialog.folderId} onClose={() => setDialog(null)} onCreatedSurface={(id) => { selectSurface(id); setTopMode('canvas') }} onOpenWorkspace={onOpenWorkspace} />}
            {dialog?.kind === 'share' && <ShareDialog onClose={() => setDialog(null)} />}
            {dialog?.kind === 'export' && <ExportDialog client={client} workspace={{ workspace_id: workspaceInfo.workspace_id, title: workspaceInfo.title }} onClose={() => setDialog(null)} />}
            {dialog?.kind === 'import' && <ImportDialog client={client} onClose={() => setDialog(null)} onOpenWorkspace={onOpenWorkspace} />}
            {dialog?.kind === 'help' && <HelpDialog onClose={() => setDialog(null)} />}
            {dialog?.kind === 'mock' && <MockDialog onClose={() => setDialog(null)} />}
            {dialog?.kind === 'extensions' && <ExtensionsDialog initialDefId={dialog.defId} phone={phone} onClose={() => setDialog(null)} />}
            {dialog?.kind === 'save-template' && <SaveTemplateDialog initialSurfaceId={activeSurface?.entity_id ?? null} onClose={() => setDialog(null)} />}
            {dialog?.kind === 'delete-surface' && <SurfaceDeleteDialog key={dialog.surface.entity_id} surface={dialog.surface} onClose={() => setDialog(null)} onDeleted={(next) => { if (next) selectSurface(next) }} />}
            <TableImportHost />
            <OfficeImportHost />
            <BindDialogHost />
            <PermissionsHost />
            <FrameworkHost />
          </WorkspaceUiContext.Provider>
        )}
      </ShellContext.Provider>
    </div>
  )
}

