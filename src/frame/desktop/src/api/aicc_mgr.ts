import { buckyos, getActiveSessionToken } from 'buckyos'
import { isMockRuntime } from '../runtime'
import { MockDataStore } from '../app/ai-center/mock/store'
import { normalizeFinanceTotals } from '../app/ai-center/datamodel/transforms'
import { toAiccRpcCallOptions } from './aicc_rpc_options'
import type {
  AIStatus,
  ApiNamespace,
  ApiType,
  KnownProviderProfile,
  LocalModel,
  LogicalNode,
  ModelHealthStatus,
  ModelMetadata,
  PricingMode,
  ProviderConfig,
  ProviderSetupCatalog,
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
  KnownProviderProfile,
  LocalModel,
  LogicalNode,
  ModelMetadata,
  ProviderView,
  ProtocolFamilyOption,
  ProviderSetupCatalog,
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
  finance_totals: [],
  finance_complete: true,
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
  settingsRevision: 0,
}

const BUILTIN_PROVIDER_NAMES: Array<[ProviderType, string, string, string]> = [
  ['sn', 'SN Router', 'https://sn.buckyos.ai/api/v1/ai', 'sn-openai'],
  ['openai', 'OpenAI', 'https://api.openai.com/v1', 'openai-responses'],
  ['claude', 'Anthropic Claude', 'https://api.anthropic.com/v1', 'claude-messages'],
  ['gemini', 'Google Gemini', 'https://generativelanguage.googleapis.com/v1beta', 'gemini-interactions'],
  ['fal', 'fal', 'https://queue.fal.run', 'fal-queue'],
  ['openrouter', 'OpenRouter', 'https://openrouter.ai/api/v1', 'openrouter-responses'],
  ['minimax', 'MiniMax', 'https://api.minimax.io/anthropic', 'minimax-messages'],
  ['kimi', 'Moonshot Kimi', 'https://api.moonshot.ai/v1', 'kimi-chat'],
  ['glm', 'Z.ai GLM', 'https://api.z.ai/api/paas/v4', 'glm-chat'],
  ['deepseek', 'DeepSeek', 'https://api.deepseek.com', 'deepseek-responses'],
  ['doubao', '豆包（火山方舟）', 'https://ark.cn-beijing.volces.com/api/v3', 'doubao-responses'],
  ['qwen', 'Qwen（阿里云百炼）', 'https://{workspace}.{region}.maas.aliyuncs.com/compatible-mode/v1', 'qwen-responses'],
]

const PROVIDER_INFLUENCE_ORDER = new Map<ProviderType, number>(
  BUILTIN_PROVIDER_NAMES.map(([profile], index) => [profile, index]),
)

const MOCK_PROVIDER_SETUP_CATALOG: ProviderSetupCatalog = {
  catalog_revision: 1,
  providers: BUILTIN_PROVIDER_NAMES.map(([provider_profile_id, display_name, base_url, protocol_adapter_id]) => ({
    provider_profile_id,
    display_name,
    base_url,
    protocol_adapter_id,
    provider_rules_id: provider_profile_id,
    ui_hints: {},
    connection_fields: provider_profile_id === 'qwen'
      ? {
        region: { mode: 'optional', default_value: 'cn-beijing', allowed_values: ['cn-beijing', 'ap-southeast-1', 'us-east-1', 'eu-central-1', 'ap-northeast-1'] },
        workspace: { mode: 'required', allowed_values: [] },
      }
      : provider_profile_id === 'glm' || provider_profile_id === 'minimax'
        ? { region: { mode: 'optional', default_value: 'global', allowed_values: ['global', 'china'] } }
        : {},
  })),
  protocol_families: [
    { protocol_family_id: 'openai', display_name: 'OpenAI compatible' },
    { protocol_family_id: 'claude', display_name: 'Claude compatible' },
    { protocol_family_id: 'gemini', display_name: 'Gemini compatible' },
  ],
}

type Listener = () => void

type RawRecord = Record<string, unknown>

interface AiccRpcClient {
  call(
    method: string,
    params: Record<string, unknown>,
    options?: { sessionToken?: string | null },
  ): Promise<unknown>
}

interface AccountInfo {
  session_token?: unknown
}

interface RawModelDirectory {
  models?: RawModelMetadata[]
  directory?: Record<string, Record<string, RawLogicalRouteItem>>
  logical_definitions?: RawLogicalDefinition[]
  routing_settings?: RawRoutingSettings
  aliases?: unknown[]
}

interface RawModelMetadata {
  provider_instance_name?: unknown
  provider_profile_id?: unknown
  protocol_adapter_id?: unknown
  model_driver_id?: unknown
  origin_model_id?: unknown
  provider_model_id?: unknown
  provider_actual_model_id?: unknown
  provider_options?: unknown
  exact_model?: unknown
  variant?: unknown
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
  finance_totals?: unknown[]
  finance_complete?: unknown
}

interface RawProviderInstanceView {
  provider_instance_name?: unknown
  provider_type?: unknown
  provider_profile_id?: unknown
  protocol_adapter_id?: unknown
  base_url?: unknown
  enabled?: unknown
  auth?: unknown
  inventory?: unknown
  health?: unknown
}

interface RawProviderListResponse {
  providers?: RawProviderInstanceView[]
  settings_revision?: unknown
  inventory_revision?: unknown
}

interface RawProviderCatalogResponse {
  catalog_revision?: unknown
  providers?: unknown[]
}

interface RawProtocolAdapterListResponse {
  adapters?: unknown[]
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
  provider_instance_name?: unknown
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
  fetchProviderSetupCatalog(): Promise<ProviderSetupCatalog>
  addProvider(draft: WizardDraft): Promise<void>
  deleteProvider(id: string): Promise<void>
  refreshProviderModels(id: string): Promise<void>
  updateProviderKey(provider: ProviderView, apiKey: string): Promise<void>
  setProviderEnabled(provider: ProviderView, enabled: boolean): Promise<void>
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
  fetchProviderSetupCatalog(): Promise<ProviderSetupCatalog>
  getUsageSummary(): UsageSummary
  getUsageTrend(granularity?: string): UsageTrendPoint[]
  addProvider(draft: WizardDraft): Promise<ProviderView>
  deleteProvider(id: string): Promise<void>
  refreshProviderModels(id: string): Promise<void>
  updateProviderKey(provider: ProviderView, apiKey: string): Promise<void>
  setProviderEnabled(provider: ProviderView, enabled: boolean): Promise<void>
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
  intervalSecs?: number
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

export class SettingsRevisionConflictError extends Error {
  readonly expectedRevision?: number
  readonly actualRevision?: number

  constructor(expectedRevision?: number, actualRevision?: number) {
    super('settings_revision_conflict')
    this.name = 'SettingsRevisionConflictError'
    this.expectedRevision = expectedRevision
    this.actualRevision = actualRevision
  }
}

export function isSettingsRevisionConflict(error: unknown): error is SettingsRevisionConflictError {
  return error instanceof SettingsRevisionConflictError
    || (error instanceof Error && error.message.includes('settings_revision_conflict'))
}

export class AICCModelStore implements AICCMgr {
  private readonly provider: AiccDataProvider
  private snapshot: StoreSnapshot
  private snapshotVersion = 0
  private refreshSeq = 0
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
    const seq = ++this.refreshSeq
    const next = await this.provider.fetchSnapshot()
    // A newer refresh started while this one was in flight; drop the stale result.
    if (seq !== this.refreshSeq) return
    this.snapshot = next
    this.snapshotVersion++
    this.emit()
  }

  fetchProviderSetupCatalog(): Promise<ProviderSetupCatalog> {
    return this.provider.fetchProviderSetupCatalog()
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
    try {
      await this.provider.updateProviderKey(provider, apiKey)
    } catch (error) {
      if (isSettingsRevisionConflict(error)) await this.refresh()
      throw error
    }
    await this.refresh()
  }

  async setProviderEnabled(provider: ProviderView, enabled: boolean): Promise<void> {
    try {
      await this.provider.setProviderEnabled(provider, enabled)
    } catch (error) {
      if (isSettingsRevisionConflict(error)) await this.refresh()
      throw error
    }
    await this.refresh()
  }

  async setProviderRoutingWeight(providerInstanceName: string, weight: number): Promise<void> {
    try {
      await this.provider.setProviderWeight(providerInstanceName, weight)
    } catch (error) {
      if (isSettingsRevisionConflict(error)) await this.refresh()
      throw error
    }
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

  async fetchProviderSetupCatalog(): Promise<ProviderSetupCatalog> {
    return MOCK_PROVIDER_SETUP_CATALOG
  }

  async addProvider(draft: WizardDraft): Promise<void> {
    this.store.addProvider(draft)
  }

  async deleteProvider(id: string): Promise<void> {
    this.store.deleteProvider(id)
  }

  async refreshProviderModels(): Promise<void> {
    this.store.refreshProviderModels()
  }

  async updateProviderKey(provider: ProviderView): Promise<void> {
    this.store.updateProviderKey(provider.config.id)
  }

  async setProviderEnabled(provider: ProviderView, enabled: boolean): Promise<void> {
    this.store.setProviderEnabled(provider.config.id, enabled)
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

  getUsageTrend(): UsageTrendPoint[] {
    return this.store.getUsageTrend()
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
        ...snapshot.providers
          .filter((provider) => provider.config.enabled)
          .flatMap((provider) => provider.status.discovered_models),
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
  private usageSummary = EMPTY_USAGE_SUMMARY
  private usageTrend: UsageTrendPoint[] = []
  private settingsRevision = 0

  async fetchSnapshot(): Promise<StoreSnapshot> {
    const dashboardRange = localTrailingDaysRange(30)
    const todayRange = localTodayRange()
    const monthRange = localCurrentMonthRange()
    const [directory, providerList, routing, usageByModel, usageByCapability, usageByApp, usageTrend, usageToday, usageThisMonth, traceQuery] = await Promise.all([
      this.call<RawModelDirectory>('models.list', {}),
      this.call<RawProviderListResponse>('provider.list', {}),
      this.call<Record<string, unknown>>('routing.get', {}),
      this.queryUsage({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        group_by: ['provider_model'],
        output_mode: 'summary',
      }),
      this.queryUsageOrEmpty({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        group_by: ['capability'],
        output_mode: 'summary',
      }),
      this.queryUsageOrEmpty({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        group_by: ['caller_app_id'],
        output_mode: 'summary',
      }),
      this.queryUsageOrEmpty({
        time_range: toRawTimeRange(dashboardRange),
        filters: {},
        time_bucket: 'day',
        output_mode: 'summary',
      }),
      this.queryUsageOrEmpty({
        time_range: toRawTimeRange(todayRange),
        filters: {},
        output_mode: 'summary',
      }),
      this.queryUsageOrEmpty({
        time_range: toRawTimeRange(monthRange),
        filters: {},
        output_mode: 'summary',
      }),
      this.queryRouteTracesOrEmpty({ limit: 20 }),
    ])
    this.usageSummary = toUsageSummary({
      byModel: usageByModel,
      byCapability: usageByCapability,
      byApp: usageByApp,
      today: usageToday,
      thisMonth: usageThisMonth,
    })
    this.usageTrend = toUsageTrend(usageTrend)
    this.settingsRevision = asNumber(providerList.settings_revision, 0)
    directory.routing_settings = asRecord(routing.routing) as RawRoutingSettings
    return toStoreSnapshot(directory, providerList, [], traceQuery.traces)
  }

  async fetchProviderSetupCatalog(): Promise<ProviderSetupCatalog> {
    const [catalog, adapters] = await Promise.all([
      this.call<RawProviderCatalogResponse>('provider.catalog', {}),
      this.call<RawProtocolAdapterListResponse>('protocol_adapter.list', {}),
    ])
    return toProviderSetupCatalog(catalog, adapters)
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
    this.settingsRevision = asNumber((result as RawRecord).settings_revision, this.settingsRevision)
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
    const result = await this.call<{ ok?: unknown; reason?: unknown; error?: unknown }>('provider.refresh_models', {
      provider_instance_name: id,
    }, { requireSession: true })
    if (result.ok !== true) {
      throw new Error(asNonEmptyString(result.reason, asNonEmptyString(result.error, 'aicc.provider_refresh_models_failed')))
    }
  }

  async updateProviderKey(provider: ProviderView, apiKey: string): Promise<void> {
    const result = await this.callProviderUpdate({
      provider_instance_name: provider.config.provider_instance_name,
      settings_revision: this.settingsRevision,
      credential: toCredential(apiKey),
    })
    this.settingsRevision = asNumber(result.settings_revision, this.settingsRevision)
  }

  async setProviderEnabled(provider: ProviderView, enabled: boolean): Promise<void> {
    const result = await this.callProviderUpdate({
      provider_instance_name: provider.config.provider_instance_name,
      settings_revision: this.settingsRevision,
      enabled,
    })
    this.settingsRevision = asNumber(result.settings_revision, this.settingsRevision)
  }

  async setProviderWeight(providerInstanceName: string, weight: number): Promise<void> {
    const current = await this.call<Record<string, unknown>>('routing.get', {})
    const revision = asNumber(current.settings_revision, this.settingsRevision)
    const routing = asRecord(current.routing)
    const providerWeights = asNumberRecord(routing.provider_weights)
    if (weight === 1) delete providerWeights[providerInstanceName]
    else providerWeights[providerInstanceName] = weight
    const result = await this.callWithConflict<Record<string, unknown>>('routing.update', {
      settings_revision: revision,
      provider_weights: providerWeights,
    })
    this.settingsRevision = asNumber(result.settings_revision, revision)
  }

  async validateConnection(draft: WizardDraft): Promise<ValidationResult> {
    const result = await this.call<Record<string, unknown>>('provider.validate', toProviderWritePayload(draft))
    return {
      base_url_reachable: asBoolean(result.base_url_reachable, false),
      auth_valid: asBoolean(result.auth_valid, false),
      models_discovered: toStringArray(result.models_discovered),
      balance_available: asBoolean(result.balance_available, false),
      errors: toStringArray(result.errors),
      error_details: toValidationErrorDetails(result.error_details, result.errors),
      resolved_protocol_adapter_id: asOptionalString(result.resolved_protocol_adapter_id),
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
    const [directory, providerList, routing] = await Promise.all([
      this.call<RawModelDirectory>('models.list', {}),
      this.call<RawProviderListResponse>('provider.list', {}),
      this.call<Record<string, unknown>>('routing.get', {}),
    ])
    directory.routing_settings = asRecord(routing.routing) as RawRoutingSettings
    const snapshot = toStoreSnapshot(directory, providerList, [])
    return {
      routingView: path
        ? {
          ...snapshot.routingView,
          logical_tree: childLogicalNodes(snapshot.routingView.logical_tree, path),
        }
        : snapshot.routingView,
      models: [
        ...snapshot.providers
          .filter((provider) => provider.config.enabled)
          .flatMap((provider) => provider.status.discovered_models),
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
        interval_secs: settings.intervalSecs,
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
    const sessionToken = await prepareSessionToken(options.requireSession === true)
    const result = await this.getClient().call(method, params, toAiccRpcCallOptions(sessionToken))
    if (!isRecord(result)) {
      throw new Error(`Invalid ${method} response`)
    }
    return result as T
  }

  private async callProviderUpdate(params: Record<string, unknown>): Promise<Record<string, unknown>> {
    return this.callWithConflict('provider.update', params)
  }

  private async callWithConflict<T>(method: string, params: Record<string, unknown>): Promise<T> {
    try {
      return await this.call<T>(method, params, { requireSession: true })
    } catch (error) {
      const conflict = parseSettingsConflict(error)
      if (conflict) throw conflict
      throw error
    }
  }

  private queryUsage(params: Record<string, unknown>): Promise<RawUsageQueryResponse> {
    return this.call<RawUsageQueryResponse>('usage.query', params)
  }

  // Dashboard aggregates are best-effort: a failed slice must not block the whole snapshot.
  private async queryUsageOrEmpty(params: Record<string, unknown>): Promise<RawUsageQueryResponse> {
    try {
      return await this.queryUsage(params)
    } catch (error) {
      console.error('aicc.usage.query failed', error)
      return {}
    }
  }

  async queryRouteTraces(params: RouteTracesQuery): Promise<RouteTracesPage> {
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
  }

  // Same best-effort contract as the dashboard usage slices: a failed trace
  // query must not block the whole snapshot.
  private async queryRouteTracesOrEmpty(params: RouteTracesQuery): Promise<RouteTracesPage> {
    try {
      return await this.queryRouteTraces(params)
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

}

async function prepareSessionToken(requireSession: boolean): Promise<string | null> {
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
  return sessionToken || null
}

function withProviderInstanceName(draft: WizardDraft, snapshot: StoreSnapshot): WizardDraft {
  if (draft.provider_instance_name?.trim()) return draft
  const providerType = draft.provider_profile_id ?? 'custom'
  const base = defaultProviderInstanceName(providerType, draft.display_name)
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
    case 'sn': return 'sn-ai-provider-main'
    case 'openai': return 'openai-main'
    case 'claude': return 'claude-main'
    case 'gemini': return 'google-gemini-main'
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

function toProviderWritePayload(draft: WizardDraft): Record<string, unknown> {
  const providerType = draft.provider_profile_id ?? 'custom'
  return {
    provider_instance_name: draft.provider_instance_name ?? defaultProviderInstanceName(providerType, draft.display_name),
    provider_type: 'cloud_api',
    provider_profile_id: providerType,
    protocol_family_id: providerType === 'custom' ? draft.protocol_family_id : undefined,
    protocol_adapter_id: providerType === 'custom' ? undefined : draft.protocol_adapter_id,
    base_url: draft.base_url.trim(),
    credentials: toCredential(draft.api_key),
    region: draft.region?.trim() || undefined,
    workspace: draft.workspace?.trim() || undefined,
    account: draft.account?.trim() || undefined,
    auto_sync_models: draft.auto_sync_models,
  }
}

function toCredential(apiKey: string): Record<string, unknown> {
  return { api_token: { locked: apiKey.trim() } }
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
    finance_totals: normalizeFinanceTotals(total.finance_totals),
    finance_complete: asBoolean(total.finance_complete, true),
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
    finance_totals: normalizeFinanceTotals(bucket.aggregate?.finance_totals),
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
      provider_instance_name: asNonEmptyString(event.provider_instance_name, providerInstanceFromExactModel(providerModel) ?? 'unknown-provider'),
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
  const raw = asRecord(value)
  const traceEvent = asRecord(raw.trace)
  const embeddedTrace = asRecord(traceEvent.route_trace_json)
  const trace = Object.keys(traceEvent).length > 0
    ? { ...embeddedTrace, ...traceEvent, ...raw }
    : raw
  const requestedModel = asOptionalString(trace.requested_model)
  if (!requestedModel) return null
  const selectedExactModel = asOptionalString(trace.selected_exact_model)
    ?? asOptionalString(trace.final_model)
  const rankedCandidates = toRankedCandidates(trace.ranked_candidates)
  const selectedPricingSnapshot = toRoutePricingSnapshot(trace.pricing_snapshot ?? trace.pricing)
    ?? rankedCandidates.find((candidate) => candidate.selected)?.pricing_snapshot
    ?? rankedCandidates.find((candidate) => candidate.exact_model === selectedExactModel)?.pricing_snapshot
  return {
    request_id: asNonEmptyString(trace.request_id, asNonEmptyString(trace.trace_id, `route-trace-${index}`)),
    session_id: asOptionalString(trace.session_id ?? trace.task_id),
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
  providerList: RawProviderListResponse,
  usageEvents: StoreSnapshot['usageEvents'] = [],
  routeTraces: RouteTrace[] = [],
): StoreSnapshot {
  const rawModels = Array.isArray(directory.models) ? directory.models : []
  const cloudProviders = (Array.isArray(providerList.providers) ? providerList.providers : [])
    .map((provider) => toProviderView(provider, rawModels))
  const localModels: LocalModel[] = []
  const models = [
    ...cloudProviders
      .filter((provider) => provider.config.enabled)
      .flatMap((provider) => provider.status.discovered_models),
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
    settingsRevision: asNumber(providerList.settings_revision, 0),
  }
}

function toProviderView(raw: RawProviderInstanceView, allModels: RawModelMetadata[]): ProviderView {
  const providerId = asNonEmptyString(raw.provider_instance_name, 'unknown-provider')
  const profile = normalizeProviderType(raw.provider_profile_id)
  const adapter = asNonEmptyString(raw.protocol_adapter_id, 'unknown')
  const enabled = asBoolean(raw.enabled, false)
  const rawInventory = asRecord(raw.inventory)
  const rawHealth = asRecord(raw.health)
  const inventoryState = asNonEmptyString(rawInventory.state, enabled ? 'not_loaded' : 'disabled')
  const healthState = asNonEmptyString(rawHealth.state, enabled ? 'unknown' : 'disabled')
  const models = allModels
    .filter((model) => asOptionalString(model.provider_instance_name) === providerId)
    .map((model) => toModelMetadata(
      model,
      'cloud_api',
      profile,
      healthState === 'healthy' ? 'available' : healthState === 'degraded' ? 'degraded' : 'unavailable',
    ))
  const costSupported = models.some((model) =>
    model.pricing.input_token_usd != null ||
    model.pricing.output_token_usd != null ||
    model.pricing.cache_input_token_usd != null ||
    model.pricing.estimated_cost_usd != null,
  )

  const config: ProviderConfig = {
    id: providerId,
    name: providerDisplayName(profile, providerId),
    enabled,
    provider_type: profile,
    provider_instance_name: providerId,
    provider_runtime_type: normalizeRuntimeType(raw.provider_type),
    provider_profile_id: profile,
    protocol_adapter_id: adapter,
    provider_origin: 'system_config',
    auth_mode: normalizeAuthMode(asRecord(raw.auth).mode),
    credential_configured: asBoolean(asRecord(raw.auth).configured, false),
    base_url: asNonEmptyString(raw.base_url, ''),
    auto_sync_models: true,
    created_at: new Date(0).toISOString(),
  }

  const status: ProviderStatus = {
    provider_id: providerId,
    is_connected: enabled && (healthState === 'healthy' || healthState === 'degraded'),
    auth_status: !enabled || !config.credential_configured
      ? 'unknown'
      : healthState === 'unavailable'
        ? 'invalid'
        : healthState === 'healthy' || healthState === 'degraded'
          ? 'ok'
          : 'unknown',
    usage_supported: false,
    balance_supported: false,
    discovered_models: models,
    model_sync_status: inventoryState === 'loaded' ? 'ok' : inventoryState === 'not_loaded' ? 'failed' : 'ok',
    last_verified_at: timestampFromMs(asOptionalNumber(rawHealth.checked_at_ms)),
    last_model_sync_at: timestampFromMs(asOptionalNumber(rawInventory.updated_at_ms)),
  }

  return {
    config,
    inventory: {
      provider_instance_name: providerId,
      provider_type: config.provider_runtime_type,
      provider_profile_id: profile,
      protocol_adapter_id: adapter,
      provider_origin: config.provider_origin,
      inventory_revision: asOptionalString(rawInventory.revision),
      models,
    },
    status,
    account: {
      provider_instance_name: providerId,
      usage_supported: false,
      cost_supported: costSupported,
      balance_supported: false,
      pricing_mode: inferPricingMode(models),
    },
  }
}

function toModelMetadata(
  raw: RawModelMetadata,
  providerRuntimeType: ProviderRuntimeType,
  providerDriver: string,
  providerHealth: ModelHealthStatus,
): ModelMetadata {
  const providerModelId = asNonEmptyString(raw.provider_model_id, 'unknown-model')
  const exactModel = asNonEmptyString(raw.exact_model, providerModelId)
  const apiTypes = toApiTypes(raw.api_types)
  const rawHealth = asRecord(raw.health)
  const status = normalizeHealthStatus(
    typeof raw.health === 'string' ? raw.health : rawHealth.status,
    providerHealth,
  )
  const quotaState = normalizeQuotaState(rawHealth.quota_state ?? raw.quota)
  const capabilities = asRecord(raw.capabilities)
  const attributes = asRecord(raw.attributes)
  const pricing = asRecord(raw.pricing)
  const isLocal = providerRuntimeType === 'local_inference'

  return {
    provider_model_id: providerModelId,
    provider_actual_model_id: asOptionalString(raw.origin_model_id ?? raw.provider_actual_model_id),
    provider_options: raw.provider_options,
    exact_model: exactModel,
    variant: asOptionalString(raw.variant),
    model_driver: asNonEmptyString(raw.model_driver_id ?? raw.model_driver, providerDriver),
    parameter_scale: asOptionalString(raw.parameter_scale),
    api_types: apiTypes,
    logical_mounts: toStringArray(raw.logical_mounts),
    capabilities: {
      streaming: asBoolean(capabilities.streaming, apiTypes.some((type) => type.startsWith('llm.'))),
      tool_call: asBoolean(capabilities.tool_call, false),
      json_schema: asBoolean(capabilities.json_schema, false),
      web_search: asBoolean(capabilities.web_search, false),
      unsupported_feature_combinations: toStringArrayArray(
        capabilities.unsupported_feature_combinations,
      ),
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
      estimated_cost_usd: asOptionalNumber(pricing.estimated_cost_usd)
        ?? asOptionalNumber(pricing.estimated_cost),
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

    node.children = children.sort((left, right) =>
      LOGICAL_LEVEL_RANK[left.level] - LOGICAL_LEVEL_RANK[right.level] || left.path.localeCompare(right.path),
    )
  }

  return rootNodes.sort(compareLogicalRoot)
}

function logicalTreeFromModels(models: ModelMetadata[]): LogicalNode[] {
  const modelIndex = buildModelIndex(models)
  const mountPaths = Array.from(modelIndex.byMount.keys()).sort()
  const nodesByPath = new Map<string, LogicalNode>()
  const rootNodes: LogicalNode[] = []

  const ensureNode = (path: string): LogicalNode => {
    const existing = nodesByPath.get(path)
    if (existing) return existing

    const apiType = inferApiType(path)
    const node: LogicalNode = {
      path,
      label: labelFromPath(path),
      level: path.includes('.') ? 'L3' : 'L2',
      api_type: apiType,
      exact_model_weights: {},
      resolved_exact_model: resolveModelForPath(path, modelIndex),
      children: [],
    }
    nodesByPath.set(path, node)

    const parentPath = parentLogicalPath(path)
    if (parentPath) {
      const parent = ensureNode(parentPath)
      parent.children = appendUniqueNode(parent.children ?? [], node)
    } else {
      rootNodes.push(node)
    }
    return node
  }

  for (const path of mountPaths) {
    const node = ensureNode(path)
    const childPaths = new Set((node.children ?? []).map((child) => child.path))
    for (const model of modelIndex.byMount.get(path) ?? []) {
      if (childPaths.has(model.exact_model)) continue
      node.children = [...(node.children ?? []), toExactModelNode(model, node.api_type)]
      childPaths.add(model.exact_model)
    }
  }

  for (const node of nodesByPath.values()) {
    node.children = (node.children ?? []).sort((left, right) => {
      if (left.level !== right.level) return left.level === 'L1' ? 1 : -1
      return left.path.localeCompare(right.path)
    })
  }

  return rootNodes.sort(compareLogicalRoot)
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
    ...providers
      .filter((provider) => provider.config.enabled)
      .flatMap((provider) => provider.status.discovered_models),
    ...localModels,
  ]
  const cloudProviderCount = providers.filter((provider) =>
    provider.config.enabled && (
      provider.config.provider_runtime_type === 'cloud_api' ||
      provider.config.provider_runtime_type === 'proxy_unknown'
    ),
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

function providerDisplayName(providerType: ProviderType, instanceName: string): string {
  return BUILTIN_PROVIDER_NAMES.find(([profile]) => profile === providerType)?.[1]
    ?? labelFromPath(instanceName)
}

export function isManagedSnProvider(provider: ProviderView): boolean {
  return provider.config.provider_profile_id === 'sn'
    && provider.config.auth_mode === 'dynamic_login'
}

function inferPricingMode(models: ModelMetadata[]): PricingMode {
  return models.some((model) =>
    model.pricing.input_token_usd != null ||
    model.pricing.output_token_usd != null ||
    model.pricing.cache_input_token_usd != null ||
    model.pricing.estimated_cost_usd != null,
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

function normalizeProviderType(value: unknown): ProviderType {
  return asOptionalString(value) ?? 'custom'
}

function normalizeAuthMode(value: unknown): ProviderConfig['auth_mode'] {
  return value === 'dynamic_login' ? 'dynamic_login' : value === 'api_key' ? 'api_key' : undefined
}

function timestampFromMs(value: number | undefined): string | undefined {
  return value == null ? undefined : new Date(value).toISOString()
}

function toProviderSetupCatalog(
  catalog: RawProviderCatalogResponse,
  adapters: RawProtocolAdapterListResponse,
): ProviderSetupCatalog {
  const providers = (Array.isArray(catalog.providers) ? catalog.providers : [])
    .map((item) => {
      const entry = asRecord(item)
      const provider_profile_id = normalizeProviderType(entry.provider_profile_id)
      if (provider_profile_id === 'custom') return null
      return {
        provider_profile_id,
        display_name: providerDisplayName(
          provider_profile_id,
          asNonEmptyString(entry.display_name, provider_profile_id),
        ),
        base_url: asNonEmptyString(entry.base_url, ''),
        protocol_adapter_id: asNonEmptyString(entry.protocol_adapter_id, ''),
        provider_rules_id: asOptionalString(entry.provider_rules_id),
        ui_hints: asRecord(entry.ui_hints),
        connection_fields: toProviderConnectionFields(entry.ui_hints),
      }
    })
    .filter((item): item is NonNullable<typeof item> => item !== null)
  const families = new Set<string>()
  for (const item of Array.isArray(adapters.adapters) ? adapters.adapters : []) {
    const family = asOptionalString(asRecord(item).protocol_family_id)
    if (family) families.add(family)
  }
  return {
    catalog_revision: asNumber(catalog.catalog_revision, 0),
    providers: sortProviderProfiles(providers),
    protocol_families: Array.from(families).sort().map((protocol_family_id) => ({
      protocol_family_id,
      display_name: `${labelFromPath(protocol_family_id)} compatible`,
    })),
  }
}

function sortProviderProfiles<T extends { provider_profile_id: ProviderType; display_name: string }>(providers: T[]): T[] {
  return [...providers].sort((left, right) => {
    const leftRank = PROVIDER_INFLUENCE_ORDER.get(left.provider_profile_id) ?? Number.MAX_SAFE_INTEGER
    const rightRank = PROVIDER_INFLUENCE_ORDER.get(right.provider_profile_id) ?? Number.MAX_SAFE_INTEGER
    return leftRank - rightRank || left.display_name.localeCompare(right.display_name)
  })
}

function toProviderConnectionFields(value: unknown): KnownProviderProfile['connection_fields'] {
  const instanceFields = asRecord(asRecord(value).instance_fields)
  const result: KnownProviderProfile['connection_fields'] = {}
  for (const name of ['region', 'workspace', 'account'] as const) {
    const field = asRecord(instanceFields[name])
    const mode = asOptionalString(field.mode)
    if (mode !== 'optional' && mode !== 'required') continue
    result[name] = {
      mode,
      default_value: asOptionalString(field.default_value),
      allowed_values: toStringArray(field.allowed_values),
    }
  }
  return result
}

function parseSettingsConflict(error: unknown): SettingsRevisionConflictError | null {
  const direct = settingsConflictRecord(error)
  if (direct) return conflictFromRecord(direct)
  const message = error instanceof Error ? error.message : String(error)
  if (!message.includes('settings_revision_conflict')) return null
  const jsonStart = message.indexOf('{')
  const jsonEnd = message.lastIndexOf('}')
  if (jsonStart >= 0 && jsonEnd > jsonStart) {
    try {
      const parsed = JSON.parse(message.slice(jsonStart, jsonEnd + 1))
      const record = settingsConflictRecord(parsed)
      if (record) return conflictFromRecord(record)
    } catch {
      return new SettingsRevisionConflictError()
    }
  }
  return new SettingsRevisionConflictError()
}

function settingsConflictRecord(value: unknown): RawRecord | null {
  const record = asRecord(value)
  if (record.code === 'settings_revision_conflict') return record
  for (const key of ['error', 'data', 'aicc']) {
    const nested = asRecord(record[key])
    if (nested.code === 'settings_revision_conflict') return nested
    const nestedError = asRecord(nested.error)
    if (nestedError.code === 'settings_revision_conflict') return nestedError
  }
  return null
}

function conflictFromRecord(record: RawRecord): SettingsRevisionConflictError {
  const details = asRecord(record.details)
  return new SettingsRevisionConflictError(
    asOptionalNumber(details.expected_revision),
    asOptionalNumber(details.actual_revision),
  )
}

function normalizeHealthStatus(value: unknown, fallback: ModelHealthStatus): ModelHealthStatus {
  if (value === 'degraded' || value === 'unavailable') return value
  if (value === 'available' || value === 'healthy') return 'available'
  return fallback
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

function toStringArrayArray(value: unknown): string[][] {
  return Array.isArray(value)
    ? value
        .map(toStringArray)
        .filter((combination) => combination.length >= 2)
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
          !['configuration', 'base_url', 'authentication', 'protocol', 'models', 'balance'].includes(kind ?? '')
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
      ? 'authentication'
      : lower.includes('base_url') || lower.includes('url') || lower.includes('connect')
        ? 'base_url'
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
// Directory children sort L3 (mount) -> L2 (target) -> L1 (exact model), then by path.
const LOGICAL_LEVEL_RANK: Record<LogicalNode['level'], number> = { L3: 0, L2: 1, L1: 2 }
