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
  FileText,
  FolderClosed,
  Search,
  Sparkles,
  Wand2,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { SearchResultItem } from './types'
import type { SearchViewState } from './data/state'
import { groupSearchItems } from './data/search'

interface SearchResultsProps {
  state: SearchViewState
  query: string
  onSelect: (item: SearchResultItem) => void
  onRetry: () => void
  onLoadMore: () => void
}

const reasonMeta: Record<
  string,
  { labelKey: string; fallback: string; icon: React.ReactNode; tone: string }
> = {
  filename: {
    labelKey: 'filebrowser.search.reason.filename',
    fallback: 'File name',
    icon: <FileText size={12} />,
    tone: 'text-[color:var(--cp-accent)]',
  },
  folder: {
    labelKey: 'filebrowser.search.reason.folder',
    fallback: 'Folder',
    icon: <FolderClosed size={12} />,
    tone: 'text-[color:var(--cp-muted)]',
  },
  fulltext: {
    labelKey: 'filebrowser.search.reason.fulltext',
    fallback: 'Full-text',
    icon: <FileText size={12} />,
    tone: 'text-[color:var(--cp-warning)]',
  },
  ai_semantic: {
    labelKey: 'filebrowser.search.reason.aiSemantic',
    fallback: 'AI semantic',
    icon: <Sparkles size={12} />,
    tone: 'text-[color:var(--cp-success)]',
  },
  ai_topic: {
    labelKey: 'filebrowser.search.reason.aiTopic',
    fallback: 'AI topic',
    icon: <Wand2 size={12} />,
    tone: 'text-[color:var(--cp-success)]',
  },
}

/** Safe fallback for reasons the registry does not know (§1.3). */
function metaFor(reason: string) {
  return (
    reasonMeta[reason] ?? {
      labelKey: `filebrowser.search.reason.${reason}`,
      fallback: reason.replaceAll('_', ' '),
      icon: <Search size={12} />,
      tone: 'text-[color:var(--cp-muted)]',
    }
  )
}

function HitButton({
  item,
  onSelect,
}: {
  item: SearchResultItem
  onSelect: (item: SearchResultItem) => void
}) {
  const { t } = useI18n()
  const meta = metaFor(item.reason)
  const { entry } = item
  return (
    <button
      type="button"
      onClick={() => onSelect(item)}
      className="flex w-full items-start gap-3 rounded-[16px] border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] p-3 text-left hover:border-[color:var(--cp-accent)]"
    >
      <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-[12px] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)]">
        {entry.kind === 'folder' ? (
          <FolderClosed size={16} className="text-[color:var(--cp-accent)]" />
        ) : (
          <FileText size={16} className="text-[color:var(--cp-muted)]" />
        )}
      </div>
      <div className="min-w-0 flex-1">
        <div className="truncate text-sm font-semibold text-[color:var(--cp-text)]">
          {entry.name}
        </div>
        <div className="mt-0.5 truncate font-mono text-[10px] text-[color:var(--cp-muted)]">
          {entry.path}
        </div>
        <div className={clsx('mt-1 inline-flex items-center gap-1 text-[11px]', meta.tone)}>
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
    </button>
  )
}

function SkeletonHits() {
  return (
    <div className="space-y-1.5 px-4 py-4" data-testid="search-skeleton">
      {Array.from({ length: 5 }, (_, i) => (
        <div
          key={i}
          className="flex items-start gap-3 rounded-[16px] border border-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)] p-3"
        >
          <span className="h-9 w-9 shrink-0 animate-pulse rounded-[12px] bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]" />
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
  onLoadMore,
}: SearchResultsProps) {
  const { t } = useI18n()

  const page = state.data
  const items = page?.items ?? []
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
      <div className="flex flex-1 flex-col items-center justify-center gap-3 p-10 text-center text-sm text-[color:var(--cp-muted)]">
        <AlertTriangle size={22} className="text-[color:var(--cp-warning)]" />
        <p>{state.error ? t(state.error.messageKey, state.error.fallback) : null}</p>
        <button
          type="button"
          onClick={onRetry}
          className="rounded-full border border-[color:var(--cp-border)] px-4 py-1.5 text-sm text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
        >
          {t('filebrowser.retry', 'Retry')}
        </button>
      </div>
    )
  } else if (loading && !items.length) {
    body = <SkeletonHits />
  } else if (!items.length) {
    body = (
      <div className="flex flex-1 items-center justify-center p-10 text-center text-sm text-[color:var(--cp-muted)]">
        {t(
          'filebrowser.search.empty',
          'No results. Try a simpler keyword, or switch to Topic mode to browse by memory.',
        )}
      </div>
    )
  } else {
    body = (
      <div className="flex-1 space-y-4 px-4 py-4">
        {sections.map(({ key, label, list }) => {
          if (!list.length) return null
          return (
            <div key={key}>
              <p className="shell-kicker mb-1.5 !text-[10px]">{label}</p>
              <div className="space-y-1.5">
                {list.map((item) => (
                  <HitButton
                    key={`${item.reason}-${item.entry.id}`}
                    item={item}
                    onSelect={onSelect}
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
            className="w-full rounded-[14px] border border-dashed border-[color:var(--cp-border)] px-4 py-2 text-sm text-[color:var(--cp-muted)] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)] disabled:opacity-60"
            data-testid="search-load-more"
          >
            {loading
              ? t('filebrowser.search.loadingMore', 'Loading…')
              : t('filebrowser.search.loadMore', 'Load more results')}
          </button>
        ) : loading ? (
          <p className="py-1 text-center text-[11px] text-[color:var(--cp-muted)]">
            {t('filebrowser.search.loadingMore', 'Loading…')}
          </p>
        ) : null}
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col overflow-y-auto">
      <div className="border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-5 py-3">
        <div className="shell-kicker">{t('filebrowser.search.title', 'Search results')}</div>
        <div className="mt-1 text-sm text-[color:var(--cp-text)]">
          {t('filebrowser.search.query', 'For “{{query}}”', { query })}
          <span className="ml-2 text-[color:var(--cp-muted)]">
            {loading && !page
              ? t('filebrowser.search.searching', 'searching…')
              : t('filebrowser.search.count', '{{count}} results', { count: items.length })}
          </span>
        </div>
        {page?.sources.length ? (
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[10px] text-[color:var(--cp-muted)]">
            {page.sources.map((source) => (
              <span
                key={source.mode}
                title={source.reason}
                className={clsx(
                  'inline-flex items-center gap-1 rounded-full border px-2 py-0.5',
                  source.state === 'ok'
                    ? 'border-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)]'
                    : 'border-[color:var(--cp-warning)] text-[color:var(--cp-warning)]',
                )}
              >
                {source.mode}
                {source.state !== 'ok' ? ` · ${source.state}` : ''}
                {source.tookMs !== undefined ? ` · ${source.tookMs}ms` : ''}
              </span>
            ))}
          </div>
        ) : null}
        {incomplete ? (
          <div
            className="mt-2 flex items-center gap-1.5 rounded-[10px] bg-[color:color-mix(in_srgb,var(--cp-warning)_14%,var(--cp-surface))] px-2.5 py-1.5 text-[11px] text-[color:var(--cp-warning)]"
            data-testid="search-partial-banner"
          >
            <AlertTriangle size={12} />
            {t(
              'filebrowser.search.partial',
              'Some sources are unavailable — coverage is incomplete.',
            )}
          </div>
        ) : null}
      </div>
      {body}
    </div>
  )
}
