# 跨 Zone 鉴权整理：三条路径的现状与取舍

日期：2026-10-29（America/Los_Angeles）。源码基线：`buckyos@688df1577`、`cyfs-gateway@245a378`、`buckyos-base`（name-lib 当前 checkout）。

本文是源码整理稿，不改变任何服务行为。§1–§5 是现状，§6 是建议，建议未拍板。关联：[#689 全链路分析](issue-689-cross-zone-attachments-protocol-todo.md)、[Agent＝特殊 App 授权 TODO](agent-appid-authorization-implementation-todo.md)。

## 1. 结论

“有身份的跨 Zone 操作如何验明身份”，系统里实际存在三条路，不是两条：

| 路径 | 机制 | 当前被谁使用 | 系统支持程度 |
|---|---|---|---|
| A. DID 文档 + 设备签名 + Zone custody | 发送方 Zone 的设备私钥签 JWT，kid 为设备 DID；接收方用 name_client 解析 DID Document，验签并判定“签名设备所在 Zone 是否托管该用户” | msg-center 原生消息入站、群同步请求、HomeStation 跨 Zone 读、verify-hub 联合登录的 Zone 定位 | 完整，现行主线 |
| B. rtcp 握手身份 | 网关 rtcp 握手验证对端设备 DID，写入 process-chain 的 `REQ.source_did` 等变量 | 仅 node executor 端口的设备 ACL（Zone 内） | 只有网关层，业务层零消费 |
| C. 对面 verify-hub 公钥验 session token | 用请求携带的信息找到 from-zone 的 verify-hub 公钥，验对方签发的 session token | 无 | 零支持，且公钥未公开 |

最近所有跨 Zone 修复都在路径 A 上做事，没有一处朝 B 或 C 推进。路径 A 的 custody 判定目前在三处各写一份（§5.1）。

## 2. 跨 Zone 操作清单与各自的鉴权方式

| 操作 | 方向 | 请求携带 | 接收方如何验 | 代码 |
|---|---|---|---|---|
| 原生消息投递（自动路由） | ZA msg-center → ZB msg-center `/inbox` PUT | MsgObject JWT，kid＝ZA 设备 DID，`cyfs-original-user`＝msg.from | `verify_signed_message_with`：解析 from 的文档或走 custody，取设备公钥验签 | [cyfs_dispatch.rs:529-626](../src/frame/msg_center/src/cyfs_dispatch.rs#L529-L626) |
| 原生消息投递（配置路由） | 同上 | 配置中的 `authorization: Bearer <token>` | 收方 `verify_trusted_session_token`，必须是收方 Zone 自己签的 token | [cyfs_dispatch.rs:24-33](../src/frame/msg_center/src/cyfs_dispatch.rs#L24-L33)、[cyfs_dispatch.rs:267-290](../src/frame/msg_center/src/cyfs_dispatch.rs#L267-L290) |
| 群同步 GET/PUT | 成员 Zone → 群宿主 Zone | 一条 Operation MsgObject 作为 proof，绑定 audience、method、path、body sha256 | `verify_group_request`：先核绑定，再走与消息相同的 `verify_signed_message_with` | [group_auth.rs:34-130](../src/frame/msg_center/src/group_auth.rs#L34-L130) |
| HomeStation 跨 Zone 读 feed/profile | ZB station → ZA station | `Authorization: DID <reader proof>`，iss＝读者 DID，aud＝`cyfs://<zone>/home`，TTL 300s，用本站设备钥签 | `verify_reader_proof` → `directory.authorizes`（custody）→ `directory.signer_key`（设备文档取钥） | [pull.rs:44-52](../src/frame/homestation/src/pull.rs#L44-L52)、[auth.rs:184-220](../src/frame/homestation/src/auth.rs#L184-L220)、[directory.rs:279-328](../src/frame/homestation/src/directory.rs#L279-L328) |
| 跨 Zone 用户登录（混合 SSO / DID 联合） | 用户在 ZB 用 ZA 的 DID 登录 | 浏览器跳 canonical Zone 授权，带回 code | ZB verify-hub 由 DID 解析 OwnerDocument → canonical ZoneDocument 主机名，后向 HTTPS 到 `https://<host>/kapi/verify-hub` 换 code 取 assertion，然后 ZB 自己签发 External 或账户 session | [verify_hub main.rs:2148-2282](../src/kernel/verify_hub/src/main.rs#L2148-L2282)、[verify_hub federation.rs:119-158](../src/kernel/verify_hub/src/federation.rs#L119-L158)、[control_panel federation.rs:514-615](../src/frame/control_panel/src/federation.rs#L514-L615) |
| 消息附件对象 GET | MessageHub/Agent → 本 Zone msg-center | 本 Zone session Bearer + `context_path` | 只验本 Zone token；无跨 Zone 回源，见 #689 | [object_access.rs:17](../src/frame/msg_center/src/object_access.rs#L17)、[object_access.rs:175](../src/frame/msg_center/src/object_access.rs#L175) |
| Node executor 端口 | Zone 内设备 → OOD 3184 | rtcp 握手身份 | 网关 process-chain `match-include $EXECUTOR_ACL ${REQ.source_device_id}` | [boot_gateway.yaml:54](../src/rootfs/etc/boot_gateway.yaml#L54) |

要点：session token 在任何一行都没有跨过 Zone 边界。它要么是收方 Zone 自己签的（配置路由、附件 GET），要么是跨 Zone 流程结束后收方新签的（联合登录）。

## 3. 路径 A：DID 文档 + 设备签名 + Zone custody

### 3.1 机制

1. 发送方用 Zone 内设备私钥（msg-center 的 `zone_signer`、HomeStation 的 `Signer`）签 JWT，kid 形如 `<device did>#<fragment>`。
2. 接收方解 kid 得到 signer DID。signer＝author 时直接解析 author 文档取公钥。
3. signer≠author 时走 custody：`zone_of(signer)` 与 `zone_of(author)` 相同即放行；否则再看 author 文档是否把 signer 列入 authentication。
4. custody 成立时从 signer 的设备文档取 auth key 验签；`did:dev` 直接从 DID 取 ed25519 公钥。
5. Zone 解析不到（名字服务不可达、超时）返回“不可用”而不是“伪造”，让发送方重试。这是 #684 修的部分，见 [f437ab4af](https://github.com/buckyos/buckyos/commit/f437ab4af)。

### 3.2 系统为它提供的现成能力

- `name_client::resolve_did(did, Some(DidDocType::Device|Zone))`、`resolve_auth_key`、`resolve_owner_document`。
- 设备文档带 `zone_did`，OwnerDocument 指向 canonical Zone，ZoneDocument 带主机名：DID → Zone → host 一条线能走通，verify-hub 联合登录定位 canonical Zone 也靠它。
- msg-center 的 `NetworkDirectory::zone_of` 带缓存与 `NotFound / Unavailable / Timeout` 三态；HomeStation 的 `Directory` 有同样三态。
- `MsgObject` JWT 本身就是带签名的信封，群请求直接复用它做 proof（Operation kind + request binding）。

### 3.3 已知风险

- custody 规则等于“Zone 可以替它托管的所有用户签名”，代码注释里已标为 known risk（[cyfs_dispatch.rs:500-501](../src/frame/msg_center/src/cyfs_dispatch.rs#L500-L501)、[directory.rs:302-303](../src/frame/homestation/src/directory.rs#L302-L303)）。这是设计选择，不是 bug，但需要在产品文档里写明。
- 验证输入是“一条消息”或“一个 reader proof”，不是通用 GET 的认证头。附件 GET 等普通 HTTPS 请求还没有等价的签名入口（#689 §4）。

## 4. 路径 B：rtcp 握手身份

- 握手后网关把 `real_source_did`、`source_did`、`source_canonical_device_id`、`source_identity_trust` 写入 REQ（[rtcp_stack.rs:341-370](../../cyfs-gateway/src/components/cyfs-gateway-lib/src/stack/rtcp_stack.rs#L341-L370)）。
- buckyos 侧唯一消费者是 node executor 端口 ACL。网关不把这些值注入 HTTP 头交给上游，kernel/frame 没有任何服务读取它们（全仓 grep `source_did` 只命中 msg-center 的联系人合并参数，与此无关）。
- [boot_gateway.yaml:62-64](../src/rootfs/etc/boot_gateway.yaml#L62-L64) 的注释已指出限制：rtcp 中转后只能看到中转节点的 DID；new_rtcp_stream 默认只允许 Zone 内中转。
- 它给出的是设备身份，给不出设备背后的用户或 Agent。要承担业务鉴权还需要在设备身份之上叠一层“代为谁”，这层正是路径 A 的 custody 在做的事。

适合的定位：网关层的设备准入与信任等级输入，而不是业务鉴权。

## 5. 路径 C：对面 verify-hub 公钥验 session token

- `verify_session_signature` 要求 iss 与 kid 都等于本地 `verify-hub`，trust key 只从本地 `zone_config.verify_hub_info` 和本地 owner key 装载（[runtime.rs:1733-1747](../src/kernel/buckyos-api/src/runtime.rs#L1733-L1747)、[runtime.rs:1849-1862](../src/kernel/buckyos-api/src/runtime.rs#L1849-L1862)）。外 Zone 签的 token 直接被拒。
- verify-hub 自己的 `load_trust_public_key_from_source` 只认本地 AgentSpec、root、本地用户、本地设备（[verify_hub main.rs:606-718](../src/kernel/verify_hub/src/main.rs#L606-L718)）。
- 公开的 ZoneDocument 里 `verify_hub_info` 是注释掉的（[name-lib zone.rs:590](../../buckyos-base/src/name-lib/src/zone.rs#L590)），只存在于本 Zone system_config 的 ZoneConfig 中。对面 Zone 拿不到这把公钥，路径 C 在基础设施上就不成立。
- token 还绑定 `AuthTarget`（target app/service）与 ceiling（Limited/External），语义上是 Zone 内凭据。混合 SSO 的设计也是“跨 Zone 只传 code，session 由本 Zone 签”，与路径 C 相反。

### 5.1 三处重复的 custody 代码

| 位置 | 函数 |
|---|---|
| msg-center 消息入站 | `zone_signs_for` + `verify_signed_message_with`，[cyfs_dispatch.rs:503-626](../src/frame/msg_center/src/cyfs_dispatch.rs#L503-L626) |
| msg-center 群请求 | `verify_group_request` 复用上面，[group_auth.rs:93-130](../src/frame/msg_center/src/group_auth.rs#L93-L130) |
| HomeStation | `NameDirectory::authorizes` + `signer_key`，[directory.rs:279-328](../src/frame/homestation/src/directory.rs#L279-L328) |

三份的判定顺序略有差异（HomeStation 多一条“signer 文档 owner＝principal”，msg-center 多一条 `is_hosted_identity` 的本地 Zone 短路），行为是否完全一致没有测试覆盖。

## 6. 建议（未拍板）

1. 继续以路径 A 为跨 Zone 身份主线，不为路径 C 公开 verify-hub 公钥。session token 保持 Zone 内凭据。
2. 把 custody 验证收口到 buckyos-api，输出统一的“可信 requester DID + acting-for DID + 认证来源 + 信任强度”，HTTPS 签名头与 rtcp 设备身份都作为它的输入。这与 #689 §5 和 agent-appid TODO §2 的方向一致。
3. 为普通 HTTPS 请求定义一个与 HomeStation `DID <proof>` 同形的签名头（aud 绑目标 Zone 与路径、短 TTL、jti 防重放），让附件 GET 等非消息请求也能走路径 A。
4. rtcp 身份只用于网关 ACL 和信任等级；若将来要把它交给业务层，需先解决中转后身份丢失的问题，并在注入上游前由网关而不是客户端写头。
5. 收口后补一组对照测试：三处 custody 对同一组（signer, author）输入给出相同结果，Zone 不可达时三处都返回可重试错误。

## 7. 最近修复回顾（按时间）

| 日期 | 提交 | 与跨 Zone 鉴权的关系 |
|---|---|---|
| 10-09 | 123aea4dc HomeStation 后端 | 引入 reader proof 与 custody 的 HomeStation 版本 |
| 10-22 | 0b9faa18a / 60259a4b5 混合 SSO | 跨 Zone 登录走 DID 解析 + 后向 code exchange，不传 token |
| 10-25 | f437ab4af #684 | custody 的 Zone 解析失败改为可重试，friend request 重发时复活 Dead 投递 |
| 10-28 | 1156ff598 #692 | 仅 HomeStation 前端播放问题，不涉及鉴权 |
| 10-28/29 | #689 分析稿、agent-appid TODO | 文档，提出统一认证入口，代码未动 |

## 8. 证据索引

- 本地 token 验签：[runtime.rs:1718-1890](../src/kernel/buckyos-api/src/runtime.rs#L1718-L1890)
- verify-hub 信任源：[verify_hub main.rs:606-790](../src/kernel/verify_hub/src/main.rs#L606-L790)
- 联合登录：[verify_hub main.rs:2131-2282](../src/kernel/verify_hub/src/main.rs#L2131-L2282)、[verify_hub federation.rs](../src/kernel/verify_hub/src/federation.rs)、[control_panel federation.rs](../src/frame/control_panel/src/federation.rs)
- 消息入站与 custody：[cyfs_dispatch.rs:262-626](../src/frame/msg_center/src/cyfs_dispatch.rs#L262-L626)
- 群请求 proof：[group_auth.rs](../src/frame/msg_center/src/group_auth.rs)
- HomeStation：[pull.rs](../src/frame/homestation/src/pull.rs)、[auth.rs](../src/frame/homestation/src/auth.rs)、[directory.rs](../src/frame/homestation/src/directory.rs)
- rtcp 身份：[rtcp_stack.rs:335-370](../../cyfs-gateway/src/components/cyfs-gateway-lib/src/stack/rtcp_stack.rs#L335-L370)、[boot_gateway.yaml:44-64](../src/rootfs/etc/boot_gateway.yaml#L44-L64)
- ZoneDocument 不含 verify-hub 公钥：[name-lib zone.rs:542-600](../../buckyos-base/src/name-lib/src/zone.rs#L542-L600)
