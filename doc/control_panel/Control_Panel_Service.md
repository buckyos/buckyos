# BuckyOS Control Panel Service 技术需求文档

> 本文件描述 `src/frame/control_panel` 中 **Control Panel Service** 的技术需求与设计。
> 内容发布与分享子系统（Share Content Mgr / Bucky-CMS）有独立文档 [`ShareContentMgr.md`](./ShareContentMgr.md)，本文不再展开。

---

## 1. 概述 (Overview)

### 1.1 定位

Control Panel Service 是 Zone 内的**核心资源管理服务**，其本质是对系统里若干核心实体的**“写功能”进行统一收口管理**：

* **Users（用户）**、**Devices（设备）**、**Apps（应用）**、**Agents（智能体）** 这四类实体的详细信息与权限，都由本服务统一读写。
* 这些实体的“写”往往不是一次单 key 写入，而是需要对 `system_config` 中**多个路径做原子事务性写入**（例如创建一个用户要同时写 `settings` / `doc` / `key` 三个路径），并伴随**权限校验**（谁能写、能写谁）。
* 把这些跨路径、带权限的写操作集中到一个服务里，避免每个前端/客户端各自拼装事务、各自判断权限，是本服务存在的根本原因。

除核心实体管理外，本服务还承载 **App 安装协议（App Installer）**、**UI Session 管理（登入/登出/取当前用户）**，以及一组面向运维的**只读诊断视图**（Zone / Gateway / Container / Dashboard / System Logs / AICC Settings）。

### 1.2 为什么需要“统一写功能管理”

1. **原子性**：一个逻辑写操作可能对应 `system_config` 中多个 key 的写入，必须全部成功或全部失败，否则系统会进入半成品状态（例如用户有 `settings` 但没有 `key`）。本服务通过 `system_config` 的 `exec_tx`（多 KVAction 事务）保证原子性。
2. **权限**：`system_config` 的某些路径（如 `users/*`、`agents/*`）受 RBAC 闸门保护。本服务负责在写入前做角色校验（`Admin`/`Root` vs 本人 vs 普通用户），并**以调用者自己的 token** 去访问 `system_config`，而非滥用服务级 token 越权。
3. **协议**：新增 App 是系统的核心写操作之一，涉及从仓库下载内容、写 spec、等待调度器分配实例等多步骤异步流程。这部分被固化为 **App 安装协议**，由 App Installer 实现。

### 1.3 部署形态

* 以 **kRPC KernelService** 形态启动（`start_control_panel_service()`，见 [main.rs:1119](../../src/frame/control_panel/src/main.rs)）。
* 运行时身份：`CONTROL_PANEL_SERVICE_NAME`，主服务端口 `CONTROL_PANEL_SERVICE_PORT`，HTTP server id 为 `"control-panel"`。
* 主 RPC 入口：`POST /kapi/control-panel`（同时兼容 `/kapi/message-hub`）。
* 后端数据真相源：`system_config` 服务（KV 配置树）+ 少量本地配置文件（`/opt/buckyos/etc/*`，仅供只读诊断）。

---

## 2. 领域模型 (Domain Model)

本服务管理四类核心实体。它们在 `system_config` 中各有一组路径，详细信息与权限均挂在这些路径上。

### 2.1 User（用户）

* 身份与账号信息，核心结构 `UserSettings`（`user_type` / `state` / 密码哈希 / `is_local` / 是否允许改密 等），来自 `buckyos-api`。
* 关联结构：`UserPrivateProfile`（用户私有展示资料，保存在独立 profile 路径）、`UserContactSettings`（DID / groups / tags / 消息平台绑定，保存在 profile 的私有扩展中）、`UserTunnelBinding`（消息平台账号绑定）。
* 状态机 `UserState`：`Active`（正常，可签发/使用 session）/ `Pending`（已邀请未激活）/ `Deleted`（软删除，记录保留但禁止登录）。
* 角色 `UserType`：`Root`（Zone 拥有者，不可删除/不可降级）/ `Admin`（管理员）/ `User`（普通）/ `Limited`（受限，改密受 `allow_password_change` 控制）/ `Guest`。
* `system_config` 路径：

  | 路径 | 内容 |
  |---|---|
  | `users/{user_id}/settings` | `UserSettings` JSON（账号主数据） |
  | `users/{user_id}/profile` | `UserPrivateProfile` JSON（普通用户可自行读写） |
  | `users/{user_id}/doc` | DID Document / OwnerConfig（身份与公开资料） |
  | `users/{user_id}/key` | ED25519 私钥（PEM，仅创建时写一次） |
  | `services/control_panel/user_invites/{invite_id}` | 邀请记录 `UserInviteRecord` |

### 2.2 Device（设备）

* 当前为**只读实体**：Control Panel 不提供设备的增删改 RPC；设备身份在 OS 引导/置备阶段确定。
* 设备元数据由 `zone_mgr` 从本地文件读取并聚合：`node_identity.json`、`{device_did}/did.json` 等，字段含 `device_name` / `device_did` / `device_type` / `net_id`。
* 设备隶属于 Zone（当前设计下 Zone 通常对应一台 OOD 设备），通过 `zone.overview` 暴露其信息。

### 2.3 App（应用）

* 核心结构 `AppServiceSpec`（`app_doc` + `user_id` + `app_index` + `permission` + `enable` + `expected_instance_count` + `state` + `spec_config`）。`permission` 保存安装时最终批准的权限项。
* `AppDoc` 描述应用本体：`name`(app_id) / `version` / `author` / `owner` / `show_name` / `presentation` / `pkg_list`（子包：web / agent / agent_skills / agent_tools / docker image）/ `service_config_tips` / `tags` / `categories` 等。
* 生命周期 `ServiceState`：`New → Running / Stopped / Stopping / Restarting / Updating / Deleted`。
* `system_config` 路径：

  | 路径 | 内容 |
  |---|---|
  | `users/{user_id}/apps/{app_id}/spec` | 用户安装的普通 App spec |
  | `users/{user_id}/agents/{app_id}/spec` | 用户安装的 Agent 型 App spec |
  | `system/apps/{app_id}/spec` | 系统内置 App（`messagehub` / `homestation` / `content-store` 等，作者 `did:bns:buckyos`） |
  | `services/{app_id}@{user_id}/instances/{node_id}` | 实例运行状态上报（节点守护进程写） |

### 2.4 Agent（智能体）

Agent 在系统中有**两副面孔**，需要区分：

1. **作为身份/账号**（由 `user_mgr` 管理）：与用户对称，有自己的 DID、profile、消息通道绑定。路径：

   | 路径 | 内容 |
   |---|---|
   | `agents/{agent_id}/doc` | Agent DID Document / 身份与归属元数据 |
   | `agents/{agent_id}/settings` | Agent 配置（`owner_user_id` / display_name / state / profile / bindings） |
   | `agents/{agent_id}/key` | ED25519 私钥（PEM，创建时写一次） |

2. **作为可部署服务**（由 App Installer 管理）：`AppDoc.get_app_type() == AppType::Agent`，安装/启停/升级与普通 App **走同一套流程**，spec 存于 `users/{user_id}/agents/{app_id}/spec`。`agent.list` 以 `agents/*` 身份目录为准，同时补充匹配 spec 的 `app_doc`、`state`、`user_id` 等服务维度字段用于展示。

---

## 3. 统一“写功能”管理 (Transactional Writes & Authorization)

这是本服务的设计核心，所有实体的写操作都遵循下面两条机制。

### 3.1 原子多路径写：`exec_tx`

对 `system_config` 的多路径写入通过 `SystemConfigClient::exec_tx(tx, None)` 完成。`tx` 是 `HashMap<String, KVAction>`，`KVAction` 至少包含：

* `Create(value)`：创建新 key，已存在则整事务失败；
* `Set(value)`：写入/覆盖。

整个事务**全成功或全失败**，由 `system_config` 层保证，本服务不实现应用层回滚。典型事务：

* **创建用户**（[user_mgr.rs](../../src/frame/control_panel/src/user_mgr.rs)）：一次事务写 `users/{uid}/settings` + `users/{uid}/doc` + `users/{uid}/key` + `users/{uid}/profile` 四路径。
* **创建 Agent**：一次事务写 `agents/{id}/doc` + `agents/{id}/settings` + `agents/{id}/key`。
* **创建邀请**：一次事务写 `services/control_panel/user_invites/{invite_id}`，若指向已存在用户则同时写其 `settings`（置 Pending）。

> ⚠️ **非原子的尾随写**：RBAC 策略（`system/rbac/policy`）是在主事务**之后**以 `append` 追加的，不在同一事务内。即“账号已建好但 RBAC 组未追加”是一个理论上可能的中间态，文档与实现需对此保持知情（后续可考虑纳入同一事务或补偿）。

### 3.2 权限闸门：调用者 token + RBAC

* **不滥用服务 token**：访问受保护路径时，本服务用**调用者自己的 session token** 构造 `SystemConfigClient`（`system_config_client_for_caller()`），让 `system_config` 侧的 RBAC 直接对调用者生效，而不是用服务级 token 绕过权限。
* **RBAC 模型**：基于角色（`UserType`）+ Casbin 风格策略，策略文本存 `system/rbac/policy`，按行追加，如 `g, {user_id}, users`、`g, {user_id}, admin`。
* **路径闸门**（来自 boot 模板约定）：`users/*`、`agents/*` 等路径对普通调用者受限；OOD 设备 token 对 `users/*/apps/*`、`users/*/agents/*` 有读写、对 `users/*/doc`、`users/*/settings` 等有只读。
* **Handler 级角色校验**：每个写 handler 先 `require_rpc_principal(principal)?` 取得已认证主体，再按操作类型校验：
  * `require_admin(principal)` — 仅 `Admin`/`Root`；
  * `require_self_or_admin(principal, target_user_id)` — 本人或管理员。

---

## 4. 详细功能需求 (Functional Requirements)

服务对外以 kRPC 暴露，方法名 → handler 的分发集中在 `handle_rpc_call()`（[main.rs](../../src/frame/control_panel/src/main.rs)）。下表按实体分组列出方法与权限。

### 4.1 用户管理 (User Management)

| 方法 | 权限 | 说明 |
|---|---|---|
| `user.list` | 已登录 | 列出用户（可选 `include_deleted`） |
| `user.get` | 本人/Admin | 取单用户详情；profile 私有扩展中的 `contact` / `did_document` 仅本人或 Admin 可见 |
| `user.create` | **Admin** | 事务创建 settings+doc+key，追加 RBAC 组；不允许创建 Root |
| `user.update` | 本人/Admin | 改 profile 中的显示名等 |
| `user.update_contact` | 本人/Admin | 改 profile 私有扩展中的 DID/groups/tags/消息平台绑定 |
| `user.profile.get` / `user.profile.set` | 本人/Admin | `users/{uid}/profile` 中的私有 profile（DID profile 只读，响应中两源合并） |
| `user.set_msg_tunnel` / `user.remove_msg_tunnel` | 本人/Admin | 增删消息平台账号绑定 |
| `user.invite.create` | **Admin** | 生成邀请（可预建 Pending 用户或绑定已有 DID），返回 `invite_url` |
| `user.invite.get` | **公开** | 凭 `invite_id` 读邀请详情（含过期、zone_did/host） |
| `user.invite.accept` | **公开** | 凭 `invite_id` + `owner_config` 接受邀请并激活账号 |
| `user.delete` | **Admin** | 软删除（置 Deleted），不可自删 |
| `user.change_password` | 本人/Admin | 本人改密受 `allow_password_change` 限制 |
| `user.change_state` | **Admin** | Active/Pending/Deleted 迁移 |
| `user.change_type` | **Admin** | 改角色；不能提升到 Root，不能改 Root |

### 4.2 设备管理 (Device)

* 无写接口。设备信息通过 `zone.overview`（见 §4.6）以只读形式暴露。设备的注册/绑定在 OS 引导与置备层完成，不在本服务范围。

### 4.3 智能体管理 (Agent — 身份维度)

由 `user_mgr` 管理，**均要求 Admin**（Agent 是 Zone 级资源）：

| 方法 | 说明 |
|---|---|
| `agent.list` / `agent.get` | 列出/查询 Agent 身份；`agent.list` 会补充匹配的用户 Agent spec 摘要 |
| `agent.create` | 事务创建 doc+settings+key |
| `agent.update` / `agent.delete` | 更新/删除 |
| `agent.profile.get` / `agent.profile.set` | Agent profile |
| `agent.set_msg_tunnel` / `agent.remove_msg_tunnel` | 消息平台绑定 |

### 4.4 应用管理 (App — 服务维度)

由 `app_servcie_mgr` + `app_installer` 管理。`apps.list` 从系统内置 registry、`zone/apps/*` 和所有用户 `apps/*` 计算目标用户的最终授权集合；Agent 读取走 `agent.list` / `agent.get`，不会出现在 `apps.list`。App 的稳定操作和授权主键是 `app_instance_id=<app_id>@<owner_user_id>`。

| 方法 | 返回 | 说明 |
|---|---|---|
| `apps.list` | `{user_id, total, apps[]}` | 当前用户的有效 App；Admin 可指定 `user_id`。每项含 `app_instance_id/app_class/runtime_type/owner_user_id/availability_match` |
| `apps.details` / `app.details` | `{app_instance_id, app_class, owner_user_id, spec, ...}` | Owner/Admin 或最终允许用户读取实例详情 |
| `apps.availability.get/set/check` | policy / decision | 个人 App 的用户组、精确用户、Guest 规则；`set` 仅允许 App Owner 的 Control Panel 用户 session，并以 revision/CAS 原子更新策略、审计和 Gateway 输入 |
| `apps.install` | `{task_id}` | 入参 `{identifier, referrer?, app_class?, options?}`；`app_class` 默认 `user_installed`，`zone_installed` 仅 Admin |
| `apps.install_package` | `{task_id}` | 本地 `.pikg` 安装，入参 `{staging_handle, app_class?, options?}`；只接受 staging handle（`pikg:sha256:<hex>` 或 NDN chunk id），不接受服务端路径（D5） |
| `apps.install.confirm` | `{task_id}` | 确认 `WaitingForApproval` 的安装：`{task_id, target?, install_params?}`；权限批准使用 `install_params.permissions: PermissionItem[]`，target/params 变化会重算 plan 并重新绑定 fingerprint |
| `apps.install.retry` | `{task_id}` | 从失败/暂停 Stage 重试（不可重试错误会拒绝） |
| `apps.install.cancel` | `{task_id}` | 取消：Prepare 前直接取消；spec 已写后先回滚再取消 |
| `apps.update` | `{task_id}` | `{app_instance_id, options?}`，走与安装同一条 Stage 流水线（见 §5.3） |
| `apps.uninstall` | `{task_id}` | `{app_instance_id, remove_data?}`；卸载个人 App 时清空授权并记录审计 tombstone |
| `apps.start` / `apps.stop` | `{task_id}` / `{ok}` | 均以 `app_instance_id` 操作；Zone App 仅 Admin 管理 |
| `app.publish` | `{ok, obj_id, app_did, app_doc_id, pikg_handle, pikg_digest, pikg_path, app_doc, publish_status}` | 开发者发布：产出带 `did/doc_type` 的 App Document、`.pikg`（同一 PikgReader 自校验）并推到仓库；`publish_status=repo_stored_candidate` 表示尚未权威发布 App DID |

* **权限**：所有 handler 要求已认证主体；目标用户默认取 `principal.username`（为自己安装），给他人安装需 admin；`SYSTEM_INTERNAL` 策略（可 auto-confirm）仅限 admin。confirm/retry/cancel 只能操作本人任务（admin 例外）。
* **安全约束**：卸载删数据仅允许命中 `/opt/buckyos/data/*/{app_id}/*` 的路径；已 `Deleted` 的 app 不可 start；同一 user + App DID 只允许一个进行中的安装/升级事务。

### 4.5 UI Session 管理 (登入/登出/当前用户)

涉及两层：

**(a) 登录鉴权层**（`sys_auth_backend`）—— 处理登入/登出与 token：

* `auth.login`：取 `username` + `password`（+可选 `appid` / `app_instance_id` / `redirect_url` / `login_nonce`），从 Gateway/redirect 解出完整 App 实例后经 `verify_hub_client.login_by_password(...)` 校验可用性并签发 token pair；若带 `redirect_url` 走 SSO（生成 pending nonce，经目标 App origin 的 `/sso_callback` 回跳并写 cookie）。
* `auth.refresh` / `auth.verify` / `auth.logout` / `auth.issue_sso_token`：刷新 / 校验 / 注销 / 签发 SSO token。
* HTTP 侧：`/sso_callback` 同时写入 host-only 的短期 `buckyos_session_token`（供 gateway 在 App 页面加载前鉴权）和 host-only、`HttpOnly` 的 `buckyos_refresh_token`；`/sso_refresh` 只读取并轮换 refresh cookie，同时更新 session cookie；`/sso_logout` 吊销 refresh token 并清理两种 cookie。两种 cookie 都使用 `Path=/; SameSite=Lax`，HTTPS 请求还会使用 `Secure`，且禁止通过 `Domain` 扩散到父域或其它 App 子域。
* **取当前用户**：受保护方法在 `authenticate_session_token_for_method()` 中校验 token（`verify_trusted_session_token`），从 `sub` 解出 username，加载 `users/{username}/settings` 校验状态为 `Active`，构造 `RpcAuthPrincipal { username, user_type, owner_did }` 传给各 handler。

**(b) 桌面 UI Session 状态层**（`ui_session_mgr`）—— 持久化每个用户的桌面会话（外观/窗口布局/图标布局/小组件布局）：

* 存储路径：`users/{user_id}/desktop/{session_id}/{state_key}`，会话元数据存 `.../_meta`。
* HTTP 入口：`POST /api/desktop`，需 session token（401 if 失败），按 `action` 分发。
* 方法：`session.list` / `session.create`（生成 UUID v4）/ `session.delete` / `session.rename`；`state.get` / `state.set` / `state.delete`，后三者支持 `json_path`（点分路径）做部分读写删。任何 state 写都会刷新会话 `updated_at`。

### 4.6 只读诊断模块 (Read-only Diagnostics)

以下模块只读，服务于运维面板，列出以明确服务边界（不在“写功能”范畴）：

* **Zone**：`zone.overview` / `zone.config` — 聚合 `start_config.json` / `node_identity.json` / `did.json`，输出 zone/device/SN/DNS 自检与文件清单。
* **Gateway**：`gateway.overview` / `gateway.config` / `gateway.file.get` — 解析 `cyfs_gateway.json` / `boot_gateway.yaml` / `node_gateway.json` 等（白名单文件，单文件 ≤2MB），输出 stacks / routes / tlsDomains。
* **Container**：`container.overview` / `container.action`（start/stop/restart，带缓存与刷新锁）。
* **Dashboard / System**：`dashboard` / `system.overview` / `system.status` / `system.metrics` / `network.overview`。
* **System Logs**：`system.logs.list` / `query` / `tail` / `download`（凭 token 经 `GET /kapi/control-panel/logs/download/{token}` 下载）。
* **AICC Settings**：`ai.overview` / `ai.provider.*` / `ai.model.*` / `ai.policy.*` / `ai.diagnostics.list` / `ai.reload` 等，管理 AI 接入与路由策略。

---

## 5. App 安装协议 (App Installer Protocol)

真相源：[doc/App 安装协议.md](../App%20安装协议.md)（Draft v0.5，§14.0 为已冻结实现基线）。实现分布：
[app_install_engine.rs](../../src/frame/control_panel/src/app_install_engine.rs)（可恢复状态机）、
[app_install_resolver.rs](../../src/frame/control_panel/src/app_install_resolver.rs)（`(App DID, "app")` 解析适配）、
[app_install_planner.rs](../../src/frame/control_panel/src/app_install_planner.rs)（InstallPlan/readiness）、
[pikg.rs](../../src/frame/control_panel/src/pikg.rs)（`.pikg` 读写与校验）、
[app_install_driver.rs](../../src/frame/control_panel/src/app_install_driver.rs) +
[app_install_deployer.rs](../../src/frame/control_panel/src/app_install_deployer.rs)（各 Stage 生产实现）、
[app_install_runner.rs](../../src/frame/control_panel/src/app_install_runner.rs)（dispatch 与恢复）。

### 5.1 Stage 流水线与持久化

```
identifier / staging handle
   └─> Resolve   resolve_did(App DID, "app")，candidate 绑定（id/expected_owner 硬约束）
   └─> Inspect   按目标 Node（devices/{node}/info，非编译期 cfg）选 package，
                 产出 InstallPlan + 六维 readiness + plan fingerprint
   └─> [WaitingForApproval]  确认权限/参数；SYSTEM_INTERNAL 可显式 auto-confirm
   └─> Acquire   只下载 plan.missing（TaskManager 子任务，kevent 加速等待）；
                 offline 模式不建下载任务
   └─> Verify    重读全部 Package Meta ObjId、重哈希 pikg 内容、selector/mount 复核，
                 输出逐项 VerificationReport；Trust+Content+Config 全 ready 才继续
   └─> Prepare   构造 spec、materialize pikg 内容进 NamedStore、端口冲突检查、
                 app_index CAS 分配，先写 install_record(state=prepared)
   └─> Deploy    写 users/{uid}/apps|agents/{app}/spec —— 部署事务开始点
   └─> Activate  等 Started 实例证据 / static web 可观察证据 + 健康检查；
                 成功后才 install_record(state=installed) -> installed proof -> Task 完成
```

* 每个 Stage 完成先把完整事务状态写回 TaskManager `Task.data`（全量镜像 patch），再进入下一 Stage；Control Panel 重启后从持久数据恢复。
* 解析状态硬规则：`Revoked/Tombstoned` 不可重试且禁止任何 fallback；`Missing`（权威回答"从未发布"）与 `unknown`（没有回答）错误码不同；unknown 且无可接受 cache 进入 `WAITING_FOR_TRUST_RESOLUTION`（任务 Paused，可 retry）。
* 错误全部结构化（`InstallError{stage, code, retryable, action}`），进 Task.data 与 install_record。

### 5.2 Runner 与恢复

安装任务只能由 Control Panel 已鉴权的 `apps.*` 业务接口创建（beta2.2 起 TaskMgr 无 runner 派发语义）。dispatch 路径收敛到同一幂等入口：RPC 侧创建后立即本地执行（低延迟正常路径）、MsgQueue 持久投递（fetch 后处理完成才 ack）、启动扫描 + 60s sweep 兜底；每条路径都以 TaskManager 为真相源重新读取任务（按 `task_type = app.install / app.update` 过滤自己的任务）。

### 5.3 升级 / 卸载

* **升级**（`apps.update`）：同一 Stage 流水线；以权威 Resolve 的 App Document Object ID / `document_version` 判定新版本（不再取 Repo 中 semver 最新）。Prepare 完成前旧版本持续运行；写新 spec 触发调度器切换；新版本 Activate 失败自动恢复旧 spec（本机部署回滚，不改变 resolver 状态），并作废 Deploy 输出供 retry 重写。权限/target/参数变化必须重新确认。
* **卸载**（`run_uninstall_task`）：内部 stop → 置 `spec.state=Deleted` 并写回 → 等实例移除 →（可选）删数据目录（受路径安全约束）→ 标记完成。

### 5.4 记录与凭证

* in-flight 真相源：TaskManager `Task.data`；长期记录：`users/{uid}/apps|agents/{app}/install_record`（含 DID 解析快照、package meta ids、pikg digest、target、状态、task id、proof id）。
* installed proof 只在 Activate + 健康检查成功后写入 Repo（`ACTION_TYPE_INSTALLED`，details 固化 `did_resolution` 快照；本地 override 不得伪装 Anchored）；RepoService 不可用时安装仍成功、proof 跳过并留日志。

---

## 6. 公开访问 URL 清单 (Public / Anonymous Access)

“是否允许公开访问”分**后端 RPC/HTTP 闸门**与**前端路由闸门**两层。

### 6.1 后端：公开 RPC 方法

未登录（无 session token）即可调用，由 `is_public_rpc_method()`（[sys_auth_backend.rs](../../src/frame/control_panel/src/sys_auth_backend.rs)）白名单控制；命中者跳过 token 校验，`principal` 为 `None`，其余方法一律要求有效 token，否则 `InvalidToken`。

| 公开 RPC 方法 | 用途 |
|---|---|
| `auth.login` | 用户名/密码登录 |
| `auth.refresh` | 刷新 token |
| `auth.verify` | 校验 token |
| `auth.logout` | 注销 |
| `auth.issue_sso_token` | 签发 SSO token |
| `user.invite.get` | 凭邀请链接读邀请详情 |
| `user.invite.accept` | 凭邀请链接接受邀请、激活账号 |

### 6.2 后端：公开 HTTP 路由

| 路由 | 方法 | 用途 |
|---|---|---|
| `/sso_callback` | GET | SSO 回调（内部校验 nonce 与 redirect_url） |
| `/sso_refresh` | POST | 凭 refresh cookie 刷新会话 |
| `/sso_logout` | POST | 注销并清会话 Cookie |
| `/`（静态 UI 回退） | GET | 提供登录页等前端静态资源 |

> `POST /api/desktop`（桌面 UI 状态）与 `POST /kapi/control-panel`（除上述公开方法外）**均需登录**。

### 6.3 前端：登录可选路由

桌面前端 `bootstrap()` 默认在无登录态时强制跳转 `/login`；以下前缀豁免该跳转（页面自身负责在登出态下渲染），见 [publicRoutes.ts](../../src/frame/desktop/src/publicRoutes.ts)：

```ts
export const PUBLIC_ROUTE_PREFIXES = ['/login', '/userprofile'] as const
// 匹配精确路径或其子路径，如 /userprofile/{user} 也豁免
```

| 前缀 | 用途 |
|---|---|
| `/login` | 登录页 |
| `/userprofile` | 可分享的公开用户资料页（渲染某用户的公开信息，登出态可见） |

---

## 7. 数据存储与配置树 (Storage Layout)

后端真相源是 `system_config`（KV 配置树）。诊断模块另读 `/opt/buckyos/etc/*` 本地文件（只读）。

| 实体 | 路径 | 写入方式 |
|---|---|---|
| User | `users/{uid}/settings` · `users/{uid}/doc` · `users/{uid}/key` · `users/{uid}/profile` | `exec_tx` 原子四写 |
| User Invite | `services/control_panel/user_invites/{invite_id}` | `exec_tx`（可含目标用户 settings） |
| Agent（身份） | `agents/{id}/doc` · `agents/{id}/settings` · `agents/{id}/key` | `exec_tx` 原子三写 |
| App / Agent（服务） | `users/{uid}/apps\|agents/{app_id}/spec` | 单次原子 `set` |
| 系统内置 App | `system/apps/{app_id}/spec` | 合成/内置 |
| App 实例状态 | `services/{app_id}@{uid}/instances/{node_id}` | Node Daemon 上报 |
| RBAC 策略 | `system/rbac/policy` | `append`（事务后追加，注意非原子） |
| UI Session | `users/{uid}/desktop/{session_id}/{state_key}` · `.../_meta` | 单次 `set`，写后刷新 `_meta.updated_at` |

---

## 8. 关键技术要点

1. **写收口**：四类核心实体的所有写都经本服务，统一处理事务与权限，避免客户端各自拼装。
2. **原子事务**：跨路径写用 `exec_tx`；唯一已知的尾随非原子点是 RBAC `append`，需知情并择机收敛。
3. **最小越权**：受保护路径用调用者 token 访问 `system_config`，RBAC 在 `system_config` 侧对调用者直接生效。
4. **异步任务化**：App 安装类重操作返回 `task_id` + 后台任务推进 + 仓库追加审计凭证，进度可观测、操作可审计。
5. **公开面最小化**：未登录可达面被收敛为 7 个公开 RPC 方法 + 3 个 SSO HTTP 路由 + 静态 UI；前端再以 `publicRoutes` 显式列白 `/login`、`/userprofile`。
