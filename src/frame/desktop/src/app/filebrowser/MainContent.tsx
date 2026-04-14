import clsx from 'clsx'
import {
  Archive,
  Code,
  FileAudio,
  FileText,
  FileVideo,
  FolderClosed,
  Image as ImageIcon,
  Sparkles,
  Upload,
  Wand2,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import type { FileEntry, Topic, ViewMode } from './types'

function kindIcon(kind: FileEntry['kind'], size = 18) {
  switch (kind) {
    case 'folder':
      return <FolderClosed size={size} className="text-[color:var(--cp-accent)]" />
    case 'image':
      return <ImageIcon size={size} className="text-[color:var(--cp-success)]" />
    case 'video':
      return <FileVideo size={size} className="text-[color:var(--cp-warning)]" />
    case 'audio':
      return <FileAudio size={size} className="text-[color:var(--cp-warning)]" />
    case 'archive':
      return <Archive size={size} className="text-[color:var(--cp-muted)]" />
    case 'code':
      return <Code size={size} className="text-[color:var(--cp-accent)]" />
    default:
      return <FileText size={size} className="text-[color:var(--cp-muted)]" />
  }
}

function formatBytes(bytes?: number) {
  if (!bytes) return '—'
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} MB`
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`
}

function formatDate(iso: string) {
  const date = new Date(iso)
  return date.toLocaleString('en-CA', {
    year: 'numeric',
    month: '2-digit',
    day: '2-digit',
    hour: '2-digit',
    minute: '2-digit',
    hour12: false,
  })
}

interface MainContentProps {
  entries: FileEntry[]
  viewMode: ViewMode
  selectedId: string | null
  onSelect: (entry: FileEntry) => void
  onOpenFolder: (path: string) => void
  currentPath: string
  topicContext: Topic | null
}

export function MainContent({
  entries,
  viewMode,
  selectedId,
  onSelect,
  onOpenFolder,
  currentPath,
  topicContext,
}: MainContentProps) {
  const { t } = useI18n()

  const isPublic = currentPath === '/public' || currentPath.startsWith('/public/')

  if (!entries.length) {
    return (
      <div className="flex h-full w-full flex-col items-center justify-center p-10 text-center text-[color:var(--cp-muted)]">
        <FolderClosed size={40} className="opacity-50" />
        <p className="mt-3 font-display text-lg text-[color:var(--cp-text)]">
          {t('filebrowser.empty.title', 'This folder is empty')}
        </p>
        <p className="mt-1 max-w-sm text-sm leading-6">
          {t(
            'filebrowser.empty.body',
            'No files here yet. Upload from your device, drop files from chat, or wait for the next sync.',
          )}
        </p>
        <button
          type="button"
          className="mt-4 inline-flex items-center gap-2 rounded-full border border-[color:var(--cp-border)] px-4 py-1.5 text-sm text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
        >
          <Upload size={14} /> {t('filebrowser.actions.upload', 'Upload')}
        </button>
      </div>
    )
  }

  return (
    <div className="flex h-full w-full flex-col">
      {topicContext ? (
        <div className="flex items-center gap-3 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,var(--cp-surface))] px-4 py-2.5">
          <Sparkles size={16} className="text-[color:var(--cp-accent)]" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold text-[color:var(--cp-text)]">
              {t('filebrowser.topic.badge', 'Topic view')}: {topicContext.title}
            </div>
            <div className="mt-0.5 line-clamp-1 text-[11px] text-[color:var(--cp-muted)]">
              {topicContext.reason}
            </div>
          </div>
          <span className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface)_86%,transparent)] px-2 py-0.5 text-[11px] font-semibold text-[color:var(--cp-muted)]">
            {t('filebrowser.topic.aggregation', 'Aggregated · not copied')}
          </span>
        </div>
      ) : null}

      {viewMode === 'list' ? (
        <div className="flex-1 overflow-y-auto">
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] text-left text-[11px] uppercase tracking-wider text-[color:var(--cp-muted)] backdrop-blur">
              <tr>
                <th className="py-2 pl-4 pr-2 font-medium">{t('filebrowser.column.name', 'Name')}</th>
                <th className="px-2 py-2 font-medium">{t('filebrowser.column.kind', 'Kind')}</th>
                <th className="px-2 py-2 font-medium">{t('filebrowser.column.size', 'Size')}</th>
                <th className="px-2 py-2 font-medium">
                  {t('filebrowser.column.modified', 'Modified')}
                </th>
                <th className="px-2 py-2 font-medium">{t('filebrowser.column.tags', 'Tags')}</th>
                {isPublic ? (
                  <th className="px-2 py-2 font-medium">
                    {t('filebrowser.column.publicUrl', 'Public URL')}
                  </th>
                ) : null}
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => {
                const selected = entry.id === selectedId
                return (
                  <tr
                    key={entry.id}
                    className={clsx(
                      'cursor-pointer border-b border-[color:color-mix(in_srgb,var(--cp-border)_40%,transparent)] transition',
                      selected
                        ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))]'
                        : 'hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_10%,transparent)]',
                    )}
                    onClick={() => onSelect(entry)}
                    onDoubleClick={() => {
                      if (entry.kind === 'folder') onOpenFolder(entry.path)
                    }}
                  >
                    <td className="py-2 pl-4 pr-2">
                      <div className="flex items-center gap-2">
                        {kindIcon(entry.kind, 16)}
                        <span className="truncate font-medium text-[color:var(--cp-text)]">
                          {entry.name}
                        </span>
                        {entry.triggersActive ? (
                          <span
                            title="AI pipeline active"
                            className="inline-flex items-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))] px-1.5 py-0.5 text-[10px] font-semibold text-[color:var(--cp-accent)]"
                          >
                            <Wand2 size={10} className="mr-1" /> AI
                          </span>
                        ) : null}
                      </div>
                    </td>
                    <td className="px-2 py-2 capitalize text-[color:var(--cp-muted)]">
                      {entry.kind}
                    </td>
                    <td className="px-2 py-2 text-[color:var(--cp-muted)]">
                      {entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}
                    </td>
                    <td className="px-2 py-2 text-[color:var(--cp-muted)]">
                      {formatDate(entry.modifiedAt)}
                    </td>
                    <td className="px-2 py-2">
                      <div className="flex flex-wrap gap-1">
                        {entry.tags?.slice(0, 2).map((tag) => (
                          <span
                            key={tag}
                            className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_16%,var(--cp-surface))] px-2 py-0.5 text-[10px] text-[color:var(--cp-muted)]"
                          >
                            #{tag}
                          </span>
                        ))}
                      </div>
                    </td>
                    {isPublic ? (
                      <td className="truncate px-2 py-2 font-mono text-[10px] text-[color:var(--cp-accent)]">
                        {entry.publicUrl ?? '—'}
                      </td>
                    ) : null}
                  </tr>
                )
              })}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="flex-1 overflow-y-auto p-4">
          <div className="grid grid-cols-[repeat(auto-fill,minmax(160px,1fr))] gap-3">
            {entries.map((entry) => {
              const selected = entry.id === selectedId
              return (
                <button
                  key={entry.id}
                  type="button"
                  onClick={() => onSelect(entry)}
                  onDoubleClick={() => {
                    if (entry.kind === 'folder') onOpenFolder(entry.path)
                  }}
                  className={clsx(
                    'flex flex-col items-center gap-2 rounded-[18px] border border-transparent p-3 text-center transition',
                    selected
                      ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))]'
                      : 'hover:border-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_8%,transparent)]',
                  )}
                >
                  <div className="flex h-16 w-16 items-center justify-center rounded-[16px] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)]">
                    {kindIcon(entry.kind, 28)}
                  </div>
                  <span className="line-clamp-2 text-[12px] font-medium text-[color:var(--cp-text)]">
                    {entry.name}
                  </span>
                  <span className="text-[10px] text-[color:var(--cp-muted)]">
                    {entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}
                  </span>
                </button>
              )
            })}
          </div>
        </div>
      )}
    </div>
  )
}

export { formatBytes, formatDate, kindIcon }
