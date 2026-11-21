import clsx from 'clsx'
import {
  Divider,
  ListItemIcon,
  ListItemText,
  Menu,
  MenuItem,
  Tooltip,
} from '@mui/material'
import {
  ArrowLeft,
  ArrowRight,
  ArrowUp,
  ArrowUpDown,
  Check,
  ChevronDown,
  ChevronRight,
  Copy,
  ClipboardPaste,
  FolderInput,
  FilePlus,
  FolderPlus,
  FolderUp,
  LayoutGrid,
  List,
  Plus,
  Search,
  SearchX,
  MoreHorizontal,
  PanelRightOpen,
  Menu as PlacesIcon,
  Upload,
  X,
} from 'lucide-react'
import type { MouseEvent as ReactMouseEvent, ReactNode } from 'react'
import { useEffect, useId, useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { LocationCapabilities } from './data/FolderReader'
import { crumbsForUrl, displayPath } from './data/urls'
import { ICON_SIZES, type IconSize } from './data/iconSize'
import { setBrowseArchives, useBrowseArchives } from './data/archiveBrowse'
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
  /** The computer's name: the root crumb of its folders. */
  computerLabel?: string
  onNavigate: (path: string) => void
  onBack: () => void
  onForward: () => void
  onUp: () => void
  canBack: boolean
  canForward: boolean
  canUp: boolean
  viewMode: ViewMode
  onViewModeChange: (mode: ViewMode) => void
  iconSize?: IconSize
  onIconSizeChange?: (size: IconSize) => void
  searchQuery: string
  onSearchChange: (query: string) => void
  onCopyPath: () => void
  onUpload?: () => void
  onNewFolder?: () => void
  onNewFile?: () => void
  selectedCount?: number
  capabilities?: LocationCapabilities
  onMore: (position: { top: number; left: number }) => void
  onDetails: () => void
  onPlaces: () => void
  onMoveTo: () => void
  onCopy?: () => void
  onPaste?: () => void
  onCopyTo?: () => void
  onAddExisting?: () => void
  onFolderUpload?: () => void
  pathEditSignal?: number
  listPreferences: { fullColumns: boolean; density: 'compact' | 'comfortable'; nameWidth: number }
  onListPreferences: (patch: Partial<TopBarProps['listPreferences']>) => void
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
          className={clsx('files-icon-btn', active && 'is-active')}
        >
          {children}
        </button>
      </span>
    </Tooltip>
  )
}

function PathCrumbs({
  url,
  rootLabel,
  computerLabel,
  onNavigate,
}: {
  url: string
  /** Title for non-dfs roots (collection/view name from reader meta). */
  rootLabel?: string
  computerLabel?: string
  onNavigate: (url: string) => void
}) {
  const crumbs = crumbsForUrl(url, rootLabel, computerLabel)

  return (
    <div className="flex min-w-0 flex-1 items-center gap-0.5 overflow-x-auto text-[13px] text-[color:var(--cp-muted)]">
      {crumbs.map((crumb, idx) => (
        <div key={crumb.url} className="flex shrink-0 items-center gap-0.5">
          <button
            type="button"
            className={clsx(
              'h-6 truncate rounded-[6px] px-1.5 transition-colors hover:bg-[color:var(--files-hover)] hover:text-[color:var(--cp-text)]',
              idx === crumbs.length - 1 && 'font-medium text-[color:var(--cp-text)]',
            )}
            onClick={(event) => {
              event.stopPropagation()
              onNavigate(crumb.url)
            }}
          >
            {crumb.label}
          </button>
          {idx < crumbs.length - 1 ? <ChevronRight size={14} className="shrink-0 opacity-70" /> : null}
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
  computerLabel,
  onNavigate,
  onBack,
  onForward,
  onUp,
  canBack,
  canForward,
  canUp,
  viewMode,
  iconSize,
  onIconSizeChange,
  onViewModeChange,
  searchQuery,
  onSearchChange,
  onCopyPath,
  onUpload,
  onNewFolder,
  onNewFile,
  selectedCount = 0,
  capabilities,
  onCopy, onPaste, onCopyTo, onMore, onDetails, onPlaces, onAddExisting, onFolderUpload, pathEditSignal, listPreferences, onListPreferences,
  sortKey = 'name',
  sortDir = 'asc',
  onSortChange,
}: TopBarProps) {
  const { t } = useI18n()
  const [searchOpen, setSearchOpen] = useState(false)
  const [tabMenuAnchor, setTabMenuAnchor] = useState<HTMLElement | null>(null)
  const [uploadMenuAnchor, setUploadMenuAnchor] = useState<HTMLElement | null>(null)
  const uploadMenuId = useId()
  const [newMenuAnchor, setNewMenuAnchor] = useState<HTMLElement | null>(null)
  const [viewMenuAnchor, setViewMenuAnchor] = useState<HTMLElement | null>(null)
  const browseArchives = useBrowseArchives()
  const [sortMenuAnchor, setSortMenuAnchor] = useState<HTMLElement | null>(null)
  const [tabContextMenu, setTabContextMenu] = useState<
    { tabId: string; top: number; left: number } | null
  >(null)
  const [pathEditing, setPathEditing] = useState(false)
  const [pathDraft, setPathDraft] = useState(currentPath)
  const searchInputRef = useRef<HTMLInputElement | null>(null)
  const pathInputRef = useRef<HTMLInputElement | null>(null)
  const searchVisible = !pathEditing && (searchOpen || !!searchQuery)
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
  const sortKeys = capabilities?.sortKeys ?? (['name', 'size', 'modified', 'kind'] as SortKey[])
  const sortDirs = capabilities?.sortDirs ?? (['asc', 'desc'] as SortDir[])

  const [previousContext, setPreviousContext] = useState({ activeTabId, currentPath, pathEditSignal })
  if (previousContext.activeTabId !== activeTabId || previousContext.currentPath !== currentPath || previousContext.pathEditSignal !== pathEditSignal) {
    setPreviousContext({ activeTabId, currentPath, pathEditSignal })
    setSearchOpen(false)
    setPathEditing(!!pathEditSignal && previousContext.pathEditSignal !== pathEditSignal)
    setPathDraft(displayPath(currentPath))
  }

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
    if (next) onNavigate(next)
    pathInputRef.current?.closest<HTMLElement>('[data-testid="filebrowser"]')?.focus()
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
    <div className="flex flex-col gap-2 border-b border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-3 py-2">
      {/* Tabs row */}
      <div className="flex items-center gap-1 overflow-x-auto" role="tablist">
        {tabs.map((tab) => {
          const active = tab.id === activeTabId
          return (
            <div
              key={tab.id}
              className={clsx(
                'group flex h-8 shrink-0 items-center gap-1 rounded-[8px] pl-3 pr-1.5 text-[13px] transition-colors',
                active
                  ? 'bg-[color:var(--files-pressed)] text-[color:var(--cp-text)]'
                  : 'text-[color:var(--cp-muted)] hover:bg-[color:var(--files-hover)] hover:text-[color:var(--cp-text)]',
              )}
              onContextMenu={(event) => {
                if (!onSendTabToRight) return
                event.preventDefault()
                setTabContextMenu({ tabId: tab.id, top: event.clientY, left: event.clientX })
              }}
            >
              <button
                type="button"
                role="tab"
                aria-selected={active}
                onClick={() => onSelectTab(tab.id)}
                className={clsx('max-w-[180px] truncate', active && 'font-medium')}
              >
                {tab.title}
              </button>
              {canCloseTab ? (
                <button
                  type="button"
                  onClick={() => onCloseTab(tab.id)}
                  className="flex size-5 items-center justify-center rounded-[4px] opacity-0 transition-opacity hover:bg-[color:var(--files-pressed)] focus-visible:opacity-100 group-hover:opacity-100"
                  aria-label={t('common.close', 'Close')}
                >
                  <X size={14} />
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
            className="files-icon-btn"
            aria-label={t('filebrowser.topbar.newTab', 'New tab')}
          >
            <Plus size={16} />
          </button>
        </Tooltip>
        <Tooltip title={t('filebrowser.topbar.closedTabs', 'Recently closed tabs')}>
          <button
            type="button"
            onClick={(event) => setTabMenuAnchor(event.currentTarget)}
            className="files-icon-btn"
            aria-label={t('filebrowser.topbar.closedTabs', 'Recently closed tabs')}
            aria-haspopup="menu"
            aria-expanded={Boolean(tabMenuAnchor)}
          >
            <ChevronDown size={16} />
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
      <div className="flex items-center gap-1">
        <button type="button" className="files-icon-btn" disabled={!canBack} onClick={onBack} aria-label={t('filebrowser.topbar.back')}>
          <ArrowLeft size={16} />
        </button>
        <button type="button" className="files-icon-btn" disabled={!canForward} onClick={onForward} aria-label={t('filebrowser.topbar.forward')}>
          <ArrowRight size={16} />
        </button>
        <button type="button" className="files-icon-btn" disabled={!canUp} onClick={onUp} aria-label={t('filebrowser.topbar.up')}>
          <ArrowUp size={16} />
        </button>

        <div
          className={clsx(
            'relative ml-1 flex h-8 min-w-0 flex-1 items-center gap-1 rounded-[8px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-2-opaque)] pl-1.5 pr-1 transition-colors',
            !searchVisible && !pathEditing && 'cursor-text',
            (searchVisible || pathEditing) && 'border-[color:var(--cp-accent)] bg-[color:var(--cp-surface-opaque)]',
          )}
          onClick={startPathEdit}
        >
          {searchVisible ? (
            <>
              <Search size={16} className="ml-0.5 shrink-0 text-[color:var(--cp-muted)]" />
              <input
                ref={searchInputRef}
                type="text"
                placeholder={t('filebrowser.topbar.searchPlaceholder', 'Search files')}
                value={searchQuery}
                onChange={(event) => onSearchChange(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Escape') closeSearch()
                }}
                className="min-w-0 flex-1 bg-transparent px-1 text-[13px] text-[color:var(--cp-text)] outline-none placeholder:text-[color:var(--cp-muted)] focus-visible:!outline-none"
              />
              <Tooltip title={t('common.close', 'Close')}>
                <button type="button" className="files-icon-btn !h-6 !min-w-6 !p-0" onClick={closeSearch} aria-label={t('messagehub.search.close', 'Close search')}>
                  <X size={14} />
                </button>
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
              aria-label={t('filebrowser.topbar.path')}
              className="min-w-0 flex-1 bg-transparent px-1.5 font-[family-name:var(--files-mono)] text-[13px] text-[color:var(--cp-text)] outline-none focus-visible:!outline-none"
            />
          ) : (
            <>
              <PathCrumbs url={currentPath} rootLabel={locationTitle} computerLabel={computerLabel} onNavigate={onNavigate} />
              <Tooltip title={t('filebrowser.topbar.copyPath', 'Copy path')}>
                <button
                  type="button"
                  aria-label={t('filebrowser.topbar.copyPath', 'Copy path')}
                  onClick={(event) => {
                    event.stopPropagation()
                    onCopyPath()
                  }}
                  className="files-icon-btn ml-auto !h-6 !min-w-6 !p-0"
                >
                  <Copy size={14} />
                </button>
              </Tooltip>
            </>
          )}
        </div>
      </div>

      {/* Toolbar row: upload / new | clipboard ops | settings ··· view mode / sort / search */}
      <div data-testid="files-toolbar" className="flex flex-wrap items-center gap-1">
        <ToolbarIconButton title={t('filebrowser.mobile.places', 'Places')} onClick={onPlaces}><PlacesIcon size={16} /></ToolbarIconButton>
        {onUpload || onFolderUpload ? (
          <>
            <button
              type="button"
              onClick={(event) => setUploadMenuAnchor(event.currentTarget)}
              disabled={!acceptsContent}
              aria-haspopup="menu"
              aria-expanded={Boolean(uploadMenuAnchor)}
              aria-controls={uploadMenuAnchor ? uploadMenuId : undefined}
              aria-label={t('filebrowser.actions.upload', 'Upload')}
              className="files-icon-btn gap-1.5 !px-2 text-[13px]"
            >
              <Upload size={16} />
              <span className="fb-toolbar-label">{t('filebrowser.actions.upload', 'Upload')}</span>
              <ChevronDown size={14} className="-ml-0.5 opacity-70" />
            </button>
            <Menu
              id={uploadMenuId}
              anchorEl={uploadMenuAnchor}
              open={Boolean(uploadMenuAnchor)}
              onClose={() => setUploadMenuAnchor(null)}
              slotProps={{ paper: { sx: { minWidth: 180 } } }}
            >
              {onUpload ? (
                <MenuItem
                  disabled={!acceptsContent}
                  onClick={() => {
                    setUploadMenuAnchor(null)
                    onUpload()
                  }}
                >
                  <ListItemIcon><Upload size={16} /></ListItemIcon>
                  <ListItemText primary={t('filebrowser.operation.uploadFiles', 'Upload files')} />
                </MenuItem>
              ) : null}
              {onFolderUpload ? (
                <MenuItem
                  disabled={!acceptsContent}
                  onClick={() => {
                    setUploadMenuAnchor(null)
                    onFolderUpload()
                  }}
                >
                  <ListItemIcon><FolderUp size={16} /></ListItemIcon>
                  <ListItemText primary={t('filebrowser.operation.uploadFolder', 'Upload folder')} />
                </MenuItem>
              ) : null}
            </Menu>
          </>
        ) : null}
        {onNewFolder || onNewFile ? (
          <>
            <ToolbarIconButton
              title={t('filebrowser.actions.new', 'New')}
              disabled={!acceptsContent}
              active={Boolean(newMenuAnchor)}
              onClick={(event) => setNewMenuAnchor(event.currentTarget)}
            >
              <span className="flex items-center gap-0.5">
                <Plus size={16} />
                <ChevronDown size={14} className="opacity-70" />
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
                    <FolderPlus size={16} />
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
                    <FilePlus size={16} />
                  </ListItemIcon>
                  <ListItemText primary={t('filebrowser.actions.newTextFile', 'New text file')} />
                </MenuItem>
              ) : null}
            </Menu>
          </>
        ) : null}

        {onCopy && <ToolbarIconButton title={t('filebrowser.menu.copyFiles', 'Copy files')} onClick={onCopy}><Copy size={16} /></ToolbarIconButton>}
        {onPaste && <ToolbarIconButton title={t('filebrowser.menu.paste', 'Paste')} onClick={onPaste}><ClipboardPaste size={16} /></ToolbarIconButton>}
        {onCopyTo && <ToolbarIconButton title={t('filebrowser.actions.copyTo', 'Copy to')} onClick={onCopyTo}><FolderInput size={16} /></ToolbarIconButton>}
        {onAddExisting && <button type="button" className="files-icon-btn !px-2 text-[13px]" onClick={onAddExisting}>{t('filebrowser.operation.addExisting', 'Add existing files')}</button>}
        <ToolbarIconButton title={t('filebrowser.mobile.moreActions', 'More actions')} onClick={(event) => { const box = event.currentTarget.getBoundingClientRect(); onMore({ left: box.left, top: box.bottom }) }}><MoreHorizontal size={16} /></ToolbarIconButton>
        <ToolbarIconButton title={t('filebrowser.menu.details', 'Details')} onClick={onDetails}><PanelRightOpen size={16} /></ToolbarIconButton>
        {selectedCount > 0 && <span className="ml-0.5 text-[12px] tabular-nums text-[color:var(--cp-muted)]" title={t('filebrowser.status.selectedCount', '{{count}} selected', { count: selectedCount })}>{selectedCount}</span>}
        <div className="min-w-2 flex-1" />

        <ToolbarIconButton
          title={t('filebrowser.topbar.viewMode', 'View mode')}
          active={Boolean(viewMenuAnchor)}
          onClick={(event) => setViewMenuAnchor(event.currentTarget)}
        >
          <span className="flex items-center gap-0.5">
            {viewMode === 'list' ? <List size={16} /> : <LayoutGrid size={16} />}
            <ChevronDown size={14} className="opacity-70" />
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
              <List size={16} />
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
              <LayoutGrid size={16} />
            </ListItemIcon>
            <ListItemText primary={t('filebrowser.view.icon', 'Icon view')} />
          </MenuItem>
          {onIconSizeChange
            ? ICON_SIZES.map((size) => (
                <MenuItem
                  key={size}
                  disabled={viewMode !== 'icon'}
                  selected={viewMode === 'icon' && iconSize === size}
                  onClick={() => {
                    setViewMenuAnchor(null)
                    onIconSizeChange(size)
                  }}
                  sx={{ pl: 6 }}
                  data-testid={`icon-size-${size}`}
                >
                  {t(`filebrowser.view.icon.${size}`, size === 'small' ? 'Small icons' : size === 'medium' ? 'Medium icons' : 'Large icons')}
                </MenuItem>
              ))
            : null}
          <Divider />
          <MenuItem selected={browseArchives} onClick={() => setBrowseArchives(!browseArchives)} data-testid="browse-archives-toggle">
            <ListItemIcon>{browseArchives ? <Check size={16} /> : null}</ListItemIcon>
            <ListItemText primary={t('archive.browseSetting', 'Browse archives as folders')} />
          </MenuItem>
          <Divider />
          <MenuItem selected={!listPreferences.fullColumns} onClick={() => onListPreferences({ fullColumns: false })}>{t('filebrowser.view.responsiveColumns', 'Responsive columns')}</MenuItem>
          <MenuItem selected={listPreferences.fullColumns} onClick={() => onListPreferences({ fullColumns: true })}>{t('filebrowser.view.fullColumns', 'All columns (horizontal scroll)')}</MenuItem>
          <Divider />
          <MenuItem selected={listPreferences.density === 'compact'} onClick={() => onListPreferences({ density: 'compact' })}>{t('filebrowser.view.compactRows', 'Compact rows')}</MenuItem>
          <MenuItem selected={listPreferences.density === 'comfortable'} onClick={() => onListPreferences({ density: 'comfortable' })}>{t('filebrowser.view.comfortableRows', 'Comfortable rows')}</MenuItem>
        </Menu>

        <button
          type="button"
          onClick={(event) => setSortMenuAnchor(event.currentTarget)}
          className="files-icon-btn gap-1.5 !px-2 text-[13px]"
          aria-label={t('filebrowser.topbar.sort', 'Sort by')}
          aria-haspopup="menu"
          aria-expanded={Boolean(sortMenuAnchor)}
        >
          <ArrowUpDown size={16} />
          <span className="fb-sort-label">{sortLabels[sortKey]}</span>
          <ChevronDown size={14} className="-ml-0.5 opacity-70" />
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
            disabled={sortKey === 'manual' || !sortDirs.includes('asc')}
            onClick={() => {
              setSortMenuAnchor(null)
              onSortChange?.(sortKey, 'asc')
            }}
          >
            <ListItemText primary={t('filebrowser.sort.asc', 'Ascending')} />
          </MenuItem>
          <MenuItem
            selected={sortDir === 'desc'}
            disabled={sortKey === 'manual' || !sortDirs.includes('desc')}
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
          {searchVisible ? <SearchX size={16} /> : <Search size={16} />}
        </ToolbarIconButton>
      </div>
    </div>
  )
}
