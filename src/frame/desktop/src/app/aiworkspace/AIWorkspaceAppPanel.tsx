/* BuckyOS AI Workspace (phase one) – Desktop app entry. See README.md in this directory. */

import { useEffect, useRef, useState } from 'react'
import './aiworkspace.css'
import { AiwsClient } from './api/client'
import { describeError, OnlineWorkspaceSession, type WorkspaceSession } from './api/session'
import { resolveTransport, TransportError } from './api/transport'
import { loadCore } from './api/wasm'
import { OfflineUnavailable, openPreparedReplica, prepareOffline, type PrepareReport } from './offline/holder'
import { ReplicaWorkspaceSession } from './offline/replicaSession'
import { WorkspaceStore } from './state/store'
import { WorkspaceList } from './ui/WorkspaceList'
import { WorkspaceView, type OfflineActions } from './ui/WorkspaceView'

export function AIWorkspaceAppPanel() {
  const [availability] = useState(resolveTransport)
  const [client] = useState(() => (availability.ok ? new AiwsClient(availability.transport) : null))
  const [store, setStore] = useState<WorkspaceStore | null>(null)
  const [opening, setOpening] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [report, setReport] = useState<PrepareReport | null>(null)
  /** The previous session letting go of its replica (database closed, holder lock released). */
  const closing = useRef<Promise<void>>(Promise.resolve())

  useEffect(() => {
    if (!store) return
    store.retain()
    return () => store.releaseSoon()
  }, [store])

  if (!client || !availability.ok) {
    return (
      <div className="aiws-root" data-testid="aiws-root">
        <div className="aiws-error" role="alert" data-testid="aiws-unavailable">{availability.ok ? '' : availability.reason}</div>
      </div>
    )
  }

  /** The session is the only thing the views use. A prepared replica whose holder lock this window gets is
   * opened without the network; every other case is online direct mode, which says why it is not offline. */
  const open = async (workspaceId: string) => {
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
      setStore(new WorkspaceStore(session, core))
    } catch (failure) {
      setError(`无法打开工作区：${describeError(failure)}`)
    } finally {
      setOpening(null)
    }
  }

  /** Close the current session completely (its replica lock and database included), then open again. */
  const close = () => {
    if (!store) return
    setStore(null)
    closing.current = store.dispose().catch(() => undefined)
  }

  const reopen = async (workspaceId: string) => {
    close()
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

  return (
    <div className="aiws-root" data-testid="aiws-root">
      {error && <div className="aiws-error" role="alert" data-testid="aiws-open-error">{error}</div>}
      {report && store && (
        <div className="aiws-warning" role="status" data-testid="aiws-prepare-report">
          已为此工作区准备离线：副本基于第 {report.meta.confirmed_seq} 次提交；资产已缓存 {report.assets.stored.length} 个
          {report.assets.skipped.length > 0 && (
            <span data-testid="aiws-prepare-skipped">，跳过 {report.assets.skipped.length} 个（{report.assets.skipped.map((asset) => `${asset.object_id.slice(0, 18)}… ${asset.size} 字节：${asset.reason}`).join('；')}）</span>
          )}
          。 <button type="button" className="aiws-link" onClick={() => setReport(null)}>知道了</button>
        </div>
      )}
      {store
        ? <WorkspaceView key={store.session.workspaceId + store.session.sessionId} store={store} offline={actions} onClose={() => { close(); setReport(null) }} />
        : <WorkspaceList client={client} onOpen={(id) => { void open(id) }} opening={opening} />}
    </div>
  )
}
