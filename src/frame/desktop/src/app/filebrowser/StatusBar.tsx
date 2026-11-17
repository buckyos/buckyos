import { Copy, CornerUpRight, Link2, PanelRightOpen } from 'lucide-react'
import { Tooltip } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { formatBytes } from './fileDisplay'
import type { FileItem } from './data/FolderReader'
import { copyPathText, displayPath } from './data/urls'

interface StatusBarProps {
  /** Canonical location url. */
  currentUrl: string
  /** Undefined while the first page is in flight or for cursor-backed listings. */
  totalCount: number | undefined
  /** Loaded window size — shown as "N+" when the total is unknown (§5.1). */
  loadedCount?: number
  hasMore?: boolean
  /** Captured selection — details for one item, a count for many. */
  selectedItems: FileItem[]
  onCopy: (text: string) => void
  /** When set, shows an expand-sidebar button at the right edge (collapsed sidebar state). */
  onExpandSidebar?: () => void
  /** Quiet extras at the right edge (the copy-task list). */
  trailing?: React.ReactNode
}

export function StatusBar({
  currentUrl,
  totalCount,
  loadedCount,
  hasMore,
  selectedItems,
  onCopy,
  onExpandSidebar,
  trailing,
}: StatusBarProps) {
  const { t } = useI18n()

  const selection = selectedItems.length === 1 ? selectedItems[0] : null
  const multiSelection = selectedItems.length > 1 ? selectedItems : null
  const multiBytes = multiSelection
    ? multiSelection.reduce((sum, item) => sum + (item.entry.sizeBytes ?? 0), 0)
    : 0
  const entry = selection?.entry ?? null
  /** Reference items surface their original location too. */
  const originalHint = selection
    ? selection.entry.link
      ? displayPath(selection.entry.link.targetUrl)
      : selection.ref && selection.ref.refPath !== selection.entry.path
        ? selection.entry.path
        : null
    : null

  return (
    <div className="flex h-8 min-w-0 shrink-0 items-center gap-2 overflow-hidden border-t border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] pl-4 pr-2 text-[12px] tabular-nums text-[color:var(--cp-muted)]">
      <span className="shrink-0">
        {totalCount !== undefined
          ? t('filebrowser.status.items', '{{count}} items', { count: totalCount })
          : loadedCount
            ? hasMore
              ? t('filebrowser.status.itemsLoaded', '{{count}}+ items', {
                  count: loadedCount,
                })
              : t('filebrowser.status.items', '{{count}} items', { count: loadedCount })
            : t('filebrowser.status.itemsUnknown', '… items')}
      </span>
      {multiSelection ? (
        <>
          <span className="opacity-60">·</span>
          <span className="shrink-0 font-medium text-[color:var(--cp-text)]">
            {totalCount !== undefined && multiSelection.length < totalCount
              ? t('filebrowser.status.selectedOf', 'selected {{count}} of {{total}}', {
                  count: multiSelection.length,
                  total: totalCount,
                })
              : t('filebrowser.status.selectedCount', '{{count}} selected', {
                  count: multiSelection.length,
                })}
          </span>
          {multiSelection.length > 0 ? (
            <>
              <span className="opacity-60">·</span>
              <span className="min-w-0 truncate">{formatBytes(multiBytes)} · {t('filebrowser.meta.knownSize', 'Known size ({{count}}/{{total}} items)', { count: multiSelection.filter((item) => item.entry.sizeBytes !== undefined).length, total: multiSelection.length })}</span>
            </>
          ) : null}
        </>
      ) : selection && entry ? (
        <>
          <span className="opacity-60">·</span>
          <span className="min-w-0 truncate">
            {t('filebrowser.status.selected', 'Selected')}: {entry.name}
          </span>
          <span className="opacity-60">·</span>
          <span className="shrink-0">{entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}</span>
          <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
            <button
              type="button"
              onClick={() => onCopy(copyPathText(entry.path))}
              className="hidden"
            >
              <Copy size={14} /> {displayPath(entry.path)}
            </button>
          </Tooltip>
          {originalHint ? (
            <Tooltip title={t('filebrowser.status.originalPath', 'Original location')}>
              <button
                type="button"
                onClick={() => onCopy(copyPathText(originalHint))}
                className="hidden"
              >
                <CornerUpRight size={14} /> {originalHint}
              </button>
            </Tooltip>
          ) : null}
          {entry.publicUrl ? (
            <>
              <span className="opacity-60">·</span>
              <Tooltip title={t('filebrowser.preview.publicUrl', 'Public URL')}>
                <button
                  type="button"
                  onClick={() => onCopy(entry.publicUrl!)}
                  className="inline-flex h-6 shrink-0 items-center gap-1 rounded-[6px] px-1.5 text-[color:var(--cp-text)] hover:bg-[color:var(--files-hover)]"
                >
                  <Link2 size={14} /> URL
                </button>
              </Tooltip>
            </>
          ) : null}
          {entry.triggersActive ? (
            <>
              <span className="opacity-60">·</span>
              <span className="inline-flex shrink-0 items-center gap-1.5">
                <span className="files-dot" aria-hidden /> {t('filebrowser.status.aiActive', 'AI pipeline active')}
              </span>
            </>
          ) : null}
        </>
      ) : (
        <>
          <span className="opacity-60">·</span>
          <span className="min-w-0 truncate font-[family-name:var(--files-mono)]">{displayPath(currentUrl)}</span>
        </>
      )}
      <span className="ml-auto" />
      {trailing}
      {onExpandSidebar ? (
        <Tooltip title={t('filebrowser.preview.expand', 'Expand sidebar')}>
          <button
            type="button"
            onClick={onExpandSidebar}
            className="files-icon-btn !h-6 !min-w-6 !p-0"
            aria-label={t('filebrowser.preview.expand', 'Expand sidebar')}
          >
            <PanelRightOpen size={14} />
          </button>
        </Tooltip>
      ) : null}
    </div>
  )
}
