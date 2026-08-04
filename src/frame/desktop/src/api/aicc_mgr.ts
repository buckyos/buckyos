import { buckyos, getActiveSessionToken } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { MockDataStore } from '../app/ai-center/mock/store'
import type {
  AiProviderCard,
  AIStatus,
  ApiNamespace,
  ApiType,
  LocalModel,
  LogicalNode,
  ModelHealthStatus,
  ModelMetadata,
  PricingMode,
  ProviderConfig,
  ProviderInventory,
  ProviderOrigin,
  ProviderRuntimeType,
  ProviderStatus,
  ProviderType,
  ProviderView,
  RouteTrace,
  RoutePolicy,
  SchedulerProfile,
  GlobalRoutingView,
  StoreSnapshot,
  UsageSummary,
  UsageTrendPoint,
  ValidationResult,
  WizardDraft,
} from '../app/ai-center/mock/types'

export type {
  AIStatus,
  ApiNamespace,
  ApiType,
  AuthStatus,
  LocalModel,
  LogicalNode,
  ModelMetadata,
  ProviderView,
  ProtocolType,
  ProviderType,
  RoutePolicy,
  RouteTrace,
  GlobalRoutingView,
  StoreSnapshot,
  UsageEvent,
  UsageSummary,
  UsageTrendPoint,
  ValidationResult,
  WizardDraft,
} from '../app/ai-center/mock/types'

const EMPTY_USAGE_SUMMARY: UsageSummary = {
  total_tokens: 0,
  total_requests: 0,
  total_estimated_cost: 0,
  today_tokens: 0,
  this_month_tokens: 0,
  by_api_namespace: {
    llm: 0,
    embedding: 0,
    rerank: 0,
    image: 0,
    vision: 0,
    audio: 0,
    video: 0,
    agent: 0,
  },
  by_provider: {},
  by_model: {},
  by_app: {},
}

const EMPTY_SNAPSHOT: StoreSnapshot = {
  providers: [],
  usageEvents: [],
  routingView: {
    logical_tree: [],
    global_exact_model_weights: {},
    provider_weights: {},
    policy: {},
  },
  routeTraces: [],
  localModels: [],
  aiStatus: {
    state: 'disabled',
    provider_count: 0,
    model_count: 0,
    default_routing_ok: false,
    health_counts: {
      available: 0,
      degraded: 0,
      unavailable: 0,
    },
    quota_warnings: 0,
    inventory_ok: false,
  },
}

type Listener = () => void

type RawRecord = Record<string, unknown>

interface AiccRpcClient {
  call(method: string, params: Record<string, unknown>): Promise<unknown>
}

interface AccountInfo {
  session_token?: unknown
}

interface RawModelDirectory {
  providers?: RawProviderInventory[]
  directory?: Record<string, Record<string, RawLogicalRouteItem>>
  logical_definitions?: RawLogicalDefinition[]
  routing_settings?: RawRoutingSettings
  aliases?: unknown[]
}

interface RawProviderInventory {
  provider_instance_name?: unknown
  name?: unknown
  provider_type?: unknown
  provider_driver?: unknown
  provider_origin?: unknown
  provider_type_revision?: unknown
  version?: unknown
  inventory_revision?: unknown
  models?: RawModelMetadata[]
}

interface RawModelMetadata {
  provider_model_id?: unknown
  provider_actual_model_id?: unknown
  provider_options?: unknown
  exact_model?: unknown
  model_driver?: unknown
  parameter_scale?: unknown
  api_types?: unknown
  logical_mounts?: unknown
  capabilities?: unknown
  attributes?: unknown
  pricing?: unknown
  health?: unknown
  quota?: unknown
}

interface RawLogicalRouteItem {
  target?: unknown
  weight?: unknown
}

interface RawLogicalDefinition {
  path?: unknown
  api_type?: unknown
  min_line?: unknown
  fallback?: unknown
  route_policy?: unknown
  scheduler_profile?: unknown
}

interface RawRoutingSettings {
  global_exact_model_weights?: unknown
  provider_weights?: unknown
  policy?: unknown
  revision?: unknown
}

interface RawUsageAggregate {
  total_requests?: unknown
  input_tokens?: unknown
  output_tokens?: unknown
  total_tokens?: unknown
  request_units?: unknown
  finance_amount?: unknown
}

interface RawUsageGroupedRow {
  group?: Record<string, unknown>
  aggregate?: RawUsageAggregate
}

interface RawUsageBucketedRow {
  bucket_start_ms?: unknown
  group?: Record<string, unknown>
  aggregate?: RawUsageAggregate
}

interface RawUsageEvent {
  event_id?: unknown
  tenant_id?: unknown
  caller_app_id?: unknown
  task_id?: unknown
  capability?: unknown
  request_model?: unknown
  provider_model?: unknown
  input_tokens?: unknown
  output_tokens?: unknown
  total_tokens?: unknown
  request_units?: unknown
  finance_snapshot_json?: unknown
  created_at_ms?: unknown
}

interface RawUsageQueryResponse {
  total?: RawUsageAggregate
  grouped?: RawUsageGroupedRow[]
  buckets?: RawUsageBucketedRow[]
  events?: RawUsageEvent[]
  next_cursor?: unknown
}

interface RawTraceQueryResponse {
  traces?: unknown[]
  next_cursor?: unknown
  total_count?: unknown
  total?: unknown
}

interface AiccDataProvider {
  fetchSnapshot(): Promise<StoreSnapshot>
  addProvider(draft: WizardDraft): Promise<void>
  deleteProvider(id: string): Promise<void>
  refreshProviderModels(id: string): Promise<void>
  updateProviderKey(provider: ProviderView, apiKey: string): Promise<void>
  setProviderWeight(providerInstanceName: string, weight: number): Promise<void>
  validateConnection(draft: WizardDraft): Promise<ValidationResult>
  getUsageSummary(): UsageSummary
  getUsageTrend(granularity?: string): UsageTrendPoint[]
  queryUsageEvents(params: UsageEventsQuery): Promise<UsageEventsPage>
  queryRoutingDirectory(path: string | null): Promise<RoutingDirectoryView>
  queryRouteTraces(params: RouteTracesQuery): Promise<RouteTracesPage>
  getCloudUpdateSettings(): Promise<CloudUpdateSettings>
  setCloudUpdateSettings(settings: CloudUpdateSettingsUpdate): Promise<CloudUpdateSettings>
}

export interface AICCMgr {
  subscribe(listener: Listener): () => void
  getSnapshot(): StoreSnapshot
  getSnapshotVersion(): number
  refresh(): Promise<void>
  getUsageSummary(): UsageSummary
  getUsageTrend(granularity?: string): UsageTrendPoint[]
  addProvider(draft: WizardDraft): Promise<ProviderView>
  deleteProvider(id: string): Promise<void>
  refreshProviderModels(id: string): Promise<void>
  updateProviderKey(provider: ProviderView, apiKey: string): Promise<void>
  setProviderRoutingWeight(providerInstanceName: string, weight: number): Promise<void>
  validateConnection(draft: WizardDraft): Promise<ValidationResult>
  queryUsageEvents(params: UsageEventsQuery): Promise<UsageEventsPage>
  queryRoutingDirectory(path: string | null): Promise<RoutingDirectoryView>
  queryRouteTraces(params: RouteTracesQuery): Promise<RouteTracesPage>
  getCloudUpdateSettings(): Promise<CloudUpdateSettings>
  setCloudUpdateSettings(settings: CloudUpdateSettingsUpdate): Promise<CloudUpdateSettings>
}

export interface CloudUpdateSettings {
  enabled: boolean
  sourceUrl?: string
  sourceConfigured: boolean
  intervalSecs: number
  status: CloudUpdateStatus
  activeRevision?: number
  lastAttemptAtMs?: number
  lastSuccessAtMs?: number
  lastError?: string
  consecutiveFailures: number
}

export type CloudUpdateStatus = 'disabled' | 'idle' | 'updating' | 'healthy' | 'degraded' | 'error'

export interface CloudUpdateSettingsUpdate {
  enabled: boolean
  sourceUrl?: string
}

export interface UsageTimeRange {
  startTimeMs: number
  endTimeMs: number
}

export interface UsageEventsQuery {
  timeRange: UsageTimeRange
  filters?: {
    providerModels?: string[]
    providerModelQuery?: string
    providerInstanceNames?: string[]
    providerInstanceQuery?: string
    appIds?: string[]
    appQuery?: string
  }
  cursor?: string
  limit: number
}

export interface UsageEventsPage {
  events: StoreSnapshot['usageEvents']
  totalRequests: number
  nextCursor?: string
}

export interface RoutingDirectoryView {
  routingView: GlobalRoutingView
  models: ModelMetadata[]
}

export interface RouteTracesQuery {
  cursor?: string
  limit: number
  taskIds?: string[]
  requestIds?: string[]
  timeRange?: UsageTimeRange
  query?: string
  outcome?: 'fallback' | 'failed' | 'warning'
  apiTypes?: string[]
  providerInstanceNames?: string[]
  selectedExactModels?: string[]
  schedulerProfiles?: string[]
}

export interface RouteTracesPage {
  traces: RouteTrace[]
  nextCursor?: string
  totalCount?: number
}

export class AICCModelStore implements AICCMgr {
  private readonly provider: AiccDataProvider
  private snapshot: StoreSnapshot
  private snapshotVersion = 0
  private listeners = new Set<Listener>()

  constructor(provider: AiccDataProvider, initialSnapshot = EMPTY_SNAPSHOT) {
    this.provider = provider
    this.snapshot = initialSnapshot
  }

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener)
    return () => this.listeners.delete(listener)
  }

  getSnapshot = (): StoreSnapshot => this.snapshot

  getSnapshotVersion = (): number => this.snapshotVersion

  async refresh(): Promise<void> {
    this.snapshot = await this.provider.fetchSnapshot()
    this.snapshotVersion++
    this.emit()
  }

  getUsageSummary(): UsageSummary {
    return this.provider.getUsageSummary()
  }

  getUsageTrend(granularity = 'day'): UsageTrendPoint[] {
    return this.provider.getUsageTrend(granularity)
  }

  async addProvider(draft: WizardDraft): Promise<ProviderView> {
    const nextDraft = withProviderInstanceName(draft, this.snapshot)
    await this.provider.addProvider(nextDraft)
    const providerInstanceName = nextDraft.provider_instance_name
    for (let attempt = 0; attempt < 5; attempt += 1) {
      await this.refresh()
      const provider = this.snapshot.providers.find((item) =>
        item.config.provider_instance_name === providerInstanceName,
      )
      if (provider) {
        return provider
      }
      await delay(250)
    }
    throw new Error('aicc.provider_add_not_reflected')
  }

  async deleteProvider(id: string): Promise<void> {
    await this.provider.deleteProvider(id)
    await this.refresh()
  }

  async refreshProviderModels(id: string): Promise<void> {
    await this.provider.refreshProviderModels(id)
    await this.refresh()
  }

  async updateProviderKey(provider: ProviderView, apiKey: string): Promise<void> {
    await this.provider.updateProviderKey(provider, apiKey)
    await this.refresh()
  }

  async setProviderRoutingWeight(providerInstanceName: string, weight: number): Promise<void> {
    await this.provider.setProviderWeight(providerInstanceName, weight)
    await this.refresh()
  }

  validateConnection(draft: WizardDraft): Promise<ValidationResult> {
    return this.provider.validateConnection(draft)
  }

  queryUsageEvents(params: UsageEventsQuery): Promise<UsageEventsPage> {
    return this.provider.queryUsageEvents(params)
  }

  queryRoutingDirectory(path: string | null): Promise<RoutingDirectoryView> {
    return this.provider.queryRoutingDirectory(path)
  }

  queryRouteTraces(params: RouteTracesQuery): Promise<RouteTracesPage> {
    return this.provider.queryRouteTraces(params)
  }

  getCloudUpdateSettings(): Promise<CloudUpdateSettings> {
    return this.provider.getCloudUpdateSettings()
  }

  setCloudUpdateSettings(settings: CloudUpdateSettingsUpdate): Promise<CloudUpdateSettings> {
    return this.provider.setCloudUpdateSettings(settings)
  }

  private emit() {
    this.listeners.forEach((listener) => listener())
  }
}

export function createAICCMgr(options: { useMock?: boolean } = {}): AICCMgr {
  const useMock = options.useMock ?? isMockRuntime()
  if (useMock) {
    const provider = new MockAiccProvider()
    return new AICCModelStore(provider, provider.fetchSnapshotSync())
  }
  return new AICCModelStore(new BuckyOSAiccProvider())
}

class MockAiccProvider implements AiccDataProvider {
  private readonly store = new MockDataStore()
  private cloudUpdateSettings: CloudUpdateSettings = {
    enabled: false,
    sourceConfigured: false,
    intervalSecs: 3600,
    status: 'disabled',
    consecutiveFailures: 0,
  }

  fetchSnapshotSync(): StoreSnapshot {
    return this.store.getSnapshot()
  }

  async fetchSnapshot(): Promise<StoreSnapshot> {
    return this.store.getSnapshot()
  }

  async addProvider(draft: WizardDraft): Promise<void> {
    this.store.addProvider(draft)
  }

  async deleteProvider(id: string): Promise<void> {
    this.store.deleteProvider(id)
  }

  async refreshProviderModels(id: string): Promise<void> {
    this.store.refreshProviderModels(id)
  }

  async updateProviderKey(provider: ProviderView): Promise<void> {
    this.store.updateProviderKey(provider.config.id)
  }

  async setProviderWeight(providerInstanceName: string, weight: number): Promise<void> {
    this.store.setProviderWeight(providerInstanceName, weight)
  }

  async validateConnection(draft: WizardDraft): Promise<ValidationResult> {
    return this.store.validateConnection(draft)
  }

  getUsageSummary(): UsageSummary {
    return this.store.getUsageSummary()
  }

  getUsageTrend(granularity = 'day'): UsageTrendPoint[] {
    return this.store.getUsageTrend(granularity)
  }

  async queryUsageEvents(params: UsageEventsQuery): Promise<UsageEventsPage> {
    const events = this.store.getSnapshot().usageEvents
    const filtered = events
      .filter((event) => usageEventMatchesQuery(event, params))
      .sort((left, right) => new Date(right.timestamp).getTime() - new Date(left.timestamp).getTime())
    const cursor = Number(params.cursor ?? 0)
    const offset = Number.isFinite(cursor) && cursor > 0 ? cursor : 0
    const page = filtered.slice(offset, offset + params.limit)
    const nextOffset = offset + page.length
    return {
      events: page,
      totalRequests: filtered.length,
      nextCursor: nextOffset < filtered.length ? nextOffset.toString() : undefined,
    }
  }

  async queryRouteTraces(params: RouteTracesQuery): Promise<RouteTracesPage> {
    const traces = this.store.getSnapshot().routeTraces
      .filter((trace) => routeTraceMatchesQuery(trace, params))
    const cursor = Number(params.cursor ?? 0)
    const offset = Number.isFinite(cursor) && cursor > 0 ? cursor : 0
    const page = traces.slice(offset, offset + params.limit)
    const nextOffset = offset + page.length
    return {
      traces: page,
      nextCursor: nextOffset < traces.length ? nextOffset.toString() : undefined,
      totalCount: traces.length,
    }
  }

  async queryRoutingDirectory(path: string | null): Promise<RoutingDirectoryView> {
    const snapshot = this.store.getSnapshot()
    return {
      routingView: {
        ...snapshot.routingView,
        logical_tree: path
          ? childLogicalNodes(snapshot.routingView.logical_tree, path)
          : snapshot.routingView.logical_tree,
      },
      models: [
        ...snapshot.providers.flatMap((provider) => provider.status.discovered_models),
        ...snapshot.localModels,
      ],
    }
  }

  async getCloudUpdateSettings(): Promise<CloudUpdateSettings> {
    return this.cloudUpdateSettings
  }

  async setCloudUpdateSettings(settings: CloudUpdateSettingsUpdate): Promise<CloudUpdateSettings> {
    this.cloudUpdateSettings = {
      enabled: settings.enabled,
      sourceUrl: settings.sourceUrl ?? this.cloudUpdateSettings.sourceUrl,
      sourceConfigured: Boolean(settings.sourceUrl ?? this.cloudUpdateSettings.sourceUrl),
      intervalSecs: this.cloudUpdateSettings.intervalSecs,
      status: settings.enabled ? 'healthy' : 'disabled',
      lastAttemptAtMs: Date.now(),
      lastSuccessAtMs: settings.enabled ? Date.now() : this.cloudUpdateSettings.lastSuccessAtMs,
      consecutiveFailures: 0,
    }
    return this.cloudUpdateSettings
  }
}

class BuckyOSAiccProvider implements AiccDataProvider {
  private client: AiccRpcClient | null = null
  private controlPanelClient: AiccRpcClient | null = null
  private usageSummary = EMPTY_USAGE_SUMMARY
  private usageTrend: UsageTrendPoint[] = []

  async fetchSnapshot(): Promise<StoreSnapshot> {
    const dashboardRange = localTrailingDaysRange(30)
    const todayRange = localTodayRange()
    const monthRange = localCurrentMonthRange()
    const [directory, usageByModel, usageByCapability, usageByApp, usageTrend, usageToday, usageThisMonth, traceQuery] = await Promise.all([
      this.call<RawModelDirectory>('models.list', {}),
      this.queryUsage({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        group_by: ['provider_model'],
        output_mode: 'summary',
      }),
      this.queryUsage({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        group_by: ['capability'],
        output_mode: 'summary',
      }),
      this.queryUsage({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        group_by: ['caller_app_id'],
        output_mode: 'summary',
      }),
      this.queryUsage({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        time_bucket: 'day',
        output_mode: 'summary',
      }),
      this.queryUsage({
        time_range: toRawTimeRange(todayRange),
        filters: {},
        output_mode: 'summary',
      }),
      this.queryUsage({
        time_range: toRawTimeRange(monthRange),
        filters: {},
        output_mode: 'summary',
      }),
      this.queryRouteTraces({ limit: 20 }),
    ])
    this.usageSummary = toUsageSummary({
      byModel: usageByModel,
      byCapability: usageByCapability,
      byApp: usageByApp,
      today: usageToday,
      thisMonth: usageThisMonth,
    })
    this.usageTrend = toUsageTrend(usageTrend)
    const rawProviders = Array.isArray(directory.providers) ? directory.providers : []
    return toStoreSnapshot(directory, rawProviders, [], traceQuery.traces)
  }

  async addProvider(draft: WizardDraft): Promise<void> {
    const result = await this.call<{
      ok?: unknown
      reason?: unknown
      error?: unknown
      reload?: { ok?: unknown; error?: unknown; reason?: unknown }
    }>('provider.add', toProviderWritePayload(draft), { requireSession: true })
    if (result.ok !== true) {
      throw new Error(asNonEmptyString(result.reason, asNonEmptyString(result.error, 'aicc.provider_add_failed')))
    }
    if (result.reload?.ok === false) {
      throw new Error(asNonEmptyString(
        result.reload.error,
        asNonEmptyString(result.reload.reason, 'aicc.provider_reload_failed'),
      ))
    }
  }

  async deleteProvider(id: string): Promise<void> {
    const result = await this.call<{ ok?: unknown; reason?: unknown }>('provider.delete', {
      provider_instance_name: id,
    }, { requireSession: true })
    if (result.ok === false) {
      throw new Error(asNonEmptyString(result.reason, 'aicc.provider_delete_failed'))
    }
  }

  async refreshProviderModels(id: string): Promise<void> {
    await this.call('provider.refresh_models', {
      provider_instance_name: id,
    }, { requireSession: true })
  }

  async updateProviderKey(provider: ProviderView, apiKey: string): Promise<void> {
    const result = await this.callControlPanel<{ provider?: unknown; ok?: unknown; reason?: unknown }>('ai.provider.set', {
      provider: toAiProviderCard(provider),
      api_key: apiKey.trim(),
    }, { requireSession: true })
    if (result.ok === false) {
      throw new Error(asNonEmptyString(result.reason, 'aicc.provider_key_update_failed'))
    }
    const reload = await this.callControlPanel<{ ok?: unknown; result?: { ok?: unknown }; reason?: unknown }>('ai.reload', {}, { requireSession: true })
    if (reload.ok === false || reload.result?.ok === false) {
      throw new Error(asNonEmptyString(reload.reason, 'aicc.provider_reload_failed'))
    }
    await this.refreshProviderModels(provider.config.id)
  }

  async setProviderWeight(providerInstanceName: string, weight: number): Promise<void> {
    const result = await this.callControlPanel<{ ok?: unknown; reason?: unknown }>('ai.provider.weight.set', {
      provider_instance_name: providerInstanceName,
      weight,
    }, { requireSession: true })
    if (result.ok !== true) {
      throw new Error(asNonEmptyString(result.reason, 'aicc.provider_weight_set_failed'))
    }
  }

  async validateConnection(draft: WizardDraft): Promise<ValidationResult> {
    const result = await this.call<Record<string, unknown>>('provider.validate', toProviderWritePayload(draft))
    return {
      endpoint_reachable: asBoolean(result.endpoint_reachable, false),
      auth_valid: asBoolean(result.auth_valid, false),
      models_discovered: toStringArray(result.models_discovered),
      balance_available: asBoolean(result.balance_available, false),
      errors: toStringArray(result.errors),
      error_details: toValidationErrorDetails(result.error_details, result.errors),
      validation_fingerprint: asOptionalString(result.validation_fingerprint),
      validation_ttl_ms: asOptionalNumber(result.validation_ttl_ms),
    }
  }

  getUsageSummary(): UsageSummary {
    return this.usageSummary
  }

  getUsageTrend(): UsageTrendPoint[] {
    return this.usageTrend
  }

  async queryUsageEvents(params: UsageEventsQuery): Promise<UsageEventsPage> {
    const response = await this.queryUsage({
      time_range: toRawTimeRange(params.timeRange),
      filters: toRawUsageFilters(params.filters),
      output_mode: 'events',
      limit: params.limit,
      cursor: params.cursor,
    })
    return {
      events: toUsageEvents(response),
      totalRequests: asNumber(response.total?.total_requests, 0),
      nextCursor: asOptionalString(response.next_cursor),
    }
  }

  async queryRoutingDirectory(path: string | null): Promise<RoutingDirectoryView> {
    const directory = await this.call<RawModelDirectory>('models.list', path ? { logical_path: path } : {})
    const rawProviders = Array.isArray(directory.providers) ? directory.providers : []
    const snapshot = toStoreSnapshot(directory, rawProviders, [])
    return {
      routingView: path
        ? {
          ...snapshot.routingView,
          logical_tree: childLogicalNodes(snapshot.routingView.logical_tree, path),
        }
        : snapshot.routingView,
      models: [
        ...snapshot.providers.flatMap((provider) => provider.status.discovered_models),
        ...snapshot.localModels,
      ],
    }
  }

  async getCloudUpdateSettings(): Promise<CloudUpdateSettings> {
    const result = await this.call<Record<string, unknown>>(
      'driver_metadata_update.get',
      {},
      { requireSession: true },
    )
    return toCloudUpdateSettings(result)
  }

  async setCloudUpdateSettings(settings: CloudUpdateSettingsUpdate): Promise<CloudUpdateSettings> {
    const result = await this.call<Record<string, unknown>>(
      'driver_metadata_update.set',
      {
        enabled: settings.enabled,
        source_url: settings.sourceUrl?.trim() || undefined,
      },
      { requireSession: true },
    )
    return cloudUpdateSettingsFromSetResult(result)
  }

  private async call<T>(
    method: string,
    params: Record<string, unknown>,
    options: { requireSession?: boolean } = {},
  ): Promise<T> {
    const result = await this.getClient().call(method, await prepareSessionToken(params, options.requireSession === true))
    if (!isRecord(result)) {
      throw new Error(`Invalid ${method} response`)
    }
    return result as T
  }

  private async callControlPanel<T>(
    method: string,
    params: Record<string, unknown>,
    options: { requireSession?: boolean } = {},
  ): Promise<T> {
    const result = await this.getControlPanelClient().call(method, await prepareSessionToken(params, options.requireSession === true))
    if (!isRecord(result)) {
      throw new Error(`Invalid ${method} response`)
    }
    return result as T
  }

  private async queryUsage(params: Record<string, unknown>): Promise<RawUsageQueryResponse> {
    try {
      return await this.call<RawUsageQueryResponse>('usage.query', params)
    } catch (error) {
      console.error('aicc.usage.query failed', error)
      return {}
    }
  }

  async queryRouteTraces(params: RouteTracesQuery): Promise<RouteTracesPage> {
    try {
      const raw = await this.call<RawTraceQueryResponse>('trace.query', {
        limit: params.limit,
        cursor: params.cursor,
        task_ids: params.taskIds,
        request_ids: params.requestIds,
        start_time_ms: params.timeRange?.startTimeMs,
        end_time_ms: params.timeRange?.endTimeMs,
        query: params.query?.trim() || undefined,
        outcome: params.outcome,
        api_types: params.apiTypes,
        provider_instance_names: params.providerInstanceNames,
        selected_exact_models: params.selectedExactModels,
        scheduler_profiles: params.schedulerProfiles,
      })
      const traces = toRouteTraces(raw)
      return {
        traces,
        nextCursor: asOptionalString(raw.next_cursor),
        totalCount: asOptionalNumber(raw.total_count) ?? asOptionalNumber(raw.total) ?? traces.length,
      }
    } catch (error) {
      console.error('aicc.trace.query failed', error)
      return { traces: [] }
    }
  }

  private getClient(): AiccRpcClient {
    if (!this.client) {
      this.client = buckyos.getServiceRpcClient('aicc') as unknown as AiccRpcClient
    }
    return this.client
  }

  private getControlPanelClient(): AiccRpcClient {
    if (!this.controlPanelClient) {
      this.controlPanelClient = buckyos.getServiceRpcClient('control-panel') as unknown as AiccRpcClient
    }
    return this.controlPanelClient
  }
}

async function prepareSessionToken(params: Record<string, unknown>, requireSession: boolean): Promise<Record<string, unknown>> {
  if (typeof params.session_token === 'string' && params.session_token.trim()) {
    return params
  }
  const accountInfo = await buckyos.getAccountInfo() as AccountInfo | null
  let sessionToken = typeof accountInfo?.session_token === 'string'
    ? accountInfo.session_token.trim()
    : ''
  if (!sessionToken) {
    const refreshedToken = await getActiveSessionToken()
    sessionToken = typeof refreshedToken === 'string' ? refreshedToken.trim() : ''
  }
  if (!sessionToken && requireSession) {
    throw new Error('Current login session expired. Please sign in again.')
  }
  return sessionToken ? { ...params, session_token: sessionToken } : params
}

function withProviderInstanceName(draft: WizardDraft, snapshot: StoreSnapshot): WizardDraft {
  if (draft.provider_instance_name?.trim()) return draft
  const providerType = draft.provider_type ?? 'custom'
  const base = defaultProviderInstanceName(providerType, draft.name)
  const used = new Set(snapshot.providers.map((provider) => provider.config.provider_instance_name))
  let candidate = base
  let suffix = 2
  while (used.has(candidate)) {
    candidate = `${base}-${suffix}`
    suffix += 1
  }
  return { ...draft, provider_instance_name: candidate }
}

function delay(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms))
}

function toCloudUpdateSettings(raw: Record<string, unknown>): CloudUpdateSettings {
  const status = asOptionalString(raw.status)
  return {
    enabled: asBoolean(raw.enabled, false),
    sourceUrl: asOptionalString(raw.source_url),
    sourceConfigured: asBoolean(raw.source_configured, false),
    intervalSecs: asOptionalNumber(raw.interval_secs) ?? 3600,
    status: isCloudUpdateStatus(status) ? status : 'disabled',
    activeRevision: asOptionalNumber(raw.active_revision),
    lastAttemptAtMs: asOptionalNumber(raw.last_attempt_at_ms),
    lastSuccessAtMs: asOptionalNumber(raw.last_success_at_ms),
    lastError: asOptionalString(raw.last_error),
    consecutiveFailures: asOptionalNumber(raw.consecutive_failures) ?? 0,
  }
}

function cloudUpdateSettingsFromSetResult(
  result: Record<string, unknown>,
): CloudUpdateSettings {
  if (result.ok !== true) {
    throw new Error(asNonEmptyString(result.error, asNonEmptyString(result.reason, 'aicc.cloud_update_save_failed')))
  }
  const runtimeApply = isRecord(result.runtime_apply) ? result.runtime_apply : {}
  if (runtimeApply.ok === false) {
    throw new Error(asNonEmptyString(runtimeApply.error, 'aicc.cloud_update_runtime_apply_failed'))
  }
  return toCloudUpdateSettings(isRecord(result.settings) ? result.settings : result)
}

function isCloudUpdateStatus(value: string | undefined): value is CloudUpdateStatus {
  return value === 'disabled'
    || value === 'idle'
    || value === 'updating'
    || value === 'healthy'
    || value === 'degraded'
    || value === 'error'
}

function defaultProviderInstanceName(providerType: ProviderType, name: string): string {
  switch (providerType) {
    case 'sn_router': return 'sn-ai-provider-main'
    case 'openai': return 'openai-main'
    case 'anthropic': return 'claude-main'
    case 'google': return 'google-gemini-main'
    case 'openrouter': return 'openrouter-main'
    case 'custom': return `custom-${slugify(name || 'provider')}`
    default: return `${slugify(providerType)}-main`
  }
}

function slugify(value: string): string {
  const slug = value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
  return slug || 'provider'
}

function defaultProviderEndpoint(providerType: ProviderType | null): string {
  switch (providerType) {
    case 'sn_router': return 'https://sn.buckyos.ai/api/v1/ai/'
    case 'openai': return 'https://api.openai.com/v1'
    case 'anthropic': return 'https://api.anthropic.com/v1'
    case 'google': return 'https://generativelanguage.googleapis.com/v1beta'
    case 'openrouter': return 'https://openrouter.ai/api/v1'
    default: return ''
  }
}

function toAiProviderCard(provider: ProviderView): AiProviderCard {
  const { config, status } = provider
  const defaultModel = status.discovered_models[0]?.provider_model_id ?? ''
  return {
    id: config.provider_instance_name,
    displayName: config.name,
    providerType: config.provider_type,
    status: status.auth_status === 'invalid'
      ? 'needs_setup'
      : status.model_sync_status === 'failed' || status.auth_status === 'expired'
        ? 'degraded'
        : 'healthy',
    endpoint: config.endpoint ?? '',
    authMode: config.auth_mode ?? 'api_key',
    credentialConfigured: true,
    availableModels: status.discovered_models.map((model) => model.provider_model_id),
    capabilities: Array.from(new Set(status.discovered_models.flatMap((model) => model.api_types))),
    defaultModel,
    note: '',
  }
}

function toProviderWritePayload(draft: WizardDraft): Record<string, unknown> {
  const providerType = draft.provider_type ?? 'custom'
  const endpoint = draft.endpoint.trim() || defaultProviderEndpoint(providerType)
  const instanceName = draft.provider_instance_name ?? defaultProviderInstanceName(providerType, draft.name)
  const providerName = draft.name.trim() || providerDisplayName(providerType, instanceName)
  return {
    provider_instance_name: instanceName,
    provider_type: providerType,
    name: providerName,
    endpoint,
    protocol_type: draft.protocol_type,
    api_key: draft.api_key,
    auto_sync_models: draft.auto_sync_models,
  }
}

function toUsageSummary(raw: {
  byModel: RawUsageQueryResponse
  byCapability: RawUsageQueryResponse
  byApp: RawUsageQueryResponse
  today: RawUsageQueryResponse
  thisMonth: RawUsageQueryResponse
}): UsageSummary {
  const total = raw.byModel.total ?? {}
  const byModel: Record<string, number> = {}
  const byProvider: Record<string, number> = {}
  for (const row of groupedRows(raw.byModel)) {
    const model = asNonEmptyString(row.group?.provider_model, '')
    if (!model) continue
    const tokens = aggregateTokens(row.aggregate)
    byModel[model] = tokens
    const provider = providerInstanceFromExactModel(model)
    if (provider) {
      byProvider[provider] = (byProvider[provider] ?? 0) + tokens
    }
  }

  const byApiNamespace = emptyApiNamespaceUsage()
  for (const row of groupedRows(raw.byCapability)) {
    const capability = asNonEmptyString(row.group?.capability, '')
    const namespace = capabilityToApiNamespace(capability)
    byApiNamespace[namespace] += aggregateTokens(row.aggregate)
  }

  const byApp: Record<string, number> = {}
  for (const row of groupedRows(raw.byApp)) {
    const app = asNonEmptyString(row.group?.caller_app_id, 'system')
    byApp[app || 'system'] = aggregateTokens(row.aggregate)
  }

  return {
    ...EMPTY_USAGE_SUMMARY,
    by_api_namespace: byApiNamespace,
    total_tokens: aggregateTokens(total),
    total_requests: asNumber(total.total_requests, 0),
    total_estimated_cost: aggregateFinanceAmount(total),
    today_tokens: aggregateTokens(raw.today.total),
    this_month_tokens: aggregateTokens(raw.thisMonth.total),
    by_provider: byProvider,
    by_model: byModel,
    by_app: byApp,
  }
}

function groupedRows(raw: RawUsageQueryResponse): RawUsageGroupedRow[] {
  return Array.isArray(raw.grouped) ? raw.grouped : []
}

function emptyApiNamespaceUsage(): Record<ApiNamespace, number> {
  return {
    llm: 0,
    embedding: 0,
    rerank: 0,
    image: 0,
    vision: 0,
    audio: 0,
    video: 0,
    agent: 0,
  }
}

function capabilityToApiNamespace(value: string): ApiNamespace {
  const lower = value.toLowerCase()
  if (lower.startsWith('embedding')) return 'embedding'
  if (lower.startsWith('rerank')) return 'rerank'
  if (lower.startsWith('image')) return 'image'
  if (lower.startsWith('vision')) return 'vision'
  if (lower.startsWith('audio')) return 'audio'
  if (lower.startsWith('video')) return 'video'
  if (lower.startsWith('agent')) return 'agent'
  return 'llm'
}

function providerInstanceFromExactModel(model: string): string | undefined {
  const at = model.lastIndexOf('@')
  if (at < 0 || at === model.length - 1) return undefined
  return model.slice(at + 1)
}

function toUsageTrend(raw: RawUsageQueryResponse): UsageTrendPoint[] {
  const buckets = Array.isArray(raw.buckets) ? raw.buckets : []
  return buckets.map((bucket) => ({
    timestamp: localDateKey(new Date(asNumber(bucket.bucket_start_ms, 0))),
    tokens: aggregateTokens(bucket.aggregate),
    estimated_cost: aggregateFinanceAmount(bucket.aggregate),
  }))
}

function toUsageEvents(raw: RawUsageQueryResponse): StoreSnapshot['usageEvents'] {
  const events = Array.isArray(raw.events) ? raw.events : []
  return events.map((event) => {
    const providerModel = asNonEmptyString(event.provider_model, '')
    const tokensIn = asOptionalNumber(event.input_tokens)
    const tokensOut = asOptionalNumber(event.output_tokens)
    const tokenEquivalent = asOptionalNumber(event.total_tokens)
      ?? asOptionalNumber(event.request_units)
      ?? ((tokensIn ?? 0) + (tokensOut ?? 0))

    return {
      id: asNonEmptyString(event.event_id, asNonEmptyString(event.task_id, `usage-${asNumber(event.created_at_ms, 0)}`)),
      timestamp: new Date(asNumber(event.created_at_ms, 0)).toISOString(),
      provider_instance_name: providerInstanceFromExactModel(providerModel) ?? 'unknown-provider',
      exact_model: providerModel || 'unknown-model',
      requested_model: asNonEmptyString(event.request_model, providerModel || 'unknown-model'),
      api_type: capabilityToApiType(asNonEmptyString(event.capability, 'llm')),
      tenant_id: asNonEmptyString(event.tenant_id, 'default'),
      app_id: asOptionalString(event.caller_app_id),
      session_id: asOptionalString(event.task_id),
      tokens_in: tokensIn,
      tokens_out: tokensOut,
      token_equivalent: tokenEquivalent,
      estimated_cost: estimatedCost(event.finance_snapshot_json),
      finance_snapshot: toUsageFinanceSnapshot(event.finance_snapshot_json),
      status: 'success',
    }
  })
}

function capabilityToApiType(value: string): ApiType {
  const inferred = inferApiType(value)
  if (inferred) return inferred
  switch (capabilityToApiNamespace(value)) {
    case 'embedding': return 'embedding.text'
    case 'rerank': return 'rerank'
    case 'image': return 'image.txt2img'
    case 'vision': return 'vision.ocr'
    case 'audio': return 'audio.tts'
    case 'video': return 'video.txt2video'
    case 'agent': return 'agent.computer_use'
    case 'llm':
    default: return 'llm'
  }
}

function estimatedCost(value: unknown): number | undefined {
  return toUsageFinanceSnapshot(value)?.amount
}

function toUsageFinanceSnapshot(value: unknown): StoreSnapshot['usageEvents'][number]['finance_snapshot'] {
  const snapshot = asRecord(value)
  const amount = asOptionalNumber(snapshot.amount)
  const currency = asOptionalString(snapshot.currency)
  const providerTraceId = asOptionalString(snapshot.provider_trace_id)

  if (amount == null && currency == null && providerTraceId == null && snapshot.billing == null) {
    return undefined
  }

  return {
    amount,
    currency,
    provider_trace_id: providerTraceId,
    billing: snapshot.billing,
  }
}

function toRouteTraces(raw: RawTraceQueryResponse): RouteTrace[] {
  const traces = Array.isArray(raw.traces) ? raw.traces : []
  return traces
    .map((trace, index) => toRouteTrace(trace, index))
    .filter((trace): trace is RouteTrace => trace !== null)
}

function toRouteTrace(value: unknown, index: number): RouteTrace | null {
  const trace = asRecord(value)
  const requestedModel = asOptionalString(trace.requested_model)
  if (!requestedModel) return null
  const selectedExactModel = asOptionalString(trace.selected_exact_model)
  const rankedCandidates = toRankedCandidates(trace.ranked_candidates)
  const selectedPricingSnapshot = toRoutePricingSnapshot(trace.pricing_snapshot ?? trace.pricing)
    ?? rankedCandidates.find((candidate) => candidate.selected)?.pricing_snapshot
    ?? rankedCandidates.find((candidate) => candidate.exact_model === selectedExactModel)?.pricing_snapshot
  return {
    request_id: asNonEmptyString(trace.request_id, `route-trace-${index}`),
    session_id: asOptionalString(trace.session_id),
    api_type: normalizeApiType(trace.api_type),
    requested_model: requestedModel,
    requested_model_type: trace.requested_model_type === 'exact' ? 'exact' : 'logical',
    resolved_logical_path: asOptionalString(trace.resolved_logical_path),
    selected_exact_model: selectedExactModel,
    selected_provider_instance_name: asOptionalString(trace.selected_provider_instance_name)
      ?? (selectedExactModel ? providerInstanceFromExactModel(selectedExactModel) : undefined),
    selected_provider_model_id: asOptionalString(trace.selected_provider_model_id),
    provider_trace_id: asOptionalString(trace.provider_trace_id),
    pricing_snapshot: selectedPricingSnapshot,
    created_at_ms: asOptionalNumber(trace.created_at_ms),
    latency_ms: asOptionalNumber(trace.latency_ms),
    duration_ms: asOptionalNumber(trace.duration_ms),
    ranked_candidates: rankedCandidates,
    filtered_candidates: toFilteredCandidates(trace.filtered_candidates),
    fallback_applied: asBoolean(trace.fallback_applied, false),
    fallback_chain: toFallbackChain(trace.fallback_chain),
    session_sticky_hit: asBoolean(trace.session_sticky_hit, false),
    scheduler_profile: normalizeSchedulerProfile(trace.scheduler_profile) ?? 'balanced',
    runtime_failover_count: asNumber(trace.runtime_failover_count, 0),
    user_summary: toRouteUserSummary(trace.user_summary),
    warnings: toStringArray(trace.warnings),
  }
}

function toRoutePricingSnapshot(value: unknown): RouteTrace['pricing_snapshot'] {
  const source = asRecord(value)
  const pricing = Object.keys(source).length > 0 ? source : asRecord(asRecord(value).pricing)
  const snapshot = {
    input_token_usd: asOptionalNumber(pricing.input_token_usd) ?? asOptionalNumber(pricing.input),
    output_token_usd: asOptionalNumber(pricing.output_token_usd) ?? asOptionalNumber(pricing.output),
    cache_input_token_usd: asOptionalNumber(pricing.cache_input_token_usd) ?? asOptionalNumber(pricing.cache_input),
    estimated_cost_usd: asOptionalNumber(pricing.estimated_cost_usd) ?? asOptionalNumber(pricing.cost),
  }
  if (
    snapshot.input_token_usd == null &&
    snapshot.output_token_usd == null &&
    snapshot.cache_input_token_usd == null &&
    snapshot.estimated_cost_usd == null
  ) {
    return undefined
  }
  return snapshot
}

function toRankedCandidates(value: unknown): RouteTrace['ranked_candidates'] {
  return Array.isArray(value)
    ? value.map((item) => {
      const candidate = asRecord(item)
      return {
        exact_model: asNonEmptyString(candidate.exact_model, 'unknown-model'),
        final_score: asOptionalNumber(candidate.final_score),
        selected: asBoolean(candidate.selected, false),
        pricing_snapshot: toRoutePricingSnapshot(candidate.pricing_snapshot ?? candidate.pricing),
        exact_model_weight: asOptionalNumber(candidate.exact_model_weight),
        provider_weight: asOptionalNumber(candidate.provider_weight),
        preference_score_inputs: toPreferenceScoreInputs(candidate.preference_score_inputs),
        score_inputs: toScoreInputs(candidate.score_inputs),
      }
    })
    : []
}

function toScoreInputs(value: unknown): RouteTrace['ranked_candidates'][number]['score_inputs'] {
  const inputs = asRecord(value)
  const cost = asOptionalNumber(inputs.cost)
  const latency = asOptionalNumber(inputs.latency)
  const reliability = asOptionalNumber(inputs.reliability)
  const quality = asOptionalNumber(inputs.quality)
  const preference = asOptionalNumber(inputs.preference)
  const cache = asOptionalNumber(inputs.cache)
  const local = asOptionalNumber(inputs.local)
  if (
    cost == null ||
    latency == null ||
    reliability == null ||
    quality == null ||
    preference == null ||
    cache == null ||
    local == null
  ) {
    return undefined
  }
  return {
    cost,
    latency,
    reliability,
    quality,
    preference,
    cache,
    local,
  }
}

function toPreferenceScoreInputs(value: unknown): RouteTrace['ranked_candidates'][number]['preference_score_inputs'] {
  const inputs = asRecord(value)
  const exactModelWeight = asOptionalNumber(inputs.exact_model_weight)
  const providerWeight = asOptionalNumber(inputs.provider_weight)
  const combinedWeight = asOptionalNumber(inputs.combined_weight)
  const preferencePenalty = asOptionalNumber(inputs.preference_penalty)
  if (
    exactModelWeight == null ||
    providerWeight == null ||
    combinedWeight == null ||
    preferencePenalty == null
  ) {
    return undefined
  }
  return {
    exact_model_weight: exactModelWeight,
    provider_weight: providerWeight,
    combined_weight: combinedWeight,
    preference_penalty: preferencePenalty,
    exact_model_weight_effect: asNonEmptyString(inputs.exact_model_weight_effect, 'neutral'),
    provider_weight_effect: asNonEmptyString(inputs.provider_weight_effect, 'neutral'),
  }
}

function toFilteredCandidates(value: unknown): RouteTrace['filtered_candidates'] {
  return Array.isArray(value)
    ? value.map((item) => {
      const candidate = asRecord(item)
      return {
        exact_model: asNonEmptyString(candidate.exact_model, 'unknown-model'),
        reason: asNonEmptyString(candidate.reason, 'filtered'),
      }
    })
    : []
}

function toFallbackChain(value: unknown): RouteTrace['fallback_chain'] {
  return Array.isArray(value)
    ? value.map((item) => {
      const fallback = asRecord(item)
      return {
        from: asNonEmptyString(fallback.from, ''),
        to: asNonEmptyString(fallback.to, ''),
        reason: asNonEmptyString(fallback.reason, 'fallback'),
      }
    })
    : []
}

function toRouteUserSummary(value: unknown): RouteTrace['user_summary'] {
  const summary = asRecord(value)
  const displayName = asOptionalString(summary.display_name)
  const modelFamily = asOptionalString(summary.model_family)
  const reasonShort = asOptionalString(summary.reason_short)
  if (!displayName || !modelFamily || !reasonShort) return undefined
  const providerOrigin = summary.provider_origin === 'local' || summary.provider_origin === 'proxy_unknown'
    ? summary.provider_origin
    : 'cloud'
  return {
    display_name: displayName,
    model_family: modelFamily,
    provider_origin: providerOrigin,
    reason_short: reasonShort,
    was_fallback: asBoolean(summary.was_fallback, false),
    was_failover: asBoolean(summary.was_failover, false),
  }
}

function normalizeApiType(value: unknown): ApiType {
  const apiType = asOptionalString(value)
  return apiType && API_TYPES.includes(apiType as ApiType) ? apiType as ApiType : 'llm'
}

function aggregateTokens(raw?: RawUsageAggregate): number {
  if (!raw) return 0
  return asNumber(raw.total_tokens, 0)
    || asNumber(raw.input_tokens, 0) + asNumber(raw.output_tokens, 0)
    || asNumber(raw.request_units, 0)
}

function aggregateFinanceAmount(raw?: RawUsageAggregate): number {
  return asNumber(raw?.finance_amount, 0)
}

function localDayStart(value = new Date()): Date {
  const result = new Date(value)
  result.setHours(0, 0, 0, 0)
  return result
}

function localMonthStart(value = new Date()): Date {
  const result = localDayStart(value)
  result.setDate(1)
  return result
}

function localTodayRange(): UsageTimeRange {
  return { startTimeMs: localDayStart().getTime(), endTimeMs: Date.now() }
}

function localCurrentMonthRange(): UsageTimeRange {
  return { startTimeMs: localMonthStart().getTime(), endTimeMs: Date.now() }
}

function localTrailingDaysRange(days: number): UsageTimeRange {
  const start = localDayStart()
  start.setDate(start.getDate() - Math.max(0, days - 1))
  return { startTimeMs: start.getTime(), endTimeMs: Date.now() }
}

function toRawTimeRange(range: UsageTimeRange): Record<string, unknown> {
  return {
    kind: 'explicit',
    start_time_ms: range.startTimeMs,
    end_time_ms: range.endTimeMs,
  }
}

function toRawUsageFilters(filters: UsageEventsQuery['filters'] = {}): Record<string, unknown> {
  const result: Record<string, unknown> = {}
  if (filters.providerModels?.length) {
    result.provider_models = filters.providerModels
  }
  if (filters.providerModelQuery?.trim()) {
    result.provider_model_query = filters.providerModelQuery.trim()
  }
  if (filters.providerInstanceNames?.length) {
    result.provider_instance_names = filters.providerInstanceNames
  }
  if (filters.providerInstanceQuery?.trim()) {
    result.provider_instance_query = filters.providerInstanceQuery.trim()
  }
  if (filters.appIds?.length) {
    result.caller_app_ids = filters.appIds
  }
  if (filters.appQuery?.trim()) {
    result.caller_app_query = filters.appQuery.trim()
  }
  return result
}

function usageEventMatchesQuery(event: StoreSnapshot['usageEvents'][number], params: UsageEventsQuery): boolean {
  const eventTime = new Date(event.timestamp).getTime()
  if (!Number.isFinite(eventTime)) return false
  if (eventTime < params.timeRange.startTimeMs || eventTime >= params.timeRange.endTimeMs) return false
  const filters = params.filters ?? {}
  if (filters.providerModels?.length && !filters.providerModels.includes(event.exact_model)) return false
  if (filters.providerModelQuery?.trim() && !includesFuzzy(event.exact_model, filters.providerModelQuery)) return false
  if (filters.providerInstanceNames?.length && !filters.providerInstanceNames.includes(event.provider_instance_name)) return false
  if (filters.providerInstanceQuery?.trim() && !includesFuzzy(event.provider_instance_name, filters.providerInstanceQuery)) return false
  const appValue = `${event.app_id ?? 'system'} ${event.agent_id ?? ''}`.trim()
  if (filters.appIds?.length && !filters.appIds.some((id) => id === (event.app_id ?? 'system') || id === event.agent_id)) return false
  if (filters.appQuery?.trim() && !includesFuzzy(appValue, filters.appQuery)) return false
  return true
}

function routeTraceMatchesQuery(trace: RouteTrace, params: RouteTracesQuery): boolean {
  const taskIds = params.taskIds?.filter(Boolean) ?? []
  const requestIds = params.requestIds?.filter(Boolean) ?? []
  const inTaskScope = taskIds.length === 0 && requestIds.length === 0 ? true : taskIds.includes(trace.request_id) ||
    (trace.session_id != null && taskIds.includes(trace.session_id)) ||
    requestIds.includes(trace.request_id)
  if (!inTaskScope) return false
  if (params.timeRange) {
    if (trace.created_at_ms == null) return false
    if (trace.created_at_ms < params.timeRange.startTimeMs) return false
    if (trace.created_at_ms > params.timeRange.endTimeMs) return false
  }
  if (params.outcome === 'fallback' && !trace.fallback_applied) return false
  if (params.outcome === 'failed' && trace.selected_exact_model) return false
  if (params.outcome === 'warning' && trace.warnings.length === 0) return false
  if (params.apiTypes?.length && !params.apiTypes.includes(trace.api_type)) return false
  if (
    params.providerInstanceNames?.length &&
    !params.providerInstanceNames.includes(trace.selected_provider_instance_name ?? '')
  ) return false
  if (
    params.selectedExactModels?.length &&
    !params.selectedExactModels.some((model) => model === trace.selected_exact_model || model === trace.requested_model)
  ) return false
  if (params.schedulerProfiles?.length && !params.schedulerProfiles.includes(trace.scheduler_profile)) return false
  if (params.query?.trim() && !routeTraceIncludesQuery(trace, params.query)) return false
  return true
}

function routeTraceIncludesQuery(trace: RouteTrace, query: string): boolean {
  return [
    trace.request_id,
    trace.requested_model,
    trace.resolved_logical_path ?? '',
    trace.selected_exact_model ?? '',
    trace.selected_provider_instance_name ?? '',
    trace.selected_provider_model_id ?? '',
    trace.provider_trace_id ?? '',
    trace.scheduler_profile,
    trace.user_summary?.reason_short ?? '',
    ...trace.warnings,
    ...trace.ranked_candidates.map((candidate) => candidate.exact_model),
    ...trace.filtered_candidates.flatMap((candidate) => [candidate.exact_model, candidate.reason]),
  ].join(' ').toLowerCase().includes(query.trim().toLowerCase())
}

function includesFuzzy(value: string, query: string): boolean {
  return value.toLowerCase().includes(query.trim().toLowerCase())
}

function localDateKey(value: Date): string {
  const year = value.getFullYear()
  const month = String(value.getMonth() + 1).padStart(2, '0')
  const day = String(value.getDate()).padStart(2, '0')
  return `${year}-${month}-${day}`
}

function childLogicalNodes(nodes: LogicalNode[], path: string): LogicalNode[] {
  return findLogicalNode(nodes, path)?.children ?? []
}

function findLogicalNode(nodes: LogicalNode[], path: string): LogicalNode | undefined {
  for (const node of nodes) {
    if (node.path === path) return node
    const child = findLogicalNode(node.children ?? [], path)
    if (child) return child
  }
  return undefined
}

function toStoreSnapshot(
  directory: RawModelDirectory,
  rawProviders: RawProviderInventory[],
  usageEvents: StoreSnapshot['usageEvents'] = [],
  routeTraces: RouteTrace[] = [],
): StoreSnapshot {
  const inventories = rawProviders.map(toProviderInventory)
  const cloudProviders = inventories
    .filter((inventory) => inventory.provider_type !== 'local_inference')
    .map(toProviderView)
  const localModels = inventories
    .filter((inventory) => inventory.provider_type === 'local_inference')
    .flatMap(toLocalModels)
  const models = [
    ...cloudProviders.flatMap((provider) => provider.status.discovered_models),
    ...localModels,
  ]
  const routingView = toGlobalRoutingView(
    directory.routing_settings,
    directory.directory,
    directory.logical_definitions,
    models,
  )

  return {
    providers: cloudProviders,
    usageEvents,
    routingView,
    routeTraces,
    localModels,
    aiStatus: computeAIStatus(cloudProviders, localModels, routingView),
  }
}

function toProviderInventory(raw: RawProviderInventory): ProviderInventory {
  const instanceName = asNonEmptyString(raw.provider_instance_name, 'unknown-provider')
  const runtimeType = normalizeRuntimeType(raw.provider_type)
  const origin = normalizeProviderOrigin(raw.provider_origin)
  const driver = asNonEmptyString(raw.provider_driver, inferDriverFromInstance(instanceName))

  return {
    provider_instance_name: instanceName,
    name: asOptionalString(raw.name),
    provider_type: runtimeType,
    provider_driver: driver,
    provider_origin: origin,
    version: asOptionalString(raw.version),
    inventory_revision: asOptionalString(raw.inventory_revision),
    models: Array.isArray(raw.models)
      ? raw.models.map((model) => toModelMetadata(model, runtimeType, driver))
      : [],
  }
}

function toProviderView(inventory: ProviderInventory): ProviderView {
  const providerId = inventory.provider_instance_name
  const providerType = inferProviderType(inventory.provider_driver, inventory.provider_instance_name)
  const models = inventory.models
  const hasUnavailable = models.some((model) => model.health.status === 'unavailable')

  const config: ProviderConfig = {
    id: providerId,
    name: inventory.name ?? providerDisplayName(providerType, inventory.provider_instance_name),
    provider_type: providerType,
    provider_instance_name: inventory.provider_instance_name,
    provider_runtime_type: inventory.provider_type,
    provider_driver: inventory.provider_driver,
    provider_origin: inventory.provider_origin,
    auto_sync_models: true,
    created_at: new Date(0).toISOString(),
  }

  const status: ProviderStatus = {
    provider_id: providerId,
    is_connected: models.length > 0 && !models.every((model) => model.health.status === 'unavailable'),
    auth_status: models.length === 0 ? 'unknown' : hasUnavailable ? 'unknown' : 'ok',
    usage_supported: false,
    balance_supported: providerType === 'sn_router',
    discovered_models: models,
    model_sync_status: models.length > 0 ? 'ok' : 'failed',
  }

  return {
    config,
    inventory,
    status,
    account: {
      provider_instance_name: inventory.provider_instance_name,
      usage_supported: false,
      cost_supported: providerType !== 'sn_router',
      balance_supported: providerType === 'sn_router',
      pricing_mode: providerType === 'sn_router' ? 'free_quota' : inferPricingMode(models),
      balance_unit: providerType === 'sn_router' ? 'credit' : undefined,
      balance_value: providerType === 'sn_router' ? 15 : undefined,
    },
  }
}

function toLocalModels(inventory: ProviderInventory): LocalModel[] {
  return inventory.models.map((model) => ({
    ...model,
    id: model.exact_model,
    name: model.provider_model_id,
    status: model.health.status === 'unavailable' ? 'error' : 'ready',
  }))
}

function toModelMetadata(
  raw: RawModelMetadata,
  providerRuntimeType: ProviderRuntimeType,
  providerDriver: string,
): ModelMetadata {
  const providerModelId = asNonEmptyString(raw.provider_model_id, 'unknown-model')
  const exactModel = asNonEmptyString(raw.exact_model, providerModelId)
  const apiTypes = toApiTypes(raw.api_types)
  const rawHealth = asRecord(raw.health)
  const status = normalizeHealthStatus(
    typeof raw.health === 'string' ? raw.health : rawHealth.status,
  )
  const quotaState = normalizeQuotaState(rawHealth.quota_state ?? raw.quota)
  const capabilities = asRecord(raw.capabilities)
  const attributes = asRecord(raw.attributes)
  const pricing = asRecord(raw.pricing)
  const isLocal = providerRuntimeType === 'local_inference'

  return {
    provider_model_id: providerModelId,
    provider_actual_model_id: asOptionalString(raw.provider_actual_model_id),
    provider_options: raw.provider_options,
    exact_model: exactModel,
    model_driver: asNonEmptyString(raw.model_driver, providerDriver),
    parameter_scale: asOptionalString(raw.parameter_scale),
    api_types: apiTypes,
    logical_mounts: toStringArray(raw.logical_mounts),
    capabilities: {
      streaming: asBoolean(capabilities.streaming, apiTypes.some((type) => type.startsWith('llm.'))),
      tool_call: asBoolean(capabilities.tool_call, false),
      json_schema: asBoolean(capabilities.json_schema, false),
      web_search: asBoolean(capabilities.web_search, false),
      vision: asBoolean(capabilities.vision, apiTypes.some((type) => type.startsWith('vision.'))),
      max_context_tokens: asOptionalNumber(capabilities.max_context_tokens),
      max_output_tokens: asOptionalNumber(capabilities.max_output_tokens),
    },
    attributes: {
      local: asBoolean(attributes.local, isLocal),
      privacy: normalizePrivacy(attributes.privacy, providerRuntimeType),
      quality_score: asOptionalNumber(attributes.quality_score),
      latency_class: normalizeLatencyClass(attributes.latency_class),
      cost_class: normalizeCostClass(attributes.cost_class),
      tier: normalizeTier(attributes.tier),
    },
    pricing: {
      input_token_usd: asOptionalNumber(pricing.input_token_usd),
      output_token_usd: asOptionalNumber(pricing.output_token_usd),
      cache_input_token_usd: asOptionalNumber(pricing.cache_input_token_usd),
    },
    health: {
      status,
      p50_latency_ms: asOptionalNumber(rawHealth.p50_latency_ms),
      p95_latency_ms: asOptionalNumber(rawHealth.p95_latency_ms),
      error_rate_5m: asOptionalNumber(rawHealth.error_rate_5m),
      quota_state: quotaState,
    },
  }
}

function toGlobalRoutingView(
  raw: RawRoutingSettings | undefined,
  directory: RawModelDirectory['directory'],
  logicalDefinitions: RawLogicalDefinition[] | undefined,
  models: ModelMetadata[],
): GlobalRoutingView {
  const modelIndex = buildModelIndex(models)
  const tree = logicalTreeFromDirectory(directory, logicalDefinitions, modelIndex)
  const logicalTree = tree.length > 0 ? tree : logicalTreeFromModels(models)

  return {
    logical_tree: logicalTree,
    global_exact_model_weights: asNumberRecord(raw?.global_exact_model_weights),
    provider_weights: asNumberRecord(raw?.provider_weights),
    policy: toRoutePolicy(raw?.policy),
    revision: asOptionalString(raw?.revision),
  }
}

function logicalTreeFromDirectory(
  directory: RawModelDirectory['directory'],
  logicalDefinitions: RawLogicalDefinition[] | undefined,
  modelIndex: ModelIndex,
): LogicalNode[] {
  if (!directory && !logicalDefinitions?.length) return []
  const rootNodes: LogicalNode[] = []
  const nodesByPath = new Map<string, LogicalNode>()
  const definitionsByPath = buildLogicalDefinitionIndex(logicalDefinitions)

  const ensureDirectoryNode = (path: string): LogicalNode => {
    const existing = nodesByPath.get(path)
    if (existing) return existing

    const parentPath = parentLogicalPath(path)
    const definition = definitionsByPath.get(path)
    const policy = toLogicalDefinitionPolicy(definition)
    const node: LogicalNode = {
      path,
      label: labelFromPath(path),
      level: 'L3',
      api_type: apiTypeFromLogicalDefinition(definition) ?? inferApiType(path),
      exact_model_weights: {},
      fallback: toFallback(definition?.fallback) ?? { mode: 'parent' },
      policy: {
        profile: 'balanced',
        allow_fallback: true,
        runtime_failover: true,
        ...policy,
      },
      resolved_exact_model: resolveModelForPath(path, modelIndex),
      children: [],
    }
    nodesByPath.set(path, node)

    if (parentPath) {
      const parent = ensureDirectoryNode(parentPath)
      parent.children = appendUniqueNode(parent.children ?? [], node)
    } else {
      rootNodes.push(node)
    }

    return node
  }

  const entriesByPath = new Map<string, Record<string, RawLogicalRouteItem>>()
  Object.entries(directory ?? {}).forEach(([path, items]) => {
    entriesByPath.set(path, items)
  })
  for (const path of definitionsByPath.keys()) {
    if (!entriesByPath.has(path)) {
      entriesByPath.set(path, {})
    }
  }

  const entries = Array.from(entriesByPath.entries()).sort(([left], [right]) => {
    const depthDiff = left.split('.').length - right.split('.').length
    return depthDiff !== 0 ? depthDiff : left.localeCompare(right)
  })

  for (const [path, items] of entries) {
    const node = ensureDirectoryNode(path)
    const definition = definitionsByPath.get(path)
    node.items = toLogicalItems(items)
    node.api_type = apiTypeFromLogicalDefinition(definition) ?? inferApiType(path)
    node.fallback = toFallback(definition?.fallback) ?? node.fallback
    node.policy = {
      ...node.policy,
      ...toLogicalDefinitionPolicy(definition),
    }
    node.resolved_exact_model = resolveModelForPath(path, modelIndex)
  }

  for (const [path] of entries) {
    const node = ensureDirectoryNode(path)
    const children = node.children ?? []
    const childPaths = new Set(children.map((child) => child.path))
    const apiType = node.api_type ?? inferApiType(path)

    for (const item of Object.values(node.items ?? {})) {
      if (childPaths.has(item.target)) continue
      children.push(toDirectoryTargetNode(item.target, apiType, modelIndex))
      childPaths.add(item.target)
    }

    for (const model of modelIndex.byMount.get(path) ?? []) {
      if (childPaths.has(model.exact_model)) continue
      children.push(toExactModelNode(model, apiType))
      childPaths.add(model.exact_model)
    }

    node.children = children.sort((left, right) => {
      if (left.level !== right.level) return left.level === 'L1' ? 1 : -1
      return left.path.localeCompare(right.path)
    })
  }

  return rootNodes.sort(compareLogicalRoot)
}

function logicalTreeFromModels(models: ModelMetadata[]): LogicalNode[] {
  const modelIndex = buildModelIndex(models)
  const mountPaths = Array.from(modelIndex.byMount.keys()).sort()
  return mountPaths.map((path) => {
    const apiType = inferApiType(path)
    return toMountNode(path, apiType, modelIndex.byMount.get(path) ?? [])
  })
}

function toMountNode(path: string, apiType: ApiType | undefined, models: ModelMetadata[]): LogicalNode {
  return {
    path,
    label: labelFromPath(path),
    level: 'L2',
    api_type: apiType,
    exact_model_weights: {},
    resolved_exact_model: models[0]?.exact_model,
    children: models.map((model) => toExactModelNode(model, apiType)),
  }
}

function toDirectoryTargetNode(
  target: string,
  apiType: ApiType | undefined,
  modelIndex: ModelIndex,
): LogicalNode {
  const exactModel = modelIndex.byExact.get(target)
  if (exactModel) return toExactModelNode(exactModel, apiType)
  if (target.includes('@')) {
    return {
      path: target,
      label: labelFromPath(target),
      level: 'L1',
      api_type: apiType,
      exact_model_weights: {},
      resolved_exact_model: target,
      locked: true,
    }
  }
  return toMountNode(target, apiType, modelIndex.byMount.get(target) ?? [])
}

function toExactModelNode(model: ModelMetadata, apiType: ApiType | undefined): LogicalNode {
  return {
    path: model.exact_model,
    label: model.provider_model_id,
    level: 'L1',
    api_type: apiType ?? model.api_types[0],
    exact_model_weights: {},
    resolved_exact_model: model.exact_model,
    locked: true,
  }
}

type ModelIndex = {
  byMount: Map<string, ModelMetadata[]>
  byExact: Map<string, ModelMetadata>
}

function buildModelIndex(models: ModelMetadata[]): ModelIndex {
  const byMount = new Map<string, ModelMetadata[]>()
  const byExact = new Map<string, ModelMetadata>()
  for (const model of models) {
    byExact.set(model.exact_model, model)
    for (const mount of model.logical_mounts) {
      const current = byMount.get(mount) ?? []
      current.push(model)
      byMount.set(mount, current)
    }
  }
  return { byMount, byExact }
}

function resolveModelForPath(path: string, modelIndex: ModelIndex): string | undefined {
  return modelIndex.byMount.get(path)?.[0]?.exact_model
}

function buildLogicalDefinitionIndex(
  definitions: RawLogicalDefinition[] | undefined,
): Map<string, RawLogicalDefinition> {
  const result = new Map<string, RawLogicalDefinition>()
  for (const definition of definitions ?? []) {
    const path = asOptionalString(definition.path)
    if (path) {
      result.set(path, definition)
    }
  }
  return result
}

function apiTypeFromLogicalDefinition(definition?: RawLogicalDefinition): ApiType | undefined {
  const value = asOptionalString(definition?.api_type)
  return value && API_TYPES.includes(value as ApiType) ? value as ApiType : undefined
}

function toLogicalDefinitionPolicy(definition?: RawLogicalDefinition): RoutePolicy {
  if (!definition) return {}
  const policy = toRoutePolicy(definition.route_policy)
  const profile = normalizeSchedulerProfile(definition.scheduler_profile) ?? policy.profile
  const required = requiredFeatures(asRecord(definition.min_line))
  return {
    ...policy,
    ...(profile ? { profile } : {}),
    ...(required.length > 0 ? { required_features: required } : {}),
  }
}

function parentLogicalPath(path: string): string | undefined {
  const index = path.lastIndexOf('.')
  return index > 0 ? path.slice(0, index) : undefined
}

function appendUniqueNode(nodes: LogicalNode[], next: LogicalNode): LogicalNode[] {
  if (nodes.some((node) => node.path === next.path)) return nodes
  return [...nodes, next]
}

function compareLogicalRoot(left: LogicalNode, right: LogicalNode): number {
  const leftIndex = LOGICAL_ROOT_ORDER.indexOf(left.path)
  const rightIndex = LOGICAL_ROOT_ORDER.indexOf(right.path)
  if (leftIndex !== rightIndex) {
    if (leftIndex === -1) return 1
    if (rightIndex === -1) return -1
    return leftIndex - rightIndex
  }
  return left.path.localeCompare(right.path)
}

function computeAIStatus(
  providers: ProviderView[],
  localModels: LocalModel[],
  routingView: GlobalRoutingView,
): AIStatus {
  const models = [
    ...providers.flatMap((provider) => provider.status.discovered_models),
    ...localModels,
  ]
  const cloudProviderCount = providers.filter((provider) =>
    provider.config.provider_runtime_type === 'cloud_api' ||
    provider.config.provider_runtime_type === 'proxy_unknown',
  ).length
  const healthCounts: Record<ModelHealthStatus, number> = {
    available: models.filter((model) => model.health.status === 'available').length,
    degraded: models.filter((model) => model.health.status === 'degraded').length,
    unavailable: models.filter((model) => model.health.status === 'unavailable').length,
  }

  return {
    state: cloudProviderCount === 0 && models.length === 0
      ? 'disabled'
      : cloudProviderCount <= 1
        ? 'single_provider'
        : 'multi_provider',
    provider_count: cloudProviderCount,
    model_count: models.length,
    default_routing_ok: routingView.logical_tree.length > 0,
    health_counts: healthCounts,
    quota_warnings: models.filter((model) =>
      model.health.quota_state === 'near_limit' ||
      model.health.quota_state === 'exhausted',
    ).length,
    inventory_ok: providers.every((provider) => provider.status.model_sync_status === 'ok'),
  }
}

function toLogicalItems(raw: unknown): Record<string, { target: string; weight: number }> {
  const items = asRecord(raw)
  return Object.fromEntries(Object.entries(items).map(([key, value]) => {
    const item = asRecord(value)
    return [
      key,
      {
        target: asNonEmptyString(item.target, key),
        weight: asNumber(item.weight, 1),
      },
    ]
  }))
}

function toRoutePolicy(raw: unknown): RoutePolicy {
  const value = asRecord(raw)
  const profileValue = lockedValue(value.profile)
  const required = asRecord(lockedValue(value.required_features))
  return {
    profile: normalizeSchedulerProfile(profileValue),
    local_only: asOptionalBoolean(lockedValue(value.local_only)),
    allow_fallback: asOptionalBoolean(lockedValue(value.allow_fallback)),
    runtime_failover: asOptionalBoolean(lockedValue(value.runtime_failover)),
    required_features: requiredFeatures(required),
    allowed_provider_instances: toStringArray(lockedValue(value.allowed_provider_instances)),
    blocked_provider_instances: toStringArray(lockedValue(value.blocked_provider_instances)),
    max_estimated_cost_usd: asOptionalNumber(lockedValue(value.max_estimated_cost_usd)),
  }
}

function toFallback(raw: unknown): LogicalNode['fallback'] {
  const value = asRecord(raw)
  const mode = asOptionalString(value.mode)
  if (
    mode === 'parent' ||
    mode === 'strict' ||
    mode === 'disabled' ||
    mode === 'target_logical' ||
    mode === 'target_exact'
  ) {
    return {
      mode,
      target: asOptionalString(value.target),
    }
  }
  return undefined
}

function requiredFeatures(raw: RawRecord): string[] {
  const features: string[] = []
  if (raw.streaming === true) features.push('streaming')
  if (raw.tool_call === true) features.push('tool_calling')
  if (raw.json_schema === true) features.push('json_output')
  if (raw.web_search === true) features.push('web_search')
  if (raw.vision === true) features.push('vision')
  return features
}

function inferProviderType(driver: string, instanceName: string): ProviderType {
  const value = `${driver} ${instanceName}`.toLowerCase()
  if (value.includes('sn')) return 'sn_router'
  if (value.includes('openai')) return 'openai'
  if (value.includes('anthropic') || value.includes('claude')) return 'anthropic'
  if (value.includes('google') || value.includes('gemini')) return 'google'
  if (value.includes('openrouter')) return 'openrouter'
  return 'custom'
}

function inferDriverFromInstance(instanceName: string): string {
  return instanceName.split(/[._-]/)[0] || 'custom'
}

function providerDisplayName(providerType: ProviderType, instanceName: string): string {
  if (providerType === 'sn_router') return 'SN Router'
  if (providerType === 'openai') return 'OpenAI'
  if (providerType === 'anthropic') return 'Anthropic'
  if (providerType === 'google') return 'Google'
  if (providerType === 'openrouter') return 'OpenRouter'
  return labelFromPath(instanceName)
}

function inferPricingMode(models: ModelMetadata[]): PricingMode {
  return models.some((model) =>
    model.pricing.input_token_usd != null ||
    model.pricing.output_token_usd != null,
  ) ? 'per_token' : 'unknown'
}

function inferApiType(path: string): ApiType | undefined {
  const exact = API_TYPES.find((type) => path === type)
  if (exact) return exact
  const known = API_TYPES.find((type) => path.startsWith(`${type}.`))
  if (known) return known
  if (path.startsWith('llm.')) return 'llm'
  if (path.startsWith('embedding.')) return 'embedding.text'
  if (path.startsWith('image.')) return 'image.txt2img'
  if (path.startsWith('vision.')) return 'vision.ocr'
  if (path.startsWith('audio.')) return 'audio.tts'
  if (path.startsWith('video.')) return 'video.txt2video'
  if (path.startsWith('agent.')) return 'agent.computer_use'
  return undefined
}

function labelFromPath(path: string): string {
  return path
    .split(/[.@_-]/)
    .filter(Boolean)
    .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
    .join(' ')
}

function lockedValue(value: unknown): unknown {
  const record = asRecord(value)
  return 'value' in record ? record.value : value
}

function normalizeRuntimeType(value: unknown): ProviderRuntimeType {
  if (value === 'local_inference' || value === 'cloud_api' || value === 'proxy_unknown') {
    return value
  }
  return 'proxy_unknown'
}

function normalizeProviderOrigin(value: unknown): ProviderOrigin {
  if (
    value === 'system_config' ||
    value === 'user_config' ||
    value === 'builtin' ||
    value === 'provider_claimed'
  ) {
    return value
  }
  return 'provider_claimed'
}

function normalizeHealthStatus(value: unknown): ModelHealthStatus {
  if (value === 'degraded' || value === 'unavailable') return value
  return 'available'
}

function normalizeQuotaState(value: unknown): ModelMetadata['health']['quota_state'] {
  if (value === 'normal' || value === 'near_limit' || value === 'exhausted') return value
  return 'unknown'
}

function normalizePrivacy(value: unknown, runtimeType: ProviderRuntimeType): ModelMetadata['attributes']['privacy'] {
  if (
    value === 'local' ||
    value === 'cloud' ||
    value === 'private_safe' ||
    value === 'public_cloud' ||
    value === 'unknown'
  ) {
    return value
  }
  if (runtimeType === 'local_inference') return 'local'
  if (runtimeType === 'proxy_unknown') return 'private_safe'
  return 'public_cloud'
}

function normalizeLatencyClass(value: unknown): ModelMetadata['attributes']['latency_class'] {
  if (value === 'fast' || value === 'normal' || value === 'slow') return value
  return 'unknown'
}

function normalizeCostClass(value: unknown): ModelMetadata['attributes']['cost_class'] {
  if (value === 'low' || value === 'medium' || value === 'high') return value
  return 'unknown'
}

function normalizeTier(value: unknown): ModelMetadata['attributes']['tier'] {
  if (value === 'flagship' || value === 'mid' || value === 'nano') return value
  return undefined
}

function normalizeSchedulerProfile(value: unknown): SchedulerProfile | undefined {
  if (
    value === 'cost_first' ||
    value === 'latency_first' ||
    value === 'quality_first' ||
    value === 'balanced' ||
    value === 'local_first' ||
    value === 'strict_local'
  ) {
    return value
  }
  return undefined
}

function toApiTypes(value: unknown): ApiType[] {
  const apiTypes = toStringArray(value).filter((item): item is ApiType =>
    API_TYPES.includes(item as ApiType),
  )
  return apiTypes.length > 0 ? apiTypes : ['llm']
}

function toStringArray(value: unknown): string[] {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === 'string' && item.trim().length > 0)
    : []
}

function toValidationErrorDetails(
  value: unknown,
  fallbackErrors: unknown,
): ValidationResult['error_details'] {
  if (Array.isArray(value)) {
    const details = value
      .map((item) => {
        const record = asRecord(item)
        const kind = asOptionalString(record.kind)
        const message = asOptionalString(record.message)
        if (
          !message ||
          (kind !== 'endpoint' && kind !== 'auth' && kind !== 'models')
        ) {
          return null
        }
        return { kind, message }
      })
      .filter((item): item is NonNullable<ValidationResult['error_details']>[number] => item !== null)
    if (details.length > 0) return details
  }

  return toStringArray(fallbackErrors).map((message) => {
    const lower = message.toLowerCase()
    const kind = lower.includes('api_key') || lower.includes('auth') || lower.includes('token')
      ? 'auth'
      : lower.includes('endpoint') || lower.includes('url') || lower.includes('connect')
        ? 'endpoint'
        : 'models'
    return { kind, message }
  })
}

function asNumberRecord(value: unknown): Record<string, number> {
  const record = asRecord(value)
  return Object.fromEntries(Object.entries(record)
    .map(([key, item]) => [key, asOptionalNumber(item)])
    .filter((entry): entry is [string, number] => entry[1] != null))
}

function asRecord(value: unknown): RawRecord {
  return isRecord(value) ? value : {}
}

function isRecord(value: unknown): value is RawRecord {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

function asNonEmptyString(value: unknown, fallback: string): string {
  return typeof value === 'string' && value.trim() ? value : fallback
}

function asOptionalString(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() ? value : undefined
}

function asNumber(value: unknown, fallback: number): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function asOptionalNumber(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) ? value : undefined
}

function asBoolean(value: unknown, fallback: boolean): boolean {
  return typeof value === 'boolean' ? value : fallback
}

function asOptionalBoolean(value: unknown): boolean | undefined {
  return typeof value === 'boolean' ? value : undefined
}

const API_TYPES: ApiType[] = [
  'llm',
  'embedding.text',
  'embedding.multimodal',
  'rerank',
  'image.txt2img',
  'image.img2img',
  'image.inpaint',
  'image.upscale',
  'image.bg_remove',
  'vision.ocr',
  'vision.caption',
  'vision.detect',
  'vision.segment',
  'audio.tts',
  'audio.asr',
  'audio.music',
  'audio.enhance',
  'video.txt2video',
  'video.img2video',
  'video.video2video',
  'video.extend',
  'video.upscale',
  'agent.computer_use',
]

const LOGICAL_ROOT_ORDER = ['llm', 'image', 'audio', 'video', 'embedding', 'rerank', 'agent', 'agent_runtime', 'multimodal']
