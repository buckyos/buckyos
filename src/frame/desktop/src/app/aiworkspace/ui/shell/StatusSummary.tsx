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
import { useAiwsText } from '../../useAiwsText'
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
  const { t } = useAiwsText()
  const status = useSessionStatus()
  const sessionMode = store.session.mode()
  const readOnlyDirect = sessionMode.kind === 'direct' && sessionMode.reason !== 'not_prepared' && status.kind === 'offline'
  const cause = status.kind === 'offline' && status.browserOffline ? t('aiworkspace.shell.conn.browserOffline') : t('aiworkspace.shell.conn.serverUnreachable')
  const text = status.kind === 'live' ? t('aiworkspace.shell.conn.live')
    : status.kind === 'connecting' ? t('aiworkspace.shell.conn.connecting')
      : status.kind === 'stopped' ? t('aiworkspace.shell.conn.stopped')
        : sessionMode.kind === 'replica' ? t('aiworkspace.shell.conn.offlineReplica', { cause })
          : readOnlyDirect ? t('aiworkspace.shell.conn.offlineReadOnly', { cause }) : t('aiworkspace.shell.conn.offlineRetrying', { cause })
  return { status, sessionMode, readOnlyDirect, text }
}

export function StatusSummary() {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const counts = useSaveCounts()
  const { status, sessionMode, text } = useConnection()
  const replica = store.session.offline
  const pendingCount = replica?.pending().length ?? 0
  useSyncExternalStore(store.userState.subscribe, store.userState.snapshot)
  const userSync = store.userState.syncState
  const clean = counts.unsaved === 0 && counts.saved_locally === 0 && counts.needs_attention === 0
  return (
    <button type="button" className={`aiws-panel aiws-status${counts.needs_attention > 0 ? ' is-attention' : ''}`} aria-pressed={shell.side === 'edits'} aria-label={t('aiworkspace.shell.status.label')} title={t('aiworkspace.shell.status.title')}
      data-testid="aiws-status" onClick={() => shell.setSide(shell.side === 'edits' ? null : 'edits')}>
      <span className={`aiws-conn aiws-conn-${status.kind}`} data-testid="aiws-conn" data-status={status.kind} data-browser-offline={status.kind === 'offline' ? String(status.browserOffline) : undefined} title={text}>
        <span className="aiws-dot" aria-hidden="true" />{status.kind !== 'live' && <span>{text}</span>}
      </span>
      {sessionMode.kind === 'replica' ? (
        <span className="aiws-chip" data-testid="aiws-mode" data-mode="replica" title={t('aiworkspace.shell.status.replicaTitle', { at: sessionMode.preparedAt })}>{t('aiworkspace.shell.status.replica')}</span>
      ) : sessionMode.reason === 'not_prepared' ? (
        <span className="aiws-sr-only" data-testid="aiws-mode" data-mode="direct" data-reason={sessionMode.reason}>{t('aiworkspace.shell.status.direct')}</span>
      ) : (
        <span className="aiws-chip aiws-chip-warn" data-testid="aiws-mode" data-mode="direct" data-reason={sessionMode.reason} title={sessionMode.detail}>
          {sessionMode.reason === 'not_holder' ? t('aiworkspace.shell.status.notHolder') : t('aiworkspace.shell.status.offlineUnavailable')}
        </span>
      )}
      <span className="aiws-save-summary" data-testid="aiws-save-summary" data-unsaved={counts.unsaved} data-local={counts.saved_locally} data-attention={counts.needs_attention} data-pending={pendingCount}>
        {counts.needs_attention > 0 && <span className="aiws-state aiws-state-needs_attention">{EDIT_STATE_LABEL.needs_attention} {counts.needs_attention}</span>}
        {counts.unsaved > 0 && <span className="aiws-state aiws-state-unsaved">{EDIT_STATE_LABEL.unsaved} {counts.unsaved}</span>}
        {counts.saved_locally > 0 && <span className="aiws-state aiws-state-saved_locally">{EDIT_STATE_LABEL.saved_locally} {counts.saved_locally}</span>}
        {clean && <span className="aiws-state aiws-state-committed">{counts.committed > 0 ? `${EDIT_STATE_LABEL.committed} ✓` : t('aiworkspace.shell.status.clean')}</span>}
      </span>
      {userSync === 'offline' && <span className="aiws-muted" data-testid="aiws-userstate-sync" data-sync={userSync} title={t('aiworkspace.shell.status.userSyncTitle')}>{t('aiworkspace.shell.status.userSync')}</span>}
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
  const { t, tNode } = useAiwsText()
  const { status, sessionMode, text } = useConnection()
  const unsavedInputs = useSyncExternalStore(store.subscribeUnsaved, store.unsavedSnapshot)
  const counts = useSaveCounts()
  const replica = store.session.offline
  const pendingCount = replica?.pending().length ?? 0
  const workspaceId = store.session.workspaceId
  return (
    <div className="aiws-status-detail" data-testid="aiws-status-detail">
      <section>
        <div className="aiws-panel-title">{t('aiworkspace.shell.detail.title')}</div>
        <div>{tNode('aiworkspace.shell.detail.connection', { state: <b>{text}</b> })}</div>
        <div className="aiws-muted">
          {sessionMode.kind === 'replica' ? t('aiworkspace.shell.detail.replica', { at: sessionMode.preparedAt })
            : sessionMode.reason === 'not_prepared' ? t('aiworkspace.shell.detail.direct', { detail: sessionMode.detail })
              : sessionMode.detail}
        </div>
        <div className="aiws-inline-form">
          {sessionMode.kind === 'direct' && sessionMode.reason === 'not_prepared' && (
            <button type="button" data-testid="aiws-prepare-offline" disabled={shell.offlineBusy !== null || status.kind !== 'live'} title={t('aiworkspace.shell.detail.prepareHint')}
              onClick={() => shell.runOffline(t('aiworkspace.shell.preparingOffline'), () => shell.offline.prepare(workspaceId, false))}>{t('aiworkspace.shell.prepareOffline')}</button>
          )}
          {sessionMode.kind === 'direct' && sessionMode.reason === 'not_holder' && (
            <button type="button" data-testid="aiws-takeover" disabled={shell.offlineBusy !== null} title={t('aiworkspace.shell.detail.takeoverHint')}
              onClick={() => shell.runOffline(t('aiworkspace.shell.takingOver'), () => shell.offline.reopen(workspaceId))}>{t('aiworkspace.shell.takeover')}</button>
          )}
          {(replica || unsavedInputs.size > 0) && (
            <button type="button" data-testid="aiws-export-local" title={t('aiworkspace.shell.detail.exportLocalHint')} onClick={() => { void store.exportLocal() }}>
              {pendingCount + unsavedInputs.size > 0 ? t('aiworkspace.shell.detail.exportLocalCount', { count: pendingCount + unsavedInputs.size }) : t('aiworkspace.shell.exportLocal')}
            </button>
          )}
        </div>
        {store.userState.syncState === 'offline' && <div className="aiws-muted">{t('aiworkspace.shell.detail.userSync')}</div>}
      </section>
      <EditsPanel />
      {counts.unsaved === 0 && counts.saved_locally === 0 && counts.needs_attention === 0 && <div className="aiws-muted" data-testid="aiws-edits-clean">{t('aiworkspace.shell.detail.clean')}</div>}
    </div>
  )
}

/** Persistent problems and the notices of this window, stacked above the status summary. */
export function ShellAlerts() {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
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
          {t('aiworkspace.shell.alert.offlineError', { error: shell.offlineError })} <button type="button" className="aiws-link" onClick={shell.clearOfflineError}>{t('aiworkspace.shell.gotIt')}</button>
        </div>
      )}
      {readOnlyDirect && (
        <div className="aiws-warning aiws-alert" role="alert" data-testid="aiws-direct-readonly">{t('aiworkspace.shell.alert.directReadOnly', { detail: sessionMode.kind === 'direct' ? sessionMode.detail : '' })}</div>
      )}
      <ShowLockAlert />
      {sessionMode.kind === 'replica' && appCache.unavailable && (
        <div className="aiws-warning aiws-alert" role="status" data-testid="aiws-app-cache-missing">
          {t('aiworkspace.shell.alert.appCacheMissing', { reason: appCache.unavailable })}
        </div>
      )}
      {sessionMode.kind === 'replica' && !sessionMode.persisted && (
        <div className="aiws-warning aiws-alert" role="status" data-testid="aiws-not-persisted">{t('aiworkspace.shell.alert.notPersisted')}</div>
      )}
      {storageProblem && (
        <div className="aiws-error aiws-alert" role="alert" data-testid="aiws-storage-problem">
          {t('aiworkspace.shell.alert.storageProblem', { problem: storageProblem })}
          <button type="button" data-testid="aiws-export-unsaved" onClick={() => { void store.exportLocal() }}>{t('aiworkspace.shell.exportUnsaved')}</button>
          <button type="button" data-testid="aiws-reopen" disabled={shell.offlineBusy !== null} onClick={() => shell.runOffline(t('aiworkspace.shell.alert.reopening'), () => shell.offline.reopen(workspaceId))}>{t('aiworkspace.shell.alert.reopen')}</button>
        </div>
      )}
      {status.kind === 'stopped' && <StoppedAlert detail={status.detail} pendingCount={pendingCount} />}
      {undo.problem && (
        <div className="aiws-warning aiws-alert" role="alert" data-testid="aiws-undo-problem">
          {undo.problem} <button type="button" className="aiws-link" onClick={() => store.undo.dismissProblem()}>{t('aiworkspace.shell.gotIt')}</button>
        </div>
      )}
      {notices.map((notice) => (
        <div key={notice.id} className={`${notice.kind === 'error' ? 'aiws-error' : 'aiws-warning'} aiws-alert`} role={notice.kind === 'error' ? 'alert' : 'status'} data-testid="aiws-notice">
          {notice.text}
          {notice.action && <button type="button" className="aiws-link" data-testid="aiws-notice-action" onClick={() => { notice.action?.run(); store.dismissNotice(notice.id) }}>{notice.action.label}</button>}
          <button type="button" className="aiws-link" onClick={() => store.dismissNotice(notice.id)}>{t('aiworkspace.common.close')}</button>
        </div>
      ))}
    </div>
  )
}

/** The workspace is being presented (第三期规划 §8.1): writes are closed until the show ends; a manager may end it. */
function ShowLockAlert() {
  const store = useStore()
  const shell = useShell()
  const { t, time } = useAiwsText()
  const lock = useShowLock()
  const [ending, setEnding] = useState(false)
  if (!lock || shell.topMode === 'show') return null
  const started = lock.started_at ? time(lock.started_at) : ''
  const manager = store.session.info().capabilities.includes('manage')
  const replica = store.session.offline !== null
  return (
    <div className="aiws-warning aiws-alert" role="status" data-testid="aiws-show-locked" data-presenter={lock.presenter}>
      {started
        ? replica ? t('aiworkspace.shell.showLock.startedReplica', { presenter: lock.presenter, time: started }) : t('aiworkspace.shell.showLock.started', { presenter: lock.presenter, time: started })
        : replica ? t('aiworkspace.shell.showLock.replica', { presenter: lock.presenter }) : t('aiworkspace.shell.showLock.direct', { presenter: lock.presenter })}
      {manager && (
        <button type="button" className="aiws-link" data-testid="aiws-show-force-end" disabled={ending} onClick={() => {
          setEnding(true)
          void shell.client.showEnd({ workspace_id: store.session.workspaceId }, lock.show_id).then((r) => unwrap(r)).then(
            () => { store.session.noteShowLock(null); store.notify('info', t('aiworkspace.shell.showLock.ended')) },
            (error: unknown) => store.notify('error', t('aiworkspace.shell.showLock.endFailed', { error: describeError(error) })),
          ).finally(() => setEnding(false))
        }}>{t('aiworkspace.shell.showLock.end')}</button>
      )}
    </div>
  )
}

function StoppedAlert({ detail, pendingCount }: { detail: string; pendingCount: number }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  const replica = store.session.offline
  const workspaceId = store.session.workspaceId
  return (
    <div className="aiws-error aiws-alert" role="alert" data-testid="aiws-stopped">
      {detail}
      {replica && <StoppedActions pendingCount={pendingCount} onDestroy={() => shell.runOffline(t('aiworkspace.shell.stopped.destroying'), async () => { await replica.destroy(); await shell.offline.reopen(workspaceId) })} />}
    </div>
  )
}

function StoppedActions({ pendingCount, onDestroy }: { pendingCount: number; onDestroy: () => void }) {
  const store = useStore()
  const shell = useShell()
  const { t } = useAiwsText()
  return (
    <>
      <button type="button" data-testid="aiws-export-pending" onClick={() => { void store.exportLocal() }}>{t('aiworkspace.shell.stopped.exportPending', { count: pendingCount })}</button>
      <ConfirmButton testId="aiws-destroy" confirmTestId="aiws-destroy-confirm" label={t('aiworkspace.shell.stopped.destroy')} confirmLabel={t('aiworkspace.shell.stopped.confirmDestroy', { count: pendingCount })} disabled={shell.offlineBusy !== null} onConfirm={onDestroy} />
    </>
  )
}


function ConfirmButton({ testId, confirmTestId, label, confirmLabel, disabled, onConfirm }: { testId: string; confirmTestId: string; label: string; confirmLabel: string; disabled?: boolean; onConfirm: () => void }) {
  const [asked, setAsked] = useState(false)
  return asked
    ? <button type="button" data-testid={confirmTestId} disabled={disabled} onClick={onConfirm}>{confirmLabel}</button>
    : <button type="button" data-testid={testId} onClick={() => setAsked(true)}>{label}</button>
}
