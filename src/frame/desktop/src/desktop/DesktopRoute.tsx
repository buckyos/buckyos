import {
  Alert,
  Button,
  Divider,
  Menu,
  MenuItem,
  useMediaQuery,
} from '@mui/material'
import { contentDescriptor, parseSystemOpen } from 'buckyos/content'
import { openContent } from '../app/content/open'
import { OpenWithHost, requestOpenWith } from '../app/content/OpenWithSheet'
import { openPreview } from '../app/preview/launch'
import { agentGuide } from '../app/agent-setup/guide'
import { agentSetupLaunchSchema, openAgentSetup } from '../app/agent-setup/launch'
import { agentBrowserLaunchFromParams, openAgentBrowser } from '../app/agent-browser/launch'
import { openSettings } from '../app/settings/launch'
import { isSettingsPage } from '../app/settings/components/layout/navigation'
import { AGENT_GUIDE_APP_ID } from '../app/agent-setup/model'
import clsx from 'clsx'
import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type MutableRefObject,
} from 'react'
import GridLayoutBase, { getCompactor } from 'react-grid-layout'
import { calcGridColWidth, type PositionParams } from 'react-grid-layout/core'
import { useNavigate, useSearchParams } from 'react-router-dom'
import type { Swiper as SwiperInstance } from 'swiper'
import { Pagination } from 'swiper/modules'
import { Swiper, SwiperSlide } from 'swiper/react'
import { globalSettingsStore } from '../app/settings/data/store'
import { findDesktopAppById } from '../app/registry'
import type { DesktopAppItem } from '../app/types'
import {
  AppIcon,
} from '../components/DesktopVisuals'
import { LoadingOrb } from '../components/LoadingOrb'
import {
  appIconSurfaceStyle,
} from '../components/DesktopVisualTokens'
import { useDesktopBackground } from './DesktopBackgroundProvider'
import {
  resetDesktopViewportProgress,
  setDesktopViewportProgress,
} from './viewportProgress'
import { DesktopTaskbar } from './DesktopTaskbar'
import { MobileTaskbar } from './MobileTaskbar'
import { StatusBar } from './StatusBar'
import { useCurrentUser } from './taskbarHooks'
import { touchSlop, useBackdropMenuGesture, useLauncherGesture, type LauncherCell } from './launcherGesture'
import { DesktopWidgetRenderer } from './widgets/WidgetRenderer'
import { isResizableWidget, widgetSizeLimits } from './widgets/catalog'
import { WidgetResizeFrame } from './WidgetResizeFrame'
import { AddToDesktopDialog } from './AddToDesktopDialog'
import { openDesktopFile, showFolderInFiles } from './desktopFolderActions'
import { watchDfsPath } from '../app/filebrowser/data/nfsp/invalidation'
import { classifyFileKind } from '../app/filebrowser/data/fileKinds'
import { fileIcon } from '../app/filebrowser/fileDisplay'
import { DesktopWindowLayer } from './windows/DesktopWindowLayer'
import {
  getDesktopWindowWorkspaceBounds,
} from './windows/geometry'
import { MobileNavProvider } from './windows/MobileNavContext'
import { MobileWindowSheet } from './windows/MobileWindowSheet'
import {
  desktopAvatarRowHeight,
  desktopWorkspaceInsets,
  mobilePagerRowHeight,
  mobileStatusBarHeight,
  mobileStatusBarMode,
  type ConnectionState,
} from './shell'
import { useTray } from './tray/trayModel'
import { useHasWallpaperPicture, useWallpaperSnapshot } from './wallpaper/store'
import { AccountAppearanceSync } from './appearance/AccountAppearanceSync'
import { useI18n } from '../i18n/provider'
import { loadingTipKeys } from '../i18n/loading-tips'
import { useAppText } from '../presentation/appText'
import type {
  AppDefinition,
  DesktopWallpaper,
  FileIconItem,
  FormFactor,
  LayoutItem,
  LayoutState,
  MockScenario,
  SupportedLocale,
  SystemPreferencesInput,
  WindowRecord,
} from '../models/ui'
import { supportedLocales } from '../models/ui'
import { signOutToLogin } from '../auth/signOut'
import { useThemeMode } from '../theme/provider'

// --- New unified store ---
import {
  useDesktopUIStore,
  useDesktopUISnapshot,
  type DesktopPlacement,
} from '../models/DesktopUIDataModel'
import {
  clamp,
  columnsForWidth,
  GRID_GAP,
  rowsForHeight,
  effectiveRowHeight,
  mapPageToGrid,
  normalizeViewportProgress,
  getPageIndex,
  type GridDensity,
  type GridRect,
} from '../models/layout'

// ---------------------------------------------------------------------------
// Hooks that remain in the view layer (DOM / browser APIs)
// ---------------------------------------------------------------------------

/**
 * Reads env(safe-area-inset-*) values for immersive fullscreen on mobile.
 */
function useSafeAreaInsets() {
  const [insets, setInsets] = useState({ top: 0, bottom: 0, left: 0, right: 0 })

  useEffect(() => {
    const probe = document.createElement('div')
    probe.style.cssText =
      'position:fixed;top:0;left:0;right:0;bottom:0;' +
      'padding-top:env(safe-area-inset-top,0px);' +
      'padding-bottom:env(safe-area-inset-bottom,0px);' +
      'padding-left:env(safe-area-inset-left,0px);' +
      'padding-right:env(safe-area-inset-right,0px);' +
      'pointer-events:none;visibility:hidden;z-index:-9999;'
    document.body.appendChild(probe)

    const update = () => {
      const cs = getComputedStyle(probe)
      const top = parseFloat(cs.paddingTop) || 0
      const bottom = parseFloat(cs.paddingBottom) || 0
      const left = parseFloat(cs.paddingLeft) || 0
      const right = parseFloat(cs.paddingRight) || 0
      setInsets((prev) => {
        if (prev.top === top && prev.bottom === bottom && prev.left === left && prev.right === right) {
          return prev
        }
        return { top, bottom, left, right }
      })
    }

    update()
    window.addEventListener('resize', update)
    window.addEventListener('orientationchange', update)

    return () => {
      window.removeEventListener('resize', update)
      window.removeEventListener('orientationchange', update)
      document.body.removeChild(probe)
    }
  }, [])

  return insets
}

/**
 * Grid geometry derived from the grid container's measured size.
 *
 * Takes the container *element* (from a callback ref) rather than a ref
 * object: the container is only mounted once the layout has loaded, so an
 * effect keyed on a ref object would run before the node exists and never
 * attach the ResizeObserver.
 */
function useGridSpec(
  container: HTMLElement | null,
  density: GridDensity,
  isMobile: boolean,
) {
  const [cols, setCols] = useState(() => (isMobile ? 4 : 10))
  const [containerHeight, setContainerHeight] = useState(720)

  useEffect(() => {
    const el = container
    if (!el) return

    const ro = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (!entry) return
      const w = entry.contentRect.width
      // The mobile slide's own padding is not grid space (desktop reserves room for the dots inside the
      // slide, so its rows are computed from the whole height as before).
      const h = entry.contentRect.height - (isMobile ? mobileSlidePaddingY : 0)
      const nextCols = isMobile ? 4 : columnsForWidth(w)
      setCols(nextCols)
      setContainerHeight(h)
      el.style.setProperty('--grid-columns', String(nextCols))
    })

    ro.observe(el)
    return () => ro.disconnect()
  }, [container, isMobile])

  const rows = rowsForHeight(containerHeight, density, isMobile)
  const rowHeight = effectiveRowHeight(containerHeight, rows, density, isMobile)

  return { cols, rows, rowHeight }
}

function useConnectionState(runtimeContainer: string): ConnectionState {
  const [isNavigatorOnline, setIsNavigatorOnline] = useState(() => navigator.onLine)

  useEffect(() => {
    const handleOnline = () => setIsNavigatorOnline(true)
    const handleOffline = () => setIsNavigatorOnline(false)
    window.addEventListener('online', handleOnline)
    window.addEventListener('offline', handleOffline)
    return () => {
      window.removeEventListener('online', handleOnline)
      window.removeEventListener('offline', handleOffline)
    }
  }, [])

  if (!isNavigatorOnline) return 'offline'
  return runtimeContainer === 'browser' ? 'degraded' : 'online'
}

function nextSupportedLocale(locale: SupportedLocale) {
  const currentIndex = supportedLocales.indexOf(locale)
  return supportedLocales[(currentIndex + 1) % supportedLocales.length]
}

// Dead-zone insets only apply on mobile; desktop windows and grid use the full viewport.
const zeroDeadZone = { top: 0, bottom: 0, left: 0, right: 0 }

// Launcher offset below the mobile home status bar.
const mobileLauncherTopInset = 42
// Breathing room of a mobile launcher page: 8px a side (vertical total 16), see the slide below. The side
// padding is tight on purpose: four columns on a 390px phone leave 89px per cell, which is what an 11px
// label like "HomeStation" needs on one line.
const mobileSlidePaddingX = 8
const mobileSlidePaddingY = 16
// Selector of the pager row below the launcher; Swiper draws the mobile page dots into it.
const mobilePagerSelector = '.desktop-mobile-pager'

// react-grid-layout only places the tiles: no compaction or collision pass of its own (that one shoves items
// downward, even past maxRows). Dragging is launcherGesture.ts, and the store resolves displacement on drop via
// applyDragCollision.
const launcherCompactor = getCompactor(null, true)
const launcherDragConfig = { enabled: false }

type WindowGeometryPatch = Partial<Pick<WindowRecord, 'x' | 'y' | 'width' | 'height'>>

/** What the desktop asks of the launcher pages: the page shown, the cell under a point on it, and turning pages. */
interface DesktopPagesController {
  pageIndex: () => number
  /** The cell of the shown page under a screen point (the nearest one when it is beside the grid). */
  cellAt: (point: { x: number; y: number }) => { x: number; y: number } | undefined
  slideTo: (pageIndex: number) => void
}

/** The menu of the desktop's empty space, and the page and cell it was opened on. */
interface DesktopMenuState extends DesktopPlacement {
  x: number
  y: number
  pageCount: number
}

// ---------------------------------------------------------------------------
// DesktopRoute — now a thin view that delegates to the unified store
// ---------------------------------------------------------------------------

export function DesktopRoute() {
  const store = useDesktopUIStore()
  const snap = useDesktopUISnapshot()
  const { locale, setLocale, t } = useI18n()
  const appText = useAppText()
  const { themeMode, setThemeMode } = useThemeMode()
  const isMobile = useMediaQuery('(max-width:768px)')
  const navigate = useNavigate()
  const formFactor: FormFactor = isMobile ? 'mobile' : 'desktop'
  const [searchParams, setSearchParams] = useSearchParams()
  const initialScenario =
    (searchParams.get('scenario') as MockScenario | null) ?? 'normal'
  const [scenario] = useState<MockScenario>(initialScenario)
  const [isLoggingOut, setIsLoggingOut] = useState(false)
  const currentUser = useCurrentUser()
  const launchHandled = useRef(false)
  useEffect(() => {
    const open = parseSystemOpen(searchParams)
    if (snap.status !== 'success' || !open || launchHandled.current) return
    launchHandled.current = true
    const { request: { source, session }, hints } = open
    if (open.target === 'preview') openPreview({ source, session })
    else if (open.target === 'chooser') requestOpenWith({ source, session, name: hints.name ?? contentDescriptor(source).name ?? '', mediaType: hints.mime, size: hints.size, exclude: open.from })
    else void openContent({ source, session }, { name: hints.name, mime: hints.mime, size: hints.size })
  }, [snap.status, searchParams])

  // The Agent setup wizard comes back here after a sign-in (`?agent_setup=<source>[&agent_id=]`).
  const agentSetupHandled = useRef(false)
  useEffect(() => {
    if (snap.status !== 'success' || agentSetupHandled.current || !searchParams.has('agent_setup')) return
    agentSetupHandled.current = true
    const launch = agentSetupLaunchSchema.safeParse({
      source: searchParams.get('agent_setup'),
      ...(searchParams.get('agent_id') ? { agent_id: searchParams.get('agent_id') } : {}),
    })
    const params = new URLSearchParams(searchParams)
    params.delete('agent_setup')
    params.delete('agent_id')
    setSearchParams(params, { replace: true })
    if (launch.success) openAgentSetup(launch.data)
  }, [snap.status, searchParams, setSearchParams])

  // A link from another app's page (`?settings=<page>`: the Agent home's "Add runtime") opens Settings on that page.
  const settingsLinkHandled = useRef(false)
  useEffect(() => {
    if (snap.status !== 'success' || settingsLinkHandled.current || !searchParams.has('settings')) return
    settingsLinkHandled.current = true
    const page = searchParams.get('settings')
    const params = new URLSearchParams(searchParams)
    params.delete('settings')
    setSearchParams(params, { replace: true })
    if (isSettingsPage(page)) openSettings(page)
  }, [snap.status, searchParams, setSearchParams])

  // A link to an Agent's browser (`?agent_browser=<agent>[&mode=<mode>]`: the Agent home, a conversation's Agent panel; the `group` of an older link is dropped).
  const agentBrowserLinkHandled = useRef(false)
  useEffect(() => {
    const launch = agentBrowserLaunchFromParams(searchParams)
    if (snap.status !== 'success' || agentBrowserLinkHandled.current || !launch) return
    agentBrowserLinkHandled.current = true
    const params = new URLSearchParams(searchParams)
    for (const name of ['agent_browser', 'group', 'mode']) params.delete(name)
    setSearchParams(params, { replace: true })
    openAgentBrowser(launch)
  }, [snap.status, searchParams, setSearchParams])

  // The Jarvis guide follows the signed-in user's own Agent created from it.
  useEffect(() => {
    if (snap.status === 'success') agentGuide.start()
  }, [snap.status])


  const workspaceRef = useRef<HTMLDivElement | null>(null)
  const [gridContainer, setGridContainer] = useState<HTMLDivElement | null>(null)
  const pagesRef = useRef<DesktopPagesController | null>(null)
  // Kept while the menu fades out, so its items do not change on the way.
  const [desktopMenu, setDesktopMenu] = useState<(DesktopMenuState & { open: boolean }) | null>(null)
  /** Where "Add…" puts what is chosen; the dialog is open while it is set. */
  const [addPlacement, setAddPlacement] = useState<DesktopPlacement | null>(null)

  const [viewportSize, setViewportSize] = useState(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }))
  const [workspaceSize, setWorkspaceSize] = useState({ width: 960, height: 720 })

  const settingsSnap = useSyncExternalStore(
    globalSettingsStore.subscribe,
    globalSettingsStore.getSnapshot,
  )
  const density = settingsSnap.desktopIconSize
  const gridSpec = useGridSpec(gridContainer, density, isMobile)

  // Sync grid spec into store
  useEffect(() => {
    store.setGridSpec(gridSpec.cols, gridSpec.rows, gridSpec.rowHeight)
  }, [store, gridSpec.cols, gridSpec.rows, gridSpec.rowHeight])

  // Init store on mount / formFactor change
  useEffect(() => {
    void store.init(formFactor, scenario)
  }, [store, formFactor, scenario])

  // The Desktop folder can change elsewhere (Files, another device): look again when the page comes back
  // into view, now and then while it is shown, and right after Files changes it in this page.
  const storeStatus = snap.status
  useEffect(() => {
    if (storeStatus !== 'success') return
    const refresh = () => {
      if (document.visibilityState === 'visible') void store.refreshDesktopFolder()
    }
    const folderPath = store.getDesktopFolderPath()
    const stopWatching = folderPath
      ? watchDfsPath(folderPath, () => void store.refreshDesktopFolder({ force: true }))
      : undefined
    const timer = window.setInterval(refresh, 30_000)
    window.addEventListener('focus', refresh)
    document.addEventListener('visibilitychange', refresh)
    return () => {
      stopWatching?.()
      window.clearInterval(timer)
      window.removeEventListener('focus', refresh)
      document.removeEventListener('visibilitychange', refresh)
    }
  }, [store, storeStatus])

  // Sync scenario to URL
  useEffect(() => {
    const current = searchParams.get('scenario') ?? 'normal'
    if (current === scenario) return
    const params = new URLSearchParams(searchParams)
    params.set('scenario', scenario)
    setSearchParams(params, { replace: true })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scenario])

  // Observe workspace size
  useEffect(() => {
    if (!workspaceRef.current) return
    const resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (!entry) return
      setWorkspaceSize({
        width: entry.contentRect.width,
        height: entry.contentRect.height,
      })
    })
    resizeObserver.observe(workspaceRef.current)
    return () => resizeObserver.disconnect()
  }, [])

  // Viewport resize → normalise window positions
  useEffect(() => {
    const updateViewportSize = () => {
      const nextViewportSize = {
        width: window.innerWidth,
        height: window.innerHeight,
      }
      setViewportSize(nextViewportSize)
      if (formFactor === 'desktop') {
        store.normalizeOpenWindowsForViewport(nextViewportSize)
      }
    }
    window.addEventListener('resize', updateViewportSize)
    window.addEventListener('orientationchange', updateViewportSize)
    return () => {
      window.removeEventListener('resize', updateViewportSize)
      window.removeEventListener('orientationchange', updateViewportSize)
    }
  }, [formFactor, store])

  // Read from snapshot — grouped by data tier
  const { status, apps, resolvedLayout } = snap
  const { appearance } = snap.syncData
  const { runtimeContainer, windowAppearance } = appearance
  const {
    activityLog,
    snackbar,
    isSystemSidebarOpen,
    isTaskbarPinned,
    selectedItemId,
    contextMenu,
    resizingItemId,
  } = snap.runtime

  const isLoading = status === 'loading'
  const hasError = status === 'error'
  const connectionState = useConnectionState(runtimeContainer)

  const resolvedDeadZone = isMobile ? store.getResolvedDeadZone() : zeroDeadZone
  const safeArea = useSafeAreaInsets()
  // Desktop: windows get the whole viewport unless the taskbar is pinned.
  const desktopInsets = desktopWorkspaceInsets(!isMobile && isTaskbarPinned)
  const desktopWorkspaceTopInset = safeArea.top + desktopInsets.top
  const desktopWorkspaceLeftInset = desktopInsets.left
  const desktopViewportBounds = useMemo(
    () =>
      getDesktopWindowWorkspaceBounds({
        safeArea,
        topInset: desktopWorkspaceTopInset,
        leftInset: desktopWorkspaceLeftInset,
        viewportSize,
      }),
    [safeArea, desktopWorkspaceTopInset, desktopWorkspaceLeftInset, viewportSize],
  )
  const workspaceInnerWidth = Math.max(
    workspaceSize.width -
      resolvedDeadZone.left -
      resolvedDeadZone.right -
      safeArea.left -
      safeArea.right -
      desktopWorkspaceLeftInset,
    320,
  )

  // ---------------------------------------------------------------------------
  // Action handlers (thin wrappers around store)
  //
  // All handlers are referentially stable so that the memoised subtrees
  // (window slots, app panels, the launcher grid) do not re-render on every
  // store update — e.g. on each pointermove while a window is dragged.
  // They read the *current* snapshot from the store instead of closing over
  // `windows`/`apps` from this render.
  // ---------------------------------------------------------------------------

  const logActivity = useCallback(
    (message: string) => store.logActivity(message, locale),
    [store, locale],
  )

  const handleOpenApp = useCallback(
    (appId: string) => {
      const app = findDesktopAppById(store.getSnapshot().apps, appId)
      if (!app) return

      if (appId === AGENT_GUIDE_APP_ID) {
        void agentGuide.open()
        logActivity(t('activity.opened', 'Opened {{name}}', { name: appText(app).title }))
        return
      }

      if (isMobile && app.manifest.mobileRedirectPath) {
        navigate(app.manifest.mobileRedirectPath)
        return
      }

      if (app.manifest.placement === 'new-container' || app.tier === 'external') {
        logActivity(
          t('activity.external', 'Requested new-container launch for {{name}}', {
            name: appText(app).title,
          }),
        )
        store.setSnackbar(t('external.body'))
        return
      }

      // The store's own `logActivity` hook is intentionally not passed here:
      // the translated "Opened …" entry below is the single activity entry.
      store.openApp(appId, {
        isMobile,
        navigate,
        viewportBounds: desktopViewportBounds,
      })
      logActivity(
        t('activity.opened', 'Opened {{name}}', { name: appText(app).title }),
      )
    },
    [store, isMobile, navigate, logActivity, t, appText, desktopViewportBounds],
  )

  const describeWindow = useCallback(
    (windowId: string) => {
      const current = store.getSnapshot()
      const target = current.runtime.windows.find((w) => w.id === windowId)
      if (!target) return null
      const app = findDesktopAppById(current.apps, target.appId)
      return app ? appText(app).title : t(target.titleKey)
    },
    [store, t, appText],
  )

  const handleCloseWindow = useCallback(
    (windowId: string) => {
      const name = describeWindow(windowId)
      if (name !== null) {
        logActivity(t('activity.closed', 'Closed {{name}}', { name }))
      }
      store.closeWindow(windowId)
    },
    [store, describeWindow, logActivity, t],
  )

  const minimizeWindow = useCallback(
    (windowId: string) => {
      const name = describeWindow(windowId)
      if (name !== null) {
        logActivity(t('activity.minimized', 'Minimized {{name}}', { name }))
      }
      store.minimizeWindow(windowId)
    },
    [store, describeWindow, logActivity, t],
  )

  const toggleMaximizeWindow = useCallback(
    (windowId: string) => {
      const name = describeWindow(windowId)
      if (name !== null) {
        logActivity(
          t('activity.maximized', 'Toggled maximize for {{name}}', { name }),
        )
      }
      store.toggleMaximizeWindow(windowId)
    },
    [store, describeWindow, logActivity, t],
  )

  const focusWindow = useCallback(
    (windowId: string) => store.focusWindow(windowId),
    [store],
  )
  const updateWindowGeometry = useCallback(
    (windowId: string, geometry: WindowGeometryPatch) =>
      store.updateWindowGeometry(windowId, geometry),
    [store],
  )

  const applySettings = useCallback(
    (values: SystemPreferencesInput) => {
      store.applySettings(values, { setLocale, setThemeMode, viewportSize })
      logActivity(t('activity.saved'))
    },
    [store, setLocale, setThemeMode, viewportSize, logActivity, t],
  )

  const restoreDefaults = useCallback(() => store.restoreDefaults(), [store])

  const toggleSidebar = useCallback(() => store.toggleSystemSidebar(), [store])
  const closeSidebar = useCallback(() => store.closeSystemSidebar(), [store])
  const toggleTaskbarPinned = useCallback(
    () => store.setTaskbarPinned(!store.getSnapshot().runtime.isTaskbarPinned, viewportSize),
    [store, viewportSize],
  )
  const handleSelectSidebarApp = useCallback(
    (appId: string) => {
      handleOpenApp(appId)
      closeSidebar()
    },
    [handleOpenApp, closeSidebar],
  )
  const handleActivateTaskbarApp = useCallback(
    (appId: string) => {
      if (!store.activateTaskbarApp(appId)) handleOpenApp(appId)
      closeSidebar()
    },
    [store, handleOpenApp, closeSidebar],
  )
  const handleCloseAppWindows = useCallback(
    (appId: string) => {
      for (const w of store.getSnapshot().runtime.windows) {
        if (w.appId === appId) handleCloseWindow(w.id)
      }
    },
    [store, handleCloseWindow],
  )
  const handleSetTaskbarAppPinned = useCallback(
    (appId: string, pinned: boolean) => store.setTaskbarAppPinned(appId, pinned),
    [store],
  )
  const handleMoveTaskbarApp = useCallback(
    (appId: string, index: number) => store.moveTaskbarApp(appId, index),
    [store],
  )
  const handleShowDesktop = useCallback(() => {
    store.toggleShowDesktop()
    closeSidebar()
  }, [store, closeSidebar])
  const handleOpenDiagnostics = useCallback(
    () => handleSelectSidebarApp('diagnostics'),
    [handleSelectSidebarApp],
  )
  const handleOpenSettings = useCallback(
    () => handleSelectSidebarApp('settings'),
    [handleSelectSidebarApp],
  )
  const handleLogout = useCallback(async () => {
    if (isLoggingOut) return

    setIsLoggingOut(true)
    try {
      if (!await store.prepareLogout()) { setIsLoggingOut(false); return }
      await signOutToLogin()
    } catch (error) {
      console.error('[logout] failed:', error)
      store.setSnackbar(t('shell.logoutFailed', 'Log out failed. Please try again.'))
      setIsLoggingOut(false)
    }
  }, [isLoggingOut, store, t])
  // apps ask for the Desktop's own sign-out (the AI Workspace identity menu)
  useEffect(() => {
    const onRequest = () => { void handleLogout() }
    window.addEventListener('buckyos:request-logout', onRequest)
    return () => window.removeEventListener('buckyos:request-logout', onRequest)
  }, [handleLogout])
  const handleCycleLocale = useCallback(
    () => setLocale(nextSupportedLocale(locale)),
    [setLocale, locale],
  )
  const handleToggleTheme = useCallback(
    () => setThemeMode(themeMode === 'light' ? 'dark' : 'light'),
    [setThemeMode, themeMode],
  )

  // Tap, drag and hold on a tile are told apart in launcherGesture.ts; these only act on the outcome.
  const handleMoveItem = useCallback(
    (pageId: string, from: LauncherCell, to: LauncherCell): LauncherCell | null => {
      store.handleGridDragStop(pageId, from, to)
      const placed = store
        .getSnapshot()
        .resolvedLayout?.pages.find((page) => page.id === pageId)
        ?.items.find((entry) => entry.id === from.i)
      return placed?.x !== undefined && placed.y !== undefined ? { ...from, x: placed.x, y: placed.y } : null
    },
    [store],
  )

  const handleOpenItem = useCallback(
    (item: LayoutItem) => {
      if (item.type === 'app') handleOpenApp(item.appId)
      else if (item.type === 'file') openDesktopFile(item)
    },
    [handleOpenApp],
  )

  const handleOpenItemContextMenu = useCallback(
    (item: LayoutItem, point: { x: number; y: number }) => {
      store.setSelectedItemId(item.id)
      store.setContextMenu({
        itemId: item.id,
        mouseX: point.x + 2,
        mouseY: point.y - 6,
      })
    },
    [store],
  )

  const handleUpdateWidgetConfig = useCallback(
    (itemId: string, patch: Record<string, unknown>) => store.updateWidgetConfig(itemId, patch),
    [store],
  )

  const handleViewportProgress = useCallback(
    (progress: number, pageCount: number) => {
      store.setViewportProgress(normalizeViewportProgress(progress, pageCount))
      // Turning the page takes the focus off a widget being resized.
      store.setResizingItemId(null)
    },
    [store],
  )

  const handleOpenDesktopMenu = useCallback(
    (point: { x: number; y: number }) => {
      const pages = pagesRef.current
      store.setContextMenu(null)
      setDesktopMenu({
        open: true,
        x: point.x,
        y: point.y,
        pageIndex: pages?.pageIndex() ?? 0,
        pageCount: store.getSnapshot().resolvedLayout?.pages.length ?? 1,
        cell: pages?.cellAt(point),
      })
    },
    [store],
  )
  const backdropGesture = useBackdropMenuGesture(handleOpenDesktopMenu)
  const closeDesktopMenu = useCallback(() => setDesktopMenu((menu) => (menu ? { ...menu, open: false } : menu)), [])

  /** Shows an item just added: selected, on its page (it may have gone to a later one when this page was full). */
  const revealItem = useCallback(
    (itemId: string) => {
      store.setSelectedItemId(itemId)
      const pageIndex = store.getSnapshot().resolvedLayout?.pages.findIndex((page) => page.items.some((item) => item.id === itemId)) ?? -1
      // After the launcher has drawn the page the item is on.
      if (pageIndex >= 0) requestAnimationFrame(() => pagesRef.current?.slideTo(pageIndex))
    },
    [store],
  )
  const handleAddWidget = useCallback(
    (widgetType: string) => {
      const itemId = store.addWidgetToDesktop(widgetType, addPlacement ?? undefined)
      setAddPlacement(null)
      if (itemId) revealItem(itemId)
    },
    [store, addPlacement, revealItem],
  )
  const handleAddApp = useCallback(
    (appId: string, label: string) => {
      const itemId = store.addAppToDesktop(appId, label, addPlacement ?? undefined)
      setAddPlacement(null)
      if (itemId) revealItem(itemId)
    },
    [store, addPlacement, revealItem],
  )
  const closeAddDialog = useCallback(() => setAddPlacement(null), [])

  const handleResizeItem = useCallback(
    (itemId: string, next: GridRect) => store.resizeWidget(itemId, next),
    [store],
  )

  // A widget's resize handles go away once the focus leaves it: a press anywhere else, Escape, or another window.
  useEffect(() => {
    if (!resizingItemId) return
    const stop = () => store.setResizingItemId(null)
    const onPointerDown = (event: PointerEvent) => {
      const owner = event.target instanceof Element ? event.target.closest('[data-item-id]') : null
      if (owner?.getAttribute('data-item-id') !== resizingItemId) stop()
    }
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape') stop()
    }
    document.addEventListener('pointerdown', onPointerDown, true)
    document.addEventListener('keydown', onKeyDown)
    window.addEventListener('blur', stop)
    return () => {
      document.removeEventListener('pointerdown', onPointerDown, true)
      document.removeEventListener('keydown', onKeyDown)
      window.removeEventListener('blur', stop)
    }
  }, [store, resizingItemId])

  // ---------------------------------------------------------------------------
  // Derived data
  // ---------------------------------------------------------------------------

  // Memoised inside the store on (apps, windows) identity.
  const windowLayerModel = store.getWindowLayerModel()
  const topMobileWindow = windowLayerModel.topWindow
  const activeMobileApp =
    formFactor === 'mobile' && topMobileWindow ? topMobileWindow.app : undefined
  const mobileStatusBarRowHeight = mobileStatusBarHeight(activeMobileApp)
  const mobileSheetTopInset =
    activeMobileApp && mobileStatusBarMode(activeMobileApp) === 'standard'
      ? safeArea.top + resolvedDeadZone.top + mobileStatusBarRowHeight
      : 0
  // On desktop the launcher grid starts below the floating avatar.
  const workspaceTopPadding =
    formFactor === 'mobile'
      ? safeArea.top + (topMobileWindow ? resolvedDeadZone.top : mobileLauncherTopInset)
      : safeArea.top + desktopAvatarRowHeight
  // On mobile the launcher stops above the pager row (page dots) and the bottom safe area. The row is
  // reserved even while an app's sheet covers it, so opening an app never re-pages the launcher.
  const mobilePagerInset = resolvedDeadZone.bottom + safeArea.bottom
  const workspaceBottomPadding =
    formFactor === 'mobile' ? mobilePagerInset + mobilePagerRowHeight : resolvedDeadZone.bottom + safeArea.bottom
  const workspaceInnerHeight = Math.max(
    workspaceSize.height - workspaceTopPadding - workspaceBottomPadding,
    360,
  )
  const shouldLockDesktopViewport = formFactor === 'desktop'
  const topMobileWindowId = topMobileWindow?.id
  const handleMinimizeTopMobileWindow = useMemo(
    () =>
      isMobile && topMobileWindowId
        ? () => minimizeWindow(topMobileWindowId)
        : undefined,
    [isMobile, topMobileWindowId, minimizeWindow],
  )
  const taskbarApps = store.getTaskbarApps()
  const contextMenuItem = contextMenu
    ? resolvedLayout?.pages.flatMap((page) => page.items).find((entry) => entry.id === contextMenu.itemId)
    : undefined
  const desktopFolderPath = status === 'success' ? store.getDesktopFolderPath() : ''
  const canEditDesktop = status === 'success' && store.canEditDesktop()

  // The status bar's message and notification trays read MessageHub,
  // msg-center and TaskMgr only while the mobile shell is up.
  const tray = useTray(isMobile && !isLoading && !hasError)

  // Background (drawn by DesktopBackgroundSync below)
  const hasWallpaperPicture = useHasWallpaperPicture()
  const backgroundPageCount = resolvedLayout?.pages.length ?? 1

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <main
      // With a picture the desktop is see-through down to DesktopBackground; labels and page dots
      // switch to light-on-dark (index.css) so they read on any image.
      className={clsx('relative isolate', hasWallpaperPicture ? null : 'bg-[color:var(--cp-bg)]')}
      data-wallpaper={hasWallpaperPicture ? 'image' : undefined}
      style={
        shouldLockDesktopViewport
          ? { height: '100dvh', overflow: 'hidden' }
          : { minHeight: '100dvh' }
      }
    >
      <DesktopBackgroundSync fallback={appearance.wallpaper} pageCount={backgroundPageCount} />
      {currentUser ? <AccountAppearanceSync key={currentUser.userId} userId={currentUser.userId} /> : null}
      <section
        className="relative z-10"
        style={
          shouldLockDesktopViewport
            ? { height: '100%', overflow: 'hidden' }
            : { minHeight: '100dvh' }
        }
      >
        <div ref={workspaceRef} className="relative h-dvh min-h-dvh">
          {!isLoading && !hasError && resolvedLayout ? (
            <MobileNavProvider>
              {isMobile ? (
                <>
                  <MobileTaskbar
                    apps={apps}
                    avatarRowHeight={mobileStatusBarRowHeight}
                    connectionState={connectionState}
                    deadZone={resolvedDeadZone}
                    isLoggingOut={isLoggingOut}
                    onClose={closeSidebar}
                    onCloseAppWindows={handleCloseAppWindows}
                    onLogout={handleLogout}
                    onOpenApp={handleSelectSidebarApp}
                    onSetAppPinned={handleSetTaskbarAppPinned}
                    onShowDesktop={handleShowDesktop}
                    open={isSystemSidebarOpen}
                    runtimeContainer={runtimeContainer}
                    safeArea={safeArea}
                    taskbarApps={taskbarApps}
                    user={currentUser}
                  />
                  <StatusBar
                    activeApp={activeMobileApp}
                    connectionState={connectionState}
                    deadZone={resolvedDeadZone}
                    safeAreaTop={safeArea.top}
                    onCycleLocale={handleCycleLocale}
                    onMinimizeWindow={handleMinimizeTopMobileWindow}
                    onNavigate={navigate}
                    onOpenDiagnostics={handleOpenDiagnostics}
                    onOpenSettings={handleOpenSettings}
                    onOpenSidebar={toggleSidebar}
                    onToggleTheme={handleToggleTheme}
                    themeMode={themeMode}
                    tray={tray}
                    user={currentUser}
                  />
                </>
              ) : (
                <DesktopTaskbar
                  apps={apps}
                  connectionState={connectionState}
                  isLoggingOut={isLoggingOut}
                  onActivateApp={handleActivateTaskbarApp}
                  onClose={closeSidebar}
                  onCloseAppWindows={handleCloseAppWindows}
                  onLogout={handleLogout}
                  onMoveApp={handleMoveTaskbarApp}
                  onOpenApp={handleSelectSidebarApp}
                  onSetAppPinned={handleSetTaskbarAppPinned}
                  onShowDesktop={handleShowDesktop}
                  onToggleOpen={toggleSidebar}
                  onTogglePinned={toggleTaskbarPinned}
                  open={isSystemSidebarOpen}
                  pinned={isTaskbarPinned}
                  runtimeContainer={runtimeContainer}
                  safeArea={safeArea}
                  taskbarApps={taskbarApps}
                  user={currentUser}
                />
              )}
              <div
                ref={setGridContainer}
                // Its empty space has the desktop's own menu (right click, or a touch held still).
                className="relative overflow-hidden select-none [-webkit-touch-callout:none]"
                data-testid="desktop-workspace"
                data-density={density}
                data-form-factor={formFactor}
                {...backdropGesture}
                style={{
                  paddingTop: workspaceTopPadding,
                  paddingBottom: workspaceBottomPadding,
                  paddingLeft: resolvedDeadZone.left + safeArea.left + desktopWorkspaceLeftInset,
                  paddingRight: resolvedDeadZone.right + safeArea.right,
                }}
              >
                <DesktopPages
                  apps={apps}
                  controllerRef={pagesRef}
                  gridCols={gridSpec.cols}
                  gridRowHeight={gridSpec.rowHeight}
                  gridRows={gridSpec.rows}
                  height={workspaceInnerHeight}
                  isMobile={isMobile}
                  layout={resolvedLayout}
                  onMoveItem={handleMoveItem}
                  onOpenItem={handleOpenItem}
                  onOpenItemContextMenu={handleOpenItemContextMenu}
                  onResizeItem={handleResizeItem}
                  onRestore={restoreDefaults}
                  onUpdateConfig={handleUpdateWidgetConfig}
                  onViewportProgress={handleViewportProgress}
                  resizingItemId={resizingItemId}
                  selectedItemId={selectedItemId}
                  width={workspaceInnerWidth - (isMobile ? mobileSlidePaddingX * 2 : 48)}
                />
              </div>

              {isMobile && (
                <div
                  data-testid="mobile-pager"
                  className="pointer-events-none absolute inset-x-0 z-30"
                  style={{ bottom: mobilePagerInset, height: mobilePagerRowHeight }}
                >
                  <div className="swiper-pagination desktop-mobile-pager pointer-events-auto" style={{ height: mobilePagerRowHeight }} />
                </div>
              )}

              {!isMobile && (
                <DesktopWindowLayer
                  activityLog={activityLog}
                  layoutState={resolvedLayout}
                  locale={locale}
                  onClose={handleCloseWindow}
                  onGeometryChange={updateWindowGeometry}
                  onFocus={focusWindow}
                  onMaximize={toggleMaximizeWindow}
                  onMinimize={minimizeWindow}
                  onSaveSettings={applySettings}
                  runtimeContainer={runtimeContainer}
                  leftInset={desktopWorkspaceLeftInset}
                  safeArea={safeArea}
                  themeMode={themeMode}
                  topInset={desktopWorkspaceTopInset}
                  uiModel={windowLayerModel}
                  windowAppearance={windowAppearance}
                  workspaceSize={viewportSize}
                />
              )}

              {isMobile && topMobileWindow && (
                <MobileWindowSheet
                  activityLog={activityLog}
                  app={topMobileWindow.app}
                  windowId={topMobileWindow.id}
                  launch={topMobileWindow.launch}
                  deadZone={resolvedDeadZone}
                  safeAreaBottom={safeArea.bottom}
                  layoutState={resolvedLayout}
                  locale={locale}
                  onSaveSettings={applySettings}
                  runtimeContainer={runtimeContainer}
                  themeMode={themeMode}
                  topInset={mobileSheetTopInset}
                  windowAppearance={windowAppearance}
                />
              )}
            </MobileNavProvider>
          ) : null}

          {isLoading ? <LoadingState /> : null}
          {hasError ? (
            <ErrorState
              onRetry={() => void store.init(formFactor, scenario)}
            />
          ) : null}
        </div>
      </section>

      <Menu
        open={Boolean(contextMenu)}
        onClose={() => store.setContextMenu(null)}
        anchorReference="anchorPosition"
        anchorPosition={
          contextMenu
            ? { top: contextMenu.mouseY, left: contextMenu.mouseX }
            : undefined
        }
      >
        <MenuItem
          disabled={contextMenuItem?.type === 'widget'}
          onClick={() => {
            if (contextMenuItem?.type === 'app') handleOpenApp(contextMenuItem.appId)
            if (contextMenuItem?.type === 'file') openDesktopFile(contextMenuItem)
            store.setContextMenu(null)
          }}
        >
          {t('common.open')}
        </MenuItem>
        {contextMenuItem?.type === 'widget' && isResizableWidget(contextMenuItem.widgetType) ? (
          <MenuItem
            data-testid="desktop-menu-resize"
            onClick={() => {
              store.setContextMenu(null)
              store.setResizingItemId(contextMenuItem.id)
            }}
          >
            {t('desktop.menu.resize', 'Resize')}
          </MenuItem>
        ) : null}
        <MenuItem
          disabled={!desktopFolderPath}
          data-testid="desktop-menu-show-in-files"
          onClick={() => {
            store.setContextMenu(null)
            showFolderInFiles(desktopFolderPath)
          }}
        >
          {t('desktop.menu.showInFiles', 'Show in Files')}
        </MenuItem>
        <MenuItem
          disabled={!contextMenuItem || contextMenuItem.type === 'file' || !desktopFolderPath}
          data-testid="desktop-menu-remove"
          onClick={() => {
            if (contextMenuItem) void store.removeDesktopItem(contextMenuItem.id)
          }}
        >
          {t('shell.removeFromDesktop', 'Remove from desktop')}
        </MenuItem>
        <Divider />
        <MenuItem
          disabled={
            !contextMenu ||
            !resolvedLayout ||
            getPageIndex(resolvedLayout, contextMenu.itemId) <= 0
          }
          onClick={() =>
            contextMenu &&
            store.moveItemBetweenPages(contextMenu.itemId, -1)
          }
        >
          {t('common.movePrev')}
        </MenuItem>
        <MenuItem
          disabled={!contextMenu || !resolvedLayout}
          onClick={() =>
            contextMenu &&
            store.moveItemBetweenPages(contextMenu.itemId, 1)
          }
        >
          {t('common.moveNext')}
        </MenuItem>
      </Menu>

      <Menu
        open={Boolean(desktopMenu?.open)}
        onClose={closeDesktopMenu}
        anchorReference="anchorPosition"
        anchorPosition={desktopMenu ? { top: desktopMenu.y, left: desktopMenu.x } : undefined}
        // A right click beside the open menu closes it instead of showing the browser's.
        onContextMenu={(event) => {
          event.preventDefault()
          closeDesktopMenu()
        }}
      >
        <MenuItem
          data-testid="desktop-menu-appearance"
          onClick={() => {
            closeDesktopMenu()
            openSettings('appearance')
          }}
        >
          {t('desktop.menu.appearance', 'Appearance settings')}
        </MenuItem>
        <Divider />
        <MenuItem
          data-testid="desktop-menu-next-page"
          disabled={!desktopMenu || desktopMenu.pageIndex >= desktopMenu.pageCount - 1}
          onClick={() => {
            closeDesktopMenu()
            if (desktopMenu) pagesRef.current?.slideTo(desktopMenu.pageIndex + 1)
          }}
        >
          {t('desktop.menu.nextPage', 'Go to next screen')}
        </MenuItem>
        <MenuItem
          data-testid="desktop-menu-previous-page"
          disabled={!desktopMenu || desktopMenu.pageIndex <= 0}
          onClick={() => {
            closeDesktopMenu()
            if (desktopMenu) pagesRef.current?.slideTo(desktopMenu.pageIndex - 1)
          }}
        >
          {t('desktop.menu.previousPage', 'Go to previous screen')}
        </MenuItem>
        <Divider />
        <MenuItem
          data-testid="desktop-menu-add"
          disabled={!canEditDesktop}
          onClick={() => {
            closeDesktopMenu()
            if (desktopMenu) setAddPlacement({ pageIndex: desktopMenu.pageIndex, cell: desktopMenu.cell })
          }}
        >
          {t('desktop.menu.add', 'Add…')}
        </MenuItem>
      </Menu>

      <AddToDesktopDialog
        apps={apps}
        isMobile={isMobile}
        layout={resolvedLayout}
        onAddApp={handleAddApp}
        onAddWidget={handleAddWidget}
        onClose={closeAddDialog}
        open={addPlacement !== null}
      />

      <OpenWithHost />
      <Alert
        severity="success"
        onClose={() => store.setSnackbar(null)}
        sx={{
          display: snackbar ? 'flex' : 'none',
          position: 'fixed',
          bottom: 20,
          right: 20,
          zIndex: 2000,
          bgcolor: 'var(--cp-surface)',
          color: 'var(--cp-text)',
        }}
      >
        {snackbar}
      </Alert>
    </main>
  )
}

// ---------------------------------------------------------------------------
// Launcher pages (Swiper + react-grid-layout)
// ---------------------------------------------------------------------------

interface DesktopPagesProps {
  apps: DesktopAppItem[]
  controllerRef: MutableRefObject<DesktopPagesController | null>
  gridCols: number
  gridRowHeight: number
  gridRows: number
  height: number
  isMobile: boolean
  layout: LayoutState
  onMoveItem: (pageId: string, from: LauncherCell, to: LauncherCell) => LauncherCell | null
  onOpenItem: (item: LayoutItem) => void
  onOpenItemContextMenu: (item: LayoutItem, point: { x: number; y: number }) => void
  onResizeItem: (itemId: string, next: GridRect) => GridRect | null
  onRestore: () => void
  onUpdateConfig: (itemId: string, patch: Record<string, unknown>) => void
  onViewportProgress: (progress: number, pageCount: number) => void
  resizingItemId: string | null
  selectedItemId: string | null
  width: number
}

const noopLayoutChange = () => {}

/**
 * Memoised: the launcher grid only depends on the resolved layout, the grid
 * spec and the selection. Window drag/resize, status-bar ticks and sidebar
 * toggles update the store without touching any of these, so this subtree
 * (and react-grid-layout's per-render layout synchronisation) is skipped.
 */
const DesktopPages = memo(function DesktopPages({
  apps,
  controllerRef,
  gridCols,
  gridRowHeight,
  gridRows,
  height,
  isMobile,
  layout,
  onMoveItem,
  onOpenItem,
  onOpenItemContextMenu,
  onResizeItem,
  onRestore,
  onUpdateConfig,
  onViewportProgress,
  resizingItemId,
  selectedItemId,
  width,
}: DesktopPagesProps) {
  const pageCount = layout.pages.length
  const gridLayouts = useMemo(
    () => layout.pages.map((page) => mapPageToGrid(page)),
    [layout],
  )
  const gridConfig = useMemo(
    () => ({
      cols: gridCols,
      rowHeight: gridRowHeight,
      margin: [GRID_GAP, GRID_GAP] as [number, number],
      containerPadding: [0, 0] as [number, number],
      maxRows: gridRows,
    }),
    [gridCols, gridRowHeight, gridRows],
  )
  const tileGrid = useMemo<PositionParams>(
    () => ({ ...gridConfig, containerWidth: width }),
    [gridConfig, width],
  )
  const swiperRef = useRef<SwiperInstance | null>(null)
  const lockPaging = useCallback((locked: boolean) => {
    if (swiperRef.current) swiperRef.current.allowTouchMove = !locked
  }, [])
  // Per-frame progress (while swiping) only touches a CSS variable; the store
  // learns about page changes once, when the slide settles.
  const handleProgress = useCallback(
    (swiper: { progress: number }) => setDesktopViewportProgress(swiper.progress, pageCount),
    [pageCount],
  )
  const handleSlideSettled = useCallback(
    (swiper: { progress: number }) => {
      setDesktopViewportProgress(swiper.progress, pageCount)
      onViewportProgress(swiper.progress, pageCount)
    },
    [onViewportProgress, pageCount],
  )
  const handleSwiper = useCallback(
    (swiper: SwiperInstance) => {
      swiperRef.current = swiper
      handleSlideSettled(swiper)
    },
    [handleSlideSettled],
  )
  useEffect(() => resetDesktopViewportProgress, [])

  useLayoutEffect(() => {
    const shown = () => {
      const swiper = swiperRef.current
      return swiper && !swiper.destroyed ? swiper : null
    }
    controllerRef.current = {
      pageIndex: () => shown()?.activeIndex ?? 0,
      cellAt: (point) => {
        const swiper = shown()
        const grid = swiper?.slides[swiper.activeIndex]?.querySelector('.react-grid-layout')
        if (!grid) return undefined
        const rect = grid.getBoundingClientRect()
        const colStep = calcGridColWidth(tileGrid) + tileGrid.margin[0]
        const rowStep = tileGrid.rowHeight + tileGrid.margin[1]
        return {
          x: clamp(Math.floor((point.x - rect.left) / colStep), 0, tileGrid.cols - 1),
          y: clamp(Math.floor((point.y - rect.top) / rowStep), 0, tileGrid.maxRows - 1),
        }
      },
      slideTo: (pageIndex) => shown()?.slideTo(pageIndex),
    }
    return () => {
      controllerRef.current = null
    }
  }, [controllerRef, tileGrid])

  if (pageCount === 0 || layout.pages.every((page) => page.items.length === 0)) {
    // As tall as the pages: the grid's row count is measured from this container.
    return <EmptyState height={height} onRestore={onRestore} />
  }

  return (
    <Swiper
      modules={[Pagination]}
      // A touch swipes pages on any screen (an iPad shows the desktop layout); the mouse drags tiles and uses the dots.
      simulateTouch={false}
      // As far as a tile tap may drift (launcherGesture.ts): a tap does not nudge the page or lose its click.
      threshold={touchSlop}
      // Mobile dots go into the dock strip (DesktopRoute); desktop keeps them inside the pages.
      pagination={isMobile ? { clickable: true, el: mobilePagerSelector } : { clickable: true }}
      className="h-full"
      style={{ height }}
      onSwiper={handleSwiper}
      onProgress={handleProgress}
      onSlideChange={handleSlideSettled}
    >
      {layout.pages.map((page, pageIndex) => (
        <SwiperSlide key={page.id} className="h-full">
          <div className={isMobile ? 'h-full px-2 py-2' : 'h-full px-6 pb-16 pt-6'}>
            <GridLayoutBase
              className="layout h-full"
              gridConfig={gridConfig}
              layout={gridLayouts[pageIndex]}
              width={width}
              resizeConfig={{ enabled: false }}
              dragConfig={launcherDragConfig}
              compactor={launcherCompactor}
              onLayoutChange={noopLayoutChange}
            >
              {page.items.map((item) => (
                // Raised above its neighbours while its resize handles are shown.
                <div key={item.id} data-resize-mode={resizingItemId === item.id ? '' : undefined}>
                  <DesktopTile
                    app={
                      item.type === 'app'
                        ? findDesktopAppById(apps, item.appId)
                        : undefined
                    }
                    grid={tileGrid}
                    isDesktop={!isMobile}
                    item={item}
                    isResizing={resizingItemId === item.id}
                    isSelected={selectedItemId === item.id}
                    lockPaging={lockPaging}
                    onMove={(from, to) => onMoveItem(page.id, from, to)}
                    onOpen={onOpenItem}
                    onOpenContextMenu={onOpenItemContextMenu}
                    onResize={onResizeItem}
                    onUpdateConfig={onUpdateConfig}
                  />
                </div>
              ))}
            </GridLayoutBase>
          </div>
        </SwiperSlide>
      ))}
    </Swiper>
  )
})

// ---------------------------------------------------------------------------
// Sub-components (unchanged)
// ---------------------------------------------------------------------------

/**
 * Hands the wallpaper to the app-wide DesktopBackground: the picture chosen in Settings, else the payload's
 * built-in one. Kept apart so a preview drag in Settings re-renders this, not the whole desktop.
 */
function DesktopBackgroundSync({ fallback, pageCount }: { fallback: DesktopWallpaper | undefined; pageCount: number }) {
  const { resetBackground, setBackground } = useDesktopBackground()
  const picture = useWallpaperSnapshot().wallpaper
  const wallpaper = useMemo(() => picture ?? fallback ?? { mode: 'infinite' as const }, [picture, fallback])

  useEffect(() => {
    setBackground({ wallpaper, pageCount })
  }, [pageCount, wallpaper, setBackground])

  useEffect(() => resetBackground, [resetBackground])

  return null
}

function LoadingState() {
  const { t } = useI18n()
  // One tip per load, kept across re-renders and language switches.
  const [tipKey] = useState(() => loadingTipKeys[Math.floor(Math.random() * loadingTipKeys.length)])
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-[color:var(--cp-bg)] px-4">
      <div className="flex max-w-md flex-col items-center text-center">
        <LoadingOrb size={48} label={t('states.loading')} />
        <p className="mt-4 text-base font-medium text-[color:var(--cp-text)]">{t('states.loading')}</p>
        <p className="mt-2 text-sm leading-6 text-[color:var(--cp-muted)]" data-testid="desktop-loading-tip">
          <span className="font-medium text-[color:var(--cp-text)]">{t('states.tipLabel')}</span>
          {' · '}
          {t(tipKey)}
        </p>
      </div>
    </div>
  )
}

function ErrorState({ onRetry }: { onRetry: () => void }) {
  const { t } = useI18n()
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-[color:var(--cp-bg)]">
      <div className="shell-panel max-w-lg p-8 text-center">
        <p className="text-xl font-medium">
          {t('states.errorTitle')}
        </p>
        <p className="mx-auto mt-2 max-w-sm text-sm leading-6 text-[color:var(--cp-muted)]">
          {t('states.errorBody')}
        </p>
        <Button onClick={onRetry} className="!mt-6">
          {t('common.retry')}
        </Button>
      </div>
    </div>
  )
}

function EmptyState({ height, onRestore }: { height: number; onRestore: () => void }) {
  const { t } = useI18n()
  return (
    <div className="flex items-center justify-center px-4" style={{ height }}>
      <div className="shell-panel max-w-lg border-dashed p-8 text-center">
        <p className="text-xl font-medium">
          {t('states.emptyTitle')}
        </p>
        <p className="mx-auto mt-2 max-w-sm text-sm leading-6 text-[color:var(--cp-muted)]">
          {t('states.emptyBody')}
        </p>
        <Button onClick={onRestore} className="!mt-6" variant="outlined">
          {t('shell.restore')}
        </Button>
      </div>
    </div>
  )
}

function DesktopTile({
  app,
  grid,
  isDesktop,
  isResizing,
  item,
  isSelected,
  lockPaging,
  onMove,
  onOpen,
  onOpenContextMenu,
  onResize,
  onUpdateConfig,
}: {
  app?: AppDefinition
  grid: PositionParams
  isDesktop: boolean
  isResizing: boolean
  item: LayoutItem
  isSelected: boolean
  lockPaging: (locked: boolean) => void
  onMove: (from: LauncherCell, to: LauncherCell) => LauncherCell | null
  onOpen: (item: LayoutItem) => void
  onOpenContextMenu: (item: LayoutItem, point: { x: number; y: number }) => void
  onResize: (itemId: string, next: GridRect) => GridRect | null
  onUpdateConfig: (itemId: string, patch: Record<string, unknown>) => void
}) {
  const appText = useAppText()
  const opens = item.type === 'app' || item.type === 'file'
  const gesture = useLauncherGesture({
    cell:
      item.x !== undefined && item.y !== undefined
        ? { i: item.id, x: item.x, y: item.y, w: item.w, h: item.h }
        : null,
    grid,
    // A widget's own controls take a touch tap as a click.
    onTap: opens ? () => onOpen(item) : undefined,
    onMenu: (point) => onOpenContextMenu(item, point),
    onMove,
    lockPaging,
  })

  const tile = item.type === 'app' && app
    ? {
      title: appText(app).title,
      testId: `desktop-app-${app.id}`,
      icon: (
        <span
          className="relative flex shrink-0 items-center justify-center overflow-hidden"
          style={{
            width: 'var(--icon-size)',
            height: 'var(--icon-size)',
            borderRadius: 'calc(var(--icon-size) * 0.28)',
            ...appIconSurfaceStyle(app.accent, app.iconKey),
          }}
        >
          <AppIcon iconKey={app.iconKey} iconUrl={app.iconUrl} fill className="text-white" />
        </span>
      ),
    }
    : item.type === 'file'
      ? { title: item.id, testId: `desktop-file-${item.id}`, icon: <DesktopFileIcon item={item} /> }
      : null

  const resizeCell = isResizing && item.type === 'widget' && item.x !== undefined && item.y !== undefined
    ? { x: item.x, y: item.y, w: item.w, h: item.h }
    : null

  return (
    <>
      <div
        data-testid={`desktop-item-${item.id}`}
        data-item-id={item.id}
        data-item-type={item.type}
        data-widget-type={item.type === 'widget' ? item.widgetType : undefined}
        className={clsx(
          'desktop-tile-shell group relative h-full border border-transparent transition-[border-color,background-color] duration-150 ease-[var(--cp-ease-smooth)]',
          // The gesture owns touches on a tile: no browser pan or zoom, text selection or iOS callout.
          'touch-none select-none [-webkit-touch-callout:none]',
          'rounded-[16px]',
          item.type === 'widget' ? 'overflow-hidden' : 'overflow-visible',
          isDesktop ? 'cursor-grab active:cursor-grabbing' : '',
          isSelected
            ? 'border-[color:color-mix(in_srgb,var(--cp-accent)_50%,transparent)] bg-[color:color-mix(in_srgb,var(--cp-accent)_8%,transparent)]'
            : item.type === 'widget'
              ? 'border-[color:var(--cp-border)]'
              : '',
        )}
        {...gesture}
      >
        {tile ? (
          <button
            type="button"
            onClick={() => onOpen(item)}
            data-testid={tile.testId}
            title={tile.title}
            className={clsx(
              'flex h-full w-full flex-col items-center rounded-[16px] bg-transparent text-center transition-colors duration-150 focus-visible:bg-[color:color-mix(in_srgb,var(--cp-text)_6%,transparent)]',
              isDesktop ? 'cursor-grab active:cursor-grabbing' : '',
            )}
            // Size-query container for the label: desktop cells are capped at
            // ~110px wide and the fixed row height only leaves room for one
            // label line, so the label scales with the cell instead of the
            // viewport (see the label's fontSize below).
            style={{ paddingTop: 'var(--icon-padding-top)', containerType: 'inline-size' }}
          >
            {tile.icon}
            {item.type === 'app' && app?.iconBadge ? (
              <span
                aria-hidden="true"
                data-testid={`desktop-app-badge-${app.id}`}
                className="pointer-events-none absolute z-20 size-3 rounded-full border-2 border-[color:var(--cp-surface)]"
                style={{
                  top: 'calc(var(--icon-padding-top) - 3px)',
                  left: 'calc(50% + var(--icon-size) / 2 - 9px)',
                  background: app.iconBadge === 'error' ? 'var(--cp-danger)' : 'var(--cp-warning)',
                }}
              />
            ) : null}
            <span
              className="desktop-icon-label max-w-full break-words overflow-hidden px-0.5 font-medium text-[color:var(--cp-text)]"
              style={{
                paddingTop: 'var(--label-padding-top)',
                fontSize: isDesktop
                  ? 'clamp(11px, 12cqw, var(--font-size-label))'
                  : 'var(--font-size-label)',
                lineHeight: 'var(--line-height-label)',
                display: '-webkit-box',
                // Desktop rows are sized for a single label line; a second
                // line would be clipped, so truncate with an ellipsis instead.
                WebkitLineClamp: isDesktop ? 1 : 2,
                WebkitBoxOrient: 'vertical',
              }}
            >
              {tile.title}
            </span>
          </button>
        ) : null}

        {item.type === 'widget' ? (
          <DesktopWidgetRenderer item={item} onUpdateConfig={onUpdateConfig} />
        ) : null}
      </div>
      {resizeCell && item.type === 'widget' ? (
        <WidgetResizeFrame
          cell={resizeCell}
          grid={grid}
          itemId={item.id}
          limits={widgetSizeLimits(item.widgetType, { cols: grid.cols, rows: grid.maxRows })}
          lockPaging={lockPaging}
          onResize={(next) => onResize(item.id, next)}
        />
      ) : null}
    </>
  )
}

/** A document or folder on the desktop: its kind's icon on a plain tile. */
function DesktopFileIcon({ item }: { item: FileIconItem }) {
  const kind = item.fileKind === 'folder' ? 'folder' : classifyFileKind(item.id)
  return (
    <span
      className="relative flex shrink-0 items-center justify-center overflow-hidden border border-[color:var(--cp-border)] bg-[color:var(--cp-surface)]"
      style={{ width: 'var(--icon-size)', height: 'var(--icon-size)', borderRadius: 'calc(var(--icon-size) * 0.28)' }}
    >
      {fileIcon({ kind, name: item.id }, 26)}
    </span>
  )
}
