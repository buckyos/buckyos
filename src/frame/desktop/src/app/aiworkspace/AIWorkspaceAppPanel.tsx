/* BuckyOS AI Workspace – Desktop app entry. See README.md in this directory.
 *
 * What opens (UI improvement §4): an explicit target (a link, or a workspace picked in the list) wins;
 * otherwise a normal start restores the workspace this identity last opened successfully; closing a
 * workspace pauses that for the rest of this app session. Leaving (close, switch, log out, closing the
 * window) first ends the current edit and waits for the saves that can complete; what would still be
 * lost is asked about, never dropped silently. */

import { useCallback, useEffect, useRef, useState } from 'react'
import './aiworkspace.css'
import { desktopUIStore } from '../../models/DesktopUIDataModel'
import type { AppContentLoaderProps } from '../types'
import { AiwsClient, ServiceFailure } from './api/client'
import { describeError, OnlineWorkspaceSession, type WorkspaceSession } from './api/session'
import { resolveTransport, TransportError } from './api/transport'
import { loadCore } from './api/wasm'
import { OfflineUnavailable, openPreparedReplica, prepareOffline, type PrepareReport } from './offline/holder'
import { ReplicaWorkspaceSession } from './offline/replicaSession'
import { forgetRecent, readRecent, rememberRecent } from './state/recent'
import { WorkspaceStore } from './state/store'
import { LeaveDialog, type LeaveSummary } from './ui/shell/dialogs'
import type { LaunchTarget } from './ui/shell/WorkspaceShell'
import { WorkspaceList } from './ui/WorkspaceList'
import { WorkspaceView, type OfflineActions } from './ui/WorkspaceView'

/** The Desktop's launch payload for a link `/?aiws=<workspace>&aiwsSurface=<surface>&aiwsBlock=<block>`. */
export interface AiwsLaunch { kind: 'aiworkspace-target'; workspaceId: string; surfaceId?: string | null; blockId?: string | null }
/** Asked by the identity menu: the Desktop runs its own sign-out flow (close guards included). */
export const LOGOUT_REQUEST_EVENT = 'buckyos:request-logout'

function launchTarget(launch: AppContentLoaderProps['launch']): (AiwsLaunch & { requestId: string }) | null {
  const payload = launch?.payload as Partial<AiwsLaunch> | undefined
  if (!launch || payload?.kind !== 'aiworkspace-target' || typeof payload.workspaceId !== 'string') return null
  return { kind: 'aiworkspace-target', workspaceId: payload.workspaceId, surfaceId: payload.surfaceId ?? null, blockId: payload.blockId ?? null, requestId: launch.requestId }
}

function isGone(failure: unknown): boolean {
  const cause = failure instanceof Error && failure.cause ? failure.cause : failure
  return [failure, cause].some((error) => error instanceof ServiceFailure && (error.code === 'NOT_FOUND' || error.code === 'PERMISSION_DENIED'))
}

export function AIWorkspaceAppPanel(props: Partial<AppContentLoaderProps> = {}) {
  const [availability] = useState(resolveTransport)
  const [client] = useState(() => (availability.ok ? new AiwsClient(availability.transport) : null))
  const [store, setStore] = useState<WorkspaceStore | null>(null)
  const [target, setTarget] = useState<LaunchTarget | null>(null)
  const [opening, setOpening] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<PrepareReport | null>(null)
  const [leave, setLeave] = useState<{ summary: LeaveSummary; reason: string; resolve: (go: boolean) => void } | null>(null)
  /** A normal start looks for the most recent workspace first: the list is not flashed before it reopens. */
  const [checkingRecent, setCheckingRecent] = useState(() => availability.ok && !launchTarget(props.launch))
  /** The previous session letting go of its replica (database closed, holder lock released). */
  const closing = useRef<Promise<void>>(Promise.resolve())
  /** Closing a workspace pauses the automatic restore for the rest of this app session. */
  const autoRestore = useRef(true)
  const handledLaunch = useRef<string | null>(null)
  const storeRef = useRef<WorkspaceStore | null>(null)
  useEffect(() => { storeRef.current = store }, [store])

  useEffect(() => {
    if (!store) return
    store.retain()
    return () => store.releaseSoon()
  }, [store])

  /** The session is the only thing the views use. A prepared replica whose holder lock this window gets is
   * opened without the network; every other case is online direct mode, which says why it is not offline. */
  const open = useCallback(async (workspaceId: string, how: { target?: LaunchTarget | null; auto?: boolean } = {}) => {
    if (!client) return
    setOpening(workspaceId)
    setError(null)
    try {
      // this window's previous session must have released the replica before the same workspace is opened again
      await closing.current
      const core = await loadCore()
      const attempt = await openPreparedReplica(workspaceId, client.transport.principalHint)
      let session: WorkspaceSession
      if (attempt.kind === 'replica') {
        session = new ReplicaWorkspaceSession(client, attempt.opened)
      } else {
        try {
          session = await OnlineWorkspaceSession.open(client, workspaceId, attempt)
        } catch (failure) {
          if (failure instanceof TransportError && attempt.reason !== 'not_prepared') {
            throw new Error(`后台不可达，且此窗口不能使用离线副本（${attempt.detail}）`, { cause: failure })
          }
          throw failure
        }
      }
      setTarget(how.target ?? null)
      setStore(new WorkspaceStore(session, core))
      void rememberRecent(client.transport, workspaceId)
    } catch (failure) {
      if (how.auto && isGone(failure)) void forgetRecent(client.transport, workspaceId)
      setError(how.target ? `无法打开链接中的工作区：${isGone(failure) ? '它不存在，或你没有访问权限' : describeError(failure)}`
        : how.auto ? `没有恢复上次打开的工作区：${describeError(failure)}` : `无法打开工作区：${describeError(failure)}`)
    } finally {
      setOpening(null)
    }
  }, [client])

  /** Close the current session completely (its replica lock and database included). */
  const dispose = useCallback(() => {
    const current = storeRef.current
    if (!current) return
    setStore(null)
    setTarget(null)
    closing.current = current.dispose().catch(() => undefined)
  }, [])

  /** The leave check: true when leaving may go ahead. */
  const confirmLeave = useCallback(async (reason: string): Promise<boolean> => {
    const current = storeRef.current
    if (!current) return true
    const summary = await current.prepareLeave()
    if (summary.memoryOnly === 0 && summary.unsaved === 0 && summary.attention === 0) return true
    return new Promise<boolean>((resolve) => setLeave({ summary, reason, resolve }))
  }, [])

  const leaveThen = useCallback((reason: string, then: () => void) => {
    void confirmLeave(reason).then((go) => { if (go) then() })
  }, [confirmLeave])

  // an explicit target from the Desktop (a link) — every new launch request is handled once
  const launch = launchTarget(props.launch)
  useEffect(() => {
    if (!client || !launch || handledLaunch.current === launch.requestId) return
    handledLaunch.current = launch.requestId
    autoRestore.current = false
    const go = () => { dispose(); void open(launch.workspaceId, { target: { surfaceId: launch.surfaceId, blockId: launch.blockId } }) }
    if (storeRef.current) leaveThen('打开链接', go)
    else go()
    // eslint-disable-next-line react-hooks/exhaustive-deps -- keyed by the launch request id
  }, [client, launch?.requestId, open, dispose, leaveThen])

  // a normal start restores this identity's most recent workspace
  useEffect(() => {
    if (!client || launch) return
    let live = true
    void readRecent(client.transport).then((recent) => {
      if (!live) return
      if (recent && autoRestore.current && !storeRef.current) {
        autoRestore.current = false
        void open(recent.workspace_id, { auto: true })
      }
      setCheckingRecent(false)
    })
    return () => { live = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- once per mount; a later launch request is handled above
  }, [client])

  // the Desktop asks before closing this window or logging out
  const windowId = props.windowId
  useEffect(() => {
    if (!windowId) return
    return desktopUIStore.registerCloseGuard(windowId, async (reason = 'user') => confirmLeave(reason === 'logout' ? '退出登录' : '关闭窗口'))
  }, [windowId, confirmLeave])

  if (!client || !availability.ok) {
    return (
      <div className="aiws-root" data-testid="aiws-root">
        <div className="aiws-error" role="alert" data-testid="aiws-unavailable">{availability.ok ? '' : availability.reason}</div>
      </div>
    )
  }

  const reopen = async (workspaceId: string) => {
    dispose()
    await open(workspaceId)
  }

  const actions: OfflineActions = {
    prepare: async (workspaceId, discardLocal) => {
      const result = await prepareOffline(client, workspaceId, { discardLocal })
      setReport(result)
      await reopen(workspaceId)
    },
    reopen,
    describe: (failure) => (failure instanceof OfflineUnavailable ? `离线不可用：${failure.message}` : `准备离线失败：${describeError(failure)}`),
  }

  const closeWorkspace = () => leaveThen('关闭工作区', () => { autoRestore.current = false; dispose(); setReport(null) })
  const switchTo = (workspaceId: string) => leaveThen('切换工作区', () => { dispose(); setReport(null); void open(workspaceId) })
  const logout = () => window.dispatchEvent(new CustomEvent(LOGOUT_REQUEST_EVENT))

  return (
    <div className="aiws-root" data-testid="aiws-root">
      {error && <div className="aiws-error aiws-root-error" role="alert" data-testid="aiws-open-error">{error} <button type="button" className="aiws-link" onClick={() => setError(null)}>知道了</button></div>}
      {report && store && (
        <div className="aiws-warning aiws-root-error" role="status" data-testid="aiws-prepare-report">
          已为此工作区准备离线：副本基于第 {report.meta.confirmed_seq} 次提交；资产已缓存 {report.assets.stored.length} 个
          {report.assets.skipped.length > 0 && (
            <span data-testid="aiws-prepare-skipped">，跳过 {report.assets.skipped.length} 个（{report.assets.skipped.map((asset) => `${asset.object_id.slice(0, 18)}… ${asset.size} 字节：${asset.reason}`).join('；')}）</span>
          )}
          。 <button type="button" className="aiws-link" onClick={() => setReport(null)}>知道了</button>
        </div>
      )}
      {store
        ? <WorkspaceView key={store.session.workspaceId + store.session.sessionId} store={store} client={client} offline={actions} target={target}
            onClose={closeWorkspace} onOpenWorkspace={switchTo} onLogout={logout}
            identity={{ principal: store.session.principal, dev: client.transport.mode === 'dev-override' }}
            devTools={import.meta.env.DEV || client.transport.mode === 'dev-override'} />
        : (opening || checkingRecent) && !error
          ? <div className="aiws-opening" role="status" data-testid="aiws-opening">正在打开工作区…</div>
          : <WorkspaceList client={client} onOpen={(id) => { void open(id) }} opening={opening} />}
      {leave && (
        <LeaveDialog summary={leave.summary} reason={leave.reason}
          onStay={() => { leave.resolve(false); setLeave(null) }}
          onExportAndLeave={() => { void store?.exportLocal().finally(() => { leave.resolve(true); setLeave(null) }) }}
          onLeave={() => { leave.resolve(true); setLeave(null) }} />
      )}
    </div>
  )
}
