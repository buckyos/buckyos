import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { Activity, Check, ChevronDown, ChevronUp, Copy, CreditCard, DollarSign, Filter, HelpCircle, Route, Wallet, X } from 'lucide-react'
import { useMediaQuery } from '@mui/material'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore, useAIStatus, useProviders, useRouteTraces, useUsageSummary, useUsageTrend } from '../../hooks/use-aicc-store'
import { SummaryCard } from '../shared/SummaryCard'
import { PagedListFooter } from '../shared/paged-list'
import type { RouteTrace, UsageEvent, UsageEventsPage, UsageTimeRange } from '../../../../api/aicc_mgr'

function formatTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`
  return String(n)
}

function sortedEntries(record: Record<string, number>, limit?: number): Array<[string, number]> {
  const entries = Object.entries(record).sort((a, b) => b[1] - a[1])
  return limit == null ? entries : entries.slice(0, limit)
}

function candidateWeightSummary(candidate: RouteTrace['ranked_candidates'][number]): string {
  const inputs = candidate.preference_score_inputs
  const exact = inputs?.exact_model_weight ?? candidate.exact_model_weight ?? 1
  const provider = inputs?.provider_weight ?? candidate.provider_weight ?? 1
  const combined = inputs?.combined_weight ?? exact * provider
  return `exact ${formatWeight(exact)} · provider ${formatWeight(provider)} · combined ${formatWeight(combined)}`
}

function formatWeight(weight: number): string {
  return weight.toFixed(2).replace(/\.?0+$/, '')
}

function usageFinanceAmount(event: UsageEvent): number {
  return event.finance_snapshot?.amount ?? 0
}

function usageTokens(event: UsageEvent): number {
  return event.token_equivalent ?? (event.tokens_in ?? 0) + (event.tokens_out ?? 0)
}

function providerInstanceFromExactModel(model: string): string | undefined {
  const at = model.lastIndexOf('@')
  if (at < 0 || at === model.length - 1) return undefined
  return model.slice(at + 1)
}

function readableUsageProviderIdentifier(event: UsageEvent): string {
  const providerInstanceName = event.provider_instance_name.trim()
  if (providerInstanceName && providerInstanceName !== 'unknown-provider') return providerInstanceName
  const exactModelProvider = providerInstanceFromExactModel(event.exact_model)
  if (exactModelProvider) return exactModelProvider
  return event.exact_model || event.requested_model || providerInstanceName || 'unknown-provider'
}

function usageProviderDisplayName(event: UsageEvent, providerNames: Map<string, string>): string {
  const identifier = readableUsageProviderIdentifier(event)
  return providerNames.get(identifier) ?? identifier
}

function formatUsd(amount: number, compact = false): string {
  if (amount === 0) return '$0'
  const abs = Math.abs(amount)
  if (abs < 0.0001) return amount < 0 ? '>-$0.0001' : '<$0.0001'
  if (abs < 0.01) return `$${amount.toFixed(4)}`
  return `$${amount.toFixed(compact ? 2 : 4)}`
}

function formatLocalTime(value: string): string {
  const date = new Date(value)
  if (Number.isNaN(date.getTime())) return value
  return date.toLocaleString(undefined, {
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
  })
}

function dateInputStart(value: string): number | null {
  if (!value) return null
  const date = new Date(`${value}T00:00:00`)
  return Number.isNaN(date.getTime()) ? null : date.getTime()
}

function dateInputEnd(value: string): number | null {
  if (!value) return null
  const date = new Date(`${value}T23:59:59.999`)
  return Number.isNaN(date.getTime()) ? null : date.getTime()
}

function localDayStart(value = new Date()): Date {
  const result = new Date(value)
  result.setHours(0, 0, 0, 0)
  return result
}

function localTrailingDaysRange(days: number): UsageTimeRange {
  const start = localDayStart()
  start.setDate(start.getDate() - Math.max(0, days - 1))
  return { startTimeMs: start.getTime(), endTimeMs: Date.now() }
}

function timeRangeToQuery(
  value: TimeRangeFilter,
  customStartDate: string,
  customEndDate: string,
  nowMs: number,
): UsageTimeRange {
  if (value === 'custom') {
    const start = dateInputStart(customStartDate) ?? localTrailingDaysRange(30).startTimeMs
    const end = dateInputEnd(customEndDate) ?? nowMs
    return { startTimeMs: start, endTimeMs: end }
  }
  const duration = value === '24h'
    ? 24 * 60 * 60 * 1000
    : value === '7d'
      ? 7 * 24 * 60 * 60 * 1000
      : value === '30d'
        ? 30 * 24 * 60 * 60 * 1000
        : null
  if (duration != null) {
    return { startTimeMs: nowMs - duration, endTimeMs: nowMs }
  }
  return localTrailingDaysRange(30)
}

function uniqueSorted(values: Array<string | undefined>): string[] {
  return Array.from(new Set(values.filter((value): value is string => Boolean(value)))).sort((a, b) => a.localeCompare(b))
}

type TimeRangeFilter = 'all' | '24h' | '7d' | '30d' | 'custom'
type BreakdownFilterTarget = 'provider' | 'model' | 'appAgent'
type MultiFilter = {
  query: string
  selected: string[]
}
const PAGE_SIZE = 10
const HOME_TRACE_LIMIT = 5
const EMPTY_MULTI_FILTER: MultiFilter = { query: '', selected: [] }

export function UsageDashboard() {
  const { t } = useI18n()
  const store = useAICCStore()
  const status = useAIStatus()
  const providers = useProviders()
  const summary = useUsageSummary()
  const trend = useUsageTrend('day')
  const traces = useRouteTraces()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const [timeRange, setTimeRange] = useState<TimeRangeFilter>('all')
  const [providerFilter, setProviderFilter] = useState<MultiFilter>(EMPTY_MULTI_FILTER)
  const [modelFilter, setModelFilter] = useState<MultiFilter>(EMPTY_MULTI_FILTER)
  const [appAgentFilter, setAppAgentFilter] = useState<MultiFilter>(EMPTY_MULTI_FILTER)
  const [customStartDate, setCustomStartDate] = useState('')
  const [customEndDate, setCustomEndDate] = useState('')
  const [detailPage, setDetailPage] = useState(1)
  const [nowMs] = useState(() => Date.now())
  const [pageCursors, setPageCursors] = useState<Record<number, string | undefined>>({ 1: undefined })
  const [usagePage, setUsagePage] = useState<UsageEventsPage>({ events: [], totalRequests: 0 })
  const [usageLoading, setUsageLoading] = useState(false)
  const [usageError, setUsageError] = useState<string | null>(null)
  const [usageRetryKey, setUsageRetryKey] = useState(0)
  const [linkedTraceTaskId, setLinkedTraceTaskId] = useState<string | null>(null)
  const [linkedTraces, setLinkedTraces] = useState<RouteTrace[]>([])
  const [linkedTraceCursor, setLinkedTraceCursor] = useState<string | undefined>()
  const [linkedTraceLoading, setLinkedTraceLoading] = useState(false)
  const [linkedTraceError, setLinkedTraceError] = useState<string | null>(null)
  const [filtersSheetOpen, setFiltersSheetOpen] = useState(false)
  const detailRef = useRef<HTMLElement | null>(null)
  const linkedTraceRef = useRef<HTMLElement | null>(null)

  const snProvider = providers.find((p) => p.config.provider_type === 'sn_router')
  const snCredit = snProvider?.account.balance_value
  const balanceProviders = providers.filter((p) => p.account.balance_supported && p.account.balance_value != null)
  const usageOnlyProviders = providers.filter((p) => p.account.usage_supported && !p.account.balance_supported)
  const maxTrendTokens = Math.max(...trend.map((p) => p.tokens), 1)
  const providerNames = useMemo(() => {
    const names = new Map<string, string>()
    for (const provider of providers) {
      const instanceName = provider.config.provider_instance_name.trim()
      const displayName = provider.config.name.trim()
      if (instanceName) {
        names.set(instanceName, displayName || instanceName)
      }
    }
    return names
  }, [providers])
  const providerOptions = useMemo(() => uniqueSorted(Object.keys(summary.by_provider)), [summary.by_provider])
  const modelOptions = useMemo(() => uniqueSorted(Object.keys(summary.by_model)), [summary.by_model])
  const appAgentOptions = useMemo(() => uniqueSorted(Object.keys(summary.by_app)), [summary.by_app])
  const usageQueryRange = useMemo(
    () => timeRangeToQuery(timeRange, customStartDate, customEndDate, nowMs),
    [customEndDate, customStartDate, nowMs, timeRange],
  )
  const usageQueryFilters = useMemo(() => ({
    providerInstanceNames: providerFilter.selected,
    providerInstanceQuery: providerFilter.query,
    providerModels: modelFilter.selected,
    providerModelQuery: modelFilter.query,
    appIds: appAgentFilter.selected,
    appQuery: appAgentFilter.query,
  }), [appAgentFilter, modelFilter, providerFilter])
  const currentCursor = pageCursors[detailPage]
  const recentTraces = traces.slice(0, HOME_TRACE_LIMIT)
  const effectiveDetailPage = detailPage
  const pageStart = (effectiveDetailPage - 1) * PAGE_SIZE
  const pagedEvents = usagePage.events
  const detailPageCount = Math.max(1, Math.ceil(usagePage.totalRequests / PAGE_SIZE))
  const canGoNext = effectiveDetailPage < detailPageCount && pageCursors[effectiveDetailPage + 1] != null
  const hasUsageMore = Boolean(pageCursors[effectiveDetailPage + 1])
  const timeRangeOptions: Array<[TimeRangeFilter, string]> = useMemo(() => [
    ['all', t('aiCenter.home.allTime', 'All time')],
    ['24h', t('aiCenter.home.last24Hours', 'Last 24 hours')],
    ['7d', t('aiCenter.home.last7Days', 'Last 7 days')],
    ['30d', t('aiCenter.home.last30Days', 'Last 30 days')],
    ['custom', t('aiCenter.home.customRange', 'Custom range')],
  ], [t])
  const activeFilterChips = useMemo(
    () => usageFilterChips(timeRange, providerFilter, modelFilter, appAgentFilter, timeRangeOptions),
    [appAgentFilter, modelFilter, providerFilter, timeRange, timeRangeOptions],
  )

  useEffect(() => {
    let cancelled = false
    async function loadUsagePage() {
      setUsageLoading(true)
      setUsageError(null)
      try {
        const page = await store.queryUsageEvents({
          timeRange: usageQueryRange,
          filters: usageQueryFilters,
          cursor: currentCursor,
          limit: PAGE_SIZE,
        })
        if (cancelled) return
        setUsagePage((current) => {
          if (isMobile && detailPage > 1) {
            return {
              ...page,
              events: mergeUsageEvents(current.events, page.events),
              totalRequests: page.totalRequests,
            }
          }
          return page
        })
        setPageCursors((current) => {
          if (current[detailPage + 1] === page.nextCursor) return current
          return {
            ...current,
            [detailPage + 1]: page.nextCursor,
          }
        })
      } catch (error) {
        if (cancelled) return
        console.error('aicc.usage.query events failed', error)
        setUsageError(t('aiCenter.home.usageLoadFailed', 'Could not load usage events.'))
      } finally {
        if (!cancelled) {
          setUsageLoading(false)
        }
      }
    }
    void loadUsagePage()
    return () => {
      cancelled = true
    }
  }, [currentCursor, detailPage, isMobile, store, t, usageQueryFilters, usageQueryRange, usageRetryKey])

  const resetUsagePaging = () => {
    setDetailPage(1)
    setPageCursors({ 1: undefined })
    setUsageError(null)
  }

  const clearUsageFilters = () => {
    setTimeRange('all')
    setProviderFilter(EMPTY_MULTI_FILTER)
    setModelFilter(EMPTY_MULTI_FILTER)
    setAppAgentFilter(EMPTY_MULTI_FILTER)
    setCustomStartDate('')
    setCustomEndDate('')
    resetUsagePaging()
  }

  const updateTimeRange = (value: TimeRangeFilter) => {
    setTimeRange(value)
    resetUsagePaging()
  }

  const updateProviderFilter = (value: MultiFilter) => {
    setProviderFilter(value)
    resetUsagePaging()
  }

  const updateModelFilter = (value: MultiFilter) => {
    setModelFilter(value)
    resetUsagePaging()
  }

  const updateAppAgentFilter = (value: MultiFilter) => {
    setAppAgentFilter(value)
    resetUsagePaging()
  }

  const balanceSubtitle = balanceProviders
    .map((p) => {
      const unit = p.account.balance_unit === 'usd' ? '$' : ''
      const suffix = p.account.balance_unit === 'credit' ? ' Credit' : ''
      return `${p.config.provider_instance_name}: ${unit}${p.account.balance_value}${suffix}`
    })
    .join(' · ')

  const usageOnlyNote = t(
    'aiCenter.home.usageOnlyProviderNote',
    '{{count}} usage-only provider(s) report usage/cost without balance.',
    { count: usageOnlyProviders.length },
  )
  const balanceOverviewValue = t(
    'aiCenter.home.balanceOverviewValue',
    '{{balanceCount}} balance / {{usageOnlyCount}} usage-only',
    { balanceCount: balanceProviders.length, usageOnlyCount: usageOnlyProviders.length },
  )
  const balanceOverviewSubtitle = balanceProviders.length > 0
    ? `${balanceSubtitle}. ${usageOnlyNote}`
    : t('aiCenter.home.usageOnlyExplain', 'No provider exposes balance yet. Usage-only providers can still report usage and cost.')

  const applyBreakdownFilter = (target: BreakdownFilterTarget, value: string) => {
    setTimeRange('all')
    setProviderFilter(target === 'provider' ? { query: '', selected: [value] } : EMPTY_MULTI_FILTER)
    setModelFilter(target === 'model' ? { query: '', selected: [value] } : EMPTY_MULTI_FILTER)
    setAppAgentFilter(target === 'appAgent' ? { query: '', selected: [value] } : EMPTY_MULTI_FILTER)
    resetUsagePaging()
    window.requestAnimationFrame(() => {
      detailRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' })
    })
  }

  const loadLinkedTraces = async (taskId: string, cursor?: string) => {
    const normalizedTaskId = taskId.trim()
    if (!normalizedTaskId || linkedTraceLoading) return
    setLinkedTraceLoading(true)
    setLinkedTraceError(null)
    try {
      const page = await store.queryRouteTraces({
        limit: 20,
        cursor,
        taskIds: [normalizedTaskId],
        requestIds: [normalizedTaskId],
      })
      setLinkedTraceTaskId(normalizedTaskId)
      setLinkedTraces((current) => cursor ? mergeRouteTraces(current, page.traces) : page.traces)
      setLinkedTraceCursor(page.nextCursor)
      window.requestAnimationFrame(() => {
        linkedTraceRef.current?.scrollIntoView({ behavior: 'smooth', block: 'start' })
      })
    } catch (error) {
      console.error('aicc.trace.query linked task failed', error)
      setLinkedTraceTaskId(normalizedTaskId)
      setLinkedTraces([])
      setLinkedTraceCursor(undefined)
      setLinkedTraceError(t('aiCenter.home.linkedTraceLoadFailed', 'Could not load route traces for this task.'))
    } finally {
      setLinkedTraceLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-6">
      <StatusAndKpiHeader
        statusTitle={t('aiCenter.home.status', 'AI Status')}
        statusValue={status.state === 'disabled' ? t('aiCenter.home.disabled', 'Disabled') : t('aiCenter.home.enabled', 'Enabled')}
        providerLabel={t('aiCenter.home.providers', 'Providers')}
        modelLabel={t('aiCenter.home.models', 'Models')}
        issueLabel={t('aiCenter.home.issues', 'Issues')}
        providerCount={status.provider_count}
        modelCount={status.model_count}
        issueCount={status.health_counts.degraded + status.health_counts.unavailable + status.quota_warnings}
        kpis={[
          {
            icon: <CreditCard size={18} />,
            title: t('aiCenter.home.credit', 'SN Credit'),
            value: snCredit != null ? `${snCredit} Credit` : '-',
            subtitle: snProvider ? `${snProvider.account.pricing_mode} / top up available` : undefined,
          },
          {
            icon: <DollarSign size={18} />,
            title: t('aiCenter.home.estimatedCost', 'Est. Cost'),
            value: formatUsd(summary.total_estimated_cost, true),
            subtitle: t('aiCenter.home.costEstimated', 'Estimated from usage events'),
          },
          {
            icon: <Wallet size={18} />,
            title: t('aiCenter.home.balanceOverview', 'Balance Overview'),
            value: balanceOverviewValue,
            subtitle: balanceOverviewSubtitle,
          },
          {
            icon: <Activity size={18} />,
            title: t('aiCenter.home.requests', 'Requests'),
            value: summary.total_requests.toString(),
            subtitle: t('aiCenter.home.totalRequests', 'Total requests'),
          },
        ]}
      />

      <div className="hidden">
        <SummaryCard
          icon={<Activity size={18} />}
          title={t('aiCenter.home.status', 'AI Status')}
          value={status.state === 'disabled' ? t('aiCenter.home.disabled', 'Disabled') : t('aiCenter.home.enabled', 'Enabled')}
          subtitle={`${status.provider_count} Provider instances · ${status.model_count} Models · ${status.health_counts.degraded} degraded`}
        />
        <SummaryCard
          icon={<CreditCard size={18} />}
          title={t('aiCenter.home.credit', 'SN Credit')}
          value={snCredit != null ? `${snCredit} Credit` : '—'}
          subtitle={snProvider ? `${snProvider.account.pricing_mode} · top up available` : undefined}
        />
        <SummaryCard
          icon={<DollarSign size={18} />}
          title={t('aiCenter.home.estimatedCost', 'Est. Cost')}
          value={formatUsd(summary.total_estimated_cost, true)}
          subtitle={t('aiCenter.home.costEstimated', 'Estimated from usage events')}
        />
        <SummaryCard
          icon={<Wallet size={18} />}
          title={t('aiCenter.home.balanceOverview', 'Balance Overview')}
          value={balanceOverviewValue}
          subtitle={balanceOverviewSubtitle}
        />
      </div>

      <div className="grid grid-cols-1 xl:grid-cols-[minmax(0,2fr)_minmax(280px,1fr)] gap-4">
        <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <div className="flex items-center justify-between mb-4">
            <h3 className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.home.trendTitle', 'Usage Trend')}
            </h3>
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.home.last30Days', 'Last 30 days')}
            </span>
          </div>
          <div className="flex items-end gap-1 h-52">
            {trend.map((point) => (
              <div key={point.timestamp} className="flex-1 flex flex-col items-center gap-1 min-w-0">
                <div
                  className="w-full rounded-t-sm"
                  title={`${point.timestamp}: ${formatTokens(point.tokens)} tokens / $${point.estimated_cost.toFixed(4)}`}
                  style={{
                    height: `${Math.max(4, (point.tokens / maxTrendTokens) * 180)}px`,
                    background: point.tokens > 0 ? 'var(--cp-accent)' : 'var(--cp-border)',
                    opacity: point.tokens > 0 ? 0.78 : 0.4,
                  }}
                />
                <span className="text-[10px] hidden sm:block" style={{ color: 'var(--cp-muted)' }}>
                  {point.timestamp.slice(3)}
                </span>
              </div>
            ))}
          </div>
        </section>

        <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <h3 className="text-sm font-medium mb-3" style={{ color: 'var(--cp-text)' }}>
            {t('aiCenter.home.categoryTitle', 'API Type Breakdown')}
          </h3>
          <div className="flex flex-col gap-2">
            {sortedEntries(summary.by_api_namespace, 8).map(([namespace, tokens]) => (
              <MeterRow key={namespace} label={namespace} value={tokens} max={summary.total_tokens} />
            ))}
          </div>
        </section>
      </div>

      <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
        <h3 className="text-sm font-medium mb-3" style={{ color: 'var(--cp-text)' }}>
          {t('aiCenter.home.usageSummary', 'Usage Summary')}
        </h3>
        <div className="grid grid-cols-2 md:grid-cols-5 gap-4 items-stretch">
          <Stat label={t('aiCenter.home.today', 'Today')} value={`${formatTokens(summary.today_tokens)} tokens`} />
          <Stat label={t('aiCenter.home.thisMonth', 'This Month')} value={`${formatTokens(summary.this_month_tokens)} tokens`} />
          <Stat label={t('aiCenter.home.total', 'Total')} value={`${formatTokens(summary.total_tokens)} tokens`} />
          <Stat label={t('aiCenter.home.requests', 'Requests')} value={summary.total_requests.toString()} />
          <Stat label={t('aiCenter.home.totalCost', 'Total Est. Cost')} value={formatUsd(summary.total_estimated_cost, true)} />
        </div>
      </section>

      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <Breakdown
          title={t('aiCenter.home.byProvider', 'By Provider Instance')}
          rows={sortedEntries(summary.by_provider)}
          total={summary.total_tokens}
          activeLabel={providerFilter.selected.length === 1 && !providerFilter.query ? providerFilter.selected[0] : undefined}
          onSelect={(label) => applyBreakdownFilter('provider', label)}
          viewAllLabel={t('aiCenter.home.viewAll', 'View all')}
          showLessLabel={t('aiCenter.home.showLess', 'Show less')}
          filterLabel={t('aiCenter.home.filterToDetail', 'Filter Usage Detail')}
          emptyLabel={t('aiCenter.home.noBreakdownData', 'No usage data yet.')}
        />
        <Breakdown
          title={t('aiCenter.home.byModel', 'By Exact Model')}
          rows={sortedEntries(summary.by_model)}
          total={summary.total_tokens}
          activeLabel={modelFilter.selected.length === 1 && !modelFilter.query ? modelFilter.selected[0] : undefined}
          onSelect={(label) => applyBreakdownFilter('model', label)}
          viewAllLabel={t('aiCenter.home.viewAll', 'View all')}
          showLessLabel={t('aiCenter.home.showLess', 'Show less')}
          filterLabel={t('aiCenter.home.filterToDetail', 'Filter Usage Detail')}
          emptyLabel={t('aiCenter.home.noBreakdownData', 'No usage data yet.')}
        />
        <Breakdown
          title={t('aiCenter.home.byApp', 'By App / Agent')}
          rows={sortedEntries(summary.by_app)}
          total={summary.total_tokens}
          activeLabel={appAgentFilter.selected.length === 1 && !appAgentFilter.query ? appAgentFilter.selected[0] : undefined}
          onSelect={(label) => applyBreakdownFilter('appAgent', label)}
          viewAllLabel={t('aiCenter.home.viewAll', 'View all')}
          showLessLabel={t('aiCenter.home.showLess', 'Show less')}
          filterLabel={t('aiCenter.home.filterToDetail', 'Filter Usage Detail')}
          emptyLabel={t('aiCenter.home.noBreakdownData', 'No usage data yet.')}
        />
      </div>

      {recentTraces.length > 0 && (
        <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <div className="mb-3 flex items-center justify-between gap-3">
            <div className="flex items-center gap-2">
              <Route size={16} style={{ color: 'var(--cp-accent)' }} />
              <h3 className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                {t('aiCenter.home.recentRouteTrace', 'Recent Route Traces')}
              </h3>
            </div>
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>{recentTraces.length}</span>
          </div>
          <div className="grid grid-cols-1 gap-2">
            {recentTraces.map((trace) => (
              <RecentTraceCard key={trace.request_id} trace={trace} />
            ))}
          </div>
        </section>
      )}

      <section ref={detailRef} className="rounded-xl overflow-hidden scroll-mt-4" style={{ border: '1px solid var(--cp-border)' }}>
        <div className="px-4 py-3 flex flex-col gap-3" style={{ background: 'var(--cp-surface)' }}>
          <div className="flex items-center justify-between gap-3">
            <h3 className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.home.detailTable', 'Usage Detail')}
            </h3>
            <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {usageLoading ? t('common.loading', 'Loading') : usagePage.totalRequests}
            </span>
          </div>
          {isMobile && (
            <div className="flex flex-col gap-2">
              <div className="flex items-center gap-2">
                <button
                  type="button"
                  onClick={() => setFiltersSheetOpen(true)}
                  className="inline-flex min-h-11 flex-1 items-center justify-center gap-2 rounded-lg px-3 text-sm font-medium"
                  style={{ color: 'var(--cp-text)', background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
                >
                  <Filter size={16} />
                  {t('aiCenter.home.filters', 'Filters')}
                </button>
                {activeFilterChips.length > 0 && (
                  <button
                    type="button"
                    onClick={clearUsageFilters}
                    className="inline-flex min-h-11 items-center justify-center rounded-lg px-3 text-sm"
                    style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
                  >
                    {t('common.clear', 'Clear')}
                  </button>
                )}
              </div>
              {activeFilterChips.length > 0 && (
                <div className="flex gap-2 overflow-x-auto pb-1">
                  {activeFilterChips.map((chip) => (
                    <span
                      key={chip}
                      className="shrink-0 rounded-full px-2.5 py-1 text-xs"
                      style={{ color: 'var(--cp-accent)', background: 'color-mix(in oklch, var(--cp-accent), transparent 88%)' }}
                    >
                      {chip}
                    </span>
                  ))}
                </div>
              )}
            </div>
          )}
          <div className="hidden grid-cols-1 gap-2 md:grid sm:grid-cols-2 xl:grid-cols-4">
            <TimeRangeFilterControl
              label={t('aiCenter.home.filterTimeRange', 'Time Range')}
              value={timeRange}
              onChange={updateTimeRange}
              customStartDate={customStartDate}
              customEndDate={customEndDate}
              customStartLabel={t('aiCenter.home.filterStartDate', 'Start Date')}
              customEndLabel={t('aiCenter.home.filterEndDate', 'End Date')}
              onCustomStartDateChange={(value) => {
                setCustomStartDate(value)
                resetUsagePaging()
              }}
              onCustomEndDateChange={(value) => {
                setCustomEndDate(value)
                resetUsagePaging()
              }}
              options={[
                ['all', t('aiCenter.home.allTime', 'All time')],
                ['24h', t('aiCenter.home.last24Hours', 'Last 24 hours')],
                ['7d', t('aiCenter.home.last7Days', 'Last 7 days')],
                ['30d', t('aiCenter.home.last30Days', 'Last 30 days')],
                ['custom', t('aiCenter.home.customRange', 'Custom range')],
              ]}
            />
            <MultiSelectFilter
              label={t('aiCenter.home.filterProvider', 'Provider')}
              value={providerFilter}
              onChange={updateProviderFilter}
              options={providerOptions}
              allLabel={t('aiCenter.home.allProviders', 'All providers')}
            />
            <MultiSelectFilter
              label={t('aiCenter.home.filterModel', 'Model')}
              value={modelFilter}
              onChange={updateModelFilter}
              options={modelOptions}
              allLabel={t('aiCenter.home.allModels', 'All models')}
            />
            <MultiSelectFilter
              label={t('aiCenter.home.filterAppAgent', 'App / Agent')}
              value={appAgentFilter}
              onChange={updateAppAgentFilter}
              options={appAgentOptions}
              allLabel={t('aiCenter.home.allAppsAgents', 'All apps / agents')}
            />
          </div>
        </div>
        <div className="hidden overflow-x-auto md:block">
          <table className="w-full min-w-[940px]">
            <thead>
              <tr style={{ background: 'var(--cp-bg)' }}>
                {['Time', 'Provider', 'Exact Model', 'API Type', 'App / Agent', 'Task / Session', 'Tokens', 'Cost', 'Status'].map((h) => (
                  <th key={h} className="text-left text-xs font-medium px-4 py-2" style={{ color: 'var(--cp-muted)' }}>
                    <span className="inline-flex items-center gap-1">
                      {h}
                      {h === 'Task / Session' && (
                        <span
                          className="inline-flex"
                          title={t('aiCenter.home.taskSessionTooltip', 'Task / Session is the AICC task id or Agent session id that produced this usage event.')}
                        >
                          <HelpCircle size={12} />
                        </span>
                      )}
                    </span>
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {pagedEvents.map((event) => {
                const tokens = usageTokens(event)
                const providerIdentifier = readableUsageProviderIdentifier(event)
                const providerDisplayName = usageProviderDisplayName(event, providerNames)
                return (
                  <tr key={event.id} style={{ borderTop: '1px solid var(--cp-border)' }}>
                    <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-muted)' }}>{formatLocalTime(event.timestamp)}</td>
                    <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-text)' }}>
                      <TruncatedText
                        value={providerDisplayName}
                        title={providerDisplayName === providerIdentifier ? providerDisplayName : `${providerDisplayName} (${providerIdentifier})`}
                        className="max-w-[160px]"
                      />
                    </td>
                    <td className="px-4 py-2 text-xs font-mono" style={{ color: 'var(--cp-text)' }}>
                      <TruncatedText value={event.exact_model} className="max-w-[220px]" />
                    </td>
                    <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-text)' }}>{event.api_type}</td>
                    <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-text)' }}>
                      <TruncatedText value={`${event.app_id ?? 'system'}${event.agent_id ? ` / ${event.agent_id}` : ''}`} className="max-w-[180px]" />
                    </td>
                    <td className="px-4 py-2 text-xs">
                      {event.session_id ? (
                        <button
                          type="button"
                          onClick={() => void loadLinkedTraces(event.session_id ?? '')}
                          className="max-w-[180px] truncate font-mono underline-offset-2 hover:underline"
                          style={{ color: 'var(--cp-accent)' }}
                          title={t('aiCenter.home.viewTaskRouteTraces', 'View route traces for this task')}
                        >
                          {event.session_id}
                        </button>
                      ) : (
                        <span style={{ color: 'var(--cp-muted)' }}>-</span>
                      )}
                    </td>
                    <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-text)' }}>{formatTokens(tokens)}</td>
                    <td className="px-4 py-2 text-xs" style={{ color: 'var(--cp-text)' }}>{formatUsd(usageFinanceAmount(event))}</td>
                    <td className="px-4 py-2 text-xs" style={{ color: event.status === 'success' ? 'var(--cp-success)' : 'var(--cp-danger)' }}>{event.status}</td>
                  </tr>
                )
              })}
              {!usageLoading && pagedEvents.length === 0 && (
                <tr style={{ borderTop: '1px solid var(--cp-border)' }}>
                  <td className="px-4 py-8 text-center text-xs" colSpan={9} style={{ color: 'var(--cp-muted)' }}>
                    {t('aiCenter.home.noUsageEvents', 'No usage events match the current filters.')}
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
        {isMobile && (
          <div className="flex flex-col gap-3 p-3" style={{ background: 'var(--cp-bg)' }}>
            {pagedEvents.map((event) => (
              <UsageEventCard
                key={event.id}
                event={event}
                providerNames={providerNames}
                onOpenTrace={(taskId) => void loadLinkedTraces(taskId)}
              />
            ))}
            {!usageLoading && pagedEvents.length === 0 && (
              <div className="rounded-lg px-3 py-8 text-center text-xs" style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface)' }}>
                {t('aiCenter.home.noUsageEvents', 'No usage events match the current filters.')}
              </div>
            )}
          </div>
        )}
        {usageError && !isMobile && (
          <div className="px-4 py-3 text-xs" style={{ color: 'var(--cp-danger)', background: 'var(--cp-surface)', borderTop: '1px solid var(--cp-border)' }}>
            {usageError}
          </div>
        )}
        {(usagePage.totalRequests > 0 || isMobile) && (
          <PagedListFooter
            mode={isMobile ? 'infinite' : 'pagination'}
            loading={usageLoading}
            error={isMobile ? usageError : null}
            hasMore={isMobile ? hasUsageMore : canGoNext}
            onLoadMore={() => {
              if (hasUsageMore) setDetailPage((page) => page + 1)
            }}
            onRetry={() => {
              setUsageError(null)
              setUsageRetryKey((value) => value + 1)
            }}
            onPreviousPage={() => setDetailPage((page) => Math.max(1, page - 1))}
            onNextPage={() => setDetailPage((page) => Math.min(detailPageCount, page + 1))}
            canGoPrevious={effectiveDetailPage > 1}
            canGoNext={canGoNext}
            pageIndex={effectiveDetailPage - 1}
            loadedCount={isMobile ? pagedEvents.length : Math.min(pageStart + pagedEvents.length, usagePage.totalRequests)}
            totalCount={usagePage.totalRequests}
            labels={{
              previous: t('common.previous', 'Previous'),
              next: t('common.next', 'Next'),
              page: t('aiCenter.home.pageNumber', 'Page {{page}}'),
              loading: t('common.loading', 'Loading'),
              loadMore: t('common.loadMore', 'Load more'),
              retry: t('common.retry', 'Retry'),
              error: t('aiCenter.home.usageLoadFailed', 'Could not load usage events.'),
              loaded: `${isMobile ? pagedEvents.length : `${pageStart + 1}-${Math.min(pageStart + pagedEvents.length, usagePage.totalRequests)}`} / ${usagePage.totalRequests}`,
            }}
          />
        )}
      </section>

      {filtersSheetOpen && (
        <UsageFiltersSheet
          onClose={() => setFiltersSheetOpen(false)}
          onClear={clearUsageFilters}
          clearLabel={t('common.clear', 'Clear')}
          closeLabel={t('common.close', 'Close')}
          title={t('aiCenter.home.filters', 'Filters')}
        >
          <TimeRangeFilterControl
            label={t('aiCenter.home.filterTimeRange', 'Time Range')}
            value={timeRange}
            onChange={updateTimeRange}
            customStartDate={customStartDate}
            customEndDate={customEndDate}
            customStartLabel={t('aiCenter.home.filterStartDate', 'Start Date')}
            customEndLabel={t('aiCenter.home.filterEndDate', 'End Date')}
            onCustomStartDateChange={(value) => {
              setCustomStartDate(value)
              resetUsagePaging()
            }}
            onCustomEndDateChange={(value) => {
              setCustomEndDate(value)
              resetUsagePaging()
            }}
            options={timeRangeOptions}
          />
          <MultiSelectFilter
            label={t('aiCenter.home.filterProvider', 'Provider')}
            value={providerFilter}
            onChange={updateProviderFilter}
            options={providerOptions}
            allLabel={t('aiCenter.home.allProviders', 'All providers')}
          />
          <MultiSelectFilter
            label={t('aiCenter.home.filterModel', 'Model')}
            value={modelFilter}
            onChange={updateModelFilter}
            options={modelOptions}
            allLabel={t('aiCenter.home.allModels', 'All models')}
          />
          <MultiSelectFilter
            label={t('aiCenter.home.filterAppAgent', 'App / Agent')}
            value={appAgentFilter}
            onChange={updateAppAgentFilter}
            options={appAgentOptions}
            allLabel={t('aiCenter.home.allAppsAgents', 'All apps / agents')}
          />
        </UsageFiltersSheet>
      )}

      {linkedTraceTaskId && (
        <section ref={linkedTraceRef} className="rounded-xl p-4 scroll-mt-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
          <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
            <div className="flex min-w-0 items-center gap-2">
              <Route size={16} style={{ color: 'var(--cp-accent)' }} />
              <h3 className="min-w-0 truncate text-sm font-medium" style={{ color: 'var(--cp-text)' }}>
                {t('aiCenter.home.linkedRouteTraces', 'Route Traces for Task / Session')}
              </h3>
            </div>
            <span className="text-xs font-mono" style={{ color: 'var(--cp-muted)' }}>{linkedTraceTaskId}</span>
          </div>
          <div className="grid grid-cols-1 gap-2">
            {linkedTraces.map((trace) => (
              <RecentTraceCard key={trace.request_id} trace={trace} />
            ))}
            {!linkedTraceLoading && linkedTraces.length === 0 && (
              <div className="rounded-lg px-3 py-8 text-center text-xs" style={{ color: 'var(--cp-muted)', background: 'var(--cp-bg)' }}>
                {t('aiCenter.home.noLinkedRouteTraces', 'No route traces were found for this task/session.')}
              </div>
            )}
          </div>
          {linkedTraceError && (
            <div className="mt-3 rounded-lg px-3 py-2 text-xs" style={{ color: 'var(--cp-warning)', background: 'var(--cp-bg)' }}>
              {linkedTraceError}
            </div>
          )}
          {linkedTraceCursor && (
            <button
              type="button"
              onClick={() => void loadLinkedTraces(linkedTraceTaskId, linkedTraceCursor)}
              disabled={linkedTraceLoading}
              className="mt-3 inline-flex min-h-9 w-full items-center justify-center gap-2 rounded-lg px-3 text-sm font-medium disabled:opacity-60"
              style={{ color: 'var(--cp-accent)', background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
            >
              <ChevronDown size={15} />
              {linkedTraceLoading
                ? t('aiCenter.home.traceLoading', 'Loading...')
                : t('aiCenter.home.traceLoadMore', 'Load more')}
            </button>
          )}
        </section>
      )}
    </div>
  )
}

function StatusAndKpiHeader({
  statusTitle,
  statusValue,
  providerLabel,
  modelLabel,
  issueLabel,
  providerCount,
  modelCount,
  issueCount,
  kpis,
}: {
  statusTitle: string
  statusValue: string
  providerLabel: string
  modelLabel: string
  issueLabel: string
  providerCount: number
  modelCount: number
  issueCount: number
  kpis: Array<{ icon: ReactNode; title: string; value: string; subtitle?: string }>
}) {
  return (
    <div className="grid grid-cols-1 gap-3 lg:grid-cols-[minmax(260px,0.9fr)_minmax(0,2fr)]">
      <div
        className="rounded-xl p-4"
        style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      >
        <div className="flex items-center gap-2 text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
          <Activity size={18} style={{ color: 'var(--cp-accent)' }} />
          {statusTitle}
        </div>
        <div className="mt-3 text-2xl font-semibold leading-tight" style={{ color: 'var(--cp-text)' }}>
          {statusValue}
        </div>
        <div className="mt-4 grid grid-cols-3 gap-2">
          <StatusMiniMetric label={providerLabel} value={providerCount} />
          <StatusMiniMetric label={modelLabel} value={modelCount} />
          <StatusMiniMetric label={issueLabel} value={issueCount} warning={issueCount > 0} />
        </div>
      </div>
      <div className="relative min-w-0">
        <div className="flex snap-x snap-mandatory gap-3 overflow-x-auto pb-1 pr-12">
          {kpis.map((kpi) => (
            <div key={kpi.title} className="min-w-[72%] snap-start sm:min-w-[240px] lg:min-w-[220px]">
              <SummaryCard icon={kpi.icon} title={kpi.title} value={kpi.value} subtitle={kpi.subtitle} />
            </div>
          ))}
        </div>
        <div
          className="pointer-events-none absolute bottom-1 right-0 top-0 w-16"
          style={{ background: 'linear-gradient(90deg, transparent, var(--cp-bg))' }}
        />
      </div>
    </div>
  )
}

function StatusMiniMetric({ label, value, warning }: { label: string; value: number; warning?: boolean }) {
  return (
    <div className="min-w-0 rounded-lg px-2 py-2" style={{ background: 'var(--cp-bg)' }}>
      <div className="truncate text-[11px]" style={{ color: 'var(--cp-muted)' }}>{label}</div>
      <div className="mt-1 text-base font-semibold tabular-nums" style={{ color: warning ? 'var(--cp-warning)' : 'var(--cp-text)' }}>
        {value}
      </div>
    </div>
  )
}

function UsageFiltersSheet({
  title,
  clearLabel,
  closeLabel,
  children,
  onClear,
  onClose,
}: {
  title: string
  clearLabel: string
  closeLabel: string
  children: ReactNode
  onClear: () => void
  onClose: () => void
}) {
  return (
    <div className="fixed inset-0 z-50 md:hidden">
      <div className="absolute inset-0" style={{ background: 'rgba(0,0,0,0.35)' }} onClick={onClose} />
      <div
        className="absolute inset-x-0 bottom-0 max-h-[82vh] overflow-y-auto rounded-t-xl p-4"
        style={{
          background: 'var(--cp-surface)',
          borderTop: '1px solid var(--cp-border)',
          paddingBottom: 'calc(1rem + env(safe-area-inset-bottom))',
        }}
      >
        <div className="mb-4 flex items-center justify-between gap-3">
          <h3 className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{title}</h3>
          <div className="flex items-center gap-2">
            <button type="button" onClick={onClear} className="min-h-11 rounded-lg px-3 text-sm" style={{ color: 'var(--cp-accent)' }}>
              {clearLabel}
            </button>
            <button
              type="button"
              onClick={onClose}
              className="flex min-h-11 min-w-11 items-center justify-center rounded-lg"
              style={{ color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}
              aria-label={closeLabel}
            >
              <X size={17} />
            </button>
          </div>
        </div>
        <div className="grid grid-cols-1 gap-3">{children}</div>
      </div>
    </div>
  )
}

function UsageEventCard({
  event,
  providerNames,
  onOpenTrace,
}: {
  event: UsageEvent
  providerNames: Map<string, string>
  onOpenTrace: (taskId: string) => void
}) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const [copied, setCopied] = useState(false)
  const tokens = usageTokens(event)
  const providerIdentifier = readableUsageProviderIdentifier(event)
  const providerDisplayName = usageProviderDisplayName(event, providerNames)
  const appAgent = `${event.app_id ?? 'system'}${event.agent_id ? ` / ${event.agent_id}` : ''}`
  const longValues = [
    providerDisplayName,
    event.exact_model,
    appAgent,
    event.session_id ?? '',
  ].filter(Boolean)
  const copyValue = longValues.join('\n')

  const copyDetails = async () => {
    try {
      await navigator.clipboard.writeText(copyValue)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    } catch {
      setCopied(false)
    }
  }

  return (
    <article className="rounded-lg p-3" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>{formatLocalTime(event.timestamp)}</div>
          <div className="mt-1 text-sm font-medium" style={{ color: event.status === 'success' ? 'var(--cp-success)' : 'var(--cp-danger)' }}>
            {event.status}
          </div>
        </div>
        <div className="text-right">
          <div className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{formatUsd(usageFinanceAmount(event))}</div>
          <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>{formatTokens(tokens)} tokens</div>
        </div>
      </div>
      <div className="mt-3 grid grid-cols-1 gap-2 text-xs">
        <UsageCardRow label={t('aiCenter.home.filterProvider', 'Provider')} value={providerDisplayName} title={providerDisplayName === providerIdentifier ? providerDisplayName : `${providerDisplayName} (${providerIdentifier})`} expanded={expanded} />
        <UsageCardRow label={t('aiCenter.home.filterModel', 'Model')} value={event.exact_model} expanded={expanded} mono />
        <UsageCardRow label={t('aiCenter.home.filterAppAgent', 'App / Agent')} value={appAgent} expanded={expanded} />
        {event.session_id && (
          <div className="flex min-w-0 items-center justify-between gap-2">
            <span className="shrink-0" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.home.taskSession', 'Task / Session')}</span>
            <button
              type="button"
              onClick={() => onOpenTrace(event.session_id ?? '')}
              className={`${expanded ? 'break-all text-right' : 'truncate'} min-w-0 font-mono underline-offset-2 hover:underline`}
              style={{ color: 'var(--cp-accent)' }}
            >
              {event.session_id}
            </button>
          </div>
        )}
      </div>
      <div className="mt-3 flex items-center gap-2">
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="inline-flex min-h-11 flex-1 items-center justify-center gap-1 rounded-lg px-3 text-xs font-medium"
          style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
        >
          {expanded ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
          {expanded ? t('aiCenter.home.collapseFields', 'Collapse') : t('aiCenter.home.expandFields', 'Expand')}
        </button>
        <button
          type="button"
          onClick={() => void copyDetails()}
          className="inline-flex min-h-11 items-center justify-center gap-1 rounded-lg px-3 text-xs font-medium"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          {copied ? <Check size={14} /> : <Copy size={14} />}
          {copied ? t('common.copied', 'Copied') : t('common.copy', 'Copy')}
        </button>
      </div>
    </article>
  )
}

function UsageCardRow({ label, value, title, expanded, mono }: { label: string; value: string; title?: string; expanded: boolean; mono?: boolean }) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-2">
      <span className="shrink-0" style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <span
        title={title ?? value}
        className={`${expanded ? 'break-all text-right' : 'truncate'} min-w-0 ${mono ? 'font-mono' : ''}`}
        style={{ color: 'var(--cp-text)' }}
      >
        {value}
      </span>
    </div>
  )
}

function RecentTraceCard({ trace }: { trace: RouteTrace }) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const selectedCandidate = selectedTraceCandidate(trace)
  const visibleCandidates = expanded
    ? trace.ranked_candidates
    : selectedCandidate ? [selectedCandidate] : []
  const hiddenCandidateCount = Math.max(0, trace.ranked_candidates.length - visibleCandidates.length)

  return (
    <article className="rounded-lg p-3" style={{ background: 'var(--cp-bg)' }}>
      <div className="grid grid-cols-1 gap-3 md:grid-cols-[minmax(0,1fr)_minmax(260px,1.2fr)]">
        <div className="min-w-0">
          <div className="truncate text-xs" style={{ color: 'var(--cp-muted)' }}>
            {trace.requested_model}{' -> '}{trace.selected_exact_model ?? t('aiCenter.home.noExactResolved', 'unresolved')}
          </div>
          <div className="mt-1 text-sm" style={{ color: 'var(--cp-text)' }}>
            {trace.user_summary?.reason_short}
          </div>
        </div>
        <div className="min-w-0 rounded-md p-2" style={{ background: 'var(--cp-surface)' }}>
          <div className="mb-1 text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.home.traceFinalSelection', 'Final selection')}
          </div>
          {selectedCandidate ? (
            <RecentTraceCandidate candidate={selectedCandidate} selected />
          ) : (
            <div className="truncate text-xs" style={{ color: trace.selected_exact_model ? 'var(--cp-text)' : 'var(--cp-warning)' }}>
              {trace.selected_exact_model ?? t('aiCenter.home.noExactResolved', 'No exact model resolved')}
            </div>
          )}
        </div>
      </div>
      {expanded && visibleCandidates.length > 0 && (
        <div className="mt-3 flex flex-col gap-1">
          {visibleCandidates.map((candidate) => (
            <RecentTraceCandidate key={candidate.exact_model} candidate={candidate} selected={candidate.selected} />
          ))}
        </div>
      )}
      {expanded && trace.filtered_candidates.length > 0 && (
        <div className="mt-3 flex flex-col gap-1">
          {trace.filtered_candidates.map((candidate) => (
            <div key={candidate.exact_model} className="flex flex-col gap-0.5 text-xs">
              <span style={{ color: 'var(--cp-warning)' }}>{candidate.exact_model}</span>
              <span style={{ color: 'var(--cp-muted)' }}>{candidate.reason}</span>
            </div>
          ))}
        </div>
      )}
      {(trace.ranked_candidates.length > 1 || trace.filtered_candidates.length > 0) && (
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="mt-3 inline-flex min-h-8 items-center gap-1 rounded-md px-2 text-xs font-medium"
          style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
        >
          {expanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
          {expanded
            ? t('aiCenter.home.traceHideCandidates', 'Hide candidates')
            : t('aiCenter.home.traceShowCandidates', 'Show candidates ({{count}})', { count: hiddenCandidateCount + trace.filtered_candidates.length })}
        </button>
      )}
    </article>
  )
}

function RecentTraceCandidate({
  candidate,
  selected,
}: {
  candidate: RouteTrace['ranked_candidates'][number]
  selected: boolean
}) {
  return (
    <div className="flex justify-between gap-3 text-xs">
      <span className="min-w-0" style={{ color: selected ? 'var(--cp-accent)' : 'var(--cp-muted)' }}>
        <span className="block truncate">{candidate.exact_model}</span>
        <span className="block" style={{ color: 'var(--cp-muted)' }}>
          {candidateWeightSummary(candidate)}
        </span>
      </span>
      <span className="shrink-0" style={{ color: 'var(--cp-muted)' }}>{candidate.final_score?.toFixed(2)}</span>
    </div>
  )
}

function selectedTraceCandidate(trace: RouteTrace): RouteTrace['ranked_candidates'][number] | undefined {
  return trace.ranked_candidates.find((candidate) => candidate.selected)
    ?? trace.ranked_candidates.find((candidate) => candidate.exact_model === trace.selected_exact_model)
}

function mergeRouteTraces(current: RouteTrace[], next: RouteTrace[]): RouteTrace[] {
  const seen = new Set(current.map((trace) => trace.request_id))
  const merged = [...current]
  for (const trace of next) {
    if (!seen.has(trace.request_id)) {
      seen.add(trace.request_id)
      merged.push(trace)
    }
  }
  return merged
}

function mergeUsageEvents(current: UsageEvent[], next: UsageEvent[]): UsageEvent[] {
  const seen = new Set(current.map((event) => event.id))
  const merged = [...current]
  for (const event of next) {
    if (!seen.has(event.id)) {
      seen.add(event.id)
      merged.push(event)
    }
  }
  return merged
}

function usageFilterChips(
  timeRange: TimeRangeFilter,
  providerFilter: MultiFilter,
  modelFilter: MultiFilter,
  appAgentFilter: MultiFilter,
  timeOptions: Array<[TimeRangeFilter, string]>,
): string[] {
  const chips: string[] = []
  if (timeRange !== 'all') {
    chips.push(timeOptions.find(([value]) => value === timeRange)?.[1] ?? timeRange)
  }
  chips.push(...filterChips(providerFilter))
  chips.push(...filterChips(modelFilter))
  chips.push(...filterChips(appAgentFilter))
  return chips
}

function filterChips(filter: MultiFilter): string[] {
  const chips = [...filter.selected]
  if (filter.query.trim()) chips.push(filter.query.trim())
  return chips
}

function TimeRangeFilterControl({
  label,
  value,
  onChange,
  options,
  customStartDate,
  customEndDate,
  customStartLabel,
  customEndLabel,
  onCustomStartDateChange,
  onCustomEndDateChange,
}: {
  label: string
  value: TimeRangeFilter
  onChange: (value: TimeRangeFilter) => void
  options: Array<[TimeRangeFilter, string]>
  customStartDate: string
  customEndDate: string
  customStartLabel: string
  customEndLabel: string
  onCustomStartDateChange: (value: string) => void
  onCustomEndDateChange: (value: string) => void
}) {
  const activeLabel = options.find(([optionValue]) => optionValue === value)?.[1] ?? label
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [open])

  return (
    <div ref={rootRef} className="relative flex min-w-0 flex-col gap-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
      <span className="truncate" title={label}>{label}</span>
      <button
        type="button"
        onClick={() => setOpen((current) => !current)}
        className="flex h-9 cursor-pointer items-center justify-between gap-2 rounded-md px-2 text-xs"
        style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
      >
        <span className="truncate">{activeLabel}</span>
        <ChevronDown size={14} style={{ color: 'var(--cp-muted)' }} />
      </button>
      {open && (
        <div
          className="absolute left-0 top-10 z-20 flex w-full min-w-56 flex-col gap-1 rounded-md p-2 shadow-lg"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          {options.map(([optionValue, optionLabel]) => (
            <button
              key={optionValue}
              type="button"
              onClick={() => {
                onChange(optionValue)
                if (optionValue !== 'custom') {
                  setOpen(false)
                }
              }}
              className="rounded px-2 py-1.5 text-left text-xs"
              style={{
                background: optionValue === value ? 'color-mix(in oklch, var(--cp-accent), transparent 86%)' : 'transparent',
                color: optionValue === value ? 'var(--cp-accent)' : 'var(--cp-text)',
              }}
            >
              {optionLabel}
            </button>
          ))}
          {value === 'custom' && (
            <div className="grid grid-cols-1 gap-2 pt-2" style={{ borderTop: '1px solid var(--cp-border)' }}>
              <label className="flex flex-col gap-1">
                <span style={{ color: 'var(--cp-muted)' }}>{customStartLabel}</span>
                <input
                  type="date"
                  value={customStartDate}
                  onChange={(event) => onCustomStartDateChange(event.target.value)}
                  className="h-8 rounded-md px-2 text-xs outline-none"
                  style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
                />
              </label>
              <label className="flex flex-col gap-1">
                <span style={{ color: 'var(--cp-muted)' }}>{customEndLabel}</span>
                <input
                  type="date"
                  value={customEndDate}
                  onChange={(event) => onCustomEndDateChange(event.target.value)}
                  className="h-8 rounded-md px-2 text-xs outline-none"
                  style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)', color: 'var(--cp-text)' }}
                />
              </label>
            </div>
          )}
        </div>
      )}
    </div>
  )
}

function MultiSelectFilter({
  label,
  value,
  onChange,
  options,
  allLabel,
}: {
  label: string
  value: MultiFilter
  onChange: (value: MultiFilter) => void
  options: string[]
  allLabel: string
}) {
  const selectedCount = value.selected.length
  const [open, setOpen] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const toggleOption = (option: string) => {
    const selected = value.selected.includes(option)
      ? value.selected.filter((item) => item !== option)
      : [...value.selected, option]
    onChange({ ...value, selected })
  }

  useEffect(() => {
    if (!open) return
    const handlePointerDown = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) {
        setOpen(false)
      }
    }
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        setOpen(false)
      }
    }
    document.addEventListener('pointerdown', handlePointerDown)
    document.addEventListener('keydown', handleKeyDown)
    return () => {
      document.removeEventListener('pointerdown', handlePointerDown)
      document.removeEventListener('keydown', handleKeyDown)
    }
  }, [open])

  return (
    <div ref={rootRef} className="relative flex min-w-0 flex-col gap-1 text-xs" style={{ color: 'var(--cp-muted)' }}>
      <span className="truncate" title={label}>{label}</span>
      <div
        className="flex h-9 items-center rounded-md"
        style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
      >
        <input
          value={value.query}
          onChange={(event) => onChange({ ...value, query: event.target.value })}
          placeholder={selectedCount > 0 ? `${selectedCount} selected` : allLabel}
          className="h-full min-w-0 flex-1 rounded-l-md bg-transparent px-2 text-xs outline-none"
          style={{ color: 'var(--cp-text)' }}
        />
        <button
          type="button"
          onClick={() => setOpen((current) => !current)}
          className="flex h-full w-8 shrink-0 items-center justify-center rounded-r-md"
          style={{ color: selectedCount > 0 ? 'var(--cp-accent)' : 'var(--cp-muted)', borderLeft: '1px solid var(--cp-border)' }}
          aria-label={`${label} options`}
        >
          <ChevronDown size={14} />
        </button>
      </div>
      {open && (
        <div
          className="absolute left-0 top-[3.75rem] z-20 flex max-h-56 w-full min-w-48 flex-col gap-1 overflow-auto rounded-md p-2 shadow-lg"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <button
            type="button"
            onClick={() => onChange({ ...value, selected: [] })}
            className="rounded px-2 py-1 text-left text-xs"
            style={{ color: 'var(--cp-accent)' }}
          >
            {allLabel}
          </button>
          {options.map((option) => (
            <label key={option} className="flex min-h-7 items-center gap-2 rounded px-2 py-1 text-xs" style={{ color: 'var(--cp-text)' }}>
              <input
                type="checkbox"
                checked={value.selected.includes(option)}
                onChange={() => toggleOption(option)}
              />
              <span className="truncate" title={option}>{option}</span>
            </label>
          ))}
        </div>
      )}
    </div>
  )
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid h-full min-w-0 grid-rows-[2rem_auto] gap-1 content-start">
      <div className="text-xs leading-4" style={{ color: 'var(--cp-muted)' }}>{label}</div>
      <div className="text-base font-semibold leading-tight" style={{ color: 'var(--cp-text)' }}>{value}</div>
    </div>
  )
}

function MeterRow({
  label,
  value,
  max,
  onClick,
  active,
  actionLabel,
}: {
  label: string
  value: number
  max: number
  onClick?: () => void
  active?: boolean
  actionLabel?: string
}) {
  const percent = max > 0 ? Math.max(3, (value / max) * 100) : 0
  const content = (
    <>
      <div className="flex items-center justify-between gap-3 text-xs mb-1 min-w-0">
        <TruncatedText value={label} className="flex-1" />
        <span className="shrink-0 tabular-nums" style={{ color: 'var(--cp-muted)' }}>{formatTokens(value)}</span>
      </div>
      <div className="h-2 rounded-full overflow-hidden" style={{ background: 'var(--cp-bg)' }}>
        <div className="h-full rounded-full" style={{ width: `${percent}%`, background: 'var(--cp-accent)' }} />
      </div>
    </>
  )

  if (!onClick) {
    return <div className="min-w-0">{content}</div>
  }

  return (
    <button
      type="button"
      onClick={onClick}
      title={`${actionLabel}: ${label}`}
      aria-label={`${actionLabel}: ${label}`}
      className="w-full min-w-0 rounded-md p-2 text-left outline-none transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)] focus-visible:ring-2 focus-visible:ring-[color:var(--cp-accent)]"
      style={{
        border: active ? '1px solid var(--cp-accent)' : '1px solid transparent',
        background: active ? 'color-mix(in oklch, var(--cp-accent), transparent 90%)' : undefined,
      }}
    >
      {content}
    </button>
  )
}

function Breakdown({
  title,
  rows,
  total,
  activeLabel,
  onSelect,
  viewAllLabel,
  showLessLabel,
  filterLabel,
  emptyLabel,
}: {
  title: string
  rows: Array<[string, number]>
  total: number
  activeLabel?: string
  onSelect: (label: string) => void
  viewAllLabel: string
  showLessLabel: string
  filterLabel: string
  emptyLabel: string
}) {
  const [expanded, setExpanded] = useState(false)
  const hiddenCount = Math.max(0, rows.length - 4)
  const visibleRows = expanded ? rows : rows.slice(0, 4)

  return (
    <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="mb-3 flex items-center justify-between gap-3">
        <h3 className="min-w-0 truncate text-sm font-medium" title={title} style={{ color: 'var(--cp-text)' }}>
          {title}
        </h3>
        <span className="shrink-0 text-xs" style={{ color: 'var(--cp-muted)' }}>
          {rows.length}
        </span>
      </div>
      <div className={`flex flex-col gap-1 ${expanded ? 'max-h-80 overflow-y-auto pr-1' : ''}`}>
        {visibleRows.map(([label, value]) => (
          <MeterRow
            key={label}
            label={label}
            value={value}
            max={total}
            active={activeLabel === label}
            actionLabel={filterLabel}
            onClick={() => onSelect(label)}
          />
        ))}
        {rows.length === 0 && (
          <div className="rounded-md px-2 py-6 text-center text-xs" style={{ color: 'var(--cp-muted)', background: 'var(--cp-bg)' }}>
            {emptyLabel}
          </div>
        )}
      </div>
      {hiddenCount > 0 && (
        <button
          type="button"
          onClick={() => setExpanded((value) => !value)}
          className="mt-3 h-8 w-full rounded-md text-xs font-medium outline-none transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)] focus-visible:ring-2 focus-visible:ring-[color:var(--cp-accent)]"
          style={{
            color: 'var(--cp-accent)',
            border: '1px solid var(--cp-border)',
          }}
        >
          {expanded ? showLessLabel : `${viewAllLabel} (${hiddenCount})`}
        </button>
      )}
    </section>
  )
}

function TruncatedText({ value, title, className = '' }: { value: string; title?: string; className?: string }) {
  return (
    <span title={title ?? value} className={`block min-w-0 truncate ${className}`} style={{ color: 'var(--cp-text)' }}>
      {value}
    </span>
  )
}
