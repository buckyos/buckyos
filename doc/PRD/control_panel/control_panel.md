# 需求清单（类NAS）

> Migration note:
> - Canonical control panel docs now live under `doc/control_panel/`.
> - Start with `doc/control_panel/README.context.md`, then read `doc/control_panel/ARCHITECTURE.context.md`, `doc/control_panel/SPEC.context.md`, and `doc/control_panel/CONTEXT.context.md`.
> - This file is retained as the main historical PRD source during migration and still contains planned material not yet normalized.

以下为控制面板的 NAS 视角功能需求，按模块划分，优先覆盖管理/可观测/安全/可扩展能力。

## 系统与运维
- 系统概览：硬件信息、系统版本、运行时间、节点健康度。
- 资源监控：CPU/内存/磁盘/网络吞吐、历史趋势、告警阈值。
- 更新管理：系统/服务版本检查、灰度更新、回滚记录。
- 日志中心：系统日志/访问日志/服务日志聚合与检索。
- 任务与调度：后台任务队列、执行状态、失败重试与通知。

## 存储与数据
- 存储池/卷管理：创建/扩容/缩容、性能与健康状态。
- 磁盘管理：SMART 状态、温度、坏道预警、替换流程。
- 文件服务：SMB/NFS/FTP/WebDAV/rsync 配置与状态。
- 共享与权限：共享目录、ACL、链接分享、有效期与审计。
- 快照/备份/恢复：定期策略、多版本、跨设备/云端备份。

## 应用与服务
- 应用列表：已安装/可用/更新，版本与运行状态。
- 服务编排：依赖关系、端口映射、资源配额、环境变量。
- 应用配置：基于 sys_config 的配置视图与快速修改。
- 应用日志：单应用日志与健康检查。

## 用户与安全
- 用户/角色管理：用户生命周期、角色权限、授权策略。
- 认证与会话：登录、二次验证、会话管理、API Key。
- 设备与客户端：接入设备列表、阻断与限速策略。
- 审计：关键操作审计、异常行为记录。

## 网络与访问
- 网络接口：IP/网关/DNS/MTU、DHCP/静态配置。
- 防火墙与端口：规则管理、端口转发、流量统计。
- 外网与 DDNS：域名/证书管理、反向代理、访问策略。

## 生态与集成
- 通知系统：邮件/短信/IM/Webhook。
- 监控集成：Prometheus/Elastic 等外部可观测对接。
- 插件机制：自定义模块、前端卡片/后端 RPC 扩展。

## PRD 拓展（控制面板）

> Canonical mapping:
> - 产品目标、边界、核心概念、用户旅程、非功能要求，迁移到 `doc/control_panel/README.context.md`
> - 运行结构、依赖、真实数据流，迁移到 `doc/control_panel/ARCHITECTURE.context.md`
> - 路由、RPC、HTTP API、状态模型，迁移到 `doc/control_panel/SPEC.context.md`
> - 命名约定、技术债、实现偏差、迁移策略，迁移到 `doc/control_panel/CONTEXT.context.md`

### 目标与边界

历史摘要：

- control panel 被定位为面向个人/家庭/小团队的 Zone 级统一管理入口。
- 它原本被设想覆盖引导、激活、启动、调度、安装升级、访问与权限等系统主链路。
- 这些高层目标已迁移到 `doc/control_panel/README.context.md`，这里不再作为主定义位置展开。

### 核心概念（快速索引）

历史摘要：

- Zone / OOD / Node / ZoneGateway / NodeGateway 是早期 control panel PRD 的核心概念骨架。
- 当前 canonical 定义已迁移到 `doc/control_panel/README.context.md` 和 `doc/control_panel/ARCHITECTURE.context.md`。

### 角色与权限模型

历史摘要：

- 早期 PRD 设想过 root 管理员、运维管理员、只读观察者、应用管理员等角色分层。
- 权限维度覆盖系统、存储、应用、网络、安全、审计、设备管理。
- “写操作需要审计”这一原则已应被视为长期保留约束。

### 典型用户旅程

历史摘要：

- 典型旅程包括首次接入、日常运维、扩容升级、外部访问四类主线。
- 这些旅程已迁移到 `doc/control_panel/README.context.md` 的产品语义层，不再在旧 PRD 中细写。

### 信息架构与导航

> Migrated-to: `doc/control_panel/README.context.md` for product IA, and `doc/control_panel/SPEC.context.md` for implemented route surface.


历史摘要：

- 早期 IA 以 Dashboard / Zone 与节点 / 系统 / 存储 / 应用 / 用户与安全 / 网络与网关 / 通知 / 日志与审计 / 设置 为一级骨架。
- “对象管理 + 任务/日志/告警闭环”是当时的导航组织原则。

### 关键功能细化

历史摘要：

- 本节曾详细枚举 Dashboard、Zone 与节点、系统运维、存储与数据、应用与服务、用户与安全、网络与网关、SN、通知事件等完整 NAS 风格能力矩阵。
- 这些内容已分流到 canonical 文档：产品语义进 `README.context.md`，运行边界进 `ARCHITECTURE.context.md`，接口与 planned surface 进 `SPEC.context.md`。
- 尚未实现的能力仍以本文件后半部 RPC 规划清单作为历史来源保留。

### 数据与状态模型（概念）

> Migrated-to: `doc/control_panel/SPEC.context.md` as canonical state models. This section remains historical until all models are normalized.


历史摘要：

- 早期状态模型覆盖 Zone、OOD、Node、Gateway、SN、事件、任务、存储、用户等对象。
- 当前 canonical model 正在 `doc/control_panel/SPEC.context.md` 中按“代码已实现 + planned 规范化字段”重建。

### 体验与交互要求

> Migrated-to: `doc/control_panel/README.context.md` for product principles, with implementation-specific caveats in `doc/control_panel/CONTEXT.context.md`.


历史摘要：

- 破坏性操作二次确认、空状态引导、列表过滤搜索批量操作、影响范围提示，是早期明确提出的 UX 要求。
- 当前 UI 风格与视觉一致性约束已迁移到 `doc/control_panel/README.context.md` 与 `doc/control_panel/CONTEXT.context.md`。

### 非功能性要求

> Migrated-to: `doc/control_panel/README.context.md`.


历史摘要：

- 本节保留的核心非功能性意图包括性能、稳定性、安全、响应式兼容。
- 这些要求已迁移到 canonical 入口文档，不再在旧 PRD 中展开。

### 指标与验收

> Historical note: these acceptance targets are not yet normalized into the canonical spec set.


历史摘要：

- 这里记录过任务成功率、MTTR、升级失败率、备份/共享成功率、节点接入时长、SN 可用性等验收指标。
- 目前这些指标尚未在 canonical 文档中完全结构化，因此保留为历史验收方向提示。

### 依赖与风险

> Migrated-to: `doc/control_panel/ARCHITECTURE.context.md` for dependency structure, and `doc/control_panel/CONTEXT.context.md` for risk and debt notes.


历史摘要：

- control panel 早期就明确依赖 sys_config、verify-hub、scheduler、gateway、slog 等基础服务。
- 多节点一致性、网络漂移、存储不可逆、SN 频繁切换抖动是长期风险主题。

## 事件通知模块方案（不新增 KernelService）

> Migrated-to: `doc/control_panel/SPEC.context.md` for contract ownership. This section remains historical/planned until the notification family is normalized there.

目标: 在不增加新内核服务的前提下, 由现有 control_panel 统一收敛事件并对外提供通知能力。

历史摘要：

- 这套方案曾设想 notification 不新增独立 kernel service，而由 `control_panel` 统一聚合、存储并提供 RPC。
- 事件来源包括 node_daemon、scheduler、gateway、repo-service、sys_config_service 等。
- 核心规划点是：本地事件存储、RBAC 过滤、webhook/email 通道、notification.* RPC、Dashboard/Notifications UI 对接。
- 详细实现尚未 canonicalize，当前仅保留为历史设计来源。

# Control Panel RPC 文档

> Canonical mapping:
> - 当前 route/RPC/HTTP contract 以 `doc/control_panel/SPEC.context.md` 为准
> - 当前真实运行结构以 `doc/control_panel/ARCHITECTURE.context.md` 为准
> - 本节保留为历史来源与待迁移详细清单

本文件集中记录 control_panel 的 RPC 规划与约定，供前后端协作实现使用。

## 接入方式

历史摘要：

- control panel RPC 历史上统一走 `POST /kapi/control-panel`。
- 请求体采用标准 kRPC 结构：`method / params / id`。
- 当前 canonical 接入定义请以 `doc/control_panel/SPEC.context.md` 为准。

## Files/Share 当前实现状态（2026-03）

> Migrated-to: `doc/control_panel/SPEC.context.md` and `doc/control_panel/ARCHITECTURE.context.md`.


历史摘要：

- Files 已并入 `control_panel` 进程，并通过 `/api/*` 暴露 HTTP surface。
- 当前 Files 主链路是 HTTP-first，而不是 `files.*` / `share.*` kRPC-first。
- Files 与 control panel 共享 verify-hub session 语义，旧独立登录接口已下线。
- Files 使用共享存储根目录与默认子账号 ACL 模型。
- 更完整的当前定义已迁移到 `doc/control_panel/ARCHITECTURE.context.md` 和 `doc/control_panel/SPEC.context.md`。

历史示例:

```json
{ "id": 1, "method": "ui.dashboard", "params": { "session_token": "..." } }
```

```json
{ "id": 1, "result": { "status": "success", "data": {} } }
```

## 命名约定

> Migrated-to: `doc/control_panel/CONTEXT.context.md` and `doc/control_panel/SPEC.context.md`.


历史摘要：

- RPC 命名统一采用 `<module>.<action>`。
- UI 历史上采用 `ui.*`，并保留 `main/layout/dashboard` 兼容别名。
- 当前 canonical 命名规则请以 `doc/control_panel/SPEC.context.md` 和 `doc/control_panel/CONTEXT.context.md` 为准。

## 通用字段约定

> Migrated-to: `doc/control_panel/SPEC.context.md`.


历史摘要：

- 这里曾定义统一的鉴权、授权、分页、排序、过滤规则。
- 这些规则已经迁移到 `doc/control_panel/SPEC.context.md`，旧 PRD 不再作为主定义位置。

## login: 管理员密码来源

> Migrated-to: `doc/control_panel/CONTEXT.context.md` as an implementation caveat once auth notes are expanded further.


历史摘要：

- 激活流程只保存 `admin_password_hash`，不保存明文密码。
- 密码最终通过 scheduler 写入 sys_config，再由 verify-hub 登录流程读取与校验。
- 这是实现侧 caveat，canonical 约束应看 `doc/control_panel/CONTEXT.context.md`。

## 版本策略

> Migrated-to: `doc/control_panel/SPEC.context.md`.


历史摘要：

- 新增字段向后兼容。
- 破坏性变更使用新 method 名称。
- 当前 canonical 版本策略已迁移到 `doc/control_panel/SPEC.context.md`。

## 模块与接口

> Migrated-to: `doc/control_panel/SPEC.context.md`. This section remains the historical detailed source list during migration.

### UI (ui.*)

> Migrated-to: `doc/control_panel/SPEC.context.md`.

历史摘要：

- `ui.main`、`ui.layout`、`ui.dashboard` 是 control panel 最早的 UI 引导接口族。
- `main` / `layout` / `dashboard` 是对应保留的 legacy alias。
- 这组接口主要负责入口占位、布局数据、仪表盘聚合数据。
- 当前请求/响应契约、字段形态、兼容策略请以 `doc/control_panel/SPEC.context.md` 为准。
- 历史实现中，dashboard 一度强调 sysinfo 实时采样和轻量资源趋势聚合。

### 认证 (auth.*)

> Migrated-to: `doc/control_panel/SPEC.context.md` and `doc/control_panel/CONTEXT.context.md`.

历史摘要：

- `auth.login`、`auth.logout`、`auth.refresh` 构成了 control panel 早期定义的认证主链路。
- 核心历史模型是 `session_token` + `refresh_token` 双 token 机制。
- `otp`、更高权限授权、兼容应用登录等扩展语义曾在不同 PRD 中出现，但当前 canonical 规则已迁移到新文档。
- 当前浏览器登录、刷新、校验与共享 session 语义请以 `doc/control_panel/SPEC.context.md` 为准。

### 用户 (user.*)

#### user.list
用途: 列出用户。

请求参数:
- page: number, optional
- page_size: number, optional
- query: string, optional

响应字段:
- items: array of user
- total: number

#### user.get
请求参数:
- user_id: string

响应字段:
- user: object

#### user.create
请求参数:
- username: string
- password: string
- email: string, optional
- roles: array, optional

响应字段:
- user_id: string

#### user.update
请求参数:
- user_id: string
- patch: object

响应字段:
- user: object

#### user.delete
请求参数:
- user_id: string

响应字段:
- ok: boolean

#### user.role.list
请求参数:
- page: number, optional
- page_size: number, optional

响应字段:
- items: array
- total: number

#### user.role.update
请求参数:
- role_id: string
- patch: object

响应字段:
- role: object

### 系统 (system.*)

> Migrated-to: `doc/control_panel/SPEC.context.md`.

历史摘要：

- `system.overview`、`system.status`、`system.metrics` 是 system 族的核心观测接口。
- `system.update.check`、`system.update.apply` 代表了早期对升级流程的规划接口。
- `system.config.test` 是偏诊断性的 system_config 连通性与读取测试接口。
- 当前哪些方法已实现、哪些仍是 planned，以及它们对应的数据模型，请以 `doc/control_panel/SPEC.context.md` 为准。

### 存储 (storage.*)

#### storage.volumes
用途: 列出卷/阵列。

响应字段:
- items: array

#### storage.volume.get
请求参数:
- volume_id: string

响应字段:
- volume: object

#### storage.volume.create
请求参数:
- name: string
- raid_level: string
- disks: array

响应字段:
- volume_id: string

#### storage.volume.expand
请求参数:
- volume_id: string
- disks: array

响应字段:
- task_id: string

#### storage.volume.delete
请求参数:
- volume_id: string

响应字段:
- ok: boolean

#### storage.disks
响应字段:
- items: array

#### storage.smart
请求参数:
- disk_id: string

响应字段:
- smart: object

#### storage.raid.rebuild
请求参数:
- volume_id: string

响应字段:
- task_id: string

### 共享 (share.*)

实现状态（当前）:
- 目前 UI 侧分享管理走 HTTP `/api/share*`，可创建、列表、删除分享链接。
- `share.*` kRPC 尚未完成落地，实现路径保留在 control_panel 内。

#### share.list
响应字段:
- items: array

#### share.get
请求参数:
- share_id: string

响应字段:
- share: object

#### share.create
请求参数:
- name: string
- path: string
- permissions: object

响应字段:
- share_id: string

#### share.update
请求参数:
- share_id: string
- patch: object

响应字段:
- share: object

#### share.delete
请求参数:
- share_id: string

响应字段:
- ok: boolean

### 文件 (files.*)

实现状态（当前）:
- 目前 UI 侧文件浏览/上传/编辑/下载走 HTTP `/api/resources*`、`/api/upload/session*`、`/api/raw*`。
- `files.*` kRPC 尚未完成落地，实现路径保留在 control_panel 内。

#### files.browse
用途: 列目录。

请求参数:
- path: string
- include_hidden: boolean, optional

响应字段:
- entries: array

#### files.stat
请求参数:
- path: string

响应字段:
- stat: object

#### files.mkdir
请求参数:
- path: string
- recursive: boolean, optional

响应字段:
- ok: boolean

#### files.delete
请求参数:
- path: string
- recursive: boolean, optional

响应字段:
- ok: boolean

#### files.move
请求参数:
- src: string
- dst: string
- overwrite: boolean, optional

响应字段:
- ok: boolean

#### files.copy
请求参数:
- src: string
- dst: string
- overwrite: boolean, optional

响应字段:
- ok: boolean

#### files.upload.init
请求参数:
- path: string
- size: number
- sha256: string, optional

响应字段:
- upload_id: string
- part_size: number

#### files.upload.part
请求参数:
- upload_id: string
- part_number: number
- data: string (base64 或二进制承载)

响应字段:
- etag: string

#### files.upload.complete
请求参数:
- upload_id: string
- parts: array

响应字段:
- ok: boolean
- path: string

#### files.download
请求参数:
- path: string

响应字段:
- url: string 或 download_token: string

### Files 分享能力迭代计划（对齐 ShareContentMgr）

#### P0（当前版本增强，先可用）
- API 补齐：`GET /api/share/:id`、`PATCH /api/share/:id`（过期时间/密码/启停）。
- 前端改造：将 `prompt` 创建分享升级为表单弹窗，支持编辑与状态展示。
- 安全增强：公共访问减少对 URL query 密码的长期依赖，改为短期访问凭据或一次性验证。
- 可观测：记录公共访问成功/失败日志，形成基础审计能力。

#### P1（能力对齐 ShareContentMgr）
- 将分享发布元数据接入 `ShareContentMgr`，引入 `name -> obj_id` 映射。
- 每次分享更新采用 sequence + CAS，保留不可变 revision 历史。
- 统一 share policy（`public` / `token_required` / `encrypted`）与扩展配置。

#### P2（运营与审计）
- 聚合访问统计（`request_count`、`bytes_sent`、`last_access_ts`），支持小时/天时间窗。
- 提供访问日志查询与图表展示，支持按状态码、来源设备、时间范围筛选。

### 备份 (backup.*)

#### backup.jobs
响应字段:
- items: array

#### backup.job.create
请求参数:
- name: string
- source: string
- target_id: string
- schedule: string

响应字段:
- job_id: string

#### backup.job.run
请求参数:
- job_id: string

响应字段:
- task_id: string

#### backup.job.stop
请求参数:
- job_id: string

响应字段:
- ok: boolean

#### backup.targets
响应字段:
- items: array

#### backup.restore
请求参数:
- target_id: string
- snapshot_id: string
- dest: string

响应字段:
- task_id: string

### 应用 (apps.*)

#### apps.list
用途: 列出当前已部署的应用/服务（基于 sys_config 的 services）。

请求参数:
- key: string, optional (默认: services)

响应字段:
- key: string
- items: array
  - name: string
  - icon: string
  - category: string
  - status: string (installed/available)
  - version: string
  - settings: object|string|null (sys_config value)

#### apps.install
请求参数:
- app_id: string
- version: string, optional

响应字段:
- task_id: string

#### apps.update
请求参数:
- app_id: string
- version: string

响应字段:
- task_id: string

#### apps.uninstall
请求参数:
- app_id: string

响应字段:
- task_id: string

#### apps.start
请求参数:
- app_id: string

响应字段:
- ok: boolean

#### apps.stop
请求参数:
- app_id: string

响应字段:
- ok: boolean

### 网络 (network.*)

#### network.interfaces
响应字段:
- items: array

#### network.interface.update
请求参数:
- name: string
- dhcp: boolean
- ip: string, optional
- gateway: string, optional
- mtu: number, optional

响应字段:
- ok: boolean

#### network.dns
请求参数:
- servers: array, optional

响应字段:
- servers: array

#### network.ddns
请求参数:
- provider: string
- hostname: string
- enabled: boolean

响应字段:
- ok: boolean

#### network.firewall.rules
响应字段:
- items: array

#### network.firewall.update
请求参数:
- rules: array

响应字段:
- ok: boolean

### 设备 (device.*)

#### device.list
响应字段:
- items: array

#### device.block
请求参数:
- device_id: string

响应字段:
- ok: boolean

#### device.unblock
请求参数:
- device_id: string

响应字段:
- ok: boolean

### 通知 (notification.*)

#### notification.list
请求参数:
- page: number, optional
- page_size: number, optional

响应字段:
- items: array
- total: number

### 日志 (log.*)

#### log.system
请求参数:
- level: string, optional
- since: string, optional
- limit: number, optional

响应字段:
- items: array

#### log.access
请求参数:
- since: string, optional
- limit: number, optional

响应字段:
- items: array

### 安全 (security.*)

#### security.2fa.enable
请求参数:
- method: string (totp/sms)

响应字段:
- secret: string, optional
- ok: boolean

#### security.2fa.disable
请求参数:
- session_token: string

响应字段:
- ok: boolean

#### security.keys
用途: API key 列表与撤销。

请求参数:
- action: string (list/revoke/create)
- key_id: string, optional

响应字段:
- items: array, optional
- key: string, optional
- ok: boolean

### 文件服务 (file_service.*)

#### file_service.list
用途: 列出文件服务状态 (SMB/NFS/AFP/FTP/WebDAV/rsync/SFTP/SSH)。

响应字段:
- items: array { name, enabled, status }

#### file_service.<proto>.get / file_service.<proto>.update
用途: 获取/更新协议服务配置。

<proto> 清单:
- smb, nfs, afp, ftp, webdav, rsync, sftp, ssh

请求参数:
- enabled: boolean, optional
- config: object, optional

响应字段:
- enabled: boolean
- config: object
- ok: boolean (update)

### iSCSI (iscsi.*)

#### iscsi.targets / iscsi.luns / iscsi.sessions
用途: 列出 target / LUN / session。

响应字段:
- items: array

#### iscsi.target.create / iscsi.target.update / iscsi.target.delete
用途: 管理 iSCSI target。

请求参数:
- target_id: string, optional
- name: string, optional
- auth: object, optional

响应字段:
- target_id: string
- ok: boolean

#### iscsi.lun.create / iscsi.lun.update / iscsi.lun.delete
用途: 管理 iSCSI LUN。

请求参数:
- lun_id: string, optional
- target_id: string, optional
- size_gb: number, optional

响应字段:
- lun_id: string
- ok: boolean

### 快照 (snapshot.*)

#### snapshot.list
用途: 列出快照。

请求参数:
- volume_id: string, optional
- share_id: string, optional

响应字段:
- items: array

#### snapshot.create / snapshot.delete / snapshot.restore
用途: 创建/删除/恢复快照。

请求参数:
- snapshot_id: string, optional
- volume_id: string, optional
- share_id: string, optional

响应字段:
- snapshot_id: string
- task_id: string
- ok: boolean

#### snapshot.schedule.list / snapshot.schedule.update
用途: 快照计划配置。

请求参数:
- schedule: object, optional

响应字段:
- items: array
- ok: boolean

### 复制 (replication.*)

#### replication.jobs
用途: 列出复制任务。

响应字段:
- items: array

#### replication.job.create / replication.job.run / replication.job.pause / replication.job.delete
用途: 复制任务管理。

请求参数:
- job_id: string, optional
- source: string, optional
- target: string, optional

响应字段:
- job_id: string
- task_id: string
- ok: boolean

#### replication.status
用途: 查看复制状态。

响应字段:
- status: object

### 同步 (sync.*)

#### sync.providers
用途: 列出云同步/厂商。

响应字段:
- items: array

#### sync.tasks
用途: 列出同步任务。

响应字段:
- items: array

#### sync.task.create / sync.task.run / sync.task.pause / sync.task.resume / sync.task.delete
用途: 同步任务管理。

请求参数:
- task_id: string, optional
- source: string, optional
- target: string, optional

响应字段:
- task_id: string
- ok: boolean

### 配额 (quota.*)

#### quota.get / quota.update / quota.defaults
用途: 获取/更新配额或默认配额。

请求参数:
- scope: string (user/share)
- target_id: string, optional
- limit_gb: number, optional

响应字段:
- items: array
- ok: boolean

### ACL/权限 (acl.*)

#### acl.get / acl.update / acl.reset
用途: 获取/更新/重置 ACL。

请求参数:
- path: string
- acl: object, optional

响应字段:
- acl: object
- ok: boolean

### 回收站 (recycle_bin.*)

#### recycle_bin.get / recycle_bin.update
用途: 获取/更新回收站设置。

请求参数:
- enabled: boolean, optional
- retention_days: number, optional

响应字段:
- enabled: boolean
- retention_days: number
- ok: boolean

#### recycle_bin.list / recycle_bin.restore / recycle_bin.delete
用途: 回收站文件操作。

请求参数:
- item_id: string, optional

响应字段:
- items: array
- ok: boolean

### 索引与搜索 (index.* / search.*)

#### index.status / index.rebuild
用途: 索引状态与重建。

响应字段:
- status: object
- task_id: string

#### search.query
用途: 文件/媒体搜索。

请求参数:
- query: string
- scope: string, optional

响应字段:
- items: array

### 媒体 (media.*)

#### media.library.scan / media.library.status
用途: 媒体库扫描与状态。

响应字段:
- status: object
- task_id: string

#### media.dlna.get / media.dlna.update
用途: DLNA 服务配置。

响应字段:
- enabled: boolean
- config: object
- ok: boolean

### 下载 (download.*)

#### download.tasks
用途: 列出下载任务。

响应字段:
- items: array

#### download.task.create / download.task.pause / download.task.resume / download.task.delete
用途: 下载任务管理 (BT/HTTP/FTP)。

请求参数:
- task_id: string, optional
- url: string, optional

响应字段:
- task_id: string
- ok: boolean

### 容器 (container.*)

#### container.list / container.images
用途: 列出容器与镜像。

响应字段:
- items: array

#### container.create / container.start / container.stop / container.update / container.delete
用途: 容器管理。

请求参数:
- container_id: string, optional
- image: string, optional
- config: object, optional

响应字段:
- container_id: string
- ok: boolean

#### container.image.pull / container.image.remove
用途: 镜像管理。

请求参数:
- image: string

响应字段:
- ok: boolean

### 虚拟机 (vm.*)

#### vm.list
用途: 列出虚拟机。

响应字段:
- items: array

#### vm.create / vm.start / vm.stop / vm.delete
用途: 虚拟机管理。

请求参数:
- vm_id: string, optional
- config: object, optional

响应字段:
- vm_id: string
- ok: boolean

#### vm.snapshot.create / vm.snapshot.restore
用途: 虚拟机快照。

请求参数:
- vm_id: string
- snapshot_id: string, optional

响应字段:
- snapshot_id: string
- ok: boolean

### 证书 (cert.*)

#### cert.list / cert.issue / cert.import / cert.delete / cert.renew
用途: 证书管理 (TLS/Let's Encrypt)。

请求参数:
- cert_id: string, optional
- domain: string, optional

响应字段:
- cert_id: string
- ok: boolean

### 反向代理 (proxy.*)

#### proxy.list / proxy.create / proxy.update / proxy.delete
用途: 反向代理规则管理。

请求参数:
- proxy_id: string, optional
- config: object, optional

响应字段:
- proxy_id: string
- ok: boolean

### VPN (vpn.*)

#### vpn.profiles
用途: 列出 VPN 配置。

响应字段:
- items: array

#### vpn.profile.create / vpn.profile.update / vpn.profile.delete
用途: VPN 配置管理。

请求参数:
- profile_id: string, optional
- config: object, optional

响应字段:
- profile_id: string
- ok: boolean

#### vpn.status / vpn.connect / vpn.disconnect
用途: VPN 连接状态/操作。

响应字段:
- status: object
- ok: boolean

### 电源 (power.*)

#### power.shutdown / power.reboot
用途: 关机/重启。

响应字段:
- ok: boolean

#### power.schedule.list / power.schedule.update
用途: 定时开关机。

请求参数:
- schedule: object, optional

响应字段:
- items: array
- ok: boolean

#### power.wol.send
用途: 发送 Wake-on-LAN。

请求参数:
- mac: string

响应字段:
- ok: boolean

### 时间 (time.*)

#### time.get / time.update
用途: 获取/更新系统时间与时区。

请求参数:
- timezone: string, optional
- datetime: string, optional

响应字段:
- time: string
- timezone: string
- ok: boolean

#### time.ntp.get / time.ntp.update
用途: NTP 设置。

请求参数:
- servers: array, optional
- enabled: boolean, optional

响应字段:
- enabled: boolean
- servers: array
- ok: boolean

### 硬件 (hardware.*)

#### hardware.sensors / hardware.fans
用途: 温度/风扇/电压等硬件状态。

响应字段:
- items: array

#### hardware.led.update
用途: 设备 LED/指示灯配置。

请求参数:
- mode: string

响应字段:
- ok: boolean

#### hardware.ups.get / hardware.ups.update
用途: UPS 配置与状态。

请求参数:
- config: object, optional

响应字段:
- status: object
- ok: boolean

### 审计 (audit.*)

#### audit.events / audit.export
用途: 审计事件查询与导出。

请求参数:
- since: string, optional
- limit: number, optional

响应字段:
- items: array
- url: string, optional

### 杀毒 (antivirus.*)

#### antivirus.status
用途: 查看杀毒服务状态。

响应字段:
- status: object

#### antivirus.scan
用途: 触发扫描。

请求参数:
- path: string, optional
- deep: boolean, optional

响应字段:
- task_id: string

#### antivirus.signatures.update
用途: 更新病毒库。

响应字段:
- ok: boolean

#### antivirus.quarantine.list / antivirus.quarantine.delete
用途: 隔离区管理。

请求参数:
- item_id: string, optional

响应字段:
- items: array
- ok: boolean

### 系统配置中心 (sys_config.*)

#### sys_config.get / sys_config.set / sys_config.list / sys_config.tree / sys_config.history
用途: 读取/写入/枚举/查看树结构/查看配置历史。

请求参数:
- key: string, optional
- value: string, optional
- prefix: string, optional
- depth: number, optional (tree)

响应字段:
- key: string
- value: string
- items: array
- tree: object
- ok: boolean

### 调度器 (scheduler.*)

#### scheduler.status / scheduler.queue.list / scheduler.task.list / scheduler.task.cancel
用途: 调度状态与任务队列管理。

请求参数:
- task_id: string, optional

响应字段:
- status: object
- items: array
- ok: boolean

### 节点与激活 (node.*)

#### node.list / node.get / node.services.list
用途: 节点列表与详情。

请求参数:
- node_id: string, optional

响应字段:
- items: array
- node: object

#### node.restart / node.shutdown
用途: 节点重启/关机。

请求参数:
- node_id: string

响应字段:
- ok: boolean

#### node.activate / node.activation.status
用途: 节点激活与状态。

请求参数:
- node_id: string

响应字段:
- status: object
- ok: boolean

### 任务管理 (task.*)

#### task.list / task.get / task.cancel / task.retry / task.logs
用途: 任务查询与控制。

请求参数:
- task_id: string, optional

响应字段:
- items: array
- task: object
- logs: array
- ok: boolean

### 验证中心 (verify.*)

#### verify.status / verify.sessions / verify.session.revoke
用途: 认证会话状态管理。

请求参数:
- session_id: string, optional

响应字段:
- status: object
- items: array
- ok: boolean

### 仓库服务 (repo.*)

#### repo.sources / repo.pkgs / repo.tasks
用途: 包仓与任务状态。

响应字段:
- items: array

#### repo.install / repo.publish / repo.sync
用途: 安装/发布/同步包。

请求参数:
- pkg_id: string, optional
- source: string, optional

响应字段:
- task_id: string
- ok: boolean

### 消息总线 (msgbus.*)

#### msgbus.status / msgbus.topics / msgbus.publish
用途: 总线状态与消息发布。

请求参数:
- topic: string, optional
- payload: object, optional

响应字段:
- status: object
- items: array
- ok: boolean

### Web 服务 (nginx.*)

#### nginx.status / nginx.sites / nginx.site.update / nginx.reload
用途: 站点与反代配置管理。

请求参数:
- site_id: string, optional
- config: object, optional

响应字段:
- status: object
- items: array
- ok: boolean

### K8s 服务 (k8s.*)

#### k8s.status / k8s.nodes / k8s.deployments / k8s.deployment.scale
用途: 集群与工作负载管理。

请求参数:
- deployment: string, optional
- replicas: number, optional

响应字段:
- status: object
- items: array
- ok: boolean

### Slog 日志服务 (slog.*)

#### slog.status / slog.streams / slog.query
用途: 日志采集与查询。

请求参数:
- query: string, optional
- limit: number, optional

响应字段:
- status: object
- items: array

### 网关与 Zone (gateway.* / zone.*)

#### gateway.status / gateway.routes.list / gateway.routes.update
用途: 网关路由管理。

请求参数:
- routes: array, optional

响应字段:
- status: object
- ok: boolean

#### zone.info / zone.config.get / zone.config.update / zone.devices.list
用途: zone 配置与设备管理。

请求参数:
- device_id: string, optional
- config: object, optional

响应字段:
- info: object
- items: array
- ok: boolean

### RBAC 与权限 (rbac.*)

#### rbac.model.get / rbac.model.update / rbac.policy.get / rbac.policy.update
用途: RBAC 模型与策略管理。

请求参数:
- model: string, optional
- policy: string, optional

响应字段:
- model: string
- policy: string
- ok: boolean

### 运行时 (runtime.*)

#### runtime.info / runtime.reload
用途: 运行时状态与配置热加载。

响应字段:
- info: object
- ok: boolean
