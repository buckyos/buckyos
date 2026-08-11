import clsx from 'clsx'
import { useVirtualizer } from '@tanstack/react-virtual'
import {
  CheckCircle2,
  Circle,
  CornerUpRight,
  FolderClosed,
  Library,
  MoreVertical,
  Sparkles,
  Upload,
  Wand2,
} from 'lucide-react'
import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { useI18n } from '../../i18n/provider'
import type { FileItemList } from './data/FileItemList'
import type { FileItem } from './data/FolderReader'
import { COLLECTION_SCHEME, displayPath } from './data/urls'
import { formatBytes, formatDate, kindIcon } from './fileDisplay'
import type { ViewMode } from './types'

/** A list item is a reference when it's a link entry or a collection member (not a group). */
function isReferenceItem(item: FileItem): boolean {
  if (item.entry.link) return true
  return !!item.ref && !item.entry.path.startsWith(COLLECTION_SCHEME)
}

function isBrokenItem(item: FileItem): boolean {
  return !!item.ref?.broken || !!item.entry.link?.broken
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

interface MainContentProps {
  list: FileItemList
  viewMode: ViewMode
  selectedKeys: ReadonlySet<string>
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  /** Open a container (folder path or collection/view url). */
  onOpenFolder: (url: string) => void
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
  onItemContextMenu,
  onViewContextMenu,
  onClearSelection,
  currentUrl,
  isMobile = false,
  selectionMode = false,
  onItemMenu,
  onLongPress,
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
    if (item.entry.kind === 'folder') onOpenFolder(item.entry.path)
  }

  // ─── Location banner (views & collections — replaces the old topic banner) ───
  const banner =
    capabilities.kind !== 'folder' && meta ? (
      <div className="flex items-center gap-3 border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_14%,var(--cp-surface))] px-4 py-2.5">
        {capabilities.kind === 'collection' ? (
          <Library size={16} className="text-[color:var(--cp-accent)]" />
        ) : (
          <Sparkles size={16} className="text-[color:var(--cp-accent)]" />
        )}
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-[color:var(--cp-text)]">
            {capabilities.kind === 'collection'
              ? `${t('filebrowser.collection.badge', 'Collection')}: ${meta.title}`
              : `${t('filebrowser.view.badge', 'View')}: ${meta.title}`}
          </div>
          {meta.description ? (
            <div className="mt-0.5 line-clamp-1 text-[11px] text-[color:var(--cp-muted)]">
              {meta.description}
            </div>
          ) : null}
        </div>
        <span className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-surface)_86%,transparent)] px-2 py-0.5 text-[11px] font-semibold text-[color:var(--cp-muted)]">
          {capabilities.kind === 'collection'
            ? t('filebrowser.collection.refCount', '{{count}} references · zero disk', {
                count: list.totalCount ?? '…',
              })
            : t('filebrowser.topic.aggregation', 'Aggregated · not copied')}
        </span>
      </div>
    ) : null

  // ─── Error / initial loading / empty states ───
  let body: React.ReactNode
  if (list.status === 'error') {
    body = (
      <div className="flex h-full w-full flex-col items-center justify-center gap-3 p-10 text-center text-[color:var(--cp-muted)]">
        <p className="text-sm">{list.error?.message ?? t('filebrowser.error', 'Failed to load')}</p>
        <button
          type="button"
          onClick={() => list.reload()}
          className="rounded-full border border-[color:var(--cp-border)] px-4 py-1.5 text-sm text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
        >
          {t('filebrowser.retry', 'Retry')}
        </button>
      </div>
    )
  } else if (list.totalCount === undefined) {
    // First page still in flight — skeleton screen.
    body = (
      <div className="flex-1 overflow-hidden px-4 py-3" data-testid="filebrowser-skeleton">
        {Array.from({ length: 12 }, (_, i) => (
          <div key={i} className="flex items-center gap-3 py-2.5">
            <span className="h-5 w-5 animate-pulse rounded-md bg-[color:color-mix(in_srgb,var(--cp-border)_55%,transparent)]" />
            <SkeletonBar width={`${36 - (i % 4) * 6}%`} />
            <span className="flex-1" />
            <SkeletonBar width="48px" />
            <SkeletonBar width="96px" />
          </div>
        ))}
      </div>
    )
  } else if (list.totalCount === 0 && list.status === 'ready') {
    body = (
      <div
        className="flex h-full w-full flex-col items-center justify-center p-10 text-center text-[color:var(--cp-muted)]"
        onContextMenu={handleViewContextMenu}
      >
        {capabilities.kind === 'collection' ? (
          <Library size={40} className="opacity-50" />
        ) : capabilities.kind === 'view' ? (
          <Sparkles size={40} className="opacity-50" />
        ) : (
          <FolderClosed size={40} className="opacity-50" />
        )}
        <p className="mt-3 font-display text-lg text-[color:var(--cp-text)]">
          {capabilities.kind === 'collection'
            ? t('filebrowser.empty.collectionTitle', 'This collection is empty')
            : capabilities.kind === 'view'
              ? t('filebrowser.empty.viewTitle', 'Nothing here')
              : t('filebrowser.empty.title', 'This folder is empty')}
        </p>
        <p className="mt-1 max-w-sm text-sm leading-6">
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
            className="mt-4 inline-flex items-center gap-2 rounded-full border border-[color:var(--cp-border)] px-4 py-1.5 text-sm text-[color:var(--cp-text)] hover:border-[color:var(--cp-accent)]"
          >
            <Upload size={14} /> {t('filebrowser.actions.upload', 'Upload')}
          </button>
        ) : null}
      </div>
    )
  } else if (viewMode === 'list' && isMobile) {
    body = (
      <MobileListView
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
      />
    )
  }

  return (
    <div className="flex h-full w-full flex-col">
      {banner}
      {body}
    </div>
  )
}

// ─── Desktop list (virtualized div-grid table — <table> doesn't virtualize) ───

const LIST_ROW_HEIGHT = 37

function DesktopListView({
  list,
  selectedKeys,
  onSelect,
  openItem,
  isPublic,
  handleItemContextMenu,
  handleViewContextMenu,
  handleBlankClick,
}: {
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
  const count = list.totalCount ?? 0

  const virtualizer = useVirtualizer({
    count,
    getScrollElement: () => parentRef.current,
    estimateSize: () => LIST_ROW_HEIGHT,
    overscan: 12,
  })
  const virtualItems = virtualizer.getVirtualItems()
  const first = virtualItems[0]?.index ?? 0
  const last = virtualItems[virtualItems.length - 1]?.index ?? 0
  useEnsureRange(list, first, last, count > 0)

  const gridTemplateColumns = `minmax(240px, 2fr) 110px 90px 150px minmax(120px, 1fr)${
    isPublic ? ' minmax(160px, 1fr)' : ''
  }`

  const headerCell = (label: string) => (
    <div role="columnheader" className="px-2 py-2 font-medium">
      {label}
    </div>
  )

  return (
    <div
      ref={parentRef}
      className="flex-1 overflow-y-auto"
      onContextMenu={handleViewContextMenu}
      onClick={handleBlankClick}
    >
      <div role="table" className="w-full select-none text-sm">
        <div
          role="row"
          className="sticky top-0 z-10 grid bg-[color:color-mix(in_srgb,var(--cp-surface)_92%,transparent)] text-left text-[11px] uppercase tracking-wider text-[color:var(--cp-muted)] backdrop-blur"
          style={{ gridTemplateColumns }}
        >
          <div role="columnheader" className="py-2 pl-4 pr-2 font-medium">
            {t('filebrowser.column.name', 'Name')}
          </div>
          {headerCell(t('filebrowser.column.kind', 'Kind'))}
          {headerCell(t('filebrowser.column.size', 'Size'))}
          {headerCell(t('filebrowser.column.modified', 'Modified'))}
          {headerCell(t('filebrowser.column.tags', 'Tags'))}
          {isPublic ? headerCell(t('filebrowser.column.publicUrl', 'Public URL')) : null}
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
                  className="grid items-center border-b border-[color:color-mix(in_srgb,var(--cp-border)_40%,transparent)]"
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
                  'grid cursor-pointer items-center border-b border-[color:color-mix(in_srgb,var(--cp-border)_40%,transparent)] transition',
                  selected
                    ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))]'
                    : 'hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_10%,transparent)]',
                  broken && 'opacity-50',
                )}
                style={rowStyle}
                onClick={(event) => onSelect(item, modifiersFromEvent(event))}
                onDoubleClick={() => openItem(item)}
                onContextMenu={handleItemContextMenu(item)}
              >
                <div role="cell" className="min-w-0 py-2 pl-4 pr-2">
                  <div className="flex items-center gap-2">
                    <span className="relative inline-flex shrink-0">
                      {kindIcon(entry.kind, 16)}
                      {isReferenceItem(item) ? <LinkBadge size={8} /> : null}
                    </span>
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
                </div>
                <div role="cell" className="px-2 py-2 capitalize text-[color:var(--cp-muted)]">
                  {entry.kind}
                </div>
                <div role="cell" className="px-2 py-2 text-[color:var(--cp-muted)]">
                  {entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}
                </div>
                <div role="cell" className="px-2 py-2 text-[color:var(--cp-muted)]">
                  {formatDate(entry.modifiedAt)}
                </div>
                <div role="cell" className="min-w-0 px-2 py-2">
                  <div className="flex flex-wrap gap-1 overflow-hidden">
                    {entry.tags?.slice(0, 2).map((tag) => (
                      <span
                        key={tag}
                        className="rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_16%,var(--cp-surface))] px-2 py-0.5 text-[10px] text-[color:var(--cp-muted)]"
                      >
                        #{tag}
                      </span>
                    ))}
                  </div>
                </div>
                {isPublic ? (
                  <div
                    role="cell"
                    className="truncate px-2 py-2 font-mono text-[10px] text-[color:var(--cp-accent)]"
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

// ─── Icon grid (virtualized by row, column count tracks container width) ───

const GRID_CELL_WIDTH = 172
const GRID_ROW_HEIGHT = 152

function IconGridView({
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
}: {
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
}) {
  const parentRef = useRef<HTMLDivElement>(null)
  const [columns, setColumns] = useState(4)

  useEffect(() => {
    const element = parentRef.current
    if (!element) return
    const compute = () =>
      setColumns(Math.max(1, Math.floor((element.clientWidth - 32) / GRID_CELL_WIDTH)))
    compute()
    const observer = new ResizeObserver(compute)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  const count = list.totalCount ?? 0
  const rowCount = Math.ceil(count / columns)

  const virtualizer = useVirtualizer({
    count: rowCount,
    getScrollElement: () => parentRef.current,
    estimateSize: () => GRID_ROW_HEIGHT,
    overscan: 6,
  })
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
              className="absolute left-0 grid w-full gap-3"
              style={{
                top: 0,
                transform: `translateY(${virtualRow.start}px)`,
                height: virtualRow.size,
                gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
              }}
            >
              {cells.map((index) => {
                const item = list.itemAt(index)
                if (!item) {
                  return (
                    <div
                      key={`skeleton-${index}`}
                      className="flex flex-col items-center gap-2 rounded-[18px] p-3"
                    >
                      <span className="h-16 w-16 animate-pulse rounded-[16px] bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]" />
                      <SkeletonBar width="70%" />
                    </div>
                  )
                }
                return (
                  <IconGridCell
                    key={item.key}
                    item={item}
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
  item,
  selected,
  selectionMode,
  onSelect,
  openItem,
  onContextMenu,
  onMenu,
  onLongPress,
}: {
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
  return (
    <div className="relative">
      <button
        type="button"
        {...handlers}
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
        className={clsx(
          'flex h-full w-full flex-col items-center gap-2 rounded-[18px] border border-transparent p-3 text-center transition',
          selected
            ? 'border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))]'
            : 'hover:border-[color:color-mix(in_srgb,var(--cp-border)_70%,transparent)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_8%,transparent)]',
          broken && 'opacity-50',
        )}
      >
        <div className="relative flex h-16 w-16 items-center justify-center rounded-[16px] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)]">
          {kindIcon(entry.kind, 28)}
          {isReferenceItem(item) ? <LinkBadge /> : null}
        </div>
        <span className="line-clamp-2 text-[12px] font-medium text-[color:var(--cp-text)]">
          {entry.name}
        </span>
        <span className="text-[10px] text-[color:var(--cp-muted)]">
          {entry.kind === 'folder' ? '—' : formatBytes(entry.sizeBytes)}
        </span>
      </button>
      {selectionMode ? (
        <span
          className={clsx(
            'pointer-events-none absolute right-1.5 top-1.5',
            selected ? 'text-[color:var(--cp-accent)]' : 'text-[color:var(--cp-muted)]',
          )}
        >
          {selected ? <CheckCircle2 size={18} /> : <Circle size={18} />}
        </span>
      ) : onMenu ? (
        <button
          type="button"
          aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
          onClick={(event) => {
            event.stopPropagation()
            onMenu()
          }}
          className="absolute right-0.5 top-0.5 flex h-8 w-8 items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,transparent)]"
        >
          <MoreVertical size={16} />
        </button>
      ) : null}
    </div>
  )
}

// ─── Mobile list (virtualized rows) ───

const MOBILE_ROW_HEIGHT = 64

function MobileListView({
  list,
  selectedKeys,
  onSelect,
  isPublic,
  selectionMode = false,
  onItemMenu,
  onLongPress,
}: {
  list: FileItemList
  selectedKeys: ReadonlySet<string>
  onSelect: (item: FileItem, modifiers?: SelectModifiers) => void
  isPublic: boolean
  selectionMode?: boolean
  onItemMenu?: (item: FileItem) => void
  onLongPress?: (item: FileItem) => void
}) {
  const parentRef = useRef<HTMLDivElement>(null)
  const count = list.totalCount ?? 0

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
                className="flex items-center gap-3 px-2.5 py-2"
                style={rowStyle}
              >
                <span className="h-12 w-12 shrink-0 animate-pulse rounded-[14px] bg-[color:color-mix(in_srgb,var(--cp-border)_45%,transparent)]" />
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
      className={clsx(
        'flex select-none items-center gap-3 rounded-[14px] px-2.5 py-2 text-left transition',
        selected
          ? 'bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))]'
          : 'hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_10%,transparent)]',
        broken && 'opacity-50',
      )}
    >
      <div className="relative flex h-12 w-12 shrink-0 items-center justify-center rounded-[14px] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_86%,transparent)]">
        {kindIcon(entry.kind, 26)}
        {isReferenceItem(item) ? <LinkBadge /> : null}
      </div>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <span className="truncate text-[13px] font-medium text-[color:var(--cp-text)]">
            {entry.name}
          </span>
          {entry.triggersActive ? (
            <span
              title="AI pipeline active"
              className="inline-flex shrink-0 items-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_26%,var(--cp-surface))] px-1.5 py-0.5 text-[9px] font-semibold text-[color:var(--cp-accent)]"
            >
              <Wand2 size={9} className="mr-1" /> AI
            </span>
          ) : null}
        </div>
        <div className="mt-0.5 truncate text-[11px] text-[color:var(--cp-muted)]">
          {meta}
          {isPublic && entry.publicUrl ? (
            <span className="ml-1 font-mono text-[color:var(--cp-accent)]">
              · {entry.publicUrl}
            </span>
          ) : null}
        </div>
      </div>
      {selectionMode ? (
        <span
          className={clsx(
            'flex h-9 w-9 shrink-0 items-center justify-center',
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
          className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,transparent)]"
        >
          <MoreVertical size={18} />
        </button>
      ) : null}
    </div>
  )
}
