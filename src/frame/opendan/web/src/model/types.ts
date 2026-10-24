export type SessionKind = 'ui' | 'work' | 'self_check' | 'goal'
export type RunState = 'created' | 'ready' | 'running' | 'waiting' | 'finished'
export type Outcome = 'succeeded' | 'failed' | 'stopped'
export type Acceptance = 'n/a' | 'pending' | 'accepted' | 'discarded'
export type WaitingKind = 'input' | 'tool' | 'event' | 'children'
export type TurnStatus = 'completed' | 'failed' | 'budget_exhausted' | 'stopped'
export type OutboxStatus = 'pending' | 'sent' | 'failed'
export type Decision = 'accept' | 'discard'

export interface Touching {
  kind: string
  ref: string
  mode: string
  since_ms: number
}

export interface Activity {
  summary: string
  touching?: Touching[]
  heartbeat_ms: number
}

export interface SessionStatus {
  rev: number
  run_state: RunState
  outcome?: Outcome
  acceptance: Acceptance
  one_line_status: string
  report_brief: string
  pending_decision?: unknown
  waiting_for?: WaitingKind
  turn_open?: boolean
  activity: Activity
  last_runner?: { runner_id: string; host?: string; pid: number; lock_epoch: number; at_ms: number }
  last_error?: unknown
  updated_at_ms: number
}

export interface Origin {
  parent_session?: string
  intent_ref?: string
  reason_messages?: string[]
  report?: string
  created_by_call?: string
}

export interface RegistryEntry {
  session_id: string
  kind: SessionKind
  class: string
  created_by: { principal: string; via: string }
  idempotency_key?: string
  route_key?: string
  driver: { principal: string }
  location: string
  input_queue?: string
  wake_event?: string
  agent_access: 'full' | 'status_only'
  origin?: Origin
  workspace?: WorkspaceRef
  artifact_id?: string
  objective: string
  scope?: { objects?: string[]; paths?: string[] }
  status: SessionStatus
  unreachable?: boolean
  location_rev: number
}

export interface WaitingFor {
  kind: WaitingKind
  refs?: string[]
  deadline_ms?: number
}

export interface OpenTurn {
  index: number
  run_id: string
  input_seq: number
  hook?: string
  inputs?: string[]
  at_ms: number
}

export interface ChildCall {
  mode: 'switch_context' | 'create_sub_context' | 'fork'
  behavior: string
  trigger: { kind: 'behavior' } | { kind: 'tool'; call_id: string; task_id: string }
  task?: string
}

export interface ProcessFrame {
  entry: string
  role: 'parked' | 'caller'
  call?: ChildCall
  run_id: string
  turns: unknown[]
}

export interface PendingEventVersion {
  seq: number | null
  key: string
  event: string
  summary: string
  data_ref: string | null
  received_at_ms: number
}

export interface PendingEvent {
  subscription_id: string | null
  source: { kind: string; id: string }
  latest: PendingEventVersion | null
  terminal: PendingEventVersion | null
  superseded: number
}

export interface OutboxEntry {
  key: string
  msg: unknown
  turn: number
  purpose?: 'reply' | 'placeholder' | 'final_edit'
  status: OutboxStatus
  msg_id?: string
  deliveries?: string[]
  error?: string
  attempts: number
  updated_at_ms: number
}

export interface SessionState {
  schema: string
  rev: number
  writer?: { runner_id: string; principal: string; host?: string; pid: number; lock_epoch: number }
  run_state: RunState
  waiting_for?: WaitingFor
  outcome?: Outcome
  acceptance: Acceptance
  result?: unknown
  turn_seq: number
  open_turn?: OpenTurn
  turns_completed: number
  current_behavior?: string
  process_entry?: string
  process_stack?: ProcessFrame[]
  live_run: { run_id: string } | null
  last_run: string | null
  pending_events?: PendingEvent[]
  reply?: unknown
  outbox?: OutboxEntry[]
  watched_tasks?: string[]
  activity: Activity
  one_line_status: string
  last_error?: unknown
  pending_decision?: unknown
  stop_requested: boolean
  updated_at_ms: number
}

// `seq` plus a body tagged by `t` (created, turn_started, input_batch,
// user_message, assistant_message, step, action_result, outcome, turn_ended,
// compaction, decide, input_rejected, event_dropped, control_applied).
export interface WorklogEntry {
  seq: number
  t: string
  [field: string]: unknown
}

export interface LockInfo {
  resource: string
  epoch: number
  holder: { runner_id: string; principal: string; host?: string; pid: number; runtime_id?: string }
  acquired_at_ms: number
  released_at_ms: number | null
}

export interface Binding {
  schema: string
  target: unknown
  runtime_id: string
  kind: string
  workdir: string
  bound_at_ms: number
  bound_by: string
  workspace?: WorkspaceBinding | null
}

// Tagged by `kind`: idle, finished, outcomes_handled, turn_closed, turn_open,
// busy, run_busy, not_driver, unregistered, bind_failed, lease_lost, error, …
export interface DriveResult {
  kind: string
  [field: string]: unknown
}

export interface HostedStatus {
  session_id: string
  class: string
  loaded: boolean
  loaded_at_ms: number
  drives: number
  last_result?: DriveResult
  last_result_at_ms: number
  idle_unload_secs: number | null
  reason: string
}

export interface SessionConfigSummary {
  session: unknown
  runtime: unknown
  workspace: unknown
  workspace_binding?: WorkspaceBinding | null
  subscriptions: unknown
  channels: unknown
  behavior: string
  frozen: { catalog_rev: string; frozen_at_ms: number; frozen_by: string; behaviors: string[] } | null
}

export interface SessionDetail {
  entry: RegistryEntry
  state?: SessionState
  report?: string
  worklog?: WorklogEntry[]
  note?: string
  config?: SessionConfigSummary
  binding?: Binding | null
  statistics?: Record<string, number> | null
  runs?: string[]
  lease_holder?: LockInfo | null
  children: string[]
  hosted: HostedStatus | null
}

export interface ActiveSession {
  session_id: string
  kind: SessionKind
  run_state: RunState
  objective: string
  activity: Activity
  relation: 'same_target' | 'overlap' | 'other'
  overlap: string[]
  possibly_interrupted: boolean
  driver: string
}

export interface WorkspaceRef {
  workspace_id: string
  access?: 'read_write' | 'read_only'
}

export interface WorkspaceLocation {
  runtime_id: string
  directory: string
}

export interface WorkspaceBinding extends WorkspaceRef {
  runtime_host: string
  revision: number
  location_revision: number
  location: WorkspaceLocation
}

/** A runtime the agent's sessions and workspaces may use (`state/runtimes/`). */
export type ComputerAccess = 'none' | 'observe' | 'control'

export interface KnownRuntime {
  version: number
  runtime_id: string
  kind: 'container' | 'host_native' | 'remote_node' | 'remote_ssh'
  name: string
  description: string
  runtime_host: string
  exec: unknown
  roots: string[]
  capabilities: unknown
  availability: 'available' | 'runtime_unavailable' | 'permission_denied'
  builtin: boolean
  /** The runtime new sessions without a workspace start in. */
  default: boolean
  computer: ComputerAccess
  revision: number
  registered_by: string
  registered_at_ms: number
  updated_at_ms: number
  checked_at_ms: number | null
  last_error: string | null
}

export interface KnownWorkspace {
  version: number
  runtime_host: string
  workspace_id: string
  name: string
  description: string
  /** The project this directory is a working copy of (http/https), or null. */
  url: string | null
  location: WorkspaceLocation
  usage: 'private' | 'collaborative'
  lifecycle: 'active' | 'archived'
  availability: 'available' | 'missing' | 'runtime_unavailable' | 'permission_denied' | 'invalid_metadata' | 'conflict'
  revision: number
  location_revision: number
  updated_at_ms: number
  checked_at_ms: number | null
  source_session: string | null
  conflict: { code: string; message: string; candidate: WorkspaceLocation; at_ms: number } | null
  last_error: string | null
}

export interface WorkspaceCreateRequest {
  operation_id: string
  name: string
  description: string
  url: string | null
  location: WorkspaceLocation
}

export interface WorkspaceImportRequest {
  operation_id: string
  name: string | null
  description: string
  url: string | null
  location: WorkspaceLocation
}

/** `AgentSettings.runtimes` as the owner chose them for this agent. */
export interface RuntimeSettings {
  runtimes: Record<string, { enabled: boolean; computer: ComputerAccess; target: unknown }>
  default_runtime_id: string
}

export interface RuntimeSettingPatch {
  runtime_id: string
  enabled?: boolean
  computer?: ComputerAccess
  default?: boolean
}

export interface ArtifactHead {
  aid: string
  workspace?: WorkspaceRef
  head: string | null
  rev: number
  updated_at_ms: number
}

export interface ArtifactVersion {
  ver: string
  session: string
  base: string | null
  state: 'produced' | 'accepted' | 'discarded'
  outputs?: string[]
  workspace_ref?: unknown
  side_effects?: { call_id: string; tool: string; note: string }[]
  updated_at_ms: number
}

export interface BehaviorMeta {
  name: string
  objective?: string
  next?: string[]
}

export interface IdentityText {
  role?: string
  self?: string
  i18n?: Record<string, string>
}

export interface Hint {
  id: string
  time: string
  sentence: string
  kind: string
}

export interface ModuleStatus {
  name: string
  enabled: boolean
  running: boolean
  note?: string
}

export interface InboxStatus {
  route_key: string
  session_id?: string
  delivered: number
  dropped: number
  held?: string
  last_at_ms: number
}

export interface LoaderStatus {
  agent_did: string
  agent_id: string
  who: string
  agent_root: string
  started_at_ms: number
  now_ms: number
  authentication: {
    state: 'ready' | 'retry_pending' | 'unavailable' | 'reauthorization_required'
    last_error: string | null
    error_code: string | null
    since: number | null
    next_retry_time: number | null
  } | null
  modules: ModuleStatus[]
  hosted: HostedStatus[]
  ui: { scans: number; last_scan_ms: number; last_error?: string; inboxes: InboxStatus[] } | null
  errors: { at_ms: number; source: string; message: string }[]
}

// The agent's profile record in the zone; without a nickname `display_name`
// is the agent's user name. `owner_did` / `desktop_url` are null outside a
// zone. `editable` is false when the service is too old to keep a profile.
export interface AgentProfile {
  agent_did: string
  agent_id: string
  display_name: string
  avatar: string | null
  bio: string
  owner_did: string | null
  desktop_url: string | null
  editable: boolean
}

export interface ProfilePatch {
  display_name?: string
  avatar?: string
  bio?: string
}

export interface TokenCount {
  input: number
  output: number
  total: number
}

export interface ModelUsage {
  model: string
  hour: TokenCount
  day: TokenCount
  all: TokenCount
}

export interface UsageByModel {
  now_ms: number
  since_ms: number | null
  models: ModelUsage[]
}

// The conversation a UI session answers: the peer (or group) and the
// conversation's session id on that side.
export interface UiBinding {
  session_id: string
  to: string
  to_session?: string | null
  kind: string
}

// Goals (`state/goals/<goal_id>/`), mirroring lib_opendan/src/protocol/goal.rs.
export type GoalLifecycle = 'achieve' | 'maintain'
export type GoalStatus = 'active' | 'paused' | 'achieved' | 'archived'
export type EpochStart = 'created' | 'reopened' | 'restarted'
export type EpochEnd = 'archived' | 'achieved' | 'restarted'
export type ActivationReasonKind = 'created' | 'scheduled' | 'child' | 'feedback' | 'manual' | 'reopened' | 'restarted' | 'event' | 'answer'
export type ActivationOutcome = 'completed' | 'failed' | 'budget_exhausted' | 'stopped' | 'needs_input'
export type ActivationDecision = 'no_action' | 'acted' | 'created_work' | 'needs_info' | 'unknown'
export type GoalQuestionStatus = 'open' | 'answered' | 'dismissed' | 'expired' | 'canceled'
export type ReceiptStage = 'prepared' | 'memory_committed' | 'published' | 'completed' | 'failed'
export type CandidateKind = 'goal' | 'schedule'
export type CandidateStatus = 'proposed' | 'needs_confirmation' | 'approved' | 'materializing' | 'created' | 'linked' | 'rejected' | 'expired' | 'blocked'

/** `extensions.opendan`: the host's trusted marks of a Goal. */
export interface OpendanGoalExtension {
  builtin?: string
  policy_version: number
  capabilities?: string[]
  records_perception?: boolean
}

export interface GoalScope {
  objects?: string[]
  paths?: string[]
  workspace?: string
}

export interface GoalSkill {
  name: string
  ref?: string
  version?: string
  note?: string
}

export interface ActivationBudget {
  max_total_tokens?: number
  max_runs?: number
}

export interface PeriodBudget {
  window_secs?: number
  max_total_tokens?: number
  max_activations?: number
  max_works?: number
}

export interface GoalBudget {
  activation?: ActivationBudget
  period?: PeriodBudget
}

export interface GoalScheduleSpec {
  every_secs?: number
  min_interval_secs?: number
  start_on_create?: boolean
}

export interface GoalEpoch {
  epoch: number
  sid: string
  started_at_ms: number
  start: EpochStart
  reason?: string
  keep?: 'all' | 'facts'
  ended_at_ms?: number
  end?: EpochEnd
  end_reason?: string
}

export interface GoalDefinition {
  schema: string
  goal_id: string
  revision: number
  change_seq?: number
  title: string
  objective: string
  scope?: GoalScope
  lifecycle?: GoalLifecycle
  achieved_when?: string
  healthy_state?: string
  distance_evaluation?: string
  constraints?: string[]
  initial_skills?: GoalSkill[]
  owner: string
  created_by: { principal: string; via: string }
  beneficiaries?: string[]
  budget?: GoalBudget
  schedule?: GoalScheduleSpec
  behavior?: string
  status: GoalStatus
  status_changed_at_ms?: number
  status_reason?: string
  status_by?: string
  session: { epoch: number; sid: string }
  epochs?: GoalEpoch[]
  idempotency_key?: string
  created_at_ms: number
  updated_at_ms?: number
  extensions?: Record<string, unknown>
}

export interface GoalChange {
  seq: number
  at_ms: number
  kind: string
  who: string
  via: string
  reason?: string
  revision?: number
  epoch?: number
  detail?: unknown
}

export interface ActivationReason {
  kind: ActivationReasonKind
  key: string
  ref?: string
  missed?: number
  note?: string
}

export interface ActivationCost {
  runs?: number
  total_tokens?: number
}

export interface GoalActivation {
  schema: string
  goal_id: string
  epoch: number
  seq: number
  session_id: string
  goal_revision: number
  started_at_ms: number
  ended_at_ms: number
  reasons?: ActivationReason[]
  outcome: ActivationOutcome
  decision: ActivationDecision
  summary?: string
  report_ref?: string
  works_created?: string[]
  works_absorbed?: string[]
  cost?: ActivationCost
  end_reason?: string
  result?: unknown
}

export interface LastActivation {
  epoch: number
  seq: number
  at_ms: number
  reasons?: ActivationReason[]
  outcome: ActivationOutcome
  decision: ActivationDecision
  summary?: string
}

export interface GoalDistance {
  text: string
  observed_at_ms?: number
  evidence_refs?: string[]
  unknowns?: string[]
}

export interface GoalQuestion {
  text: string
  at_ms: number
  epoch: number
  activation: number
  notify_id?: string
  status?: GoalQuestionStatus
  answer?: string
  candidate_id?: string
}

export interface GoalWork {
  sid: string
  objective?: string
  epoch: number
  created_in_activation: number
  goal_revision: number
}

export interface RecentWork {
  sid: string
  outcome?: Outcome
  one_line_status?: string
  report_ref?: string
  absorbed_in: [number, number]
}

export interface GoalUsage {
  period_start_ms: number
  tokens?: number
  activations?: number
  works?: number
  work_tokens?: number
}

export interface ConsolidationReceipt {
  operation_id: string
  epoch: number
  activation: number
  batch_id: string
  graph_seq: number
  refs?: string[]
  batch: unknown
  plan?: unknown
  stage: ReceiptStage
  memory_occasion?: string
  operations?: number
  dispositions?: number
  cleanup_pending?: boolean
  created_at_ms: number
  updated_at_ms?: number
  last_error?: string
}

export interface MaintenanceCandidate {
  candidate_id: string
  revision: number
  kind: CandidateKind
  semantic_key: string
  title: string
  action: string
  scope?: string[]
  source_refs?: string[]
  evidence_refs?: string[]
  intent_basis: string
  authorization_ref?: string
  missing_fields?: string[]
  proposed?: unknown
  estimated_budget?: string
  expected_effects?: string[]
  status: CandidateStatus
  question_id?: string
  notify_id?: string
  approved_revision?: number
  approved_by?: string
  approved_at_ms?: number
  linked_id?: string
  operation_key?: string
  last_error?: string
  retry_at_ms?: number
  created_at_ms: number
  updated_at_ms?: number
  review_at_ms?: number
  expires_at_ms?: number
}

export interface MaintenanceState {
  receipts?: ConsolidationReceipt[]
  candidates?: MaintenanceCandidate[]
  last_successful_consolidation_at_ms?: number
  consolidations?: number
  operations?: number
  dispositions?: number
  abandoned_batches?: number
  last_error?: string
}

export interface GoalState {
  schema: string
  goal_id: string
  rev: number
  epoch: number
  goal_revision_seen?: number
  last_activation?: LastActivation
  distance?: GoalDistance
  pending?: string[]
  questions?: GoalQuestion[]
  active_works?: GoalWork[]
  absorbed?: string[]
  recent_works?: RecentWork[]
  current_skills?: GoalSkill[]
  usage: GoalUsage
  last_error?: unknown
  maintenance?: MaintenanceState
  updated_at_ms?: number
}

export interface PendingFire {
  slot: number
  key: string
  missed: number
}

export interface MaintenancePending {
  new?: number
  due?: number
  waiting?: number
  retry_cleanup?: number
  next_deadline?: string
  oldest_at_ms?: number
}

/** `no_work | no_change | active | mechanical | paused | disabled | unknown | backoff | period_budget`. */
export interface MaintenanceGate {
  status: string
  last_checked_slot?: number
  last_baseline_slot?: number
  last_checked_at_ms?: number
  last_full_check_at_ms?: number
  last_skip_reason?: string
  last_inputs_digest?: string
  pending?: MaintenancePending
  due_reviews?: number
  next_review_at?: string
  unfinished_receipts?: number
  last_run_at_ms?: number
  last_run_slot?: number
  backoff_until_ms?: number
  backoff_level?: number
  last_error?: string
}

export interface SkippedFire {
  slot: number
  at_ms: number
  reason: string
}

export interface GoalSchedule {
  schema: string
  goal_id: string
  backend: string
  every_secs?: number
  last_slot?: number
  pending?: PendingFire
  last_posted_key?: string
  last_fire_at_ms?: number
  start_posted_epoch?: number
  skipped?: SkippedFire[]
  last_error?: string
  gate?: MaintenanceGate
  updated_at_ms?: number
}
