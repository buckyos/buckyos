# BuckyOS Control Panel Service 技术需求文档

> 本文件描述 `src/frame/control_panel` 中 **Control Panel Service** 的技术需求与设计。
> 内容发布与分享子系统（Share Content Mgr / Bucky-CMS）的旧设计见 [`ShareContentMgr.md`](./ShareContentMgr.md)；第一阶段分享已改在 nfs-server 实现（`product/bucky_file/nfs_server.md` §12），control-panel 不再承载。

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

* 身份与账号信息，核心结构 `UserSettings`（`user_type` / `state` / 密码哈希 / `is_local` / 是否允许改密 / 是否允许改通讯录 / 系统用户组 `groups` 等），来自 `buckyos-api`。
* 关联结构：`UserPrivateProfile`（用户私有展示资料，保存在独立 profile 路径）、`UserContactSettings`（DID / tags / 消息平台绑定，保存在 profile 的私有扩展中）、`UserTunnelBinding`（消息平台账号绑定）、`UserGroup` 与 `ContactProjectionConfig`（`system/user_groups/` 下的系统用户组与通讯录投影规则）。
* 状态机 `UserState`：`Active`（正常，可签发/使用 session）/ `Pending`（已邀请未激活）/ `Deleted`（软删除，记录保留但禁止登录）。
* 角色 `UserType`：`Root`（Zone owner 账号记录，不可登录、不可删除/不可变更）/ `Admin`（管理员）/ `User`（普通）/ `Limited`（零权限底座，经系统用户组获得 App 与资源授权）。各类型的四个维度、系统用户组与通讯录投影的权威定义见 [UserType.md](../../product/users_and_agents/UserType.md)，代码中的集中判定在 `UserType` / `UserSettings` 方法上（`is_privileged`、`can_create_agent`、`can_change_password`、`can_edit_contacts` 等）。
* `system_config` 路径：

  | 路径 | 内容 |
  |---|---|
  | `users/{user_id}/settings` | `UserSettings` JSON（账号主数据） |
  | `users/{user_id}/profile` | `UserPrivateProfile` JSON（普通用户可自行读写） |
  | `users/{user_id}/doc` | DID Document / OwnerConfig（身份与公开资料） |
  | `users/{user_id}/key` | ED25519 私钥（PEM，仅创建时写一次） |
  | `services/control_panel/user_invites/{invite_id}` | 邀请记录 `UserInviteRecord` |

### 2.2 Device（设备）

* 当前为**只读实体**：Control Panel 不提供设备的增删改 RPC；设备身份在 OS 引导/置备阶段确定（Desktop 版的 Settings 里“添加设备”按钮只提示暂不支持）。
* Zone 内全部设备由 `zone.devices.list` 列出（[cluster_mgr.rs](../../src/frame/control_panel/src/cluster_mgr.rs)）：取 `devices/` 下登记的设备与 ZoneDocument `oods` 的并集（OOD 按文档顺序在前），每台读 `devices/{name}/info`（node_daemon 约每 30 秒写一次的 `DeviceInfo`，含 OS、CPU、内存、存储、GPU、IP 与 `network_observation`）。`update_time` 超过 120 秒未刷新即判为离线；从未上报的 OOD 也会列出并带 `info_error`。
* 本节点的本地文件（`node_identity.json`、`{device_did}/did.json` 等）仍由 `zone_mgr` 的 `zone.overview` 聚合，用于 SN/DNS 自检。

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
| `user.list` | 已登录 | 列出用户（可选 `include_deleted`）；每项附带公开 profile 投影中的 `did` 与 `avatar`（遵守 `to_public_profile()` 可见性），以及 `groups`、`allow_password_change` / `allow_contact_edit`（覆盖值，可为 null）和生效值 `can_change_password` / `can_edit_contacts` |
| `user.get` | 本人/Admin | 取单用户详情（含 `groups`，`allow_password_change` / `allow_contact_edit` 为生效值）；profile 私有扩展中的 `contact` / `did_document` 仅本人或 Admin 可见 |
| `user.create` | **Admin** | 事务创建 settings+doc+key+profile；`user_type` 只接受 `user` / `limited`（Admin 经 `user.change_type` 提升）；可选 `groups`（已定义的系统用户组 id）、`allow_password_change`、`allow_contact_edit`；随后刷新 RBAC 并通知 msg-center |
| `user.update` | 本人/Admin | 改 profile 中的显示名等 |
| `user.update_contact` | 本人/Admin | 改 profile 私有扩展中的 DID/tags/消息平台绑定；传 `groups` 会被拒绝（系统用户组由 `user_group.set_members` 管理） |
| `user.profile.get` / `user.profile.set` | 本人/Admin | `users/{uid}/profile` 中的私有 profile（DID profile 只读，响应中两源合并：本地显式字段优先，DID 文档 `profile`/`meta` 只补本地为空的字段）。`set` 的文本字段三态：省略=不改、`null` 或空串=清除（回落到 DID 文档值）、字符串=设置；`avatar` 只接受 ≤192px、≤512KB 的 base64 PNG/JPEG/WebP data URL。保存后通知 msg-center 重新同步 Zone 联系人 |
| `user.set_msg_tunnel` / `user.remove_msg_tunnel` | 本人/Admin | 增删消息平台账号绑定 |
| `user.invite.create` | **Admin** | 生成邀请（可预建 Pending 用户或绑定已有 DID），返回 `invite_url`；`user_type` 为 `admin` / `user` / `limited`，`groups` 为已定义的系统用户组 id，接受邀请时写入账号 |
| `user.invite.get` | **公开** | 凭 `invite_id` 读邀请详情（含过期、zone_did/host） |
| `user.invite.accept` | **公开** | 凭 `invite_id` + `owner_config` 接受邀请并激活账号 |
| `user.delete` | **Admin** | 软删除（置 Deleted），不可自删 |
| `user.change_password` | 本人/Admin | 本人改密受 `allow_password_change` 限制 |
| `user.change_state` | **Admin** | Active/Pending/Deleted 迁移 |
| `user.change_type` | **Admin** | 只在 `admin` / `user` / `limited` 之间转换，不能改 Root；随后刷新 RBAC 并通知 msg-center（通讯录投影依赖类型），旧会话不自动获得新权限 |
| `user.set_account_options` | **Admin** | `{ user_id, allow_password_change?, allow_contact_edit? }`，`null` 恢复类型默认；返回生效值 |
| `user_group.list` | **Admin** | `{ groups: [UserGroup + members], projections: { rules, is_default, updated_by, updated_at } }` |
| `user_group.create` / `user_group.update` | **Admin** | `{ group_id, name, description? }`；`group_id` 为 `[a-z][a-z0-9_-]{0,31}`，写 `system/user_groups/<id>` |
| `user_group.delete` | **Admin** | 删除组定义，同时从成员的 `UserSettings.groups` 和投影规则中移除；刷新 RBAC 并通知 msg-center |
| `user_group.set_members` | **Admin** | `{ group_id, add?, remove? }`；Root 不能加入；刷新 RBAC 并通知 msg-center |
| `user_group.set_projections` / `user_group.reset_projections` | **Admin** | 保存单向“来源组 → 目标组”规则（`type:admin|user|limited` 或 `group:<id>`）到 `system/user_groups/_projections`；reset 删除该 key，恢复默认（Admin、User 互见） |

### 4.2 设备管理 (Device)

* 无写接口。设备信息通过 `zone.devices.list`（全部设备）与 `zone.overview`（本节点文件与 SN 自检）以只读形式暴露（见 §4.6）。设备的注册/绑定在 OS 引导与置备层完成，不在本服务范围。

### 4.3 智能体管理 (Agent — 身份维度)

由 `agent_mgr` 管理。Zone Owner、Admin 与普通 User 都只能为自己创建 Agent，Limited 被拒绝（`limited_user`）。创建 Agent = 基于 Agent 模板构造一个独立 App（AppDID = AgentDID，AppId = AgentId，实例 `<agent_id>@<owner>`）并以创建者身份安装，再写入绑定到它的 AgentSpec。记录在 `users/{owner}/agents/{agent_id}/{spec,key,install_record,settings,profile,info}`；`install_record` 是创建状态的唯一真相，后台驱动按 `runtime → bind → tunnel → start` 推进并可恢复。产品行为见 `product/users_and_agents/BuckyOS Agent的初始化流程PRD.md`。错误以 `ReasonError("<code>: …")` 返回。

`agent.create` 的可选字段 `permission_mode` 为 `read_write`（默认）或 `read_only`。Add Agent 向导默认选择读写，用户可选择只读；该选择保存到 `AgentSpec.permission_mode`，并在 `agent.list` / `agent.get` 的 Agent 条目顶层返回。构造 App 和安装批准中的 `user/home` 权限分别使用 `actions: ["read", "write"]` 或 `["read"]`；创建重试和模板自动更新沿用该 Agent 的选择。授权范围以创建者为准，不接受客户端指定其他 Owner。

Agent 按所选模式使用 Owner 的权限；只读模式限制用户数据写入，Agent 自己的 `agent_root` 始终可读写，以保存 Session、记忆、工作区、技能和运行状态。该字段是创建授权，不放在可由 Agent 修改的 `settings` 或 `agent.toml` 中；`agent.update` 不提供权限模式切换。

Control Panel 在校验 App session token 后读取绑定 Agent 的权限模式；只读 Agent 仅允许明确列出的读取 RPC，用户/Agent 配置、App 生命周期、容器操作等写请求返回 `NoPermission`。`/api/desktop` 的状态/会话写操作与壁纸上传使用同一限制。用户本人登录 Agent 页面取得的 User session token 继续按用户权限处理。

| 方法 | 说明 |
|---|---|
| `agent.check_name` | 名称检查（与 `user.create` 共用）：DNS 标签、保留名、用户名、任何用户名下的 AgentId（含进行中）、已分配的 App/实例主机名；不可用时给出建议名 |
| `agent.list_templates` | 系统内置模板（`system/install_settings.agent_templates`）与调用者用 CLI 安装的模板（`users/{owner}/agent_templates/*`）；`template_id` 为 `bundled:<app_id>` / `installed:<app_id>` |
| `agent.create` | 以 `idempotency_key` 幂等；`permission_mode` 默认 `read_write`，可选 `read_only`；写入保留记录后立即返回状态，后台构造并安装 App、绑定 AgentSpec、校验并绑定 Telegram Bot（可选）、等待 OpenDAN 写入 `info` |
| `agent.create.status` / `agent.create.retry` / `agent.create.cancel` | 查询创建状态；从失败步骤继续（`skip_tunnel` 放弃通道完成创建）；写入 spec 前取消 |
| `agent.list` / `agent.get` | 本人名下（Admin 为全部）的 Agent，包括进行中与失败的创建；settings 中不含 Bot Token |
| `agent.update` | 本期只允许修改 `allow_group` |
| `agent.profile.get` / `agent.profile.set` | `{display_name, avatar, bio}`，显示名称回退为 Agent 用户名 |
| `agent.delete` | 先删身份记录，再刷新 RBAC、通知 msg-center、卸载构造 App，最后释放名字 |
| `agent.set_msg_tunnel` / `agent.remove_msg_tunnel` | 读写 `settings.msg_tunnels`（本期最多一个 Telegram Bot） |

`apps.submit` 收到 Agent 类型的 PIKG 时不安装运行时实例，而是注册为调用者的 Agent 模板（返回 `template_registered` / `template_updated` / `satisfied`，`task_id` 为空）；模板更新会按各 Agent 的 `template_auto_update` 重建其构造 App。

### 4.4 应用管理 (App — 服务维度)

由 `app_servcie_mgr` + `app_installer` 管理。`apps.list` 从系统内置服务、用户 AppSpec 与 availability policy 计算目标用户的最终授权集合；Agent 读取走 `agent.list` / `agent.get`；各 Agent 的构造 App 作为 `runtime_type` 为 agent 的实例出现在 `apps.list`。产品身份是 canonical AppDID，可逆 key 是 AppId；Owner 范围内的安装和运行目标是 `AppInstanceId = {app_id}@{owner_user_id}`。普通 App 只有这一种 Owner 范围安装模型。

| 方法 | 返回 | 说明 |
|---|---|---|
| `apps.list` | `{user_id, total, apps[]}` | 当前用户的有效 App；每项包含 `app_id/app_did/app_instance_id/runtime_type/owner_user_id/availability_match/web_hosts`，以及展示字段 `name`（AppDoc 包名）、`show_name`、`presentation`（`title/summary/description` 的语言→文本映射，无则 null）、`app_icon_url`（`presentation.links.app_icon_url`；`presentation.icons` 的 ObjId 尚无浏览器可加载路由，不返回） |
| `apps.details` / `app.details` | typed details | 接受统一 selector（`selector/app_instance_id/app_did/identifier`），按可见 Owner 范围唯一选择；0 个返回 NotFound，多个返回 `AMBIGUOUS_APP_TARGET` 与脱敏候选 |
| `apps.status` / `app.status` | `AppInstallationStatusSnapshot` | 聚合 install record、desired spec、active task、scheduled/runtime instance、目标/上次成功/回滚 deployment、typed deployment error 与 Static Web gateway generation evidence |
| `apps.availability.get/set/check` | policy / decision | 个人 App 的用户类型组（`admins`/`users`/`limited`）、系统用户组（`group:<id>`，必须已定义）、精确用户与匿名（`guest` 伪组）规则；`set` 仅允许 App Owner 的 Control Panel 用户 session，并以 revision/CAS 原子更新策略与审计；scheduler 单独把 policy 投影为 Gateway access mode，不回写 AppSpec |
| `apps.staging.finalize/status/release` | `PikgStagingMetadata` | `finalize` 接受 FileObject 的 `source_obj_id` 或本机 incoming 文件的 `local_file_id`（二选一），以及必填 `pikg_digest/size` 和 `purpose=inspect|install`；完整落盘并校验后返回 handle、digest、size、TTL；handle 绑定 principal、App、Zone 与租约，不包含路径或 digest |
| `apps.inspect` | `InstallInspection` | 对 Catalog 或 staged PIKG 做无安装副作用的首次安装/升级预检；`action=upgrade` 时生成升级 inspection |
| `apps.plan.recompute` | `InstallInspection` | 接受旧 plan、同一 source 以及新的 target/InstallParams，权威重算 plan 与 fingerprint；source/scope 变化返回 `PLAN_STALE` |
| `apps.submit` / `apps.install` | `{action, task_id?, app_instance_id, plan_fingerprint?}` | 权威动作矩阵。首次安装必须提交 `FreshInstall` plan，升级必须提交 `Upgrade` plan；相同发布返回同步 `satisfied`。提交时按 plan 内 canonical task ID 重新检查 source/scope/fingerprint，并以同一 ID 创建 TaskManager task。所有 mutation 必须提交 principal 稳定生成的 `idempotency_key` 与已展示 fingerprint |
| `apps.install.status` | `AppInstallStatusSnapshot` | 按 `task_id` 返回 typed stage、inspection、approval、verification、error、actions 与 terminal result，不暴露 Task 内部 JSON |
| `apps.install.confirm` | `{task_id}` | 只接受 `{task_id, plan_fingerprint}`；不得在 confirm 同时改 target/params |
| `apps.install.retry` | `{task_id, retry_of}` | 接受旧 `task_id` 与新的 `idempotency_key`；Failed task 保持 Terminal，新建带 `retry_of`/parent 关系的 task |
| `apps.install.cancel` | `{task_id, task_phase, task_outcome, mutation_released, cleanup_pending}` | 接受 `{task_id, force?: boolean}`，先持久化 TaskMgr cancel intent；普通取消清理失败报告 Failed，force 允许 staging 清理延后。成功返回 Terminal/Canceled 与已释放 mutation；部署已提交时拒绝取消 |
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
* `auth.refresh` / `auth.verify` / `auth.logout`：刷新 / 校验 / 注销。
* `auth.prepare_app_launch`：桌面免密打开 App。参数只有 `app_instance_id` 和 `redirect_url`（最终 App URL），不接受 `username`、`target` 等自报身份的字段。要求普通（非 sudo）Control Panel 用户会话，不在公开白名单内；通用认证对本方法返回 `InvalidToken` 时（缺 token、验签或有效期失败、用户 inactive、读 user settings 失败）映射为 `source_session_inactive` 结果。`redirect_url` 按与 SSO 相同的 Gateway route 规则解析，必须是与 `app_instance_id` 完全一致的 App target，否则返回 `invalid_target`（读 gateway info 失败等仍是 RPC error）。随后以本服务的服务会话调用 verify-hub `delegate_user_session`（verify-hub 只接受 Control Panel 服务会话调用），并把原始桌面 token 作为来源证明；`source_inactive` / `access_denied` 映射为 `source_session_inactive` / `access_denied`。返回的 token pair 必须是同一用户、目标实例的普通 User session/refresh 且未过期，才和密码登录一样存进 60 秒一次性 pending，响应 `{ "status": "ready", "callback_url": "<app origin>/sso_callback?nonce=…&redirect_url=…" }`，不含 token。所有 `auth.*` 响应带 `Cache-Control: no-store`。结果状态、桌面处理和已知取舍见 [SSO.md](../sdk/SSO.md)“从桌面打开 App”。
* HTTP 侧：`/sso_callback` 只有在 pending/callback/实际 request 的 origin、redirect 和 Gateway route target 全部一致，且 token pair 的 target/use 合法后才写两枚 host-only cookie（响应 `Cache-Control: no-store`）；失败会尽力吊销 pending refresh。nonce、token 和含 nonce 的 URL 不写入日志。`/sso_refresh` 在调用 Verify Hub 前后都校验当前 origin/route 和 target，任何失败都清 cookie；`/sso_logout` 尽力吊销后始终清 cookie。两种 cookie 都使用 `Path=/; SameSite=Lax`，HTTPS 请求还会使用 `Secure`，且禁止通过 `Domain` 扩散。
* **取当前用户**：受保护方法在 `authenticate_session_token_for_method()` 中校验 token（`verify_trusted_session_token`），token 带 `external: true` 标记（本 Zone 无账号记录的 DID）时直接拒绝，v1 不向 External 开放任何 Control Panel 方法；其余从 `sub` 解出 username，加载 `users/{username}/settings` 校验为 Active 且非 Root，构造 `RpcAuthPrincipal { username, user_type, owner_did }` 传给各 handler。

**(b) 桌面 UI Session 状态层**（`ui_session_mgr`）—— 持久化每个用户的桌面会话（外观/窗口布局/图标布局/小组件布局）：

* 存储路径：`users/{user_id}/desktop/{session_id}/{state_key}`，会话元数据存 `.../_meta`。
* HTTP 入口：`POST /api/desktop`，需 session token（401 if 失败），按 `action` 分发。
* 方法：`session.list` / `session.create`（生成 UUID v4）/ `session.delete` / `session.rename`；`state.get` / `state.set` / `state.delete`，后三者支持 `json_path`（点分路径）做部分读写删。任何 state 写都会刷新会话 `updated_at`；写入一个还没有 `_meta` 的会话时补建 `_meta`（`name` = session_id）。`session_id`、`state_key` 只接受 `[A-Za-z0-9_-]{1,64}`，`state_key` 不能以 `_` 开头，否则 400。`state.get` 读从未写过的 state 返回 `{value: null, version: null, exists: false}`（不是错误），存在时 `exists: true`。
* **账号级外观**：Desktop 所有设备共用约定的会话 `default`，外观设置存在 `state_key = appearance`，每项设置单独按 `json_path` 写，不同设备改不同设置不会互相覆盖：`{theme: "light"|"dark", locale, iconSize: "small"|"medium"|"large", window: {titleBarOpacity, backgroundOpacity}, wallpaper: {image: {asset, width, height, name} | null, fit: "fill"|"fit"|"stretch"|"tile"|"center"|"panorama", tileScale, focusX, focusY}}`。前端先用本机缓存立即绘制，启动和回到前台时读账号值（账号优先；本机改了但账号还没确认的设置以本机为准，跨刷新保留）；账号缺的设置用本机值补写，但不会把浏览器里另一个账号的壁纸带给新账号。状态栏消息预览、安全区、任务栏和图标位置仍是设备级设置（桌面上有哪些项目跟着账号走，见下方“桌面文件夹”）。
* **壁纸图片**：图片字节不进 SystemConfig（单值上限），存在用户 home：`data/home/{user}/.local/share/control-panel/wallpapers/{sha256 前 32 位}.{jpg|png|gif|webp|avif}`。`POST /api/desktop/wallpaper`（body 为图片原始字节，≤16 MiB，按文件头识别类型，其它类型 415、超限 413）返回 `{ok, data: {asset, bytes}}`，同时只保留最新 3 张、删除更早的；`GET /api/desktop/wallpaper/{asset}` 返回图片，名字即内容哈希，响应 `Cache-Control: private, max-age=31536000, immutable`。两者都需登录。CSS/`<img>` 加载带不了请求头，所以读图（`GET`）在没有 `X-Auth`/`Authorization`/查询参数 token 时优先认 Gateway SSO 会话 cookie `buckyos_session_token`（根域 SSO 登录后由 `/sso_refresh` 持续续期），其次才是旧的 `control-panel_token` 等 cookie；Desktop 因此直接把这个 URL 用作 CSS 背景，加载失败时再用带 `X-Auth` 的 fetch 取回。上传（`POST`）不认 SSO cookie，必须带请求头：同站其它子域应用发出的请求也会捎带这个 cookie。
* **桌面文件夹**（`desktop_folder.rs`）：像 Windows 一样，桌面显示的是用户 home 下的一个文件夹 `data/home/{user}/Desktop`（Files 里是 `/home/{user}/Desktop`），所以所有设备、所有会话看到的是同一批项目，会话（设备 × 形态）只保存项目的位置。以 `.` 开头的名字隐藏。两类小 JSON 文件是桌面对象：`*.lnk` 快捷方式 `{"app": "<内置 id | AppId | 实例 id>"}`，`*.widget` 小组件 `{"widget": "clock"}` / `{"widget": "notepad", "text": "..."}`；其它文件和文件夹按文档/文件夹显示（文档用内容处理器打开，文件夹在 Files 中打开）。动作：
  * `items.list {seed?: [{name, text}]}` → `{path, created, items: [{name, kind: "file"|"dir", size, mtime, text?}]}`。文件夹不存在时创建并写入 `seed`（`created: true`，seed 只接受 `.lnk`/`.widget`，每个 ≤64 KiB、最多 64 个）；用户自己清空的文件夹保持为空，删掉整个文件夹则下次按 seed 重建。`.lnk`/`.widget` 文件（≤64 KiB）随列表返回内容 `text`；符号链接不跟随、不读内容。
  * `items.write {name, text, create?}` 写一个 `.lnk`/`.widget` 文件（临时文件 + rename）；`create: true` 时已存在则 400 `exists: {name}`。
  * `items.delete {name}` 删除一个 `.lnk`/`.widget` 文件（不存在视为成功）。文档和文件夹只在 Files 里改。
  * 名字必须是单个可见路径段（≤255 字节，无 `/` `\` 控制字符，不以 `.` 开头），否则 400。
  * 新账号的默认内容 = Desktop 前端提供的 `seed`（内置应用快捷方式、时钟和上手说明便笺，按当前语言命名）+ `system/install_settings.pre_install_apps.{app_id}.desktop_shortcut: {name}` 声明的预装应用快捷方式（如 Text Editor，写成 `{name}.lnk`，与 seed 重名时编号）。Desktop 源码不出现任何第三方 App 的身份。安装应用不会自动在桌面放快捷方式，应用在“全部应用”里，用户可从那里“添加到桌面”。

### 4.6 只读诊断模块 (Read-only Diagnostics)

以下模块只读，服务于运维面板，列出以明确服务边界（不在“写功能”范畴）：

* **Zone**：`zone.overview` / `zone.config` — 聚合 `start_config.json` / `node_identity.json` / `did.json`，输出 zone/device/SN/DNS 自检与文件清单。
* **设备与证书**：`zone.devices.list` 返回 `{zone_did, hostname, owner_did, sn, local_device, online_window_secs, devices: [...]}`，每台设备含 `role`（`ood`/`gateway`/设备类型）、`online`、`last_seen`、`os_version`、`cpu`/`memory`/`storage`/`gpu`、`ips`、`endpoints`、`ipv6_state`（见 §2.2）。`zone.certificates.list` 用 name-client `IdentityRoots` 读取本节点身份库里 Zone 主机名及其通配符的 server 证书元数据（cyfs-gateway ACME 管理器写入），返回 `{hostname, node, certificates: [{host, source, issuer, not_before, not_after, days_remaining, dns_names, fingerprint_sha256, state, cert_pem}]}`；`state` 为 `valid`/`expired`/`not_yet_valid`/`unusable`/`unknown`，`cert_pem` 只是公开证书，不含私钥。两者只读，需登录。
* **Gateway**：`gateway.overview` / `gateway.config` / `gateway.file.get` — 解析 `cyfs_gateway.json` / `boot_gateway.yaml` / `node_gateway.json` 等（白名单文件，单文件 ≤2MB），输出 stacks / routes / tlsDomains。
* **Container**：`container.overview` / `container.action`（start/stop/restart，带缓存与刷新锁）。
* **Zone 展示**：`zone.presentation.get`（**公开**）返回 `{zone_did, hostname, settings, document}`，`settings` 是管理员显式配置（SystemConfig `system/zone_presentation`，类型 `ZonePresentationConfig`），`document` 是 ZoneDocument 扩展字段中约定的 `display_name/description/icon` 候选；两者分开返回，由前端按「设置 → 文档 → DID 可读标识 → 默认」取值。`zone.presentation.set` 只接受 Admin/Root 的 Control Panel 用户 sudo session，以调用者 token 经 RBAC（`su_admin`）与 revision CAS 写入；字段三态同 `user.profile.set`，`icon` 只接受 ≤512px、≤512KB 的 base64 PNG/JPEG/WebP data URL。它只改显示元数据，不改 Zone DID、域名或 owner，也不签发/更新 ZoneDocument。键不存在即「未配置」。
* **Dashboard / System**：`dashboard` / `system.overview` / `system.buckyos_info.get` / `system.dev_mode.get` / `system.dev_mode.set` / `system.status` / `system.metrics` / `network.overview`。`system.buckyos_info.get` 只读返回 SystemConfig `system/buckyos_info`；`system.dev_mode.get` 返回 `system/buckyos_dev_config`，`system.dev_mode.set` 只接受面向 Control Panel 的用户 sudo session，并继续以调用者 token 通过 SystemConfig RBAC 和 revision CAS 写入。SystemConfig 服务对这个 key 的 create/set/set-by-path/append/delete/transaction 运行期写入还有统一 sudo 门禁，首次 boot bootstrap 初始化除外。
* **System Logs**：`system.logs.list` / `query` / `tail` / `download`（凭 token 经 `GET /kapi/control-panel/logs/download/{token}` 下载）。
* **AICC Settings**：`ai.overview` / `ai.provider.*` / `ai.model.*` / `ai.policy.*` / `ai.diagnostics.list` / `ai.reload` 等，管理 AI 接入与路由策略。

### 4.7 URL 下载 (`download.create`)

用户在 Task Center 手工新建的下载由 control-panel 自己执行（与 `diagnostic.collect` 同一模式），实现在 `download_mgr.rs`：

* 请求：`download.create {url, filename?, idempotency_key}`，需登录的 Zone 用户（用户名要能作为 `data/home` 下的目录名）。只接受 `http(s)` URL；`filename` 净化为单个路径段，缺省时依次取响应的 `Content-Disposition`、最终 URL 的最后一段、`download`。返回 `{task_id}`。
* 任务：以调用者为 creator 调 `create_delegated_task`，schema `download/v1` 的第 2 版（`presentation_schema.system_notify = progress`，见 TaskMgr 2.0 §9.1），`storage_domain = User`，input 是 `DownloadTaskData`（只有 `request`，`options.local_path = /home/<user>/Downloads`）。task-manager 还没种下这一版时，control-panel 作为发布者自己 `register_task_schema` 后重试。
* 执行：文件先写成 `data/home/<user>/Downloads/.download-<task_id>.part`，完成后改名为最终文件名（重名追加 ` (1)`、` (2)`…）。进度每 0.5 秒以 `TaskDataProgress {bytes: {completed, total}}` 写入，message 为 `已下载 / 总大小`。声明 `cancel = Safe`：收到取消请求后删除临时文件并 `ack_control(applied)`。超过 120 秒收不到数据即失败。结果是 `DownloadTaskResult {state: completed, local_path: /home/<user>/Downloads/<name>, downloaded_bytes, total_bytes, completed_at}`；失败码 `download_failed`。
* 重启：不续传。control-panel 启动时把上次遗留的非终态下载以 `download_interrupted` 失败，并删除它们的临时文件。
* 已知风险：服务端会去访问用户给的任意 URL（包括 Zone 内网地址），目前不做限制。

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

安装任务只能由 Control Panel 已鉴权的 `apps.*` 业务接口创建。TaskManager 成功持久 delegated task 后，RPC 路径立即启动本地 runner；Control Panel 启动扫描与 60s sweep 分页恢复本 runner 的 Accepted/Running task，并消费 WaitingForApproval/Paused 的 pending Cancel。没有 Cancel 的等待任务仍只由 confirm/retry 唤醒，进程内 task-id guard 合并重复扫描。confirm/retry 与取消按 task_id 串行化；已持久化 Cancel 时禁止恢复安装。正确性不依赖 KMSG、runner inbox、`task_ready` 或 KEvent；revision + runner epoch fencing 拒绝过期执行体写回。

取消先确认 scheduler 尚未提交 desired state，再确认任务终态并条件释放 mutation；等待且尚未进入 Prepare 的任务无需访问 scheduler。mutation key 的释放独立于 staging，早期任务从 submitted_plan 获取 AppInstanceId，并通过 task_id 所有权检查与 KV 版本 CAS 防止旧任务删除新锁。活跃任务即使超过 expires_at 也不能被接管；任务已终态或 TaskMgr 明确返回 task_not_found 时可以回收，网络与权限错误保留锁。task_id 尚未绑定的预约有 300 秒创建窗口，绑定时复核预约所有者。

启动和 sweep 同时回收终态任务的 mutation key 和 staging 租约。force 清理失败时现有持久 staging 租约就是恢复依据，无需新增队列；Interrupt 取消不承诺临时文件已经删除。已取消任务的重复取消幂等，成功响应包含 task_id、task_phase、task_outcome、mutation_released、cleanup_pending。

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
| `user.invite.get` | 凭邀请链接读邀请详情 |
| `user.invite.accept` | 凭邀请链接接受邀请、激活账号 |
| `zone.presentation.get` | 登录页等读取 Zone 名称、描述、图标（只含展示字段，不开放 `zone.overview`） |

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
| Zone 展示设置 | `system/zone_presentation` | 调用者 sudo token + revision CAS（不存在时 create） |
| UI Session | `users/{uid}/desktop/{session_id}/{state_key}` · `.../_meta` | 单次 `set`，写后刷新（或补建）`_meta.updated_at` |
| 桌面壁纸图片 | 文件 `data/home/{uid}/.local/share/control-panel/wallpapers/{hash}.{ext}` | 临时文件 + rename，保留最新 3 张 |
| 桌面文件夹 | 目录 `data/home/{uid}/Desktop`（`*.lnk` / `*.widget` 由本服务写） | 首次列出时建目录并写 seed；单文件临时文件 + rename |

---

## 8. 关键技术要点

1. **写收口**：四类核心实体的所有写都经本服务，统一处理事务与权限，避免客户端各自拼装。
2. **原子事务**：跨路径写用 `exec_tx`；唯一已知的尾随非原子点是 RBAC `append`，需知情并择机收敛。
3. **最小越权**：受保护路径用调用者 token 访问 `system_config`，RBAC 在 `system_config` 侧对调用者直接生效。
4. **异步任务化**：App 安装类重操作返回 `task_id` + 后台任务推进 + 仓库追加审计凭证，进度可观测、操作可审计。
5. **公开面最小化**：未登录可达面被收敛为 7 个公开 RPC 方法 + 3 个 SSO HTTP 路由 + 静态 UI；前端再以 `publicRoutes` 显式列白 `/login`、`/userprofile`。
