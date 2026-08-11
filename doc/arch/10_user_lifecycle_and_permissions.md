# 10. 用户生命周期与权限模型（创建 / 删除 / 用户类型权限）

本文是 BuckyOS **用户管理的权威架构文档**，回答三个问题：

1. 系统里有哪些**用户类型**，它们在权限上有什么不同；
2. **创建用户**时会发生哪些操作——哪些是**系统（OS）负责**的，哪些是 **App 必须遵循标准**自己做的；
3. **删除用户**时会发生哪些操作——同样区分系统职责与 App 标准职责。

与本文档集其它部分一致，约束优先级为：**实际代码 > notepads > 旧 doc**。本文同时承担「定义标准」与「标注现状」两个角色，因此每条职责都带状态标记：

- ✅ **已实现**：当前代码已落地，可依赖。
- ⚠️ **部分实现 / 不一致**：代码存在但与标准有偏差，使用需谨慎。
- ❌ **未实现（标准要求）**：本文定义为标准，但当前代码尚未落地，属于待补齐项。

> 身份验证、token 签发/刷新、RBAC enforce 的底层机制见 [`07_identity_and_rbac.md`](07_identity_and_rbac.md)；本文聚焦“用户”这一主体的生命周期与权限差异，不重复 token 链路。

实现锚点（本文引用的主要代码）：
- 用户 RPC（创建/删除/改类型）：`src/frame/control_panel/src/user_mgr.rs`
- 用户数据结构：`src/kernel/buckyos-api/src/control_panel.rs`
- RBAC 策略生成（role 落地的真正来源）：`src/kernel/scheduler/src/system_config_agent.rs`（`update_rbac`）
- 部署态基础策略：`src/rootfs/etc/scheduler/boot.template.toml`
- 引导期账户/默认应用：`src/kernel/scheduler/src/system_config_builder.rs`
- 登录校验：`src/kernel/verify_hub/src/main.rs`
- App 数据路径约定：`doc/path_usage.md`、`src/kernel/node_daemon/src/app_loader.rs`

---

## 1. 核心数据模型

### 1.1 用户记录 `UserSettings`

用户账户的权威记录是 system-config 中的 `users/<user_id>/settings`（`UserSettings`，定义于 `src/kernel/buckyos-api/src/control_panel.rs`）：

| 字段 | 含义 |
|---|---|
| `user_id` | 用户唯一名（小写，`[a-z0-9_-.]`，1–64 字符） |
| `user_type` | `Root` / `Admin` / `User` / `Limited` / `Guest`（见 §2） |
| `password` | 口令哈希（**客户端计算后上传，服务端从不接触明文**） |
| `state` | `Active` / `Suspended` / `Deleted` / `Banned`（见 §1.3） |
| `res_pool_id` | 资源池，默认 `"default"` |
| `is_local` | 是否为本地持有用户私钥的账号 |
| `allow_password_change` | 是否允许用户自行改密（可选） |

### 1.2 用户相关的 system-config key 命名空间

| Key | 写入者 | 内容 |
|---|---|---|
| `users/<id>/settings` | control_panel / scheduler | `UserSettings`（账户权威记录） |
| `users/<id>/profile` | control_panel / scheduler | `UserPrivateProfile`（用户私有 Profile，含显示名和系统 contact 私有扩展，普通用户可自行读写） |
| `users/<id>/doc` | control_panel / scheduler | 用户 DID 文档 |
| `users/<id>/apps/<app_id>/spec` | control_panel / scheduler | 用户安装的 App 规格 |
| `users/<id>/agents/<agent_id>/spec`、`.../settings` | scheduler | 用户的 Agent 配置 |
| `system/rbac/policy` | **scheduler（`ood` 身份）** | 动态策略尾部：按用户/节点/服务生成的分组行 |
| API runtime 内置 RBAC 配置 | API runtime / scheduler 二进制 | Casbin model + 稳定角色权限；运行时与 `system/rbac/policy` 合成生效策略 |

> 约定：App 自身的可写数据**不在** system-config，而在 DFS / 本地数据目录（见 §5、§6.2）。system-config 只放控制面状态。

### 1.3 用户状态机 `UserState`

`Active → Suspended / Banned / Deleted`。

> ⚠️ **重要现状**：当前删除是“软删除”（仅把 `state` 置为 `Deleted`），但**登录路径不检查 `state`**（`src/kernel/verify_hub/src/main.rs` 的 `handle_login_by_password` 只校验口令哈希）。因此 `Suspended` / `Banned` / `Deleted` 用户当前仍能登录并拿到合法 token。这是标准与实现的关键差距，详见 §4.3 与 §7。

---

## 2. 用户类型与权限差异

### 2.1 类型 → RBAC 角色的映射

用户的有效权限**不来自 `user_type` 字段本身**，而来自 scheduler 在每轮调度时把 `user_type` 翻译成 Casbin 分组行 `g, <user_id>, <role>` 写入 `system/rbac/policy`（`src/kernel/scheduler/src/system_config_agent.rs` 的 `update_rbac`）。该 KV 是动态角色归属的真相源，完整权限还会叠加 API runtime 内置的稳定角色策略。

| `user_type` | 落地 RBAC role | 生成位置 |
|---|---|---|
| `Root` | （不通过 `g` 分配，按**精确主体名 `root`** 匹配；对应 Owner Root Key） | 引导账户 `system_config_builder.rs`；调度桥接把 `Root`→`Admin` role |
| `Admin` | `admin` | `system_config_agent.rs`（`update_rbac`）；创建 admin 时 `user_mgr.rs` 也会追加 `g,<id>,admin` |
| `User` | `users` | `system_config_agent.rs` |
| `Limited` | `limited` | `system_config_agent.rs` |
| `Guest` | （无映射） | — |

### 2.2 各角色的权限（部署态 `boot.template.toml` 基础策略）

资源以 `/config/<key-path>` 粒度授权，`{user}` / `{app}` 等占位符把路径段绑定到主体（实现自身隔离）。默认 **deny-by-default**，显式 `deny` 优先。

| 角色 | 读 | 写 | 关键差异 |
|---|---|---|---|
| **root / Owner** | `/config/*` | `/config/*` | 最高权威；可写 `boot/config`。由 Owner 私钥（钱包）直接签名，“持有 Root Key 即天然 sudo” |
| **admin** | `boot/*`、`system/rbac/policy`（只读）、`system/scheduler/*`（只读） | `agents/*`、`users/*`、`services/*` | 能管理**所有用户**与服务，但**不能改 RBAC 策略本身**（防越权提权）；这是“日常管理模式”而非 Owner |
| **user** | `boot/*`、`agents/*/doc`、自己的 `users/{user}/*`、`services/*/info` | 自己的 `users/{user}/apps/*/*`、`users/{user}/agents/*/*` | 完全限定在**自己名下**（`{user}` 绑定），不能看/改别人 |
| **limited** | — | — | ⚠️ 角色行会生成，但**策略里没有任何 `p, limited, ...` 规则** → 当前等于零权限（全 deny） |
| **guest** | — | — | ❌ `Guest` 类型在 `update_rbac` 里无映射 → 不生成分组行 → 无任何权限 |

> 系统/服务侧角色（与“用户”正交，仅供理解全貌）：`kernel`（`/config/*` 全权，且**绕过用户维度检查**）、`ood`（调度器写策略用）、`service`（系统服务）、`app`（App 服务，按 `{app}` 隔离）。详见 §6.3 与 [`07_identity_and_rbac.md`](07_identity_and_rbac.md)。

### 2.3 双主体 enforce：用户权限与 App 权限是“与”关系

一次来自 App 的请求，是否放行由 `rbac::enforce(userid, appid, resource, action)` 决定，它**分别对用户维度和 App 维度各判一次，再取 AND**（`src/kernel/buckyos-api/src/runtime.rs`、`src/rbac`）：

```
allow = enforce(appid, resource, action)   // App 角色是否允许
      AND
        enforce(userid, resource, action)   // 用户角色是否允许
//（appid == "kernel" 时跳过用户维度）
```

**含义**：一个 App 能对某资源做的事 = 该 App 声明的权限 ∩ 当前操作用户的权限。降权用户（如 `user`）用某个权限很大的 App，也只能在自己 `users/{user}/*` 名下操作。这是“用户类型差异”在运行期真正生效的地方。

### 2.4 用户身份（principal）与设备/服务的区别

| Principal | 私钥位置 | 登录方式 |
|---|---|---|
| **用户**（含 Owner/root） | 钱包 | `login_by_passwd(username, password)` |
| **设备**（OOD/Node） | 设备磁盘 | `login_by_jwt`，设备自签 |
| **服务 / App** | 无（用引导 JWT） | 宿主设备签发一次性 bootstrap JWT，换取 token |

详见 [`07_identity_and_rbac.md`](07_identity_and_rbac.md) §principal 与 `Login机制设计.md`。

### 2.5 已知“类型”相关差距（写入标准，待对齐）

- ⚠️ `Root` / `Guest` 在调度桥接 `map_api_user_type` 中会塌缩：`Root→Admin role`、`Guest→User-或-无`。API 层的 `Root`/`Guest` 与 RBAC 层并非一一对应。真正的 root 是 **Owner Key**（精确名 `root`），不是某个 user_type。
- ⚠️ `limited` 角色无策略规则、`guest` 无映射 → **L3「受限/访客」用户在 [`BuckyOS Security Arch.md`](../BuckyOS%20Security%20Arch.md) 中有设计，但当前未实现**。
- ❌ **sudo / super_user / super_admin**：在 `verify-hub/rbac.md`、`Login机制设计.md` 中有完整设计（`su_<user>` 主体、用户私钥自签、极短 TTL），但**代码中没有运行期提权引擎**——`su_*` 目前只是手写策略约定（测试夹具里出现），无解析、无 TTL、无 super_* 组自动创建。当前唯一“天然高权”路径是 Owner 用 Root Key 直接签名。

---

## 3. 创建用户

存在**两条创建路径**，行为不同，文档与实现都必须区分：

- **路径 A — 引导期（bootstrap）**：Zone 激活时由 scheduler 模板铸造初始 `root` + 首个 `admin`。
- **路径 B — 运行期**：管理员通过 control_panel 的 `user.create` RPC 创建后续用户。

### 3.1 系统职责（OS 负责）

#### 路径 A：引导期（`src/kernel/scheduler/src/system_config_builder.rs`）

| 操作 | 说明 | 状态 |
|---|---|---|
| 写 `users/root/settings` | `Root` 型，`state=Active`，口令哈希同 admin | ✅ |
| 写 `users/<admin>/settings` | `Admin` 型 | ✅ |
| 写 `users/<admin>/doc` | `OwnerConfig`，**Owner 公钥**写入；私钥留在 Owner 侧，绝不入库 | ✅ |
| 追加 `g,<admin>,admin` | 引导期 RBAC 分组 | ✅ |
| 安装默认 Agent `buckyos_jarvis` | 生成 Ed25519 密钥对并写入 | ✅ |
| 安装默认 App | `buckyos_filebrowser`、`buckyos_systest`（来自 `boot.template.toml` 的 `pre_install_apps`） | ✅ |

> 默认 App/Agent **只为引导期 admin 用户**预装，后续用户没有。

#### 路径 B：运行期（`src/frame/control_panel/src/user_mgr.rs` `handle_user_create`）

| 操作 | 说明 | 状态 |
|---|---|---|
| 权限校验 | 仅 Admin/Root 可创建（`require_admin`） | ✅ |
| 用户名校验 | 1–64 字符、字符集限制、拒绝保留名 `root\|system\|admin\|guest` | ✅ |
| 要求 `password_hash` | 服务端不生成、不接触明文口令 | ✅ |
| 拒绝创建 `Root` | 运行期不能造 Root | ✅ |
| 事务写 `users/<id>/settings` + `users/<id>/doc` | `state=Active`，`res_pool_id=default`；doc 为最小 `{id,name,full_name}` | ✅ |
| RBAC 分组 | 用 service token 追加当前用户类型对应的角色分组（`Admin -> admin`，`User -> users`，`Limited -> limited`）；失败仅告警不致命，scheduler `update_rbac` 会在下一轮重建 | ⚠️ |
| DID 密钥对 | **运行期不生成密钥对**（与引导期 `OwnerConfig` 不对称，doc 无公钥字段） | ⚠️ |
| Home 目录 / 数据目录 | **不创建**（见下「App 标准」） | ❌（标准要求显式 provision） |
| 默认 App / Agent | **不安装** | ❌（标准要求可配默认集） |

### 3.2 App 应遵循的标准职责

系统**不会**在创建用户时为 App 准备任何东西。App 必须遵循以下标准，做到“新用户首次到达即可用”：

1. **首次访问即初始化（lazy provisioning）。** App 不能假设系统已为新用户建好数据目录或 App 记录。App 在该用户**首次访问**时，应在自己的 per-user 数据区（§6.2 的 `…/home/<user>/.local/share/<appid>/`）按需创建初始结构。
2. **数据严格按 `owner_user_id` 隔离。** App 的可写数据路径由 loader 烘焙了 `owner_user_id`（系统强制，§6.2），App 不得跨用户读写，也不得把多个用户的数据混存到同一路径。
3. **身份只认 session-token，不自建用户表。** App 判断“当前是谁”必须解析系统下发的 session-token（含 `userid`+`appid`），不得维护独立的用户名/口令体系（§6.1）。
4. **权限判断交给系统 RBAC，不自行放行。** App 对 system-config / kRPC 资源的访问会被 `enforce()` 透明拦截（§2.3）。App 不应假设“能创建用户就能访问其数据”，越权访问会被系统拒绝。

> 标准缺口（❌，待平台补齐以支撑上述约定）：当前没有 `onUserCreate` 事件让 App 预热数据；App 只能依赖“首次访问即初始化”。若未来要支持“管理员建号即为各 App 预置空间”，需要新增用户级生命周期事件（与 §5 的 `onUserDelete` 对称）。

---

## 4. 删除用户

### 4.1 系统职责（`src/frame/control_panel/src/user_mgr.rs` `handle_user_delete`）

当前删除是**纯软删除**：

| 操作 | 说明 | 状态 |
|---|---|---|
| 权限校验 | 仅 Admin/Root | ✅ |
| 拒绝删除 `root` / 拒绝自删 | 安全护栏 | ✅ |
| 置 `users/<id>/settings.state = Deleted` | 仅改状态字段后写回 | ✅ |

### 4.2 系统**当前不做**的事（标准要求，但未实现）

| 应做（标准） | 现状 | 状态 |
|---|---|---|
| 移除 / 失效 `users/<id>/settings`、`users/<id>/doc` | doc 原样保留，settings 仅改 state | ❌ |
| 清理 RBAC 分组行 `g,<id>,<role>` | **不清理**；更糟的是 `update_rbac` 读取 `users/*/settings` 时**不过滤 `state==Deleted`**，每轮调度还会把分组行重新生成 | ❌ |
| 清理 `users/<id>/apps/*`、`users/<id>/agents/*` | 全部成为孤儿 | ❌ |
| 级联通知 App / 调度器 | 无任何回调、无 kevent | ❌ |
| 删除用户名下 DFS / Home 数据 | 不删，全部留盘 | ❌ |
| **登录拦截被删用户** | `handle_login_by_password` 不看 `state` → **被删用户仍能登录** | ❌（安全缺陷，优先级最高） |

### 4.3 App 应遵循的标准职责

> 现实约束：**当前系统在用户删除时不向 App 发出任何信号**（无 `onUserDelete` hook / event / callback）。因此“删除用户后清理其数据”这件事，目前完全没有标准落地路径。

本文将以下定为**目标标准**（待平台补齐 §7 的 `onUserDelete` 事件后，App 必须遵循）：

1. **订阅用户级生命周期事件。** 一旦平台提供 `onUserDelete(user_id)`（与现有 App 安装的 `onAppInstall` 对称），App 应订阅并在收到后清理该用户在本 App 名下的数据（`…/home/<user>/.local/share/<appid>/`）与索引。
2. **遵守 `persistence` 声明。** App 在 manifest 的 `install.mounts[].persistence`（`keep_on_uninstall` / `delete_on_uninstall`，见 `doc/app安装协议.md`）声明数据去留；这套声明目前只在 **App 卸载**时语义化，标准要求未来在**用户删除**时同样适用于“该用户那份数据”。
3. **不依赖系统替你删数据。** 在事件机制落地前，App 必须把“用户已不存在但数据仍在”视为可能状态，至少在 UI / 访问层做到不向已删用户暴露其残留数据。
4. **幂等清理。** 删除清理必须可重试、可重入（软删除可能被恢复，也可能多次触发）。

> 注意区分两条生命周期：**App 卸载**（`path_usage.md` 说 `.local/share/<appid>` 可选删除）针对的是“某 App 在某用户下被卸载”，**不等于**“某用户被删除”。后者目前在系统侧无任何机制。

---

## 5. 系统 vs App 职责总表

| 阶段 | 系统（OS）负责 | App 按标准负责 |
|---|---|---|
| **创建用户** | 写 `users/<id>/settings`+`/doc`；按类型生成 RBAC 分组（admin 即时、其余 scheduler 补齐）；引导期预装默认 App/Agent | 首次访问 lazy 初始化自身 per-user 数据；按 `owner_user_id` 隔离；只认 session-token；权限交给系统 enforce |
| **删除用户** | （现状）软删除置 `state=Deleted`。（标准）应级联清 RBAC/孤儿 key、拦截登录、发 `onUserDelete` | （标准）订阅 `onUserDelete` 后清理本 App 名下该用户数据，遵守 `persistence`，幂等 |
| **隔离边界** | 强制：数据路径烘焙 `owner_user_id`；RBAC 按 `{user}`/`{app}` 绑定；容器不可越界写 | 遵守：不跨用户、不跨 App 读写；不自建身份/权限体系 |

---

## 6. App 视角的强制约束（系统已 ENFORCE 的部分）

为避免把“约定”误读成“保证”，明确哪些是系统**强制**的：

### 6.1 身份（ENFORCED 的网关 + CONVENTION 的转发）

- App 页面身份 = verify-hub 签发、绑定 `userid`+`appid` 的 session-token；写入 cookie `buckyos_session_token`。
- 网关 `check_oauth` 读该 cookie 放行 → **强制**（无 token 进不来）。
- `app-web-page → app-service → kRPC` 链路中，app-service **必须转发页面的 session-token**（代表真实操作者），而非用自己的 service token → 这是 **App 必须遵守的约定**（系统不强制，但用错会导致越权/降权错误）。

### 6.2 数据路径隔离（ENFORCED）

容器内 App 的可写永久数据区固定为（`src/kernel/node_daemon/src/app_loader.rs`、`doc/path_usage.md`）：

```
/home/<owner_user_id>/.local/share/<app_id>/      # 容器视角
$BUCKYOS_ROOT/data/home/<owner_user_id>/.local/share/<app_id>/   # 宿主视角
```

`owner_user_id` 由 loader 烘焙进挂载路径与环境变量（`BUCKYOS_OWNER_USER_ID`/`BUCKYOS_APP_ID`/`BUCKYOS_DATA_DIR`）及容器 label，挂载默认 `ro`、显式 `:rw` 才可写，且被限制在 `BUCKYOS_ROOT` 内 → App **无法**从容器里写入另一个用户的数据区。Files/DFS 侧则是**共享物理根 + ACL 隔离**（不再按用户名切物理根，见 `doc/control_panel/SPEC.context.md`）。

### 6.3 RBAC（ENFORCED，App 无需自己判）

App 访问 system-config / kRPC 资源时，服务端 `BuckyOSRuntime::enforce()` 透明执行权限判断（`src/kernel/buckyos-api/src/runtime.rs`），失败返回 `NoPermission`。`app` 角色按 `{app}` 模板隔离（只能动自己 `apps/{app}/*` 子树）。App **声明**所需权限用 `PermissionItem`，以 `scope_path` 标识 RBAC 资源路径（如 `kapi/aicc`、`user/home`、`zone/library`、`wan`，见 `src/kernel/buckyos-api/src/permission.rs`、`doc/App 安装协议.md`），超出默认沙箱的需安装时用户显式授权，批准结果写入 `AppServiceSpec.permission`。

---

## 7. 待补齐项（本文定义为标准，实现需对齐）

按优先级：

1. **🔴 登录拦截用户状态**：`handle_login_by_password` 必须检查 `state`，对 `Deleted`/`Suspended`/`Banned` 拒绝发 token。当前被删用户仍可登录，属安全缺陷。
2. **🔴 删除时清理 RBAC**：`update_rbac` 应过滤 `state==Deleted` 用户，停止为其重生成分组行；删除路径应主动移除 `g,<id>,<role>`。
3. **🟠 用户级生命周期事件 `onUserDelete`（及可选 `onUserCreate`）**：与现有 `onAppInstall` 对称，作为 App 清理/预热该用户数据的标准触发点（§3.2、§4.3 依赖它）。
4. **🟠 删除语义明确化**：定义软删除（可恢复，保留数据）与硬删除（清孤儿 key + 级联数据）的边界与触发方式。
5. **🟡 `limited` / `guest` 落地**：为 `limited` 写最小策略规则，为 `guest` 建立映射，兑现 [`BuckyOS Security Arch.md`](../BuckyOS%20Security%20Arch.md) 的 L3 受限/访客用户设计。
6. **🟡 sudo / 提权引擎**：把 `Login机制设计.md` / `rbac.md` 的 `su_*` 设计从“手写策略约定”落到运行期（解析 + 短 TTL + super_* 组）。
7. **🟡 运行期用户 DID 密钥**：消除运行期创建用户“doc 无公钥”与引导期 `OwnerConfig` 的不对称。

---

## 8. 与既有文档的关系

- [`07_identity_and_rbac.md`](07_identity_and_rbac.md)：token / trust keys / enforce 底层机制。
- [`BuckyOS Security Arch.md`](../BuckyOS%20Security%20Arch.md)：L0–L3 用户分层与 S0–S3 软件分层的概念模型（本文给出其代码映射与落地差距）。
- `doc/arch/verify-hub/rbac.md`、`doc/arch/verify-hub/Login机制设计.md`：RBAC 与登录/sudo 的设计稿（含未实现部分）。
- `product/users_and_agents/Users_Agents_PRD.md`：Users & Agents 产品形态（联系人/群组/Agent），非 RBAC 角色定义。
- `doc/path_usage.md`、`doc/app安装协议.md`：App 数据路径与 manifest 权限/挂载声明。
