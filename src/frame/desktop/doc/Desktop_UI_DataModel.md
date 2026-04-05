# BuckyOS Web Desktop UI Data Model

## 1. 文档说明

- 服务：`BuckyOS Web Desktop`
- 当前原型范围：
  - 桌面图标/Widget 布局
  - 多窗口系统
  - 主题、语言、运行容器切换
  - 移动端重定向与状态栏模式
  - mock-first 的应用目录与默认桌面数据
- 本文以当前实现为准，不再描述已经下线或尚未落地的理想模型。

参考实现：

- 需求文档：[BuckyOS_Web_Desktop_需求文档.md](./BuckyOS_Web_Desktop_需求文档.md)
- 核心类型：[ui.ts](../src/models/ui.ts)
- mock 数据：[data.ts](../src/mock/data.ts)
- mock Provider：[provider.ts](../src/mock/provider.ts)
- 桌面路由/状态编排：[DesktopRoute.tsx](../src/desktop/DesktopRoute.tsx)
- 窗口模型辅助：[model.ts](../src/desktop/windows/model.ts)
- 应用注册：[registry.tsx](../src/app/registry.tsx)

## 2. 顶层数据与持久化边界

### 2.1 顶层 Payload

当前桌面数据入口是 `DesktopPayload`：

```ts
export interface DesktopPayload {
  overview: {
    titleKey: string
    subtitleKey: string
  }
  wallpaper: DesktopWallpaper
  apps: AppDefinition[]
  layout: LayoutState
}
```

说明：

- `overview` 目前只提供标题和副标题文案 key。
- `wallpaper` 已经进入 payload，旧文档遗漏了这一层。
- `apps` 是当前 form factor 可用的应用目录，不一定等于全部注册应用。
- `layout` 是桌面图标与 Widget 的默认布局。

### 2.2 当前本地持久化边界

当前实现中，桌面壳会把以下状态持久化到 `localStorage`：

- `buckyos.prototype.locale.v1`：语言
- `buckyos.prototype.theme.v1`：主题
- `buckyos.prototype.runtime.v1`：运行容器（`browser | desktop-app | mobile-app`）
- `buckyos.layout.desktop.v1` / `buckyos.layout.mobile.v1`：按 form factor 分开的布局状态
- `buckyos.window-appearance.v1`：窗口标题栏/背景透明度
- `buckyos.window-geometry.desktop.v1`：按 `appId` 记录的桌面窗口几何信息

不持久化的运行态数据：

- `WindowRecord[]` 本身不直接落盘
- `SystemSidebarDataModel` 全部为运行时派生
- SWR 加载状态与错误状态不持久化

## 3. 核心类型定义

### 3.1 基础枚举

```ts
export type SupportedLocale =
  | 'en'
  | 'zh-CN'
  | 'ja'
  | 'ko'
  | 'fr'
  | 'de'
  | 'es'
  | 'ar'

export type ThemeMode = 'light' | 'dark'
export type RuntimeContainer = 'browser' | 'desktop-app' | 'mobile-app'
export type FormFactor = 'desktop' | 'mobile'
export type MockScenario = 'normal' | 'empty' | 'error'
export type LoadingState = 'idle' | 'loading' | 'success' | 'error'
export type DesktopItemType = 'app' | 'widget'
export type WidgetType = string
export type DesktopWallpaperMode = 'panorama' | 'tile' | 'infinite'
export type DisplayMode = 'windowed' | 'maximized' | 'fullscreen'
export type WindowState = 'windowed' | 'maximized' | 'minimized'
export type IntegrationTier = 'system' | 'sdk' | 'external'
export type MobileStatusBarMode = 'compact' | 'standard'
```

说明：

- `WidgetType` 现在是开放字符串，不再限制为 `clock | notepad`。
- 但当前 `DesktopWidgetRenderer` 只注册了 `clock` 和 `notepad`，其他类型会显示 unsupported fallback。
- `DisplayMode` 包含 `fullscreen`，但 `WindowRecord.state` 目前没有 `fullscreen`；窗口运行态仍只有 `windowed | maximized | minimized`。

### 3.2 布局模型

```ts
export interface DeadZone {
  top: number
  bottom: number
  left: number
  right: number
}

export interface LayoutItemBase {
  id: string
  type: DesktopItemType
  x?: number
  y?: number
  w: number
  h: number
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
```

说明：

- `LayoutState` 仍是桌面布局的稳定持久化对象。
- `x` / `y` 现在允许省略，表示“未定位项”。
- 未定位项会在渲染前通过 `resolveLayout()` 自动放到页尾：
  - 桌面：`col-major`，先上到下，再左到右
  - 移动：`row-major`，先左到右，再上到下
- 当网格尺寸缩小导致项目越界时，`invalidatePositions()` 会把该项目改回未定位状态。
- `deadZone` 既影响桌面图标工作区，也影响窗口工作区和侧边栏安全区域。

### 3.3 壁纸与窗口外观模型

```ts
export interface DesktopWallpaper {
  mode: DesktopWallpaperMode
  imageUrl?: string
  tileSize?: number
}

export interface WindowAppearancePreferences {
  titleBarOpacity: number
  backgroundOpacity: number
}

export const defaultWindowAppearancePreferences = {
  titleBarOpacity: 100,
  backgroundOpacity: 100,
}
```

说明：

- `DesktopPayload.wallpaper` 当前默认值是 `{ mode: 'infinite' }`。
- 窗口外观偏好已独立出 `WindowAppearancePreferences`，不再只是设置页里的临时表单值。

### 3.4 应用目录与窗口清单

```ts
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
```

说明：

- `mobileStatusBarMode` 是新增且已实际生效的字段，用于控制移动端状态栏高度与内容。
- `contentPadding` 已用于窗口容器，`none` 表示内容全出血。
- `mobileRedirectPath` 已实际生效；在移动端点击应用时，如果存在该字段，会直接导航而不是开窗口。
- `desktopWindow` 定义桌面端窗口默认宽高与最小尺寸。
- `supportedFormFactors` / `showInLauncher` 已被桌面逻辑消费，但当前 mock `appCatalog` 里还没有显式设置这些字段，默认行为分别是：
  - `supportedFormFactors` 未设置时，视为同时支持 desktop/mobile
  - `showInLauncher` 未设置时，视为显示在 launcher

### 3.5 窗口运行态模型

```ts
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
```

配套派生模型：

```ts
export interface DesktopWindowDataModel extends WindowRecord {
  app: DesktopAppItem
}

export interface DesktopWindowLayerDataModel {
  windows: DesktopWindowDataModel[]
  topWindow?: DesktopWindowDataModel
}
```

说明：

- `createWindowRecord()` 会根据 `manifest.defaultMode` 生成初始 `state`：
  - `defaultMode === 'windowed'` 时为 `windowed`
  - 否则一律落到 `maximized`
- 因此 `fullscreen` 目前仍是 manifest 能力声明，不是单独持久化运行态。
- `minimizedOrder` 用于侧边栏切换顺序。
- 几何信息会按 `appId` 写入 `buckyos.window-geometry.desktop.v1`，下次重新打开同一 app 时复用。

### 3.6 侧边栏派生模型

```ts
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
```

说明：

- `switchApps` 来自最小化窗口，按 `minimizedOrder` 升序排列。
- 当前实现只把 `settings` 和 `diagnostics` 排除在 `switchApps` 外。
- `systemApps` 当前固定尝试展示 `settings`、`diagnostics`、`users-agents`。
- 也就是说，`users-agents` 既可能出现在固定系统区，也可能因为被最小化而出现在切换区；文档需要按这一真实行为记录。

### 3.7 通用加载态

```ts
export interface DataState<T> {
  status: LoadingState
  data: T | null
  error: string | null
}
```

## 4. 当前 mock 默认数据

### 4.1 默认 Dead Zone 与 Wallpaper

```ts
const defaultDeadZone = {
  top: 0,
  bottom: 8,
  left: 5,
  right: 5,
}

const defaultWallpaper = {
  mode: 'infinite',
}
```

补充：

- `DesktopRoute` 内含历史 dead zone 迁移逻辑，旧值会迁移到当前默认值。

### 4.2 当前应用目录

当前 `appCatalog` 共 14 个应用：

| appId | tier | 备注 |
| --- | --- | --- |
| `ai-center` | `system` | `contentPadding: 'none'` |
| `settings` | `system` | `contentPadding: 'none'` |
| `files` | `sdk` | 移动端重定向到 `/files` |
| `studio` | `sdk` | 支持 `allowFullscreen: true` |
| `market` | `system` | 普通窗口应用 |
| `diagnostics` | `system` | 固定系统应用之一 |
| `demos` | `sdk` | 普通窗口应用 |
| `codeassistant` | `sdk` | 移动端重定向到 `/messagehub?entityId=agent-coder` |
| `messagehub` | `system` | 移动端重定向到 `/messagehub` |
| `homestation` | `system` | 移动端重定向到 `/homestation` |
| `users-agents` | `system` | 固定系统应用之一 |
| `task-center` | `system` | 移动端重定向到 `/taskcenter` |
| `app-service` | `system` | `contentPadding: 'none'` |
| `docs` | `external` | 特殊：`defaultMode: 'maximized'`、`placement: 'new-container'`、`titleBarMode: 'custom'` |

### 4.3 当前默认布局

桌面端默认布局：

- 共 2 页。
- 第 1 页包含：
  - `clock` widget
  - `settings` / `files` / `studio` / `market` / `docs` / `demos` / `codeassistant` / `messagehub`
  - `ai-center` / `homestation`
  - `notepad` widget
- 第 2 页包含：
  - `diagnostics` / `users-agents` / `task-center` / `app-service`

移动端默认布局：

- 共 2 页。
- 第 1 页包含 1 个 `clock` widget 和 14 个应用入口。
- 第 2 页包含 1 个 `notepad` widget。

补充：

- `buildDesktopPayload()` 会先按 form factor 过滤应用，再基于 `showInLauncher` 过滤布局里的 app item。
- 如果是 `empty` 场景，只返回 1 个空页面，不返回默认图标/Widget。
- 当默认布局新增了 launcher app，而用户本地布局里还没有时，`reconcileLayoutWithDefaultApps()` 会把新增 app 以“未定位项”追加到最后一页。

## 5. 输入模型

### 5.1 系统偏好输入

当前实际 schema：

```ts
export const systemPreferencesInputSchema = z.object({
  locale: z.enum(['en', 'zh-CN', 'ja', 'ko', 'fr', 'de', 'es', 'ar']),
  theme: z.enum(['light', 'dark']),
  runtimeContainer: z.enum(['browser', 'desktop-app', 'mobile-app']),
  deadZoneTop: z.coerce.number().int().min(0).max(96),
  deadZoneBottom: z.coerce.number().int().min(0).max(120),
  deadZoneLeft: z.coerce.number().int().min(0).max(72),
  deadZoneRight: z.coerce.number().int().min(0).max(72),
  titleBarOpacity: z.coerce.number().int().min(0).max(100),
  backgroundOpacity: z.coerce.number().int().min(0).max(100),
})
```

说明：

- 旧文档缺少 `titleBarOpacity` 和 `backgroundOpacity`。
- `z.coerce.number()` 表示表单字符串输入也会先被转成数字再校验。
- `applySettings()` 会同时更新：
  - 语言
  - 主题
  - 运行容器
  - 布局中的 `deadZone`
  - 窗口外观偏好

默认示例：

```ts
{
  locale: 'en',
  theme: 'light',
  runtimeContainer: 'browser',
  deadZoneTop: 0,
  deadZoneBottom: 8,
  deadZoneLeft: 5,
  deadZoneRight: 5,
  titleBarOpacity: 100,
  backgroundOpacity: 100,
}
```

非法示例：

```ts
{
  locale: 'it',
  theme: 'system',
  runtimeContainer: 'tablet-app',
  deadZoneTop: -1,
  deadZoneBottom: 240,
  deadZoneLeft: 100,
  deadZoneRight: 100,
  titleBarOpacity: 120,
  backgroundOpacity: -5,
}
```

### 5.2 Notepad Widget 输入

```ts
export const noteInputSchema = z.object({
  content: z.string().trim().min(1).max(180),
})
```

默认示例：

```ts
{ content: 'Review drag semantics, dead zone behavior, and window polish.' }
```

非法示例：

```ts
{ content: '' }
```

## 6. 当前实现备注

- `fetchDesktopPayload()` 当前只是 mock provider，正常场景延迟约 `360ms`，错误/空态场景约 `420ms`。
- `resolveDesktopApps()` 会把 `AppDefinition[]` 映射为带 `loader` 的 `DesktopAppItem[]`；找不到 loader 的应用会回退到 `UnsupportedAppPanel`。
- `docs` 是当前唯一明显的“外部容器型”应用：`placement: 'new-container'`，且默认最大化。
- 窗口打开后，桌面端支持按视口和 dead zone 重新规范化几何信息；移动端则主要走全屏 sheet/redirect 行为。
- 这个文档只覆盖桌面壳 UI 模型，不覆盖各子应用内部各自的 mock store 和领域数据模型。
