import clsx from 'clsx'
import { FileText, FolderClosed, Sparkles, Wand2 } from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { FileEntry } from './types'

interface SearchResultsProps {
  hits: { entry: FileEntry; reason: string; detail: string }[]
  query: string
  onSelect: (entry: FileEntry) => void
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

export function SearchResultsPanel({ hits, query, onSelect }: SearchResultsProps) {
  const { t } = useI18n()

  const grouped = {
    traditional: hits.filter((h) => h.reason === 'filename' || h.reason === 'folder' || h.reason === 'fulltext'),
    ai: hits.filter((h) => h.reason === 'ai_semantic' || h.reason === 'ai_topic'),
  }

  return (
    <div className="flex h-full w-full flex-col overflow-y-auto">
      <div className="border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-5 py-3">
        <div className="shell-kicker">{t('filebrowser.search.title', 'Search results')}</div>
        <div className="mt-1 text-sm text-[color:var(--cp-text)]">
          {t('filebrowser.search.query', 'For “{{query}}”', { query })}
          <span className="ml-2 text-[color:var(--cp-muted)]">
            {t('filebrowser.search.count', '{{count}} results', { count: hits.length })}
          </span>
        </div>
      </div>

      {!hits.length ? (
        <div className="flex flex-1 items-center justify-center p-10 text-center text-sm text-[color:var(--cp-muted)]">
          {t(
            'filebrowser.search.empty',
            'No results. Try a simpler keyword, or switch to Topic mode to browse by memory.',
          )}
        </div>
      ) : (
        <div className="flex-1 space-y-4 px-4 py-4">
          {([
            ['traditional', t('filebrowser.search.section.traditional', 'Traditional matches')],
            ['ai', t('filebrowser.search.section.ai', 'AI-enhanced matches')],
          ] as const).map(([key, label]) => {
            const list = grouped[key]
            if (!list.length) return null
            return (
              <div key={key}>
                <p className="shell-kicker mb-1.5 !text-[10px]">{label}</p>
                <div className="space-y-1.5">
                  {list.map(({ entry, reason, detail }) => {
                    const meta = reasonMeta[reason]
                    return (
                      <button
                        key={`${reason}-${entry.id}`}
                        type="button"
                        onClick={() => onSelect(entry)}
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
                          <div
                            className={clsx(
                              'mt-1 inline-flex items-center gap-1 text-[11px]',
                              meta?.tone,
                            )}
                          >
                            {meta?.icon}
                            {t(meta?.labelKey ?? '', meta?.fallback ?? reason)}
                            <span className="text-[color:var(--cp-muted)]"> · {detail}</span>
                          </div>
                        </div>
                      </button>
                    )
                  })}
                </div>
              </div>
            )
          })}
        </div>
      )}
    </div>
  )
}
