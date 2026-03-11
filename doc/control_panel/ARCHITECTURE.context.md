# Control Panel Architecture

## Purpose

- 描述 `control_panel` 当前真实运行结构，而不是抽象 PRD 愿景图。
- 帮助读者快速理解 Rust backend、React frontend、内嵌 Files、workspace 数据源之间的关系。
- 为后续“从代码提取结构 -> 反哺规格”的流程提供蓝图基线。

## Runtime Surfaces

- `Static SPA`: 由 `control_panel` 服务直接在 `/` 下提供前端静态资源。
- `Control Panel kRPC`: 由 `control_panel` 服务在 `/kapi/control-panel` 暴露。
- `Embedded Files HTTP API`: 由 `control_panel` 内嵌 `file_manager` 在 `/api/*` 暴露。
- `Workspace external backends`: workspace 在前端里可见，但开发态还会通过 `/kapi/opendan`、`/kapi/task-manager` 访问其他服务，这部分不属于 Rust `control_panel` 自身能力。

### Surface Ownership Table

| Surface | Served by | Primary consumers | Ownership note |
| --- | --- | --- | --- |
| `/` | Rust `control_panel` service | browser | static asset hosting only |
| `/kapi/control-panel` | Rust `control_panel` service | main admin UI | canonical RPC surface for control panel |
| `/api/*` | embedded `file_manager` inside `control_panel` | Files UI and public share flows | HTTP-first contract, not kRPC-first |
| `/kapi/opendan` | external service via gateway/proxy | workspace UI | not owned by Rust `control_panel` |
| `/kapi/task-manager` | external service via gateway/proxy | workspace UI | not owned by Rust `control_panel` |

## Backend Structure

- 服务入口在 `src/frame/control_panel/src/main.rs:4413`。
- `ControlPanelServer` 是 backend composition root，负责挂载 kRPC、Files HTTP、静态资源。
- 同一个服务进程内承担 3 类核心职责：
  - 系统控制面板 RPC
  - 内嵌 Files HTTP 服务
  - 前端静态资源分发
- 当前后端仍偏单体，很多 handler 直接集中在 `src/frame/control_panel/src/main.rs:3962` 一带的 dispatch 中。

### Backend Composition Table

| Area | Primary file | Responsibility |
| --- | --- | --- |
| service bootstrap | `src/frame/control_panel/src/main.rs:4413` | runtime init, login, runner startup |
| RPC dispatch | `src/frame/control_panel/src/main.rs:3982` | method routing and handler fan-out |
| HTTP routing | `src/frame/control_panel/src/main.rs:4371` | path-based split between `/kapi`, `/api`, and static web |
| embedded files backend | `src/frame/control_panel/src/file_manager.rs` | file CRUD, share, upload, preview, ACL |
| future/sidecar content manager | `src/frame/control_panel/src/share_content_mgr.rs` | present in source, runtime role still unclear |

## Frontend Structure

- React 启动链路是 `src/frame/control_panel/web/src/main.tsx` -> `src/frame/control_panel/web/src/App.tsx`。
- 认证上下文由 `src/frame/control_panel/web/src/auth/AuthProvider.tsx` 和 `src/frame/control_panel/web/src/auth/authManager.ts` 驱动。
- 路由由 `src/frame/control_panel/web/src/routes/router.tsx:22` 定义。
- 普通控制面板页面主要通过 `src/frame/control_panel/web/src/api/index.ts:4` 的 kRPC client 访问 `/kapi/control-panel`。
- Files 页面是特例，直接对 `/api/*` 发 HTTP 请求。

### Desktop Experience Core

- `/` 入口对应 `src/frame/control_panel/web/src/ui/pages/DesktopHomePage.tsx`，它不是单一 dashboard page，而是一个全集成 desktop 容器。
- 这个 desktop 容器在一个页面内部管理多个 window module，包括 `monitor`、`network`、`containers`、`files`、`storage`、`logs`、`apps`、`settings`、`users`。
- 这些模块当前不是按一级路由拆开的，而是由 desktop 内部 window state、z-index、drag/resize/minimize/maximize 等机制统一调度。
- 因此，desktop 是 control panel 的 primary shell，其他模块更准确地说是“desktop windows”而不是“homepage widgets”。

### Frontend Composition Table

| Layer | Primary file | Responsibility |
| --- | --- | --- |
| app shell bootstrap | `src/frame/control_panel/web/src/App.tsx:1` | wrap router with auth provider |
| auth state | `src/frame/control_panel/web/src/auth/AuthProvider.tsx:11` | initialize runtime, resolve login state, expose auth actions |
| route graph | `src/frame/control_panel/web/src/routes/router.tsx:22` | public, protected, Files, workspace route topology |
| desktop shell | `src/frame/control_panel/web/src/ui/pages/DesktopHomePage.tsx:148` | integrated desktop container, window manager, core homepage experience |
| main RPC API layer | `src/frame/control_panel/web/src/api/index.ts:4` | kRPC wrapper and mock fallback behavior |
| workspace API layer | `src/frame/control_panel/web/src/api/workspace.ts:783` | OpenDan and TaskManager clients |
| Files UI surface | `src/frame/control_panel/web/src/ui/pages/FileManagerPage.tsx:54` | direct HTTP contract consumer |

### Desktop Window Map

| Window id | Current title | Current role |
| --- | --- | --- |
| `monitor` | `System Monitor` | system overview and metrics |
| `network` | `Network Monitor` | network state and trends |
| `containers` | `Container Manager` | container runtime overview |
| `files` | `Files` | embedded file manager surface |
| `storage` | `Storage Center` | storage health and capacity |
| `logs` | `System Logs` | log query and inspection |
| `apps` | `Applications` | installed app and version view |
| `settings` | `Settings` | system and policy settings |
| `users` | `Users` | user and role-related management |

## Files Subsystem

- Files 当前不是独立服务，而是 `control_panel` 内嵌模块。
- 后端实现位于 `src/frame/control_panel/src/file_manager.rs`。
- 它负责：资源浏览与 CRUD、上传会话、收藏/最近/回收站、分享、公开预览与下载、缩略图与部分预览转换、ACL 判定。
- HTTP 入口以 `/api/resources*`、`/api/share*`、`/api/public/*`、`/api/upload/session*` 为主。
- 当前前端主链路不走 `files.*` 或 `share.*` kRPC，这是架构上的重要特例。

### Files Boundary Rules

- Files 在部署上跟随 `control_panel` 一起发布。
- Files 在鉴权上复用 control panel session，不维护独立登录态。
- Files 在契约上以 HTTP endpoint 为主，不应被误写成“当前是 RPC-first 架构”。
- Files 的公开分享视图同时面向登录用户和匿名访问，因此需要和主控制台页面分开理解。

## Workspace Subsystem

- `workspace` 通过同一个 React app 路由挂载在 `/workspace` 下。
- 其客户端主要在 `src/frame/control_panel/web/src/api/workspace.ts:783`，依赖 `/kapi/opendan/` 与 `/kapi/task-manager/`。
- 因此它在“产品体验上属于 control panel”，但在“后端所有权上”不等同于 Rust `control_panel` 服务。
- 改动 workspace 时，不能默认认为只改 `control_panel` backend 就够了。

### Ownership Matrix

| Concern | Frontend owner | Backend owner | Notes |
| --- | --- | --- | --- |
| login and session bootstrap | control panel web | Rust `control_panel` + verify-hub semantics | shared across main UI and Files |
| dashboard/monitor/network/settings pages | control panel web | Rust `control_panel` | kRPC-first |
| Files page and public share page | control panel web | embedded `file_manager` | HTTP-first |
| workspace | control panel web | external OpenDan/TaskManager services | same shell, different backend boundary |

## Authentication And Session Flow

- 浏览器登录通过 `auth.login` 获取 `session_token` 与 `refresh_token`，实现见 `src/frame/control_panel/web/src/auth/authManager.ts:188`。
- 主 UI 页面通过 kRPC client 持有并刷新 session。
- Files HTTP 使用同一套 session 语义，token 可经由 `X-Auth`、query `auth` 或 cookie 进入后端。
- 这意味着 auth 是共享的；Files 不应再回到独立登录模型。

### Auth Flow Table

| Step | Browser action | Backend surface | Result |
| --- | --- | --- | --- |
| runtime init | `AuthProvider` calls `ensureAuthRuntime()` | control-panel auth client setup | auth runtime becomes usable |
| session check | `ensureSessionToken()` | `auth.verify` and possibly `auth.refresh` | browser decides authenticated vs unauthenticated |
| login | `loginWithPassword()` | `auth.login` | returns session and refresh tokens |
| SSO login | `SsoLoginPage` calls `auth.login` with `client_id` as `appid` | `/kapi/control-panel` | returns app-scoped token payload for SSO use |
| main page request | `api/index.ts` kRPC call | `/kapi/control-panel` | authenticated RPC response |
| Files request | direct `fetch('/api/...')` | embedded `file_manager` | authenticated HTTP response using shared token semantics |

### Gateway OAuth Flow

- 浏览器访问 app host -> gateway 在 `src/rootfs/etc/boot_gateway.yaml:160` 执行 `check_oauth`。
- 对 private app，gateway 当前检查 cookie `buckyos_session_token`，并验证 JWT 与 `payload.appid == $APP_ID`。
- 这说明 gateway OAuth 的认证介质是 cookie，而不是 control panel SPA 使用的 localStorage。
- 当前实现中，`/sso/login` 会把登录返回的 app-specific token 写入 `buckyos_session_token` cookie；相关架构背景见 `doc/arch/boot_gateay的配置生成.md`。
- 当前 gateway 失败跳转仍指向 `/login`，文档已记录该实现现状与建议方向，但本次未改动代码。

## Primary Data Flows

### Desktop Window Flow

- Browser enters `/` -> `DesktopHomePage` bootstraps layout and shared desktop state -> user opens or focuses desktop windows -> each window reads from shared state and/or its own fetch path -> desktop shell handles focus order, drag, resize, minimize, maximize.

### Admin Page Flow

- Browser UI -> `src/api/index.ts` -> `/kapi/control-panel` -> `ControlPanelServer` dispatch -> runtime/system integrations -> response.

### Files Flow

- Browser UI -> `FileManagerPage.tsx` direct `fetch('/api/...')` -> embedded `file_manager` -> file/share/ACL logic -> response.

### Workspace Flow

- Browser UI -> `src/api/workspace.ts` -> `/kapi/opendan` or `/kapi/task-manager` -> external service response.

### Flow Comparison Table

| Flow | Transport | Frontend entry | Backend owner | Key risk |
| --- | --- | --- | --- | --- |
| desktop windows | mixed kRPC + HTTP inside one shell | `DesktopHomePage.tsx` | mixed: Rust `control_panel` plus embedded Files | monolithic page code can blur boundaries |
| admin pages | kRPC | `src/api/index.ts` | Rust `control_panel` | mock fallback can hide failures |
| Files | HTTP | `FileManagerPage.tsx` | embedded `file_manager` | easy to mistakenly document as RPC |
| workspace | kRPC | `src/api/workspace.ts` | external services | ownership confusion during changes |

## Module Dependency Map

- Backend 依赖 verify-hub 风格 session 体系、system config、系统状态采集、网关/容器/应用等底层服务。
- Frontend 依赖路由、认证状态、UI 页面、kRPC API wrapper。
- Files 依赖共享会话语义与自身 HTTP 契约。
- Workspace 依赖 OpenDan/TaskManager 客户端，而不是 `control_panel` backend 的同名 handler。

### Dependency Layers

| Layer | Depends on |
| --- | --- |
| control panel web shell | auth provider, router, route-level pages |
| main page components | `api/index.ts`, shared global types, auth state |
| Files UI | shared auth token state, `/api/*` contract, file preview components |
| Rust `control_panel` service | runtime init, session verification model, system integrations |
| embedded `file_manager` | filesystem root policy, token extraction, ACL policy, share/public delivery logic |

## Request Routing Map

- `/` -> 静态前端资源，由 `runner.add_dir_handler_with_options` 挂载。
- `/kapi/control-panel` -> `serve_http_by_rpc_handler`。
- `/api` 与 `/api/*` -> `self.file_manager.serve_request(...)`。
- 这 3 条路径是理解系统行为和部署方式的最小架构切面。

### Request Routing Table

| Incoming path | Handler | Source file |
| --- | --- | --- |
| `/` | static directory handler with SPA fallback | `src/frame/control_panel/src/main.rs:4453` |
| `/kapi/control-panel` | RPC adapter into `ControlPanelServer` | `src/frame/control_panel/src/main.rs:4384` |
| `/api` and `/api/*` | embedded files server request handler | `src/frame/control_panel/src/main.rs:4380` |

## Implemented vs Planned Architecture

- `Implemented`
  - 主控制面板 SPA
  - desktop shell with integrated window model on `/`
  - `auth.*`、`ui.*`、部分 `system.*`、`apps.*`、`network.overview`、`zone.overview`、`gateway.overview`、`container.overview`
  - 内嵌 Files HTTP surface
- `Planned`
  - 大量 NAS 风格管理能力的完整 RPC 落地
  - 安装页、分享安装页、更多通知/审计/备份/ACL UI 的完整统一实现
- `Needs clarification`
  - `src/frame/control_panel/src/share_content_mgr.rs` 在当前结构中看起来更像预留或旁路能力，需要后续明确其运行地位。

## Frontend Refactoring Direction

- `src/frame/control_panel/web/src/ui/pages/DesktopHomePage.tsx` 当前承担了 desktop shell、window manager、多个 window 的渲染与状态逻辑，文件体量已经偏大。
- 推荐未来在 `src/frame/control_panel/web/src/ui/pages/desktop/` 下按 window 拆分代码，以“desktop shell + per-window modules”的结构组织：
  - `DesktopShell.tsx` 或保留 `DesktopHomePage.tsx` 作为容器
  - `desktop/MonitorWindow.tsx`
  - `desktop/NetworkWindow.tsx`
  - `desktop/ContainersWindow.tsx`
  - `desktop/FilesWindow.tsx`
  - `desktop/StorageWindow.tsx`
  - `desktop/LogsWindow.tsx`
  - `desktop/AppsWindow.tsx`
  - `desktop/SettingsWindow.tsx`
  - `desktop/UsersWindow.tsx`
- 这样拆分的目标不是把 desktop 变回路由式页面，而是在保留 integrated desktop model 的前提下改善代码边界。
- desktop 内模块的组织单位应优先是 `window`，而不是重新强行回到“每个功能一条首页路由”的思路。

## Verification Anchors In Code

- `src/frame/control_panel/src/main.rs:4371`
- `src/frame/control_panel/src/main.rs:4413`
- `src/frame/control_panel/src/file_manager.rs`
- `src/rootfs/etc/boot_gateway.yaml:160`
- `src/frame/control_panel/web/src/routes/router.tsx:22`
- `src/frame/control_panel/web/src/ui/pages/SsoLoginPage.tsx:122`
- `src/frame/control_panel/web/src/api/index.ts:4`
- `src/frame/control_panel/web/src/api/workspace.ts:783`
- `src/frame/control_panel/web/src/auth/authManager.ts:11`
- `src/frame/control_panel/web/vite.config.ts:13`

## Migration Status

- `doc/PRD/control_panel/control_panel.md` 中“信息架构与导航”“依赖与风险”“Files/Share 当前实现状态”应以重写后形式收敛到这里。
- 本文优先描述当前真实结构，不负责承载完整接口清单。
