/* Messages between the page and the Replica Worker (design §6.1). The Worker owns the replica
 * database (SQLite WASM on the `opfs-sahpool` VFS) and the engine (the WASM `Replica`); the page
 * owns the network. Everything here is structured-clone friendly. */

import type { AnnotationContent, CommitEvent, CommitRequest, EntityEnvelope, Json, ListAnnotationsParams, QueryPage, QueryParams, Selector, WorkspaceInfo } from '../api/types'

export const REPLICA_LOCK_PREFIX = 'aiworkspace-replica:'
/** Version of the client tables this build writes (`replica_meta.client_schema`). */
export const CLIENT_SCHEMA = 1
/** Assets larger than this are not pulled into the offline asset area (they are listed as skipped). */
export const ASSET_SIZE_CAP = 8 * 1024 * 1024

export type PendingState = 'queued' | 'sending' | 'unknown' | 'conflict' | 'rejected' | 'blocked_asset'

/** What the UI needs to show a pending submission again after the tab was closed. */
export interface PendingMeta {
  editId?: string
  label?: string
  mine?: Json
  hasMine?: boolean
  undoable?: boolean
  /** `richtext`: an editor update of this entity (handled by its RichTextCollab). */
  kind?: 'richtext'
  entity_id?: string
}

export interface PendingRow {
  idempotency_key: string
  request: CommitRequest
  state: PendingState
  result: Json | null
  /** Part of the working view right now. */
  applied: boolean
  meta: PendingMeta | null
  created_at: string
}

export interface ReplicaMeta {
  workspace_id: string
  principal: string
  epoch: string
  confirmed_seq: number
  prepared_at: string
  info: WorkspaceInfo
  client_schema: number
}

export type InitResult =
  | { ok: true; prepared: ReplicaMeta | null; pendingCount: number; sqlite: string; vfs: 'opfs-sahpool' }
  | { ok: false; reason: string }

export interface LocalSaved { status: 'saved_locally'; idempotency_key: string; provisional_seq: number; pending: PendingRow[] }
/** The same conflict / rejected result the backend would give, produced by the same planner. */
export interface LocalRefused { status: 'conflict' | 'rejected'; code: string; [key: string]: unknown }

export interface AssetRow { object_id: string; media_type: string; size: number }

export interface RichTextDraftRow { draft_id: string; entity_id: string; kind: string; base: Json; content: Json; updated_at: string }

export interface LocalExport {
  format: 'buckyos.aiworkspace.local-pending/1'
  exported_at: string
  meta: ReplicaMeta
  pending: PendingRow[]
  drafts: RichTextDraftRow[]
}

export type ReadEntry = (EntityEnvelope & { content: unknown; head_seq: number }) | { error: { code: string; detail?: string } }

/** One method per Worker operation. Every mutation resolves only after its SQLite transaction committed. */
export interface ReplicaApi {
  init(options: { workspaceId: string; testHooks: boolean }): InitResult
  /** Replace the confirmed layer with a freshly downloaded replica database. Refused while pending rows exist unless `discardLocal`. */
  importReplica(args: { bytes: ArrayBuffer; principal: string; info: WorkspaceInfo; epoch: string; headSeq: number; preparedAt: string; discardLocal: boolean }): ReplicaMeta
  /** Rows → engine → restore pending. */
  open(): { meta: ReplicaMeta; pending: PendingRow[] }
  outline(): EntityEnvelope[]
  read(entityId: string, selector: Selector | null): EntityEnvelope & { content: unknown; head_seq: number }
  readMany(targets: { entity_id: string; selector?: Selector }[]): ReadEntry[]
  listAnnotations(params: ListAnnotationsParams): (EntityEnvelope & { content: AnnotationContent; head_seq: number })[]
  query(params: QueryParams): QueryPage
  collab(entityId: string): { lineage_id: string; snapshot: string; pending_updates: string[] }
  submit(request: CommitRequest, meta: PendingMeta | null): LocalSaved | LocalRefused
  nextToSend(): CommitRequest | null
  mark(key: string, state: PendingState, result: Json | null): PendingRow[]
  /** `unsentOnly`: refuse to remove a submission the backend may already have (`sending` / `unknown`). */
  discard(key: string, unsentOnly: boolean): { removed: PendingRow | null; pending: PendingRow[] }
  applyRemote(events: CommitEvent[]): { confirmed_seq: number; settled: string[]; pending: PendingRow[] }
  pending(): PendingRow[]
  updateInfo(info: WorkspaceInfo): null
  saveDraft(draft: RichTextDraftRow): null
  exportLocal(): LocalExport
  listAssets(): AssetRow[]
  putAsset(objectId: string, bytes: ArrayBuffer): null
  getAsset(objectId: string): ArrayBuffer | null
  /** Remove the replica database and the asset area of this Workspace. */
  destroy(): null
  close(): null
  /** e2e only (refused unless `testHooks`): make the next `count` storage transactions fail. */
  testFailTransactions(kind: 'quota' | 'error', count: number): null
}

export type ReplicaOp = keyof ReplicaApi

export interface WorkerRequest { id: number; op: ReplicaOp; args: unknown[] }

export interface WorkerFailure {
  /** `storage`: the replica database could not be written (quota, transaction error). `service`: a structured engine error. */
  kind: 'storage' | 'service' | 'internal'
  message: string
  storageKind?: 'quota' | 'transaction'
  service?: { code: string; detail?: string; sub_code?: string; data?: Record<string, unknown>; retryable?: boolean }
}

export type WorkerResponse = { id: number; ok: true; value: unknown } | { id: number; ok: false; error: WorkerFailure }
