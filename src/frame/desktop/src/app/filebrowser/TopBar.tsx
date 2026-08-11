import clsx from 'clsx'
import {
  Divider,
  IconButton,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  TextField,
  Tooltip,
} from '@mui/material'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ArrowUpDown,
  ChevronDown,
  ChevronRight,
  ClipboardPaste,
  Copy,
  FilePlus,
  FolderInput,
  FolderPlus,
  LayoutGrid,
  List,
  PenLine,
  Plus,
  Scissors,
  Search,
  SearchX,
  Settings,
  Trash2,
  Upload,
  X,
} from 'lucide-react'
import type { MouseEvent as ReactMouseEvent, ReactNode } from 'react'
import { useEffect, useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { LocationCapabilities } from './data/FolderReader'
import { crumbsForUrl, displayPath } from './data/urls'
import type { BrowserTab, SortDir, SortKey, ViewMode } from './types'

interface TopBarProps {
  tabs: BrowserTab[]
  activeTabId: string
  onSelectTab: (id: string) => void
  onCloseTab: (id: string) => void
  onNewTab?: () => void
  closedTabs?: BrowserTab[]
  onRestoreClosedTab?: (tab: BrowserTab) => void
  /** Show the new-tab button and the closed-tabs menu (hidden in the right pane). */
  showTabControls?: boolean
  /** Allow closing the last remaining tab (the right pane disappears when empty). */
  allowCloseLast?: boolean
  /** When provided, tabs get a context menu with a "Send to Right" action. */
  onSendTabToRight?: (id: string) => void
  canSendToRight?: boolean
  /** Canonical location url of the pane. */
  currentPath: string
  /** Title for non-dfs locations (drives the breadcrumb root label). */
  locationTitle?: string
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
  onUpload?: () => void
  onNewFolder?: () => void
  onNewFile?: () => void
  /** Number of selected entries in the pane — drives the clipboard ops. */
  selectedCount?: number
  /** Location capabilities — every toolbar affordance trims itself by these. */
  capabilities?: LocationCapabilities
  canPaste?: boolean
  onCut?: () => void
  onCopy?: () => void
  onPaste?: () => void
  onRename?: () => void
  onDelete?: () => void
  onSettings?: () => void
  moveTargets?: { label: string; path: string }[]
  onMoveTo?: (path: string) => void
  sortKey?: SortKey
  sortDir?: SortDir
  onSortChange?: (key: SortKey, dir: SortDir) => void
}

/** Icon-only toolbar button with a tooltip (span keeps the tooltip alive when disabled). */
function ToolbarIconButton({
  title,
  disabled,
  active,
  onClick,
  children,
}: {
  title: string
  disabled?: boolean
  active?: boolean
  onClick?: (event: ReactMouseEvent<HTMLButtonElement>) => void
  children: ReactNode
}) {
  return (
    <Tooltip title={title}>
      <span className="inline-flex">
        <button
          type="button"
          disabled={disabled}
          onClick={onClick}
          aria-label={title}
          className={clsx(
            'flex h-7 min-w-7 shrink-0 items-center justify-center rounded-md px-1.5 text-[color:var(--cp-muted)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,transparent)] hover:text-[color:var(--cp-text)] disabled:pointer-events-none disabled:opacity-40',
            active &&
              'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] text-[color:var(--cp-text)]',
          )}
        >
          {children}
        </button>
      </span>
    </Tooltip>
  )
}

function ToolbarDivider() {
  return (
    <div className="mx-1 h-4 w-px shrink-0 bg-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)]" />
  )
}

function PathCrumbs({
  url,
  rootLabel,
  onNavigate,
}: {
  url: string
  /** Title for non-dfs roots (collection/view name from reader meta). */
  rootLabel?: string
  onNavigate: (url: string) => void
}) {
  const crumbs = crumbsForUrl(url, rootLabel)

  return (
    <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto text-sm text-[color:var(--cp-muted)]">
      {crumbs.map((crumb, idx) => (
        <div key={crumb.url} className="flex shrink-0 items-center gap-1">
          <button
            type="button"
            className={clsx(
              'truncate rounded-md px-1.5 py-0.5 hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,transparent)]',
              idx === crumbs.length - 1 && 'font-semibold text-[color:var(--cp-text)]',
            )}
            onClick={(event) => {
              event.stopPropagation()
              onNavigate(crumb.url)
            }}
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
  closedTabs = [],
  onRestoreClosedTab,
  showTabControls = true,
  allowCloseLast = false,
  onSendTabToRight,
  canSendToRight = false,
  currentPath,
  locationTitle,
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
  onUpload,
  onNewFolder,
  onNewFile,
  selectedCount = 0,
  capabilities,
  canPaste = false,
  onCut,
  onCopy,
  onPaste,
  onRename,
  onDelete,
  onSettings,
  moveTargets = [],
  onMoveTo,
  sortKey = 'name',
  sortDir = 'asc',
  onSortChange,
}: TopBarProps) {
  const { t } = useI18n()
  const [searchOpen, setSearchOpen] = useState(false)
  const [tabMenuAnchor, setTabMenuAnchor] = useState<HTMLElement | null>(null)
  const [newMenuAnchor, setNewMenuAnchor] = useState<HTMLElement | null>(null)
  const [moveMenuAnchor, setMoveMenuAnchor] = useState<HTMLElement | null>(null)
  const [viewMenuAnchor, setViewMenuAnchor] = useState<HTMLElement | null>(null)
  const [sortMenuAnchor, setSortMenuAnchor] = useState<HTMLElement | null>(null)
  const [tabContextMenu, setTabContextMenu] = useState<
    { tabId: string; top: number; left: number } | null
  >(null)
  const [pathEditing, setPathEditing] = useState(false)
  const [pathDraft, setPathDraft] = useState(currentPath)
  const searchInputRef = useRef<HTMLInputElement | null>(null)
  const pathInputRef = useRef<HTMLInputElement | null>(null)
  const searchVisible = searchOpen || !!searchQuery
  const canCloseTab = tabs.length > 1 || allowCloseLast
  const sortLabels: Record<SortKey, string> = {
    manual: t('filebrowser.sort.manual', 'Custom order'),
    name: t('filebrowser.column.name', 'Name'),
    size: t('filebrowser.column.size', 'Size'),
    modified: t('filebrowser.column.modified', 'Modified'),
    kind: t('filebrowser.column.kind', 'Kind'),
  }
  // Capability-derived affordances (default to plain-folder behaviour).
  const acceptsContent = capabilities?.acceptsContent ?? true
  const acceptsReferences = capabilities?.acceptsReferences ?? false
  const removal = capabilities ? capabilities.removal : 'destroy'
  const sortKeys = capabilities?.sortKeys ?? (['name', 'size', 'modified', 'kind'] as SortKey[])
  const pasteEnabled = canPaste && (acceptsContent || acceptsReferences)
  const deleteLabel =
    removal === 'remove-ref'
      ? t('filebrowser.actions.removeFromCollection', 'Remove from collection')
      : t('filebrowser.actions.delete', 'Delete')

  useEffect(() => {
    if (searchVisible) searchInputRef.current?.focus()
  }, [searchVisible])

  useEffect(() => {
    if (pathEditing) {
      pathInputRef.current?.focus()
      pathInputRef.current?.select()
    }
  }, [pathEditing])

  const startPathEdit = () => {
    if (searchVisible || pathEditing) return
    setPathDraft(displayPath(currentPath))
    setPathEditing(true)
  }

  const commitPathEdit = () => {
    const next = pathDraft.trim()
    setPathEditing(false)
    if (next && next !== displayPath(currentPath)) onNavigate(next)
  }

  const closeSearch = () => {
    onSearchChange('')
    setSearchOpen(false)
  }

  const closeTabMenu = () => {
    setTabMenuAnchor(null)
  }

  const handleNewTab = () => {
    closeTabMenu()
    onNewTab?.()
  }

  const closeTabContextMenu = () => {
    setTabContextMenu(null)
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
              onContextMenu={(event) => {
                if (!onSendTabToRight) return
                event.preventDefault()
                setTabContextMenu({ tabId: tab.id, top: event.clientY, left: event.clientX })
              }}
            >
              <button
                type="button"
                onClick={() => onSelectTab(tab.id)}
                className="max-w-[180px] truncate font-medium"
              >
                {tab.title}
              </button>
              {canCloseTab ? (
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
        {showTabControls ? (
        <>
        <Tooltip title={t('filebrowser.topbar.newTab', 'New tab')}>
          <button
            type="button"
            onClick={onNewTab}
            className="flex h-7 shrink-0 items-center gap-0.5 rounded-md px-1.5 text-[color:var(--cp-muted)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,transparent)] hover:text-[color:var(--cp-text)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--cp-accent)]"
            aria-label={t('filebrowser.topbar.newTab', 'New tab')}
          >
            <Plus size={14} />
          </button>
        </Tooltip>
        <Tooltip title={t('filebrowser.topbar.closedTabs', 'Recently closed tabs')}>
          <button
            type="button"
            onClick={(event) => setTabMenuAnchor(event.currentTarget)}
            className={clsx(
              'flex h-7 shrink-0 items-center rounded-md px-1 text-[color:var(--cp-muted)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,transparent)] hover:text-[color:var(--cp-text)] focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[color:var(--cp-accent)]',
              tabMenuAnchor && 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,transparent)] text-[color:var(--cp-text)]',
            )}
            aria-label={t('filebrowser.topbar.closedTabs', 'Recently closed tabs')}
            aria-haspopup="menu"
            aria-expanded={Boolean(tabMenuAnchor)}
          >
            <ChevronDown size={14} />
          </button>
        </Tooltip>
        <Menu
          anchorEl={tabMenuAnchor}
          open={Boolean(tabMenuAnchor)}
          onClose={closeTabMenu}
          slotProps={{ paper: { sx: { minWidth: 220, maxWidth: 320 } } }}
        >
          <MenuItem onClick={handleNewTab}>
            <ListItemText primary={t('filebrowser.topbar.newTab', 'New tab')} />
          </MenuItem>
          {onSendTabToRight ? (
            <MenuItem
              disabled={!canSendToRight}
              onClick={() => {
                closeTabMenu()
                onSendTabToRight(activeTabId)
              }}
            >
              <ListItemText primary={t('filebrowser.topbar.sendToRight', 'Send to Right')} />
            </MenuItem>
          ) : null}
          <Divider />
          {closedTabs.length > 0 ? (
            closedTabs.map((tab) => (
              <MenuItem
                key={tab.id}
                onClick={() => {
                  closeTabMenu()
                  onRestoreClosedTab?.(tab)
                }}
              >
                <ListItemText
                  primary={tab.title}
                  secondary={tab.path}
                  primaryTypographyProps={{ noWrap: true, fontSize: 13 }}
                  secondaryTypographyProps={{ noWrap: true, fontSize: 12 }}
                />
              </MenuItem>
            ))
          ) : (
            <MenuItem disabled>
              <ListItemText primary={t('filebrowser.topbar.noClosedTabs', 'No recently closed tabs')} />
            </MenuItem>
          )}
        </Menu>
        </>
        ) : null}
        {onSendTabToRight ? (
          <Menu
            open={Boolean(tabContextMenu)}
            onClose={closeTabContextMenu}
            anchorReference="anchorPosition"
            anchorPosition={
              tabContextMenu
                ? { top: tabContextMenu.top, left: tabContextMenu.left }
                : undefined
            }
            slotProps={{ paper: { sx: { minWidth: 180 } } }}
          >
            <MenuItem
              disabled={!canSendToRight}
              onClick={() => {
                const tabId = tabContextMenu?.tabId
                closeTabContextMenu()
                if (tabId) onSendTabToRight(tabId)
              }}
            >
              <ListItemText primary={t('filebrowser.topbar.sendToRight', 'Send to Right')} />
            </MenuItem>
            <MenuItem
              disabled={!canCloseTab}
              onClick={() => {
                const tabId = tabContextMenu?.tabId
                closeTabContextMenu()
                if (tabId) onCloseTab(tabId)
              }}
            >
              <ListItemText primary={t('filebrowser.topbar.closeTab', 'Close tab')} />
            </MenuItem>
          </Menu>
        ) : null}
      </div>

      {/* Address row: history navigation + path / search input */}
      <div className="flex items-center gap-1.5">
        <div className="flex h-8 shrink-0 items-stretch overflow-hidden rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_80%,transparent)]">
          <IconButton
            size="small"
            disabled={!canBack}
            onClick={onBack}
            aria-label="back"
            className="!h-auto !w-9 !rounded-none !p-0"
          >
            <ArrowLeft size={16} />
          </IconButton>
          <IconButton
            size="small"
            disabled={!canForward}
            onClick={onForward}
            aria-label="forward"
            className="!h-auto !w-9 !rounded-none !p-0"
          >
            <ArrowRight size={16} />
          </IconButton>
        </div>
        <IconButton size="small" disabled={!canUp} onClick={onUp} aria-label="up">
          <ArrowUp size={16} />
        </IconButton>

        <div
          className={clsx(
            'relative ml-1 flex h-8 min-w-0 flex-1 items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] pl-2 pr-1.5',
            !searchVisible && !pathEditing && 'cursor-text',
          )}
          onClick={startPathEdit}
        >
          {searchVisible ? (
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
          ) : pathEditing ? (
            <input
              ref={pathInputRef}
              value={pathDraft}
              onChange={(event) => setPathDraft(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') commitPathEdit()
                if (event.key === 'Escape') setPathEditing(false)
              }}
              onBlur={() => setPathEditing(false)}
              spellCheck={false}
              aria-label="path"
              className="min-w-0 flex-1 bg-transparent px-1 font-mono text-[13px] outline-none"
              style={{ color: 'var(--cp-text)' }}
            />
          ) : (
            <>
              <PathCrumbs url={currentPath} rootLabel={locationTitle} onNavigate={onNavigate} />
              <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
                <button
                  type="button"
                  aria-label={t('filebrowser.topbar.copyPath', 'Copy path')}
                  onClick={(event) => {
                    event.stopPropagation()
                    onCopyPath()
                  }}
                  className="ml-auto flex shrink-0 items-center self-stretch pl-2 pr-1 text-[color:var(--cp-muted)] transition hover:text-[color:var(--cp-text)]"
                >
                  <Copy size={13} />
                </button>
              </Tooltip>
            </>
          )}
        </div>
      </div>

      {/* Toolbar row: upload / new | clipboard ops | settings ··· view mode / sort / search */}
      <div className="flex items-center gap-0.5 overflow-x-auto">
        {onUpload ? (
          <button
            type="button"
            onClick={onUpload}
            disabled={!acceptsContent}
            className="flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2 text-xs text-[color:var(--cp-muted)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,transparent)] hover:text-[color:var(--cp-text)] disabled:pointer-events-none disabled:opacity-40"
          >
            <Upload size={14} />
            {t('filebrowser.actions.upload', 'Upload')}
          </button>
        ) : null}
        {onNewFolder || onNewFile ? (
          <>
            <ToolbarIconButton
              title={t('filebrowser.actions.new', 'New')}
              disabled={!acceptsContent}
              active={Boolean(newMenuAnchor)}
              onClick={(event) => setNewMenuAnchor(event.currentTarget)}
            >
              <span className="flex items-center">
                <Plus size={14} />
                <ChevronDown size={10} />
              </span>
            </ToolbarIconButton>
            <Menu
              anchorEl={newMenuAnchor}
              open={Boolean(newMenuAnchor)}
              onClose={() => setNewMenuAnchor(null)}
              slotProps={{ paper: { sx: { minWidth: 180 } } }}
            >
              {onNewFolder ? (
                <MenuItem
                  onClick={() => {
                    setNewMenuAnchor(null)
                    onNewFolder()
                  }}
                >
                  <ListItemIcon>
                    <FolderPlus size={15} />
                  </ListItemIcon>
                  <ListItemText primary={t('filebrowser.actions.newFolder', 'New folder')} />
                </MenuItem>
              ) : null}
              {onNewFile ? (
                <MenuItem
                  onClick={() => {
                    setNewMenuAnchor(null)
                    onNewFile()
                  }}
                >
                  <ListItemIcon>
                    <FilePlus size={15} />
                  </ListItemIcon>
                  <ListItemText primary={t('filebrowser.actions.newTextFile', 'New text file')} />
                </MenuItem>
              ) : null}
            </Menu>
          </>
        ) : null}

        <ToolbarDivider />

        <ToolbarIconButton
          title={t('filebrowser.actions.cut', 'Cut')}
          disabled={!acceptsContent || !selectedCount || !onCut}
          onClick={onCut}
        >
          <Scissors size={14} />
        </ToolbarIconButton>
        <ToolbarIconButton
          title={t('filebrowser.actions.copy', 'Copy')}
          disabled={!selectedCount || !onCopy}
          onClick={onCopy}
        >
          <Copy size={14} />
        </ToolbarIconButton>
        <ToolbarIconButton
          title={
            acceptsReferences
              ? t('filebrowser.actions.pasteAsRef', 'Paste as references')
              : t('filebrowser.actions.paste', 'Paste')
          }
          disabled={!pasteEnabled || !onPaste}
          onClick={onPaste}
        >
          <ClipboardPaste size={14} />
        </ToolbarIconButton>
        <ToolbarIconButton
          title={t('filebrowser.actions.rename', 'Rename')}
          disabled={!acceptsContent || selectedCount !== 1 || !onRename}
          onClick={onRename}
        >
          <PenLine size={14} />
        </ToolbarIconButton>
        <ToolbarIconButton
          title={t('filebrowser.actions.moveTo', 'Move to')}
          disabled={!acceptsContent || !selectedCount || !moveTargets.length || !onMoveTo}
          active={Boolean(moveMenuAnchor)}
          onClick={(event) => setMoveMenuAnchor(event.currentTarget)}
        >
          <FolderInput size={14} />
        </ToolbarIconButton>
        <Menu
          anchorEl={moveMenuAnchor}
          open={Boolean(moveMenuAnchor)}
          onClose={() => setMoveMenuAnchor(null)}
          slotProps={{ paper: { sx: { minWidth: 200, maxHeight: 320 } } }}
        >
          {moveTargets.map((target) => (
            <MenuItem
              key={target.path}
              onClick={() => {
                setMoveMenuAnchor(null)
                onMoveTo?.(target.path)
              }}
            >
              <ListItemText primary={target.label} primaryTypographyProps={{ noWrap: true, fontSize: 13 }} />
            </MenuItem>
          ))}
        </Menu>
        {removal !== null ? (
          <ToolbarIconButton
            title={deleteLabel}
            disabled={!selectedCount || !onDelete}
            onClick={onDelete}
          >
            <Trash2 size={14} />
          </ToolbarIconButton>
        ) : null}

        <ToolbarDivider />

        <ToolbarIconButton
          title={t('filebrowser.actions.settings', 'Settings')}
          disabled={!onSettings}
          onClick={onSettings}
        >
          <Settings size={14} />
        </ToolbarIconButton>

        <div className="min-w-2 flex-1" />

        <ToolbarIconButton
          title={t('filebrowser.topbar.viewMode', 'View mode')}
          active={Boolean(viewMenuAnchor)}
          onClick={(event) => setViewMenuAnchor(event.currentTarget)}
        >
          <span className="flex items-center">
            {viewMode === 'list' ? <List size={14} /> : <LayoutGrid size={14} />}
            <ChevronDown size={10} />
          </span>
        </ToolbarIconButton>
        <Menu
          anchorEl={viewMenuAnchor}
          open={Boolean(viewMenuAnchor)}
          onClose={() => setViewMenuAnchor(null)}
          slotProps={{ paper: { sx: { minWidth: 160 } } }}
        >
          <MenuItem
            selected={viewMode === 'list'}
            onClick={() => {
              setViewMenuAnchor(null)
              onViewModeChange('list')
            }}
          >
            <ListItemIcon>
              <List size={15} />
            </ListItemIcon>
            <ListItemText primary={t('filebrowser.view.list', 'List view')} />
          </MenuItem>
          <MenuItem
            selected={viewMode === 'icon'}
            onClick={() => {
              setViewMenuAnchor(null)
              onViewModeChange('icon')
            }}
          >
            <ListItemIcon>
              <LayoutGrid size={15} />
            </ListItemIcon>
            <ListItemText primary={t('filebrowser.view.icon', 'Icon view')} />
          </MenuItem>
        </Menu>

        <button
          type="button"
          onClick={(event) => setSortMenuAnchor(event.currentTarget)}
          className={clsx(
            'flex h-7 shrink-0 items-center gap-1 rounded-md px-2 text-xs text-[color:var(--cp-muted)] transition hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_12%,transparent)] hover:text-[color:var(--cp-text)]',
            sortMenuAnchor &&
              'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] text-[color:var(--cp-text)]',
          )}
          aria-label={t('filebrowser.topbar.sort', 'Sort by')}
          aria-haspopup="menu"
          aria-expanded={Boolean(sortMenuAnchor)}
        >
          <ArrowUpDown size={13} />
          {sortLabels[sortKey]}
          <ChevronDown size={10} />
        </button>
        <Menu
          anchorEl={sortMenuAnchor}
          open={Boolean(sortMenuAnchor)}
          onClose={() => setSortMenuAnchor(null)}
          slotProps={{ paper: { sx: { minWidth: 160 } } }}
        >
          {sortKeys.map((key) => (
            <MenuItem
              key={key}
              selected={sortKey === key}
              onClick={() => {
                setSortMenuAnchor(null)
                onSortChange?.(key, sortDir)
              }}
            >
              <ListItemText primary={sortLabels[key]} />
            </MenuItem>
          ))}
          <Divider />
          {/* Direction is meaningless under manual order (always collection-defined). */}
          <MenuItem
            selected={sortDir === 'asc'}
            disabled={sortKey === 'manual'}
            onClick={() => {
              setSortMenuAnchor(null)
              onSortChange?.(sortKey, 'asc')
            }}
          >
            <ListItemText primary={t('filebrowser.sort.asc', 'Ascending')} />
          </MenuItem>
          <MenuItem
            selected={sortDir === 'desc'}
            disabled={sortKey === 'manual'}
            onClick={() => {
              setSortMenuAnchor(null)
              onSortChange?.(sortKey, 'desc')
            }}
          >
            <ListItemText primary={t('filebrowser.sort.desc', 'Descending')} />
          </MenuItem>
        </Menu>

        <ToolbarIconButton
          title={
            searchVisible
              ? t('common.close', 'Close')
              : t('filebrowser.topbar.search', 'Search')
          }
          onClick={() => (searchVisible ? closeSearch() : setSearchOpen(true))}
        >
          {searchVisible ? <SearchX size={14} /> : <Search size={14} />}
        </ToolbarIconButton>
      </div>
    </div>
  )
}
