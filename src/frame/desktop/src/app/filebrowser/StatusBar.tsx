import { Copy, Link2, Wand2 } from 'lucide-react'
import { Tooltip } from '@mui/material'
import { useI18n } from '../../i18n/provider'
import { formatBytes } from './MainContent'
import type { FileEntry } from './types'

interface StatusBarProps {
  currentPath: string
  totalCount: number
  selection: FileEntry | null
  onCopy: (text: string) => void
}

export function StatusBar({ currentPath, totalCount, selection, onCopy }: StatusBarProps) {
  const { t } = useI18n()

  return (
    <div className="flex flex-wrap items-center gap-3 border-t border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_90%,transparent)] px-3 py-1.5 text-[11px] text-[color:var(--cp-muted)]">
      <span>
        {t('filebrowser.status.items', '{{count}} items', { count: totalCount })}
      </span>
      {selection ? (
        <>
          <span className="opacity-60">·</span>
          <span className="truncate">
            {t('filebrowser.status.selected', 'Selected')}: {selection.name}
          </span>
          <span className="opacity-60">·</span>
          <span>
            {selection.kind === 'folder' ? '—' : formatBytes(selection.sizeBytes)}
          </span>
          <span className="opacity-60">·</span>
          <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
            <button
              type="button"
              onClick={() => onCopy(selection.path)}
              className="inline-flex items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-2 py-0.5 font-mono text-[10px] hover:border-[color:var(--cp-accent)] hover:text-[color:var(--cp-accent)]"
            >
              <Copy size={10} /> {selection.path}
            </button>
          </Tooltip>
          {selection.publicUrl ? (
            <>
              <span className="opacity-60">·</span>
              <Tooltip title={t('filebrowser.preview.publicUrl', 'Public URL')}>
                <button
                  type="button"
                  onClick={() => onCopy(selection.publicUrl!)}
                  className="inline-flex items-center gap-1 rounded-full border border-[color:var(--cp-accent)] px-2 py-0.5 font-mono text-[10px] text-[color:var(--cp-accent)]"
                >
                  <Link2 size={10} /> URL
                </button>
              </Tooltip>
            </>
          ) : null}
          {selection.triggersActive ? (
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
          <span className="truncate font-mono text-[10px]">{currentPath}</span>
        </>
      )}
    </div>
  )
}
