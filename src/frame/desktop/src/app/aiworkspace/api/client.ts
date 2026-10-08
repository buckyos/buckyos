/* Typed client for the kRPC methods of src/frame/aiworkspace/README.md. One method per service
 * method, no behaviour of its own: retries, idempotency and the change stream live in session.ts. */

import type {
  AnnotationRead, ChangesPage, CollabState, CommitRequest, CommitResult, EntityEnvelope, ExportResult, GrantList, ListAnnotationsParams, LockInfo, PrepareResult,
  QueryPage, QueryParams, ReadResult, Result, RunView, Selector, WorkspaceInfo, WorkspaceSummary, Capability, Json,
  DerivedRecord, FreshnessInfo, Operation, RelationsInfo, Subject, VersionInfo, WishChoices, WishRunView,
  ShowCommand, ShowLockInfo, ShowNotes, ShowStart, ShowState, ShowWatch,
} from './types'
import type { Transport } from './transport'

type Ws = { workspace_id: string }

export class AiwsClient {
  readonly transport: Transport
  constructor(transport: Transport) {
    this.transport = transport
  }

  private call<T>(method: string, params: Record<string, unknown>, signal?: AbortSignal): Promise<T> {
    return this.transport.call<T>(method, params, signal)
  }

  // ---- workspaces
  wsCreate(title: string) { return this.call<Result<WorkspaceSummary>>('ws.create', { title }) }
  wsList() { return this.call<Result<{ workspaces: WorkspaceSummary[] }>>('ws.list', {}) }
  wsGetInfo(ws: Ws) { return this.call<Result<WorkspaceInfo>>('ws.get_info', ws) }
  wsDelete(ws: Ws) { return this.call<Result<object>>('ws.delete', ws) }
  wsFork(ws: Ws) { return this.call<Result<{ workspace_id: string; epoch: string; content_root: string }>>('ws.fork', ws) }
  wsGrant(ws: Ws, subject: string, capabilities: Capability[], scope_entity_id?: string) {
    return this.call<Result<object>>('ws.grant', { ...ws, subject, capabilities, ...(scope_entity_id ? { scope_entity_id } : {}) })
  }
  wsRevoke(ws: Ws, subject: string, scope_entity_id?: string) {
    return this.call<Result<object>>('ws.revoke', { ...ws, subject, ...(scope_entity_id ? { scope_entity_id } : {}) })
  }
  wsListGrants(ws: Ws) { return this.call<Result<GrantList>>('ws.list_grants', ws) }
  wsListSubjects(ws: Ws) { return this.call<Result<{ subjects: Subject[] }>>('ws.list_subjects', ws) }
  wsGetUserState(ws: Ws) { return this.call<Result<{ entries: Record<string, Json>; updated_at: Record<string, string> }>>('ws.get_user_state', ws) }
  wsSetUserState(ws: Ws, entries: Record<string, Json | null>) { return this.call<Result<{ updated_at: string }>>('ws.set_user_state', { ...ws, entries }) }
  wsBeginImport() { return this.call<Result<{ upload_id: string }>>('ws.begin_import', {}) }
  wsImport(upload_id: string, semantics: 'restore' | 'new', replace: boolean) {
    return this.call<Result<{ workspace_id: string; epoch: string; content_root?: string }>>('ws.import', { upload_id, semantics, ...(replace ? { replace: true } : {}) })
  }

  // ---- reads
  outline(ws: Ws) { return this.call<Result<{ entities: EntityEnvelope[] }>>('doc.outline', ws) }
  read<C>(ws: Ws, entity_id: string, selector?: Selector) {
    return this.call<ReadResult<C>>('doc.read', { ...ws, entity_id, ...(selector ? { selector } : {}) })
  }
  readMany(ws: Ws, targets: { entity_id: string; selector?: Selector }[]) {
    return this.call<Result<{ results: ReadResult[] }>>('doc.read', { ...ws, targets })
  }
  listAnnotations(ws: Ws, params: ListAnnotationsParams) { return this.call<Result<{ annotations: AnnotationRead[] }>>('doc.list_annotations', { ...ws, ...params }) }
  listChildren(ws: Ws, entity_id: string) { return this.call<Result<{ children: EntityEnvelope[] }>>('doc.list_children', { ...ws, entity_id }) }
  query(ws: Ws, params: QueryParams) { return this.call<Result<QueryPage>>('doc.query', { ...ws, ...params }) }
  getCollabState(ws: Ws, entity_id: string) { return this.call<Result<CollabState>>('doc.get_collab_state', { ...ws, entity_id }) }
  freshness(ws: Ws, entity_ids: string[]) { return this.call<Result<{ items: FreshnessInfo[]; head_seq: number }>>('doc.freshness', { ...ws, entity_ids }) }
  relations(ws: Ws, entity_id: string) { return this.call<Result<RelationsInfo>>('doc.relations', { ...ws, entity_id }) }
  listVersions(ws: Ws, entity_id: string) { return this.call<Result<{ versions: VersionInfo[]; content_rev: number }>>('doc.list_versions', { ...ws, entity_id }) }
  restoreVersion(ws: Ws, entity_id: string, content_rev: number) { return this.call<Result<{ operations: Operation[]; derived: DerivedRecord | null }>>('doc.restore_version', { ...ws, entity_id, content_rev }) }

  // ---- writes
  commit(request: CommitRequest) { return this.call<CommitResult>('doc.commit', request as unknown as Record<string, unknown>) }
  prepare(request: CommitRequest) { return this.call<PrepareResult>('doc.prepare', request as unknown as Record<string, unknown>) }
  getSubmission(ws: Ws, epoch: string, idempotency_key: string) {
    return this.call<Result<{ status: 'accepted' | 'not_found'; result?: CommitResult }>>('doc.get_submission', { ...ws, epoch, idempotency_key })
  }
  undo(ws: Ws, params: { epoch: string; commit_id: string; idempotency_key: string; session_id: string; mode?: 'all_or_nothing' | 'partial'; plan_digest?: string }) {
    return this.call<CommitResult>('doc.undo', { ...ws, ...params })
  }

  // ---- change stream
  getChanges(ws: Ws, epoch: string, after_seq: number, limit = 200) {
    return this.call<Result<ChangesPage>>('doc.get_changes', { ...ws, epoch, after_seq, limit })
  }
  waitChanges(ws: Ws, epoch: string, after_seq: number, timeout_ms: number, signal?: AbortSignal) {
    return this.call<Result<{ epoch: string; head_seq: number; timed_out: boolean; show_lock?: ShowLockInfo | null }>>('doc.wait_changes', { ...ws, epoch, after_seq, timeout_ms }, signal)
  }

  // ---- packages
  checkpoint(ws: Ws) { return this.call<Result<{ content_root: string; snapshot: string }>>('doc.checkpoint', ws) }
  export(ws: Ws, mode: 'share' | 'personal_backup', self_contained: boolean, include_notes?: boolean) {
    return this.call<Result<ExportResult>>('doc.export', { ...ws, mode, self_contained, ...(include_notes === undefined ? {} : { include_notes }) })
  }

  // ---- shows (第三期规划 §11.3): the stage's calls; a prompter uses `prompterCall` with the show's token
  showStart(ws: Ws, path_id: string, live: boolean) { return this.call<Result<ShowStart>>('show.start', { ...ws, path_id, live }) }
  showHeartbeat(ws: Ws, show_id: string) { return this.call<Result<{ expires_at: string }>>('show.heartbeat', { ...ws, show_id }) }
  showPublish(ws: Ws, show_id: string, seq: number, state: ShowState) { return this.call<Result<{ seq: number }>>('show.publish', { ...ws, show_id, seq, state }) }
  showCommand(ws: Ws, show_id: string, command: ShowCommand) { return this.call<Result<{ cursor: number; duplicate: boolean }>>('show.command', { ...ws, show_id, command }) }
  showWatch(ws: Ws, show_id: string, params: { after_seq?: number; after_command?: number; timeout_ms?: number }, signal?: AbortSignal) {
    return this.call<Result<ShowWatch>>('show.watch', { ...ws, show_id, ...params }, signal)
  }
  showNotes(ws: Ws, show_id: string) { return this.call<Result<ShowNotes>>('show.notes', { ...ws, show_id }) }
  showEnd(ws: Ws, show_id: string) { return this.call<Result<object>>('show.end', { ...ws, show_id }) }

  // ---- offline replica (design §6.2)
  replicaBootstrap(ws: Ws) {
    return this.call<Result<{ replica_id: string; epoch: string; head_seq: number; principal: string; prepared_at: string }>>('replica.bootstrap', ws)
  }

  // ---- assets
  assetBeginUpload(ws: Ws, size: number, file_name?: string) {
    return this.call<Result<{ upload_id: string }>>('asset.begin_upload', { ...ws, size, ...(file_name ? { file_name } : {}) })
  }
  assetFinishUpload(ws: Ws, upload_id: string) {
    return this.call<Result<{ object_id: string; media_type: string; size: number }>>('asset.finish_upload', { ...ws, upload_id })
  }

  // ---- controlled processing (Mock)
  procStart(ws: Ws, program: string, params: Record<string, Json>, idempotency_key: string) {
    return this.call<Result<RunView>>('proc.start', { ...ws, program, params, idempotency_key })
  }
  procGet(ws: Ws, run_id: string) { return this.call<Result<RunView>>('proc.get', { ...ws, run_id }) }
  procApply(ws: Ws, run_id: string, session_id: string) { return this.call<Result<RunView>>('proc.apply', { ...ws, run_id, session_id }) }
  procCancel(ws: Ws, run_id: string) { return this.call<Result<{ state: string; already_applied: boolean; commit_id?: string }>>('proc.cancel', { ...ws, run_id }) }

  // ---- wish runs (wish.xllm@1 / wish.mock@1, 许愿格 §13.1)
  wishStart(ws: Ws, program: string, params: Record<string, Json>, idempotency_key: string) {
    return this.call<Result<WishRunView>>('proc.start', { ...ws, program, params, idempotency_key })
  }
  wishGet(ws: Ws, run_id: string, choices?: WishChoices) {
    return this.call<Result<WishRunView>>('proc.get', { ...ws, run_id, ...(choices ? { choices: choices as unknown as Json } : {}) })
  }
  wishApply(ws: Ws, run_id: string, plan_digest: string, session_id: string) {
    return this.call<Result<WishRunView>>('proc.apply', { ...ws, run_id, plan_digest, session_id })
  }
  wishList(ws: Ws, wish_id: string, limit: number) { return this.call<Result<{ runs: WishRunView[] }>>('proc.list', { ...ws, wish_id, limit }) }

  // ---- write locks
  lockAcquire(ws: Ws, entity_ids: string[], session_id: string) {
    return this.call<Result<{ locks: { entity_id: string; lock_id: string; expires_at: string }[] }>>('lock.acquire', { ...ws, entity_ids, session_id })
  }
  lockRenew(ws: Ws, lock_ids: string[]) {
    return this.call<Result<{ locks: { entity_id: string; lock_id: string; expires_at: string }[] }>>('lock.renew', { ...ws, lock_ids })
  }
  lockRelease(ws: Ws, lock_ids: string[]) { return this.call<Result<object>>('lock.release', { ...ws, lock_ids }) }
  lockBreak(ws: Ws, entity_id: string) { return this.call<Result<object>>('lock.break', { ...ws, entity_id }) }
  lockList(ws: Ws) { return this.call<Result<{ locks: LockInfo[] }>>('lock.list', ws) }
}

/** Unwrap `{ ok: true, ... }`; a business error becomes a thrown ServiceFailure the UI can display. */
export class ServiceFailure extends Error {
  readonly code: string
  readonly subCode?: string
  readonly data?: Record<string, unknown>
  readonly retryable: boolean
  constructor(error: { code: string; detail?: string; sub_code?: string; data?: Record<string, unknown>; retryable?: boolean }) {
    super(`${error.code}${error.sub_code ? `/${error.sub_code}` : ''}${error.detail ? `: ${error.detail}` : ''}`)
    this.name = 'ServiceFailure'
    this.code = error.code
    this.subCode = error.sub_code
    this.data = error.data
    this.retryable = error.retryable ?? false
  }
}

export function unwrap<T>(result: Result<T>): { ok: true } & T {
  if (result && result.ok) return result
  throw new ServiceFailure(result?.error ?? { code: 'UNKNOWN', detail: 'malformed response' })
}
