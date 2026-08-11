import type {
  AIStatus,
  ApiNamespace,
  LocalModel,
  ModelMetadata,
  ProviderView,
  RouteTrace,
  GlobalRoutingView,
  StoreSnapshot,
  UsageEvent,
  UsageSummary,
  UsageTrendPoint,
  ValidationResult,
  WizardDraft,
} from './types'
import { getEmptySeed, getPopulatedSeed, model } from './seed'

function getScenarioFromURL(): 'empty' | 'populated' {
  const params = new URLSearchParams(window.location.search)
  return params.get('scenario') === 'populated' ? 'populated' : 'empty'
}

function namespaceFromApiType(apiType: string): ApiNamespace {
  if (apiType === 'rerank') return 'rerank'
  const namespace = apiType.split('.')[0]
  if (
    namespace === 'llm' ||
    namespace === 'embedding' ||
    namespace === 'image' ||
    namespace === 'vision' ||
    namespace === 'audio' ||
    namespace === 'video' ||
    namespace === 'agent'
  ) {
    return namespace
  }
  return 'llm'
}

function usageFinanceAmount(event: UsageEvent): number {
  return event.finance_snapshot?.amount ?? 0
}

const discoveredModelsByType: Record<string, string[]> = {
  sn_router: ['sn-router-balanced@sn-router', 'sn-router-image@sn-router'],
  openai: ['gpt-5.1@openai-main', 'gpt-5.1-mini@openai-main', 'text-embedding-3-large@openai-main'],
  anthropic: ['claude-opus-4.7@anthropic-work', 'claude-sonnet-4.5@anthropic-work'],
  google: ['gemini-2.5-pro@google-main', 'gemini-2.5-flash@google-main'],
  openrouter: ['openrouter-auto@openrouter-main', 'deepseek-r1@openrouter-main'],
  custom: ['custom-chat-model@custom-provider', 'custom-embedding@custom-provider'],
}

function modelsForDraft(draft: WizardDraft, instanceName: string): ModelMetadata[] {
  const names = discoveredModelsByType[draft.provider_type ?? 'custom'] ?? []
  return names.map((exactName) => {
    const providerModelId = exactName.split('@')[0] ?? exactName
    const apiTypes = providerModelId.includes('embedding') ? ['embedding.text' as const] : ['llm' as const]
    const mount = providerModelId.includes('embedding') ? 'embedding.large' : `llm.${draft.provider_type ?? 'custom'}`
    return model(providerModelId, instanceName, [mount], apiTypes)
  })
}

export class MockDataStore {
  private providers: Map<string, ProviderView>
  private usageEvents: UsageEvent[]
  private routingView: GlobalRoutingView
  private routeTraces: RouteTrace[]
  private localModels: LocalModel[]
  private snapshot: StoreSnapshot
  private usageSummary: UsageSummary
  private usageTrend: UsageTrendPoint[]
  private listeners: Set<() => void> = new Set()
  private snapshotVersion = 0

  constructor() {
    const scenario = getScenarioFromURL()
    const seed = scenario === 'populated' ? getPopulatedSeed() : getEmptySeed()

    this.providers = new Map(seed.providers.map((p) => [p.config.id, p]))
    this.usageEvents = seed.usageEvents
    this.routingView = seed.routingView
    this.routeTraces = seed.routeTraces
    this.localModels = seed.localModels
    this.snapshot = this.buildSnapshot()
    this.usageSummary = this.computeUsageSummary()
    this.usageTrend = this.computeUsageTrend()
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = (): StoreSnapshot => {
    return this.snapshot
  }

  getSnapshotVersion = (): number => this.snapshotVersion

  private notify() {
    this.snapshot = this.buildSnapshot()
    this.usageSummary = this.computeUsageSummary()
    this.usageTrend = this.computeUsageTrend()
    this.snapshotVersion++
    this.listeners.forEach((fn) => fn())
  }

  private buildSnapshot(): StoreSnapshot {
    return {
      providers: Array.from(this.providers.values()),
      usageEvents: this.usageEvents,
      routingView: this.routingView,
      routeTraces: this.routeTraces,
      localModels: this.localModels,
      aiStatus: this.computeAIStatus(),
    }
  }

  private allModels(): ModelMetadata[] {
    return [
      ...Array.from(this.providers.values()).flatMap((p) => p.status.discovered_models),
      ...this.localModels,
    ]
  }

  private computeAIStatus(): AIStatus {
    const providers = Array.from(this.providers.values())
    const cloudProviderCount = providers.filter((p) => p.config.provider_runtime_type === 'cloud_api' || p.config.provider_runtime_type === 'proxy_unknown').length
    const models = this.allModels()
    const hasProviderOrModel = cloudProviderCount > 0 || models.length > 0
    const healthCounts = {
      available: models.filter((m) => m.health.status === 'available').length,
      degraded: models.filter((m) => m.health.status === 'degraded').length,
      unavailable: models.filter((m) => m.health.status === 'unavailable').length,
    }
    const quotaWarnings = models.filter((m) => m.health.quota_state === 'near_limit' || m.health.quota_state === 'exhausted').length

    let state: AIStatus['state'] = 'disabled'
    if (cloudProviderCount === 1) state = 'single_provider'
    else if (cloudProviderCount > 1 || (hasProviderOrModel && models.length > 1)) state = 'multi_provider'

    return {
      state,
      provider_count: cloudProviderCount,
      model_count: models.length,
      default_routing_ok: this.routingView.logical_tree.length > 0,
      health_counts: healthCounts,
      quota_warnings: quotaWarnings,
      inventory_ok: providers.every((p) => p.status.model_sync_status === 'ok'),
    }
  }

  getAIStatus(): AIStatus {
    return this.snapshot.aiStatus
  }

  getProviders(): ProviderView[] {
    return Array.from(this.providers.values())
  }

  getProvider(id: string): ProviderView | undefined {
    return this.providers.get(id)
  }

  addProvider(draft: WizardDraft): ProviderView {
    const id = `provider-${Date.now()}`
    const providerType = draft.provider_type ?? 'custom'
    const instanceName = draft.provider_instance_name ?? `${providerType}-${Date.now().toString(36)}`
    const models = modelsForDraft(draft, instanceName)
    const isSnRouter = providerType === 'sn_router'

    const view: ProviderView = {
      config: {
        id,
        name: draft.name || providerType,
        provider_type: providerType,
        provider_instance_name: instanceName,
        provider_runtime_type: isSnRouter ? 'proxy_unknown' : 'cloud_api',
        provider_driver: isSnRouter ? 'sn' : providerType,
        provider_origin: isSnRouter ? 'builtin' : 'user_config',
        auth_mode: draft.api_key ? 'api_key' : 'oauth',
        endpoint: draft.endpoint || undefined,
        protocol_type: draft.protocol_type ?? undefined,
        auto_sync_models: draft.auto_sync_models,
        created_at: new Date().toISOString(),
      },
      inventory: {
        provider_instance_name: instanceName,
        provider_type: isSnRouter ? 'proxy_unknown' : 'cloud_api',
        provider_driver: isSnRouter ? 'sn' : providerType,
        provider_origin: isSnRouter ? 'builtin' : 'user_config',
        inventory_revision: `${instanceName}-rev-now`,
        version: 'mock',
        models,
      },
      status: {
        provider_id: id,
        is_connected: true,
        auth_status: 'ok',
        usage_supported: true,
        balance_supported: providerType !== 'custom',
        discovered_models: models,
        model_sync_status: 'ok',
        last_verified_at: new Date().toISOString(),
        last_model_sync_at: new Date().toISOString(),
      },
      account: {
        provider_instance_name: instanceName,
        usage_supported: true,
        cost_supported: !isSnRouter,
        balance_supported: providerType !== 'custom',
        pricing_mode: isSnRouter ? 'free_quota' : 'per_token',
        balance_unit: isSnRouter ? 'credit' : 'usd',
        balance_value: isSnRouter ? 500 : undefined,
      },
    }

    this.providers.set(id, view)
    this.notify()
    return view
  }

  deleteProvider(id: string): void {
    this.providers.delete(id)
    this.notify()
  }

  refreshProviderModels(_id: string): void {
    this.notify()
  }

  updateProviderKey(id: string): void {
    const provider = this.providers.get(id)
    if (!provider) return
    this.providers.set(id, {
      ...provider,
      config: {
        ...provider.config,
        auth_mode: 'api_key',
      },
      status: {
        ...provider.status,
        auth_status: 'ok',
        is_connected: true,
        model_sync_status: 'ok',
        last_verified_at: new Date().toISOString(),
        last_model_sync_at: new Date().toISOString(),
      },
    })
    this.notify()
  }

  setProviderWeight(providerInstanceName: string, weight: number): void {
    const nextWeights = { ...this.routingView.provider_weights }
    if (weight === 1) {
      delete nextWeights[providerInstanceName]
    } else {
      nextWeights[providerInstanceName] = weight
    }
    this.routingView = {
      ...this.routingView,
      provider_weights: nextWeights,
    }
    this.notify()
  }

  validateConnection(draft: WizardDraft): ValidationResult {
    const errors: string[] = []
    const errorDetails: ValidationResult['error_details'] = []
    let endpointReachable = true
    let authValid = true

    if (draft.provider_type === 'custom' && !draft.endpoint) {
      endpointReachable = false
      const message = 'Endpoint URL is required for custom providers'
      errors.push(message)
      errorDetails.push({ kind: 'endpoint', message })
    }

    if (draft.provider_type !== 'sn_router' && !draft.api_key) {
      authValid = false
      const message = 'API Key is required'
      errors.push(message)
      errorDetails.push({ kind: 'auth', message })
    }

    const models =
      endpointReachable && authValid
        ? discoveredModelsByType[draft.provider_type ?? 'custom'] ?? []
        : []

    return {
      endpoint_reachable: endpointReachable,
      auth_valid: authValid,
      models_discovered: models,
      balance_available: draft.provider_type !== 'custom' && authValid,
      errors,
      error_details: errorDetails,
    }
  }

  getUsageSummary(): UsageSummary {
    return this.usageSummary
  }

  private computeUsageSummary(): UsageSummary {
    const now = new Date()
    const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
    const monthStart = new Date(now.getFullYear(), now.getMonth(), 1).getTime()

    const byApiNamespace: Record<ApiNamespace, number> = {
      llm: 0,
      embedding: 0,
      rerank: 0,
      image: 0,
      vision: 0,
      audio: 0,
      video: 0,
      agent: 0,
    }
    const byProvider: Record<string, number> = {}
    const byModel: Record<string, number> = {}
    const byApp: Record<string, number> = {}

    let totalTokens = 0
    let totalRequests = 0
    let totalCost = 0
    let todayTokens = 0
    let monthTokens = 0

    for (const evt of this.usageEvents) {
      const tokens = evt.token_equivalent ?? (evt.tokens_in ?? 0) + (evt.tokens_out ?? 0)
      const ts = new Date(evt.timestamp).getTime()
      const namespace = namespaceFromApiType(evt.api_type)

      totalTokens += tokens
      totalRequests++
      totalCost += usageFinanceAmount(evt)

      if (ts >= todayStart) todayTokens += tokens
      if (ts >= monthStart) monthTokens += tokens

      byApiNamespace[namespace] = (byApiNamespace[namespace] ?? 0) + tokens
      byProvider[evt.provider_instance_name] = (byProvider[evt.provider_instance_name] ?? 0) + tokens
      byModel[evt.exact_model] = (byModel[evt.exact_model] ?? 0) + tokens
      byApp[evt.app_id ?? 'system'] = (byApp[evt.app_id ?? 'system'] ?? 0) + tokens
    }

    return {
      total_tokens: totalTokens,
      total_requests: totalRequests,
      total_estimated_cost: Number(totalCost.toFixed(2)),
      today_tokens: todayTokens,
      this_month_tokens: monthTokens,
      by_api_namespace: byApiNamespace,
      by_provider: byProvider,
      by_model: byModel,
      by_app: byApp,
    }
  }

  getUsageTrend(_granularity: string): UsageTrendPoint[] {
    return this.usageTrend
  }

  private computeUsageTrend(): UsageTrendPoint[] {
    const now = Date.now()
    const day = 24 * 60 * 60 * 1000
    const points: UsageTrendPoint[] = []

    for (let i = 29; i >= 0; i--) {
      const dayStart = now - (i + 1) * day
      const dayEnd = now - i * day
      let tokens = 0
      let cost = 0

      for (const evt of this.usageEvents) {
        const ts = new Date(evt.timestamp).getTime()
        if (ts >= dayStart && ts < dayEnd) {
          tokens += evt.token_equivalent ?? (evt.tokens_in ?? 0) + (evt.tokens_out ?? 0)
          cost += usageFinanceAmount(evt)
        }
      }

      points.push({
        timestamp: new Date(dayEnd).toISOString().slice(5, 10),
        tokens,
        estimated_cost: Number(cost.toFixed(4)),
      })
    }
    return points
  }

  getUsageEvents(filters?: { provider_instance_name?: string; model?: string }): UsageEvent[] {
    let events = this.usageEvents
    if (filters?.provider_instance_name) {
      events = events.filter((e) => e.provider_instance_name === filters.provider_instance_name)
    }
    if (filters?.model) {
      events = events.filter((e) => e.exact_model === filters.model)
    }
    return events
  }

  getLocalModels(): LocalModel[] {
    return this.localModels
  }

  getGlobalRoutingView(): GlobalRoutingView {
    return this.routingView
  }

  getRouteTraces(): RouteTrace[] {
    return this.routeTraces
  }
}
