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
import useSWR from 'swr'
import { globalSettingsStore } from '../app/settings/mock/store'
import {
  findDesktopAppById,
  resolveDesktopApps,
} from '../app/registry'
import type { DesktopAppItem } from '../app/types'
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
  getDesktopWindowPositionBounds,
  getDesktopWindowWorkspaceBounds,
} from './windows/geometry'
import { MobileNavProvider } from './windows/MobileNavContext'
import { MobileWindowSheet } from './windows/MobileWindowSheet'
import {
  createDesktopWindowLayerDataModel,
  createWindowRecord,
  resolveDesktopWindowSizing,
} from './windows/model'
import {
  mobileStatusBarMode,
  shellStatusBarHeight,
  type ConnectionState,
  type StatusTip,
  type StatusTrayState,
} from './shell'
import { useI18n } from '../i18n/provider'
import { defaultDeadZone } from '../mock/data'
import { fetchDesktopPayload } from '../mock/provider'
import {
  defaultWindowAppearancePreferences,
  isLauncherApp,
  supportedLocales,
  windowAppearancePreferencesSchema,
} from '../models/ui'
import type {
  AppDefinition,
  DesktopPageState,
  FormFactor,
  LayoutItem,
  LayoutState,
  MockScenario,
  SupportedLocale,
  SystemSidebarAppItem,
  SystemSidebarDataModel,
  SystemPreferencesInput,
  ThemeMode,
  WindowAppearancePreferences,
  WindowRecord,
} from '../models/ui'
import { useThemeMode } from '../theme/provider'

const runtimeStorageKey = 'buckyos.prototype.runtime.v1'
const windowGeometryStorageKey = 'buckyos.window-geometry.desktop.v1'
const windowAppearanceStorageKey = 'buckyos.window-appearance.v1'
const desktopMinCanvasSize = { width: 960, height: 720 }

type WindowGeometry = Pick<WindowRecord, 'x' | 'y' | 'width' | 'height'>
type WindowGeometryMap = Record<string, WindowGeometry>

function isFiniteNumber(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value)
}

function sanitizeWindowGeometryMap(
  input: unknown,
): WindowGeometryMap {
  if (!input || typeof input !== 'object') {
    return {}
  }

  return Object.fromEntries(
    Object.entries(input).flatMap(([appId, geometry]) => {
      if (
        !geometry ||
        typeof geometry !== 'object' ||
        !isFiniteNumber((geometry as WindowGeometry).x) ||
        !isFiniteNumber((geometry as WindowGeometry).y) ||
        !isFiniteNumber((geometry as WindowGeometry).width) ||
        !isFiniteNumber((geometry as WindowGeometry).height)
      ) {
        return []
      }

      return [[appId, geometry as WindowGeometry]]
    }),
  )
}

function clamp(value: number, min: number, max: number) {
  return Math.min(Math.max(value, min), max)
}

function normalizeWindowGeometryForViewport(
  app: AppDefinition,
  geometry: Partial<WindowGeometry> | undefined,
  index: number,
  viewportBounds: ReturnType<typeof getDesktopWindowWorkspaceBounds>,
) {
  const sizing = resolveDesktopWindowSizing(app)
  const minWidth = Math.min(sizing.minWidth, viewportBounds.maxWidth)
  const minHeight = Math.min(sizing.minHeight, viewportBounds.maxHeight)
  const width = clamp(
    geometry?.width ?? sizing.width,
    minWidth,
    viewportBounds.maxWidth,
  )
  const height = clamp(
    geometry?.height ?? sizing.height,
    minHeight,
    viewportBounds.maxHeight,
  )
  const defaultX = viewportBounds.minX + 24 + (index % 4) * 36
  const defaultY = viewportBounds.minY + 18 + (index % 3) * 32
  const positionBounds = getDesktopWindowPositionBounds(viewportBounds, {
    width,
    height,
  })

  return {
    width,
    height,
    x: clamp(geometry?.x ?? defaultX, positionBounds.minX, positionBounds.maxX),
    y: clamp(geometry?.y ?? defaultY, positionBounds.minY, positionBounds.maxY),
  }
}

function sameWindowGeometry(left: WindowGeometry | undefined, right: WindowGeometry) {
  return (
    left?.x === right.x &&
    left?.y === right.y &&
    left?.width === right.width &&
    left?.height === right.height
  )
}

/**
 * Reads env(safe-area-inset-*) values for immersive fullscreen on mobile.
 * Requires viewport-fit=cover on the viewport meta tag.
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

/** Density tier for the grid slot system. */
export type GridDensity = 'small' | 'medium' | 'large'

const densityRowHeight: Record<GridDensity, number> = {
  small: 92,
  medium: 108,
  large: 124,
}

const GRID_GAP = 2

/**
 * Maximum width (px) a single grid cell may occupy on desktop.
 * When the container is wide enough that cells would exceed this,
 * more columns are added to keep cells compact.
 */
const MAX_CELL_WIDTH = 110

/** Minimum columns on desktop so the grid never looks too sparse. */
const MIN_DESKTOP_COLS = 6

/** Compute column count so that each cell stays within MAX_CELL_WIDTH. */
function columnsForWidth(width: number): number {
  const cols = Math.ceil((width + GRID_GAP) / (MAX_CELL_WIDTH + GRID_GAP))
  return Math.max(MIN_DESKTOP_COLS, cols)
}

/**
 * Minimum row height on desktop — more compact than the density value
 * so that more rows fit and the grid fully utilises vertical space.
 * icon-padding-top(10) + icon(48) + label-padding(4) + 1 line(16) = 78
 */
const DESKTOP_MIN_ROW_HEIGHT = 78

/**
 * Compute how many rows fit in the available height.
 * On desktop uses a compact minimum; on mobile uses the density value.
 */
function rowsForHeight(
  height: number,
  density: GridDensity,
  isMobile: boolean,
): number {
  const slotH = isMobile ? densityRowHeight[density] : DESKTOP_MIN_ROW_HEIGHT
  return Math.max(1, Math.floor((height + GRID_GAP) / (slotH + GRID_GAP)))
}

/**
 * Given the number of rows that fit, compute the actual row height
 * so that the grid fills the entire container height evenly.
 */
function stretchedRowHeight(height: number, rows: number): number {
  if (rows <= 0) return densityRowHeight.medium
  return (height - (rows - 1) * GRID_GAP) / rows
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
      // Sync CSS variable for any pure-CSS consumers
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

  if (!isNavigatorOnline) {
    return 'offline'
  }

  return runtimeContainer === 'browser' ? 'degraded' : 'online'
}

function nextSupportedLocale(locale: SupportedLocale) {
  const currentIndex = supportedLocales.indexOf(locale)
  return supportedLocales[(currentIndex + 1) % supportedLocales.length]
}

function normalizeViewportProgress(progress: number, pageCount: number) {
  if (!Number.isFinite(progress) || pageCount <= 1) {
    return 0
  }

  return Math.min(Math.max(progress, 0), 1)
}

function readJson<T>(key: string) {
  const raw = window.localStorage.getItem(key)
  if (!raw) {
    return null
  }

  try {
    return JSON.parse(raw) as T
  } catch {
    return null
  }
}

function writeJson<T>(key: string, value: T) {
  window.localStorage.setItem(key, JSON.stringify(value))
}

function readWindowAppearancePreferences() {
  const parsed = windowAppearancePreferencesSchema.safeParse(
    readJson(windowAppearanceStorageKey),
  )

  return parsed.success
    ? parsed.data
    : { ...defaultWindowAppearancePreferences }
}

function layoutStorageKey(formFactor: FormFactor) {
  return `buckyos.layout.${formFactor}.v1`
}

function legacyDeadZone(formFactor: FormFactor) {
  return formFactor === 'desktop'
    ? { top: 64, bottom: 24, left: 20, right: 20 }
    : { top: 52, bottom: 20, left: 12, right: 12 }
}

function matchesDeadZone(
  target: LayoutState['deadZone'] | undefined,
  expected: LayoutState['deadZone'],
) {
  return (
    target?.top === expected.top &&
    target?.bottom === expected.bottom &&
    target?.left === expected.left &&
    target?.right === expected.right
  )
}

function migrateDeadZone(layout: LayoutState, formFactor: FormFactor) {
  if (!matchesDeadZone(layout.deadZone, legacyDeadZone(formFactor))) {
    return layout
  }

  return {
    ...layout,
    deadZone: { ...defaultDeadZone },
  }
}

const systemSidebarSystemAppIds = new Set(['settings', 'diagnostics'])

function createSystemSidebarDataModel(
  apps: DesktopAppItem[],
  windows: WindowRecord[],
  currentAppId?: string,
): SystemSidebarDataModel {
  const appMap = new Map(apps.map((app) => [app.id, app]))
  const toSidebarApp = (
    app: DesktopAppItem | undefined,
  ): SystemSidebarAppItem | null =>
    app
      ? {
          appId: app.id,
          iconKey: app.iconKey,
          labelKey: app.labelKey,
        }
      : null

  const seenSwitchApps = new Set<string>()
  const switchApps = windows
    .filter(
      (windowItem) =>
        windowItem.state === 'minimized' &&
        windowItem.minimizedOrder !== null &&
        !systemSidebarSystemAppIds.has(windowItem.appId),
    )
    .sort((a, b) => (a.minimizedOrder ?? 0) - (b.minimizedOrder ?? 0))
    .map((windowItem) => {
      const app = appMap.get(windowItem.appId)

      if (
        !app ||
        windowItem.minimizedOrder === null ||
        seenSwitchApps.has(app.id)
      ) {
        return null
      }

      seenSwitchApps.add(app.id)

      return {
        appId: app.id,
        iconKey: app.iconKey,
        labelKey: app.labelKey,
        minimizedOrder: windowItem.minimizedOrder,
      }
    })
    .filter((app): app is SystemSidebarDataModel['switchApps'][number] => Boolean(app))

  const systemApps = ['settings', 'diagnostics', 'users-agents']
    .map((appId) => toSidebarApp(appMap.get(appId)))
    .filter((app): app is SystemSidebarAppItem => Boolean(app))

  return {
    currentAppId,
    runningAppCount: windows.filter((windowItem) => windowItem.state !== 'minimized')
      .length,
    switchApps,
    systemApps,
  }
}

function getPageIndex(layoutState: LayoutState, itemId: string) {
  return layoutState.pages.findIndex((page) =>
    page.items.some((item) => item.id === itemId),
  )
}

function fits(
  page: DesktopPageState,
  x: number,
  y: number,
  w: number,
  h: number,
  cols: number,
  rows: number,
  excludeId?: string,
) {
  if (x + w > cols || y + h > rows) {
    return false
  }

  return !page.items.some((item) => {
    if (item.id === excludeId) {
      return false
    }

    if (item.x === undefined || item.y === undefined) {
      return false
    }

    return !(
      x + w <= item.x ||
      item.x + item.w <= x ||
      y + h <= item.y ||
      item.y + item.h <= y
    )
  })
}

/**
 * Scan order for placing items in the grid.
 * - 'row-major': left→right, then top→bottom (mobile)
 * - 'col-major': top→bottom, then left→right (desktop)
 */
type ScanOrder = 'row-major' | 'col-major'

/**
 * Find a slot at the tail of the page (after the last positioned content).
 * Unlike `findNextSlot` which scans from (0,0) and fills gaps,
 * this only places items after the last content, preserving manual layout.
 *
 * @param scanOrder 'col-major' scans top→bottom per column (desktop),
 *                  'row-major' scans left→right per row (mobile).
 */
function findTailSlot(
  page: DesktopPageState,
  w: number,
  h: number,
  cols: number,
  rows: number,
  scanOrder: ScanOrder = 'row-major',
): { x: number; y: number } | null {
  if (scanOrder === 'col-major') {
    return findTailSlotColMajor(page, w, h, cols, rows)
  }

  // --- row-major (mobile) ---
  let maxLinearEnd = 0
  for (const item of page.items) {
    if (item.x === undefined || item.y === undefined) continue
    for (let row = item.y; row < item.y + item.h; row++) {
      const linearEnd = row * cols + (item.x + item.w)
      maxLinearEnd = Math.max(maxLinearEnd, linearEnd)
    }
  }

  const startRow = Math.floor(maxLinearEnd / cols)
  const startCol = maxLinearEnd % cols

  for (let y = startRow; y + h <= rows; y++) {
    const sx = y === startRow ? startCol : 0
    for (let x = sx; x + w <= cols; x++) {
      if (fits(page, x, y, w, h, cols, rows)) {
        return { x, y }
      }
    }
  }

  return null
}

/**
 * Column-major tail placement: scan top→bottom within each column,
 * then move to the next column. Position 0 = (0,0), 1 = (0,1), …
 */
function findTailSlotColMajor(
  page: DesktopPageState,
  w: number,
  h: number,
  cols: number,
  rows: number,
): { x: number; y: number } | null {
  // Compute the column-major linear end of all positioned items
  let maxLinearEnd = 0
  for (const item of page.items) {
    if (item.x === undefined || item.y === undefined) continue
    for (let col = item.x; col < item.x + item.w; col++) {
      const linearEnd = col * rows + (item.y + item.h)
      maxLinearEnd = Math.max(maxLinearEnd, linearEnd)
    }
  }

  const startCol = Math.floor(maxLinearEnd / rows)
  const startRow = maxLinearEnd % rows

  for (let x = startCol; x + w <= cols; x++) {
    const sy = x === startCol ? startRow : 0
    for (let y = sy; y + h <= rows; y++) {
      if (fits(page, x, y, w, h, cols, rows)) {
        return { x, y }
      }
    }
  }

  return null
}

/**
 * Mark positioned items as unpositioned when their position exceeds
 * the current grid bounds (e.g. after a resize).
 */
function invalidatePositions(
  layout: LayoutState,
  cols: number,
  rows: number,
): LayoutState {
  let anyChanged = false
  const pages = layout.pages.map((page) => {
    let pageChanged = false
    const items = page.items.map((item) => {
      if (item.x === undefined || item.y === undefined) return item
      if (item.x + item.w > cols || item.y + item.h > rows) {
        pageChanged = true
        return { ...item, x: undefined, y: undefined }
      }
      return item
    })
    if (pageChanged) anyChanged = true
    return pageChanged ? { ...page, items } : page
  })
  return anyChanged ? { ...layout, pages } : layout
}

/**
 * Resolve all unpositioned items by placing them at the tail of each page.
 * Returns a fully-positioned layout suitable for rendering.
 *
 * @param scanOrder 'col-major' for desktop (top→bottom per column),
 *                  'row-major' for mobile (left→right per row).
 */
function resolveLayout(
  layout: LayoutState,
  cols: number,
  rows: number,
  scanOrder: ScanOrder = 'row-major',
): LayoutState {
  const unpositioned: LayoutItem[] = []
  const resolvedPages: DesktopPageState[] = layout.pages.map((page) => ({
    ...page,
    items: page.items.filter((item) => {
      if (item.x === undefined || item.y === undefined) {
        unpositioned.push(item)
        return false
      }
      return true
    }),
  }))

  if (unpositioned.length === 0) {
    return layout
  }

  for (const item of unpositioned) {
    let placed = false
    for (const page of resolvedPages) {
      const slot = findTailSlot(page, item.w, item.h, cols, rows, scanOrder)
      if (slot) {
        page.items.push({ ...item, x: slot.x, y: slot.y })
        placed = true
        break
      }
    }
    if (!placed) {
      resolvedPages.push({
        id: `${layout.formFactor}-page-${resolvedPages.length + 1}`,
        items: [{ ...item, x: 0, y: 0 }],
      })
    }
  }

  return { ...layout, pages: resolvedPages }
}

function mapPageToGrid(page: DesktopPageState): GridLayoutItem[] {
  return page.items
    .filter(
      (item): item is LayoutItem & { x: number; y: number } =>
        item.x !== undefined && item.y !== undefined,
    )
    .map((item) => ({
      i: item.id,
      x: item.x,
      y: item.y,
      w: item.w,
      h: item.h,
      static: false,
    }))
}

function sanitizeLayoutForApps(
  layout: LayoutState,
  apps: DesktopAppItem[],
) {
  const launcherAppIds = new Set(
    apps
      .filter((app) => isLauncherApp(app))
      .map((app) => app.id),
  )

  return {
    ...layout,
    pages: layout.pages.map((page) => ({
      ...page,
      items: page.items.filter(
        (item) => item.type === 'widget' || launcherAppIds.has(item.appId),
      ),
    })),
  }
}

function reconcileLayoutWithDefaultApps(
  layout: LayoutState,
  defaultLayout: LayoutState,
  apps: DesktopAppItem[],
  formFactor: FormFactor,
) {
  const sanitizedLayout = sanitizeLayoutForApps(layout, apps)
  const launcherAppIds = new Set(
    apps
      .filter((app) => isLauncherApp(app))
      .map((app) => app.id),
  )
  const existingAppIds = new Set(
    sanitizedLayout.pages.flatMap((page) =>
      page.items.flatMap((item) => (item.type === 'app' ? [item.appId] : [])),
    ),
  )

  const newItems: LayoutItem[] = []
  defaultLayout.pages.forEach((defaultPage) => {
    defaultPage.items.forEach((item) => {
      if (
        item.type !== 'app' ||
        !launcherAppIds.has(item.appId) ||
        existingAppIds.has(item.appId)
      ) {
        return
      }
      newItems.push({ ...item, x: undefined, y: undefined })
      existingAppIds.add(item.appId)
    })
  })

  if (newItems.length === 0) {
    return sanitizedLayout
  }

  const pages = sanitizedLayout.pages.map((page) => ({
    ...page,
    items: [...page.items],
  }))
  if (pages.length === 0) {
    pages.push({ id: `${formFactor}-page-1`, items: [] })
  }
  pages[pages.length - 1].items.push(...newItems)

  return { ...sanitizedLayout, pages }
}

export function DesktopRoute() {
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
  const [layoutState, setLayoutState] = useState<LayoutState | null>(null)
  const [windows, setWindows] = useState<WindowRecord[]>([])
  const [snackbar, setSnackbar] = useState<string | null>(null)
  const [activityLog, setActivityLog] = useState<string[]>([])
  const [isSystemSidebarOpen, setIsSystemSidebarOpen] = useState(false)
  const [selectedItemId, setSelectedItemId] = useState<string | null>(null)
  const [viewportProgress, setViewportProgress] = useState(0)
  const nextMinimizedOrderRef = useRef(1)
  const [runtimeContainer, setRuntimeContainer] = useState(() => {
    return (
      (window.localStorage.getItem(runtimeStorageKey) as
        | 'browser'
        | 'desktop-app'
        | 'mobile-app'
        | null) ?? 'browser'
    )
  })
  const [windowAppearance, setWindowAppearance] = useState<WindowAppearancePreferences>(
    () => readWindowAppearancePreferences(),
  )
  const [contextMenu, setContextMenu] = useState<{
    itemId: string
    mouseX: number
    mouseY: number
  } | null>(null)
  const windowGeometryByAppRef = useRef<WindowGeometryMap>(
    sanitizeWindowGeometryMap(readJson(windowGeometryStorageKey)),
  )
  const suppressOpenItemId = useRef<string | null>(null)
  const draggedOpenBlockItemId = useRef<string | null>(null)
  const draggedOpenBlockTimeoutId = useRef<number | null>(null)
  const workspaceRef = useRef<HTMLDivElement | null>(null)
  const [viewportSize, setViewportSize] = useState(() => ({
    width: window.innerWidth,
    height: window.innerHeight,
  }))
  const [workspaceSize, setWorkspaceSize] = useState({ width: 960, height: 720 })
  const settingsSnap = useSyncExternalStore(globalSettingsStore.subscribe, globalSettingsStore.getSnapshot)
  const density = (settingsSnap.session.appearance.fontSize ?? 'medium') as GridDensity
  const gridContainerRef = useRef<HTMLDivElement | null>(null)

  const { data, error, isLoading, mutate } = useSWR(
    ['desktop-payload', formFactor, scenario],
    ([, nextFormFactor, nextScenario]) =>
      fetchDesktopPayload({
        formFactor: nextFormFactor as FormFactor,
        scenario: nextScenario as MockScenario,
      }),
  )

  const apps = useMemo(
    () => resolveDesktopApps(data?.apps ?? [], formFactor),
    [data?.apps, formFactor],
  )
  const connectionState = useConnectionState(runtimeContainer)
  const gridSpec = useGridSpec(gridContainerRef, density, isMobile)
  const currentSpec = gridSpec
  const resetViewportState = () => {
    setWindows([])
  }
  const applyResolvedLayout = (nextLayout: LayoutState, defaultLayout: LayoutState) => {
    setLayoutState(
      reconcileLayoutWithDefaultApps(nextLayout, defaultLayout, apps, formFactor),
    )
  }

  useEffect(() => {
    resetViewportState()
    setIsSystemSidebarOpen(false)
    setViewportProgress(0)
  }, [formFactor])

  useEffect(() => {
    window.localStorage.setItem(runtimeStorageKey, runtimeContainer)
  }, [runtimeContainer])

  useEffect(() => {
    writeJson(windowAppearanceStorageKey, windowAppearance)
  }, [windowAppearance])

  useEffect(() => {
    return () => {
      if (draggedOpenBlockTimeoutId.current) {
        window.clearTimeout(draggedOpenBlockTimeoutId.current)
      }
    }
  }, [])

  useEffect(() => {
    const current = searchParams.get('scenario') ?? 'normal'
    if (current === scenario) {
      return
    }

    const params = new URLSearchParams(searchParams)
    params.set('scenario', scenario)
    setSearchParams(params, { replace: true })
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scenario])

  useEffect(() => {
    if (!data) {
      return
    }

    if (scenario === 'normal') {
      const stored = readJson<LayoutState>(layoutStorageKey(formFactor))
      applyResolvedLayout(
        stored ? migrateDeadZone(stored, formFactor) : data.layout,
        data.layout,
      )
      return
    }
    applyResolvedLayout(data.layout, data.layout)
  }, [apps, data, formFactor, scenario])

  useEffect(() => {
    if (!layoutState || scenario !== 'normal') {
      return
    }

    writeJson(layoutStorageKey(formFactor), layoutState)
  }, [formFactor, layoutState, scenario])

  // When the grid spec changes, mark out-of-bounds items as unpositioned
  useEffect(() => {
    setLayoutState((prev) => {
      if (!prev) return prev
      const next = invalidatePositions(prev, currentSpec.cols, currentSpec.rows)
      return next !== prev ? next : prev
    })
  }, [currentSpec.cols, currentSpec.rows])

  // Compute a fully-positioned layout for rendering (fills in auto-positions)
  const resolvedLayout = useMemo(() => {
    if (!layoutState) return null
    const scanOrder: ScanOrder = isMobile ? 'row-major' : 'col-major'
    return resolveLayout(layoutState, currentSpec.cols, currentSpec.rows, scanOrder)
  }, [layoutState, currentSpec.cols, currentSpec.rows, isMobile])

  useEffect(() => {
    if (!workspaceRef.current) {
      return
    }

    const resizeObserver = new ResizeObserver((entries) => {
      const entry = entries[0]

      if (!entry) {
        return
      }

      setWorkspaceSize({
        width: entry.contentRect.width,
        height: entry.contentRect.height,
      })
    })

    resizeObserver.observe(workspaceRef.current)

    return () => resizeObserver.disconnect()
  }, [])

  const resolvedDeadZone = useMemo(
    () => layoutState?.deadZone ?? data?.layout.deadZone ?? { top: 0, bottom: 0, left: 0, right: 0 },
    [layoutState?.deadZone, data?.layout.deadZone],
  )
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
    workspaceSize.width
      - resolvedDeadZone.left - resolvedDeadZone.right
      - safeArea.left - safeArea.right,
    320,
  )

  const logActivity = (message: string) => {
    const stamp = new Intl.DateTimeFormat(locale, {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }).format(new Date())

    setActivityLog((prev) => [`${stamp} · ${message}`, ...prev].slice(0, 8))
  }

  const persistWindowGeometry = useCallback((appId: string, geometry: WindowGeometry) => {
    if (sameWindowGeometry(windowGeometryByAppRef.current[appId], geometry)) {
      return
    }

    windowGeometryByAppRef.current = {
      ...windowGeometryByAppRef.current,
      [appId]: geometry,
    }
    writeJson(windowGeometryStorageKey, windowGeometryByAppRef.current)
  }, [])

  const normalizeOpenWindowsForViewport = useCallback((
    nextViewportSize: { width: number; height: number },
    nextDeadZone = resolvedDeadZone,
    nextSafeArea = safeArea,
  ) => {
    const nextBounds = getDesktopWindowWorkspaceBounds({
      deadZone: nextDeadZone,
      safeArea: nextSafeArea,
      topInset:
        nextSafeArea.top + nextDeadZone.top + shellStatusBarHeight('desktop'),
      viewportSize: nextViewportSize,
    })

    setWindows((prev) => {
      let changed = false
      const next = prev.map((windowItem, index) => {
        const app = findDesktopAppById(apps, windowItem.appId)

        if (!app) {
          return windowItem
        }

        const geometry = normalizeWindowGeometryForViewport(
          app,
          windowItem,
          index,
          nextBounds,
        )

        if (sameWindowGeometry(windowItem, geometry)) {
          return windowItem
        }

        changed = true
        persistWindowGeometry(windowItem.appId, geometry)
        return {
          ...windowItem,
          ...geometry,
        }
      })

      return changed ? next : prev
    })
  }, [apps, persistWindowGeometry, resolvedDeadZone, safeArea])

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

  const restoreDefaults = () => {
    if (!data) {
      return
    }

    window.localStorage.removeItem(layoutStorageKey(formFactor))
    window.localStorage.removeItem(windowGeometryStorageKey)
    setLayoutState(structuredClone(data.layout))
    setWindows([])
    windowGeometryByAppRef.current = {}
  }

  const handleOpenApp = (appId: string) => {
    const app = findDesktopAppById(apps, appId)

    if (!app) {
      return
    }

    if (isMobile && app.manifest.mobileRedirectPath) {
      navigate(app.manifest.mobileRedirectPath)
      return
    }

    if (app.manifest.placement === 'new-container' || app.tier === 'external') {
      const message = t('activity.external', 'Requested new-container launch for {{name}}', {
        name: t(app.labelKey, app.id),
      })
      logActivity(message)
      setSnackbar(t('external.body'))
      return
    }

    setWindows((prev) => {
      const existing = prev.find((windowItem) => windowItem.appId === appId)

      if (existing) {
        return prev.map((windowItem, index) =>
          windowItem.id === existing.id
            ? {
                ...windowItem,
                state:
                  app.manifest.defaultMode === 'windowed'
                    ? 'windowed'
                    : 'maximized',
                minimizedOrder: null,
                zIndex: prev.length + 10,
              }
            : { ...windowItem, zIndex: 10 + index },
        )
      }

      const preferredGeometry =
        scenario === 'normal' ? windowGeometryByAppRef.current[app.id] : undefined
      const normalizedGeometry = normalizeWindowGeometryForViewport(
        app,
        preferredGeometry,
        prev.length,
        desktopViewportBounds,
      )

      return [...prev, createWindowRecord(app, prev.length, normalizedGeometry)]
    })
    logActivity(
      t('activity.opened', 'Opened {{name}}', { name: t(app.labelKey, app.id) }),
    )
  }

  const handleCloseWindow = (windowId: string) => {
    const closing = windows.find((windowItem) => windowItem.id === windowId)
    if (closing) {
      const app = findDesktopAppById(apps, closing.appId)
      persistWindowGeometry(closing.appId, {
        x: closing.x,
        y: closing.y,
        width: closing.width,
        height: closing.height,
      })
      logActivity(
        t('activity.closed', 'Closed {{name}}', {
          name: t(app?.labelKey ?? closing.titleKey),
        }),
      )
    }

    setWindows((prev) => prev.filter((windowItem) => windowItem.id !== windowId))
  }

  const updateWindowGeometry = (
    windowId: string,
    geometry: Partial<WindowGeometry>,
  ) => {
    setWindows((prev) =>
      prev.map((windowItem) => {
        if (windowItem.id !== windowId) {
          return windowItem
        }

        const nextWindow = { ...windowItem, ...geometry }
        persistWindowGeometry(windowItem.appId, {
          x: nextWindow.x,
          y: nextWindow.y,
          width: nextWindow.width,
          height: nextWindow.height,
        })
        return nextWindow
      }),
    )
  }

  const focusWindow = (windowId: string) => {
    setWindows((prev) => {
      const top = prev.length + 12
      return prev.map((windowItem, index) =>
        windowItem.id === windowId
          ? { ...windowItem, zIndex: top }
          : { ...windowItem, zIndex: 10 + index },
      )
    })
  }

  const toggleMaximizeWindow = (windowId: string) => {
    const target = windows.find((windowItem) => windowItem.id === windowId)
    if (!target) {
      return
    }

    const app = findDesktopAppById(apps, target.appId)
    logActivity(
      t('activity.maximized', 'Toggled maximize for {{name}}', {
        name: t(app?.labelKey ?? target.titleKey),
      }),
    )
    setWindows((prev) =>
      prev.map((windowItem) =>
        windowItem.id === windowId
          ? {
              ...windowItem,
              state:
                windowItem.state === 'maximized' ? 'windowed' : 'maximized',
            }
          : windowItem,
      ),
    )
  }

  const minimizeWindow = (windowId: string) => {
    const target = windows.find((windowItem) => windowItem.id === windowId)
    if (!target) {
      return
    }

    const app = findDesktopAppById(apps, target.appId)
    logActivity(
      t('activity.minimized', 'Minimized {{name}}', {
        name: t(app?.labelKey ?? target.titleKey),
      }),
    )

    const minimizedOrder = nextMinimizedOrderRef.current++

    setWindows((prev) =>
      prev.map((windowItem) =>
        windowItem.id === windowId
          ? { ...windowItem, state: 'minimized', minimizedOrder }
          : windowItem,
      ),
    )
  }

  // Layout changes are driven by drag handlers and resolveLayout;
  // we intentionally skip the react-grid-layout onLayoutChange callback
  // so that auto-positioned items keep their undefined stored position.
  const handleLayoutChange = (_pageId: string, _nextLayout: Layout) => {}

  const suppressNextOpen = (itemId: string) => {
    suppressOpenItemId.current = itemId
    window.setTimeout(() => {
      if (suppressOpenItemId.current === itemId) {
        suppressOpenItemId.current = null
      }
    }, 180)
  }

  const blockOpenAfterDrag = (itemId: string) => {
    draggedOpenBlockItemId.current = itemId
    if (draggedOpenBlockTimeoutId.current) {
      window.clearTimeout(draggedOpenBlockTimeoutId.current)
    }
    draggedOpenBlockTimeoutId.current = window.setTimeout(() => {
      if (draggedOpenBlockItemId.current === itemId) {
        draggedOpenBlockItemId.current = null
      }
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
    if (!itemId) {
      return
    }

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

    if (!newItem) {
      return
    }

    const positionChanged =
      !oldItem ||
      oldItem.x !== newItem.x ||
      oldItem.y !== newItem.y ||
      oldItem.w !== newItem.w ||
      oldItem.h !== newItem.h

    if (!positionChanged) {
      return
    }

    // Give the dragged item an explicit position.
    // Any explicitly-positioned item it overlaps becomes unpositioned
    // and will be auto-placed at the page tail by resolveLayout.
    setLayoutState((prev) => {
      if (!prev) {
        return prev
      }

      return {
        ...prev,
        pages: prev.pages.map((page) => {
          if (page.id !== pageId) return page
          return {
            ...page,
            items: page.items.map((item) => {
              if (item.id === newItem.i) {
                return { ...item, x: newItem.x, y: newItem.y }
              }
              // Displace overlapping explicitly-positioned items
              if (item.x !== undefined && item.y !== undefined) {
                const overlaps = !(
                  newItem.x + newItem.w <= item.x ||
                  item.x + item.w <= newItem.x ||
                  newItem.y + newItem.h <= item.y ||
                  item.y + item.h <= newItem.y
                )
                if (overlaps) {
                  return { ...item, x: undefined, y: undefined }
                }
              }
              return item
            }),
          }
        }),
      }
    })
  }

  const moveItemBetweenPages = (itemId: string, direction: -1 | 1) => {
    if (!resolvedLayout) return

    // Use the resolved layout to determine which page the user sees the item on
    const resolvedPageIndex = getPageIndex(resolvedLayout, itemId)
    if (resolvedPageIndex < 0) return

    setLayoutState((prev) => {
      if (!prev) {
        return prev
      }

      // Find and remove from current page in layoutState
      const currentPageIndex = getPageIndex(prev, itemId)
      if (currentPageIndex < 0) return prev

      const item = prev.pages[currentPageIndex].items.find((entry) => entry.id === itemId)
      if (!item) return prev

      const targetPageIndex = resolvedPageIndex + direction
      if (targetPageIndex < 0) return prev

      const nextPages = prev.pages.map((page) => ({
        ...page,
        items: [...page.items],
      }))

      if (targetPageIndex >= nextPages.length) {
        nextPages.push({
          id: `${formFactor}-page-${nextPages.length + 1}`,
          items: [],
        })
      }

      nextPages[currentPageIndex].items = nextPages[currentPageIndex].items.filter(
        (entry) => entry.id !== itemId,
      )
      // Add to target page with undefined position (auto-placed at tail)
      nextPages[targetPageIndex].items.push({ ...item, x: undefined, y: undefined })

      return {
        ...prev,
        pages: nextPages,
      }
    })
    setContextMenu(null)
  }

  const updateWidgetNote = (itemId: string, content: string) => {
    setLayoutState((prev) => {
      if (!prev) {
        return prev
      }

      return {
        ...prev,
        pages: prev.pages.map((page) => ({
          ...page,
          items: page.items.map((item) =>
            item.id === itemId && item.type === 'widget'
              ? {
                  ...item,
                  config: {
                    ...item.config,
                    content,
                  },
                }
              : item,
          ),
        })),
      }
    })
  }

  const applySettings = (values: SystemPreferencesInput) => {
    const nextDeadZone = {
      top: values.deadZoneTop,
      bottom: values.deadZoneBottom,
      left: values.deadZoneLeft,
      right: values.deadZoneRight,
    }
    const nextWindowAppearance = {
      titleBarOpacity: values.titleBarOpacity,
      backgroundOpacity: values.backgroundOpacity,
    }

    setLocale(values.locale)
    setThemeMode(values.theme as ThemeMode)
    setRuntimeContainer(values.runtimeContainer)
    setWindowAppearance(nextWindowAppearance)
    setLayoutState((prev) => {
      if (!prev) {
        return prev
      }

      return {
        ...prev,
        deadZone: nextDeadZone,
      }
    })
    if (formFactor === 'desktop') {
      normalizeOpenWindowsForViewport(viewportSize, nextDeadZone)
    }

    logActivity(t('activity.saved'))
    setSnackbar(t('activity.saved'))
  }

  const windowLayerModel = useMemo(
    () => createDesktopWindowLayerDataModel(apps, windows),
    [apps, windows],
  )
  const topMobileWindow = windowLayerModel.topWindow
  const activeMobileApp =
    formFactor === 'mobile' && topMobileWindow
      ? topMobileWindow.app
      : undefined
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
    workspaceSize.height - workspaceTopPadding - resolvedDeadZone.bottom - safeArea.bottom,
    360,
  )
  const shouldLockDesktopViewport =
    formFactor === 'desktop' &&
    viewportSize.width >= desktopMinCanvasSize.width &&
    viewportSize.height >= desktopMinCanvasSize.height
  const trayState = useMemo<StatusTrayState>(
    () => {
      const statusTips: StatusTip[] = [
        {
          id: 'recent-shell-action',
          tone: 'success',
          taskLabel: t('tips.task.shell'),
          title: t('tips.card.recent.title'),
          body: activityLog[0] ?? t('tips.card.recent.body'),
          statusLabel: t('tips.status.completed'),
          timeLabel: t('tips.time.justNow'),
        },
        {
          id: 'mobile-touch-audit',
          tone: 'error',
          taskLabel: t('tips.task.mobile'),
          title: t('tips.card.touch.title'),
          body: t('tips.card.touch.body'),
          statusLabel: t('tips.status.failed'),
          timeLabel: t('tips.time.twoMinutes'),
        },
        {
          id: 'diagnostics-export',
          tone: 'progress',
          taskLabel: t('tips.task.report'),
          title: t('tips.card.report.title'),
          body: t('tips.card.report.body'),
          statusLabel: t('tips.status.running'),
          timeLabel: t('tips.time.queue'),
        },
        {
          id: 'runtime-cache-warmed',
          tone: 'success',
          taskLabel: t('tips.task.runtime'),
          title: t('tips.card.cache.title'),
          body: t('tips.card.cache.body'),
          statusLabel: t('tips.status.completed'),
          timeLabel: '5m',
        },
        {
          id: 'notes-sync-retry',
          tone: 'error',
          taskLabel: t('tips.task.sync'),
          title: t('tips.card.sync.title'),
          body: t('tips.card.sync.body'),
          statusLabel: t('tips.status.failed'),
          timeLabel: '9m',
        },
        {
          id: 'docs-index-refresh',
          tone: 'progress',
          taskLabel: t('tips.task.index'),
          title: t('tips.card.index.title'),
          body: t('tips.card.index.body'),
          statusLabel: t('tips.status.running'),
          timeLabel: '12m',
        },
        {
          id: 'launcher-metrics-pushed',
          tone: 'success',
          taskLabel: t('tips.task.metrics'),
          title: t('tips.card.metrics.title'),
          body: t('tips.card.metrics.body'),
          statusLabel: t('tips.status.completed'),
          timeLabel: '18m',
        },
        {
          id: 'widget-layout-reflow',
          tone: 'progress',
          taskLabel: t('tips.task.layout'),
          title: t('tips.card.layout.title'),
          body: t('tips.card.layout.body'),
          statusLabel: t('tips.status.running'),
          timeLabel: '24m',
        },
      ]

      return {
        backupActive: windows.some(
          (windowItem) =>
            windowItem.appId === 'files' && windowItem.state !== 'minimized',
        ),
        messageCount: Math.min(
          windows.filter((windowItem) => windowItem.state !== 'minimized').length,
          3,
        ),
        notificationCount: Math.min(statusTips.length, 9),
        tips: statusTips,
      }
    },
    [activityLog, t, windows],
  )

  const toggleSidebar = () => setIsSystemSidebarOpen((prev) => !prev)
  const closeSidebar = () => setIsSystemSidebarOpen(false)
  const handleReturnDesktop = () => {
    setWindows((prev) => {
      const visibleWindowIds = [...prev]
        .filter((windowItem) => windowItem.state !== 'minimized')
        .sort((a, b) => a.zIndex - b.zIndex)
        .map((windowItem) => windowItem.id)
      const minimizedOrderMap = new Map(
        visibleWindowIds.map((windowId, index) => [
          windowId,
          nextMinimizedOrderRef.current + index,
        ]),
      )

      nextMinimizedOrderRef.current += visibleWindowIds.length

      return prev.map((windowItem) =>
        windowItem.state === 'minimized'
          ? windowItem
          : {
              ...windowItem,
              state: 'minimized',
              minimizedOrder: minimizedOrderMap.get(windowItem.id) ?? null,
            },
      )
    })
    closeSidebar()
  }
  const handleSelectSidebarApp = (appId: string) => {
    handleOpenApp(appId)
    closeSidebar()
  }
  const handleCycleLocale = () => setLocale(nextSupportedLocale(locale))
  const handleToggleTheme = () =>
    setThemeMode(themeMode === 'light' ? 'dark' : 'light')
  const systemSidebarModel = createSystemSidebarDataModel(
    apps,
    windows,
    activeMobileApp?.id,
  )

  const backgroundWallpaper = useMemo(
    () => data?.wallpaper ?? { mode: 'infinite' as const },
    [data?.wallpaper],
  )
  const backgroundPageCount = resolvedLayout?.pages.length ?? data?.layout.pages.length ?? 1

  useEffect(() => {
    setBackground({
      wallpaper: backgroundWallpaper,
      pageCount: backgroundPageCount,
      viewportProgress,
    })
  }, [
    backgroundPageCount,
    backgroundWallpaper,
    setBackground,
    viewportProgress,
  ])

  useEffect(() => resetBackground, [resetBackground])

  return (
    <main
      className="relative isolate bg-[color:var(--cp-bg)]"
      style={
        shouldLockDesktopViewport
          ? {
              height: '100dvh',
              overflow: 'hidden',
            }
          : {
              minHeight: '100dvh',
            }
      }
    >
      <section
        className="relative z-10"
        style={
          shouldLockDesktopViewport
            ? {
                height: '100%',
                overflow: 'hidden',
              }
            : {
                minHeight: '100dvh',
              }
        }
      >
        <div
          ref={workspaceRef}
          className="relative h-dvh min-h-dvh"
        >
          {!isLoading && !error && resolvedLayout ? (
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
                      setViewportProgress(
                        normalizeViewportProgress(swiper.progress, resolvedLayout.pages.length),
                      )
                    }
                    onProgress={(swiper) =>
                      setViewportProgress(
                        normalizeViewportProgress(swiper.progress, resolvedLayout.pages.length),
                      )
                    }
                    onSlideChange={(swiper) =>
                      setViewportProgress(
                        normalizeViewportProgress(swiper.progress, resolvedLayout.pages.length),
                      )
                    }
                  >
                    {resolvedLayout.pages.map((page) => (
                      <SwiperSlide key={page.id} className="h-full">
                        <div className="h-full px-4 pb-16 pt-6 sm:px-6">
                          <GridLayoutBase
                            className="layout h-full"
                            gridConfig={{
                              cols: currentSpec.cols,
                              rowHeight: currentSpec.rowHeight,
                              margin: [GRID_GAP, GRID_GAP],
                              containerPadding: [0, 0],
                              maxRows: currentSpec.rows,
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
                                      : setSelectedItemId(item.id)
                                  }
                                  onOpenContextMenu={(event) => {
                                    event.preventDefault()
                                    setSelectedItemId(item.id)
                                    setContextMenu({
                                      itemId: item.id,
                                      mouseX: event.clientX + 2,
                                      mouseY: event.clientY - 6,
                                    })
                                  }}
                                  onSaveNote={updateWidgetNote}
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
          {error ? <ErrorState onRetry={() => void mutate()} /> : null}
        </div>
      </section>

      <Menu
        open={Boolean(contextMenu)}
        onClose={() => setContextMenu(null)}
        anchorReference="anchorPosition"
        anchorPosition={
          contextMenu
            ? { top: contextMenu.mouseY, left: contextMenu.mouseX }
            : undefined
        }
      >
        <MenuItem
          onClick={() => {
            const pageIndex = contextMenu && resolvedLayout ? getPageIndex(resolvedLayout, contextMenu.itemId) : -1
            const item = pageIndex >= 0 ? resolvedLayout?.pages[pageIndex].items.find((entry) => entry.id === contextMenu?.itemId) : null
            if (item?.type === 'app') {
              handleOpenApp(item.appId)
            }
            setContextMenu(null)
          }}
        >
          {t('common.open')}
        </MenuItem>
        <MenuItem
          disabled={!contextMenu || !resolvedLayout || getPageIndex(resolvedLayout, contextMenu.itemId) <= 0}
          onClick={() => contextMenu && moveItemBetweenPages(contextMenu.itemId, -1)}
        >
          {t('common.movePrev')}
        </MenuItem>
        <MenuItem
          disabled={!contextMenu || !resolvedLayout}
          onClick={() => contextMenu && moveItemBetweenPages(contextMenu.itemId, 1)}
        >
          {t('common.moveNext')}
        </MenuItem>
      </Menu>

      <Alert
        severity="success"
        onClose={() => setSnackbar(null)}
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
        <p className="mt-2 font-display text-2xl font-semibold">{t('states.errorTitle')}</p>
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
        <p className="mt-2 font-display text-2xl font-semibold">{t('states.emptyTitle')}</p>
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
  const touchStartRef = useRef<{ x: number; y: number; pointerId: number } | null>(null)
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
    if (isDesktop || pointerType === 'mouse') {
      return
    }

    const start = touchStartRef.current
    clearAppPointer()

    if (!start || start.pointerId !== pointerId) {
      return
    }

    const distance = Math.hypot(clientX - start.x, clientY - start.y)
    if (distance <= 12) {
      onOpen()
    }
  }

  const handleAppPointerDown = (event: ReactPointerEvent<HTMLButtonElement>) => {
    if (isDesktop || event.pointerType === 'mouse') {
      return
    }

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

  const handleAppPointerUp = (event: ReactPointerEvent<HTMLButtonElement>) => {
    completeAppPointer(event.pointerId, event.clientX, event.clientY, event.pointerType)
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
