# Control Panel Specification

## Purpose

- 作为 `control_panel` 的主规格文件，描述对外可见行为。
- 把路由、鉴权表面、kRPC 契约、Files HTTP 契约、关键状态模型放在同一个可审阅位置。
- 明确区分 `Implemented`、`Planned`、`Derived`、`Historical`。

## Spec Status Rules

- `Implemented`: behavior can be verified in code or active runtime.
- `Planned`: intended behavior not yet fully implemented.
- `Derived`: structure extracted from code and normalized for readability.
- `Historical`: retained only to preserve design history or explain migration.

## Status Legend

- `Implemented`: 可从现有代码或运行行为验证。
- `Planned`: 已有产品方向，但尚未完整实现。
- `Derived`: 从代码结构提取并语义化整理。
- `Historical`: 仅为迁移与背景保留，不代表当前行为。

## Route Surface

### Implemented

- `/login`
  - control panel 自身登录页。
- `/sso/login`
  - SSO 授权弹窗页。
- `/workspace`
  - 受保护的 workspace 体验。
- `/`
  - desktop 首页。
  - Desktop 中的 Message Hub 图标当前会以 `_blank` 方式打开 `/message-hub/chat`。
  - Desktop 中的 Workspace 图标当前会以 `_blank` 方式打开 `/workspace`。
  - Desktop 中新增 `AI Models` 图标，并在 desktop 内打开 AI 管理窗口。
- `/message-hub`
  - 独立 Message Hub route 前缀。
- `/message-hub/chat`
  - 当前的 Message Hub 主页面。
- `/share/:shareId`
  - 公开分享查看页，由 `FileManagerPage` 承载。
- `/files/detail`
  - 文件详情页。
- `/monitor` `/network` `/users` `/storage` `/containers` `/dapps` `/settings` `/notifications` `/system-logs`
  - 主控制面板导航页。
- `/index` `/index.html` -> 重定向到 `/monitor`。
- `/0monitor` -> legacy redirect。

### Planned Or Historical

- `/install.html`
- `/share_app.html`
- `/ndn/publish.html`
- `/my_content.html?content_id=...`
- `/login_index.html`

以上入口来自旧 PRD 或旧兼容流程；除非在代码中重新挂载，否则不能视为当前已实现 route surface。

## Authentication Surface

### Implemented

- `auth.login`
  - 浏览器以 `username` + hash 后的密码 + `login_nonce` 发起登录。
- `auth.refresh`
  - 用 `refresh_token` 刷新会话。
- `auth.logout`
  - 清理会话。
- `auth.verify`
  - 用于 session 合法性检查。

### Login Surface Split

- `/login`
  - 面向 control panel 自身的登录入口。
  - 当前主职责是建立 control panel web 的本地会话状态。
  - 当前实现以 localStorage 中的 account/session 信息为主。
- `/sso/login`
  - 面向目标 app 的 SSO 登录入口。
  - 当前页面会读取 `client_id`，并以该 `client_id` 作为 `auth.login` 的 `appid` 发起登录。
  - 当前实现中，它还会把返回的 app-specific `session_token` 写入 cookie `buckyos_session_token`。
  - 从职责上，它比 `/login` 更适合承载 gateway OAuth 场景下的 app-specific token 发放。

### Session Transport Rules

- kRPC 主链路通过 client 侧 session token 发送。
- Files HTTP 支持 `X-Auth`、query `auth`、cookie(`control-panel_token` / `control_panel_token` / `auth`)。
- 兼容应用与页面代理流程中的 token 规则仍保留为重要约束，但当前 control panel 主实现以前端 authManager 为准。

### Chat Wrapper Rules

- 浏览器侧 `chat.*` 必须经由 browser-safe wrapper，而不是直接访问 `/kapi/msg-center`。
- 这样做是为了复用 control panel 既有的 session 校验、方法级授权、以及 `sys` / zone host 的统一转发入口。
- 当前 Message Hub route 默认走 `/kapi/message-hub`；旧 `/kapi/control-panel` chat surface 仍作为迁移期兼容入口存在。
- `chat.bootstrap`、`chat.contact.list`、`chat.message.list`、chat stream 当前对已登录用户开放。
- `chat.message.send` 当前仍保留更高权限要求；普通只读账户不会获得发送能力。
- 前端不应提交 `owner` 或 `contact_mgr_owner` 这类底层 scope 参数；这些值由 control panel backend 从 authenticated user 推导。
- 当前 owner DID 的推导规则是：优先使用用户 contact 配置中的 DID，缺省回退到 `did:bns:<username>`。

### Chat Streaming Rules

- 实时聊天更新不新增端口，也不新增独立 chat service 对外入口。
- 当前实时链路使用 HTTP streaming endpoint：`POST /kapi/message-hub/chat/stream`。
- 该 endpoint 不是 kRPC method，而是 control panel 内部额外暴露的 chat transport helper。
- transport 当前采用 `fetch` + streamed NDJSON；不使用 WebSocket。
- 浏览器发送 `session_token` 与 `peer_did` 建立流；control panel 完成鉴权、owner DID 推导、`msg-center` 事件订阅，再把规范化后的 chat event 持续写回浏览器。
- 当前 streaming 的语义是 message-level realtime：当 `msg-center` inbox/outbox 记录发生变化时，浏览器收到增量消息或重同步提示。
- 当前 streaming 不是 token-level LLM delta；如果未来要支持 agent token stream，需要在 OpenDan/AICC 层先提供可流化上游。

### SSO Cookie Requirement For Gateway OAuth

- 对于普通 control panel SPA 登录，localStorage 会话足以驱动前端自身状态。
- 对于 gateway 的 app OAuth 检查，当前规则并不读取 localStorage，而是读取 cookie `buckyos_session_token`。
- 因此，如果要让 app 页面经由 gateway `check_oauth` 放行，登录链路最终必须写入 `buckyos_session_token` cookie。
- 当前实现已经由 `/sso/login` 完成这件事，而不是让 `/login` 同时承担 control panel 本地登录和 app-specific SSO cookie 发放两种角色。
- 当前仓库中的 gateway 规则见 `src/rootfs/etc/boot_gateway.yaml:160`；相关背景说明见 `doc/arch/boot_gateay的配置生成.md`。

### Documented Follow-Up

- 当前 `boot_gateway.yaml` 在 OAuth 失败后仍跳转到 `/login`，而不是 `/sso/login`。
- 这会造成 control panel 本地登录入口与 app-specific SSO 入口的职责混用风险。
- 目前先在文档中记录该问题与推荐方向，不在本次文档更新中直接修改 gateway 配置代码。

### Planned

- 2FA、superuser/sudo 签名页、API key 等仍属规划或未完全落地。

## kRPC Contract Surface

### Naming Rules

- 统一采用 `<module>.<action>`。
- UI 相关采用 `ui.*`。
- legacy alias `main`、`layout`、`dashboard` 仍兼容到 `ui.main`、`ui.layout`、`ui.dashboard`。

### Common Rules

- 鉴权：除 `auth.*` 外，接口默认要求 verify-hub `session_token`。
- 授权：按用户类型做方法级授权。
- 分页：`page`/`page_size` 或 `cursor`/`limit`。
- 排序：`sort` + `order`。
- 过滤：`query`。
- 版本策略：新增字段向后兼容；破坏性变更使用新 method 名称。

### Namespace Status Matrix

| Namespace or Method Group | Status | Notes |
| --- | --- | --- |
| `ui.main` / `ui.layout` / `ui.dashboard` | Implemented | legacy aliases `main` / `layout` / `dashboard` are still accepted |
| `auth.login` / `auth.logout` / `auth.refresh` / `auth.verify` | Implemented | current browser auth flow depends on them |
| `system.overview` / `system.status` / `system.metrics` | Implemented | used by dashboard and monitor views |
| `system.logs.list` / `system.logs.query` / `system.logs.tail` / `system.logs.download` | Implemented | log download also has an HTTP helper path |
| `system.config.test` | Implemented | mostly diagnostic |
| `apps.list` / `apps.version.list` | Implemented | install/update lifecycle is still planned-heavy |
| `network.overview` / `network.metrics` | Implemented | interface and firewall config remain planned-heavy |
| `zone.overview` / `zone.config` | Implemented | route naming differs from older PRD wording |
| `gateway.overview` / `gateway.config` / `gateway.file.get` | Implemented | route/rpc aliases exist for overview/config |
| `container.overview` / `container.action` | Implemented | aliases for `containers.*` and `docker.*` also exist |
| `chat.*` | Implemented | Message Hub wrapper over `msg-center`; current implementation is still hosted by the control-panel service during migration |
| `ai.overview` / `ai.provider.list` / `ai.model.list` / `ai.policy.list` / `ai.diagnostics.list` | Implemented | control_panel facade over AI model management state for the desktop AI Models window |
| `ai.provider.test` | Implemented | control_panel calls `/kapi/aicc` on behalf of the desktop AI Models window for lightweight provider diagnostics |
| `ai.reload` | Implemented | control_panel requests `service.reload_settings` on the AICC service after config changes |
| `ai.provider.set` / `ai.model.set` / `ai.policy.set` | Implemented | control_panel persists provider/model/policy edits into system config owned by the AI Models facade |
| `ai.message_hub.thread_summary` | Implemented | control_panel gathers the current direct thread, picks the Message Hub summary model policy, and asks AICC for a compact thread summary |

#### MiniMax Notes

- `MiniMax` is managed from the `AI Models` desktop window as provider id `minimax-main`.
- The preferred runtime endpoint is Anthropic-compatible rather than OpenAI-compatible.
- Current expected endpoints are:
  - `https://api.minimax.io/anthropic/v1`
  - `https://api.minimaxi.com/anthropic/v1`
- `control_panel` stores the provider management state and API key, then writes runnable MiniMax config into AICC-facing settings.
- Current MiniMax aliases are:
  - `minimax-code-plan`
  - `minimax-api`
- Current curated MiniMax model list in the provider editor is:
  - `MiniMax-M2.5`
  - `MiniMax-M2.5-highspeed`
  - `MiniMax-M2.1`
  - `MiniMax-M2.1-highspeed`
  - `MiniMax-M2`
| `notification.list` | Implemented | broader notification management is planned |
| `files.browse` | Partially implemented | current UI still prefers HTTP `/api/resources*` |
| `user.*` | Planned | dispatch exists but current handlers are placeholders |
| `storage.*` | Planned | volume/disk/SMART model documented in PRD, not yet landed |
| `share.*` | Planned | current share UI runs on HTTP `/api/share*` |
| `files.*` except browse | Planned | current CRUD/upload/download contract is HTTP-first |
| `backup.*` | Planned | job model exists only in PRD today |
| `device.*` | Planned | placeholder handlers only |
| `security.*` | Planned | 2FA and API key surface still roadmap-level |
| `file_service.*` | Planned | protocol config model is PRD-driven |
| `iscsi.*` / `snapshot.*` / `replication.*` / `sync.*` | Planned | storage service family |
| `quota.*` / `acl.*` / `recycle_bin.*` / `index.*` / `search.*` / `media.*` / `download.*` | Planned | some neighboring HTTP abilities exist, but not these kRPC namespaces |
| `vm.*` / `cert.*` / `proxy.*` / `vpn.*` / `power.*` / `time.*` / `hardware.*` / `audit.*` / `antivirus.*` | Planned | modeled in PRD, not active in code |
| `sys_config.*` | Partially implemented | backend dispatch includes this family, but full contract should be normalized separately |
| `scheduler.*` / `node.*` / `task.*` / `verify.*` / `repo.*` / `msgbus.*` / `nginx.*` / `k8s.*` / `slog.*` / `rbac.*` / `runtime.*` | Planned | dispatch stubs and naming reservations exist |

### PRD Namespace Families Retained As Planned

- Storage family: `storage.*`, `quota.*`, `acl.*`, `recycle_bin.*`, `snapshot.*`, `replication.*`, `sync.*`
- File/share family: `share.*`, `files.*`, `index.*`, `search.*`, `media.*`, `download.*`
- System operations family: `backup.*`, `device.*`, `security.*`, `audit.*`, `power.*`, `time.*`, `hardware.*`, `antivirus.*`
- Service integration family: `file_service.*`, `iscsi.*`, `cert.*`, `proxy.*`, `vpn.*`, `nginx.*`, `k8s.*`, `repo.*`, `msgbus.*`, `slog.*`
- Platform coordination family: `scheduler.*`, `node.*`, `task.*`, `verify.*`, `rbac.*`, `runtime.*`

这些族群继续保留在规格里，是为了守住命名空间和产品意图，但不应被误读为当前已实现能力。

### Method-Level Contract Tables

#### `ui.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `ui.main` | Implemented | `session_token?` | placeholder object with `test` | also available as `main` |
| `ui.layout` | Implemented | `session_token` | `RootLayoutData`-like payload | also available as `layout` |
| `ui.dashboard` | Implemented | `session_token` | `DashboardState`-like payload | also available as `dashboard` |

#### `auth.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `auth.login` | Implemented | `username`, `password`, `appid`, `source_url`, `login_nonce`, `otp?` | session and user payload | browser currently sends hash-password, not plaintext |
| `auth.logout` | Implemented | implicit session or `session_token` | `ok` or equivalent success result | frontend ignores failures during local cleanup |
| `auth.refresh` | Implemented | `refresh_token` | new `session_token`, optional new `refresh_token` | used by auth manager |
| `auth.verify` | Implemented | `session_token`, `appid` | boolean-like verification result | cached on frontend |

#### `system.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `system.overview` | Implemented | session | `SystemOverview` | |
| `system.status` | Implemented | session | `SystemStatusResponse` | |
| `system.metrics` | Implemented | session | `SystemMetrics` | |
| `system.logs.list` | Implemented | filters optional | log entries | |
| `system.logs.query` | Implemented | query filters | `SystemLogQueryResponse` | |
| `system.logs.tail` | Implemented | tail filters | log entries | |
| `system.logs.download` | Implemented | download filters | `SystemLogDownloadResponse` | HTTP helper route also exists |
| `system.update.check` | Planned | optional | update info | dispatch placeholder |
| `system.update.apply` | Planned | `version` | `task_id` | dispatch placeholder |
| `system.config.test` | Implemented | `key?`, `service_url?`, `session_token?` | `{ key, value, version, isChanged }` | diagnostic surface |

#### `apps.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `apps.list` | Implemented | `key?` | `AppsListResponse` | reads deployed services-style data |
| `apps.version.list` | Implemented | optional | `AppsVersionListResponse` | |
| `apps.install` | Planned | `app_id`, `version?` | `task_id` | PRD-defined, handler pending |
| `apps.update` | Planned | `app_id`, `version` | `task_id` | |
| `apps.uninstall` | Planned | `app_id` | `task_id` | |
| `apps.start` | Planned | `app_id` | `ok` | |
| `apps.stop` | Planned | `app_id` | `ok` | |

#### `chat.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `chat.bootstrap` | Implemented | session | `ChatBootstrapResponse` | returns current owner scope and capability flags |
| `chat.contact.list` | Implemented | `keyword?`, `limit?`, `offset?` | `ChatContactListResponse` | wraps msg-center contact query in current owner scope |
| `chat.message.list` | Implemented | `peer_did`, `limit?` | `ChatMessageListResponse` | current minimal version merges latest inbox/outbox direct messages; no cursor contract yet |
| `chat.message.send` | Implemented | `target_did`, `content`, `thread_id?` | `ChatSendMessageResponse` | sends text chat through msg-center `post_send`; write permission is more restrictive than read flows |

#### Chat Streaming HTTP Surface

| Path | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `POST /kapi/message-hub/chat/stream` | Implemented | `session_token`, `peer_did` | `application/x-ndjson` event stream | primary Message Hub stream surface |
| `POST /kapi/control-panel/chat/stream` | Implemented | `session_token`, `peer_did` | `application/x-ndjson` event stream | legacy compatibility surface during migration |

##### `POST /kapi/message-hub/chat/stream`

- Request body
  - `session_token`: control panel session token.
  - `peer_did`: target DID whose direct-chat updates should be observed.
- Stream event types
  - `ack`: stream accepted, includes current owner scope and keepalive interval.
  - `message`: normalized `ChatMessage` update derived from `msg-center` record change.
  - `resync`: frontend should reload the current message list because a record changed but could not be normalized incrementally.
  - `keepalive`: no-op event to keep the HTTP stream alive during idle periods.
- Transport notes
  - Same origin and same control-panel service as the rest of the admin UI.
  - Streamed events are scoped by authenticated user -> owner DID mapping plus the requested peer DID.
  - No extra gateway host, no extra service port, no WebSocket handshake.

### Chat Status Notes

- 当前 `chat` 的目标是先把 control panel 内的一等消息入口建起来，而不是在第一版里替代未来独立 chat app。
- 当前 Message Hub 主入口位于 `/message-hub/chat`，desktop 图标只负责以 `_blank` 方式打开该页面。
- 当前实现只覆盖最小 direct chat 能力：owner scope bootstrap、联系人列表、单目标消息查看、文本发送。
- 当前已补上基于 `msg-center` box event 的 message-level realtime；群组控制、临时授权流、独立 chat app 互联、token-level agent stream、以及 OpenDan agent control channel 仍保留为后续扩展。

#### `network.*`, `zone.*`, `gateway.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `network.overview` / `network.metrics` | Implemented | session | `NetworkOverview` | both names map to same backend path today |
| `network.interfaces` | Planned | optional | interface list | |
| `network.interface.update` | Planned | interface config patch | `ok` | |
| `network.dns` | Planned | `servers?` | DNS config | |
| `network.ddns` | Planned | provider config | `ok` | |
| `network.firewall.rules` | Planned | optional | rules list | |
| `network.firewall.update` | Planned | `rules` | `ok` | |
| `zone.overview` / `zone.config` | Implemented | session | `ZoneOverview` | older PRD names differ |
| `gateway.overview` / `gateway.config` | Implemented | session | `GatewayOverview` | |
| `gateway.file.get` | Implemented | file selector | `GatewayFileContent` | |

#### `container.*`

| Method | Status | Request | Response | Notes |
| --- | --- | --- | --- | --- |
| `container.overview` | Implemented | session | `ContainerOverview` | aliases: `containers.overview`, `docker.overview` |
| `container.action` | Implemented | container id + action | `ContainerActionResponse` | aliases: `containers.action`, `docker.action` |
| `container.list` / `container.images` | Planned | optional | `items` | modeled in PRD, not active in current dispatch |
| `container.create` / `container.start` / `container.stop` / `container.update` / `container.delete` | Planned | container config | `container_id` or `ok` | |
| `container.image.pull` / `container.image.remove` | Planned | `image` | `ok` | |

#### Notification And Secondary Platform Families

| Method Group | Status | Contract note |
| --- | --- | --- |
| `notification.list` | Implemented | list surface exists; ack/rules/channels remain planned |
| `user.*` | Planned | full CRUD and role policy model retained from PRD |
| `storage.*` | Planned | NAS-oriented volume/disk/SMART family |
| `share.*` | Planned | current UI uses HTTP `/api/share*` instead |
| `files.*` | Planned except `files.browse` | current UI uses HTTP `/api/resources*` and upload APIs |
| `backup.*`, `device.*`, `security.*`, `file_service.*`, `iscsi.*` | Planned | service-management families |
| `snapshot.*`, `replication.*`, `sync.*`, `quota.*`, `acl.*`, `recycle_bin.*`, `index.*`, `search.*`, `media.*`, `download.*` | Planned | storage/data lifecycle families |
| `vm.*`, `cert.*`, `proxy.*`, `vpn.*`, `power.*`, `time.*`, `hardware.*`, `audit.*`, `antivirus.*` | Planned | system capability families |
| `sys_config.*`, `scheduler.*`, `node.*`, `task.*`, `verify.*`, `repo.*`, `msgbus.*`, `nginx.*`, `k8s.*`, `slog.*`, `rbac.*`, `runtime.*` | Planned or partially implemented | name reservations and stubs exist in current dispatch |

## Files HTTP API Surface

### Implemented Endpoints

- `GET /api/health`
- `GET /api/search`
- `POST /api/resources/reindex`
- `GET /api/resources/nav`
- `GET /api/recent`
- `GET|POST|DELETE /api/favorites`
- `GET /api/recycle-bin`
- `POST /api/recycle-bin/restore`
- `DELETE /api/recycle-bin/item/:id`
- `POST /api/upload/session`
- `GET|PUT|DELETE /api/upload/session/:id`
- `POST /api/upload/session/:id/complete`
- `GET|POST /api/share`
- `DELETE /api/share/:id`
- `GET /api/public/share/:id`
- `GET /api/public/dl/:id`
- `GET /api/preview/pdf/*path`
- `GET /api/thumb/*path`
- `GET|POST|PUT|PATCH|DELETE /api/resources/*path`
- `GET /api/raw/*path`

### Method-Level HTTP Notes

| Endpoint | Methods | Current use |
| --- | --- | --- |
| `/api/resources/*path` | `GET`, `POST`, `PUT`, `PATCH`, `DELETE` | browse, content read, mkdir/create, rename/move/copy, save, delete |
| `/api/upload/session` | `POST` | create upload session |
| `/api/upload/session/:id` | `GET`, `PUT`, `DELETE` | inspect session, upload chunk, cancel session |
| `/api/upload/session/:id/complete` | `POST` | finalize upload |
| `/api/share` | `GET`, `POST` | list shares, create share |
| `/api/share/:id` | `DELETE` | delete share; `GET`/`PATCH` are still planned in docs |
| `/api/public/share/:id` | `GET` | public share view payload |
| `/api/public/dl/:id` | `GET` | public file download or inline content |
| `/api/search` | `GET` | file search |
| `/api/favorites` | `GET`, `POST`, `DELETE` | star/unstar and list favorites |
| `/api/recent` | `GET` | recent files list |
| `/api/recycle-bin` | `GET` | recycle bin list |
| `/api/recycle-bin/restore` | `POST` | restore recycle-bin item |
| `/api/recycle-bin/item/:id` | `DELETE` | permanently delete recycle-bin item |
| `/api/resources/nav` | `GET` | file navigation helpers |
| `/api/resources/reindex` | `POST` | reindex metadata |
| `/api/preview/pdf/*path` | `GET` | PDF or office-to-PDF preview asset |
| `/api/thumb/*path` | `GET` | thumbnail asset |
| `/api/raw/*path` | `GET` | raw file bytes |

### ACL Rules

- 默认共享物理根目录，不再按用户名切单独物理根。
- `root/admin` 可读写全目录。
- `user/limited/guest` 默认可读 `Public` 与 `Inbox/<username>`，可写 `Inbox/<username>`。

### Notes

- Files 当前真实契约以 HTTP surface 为准，而不是旧 `files.*` / `share.*` RPC 规划。

## Public Share And Download Surface

### Implemented

- `GET /api/public/share/:id`
  - 返回分享详情和可公开查看内容。
- `GET /api/public/dl/:id`
  - 返回公开下载，支持 inline 与 download 语义差异。

### Current Preview Notes

- 已支持图片、音视频、PDF、部分 office-to-pdf、markdown/text/code 等预览链路。
- 公开预览与管理态预览在能力上应尽量保持一致，但要接受安全与体积限制。

## Planned Install, Share-App, And Publish Surfaces

本节承接 `doc/PRD/control_panel/app安装UI.md`，但只以 `Planned` 语义进入 canonical spec。

### Install Entry Surfaces

| Surface | Status | Purpose | Notes |
| --- | --- | --- | --- |
| third-party web install button | Planned | 从外部网页拉起安装流程 | 需要检测本机能力、唤起 desktop/app、并提供失败兜底 |
| `install.html` | Planned | 控制面板内的统一安装流程页 | 当前未在主 route surface 中实现 |
| BuckyOS app install guide page | Planned | 引导安装 BuckyOS App 后继续安装目标 app | 应保留原始 meta url 或安装意图 |
| desktop add-app flow | Planned | 通过粘贴 APP_META_JSON 或等价文本进入安装 | 可与 clipboard/scanner 入口融合 |
| mobile scan-to-install | Planned | 扫码进入安装流程 | 与二维码分享链路配套 |

### Planned Install Flow Stages

| Stage | Status | User-visible goal | Contract note |
| --- | --- | --- | --- |
| install confirm | Planned | 看懂 app 是什么、从哪里来、风险如何 | 需要展示 app 基本信息、来源、信任与权限摘要 |
| install advanced config | Planned | 调整 mount、路由、端口、bind 策略 | 仅暴露用户能理解且安全的配置项 |
| install progress | Planned | 跟踪 meta 拉取、校验、调度、启动、访问检查 | 应返回 `task_id` 并可查询进度 |
| install success | Planned | 打开应用、复制地址、分享、上报安装证明 | 属于安装结束后的 next actions surface |
| install failure | Planned | 告诉用户失败类别和可执行下一步 | 错误应面向人类可理解，而不只是原始日志 |

### Planned Install Contract Notes

- 安装主流程应通过 control panel 入口发起，并返回可追踪的 `task_id`。
- 安装进度模型后续应与任务体系统一，不应额外发明第二套异步状态机。
- 失败状态至少应区分：meta 无效、来源不可达、镜像拉取失败、端口冲突、权限不足、资源不足。
- 安装 UI 是产品 surface，不等于底层调度器或镜像系统的原始错误展示层。

### Planned Share-App Surfaces

| Surface | Status | Purpose | Notes |
| --- | --- | --- | --- |
| share installed app entry | Planned | 为已安装 app 生成可传播分享载体 | 不等于发布 app |
| `share_app.html` | Planned | 展示分享来源并进入安装流程 | 当前不是已实现 route |
| share link / QR / app text / text QR | Planned | 支持不同传播介质 | 应标注可用性和信任提示 |
| desktop paste-install page | Planned | 手工粘贴 app text 进入安装 | 与 add-app flow 重合 |

### Planned Trust And Store Surfaces

| Surface | Status | Purpose |
| --- | --- | --- |
| app source management | Planned | 管理 source URL、信任等级、同步状态 |
| author/contact trust management | Planned | 维护作者或联系人的信任等级 |
| share-source trust management | Planned | 管理某好友/网站/渠道的分享信任度 |
| trust explanation page | Planned | 向用户解释“为什么信任/为什么风险” |
| built-in app store | Planned | 浏览、筛选、安装、更新、卸载 app |

### Planned Payment And Publish Surfaces

| Surface | Status | Purpose |
| --- | --- | --- |
| install-time payment | Planned | 为非免费 app 提供支付流程 |
| install proof and reward view | Planned | 展示安装证明上报与奖励信息 |
| app publish surface | Planned | 支持发布 app 与分发相关流程 |

### Boundary Rules

- 这些 surface 当前全部属于 `Planned`，不能在文档或 UI 文案中伪装成已实现入口。
- app 安装分享与 Files 分享不是同一个产品能力，前者是应用分发，后者是文件访问分享。
- 安装/分享/发布相关 route 名称在真正落地前，应继续视为保留名，而不是稳定对外 contract。

## Key State Models

### Implemented Canonical Models

#### User And Session

- `StoredAccountInfo`
  - `user_name`
  - `user_id?`
  - `user_type?`
  - `session_token`
  - `refresh_token?`
- Contract notes
  - 登录成功后最关键的产物是 `session_token`。
  - `refresh_token` 用于会话续期。
  - 当前前端把账号态持久化在浏览器本地存储中。

#### Root Layout Data

- `RootLayoutData`
  - `primaryNav: NavItem[]`
  - `secondaryNav: NavItem[]`
  - `profile: UserProfile`
  - `systemStatus: SystemStatus`
- `NavItem`
  - `label`
  - `icon`
  - `path`
  - `badge?`
- `SystemStatus`
  - `label`
  - `state`
  - `networkPeers`
  - `activeSessions`

#### Dashboard And Monitor

- `DashboardState`
  - `recentEvents`
  - `dapps`
  - `quickActions`
  - `resourceTimeline`
  - `storageSlices`
  - `storageCapacityGb`
  - `storageUsedGb`
  - `devices?`
  - `disks?`
  - `cpu?`
  - `memory?`
- `SystemOverview`
  - `name`
  - `model`
  - `os`
  - `version`
  - `uptime_seconds`
- `SystemMetrics`
  - `cpu`
  - `memory`
  - `disk`
  - `network`
  - `resourceTimeline?`
  - `networkTimeline?`
  - `swap?`
  - `loadAverage?`
  - `processCount?`
  - `uptimeSeconds?`
- `SystemStatusResponse`
  - `state`
  - `warnings`
  - `services`

#### Network And Gateway

- `NetworkOverview`
  - `summary`
  - `timeline`
  - `perInterface`
- `GatewayOverview`
  - `mode`
  - `etcDir`
  - `files`
  - `includes`
  - `stacks`
  - `tlsDomains`
  - `routes`
  - `routePreview`
  - `customOverrides`
  - `notes`
- `ZoneOverview`
  - `etcDir`
  - `zone`
  - `device`
  - `sn`
  - `files`
  - `notes`

#### Container And Apps

- `ContainerOverview`
  - `available`
  - `daemonRunning`
  - `server`
  - `summary`
  - `containers`
  - `notes`
- `AppsListResponse`
  - `items`
  - `key?`
- `AppsVersionListResponse`
  - `items`
  - `key?`

### Files And Share Models

这些模型在当前代码中没有像前端 `interface.d.ts` 那样集中统一，后续应在本文件中逐步 canonicalize。

#### File Resource Entry

- Current code-backed fields
  - `name`
  - `path`
  - `is_dir`
  - `size`
  - `modified`
- Recommended normalized aliases for docs and future API cleanup
  - `isDir`
  - `modifiedAt`
  - `mimeType?`
  - `starred?`
  - `trashed?`

#### Share Item

- Current code-backed fields
  - `id`
  - `owner`
  - `path`
  - `created_at`
  - `expires_at?`
  - `password_required`
- Observed backend/public payload additions
  - `public_download_url?`
- Recommended normalized aliases for docs and future API cleanup
  - `createdAt`
  - `expiresAt`
  - `passwordProtected`
  - `updatedAt?`
  - `name?`
  - `url?`

#### Upload Session

- Current code-backed fields
  - `id`
  - `owner`
  - `path`
  - `size`
  - `chunk_size`
  - `uploaded_size`
  - `override_existing`
  - `created_at`
  - `updated_at`
- Recommended normalized aliases for docs and future API cleanup
  - `chunkSize`
  - `uploadedSize`
  - `overrideExisting`
  - `createdAt`
  - `updatedAt`
  - `status?`

#### Notification Item

- Current minimum visible model
  - `items[]`
  - `total`
- Planned richer fields from PRD
  - `id`
  - `source`
  - `level`
  - `title`
  - `detail`
  - `created_at`
  - `ack_at?`
  - `actor?`

### Model Governance Rule

- 已在 TypeScript 全局类型中稳定存在的模型，优先按 `interface.d.ts` 收敛。
- 仅在 PRD 中存在、但未被代码消费的模型，先标为 `Planned`。
- Files/share 模型后续需要单独补“canonical field table”，避免前后端各自发散。

## Compatibility And Legacy Aliases

- `main` / `layout` / `dashboard` 仍兼容到 `ui.*`。
- 旧 Files 独立登录接口 `POST /api/login` 与 `POST /api/renew` 已下线。
- 旧 PRD 中出现但未实际挂载的页面名，迁移期保留为 `Historical`。

## Implemented vs Planned Matrix

### Implemented

- 控制面板主登录与受保护路由
- 主控制面板 UI 基础页面
- 部分系统/网络/容器/应用概览型 kRPC
- 内嵌 Files HTTP 体系
- 公开分享查看与下载

### Planned

- NAS 风格完整运维矩阵
- 细粒度 ACL 与 share/files kRPC 收敛
- 安装页、发布页、内置商店、经济系统
- 完整审计、备份、快照、配额、文件服务协议配置

### Deferred But Reserved In Spec

- `login_index.html` 兼容应用登录引导模型
- `install.html` 与 `share_app.html` 安装流 UI
- `ndn/publish.html` 与 publish receipt 流程
- 超级用户签名授权页

## Acceptance And Verification Hooks

- 路由核对：`src/frame/control_panel/web/src/routes/router.tsx:22`
- kRPC dispatch 核对：`src/frame/control_panel/src/main.rs:3982`
- HTTP surface 核对：`src/frame/control_panel/src/file_manager.rs:6655`
- Files 前端消费路径核对：`src/frame/control_panel/web/src/ui/pages/FileManagerPage.tsx:1165`

## Migration Status

- `doc/PRD/control_panel/control_panel.md` 中的“命名约定”“通用字段约定”“版本策略”“模块与接口”“Files/Share 当前实现状态”主要迁移到这里。
- `doc/PRD/control_panel/SSO.md` 中 contract-level 的 token 规则主要迁移到这里。
- `doc/PRD/control_panel/app安装UI.md` 中与控制面板直接相关的 route/spec 仍可在后续以 `Planned` 形式纳入这里。
