# SystemServiceId 登录与 SSO 目标修复 TODO

> 面向后续 CodeAgent。
>
> 本文记录 Desktop 在 DV 环境登录时报错
> `RPC call error: Parse Request Error: redirect_url target '_' is a system service, not an AppInstance`
> 的根因、冻结设计和实施步骤。
>
> 当前是 beta 2.2 breaking change，不保留 `control-panel@system` 等旧格式兼容路径。

---

## 0. 目标

修复用户登录到 BuckyOS 系统服务时，被错误要求提供 `AppInstanceId` 的问题，并把登录、token、刷新、sudo、SSO callback 和 token verification 的目标模型统一为：

```text
登录主体（谁在登录）                 服务目标（登录给谁使用）

principal_kind = user               SystemServiceId
                                      例：control-panel、kmsg

principal_kind = user               AppInstanceId
                                      例：filebrowser.example@alice
```

必须满足：

- `SystemServiceId` 是不带 `@` 的系统服务唯一 ID，例如 `control-panel`、`kmsg`。
- `AppInstanceId` 是 `{app_id}@{owner_user_id}`。
- 系统服务不创建虚假的 AppSpec、AppRegistry 记录或 AppInstanceId。
- 用户登录系统服务时，`principal_kind` 仍然是 `user`，不能错误改成 `system`。
- 普通 App token 继续精确绑定 AppInstanceId；不能退化为只校验 AppId。
- Gateway、共享本地验签和 RBAC 都必须保留 App/System target kind，不能只比较裸 `appid`。
- App 页面 Cookie 鉴权必须精确比较 AppInstanceId 和 owner，不能让同一 AppId 的不同 Owner 实例互相放行。
- SSO callback 和 refresh 必须绑定签发时的 canonical origin/route target，不能把 token 送到另一个 host。
- Desktop 根域登录、refresh、logout、sudo 和普通 App SSO 都能工作。

---

## 1. 当前故障与根因

### 1.1 复现路径

Desktop 没有有效 session 时，`src/frame/desktop/src/main.tsx` 构造：

```text
/login?appid=control-panel&redirect_url=https://<zone-host>/...
```

Control Panel 处理 `auth.login` 时：

1. `redirect_url` host 等于 Zone host。
2. `resolve_sso_target_app_instance_id` 将根域映射为 Gateway `app_info` key `_`。
3. `_` 的路由项是 `service_id = control-panel`，没有 `app_instance_id`。
4. 当前代码无条件要求 `app_instance_id`，于是返回：

```text
redirect_url target `_` is a system service, not an AppInstance
```

错误发生在密码校验和 token 签发之前，与 DV 用户密码、Cookie 或 TLS 配置无关。

### 1.2 回归来源

提交 `9c817e8b` 删除了旧的 `control-panel@system` fallback，同时保留了“所有用户登录目标都必须是 AppInstance”的流程，导致系统服务登录没有合法表示。

这个提交删除虚假 AppInstance 的方向是正确的，缺失的是显式的 System target 分支。不得通过恢复下面任一做法修复：

- 不得恢复 `control-panel@system`、`kmsg@system`。
- 不得给 Gateway `_` 人工增加 `app_instance_id`。
- 不得给系统服务创建普通用户 AppSpec 或 AppRegistry entry。
- 不得把用户登录 token 的 `principal_kind` 改为 `system`。

### 1.3 已有可复用定义

优先复用：

- `buckyos_api::SystemServiceId`
- `buckyos_api::ServiceIdentity::{App, System}`
- `buckyos_api::CONTROL_PANEL_SERVICE_UNIQUE_ID`
- `buckyos_api::KMSG_SERVICE_UNIQUE_ID`
- 其它 `*_SERVICE_UNIQUE_ID`
- `buckyos_api::AppId`
- `buckyos_api::AppInstanceId`

`*_SERVICE_UNIQUE_ID` 的值就是 SystemServiceId 的 canonical string，不再把它称作 AppId。

---

## 2. 冻结设计

### 2.1 Principal 与 Target 必须正交

`principal_kind` 描述 token 的主体类型：

```text
user / device / app / system / agent
```

登录 target 描述 token 要访问的服务实例：

```text
App target       -> 精确 AppInstanceId
System target    -> SystemServiceId
```

因此，用户在 Desktop 登录后的 token 是：

```text
principal_kind = user
target_kind    = system
appid          = control-panel
```

而不是：

```text
principal_kind = system              # 错误：丢失真实用户主体语义
app_instance_id = control-panel@system # 错误：伪造 AppInstance
```

### 2.2 新增共享 AuthTarget/TokenTarget 强类型

在 `buckyos-api` 的合适共享模块新增一个明确的目标类型。命名可在实现时结合现有模块选择 `AuthTarget` 或 `TokenTarget`，语义必须如下：

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AuthTarget {
    App {
        app_instance_id: AppInstanceId,
    },
    System {
        service_id: SystemServiceId,
    },
}
```

要求：

- 不用裸 `String` 在各层重复判断是否包含 `@`。
- App 分支从 `AppInstanceId` 确定 AppId。
- System 分支只接受 `SystemServiceId::parse` 通过的值。
- 提供 canonical target/cache key，例如 `app:<app_instance_id>` 和 `system:<service_id>`，避免命名碰撞。
- 另提供 kind-aware authorization identity：App 分支至少是 `app:<app_id>`，System 分支是 `system:<service_id>`；RBAC 不得继续只吃裸 `appid`。
- 如需转换到已有 `ServiceIdentity`，集中实现转换，不在调用点手工拼装。

`ServiceIdentity::App` 只包含 AppId，不能单独承担 token 的 App target，因为认证必须精确到 AppInstanceId。

target 的精确绑定与 RBAC policy 粒度是两件事：token 必须绑定 AppInstanceId；如果当前 RBAC 仍按 AppId 授权，也必须用带 kind 的 AppId key，避免 `AppId("control-panel")` 与 `SystemServiceId("control-panel")` 权限碰撞。

### 2.3 Token claim 约束

保留 `RPCSessionToken.appid` 作为目标的兼容字段，但必须增加显式 target kind；不能仅通过 `app_instance_id` 是否存在来猜测，因为“损坏的 App token”不能被降级解释为 System token。

在共享 API 中新增：

```text
target_kind = app | system
token_use   = session | refresh
```

现有 `sudo` claim 继续表示 session 的提权子类型；用途组合必须唯一且集中校验：

```text
普通 session -> token_use=session, sudo=false
sudo session -> token_use=session, sudo=true
refresh       -> token_use=refresh, sudo=false
```

其它组合一律拒绝，不能让调用方分别猜测 token 的用途。

并提供集中式 bind/parse/validate helper。最终 claim 约束：

| Target | `appid` | `target_kind` | `app_instance_id` | `app_owner_user_id` |
|---|---|---|---|---|
| App | canonical AppId | `app` | 必须存在且 AppId 匹配 | 必须存在且 owner 匹配 |
| System | canonical SystemServiceId | `system` | 必须不存在 | 必须不存在 |

要求 fail closed：

- 缺少或未知 `target_kind` 时拒绝新 token。
- `target_kind=app` 但缺少/损坏 AppInstanceId 时拒绝。
- `target_kind=system` 但带 AppInstance/owner claim 时拒绝。
- `appid` 与目标强类型不一致时拒绝。
- `principal_kind` 与 `target_kind` 不得混为一个字段。
- session token 与 refresh token 必须携带完全相同的 target。
- session/sudo token 必须是 `token_use=session`；refresh token 必须是 `token_use=refresh`，并校验上述 `sudo` 组合，不能只靠 Cookie 名或调用路径猜测。
- refresh token 不能作为普通 RPC/App 页面 session token 使用；sudo token 也不能写入普通 SSO session Cookie。

签发与验签边界冻结如下：

- 所有 Verify Hub 签发的 session、refresh、sudo token，不论 `principal_kind`，都必须携带合法的 `token_use + AuthTarget`。
- Root/User/Device/System/Agent 私钥直接签发、用于调用 `login_by_jwt` 的 JWT 是 **LoginAssertion**，不是 Verify Hub session token；它只能进入 exchange/login 入口，不能直接被 Gateway、RBAC 或普通服务当作 session token。
- `generate_service_login_jwt`、`RPCSessionToken::generate_jwt_token` 等现有构造器必须明确返回 LoginAssertion，或拆成命名清楚的 assertion helper；不能继续让同一个 wire object 同时扮演 assertion 和 session。
- `BuckyOSRuntime::verify_trusted_session_token` 必须对 Verify Hub issuer 的 token 调用共享 target/token-use validator；task-manager、workflow、Control Panel 等本地验签调用方不能绕过 claim invariants。
- 非 Verify Hub issuer 的 LoginAssertion 在普通 session 验签入口必须拒绝；只允许在明确的 boot/exchange 信任路径使用。

本任务是 beta 2.2 内部 breaking fix，不读取旧的 `control-panel@system` token，也不为缺少 `target_kind` 的旧用户 token 增加 fallback。

cache key 必须区分两种用途：

- session/refresh cache key 包含 canonical target key，避免 App/System 或不同 AppInstance 碰撞。
- LoginAssertion replay key 只由可信 issuer、subject、JTI/nonce 等 assertion 身份构成，**不得包含 target**；同一个 assertion 换一个 target 再提交也必须被判定为 replay。

### 2.4 登录授权与目标身份是两回事

能够解析成 `SystemServiceId` 不等于允许交互式用户登录。实现必须保留独立的授权判断：

- `redirect_url` 目标必须是当前 Zone 内由 Gateway 实际路由的目标。
- System target 必须是允许用户登录的系统服务；整理并复用/替换当前 `is_system_login_target`，不要仅依赖字符串语法。
- Desktop 根域 `_ -> control-panel` 必须允许。
- 是否允许 `kmsg` 等其它 SystemServiceId 作为直接用户登录目标，应由明确 allowlist/策略决定，不因它能通过 `SystemServiceId::parse` 自动允许。
- App target 继续调用 `AppAvailabilityResolver`，并精确校验用户对 AppInstance 的 availability。

---

## 3. 协议与接口修改

### 3.1 Verify Hub 请求

修改 `src/kernel/buckyos-api/src/verify_hub_client.rs`：

- [ ] `LoginByPasswordRequest` 使用结构化 `target: AuthTarget`，删除强制的 `appid + app_instance_id` 双字符串输入。
- [ ] `VerifyHubClient::login_by_password` 和 `VerifyHubApiHandler::handle_login_by_password` 接收 `AuthTarget`。
- [ ] `LoginByJwtRequest.login_params` 改成有 `deny_unknown_fields` 的 typed params，并使用结构化 target；不再保留可覆盖 `type`/`jwt` 等字段的任意 `Value` 合并路径，也不能默认拼出 `@system`。
- [ ] `SudoByPasswordRequest`、`sudo_by_password` 同步改为 `AuthTarget`；当前 `control-panel@system` sudo 调用必须迁移为 System target。
- [ ] `VerifyTokenRequest` 改用 `expected_target: Option<AuthTarget>`；传入时必须精确比较 kind/id，App 调用方必须提供 AppInstanceId，不保留 AppId-only 的弱校验模式。
- [ ] serde 使用 `deny_unknown_fields`，补齐 JSON round-trip 和非法组合测试。
- [ ] 所有 Verify Hub token response 都写入 `token_use`，并保证 refresh/session target 完全一致。

不要只修改 Control Panel 到 Verify Hub 的某一个调用点，否则 refresh、sudo 或直接 Verify Hub 调用仍会保留错误模型。

### 3.2 SSO URL 边界

浏览器 `/login` URL 可以继续携带 `redirect_url`。服务器必须以经过校验的 redirect/gateway route 为目标真相，不信任前端传入的裸 `appid`。

`redirect_url` 的 canonical origin 规则必须冻结并 fail closed：

- 正式环境只允许 `https`；如本地/DV 必须支持 `http`，只能通过显式 dev 配置开放，不能自动降级。
- host 必须是当前 Zone 根域或 Gateway `app_info` 中实际存在的 Zone 内 host。
- 只允许默认端口或当前 Gateway 明确监听/暴露的端口；Cookie 不区分端口，不能接受任意 `:<port>`。
- URL 不允许 username/password；path、query、fragment 可以保留用于登录后恢复，但 canonical origin 必须单独保存和比较。
- 登录时保存的 canonical origin 与 callback 实际 request origin、callback `redirect_url` origin 必须完全一致；这不是可选加固。
- AppInstance 可以有多个合法 shortcut origin，但一次 pending login 只能绑定签发时选择的那个 origin，不能仅因两个 origin 解析到同一 AppInstance 就互换。
- `System(control-panel)` 的交互式 token delivery origin 固定为 Zone 根域 `_`；`sys`/`www` 可以继续承载登录 UI，但不能成为 Desktop control-panel System token 的替代 Cookie origin。

本任务不强制顺带重命名现有 `appid`/`client_id` query 参数；若实现中要统一命名，必须同步：

- Desktop 登录 URL
- `LoginPage.tsx`
- WebSDK `AuthClient.login()`
- `doc/sdk/SSO.md`
- `doc/sdk/runtime-login.md`

不得出现前端参数说是 `control-panel`、服务器却根据 redirect 签发给另一个 target 且无显式校验的情况。

---

## 4. Control Panel 修改

主要文件：`src/frame/control_panel/src/sys_auth_backend.rs`。

### 4.1 合并 SSO target resolver

当前 `resolve_sso_target_appid` 与 `resolve_sso_target_app_instance_id` 分开解析同一个 Gateway entry，容易产生分支不一致。替换为一个 resolver：

```text
resolve_sso_auth_target(redirect_url, zone_host) -> ResolvedSsoAuthTarget {
    auth_target,
    canonical_origin,
    canonical_redirect_url,
}
```

解析规则：

1. 按 3.2 的 canonical origin 规则严格解析 URL，拒绝非法 scheme/port、credentials、无 host、Zone 外 host和非法 host。
2. Zone 根域映射为 Gateway `app_info["_"]`。
3. App 子域映射到对应 Gateway `app_info[app_key]`。
4. entry 有合法 `app_instance_id`：返回 `AuthTarget::App`。
5. entry 无 `app_instance_id`、有合法 `service_id`：返回 `AuthTarget::System`。
6. 两者都存在、两者都不存在或字段非法：拒绝歧义/损坏配置。
7. App entry 中的 `app_id`、`app_instance_id`、`app_owner_user_id` 必须同时存在，且 AppId/owner 与 AppInstanceId 一致。
8. System entry 的 `service_id` 必须通过 `SystemServiceId::parse` 和交互式登录策略。
9. `System(control-panel)` 作为 token delivery target 时只接受 Zone 根域 `_`；不能因 `sys`/`www` 也路由到 control-panel 就把 token 写到这些替代 origin。

不要再让 `_` 进入 AppInstanceId parser。

### 4.2 `auth.login`

- [ ] 有 `redirect_url` 时，通过统一 resolver 得到 target。
- [ ] 无 `redirect_url` 的直接登录必须显式提交结构化 target，不能默认 `control-panel@system`。
- [ ] 如果暂时保留浏览器传入的 `appid`，只把它当一致性校验，不把它当权威 target。
- [ ] 调用 Verify Hub 时传递结构化 target。
- [ ] 日志分别输出 `principal_kind`、`target_kind`、canonical target id，不再记录伪造 instance。

### 4.3 Pending SSO 与 callback

扩展 `PendingSsoLogin`，至少保存：

```text
session_token
refresh_token
auth_target
redirect origin 或经过规范化的 redirect_url
created_at
```

- [ ] `/sso_callback` 重新解析 callback 的 `redirect_url` 后，必须与 pending target 一致。
- [ ] callback 实际 request origin、callback `redirect_url` canonical origin、pending canonical origin 必须三者一致；同 target 的不同 shortcut origin 也不能互换。
- [ ] App token 只能写到对应 App origin；System `control-panel` token 写到 Zone 根域。
- [ ] target/origin 不匹配时拒绝并消费 pending nonce；尽力用 pending refresh token 调用 logout/revoke，不能继续写 Cookie 或遗留可刷新的孤儿 session。
- [ ] `/sso_refresh` 在 rotation 前解析并验证 refresh token 的 `token_use + AuthTarget`，再把当前 HTTP request origin/Host 解析成 Gateway AuthTarget，kind/id 不一致时拒绝并清 Cookie。
- [ ] `/sso_refresh` 保持原 target，不能在刷新时把 System token 转成 App token，反之亦然；也不能把 App A 的 token 从已改指 App B 的 shortcut host 返回给 App B。
- [ ] `/sso_logout` 仍可在 session 过期时使用 refresh Cookie，并在本地清除两枚 Cookie；服务端 revoke 失败要记录但不能阻止清 Cookie。

### 4.4 Control Panel 自身鉴权

当前 `is_control_panel_session` 检查 `app_instance_id == "control-panel@system"`，必须删除。

新的条件应至少包括：

```text
principal_kind == user
target_kind == system
token_use == session
appid == CONTROL_PANEL_SERVICE_UNIQUE_ID
不存在 app_instance_id/app_owner_user_id claim
```

- [ ] 使用 `CONTROL_PANEL_SERVICE_UNIQUE_ID`，删除本文件重复硬编码的 `CONTROL_PANEL_AUTH_APPID`，或使其直接引用权威常量。
- [ ] `RpcAuthPrincipal.is_user_session` 仍为 true。
- [ ] `RpcAuthPrincipal.is_control_panel_session` 对合法 System target 为 true。
- [ ] App token 即使 `appid` 文本碰巧类似 `control-panel`，也不能被判定为 Control Panel system session。

### 4.5 Boot Gateway 鉴权与路由边界

主要文件：`src/rootfs/etc/boot_gateway.yaml`。它直接执行私有 App 页面 Cookie 鉴权，是本任务的认证消费者，不能只修改 Control Panel/Verify Hub。

`get_app_info_from_req` 必须与 Control Panel resolver 使用相同的 entry invariants：

- [ ] App entry 必须有合法的 `app_id + app_instance_id + app_owner_user_id`，且不得带 `service_id`。
- [ ] System entry 必须有合法 `service_id`，且不得带 App 身份字段。
- [ ] 两类字段同时存在、必填字段缺失或字段为空时 fail closed，不再仅靠“是否有 service_id”宽松分类。

私有 App 的 `check_oauth` 在 `verify-jwt` 成功后必须继续检查：

```text
iss == verify-hub
principal_kind == user
token_use == session
sudo == false
target_kind == app
appid == TARGET_APP_INFO.app_id
app_instance_id == TARGET_APP_INFO.app_instance_id
app_owner_user_id == TARGET_APP_INFO.app_owner_user_id
```

- [ ] 缺少/未知 target claim、非 Verify Hub issuer、System target、不同 Owner 的同 AppId token 全部拒绝。
- [ ] refresh token 和 sudo token 不能作为 `buckyos_session_token` 放行 App 页面。
- [ ] `/sso_callback`、`/sso_refresh`、`/sso_logout` 保持无需现有 session 即可转发到 Control Panel；不能在 Gateway 提前加 session gate，真正的 origin/target 校验由 4.3 完成。
- [ ] `forward_to_app` 的 group id 改为包含 `app_instance_id`，避免不同 Owner 实例在 failure-state/日志中共用 `app:<app_id>` 名称。
- [ ] `service_info["system_config"]` 和 `/kapi/system_config` 是 Gateway 内部 routing service name，不是 SSO `SystemServiceId`；本任务不要机械重命名为 `system-config`。

scheduler 当前生成的 App entry 已包含 `app_id/app_instance_id/app_owner_user_id`，System entry 已包含 `service_id`，因此本任务不需要新增 Gateway schema；需要同步旧 example、debug fixture 和重复的 Gateway 测试配置。

---

## 5. Verify Hub 修改

主要文件：`src/kernel/verify_hub/src/main.rs`。

### 5.1 替换 App-only scope

当前 `AppTokenScope` 只接受 AppInstanceId，应替换为能表达两类 target 的结构，或直接复用共享 `AuthTarget`。

- [ ] App target：执行 AppId 一致性校验和 `AppAvailabilityResolver::check_user`。
- [ ] System target：验证 SystemServiceId 和 user-login allowlist，不调用 AppAvailabilityResolver。
- [ ] cache/session key 使用带 kind 的 canonical key。
- [ ] LoginAssertion replay key 不包含 target；同一 issuer/subject/JTI assertion 换 target 重放也必须拒绝。
- [ ] session/refresh token 通过统一 helper 绑定 target claims。

### 5.2 Password/JWT/Sudo

- [ ] password login 对 App/System target 都签发 `principal_kind=user` token。
- [ ] JWT user login 使用同一 target 校验流程。
- [ ] sudo 的主体仍是 user，目标可以是 SystemServiceId；`aud` 继续表示 sudo 授权范围，不替代 target。
- [ ] Root 用户拒绝逻辑保持不变。
- [ ] 系统服务进程自身的 `principal_kind=system` token 流程不应被用户登录改动破坏；Verify Hub exchange 后的 token 同样携带显式 AuthTarget。
- [ ] AppService 进程若获得 App target token，必须从 runtime 的真实 AppInstanceId 构造 target，不能退化到 AppId-only。
- [ ] LoginAssertion 只能用于 exchange，Verify Hub 必须根据可信 issuer/principal 规则校验 assertion 与请求 target 的一致性和授权。

### 5.3 Refresh 与 verify

- [ ] refresh 从 refresh token 严格恢复 AuthTarget。
- [ ] App refresh 重新检查精确 AppInstance availability。
- [ ] System refresh 重新检查用户状态和 System login target 是否仍允许。
- [ ] 生成的新 token pair 保留原 target kind/id。
- [ ] refresh/session 分别写入并验证正确 `token_use`；refresh token 不能通过 session verify。
- [ ] verify 对 App/System 分支分别执行 claim invariants，并拒绝非 Verify Hub LoginAssertion 冒充 session。
- [ ] expected target 校验同时比较 kind 与 id，不能只比较裸 `appid` 字符串。

### 5.4 共享本地验签与 RBAC

主要入口：`src/kernel/buckyos-api/src/runtime.rs`，以及使用它的 Control Panel、task-manager、workflow 和其它系统服务。

- [ ] `BuckyOSRuntime::verify_trusted_session_token` 验签成功后调用共享 `token_use/AuthTarget` validator，返回的 token target 可被调用方以强类型读取。
- [ ] `BuckyOSRuntime::enforce` 不再从裸 `appid`/`aud` 猜授权应用身份；先验证 target，再将其转换成 kind-aware authorization identity。
- [ ] RBAC policy、模板和调用参数至少区分 `app:<app_id>` 与 `system:<service_id>`；App target 仍额外保留精确 AppInstanceId 校验。
- [ ] `RpcAuthPrincipal.authenticated_app_id`、task/workflow `ActorRef`、creator/audit identity 等如继续保存裸字符串，会产生 App/System 碰撞；改成共享强类型或同时保存 kind + canonical id。
- [ ] 搜索所有直接调用 `RPCSessionToken::from_string`、`get_subs()` 或读取 `token.appid` 后做授权的路径，迁移到共享 validator/helper。
- [ ] 不允许仅修 Verify Hub 的 `verify_token` RPC；多数生产服务当前走本地验签，并不会调用该 RPC。

---

## 6. Desktop、WebSDK 与调用方迁移

- [ ] Desktop 根域无 session 时仍跳到 `/login`，登录成功后能在根域写入 host-only session/refresh Cookie。
- [ ] `LoginPage.tsx` 不再假设所有 redirect target 都有 AppInstanceId。
- [ ] 登出后重新登录仍使用 System target `control-panel`。
- [ ] WebSDK 普通 App SSO 保持 AppInstance target；目标来自 Gateway route，不由 SDK 猜 owner。
- [ ] 更新 `test/test_control_panel/test_local_user.ts`：删除 `control-panel@system`，admin/local user 登录和 sudo 使用 System target。
- [ ] 更新 `src/test/test_boot_gatweay` 的 `node_gateway_info` builder 和 JWT fixtures，使 App route/合法 token 携带精确 instance、owner、target kind 和 token use。
- [ ] 检查 `cyfs-gateway/tests/buckyos/boot_gateway.yaml` 这份重复且已落后的测试配置：同步相关鉴权逻辑，或删除副本并让测试引用权威配置，不能继续验证旧的 appid-only 模型。
- [ ] 全仓搜索并迁移所有用于系统服务认证的 `*@system` 字符串；不要误改真正的普通 AppInstance fixture。

---

## 7. 测试要求

### 7.1 `buckyos-api` 单元测试

- [ ] AuthTarget App/System serde round-trip。
- [ ] `control-panel`、`kmsg` 可解析为 SystemServiceId。
- [ ] AppInstanceId 必须带合法 `@owner`。
- [ ] unknown kind、空 service id、非法字符、App/System 字段混用被拒绝。
- [ ] token target bind/parse helper 的合法和非法 claim 组合。
- [ ] `token_use` 缺失/未知、session/refresh 用途混用被拒绝。
- [ ] `token_use` 与 `sudo` 的三种合法组合通过，其它组合被拒绝。
- [ ] Verify Hub session 与非 Verify Hub LoginAssertion 的共享解析边界测试。

### 7.2 Verify Hub 单元测试

- [ ] 用户登录 System `control-panel` 成功，token 没有 AppInstance/owner claim。
- [ ] System 登录 token 的 `principal_kind` 是 user、`target_kind` 是 system。
- [ ] 用户登录普通 App 时仍精确绑定 AppInstanceId。
- [ ] App target 缺 instance、owner 不匹配、appid 不匹配全部拒绝。
- [ ] System target 携带 App claims 时拒绝。
- [ ] 未授权 SystemServiceId 的交互式登录被拒绝。
- [ ] System target refresh 后 target 不变。
- [ ] App target refresh 后 instance 不变且重新检查 availability。
- [ ] verify expected target 的 kind 不一致时拒绝，即使裸字符串相同。
- [ ] verify expected App target 必须精确到 AppInstanceId，不接受 AppId-only。
- [ ] 同一 LoginAssertion 首次 exchange 成功后，改成另一个 AppInstance 或同名 System target 再提交仍按 replay 拒绝。
- [ ] refresh token 作为 session token 验证时拒绝；sudo token 不会进入普通 SSO Cookie 流程。
- [ ] System target sudo 成功且 `aud` 正确。
- [ ] root password/JWT/sudo 拒绝用例继续通过。

### 7.3 Control Panel 单元测试

恢复当前被 `#[cfg(all(test, any()))]` 禁用的 `sys_auth_backend` tests，并补充：

- [ ] Zone 根域 `_` 解析为 `System(control-panel)`。
- [ ] 带 `app_instance_id` 的 App route 解析为精确 App target。
- [ ] App entry 的 `app_id`/owner 与 AppInstanceId 不一致时拒绝。
- [ ] Zone 外 URL、HTTP downgrade、非 Gateway 端口和带 credentials URL 被拒绝。
- [ ] Gateway entry 同时有/同时没有 `service_id`、`app_instance_id` 时拒绝。
- [ ] Pending target 与 callback redirect target 不一致时拒绝。
- [ ] Pending target 相同但 callback canonical origin/shortcut 不一致时拒绝。
- [ ] callback 实际 request origin 与 redirect/pending origin 不一致时拒绝。
- [ ] App A refresh Cookie 从当前已路由到 App B 的 host 请求时拒绝并清 Cookie。
- [ ] `System(control-panel)` 的 `sys`/`www` callback origin 被拒绝，Zone 根域成功。
- [ ] 合法 System control-panel token 被识别为 user/control-panel session。
- [ ] 旧 `control-panel@system` token 被拒绝。
- [ ] Cookie 仍为 host-only，session 与 refresh Cookie 属性不回退。

### 7.4 Boot Gateway debug tests

当前 debug suite 的 15/15 通过仅是修改前基线：fixture JWT 只有 `iss/appid/sub/exp` 等旧 claim，没有覆盖 target kind、token use、AppInstance 或 owner；不能把这个结果当作本任务的完成证据。

扩充 `src/test/test_boot_gatweay`：

- [ ] 合法 Verify Hub user/session/App target token 精确匹配 route instance/owner 时放行。
- [ ] 同 AppId、不同 AppInstanceId/owner 的 token 被拒绝。
- [ ] `target_kind=system` 但裸 `appid` 与 AppId 相同的 token 被拒绝。
- [ ] 缺少 `target_kind`/`token_use` 的旧 token 被拒绝。
- [ ] appid、instance、owner 任一不匹配时拒绝。
- [ ] 非 Verify Hub issuer 即使自己填写合法 target claims 也被拒绝。
- [ ] refresh/sudo token 放入 `buckyos_session_token` 时拒绝。
- [ ] `app_info` entry 字段混用/缺失时拒绝。
- [ ] `/sso_callback`、`/sso_refresh`、`/sso_logout` 在没有 session Cookie 时仍正确转发到 Control Panel。

### 7.5 共享验签与 RBAC tests

- [ ] `BuckyOSRuntime::verify_trusted_session_token` 对 Verify Hub token 强制 target/token-use invariants。
- [ ] task-manager/workflow/Control Panel 等本地 verifier 拒绝缺 target kind 的旧 Verify Hub user token。
- [ ] LoginAssertion 不能直接通过普通 session verifier，但仍可进入明确的 Verify Hub exchange 路径。
- [ ] `App(control-panel@alice)` 不能获得 `System(control-panel)` 的 RBAC 权限，即使两者裸 `appid` 相同。
- [ ] kind-aware ActorRef/creator/audit identity round-trip 后不发生 App/System 碰撞。

### 7.6 DV/E2E

至少验证：

1. 清除浏览器站点数据，在 `https://<zone-host>/` 登录 Desktop。
2. 登录后回到原 Desktop URL，不再出现本 TODO 记录的 Parse Request Error。
3. 刷新页面仍保持登录。
4. 等待/模拟 session token 过期后 `/sso_refresh` 成功。
5. logout 清除两枚 Cookie，再次登录成功。
6. 普通已安装 App 子域 SSO 成功，token 精确绑定该 AppInstanceId。
7. 修改 callback 的 redirect host/target 被拒绝。
8. 把同一 AppId 的另一个 Owner/实例 token 放到当前 App host 时被 Gateway 拒绝。
9. 修改 shortcut route 后，旧 target refresh Cookie 被拒绝并清除。
10. `test/local_user_dv.sh` 通过，且测试中不存在 `control-panel@system`。

---

## 8. 文档联动

- [ ] `doc/sdk/SSO.md`：redirect 可解析到 AppInstance 或 SystemServiceId。
- [ ] `doc/sdk/runtime-login.md`：补充 System target token claims。
- [ ] `doc/control_panel/Control_Panel_Service.md`：更新 `auth.login`、callback、refresh 请求与校验流程。
- [ ] `doc/arch/10_user_lifecycle_and_permissions.md`：把 Gateway 的“appid-only 强制鉴权”更新为精确 AppInstance + target kind/token use 校验。
- [ ] Gateway 架构/配置文档：说明 `app_info` 的 App/System entry invariants、`system_config` routing alias 与 SystemServiceId 的区别。
- [ ] `doc/App 安装协议.md`：把 token 的主体类型与目标类型分开描述，避免“只有 system principal 才能以 SystemServiceId 为 appid”被误读为用户不能登录系统服务。
- [ ] `notepads/app-id-simplification-todo.md`：修正 SDK/Auth 完成状态或补充本回归修复链接。

---

## 9. 推荐实施顺序

### Phase 1：先建立失败测试和共享类型

- [ ] 为根域 `_ -> control-panel` 写失败测试，确认能稳定复现当前错误。
- [ ] 新增 AuthTarget、token use、LoginAssertion/session 边界和 target claim helper。
- [ ] 冻结 kind-aware authorization identity 和 RBAC key 迁移规则。
- [ ] 完成 buckyos-api serde/claim 单测。

### Phase 2：改 Verify Hub

- [ ] 修改 password/JWT/sudo 请求和 handler。
- [ ] 修改 token generation、session cache key、target-independent replay key、refresh、verify。
- [ ] 跑 Verify Hub 全部单测。

### Phase 3：改共享本地验签与 RBAC

- [ ] 修改 `BuckyOSRuntime::verify_trusted_session_token` 和 `enforce`。
- [ ] 迁移 RBAC policy/key、ActorRef、creator/audit identity 及直接读取 token.appid 的授权路径。
- [ ] 跑 buckyos-api、task-manager、workflow 和 RBAC 定向测试。

### Phase 4：改 Control Panel 与 Boot Gateway

- [ ] 合并 target resolver。
- [ ] 修改 login、pending callback、refresh 和 principal 判定。
- [ ] 恢复并扩充 sys_auth_backend 单测。
- [ ] 修改 `boot_gateway.yaml` 的 entry validation、Cookie target 校验和 AppInstance forward group。
- [ ] 更新并运行 boot Gateway debug tests。

### Phase 5：迁移前端、测试和文档

- [ ] 修改 Desktop/LoginPage/WebSDK 受影响调用。
- [ ] 迁移 local-user 和其它 sudo/login 测试。
- [ ] 更新协议与 SDK 文档。

### Phase 6：构建与 DV 验收

- [ ] Rust 定向测试。
- [ ] Web 构建和 BuckyOS 全量构建。
- [ ] 全新启动 DV 环境。
- [ ] 完成 Desktop + App SSO + refresh + logout + local-user DV 矩阵。

按阶段提交时，不允许出现“Phase 2 完成但仓库无法编译”的中间提交；共享 API 改动与调用方迁移应保持每个提交可构建。

---

## 10. 建议验证命令

从 `buckyos/src` 运行：

```bash
cargo test -p buckyos-api
cargo test -p verify_hub
cargo test -p control_panel
cargo test -p task_manager
cargo test -p workflow
uv run buckyos-build.py
```

从 `buckyos` 根目录运行：

```bash
uv run src/test/test_boot_gatweay/run_debug_tests.py
uv run src/check.py
bash test/local_user_dv.sh
```

再进行浏览器 Desktop 登录和普通 App SSO 人工/DV 验收。

最终执行搜索 gate，确认系统服务伪 AppInstance 已清除：

```bash
grep -RIn --exclude-dir=.git --exclude-dir=target --exclude-dir=node_modules \
  'control-panel@system\|kmsg@system' src test doc
```

预期没有活跃协议、实现或测试命中；如历史说明必须保留文字，应明确标为“禁止的旧格式”。

---

## 11. Definition of Done

- [ ] Desktop 在全新 DV 环境能从 Zone 根域登录。
- [ ] `_` 被解析为 `SystemServiceId("control-panel")`，而不是 AppInstanceId。
- [ ] 用户 System token 的 principal/target 两个维度正确且可独立校验。
- [ ] 普通 App token 仍精确绑定 AppInstanceId 和 owner。
- [ ] login、JWT login、sudo、refresh、verify、callback、Gateway 和共享本地验签使用同一 AuthTarget 模型。
- [ ] Verify Hub token 明确区分 session/refresh，LoginAssertion 不能直接作为 session 使用。
- [ ] RBAC 和 creator/audit identity 区分同名 App/System target，不再只依赖裸 `appid`。
- [ ] 不存在 `control-panel@system` 运行时兼容路径。
- [ ] callback 不能把 pending token 改送到不同 target 或同 target 的不同 origin。
- [ ] refresh request origin 当前 Gateway target 与 token target 不一致时拒绝并清 Cookie。
- [ ] Boot Gateway 私有 App 页面精确校验 Verify Hub user/session/AppInstance/owner claims。
- [ ] Control Panel 被禁用的 auth tests 恢复执行。
- [ ] 定向 Rust tests、Boot Gateway debug tests、BuckyOS build、local-user DV、Desktop/App SSO 验收通过。
- [ ] 协议、SDK、Control Panel、Gateway 和 RBAC 文档已同步。

---

## 12. CodeAgent 完成后必须报告

- 修改了哪些共享类型、wire fields 和 token claims。
- LoginAssertion、session、refresh、sudo 的边界和 `token_use` 如何验证。
- System target 与 App target 分别如何签发、刷新和校验。
- 为什么 `principal_kind=user + target_kind=system` 是正确组合。
- Gateway、共享本地 verifier 和 RBAC 如何保留 target kind 与精确 AppInstance。
- callback/refresh 如何绑定 canonical origin 和当前 Gateway route。
- 是否删除了所有 `control-panel@system` 活跃路径。
- 跑过的测试、构建和 DV 用例及结果。
- 未验证的浏览器、Cookie、Gateway 或多用户风险。
