import { useEffect, useMemo, useRef, useState } from 'react'
import type { ReactNode } from 'react'
import { useMediaQuery } from '@mui/material'
import {
  Activity,
  AlertTriangle,
  ArrowLeft,
  Box,
  Braces,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Cloud,
  Code2,
  Copy,
  Cpu,
  DollarSign,
  FileText,
  Filter,
  FolderTree,
  GitBranch,
  Image,
  Layers,
  MessageSquare,
  Route,
  Search,
  Sparkles,
  Zap,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import {
  useLocalModels,
  useProviders,
  useRouteTraces,
  useGlobalRoutingView,
  useAICCStore,
} from './hooks/use-aicc-store'
import { StatusBadge } from './components/shared/StatusBadge'
import type { LogicalNode, ModelMetadata, RouteTrace, RoutingDirectoryView } from '../../api/aicc_mgr'
import { PagedListFooter } from './components/shared/paged-list'
import { LongField } from './components/shared/LongField'

type FilterKey = 'provider' | 'apiType' | 'capability' | 'cost' | 'latency' | 'health' | 'location'

type MultiFilter = {
  query: string
  selected: string[]
}

type RoutingFilters = Record<FilterKey, MultiFilter>

type ScenarioView = {
  node: LogicalNode
  useCase: UseCaseKind
  title: string
  description: string
  selectedModel?: ModelMetadata
  selectedExactModel?: string
  trace?: RouteTrace
  candidates: ModelMetadata[]
  groups: ModelGroup[]
  score: number
}

type ModelGroup = {
  key: string
  primary: ModelMetadata
  variants: ModelMetadata[]
}

type UseCaseKind = 'chat' | 'code' | 'plan' | 'image' | 'embed' | 'vision' | 'audio' | 'other'
type TraceOutcomeFilter = 'all' | 'fallback' | 'failed' | 'warning'
type TraceCandidateSection = 'none' | 'ranked' | 'filtered'
type TraceEmptyStateKind = 'none-yet' | 'load-failed' | 'no-matches'

function emptyMultiFilter(): MultiFilter {
  return { query: '', selected: [] }
}

function defaultRoutingFilters(): RoutingFilters {
  return {
    provider: emptyMultiFilter(),
    apiType: emptyMultiFilter(),
    capability: emptyMultiFilter(),
    cost: emptyMultiFilter(),
    latency: emptyMultiFilter(),
    health: emptyMultiFilter(),
    location: emptyMultiFilter(),
  }
}

const USE_CASE_ORDER: UseCaseKind[] = ['chat', 'code', 'plan', 'image', 'embed', 'vision', 'audio', 'other']
const ROUTE_TRACE_PAGE_SIZE = 20

export function RoutingPage() {
  const { t } = useI18n()
  const store = useAICCStore()
  const routingView = useGlobalRoutingView()
  const snapshotTraces = useRouteTraces()
  const providers = useProviders()
  const localModels = useLocalModels()
  const isMobile = useMediaQuery('(max-width: 767px)')
  const isCompactDesktop = useMediaQuery('(min-width: 768px) and (max-width: 1100px)')
  const [query, setQuery] = useState('')
  const [filters, setFilters] = useState<RoutingFilters>(() => defaultRoutingFilters())
  const [selectedPath, setSelectedPath] = useState<string | null>(null)
  const [currentPath, setCurrentPath] = useState<string | null>(null)
  const [showMobileScenarioDetail, setShowMobileScenarioDetail] = useState(false)
  const [mobileScenarioPane, setMobileScenarioPane] = useState<'scenario' | 'trace'>('scenario')
  const [filtersOpen, setFiltersOpen] = useState(false)
  const [traces, setTraces] = useState<RouteTrace[]>(snapshotTraces)
  const [traceNextCursor, setTraceNextCursor] = useState<string | undefined>()
  const [tracePageIndex, setTracePageIndex] = useState(0)
  const [traceLoading, setTraceLoading] = useState(false)
  const [traceError, setTraceError] = useState<'initial' | 'more' | null>(null)
  const [traceOutcomeFilter, setTraceOutcomeFilter] = useState<TraceOutcomeFilter>('all')
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null)

  const snapshotModels = useMemo(() => [
    ...providers.flatMap((provider) => provider.status.discovered_models),
    ...localModels,
  ], [providers, localModels])
  const [directoryView, setDirectoryView] = useState<RoutingDirectoryView>(() => ({
    routingView,
    models: snapshotModels,
  }))

  useEffect(() => {
    let cancelled = false
    async function loadDirectory() {
      try {
        const view = await store.queryRoutingDirectory(currentPath)
        if (!cancelled) {
          setDirectoryView(view)
        }
      } catch (error) {
        console.error('aicc.models.list logical_path failed', error)
        if (!cancelled) {
          setDirectoryView({
            routingView: {
              ...routingView,
              logical_tree: currentPath ? childNodesAtPath(routingView.logical_tree, currentPath) : routingView.logical_tree,
            },
            models: snapshotModels,
          })
        }
      }
    }
    void loadDirectory()
    return () => {
      cancelled = true
    }
  }, [currentPath, routingView, snapshotModels, store])

  useEffect(() => {
    let cancelled = false
    async function loadInitialTraces() {
      try {
        const page = await store.queryRouteTraces({ limit: ROUTE_TRACE_PAGE_SIZE })
        if (!cancelled) {
          setTraces(page.traces)
          setTraceNextCursor(page.nextCursor)
          setTracePageIndex(0)
          setTraceError(null)
        }
      } catch (error) {
        console.error('aicc.trace.query initial page failed', error)
        if (!cancelled) {
          setTraces(snapshotTraces)
          setTraceNextCursor(snapshotTraces.length >= ROUTE_TRACE_PAGE_SIZE ? String(ROUTE_TRACE_PAGE_SIZE) : undefined)
          setTracePageIndex(0)
          setTraceError('initial')
        }
      }
    }
    void loadInitialTraces()
    return () => {
      cancelled = true
    }
  }, [snapshotTraces, store])

  const activeRoutingView = directoryView.routingView
  const models = directoryView.models

  const providerNames = useMemo(() => new Map([
    ...providers.map((provider) => [
      provider.config.provider_instance_name,
      provider.config.name,
    ] as const),
    ['local', t('aiCenter.routing.localProvider', 'Local runtime')] as const,
  ]), [providers, t])

  const directoryNodes = activeRoutingView.logical_tree
  const scenarios = useMemo(() => buildScenarios(directoryNodes, models, traces), [
    directoryNodes,
    models,
    traces,
  ])
  const filterOptions = useMemo(() => buildFilterOptions(models), [models])
  const visibleScenarios = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase()
    return scenarios
      .filter((scenario) => scenarioMatchesQuery(scenario, normalizedQuery))
      .filter((scenario) => scenarioMatchesFilters(scenario, filters))
      .sort(compareScenario)
  }, [scenarios, query, filters])
  const scenarioByPath = useMemo(
    () => new Map(visibleScenarios.map((scenario) => [scenario.node.path, scenario])),
    [visibleScenarios],
  )
  const allScenarioByPath = useMemo(
    () => new Map(scenarios.map((scenario) => [scenario.node.path, scenario])),
    [scenarios],
  )
  const queryActive = query.trim().length > 0 || Object.values(filters).some((value) => value.query.trim().length > 0 || value.selected.length > 0)
  const directoryEntries = visibleScenarios
  const selectedScenario = visibleScenarios.find((scenario) => scenario.node.path === selectedPath)
    ?? directoryEntries[0]
    ?? visibleScenarios[0]
  const selectedTracePath = selectedTraceId
    ? traceLogicalPath(traces.find((trace) => trace.request_id === selectedTraceId))
    : null
  const activeTracePath = selectedTracePath ?? selectedPath
  const visibleTraces = useMemo(
    () => traces
      .filter((trace) => !activeTracePath || traceMatchesScenarioPath(trace, activeTracePath))
      .filter((trace) => traceMatchesOutcome(trace, traceOutcomeFilter)),
    [activeTracePath, traceOutcomeFilter, traces],
  )
  const traceCostByExactModel = useMemo(
    () => new Map(models.map((model) => [
      model.exact_model,
      (model.pricing.input_token_usd ?? 0) + (model.pricing.output_token_usd ?? 0),
    ] as const)),
    [models],
  )
  const traceEmptyState = traceEmptyStateKind(traces.length, visibleTraces.length, traceError, Boolean(activeTracePath), traceOutcomeFilter)
  const retryTraceLoad = () => {
    if (traceError === 'initial') {
      void loadTracePage(tracePageIndex)
    } else {
      void loadMoreTraces()
    }
  }

  if (routingView.logical_tree.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-16 text-center">
        <GitBranch size={40} style={{ color: 'var(--cp-muted)' }} />
        <p className="text-sm mt-3" style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.routing.notConfigured', 'No logical directory configured')}
        </p>
      </div>
    )
  }

  const updateFilter = (key: FilterKey, value: MultiFilter) => {
    setFilters((current) => ({ ...current, [key]: value }))
  }

  const loadTracePage = async (pageIndex: number) => {
    if (traceLoading) return
    const nextPageIndex = Math.max(0, pageIndex)
    setTraceLoading(true)
    setTraceError(null)
    try {
      const page = await store.queryRouteTraces({
        limit: ROUTE_TRACE_PAGE_SIZE,
        cursor: nextPageIndex > 0 ? String(nextPageIndex * ROUTE_TRACE_PAGE_SIZE) : undefined,
      })
      setTraces(page.traces)
      setTraceNextCursor(page.nextCursor)
      setTracePageIndex(nextPageIndex)
    } catch (error) {
      console.error('aicc.trace.query page failed', error)
      setTraceError('initial')
    } finally {
      setTraceLoading(false)
    }
  }

  const loadMoreTraces = async () => {
    if (!traceNextCursor || traceLoading) return
    setTraceLoading(true)
    setTraceError(null)
    try {
      const page = await store.queryRouteTraces({
        limit: ROUTE_TRACE_PAGE_SIZE,
        cursor: traceNextCursor,
      })
      setTraces((current) => mergeRouteTraces(current, page.traces))
      setTraceNextCursor(page.nextCursor)
    } catch (error) {
      console.error('aicc.trace.query next page failed', error)
      setTraceError('more')
    } finally {
      setTraceLoading(false)
    }
  }

  return (
    <div className="flex flex-col gap-4">
      <RoutingHeader revision={activeRoutingView.revision} scenarioCount={visibleScenarios.length} />

      <RoutingFiltersBar
        query={query}
        filters={filters}
        options={filterOptions}
        onQueryChange={setQuery}
        onFilterChange={updateFilter}
        filtersOpen={filtersOpen}
        onToggleFilters={() => setFiltersOpen((value) => !value)}
      />

      {!queryActive && (
        <RoutingBreadcrumbs
          currentPath={currentPath}
          scenarios={scenarioByPath}
          onNavigate={(path) => {
            setCurrentPath(path)
            setSelectedPath(path)
            setShowMobileScenarioDetail(false)
          }}
        />
      )}

      {isMobile && showMobileScenarioDetail && selectedScenario ? (
        <div className="flex flex-col gap-4">
          <div className="flex min-h-11 items-center gap-1 rounded-xl p-1" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <button
              type="button"
              onClick={() => setShowMobileScenarioDetail(false)}
              className="inline-flex min-h-9 items-center gap-1 rounded-lg px-2 text-xs font-medium"
              style={{ color: 'var(--cp-accent)' }}
            >
              <ArrowLeft size={15} />
              {t('aiCenter.routing.mainPage', 'Routing')}
            </button>
            {([
              ['scenario', t('aiCenter.routing.scenarioInfo', 'Scenario')],
              ['trace', t('aiCenter.routing.tracePage', 'Trace')],
            ] as Array<['scenario' | 'trace', string]>).map(([pane, label]) => (
              <button
                key={pane}
                type="button"
                onClick={() => setMobileScenarioPane(pane)}
                className="min-h-9 flex-1 rounded-lg px-2 text-xs font-medium"
                style={{
                  background: mobileScenarioPane === pane ? 'var(--cp-surface-2)' : 'transparent',
                  color: mobileScenarioPane === pane ? 'var(--cp-text)' : 'var(--cp-muted)',
                  border: mobileScenarioPane === pane ? '1px solid var(--cp-border)' : '1px solid transparent',
                }}
              >
                {label}
              </button>
            ))}
          </div>
          {mobileScenarioPane === 'scenario' ? (
            <ScenarioInspector scenario={selectedScenario} providerNames={providerNames} />
          ) : (
            <TraceExplorer
              traces={visibleTraces}
              loadedCount={traces.length}
              compact={isMobile}
              outcomeFilter={traceOutcomeFilter}
              activeLogicalPath={activeTracePath}
              activeTraceId={selectedTraceId}
              hasMore={Boolean(traceNextCursor)}
              pageIndex={tracePageIndex}
              canGoPrevious={false}
              canGoNext={false}
              loading={traceLoading}
              error={traceError}
              onOutcomeFilterChange={setTraceOutcomeFilter}
              onLoadMore={loadMoreTraces}
              onRetry={retryTraceLoad}
              onPreviousPage={() => undefined}
              onNextPage={() => undefined}
              onTraceSelect={(trace) => {
                const logicalPath = traceLogicalPath(trace)
                setSelectedTraceId(trace.request_id)
                if (logicalPath && allScenarioByPath.has(logicalPath)) {
                  setSelectedPath(logicalPath)
                }
              }}
              onClearScenarioFilter={() => {
                setSelectedTraceId(null)
                setSelectedPath(null)
              }}
              emptyState={traceEmptyState}
              costByExactModel={traceCostByExactModel}
            />
          )}
        </div>
      ) : (
      <div className={isMobile || isCompactDesktop ? 'flex flex-col gap-4' : 'grid grid-cols-[220px_minmax(0,1fr)_360px] gap-4 items-start'}>
        {!isMobile && (
          <DirectoryNavigator
            nodes={routingView.logical_tree}
            currentPath={currentPath}
            selectedPath={selectedPath}
            onNavigate={(path) => {
              setCurrentPath(path)
              setSelectedPath(path)
              setSelectedTraceId(null)
              setShowMobileScenarioDetail(false)
            }}
          />
        )}
        <section className="flex min-w-0 flex-col gap-3">
          <div className="flex items-center justify-between gap-3 rounded-xl px-3 py-2" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
            <div className="min-w-0">
              <div className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
                {t('aiCenter.routing.currentDirectory', 'Current directory')}
              </div>
              <LongField value={currentPath ?? 'Routing'} className="text-sm" mono copyable={Boolean(currentPath)} />
            </div>
            <span className="shrink-0 text-xs" style={{ color: 'var(--cp-muted)' }}>
              {directoryEntries.length} {t('aiCenter.routing.scenarios', 'scenarios')}
            </span>
          </div>
          {directoryEntries.length > 0 ? directoryEntries.map((scenario) => (
            <ScenarioCard
              key={scenario.node.path}
              scenario={scenario}
              providerNames={providerNames}
              hasChildren={!queryActive && canNavigateIntoPath(routingView.logical_tree, scenario.node.path)}
              selected={selectedScenario?.node.path === scenario.node.path}
              onSelect={() => {
                setSelectedPath(scenario.node.path)
                setSelectedTraceId(null)
                if (isMobile) {
                  setMobileScenarioPane('scenario')
                  setShowMobileScenarioDetail(true)
                }
              }}
              onOpen={() => {
                setCurrentPath(scenario.node.path)
                setSelectedPath(scenario.node.path)
                setShowMobileScenarioDetail(false)
              }}
            />
          )) : (
            <EmptyResults />
          )}
        </section>

        <aside className="flex min-w-0 flex-col gap-4">
          {selectedScenario && (
            <ScenarioInspector scenario={selectedScenario} providerNames={providerNames} />
          )}
          {!isMobile && <TraceExplorer
            traces={visibleTraces}
            loadedCount={traces.length}
            compact={false}
            outcomeFilter={traceOutcomeFilter}
            activeLogicalPath={activeTracePath}
            activeTraceId={selectedTraceId}
            hasMore={Boolean(traceNextCursor)}
            pageIndex={tracePageIndex}
            canGoPrevious={tracePageIndex > 0}
            canGoNext={Boolean(traceNextCursor)}
            loading={traceLoading}
            error={traceError}
            onOutcomeFilterChange={setTraceOutcomeFilter}
            onLoadMore={loadMoreTraces}
            onRetry={retryTraceLoad}
            onPreviousPage={() => void loadTracePage(tracePageIndex - 1)}
            onNextPage={() => void loadTracePage(tracePageIndex + 1)}
            onTraceSelect={(trace) => {
              const logicalPath = traceLogicalPath(trace)
              setSelectedTraceId(trace.request_id)
              if (logicalPath && allScenarioByPath.has(logicalPath)) {
                setSelectedPath(logicalPath)
              }
            }}
            onClearScenarioFilter={() => {
              setSelectedTraceId(null)
              setSelectedPath(null)
            }}
            emptyState={traceEmptyState}
            costByExactModel={traceCostByExactModel}
          />}
        </aside>
      </div>
      )}
    </div>
  )
}

function RoutingHeader({ revision, scenarioCount }: { revision?: string; scenarioCount: number }) {
  const { t } = useI18n()
  return (
    <div className="flex flex-col gap-1">
      <h2 className="text-lg font-semibold" style={{ color: 'var(--cp-text)' }}>
        {t('aiCenter.routing.title', 'Routing by Scenario')}
      </h2>
      <p className="text-sm" style={{ color: 'var(--cp-muted)' }}>
        {t('aiCenter.routing.subtitle', 'Read-only view of which model each logical path will prefer now. Variants are folded under their base model.')}
      </p>
      <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {t('aiCenter.routing.revision', 'Revision')}: {revision ?? '-'} / {t('aiCenter.routing.scenarioCount', '{{count}} scenarios', { count: scenarioCount })}
      </div>
    </div>
  )
}

function RoutingFiltersBar({
  query,
  filters,
  options,
  onQueryChange,
  onFilterChange,
  filtersOpen,
  onToggleFilters,
}: {
  query: string
  filters: RoutingFilters
  options: Record<FilterKey, string[]>
  onQueryChange: (value: string) => void
  onFilterChange: (key: FilterKey, value: MultiFilter) => void
  filtersOpen: boolean
  onToggleFilters: () => void
}) {
  const { t } = useI18n()
  const activeFilterCount = Object.values(filters).reduce((count, filter) => count + filter.selected.length + (filter.query.trim() ? 1 : 0), 0)
  return (
    <section
      className="rounded-xl p-3 flex flex-col gap-3"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
    >
      <div className="relative flex items-center gap-2">
        <Search size={17} style={{ color: 'var(--cp-muted)' }} />
        <input
          value={query}
          onChange={(event) => onQueryChange(event.target.value)}
          placeholder={t('aiCenter.routing.search', 'Search logical path, model, provider, capability...')}
          className="min-h-10 flex-1 rounded-lg px-3 pr-12 text-sm outline-none"
          style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        />
        <button
          type="button"
          onClick={onToggleFilters}
          className="absolute right-1.5 top-1/2 flex h-7 min-w-7 -translate-y-1/2 items-center justify-center gap-1 rounded-md px-1.5 text-xs"
          style={{
            color: filtersOpen || activeFilterCount > 0 ? 'var(--cp-accent)' : 'var(--cp-muted)',
            background: filtersOpen ? 'var(--cp-surface)' : 'transparent',
          }}
          aria-label={t('aiCenter.routing.filters', 'Filters')}
        >
          <Filter size={14} />
          {activeFilterCount > 0 && <span>{activeFilterCount}</span>}
        </button>
      </div>

      {filtersOpen && <div className="grid grid-cols-1 sm:grid-cols-2 xl:grid-cols-4 gap-2">
        <MultiSelectFilter label={t('aiCenter.routing.provider', 'Provider')} value={filters.provider} options={options.provider} onChange={(value) => onFilterChange('provider', value)} />
        <MultiSelectFilter label={t('aiCenter.routing.apiType', 'API Type')} value={filters.apiType} options={options.apiType} onChange={(value) => onFilterChange('apiType', value)} />
        <MultiSelectFilter label={t('aiCenter.routing.capability', 'Capability')} value={filters.capability} options={options.capability} onChange={(value) => onFilterChange('capability', value)} />
        <MultiSelectFilter label={t('aiCenter.routing.cost', 'Cost')} value={filters.cost} options={options.cost} onChange={(value) => onFilterChange('cost', value)} />
        <MultiSelectFilter label={t('aiCenter.routing.latency', 'Latency')} value={filters.latency} options={options.latency} onChange={(value) => onFilterChange('latency', value)} />
        <MultiSelectFilter label={t('aiCenter.routing.health', 'Health')} value={filters.health} options={options.health} onChange={(value) => onFilterChange('health', value)} />
        <MultiSelectFilter label={t('aiCenter.routing.location', 'Local/Cloud')} value={filters.location} options={options.location} onChange={(value) => onFilterChange('location', value)} />
      </div>}
    </section>
  )
}

function MultiSelectFilter({
  label,
  value,
  options,
  onChange,
}: {
  label: string
  value: MultiFilter
  options: string[]
  onChange: (value: MultiFilter) => void
}) {
  const { t } = useI18n()
  const selectedCount = value.selected.length
  const [open, setOpen] = useState(false)
  const [showAllOptions, setShowAllOptions] = useState(false)
  const rootRef = useRef<HTMLDivElement | null>(null)
  const visibleOptions = showAllOptions
    ? options
    : Array.from(new Set([...options.slice(0, 6), ...value.selected]))
  const hiddenOptionCount = Math.max(0, options.length - visibleOptions.length)
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
        className="flex min-h-8 items-center rounded-md"
        style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
      >
        <input
          value={value.query}
          onChange={(event) => onChange({ ...value, query: event.target.value })}
          placeholder={selectedCount > 0 ? `${selectedCount} selected` : 'All'}
          className="min-w-0 flex-1 rounded-l-md bg-transparent px-2 text-xs outline-none"
          style={{ color: 'var(--cp-text)' }}
        />
        <button
          type="button"
          onClick={() => setOpen((current) => !current)}
          className="flex h-8 w-8 shrink-0 items-center justify-center rounded-r-md"
          style={{ color: selectedCount > 0 ? 'var(--cp-accent)' : 'var(--cp-muted)', borderLeft: '1px solid var(--cp-border)' }}
          aria-label={`${label} options`}
        >
          <ChevronDown size={14} />
        </button>
      </div>
      {open && (
        <div
          className="absolute left-0 top-[3.35rem] z-20 flex max-h-56 w-full min-w-48 flex-col gap-1 overflow-auto rounded-md p-2 shadow-lg"
          style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
        >
          <button
            type="button"
            onClick={() => onChange({ ...value, selected: [] })}
            className="rounded px-2 py-1 text-left text-xs"
            style={{ color: 'var(--cp-accent)' }}
          >
            All
          </button>
          {visibleOptions.map((option) => (
            <label key={option} className="flex min-h-7 items-center gap-2 rounded px-2 py-1 text-xs" style={{ color: 'var(--cp-text)' }}>
              <input
                type="checkbox"
                checked={value.selected.includes(option)}
                onChange={() => toggleOption(option)}
              />
              <span className="truncate" title={option}>{option}</span>
            </label>
          ))}
          {hiddenOptionCount > 0 && (
            <button
              type="button"
              onClick={() => setShowAllOptions(true)}
              className="rounded px-2 py-1 text-left text-xs"
              style={{ color: 'var(--cp-accent)' }}
            >
              {t('aiCenter.routing.showMoreOptions', 'Show more')} ({hiddenOptionCount})
            </button>
          )}
          {showAllOptions && options.length > 6 && (
            <button
              type="button"
              onClick={() => setShowAllOptions(false)}
              className="rounded px-2 py-1 text-left text-xs"
              style={{ color: 'var(--cp-accent)' }}
            >
              {t('aiCenter.routing.showLessOptions', 'Show less')}
            </button>
          )}
        </div>
      )}
    </div>
  )
}

function DirectoryNavigator({
  nodes,
  currentPath,
  selectedPath,
  onNavigate,
}: {
  nodes: LogicalNode[]
  currentPath: string | null
  selectedPath: string | null
  onNavigate: (path: string | null) => void
}) {
  const { t } = useI18n()
  return (
    <aside className="sticky top-4 flex max-h-[calc(100dvh-10rem)] min-w-0 flex-col overflow-hidden rounded-xl" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="flex items-center justify-between gap-2 px-3 py-2" style={{ borderBottom: '1px solid var(--cp-border)' }}>
        <div className="flex min-w-0 items-center gap-2">
          <FolderTree size={15} style={{ color: 'var(--cp-accent)' }} />
          <span className="truncate text-xs font-medium" style={{ color: 'var(--cp-text)' }}>
            {t('aiCenter.routing.logicalDirectory', 'Logical directory')}
          </span>
        </div>
        <button
          type="button"
          onClick={() => onNavigate(null)}
          className="shrink-0 rounded-md px-2 py-1 text-xs"
          style={{ color: currentPath == null ? 'var(--cp-text)' : 'var(--cp-accent)' }}
        >
          {t('aiCenter.routing.root', 'Root')}
        </button>
      </div>
      <div className="min-h-0 overflow-y-auto p-2">
        <DirectoryNodeList
          nodes={nodes}
          depth={0}
          currentPath={currentPath}
          selectedPath={selectedPath}
          onNavigate={onNavigate}
        />
      </div>
    </aside>
  )
}

function DirectoryNodeList({
  nodes,
  depth,
  currentPath,
  selectedPath,
  onNavigate,
}: {
  nodes: LogicalNode[]
  depth: number
  currentPath: string | null
  selectedPath: string | null
  onNavigate: (path: string) => void
}) {
  return (
    <div className="flex flex-col gap-1">
      {nodes.filter(isLogicalDirectoryNode).map((node) => {
        const active = node.path === currentPath || node.path === selectedPath
        const children = (node.children ?? []).filter(isLogicalDirectoryNode)
        return (
          <div key={node.path} className="min-w-0">
            <button
              type="button"
              onClick={() => onNavigate(node.path)}
              className="flex min-h-8 w-full min-w-0 items-center gap-2 rounded-lg px-2 text-left text-xs"
              title={node.path}
              style={{
                paddingLeft: `${8 + depth * 12}px`,
                background: active ? 'var(--cp-surface-2)' : 'transparent',
                color: active ? 'var(--cp-text)' : 'var(--cp-muted)',
                border: active ? '1px solid var(--cp-border)' : '1px solid transparent',
              }}
            >
              {children.length > 0 ? <FolderTree size={13} className="shrink-0" /> : <Box size={13} className="shrink-0" />}
              <span className="min-w-0 truncate font-mono">{lastPathSegment(node.path)}</span>
            </button>
            {children.length > 0 && depth < 3 && (
              <DirectoryNodeList
                nodes={children}
                depth={depth + 1}
                currentPath={currentPath}
                selectedPath={selectedPath}
                onNavigate={onNavigate}
              />
            )}
          </div>
        )
      })}
    </div>
  )
}

function RoutingBreadcrumbs({
  currentPath,
  scenarios,
  onNavigate,
}: {
  currentPath: string | null
  scenarios: Map<string, ScenarioView>
  onNavigate: (path: string | null) => void
}) {
  const parts = breadcrumbPaths(currentPath)
  return (
    <nav
      className="flex flex-wrap items-center gap-1 rounded-xl px-3 py-2 text-sm"
      style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}
      aria-label="Routing breadcrumb"
    >
      <button
        type="button"
        onClick={() => onNavigate(null)}
        className="rounded-md px-2 py-1 text-xs font-medium"
        style={{ color: currentPath == null ? 'var(--cp-text)' : 'var(--cp-accent)' }}
      >
        Routing
      </button>
      {parts.map((path) => (
        <span key={path} className="inline-flex items-center gap-1">
          <ChevronRight size={13} style={{ color: 'var(--cp-muted)' }} />
          <button
            type="button"
            onClick={() => onNavigate(path)}
            className="max-w-[180px] truncate rounded-md px-2 py-1 text-xs font-medium"
            title={path}
            style={{ color: path === currentPath ? 'var(--cp-text)' : 'var(--cp-accent)' }}
          >
            {scenarios.get(path)?.title ?? lastPathSegment(path)}
          </button>
        </span>
      ))}
    </nav>
  )
}

function ScenarioCard({
  scenario,
  providerNames,
  hasChildren,
  selected,
  onSelect,
  onOpen,
}: {
  scenario: ScenarioView
  providerNames: Map<string, string>
  hasChildren: boolean
  selected: boolean
  onSelect: () => void
  onOpen: () => void
}) {
  const { t } = useI18n()
  const primary = scenario.selectedModel
  const status = primary
    ? primary.health.status === 'available' ? 'ok' : primary.health.status === 'degraded' ? 'warning' : 'error'
    : 'warning'
  return (
    <article
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onSelect()
        }
      }}
      className="w-full rounded-xl p-4 text-left"
      style={{
        background: selected ? 'var(--cp-surface-2)' : 'var(--cp-surface)',
        border: '1px solid var(--cp-border)',
      }}
    >
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0 flex items-start gap-3">
          {hasChildren ? (
            <button
              type="button"
              onClick={(event) => {
                event.stopPropagation()
                onOpen()
              }}
              onKeyDown={(event) => {
                if (event.key === 'Enter' || event.key === ' ') {
                  event.preventDefault()
                  event.stopPropagation()
                  onOpen()
                }
              }}
              className="relative flex h-11 w-11 shrink-0 items-center justify-center rounded-lg transition-shadow hover:shadow-md"
              style={{
                background: 'var(--cp-bg)',
                color: 'var(--cp-accent)',
                border: '1px solid var(--cp-border)',
              }}
              aria-label={`Open ${scenario.node.path}`}
            >
              <FolderTree size={19} />
              <span
                className="absolute -bottom-1 -right-1 flex h-5 w-5 items-center justify-center rounded-full"
                style={{ background: 'var(--cp-accent)', color: '#fff', border: '2px solid var(--cp-surface)' }}
                aria-hidden
              >
                <ChevronRight size={12} />
              </span>
            </button>
          ) : (
            <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-lg" style={{ background: 'var(--cp-bg)', color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}>
              <Box size={18} />
            </div>
          )}
          <div className="min-w-0">
            <div className="flex flex-wrap items-center gap-2">
              <h3 className="text-base font-semibold" style={{ color: 'var(--cp-text)' }}>{scenario.title}</h3>
              <LongField value={scenario.node.path} className="text-xs" mono tone="muted" copyable={false} />
            </div>
            <p className="text-sm mt-1" style={{ color: 'var(--cp-muted)' }}>{scenario.description}</p>
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <StatusBadge status={status} label={primary?.health.status ?? t('aiCenter.routing.unresolved', 'unresolved')} />
        </div>
      </div>

      <div className="mt-4 rounded-lg p-3" style={{ background: 'var(--cp-bg)' }}>
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="min-w-0">
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
              {t('aiCenter.routing.currentPreferred', 'Current preferred model')}
            </div>
            <LongField
              value={primary?.provider_model_id ?? scenario.selectedExactModel ?? t('aiCenter.routing.noModel', 'No model resolved')}
              className="text-sm font-medium"
              copyable={Boolean(primary?.provider_model_id ?? scenario.selectedExactModel)}
            />
            <LongField
              value={primary ? `${providerNames.get(providerFromExact(primary.exact_model)) ?? providerFromExact(primary.exact_model)} / ${primary.exact_model}` : scenario.node.fallback?.target ?? '-'}
              className="text-xs"
              mono
              tone="muted"
              expandable
            />
          </div>
          {primary && (
            <div className="flex flex-wrap items-center gap-2">
              <MetricChip icon={<Sparkles size={13} />} label="Q" value={formatQuality(primary.attributes.quality_score)} />
              <MetricChip icon={<Zap size={13} />} label="Latency" value={formatLatency(primary)} />
              <MetricChip icon={<DollarSign size={13} />} label="Cost" value={primary.attributes.cost_class} />
              <MetricChip icon={primary.attributes.local ? <Cpu size={13} /> : <Cloud size={13} />} label="Run" value={primary.attributes.local ? 'local' : 'cloud'} />
            </div>
          )}
        </div>
      </div>

      <div className="mt-3 flex flex-wrap items-center gap-2 text-xs" style={{ color: 'var(--cp-muted)' }}>
        <span>{scenario.node.policy?.profile ?? 'balanced'}</span>
        <span>/</span>
        <span>{scenario.node.api_type ?? 'mixed'}</span>
        <span>/</span>
        <span>{scenario.groups.length} {t('aiCenter.routing.baseModels', 'base models')}</span>
        <span>/</span>
        <span>{scenario.groups.reduce((count, group) => count + group.variants.length, 0)} {t('aiCenter.routing.foldedVariants', 'folded variants')}</span>
      </div>
    </article>
  )
}

function ScenarioInspector({
  scenario,
  providerNames,
}: {
  scenario: ScenarioView
  providerNames: Map<string, string>
}) {
  const { t } = useI18n()
  const [expanded, setExpanded] = useState(false)
  const visibleGroups = expanded ? scenario.groups : scenario.groups.slice(0, 4)

  return (
    <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="flex items-start justify-between gap-3 mb-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <UseCaseIcon kind={scenario.useCase} small />
            <h3 className="text-sm font-semibold" style={{ color: 'var(--cp-text)' }}>
              {t('aiCenter.routing.scenarioDetail', 'Scenario Detail')}
            </h3>
          </div>
          <LongField value={scenario.node.path} className="mt-1 text-xs" mono tone="muted" />
        </div>
        <StatusBadge status={scenario.selectedModel ? 'ok' : 'warning'} label={scenario.selectedModel ? 'resolved' : 'unresolved'} />
      </div>

      <div className="grid grid-cols-1 gap-2 text-sm">
        <Fact label={t('aiCenter.routing.useCase', 'Use case')} value={scenario.title} />
        <Fact label={t('aiCenter.routing.apiType', 'API Type')} value={scenario.node.api_type ?? 'mixed'} />
        <Fact label={t('aiCenter.routing.profile', 'Profile')} value={scenario.node.policy?.profile ?? 'balanced'} />
        <Fact label={t('aiCenter.routing.required', 'Required')} value={scenario.node.policy?.required_features?.join(', ') || 'none'} />
        <Fact label={t('aiCenter.routing.fallback', 'Fallback')} value={`${scenario.node.fallback?.mode ?? 'inherit'}${scenario.node.fallback?.target ? ` -> ${scenario.node.fallback.target}` : ''}`} />
      </div>

      <div className="mt-4">
        <div className="flex items-center justify-between gap-2 mb-2">
          <h4 className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.routing.rankedBaseModels', 'Ranked base models')}
          </h4>
          {scenario.groups.length > 4 && (
            <button
              type="button"
              onClick={() => setExpanded((value) => !value)}
              className="inline-flex items-center gap-1 text-xs"
              style={{ color: 'var(--cp-accent)' }}
            >
              {expanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
              {expanded ? t('aiCenter.routing.showLess', 'Show less') : t('aiCenter.routing.showAll', 'Show all')}
            </button>
          )}
        </div>
        <div className="flex flex-col gap-2">
          {visibleGroups.map((group, index) => (
          <ModelGroupRow
              key={group.key}
              index={index}
              group={group}
              selected={modelGroupHasExact(group, scenario.selectedModel?.exact_model)}
              providerNames={providerNames}
            />
          ))}
          {scenario.groups.length === 0 && (
            <div className="flex items-start gap-2 rounded-lg p-3" style={{ background: 'var(--cp-bg)' }}>
              <AlertTriangle size={16} style={{ color: 'var(--cp-warning)' }} />
              <div className="text-sm" style={{ color: 'var(--cp-text)' }}>
                {t('aiCenter.routing.noCandidates', 'No discovered model currently matches this logical path and policy.')}
              </div>
            </div>
          )}
        </div>
      </div>
    </section>
  )
}

function ModelGroupRow({
  index,
  group,
  selected,
  providerNames,
}: {
  index: number
  group: ModelGroup
  selected: boolean
  providerNames: Map<string, string>
}) {
  const model = group.primary
  return (
    <article
      className="rounded-lg p-3"
      style={{
        background: selected ? 'var(--cp-surface-2)' : 'var(--cp-bg)',
        border: `1px solid ${selected ? 'var(--cp-accent)' : 'transparent'}`,
      }}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>#{index + 1}</span>
            <LongField value={model.provider_model_id} className="text-sm font-medium" />
          </div>
          <LongField
            value={`${providerNames.get(providerFromExact(model.exact_model)) ?? providerFromExact(model.exact_model)} / ${model.exact_model}`}
            className="mt-1 text-xs"
            mono
            tone="muted"
            expandable
          />
        </div>
        <StatusBadge status={model.health.status === 'available' ? 'ok' : model.health.status === 'degraded' ? 'warning' : 'error'} label={model.health.status} />
      </div>
      <div className="flex flex-wrap gap-1.5 mt-3">
        <MetricChip icon={<Sparkles size={13} />} label="Q" value={formatQuality(model.attributes.quality_score)} />
        <MetricChip icon={<Zap size={13} />} label="Latency" value={formatLatency(model)} />
        <MetricChip icon={<DollarSign size={13} />} label="Cost" value={model.attributes.cost_class} />
        <MetricChip icon={<Layers size={13} />} label="Variants" value={group.variants.length.toString()} />
      </div>
      {group.variants.length > 0 && (
        <div className="mt-2 text-xs truncate" style={{ color: 'var(--cp-muted)' }}>
          <LongField value={group.variants.map((variant) => variant.provider_model_id).join(', ')} tone="muted" copyable={false} expandable />
        </div>
      )}
    </article>
  )
}

function TraceExplorer({
  traces,
  loadedCount,
  compact,
  outcomeFilter,
  activeLogicalPath,
  activeTraceId,
  hasMore,
  pageIndex,
  canGoPrevious,
  canGoNext,
  loading,
  error,
  onOutcomeFilterChange,
  onLoadMore,
  onRetry,
  onPreviousPage,
  onNextPage,
  onTraceSelect,
  onClearScenarioFilter,
  emptyState,
  costByExactModel,
}: {
  traces: RouteTrace[]
  loadedCount: number
  compact: boolean
  outcomeFilter: TraceOutcomeFilter
  activeLogicalPath: string | null
  activeTraceId: string | null
  hasMore: boolean
  pageIndex: number
  canGoPrevious: boolean
  canGoNext: boolean
  loading: boolean
  error: 'initial' | 'more' | null
  onOutcomeFilterChange: (value: TraceOutcomeFilter) => void
  onLoadMore: () => void
  onRetry: () => void
  onPreviousPage: () => void
  onNextPage: () => void
  onTraceSelect: (trace: RouteTrace) => void
  onClearScenarioFilter: () => void
  emptyState: TraceEmptyStateKind
  costByExactModel: Map<string, number>
}) {
  const { t } = useI18n()
  const segmentOptions: Array<{ key: TraceOutcomeFilter; label: string }> = [
    { key: 'all', label: t('aiCenter.routing.traceSegmentAll', 'All') },
    { key: 'fallback', label: t('aiCenter.routing.traceSegmentFallback', 'Fallback') },
    { key: 'failed', label: t('aiCenter.routing.traceSegmentFailed', 'Failed') },
    { key: 'warning', label: t('aiCenter.routing.traceSegmentWarnings', 'Warnings') },
  ]
  return (
    <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="flex flex-wrap items-center justify-between gap-3 mb-3">
        <div className="flex items-center gap-2">
          <Route size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3 className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.routing.routeTraceAudit', 'Route Trace Audit')}</h3>
        </div>
        <div className={compact ? 'hidden' : 'text-xs'} style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.routing.tracePageLoaded', 'Page {{page}} / loaded {{count}} traces', { page: pageIndex + 1, count: loadedCount })}
        </div>
      </div>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
        <div className="flex min-h-9 flex-wrap items-center gap-1 rounded-lg p-1" style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}>
          {segmentOptions.map((option) => (
            <button
              key={option.key}
              type="button"
              onClick={() => onOutcomeFilterChange(option.key)}
              className="rounded-md px-3 py-1.5 text-xs font-medium"
              style={{
                background: outcomeFilter === option.key ? 'var(--cp-surface-2)' : 'transparent',
                color: outcomeFilter === option.key ? 'var(--cp-text)' : 'var(--cp-muted)',
                border: outcomeFilter === option.key ? '1px solid var(--cp-border)' : '1px solid transparent',
              }}
            >
              {option.label}
            </button>
          ))}
        </div>
        {activeLogicalPath && (
          <button
            type="button"
            onClick={onClearScenarioFilter}
            className="min-h-9 rounded-md px-3 text-xs font-medium"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
            title={activeLogicalPath}
          >
            {t('aiCenter.routing.traceLinkedScenario', 'Scenario')}: {activeLogicalPath}
          </button>
        )}
      </div>
      <div className={compact ? 'flex flex-col gap-3' : 'grid grid-cols-1 gap-3'}>
        {loading && traces.length === 0 && <TraceSkeletonRows />}
        {traces.map((trace) => (
          <TraceCard
            key={trace.request_id}
            trace={trace}
            active={trace.request_id === activeTraceId}
            onSelect={() => onTraceSelect(trace)}
            estimatedCost={estimatedTraceCost(trace, costByExactModel)}
          />
        ))}
        {!loading && traces.length === 0 && (
          <div className="rounded-lg px-3 py-8 text-center text-xs" style={{ color: 'var(--cp-muted)', background: 'var(--cp-bg)' }}>
            {traceEmptyStateLabel(emptyState, t)}
          </div>
        )}
      </div>
      <PagedListFooter
        mode={compact ? 'infinite' : 'pagination'}
        loading={loading}
        error={error ? (error === 'more'
          ? t('aiCenter.routing.traceLoadMoreFailed', 'Failed to load more route traces')
          : t('aiCenter.routing.traceLoadFailed', 'Failed to load route traces')) : null}
        hasMore={hasMore}
        onLoadMore={onLoadMore}
        onRetry={onRetry}
        onPreviousPage={onPreviousPage}
        onNextPage={onNextPage}
        canGoPrevious={canGoPrevious}
        canGoNext={canGoNext}
        pageIndex={pageIndex}
        loadedCount={traces.length}
        totalCount={loadedCount}
        labels={{
          previous: t('aiCenter.routing.tracePreviousPage', 'Previous'),
          next: t('aiCenter.routing.traceNextPage', 'Next'),
          page: t('aiCenter.routing.tracePage', 'Page {{page}}'),
          loading: t('aiCenter.routing.traceLoading', 'Loading...'),
          loadMore: t('aiCenter.routing.traceLoadMore', 'Load more'),
          retry: t('common.retry', 'Retry'),
          error: t('aiCenter.routing.traceLoadFailed', 'Failed to load route traces'),
          loaded: t('aiCenter.routing.tracePageLoaded', 'Page {{page}} / loaded {{count}} traces', { page: pageIndex + 1, count: loadedCount }),
        }}
      />
    </section>
  )
}

function TraceCard({
  trace,
  active,
  onSelect,
  estimatedCost,
}: {
  trace: RouteTrace
  active: boolean
  onSelect: () => void
  estimatedCost?: number
}) {
  const { t } = useI18n()
  const [candidateSection, setCandidateSection] = useState<TraceCandidateSection>('none')
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const selectedCandidate = selectedTraceCandidate(trace)
  const status = traceStatus(trace)
  const providerTraceId = trace.provider_trace_id
  const metaItems = [
    trace.selected_provider_instance_name ? `${t('aiCenter.routing.provider', 'Provider')}: ${trace.selected_provider_instance_name}` : '',
    trace.selected_provider_model_id ? `${t('aiCenter.routing.providerModel', 'Provider model')}: ${trace.selected_provider_model_id}` : '',
    `${t('aiCenter.routing.profile', 'Profile')}: ${trace.scheduler_profile}`,
    trace.created_at_ms ? formatTraceTime(trace.created_at_ms) : '',
    formatTraceDuration(trace),
  ].filter(Boolean)
  const traceFields = [
    { key: 'request_id', label: t('aiCenter.routing.requestId', 'Request ID'), value: trace.request_id },
    { key: 'requested_model', label: t('aiCenter.routing.requestedModel', 'Requested model'), value: trace.requested_model },
    { key: 'selected_exact_model', label: t('aiCenter.routing.selectedExactModel', 'Selected exact model'), value: trace.selected_exact_model },
    { key: 'estimated_cost', label: t('aiCenter.routing.estimatedCost', 'Estimated cost'), value: estimatedCost == null ? '-' : formatUsd(estimatedCost) },
  ]
  const copyFields = [
    ...traceFields.filter((item): item is { key: string; label: string; value: string } => Boolean(item.value && item.value !== '-')),
    { key: 'provider_trace_id', label: 'provider trace id', value: providerTraceId },
  ].filter((item): item is { key: string; label: string; value: string } => Boolean(item.value))

  const copyField = async (key: string, value: string) => {
    try {
      await navigator.clipboard.writeText(value)
      setCopiedKey(key)
      window.setTimeout(() => setCopiedKey(null), 1200)
    } catch {
      setCopiedKey(null)
    }
  }

  return (
    <article
      role="button"
      tabIndex={0}
      onClick={onSelect}
      onKeyDown={(event) => {
        if (event.key === 'Enter' || event.key === ' ') {
          event.preventDefault()
          onSelect()
        }
      }}
      className="rounded-lg p-3 text-left"
      style={{
        background: active ? 'var(--cp-surface-2)' : 'var(--cp-bg)',
        border: `1px solid ${active ? 'var(--cp-accent)' : 'transparent'}`,
      }}
    >
      <div className="flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <LongField value={metaItems.join(' / ')} className="mt-1 text-xs" tone="muted" copyable={false} expandable />
        </div>
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          {trace.warnings.length > 0 && (
            <StatusBadge status="warning" label={t('aiCenter.routing.traceWarnings', '{{count}} warnings', { count: trace.warnings.length })} />
          )}
          <StatusBadge status={status === 'selected' ? 'ok' : status === 'fallback' ? 'warning' : 'error'} label={status} />
        </div>
      </div>
      <p className="text-sm mt-2" style={{ color: 'var(--cp-text)' }}>{trace.user_summary?.reason_short}</p>
      {selectedCandidate && (
        <div className="mt-2 rounded-md px-2 py-1.5 text-xs" style={{ color: 'var(--cp-muted)', background: 'var(--cp-surface)' }}>
          {candidateWeightSummary(selectedCandidate)}
        </div>
      )}

      <div className="mt-3 overflow-hidden rounded-md" style={{ border: '1px solid var(--cp-border)', background: 'var(--cp-surface)' }}>
        <table className="w-full table-fixed text-xs">
          <tbody>
            {traceFields.map((field) => (
              <tr key={field.key} style={{ borderTop: '1px solid var(--cp-border)' }}>
                <th className="w-36 px-2 py-2 text-left font-medium" style={{ color: 'var(--cp-muted)' }}>{field.label}</th>
                <td className="min-w-0 px-2 py-2" style={{ color: 'var(--cp-text)' }}>
                  <LongField
                    value={field.value}
                    fallback={field.key === 'selected_exact_model' ? t('aiCenter.routing.noExactResolved', 'No exact model resolved') : '-'}
                    mono={field.key !== 'estimated_cost'}
                    tone={field.key === 'selected_exact_model' && !field.value ? 'danger' : 'default'}
                    expandable
                  />
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>

      <div className="mt-2 flex flex-wrap gap-1.5">
        {copyFields.map((field) => (
          <button
            key={field.key}
            type="button"
            onClick={(event) => {
              event.stopPropagation()
              void copyField(field.key, field.value)
            }}
            className="inline-flex min-h-8 max-w-full items-center gap-1 rounded-md px-2 text-xs"
            style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)', background: 'var(--cp-surface)' }}
            title={field.value}
          >
            {copiedKey === field.key ? <Check size={13} /> : <Copy size={13} />}
            <span className="truncate">{field.label}</span>
          </button>
        ))}
      </div>

      <div className="mt-3">
        <select
          value={candidateSection}
          onClick={(event) => event.stopPropagation()}
          onChange={(event) => setCandidateSection(event.target.value as TraceCandidateSection)}
          className="h-9 rounded-md px-2 text-xs outline-none"
          style={{ background: 'var(--cp-surface)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          <option value="none">{t('common.collapse', 'Collapse')}</option>
          <option value="ranked">{t('aiCenter.routing.traceShowRankedCandidates', 'Show ranked candidates ({{count}})', { count: trace.ranked_candidates.length })}</option>
          <option value="filtered">{t('aiCenter.routing.traceShowFilteredCandidates', 'Show filtered out ({{count}})', { count: trace.filtered_candidates.length })}</option>
        </select>
      </div>

      {candidateSection === 'ranked' && (
        <div className="mt-3 rounded-md p-2" style={{ background: 'var(--cp-surface)' }}>
          {trace.ranked_candidates.length > 0 ? (
            <div className="flex flex-col gap-1">
              {trace.ranked_candidates.map((candidate, index) => (
                <TraceCandidateRow
                  key={candidate.exact_model}
                  candidate={candidate}
                  rank={rankedCandidateRank(trace, candidate, index)}
                  selected={candidate.selected}
                  reason={candidate.selected ? 'selected' : trace.fallback_applied && candidate.exact_model === trace.selected_exact_model ? 'fallback' : 'ranked'}
                />
              ))}
            </div>
          ) : (
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.routing.noRankedCandidates', 'No ranked candidates.')}</div>
          )}
          <CollapseCandidatesButton onClick={() => setCandidateSection('none')} />
        </div>
      )}

      {candidateSection === 'filtered' && (
        <div className="mt-3 rounded-md p-2" style={{ background: 'var(--cp-surface)' }}>
          {trace.filtered_candidates.length > 0 ? (
            <div className="flex flex-col gap-1">
              {trace.filtered_candidates.map((candidate) => (
                <TraceFilteredCandidateRow key={candidate.exact_model} candidate={candidate} />
              ))}
            </div>
          ) : (
            <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>{t('aiCenter.routing.noFilteredCandidates', 'No candidates were filtered out.')}</div>
          )}
          <CollapseCandidatesButton onClick={() => setCandidateSection('none')} />
        </div>
      )}
      {trace.warnings.length > 0 && (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {trace.warnings.map((warning) => (
            <span key={warning} className="rounded-md px-2 py-1 text-xs" style={{ color: 'var(--cp-warning)', background: 'var(--cp-surface)' }}>
              {warning}
            </span>
          ))}
        </div>
      )}
    </article>
  )
}

function TraceCandidateRow({
  candidate,
  rank,
  selected,
  reason,
}: {
  candidate: RouteTrace['ranked_candidates'][number]
  rank: number
  selected: boolean
  reason: string
}) {
  return (
    <div
      className="flex justify-between gap-3 rounded-md px-2 py-1.5 text-xs"
      style={{
        background: selected ? 'var(--cp-bg)' : 'transparent',
        border: `1px solid ${selected ? 'var(--cp-accent)' : 'transparent'}`,
      }}
    >
      <span className="min-w-0" style={{ color: selected ? 'var(--cp-accent)' : 'var(--cp-text)' }}>
        <span className="block truncate">
          <LongField value={`#${rank} ${candidate.exact_model}`} copyable={false} />
        </span>
        <span className="block" style={{ color: 'var(--cp-muted)' }}>
          {candidateWeightSummary(candidate)}
        </span>
      </span>
      <span className="shrink-0 text-right" style={{ color: 'var(--cp-muted)' }}>
        <span className="block">{candidate.final_score != null ? candidate.final_score.toFixed(2) : '-'}</span>
        <span className="block">{reason}</span>
      </span>
    </div>
  )
}

function TraceFilteredCandidateRow({ candidate }: { candidate: RouteTrace['filtered_candidates'][number] }) {
  return (
    <div className="flex justify-between gap-3 rounded-md px-2 py-1.5 text-xs">
      <span className="min-w-0">
        <LongField value={candidate.exact_model} tone="warning" />
        <span className="block" style={{ color: 'var(--cp-muted)' }}>{candidate.reason}</span>
      </span>
      <span className="shrink-0" style={{ color: 'var(--cp-muted)' }}>filtered</span>
    </div>
  )
}

function CollapseCandidatesButton({ onClick }: { onClick: () => void }) {
  const { t } = useI18n()
  return (
    <button
      type="button"
      onClick={(event) => {
        event.stopPropagation()
        onClick()
      }}
      className="mt-2 inline-flex min-h-8 w-full items-center justify-center gap-1 rounded-md px-2 text-xs font-medium"
      style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
    >
      <ChevronUp size={13} />
      {t('common.collapse', 'Collapse')}
    </button>
  )
}

function EmptyResults() {
  const { t } = useI18n()
  return (
    <div className="rounded-xl p-8 text-center" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <Search size={32} className="mx-auto" style={{ color: 'var(--cp-muted)' }} />
      <p className="text-sm mt-3" style={{ color: 'var(--cp-muted)' }}>
        {t('aiCenter.routing.noMatches', 'No routing scenarios match the current filters.')}
      </p>
    </div>
  )
}

function UseCaseIcon({ kind, small = false }: { kind: UseCaseKind; small?: boolean }) {
  const size = small ? 15 : 19
  const icon = kind === 'chat' ? <MessageSquare size={size} />
    : kind === 'code' ? <Code2 size={size} />
      : kind === 'plan' ? <FileText size={size} />
        : kind === 'image' ? <Image size={size} />
          : kind === 'embed' ? <Braces size={size} />
            : kind === 'vision' ? <Activity size={size} />
              : kind === 'audio' ? <Zap size={size} />
                : <GitBranch size={size} />

  return (
    <div
      className={`${small ? 'h-7 w-7 rounded-md' : 'h-10 w-10 rounded-lg'} shrink-0 flex items-center justify-center`}
      style={{ background: 'var(--cp-bg)', color: 'var(--cp-accent)' }}
    >
      {icon}
    </div>
  )
}

function MetricChip({ icon, label, value }: { icon: ReactNode; label: string; value: string }) {
  return (
    <span
      className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 text-xs"
      style={{ background: 'var(--cp-surface)', color: 'var(--cp-muted)', border: '1px solid var(--cp-border)' }}
    >
      {icon}
      <span>{label}</span>
      <span style={{ color: 'var(--cp-text)' }}>{value}</span>
    </span>
  )
}

function Fact({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-3 rounded-lg px-3 py-2" style={{ background: 'var(--cp-bg)' }}>
      <span className="text-xs shrink-0" style={{ color: 'var(--cp-muted)' }}>{label}</span>
      <LongField value={value} className="justify-end text-right text-xs" expandable />
    </div>
  )
}

function buildScenarios(nodes: LogicalNode[], models: ModelMetadata[], traces: RouteTrace[]): ScenarioView[] {
  const modelByExact = new Map(models.map((model) => [model.exact_model, model]))
  const scenarios = nodes
    .filter((node) => node.level !== 'L1')
    .filter((node) => isScenarioNode(node))
    .map((node) => {
      const trace = traces.find((item) => item.resolved_logical_path === node.path || item.requested_model === node.path)
      const selectedExactModel = trace?.selected_exact_model ?? node.resolved_exact_model
      const selectedModel = selectedExactModel ? modelByExact.get(selectedExactModel) : undefined
      const candidates = scenarioCandidates(node, models, trace)
      const groups = groupModels(candidates)
      const useCase = useCaseFromPath(node.path, node.api_type)
      return {
        node,
        useCase,
        title: scenarioTitle(node, useCase),
        description: scenarioDescription(node, useCase),
        selectedModel,
        selectedExactModel,
        trace,
        candidates,
        groups,
        score: scenarioScore(node, selectedModel, trace),
      }
    })

  return scenarios
}

function TraceSkeletonRows() {
  return (
    <>
      {[0, 1].map((index) => (
        <div
          key={index}
          className="min-h-[220px] animate-pulse rounded-lg p-3"
          style={{ background: 'var(--cp-bg)', border: '1px solid transparent' }}
        >
          <div className="mb-3 h-4 w-3/4 rounded" style={{ background: 'var(--cp-border)' }} />
          <div className="mb-4 h-3 w-1/2 rounded" style={{ background: 'var(--cp-border)' }} />
          <div className="mb-3 h-16 rounded-md" style={{ background: 'var(--cp-surface)' }} />
          <div className="h-16 rounded-md" style={{ background: 'var(--cp-surface)' }} />
        </div>
      ))}
    </>
  )
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

function scenarioCandidates(node: LogicalNode, models: ModelMetadata[], trace?: RouteTrace): ModelMetadata[] {
  const childPaths = new Set(flattenNodes(node.children ?? []).map((child) => child.path))
  const itemTargets = new Set(Object.values(node.items ?? {}).map((item) => item.target))
  const traceCandidates = new Set(trace?.ranked_candidates.map((candidate) => candidate.exact_model) ?? [])
  const required = new Set(node.policy?.required_features ?? [])

  return models
    .filter((model) => modelMatchesLogicalNode(model, node, childPaths, itemTargets, traceCandidates))
    .filter((model) => modelSupportsRequired(model, required))
    .sort((left, right) => compareModelForScenario(left, right, node, trace))
}

function modelMatchesLogicalNode(
  model: ModelMetadata,
  node: LogicalNode,
  childPaths: Set<string>,
  itemTargets: Set<string>,
  traceCandidates: Set<string>,
): boolean {
  if (traceCandidates.has(model.exact_model)) return true
  if (node.resolved_exact_model === model.exact_model) return true
  if (node.api_type && model.api_types.includes(node.api_type)) {
    if (model.logical_mounts.includes(node.path)) return true
    if (model.logical_mounts.some((mount) => childPaths.has(mount) || itemTargets.has(mount))) return true
  }
  return model.logical_mounts.some((mount) => mount === node.path || childPaths.has(mount) || itemTargets.has(mount))
}

function modelSupportsRequired(model: ModelMetadata, required: Set<string>): boolean {
  if (required.size === 0) return true
  if (required.has('streaming') && !model.capabilities.streaming) return false
  if ((required.has('tool_call') || required.has('tool_calling')) && !model.capabilities.tool_call) return false
  if ((required.has('json_schema') || required.has('json_output')) && !model.capabilities.json_schema) return false
  if (required.has('web_search') && !model.capabilities.web_search) return false
  if (required.has('vision') && !model.capabilities.vision) return false
  return true
}

function groupModels(models: ModelMetadata[]): ModelGroup[] {
  const groups = new Map<string, ModelMetadata[]>()
  const groupOrder = new Map<string, number>()
  models.forEach((model, index) => {
    const key = baseModelKey(model)
    if (!groupOrder.has(key)) groupOrder.set(key, index)
    groups.set(key, [...(groups.get(key) ?? []), model])
  })
  return Array.from(groups.entries())
    .map(([key, items]) => {
      const sorted = [...items].sort(compareModelPriority)
      const [primary, ...variants] = sorted
      return primary ? { key, primary, variants } : null
    })
    .filter((group): group is ModelGroup => group !== null)
    .sort((left, right) => (groupOrder.get(left.key) ?? 0) - (groupOrder.get(right.key) ?? 0))
}

function modelGroupHasExact(group: ModelGroup, exactModel?: string): boolean {
  if (!exactModel) return false
  return group.primary.exact_model === exactModel || group.variants.some((model) => model.exact_model === exactModel)
}

function buildFilterOptions(models: ModelMetadata[]): Record<FilterKey, string[]> {
  return {
    provider: uniqueSorted(models.map((model) => providerFromExact(model.exact_model))),
    apiType: uniqueSorted(models.flatMap((model) => model.api_types)),
    capability: uniqueSorted(models.flatMap(modelCapabilities)),
    cost: uniqueSorted(models.map((model) => model.attributes.cost_class)),
    latency: uniqueSorted(models.map((model) => model.attributes.latency_class)),
    health: uniqueSorted(models.map((model) => model.health.status)),
    location: ['local', 'cloud'],
  }
}

function scenarioMatchesQuery(scenario: ScenarioView, query: string): boolean {
  if (!query) return true
  const haystack = [
    scenario.node.path,
    scenario.title,
    scenario.description,
    scenario.node.api_type ?? '',
    scenario.node.policy?.profile ?? '',
    scenario.selectedExactModel ?? '',
    ...scenario.candidates.flatMap((model) => [
      model.provider_model_id,
      model.provider_actual_model_id ?? '',
      model.exact_model,
      providerFromExact(model.exact_model),
      ...model.logical_mounts,
      ...model.api_types,
      ...modelCapabilities(model),
      model.attributes.cost_class,
      model.attributes.latency_class,
      model.health.status,
    ]),
  ].join(' ').toLowerCase()
  return haystack.includes(query)
}

function scenarioMatchesFilters(scenario: ScenarioView, filters: RoutingFilters): boolean {
  if (Object.values(filters).every((value) => value.query.trim().length === 0 && value.selected.length === 0)) return true
  return scenario.candidates.some((model) =>
    multiFilterMatches(filters.provider, [providerFromExact(model.exact_model)])
    && multiFilterMatches(filters.apiType, model.api_types)
    && multiFilterMatches(filters.capability, modelCapabilities(model))
    && multiFilterMatches(filters.cost, [model.attributes.cost_class])
    && multiFilterMatches(filters.latency, [model.attributes.latency_class])
    && multiFilterMatches(filters.health, [model.health.status])
    && multiFilterMatches(filters.location, [model.attributes.local ? 'local' : 'cloud']),
  )
}

function multiFilterMatches(filter: MultiFilter, values: string[]): boolean {
  if (filter.selected.length > 0 && !values.some((value) => filter.selected.includes(value))) return false
  const query = filter.query.trim().toLowerCase()
  if (query && !values.some((value) => value.toLowerCase().includes(query))) return false
  return true
}

function compareScenario(left: ScenarioView, right: ScenarioView): number {
  return USE_CASE_ORDER.indexOf(left.useCase) - USE_CASE_ORDER.indexOf(right.useCase)
    || right.score - left.score
    || left.node.path.localeCompare(right.node.path)
}

function compareModelForScenario(left: ModelMetadata, right: ModelMetadata, node: LogicalNode, trace?: RouteTrace): number {
  const traceDiff = traceCandidateScore(right, trace) - traceCandidateScore(left, trace)
  if (traceDiff !== 0) return traceDiff
  const profileDiff = profileModelScore(right, node) - profileModelScore(left, node)
  if (profileDiff !== 0) return profileDiff
  return compareModelPriority(left, right)
}

function compareModelPriority(left: ModelMetadata, right: ModelMetadata): number {
  return healthScore(right) - healthScore(left)
    || variantScore(right) - variantScore(left)
    || (right.attributes.quality_score ?? 0) - (left.attributes.quality_score ?? 0)
    || latencyScore(right.attributes.latency_class) - latencyScore(left.attributes.latency_class)
    || costScore(right.attributes.cost_class) - costScore(left.attributes.cost_class)
    || versionScore(right.provider_model_id) - versionScore(left.provider_model_id)
    || left.provider_model_id.localeCompare(right.provider_model_id)
}

function profileModelScore(model: ModelMetadata, node: LogicalNode): number {
  const profile = node.policy?.profile ?? 'balanced'
  let score = 0
  if (node.resolved_exact_model === model.exact_model) score += 100
  if (node.policy?.local_only && model.attributes.local) score += 40
  if ((profile === 'local_first' || profile === 'strict_local') && model.attributes.local) score += 25
  if (profile === 'quality_first') score += (model.attributes.quality_score ?? 0) / 2
  if (profile === 'latency_first') score += latencyScore(model.attributes.latency_class) * 8
  if (profile === 'cost_first') score += costScore(model.attributes.cost_class) * 8
  return score
}

function traceCandidateScore(model: ModelMetadata, trace?: RouteTrace): number {
  const candidate = trace?.ranked_candidates.find((item) => item.exact_model === model.exact_model)
  if (!candidate) return 0
  return (candidate.selected ? 100 : 0) + ((candidate.final_score ?? 0) * 10)
}

function scenarioScore(node: LogicalNode, model?: ModelMetadata, trace?: RouteTrace): number {
  if (!model) return 0
  return 100
    + profileModelScore(model, node)
    + traceCandidateScore(model, trace)
    + healthScore(model)
    + (model.attributes.quality_score ?? 0)
}

function isScenarioNode(node: LogicalNode): boolean {
  if (node.path === 'llm') return true
  if (node.level === 'L3') return true
  return Boolean(node.items && Object.keys(node.items).length > 0)
}

function flattenNodes(nodes: LogicalNode[]): LogicalNode[] {
  return nodes.flatMap((node) => [node, ...flattenNodes(node.children ?? [])])
}

function childNodesAtPath(nodes: LogicalNode[], path: string | null): LogicalNode[] {
  if (!path) return nodes
  return findNodeByPath(nodes, path)?.children ?? []
}

function canNavigateIntoPath(nodes: LogicalNode[], path: string): boolean {
  return childNodesAtPath(nodes, path).some(isLogicalDirectoryNode)
}

function isLogicalDirectoryNode(node: LogicalNode): boolean {
  return !node.locked && !node.path.includes('@')
}

function findNodeByPath(nodes: LogicalNode[], path: string): LogicalNode | undefined {
  for (const node of nodes) {
    if (node.path === path) return node
    const child = findNodeByPath(node.children ?? [], path)
    if (child) return child
  }
  return undefined
}

function useCaseFromPath(path: string, apiType?: string): UseCaseKind {
  const value = `${path} ${apiType ?? ''}`.toLowerCase()
  if (value.includes('code') || value.includes('coder')) return 'code'
  if (value.includes('plan') || value.includes('reason')) return 'plan'
  if (value.includes('image')) return 'image'
  if (value.includes('embedding')) return 'embed'
  if (value.includes('vision') || value.includes('ocr')) return 'vision'
  if (value.includes('audio') || value.includes('tts') || value.includes('asr')) return 'audio'
  if (value.includes('llm') || value.includes('chat')) return 'chat'
  return 'other'
}

function scenarioTitle(node: LogicalNode, useCase: UseCaseKind): string {
  if (node.path === 'llm') return 'Chat'
  if (useCase === 'code') return 'Code'
  if (useCase === 'plan') return node.path.includes('reason') ? 'Reasoning / Plan' : 'Plan'
  if (useCase === 'image') return 'Image'
  if (useCase === 'embed') return 'Embedding'
  if (useCase === 'vision') return 'Vision'
  if (useCase === 'audio') return 'Audio'
  return node.label || node.path
}

function scenarioDescription(node: LogicalNode, useCase: UseCaseKind): string {
  const profile = node.policy?.profile ?? 'balanced'
  if (useCase === 'chat') return `General conversation and default LLM traffic, sorted with ${profile}.`
  if (useCase === 'code') return `Coding requests that need tool use and low-friction iteration, sorted with ${profile}.`
  if (useCase === 'plan') return `Planning or reasoning requests that favor stronger model quality, sorted with ${profile}.`
  if (useCase === 'image') return `Image generation and editing routes, sorted with ${profile}.`
  if (useCase === 'embed') return `Embedding routes for retrieval and semantic indexing, sorted with ${profile}.`
  if (useCase === 'vision') return `Vision and OCR routes, sorted with ${profile}.`
  if (useCase === 'audio') return `Speech and audio routes, sorted with ${profile}.`
  return `${node.label || node.path}, sorted with ${profile}.`
}

function modelCapabilities(model: ModelMetadata): string[] {
  const result: string[] = []
  if (model.capabilities.streaming) result.push('streaming')
  if (model.capabilities.tool_call) result.push('tool_call')
  if (model.capabilities.json_schema) result.push('json_schema')
  if (model.capabilities.web_search) result.push('web_search')
  if (model.capabilities.vision) result.push('vision')
  return result
}

function baseModelKey(model: ModelMetadata): string {
  return (model.provider_actual_model_id ?? model.provider_model_id)
    .replace(/@(.*)$/, '')
    .replace(/[-_.](reasoning|thinking|think|mini|nano|small|flash|lite|preview|latest|turbo|fast|low|high)$/i, '')
}

function variantScore(model: ModelMetadata): number {
  if (model.provider_actual_model_id && model.provider_actual_model_id !== model.provider_model_id) return 0
  return baseModelKey(model).toLowerCase() === model.provider_model_id.replace(/@(.*)$/, '').toLowerCase() ? 2 : 1
}

function healthScore(model: ModelMetadata): number {
  if (model.health.quota_state === 'exhausted') return -10
  if (model.health.status === 'available') return 3
  if (model.health.status === 'degraded') return 1
  return -5
}

function latencyScore(value: ModelMetadata['attributes']['latency_class']): number {
  if (value === 'fast') return 3
  if (value === 'normal') return 2
  if (value === 'slow') return 1
  return 0
}

function costScore(value: ModelMetadata['attributes']['cost_class']): number {
  if (value === 'low') return 3
  if (value === 'medium') return 2
  if (value === 'high') return 1
  return 0
}

function versionScore(value: string): number {
  const normalized = value
    .replace(/\bmini\b|\bnano\b|\bpreview\b|\blite\b|\bflash\b/gi, '')
  const matches = normalized.match(/\d+(?:\.\d+)?/g) ?? []
  return matches.reduce((score, item, index) => score + Number(item) / (index + 1), 0)
}

function providerFromExact(exactModel: string): string {
  return exactModel.split('@')[1] ?? 'unknown'
}

function breadcrumbPaths(path: string | null): string[] {
  if (!path) return []
  const parts = path.split('.')
  return parts.map((_, index) => parts.slice(0, index + 1).join('.'))
}

function lastPathSegment(path: string): string {
  const parts = path.split('.')
  return parts[parts.length - 1] || path
}

function formatQuality(value?: number): string {
  if (value == null) return '-'
  return value > 1 ? value.toFixed(0) : Math.round(value * 100).toString()
}

function formatLatency(model: ModelMetadata): string {
  return `${model.attributes.latency_class}${model.health.p95_latency_ms ? ` p95 ${model.health.p95_latency_ms}ms` : ''}`
}

function uniqueSorted(values: string[]): string[] {
  return Array.from(new Set(values.filter(Boolean))).sort((left, right) => left.localeCompare(right))
}

function selectedTraceCandidate(trace: RouteTrace): RouteTrace['ranked_candidates'][number] | undefined {
  return trace.ranked_candidates.find((candidate) => candidate.selected)
    ?? trace.ranked_candidates.find((candidate) => candidate.exact_model === trace.selected_exact_model)
}

function traceMatchesOutcome(trace: RouteTrace, filter: TraceOutcomeFilter): boolean {
  if (filter === 'all') return true
  if (filter === 'fallback') return trace.fallback_applied
  if (filter === 'failed') return !trace.selected_exact_model
  if (filter === 'warning') return trace.warnings.length > 0
  return true
}

function traceMatchesScenarioPath(trace: RouteTrace, path: string): boolean {
  return traceLogicalPath(trace) === path
}

function traceLogicalPath(trace?: RouteTrace): string | null {
  if (!trace) return null
  return trace.resolved_logical_path ?? (trace.requested_model_type === 'logical' ? trace.requested_model : null)
}

function traceStatus(trace: RouteTrace): 'selected' | 'fallback' | 'failed' {
  if (!trace.selected_exact_model) return 'failed'
  return trace.fallback_applied ? 'fallback' : 'selected'
}

function estimatedTraceCost(trace: RouteTrace, costByExactModel: Map<string, number>): number | undefined {
  if (!trace.selected_exact_model) return undefined
  return costByExactModel.get(trace.selected_exact_model)
}

function formatUsd(amount: number): string {
  if (amount === 0) return '$0.0'
  const abs = Math.abs(amount)
  if (abs < 0.0001) return amount < 0 ? '>-$0.0001' : '<$0.0001'
  if (abs < 0.01) return `$${amount.toFixed(4)}`
  return `$${amount.toFixed(2)}`
}

function rankedCandidateRank(
  trace: RouteTrace,
  candidate: RouteTrace['ranked_candidates'][number],
  fallbackIndex: number,
): number {
  const index = trace.ranked_candidates.findIndex((item) => item.exact_model === candidate.exact_model)
  return (index >= 0 ? index : fallbackIndex) + 1
}

function traceEmptyStateKind(
  loadedCount: number,
  visibleCount: number,
  error: 'initial' | 'more' | null,
  hasScenarioFilter: boolean,
  outcomeFilter: TraceOutcomeFilter,
): TraceEmptyStateKind {
  if (loadedCount === 0 && error === 'initial') return 'load-failed'
  if (loadedCount === 0) return 'none-yet'
  if (visibleCount === 0 && (hasScenarioFilter || outcomeFilter !== 'all')) return 'no-matches'
  return 'no-matches'
}

function traceEmptyStateLabel(kind: TraceEmptyStateKind, t: (key: string, fallback: string) => string): string {
  if (kind === 'load-failed') return t('aiCenter.routing.traceLoadFailed', 'Failed to load traces')
  if (kind === 'none-yet') return t('aiCenter.routing.traceNoneYet', 'No route traces yet')
  return t('aiCenter.routing.traceNoMatches', 'No traces match current scenario/filter')
}

function formatTraceTime(createdAtMs: number): string {
  const date = new Date(createdAtMs)
  if (Number.isNaN(date.getTime())) return ''
  return date.toLocaleString()
}

function formatTraceDuration(trace: RouteTrace): string {
  const value = trace.duration_ms ?? trace.latency_ms
  if (value == null) return ''
  return `${value}ms`
}

function candidateWeightSummary(candidate: RouteTrace['ranked_candidates'][number]): string {
  const inputs = candidate.preference_score_inputs
  const exact = inputs?.exact_model_weight ?? candidate.exact_model_weight ?? 1
  const provider = inputs?.provider_weight ?? candidate.provider_weight ?? 1
  const combined = inputs?.combined_weight ?? exact * provider
  const providerEffect = inputs?.provider_weight_effect ?? weightEffect(provider)
  return `exact ${formatWeight(exact)} / provider ${formatWeight(provider)} ${providerEffect} / combined ${formatWeight(combined)}`
}

function weightEffect(weight: number): string {
  if (weight <= 0) return 'disabled'
  if (weight < 1) return 'downweighted'
  if (weight > 1) return 'upweighted'
  return 'neutral'
}

function formatWeight(weight: number): string {
  return weight.toFixed(2).replace(/\.?0+$/, '')
}
