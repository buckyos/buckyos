/* Who may open a Workspace's replica, and how one is prepared (design §6.1, §6.2).
 *
 *   - single holder: the Web Lock `aiworkspace-replica:<workspace_id>` (exclusive, `ifAvailable`).
 *     Only the tab that holds it starts a Replica Worker; `opfs-sahpool` could not be opened twice
 *     anyway. Everyone else stays in online direct mode;
 *   - explicit preparation: `replica.bootstrap` → download → import into OPFS → optionally the
 *     referenced asset bytes;
 *   - a small index in localStorage lists which Workspaces were prepared here, so the list can be
 *     shown with the backend unreachable. It is only a hint: the replica database is the truth. */

import { unwrap, type AiwsClient } from '../api/client'
import type { WorkspaceInfo } from '../api/types'
import { DEV_OVERRIDE_KEY } from '../api/transport'
import { ReplicaClient } from './client'
import { ASSET_SIZE_CAP, REPLICA_LOCK_PREFIX, type AssetRow, type PendingRow, type ReplicaMeta } from './protocol'

const INDEX_KEY = 'aiworkspace.offline.index'

export interface PreparedHint { workspace_id: string; title: string; principal: string; prepared_at: string }

export function preparedIndex(): Record<string, PreparedHint> {
  try { return JSON.parse(window.localStorage.getItem(INDEX_KEY) ?? '{}') as Record<string, PreparedHint> } catch { return {} }
}

function writeIndex(update: (index: Record<string, PreparedHint>) => void) {
  try {
    const index = preparedIndex()
    update(index)
    window.localStorage.setItem(INDEX_KEY, JSON.stringify(index))
  } catch { /* the index is a convenience */ }
}

export function forgetPrepared(workspaceId: string) {
  writeIndex((index) => { delete index[workspaceId] })
}

function testHooksEnabled(): boolean {
  try { return window.localStorage.getItem(DEV_OVERRIDE_KEY) !== null } catch { return false }
}

/** Why this browser context cannot hold an offline replica at all, or null. */
export function offlineUnavailableReason(): string | null {
  if (!window.isSecureContext) return '当前页面不是安全上下文（需要 HTTPS 或 localhost），浏览器不提供 OPFS 与 Service Worker'
  if (typeof Worker === 'undefined') return '此浏览器不支持 Web Worker'
  if (typeof navigator.storage?.getDirectory !== 'function') return '此浏览器没有 OPFS（Origin Private File System）'
  if (typeof navigator.locks?.request !== 'function') return '此浏览器没有 Web Locks，无法保证只有一个窗口写入离线副本'
  return null
}

export interface ReplicaLock { release(): void }

/** The exclusive holder lock; null when another tab (or Worker) of this origin holds it. */
export function acquireReplicaLock(workspaceId: string): Promise<ReplicaLock | null> {
  return new Promise((resolve, reject) => {
    navigator.locks.request(`${REPLICA_LOCK_PREFIX}${workspaceId}`, { mode: 'exclusive', ifAvailable: true }, (lock) => {
      if (!lock) { resolve(null); return undefined }
      return new Promise<void>((release) => {
        let released = false
        resolve({ release: () => { if (!released) { released = true; release() } } })
      })
    }).catch(reject)
  })
}

export async function replicaLockHeld(workspaceId: string): Promise<boolean> {
  try {
    const state = await navigator.locks.query()
    return (state.held ?? []).some((lock) => lock.name === `${REPLICA_LOCK_PREFIX}${workspaceId}`)
  } catch {
    return false
  }
}

export type DirectReason = 'not_prepared' | 'not_holder' | 'unavailable'

export interface OpenedReplica { replica: ReplicaClient; lock: ReplicaLock; meta: ReplicaMeta; pending: PendingRow[]; persisted: boolean }

export type ReplicaOpenResult = { kind: 'replica'; opened: OpenedReplica } | { kind: 'direct'; reason: DirectReason; detail: string }

/** Become the holder of an already prepared replica. Never touches the network. */
export async function openPreparedReplica(workspaceId: string, principal: string | null): Promise<ReplicaOpenResult> {
  if (!preparedIndex()[workspaceId]) return { kind: 'direct', reason: 'not_prepared', detail: '尚未为此工作区准备离线' }
  const unavailable = offlineUnavailableReason()
  if (unavailable) return { kind: 'direct', reason: 'unavailable', detail: unavailable }
  const lock = await acquireReplicaLock(workspaceId)
  if (!lock) return { kind: 'direct', reason: 'not_holder', detail: '另一个窗口正持有此工作区的离线副本' }
  let replica: ReplicaClient | null = null
  try {
    const started = await ReplicaClient.start(workspaceId, testHooksEnabled())
    replica = started.client
    if (!started.init.ok) {
      lock.release()
      return { kind: 'direct', reason: 'unavailable', detail: started.init.reason }
    }
    const prepared = started.init.prepared
    if (!prepared) {
      // the index was stale (site data cleared, or preparation never finished)
      forgetPrepared(workspaceId)
      await replica.close()
      lock.release()
      return { kind: 'direct', reason: 'not_prepared', detail: '本机没有此工作区的离线副本（浏览器存储可能已被清理）' }
    }
    if (principal && prepared.principal !== principal) {
      await replica.close()
      lock.release()
      return { kind: 'direct', reason: 'not_prepared', detail: `本机的离线副本属于另一个身份（${prepared.principal}）` }
    }
    const opened = await replica.call('open')
    let persisted = false
    try { persisted = await navigator.storage.persisted() } catch { /* reported as not persisted */ }
    return { kind: 'replica', opened: { replica, lock, meta: opened.meta, pending: opened.pending, persisted } }
  } catch (error) {
    replica?.terminate()
    lock.release()
    return { kind: 'direct', reason: 'unavailable', detail: `本机的离线副本无法打开：${error instanceof Error ? error.message : String(error)}` }
  }
}

/** Offline cannot be offered here; the message names the reason (design §6.1). */
export class OfflineUnavailable extends Error {
  constructor(reason: string) {
    super(reason)
    this.name = 'OfflineUnavailable'
  }
}

export interface PrepareReport {
  meta: ReplicaMeta
  assets: { stored: AssetRow[]; skipped: (AssetRow & { reason: string })[] }
}

/** The explicit user action "准备离线". Needs workspace-level `read`; a backend refusal is thrown as is. */
export async function prepareOffline(client: AiwsClient, workspaceId: string, options: { discardLocal?: boolean; onProgress?: (text: string) => void } = {}): Promise<PrepareReport> {
  const progress = options.onProgress ?? (() => undefined)
  const unavailable = offlineUnavailableReason()
  if (unavailable) throw new OfflineUnavailable(unavailable)
  let persisted = false
  try { persisted = await navigator.storage.persist() } catch { /* treated as refused */ }
  if (!persisted) throw new OfflineUnavailable('浏览器拒绝了持久化存储申请：本地数据可能在空间紧张时被浏览器清理，因此不启用离线')
  const lock = await acquireReplicaLock(workspaceId)
  if (!lock) throw new OfflineUnavailable('另一个窗口正持有此工作区的离线副本')
  let replica: ReplicaClient | null = null
  try {
    const started = await ReplicaClient.start(workspaceId, testHooksEnabled())
    replica = started.client
    if (!started.init.ok) throw new OfflineUnavailable(started.init.reason)
    if (started.init.prepared && started.init.pendingCount > 0 && !options.discardLocal) {
      throw new OfflineUnavailable(`本机副本里还有 ${started.init.pendingCount} 条未提交的修改；请先打开工作区导出或处理它们`)
    }
    progress('向后台申请副本…')
    const info: WorkspaceInfo = unwrap(await client.wsGetInfo({ workspace_id: workspaceId }))
    const boot = unwrap(await client.replicaBootstrap({ workspace_id: workspaceId }))
    progress('下载副本库…')
    const file = await client.transport.download(`replica/${workspaceId}/${boot.replica_id}`)
    progress('写入本机存储…')
    const meta = await replica.call('importReplica', {
      bytes: await file.arrayBuffer(), principal: boot.principal, info: { ...info, epoch: boot.epoch }, epoch: boot.epoch,
      headSeq: boot.head_seq, preparedAt: boot.prepared_at, discardLocal: options.discardLocal === true,
    })
    const report: PrepareReport = { meta, assets: { stored: [], skipped: [] } }
    for (const asset of await replica.call('listAssets')) {
      if (asset.size > ASSET_SIZE_CAP) { report.assets.skipped.push({ ...asset, reason: `超过 ${ASSET_SIZE_CAP / 1024 / 1024} MiB 的上限` }); continue }
      progress(`下载资产 ${report.assets.stored.length + report.assets.skipped.length + 1}…`)
      try {
        const bytes = await (await client.transport.download(`asset/${workspaceId}/${asset.object_id}`)).arrayBuffer()
        await replica.call('putAsset', asset.object_id, bytes)
        report.assets.stored.push(asset)
      } catch (error) {
        report.assets.skipped.push({ ...asset, reason: error instanceof Error ? error.message : String(error) })
      }
    }
    writeIndex((index) => { index[workspaceId] = { workspace_id: workspaceId, title: info.title, principal: boot.principal, prepared_at: boot.prepared_at } })
    await replica.close()
    return report
  } catch (error) {
    replica?.terminate()
    throw error
  } finally {
    lock.release()
  }
}

export function noteTitle(workspaceId: string, title: string) {
  writeIndex((index) => { if (index[workspaceId]) index[workspaceId].title = title })
}
