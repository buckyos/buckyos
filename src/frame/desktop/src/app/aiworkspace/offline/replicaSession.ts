/* ReplicaWorkspaceSession: the WorkspaceSession of the window that holds the offline replica (design §6).
 *
 *   reads    working view of the Replica Worker (confirmed layer + pending submissions replayed)
 *   writes   `submit_local` → pending row committed in the replica database → "已保存到本设备" → send loop
 *
 * The network stays in the page (the zone transport and its session token live here); the Worker
 * owns the engine and the database. This differs from the drawing in design §6.1, which puts the
 * transport into the Worker; the atomicity rules are unaffected because every state change still
 * goes through one Worker transaction.
 *
 * Reconnect order (design §6.4), every time the backend becomes reachable:
 *   1. re-authenticate (the transport does it per request)
 *   2. `ws.get_info`: epoch unchanged, access still there
 *   3. `doc.get_changes` catch-up → `apply_remote` (rebase; own submissions are recognised by key)
 *   4. submissions left `sending` / `unknown` → `doc.get_submission`: accepted → catch up again,
 *      not_found → back to `queued`, to be resent unchanged
 *   5. send loop: one `queued` submission at a time; an accepted one is removed only when the change
 *      stream brings it back; `conflict` / `rejected` stay as "需要处理" and the loop continues.
 * `EPOCH_MISMATCH` / `BASE_TOO_OLD` / lost access stop everything; nothing is discarded. */

import { ServiceFailure, type AiwsClient } from '../api/client'
import { OnlineWorkspaceSession, SESSION_STOP_CODES, SESSION_STOP_TEXT, browserOffline } from '../api/session'
import type { CommitOptions, OfflineControls, ReadOk, SessionMode, SessionStatus, SubmissionEvent, WorkspaceSession } from '../api/session'
import { TransportError } from '../api/transport'
import {
  PROTOCOL_VERSION,
  type CollabState, type CommitEvent, type CommitOutcome, type CommitRequest, type CommitResult, type EntityEnvelope, type Json, type LockInfo,
  type Operation, type PrepareResult, type QueryPage, type QueryParams, type RunView, type Selector, type Touched, type WorkspaceInfo,
  type AnnotationContent, type ListAnnotationsParams,
} from '../api/types'
import { StorageFailure, describeStorageFailure, type ReplicaClient } from './client'
import { forgetPrepared, noteTitle, type OpenedReplica, type ReplicaLock } from './holder'
import { ASSET_SIZE_CAP, type LocalExport, type LocalRefused, type LocalSaved, type PendingRow, type PendingState } from './protocol'

const STOP_TEXT: Record<string, string> = {
  EPOCH_MISMATCH: '此工作区的历史已被恢复操作替换：本机副本与其中的待提交修改是针对旧历史的，已停止发送，什么都没有丢弃。请导出待提交内容，然后删除本机副本并重新准备离线。',
  BASE_TOO_OLD: '后台的变化流已不覆盖本机副本的基准：已停止发送，什么都没有丢弃。请导出待提交内容，然后删除本机副本并重新准备离线。',
  PERMISSION_DENIED: '你对此工作区的访问权限已被撤回：已停止发送，本机的待提交修改全部保留，可以查看和导出。',
  NOT_FOUND: '此工作区在后台已不存在，或你已无权访问：已停止发送，本机的待提交修改全部保留，可以查看和导出。',
  REPLICA_BROKEN: '本机副本无法应用后台已接受的提交：已停止同步，什么都没有丢弃。请导出待提交内容，然后删除本机副本并重新准备离线。',
}

function wait(ms: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    const timer = window.setTimeout(done, ms)
    function done() {
      window.clearTimeout(timer)
      signal.removeEventListener('abort', done)
      resolve()
    }
    signal.addEventListener('abort', done)
  })
}

/** What a local submission changes in the working view, coarse but never too little: views re-read what is named here. */
function localTouched(request: CommitRequest): Touched[] {
  const out: Touched[] = []
  for (const op of request.operations) {
    const name = op.op
    const entityId = typeof op.entity_id === 'string' ? op.entity_id : ''
    if (name.startsWith('table.')) {
      const source = typeof op.source_id === 'string' ? op.source_id : ''
      const values = Array.isArray(op.values) ? op.values as { record_id?: unknown; field_id?: unknown }[] : null
      if ((name === 'table.set_values' || name === 'table.unset_values') && values && values.every((item) => typeof item.record_id === 'string' && typeof item.field_id === 'string')) {
        for (const item of values) out.push({ entity_id: source, selector: { kind: 'table_cell', record_id: item.record_id as string, field_id: item.field_id as string }, change: 'value', rev: 0 })
      } else {
        out.push({ entity_id: source, change: 'schema', rev: 0 })
      }
    } else if (name.startsWith('richtext.')) {
      out.push({ entity_id: entityId, change: 'text', rev: 0 })
    } else {
      const change = name === 'entity.create' ? 'created' : name === 'entity.delete' ? 'deleted' : name === 'entity.restore' ? 'restored'
        : name === 'tree.move' ? 'moved' : name === 'tree.place' ? 'placed' : name === 'entity.rename' || name === 'entity.set_write_policy' ? 'renamed' : 'view'
      out.push({ entity_id: entityId, change, rev: 0 })
      const subtree = (op.subtree as { delete?: unknown } | undefined)?.delete
      if (Array.isArray(subtree)) for (const child of subtree) if (typeof child === 'string') out.push({ entity_id: child, change, rev: 0 })
    }
  }
  return out
}

export class ReplicaWorkspaceSession implements WorkspaceSession {
  readonly workspaceId: string
  readonly sessionId: string
  readonly offline: OfflineControls
  private readonly client: AiwsClient
  private readonly replica: ReplicaClient
  private readonly lock: ReplicaLock
  private readonly direct: OnlineWorkspaceSession
  private readonly sessionMode: SessionMode
  private readonly epoch: string
  private readonly replicaPrincipal: string
  private wsInfo: WorkspaceInfo
  private confirmedSeq: number
  private rows: PendingRow[]
  private currentStatus: SessionStatus = { kind: 'connecting' }
  private storageIssue: string | null = null
  private keyCounter = 0
  private closed = false
  private disposed = false
  private released: Promise<void> = Promise.resolve()
  private lockHolders = new Map<string, LockInfo>()
  private wakeAbort = new AbortController()
  private waitAbort: AbortController | null = null
  private serial: Promise<unknown> = Promise.resolve()
  private readonly statusListeners = new Set<() => void>()
  private readonly changeListeners = new Set<(event: CommitEvent) => void>()
  private readonly submissionListeners = new Set<(event: SubmissionEvent) => void>()
  private readonly seqWaiters: { seq: number; resolve: () => void }[] = []
  private readonly onOnline = () => this.wake()
  private readonly offDeath: () => void

  constructor(client: AiwsClient, opened: OpenedReplica) {
    this.client = client
    this.replica = opened.replica
    this.lock = opened.lock
    this.workspaceId = opened.meta.workspace_id
    this.epoch = opened.meta.epoch
    this.replicaPrincipal = opened.meta.principal
    this.wsInfo = { ...opened.meta.info, epoch: opened.meta.epoch, head_seq: opened.meta.confirmed_seq }
    this.confirmedSeq = opened.meta.confirmed_seq
    this.rows = opened.pending
    this.sessionMode = { kind: 'replica', preparedAt: opened.meta.prepared_at, persisted: opened.persisted }
    this.direct = OnlineWorkspaceSession.hosted(client, this.wsInfo, {
      poke: () => this.wake(),
      whenApplied: (seq) => this.whenApplied(seq),
      onStop: (code) => this.stop(code),
    })
    this.sessionId = this.direct.sessionId
    this.offDeath = this.replica.onDeath(() => this.noteStorage(new StorageFailure('worker', this.replica.dead ?? 'Worker 已退出')))
    this.offline = {
      pending: () => this.rows,
      subscribeSubmissions: (listener) => {
        this.submissionListeners.add(listener)
        return () => { this.submissionListeners.delete(listener) }
      },
      storageProblem: () => this.storageIssue,
      discardPending: (key, options) => this.discardPending(key, options?.force === true),
      requeuePending: (key) => this.requeuePending(key),
      exportLocal: () => this.exportLocal(),
      saveDraft: async (draft) => { await this.replica.call('saveDraft', draft) },
      destroy: () => this.destroy(),
      test: client.transport.mode === 'dev-override' ? {
        failTransactions: async (kind, count) => { await this.replica.call('testFailTransactions', kind, count) },
        killWorker: () => this.replica.testKillWorker(),
      } : null,
    }
    window.addEventListener('online', this.onOnline)
    void this.syncLoop()
  }

  get principal(): string | null { return this.client.transport.principalHint ?? this.replicaPrincipal }
  private get ws() { return { workspace_id: this.workspaceId } }

  mode() { return this.sessionMode }
  info() { return this.wsInfo }
  status() { return this.currentStatus }
  appliedSeq() { return this.confirmedSeq }

  subscribeStatus(listener: () => void) {
    this.statusListeners.add(listener)
    return () => { this.statusListeners.delete(listener) }
  }

  subscribeChanges(listener: (event: CommitEvent) => void) {
    this.changeListeners.add(listener)
    return () => { this.changeListeners.delete(listener) }
  }

  whenApplied(seq: number): Promise<void> {
    if (this.confirmedSeq >= seq || this.currentStatus.kind === 'stopped' || this.closed) return Promise.resolve()
    this.wake()
    return new Promise((resolve) => { this.seqWaiters.push({ seq, resolve }) })
  }

  private isStopped() { return this.currentStatus.kind === 'stopped' }
  private isLive() { return this.currentStatus.kind === 'live' }

  private setStatus(status: SessionStatus) {
    const previous = this.currentStatus
    if (previous.kind === 'stopped') return
    const same = previous.kind === status.kind && (status.kind !== 'offline' || (previous.kind === 'offline' && previous.detail === status.detail && previous.browserOffline === status.browserOffline))
    this.currentStatus = status
    if (!same) this.statusListeners.forEach((listener) => listener())
  }

  private stop(code: string) {
    if (this.isStopped()) return
    this.setStatus({ kind: 'stopped', code, detail: STOP_TEXT[code] ?? SESSION_STOP_TEXT[code] ?? code })
    this.waitAbort?.abort()
    this.wakeAbort.abort()
    this.seqWaiters.splice(0).forEach((waiter) => waiter.resolve())
  }

  private noteStorage(failure: StorageFailure) {
    const text = describeStorageFailure(failure)
    if (this.storageIssue === text) return
    this.storageIssue = text
    this.statusListeners.forEach((listener) => listener())
  }

  private clearStorageIssue() {
    if (this.storageIssue === null || this.replica.dead !== null) return
    this.storageIssue = null
    this.statusListeners.forEach((listener) => listener())
  }

  /** Worker calls that change state run one after another, each seeing the pending list the previous one left. */
  private exclusive<T>(work: () => Promise<T>): Promise<T> {
    const run = this.serial.then(work, work)
    this.serial = run.catch(() => undefined)
    return run
  }

  private emitChange(event: CommitEvent) {
    for (const listener of [...this.changeListeners]) {
      try { listener(event) } catch (error) { console.error('[aiworkspace] change listener failed', error) }
    }
  }

  private emitSubmission(event: SubmissionEvent) {
    for (const listener of [...this.submissionListeners]) {
      try { listener(event) } catch (error) { console.error('[aiworkspace] submission listener failed', error) }
    }
  }

  /** Take over the Worker's pending list; tell the UI what changed and which parts of the working view moved. */
  private adopt(next: PendingRow[], settled: readonly string[] = [], events: readonly CommitEvent[] = []) {
    const before = new Map(this.rows.map((row) => [row.idempotency_key, row]))
    const after = new Map(next.map((row) => [row.idempotency_key, row]))
    this.rows = next
    const moved: Touched[] = []
    for (const row of next) {
      const old = before.get(row.idempotency_key)
      if (!old || old.applied !== row.applied) moved.push(...localTouched(row.request))
      if (!old || old.state !== row.state) this.emitSubmission({ key: row.idempotency_key, state: row.state, result: row.result, meta: row.meta, request: row.request })
    }
    for (const old of before.values()) {
      if (after.has(old.idempotency_key)) continue
      if (settled.includes(old.idempotency_key)) {
        const event = events.find((item) => item.idempotency_key === old.idempotency_key)
        this.emitSubmission({ key: old.idempotency_key, state: 'accepted', result: null, meta: old.meta, request: old.request, commit_id: event?.commit_id, seq: event?.seq })
      } else {
        if (old.applied) moved.push(...localTouched(old.request))
        this.emitSubmission({ key: old.idempotency_key, state: 'discarded', result: null, meta: old.meta, request: old.request })
      }
    }
    if (moved.length > 0) this.emitChange({ seq: this.confirmedSeq, local: true, touched: moved })
  }

  // ---- sync loop (design §6.4)

  private wake() {
    this.wakeAbort.abort()
    this.waitAbort?.abort()
  }

  private async syncLoop() {
    let backoff = 500
    while (!this.closed && !this.isStopped()) {
      this.wakeAbort = new AbortController()
      try {
        await this.reconnect()
        if (this.closed || this.isStopped()) break
        this.setStatus({ kind: 'live' })
        backoff = 500
        if (this.wakeAbort.signal.aborted) continue
        // wait for a remote change or for local work, whichever comes first (a transport that cannot abort
        // its request simply has the abandoned long poll run out in the background)
        const poll = new AbortController()
        this.waitAbort = poll
        const remote = this.client.waitChanges(this.ws, this.epoch, this.confirmedSeq, 25000, poll.signal)
        const wakeSignal = this.wakeAbort.signal
        const local = new Promise<null>((resolve) => { wakeSignal.addEventListener('abort', () => resolve(null), { once: true }) })
        const woke = await Promise.race([remote, local])
        if (woke === null) { poll.abort(); void remote.catch(() => undefined); continue }
        if (!woke.ok) this.streamError(woke.error.code, woke.error.detail)
      } catch (error) {
        if (this.closed || this.isStopped()) break
        if (error instanceof StorageFailure) {
          // the replica cannot be written: nothing may be sent or confirmed until it can
          this.noteStorage(error)
          if (this.replica.dead !== null) break
          await wait(3000, this.wakeAbort.signal)
          continue
        }
        if (!(error instanceof TransportError)) {
          console.error('[aiworkspace] replica sync failed', error)
          this.stop('REPLICA_BROKEN')
          break
        }
        this.setStatus({ kind: 'offline', detail: error.message, browserOffline: browserOffline() })
        // local edits do not hurry the next attempt; the browser's `online` event does
        await wait(backoff, this.wakeAbort.signal)
        backoff = Math.min(backoff * 2, 8000)
      } finally {
        this.waitAbort = null
      }
    }
  }

  private streamError(code: string, detail?: string) {
    if (SESSION_STOP_CODES.has(code) || code === 'PERMISSION_DENIED' || code === 'NOT_FOUND') { this.stop(code); return }
    throw new TransportError('protocol', `${code}${detail ? `: ${detail}` : ''}`)
  }

  private async reconnect() {
    // 2. still allowed, same history?
    const info = await this.client.wsGetInfo(this.ws)
    if (!info.ok) { this.streamError(info.error.code, info.error.detail); return }
    if (info.epoch !== this.epoch) { this.stop('EPOCH_MISMATCH'); return }
    if (info.title !== this.wsInfo.title || info.capabilities.join() !== this.wsInfo.capabilities.join()) {
      this.wsInfo = { ...info, head_seq: this.confirmedSeq }
      noteTitle(this.workspaceId, info.title)
      await this.replica.call('updateInfo', this.wsInfo)
      this.emitChange({ seq: this.confirmedSeq, local: true, touched: [{ entity_id: 'root', change: 'renamed', rev: 0 }] })
    }
    // 3. catch up and rebase
    await this.catchUp()
    if (this.isStopped()) return
    // 4. submissions whose fate is unknown (response lost, or the tab was closed while sending)
    const undecided = (item: PendingRow) => item.state === 'unknown' || item.state === 'sending'
    for (const key of this.rows.filter(undecided).map((item) => item.idempotency_key)) {
      const row = this.rows.find((item) => item.idempotency_key === key)
      if (!row || !undecided(row)) continue // settled by a catch-up in the meantime
      const submission = await this.client.getSubmission(this.ws, this.epoch, row.idempotency_key)
      if (!submission.ok) { this.streamError(submission.error.code, submission.error.detail); return }
      if (submission.status === 'accepted') await this.catchUp() // accepted after our catch-up: the stream settles it by key
      else await this.mark(row.idempotency_key, 'queued', null)
      if (this.isStopped()) return
    }
    // 5. send, one at a time
    while (!this.closed && !this.isStopped()) {
      let request: CommitRequest | null
      try {
        request = await this.replica.call('nextToSend')
      } catch (error) {
        // the next one builds on a submission that is not settled yet: try again after the next change
        if (error instanceof ServiceFailure && error.code === 'DEPENDENCY_UNAVAILABLE') break
        throw error
      }
      if (!request) break
      const key = request.idempotency_key
      await this.mark(key, 'sending', null)
      let result: CommitResult
      try {
        result = await this.client.commit(request)
      } catch (error) {
        if (error instanceof TransportError) await this.mark(key, 'unknown', null)
        throw error
      }
      if (result.status === 'accepted') {
        // not removed here: the change stream brings it back by key — the only path that advances the confirmed layer
        await this.catchUp()
      } else if (SESSION_STOP_CODES.has(result.code)) {
        await this.mark(key, 'queued', null) // not judged: it was written against a history that is gone
        this.stop(result.code)
      } else {
        await this.mark(key, result.status, result as unknown as Json)
      }
    }
  }

  private mark(key: string, state: PendingState, result: Json | null): Promise<void> {
    return this.exclusive(async () => {
      this.adopt(await this.replica.call('mark', key, state, result))
      this.clearStorageIssue()
    })
  }

  private async catchUp() {
    for (;;) {
      if (this.closed || this.isStopped()) return
      const page = await this.client.getChanges(this.ws, this.epoch, this.confirmedSeq)
      if (!page.ok) { this.streamError(page.error.code, page.error.detail); return }
      if (page.changes.length > 0) {
        await this.exclusive(async () => {
          let applied: { confirmed_seq: number; settled: string[]; pending: PendingRow[] }
          try {
            applied = await this.replica.call('applyRemote', page.changes)
          } catch (error) {
            if (error instanceof ServiceFailure) { this.stop('REPLICA_BROKEN'); return }
            throw error
          }
          this.clearStorageIssue()
          const from = this.confirmedSeq
          this.confirmedSeq = applied.confirmed_seq
          this.wsInfo = { ...this.wsInfo, head_seq: Math.max(this.wsInfo.head_seq, applied.confirmed_seq) }
          for (const event of page.changes) if (event.seq > from) this.emitChange(event)
          this.adopt(applied.pending, applied.settled, page.changes)
        })
      }
      for (let i = this.seqWaiters.length - 1; i >= 0; i--) {
        if (this.seqWaiters[i].seq <= this.confirmedSeq) this.seqWaiters.splice(i, 1)[0].resolve()
      }
      if (!page.more) return
    }
  }

  // ---- reads: the working view

  private limitCapabilities<T extends EntityEnvelope>(envelope: T): T {
    // the replica plans with full capabilities (the backend decides); show only what this caller was granted
    const granted = new Set<string>(this.wsInfo.capabilities)
    return { ...envelope, capabilities: envelope.capabilities.filter((capability) => granted.has(capability)) }
  }

  async outline() {
    const entities = await this.replica.call('outline')
    if (this.isLive() && entities.some((entity) => entity.write_policy === 'lock_required')) {
      // lock holders are deployment state, not part of the replica: ask while the backend is there
      try { this.lockHolders = new Map((await this.direct.lockList()).map((lock) => [lock.entity_id, lock])) } catch { /* shown without holders */ }
    } else if (!this.isLive()) {
      this.lockHolders = new Map()
    }
    return entities.map((entity) => {
      const holder = this.lockHolders.get(entity.entity_id)
      return this.limitCapabilities({ ...entity, lock_holder: holder ? { principal: holder.principal, session_id: holder.session_id, acquired_at: holder.acquired_at, expires_at: holder.expires_at, lock_id: holder.lock_id } : null })
    })
  }

  private shape<C>(read: EntityEnvelope & { content: unknown; head_seq: number }): ReadOk<C> {
    const result = this.limitCapabilities(read)
    if (result.type_id === 'buckyos.asset-ref' && result.content && typeof result.content === 'object') {
      // availability is a fact of the backend's object store; offline the bytes come from the asset area or not at all
      result.content = { availability: 'available', ...(result.content as Record<string, unknown>) }
    }
    return result as ReadOk<C>
  }

  async read<C>(entityId: string, selector?: Selector) {
    return this.shape<C>(await this.replica.call('read', entityId, selector ?? null))
  }

  async readMany(targets: { entity_id: string; selector?: Selector }[]) {
    return (await this.replica.call('readMany', targets)).map((entry) => ('error' in entry ? entry : this.shape<unknown>(entry)))
  }

  async listAnnotations(params: ListAnnotationsParams) {
    return (await this.replica.call('listAnnotations', params)).map((entry) => this.shape<AnnotationContent>(entry))
  }

  async query(params: QueryParams): Promise<QueryPage> {
    try {
      return await this.replica.call('query', params)
    } catch (error) {
      if (!(error instanceof ServiceFailure) || error.code !== 'DEPENDENCY_UNAVAILABLE') throw error
      // a URL query table: its rows exist only at the source, the replica has none of them
      if (this.isLive()) return this.direct.query(params)
      throw new ServiceFailure({ code: 'DEPENDENCY_UNAVAILABLE', detail: 'URL 查询表的数据只在数据源上，需要连接后台才能查询；当前离线，没有任何结果可显示' })
    }
  }

  async getCollabState(entityId: string): Promise<CollabState> {
    const state = await this.replica.call('collab', entityId)
    return {
      entity_id: entityId, lineage_id: state.lineage_id, engine: 'loro', engine_version: '', encoding: 'snapshot', snapshot: state.snapshot,
      seq: this.confirmedSeq, content_rev: 0, pending_updates: state.pending_updates,
    }
  }

  // ---- writes

  private nextKey() {
    this.keyCounter += 1
    return `${this.sessionId}/r${this.keyCounter}`
  }

  private offlineFailure(what: string): ServiceFailure {
    const status = this.currentStatus
    const why = status.kind === 'stopped' ? '同步已停止' : status.kind === 'offline' && status.browserOffline ? '浏览器处于离线状态' : '后台不可达'
    return new ServiceFailure({ code: 'BACKEND_UNREACHABLE', detail: `${what}（${why}）` })
  }

  async commit(operations: Operation[], options?: CommitOptions): Promise<CommitOutcome> {
    const key = this.nextKey()
    const status = this.currentStatus
    if (status.kind === 'stopped') {
      return { status: 'rejected', code: status.code, retryable: false, detail: status.detail, errors: [{ code: status.code, detail: status.detail }], idempotency_key: key }
    }
    const request: CommitRequest = {
      protocol_version: PROTOCOL_VERSION,
      workspace_id: this.workspaceId,
      epoch: this.epoch,
      idempotency_key: key,
      session_id: this.sessionId,
      origin: 'human',
      ...(options?.undoGroup ? { undo_group: options.undoGroup } : {}),
      ...(options?.message ? { message: options.message } : {}),
      operations,
    }
    const outcome = await this.exclusive(async (): Promise<CommitOutcome | 'direct'> => {
      let local: LocalSaved | LocalRefused
      try {
        local = await this.replica.call('submit', request, options?.meta ?? (options?.label ? { label: options.label } : null))
      } catch (error) {
        if (!(error instanceof StorageFailure)) throw error
        this.noteStorage(error)
        return { status: 'storage_failed', idempotency_key: key, detail: describeStorageFailure(error) }
      }
      if (local.status === 'saved_locally') {
        // only now — the Worker answered after its transaction committed — is the edit saved on this device
        const saved = local as LocalSaved
        this.clearStorageIssue()
        this.adopt(saved.pending)
        if (this.isLive() || this.currentStatus.kind === 'connecting') this.wake()
        return { status: 'saved_locally', idempotency_key: key, provisional_seq: saved.provisional_seq }
      }
      const refused = local as LocalRefused
      if (refused.code === 'LOCK_REQUIRED') {
        // design §2.11: a `lock_required` object is not edited through the replica. Online, the commit goes
        // straight to the backend (which checks this session's lock); unreachable, the object is read-only.
        if (this.isLive()) return 'direct'
        const detail = '此对象启用了写锁：后台不可达时它是只读的，这次修改没有保存'
        return { status: 'rejected', code: 'LOCK_REQUIRED', retryable: true, detail, errors: [{ code: 'LOCK_REQUIRED', detail }], idempotency_key: key }
      }
      return { ...(refused as unknown as CommitResult), idempotency_key: key }
    })
    return outcome === 'direct' ? this.direct.commit(operations, options) : outcome
  }

  retryUnknown(idempotencyKey: string) { return this.direct.retryUnknown(idempotencyKey) }

  async prepare(operations: Operation[]): Promise<PrepareResult> {
    if (!this.isLive()) throw this.offlineFailure('预检在后台执行，现在无法进行')
    return this.direct.prepare(operations)
  }

  undoCommit(commitId: string, options?: { mode?: 'all_or_nothing' | 'partial'; planDigest?: string }) {
    return this.direct.undoCommit(commitId, options)
  }

  private discardPending(key: string, force: boolean): Promise<Operation[] | null> {
    return this.exclusive(async () => {
      const result = await this.replica.call('discard', key, !force)
      this.clearStorageIssue()
      this.adopt(result.pending)
      return result.removed ? result.removed.request.operations : null
    })
  }

  private async requeuePending(key: string) {
    await this.mark(key, 'queued', null)
    this.wake()
  }

  private exportLocal(): Promise<LocalExport> {
    return this.replica.call('exportLocal')
  }

  private async destroy() {
    this.closed = true
    this.wake()
    await this.replica.call('destroy')
    forgetPrepared(this.workspaceId)
  }

  // ---- needs the backend: say so when it is not there

  async lockAcquire(entityId: string) {
    if (!this.isLive()) throw this.offlineFailure('需要写锁的对象在后台不可达时只读')
    return this.direct.lockAcquire(entityId)
  }
  lockRenew(lockIds: string[]) { return this.direct.lockRenew(lockIds) }
  lockRelease(lockIds: string[]) { return this.direct.lockRelease(lockIds) }
  lockBreak(entityId: string) { return this.direct.lockBreak(entityId) }
  lockList() { return this.direct.lockList() }

  async uploadAsset(file: Blob, fileName?: string) {
    if (!this.isLive()) throw this.offlineFailure('上传资产需要连接后台，现在没有上传（离线暂存后延迟上传尚未实现）')
    return this.direct.uploadAsset(file, fileName)
  }

  async fetchAsset(objectId: string): Promise<Blob> {
    const cached = this.replica.dead === null ? await this.replica.call('getAsset', objectId).catch(() => null) : null
    if (cached) return new Blob([cached])
    if (!this.isLive()) throw this.offlineFailure('这个资产的内容没有缓存到本机（准备离线时被跳过，或之后才加入）')
    const blob = await this.direct.fetchAsset(objectId)
    if (blob.size <= ASSET_SIZE_CAP) void blob.arrayBuffer().then((bytes) => this.replica.call('putAsset', objectId, bytes)).catch(() => undefined)
    return blob
  }

  async procStart(program: string, params: Record<string, Json>): Promise<RunView> {
    if (!this.isLive()) throw this.offlineFailure('受控加工在后台执行，现在没有运行，也没有生成任何结果')
    return this.direct.procStart(program, params)
  }
  async procGet(runId: string) {
    if (!this.isLive()) throw this.offlineFailure('无法读取运行状态')
    return this.direct.procGet(runId)
  }
  async procApply(runId: string) {
    if (!this.isLive()) throw this.offlineFailure('受控加工的结果由后台应用，现在没有应用')
    return this.direct.procApply(runId)
  }
  async procCancel(runId: string) {
    if (!this.isLive()) throw this.offlineFailure('无法取消运行')
    return this.direct.procCancel(runId)
  }

  close() {
    if (this.disposed) return
    this.disposed = true
    this.closed = true
    window.removeEventListener('online', this.onOnline)
    this.offDeath()
    this.wake()
    this.statusListeners.clear()
    this.changeListeners.clear()
    this.submissionListeners.clear()
    this.seqWaiters.splice(0).forEach((waiter) => waiter.resolve())
    // the lock is released only after the database is closed: the next holder must find the files free
    this.released = this.serial.then(() => this.replica.close()).catch(() => undefined).then(() => this.lock.release())
  }

  whenClosed() { return this.released }
}
