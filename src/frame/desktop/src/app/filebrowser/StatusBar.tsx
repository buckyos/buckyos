import { Copy, CornerUpRight, Link2, PanelRightOpen, Wand2 } from 'lucide-react'
import { Tooltip } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { formatBytes } from './fileDisplay'
import type { FileItem } from './data/FolderReader'
import { displayPath } from './data/urls'

interface StatusBarProps {
  /** Canonical location url. */
  currentUrl: string
  /** Undefined while the first page is in flight — renders as "…". */
  totalCount: number | undefined
  /** Captured selection — details for one item, a count for many. */
  selectedItems: FileItem[]
  onCopy: (text: string) => void
  /** When set, shows an expand-sidebar button at the right edge (collapsed sidebar state). */
  onExpandSidebar?: () => void
}

export function StatusBar({
  currentUrl,
  totalCount,
  selectedItems,
  onCopy,
  onExpandSidebar,
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
    <div className="flex flex-wrap items-center gap-3 border-t border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_90%,transparent)] px-3 py-1.5 text-[11px] text-[color:var(--cp-muted)]">
      <span>
        {totalCount === undefined
          ? t('filebrowser.status.itemsUnknown', '… items')
          : t('filebrowser.status.items', '{{count}} items', { count: totalCount })}
      </span>
      {multiSelection ? (
        <>
          <span className="opacity-60">·</span>
          <span className="font-semibold text-[color:var(--cp-text)]">
            {totalCount !== undefined && multiSelection.length < totalCount
              ? t('filebrowser.status.selectedOf', 'selected {{count}} of {{total}}', {
                  count: multiSelection.length,
                  total: totalCount,
                })
              : t('filebrowser.status.selectedCount', '{{count}} selected', {
                  count: multiSelection.length,
                })}
          </span>
          {multiBytes > 0 ? (
            <>
              <span className="opacity-60">·</span>
              <span>{formatBytes(multiBytes)}</span>
            </>
          ) : null}
        </>
      ) : selection && entry ? (
        <>
          <span className="opacity-60">·</span>
          <span className="truncate">
            {t('filebrowser.status.selected', 'Selected')}: {entry.name}
          </span>
          <span className="opacity-60">·</span>
          <span>{entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}</span>
          <span className="opacity-60">·</span>
          <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
            <button
              type="button"
              onClick={() => onCopy(entry.path)}
              className="inline-flex items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-2 py-0.5 font-mono text-[10px] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)]"
            >
              <Copy size={10} /> {displayPath(entry.path)}
            </button>
          </Tooltip>
          {originalHint ? (
            <Tooltip title={t('filebrowser.status.originalPath', 'Original location')}>
              <button
                type="button"
                onClick={() => onCopy(originalHint)}
                className="inline-flex items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-2 py-0.5 font-mono text-[10px] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)]"
              >
                <CornerUpRight size={10} /> {originalHint}
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
                  className="inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-accent)] px-2 py-0.5 font-mono text-[10px] text-[color:var(--cp-accent)]"
                >
                  <Link2 size={10} /> URL
                </button>
              </Tooltip>
            </>
          ) : null}
          {entry.triggersActive ? (
            <>
              <span className="opacity-60">·</span>
              <span className="inline-flex items-center gap-1 text-[color:var(--cp-accent)]">
                <Wand2 size={10} /> {t('filebrowser.status.aiActive', 'AI pipeline active')}
              </span>
            </>
          ) : null}
        </>
      ) : (
        <>
          <span className="opacity-60">·</span>
          <span className="truncate font-mono text-[10px]">{displayPath(currentUrl)}</span>
        </>
      )}
      {onExpandSidebar ? (
        <Tooltip title={t('filebrowser.preview.expand', 'Expand sidebar')}>
          <button
            type="button"
            onClick={onExpandSidebar}
            className="ml-auto inline-flex shrink-0 items-center rounded-md p-0.5 hover:text-[color:var(--cp-text)]"
            aria-label={t('filebrowser.preview.expand', 'Expand sidebar')}
          >
            <PanelRightOpen size={13} />
          </button>
        </Tooltip>
      ) : null}
    </div>
  )
}
