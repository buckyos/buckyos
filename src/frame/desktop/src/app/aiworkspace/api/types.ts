/* Wire types of the aiworkspace service (see src/frame/aiworkspace/README.md and the detailed design §2–§5).
 * Only what the Desktop app reads is typed; unknown keys are preserved as `unknown`. */

export const PROTOCOL_VERSION = '0.1'

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json }

export type Capability = 'read' | 'append' | 'update' | 'delete' | 'structure' | 'comment' | 'export' | 'manage'

export interface ServiceError {
  code: string
  retryable?: boolean
  detail?: string
  sub_code?: string
  data?: Record<string, unknown>
}

/** `{ ok: true, ... } | { ok: false, error }` — every non-commit method. */
export type Result<T> = ({ ok: true } & T) | { ok: false; error: ServiceError }

export interface WorkspaceSummary {
  workspace_id: string
  title: string
  epoch: string
  head_seq: number
  capabilities: Capability[]
}

export interface WorkspaceInfo extends WorkspaceSummary {
  format_version: string
  protocol_version: string
  head_commit_id: string | null
  forked_from: unknown
}

export type TypeId =
  | 'buckyos.container'
  | 'buckyos.record'
  | 'buckyos.richtext'
  | 'buckyos.table-source'
  | 'buckyos.cell'
  | 'buckyos.asset-ref'
  | 'buckyos.annotation'

export interface LockHolder {
  principal: string
  session_id?: string
  acquired_at?: string
  expires_at?: string
  lock_id?: string
}

export interface EntityEnvelope {
  entity_id: string
  type_id: TypeId | string
  schema_version: number
  name: string | null
  title?: string | null
  kind?: string
  parent_id?: string
  order_key?: string
  scope: string
  deleted: boolean
  content_rev: number
  meta_rev: number
  life_rev: number
  struct_rev?: number
  write_policy: 'open' | 'lock_required'
  lock_holder?: LockHolder | null
  capabilities: Capability[]
  degraded?: string
}

export interface Selector {
  kind: string
  record_id?: string
  field_id?: string
  block_id?: string
  key?: string
}

export interface Reference {
  entity_id: string
  version?: { mode: string }
  selector?: Selector
  workspace_id?: string
}

export type FieldType = 'text' | 'number' | 'decimal' | 'date' | 'datetime' | 'boolean' | 'select' | 'multi_select' | 'object_ref'

export interface OptionDef { option_id: string; label: string }

export interface FieldDef {
  field_id: string
  name: string
  type: FieldType
  required?: boolean
  nullable?: boolean
  unique?: boolean
  scale?: number
  options?: OptionDef[]
  description?: string
  maintained_by?: 'human' | 'program'
  order_key?: string
  def_rev: number
  type_rev: number
  values_rev: number
}

export interface TableSourceContent {
  fields: FieldDef[]
  key_revs: Record<string, number>
  members_rev: number
  payload: { title_field_id?: string; description?: string; data_mode?: string }
  record_count: number
}

export interface Diagnostic { key: string; code: string; field_id?: string; option_id?: string }

export type FilterNode =
  | { op: 'and' | 'or'; args: FilterNode[] }
  | { op: 'not'; arg: FilterNode }
  | { op: 'cmp'; field_id: string; operator: string; value?: Json }

export interface SortSpec { field_id: string; direction: 'asc' | 'desc' }

export interface CellPayload {
  source_ref: Reference
  view: { type: 'table' | 'richtext' | 'record' | 'asset' }
  title?: string
  fields?: { field_id: string; width?: number }[]
  filter?: FilterNode | null
  sorts?: SortSpec[]
  group?: { field_id: string } | null
  options?: Record<string, Json>
}

export interface KeyedContent<P> { payload: P; key_revs: Record<string, number>; diagnostics?: Diagnostic[] }

export interface RecordPropDef { key: string; name: string; type: FieldType; required?: boolean; nullable?: boolean; scale?: number; options?: OptionDef[] }
export interface RecordContent { schema: { properties: RecordPropDef[] }; props: Record<string, Json>; key_revs: Record<string, number> }

export interface AssetPayload { object_id: string; media_type?: string; size?: number; file_name?: string; image?: { width: number; height: number } }
export interface AssetContent extends KeyedContent<AssetPayload> { availability: 'available' | 'missing' | 'corrupt' }

export interface AnnotationPayload { target: Reference; kind: 'note' | 'highlight'; body: string; style?: Record<string, Json>; author?: string }
export interface AnnotationContent extends KeyedContent<AnnotationPayload> { anchor_state: 'resolved' | 'target_deleted' }

export interface AstNode { type: string; attrs?: Record<string, Json>; content?: AstNode[]; marks?: { type: string; attrs?: Record<string, Json> }[]; text?: string }
export interface RichTextContent { editor_schema: string; content: AstNode; content_rev?: number; blocks: Record<string, { hash: string; struct_rev: number }> }

export type ReadResult<C = unknown> = Result<EntityEnvelope & { content: C; head_seq: number }>

export interface CellMeta { derived?: { run_id?: string; program?: string; inputs?: unknown[] }; manual_override?: boolean }

export interface QueryRow {
  record_id: string
  rev: number
  revs: Record<string, number>
  values: Record<string, Json>
  meta?: Record<string, CellMeta>
  body_ref?: Reference
}

export interface QueryParams {
  view_id?: string
  source_id?: string
  filter?: FilterNode | null
  sorts?: SortSpec[]
  fields?: string[]
  limit?: number
  cursor?: string
  consistency?: 'snapshot' | 'best_effort'
}

export interface QueryPage {
  rows: QueryRow[]
  total: number
  next_cursor?: string | null
  consistency: string
  content_rev: number
  source_id: string
  diagnostics?: Diagnostic[]
  data_mode?: string
}

export interface CollabState {
  entity_id: string; lineage_id: string; engine: string; engine_version: string; encoding: string; snapshot: string; seq: number; content_rev: number
  /** Replica session only: updates of this device's still-undecided submissions (confirmed snapshot + these = working document). */
  pending_updates?: string[]
}

export interface Operation { op: string; [key: string]: unknown }

export interface Touched { entity_id: string; selector?: Selector; change: string; rev: number }

export interface ServerOp { entity_id: string; lineage_id: string; op_index: number; update: string }

export interface CommitAccepted { status: 'accepted'; commit_id: string; seq: number; replayed: boolean; touched: Touched[]; server_ops: ServerOp[] }

export interface ConflictItem {
  code: string
  op_index?: number
  item_index?: number
  entity_id?: string
  selector?: Selector
  expected_rev?: number
  current_rev?: number
  current_value?: Json
  detail?: string
  [key: string]: unknown
}

export interface CommitConflict { status: 'conflict'; code: string; retryable: boolean; conflicts: ConflictItem[]; plan_digest?: string; [key: string]: unknown }

export interface RejectError { code: string; sub_code?: string; detail?: string; op_index?: number; path?: string; retryable?: boolean; data?: Record<string, unknown> }

export interface CommitRejected { status: 'rejected'; code: string; sub_code?: string; retryable: boolean; errors?: RejectError[]; detail?: string; request_digest?: string; [key: string]: unknown }

export type CommitResult = CommitAccepted | CommitConflict | CommitRejected

/** `unknown`: the request may or may not have been accepted and the backend could not be asked (§2.6, §6.4). */
export interface CommitUnknown { status: 'unknown'; idempotency_key: string; detail: string }

/** Replica session: the submission is durable in this device's replica database and queued for sending (§6.3). */
export interface CommitSavedLocally { status: 'saved_locally'; idempotency_key: string; provisional_seq: number }

/** Replica session: the replica database could not be written. The edit is NOT saved anywhere (§6.5). */
export interface CommitStorageFailed { status: 'storage_failed'; idempotency_key: string; detail: string }

export type CommitOutcome = (CommitResult | CommitUnknown | CommitSavedLocally | CommitStorageFailed) & { idempotency_key: string }

export interface PrepareOk { status: 'ok'; touched: Touched[]; request_digest: string; would_be_seq: number; reports?: unknown[]; [key: string]: unknown }
export type PrepareResult = PrepareOk | CommitConflict | CommitRejected

export interface CommitRequest {
  protocol_version: string
  workspace_id: string
  epoch: string
  idempotency_key: string
  session_id: string
  origin?: 'human' | 'agent' | 'program' | 'import' | 'system'
  undo_group?: string
  message?: string
  preconditions?: unknown[]
  operations: Operation[]
}

export interface CommitEvent {
  seq: number
  /** Replica session only: not a commit — the local working view changed (a submission was saved, discarded or left the view). Carries `touched` only. */
  local?: boolean
  commit_id?: string
  origin?: string
  run_id?: string | null
  undoes?: string | null
  idempotency_key?: string
  author?: string
  accepted_at?: string
  touched?: Touched[]
  ops?: (Operation & { entity_id?: string; update?: string; server_update?: string; source_id?: string })[]
}

export interface ChangesPage { epoch: string; head_seq: number; changes: CommitEvent[]; more: boolean }

export interface LockInfo { entity_id: string; lock_id?: string; principal: string; session_id?: string; acquired_at: string; expires_at: string }

export interface RunView {
  run_id: string
  program: string
  state: 'planning' | 'running' | 'validating' | 'waiting_confirmation' | 'applying' | 'succeeded' | 'failed' | 'cancelled'
  simulated: boolean
  params: Record<string, Json>
  candidate: CommitRequest | null
  prepare: PrepareResult | null
  warnings: unknown[]
  /** The commit result of the application, or `{ error }` when the run failed before producing a candidate. */
  result: CommitResult | { error: ServiceError; status?: undefined } | null
  inputs: unknown[]
}

export interface ExportResult { export_id: string; manifest: { export_mode: string; content_root: string; self_contained: boolean; missing: unknown[]; excluded_entities: number; [key: string]: unknown } }

export interface Grant { subject: string; scope_entity_id: string | null; capabilities: Capability[] }
