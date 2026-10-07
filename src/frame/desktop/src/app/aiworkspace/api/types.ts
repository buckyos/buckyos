/* Wire types of the aiworkspace service (see src/frame/aiworkspace/README.md and the detailed design §2–§5).
 * Only what the Desktop app reads is typed; unknown keys are preserved as `unknown`. */

export const PROTOCOL_VERSION = '0.3'

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
  | 'buckyos.wish'
  | 'buckyos.block-def'

/** Fixed ids of the two trees (phase two §4): data tree root, Surface collection, canvas content area. */
export const ROOT_ID = 'root'
export const DATA_ID = 'data'
export const SURFACES_ID = 'surfaces'
export const CANVAS_CONTENT_ID = 'canvas-content'
export const SYSTEM_IDS: ReadonlySet<string> = new Set([ROOT_ID, DATA_ID, SURFACES_ID, CANVAS_CONTENT_ID])

/** Free-layout placement relative to the parent container; stacking order is the sibling `order_key`. */
/** `rotation`: degrees clockwise about the centre, `[0, 360)`, absent = 0 (标准对象的交互改进 §7.1). */
export interface Placement { x: number; y: number; w: number; h: number; rotation?: number }

/** Container kinds of phase two: `folder` (data tree), `surface`, `group` (BlockTree); `root` / `data` / `surfaces` are system nodes. */
export type ContainerKind = 'root' | 'data' | 'surfaces' | 'folder' | 'surface' | 'group'

/** Generation dependency record of a result entity (phase two §7.3). */
export interface DerivedInput {
  entity_id: string
  selector?: Selector
  version: { mode: 'follow' | 'fixed'; rev?: number; hash?: string }
  label?: string
}
export interface DerivedRecord {
  wish_id: string
  run_id: string
  executor: string
  inputs: DerivedInput[]
  generated_rev: number
  simulated?: boolean
  at?: string
  output_mode?: 'overwrite' | 'new'
  group?: string
  stale_at_import?: boolean
  /** The user kept their manual version against a later run: it stands for that run's inputs. */
  kept_manual?: boolean
  config_digest?: string
  result_key?: string
  approach?: 'program' | 'direct'
  program_digest?: string
  model_judgment?: boolean
  external_data?: boolean
  mode?: 'generate' | 'program'
}

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
  kind?: ContainerKind | string
  layout?: { mode: 'flow' | 'free' } | null
  /** Folders: `canvas_content` (the area) or `surface_content` (one Surface's folder). */
  system?: 'canvas_content' | 'surface_content' | null
  /** A content folder's Surface; a Surface's content folder. */
  surface_id?: string | null
  content_folder_id?: string | null
  /** A Surface's preset icon id (UI improvement §5.1); absent means the default of its layout. */
  icon?: string | null
  /** A Block or group the user locked (标准对象的交互改进 §7.2): the client does not move, resize or delete it. */
  locked?: boolean | null
  /** A connector's geometry keys as the outline projects them (连接线实现方案 §9.2; `config` is not projected). */
  connector?: ConnectorProjection | null
  parent_id?: string
  order_key?: string
  placement?: Placement
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
  derived?: DerivedRecord | null
  /** Cells (outline only). */
  view_type?: string | null
  view_version?: number | null
  source_id?: string | null
  def_id?: string | null
  /** Wishes / definitions / annotations / assets (outline only). */
  executor?: string | null
  output_mode?: string | null
  def_kind?: string | null
  target_id?: string | null
  annotation_kind?: string | null
  media_type?: string | null
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

export type BlockSnapshot = { object_id: string; media_type: string; size: number }

/** The connector keys of a Cell with `view.type = connector` (连接线实现方案 §4.2). */
export interface ConnectorProjection {
  start: ConnectorEnd | null
  end: ConnectorEnd | null
  flip?: { h?: boolean; v?: boolean }
  route?: 'straight' | 'elbow' | 'curve'
  controls?: { u: number; v: number; dx?: number; dy?: number }[]
  label?: { t: number; offset?: number }
}
export interface ConnectorEnd { entity_id: string; anchor: { kind: 'auto' } | { kind: 'point'; x: number; y: number } }

export interface CellPayload {
  /** Absent on a pure UI Block (frame, shape…). */
  source_ref?: Reference
  /** Renderer id and version (D6: the backend checks the format only; the registry decides support). */
  view: { type: string; version?: number }
  title?: string
  fields?: { field_id: string; width?: number }[]
  filter?: FilterNode | null
  sorts?: SortSpec[]
  group?: { field_id: string } | null
  options?: Record<string, Json>
  /** Renderer configuration (≤ 64 KiB). */
  config?: Record<string, Json> & { snapshot?: BlockSnapshot }
  /** A Block definition entity this Block uses (blocks its deletion). */
  def_ref?: Reference
  /** Named data bindings (`aiws` v2): `source_ref` is the binding called `source`. */
  bindings?: Record<string, { entity_id: string; selector?: Selector }>
  /** Connector keys (only with `view.type = connector`). */
  start?: ConnectorEnd | null
  end?: ConnectorEnd | null
  flip?: ConnectorProjection['flip']
  route?: ConnectorProjection['route']
  controls?: ConnectorProjection['controls']
  label?: ConnectorProjection['label']
}

export type WishExecutor = 'mock' | 'xllm' | 'agent-work-session'
/** A wish input: what is read (`entity`, a saved table view, a table query) and the stable name programs use (许愿格 §4.1). */
export interface WishInput { entity_id: string; selector?: Selector; version?: { mode: 'follow' | 'fixed'; rev?: number; hash?: string }; label?: string; name?: string; appended_by?: string }
export type WishResultType = 'richtext' | 'record' | 'table' | 'table_columns' | 'image' | 'asset' | 'html' | 'video'
export interface WishView { renderer: string; config?: Record<string, Json>; title?: string; size?: { w: number; h: number } }
export interface WishContractResult { name: string; type: WishResultType; title?: string; approach: 'program' | 'direct'; key?: string[]; target?: string; views?: WishView[]; description?: string; fields?: string[] }
export interface WishCheckDef { id: string; kind: 'program' | 'review'; text: string }
export interface WishBlocker { code: string; message: string; input_label?: string; candidates?: string[] }
/** `wish.analysis.v2` as persisted (handles already bound to ids; `basis.digest` computed by the core). */
export interface WishAnalysis {
  schema_version: 'wish.analysis.v2'
  status: 'ready' | 'needs_input'
  prompt: string
  context_prompt: string
  output_contract: { results: WishContractResult[]; placement?: string; dynamic?: boolean }
  checks: WishCheckDef[]
  blockers: WishBlocker[]
  warnings: string[]
  basis?: { digest?: string; reads?: DerivedInput[] }
  run_id?: string
  at?: string
  executor?: string
  summary?: string
}
export interface WishRefinement { text: string; at?: string; run_id?: string }
export interface WishProgram { language: 'js'; api_version: number; source: string; digest: string; produces: string[]; run_id?: string; edited_by?: string }
export interface WishResultBinding { type: WishResultType; entity_id: string; approach?: 'program' | 'direct'; title?: string; cells?: string[]; key?: string[]; fields?: Record<string, string>; def_id?: string; target?: string }
export interface WishCheckSummary { passed: number; failed: number; not_run: number; review: number; items: { id: string; kind: string; status: string; text: string }[] }
export interface WishLastRun {
  run_id: string
  state: 'succeeded' | 'failed' | 'cancelled'
  at?: string
  seq?: number
  mode?: 'generate' | 'program'
  executor?: string
  read_set?: DerivedInput[]
  produced?: string[]
  /** Results of the previous group that were not generated this time (not deleted, §10.2). */
  missing?: string[]
  group?: string
  simulated?: boolean
  config_digest?: string
  program_digest?: string
  checks?: WishCheckSummary
  result_bindings?: { group?: string; folder?: string; results: Record<string, WishResultBinding> }
  error?: string
}
export interface WishPayload {
  title?: string
  prompt: string
  knowledge?: string
  refinements?: WishRefinement[]
  analysis?: WishAnalysis
  inputs?: WishInput[]
  executor: WishExecutor
  output?: { container_id?: string; surface_id?: string; name: string; type?: string }
  output_mode?: 'overwrite' | 'new'
  executor_config?: Record<string, Json>
  program?: WishProgram
  last_run?: WishLastRun
}

// ---- wish runs on the service (proc.* with wish.xllm@1 / wish.mock@1)

export type WishStage = 'analyze' | 'execute' | 'rerun_program' | 'repair_program'
export type WishRunState = 'queued' | 'snapshotting' | 'running' | 'validating' | 'waiting_confirmation' | 'applying' | 'succeeded' | 'failed' | 'cancelled' | 'interrupted' | 'conflict' | 'rejected'
export interface WishRunProgress {
  phase?: string
  activity?: string
  tool_calls?: number
  program_runs?: number
  waiting_model?: boolean
  model?: string
  last_tool?: string
  last_command?: string
  llm_map?: { calls: number; cached: number; items: number }
  warning?: string
}
export interface WishResult {
  name: string
  type: WishResultType
  title?: string
  approach?: 'program' | 'direct'
  views?: WishView[]
  table?: { fields: { name: string; type?: string; options?: string[] }[]; key: string[]; rows: Record<string, Json>[] }
  columns?: { target: string; target_id: string; fields: { name: string; type?: string }[]; values: Record<string, Record<string, Json>> }
  record?: { properties: { name: string; type: string }[]; props: Record<string, Json> }
  markdown?: string
  file?: { media_type: string; size: number; image?: { width: number; height: number }; object_id?: string; file_name?: string }
  html?: { html: string; css?: string; js?: string; bindings?: Record<string, string> }
}
export interface WishCheckResult { id: string; kind: 'program' | 'review'; text: string; status: 'passed' | 'failed' | 'not_run' | 'review'; detail?: Json; self_assessment?: Json }
export interface WishCandidate {
  results?: WishResult[]
  facts?: Record<string, Json>
  checks?: WishCheckResult[]
  summary?: string
  assumptions?: string[]
  warnings?: string[]
  review_notes?: Json[]
  uncited_numbers?: { result: string; numbers: string[] }[]
  external_data?: string[]
  model_judgment?: string[]
  refinements?: WishRefinement[]
  program?: { digest: string; object_id?: string; produces?: string[] } | null
  appended_inputs?: WishInput[]
  mode?: 'generate' | 'program'
  simulated?: boolean
  /** Analysis stage. */
  analysis?: WishAnalysis
  inputs?: WishInput[]
}
export interface WishPlanChange { kind: string; field?: string; count?: number; destructive?: boolean; from?: string; to?: string | number; type?: string }
export interface WishPlanSummary {
  kind?: 'analysis'
  mode?: 'overwrite' | 'new'
  simulated?: boolean
  group?: { group_id: string; folder_id: string; surface_id: string | null; exists: boolean; rect: Placement | null; title: string }
  results?: { name: string; type: WishResultType; title?: string; entity_id: string; action: 'create' | 'update' | 'unchanged' | 'keep_manual' | 'new_copy'; approach?: string; blocks?: number; reordered?: boolean }[]
  missing?: { name: string; entity_id: string }[]
  manual?: { name: string; entity_id: string; cells?: number; field_deleted?: string }[]
  structure?: { name: string; entity_id: string; changes: WishPlanChange[]; rows_before?: number; rows_after?: number }[]
  destructive?: boolean
  appended_inputs?: WishInput[]
  program?: { from: string | null; to: string } | null
  refinements?: WishRefinement[]
  checks?: WishCheckResult[]
  failed_checks?: number
  problems: string[]
  operations?: number
  status?: string
  inputs?: WishInput[]
}
export interface WishRunView {
  run_id: string
  program: 'wish.xllm@1' | 'wish.mock@1'
  state: WishRunState
  stage: WishStage
  wish_id: string
  params: Record<string, Json>
  simulated: boolean
  created_at: string
  updated_at: string
  warnings?: Json
  error?: (ServiceError & { data?: Json }) | null
  usage?: Json
  progress?: WishRunProgress | null
  candidate?: WishCandidate | null
  preview?: { plan_digest: string; ready: boolean; summary: WishPlanSummary }
  applied?: { plan_digest: string; status: 'accepted' | 'conflict' | 'rejected'; commit: CommitResult }
  parent_run_id?: string | null
  feedback?: string | null
  program_log?: string | null
}
export interface WishChoices { results?: Record<string, 'keep' | 'replace' | 'new'>; confirm_structure?: boolean }

export interface BlockDefPayload {
  def_id: string
  version?: number
  kind: 'declarative' | 'html'
  title?: string
  description?: string
  accepts?: string[]
  allow_no_source?: boolean
  default_size?: { w: number; h: number }
  declarative?: Record<string, Json>
  html?: { html: string; css?: string; js?: string; api_version?: number }
  config_schema?: Json
  actions?: Json
  inspector?: Json
}

export type FreshnessStatus = 'current' | 'stale' | 'upstream_stale' | 'unavailable' | 'unknown' | 'none'
export interface FreshnessInputLine {
  entity_id: string
  selector?: Selector
  label?: string
  mode: 'follow' | 'fixed'
  readable?: boolean
  type_id?: string
  name?: string
  reason?: string
  recorded?: Json
  current?: Json
  changed?: boolean
  newer?: boolean
}
export interface FreshnessInfo {
  entity_id: string
  status: FreshnessStatus
  inputs?: FreshnessInputLine[]
  changed_inputs?: FreshnessInputLine[]
  upstream?: { entity_id: string; status: FreshnessStatus; changed_inputs?: FreshnessInputLine[] }[]
  manual_modified?: boolean
  imported_stale?: boolean
  generated_rev?: number
  content_rev?: number
  wish_id?: string | null
  run_id?: string | null
  executor?: string | null
  simulated?: boolean
  /** Generated under another configuration than the wish has now. */
  config_changed?: boolean
  approach?: 'program' | 'direct' | null
  result_key?: string | null
  external_data?: boolean
  model_judgment?: boolean
  /** Wishes only. */
  wish?: boolean
  needs_analysis?: boolean
  analysis_status?: 'ready' | 'needs_input' | null
  input_problems?: { entity_id: string; reason: string }[]
  last_run?: WishLastRun | null
  produced?: string[]
  /** The current result group, per logical result. */
  results?: { name: string; type: string; entity_id?: string; approach?: string | null; status: FreshnessStatus; manual_modified?: boolean; external_data?: boolean; model_judgment?: boolean }[]
  /** Program results are current but written ones are not: the text needs regenerating. */
  direct_stale?: boolean
  checks?: WishCheckSummary | null
  error?: ServiceError
}

export interface EntityBrief { entity_id: string; type_id: string; name: string | null; title?: string | null; deleted: boolean; parent_id?: string; kind?: string; view_type?: string }
export interface RelationLine { kind: string; selector: string; entity_id: string; target?: EntityBrief; source?: EntityBrief; readable?: boolean; missing?: boolean; external?: boolean; blocks_delete?: boolean; object_id?: string; workspace_id?: string }
export interface RelationsInfo {
  entity_id: string
  entity: EntityBrief
  outgoing: RelationLine[]
  incoming: RelationLine[]
  hidden_incoming: boolean
  blocks: EntityBrief[]
  produced: EntityBrief[]
  dependents: { kind: string; entity: EntityBrief }[]
  derived: DerivedRecord | null
  freshness?: FreshnessInfo
}

export interface VersionInfo {
  content_rev: number
  object_id: string
  derived: DerivedRecord | null
  kind: 'generated' | 'checkpoint'
  created_at: string
  commit_id?: string | null
  author?: string | null
  origin?: string | null
  run_id?: string | null
  message?: string | null
  accepted_at?: string | null
}

export interface Subject { subject: string; kind: 'user' | 'agent' | string }

export interface KeyedContent<P> { payload: P; key_revs: Record<string, number>; diagnostics?: Diagnostic[] }

export interface RecordPropDef { key: string; name: string; type: FieldType; required?: boolean; nullable?: boolean; scale?: number; options?: OptionDef[] }
export interface RecordContent { schema: { properties: RecordPropDef[] }; props: Record<string, Json>; key_revs: Record<string, number> }

export interface AssetPayload { object_id: string; media_type?: string; size?: number; file_name?: string; image?: { width: number; height: number } }
export interface AssetContent extends KeyedContent<AssetPayload> { availability: 'available' | 'missing' | 'corrupt' }

/** A position inside an annotation's target: a built-in kind (`richtext_text`) or an application's `<app>/<name>`. */
export interface AnnotationRange { kind: string; [key: string]: Json }
export interface AnnotationQuote { exact: string; prefix?: string; suffix?: string }
export interface AnnotationContext { quote?: AnnotationQuote; label?: string }
export interface AnnotationPayload {
  target: Reference
  range?: AnnotationRange
  context?: AnnotationContext
  kind: 'note' | 'highlight'
  body: string
  style?: Record<string, Json>
  author?: string
}
/** Where an annotation can be shown now (design §3.7): resolved at the recorded depth, degraded to a coarser one, … */
export interface AnchorInfo {
  state: 'resolved' | 'degraded' | 'target_deleted' | 'unsupported'
  level: 'range' | 'target' | 'entity' | 'none'
  range_status?: 'exact' | 'relocated' | 'lost' | 'unchecked'
  position?: { block_id?: string; end_block_id?: string; text?: string }
}
export interface AnnotationContent extends KeyedContent<AnnotationPayload> { anchor: AnchorInfo }
export type AnnotationRead = EntityEnvelope & { content: AnnotationContent }
export interface ListAnnotationsParams { target_ids?: string[]; parent_id?: string }

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
/** `complete`: the caller is a manager and saw every row; otherwise only the rows that apply to them. */
export interface GrantList { grants: Grant[]; complete: boolean; owner: string | null; principal: string }

export type WishPayloadRead = KeyedContent<WishPayload>
export type BlockDefRead = KeyedContent<BlockDefPayload>
