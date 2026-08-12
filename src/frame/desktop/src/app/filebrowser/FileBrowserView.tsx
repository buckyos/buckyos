import { IconButton, useMediaQuery } from '@mui/material'
import clsx from 'clsx'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react'
import {
  Camera,
  ChevronRight,
  FolderPlus,
  Image as ImageIcon,
  ListChecks,
  Menu as MenuIcon,
  MoreVertical,
  PanelRightClose,
  Plus,
  RefreshCw,
  Search,
  Trash2,
  Upload as UploadIcon,
  X,
} from 'lucide-react'
import { useI18n } from '../../i18n/provider'
import {
  useMobileBackHandler,
  useMobileTitleOverride,
} from '../../desktop/windows/MobileNavContext'
import { MainContent } from './MainContent'
import type { MenuPosition, SelectModifiers } from './MainContent'
import { FileContextMenu } from './menu/FileContextMenu'
import { MobileMenuSheet } from './menu/MobileMenuSheet'
import { fileBrowserMenuRegistry } from './menu/registry'
import type { FileMenuAction, FileMenuContext, FileMenuSection } from './menu/types'
import { PreviewPanel } from './PreviewPanel'
import { SearchResultsPanel } from './SearchResultsPanel'
import { Sidebar } from './Sidebar'
import { StatusBar } from './StatusBar'
import { TopBar } from './TopBar'
import { asCollectionReader } from './data/CollectionModel'
import type { FileItem } from './data/FolderReader'
import { installFileBrowserData } from './data/install'
import { resolveReader } from './data/readerRegistry'
import { useFolderList } from './data/useFolderList'
import {
  collectionUrl,
  crumbsForUrl,
  displayPath,
  fallbackTitle,
  normalizeUrl,
  parentUrl,
  parseCollectionUrl,
} from './data/urls'
import { collectionStore } from './mock/collections'
import { defaultTabs, fileBrowserSnapshot, searchFiles } from './mock/data'
import type { BrowserTab, DfsNode, FileEntry, SortDir, SortKey, ViewMode } from './types'

installFileBrowserData()

interface HistoryState {
  back: string[]
  forward: string[]
}

interface DetachedTab {
  tab: BrowserTab
  history: HistoryState
}

/** Ad-hoc item wrapper for entry-shaped sources (search hits). */
function itemOf(entry: FileEntry): FileItem {
  return { key: entry.id, entry }
}

/** Self-contained state for one browser pane (tabs, history, list, selection, search, view). */
function useBrowserPane(initialTabs: BrowserTab[]) {
  const [tabs, setTabs] = useState<BrowserTab[]>(() =>
    initialTabs.map((tab) => ({ ...tab, path: normalizeUrl(tab.path) })),
  )
  const [activeTabId, setActiveTabId] = useState(initialTabs[0]?.id ?? '')
  const [history, setHistory] = useState<Record<string, HistoryState>>(() =>
    Object.fromEntries(initialTabs.map((tab) => [tab.id, { back: [], forward: [] }])),
  )
  const [viewMode, setViewMode] = useState<ViewMode>('list')
  const [sortKey, setSortKey] = useState<SortKey>('name')
  const [sortDir, setSortDir] = useState<SortDir>('asc')
  const [searchQuery, setSearchQuery] = useState('')

  const activeTab = tabs.find((tab) => tab.id === activeTabId) ?? tabs[0] ?? null
  const currentUrl = activeTab?.path ?? 'dfs:///home'
  const activeHistory = history[activeTabId] ?? { back: [], forward: [] }

  const list = useFolderList(currentUrl, sortKey, sortDir)

  // Entering a different location kind resets the sort to its default
  // (collections open in manual order); an invalid key always resets.
  const prevKindRef = useRef(list.capabilities.kind)
  useEffect(() => {
    const caps = list.capabilities
    if (prevKindRef.current !== caps.kind || !caps.sortKeys.includes(sortKey)) {
      prevKindRef.current = caps.kind
      setSortKey(caps.defaultSortKey)
      setSortDir(caps.defaultSortKey === 'modified' ? 'desc' : 'asc')
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list])

  // ─── Selection: keys + captured items (no global entry index lookups) ───
  const [selectedKeys, setSelectedKeys] = useState<ReadonlySet<string>>(() => new Set())
  const [selectedItemsMap, setSelectedItemsMap] = useState<Map<string, FileItem>>(
    () => new Map(),
  )
  /** Anchor item key for shift range selection. */
  const selectionAnchorRef = useRef<string | null>(null)
  /** Original path waiting to be selected after a "jump to original" navigation. */
  const pendingRevealRef = useRef<string | null>(null)

  const applySelection = useCallback(
    (keys: string[], picked?: Map<string, FileItem>) => {
      setSelectedKeys(new Set(keys))
      setSelectedItemsMap((prev) => {
        const next = new Map<string, FileItem>()
        for (const key of keys) {
          const item = picked?.get(key) ?? list.loadedItemByKey(key) ?? prev.get(key)
          if (item) next.set(key, item)
        }
        return next
      })
    },
    [list],
  )

  const clearSelection = useCallback(() => {
    selectionAnchorRef.current = null
    setSelectedKeys(new Set())
    setSelectedItemsMap(new Map())
  }, [])

  /**
   * Click-selection with desktop modifiers: plain click selects one item,
   * ctrl/cmd toggles it, shift selects the loaded-key range from the anchor.
   * Ranges across unloaded gaps are not supported (loadedKeys only).
   */
  const selectItem = useCallback(
    (item: FileItem, modifiers: SelectModifiers = {}) => {
      if (modifiers.shift) {
        const keys = list.loadedKeys()
        const anchor =
          selectionAnchorRef.current && keys.includes(selectionAnchorRef.current)
            ? selectionAnchorRef.current
            : item.key
        const from = keys.indexOf(anchor)
        const to = keys.indexOf(item.key)
        if (from !== -1 && to !== -1) {
          const [start, end] = from <= to ? [from, to] : [to, from]
          applySelection(keys.slice(start, end + 1))
          return
        }
      }
      selectionAnchorRef.current = item.key
      if (modifiers.toggle) {
        const next = new Set(selectedKeys)
        if (next.has(item.key)) next.delete(item.key)
        else next.add(item.key)
        applySelection([...next], new Map([[item.key, item]]))
        return
      }
      applySelection([item.key], new Map([[item.key, item]]))
    },
    [list, selectedKeys, applySelection],
  )

  /** Select-all semantics this iteration: all *loaded* items. */
  const selectAll = useCallback(() => {
    applySelection(list.loadedKeys())
  }, [list, applySelection])

  // Resolve a pending "reveal" once the destination listing is ready.
  useEffect(() => {
    const target = pendingRevealRef.current
    if (!target || list.status !== 'ready') return
    pendingRevealRef.current = null
    for (const key of list.loadedKeys()) {
      const item = list.loadedItemByKey(key)
      if (item && item.entry.path === target) {
        applySelection([key])
        return
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list, list.status, list.snapshot])

  const updateTab = useCallback((tabId: string, next: Partial<BrowserTab>) => {
    setTabs((prev) => prev.map((tab) => (tab.id === tabId ? { ...tab, ...next } : tab)))
  }, [])

  // Refine the provisional tab title once the reader's meta is known
  // (collection/view titles aren't derivable from the url).
  useEffect(() => {
    const title = list.meta?.title
    if (activeTab && title && activeTab.title !== title) {
      updateTab(activeTab.id, { title })
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [list, list.snapshot])

  const pushHistory = useCallback((tabId: string, url: string) => {
    setHistory((prev) => {
      const curr = prev[tabId] ?? { back: [], forward: [] }
      return {
        ...prev,
        [tabId]: { back: [...curr.back, url], forward: [] },
      }
    })
  }, [])

  const navigate = useCallback(
    (input: string, options?: { suppressHistory?: boolean }) => {
      if (!activeTab) return
      const url = normalizeUrl(input)
      if (url === currentUrl) return
      if (!options?.suppressHistory) pushHistory(activeTab.id, currentUrl)
      updateTab(activeTab.id, { path: url, title: fallbackTitle(url) })
      clearSelection()
    },
    [activeTab, currentUrl, pushHistory, updateTab, clearSelection],
  )

  /** Navigate to the parent of `originalPath` and select the item once loaded. */
  const revealOriginal = useCallback(
    (originalPath: string) => {
      const parent = originalPath.split('/').slice(0, -1).join('/') || '/'
      pendingRevealRef.current = originalPath
      navigate(parent)
    },
    [navigate],
  )

  const back = () => {
    if (!activeTab) return
    const hist = history[activeTab.id]
    if (!hist || hist.back.length === 0) return
    const previous = hist.back[hist.back.length - 1]
    setHistory((prev) => ({
      ...prev,
      [activeTab.id]: {
        back: hist.back.slice(0, -1),
        forward: [currentUrl, ...hist.forward],
      },
    }))
    updateTab(activeTab.id, { path: previous, title: fallbackTitle(previous) })
    clearSelection()
  }

  const forward = () => {
    if (!activeTab) return
    const hist = history[activeTab.id]
    if (!hist || hist.forward.length === 0) return
    const next = hist.forward[0]
    setHistory((prev) => ({
      ...prev,
      [activeTab.id]: {
        back: [...hist.back, currentUrl],
        forward: hist.forward.slice(1),
      },
    }))
    updateTab(activeTab.id, { path: next, title: fallbackTitle(next) })
    clearSelection()
  }

  const goUp = () => {
    const up = parentUrl(currentUrl)
    if (up) navigate(up)
  }

  /** Append a tab (optionally with its history) and make it active. */
  const adoptTab = useCallback(
    (tab: BrowserTab, tabHistory?: HistoryState) => {
      const normalized = { ...tab, path: normalizeUrl(tab.path) }
      setTabs((prev) => [...prev, normalized])
      setHistory((prev) => ({ ...prev, [tab.id]: tabHistory ?? { back: [], forward: [] } }))
      setActiveTabId(tab.id)
      clearSelection()
      setSearchQuery('')
    },
    [clearSelection],
  )

  /** Remove a tab and hand back its data so it can be moved or remembered. */
  const detachTab = useCallback(
    (id: string): DetachedTab | null => {
      const tab = tabs.find((item) => item.id === id)
      if (!tab) return null
      const tabHistory = history[id] ?? { back: [], forward: [] }
      setHistory((prev) => {
        const next = { ...prev }
        delete next[id]
        return next
      })
      setTabs((prev) => {
        const closingIndex = prev.findIndex((item) => item.id === id)
        const next = prev.filter((item) => item.id !== id)
        if (id === activeTabId && next.length) {
          setActiveTabId(next[Math.min(closingIndex, next.length - 1)].id)
        }
        return next
      })
      if (id === activeTabId) {
        clearSelection()
        setSearchQuery('')
      }
      return { tab, history: tabHistory }
    },
    [tabs, history, activeTabId, clearSelection],
  )

  return {
    tabs,
    activeTabId,
    setActiveTabId,
    activeTab,
    currentUrl,
    activeHistory,
    list,
    viewMode,
    setViewMode,
    sortKey,
    setSortKey,
    sortDir,
    setSortDir,
    selectedKeys,
    selectedItemsMap,
    applySelection,
    selectItem,
    selectAll,
    clearSelection,
    searchQuery,
    setSearchQuery,
    navigate,
    revealOriginal,
    back,
    forward,
    goUp,
    updateTab,
    adoptTab,
    detachTab,
  }
}

type BrowserPane = ReturnType<typeof useBrowserPane>

export function FileBrowserView() {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 900px)')
  // Tailwind xl — the preview sidebar only exists at this width, so the
  // expand control in the status bar should follow the same gate.
  const isXl = useMediaQuery('(min-width: 1280px)')

  const left = useBrowserPane(defaultTabs)
  const right = useBrowserPane([])
  const [closedTabs, setClosedTabs] = useState<BrowserTab[]>([])
  const [focusedSide, setFocusedSide] = useState<'left' | 'right'>('left')

  const [advancedMode, setAdvancedMode] = useState(false)
  const [toast, setToast] = useState<string | null>(null)
  const [previewCollapsed, setPreviewCollapsed] = useState(false)
  /** Toolbar cut/copy clipboard, shared by both panes (mock — entries only). */
  const [clipboard, setClipboard] = useState<{ entries: FileEntry[]; mode: 'cut' | 'copy' } | null>(
    null,
  )

  // Live collection list (sidebar + "Add to Collection" submenu).
  useSyncExternalStore(collectionStore.subscribeList, collectionStore.listSnapshot)
  const collections = collectionStore.list()

  // Mobile-only panel states
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false)
  const [mobilePreviewOpen, setMobilePreviewOpen] = useState(false)
  const [mobileUploadOpen, setMobileUploadOpen] = useState(false)
  /** Google-Drive-style multi-select mode (entered by long-press). */
  const [mobileSelectMode, setMobileSelectMode] = useState(false)
  /** Bottom-sheet menu — same registry sections as the desktop popup. */
  const [mobileMenu, setMobileMenu] = useState<{
    title?: string
    context: FileMenuContext
    sections: FileMenuSection[]
  } | null>(null)

  // Deselecting the last item leaves selection mode.
  useEffect(() => {
    if (mobileSelectMode && left.selectedKeys.size === 0) setMobileSelectMode(false)
  }, [mobileSelectMode, left.selectedKeys])

  // The split layout is desktop-only; the right pane exists while it holds tabs.
  const splitActive = !isMobile && right.tabs.length > 0
  const focusedIsRight = splitActive && focusedSide === 'right'
  const focusedPane = focusedIsRight ? right : left

  const rememberClosedTab = (tab: BrowserTab) => {
    setClosedTabs((prev) => [tab, ...prev].slice(0, 10))
  }

  const handleNewTab = () => {
    left.adoptTab({ id: `tab-${Date.now()}`, title: 'Home', path: '/home' })
    setFocusedSide('left')
  }

  const handleCloseLeftTab = (id: string) => {
    if (left.tabs.length <= 1) return
    const detached = left.detachTab(id)
    if (detached) rememberClosedTab(detached.tab)
  }

  const handleCloseRightTab = (id: string) => {
    const detached = right.detachTab(id)
    if (detached) rememberClosedTab(detached.tab)
  }

  const handleSendToRight = (id: string) => {
    if (left.tabs.length <= 1) return
    const detached = left.detachTab(id)
    if (!detached) return
    right.adoptTab(detached.tab, detached.history)
    setFocusedSide('right')
  }

  const handleRestoreClosedTab = (tab: BrowserTab) => {
    setClosedTabs((prev) => prev.filter((item) => item.id !== tab.id))
    left.adoptTab({ ...tab, id: `tab-${Date.now()}` })
    setFocusedSide('left')
  }

  const showToast = (message: string) => {
    setToast(message)
    window.setTimeout(() => setToast(null), 2000)
  }

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      showToast(t('filebrowser.toast.copied', 'Copied to clipboard'))
    } catch {
      showToast(t('filebrowser.toast.copyFailed', 'Copy failed'))
    }
  }

  const leftSelectedItems = useMemo(
    () => [...left.selectedItemsMap.values()],
    [left.selectedItemsMap],
  )
  const rightSelectedItems = useMemo(
    () => [...right.selectedItemsMap.values()],
    [right.selectedItemsMap],
  )
  // The preview panel & detail status only make sense for a single selection.
  const leftSelectedItem = leftSelectedItems.length === 1 ? leftSelectedItems[0] : null
  const rightSelectedItem = rightSelectedItems.length === 1 ? rightSelectedItems[0] : null

  const leftSearchHits = useMemo(
    () => (left.searchQuery.trim() ? searchFiles(left.searchQuery) : []),
    [left.searchQuery],
  )
  const rightSearchHits = useMemo(
    () => (right.searchQuery.trim() ? searchFiles(right.searchQuery) : []),
    [right.searchQuery],
  )

  const handleOpenEntry = (item: FileItem) => {
    left.applySelection([item.key], new Map([[item.key, item]]))
    if (isMobile) setMobilePreviewOpen(true)
  }

  const handleOpenFolder = (url: string) => {
    left.navigate(url)
    setMobileSidebarOpen(false)
  }

  // ─── Mobile interactions: tap opens, long-press enters selection mode ───

  const handleMobileItemTap = (item: FileItem) => {
    if (mobileSelectMode) {
      left.selectItem(item, { toggle: true })
      return
    }
    if (item.entry.kind === 'folder') {
      handleOpenFolder(item.entry.path)
      return
    }
    left.applySelection([item.key], new Map([[item.key, item]]))
    setMobilePreviewOpen(true)
  }

  const handleMobileLongPress = (item: FileItem) => {
    if (mobileSelectMode) {
      left.selectItem(item, { toggle: true })
      return
    }
    setMobileSelectMode(true)
    left.selectItem(item)
  }

  const exitMobileSelectMode = () => {
    setMobileSelectMode(false)
    left.clearSelection()
  }

  // ─── Collection mutations go through the reader interface (future RPC shape) ───

  const withCollection = async (
    url: string,
    fn: (reader: NonNullable<ReturnType<typeof asCollectionReader>>) => Promise<void>,
  ) => {
    const reader = resolveReader(url)
    const collection = asCollectionReader(reader)
    if (!collection) {
      reader.dispose()
      return
    }
    try {
      await fn(collection)
    } finally {
      reader.dispose()
    }
  }

  const addEntriesToCollection = async (collectionId: string, entries: FileEntry[]) => {
    const targets = entries.map((entry) => normalizeUrl(entry.path))
    if (!targets.length) return
    await withCollection(collectionUrl(collectionId), (reader) =>
      reader.addReferences(targets),
    )
    const title = collectionStore.get(collectionId)?.title ?? collectionId
    showToast(
      t('filebrowser.toast.addedToCollection', 'Added {{count}} reference(s) to {{title}}', {
        count: targets.length,
        title,
      }),
    )
  }

  const promptNewCollection = (): string | null => {
    const name = window.prompt(
      t('filebrowser.prompt.newCollection', 'Name for the new collection:'),
      '',
    )
    if (!name?.trim()) return null
    return collectionStore.create(name.trim())
  }

  // ─── Context menu (desktop) ───

  /** Quick "Move to" targets: DFS roots and their first-level folders. */
  const moveTargets = useMemo(() => {
    const targets: { label: string; path: string }[] = []
    const visit = (nodes: DfsNode[], depth: number) => {
      for (const node of nodes) {
        // Only real storage destinations qualify (the Shared root is a collection now).
        if (!node.path.startsWith('/')) continue
        targets.push({ label: node.path, path: node.path })
        if (node.children && depth < 1) visit(node.children, depth + 1)
      }
    }
    visit(fileBrowserSnapshot.dfsRoots, 0)
    return targets
  }, [])

  const [contextMenu, setContextMenu] = useState<{
    side: 'left' | 'right'
    position: MenuPosition
    context: FileMenuContext
    sections: FileMenuSection[]
  } | null>(null)

  const paneFor = (side: 'left' | 'right') => (side === 'right' ? right : left)

  // ─── Toolbar (desktop) ───

  const cutEntries = (entries: FileEntry[]) => {
    if (!entries.length) return
    setClipboard({ entries, mode: 'cut' })
    showToast(t('filebrowser.toast.cut', 'Cut {{count}} item(s)', { count: entries.length }))
  }

  const copyEntries = (entries: FileEntry[]) => {
    if (!entries.length) return
    setClipboard({ entries, mode: 'copy' })
    showToast(
      t('filebrowser.toast.copyItems', 'Copied {{count}} item(s)', { count: entries.length }),
    )
  }

  const pasteInto = (pane: BrowserPane) => {
    if (!clipboard) return
    const capabilities = pane.list.capabilities
    if (capabilities.acceptsReferences) {
      // Pasting into a collection builds references — no storage is touched.
      const collection = parseCollectionUrl(pane.currentUrl)
      if (collection) void addEntriesToCollection(collection.collectionId, clipboard.entries)
      if (clipboard.mode === 'cut') setClipboard(null)
      return
    }
    if (!capabilities.acceptsContent) return
    showToast(
      t('filebrowser.toast.paste', 'Pasted {{count}} item(s) to {{target}} (mock)', {
        count: clipboard.entries.length,
        target: displayPath(pane.currentUrl),
      }),
    )
    if (clipboard.mode === 'cut') setClipboard(null)
  }

  const removeFromCollection = async (pane: BrowserPane, keys: string[]) => {
    if (!keys.length) return
    await withCollection(pane.currentUrl, (reader) => reader.removeItems(keys))
    pane.clearSelection()
    showToast(
      t('filebrowser.toast.removedRefs', 'Removed {{count}} reference(s)', {
        count: keys.length,
      }),
    )
  }

  const moveItems = async (pane: BrowserPane, items: FileItem[], delta: -1 | 1) => {
    const withRef = items.filter((item) => item.ref)
    if (!withRef.length) return
    const minIndex = Math.min(...withRef.map((item) => item.ref!.orderIndex))
    const toIndex = delta === -1 ? minIndex - 1 : minIndex + 1
    await withCollection(pane.currentUrl, (reader) =>
      reader.reorder(
        withRef.map((item) => item.key),
        Math.max(0, toIndex),
      ),
    )
  }

  /** Everything the TopBar toolbar row needs, resolved per pane. */
  const toolbarPropsFor = (side: 'left' | 'right') => {
    const pane = paneFor(side)
    const selected = (side === 'right' ? rightSelectedItems : leftSelectedItems).map(
      (item) => item.entry,
    )
    const selectedKeys = [...(side === 'right' ? right : left).selectedKeys]
    const capabilities = pane.list.capabilities
    return {
      selectedCount: selected.length,
      capabilities,
      canPaste: !!clipboard,
      onCut: () => cutEntries(selected),
      onCopy: () => copyEntries(selected),
      onPaste: () => pasteInto(pane),
      onRename: () =>
        showToast(
          t('filebrowser.toast.rename', 'Rename "{{name}}" (mock)', {
            name: selected[0]?.name ?? '',
          }),
        ),
      onDelete: () => {
        if (capabilities.removal === 'remove-ref') {
          void removeFromCollection(pane, selectedKeys)
          return
        }
        showToast(
          t('filebrowser.toast.delete', 'Deleted {{count}} item(s) (mock)', {
            count: selected.length,
          }),
        )
        pane.clearSelection()
      },
      onSettings: () => showToast(`${t('filebrowser.actions.settings', 'Settings')} (mock)`),
      moveTargets,
      onMoveTo: (path: string) =>
        showToast(
          t('filebrowser.toast.move', 'Move {{count}} item(s) to {{target}} (mock)', {
            count: selected.length,
            target: path,
          }),
        ),
      sortKey: pane.sortKey,
      sortDir: pane.sortDir,
      onSortChange: (key: SortKey, dir: SortDir) => {
        pane.setSortKey(key)
        pane.setSortDir(dir)
      },
      onUpload: () => showToast(`${t('filebrowser.actions.upload', 'Upload')} (mock)`),
      onNewFolder: () => showToast(`${t('filebrowser.actions.newFolder', 'New folder')} (mock)`),
      onNewFile: () =>
        showToast(`${t('filebrowser.actions.newTextFile', 'New text file')} (mock)`),
    }
  }

  const openMenu = (side: 'left' | 'right', position: MenuPosition, item?: FileItem) => {
    const pane = paneFor(side)
    let items: FileItem[] = []
    if (item) {
      if (pane.selectedKeys.has(item.key)) {
        items = [...pane.selectedItemsMap.values()]
      } else {
        // Right-clicking outside the current selection re-anchors it to the item.
        pane.applySelection([item.key], new Map([[item.key, item]]))
        items = [item]
      }
    }
    const context: FileMenuContext = {
      target: item ? (items.length > 1 ? 'selection' : 'item') : 'view',
      items,
      entries: items.map((entry) => entry.entry),
      currentUrl: pane.currentUrl,
      viewMode: pane.viewMode,
      capabilities: pane.list.capabilities,
      sortKey: pane.sortKey,
      collections: collections.map((collection) => ({
        id: collection.id,
        title: collection.title,
      })),
      moveTargets,
      pane: {
        canOpenInNewTab: true,
        canOpenInRightPane: side === 'left',
      },
    }
    setContextMenu({
      side,
      position,
      context,
      sections: fileBrowserMenuRegistry.build(context),
    })
  }

  /** Dispatch a menu command — shared by the desktop popup and the mobile sheet. */
  const runMenuAction = (
    pane: BrowserPane,
    context: FileMenuContext,
    action: FileMenuAction,
  ) => {
    const items = context.items
    const first = items[0]
    const entries = context.entries
    const count = entries.length
    switch (action.command) {
      case 'open':
        if (!first) break
        if (first.entry.kind === 'folder') {
          // Group entries carry collection:// paths; folder refs carry their
          // original dfs path — navigation normalizes both.
          pane.navigate(first.entry.path)
        } else {
          pane.applySelection([first.key], new Map([[first.key, first]]))
          if (isMobile) setMobilePreviewOpen(true)
          else setPreviewCollapsed(false)
        }
        break
      case 'open-new-tab':
        if (first?.entry.kind === 'folder') {
          pane.adoptTab({
            id: `tab-${Date.now()}`,
            title: first.entry.name,
            path: first.entry.path,
          })
        }
        break
      case 'open-right':
        if (first?.entry.kind === 'folder') {
          right.adoptTab({
            id: `tab-${Date.now()}`,
            title: first.entry.name,
            path: first.entry.path,
          })
          setFocusedSide('right')
        }
        break
      case 'copy-path':
        copyText(
          entries.length
            ? entries.map((item) => item.path).join('\n')
            : displayPath(context.currentUrl),
        )
        break
      case 'copy-ref-path':
        if (first?.ref) copyText(first.ref.refPath)
        break
      case 'copy-public-url':
        if (first?.entry.publicUrl) copyText(first.entry.publicUrl)
        break
      case 'jump-to-original': {
        if (!first) break
        const original = first.entry.link
          ? displayPath(first.entry.link.targetUrl)
          : first.entry.path
        pane.revealOriginal(original)
        break
      }
      case 'add-to-collection': {
        const collectionId = String(action.args?.collectionId ?? '')
        if (collectionId) void addEntriesToCollection(collectionId, entries)
        break
      }
      case 'new-collection': {
        const id = promptNewCollection()
        if (id && entries.length) void addEntriesToCollection(id, entries)
        break
      }
      case 'remove-from-collection':
      case 'remove-broken':
        void removeFromCollection(
          pane,
          items.map((item) => item.key),
        )
        break
      case 'move-item-up':
        void moveItems(pane, items, -1)
        break
      case 'move-item-down':
        void moveItems(pane, items, 1)
        break
      case 'new-group': {
        const name = window.prompt(
          t('filebrowser.prompt.newGroup', 'Name for the new group:'),
          '',
        )
        if (name?.trim()) {
          void withCollection(pane.currentUrl, (reader) => reader.createGroup(name.trim()))
        }
        break
      }
      case 'download':
        showToast(
          t('filebrowser.toast.download', 'Downloading {{count}} item(s) (mock)', { count }),
        )
        break
      case 'share':
        showToast(
          t('filebrowser.toast.share', 'Share "{{name}}" (mock)', {
            name: first?.entry.name ?? '',
          }),
        )
        break
      case 'rename':
        showToast(
          t('filebrowser.toast.rename', 'Rename "{{name}}" (mock)', {
            name: first?.entry.name ?? '',
          }),
        )
        break
      case 'move-to':
        showToast(
          t('filebrowser.toast.move', 'Move {{count}} item(s) to {{target}} (mock)', {
            count,
            target: String(action.args?.path ?? ''),
          }),
        )
        break
      case 'delete':
        showToast(t('filebrowser.toast.delete', 'Deleted {{count}} item(s) (mock)', { count }))
        pane.clearSelection()
        break
      case 'new-folder':
        showToast(`${t('filebrowser.actions.newFolder', 'New folder')} (mock)`)
        break
      case 'upload':
        showToast(`${t('filebrowser.actions.upload', 'Upload')} (mock)`)
        break
      case 'refresh':
        pane.list.reload()
        break
      case 'select-all':
        pane.selectAll()
        if (isMobile) setMobileSelectMode(true)
        break
      case 'view-list':
        pane.setViewMode('list')
        break
      case 'view-icon':
        pane.setViewMode('icon')
        break
      default:
        showToast(`${action.command} (mock)`)
    }
  }

  const handleMenuAction = (action: FileMenuAction) => {
    if (!contextMenu) return
    runMenuAction(paneFor(contextMenu.side), contextMenu.context, action)
  }

  // ─── Mobile menus: same registry data rendered as a bottom sheet ───

  /** 0 items → view menu, 1 → item menu, 2+ → selection menu. */
  const openMobileMenu = (items: FileItem[]) => {
    const context: FileMenuContext = {
      target: items.length === 0 ? 'view' : items.length > 1 ? 'selection' : 'item',
      items,
      entries: items.map((item) => item.entry),
      currentUrl: left.currentUrl,
      viewMode: left.viewMode,
      capabilities: left.list.capabilities,
      sortKey: left.sortKey,
      collections: collections.map((collection) => ({
        id: collection.id,
        title: collection.title,
      })),
      moveTargets,
      pane: { canOpenInNewTab: false, canOpenInRightPane: false },
    }
    setMobileMenu({
      title:
        items.length === 1
          ? items[0].entry.name
          : items.length > 1
            ? t('filebrowser.mobile.selectedCount', '{{count}} selected', {
                count: items.length,
              })
            : undefined,
      context,
      sections: fileBrowserMenuRegistry.build(context),
    })
  }

  const handleMobileMenuAction = (action: FileMenuAction) => {
    if (!mobileMenu) return
    runMenuAction(left, mobileMenu.context, action)
  }

  const handleMobileDelete = () => {
    const keys = [...left.selectedKeys]
    if (!keys.length) return
    if (left.list.capabilities.removal === 'remove-ref') {
      void removeFromCollection(left, keys)
      return
    }
    showToast(
      t('filebrowser.toast.delete', 'Deleted {{count}} item(s) (mock)', { count: keys.length }),
    )
    left.clearSelection()
  }

  const handleCreateCollection = () => {
    const id = promptNewCollection()
    if (id) left.navigate(collectionUrl(id))
  }

  const leftSearchActive = !!left.searchQuery.trim()
  const rightSearchActive = !!right.searchQuery.trim()

  const focusedSelectedItem = focusedIsRight ? rightSelectedItem : leftSelectedItem

  const mobileTitleText = leftSelectedItem?.entry.name ?? left.activeTab?.title ?? 'root'
  const mobileSubtitleText =
    leftSelectedItem?.entry.summary ??
    left.list.meta?.description ??
    (displayPath(left.currentUrl) === '/'
      ? t('filebrowser.mobile.rootHint', 'Root directory')
      : displayPath(left.currentUrl))

  const mobileTitleOverride = useMemo(
    () => (isMobile ? { title: mobileTitleText, subtitle: mobileSubtitleText } : null),
    [isMobile, mobileTitleText, mobileSubtitleText],
  )
  useMobileTitleOverride(mobileTitleOverride)

  const canMobileBack = isMobile && left.activeHistory.back.length > 0
  useMobileBackHandler(canMobileBack ? left.back : null)

  // ─── Mobile layout ───
  if (isMobile) {
    const crumbs = crumbsForUrl(left.currentUrl, left.list.meta?.title)

    return (
      <div className="relative flex h-full w-full flex-col overflow-hidden" style={{ background: 'var(--cp-bg)' }}>
        {/* Operations bar: drawer toggle + search + menu — or the selection bar */}
        {mobileSelectMode ? (
          <div className="flex items-center gap-1 px-3 pt-2 pb-1">
            <IconButton
              size="small"
              onClick={exitMobileSelectMode}
              aria-label={t('filebrowser.mobile.exitSelect', 'Exit selection')}
            >
              <X size={18} />
            </IconButton>
            <span
              className="min-w-0 flex-1 truncate text-[15px] font-semibold"
              style={{ color: 'var(--cp-text)' }}
            >
              {t('filebrowser.mobile.selectedCount', '{{count}} selected', {
                count: left.selectedKeys.size,
              })}
            </span>
            <IconButton
              size="small"
              onClick={left.selectAll}
              aria-label={t('filebrowser.menu.selectAll', 'Select all')}
            >
              <ListChecks size={18} />
            </IconButton>
            {left.list.capabilities.removal !== null ? (
              <IconButton
                size="small"
                onClick={handleMobileDelete}
                aria-label={
                  left.list.capabilities.removal === 'remove-ref'
                    ? t('filebrowser.actions.removeFromCollection', 'Remove from collection')
                    : t('filebrowser.actions.delete', 'Delete')
                }
              >
                <Trash2 size={18} />
              </IconButton>
            ) : null}
            <IconButton
              size="small"
              onClick={() => openMobileMenu([...left.selectedItemsMap.values()])}
              aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
            >
              <MoreVertical size={18} />
            </IconButton>
          </div>
        ) : (
          <div className="flex items-center gap-2 px-3 pt-2 pb-1">
            <IconButton
              size="small"
              onClick={() => setMobileSidebarOpen((v) => !v)}
              aria-label={t('filebrowser.mobile.places', 'Places')}
              className={clsx(
                mobileSidebarOpen &&
                  '!bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_28%,var(--cp-surface))] !text-[color:var(--cp-text)]',
              )}
            >
              <MenuIcon size={16} />
            </IconButton>
            <div className="relative flex min-w-0 flex-1 items-center gap-1 rounded-full border border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface-2)_88%,transparent)] px-2 py-1">
              <Search size={14} className="ml-1 shrink-0 text-[color:var(--cp-muted)]" />
              <input
                type="text"
                value={left.searchQuery}
                onChange={(event) => left.setSearchQuery(event.target.value)}
                placeholder={t(
                  'filebrowser.topbar.searchPlaceholder',
                  'Search across files, folders, AI summaries…',
                )}
                className="min-w-0 flex-1 bg-transparent text-xs outline-none placeholder:text-[color:var(--cp-muted)]"
                style={{ color: 'var(--cp-text)' }}
              />
              {left.searchQuery ? (
                <IconButton
                  size="small"
                  onClick={() => left.setSearchQuery('')}
                  aria-label={t('common.close', 'Close')}
                >
                  <X size={12} />
                </IconButton>
              ) : null}
            </div>
            <IconButton
              size="small"
              onClick={() => openMobileMenu([])}
              aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
            >
              <MoreVertical size={18} />
            </IconButton>
          </div>
        )}

        {/* Address bar: path crumbs + refresh on the right */}
        <div className="flex items-center gap-2 px-3 pb-2 pt-1">
          <div className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto text-[13px] text-[color:var(--cp-muted)]">
            {crumbs.map((crumb, idx) => (
              <div key={crumb.url} className="flex shrink-0 items-center gap-1">
                <button
                  type="button"
                  className={clsx(
                    'truncate rounded-md px-1.5 py-1',
                    idx === crumbs.length - 1 && 'font-semibold text-[color:var(--cp-text)]',
                  )}
                  onClick={() => left.navigate(crumb.url)}
                >
                  {crumb.label}
                </button>
                {idx < crumbs.length - 1 ? (
                  <ChevronRight size={13} className="opacity-60" />
                ) : null}
              </div>
            ))}
          </div>
          <button
            type="button"
            onClick={() => left.list.reload()}
            aria-label={t('filebrowser.topbar.refresh', 'Refresh')}
            className="shrink-0 p-1 text-[color:var(--cp-muted)] hover:text-[color:var(--cp-text)]"
          >
            <RefreshCw size={16} />
          </button>
        </div>

        <div className="flex-1 overflow-hidden">
          {leftSearchActive ? (
            <SearchResultsPanel
              hits={leftSearchHits}
              query={left.searchQuery}
              onSelect={(entry) => {
                if (entry.kind === 'folder') left.setSearchQuery('')
                handleMobileItemTap(itemOf(entry))
              }}
            />
          ) : (
            <MainContent
              list={left.list}
              viewMode={left.viewMode}
              selectedKeys={left.selectedKeys}
              onSelect={handleMobileItemTap}
              onOpenFolder={handleOpenFolder}
              currentUrl={left.currentUrl}
              isMobile
              selectionMode={mobileSelectMode}
              onItemMenu={(item) => openMobileMenu([item])}
              onLongPress={handleMobileLongPress}
            />
          )}
        </div>

        {/* Floating action button — upload (hidden while selecting) */}
        {!mobileSelectMode ? (
          <button
            type="button"
            onClick={() => setMobileUploadOpen(true)}
            aria-label={t('filebrowser.actions.upload', 'Upload')}
            className="absolute bottom-5 right-5 z-20 flex h-14 w-14 items-center justify-center rounded-full text-white shadow-[0_10px_28px_rgba(0,0,0,0.22)] transition active:scale-95"
            style={{ background: 'var(--cp-accent)' }}
          >
            <Plus size={26} />
          </button>
        ) : null}

        {/* Upload action sheet */}
        {mobileUploadOpen ? (
          <div className="absolute inset-0 z-40 flex items-end">
            <div
              className="absolute inset-0 bg-black/50"
              onClick={() => setMobileUploadOpen(false)}
            />
            <div
              className="relative w-full rounded-t-[28px] border-t border-[color:var(--cp-border)] pb-5 pt-2"
              style={{ background: 'var(--cp-surface)' }}
            >
              <div className="mx-auto mb-2 h-1 w-10 rounded-full bg-[color:var(--cp-border)]" />
              <div className="px-5 pb-1 text-[13px] font-semibold text-[color:var(--cp-text)]">
                {t('filebrowser.upload.title', 'Add to this folder')}
              </div>
              <div className="grid grid-cols-4 gap-1 px-3 py-3">
                {[
                  {
                    key: 'files',
                    icon: <UploadIcon size={22} />,
                    label: t('filebrowser.upload.files', 'Files'),
                  },
                  {
                    key: 'photos',
                    icon: <ImageIcon size={22} />,
                    label: t('filebrowser.upload.photos', 'Photos'),
                  },
                  {
                    key: 'camera',
                    icon: <Camera size={22} />,
                    label: t('filebrowser.upload.camera', 'Camera'),
                  },
                  {
                    key: 'folder',
                    icon: <FolderPlus size={22} />,
                    label: t('filebrowser.upload.newFolder', 'New folder'),
                  },
                ].map((action) => (
                  <button
                    key={action.key}
                    type="button"
                    onClick={() => {
                      setMobileUploadOpen(false)
                      showToast(`${action.label} (mock)`)
                    }}
                    className="flex flex-col items-center gap-1.5 rounded-[16px] p-2 text-[11px] text-[color:var(--cp-text)] hover:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_18%,transparent)]"
                  >
                    <div
                      className="flex h-12 w-12 items-center justify-center rounded-full text-[color:var(--cp-accent)]"
                      style={{
                        background:
                          'color-mix(in srgb, var(--cp-accent-soft) 32%, var(--cp-surface))',
                      }}
                    >
                      {action.icon}
                    </div>
                    <span className="text-center leading-tight">{action.label}</span>
                  </button>
                ))}
              </div>
            </div>
          </div>
        ) : null}

        {/* Sidebar drawer */}
        {mobileSidebarOpen ? (
          <div className="absolute inset-0 z-40 flex">
            <div
              className="absolute inset-0 bg-black/50"
              onClick={() => setMobileSidebarOpen(false)}
            />
            <div
              className="relative flex h-full w-2/3 flex-col gap-3 border-r border-[color:var(--cp-border)] p-3"
              style={{ background: 'var(--cp-surface)' }}
            >
              <Sidebar
                dfsRoots={fileBrowserSnapshot.dfsRoots}
                devices={fileBrowserSnapshot.devices}
                topics={fileBrowserSnapshot.topics}
                collections={collections}
                activeUrl={left.currentUrl}
                advancedMode={advancedMode}
                onToggleAdvanced={setAdvancedMode}
                onNavigate={left.navigate}
                onCreateCollection={handleCreateCollection}
                onAfterNavigate={() => setMobileSidebarOpen(false)}
                compact
              />
            </div>
          </div>
        ) : null}

        {/* Preview bottom sheet */}
        {mobilePreviewOpen && leftSelectedItem ? (
          <div className="absolute inset-0 z-30 flex items-end">
            <div
              className="absolute inset-0 bg-black/40"
              onClick={() => setMobilePreviewOpen(false)}
            />
            <div
              className="relative flex h-2/3 w-full flex-col rounded-t-[28px] border-t border-[color:var(--cp-border)]"
              style={{ background: 'var(--cp-surface)' }}
            >
              <div className="flex items-center justify-between px-4 py-2">
                <span className="shell-kicker">
                  {t('filebrowser.mobile.preview', 'Preview')}
                </span>
                <button
                  type="button"
                  onClick={() => setMobilePreviewOpen(false)}
                  className="rounded-full border border-[color:var(--cp-border)] px-2.5 py-1 text-xs"
                >
                  {t('common.close', 'Close')}
                </button>
              </div>
              <div className="flex-1 overflow-hidden">
                <PreviewPanel
                  item={leftSelectedItem}
                  topics={fileBrowserSnapshot.topics}
                  onJumpToTopic={(id) => {
                    left.navigate(`view://topic/${id}`)
                    setMobilePreviewOpen(false)
                  }}
                  onJumpToPath={(path) => {
                    left.navigate(path)
                    setMobilePreviewOpen(false)
                  }}
                  embedded
                />
              </div>
            </div>
          </div>
        ) : null}

        {/* Context menu bottom sheet (item / selection / view) */}
        <MobileMenuSheet
          open={Boolean(mobileMenu)}
          title={mobileMenu?.title}
          sections={mobileMenu?.sections ?? []}
          onInvoke={handleMobileMenuAction}
          onClose={() => setMobileMenu(null)}
        />

        {toast ? (
          <div className="pointer-events-none absolute bottom-14 left-1/2 -translate-x-1/2 rounded-full bg-black/80 px-3 py-1.5 text-xs text-white">
            {toast}
          </div>
        ) : null}
      </div>
    )
  }

  // ─── Desktop layout ───
  // VS Code-like shell: full-height nav tree | one or two self-contained panes
  // (tabs + address row + toolbar + content + status bar) | collapsible sidebar.
  return (
    <div
      className="relative flex h-full w-full overflow-hidden"
      style={{ background: 'var(--cp-bg)' }}
    >
      <aside
        className="hidden w-[260px] shrink-0 flex-col overflow-hidden border-r border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_82%,transparent)] px-2 pt-2 md:flex"
        onMouseDownCapture={() => setFocusedSide('left')}
      >
        <Sidebar
          dfsRoots={fileBrowserSnapshot.dfsRoots}
          devices={fileBrowserSnapshot.devices}
          topics={fileBrowserSnapshot.topics}
          collections={collections}
          activeUrl={left.currentUrl}
          advancedMode={advancedMode}
          onToggleAdvanced={setAdvancedMode}
          onNavigate={left.navigate}
          onCreateCollection={handleCreateCollection}
        />
      </aside>

      <main
        className="flex min-w-0 flex-1 flex-col"
        onMouseDownCapture={() => setFocusedSide('left')}
      >
        <TopBar
          tabs={left.tabs}
          activeTabId={left.activeTabId}
          onSelectTab={left.setActiveTabId}
          onCloseTab={handleCloseLeftTab}
          onNewTab={handleNewTab}
          closedTabs={closedTabs}
          onRestoreClosedTab={handleRestoreClosedTab}
          currentPath={left.currentUrl}
          locationTitle={left.list.meta?.title}
          onNavigate={left.navigate}
          onBack={left.back}
          onForward={left.forward}
          onUp={left.goUp}
          canBack={left.activeHistory.back.length > 0}
          canForward={left.activeHistory.forward.length > 0}
          canUp={parentUrl(left.currentUrl) !== null}
          viewMode={left.viewMode}
          onViewModeChange={left.setViewMode}
          searchQuery={left.searchQuery}
          onSearchChange={left.setSearchQuery}
          onCopyPath={() => copyText(displayPath(left.currentUrl))}
          onSendTabToRight={handleSendToRight}
          canSendToRight={left.tabs.length > 1}
          {...toolbarPropsFor('left')}
        />
        <div className="min-h-0 flex-1 overflow-hidden">
          {leftSearchActive ? (
            <SearchResultsPanel
              hits={leftSearchHits}
              query={left.searchQuery}
              onSelect={(entry) => handleOpenEntry(itemOf(entry))}
            />
          ) : (
            <MainContent
              list={left.list}
              viewMode={left.viewMode}
              selectedKeys={left.selectedKeys}
              onSelect={(item, modifiers) => left.selectItem(item, modifiers)}
              onOpenFolder={handleOpenFolder}
              onItemContextMenu={(item, position) => openMenu('left', position, item)}
              onViewContextMenu={(position) => openMenu('left', position)}
              onClearSelection={left.clearSelection}
              currentUrl={left.currentUrl}
            />
          )}
        </div>
        <StatusBar
          currentUrl={left.currentUrl}
          totalCount={left.list.totalCount}
          selectedItems={leftSelectedItems}
          onCopy={copyText}
          onExpandSidebar={
            !splitActive && previewCollapsed && isXl
              ? () => setPreviewCollapsed(false)
              : undefined
          }
        />
      </main>

      {splitActive ? (
        <section
          className="flex min-w-0 flex-1 flex-col border-l border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)]"
          onMouseDownCapture={() => setFocusedSide('right')}
        >
          <TopBar
            tabs={right.tabs}
            activeTabId={right.activeTabId}
            onSelectTab={right.setActiveTabId}
            onCloseTab={handleCloseRightTab}
            showTabControls={false}
            allowCloseLast
            currentPath={right.currentUrl}
            locationTitle={right.list.meta?.title}
            onNavigate={right.navigate}
            onBack={right.back}
            onForward={right.forward}
            onUp={right.goUp}
            canBack={right.activeHistory.back.length > 0}
            canForward={right.activeHistory.forward.length > 0}
            canUp={parentUrl(right.currentUrl) !== null}
            viewMode={right.viewMode}
            onViewModeChange={right.setViewMode}
            searchQuery={right.searchQuery}
            onSearchChange={right.setSearchQuery}
            onCopyPath={() => copyText(displayPath(right.currentUrl))}
            {...toolbarPropsFor('right')}
          />
          <div className="min-h-0 flex-1 overflow-hidden">
            {rightSearchActive ? (
              <SearchResultsPanel
                hits={rightSearchHits}
                query={right.searchQuery}
                onSelect={(entry) =>
                  right.applySelection([entry.id], new Map([[entry.id, itemOf(entry)]]))
                }
              />
            ) : (
              <MainContent
                list={right.list}
                viewMode={right.viewMode}
                selectedKeys={right.selectedKeys}
                onSelect={(item, modifiers) => right.selectItem(item, modifiers)}
                onOpenFolder={(url) => right.navigate(url)}
                onItemContextMenu={(item, position) => openMenu('right', position, item)}
                onViewContextMenu={(position) => openMenu('right', position)}
                onClearSelection={right.clearSelection}
                currentUrl={right.currentUrl}
              />
            )}
          </div>
          <StatusBar
            currentUrl={right.currentUrl}
            totalCount={right.list.totalCount}
            selectedItems={rightSelectedItems}
            onCopy={copyText}
            onExpandSidebar={
              previewCollapsed && isXl ? () => setPreviewCollapsed(false) : undefined
            }
          />
        </section>
      ) : null}

      {!previewCollapsed ? (
        <aside className="hidden w-[320px] shrink-0 flex-col overflow-hidden border-l border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_86%,transparent)] xl:flex">
          <div className="flex items-center justify-between border-b border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] px-4 py-2">
            <span className="shell-kicker">
              {t('filebrowser.preview.title', 'Preview & Meta')}
            </span>
            <IconButton
              size="small"
              onClick={() => setPreviewCollapsed(true)}
              aria-label={t('filebrowser.preview.collapse', 'Collapse sidebar')}
            >
              <PanelRightClose size={14} />
            </IconButton>
          </div>
          <div className="flex-1 overflow-hidden">
            <PreviewPanel
              item={focusedSelectedItem}
              topics={fileBrowserSnapshot.topics}
              onJumpToTopic={(id) => focusedPane.navigate(`view://topic/${id}`)}
              onJumpToPath={(path) => focusedPane.navigate(path)}
              embedded
            />
          </div>
        </aside>
      ) : null}

      <FileContextMenu
        position={contextMenu?.position ?? null}
        sections={contextMenu?.sections ?? []}
        onInvoke={handleMenuAction}
        onClose={() => setContextMenu(null)}
      />

      {toast ? (
        <div className="pointer-events-none absolute bottom-8 left-1/2 -translate-x-1/2 rounded-full bg-black/80 px-3 py-1.5 text-xs text-white">
          {toast}
        </div>
      ) : null}
    </div>
  )
}
