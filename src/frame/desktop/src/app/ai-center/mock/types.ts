// ========== Enums ==========

export type ProviderType =
  | 'sn_router' | 'openai' | 'anthropic' | 'google' | 'openrouter' | 'custom'

export type AuthMode = 'api_key' | 'oauth'
export type ProtocolType = 'openai_compatible' | 'anthropic_compatible' | 'google_compatible'
export type AuthStatus = 'ok' | 'expired' | 'invalid' | 'unknown'
export type ModelSyncStatus = 'ok' | 'syncing' | 'failed'
export type UsageCategory = 'text' | 'image' | 'audio' | 'video'
export type AISystemState = 'disabled' | 'single_provider' | 'multi_provider'

// ========== Provider ==========

export interface ProviderConfig {
  id: string
  name: string
  provider_type: ProviderType
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
  discovered_models: string[]
  model_sync_status: ModelSyncStatus
  last_verified_at?: string
  last_model_sync_at?: string
}

export interface ProviderAccountStatus {
  provider_id: string
  usage_supported: boolean
  cost_supported: boolean
  balance_supported: boolean
  usage_value?: number
  estimated_cost?: number
  balance_unit?: 'usd' | 'credit'
  balance_value?: number
  topup_url?: string
}

export interface ProviderView {
  config: ProviderConfig
  status: ProviderStatus
  account: ProviderAccountStatus
}

// ========== Usage ==========

export interface UsageEvent {
  id: string
  timestamp: string
  provider_id: string
  model_name: string
  category: UsageCategory
  app_id?: string
  agent_id?: string
  session_id?: string
  tokens_in: number
  tokens_out: number
  estimated_cost?: number
  status: 'success' | 'failed'
}

export interface UsageSummary {
  total_tokens: number
  total_requests: number
  total_estimated_cost: number
  today_tokens: number
  this_month_tokens: number
  by_category: Record<UsageCategory, number>
  by_provider: Record<string, number>
  by_model: Record<string, number>
}

export interface UsageTrendPoint {
  timestamp: string
  tokens: number
  estimated_cost: number
}

// ========== Routing ==========

export interface LogicalModelCandidate {
  provider_id: string
  model_name: string
  priority: number
  enabled: boolean
}

export interface LogicalModelConfig {
  name: string
  candidates: LogicalModelCandidate[]
  resolved_model?: string
}

// ========== Local Model ==========

export interface LocalModel {
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
}

// ========== Wizard ==========

export interface WizardDraft {
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
}

// ========== Store Snapshot ==========

export interface StoreSnapshot {
  providers: ProviderView[]
  usageEvents: UsageEvent[]
  logicalModels: LogicalModelConfig[]
  localModels: LocalModel[]
  aiStatus: AIStatus
}
