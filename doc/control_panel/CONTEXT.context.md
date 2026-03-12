# Control Panel Context

## Purpose

- 记录那些不适合写进主规格、但不理解就容易改坏系统的事实。
- 把命名约定、架构不变量、实现特例、技术债、迁移策略放在一个地方。
- 让后续人和 AI 改 `control_panel` 时先知道哪些地方“看起来能改，实际上不能乱动”。

## Detailed Outline For This File

- `Naming And Terminology Rules`
- `Architectural Invariants`
- `Non-Obvious Implementation Facts`
- `Known Gaps And Technical Debt`
- `Safe Change Guidelines`
- `Migration Classification From doc/PRD/control_panel`
- `Historical Notes To Preserve`

## Naming And Terminology Rules

- Rust/backend 模块统一叫 `control_panel`。
- 打包与构建上下文里 web 模块统一叫 `control_panel_web`。
- kRPC 命名统一用 `<module>.<action>`。
- `Files` 是产品表面；`file_manager` 是当前后端实现模块。
- `workspace` 是 control panel web 中的一个体验，不等同于 Rust `control_panel` backend 功能域。
- `chat` 是 control panel 内的产品短名；底层消息服务仍叫 `msg-center`。
- 当前 `chat` 入口是 desktop subwindow，不是独立 `/chat` route。

## Architectural Invariants

- `control_panel` 统一承载静态 web、control-panel kRPC、embedded Files HTTP 这 3 个入口面。
- chat 浏览器入口必须走 `control_panel` 的 `chat.*` wrapper，而不是让 control panel web 直连 `/kapi/msg-center`。
- Files 内嵌是当前架构事实；如果重新引入独立登录、独立部署假设，需要明确说明是架构变更。
- session 语义在主控制面板和 Files 之间共享。
- 路由、后端 payload、前端 types 需要联动变更。
- 文档中所有 `Implemented` 断言必须可被代码或运行行为支撑。
- `/login` 的 control-panel 本地登录职责，与 `/sso/login` 的 app-specific SSO 职责，不应长期混写。

## Non-Obvious Implementation Facts

- Files 主链路是 direct HTTP，不是 `files.*`/`share.*` kRPC。
- `workspace` 和主控制面板共享前端壳，但后台不是一个系统。
- `msg-center` 原生接口主要是 owner DID / contact-manager scope 视角，不是 control-panel session 视角。
- chat realtime 当前应该优先走 control panel service 内的 HTTP streaming helper，而不是为浏览器额外暴露 msg-center 或新开 WebSocket 端口。
- `src/frame/control_panel/web/src/api/index.ts` 有 mock fallback 逻辑，开发时“页面能显示”不等于后端真的通了。
- 旧 PRD 中大量 RPC/页面设想比当前代码宽得多；迁移时优先保真，不优先求全。
- desktop 首页 `/` 不是普通 route page，而是 integrated desktop shell；其中大量模块是在单页内部按 window 方式组织的。
- gateway app OAuth 当前检查的是 cookie `buckyos_session_token`，而不是 control panel localStorage。
- `SsoLoginPage` 已经具备 `client_id` -> `auth.login(appid=client_id)` 的 app-aware 登录语义，并负责把返回 token 落到 `buckyos_session_token` cookie。
- chat wrapper 的 owner scope 应由 authenticated user 反推，优先取 user contact DID，缺省回退到 `did:bns:<username>`。

## Known Gaps And Technical Debt

- `src/frame/control_panel/src/main.rs` 过大，dispatch 与 domain logic 混杂。
- Files/Share 真实表面是 HTTP，但历史文档里长期以 RPC 规划形式存在。
- `doc/PRD/control_panel/control_panel.md` 混合了产品愿景、接口规格、现状说明、路线图。
- `src/frame/control_panel/src/share_content_mgr.rs` 的定位仍需进一步澄清。
- `src/frame/control_panel/web/src/ui/pages/DesktopHomePage.tsx` 过大，desktop shell、window manager、window content 耦合在同一文件中。
- chat 当前仍是 control panel 内的最小 desktop entry；message-level realtime 已在 control panel service 内实现，读能力已对登录用户开放，但 token-level agent stream、独立 chat app 对接、agent control channel 仍在后续范围。

## Safe Change Guidelines

- 改路由前，同时看 router、导航、历史入口名、受保护路由约束。
- 改 auth 前，同时看前端 authManager、backend `auth.*`、Files token 接收规则。
- 改 Files 前，同时看 `FileManagerPage.tsx`、`file_manager.rs`、公开分享页面行为。
- 改 chat 前，同时看 `control_panel` 的 auth/permission、`MsgCenterClient` 合同，以及 owner DID 的映射规则。
- 改 chat realtime 前，同时看 `msg-center` 的 box changed kevent 语义；该事件只表示 record changed，不天然等同于 token-level text delta。
- 改文档前，先决定某个事实应归属 README、ARCHITECTURE、SPEC 还是 CONTEXT，避免重复定义。
- 改 desktop 首页前，不要把 window-based integrated model 误改成若干彼此独立的普通 route page。
- 改 SSO 前，同时看 `src/frame/control_panel/web/src/ui/pages/SsoLoginPage.tsx`、`src/rootfs/etc/boot_gateway.yaml` 和 `doc/arch/boot_gateay的配置生成.md`。

## Safe Refactoring Boundaries

- 可以安全重构文档组织，但不要改变 `Implemented`/`Planned` 的语义边界。
- 可以重构前端组件，但不要默默改变 kRPC payload 字段名。
- 可以演进 chat UI，但不要把 `msg-center` 的 owner / contact_mgr_owner 参数直接暴露成浏览器可任意指定的控制面。
- 可以优化 Files 交互，但不要破坏共享 session 和 ACL 假设。
- 如果要把 HTTP Files 契约迁回 kRPC，需要先在规格中声明迁移路径。
- 可以把 SSO cookie 写入职责收敛到 `/sso/login`，但不要在未理清 gateway 跳转链路前误以为 `/login` 与 `/sso/login` 完全等价。

## UI And Visual Guardrails

- 当前 control panel 的视觉方向是“系统控制台 + 轻桌面工作台”，不是通用企业 CRUD 后台。
- 不要把页面改回默认化的白底表格站，也不要把局部页面做成与主壳层完全不同的产品语气。
- 视觉统一优先级高于单页局部创意；新组件应先服从已有 token 和容器语言。
- `RootLayout`、`DesktopHomePage`、`FileManagerPage` 共同定义了当前视觉三件套：
  - 系统侧栏与主内容工作区
  - 桌面/窗口式系统工作台
  - 工具型文件管理工作区
- Files 可以更工具化，但不应脱离主色、边框、圆角、状态色语义。
- 弹窗、按钮、标签、卡片、表格、搜索栏应减少页面级自定义变体，优先收敛成稳定模式。

### Visual Tokens To Preserve

- 主色：`#0f766e`
- 强调色：`#f59e0b`
- 中性基底：`#0f172a`、`#52606d`、`#d7e1df`、`#f4f8f7`、`#ffffff`
- 标题字体：`Space Grotesk`
- 正文字体：`Work Sans`
- 圆角尺度：`8 / 12 / 18 / 24`
- 阴影风格：soft / strong，避免厚重模糊阴影

### Common Visual Failure Modes

- 把新页面做成默认 Tailwind 模板感，失去 control panel 的系统气质。
- 颜色失控：每个模块自己发明一套主色或 warning/success 语义。
- 圆角和阴影失控：同层级组件风格不一致，导致 UI 像多个产品拼接。
- Files 与主控制台割裂：像外嵌第三方工具，而不是同一系统表面。
- 交互过度动画化或 hover 过强，破坏“稳定工具”感。

### Safe UI Change Rules

- 改视觉前先检查 `src/frame/control_panel/SKILL.md` 是否已有明确规则。
- 改一个主容器样式时，至少同步检查 `RootLayout`、Desktop、Files、弹窗。
- 新增状态色或尺寸 token 前，应先判断能否复用现有 token。
- 任何声称“视觉统一”的改动都应覆盖桌面端与移动端，而不是只修一个断点。
- 改 Desktop 时，优先把代码按 `window` 语义拆分，而不是把 desktop 的 integrated experience 打散成互相不一致的碎页面。

## Migration Classification From doc/PRD/control_panel

### `doc/PRD/control_panel/README.md`

- `直接吸收`:
  - 控制面板是系统服务
  - `sys` 短域名定位
  - 入口页面枚举这类高层导航信息
- `需要重写`:
  - 入口页面需要按当前已实现路由和 PRD-only 页面重新分层
  - popup 相关描述需要改成产品原则而不是零散列举
- `保留为历史记录`:
  - 旧页面命名如果尚未映射到当前路由，可先保留为 historical notes

### `doc/PRD/control_panel/SSO.md`

- `直接吸收`:
  - session token / refresh token 的基本模型
  - 兼容应用不要混用 service token 与 page token 的原则
- `需要重写`:
  - 当前 control panel 路由约定需要与现有前端路由和 auth 实现对齐
  - SSO 说明需要拆成 contract-level 行为和 context-level 注意事项
- `保留为历史记录`:
  - 超级用户签名页等未完全落地部分

### `doc/PRD/control_panel/app安装UI.md`

- `直接吸收`:
  - 安装流程阶段划分：确认、配置、进度、失败
  - 分享安装和信任解释这类高层产品目标
- `需要重写`:
  - 页面命名、路由、入口和任务状态流需要与当前实现或明确的 planned 状态对齐
  - 未来商店、经济/付费相关内容需要拆成 planned sections，不应混入当前实现规格
- `保留为历史记录`:
  - 远期商店/付费设想中未进入近期实现范围的部分

### `doc/PRD/control_panel/系统的GC工作.md`

- `直接吸收`:
  - 数据分类和自动/手动清理的核心意图
- `需要重写`:
  - 需要扩写为生命周期、可见性、策略边界，而不是一句话需求
- `保留为历史记录`:
  - 无，内容太短，重写后旧文档可直接降级为 historical stub

### `doc/PRD/control_panel/control_panel.md`

- `直接吸收`:
  - 目标与边界
  - 核心概念
  - 角色与权限模型
  - 用户旅程
  - 命名约定
  - 通用字段约定
  - 版本策略
  - 已经贴近实现的 Files 当前状态说明
- `需要重写`:
  - 混合在一起的愿景、信息架构、实现现状、RPC 规划
  - 所有应按新四文件职责拆散的章节
  - 所有无法直接从代码验证的“当前状态”句子
- `保留为历史记录`:
  - 大量尚未实现的 RPC 规划清单
  - 作为迁移期间的完整历史来源文件

## Historical Notes To Preserve

- Old PRD docs remain useful as intent archives during migration.
- Historical docs should not silently define current behavior once a canonical replacement exists.
- When an old section is superseded, add a pointer instead of deleting context immediately.
