# BuckyOS Web Desktop App SDK（最小文档）

本文档面向 App 开发者，总结当前仓库里已经存在、且相对稳定的最小接入套路。

目标不是定义一个完整 SDK，而是回答三个实际问题：

1. 一个 App 怎么同时支持独立页面运行和 Desktop 内嵌运行
2. 一个 App 在 Desktop 里到底能依赖哪些宿主能力
3. 当前哪些能力只是字段存在，哪些能力已经真正落地

## 1. 当前推荐模型

当前最推荐的模型不是“直接写一个 Panel”，而是把 App 拆成三层：

1. `AppView`
   业务 UI 本体，尽量不依赖 Desktop 宿主
2. `PanelAdapter`
   Desktop 内嵌适配层，把宿主传进来的 `AppContentLoaderProps` 转成业务 View 需要的 props
3. `RouteAdapter`
   独立页面适配层，把 URL、Router、页面级导航转成业务 View 需要的 props

上面两个Adapter至少要实现一个。
应用可以实现多个PanelAdapter，但通常只有一个RouteAdapter

`MessageHub` 是当前最接近这个模型的实现：

- `MessageHubView` 是业务 UI 本体
- `MessageHubAppPanel` 用于 Desktop 内嵌
- `MessageHubRoute` 用于独立页面

结论：

- `Panel` 不等于 `Window`
- `Panel` 更像 Desktop 宿主下的适配器
- 真正应该长期复用的是 `AppView`

## 2. 最小目录形态

推荐一个新 App 采用类似结构：

```text
src/app/myapp/
  MyAppView.tsx
  MyAppAppPanel.tsx
  MyAppRoute.tsx
  types.ts
  mock/
```

职责建议：

- `MyAppView.tsx`
  只表达业务界面和业务状态
- `MyAppAppPanel.tsx`
  处理 Desktop 嵌入时的宿主输入
- `MyAppRoute.tsx`
  处理独立页面 URL 参数、路由跳转、页面级导航

## 3. Desktop 宿主给 App 的最小输入

当前 Desktop 宿主会给 Panel 传入下面这些字段：

```ts
export interface AppContentLoaderProps {
  activityLog: string[]
  app: AppDefinition
  layoutState: LayoutState
  locale: string
  onSaveSettings: (values: SystemPreferencesInput) => void
  runtimeContainer: string
  themeMode: ThemeMode
}
```

这些字段更适合被看作“宿主上下文”，而不是业务模型本身。

建议：

- 不要让 `AppView` 直接依赖整个 `AppContentLoaderProps`
- 在 `PanelAdapter` 中把宿主字段翻译成业务 props
- 只有确实与宿主强相关的功能，才在 Panel 层处理

## 4. 当前存在的 UI 容器

当前实现里，真正承载 App 内容的 UI 容器只有两种 in-place 容器：

1. 桌面受管窗口
2. 移动端受管单页容器

### 4.1 桌面受管窗口

桌面端会把 App 内容渲染到系统窗口内。

宿主提供：

- 系统 Title Bar
- 拖拽
- 缩放
- 聚焦
- 最小化
- 最大化
- 窗口内 dialog 容器

适用场景：

- 设置页
- 工具页
- 多栏信息管理页
- 需要和其他窗口并存的 App

### 4.2 移动端受管单页容器

移动端不会渲染一个桌面意义上的浮动窗口，而是把当前 top app 渲染成一个全屏容器。

宿主提供：

- 顶层内容承载
- safe area / dead zone 处理
- 窗口内 dialog 容器
- 与系统状态栏的组合行为

适用场景：

- 同一个 App 在移动端仍希望复用核心内容视图

### 4.3 独立页面容器

这不是 Desktop 宿主的一部分，而是普通 Web Route。

适用场景：

- App 需要直接以页面存在
- App 需要浏览器 URL / deep link
- App 需要脱离 Desktop 壳层单独访问

`MessageHub` 当前就是这种“双宿主”模式。

## 5. 推荐的双宿主写法

### 5.1 先写业务 View

```tsx
export function MyAppView({
  initialItemId,
  embedded,
}: {
  initialItemId?: string | null
  embedded: boolean
}) {
  return <div>...</div>
}
```

原则：

- 业务 View 只关心业务状态
- 不直接知道自己是不是一个 Desktop Window
- 只通过少量明确的 props 感知宿主差异

### 5.2 再写 Desktop Panel Adapter

```tsx
import type { AppContentLoaderProps } from '../types'
import { MyAppView } from './MyAppView'

export function MyAppAppPanel(props: AppContentLoaderProps) {
  return (
    <MyAppView
      embedded
    />
  )
}
```

PanelAdapter 负责：

- 接收 `AppContentLoaderProps`
- 处理与 Desktop 宿主有关的适配
- 把宿主上下文翻译给业务 View

### 5.3 再写 Standalone Route Adapter

```tsx
import { useSearchParams } from 'react-router-dom'
import { MyAppView } from './MyAppView'

export function MyAppRoute() {
  const [searchParams] = useSearchParams()
  const itemId = searchParams.get('itemId')

  return (
    <MyAppView
      initialItemId={itemId}
      embedded={false}
    />
  )
}
```

RouteAdapter 负责：

- 读取 URL 参数
- 对接 Router
- 提供页面级导航语义

## 6. App 在 Desktop 中可声明的最小 Manifest

当前 `AppDefinition.manifest` 里最重要的字段如下：

```ts
export interface WindowManifest {
  defaultMode: DisplayMode
  allowMinimize: boolean
  allowMaximize: boolean
  allowClose: boolean
  allowFullscreen: boolean
  mobileFullscreenBehavior: 'cover_dead_zone' | 'keep_dead_zone'
  mobileStatusBarMode: 'compact' | 'standard'
  titleBarMode: 'system' | 'custom'
  placement: 'inplace' | 'new-container'
  contentPadding?: 'default' | 'none'
  mobileRedirectPath?: string
  desktopWindow?: {
    width: number
    height: number
    minWidth?: number
    minHeight?: number
  }
}
```

对 App 开发者最有用的字段是：

- `placement`
  是否走同容器窗口系统
- `desktopWindow`
  桌面窗口默认尺寸和最小尺寸
- `contentPadding`
  内容区是否由系统自动加 padding
- `mobileStatusBarMode`
  移动端状态栏组合方式
- `mobileRedirectPath`
  移动端是否直接跳到独立 Route

### 6.1 推荐默认值

一般业务 App 推荐从下面的策略起步：

```ts
manifest: {
  defaultMode: 'windowed',
  allowMinimize: true,
  allowMaximize: true,
  allowClose: true,
  allowFullscreen: false,
  mobileFullscreenBehavior: 'cover_dead_zone',
  mobileStatusBarMode: 'compact',
  titleBarMode: 'system',
  placement: 'inplace',
  desktopWindow: {
    width: 900,
    height: 620,
    minWidth: 680,
    minHeight: 420,
  },
}
```

## 7. 互操作性

互操作性指的是：

- 一个 App 在自己的逻辑里，打开另一个 App
- 一个 App 在自己的逻辑里，跳转到另一个 App 的某个具体状态

当前建议先把这两个动作区分清楚：

- `open`
  目标是在 Desktop 宿主里打开另一个 App 的窗口
- `navigate`
  目标是跳转到另一个 App 的独立页面或 deep link

### 7.1 当前已经可依赖的能力：跳转到独立页面 / deep link

这条能力当前已经存在，因为整个应用运行在 React Router 之上。

如果目标 App 提供了独立 Route，例如：

```ts
'/messagehub'
```

那么当前 App 可以直接通过 Router 跳转：

```tsx
import { useNavigate } from 'react-router-dom'

export function MyAppView() {
  const navigate = useNavigate()

  return (
    <button
      type="button"
      onClick={() => navigate('/messagehub?entityId=agent-coder')}
    >
      Open MessageHub
    </button>
  )
}
```

这适合：

- 打开另一个 App 的独立页面
- 进入另一个 App 的特定对象、会话、详情页
- 在移动端直接进入目标 App 的完整页面体验

推荐做法：

- 让目标 App 提供稳定的 Route Adapter
- 通过 URL 参数表达目标状态
- 把 deep link 当成跨 App 跳转的最小协议

例如：

- `/messagehub?entityId=agent-coder`
- `/myapp?itemId=123`

### 7.2 当前已经可依赖的能力：移动端跳到目标 App 的独立 Route

如果目标 App 在 manifest 中声明了：

```ts
mobileRedirectPath: '/messagehub'
```

那么系统在移动端打开这个 App 时，会直接跳到对应 Route，而不是继续留在 Desktop 内嵌容器里。

这意味着：

- App 可以把“独立页面”作为移动端互操作的主要落点
- 复杂 App 在移动端更适合通过 route/deep link 互相进入

### 7.3 当前还没有公开成 SDK 的能力：从一个 Panel 直接打开另一个 Desktop App 窗口

Desktop 宿主内部确实有打开 App 的逻辑，但它目前是 shell 内部实现，不是公开给 App 的 SDK。

也就是说，当前 Panel 不应依赖下面这种能力：

- 直接命令宿主“打开 appId=xxx 的桌面窗口”
- 直接命令宿主“聚焦某个已存在窗口”
- 直接命令宿主“如果没打开则创建窗口，否则激活已有窗口”

对 App 开发者的结论是：

- 当前 SDK 里，没有稳定公开的 `openApp(appId)` 接口
- 不要直接 import Desktop shell 内部逻辑
- 不要把跨 App 打开窗口建立在宿主私有实现上

### 7.4 当前推荐的互操作策略

当前最稳妥的策略是：

1. deep link first
2. route as protocol
3. desktop window open 交给未来宿主 API

具体说：

- 如果目标 App 可以独立运行，优先提供 Route Adapter
- 如果需要跨 App 进入具体状态，优先设计 URL 参数
- 如果未来系统补充 `openApp(appId, options)` 这类宿主 API，再把“打开窗口”接到同一套 deep link 语义上

推荐把跨 App 目标统一描述成：

- `appId`
- `path`
- `query`
- `state`

而不是直接耦合到某个窗口实例。

### 7.5 当前文档建议的最小约束

对于一个希望被其他 App 打开的 App，建议至少提供：

1. 一个独立 Route
2. 一套稳定的 query 参数协议
3. 一个 Route Adapter

例如 `MessageHub` 可被理解为：

- App 标识：`messagehub`
- 独立入口：`/messagehub`
- deep link 示例：`/messagehub?entityId=agent-coder`

这样别的 App 即使不能直接要求 Desktop 宿主“打开一个 MessageHub 窗口”，也至少可以稳定地“跳到 MessageHub 对应状态”。

## 8. 当前真正可用的宿主能力

### 8.1 内容区 padding 控制

如果你的 View 自己管理完整布局，通常应该设：

```ts
contentPadding: 'none'
```

否则系统窗口内容区会自动加内边距。

适合 `contentPadding: 'none'` 的典型场景：

- 三栏布局
- 聊天 / 工作台
- 文件浏览器
- 自带 header / sidebar / split view 的复杂界面

### 8.2 系统控件：窗口内 Dialog

当前唯一明确存在的宿主级 imperative API 是窗口内 dialog：

```ts
const windowDialog = useWindowDialog()
```

可用能力：

- `modal`
- `sheet`
- `fullscreen`

注意：

- `fullscreen` 受权限限制
- 当前 mock 规则下，只有 `system` tier 默认允许 fullscreen dialog

这类能力应被视为系统控件，而不是 App 间互操作能力。

### 8.3 移动端重定向到独立页面

如果一个 App 在移动端更适合完整独立页面，而不是 Desktop shell 内嵌容器，可以声明：

```ts
mobileRedirectPath: '/myapp'
```

这适合：

- 聊天类应用
- 长流程应用
- 需要 URL 的内容型应用

## 9. 新 App 的最小接入步骤

### 9.1 实现 Panel Adapter

在 `src/app/<app>/` 下实现 `MyAppAppPanel.tsx`。

### 9.2 注册到 app registry

在 `src/app/registry.tsx` 中注册 loader。

### 9.3 在 app catalog 中声明 manifest

在 `src/mock/data.ts` 中添加 `AppDefinition`。

### 9.4 如果需要独立页面，增加 route

在 `src/App.tsx` 里增加独立 route，并实现 `MyAppRoute.tsx`。

## 10. 什么时候应该做“双宿主 App”

适合同时支持独立页面和 Desktop 内嵌的 App：

- 通讯类
- 文件类
- 管理台类
- 长驻型工作流类

判断标准：

- 脱离 Desktop 后，这个 App 仍然有独立成立的使用价值
- 用户会希望通过 URL 直接进入某个状态
- App 本身有较强的信息架构，不只是一个小工具面板

不一定需要双宿主的 App：

- 只服务于 Desktop 壳层的小工具
- 高度依赖系统上下文的设置面板
- 很轻量、没有独立导航意义的操作面板

## 11. 当前实现的已知限制

下面这些要明确视为“当前实现状态”，不要当成稳定 SDK 承诺：

### 11.1 `new-container` 还没有真正实现

虽然 manifest 有：

```ts
placement: 'new-container'
```

但当前点击后只是记录行为并提示，不会真的创建新容器。

### 11.2 `titleBarMode: 'custom'` 还没有真正落地

虽然 manifest 有这个字段，但当前桌面 in-place 窗口仍然统一使用系统 Title Bar。

### 11.3 还没有公开的 App-to-App `openApp()` 能力

当前 App 间可以稳定做的是 route/deep link 跳转，不能稳定做的是“命令宿主打开另一个桌面窗口”。

如果业务场景明确依赖这个能力，应该先扩展宿主 SDK，而不是让 App 直接调用私有实现。

### 11.4 `allowClose` 当前没有真正控制按钮显示

`allowMinimize` 和 `allowMaximize` 已经生效，`allowClose` 当前没有完整生效。

### 11.5 `defaultMode: 'fullscreen'` 当前不会得到真正 fullscreen window

当前非 `windowed` 基本都会落到 `maximized` 行为。

### 11.6 还没有公开的 Window Handle API

Panel 当前拿不到这些能力：

- 主动改窗口位置
- 主动改窗口大小
- 主动触发最小化 / 最大化
- 主动改窗口标题
- 主动控制窗口层级

如果业务确实需要这些能力，当前更适合先扩展宿主协议，而不是让 App 直接依赖内部实现。

## 12. 最小实践建议

给一个新 App 的最小建议如下：

1. 先写 `AppView`
2. 让 `AppView` 不直接依赖 Desktop 宿主
3. 用 `PanelAdapter` 处理 Desktop 接入
4. 用 `RouteAdapter` 处理独立页面接入
5. 把跨 App 进入优先建模成 route/deep link
6. 复杂布局默认使用 `contentPadding: 'none'`
7. 需要移动端独立体验时使用 `mobileRedirectPath`
8. 不要依赖 `new-container`、`custom title bar`、`openApp()`、`fullscreen window` 这些尚未落地的能力

## 13. 一个最小骨架

```tsx
// MyAppView.tsx
export function MyAppView({
  embedded,
  initialItemId,
}: {
  embedded: boolean
  initialItemId?: string | null
}) {
  return <div>{embedded ? 'embedded' : 'standalone'}:{initialItemId}</div>
}
```

```tsx
// MyAppAppPanel.tsx
import type { AppContentLoaderProps } from '../types'
import { MyAppView } from './MyAppView'

export function MyAppAppPanel(_: AppContentLoaderProps) {
  return <MyAppView embedded />
}
```

```tsx
// MyAppRoute.tsx
import { useSearchParams } from 'react-router-dom'
import { MyAppView } from './MyAppView'

export function MyAppRoute() {
  const [searchParams] = useSearchParams()

  return (
    <MyAppView
      embedded={false}
      initialItemId={searchParams.get('itemId')}
    />
  )
}
```

这就是当前仓库里最符合现实的最小 SDK 接入套路。
