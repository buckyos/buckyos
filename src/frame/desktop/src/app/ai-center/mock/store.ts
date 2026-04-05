import type {
  AIStatus,
  LogicalModelConfig,
  LocalModel,
  ProviderView,
  StoreSnapshot,
  UsageEvent,
  UsageSummary,
  UsageTrendPoint,
  ValidationResult,
  WizardDraft,
} from './types'
import { getEmptySeed, getPopulatedSeed } from './seed'

function getScenarioFromURL(): 'empty' | 'populated' {
  const params = new URLSearchParams(window.location.search)
  return params.get('scenario') === 'populated' ? 'populated' : 'empty'
}

// Model lists returned by validateConnection per provider type
const discoveredModelsByType: Record<string, string[]> = {
  sn_router: ['llm-chat-standard', 'llm-chat-advanced', 'llm-code', 'txt2img-standard'],
  openai: ['gpt-4o', 'gpt-4o-mini', 'gpt-4-turbo', 'gpt-3.5-turbo', 'dall-e-3', 'dall-e-2', 'whisper-1', 'tts-1', 'tts-1-hd', 'text-embedding-3-large', 'text-embedding-3-small', 'text-embedding-ada-002'],
  anthropic: ['claude-3-opus', 'claude-3-sonnet', 'claude-3-haiku'],
  google: ['gemini-pro', 'gemini-pro-vision', 'gemini-ultra'],
  openrouter: ['openrouter/auto', 'openrouter/gpt-4o', 'openrouter/claude-3-sonnet'],
  custom: ['model-a', 'model-b'],
}

export class MockDataStore {
  private providers: Map<string, ProviderView>
  private usageEvents: UsageEvent[]
  private logicalModels: LogicalModelConfig[]
  private localModels: LocalModel[]
  private snapshot: StoreSnapshot
  private usageSummary: UsageSummary
  private listeners: Set<() => void> = new Set()
  private snapshotVersion = 0

  constructor() {
    const scenario = getScenarioFromURL()
    const seed = scenario === 'populated' ? getPopulatedSeed() : getEmptySeed()

    this.providers = new Map(seed.providers.map((p) => [p.config.id, p]))
    this.usageEvents = seed.usageEvents
    this.logicalModels = seed.logicalModels
    this.localModels = seed.localModels
    this.snapshot = this.buildSnapshot()
    this.usageSummary = this.computeUsageSummary()
  }

  // ---- Subscription (useSyncExternalStore compatible) ----

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
    this.snapshotVersion++
    this.listeners.forEach((fn) => fn())
  }

  private buildSnapshot(): StoreSnapshot {
    return {
      providers: Array.from(this.providers.values()),
      usageEvents: this.usageEvents,
      logicalModels: this.logicalModels,
      localModels: this.localModels,
      aiStatus: this.computeAIStatus(),
    }
  }

  // ---- Provider Operations ----

  private computeAIStatus(): AIStatus {
    const providers = Array.from(this.providers.values())
    const count = providers.length
    const modelCount = providers.reduce(
      (sum, p) => sum + p.status.discovered_models.length,
      0,
    )
    const hasRouting = this.logicalModels.some(
      (lm) => lm.resolved_model !== undefined,
    )

    let state: AIStatus['state'] = 'disabled'
    if (count === 1) state = 'single_provider'
    else if (count > 1) state = 'multi_provider'

    return {
      state,
      provider_count: count,
      model_count: modelCount,
      default_routing_ok: hasRouting,
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
    const models = discoveredModelsByType[draft.provider_type ?? 'custom'] ?? []

    const view: ProviderView = {
      config: {
        id,
        name: draft.name || (draft.provider_type ?? 'Custom'),
        provider_type: draft.provider_type ?? 'custom',
        auth_mode: draft.api_key ? 'api_key' : undefined,
        endpoint: draft.endpoint || undefined,
        protocol_type: draft.protocol_type ?? undefined,
        auto_sync_models: draft.auto_sync_models,
        created_at: new Date().toISOString(),
      },
      status: {
        provider_id: id,
        is_connected: true,
        auth_status: 'ok',
        usage_supported: true,
        balance_supported: draft.provider_type !== 'custom',
        discovered_models: models,
        model_sync_status: 'ok',
        last_verified_at: new Date().toISOString(),
        last_model_sync_at: new Date().toISOString(),
      },
      account: {
        provider_id: id,
        usage_supported: true,
        cost_supported: draft.provider_type !== 'sn_router',
        balance_supported: draft.provider_type !== 'custom',
        balance_unit: draft.provider_type === 'sn_router' ? 'credit' : 'usd',
        balance_value: draft.provider_type === 'sn_router' ? 500 : undefined,
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
    // noop in mock
    this.notify()
  }

  // ---- Wizard Simulation ----

  validateConnection(draft: WizardDraft): ValidationResult {
    const errors: string[] = []
    let endpointReachable = true
    let authValid = true

    if (draft.provider_type === 'custom' && !draft.endpoint) {
      endpointReachable = false
      errors.push('Endpoint URL is required for custom providers')
    }

    if (draft.provider_type !== 'sn_router' && !draft.api_key) {
      authValid = false
      errors.push('API Key is required')
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
    }
  }

  // ---- Usage ----

  getUsageSummary(): UsageSummary {
    return this.usageSummary
  }

  private computeUsageSummary(): UsageSummary {
    const now = new Date()
    const todayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime()
    const monthStart = new Date(now.getFullYear(), now.getMonth(), 1).getTime()

    const byCategory: Record<string, number> = { text: 0, image: 0, audio: 0, video: 0 }
    const byProvider: Record<string, number> = {}
    const byModel: Record<string, number> = {}

    let totalTokens = 0
    let totalRequests = 0
    let totalCost = 0
    let todayTokens = 0
    let monthTokens = 0

    for (const evt of this.usageEvents) {
      const tokens = evt.tokens_in + evt.tokens_out
      const ts = new Date(evt.timestamp).getTime()

      totalTokens += tokens
      totalRequests++
      totalCost += evt.estimated_cost ?? 0

      if (ts >= todayStart) todayTokens += tokens
      if (ts >= monthStart) monthTokens += tokens

      byCategory[evt.category] = (byCategory[evt.category] ?? 0) + tokens
      byProvider[evt.provider_id] = (byProvider[evt.provider_id] ?? 0) + tokens
      byModel[evt.model_name] = (byModel[evt.model_name] ?? 0) + tokens
    }

    return {
      total_tokens: totalTokens,
      total_requests: totalRequests,
      total_estimated_cost: Number(totalCost.toFixed(2)),
      today_tokens: todayTokens,
      this_month_tokens: monthTokens,
      by_category: byCategory as UsageSummary['by_category'],
      by_provider: byProvider,
      by_model: byModel,
    }
  }

  getUsageTrend(_granularity: string): UsageTrendPoint[] {
    // Simplified: group by day over past 30 days
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
          tokens += evt.tokens_in + evt.tokens_out
          cost += evt.estimated_cost ?? 0
        }
      }

      points.push({
        timestamp: new Date(dayEnd).toISOString().slice(0, 10),
        tokens,
        estimated_cost: Number(cost.toFixed(4)),
      })
    }

    return points
  }

  getUsageEvents(filters?: { provider_id?: string; model?: string }): UsageEvent[] {
    let events = this.usageEvents
    if (filters?.provider_id) {
      events = events.filter((e) => e.provider_id === filters.provider_id)
    }
    if (filters?.model) {
      events = events.filter((e) => e.model_name === filters.model)
    }
    return events
  }

  // ---- Models ----

  getLocalModels(): LocalModel[] {
    return this.localModels
  }

  // ---- Routing ----

  getLogicalModels(): LogicalModelConfig[] {
    return this.logicalModels
  }

  updateLogicalModel(name: string, config: LogicalModelConfig): void {
    const idx = this.logicalModels.findIndex((m) => m.name === name)
    if (idx >= 0) {
      this.logicalModels[idx] = config
    } else {
      this.logicalModels.push(config)
    }
    this.notify()
  }
}
