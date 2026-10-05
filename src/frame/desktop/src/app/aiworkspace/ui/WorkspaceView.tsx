/* One open workspace: top bar (status, undo, save states), outline, the flow page and side panels. */

import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
import { describeError, type ReadOk } from '../api/session'
import type { AnnotationContent, CellPayload, EntityEnvelope, KeyedContent, Reference } from '../api/types'
import { orderKeyBetween } from '../api/wasm'
import { EDIT_STATE_LABEL } from '../state/edits'
import { applyServiceWorkerUpdate } from '../../../serviceWorker'
import { StoreContext, WorkspaceUiContext, useAppCache, useEdits, useLoad, useSessionStatus, useStore, useVersion, useWorkspaceUi, type AnnotationMark, type WorkspaceUi } from '../state/hooks'
import type { WorkspaceStore } from '../state/store'
import { CellBody } from './cells'
import { assetOps, creationOps, entityLabel, sortedChildren, type NewKind } from './creators'
import { AnnotationsPanel, EditsPanel, MockRunPanel, OutlinePanel } from './panels'

/** What the top bar can ask of the app panel: prepare the offline replica, reopen the workspace in the mode that is possible now. */
export interface OfflineActions {
  prepare(workspaceId: string, discardLocal: boolean): Promise<void>
  reopen(workspaceId: string): Promise<void>
  describe(failure: unknown): string
}

export function WorkspaceView({ store, offline, onClose }: { store: WorkspaceStore; offline: OfflineActions; onClose: () => void }) {
  return (
    <StoreContext.Provider value={store}>
      <WorkspaceShell onClose={onClose} offline={offline} />
    </StoreContext.Provider>
  )
}

function useAnnotations(entities: EntityEnvelope[] | undefined): AnnotationMark[] {
  const store = useStore()
  const any = useVersion('any')
  const ids = useMemo(() => (entities ?? []).filter((entity) => entity.type_id === 'buckyos.annotation' && !entity.deleted).map((entity) => entity.entity_id), [entities])
  const key = ids.join(',')
  const load = useCallback(async (): Promise<AnnotationMark[]> => {
    const list = key === '' ? [] : key.split(',')
    if (list.length === 0) return []
    const results = await store.session.readMany(list.map((entity_id) => ({ entity_id })))
    return results.flatMap((result, index) => {
      if ('error' in result) return []
      const content = result.content as AnnotationContent
      return [{ entityId: list[index], payload: content.payload, anchorState: content.anchor_state }]
    })
  }, [store, key])
  return useLoad(load, any).data ?? []
}

function WorkspaceShell({ onClose, offline }: { onClose: () => void; offline: OfflineActions }) {
  const store = useStore()
  const outlineVersion = useVersion('outline')
  const loadOutline = useCallback(() => store.session.outline(), [store])
  const outline = useLoad(loadOutline, outlineVersion)
  const entities = outline.data
  const annotations = useAnnotations(entities)
  const [selected, setSelected] = useState<string | null>(null)
  const [draft, setDraft] = useState<{ target: Reference; label: string } | null>(null)
  const [side, setSide] = useState<'annotations' | 'mock'>('annotations')

  const lockRequired = (entities ?? []).some((entity) => entity.write_policy === 'lock_required')
  useEffect(() => {
    store.setLockPolling(lockRequired)
    return () => store.setLockPolling(false)
  }, [store, lockRequired])

  const openEntity = useCallback((entityId: string) => {
    setSelected(entityId)
    document.querySelector(`[data-cell-source="${CSS.escape(entityId)}"], [data-cell-id="${CSS.escape(entityId)}"]`)?.scrollIntoView({ block: 'center', behavior: 'smooth' })
  }, [])
  const annotate = useCallback((target: Reference, label: string) => { setDraft({ target, label }); setSide('annotations') }, [])
  const ui = useMemo<WorkspaceUi | null>(() => entities ? {
    entities, byId: new Map(entities.map((entity) => [entity.entity_id, entity])), annotations, openEntity, annotate,
  } : null, [entities, annotations, openEntity, annotate])

  // Ctrl/Cmd+Z goes to the UndoCoordinator: exactly one step per key press (design §2.7). The listener
  // is on the window because focus falls back to <body> when an inline editor closes; it only acts
  // while this workspace is the surface the user is working in. Plain form fields keep their own text
  // undo while they are being typed in; the rich text editor routes its undo keys here itself.
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
    return () => {
      window.removeEventListener('pointerdown', onPointer, true)
      window.removeEventListener('keydown', onKey)
    }
  }, [store])

  const pages = entities ? sortedChildren(entities, 'root').filter((entity) => entity.kind === 'page') : []
  const page = pages[0] ?? null

  return (
    <div ref={rootRef} className="aiws-workspace" data-testid="aiws-workspace" data-workspace-id={store.session.workspaceId} data-session-id={store.session.sessionId}>
      <TopBar onClose={onClose} side={side} onSide={setSide} offline={offline} />
      <Notices />
      {outline.error && !entities && <div className="aiws-error" role="alert">无法读取工作区：{outline.error}</div>}
      {ui && (
        <WorkspaceUiContext.Provider value={ui}>
          <div className="aiws-columns">
            <aside className="aiws-left">
              <OutlinePanel selected={selected} onSelect={setSelected} />
            </aside>
            <main className="aiws-page" data-testid="aiws-page">
              {page ? <PageFlow page={page} selected={selected} onSelect={setSelected} /> : <NoPage />}
            </main>
            <aside className="aiws-right">
              <EditsPanel />
              {side === 'annotations'
                ? <AnnotationsPanel pageId={page?.entity_id ?? null} draft={draft} onDraftDone={() => setDraft(null)} />
                : <MockRunPanel />}
            </aside>
          </div>
        </WorkspaceUiContext.Provider>
      )}
    </div>
  )
}

function NoPage() {
  const store = useStore()
  const { entities } = useWorkspaceUi()
  return (
    <div className="aiws-empty">
      <p>这个工作区还没有工作页。</p>
      <button type="button" data-testid="aiws-create-page" onClick={() => {
        let key: string
        try { key = orderKeyBetween(store.core, sortedChildren(entities, 'root').at(-1)?.order_key, null) } catch (error) { store.notify('error', describeError(error)); return }
        void store.submit({ editId: 'entity:new-page', label: '创建工作页', operations: [{ op: 'entity.create', entity_id: 'page-main', type_id: 'buckyos.container', parent_id: 'root', order_key: key, payload: { kind: 'page', layout: { mode: 'flow' }, title: '工作页' } }] })
      }}>创建工作页</button>
    </div>
  )
}

function TopBar({ onClose, side, onSide, offline }: { onClose: () => void; side: 'annotations' | 'mock'; onSide: (side: 'annotations' | 'mock') => void; offline: OfflineActions }) {
  const store = useStore()
  const status = useSessionStatus()
  const undo = useSyncExternalStore(store.undo.subscribe, store.undo.snapshot)
  const unsavedInputs = useSyncExternalStore(store.subscribeUnsaved, store.unsavedSnapshot)
  const edits = useEdits()
  const counts = { unsaved: 0, saved_locally: 0, committed: 0, needs_attention: 0 }
  for (const entry of edits.values()) counts[entry.state] += 1
  const info = store.session.info()
  const mode = store.session.mode()
  const replica = store.session.offline
  const storageProblem = replica?.storageProblem() ?? null
  const pendingCount = replica?.pending().length ?? 0
  const top = undo.undo.at(-1)
  const workspaceId = store.session.workspaceId
  const [offlineBusy, setOfflineBusy] = useState<string | null>(null)
  const [offlineError, setOfflineError] = useState<string | null>(null)
  const [confirmDestroy, setConfirmDestroy] = useState(false)
  const appCache = useAppCache()
  const run = async (label: string, work: () => Promise<void>) => {
    setOfflineBusy(label)
    setOfflineError(null)
    try { await work() } catch (error) { setOfflineError(offline.describe(error)) } finally { setOfflineBusy(null) }
  }
  const readOnlyDirect = mode.kind === 'direct' && mode.reason !== 'not_prepared' && status.kind === 'offline'
  const connection = status.kind === 'live' ? '已连接'
    : status.kind === 'connecting' ? '连接中'
      : status.kind === 'stopped' ? '已停止同步'
        : `${status.browserOffline ? '浏览器离线' : '后台不可达（浏览器网络在线）'}${mode.kind === 'replica' ? '，修改保存在本设备' : readOnlyDirect ? '，只读' : '，正在重试'}`
  const undoTitle = !top ? '没有可撤销的步骤'
    : `撤销：${top.kind === 'commit' || top.kind === 'pending' || top.kind === 'resubmit' ? top.label : '富文本编辑'}${top.kind === 'pending' ? '（尚未发送，直接从待提交队列移除）' : ''}（Ctrl+Z）`
  return (
    <header className="aiws-topbar">
      <button type="button" data-testid="aiws-back" onClick={onClose}>← 工作区列表</button>
      <b className="aiws-title">{info.title}</b>
      <span className={`aiws-conn aiws-conn-${status.kind}`} data-testid="aiws-conn" data-status={status.kind} data-browser-offline={status.kind === 'offline' ? String(status.browserOffline) : undefined}>{connection}</span>
      {mode.kind === 'replica' ? (
        <span className="aiws-chip" data-testid="aiws-mode" data-mode="replica" title={`本窗口持有此工作区的离线副本（准备于 ${mode.preparedAt}）。存储：SQLite WASM / opfs-sahpool`}>离线副本 · 本窗口持有</span>
      ) : mode.reason === 'not_prepared' ? (
        <>
          <span className="aiws-muted" data-testid="aiws-mode" data-mode="direct" data-reason={mode.reason} title={`此窗口未启用离线副本：所有读写直接经过后台。${mode.detail}`}>在线直连</span>
          <button type="button" data-testid="aiws-prepare-offline" disabled={offlineBusy !== null || status.kind !== 'live'} title="把这个工作区下载到本设备，之后断网也能打开和编辑"
            onClick={() => { void run('正在准备离线…', () => offline.prepare(workspaceId, false)) }}>准备离线</button>
        </>
      ) : (
        <>
          <span className="aiws-chip aiws-chip-warn" data-testid="aiws-mode" data-mode="direct" data-reason={mode.reason} title={mode.detail}>
            {mode.reason === 'not_holder' ? '此窗口未启用离线' : '离线不可用'}：{mode.detail}
          </span>
          {mode.reason === 'not_holder' && (
            <button type="button" data-testid="aiws-takeover" disabled={offlineBusy !== null} title="持有离线副本的窗口关闭后，本窗口可以接管"
              onClick={() => { void run('正在接管…', () => offline.reopen(workspaceId)) }}>接管离线副本</button>
          )}
        </>
      )}
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
      <button type="button" data-testid="aiws-side-annotations" aria-pressed={side === 'annotations'} onClick={() => onSide('annotations')}>批注</button>
      <button type="button" data-testid="aiws-side-mock" aria-pressed={side === 'mock'} onClick={() => onSide('mock')}>受控加工</button>
      {offlineError && (
        <div className="aiws-error aiws-stop" role="alert" data-testid="aiws-offline-unavailable">
          {offlineError}。此窗口保持在线直连模式。 <button type="button" className="aiws-link" onClick={() => setOfflineError(null)}>知道了</button>
        </div>
      )}
      {readOnlyDirect && (
        <div className="aiws-warning aiws-stop" role="alert" data-testid="aiws-direct-readonly">后台不可达，而此窗口未启用离线（{mode.kind === 'direct' ? mode.detail : ''}）：当前只读，不能编辑。</div>
      )}
      {mode.kind === 'replica' && appCache.unavailable && (
        <div className="aiws-warning aiws-stop" role="status" data-testid="aiws-app-cache-missing">
          工作区数据已在本设备，但应用本身没有缓存（{appCache.unavailable}）：已打开的窗口断网后可以继续工作，关闭后在没有网络时无法重新启动。
        </div>
      )}
      {appCache.updateWaiting && (
        <div className="aiws-warning aiws-stop" role="status" data-testid="aiws-app-update">
          应用的新版本已下载，关闭所有窗口后生效。 <button type="button" className="aiws-link" onClick={applyServiceWorkerUpdate}>现在更新并重新载入</button>
        </div>
      )}
      {mode.kind === 'replica' && !mode.persisted && (
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

function Notices() {
  const store = useStore()
  const notices = useSyncExternalStore(store.subscribeNotices, store.noticeSnapshot)
  if (notices.length === 0) return null
  return (
    <div className="aiws-notices">
      {notices.map((notice) => (
        <div key={notice.id} className={notice.kind === 'error' ? 'aiws-error' : 'aiws-warning'} role={notice.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-notice">
          {notice.text} <button type="button" className="aiws-link" onClick={() => store.dismissNotice(notice.id)}>关闭</button>
        </div>
      ))}
    </div>
  )
}

function PageFlow({ page, selected, onSelect }: { page: EntityEnvelope; selected: string | null; onSelect: (entityId: string | null) => void }) {
  return (
    <div className="aiws-flow">
      <h2 className="aiws-page-title">{page.title ?? page.name ?? '工作页'}</h2>
      <ContainerFlow container={page} selected={selected} onSelect={onSelect} />
    </div>
  )
}

/** Renders the cells and groups of a container in `(order_key, entity_id)` order; content entities live in the outline only. */
function ContainerFlow({ container, selected, onSelect }: { container: EntityEnvelope; selected: string | null; onSelect: (entityId: string | null) => void }) {
  const { entities } = useWorkspaceUi()
  const shown = sortedChildren(entities, container.entity_id).filter((entity) => entity.type_id === 'buckyos.cell' || (entity.type_id === 'buckyos.container' && entity.kind === 'group'))
  return (
    <>
      {shown.map((entity, index) => (
        entity.type_id === 'buckyos.cell'
          ? <CellFrame key={entity.entity_id} cell={entity} siblings={shown} index={index} selected={selected === entity.entity_id} onSelect={onSelect} />
          : (
            <section key={entity.entity_id} className={`aiws-group${selected === entity.entity_id ? ' is-selected' : ''}`} data-testid={`aiws-group-${entity.entity_id}`} data-cell-id={entity.entity_id}>
              <header className="aiws-cell-head">
                <button type="button" className="aiws-cell-title" onClick={() => onSelect(entity.entity_id)}>分组 · {entityLabel(entity)}</button>
                <span className="aiws-grow" />
                <ReorderButtons entity={entity} siblings={shown} index={index} />
              </header>
              <ContainerFlow container={entity} selected={selected} onSelect={onSelect} />
              <AddMenu parent={entity} />
            </section>
          )
      ))}
      {container.kind === 'page' && <AddMenu parent={container} />}
    </>
  )
}

function ReorderButtons({ entity, siblings, index }: { entity: EntityEnvelope; siblings: EntityEnvelope[]; index: number }) {
  const store = useStore()
  const { entities } = useWorkspaceUi()
  if (!entity.capabilities.includes('structure')) return null
  const move = (direction: -1 | 1) => {
    // Keys are taken between the *actual* neighbours (content entities included), so the result is the visible order.
    const all = sortedChildren(entities, entity.parent_id ?? '')
    const target = siblings[index + direction]
    const at = all.findIndex((item) => item.entity_id === target.entity_id)
    const before = direction === -1 ? all[at - 1]?.order_key : all[at]?.order_key
    const after = direction === -1 ? all[at]?.order_key : all[at + 1]?.order_key
    let key: string
    try { key = orderKeyBetween(store.core, before, after) } catch (error) { store.notify('error', `无法生成顺序键：${describeError(error)}`); return }
    // tree.place without `expect`: reordering merges automatically, the later arrival wins (design §3.1).
    void store.submit({ editId: `entity:${entity.entity_id}`, label: `调整顺序 ${entityLabel(entity)}`, operations: [{ op: 'tree.place', entity_id: entity.entity_id, order_key: key }] })
  }
  return (
    <>
      <button type="button" className="aiws-icon" title="上移" aria-label="上移" data-testid={`aiws-up-${entity.entity_id}`} disabled={index === 0} onClick={() => move(-1)}>↑</button>
      <button type="button" className="aiws-icon" title="下移" aria-label="下移" data-testid={`aiws-down-${entity.entity_id}`} disabled={index === siblings.length - 1} onClick={() => move(1)}>↓</button>
    </>
  )
}

function CellFrame({ cell, siblings, index, selected, onSelect }: { cell: EntityEnvelope; siblings: EntityEnvelope[]; index: number; selected: boolean; onSelect: (entityId: string | null) => void }) {
  const store = useStore()
  const { byId } = useWorkspaceUi()
  const version = useVersion(`e:${cell.entity_id}`)
  const load = useCallback(() => store.session.read<KeyedContent<CellPayload>>(cell.entity_id), [store, cell.entity_id])
  const read = useLoad<ReadOk<KeyedContent<CellPayload>>>(load, version)
  const payload = read.data?.content.payload
  const source = payload ? byId.get(payload.source_ref.entity_id) : undefined
  const title = payload?.title ?? (source ? entityLabel(source) : cell.entity_id)
  const [renaming, setRenaming] = useState<string | null>(null)
  return (
    <section className={`aiws-cell${selected ? ' is-selected' : ''}`} data-testid={`aiws-cell-frame-${cell.entity_id}`} data-cell-id={cell.entity_id} data-cell-source={payload?.source_ref.entity_id} data-view={payload?.view.type}>
      <header className="aiws-cell-head">
        {renaming !== null ? (
          <form onSubmit={(event) => {
            event.preventDefault()
            const next = renaming.trim()
            setRenaming(null)
            if (!read.data || next === (payload?.title ?? '')) return
            void store.submit({
              editId: `key:${cell.entity_id}:title`, label: `单元标题 → ${next || '（清除）'}`, mine: next, hasMine: true,
              operations: [next === ''
                ? { op: 'entity.unset_keys', entity_id: cell.entity_id, keys: [{ key: 'title', expect: { rev: read.data.content.key_revs.title ?? 0 } }] }
                : { op: 'entity.set_keys', entity_id: cell.entity_id, keys: [{ key: 'title', value: next, expect: { rev: read.data.content.key_revs.title ?? 0 } }] }],
            })
          }}>
            <input aria-label="单元标题" autoFocus value={renaming} onChange={(event) => setRenaming(event.target.value)} onBlur={() => setRenaming(null)} />
          </form>
        ) : (
          <button type="button" className="aiws-cell-title" data-testid={`aiws-cell-title-${cell.entity_id}`} onClick={() => onSelect(cell.entity_id)} onDoubleClick={() => { if (cell.capabilities.includes('update')) setRenaming(payload?.title ?? '') }} title="单击选中，双击改标题">{title}</button>
        )}
        <span className="aiws-muted">{payload?.view.type === 'table' ? '表格视图' : payload?.view.type === 'richtext' ? '富文本' : payload?.view.type === 'record' ? '记录' : payload?.view.type === 'asset' ? '资产' : ''}</span>
        <span className="aiws-grow" />
        <ReorderButtons entity={cell} siblings={siblings} index={index} />
        {cell.capabilities.includes('delete') && (
          <button type="button" className="aiws-icon" title="删除此单元（不删除它引用的数据）" aria-label="删除单元" onClick={() => {
            void store.submit({ editId: `entity:${cell.entity_id}`, label: `删除单元 ${title}`, operations: [{ op: 'entity.delete', entity_id: cell.entity_id, expect: { rev: cell.life_rev } }] })
          }}>×</button>
        )}
      </header>
      <div className="aiws-cell-body"><CellBody cellId={cell.entity_id} /></div>
    </section>
  )
}

function AddMenu({ parent }: { parent: EntityEnvelope }) {
  const store = useStore()
  const { entities } = useWorkspaceUi()
  const [open, setOpen] = useState(false)
  const [title, setTitle] = useState('')
  const [sourceId, setSourceId] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  if (!parent.capabilities.includes('structure')) return null
  const sources = entities.filter((entity) => entity.type_id === 'buckyos.table-source' && !entity.deleted)
  const add = (kind: NewKind) => {
    const operations = creationOps(store.core, entities, parent.entity_id, kind, title.trim(), sourceId || undefined)
    if (operations.length === 0) return
    void store.submit({ editId: `entity:new:${parent.entity_id}`, label: `新建${{ group: '分组', richtext: '富文本', table: '表格', record: '记录', view: '表格视图' }[kind]}`, operations })
      .then((outcome) => { if (outcome.status === 'accepted') { setOpen(false); setTitle('') } })
  }
  const addImage = async (file: File) => {
    setBusy('上传中…')
    try {
      const uploaded = await store.session.uploadAsset(file, file.name)
      const outcome = await store.submit({ editId: `entity:new:${parent.entity_id}`, label: `添加图片 ${file.name}`, operations: assetOps(store.core, entities, parent.entity_id, uploaded.object_id, file.name) })
      setBusy(outcome.status === 'accepted' ? null : '未被接受，见“需要处理”')
      if (outcome.status === 'accepted') setOpen(false)
    } catch (error) {
      setBusy(`上传失败：${describeError(error)}`)
    }
  }
  return (
    <div className="aiws-add" data-testid={`aiws-add-${parent.entity_id}`}>
      {!open ? <button type="button" data-testid={`aiws-add-open-${parent.entity_id}`} onClick={() => setOpen(true)}>+ 添加到「{entityLabel(parent)}」</button> : (
        <div className="aiws-inline-form">
          <input aria-label="标题" placeholder="标题（可选）" value={title} onChange={(event) => setTitle(event.target.value)} />
          <button type="button" data-testid="aiws-new-richtext" onClick={() => add('richtext')}>富文本</button>
          <button type="button" data-testid="aiws-new-table" onClick={() => add('table')}>表格</button>
          <button type="button" data-testid="aiws-new-record" onClick={() => add('record')}>记录</button>
          <button type="button" data-testid="aiws-new-group" onClick={() => add('group')}>分组</button>
          <label className="aiws-link">图片…
            <input type="file" hidden accept="image/*" data-testid="aiws-new-image" onChange={(event) => { const file = event.target.files?.[0]; event.target.value = ''; if (file) void addImage(file) }} />
          </label>
          {sources.length > 0 && (
            <>
              <select aria-label="已有数据源" value={sourceId} onChange={(event) => setSourceId(event.target.value)}>
                <option value="">已有表…</option>
                {sources.map((source) => <option key={source.entity_id} value={source.entity_id}>{entityLabel(source)}</option>)}
              </select>
              <button type="button" data-testid="aiws-new-view" disabled={!sourceId} onClick={() => add('view')}>为它添加视图</button>
            </>
          )}
          <button type="button" onClick={() => setOpen(false)}>收起</button>
          {busy && <span>{busy}</span>}
        </div>
      )}
    </div>
  )
}
