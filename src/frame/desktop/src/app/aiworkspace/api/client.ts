/* Typed client for the kRPC methods of src/frame/aiworkspace/README.md. One method per service
 * method, no behaviour of its own: retries, idempotency and the change stream live in session.ts. */

import type {
  AnnotationRead, BucketBinding, BucketDownload, ChangesPage, ClonedFrom, CollabState, CommitRequest, CommitResult, EntityEnvelope, ExportResult, AclChange, AclEntryList, AclGroup, ListAnnotationsParams, LockInfo, PrepareResult, StagedAsset,
  QueryPage, QueryParams, ReadResult, Result, RunView, SqlParams, SqlResult, Selector, WorkspaceInfo, WorkspaceSummary, Json,
  DerivedRecord, FreshnessInfo, Operation, RelationsInfo, Subject, VersionInfo, WishChoices, WishRunView,
  ShowCommand, ShowLockInfo, ShowNotes, ShowStart, ShowState, ShowWatch, ShareEntry, EntryJoin, WishClaim, WishToolAnswer, WishStageFiles, HostedExecConfig, Publication, Referrer,
  ExtBuildResult, ExtExportResult, ExtMigration, ExtPackResult, LibraryPackage, TemplateSpec,
} from './types'
import { commitWithParts, parseAccessDenial, type AccessDenial } from 'buckyos'
import { testHooks } from './testHooks'
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
  wsClone(ws: Ws) { return this.call<Result<{ workspace_id: string; epoch: string; content_root: string; cloned_from: ClonedFrom; excluded_entities: number }>>('ws.clone', ws) }
  // permissions (README “Permissions”, 权限需求 §4): entries by path, logical groups, anonymous access
  aclSet(ws: Ws, changes: AclChange[]) { return this.call<Result<{ acl_rev: number }>>('acl.set', { ...ws, changes }) }
  aclQueryAll(ws: Ws) { return this.call<Result<AclEntryList>>('acl.query', { ...ws, by: 'all' }) }
  aclGroups(ws: Ws) { return this.call<Result<{ groups: AclGroup[]; complete: boolean; anonymous: boolean }>>('acl.groups', ws) }
  aclSetGroup(ws: Ws, group_id: string, title?: string) { return this.call<Result<object>>('acl.set_group', { ...ws, group_id, ...(title ? { title } : {}) }) }
  aclMembers(ws: Ws, group_id: string, add: string[], remove: string[] = []) { return this.call<Result<object>>('acl.members', { ...ws, group_id, add, remove }) }
  aclSetAnonymous(ws: Ws, enabled: boolean) { return this.call<Result<object>>('acl.set_anonymous', { ...ws, enabled }) }
  wsSetHostedExec(ws: Ws, config: HostedExecConfig) { return this.call<Result<{ hosted_exec: HostedExecConfig }>>('ws.set_hosted_exec', { ...ws, config: config as unknown as Json }) }
  wsListSubjects(ws: Ws) { return this.call<Result<{ subjects: Subject[] }>>('ws.list_subjects', ws) }
  // ---- publication by path (内核架构改进 D26)
  wsPublish(ws: Ws, entity_id: string, name?: string) { return this.call<Result<Publication>>('ws.publish', { ...ws, entity_id, ...(name ? { name } : {}) }) }
  wsUnpublish(ws: Ws, entity_id: string) { return this.call<Result<{ removed: number }>>('ws.unpublish', { ...ws, entity_id }) }
  wsListPublications(ws: Ws) { return this.call<Result<{ publications: Publication[] }>>('ws.list_publications', ws) }
  wsListReferrers(ws: Ws, entity_id: string) { return this.call<Result<{ referrers: Referrer[] }>>('ws.list_referrers', { ...ws, entity_id }) }
  // extension controls (扩展控件 P0, README “Extension controls”): the Workspace's definitions and the caller's library
  extBuild(ws: Ws, params: Record<string, unknown>) { return this.call<Result<ExtBuildResult>>('ext.build', { ...ws, ...params }) }
  extExport(ws: Ws, entity_ids: string[], purpose: 'package' | 'pull' = 'package') { return this.call<Result<ExtExportResult>>('ext.export', { ...ws, entity_ids, purpose }) }
  extImport(ws: Ws, items: { pkg_id: string; ident: string }[]) { return this.call<Result<ExtBuildResult>>('ext.import', { ...ws, items }) }
  extInstallFrom(ws: Ws, entity_ids: string[], pkg: { name: string; version: string; title?: string }, templates?: TemplateSpec[]) {
    return this.call<Result<{ pkg_id: string; version: string; action: string }>>('ext.install', { from: { ...ws, entity_ids, ...(templates?.length ? { templates } : {}) }, package: pkg, mode: 'dev' })
  }
  /** P2 (Q30): rebuild definitions from the version of their package in the caller's library, with a migration. */
  extUpgrade(ws: Ws, items: { def_entity_id: string; pkg_id?: string }[], opts: { migration?: Record<string, ExtMigration>; force?: boolean; dry_run?: boolean }) {
    return this.call<Result<ExtBuildResult>>('ext.upgrade', { ...ws, items, ...opts })
  }
  /** The files of library controls (base64), e.g. to compare a newer version's interface. */
  extLibraryBundle(pkg_id: string, idents?: string[]) {
    return this.call<Result<{ manifest: Json; controls: { ident: string; files: { name: string; size: number; obj_id: string; base64: string }[] }[] }>>('ext.library_bundle', { pkg_id, ...(idents ? { idents } : {}) })
  }
  /** P3 (Q32): a template of chosen Surfaces (its manifest item, the definitions it needs, its files). */
  extExportTemplate(ws: Ws, spec: TemplateSpec) {
    return this.call<Result<{ template: Json; controls: Json[]; files: { path: string; size: number; base64: string }[] }>>('ext.export_template', { ...ws, ...spec })
  }
  /** A new Workspace from a template of the caller's library. */
  extCreateFromTemplate(pkg_id: string, template_id: string, title?: string, params?: Record<string, Json>) {
    return this.call<Result<{ workspace_id: string; title: string; epoch: string }>>('ext.create_from_template', { pkg_id, template_id, ...(title ? { title } : {}), ...(params ? { params } : {}) })
  }
  extListLibrary() { return this.call<Result<{ packages: LibraryPackage[] }>>('ext.list_library', {}) }
  /** P1 (Q20): the service packs the definitions (and what they build on) as an `aiws` .pikg in the caller's `exports` bucket. */
  extPack(ws: Ws, entity_ids: string[], pkg: { name: string; version: string; title?: string }, templates?: TemplateSpec[]) { return this.call<Result<ExtPackResult>>('ext.pack', { ...ws, entity_ids, package: pkg, ...(templates?.length ? { templates } : {}) }) }
  /** A .pikg bound in the caller's own `exports` bucket into its library (the binding is consumed, Q21). */
  extInstallBound(binding: { bucket_id: string; key: string }) {
    return this.call<Result<{ pkg_id: string; version: string; action: string; controls: Json[] }>>('ext.install', { pikg: binding, mode: 'dev' })
  }
  // share entries (hybrid SSO contract §8.3, 权限需求 R8): links that offer sign-in methods into a logical group
  wsCreateEntry(ws: Ws, params: { group_id: string; methods: ShareEntry['methods']; expires_at?: number; max_joins?: number }) {
    return this.call<Result<{ entry: ShareEntry }>>('ws.create_entry', { ...ws, ...params })
  }
  wsListEntries(ws: Ws) { return this.call<Result<{ entries: ShareEntry[] }>>('ws.list_entries', ws) }
  wsCloseEntry(ws: Ws, entry_id: string, remove_members = false) { return this.call<Result<{ entry: ShareEntry; removed_members: number }>>('ws.close_entry', { ...ws, entry_id, remove_members }) }
  wsJoinEntry(entry_id: string) { return this.call<Result<EntryJoin>>('ws.join_entry', { entry_id }) }
  wsGetUserState(ws: Ws) { return this.call<Result<{ entries: Record<string, Json>; updated_at: Record<string, string>; revisions: Record<string, number> }>>('ws.get_user_state', ws) }
  /** `expect: { key: revision }` makes a write conditional (D13): a moved revision is a REVISION_CONFLICT. */
  wsSetUserState(ws: Ws, entries: Record<string, Json | null>, expect?: Record<string, number>) { return this.call<Result<{ updated_at: string; revisions: Record<string, number> }>>('ws.set_user_state', { ...ws, entries, ...(expect ? { expect } : {}) }) }
  /** The package the caller bound into its own `exports` bucket; a successful import consumes it. */
  wsImport(pkg: BucketBinding, semantics: 'restore' | 'new', replace: boolean) {
    return this.call<Result<{ workspace_id: string; epoch: string; content_root?: string }>>('ws.import', { bucket_id: pkg.bucket_id, key: pkg.key, semantics, ...(replace ? { replace: true } : {}) })
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
  /** `doc.query { sql }`: one SELECT over the logical tables (核心 API 调整 §3.4). */
  sql(ws: Ws, params: SqlParams) { return this.call<Result<SqlResult>>('doc.query', { ...ws, ...params }) }
  /** `doc.source_query`: the rows of a URL table, queried at its source (filter, sorts, fields, paging). */
  sourceQuery(ws: Ws, params: QueryParams) { return this.call<Result<QueryPage>>('doc.source_query', { ...ws, ...params }) }
  getCollabState(ws: Ws, entity_id: string) { return this.call<Result<CollabState>>('doc.get_collab_state', { ...ws, entity_id }) }
  freshness(ws: Ws, entity_ids: string[]) { return this.call<Result<{ items: FreshnessInfo[]; head_seq: number }>>('doc.freshness', { ...ws, entity_ids }) }
  relations(ws: Ws, entity_id: string) { return this.call<Result<RelationsInfo>>('doc.relations', { ...ws, entity_id }) }
  listVersions(ws: Ws, entity_id: string) { return this.call<Result<{ versions: VersionInfo[]; content_rev: number }>>('doc.list_versions', { ...ws, entity_id }) }
  restoreVersion(ws: Ws, entity_id: string, content_rev: number) { return this.call<Result<{ operations: Operation[]; derived: DerivedRecord | null }>>('doc.restore_version', { ...ws, entity_id, content_rev }) }

  // ---- writes
  /** One Commit whatever its size: a large one is sent in parts (`doc.stage`) and assembled by the service. */
  commit(request: CommitRequest, onPart?: (part: number, total: number) => void) {
    return commitWithParts<CommitResult>((method, params) => this.call(method, params), request as unknown as Record<string, unknown>, { onPart, partBytes: testHooks()?.commitPartBytes })
  }
  prepare(request: CommitRequest) {
    return commitWithParts<PrepareResult>((method, params) => this.call(method, params), request as unknown as Record<string, unknown>, { method: 'doc.prepare' })
  }
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
  /** A Range-playable address of an asset (控件机制 TODO G17, H6): media play while they load. */
  assetStreamUrl(ws: Ws, object_id: string) { return this.call<Result<{ url: string }>>('asset.stream_url', { ...ws, object_id }) }
  /** The service's clock (控件机制 TODO G16): unix ms when it answered. */
  wsTime() { return this.call<Result<{ server_ms: number }>>('ws.time', {}) }
  waitChanges(ws: Ws, epoch: string, after_seq: number, timeout_ms: number, signal?: AbortSignal) {
    return this.call<Result<{ epoch: string; head_seq: number; timed_out: boolean; show_lock?: ShowLockInfo | null; server_ms?: number }>>('doc.wait_changes', { ...ws, epoch, after_seq, timeout_ms }, signal)
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
    return this.call<Result<{ replica_id: string; epoch: string; head_seq: number; principal: string; prepared_at: string; download: BucketDownload }>>('replica.bootstrap', ws)
  }

  // ---- assets: uploaded through NDM, then taken over into the Workspace owner's `assets` bucket
  assetImportObject(ws: Ws, source: { obj_id: string } | BucketBinding) {
    return this.call<Result<StagedAsset>>('asset.import_object', { ...ws, ...source })
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

  // ---- the executor protocol of a run outside the service (内核架构改进 T3-2, T3-4)
  procClaim(ws: Ws, run_id: string, claim_token: string, executor: Json) { return this.call<Result<WishClaim>>('proc.claim', { ...ws, run_id, claim_token, executor }) }
  procHeartbeat(ws: Ws, run_id: string, attempt: number, progress?: Json) { return this.call<Result<{ lease_until: string }>>('proc.heartbeat', { ...ws, run_id, attempt, ...(progress ? { progress } : {}) }) }
  procReport(ws: Ws, run_id: string, attempt: number, body: Record<string, Json>) { return this.call<Result<WishRunView>>('proc.report', { ...ws, run_id, attempt, ...body }) }
  procTool(ws: Ws, run_id: string, attempt: number, name: string, args: Json) { return this.call<Result<WishToolAnswer>>('proc.tool', { ...ws, run_id, attempt, name, args }) }
  procProgram(ws: Ws, run_id: string, attempt: number, body: { source: string; results: Json | null; log: string; files: Json }) { return this.call<Result<WishToolAnswer>>('proc.program', { ...ws, run_id, attempt, ...body }) }
  procFiles(ws: Ws, run_id: string, attempt: number) { return this.call<Result<WishStageFiles>>('proc.files', { ...ws, run_id, attempt }) }

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
  /** Why access was refused (D-4), when it was: the reason and who can change it. */
  readonly denial: AccessDenial | null
  constructor(error: { code: string; detail?: string; sub_code?: string; data?: Record<string, unknown>; retryable?: boolean; denial?: unknown }) {
    super(`${error.code}${error.sub_code ? `/${error.sub_code}` : ''}${error.detail ? `: ${error.detail}` : ''}`)
    this.name = 'ServiceFailure'
    this.code = error.code
    this.subCode = error.sub_code
    this.data = error.data
    this.retryable = error.retryable ?? false
    this.denial = parseAccessDenial(error.denial)
  }
}

export function unwrap<T>(result: Result<T>): { ok: true } & T {
  if (result && result.ok) return result
  throw new ServiceFailure(result?.error ?? { code: 'UNKNOWN', detail: 'malformed response' })
}
