# RBAC 设计与配置说明

本文档说明 BuckyOS RBAC 的设计思路和核心资源隔离原则，用来指导内置默认策略与 `system/rbac/policy` 动态尾部的编写。登录、token 签发、密码验证等认证细节不在这里展开。


## 目标

BuckyOS 的权限判断回答的是一个四元组问题：

```text
(userid, appid, action, resource_path)
```

也就是“哪个用户通过哪个 App/Service，对哪个资源执行什么动作”。RBAC 的目标不是只判断用户，也不是只判断服务，而是让系统能同时限制两个维度：

- `userid` 限制用户自己能做什么。
- `appid` 限制当前 App/Service 能代表用户做到哪里。

因此普通应用不能因为用户是 admin 就获得系统级权限；反过来，一个高权限系统 App 也不能替普通用户越过用户自身权限。

## 执行模型

系统服务常用入口：

```rust
BuckyOSRuntime::enforce(&self, req: &RPCRequest, action: &str, resource_path: &str) -> Result<(String, String)>
```

system-config 服务也直接调用：

```rust
rbac::enforce(userid, appid, resource_path, action, sudo_mode)
```

RBAC 目标语义是：

1. 先检查 `appid` 对 `resource_path/action` 是否有权限。
2. 继续检查 `userid`。这里的 `userid` 在用户请求中是用户主体，在设备启动后的内核行为中可以是设备主体。
3. `userid` 通过时，返回 AppID 与 UserID 的交集结果。
4. `userid` 未通过，且请求带有 sudo 模式时，再检查 sudo 主体。

这就是 RBAC 的基本交集模型：请求必须同时满足 AppID 权限和 UserID/DeviceID 权限。


## 策略文件

- API runtime 内置 RBAC 配置：RBAC model 与稳定角色权限，随二进制升级更新。
- `system/rbac/policy`：运行期动态策略尾部，由 scheduler 基于当前用户、节点、服务重新构造。

scheduler 当前会根据系统状态自动追加组关系：

```rbac
g, alice, admin
g, su_alice, su_admin
g, bob, users
g, su_bob, su_user

g, ood1, ood
g, node1, node
g, demo-app, app

```

因此内置默认策略应主要表达稳定的角色权限，`system/rbac/policy` 只保存当前系统里真实用户、节点、服务对应的动态分组行。完整策略由 API runtime 在加载 RBAC 时合成。



## 基于 URL 的资源 key 布局（2026-10-29 定稿）

> 从 Agent 的万能 read 开始考虑：Agent 看到的对象 URL 与 RBAC 的资源 key 是同一个字符串。URL 规范见 [key url.md](../../key%20url.md) 第一部分，裁决记录见 [notepads/agent-object-url-todo.md](../../../notepads/agent-object-url-todo.md) §1.5。本节是第 1 段产出：只定规则与目标 policy，现有 policy 的改动在第 2 段随各服务 Provider 落地。

### 原则

1. **对象路径 = 资源 key。** 服务对象 `obj://<service>/<path>` 与 Agent read / xcall 用的 URL 同形，enforce 不另做映射。host 段只允许 key url.md §4 的两类值：current zone 服务名、带 Zone 的实体 DID hostname。
2. **别名归一化先于 enforce。** `//config/x`、`obj://config/x`、裸 key `x` 在进入 enforce 前归一化为同一个资源 key；带 selector / version 的 query 不参与资源 key（权限按对象给，不按视图给）。归一化后同一资源只有一个 key，换 scheme / 简写不能扩大权限（G4）。
3. **RBAC 只治理 current zone 的资源。** 跨 Zone 的 https Object URL 到达本 Zone 服务后，由网关 / verify-hub 换成本 Zone 的 obj:// 资源 key 再 enforce；外部 token 走 external 主体规则，不绕过本表。
4. **服务内细粒度 ACL 在 RBAC 之后。** RBAC 给组级粗粒度（能不能碰这类资源），msg-center 的 owner 绑定、bucket 的授权 / `bkt_` 令牌、aiworkspace 的分享、homestation 的 reader proof 在服务内再判。RBAC 放行不等于服务放行；RBAC 拒绝则服务不再判。
5. **kevent 不独立配置。** 订阅 `/obj/<host>/<path>/<event>` 的权限 = 对 `obj://<host>/<path>` 的 `subscribe` 权限；`read` 不隐含 `subscribe`。

### 各服务资源 key

| 服务 / host | 资源 key 布局 | 说明 |
| --- | --- | --- |
| `config` | `obj://config/<key_path>`（现有：`boot/*`、`system/*`、`users/{user}/{settings,doc,profile,apps/..,agents/..}`、`services/{service}/{info,settings,instances/{node}}`、`security/*`） | 不变。新增 Zone 级 `system/default_device`（key url.md §3 的 default device），读 `users` / 写 `admin` |
| `dfs` | `obj://dfs/<path>`，与 `cyfs:///<path>` 同一资源、同一 key；现有 `cyfs:///users/{users}/*` 规则迁为 `obj://dfs/users/{users}/*` 并保留 cyfs 写法为别名 | 文件对象能力与文件数据共用权限；NFS 导出与挂载不改变 key |
| `taskmgr` | `obj://taskmgr/<task_id>`（单个 Task）、`obj://taskmgr/users/{users}`（某用户 Task 集合视图） | 现有 `obj://task/{user}` 与服务名不一致，第 2 段改名为 `obj://taskmgr/users/{user}`；TaskMgr 内部按 creator app_id 的细判不变 |
| `kmsg` | `obj://kmsg/<queue_id>` | `read` 拉取、`write` 投递、`subscribe` 事件 |
| `bucket` | `obj://bucket/<scope>/<appid>/<owner>/<name>/<key>`（现有） | 不变；`list\|read\|create\|write\|delete\|manage` |
| `ndm` | `obj://ndm/{uploads,objects,retain,manage}`（现有） | 不变 |
| `msg-center` | 对象路径：`obj://msg-center/owners/<owner_did>/sessions[/<sid>]`、`obj://msg-center/owners/<owner_did>/contacts[/<did>]`、`obj://msg-center/messages/<msg_id>`。资源 key：现有 `inbox/<addr>`、`sent/<addr>`、`group_inbox/<addr>`、`request_box/<addr>`、`objects`、`notify`、`contact_tags/<app>`、`contact_membership`、`group`（create）保留 | `owners/<did>/...` 前缀与现有 `owners/*` 规则同形（裁决 D12）；群组对象 `obj://<group-did-host>/...` 的权限由 GroupMgr 按 action 判 |
| `aiworkspace` | `obj://aiworkspace/<ws_id>`（工作区）、`obj://aiworkspace/<ws_id>/entities/<entity_id>`、`obj://aiworkspace/<ws_id>/path/<...>` | 组级：`users` 对 `obj://aiworkspace/*` `read\|write\|create`，`agent` 对 `obj://aiworkspace/*` `read\|list\|query\|subscribe`；owner / 分享 / public 在服务内按工作区 ACL 判。wish run 的 `proc.*` 仍按 `proc.claim` |
| `homestation` | `obj://homestation/home/{users}`、`.../feed`、`.../feed/@/<key>`、`.../inbox`、`obj://homestation/home/~zone` | owner：`users` 对 `obj://homestation/home/{users}/*` `read\|write\|create\|delete`；public 读由 reader proof / 公开策略在服务内判；`inbox` 的 `write` 即投递动作；跨 Zone 投递经网关换成本 key 后以 external 主体 enforce |
| `opendan` | `obj://opendan/`（发现，`list\|query`）；Agent 对象 `obj://<agent-id>/{sessions,workspaces,goals,artifacts}/...` | Agent 自己：`p, agent, obj://{agent}/*, all, allow`（matcher 以 `app:` + 变量绑定到 Agent 的 App 主体）；owner 读自己 Agent 的对象由 scheduler 在"添加 Agent"时生成精确规则 `p, <owner>, obj://<agent-id>/*, read\|list\|query\|subscribe, allow`；其他用户默认无 |
| `aicc` | `obj://aicc/on_behalf_of`、`obj://config/services/aicc/*`（现有） | 不变 |
| 实体对象（通用） | `obj://<entity-host>/<path>`，entity-host 为 User / Agent / Group / Device DID 的 canonical hostname | 实体自己 `all`；owner 规则由 scheduler 按实体类型生成；不靠通配 `obj://*` |

### agent 组的默认权限（G1 ③）

现状 `p, agent_runtime, obj://msg-center/*, all, allow` 与 `p, agent, obj://msg-center/*, all, allow` 过宽，且 msg-center 的 owner 绑定对 App / Agent token 落 `_ => Ok(())`。目标：

```rbac
# Agent 自己的 mailbox（addr 以 Agent DID 开头）
p, agent, obj://msg-center/inbox/{agent}/*,        read|write, allow
p, agent, obj://msg-center/sent/{agent}/*,         read|write, allow
p, agent, obj://msg-center/request_box/{agent}/*,  read|write, allow
p, agent, obj://msg-center/objects,                read,       allow
p, agent, obj://msg-center/notify,                 read|write, allow
# 观察 owner 的会话与联系人：只读（Message Center.md §5.6），owner 绑定由 msg-center 按 agent_owner 判
p, agent, obj://msg-center/owners/*/sessions/*,    read|list|query|subscribe, allow
p, agent, obj://msg-center/owners/*/contacts/*,    read|list|query,           allow
# agent_runtime（未绑定 AgentSpec 前的 App runtime）只读自己的 spec，不碰 msg-center
```

`agent_runtime` 组删除 msg-center 规则；`system:opendan` 的 `obj://msg-center/*` `all` 保留但只用于 loader 代 Agent 建 mailbox / 注册，不用于读 owner 数据。`agent_read_only` 的 `obj://*` `^(read|list|query|subscribe)$` 继续作为只读 Agent 的底座。

### Profile `x-buckyos:rbac` / `risk` 与 RBAC action 的对应（G2）

| Profile 能力 | `x-buckyos:risk` | RBAC action | Governance |
| --- | --- | --- | --- |
| property 读 | `read_only` | `read` | allow |
| IndexTrait `query` / `page` | `read_only` | `list` 或 `query` | allow |
| event 订阅 | `read_only` | `subscribe` | allow；`read` 不隐含 `subscribe` |
| action，只读（不改资源） | `read_only` | `read` | allow |
| action，幂等写（重试安全但改资源） | `idempotent_write` | `write` | 按资源走 require_approval；幂等 ≠ 只读 |
| action，一般写 | `write` | `write` | require_approval |
| action，创建子对象 | `write` | `create` | require_approval |
| action，删除 | `destructive` | `delete` | require_approval，默认 deny 给 agent |
| action，管理（权限、授权、令牌） | `admin` | `manage` | deny 给 agent，除非精确规则 |

Profile 必须为每个 action 声明 `x-buckyos:rbac`（资源 key 模板 + action）与 `x-buckyos:risk`；未声明的方法不可调用，不由路径猜。Governance 的 allow / require_approval / deny 先看 RBAC（拒绝即止），再看 risk。

### 现有 policy 的改动清单（第 2 段执行）

> 2026-10-29 执行结果：1、2、4、6 已落地（`rbac_config.rs` 默认策略、scheduler `update_rbac`、msg-center `owner_session.rs`、task-manager `acl.rs`，回归测试 `rbac_config::tests::agents_observe_owner_sessions_read_only_and_own_their_objects`、msg-center `mailbox_owner_binding_follows_the_tokens_principal`）；3 无需迁移：默认策略里没有 `cyfs:///users` 规则，DFS 的路径权限在 nfs-server 的路径 ACL，`obj://dfs/<path>` 与 `cyfs:///<path>` 经 `buckyos_api::ObjectUrl::rbac_resource_key` 归一到同一 key；5 只完成了事件改名，kevent 服务目前不鉴权订阅者（kernel/kevent 无 enforce），按对象 key 的 `subscribe` enforce 仍待 kevent 携带调用者身份后实现。msg-center 的 RBAC key 编码同时调整：`MailboxAddress` 不再把 `.` 编成 `%2E`（与 `obj://` 规范化一致），scheduler 生成精确规则时用 `buckyos_api::rbac_literal` 转义正则元字符。

1. 删除 `rbac_config.rs` 中 `agent_runtime` / `agent` 对 `obj://msg-center/*` 的 `all`，换成上面的 agent 组规则；msg-center `authorize_owner_read_identity` 的 `_ => Ok(())` 与 `None => Ok(())` 分支改为按 token owner / sub 校验。
2. `obj://task/{user}` → `obj://taskmgr/users/{user}`，TaskMgr 的 enforce 调用同步。
3. `cyfs:///users/{users}/*` → `obj://dfs/users/{users}/*`，归一化层把 `cyfs:///` 别名映射到同一 key。
4. 新增 `aiworkspace`、`homestation`、`obj://opendan/`、实体对象的组级规则；scheduler "添加 Agent" 增量规则补 owner 对 `obj://<agent-id>/*` 的只读。
5. kevent 订阅改为按对象 key 的 `subscribe` enforce；现有 `/msg_center/...`、`/aiworkspace/...`、`/task_mgr/...` 前缀随 D9 迁到 `/obj/<host>/...`。
6. 新增 `system/default_device` 配置键及其读写规则。

## 调度器增量生成规则

调度器构造 `system/rbac/policy` 时，只写会随 system-config 当前状态变化的真实主体组关系和必要的精确授权。稳定的角色权限放在 API runtime 内置默认策略里，完整策略由 API runtime 合成。

### 添加用户

当 `users/$userid/settings` 中存在一个 active 用户时，scheduler 应增加：

```rbac
g, $userid, admin|users|limited
```

用户类型来自 `UserSettings.user_type`（权威定义见 [UserType.md](../../../product/users_and_agents/UserType.md)）：

- `Root` 不生成组关系，`root` 主体由基础策略处理。
- `Admin` 加入 `admin`。
- `User` 加入 `users`。
- `Limited` 加入 `limited`。`limited` 只有自身策略：`p, limited, obj://config/users/{limited}/settings, read`、`.../profile, read|write`，以及 `p, limited, obj://msg-center/owners/*/contacts, read|write`（自己的通讯录；msg-center 把 owner 绑定到调用者，并按“能否改通讯录”拦写）。
- 其它 `type` 值不进入任何组，scheduler 告警并跳过。

用户所在的系统用户组（`UserSettings.groups`，定义在 `system/user_groups/<id>`）生成：

```rbac
g, $userid, group:$group_id
```

系统用户组不带任何 `p` 规则，只作为 App 可用性、资源授权和通讯录投影引用的身份标签。

如果该用户支持 sudo，还应生成 sudo 主体关系：

```rbac
g, su_$userid, su_admin
g, su_$userid, su_user
```

具体加入 `su_admin` 还是 `su_user` 取决于原用户类型。`limited` 用户只有存在明确新增权限时才需要生成 sudo 规则。

涉及用户自有资源的 sudo 权限必须生成精确规则，例如：

```rbac
p, su_bob, obj://config/users/bob/settings, write, allow
```

不要用 `p, su_user, obj://config/users/{user}/settings, write, allow` 表达这类权限。当前 matcher 不能把 `su_bob` 反向绑定到 `bob`，该规则会退化为对所有 `obj://config/users/*/settings` 的通配匹配。

当用户被禁用、删除或不再 active 时，scheduler 通过重新构造 `system/rbac/policy` 移除该用户及其 sudo 主体的动态规则。

### 添加设备

当 `devices/$deviceid/info` 中出现设备，scheduler 会把它加入调度上下文。设备权限应遵守 `BuckyOS 设备权限.md` 的核心结论：DeviceID 应成为权限判断的一等输入；在当前 RBAC 接口仍以 `userid + appid` 为主的阶段，可以先用 `$deviceid` 作为兼容主体，但策略必须保留“这是设备主体”的语义。

设备角色必须来自可信状态，例如 DeviceDoc、ZoneConfig 或管理员配置，不能来自设备自己上报的普通 `DeviceInfo` 字段。


按设备类型，scheduler 的组关系是：

```rbac
g, ood1, ood
g, gateway1, gateway
g, node1, server
g, alice-macbook, client
g, pir-living-room, sensor
g, zigbee-hub, iot_controller
```

其中 OOD 和 Gateway 都属于 Kernel Device 的高信任角色，但不等于无差别 root。若一台设备同时是 OOD 和 Gateway，应同时加入两个角色：

```rbac
g, ood-gw1, ood
g, ood-gw1, gateway
```

设备自身路径的权限应生成精确规则。典型规则：

```rbac
p, node1, obj://config/devices/node1/info, read|write, allow
p, node1, obj://config/nodes/node1/config, read, allow
p, node1, obj://config/nodes/node1/gateway_config, read, allow
```

这表达“设备只能默认更新自己的 DeviceInfo，只能读取自己的 node config”。不要用下面这种 role 级规则表达自有设备路径：

```rbac
p, server, obj://config/devices/{device}/info, read|write, allow
```

因为当前 matcher 不会把 `server` 角色反向绑定成某个具体 device id。

OOD 可以拥有维护 Zone 内节点配置的角色权限，例如读全局配置、写调度结果、写 `system/rbac/policy`。普通 Server Node 不应因为运行同名 `node-daemon` 或 `AppID=kernel` 就获得 OOD 权限。

Client Device 默认权限不应超过 device owner。当前 RBAC 尚未把 `DeviceID + UserID + AppID` 同时作为一等输入时，Client Device 的策略应保守：允许认证接入、服务发现和读取必要拓扑信息；涉及用户数据访问仍由显式用户 token 和 AppID 继续约束。

Sensor / IoT 设备只应获得极小写权限，例如写自己的 DeviceInfo，或向自己命名空间下的事件路径 fire event：

```rbac
p, pir-living-room, obj://config/devices/pir-living-room/info, read|write, allow
p, pir-living-room, obj://kevents/devices/pir-living-room/*, fire, allow
```

高频遥测、事件流和应用状态不应写入 system-config，应进入 IoT Hub、消息系统、事件系统或专用应用存储。

### 添加 App

当 `users/$userid/apps/$appid/spec` 存在且 App 需要运行时，scheduler 应把该 AppID 加入 `app` 角色：

```rbac
g, $appid, app
```

App 的通用权限应通过内置默认策略的 `{app}` 变量绑定 AppID，例如：

```rbac
p, app, obj://config/users/*/apps/{app}/settings, read|write, allow
p, app, obj://config/users/*/apps/{app}/info, read|write, allow
```

这类规则表达“App 只能访问自己的 App 子树”。它不绑定用户，因此最终仍要依赖 UserID 维度限制当前用户能访问哪些 `users/$userid` 路径。

App spec/config 通常由安装流程、scheduler 或系统服务生成。普通 App 可以读必要 spec/config，但不应直接写自己的 spec/config，否则会绕过安装、调度和审计流程。

如果 AppID 在多个用户下安装，仍然只需要一条：

```rbac
g, $appid, app
```

因为 AppID 维度限制 App 子树，UserID 维度限制具体用户。

### 添加 Agent

Agent是一种特殊的App,但权限比App要大一些
1）Agent有自己的身份文件
2）Agent有自己的rootfs,以appdata的方式保存的，因为一个Agent实例只为一个用户服务。
3）Agent通常会得到自己Owner的授权

当前 scheduler 会把 `users/$userid/agents/$agentid/spec` 与普通 App spec 一样构造成 `ServiceSpecType::App`。因此 runnable Agent 也应先加入 `app` 角色：

```rbac
g, $agentid, app
```

同时内置默认策略应为 Agent 子树提供与 App 子树对应的隔离规则：

```rbac
p, app, obj://config/users/*/agents/{app}/settings, read|write, allow
p, app, obj://config/users/*/agents/{app}/info, read|write, allow
```

如果未来需要区分普通 App 和 Agent 的权限，应增加独立的 `agent` 角色，而不是让所有 Agent 继承更多 `app` 权限。Agent 的 session、工具授权、长期记忆和工作目录通常比普通 App 更敏感，默认应只访问自己的 agent scope 和被用户显式授权的资源。

## 策略表达规则

RBAC policy 使用两类规则：

```rbac
p, subject_or_role, resource_pattern, action_regex, allow
g, subject, role
```

当前模型的 matcher 有一个重要特性：如果资源 pattern 里存在与 `p.sub` 同名的变量，就会把该变量绑定到当前请求主体；如果取不到同名变量，则只做路径匹配。

例如：

```rbac
p, users, obj://config/users/{users}/*, read, allow
g, bob, users
```

`bob` 只能读 `obj://config/users/bob/*`，不能读 `obj://config/users/alice/*`。因为 pattern 里的 `{users}` 会和 `p.sub == users` 绑定。

再比如：

```rbac
p, app, obj://config/users/*/apps/{app}/settings, read|write, allow
g, photos, app
```

`photos` 只能访问任意用户下 `apps/photos/settings`，不能访问 `apps/mail/settings`。这就是 AppID 维度的隔离。

但下面这类规则不会绑定原用户：

```rbac
p, su_user, obj://config/users/{user}/settings, write, allow
```

因为当前主体是 `su_bob`，角色是 `su_user`，而 pattern 里的变量名是 `{user}`，不是 `{su_user}`。它会退化成对 `obj://config/users/*/settings` 的通配匹配。需要按原用户绑定时，应由 scheduler 生成精确规则。

## 主体分层

RBAC 主体大体分为几类：

| 主体 | 典型角色 | 设计语义 |
| --- | --- | --- |
| `root` | `root` | Zone owner/root key 对应的最高权限；系统不应把普通 sudo 配成 root 等价权限 |
| 内核服务 | `kernel` | node-daemon、scheduler、system-config、verify-hub 等核心服务 |
| 系统服务 | `system` | repo-service、msg-center、aicc 等系统服务，通常只写自己的服务配置和服务数据 |
| 用户应用 | `app` | 普通 App/Agent，必须被 AppID 限制在自己的 app scope 内 |
| OOD/Node | `ood`、`node` | 设备和节点主体，用于节点状态上报、节点配置读取等 |
| 登录用户 | `admin`、`users`、`limited` | 可登录用户的日常权限；`limited` 只有自身资料权限 |
| 系统用户组 | `group:<id>` | 管理员维护的标签，不带系统策略 |
| sudo 用户 | `su_admin`、`su_user` | 仅表达 sudo 后新增的敏感权限 |
| 外部访问者 | 无 | External（无账号记录的已认证 DID）与匿名访问者不进入任何 RBAC 组，权限只来自资源方授权 |

## 核心资源隔离

### boot config

`obj://config/boot/*` 是 Zone 的根信任信息，例如 ZoneConfig、owner public key、verify-hub public key 等。

设计原则：

- 大多数主体可以读取必要的 boot 信息，以便完成验签和系统发现。
- 写权限必须极少，通常只属于 `root/kernel` 级别。
- `admin` 即使用 sudo，也不应获得随意修改 boot/zone-config 的权限。

### RBAC 配置

`obj://config/system/rbac/policy` 与 API runtime 内置 RBAC 配置共同控制系统授权行为。

设计原则：

- model 和稳定角色权限是 API runtime 内置配置，随二进制升级更新。
- `policy` 是 scheduler 构造的动态尾部，应由 scheduler/OOD/kernel 路径维护。
- 普通 service 可以读 RBAC，用于本地 enforce；不应直接写。
- 写 `policy` 等价于改变系统授权结果，必须当作高敏感操作。

### 用户配置

`obj://config/users/$userid/*` 保存用户设置、profile、应用设置、agent/app 配置等。

设计原则：

- 普通 `user` 只能访问自己的用户树。
- `admin` 可以管理用户配置，但不等于能读取或改写所有用户的私密数据。
- `settings` 这类敏感用户配置可以要求 sudo。
- `profile` 保存 `UserPrivateProfile`，普通用户可自行读写，不要求 sudo。
- 需要绑定原用户的 sudo 权限，使用 scheduler 生成精确规则：

```rbac
p, su_bob, obj://config/users/bob/settings, write, allow
```

用户名创建后不可修改，因此这类精确规则可以稳定生成。

### App 子树

`obj://config/users/$userid/apps/$appid/*` 和 `obj://config/users/$userid/agents/$agentid/*` 是 App/Agent 的用户域配置。

设计原则：

- AppID 只能访问自己的 app scope：

```rbac
p, app, obj://config/users/*/apps/{app}/settings, read|write, allow
p, app, obj://config/users/*/apps/{app}/info, read|write, allow
```

- App 不能通过用户 token 访问其它 App 的配置。
- 用户可以管理自己名下 App 的日常配置。
- App spec/config 这类由系统生成或安装流程管理的数据，不应被普通 App 随意写。

### Service 配置

`obj://config/services/$service/*` 保存系统服务或 App Service 的运行配置、状态、info 等。

设计原则：

- `service` 只能写自己的服务配置：

```rbac
p, service, obj://config/services/{service}/*, read|write, allow
```

- `services/*/info` 可以给其它服务或用户读取，用于服务发现和状态展示。
- 修改服务 spec、部署状态、调度结果通常属于 admin/scheduler/kernel 权限。

### Node 和 Device

`obj://config/devices/$device/*` 与 `obj://config/nodes/$node/*` 表达设备 DID、设备状态、节点运行配置、gateway config 等。

设计原则：

- Device/Node 的 doc 类信息可读，用于身份验证和系统发现。
- 设备实时 info 由对应设备或 node-daemon 上报。
- node config/gateway config 由 scheduler/OOD 构造，普通用户和普通 App 不应写。
- OOD 可以管理 Zone 内节点配置；普通 server node 不应具备 OOD 级别权限。


## 用户类型与 sudo

可登录用户与 RBAC 的关系如下：

| 用户类型 | 普通主体 | sudo 主体 | RBAC 配置目标 |
| --- | --- | --- | --- |
| root | `root` | `root` | 最高权限，不是日常可登录用户 |
| admin | `$userid` | `su_$userid` | 日常管理权限在 `admin`；敏感管理动作放到 `su_admin` |
| user | `$userid` | `su_$userid` | 日常使用权限在 `user`；敏感自有数据操作放到 `su_user` |
| limited | `$userid` | `su_$userid` | 只有存在明确新增权限时才需要 sudo |

sudo 的当前套路是机械的身份切换：

1. 用户用普通账号登录，得到普通 session token。
2. 需要敏感操作时，通过 verify-hub 拿到短时 sudo session token。
3. 请求携带 sudo token 时，鉴权层把用户主体从 `$userid` 映射成 `su_$userid`。
4. RBAC 先检查 `$userid`；未通过时才检查 `su_$userid`。

因此 sudo policy 不需要是原用户权限的超集，只需要表达“sudo 后新增的权限”。

建议使用两个 sudo 权限组承载通用 sudo 权限：

```rbac
g, su_alice, su_admin
g, su_bob, su_user
```

但凡权限必须绑定原用户身份，都应由 scheduler 生成 per-user 精确规则，避免 `su_user` 组规则意外放开所有用户数据。

## 配置原则

编写或生成 RBAC policy 时至少遵守：

1. 先判断资源属于哪个隔离域：boot/system/users/apps/services/nodes/dfs。
2. 用户权限和 AppID 权限都要收窄；不要只看其中一个维度。
3. 有 `{user}`、`{app}`、`{service}` 这类变量时，确认变量名是否真的会绑定当前主体。
4. 需要绑定原用户身份的 sudo 权限，用 scheduler 展开精确规则。
5. `admin` 和 `su_admin` 都不等于 `root`。
6. App/Service 只给自己的 scope 写权限；跨 scope 读写必须是明确的产品需求。
7. `obj://config/system/rbac/*`、`obj://config/boot/*`、node config、service spec 这类控制面数据默认按高敏感处理。

## 需要避免的配置

不要用宽泛规则替代隔离设计：

```rbac
p, app, obj://config/users/*, read|write, allow
p, service, obj://config/services/*, read|write, allow
p, su_user, obj://config/users/*, read|write, allow
p, su_admin, obj://config/*, read|write, allow
```

这些规则会破坏 AppID 隔离、服务自有配置隔离、用户自有数据隔离，或者把 sudo admin 提升成事实上的 root。
