# BuckyOS Desktop 通用 UI 组件

> 整理自 desktop 现有实现，覆盖**控件 / 面板原语 / 视觉令牌 / 对话框 / 提权 / 样式类**，以及写内置 System App 必读的**桌面外壳与窗口框架**。
> Demo App（[`DemosAppPanel.tsx`](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx)）是控件类组件的活样板，写新 App 时优先照抄。
>
> **分层**：基础控件直接用 MUI（统一 theme，不要自造）；面板/布局用 desktop 自带原语；标题栏/侧边栏/状态栏由 shell 托管，App 只写内容面板；颜色/圆角/阴影一律走 `--cp-*` 设计令牌和 `shell-*` 样式类，不要写死颜色。
> 路径均相对本文件（`product/`）。
>
> **目录**：一~六 = App 内复用的组件与样式；七~八 = 桌面外壳/窗口框架与全局 Hook（写内置 App 看这里）。

---
## 零、系统通用对话框

第三方应用，可以通过通用对话框完成的系统功能

- 触发联合登陆 (/login)

下面是有正常权限后可以进一步实现的功能（细节参考Buckyos Sys Dlg.md)

- 文件选择对话框
- Share... (分享内容)
- 拉起App 安装/升级 （在current zone消费内容）
- 触发签名（包括sudo授权），目前仅钱包环境生效
- 触发支付确认




## 一、基础控件（MUI，已统一主题）

> **目的**：交互一致、自动适配明暗主题和移动端，不重复造轮子。**方式**：从 `@mui/material` 直接 import，主题由全局 ThemeProvider 注入。

| 组件 | 目的 | 参考例子 |
| --- | --- | --- |
| `Button` / `IconButton` | 主/次/文字按钮、纯图标按钮；支持 `startIcon`/`endIcon`/`disabled` | [DemosAppPanel.tsx:240-284](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L240) |
| `TextField` | 单行/多行（`multiline`）/下拉（`select`）输入；带 `InputAdornment` 前后缀 | [DemosAppPanel.tsx:312-363](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L312) |
| `Checkbox` / `Radio` / `Switch` | 多选 / 单选组 / 开关，统一用 `FormControlLabel` 包裹加文字 | [DemosAppPanel.tsx:372-429](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L372) |
| `Slider` | 数值区间拖拽选择 | [DemosAppPanel.tsx:436-441](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L436) |
| `Tabs` / `Tab` | 区域内分页切换；移动端用 `variant="scrollable"` | [DemosAppPanel.tsx:454-463](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L454) |
| `Menu` / `MenuItem` | 锚定弹出菜单（更多操作、上下文菜单） | [DemosAppPanel.tsx:295-309](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L295) |
| `Chip` | 状态/标签/筛选小药丸 | [DemosAppPanel.tsx:286-293](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L286) |
| `Alert` | info/success/warning/error 四档行内反馈 | [DemosAppPanel.tsx:466-472](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L466) |
| `LinearProgress` / `CircularProgress` | 进度条 / 转圈 loading（按钮内 loading 常用） | [DemosAppPanel.tsx:486-490](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L486) |

补充一些React的基础空间
- 实体卡片
- 文件（Named Object)卡片
- 文件 Review


---

## 二、面板与布局原语（desktop 自带）

> **目的**：让每个 App 面板有统一的标题区、分区和卡片节奏。**方式**：从 [`components/AppPanelPrimitives.tsx`](../src/frame/desktop/src/components/AppPanelPrimitives.tsx) import。

| 组件 | 目的 / 方式 | 参考例子 |
| --- | --- | --- |
| `PanelIntro` | App 面板顶部标题块：`kicker` + `title` + `body`，右侧可挂 `aside`。 | [DemosAppPanel.tsx:175-208](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L175) |
| `DemoSection` | 带标题/说明的内容分区卡片（`shell-subtle-panel` 外壳），承载一组控件。 | [DemosAppPanel.tsx:239-310](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L239) |
| `MetricCard` | 指标小卡：`label` + `value` + `tone`（accent/success/warning/neutral）。 | [DemosAppPanel.tsx:223-235](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L223) |

---

## 三、图标与视觉令牌

> **目的**：图标与品牌色面统一。**方式**：从 [`components/DesktopVisuals.tsx`](../src/frame/desktop/src/components/DesktopVisuals.tsx) / [`DesktopVisualTokens.ts`](../src/frame/desktop/src/components/DesktopVisualTokens.ts) import；通用图标直接用 `lucide-react`。

| 组件 / 工具 | 目的 / 方式 | 参考例子 |
| --- | --- | --- |
| `AppIcon` | 按 `iconKey` 渲染 App 图标（统一尺寸，内置 iconMap），兜底 `LayoutGrid`。 | [DemosAppPanel.tsx:186](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L186) |
| `TierBadge` | 显示 App 层级（system/sdk/...）的彩色徽章。 | [DesktopVisuals.tsx:46](../src/frame/desktop/src/components/DesktopVisuals.tsx#L46) |
| `appIconSurfaceStyle()` | 生成图标底色渐变（tile/window 两档），配 `--cp-accent-soft` 用。 | [DemosAppPanel.tsx:182-185](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L182) |
| `panelToneClasses` | accent/success/warning/neutral 四档配色类，徽章/标签复用。 | [DesktopVisualTokens.ts:1](../src/frame/desktop/src/components/DesktopVisualTokens.ts#L1) |
| `lucide-react` 图标 | 通用线性图标（`Check`/`Search`/`Bell`...），统一传 `size`。 | [DemosAppPanel.tsx:20-27](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L20) |

---

## 四、对话框 / 弹窗（窗口内模态）

> **目的**：App 在自己窗口内弹出模态/底部 sheet/全屏对话框并拿到返回值，移动端自动降级。**方式**：`const dlg = useWindowDialog(); const r = await dlg.open<T>({...})`，见 [`desktop/windows/dialogs.tsx`](../src/frame/desktop/src/desktop/windows/dialogs.tsx)。

| 能力 | 目的 / 方式 | 参考例子 |
| --- | --- | --- |
| `useWindowDialog().open()` | 打开对话框，`renderBody`/`renderActions` 自定义内容，Promise 返回结果。`presentation`: modal/sheet/fullscreen/auto；`size`: sm/md/lg。 | [DemosAppPanel.tsx:100-136](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L100) |
| `controls.close(result)` / `dismiss()` | body/actions 内回传结果或取消关闭。 | [DemosAppPanel.tsx:113-126](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L113) |
| fullscreen + 权限 | 全屏对话框受权限门控，越权抛 `WindowDialogPermissionError`，需 try/catch。 | [DemosAppPanel.tsx:138-171](../src/frame/desktop/src/app/demos/DemosAppPanel.tsx#L138) |

---

## 五、Sudo 提权对话框

> **目的**：执行敏感操作前用密码换取临时 sudo token（约 3 分钟有效）。**方式**：`const { requestSudo } = useSudo(); const grant = await requestSudo({ reason, aud })`，取消返回 `null`，见 [`components/sudo.tsx`](../src/frame/desktop/src/components/sudo.tsx)。

| 能力 | 目的 / 方式 | 参考例子 |
| --- | --- | --- |
| `useSudo()` / `useSudoByPassword()` | 弹出密码确认框，返回 `SudoGrant`（含 `sessionToken`/过期时间）。 | [sudo.tsx:359-408](../src/frame/desktop/src/components/sudo.tsx#L359) |
| `sudoByPassword()` | 纯函数版（非 Hook），直接调 verify-hub 换 token。 | [sudo.tsx:149-215](../src/frame/desktop/src/components/sudo.tsx#L149) |

---

## 六、样式类与设计令牌（CSS）

> **目的**：颜色、圆角、阴影、滚动条全局一致，自动适配明暗主题。**方式**：写 className 用 `shell-*`，配色用 `var(--cp-*)`（勿写死十六进制），定义见 [`src/index.css`](../src/frame/desktop/src/index.css)。

| 类 / 令牌 | 目的 / 用法 |
| --- | --- |
| `shell-panel` | App 主面板外壳（大圆角 + 渐变 + 阴影）。 |
| `shell-subtle-panel` | 次级分区卡片（`DemoSection` 即用此）。 |
| `shell-pill` / `shell-kicker` | 胶囊容器 / 大写小标题文字样式。 |
| `desktop-scrollbar` | 统一细滚动条（可滚动区加此类）。 |
| `--cp-text` / `--cp-muted` / `--cp-border` | 正文 / 次要文字 / 描边色。 |
| `--cp-surface` / `--cp-surface-2` / `--cp-bg` | 面板面 / 次级面 / 桌面背景色。 |
| `--cp-accent` / `--cp-accent-soft` | 品牌强调色 / 柔和强调色。 |
| `--cp-success` / `--cp-warning` / `--cp-danger` | 成功 / 警告 / 危险语义色。 |

---

## 七、桌面外壳 / 窗口框架（写内置 App 必读）

> 内置 System App 不用自己拼标题栏、侧边栏、状态栏——这些由 shell 统一提供。App 只写「内容面板」，外壳和窗口生命周期由框架托管。

### 7.1 App 接入约定

> **目的**：一个 App = 一个内容组件 + 一份 manifest，注册后由 shell 渲染进窗口。**方式**：写 `XxxAppPanel(props: AppContentLoaderProps)`，在 registry 注册 id→loader。

| 件 | 目的 / 方式 | 参考 |
| --- | --- | --- |
| `AppContentLoaderProps` | App 面板入参契约：`locale`/`themeMode`/`app`/`layoutState`/`onSaveSettings` 等，shell 注入。 | [app/types.ts:10](../src/frame/desktop/src/app/types.ts#L10) |
| `appLoaders` 注册表 | 在此把 `appId` 映射到面板组件；未注册自动落 `UnsupportedAppPanel`。 | [app/registry.tsx:24](../src/frame/desktop/src/app/registry.tsx#L24) |
| `AppDefinition` / `WindowManifest` | 声明 `iconKey`/`labelKey`/`accent`/`tier` 与窗口行为（默认模式、可否最大化/全屏、移动端状态栏模式、桌面初始尺寸）。 | [models/ui.ts:136](../src/frame/desktop/src/models/ui.ts#L136) |

### 7.2 外壳组件（由 shell 渲染，App 一般只配置不直接调）

| 组件 | 目的 / 方式 | 参考 |
| --- | --- | --- |
| `StatusBar` | 顶部状态栏：连接态/时钟/消息&通知托盘/App 菜单，桌面与移动端自适应。 | [StatusBar.tsx:330](../src/frame/desktop/src/desktop/StatusBar.tsx#L330) |
| `SystemSidebar` | 左侧系统抽屉：账户、回桌面、切换运行中 App、系统 App 列表、登出。 | [SystemSidebar.tsx:19](../src/frame/desktop/src/desktop/SystemSidebar.tsx#L19) |
| `StandaloneAppTitleBar` | 独立窗口/移动端的 App 标题栏（图标+标题+返回），`titleBarMode: 'custom'` 时用。 | [StandaloneAppTitleBar.tsx:17](../src/frame/desktop/src/desktop/StandaloneAppTitleBar.tsx#L17) |

### 7.3 移动端导航 Hook（App 内部直接用）

> **目的**：让 App 把「返回」和「动态标题」挂到 shell 的移动端状态栏上。**方式**：从 [`windows/MobileNavContext.tsx`](../src/frame/desktop/src/desktop/windows/MobileNavContext.tsx) import 的 Hook。

| Hook | 目的 / 方式 | 参考 |
| --- | --- | --- |
| `useMobileBackHandler(fn)` | 进入二级页时注册返回回调，回根页传 `null`；shell 顶栏自动显示返回箭头。 | [MobileNavContext.tsx:74](../src/frame/desktop/src/desktop/windows/MobileNavContext.tsx#L74) |
| `useMobileTitleOverride({title,subtitle})` | 用当前页内容（如路径）覆盖顶栏标题，传 `null` 还原静态标题。 | [MobileNavContext.tsx:88](../src/frame/desktop/src/desktop/windows/MobileNavContext.tsx#L88) |

### 7.4 桌面挂件（Widget）

> **目的**：桌面上的小卡片（时钟、便签等），区别于窗口化 App。**方式**：实现 `DesktopWidgetComponent`（入参 `DesktopWidgetProps`），由 `DesktopWidgetRenderer` 渲染。

| 件 | 目的 / 方式 | 参考 |
| --- | --- | --- |
| `DesktopWidgetProps` / `DesktopWidgetComponent` | 挂件契约：`item` + `onSaveNote`。 | [widgets/types.ts:1](../src/frame/desktop/src/desktop/widgets/types.ts#L1) |
| `ClockWidget` / `NotepadWidget` | 现成挂件样例（时钟、便签）。 | [widgets/ClockWidget.tsx](../src/frame/desktop/src/desktop/widgets/ClockWidget.tsx) |

---

## 八、全局上下文 Hook

> **目的**：文案国际化与主题，跨所有组件可用。**方式**：从对应 provider import，组件树已在根部包好 Provider。

| Hook | 目的 / 方式 | 参考 |
| --- | --- | --- |
| `useI18n()` | `t(key, fallback?, vars?)` 取文案 + 当前 `locale`；所有可见文字必须走它。 | [i18n/provider.tsx:62](../src/frame/desktop/src/i18n/provider.tsx#L62) |
| `useThemeMode()` | 读/切明暗主题（`themeMode` / `setThemeMode`）。 | [theme/provider.tsx:278](../src/frame/desktop/src/theme/provider.tsx#L278) |
| `useWindowDialog()` | 窗口内对话框（见第四节）。 | [windows/dialogs.tsx:314](../src/frame/desktop/src/desktop/windows/dialogs.tsx#L314) |
| `useSudo()` | Sudo 提权（见第五节）。 | [components/sudo.tsx:410](../src/frame/desktop/src/components/sudo.tsx#L410) |

---

## 约定速查

- **配色**：永远走 `var(--cp-*)`，需要混色用 `color-mix(in srgb, var(--cp-...) N%, ...)`。
- **基础控件**：能用 MUI 就用 MUI，别自造同类组件。
- **面板结构**：`PanelIntro`（标题）→ `MetricCard`（指标行）→ 多个 `DemoSection`（分区）。
- **文案**：所有可见文字走 `useI18n()` 的 `t(key, fallback)`，不硬编码。
- **响应式**：`useMediaQuery` 判断窄屏，关键交互在移动端要有 fallback（见 demo 的全屏按钮）。
