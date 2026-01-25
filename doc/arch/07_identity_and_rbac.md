# 07. 身份与权限（verify-hub / session-token / RBAC）

BuckyOS 把“身份与权限”放在系统主链路上，目标不是“给网关加一个鉴权层”，而是让系统内部协作也具备可验证的身份与可执行的权限语义：
- 每个请求都能回答：`谁( userid )`、`以哪个 app 身份( appid )`、`要访问哪个资源( resource_path )`、`做什么动作( action )`。
- RBAC 策略是 system-config 的一部分（控制面状态），因此会参与调度与系统行为推导，而不是只停留在某个入口。

本文档按两个问题展开：
1) token 是什么、谁签、谁验证、如何在服务侧 enforce；
2) trust keys / RBAC policy 更新时，系统如何收敛，以及有哪些“看起来像 bug 其实是缓存/过期”的坑。

实现/协议参考：
- `src/kernel/verify_hub/src/main.rs`
- `src/kernel/verify_hub/README.md`
- `new_doc/ref/notepads/rbac.md`

## 读者的心智模型（把链路看对）

把身份与权限拆成三层：
- 身份载体：`RPCSessionToken`（本质是 JWT + 一些解析后的字段）。
- 信任根：每个服务维护一份 trust keys（public keys），用来验证“这个 JWT 该信谁”。
- 策略执行：RBAC model/policy 决定 `(userid, appid)` 是否能对 `resource_path` 做 `action`。

因此一次完整的服务端处理通常是：
1) `verify`：校验 token 的签名与 exp（以及 kid 是否受信）；
2) `enforce`：读取 RBAC policy（可能带缓存），执行权限判断；
3) `handle`：业务逻辑。

## 关键数据结构：RPCSessionToken（从代码抽取的最小视图）

注意：这里只描述在本仓库代码中“被实际使用到”的字段，不引入额外猜测。

### 1) verify-hub 签发的 session_token 形态
`src/kernel/verify_hub/src/main.rs` 里，verify-hub 构造的 `RPCSessionToken` 明确包含字段：
- `token_type: RPCSessionTokenType::JWT`
- `nonce: Option<u64>`
- `appid: Option<String>`
- `userid: Option<String>`
- `token: Option<String>`（最终 JWT 字符串）
- `session: Option<u64>`
- `iss: Option<String>`
- `exp: Option<u64>`（unix timestamp seconds）

对应代码锚点（字段初始化）：`src/kernel/verify_hub/src/main.rs`。

关键语义（从实现可见）：
- verify-hub 会把 `RPCSessionToken` 作为 JWT claims，再把生成出的 JWT 写回 `token` 字段（见 `src/kernel/verify_hub/src/main.rs` 内 `generate_session_token()` 以及 `get_my_krpc_token()`）。
- `exp` 用于服务侧/verify-hub 侧的过期判断（`src/kernel/verify_hub/src/main.rs` 内 `handle_verify_session_token()`）。

### 2) system-config 服务端对 RPCSessionToken 的使用方式
system-config 服务端的调用入口会把请求携带的 `session_token` 解析为 `RPCSessionToken` 并验证，再把它“作为已认证身份”传给各个 handler：
- 解析：`RPCSessionToken::from_string(session_token.as_str())`（`src/kernel/sys_config_service/src/main.rs`）
- 验证：`verify_session_token(&mut rpc_session_token).await?`（`src/kernel/sys_config_service/src/main.rs`）
- 过期检查：当 `rpc_session_token.exp.is_some()` 时对比 `now > exp`，过期则返回 `RPCErrors::TokenExpired`（`src/kernel/sys_config_service/src/main.rs`）
- 授权：各 handler 内调用 `enforce(userid, session_token.appid.as_deref(), full_res_path, "read"/"write")`（例如 `src/kernel/sys_config_service/src/main.rs` 的 `handle_get()` / `handle_set()` 等）

这意味着：system-config 把“身份验证”与“资源授权”分开处理，且 RBAC 的资源粒度直接绑定到 KV path（`full_res_path`）。

## verify-hub：登录与 token 中心

verify-hub 的职责不是“替所有服务做鉴权”，而是提供一个统一的登录与 session token 发行点。

协议与典型链路（文字版）：`src/kernel/verify_hub/README.md`
- node-daemon 用设备签名构造 service jwt，并传给 app/service。
- app/service 使用该 jwt 调用 verify-hub `login`，得到 `session_token`。
- app/service 携带 `session_token` 去访问其它内核服务。
- 被调用方验证 token，并结合 RBAC 限制后返回。

### session_token 的生成与刷新（nonce/session 的用法）
从 `src/kernel/verify_hub/src/main.rs` 可以观察到两条重要机制：

1) login jwt 一次性（replay 防护）
- 外部 jwt 登录会以 `{userid}_{appid}_{nonce}` 作为 key 写入 `TOKEN_CACHE`。
- 如果 cache 已存在，则返回 `login jwt already used`（`src/kernel/verify_hub/src/main.rs`）。

2) rolling nonce（刷新链路）
- 当传入 jwt 的 kid/iss 表现为 verify-hub 自签时，会进入“刷新”分支：从 cache 中取出旧 token，并要求 `old_token.nonce == token_nonce`。
- 刷新成功后，会生成新的 `nonce` 并覆盖 cache。
- 这使得 token 刷新具备“链式一致性”：只有持有最新 nonce 的客户端才能继续刷新（`src/kernel/verify_hub/src/main.rs`）。

token 有效期：
- verify-hub 客户端常量 `VERIFY_HUB_TOKEN_EXPIRE_TIME = 60*10`（10 分钟）见 `src/kernel/buckyos-api/src/verify_hub_client.rs`。
- password 登录路径中示例使用 `3600 * 24 * 7`（7 天）见 `src/kernel/verify_hub/src/main.rs`。

### verify_token：集中验证接口（可选路径）
verify-hub 提供 `verify_token` 协议（`src/kernel/verify_hub/README.md`）：
- request: `{ "method": "verify_token", "params": { "session_token": "$session_token" } }`
- response: `{ "userid": "...", "appid": "...", "exp": 123... }`

这条路径适用于“验证方不想/不能维护 trust keys”的场景：把 token 发给 verify-hub 由其验证。

## 服务侧 enforce：token 校验 + RBAC 执行

### 典型伪代码：login -> session_token -> verify/enforce

```text
client_or_service_boot():
  // 1) 拿到一个用于登录的 jwt（device/user 私钥自签，或来自 node-daemon 注入）
  login_jwt = env_or_key.generate_jwt()

  // 2) 向 verify-hub login
  session_token = verify_hub.login(type="jwt", jwt=login_jwt)

call_other_service(req):
  req.session_token = session_token
  return rpc.call(req)

service_handle(req):
  // 3) 先验证 token（两种模式）
  if local_has_trust_keys:
     (userid, appid) = local_verify_jwt(req.session_token)
  else:
     (userid, appid) = verify_hub.verify_token(req.session_token)

  // 4) enforce
  allowed = rbac.enforce(userid, appid, req.resource_path, req.action)
  if !allowed:
     deny

  // 5) 业务逻辑
  handle(req)
```

### 本仓库里的“落地形态”
- system-config 服务端把 `RPCSessionToken` 作为 handler 入参，并在 handler 内做 `enforce()`（`src/kernel/sys_config_service/src/main.rs`）。
- runtime 提供通用的 `BuckyOSRuntime::enforce()`：
  - 本地解码 JWT header.kid 并从 `trust_keys` 查找对应 `DecodingKey` 进行验签
  - 拉取 `system/rbac/policy`（通过 system-config client，带 cache），如 `is_changed` 则 `rbac::update_enforcer(...)`
  - 执行 `rbac::enforce(...)`（`src/kernel/buckyos-api/src/runtime.rs`）

## trust keys：为什么 boot/config 变化会影响验证结果

trust keys 的核心作用是：把 JWT header 的 `kid` 映射到可用于验签的 `DecodingKey`。

### system-config 如何初始化与刷新 trust keys
`src/kernel/sys_config_service/src/main.rs` 里存在一个显式的刷新入口：`handle_refresh_trust_keys()`，其行为（按代码路径）包括：
- 清空：`TRUST_KEYS.lock().await.clear()`（`src/kernel/sys_config_service/src/main.rs`）
- 从 `boot/config` 读取 `ZoneConfig` 并导入：
  - `verify-hub` 的 public key（插入 key `"verify-hub"`）
  - owner 的 public key（插入 key `"root"`、`"$default"`、以及 `owner_did.to_string()`）
- 从环境变量 `BUCKYOS_THIS_DEVICE` 读取本机 `DeviceConfig`，并把设备默认 key 同时插入两种 kid：
  - `device_doc.name`
  - `device_doc.id.to_string()`

同时：
- service 启动时会调用 `init_by_boot_config()`，其中第一步就是 `handle_refresh_trust_keys()`（`src/kernel/sys_config_service/src/main.rs`）。
- system-config 暴露了 rpc 方法 `sys_refresh_trust_keys` 来触发刷新（`src/kernel/sys_config_service/src/main.rs`）。

这解释了一个现象：当 `boot/config`（ZoneConfig）发生变化（例如 verify-hub public key 或 owner key 更新）时，旧的 trust keys 会让 token 验签失败或产生不一致行为，必须刷新。

### 服务侧 trust keys 的更新时机
runtime 内部也维护 trust keys，并提供刷新逻辑（`src/kernel/buckyos-api/src/runtime.rs` 的 `refresh_trust_keys()`）。另外其 keep-alive 会周期执行：
- 重新拉取 RBAC config 并重建 enforcer
- `refresh_trust_keys()`（`src/kernel/buckyos-api/src/runtime.rs`）

因此“boot/config 更新后多久生效”取决于服务是否及时刷新 trust keys，以及服务是否重启。

## RBAC 策略与缓存：10s 延迟来自哪里

notepads 已明确标注了传播延迟：`new_doc/ref/notepads/rbac.md`。

落地实现里，这个“最长约 10s”并不是 RBAC 引擎内部硬编码，而是 system-config client 的缓存 TTL：
- `CONFIG_CACHE_TIME: u64 = 10`（`src/kernel/buckyos-api/src/system_config.rs`）
- cache 覆盖 key 前缀包含 `system/rbac/`（`src/kernel/buckyos-api/src/system_config.rs`）

这意味着：
- 你写入 `system/rbac/policy` 后，各个服务进程可能要等到各自缓存过期（或主动失效）才会看到新策略。
- notepads 提到的 watch/主动失效仍是 TODO（`new_doc/ref/notepads/rbac.md` 与 `src/kernel/buckyos-api/src/system_config.rs`）。

## 常见坑（调试时优先排查）

### 1) “改了权限不生效”
- 先确认写的是 `system/rbac/policy`（不是 base_policy）。
- 再确认缓存窗口：`CONFIG_CACHE_TIME = 10`（`src/kernel/buckyos-api/src/system_config.rs`）+ 分布式传播。
- 观察点：`new_doc/ref/notepads/rbac.md` 明确写了“最长 10 秒”。

### 2) “token 明明刚拿到就过期/失败”
- verify-hub token 默认 10 分钟（`src/kernel/buckyos-api/src/verify_hub_client.rs`），服务侧还会检查 `exp`（例如 `src/kernel/sys_config_service/src/main.rs`）。
- runtime 会在接近过期时尝试 renew：`renew_token_from_verify_hub()` 内对 `exp - 30` 做判断并触发 `login_by_jwt`（`src/kernel/buckyos-api/src/runtime.rs`）。

### 3) “刷新 token 后，旧 token 立刻不能用了”
- verify-hub 的 refresh 使用 rolling nonce 并更新 `TOKEN_CACHE`（`src/kernel/verify_hub/src/main.rs`）。
- 如果网络重试/响应丢失导致客户端没拿到新 token，会出现“持有旧 nonce 无法继续刷新”的现象。

### 4) “verify-hub 重启后，部分会话异常”
- verify-hub 的 `TOKEN_CACHE` 是内存态静态 HashMap（`src/kernel/verify_hub/src/main.rs`），没有持久化/回收策略。
- 服务重启意味着 cache 丢失：一些依赖 cache 语义的刷新/一次性校验会表现为失败或行为变化。

## 与类似系统的差异点（再强调一次）

- BuckyOS 的身份模型与 ZoneBootConfig/Secure Boot 强耦合：先建立可信 Zone，再谈服务间信任（信任根最终落到 `boot/config` 与 trust keys 刷新链路）。
- RBAC 策略是调度器/系统状态的一部分，而不仅是某个网关或某个 API 的局部配置。
- 权限调试要同时看三件事：token 验签（trust keys）、token 过期/刷新（verify-hub）、RBAC policy 缓存传播（10s）。
