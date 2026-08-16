# App 分类与用户可用性实现设计

状态：Frozen / Implemented（Beta 2.2）

本文冻结 `proposal.md` §16 的实现门槛。Beta 2.2 不兼容旧 `apps.list`、旧安装记录或缺少实例 claim 的用户 token。

## 1. 权威模型与持久化

共享类型位于 `buckyos-api::app_availability`。所有组件复用 `evaluate_app_availability` / `AppAvailabilityResolver`，不各自实现优先级。

### 1.1 App 身份

```text
app_instance_id = <app_id>@<owner_user_id>
system_builtin owner = system
zone_installed owner = system
```

`AppServiceSpec.app_class` 为必填枚举：`system_builtin | user_installed | zone_installed`。系统内置 registry 的唯一代码真相源是 `SYSTEM_BUILTIN_APPS`，初始包含 `messagehub`、`homestation`、`content-store`。

### 1.2 安装记录

`APP_INSTALL_SCHEMA_VERSION = 3`。`AppInstallTaskRequest`、`AppUpdateTaskRequest` 和 `InstallRecord` 都持久化 `app_class`；`InstallRecord` 额外持久化 `app_instance_id`。

| 分类 | spec | install record |
|---|---|---|
| user App | `users/{owner}/apps/{app}/spec` | `users/{owner}/apps/{app}/install_record` |
| user Agent | `users/{owner}/agents/{agent}/spec` | `users/{owner}/agents/{agent}/install_record` |
| Zone App | `zone/apps/{app}/spec` | `zone/apps/{app}/install_record` |

Zone App 只允许 Admin/Root 创建和管理，不向用户树复制。系统内置 App 不经过 Installer。

### 1.3 可用性策略

```text
services/control_panel/app_availability/policies/{app_instance_id}
services/control_panel/app_availability/audit/{app_instance_id}/{revision}
```

策略 schema 固定为 v1，字段为 `schema_version`、`app_instance_id`、`default_effect=deny`、`group_rules`、`user_rules`、`revision`、`updated_by`、`updated_at`。

写入是一次 system-config 事务：策略 create/update、不可变审计事件、以及 App spec 内所有 expose service 的 `allow_guest` 同时提交。更新现有策略时以策略 KV 的 system-config version 为 CAS 主键；请求的 `expected_revision` 必须等于策略内容 revision。未配置策略等价 revision 0，首次写入使用 Create。卸载个人 App 时用同一事务把 spec 标记为 Deleted、清空规则并关闭所有 `allow_guest`，策略 revision 继续递增并写入 `uninstall_reset` 审计事件；因此重装不会继承旧授权，同时不会丢失审计序列。

## 2. Control Panel kRPC

所有接口都要求有效用户 session。`apps.availability.set` 还要求 token 的 `principal_kind=user`，设备签发的 App Service token 即使 `sub` 是 Owner 也不能写策略。

### `apps.list`

请求：`{ "user_id"?: string }`。普通用户仅可查询自己；Admin/Root 可查询指定用户。返回 `{ user_id, total, apps[] }`，每项必含：

```text
app_id, app_instance_id, app_class, runtime_type, owner_user_id,
availability_match, enable, state, app_index
```

只返回最终允许且未删除的非 Agent 实例；以 `app_instance_id` 去重和操作。

### `apps.details`

请求：`{ "app_instance_id": string }`。Owner/Admin 或对该实例最终允许的用户可读。

### `apps.availability.get`

请求：`{ "app_instance_id": string }`。仅个人 App；Owner 或 Admin 可读。没有持久策略时返回 revision 0 的默认拒绝策略。

### `apps.availability.set`

请求：

```json
{
  "app_instance_id": "notes@alice",
  "expected_revision": 3,
  "group_rules": [{ "group_id": "users", "effect": "allow" }],
  "user_rules": [{ "user_id": "bob", "effect": "deny" }]
}
```

仅有效 Owner 可写；Admin 不代替其他 Owner 改个人策略。组固定为 `admins/users/limited/guest`，规则主体必须存在且不得重复，Profile/contact groups 不参与授权。

### `apps.availability.check`

请求：`{ "app_instance_id": string, "user_id"?: string }`。返回 `allowed`、分类、Owner、匹配层级和稳定 reason。普通用户只能查自己；Owner 可诊断自己的 App；Admin 可诊断任意关系。`user_id=guest` 使用匿名判定。

安装、升级、启动、停止、卸载 RPC 的外部操作主键统一为 `app_instance_id`；安装请求以 `app_class` 选择个人或 Zone 作用域。

## 3. Verify Hub 与 token

密码和 sudo 请求必须同时携带基础 `appid` 与完整 `app_instance_id`。JWT/SSO 用户登录从 Gateway/redirect 或 `login_params.app_instance_id` 取得实例；第三方个人 App 缺少完整实例时 fail closed。设备/Kernel/服务主体保留设备信任链，不执行普通 User–App 判定。

用户 session/refresh token claims：

```text
appid
app_instance_id
app_owner_user_id   # 非 system_builtin
principal_kind=user
```

Verify Hub 在密码登录、用户主体 JWT/SSO、sudo 和每次 refresh 签发前直接读取 system-config 并调用共享 Resolver。读取失败、实例不存在、用户或 Owner 非 Active、策略无效均返回 `AppAccessDenied`。allow 判定不做无界缓存；策略传播上限是 system-config 最新读取延迟。已签发 session token 不主动撤销，最长保留到 15 分钟 TTL；refresh 必须重新判定。

`verify_token` 可同时接收期望 `appid` 与 `app_instance_id`，两者分别校验，防止同名不同 Owner 实例复用 token。

## 4. Gateway 与 Guest

可用性策略是 Guest 的逻辑真相源。Control Panel 在同一 CAS 事务内把 `guest=allow` 编译到 App spec 的每个 `ServiceExposeConfig.allow_guest`。scheduler 将该字段编译为 Gateway `access_mode=Public|Private`，并在 App entry 中携带 `app_instance_id` 与 `app_owner_user_id`。

Static Web 不再因部署类型自动公开。系统内置 App 的匿名声明由系统 registry/公开配置控制，默认不公开。

## 5. Desktop

生产 Desktop 启动时调用权威 `apps.list`，仅保留 Control Panel 自身管理面板和返回的有效 App。后端 App 定义以 `app_instance_id` 作为 UI id，`logicalAppId` 仅用于选择内置 renderer；同名不同 Owner 生成独立 launcher item。Users & Agents 为每个有权查询的用户分别调用 `apps.list(user_id)`，不再用 spec Owner 推导可用 App。

## 6. 确定性与失败策略

判定顺序固定：实例/主体状态 → Owner → 精确用户 → 匹配组（deny 优先）→ 默认 deny。`AppServiceSpec.enable/state` 不作为授权规则；仅 `Deleted` 从发现和新登录中移除。所有安全读取 fail closed，列表枚举遇到无效单项 spec 时忽略该项，但 system-config 列表或策略读取失败会使整个请求失败。
