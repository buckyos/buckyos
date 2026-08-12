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



## 主要资源的路径

> 从Agent的万能read开始考虑

资源有各个系统服务提供，用路表达

- obj://config/*
- obj://kmsg/* 
- obj://msg_center/*
- obj://task_mgr/* 由task-mgr提供的task资源，目前该服务有基于自己业务逻辑的权限管理
- obj://$box_id，例如 obj:///msg_center/$userid/box_in_$userid
- kevent 不独立配置，完全看相关对象的res_path 


下面的是规划中未完整实现的

- obj://workflow/*
- obj://dfs/* dfs文件系统路径


## 调度器增量生成规则

调度器构造 `system/rbac/policy` 时，只写会随 system-config 当前状态变化的真实主体组关系和必要的精确授权。稳定的角色权限放在 API runtime 内置默认策略里，完整策略由 API runtime 合成。

### 添加用户

当 `users/$userid/settings` 中存在一个 active 用户时，scheduler 应增加：

```rbac
g, $userid, admin|users|limited
```

用户类型来自 `UserSettings.user_type`：

- `Root` 在 scheduler 视角映射为 `admin`，但 `root` 主体本身由基础策略处理，不应当作普通用户生成。
- `Admin` 加入 `admin`。
- `User` 加入 `user`。
- `Limited` 加入 `limited`。

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
| 登录用户 | `admin`、`user`、`limited` | 可登录用户的日常权限 |
| sudo 用户 | `su_admin`、`su_user` | 仅表达 sudo 后新增的敏感权限 |
| 外部访问者 | `guest`、`friend/contact` | 外部或匿名访问，默认只允许显式 public resource |

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
