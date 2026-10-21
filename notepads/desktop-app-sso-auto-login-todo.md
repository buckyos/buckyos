# 桌面打开 App 自动登录 TODO

日期：2026-10-21

状态：已实施（2026-10-21），单测、Desktop e2e 与 test.buckyos.io 真实 Zone DV 通过，改动未提交。实施记录见文末。（同日先按 review 修订：结果状态枚举、delegate 只允许 Control Panel 调用、refresh 宽限期、已知取舍。）

## 目标与范围

用户登录 BuckyOS 桌面后，从桌面打开有权访问的 Web App，应直接进入该 App，不再重复输入密码。桌面先携带自己的用户会话申请一次性启动票据，再让 iframe 加载由 Control Panel 处理、位于目标 App 域名的 SSO callback；callback 写入目标 App 的独立 Cookie 后跳转到真正页面。

第一版限定为同一 Zone 内、Gateway 已登记的 App 实例。来源是已认证的 `System(control-panel)` 用户会话，目标是明确的 `AuthTarget::App`。用户仍受 App availability 和后续业务 RBAC 约束。

不新增 `desktop` AppId，不给普通 App 传递桌面 token，不共享父域 Cookie，不修改网关为接受跨 target token。跨 Zone 登录、桌面退出时全局注销所有 App、第三方应用自有账号系统适配不属于本任务。

## 当前实现与复用入口

以实施时的源码为准，先核对以下入口，保留已有无关工作树修改。

| 环节 | 当前实现与入口 |
| --- | --- |
| 桌面身份 | [main.tsx](../src/frame/desktop/src/main.tsx) 调用 `initBuckyOS('control-panel')`；[loginTarget.ts](../src/frame/desktop/src/auth/loginTarget.ts) 使用 `System(control-panel)`。没有独立 desktop 身份。 |
| 内置 App | [registry.tsx](../src/frame/desktop/src/app/registry.tsx) 中的系统面板在桌面前端 runtime 内运行，共享桌面会话。 |
| 安装的 Web App | [WebAppFramePanel.tsx](../src/frame/desktop/src/app/web-frame/WebAppFramePanel.tsx) 直接加载 App origin；[SystestAppPanel.tsx](../src/frame/desktop/src/app/systest/SystestAppPanel.tsx) 有独立 iframe 入口，两者都没有启动换票。 |
| 桌面 RPC | [rpc.ts](../src/frame/desktop/src/api/rpc.ts) 已封装携带当前 session 的 Control Panel client。 |
| SSO 服务端 | [sys_auth_backend.rs](../src/frame/control_panel/src/sys_auth_backend.rs) 已实现 URL 到 AuthTarget 的解析、`PendingSsoLogin`、60 秒 nonce、一次性 callback、host-only Cookie、refresh 和 logout。 |
| Token 签发 | [verify_hub/main.rs](../src/kernel/verify_hub/src/main.rs) 已有用户状态检查、`resolve_user_auth_target` 和 `generate_token_pair`；`login_by_jwt` 拒绝 verify-hub session，refresh 保持原 target，目前没有跨 target 会话换票。 |
| 共享协议 | [verify_hub_client.rs](../src/kernel/buckyos-api/src/verify_hub_client.rs) 包含 request、client、handler trait 和 RPC adapter；[app_identity.rs](../src/kernel/buckyos-api/src/app_identity.rs) 定义 AuthTarget。 |
| App 可用性 | [app_availability.rs](../src/kernel/buckyos-api/src/app_availability.rs) 的 `check_user` 检查用户、安装实例、owner 状态和 availability policy。 |
| Gateway | [boot_gateway.yaml](../src/rootfs/etc/boot_gateway.yaml) 已将 `/sso_callback`、`/sso_refresh`、`/sso_logout` 转发给 Control Panel；App 鉴权精确匹配 target kind、appid、instance 和 owner。 |
| Web SDK | 同级 [buckyos-websdk/runtime.ts](../../buckyos-websdk/src/runtime.ts) 已通过本域 `/sso_refresh` 恢复页面会话；原则上沿用现有能力。 |

注意：`auth.issue_sso_token` 出现在 Control Panel 文档和 `is_public_rpc_method` 白名单中，但当前没有 handler，不能当作已有实现。本任务使用下文明确的受保护方法，不实现匿名签发。

以下现状决定了下文的接口形状：

- kRPC 错误在线上只是字符串（`RPCResult::Failed(String)`）。Rust client 收到后统一转成 `ReasonError("rpc call error: …")`，websdk 只抛带 message 的 `RPCError`，跨一次 RPC 枚举就丢了。调用方需要分支处理的结果因此放在成功响应的 `status` 字段里，不靠错误文本区分。
- `/kapi/verify-hub` 经网关对浏览器可达，桌面 [sudo.tsx](../src/frame/desktop/src/components/sudo.tsx) 就是直接调用；`VerifyHubRpcHandler` 目前不检查调用方。
- websdk 每次页面加载都调用 `/sso_refresh`，去重只在单个页面 runtime 内（`browserRefreshPromise`）。同一 host 的多个窗口共用 refresh Cookie，会并发续期；桌面打开内容时会为同一 App 新建多个窗口（`openAppWindow` 的 `newInstance`）。

已有 [SystemServiceId 登录与 SSO 目标修复 TODO](fix-system-service-sso-auth-target-todo.md) 提供背景；本任务复用已经落地的 AuthTarget 模型，不重复引入模型或旧格式兼容分支。

## 确定的接口和 HTTP 流程

浏览器入口采用现有 `POST /kapi/control-panel`，新增 RPC 方法 `auth.prepare_app_launch`。不再并行新增 `/sso_launch` HTTP 端点，也不实现仅凭导航 Cookie 自动签发的 GET 启动入口。

### 1 桌面申请一次性启动票据

请求发往桌面当前同源入口，不假定桌面会话 Cookie 一定位于 `sys.<zone>`。桌面通过现有 SDK client 携带当前用户 session；不得在 URL 中传入 session 或 refresh token。

以下使用现有 kRPC wire format，`sys[0]` 为 seq，`sys[1]` 为 token：

```http
POST /kapi/control-panel HTTP/1.1
Host: zone.example
Content-Type: application/json

{
  "method": "auth.prepare_app_launch",
  "params": {
    "app_instance_id": "notes@alice",
    "redirect_url": "https://notes.zone.example/doc/123#bfp=frame-nonce"
  },
  "sys": [1, "<desktop-session-token>"]
}
```

参数不接受 `username`、自报 `source_appid` 或用于替代来源身份的 `target`。`app_instance_id` 表示桌面期望打开的实例；服务端必须从 Gateway route 解析真实 AuthTarget，并核对两者完全一致。

Control Panel 校验来源，调用 Verify Hub 换票，把目标 token pair 保存到 pending 记录中。响应只返回一次性 callback URL，不返回 token pair：

```http
HTTP/1.1 200 OK
Content-Type: application/json
Cache-Control: no-store

{
  "result": {
    "status": "ready",
    "callback_url": "https://notes.zone.example/sso_callback?nonce=123&redirect_url=https%3A%2F%2Fnotes.zone.example%2Fdoc%2F123%23bfp%3Dframe-nonce"
  },
  "sys": [1]
}
```

示例 nonce 仅表示字段位置。实际复用服务端 OS 随机生成、JS 可安全表示的 nonce 和 60 秒 TTL；不得使用示例常量或时间戳代替随机票据。

预期内的失败同样以成功 envelope 返回，由 `result.status` 区分。桌面只按这个枚举分支，不解析错误文本：

| `status` | 含义 | 桌面处理 |
| --- | --- | --- |
| `ready` | 附带 `callback_url` | 导航 iframe |
| `source_session_inactive` | 桌面会话认证失败，或来源 family 已注销、不存在（例如 verify-hub 重启后内存会话清空） | 用 SDK 现有 refresh 能力强制刷新一次；成功则重试一次 prepare，仍是该状态就显示错误和重试；refresh 失败才进入桌面已有登录流程 |
| `access_denied` | availability 拒绝：用户无权、实例不存在或已删除、owner 不可用 | 显示访问错误，不自动重试 |
| `invalid_target` | URL 或 Gateway route 不合法、不在本 Zone、不是 App route，或与 `app_instance_id` 不一致 | 显示无法打开；内容打开沿用现有回退到 Preview 的处理 |

kRPC error envelope、HTTP 非 200 和网络错误一律视为暂时性失败，显示重试，不跳登录页。

### 2 iframe 导航到目标 App 的 callback

桌面取得响应后执行等价于 `iframe.src = callback_url` 的导航。POST 返回 JSON，不能依赖 fetch 自动跟随 302 来代替 iframe 导航。

```http
GET /sso_callback?nonce=123&redirect_url=... HTTP/1.1
Host: notes.zone.example
```

Gateway 把该路径转发给 Control Panel。此请求不需要预先具有目标 App Cookie；继续沿用 SSO 特殊路径在 App 页面鉴权之前分流的现有行为。

### 3 callback 写入 App Cookie 并跳转

Control Panel 原子消费 pending nonce，重新核对 pending target、当前 Gateway route、实际 request origin 和完整 canonical redirect，然后返回：

```http
HTTP/1.1 302 Found
Location: https://notes.zone.example/doc/123#bfp=frame-nonce
Set-Cookie: buckyos_session_token=<app-session>; Path=/; Secure; SameSite=Lax
Set-Cookie: buckyos_refresh_token=<app-refresh>; Path=/; Secure; HttpOnly; SameSite=Lax
Cache-Control: no-store
```

示例省略随 token 有效期设置的 `Max-Age`，实际复用现有 cookie builder。两枚 Cookie 均无 `Domain`，因此只属于目标 App host；HTTPS 使用 `Secure`，DV HTTP 继续受现有显式配置控制。

目标 URL 的 path、query、hash 必须完整保存。`#bfp` 本身不会出现在最终页面的 HTTP 请求中，但会经编码后的 `redirect_url` 参数进入 pending，并通过 `Location` 恢复，供 AppFrame 握手使用。

### 4 App 沿用现有恢复流程

浏览器访问真正的 App 页面时自动携带 App session Cookie。SDK 随后调用该 App origin 的 `POST /sso_refresh`，得到自己的 session token 和 user_info。网关、refresh 和业务 API 继续按该 App 的 target 校验。

例如 Alice 打开获准访问的 `notes@bob`，新 token 的 `sub` 仍为 Alice，`app_owner_user_id` 为 Bob；不能将操作者改为 App owner，也不能继承桌面的 system target 或 sudo 权限。

## 已知取舍与风险

第一版有意接受以下几点，实施时不增加探测、联动撤销或隔离机制，并在交付文档中写明：

- 委托出的 App family 与来源会话无关，桌面 logout 不影响它，refresh 轮换后可以一直续期。所以只要泄露一枚 15 分钟有效的桌面 session，就能换出长期有效的 App 会话。
- 每次启动都签一条新 family 并覆盖目标 host 的 Cookie，不探测该 host 上是否已有会话。被覆盖的旧 family 和过期未兑换的 pending 一样，留在 verify-hub 内存里，直到 refresh 过期（7 天）由 gc 清理。
- refresh 宽限期（B 节）内，出示上一枚 refresh 的请求能拿到当前 token。被窃取的旧 refresh 只在合法轮换后的几秒内有用。

## 实施任务

### A Verify Hub 和共享协议

- [x] 在 `buckyos-api/src/verify_hub_client.rs` 新增 `delegate_user_session` 的 request、client 方法、handler trait 方法和 RPC adapter，参数为 `source_session_token` 与结构化 `target`。结果带 `status`：`issued`（附现有 `TokenPair` 的两个字段）、`source_inactive`、`access_denied`；其它失败仍走 RPC error。adapter 把请求 `sys` 中的调用方 token（`req.token`）一并交给 handler。同步更新相关 trait 测试实现（`MockVerifyHub`）。
- [x] Verify Hub 只接受 Control Panel 调用该方法。调用方 token 必须通过 verify-hub 签名、issuer、有效期和 claims 校验，且满足 `token_use=session`、`principal_kind` 为 System 或 Device（沿用 task-manager、nfs-server 对服务主体的判定）、target 为 `System(control-panel)`、`sudo=false`。用户 session（包括桌面 token 本身）、其他服务、未换票的设备签名 LoginAssertion 一律以 `NoPermission` 拒绝。Control Panel 是 KernelService，`get_verify_hub_client()` 走 `get_zone_service_krpc_client`，会带上 runtime 的服务 session；启动早期 runtime token 尚未换成 verify-hub session 时调用会失败，按暂时性失败处理。
- [x] 在 Verify Hub 实现该方法。来源必须通过本 Zone verify-hub 签名、issuer、有效期及完整 claims 校验，并满足 `principal_kind=user`、`token_use=session`、`AuthTarget::System(control-panel)`、`sudo=false`。签名或有效期失败、用户 inactive、来源 family 不活跃时返回 `source_inactive`；来源种类不符属于调用方错误，返回 `InvalidToken`。
- [x] 从已验证来源 token 取 `sub`；复用用户 active 检查并拒绝 root 等现有禁止的主体。来源身份不能由 Control Panel service token、设备 assertion、参数中的 appid 或 Referer 替代。
- [x] 第一版仅接受 App target，调用 `resolve_user_auth_target` 复用 availability 判定，拒绝时返回 `access_denied`。普通 App session、同名 `App(control-panel@user)`、system/device/app principal、refresh 和 sudo token 均不能作为来源；不得给此接口开放任意 System target。
- [x] 拆开 availability 的拒绝和暂时性失败。`AppAvailabilityResolver` 目前把 system-config 的 `KeyNotFound` 和读失败都转成 `ReasonError`，`resolve_user_auth_target` 又把 `check_user` 的任何错误都映射成 `NoPermission("AppAccessDenied")`，暂时性失败因此会冒充无权访问。按枚举拆分：用户或实例 spec 不存在、实例已删除映射为 `KeyNotExist`（保留现有消息文本），用户 inactive 保持 `NoPermission`。`resolve_user_auth_target` 只把这两类和 `decision.allowed=false` 报为 `AppAccessDenied`，其它错误原样返回。密码登录、sudo、refresh 共用这条路径，拒绝语义不变，只是暂时性失败不再冒充拒绝。
- [x] 生成独立随机 session_id，并通过 `generate_token_pair` 签发同一用户、目标实例的普通 User token pair；不消耗或轮换桌面 refresh。复用现有按 session_key 存放的 `TOKEN_CACHE`、`REFRESH_TOKEN_CACHE` 和 refresh 生命周期（verify-hub 没有专门的 App token 缓存）。

### B 来源会话存活与并发

当前 `verify_token` 的验签与 claims 校验不能证明来源尚未注销；logout 删除会话缓存后，旧 JWT 仍可能未到 exp。新委托入口必须额外检查活跃会话。

- [x] 用 `(sub, target.canonical_key(), session_id)` 查来源会话，复用现有 `REFRESH_TOKEN_CACHE` 中未过期的 family 记录，不新增会话数据库。
- [x] 不把来源 access token 的 jti 与最新 access token jti 做相等判断：正常 refresh 保留 session_id、替换 token，旧但未过期的 access token 仍属于同一活跃会话。
- [x] 收敛 refresh/cache 临界区。现有 refresh 先读取校验，在锁外查询用户和 availability，再删除旧 refresh、签发、插入；中间会出现暂时性“不活跃”，logout 之后还可能重新插入而复活会话。`REFRESH_TOKEN_CACHE` 本身就是一把全局 Mutex，不新增锁结构：网络查询放在锁外，签发后在一次加锁内以旧 refresh jti 做 compare-and-replace，记录已删除则拒绝提交。保留 `validate_active_refresh_token` 的重用检测和整条 family 撤销语义；CAS 发现旧 refresh 已被消费时不能静默降级为只拒绝这一次请求，宽限期是唯一例外（见下条）。
- [x] 增加 refresh 宽限期，解决同一 host 多窗口并发续期（见“当前实现”的说明）。family 记录额外保存上一枚 refresh 的 jti 和轮换时间。如果出示的正是刚被轮换掉的上一枚，且在宽限期内（秒级常量，例如 10 秒），就返回该 family 当前的那对 token，不再签发也不撤销；当前 session 可以取自同一 session_key 的 `TOKEN_CACHE`，也可以随 family 记录保存。更旧的 jti 或超出宽限期，仍按重用撤销整条 family。CAS 失败时同样适用这条规则；logout 删除记录后不能经宽限复活。不在 websdk 加跨窗口锁：Web Locks 只在安全上下文可用，DV 配置默认 `force_https: false`，用 HTTP 访问时没有这个 API。
- [x] 在同一次 `REFRESH_TOKEN_CACHE` 加锁内确认来源 family 仍然存在，并插入新的目标 family，保证 logout 完成后不能再提交新委托。网络查询、用户和 availability 查询放在锁外，避免长时间持锁。
- [x] 明确第一版注销语义：桌面 logout 禁止后续新委托；此前已签发的 callback 票据仍按 60 秒 TTL 消费，App 独立会话按自身 logout/有效期结束。全局注销及联动撤销未消费票据不在本任务实现，相关风险见“已知取舍与风险”。

### C Control Panel 启动入口

- [x] 在 `main.rs` RPC dispatch 和 `sys_auth_backend.rs` 增加 `auth.prepare_app_launch`。它必须经过正常认证，且显式要求普通 Control Panel 用户 session，不能加入 `is_public_rpc_method`。dispatch 的通用认证对本方法返回 `InvalidToken`（缺少 token、签名或有效期失败、用户 inactive）时，在 Control Panel 进程内按枚举映射为 `source_session_inactive` 结果，而不是 error envelope；现有认证把 user settings 读失败也报为 `InvalidToken`，桌面“刷新一次再重试”的处理能覆盖这种情况。其它认证错误仍走 error envelope。通过认证但不是 `is_control_panel_session`，或带 sudo 的 token，直接以 `NoPermission` 拒绝。
- [x] 复用 `resolve_sso_auth_target`：限制当前 Zone、实际 Gateway route、合法 scheme/port、App 身份三字段一致；与请求 `app_instance_id` 精确匹配。不能只凭 URL 后缀或 AppId 推测 owner。`ParseRequestError`、`NoPermission` 和实例不一致返回 `invalid_target`；读 gateway info 失败等 `ReasonError` 走 error envelope。
- [x] 调用新增 Verify Hub client，明确传入原始桌面用户 token 作为 source proof。Control Panel 自身 RPC transport 上的 service token 只用于让 verify-hub 认定调用方（见 A 节），不等于这个用户证明，两者缺一不可。`source_inactive`、`access_denied` 分别映射为 `source_session_inactive`、`access_denied`。
- [x] 校验返回 token pair 的 target、User principal、普通 session/refresh 用途、同一 sub 和有效期，再复用 `PendingSsoLogin`。提取密码登录和启动登录共用的小函数，避免复制签发后存票/拼 callback 的整段逻辑。
- [x] 服务端根据 canonical target origin 构造 `callback_url`，完整编码 canonical redirect。新方法响应和 callback 响应设 `Cache-Control: no-store`。票据、token 和含 nonce 的完整 URL 不写入普通日志：现有 `handle_auth_login`、`serve_sso_callback`、`store_pending_sso_login`、`take_pending_sso_login` 和 `summarize_pending_sso_logins` 都打印 nonce 明文，一并改掉，密码登录的日志也会随之变化。
- [x] nonce 一次性消费和 route/origin/redirect 绑定保持不变。callback 校验失败时沿用现有 `revoke_pending_sso_login` 撤销。过期未兑换的 pending 不额外撤销，和被覆盖的旧 family 一样留到 refresh 过期（见“已知取舍与风险”），不改动 `retain` 逻辑。
- [x] 清理 `auth.issue_sso_token` 这个无实现的公开占位及相关文档描述，不增加别名或兼容接口；其它历史 auth 占位问题不扩大处理。

pending 当前是 Control Panel 进程内状态，来源 family 的 refresh 缓存也在 Verify Hub 进程内。沿用现有部署前提；若实施环境有多实例路由，必须验证申请与 callback 能命中同一 pending 存储，delegate、refresh、logout 也能访问同一会话状态，不能默认多实例随机转发可用。本任务不预设新增共享存储。

### D Desktop 启动链路

- [x] 在桌面现有 API/启动模块内封装申请启动票据的逻辑，复用带 session 的 `callRpc`，不新增通用认证框架或依赖。
- [x] 接入 `WebAppFramePanel` 与 `SystestAppPanel`；覆盖图标启动、内容打开及已有窗口的重新导航。已打开窗口仅聚焦、主题/语言更新时不重新签发或重载。
- [x] 先构造最终 App URL，再申请票据。保留内容打开的 path/query/hash，区分 AppFrame 的 `bfp` nonce 与服务端 SSO nonce；AppFrameHost 仍以最终 App origin 校验消息。
- [x] 处理 React StrictMode、重复点击、组件卸载及请求乱序，避免重复 effect 不断创建会话，或旧响应覆盖新的启动目标。只在票据准备成功后挂载/导航目标 iframe，准备中展示加载状态。
- [x] 按上文 `status` 表处理结果，只按枚举分支。`source_session_inactive` 最多刷新一次、重试一次，只有 refresh 失败才进入桌面已有登录流程；不得全部降级成密码页或形成登录循环。
- [x] 已接入 SSO 的 App 继续通过本域 `/sso_refresh` 登录；先验证 SDK 现有能力，只有实际发现缺口才修改同级 websdk 源码和相应测试，不编辑 dist/rootfs 生成副本。多窗口并发续期由 verify-hub 宽限期处理，websdk 不加跨窗口锁。

### E 文档与手册

- [x] 更新 [SSO.md](../doc/sdk/SSO.md) 和 [Control_Panel_Service.md](../doc/control_panel/Control_Panel_Service.md)，说明新方法、来源限制、delegate 只允许 Control Panel 调用、HTTP 顺序、实例绑定、结果状态、退出语义和已知取舍，并删除相关不存在接口的描述。
- [x] refresh 宽限期改变了“旧 refresh 立即失效”的表述，同步更新 [SSO.md](../doc/sdk/SSO.md) 的轮换说明和 [runtime-login.md](../doc/sdk/runtime-login.md) 中关于旧 refresh 再次使用的段落。
- [x] 按仓库规则检查 [buckyos_admin.md](../src/apps/jarvis_agent/agent/cmd_skills/buckyos_admin.md) 是否涉及身份/SSO 排障内容；更新必要部分，或在交付说明中解释无需修改的依据。

## 验收用例

后端单测覆盖认证、授权和生命周期，浏览器真实 Zone 验证覆盖域名与 Cookie 行为。现有 iframe route mock 通过不能代替真实 SSO 验证。

| 场景 | 预期结果 |
| --- | --- |
| 已登录桌面首次打开从未登录的 App | 无密码页，callback 写入目标 host Cookie，App 取得正确用户与实例的 session。 |
| 普通用户打开他人共享 App | availability 允许才成功；sub 是操作者，owner 是安装用户。管理员管理权不能替代 App availability。 |
| 普通 App token、同名 control-panel App、service/device token、refresh、sudo 作为来源 | 拒绝，不能跨 App 换票或获得系统 target。 |
| 用户 session（包括桌面 token）或其他服务直接调用 verify-hub 的 `delegate_user_session` | 拒绝，浏览器拿不到委托出的 token pair。 |
| 来源伪造、过期、用户 inactive 或已注销 | 返回 `source_session_inactive`，不生成可用目标会话。 |
| verify-hub 重启后（内存会话已清空）从桌面打开 App | 返回 `source_session_inactive`；桌面 refresh 失败后进入登录流程，不显示无权访问。 |
| availability 判定时 system-config 读失败 | 按暂时性失败处理并可重试，不报 `access_denied`；无权、实例不存在仍报 `access_denied`。 |
| 桌面 refresh 后，用同一 family 的旧但未过期 access token 换票 | 在来源 family 活跃时允许；不能仅因 jti 已更新而误拒。 |
| refresh 与 logout 并发；委托与 logout 并发 | 不复活来源会话；logout 完成后不能再提交新委托。 |
| 两次并发使用同一 refresh | 只签发一次；宽限期内两个请求拿到同一对 token，family 不被撤销。 |
| 同一 App 连开两个窗口，两个页面同时加载，之后同时续期 | 两个窗口都保持登录，不撤销 family，也不清掉 Cookie。 |
| 两次消费同一 callback nonce | 最多一次成功。 |
| 超出宽限期后再用已轮换的 refresh，或出示更旧的 refresh | 保留现有重用检测：撤销对应 family，已轮换的新 refresh 随后也无法续期。 |
| 同 AppId 不同 owner，或请求实例与 URL route 不匹配 | 返回 `invalid_target`；网关不能接受别的实例 token。 |
| 外 Zone、未知 host、非法端口/scheme/credentials、改写 redirect | 按现有 URL/route 规则返回 `invalid_target`，不泄露 token。 |
| callback 重放、过期或 route 在签发后改变 | 拒绝；失败路径清理未成功交付的会话。 |
| App 有既存登录态后，通过另一桌面用户重新启动 | 新页面身份与本次桌面用户一致，不被缓存 user_info 覆盖；记录同 host 多窗口共享 Cookie 的现有语义。 |
| 内容打开 URL 含 path、query、bfp hash | 文档打开和 AppFrame 握手正常；关闭、主题/语言消息不受影响。 |
| 桌面重复点击、StrictMode、窗口快速关闭或切换 | 不循环换票，不加载过期响应；普通聚焦不刷新登录。 |
| 桌面 logout 后的已有 App 会话/此前票据 | 保持上文明确的独立生命周期，不宣称全局退出。 |
| 普通密码登录、sudo、现有 refresh/logout | 行为不回归，既有 target 校验仍生效。 |

## 验证和交付

- [x] 在 `buckyos/src/` 执行相关 Rust 检查，至少覆盖 `cargo test -p buckyos-api -p verify_hub -p control_panel -- --test-threads=1`；先用相关测试过滤器迭代，再运行受影响 package 的检查。
- [x] 在 `src/frame/desktop/` 执行 `pnpm check`、`pnpm build` 和受影响的 Playwright 用例；更新启动 mock，让它返回带 `status` 的结果并覆盖各个状态，不让 mock 掩盖真实重定向要求。
- [x] （未修改 websdk，不适用）如修改 websdk，按其仓库规则运行相关 session/auth 测试和构建。现有参考测试包括 `tests/browser/session_refresh.unit.test.ts` 和 `tests/browser/auth_client.node.test.ts`。
- [x] 在已激活的真实 Zone 中验证一次“登录桌面 → 首次打开 App → callback 写 Cookie → App refresh 成功”的完整链路，并覆盖权限拒绝、注销后换票失败、内容打开，以及同一 App 连开两个窗口。需要额外 hostname/浏览器部署条件时明确记录。
- [x] 验证受影响模块构建及必要的分发；遵守仓库构建入口，不直接编辑生成产物。不得通过本任务重新安装或清空用户 Zone。
- [x] 交付列出实际修改文件、接口/共享类型/SDK/文档和手册联动、执行的检查及结果、未验证的平台或部署限制；回填本 TODO 完成状态，不把未运行的 DV 标为通过。

完成标准：桌面登录用户可无感打开已授权 App；每个 App 获得自己的实例绑定用户会话；普通 App 和浏览器都无法利用新入口跨 target 换票或直接拿到 token pair；同一 App 多窗口并发续期不掉登录；现有密码登录、Cookie 隔离、refresh、logout、sudo 和 AppFrame 行为保持有效。


## 实施记录（2026-10-21）

### 改动文件

| 范围 | 文件 |
| --- | --- |
| 共享协议 | `src/kernel/buckyos-api/src/verify_hub_client.rs`：`DelegateUserSessionRequest`（`deny_unknown_fields`）、`DelegateUserSessionResult`（`status` 为线上判别字段：`issued` 附 `session_token`/`refresh_token`、`source_inactive`、`access_denied`）、client `delegate_user_session`、handler trait、adapter 传入 `req.token`；`MockVerifyHub` 与 adapter 测试 |
| availability | `src/kernel/buckyos-api/src/app_availability.rs`：用户/实例 spec 不存在、实例已删除 → `KeyNotExist`（消息文本不变），其余读失败 → `ReasonError`；owner settings 只有不存在或 inactive 才算 owner 不可用，暂时性读失败原样返回 |
| Verify Hub | `src/kernel/verify_hub/src/main.rs`：`REFRESH_TOKEN_CACHE` 值改为 `RefreshFamily`（当前 refresh、当前 session、上一枚 jti 与轮换时间）；refresh 先分类出示的 jti，锁外做网络检查，提交时 `commit_refresh_rotation` 以旧 jti 做 compare-and-replace；`REFRESH_GRACE_SECONDS = 10`；logout 删除的记录不会经宽限或在途 refresh 复活；`handle_delegate_user_session`；`resolve_user_auth_target` 只把 `KeyNotExist`/`NoPermission`/`allowed=false` 报为 `AppAccessDenied` |
| Control Panel | `src/frame/control_panel/src/sys_auth_backend.rs`：`auth.prepare_app_launch`、`AppLaunchResult`、`resolve_app_launch_target`、`validate_delegated_token_pair`、`sso_callback_url`、与密码登录共用的 `store_new_pending_sso_login`；callback 响应 `no-store`；日志去掉 nonce 和完整 redirect；删 `auth.issue_sso_token`。`main.rs`：dispatch 对本方法把认证 `InvalidToken` 映射为 `source_session_inactive`；所有 `auth.*` kRPC 响应带 `Cache-Control: no-store` |
| Desktop | `src/api/app_launch.ts`（请求与结果校验）、`src/api/app_launch_mock.ts`（mock 场景）、`src/app/web-frame/launch.ts`（流程与 `useAppLaunch`）、`src/app/web-frame/AppLaunchStatus.tsx`、`WebAppFramePanel.tsx`、`systest/SystestAppPanel.tsx`、`src/auth/loginRedirect.ts`（从 `main.tsx` 移出）、`src/i18n/app-launch.ts` |
| 测试 | Desktop `tests/e2e/pages/app-launch.spec.ts`（新）、`content-open.spec.ts`、`desktop.spec.ts`、`tests/e2e/fixtures/desktop.ts`（`routeAppHost`）、`tests/dv/app-launch.spec.ts`（新） |
| 文档 | `doc/sdk/SSO.md`（新增“从桌面打开 App”与已知取舍、轮换说明）、`doc/sdk/runtime-login.md`（refresh 宽限期、`AppAccessDenied` 范围）、`doc/control_panel/Control_Panel_Service.md`（新方法、no-store、公开方法改为 7 个） |

### 实施中的决定

- 桌面“强制刷新一次”用 SDK 现有能力实现：`buckyos.logout(false)`（只清 runtime 内存态，不调 `/sso_logout`）后 `getActiveSessionToken()`，SDK 因此走 `/sso_refresh`；多个窗口共用一个刷新 promise。没有修改 websdk。已知小概率竞态：清状态时若 SDK 自动续期正在进行，其结果会被丢弃，本次刷新可能判失败而多跳一次登录页。
- 启动去重：同一窗口同一目标 URL（含窗口自己的 `#bfp` nonce）的在途请求共用一个 promise，StrictMode 的重复 effect 不会多签发；答复按 URL 键丢弃过期结果。目标 URL 变化（重新导航）会重新申请；当前 UI 没有让已开 Web App 窗口换 launch 的路径（复用窗口经 AppFrame `frame.open` 交付内容，不再换票），这条路径没有单独 e2e。
- `auth.*` 全部加 `no-store`（TODO 只要求新方法；`auth.login` 同样返回 token）。
- mock：localStorage `buckyos.mock.app-launch` 取 `ready`（默认）/`access_denied`/`invalid_target`/`error`/`inactive_once`/`inactive`/`signed_out`；`ready` 返回与服务端同形的 `/sso_callback` URL。e2e 的 App host fixture 用页面脚本完成 callback 跳转，因为 Playwright 不会拦截 fulfill 出的 302 之后的请求（会打到 vite dev server）。
- `buckyos_admin.md` 无需修改：手册不涉及浏览器 SSO/登录；新 RPC 只接受 Control Panel 用户会话，Agent（App 主体）无法调用；refresh 宽限期不改变 Agent 注入 token 的使用方式。

### 验证

- `cd src && cargo test -p buckyos-api -p verify_hub -p control_panel -- --test-threads=1`：buckyos-api 250、control_panel 143、verify_hub 19 全部通过。新增覆盖：refresh 分类与 CAS、并发同一 refresh 拿同一对、超出宽限撤销 family、logout 后在途 refresh 不复活、delegate 调用方限制（用户会话/refresh/其它服务/App 主体/sudo/设备 LoginAssertion/非 JWT）、来源校验（伪造/过期/App/refresh/sudo/服务会话/已注销）、旧 access token 在 family 活跃时可用、logout 后不能提交委托；control-panel 的目标解析、callback URL 往返、结果状态线上格式、参数拒绝自报身份、委托 token pair 校验。
- Desktop：`pnpm check`、`pnpm exec eslint src tests`（0 error，9 个既有 warning）、`pnpm build` 通过；`pnpm exec playwright test` 全量 337 通过、3 失败、45 跳过。失败中 `ai-center.spec.ts:3`、`desktop.spec.ts:330 SN Router` 在未修改的 desktop 上同样失败（基线）；`filebrowser.spec.ts:150` 单跑通过（并发负载下不稳）。`tests/e2e/pages/app-launch.spec.ts` 7 例覆盖一次启动只换票一次、各状态处理、刷新一次后重试、刷新失败跳登录页、内容打开 invalid_target 回退 Preview。
- 真实 Zone（test.buckyos.io，本机 `/opt/buckyos`）：`uv run buckyos-build.py -s verify_hub control_panel desktop` 构建后，把 `rootfs/bin` 下的 `verify_hub`、`control_panel` 和 `control-panel/web` 拷入 `/opt/buckyos/bin`，结束两个进程由 node-daemon 拉起（原文件备份在 `/home/lzc/project/deployment-backups/20261021-desktop-app-sso/`）。`cd src/frame/desktop && BUCKYOS_TEST_ZONE_HOST=test.buckyos.io pnpm exec playwright test --config=playwright.dv.config.ts tests/dv/app-launch.spec.ts`：4 通过、1 跳过。
  - 桌面登录 devtest → All apps 打开 text-editor：iframe 先加载 `text-editor.<zone>/sso_callback`，落到带 `#bfp` 的 App URL，无登录页，`/sso_refresh` 200；App host 拿到 host-only 的两枚 Cookie（refresh 为 HttpOnly），session 为 `sub=devtest`、`app_instance_id=text-editor…@devtest`、`principal_kind=user`、非 sudo；发往 App host 的 `/sso_refresh` 只带一枚 refresh cookie。
  - `?open=cyfs:///home/devtest/<临时 md>`：callback 携带含 query 与 `%23bfp%3D` 的完整 URL，落地后编辑器经 AppFrame 报出文档标题。
  - 两个标签页各消费一张票据后三轮同时 `/sso_refresh`：每轮都是 200 且两边拿到同一枚 token（宽限期生效），family 未被撤销。
  - kRPC：`ready` 响应带 `no-store`、不含 token；callback 302 带 `no-store`、Location 为完整 redirect，第二次消费失败；devtest 打开 bob 的 Agent App（r03share）→ `access_denied`；别的 owner 的同名实例、Zone 根、外 Zone、未知 host → `invalid_target`；带 `username` 参数 → RPC error；非法/缺失 token → `source_session_inactive`；App 会话调用 → RPC error（需 Control Panel 用户会话）；浏览器直调 verify-hub `delegate_user_session` → `only control-panel may delegate…`；verify-hub `logout` 后用旧 session 换票 → `source_session_inactive`。
  - 共享 App 用例跳过：DV 上 bob 对 text-editor@devtest 没有 availability，实测返回 `access_denied`（sub/owner 的服务端检查由 control-panel 单测覆盖）。
  - 一次性脚本：登录桌面后重启 verify-hub，再从桌面打开 App → control-panel 日志 `status=source_session_inactive`、verify-hub 日志 `delegation source session is not active`，桌面刷新失败后进入 `/login?redirect_url=…`。DV 因此重启过两次 verify-hub，期间 Zone 内所有浏览器会话都需重新登录。

### 未验证与遗留

- DV 只走了 HTTPS；`force_https: false` 的 HTTP 模式未实测。
- 桌面 UI 里同一 App 真正连开两个窗口未实测（两个标签页直接消费票据代替）；“另一桌面用户重新启动同一 host”只有服务端检查，浏览器侧未跑。
- 多实例部署前提未验证（pending 在 Control Panel 进程内存、会话在 verify-hub 内存）。
- DV 现在运行的是本工作树（未提交）的 verify_hub、control_panel 和 desktop web；回退用上述备份。浏览器里的桌面要关掉所有标签页重开才会换到新前端（service worker 缓存）。
