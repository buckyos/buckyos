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

/** Asked by the identity menu: the Desktop runs its own sign-out flow (close guards included). */
export const LOGOUT_REQUEST_EVENT = 'buckyos:request-logout'

export interface TabHost {
  workspaceId: string | null
  target: LaunchTarget | null
  navigate: (workspaceId: string | null, options?: { replace?: boolean }) => void
  consumeTarget: () => void
  onShown: (workspace: { workspaceId: string; title: string } | null) => void
  home: () => void
  signOut: () => Promise<void>
}

function isGone(failure: unknown): boolean {
  const cause = failure instanceof Error && failure.cause ? failure.cause : failure
  return [failure, cause].some((error) => error instanceof ServiceFailure && (error.code === 'NOT_FOUND' || error.code === 'PERMISSION_DENIED'))
}

export function AIWorkspaceAppPanel(props: Partial<AppContentLoaderProps> & { tab?: TabHost } = {}) {
  const tab = props.tab ?? null
  const [availability] = useState(resolveTransport)
  const [client] = useState(() => (availability.ok ? new AiwsClient(availability.transport) : null))
  const [store, setStore] = useState<WorkspaceStore | null>(null)
  const [target, setTarget] = useState<LaunchTarget | null>(null)
  const [opening, setOpening] = useState<string | null>(() => tab?.workspaceId ?? null)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<PrepareReport | null>(null)
  const [leave, setLeave] = useState<{ summary: LeaveSummary; reason: string; resolve: (go: boolean) => void } | null>(null)
  /** A normal start looks for the most recent workspace first: the list is not flashed before it reopens. */
  const [checkingRecent, setCheckingRecent] = useState(() => availability.ok && !tab)
  /** The previous session letting go of its replica (database closed, holder lock released). */
  const closing = useRef<Promise<void>>(Promise.resolve())
  /** Closing a workspace pauses the automatic restore for the rest of this app session. */
  const autoRestore = useRef(!tab)
  const storeRef = useRef<WorkspaceStore | null>(null)
  useEffect(() => { storeRef.current = store }, [store])
  const openSeq = useRef(0)
  const tabRef = useRef(tab)
  useEffect(() => { tabRef.current = tab })

  useEffect(() => {
    if (!store) return
    store.retain()
    return () => store.releaseSoon()
  }, [store])

  /** The session is the only thing the views use. A prepared replica whose holder lock this window gets is
   * opened without the network; every other case is online direct mode, which says why it is not offline. */
  const open = useCallback(async (workspaceId: string, how: { target?: LaunchTarget | null; auto?: boolean; link?: boolean } = {}) => {
    if (!client) return
    const seq = ++openSeq.current
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
      const next = new WorkspaceStore(session, core)
      if (seq !== openSeq.current) { closing.current = next.dispose().catch(() => undefined); return }
      setTarget(how.target ?? null)
      storeRef.current = next
      setStore(next)
      void rememberRecent(client.transport, workspaceId)
      if (how.target) tabRef.current?.consumeTarget()
    } catch (failure) {
      if (seq !== openSeq.current) return
      if (how.auto && isGone(failure)) void forgetRecent(client.transport, workspaceId)
      setError(how.link ? `无法打开链接中的工作区：${isGone(failure) ? '它不存在，或你没有访问权限' : describeError(failure)}`
        : how.auto ? `没有恢复上次打开的工作区：${describeError(failure)}` : `无法打开工作区：${describeError(failure)}`)
    } finally {
      if (seq === openSeq.current) setOpening(null)
    }
  }, [client])

  /** Close the current session completely (its replica lock and database included). */
  const dispose = useCallback(() => {
    const current = storeRef.current
    if (!current) return
    storeRef.current = null
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

  const routed = tab?.workspaceId ?? null
  const handledRoute = useRef<string | null | undefined>(undefined)
  useEffect(() => {
    const host = tabRef.current
    if (!client || !host || handledRoute.current === routed) return
    const first = handledRoute.current === undefined
    handledRoute.current = routed
    const current = storeRef.current?.session.workspaceId ?? null
    if (routed === current) return
    const go = () => {
      dispose()
      setReport(null)
      if (routed) void open(routed, { target: host.target, link: first || host.target !== null })
      else { openSeq.current += 1; setOpening(null) }
    }
    if (!storeRef.current) { go(); return }
    void confirmLeave(routed ? '切换工作区' : '关闭工作区').then((ok) => {
      if (ok) go()
      else host.navigate(current, { replace: true })
    })
  }, [client, routed, open, dispose, confirmLeave])

  const shownId = store?.session.workspaceId ?? null
  const shownTitle = store?.session.info().title ?? null
  useEffect(() => {
    tabRef.current?.onShown(shownId ? { workspaceId: shownId, title: shownTitle ?? shownId } : null)
  }, [shownId, shownTitle])

  const isTab = tab !== null
  useEffect(() => {
    if (!isTab) return
    const onBeforeUnload = (event: BeforeUnloadEvent) => {
      const summary = storeRef.current?.leaveSummary()
      if (summary && (summary.memoryOnly > 0 || summary.unsaved > 0 || summary.attention > 0)) event.preventDefault()
    }
    window.addEventListener('beforeunload', onBeforeUnload)
    return () => window.removeEventListener('beforeunload', onBeforeUnload)
  }, [isTab])

  // a normal start restores this identity's most recent workspace
  useEffect(() => {
    if (!client || isTab) return
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

  const closeWorkspace = () => leaveThen('关闭工作区', () => {
    autoRestore.current = false
    dispose()
    setReport(null)
    tab?.navigate(null)
  })
  const show = (workspaceId: string) => {
    if (tab && workspaceId !== tab.workspaceId) tab.navigate(workspaceId)
    else void open(workspaceId)
  }
  const switchTo = (workspaceId: string) => leaveThen('切换工作区', () => {
    dispose()
    setReport(null)
    show(workspaceId)
  })
  const pick = (workspaceId: string) => show(workspaceId)
  const logout = tab
    ? () => leaveThen('退出登录', () => { tab.signOut().catch((failure: unknown) => setError(`退出登录失败：${describeError(failure)}`)) })
    : () => window.dispatchEvent(new CustomEvent(LOGOUT_REQUEST_EVENT))
  const goHome = tab ? () => leaveThen('返回桌面', () => { dispose(); tab.home() }) : null

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
            onClose={closeWorkspace} onOpenWorkspace={switchTo} onLogout={logout} onHome={goHome}
            identity={{ principal: store.session.principal, dev: client.transport.mode === 'dev-override' }}
            devTools={import.meta.env.DEV || client.transport.mode === 'dev-override'} />
        : (opening || checkingRecent) && !error
          ? <div className="aiws-opening" role="status" data-testid="aiws-opening">正在打开工作区…</div>
          : <WorkspaceList client={client} onOpen={pick} opening={opening} />}
      {leave && (
        <LeaveDialog summary={leave.summary} reason={leave.reason}
          onStay={() => { leave.resolve(false); setLeave(null) }}
          onExportAndLeave={() => { void store?.exportLocal().finally(() => { leave.resolve(true); setLeave(null) }) }}
          onLeave={() => { leave.resolve(true); setLeave(null) }} />
      )}
    </div>
  )
}
