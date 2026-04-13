/**
 * DesktopUIDataModel -- unified Desktop UI data store.
 *
 * Data is organised into two tiers:
 *
 * ┌─────────────────────────────────────────────────────────┐
 * │  需要同步的数据 (syncData)                                │
 * │  ─────────────────────────────────────────────────────  │
 * │  1. 外观设置      (appearance)                           │
 * │  2. 窗口布局设置   (windowLayout)                         │
 * │  3. AppItem 配置  (appItemConfig)                        │
 * │  4. AppItem 布局  (appItemLayout)                        │
 * │  5. 小部件布局配置 (widgetLayout)                          │
 * ├─────────────────────────────────────────────────────────┤
 * │  本地运行时瞬态 (runtime) — 不持久化、不同步                  │
 * │  windows / snackbar / contextMenu / grid spec / ...     │
 * └─────────────────────────────────────────────────────────┘
 *
 * Initialisation:
 *   调用 init(formFactor, scenario?) 即可，内部通过 isMockRuntime()
 *   (来�� runtime.ts) 自动判断走 mock 路径还是真实 API 路径。
 *   - MockRuntime (pnpm run dev / VITE_CP_USE_MOCK=true)  → initByMock
 *   - 正式环境                                              → initByReal
 */
import { createContext, useContext, useSyncExternalStore } from 'react'
import { resolveDesktopApps, findDesktopAppById } from '../app/registry'
import type { DesktopAppItem } from '../app/types'
import {
  getDesktopWindowPositionBounds,
  getDesktopWindowWorkspaceBounds,
} from '../desktop/windows/geometry'
import {
  createDesktopWindowLayerDataModel,
  createWindowRecord,
  resolveDesktopWindowSizing,
} from '../desktop/windows/model'
import { shellStatusBarHeight } from '../desktop/shell'
import { defaultDeadZone } from '../mock/data'
import { fetchDesktopPayload } from '../mock/provider'
import { isMockRuntime } from '../runtime'
import type {
  AppDefinition,
  AppearanceSettings,
  AppIconItem,
  AppItemLayoutSettings,
  DeadZone,
  DesktopPageState,
  DesktopPayload,
  DesktopSyncData,
  FormFactor,
  LayoutState,
  MockScenario,
  PlacementType,
  RuntimeContainer,
  SupportedLocale,
  SystemPreferencesInput,
  SystemSidebarAppItem,
  SystemSidebarDataModel,
  ThemeMode,
  WidgetItem,
  WidgetLayoutSettings,
  WindowAppearancePreferences,
  WindowGeometry,
  WindowLayoutSettings,
  WindowRecord,
} from './ui'
import {
  clamp,
  coordToSlot,
  findNearestEmptySlot,
  invalidatePositions,
  isPageFull,
  layoutStorageKey,
  migrateDeadZone,
  migrateToSlotModel,
  readJson,
  readWindowAppearancePreferences,
  reconcileLayoutWithDefaultApps,
  reorderWithinPage,
  resolveLayout,
  runtimeStorageKey,
  sameWindowGeometry,
  sanitizeWindowGeometryMap,
  windowAppearanceStorageKey,
  windowGeometryStorageKey,
  writeJson,
  type ScanOrder,
  type WindowGeometryMap,
} from './layout'

// ---------------------------------------------------------------------------
// Snapshot shape
// ---------------------------------------------------------------------------

/**
 * 本地运行时瞬态 — 不需要持久化/同步，仅存在于当前会话。
 */
export interface DesktopRuntimeState {
  /** 当前打开的窗口列表（运行中实例） */
  windows: WindowRecord[]
  /** 活动日志 */
  activityLog: string[]
  /** Snackbar 提示信息 */
  snackbar: string | null
  /** 系统侧边栏是否展开 */
  isSystemSidebarOpen: boolean
  /** 当前选中的桌面项目 */
  selectedItemId: string | null
  /** 桌面翻页进度 (0~1) */
  viewportProgress: number
  /** 右键菜单状态 */
  contextMenu: ContextMenuState | null
  /** 当前网格列数（由容器宽度计算） */
  gridCols: number
  /** 当前网格行数（由容器高度计算） */
  gridRows: number
  /** 当前网格行高（由容器高度计算） */
  gridRowHeight: number
}

export interface ContextMenuState {
  itemId: string
  mouseX: number
  mouseY: number
}

/**
 * DesktopUISnapshot — 完整的 UI 状态快照。
 *
 * 字段按数据性质分为三层：
 *  - 加载状态 (status/error/formFactor/scenario)
 *  - 需要同步的 5 个数据分组 (syncData)
 *  - 本地运行时瞬态 (runtime)
 *
 * 同时保留展平的便捷访问器（见底部 selector hooks）。
 */
export interface DesktopUISnapshot {
  // ── 加载 / 环境 ──
  status: 'idle' | 'loading' | 'success' | 'error'
  error: string | null
  formFactor: FormFactor
  scenario: MockScenario

  // ── 需要同步的数据（5 个分组） ──
  syncData: DesktopSyncData

  // ── 合并后的 LayoutState（由 Group 4 + 5 组合而成） ──
  layoutState: LayoutState | null
  /** 经过自动分配位置后的完整布局（derived） */
  resolvedLayout: LayoutState | null

  // ── 解析后的 AppItem 列表（带 loader，derived from Group 3） ──
  apps: DesktopAppItem[]

  // ── 本地运行时瞬态 ──
  runtime: DesktopRuntimeState
}

// ---------------------------------------------------------------------------
// Helpers: LayoutState ↔ Group 4 + Group 5
// ---------------------------------------------------------------------------

/**
 * 从 LayoutState 中拆分出 AppItem 布局（Group 4）。
 */
function extractAppItemLayout(layout: LayoutState): AppItemLayoutSettings {
  return {
    pages: layout.pages.map((page) => ({
      pageId: page.id,
      items: page.items.filter((item): item is AppIconItem => item.type === 'app'),
    })),
  }
}

/**
 * 从 LayoutState 中拆分出小部件布局（Group 5）。
 */
function extractWidgetLayout(layout: LayoutState): WidgetLayoutSettings {
  return {
    pages: layout.pages.map((page) => ({
      pageId: page.id,
      items: page.items.filter((item): item is WidgetItem => item.type === 'widget'),
    })),
  }
}

/**
 * 将 Group 4 + Group 5 合并回 LayoutState。
 */
export function mergeToLayoutState(
  formFactor: FormFactor,
  deadZone: DeadZone,
  appItemLayout: AppItemLayoutSettings,
  widgetLayout: WidgetLayoutSettings,
): LayoutState {
  // Collect all page ids in order (union of both groups)
  const pageIdOrder: string[] = []
  const seen = new Set<string>()
  for (const page of [...appItemLayout.pages, ...widgetLayout.pages]) {
    if (!seen.has(page.pageId)) {
      seen.add(page.pageId)
      pageIdOrder.push(page.pageId)
    }
  }

  const appByPage = new Map(appItemLayout.pages.map((p) => [p.pageId, p.items]))
  const widgetByPage = new Map(widgetLayout.pages.map((p) => [p.pageId, p.items]))

  return {
    version: 1,
    formFactor,
    deadZone,
    pages: pageIdOrder.map((pageId) => ({
      id: pageId,
      items: [
        ...(widgetByPage.get(pageId) ?? []),
        ...(appByPage.get(pageId) ?? []),
      ],
    })),
  }
}

/**
 * Build the full syncData from current state.
 */
function buildSyncData(
  appearance: AppearanceSettings,
  windowLayout: WindowLayoutSettings,
  apps: AppDefinition[],
  layout: LayoutState | null,
): DesktopSyncData {
  const emptyAppLayout: AppItemLayoutSettings = { pages: [] }
  const emptyWidgetLayout: WidgetLayoutSettings = { pages: [] }

  return {
    appearance,
    windowLayout,
    appItemConfig: { apps },
    appItemLayout: layout ? extractAppItemLayout(layout) : emptyAppLayout,
    widgetLayout: layout ? extractWidgetLayout(layout) : emptyWidgetLayout,
  }
}

// ---------------------------------------------------------------------------
// Store class
// ---------------------------------------------------------------------------

export class DesktopUIStore {
  // ---- listener protocol --------------------------------------------------
  private listeners = new Set<() => void>()
  private snapshot: DesktopUISnapshot

  // ---- internal mutable refs ----------------------------------------------
  private nextMinimizedOrder = 1
  private windowGeometryByApp: WindowGeometryMap = {}
  private defaultPayload: DesktopPayload | null = null

  /**
   * Resize reflow debounce timer.
   * Reflow (invalidation + tail-append) only happens after resize stops,
   * not on every frame. During resize we only do "pure resize" (geometry update).
   */
  private resizeReflowTimer: ReturnType<typeof setTimeout> | null = null
  private static readonly RESIZE_REFLOW_DELAY = 200

  constructor() {
    const runtimeContainer =
      (window.localStorage.getItem(runtimeStorageKey) as RuntimeContainer | null) ?? 'browser'
    const windowAppearance = readWindowAppearancePreferences()

    const appearance: AppearanceSettings = {
      runtimeContainer,
      wallpaper: { mode: 'infinite' },
      deadZone: { ...defaultDeadZone },
      windowAppearance,
    }

    const windowLayout: WindowLayoutSettings = {
      geometryByApp: {},
    }

    this.snapshot = {
      status: 'idle',
      error: null,
      formFactor: 'desktop',
      scenario: 'normal',

      syncData: {
        appearance,
        windowLayout,
        appItemConfig: { apps: [] },
        appItemLayout: { pages: [] },
        widgetLayout: { pages: [] },
      },

      layoutState: null,
      resolvedLayout: null,
      apps: [],

      runtime: {
        windows: [],
        activityLog: [],
        snackbar: null,
        isSystemSidebarOpen: false,
        selectedItemId: null,
        viewportProgress: 0,
        contextMenu: null,
        gridCols: 10,
        gridRows: 8,
        gridRowHeight: 78,
      },
    }

    this.windowGeometryByApp = sanitizeWindowGeometryMap(
      readJson(windowGeometryStorageKey),
    )
  }

  // ---- useSyncExternalStore protocol --------------------------------------

  subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => {
      this.listeners.delete(listener)
    }
  }

  getSnapshot = () => this.snapshot

  private notify() {
    this.snapshot = { ...this.snapshot }
    for (const listener of this.listeners) listener()
  }

  /**
   * Low-level update: accepts partial top-level fields + partial runtime.
   */
  private update(
    partial: Partial<Pick<DesktopUISnapshot, 'status' | 'error' | 'formFactor' | 'scenario' | 'layoutState' | 'apps'>> & {
      runtime?: Partial<DesktopRuntimeState>
      appearance?: Partial<AppearanceSettings>
    },
  ) {
    // Merge top-level scalars
    if (partial.status !== undefined) this.snapshot.status = partial.status
    if (partial.error !== undefined) this.snapshot.error = partial.error
    if (partial.formFactor !== undefined) this.snapshot.formFactor = partial.formFactor
    if (partial.scenario !== undefined) this.snapshot.scenario = partial.scenario
    if (partial.apps !== undefined) this.snapshot.apps = partial.apps
    if (partial.layoutState !== undefined) this.snapshot.layoutState = partial.layoutState

    // Merge runtime
    if (partial.runtime) {
      this.snapshot.runtime = { ...this.snapshot.runtime, ...partial.runtime }
    }

    // Merge appearance
    if (partial.appearance) {
      this.snapshot.syncData = {
        ...this.snapshot.syncData,
        appearance: { ...this.snapshot.syncData.appearance, ...partial.appearance },
      }
    }

    // Recompute derived data
    this.recomputeResolvedLayout()
    this.rebuildSyncData()
    this.notify()
  }

  // ---- Derived data recomputation -----------------------------------------

  private recomputeResolvedLayout() {
    const { layoutState, formFactor } = this.snapshot
    const { gridCols, gridRows } = this.snapshot.runtime
    if (!layoutState) {
      this.snapshot.resolvedLayout = null
      return
    }
    const scanOrder: ScanOrder = formFactor === 'mobile' ? 'row-major' : 'col-major'
    this.snapshot.resolvedLayout = resolveLayout(layoutState, gridCols, gridRows, scanOrder)
  }

  private rebuildSyncData() {
    const { layoutState } = this.snapshot
    const { appearance } = this.snapshot.syncData
    const appDefs = this.snapshot.syncData.appItemConfig.apps

    this.snapshot.syncData = buildSyncData(
      appearance,
      { geometryByApp: { ...this.windowGeometryByApp } },
      appDefs,
      layoutState,
    )
  }

  // ========================================================================
  // Initialisation
  // ========================================================================

  /**
   * 统一入口 — 通过 `isMockRuntime()` 自动判断初始化路径。
   *
   * - MockRuntime (`pnpm run dev`)  → 走 mock 数据
   * - 正式环境                       → 走真实 API（当前 stub）
   */
  async init(formFactor: FormFactor, scenario: MockScenario = 'normal') {
    if (isMockRuntime()) {
      await this.initByMock(formFactor, scenario)
    } else {
      await this.initByReal(formFactor)
    }
  }

  /**
   * Re-init when form factor changes (e.g. responsive breakpoint).
   */
  async switchFormFactor(formFactor: FormFactor) {
    if (formFactor === this.snapshot.formFactor) return
    await this.init(formFactor, this.snapshot.scenario)
  }

  /**
   * Mock 初��化 — 使用 mock/provider 获取模拟数据。
   */
  private async initByMock(formFactor: FormFactor, scenario: MockScenario) {
    this.update({
      status: 'loading',
      error: null,
      formFactor,
      scenario,
      runtime: {
        windows: [],
        isSystemSidebarOpen: false,
        viewportProgress: 0,
      },
    })

    try {
      const payload = await fetchDesktopPayload({ formFactor, scenario })
      this.defaultPayload = payload
      const apps = resolveDesktopApps(payload.apps, formFactor)
      let layoutState: LayoutState

      if (scenario === 'normal') {
        const stored = readJson<LayoutState>(layoutStorageKey(formFactor))
        layoutState = stored
          ? migrateDeadZone(stored, formFactor)
          : payload.layout
      } else {
        layoutState = payload.layout
      }

      layoutState = reconcileLayoutWithDefaultApps(
        layoutState,
        payload.layout,
        apps,
        formFactor,
      )

      // Migrate legacy layouts to slot-based model
      const { gridCols, gridRows } = this.snapshot.runtime
      layoutState = migrateToSlotModel(layoutState, gridCols, gridRows)

      // Validate positions against current grid (init-time reflow)
      layoutState = invalidatePositions(layoutState, gridCols, gridRows)

      // Populate syncData from payload
      this.snapshot.syncData.appItemConfig = { apps: payload.apps }

      this.update({
        status: 'success',
        apps,
        layoutState,
        appearance: {
          wallpaper: payload.wallpaper,
          deadZone: layoutState.deadZone,
        },
      })
    } catch (err) {
      this.update({
        status: 'error',
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  /**
   * 真实初始化 — 生产环境从后端 API 获取数据。
   * TODO: 对接真实后端后实现。
   */
  private async initByReal(formFactor: FormFactor) {
    // 暂时回退到 mock 数据，后续替换为真实 API 调用
    await this.initByMock(formFactor, 'normal')
  }

  // ========================================================================
  // Grid spec (driven by container resize observer in the view)
  // ========================================================================

  /**
   * Update grid geometry.
   *
   * **Pure resize** (during resize): only update cols/rows/rowHeight so
   * the grid re-renders at the new size. No invalidation or reflow.
   *
   * **Reflow resize** (after resize settles): debounced — invalidates
   * out-of-bounds items and runs tail-append placement, then persists.
   */
  setGridSpec(cols: number, rows: number, rowHeight: number) {
    const { runtime } = this.snapshot
    if (
      cols === runtime.gridCols &&
      rows === runtime.gridRows &&
      rowHeight === runtime.gridRowHeight
    ) {
      return
    }

    // Phase 1: pure resize — only update geometry, no invalidation
    this.update({
      runtime: { gridCols: cols, gridRows: rows, gridRowHeight: rowHeight },
    })

    // Phase 2: debounced reflow — runs after resize stops
    if (this.resizeReflowTimer) {
      clearTimeout(this.resizeReflowTimer)
    }
    this.resizeReflowTimer = setTimeout(() => {
      this.resizeReflowTimer = null
      this.executeReflow(cols, rows)
    }, DesktopUIStore.RESIZE_REFLOW_DELAY)
  }

  /**
   * Execute a reflow: invalidate out-of-bounds items, then recompute layout.
   * Called after resize settles (debounced) or on init.
   */
  private executeReflow(cols: number, rows: number) {
    let { layoutState } = this.snapshot
    if (!layoutState) return

    const before = layoutState
    layoutState = invalidatePositions(layoutState, cols, rows)

    // Migrate legacy layouts to slot model
    layoutState = migrateToSlotModel(layoutState, cols, rows)

    if (layoutState !== before) {
      this.update({ layoutState })
      this.persistLayout()
    }
  }

  // ========================================================================
  // Window actions (操作运行时窗口 — 不属于同步数据)
  // ========================================================================

  openApp(
    appId: string,
    opts: {
      isMobile: boolean
      navigate?: (path: string) => void
      logActivity?: (msg: string) => void
      viewportBounds: ReturnType<typeof getDesktopWindowWorkspaceBounds>
    },
  ) {
    const app = findDesktopAppById(this.snapshot.apps, appId)
    if (!app) return

    if (opts.isMobile && app.manifest.mobileRedirectPath) {
      opts.navigate?.(app.manifest.mobileRedirectPath)
      return
    }

    if (app.manifest.placement === 'new-container' || app.tier === 'external') {
      opts.logActivity?.(`Requested new-container launch for ${appId}`)
      this.update({ runtime: { snackbar: `External app: ${appId}` } })
      return
    }

    const { windows } = this.snapshot.runtime
    const { scenario } = this.snapshot
    const existing = windows.find((w) => w.appId === appId)

    if (existing) {
      const nextWindows = windows.map((w, i) =>
        w.id === existing.id
          ? {
              ...w,
              state: (app.manifest.defaultMode === 'windowed'
                ? 'windowed'
                : 'maximized') as WindowRecord['state'],
              minimizedOrder: null,
              zIndex: windows.length + 10,
            }
          : { ...w, zIndex: 10 + i },
      )
      this.update({ runtime: { windows: nextWindows } })
    } else {
      // Group 2: 用保存的窗口布局设置作为参考
      const preferredGeometry =
        scenario === 'normal' ? this.windowGeometryByApp[app.id] : undefined
      const geometry = this.normalizeWindowGeometry(
        app,
        preferredGeometry,
        windows.length,
        opts.viewportBounds,
      )
      const record = createWindowRecord(app, windows.length, geometry)
      this.update({ runtime: { windows: [...windows, record] } })
    }
    opts.logActivity?.(`Opened ${appId}`)
  }

  closeWindow(windowId: string) {
    const closing = this.snapshot.runtime.windows.find((w) => w.id === windowId)
    if (closing) {
      // Group 2: 窗口关闭时记录位置和大小
      this.persistWindowGeometry(closing.appId, {
        x: closing.x,
        y: closing.y,
        width: closing.width,
        height: closing.height,
      })
    }
    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.filter((w) => w.id !== windowId),
      },
    })
  }

  minimizeWindow(windowId: string) {
    const order = this.nextMinimizedOrder++
    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.map((w) =>
          w.id === windowId
            ? { ...w, state: 'minimized' as const, minimizedOrder: order }
            : w,
        ),
      },
    })
  }

  toggleMaximizeWindow(windowId: string) {
    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.map((w) =>
          w.id === windowId
            ? {
                ...w,
                state:
                  w.state === 'maximized'
                    ? ('windowed' as const)
                    : ('maximized' as const),
              }
            : w,
        ),
      },
    })
  }

  focusWindow(windowId: string) {
    const top = this.snapshot.runtime.windows.length + 12
    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.map((w, i) =>
          w.id === windowId
            ? { ...w, zIndex: top }
            : { ...w, zIndex: 10 + i },
        ),
      },
    })
  }

  updateWindowGeometry(
    windowId: string,
    geometry: Partial<WindowGeometry>,
  ) {
    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.map((w) => {
          if (w.id !== windowId) return w
          const next = { ...w, ...geometry }
          // Group 2: 实时更新窗口布局设置
          this.persistWindowGeometry(w.appId, {
            x: next.x,
            y: next.y,
            width: next.width,
            height: next.height,
          })
          return next
        }),
      },
    })
  }

  /**
   * Minimize all visible windows ("return to desktop").
   */
  returnToDesktop() {
    const visibleWindowIds = [...this.snapshot.runtime.windows]
      .filter((w) => w.state !== 'minimized')
      .sort((a, b) => a.zIndex - b.zIndex)
      .map((w) => w.id)

    const minimizedOrderMap = new Map(
      visibleWindowIds.map((id, i) => [id, this.nextMinimizedOrder + i]),
    )
    this.nextMinimizedOrder += visibleWindowIds.length

    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.map((w) =>
          w.state === 'minimized'
            ? w
            : {
                ...w,
                state: 'minimized' as const,
                minimizedOrder: minimizedOrderMap.get(w.id) ?? null,
              },
        ),
        isSystemSidebarOpen: false,
      },
    })
  }

  /**
   * Normalise all open window positions for a new viewport size.
   */
  normalizeOpenWindowsForViewport(
    viewportSize: { width: number; height: number },
    deadZone?: DeadZone,
    safeArea?: { top: number; bottom: number; left: number; right: number },
  ) {
    const resolvedDeadZone =
      deadZone ?? this.snapshot.syncData.appearance.deadZone ?? defaultDeadZone
    const resolvedSafeArea = safeArea ?? { top: 0, bottom: 0, left: 0, right: 0 }
    const bounds = getDesktopWindowWorkspaceBounds({
      deadZone: resolvedDeadZone,
      safeArea: resolvedSafeArea,
      topInset:
        resolvedSafeArea.top +
        resolvedDeadZone.top +
        shellStatusBarHeight('desktop'),
      viewportSize,
    })

    let changed = false
    const nextWindows = this.snapshot.runtime.windows.map((w, i) => {
      const app = findDesktopAppById(this.snapshot.apps, w.appId)
      if (!app) return w
      const geometry = this.normalizeWindowGeometry(app, w, i, bounds)
      if (sameWindowGeometry(w, geometry)) return w
      changed = true
      this.persistWindowGeometry(w.appId, geometry)
      return { ...w, ...geometry }
    })

    if (changed) {
      this.update({ runtime: { windows: nextWindows } })
    }
  }

  // ========================================================================
  // Layout actions (操作 Group 4 + Group 5)
  // ========================================================================

  /**
   * Handle drag-stop with proposal-compliant collision rules:
   *
   * 1. **Empty target slot** → place directly, 100% success
   * 2. **Occupied target, page NOT full** → local collision: bump
   *    the displaced item to the nearest empty slot (Manhattan distance)
   * 3. **Occupied target, page IS full** → same-page reorder:
   *    shift items between source and target to make room
   *
   * Drag logic is intentionally separate from resize reflow.
   */
  handleGridDragStop(
    pageId: string,
    oldItem: { i: string; x: number; y: number; w: number; h: number } | null,
    newItem: { i: string; x: number; y: number; w: number; h: number } | null,
  ) {
    if (!newItem) return
    const positionChanged =
      !oldItem ||
      oldItem.x !== newItem.x ||
      oldItem.y !== newItem.y ||
      oldItem.w !== newItem.w ||
      oldItem.h !== newItem.h

    if (!positionChanged) return

    const { layoutState, formFactor } = this.snapshot
    if (!layoutState) return

    const { gridCols: cols, gridRows: rows } = this.snapshot.runtime
    const order: ScanOrder = formFactor === 'mobile' ? 'row-major' : 'col-major'

    this.update({
      layoutState: {
        ...layoutState,
        pages: layoutState.pages.map((page) => {
          if (page.id !== pageId) return page
          return this.applyDragCollision(page, oldItem, newItem, cols, rows, order)
        }),
      },
    })

    this.persistLayout()
  }

  /**
   * Apply drag collision rules to a single page.
   */
  private applyDragCollision(
    page: DesktopPageState,
    oldItem: { i: string; x: number; y: number; w: number; h: number } | null,
    newItem: { i: string; x: number; y: number; w: number; h: number },
    cols: number,
    rows: number,
    order: ScanOrder,
  ): DesktopPageState {
    const targetSlot = coordToSlot(newItem.x, newItem.y, cols, rows, order)
    const sourceSlot = oldItem
      ? coordToSlot(oldItem.x, oldItem.y, cols, rows, order)
      : undefined

    // Check if target slot is occupied by another item
    const occupant = page.items.find((item) => {
      if (item.id === newItem.i) return false
      if (item.x === undefined || item.y === undefined) return false
      return !(
        newItem.x + newItem.w <= item.x ||
        item.x + item.w <= newItem.x ||
        newItem.y + newItem.h <= item.y ||
        item.y + item.h <= newItem.y
      )
    })

    // Rule 1: empty slot → place directly
    if (!occupant) {
      return {
        ...page,
        items: page.items.map((item) =>
          item.id === newItem.i
            ? {
                ...item,
                x: newItem.x,
                y: newItem.y,
                slotIndex: targetSlot,
                placementType: 'manual' as PlacementType,
              }
            : item,
        ),
      }
    }

    // Rule 3: page is full → same-page reorder (shift items)
    if (sourceSlot !== undefined && isPageFull(page, cols, rows, order)) {
      return reorderWithinPage(page, sourceSlot, targetSlot, cols, rows, order)
    }

    // Rule 2: page not full → local collision (bump displaced to nearest empty)
    const emptySlot = findNearestEmptySlot(
      page,
      occupant.x!,
      occupant.y!,
      cols,
      rows,
      order,
      newItem.i, // exclude dragged item from occupancy check
    )

    return {
      ...page,
      items: page.items.map((item) => {
        if (item.id === newItem.i) {
          return {
            ...item,
            x: newItem.x,
            y: newItem.y,
            slotIndex: targetSlot,
            placementType: 'manual' as PlacementType,
          }
        }
        if (item.id === occupant.id && emptySlot) {
          const bumpSlot = coordToSlot(emptySlot.x, emptySlot.y, cols, rows, order)
          return {
            ...item,
            x: emptySlot.x,
            y: emptySlot.y,
            slotIndex: bumpSlot,
            placementType: 'manual' as PlacementType,
          }
        }
        // If no empty slot found, mark occupant as unpositioned (fallback)
        if (item.id === occupant.id && !emptySlot) {
          return { ...item, x: undefined, y: undefined, slotIndex: undefined }
        }
        return item
      }),
    }
  }

  moveItemBetweenPages(itemId: string, direction: -1 | 1) {
    const { layoutState, resolvedLayout, formFactor } = this.snapshot
    if (!layoutState || !resolvedLayout) return

    const resolvedPageIndex = resolvedLayout.pages.findIndex((page) =>
      page.items.some((item) => item.id === itemId),
    )
    if (resolvedPageIndex < 0) return

    const currentPageIndex = layoutState.pages.findIndex((page) =>
      page.items.some((item) => item.id === itemId),
    )
    if (currentPageIndex < 0) return

    const item = layoutState.pages[currentPageIndex].items.find(
      (entry) => entry.id === itemId,
    )
    if (!item) return

    const targetPageIndex = resolvedPageIndex + direction
    if (targetPageIndex < 0) return

    const nextPages = layoutState.pages.map((page) => ({
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
    nextPages[targetPageIndex].items.push({
      ...item,
      x: undefined,
      y: undefined,
      slotIndex: undefined,
      preferredPage: targetPageIndex,
      placementType: 'manual',
    })

    this.update({
      layoutState: { ...layoutState, pages: nextPages },
      runtime: { contextMenu: null },
    })
    this.persistLayout()
  }

  /** Group 5: 更新小部件配置（笔记内容） */
  updateWidgetNote(itemId: string, content: string) {
    const { layoutState } = this.snapshot
    if (!layoutState) return

    this.update({
      layoutState: {
        ...layoutState,
        pages: layoutState.pages.map((page) => ({
          ...page,
          items: page.items.map((item) =>
            item.id === itemId && item.type === 'widget'
              ? { ...item, config: { ...item.config, content } }
              : item,
          ),
        })),
      },
    })
    this.persistLayout()
  }

  restoreDefaults() {
    if (!this.defaultPayload) return
    const { formFactor } = this.snapshot
    window.localStorage.removeItem(layoutStorageKey(formFactor))
    window.localStorage.removeItem(windowGeometryStorageKey)
    this.windowGeometryByApp = {}

    this.update({
      layoutState: structuredClone(this.defaultPayload.layout),
      runtime: { windows: [] },
    })
  }

  // ========================================================================
  // Preferences / settings (操作 Group 1: 外观设置)
  // ========================================================================

  applySettings(
    values: SystemPreferencesInput,
    callbacks: {
      setLocale: (locale: SupportedLocale) => void
      setThemeMode: (mode: ThemeMode) => void
      viewportSize: { width: number; height: number }
    },
  ) {
    const nextDeadZone: DeadZone = {
      top: values.deadZoneTop,
      bottom: values.deadZoneBottom,
      left: values.deadZoneLeft,
      right: values.deadZoneRight,
    }
    const nextWindowAppearance: WindowAppearancePreferences = {
      titleBarOpacity: values.titleBarOpacity,
      backgroundOpacity: values.backgroundOpacity,
    }

    // locale & theme are managed by their own Providers
    callbacks.setLocale(values.locale)
    callbacks.setThemeMode(values.theme as ThemeMode)

    window.localStorage.setItem(runtimeStorageKey, values.runtimeContainer)
    writeJson(windowAppearanceStorageKey, nextWindowAppearance)

    const { layoutState } = this.snapshot
    this.update({
      layoutState: layoutState
        ? { ...layoutState, deadZone: nextDeadZone }
        : layoutState,
      appearance: {
        runtimeContainer: values.runtimeContainer as RuntimeContainer,
        windowAppearance: nextWindowAppearance,
        deadZone: nextDeadZone,
      },
      runtime: { snackbar: 'Settings saved' },
    })

    if (this.snapshot.formFactor === 'desktop') {
      this.normalizeOpenWindowsForViewport(callbacks.viewportSize, nextDeadZone)
    }
    this.persistLayout()
  }

  // ========================================================================
  // Simple setters for transient UI state (runtime)
  // ========================================================================

  setSnackbar(msg: string | null) {
    this.update({ runtime: { snackbar: msg } })
  }

  setSelectedItemId(id: string | null) {
    this.update({ runtime: { selectedItemId: id } })
  }

  setContextMenu(menu: ContextMenuState | null) {
    this.update({ runtime: { contextMenu: menu } })
  }

  setViewportProgress(progress: number) {
    if (progress === this.snapshot.runtime.viewportProgress) return
    this.update({ runtime: { viewportProgress: progress } })
  }

  toggleSystemSidebar() {
    this.update({
      runtime: { isSystemSidebarOpen: !this.snapshot.runtime.isSystemSidebarOpen },
    })
  }

  closeSystemSidebar() {
    this.update({ runtime: { isSystemSidebarOpen: false } })
  }

  logActivity(message: string, locale: string) {
    const stamp = new Intl.DateTimeFormat(locale, {
      hour: '2-digit',
      minute: '2-digit',
      second: '2-digit',
    }).format(new Date())

    this.update({
      runtime: {
        activityLog: [
          `${stamp} · ${message}`,
          ...this.snapshot.runtime.activityLog,
        ].slice(0, 8),
      },
    })
  }

  // ========================================================================
  // Derived data (computed from snapshot, no state change)
  // ========================================================================

  getWindowLayerModel() {
    return createDesktopWindowLayerDataModel(
      this.snapshot.apps,
      this.snapshot.runtime.windows,
    )
  }

  getSystemSidebarDataModel(currentAppId?: string): SystemSidebarDataModel {
    const { apps } = this.snapshot
    const { windows } = this.snapshot.runtime
    const systemSidebarSystemAppIds = new Set(['settings', 'diagnostics'])
    const appMap = new Map(apps.map((app) => [app.id, app]))
    const toSidebarApp = (
      app: DesktopAppItem | undefined,
    ): SystemSidebarAppItem | null =>
      app
        ? { appId: app.id, iconKey: app.iconKey, labelKey: app.labelKey }
        : null

    const seenSwitchApps = new Set<string>()
    const switchApps = windows
      .filter(
        (w) =>
          w.state === 'minimized' &&
          w.minimizedOrder !== null &&
          !systemSidebarSystemAppIds.has(w.appId),
      )
      .sort((a, b) => (a.minimizedOrder ?? 0) - (b.minimizedOrder ?? 0))
      .map((w) => {
        const app = appMap.get(w.appId)
        if (!app || w.minimizedOrder === null || seenSwitchApps.has(app.id))
          return null
        seenSwitchApps.add(app.id)
        return {
          appId: app.id,
          iconKey: app.iconKey,
          labelKey: app.labelKey,
          minimizedOrder: w.minimizedOrder,
        }
      })
      .filter(
        (app): app is SystemSidebarDataModel['switchApps'][number] => Boolean(app),
      )

    const systemApps = ['settings', 'diagnostics', 'users-agents']
      .map((appId) => toSidebarApp(appMap.get(appId)))
      .filter((app): app is SystemSidebarAppItem => Boolean(app))

    return {
      currentAppId,
      runningAppCount: windows.filter((w) => w.state !== 'minimized').length,
      switchApps,
      systemApps,
    }
  }

  getResolvedDeadZone(): DeadZone {
    return (
      this.snapshot.syncData.appearance.deadZone ??
      this.defaultPayload?.layout.deadZone ?? { top: 0, bottom: 0, left: 0, right: 0 }
    )
  }

  // ========================================================================
  // Private helpers
  // ========================================================================

  /** Group 2: 持久化窗口几何信息 */
  private persistWindowGeometry(appId: string, geometry: WindowGeometry) {
    if (sameWindowGeometry(this.windowGeometryByApp[appId], geometry)) return
    this.windowGeometryByApp = { ...this.windowGeometryByApp, [appId]: geometry }
    writeJson(windowGeometryStorageKey, this.windowGeometryByApp)
  }

  /** 持久化合并后的 Layout（包含 Group 1 deadZone + Group 4 + Group 5） */
  private persistLayout() {
    const { layoutState, formFactor, scenario } = this.snapshot
    if (!layoutState || scenario !== 'normal') return
    writeJson(layoutStorageKey(formFactor), layoutState)
  }

  private normalizeWindowGeometry(
    app: AppDefinition,
    geometry: Partial<WindowGeometry> | undefined,
    index: number,
    viewportBounds: ReturnType<typeof getDesktopWindowWorkspaceBounds>,
  ): WindowGeometry {
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
}

// ---------------------------------------------------------------------------
// Singleton + React context + hooks
// ---------------------------------------------------------------------------

/** Global store instance. */
export const desktopUIStore = new DesktopUIStore()

const DesktopUIStoreContext = createContext<DesktopUIStore>(desktopUIStore)

export const DesktopUIStoreProvider = DesktopUIStoreContext.Provider

export function useDesktopUIStore() {
  return useContext(DesktopUIStoreContext)
}

/**
 * Subscribe to the full snapshot.
 */
export function useDesktopUISnapshot() {
  const store = useDesktopUIStore()
  return useSyncExternalStore(store.subscribe, store.getSnapshot)
}

// ---------------------------------------------------------------------------
// Convenience selector hooks
// ---------------------------------------------------------------------------

/** 需要同步的完整数据 */
export function useDesktopSyncData() {
  return useDesktopUISnapshot().syncData
}

/** Group 1: 外观设置 */
export function useAppearanceSettings() {
  return useDesktopUISnapshot().syncData.appearance
}

/** Group 2: 窗口布局设置 */
export function useWindowLayoutSettings() {
  return useDesktopUISnapshot().syncData.windowLayout
}

/** Group 3: AppItem 配置 */
export function useAppItemConfig() {
  return useDesktopUISnapshot().syncData.appItemConfig
}

/** Group 4: AppItem 布局 */
export function useAppItemLayout() {
  return useDesktopUISnapshot().syncData.appItemLayout
}

/** Group 5: 小部件布局配置 */
export function useWidgetLayout() {
  return useDesktopUISnapshot().syncData.widgetLayout
}

/** 解析后的 App 列表（带 loader） */
export function useDesktopApps() {
  return useDesktopUISnapshot().apps
}

/** 布局状态 */
export function useDesktopLayout() {
  const snap = useDesktopUISnapshot()
  return {
    layoutState: snap.layoutState,
    resolvedLayout: snap.resolvedLayout,
  }
}

/** 运行时瞬态 */
export function useDesktopRuntime() {
  return useDesktopUISnapshot().runtime
}

/** 加载状态 */
export function useDesktopUIStatus() {
  const snap = useDesktopUISnapshot()
  return { status: snap.status, error: snap.error }
}
