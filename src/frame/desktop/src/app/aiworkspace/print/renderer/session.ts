/* The Workspace session of a print (打印 TODO T7, Q20): the package's rows (the caller's readable rows of a fixed head)
 * in the WASM replica, as the session replay does (session/replay.ts). Reads come from the replica, the assets from the
 * package; nothing is written, nothing runs, no request leaves the page. */

import type { PrintPackage } from '@print/package.ts'
import { ServiceFailure } from '../../api/client'
import type { ReadOk, SessionMode, SessionStatus, WorkspaceSession } from '../../api/session'
import type { CollabState, CommitOutcome, EntityEnvelope, QueryPage, QueryParams, RelationsInfo, Selector, WorkspaceInfo } from '../../api/types'
import type { AiwsCore } from '../../api/wasm'
import { aiwsT } from '../../i18n'
import { printAssetUrl } from './assets'

type Replica = InstanceType<AiwsCore['Replica']>

// one object each: React reads them as external store snapshots
const LIVE: SessionStatus = { kind: 'live' }
const DIRECT: SessionMode = { kind: 'direct', reason: 'not_prepared', detail: '' }

function engine<T>(run: () => T): T {
  try {
    return run()
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    let parsed: { code?: string; detail?: string } | null = null
    try { parsed = JSON.parse(message) as { code?: string; detail?: string } } catch { /* not the engine's error object */ }
    if (parsed && typeof parsed.code === 'string') throw new ServiceFailure({ code: parsed.code, detail: parsed.detail })
    throw error instanceof Error ? error : new Error(message)
  }
}

function base64(bytes: Uint8Array): string {
  let text = ''
  for (let i = 0; i < bytes.length; i += 0x8000) text += String.fromCharCode(...bytes.subarray(i, i + 0x8000))
  return btoa(text)
}

export class PrintSession implements WorkspaceSession {
  readonly workspaceId: string
  readonly sessionId = 'print'
  readonly principal: string
  readonly offline = null
  private readonly replica: Replica
  private readonly pkg: PrintPackage
  private readonly wsInfo: WorkspaceInfo

  constructor(core: AiwsCore, pkg: PrintPackage) {
    this.pkg = pkg
    this.workspaceId = pkg.workspace_id
    this.principal = pkg.principal
    this.replica = engine(() => new core.Replica(pkg.principal, pkg.workspace_id, pkg.epoch, JSON.stringify(pkg.rows), pkg.head_seq, 1))
    this.wsInfo = {
      workspace_id: pkg.workspace_id, title: pkg.title, epoch: pkg.epoch, head_seq: pkg.head_seq, capabilities: ['read'],
      format_version: '', protocol_version: '', head_commit_id: null, cloned_from: null, principal: pkg.principal,
    }
  }

  private refuse(): Promise<never> {
    return Promise.reject(new ServiceFailure({ code: 'PERMISSION_DENIED', detail: aiwsT('aiworkspace.printrender.readOnly') }))
  }
  private refused(): Promise<CommitOutcome> {
    return Promise.resolve({ status: 'rejected', code: 'PERMISSION_DENIED', detail: aiwsT('aiworkspace.printrender.readOnly'), idempotency_key: '' } as unknown as CommitOutcome)
  }

  mode(): SessionMode { return DIRECT }
  info() { return this.wsInfo }
  status(): SessionStatus { return LIVE }
  appliedSeq() { return this.pkg.head_seq }
  showLock() { return null }
  noteShowLock() { /* a print never meets a show */ }
  subscribeStatus() { return () => undefined }
  subscribeChanges() { return () => undefined }
  async whenApplied() { /* the head is fixed */ }
  async serverTime() { return null }
  async assetStreamUrl(objectId: string) { return printAssetUrl(objectId) }

  async outline(): Promise<EntityEnvelope[]> {
    return (JSON.parse(engine(() => this.replica.outline())) as { entities: EntityEnvelope[] }).entities
  }

  private readNow<C>(entityId: string, selector?: Selector): ReadOk<C> {
    const r = JSON.parse(engine(() => this.replica.read(entityId, selector ? JSON.stringify(selector) : undefined))) as ReadOk<C>
    r.head_seq = this.pkg.head_seq
    if (r.type_id === 'buckyos.asset-ref' && r.content && typeof r.content === 'object') {
      // the bytes a page shows are in the package (a video or a sound: its poster only)
      const payload = (r.content as { payload?: { object_id?: string; poster?: string } }).payload
      const packaged = Boolean(printAssetUrl(payload?.object_id) ?? printAssetUrl(payload?.poster))
      r.content = { ...(r.content as Record<string, unknown>), availability: packaged ? 'available' : 'missing' } as C
    }
    return r
  }

  async read<C>(entityId: string, selector?: Selector) { return this.readNow<C>(entityId, selector) }
  async readMany(targets: { entity_id: string; selector?: Selector }[]) {
    return targets.map((t) => {
      try { return this.readNow<unknown>(t.entity_id, t.selector) } catch (error) { return { error: { code: error instanceof ServiceFailure ? error.code : 'NOT_FOUND', detail: String(error) } } }
    })
  }
  /** Comments and annotations are not printed (Q7). */
  async listAnnotations() { return [] }
  async query(params: QueryParams): Promise<QueryPage> {
    return JSON.parse(engine(() => this.replica.query(JSON.stringify(params)))) as QueryPage
  }
  sql() { return this.refuse() }
  async getCollabState(entityId: string): Promise<CollabState> {
    const lineage = this.replica.lineage_id(entityId)
    if (!lineage) throw new ServiceFailure({ code: 'NOT_FOUND', detail: entityId })
    return { entity_id: entityId, lineage_id: lineage, engine: 'loro', engine_version: '', encoding: 'snapshot', snapshot: base64(engine(() => this.replica.collab_snapshot(entityId))), seq: this.pkg.head_seq, content_rev: 0 }
  }
  async freshness() { return [] }
  async relations(entityId: string): Promise<RelationsInfo> {
    return { entity_id: entityId, outgoing: [], incoming: [], hidden_incoming: 0, shown_in: [], produced: [] } as unknown as RelationsInfo
  }
  listVersions() { return this.refuse() }
  restoreVersionPlan() { return this.refuse() }

  /** Nobody's personal state is printed (folds, table presentation: §3.2). */
  async getUserState() { return {} }
  async setUserState() { /* nothing is kept */ }

  subscribeAcl() { return () => undefined }
  async listSubjects() { return [] }
  publish() { return this.refuse() }
  unpublish() { return this.refuse() }
  async listPublications() { return [] }
  async listReferrers() { return [] }
  extBuild() { return this.refuse() }
  extExport() { return this.refuse() }
  extImport() { return this.refuse() }
  extInstallFrom() { return this.refuse() }
  extUpgrade() { return this.refuse() }
  extLibraryBundle() { return this.refuse() }
  extExportTemplate() { return this.refuse() }
  async extListLibrary() { return [] }
  extPack() { return this.refuse() }
  downloadPack() { return this.refuse() }
  installPack() { return this.refuse() }
  installPikgFile() { return this.refuse() }

  commit() { return this.refused() }
  retryUnknown() { return this.refused() }
  async submissionStatus() { return { status: 'not_found' as const } }
  prepare() { return this.refuse() }
  undoCommit() { return this.refused() }

  lockAcquire() { return this.refuse() }
  async lockRenew() { /* nothing is held */ }
  async lockRelease() { /* nothing is held */ }
  lockBreak() { return this.refuse() }
  async lockList() { return [] }

  call() { return this.refuse() }
  uploadAsset() { return this.refuse() }
  uploadTransient() { return this.refuse() }
  contentServer() { return this.refuse() }
  importAssets() { return this.refuse() }
  /** The bytes of an asset are not readable here (file:// fetches nothing): pages show assets by their address. */
  fetchAsset(): Promise<never> {
    return Promise.reject(new ServiceFailure({ code: 'NOT_FOUND', detail: aiwsT('aiworkspace.printrender.assetBytes') }))
  }

  procStart() { return this.refuse() }
  procGet() { return this.refuse() }
  procCancel() { return this.refuse() }
  wishStart() { return this.refuse() }
  wishGet() { return this.refuse() }
  async wishList() { return [] }
  procRollback() { return this.refuse() }
  procClaim() { return this.refuse() }
  procHeartbeat() { return this.refuse() }
  procReport() { return this.refuse() }
  procTool() { return this.refuse() }
  procProgram() { return this.refuse() }
  procFiles() { return this.refuse() }
  importObject() { return this.refuse() }

  presenceHeartbeat() { return this.refuse() }
  async presenceLeave() { /* never present */ }
  presenceList() { return this.refuse() }
  workLog() { return this.refuse() }

  close() { this.replica.free() }
  async whenClosed() { /* nothing held */ }
}
