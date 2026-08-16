# App 分类与用户可用性管理需求

状态：Implemented

目标版本：Beta 2.2

实现设计与冻结协议见 [`design.md`](design.md)。

本文定义两组相关需求：

1. Control Panel `apps.list` 必须明确区分 App 的运行类型、安装类型和当前用户的可用关系；
2. Control Panel 成为 App–User 可用关系的唯一管理入口，Verify Hub 在为目标 App 签发登录 token 前强制检查该关系。

Beta 2.2 是 breaking change，本文不要求兼容旧的 `apps.list` 返回结构或旧的隐式可见性行为。

---

## 1. 背景与现状

当前 `apps.list` 的结果由两部分拼接：

- `users/<user_id>/apps/<app_id>/spec` 下的用户安装 App；
- Control Panel 代码中硬编码的系统内置 App，目前包括 `messagehub`、`homestation`、`content-store`。

现有实现存在以下问题：

1. `AppType` 的 `service / dapp / web / agent` 描述的是 App 的运行和交付形态，不能表达“系统内置 App、用户安装 App、为所有用户安装的 App”。
2. `is_system: bool` 只能区分系统与非系统，不能说明 App 的安装作用域、Owner 和用户为什么能使用它。
3. `apps.list` 把“App 安装在谁名下”近似当作“谁能看到 App”，没有独立的 App–User 可用关系。
4. App Owner 不能为自己的 App 配置用户组或特定用户的可用规则。
5. Verify Hub 登录时虽然知道目标 `appid`，但当前只验证用户凭证和用户状态，不验证该用户是否允许进入目标 App。
6. 当前 App 实例身份实际采用 `app_id@owner_user_id`。只使用基础 `app_id` 做授权，无法区分不同 Owner 安装的同名 App 实例。

本文中的“可见”不是单纯隐藏 Desktop 图标，而是“用户是否被系统允许发现、登录并使用目标 App”。UI 展示必须由这项系统授权结果派生，不能成为唯一安全边界。

---

## 2. 目标

### 2.1 `apps.list` 目标

- 明确区分 App 的运行类型与安装类型；
- 返回当前用户实际可用的 App，而不是仅返回安装在该用户目录下的 App；
- 说明每个 App 为什么对当前用户可用；
- 支持管理员查看指定用户的有效 App 列表；
- 不允许普通用户枚举其他用户的 App 列表；
- 为 Desktop、Users & Agents、App 管理页提供同一份权威结果。

### 2.2 App–User 可用关系目标

- 普通用户默认可用：系统内置 App、自己安装的 App、管理员为所有用户安装的 App；
- 用户还可以使用其他 App Owner 明确共享给其用户组或个人的 App；
- App Owner 可以随时管理自己 App 的可用用户范围；
- 授权优先按组配置，再用特定用户规则做例外；
- 特定用户精确规则优先于用户组规则；
- 所有修改必须经过 Control Panel 校验和落盘，App 进程不能直接修改授权关系；
- Verify Hub 在登录和 token 刷新时强制执行可用性检查。

### 2.3 安装权限目标

- 普通用户可以为自己安装 App；
- 只有管理员可以把 App 安装为 Zone 级“所有用户 App”；
- 普通用户把自己的 App 共享给 `users` 等用户组，不等同于“为所有用户安装”：该 App 仍由个人 Owner 管理，并随 Owner 的 App 生命周期存在；
- Zone 级所有用户 App 由管理员管理，不依赖某个普通用户的账号生命周期。

---

## 3. 非目标

本期不处理：

- App 内部业务数据的对象级 ACL；
- App 内部自行建立的用户、会员或内容权限系统；
- 自定义组织部门、动态群组和跨 Zone 群组；
- 已签发短期 session token 的主动踢下线；
- Desktop 布局和图标位置的持久化策略；
- 向前兼容旧版 `apps.list.is_system` 或基于旧字段推导可用性。

---

## 4. 核心概念

### 4.1 App 逻辑身份与 App 实例身份

- `app_id`：App 的逻辑名称，例如 `messagehub`、`notes`；
- `app_instance_id`：Zone 内可被独立安装、路由和授权的 App 实例，普通 App 使用现有规范：

```text
<app_id>@<owner_user_id>
```

- 系统内置 App 使用稳定的系统实例身份，例如：

```text
<app_id>@system
```

App–User 可用关系必须绑定 `app_instance_id`，不能只绑定基础 `app_id`。

### 4.2 App Owner

App Owner 是 `AppServiceSpec.user_id` 当前表达的安装所有者。Owner 与正在访问 App 的用户是两个不同概念：同一个 App 实例可以服务多个被授权用户。

对于系统内置 App，Owner 视为 `system`；对于 Zone 级所有用户 App，其安装和可用性控制权属于管理员控制面。

### 4.3 可见、可登录与运行状态

需要区分三种状态：

1. **授权可用**：根据 App 分类和 App–User 策略，用户有权使用该 App；
2. **可登录**：授权可用且 Verify Hub 允许为目标 App 签发 token；
3. **运行可用**：App 实例已安装且运行状态允许提供服务。

`AppServiceSpec.enable` 和 `state` 属于运行状态，不是用户可用性规则。已停止但仍安装的 App 可以继续出现在管理列表中，并明确显示当前不可运行；已删除 App 不得出现在普通 `apps.list` 中，也不得登录。

---

## 5. App 分类

### 5.1 保留现有运行类型

现有 `AppType` 继续描述运行和交付形态，但在 `apps.list` 返回中命名为 `runtime_type`，避免与安装分类混淆：

| `runtime_type` | 含义 |
|---|---|
| `service` | 系统或框架服务形态 |
| `dapp` | App Service，包含 Docker、Script Host 等部署形态 |
| `web` | 静态 Web App |
| `agent` | Agent；普通 `apps.list` 默认仍不返回 Agent |

### 5.2 新增安装分类 `app_class`

| `app_class` | 含义 | 默认可用范围 | 管理者 |
|---|---|---|---|
| `system_builtin` | 跟随 BuckyOS 版本发布的系统内置 App | 所有有效用户 | 系统 |
| `user_installed` | 普通用户或管理员安装在个人名下的 App | Owner 自己 | App Owner |
| `zone_installed` | 管理员安装为 Zone 级所有用户 App | 所有当前及未来有效用户 | 管理员 |

初始 `system_builtin` 清单包含：

- `messagehub`
- `homestation`
- `content-store`

系统内置清单应由一份权威 registry 产生；Control Panel、Desktop 和测试不应长期维护互相独立的硬编码清单。registry 的具体交付形式由实现设计确定。

### 5.3 安装分类与共享关系正交

`user_installed` App 被分享给管理员组或普通用户组后，仍然是 `user_installed`，不能因为使用者变多而变成 `zone_installed`。

`zone_installed` 表达的是 Zone 级安装、生命周期和管理责任，而不只是“当前恰好对所有用户可见”。

---

## 6. 用户主体组

### 6.1 内置可用性组

V1 至少支持以下系统主体组：

| `group_id` | 成员来源 |
|---|---|
| `admins` | `UserType::Admin`；Root 不作为日常登录用户 |
| `users` | `UserType::User` |
| `limited` | `UserType::Limited` |
| `guest` | 无 session token 的匿名主体 |

后续可以增加由管理员维护的 Zone 自定义组，但不属于本期范围。

### 6.2 安全边界

App 可用性组必须由 Control Panel 根据可信的用户类型或专用组成员关系计算。

当前 `UserContactSettings.groups` 属于联系人/Profile 数据，并且用户可以修改自己的该字段，因此严禁把它作为 App 授权组来源。否则用户可以通过修改 Profile 把自己加入高权限组。

`guest` 是保留的匿名伪组，不是普通可登录账号。允许 `guest` 使用 App 时，Gateway 必须使用同一份可用性策略允许匿名访问；Guest 不经过 Verify Hub 登录。

---

## 7. App 可用性策略

### 7.1 策略结构

每个需要自定义范围的 `user_installed` App 实例有一份由 Control Panel 管理的策略：

```json
{
  "schema_version": 1,
  "app_instance_id": "notes@alice",
  "default_effect": "deny",
  "group_rules": [
    { "group_id": "users", "effect": "allow" },
    { "group_id": "limited", "effect": "deny" }
  ],
  "user_rules": [
    { "user_id": "bob", "effect": "deny" },
    { "user_id": "charlie", "effect": "allow" }
  ],
  "revision": 3,
  "updated_by": "alice",
  "updated_at": 0
}
```

逻辑真相源属于 Control Panel 管理的 Zone 控制面状态。实现时应保存在 system-config 的系统管理命名空间中，而不是 App 自己的数据目录或用户可直接写入的 Profile。

所有写入必须带 `revision` 或等价 CAS 条件，避免多个管理页面同时修改时静默覆盖。

### 7.2 默认策略

- `system_builtin`：系统隐式允许所有有效登录用户；匿名 Guest 是否允许由系统 App 的公开访问声明决定；
- `zone_installed`：隐式允许所有有效登录用户；只能由管理员安装、卸载或改变 Zone 安装作用域；
- `user_installed`：Owner 永远允许，其他主体默认拒绝，由 Owner 的组规则和用户规则开放。

Owner 不能通过规则拒绝自己使用自己的 App。Owner 被删除、封禁或 App 被删除后，该隐式允许失效。

### 7.3 匹配优先级

对 `user_installed` App，按以下顺序计算：

1. App 已删除，或 Owner 已删除/封禁：拒绝；
2. 当前用户就是有效 Owner：允许；
3. 存在当前 `user_id` 的精确规则：直接使用该规则；
4. 计算用户所属系统组：
   - 任一匹配组存在 `deny`：拒绝；
   - 否则任一匹配组存在 `allow`：允许；
5. 使用 `default_effect`，V1 固定默认为 `deny`。

因此精确用户 `allow` 可以覆盖组 `deny`，精确用户 `deny` 也可以覆盖组 `allow`。用户同时命中多个组且没有精确规则时，`deny` 优先于 `allow`。

### 7.4 普通用户的有效 App 列表

普通有效用户的 `apps.list` 结果为以下集合的并集：

```text
系统内置 App
+ 自己安装且未删除的 App
+ 管理员为所有用户安装且未删除的 App
+ 其他 Owner 通过组规则或精确用户规则授权的 App
```

集合元素按 `app_instance_id` 唯一标识。同一个基础 `app_id` 由不同 Owner 安装时，不得静默合并为一条。

---

## 8. 管理权限

### 8.1 普通用户 / App Owner

普通用户可以：

- 为自己安装、升级、停止和卸载 `user_installed` App；
- 查看自己 App 的完整可用性策略；
- 通过内置组规则和精确用户规则调整自己 App 的可用范围；
- 将 App 分享给 `admins`、`users`、`limited`、`guest` 或特定用户。

普通用户不可以：

- 创建或修改 `system_builtin`；
- 把 App 安装为 `zone_installed`；
- 修改其他 Owner App 的策略；
- 直接写入可用性策略的 system-config key；
- 修改系统主体组的成员关系。

### 8.2 管理员

管理员可以：

- 把 App 安装为 `zone_installed`；
- 管理 Zone 级 App 的安装、升级、停止和卸载；
- 查询任意用户的有效 App 列表和匹配原因；
- 查询 Zone 内所有 App 的安装分类、Owner 和当前状态。

管理员为所有用户安装 App 时，不需要为每个用户复制一份 App spec 或 grant。新创建用户应自动通过 `zone_installed` 规则获得该 App。

### 8.3 系统内置 App

系统内置 App 跟随 BuckyOS 版本交付，不由普通 App 安装 RPC 创建。其分类和默认用户范围不可由普通 App Owner 修改。

---

## 9. Control Panel API 需求

### 9.1 `apps.list`

#### 请求

```json
{
  "user_id": "optional-target-user"
}
```

- `user_id` 省略时查询当前登录用户；
- 普通用户只能查询自己；
- 管理员可以查询指定用户；
- 查询不存在、Deleted、Banned 或 Suspended 用户时返回明确错误，不返回伪空列表。

#### 返回

```json
{
  "user_id": "bob",
  "total": 1,
  "apps": [
    {
      "app_id": "notes",
      "app_instance_id": "notes@alice",
      "app_class": "user_installed",
      "runtime_type": "dapp",
      "owner_user_id": "alice",
      "availability_match": {
        "type": "exact_user",
        "subject": "bob"
      },
      "enable": true,
      "state": "running",
      "app_index": 10
    }
  ]
}
```

`availability_match.type` 至少支持：

| 值 | 含义 |
|---|---|
| `system_builtin` | 系统内置默认允许 |
| `owner` | 当前用户是 App Owner |
| `zone_all_users` | 管理员为所有用户安装 |
| `group` | 命中允许组规则 |
| `exact_user` | 命中特定用户允许规则 |

组匹配时 `subject` 返回实际 `group_id`，精确匹配时返回 `user_id`。该字段用于 UI 解释和管理员诊断，不返回完整策略内容。

旧字段处理：

- `is_system` 被 `app_class` 替代；
- 旧 `app_type` 改名为 `runtime_type`；
- 旧 `user_id` 的 Owner 语义改为明确的 `owner_user_id`；
- 新增 `app_instance_id` 作为授权、路由和 UI 操作的稳定主键。

普通 `apps.list` 只返回授权允许的 App。管理员需要查看未授权或已删除 App 时，应使用 App 管理列表接口，而不是通过 `apps.list` 增加含混的开关。

### 9.2 可用性策略接口

Control Panel 至少提供以下能力：

```text
apps.availability.get
apps.availability.set
apps.availability.check
```

建议语义：

- `get(app_instance_id)`：Owner 获取自己 App 的完整策略；管理员可用于诊断；
- `set(app_instance_id, expected_revision, group_rules, user_rules)`：用完整新策略做原子替换；
- `check(user_id, app_instance_id)`：返回最终 allow/deny、匹配层级和原因；主要供 Verify Hub、Gateway、测试和管理员诊断使用。

`check` 不是允许普通用户任意枚举 `(user, app)` 关系的公开接口。调用者应限制为 Verify Hub、Gateway、Control Panel 等受信系统主体，以及有权查看目标 App 的 Owner 或管理员。

`set` 必须由 Control Panel 验证：

- 调用者是否是 App Owner；
- 目标用户和组是否存在且可引用；
- 规则是否重复或冲突；
- 是否试图修改系统内置或 Zone 级隐式策略；
- `expected_revision` 是否仍是最新版本。

App Service 自身的 service token 不得调用 `apps.availability.set` 修改自己的可用范围。修改必须来自经过用户认证的 Control Panel 管理请求。

---

## 10. 安装与用户生命周期

### 10.1 安装个人 App

普通用户安装 App 时：

1. 创建 `user_installed` App spec；
2. Owner 默认允许；
3. 其他用户默认拒绝；
4. Owner 后续通过可用性接口增加组或用户规则。

### 10.2 为所有用户安装 App

管理员安装 `zone_installed` App 时：

1. 只创建一份 Zone 级 App 安装记录和运行 spec；
2. 所有当前有效用户自动可用；
3. 后续新建用户自动可用；
4. 不向每个 `users/<id>/apps` 复制 App spec；
5. 普通用户不能卸载或改变其 Zone 级作用域。

### 10.3 新建用户

创建用户不需要枚举并复制系统 App 或 Zone App。用户首次调用 `apps.list` 时，通过分类和规则计算得到：

- 系统内置 App；
- Zone 级所有用户 App；
- 该用户自己安装的 App，初始为空；
- 通过系统组或精确用户规则获得的共享 App。

邀请记录中的 `app_ids` 已移除；App 授权统一由 Owner 通过 `apps.availability.set` 创建明确的实例级规则。

### 10.4 用户类型和状态变化

- 用户类型变化后，组规则的有效结果必须在下一次查询和登录检查时按新类型计算；
- `Deleted`、`Banned`、`Suspended` 用户不得登录任何 App；
- 用户删除后，其作为访问者的精确规则可以保留为审计记录，但不生效；
- user-owned App 的 Owner 被删除或封禁后，App 默认停止对其他用户提供新登录，等待管理员恢复或接管。

### 10.5 App 卸载

- App 卸载后，新的 `apps.list` 不再返回该实例；
- Verify Hub 必须拒绝向该实例签发新 token；
- 可用性策略随安装记录进入 tombstone 或被清理，具体保留策略由持久数据设计确定；
- 重装不能无条件继承旧授权，除非安装流程明确向 Owner 展示并确认恢复旧策略。

---

## 11. Verify Hub 登录强制检查

### 11.1 检查时机

Verify Hub 在以下操作签发新 session token 前必须检查 App 可用性：

- 用户名/密码登录；
- 钱包或 JWT 登录中的用户主体登录；
- SSO 换取本 Zone token；
- refresh token 换取新 token；
- sudo token 仍需满足目标 App 的基础可用关系。

设备、Kernel、系统服务等非用户主体继续走各自的 service/device 信任链，不套用普通 User–App 可用性规则。

### 11.2 目标身份必须无歧义

当前基础 `appid` 不能区分 `notes@alice` 与 `notes@bob`。登录链路必须从 Gateway/redirect 解析出 `app_instance_id`，并把它传给 Verify Hub。

新 session token 至少应绑定：

```text
appid
app_instance_id
app_owner_user_id（非系统 App）
```

如果不增加实例级 claim，则 Verify Hub 对基础 `appid` 的一次授权会错误地允许用户访问同名的其他 Owner 实例。

### 11.3 判定流程

```text
验证用户身份和用户状态
  -> 解析并验证目标 app_instance_id
  -> Control Panel AppAvailabilityResolver.check(user, app_instance)
  -> allow: 签发绑定目标实例的 token
  -> deny: 拒绝登录
```

策略数据读取失败、实例不存在或结果不完整时必须 fail closed，不能因为 Control Panel 不可用而默认允许第三方 App 登录。

### 11.4 拒绝结果

对客户端返回稳定的权限错误，例如 `AppAccessDenied`。外部错误不应泄露完整组规则和其他用户信息；详细匹配原因只写安全日志或返回给有权限的管理员诊断接口。

### 11.5 策略变更后的生效

V1 要求：

- 新登录立即使用最新策略；
- refresh token 换发新 token 时重新检查；
- 被撤销用户不能继续刷新；
- 已经签发的短期 session token 可以继续使用到过期，本期不要求主动踢下线；
- 产品 UI 必须说明撤销对既有会话存在最长一个 session token TTL 的生效窗口。

---

## 12. Gateway 与 Guest

Verify Hub 只能限制需要登录的用户。`guest` 没有 session token，因此 Guest 访问必须由 Gateway 使用同一个 AppAvailabilityResolver 或等价的编译结果判定。

要求：

- Owner 给 `guest` 组配置 `allow` 时，Control Panel 同步生成或更新 App 的公开访问配置；
- Owner 删除 `guest` allow 后，Gateway 停止匿名放行；
- `ServiceExposeConfig.allow_guest` 与 App 可用性策略不能成为两个可独立修改、互相冲突的真相源；
- Static Web 当前不能因为部署形态而被无条件视为 Public，是否匿名可见同样由系统内置规则或 `guest` 策略决定。

---

## 13. 一致性与审计要求

- 可用性策略必须有 `schema_version`、`revision`、`updated_by`、`updated_at`；
- Control Panel 的策略修改应使用事务或 CAS，避免丢失更新；
- 策略变更应记录审计事件，至少包含 App 实例、操作者、旧 revision、新 revision 和变更摘要；
- `apps.list`、Verify Hub 和 Gateway 必须使用同一套纯判定逻辑或同一份编译结果，不能分别实现三套优先级；
- 如果使用缓存，必须定义主动失效或明确的最大传播延迟；登录安全检查不得无界使用旧 allow 结果；
- 对同一输入 `(user state, user groups, app class, app state, policy)`，所有组件必须得到确定性的相同结果。

---

## 14. 主要实现影响面

本需求预计至少影响：

- `src/frame/control_panel/src/app_servcie_mgr.rs`
  - `apps.list` 分类、有效列表计算和权限限制；
  - 新增 availability 管理/检查接口；
- `src/frame/control_panel/src/app_installer.rs`
- `src/frame/control_panel/src/app_install_deployer.rs`
  - 区分个人安装与 Zone 级安装；
- `src/kernel/buckyos-api/src/app_mgr.rs`
  - 新增 App 安装分类、实例身份和可用性数据结构；
- `src/kernel/buckyos-api/src/control_panel.rs`
  - 用户系统组/类型解析接口；
- `src/kernel/verify_hub/src/main.rs`
  - 登录、SSO、刷新、sudo 签发前检查；
- `src/frame/control_panel/src/sys_auth_backend.rs`
  - 从 redirect/Gateway 信息解析完整 App 实例身份；
- `src/kernel/scheduler/src/system_config_agent.rs`
  - Gateway App 信息携带实例身份；Guest 公开策略收敛；
- `src/frame/desktop/src/api/app_mgr.ts`
  - 接收新的 `apps.list` 模型；
- Desktop launcher / Users & Agents
  - 改为消费权威 `apps.list`，不再使用独立硬编码清单解释用户可用性；
- `doc/App 安装协议.md`
  - 增加个人安装、Zone 安装和可用性策略的协议定义；
- `doc/arch/10_user_lifecycle_and_permissions.md`
  - 增加新用户默认 App 和 App–User 生命周期规则。

---

## 15. 验收标准

### 15.1 `apps.list`

- [x] 普通用户能看到所有 `system_builtin`；
- [x] 普通用户能看到自己安装的未删除 App；
- [x] 普通用户能看到所有 `zone_installed`；
- [x] 普通用户能看到通过组或精确用户规则允许的其他 Owner App；
- [x] 普通用户看不到未授权 App；
- [x] 普通用户不能通过 `user_id` 查询其他用户；
- [x] 管理员能查询指定用户并看到每个 App 的匹配原因；
- [x] 返回同时包含 `app_class`、`runtime_type`、`app_instance_id`、`owner_user_id`；
- [x] 同名不同 Owner App 不会被错误合并。

### 15.2 安装权限

- [x] 普通用户可以为自己安装 `user_installed`；
- [x] 普通用户尝试安装 `zone_installed` 被拒绝；
- [x] 管理员安装 `zone_installed` 后，所有当前用户立即可见；
- [x] 在 Zone App 安装后创建的新用户无需复制 spec 即可看到该 App；
- [x] 普通用户共享给 `users` 组的 App 仍显示为 `user_installed`，且生命周期仍属于 Owner。

### 15.3 策略优先级

- [x] 组 `users=allow`、精确 `bob=deny` 时，Bob 被拒绝；
- [x] 组 `limited=deny`、精确 `charlie=allow` 时，Charlie 被允许；
- [x] 同时命中 allow 组和 deny 组且无精确规则时，最终拒绝；
- [x] Owner 不配置任何规则时仍能使用自己的 App；
- [x] 非 Owner 不能修改目标 App 策略；
- [x] App Service token 不能修改自身策略。

### 15.4 Verify Hub

- [x] 未授权用户即使知道目标 URL 和 `appid` 也无法登录；
- [x] 授权用户可以正常取得绑定目标 App 实例的 token；
- [x] `notes@alice` 的授权不能用于登录 `notes@bob`；
- [x] 策略撤销后新登录和 token refresh 被拒绝；
- [x] 系统/服务主体登录不被错误套用普通用户规则；
- [x] 策略读取失败时登录 fail closed。

### 15.5 Guest

- [x] `guest=allow` 时 Gateway 可以匿名访问目标 App；
- [x] 删除 `guest=allow` 后 Gateway 停止匿名访问；
- [x] Guest 不通过 Verify Hub 伪造为普通可登录账号；
- [x] Static Web App 不再因类型为 `web` 就自动公开。

---

## 16. 后续设计门槛

以下实现门槛均已在 [`design.md`](design.md) 冻结并落地：

1. Control Panel kRPC 请求/响应协议；
2. App 分类、可用性策略和 Zone 安装记录的持久数据格式；
3. token 中 App 实例 claim 的命名和验证规则；
4. Gateway 消费 Guest 策略的配置格式；
5. Desktop 权威 App registry 与 `apps.list` 的集成方式；
6. 缓存失效、审计事件和策略变更传播上限。
