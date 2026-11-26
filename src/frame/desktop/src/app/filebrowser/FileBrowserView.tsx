import { IconButton, useMediaQuery } from '@mui/material'
import clsx from 'clsx'
import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react'
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
import { ArchivePasswordDialog } from '../../components/preview/ArchivePasswordDialog'
import { normalizeCyfsPath } from '../../components/preview/session'
import type { ContentRef, PreviewSessionContext } from '../../components/preview/types'
import { useI18n } from '../../i18n/provider'
import { contentDescriptor, type HandlerPlan } from 'buckyos/content'
import { openContent, contentHandlers, handlerLabel, contentRegistry } from '../content/open'
import { openPreview } from '../preview/launch'
import { useAppPath } from '../pathContext'
import { appPath } from '../standalone/address'
import type { FilesOpenRequest } from './launch'
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
import { useTransfersPanelState } from './useTransfers'
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
import { PermissionsDialog } from './dialogs/PermissionsDialog'
import type { PermissionsRequest } from './dialogs/PermissionsDialog'
import { commandState } from './menu/commands'
import { SELECT_DIALOG_PATH, SHARE_DIALOG_PATH, useSystemDialog } from '../../sysdlg'
import type { SelectedContent } from 'buckyos/content'
import { classifyFileKind } from './data/fileKinds'
import '../../components/preview/files.css'
import './filebrowser.css'
import type { FileItem } from './data/FolderReader'
import { fileBrowserDataMode, installFileBrowserData } from './data/install'
import { resolveReader } from './data/readerRegistry'
import { collectionTitleSchema, entryNameSchema, validationFallback } from './data/schemas'
import { dfsFolderChildren, forgetFolderChildren, useSidebarDfs, useSidebarHost, useSidebarTopics } from './data/sidebarSources'
import { useMyHome, type MyHome } from './data/myHome'
import { setIconSize, useIconSize, type IconSize } from './data/iconSize'
import { useBrowseArchives } from './data/archiveBrowse'
import { nfspClient } from './data/nfsp/client'
import { hostChildren, hostContentRef, hostGrantExpiry, hostRootUrl, isHostUnlocked, isSudoRequired, lockHost, newHostSessionKey, onHostChanged, unlockHost } from './data/hostRealm'
import { useSudoByPassword } from '../../components/sudo'
import { isMockRuntime } from '../../runtime'
import { mockHostRequestPassword } from './mock/host'
import { toUiError } from './data/state'
import { stashLocalInput, transferStore, type LocalUpload } from './data/transfers'
import { describeUpload, droppedUploads, selectedUploads } from './data/localUploads'
import { CONTENT_REFS_FORMAT, TRANSFER_HANDLE_FORMAT, transferTypes } from 'buckyos/transfer'
import { readZoneContent, type ZoneContent } from 'buckyos/content'
import {
  afterShortcut, copyWithCommand, desktopTransfers, dfsPathOfContent, isEditableTarget, isZoneEntry, readClipboardCommand, readTransferEvent,
  offerWindowSelection, registerWindowTransferTarget, writeTransferEvent, zoneContentOfEntry,
  type TransferDelivery, type TransferDeliveryResult, type TransferItemStatus, type TransferOperation, type TransferPoint, type TransferProbe, type TransferProbeAnswer,
} from '../../transfer'
import {
  allowedEffect, classifyIncoming, contentOfAddress, cutEntryIds, effectAllowedOf, mayCarryFiles, offerEntries, payloadOf, realmOfPath,
  referenceEntries, remainingEntries, resolveZoneItems, type FilesTransferPayload, type Incoming,
} from './data/transfer'
import { useBrowserPane } from './data/useBrowserPane'
import {
  COLLECTION_SCHEME,
  collectionUrl,
  copyPathText,
  crumbsForUrl,
  dfsPathOf,
  displayPath,
  LIBRARY_URL,
  normalizeUrl,
  parentUrl,
  parseDeviceUrl,
  pathOfFileUri,
  deviceUrl,
  PUBLISH_URL,
  archiveRootOf,
  archiveRootUrl,
  archiveSplit,
  isArchiveName,
} from './data/urls'
import type {
  BrowserTab,
  DfsNode,
  FileEntry,
  SortDir,
  SortKey,
  Topic,
} from './types'

installFileBrowserData()

/** Stable empty list while topics are loading/errored — a fresh `[]` per render would re-run preview enrichment. */
const NO_TOPICS: Topic[] = []

/** BuckyOS sudo for the computer tree lasts 30 minutes (Verify Hub's cap). */
const HOST_SUDO_SECONDS = 30 * 60

/** Session-local tab id (Volatile, §6.2) — module-level so the React
 * Compiler treats event-handler calls as opaque rather than render-impure. */
function newTabId(): string {
  return `tab-${crypto.randomUUID()}`
}

function itemOf(entry: FileEntry): FileItem {
  return { key: entry.id, entry }
}

/** The folder path file operations take: a DFS path, or the computer folder's device URL. */
function writablePathOf(url: string): string | null {
  return dfsPathOf(url) ?? (parseDeviceUrl(url) ? url : null)
}

/** The request each window followed last. */
const followedRequests = new Map<string, string>()

type BrowserPane = ReturnType<typeof useBrowserPane>

/** Every open Files window's panes: a cut can be pasted in another window, and a move updates the selection of all. */
const paneReconcilers = new Set<() => Array<(ids: Set<string>) => void>>()

/** A cut being moved: its record and each entry's item key there. */
type CutLink = { offerId: string; keys: Map<string, string> }
/** What a drop asked for with its modifier keys. */
type DropIntent = { copy: boolean; move: boolean }

type FileBrowserProps = { windowId?: string; openRequest?: FilesOpenRequest }

/** Files opens on "My files" — the signed-in user's own home — once it is known, so `/home` never flashes. */
export function FileBrowserView(props: FileBrowserProps) {
  const home = useMyHome()
  if (!home) return <div className="filebrowser files-ui flex h-full w-full items-center justify-center" style={{ background: 'var(--cp-bg)' }} data-testid="filebrowser-starting" aria-busy="true" />
  return <FileBrowserWindow {...props} home={home} />
}

function FileBrowserWindow({ windowId, openRequest, home }: FileBrowserProps & { home: MyHome }) {
  const { t } = useI18n()
  const homeUrl = normalizeUrl(home.path)
  const myFilesLabel = t('filebrowser.sidebar.myFiles', 'My files')
  // First open: one "My files" tab, or the folder the launch names (no extra history step).
  const [initialTabs] = useState<BrowserTab[]>(() => [{ id: 'tab-home', title: myFilesLabel, path: openRequest?.path ?? home.path }])
  const [openedAt] = useState(() => normalizeUrl(openRequest?.path ?? home.path))
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

  const left = useBrowserPane(initialTabs, windowId ? `${windowId}:left` : undefined, home.path)
  const right = useBrowserPane([], windowId ? `${windowId}:right` : undefined, home.path)
  // A computer folder's URL names this window's sudo session: it is never an address.
  useAppPath(appPath('', { path: left.currentUrl !== homeUrl ? dfsPathOf(left.currentUrl) : null }))
  const livePanes = useRef([left, right])
  useEffect(() => { livePanes.current = [left, right] }, [left, right])
  useEffect(() => {
    const panes = () => livePanes.current.map((pane) => pane.reconcileMovedEntries)
    paneReconcilers.add(panes)
    return () => { paneReconcilers.delete(panes) }
  }, [])
  const [, refreshCopyCapability] = useState(0)
  useEffect(() => { const refresh = () => refreshCopyCapability((n) => n + 1); window.addEventListener('files-copy-capability', refresh); return () => window.removeEventListener('files-copy-capability', refresh) }, [])
  const [closedTabs, setClosedTabs] = useState<BrowserTab[]>([])
  const transfersPanel = useTransfersPanelState()
  const [focusedSide, setFocusedSide] = useState<'left' | 'right'>('left')
  // A launch that names a folder (the Desktop folder, a folder on the desktop) shows it in the left pane.
  const handledOpenRequest = useRef<string | null>(null)
  useEffect(() => {
    if (!openRequest || handledOpenRequest.current === openRequest.requestId) return
    handledOpenRequest.current = openRequest.requestId
    // A phone remounts the panel with the request it already followed.
    if (windowId && followedRequests.get(windowId) === openRequest.requestId) return
    if (windowId) followedRequests.set(windowId, openRequest.requestId)
    if (openRequest.path) left.navigate(openRequest.path)
    setFocusedSide('left')
  }, [openRequest, left, windowId])

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
  /** The root of an archive whose names are encrypted, waiting for its password. */
  const [archivePassword, setArchivePassword] = useState<{ pane: BrowserPane; url: string; name: string } | null>(null)
  const [defaultApp, setDefaultApp] = useState<{ plans: HandlerPlan[]; request: Parameters<typeof openContent>[0]; name: string; size?: number; key: string; always: boolean; selector: string } | null>(null)
  const [previewCollapsed, setPreviewCollapsed] = useState(true)
  // A pending cut is the desktop's record: its marking follows the record (moved items, Esc, a newer copy).
  useSyncExternalStore(desktopTransfers.subscribe, desktopTransfers.getVersion)
  const pendingCut = desktopTransfers.cutOf('files', windowId)
  /** A paste or drop that needs the user's choice first (a stale cut, BuckyOS addresses, a move that can only copy). */
  const [transferChoice, setTransferChoice] = useState<{ message: string; confirmLabel: string; onConfirm: () => void } | null>(null)
  const dragOffer = useRef<string | null>(null)
  /** Active form dialog (new collection/group, rename, new folder). */
  const [namePrompt, setNamePrompt] = useState<NamePromptRequest | null>(null)
  const [deleteRequest, setDeleteRequest] = useState<DeleteRequest | null>(null)
  const { batchTask, conflictRequest } = useOperationFeedback()
  const { setBatchTask } = operationFeedback
  const [permissionsRequest, setPermissionsRequest] = useState<PermissionsRequest | null>(null)
  const systemDialog = useSystemDialog()
  const busyRef = useRef(false)
  const [dragTarget, setDragTarget] = useState<string | null>(null)
  const [pathEditSignal, setPathEditSignal] = useState(0)

  // Sidebar sources — separate async states so one failing source never
  // blanks the browser (§4.3).
  const dfsSource = useSidebarDfs()
  const hostSource = useSidebarHost()
  const topicsSource = useSidebarTopics()
  const iconSize = useIconSize()
  const browseArchivesOn = useBrowseArchives()
  /** A `.zip` this server opens, with "Browse archives as folders" on, opens as a folder. */
  const enterArchives = (item: FileItem) =>
    browseArchivesOn && item.entry.kind !== 'folder' && item.entry.path.startsWith('/') && !archiveSplit(item.entry.path) &&
    isArchiveName(item.entry.name) && nfspClient().raw.opensArchive(item.entry.name)

  // ─── The computer (Desktop Lite): unlocked per window with BuckyOS sudo ───
  const [hostKey] = useState(newHostSessionKey)
  const [hostUnlocked, setHostUnlocked] = useState(false)
  const hostInfo = hostSource.state.data ?? null
  const requestSudo = useSudoByPassword()
  const unlocking = useRef<Promise<boolean> | null>(null)
  useEffect(() => onHostChanged(hostKey, () => setHostUnlocked(isHostUnlocked(hostKey))), [hostKey])
  // Closing the window drops the token and the sudo session at once.
  useEffect(() => () => lockHost(hostKey), [hostKey])
  // The grant ends on time even when nothing is listing.
  useEffect(() => {
    if (!hostUnlocked) return
    const expiry = hostGrantExpiry(hostKey)
    if (!expiry) return
    const timer = window.setTimeout(() => lockHost(hostKey), Math.max(0, expiry - Date.now()))
    return () => window.clearTimeout(timer)
  }, [hostKey, hostUnlocked])
  /** Asks for the BuckyOS sudo password (once at a time); resolves true when the computer is unlocked. */
  const unlockComputer = useCallback((): Promise<boolean> => {
    if (unlocking.current) return unlocking.current
    const attempt = (async () => {
      const grant = await requestSudo({
        aud: 'nfs-server',
        expiresInSecs: HOST_SUDO_SECONDS,
        title: t('filebrowser.host.unlockTitle', 'Unlock {{name}}', { name: hostInfo?.label ?? t('filebrowser.host.computer', 'this computer') }),
        reason: t('filebrowser.host.unlockReason', 'See every folder on this computer.'),
        confirmLabel: t('filebrowser.host.unlock', 'Unlock'),
        // The sudo account is the user the file service knows.
        ...(home.userId ? { username: home.userId } : {}),
        ...(fileBrowserDataMode() === 'mock'
          ? { username: 'devtest', appid: 'control-panel', requestPassword: mockHostRequestPassword }
          : isMockRuntime() ? { appid: 'control-panel' } : {}),
      }).catch(() => null)
      if (!grant) return false
      try {
        await unlockHost(hostKey, { token: grant.sessionToken, expiresAtMs: grant.expiresAtMs })
        return true
      } catch {
        return false
      }
    })().finally(() => { unlocking.current = null })
    unlocking.current = attempt
    return attempt
  }, [requestSudo, hostKey, hostInfo, home.userId, t])
  /** Runs an operation; a computer that relocked mid-way asks for sudo again and retries it once. */
  const withHostRetry = useCallback(async <T,>(operation: () => Promise<T>): Promise<T> => {
    try {
      return await operation()
    } catch (err) {
      if (!isSudoRequired(err) || !await unlockComputer()) throw err
      return operation()
    }
  }, [unlockComputer])
  const hostRoot = hostRootUrl(hostKey)
  /** Location input from the address bar: a pasted `file:` URI opens the computer folder it names. */
  const toLocation = (input: string): string | null => {
    const filePath = pathOfFileUri(input)
    if (filePath === null) return input
    if (!hostInfo) { showToast(t('filebrowser.host.unavailable', 'This computer\'s folders cannot be opened here')); return null }
    return deviceUrl(hostKey, filePath)
  }
  /** Tabs on a fixed sidebar place carry its name. */
  const placeTitles: Record<string, string | undefined> = {
    [PUBLISH_URL]: t('filebrowser.sidebar.published', 'Published'),
    [LIBRARY_URL]: t('filebrowser.sidebar.library', 'Library'),
    [hostRoot]: hostInfo?.label,
    [homeUrl]: myFilesLabel,
  }
  const titledTabs = (tabs: BrowserTab[]) => tabs.map((tab) => ({ ...tab, title: placeTitles[tab.path] ?? tab.title }))
  const topicList = topicsSource.state.data ?? NO_TOPICS

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
  const uploadRequest = openRequest?.upload ? openRequest.requestId : null
  useEffect(() => { if (uploadRequest) setMobileUploadOpen(true) }, [uploadRequest])

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
    left.adoptTab({ id: newTabId(), title: myFilesLabel, path: home.path })
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
  // The focused pane's Zone selection, offered to the Shell: its drag handle carries it to an App in another window
  // (剪贴板规范 §5) — the only way into a cross-origin App, which a native drag from this document does not reach.
  const offeredSelection = focusedSide === 'right' ? rightSelectedItems : leftSelectedItems
  useEffect(() => {
    const zone = offeredSelection.map((item) => item.entry).filter(isZoneEntry)
    if (!windowId || !zone.length || zone.length !== offeredSelection.length) return
    return offerWindowSelection('files', windowId, {
      items: zone.map((entry) => ({ kind: entry.kind === 'folder' ? 'zone-folder' as const : 'zone-file' as const, formats: { [CONTENT_REFS_FORMAT]: zoneContentOfEntry(entry) } })),
      operations: ['copy', 'link'],
    })
  }, [offeredSelection, windowId])
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
  /** Folders the computer's account cannot open stay closed (shown with a lock). */
  const openable = (entry: FileEntry) => {
    if (entry.accessLevel !== 'none' || !parseDeviceUrl(entry.path)) return true
    showToast(t('filebrowser.host.noAccess', 'This computer account cannot open "{{name}}"', { name: entry.name }))
    return false
  }

  // ─── Preview App hand-off (PRD §14.1 / §14.4): Source + Session Context ───
  // Folder listings become a Container Context; views, collections and search
  // results hand over an explicit list of the loaded items instead.

  const previewSourceOf = (item: FileItem): ContentRef =>
    parseDeviceUrl(item.entry.path)
      ? hostContentRef(item.entry)
      : { kind: 'cyfs-path', path: normalizeCyfsPath(item.entry.path) }
  /** Icon-view thumbnails read files in the Zone and on the unlocked computer. */
  const thumbnailSource = useCallback((entry: FileEntry): ContentRef | null => {
    // Members of an archive are decoded on read: no thumbnails for a whole listing of them.
    if (entry.kind === 'folder' || entry.accessLevel === 'none' || archiveSplit(entry.path)) return null
    if (entry.path.startsWith('/')) return { kind: 'cyfs-path', path: normalizeCyfsPath(entry.path) }
    return parseDeviceUrl(entry.path) ? hostContentRef(entry) : null
  }, [])

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
    if (!opts?.newWindow && enterArchives(item)) {
      pane.navigate(archiveRootUrl(item.entry.path))
      return
    }
    void openContent({
      source: previewSourceOf(item),
      session: previewSessionOf(pane, item),
      origin: { app: 'files', hostContext: pane.currentUrl, windowId },
      newWindow: opts?.newWindow,
    }, { name: item.entry.name, size: item.entry.sizeBytes })
  }

  /** Multi-selection → stable explicit list; only the chosen files take part (§14.4). */
  const handlePreviewItems = (pane: BrowserPane, chosen: FileItem[]) => {
    const files = chosen.filter((entry) => entry.entry.kind !== 'folder')
    if (!files.length) return
    if (files.length === 1) {
      openPreview({ source: previewSourceOf(files[0]), session: previewSessionOf(pane, files[0]), origin: { app: 'files', windowId } })
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
      if (openable(item.entry)) handleOpenFolder(item.entry.path)
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
    const parent = writablePathOf(pane.currentUrl)
    if (!parent || !pane.list.capabilities.acceptsContent) return
    setNamePrompt({
      title: t('filebrowser.actions.newFolder', 'New folder'),
      label: t('filebrowser.prompt.folderName', 'Folder name'),
      submitLabel: t('filebrowser.actions.create', 'Create'),
      schema: entryNameSchema,
      onSubmit: async (value) => {
        try {
          await withHostRetry(async () => {
            if (await folderOps().nameExists(parent, value)) {
              throw new Error(
                t('filebrowser.error.nameConflict', '"{{name}}" already exists here', {
                  name: value,
                }),
              )
            }
            await folderOps().createFolder(parent, value)
          })
          forgetFolderChildren(pane.currentUrl)
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
          await withHostRetry(async () => {
            if (await folderOps().nameExists(parent, value)) {
              throw new Error(
                t('filebrowser.error.nameConflict', '"{{name}}" already exists here', {
                  name: value,
                }),
              )
            }
            await folderOps().renameEntry(captured, value)
          })
          if (captured.kind === 'folder') forgetFolderChildren(normalizeUrl(parent))
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

  const enqueueInputs = async (target: string, inputs: LocalUpload[]) => {
    const parent = writablePathOf(target)
    if (!parent) { showToast(t('filebrowser.operation.localUploadUnsupported', 'Local files can only be uploaded to a writable folder')); return }
    const resolveConflict = conflictResolver()
    let cancelled = false
    for (const input of inputs) {
      const candidate = { localId: crypto.randomUUID(), ...describeUpload(input) }
      const retry = () => void enqueueInputs(target, [input])
      if (cancelled) { transferStore.recordOutcome(target, candidate, 'cancelled', null, retry); continue }
      try {
        let name = candidate.name
        entryNameSchema.parse(name)
        const existing = await withHostRetry(() => folderOps().statEntry(parent, name))
        if (existing) {
          const choice = await resolveConflict({ source: { id: '', path: name, name, kind: input.kind === 'file' ? 'other' : 'folder', sizeBytes: candidate.sizeBytes, modifiedAt: new Date().toISOString() }, target: existing, targetPath: parent })
          if (choice === 'cancel') { cancelled = true; transferStore.recordOutcome(target, candidate, 'cancelled', null, retry); continue }
          if (choice === 'skip') { transferStore.recordOutcome(target, candidate, 'skipped'); continue }
          name = await availableName(parent, name)
        }
        stashLocalInput(candidate.localId, input)
        const { rejected } = transferStore.enqueue(target, [{ ...candidate, name }])
        for (const reject of rejected) {
          const key = reject.messageKeys[0]
          transferStore.recordOutcome(target, candidate, 'error', { code: 'VALIDATION', messageKey: key, fallback: validationFallback[key] ?? key, retryable: false })
        }
      } catch (error) { transferStore.recordOutcome(target, candidate, 'error', toUiError(error), retry) }
    }
  }

  const enqueueFiles = (target: string, files: FileList | File[] | null) => {
    if (!files?.length) return
    return enqueueInputs(target, selectedUploads([...files]))
  }

  const triggerUpload = (pane: BrowserPane, accept?: string, folder = false) => {
    if (!pane.list.capabilities.acceptsContent || pane.searchQuery) return
    const target = pane.currentUrl
    const input = document.createElement('input')
    input.type = 'file'
    input.multiple = true
    if (accept) input.accept = accept
    if (folder) input.webkitdirectory = true
    input.onchange = () => {
      if (folder && !input.files?.length) { showToast(t('filebrowser.operation.emptyFolderPicker', 'This browser cannot select an empty folder; drag the folder here instead')); return }
      void enqueueFiles(target, input.files)
    }
    input.click()
  }

  // ─── Context menu (desktop) ───

  /** Quick "Move to" targets: my files and the DFS roots (real storage destinations only). */
  const dfsRoots = dfsSource.state.data
  const moveTargets = useMemo(() => {
    const roots: DfsNode[] = dfsRoots ?? []
    const paths = [home.path, ...roots.map((node) => node.path).filter((path) => path.startsWith('/') && path !== home.path)]
    return paths.map((path) => ({ label: path, path }))
  }, [dfsRoots, home.path])

  const [contextMenu, setContextMenu] = useState<{
    side: 'left' | 'right'
    position: MenuPosition
    context: FileMenuContext
    sections: FileMenuSection[]
  } | null>(null)

  const paneFor = (side: 'left' | 'right') => (side === 'right' ? right : left)

  // ─── Clipboard and drags (剪贴板规范 §3, §6): Zone references for every App, a desktop record for Files ───

  /** A copy, cut or drag of these items: what the source allows and why it refuses the rest. */
  const offerItems = (pane: BrowserPane, purpose: 'copy' | 'cut' | 'drag', items: FileItem[]) => {
    const entries = items.map((item) => item.entry)
    const move = commandState('move-to', contextFor(pane, items))
    const moves = purpose === 'cut' || (purpose === 'drag' && move.state === 'available')
    const operations: TransferOperation[] = ['copy', ...(entries.every(isZoneEntry) ? ['link' as const] : []), ...(moves ? ['move' as const] : [])]
    return offerEntries(purpose, entries, { windowId, side: pane === right ? 'right' : 'left' }, operations, move.state === 'available' ? {} : { move: move.reason })
  }
  const copiedToast = (purpose: 'copy' | 'cut', count: number) => showToast(purpose === 'cut'
    ? t('filebrowser.toast.cut', 'Cut {{count}} item(s)', { count })
    : t('filebrowser.toast.copyItems', 'Copied {{count}} item(s)', { count }))
  /** Copy / Cut from the toolbar or a menu: the same representations through the async clipboard. */
  const copyCommand = async (pane: BrowserPane, purpose: 'copy' | 'cut', items: FileItem[]) => {
    if (!items.length) return
    const { offer, representations } = offerItems(pane, purpose, items)
    const result = await copyWithCommand(representations)
    if (result.ok && result.complete) { copiedToast(purpose, items.length); return }
    // Without `text/html` the handle did not reach the clipboard: nothing there can be pasted as these items.
    desktopTransfers.withdraw(offer.offerId)
    showToast(result.ok
      ? t('filebrowser.transfer.namesOnly', 'Only the names were copied, as text. Press Ctrl/⌘C to copy the items.')
      : t('filebrowser.transfer.writeFailed', 'Could not write to the clipboard. Press Ctrl/⌘C to copy.'))
  }

  /** Runs a batch and shows its results; null when it did not start (another operation runs, the items of a cut are
   * being moved elsewhere). A move of a cut holds its items for this operation and consumes the ones that moved. */
  const runBatch = async (pane: BrowserPane, items: FileItem[], kind: 'copy' | 'move' | 'delete' | 'remove-ref' | 'add-ref', target?: string, cut?: CutLink, previousResults: OperationResult[] = [], retryOf?: string, requestKey = crypto.randomUUID(), resumeTaskId?: string): Promise<OperationResult[] | null> => {
    if (!items.length) return null
    const cutKeys = kind === 'move' && cut ? items.flatMap((item) => cut.keys.get(item.entry.id) ?? []) : []
    let holding = false
    if (cut && cutKeys.length) {
      const held = desktopTransfers.reserve(cut.offerId, cutKeys, requestKey)
      if (!held.ok && held.reason === 'busy') { showToast(t('filebrowser.transfer.busy', 'These items are already being moved in another window.')); return null }
      // A retry of a move the user started still runs when the cut itself is gone.
      holding = held.ok
    }
    if (operationFeedback.isRunning()) {
      if (holding) desktopTransfers.release(cut!.offerId, cutKeys, requestKey)
      showToast(t('filebrowser.commandReason.Wait for the current operation to finish', 'Wait for the current operation to finish'))
      return null
    }
    busyRef.current = true
    const selectedAtStart = [...pane.selectedItemsMap.values()]
    const reconcileSources = [...paneReconcilers].flatMap((panes) => panes())
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
    if (holding) {
      const moved = cutKeys.filter((key) => captured.some((item) => succeeded.has(item.entry.id) && cut!.keys.get(item.entry.id) === key))
      desktopTransfers.consume(cut!.offerId, moved, requestKey)
      desktopTransfers.release(cut!.offerId, cutKeys.filter((key) => !moved.includes(key)), requestKey)
    }
    const retry = () => {
      if (kind === 'copy') { void runBatch(pane, captured, kind, target, undefined, [], monitorFailed ? undefined : copyTaskId, monitorFailed ? requestKey : undefined, monitorFailed ? copyTaskId : undefined); return }
      const failed = new Set(results.filter((result) => result.status === 'failed').map((result) => result.entry.id))
      const current = livePanes.current.find((current) => current.activeTabId === pane.activeTabId && current.currentUrl === pane.currentUrl && [...current.selectedItemsMap.values()].every((item) => failed.has(item.entry.id)))
      void runBatch(current ?? pane, captured.filter((item) => failed.has(item.entry.id)), kind, target, cut, results.filter((result) => result.status !== 'failed'))
    }
    setBatchTask((current) => ({ ...initial, ...current, taskId: copyTaskId, summary: monitorFailed ? undefined : current?.summary, results, running: false, cancelling: false, retry }))
    if (kind !== 'add-ref' && kind !== 'remove-ref') {
      // A move or delete also changes the folders the items came from (a cut pasted in another pane or window).
      const sources = kind === 'copy' ? [] : captured.flatMap((item) => parentUrl(normalizeUrl(item.entry.path)) ?? [])
      for (const folder of new Set([pane.currentUrl, ...sources, ...(target ? [normalizeUrl(target)] : [])])) forgetFolderChildren(folder)
    }
    // The computer relocked under the operation: ask for sudo again, then redo what failed.
    if (results.some((result) => result.error?.code === 'SUDO_REQUIRED')) void unlockComputer().then((ok) => { if (ok) retry() })
    return results
  }

  const moveSelected = (pane: BrowserPane, entries: FileEntry[], target: string) => runBatch(pane, entries.map(itemOf), 'move', target)
  const zonePath = (item: SelectedContent) => item.origin === 'zone' && item.source.kind === 'cyfs-path' ? item.source.path.slice('cyfs://'.length) : null
  const pickDestination = async (pane: BrowserPane, entries: FileEntry[], copy: boolean): Promise<string | null> => {
    const result = await systemDialog.open(SELECT_DIALOG_PATH, {
      mode: 'folder', writable: true,
      start: { kind: 'cyfs-path', path: normalizeCyfsPath(dfsPathOf(pane.currentUrl) ?? home.path) },
      title: `${copy ? t('filebrowser.actions.copyTo', 'Copy to') : t('filebrowser.actions.moveTo', 'Move to')} · ${entries.length}`,
      confirmLabel: copy ? t('filebrowser.operation.copyHere', 'Copy here') : t('filebrowser.operation.moveHere', 'Move here'),
      exclude: entries.filter((entry) => entry.kind === 'folder' && entry.path.startsWith('/')).slice(0, 1000).map((entry) => ({ kind: 'cyfs-path' as const, path: normalizeCyfsPath(entry.path) })),
    }, { id: 'files' })
    return result.action === 'selected' ? zonePath(result.items[0]) : null
  }
  const requestCopy = (pane: BrowserPane, entries: FileEntry[]) => { void pickDestination(pane, entries, true).then((path) => { if (path) void runBatch(pane, entries.map(itemOf), 'copy', path) }) }
  const requestMove = (pane: BrowserPane, entries: FileEntry[]) => { void pickDestination(pane, entries, false).then((path) => { if (path) void moveSelected(pane, entries, path) }) }
  const requestExisting = (pane: BrowserPane) => {
    const url = pane.currentUrl
    void systemDialog.open(SELECT_DIALOG_PATH, {
      mode: 'open', multiple: true, title: t('filebrowser.operation.addExisting', 'Add existing files'), confirmLabel: t('filebrowser.operation.addReferences', 'Add references'),
    }, { id: 'files-references' }).then((result) => {
      if (result.action !== 'selected') return
      const entries = result.items.flatMap((item): FileEntry[] => {
        const path = zonePath(item)
        return path ? [{ id: path, name: item.origin === 'zone' && item.hints.name || path.split('/').pop() || path, kind: classifyFileKind(path), path, modifiedAt: '', sizeBytes: item.origin === 'zone' ? item.hints.size : undefined }] : []
      })
      if (entries.length) void runBatch(pane, entries.map(itemOf), 'add-ref', url)
    })
  }
  /** Where a paste or drop lands in a pane: references in a collection, content in a writable folder. */
  const destinationOf = (pane: BrowserPane) => {
    const searching = !!pane.searchQuery
    return {
      collection: !searching && pane.list.capabilities.acceptsReferences,
      folder: !searching && pane.list.capabilities.acceptsContent ? writablePathOf(pane.currentUrl) : null,
    }
  }
  /** How a Files selection lands in a folder: move within one realm by default, copy across realms or when the
   * source refuses a move; Ctrl/Alt copies, Shift moves (§6). */
  const filesOperation = (payload: FilesTransferPayload, offer: { operations: readonly TransferOperation[]; purpose: string }, folder: string, intent?: DropIntent): 'copy' | 'move' | 'ask' => {
    if (offer.purpose === 'cut') return 'move'
    if (offer.purpose === 'copy' || intent?.copy) return 'copy'
    const movable = offer.operations.includes('move')
    if (intent?.move) return movable ? 'move' : 'ask'
    return movable && payload.realm === realmOfPath(folder) ? 'move' : 'copy'
  }
  const reasonText = (reason: string | undefined) => reason ? t(`filebrowser.commandReason.${reason}`, reason) : t('filebrowser.operation.unavailable')
  /** Items that could not even start (gone, object references) are reported like a finished batch. */
  const reportFailed = (title: string, failed: OperationResult[]) => {
    setBatchTask({ ownerId: windowId, title, results: failed, total: failed.length, running: false, cancel: () => {}, retry: () => {} })
    return failed
  }
  /** Copies Zone references into a folder: each path is looked up with this user's session first. */
  const copyReferences = async (pane: BrowserPane, items: ZoneContent[], folder: string) => {
    if (!folderOps().supportsCopy) { showToast(reasonText(folderOps().copyUnavailableReason ?? 'File content copying is not supported')); return null }
    const { entries, failed } = await resolveZoneItems(items, (parent, name) => withHostRetry(() => folderOps().statEntry(parent, name)))
    if (!entries.length) return reportFailed(`${t('filebrowser.actions.copyTo', 'Copy to')} · ${folder}`, failed)
    return runBatch(pane, entries.map(itemOf), 'copy', folder, undefined, failed)
  }
  const linkReferences = async (pane: BrowserPane, items: ZoneContent[]) => {
    const { entries, failed } = referenceEntries(items)
    if (!entries.length) return reportFailed(`${t('filebrowser.operation.addReferences', 'Add references')} · ${displayPath(pane.currentUrl)}`, failed)
    return runBatch(pane, entries.map(itemOf), 'add-ref', pane.currentUrl, undefined, failed)
  }
  const askTransfer = (message: string, confirmLabel: string, onConfirm: () => void) => setTransferChoice({ message, confirmLabel, onConfirm })
  const closeTransferChoice = () => { setTransferChoice(null); rootRef.current?.focus() }

  /**
   * One paste or drop into a pane, in the registered order (§2.3): a live Files record (cut → move, copy → copy, drag →
   * move or copy), Zone references from any App (copy into a folder, references in a collection), local files (upload),
   * BuckyOS addresses (only after the user chooses), text (explained). A handle whose record is gone never brings back an
   * earlier selection; a stale cut asks before copying (§6).
   */
  const receive = (pane: BrowserPane, incoming: Incoming, options: { intent?: DropIntent; local?: () => Promise<LocalUpload[]> } = {}) => {
    const { collection, folder } = destinationOf(pane)
    const count = (n: number) => ({ count: n })
    if (incoming.kind === 'text') { showToast(t('filebrowser.transfer.textOnly', 'This is text. Files can only paste files and folders here.')); return }
    if (incoming.kind === 'empty') { showToast(t('filebrowser.transfer.empty', 'There is nothing here Files can paste.')); return }
    if (incoming.kind === 'stale') { showToast(t('filebrowser.transfer.stale', 'The copied items are no longer available. Copy them again.')); return }
    if (!collection && !folder) { showToast(reasonText(pane.searchQuery ? 'Choose a writable destination folder' : 'Choose a writable folder')); return }
    if (incoming.kind === 'local') {
      if (!folder) { showToast(t('filebrowser.operation.localUploadUnsupported', 'Local files can only be uploaded to a writable folder')); return }
      void options.local?.().then((inputs) => enqueueInputs(pane.currentUrl, inputs)).catch((error: unknown) => showToast(String(error)))
      return
    }
    if (incoming.kind === 'addresses') {
      const items = incoming.refs.map(contentOfAddress)
      if (collection) askTransfer(t('filebrowser.transfer.addressesLink', 'Add references to the {{count}} item(s) these BuckyOS addresses name?', count(items.length)), t('filebrowser.operation.addReferences', 'Add references'), () => void linkReferences(pane, items))
      else askTransfer(t('filebrowser.transfer.addressesCopy', 'Copy the {{count}} item(s) these BuckyOS addresses name here?', count(items.length)), t('filebrowser.operation.copyHere', 'Copy here'), () => void copyReferences(pane, items, folder!))
      return
    }
    if (incoming.kind === 'refs') {
      if (collection) { void linkReferences(pane, incoming.items); return }
      const copy = () => void copyReferences(pane, incoming.items, folder!)
      if (incoming.staleCut) askTransfer(t('filebrowser.transfer.cutStale', 'The cut is no longer available. Copy {{count}} item(s) here instead?', count(incoming.items.length)), t('filebrowser.transfer.copyInstead', 'Copy instead'), copy)
      else if (options.intent?.move) askTransfer(t('filebrowser.transfer.moveUnavailable', '{{count}} item(s) cannot be moved here. Copy them instead?', count(incoming.items.length)), t('filebrowser.transfer.copyInstead', 'Copy instead'), copy)
      else copy()
      return
    }
    const { offer, payload } = incoming
    const remaining = remainingEntries(offer, payload)
    const entries = remaining.map((item) => item.entry)
    if (collection) {
      // A cut added to a collection stays a pending cut: the originals stay in place (§6).
      const zone = entries.filter(isZoneEntry)
      if (!zone.length) { showToast(t('filebrowser.transfer.notHere', 'These items cannot be added here.')); return }
      void runBatch(pane, zone.map(itemOf), 'add-ref', pane.currentUrl)
      return
    }
    const operation = filesOperation(payload, offer, folder!, options.intent)
    const copy = () => {
      if (!folderOps().supportsCopy) { showToast(reasonText(folderOps().copyUnavailableReason ?? 'File content copying is not supported')); return }
      void runBatch(pane, entries.map(itemOf), 'copy', folder!)
    }
    if (operation === 'ask') { askTransfer(t('filebrowser.transfer.moveUnavailable', '{{count}} item(s) cannot be moved here. Copy them instead?', count(entries.length)), t('filebrowser.transfer.copyInstead', 'Copy instead'), copy); return }
    if (operation === 'copy') { copy(); return }
    // Dropping items back where they are is no move.
    if (offer.purpose === 'drag' && entries.every((entry) => parentUrl(normalizeUrl(entry.path)) === normalizeUrl(folder!))) return
    void runBatch(pane, entries.map(itemOf), 'move', folder!, offer.purpose === 'cut' ? { offerId: offer.offerId, keys: new Map(remaining.map((item) => [item.entry.id, item.key])) } : undefined)
  }
  /** Paste from the toolbar or a menu: reads the system clipboard inside the user's click. */
  const pasteCommand = async (pane: BrowserPane) => {
    const result = await readClipboardCommand()
    if (!result.ok) { showToast(t('filebrowser.transfer.readDenied', 'The browser did not allow reading the clipboard. Press Ctrl/⌘V to paste.')); return }
    receive(pane, classifyIncoming(result.reader), { local: () => Promise.resolve(selectedUploads(result.reader.files)) })
  }
  const removeFromCollection = (pane: BrowserPane, keys: string[]) => {
    const items = keys.map((key) => pane.selectedItemsMap.get(key) ?? pane.list.loadedItemByKey(key)).filter((item): item is FileItem => !!item)
    setDeleteRequest({ items, location: displayPath(pane.currentUrl), references: true, submit: () => void runBatch(pane, items, 'remove-ref') })
  }
  const deleteSelected = (pane: BrowserPane, items: FileItem[]) => {
    if (!items.length) return
    setDeleteRequest({ items, location: displayPath(pane.currentUrl), references: false, submit: () => void runBatch(pane, items, 'delete') })
  }
  const downloadEntries = async (entries: FileEntry[]) => {
    if (entries.some((entry) => parseDeviceUrl(entry.path)) && !isHostUnlocked(hostKey) && !await unlockComputer()) return
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
    viewMode: pane.viewMode, iconSize, capabilities: pane.list.capabilities, sortKey: pane.sortKey,
    collections: collections.map((item) => ({ id: item.id, title: item.title })), moveTargets,
    pane: { canOpenInNewTab: !isMobile, canOpenInRightPane: !isMobile && width >= 700 },
    searching: !!pane.searchQuery, contextToken: pane.contextToken,
    loadedCount: pane.searchQuery ? pane.searchItems.length : pane.list.loadedKeys().length,
    busy: !!batchTask?.running || pane.enumerating,
    otherPath: splitActive ? writablePathOf(pane === left ? right.currentUrl : left.currentUrl) ?? undefined : undefined,
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
      onCopy: commandState('copy', ctx).state === 'available' ? () => void copyCommand(pane, 'copy', items) : undefined,
      onPaste: commandState('paste', ctx).state === 'available' ? () => void pasteCommand(pane) : undefined,
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
    if (items.length === 1 && items[0].entry.kind !== 'folder') {
      void contentHandlers({ source: previewSourceOf(items[0]), session: previewSessionOf(pane, items[0]) }, { name: items[0].entry.name, size: items[0].entry.sizeBytes }).then(plans => {
        context.contentHandlers = plans.map(plan => ({ key: plan.handlerKey, label: handlerLabel(plan) }))
        setContextMenu(current => current?.context === context ? { ...current, sections: fileBrowserMenuRegistry.build(context) } : current)
      })
    }
  }

  /** Dispatch a menu command — shared by the desktop popup and the mobile sheet. */
  const runMenuAction = (
    pane: BrowserPane,
    context: FileMenuContext,
    action: FileMenuAction,
  ) => {
    if (context.contextToken !== pane.contextToken) { showToast(t('filebrowser.operation.contextChanged', 'The selection changed. Open the menu again.')); return }
    const state = commandState(action.command, { ...context, busy: busyRef.current || pane.enumerating })
    if (state.state !== 'available') { showToast(state.reason ? t(`filebrowser.commandReason.${state.reason}`, state.reason) : t('filebrowser.operation.unavailable')); return }
    const items = context.items
    const first = items[0]
    const entries = context.entries
    switch (action.command) {
      case 'open-handler':
      case 'choose-default': {
        if (!first || first.entry.kind === 'folder') break
        const request = { source: previewSourceOf(first), session: previewSessionOf(pane, first), origin: { app: 'files', windowId } }
        void (async () => {
          const plans = await contentHandlers(request, { name: first.entry.name, size: first.entry.sizeBytes })
          const key = action.args?.handlerKey as string | undefined
          if (action.command === 'choose-default') {
            const descriptor = contentDescriptor(request.source as import('buckyos/content').TransferableContentRef, { name: first.entry.name })
            setDefaultApp({ plans, request, name: first.entry.name, size: first.entry.sizeBytes, key: plans[0]?.handlerKey ?? '', always: true, selector: `mime:${descriptor.mime}` })
            return
          }
          await openContent(request, { handlerKey: key, name: first.entry.name, size: first.entry.sizeBytes })
        })().catch(error => showToast(String(error)))
        break
      }
      case 'open':
        if (!first) break
        if (first.entry.kind === 'folder') {
          // Group entries carry collection:// paths; folder refs carry their
          // original dfs path — navigation normalizes both.
          if (openable(first.entry)) pane.navigate(first.entry.path)
        } else {
          pane.applySelection([first.key], new Map([[first.key, first]]))
          handleOpenFile(pane, first)
        }
        break
      case 'preview-new-window':
        if (first && first.entry.kind !== 'folder') handleOpenFile(pane, first, { newWindow: true })
        break
      case 'preview-archive':
        if (first && first.entry.kind !== 'folder') openPreview({ source: previewSourceOf(first), origin: { app: 'files', hostContext: pane.currentUrl, windowId } })
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
      case 'cut': void copyCommand(pane, 'cut', items); break
      case 'copy': void copyCommand(pane, 'copy', items); break
      case 'copy-to': requestCopy(pane, entries); break
      case 'copy-other': {
        const path = String(action.args?.path ?? context.otherPath ?? '')
        if (path) void runBatch(pane, items, 'copy', path)
        break
      }
      case 'paste': void pasteCommand(pane); break
      case 'details':
        if (first) pane.applySelection([first.key], new Map([[first.key, first]]))
        if (isMobile) setMobilePreviewOpen(true)
        else setPreviewCollapsed(false)
        break
      case 'add-existing': requestExisting(pane); break
      case 'copy-path':
        copyText(
          entries.length
            ? entries.map((item) => copyPathText(item.path)).join('\n')
            : copyPathText(context.currentUrl),
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
        void downloadEntries(entries)
        break
      case 'rename':
        if (first) requestRename(pane, first)
        break
      case 'move-to': requestMove(pane, entries); break
      case 'share':
        if (first) void systemDialog.open(SHARE_DIALOG_PATH, { source: first.entry.path, title: first.entry.name })
        break
      case 'permissions':
        if (first) setPermissionsRequest({ path: first.entry.path, name: first.entry.name, onSaved: () => pane.list.reload() })
        break
      case 'folder-permissions': {
        const path = dfsPathOf(context.currentUrl)
        if (path) setPermissionsRequest({ path, name: path.slice(path.lastIndexOf('/') + 1) || path, onSaved: () => pane.list.reload() })
        break
      }
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
      case 'icon-size':
        setIconSize(String(action.args?.size ?? 'medium') as IconSize)
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
    if (items.length === 1 && items[0].entry.kind !== 'folder') void contentHandlers({ source: previewSourceOf(items[0]), session: previewSessionOf(left, items[0]) }, { name: items[0].entry.name, size: items[0].entry.sizeBytes }).then(plans => {
      context.contentHandlers = plans.map(plan => ({ key: plan.handlerKey, label: handlerLabel(plan) }))
      setMobileMenu(current => current?.context === context ? { ...current, sections: fileBrowserMenuRegistry.build(context) } : current)
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

  const leftTabTitle = left.activeTab ? titledTabs([left.activeTab])[0].title : undefined
  const mobileTitleText = (mobilePreviewOpen ? leftSelectedItem?.entry.name : undefined) ?? leftTabTitle ?? 'root'
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

  /** Esc also cancels this window's pending cut: its items are no longer marked or moved by a paste. */
  const cancelCut = () => { if (pendingCut) desktopTransfers.withdraw(pendingCut.offerId) }
  const handleKeyDown = (event: React.KeyboardEvent) => {
    if ((event.target as HTMLElement).closest('input, textarea, select, [contenteditable="true"], [role="dialog"]') || namePrompt || deleteRequest || conflictRequest || transferChoice) return
    const pane = focusedPane
    const items = [...pane.selectedItemsMap.values()]
    const ctx = contextFor(pane, items)
    const cmd = event.ctrlKey || event.metaKey
    let command: string | undefined
    if (cmd && event.key.toLowerCase() === 'a') { event.preventDefault(); void pane.selectAll(); return }
    if (cmd && event.key.toLowerCase() === 'l') { event.preventDefault(); setPathEditSignal((value) => value + 1); return }
    if (event.altKey && event.key === 'ArrowLeft') { event.preventDefault(); pane.back(); return }
    if (event.altKey && event.key === 'ArrowRight') { event.preventDefault(); pane.forward(); return }
    if (event.key === 'Escape') { event.preventDefault(); pane.cancelEnumeration(); pane.clearSelection(); cancelCut(); return }
    const clip = cmd && !event.shiftKey && !event.altKey ? ({ c: 'copy', x: 'cut', v: 'paste' } as const)[event.key.toLowerCase() as 'c' | 'x' | 'v'] : undefined
    if (clip) {
      // The real copy / cut / paste event does the work (see the clipboard listeners); a browser that fires none for
      // the shortcut goes through the async clipboard instead.
      afterShortcut(clip, () => runMenuAction(pane, ctx, { type: 'action', id: clip, command: clip, label: { key: '', fallback: clip } }))
      return
    }
    if (event.metaKey && event.key === 'Backspace') command = pane.list.capabilities.removal === 'remove-ref' ? 'remove-from-collection' : 'delete'
    else if (cmd) return
    else if (event.key === 'F2') command = 'rename'
    else if (event.key === 'Delete' || (event.metaKey && event.key === 'Backspace')) command = pane.list.capabilities.removal === 'remove-ref' ? 'remove-from-collection' : 'delete'
    else if (event.key === 'Enter' && items.length === 1) command = 'open'
    if (command) { event.preventDefault(); runMenuAction(pane, ctx, { type: 'action', id: command, command, label: { key: '', fallback: command } }) }
  }

  /** Copy, cut and paste events of this window: whichever element the browser targets (the selection may sit in
   * another window), they belong to Files while its focus is here, outside fields, editors and dialogs (§3.3). */
  const clipboardBlocked = (event: ClipboardEvent) => {
    const root = rootRef.current
    const active = document.activeElement
    if (!root || event.defaultPrevented || !active || !root.contains(active) || isEditableTarget(active) || isEditableTarget(event.target)) return true
    if (namePrompt || deleteRequest || conflictRequest || transferChoice || permissionsRequest || defaultApp) return true
    // Text the user selected inside Files (the details panel) is copied as text.
    const selection = document.getSelection()
    return event.type !== 'paste' && !!selection && !selection.isCollapsed && !!selection.toString().trim() && root.contains(selection.anchorNode)
  }
  const onClipboardEvent = (event: ClipboardEvent) => {
    if (clipboardBlocked(event)) return
    const pane = focusedPane
    if (event.type === 'paste') {
      const reader = readTransferEvent(event.clipboardData)
      event.preventDefault()
      // Another operation running is told by the batch itself: a cut held by another window says it is being moved.
      const state = commandState('paste', { ...contextFor(pane, []), busy: false })
      if (state.state !== 'available') { showToast(reasonText(state.reason)); return }
      receive(pane, classifyIncoming(reader), { local: () => Promise.resolve(selectedUploads(reader.files)) })
      return
    }
    const purpose = event.type === 'cut' ? 'cut' : 'copy'
    const items = [...pane.selectedItemsMap.values()]
    if (!items.length) return
    const state = commandState(purpose, contextFor(pane, items))
    if (state.state !== 'available') { showToast(reasonText(state.reason)); return }
    const { offer, representations } = offerItems(pane, purpose, items)
    if (!writeTransferEvent(event.clipboardData, representations)) { desktopTransfers.withdraw(offer.offerId); void copyCommand(pane, purpose, items); return }
    event.preventDefault()
    copiedToast(purpose, items.length)
  }
  const clipboardListener = useRef(onClipboardEvent)
  useEffect(() => { clipboardListener.current = onClipboardEvent })
  useEffect(() => {
    const listener = (event: ClipboardEvent) => clipboardListener.current(event)
    for (const type of ['copy', 'cut', 'paste'] as const) document.addEventListener(type, listener)
    return () => { for (const type of ['copy', 'cut', 'paste'] as const) document.removeEventListener(type, listener) }
  }, [])

  // ─── The Shell's router (cross-window drag handle, "Paste desktop object…"): this window as a target (§5) ───

  const paneAt = (at: TransferPoint): { side: 'left' | 'right'; pane: BrowserPane } => {
    if (at === 'focus' || !splitActive) return { side: focusedIsRight ? 'right' : 'left', pane: focusedPane }
    const root = rootRef.current?.getBoundingClientRect()
    const rightPane = rootRef.current?.querySelector('[data-pane="right"]')?.getBoundingClientRect()
    return root && rightPane && root.left + at.x >= rightPane.left ? { side: 'right', pane: right } : { side: 'left', pane: left }
  }
  const placeName = (pane: BrowserPane) => placeTitles[pane.currentUrl] ?? pane.list.meta?.title ?? (displayPath(pane.currentUrl).split('/').filter(Boolean).pop() || displayPath(pane.currentUrl))
  /** What this window would do with the items at a point. Looks at kinds, formats and operations only. */
  const probeTransfer = (probe: TransferProbe): TransferProbeAnswer | null => {
    const { side, pane } = paneAt(probe.at)
    const { collection, folder } = destinationOf(pane)
    const accept: TransferProbeAnswer['accept'] = []
    for (const item of probe.items) {
      const zone = (item.kind === 'zone-file' || item.kind === 'zone-folder') && item.formats.includes(CONTENT_REFS_FORMAT)
      if (zone && collection && item.operations.includes('link')) accept.push({ itemKey: item.itemKey, format: CONTENT_REFS_FORMAT, operation: 'link' })
      else if (zone && folder && folderOps().supportsCopy && item.operations.includes('copy')) accept.push({ itemKey: item.itemKey, format: CONTENT_REFS_FORMAT, operation: 'copy' })
      else if (item.kind === 'local-file' && item.formats.includes('files') && folder && item.operations.includes('copy')) accept.push({ itemKey: item.itemKey, format: 'files', operation: 'copy' })
    }
    if (!accept.length) return null
    const name = placeName(pane)
    const action = accept.every((a) => a.operation === 'link') ? t('filebrowser.transfer.actionLink', 'Add references to {{name}}', { name })
      : accept.every((a) => a.format === 'files') ? t('filebrowser.transfer.actionUpload', 'Upload to {{name}}', { name })
        : t('filebrowser.transfer.actionCopy', 'Copy into {{name}}', { name })
    return { targetId: `${side}\n${pane.currentUrl}`, accept, action }
  }
  const deliveries = useRef(new Map<string, Promise<TransferDeliveryResult>>())
  /** Commits a delivery once per operation: copies or references for Zone items, uploads for local files. */
  const deliverTransfer = (delivery: TransferDelivery): Promise<TransferDeliveryResult> => {
    const known = deliveries.current.get(delivery.operationId)
    if (known) return known
    const failAll = (error: string): TransferDeliveryResult => ({ status: 'failed', items: delivery.items.map((item) => ({ itemKey: item.itemKey, status: 'failed', error })) })
    const run = async (): Promise<TransferDeliveryResult> => {
      const [side, url] = delivery.targetId.split('\n')
      const pane = side === 'right' ? right : left
      const { collection, folder } = destinationOf(pane)
      if (pane.currentUrl !== url || (delivery.operation === 'link' ? !collection : !folder)) return failAll(t('filebrowser.transfer.destinationChanged', 'The destination changed. Try again.'))
      const statuses = new Map<string, { status: TransferItemStatus; error?: string }>()
      const zone: { itemKey: string; item: ZoneContent }[] = []
      const files: { itemKey: string; file: File }[] = []
      for (const item of delivery.items) {
        const content = item.format === CONTENT_REFS_FORMAT ? readZoneContent(item.value) : undefined
        if (content) zone.push({ itemKey: item.itemKey, item: content })
        else if (item.format === 'files' && item.value instanceof File && folder) files.push({ itemKey: item.itemKey, file: item.value })
        else statuses.set(item.itemKey, { status: 'skipped' })
      }
      if (files.length) {
        void enqueueInputs(pane.currentUrl, selectedUploads(files.map((item) => item.file)))
        for (const item of files) statuses.set(item.itemKey, { status: 'accepted' })
      }
      if (zone.length) {
        const items = zone.map((item) => item.item)
        const results = delivery.operation === 'link' ? await linkReferences(pane, items) : await copyReferences(pane, items, folder!)
        const byPath = new Map(zone.map((item) => [dfsPathOfContent(item.item) ?? '', item.itemKey]))
        for (const result of results ?? []) {
          const itemKey = byPath.get(result.entry.path)
          if (itemKey === undefined || statuses.has(itemKey)) continue
          statuses.set(itemKey, { status: result.status === 'success' ? 'done' : result.status, ...(result.error ? { error: t(result.error.messageKey, result.error.fallback) } : {}) })
        }
      }
      const items = delivery.items.map((item) => ({ itemKey: item.itemKey, ...(statuses.get(item.itemKey) ?? { status: 'failed' as const }) }))
      const status = items.some((item) => item.status === 'accepted') ? 'accepted' : items.some((item) => item.status === 'done') ? 'done' : items.every((item) => item.status === 'cancelled') ? 'cancelled' : 'failed'
      return { status, items }
    }
    const result = run().catch((error: unknown) => failAll(String(error)))
    deliveries.current.set(delivery.operationId, result)
    return result
  }
  const transferTarget = useRef({ probe: probeTransfer, deliver: deliverTransfer })
  useEffect(() => { transferTarget.current = { probe: probeTransfer, deliver: deliverTransfer } })
  useEffect(() => {
    if (!windowId) return
    return registerWindowTransferTarget(windowId, {
      probe: (probe) => transferTarget.current.probe(probe),
      deliver: (delivery) => transferTarget.current.deliver(delivery),
    })
  }, [windowId])

  /** The drop effect a drag would get in a pane, from its types and this desktop's live Files drag (hover only). */
  const dropEffectFor = (pane: BrowserPane, event: React.DragEvent): DataTransfer['dropEffect'] => {
    const types = transferTypes(event.dataTransfer)
    const { collection, folder } = destinationOf(pane)
    const allowed = event.dataTransfer.effectAllowed
    const drag = types.includes(TRANSFER_HANDLE_FORMAT) ? desktopTransfers.dragOf('files') : undefined
    const payload = payloadOf(drag)
    if (drag && payload) {
      if (collection) return payload.realm === 'zone' ? allowedEffect('link', allowed) : 'none'
      if (!folder) return 'none'
      const operation = filesOperation(payload, drag, folder, { copy: event.ctrlKey || event.altKey, move: event.shiftKey })
      if (operation === 'move' && payload.entries.every((entry) => parentUrl(normalizeUrl(entry.path)) === normalizeUrl(folder))) return 'none'
      return allowedEffect(operation === 'move' ? 'move' : 'copy', allowed)
    }
    if (collection) return types.includes('files') && !mayCarryFiles(types.filter((type) => type !== 'files')) ? 'none' : allowedEffect('link', allowed)
    return folder ? allowedEffect('copy', allowed) : 'none'
  }
  const dropProps = (pane: BrowserPane) => ({
    onDragStart: (event: React.DragEvent) => {
      const key = (event.target as HTMLElement).closest<HTMLElement>('[data-item-key]')?.dataset.itemKey
      const item = key ? pane.list.loadedItemByKey(key) : undefined
      if (!item) return
      const items = pane.selectedItemsMap.has(item.key) ? [...pane.selectedItemsMap.values()] : [item]
      if (dragOffer.current) desktopTransfers.withdraw(dragOffer.current)
      // The same content as a copy: other Apps (an AIWS canvas, a chat draft) read the Zone references.
      const { offer, representations } = offerItems(pane, 'drag', items)
      writeTransferEvent(event.dataTransfer, representations)
      event.dataTransfer.effectAllowed = effectAllowedOf([...offer.operations])
      dragOffer.current = offer.offerId
    },
    onDragEnd: () => { if (dragOffer.current) desktopTransfers.withdraw(dragOffer.current); dragOffer.current = null; setDragTarget(null) },
    onDragOver: (event: React.DragEvent) => {
      // Hover looks at the types only: nothing is read, uploaded or copied before the drop.
      if (!mayCarryFiles(transferTypes(event.dataTransfer))) return
      event.preventDefault()
      const effect = dropEffectFor(pane, event)
      event.dataTransfer.dropEffect = effect
      setDragTarget(effect === 'none' ? null : pane.currentUrl)
    },
    onDragLeave: (event: React.DragEvent) => { if (!event.currentTarget.contains(event.relatedTarget as Node)) setDragTarget(null) },
    onDrop: (event: React.DragEvent) => {
      event.preventDefault()
      setDragTarget(null)
      const incoming = classifyIncoming(readTransferEvent(event.dataTransfer))
      // Folders dropped from this device are walked from the event's items, which only exist during the event.
      const local = incoming.kind === 'local' ? droppedUploads(event.dataTransfer) : null
      local?.catch(() => undefined)
      receive(pane, incoming, { intent: { copy: event.ctrlKey || event.altKey, move: event.shiftKey }, local: () => local ?? Promise.resolve([]) })
    },
  })
  const dialogs = <>
    {defaultApp && <div className="files-scrim absolute inset-0 z-50 flex items-center justify-center p-4"><section role="dialog" aria-modal="true" aria-label={t('filebrowser.menu.chooseDefault', 'Choose default app…')} className="flex w-[min(384px,100%)] flex-col gap-4 rounded-[12px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] p-5 shadow-[var(--cp-panel-shadow)]">
      <h2 className="text-[16px] font-medium">{t('filebrowser.menu.chooseDefault', 'Choose default app…')}</h2>
      <div className="flex flex-col gap-0.5">
        {defaultApp.plans.map(plan => <label key={plan.handlerKey} className="fb-row flex h-9 cursor-pointer items-center gap-3 rounded-[8px] px-2 text-[14px]"><input type="radio" name="content-handler" checked={defaultApp.key === plan.handlerKey} onChange={() => setDefaultApp({ ...defaultApp, key: plan.handlerKey })} /> {handlerLabel(plan)}</label>)}
      </div>
      <label className="flex items-start gap-3 px-2 text-[13px] text-[color:var(--cp-muted)]"><input type="checkbox" className="mt-0.5" checked={defaultApp.always} onChange={event => setDefaultApp({ ...defaultApp, always: event.target.checked })} /> {t('filebrowser.menu.alwaysUse', 'Always use this app for {{selector}}', { selector: defaultApp.selector })}</label>
      <div className="flex justify-end gap-2"><button type="button" className="files-btn" onClick={() => setDefaultApp(null)}>{t('common.cancel', 'Cancel')}</button><button type="button" className="files-btn files-btn-primary" onClick={() => { const choice = defaultApp; void (async () => { if (choice.always) await (await contentRegistry())?.setDefault('open', choice.selector, choice.key); setDefaultApp(null); await openContent(choice.request, { handlerKey: choice.key, name: choice.name, size: choice.size }) })().catch(error => showToast(String(error))) }}>{t('filebrowser.menu.open', 'Open')}</button></div>
    </section></div>}

    {transferChoice && <div className="files-scrim absolute inset-0 z-50 flex items-center justify-center p-4"><section role="dialog" aria-modal="true" aria-label={t('filebrowser.menu.paste', 'Paste')} data-testid="files-transfer-choice" className="flex w-[min(384px,100%)] flex-col gap-4 rounded-[12px] border border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] p-5 shadow-[var(--cp-panel-shadow)]" onKeyDown={(event) => { if (event.key === 'Escape') { event.stopPropagation(); closeTransferChoice() } }}>
      <p className="text-[14px]">{transferChoice.message}</p>
      <div className="flex justify-end gap-2"><button type="button" className="files-btn" onClick={closeTransferChoice}>{t('common.cancel', 'Cancel')}</button><button type="button" autoFocus className="files-btn files-btn-primary" onClick={() => { const choice = transferChoice; closeTransferChoice(); choice.onConfirm() }}>{transferChoice.confirmLabel}</button></div>
    </section></div>}
    <DeleteDialog request={deleteRequest} onClose={() => setDeleteRequest(null)} />
    <ConflictDialog request={conflictRequest?.ownerId === windowId ? conflictRequest : null} />
    {permissionsRequest && <PermissionsDialog key={permissionsRequest.path} request={permissionsRequest} onClose={() => setPermissionsRequest(null)} />}
    <BatchResults task={batchTask?.ownerId === windowId ? batchTask : null} onClose={() => setBatchTask(null)} />
    {dragTarget && <div className="pointer-events-none absolute inset-2 z-40 flex items-center justify-center rounded-[12px] border-2 border-dashed border-[color:var(--cp-accent)] bg-[color:color-mix(in_srgb,var(--cp-surface-opaque)_90%,transparent)] p-6 text-[14px] font-medium">{t('filebrowser.operation.dropDestination', 'Drop into {{path}}', { path: displayPath(dragTarget) })}</div>}
  </>

  const sidebarProps = (pane: BrowserPane) => ({
    dfs: dfsSource, host: hostSource, topics: topicsSource, collections, homeUrl, openedAt,
    hostUnlocked, hostRootUrl: hostRoot, loadChildren: dfsFolderChildren, loadHostChildren: hostChildren,
    onUnlockHost: () => void unlockComputer().then((ok) => { if (ok) { pane.navigate(hostRoot); setMobileSidebarOpen(false) } }),
    onCreateCollection: handleCreateCollection,
  })
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
    ...(archiveRootOf(pane.currentUrl) ? {
      onArchivePassword: () => {
        const url = archiveRootOf(pane.currentUrl)
        const outer = archiveSplit(dfsPathOf(pane.currentUrl) ?? '')?.outer ?? ''
        if (url) setArchivePassword({ pane, url, name: outer.split('/').pop() ?? '' })
      },
      readOnlyNote: t('archive.readOnly', 'Files in an archive are read-only'),
      emptyState: { title: archiveSplit(dfsPathOf(pane.currentUrl) ?? '')?.member ? t('archive.emptyFolder.title', 'This folder is empty') : t('archive.empty.title', 'This archive is empty') },
    } : {}),
    thumbnailSource,
    onUnlock: () => void unlockComputer().then((ok) => { if (ok) pane.list.reload() }),
    scroll: pane.scroll, onScroll: pane.setScroll, revealIndex: pane.revealIndex,
    cutIds: cutEntryIds(pendingCut),
    onSelectLoaded: pane.selectLoaded, onSelectAll: () => void pane.selectAll(),
    sortKey: pane.sortKey, sortDir: pane.sortDir,
    onSortChange: (key: SortKey, dir: SortDir) => { pane.setSortKey(key); pane.setSortDir(dir) },
  })
  const selectionProgress = (pane: BrowserPane) => <>
    {pane.enumerating && <div role="status" className="flex items-center gap-2 border-b border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-4 py-1 text-[12px] text-[color:var(--cp-muted)]">{t('filebrowser.operation.enumerating', 'Loading the full selection / locating file…')}<button type="button" className="files-btn files-btn-ghost !min-h-7 !px-2" onClick={pane.cancelEnumeration}>{t('common.cancel', 'Cancel')}</button></div>}
    {pane.selectionNotice && <p role="status" className="border-b border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-4 py-2 text-[12px] text-[color:var(--cp-muted)]">{t('filebrowser.operation.loadedRange', 'Range selection includes loaded rows only. Use Select all for the full folder.')}</p>}
    {pane.selectionError && <p role="alert" className="border-b border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-4 py-2 text-[12px] text-[color:var(--cp-danger)]">{pane.selectionError}</p>}
  </>

  // ─── Mobile layout ───
  if (isMobile) {
    const crumbs = crumbsForUrl(left.currentUrl, left.list.meta?.title, hostInfo?.label)

    return (
      <div ref={rootRef} data-testid="filebrowser" tabIndex={-1} onKeyDown={handleKeyDown} {...dropProps(left)} className="filebrowser files-ui fb-mobile relative flex h-full w-full flex-col overflow-hidden" style={{ background: 'var(--cp-surface-opaque)' }}>
        {/* Operations bar: drawer toggle + search + menu — or the selection bar */}
        {mobileSelectMode ? (
          <div className="flex items-center gap-1 px-2 pt-2 pb-1">
            <IconButton
              size="small"
              onClick={exitMobileSelectMode}
              aria-label={t('filebrowser.mobile.exitSelect', 'Exit selection')}
            >
              <X size={20} />
            </IconButton>
            <span className="min-w-0 flex-1 truncate text-[16px] font-medium tabular-nums text-[color:var(--cp-text)]">
              {t('filebrowser.mobile.selectedCount', '{{count}} selected', {
                count: left.selectedKeys.size,
              })}
            </span>
            <IconButton
              size="small"
              onClick={left.selectAll}
              aria-label={t('filebrowser.menu.selectAll', 'Select all')}
            >
              <ListChecks size={20} />
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
                <Trash2 size={20} />
              </IconButton>
            ) : null}
            <IconButton
              size="small"
              onClick={() => openMobileMenu([...left.selectedItemsMap.values()])}
              aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
            >
              <MoreVertical size={20} />
            </IconButton>
          </div>
        ) : (
          <div className="flex items-center gap-1 px-2 pt-2 pb-1">
            <IconButton
              size="small"
              onClick={() => setMobileSidebarOpen((v) => !v)}
              aria-label={t('filebrowser.mobile.places', 'Places')}
              className={clsx(mobileSidebarOpen && '!bg-[color:var(--files-pressed)]')}
            >
              <MenuIcon size={20} />
            </IconButton>
            <div className="relative flex h-10 min-w-0 flex-1 items-center gap-2 rounded-[10px] bg-[color:var(--cp-surface-2-opaque)] pl-3 pr-1">
              <Search size={18} className="shrink-0 text-[color:var(--cp-muted)]" />
              <input
                type="text"
                value={left.searchQuery}
                onChange={(event) => left.setSearchQuery(event.target.value)}
                placeholder={t('filebrowser.topbar.searchPlaceholder', 'Search files')}
                className="min-w-0 flex-1 bg-transparent text-[16px] text-[color:var(--cp-text)] outline-none placeholder:text-[color:var(--cp-muted)] focus-visible:!outline-none"
              />
              {left.searchQuery ? (
                <IconButton
                  size="small"
                  onClick={() => left.setSearchQuery('')}
                  aria-label={t('common.close', 'Close')}
                >
                  <X size={16} />
                </IconButton>
              ) : null}
            </div>
            <IconButton
              size="small"
              onClick={() => openMobileMenu([])}
              aria-label={t('filebrowser.mobile.moreActions', 'More actions')}
            >
              <MoreVertical size={20} />
            </IconButton>
          </div>
        )}

        {mobileSelectMode && <div className="flex gap-2 overflow-x-auto px-3 pb-2">{[
          { command: 'download', label: t('filebrowser.menu.download', 'Download') },
          { command: 'copy', label: t('filebrowser.menu.copyFiles', 'Copy files') },
          { command: 'paste', label: t('filebrowser.menu.paste', 'Paste') },
          { command: 'copy-to', label: t('filebrowser.actions.copyTo', 'Copy to') },
          { command: 'copy-other', label: t('filebrowser.menu.copyOther', 'Copy to other pane') },
          { command: 'move-to', label: t('filebrowser.actions.moveTo', 'Move to') },
        ].map(({ command, label }) => <button type="button" key={command} className="files-btn !rounded-[10px]" disabled={commandState(command, contextFor(left, leftSelectedItems)).state !== 'available'} onClick={() => runMenuAction(left, contextFor(left, leftSelectedItems), { type: 'action', id: command, command, label: { key: '', fallback: label } })}>{label}</button>)}</div>}
        {selectionProgress(left)}
        {/* Address bar: path crumbs + refresh on the right */}
        <div className="flex items-center border-b border-[color:var(--files-hairline)] px-2">
          <div className="flex min-w-0 flex-1 items-center overflow-x-auto text-[14px] text-[color:var(--cp-muted)]">
            {crumbs.map((crumb, idx) => (
              <div key={crumb.url} className="flex shrink-0 items-center">
                <button
                  type="button"
                  className={clsx(
                    '!min-w-0 truncate rounded-[8px] px-2',
                    idx === crumbs.length - 1 && 'font-medium text-[color:var(--cp-text)]',
                  )}
                  onClick={() => left.navigate(crumb.url)}
                >
                  {crumb.label}
                </button>
                {idx < crumbs.length - 1 ? (
                  <ChevronRight size={16} className="shrink-0 opacity-70" />
                ) : null}
              </div>
            ))}
          </div>
          <CopyTasks ownerId={windowId} reveal={(path) => livePanes.current[0].revealOriginal(path)} compact onShowTransfers={transfersPanel.show} />
          <button
            type="button"
            onClick={() => left.list.reload()}
            aria-label={t('filebrowser.topbar.refresh', 'Refresh')}
            className="flex shrink-0 items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:var(--files-pressed)]"
          >
            <RefreshCw size={20} />
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
            className="absolute bottom-5 right-4 z-20 flex h-14 w-14 items-center justify-center rounded-[16px] bg-[color:var(--files-primary)] text-[color:var(--files-on-primary)] shadow-[var(--files-float-shadow)] transition-transform active:scale-95"
          >
            <Plus size={24} />
          </button>
        ) : null}

        {/* Upload action sheet */}
        {mobileUploadOpen ? (
          <div className="absolute inset-0 z-40 flex items-end">
            <div
              className="files-scrim absolute inset-0"
              onClick={() => setMobileUploadOpen(false)}
            />
            <div className="files-sheet relative w-full pb-6 pt-2">
              <div className="files-sheet-grip mb-3" />
              <div className="px-5 pb-1 text-[16px] font-medium text-[color:var(--cp-text)]">
                {t('filebrowser.upload.title', 'Add to this folder')}
              </div>
              <div className="grid grid-cols-3 gap-2 px-3 py-3">
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
                    className="flex flex-col items-center gap-2 rounded-[12px] p-2 text-[13px] text-[color:var(--cp-text)] active:bg-[color:var(--files-pressed)]"
                  >
                    <div className="flex h-12 w-12 items-center justify-center rounded-[12px] bg-[color:var(--cp-surface-2-opaque)] text-[color:var(--cp-text)]">
                      {action.icon}
                    </div>
                    <span className="text-center leading-4">{action.label}</span>
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
              className="files-scrim absolute inset-0"
              onClick={() => setMobileSidebarOpen(false)}
            />
            <div className="relative flex h-full w-[min(320px,80%)] flex-col gap-3 border-r border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)] px-2 py-3">
              <Sidebar
                {...sidebarProps(left)}
                activeUrl={left.currentUrl}
                onNavigate={left.navigate}
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
              className="files-scrim absolute inset-0"
              onClick={() => setMobilePreviewOpen(false)}
            />
            <div className="files-sheet relative flex h-2/3 w-full flex-col pt-2">
              <div className="files-sheet-grip" />
              <div className="flex items-center justify-between pl-5 pr-2">
                <span className="text-[16px] font-medium text-[color:var(--cp-text)]">
                  {t('filebrowser.mobile.preview', 'Details')}
                </span>
                <button
                  type="button"
                  onClick={() => setMobilePreviewOpen(false)}
                  aria-label={t('common.close', 'Close')}
                  className="flex items-center justify-center rounded-full text-[color:var(--cp-muted)] active:bg-[color:var(--files-pressed)]"
                >
                  <X size={20} />
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
        <TransfersPanel onOpenTarget={(path) => left.revealOriginal(path)} panel={transfersPanel} />
        <NamePromptDialog request={namePrompt} onClose={() => setNamePrompt(null)} />

        {toast ? (
          <div className="files-toast pointer-events-none absolute bottom-24 left-1/2 max-w-[calc(100%-32px)] -translate-x-1/2">
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
      className={clsx('filebrowser files-ui relative flex h-full w-full overflow-hidden', splitActive && 'fb-split')}
      style={{ background: 'var(--cp-bg)' }}
    >
      {((!navCollapsed && (!splitActive || width >= 1300)) || navOverlayOpen) && <aside
        style={{ width: navWidth, resize: 'horizontal', minWidth: 180, maxWidth: 300, ...(navOverlayOpen && splitActive && width < 1300 ? { position: 'absolute' as const, top: 129, bottom: 0, left: 0, zIndex: 25, background: 'var(--cp-bg)', boxShadow: 'var(--files-float-shadow)' } : {}) }}
        onPointerUp={(event) => { const value = event.currentTarget.getBoundingClientRect().width; setNavWidth(value); localStorage.setItem('files.navWidth', String(value)) }}
        className="flex shrink-0 flex-col overflow-hidden border-r border-[color:var(--files-hairline)] px-2 pt-2"
      >
        <Sidebar
          {...sidebarProps(focusedPane)}
          activeUrl={focusedPane.currentUrl}
          onNavigate={(url) => { focusedPane.navigate(url); setNavOverlayOpen(false) }}
        />
      </aside>}

      <main
        data-pane="left" data-list-status={left.list.status} data-location={left.currentUrl}
        data-active={!focusedIsRight}
        {...dropProps(left)}
        className="fb-pane flex min-w-0 flex-1 flex-col bg-[color:var(--cp-surface-opaque)]"
        onMouseDownCapture={() => setFocusedSide('left')}
      >
        <TopBar
          tabs={titledTabs(left.tabs)}
          activeTabId={left.activeTabId}
          onSelectTab={left.setActiveTabId}
          onCloseTab={handleCloseLeftTab}
          onNewTab={handleNewTab}
          closedTabs={closedTabs}
          onRestoreClosedTab={handleRestoreClosedTab}
          currentPath={left.currentUrl}
          locationTitle={left.list.meta?.title}
          computerLabel={hostInfo?.label}
          onNavigate={(input) => { const target = toLocation(input); if (target) left.navigate(target) }}
          onBack={left.back}
          onForward={left.forward}
          onUp={left.goUp}
          canBack={left.activeHistory.back.length > 0}
          canForward={left.activeHistory.forward.length > 0}
          canUp={parentUrl(left.currentUrl) !== null}
          viewMode={left.viewMode}
          onViewModeChange={left.setViewMode}
          iconSize={iconSize}
          onIconSizeChange={setIconSize}
          searchQuery={left.searchQuery}
          onSearchChange={left.setSearchQuery}
          onCopyPath={() => copyText(copyPathText(left.currentUrl))}
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
          trailing={<CopyTasks ownerId={windowId} reveal={(path) => livePanes.current[0].revealOriginal(path)} onShowTransfers={transfersPanel.show} />}
        />
      </main>

      {splitActive ? (
        <section
          data-pane="right" data-list-status={right.list.status} data-location={right.currentUrl}
          data-active={focusedIsRight}
          {...dropProps(right)}
          className="fb-pane flex min-w-0 flex-1 flex-col border-l border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)]"
          onMouseDownCapture={() => setFocusedSide('right')}
        >
          <TopBar
            tabs={titledTabs(right.tabs)}
            activeTabId={right.activeTabId}
            onSelectTab={right.setActiveTabId}
            onCloseTab={handleCloseRightTab}
            showTabControls={false}
            allowCloseLast
            currentPath={right.currentUrl}
            locationTitle={right.list.meta?.title}
            computerLabel={hostInfo?.label}
            onNavigate={(input) => { const target = toLocation(input); if (target) right.navigate(target) }}
            onBack={right.back}
            onForward={right.forward}
            onUp={right.goUp}
            canBack={right.activeHistory.back.length > 0}
            canForward={right.activeHistory.forward.length > 0}
            canUp={parentUrl(right.currentUrl) !== null}
            viewMode={right.viewMode}
            onViewModeChange={right.setViewMode}
            iconSize={iconSize}
            onIconSizeChange={setIconSize}
            searchQuery={right.searchQuery}
            onSearchChange={right.setSearchQuery}
            onCopyPath={() => copyText(copyPathText(right.currentUrl))}
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
        <aside className={clsx("flex w-[320px] max-w-full shrink-0 flex-col overflow-hidden border-l border-[color:var(--files-hairline)] bg-[color:var(--cp-surface-opaque)]", (width < 1300 || splitActive) && "absolute bottom-0 top-[129px] right-0 z-20 shadow-[var(--files-float-shadow)]")}>
          <div className="flex h-10 shrink-0 items-center justify-between border-b border-[color:var(--files-hairline)] pl-4 pr-2">
            <span className="files-label">
              {t('filebrowser.preview.title', 'Details')}
            </span>
            <button
              type="button"
              className="files-icon-btn"
              onClick={() => setPreviewCollapsed(true)}
              aria-label={t('filebrowser.preview.collapse', 'Collapse sidebar')}
            >
              <PanelRightClose size={16} />
            </button>
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
      <TransfersPanel onOpenTarget={(path) => focusedPane.revealOriginal(path)} panel={transfersPanel} />
      <NamePromptDialog request={namePrompt} onClose={() => setNamePrompt(null)} />

      {archivePassword ? (
        <div className="fixed inset-0 z-[70]">
          <ArchivePasswordDialog
            name={archivePassword.name}
            url={archivePassword.url}
            onUnlocked={() => { archivePassword.pane.list.reload(); setArchivePassword(null) }}
            onCancel={() => setArchivePassword(null)}
          />
        </div>
      ) : null}
      {toast ? (
        <div className="files-toast pointer-events-none absolute bottom-12 left-1/2 max-w-[calc(100%-32px)] -translate-x-1/2">
          {toast}
        </div>
      ) : null}
    </div>
  )
}
