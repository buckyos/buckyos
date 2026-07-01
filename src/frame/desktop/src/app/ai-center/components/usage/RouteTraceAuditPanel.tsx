import { useEffect, useMemo, useState } from 'react'
import { Check, ChevronDown, ChevronUp, Copy, Route, Search } from 'lucide-react'
import { useI18n } from '../../../../i18n/provider'
import { useAICCStore, useRouteTraces } from '../../hooks/use-aicc-store'
import { StatusBadge } from '../shared/StatusBadge'
import { PagedListFooter } from '../shared/paged-list'
import { LongField } from '../shared/LongField'
import type { RouteTrace } from '../../../../api/aicc_mgr'

type TraceOutcomeFilter = 'all' | 'fallback' | 'failed' | 'warning'
type TraceEmptyStateKind = 'none-yet' | 'load-failed' | 'no-matches'

const ROUTE_TRACE_PAGE_SIZE = 20

export function RouteTraceAuditPanel({ compact }: { compact: boolean }) {
  const { t } = useI18n()
  const store = useAICCStore()
  const snapshotTraces = useRouteTraces()
  const [query, setQuery] = useState('')
  const [outcomeFilter, setOutcomeFilter] = useState<TraceOutcomeFilter>('all')
  const [traces, setTraces] = useState<RouteTrace[]>(snapshotTraces)
  const [traceNextCursor, setTraceNextCursor] = useState<string | undefined>()
  const [tracePageIndex, setTracePageIndex] = useState(0)
  const [traceLoading, setTraceLoading] = useState(false)
  const [traceError, setTraceError] = useState<'initial' | 'more' | null>(null)
  const [selectedTraceId, setSelectedTraceId] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false
    async function loadInitialTraces() {
      setTraceLoading(true)
      try {
        const page = await store.queryRouteTraces({ limit: ROUTE_TRACE_PAGE_SIZE })
        if (!cancelled) {
          setTraces(page.traces)
          setTraceNextCursor(page.nextCursor)
          setTracePageIndex(0)
          setTraceError(null)
        }
      } catch (error) {
        console.error('aicc.trace.query usage audit failed', error)
        if (!cancelled) {
          setTraces(snapshotTraces)
          setTraceNextCursor(snapshotTraces.length >= ROUTE_TRACE_PAGE_SIZE ? String(ROUTE_TRACE_PAGE_SIZE) : undefined)
          setTracePageIndex(0)
          setTraceError('initial')
        }
      } finally {
        if (!cancelled) setTraceLoading(false)
      }
    }
    void loadInitialTraces()
    return () => {
      cancelled = true
    }
  }, [snapshotTraces, store])

  const visibleTraces = useMemo(() => {
    const normalizedQuery = query.trim().toLowerCase()
    return traces
      .filter((trace) => traceMatchesOutcome(trace, outcomeFilter))
      .filter((trace) => traceMatchesQuery(trace, normalizedQuery))
  }, [outcomeFilter, query, traces])
  const emptyState = traceEmptyStateKind(traces.length, visibleTraces.length, traceError, query.trim().length > 0, outcomeFilter)

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
      console.error('aicc.trace.query usage audit page failed', error)
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
      console.error('aicc.trace.query usage audit more failed', error)
      setTraceError('more')
    } finally {
      setTraceLoading(false)
    }
  }

  const retryTraceLoad = () => {
    if (traceError === 'initial') void loadTracePage(tracePageIndex)
    else void loadMoreTraces()
  }

  const segmentOptions: Array<{ key: TraceOutcomeFilter; label: string }> = [
    { key: 'all', label: t('aiCenter.routing.traceSegmentAll', 'All') },
    { key: 'fallback', label: t('aiCenter.routing.traceSegmentFallback', 'Fallback') },
    { key: 'failed', label: t('aiCenter.routing.traceSegmentFailed', 'Failed') },
    { key: 'warning', label: t('aiCenter.routing.traceSegmentWarnings', 'Warnings') },
  ]

  return (
    <section className="rounded-xl p-4" style={{ background: 'var(--cp-surface)', border: '1px solid var(--cp-border)' }}>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div className="flex items-center gap-2">
          <Route size={16} style={{ color: 'var(--cp-accent)' }} />
          <h3 className="text-sm font-medium" style={{ color: 'var(--cp-text)' }}>{t('aiCenter.routing.routeTraceAudit', 'Route Trace Audit')}</h3>
        </div>
        <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
          {t('aiCenter.routing.tracePageLoaded', 'Page {{page}} / loaded {{count}} traces', { page: tracePageIndex + 1, count: traces.length })}
        </div>
      </div>

      <div className="mb-3 grid grid-cols-1 gap-2 lg:grid-cols-[minmax(260px,1fr)_auto]">
        <label className="relative block">
          <Search size={15} className="absolute left-3 top-1/2 -translate-y-1/2" style={{ color: 'var(--cp-muted)' }} />
          <input
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={t('aiCenter.routing.traceSearch', 'Search request, model, provider, scenario...')}
            className="min-h-10 w-full rounded-lg py-2 pl-9 pr-3 text-sm outline-none"
            style={{ background: 'var(--cp-bg)', color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
          />
        </label>
        <div className="flex min-h-10 flex-wrap items-center gap-1 rounded-lg p-1" style={{ background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}>
          {segmentOptions.map((option) => (
            <button
              key={option.key}
              type="button"
              onClick={() => setOutcomeFilter(option.key)}
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
      </div>

      <div className={compact ? 'flex flex-col gap-3' : 'grid grid-cols-1 gap-3'}>
        {traceLoading && visibleTraces.length === 0 && <TraceAuditSkeletonRows />}
        {visibleTraces.map((trace) => (
          <TraceAuditCard
            key={trace.request_id}
            trace={trace}
            active={trace.request_id === selectedTraceId}
            onSelect={() => setSelectedTraceId(trace.request_id)}
          />
        ))}
        {!traceLoading && visibleTraces.length === 0 && (
          <div className="rounded-lg px-3 py-8 text-center text-xs" style={{ color: 'var(--cp-muted)', background: 'var(--cp-bg)' }}>
            {traceEmptyStateLabel(emptyState, t)}
          </div>
        )}
      </div>

      <PagedListFooter
        mode={compact ? 'infinite' : 'pagination'}
        loading={traceLoading}
        error={traceError ? (traceError === 'more'
          ? t('aiCenter.routing.traceLoadMoreFailed', 'Failed to load more route traces')
          : t('aiCenter.routing.traceLoadFailed', 'Failed to load route traces')) : null}
        hasMore={Boolean(traceNextCursor)}
        onLoadMore={loadMoreTraces}
        onRetry={retryTraceLoad}
        onPreviousPage={() => void loadTracePage(tracePageIndex - 1)}
        onNextPage={() => void loadTracePage(tracePageIndex + 1)}
        canGoPrevious={!compact && tracePageIndex > 0}
        canGoNext={!compact && Boolean(traceNextCursor)}
        pageIndex={tracePageIndex}
        loadedCount={visibleTraces.length}
        totalCount={traces.length}
        labels={{
          previous: t('aiCenter.routing.tracePreviousPage', 'Previous'),
          next: t('aiCenter.routing.traceNextPage', 'Next'),
          page: t('aiCenter.routing.tracePage', 'Page {{page}}'),
          loading: t('aiCenter.routing.traceLoading', 'Loading...'),
          loadMore: t('aiCenter.routing.traceLoadMore', 'Load more'),
          retry: t('common.retry', 'Retry'),
          error: t('aiCenter.routing.traceLoadFailed', 'Failed to load route traces'),
          loaded: t('aiCenter.routing.tracePageLoaded', 'Page {{page}} / loaded {{count}} traces', { page: tracePageIndex + 1, count: traces.length }),
        }}
      />
    </section>
  )
}

function TraceAuditCard({ trace, active, onSelect }: { trace: RouteTrace; active: boolean; onSelect: () => void }) {
  const { t } = useI18n()
  const [rankedExpanded, setRankedExpanded] = useState(false)
  const [filteredExpanded, setFilteredExpanded] = useState(false)
  const [copiedKey, setCopiedKey] = useState<string | null>(null)
  const selectedCandidate = selectedTraceCandidate(trace)
  const visibleRankedCandidates = rankedExpanded
    ? trace.ranked_candidates
    : selectedCandidate ? [selectedCandidate] : trace.ranked_candidates.slice(0, 2)
  const hiddenRankedCount = Math.max(0, trace.ranked_candidates.length - visibleRankedCandidates.length)
  const status = traceStatus(trace)
  const metaItems = [
    trace.selected_provider_instance_name ? `${t('aiCenter.routing.provider', 'Provider')}: ${trace.selected_provider_instance_name}` : '',
    trace.selected_provider_model_id ? `${t('aiCenter.routing.providerModel', 'Provider model')}: ${trace.selected_provider_model_id}` : '',
    `${t('aiCenter.routing.profile', 'Profile')}: ${trace.scheduler_profile}`,
    trace.created_at_ms ? formatTraceTime(trace.created_at_ms) : '',
    formatTraceDuration(trace),
  ].filter(Boolean)
  const copyFields = [
    { key: 'request_id', label: 'request_id', value: trace.request_id },
    { key: 'requested_model', label: 'requested_model', value: trace.requested_model },
    { key: 'selected_exact_model', label: 'selected_exact_model', value: trace.selected_exact_model },
    { key: 'provider_trace_id', label: 'provider trace id', value: trace.provider_trace_id },
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
          <div className="flex flex-wrap items-center gap-2">
            <LongField value={trace.requested_model} className="text-sm" mono />
            <span style={{ color: 'var(--cp-muted)' }}>{'->'}</span>
            <LongField
              value={trace.selected_exact_model}
              fallback={t('aiCenter.routing.noExactResolved', 'No exact model resolved')}
              className="text-sm"
              mono
              tone={trace.selected_exact_model ? 'default' : 'danger'}
              expandable
            />
          </div>
          <LongField value={metaItems.join(' / ')} className="mt-1 text-xs" tone="muted" copyable={false} expandable />
        </div>
        <div className="flex flex-wrap items-center justify-end gap-1.5">
          {trace.warnings.length > 0 && (
            <StatusBadge status="warning" label={t('aiCenter.routing.traceWarnings', '{{count}} warnings', { count: trace.warnings.length })} />
          )}
          <StatusBadge status={status === 'selected' ? 'ok' : status === 'fallback' ? 'warning' : 'error'} label={status} />
        </div>
      </div>
      <p className="mt-2 text-sm" style={{ color: 'var(--cp-text)' }}>{trace.user_summary?.reason_short}</p>

      <div className="mt-3 flex flex-wrap gap-1.5">
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

      <div className="mt-3 rounded-md p-2" style={{ background: 'var(--cp-surface)' }}>
        <div className="mb-2 flex items-center justify-between gap-2">
          <div className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.routing.rankedCandidates', 'Ranked candidates')}
          </div>
          <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>{trace.ranked_candidates.length}</div>
        </div>
        {visibleRankedCandidates.length > 0 ? (
          <div className="flex flex-col gap-1">
            {visibleRankedCandidates.map((candidate, index) => (
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
          <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.routing.noRankedCandidates', 'No ranked candidates.')}
          </div>
        )}
        {hiddenRankedCount > 0 && (
          <button
            type="button"
            onClick={(event) => {
              event.stopPropagation()
              setRankedExpanded((value) => !value)
            }}
            className="mt-2 inline-flex min-h-8 items-center gap-1 rounded-md px-2 text-xs font-medium"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
          >
            {rankedExpanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            {rankedExpanded
              ? t('aiCenter.routing.traceHideCandidates', 'Hide candidates')
              : t('aiCenter.routing.traceShowRankedCandidates', 'Show ranked candidates ({{count}})', { count: hiddenRankedCount })}
          </button>
        )}
      </div>

      <div className="mt-3 rounded-md p-2" style={{ background: 'var(--cp-surface)' }}>
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <div className="text-xs font-medium" style={{ color: 'var(--cp-muted)' }}>
            {t('aiCenter.routing.filteredOut', 'Filtered out')}
          </div>
          <button
            type="button"
            disabled={trace.filtered_candidates.length === 0}
            onClick={(event) => {
              event.stopPropagation()
              setFilteredExpanded((value) => !value)
            }}
            className="inline-flex min-h-7 items-center gap-1 rounded-md px-2 text-xs font-medium disabled:opacity-50"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
          >
            {filteredExpanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}
            {filteredExpanded
              ? t('common.collapse', 'Collapse')
              : t('aiCenter.routing.traceShowFilteredCandidates', 'Show {{count}}', { count: trace.filtered_candidates.length })}
          </button>
        </div>
        {!filteredExpanded && (
          <div className="text-xs" style={{ color: 'var(--cp-muted)' }}>
            {trace.filtered_candidates.length > 0
              ? t('aiCenter.routing.filteredOutCollapsed', '{{count}} candidates hidden', { count: trace.filtered_candidates.length })
              : t('aiCenter.routing.noFilteredCandidates', 'No candidates were filtered out.')}
          </div>
        )}
        {filteredExpanded && (
          <div className="flex flex-col gap-1">
            {trace.filtered_candidates.map((candidate) => (
              <TraceFilteredCandidateRow key={candidate.exact_model} candidate={candidate} />
            ))}
            <button
              type="button"
              onClick={(event) => {
                event.stopPropagation()
                setFilteredExpanded(false)
              }}
              className="mt-2 inline-flex min-h-8 w-full items-center justify-center gap-1 rounded-md px-2 text-xs font-medium"
              style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
            >
              <ChevronUp size={13} />
              {t('common.collapse', 'Collapse')}
            </button>
          </div>
        )}
      </div>

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

function TraceAuditSkeletonRows() {
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

function traceMatchesQuery(trace: RouteTrace, query: string): boolean {
  if (!query) return true
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
  ].join(' ').toLowerCase().includes(query)
}

function traceMatchesOutcome(trace: RouteTrace, filter: TraceOutcomeFilter): boolean {
  if (filter === 'all') return true
  if (filter === 'fallback') return trace.fallback_applied
  if (filter === 'failed') return !trace.selected_exact_model
  if (filter === 'warning') return trace.warnings.length > 0
  return true
}

function selectedTraceCandidate(trace: RouteTrace): RouteTrace['ranked_candidates'][number] | undefined {
  return trace.ranked_candidates.find((candidate) => candidate.selected)
    ?? trace.ranked_candidates.find((candidate) => candidate.exact_model === trace.selected_exact_model)
}

function traceStatus(trace: RouteTrace): 'selected' | 'fallback' | 'failed' {
  if (!trace.selected_exact_model) return 'failed'
  return trace.fallback_applied ? 'fallback' : 'selected'
}

function rankedCandidateRank(trace: RouteTrace, candidate: RouteTrace['ranked_candidates'][number], fallbackIndex: number): number {
  const index = trace.ranked_candidates.findIndex((item) => item.exact_model === candidate.exact_model)
  return (index >= 0 ? index : fallbackIndex) + 1
}

function traceEmptyStateKind(
  loadedCount: number,
  visibleCount: number,
  error: 'initial' | 'more' | null,
  hasSearch: boolean,
  outcomeFilter: TraceOutcomeFilter,
): TraceEmptyStateKind {
  if (loadedCount === 0 && error === 'initial') return 'load-failed'
  if (loadedCount === 0) return 'none-yet'
  if (visibleCount === 0 && (hasSearch || outcomeFilter !== 'all')) return 'no-matches'
  return 'no-matches'
}

function traceEmptyStateLabel(kind: TraceEmptyStateKind, t: (key: string, fallback: string) => string): string {
  if (kind === 'load-failed') return t('aiCenter.routing.traceLoadFailed', 'Failed to load traces')
  if (kind === 'none-yet') return t('aiCenter.routing.traceNoneYet', 'No route traces yet')
  return t('aiCenter.routing.traceNoMatches', 'No traces match current filters')
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
  if (weight > 1) return 'boost'
  if (weight < 1) return 'down'
  return 'neutral'
}

function formatWeight(weight: number): string {
  return weight.toFixed(2).replace(/\.?0+$/, '')
}
