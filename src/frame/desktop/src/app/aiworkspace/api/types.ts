/* Wire types of the aiworkspace service (see src/frame/aiworkspace/README.md and the detailed design §2–§5).
 * Only what the Desktop app reads is typed; unknown keys are preserved as `unknown`. */

import type { StoredPlacement } from '@controls/placement'

export const PROTOCOL_VERSION = '1.1'

export type Json = null | boolean | number | string | Json[] | { [key: string]: Json }

export type Capability = 'read' | 'append' | 'update' | 'delete' | 'structure' | 'comment' | 'export' | 'manage' | 'run'

export interface ServiceError {
  code: string
  retryable?: boolean
  detail?: string
  sub_code?: string
  data?: Record<string, unknown>
  /** An access refusal's D-4 denial (`AccessDenial` of the WebSDK), validated by ServiceFailure. */
  denial?: unknown
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
  /** The show presenting this Workspace (第三期规划 §8.1): document writes are closed until it ends. */
  show_lock?: ShowLockInfo | null
  /** `show` for the temporary clone of a show (§8.3). */
  purpose?: string | null
  /** Who the service takes the caller for (a guest of a public link only learns its principal here). */
  principal?: string
  /** Whose AICC the wish blocks use (AI Center Quota PRD §3.3). */
  wish_aicc_mode?: WishAiccMode
  /** Where wish runs execute by default and whether the transitional in-service executor may be chosen (内核架构改进 D14). */
  wish_execution?: { default: WishExecutionMode; service: boolean }
  /** The owner's hosted execution configuration (D22): an Agent runs tasks for collaborators with `run`. */
  hosted_exec?: HostedExecConfig
  /** The Workspace's own origin for publication links (D21), `null` when the deployment has no host. */
  public_base?: string | null
  /** Bytes this Workspace's Sources fetched (D24). */
  traffic?: number
}

/** Where a sourced entity's content comes from (内核架构改进 D12); such an entity is read-only for people and Blocks. */
export interface EntitySource {
  kind: 'url' | 'workspace'
  mode: 'live' | 'fixed'
  url?: string | null
  workspace_id?: string | null
  entity_id?: string | null
  zone?: string | null
  refresh?: { interval_s: number } | null
  auth_mode?: 'none' | 'hosted'
}

/** A published data entity (D26): its link is the URL projection of the reference. */
export interface Publication { entity_id: string; name: string; path: string; url?: string; reference: Reference; published_at: string; published_by: string; referrers: number; current?: boolean }
export interface Referrer { referrer: { zone?: string | null; workspace_id?: string; entity_id?: string | null; principal?: string | null }; registered_at: string }

/** The owner's hosted execution configuration (`ws.set_hosted_exec`, D22). */
export interface HostedExecConfig { enabled: boolean; agent_id?: string; allowed_wishes?: string[]; triggerers?: string[]; limits?: Record<string, Json>; digest?: string }

/** Where a wish run executes (内核架构改进 §9, D14): the service (transitional), this browser, or the owner's Agent. */
export type WishExecutionMode = 'service' | 'local' | 'hosted'

export type WishAiccMode = 'workspace_owner' | 'current_user'
/** Whose AICC a run uses, fixed when it was accepted: `user_id` pays, `operator_id` started it. */
export interface WishRunAicc { mode: WishAiccMode; user_id: string; operator_id?: string }

export interface ShowLockInfo { show_id: string; presenter: string; started_at: string }

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
  | 'buckyos.viewport'
  | 'buckyos.show-path'

/** Fixed ids of the two trees (phase two §4): data tree root, Surface collection, canvas content area. */
export const ROOT_ID = 'root'
export const DATA_ID = 'data'
export const SURFACES_ID = 'surfaces'
export const CANVAS_CONTENT_ID = 'canvas-content'
/** The system folder of presentation paths and Viewports (第三期规划 §6.2). */
export const SHOWS_ID = 'shows'
export const SYSTEM_IDS: ReadonlySet<string> = new Set([ROOT_ID, DATA_ID, SURFACES_ID, CANVAS_CONTENT_ID, SHOWS_ID])

/** Free-layout placement relative to the parent container; stacking order is the sibling `order_key`. */
/** `rotation`: degrees clockwise about the centre, `[0, 360)`, absent = 0 (标准对象的交互改进 §7.1). */
export interface Placement { x: number; y: number; w: number; h: number; rotation?: number }
/** A placement as stored (脑图方案 §5.1): inside a control a component may be `"auto"`, computed when shown. Read laid-out
 * rectangles for geometry; only a Block outside any control has a plain `Placement`. */
export type { StoredPlacement, Axis } from '@controls/placement'

/** Container kinds of phase two: `folder` (data tree), `surface`, `group` (BlockTree); `root` / `data` / `surfaces` are system nodes;
 * a Block-control's kind is its control id (`mindmap`, 脑图方案 §6.2). */
export type ContainerKind = 'root' | 'data' | 'surfaces' | 'folder' | 'surface' | 'group' | 'mindmap'

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
  layout?: { mode: 'flow' | 'free' | 'tree' } | null
  /** A control's attributes (outline only; a Surface's `config` is not projected). */
  config?: Record<string, Json> | null
  /** Folders: `canvas_content` (the area), `surface_content` (one Surface's folder) or `shows` (presentation paths). */
  system?: 'canvas_content' | 'surface_content' | 'shows' | null
  /** A content folder's Surface; a Surface's content folder. */
  surface_id?: string | null
  content_folder_id?: string | null
  /** A Surface's preset icon id (UI improvement §5.1); absent means the default of its layout. */
  icon?: string | null
  /** A Block or group the user locked (标准对象的交互改进 §7.2): the client does not move, resize or delete it. */
  locked?: boolean | null
  /** A connector's geometry keys as the outline projects them (连接线实现方案 §9.2; `config` is not projected). */
  connector?: ConnectorProjection | null
  /** An ink segment's logical stroke (手写笔 TODO D0): the segments of one stroke act as one. */
  ink_stroke?: string | null
  parent_id?: string
  order_key?: string
  placement?: StoredPlacement
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
  /** A Block's role name inside its control (R22) and a mind map node's layout hints (`config.side`, `config.branch_color`). */
  slot?: string | null
  side?: 'left' | 'right' | null
  branch_color?: string | null
  /** Wishes / definitions / annotations / assets (outline only). */
  executor?: string | null
  /** Set on a sourced entity (D12). */
  source?: EntitySource | null
  output_mode?: string | null
  def_kind?: string | null
  /** A control definition's identity `<author_did>#<name>`, manifest version, build revision, package and source
   * snapshot directory (扩展控件 P0; outline only). */
  ident?: string | null
  manifest_version?: number | null
  def_version?: number | null
  provenance_pkg?: string | null
  source_dir_id?: string | null
  target_id?: string | null
  annotation_kind?: string | null
  media_type?: string | null
  /** A Block that may be operated on stage (`presentation.live`, 第三期规划 §10.2; outline only). */
  live?: boolean | null
  /** A Viewport's navigation point (outline only, §5.1). */
  viewport?: { surface_id: string | null; center: { x: number; y: number } | null; zoom: number | null } | null
  /** A presentation path (outline only, §6.1): its purpose, stage size and number of steps. */
  show_path?: { purpose: ShowPurpose | null; stage: StageSize | null; steps: number } | null
}

// ---- presentation (第三期规划 §4–§6)

export type ShowPurpose = 'presentation' | 'guide'
export interface StageSize { w: number; h: number }
export type StepTransition = 'auto' | 'fly' | 'fade' | 'cut'
/** One step of a presentation path: a Frame or a Viewport, referenced, never copied. */
export interface ShowStep {
  id: string
  target: { kind: 'frame' | 'viewport'; entity_id: string }
  title?: string
  enabled?: boolean
  transition?: StepTransition
  /** Cells hidden while this step is shown (shows only). */
  hide?: string[]
}
export interface ShowPathPayload { title?: string; purpose: ShowPurpose; stage: StageSize; background?: string; steps: ShowStep[] }
export interface ViewportPayload { title?: string; surface_ref: { entity_id: string }; center: { x: number; y: number }; zoom: number; notes?: string; caption?: string }
/** A Cell's `presentation`: `live` on any Block; the rest on a Frame. */
export interface PresentationProps { live?: boolean; background?: string; notes?: string; caption?: string }

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

export type FieldType = 'text' | 'number' | 'decimal' | 'date' | 'datetime' | 'boolean' | 'select' | 'multi_select' | 'object_ref' | 'currency'

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
  /** A currency field's default currency (ISO 4217); its values are `{ amount, currency }`. */
  currency?: string
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
export interface ConnectorEnd { entity_id: string; anchor: { kind: 'named'; id: string } }

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
  /** Presentation properties (第三期规划 §4.1, §10.2). */
  presentation?: PresentationProps
  /** A Block's role name inside its control (R22). */
  slot?: string
}

export type WishExecutor = 'mock' | 'xllm' | 'agent-work-session'
/** A wish input: what is read (`entity`, a saved table view, a table query) and the stable name programs use (许愿格 §4.1). */
export interface WishInput { entity_id: string; selector?: Selector; version?: { mode: 'follow' | 'fixed'; rev?: number; hash?: string }; label?: string; name?: string; appended_by?: string }
export type WishResultType = 'richtext' | 'record' | 'table' | 'table_columns' | 'image' | 'asset' | 'html' | 'video' | 'control'
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
  /** A Block-control instance (a mind map, 脑图 TODO M3): its tree; `under` names the input node it grows. */
  control?: { kind: string; config?: Record<string, Json>; nodes: { text: string; children: unknown[] }[]; under?: string }
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
  aicc?: WishRunAicc | null
  /** The execution context the run bound (§9.6). */
  context?: Record<string, Json> | null
  /** Hosted runs: how the Agent was reached (`unavailable` carries the claim token for whoever hands it on). */
  dispatch?: { status: string; claim_token?: string; detail?: string } | null
  execution?: { mode: WishExecutionMode; attempt: number; lease_until?: string | null; executor?: Json }
  /** Once, to the browser that started a local run: what claims it. */
  claim_token?: string
}

/** What `proc.claim` opens for an executor outside the service (T3-4): a model stage, or the program to re-run. */
export interface WishStageBrief {
  kind: 'model' | 'program'
  stage: WishStage
  system?: string
  user?: string
  tools?: { name: string; description: string; parameters: Json }[]
  model?: string
  iterations?: number
  source?: string | null
}
export interface WishClaim { run_id: string; attempt: number; run_token: string; lease_until: string; lease_ms: number; stage?: WishStageBrief }
export interface WishToolAnswer { status: 'ok' | 'error'; text?: string; map?: Json }
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
  /** Extension controls (扩展控件 P0, src/frame/aiworkspace/README.md “Extension controls”). */
  ident?: { author_did: string; name: string }
  manifest_version?: number
  source_dir?: { entity_id: string }
  bindings?: Record<string, ControlBinding>
  catalog?: { group?: string; icon?: string; description?: string }
  agent?: string
  provenance?: { pkg_id?: string; version?: string; digest?: string }
}

/** A named data input a control declares (its instances bind it in the Cell's `bindings`). */
export interface ControlBinding { accepts?: string[]; required?: boolean; title?: string; description?: string }

/** One problem of a control's source (`ext.build`'s `BUILD_FAILED.data.diagnostics`). */
export interface BuildDiagnostic { file: string; line?: number; col?: number; severity: 'error' | 'warning'; message: string; instances?: { entity_id: string; title?: string | null }[] }

/** `ext.build` / `ext.import`: one entry per control. */
export interface ExtControlResult { ident: string; def_entity_id: string; dir_entity_id?: string; action: 'created' | 'rebuilt' | 'unchanged' | 'satisfied'; version?: number; def_rev?: number; dir_rev?: number; build?: { input_digest: string; builder_version: string; built_at: string } }
export interface ExtBuildResult extends Partial<ExtControlResult> { status: 'accepted' | 'unchanged' | 'checked'; commit_id?: string; seq?: number; unchanged?: boolean; controls: ExtControlResult[]; diagnostics: BuildDiagnostic[] }
export interface ExtExportResult {
  manifest: { manifest_version: number; purpose: 'aiws'; host_api?: Record<string, number>; controls: { ident: string; name: string; kind: string; version: number; digest: string; dir: string; title?: string }[]; workspace_templates: Json[] }
  controls: { entity_id: string; ident: string; kind: string; version: number; def_rev: number; dir_rev: number; built: boolean; digest: string; dir: string; files: { name: string; obj_id?: string; size: number; text?: string }[] }[]
  files: { path: string; obj_id?: string; size: number; text?: string }[]
}
export interface LibraryControl { ident: string; name: string; kind: string; version: number; digest: string; dir: string; title?: string | null; description?: string | null; catalog?: Record<string, Json> | null }
export interface LibraryPackage { pkg_id: string; version: string; title: string; author: string; mode: string; installed_at: string; controls: LibraryControl[] }

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

export interface RecordPropDef { key: string; name: string; type: FieldType; required?: boolean; nullable?: boolean; scale?: number; currency?: string; options?: OptionDef[] }
export interface RecordContent { schema: { properties: RecordPropDef[] }; props: Record<string, Json>; key_revs: Record<string, number> }

/** `poster`: an image asset picturing the content (a video frame, a PDF's first page, a cover; 多媒体对象 §3). */
export interface AssetPayload {
  object_id: string; media_type?: string; size?: number; file_name?: string; image?: { width: number; height: number }
  poster?: string; duration_ms?: number; page_count?: number
}
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
  /** The name a guest chose to sign with (UI improvement §8.3); `author` stays the principal. */
  author_name?: string
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
  /** The row delta replicas apply as it is (内核架构改进 D20); present whenever `ops` is. */
  rows?: Record<string, unknown>
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

/** A file the service put into the owner's `exports` bucket (7 days) and the read token for that key alone. */
export interface BucketDownload { bucket_id: string; key: string; version: number; obj_id: string; token: string }
/** Where an upload is bound (`ndm` finalize `auto_bind`): the caller's own bucket of this service. */
export interface BucketBinding { bucket_id: string; key: string }
export interface StagedAsset { object_id: string; media_type: string; size: number; bucket_id: string; key: string; version: number }

export interface ExportResult { export_id: string; download: BucketDownload; manifest: { export_mode: string; content_root: string; self_contained: boolean; missing: unknown[]; excluded_entities: number; [key: string]: unknown } }

/** `via_entry`: the share entry whose join wrote the row. Subjects: a principal (user id, External DID), `public`,
 * `*` (unrestricted accounts), `group:<id>`, `contact:<owner_did>#<tag>`. */
export interface Grant { subject: string; scope_entity_id: string | null; capabilities: Capability[]; via_entry?: string | null }

/** A share entry (hybrid SSO contract §8.3): its id names the workspace (`<workspace_id>~se_…`); times are unix seconds. */
export interface ShareEntry {
  entry_id: string
  workspace_id: string
  scope_entity_id: string | null
  capabilities: Capability[]
  methods: { guest: boolean; password: boolean; self_register: boolean; did_login: boolean }
  state: 'active' | 'closed' | 'expired'
  expires_at: number | null
  created_by: string
  created_at: number
}

/** `ws.join_entry`: what the caller now holds through the entry — a grant of its own (`granted`), or, for an entry open
 * to guests that does not admit its sign-in method, the entry's access as anyone has it (no grant, ends with the entry). */
export interface EntryJoin { workspace_id: string; entry_id: string; scope_entity_id: string | null; capabilities: Capability[]; joined: boolean; granted: boolean; path: string }
/** `complete`: the caller is a manager and saw every row; otherwise only the rows that apply to them. */
export interface GrantList { grants: Grant[]; complete: boolean; owner: string | null; principal: string }

export type WishPayloadRead = KeyedContent<WishPayload>
export type BlockDefRead = KeyedContent<BlockDefPayload>

// ---- shows (第三期规划 §11.3, §12)

/** What `show.start` answers. */
export interface ShowStart {
  show_id: string
  prompter_token: string
  clone_workspace_id: string | null
  locked: boolean
  live: boolean
  started_at: string
  expires_at: string
  lease_ms: number
}
/** A control input from any entry point (keys, the control bar, a prompter, touch). */
export interface ShowCommand { command_id: string; kind: ShowCommandKind; args?: Record<string, Json> }
export type ShowCommandKind = 'next' | 'prev' | 'goto' | 'first' | 'last' | 'black' | 'back'
/** The state the stage publishes (the prompters only show it). */
export interface ShowState {
  show_id: string
  path_id: string
  step_id: string | null
  /** Index among the steps that are shown (enabled and not dangling). */
  index: number
  count: number
  free: boolean
  black: boolean
  /** The laser pointer in stage coordinates `[0, 1]²`, or absent. */
  pointer?: { x: number; y: number } | null
  transition?: string | null
  started_at: string
}
export interface ShowWatch { state: ShowState | Record<string, never>; seq: number; commands: { cursor: number; command: ShowCommand }[]; cursor: number; path_id: string; started_at: string; expires_at: string; timed_out: boolean }
export interface ShowNotes {
  title?: string | null
  purpose?: ShowPurpose | null
  stage?: StageSize | null
  steps: { id: string; kind: 'frame' | 'viewport'; target_id: string; title: string | null; enabled: boolean; missing: boolean; notes: string | null; caption: string | null }[]
}
