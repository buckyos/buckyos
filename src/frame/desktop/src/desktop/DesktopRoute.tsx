import {
  Alert,
  Button,
  Menu,
  MenuItem,
  useMediaQuery,
} from '@mui/material'
import clsx from 'clsx'
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
} from 'react'
import GridLayoutBase, {
  type Layout,
  type LayoutItem as GridLayoutItem,
  noCompactor,
} from 'react-grid-layout'
import { useNavigate, useSearchParams } from 'react-router-dom'
import { Pagination } from 'swiper/modules'
import { Swiper, SwiperSlide } from 'swiper/react'
import { globalSettingsStore } from '../app/settings/mock/store'
import { findDesktopAppById } from '../app/registry'
import {
  AppIcon,
} from '../components/DesktopVisuals'
import {
  appIconSurfaceStyle,
} from '../components/DesktopVisualTokens'
import { useDesktopBackground } from './DesktopBackgroundProvider'
import { StatusBar } from './StatusBar'
import { SystemSidebar } from './SystemSidebar'
import { DesktopWidgetRenderer } from './widgets/WidgetRenderer'
import { DesktopWindowLayer } from './windows/DesktopWindowLayer'
import {
  getDesktopWindowWorkspaceBounds,
} from './windows/geometry'
import { MobileNavProvider } from './windows/MobileNavContext'
import { MobileWindowSheet } from './windows/MobileWindowSheet'
import {
  mobileStatusBarMode,
  shellStatusBarHeight,
  type ConnectionState,
} from './shell'
import { useI18n } from '../i18n/provider'
import type {
  AppDefinition,
  FormFactor,
  LayoutItem,
  MockScenario,
  SupportedLocale,
  SystemPreferencesInput,
} from '../models/ui'
import { supportedLocales } from '../models/ui'
import { useThemeMode } from '../theme/provider'

// --- New unified store ---
import {
  useDesktopUIStore,
  useDesktopUISnapshot,
} from '../models/DesktopUIDataModel'
import {
  columnsForWidth,
  GRID_GAP,
  rowsForHeight,
  stretchedRowHeight,
  densityRowHeight,
  desktopMinCanvasSize,
  mapPageToGrid,
  normalizeViewportProgress,
  getPageIndex,
  type GridDensity,
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

function useGridSpec(
  containerRef: { current: HTMLElement | null },
  density: GridDensity,
  isMobile: boolean,
) {
  const [cols, setCols] = useState(() => (isMobile ? 4 : 10))
  const [containerHeight, setContainerHeight] = useState(720)

  useEffect(() => {
    const el = containerRef.current
    if (!el) return

    const ro = new ResizeObserver((entries) => {
      const entry = entries[0]
      if (!entry) return
      const w = entry.contentRect.width
      const h = entry.contentRect.height
      const nextCols = isMobile ? 4 : columnsForWidth(w)
      setCols(nextCols)
      setContainerHeight(h)
      el.style.setProperty('--grid-columns', String(nextCols))
    })

    ro.observe(el)
    return () => ro.disconnect()
  }, [containerRef, isMobile])

  const rows = rowsForHeight(containerHeight, density, isMobile)
  const rowHeight = isMobile
    ? densityRowHeight[density]
    : stretchedRowHeight(containerHeight, rows)

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

// ---------------------------------------------------------------------------
// DesktopRoute — now a thin view that delegates to the unified store
// ---------------------------------------------------------------------------

export function DesktopRoute() {
  const store = useDesktopUIStore()
  const snap = useDesktopUISnapshot()
  const { resetBackground, setBackground } = useDesktopBackground()
  const { locale, setLocale, t } = useI18n()
  const { themeMode, setThemeMode } = useThemeMode()
  const isMobile = useMediaQuery('(max-width:768px)')
  const navigate = useNavigate()
  const formFactor: FormFactor = isMobile ? 'mobile' : 'desktop'
  const [searchParams, setSearchParams] = useSearchParams()
  const initialScenario =
    (searchParams.get('scenario') as MockScenario | null) ?? 'normal'
  const [scenario] = useState<MockScenario>(initialScenario)

  // Refs for drag suppression (view-only concern)
  const suppressOpenItemId = useRef<string | null>(null)
  const draggedOpenBlockItemId = useRef<string | null>(null)
  const draggedOpenBlockTimeoutId = useRef<number | null>(null)
  const workspaceRef = useRef<HTMLDivElement | null>(null)
  const gridContainerRef = useRef<HTMLDivElement | null>(null)

  const [viewportSize, setViewportSize] = useState(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }))
  const [workspaceSize, setWorkspaceSize] = useState({ width: 960, height: 720 })

  const settingsSnap = useSyncExternalStore(
    globalSettingsStore.subscribe,
    globalSettingsStore.getSnapshot,
  )
  const density = (settingsSnap.session.appearance.fontSize ?? 'medium') as GridDensity
  const gridSpec = useGridSpec(gridContainerRef, density, isMobile)

  // Sync grid spec into store
  useEffect(() => {
    store.setGridSpec(gridSpec.cols, gridSpec.rows, gridSpec.rowHeight)
  }, [store, gridSpec.cols, gridSpec.rows, gridSpec.rowHeight])

  // Init store on mount / formFactor change
  useEffect(() => {
    void store.init(formFactor, scenario)
  }, [store, formFactor, scenario])

  // Sync scenario to URL
  useEffect(() => {
    const current = searchParams.get('scenario') ?? 'normal'
    if (current === scenario) return
    const params = new URLSearchParams(searchParams)
    params.set('scenario', scenario)
    setSearchParams(params, { replace: true })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scenario])

  // Cleanup drag timeout
  useEffect(() => {
    return () => {
      if (draggedOpenBlockTimeoutId.current) {
        window.clearTimeout(draggedOpenBlockTimeoutId.current)
      }
    }
  }, [])

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
  const normalizeOpenWindowsForViewport = useCallback(
    (nextViewportSize: { width: number; height: number }) => {
      store.normalizeOpenWindowsForViewport(nextViewportSize)
    },
    [store],
  )

  useEffect(() => {
    const updateViewportSize = () => {
      const nextViewportSize = {
        width: window.innerWidth,
        height: window.innerHeight,
      }
      setViewportSize(nextViewportSize)
      if (formFactor === 'desktop') {
        normalizeOpenWindowsForViewport(nextViewportSize)
      }
    }
    window.addEventListener('resize', updateViewportSize)
    window.addEventListener('orientationchange', updateViewportSize)
    return () => {
      window.removeEventListener('resize', updateViewportSize)
      window.removeEventListener('orientationchange', updateViewportSize)
    }
  }, [formFactor, normalizeOpenWindowsForViewport])

  // Read from snapshot — grouped by data tier
  const { status, error: loadError, apps, resolvedLayout } = snap
  const { appearance } = snap.syncData
  const { runtimeContainer, windowAppearance } = appearance
  const {
    windows,
    activityLog,
    snackbar,
    isSystemSidebarOpen,
    selectedItemId,
    viewportProgress,
    contextMenu,
  } = snap.runtime

  const isLoading = status === 'loading'
  const hasError = status === 'error'
  const connectionState = useConnectionState(runtimeContainer)

  const resolvedDeadZone = store.getResolvedDeadZone()
  const safeArea = useSafeAreaInsets()
  const desktopWorkspaceTopInset =
    safeArea.top + resolvedDeadZone.top + shellStatusBarHeight('desktop')
  const desktopViewportBounds = getDesktopWindowWorkspaceBounds({
    deadZone: resolvedDeadZone,
    safeArea,
    topInset: desktopWorkspaceTopInset,
    viewportSize,
  })
  const workspaceInnerWidth = Math.max(
    workspaceSize.width -
      resolvedDeadZone.left -
      resolvedDeadZone.right -
      safeArea.left -
      safeArea.right,
    320,
  )

  // ---------------------------------------------------------------------------
  // Action handlers (thin wrappers around store)
  // ---------------------------------------------------------------------------

  const logActivity = (message: string) => store.logActivity(message, locale)

  const handleOpenApp = (appId: string) => {
    const app = findDesktopAppById(apps, appId)
    if (!app) return

    if (isMobile && app.manifest.mobileRedirectPath) {
      navigate(app.manifest.mobileRedirectPath)
      return
    }

    if (app.manifest.placement === 'new-container' || app.tier === 'external') {
      logActivity(
        t('activity.external', 'Requested new-container launch for {{name}}', {
          name: t(app.labelKey, app.id),
        }),
      )
      store.setSnackbar(t('external.body'))
      return
    }

    store.openApp(appId, {
      isMobile,
      navigate,
      logActivity,
      viewportBounds: desktopViewportBounds,
    })
    logActivity(
      t('activity.opened', 'Opened {{name}}', { name: t(app.labelKey, app.id) }),
    )
  }

  const handleCloseWindow = (windowId: string) => {
    const closing = windows.find((w) => w.id === windowId)
    if (closing) {
      const app = findDesktopAppById(apps, closing.appId)
      logActivity(
        t('activity.closed', 'Closed {{name}}', {
          name: t(app?.labelKey ?? closing.titleKey),
        }),
      )
    }
    store.closeWindow(windowId)
  }

  const minimizeWindow = (windowId: string) => {
    const target = windows.find((w) => w.id === windowId)
    if (target) {
      const app = findDesktopAppById(apps, target.appId)
      logActivity(
        t('activity.minimized', 'Minimized {{name}}', {
          name: t(app?.labelKey ?? target.titleKey),
        }),
      )
    }
    store.minimizeWindow(windowId)
  }

  const toggleMaximizeWindow = (windowId: string) => {
    const target = windows.find((w) => w.id === windowId)
    if (target) {
      const app = findDesktopAppById(apps, target.appId)
      logActivity(
        t('activity.maximized', 'Toggled maximize for {{name}}', {
          name: t(app?.labelKey ?? target.titleKey),
        }),
      )
    }
    store.toggleMaximizeWindow(windowId)
  }

  const focusWindow = (windowId: string) => store.focusWindow(windowId)
  const updateWindowGeometry = (
    windowId: string,
    geometry: Partial<Pick<import('../models/ui').WindowRecord, 'x' | 'y' | 'width' | 'height'>>,
  ) => store.updateWindowGeometry(windowId, geometry)

  const applySettings = (values: SystemPreferencesInput) => {
    store.applySettings(values, { setLocale, setThemeMode, viewportSize })
    logActivity(t('activity.saved'))
  }

  const restoreDefaults = () => store.restoreDefaults()
  const handleReturnDesktop = () => store.returnToDesktop()

  const toggleSidebar = () => store.toggleSystemSidebar()
  const closeSidebar = () => store.closeSystemSidebar()
  const handleSelectSidebarApp = (appId: string) => {
    handleOpenApp(appId)
    closeSidebar()
  }
  const handleCycleLocale = () => setLocale(nextSupportedLocale(locale))
  const handleToggleTheme = () =>
    setThemeMode(themeMode === 'light' ? 'dark' : 'light')

  // ---------------------------------------------------------------------------
  // Drag suppression helpers (view-layer only)
  // ---------------------------------------------------------------------------

  const suppressNextOpen = (itemId: string) => {
    suppressOpenItemId.current = itemId
    window.setTimeout(() => {
      if (suppressOpenItemId.current === itemId) suppressOpenItemId.current = null
    }, 180)
  }

  const blockOpenAfterDrag = (itemId: string) => {
    draggedOpenBlockItemId.current = itemId
    if (draggedOpenBlockTimeoutId.current) {
      window.clearTimeout(draggedOpenBlockTimeoutId.current)
    }
    draggedOpenBlockTimeoutId.current = window.setTimeout(() => {
      if (draggedOpenBlockItemId.current === itemId) draggedOpenBlockItemId.current = null
      draggedOpenBlockTimeoutId.current = null
    }, 240)
  }

  const consumeOpenBlock = (itemId: string) => {
    if (draggedOpenBlockItemId.current === itemId) {
      draggedOpenBlockItemId.current = null
      if (draggedOpenBlockTimeoutId.current) {
        window.clearTimeout(draggedOpenBlockTimeoutId.current)
        draggedOpenBlockTimeoutId.current = null
      }
      return true
    }
    if (suppressOpenItemId.current === itemId) {
      suppressOpenItemId.current = null
      return true
    }
    return false
  }

  const handleGridDragStart = (
    _pageId: string,
    oldItem: GridLayoutItem | null,
    newItem: GridLayoutItem | null,
  ) => {
    const itemId = newItem?.i ?? oldItem?.i
    if (!itemId) return
    blockOpenAfterDrag(itemId)
    suppressNextOpen(itemId)
  }

  const handleGridDragStop = (
    pageId: string,
    oldItem: GridLayoutItem | null,
    newItem: GridLayoutItem | null,
  ) => {
    const itemId = newItem?.i ?? oldItem?.i
    if (itemId) {
      blockOpenAfterDrag(itemId)
      suppressNextOpen(itemId)
    }
    if (!newItem) return
    store.handleGridDragStop(pageId, oldItem, newItem)
  }

  const handleLayoutChange = (_pageId: string, _nextLayout: Layout) => {}

  // ---------------------------------------------------------------------------
  // Derived data
  // ---------------------------------------------------------------------------

  const windowLayerModel = useMemo(
    () => store.getWindowLayerModel(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [apps, windows],
  )
  const topMobileWindow = windowLayerModel.topWindow
  const activeMobileApp =
    formFactor === 'mobile' && topMobileWindow ? topMobileWindow.app : undefined
  const shellBarHeight = shellStatusBarHeight(formFactor, activeMobileApp)
  const mobileSheetTopInset =
    activeMobileApp && mobileStatusBarMode(activeMobileApp) === 'standard'
      ? safeArea.top + resolvedDeadZone.top + shellBarHeight
      : 0
  const workspaceTopPadding =
    formFactor === 'mobile' && topMobileWindow
      ? safeArea.top + resolvedDeadZone.top
      : desktopWorkspaceTopInset
  const workspaceInnerHeight = Math.max(
    workspaceSize.height -
      workspaceTopPadding -
      resolvedDeadZone.bottom -
      safeArea.bottom,
    360,
  )
  const shouldLockDesktopViewport =
    formFactor === 'desktop' &&
    viewportSize.width >= desktopMinCanvasSize.width &&
    viewportSize.height >= desktopMinCanvasSize.height
  const systemSidebarModel = store.getSystemSidebarDataModel(activeMobileApp?.id)

  const trayState = useMemo(() => {
    const statusTips = [
      {
        id: 'recent-shell-action',
        tone: 'success' as const,
        taskLabel: t('tips.task.shell'),
        title: t('tips.card.recent.title'),
        body: activityLog[0] ?? t('tips.card.recent.body'),
        statusLabel: t('tips.status.completed'),
        timeLabel: t('tips.time.justNow'),
      },
      {
        id: 'mobile-touch-audit',
        tone: 'error' as const,
        taskLabel: t('tips.task.mobile'),
        title: t('tips.card.touch.title'),
        body: t('tips.card.touch.body'),
        statusLabel: t('tips.status.failed'),
        timeLabel: t('tips.time.twoMinutes'),
      },
      {
        id: 'diagnostics-export',
        tone: 'progress' as const,
        taskLabel: t('tips.task.report'),
        title: t('tips.card.report.title'),
        body: t('tips.card.report.body'),
        statusLabel: t('tips.status.running'),
        timeLabel: t('tips.time.queue'),
      },
      {
        id: 'runtime-cache-warmed',
        tone: 'success' as const,
        taskLabel: t('tips.task.runtime'),
        title: t('tips.card.cache.title'),
        body: t('tips.card.cache.body'),
        statusLabel: t('tips.status.completed'),
        timeLabel: '5m',
      },
      {
        id: 'notes-sync-retry',
        tone: 'error' as const,
        taskLabel: t('tips.task.sync'),
        title: t('tips.card.sync.title'),
        body: t('tips.card.sync.body'),
        statusLabel: t('tips.status.failed'),
        timeLabel: '9m',
      },
      {
        id: 'docs-index-refresh',
        tone: 'progress' as const,
        taskLabel: t('tips.task.index'),
        title: t('tips.card.index.title'),
        body: t('tips.card.index.body'),
        statusLabel: t('tips.status.running'),
        timeLabel: '12m',
      },
      {
        id: 'launcher-metrics-pushed',
        tone: 'success' as const,
        taskLabel: t('tips.task.metrics'),
        title: t('tips.card.metrics.title'),
        body: t('tips.card.metrics.body'),
        statusLabel: t('tips.status.completed'),
        timeLabel: '18m',
      },
      {
        id: 'widget-layout-reflow',
        tone: 'progress' as const,
        taskLabel: t('tips.task.layout'),
        title: t('tips.card.layout.title'),
        body: t('tips.card.layout.body'),
        statusLabel: t('tips.status.running'),
        timeLabel: '24m',
      },
    ]

    return {
      backupActive: windows.some(
        (w) => w.appId === 'files' && w.state !== 'minimized',
      ),
      messageCount: Math.min(
        windows.filter((w) => w.state !== 'minimized').length,
        3,
      ),
      notificationCount: Math.min(statusTips.length, 9),
      tips: statusTips,
    }
  }, [activityLog, t, windows])

  // Background
  const backgroundWallpaper = useMemo(
    () => appearance.wallpaper ?? { mode: 'infinite' as const },
    [appearance.wallpaper],
  )
  const backgroundPageCount = resolvedLayout?.pages.length ?? 1

  useEffect(() => {
    setBackground({
      wallpaper: backgroundWallpaper,
      pageCount: backgroundPageCount,
      viewportProgress,
    })
  }, [backgroundPageCount, backgroundWallpaper, setBackground, viewportProgress])

  useEffect(() => resetBackground, [resetBackground])

  // ---------------------------------------------------------------------------
  // Render
  // ---------------------------------------------------------------------------

  return (
    <main
      className="relative isolate bg-[color:var(--cp-bg)]"
      style={
        shouldLockDesktopViewport
          ? { height: '100dvh', overflow: 'hidden' }
          : { minHeight: '100dvh' }
      }
    >
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
              <SystemSidebar
                connectionState={connectionState}
                deadZone={resolvedDeadZone}
                onClose={closeSidebar}
                onOpenApp={handleSelectSidebarApp}
                onReturnDesktop={handleReturnDesktop}
                open={isSystemSidebarOpen}
                runtimeContainer={runtimeContainer}
                safeAreaTop={safeArea.top}
                safeAreaBottom={safeArea.bottom}
                uiModel={systemSidebarModel}
              />
              <StatusBar
                activeApp={activeMobileApp}
                connectionState={connectionState}
                deadZone={resolvedDeadZone}
                formFactor={formFactor}
                safeAreaTop={safeArea.top}
                onCycleLocale={handleCycleLocale}
                onMinimizeWindow={
                  isMobile && topMobileWindow
                    ? () => minimizeWindow(topMobileWindow.id)
                    : undefined
                }
                onOpenDiagnostics={() => handleSelectSidebarApp('diagnostics')}
                onOpenSettings={() => handleSelectSidebarApp('settings')}
                onOpenSidebar={toggleSidebar}
                onToggleTheme={handleToggleTheme}
                themeMode={themeMode}
                trayState={trayState}
              />
              <div
                ref={gridContainerRef}
                className="relative overflow-hidden"
                data-density={density}
                style={{
                  minWidth: isMobile ? undefined : desktopMinCanvasSize.width,
                  minHeight: isMobile ? undefined : desktopMinCanvasSize.height,
                  paddingTop: workspaceTopPadding,
                  paddingBottom: resolvedDeadZone.bottom + safeArea.bottom,
                  paddingLeft: resolvedDeadZone.left + safeArea.left,
                  paddingRight: resolvedDeadZone.right + safeArea.right,
                }}
              >
                {resolvedLayout.pages.length === 0 ||
                resolvedLayout.pages.every((page) => page.items.length === 0) ? (
                  <EmptyState onRestore={restoreDefaults} />
                ) : (
                  <Swiper
                    modules={[Pagination]}
                    allowTouchMove={isMobile}
                    pagination={{ clickable: true }}
                    className="h-full"
                    style={{ height: workspaceInnerHeight }}
                    onSwiper={(swiper) =>
                      store.setViewportProgress(
                        normalizeViewportProgress(
                          swiper.progress,
                          resolvedLayout.pages.length,
                        ),
                      )
                    }
                    onProgress={(swiper) =>
                      store.setViewportProgress(
                        normalizeViewportProgress(
                          swiper.progress,
                          resolvedLayout.pages.length,
                        ),
                      )
                    }
                    onSlideChange={(swiper) =>
                      store.setViewportProgress(
                        normalizeViewportProgress(
                          swiper.progress,
                          resolvedLayout.pages.length,
                        ),
                      )
                    }
                  >
                    {resolvedLayout.pages.map((page) => (
                      <SwiperSlide key={page.id} className="h-full">
                        <div className="h-full px-4 pb-16 pt-6 sm:px-6">
                          <GridLayoutBase
                            className="layout h-full"
                            gridConfig={{
                              cols: gridSpec.cols,
                              rowHeight: gridSpec.rowHeight,
                              margin: [GRID_GAP, GRID_GAP],
                              containerPadding: [0, 0],
                              maxRows: gridSpec.rows,
                            }}
                            layout={mapPageToGrid(page)}
                            width={workspaceInnerWidth - (isMobile ? 32 : 48)}
                            resizeConfig={{ enabled: false }}
                            dragConfig={{
                              enabled: true,
                              handle: '.desktop-tile-shell',
                              cancel: '.widget-interactive',
                              threshold: isMobile ? 5 : 4,
                            }}
                            compactor={noCompactor}
                            onDragStart={(_, oldItem, newItem) =>
                              handleGridDragStart(page.id, oldItem, newItem)
                            }
                            onLayoutChange={(next: Layout) =>
                              handleLayoutChange(page.id, next)
                            }
                            onDragStop={(_, oldItem, newItem) =>
                              handleGridDragStop(page.id, oldItem, newItem)
                            }
                          >
                            {page.items.map((item) => (
                              <div key={item.id}>
                                <DesktopTile
                                  app={
                                    item.type === 'app'
                                      ? findDesktopAppById(apps, item.appId)
                                      : undefined
                                  }
                                  isDesktop={!isMobile}
                                  item={item}
                                  isSelected={selectedItemId === item.id}
                                  onOpen={() =>
                                    item.type === 'app'
                                      ? consumeOpenBlock(item.id)
                                        ? undefined
                                        : handleOpenApp(item.appId)
                                      : store.setSelectedItemId(item.id)
                                  }
                                  onOpenContextMenu={(event) => {
                                    event.preventDefault()
                                    store.setSelectedItemId(item.id)
                                    store.setContextMenu({
                                      itemId: item.id,
                                      mouseX: event.clientX + 2,
                                      mouseY: event.clientY - 6,
                                    })
                                  }}
                                  onSaveNote={(itemId, content) =>
                                    store.updateWidgetNote(itemId, content)
                                  }
                                />
                              </div>
                            ))}
                          </GridLayoutBase>
                        </div>
                      </SwiperSlide>
                    ))}
                  </Swiper>
                )}
              </div>

              {!isMobile && (
                <DesktopWindowLayer
                  activityLog={activityLog}
                  deadZone={resolvedDeadZone}
                  layoutState={resolvedLayout}
                  locale={locale}
                  onClose={handleCloseWindow}
                  onGeometryChange={updateWindowGeometry}
                  onFocus={focusWindow}
                  onMaximize={toggleMaximizeWindow}
                  onMinimize={minimizeWindow}
                  onSaveSettings={applySettings}
                  runtimeContainer={runtimeContainer}
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
          onClick={() => {
            const pageIndex =
              contextMenu && resolvedLayout
                ? getPageIndex(resolvedLayout, contextMenu.itemId)
                : -1
            const item =
              pageIndex >= 0
                ? resolvedLayout?.pages[pageIndex].items.find(
                    (entry) => entry.id === contextMenu?.itemId,
                  )
                : null
            if (item?.type === 'app') {
              handleOpenApp(item.appId)
            }
            store.setContextMenu(null)
          }}
        >
          {t('common.open')}
        </MenuItem>
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
// Sub-components (unchanged)
// ---------------------------------------------------------------------------

function LoadingState() {
  const { t } = useI18n()
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-[color:var(--cp-surface)]/72 backdrop-blur-xl">
      <div className="shell-panel max-w-lg px-7 py-8 text-center">
        <div className="mx-auto mb-5 flex h-16 w-16 items-center justify-center rounded-full bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_16%,var(--cp-surface))]">
          <div className="h-11 w-11 animate-pulse rounded-full border border-[color:color-mix(in_srgb,var(--cp-accent)_26%,transparent)] bg-[radial-gradient(circle_at_30%_30%,color-mix(in_srgb,var(--cp-accent-soft)_65%,white),color-mix(in_srgb,var(--cp-accent)_88%,transparent))]" />
        </div>
        <p className="shell-kicker">Prototype</p>
        <p className="mt-2 font-display text-2xl font-semibold sm:text-[2rem]">
          {t('states.loadingTitle')}
        </p>
        <p className="mx-auto mt-2 max-w-sm text-sm leading-6 text-[color:var(--cp-muted)]">
          {t('states.loadingBody')}
        </p>
      </div>
    </div>
  )
}

function ErrorState({ onRetry }: { onRetry: () => void }) {
  const { t } = useI18n()
  return (
    <div className="absolute inset-0 flex items-center justify-center bg-[color:var(--cp-surface)]/78 backdrop-blur-xl">
      <div className="shell-panel max-w-lg px-7 py-8 text-center">
        <p className="shell-kicker">Recovery</p>
        <p className="mt-2 font-display text-2xl font-semibold">
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

function EmptyState({ onRestore }: { onRestore: () => void }) {
  const { t } = useI18n()
  return (
    <div className="flex h-full items-center justify-center px-4">
      <div className="shell-panel max-w-lg border-dashed px-7 py-8 text-center">
        <p className="shell-kicker">Layout</p>
        <p className="mt-2 font-display text-2xl font-semibold">
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
  isDesktop,
  item,
  isSelected,
  onOpen,
  onOpenContextMenu,
  onSaveNote,
}: {
  app?: AppDefinition
  isDesktop: boolean
  item: LayoutItem
  isSelected: boolean
  onOpen: () => void
  onOpenContextMenu: (event: ReactMouseEvent<HTMLDivElement>) => void
  onSaveNote: (itemId: string, content: string) => void
}) {
  const { t } = useI18n()
  const touchStartRef = useRef<{
    x: number
    y: number
    pointerId: number
  } | null>(null)
  const releaseAppPointerRef = useRef<(() => void) | null>(null)

  useEffect(() => {
    return () => {
      releaseAppPointerRef.current?.()
      releaseAppPointerRef.current = null
    }
  }, [])

  const clearAppPointer = () => {
    touchStartRef.current = null
    releaseAppPointerRef.current?.()
    releaseAppPointerRef.current = null
  }

  const completeAppPointer = (
    pointerId: number,
    clientX: number,
    clientY: number,
    pointerType: string,
  ) => {
    if (isDesktop || pointerType === 'mouse') return
    const start = touchStartRef.current
    clearAppPointer()
    if (!start || start.pointerId !== pointerId) return
    const distance = Math.hypot(clientX - start.x, clientY - start.y)
    if (distance <= 12) onOpen()
  }

  const handleAppPointerDown = (
    event: ReactPointerEvent<HTMLButtonElement>,
  ) => {
    if (isDesktop || event.pointerType === 'mouse') return
    clearAppPointer()
    touchStartRef.current = {
      x: event.clientX,
      y: event.clientY,
      pointerId: event.pointerId,
    }

    const handleWindowPointerUp = (pointerEvent: PointerEvent) => {
      completeAppPointer(
        pointerEvent.pointerId,
        pointerEvent.clientX,
        pointerEvent.clientY,
        pointerEvent.pointerType,
      )
    }

    const handleWindowPointerCancel = (pointerEvent: PointerEvent) => {
      if (touchStartRef.current?.pointerId === pointerEvent.pointerId) {
        clearAppPointer()
      }
    }

    window.addEventListener('pointerup', handleWindowPointerUp)
    window.addEventListener('pointercancel', handleWindowPointerCancel)
    releaseAppPointerRef.current = () => {
      window.removeEventListener('pointerup', handleWindowPointerUp)
      window.removeEventListener('pointercancel', handleWindowPointerCancel)
    }
  }

  const handleAppPointerUp = (
    event: ReactPointerEvent<HTMLButtonElement>,
  ) => {
    completeAppPointer(
      event.pointerId,
      event.clientX,
      event.clientY,
      event.pointerType,
    )
  }

  return (
    <div
      data-testid={`desktop-item-${item.id}`}
      className={clsx(
        'desktop-tile-shell group relative h-full border border-transparent transition-[transform,border-color,box-shadow,background-color] duration-200 ease-[var(--cp-ease-emphasis)]',
        item.type === 'widget' ? 'rounded-[22px]' : 'rounded-[28px]',
        item.type === 'widget' ? 'overflow-hidden' : 'overflow-visible',
        isDesktop ? 'cursor-grab active:cursor-grabbing' : '',
        isSelected
          ? 'border-[color:color-mix(in_srgb,var(--cp-accent)_46%,transparent)] shadow-[0_0_0_4px_color-mix(in_srgb,var(--cp-accent)_15%,transparent)]'
          : item.type === 'widget'
            ? 'hover:border-[color:color-mix(in_srgb,var(--cp-border)_84%,transparent)]'
            : '',
      )}
      onContextMenu={onOpenContextMenu}
    >
      {item.type === 'app' && app ? (
        <button
          type="button"
          onClick={isDesktop ? onOpen : undefined}
          onPointerDown={handleAppPointerDown}
          onPointerUp={handleAppPointerUp}
          onPointerCancel={clearAppPointer}
          data-testid={`desktop-app-${app.id}`}
          title={t(app.labelKey, app.id)}
          className={clsx(
            'flex h-full w-full flex-col items-center rounded-[28px] bg-transparent text-center transition-[background-color,box-shadow] duration-200 ease-[var(--cp-ease-emphasis)] focus-visible:bg-[color:color-mix(in_srgb,var(--cp-accent-soft)_10%,var(--cp-surface))]',
            isDesktop ? 'cursor-grab active:cursor-grabbing' : '',
          )}
          style={{ paddingTop: 'var(--icon-padding-top)' }}
        >
          <span
            className="relative flex shrink-0 items-center justify-center overflow-hidden rounded-[20px] shadow-[0_8px_20px_color-mix(in_srgb,var(--cp-shadow)_10%,transparent)]"
            style={{
              width: 'var(--icon-size)',
              height: 'var(--icon-size)',
              ...appIconSurfaceStyle(app.accent),
            }}
          >
            <AppIcon iconKey={app.iconKey} className="text-white" />
          </span>
          <span
            className="max-w-full overflow-hidden px-0.5 font-display font-semibold text-[color:var(--cp-text)]"
            style={{
              paddingTop: 'var(--label-padding-top)',
              fontSize: 'var(--font-size-label)',
              lineHeight: 'var(--line-height-label)',
              display: '-webkit-box',
              WebkitLineClamp: 2,
              WebkitBoxOrient: 'vertical',
            }}
          >
            {t(app.labelKey, app.id)}
          </span>
        </button>
      ) : null}

      {item.type === 'widget' ? (
        <DesktopWidgetRenderer item={item} onSaveNote={onSaveNote} />
      ) : null}
    </div>
  )
}
