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
import { fetchCurrentAccount } from '../api/account'
import { userTypeCanCreateAgents } from '../api/user_type'
import { fetchAppList } from '../api/app_mgr'
import { deleteDesktopItem, listDesktopFolder, writeDesktopItem, type DesktopFolderListing, type DesktopSeedItem } from '../api/desktop_items'
import { translateNow } from '../i18n/translate'
import { currentZoneHost } from '../presentation/zone'
import { AGENT_GUIDE_APP_ID } from '../app/agent-setup/model'
import {
  resolveDesktopApps,
  findDesktopAppById,
} from '../app/registry'
import {
  buildAuthorizedAppDefinitions,
  withAccountBuiltins,
} from '../app/backend-apps'
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
import { desktopAvatarRowHeight, desktopWorkspaceInsets } from '../desktop/shell'
import { defaultDeadZone } from '../mock/data'
import {
  buildDesktopLayout,
  desktopEntryTarget,
  desktopSeed,
  fileNameStem,
  isDesktopObject,
  parseDesktopEntry,
  sameDesktopEntries,
  SHORTCUT_EXTENSION,
  shortcutRefForApp,
  shortcutText,
  uniqueFileName,
  widgetText,
  type DesktopEntry,
} from './desktopFolder'
import { fetchDesktopPayload } from '../mock/provider'
import { isMockRuntime } from '../runtime'
import { isLauncherApp } from './ui'
import type {
  AppDefinition,
  AppearanceSettings,
  AppIconItem,
  AppItemLayoutSettings,
  DeadZone,
  DesktopPayload,
  DesktopSyncData,
  FormFactor,
  LayoutState,
  MockScenario,
  RuntimeContainer,
  SupportedLocale,
  SystemPreferencesInput,
  TaskbarAppItem,
  ThemeMode,
  WidgetItem,
  WidgetLayoutSettings,
  WindowAppearancePreferences,
  WindowGeometry,
  WindowLayoutSettings,
  WindowLaunch,
  WindowRecord,
} from './ui'
import {
  applyDragCollision,
  clamp,
  invalidatePositions,
  layoutStorageKey,
  migrateToSlotModel,
  readJson,
  readWindowAppearancePreferences,
  refreshSlotIndexes,
  resolveLayout,
  runtimeStorageKey,
  sameWindowGeometry,
  sanitizeWindowGeometryMap,
  windowAppearanceStorageKey,
  desktopTaskbarAppsStorageKey,
  desktopTaskbarPinnedStorageKey,
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
  /** 系统侧边栏是否展开（桌面端即左侧 taskbar 是否划出） */
  isSystemSidebarOpen: boolean
  /** 桌面端 taskbar 是否锁定常驻（持久化到 localStorage） */
  isTaskbarPinned: boolean
  /** 固定在桌面端 taskbar 上的应用，按显示顺序（持久化到 localStorage） */
  taskbarAppIds: string[]
  /** 用户拖动后的 taskbar 按钮顺序，含未固定的运行中应用（仅当前会话） */
  taskbarAppOrder: string[]
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

/** What the Jarvis guide item shows: its linked Agent's name and avatar, or a creation status mark. */
export interface AgentGuidePresentation {
  label?: string
  iconUrl?: string
  badge?: AppDefinition['iconBadge']
}

const AGENT_GUIDE_LABEL_KEY = 'apps.agentGuide'

/** What the taskbar holds before the user pins or unpins anything. */
const defaultTaskbarAppIds = ['messagehub', 'aiworkspace', 'homestation', 'ai-center']

function readTaskbarAppIds(): string[] {
  const stored = readJson<unknown>(desktopTaskbarAppsStorageKey)
  return Array.isArray(stored) && stored.every((id) => typeof id === 'string')
    ? stored
    : defaultTaskbarAppIds
}

/** The state a window comes back to from `minimized`. */
function restoredWindowState(app: AppDefinition, w: WindowRecord): WindowRecord['state'] {
  if (w.state !== 'minimized') return w.state
  return w.restoreState ?? (app.manifest.defaultMode === 'windowed' ? 'windowed' : 'maximized')
}

function minimizedWindow(w: WindowRecord, minimizedOrder: number | null): WindowRecord {
  return {
    ...w,
    state: 'minimized',
    restoreState: w.state === 'minimized' ? w.restoreState : w.state,
    minimizedOrder,
  }
}

async function agentGuideAllowedForAccount(): Promise<boolean> {
  const account = await fetchCurrentAccount().catch(() => null)
  return Boolean(account && userTypeCanCreateAgents(account.user_type))
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
 * Bring `targetId` to `targetZIndex` and renumber the rest as `10 + index`
 * (the shell's long-standing stacking rule). Records whose values are
 * already right are returned as the same object, so the window layer's
 * memoised slots only re-render for windows that actually changed; when
 * nothing changes at all the input array itself is returned.
 */
function raiseWindow(
  windows: WindowRecord[],
  targetId: string,
  targetZIndex: number,
  patch: Partial<WindowRecord> = {},
): WindowRecord[] {
  let changed = false
  const next = windows.map((w, i) => {
    if (w.id === targetId) {
      const patched = { ...w, ...patch, zIndex: targetZIndex }
      const same = (Object.keys(patched) as Array<keyof WindowRecord>).every(
        (key) => patched[key] === w[key],
      )
      if (same) return w
      changed = true
      return patched
    }
    if (w.zIndex === 10 + i) return w
    changed = true
    return { ...w, zIndex: 10 + i }
  })
  return changed ? next : windows
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
  private agentGuideAllowed = false
  private agentGuidePresentation: AgentGuidePresentation = {}

  /** The Desktop folder's entries (models/desktopFolder.ts); the layout shows the ones this account can open. */
  private desktopEntries: DesktopEntry[] = []
  /** The folder as Files shows it; empty until it is listed. */
  private desktopFolderPath = ''
  /** False when the folder could not be listed: the defaults are shown, changes are not saved. */
  private desktopFolderAvailable = true
  private desktopFolderRefresh: Promise<void> | null = null
  private lastDesktopFolderRefresh = 0
  /** Folder writes in flight: a listing taken meanwhile could miss them, so refreshes wait. */
  private desktopFolderWrites = 0
  private static readonly DESKTOP_FOLDER_REFRESH_INTERVAL = 4000

  /**
   * Resize reflow debounce timer.
   * Reflow (invalidation + tail-append) only happens after resize stops,
   * not on every frame. During resize we only do "pure resize" (geometry update).
   */
  private resizeReflowTimer: ReturnType<typeof setTimeout> | null = null
  private static readonly RESIZE_REFLOW_DELAY = 200
  /** Last workspace bounds the view reported — lets non-React callers open windows. */
  private lastViewportBounds: ReturnType<typeof getDesktopWindowWorkspaceBounds> | null = null

  /**
   * Window geometry is updated on every pointermove while dragging/resizing.
   * The in-memory map is always current; the localStorage write is coalesced
   * so we don't JSON.stringify + setItem on every frame.
   */
  private geometryWriteTimer: number | null = null
  private static readonly GEOMETRY_WRITE_DELAY = 200

  /**
   * Monotonic init token. `init()` can be invoked concurrently (StrictMode
   * double-effects, form-factor flips while a fetch is in flight); only the
   * most recent call is allowed to commit its result.
   */
  private initSeq = 0
  private inflightInit: { key: string; promise: Promise<void> } | null = null

  /** Windows hidden by the last "show desktop", bottom-to-top. */
  private shownDesktopWindowIds: string[] = []

  /** Memo for `getTaskbarApps()`. */
  private taskbarAppsCache: {
    apps: DesktopAppItem[]
    windows: WindowRecord[]
    taskbarAppIds: string[]
    taskbarAppOrder: string[]
    items: TaskbarAppItem[]
  } | null = null

  /** Memo for `getWindowLayerModel()` — keyed on the inputs' identity. */
  private windowLayerModelCache: {
    apps: DesktopAppItem[]
    windows: WindowRecord[]
    model: ReturnType<typeof createDesktopWindowLayerDataModel>
  } | null = null

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
        isTaskbarPinned: readJson<boolean>(desktopTaskbarPinnedStorageKey) === true,
        taskbarAppIds: readTaskbarAppIds(),
        taskbarAppOrder: [],
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

    // Make sure a coalesced geometry write is not lost when the page goes away.
    window.addEventListener('pagehide', () => this.flushGeometryWrite())
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
    for (const listener of this.listeners) listener()
  }

  /**
   * Low-level update: accepts partial top-level fields + partial runtime.
   *
   * The snapshot is treated as immutable: a new object is produced and the
   * derived fields (`resolvedLayout`, `syncData`) are only recomputed when
   * one of their inputs actually changed. This matters because window
   * drag/resize calls `update()` on every pointermove — recomputing the
   * layout there would also churn the identity of `resolvedLayout`, which
   * every open app panel receives as a prop.
   */
  private update(
    partial: Partial<Pick<DesktopUISnapshot, 'status' | 'error' | 'formFactor' | 'scenario' | 'layoutState' | 'apps'>> & {
      runtime?: Partial<DesktopRuntimeState>
      appearance?: Partial<AppearanceSettings>
      /** Group 3: replace the AppItem config source list. */
      appItemApps?: AppDefinition[]
    },
  ) {
    const prev = this.snapshot
    const next: DesktopUISnapshot = { ...prev }

    // Merge top-level scalars
    if (partial.status !== undefined) next.status = partial.status
    if (partial.error !== undefined) next.error = partial.error
    if (partial.formFactor !== undefined) next.formFactor = partial.formFactor
    if (partial.scenario !== undefined) next.scenario = partial.scenario
    if (partial.apps !== undefined) next.apps = partial.apps
    if (partial.layoutState !== undefined) next.layoutState = partial.layoutState

    // Merge runtime
    if (partial.runtime) {
      next.runtime = { ...prev.runtime, ...partial.runtime }
    }

    // Merge appearance
    const appearance = partial.appearance
      ? { ...prev.syncData.appearance, ...partial.appearance }
      : prev.syncData.appearance
    const appItemApps = partial.appItemApps ?? prev.syncData.appItemConfig.apps

    // Derived: resolvedLayout (only when its inputs changed)
    const layoutInputsChanged =
      next.layoutState !== prev.layoutState ||
      next.formFactor !== prev.formFactor ||
      next.runtime.gridCols !== prev.runtime.gridCols ||
      next.runtime.gridRows !== prev.runtime.gridRows
    if (layoutInputsChanged) {
      next.resolvedLayout = this.computeResolvedLayout(next)
    }

    // Derived: syncData (rebuild only the groups whose source changed)
    const layoutChanged = next.layoutState !== prev.layoutState
    const geometryChanged =
      prev.syncData.windowLayout.geometryByApp !== this.windowGeometryByApp
    if (
      layoutChanged ||
      geometryChanged ||
      appearance !== prev.syncData.appearance ||
      appItemApps !== prev.syncData.appItemConfig.apps
    ) {
      next.syncData = {
        appearance,
        windowLayout: geometryChanged
          ? { geometryByApp: this.windowGeometryByApp }
          : prev.syncData.windowLayout,
        appItemConfig:
          appItemApps === prev.syncData.appItemConfig.apps
            ? prev.syncData.appItemConfig
            : { apps: appItemApps },
        appItemLayout: layoutChanged
          ? next.layoutState
            ? extractAppItemLayout(next.layoutState)
            : { pages: [] }
          : prev.syncData.appItemLayout,
        widgetLayout: layoutChanged
          ? next.layoutState
            ? extractWidgetLayout(next.layoutState)
            : { pages: [] }
          : prev.syncData.widgetLayout,
      }
    }

    this.snapshot = next
    this.notify()
  }

  // ---- Derived data recomputation -----------------------------------------

  private computeResolvedLayout(snapshot: DesktopUISnapshot): LayoutState | null {
    const { layoutState, formFactor } = snapshot
    const { gridCols, gridRows } = snapshot.runtime
    if (!layoutState) return null
    const scanOrder: ScanOrder = formFactor === 'mobile' ? 'row-major' : 'col-major'
    return resolveLayout(layoutState, gridCols, gridRows, scanOrder)
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
    // Coalesce identical concurrent calls (StrictMode runs the mount effect
    // twice) so we neither double-fetch nor let a stale result win.
    const key = `${formFactor}:${scenario}`
    if (this.inflightInit?.key === key) {
      await this.inflightInit.promise
      return
    }
    const seq = ++this.initSeq
    const promise = isMockRuntime()
      ? this.initByMock(formFactor, scenario, seq)
      : this.initByReal(formFactor, seq)
    this.inflightInit = { key, promise }
    try {
      await promise
    } finally {
      if (this.inflightInit?.promise === promise) this.inflightInit = null
    }
  }

  /** True when a newer `init()` superseded the call identified by `seq`. */
  private isStaleInit(seq: number) {
    return seq !== this.initSeq
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
  private async initByMock(formFactor: FormFactor, scenario: MockScenario, seq: number) {
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
      const [rawPayload, agentGuideAllowed, folder] = await Promise.all([
        fetchDesktopPayload({ formFactor, scenario }),
        agentGuideAllowedForAccount(),
        scenario === 'normal' ? this.listFolder() : Promise.resolve(null),
      ])
      if (this.isStaleInit(seq)) return
      this.agentGuideAllowed = agentGuideAllowed
      const payload = { ...rawPayload, apps: withAccountBuiltins(rawPayload.apps, { agentGuide: agentGuideAllowed }) }
      this.defaultPayload = payload
      const apps = this.resolveApps(payload.apps, formFactor)
      this.acceptFolder(folder)
      const stored = scenario === 'normal' ? readJson<LayoutState>(layoutStorageKey(formFactor)) : null
      let layoutState = this.layoutFromFolder(apps, formFactor, stored, stored?.deadZone ?? payload.layout.deadZone)

      // Fill slotIndex / seq for items that only carry x/y
      const { gridCols, gridRows } = this.snapshot.runtime
      layoutState = migrateToSlotModel(layoutState, gridCols, gridRows)

      // Validate positions against current grid (init-time reflow)
      layoutState = invalidatePositions(layoutState, gridCols, gridRows)

      this.update({
        status: 'success',
        apps,
        layoutState,
        // Populate syncData (Group 3) from payload
        appItemApps: payload.apps,
        appearance: {
          wallpaper: payload.wallpaper,
          deadZone: layoutState.deadZone,
        },
      })
    } catch (err) {
      if (this.isStaleInit(seq)) return
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
  private async initByReal(formFactor: FormFactor, seq: number) {
    this.update({
      status: 'loading',
      error: null,
      formFactor,
      scenario: 'normal',
      runtime: {
        windows: [],
        isSystemSidebarOpen: false,
        viewportProgress: 0,
      },
    })

    try {
      const [payload, appsResult, agentGuideAllowed, folder] = await Promise.all([
        fetchDesktopPayload({ formFactor, scenario: 'normal' }),
        fetchAppList(),
        agentGuideAllowedForAccount(),
        this.listFolder(),
      ])
      if (this.isStaleInit(seq)) return
      if (!appsResult.data) {
        throw appsResult.error ?? new Error('apps.list unavailable')
      }

      this.agentGuideAllowed = agentGuideAllowed
      const authorizedDefinitions = withAccountBuiltins(
        buildAuthorizedAppDefinitions(payload.apps, appsResult.data.apps, this.backendAppMapperOptions()),
        { agentGuide: agentGuideAllowed },
      )
      this.defaultPayload = { ...payload, apps: authorizedDefinitions }
      const apps = this.resolveApps(authorizedDefinitions, formFactor)
      this.acceptFolder(folder)
      const stored = readJson<LayoutState>(layoutStorageKey(formFactor))
      let layoutState = this.layoutFromFolder(apps, formFactor, stored, stored?.deadZone ?? payload.layout.deadZone)
      const { gridCols, gridRows } = this.snapshot.runtime
      layoutState = migrateToSlotModel(layoutState, gridCols, gridRows)
      layoutState = invalidatePositions(layoutState, gridCols, gridRows)

      this.update({
        status: 'success',
        apps,
        layoutState,
        appItemApps: authorizedDefinitions,
        appearance: {
          wallpaper: payload.wallpaper,
          deadZone: layoutState.deadZone,
        },
      })
    } catch (err) {
      if (this.isStaleInit(seq)) return
      this.update({
        status: 'error',
        error: err instanceof Error ? err.message : String(err),
      })
    }
  }

  // ========================================================================
  // Desktop folder (models/desktopFolder.ts)
  // ========================================================================

  /**
   * Lists the Desktop folder (creating it with the default items for a new account). When it cannot be listed
   * — an older control panel, a network error — the default items are shown without being saved.
   */
  private async listFolder(): Promise<DesktopFolderListing & { unavailable?: boolean }> {
    const seed = desktopSeed(translateNow)
    try {
      return await listDesktopFolder(seed)
    } catch (error) {
      console.warn('desktop: the Desktop folder is unavailable, showing the default items', error)
      return {
        path: '',
        created: false,
        unavailable: true,
        items: seed.map((item) => ({ name: item.name, kind: 'file' as const, size: item.text.length, mtime: 0, text: item.text })),
      }
    }
  }

  /** Takes a listing in; null means no folder (the empty mock scenario). */
  private acceptFolder(folder: (DesktopFolderListing & { unavailable?: boolean }) | null) {
    this.desktopEntries = folder ? folder.items.map(parseDesktopEntry) : []
    this.desktopFolderPath = folder?.path ?? ''
    this.desktopFolderAvailable = Boolean(folder && !folder.unavailable)
    this.lastDesktopFolderRefresh = Date.now()
  }

  /** The folder's entries as a layout, placed where `positions` has them (or by default when it is null). */
  private layoutFromFolder(
    apps: readonly AppDefinition[],
    formFactor: FormFactor,
    positions: LayoutState | null,
    deadZone: DeadZone,
  ): LayoutState {
    return buildDesktopLayout({
      entries: this.desktopEntries,
      folderPath: this.desktopFolderPath,
      apps: apps.filter(isLauncherApp),
      formFactor,
      deadZone,
      positions,
    })
  }

  /** Rebuilds the layout from the current entries and apps, keeping every item where it is shown now. */
  private relayoutFromFolder(positionsOverride?: LayoutState | null) {
    const { layoutState, resolvedLayout, formFactor, apps } = this.snapshot
    if (!layoutState) return
    const positions = positionsOverride === undefined ? (resolvedLayout ?? layoutState) : positionsOverride
    const { gridCols, gridRows } = this.snapshot.runtime
    let next = this.layoutFromFolder(apps, formFactor, positions, layoutState.deadZone)
    next = migrateToSlotModel(next, gridCols, gridRows)
    next = invalidatePositions(next, gridCols, gridRows)
    if (JSON.stringify(next.pages) === JSON.stringify(layoutState.pages)) return
    this.update({ layoutState: next })
    this.persistLayout()
  }

  /** The Desktop folder as Files shows it (`/home/{user}/Desktop`); empty when it is unavailable. */
  getDesktopFolderPath(): string {
    return this.desktopFolderPath
  }

  /**
   * Picks up changes made elsewhere (Files, another device). Coalesced and rate-limited, so callers can ask
   * on every focus; `force` skips the rate limit (after a local change in Files).
   */
  async refreshDesktopFolder(options: { force?: boolean } = {}): Promise<void> {
    if (!this.canEditDesktop() || this.desktopFolderWrites > 0) return
    if (this.desktopFolderRefresh) return this.desktopFolderRefresh
    if (!options.force && Date.now() - this.lastDesktopFolderRefresh < DesktopUIStore.DESKTOP_FOLDER_REFRESH_INTERVAL) return
    const seq = this.initSeq
    this.lastDesktopFolderRefresh = Date.now()
    this.desktopFolderRefresh = (async () => {
      const folder = await this.listFolder()
      if (seq !== this.initSeq || folder.unavailable || this.desktopFolderWrites > 0) return
      const entries = folder.items.map(parseDesktopEntry)
      if (!folder.created && sameDesktopEntries(entries, this.desktopEntries)) return
      this.acceptFolder(folder)
      // A folder created again (the user deleted it) starts over with the default arrangement.
      this.relayoutFromFolder(folder.created ? null : undefined)
    })().finally(() => {
      this.desktopFolderRefresh = null
    })
    return this.desktopFolderRefresh
  }

  /** True when the Desktop folder was listed, so the desktop can change it. */
  canEditDesktop(): boolean {
    return this.snapshot.status === 'success' && this.snapshot.scenario === 'normal' && this.desktopFolderAvailable
  }

  /** Runs a folder write; a failure is shown and the folder read again, so the desktop matches it. */
  private async writeFolder(write: () => Promise<void>): Promise<void> {
    this.desktopFolderWrites += 1
    let failed = false
    try {
      await write()
    } catch (error) {
      failed = true
      console.warn('desktop: Desktop folder write failed', error)
      this.update({ runtime: { snackbar: translateNow('desktop.folder.saveFailed', 'The Desktop folder could not be changed.') } })
    } finally {
      this.desktopFolderWrites -= 1
    }
    if (failed) await this.refreshDesktopFolder({ force: true })
  }

  /** Whether a shortcut on the desktop opens this app. */
  isAppOnDesktop(appId: string): boolean {
    return Boolean(
      this.snapshot.layoutState?.pages.some((page) => page.items.some((item) => item.type === 'app' && item.appId === appId)),
    )
  }

  /** The Desktop folder entry an item shows. */
  getDesktopEntry(itemId: string): DesktopEntry | undefined {
    return this.desktopEntries.find((entry) => entry.name === itemId)
  }

  /** Puts a shortcut to the app on the desktop: `{label}.lnk` in the Desktop folder, where there is room. */
  async addAppToDesktop(appId: string, label: string): Promise<void> {
    const app = this.snapshot.apps.find((entry) => entry.id === appId)
    if (!app || !this.canEditDesktop() || this.isAppOnDesktop(appId)) return
    const taken = new Set(this.desktopEntries.map((entry) => entry.name))
    const name = uniqueFileName(fileNameStem(label, appId), SHORTCUT_EXTENSION, taken)
    const ref = shortcutRefForApp(app)
    this.desktopEntries = [...this.desktopEntries, { name, kind: 'app', app: ref, raw: { app: ref } }]
    this.relayoutFromFolder()
    await this.writeFolder(() => writeDesktopItem(name, shortcutText(ref), { create: true }))
  }

  /** Removes a shortcut or widget from the desktop (deletes its file). Documents and folders stay. */
  async removeDesktopItem(itemId: string): Promise<void> {
    const entry = this.getDesktopEntry(itemId)
    if (!entry || !isDesktopObject(entry) || !this.canEditDesktop()) return
    this.desktopEntries = this.desktopEntries.filter((other) => other !== entry)
    this.update({ runtime: { contextMenu: null, selectedItemId: null } })
    this.relayoutFromFolder()
    await this.writeFolder(() => deleteDesktopItem(entry.name))
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
    // While a form-factor switch loads, the layout still is the other form factor's.
    if (!layoutState || this.snapshot.status !== 'success') return

    const before = layoutState
    layoutState = invalidatePositions(layoutState, cols, rows)

    // Migrate legacy layouts to slot model
    layoutState = migrateToSlotModel(layoutState, cols, rows)

    // Slot indexes are grid-relative; keep them in step with the new grid.
    layoutState = refreshSlotIndexes(layoutState, cols, rows)

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
    this.lastViewportBounds = opts.viewportBounds

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
      const nextWindows = raiseWindow(windows, existing.id, windows.length + 10, {
        state: restoredWindowState(app, existing),
        minimizedOrder: null,
      })
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
      // A new window always lands on top of the currently focused one.
      const topZIndex = windows.reduce((top, w) => Math.max(top, w.zIndex), 10 + windows.length)
      const record = { ...createWindowRecord(app, windows.length, geometry), zIndex: topZIndex + 1 }
      this.update({ runtime: { windows: [...windows, record] } })
    }
    opts.logActivity?.(`Opened ${appId}`)
  }

  private fallbackViewportBounds() {
    const insets = desktopWorkspaceInsets(this.snapshot.runtime.isTaskbarPinned)
    return getDesktopWindowWorkspaceBounds({
      safeArea: { top: 0, bottom: 0, left: 0, right: 0 },
      topInset: insets.top,
      leftInset: insets.left,
      viewportSize: { width: window.innerWidth, height: window.innerHeight },
    })
  }

  /**
   * Opens — or re-targets — a window that carries a launch request.
   *
   * Unlike `openApp` (one window per app), callers can ask for a fresh
   * instance (`newInstance`) or address an existing window by id. Multi-window
   * apps such as Preview schedule their own windows on top of this primitive
   * and only exchange launch metadata with the shell, never content.
   * Returns the window id, or null when the app is unknown.
   */
  openAppWindow(
    appId: string,
    opts: {
      viewportBounds?: ReturnType<typeof getDesktopWindowWorkspaceBounds>
      launch?: WindowLaunch
      title?: string
      newInstance?: boolean
      windowId?: string
      instanceKey?: string
    } = {},
  ): string | null {
    const app = findDesktopAppById(this.snapshot.apps, appId)
    if (!app) return null
    const { windows } = this.snapshot.runtime
    const target = opts.windowId
      ? windows.find((w) => w.id === opts.windowId)
      : opts.newInstance
        ? undefined
        : windows.find((w) => w.appId === appId && (opts.instanceKey ? w.instanceKey === opts.instanceKey : true))

    if (target) {
      const top = windows.reduce((max, w) => Math.max(max, w.zIndex), windows.length + 10) + 1
      this.update({
        runtime: {
          windows: raiseWindow(windows, target.id, top, {
            state: restoredWindowState(app, target),
            minimizedOrder: null,
            launch: opts.launch ?? target.launch,
            title: opts.title ?? target.title,
          }),
        },
      })
      return target.id
    }

    const bounds = opts.viewportBounds ?? this.lastViewportBounds ?? this.fallbackViewportBounds()
    const preferredGeometry =
      this.snapshot.scenario === 'normal' ? this.windowGeometryByApp[app.id] : undefined
    const geometry = this.normalizeWindowGeometry(app, preferredGeometry, windows.length, bounds)
    const siblings = windows.filter((w) => w.appId === appId).length
    // Cascade sibling instances so a second window never lands exactly on the first.
    const cascade = siblings * 28
    const topZIndex = windows.reduce((top, w) => Math.max(top, w.zIndex), 10 + windows.length)
    const record: WindowRecord = {
      ...createWindowRecord(app, windows.length, geometry),
      id: `${app.id}-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
      // A new window always lands on top of the currently focused one.
      zIndex: topZIndex + 1,
      x: clamp(geometry.x + cascade, bounds.minX, Math.max(bounds.minX, bounds.maxRight - geometry.width)),
      y: clamp(geometry.y + cascade, bounds.minY, Math.max(bounds.minY, bounds.maxBottom - geometry.height)),
      instanceKey: opts.instanceKey,
      launch: opts.launch,
      title: opts.title,
    }
    this.update({ runtime: { windows: [...windows, record] } })
    return record.id
  }

  /** Update the shell-visible metadata of a window (title, launch request). */
  updateWindow(windowId: string, patch: Partial<Pick<WindowRecord, 'title' | 'launch' | 'instanceKey'>>) {
    const { windows } = this.snapshot.runtime
    if (!windows.some((w) => w.id === windowId)) return
    this.update({
      runtime: {
        windows: windows.map((w) => (w.id === windowId ? { ...w, ...patch } : w)),
      },
    })
  }

  async refreshApps() {
    const result = await fetchAppList()
    if (!result.data) throw result.error ?? new Error('apps.list unavailable')
    const definitions = withAccountBuiltins(
      buildAuthorizedAppDefinitions(this.defaultPayload?.apps ?? [], result.data.apps, this.backendAppMapperOptions()),
      { agentGuide: this.agentGuideAllowed },
    )
    this.update({ apps: this.resolveApps(definitions, this.snapshot.formFactor) })
    // A shortcut to an app installed (or removed) since shows (or hides) now.
    this.relayoutFromFolder()
  }

  /** Updates the Jarvis guide item's label, image and status mark (see app/agent-setup/guide.ts). */
  setAgentGuidePresentation(presentation: AgentGuidePresentation) {
    const current = this.agentGuidePresentation
    if (
      current.label === presentation.label &&
      current.iconUrl === presentation.iconUrl &&
      current.badge === presentation.badge
    ) return
    this.agentGuidePresentation = presentation
    if (!this.snapshot.apps.some((app) => app.id === AGENT_GUIDE_APP_ID)) return
    this.update({ apps: this.snapshot.apps.map((app) => this.decorateApp(app)) })
  }

  private decorateApp<T extends AppDefinition>(app: T): T {
    if (app.id !== AGENT_GUIDE_APP_ID) return app
    const { label, iconUrl, badge } = this.agentGuidePresentation
    return { ...app, labelKey: AGENT_GUIDE_LABEL_KEY, text: label ? { title: label } : undefined, iconUrl, iconBadge: badge }
  }

  private backendAppMapperOptions() {
    return { zoneHost: currentZoneHost(), protocol: window.location.protocol }
  }

  private resolveApps(definitions: AppDefinition[], formFactor: FormFactor): DesktopAppItem[] {
    return resolveDesktopApps(definitions, formFactor).map((app) => this.decorateApp(app))
  }

  private closeGuards = new Map<string, (reason?: 'user' | 'logout' | 'shell') => Promise<boolean>>()
  private closingWindows = new Set<string>()

  registerCloseGuard(windowId: string, guard: (reason?: 'user' | 'logout' | 'shell') => Promise<boolean>) {
    this.closeGuards.set(windowId, guard)
    return () => { this.closeGuards.delete(windowId) }
  }

  async prepareLogout(): Promise<boolean> {
    for (const guard of this.closeGuards.values()) if (!await guard('logout')) return false
    return true
  }

  closeWindow(windowId: string, force = false) {
    const guard = this.closeGuards.get(windowId)
    if (!force && guard) {
      if (this.closingWindows.has(windowId)) return
      this.closingWindows.add(windowId)
      void guard().then(allow => { if (allow) this.closeWindow(windowId, true) })
        .finally(() => this.closingWindows.delete(windowId))
      return
    }
    this.closeGuards.delete(windowId)
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
          w.id === windowId ? minimizedWindow(w, order) : w,
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
    const { windows } = this.snapshot.runtime
    if (!windows.some((w) => w.id === windowId)) return
    const nextWindows = raiseWindow(windows, windowId, windows.length + 12)
    // Clicking the window that is already in front changes nothing.
    if (nextWindows === windows) return
    this.update({ runtime: { windows: nextWindows } })
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
    this.shownDesktopWindowIds = visibleWindowIds

    this.update({
      runtime: {
        windows: this.snapshot.runtime.windows.map((w) =>
          w.state === 'minimized'
            ? w
            : minimizedWindow(w, minimizedOrderMap.get(w.id) ?? null),
        ),
        isSystemSidebarOpen: false,
      },
    })
  }

  /**
   * The taskbar's "Show desktop": minimizes every window, and when nothing has
   * been brought back since, a second press restores the windows it hid in
   * their old stacking order.
   */
  toggleShowDesktop() {
    const { windows } = this.snapshot.runtime
    if (windows.some((w) => w.state !== 'minimized')) {
      this.returnToDesktop()
      return
    }
    const hidden = this.shownDesktopWindowIds
    this.shownDesktopWindowIds = []
    // `hidden` is bottom-to-top, so later entries land higher.
    let top = windows.reduce((max, w) => Math.max(max, w.zIndex), 10 + windows.length)
    const restored = new Map<string, number>()
    for (const id of hidden) {
      if (windows.some((w) => w.id === id)) restored.set(id, ++top)
    }
    if (restored.size === 0) return
    this.update({
      runtime: {
        windows: windows.map((w) => {
          const zIndex = restored.get(w.id)
          const app = findDesktopAppById(this.snapshot.apps, w.appId)
          if (zIndex === undefined || !app) return w
          return { ...w, state: restoredWindowState(app, w), minimizedOrder: null, zIndex }
        }),
      },
    })
  }

  /**
   * A taskbar click on an app that has windows: minimizes its window when it
   * is in front, otherwise restores and raises its front-most window (or the
   * one minimized last). Returns false when the app has no window, so the
   * caller launches it.
   */
  activateTaskbarApp(appId: string): boolean {
    const own = this.snapshot.runtime.windows.filter((w) => w.appId === appId)
    if (own.length === 0) return false
    const front = this.getWindowLayerModel().topWindow
    if (front?.appId === appId) {
      this.minimizeWindow(front.id)
      return true
    }
    const visible = own.filter((w) => w.state !== 'minimized')
    const target = visible.length > 0
      ? visible.reduce((a, b) => (b.zIndex > a.zIndex ? b : a))
      : own.reduce((a, b) => ((b.minimizedOrder ?? 0) > (a.minimizedOrder ?? 0) ? b : a))
    this.openAppWindow(appId, { windowId: target.id })
    return true
  }

  /**
   * Normalise all open window positions for a new viewport size.
   */
  normalizeOpenWindowsForViewport(
    viewportSize: { width: number; height: number },
    safeArea?: { top: number; bottom: number; left: number; right: number },
  ) {
    const resolvedSafeArea = safeArea ?? { top: 0, bottom: 0, left: 0, right: 0 }
    const insets = desktopWorkspaceInsets(this.snapshot.runtime.isTaskbarPinned)
    const bounds = getDesktopWindowWorkspaceBounds({
      safeArea: resolvedSafeArea,
      topInset: resolvedSafeArea.top + insets.top,
      leftInset: insets.left,
      viewportSize,
    })
    this.lastViewportBounds = bounds

    let changed = false
    const nextWindows = this.snapshot.runtime.windows.map((w, i) => {
      const app = findDesktopAppById(this.snapshot.apps, w.appId)
      if (!app) return w
      const normalizedGeometry = this.normalizeWindowGeometry(app, w, i, bounds)
      const geometry = {
        ...normalizedGeometry,
        x: clamp(
          normalizedGeometry.x,
          bounds.minX,
          bounds.maxRight - normalizedGeometry.width,
        ),
        y: clamp(
          normalizedGeometry.y,
          bounds.minY,
          bounds.maxBottom - normalizedGeometry.height,
        ),
      }
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
   * Handle drag-stop with proposal-compliant collision rules
   * (see `applyDragCollision` in layout.ts):
   *
   * 1. **Empty target footprint** → place directly, 100% success
   * 2. **Occupied** → bump every displaced item to the nearest position
   *    where its full footprint fits (same page only)
   * 3. **Nothing fits** → same-page reorder for all-1×1 pages,
   *    otherwise the drop reverts
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

    const { layoutState, resolvedLayout, formFactor } = this.snapshot
    if (!layoutState || !resolvedLayout) return

    const { gridCols: cols, gridRows: rows } = this.snapshot.runtime
    const order: ScanOrder = formFactor === 'mobile' ? 'row-major' : 'col-major'

    // Collide against the page *as displayed*: auto-placed items only have
    // a position in `resolvedLayout`. Resolving them on the unpositioned
    // `layoutState` page ignored them as occupants and, worse, re-derived
    // their tail slot after the drop, so every auto item on the page slid
    // along behind the dragged one.
    const resolvedPageIndex = resolvedLayout.pages.findIndex((page) => page.id === pageId)
    if (resolvedPageIndex < 0) return
    const nextPage = applyDragCollision(
      resolvedLayout.pages[resolvedPageIndex],
      oldItem,
      newItem,
      cols,
      rows,
      order,
    )
    if (nextPage === resolvedLayout.pages[resolvedPageIndex]) return

    // Commit the layout *as displayed*, with the target page replaced by the
    // collision result. `resolveLayout` places every item of `layoutState`
    // on some page (creating overflow pages when needed), so taking its
    // pages wholesale keeps items that a shrunken grid pushed off the page
    // they are stored on: replacing only the target page with its resolved
    // contents used to drop those (they were stored on the target page but
    // displayed elsewhere), together with their config such as note text.
    // Auto-placed items keep `placementType: 'auto'`, so a later resize may
    // still reflow them.
    const pages = resolvedLayout.pages.map((page, index) =>
      index === resolvedPageIndex ? nextPage : page,
    )

    this.update({ layoutState: { ...layoutState, pages } })
    this.persistLayout()
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

    // `resolvedLayout` may contain overflow pages that do not exist in
    // `layoutState` yet, so the target index can be more than one page past
    // the end — create every missing page, not just one.
    while (targetPageIndex >= nextPages.length) {
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

  /** Saves a note widget's text into its `.widget` file. */
  updateWidgetNote(itemId: string, content: string) {
    const { layoutState } = this.snapshot
    const entry = this.getDesktopEntry(itemId)
    if (!layoutState || entry?.kind !== 'widget') return

    const raw = { ...entry.raw, text: content }
    this.desktopEntries = this.desktopEntries.map((other) => (other === entry ? { ...entry, raw } : other))
    this.update({
      layoutState: {
        ...layoutState,
        pages: layoutState.pages.map((page) => ({
          ...page,
          items: page.items.map((item) =>
            item.id === itemId && item.type === 'widget'
              ? { ...item, config: { ...item.config, text: content } }
              : item,
          ),
        })),
      },
    })
    if (!this.canEditDesktop()) return
    void this.writeFolder(() => writeDesktopItem(entry.name, widgetText(raw)))
  }

  /**
   * Puts this device's desktop back to the default arrangement and closes its windows. The default items the
   * Desktop folder no longer has (by what they open, not by name) are written back first.
   */
  async restoreDefaults() {
    if (!this.defaultPayload) return
    const { formFactor } = this.snapshot
    window.localStorage.removeItem(layoutStorageKey(formFactor))
    this.cancelGeometryWrite()
    window.localStorage.removeItem(windowGeometryStorageKey)
    this.windowGeometryByApp = {}
    this.update({ runtime: { windows: [] } })

    const missing: DesktopSeedItem[] = []
    if (this.canEditDesktop()) {
      const present = new Set(this.desktopEntries.map(desktopEntryTarget))
      const taken = new Set(this.desktopEntries.map((entry) => entry.name))
      for (const item of desktopSeed(translateNow)) {
        const entry = parseDesktopEntry({ name: item.name, kind: 'file', size: 0, mtime: 0, text: item.text })
        if (present.has(desktopEntryTarget(entry))) continue
        const dot = item.name.lastIndexOf('.')
        const name = uniqueFileName(item.name.slice(0, dot), item.name.slice(dot), taken)
        taken.add(name)
        missing.push({ name, text: item.text })
        this.desktopEntries = [...this.desktopEntries, { ...entry, name }]
      }
    }
    this.relayoutFromFolder(null)
    if (missing.length === 0) return
    await this.writeFolder(async () => {
      for (const item of missing) await writeDesktopItem(item.name, item.text, { create: true })
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
      this.normalizeOpenWindowsForViewport(callbacks.viewportSize)
    }
    this.persistLayout()
  }

  /** Window opacity from the account (AccountAppearanceSync); the Settings app saves it through applySettings. */
  setWindowAppearance(next: WindowAppearancePreferences) {
    const current = this.snapshot.syncData.appearance.windowAppearance
    if (
      current.titleBarOpacity === next.titleBarOpacity &&
      current.backgroundOpacity === next.backgroundOpacity
    ) {
      return
    }
    writeJson(windowAppearanceStorageKey, next)
    this.update({ appearance: { windowAppearance: { ...next } } })
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

  /**
   * Pin (or unpin) the desktop taskbar. A pinned taskbar takes its width from
   * the window workspace, so open windows are pushed out from under it.
   * Unpinning leaves it open until the next outside click.
   */
  setTaskbarPinned(pinned: boolean, viewportSize: { width: number; height: number }) {
    if (pinned === this.snapshot.runtime.isTaskbarPinned) return
    writeJson(desktopTaskbarPinnedStorageKey, pinned)
    this.update({ runtime: { isTaskbarPinned: pinned, isSystemSidebarOpen: true } })
    this.normalizeOpenWindowsForViewport(viewportSize)
  }

  /** Pin an app to the end of the desktop taskbar, or unpin it. */
  setTaskbarAppPinned(appId: string, pinned: boolean) {
    const current = this.snapshot.runtime.taskbarAppIds
    if (current.includes(appId) === pinned) return
    const next = pinned ? [...current, appId] : current.filter((id) => id !== appId)
    writeJson(desktopTaskbarAppsStorageKey, next)
    this.update({ runtime: { taskbarAppIds: next } })
  }

  /**
   * Move a taskbar button to `index` among the buttons the taskbar shows.
   * Pinned apps keep their new order across reloads; running ones that are
   * not pinned keep their place for the session.
   */
  moveTaskbarApp(appId: string, index: number) {
    const order = this.getTaskbarApps().map((item) => item.appId)
    const from = order.indexOf(appId)
    if (from < 0 || from === index) return
    order.splice(from, 1)
    order.splice(index, 0, appId)
    const pinned = this.snapshot.runtime.taskbarAppIds
    // Pinned apps the taskbar cannot show (not installed here) stay pinned.
    const taskbarAppIds = [
      ...order.filter((id) => pinned.includes(id)),
      ...pinned.filter((id) => !order.includes(id)),
    ]
    writeJson(desktopTaskbarAppsStorageKey, taskbarAppIds)
    this.update({ runtime: { taskbarAppIds, taskbarAppOrder: order } })
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
    const { apps } = this.snapshot
    const { windows } = this.snapshot.runtime
    const cached = this.windowLayerModelCache
    if (cached && cached.apps === apps && cached.windows === windows) {
      return cached.model
    }
    const model = createDesktopWindowLayerDataModel(apps, windows)
    this.windowLayerModelCache = { apps, windows, model }
    return model
  }

  /**
   * The desktop taskbar's app buttons: pinned apps in pinned order, then the
   * other apps with open windows in the order they were opened. Buttons the
   * user has dragged keep that order ahead of the rest.
   */
  getTaskbarApps(): TaskbarAppItem[] {
    const { apps } = this.snapshot
    const { windows, taskbarAppIds, taskbarAppOrder } = this.snapshot.runtime
    const cached = this.taskbarAppsCache
    if (
      cached &&
      cached.apps === apps &&
      cached.windows === windows &&
      cached.taskbarAppIds === taskbarAppIds &&
      cached.taskbarAppOrder === taskbarAppOrder
    ) {
      return cached.items
    }
    const frontAppId = this.getWindowLayerModel().topWindow?.appId
    const pinned = new Set(taskbarAppIds)
    const rank = (id: string) => {
      const index = taskbarAppOrder.indexOf(id)
      return index < 0 ? taskbarAppOrder.length : index
    }
    const ids = [...new Set([...taskbarAppIds, ...windows.map((w) => w.appId)])].sort(
      (a, b) => rank(a) - rank(b),
    )
    const items = ids.flatMap((appId): TaskbarAppItem[] => {
      const app = findDesktopAppById(apps, appId)
      if (!app) return []
      return [{
        appId: app.id,
        accent: app.accent,
        iconKey: app.iconKey,
        iconUrl: app.iconUrl,
        labelKey: app.labelKey,
        text: app.text,
        pinned: pinned.has(app.id),
        windowCount: windows.filter((w) => w.appId === app.id).length,
        active: frontAppId === app.id,
      }]
    })
    // Window drags replace `windows` on every move; keep the old array when
    // nothing the taskbar shows has changed.
    const same =
      cached !== null &&
      cached.items.length === items.length &&
      cached.items.every((item, i) => {
        const next = items[i]
        return (Object.keys(next) as Array<keyof TaskbarAppItem>).every((key) => next[key] === item[key])
      })
    const result = same && cached ? cached.items : items
    this.taskbarAppsCache = { apps, windows, taskbarAppIds, taskbarAppOrder, items: result }
    return result
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

  /** Group 2: 持久化窗口几何信息（内存立即更新，localStorage 写入合并） */
  private persistWindowGeometry(appId: string, geometry: WindowGeometry) {
    if (sameWindowGeometry(this.windowGeometryByApp[appId], geometry)) return
    this.windowGeometryByApp = { ...this.windowGeometryByApp, [appId]: geometry }
    if (this.geometryWriteTimer !== null) return
    this.geometryWriteTimer = window.setTimeout(
      () => this.flushGeometryWrite(),
      DesktopUIStore.GEOMETRY_WRITE_DELAY,
    )
  }

  private flushGeometryWrite() {
    if (this.geometryWriteTimer === null) return
    window.clearTimeout(this.geometryWriteTimer)
    this.geometryWriteTimer = null
    writeJson(windowGeometryStorageKey, this.windowGeometryByApp)
  }

  private cancelGeometryWrite() {
    if (this.geometryWriteTimer === null) return
    window.clearTimeout(this.geometryWriteTimer)
    this.geometryWriteTimer = null
  }

  /** 持久化合并后的 Layout（包含 Group 1 deadZone + Group 4 + Group 5） */
  private persistLayout() {
    const { layoutState, formFactor, scenario, status } = this.snapshot
    if (!layoutState || scenario !== 'normal' || status !== 'success' || layoutState.formFactor !== formFactor) return
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
    // New windows open below the desktop's floating avatar.
    const defaultY = viewportBounds.minY + desktopAvatarRowHeight + (index % 3) * 32
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
