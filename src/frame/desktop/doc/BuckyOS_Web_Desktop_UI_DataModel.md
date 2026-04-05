# BuckyOS Web Desktop UI DataModel

## 1. Overview

- Service: `BuckyOS Web Desktop`
- Prototype scope:
  - Desktop Layout System
  - Window System
  - Theme and locale switching
  - Mock-first app launcher and widgets
- Source PRD: [BuckyOS_Web_Desktop_需求文档.md](/Users/liuzhicong/project/buckyos_desktop/doc/BuckyOS_Web_Desktop_需求文档.md)
- Source implementation:
  - [ui.ts](/Users/liuzhicong/project/buckyos_desktop/src/models/ui.ts)
  - [data.ts](/Users/liuzhicong/project/buckyos_desktop/src/mock/data.ts)
  - [provider.ts](/Users/liuzhicong/project/buckyos_desktop/src/mock/provider.ts)

## 2. DataModel Definitions

### 2.1 Layout Model

```ts
export type FormFactor = 'desktop' | 'mobile';
export type DesktopItemType = 'app' | 'widget';
export type WidgetType = 'clock' | 'notepad';

export interface DeadZone {
  top: number;
  bottom: number;
  left: number;
  right: number;
}

export interface LayoutItemBase {
  id: string;
  type: DesktopItemType;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface AppIconItem extends LayoutItemBase {
  type: 'app';
  appId: string;
}

export interface WidgetItem extends LayoutItemBase {
  type: 'widget';
  widgetType: WidgetType;
  config: Record<string, unknown>;
}

export type LayoutItem = AppIconItem | WidgetItem;

export interface DesktopPageState {
  id: string;
  items: LayoutItem[];
}

export interface LayoutState {
  version: number;
  formFactor: FormFactor;
  deadZone: DeadZone;
  pages: DesktopPageState[];
}
```

Notes:

- `LayoutState` is the stable top-level persistence object.
- `formFactor` is part of the storage boundary, not just a rendering hint.
- `deadZone` affects both desktop placement and window workspace bounds.

### 2.2 Window Model

```ts
export type DisplayMode = 'windowed' | 'maximized' | 'fullscreen';
export type WindowState = 'windowed' | 'maximized' | 'minimized';
export type IntegrationTier = 'system' | 'sdk' | 'external';

export interface WindowManifest {
  defaultMode: DisplayMode;
  allowMinimize: boolean;
  allowMaximize: boolean;
  allowClose: boolean;
  allowFullscreen: boolean;
  mobileFullscreenBehavior: 'cover_dead_zone' | 'keep_dead_zone';
  titleBarMode: 'system' | 'custom';
  placement: 'inplace' | 'new-container';
}

export interface AppDefinition {
  id: string;
  iconKey: string;
  labelKey: string;
  summaryKey: string;
  accent: string;
  tier: IntegrationTier;
  manifest: WindowManifest;
}

export interface WindowRecord {
  id: string;
  appId: string;
  state: WindowState;
  minimizedOrder: number | null;
  titleKey: string;
  x: number;
  y: number;
  width: number;
  height: number;
  zIndex: number;
}
```

Notes:

- `AppDefinition.manifest` is the UI-side contract used to resolve open behavior.
- `WindowRecord` is ephemeral runtime state and is not currently persisted.
- `minimizedOrder` is an explicit UI ordering field used by the sidebar switcher.

### 2.3 Sidebar UI Model

```ts
export interface SystemSidebarAppItem {
  appId: string;
  iconKey: string;
  labelKey: string;
}

export interface SystemSidebarSwitchAppItem extends SystemSidebarAppItem {
  minimizedOrder: number;
}

export interface SystemSidebarDataModel {
  currentAppId?: string;
  runningAppCount: number;
  switchApps: SystemSidebarSwitchAppItem[];
  systemApps: SystemSidebarAppItem[];
}
```

Notes:

- `switchApps` contains only minimized, launcher-scope apps.
- `switchApps` is sorted ascending by `minimizedOrder`, so the list reads top-to-bottom in minimize sequence.
- `systemApps` is a fixed utility group for always-available shell entries such as Settings and Diagnostics.

### 2.4 Data Fetch Model

```ts
export type MockScenario = 'normal' | 'empty' | 'error';
export type LoadingState = 'idle' | 'loading' | 'success' | 'error';

export interface DesktopPayload {
  overview: {
    titleKey: string;
    subtitleKey: string;
  };
  apps: AppDefinition[];
  layout: LayoutState;
}

export interface DataState<T> {
  status: LoadingState;
  data: T | null;
  error: string | null;
}
```

## 3. Input Models

### 3.1 Settings Form

Used by the Settings app window.

```ts
export const systemPreferencesInputSchema = z.object({
  locale: z.enum(['en', 'zh-CN', 'ja', 'ko', 'fr', 'de', 'es', 'ar']),
  theme: z.enum(['light', 'dark']),
  runtimeContainer: z.enum(['browser', 'desktop-app', 'mobile-app']),
  deadZoneTop: z.number().int().min(0).max(96),
  deadZoneBottom: z.number().int().min(0).max(120),
  deadZoneLeft: z.number().int().min(0).max(72),
  deadZoneRight: z.number().int().min(0).max(72),
});

export type SystemPreferencesInput = z.infer<typeof systemPreferencesInputSchema>;
```

Rules:

- Locale is restricted to eight system languages.
- Theme is system-level `light | dark`.
- Runtime container is a UI-only selector used to simulate Browser/Desktop/Mobile shell behavior.
- Dead zone values are constrained to explicit integer ranges.

Default sample:

```ts
{
  locale: 'en',
  theme: 'light',
  runtimeContainer: 'browser',
  deadZoneTop: 24,
  deadZoneBottom: 28,
  deadZoneLeft: 20,
  deadZoneRight: 20
}
```

Invalid sample:

```ts
{
  locale: 'it',
  theme: 'system',
  runtimeContainer: 'tablet-app',
  deadZoneTop: -1,
  deadZoneBottom: 240,
  deadZoneLeft: 100,
  deadZoneRight: 100
}
```

### 3.2 Notepad Widget Input

```ts
export const noteInputSchema = z.object({
  content: z.string().trim().min(1).max(180),
});

export type NoteInput = z.infer<typeof noteInputSchema>;
```

Default sample:

```ts
{ content: 'Stage Title Bar defaults, window manifest, and layout scope split.' }
```

Invalid sample:

```ts
{ content: '' }
```

## 4. State Definitions

Each primary data view supports the following states:

```ts
export type LoadingState = 'idle' | 'loading' | 'success' | 'error';

export interface DataState<T> {
  status: LoadingState;
  data: T | null;
  error: string | null;
}
```

Applied views:

- Desktop shell payload
  - Loading: centered loading card
  - Success: pages/widgets/apps render
  - Empty: empty layout card with restore action
  - Error: retry card
- Window runtime
  - Normal: window visible in desktop or mobile title-bar container
  - Minimized: removed from front layer but preserved in running set
  - Maximized: occupies available workspace
- Widget editor
  - Normal: note content visible
  - Validation error: submit blocked by `noteInputSchema`

## 5. Pagination & Aggregation

- Pagination model: page-based desktop layout, not list pagination.
- Layout pages:
  - Desktop default: 2 pages
  - Mobile default: 2 pages
- Placement:
  - Items are stored with explicit `x/y/w/h`.
  - Cross-page movement creates a new page if the target page does not exist.
- Aggregation used in UI:
  - `totalItems = sum(page.items.length)`
  - `currentPage + 1 / pageCount`

## 6. Field Stability Classification

| Field | Stability | Notes |
|---|---|---|
| `LayoutState.version` | Frozen | Required for migration and compatibility. |
| `LayoutState.formFactor` | Frozen | Defines storage split and rendering contract. |
| `LayoutState.pages` | Frozen | Core desktop layout container. |
| `LayoutItem.id` | Frozen | Stable identity for drag, save, and widget config updates. |
| `AppIconItem.appId` | Frozen | Stable launcher-to-app mapping. |
| `WidgetItem.widgetType` | Frozen | Shared widget rendering contract. |
| `WidgetItem.config` | Extensible | Widget-specific configuration can evolve. |
| `AppDefinition.manifest` | Extensible | Window feature flags may expand. |
| `WindowRecord.minimizedOrder` | Volatile | Sidebar-only ordering metadata for minimized apps. |
| `WindowRecord.x/y/width/height` | Volatile | Runtime-only state; not yet persisted. |
| `DesktopPayload.overview` | Volatile | Presentational shell metadata. |

## 7. Mock Data Contract

### 7.1 Normal State

- Desktop payload returns:
  - App catalog with six mock apps
  - Two desktop pages
  - Two mobile pages
  - Default widgets: `clock`, `notepad`
- Example desktop items:

```ts
{
  id: 'widget-clock',
  type: 'widget',
  widgetType: 'clock',
  x: 0,
  y: 0,
  w: 2,
  h: 1,
  config: {}
}

{
  id: 'app-settings',
  type: 'app',
  appId: 'settings',
  x: 2,
  y: 0,
  w: 1,
  h: 1
}
```

### 7.2 Empty State

```ts
{
  version: 1,
  formFactor: 'desktop',
  deadZone: { top: 24, bottom: 28, left: 20, right: 20 },
  pages: [{ id: 'desktop-page-1', items: [] }]
}
```

### 7.3 Error State

- Provider throws `mock.provider.desktop_unavailable`.
- UI maps it to the retryable error card state.

## 8. Storage Scope

- Desktop layout key: `buckyos.layout.desktop.v1`
- Mobile layout key: `buckyos.layout.mobile.v1`
- Theme key: `buckyos.prototype.theme.v1`
- Locale key: `buckyos.prototype.locale.v1`
- Runtime container key: `buckyos.prototype.runtime.v1`

## 9. KRPC Mapping Notes

Current prototype is intentionally zero-backend. Expected integration notes:

- `LayoutState` should map to a user-scoped layout document.
- `AppDefinition.manifest` can map to manifest metadata delivered by system registry or app bundle config.
- `WindowRecord` should remain client runtime state unless a future session-restore feature requires persistence.
