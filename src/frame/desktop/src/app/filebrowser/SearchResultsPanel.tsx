import { useEffect, useRef } from 'react'
import type { MenuPosition, SelectModifiers } from './MainContent'
/**
 * Search results view (UI_DATAMODEL.md §2.9/§4.4).
 *
 * Consumes the async SearchViewState: skeleton while the first page loads,
 * search-scoped error + retry (the folder underneath is never lost), grouped
 * results with per-hit evidence, source status/partial-coverage indicator,
 * and cursor continuation ("load more"). Unknown reasons render in a generic
 * section — never dropped.
 */

import clsx from 'clsx'
import {
  AlertTriangle,
  Check,
  FileText,
  FolderClosed,
  Hash,
  MoreVertical,
  ScanSearch,
  Search,
  TextSearch,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { SearchResultItem } from './types'
import type { SearchViewState } from './data/state'
import { groupSearchItems } from './data/search'
import { fileIcon } from './fileDisplay'

interface SearchResultsProps {
  state: SearchViewState
  query: string
  onSelect: (item: SearchResultItem, modifiers?: SelectModifiers) => void
  onRetry: () => void
  onLoadMore: () => void
  selectedKeys: ReadonlySet<string>
  mobile?: boolean
  onOpen: (item: SearchResultItem) => void
  onMenu: (item: SearchResultItem, position: MenuPosition) => void
  scope: 'current' | 'all'
  currentUrl: string
  onScopeChange: (scope: 'current' | 'all') => void
  kind: string
  modified: string
  onFilterChange: (kind: string, modified: string) => void
  onExit: () => void
  scroll: number
  onScroll: (scroll: number) => void
}

const reasonMeta: Record<
  string,
  { labelKey: string; fallback: string; icon: React.ReactNode; tone: string }
> = {
  filename: {
    labelKey: 'filebrowser.search.reason.filename',
    fallback: 'File name',
    icon: <FileText size={14} />,
    tone: 'text-[color:var(--cp-muted)]',
  },
  folder: {
    labelKey: 'filebrowser.search.reason.folder',
    fallback: 'Folder',
    icon: <FolderClosed size={14} />,
    tone: 'text-[color:var(--cp-muted)]',
  },
  fulltext: {
    labelKey: 'filebrowser.search.reason.fulltext',
    fallback: 'Full-text',
    icon: <TextSearch size={14} />,
    tone: 'text-[color:var(--cp-muted)]',
  },
  ai_semantic: {
    labelKey: 'filebrowser.search.reason.aiSemantic',
    fallback: 'AI semantic',
    icon: <ScanSearch size={14} />,
    tone: 'text-[color:var(--cp-muted)]',
  },
  ai_topic: {
    labelKey: 'filebrowser.search.reason.aiTopic',
    fallback: 'AI topic',
    icon: <Hash size={14} />,
    tone: 'text-[color:var(--cp-muted)]',
  },
}

/** Safe fallback for reasons the registry does not know (§1.3). */
function metaFor(reason: string) {
  return (
    reasonMeta[reason] ?? {
      labelKey: `filebrowser.search.reason.${reason}`,
      fallback: reason.replaceAll('_', ' '),
      icon: <Search size={14} />,
      tone: 'text-[color:var(--cp-muted)]',
    }
  )
}

function HitButton({
  item,
  onSelect, onOpen, onMenu, selected, mobile,
}: {
  item: SearchResultItem
  selected: boolean
  mobile?: boolean
  onOpen: (item: SearchResultItem) => void
  onMenu: (item: SearchResultItem, position: MenuPosition) => void
  onSelect: (item: SearchResultItem, modifiers?: SelectModifiers) => void
}) {
  const { t } = useI18n()
  const meta = metaFor(item.reason)
  const { entry } = item
  return (
    <div className="relative"><button
      type="button"
      data-testid="search-hit"
      data-entry-id={entry.id}
      aria-pressed={selected}
      onClick={(event) => onSelect(item, { toggle: event.ctrlKey || event.metaKey, shift: event.shiftKey })}
      onDoubleClick={() => onOpen(item)}
      onKeyDown={(event) => { if (event.key === 'Enter') { event.preventDefault(); event.stopPropagation(); onOpen(item) } }}
      onContextMenu={(event) => { event.preventDefault(); onMenu(item, { top: event.clientY, left: event.clientX }) }}
      data-selected={selected}
      className={clsx('fb-row flex w-full items-start gap-3 rounded-[10px] px-3 py-2.5 text-left', mobile && 'pr-12')}
    >
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[8px] bg-[color:var(--cp-surface-2-opaque)]">
        {selected ? <Check size={16} aria-hidden className="text-[color:var(--cp-text)]" /> : fileIcon(entry, 18)}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-[14px] text-[color:var(--cp-text)]">
          {entry.name}
        </div>
        <div className="mt-0.5 truncate font-[family-name:var(--files-mono)] text-[12px] text-[color:var(--cp-muted)]">
          {entry.path}
        </div>
        <div className={clsx('mt-1 inline-flex items-center gap-1.5 text-[12px] tabular-nums', meta.tone)}>
          {meta.icon}
          {t(meta.labelKey, meta.fallback)}
          <span className="text-[color:var(--cp-muted)]"> · {item.detail}</span>
          {item.score !== undefined ? (
            <span className="text-[color:var(--cp-muted)]">
              {' '}
              · {Math.round(item.score * 100)}%
            </span>
          ) : null}
        </div>
      </div>
    </button>{mobile && <button type="button" className="absolute right-1 top-1 flex h-11 w-11 items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:var(--files-pressed)]" aria-label={t('filebrowser.mobile.moreActions', 'More actions')} onClick={(event) => onMenu(item, { left: event.clientX, top: event.clientY })}><MoreVertical size={20} /></button>}</div>
  )
}

function SkeletonHits() {
  return (
    <div className="space-y-1 px-3 py-3" data-testid="search-skeleton">
      {Array.from({ length: 5 }, (_, i) => (
        <div
          key={i}
          className="flex items-start gap-3 rounded-[10px] px-3 py-2.5"
        >
          <span className="h-9 w-9 shrink-0 animate-pulse rounded-[8px] bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]" />
          <div className="flex min-w-0 flex-1 flex-col gap-1.5 pt-1">
            <span
              className="inline-block h-3 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_55%,transparent)]"
              style={{ width: `${52 - (i % 3) * 9}%` }}
            />
            <span
              className="inline-block h-2.5 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]"
              style={{ width: '36%' }}
            />
          </div>
        </div>
      ))}
    </div>
  )
}

export function SearchResultsPanel({
  state,
  query,
  onSelect,
  onRetry,
  onLoadMore, mobile, selectedKeys, onOpen, onMenu, scope, currentUrl, onScopeChange, kind, modified, onFilterChange, onExit, scroll, onScroll,
}: SearchResultsProps) {
  const { t } = useI18n()

  const scroller = useRef<HTMLDivElement>(null)
  const restored = useRef(false)
  useEffect(() => { restored.current = false }, [query, scope, kind, modified])
  useEffect(() => { if (state.status === 'success' && !restored.current && scroller.current) { scroller.current.scrollTop = scroll; restored.current = true } }, [state.status, query, scope, scroll])
  const page = state.data
  const items = (page?.items ?? []).filter((hit) => (!kind || hit.entry.kind === kind) && (!modified || Date.parse(hit.entry.modifiedAt) >= Date.parse(modified)))
  const loading = state.status === 'loading'
  const degradedSources = page?.sources.filter((source) => source.state !== 'ok') ?? []
  const incomplete = !!page && (page.partial || degradedSources.length > 0)
  const grouped = groupSearchItems(items)

  const sections: { key: string; label: string; list: SearchResultItem[] }[] = [
    {
      key: 'traditional',
      label: t('filebrowser.search.section.traditional', 'Traditional matches'),
      list: grouped.traditional,
    },
    {
      key: 'ai',
      label: t('filebrowser.search.section.ai', 'AI-enhanced matches'),
      list: grouped.ai,
    },
    {
      key: 'other',
      label: t('filebrowser.search.section.other', 'Other matches'),
      list: grouped.other,
    },
  ]

  let body: React.ReactNode
  if (state.status === 'error' && !items.length) {
    body = (
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-10 text-center text-[14px] text-[color:var(--cp-muted)]">
        <AlertTriangle size={24} strokeWidth={1.5} className="text-[color:var(--files-warning-ink)]" />
        <p>{state.error ? t(state.error.messageKey, state.error.fallback) : null}</p>
        <button
          type="button"
          onClick={onRetry}
          className="files-btn"
        >
          {t('filebrowser.retry', 'Retry')}
        </button>
      </div>
    )
  } else if (loading && !items.length) {
    body = <SkeletonHits />
  } else if (!items.length) {
    body = (
      <div className="flex flex-1 items-center justify-center p-10 text-center text-[14px] leading-6 text-[color:var(--cp-muted)]">
        {t(
          'filebrowser.search.empty',
          'No results. Try a simpler keyword, or switch to Topic mode to browse by memory.',
        )}
      </div>
    )
  } else {
    body = (
      <div className="flex-1 space-y-5 px-3 py-4">
        {sections.map(({ key, label, list }) => {
          if (!list.length) return null
          return (
            <div key={key}>
              <p className="files-label mb-1 px-3">{label}</p>
              <div className="space-y-0.5">
                {list.map((item) => (
                  <HitButton
                    key={`${item.reason}-${item.entry.id}`}
                    item={item}
                    onSelect={onSelect}
                    onOpen={onOpen}
                    onMenu={onMenu}
                    selected={selectedKeys.has(item.entry.id)}
                    mobile={mobile}
                  />
                ))}
              </div>
            </div>
          )
        })}
        {page?.nextCursor ? (
          <button
            type="button"
            onClick={onLoadMore}
            disabled={loading}
            className="files-btn w-full"
            data-testid="search-load-more"
          >
            {loading
              ? t('filebrowser.search.loadingMore', 'Loading…')
              : t('filebrowser.search.loadMore', 'Load more results')}
          </button>
        ) : loading ? (
          <p className="py-1 text-center text-[12px] text-[color:var(--cp-muted)]">
            {t('filebrowser.search.loadingMore', 'Loading…')}
          </p>
        ) : null}
      </div>
    )
  }

  return (
    <div ref={scroller} onScroll={(event) => onScroll(event.currentTarget.scrollTop)} className="flex h-full w-full flex-col overflow-y-auto">
      <div className="flex shrink-0 flex-wrap items-center gap-x-3 gap-y-2 border-b border-[color:var(--files-hairline)] px-4 py-2 text-[13px] text-[color:var(--cp-muted)]">
        <label>{t('filebrowser.search.scope', 'Search in')} <select aria-label={t('filebrowser.search.scope', 'Search in')} value={scope} onChange={(event) => onScopeChange(event.target.value as 'current' | 'all')} className="files-input ml-1 !min-h-8 !w-auto max-w-52 !px-2 !text-[13px]">
          <option value="current">{t('filebrowser.search.currentScope', 'Current folder and subfolders')} · {currentUrl}</option><option value="all">{t('filebrowser.search.allScope', 'All accessible files')}</option>
        </select></label>
        <label>{t('filebrowser.column.kind', 'Kind')} <select aria-label={t('filebrowser.column.kind', 'Kind')} value={kind} onChange={(event) => onFilterChange(event.target.value, modified)} className="files-input ml-1 !min-h-8 !w-auto !px-2 !text-[13px]"><option value="">{t('filebrowser.search.allKinds', 'All types')}</option>{['folder','document','image','video','audio','archive','code','other'].map((kind) => <option key={kind} value={kind}>{t(`filebrowser.kind.${kind}`, kind)}</option>)}</select></label>
        <label>{t('filebrowser.search.modifiedSince', 'Modified since')} <input aria-label={t('filebrowser.search.modifiedSince', 'Modified since')} type="date" value={modified} onChange={(event) => onFilterChange(kind, event.target.value)} className="files-input ml-1 !min-h-8 !w-auto !px-2 !text-[13px]" /></label>
        {(kind || modified) && <button type="button" className="files-btn files-btn-ghost" onClick={() => onFilterChange('', '')}>{t('filebrowser.search.clearFilters', 'Clear filters')}</button>}
        <button type="button" className="files-btn files-btn-ghost ml-auto" onClick={onExit}>{t('filebrowser.search.exit', 'Exit search')}</button>
        {(kind || modified) && <p className="w-full text-[12px]">{t('filebrowser.search.loadedFilters', 'Filters apply to returned results. Load more to check the remaining results.')}</p>}
      </div>
      <div className="border-b border-[color:var(--files-hairline)] px-4 py-3">
        <div className="files-label">{t('filebrowser.search.title', 'Search results')}</div>
        <div className="mt-1 text-[14px] text-[color:var(--cp-text)]">
          {t('filebrowser.search.query', 'For “{{query}}”', { query })}
          <span className="ml-2 tabular-nums text-[color:var(--cp-muted)]">
            {loading && !page
              ? t('filebrowser.search.searching', 'searching…')
              : t('filebrowser.search.count', '{{count}} results', { count: items.length })}
          </span>
        </div>
        {page?.sources.length ? (
          <details className="mt-2 text-[12px] text-[color:var(--cp-muted)]"><summary className="cursor-pointer">{t('filebrowser.search.sourceDetails', 'Search source details')}</summary><div className="flex flex-wrap gap-1.5">
            {page.sources.map((source) => (
              <span
                key={source.mode}
                title={source.reason}
                className={clsx('files-chip mt-1.5 tabular-nums', source.state !== 'ok' && '!text-[color:var(--files-warning-ink)]')}
              >
                {source.mode}
                {source.state !== 'ok' ? ` · ${source.state}` : ''}
                {source.tookMs !== undefined ? ` · ${source.tookMs}ms` : ''}
              </span>
            ))}
          </div></details>
        ) : null}
        {incomplete ? (
          <div
            className="mt-2 flex items-center gap-2 rounded-[8px] bg-[color:color-mix(in_srgb,var(--cp-warning)_14%,var(--cp-surface-opaque))] px-3 py-2 text-[12px] text-[color:var(--files-warning-ink)]"
            data-testid="search-partial-banner"
          >
            <AlertTriangle size={14} className="shrink-0" />
            {t(
              'filebrowser.search.partial',
              'Some sources are unavailable — coverage is incomplete.',
            )}
          </div>
        ) : null}
      </div>
      {body}
      {!items.length && page?.nextCursor && <button type="button" className="files-btn mx-4 mb-4" onClick={onLoadMore} disabled={loading}>{t('filebrowser.search.loadMore', 'Load more results')}</button>}
    </div>
  )
}
