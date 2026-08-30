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

* 核心结构 `AppServiceSpec` 以 `app_instance_id/app_did/owner_user_id` 绑定身份，保存独立 `AppDoc` snapshot、DeploymentIdentity、批准权限、运行状态与 service config；`app_name/app_host_name/app_index` 是 scheduler 从 AppRegistry 投影的只读字段。
* `AppDoc` 是独立 schema：它 flatten/inherit `BaseContentObject` 以保持 Named Content Object 的流转语义，其中 `did` 是身份、`name/tags/categories/references` 是非身份内容元数据；`version` 是 App 语义版本，`presentation/show_name` 只用于展示。`pkg_list`、selector/runtime/SDK requirements、permissions 和 service config tips 不继承 PackageMeta。
* 生命周期 `ServiceState`：`New → Running / Stopped / Stopping / Restarting / Updating / Deleted`。
* `system_config` 路径：

  | 路径 | 内容 |
  |---|---|
  | `users/{user_id}/apps/{app_id}/spec` | 用户安装的普通 App spec |
  | `users/{user_id}/agents/{agent_id}/spec` | `AgentSpec` identity + `AgentServiceBinding` |
  | `services/{app_instance_id}/instances/{node_id}` | 实例运行状态上报（节点守护进程写） |

### 2.4 Agent（智能体）

Agent identity 与承载它的 runtime App 是两个独立对象：

1. **作为身份/账号**（由 `user_mgr` 管理）：与用户对称，有自己的 DID、profile、消息通道绑定。路径：

   | 路径 | 内容 |
   |---|---|
   | `agents/{agent_id}/doc` | Agent DID Document / 身份与归属元数据 |
   | `agents/{agent_id}/settings` | Agent 配置（`owner_user_id` / display_name / state / profile / bindings） |
   | `agents/{agent_id}/key` | ED25519 私钥（PEM，创建时写一次） |

2. **作为 runtime binding**：`users/{owner}/agents/{agent_id}/spec` 保存 `AgentSpec`，其中 `AgentServiceBinding` 精确指向普通 `AppInstanceId + service_name`。多个 Agent 可以共享同一 runtime App；删除 binding 不等于停止 runtime。

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

由 `app_servcie_mgr` + `app_installer` 管理。`apps.list` 从系统内置服务、用户 AppSpec 与 availability policy 计算目标用户的最终授权集合；Agent 读取走 `agent.list` / `agent.get`，不会出现在 `apps.list`。产品身份是 canonical AppDID，可逆 key 是 AppId；Owner 范围内的安装和运行目标是 `AppInstanceId = {app_id}@{owner_user_id}`。普通 App 只有这一种 Owner 范围安装模型。

| 方法 | 返回 | 说明 |
|---|---|---|
| `apps.list` | `{user_id, total, apps[]}` | 当前用户的有效 App；每项包含 `app_id/app_did/app_instance_id/runtime_type/owner_user_id/availability_match/web_hosts` |
| `apps.details` / `app.details` | typed details | 接受统一 selector（`selector/app_instance_id/app_did/identifier`），按可见 Owner 范围唯一选择；0 个返回 NotFound，多个返回 `AMBIGUOUS_APP_TARGET` 与脱敏候选 |
| `apps.status` / `app.status` | `AppInstallationStatusSnapshot` | 聚合 install record、desired spec、active task、scheduled/runtime instance、目标/上次成功/回滚 deployment、typed deployment error 与 Static Web gateway generation evidence |
| `apps.availability.get/set/check` | policy / decision | 个人 App 的用户组、精确用户、Guest 规则；`set` 仅允许 App Owner 的 Control Panel 用户 session，并以 revision/CAS 原子更新策略与审计；scheduler 单独把 policy 投影为 Gateway access mode，不回写 AppSpec |
| `apps.staging.finalize/status/release` | `PikgStagingMetadata` | `finalize` 接受上传所得 `source_obj_id` 和 `purpose=inspect|install`，返回不可猜测 handle、digest、size、TTL；handle 绑定 principal、App、Zone 与租约，不包含路径或 digest |
| `apps.inspect` | `InstallInspection` | 对 Catalog 或 staged PIKG 做无安装副作用的首次安装/升级预检；`action=upgrade` 时生成升级 inspection |
| `apps.plan.recompute` | `InstallInspection` | 接受旧 plan、同一 source 以及新的 target/InstallParams，权威重算 plan 与 fingerprint；source/scope 变化返回 `PLAN_STALE` |
| `apps.submit` / `apps.install` | `{action, task_id?, app_instance_id, plan_fingerprint?}` | 权威动作矩阵。首次安装必须提交 `FreshInstall` plan，升级必须提交 `Upgrade` plan；相同发布返回同步 `satisfied`。提交时按 plan 内 canonical task ID 重新检查 source/scope/fingerprint，并以同一 ID 创建 TaskManager task。所有 mutation 必须提交 principal 稳定生成的 `idempotency_key` 与已展示 fingerprint |
| `apps.install.status` | `AppInstallStatusSnapshot` | 按 `task_id` 返回 typed stage、inspection、approval、verification、error、actions 与 terminal result，不暴露 Task 内部 JSON |
| `apps.install.confirm` | `{task_id}` | 只接受 `{task_id, plan_fingerprint}`；不得在 confirm 同时改 target/params |
| `apps.install.retry` | `{task_id, retry_of}` | 接受旧 `task_id` 与新的 `idempotency_key`；Failed task 保持 Terminal，新建带 `retry_of`/parent 关系的 task |
| `apps.install.cancel` | `{task_id}` | 先持久化 TaskMgr cancel intent；runner 在安全边界 ack，Deploy 后先完成回滚/收敛 |
| `apps.update.check` / `apps.upgrade.check` | typed batch availability | 有 selector 时检查单项，无 selector 时检查当前调用方可管理的全部 Catalog 安装 |
| `apps.upgrade` | batch root task | 无 selector，要求 `idempotency_key`；创建 `app.update_batch/v1` root，仅为 `UpdateAvailable` 项创建 child，结果保留 satisfied/blocked/failed/succeeded |
| `apps.start/stop/restart` | lifecycle task | 接受统一 selector 与 `idempotency_key`；持久 runner 可恢复。restart 当前只支持默认 `recreate`，`rolling` 稳定拒绝 |
| `apps.uninstall` | uninstall task | 接受统一 selector、`idempotency_key` 和显式 `data_disposition=retain|delete`；delete 只消费 Installer 生成的 data/cache manifest，不删除 external mount、Secret 或共享资源 |
| `app.publish` | `{ok, obj_id, app_did, app_doc_id, pikg_handle, pikg_digest, pikg_path, app_doc, publish_status}` | 开发者发布：产出带 `did/doc_type` 的 App Document、`.pikg`（同一 PikgReader 自校验）并推到仓库；`publish_status=repo_stored_candidate` 表示尚未权威发布 App DID |

* **权限**：所有 handler 要求已认证主体；目标用户默认取 `principal.username`（为自己安装），给他人安装需 admin；`SYSTEM_INTERNAL` 策略（可 auto-confirm）仅限 admin。confirm/retry/cancel 只能操作本人任务（admin 例外）。
* **安全约束**：同一个 `AppInstanceId` 的 install/upgrade/start/stop/restart/uninstall 共用 system-config CAS mutation ownership；卸载删除范围只来自 typed manifest；已 `Deleted` 的 App 不可 start；runtime 仍被 AgentSpec.binding 引用时不可卸载。

### 4.5 UI Session 管理 (登入/登出/当前用户)

涉及两层：

**(a) 登录鉴权层**（`sys_auth_backend`）—— 处理登入/登出与 token：

* `auth.login`：取 `username` + `password` + 结构化 `target`；SSO 模式以 Gateway/redirect 为真相源解析 `AuthTarget::App(AppInstanceId)` 或 `AuthTarget::System(SystemServiceId)`，前端 `appid` 只做一致性检查。根域 `_` 固定解析为 `System(control-panel)`，不创建 `control-panel@system`。若带 `redirect_url`，pending nonce 同时绑定 target、canonical origin 和完整 redirect。
* `auth.refresh` / `auth.verify` / `auth.logout` / `auth.issue_sso_token`：刷新 / 校验 / 注销 / 签发 SSO token。
* HTTP 侧：`/sso_callback` 只有在 pending/callback/实际 request 的 origin、redirect 和 Gateway route target 全部一致，且 token pair 的 target/use 合法后才写两枚 host-only cookie；失败会尽力吊销 pending refresh。`/sso_refresh` 在调用 Verify Hub 前后都校验当前 origin/route 和 target，任何失败都清 cookie；`/sso_logout` 尽力吊销后始终清 cookie。两种 cookie 都使用 `Path=/; SameSite=Lax`，HTTPS 请求还会使用 `Secure`，且禁止通过 `Domain` 扩散。
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
* **Dashboard / System**：`dashboard` / `system.overview` / `system.buckyos_info.get` / `system.status` / `system.metrics` / `network.overview`。其中 `system.buckyos_info.get` 只读返回 SystemConfig `system/buckyos_info` 的 typed `BuckyOSInfo`，供 Settings 展示版本、构建、发布通道、安装及更新时间。
* **System Logs**：`system.logs.list` / `query` / `tail` / `download`（凭 token 经 `GET /kapi/control-panel/logs/download/{token}` 下载）。
* **AICC Settings**：`ai.overview` / `ai.provider.*` / `ai.model.*` / `ai.policy.*` / `ai.diagnostics.list` / `ai.reload` 等，管理 AI 接入与路由策略。

---

## 5. App 安装协议 (App Installer Protocol)

真相源：[doc/App 安装协议.md](../App%20安装协议.md)（Draft v0.5，§14.0 为已冻结实现基线）。实现分布：
[app_install_engine.rs](../../src/frame/control_panel/src/app_install_engine.rs)（可恢复状态机）、
[app_install_resolver.rs](../../src/frame/control_panel/src/app_install_resolver.rs)（`(App DID, "app")` 解析适配）、
[app_install_planner.rs](../../src/frame/control_panel/src/app_install_planner.rs)（不可变 InstallPlan 与动态 Inspection）、
[pikg.rs](../../src/frame/control_panel/src/pikg.rs)（`.pikg` 读写与校验）、
[app_install_driver.rs](../../src/frame/control_panel/src/app_install_driver.rs) +
[app_install_deployer.rs](../../src/frame/control_panel/src/app_install_deployer.rs)（各 Stage 生产实现）、
[app_install_runner.rs](../../src/frame/control_panel/src/app_install_runner.rs)（dispatch 与恢复）。

### 5.1 Stage 流水线与持久化

```
identifier / staging handle
   └─> Resolve   resolve_did(App DID, "app")，candidate 绑定（id/expected_owner 硬约束）
   └─> Inspect   按目标 Node（devices/{node}/info，非编译期 cfg）选 package，
                 产出 immutable InstallPlan + dynamic InstallPlanStatus + fingerprint
   └─> [WaitingForApproval]  确认权限/参数；SYSTEM_INTERNAL 可显式 auto-confirm
   └─> Acquire   只下载 plan.missing（TaskManager 子任务，kevent 加速等待）；
                 offline 模式不建下载任务
   └─> Verify    重读全部 Package Meta ObjId、重哈希 pikg 内容、selector/mount 复核，
                 输出逐项 VerificationReport；Trust+Content+Config 全 ready 才继续
   └─> Prepare   构造 spec、materialize pikg 内容进 NamedStore、端口冲突检查、
                 app_index CAS 分配，先写 install_record(state=prepared)
   └─> Scheduler claim/commit    写 users/{uid}/apps/{app_id}/spec —— desired-state commit point
   └─> Activate  等 exact DeploymentIdentity + 新鲜 epoch/session/health 实例证据，
                 或 static web 物化 + gateway config generation ack；
                 成功后才 install_record(state=installed) -> installed proof -> Task 完成
```

* 不可变业务请求保存在 `Task.input`；每个 Stage 完成先把 `AppInstallProgressEnvelope {transaction, display}` 完整写回 `Task.progress`，展示百分比更新不得覆盖 transaction；成功终态在 `Task.result` 保存 typed output。
* 解析状态硬规则：`Revoked/Tombstoned` 不可重试且禁止任何 fallback；`Missing`（权威回答"从未发布"）与 `unknown`（没有回答）错误码不同；unknown 且无可接受 cache 进入 `WAITING_FOR_TRUST_RESOLUTION`（任务 Paused，可 retry）。
* 错误全部结构化（`InstallError{stage, code, retryable, action}`），进入事务快照、terminal result 与 install_record。

### 5.2 Runner 与恢复

安装任务只能由 Control Panel 已鉴权的 `apps.*` 业务接口创建。TaskManager 成功持久 delegated task 后，RPC 路径立即启动本地 runner；Control Panel 启动扫描与 30s sweep 分页恢复本 runner 的 Accepted/Running task。WaitingForApproval/Paused/Terminal 不会被 sweep 自动执行，进程内 task-id guard 合并重复扫描。正确性不依赖 KMSG、runner inbox、`task_ready` 或 KEvent；revision + runner epoch fencing 拒绝过期执行体写回。

### 5.3 升级 / 卸载

* **升级**：同一 Stage 流水线；`AppUpdateTaskRequest` 冻结已批准的 `Upgrade` plan 与 fingerprint，并用 plan 的 canonical task ID 创建 TaskManager task。以权威 Resolve 的 App Document Object ID / `document_version` 判定新版本，继承当前组件、权限、mount/settings/env/resource pool、实例数和停止期望。当前切换策略是 in-place/recreate，写新 spec 后存在停机窗口，不宣称 blue-green/rolling。Activate 失败先恢复冻结 previous spec，再等待 previous `DeploymentIdentity` 重新满足 readiness；结果区分 target failed + previous restored、rollback failed 与 partial target。
* **卸载/生命周期**：`app.start/v1` 与 `app.uninstall/v1` 都是 delegated durable task，共用启动扫描/sweep 与 mutation ownership。卸载先 stop 并等待 exact target evidence 消失，再删除 spec/record；`delete` 只删除 typed manifest 中的私有 data/cache，`retain` 不删除。进入 metadata deletion 边界后 cancel 被禁止。
* **批量升级**：`apps.upgrade` 创建 `app.update_batch/v1` root；每个 `UpdateAvailable` item 同时冻结 Upgrade plan 与 fingerprint，child 使用该 plan 的 canonical task ID，并通过 parent/root 和稳定派生 idempotency key 关联。单项失败不影响其它 child，root 结果保存每项 terminal outcome，可在 Control Panel 重启后恢复。

### 5.4 记录与凭证

* in-flight 真相源：TaskManager `input/progress/result`；长期记录：`users/{uid}/apps/{app_id}/install` 与 `users/{uid}/agents/{agent_id}/install`（含 App/Agent DID、AppInstance/binding、source、target/previous deployment、参数、解析快照、exact package meta ids、pikg digest、状态、task/proof id）。
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
