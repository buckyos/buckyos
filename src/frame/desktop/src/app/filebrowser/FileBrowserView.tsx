import { IconButton, useMediaQuery } from '@mui/material'
import clsx from 'clsx'
import { useEffect, useMemo, useRef, useState } from 'react'
import {
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
import { normalizeCyfsPath } from '../../components/preview/session'
import type { ContentRef, PreviewSessionContext } from '../../components/preview/types'
import { useI18n } from '../../i18n/provider'
import { openPreview } from '../preview/launch'
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
import { TransfersPanel } from './TransfersPanel'
import { NamePromptDialog } from './dialogs/NamePromptDialog'
import type { NamePromptRequest } from './dialogs/NamePromptDialog'
import { asCollectionReader } from './data/CollectionModel'
import { collectionDirectory, useCollections } from './data/collectionDirectory'
import { availableName, folderOps } from './data/folderOps'
import type { OperationResult, OperationConflict, ConflictChoice } from './data/folderOps'
import { CopyTasks } from './dialogs/CopyTasks'
import { DeleteDialog, ConflictDialog, BatchResults } from './dialogs/OperationDialogs'
import type { DeleteRequest } from './dialogs/OperationDialogs'
import { operationFeedback, useOperationFeedback } from './data/operationFeedback'
import type { BatchTask } from './data/operationFeedback'
import { MoveTargetDialog } from './dialogs/MoveTargetDialog'
import type { TargetRequest } from './dialogs/MoveTargetDialog'
import { commandState } from './menu/commands'
import './filebrowser.css'
import type { FileItem } from './data/FolderReader'
import { installFileBrowserData } from './data/install'
import { resolveReader } from './data/readerRegistry'
import { collectionTitleSchema, entryNameSchema, validationFallback } from './data/schemas'
import { useSidebarDevices, useSidebarDfs, useSidebarTopics } from './data/sidebarSources'
import { toUiError } from './data/state'
import { stashLocalFile, transferStore } from './data/transfers'
import { useBrowserPane } from './data/useBrowserPane'
import {
  COLLECTION_SCHEME,
  collectionUrl,
  crumbsForUrl,
  dfsPathOf,
  displayPath,
  normalizeUrl,
  parentUrl,
} from './data/urls'
import type {
  BrowserTab,
  ClipboardState,
  DfsNode,
  FileEntry,
  SortDir,
  SortKey,
} from './types'

installFileBrowserData()

/** Initial pane tabs — backend-independent defaults. */
const DEFAULT_TABS: BrowserTab[] = [
  { id: 'tab-home', title: 'Home', path: '/home' },
  { id: 'tab-pictures', title: 'Pictures', path: '/home/Pictures' },
]

/** Session-local tab id (Volatile, §6.2) — module-level so the React
 * Compiler treats event-handler calls as opaque rather than render-impure. */
function newTabId(): string {
  return `tab-${crypto.randomUUID()}`
}

function itemOf(entry: FileEntry): FileItem {
  return { key: entry.id, entry }
}

type BrowserPane = ReturnType<typeof useBrowserPane>
const FILE_DRAG_TYPE = 'application/x-buckyos-file-items'
let draggedFiles: { token: string; pane: BrowserPane; items: FileItem[] } | null = null

export function FileBrowserView({ windowId }: { windowId?: string }) {
  const { t } = useI18n()
  const isMobile = useMediaQuery('(max-width: 900px)')
  // Tailwind xl — the preview sidebar only exists at this width, so the
  // expand control in the status bar should follow the same gate.
  const rootRef = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(1024)
  const [navOverlayOpen, setNavOverlayOpen] = useState(false)
  const [navCollapsed, setNavCollapsed] = useState(() => localStorage.getItem('files.navCollapsed') === 'true')
  const [navWidth, setNavWidth] = useState(() => Math.max(180, Math.min(300, Number(localStorage.getItem('files.navWidth')) || 220)))
  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    const observer = new ResizeObserver(([entry]) => setWidth(entry.contentRect.width))
    observer.observe(el)
    return () => observer.disconnect()
  }, [isMobile])

  const left = useBrowserPane(DEFAULT_TABS, windowId ? `${windowId}:left` : undefined)
  const right = useBrowserPane([], windowId ? `${windowId}:right` : undefined)
  const livePanes = useRef([left, right])
  useEffect(() => { livePanes.current = [left, right] }, [left, right])
  const [, refreshCopyCapability] = useState(0)
  useEffect(() => { const refresh = () => refreshCopyCapability((n) => n + 1); window.addEventListener('files-copy-capability', refresh); return () => window.removeEventListener('files-copy-capability', refresh) }, [])
  const [closedTabs, setClosedTabs] = useState<BrowserTab[]>([])
  const [focusedSide, setFocusedSide] = useState<'left' | 'right'>('left')

  const [advancedMode, setAdvancedMode] = useState(false)
  const [listPreferences, setListPreferences] = useState<{ fullColumns: boolean; density: 'compact' | 'comfortable'; nameWidth: number }>(() => {
    try { return { fullColumns: false, density: 'compact', nameWidth: 260, ...JSON.parse(localStorage.getItem('files.listPreferences') ?? '{}') } }
    catch { return { fullColumns: false, density: 'compact', nameWidth: 260 } }
  })
  const changeListPreferences = (patch: Partial<typeof listPreferences>) => setListPreferences((previous) => {
    const next = { ...previous, ...patch }
    localStorage.setItem('files.listPreferences', JSON.stringify(next))
    return next
  })
  const [toast, setToast] = useState<string | null>(null)
  const [previewCollapsed, setPreviewCollapsed] = useState(true)
  /** Toolbar clipboard, shared by both panes. */
  const [clipboard, setClipboard] = useState<ClipboardState | null>(null)
  /** Active form dialog (new collection/group, rename, new folder). */
  const [namePrompt, setNamePrompt] = useState<NamePromptRequest | null>(null)
  const [deleteRequest, setDeleteRequest] = useState<DeleteRequest | null>(null)
  const { batchTask, conflictRequest } = useOperationFeedback()
  const { setBatchTask } = operationFeedback
  const [targetRequest, setTargetRequest] = useState<TargetRequest | null>(null)
  const busyRef = useRef(false)
  const [dragTarget, setDragTarget] = useState<string | null>(null)
  const [pathEditSignal, setPathEditSignal] = useState(0)

  // Sidebar sources — separate async states so one failing source never
  // blanks the browser (§4.3).
  const dfsSource = useSidebarDfs()
  const devicesSource = useSidebarDevices()
  const topicsSource = useSidebarTopics()
  const topicList = topicsSource.state.data ?? []

  // Live collection list (sidebar + "Add to Collection" submenu).
  const collections = useCollections()

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

  useEffect(() => { setPreviewCollapsed(true); setMobilePreviewOpen(false); setMobileUploadOpen(false) }, [left.activeTabId, left.currentUrl, right.activeTabId, right.currentUrl])

  // Deselecting the last item leaves selection mode.
  useEffect(() => {
    if (mobileSelectMode && left.selectedKeys.size === 0) setMobileSelectMode(false)
  }, [mobileSelectMode, left.selectedKeys])

  // The split layout is desktop-only; the right pane exists while it holds tabs.
  const splitActive = !isMobile && width >= 700 && right.tabs.length > 0
  useEffect(() => {
    if (width >= 700 && !isMobile) return
    for (const tab of right.tabs) { const detached = right.detachTab(tab.id); if (detached) left.adoptTab(detached.tab, detached.history, detached.state) }
    setFocusedSide('left')
  }, [width, isMobile, left, right])
  const focusedIsRight = splitActive && focusedSide === 'right'
  const focusedPane = focusedIsRight ? right : left

  const rememberClosedTab = (tab: BrowserTab) => {
    setClosedTabs((prev) => [tab, ...prev].slice(0, 10))
  }

  const handleNewTab = () => {
    left.adoptTab({ id: newTabId(), title: 'Home', path: '/home' })
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
    right.adoptTab(detached.tab, detached.history, detached.state)
    setFocusedSide('right')
  }

  const handleRestoreClosedTab = (tab: BrowserTab) => {
    setClosedTabs((prev) => prev.filter((item) => item.id !== tab.id))
    left.adoptTab({ ...tab, id: newTabId() })
    setFocusedSide('left')
  }

  const showToast = (message: string) => {
    setToast(message)
    window.setTimeout(() => setToast(null), 2000)
  }

  /** Localized display text for a normalized (or unknown) thrown error. */
  const uiErrorText = (err: unknown) => {
    const ui = toUiError(err)
    return t(ui.messageKey, ui.fallback)
  }

  /** Form-dialog helper: NamePromptDialog renders `Error.message` only. */
  const asFormError = (err: unknown): Error =>
    err instanceof Error ? err : new Error(uiErrorText(err))

  const copyText = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text)
      setClipboard({ entries: [], mode: 'text', text, token: crypto.randomUUID() })
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

  /**
   * Search selection (§2.9): a folder leaves search and navigates to its
   * original location; a file opens its preview context. entry.path is never
   * rewritten by search.
   */
  const handleOpenFolder = (url: string) => {
    left.navigate(url)
    setMobileSidebarOpen(false)
  }

  // ─── Preview App hand-off (PRD §14.1 / §14.4): Source + Session Context ───
  // Folder listings become a Container Context; views, collections and search
  // results hand over an explicit list of the loaded items instead.

  const previewSourceOf = (item: FileItem): ContentRef => ({
    kind: 'cyfs-path',
    path: normalizeCyfsPath(item.entry.path),
  })

  const previewSessionOf = (pane: BrowserPane, item: FileItem): PreviewSessionContext => {
    const dfs = dfsPathOf(pane.currentUrl)
    if (dfs !== null && !pane.searchQuery) {
      return {
        kind: 'container',
        container: { kind: 'cyfs-path', path: normalizeCyfsPath(dfs) },
        current: previewSourceOf(item),
      }
    }
    const loaded = (pane.searchQuery ? pane.searchItems : pane.list
      .loadedKeys()
      .map((key) => pane.list.loadedItemByKey(key))
      .filter((entry): entry is FileItem => !!entry)).filter((item) => item.entry.kind !== 'folder')
    const currentIndex = Math.max(0, loaded.findIndex((entry) => entry.key === item.key))
    return {
      kind: 'list',
      sessionId: `files:${pane.currentUrl}`,
      items: loaded.map((entry) => ({ id: entry.key, source: previewSourceOf(entry), title: entry.entry.name })),
      currentIndex,
    }
  }

  const handleOpenFile = (pane: BrowserPane, item: FileItem, opts?: { newWindow?: boolean }) => {
    openPreview({
      source: previewSourceOf(item),
      session: previewSessionOf(pane, item),
      origin: { app: 'files', hostContext: pane.currentUrl, windowId },
      newWindow: opts?.newWindow,
    })
  }

  /** Multi-selection → stable explicit list; only the chosen files take part (§14.4). */
  const handlePreviewItems = (pane: BrowserPane, chosen: FileItem[]) => {
    const files = chosen.filter((entry) => entry.entry.kind !== 'folder')
    if (!files.length) return
    if (files.length === 1) {
      handleOpenFile(pane, files[0])
      return
    }
    openPreview({
      source: previewSourceOf(files[0]),
      session: {
        kind: 'list',
        sessionId: `files:selection:${newTabId()}`,
        items: files.map((entry) => ({ id: entry.key, source: previewSourceOf(entry), title: entry.entry.name })),
        currentIndex: 0,
        navigation: 'wrap',
      },
      origin: { app: 'files', hostContext: pane.currentUrl, windowId },
    })
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
    handleOpenFile(left, item)
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

  const addEntriesToCollection = (collectionId: string, entries: FileEntry[], pane = focusedPane) =>
    runBatch(pane, entries.map(itemOf), 'add-ref', collectionUrl(collectionId))

  // ─── Form dialogs (react-hook-form + Zod schemas, §3 — no window.prompt) ───

  const requestNewCollection = (onCreated?: (id: string) => void) => {
    setNamePrompt({
      title: t('filebrowser.prompt.newCollection', 'New collection'),
      label: t('filebrowser.prompt.collectionTitle', 'Title'),
      submitLabel: t('filebrowser.actions.create', 'Create'),
      schema: collectionTitleSchema,
      onSubmit: async (value) => {
        try {
          const id = await collectionDirectory().create(value)
          onCreated?.(id)
        } catch (err) {
          throw asFormError(err)
        }
      },
    })
  }

  const requestNewGroup = (pane: BrowserPane) => {
    const url = pane.currentUrl
    setNamePrompt({
      title: t('filebrowser.prompt.newGroup', 'New group'),
      label: t('filebrowser.prompt.groupName', 'Group name'),
      submitLabel: t('filebrowser.actions.create', 'Create'),
      schema: entryNameSchema,
      onSubmit: async (value) => {
        try {
          await withCollection(url, (reader) => reader.createGroup(value))
        } catch (err) {
          throw asFormError(err)
        }
      },
    })
  }

  const requestNewFolder = (pane: BrowserPane) => {
    const parent = dfsPathOf(pane.currentUrl)
    if (!parent || !pane.list.capabilities.acceptsContent) return
    setNamePrompt({
      title: t('filebrowser.actions.newFolder', 'New folder'),
      label: t('filebrowser.prompt.folderName', 'Folder name'),
      submitLabel: t('filebrowser.actions.create', 'Create'),
      schema: entryNameSchema,
      onSubmit: async (value) => {
        try {
          if (await folderOps().nameExists(parent, value)) {
            throw new Error(
              t('filebrowser.error.nameConflict', '"{{name}}" already exists here', {
                name: value,
              }),
            )
          }
          await folderOps().createFolder(parent, value)
        } catch (err) {
          throw asFormError(err)
        }
      },
    })
  }

  const requestRename = (pane: BrowserPane, item: FileItem) => {
    const isGroup = !!item.ref && item.entry.path.startsWith(COLLECTION_SCHEME)
    const url = pane.currentUrl
    const captured = { ...item.entry }
    setNamePrompt({
      title: t('filebrowser.actions.rename', 'Rename'),
      label: t('filebrowser.prompt.entryName', 'Name'),
      submitLabel: t('filebrowser.actions.rename', 'Rename'),
      defaultValue: item.entry.name,
      selectStem: item.entry.kind !== 'folder' && !isGroup,
      schema: entryNameSchema,
      onSubmit: async (value) => {
        try {
          if (!pane.isCurrent()) throw new Error(t('filebrowser.operation.contextChanged', 'The selection changed. Open the menu again.'))
          if (isGroup) {
            await withCollection(url, (reader) => reader.renameGroup(item.key, value))
            return
          }
          if (value === item.entry.name) return
          const parent = item.entry.path.split('/').slice(0, -1).join('/') || '/'
          if (await folderOps().nameExists(parent, value)) {
            throw new Error(
              t('filebrowser.error.nameConflict', '"{{name}}" already exists here', {
                name: value,
              }),
            )
          }
          await folderOps().renameEntry(captured, value)
        } catch (err) {
          throw asFormError(err)
        }
        pane.list.reload()
        pane.reconcileSelection([])
      },
    })
  }

  // ─── Uploads (probe → upload → commit through the transfer store, §4.7) ───

  const conflictResolver = () => {
    const choices = new Map<string, ConflictChoice>()
    return async (conflict: OperationConflict): Promise<ConflictChoice> => {
      const kind = `${conflict.source.kind === 'folder'}:${conflict.target.kind === 'folder'}`
      const previous = choices.get(kind)
      if (previous) return previous
      const { choice, apply } = await operationFeedback.requestConflict(conflict, windowId)
      if (apply) choices.set(kind, choice)
      return choice
    }
  }

  const enqueueFiles = async (target: string, files: FileList | File[] | null) => {
    if (!files?.length) return
    const parent = dfsPathOf(target)
    if (!parent) { showToast(t('filebrowser.operation.localUploadUnsupported', 'Local files can only be uploaded to a writable folder')); return }
    const resolveConflict = conflictResolver()
    let cancelled = false
    for (const file of [...files]) {
      let destination = parent
      const candidate = { localId: crypto.randomUUID(), name: file.name, sizeBytes: file.size, mimeType: file.type || undefined }
      const retry = () => void enqueueFiles(target, [file])
      if (cancelled) { transferStore.recordOutcome(target, candidate, 'cancelled', null, retry); continue }
      try {
        if (file.webkitRelativePath) {
          const segments = file.webkitRelativePath.split('/').slice(0, -1)
          for (const segment of segments) {
            entryNameSchema.parse(segment)
            const existing = await folderOps().statEntry(destination, segment)
            if (existing && existing.kind !== 'folder') throw new Error(t('filebrowser.operation.folderNameConflict', 'A file blocks the upload folder path'))
            if (!existing) await folderOps().createFolder(destination, segment)
            destination = `${destination === '/' ? '' : destination}/${segment}`
          }
        }
        let name = file.name
        const existing = await folderOps().statEntry(destination, name)
        if (existing) {
          const choice = await resolveConflict({ source: { id: '', path: file.webkitRelativePath || name, name, kind: 'other', sizeBytes: file.size, modifiedAt: new Date(file.lastModified).toISOString() }, target: existing, targetPath: destination })
          if (choice === 'cancel') { cancelled = true; transferStore.recordOutcome(normalizeUrl(destination), candidate, 'cancelled', null, retry); continue }
          if (choice === 'skip') { transferStore.recordOutcome(normalizeUrl(destination), candidate, 'skipped'); continue }
          name = await availableName(destination, name)
        }
        const localId = crypto.randomUUID()
        stashLocalFile(localId, file)
        const { rejected } = transferStore.enqueue(normalizeUrl(destination), [{ ...candidate, localId, name }], retry)
        for (const reject of rejected) {
          const key = reject.messageKeys[0]
          transferStore.recordOutcome(normalizeUrl(destination), candidate, 'error', { code: 'VALIDATION', messageKey: key, fallback: validationFallback[key] ?? key, retryable: false })
        }
      } catch (err) { transferStore.recordOutcome(normalizeUrl(destination), candidate, 'error', toUiError(err), retry) }
    }
  }

  const triggerUpload = (pane: BrowserPane, accept?: string, folder = false) => {
    if (!pane.list.capabilities.acceptsContent || pane.searchQuery) return
    const target = pane.currentUrl
    const input = document.createElement('input')
    input.type = 'file'
    input.multiple = true
    if (accept) input.accept = accept
    if (folder) input.webkitdirectory = true
    input.onchange = () => void enqueueFiles(target, input.files)
    input.click()
  }

  // ─── Context menu (desktop) ───

  /** Quick "Move to" targets: DFS roots and their first-level folders. */
  const dfsRoots = dfsSource.state.data
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
    visit(dfsRoots ?? [], 0)
    return targets
  }, [dfsRoots])

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
    setClipboard({ entries, mode: 'cut', token: crypto.randomUUID() })
    showToast(t('filebrowser.toast.cut', 'Cut {{count}} item(s)', { count: entries.length }))
  }

  const copyEntries = (entries: FileEntry[], mode: 'copy' | 'references' = 'copy') => {
    if (!entries.length) return
    setClipboard({ entries, mode, token: crypto.randomUUID() })
    showToast(
      t('filebrowser.toast.copyItems', 'Copied {{count}} item(s)', { count: entries.length }),
    )
  }

  const runBatch = async (pane: BrowserPane, items: FileItem[], kind: 'copy' | 'move' | 'delete' | 'remove-ref' | 'add-ref', target?: string, cutClipboard?: ClipboardState | null, previousResults: OperationResult[] = [], retryOf?: string, requestKey = crypto.randomUUID(), resumeTaskId?: string) => {
    if (operationFeedback.isRunning() || !items.length) return
    busyRef.current = true
    const selectedAtStart = [...pane.selectedItemsMap.values()]
    const reconcileSources = livePanes.current.map((source) => source.reconcileMovedEntries)
    const captured = items.map((item) => ({ ...item, entry: { ...item.entry } }))
    const controller = new AbortController()
    const title = `${kind === 'copy' ? t('filebrowser.actions.copyTo', 'Copy to') : kind === 'move' ? t('filebrowser.actions.moveTo', 'Move to') : kind === 'delete' ? t('filebrowser.operation.permanentDelete', 'Permanently delete') : kind === 'add-ref' ? t('filebrowser.operation.addReferences', 'Add references') : t('filebrowser.actions.removeFromCollection', 'Remove from collection')} · ${target ?? displayPath(pane.currentUrl)}`
    const initial: BatchTask = { operationKey: requestKey, ownerId: windowId, reveal: (path) => livePanes.current[0].revealOriginal(path), title, total: items.length + previousResults.length, results: previousResults, running: true, cancel: () => { controller.abort(); setBatchTask((current) => current ? { ...current, cancelling: true } : current) }, retry: () => {} }
    setBatchTask(initial)
    let results: OperationResult[] = []
    const onProgress = (results: OperationResult[]) => setBatchTask((prev) => prev?.operationKey === requestKey ? { ...prev, results: [...previousResults, ...results] } : prev)
    let monitorFailed = false
    let copyTaskId = resumeTaskId
    const options = { signal: controller.signal, onProgress, onConflict: conflictResolver(), retryOf, requestKey,
      onCopyConflict: (conflict: OperationConflict) => operationFeedback.requestConflict(conflict, windowId),
      onTask: (task: { taskId: string; total: number; cancelling: boolean; loadMore?: () => Promise<void> }) => {
        copyTaskId = task.taskId
        setBatchTask((current) => current?.operationKey === requestKey ? { ...current, ...task } : current)
      },
    }
    try {
      if (kind === 'copy') results = resumeTaskId && folderOps().resumeCopy ? await folderOps().resumeCopy!(resumeTaskId, options) : await folderOps().copyEntries(captured.map((item) => item.entry), target!, options)
      else if (kind === 'move') results = await folderOps().moveEntries(captured.map((item) => item.entry), target!, options)
      else if (kind === 'delete') results = await folderOps().deleteEntries(captured.map((item) => item.entry), options)
      else {
        for (const item of captured) {
          const result: OperationResult = { entry: item.entry, itemKey: item.key, targetPath: target, status: controller.signal.aborted ? 'cancelled' : 'success' }
          if (result.status === 'success') try { await withCollection(kind === 'add-ref' ? target! : pane.currentUrl, (reader) => kind === 'add-ref' ? reader.addReferences([normalizeUrl(item.entry.path)]) : reader.removeItems([item.key])) } catch (err) { result.status = 'failed'; result.error = toUiError(err) }
          results.push(result)
          onProgress([...results])
        }
      }
    } catch (err) {
      monitorFailed = true
      results = captured.map((item) => ({ entry: item.entry, status: 'failed', error: toUiError(err) }))
    } finally { busyRef.current = false }
    results = [...previousResults, ...results.map((result, i) => ({ ...result, itemKey: captured[i]?.key }))]
    const successfulKeys = new Set(results.filter((result) => result.status === 'success').map((result) => result.itemKey))
    const succeeded = new Set(results.filter((result) => result.status === 'success').map((result) => result.entry.id))
    if (kind === 'move' || kind === 'delete') reconcileSources.forEach((reconcile) => reconcile(succeeded))
    if (kind !== 'copy') pane.reconcileSelection(selectedAtStart.filter((item) => !successfulKeys.has(item.key)))
    if (kind === 'move' && cutClipboard) setClipboard((current) => {
      if (!current || current.token !== cutClipboard.token) return current
      const entries = current.entries.filter((entry) => !succeeded.has(entry.id))
      return entries.length ? { ...current, entries } : null
    })
    setBatchTask((current) => ({ ...initial, ...current, taskId: copyTaskId, summary: monitorFailed ? undefined : current?.summary, results, running: false, cancelling: false, retry: () => {
      if (kind === 'copy') { void runBatch(pane, captured, kind, target, undefined, [], monitorFailed ? undefined : copyTaskId, monitorFailed ? requestKey : undefined, monitorFailed ? copyTaskId : undefined); return }
      const failed = new Set(results.filter((result) => result.status === 'failed').map((result) => result.entry.id))
      const current = livePanes.current.find((current) => current.activeTabId === pane.activeTabId && current.currentUrl === pane.currentUrl && [...current.selectedItemsMap.values()].every((item) => failed.has(item.entry.id)))
      void runBatch(current ?? pane, captured.filter((item) => failed.has(item.entry.id)), kind, target, cutClipboard, results.filter((result) => result.status !== 'failed'))
    } }))
  }

  const moveSelected = (pane: BrowserPane, entries: FileEntry[], target: string, clip?: ClipboardState | null) => runBatch(pane, entries.map(itemOf), 'move', target, clip)
  const requestCopy = (pane: BrowserPane, entries: FileEntry[]) => setTargetRequest({ entries, copy: true, initial: dfsPathOf(pane.currentUrl) ?? '/home', submit: (path) => void runBatch(pane, entries.map(itemOf), 'copy', path) })
  const requestMove = (pane: BrowserPane, entries: FileEntry[]) => setTargetRequest({ entries, initial: dfsPathOf(pane.currentUrl) ?? '/home', submit: (path) => void moveSelected(pane, entries, path) })
  const requestExisting = (pane: BrowserPane) => {
    const url = pane.currentUrl
    setTargetRequest({ entries: [], initial: '/home', references: true, submit: (_, entries) => {
      if (entries?.length) void runBatch(pane, entries.map(itemOf), 'add-ref', url)
    } })
  }
  const pasteInto = (pane: BrowserPane) => {
    if (!clipboard) return
    if (pane.list.capabilities.acceptsReferences && clipboard.mode === 'references') {
      void runBatch(pane, clipboard.entries.map(itemOf), 'add-ref', pane.currentUrl)
      return
    }
    const target = dfsPathOf(pane.currentUrl)
    if (target && pane.list.capabilities.acceptsContent && clipboard.mode === 'copy') void runBatch(pane, clipboard.entries.map(itemOf), 'copy', target)
    if (target && pane.list.capabilities.acceptsContent && clipboard.mode === 'cut') void moveSelected(pane, clipboard.entries, target, clipboard)
  }
  const removeFromCollection = (pane: BrowserPane, keys: string[]) => {
    const items = keys.map((key) => pane.selectedItemsMap.get(key) ?? pane.list.loadedItemByKey(key)).filter((item): item is FileItem => !!item)
    setDeleteRequest({ items, location: displayPath(pane.currentUrl), references: true, submit: () => void runBatch(pane, items, 'remove-ref') })
  }
  const deleteSelected = (pane: BrowserPane, items: FileItem[]) => {
    if (!items.length) return
    setDeleteRequest({ items, location: displayPath(pane.currentUrl), references: false, submit: () => void runBatch(pane, items, 'delete') })
  }
  const downloadEntries = (entries: FileEntry[]) => {
    const results: OperationResult[] = entries.map((entry) => {
      const url = folderOps().downloadUrl(entry)
      if (!url) return { entry, status: 'skipped', error: { code: 'UNSUPPORTED', messageKey: entry.kind === 'folder' ? 'filebrowser.operation.folderDownloadUnsupported' : 'filebrowser.toast.downloadUnavailable', fallback: entry.kind === 'folder' ? 'Folder download is not supported by this server' : 'Download is not available here', retryable: false } }
      const anchor = document.createElement('a')
      anchor.href = url; anchor.download = entry.name; anchor.click()
      return { entry, status: 'success' }
    })
    setBatchTask({ ownerId: windowId, title: t('filebrowser.operation.browserDownload', 'Downloads handed to the browser'), results, total: entries.length, running: false, cancel: () => {}, retry: () => {} })
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
  const contextFor = (pane: BrowserPane, items: FileItem[]): FileMenuContext => ({
    target: items.length > 1 ? 'selection' : items.length ? 'item' : 'view',
    items, entries: items.map((item) => item.entry), currentUrl: pane.currentUrl,
    viewMode: pane.viewMode, capabilities: pane.list.capabilities, sortKey: pane.sortKey,
    collections: collections.map((item) => ({ id: item.id, title: item.title })), moveTargets,
    pane: { canOpenInNewTab: !isMobile, canOpenInRightPane: !isMobile && width >= 700 },
    clipboard, searching: !!pane.searchQuery, contextToken: pane.contextToken,
    loadedCount: pane.searchQuery ? pane.searchItems.length : pane.list.loadedKeys().length,
    busy: !!batchTask?.running || pane.enumerating,
    otherPath: splitActive ? dfsPathOf(pane === left ? right.currentUrl : left.currentUrl) ?? undefined : undefined,
  })
  const toolbarPropsFor = (side: 'left' | 'right') => {
    const pane = paneFor(side)
    const items = [...pane.selectedItemsMap.values()]
    const ctx = contextFor(pane, items)
    return {
      selectedCount: items.length, capabilities: pane.list.capabilities,
      listPreferences, onListPreferences: changeListPreferences,
      onMore: (position: MenuPosition) => openMenu(side, position, items[0]),
      onDetails: () => setPreviewCollapsed((value) => !value),
      onPlaces: () => { if (splitActive && width < 1300) { setNavOverlayOpen(!navOverlayOpen); return }; const next = !navCollapsed; setNavCollapsed(next); localStorage.setItem('files.navCollapsed', String(next)) },
      onMoveTo: () => requestMove(pane, ctx.entries),
      onCopy: commandState('copy', ctx).state === 'available' ? () => copyEntries(ctx.entries.map((entry) => ({ ...entry }))) : undefined,
      onPaste: commandState('paste', ctx).state === 'available' ? () => pasteInto(pane) : undefined,
      onCopyTo: commandState('copy-to', ctx).state === 'available' ? () => requestCopy(pane, ctx.entries) : undefined,
      sortKey: pane.sortKey, sortDir: pane.sortDir,
      onSortChange: (key: SortKey, dir: SortDir) => { pane.setSortKey(key); pane.setSortDir(dir) },
      onUpload: commandState('upload', ctx).state === 'available' ? () => triggerUpload(pane) : undefined,
      onFolderUpload: commandState('upload', ctx).state === 'available' && 'webkitdirectory' in document.createElement('input') ? () => triggerUpload(pane, undefined, true) : undefined,
      onNewFolder: commandState('new-folder', ctx).state === 'available' ? () => requestNewFolder(pane) : undefined,
      onAddExisting: pane.list.capabilities.acceptsReferences ? () => requestExisting(pane) : undefined,
      pathEditSignal: focusedPane === pane ? pathEditSignal : 0,
    }
  }

  const openMenu = (side: 'left' | 'right', position: MenuPosition, item?: FileItem) => {
    const pane = paneFor(side)
    let items: FileItem[] = []
    let selectionChanged = false
    if (item) {
      if (pane.selectedKeys.has(item.key)) {
        items = [...pane.selectedItemsMap.values()]
      } else {
        // Right-clicking outside the current selection re-anchors it to the item.
        selectionChanged = true
        pane.applySelection([item.key], new Map([[item.key, item]]))
        items = [item]
      }
    }
    const context = contextFor(pane, items)
    if (selectionChanged) { const [tabId, revision] = pane.contextToken.split(':'); context.contextToken = `${tabId}:${Number(revision) + 1}` }
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
    if (context.contextToken !== pane.contextToken) { showToast(t('filebrowser.operation.contextChanged', 'The selection changed. Open the menu again.')); return }
    const state = commandState(action.command, { ...context, busy: busyRef.current || pane.enumerating })
    if (state.state !== 'available') { showToast(t(`filebrowser.commandReason.${state.reason}`, state.reason)); return }
    const items = context.items
    const first = items[0]
    const entries = context.entries
    switch (action.command) {
      case 'open':
        if (!first) break
        if (first.entry.kind === 'folder') {
          // Group entries carry collection:// paths; folder refs carry their
          // original dfs path — navigation normalizes both.
          pane.navigate(first.entry.path)
        } else {
          pane.applySelection([first.key], new Map([[first.key, first]]))
          handleOpenFile(pane, first)
        }
        break
      case 'preview-new-window':
        if (first && first.entry.kind !== 'folder') handleOpenFile(pane, first, { newWindow: true })
        break
      case 'preview-selection':
        handlePreviewItems(pane, items)
        break
      case 'open-new-tab':
        if (first?.entry.kind === 'folder') {
          pane.adoptTab({
            id: newTabId(),
            title: first.entry.name,
            path: first.entry.path,
          })
        }
        break
      case 'open-right':
        if (first?.entry.kind === 'folder') {
          right.adoptTab({
            id: newTabId(),
            title: first.entry.name,
            path: first.entry.path,
          })
          setFocusedSide('right')
        }
        break
      case 'cut': cutEntries(entries.map((entry) => ({ ...entry }))); break
      case 'copy': copyEntries(entries.map((entry) => ({ ...entry }))); break
      case 'copy-references': copyEntries(entries.map((entry) => ({ ...entry })), 'references'); break
      case 'copy-to': requestCopy(pane, entries); break
      case 'copy-other': {
        const path = String(action.args?.path ?? context.otherPath ?? '')
        if (path) void runBatch(pane, items, 'copy', path)
        break
      }
      case 'paste': pasteInto(pane); break
      case 'details':
        if (first) pane.applySelection([first.key], new Map([[first.key, first]]))
        if (isMobile) setMobilePreviewOpen(true)
        else setPreviewCollapsed(false)
        break
      case 'add-existing': requestExisting(pane); break
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
        if (collectionId) void addEntriesToCollection(collectionId, entries, pane)
        break
      }
      case 'new-collection': {
        const toAdd = [...entries]
        requestNewCollection((id) => {
          if (toAdd.length) void addEntriesToCollection(id, toAdd, pane)
        })
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
      case 'new-group':
        requestNewGroup(pane)
        break
      case 'download':
        downloadEntries(entries)
        break
      case 'rename':
        if (first) requestRename(pane, first)
        break
      case 'move-to': requestMove(pane, entries); break
      case 'move-other': {
        const path = String(action.args?.path ?? '')
        if (path) void moveSelected(pane, entries, path)
        break
      }
      case 'delete':
        void deleteSelected(pane, items)
        break
      case 'new-folder':
        requestNewFolder(pane)
        break
      case 'upload':
        triggerUpload(pane)
        break
      case 'refresh':
        if (pane.searchQuery) pane.search.retry()
        else pane.list.reload()
        break
      case 'select-loaded':
        pane.selectLoaded()
        if (isMobile) setMobileSelectMode(true)
        break
      case 'select-all':
        void pane.selectAll()
        if (isMobile) setMobileSelectMode(true)
        break
      case 'view-list':
        pane.setViewMode('list')
        break
      case 'view-icon':
        pane.setViewMode('icon')
        break
      default:
        break
    }
  }

  const handleMenuAction = (action: FileMenuAction) => {
    if (!contextMenu) return
    runMenuAction(paneFor(contextMenu.side), contextMenu.context, action)
  }

  // ─── Mobile menus: same registry data rendered as a bottom sheet ───

  /** 0 items → view menu, 1 → item menu, 2+ → selection menu. */
  const openMobileMenu = (items: FileItem[]) => {
    const context = contextFor(left, items)
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
    const command = left.list.capabilities.removal === 'remove-ref' ? 'remove-from-collection' : 'delete'
    if (commandState(command, contextFor(left, [...left.selectedItemsMap.values()])).state !== 'available') return
    const keys = [...left.selectedKeys]
    if (!keys.length) return
    if (left.list.capabilities.removal === 'remove-ref') {
      void removeFromCollection(left, keys)
      return
    }
    void deleteSelected(left, [...left.selectedItemsMap.values()])
  }

  const handleCreateCollection = () => {
    requestNewCollection((id) => left.navigate(collectionUrl(id)))
  }

  const leftSearchActive = !!left.searchQuery.trim()
  const rightSearchActive = !!right.searchQuery.trim()

  const focusedSelectedItem = focusedIsRight ? rightSelectedItem : leftSelectedItem

  const mobileTitleText = (mobilePreviewOpen ? leftSelectedItem?.entry.name : undefined) ?? left.activeTab?.title ?? 'root'
  const mobileSubtitleText =
    (mobilePreviewOpen ? leftSelectedItem?.entry.summary : undefined) ??
    left.list.meta?.description ??
    (displayPath(left.currentUrl) === '/'
      ? t('filebrowser.mobile.rootHint', 'Root directory')
      : displayPath(left.currentUrl))

  const mobileTitleOverride = useMemo(
    () => (isMobile ? { title: mobileTitleText, subtitle: mobileSubtitleText } : null),
    [isMobile, mobileTitleText, mobileSubtitleText],
  )
  useMobileTitleOverride(mobileTitleOverride)

  const canMobileBack = isMobile && (mobileMenu || mobilePreviewOpen || mobileSidebarOpen || mobileUploadOpen || mobileSelectMode || left.activeHistory.back.length > 0)
  useMobileBackHandler(canMobileBack ? () => {
    if (mobileMenu) setMobileMenu(null)
    else if (mobilePreviewOpen) setMobilePreviewOpen(false)
    else if (mobileUploadOpen) setMobileUploadOpen(false)
    else if (mobileSidebarOpen) setMobileSidebarOpen(false)
    else if (mobileSelectMode) exitMobileSelectMode()
    else left.back()
  } : null)

  const handleKeyDown = (event: React.KeyboardEvent) => {
    if ((event.target as HTMLElement).closest('input, textarea, select, [contenteditable="true"], [role="dialog"]') || namePrompt || deleteRequest || targetRequest || conflictRequest) return
    const pane = focusedPane
    const items = [...pane.selectedItemsMap.values()]
    const ctx = contextFor(pane, items)
    const cmd = event.ctrlKey || event.metaKey
    let command: string | undefined
    if (cmd && event.key.toLowerCase() === 'a') { event.preventDefault(); void pane.selectAll(); return }
    if (cmd && event.key.toLowerCase() === 'l') { event.preventDefault(); setPathEditSignal((value) => value + 1); return }
    if (event.altKey && event.key === 'ArrowLeft') { event.preventDefault(); pane.back(); return }
    if (event.altKey && event.key === 'ArrowRight') { event.preventDefault(); pane.forward(); return }
    if (event.key === 'Escape') { event.preventDefault(); pane.cancelEnumeration(); pane.clearSelection(); setClipboard(null); return }
    if (event.metaKey && event.key === 'Backspace') command = pane.list.capabilities.removal === 'remove-ref' ? 'remove-from-collection' : 'delete'
    else if (cmd) command = ({ c: 'copy', x: 'cut', v: 'paste' } as Record<string, string>)[event.key.toLowerCase()]
    else if (event.key === 'F2') command = 'rename'
    else if (event.key === 'Delete' || (event.metaKey && event.key === 'Backspace')) command = pane.list.capabilities.removal === 'remove-ref' ? 'remove-from-collection' : 'delete'
    else if (event.key === 'Enter' && items.length === 1) command = 'open'
    if (command) { event.preventDefault(); runMenuAction(pane, ctx, { type: 'action', id: command, command, label: { key: '', fallback: command } }) }
  }
  const dropProps = (pane: BrowserPane) => ({
    onDragStart: (event: React.DragEvent) => {
      const key = (event.target as HTMLElement).closest<HTMLElement>('[data-item-key]')?.dataset.itemKey
      const item = key ? pane.list.loadedItemByKey(key) : undefined
      if (!item) return
      const items = pane.selectedItemsMap.has(item.key) ? [...pane.selectedItemsMap.values()] : [item]
      const token = crypto.randomUUID()
      draggedFiles = { token, pane, items: items.map((item) => ({ ...item, entry: { ...item.entry } })) }
      event.dataTransfer.setData(FILE_DRAG_TYPE, token)
      event.dataTransfer.effectAllowed = 'copyMove'
    },
    onDragEnd: () => { draggedFiles = null; setDragTarget(null) },
    onDragOver: (event: React.DragEvent) => {
      const internal = event.dataTransfer.types.includes(FILE_DRAG_TYPE)
      if (!internal && !event.dataTransfer.types.includes('Files')) return
      event.preventDefault()
      event.dataTransfer.dropEffect = pane.searchQuery ? 'none' : internal && pane.list.capabilities.acceptsReferences ? 'copy' : pane.list.capabilities.acceptsContent ? internal && !event.ctrlKey && !event.altKey ? 'move' : 'copy' : 'none'
      setDragTarget(pane.currentUrl)
    },
    onDragLeave: (event: React.DragEvent) => { if (!event.currentTarget.contains(event.relatedTarget as Node)) setDragTarget(null) },
    onDrop: (event: React.DragEvent) => {
      event.preventDefault()
      setDragTarget(null)
      if (event.dataTransfer.types.includes(FILE_DRAG_TYPE)) {
        const source = draggedFiles
        draggedFiles = null
        if (!source || source.token !== event.dataTransfer.getData(FILE_DRAG_TYPE) || !source.pane.isCurrent()) { showToast(t('filebrowser.operation.contextChanged', 'The selection changed. Open the menu again.')); return }
        if (pane.searchQuery) { showToast(t('filebrowser.operation.invalidDestination', 'Choose a writable folder outside the selected folders.')); return }
        if (pane.list.capabilities.acceptsReferences) { void runBatch(source.pane, source.items, 'add-ref', pane.currentUrl); return }
        const copying = event.ctrlKey || event.altKey
        const state = commandState(copying ? 'copy-to' : 'move-to', contextFor(source.pane, source.items))
        if (state.state !== 'available') { showToast(t(`filebrowser.commandReason.${state.reason}`, state.reason ?? 'This action is unavailable')); return }
        const target = dfsPathOf(pane.currentUrl)
        if (!target || !pane.list.capabilities.acceptsContent) { showToast(t('filebrowser.operation.invalidDestination', 'Choose a writable folder outside the selected folders.')); return }
        void runBatch(source.pane, source.items, copying ? 'copy' : 'move', target)
        return
      }
      if (!pane.list.capabilities.acceptsContent || pane.searchQuery) { showToast(t('filebrowser.operation.localUploadUnsupported', 'Local files can only be uploaded to a writable folder')); return }
      if ([...event.dataTransfer.items].some((item) => item.webkitGetAsEntry?.()?.isDirectory)) { showToast(t('filebrowser.operation.useFolderUpload', 'Use Upload folder to preserve directory structure')); return }
      void enqueueFiles(pane.currentUrl, event.dataTransfer.files)
    },
  })
  const dialogs = <>
    <DeleteDialog request={deleteRequest} onClose={() => setDeleteRequest(null)} />
    <ConflictDialog request={conflictRequest?.ownerId === windowId ? conflictRequest : null} />
    <CopyTasks ownerId={windowId} reveal={(path) => livePanes.current[0].revealOriginal(path)} />
    {targetRequest && <MoveTargetDialog request={targetRequest} onClose={() => setTargetRequest(null)} />}
    <BatchResults task={batchTask?.ownerId === windowId ? batchTask : null} onClose={() => setBatchTask(null)} />
    {dragTarget && <div className="pointer-events-none absolute inset-2 z-40 flex items-center justify-center rounded-xl border-2 border-dashed border-[color:var(--cp-accent)] bg-[color:var(--cp-surface)]/90 p-6">{t('filebrowser.operation.dropDestination', 'Drop into {{path}}', { path: displayPath(dragTarget) })}</div>}
  </>

  const searchProps = (pane: BrowserPane) => ({
    scope: pane.searchScope, onScopeChange: pane.setSearchScope, currentUrl: pane.currentUrl,
    kind: pane.searchKind, modified: pane.searchModified, onFilterChange: pane.setSearchFilter,
    onExit: () => pane.setSearchQuery(''), selectedKeys: pane.selectedKeys,
    onSelect: (hit: import('./types').SearchResultItem, modifiers?: SelectModifiers) => { if (isMobile && !mobileSelectMode) { if (hit.entry.kind === 'folder') pane.navigate(hit.entry.path); else handleOpenFile(pane, itemOf(hit.entry)) } else pane.selectItem(itemOf(hit.entry), modifiers) },
    mobile: isMobile,
    onOpen: (hit: import('./types').SearchResultItem) => hit.entry.kind === 'folder' ? pane.navigate(hit.entry.path) : handleOpenFile(pane, itemOf(hit.entry)),
    onMenu: (hit: import('./types').SearchResultItem, position: MenuPosition) => isMobile ? openMobileMenu([itemOf(hit.entry)]) : openMenu(pane === left ? 'left' : 'right', position, itemOf(hit.entry)),
    scroll: pane.scroll, onScroll: pane.setScroll,
  })
  const listProps = (pane: BrowserPane) => ({
    ...listPreferences, onNameWidthChange: (nameWidth: number) => changeListPreferences({ nameWidth }),
    scroll: pane.scroll, onScroll: pane.setScroll, revealIndex: pane.revealIndex,
    cutIds: clipboard?.mode === 'cut' ? new Set(clipboard.entries.map((entry) => entry.id)) : new Set<string>(),
    onSelectLoaded: pane.selectLoaded, onSelectAll: () => void pane.selectAll(),
    sortKey: pane.sortKey, sortDir: pane.sortDir,
    onSortChange: (key: SortKey, dir: SortDir) => { pane.setSortKey(key); pane.setSortDir(dir) },
  })
  const selectionProgress = (pane: BrowserPane) => <>
    {pane.enumerating && <div role="status" className="flex items-center gap-2 px-3 py-1 text-xs">{t('filebrowser.operation.enumerating', 'Loading the full selection / locating file…')}<button className="min-h-8 underline" onClick={pane.cancelEnumeration}>{t('common.cancel', 'Cancel')}</button></div>}
    {pane.selectionNotice && <p role="status" className="px-3 text-xs">{t('filebrowser.operation.loadedRange', 'Range selection includes loaded rows only. Use Select all for the full folder.')}</p>}
    {pane.selectionError && <p role="alert" className="px-3 text-xs">{pane.selectionError}</p>}
  </>

  // ─── Mobile layout ───
  if (isMobile) {
    const crumbs = crumbsForUrl(left.currentUrl, left.list.meta?.title)

    return (
      <div ref={rootRef} data-testid="filebrowser" tabIndex={-1} onKeyDown={handleKeyDown} {...dropProps(left)} className="filebrowser fb-mobile relative flex h-full w-full flex-col overflow-hidden" style={{ background: 'var(--cp-bg)' }}>
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

        {mobileSelectMode && <div className="flex flex-wrap gap-1 px-3 text-xs">{[
          { command: 'download', label: t('filebrowser.menu.download', 'Download') },
          { command: 'copy', label: t('filebrowser.menu.copyFiles', 'Copy files') },
          { command: 'paste', label: t('filebrowser.menu.paste', 'Paste') },
          { command: 'copy-to', label: t('filebrowser.actions.copyTo', 'Copy to') },
          { command: 'copy-other', label: t('filebrowser.menu.copyOther', 'Copy to other pane') },
          { command: 'move-to', label: t('filebrowser.actions.moveTo', 'Move to') },
          { command: 'copy-references', label: t('filebrowser.menu.copyReferences', 'Copy references for a collection') },
        ].map(({ command, label }) => <button key={command} className="rounded-lg border border-[color:var(--cp-border)] px-2 disabled:opacity-40" disabled={commandState(command, contextFor(left, leftSelectedItems)).state !== 'available'} onClick={() => runMenuAction(left, contextFor(left, leftSelectedItems), { type: 'action', id: command, command, label: { key: '', fallback: label } })}>{label}</button>)}</div>}
        {selectionProgress(left)}
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
              state={left.search.state}
              query={left.searchQuery}
              {...searchProps(left)}
              onRetry={left.search.retry}
              onLoadMore={left.search.loadMore}
            />
          ) : (
            <MainContent
              key={`${left.activeTabId}:${left.currentUrl}`}
              {...listProps(left)}
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
              onUpload={() => triggerUpload(left)}
            />
          )}
        </div>

        {/* Floating action button — upload (hidden while selecting) */}
        {!mobileSelectMode && (left.list.capabilities.acceptsContent || left.list.capabilities.acceptsReferences) && !leftSearchActive ? (
          <button
            type="button"
            onClick={() => left.list.capabilities.acceptsReferences ? requestExisting(left) : setMobileUploadOpen(true)}
            aria-label={left.list.capabilities.acceptsReferences ? t('filebrowser.operation.addExisting', 'Add existing files') : t('filebrowser.actions.upload', 'Upload')}
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
              <div className="grid grid-cols-3 gap-1 px-3 py-3">
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
                      if (action.key === 'files' || action.key === 'photos') {
                        triggerUpload(left, action.key === 'photos' ? 'image/*' : undefined)
                      } else if (action.key === 'folder') {
                        requestNewFolder(left)
                      }
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
                dfs={dfsSource}
                devices={devicesSource}
                topics={topicsSource}
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
                  topics={topicList}
                  onJumpToTopic={(id) => {
                    left.navigate(`view://topic/${id}`)
                    setMobilePreviewOpen(false)
                  }}
                  onJumpToPath={(path) => {
                    left.navigate(path)
                    setMobilePreviewOpen(false)
                  }}
                  onOpenFile={(item) => handleOpenFile(left, item)}
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

        {dialogs}
        <TransfersPanel onOpenTarget={(path) => left.revealOriginal(path)} mobile={isMobile} />
        <NamePromptDialog request={namePrompt} onClose={() => setNamePrompt(null)} />

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
      ref={rootRef}
      data-testid="filebrowser"
      tabIndex={-1}
      onKeyDown={handleKeyDown}
      className="filebrowser relative flex h-full w-full overflow-hidden"
      style={{ background: 'var(--cp-bg)' }}
    >
      {((!navCollapsed && (!splitActive || width >= 1300)) || navOverlayOpen) && <aside
        style={{ width: navWidth, resize: 'horizontal', minWidth: 180, maxWidth: 300, ...(navOverlayOpen && splitActive && width < 1300 ? { position: 'absolute' as const, top: 128, bottom: 0, left: 0, zIndex: 25, background: 'var(--cp-surface)' } : {}) }}
        onPointerUp={(event) => { const value = event.currentTarget.getBoundingClientRect().width; setNavWidth(value); localStorage.setItem('files.navWidth', String(value)) }}
        className="flex shrink-0 flex-col overflow-hidden border-r border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-surface)_82%,transparent)] px-2 pt-2"
      >
        <Sidebar
          dfs={dfsSource}
          devices={devicesSource}
          topics={topicsSource}
          collections={collections}
          activeUrl={focusedPane.currentUrl}
          advancedMode={advancedMode}
          onToggleAdvanced={setAdvancedMode}
          onNavigate={(url) => { focusedPane.navigate(url); setNavOverlayOpen(false) }}
          onCreateCollection={handleCreateCollection}
        />
      </aside>}

      <main
        data-pane="left" data-list-status={left.list.status} data-location={left.currentUrl}
        data-active={!focusedIsRight}
        {...dropProps(left)}
        className="fb-pane flex min-w-0 flex-1 flex-col"
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
          canSendToRight={left.tabs.length > 1 && width >= 700}
          {...toolbarPropsFor('left')}
        />
        {selectionProgress(left)}
        <div className="min-h-0 flex-1 overflow-hidden">
          {leftSearchActive ? (
            <SearchResultsPanel
              state={left.search.state}
              query={left.searchQuery}
              {...searchProps(left)}
              onRetry={left.search.retry}
              onLoadMore={left.search.loadMore}
            />
          ) : (
            <MainContent
              key={`${left.activeTabId}:${left.currentUrl}`}
              {...listProps(left)}
              list={left.list}
              viewMode={left.viewMode}
              selectedKeys={left.selectedKeys}
              onSelect={(item, modifiers) => left.selectItem(item, modifiers)}
              onOpenFolder={handleOpenFolder}
              onOpenFile={(item) => handleOpenFile(left, item)}
              onItemContextMenu={(item, position) => openMenu('left', position, item)}
              onViewContextMenu={(position) => openMenu('left', position)}
              onClearSelection={left.clearSelection}
              currentUrl={left.currentUrl}
              onUpload={() => triggerUpload(left)}
            />
          )}
        </div>
        <StatusBar
          currentUrl={left.currentUrl}
          totalCount={leftSearchActive ? (left.search.state.data?.nextCursor ? undefined : left.searchItems.length) : left.list.totalCount}
          loadedCount={leftSearchActive ? left.searchItems.length : left.list.loadedCount}
          hasMore={leftSearchActive ? !!left.search.state.data?.nextCursor : left.list.hasMore}
          selectedItems={leftSelectedItems}
          onCopy={copyText}
          onExpandSidebar={
            previewCollapsed
              ? () => setPreviewCollapsed(false)
              : undefined
          }
        />
      </main>

      {splitActive ? (
        <section
          data-pane="right" data-list-status={right.list.status} data-location={right.currentUrl}
          data-active={focusedIsRight}
          {...dropProps(right)}
          className="fb-pane flex min-w-0 flex-1 flex-col border-l border-[color:color-mix(in_srgb,var(--cp-border)_60%,transparent)]"
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
          {selectionProgress(right)}
          <div className="min-h-0 flex-1 overflow-hidden">
            {rightSearchActive ? (
              <SearchResultsPanel
                state={right.search.state}
                query={right.searchQuery}
                {...searchProps(right)}
                onRetry={right.search.retry}
                onLoadMore={right.search.loadMore}
              />
            ) : (
              <MainContent
                key={`${right.activeTabId}:${right.currentUrl}`}
                {...listProps(right)}
                list={right.list}
                viewMode={right.viewMode}
                selectedKeys={right.selectedKeys}
                onSelect={(item, modifiers) => right.selectItem(item, modifiers)}
                onOpenFolder={(url) => right.navigate(url)}
                onOpenFile={(item) => handleOpenFile(right, item)}
                onItemContextMenu={(item, position) => openMenu('right', position, item)}
                onViewContextMenu={(position) => openMenu('right', position)}
                onClearSelection={right.clearSelection}
                currentUrl={right.currentUrl}
                onUpload={() => triggerUpload(right)}
              />
            )}
          </div>
          <StatusBar
            currentUrl={right.currentUrl}
            totalCount={rightSearchActive ? (right.search.state.data?.nextCursor ? undefined : right.searchItems.length) : right.list.totalCount}
            loadedCount={rightSearchActive ? right.searchItems.length : right.list.loadedCount}
            hasMore={rightSearchActive ? !!right.search.state.data?.nextCursor : right.list.hasMore}
            selectedItems={rightSelectedItems}
            onCopy={copyText}
            onExpandSidebar={
              previewCollapsed ? () => setPreviewCollapsed(false) : undefined
            }
          />
        </section>
      ) : null}

      {!previewCollapsed ? (
        <aside className={clsx("flex w-[320px] max-w-full shrink-0 flex-col overflow-hidden border-l border-[color:var(--cp-border)] bg-[color:var(--cp-surface)]", (width < 1300 || splitActive) && "absolute bottom-0 top-[128px] right-0 z-20 shadow-xl")}>
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
              items={[...focusedPane.selectedItemsMap.values()]}
              onOpenFile={(item) => handleOpenFile(focusedPane, item)}
              topics={topicList}
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

      {dialogs}
      <TransfersPanel onOpenTarget={(path) => focusedPane.revealOriginal(path)} />
      <NamePromptDialog request={namePrompt} onClose={() => setNamePrompt(null)} />

      {toast ? (
        <div className="pointer-events-none absolute bottom-8 left-1/2 -translate-x-1/2 rounded-full bg-black/80 px-3 py-1.5 text-xs text-white">
          {toast}
        </div>
      ) : null}
    </div>
  )
}
