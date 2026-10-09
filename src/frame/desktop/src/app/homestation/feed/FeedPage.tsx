import { ArrowRight, EyeOff, Inbox, Loader2, Search, SlidersHorizontal } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useI18n } from '../../../i18n/provider'
import { FeedCard } from '../card/FeedCard'
import type { ReadingQuery, TopicView } from '../datamodel/types'
import { FilterBar } from '../FilterBar'
import type { HomeStationStore } from '../mock/store'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import type { ReadingMode } from '../types'
import { EmptyState, ErrorState, ListSkeleton } from '../ui/primitives'
import { useReadingList } from './useReadingList'
import { useScrollMemory } from './useScrollMemory'

const selectSync = (store: HomeStationStore) => store.peekSyncStatus()

function CatchUpEntry() {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  if (sync.candidates === 0) return null
  return (
    <button
      type="button"
      className="mx-4 my-4 flex w-[calc(100%-2rem)] items-center gap-3 rounded-2xl border px-4 py-3 text-left text-sm"
      style={{ borderColor: 'var(--hs-divider)', background: 'var(--hs-subtle-bg)' }}
      onClick={() => nav.navigate({ name: 'candidates' })}
      data-testid="hs-catchup-entry"
    >
      <Inbox size={18} style={{ color: 'var(--cp-accent)' }} />
      <span className="flex-1">
        <span className="block font-medium">{t('homestation.catchup.entry', '{{n}} more from people you follow haven’t entered your feed', { n: sync.candidates })}</span>
        <span className="block text-xs" style={{ color: 'var(--cp-muted)' }}>{t('homestation.catchup.entryHint', 'Fetched but not selected yet. Open to catch up.')}</span>
      </span>
      <ArrowRight size={16} style={{ color: 'var(--cp-muted)' }} />
    </button>
  )
}

export function FeedPage({ query, onQueryChange, readingMode, onReadingModeChange, topics, isMobile, header }: { query: ReadingQuery; onQueryChange: (patch: Partial<ReadingQuery>) => void; readingMode: ReadingMode; onReadingModeChange: (mode: ReadingMode) => void; topics: TopicView[]; isMobile: boolean; header?: React.ReactNode }) {
  const { t } = useI18n()
  const store = useHomeStationStore()
  const scrollRef = useRef<HTMLDivElement>(null)
  const sentinelRef = useRef<HTMLDivElement>(null)
  const list = useReadingList(query)
  const { objIds, meta, viewKey } = list
  const ready = !list.isLoading && !list.error
  useScrollMemory(scrollRef, `${store.id}:reading`, viewKey, objIds.join(','), ready)

  const { hasMore, isLoadingMore, loadMore } = list
  useEffect(() => {
    const sentinel = sentinelRef.current
    const root = scrollRef.current
    if (!sentinel || !root || !hasMore) return
    const observer = new IntersectionObserver(entries => {
      if (entries.some(entry => entry.isIntersecting) && !isLoadingMore) loadMore()
    }, { root, rootMargin: '240px' })
    observer.observe(sentinel)
    return () => observer.disconnect()
  }, [hasMore, isLoadingMore, loadMore])

  const topic = topics.find(entry => entry.id === query.topicId)
  const searching = query.search.trim().length > 0

  return (
    <div ref={scrollRef} className="desktop-scrollbar relative min-h-0 flex-1 overflow-y-auto" data-testid="hs-feed-scroll">
      {header}
      <div className="sticky top-0 z-10" style={{ background: 'var(--cp-bg)', borderBottom: '1px solid var(--hs-divider)' }}>
        <FilterBar query={query} readingMode={readingMode} topics={topics} onQueryChange={onQueryChange} onReadingModeChange={onReadingModeChange} isMobile={isMobile} />
        {topic || searching ? (
          <div className="flex flex-wrap items-center gap-1.5 px-4 pb-2 text-[11px]" style={{ color: 'var(--cp-muted)' }} data-testid="hs-view-scope">
            {topic ? <button type="button" className="hs-badge is-accent" onClick={() => onQueryChange({ topicId: null })}># {topic.name} ×</button> : null}
            {searching ? (
              <span className="inline-flex items-center gap-1"><Search size={11} />{t('homestation.search.scope', 'Searching “{{q}}” in this view only — not the whole network or full source history', { q: query.search.trim() })}</span>
            ) : null}
          </div>
        ) : null}
      </div>
      {meta && (meta.hiddenByRules > 0 || query.showFiltered || meta.hiddenByMute > 0) ? (
        <div className="flex flex-wrap items-center gap-2 px-4 py-2 text-xs" style={{ color: 'var(--cp-muted)', borderBottom: '1px solid var(--hs-divider)' }} data-testid="hs-hidden-summary">
          <SlidersHorizontal size={13} />
          {query.showFiltered ? (
            <span>{t('homestation.filters.showingFiltered', 'Showing items hidden by your filters (marked).')}</span>
          ) : meta.hiddenByRules > 0 ? (
            <span>{t('homestation.filters.hiddenCount', '{{n}} hidden by your filters', { n: meta.hiddenByRules })}</span>
          ) : null}
          {meta.hiddenByRules > 0 || query.showFiltered ? (
            <button type="button" className="hs-badge is-accent" data-testid="hs-toggle-filtered" onClick={() => onQueryChange({ showFiltered: !query.showFiltered })}>
              {query.showFiltered ? t('homestation.filters.hideAgain', 'Hide them again') : t('homestation.filters.show', 'Show them')}
            </button>
          ) : null}
          {meta.hiddenByMute > 0 ? <span className="inline-flex items-center gap-1"><EyeOff size={12} />{t('homestation.filters.mutedCount', '{{n}} from people you chose not to see', { n: meta.hiddenByMute })}</span> : null}
        </div>
      ) : null}
      <div className="mx-auto w-full max-w-[760px]">
        {list.isLoading ? <ListSkeleton rows={4} /> : null}
        {list.error ? <ErrorState onRetry={list.reload} /> : null}
        {ready && objIds.length === 0 ? (
          <EmptyState
            icon={<Inbox size={36} strokeWidth={1.3} />}
            title={searching ? t('homestation.empty.search', 'No results for “{{q}}” in this view', { q: query.search.trim() }) : query.filter !== 'all' || query.topicId ? t('homestation.empty.view', 'Nothing in this view yet') : t('homestation.empty.feed', 'Your reading list is empty')}
            body={searching ? t('homestation.empty.searchBody', 'Search covers every page of this view, not just what is loaded. Try another view or clear the search.') : t('homestation.empty.feedBody', 'Your HomeStation prepares items from your sources in the background. Add sources or check the catch-up list.')}
          />
        ) : null}
        {objIds.map(objId => <FeedCard key={objId} objId={objId} mode={readingMode === 'immersive' ? 'standard' : readingMode} />)}
        <div ref={sentinelRef} />
        {isLoadingMore ? (
          <div className="flex items-center justify-center gap-2 py-4 text-xs" style={{ color: 'var(--cp-muted)' }}><Loader2 size={14} className="animate-spin" />{t('homestation.state.loadingMore', 'Loading more…')}</div>
        ) : hasMore && ready ? (
          <div className="flex justify-center py-3"><button type="button" className="hs-btn" onClick={loadMore} data-testid="hs-load-more">{t('homestation.state.loadMore', 'Load more')}</button></div>
        ) : ready && objIds.length > 0 ? (
          <p className="py-4 text-center text-[11px]" style={{ color: 'var(--cp-muted)' }}>{t('homestation.state.endOfView', 'End of this view · {{n}} items', { n: meta?.total ?? objIds.length })}</p>
        ) : null}
        {ready && !hasMore && query.filter === 'following' ? <CatchUpEntry /> : null}
      </div>
      <div style={{ height: isMobile ? 88 : 24 }} />
    </div>
  )
}
