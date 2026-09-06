import type { InstallInput } from './schemas'

export type TaskId = string
export type ServiceLayer = 'app' | 'system' | 'kernel'
export type AppServiceViewStatus = 'loading' | 'ready' | 'error'
export type RuntimeType = 'docker' | 'script' | 'web' | 'agent' | 'unknown'
export type AppServiceStatus =
  | 'installing'
  | 'deploying'
  | 'starting'
  | 'running'
  | 'stopping'
  | 'stopped'
  | 'activation_failed'
  | 'error'
  | 'unknown'
export type EvidenceState = 'READY' | 'NOT_READY' | 'UNKNOWN'
export type DocumentStatus =
  | 'Active'
  | 'Missing'
  | 'Expired'
  | 'Revoked'
  | 'Tombstoned'
  | 'Migrated'
  | 'Unknown'
export type InstallStage =
  | 'acquire'
  | 'verify'
  | 'prepare'
  | 'deploy'
  | 'activate'
  | 'unknown'
export type TaskAction =
  | 'cancel'
  | 'retry'
  | 'resume'
  | 'inspect'
  | 'change-source'

export interface ScopeContext {
  zone_id: string
  user_id: string
  role: 'admin' | 'user'
}
export interface InstallTarget {
  node_id: string
  node_did: string
  label: string
  os: string
  arch: string
  online: boolean
}
export interface PermissionItem {
  scope_path: string
  actions: string[]
  required: boolean
  exp: number | null
}
export interface Endpoint {
  name: string
  protocol: 'http' | 'https' | 'tcp' | 'udp'
  inner_port: number
  required: boolean
  route: 'web' | 'port'
}
export interface MountDeclaration {
  kind: 'data' | 'local_cache' | 'external'
  path: string
  target_path: string
  access: 'read_only' | 'read_write' | 'read_write_append'
  required: boolean
}
export interface EnvironmentDeclaration {
  name: string
  default: string
  required: boolean
  sensitive: boolean
  system: boolean
}
export interface AppDocumentView {
  schema_version: 1
  doc_type: 'app'
  did: string
  object_id: string
  app_id: string
  show_name: string
  version: string
  description_key: string
  app_type: 'dapp' | 'web' | 'agent'
  runtime_type: RuntimeType
  owner: string
  controller: string
  author: string
  publisher: string
  iconKey: string
  components: Array<{ name: string; required: boolean }>
  permissions: PermissionItem[]
  endpoints: Endpoint[]
  mounts: MountDeclaration[]
  environment: EnvironmentDeclaration[]
  start_param?: string
  container_param?: string
}
export type MockScenario =
  | 'normal'
  | 'developer'
  | 'revoked'
  | 'migrated'
  | 'tombstoned'
  | 'trust-pending'
  | 'signature-fail'
  | 'owner-fail'
  | 'document-fail'
  | 'content-missing'
  | 'unsupported'
  | 'config-invalid'
  | 'fail-download'
  | 'fail-committed'
  | 'paused'
  | 'stale'
  | 'activation-fail'
  | 'offline-node'
  | 'runtime-timeout'
  | 'runtime-unknown'
  | 'stale-evidence'
  | 'expired-reference'
  | 'import-fail'
  | 'upgrade'
  | 'downgrade'
export interface SourceReference {
  reference_id: string
  kind: 'identifier' | 'local-pikg' | 'personal-server-pikg' | 'url' | 'appdoc'
  display_name: string
  referrer?: string
  size_bytes: number | null
  expires_at: number | null
  state: 'ready' | 'released' | 'expired'
  fixture: string
  scenario: MockScenario
  content_available: boolean
}
export interface PickedPikgFile {
  location: 'device' | 'personal-server'
  name: string
  sizeBytes: number
  file?: File
  fixture?: string
}
export type SourceParseResult =
  | { ok: true; source: SourceReference }
  | { ok: false; code: string }
export interface FieldIssue {
  field: string
  code: string
  action: 'recheck' | 'change-source' | 'edit'
}
export interface PlanReadiness {
  document: EvidenceState
  signature: EvidenceState
  owner: EvidenceState
  authority: EvidenceState
  content: EvidenceState
  target: EvidenceState
  config: EvidenceState
  document_status: DocumentStatus
  local_developer_authority: boolean
  install: 'OFFLINE_READY' | 'CONTENT_DOWNLOAD_REQUIRED' | 'BLOCKED' | 'UNKNOWN'
  issues: FieldIssue[]
}
export interface InstallPlan {
  readonly schema_version: 4
  readonly plan_fingerprint: string
  readonly app_instance_id: string
  readonly owner_user_id: string
  readonly app: AppDocumentView
  readonly target: InstallTarget
  readonly input: InstallInput
  readonly plan_use: 'FRESH_INSTALL' | 'UPGRADE' | 'SATISFIED'
  readonly previous_version: string | null
  readonly previous_generation: number
  readonly download_bytes: number | null
  readonly created_at: number
  readonly expires_at: number
}
export interface InspectionDraft {
  draft_id: string
  source: SourceReference
  input: InstallInput
  app: AppDocumentView
  status: 'inspecting' | 'ready' | 'dirty' | 'blocked' | 'submitting' | 'stale'
  plan: InstallPlan | null
  readiness: PlanReadiness | null
  inspection_revision: number
  error: string | null
  suggestions: boolean
}
export interface DeploymentIdentity {
  app_instance_id: string
  task_id: TaskId
  app_doc_object_id: string
  spec_generation: number
}
export interface RuntimeView {
  type: RuntimeType
  status: AppServiceStatus
  expected_deployment: DeploymentIdentity | null
  evidence: {
    deployment: DeploymentIdentity
    observed_at: number
    valid_until: number
  } | null
  docker: {
    engine: 'running' | 'not_running' | 'unknown'
    image: 'present' | 'missing' | 'pulling' | 'unknown'
    imageName: string
    container: 'running' | 'stopped' | 'error' | 'not_created' | 'unknown'
  } | null
  process_health: EvidenceState
  web_accessible: EvidenceState
  environment_ready: EvidenceState
  agent_bindings: number | null
  reason: string | null
}
export interface InstallRecord {
  app_instance_id: string
  owner_user_id: string
  app_did: string
  app_doc_object_id: string
  task_id: TaskId | null
  version: string
  state: 'configuration_submitted' | 'installed'
  deployment: DeploymentIdentity | null
  app_name: string | null
  app_host_name: string | null
  app_index: number | null
  address: string | null
}
export interface InstallTask {
  task_id: TaskId
  schema_id: string
  app_instance_id: string
  owner_user_id: string
  app: AppDocumentView
  plan: InstallPlan
  readiness: PlanReadiness
  source: SourceReference
  phase: 'Running' | 'Waiting' | 'Terminal' | 'Unknown'
  outcome: 'Succeeded' | 'Failed' | 'Canceled' | null
  stage: InstallStage
  progress: number | null
  desired_state_committed: boolean
  available_actions: TaskAction[]
  retry_of: TaskId | null
  error: { code: string; retryable: boolean; field?: string } | null
  result: InstallRecord | null
  tick: number
  updated_at: number
}
export interface AppServiceItem {
  id: string
  app_instance_id: string | null
  system_service_id: string | null
  owner_user_id: string | null
  app_did: string | null
  name: string
  description: string
  iconKey: string
  version: string
  layer: ServiceLayer
  status: AppServiceStatus
  runtime: RuntimeView
  record: InstallRecord | null
  docker: RuntimeView['docker']
  diagnostics: string[]
  spec: Record<string, string>
  settings: Record<string, string>
  serviceInfo: Record<string, string>
  logs: string[]
  installTaskId?: string
  available_actions: Array<'start' | 'stop'>
  operation: 'start' | 'stop' | null
  operation_error: string | null
}
export type SubmitResult =
  | { action: 'submitted'; task_id: TaskId; retry_of: TaskId | null }
  | { action: 'satisfied'; task_id: null; app_instance_id: string }
