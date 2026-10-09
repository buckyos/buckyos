/* The status area, bottom right of the work area (UI improvement §10): the compact save / sync /
 * connection summary at the bottom, alerts and notices stacked above it. Status-like things — and the
 * expandable work logs to come — live here. Clicking the summary opens the "修改状态" panel (the pending
 * list, connection and offline details, export of what the backend does not have). Problems stay
 * visible: "needs attention" keeps its count on the summary, and stopped sync, storage failures and
 * read-only direct mode are persistent alerts with their recovery actions — never a toast that goes
 * away by itself. */

import { useState, useSyncExternalStore } from 'react'
import { EDIT_STATE_LABEL } from '../../state/edits'
import { unwrap } from '../../api/client'
import { describeError } from '../../api/session'
import { useAppCache, useEdits, useSessionStatus, useShowLock, useStore } from '../../state/hooks'
import { EditsPanel } from './panels'
import { useShell } from './shellContext'

function useSaveCounts() {
  const edits = useEdits()
  const counts = { unsaved: 0, saved_locally: 0, committed: 0, needs_attention: 0 }
  for (const entry of edits.values()) counts[entry.state] += 1
  return counts
}

function useConnection() {
  const store = useStore()
  const status = useSessionStatus()
  const sessionMode = store.session.mode()
  const readOnlyDirect = sessionMode.kind === 'direct' && sessionMode.reason !== 'not_prepared' && status.kind === 'offline'
  const text = status.kind === 'live' ? '已连接'
    : status.kind === 'connecting' ? '连接中'
      : status.kind === 'stopped' ? '已停止同步'
        : `${status.browserOffline ? '浏览器离线' : '后台不可达（浏览器网络在线）'}${sessionMode.kind === 'replica' ? '，修改保存在本设备' : readOnlyDirect ? '，只读' : '，正在重试'}`
  return { status, sessionMode, readOnlyDirect, text }
}

export function StatusSummary() {
  const store = useStore()
  const shell = useShell()
  const counts = useSaveCounts()
  const { status, sessionMode, text } = useConnection()
  const replica = store.session.offline
  const pendingCount = replica?.pending().length ?? 0
  useSyncExternalStore(store.userState.subscribe, store.userState.snapshot)
  const userSync = store.userState.syncState
  const clean = counts.unsaved === 0 && counts.saved_locally === 0 && counts.needs_attention === 0
  return (
    <button type="button" className={`aiws-panel aiws-status${counts.needs_attention > 0 ? ' is-attention' : ''}`} aria-pressed={shell.side === 'edits'} aria-label="修改状态与离线详情" title="修改状态、连接与离线详情"
      data-testid="aiws-status" onClick={() => shell.setSide(shell.side === 'edits' ? null : 'edits')}>
      <span className={`aiws-conn aiws-conn-${status.kind}`} data-testid="aiws-conn" data-status={status.kind} data-browser-offline={status.kind === 'offline' ? String(status.browserOffline) : undefined} title={text}>
        <span className="aiws-dot" aria-hidden="true" />{status.kind !== 'live' && <span>{text}</span>}
      </span>
      {sessionMode.kind === 'replica' ? (
        <span className="aiws-chip" data-testid="aiws-mode" data-mode="replica" title={`本窗口持有此工作区的离线副本（准备于 ${sessionMode.preparedAt}）。存储：SQLite WASM / opfs-sahpool`}>离线副本</span>
      ) : sessionMode.reason === 'not_prepared' ? (
        <span className="aiws-sr-only" data-testid="aiws-mode" data-mode="direct" data-reason={sessionMode.reason}>在线直连</span>
      ) : (
        <span className="aiws-chip aiws-chip-warn" data-testid="aiws-mode" data-mode="direct" data-reason={sessionMode.reason} title={sessionMode.detail}>
          {sessionMode.reason === 'not_holder' ? '此窗口未启用离线' : '离线不可用'}
        </span>
      )}
      <span className="aiws-save-summary" data-testid="aiws-save-summary" data-unsaved={counts.unsaved} data-local={counts.saved_locally} data-attention={counts.needs_attention} data-pending={pendingCount}>
        {counts.needs_attention > 0 && <span className="aiws-state aiws-state-needs_attention">{EDIT_STATE_LABEL.needs_attention} {counts.needs_attention}</span>}
        {counts.unsaved > 0 && <span className="aiws-state aiws-state-unsaved">{EDIT_STATE_LABEL.unsaved} {counts.unsaved}</span>}
        {counts.saved_locally > 0 && <span className="aiws-state aiws-state-saved_locally">{EDIT_STATE_LABEL.saved_locally} {counts.saved_locally}</span>}
        {clean && <span className="aiws-state aiws-state-committed">{counts.committed > 0 ? `${EDIT_STATE_LABEL.committed} ✓` : '没有未提交的修改'}</span>}
      </span>
      {userSync === 'offline' && <span className="aiws-muted" data-testid="aiws-userstate-sync" data-sync={userSync} title="用户工作状态（视口、面板等）待上送">工作状态待同步</span>}
      {shell.offlineBusy && <span className="aiws-muted" data-testid="aiws-offline-busy">{shell.offlineBusy}</span>}
    </button>
  )
}

/** The status area: placed by the view that owns the work area (canvas, data source), clear of the right panel. */
export function StatusDock() {
  return (
    <div className="aiws-status-dock" data-testid="aiws-status-dock">
      <ShellAlerts />
      <StatusSummary />
    </div>
  )
}

/** The "修改状态" panel: connection and offline details, then the edits that are not committed. */
export function StatusDetail() {
  const store = useStore()
  const shell = useShell()
  const { status, sessionMode, text } = useConnection()
  const unsavedInputs = useSyncExternalStore(store.subscribeUnsaved, store.unsavedSnapshot)
  const counts = useSaveCounts()
  const replica = store.session.offline
  const pendingCount = replica?.pending().length ?? 0
  const workspaceId = store.session.workspaceId
  return (
    <div className="aiws-status-detail" data-testid="aiws-status-detail">
      <section>
        <div className="aiws-panel-title">连接与离线</div>
        <div>连接：<b>{text}</b></div>
        <div className="aiws-muted">
          {sessionMode.kind === 'replica' ? `本窗口持有离线副本（准备于 ${sessionMode.preparedAt}）：断网后可继续编辑，修改先保存在本设备，联网后提交。`
            : sessionMode.reason === 'not_prepared' ? `在线直连：所有读写直接经过后台。${sessionMode.detail}`
              : sessionMode.detail}
        </div>
        <div className="aiws-inline-form">
          {sessionMode.kind === 'direct' && sessionMode.reason === 'not_prepared' && (
            <button type="button" data-testid="aiws-prepare-offline" disabled={shell.offlineBusy !== null || status.kind !== 'live'} title="把这个工作区下载到本设备，之后断网也能打开和编辑"
              onClick={() => shell.runOffline('正在准备离线…', () => shell.offline.prepare(workspaceId, false))}>准备离线</button>
          )}
          {sessionMode.kind === 'direct' && sessionMode.reason === 'not_holder' && (
            <button type="button" data-testid="aiws-takeover" disabled={shell.offlineBusy !== null} title="持有离线副本的窗口关闭后，本窗口可以接管"
              onClick={() => shell.runOffline('正在接管…', () => shell.offline.reopen(workspaceId))}>接管离线副本</button>
          )}
          {(replica || unsavedInputs.size > 0) && (
            <button type="button" data-testid="aiws-export-local" title="把尚未被后台接受的内容（未保存的输入、本机待提交队列、草稿）导出为 JSON 文件" onClick={() => { void store.exportLocal() }}>
              导出本机未提交内容{pendingCount + unsavedInputs.size > 0 ? ` ${pendingCount + unsavedInputs.size}` : ''}
            </button>
          )}
        </div>
        {store.userState.syncState === 'offline' && <div className="aiws-muted">工作状态（视口、面板偏好）待同步：联网后自动上送，不影响文档。</div>}
      </section>
      <EditsPanel />
      {counts.unsaved === 0 && counts.saved_locally === 0 && counts.needs_attention === 0 && <div className="aiws-muted" data-testid="aiws-edits-clean">没有未提交的修改。</div>}
    </div>
  )
}

/** Persistent problems and the notices of this window, stacked above the status summary. */
export function ShellAlerts() {
  const store = useStore()
  const shell = useShell()
  const { status, sessionMode, readOnlyDirect } = useConnection()
  const undo = useSyncExternalStore(store.undo.subscribe, store.undo.snapshot)
  const notices = useSyncExternalStore(store.subscribeNotices, store.noticeSnapshot)
  const appCache = useAppCache()
  const replica = store.session.offline
  const storageProblem = replica?.storageProblem() ?? null
  const pendingCount = replica?.pending().length ?? 0
  const workspaceId = store.session.workspaceId
  return (
    <div className="aiws-alerts" aria-live="polite">
      {shell.offlineError && (
        <div className="aiws-error aiws-alert" role="alert" data-testid="aiws-offline-unavailable">
          {shell.offlineError}。此窗口保持在线直连模式。 <button type="button" className="aiws-link" onClick={shell.clearOfflineError}>知道了</button>
        </div>
      )}
      {readOnlyDirect && (
        <div className="aiws-warning aiws-alert" role="alert" data-testid="aiws-direct-readonly">后台不可达，而此窗口未启用离线（{sessionMode.kind === 'direct' ? sessionMode.detail : ''}）：当前只读，不能编辑。</div>
      )}
      <ShowLockAlert />
      {sessionMode.kind === 'replica' && appCache.unavailable && (
        <div className="aiws-warning aiws-alert" role="status" data-testid="aiws-app-cache-missing">
          工作区数据已在本设备，但应用本身没有缓存（{appCache.unavailable}）：已打开的窗口断网后可以继续工作，关闭后在没有网络时无法重新启动。
        </div>
      )}
      {sessionMode.kind === 'replica' && !sessionMode.persisted && (
        <div className="aiws-warning aiws-alert" role="status" data-testid="aiws-not-persisted">浏览器没有批准持久化存储：空间紧张时本机副本可能被浏览器清理。浏览器存储不能代替备份，请及时联网提交或导出。</div>
      )}
      {storageProblem && (
        <div className="aiws-error aiws-alert" role="alert" data-testid="aiws-storage-problem">
          本机存储写入失败：{storageProblem}。出现此提示后的修改没有保存到本设备，仍标为“未保存”；请先导出，再重新打开工作区。
          <button type="button" data-testid="aiws-export-unsaved" onClick={() => { void store.exportLocal() }}>导出未保存的输入</button>
          <button type="button" data-testid="aiws-reopen" disabled={shell.offlineBusy !== null} onClick={() => shell.runOffline('正在重新打开…', () => shell.offline.reopen(workspaceId))}>重新打开工作区</button>
        </div>
      )}
      {status.kind === 'stopped' && <StoppedAlert detail={status.detail} pendingCount={pendingCount} />}
      {undo.problem && (
        <div className="aiws-warning aiws-alert" role="alert" data-testid="aiws-undo-problem">
          {undo.problem} <button type="button" className="aiws-link" onClick={() => store.undo.dismissProblem()}>知道了</button>
        </div>
      )}
      {notices.map((notice) => (
        <div key={notice.id} className={`${notice.kind === 'error' ? 'aiws-error' : 'aiws-warning'} aiws-alert`} role={notice.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-notice">
          {notice.text}
          {notice.action && <button type="button" className="aiws-link" data-testid="aiws-notice-action" onClick={() => { notice.action?.run(); store.dismissNotice(notice.id) }}>{notice.action.label}</button>}
          <button type="button" className="aiws-link" onClick={() => store.dismissNotice(notice.id)}>关闭</button>
        </div>
      ))}
    </div>
  )
}

/** The workspace is being presented (第三期规划 §8.1): writes are closed until the show ends; a manager may end it. */
function ShowLockAlert() {
  const store = useStore()
  const shell = useShell()
  const lock = useShowLock()
  const [ending, setEnding] = useState(false)
  if (!lock || shell.topMode === 'show') return null
  const started = lock.started_at ? new Date(lock.started_at).toLocaleTimeString() : ''
  const manager = store.session.info().capabilities.includes('manage')
  const replica = store.session.offline !== null
  return (
    <div className="aiws-warning aiws-alert" role="status" data-testid="aiws-show-locked" data-presenter={lock.presenter}>
      {lock.presenter} 正在放映这个工作区{started ? `（${started} 开始）` : ''}：放映期间关闭写入{replica ? '，本设备的修改在放映结束后自动发送' : '，放映结束后恢复'}。
      {manager && (
        <button type="button" className="aiws-link" data-testid="aiws-show-force-end" disabled={ending} onClick={() => {
          setEnding(true)
          void shell.client.showEnd({ workspace_id: store.session.workspaceId }, lock.show_id).then((r) => unwrap(r)).then(
            () => { store.session.noteShowLock(null); store.notify('info', '已结束放映：工作区恢复写入。') },
            (error: unknown) => store.notify('error', `没有结束放映：${describeError(error)}`),
          ).finally(() => setEnding(false))
        }}>结束放映</button>
      )}
    </div>
  )
}

function StoppedAlert({ detail, pendingCount }: { detail: string; pendingCount: number }) {
  const store = useStore()
  const shell = useShell()
  const replica = store.session.offline
  const workspaceId = store.session.workspaceId
  return (
    <div className="aiws-error aiws-alert" role="alert" data-testid="aiws-stopped">
      {detail}
      {replica && <StoppedActions pendingCount={pendingCount} onDestroy={() => shell.runOffline('正在删除本机副本…', async () => { await replica.destroy(); await shell.offline.reopen(workspaceId) })} />}
    </div>
  )
}

function StoppedActions({ pendingCount, onDestroy }: { pendingCount: number; onDestroy: () => void }) {
  const store = useStore()
  const shell = useShell()
  return (
    <>
      <button type="button" data-testid="aiws-export-pending" onClick={() => { void store.exportLocal() }}>导出待提交内容（{pendingCount} 条）</button>
      <ConfirmButton testId="aiws-destroy" confirmTestId="aiws-destroy-confirm" label="删除本机副本…" confirmLabel={`确认删除本机副本（${pendingCount} 条待提交将丢失）`} disabled={shell.offlineBusy !== null} onConfirm={onDestroy} />
    </>
  )
}


function ConfirmButton({ testId, confirmTestId, label, confirmLabel, disabled, onConfirm }: { testId: string; confirmTestId: string; label: string; confirmLabel: string; disabled?: boolean; onConfirm: () => void }) {
  const [asked, setAsked] = useState(false)
  return asked
    ? <button type="button" data-testid={confirmTestId} disabled={disabled} onClick={onConfirm}>{confirmLabel}</button>
    : <button type="button" data-testid={testId} onClick={() => setAsked(true)}>{label}</button>
}
