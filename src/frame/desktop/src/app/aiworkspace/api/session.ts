/* WorkspaceSession: the single interface the UI reads and writes one Workspace through.
 *
 * `OnlineWorkspaceSession` is the direct-to-backend implementation (design §6.1 "在线直连模式").
 * `ReplicaWorkspaceSession` (offline/replicaSession.ts) implements the same interface on the offline
 * replica (design §6): reads come from the Replica Worker's working view, `commit` becomes
 * "saved on this device → queued → sent". Nothing above this file talks to the client directly. */

import { AiwsClient, ServiceFailure, unwrap } from './client'
import { randomId } from './ids'
import { TransportError } from './transport'
import type { LocalExport, PendingMeta, PendingRow, PendingState, RichTextDraftRow } from '../offline/protocol'
import {
  PROTOCOL_VERSION,
  type CollabState, type CommitEvent, type CommitOutcome, type CommitRequest, type CommitResult, type EntityEnvelope, type Json,
  type LockInfo, type Operation, type PrepareResult, type QueryPage, type QueryParams, type RunView, type Selector, type WorkspaceInfo,
  type AnnotationRead, type ListAnnotationsParams, type Capability, type DerivedRecord, type FreshnessInfo, type GrantList, type RelationsInfo,
  type Subject, type VersionInfo, type WishChoices, type WishRunView, type ShowLockInfo,
} from './types'

export type SessionStatus =
  | { kind: 'connecting' }
  | { kind: 'live' }
  /** The backend cannot be reached right now; the follower keeps retrying.
   * `browserOffline`: the browser itself reports no network (as opposed to "network up, BuckyOS unreachable"). */
  | { kind: 'offline'; detail: string; browserOffline: boolean }
  /** Synchronisation stopped for good (history replaced, access lost…): nothing is sent any more. */
  | { kind: 'stopped'; code: string; detail: string }

export interface CommitOptions {
  message?: string
  undoGroup?: string
  /** Read-set preconditions (design §2.6): the commit applies only if these version cells are unchanged. */
  preconditions?: unknown[]
  origin?: 'human' | 'agent' | 'program'
  runId?: string
  /** Label of the edit in the save-state list. */
  label?: string
  /** Replica session: stored with the pending row so the edit can be shown again after a restart. */
  meta?: PendingMeta
}

/** How this window is connected to the Workspace (design §6.1). */
export type SessionMode =
  /** Online direct: every read and write goes to the backend. `not_holder` / `unavailable` windows are
   * read-only while the backend is unreachable. */
  | { kind: 'direct'; reason: 'not_prepared' | 'not_holder' | 'unavailable'; detail: string }
  /** This window holds the Workspace's offline replica. */
  | { kind: 'replica'; preparedAt: string; persisted: boolean }

/** A pending submission changed: saved, state change, accepted (settled by key), or discarded by the user. */
export interface SubmissionEvent {
  key: string
  state: PendingState | 'accepted' | 'discarded'
  result: Json | null
  meta: PendingMeta | null
  request: CommitRequest
  /** `accepted` only. */
  commit_id?: string
  seq?: number
}

/** What only a replica-backed session offers. */
export interface OfflineControls {
  /** Pending submissions in local order (durable rows of the replica database). */
  pending(): readonly PendingRow[]
  subscribeSubmissions(listener: (event: SubmissionEvent) => void): () => void
  /** Why the replica database is unusable right now (Worker gone, quota, transaction error), or null. */
  storageProblem(): string | null
  /** Remove a submission that was not sent yet (undo / "discard my input"). No network. Resolves with its operations. */
  discardPending(key: string, options?: { force?: boolean }): Promise<Operation[] | null>
  /** Put a refused submission back into the send queue unchanged (same key). */
  requeuePending(key: string): Promise<void>
  /** Pending submissions and drafts as stored in the replica database. */
  exportLocal(): Promise<LocalExport>
  /** Keep a refused rich text content in the replica database (design §6.2 `drafts`). */
  saveDraft(draft: RichTextDraftRow): Promise<void>
  /** Delete this device's replica of the Workspace (pending submissions included). */
  destroy(): Promise<void>
  /** e2e hooks (dev override only). */
  test: { failTransactions(kind: 'quota' | 'error', count: number): Promise<void>; killWorker(): void } | null
}

export type ReadOk<C> = EntityEnvelope & { content: C; head_seq: number }

export interface WorkspaceSession {
  readonly workspaceId: string
  /** Per-tab session id: undo stack ownership and write-lock holder identity. */
  readonly sessionId: string
  readonly principal: string | null
  mode(): SessionMode
  /** Non-null exactly when `mode().kind === 'replica'`. */
  readonly offline: OfflineControls | null
  info(): WorkspaceInfo
  status(): SessionStatus
  appliedSeq(): number
  /** The show presenting this workspace (第三期规划 §8.1): writes are closed until it ends. Reported with the
   * status listeners; learnt from the change-stream follower and from refused commits. */
  showLock(): ShowLockInfo | null
  noteShowLock(lock: ShowLockInfo | null): void
  subscribeStatus(listener: () => void): () => void
  /** Commit events in `seq` order, each exactly once (own commits included). */
  subscribeChanges(listener: (event: CommitEvent) => void): () => void
  /** Resolves once the change stream has been applied up to `seq`. */
  whenApplied(seq: number): Promise<void>

  outline(): Promise<EntityEnvelope[]>
  read<C>(entityId: string, selector?: Selector): Promise<ReadOk<C>>
  readMany(targets: { entity_id: string; selector?: Selector }[]): Promise<(ReadOk<unknown> | { error: { code: string; detail?: string } })[]>
  /** Annotations anchored to `target_ids` and/or placed under `parent_id`, each with its anchor. */
  listAnnotations(params: ListAnnotationsParams): Promise<AnnotationRead[]>
  query(params: QueryParams): Promise<QueryPage>
  getCollabState(entityId: string): Promise<CollabState>
  /** Dependency-based freshness of results and wishes (computed by the core, phase two §7.5). */
  freshness(entityIds: string[]): Promise<FreshnessInfo[]>
  /** What an entity references / is referenced by / was generated from (phase two §6.2). */
  relations(entityId: string): Promise<RelationsInfo>
  /** Addressable versions of an entity (needs the backend). */
  listVersions(entityId: string): Promise<{ versions: VersionInfo[]; content_rev: number }>
  /** The operations restoring a stored version, to be submitted as an ordinary commit (needs the backend). */
  restoreVersionPlan(entityId: string, contentRev: number): Promise<{ operations: Operation[]; derived: DerivedRecord | null }>

  /** User work state on the server (phase two §4.4): per subject, per Workspace; never a document Commit. Needs the backend. */
  getUserState(): Promise<Record<string, Json>>
  setUserState(entries: Record<string, Json | null>): Promise<void>

  /** Deployment management (online only; never queued, never undoable). */
  listGrants(): Promise<GrantList>
  grant(subject: string, capabilities: Capability[], scopeEntityId?: string): Promise<void>
  revoke(subject: string, scopeEntityId?: string): Promise<void>
  listSubjects(): Promise<Subject[]>

  commit(operations: Operation[], options?: CommitOptions): Promise<CommitOutcome>
  /** Resend a submission whose result is still unknown: same key, same content. */
  retryUnknown(idempotencyKey: string): Promise<CommitOutcome>
  prepare(operations: Operation[]): Promise<PrepareResult>
  undoCommit(commitId: string, options?: { mode?: 'all_or_nothing' | 'partial'; planDigest?: string }): Promise<CommitOutcome>

  lockAcquire(entityId: string): Promise<{ lock_id: string; expires_at: string }>
  lockRenew(lockIds: string[]): Promise<void>
  lockRelease(lockIds: string[]): Promise<void>
  lockBreak(entityId: string): Promise<void>
  lockList(): Promise<LockInfo[]>

  uploadAsset(file: Blob, fileName?: string): Promise<{ object_id: string; media_type: string; size: number }>
  fetchAsset(objectId: string): Promise<Blob>

  procStart(program: string, params: Record<string, Json>): Promise<RunView>
  procGet(runId: string): Promise<RunView>
  procApply(runId: string): Promise<RunView>
  procCancel(runId: string): Promise<{ state: string; already_applied: boolean; commit_id?: string }>

  /** Wish runs on the service (need the backend): start a stage, read it (with a preview for `choices`), apply a previewed plan, list. */
  wishStart(program: 'wish.xllm@1' | 'wish.mock@1', params: Record<string, Json>): Promise<WishRunView>
  wishGet(runId: string, choices?: WishChoices): Promise<WishRunView>
  wishApply(runId: string, planDigest: string): Promise<WishRunView>
  wishList(wishId: string, limit?: number): Promise<WishRunView[]>

  close(): void
  /** Resolves once a closed session released what it held exclusively (replica database, holder lock). */
  whenClosed(): Promise<void>
}

const STOP_CODES = new Set(['EPOCH_MISMATCH', 'BASE_TOO_OLD', 'VERSION_MISMATCH'])
const STOP_TEXT: Record<string, string> = {
  VERSION_MISMATCH: '后台已升级到新版本的 AI Workspace，本页面是旧版本：已停止同步，未提交的修改仍保留在本窗口。请刷新页面。',
  EPOCH_MISMATCH: '此工作区的历史已被恢复操作替换。本窗口基于旧历史的版本号与未提交修改不再适用，已停止同步；请重新打开工作区。',
  BASE_TOO_OLD: '变化流已不覆盖本窗口的基准，已停止同步；请重新打开工作区。',
  PERMISSION_DENIED: '你对此工作区的访问权限已被撤回，已停止同步。',
  NOT_FOUND: '此工作区已不存在或你已无权访问，已停止同步。',
}

export const SESSION_STOP_CODES: ReadonlySet<string> = STOP_CODES
export const SESSION_STOP_TEXT: Readonly<Record<string, string>> = STOP_TEXT

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => window.setTimeout(resolve, ms))
}

/** A different protocol version on the other side (标准对象的交互改进 §7.1, 连接线方案 §10.3): this page must be
 * reloaded, which is not the same as a broken replica. A refusal of a request's own version says the same. */
export function versionMismatch(info: { protocol_version?: string }): boolean {
  return Boolean(info.protocol_version) && info.protocol_version !== PROTOCOL_VERSION
}
export function refusedVersion(code: string | undefined, detail: string | undefined): boolean {
  return code === 'UNSUPPORTED_VERSION' && /protocol_version/.test(detail ?? '')
}

export function browserOffline(): boolean {
  return typeof navigator !== 'undefined' && navigator.onLine === false
}

/** A session that follows the change stream itself and lends this one its position (the replica session). */
export interface SessionHost {
  poke(): void
  whenApplied(seq: number): Promise<void>
  onStop(code: string, detail: string): void
}

export class OnlineWorkspaceSession implements WorkspaceSession {
  readonly workspaceId: string
  readonly sessionId = randomId('s')
  readonly offline = null
  private readonly client: AiwsClient
  private readonly sessionMode: SessionMode
  private readonly host: SessionHost | null
  private wsInfo: WorkspaceInfo
  private currentStatus: SessionStatus = { kind: 'connecting' }
  private lock: ShowLockInfo | null = null
  private applied: number
  private keyCounter = 0
  private closed = false
  private waitAbort: AbortController | null = null
  private catchUpChain: Promise<void> = Promise.resolve()
  private readonly statusListeners = new Set<() => void>()
  private readonly changeListeners = new Set<(event: CommitEvent) => void>()
  private readonly seqWaiters: { seq: number; resolve: () => void }[] = []
  /** Requests whose result is unknown, kept verbatim for a same-key resend. */
  private readonly unknown = new Map<string, { kind: 'commit'; request: CommitRequest } | { kind: 'undo'; params: Parameters<AiwsClient['undo']>[1] }>()

  private constructor(client: AiwsClient, info: WorkspaceInfo, mode: SessionMode, host: SessionHost | null) {
    this.client = client
    this.workspaceId = info.workspace_id
    this.wsInfo = info
    this.applied = info.head_seq
    this.sessionMode = mode
    this.host = host
    this.lock = info.show_lock ?? null
  }

  static async open(client: AiwsClient, workspaceId: string, mode: SessionMode = { kind: 'direct', reason: 'not_prepared', detail: '尚未为此工作区准备离线' }): Promise<OnlineWorkspaceSession> {
    const info = unwrap(await client.wsGetInfo({ workspace_id: workspaceId }))
    const session = new OnlineWorkspaceSession(client, info, mode, null)
    if (versionMismatch(info)) session.noteCode('VERSION_MISMATCH')
    else void session.follow()
    return session
  }

  /** The direct-to-backend calls of a replica session (locks, uploads, Mock runs, commits of lock-protected
   * objects, undo): no follower of its own, the host owns the change stream. */
  static hosted(client: AiwsClient, info: WorkspaceInfo, host: SessionHost): OnlineWorkspaceSession {
    return new OnlineWorkspaceSession(client, info, { kind: 'direct', reason: 'not_prepared', detail: '' }, host)
  }

  mode() { return this.sessionMode }

  get principal(): string | null { return this.client.transport.principalHint }
  private get ws() { return { workspace_id: this.workspaceId } }
  private get epoch() { return this.wsInfo.epoch }

  private isStopped(): boolean { return this.currentStatus.kind === 'stopped' }

  /** A window that could not become the replica holder is read-only while the backend is unreachable. */
  private readOnlyNow(): boolean {
    return this.host === null && this.sessionMode.kind === 'direct' && this.sessionMode.reason !== 'not_prepared' && this.currentStatus.kind === 'offline'
  }

  info() { return this.wsInfo }
  status() { return this.currentStatus }
  appliedSeq() { return this.applied }
  showLock() { return this.lock }

  noteShowLock(lock: ShowLockInfo | null) {
    if ((lock?.show_id ?? null) === (this.lock?.show_id ?? null)) return
    this.lock = lock
    this.statusListeners.forEach((listener) => listener())
  }

  subscribeStatus(listener: () => void) {
    this.statusListeners.add(listener)
    return () => { this.statusListeners.delete(listener) }
  }

  subscribeChanges(listener: (event: CommitEvent) => void) {
    this.changeListeners.add(listener)
    return () => { this.changeListeners.delete(listener) }
  }

  whenApplied(seq: number): Promise<void> {
    if (this.host) return this.host.whenApplied(seq)
    if (this.applied >= seq || this.currentStatus.kind === 'stopped') return Promise.resolve()
    this.poke()
    return new Promise((resolve) => { this.seqWaiters.push({ seq, resolve }) })
  }

  private setStatus(status: SessionStatus) {
    if (this.currentStatus.kind === 'stopped') return
    const previous = this.currentStatus
    const same = previous.kind === status.kind && (status.kind !== 'offline' || (previous.kind === 'offline' && previous.detail === status.detail && previous.browserOffline === status.browserOffline))
    this.currentStatus = status
    if (!same) this.statusListeners.forEach((listener) => listener())
    if (status.kind === 'stopped') {
      this.waitAbort?.abort()
      this.seqWaiters.splice(0).forEach((waiter) => waiter.resolve())
    }
  }

  /** A stop code makes every later request pointless or harmful: remember it and tell the user. */
  private noteCode(code: string | undefined, detail?: string) {
    if (refusedVersion(code, detail)) code = 'VERSION_MISMATCH'
    if (!code) return
    if (!STOP_CODES.has(code)) return
    this.setStatus({ kind: 'stopped', code, detail: STOP_TEXT[code] ?? detail ?? code })
    this.host?.onStop(code, STOP_TEXT[code] ?? detail ?? code)
  }

  private fail(error: unknown): never {
    if (error instanceof ServiceFailure) this.noteCode(error.code, error.message)
    throw error
  }

  // ---- change stream follower (design §2.8): wake-up → get_changes → apply by seq

  private async follow() {
    let backoff = 500
    while (!this.closed && !this.isStopped()) {
      try {
        // back after an outage: the service may have been upgraded meanwhile
        if (this.currentStatus.kind === 'offline') {
          const info = await this.client.wsGetInfo(this.ws)
          if (info.ok && versionMismatch(info)) { this.noteCode('VERSION_MISMATCH'); break }
        }
        await this.catchUp()
        if (this.closed || this.isStopped()) break
        this.setStatus({ kind: 'live' })
        backoff = 500
        this.waitAbort = new AbortController()
        const woke = await this.client.waitChanges(this.ws, this.epoch, this.applied, 25000, this.waitAbort.signal)
        if (!woke.ok) this.handleStreamError(woke.error.code, woke.error.detail)
        else this.noteShowLock(woke.show_lock ?? null)
      } catch (error) {
        if (this.closed || this.isStopped()) break
        if (!(error instanceof TransportError)) throw error
        this.setStatus({ kind: 'offline', detail: error.message, browserOffline: browserOffline() })
        await sleep(backoff)
        backoff = Math.min(backoff * 2, 8000)
      }
    }
  }

  private handleStreamError(code: string, detail?: string) {
    if (STOP_CODES.has(code)) { this.noteCode(code, detail); return }
    if (code === 'PERMISSION_DENIED' || code === 'NOT_FOUND') {
      this.setStatus({ kind: 'stopped', code, detail: STOP_TEXT[code] })
      return
    }
    throw new TransportError('protocol', `${code}${detail ? `: ${detail}` : ''}`)
  }

  private poke() {
    if (this.host) { this.host.poke(); return }
    void this.catchUp().catch(() => { /* the follower loop reports connectivity */ })
  }

  private catchUp(): Promise<void> {
    const run = this.catchUpChain.then(() => this.catchUpNow())
    this.catchUpChain = run.catch(() => undefined)
    return run
  }

  private async catchUpNow() {
    for (;;) {
      if (this.closed || this.isStopped()) return
      const page = await this.client.getChanges(this.ws, this.epoch, this.applied)
      if (!page.ok) { this.handleStreamError(page.error.code, page.error.detail); return }
      for (const event of page.changes) {
        if (event.seq <= this.applied) continue // duplicate delivery
        this.applied = event.seq
        this.wsInfo = { ...this.wsInfo, head_seq: Math.max(this.wsInfo.head_seq, event.seq) }
        for (const listener of [...this.changeListeners]) {
          try { listener(event) } catch (error) { console.error('[aiworkspace] change listener failed', error) }
        }
      }
      for (let i = this.seqWaiters.length - 1; i >= 0; i--) {
        if (this.seqWaiters[i].seq <= this.applied) this.seqWaiters.splice(i, 1)[0].resolve()
      }
      if (!page.more) return
    }
  }

  // ---- reads

  async outline() {
    try { return unwrap(await this.client.outline(this.ws)).entities } catch (error) { return this.fail(error) }
  }

  async read<C>(entityId: string, selector?: Selector) {
    try { return unwrap(await this.client.read<C>(this.ws, entityId, selector)) as ReadOk<C> } catch (error) { return this.fail(error) }
  }

  async readMany(targets: { entity_id: string; selector?: Selector }[]) {
    try {
      const { results } = unwrap(await this.client.readMany(this.ws, targets))
      // entries of a batch read are bare envelopes or `{ error }` (no `ok` flag per entry)
      return (results as unknown[]).map((result) => {
        const entry = result as { error?: { code: string; detail?: string } }
        return entry.error ? { error: entry.error } : (result as ReadOk<unknown>)
      })
    } catch (error) { return this.fail(error) }
  }

  async listAnnotations(params: ListAnnotationsParams) {
    try { return unwrap(await this.client.listAnnotations(this.ws, params)).annotations } catch (error) { return this.fail(error) }
  }

  async query(params: QueryParams) {
    try { return unwrap(await this.client.query(this.ws, params)) } catch (error) { return this.fail(error) }
  }

  async getCollabState(entityId: string) {
    try { return unwrap(await this.client.getCollabState(this.ws, entityId)) } catch (error) { return this.fail(error) }
  }

  async freshness(entityIds: string[]) {
    if (entityIds.length === 0) return []
    try { return unwrap(await this.client.freshness(this.ws, entityIds)).items } catch (error) { return this.fail(error) }
  }

  async relations(entityId: string) {
    try { return unwrap(await this.client.relations(this.ws, entityId)) } catch (error) { return this.fail(error) }
  }

  async listVersions(entityId: string) {
    try { const r = unwrap(await this.client.listVersions(this.ws, entityId)); return { versions: r.versions, content_rev: r.content_rev } } catch (error) { return this.fail(error) }
  }

  async restoreVersionPlan(entityId: string, contentRev: number) {
    try { const r = unwrap(await this.client.restoreVersion(this.ws, entityId, contentRev)); return { operations: r.operations, derived: r.derived } } catch (error) { return this.fail(error) }
  }

  async getUserState() {
    try { return unwrap(await this.client.wsGetUserState(this.ws)).entries } catch (error) { return this.fail(error) }
  }

  async setUserState(entries: Record<string, Json | null>) {
    try { unwrap(await this.client.wsSetUserState(this.ws, entries)) } catch (error) { return this.fail(error) }
  }

  async listGrants() {
    try { const r = unwrap(await this.client.wsListGrants(this.ws)); return { grants: r.grants, complete: r.complete, owner: r.owner, principal: r.principal } } catch (error) { return this.fail(error) }
  }
  async grant(subject: string, capabilities: Capability[], scopeEntityId?: string) {
    try { unwrap(await this.client.wsGrant(this.ws, subject, capabilities, scopeEntityId)) } catch (error) { return this.fail(error) }
  }
  async revoke(subject: string, scopeEntityId?: string) {
    try { unwrap(await this.client.wsRevoke(this.ws, subject, scopeEntityId)) } catch (error) { return this.fail(error) }
  }
  async listSubjects() {
    try { return unwrap(await this.client.wsListSubjects(this.ws)).subjects } catch (error) { return this.fail(error) }
  }

  // ---- writes (design §2.6)

  private nextKey() {
    this.keyCounter += 1
    return `${this.sessionId}/${this.keyCounter}`
  }

  private refused(key: string): CommitOutcome | null {
    const status = this.currentStatus
    if (status.kind !== 'stopped') return null
    return { status: 'rejected', code: status.code, retryable: false, detail: status.detail, errors: [{ code: status.code, detail: status.detail }], idempotency_key: key }
  }

  private request(operations: Operation[], key: string, options?: CommitOptions): CommitRequest {
    return {
      protocol_version: PROTOCOL_VERSION,
      workspace_id: this.workspaceId,
      epoch: this.epoch,
      idempotency_key: key,
      session_id: this.sessionId,
      origin: options?.origin ?? 'human',
      ...(options?.undoGroup ? { undo_group: options.undoGroup } : {}),
      ...(options?.message ? { message: options.message } : {}),
      ...(options?.preconditions && options.preconditions.length > 0 ? { preconditions: options.preconditions } : {}),
      operations,
    }
  }

  commit(operations: Operation[], options?: CommitOptions): Promise<CommitOutcome> {
    const key = this.nextKey()
    const refused = this.refused(key)
    if (refused) return Promise.resolve(refused)
    if (this.readOnlyNow()) {
      // design §6.1: a window without the replica does not edit while the backend is unreachable
      const detail = '后台不可达，且此窗口未启用离线：当前只读，这次修改没有保存到任何地方'
      return Promise.resolve({ status: 'rejected', code: 'BACKEND_UNREACHABLE', retryable: true, detail, errors: [{ code: 'BACKEND_UNREACHABLE', detail }], idempotency_key: key })
    }
    const request = this.request(operations, key, options)
    return this.deliver(key, () => this.client.commit(request), { kind: 'commit', request })
  }

  undoCommit(commitId: string, options?: { mode?: 'all_or_nothing' | 'partial'; planDigest?: string }): Promise<CommitOutcome> {
    const key = this.nextKey()
    const refused = this.refused(key)
    if (refused) return Promise.resolve(refused)
    const params = {
      epoch: this.epoch, commit_id: commitId, idempotency_key: key, session_id: this.sessionId,
      ...(options?.mode ? { mode: options.mode } : {}), ...(options?.planDigest ? { plan_digest: options.planDigest } : {}),
    }
    return this.deliver(key, () => this.client.undo(this.ws, params), { kind: 'undo', params })
  }

  retryUnknown(idempotencyKey: string): Promise<CommitOutcome> {
    const pending = this.unknown.get(idempotencyKey)
    if (!pending) return Promise.resolve({ status: 'unknown', idempotency_key: idempotencyKey, detail: '没有可重发的请求' })
    const refused = this.refused(idempotencyKey)
    if (refused) return Promise.resolve(refused)
    return this.deliver(idempotencyKey, () => (pending.kind === 'commit' ? this.client.commit(pending.request) : this.client.undo(this.ws, pending.params)), pending)
  }

  /** Send; on a transport failure the result is unknown → `doc.get_submission` → resend the same key. */
  private async deliver(
    key: string,
    send: () => Promise<CommitResult>,
    keep: { kind: 'commit'; request: CommitRequest } | { kind: 'undo'; params: Parameters<AiwsClient['undo']>[1] },
  ): Promise<CommitOutcome> {
    let lastError = ''
    for (let attempt = 0; attempt < 3; attempt++) {
      let result: CommitResult | null = null
      try {
        result = await send()
      } catch (error) {
        if (!(error instanceof TransportError)) throw error
        lastError = error.message
        try {
          const submission = await this.client.getSubmission(this.ws, this.epoch, key)
          if (!submission.ok) {
            this.noteCode(submission.error.code, submission.error.detail)
            const refused = this.refused(key)
            if (refused) return refused
          } else if (submission.status === 'accepted' && submission.result) {
            result = submission.result
          } else {
            continue // not_found: the backend never accepted this key in this epoch → resend as is
          }
        } catch {
          break // the backend cannot even be asked: stay unknown
        }
      }
      if (result) {
        this.unknown.delete(key)
        if (result.status === 'rejected' && result.code === 'SHOW_LOCKED') this.noteShowLock((result.errors?.[0]?.data as ShowLockInfo | undefined) ?? null)
        if (result.status === 'accepted') this.poke()
        else this.noteCode(result.code, STOP_TEXT[result.code] ?? result.detail)
        const refused = result.status !== 'accepted' ? this.refused(key) : null
        return refused ?? { ...result, idempotency_key: key }
      }
    }
    this.unknown.set(key, keep)
    return { status: 'unknown', idempotency_key: key, detail: lastError || '后台不可达' }
  }

  async prepare(operations: Operation[]) {
    return this.client.prepare(this.request(operations, this.nextKey()))
  }

  // ---- write locks (design §2.11)

  async lockAcquire(entityId: string) {
    const result = unwrap(await this.client.lockAcquire(this.ws, [entityId], this.sessionId))
    const lock = result.locks.find((item) => item.entity_id === entityId)
    if (!lock) throw new ServiceFailure({ code: 'LOCK_NOT_REQUIRED', detail: 'the entity does not require a write lock' })
    return { lock_id: lock.lock_id, expires_at: lock.expires_at }
  }

  async lockRenew(lockIds: string[]) { unwrap(await this.client.lockRenew(this.ws, lockIds)) }
  async lockRelease(lockIds: string[]) { unwrap(await this.client.lockRelease(this.ws, lockIds)) }
  async lockBreak(entityId: string) { unwrap(await this.client.lockBreak(this.ws, entityId)) }
  async lockList() { return unwrap(await this.client.lockList(this.ws)).locks }

  // ---- assets (design §3.6): verified bytes first, then the reference is committed by the caller

  async uploadAsset(file: Blob, fileName?: string) {
    const begin = unwrap(await this.client.assetBeginUpload(this.ws, file.size, fileName))
    await this.client.transport.upload(begin.upload_id, file)
    const done = unwrap(await this.client.assetFinishUpload(this.ws, begin.upload_id))
    return { object_id: done.object_id, media_type: done.media_type, size: done.size }
  }

  fetchAsset(objectId: string) {
    return this.client.transport.download(`asset/${this.workspaceId}/${objectId}`)
  }

  // ---- controlled processing (design §7)

  async procStart(program: string, params: Record<string, Json>) {
    return unwrap(await this.client.procStart(this.ws, program, params, `${this.sessionId}/run/${randomId()}`))
  }
  async procGet(runId: string) { return unwrap(await this.client.procGet(this.ws, runId)) }
  async procApply(runId: string) {
    const run = unwrap(await this.client.procApply(this.ws, runId, this.sessionId))
    if (run.result && run.result.status === 'accepted') this.poke()
    return run
  }
  async procCancel(runId: string) { return unwrap(await this.client.procCancel(this.ws, runId)) }

  async wishStart(program: 'wish.xllm@1' | 'wish.mock@1', params: Record<string, Json>) {
    return unwrap(await this.client.wishStart(this.ws, program, params, `${this.sessionId}/wish/${randomId()}`))
  }
  async wishGet(runId: string, choices?: WishChoices) { return unwrap(await this.client.wishGet(this.ws, runId, choices)) }
  async wishApply(runId: string, planDigest: string) {
    const run = unwrap(await this.client.wishApply(this.ws, runId, planDigest, this.sessionId))
    if (run.applied?.status === 'accepted') this.poke()
    return run
  }
  async wishList(wishId: string, limit = 10) { return unwrap(await this.client.wishList(this.ws, wishId, limit)).runs }

  close() {
    this.closed = true
    this.waitAbort?.abort()
    this.statusListeners.clear()
    this.changeListeners.clear()
    this.seqWaiters.splice(0).forEach((waiter) => waiter.resolve())
  }

  whenClosed() { return Promise.resolve() }
}

export function describeError(error: unknown): string {
  if (error instanceof ServiceFailure) return error.message
  if (error instanceof TransportError) return `无法连接后台${browserOffline() ? '（浏览器处于离线状态）' : ''}：${error.message}`
  return error instanceof Error ? error.message : String(error)
}
