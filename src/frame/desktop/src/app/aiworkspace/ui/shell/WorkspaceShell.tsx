/* WorkspaceShell (phase two §5): the top-level modes (data source / canvas), save and sync state,
 * the single undo stack, notices and the user work state. Switching a mode never destroys the
 * session, the undo coordinator or the pending queue — both views live under one store. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import type { CapturedAnchor } from '../../anchors/registry'
import type { EntityEnvelope } from '../../api/types'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { applyServiceWorkerUpdate } from '../../../../serviceWorker'
import { WorkspaceUiContext, useAppCache, useEdits, useLoad, useOutlineVersion, useSessionStatus, useStore, useUserState, useVersion, type AnnotationMark, type WorkspaceUi } from '../../state/hooks'
import { BlockHost } from '../blocks/BlockHost'
import { CanvasView, type CanvasFocus } from '../canvas/CanvasView'
import { DataSourceView } from '../sources/DataSourceView'
import type { OfflineActions } from '../WorkspaceView'
import { EditsPanel, MockRunPanel, Notices } from './panels'

export type TopMode = 'sources' | 'canvas'

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

export function WorkspaceShell({ onClose, offline }: { onClose: () => void; offline: OfflineActions }) {
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
  const modeState = useUserState<TopMode>('mode')
  const mode: TopMode = modeState === 'sources' ? 'sources' : 'canvas'
  const setMode = (next: TopMode) => store.userState.set('mode', next)
  const [selected, setSelected] = useState<string | null>(null)
  const [focus, setFocus] = useState<CanvasFocus | null>(null)
  const [draft, setDraft] = useState<CapturedAnchor | null>(null)
  const [activeAnnotation, setActiveAnnotation] = useState<string | null>(null)
  const [side, setSide] = useState<'none' | 'mock'>('none')
  const canComment = store.session.info().capabilities.includes('comment')
  const activeSurface = useUserState<string>('surface:active')
  const surface = entities.find((e) => e.kind === 'surface' && e.entity_id === activeSurface) ?? entities.find((e) => e.kind === 'surface')
  const scopeParent = mode === 'canvas' ? (surface?.content_folder_id ?? null) : (selected ? (store.outline.get(selected)?.type_id === 'buckyos.container' ? selected : store.outline.get(selected)?.parent_id ?? 'data') : 'data')
  const annotations = useAnnotations(scopeParent, entities)

  const lockRequired = entities.some((entity) => entity.write_policy === 'lock_required')
  useEffect(() => { store.setLockPolling(lockRequired); return () => store.setLockPolling(false) }, [store, lockRequired])
  // stale user state of Surfaces that are gone (§4.4)
  useEffect(() => { if (loaded) store.userState.pruneSurfaces(new Set(entities.filter((e) => e.kind === 'surface').map((e) => e.entity_id))) }, [store, entities, loaded])

  /** Show an entity: a Block focuses its Surface on the canvas; data opens in the data-source view. */
  const openEntity = useCallback((entityId: string) => {
    const entity = store.outline.get(entityId)
    if (!entity) return
    if (entity.type_id === 'buckyos.cell' || entity.kind === 'group') {
      const surfaceId = store.outline.ancestors(entityId).map((id) => store.outline.get(id)).find((e) => e?.kind === 'surface')?.entity_id
      if (surfaceId) { setFocus({ surfaceId, blockId: entityId, nonce: Date.now() }); store.userState.set('surface:active', surfaceId); setMode('canvas') }
      return
    }
    if (entity.kind === 'surface') { setFocus({ surfaceId: entityId, blockId: null, nonce: Date.now() }); setMode('canvas'); return }
    setSelected(entityId)
    setMode('sources')
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [store])
  const annotate = useCallback((anchor: CapturedAnchor) => setDraft(anchor), [])
  const renderCell = useCallback((cellId: string, depth: number) => {
    const target = store.outline.get(cellId)
    if (!target || target.deleted) return <div className="aiws-warning">嵌入的 Block 不存在或已删除（{cellId}）</div>
    if (target.type_id !== 'buckyos.cell') return <div className="aiws-warning">嵌入目标不是 Block（{cellId}）</div>
    return (
      <div className="aiws-embedded" data-testid={`aiws-embed-${cellId}`}>
        <div className="aiws-embedded-title">嵌入 · {target.title ?? target.name ?? cellId}（只读）</div>
        <BlockHost cellId={cellId} mode="view" view="source" depth={depth} />
      </div>
    )
  }, [store])
  const ui = useMemo<WorkspaceUi & { draft: CapturedAnchor | null; clearDraft: () => void }>(() => ({
    entities, byId: new Map(entities.map((entity) => [entity.entity_id, entity])), annotations, openEntity,
    annotate: canComment ? annotate : null, activeAnnotation, setActiveAnnotation, renderCell, draft, clearDraft: () => setDraft(null),
  }), [entities, annotations, openEntity, annotate, canComment, activeAnnotation, renderCell, draft])

  // Ctrl/Cmd+Z goes to the UndoCoordinator: exactly one step per key press (design §2.7)
  const rootRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    let active = true
    const onPointer = (event: PointerEvent) => { active = Boolean(rootRef.current?.contains(event.target as Node)) }
    const onKey = (event: KeyboardEvent) => {
      if (!(event.ctrlKey || event.metaKey) || event.altKey || event.defaultPrevented) return
      const key = event.key.toLowerCase()
      if (key !== 'z' && key !== 'y') return
      const root = rootRef.current
      const target = event.target as HTMLElement | null
      if (!root || !target) return
      const inside = root.contains(target)
      if (!inside && !(active && (target === document.body || target === document.documentElement))) return
      if (inside && target.closest('input, textarea, select')) return
      event.preventDefault()
      if (key === 'y' || event.shiftKey) void store.undo.redo()
      else void store.undo.undo()
    }
    window.addEventListener('pointerdown', onPointer, true)
    window.addEventListener('keydown', onKey)
    return () => { window.removeEventListener('pointerdown', onPointer, true); window.removeEventListener('keydown', onKey) }
  }, [store])

  return (
    <div ref={rootRef} className="aiws-workspace" data-testid="aiws-workspace" data-workspace-id={store.session.workspaceId} data-session-id={store.session.sessionId} data-top-mode={mode}>
      <TopBar onClose={onClose} offline={offline} mode={mode} onMode={setMode} side={side} onSide={setSide} />
      <Notices />
      {loadError && !loaded && <div className="aiws-error" role="alert">无法读取工作区：{loadError}</div>}
      {loaded && (
        <WorkspaceUiContext.Provider value={ui}>
          <div className="aiws-shell-body">
            <div className="aiws-shell-main">
              {mode === 'sources' ? <DataSourceView selected={selected} onSelect={setSelected} /> : <CanvasView focus={focus} />}
            </div>
            <aside className="aiws-shell-side">
              <EditsPanel />
              {side === 'mock' && <MockRunPanel />}
            </aside>
          </div>
        </WorkspaceUiContext.Provider>
      )}
    </div>
  )
}

function TopBar({ onClose, offline, mode, onMode, side, onSide }: { onClose: () => void; offline: OfflineActions; mode: TopMode; onMode: (mode: TopMode) => void; side: 'none' | 'mock'; onSide: (side: 'none' | 'mock') => void }) {
  const store = useStore()
  const status = useSessionStatus()
  const undo = useSyncExternalStore(store.undo.subscribe, store.undo.snapshot)
  const unsavedInputs = useSyncExternalStore(store.subscribeUnsaved, store.unsavedSnapshot)
  const edits = useEdits()
  const counts = { unsaved: 0, saved_locally: 0, committed: 0, needs_attention: 0 }
  for (const entry of edits.values()) counts[entry.state] += 1
  const info = store.session.info()
  const sessionMode = store.session.mode()
  const replica = store.session.offline
  const storageProblem = replica?.storageProblem() ?? null
  const pendingCount = replica?.pending().length ?? 0
  const top = undo.undo.at(-1)
  const workspaceId = store.session.workspaceId
  const [offlineBusy, setOfflineBusy] = useState<string | null>(null)
  const [offlineError, setOfflineError] = useState<string | null>(null)
  const [confirmDestroy, setConfirmDestroy] = useState(false)
  const appCache = useAppCache()
  useSyncExternalStore(store.userState.subscribe, store.userState.snapshot)
  const run = async (label: string, work: () => Promise<void>) => {
    setOfflineBusy(label)
    setOfflineError(null)
    try { await work() } catch (error) { setOfflineError(offline.describe(error)) } finally { setOfflineBusy(null) }
  }
  const readOnlyDirect = sessionMode.kind === 'direct' && sessionMode.reason !== 'not_prepared' && status.kind === 'offline'
  const connection = status.kind === 'live' ? '已连接'
    : status.kind === 'connecting' ? '连接中'
      : status.kind === 'stopped' ? '已停止同步'
        : `${status.browserOffline ? '浏览器离线' : '后台不可达（浏览器网络在线）'}${sessionMode.kind === 'replica' ? '，修改保存在本设备' : readOnlyDirect ? '，只读' : '，正在重试'}`
  const undoTitle = !top ? '没有可撤销的步骤'
    : `撤销：${top.kind === 'commit' || top.kind === 'pending' || top.kind === 'resubmit' ? top.label : '富文本编辑'}${top.kind === 'pending' ? '（尚未发送，直接从待提交队列移除）' : ''}（Ctrl+Z）`
  return (
    <header className="aiws-topbar">
      <button type="button" data-testid="aiws-back" onClick={onClose}>← 工作区列表</button>
      <b className="aiws-title">{info.title}</b>
      <div className="aiws-mode-switch" role="tablist" aria-label="顶层模式" data-testid="aiws-top-modes">
        <button type="button" role="tab" aria-selected={mode === 'sources'} data-testid="aiws-top-sources" onClick={() => onMode('sources')}>数据源</button>
        <button type="button" role="tab" aria-selected={mode === 'canvas'} data-testid="aiws-top-canvas" onClick={() => onMode('canvas')}>画布</button>
        <button type="button" role="tab" aria-selected={false} disabled title="播放模式本期不实现" data-testid="aiws-top-play">开始演示</button>
      </div>
      <span className={`aiws-conn aiws-conn-${status.kind}`} data-testid="aiws-conn" data-status={status.kind} data-browser-offline={status.kind === 'offline' ? String(status.browserOffline) : undefined}>{connection}</span>
      {sessionMode.kind === 'replica' ? (
        <span className="aiws-chip" data-testid="aiws-mode" data-mode="replica" title={`本窗口持有此工作区的离线副本（准备于 ${sessionMode.preparedAt}）。存储：SQLite WASM / opfs-sahpool`}>离线副本 · 本窗口持有</span>
      ) : sessionMode.reason === 'not_prepared' ? (
        <>
          <span className="aiws-muted" data-testid="aiws-mode" data-mode="direct" data-reason={sessionMode.reason} title={`此窗口未启用离线副本：所有读写直接经过后台。${sessionMode.detail}`}>在线直连</span>
          <button type="button" data-testid="aiws-prepare-offline" disabled={offlineBusy !== null || status.kind !== 'live'} title="把这个工作区下载到本设备，之后断网也能打开和编辑"
            onClick={() => { void run('正在准备离线…', () => offline.prepare(workspaceId, false)) }}>准备离线</button>
        </>
      ) : (
        <>
          <span className="aiws-chip aiws-chip-warn" data-testid="aiws-mode" data-mode="direct" data-reason={sessionMode.reason} title={sessionMode.detail}>
            {sessionMode.reason === 'not_holder' ? '此窗口未启用离线' : '离线不可用'}：{sessionMode.detail}
          </span>
          {sessionMode.reason === 'not_holder' && (
            <button type="button" data-testid="aiws-takeover" disabled={offlineBusy !== null} title="持有离线副本的窗口关闭后，本窗口可以接管"
              onClick={() => { void run('正在接管…', () => offline.reopen(workspaceId)) }}>接管离线副本</button>
          )}
        </>
      )}
      <span className="aiws-muted" data-testid="aiws-userstate-sync" data-sync={store.userState.syncState} title="用户工作状态：服务端按用户与工作区保存，同时保存在浏览器">{store.userState.syncState === 'synced' ? '' : store.userState.syncState === 'offline' ? '工作状态待上送' : ''}</span>
      {offlineBusy && <span className="aiws-muted" data-testid="aiws-offline-busy">{offlineBusy}</span>}
      {store.session.principal && <span className="aiws-muted" data-testid="aiws-principal">{store.session.principal}</span>}
      <span className="aiws-grow" />
      <span className="aiws-save-summary" data-testid="aiws-save-summary" data-unsaved={counts.unsaved} data-local={counts.saved_locally} data-attention={counts.needs_attention} data-pending={pendingCount}>
        {counts.unsaved > 0 && <span className="aiws-state aiws-state-unsaved">{EDIT_STATE_LABEL.unsaved} {counts.unsaved}</span>}
        {counts.saved_locally > 0 && <span className="aiws-state aiws-state-saved_locally">{EDIT_STATE_LABEL.saved_locally} {counts.saved_locally}</span>}
        {counts.needs_attention > 0 && <span className="aiws-state aiws-state-needs_attention">{EDIT_STATE_LABEL.needs_attention} {counts.needs_attention}</span>}
        {counts.unsaved === 0 && counts.saved_locally === 0 && counts.needs_attention === 0 && <span className="aiws-state aiws-state-committed">{counts.committed > 0 ? EDIT_STATE_LABEL.committed : '没有未提交的修改'}</span>}
      </span>
      {(replica || unsavedInputs.size > 0) && (
        <button type="button" data-testid="aiws-export-local" title="把尚未被后台接受的内容（未保存的输入、本机待提交队列、草稿）导出为 JSON 文件" onClick={() => { void store.exportLocal() }}>
          导出本机未提交内容{pendingCount + unsavedInputs.size > 0 ? ` ${pendingCount + unsavedInputs.size}` : ''}
        </button>
      )}
      <button type="button" data-testid="aiws-undo" disabled={undo.undo.length === 0 || undo.busy} title={undoTitle} onClick={() => { void store.undo.undo() }}>撤销 {undo.undo.length}</button>
      <button type="button" data-testid="aiws-redo" disabled={undo.redo.length === 0 || undo.busy} onClick={() => { void store.undo.redo() }}>重做 {undo.redo.length}</button>
      <button type="button" data-testid="aiws-side-mock" aria-pressed={side === 'mock'} onClick={() => onSide(side === 'mock' ? 'none' : 'mock')}>受控加工</button>
      {offlineError && (
        <div className="aiws-error aiws-stop" role="alert" data-testid="aiws-offline-unavailable">
          {offlineError}。此窗口保持在线直连模式。 <button type="button" className="aiws-link" onClick={() => setOfflineError(null)}>知道了</button>
        </div>
      )}
      {readOnlyDirect && (
        <div className="aiws-warning aiws-stop" role="alert" data-testid="aiws-direct-readonly">后台不可达，而此窗口未启用离线（{sessionMode.kind === 'direct' ? sessionMode.detail : ''}）：当前只读，不能编辑。</div>
      )}
      {sessionMode.kind === 'replica' && appCache.unavailable && (
        <div className="aiws-warning aiws-stop" role="status" data-testid="aiws-app-cache-missing">
          工作区数据已在本设备，但应用本身没有缓存（{appCache.unavailable}）：已打开的窗口断网后可以继续工作，关闭后在没有网络时无法重新启动。
        </div>
      )}
      {appCache.updateWaiting && (
        <div className="aiws-warning aiws-stop" role="status" data-testid="aiws-app-update">
          应用的新版本已下载，关闭所有窗口后生效。 <button type="button" className="aiws-link" onClick={applyServiceWorkerUpdate}>现在更新并重新载入</button>
        </div>
      )}
      {sessionMode.kind === 'replica' && !sessionMode.persisted && (
        <div className="aiws-warning aiws-stop" role="status" data-testid="aiws-not-persisted">浏览器没有批准持久化存储：空间紧张时本机副本可能被浏览器清理。浏览器存储不能代替备份，请及时联网提交或导出。</div>
      )}
      {storageProblem && (
        <div className="aiws-error aiws-stop" role="alert" data-testid="aiws-storage-problem">
          本机存储写入失败：{storageProblem}。出现此提示后的修改没有保存到本设备，仍标为“未保存”；请先导出，再重新打开工作区。
          <button type="button" data-testid="aiws-export-unsaved" onClick={() => { void store.exportLocal() }}>导出未保存的输入</button>
          <button type="button" data-testid="aiws-reopen" disabled={offlineBusy !== null} onClick={() => { void run('正在重新打开…', () => offline.reopen(workspaceId)) }}>重新打开工作区</button>
        </div>
      )}
      {status.kind === 'stopped' && (
        <div className="aiws-error aiws-stop" role="alert" data-testid="aiws-stopped">
          {status.detail}
          {replica && (
            <>
              <button type="button" data-testid="aiws-export-pending" onClick={() => { void store.exportLocal() }}>导出待提交内容（{pendingCount} 条）</button>
              {confirmDestroy ? (
                <button type="button" data-testid="aiws-destroy-confirm" disabled={offlineBusy !== null}
                  onClick={() => { void run('正在删除本机副本…', async () => { await replica.destroy(); await offline.reopen(workspaceId) }) }}>确认删除本机副本（{pendingCount} 条待提交将丢失）</button>
              ) : <button type="button" data-testid="aiws-destroy" onClick={() => setConfirmDestroy(true)}>删除本机副本…</button>}
            </>
          )}
        </div>
      )}
      {undo.problem && (
        <div className="aiws-warning aiws-stop" role="alert" data-testid="aiws-undo-problem">
          {undo.problem} <button type="button" className="aiws-link" onClick={() => store.undo.dismissProblem()}>知道了</button>
        </div>
      )}
    </header>
  )
}
