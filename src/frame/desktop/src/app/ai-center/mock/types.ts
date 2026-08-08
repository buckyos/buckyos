// ========== Enums ==========

export type ProviderType =
  | 'sn_router' | 'openai' | 'anthropic' | 'google' | 'openrouter' | 'custom'

export type ProviderRuntimeType = 'local_inference' | 'cloud_api' | 'proxy_unknown'
export type ProviderOrigin = 'system_config' | 'user_config' | 'builtin' | 'provider_claimed'
export type AuthMode = 'api_key' | 'oauth'
export type ProtocolType = 'openai_compatible' | 'anthropic_compatible' | 'google_compatible'
export type AuthStatus = 'ok' | 'expired' | 'invalid' | 'unknown'
export type ModelSyncStatus = 'ok' | 'syncing' | 'failed'
export type AISystemState = 'disabled' | 'single_provider' | 'multi_provider'

export type ApiNamespace =
  | 'llm' | 'embedding' | 'rerank' | 'image' | 'vision' | 'audio' | 'video' | 'agent'

export type ApiType =
  | 'llm'
  | 'embedding.text' | 'embedding.multimodal'
  | 'rerank'
  | 'image.txt2img' | 'image.img2img' | 'image.inpaint' | 'image.upscale' | 'image.bg_remove'
  | 'vision.ocr' | 'vision.caption' | 'vision.detect' | 'vision.segment'
  | 'audio.tts' | 'audio.asr' | 'audio.music' | 'audio.enhance'
  | 'video.txt2video' | 'video.img2video' | 'video.video2video' | 'video.extend' | 'video.upscale'
  | 'agent.computer_use'

export type ModelHealthStatus = 'available' | 'degraded' | 'unavailable'
export type QuotaState = 'normal' | 'near_limit' | 'exhausted' | 'unknown'
export type SchedulerProfile =
  | 'cost_first' | 'latency_first' | 'quality_first' | 'balanced' | 'local_first' | 'strict_local'
export type FallbackMode = 'parent' | 'strict' | 'disabled' | 'target_logical' | 'target_exact'
export type PricingMode = 'per_token' | 'subscription' | 'free_quota' | 'unknown'

// ========== Inventory / Models ==========

export interface ModelMetadata {
  provider_model_id: string
  provider_actual_model_id?: string
  provider_options?: unknown
  exact_model: string
  model_driver: string
  parameter_scale?: string
  api_types: ApiType[]
  logical_mounts: string[]
  capabilities: {
    streaming: boolean
    tool_call: boolean
    json_schema: boolean
    web_search: boolean
    web_search_with_tool_call?: boolean
    vision: boolean
    max_context_tokens?: number
    max_output_tokens?: number
  }
  attributes: {
    local: boolean
    privacy: 'local' | 'cloud' | 'private_safe' | 'public_cloud' | 'unknown'
    quality_score?: number
    latency_class: 'fast' | 'normal' | 'slow' | 'unknown'
    cost_class: 'low' | 'medium' | 'high' | 'unknown'
    tier?: 'flagship' | 'mid' | 'nano'
  }
  pricing: {
    input_token_usd?: number
    output_token_usd?: number
    cache_input_token_usd?: number
  }
  health: {
    status: ModelHealthStatus
    p50_latency_ms?: number
    p95_latency_ms?: number
    error_rate_5m?: number
    quota_state: QuotaState
  }
}

export interface ProviderInventory {
  provider_instance_name: string
  name?: string
  provider_type: ProviderRuntimeType
  provider_driver: string
  provider_origin: ProviderOrigin
  version?: string
  inventory_revision?: string
  models: ModelMetadata[]
}

// ========== Provider ==========

export interface ProviderConfig {
  id: string
  name: string
  provider_type: ProviderType
  provider_instance_name: string
  provider_runtime_type: ProviderRuntimeType
  provider_driver: string
  provider_origin: ProviderOrigin
  auth_mode?: AuthMode
  endpoint?: string
  protocol_type?: ProtocolType
  auto_sync_models: boolean
  created_at: string
}

export interface ProviderStatus {
  provider_id: string
  is_connected: boolean
  auth_status: AuthStatus
  usage_supported: boolean
  balance_supported: boolean
  discovered_models: ModelMetadata[]
  model_sync_status: ModelSyncStatus
  last_verified_at?: string
  last_model_sync_at?: string
}

export interface ProviderAccountStatus {
  provider_instance_name: string
  usage_supported: boolean
  cost_supported: boolean
  balance_supported: boolean
  pricing_mode: PricingMode
  usage_value?: number
  estimated_cost?: number
  balance_unit?: 'usd' | 'credit'
  balance_value?: number
  topup_url?: string
}

export interface ProviderView {
  config: ProviderConfig
  inventory: ProviderInventory
  status: ProviderStatus
  account: ProviderAccountStatus
}

export type AiProviderCard = {
  id: string
  displayName: string
  providerType: string
  status: 'healthy' | 'needs_setup' | 'degraded' | 'planned'
  endpoint: string
  authMode: string
  credentialConfigured?: boolean
  maskedApiKey?: string
  availableModels?: string[]
  capabilities: string[]
  defaultModel: string
  note: string
}

// ========== Usage ==========

export interface UsageEvent {
  id: string
  timestamp: string
  provider_instance_name: string
  exact_model: string
  requested_model: string
  api_type: ApiType
  tenant_id: string
  app_id?: string
  agent_id?: string
  session_id?: string
  tokens_in?: number
  tokens_out?: number
  token_equivalent?: number
  estimated_cost?: number
  finance_snapshot?: UsageFinanceSnapshot
  status: 'success' | 'failed'
}

export interface UsageFinanceSnapshot {
  amount?: number
  currency?: string
  provider_trace_id?: string
  billing?: unknown
}

export interface UsageSummary {
  total_tokens: number
  total_requests: number
  total_estimated_cost: number
  today_tokens: number
  this_month_tokens: number
  by_api_namespace: Record<ApiNamespace, number>
  by_provider: Record<string, number>
  by_model: Record<string, number>
  by_app: Record<string, number>
}

export interface UsageTrendPoint {
  timestamp: string
  tokens: number
  estimated_cost: number
}

// ========== Routing ==========

export interface LogicalRouteItem {
  target: string
  weight: number
}

export interface RoutePolicy {
  profile?: SchedulerProfile
  local_only?: boolean
  allow_fallback?: boolean
  runtime_failover?: boolean
  required_features?: string[]
  allowed_provider_instances?: string[]
  blocked_provider_instances?: string[]
  max_estimated_cost_usd?: number
}

export interface LogicalNode {
  path: string
  label: string
  level: 'L3' | 'L2' | 'L1'
  api_type?: ApiType
  items?: Record<string, LogicalRouteItem>
  exact_model_weights: Record<string, number>
  fallback?: { mode: FallbackMode; target?: string }
  policy?: RoutePolicy
  locked?: boolean
  children?: LogicalNode[]
  resolved_exact_model?: string
}

export interface GlobalRoutingView {
  logical_tree: LogicalNode[]
  global_exact_model_weights: Record<string, number>
  provider_weights: Record<string, number>
  policy: RoutePolicy
  revision?: string
}

export interface RouteTrace {
  request_id: string
  session_id?: string
  api_type: ApiType
  requested_model: string
  requested_model_type: 'exact' | 'logical'
  resolved_logical_path?: string
  selected_exact_model?: string
  selected_provider_instance_name?: string
  selected_provider_model_id?: string
  provider_trace_id?: string
  pricing_snapshot?: {
    input_token_usd?: number
    output_token_usd?: number
    cache_input_token_usd?: number
    estimated_cost_usd?: number
  }
  created_at_ms?: number
  latency_ms?: number
  duration_ms?: number
  ranked_candidates: Array<{
    exact_model: string
    final_score?: number
    selected: boolean
    pricing_snapshot?: RouteTrace['pricing_snapshot']
    exact_model_weight?: number
    provider_weight?: number
    preference_score_inputs?: {
      exact_model_weight: number
      provider_weight: number
      combined_weight: number
      preference_penalty: number
      exact_model_weight_effect: string
      provider_weight_effect: string
    }
    score_inputs?: {
      cost: number
      latency: number
      reliability: number
      quality: number
      preference: number
      cache: number
      local: number
    }
  }>
  filtered_candidates: Array<{ exact_model: string; reason: string }>
  fallback_applied: boolean
  fallback_chain: Array<{ from: string; to: string; reason: string }>
  session_sticky_hit: boolean
  scheduler_profile: SchedulerProfile
  runtime_failover_count: number
  user_summary?: {
    display_name: string
    model_family: string
    provider_origin: 'cloud' | 'local' | 'proxy_unknown'
    reason_short: string
    was_fallback: boolean
    was_failover: boolean
  }
  warnings: string[]
}

// ========== Local Model ==========

export interface LocalModel extends ModelMetadata {
  id: string
  name: string
  size_bytes?: number
  status: 'ready' | 'loading' | 'error'
  last_used_at?: string
}

// ========== System ==========

export interface AIStatus {
  state: AISystemState
  provider_count: number
  model_count: number
  default_routing_ok: boolean
  health_counts: Record<ModelHealthStatus, number>
  quota_warnings: number
  inventory_ok: boolean
}

// ========== Wizard ==========

export interface WizardDraft {
  provider_instance_name?: string
  provider_type: ProviderType | null
  name: string
  endpoint: string
  protocol_type: ProtocolType | null
  api_key: string
  auto_sync_models: boolean
}

export interface ValidationResult {
  endpoint_reachable: boolean
  auth_valid: boolean
  models_discovered: string[]
  balance_available: boolean
  errors: string[]
  error_details?: ValidationErrorDetail[]
  validation_fingerprint?: string
  validation_ttl_ms?: number
}

export interface ValidationErrorDetail {
  kind: 'endpoint' | 'auth' | 'models'
  message: string
}

// ========== Store Snapshot ==========

export interface StoreSnapshot {
  providers: ProviderView[]
  usageEvents: UsageEvent[]
  routingView: GlobalRoutingView
  routeTraces: RouteTrace[]
  localModels: LocalModel[]
  aiStatus: AIStatus
}
