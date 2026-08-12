import { useEffect, useRef, type RefObject } from 'react'
import { ChevronDown, ChevronLeft, ChevronRight, RefreshCw } from 'lucide-react'

export type PagedListMode = 'pagination' | 'infinite'

interface InfiniteScrollSentinelProps {
  loading: boolean
  error?: string | null
  hasMore: boolean
  onLoadMore: () => void
  onRetry?: () => void
  rootRef?: RefObject<HTMLElement | null>
  loadingLabel: string
  loadMoreLabel: string
  retryLabel: string
  errorLabel: string
}

export function InfiniteScrollSentinel({
  loading,
  error,
  hasMore,
  onLoadMore,
  onRetry,
  rootRef,
  loadingLabel,
  loadMoreLabel,
  retryLabel,
  errorLabel,
}: InfiniteScrollSentinelProps) {
  const sentinelRef = useRef<HTMLDivElement | null>(null)

  useEffect(() => {
    const node = sentinelRef.current
    if (!node || loading || error || !hasMore) return
    const observer = new IntersectionObserver(
      (entries) => {
        if (entries.some((entry) => entry.isIntersecting)) {
          onLoadMore()
        }
      },
      { root: rootRef?.current ?? null, rootMargin: '160px 0px' },
    )
    observer.observe(node)
    return () => observer.disconnect()
  }, [error, hasMore, loading, onLoadMore, rootRef])

  if (!hasMore && !loading && !error) {
    return <div ref={sentinelRef} className="h-1" aria-hidden />
  }

  return (
    <div ref={sentinelRef} className="mt-3 flex min-h-11 items-center justify-center">
      {loading ? (
        <div className="inline-flex min-h-11 items-center gap-2 text-sm" style={{ color: 'var(--cp-muted)' }}>
          <RefreshCw size={15} className="animate-spin" />
          {loadingLabel}
        </div>
      ) : error ? (
        <div className="flex w-full flex-col gap-2 rounded-lg px-3 py-2 text-sm" style={{ background: 'var(--cp-bg)', color: 'var(--cp-warning)' }}>
          <span>{errorLabel}</span>
          <button
            type="button"
            onClick={onRetry ?? onLoadMore}
            className="inline-flex min-h-11 items-center justify-center gap-2 rounded-lg px-3 font-medium"
            style={{ color: 'var(--cp-accent)', border: '1px solid var(--cp-border)' }}
          >
            <RefreshCw size={15} />
            {retryLabel}
          </button>
        </div>
      ) : hasMore ? (
        <button
          type="button"
          onClick={onLoadMore}
          className="inline-flex min-h-11 w-full items-center justify-center gap-2 rounded-lg px-3 text-sm font-medium"
          style={{ color: 'var(--cp-accent)', background: 'var(--cp-bg)', border: '1px solid var(--cp-border)' }}
        >
          <ChevronDown size={15} />
          {loadMoreLabel}
        </button>
      ) : null}
    </div>
  )
}

interface PagedListFooterProps {
  mode: PagedListMode
  loading: boolean
  error?: string | null
  hasMore: boolean
  onLoadMore: () => void
  onRetry?: () => void
  onPreviousPage?: () => void
  onNextPage?: () => void
  canGoPrevious?: boolean
  canGoNext?: boolean
  pageIndex?: number
  loadedCount?: number
  totalCount?: number
  rootRef?: RefObject<HTMLElement | null>
  labels: {
    previous: string
    next: string
    page: string
    loading: string
    loadMore: string
    retry: string
    error: string
    loaded?: string
  }
}

export function PagedListFooter({
  mode,
  loading,
  error,
  hasMore,
  onLoadMore,
  onRetry,
  onPreviousPage,
  onNextPage,
  canGoPrevious = false,
  canGoNext = false,
  pageIndex = 0,
  loadedCount,
  totalCount,
  rootRef,
  labels,
}: PagedListFooterProps) {
  if (mode === 'infinite') {
    return (
      <InfiniteScrollSentinel
        loading={loading}
        error={error}
        hasMore={hasMore}
        onLoadMore={onLoadMore}
        onRetry={onRetry}
        rootRef={rootRef}
        loadingLabel={labels.loading}
        loadMoreLabel={labels.loadMore}
        retryLabel={labels.retry}
        errorLabel={labels.error}
      />
    )
  }

  return (
    <div className="flex flex-wrap items-center justify-between gap-3 px-4 py-3" style={{ background: 'var(--cp-surface)', borderTop: '1px solid var(--cp-border)' }}>
      <span className="text-xs" style={{ color: 'var(--cp-muted)' }}>
        {labels.loaded ?? (totalCount == null || loadedCount == null ? '' : `${loadedCount} / ${totalCount}`)}
      </span>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onPreviousPage}
          disabled={!canGoPrevious || loading}
          className="inline-flex min-h-11 items-center gap-1 rounded-md px-3 text-xs font-medium disabled:opacity-45"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          <ChevronLeft size={14} />
          {labels.previous}
        </button>
        <span className="text-xs tabular-nums" style={{ color: 'var(--cp-muted)' }}>
          {loading ? labels.loading : labels.page.replace('{{page}}', String(pageIndex + 1))}
        </span>
        <button
          type="button"
          onClick={onNextPage}
          disabled={!canGoNext || loading}
          className="inline-flex min-h-11 items-center gap-1 rounded-md px-3 text-xs font-medium disabled:opacity-45"
          style={{ color: 'var(--cp-text)', border: '1px solid var(--cp-border)' }}
        >
          {labels.next}
          <ChevronRight size={14} />
        </button>
      </div>
    </div>
  )
}
