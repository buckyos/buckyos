import { z } from 'zod'

export const supportedLocales = [
  'en',
  'zh-CN',
  'ja',
  'ko',
  'fr',
  'de',
  'es',
  'ar',
] as const

export type SupportedLocale = (typeof supportedLocales)[number]
export type ThemeMode = 'light' | 'dark'
export type RuntimeContainer = 'browser' | 'desktop-app' | 'mobile-app'
export type FormFactor = 'desktop' | 'mobile'
export type MockScenario = 'normal' | 'empty' | 'error'
export type DesktopItemType = 'app' | 'widget'

/**
 * Placement provenance — tracks how an item ended up at its current slot.
 * - `manual`: user explicitly dragged/placed it
 * - `auto`: system auto-assigned (e.g. newly installed app)
 * - `reflow`: system re-placed due to resize / init recovery
 */
export type PlacementType = 'manual' | 'auto' | 'reflow'
export type DesktopWallpaperMode = 'panorama' | 'tile' | 'infinite'
export type WidgetType = string
export type DisplayMode = 'windowed' | 'maximized' | 'fullscreen'
export type WindowState = 'windowed' | 'maximized' | 'minimized'
export type LoadingState = 'idle' | 'loading' | 'success' | 'error'
export type IntegrationTier = 'system' | 'sdk' | 'external'
export type MobileStatusBarMode = 'compact' | 'standard'

export interface DeadZone {
  top: number
  bottom: number
  left: number
  right: number
}

export interface LayoutItemBase {
  id: string
  type: DesktopItemType
  /** Grid column. `undefined` = unpositioned (auto-placed at page tail). */
  x?: number
  /** Grid row. `undefined` = unpositioned (auto-placed at page tail). */
  y?: number
  w: number
  h: number
  /**
   * Linear slot index within the page (column-major for desktop, row-major for mobile).
   * `undefined` means unpositioned — will be auto-placed at page tail.
   * This is the **source of truth**; x/y are derived from it.
   */
  slotIndex?: number
  /** The page this item prefers to live on (used during reflow). */
  preferredPage?: number
  /** How this item was placed at its current slot. */
  placementType?: PlacementType
  /** Stable ordering — e.g. install order or first-appear order. */
  seq?: number
}

export interface AppIconItem extends LayoutItemBase {
  type: 'app'
  appId: string
}

export interface WidgetItem extends LayoutItemBase {
  type: 'widget'
  widgetType: WidgetType
  config: Record<string, unknown>
}

export type LayoutItem = AppIconItem | WidgetItem

export interface DesktopPageState {
  id: string
  items: LayoutItem[]
}

export interface LayoutState {
  version: number
  formFactor: FormFactor
  deadZone: DeadZone
  pages: DesktopPageState[]
}

export interface DesktopWallpaper {
  mode: DesktopWallpaperMode
  imageUrl?: string
  tileSize?: number
}

export interface WindowAppearancePreferences {
  titleBarOpacity: number
  backgroundOpacity: number
}

export const defaultWindowAppearancePreferences: WindowAppearancePreferences = {
  titleBarOpacity: 100,
  backgroundOpacity: 100,
}

export const windowAppearancePreferencesSchema = z.object({
  titleBarOpacity: z.coerce.number().int().min(0).max(100),
  backgroundOpacity: z.coerce.number().int().min(0).max(100),
})

export interface DesktopWindowSizing {
  width: number
  height: number
  minWidth?: number
  minHeight?: number
}

export interface WindowManifest {
  defaultMode: DisplayMode
  allowMinimize: boolean
  allowMaximize: boolean
  allowClose: boolean
  allowFullscreen: boolean
  mobileFullscreenBehavior: 'cover_dead_zone' | 'keep_dead_zone'
  mobileStatusBarMode: MobileStatusBarMode
  titleBarMode: 'system' | 'custom'
  placement: 'inplace' | 'new-container'
  contentPadding?: 'default' | 'none'
  mobileRedirectPath?: string
  desktopWindow?: DesktopWindowSizing
  supportedFormFactors?: FormFactor[]
  showInLauncher?: boolean
}

export interface AppDefinition {
  id: string
  iconKey: string
  labelKey: string
  summaryKey: string
  accent: string
  tier: IntegrationTier
  manifest: WindowManifest
}

export function supportsFormFactor(
  app: AppDefinition,
  formFactor: FormFactor,
) {
  return app.manifest.supportedFormFactors?.includes(formFactor) ?? true
}

export function isLauncherApp(app: AppDefinition) {
  return app.manifest.showInLauncher ?? true
}

export interface DesktopPayload {
  overview: {
    titleKey: string
    subtitleKey: string
  }
  wallpaper: DesktopWallpaper
  apps: AppDefinition[]
  layout: LayoutState
}

export interface DataState<T> {
  status: LoadingState
  data: T | null
  error: string | null
}

export interface SystemSidebarAppItem {
  appId: string
  iconKey: string
  labelKey: string
}

export interface SystemSidebarSwitchAppItem extends SystemSidebarAppItem {
  minimizedOrder: number
}

export interface SystemSidebarDataModel {
  currentAppId?: string
  runningAppCount: number
  switchApps: SystemSidebarSwitchAppItem[]
  systemApps: SystemSidebarAppItem[]
}

export const systemPreferencesInputSchema = z.object({
  locale: z.enum(supportedLocales),
  theme: z.enum(['light', 'dark']),
  runtimeContainer: z.enum(['browser', 'desktop-app', 'mobile-app']),
  deadZoneTop: z.coerce.number().int().min(0).max(96),
  deadZoneBottom: z.coerce.number().int().min(0).max(120),
  deadZoneLeft: z.coerce.number().int().min(0).max(72),
  deadZoneRight: z.coerce.number().int().min(0).max(72),
  titleBarOpacity: z.coerce.number().int().min(0).max(100),
  backgroundOpacity: z.coerce.number().int().min(0).max(100),
})

export type SystemPreferencesInput = z.infer<typeof systemPreferencesInputSchema>

export const noteInputSchema = z.object({
  content: z
    .string()
    .trim()
    .min(1)
    .max(180),
})

export type NoteInput = z.infer<typeof noteInputSchema>

export interface WindowRecord {
  id: string
  appId: string
  state: WindowState
  minimizedOrder: number | null
  titleKey: string
  x: number
  y: number
  width: number
  height: number
  zIndex: number
}

// =========================================================================
// Desktop UI DataModel — 需要同步的 5 个数据分组
// =========================================================================

/**
 * Group 1: 外观设置
 * 语言、颜色主题、字体大小等全局外观配置。
 *
 * - locale / themeMode 由各自的 Provider 管理（I18nProvider, ThemeProvider）
 * - 本分组收集其余外观相关的持久化设置
 */
export interface AppearanceSettings {
  /** 运行容器类型（影响连接状态显示等） */
  runtimeContainer: RuntimeContainer
  /** 桌面壁纸配置 */
  wallpaper: DesktopWallpaper
  /** 安全区域 / 边距 */
  deadZone: DeadZone
  /** 窗口半透明度偏好 */
  windowAppearance: WindowAppearancePreferences
}

/**
 * Group 2: 窗口布局设置
 * 窗口关闭时记录位置和大小，重新打开时作为参考。
 * 以 appId 为 key 存储每个 App 上次的窗口几何信息。
 */
export interface WindowLayoutSettings {
  /** appId → 上次关闭时的几何信息 */
  geometryByApp: Record<string, WindowGeometry>
}

/** 单个窗口的几何信息 */
export interface WindowGeometry {
  x: number
  y: number
  width: number
  height: number
}

/**
 * Group 3: AppItem 配置
 * 桌面上要显示哪些 AppItem（应用列表 + 元信息）。
 * 来源于服务端 payload，按 formFactor 过滤后得到。
 */
export interface AppItemConfig {
  apps: AppDefinition[]
}

/**
 * Group 4: AppItem 布局设置
 * 每个 App 图标在桌面网格中的位置。
 * 规则：如果查询不到布局（x/y 为 undefined），通过本地逻辑自动分配位置。
 */
export interface AppItemLayoutSettings {
  pages: AppItemPageLayout[]
}

export interface AppItemPageLayout {
  pageId: string
  items: AppIconItem[]
}

/**
 * Group 5: 小部件布局配置
 * 小部件在桌面网格中的位置与自身配置。
 * 规则：如果小部件没有布局配置（不在列表中），则不展示。
 */
export interface WidgetLayoutSettings {
  pages: WidgetPageLayout[]
}

export interface WidgetPageLayout {
  pageId: string
  items: WidgetItem[]
}

/**
 * 5 个同步数据分组的聚合接口。
 * DesktopUIStore 将在 init 时构建此对象，并在变更时持久化 / 同步。
 */
export interface DesktopSyncData {
  /** Group 1: 外观设置 */
  appearance: AppearanceSettings
  /** Group 2: 窗口布局设置 */
  windowLayout: WindowLayoutSettings
  /** Group 3: AppItem 配置 */
  appItemConfig: AppItemConfig
  /** Group 4: AppItem 布局设置 */
  appItemLayout: AppItemLayoutSettings
  /** Group 5: 小部件布局配置 */
  widgetLayout: WidgetLayoutSettings
}
