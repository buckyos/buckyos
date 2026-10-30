# 跨 Zone 登录断言契约

日期：2026-10-30。状态：P0-3 冻结稿（[实施 TODO](<../../../notepads/cross-zone-comm-todo.md>) §2 规则 2、§6）；与代码冲突时以代码和本文同步修订为准。

依据：TODO §1 裁决 X-1～X-5、[混合 SSO 技术契约](<混合SSO技术契约.md>)（External 会话、D-4 原因码）。实现：`buckyos-api/src/cross_zone_auth.rs`（断言、custody、受保护头名）、`buckyos-api/src/cross_zone_client.rs`（调用方）、verify-hub `login_by_cross_zone_assertion`。

## 1. 总体

Source Zone 的服务进程要访问 Target Zone 的系统服务时：

1. 用本节点设备私钥签一份**登录断言**（§2），说明“设备 D 代表主体 P，要访问 Target Zone 的服务 S”。
2. 把断言交给 Target verify-hub 的 `login_by_cross_zone_assertion`（§4），换回 Target 本 Zone 签的 External token pair。
3. 之后对 S 的请求带 `Authorization: Bearer <session_token>`；S 只认本 Zone token。

Token 不跨 Zone：调用方按 Target 主机名、服务、主体和应用隔离缓存（§7）；Target 的 token 只在 Target 有效（§5）。RTCP 与 HTTPS 使用同一断言验证规则，但客户端每次登录传输尝试都签新断言；RTCP 路径多一项一致性校验（§4.3）。端到端接入与排障见[跨 Zone 通信开发者指南](../../sdk/cross-zone-communication.md)。

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
| `jti` | 是 | 随机串；Target 在断言有效期内用进程内缓存检查 `(iss, jti)` 重用，不提供跨重启或多副本的全局一次性保证 |
| `iat`、`exp` | 是 | Unix 秒；`0 < exp − iat ≤ 60`；`iat ≤ now + 30`；`exp > now` |

`aud` 与 Target 自身 Zone DID 的比较按 Zone 主机名（`DID::to_host_name`，小写、去尾点）进行，`did:bns:alice` 与 `did:web:alice.bns.did` 视为同一 Zone；`did:web:<Target ZoneDocument hostname>` 也算命中（只知道 Zone 主机名的调用方，如 msg-center 群绑定、HomeStation，用 `CrossZoneClient::request_host` 发出的断言即为此形）。

签发：`buckyos_api::sign_cross_zone_assertion(&DeviceSigner, requester, target_zone, target, app)`。`DeviceSigner` 可由 runtime 提供（先 `load_device_private_key()`，再 `BuckyOSRuntime::device_signer()`，kid 为 `<device did>#main_key`）；msg-center 与 HomeStation 的跨 Zone 登录复用这一入口。

## 3. Custody：设备能否代表主体

一份实现：`buckyos_api::check_custody`，登录断言、msg-center 消息签名和 HomeStation 对象验签共用。先用 `resolve_signer_key` 解析签名者，再传入 resolver、`SignerKey`、requester DID 和可选本地托管 Zone 提示：

1. **签名钥**：kid 的 DID 若为 `did:dev`，公钥取自 DID 本身，没有文档；否则先按 `Device` 类型解析文档，`NotFound` 再按默认类型解析，用 kid 的 fragment 取 `authentication` 中的钥。
2. requester ＝ 签名 DID → `Signer`（主体用自己的钥签）。
3. 调用方传入的 `local_principal_zone` 与设备 `zone_did` 一致 → `SameZone`。本地提示只增加一种通过方式，不覆盖主体文档允许的代表关系。
4. requester 文档先按 `Owner` 类型解析，`NotFound` 再取默认类型；其 `authentication` 列出 kid（相对 id `#k` 按文档自己的 DID 展开）或签名 DID → `ListedInDoc`。
5. 设备文档的 `zone_did` ＝ requester 文档给出的 canonical Zone → `SameZone`。OwnerDocument 取 `binded_zone_list[0]`，为空或无绑定的自托管 Owner 取 DID 自身；AgentDocument 取其 `owner` 的 canonical Zone；其他文档取 `zone_did`。
6. 都不成立 → 拒绝（`NotAuthorized`）。无法取得签名钥 → `UnknownSigner`。

解析三态：`NotFound` 是确定答案；`Unavailable`、`Timeout` 返回 `CustodyError::Unavailable`，调用方必须当作可重试，不得当作拒绝或通过。

`did:dev` 公钥本身不提供设备 Zone 归属，不能凭 `SameZone` 代表其他主体；但当前登录路径没有额外禁止 `Signer` 或主体文档显式授权的 `ListedInDoc`。这与 RTCP 的具名设备准入是两个检查层。

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
- `verify_session_token_opened` 校验会话及 scope，不额外比较 target 与当前服务名；业务权限仍由服务检查。HomeStation Inbox 以会话控制入口，以已验签对象确定 publisher，并不统一要求 publisher 等于会话主体。
- 消息 JWT 仍是内容签名对象，不再充当认证凭据；`cyfs-group-request` 证明头与 HomeStation `Authorization: DID <proof>` 已删除。

## 6. 受保护头

`X-Cyfs-Rtcp-Device: <device did>`、`X-Cyfs-Rtcp-Trust: <identity_trust>`。只由 cyfs-gateway 在 RTCP 来源流上写；所有 HTTP 入口先删除同名头与 `cyfs-original-user`。业务代码只信网关写入的值。

## 7. 传输与 CrossZoneClient

`buckyos_api::CrossZoneClient`：

- 解析：Target Zone DID → ZoneDocument → `hostname`。
- RTCP：经本机网关 SOCKS5 入口 `127.0.0.1:3186`（用户名密码由 node_daemon 生成在 `$BUCKYOS_ROOT/etc/cross_zone_socks.json`，`{"username","password"}`，Unix 权限 0600），请求 `http://<zone hostname>/...` 并显式带 `Host`；网关把 CONNECT `<zone hostname>:80` 转成到 Target Zone 默认网关设备的 RTCP 流（读其 `rtcp_port`）；非网关节点经本 Zone 网关 TCP 中转（`NODE_INFO.cross_zone_relay`，嵌套 bootstrap URL），握手端到端。Target 网关的外 Zone 通用转发仅开放 80（带受保护头进 node_gateway）与 443；3184 由优先执行的 Node Executor 设备 ACL 单独判定。
- SOCKS 层的拒绝（RTCP 不通、准入拒绝）可表现为连接失败，触发 HTTPS 降级；入站 `known_owner` 验证不向普通 provider / method authority 发起网络查询，仍可使用 Zone locker 和本地缓存。从这些来源取不到 Source 的可信设备/owner 资料时，首次登录可能走 HTTPS，解析填充缓存后由后续重探尝试 RTCP。
- HTTPS 降级：`https://<zone hostname>/...`。
- 传输状态按 Target 主机名维护：初始 `Probing`，随后为 `RtcpOk`、`HttpsFallback` 或 `Unreachable`。RTCP 失败进入 `HttpsFallback`，5 min 窗口内使用 HTTPS，之后由下次请求重探 RTCP；没有独立后台探测。两条都失败返回可重试错误。任意 HTTP 响应都结束传输选择，不因 401/403/5xx 换协议。
- Token 缓存键 `(target_host, service, principal_did, app_id)`，进程内存；同一键登录/续期串行，access 进入过期前 60 s 时优先 refresh。refresh 传输失败返回可重试错误，其他刷新拒绝再登录。HTTP `request*` 遇 401 清除缓存、重新登录并重发一次，仍 401 返回认证错误；403 原样返回。
- 网络超时的写请求不自动重放，调用方按消息 id 幂等重试。
- 错误：`CrossZoneError::Transport`（可重试）、`Auth`（不可重试，`zone_unavailable` 除外）、`Unroutable`（Target Zone 没有可用文档）、`Target`（kRPC 业务错误）。HTTP 请求（`request`/`request_to`/`request_host`）除 401 外的应答原样返回给调用方。
- 同 Zone 访问也走同一路径：HomeStation 把本 Zone 主机名映射到本机网关 `http://127.0.0.1:3180`（同时承载 `/kapi/verify-hub` 与 `/home/`）。

## 8. HomeStation standalone 模式

standalone（`homestation --config`，及其多节点测试）没有 verify-hub：每个 standalone Zone 在 `/kapi/verify-hub` 上自带一个替身（`homestation/src/standalone_hub.rs`），用同一个 `verify_signed_assertion` 校验断言，文档来源是配置里声明的 `StaticDirectory`（以 DID 文档形状提供：`#main_key`、列出可代签的设备、`zone_did`/`binded_zone_list`）；签出的 token 是 Zone 钥签的 JWT，只在该 standalone Zone 有效。
