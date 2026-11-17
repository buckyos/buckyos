import clsx from 'clsx'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  ArrowDown,
  ArrowUp,
  CheckCircle2,
  Circle,
  CornerUpRight,
  FolderClosed,
  Layers,
  Library,
  Lock,
  LockOpen,
  MoreVertical,
  Upload,
} from 'lucide-react'
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { FileItemList } from './data/FileItemList'
import type { FileItem } from './data/FolderReader'
import { COLLECTION_SCHEME, displayPath } from './data/urls'
import { fileIcon, formatBytes, formatDate } from './fileDisplay'
import { FileThumb } from '../../components/preview/FileThumb'
import type { ContentRef } from '../../components/preview/types'
import { ICON_GRID, useIconSize } from './data/iconSize'
import { isSudoRequired } from './data/hostRealm'
import type { ViewMode, SortKey, SortDir } from './types'
import '../../components/preview/files.css'
import './filebrowser.css'

/** A list item is a reference when it's a link entry or a collection member (not a group). */
function isReferenceItem(item: FileItem): boolean {
  if (item.entry.link) return true
  return !!item.ref && !item.entry.path.startsWith(COLLECTION_SCHEME)
}

function isBrokenItem(item: FileItem): boolean {
  return !!item.ref?.broken || !!item.entry.link?.broken
}

/** A folder or file on the computer that its OS account cannot open. */
function isLockedEntry(entry: FileItem['entry']): boolean {
  return entry.accessLevel === 'none' && entry.path.startsWith('device://')
}

function entryIcon(entry: FileItem['entry'], size: number) {
  return isLockedEntry(entry)
    ? <Lock size={size} className="text-[color:var(--cp-muted)]" aria-label="locked" data-locked="true" />
    : fileIcon(entry, size)
}

/** The signature dot: the AI pipeline is working on this entry. */
function AiDot({ title }: { title: string }) {
  return <span className="files-dot" title={title} aria-label={title} role="img" data-testid="ai-active" />
}

/** Finder-alias style corner badge for reference items. */
function LinkBadge({ size = 10 }: { size?: number }) {
  return (
    <span
      className="absolute -bottom-0.5 -left-0.5 flex items-center justify-center rounded-full border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)] p-[1px] text-[color:var(--cp-muted)]"
      data-testid="link-badge"
    >
      <CornerUpRight size={size} />
    </span>
  )
}

/** Modifier keys captured on click — drives multi-select on desktop. */
export interface SelectModifiers {
  shift?: boolean
  toggle?: boolean
}

export interface MenuPosition {
  top: number
  left: number
}

interface PresentationProps {
  fullColumns?: boolean
  density?: 'compact' | 'comfortable'
  nameWidth?: number
  onNameWidthChange?: (width: number) => void
  onClearSelection?: () => void
  scroll?: number
  onScroll?: (scroll: number) => void
  revealIndex?: number | null
  cutIds?: ReadonlySet<string>
  onSelectLoaded?: () => void
  onSelectAll?: () => void
  sortKey?: SortKey
  sortDir?: SortDir
  onSortChange?: (key: SortKey, dir: SortDir) => void
  /** Where the icon view's thumbnails read an entry's content (null = icon only). */
  thumbnailSource?: (entry: FileItem['entry']) => ContentRef | null
  /** The location is the locked computer: offer BuckyOS sudo. */
  onUnlock?: () => void
  /** The location is an archive whose names are encrypted (7z `-mhe`): offer its password. */
  onArchivePassword?: () => void
  /** The host's own empty state (an archive is not a folder you upload into). */
  emptyState?: { title: string; body?: string; icon?: React.ReactNode }
  /** The host's wording for a read-only location (default: "Read-only folder"). */
  readOnlyNote?: string
}
interface MainContentProps extends PresentationProps {
  list: FileItemList
  viewMode: ViewMode
  selectedKeys: ReadonlySet<string>
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  /** Open a container (folder path or collection/view url). */
  onOpenFolder: (url: string) => void
  /** Open a file (double-click / Enter) — the host hands it to the Preview App. */
  onOpenFile?: (item: FileItem) => void
  /** Right-click on an item (desktop). */
  onItemContextMenu?: (item: FileItem, position: MenuPosition) => void
  /** Right-click on blank space (desktop). */
  onViewContextMenu?: (position: MenuPosition) => void
  /** Click on blank space clears the selection (desktop). */
  onClearSelection?: () => void
  currentUrl: string
  isMobile?: boolean
  /** Mobile selection mode — per-item menus become checkboxes. */
  selectionMode?: boolean
  /** Tap on an item's ⋮ button (mobile). */
  onItemMenu?: (item: FileItem) => void
  /** Long-press on an item (mobile) — enters/extends selection mode. */
  onLongPress?: (item: FileItem) => void
  /** Start an upload into this location (empty-state action). */
  onUpload?: () => void
}

/**
 * Virtual extent (§5.1): the known total, or the loaded window plus one
 * sentinel row that demand-loads the next cursor page when scrolled into view.
 */
function virtualCountOf(list: FileItemList): number {
  return list.totalCount ?? list.loadedCount + (list.hasMore ? 1 : 0)
}

const modifiersFromEvent = (event: React.MouseEvent): SelectModifiers => ({
  shift: event.shiftKey,
  toggle: event.metaKey || event.ctrlKey,
})

const LONG_PRESS_MS = 450
const LONG_PRESS_TOLERANCE = 12

/**
 * Touch long-press detection. Returns pointer handlers for the pressable
 * element and `consumeLongPress`, which the click handler calls to swallow
 * the synthetic click that follows a fired long-press.
 */
function useLongPress(onLongPress?: () => void) {
  const timerRef = useRef<number | null>(null)
  const firedRef = useRef(false)
  const originRef = useRef({ x: 0, y: 0 })

  const cancel = useCallback(() => {
    if (timerRef.current !== null) {
      window.clearTimeout(timerRef.current)
      timerRef.current = null
    }
  }, [])

  useEffect(() => cancel, [cancel])

  const consumeLongPress = useCallback(() => {
    const fired = firedRef.current
    firedRef.current = false
    return fired
  }, [])

  const handlers = useMemo(() => {
    if (!onLongPress) return {}
    return {
      onPointerDown: (event: React.PointerEvent) => {
        if (event.pointerType === 'mouse') return
        firedRef.current = false
        originRef.current = { x: event.clientX, y: event.clientY }
        cancel()
        timerRef.current = window.setTimeout(() => {
          timerRef.current = null
          firedRef.current = true
          onLongPress()
        }, LONG_PRESS_MS)
      },
      onPointerMove: (event: React.PointerEvent) => {
        if (timerRef.current === null) return
        const dx = event.clientX - originRef.current.x
        const dy = event.clientY - originRef.current.y
        if (dx * dx + dy * dy > LONG_PRESS_TOLERANCE * LONG_PRESS_TOLERANCE) cancel()
      },
      onPointerUp: cancel,
      onPointerCancel: cancel,
    }
  }, [onLongPress, cancel])

  return { handlers, consumeLongPress }
}

/** Track the first..last visible indexes and demand-load them. */
function useEnsureRange(list: FileItemList, first: number, last: number, active: boolean) {
  useEffect(() => {
    if (active) list.ensureRange(first, last)
  }, [list, first, last, active])
}

function SkeletonBar({ width }: { width: string }) {
  return (
    <span
      className="inline-block h-3 animate-pulse rounded-full bg-[color:color-mix(in_srgb,var(--cp-border)_55%,transparent)]"
      style={{ width }}
    />
  )
}

export function MainContent({
  list,
  viewMode,
  selectedKeys,
  onSelect,
  onOpenFolder,
  onOpenFile,
  onItemContextMenu,
  onViewContextMenu,
  onClearSelection,
  currentUrl,
  isMobile = false,
  selectionMode = false,
  onItemMenu,
  onLongPress,
  onUpload,
  ...presentation
}: MainContentProps) {
  const { t } = useI18n()

  const path = displayPath(currentUrl)
  const isPublic = path === '/public' || path.startsWith('/public/')
  const capabilities = list.capabilities
  const meta = list.meta

  const handleItemContextMenu = (item: FileItem) => (event: React.MouseEvent) => {
    if (!onItemContextMenu) return
    event.preventDefault()
    event.stopPropagation()
    onItemContextMenu(item, { top: event.clientY, left: event.clientX })
  }

  const handleViewContextMenu = (event: React.MouseEvent) => {
    if (!onViewContextMenu) return
    event.preventDefault()
    onViewContextMenu({ top: event.clientY, left: event.clientX })
  }

  const handleBlankClick = (event: React.MouseEvent) => {
    if (event.target === event.currentTarget) onClearSelection?.()
  }

  const openItem = (item: FileItem) => {
    if (item.entry.kind === 'folder') {
      // The computer's account cannot open it (shown with a lock).
      if (!isLockedEntry(item.entry)) onOpenFolder(item.entry.path)
    } else onOpenFile?.(item)
  }

  // ─── Location banner (views & collections — replaces the old topic banner) ───
  const banner =
    capabilities.kind !== 'folder' && meta ? (
      <div className="flex items-center gap-3 border-b border-[color:var(--files-hairline)] px-4 py-3">
        {capabilities.kind === 'collection' ? (
          <Library size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
        ) : (
          <Layers size={16} className="shrink-0 text-[color:var(--cp-muted)]" />
        )}
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] font-medium text-[color:var(--cp-text)]">
            {capabilities.kind === 'collection'
              ? `${t('filebrowser.collection.badge', 'Collection')}: ${meta.title}`
              : `${t('filebrowser.view.badge', 'View')}: ${meta.title}`}
          </div>
          {meta.description ? (
            <div className="mt-0.5 line-clamp-1 text-[12px] text-[color:var(--cp-muted)]">
              {meta.description}
            </div>
          ) : null}
        </div>
        <span className="shrink-0 text-[12px] tabular-nums text-[color:var(--cp-muted)]">
          {capabilities.kind === 'collection'
            ? t('filebrowser.collection.itemCount', '{{count}} items · originals stay in place', {
                count: list.totalCount ?? '…',
              })
            : t('filebrowser.topic.aggregation', 'Aggregated · not copied')}
        </span>
      </div>
    ) : null

  // ─── Error / initial loading / empty states ───
  let body: React.ReactNode
  if (list.status === 'error' && isSudoRequired(list.error)) {
    body = (
      <div className="flex h-full w-full flex-col items-center justify-center gap-2 p-10 text-center text-[color:var(--cp-muted)]" data-testid="filebrowser-host-locked">
        <Lock size={32} strokeWidth={1.5} />
        <p className="mt-2 text-[16px] font-medium text-[color:var(--cp-text)]">{t('filebrowser.host.lockedTitle', 'This computer is locked')}</p>
        <p className="max-w-sm text-[14px] leading-6">{t('filebrowser.host.lockedBody', 'Confirm your password to see every folder on this computer. The unlock lasts 30 minutes, or until this window closes.')}</p>
        {presentation.onUnlock ? (
          <button
            type="button"
            onClick={presentation.onUnlock}
            className="files-btn files-btn-primary files-btn-lg mt-2"
          >
            <LockOpen size={16} /> {t('filebrowser.host.unlock', 'Unlock')}
          </button>
        ) : null}
      </div>
    )
  } else if (list.status === 'error' && presentation.onArchivePassword && (list.error as { code?: unknown } | null)?.code === 'PASSWORD_REQUIRED') {
    body = (
      <div className="flex h-full w-full flex-col items-center justify-center gap-2 p-10 text-center text-[color:var(--cp-muted)]" data-testid="filebrowser-archive-locked">
        <Lock size={32} strokeWidth={1.5} />
        <p className="mt-2 text-[16px] font-medium text-[color:var(--cp-text)]">{t('archive.locked.title', 'This archive is encrypted')}</p>
        <p className="max-w-sm text-[14px] leading-6">{t('archive.locked.body', 'Even the names inside are encrypted. Enter the password to see them.')}</p>
        <button
          type="button"
          onClick={presentation.onArchivePassword}
          className="files-btn files-btn-primary files-btn-lg mt-2"
        >
          <LockOpen size={16} /> {t('archive.password.enter', 'Enter password')}
        </button>
      </div>
    )
  } else if (list.status === 'error') {
    body = (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 p-10 text-center text-[color:var(--cp-muted)]">
        <p className="text-[14px]">{list.error?.message ?? t('filebrowser.error', 'Failed to load')}</p>
        <button
          type="button"
          onClick={() => list.reload()}
          className="files-btn"
        >
          {t('filebrowser.retry', 'Retry')}
        </button>
      </div>
    )
  } else if (virtualCountOf(list) === 0 && list.status !== 'ready') {
    // First page still in flight — skeleton screen. Once anything is loaded
    // (or the extent is known) the virtualized views take over, including the
    // unknown-total load-more sentinel.
    body = (
      <div className="flex-1 overflow-hidden px-4 py-3" data-testid="filebrowser-skeleton">
        {Array.from({ length: 12 }, (_, i) => (
          <div key={i} className="flex items-center gap-3 py-2.5">
            <span className="h-4 w-4 animate-pulse rounded-[4px] bg-[color:color-mix(in_srgb,var(--cp-border)_55%,transparent)]" />
            <SkeletonBar width={`${36 - (i % 4) * 6}%`} />
            <span className="flex-1" />
            <SkeletonBar width="48px" />
            <SkeletonBar width="96px" />
          </div>
        ))}
      </div>
    )
  } else if (virtualCountOf(list) === 0 && list.status === 'ready' && presentation.emptyState) {
    body = (
      <div className="flex h-full w-full flex-col items-center justify-center p-10 text-center text-[color:var(--cp-muted)]" data-testid="filebrowser-empty">
        {presentation.emptyState.icon ?? <FolderClosed size={32} strokeWidth={1.5} />}
        <p className="mt-4 text-[16px] font-medium text-[color:var(--cp-text)]">{presentation.emptyState.title}</p>
        {presentation.emptyState.body ? <p className="mt-1 max-w-sm text-[14px] leading-6">{presentation.emptyState.body}</p> : null}
      </div>
    )
  } else if (virtualCountOf(list) === 0 && list.status === 'ready') {
    body = (
      <div
        className="flex h-full w-full flex-col items-center justify-center p-10 text-center text-[color:var(--cp-muted)]"
        onContextMenu={handleViewContextMenu}
      >
        {capabilities.kind === 'collection' ? (
          <Library size={32} strokeWidth={1.5} />
        ) : capabilities.kind === 'view' ? (
          <Layers size={32} strokeWidth={1.5} />
        ) : (
          <FolderClosed size={32} strokeWidth={1.5} />
        )}
        <p className="mt-4 text-[16px] font-medium text-[color:var(--cp-text)]">
          {capabilities.kind === 'collection'
            ? t('filebrowser.empty.collectionTitle', 'This collection is empty')
            : capabilities.kind === 'view'
              ? t('filebrowser.empty.viewTitle', 'Nothing here')
              : t('filebrowser.empty.title', 'This folder is empty')}
        </p>
        <p className="mt-1 max-w-sm text-[14px] leading-6">
          {capabilities.kind === 'collection'
            ? t(
                'filebrowser.empty.collectionBody',
                'Drag files in, or use "Add to Collection" from any file\'s context menu. References never move the originals.',
              )
            : capabilities.kind === 'view'
              ? t('filebrowser.empty.viewBody', 'No content matches this view\'s conditions.')
              : t(
                  'filebrowser.empty.body',
                  'No files here yet. Upload from your device, drop files from chat, or wait for the next sync.',
                )}
        </p>
        {capabilities.acceptsContent ? (
          <button
            type="button"
            onClick={onUpload}
            className="files-btn mt-4"
          >
            <Upload size={16} /> {t('filebrowser.actions.upload', 'Upload')}
          </button>
        ) : null}
      </div>
    )
  } else if (viewMode === 'list' && isMobile) {
    body = (
      <MobileListView
        presentation={{ ...presentation, onClearSelection }}
        list={list}
        selectedKeys={selectedKeys}
        onSelect={onSelect}
        isPublic={isPublic}
        selectionMode={selectionMode}
        onItemMenu={onItemMenu}
        onLongPress={onLongPress}
      />
    )
  } else if (viewMode === 'list') {
    body = (
      <DesktopListView
        presentation={{ ...presentation, onClearSelection }}
        list={list}
        selectedKeys={selectedKeys}
        onSelect={onSelect}
        openItem={openItem}
        isPublic={isPublic}
        handleItemContextMenu={handleItemContextMenu}
        handleViewContextMenu={handleViewContextMenu}
        handleBlankClick={handleBlankClick}
      />
    )
  } else {
    body = (
      <IconGridView
        presentation={{ ...presentation, onClearSelection }}
        list={list}
        selectedKeys={selectedKeys}
        onSelect={onSelect}
        openItem={openItem}
        handleItemContextMenu={handleItemContextMenu}
        handleViewContextMenu={handleViewContextMenu}
        handleBlankClick={handleBlankClick}
        selectionMode={isMobile ? selectionMode : false}
        onItemMenu={isMobile ? onItemMenu : undefined}
        onLongPress={isMobile ? onLongPress : undefined}
        isMobile={isMobile}
      />
    )
  }

  return (
    <div className="fb-content flex h-full w-full flex-col">
      {banner}
      {list.capabilities.availability === 'loading' && <p role="status" className="border-b border-[color:var(--files-hairline)] px-4 py-2 text-[12px] text-[color:var(--cp-muted)]">{t('filebrowser.operation.permissionsLoading', 'Permissions are loading')}</p>}
      {list.capabilities.kind === 'folder' && !list.capabilities.acceptsContent && list.capabilities.availability !== 'loading' && <p className="border-b border-[color:var(--files-hairline)] px-4 py-2 text-[12px] text-[color:var(--cp-muted)]" data-testid="filebrowser-read-only">{presentation.readOnlyNote ?? t('filebrowser.operation.readOnly', 'Read-only folder')}</p>}
      {body}
    </div>
  )
}

// ─── Desktop list (virtualized div-grid table — <table> doesn't virtualize) ───

function useScrollState(parent: React.RefObject<HTMLDivElement | null>, settings: PresentationProps, revealTop: number | null) {
  const onScroll = useRef(settings.onScroll)
  const [initialScroll] = useState(settings.scroll ?? 0)
  useLayoutEffect(() => { onScroll.current = settings.onScroll })
  useLayoutEffect(() => {
    const el = parent.current
    if (!el) return
    el.scrollTo({ top: initialScroll })
    const listener = () => onScroll.current?.(el.scrollTop)
    el.addEventListener('scroll', listener)
    return () => el.removeEventListener('scroll', listener)
  }, [parent, initialScroll])
  useEffect(() => { if (revealTop !== null && parent.current) parent.current.scrollTo({ top: revealTop }) }, [parent, revealTop])
}

const LIST_ROW_HEIGHT = 36

function DesktopListView({
  presentation,
  list,
  selectedKeys,
  onSelect,
  openItem,
  isPublic,
  handleItemContextMenu,
  handleViewContextMenu,
  handleBlankClick,
}: {
  presentation: PresentationProps
  list: FileItemList
  selectedKeys: ReadonlySet<string>
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  openItem: (item: FileItem) => void
  isPublic: boolean
  handleItemContextMenu: (item: FileItem) => (event: React.MouseEvent) => void
  handleViewContextMenu: (event: React.MouseEvent) => void
  handleBlankClick: (event: React.MouseEvent) => void
}) {
  const { t } = useI18n()
  const parentRef = useRef<HTMLDivElement>(null)
  const count = virtualCountOf(list)
  const rowHeight = presentation.density === 'comfortable' ? 48 : LIST_ROW_HEIGHT
  const resizeStart = useRef<{ x: number; width: number } | null>(null)

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 12,
  })
  useEffect(() => virtualizer.measure(), [rowHeight, virtualizer])
  const virtualItems = virtualizer.getVirtualItems()
  const first = virtualItems[0]?.index ?? 0
  const last = virtualItems[virtualItems.length - 1]?.index ?? 0
  useEnsureRange(list, first, last, count > 0)

  useScrollState(parentRef, presentation, presentation.revealIndex != null ? presentation.revealIndex * rowHeight : null)
  const [focusIndex, setFocusIndex] = useState<number | null>(null)
  const pendingModifiers = useRef<SelectModifiers>({})
  useEffect(() => {
    if (focusIndex === null) return
    const item = list.itemAt(focusIndex)
    if (!item) { list.ensureRange(focusIndex, focusIndex); return }
    onSelect(item, pendingModifiers.current)
    parentRef.current?.querySelector<HTMLElement>(`[data-row-index="${focusIndex}"]`)?.focus({ preventScroll: true })
    setFocusIndex(null)
  }, [focusIndex, list, list.snapshot, onSelect])
  const gridTemplateColumns = undefined
  const sortMark = (key: SortKey) => presentation.sortKey === key
    ? presentation.sortDir === 'desc' ? <ArrowDown size={12} className="shrink-0" aria-hidden /> : <ArrowUp size={12} className="shrink-0" aria-hidden />
    : null
  const sortButton = (label: string, key: SortKey) => (
    <button type="button" className="inline-flex items-center gap-1 rounded-[4px] hover:text-[color:var(--cp-text)] disabled:hover:text-inherit" disabled={!list.capabilities.sortKeys.includes(key)} onClick={() => presentation.onSortChange?.(key, presentation.sortKey === key && presentation.sortDir === 'asc' && (list.capabilities.sortDirs?.includes('desc') ?? true) ? 'desc' : 'asc')}>{label}{sortMark(key)}</button>
  )
  const headerCell = (label: string, column: string, key?: SortKey) => (
    <div role="columnheader" className={`fb-column-${column} flex h-8 items-center px-2`} aria-sort={key && presentation.sortKey === key ? presentation.sortDir === 'desc' ? 'descending' : 'ascending' : undefined}>
      {key ? sortButton(label, key) : label}
    </div>
  )

  return (
    <div
      ref={parentRef}
      className="flex-1 overflow-y-auto"
      onContextMenu={handleViewContextMenu}
      onClick={handleBlankClick}
    >
      <div role="table" className={clsx("fb-table w-full select-none text-sm", isPublic && "fb-public", presentation.fullColumns && "fb-full")} style={presentation.fullColumns ? { minWidth: (presentation.nameWidth ?? 260) + 462 + (isPublic ? 160 : 0), '--fb-name-width': `${presentation.nameWidth ?? 260}px` } as React.CSSProperties : undefined}>
        <div
          role="row"
          className="fb-list-row fb-list-head sticky top-0 z-10 grid items-center border-b border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] text-left text-[12px] font-medium text-[color:var(--cp-muted)]"
          style={{ gridTemplateColumns }}
        >
          <div role="columnheader" className="relative flex h-8 items-center gap-2 pl-4 pr-2" aria-sort={presentation.sortKey === 'name' ? presentation.sortDir === 'desc' ? 'descending' : 'ascending' : undefined}>
            <input type="checkbox" aria-label={t('filebrowser.menu.selectLoaded', 'Select loaded {{count}} items', { count: list.loadedKeys().length })} checked={list.loadedKeys().length > 0 && list.loadedKeys().every((key) => selectedKeys.has(key))} ref={(el) => { if (el) el.indeterminate = selectedKeys.size > 0 && !list.loadedKeys().every((key) => selectedKeys.has(key)) }} onChange={(event) => event.target.checked ? presentation.onSelectLoaded?.() : presentation.onClearSelection?.()} />
            {sortButton(t('filebrowser.column.name', 'Name'), 'name')}
            {presentation.fullColumns && <span role="separator" aria-label={t('filebrowser.view.resizeName', 'Resize name column')} aria-orientation="vertical" tabIndex={0} className="absolute inset-y-0 right-0 w-2 cursor-col-resize touch-none border-r border-[color:var(--cp-border)]" onPointerDown={(event) => { resizeStart.current = { x: event.clientX, width: presentation.nameWidth ?? 260 }; event.currentTarget.setPointerCapture(event.pointerId) }} onPointerMove={(event) => { if (resizeStart.current) presentation.onNameWidthChange?.(Math.max(160, Math.min(600, resizeStart.current.width + event.clientX - resizeStart.current.x))) }} onPointerUp={() => { resizeStart.current = null }} onPointerCancel={() => { resizeStart.current = null }} onKeyDown={(event) => { if (event.key === 'ArrowLeft' || event.key === 'ArrowRight') { event.preventDefault(); event.stopPropagation(); presentation.onNameWidthChange?.(Math.max(160, Math.min(600, (presentation.nameWidth ?? 260) + (event.key === 'ArrowLeft' ? -20 : 20)))) } }} />}
          </div>
          {headerCell(t('filebrowser.column.kind', 'Kind'), 'kind', 'kind')}
          {headerCell(t('filebrowser.column.size', 'Size'), 'size', 'size')}
          {headerCell(t('filebrowser.column.modified', 'Modified'), 'modified', 'modified')}
          {headerCell(t('filebrowser.column.tags', 'Tags'), 'tags')}
          {isPublic ? headerCell(t('filebrowser.column.publicUrl', 'Public URL'), 'url') : null}
        </div>

        <div
          className="relative w-full"
          style={{ height: virtualizer.getTotalSize() }}
          onClick={handleBlankClick}
        >
          {virtualItems.map((virtualRow) => {
            const item = list.itemAt(virtualRow.index)
            const rowStyle: React.CSSProperties = {
              position: 'absolute',
              top: 0,
              left: 0,
              width: '100%',
              height: virtualRow.size,
              transform: `translateY(${virtualRow.start}px)`,
              gridTemplateColumns,
            }
            if (!item) {
              return (
                <div
                  key={`skeleton-${virtualRow.index}`}
                  role="row"
                  className="fb-list-row grid items-center"
                  style={rowStyle}
                >
                  <div className="py-2 pl-4 pr-2">
                    <SkeletonBar width="55%" />
                  </div>
                  <div className="px-2 py-2">
                    <SkeletonBar width="60%" />
                  </div>
                  <div className="px-2 py-2">
                    <SkeletonBar width="70%" />
                  </div>
                  <div className="px-2 py-2">
                    <SkeletonBar width="80%" />
                  </div>
                  <div className="px-2 py-2" />
                  {isPublic ? <div className="px-2 py-2" /> : null}
                </div>
              )
            }
            const { entry } = item
            const selected = selectedKeys.has(item.key)
            const broken = isBrokenItem(item)
            return (
              <div
                key={item.key}
                role="row"
                className={clsx(
                  'fb-list-row fb-row grid cursor-default items-center text-[13px]',
                  (broken || presentation.cutIds?.has(entry.id)) && 'opacity-50',
                )}
                style={rowStyle}
                tabIndex={selected || (selectedKeys.size === 0 && virtualRow.index === 0) ? 0 : -1}
                aria-selected={selected}
                data-entry-id={entry.id}
                data-item-key={item.key}
                draggable={!broken}
                data-row-index={virtualRow.index}
                onClick={(event) => { event.currentTarget.focus(); onSelect(item, modifiersFromEvent(event)) }}
                onKeyDown={(event) => {
                  if (event.target instanceof HTMLInputElement) return
                  if (event.key === 'ArrowDown' || event.key === 'ArrowUp' || event.key === 'Home' || event.key === 'End') {
                    event.preventDefault(); event.stopPropagation()
                    const next = event.key === 'Home' ? 0 : event.key === 'End' ? count - 1 : Math.max(0, Math.min(count - 1, virtualRow.index + (event.key === 'ArrowDown' ? 1 : -1)))
                    pendingModifiers.current = { shift: event.shiftKey }
                    virtualizer.scrollToIndex(next)
                    setFocusIndex(next)
                  } else if (event.key === ' ') { event.preventDefault(); onSelect(item, { toggle: true }) }
                }}
                onDoubleClick={() => openItem(item)}
                onContextMenu={handleItemContextMenu(item)}
              >
                <div role="cell" aria-label={entry.name} className="min-w-0 pl-4 pr-2">
                  <div className="flex items-center gap-2.5">
                    <input type="checkbox" aria-label={t('filebrowser.operation.selectName', 'Select {{name}}', { name: entry.name })} checked={selected} onClick={(event) => event.stopPropagation()} onChange={() => onSelect(item, { toggle: true })} />
                    <span className="relative inline-flex shrink-0">
                      {entryIcon(entry, 16)}
                      {isReferenceItem(item) ? <LinkBadge size={8} /> : null}
                    </span>
                    <span className="truncate text-[14px] text-[color:var(--cp-text)]">
                      {entry.name}
                    </span>
                    {entry.triggersActive ? <AiDot title={t('filebrowser.status.aiActive', 'AI pipeline active')} /> : null}
                  </div>
                </div>
                <div role="cell" className="fb-column-kind truncate px-2 capitalize text-[color:var(--cp-muted)]">
                  {t(`filebrowser.kind.${entry.kind}`, entry.kind)}
                </div>
                <div role="cell" className="fb-column-size px-2 text-[color:var(--cp-muted)]">
                  {entry.kind === 'folder' ? <span title={t('filebrowser.meta.notCalculated', 'Not calculated')}>—</span> : formatBytes(entry.sizeBytes)}
                </div>
                <div role="cell" className="fb-column-modified truncate px-2 text-[color:var(--cp-muted)]">
                  {formatDate(entry.modifiedAt)}
                </div>
                <div role="cell" className="fb-column-tags min-w-0 px-2">
                  <div className="flex min-w-0 gap-2 overflow-hidden text-[12px] text-[color:var(--cp-muted)]">
                    {entry.tags?.slice(0, 2).map((tag) => (
                      <span key={tag} className="min-w-0 truncate">
                        #{tag}
                      </span>
                    ))}
                  </div>
                </div>
                {isPublic ? (
                  <div
                    role="cell"
                    className="fb-column-url truncate px-2 font-[family-name:var(--files-mono)] text-[12px] text-[color:var(--cp-muted)]"
                  >
                    {entry.publicUrl ?? '—'}
                  </div>
                ) : null}
              </div>
            )
          })}
        </div>
      </div>
    </div>
  )
}

// ─── Icon grid (virtualized by row; three icon sizes — desktop by cell width, phones by columns) ───

const GRID_GAP = 12
const GRID_PADDING = 32

function IconGridView({
  presentation,
  list,
  selectedKeys,
  onSelect,
  openItem,
  handleItemContextMenu,
  handleViewContextMenu,
  handleBlankClick,
  selectionMode = false,
  onItemMenu,
  onLongPress,
  isMobile = false,
}: {
  presentation: PresentationProps
  list: FileItemList
  selectedKeys: ReadonlySet<string>
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  openItem: (item: FileItem) => void
  handleItemContextMenu: (item: FileItem) => (event: React.MouseEvent) => void
  handleViewContextMenu: (event: React.MouseEvent) => void
  handleBlankClick: (event: React.MouseEvent) => void
  selectionMode?: boolean
  onItemMenu?: (item: FileItem) => void
  onLongPress?: (item: FileItem) => void
  isMobile?: boolean
}) {
  const parentRef = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)
  const iconSize = useIconSize()
  const metrics = ICON_GRID[iconSize]

  useEffect(() => {
    const element = parentRef.current
    if (!element) return
    const compute = () => setWidth(element.clientWidth)
    compute()
    const observer = new ResizeObserver(compute)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const inner = Math.max(0, width - GRID_PADDING)
  const columns = isMobile ? metrics.columns : Math.max(1, Math.floor((inner + GRID_GAP) / (metrics.cell + GRID_GAP)))
  const cellWidth = columns > 0 ? (inner - GRID_GAP * (columns - 1)) / columns : metrics.cell
  // The cell pads 12px on each side; the box never outgrows a narrow cell.
  const box = Math.max(48, Math.min(metrics.box, Math.floor(cellWidth - 24)))
  const rowHeight = box + 96

  const count = virtualCountOf(list)
  const rowCount = Math.ceil(count / columns)
  useScrollState(parentRef, presentation, presentation.revealIndex != null ? Math.floor(presentation.revealIndex / columns) * rowHeight : null)

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => rowHeight,
    overscan: 4,
  })
  useLayoutEffect(() => {
    virtualizer.measure()
  }, [virtualizer, rowHeight, columns])
  const virtualRows = virtualizer.getVirtualItems()
  const firstIndex = (virtualRows[0]?.index ?? 0) * columns
  const lastIndex = ((virtualRows[virtualRows.length - 1]?.index ?? 0) + 1) * columns - 1
  useEnsureRange(list, firstIndex, lastIndex, count > 0)

  return (
    <div
      ref={parentRef}
      className="flex-1 overflow-y-auto p-4"
      onContextMenu={handleViewContextMenu}
      onClick={handleBlankClick}
      data-testid="icon-grid"
      data-icon-size={iconSize}
      data-columns={columns}
    >
      <div
        className="relative w-full select-none"
        style={{ height: virtualizer.getTotalSize() }}
        onClick={handleBlankClick}
      >
        {virtualRows.map((virtualRow) => {
          const start = virtualRow.index * columns
          const cells = Array.from(
            { length: Math.min(columns, count - start) },
            (_, i) => start + i,
          )
          return (
            <div
              key={virtualRow.index}
              className="absolute left-0 grid w-full"
              style={{
                top: 0,
                transform: `translateY(${virtualRow.start}px)`,
                height: virtualRow.size,
                columnGap: GRID_GAP,
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              }}
            >
              {cells.map((index) => {
                const item = list.itemAt(index)
                if (!item) {
                  return (
                    <div
                      key={`skeleton-${index}`}
                      className="flex flex-col items-center gap-2 rounded-[12px] p-3"
                    >
                      <span className="animate-pulse rounded-[8px] bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]" style={{ width: box, height: box }} />
                      <SkeletonBar width="70%" />
                    </div>
                  )
                }
                return (
                  <IconGridCell
                    key={item.key}
                    item={item}
                    box={box}
                    thumbnailSource={presentation.thumbnailSource}
                    cut={presentation.cutIds?.has(item.entry.id)}
                    selected={selectedKeys.has(item.key)}
                    selectionMode={selectionMode}
                    onSelect={onSelect}
                    openItem={openItem}
                    onContextMenu={handleItemContextMenu(item)}
                    onMenu={onItemMenu ? () => onItemMenu(item) : undefined}
                    onLongPress={onLongPress ? () => onLongPress(item) : undefined}
                  />
                )
              })}
            </div>
          )
        })}
      </div>
    </div>
  )
}

function IconGridCell({
  item, cut, box, thumbnailSource,
  selected,
  selectionMode,
  onSelect,
  openItem,
  onContextMenu,
  onMenu,
  onLongPress,
}: {
  cut?: boolean
  box: number
  thumbnailSource?: (entry: FileItem['entry']) => ContentRef | null
  item: FileItem
  selected: boolean
  selectionMode: boolean
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  openItem: (item: FileItem) => void
  onContextMenu: (event: React.MouseEvent) => void
  onMenu?: () => void
  onLongPress?: () => void
}) {
  const { t } = useI18n()
  const { handlers, consumeLongPress } = useLongPress(onLongPress)
  const { entry } = item
  const broken = isBrokenItem(item)
  const source = useMemo(() => thumbnailSource?.(entry) ?? null, [thumbnailSource, entry])
  return (
    <div className="relative min-w-0">
      <button
        type="button"
        {...handlers}
        data-item-key={item.key}
        draggable={!onLongPress && !broken}
        onClick={(event) => {
          if (consumeLongPress()) return
          onSelect(item, modifiersFromEvent(event))
        }}
        onDoubleClick={() => openItem(item)}
        onContextMenu={(event) => {
          // Mobile long-press is selection mode, not the browser menu.
          if (onLongPress) {
            event.preventDefault()
            return
          }
          onContextMenu(event)
        }}
        data-selected={selected}
        className={clsx(
          'fb-row flex h-full w-full flex-col items-center gap-2 rounded-[12px] border border-transparent p-3 text-center',
          selected && 'border-[color:var(--files-selected-line)]',
          (broken || cut) && 'opacity-50',
        )}
      >
        <div className="relative shrink-0" style={{ width: box, height: box }}>
          <div className="flex size-full items-center justify-center overflow-hidden rounded-[8px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-2-opaque)]">
            {isLockedEntry(entry) ? entryIcon(entry, Math.round(Math.min(56, box * 0.42))) : <FileThumb entry={entry} source={source} iconSize={Math.round(Math.min(56, box * 0.42))} box={box} marks />}
          </div>
          {isReferenceItem(item) ? <LinkBadge /> : null}
        </div>
        <span
          title={entry.name}
          className="line-clamp-2 w-full break-words text-[13px] leading-[18px] text-[color:var(--cp-text)]"
        >
          {entry.name}
          {entry.triggersActive ? <span className="ml-1.5 inline-block align-middle"><AiDot title={t('filebrowser.status.aiActive', 'AI pipeline active')} /></span> : null}
        </span>
        <span className="text-[12px] tabular-nums text-[color:var(--cp-muted)]">
          {entry.kind === 'folder' ? '\u00a0' : formatBytes(entry.sizeBytes)}
        </span>
      </button>
      {selectionMode ? (
        <span
          className={clsx(
            'pointer-events-none absolute right-1.5 top-1.5',
            selected ? 'text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)]',
          )}
        >
          {selected ? <CheckCircle2 size={20} /> : <Circle size={20} />}
        </span>
      ) : onMenu ? (
        <button
          type="button"
          aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
          onClick={(event) => {
            event.stopPropagation()
            onMenu()
          }}
          className="absolute right-0.5 top-0.5 flex h-11 w-11 items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:var(--files-pressed)]"
        >
          <MoreVertical size={20} />
        </button>
      ) : null}
    </div>
  )
}

// ─── Mobile list (virtualized rows) ───

const MOBILE_ROW_HEIGHT = 64

function MobileListView({
  presentation,
  list,
  selectedKeys,
  onSelect,
  isPublic,
  selectionMode = false,
  onItemMenu,
  onLongPress,
}: {
  presentation: PresentationProps
  list: FileItemList
  selectedKeys: ReadonlySet<string>
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  isPublic: boolean
  selectionMode?: boolean
  onItemMenu?: (item: FileItem) => void
  onLongPress?: (item: FileItem) => void
}) {
  const parentRef = useRef<HTMLDivElement>(null)
  useScrollState(parentRef, presentation, presentation.revealIndex != null ? presentation.revealIndex * MOBILE_ROW_HEIGHT : null)
  const count = virtualCountOf(list)

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => parentRef.current,
    estimateSize: () => MOBILE_ROW_HEIGHT,
    overscan: 10,
  })
  const virtualItems = virtualizer.getVirtualItems()
  const first = virtualItems[0]?.index ?? 0
  const last = virtualItems[virtualItems.length - 1]?.index ?? 0
  useEnsureRange(list, first, last, count > 0)

  return (
    <div ref={parentRef} className="flex-1 overflow-y-auto px-3 py-2">
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualItems.map((virtualRow) => {
          const item = list.itemAt(virtualRow.index)
          const rowStyle: React.CSSProperties = {
            position: 'absolute',
            top: 0,
            left: 0,
            width: '100%',
            height: virtualRow.size,
            transform: `translateY(${virtualRow.start}px)`,
          }
          if (!item) {
            return (
              <div
                key={`skeleton-${virtualRow.index}`}
                className="flex items-center gap-3 px-2 py-2"
                style={rowStyle}
              >
                <span className="h-11 w-11 shrink-0 animate-pulse rounded-[10px] bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]" />
                <div className="flex min-w-0 flex-1 flex-col gap-1.5">
                  <SkeletonBar width="48%" />
                  <SkeletonBar width="32%" />
                </div>
              </div>
            )
          }
          return (
            <MobileListRow
              key={item.key}
              item={item}
              style={rowStyle}
              selected={selectedKeys.has(item.key)}
              isPublic={isPublic}
              selectionMode={selectionMode}
              onTap={() => onSelect(item)}
              onMenu={onItemMenu ? () => onItemMenu(item) : undefined}
              onLongPress={onLongPress ? () => onLongPress(item) : undefined}
            />
          )
        })}
      </div>
    </div>
  )
}

function MobileListRow({
  item,
  style,
  selected,
  isPublic,
  selectionMode,
  onTap,
  onMenu,
  onLongPress,
}: {
  item: FileItem
  style: React.CSSProperties
  selected: boolean
  isPublic: boolean
  selectionMode: boolean
  onTap: () => void
  onMenu?: () => void
  onLongPress?: () => void
}) {
  const { t } = useI18n()
  const { handlers, consumeLongPress } = useLongPress(onLongPress)
  const { entry } = item
  const broken = isBrokenItem(item)
  const meta =
    entry.kind === 'folder'
      ? `${t('filebrowser.kind.folder', 'Folder')} · ${formatDate(entry.modifiedAt)}`
      : `${formatBytes(entry.sizeBytes)} · ${formatDate(entry.modifiedAt)}`
  return (
    <div
      role="button"
      tabIndex={0}
      style={style}
      data-item-key={item.key}
      {...handlers}
      onClick={() => {
        if (consumeLongPress()) return
        onTap()
      }}
      onKeyDown={(event) => {
        if (event.key === 'Enter') onTap()
      }}
      onContextMenu={(event) => {
        // Mobile long-press is selection mode, not the browser menu.
        if (onLongPress) event.preventDefault()
      }}
      data-selected={selected}
      className={clsx(
        'fb-row flex select-none items-center gap-3 rounded-[12px] px-2 py-2 text-left active:bg-[color:var(--files-pressed)]',
        broken && 'opacity-50',
      )}
    >
      <div className="relative flex h-11 w-11 shrink-0 items-center justify-center rounded-[10px] bg-[color:var(--cp-surface-2-opaque)]">
        {entryIcon(entry, 24)}
        {isReferenceItem(item) ? <LinkBadge /> : null}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-[15px] leading-5 text-[color:var(--cp-text)]">
            {entry.name}
          </span>
          {entry.triggersActive ? <AiDot title={t('filebrowser.status.aiActive', 'AI pipeline active')} /> : null}
        </div>
        <div className="mt-0.5 truncate text-[13px] leading-[18px] tabular-nums text-[color:var(--cp-muted)]">
          {meta}
          {isPublic && entry.publicUrl ? (
            <span className="ml-1 font-[family-name:var(--files-mono)]">
              · {entry.publicUrl}
            </span>
          ) : null}
        </div>
      </div>
      {selectionMode ? (
        <span
          className={clsx(
            'flex h-11 w-11 shrink-0 items-center justify-center',
            selected ? 'text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)]',
          )}
        >
          {selected ? <CheckCircle2 size={20} /> : <Circle size={20} />}
        </span>
      ) : onMenu ? (
        <button
          type="button"
          aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
          onClick={(event) => {
            event.stopPropagation()
            onMenu()
          }}
          className="flex h-11 w-11 shrink-0 items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:var(--files-pressed)]"
        >
          <MoreVertical size={20} />
        </button>
      ) : null}
    </div>
  )
}
