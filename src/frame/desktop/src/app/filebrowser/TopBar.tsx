import clsx from 'clsx'
import { IconButton, TextField, Tooltip } from '@mui/material'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ChevronRight,
  Copy,
  LayoutGrid,
  List,
  Plus,
  RefreshCw,
  Search,
  X,
} from 'lucide-react'
import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { BrowserTab, ViewMode } from './types'

interface TopBarProps {
  tabs: BrowserTab[]
  activeTabId: string
  onSelectTab: (id: string) => void
  onCloseTab: (id: string) => void
  onNewTab: () => void
  currentPath: string
  onNavigate: (path: string) => void
  onBack: () => void
  onForward: () => void
  onUp: () => void
  canBack: boolean
  canForward: boolean
  canUp: boolean
  viewMode: ViewMode
  onViewModeChange: (mode: ViewMode) => void
  searchQuery: string
  onSearchChange: (query: string) => void
  onCopyPath: () => void
}

function PathCrumbs({ path, onNavigate }: { path: string; onNavigate: (p: string) => void }) {
  const segments = path.split('/').filter(Boolean)
  const crumbs: { label: string; path: string }[] = [{ label: 'root', path: '/' }]
  let running = ''
  for (const segment of segments) {
    running += `/${segment}`
    crumbs.push({ label: segment, path: running })
  }

  return (
    <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto text-sm text-[color:var(--cp-muted)]">
      {crumbs.map((crumb, idx) => (
        <div key={crumb.path} className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            className={clsx(
              'truncate rounded-md px-1.5 py-0.5 hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)]',
              idx === crumbs.length - 1 && 'font-semibold text-[color:var(--cp-text)]',
            )}
            onClick={() => onNavigate(crumb.path)}
          >
            {crumb.label}
          </button>
          {idx < crumbs.length - 1 ? <ChevronRight size={12} className="opacity-60" /> : null}
        </div>
      ))}
    </div>
  )
}

export function TopBar({
  tabs,
  activeTabId,
  onSelectTab,
  onCloseTab,
  onNewTab,
  currentPath,
  onNavigate,
  onBack,
  onForward,
  onUp,
  canBack,
  canForward,
  canUp,
  viewMode,
  onViewModeChange,
  searchQuery,
  onSearchChange,
  onCopyPath,
}: TopBarProps) {
  const { t } = useI18n()
  const [searchOpen, setSearchOpen] = useState(false)
  const searchInputRef = useRef<HTMLInputElement | null>(null)

  useEffect(() => {
    if (searchQuery) setSearchOpen(true)
  }, [searchQuery])

  useEffect(() => {
    if (searchOpen) searchInputRef.current?.focus()
  }, [searchOpen])

  const closeSearch = () => {
    onSearchChange('')
    setSearchOpen(false)
  }

  return (
    <div className="flex flex-col gap-2 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_88%,transparent)] px-3 py-2 sm:px-4">
      {/* Tabs row */}
      <div className="flex items-center gap-1 overflow-x-auto">
        {tabs.map((tab) => {
          const active = tab.id === activeTabId
          return (
            <div
              key={tab.id}
              className={clsx(
                'group flex shrink-0 items-center gap-2 rounded-t-[12px] border-b-2 px-3 py-1.5 text-sm',
                active
                  ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,var(--cp-surface))] text-[color:var(--cp-text)]'
                  : 'border-transparent text-[color:var(--cp-muted)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_10%,transparent)]',
              )}
            >
              <button
                type="button"
                onClick={() => onSelectTab(tab.id)}
                className="max-w-[180px] truncate font-medium"
              >
                {tab.title}
              </button>
              {tabs.length > 1 ? (
                <button
                  type="button"
                  onClick={() => onCloseTab(tab.id)}
                  className="opacity-0 transition group-hover:opacity-100"
                  aria-label={t('common.close', 'Close')}
                >
                  <X size={12} />
                </button>
              ) : null}
            </div>
          )
        })}
        <Tooltip title={t('filebrowser.topbar.newTab', 'New tab')}>
          <IconButton size="small" onClick={onNewTab}>
            <Plus size={14} />
          </IconButton>
        </Tooltip>
      </div>

      {/* Nav row */}
      <div className="flex items-center gap-1.5">
        <IconButton size="small" disabled={!canBack} onClick={onBack} aria-label="back">
          <ArrowLeft size={16} />
        </IconButton>
        <IconButton size="small" disabled={!canForward} onClick={onForward} aria-label="forward">
          <ArrowRight size={16} />
        </IconButton>
        <IconButton size="small" disabled={!canUp} onClick={onUp} aria-label="up">
          <ArrowUp size={16} />
        </IconButton>
        <IconButton size="small" onClick={() => onNavigate(currentPath)} aria-label="refresh">
          <RefreshCw size={14} />
        </IconButton>

        <div className="relative ml-1 flex min-w-0 flex-1 items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] px-2 py-1">
          {searchOpen ? (
            <>
              <Search size={14} className="ml-1 shrink-0 text-[color:var(--cp-muted)]" />
              <TextField
                inputRef={searchInputRef}
                fullWidth
                variant="standard"
                placeholder={t(
                  'filebrowser.topbar.searchPlaceholder',
                  'Search across files, folders, AI summaries…',
                )}
                value={searchQuery}
                onChange={(event) => onSearchChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') closeSearch()
                }}
                InputProps={{ disableUnderline: true }}
                sx={{
                  '& .MuiInputBase-input': {
                    fontSize: 13,
                    padding: '2px 0',
                    color: 'var(--cp-text)',
                  },
                }}
              />
              <Tooltip title={t('common.close', 'Close')}>
                <IconButton size="small" onClick={closeSearch} aria-label="close-search">
                  <X size={13} />
                </IconButton>
              </Tooltip>
            </>
          ) : (
            <>
              <PathCrumbs path={currentPath} onNavigate={onNavigate} />
              <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
                <IconButton size="small" onClick={onCopyPath}>
                  <Copy size={13} />
                </IconButton>
              </Tooltip>
            </>
          )}
        </div>

        <div className="flex items-center rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_80%,transparent)]">
          <IconButton
            size="small"
            onClick={() => onViewModeChange('list')}
            className={clsx(
              viewMode === 'list' &&
                '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
            )}
            aria-label={t('filebrowser.view.list', 'List view')}
          >
            <List size={14} />
          </IconButton>
          <IconButton
            size="small"
            onClick={() => onViewModeChange('icon')}
            className={clsx(
              viewMode === 'icon' &&
                '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
            )}
            aria-label={t('filebrowser.view.icon', 'Icon view')}
          >
            <LayoutGrid size={14} />
          </IconButton>
        </div>

        <Tooltip title={t('filebrowser.topbar.search', 'Search')}>
          <IconButton
            size="small"
            onClick={() => setSearchOpen((v) => !v)}
            className={clsx(
              searchOpen &&
                '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
            )}
            aria-label={t('filebrowser.topbar.search', 'Search')}
          >
            <Search size={14} />
          </IconButton>
        </Tooltip>
      </div>
    </div>
  )
}
