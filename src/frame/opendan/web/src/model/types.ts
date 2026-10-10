export type SessionKind = 'ui' | 'work' | 'self_improve' | 'self_check'
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

export interface PerceptionCursor {
  offsets: Record<string, number>
  updated_at_ms: number
}

export interface BacklogItem {
  session_id: string
  from_offset: number
  to_offset: number
}

export interface PerceptionRecord {
  seq: number
  at_ms: number
  session_id: string
  kind: string
  source: string
  tags?: string[]
  objects?: string[]
  summary?: string
  payload?: unknown
  refs?: unknown
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

export interface KnownWorkspace {
  version: number
  runtime_host: string
  workspace_id: string
  name: string
  description: string
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
