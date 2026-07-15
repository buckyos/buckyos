export type ServiceRole = 'tech' | 'ops'
export type ViewMode = 'browse' | 'edit'
export type MatchType = 'exact' | 'pattern' | 'default'
export type WarningSeverity = 'info' | 'warning' | 'blocked'
export type LoadingStatus = 'idle' | 'loading' | 'success' | 'error'

export interface DataState<T> {
  status: LoadingStatus
  data: T | null
  error: string | null
}

export interface ProviderRecord {
  provider_key: string
  provider_driver: string
  name: string
  base_url: string | null
  provider_kind: 'origin' | 'aggregator'
  protocol_family: string | null
  enabled: boolean
  owner_service: ServiceRole
  revision: string
  created_at: number
  updated_at: number
}

export interface ModelParamRuleRecord {
  rule_key: string
  provider_key: string | null
  source_rule_key: string | null
  match_type: MatchType
  original_provider: string | null
  model_id_selector: string | null
  priority: number | null
  model_driver: string | null
  api_types: string[]
  logical_mounts: string[]
  capabilities: Record<string, boolean | number | string>
  attributes: Record<string, unknown> | null
  context_limits: Record<string, number> | null
  pricing: Record<string, unknown> | null
  exclude: boolean
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface MetadataVariantRecord {
  variant_key: string
  provider_key: string | null
  source_variant_key: string | null
  selector_type: 'exact' | 'pattern'
  original_provider: string | null
  model_id_selector: string
  priority: number
  nick: string | null
  content: Record<string, unknown>
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface MetadataVersionRuleRecord {
  version_rule_key: string
  provider_key: string | null
  source_version_rule_key: string | null
  selector_type: 'exact' | 'pattern'
  original_provider: string | null
  model_id_selector: string
  priority: number
  nick: string | null
  content: Record<string, unknown>
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface ProviderModelRuleRecord {
  rule_key: string
  provider_key: string
  rule_type: 'include_origin' | 'include_pattern'
  selector: string
  priority: number
  enabled: boolean
  created_at: number
  updated_at: number
}

export interface ModelNickRecord {
  nick_key: string
  provider_key: string
  original_provider: string | null
  model_id: string
  nick: string
  selector_type: 'exact' | 'pattern'
  priority: number
  created_at: number
  updated_at: number
}

export interface LogicalDirectoryRecord {
  directory_key: string
  path: string
  title: string
  parent_key: string | null
  model_rule_keys: string[]
  created_at: number
  updated_at: number
}

export interface DictionaryItem {
  key: string
  label: string
  kind: 'api_type' | 'capability'
  value_type: 'boolean' | 'number'
  referenced_by: number
}

export interface OpsOverlayRecord {
  overlay_key: string
  target_type: 'provider' | 'model_param_rule' | 'variants' | 'version_rules'
  target_key: string
  disabled: boolean
  ops_patch: Record<string, unknown>
  created_at: number
  updated_at: number
}

export interface TechSourceRecord {
  service_url: string
  source_revision: string
  ops_revision: string
  last_sync_at: number | null
  last_success_at: number | null
  last_error: string | null
  stale: boolean
  cache_revision: string
}

export interface EditSessionRecord {
  session_id: string
  service_role: ServiceRole
  operator_id: string
  base_revision: string
  status: 'editing' | 'previewed' | 'approved' | 'published' | 'discarded'
  pending_change_count: number
  created_at: number
  updated_at: number
}

export interface PendingChangeRecord {
  change_key: string
  target_type: string
  target_key: string
  action: 'create' | 'update' | 'delete' | 'disable'
  summary: string
  risk: WarningSeverity
}

export interface WarningRecord {
  warning_key: string
  severity: WarningSeverity
  target_type: string
  target_key: string
  message_key: string
  detail?: string
  created_at: number
}

export interface ChangeLogRecord {
  change_id: string
  service_role: ServiceRole
  operator_id: string
  from_revision: string
  to_revision: string
  source_revision: string | null
  source_stale: boolean
  summary: string
  created_at: number
}

export type ImportPlanActionName =
  | 'upsert_provider'
  | 'disable_provider'
  | 'upsert_model_param_rule'
  | 'delete_model_param_rule'
  | 'include_models'
  | 'exclude_models'
  | 'set_model_nick'
  | 'upsert_variant'
  | 'upsert_version_rule'
  | 'set_logical_mounts'
  | 'upsert_logical_directory'
  | 'delete_logical_directory'
  | 'move_logical_directory'
  | 'set_api_types'
  | 'upsert_api_type'
  | 'delete_api_type'
  | 'set_capabilities'
  | 'upsert_capability'
  | 'delete_capability'

export interface ImportPlanActionRecord {
  action_key: string
  action: ImportPlanActionName | 'unsupported'
  raw_action: string
  target_type: string
  target_key: string
  selector: string | null
  match_type: MatchType | null
  priority: number | null
  hit_count: number
  samples: string[]
  affected_count: number
  reference_samples: string[]
  published_selector: string | null
  fallback_behavior: string | null
  source_record: string | null
  field_changes: Array<{
    field: string
    before: string
    after: string
  }>
  risk: WarningSeverity
  summary: string
  errors: string[]
}

export interface ImportPlanParseResult {
  plan_id: string
  title: string
  action_count: number
  supported_count: number
  error_count: number
  actions: ImportPlanActionRecord[]
  warnings: WarningRecord[]
}

export interface ImportPlanDraftRecord {
  draft_id: string
  title: string
  text: string
  parse_result: ImportPlanParseResult | null
  workspace: ProviderCloudSeed
  saved_at: number
}

export interface ProviderCloudSeed {
  published_revision: string
  source_revision: string
  ops_revision: string
  tech_source: TechSourceRecord
  providers: ProviderRecord[]
  model_param_rules: ModelParamRuleRecord[]
  metadata_variants: MetadataVariantRecord[]
  metadata_version_rules: MetadataVersionRuleRecord[]
  provider_model_rules: ProviderModelRuleRecord[]
  model_nicks: ModelNickRecord[]
  logical_directories: LogicalDirectoryRecord[]
  dictionaries: DictionaryItem[]
  ops_overlays: OpsOverlayRecord[]
  edit_session: EditSessionRecord | null
  pending_changes: PendingChangeRecord[]
  change_logs: ChangeLogRecord[]
  warnings: WarningRecord[]
  import_plan_result: ImportPlanParseResult | null
  import_plan_draft: ImportPlanDraftRecord | null
  revision_conflict: boolean
}

export interface DriverMetadataRule {
  [key: string]: unknown
  id?: string
  pattern?: string
  model_driver?: string
  exclude?: boolean
  parameter_scale?: string
  api_types?: string[]
  logical_mounts?: string[]
  capabilities?: Record<string, boolean | number | string>
  context_limits?: Record<string, number>
  pricing?: {
    price: number
    currency: string
    unit?: string
  }
  estimated_cost_usd?: number
  estimated_latency_ms?: number
  quality_score?: number
  latency_class?: string
  cost_class?: string
}

export interface DriverMetadataDocument {
  [key: string]: unknown
  schema_version: 1
  provider_driver: string
  name?: string | null
  protocol_family: string
  base_url?: string | null
  revision: string
  models: DriverMetadataRule[]
  patterns: DriverMetadataRule[]
  defaults: DriverMetadataRule
  variants: Array<Record<string, unknown>>
  version_rules: Array<Record<string, unknown>>
  signature: Record<string, unknown> | null
}

export interface OpsBulkPreviewRow {
  rule_key: string
  provider_key: string | null
  original_provider: string | null
  model_id_selector: string | null
  api_types: string[]
  capabilities: string[]
  visible_before: boolean
  visible_after: boolean
  pricing_before: {
    input: number
    output: number
  } | null
  pricing_after: {
    input: number
    output: number
  } | null
  routing_weight_before: number
  routing_weight_after: number
  recommendation_before: string
  recommendation_after: string
  display_priority_before: number
  display_priority_after: number
}

export interface OpsBulkOperationPreview {
  hit_count: number
  samples: OpsBulkPreviewRow[]
  visibility_removed: number
  visibility_added: number
  price_changed: number
  routing_changed: number
  display_priority_changed: number
}

export interface PublishPreview {
  revision: string
  provider_count: number
  model_rule_count: number
  pending_change_count: number
  warnings: WarningRecord[]
  impact: {
    providers: number
    model_rules: number
    dictionaries: number
  }
  ops_impact: {
    visible_providers: number
    visible_models: number
    disabled_providers: number
    disabled_models: number
    overlay_hits: number
    discarded_technical_fields: number
    source_stale: boolean
  }
  published_json: DriverMetadataDocument[]
}
