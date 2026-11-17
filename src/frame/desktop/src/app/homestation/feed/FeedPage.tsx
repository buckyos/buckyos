import { ChevronRight, Inbox, Loader2, Search, X } from 'lucide-react'
import { useEffect, useRef } from 'react'
import { useI18n } from '../../../i18n/provider'
import { FeedCard } from '../card/FeedCard'
import type { ReadingQuery, TopicView } from '../datamodel/types'
import { FilterBar } from '../FilterBar'
import type { HomeStationStore } from '../store/types'
import { useHsNav } from '../navContext'
import { useHomeStationStore, useStoreSelector } from '../store/context'
import type { ReadingMode } from '../types'
import { Avatar, ColumnBody, EmptyState, ErrorState, ListSkeleton } from '../ui/primitives'
import { useReadingList } from './useReadingList'
import { useScrollMemory } from './useScrollMemory'

const selectSync = (store: HomeStationStore) => store.peekSyncStatus()
const selectOwner = (store: HomeStationStore) => store.peekIdentity(store.owner)

function CatchUpEntry({ isMobile }: { isMobile: boolean }) {
  const { t } = useI18n()
  const nav = useHsNav()
  const sync = useStoreSelector(selectSync)
  if (sync.candidates === 0) return null
  return (
    <button
      type="button"
      className={isMobile ? 'hs-embed mx-4 my-4 flex w-[calc(100%-32px)] items-center gap-3 text-left' : 'hs-embed mx-6 my-4 flex w-[calc(100%-48px)] items-center gap-3 text-left'}
      onClick={() => nav.navigate({ name: 'candidates' })}
      data-testid="hs-catchup-entry"
    >
      <Inbox size={20} style={{ color: 'var(--cp-muted)' }} />
      <span className="min-w-0 flex-1">
        <span className="block text-[15px] font-medium leading-[22px]">{t('homestation.catchup.entry', '{{n}} more from people you follow haven’t entered your feed', { n: sync.candidates })}</span>
        <span className="block text-[13px] leading-5" style={{ color: 'var(--cp-muted)' }}>{t('homestation.catchup.entryHint', 'Fetched but not selected yet. Open to catch up.')}</span>
      </span>
      <ChevronRight size={18} style={{ color: 'var(--cp-muted)' }} />
    </button>
  )
}

function ComposeRow() {
  const { t } = useI18n()
  const nav = useHsNav()
  const owner = useStoreSelector(selectOwner)
  const open = () => nav.navigate({ name: 'publish' })
  return (
    <div className="hs-card hs-card-row items-center">
      <Avatar identity={owner} size={36} />
      <button type="button" className="min-w-0 flex-1 truncate text-left text-[15px] leading-[22px]" style={{ color: 'var(--cp-muted)' }} onClick={open} data-testid="hs-compose-open">
        {t('homestation.publish.placeholder', 'What’s new?')}
      </button>
      <button type="button" className="hs-btn" onClick={open}>{t('homestation.publish.submit', 'Post')}</button>
    </div>
  )
}

function SearchField({ query, onQueryChange, onClose, isMobile }: { query: ReadingQuery; onQueryChange: (patch: Partial<ReadingQuery>) => void; onClose: () => void; isMobile: boolean }) {
  const { t } = useI18n()
  return (
    <div className={isMobile ? 'px-4 pt-3' : 'px-6 pt-4'}>
      <div className="hs-search-field flex h-10 items-center gap-2 rounded-[10px] px-3" style={{ background: 'var(--hs-chip-bg)' }}>
        <Search size={16} style={{ color: 'var(--cp-muted)' }} />
        <input
          type="search"
          value={query.search}
          autoFocus
          onChange={event => onQueryChange({ search: event.target.value })}
          placeholder={t('homestation.search.placeholder', 'Search this view')}
          aria-label={t('homestation.search.placeholder', 'Search this view')}
          className="min-w-0 flex-1 bg-transparent text-[15px] outline-none"
          data-testid="hs-search-input"
        />
        <button type="button" className="flex h-6 w-6 items-center justify-center rounded-full" aria-label={t('homestation.search.clear', 'Clear search')} onClick={() => { onQueryChange({ search: '' }); onClose() }} style={{ color: 'var(--cp-muted)' }}>
          <X size={16} />
        </button>
      </div>
    </div>
  )
}

export function FeedPage({ query, onQueryChange, readingMode, onReadingModeChange, topics, isMobile, header, showSearch = false, onCloseSearch }: { query: ReadingQuery; onQueryChange: (patch: Partial<ReadingQuery>) => void; readingMode: ReadingMode; onReadingModeChange: (mode: ReadingMode) => void; topics: TopicView[]; isMobile: boolean; header?: React.ReactNode; showSearch?: boolean; onCloseSearch?: () => void }) {
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
  const edge = isMobile ? 'px-4' : 'px-6'

  return (
    <div className="flex h-full min-h-0 flex-col">
      {header}
      <ColumnBody ref={scrollRef} testId="hs-feed-scroll">
        {!isMobile ? <ComposeRow /> : null}
        <div className="sticky top-0 z-10" style={{ background: 'var(--hs-column)' }}>
          {showSearch ? <SearchField query={query} onQueryChange={onQueryChange} onClose={() => onCloseSearch?.()} isMobile={isMobile} /> : null}
          <FilterBar query={query} readingMode={readingMode} topics={topics} onQueryChange={onQueryChange} onReadingModeChange={onReadingModeChange} isMobile={isMobile} />
        </div>
        {topic || searching ? (
          <div className={`hs-meta ${edge} py-3`} style={{ borderBottom: '1px solid var(--hs-line)' }} data-testid="hs-view-scope">
            {topic ? (
              <button type="button" className="hs-link" onClick={() => onQueryChange({ topicId: null })} aria-label={t('homestation.topics.clear', 'Clear topic {{name}}', { name: topic.name })}>
                {t('homestation.topics.active', 'Topic: {{name}}', { name: topic.name })}
                <X size={14} />
              </button>
            ) : null}
            {searching ? <span>{t('homestation.search.scope', 'Searching this view for “{{q}}”', { q: query.search.trim() })}</span> : null}
          </div>
        ) : null}
        {meta && (meta.hiddenByRules > 0 || query.showFiltered || meta.hiddenByMute > 0) ? (
          <div className={`hs-meta ${edge} py-3`} style={{ borderBottom: '1px solid var(--hs-line)' }} data-testid="hs-hidden-summary">
            {query.showFiltered ? (
              <span>{t('homestation.filters.showingFiltered', 'Showing items your filters hide, marked')}</span>
            ) : meta.hiddenByRules > 0 ? (
              <span>{t('homestation.filters.hiddenCount', '{{n}} hidden by your filters', { n: meta.hiddenByRules })}</span>
            ) : null}
            {meta.hiddenByRules > 0 || query.showFiltered ? (
              <button type="button" className="hs-link" data-testid="hs-toggle-filtered" onClick={() => onQueryChange({ showFiltered: !query.showFiltered })}>
                {query.showFiltered ? t('homestation.filters.hideAgain', 'Hide them again') : t('homestation.filters.show', 'Show them')}
              </button>
            ) : null}
            {meta.hiddenByMute > 0 ? <span>{t('homestation.filters.mutedCount', '{{n}} from people you chose not to see', { n: meta.hiddenByMute })}</span> : null}
          </div>
        ) : null}
        {list.isLoading ? <ListSkeleton rows={4} /> : null}
        {list.error ? <ErrorState onRetry={list.reload} /> : null}
        {ready && objIds.length === 0 ? (
          <EmptyState
            icon={<Inbox size={32} />}
            title={searching ? t('homestation.empty.search', 'No results for “{{q}}” in this view', { q: query.search.trim() }) : query.filter !== 'all' || query.topicId ? t('homestation.empty.view', 'Nothing in this view yet') : t('homestation.empty.feed', 'Your reading list is empty')}
            body={searching ? t('homestation.empty.searchBody', 'Search covers every page of this view, not just what is loaded. Try another view or clear the search.') : t('homestation.empty.feedBody', 'Your HomeStation prepares items from your sources in the background. Add sources or check the catch-up list.')}
          />
        ) : null}
        {objIds.map(objId => <FeedCard key={objId} objId={objId} mode={readingMode === 'immersive' ? 'standard' : readingMode} />)}
        <div ref={sentinelRef} />
        {isLoadingMore ? (
          <div className="flex items-center justify-center gap-2 py-6 text-[13px]" style={{ color: 'var(--cp-muted)' }}><Loader2 size={16} className="animate-spin" />{t('homestation.state.loadingMore', 'Loading more…')}</div>
        ) : hasMore && ready ? (
          <div className="flex justify-center py-4"><button type="button" className="hs-btn" onClick={loadMore} data-testid="hs-load-more">{t('homestation.state.loadMore', 'Load more')}</button></div>
        ) : ready && objIds.length > 0 ? (
          <p className="py-6 text-center text-[13px] tabular-nums" style={{ color: 'var(--cp-muted)' }}>{t('homestation.state.endOfView', 'End of this view · {{n}} items', { n: meta?.total ?? objIds.length })}</p>
        ) : null}
        {ready && !hasMore && query.filter === 'following' ? <CatchUpEntry isMobile={isMobile} /> : null}
        <div className="h-6" />
      </ColumnBody>
    </div>
  )
}
