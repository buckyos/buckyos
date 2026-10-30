# 跨 Zone 登录断言契约

日期：2026-10-30。状态：P0-3 冻结稿（[实施 TODO](<../../../notepads/cross-zone-comm-todo.md>) §2 规则 2、§6）；与代码冲突时以代码和本文同步修订为准。

依据：TODO §1 裁决 X-1～X-5、[混合 SSO 技术契约](<混合SSO技术契约.md>)（External 会话、D-4 原因码）。实现：`buckyos-api/src/cross_zone_auth.rs`（断言、custody、受保护头名）、`buckyos-api/src/cross_zone_client.rs`（调用方）、verify-hub `login_by_cross_zone_assertion`。

## 1. 总体

Source Zone 的服务进程要访问 Target Zone 的系统服务时：

1. 用本节点设备私钥签一份**登录断言**（§2），说明“设备 D 代表主体 P，要访问 Target Zone 的服务 S”。
2. 把断言交给 Target verify-hub 的 `login_by_cross_zone_assertion`（§4），换回 Target 本 Zone 签的 External token pair。
3. 之后对 S 的请求带 `Authorization: Bearer <session_token>`；S 只认本 Zone token。

Token 不跨 Zone：调用方的缓存键含 Target Zone DID；Target 的 token 只在 Target 有效（§5）。传输（RTCP 首选、HTTPS 降级）与认证无关：两条路提交同一份断言，RTCP 路径多一项一致性校验（§4.3）。

## 2. 登录断言

JWT，`alg=EdDSA`。header `kid = <设备 DID>#<key id>`（如 `did:web:ood1.alice.example.com#main_key`）。

| claim | 必填 | 含义 |
| --- | --- | --- |
| `iss` | 是 | 请求主体 DID（用户或 Agent），custody 校验中的 author |
| `sub` | 是 | 等于 `iss` |
| `dev` | 是 | 签名设备 DID，必须等于 `kid` 的 DID 部分 |
| `aud` | 是 | Target Zone DID 字符串 |
| `target` | 是 | 请求的 Target 服务，`system:<service_id>`；决定 token target（§4.2 白名单） |
| `app` | 否 | 调用应用的 AppId（hostname 形）；只记入 token 的 `src_app`，不参与授权 |
| `jti` | 是 | 随机串；Target 在断言有效期内拒绝重复 `(iss, jti)` |
| `iat`、`exp` | 是 | Unix 秒；`0 < exp − iat ≤ 60`；`iat ≤ now + 30`；`exp > now` |

`aud` 与 Target 自身 Zone DID 的比较按 Zone 主机名（`DID::to_host_name`，小写、去尾点）进行，`did:bns:alice` 与 `did:web:alice.bns.did` 视为同一 Zone；`did:web:<Target ZoneDocument hostname>` 也算命中（只知道 Zone 主机名的调用方，如 msg-center 群绑定、HomeStation，用 `CrossZoneClient::request_host` 发出的断言即为此形）。

签发：`buckyos_api::sign_cross_zone_assertion(&DeviceSigner, requester, target_zone, target, app)`。`DeviceSigner` 由 runtime 提供（`BuckyOSRuntime::device_signer()`，kid 固定 `<device did>#main_key`）；msg-center 与 HomeStation 不再各自持有签名钥。

## 3. Custody：设备能否代表主体

一份实现：`buckyos_api::check_custody`（断言、msg-center 消息签名、群请求、HomeStation 读者校验共用）。输入 `(signer kid, requester DID)`：

1. **签名钥**：kid 的 DID 若为 `did:dev`，公钥取自 DID 本身，没有文档；否则先按 `Device` 类型解析文档，`NotFound` 再按默认类型解析，用 kid 的 fragment 取 `authentication` 中的钥。
2. requester ＝ 签名 DID → `Signer`（主体用自己的钥签）。
3. requester 文档（默认类型）存在且 `authentication` 列出 kid（相对 id `#k` 按文档自己的 DID 展开）或签名 DID → `ListedInDoc`。
4. 设备文档的 `zone_did` ＝ requester 的 canonical Zone → `SameZone`。canonical Zone：服务本地记录先答（本 Zone 托管的用户与 Agent，`CustodyResolver::local_zone_of`）；否则 OwnerDocument 取 `binded_zone_list[0]`，为空取 DID 自身（自托管 Zone）；AgentDocument 取其 `owner` 的 canonical Zone；其他文档取 `zone_did`。
5. 都不成立 → 拒绝（`NotAuthorized`）。签名钥或文档不存在 → `UnknownSigner`。

解析三态：`NotFound` 是确定答案；`Unavailable`、`Timeout` 返回 `CustodyError::Unavailable`，调用方必须当作可重试，不得当作拒绝或通过。

断言登录额外要求 custody ≠ `Signer` 时设备有文档：`did:dev` 设备只有 KeyDid 级身份，不能代表任何用户（最小实现对端必须有可解析的设备文档）。

已知风险（沿用）：`SameZone` 意味着 Zone 可以替自己的用户签名。

## 4. verify-hub `login_by_cross_zone_assertion`

### 4.1 接口

公开方法（不需要 caller token）。

```json
{"method": "login_by_cross_zone_assertion",
 "params": {"assertion": "<JWT>", "scopes": ["msg_inbox"]}}
```

`scopes` 可省略或为空，表示 target 允许的全部 scope。成功返回现有 `TokenPair`（`session_token`、`refresh_token`）。失败为 kRPC `RPCErrors::Detailed`，`code` 取下表（`buckyos_api::CrossZoneLoginError`）：

| code | 含义 | 调用方处理 |
| --- | --- | --- |
| `assertion_invalid` | 格式、签名、时间窗、`aud`、`dev`、重放不合格 | 认证失败，不换传输重试 |
| `custody_rejected` | 设备不能代表 `iss` | 认证失败 |
| `zone_unavailable` | 解析设备或主体的 Zone 暂不可用 | 可重试 |
| `rtcp_identity_mismatch` | RTCP 隧道对端设备与断言设备不同 Zone | 认证失败 |
| `target_not_allowed` | `target` 不在白名单或请求了 target 不允许的 scope | 认证失败 |

### 4.2 签发的会话

External 家族（混合 SSO §2.1），不映射本 Zone 账号：

| 项 | 值 |
| --- | --- |
| `sub` | 断言 `iss`（完整 DID） |
| `principal_kind` / `external` | `user` / `true` |
| target | 断言 `target` |
| `auth_method` | `cross_zone_assertion` |
| `restricted` / `scopes` | `true` / 请求的 scope（白名单内） |
| `zone` | 本 Zone DID（所有 verify-hub token 都带，§5） |
| `src_device` | 断言设备 DID |
| `src_app` | 断言 `app`（有则带） |
| 寿命 | access 15 min、refresh 7 d、家族 30 d 封顶；`refresh_token` 续期不向 Source 复核 |

target 白名单与可请求 scope：

| target | scopes |
| --- | --- |
| `system:msg-center` | `msg_inbox`（`PUT/GET /kapi/msg-center/<did>/inbox`，跨 Zone 专用）、`msg_group`（`/kapi/msg-center/groups/...` 群同步 HTTP 路径；与联合登录 External 同一 scope，即同一 DID 的成员能力） |
| `system:homestation` | `homestation`（与联合登录 External 同一 scope：协议读取、评论、`/home/<user>/inbox` 投递） |

### 4.3 RTCP 一致性校验

请求带受保护头 `X-Cyfs-Rtcp-Device` 时（只可能由网关在 RTCP 来源流上写入，§6），该设备的 Zone 必须等于断言设备的 Zone（断言设备无 Zone 时比 `iss` 的 canonical Zone）；否则 `rtcp_identity_mismatch`。若流是从本 Zone 先拨出、被对方反向复用的隧道上回来的，网关以拨号时的 Zone DID 命名对端（握手已按该 Zone 的网关设备钥验证），此时头的值就是 Zone DID，与期望 Zone 同主机名即通过（DV 实测：bob 经 alice 拨出的隧道回到 alice 时头为 `did:bns:bob`）。没有该头（HTTPS 降级路径）时断言是唯一来源证明。

## 5. 业务侧校验

- verify-hub 签发的每个 token 都带 `zone` claim（签发 Zone DID）。`BuckyOSRuntime` 验证 session token 时，带 `zone` 的必须与本 Zone 同一主机名，否则 `InvalidToken`。
- 业务服务用 `verify_session_token_opened(token, scope)` 放行 External：msg-center inbox 用 `msg_inbox`、群同步用 `msg_group`，HomeStation 协议路径用 `homestation`。主体是 `sub`（DID）；msg-center 入站要求 `msg.from ＝ sub`，群请求要求 actor ＝ `sub`。
- 消息 JWT 仍是内容签名对象，不再充当认证凭据；`cyfs-group-request` 证明头与 HomeStation `Authorization: DID <proof>` 已删除。

## 6. 受保护头

`X-Cyfs-Rtcp-Device: <device did>`、`X-Cyfs-Rtcp-Trust: <identity_trust>`。只由 cyfs-gateway 在 RTCP 来源流上写；所有 HTTP 入口先删除同名头与 `cyfs-original-user`。业务代码只信网关写入的值。

## 7. 传输与 CrossZoneClient

`buckyos_api::CrossZoneClient`：

- 解析：Target Zone DID → ZoneDocument → `hostname`。
- RTCP：经本机网关 SOCKS5 入口 `127.0.0.1:3186`（用户名密码由 node_daemon 生成在 `$BUCKYOS_ROOT/etc/cross_zone_socks.json`，`{"username","password"}`，0600），请求 `http://<zone hostname>/...` 并显式带 `Host`；网关把 CONNECT `<zone hostname>:80` 转成到 Target Zone 默认网关设备的 RTCP 流（读其 `rtcp_port`）；非网关节点经本 Zone 网关 TCP 中转（`NODE_INFO.cross_zone_relay`，嵌套 bootstrap URL），握手端到端。Target 网关只把非同 Zone 来源的 80（带受保护头进 node_gateway）与 443 放进来。
- SOCKS 层的拒绝（RTCP 不通、准入拒绝）与连不上同样算传输失败，转 HTTPS；`known_owner` 准入在 Target 本机未缓存 Source 设备文档时拒绝，故首次登录通常走 HTTPS，之后重探切回 RTCP。
- HTTPS 降级：`https://<zone hostname>/...`。
- 传输状态按 Target Zone 维护：`RtcpOk`、`HttpsFallback`、`Unreachable`；RTCP 失败进入 `HttpsFallback`，之后每 5 min 重探 RTCP；两条都失败返回可重试错误。只有传输失败才换协议；verify-hub 拒绝或业务 401/403 直接失败。
- Token 缓存键 `(target_zone_did, principal_did, app_id)`，进程内存；access 过期前 60 s 用 refresh 换；refresh 失败或业务 401 → 重新登录一次；仍失败 → 认证错误。
- 网络超时的写请求不自动重放，调用方按消息 id 幂等重试。
- 错误：`CrossZoneError::Transport`（可重试）、`Auth`（不可重试，`zone_unavailable` 除外）、`Unroutable`（Target Zone 没有可用文档）、`Target`（kRPC 业务错误）。HTTP 请求（`request`/`request_to`/`request_host`）除 401 外的应答原样返回给调用方。
- 同 Zone 访问也走同一路径：HomeStation 把本 Zone 主机名映射到本机网关 `http://127.0.0.1:3180`（同时承载 `/kapi/verify-hub` 与 `/home/`）。

## 8. HomeStation standalone 模式

standalone（`homestation --config`，及其多节点测试）没有 verify-hub：每个 standalone Zone 在 `/kapi/verify-hub` 上自带一个替身（`homestation/src/standalone_hub.rs`），用同一个 `verify_signed_assertion` 校验断言，文档来源是配置里声明的 `StaticDirectory`（以 DID 文档形状提供：`#main_key`、列出可代签的设备、`zone_did`/`binded_zone_list`）；签出的 token 是 Zone 钥签的 JWT，只在该 standalone Zone 有效。
