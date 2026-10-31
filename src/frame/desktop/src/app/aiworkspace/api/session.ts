/* WorkspaceSession: the single interface the UI reads and writes one Workspace through.
 *
 * `OnlineWorkspaceSession` is the direct-to-backend implementation (design §6.1 "在线直连模式").
 * `ReplicaWorkspaceSession` (offline/replicaSession.ts) implements the same interface on the offline
 * replica (design §6): reads come from the Replica Worker's working view, `commit` becomes
 * "saved on this device → queued → sent". Nothing above this file talks to the client directly. */

import type { AccessDenial, AdjustLocation, DenyReason } from 'buckyos'
import { ndm } from 'buckyos'
import { AiwsClient, ServiceFailure, unwrap } from './client'
import { randomId } from './ids'
import { TransportError } from './transport'
import { aiwsT, localizedLabels } from '../i18n'
import type { LocalExport, PendingMeta, PendingRow, PendingState, RichTextDraftRow } from '../offline/protocol'
import {
  PROTOCOL_VERSION,
  type CollabState, type CommitEvent, type CommitOutcome, type CommitRequest, type CommitResult, type EntityEnvelope, type Json,
  type LockInfo, type Operation, type PrepareResult, type QueryPage, type QueryParams, type RunView, type Selector, type WorkspaceInfo,
  type AnnotationRead, type ListAnnotationsParams, type Capability, type DerivedRecord, type FreshnessInfo, type GrantList, type RelationsInfo,
  type Subject, type VersionInfo, type WishChoices, type WishRunView, type ShowLockInfo, type ServiceError, type WishAiccMode, type WishClaim, type WishToolAnswer, type HostedExecConfig, type Publication, type Referrer,
  type BucketBinding, type StagedAsset, type ExtBuildResult, type ExtExportResult, type ExtPackResult, type LibraryPackage,
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
  /** The key this submission uses (内核架构改进 D13 draft protocol): a form Block records it with its draft before
   * sending, so a recovery can ask `doc.get_submission` whether the commit happened. */
  idempotencyKey?: string
  /** Replica session: stored with the pending row so the edit can be shown again after a restart. */
  meta?: PendingMeta
}

export type ImportedAsset = { kind: 'file'; name: string; object_id: string; size: number } | { kind: 'dir'; name: string; children: ImportedAsset[] }

/** One file into a Workspace (design §3.6, Durable Data Schema §3.3): uploaded through NDM and bound into the
 * caller's own `assets` bucket at finalize, then taken over by the service into the Workspace owner's bucket.
 * A caller without a bucket of its own (a guest) uploads unbound and the service binds what it may read. */
export async function uploadWorkspaceAsset(client: AiwsClient, workspaceId: string, file: File): Promise<StagedAsset> {
  const options = await client.transport.importOptions()
  const own = await client.transport.ownBucket('assets')
  const snapshot = await ndm.importFiles([file], { ...options, mode: 'single_file', autoStartUpload: false })
  const ws = { workspace_id: workspaceId }
  try {
    if (!own) {
      await ndm.ensureUploaded(snapshot.sessionId, options)
      return unwrap(await client.assetImportObject(ws, { obj_id: snapshot.selection.objectId }))
    }
    const status = await ndm.ensureUploaded(snapshot.sessionId, { ...options, bind: { bucketId: own, key: `${workspaceId}/${snapshot.selection.objectId}` } })
    if (!status.binding) throw new Error('the upload was not bound')
    return unwrap(await client.assetImportObject(ws, { bucket_id: status.binding.bucket_id, key: status.binding.key }))
  } finally { ndm.releaseImportSession(snapshot.sessionId) }
}

/** A workspace package into the caller's own `exports` bucket, ready for `ws.import`. */
export async function uploadWorkspacePackage(client: AiwsClient, file: File, key = `import/${randomId()}.bcanvas`): Promise<BucketBinding> {
  const own = await client.transport.ownBucket('exports')
  if (!own) throw new Error('A workspace package is imported by a signed-in account (it is uploaded into its own bucket).')
  const options = await client.transport.importOptions()
  const snapshot = await ndm.importFiles([file], { ...options, mode: 'single_file', autoStartUpload: false })
  try {
    const status = await ndm.ensureUploaded(snapshot.sessionId, { ...options, bind: { bucketId: own, key } })
    if (!status.binding) throw new Error('the upload was not bound')
    return { bucket_id: status.binding.bucket_id, key: status.binding.key }
  } finally { ndm.releaseImportSession(snapshot.sessionId) }
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
  /** Whose AICC the wish blocks use (AI Center Quota PRD §3.3); a manager's setting, online only. */
  setWishAiccMode(mode: WishAiccMode): Promise<WishAiccMode>
  /** The owner's hosted execution configuration (D22, T3-7); `manage`, online only. */
  setHostedExec(config: HostedExecConfig): Promise<HostedExecConfig>
  /** Publication by path (D26): `manage`; the link is in `url` when the deployment has a host. */
  publish(entityId: string, name?: string): Promise<Publication>
  unpublish(entityId: string): Promise<void>
  listPublications(): Promise<Publication[]>
  listReferrers(entityId: string): Promise<Referrer[]>
  /** Extension controls (扩展控件 P0, README “Extension controls”): builds, bundles, the caller's library; online only.
   * A refused build throws a ServiceFailure whose `data.diagnostics` say where. */
  extBuild(params: Record<string, Json>): Promise<ExtBuildResult>
  extExport(entityIds: string[], purpose?: 'package' | 'pull'): Promise<ExtExportResult>
  extImport(items: { pkg_id: string; ident: string }[]): Promise<ExtBuildResult>
  extInstallFrom(entityIds: string[], pkg: { name: string; version: string; title?: string }): Promise<{ pkg_id: string; version: string; action: string }>
  extListLibrary(): Promise<LibraryPackage[]>
  /** P1: the service packs controls as a .pikg (Q20); `downloadPack` fetches it, `installPack` puts it into the
   * caller's library (consuming the binding); a local .pikg file is uploaded into `exports` and installed (Q21). */
  extPack(entityIds: string[], pkg: { name: string; version: string; title?: string }): Promise<ExtPackResult>
  downloadPack(pack: ExtPackResult): Promise<Blob>
  installPack(binding: BucketBinding): Promise<{ pkg_id: string; version: string; action: string }>
  installPikgFile(file: File): Promise<{ pkg_id: string; version: string; action: string }>

  commit(operations: Operation[], options?: CommitOptions): Promise<CommitOutcome>
  /** Resend a submission whose result is still unknown: same key, same content. */
  retryUnknown(idempotencyKey: string): Promise<CommitOutcome>
  /** Whether a submission with this key was accepted (`doc.get_submission`, D08/D13). */
  submissionStatus(epoch: string, idempotencyKey: string): Promise<{ status: 'accepted' | 'not_found'; result?: CommitResult }>
  prepare(operations: Operation[]): Promise<PrepareResult>
  undoCommit(commitId: string, options?: { mode?: 'all_or_nothing' | 'partial'; planDigest?: string }): Promise<CommitOutcome>

  lockAcquire(entityId: string): Promise<{ lock_id: string; expires_at: string }>
  lockRenew(lockIds: string[]): Promise<void>
  lockRelease(lockIds: string[]): Promise<void>
  lockBreak(entityId: string): Promise<void>
  lockList(): Promise<LockInfo[]>

  /** A service method this interface does not wrap (a session's `session.*`, 第四期规划): raw kRPC, needs the backend. */
  call<T>(method: string, params: Record<string, unknown>): Promise<T>
  uploadAsset(file: Blob, fileName?: string): Promise<StagedAsset>
  importAssets(files: File[], folder: boolean, signal?: AbortSignal): Promise<ImportedAsset[]>
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
  /** The executor protocol (内核架构改进 T3-4): this browser as the executor of a local run. */
  procClaim(runId: string, claimToken: string): Promise<WishClaim>
  procHeartbeat(runId: string, attempt: number, progress?: Json): Promise<void>
  procReport(runId: string, attempt: number, body: Record<string, Json>): Promise<WishRunView>
  procTool(runId: string, attempt: number, name: string, args: Json): Promise<WishToolAnswer>
  procProgram(runId: string, attempt: number, body: { source: string; results: Json | null; log: string; files: Json }): Promise<WishToolAnswer>
  procFiles(runId: string, attempt: number): Promise<Record<string, { text?: string; base64?: string; skipped?: string }>>

  close(): void
  /** Resolves once a closed session released what it held exclusively (replica database, holder lock). */
  whenClosed(): Promise<void>
}

const STOP_CODES = new Set(['EPOCH_MISMATCH', 'BASE_TOO_OLD', 'VERSION_MISMATCH'])
const STOP_TEXT: Readonly<Record<string, string>> = localizedLabels({
  VERSION_MISMATCH: 'aiworkspace.core.session.stop.versionMismatch',
  EPOCH_MISMATCH: 'aiworkspace.core.session.stop.epochMismatch',
  BASE_TOO_OLD: 'aiworkspace.core.session.stop.baseTooOld',
  PERMISSION_DENIED: 'aiworkspace.core.session.stop.permissionDenied',
  NOT_FOUND: 'aiworkspace.core.session.stop.notFound',
})

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

  static async open(client: AiwsClient, workspaceId: string, mode: SessionMode = { kind: 'direct', reason: 'not_prepared', detail: aiwsT('aiworkspace.core.session.notPrepared') }): Promise<OnlineWorkspaceSession> {
    const info = unwrap(await client.wsGetInfo({ workspace_id: workspaceId }))
    if (info.principal) client.transport.learnPrincipal(info.principal)
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
  async setWishAiccMode(mode: WishAiccMode) {
    try {
      const applied = unwrap(await this.client.wsSetWishAicc(this.ws, mode)).wish_aicc_mode
      this.wsInfo = { ...this.wsInfo, wish_aicc_mode: applied }
      return applied
    } catch (error) { return this.fail(error) }
  }
  async setHostedExec(config: HostedExecConfig) {
    try {
      const applied = unwrap(await this.client.wsSetHostedExec(this.ws, config)).hosted_exec
      this.wsInfo = { ...this.wsInfo, hosted_exec: applied }
      return applied
    } catch (error) { return this.fail(error) }
  }
  async publish(entityId: string, name?: string) { try { return unwrap(await this.client.wsPublish(this.ws, entityId, name)) } catch (error) { return this.fail(error) } }
  async unpublish(entityId: string) { try { unwrap(await this.client.wsUnpublish(this.ws, entityId)) } catch (error) { return this.fail(error) } }
  async listPublications() { try { return unwrap(await this.client.wsListPublications(this.ws)).publications } catch (error) { return this.fail(error) } }
  async listReferrers(entityId: string) { try { return unwrap(await this.client.wsListReferrers(this.ws, entityId)).referrers } catch (error) { return this.fail(error) } }
  async extBuild(params: Record<string, Json>) { try { return unwrap(await this.client.extBuild(this.ws, params)) } catch (error) { return this.fail(error) } }
  async extExport(entityIds: string[], purpose: 'package' | 'pull' = 'package') { try { return unwrap(await this.client.extExport(this.ws, entityIds, purpose)) } catch (error) { return this.fail(error) } }
  async extImport(items: { pkg_id: string; ident: string }[]) { try { return unwrap(await this.client.extImport(this.ws, items)) } catch (error) { return this.fail(error) } }
  async extInstallFrom(entityIds: string[], pkg: { name: string; version: string; title?: string }) { try { return unwrap(await this.client.extInstallFrom(this.ws, entityIds, pkg)) } catch (error) { return this.fail(error) } }
  async extListLibrary() { try { return unwrap(await this.client.extListLibrary()).packages } catch (error) { return this.fail(error) } }
  async extPack(entityIds: string[], pkg: { name: string; version: string; title?: string }) { try { return unwrap(await this.client.extPack(this.ws, entityIds, pkg)) } catch (error) { return this.fail(error) } }
  async downloadPack(pack: ExtPackResult) { return this.client.transport.fetchDownload(pack.download) }
  async installPack(binding: BucketBinding) { try { return unwrap(await this.client.extInstallBound(binding)) } catch (error) { return this.fail(error) } }
  async installPikgFile(file: File) {
    const binding = await uploadWorkspacePackage(this.client, file, `aiws/${randomId()}.pikg`)
    return this.installPack(binding)
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
    const key = options?.idempotencyKey ?? this.nextKey()
    const refused = this.refused(key)
    if (refused) return Promise.resolve(refused)
    if (this.readOnlyNow()) {
      // design §6.1: a window without the replica does not edit while the backend is unreachable
      const detail = aiwsT('aiworkspace.core.session.readOnlyUnsaved')
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
    if (!pending) return Promise.resolve({ status: 'unknown', idempotency_key: idempotencyKey, detail: aiwsT('aiworkspace.core.session.nothingToResend') })
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
    return { status: 'unknown', idempotency_key: key, detail: lastError || aiwsT('aiworkspace.core.session.unreachable') }
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
    if (file.size > 32 * 1024 * 1024) throw new Error('Workspace assets are limited to 32 MiB.')
    const source = file instanceof File ? file : new File([file], fileName ?? 'asset', { type: file.type })
    return uploadWorkspaceAsset(this.client, this.workspaceId, source)
  }

  async importAssets(files: File[], folder: boolean, signal?: AbortSignal): Promise<ImportedAsset[]> {
    if (files.some(file => file.size > 32 * 1024 * 1024)) throw new Error('Workspace assets are limited to 32 MiB per file.')
    const options = await this.client.transport.importOptions()
    const snapshot = await ndm.importFiles(files, { ...options, mode: folder ? 'single_dir' : 'multi_file', autoStartUpload: false, signal })
    try {
      await ndm.ensureUploaded(snapshot.sessionId, { ...options, signal })
      const stage = async (item: ndm.ImportedFileObject | ndm.ImportedDirObject): Promise<ImportedAsset> => {
        signal?.throwIfAborted()
        if (item.kind === 'dir') {
          const children: ImportedAsset[] = []
          for (const child of item.children ?? []) children.push(await stage(child))
          return { kind: 'dir', name: item.name, children }
        }
        const asset = unwrap(await this.client.assetImportObject(this.ws, { obj_id: item.objectId }))
        return { kind: 'file', name: item.name, object_id: asset.object_id, size: asset.size }
      }
      const items: ImportedAsset[] = []
      for (const item of snapshot.items) items.push(await stage(item))
      return items
    } finally { ndm.releaseImportSession(snapshot.sessionId) }
  }

  fetchAsset(objectId: string) {
    return this.client.transport.download(`asset/${this.workspaceId}/${objectId}`)
  }

  call<T>(method: string, params: Record<string, unknown>): Promise<T> {
    return this.client.transport.call<T>(method, params)
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
  async submissionStatus(epoch: string, idempotencyKey: string) { try { return unwrap(await this.client.getSubmission(this.ws, epoch, idempotencyKey)) } catch (error) { return this.fail(error) } }
  async procClaim(runId: string, claimToken: string) { return unwrap(await this.client.procClaim(this.ws, runId, claimToken, { kind: 'browser', session_id: this.sessionId })) }
  async procHeartbeat(runId: string, attempt: number, progress?: Json) { unwrap(await this.client.procHeartbeat(this.ws, runId, attempt, progress)) }
  async procReport(runId: string, attempt: number, body: Record<string, Json>) { return unwrap(await this.client.procReport(this.ws, runId, attempt, body)) }
  async procTool(runId: string, attempt: number, name: string, args: Json) { return unwrap(await this.client.procTool(this.ws, runId, attempt, name, args)) }
  async procProgram(runId: string, attempt: number, body: { source: string; results: Json | null; log: string; files: Json }) { return unwrap(await this.client.procProgram(this.ws, runId, attempt, body)) }
  async procFiles(runId: string, attempt: number) { return unwrap(await this.client.procFiles(this.ws, runId, attempt)).files }

  close() {
    this.closed = true
    this.waitAbort?.abort()
    this.statusListeners.clear()
    this.changeListeners.clear()
    this.seqWaiters.splice(0).forEach((waiter) => waiter.resolve())
  }

  whenClosed() { return Promise.resolve() }
}

/** The D-4 denial behind a failure (a business refusal or a refused session), its cause included. */
export function denialOf(error: unknown): AccessDenial | null {
  const cause = error instanceof Error && error.cause ? error.cause : null
  for (const e of [error, cause]) {
    if ((e instanceof ServiceFailure || e instanceof TransportError) && e.denial) return e.denial
  }
  return null
}

/** What a denial means to the person who met it (contract §4.2): the category for everyone; who can change it and
 * where only for a caller who is not a visitor (a visitor — guest, Limited account, External DID — is told whom to
 * contact, never where the setting lives). */
export function describeDenial(denial: AccessDenial, visitor: boolean): string {
  const text = aiwsT(DENIAL_TEXT[denial.reason])
  const where = denial.adjust.location ? ADJUST_TEXT[denial.adjust.location] : undefined
  return visitor || !where ? text : aiwsT('aiworkspace.core.denial.withAdjust', { text, where: aiwsT(where) })
}

const DENIAL_TEXT: Readonly<Record<DenyReason, string>> = {
  session_kind_not_allowed: 'aiworkspace.core.denial.sessionKindNotAllowed',
  session_ceiling_exceeded: 'aiworkspace.core.denial.sessionCeilingExceeded',
  resource_grant_missing: 'aiworkspace.core.denial.resourceGrantMissing',
  entry_unavailable: 'aiworkspace.core.denial.entryUnavailable',
  zone_method_disabled: 'aiworkspace.core.denial.zoneMethodDisabled',
  account_state: 'aiworkspace.core.denial.accountState',
  credential_state: 'aiworkspace.core.denial.credentialState',
  source_revoked: 'aiworkspace.core.denial.sourceRevoked',
  authorization_unavailable: 'aiworkspace.core.denial.authorizationUnavailable',
}

/** Where a manager changes the outcome (`reauth` / `credential` are the visitor's own steps). */
const ADJUST_TEXT: Readonly<Partial<Record<AdjustLocation, string>>> = {
  share_panel: 'aiworkspace.core.denial.adjustSharePanel',
  'cp:sso_settings': 'aiworkspace.core.denial.adjustSsoSettings',
  'cp:users': 'aiworkspace.core.denial.adjustUsers',
  'cp:exclusive_invites': 'aiworkspace.core.denial.adjustExclusiveInvites',
}

/** A wish run refused under the AI quota or resource policy (§7.4), by its sub-code: the run's own payer
 * learns it is their quota; anyone else only that the block is limited. `undefined` for other failures. */
export function aiRefusalText(subCode: string | undefined, data: Record<string, unknown> | undefined): string | undefined {
  const aicc = (data?.aicc ?? {}) as { user_id?: string; operator_id?: string; mode?: string }
  const own = aicc.user_id !== undefined && aicc.user_id === aicc.operator_id
  switch (subCode) {
    case 'AI_QUOTA_EXHAUSTED': return aiwsT(own ? 'aiworkspace.wish.aicc.quotaOwn' : 'aiworkspace.wish.aicc.quotaOther')
    case 'AI_QUOTA_BUDGET_EXCEEDED': return aiwsT(own ? 'aiworkspace.wish.aicc.budgetOwn' : 'aiworkspace.wish.aicc.quotaOther')
    case 'AI_RESOURCE_NOT_PERMITTED': return aiwsT(own ? 'aiworkspace.wish.aicc.notPermittedOwn' : 'aiworkspace.wish.aicc.notPermittedOther')
    case 'AI_ATTRIBUTION_UNAVAILABLE': return aiwsT(aicc.mode === 'current_user' ? 'aiworkspace.wish.aicc.noOwnResources' : 'aiworkspace.wish.aicc.noAttribution')
    default: return undefined
  }
}

/** The reserved codes of 内核架构改进 D09 as the user reads them; `undefined` for every other code (its detail is shown). */
export function errorCodeText(code: string | undefined, data: Record<string, unknown> | undefined): string | undefined {
  const bytes = (k: string) => Number(data?.[k] ?? 0)
  switch (code) {
    case 'QUOTA_EXCEEDED': return aiwsT(bytes('reclaimable_bytes') >= bytes('needed_bytes') ? 'aiworkspace.core.error.quotaExceededReclaimable' : 'aiworkspace.core.error.quotaExceeded', { needed: formatBytes(bytes('needed_bytes')), reclaimable: formatBytes(bytes('reclaimable_bytes')) })
    case 'QUOTA_LIMIT': return aiwsT('aiworkspace.core.error.quotaLimit')
    case 'VERSION_NOT_RETAINED': return aiwsT('aiworkspace.core.error.versionNotRetained')
    case 'RESOURCE_RECLAIMED': return aiwsT('aiworkspace.core.error.resourceReclaimed')
    case 'EXECUTION_CONTEXT_MISSING': return aiwsT('aiworkspace.core.error.executionContextMissing')
    case 'DELEGATION_STALE': return aiwsT('aiworkspace.core.error.delegationStale')
    case 'RUN_REVOKED': return aiwsT('aiworkspace.core.error.runRevoked')
    default: return undefined
  }
}

function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n <= 0) return '0 B'
  const units = ['B', 'KiB', 'MiB', 'GiB']
  let i = 0
  let v = n
  while (v >= 1024 && i < units.length - 1) { v /= 1024; i += 1 }
  return `${i === 0 ? v : v.toFixed(1)} ${units[i]}`
}

/** The failure of a run as shown to its viewer. */
export function runErrorText(error: (ServiceError & { data?: unknown }) | null | undefined): string | undefined {
  if (!error) return undefined
  const data = error.data as Record<string, unknown> | undefined
  return aiRefusalText(error.sub_code, data) ?? errorCodeText(error.code, data) ?? error.detail
}

export function describeError(error: unknown): string {
  if (error instanceof ServiceFailure) return aiRefusalText(error.subCode, error.data) ?? errorCodeText(error.code, error.data) ?? error.message
  if (error instanceof TransportError) return aiwsT(browserOffline() ? 'aiworkspace.core.session.cannotConnectOffline' : 'aiworkspace.core.session.cannotConnect', { error: error.message })
  return error instanceof Error ? error.message : String(error)
}
