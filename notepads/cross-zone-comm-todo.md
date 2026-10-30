# BuckyOS 跨 Zone 通信实施 TODO

日期：2026-10-30

状态：三项裁决已由用户定案（§1）。2026-10-30 P0–P3 与 P1 传输层已实施（四仓均未提交），ssotest DV 除 V-6（单测覆盖）、V-12（未全新重装）外均通过；实施记录、偏差、DV 结果与遗留见 §8。本文是 code agent 的工作指南：先读 §2 硬规则，再按 §4 分期领任务；"完成"以 §5 验收为准。

路径相对 `buckyos/`，`cyfs-gateway/`、`buckyos-base/` 为同级仓。源码基线：`buckyos@688df1577`、`cyfs-gateway@1aa9edd`、`buckyos-base@35bac92`。依据文档：

- 方案：[BuckyOS 跨Zone通信方案思考.md](BuckyOS%20跨Zone通信方案思考.md)（方向稿）
- review 与现状核实：[cross-zone-comm-proposal-review.md](cross-zone-comm-proposal-review.md)（§1 现状表、§2 设计问题、§5 裁决）
- 鉴权三条路径整理：[cross-zone-auth-overview.md](cross-zone-auth-overview.md)
- 已定的 External 会话模型：[hybrid-sso-implementation-todo.md](hybrid-sso-implementation-todo.md)（D-7、Q-03、Q-04、附录 A）
- 附件跨 Zone 读：[issue-689-cross-zone-attachments-protocol-todo.md](issue-689-cross-zone-attachments-protocol-todo.md)（§5.2 的认证层契约由本文落实）

## 0. 目标与边界

**目标**：Zone 内服务（msg-center、HomeStation、Agent runtime 等）访问另一个 Zone 的系统服务时，统一走"先向 Target verify-hub 登录换本 Zone Token，再调业务服务"的路径；传输 RTCP 首选、HTTPS 降级，由 Gateway 与 buckyos-api 封装；Device/Zone/User 的代表关系验证收口为一份实现。

**同时修掉**跨 Zone 真跑起来才暴露的老模块问题（review §5.2 十四项），全部归入 §4。

**不在范围**：Relay 节点的发现字段与准入（§4 P4 只留接口）；反向建连（P4）；用户级同意流程（已由混合 SSO 联合登录覆盖）；消息格式与 Inbox 业务规则；浏览器入口。

**不写兼容**：按 `AGENTS.md` 与"DV 可全新安装"规则，msg-center 投递、群同步、HomeStation 拉取切到 Token 路径后，旧的"消息 JWT 即凭据"入站分支删除，不双轨；DV 用 `uv run src/start.py --all` 重装三 Zone VM 组（ssotest）。

## 1. 已定裁决（2026-10-30，不再讨论）

| 编号 | 裁决 |
|---|---|
| X-1 | 投递也先换 Token。聊天是反复投递，隧道与 Token 保持一段时间有效。Token 寿命沿用 External：access 15 min、refresh 7 d、家族 30 d 封顶、续期不向 Source 复核。 |
| X-2 | custody（设备→Zone→用户代表关系）验证收口到 buckyos-api 库；verify-hub、msg-center、HomeStation 都是调用方，不再各写一份。 |
| X-3 | 跨 Zone 登录凭据是**签名断言**（设备私钥签的短 TTL JWT，同 HomeStation `DID <proof>` 形）。它是辅助设施：RTCP 路径上以握手得到的设备身份为主、断言做一致性校验；HTTPS 降级路径上断言是唯一来源证明。不可信的是未签名的头（`cyfs-original-user` 等），不是签过名的 Authorization。 |
| X-4（沿用方案） | Token 只由 Target Zone verify-hub 签，session token 不跨 Zone，不公开 verify-hub 公钥。业务服务只认本 Zone Token，支持外部身份（DID 形 sub）。 |
| X-5（沿用方案 + review §2.7） | 跨 Zone 出口统一经 Zone 网关设备（ZoneDocument 默认网关 OOD），其他节点经现有 via-zone-gateway 路由借道，不每台设备各建跨 Zone 隧道。 |

## 2. 给 code agent 的硬规则

1. **同一份 custody。** 新建 `src/kernel/buckyos-api/src/cross_zone_auth.rs`（名称可调）提供 `verify_signed_assertion(jwt, expect_aud) -> VerifiedRequester { requester_did, device_did, device_zone_did, custody: SameZone|ListedInDoc, trust, app_id? }`。msg-center `cyfs_dispatch.rs:503-626` 的 `zone_signs_for`/`verify_signed_message_with`、`group_auth.rs:93-130`、HomeStation `directory.rs:279-328` 的判定全部改为调用它；Zone 解析不可达返回可重试错误（沿用 #684 的三态）。
2. **断言不冻结格式前不写业务。** §6 的断言字段由负责 P0 的工程师先冻结写进 `doc/`，P2/P3 才能开工。
3. **受保护头由网关写、业务只读。** 任何表示"RTCP 已验证设备身份"的 HTTP 头只能由 cyfs-gateway 在 RTCP 入站流上写入，非 RTCP 入口一律 `map-remove`。业务代码不得信任请求自带的同名头。
4. **Token 不跨 Zone。** 调用方缓存的是"对某个 Target Zone 的 Token"，键必须含 target zone DID；不得把本 Zone Token 发到外 Zone，也不得把 A 的 Token 用于 B。
5. **传输失败与认证拒绝分开。** RTCP 连不上 → 降级 HTTPS；verify-hub 401/403 或网关准入拒绝 → 直接失败，不换协议重试（方案 §15.1）。
6. **外部身份不截短。** External 会话 sub 为完整 DID；ACL、存储、比较都用 DID，展示名另取。
7. **不新增认证服务、不新增权限引擎。** 一切在 verify-hub + buckyos-api + cyfs-gateway 现有组件内完成。

## 3. 目标链路

```text
Source Zone 服务进程（msg-center / homestation / agent runtime）
  │ CrossZoneClient.call(target_zone_did, service, principal, req)
  ├─ 解析：target_zone_did → ZoneDocument(hostname, 默认网关设备 did + rtcp_port)   [name-client，一份实现]
  ├─ Token 缓存命中？ (target_zone, principal_did, app_id) → 直接带 Bearer
  └─ 未命中：签断言（本节点设备钥，kid=<dev did>#key）→ POST /kapi/verify-hub login_by_cross_zone_assertion
        │
        ├─ 传输 1：经本机 cyfs-gateway SOCKS5 → PROXY rtcp://<gw dev hostname>:<rtcp_port>/ → Target 网关 RTCP
        │        Target 网关：准入（admission + ACL）→ 写受保护头 X-Cyfs-Rtcp-Device → http stack → /kapi/verify-hub
        └─ 传输 2（RTCP 失败）：https://<zone hostname>/kapi/verify-hub   （Target 网关清掉 X-Cyfs-Rtcp-Device）
                │
          Target verify-hub：verify_signed_assertion(aud=本 Zone) → 若有 X-Cyfs-Rtcp-Device，要求与断言设备同 Zone
                → 签 External 家族 token pair（sub=requester DID, external, zone=本 Zone, target=请求的 service）
                │
          后续业务请求：同一传输 + Authorization: Bearer <access>；业务服务 verify_session_token_opened(scope)
```

## 4. 分期任务

### P0 认证底座（先做，P2/P3 依赖）

| 任务 | 落点 | 要点 |
|---|---|---|
| P0-1 custody 库 | `buckyos-api/src/cross_zone_auth.rs`（新） | 规则：kid 定位设备文档（`resolve_did(dev, Some(Device))`，取不到按默认类型）；requester＝断言 `iss`；requester 文档存在且 authentication 列出 kid → `ListedInDoc`；否则 `zone_of(device) == canonical_zone_of(requester)` → `SameZone`；都不成立拒绝。Zone 解析 NotFound/Unavailable/Timeout 三态，后两者返回可重试。Zone 主机解析统一为 OwnerDocument → canonical Zone → ZoneDocument hostname（替换 msg-center `network_directory.rs:86-200` 的 `binded_zone_list[0]`）。 |
| P0-2 三处调用方切换 | `msg_center/cyfs_dispatch.rs`、`group_auth.rs`、`homestation/directory.rs`+`auth.rs` | 删本地判定，调 P0-1；补对照测试：同 (signer, author) 输入三处同结果；Zone 不可达三处都可重试。 |
| P0-3 签名断言冻结 | `doc/`（新文）+ `buckyos-api` 的签发/校验函数 | 字段见 §6.1。签发函数供 CrossZoneClient 与测试用；校验即 P0-1 的入口。 |
| P0-4 verify-hub 新登录 | `verify_hub/src/main.rs`（参照 `login_by_remote_verify_hub` 2148-2281）、`verify_hub_client.rs` 方法表 | 新方法 `login_by_cross_zone_assertion`：公开调用（无 caller token）；校 aud＝本 Zone DID；jti 重放缓存复用 `assertion:{iss}:{sub}:{jti}`；若请求带受保护头 `X-Cyfs-Rtcp-Device`，要求其与断言设备同 Zone（调 P0-1 的 zone_of）；签 External 家族。**不改** `login_by_jwt` 的本地信任范围。 |
| P0-5 External 会话扩展 | `verify_hub/src/hybrid_sso.rs:122-168`、`family_store.rs`、`buckyos-api/src/app_availability.rs` | target 允许 `system:<service>`（msg-center、homestation、aiworkspace…白名单）而不只 control-panel；claims 加 `zone`（签发 Zone DID）、`auth_method=cross_zone_assertion`、`src_device`；scopes 增 `msg_inbox`、`msg_group_sync`、`homestation_pull`（名字随 P3 定）。 |
| P0-6 业务侧校验 | `buckyos-api/src/runtime.rs:1836-1889` | `verify_session_signature`：token 带 `zone` 时必须等于本 Zone DID；`verify_session_token_opened` 按 scope 放行 External。msg-center `cyfs_dispatch::serve` 287 行改用 opened 版本。 |

### P1 传输底座（与 P0 并行）

| 任务 | 落点 | 要点 |
|---|---|---|
| P1-1 读 `rtcp_port` | `cyfs-gateway-lib/src/rtcp/rtcp.rs:3859`、`1321-1421` | 解析到设备文档且 URL 未显式给端口时，用 `DeviceDocument.rtcp_port`，否则 2980。 |
| P1-2 关 KeyDid 口子 | `rtcp.rs:2354-2363`、`2493-2522` | 无 `device_doc_jwt` 的对端走 `admit_without_named_identity`，受 `anonymous` 约束；默认 reject。补测试（现有 13176-13184 的 did:dev 用例改为显式 `anonymous: allow`）。 |
| P1-3 跨 Zone 入站准入 | `rtcp.rs:84-89`、`boot_gateway.yaml:37-64` | buckyos `node_rtcp` 配 `inbound_admission: {anonymous: reject, named_min_relation: known_owner}`；process-chain 增加规则：外 Zone 来源（`source_zone_did != 本 Zone`）只允许 `dest_port` 为 http stack 端口（80），其余 reject；3184 ACL 保持。实测 `known_owner` 下设备文档不在本地缓存时的退路（`not_allow_request`），必要时允许对 verify-hub 端口的首连触发一次受限解析。 |
| P1-4 受保护头 | `buckyos-base/src/buckyos-http-server/src/lib.rs:161-169`（StreamInfo）、`cyfs-gateway-lib/src/stack/rtcp_stack.rs:452-464`、`boot_gateway.yaml` http 块 | StreamInfo 加 `source_device_did`、`source_identity_trust`；rtcp→http 交接带过去；http stack 在 RTCP 来源上 `map-add REQ X-Cyfs-Rtcp-Device`，所有入口先 `map-remove` 该头与 `cyfs-original-user`。`cyfs-original-user` 在 P3 后不再被任何服务读取，可一并删。 |
| P1-5 隧道生命周期 | `rtcp.rs:5617-5664`（run loop）、`rtcp_stack.rs:1780-1859`（keep_tunnel 心跳） | 非 keep_tunnel 隧道加空闲超时（建议 10 min）与被动探活（空闲 60 s 后 ping）；超时关闭并从 map 移除。 |
| P1-6 via-sn URL 空 path | `node_daemon/src/boot.rs:301-317`、`scheduler/src/zone_route_builder.rs:196-215`、`tunnel.rs:168-174` | 实跑确认；若确不可用，改拼 `rtcp://<enc(rtcp://sn/<target>:port)>@<target>:port`。 |
| P1-7 删注释式策略 | `boot_gateway.yaml:60-64` | "默认只允许 zone 内中转"注释删除；若要限制中转来源，用 P1-3 的规则表达。 |
| P1-8 SOCKS5 本地鉴权 | `boot_gateway.yaml:89-129`、`cyfs-socks` | 新增 `cross_zone_entry` SOCKS 入口（独立端口，建议 3186）：仅 `127.0.0.1` 绑定 + 用户名密码（密码由 node_daemon 生成写入本机 `etc/`，只系统服务可读）；App 容器网段不放行。现有 3185 executor 入口不动。 |
| P1-9 跨 Zone 出口经网关 | `scheduler/src/zone_route_builder.rs:653-782` | 非网关节点的跨 Zone 目标自动生成 via-zone-gateway 嵌套 URL；CrossZoneClient 不关心本机是否网关。 |

### P2 CrossZoneClient（依赖 P0-3、P1-8）

| 任务 | 落点 | 要点 |
|---|---|---|
| P2-1 客户端 | `buckyos-api/src/cross_zone_client.rs`（新）；删 `runtime.rs:2880-2931` 注释掉的 rtcp 选路 | API：`call(target_zone_did, service_id, principal: Principal, kRPC req)`；`Principal` = {user_did | agent_did, app_id?}。内部：解析 → 传输探测 → 登录 → 缓存 → 调用 → 刷新。 |
| P2-2 传输状态 | 同上 | 按 target zone 维护 {RtcpOk, HttpsFallback, Unreachable, Probing}；RTCP 失败进入 HttpsFallback 后每 5 min 重探一次 RTCP；两者都失败返回可重试错误。网络超时的写请求不自动重放（方案 §15.4），由调用方按 msg id 幂等重试。 |
| P2-3 Token 缓存 | 同上 | 键 (target_zone_did, principal_did, app_id)；进程内存；access 过期前 60 s 用 refresh 换；refresh 失败或 401 → 重新登录一次；仍失败 → 认证错误（不换传输）。 |
| P2-4 断言签发 | 同上 | 用本节点设备钥（msg-center 现有 `zone_signer`、HomeStation `Signer` 统一改用 runtime 提供的 device signer）。 |

### P3 业务接入（依赖 P0、P2）

| 任务 | 落点 | 要点 |
|---|---|---|
| P3-1 msg-center 投递与群同步 | `cyfs_dispatch.rs:613-756`、`group_sync.rs:71-87`、`group_http.rs:108-153` | 出站改走 CrossZoneClient；入站 `/inbox`、群 GET/PUT 只认 Bearer（External, scope）＋ `msg.from`/actor 必须等于 token sub 或其 custody 范围；删消息 JWT 作凭据与 `cyfs-group-request` proof 分支。消息本身仍是签名对象（内容完整性），只是不再当认证凭据。配置路由 `outgoing.authorization` 保留。 |
| P3-2 HomeStation | `pull.rs:32-52`、`auth.rs:180-215`、`delivery.rs:205-214` | 拉取与投递改走 CrossZoneClient；`DID <proof>` 头删除（断言格式并入 P0-3）；入站只认 Bearer External。 |
| P3-3 外部身份 | 各服务 External 处理表（review §1 "External 会话"行） | msg-center notify、system-config 保持拒绝；msg-center inbox/group、homestation 按 scope 放行；RBAC 不加 External 角色。 |
| P3-4 Agent runtime | `lib_opendan` 的跨 Zone 调用点（#689 相关） | 附件跨 Zone 读改用 CrossZoneClient 取 Token 后走对象 GET（#689 §5 的"统一认证层"即本文 P0）。 |
| P3-5 方案稿回写 | `BuckyOS 跨Zone通信方案思考.md` §14.2、review §4 的文字修订 | 把"待核"改为已核实，端口/SOCKS5/准入三处按现状改写。 |

### P4 后排

- 反向建连：verify-hub 登录成功后向本机网关投递"候选对端"事件，网关异步尝试 RTCP；目标地址只取 DID 文档解析结果（方案 §8.4、§15.2）。
- Relay 发现：`DeviceDocument` 增 `relay` 字段的提案；对接 cyfs-sn `RelayNode`/`check_relay_admission`（目前零调用）。
- 禁用 RTCP 的显式表示（方案 §5.5）。

## 5. 验收（ssotest 三 Zone VM 组：SN + alice/bob/charlie）

| 编号 | 场景 | 预期 |
|---|---|---|
| V-1 | bob → alice 私聊，alice RTCP 可达 | 第一条消息触发登录，后续复用 Token；alice 网关日志见 RTCP 来源头；msg-center 入站 Bearer External |
| V-2 | 同上，iptables 封 alice 2980 | 自动降级 HTTPS；登录仍成功；封禁解除后 ≤5 min 切回 RTCP |
| V-3 | 群同步 charlie(host) ↔ alice/bob | GET/PUT 均 Bearer，无 proof 头 |
| V-4 | HomeStation alice 拉 bob feed、评论回投 | 过；伪造 `did:dev:<x>#main_key` 签的断言被 401 |
| V-5 | Token 到期 | access 过期自动 refresh；refresh 过期重新登录；30 d 封顶由单测覆盖 |
| V-6 | 裸 did:dev 对端（无 device_doc_jwt）连 alice | 被拒（P1-2） |
| V-7 | 外 Zone 设备 RTCP 连 alice 的 3184 | 被拒（P1-3） |
| V-8 | App 容器内进程连本机 3186 | 被拒（P1-8） |
| V-9 | HTTPS 请求自带 `X-Cyfs-Rtcp-Device` | 网关清掉，verify-hub 当作无 RTCP 上下文处理 |
| V-10 | alice 的 Token 发给 charlie | 401（zone claim 不符） |
| V-11 | 三处 custody 对照单测 | 同输入同结果；Zone 不可达可重试 |
| V-12 | 全新安装 DV | 三 Zone 重装后 V-1～V-4 全过 |

## 6. 需冻结的接口（P0 工程师冻结后写入 `doc/`）

### 6.1 跨 Zone 登录断言

JWT，EdDSA。header `kid=<device did>#<key id>`。claims：

| 字段 | 含义 |
|---|---|
| `iss` | 请求主体 DID（用户或 Agent）。custody 校验的 author |
| `sub` | 同 `iss` |
| `dev` | 签名设备 DID（与 kid 一致，便于不解 header 读取） |
| `aud` | Target Zone DID |
| `app` | 可选，调用应用 AppDID（hostname 形）；进 token 的 app 维度 |
| `target` | 请求的 Target 服务 id（`system:<service>`），决定 token target |
| `jti`、`iat`、`exp` | exp − iat ≤ 60 s |

### 6.2 verify-hub `login_by_cross_zone_assertion`

入参：assertion、requested scopes。出参：现有 token pair 结构。错误码：`assertion_invalid`、`custody_rejected`、`zone_unavailable`（可重试）、`rtcp_identity_mismatch`、`target_not_allowed`。

### 6.3 受保护头

`X-Cyfs-Rtcp-Device: <device did>`、`X-Cyfs-Rtcp-Trust: <identity_trust>`。只由网关在 RTCP 来源流上写；其他入口 remove。

### 6.4 CrossZoneClient 错误分类

`Transport(Retryable)`、`Auth(Fatal)`、`Target(业务错误透传)`。

## 7. 坑与注意

- RTCP 每条数据流是一条新 TCP，由隧道发起方拨号：Target 作为被连方给 Source 回流时走 ROpen，不需要 Source 可达。但两端都 NAT 时正反向都不通，只能 Relay（P4）。
- `verify_trusted_jwt` 按 `iss` 取钥、忽略 `kid`；P0-4 的新方法不能复用它，必须走 P0-1。
- did:dev/did:key 没有文档，断言 kid 若为 did:dev 只能得到 KeyDid 级身份，custody 必然失败；最小实现对端必须有可解析的设备文档。
- `known_owner` 准入下对面做离线验证，文档不在本地 locker/缓存时会落到 `anonymous` 规则；P1-3 要实测，不能假设。
- msg-center 现有 `verify_trusted_session_token` 拒绝 External；P0-6 没做前 P3-1 的入站会全 401。
- 删 `cyfs-original-user` 前确认 cyfs-ndn `cyfs_http.rs:331` 与 cyfs-gateway `named_inbox_cache_server.rs:609` 的使用方是否仍需要。

## 8. 实施记录（2026-10-30，未提交）

冻结契约：[跨Zone登录断言契约](../doc/arch/verify-hub/跨Zone登录断言契约.md)（P0-3）。四仓改动均未提交：buckyos、cyfs-gateway、buckyos-base（仅 `StreamInfo`）、buckyos-websdk（仅 `hybrid_sso.ts` 类型镜像）。

### 8.1 落点

| 任务 | 落点 | 说明 |
|---|---|---|
| P0-1/P0-3 | `buckyos-api/src/cross_zone_auth.rs` | `CustodyResolver`（`NameServiceResolver`、`StaticCustodyResolver`）、`resolve_signer_key`、`check_custody`（`Signer`/`ListedInDoc`/`SameZone`；本地托管提示只增加放行途径）、`canonical_zone_of`、`zone_hostname`、`device_zone_of`、`DeviceSigner`、`sign_cross_zone_assertion`、`verify_signed_assertion`、`CrossZoneLoginError` 五个 wire 码、受保护头名、target 白名单、`custody_cases` 共享对照表 |
| P0-4 | verify-hub `handle_login_by_cross_zone_assertion`；`verify_hub_client.rs` `CrossZoneLoginRequest` | 公开方法；`aud` 认本 Zone DID 或 `did:web:<ZoneDocument hostname>`；jti 重放缓存；`X-Cyfs-Rtcp-Device` 由 `serve_request` 读出，要求与断言设备（无 Zone 时主体 canonical Zone）同 Zone |
| P0-5 | `hybrid_sso.rs` `AuthMethod::CrossZoneAssertion`、`SessionScope::MsgInbox`；verify-hub `FamilyClaims::cross_zone`（`src_device`/`src_app`） | target 白名单：`system:msg-center` → `msg_inbox`+`msg_group`，`system:homestation` → `homestation`。未新增 `msg_group_sync`/`homestation_pull`：群同步与 HomeStation 复用联合登录 External 的 scope（同一 DID 的成员/读者能力），只有私人 inbox 投递是跨 Zone 专用 scope。Limited 账号 ceiling 不含 `msg_inbox` |
| P0-5/P0-6 | verify-hub `sign_session_token` 与自身服务 token 都写 `zone` claim；`runtime.rs` `verify_session_signature` 拒绝他 Zone token | 带 `zone` 的 token 必须与本 Zone 同主机名 |
| P2 | `buckyos-api/src/cross_zone_client.rs` | `request`/`request_to`/`request_host`（HTTP，除 401 外原样返回）、`call`（kRPC）；传输状态 `Probing/RtcpOk/HttpsFallback/Unreachable`，RTCP 失败 5 min 后重探；只有连不上（含 SOCKS 层拒绝）或可重放的 GET 才换传输；Token 缓存键（Zone 主机、服务、主体、App），access 剩 60 s 即 refresh，401/refresh 失败重登一次；`runtime.device_signer()`；删除 runtime 注释掉的 rtcp 选路；buckyos-api 的 reqwest 加 `socks` feature |
| P0-2/P3-1 | msg-center `cyfs_dispatch.rs`、`group_http.rs`、`group_sync.rs`、`message_hub.rs`、`network_directory.rs`（`ZoneSignerSlot` 改存 `DeviceSigner`，新增 `CustodySlot`/`CrossZoneSlot`）、`main.rs`；删 `group_auth.rs` | 入站 inbox 只认 Bearer（`msg_inbox` scope，`msg.from == sub`，External 发送者按 sender 限速）；群 HTTP 只认 Bearer，删 proof 头与 `cyfs-original-user` 要求；出站 auto route = canonical Zone → ZoneDocument hostname，经 CrossZoneClient 以 `msg.from` 登录投递；群绑定经 `request_host`；配置路由 `outgoing.authorization`/`joined_groups.authorization` 保留；消息 JWT 仍验签，custody 走库 |
| P0-2/P3-2 | HomeStation `directory.rs`（`NameDirectory` 的 `zone_of`/`signer_key`/`authorizes` 走库；`StaticDirectory` 以文档形状实现 `CustodyResolver`）、`pull.rs`、`delivery.rs`、`auth.rs`（删 reader proof）、`http.rs`（inbox PUT 要求本 Zone 会话；standalone 的 `/kapi/verify-hub`）、新文件 `standalone_hub.rs`、`lib.rs`（`Station.cross_zone`）、`main.rs`、`standalone.rs` | 系统模式本 Zone 主机名映射 `http://127.0.0.1:3180`（网关同时承载 verify-hub 与 `/home/`）；standalone 模式由替身 verify-hub 用同一断言校验签 Zone 钥 JWT（契约 §8）；inbox 不要求调用者＝对象发布者（对象各自带签名，转发是既有语义） |
| P3-3 | — | 由构造满足：msg-center notify 等 kRPC 方法对受限会话无 scope；system-config 用 `verify_trusted_session_token` 拒绝受限会话；RBAC 未加 External 角色 |
| P3-4 | — | `lib_opendan` 当前没有任何跨 Zone 调用点；#689 的附件跨 Zone 读（对方收件人读已发送附件的业务规则）本身未实现。认证层（P0）与客户端（P2）已可用，留给 #689 实施时直接调用 `CrossZoneClient` |
| P3-5 | 方案稿就地补注（顶部回写说明 + §3/§5.3/§5.5/§5.7/§6.2/§6.3/§6.4/§6.5/§8.3/§8.5/§9.3/§9.4/§10.2/§10.4/§14.2） | |
| P1-1～P1-9 | cyfs-gateway `rtcp.rs`/`protocol.rs`/`tunnel.rs`/`rtcp_stack.rs`/`server.rs`/`http_server.rs`/`tcp.rs`、cyfs-socks `server.rs`；buckyos-base `StreamInfo`；buckyos `boot_gateway.yaml`、`node_daemon/boot.rs`、`scheduler/zone_route_builder.rs`、`doc/port_usage.md` | 见 8.2 偏差 |

### 8.2 与本文的偏差（P1）

1. 外 Zone 来源放行 80 与 443（不只 80）：SN 经 keep-tunnel 把 80/443 中转进 NAT 节点，拒 443 会断掉只经 SN 可达 Zone 的 HTTPS。80 进 node_gateway 并写受保护头，443 原样进 TLS stack（等同公网 HTTPS）。
2. “同 Zone”不比较 `source_zone_did`（`known_owner` 下外 Zone owner 可签一份自称本 Zone 的设备文档），改由 RTCP 按已验证 owner 与本节点锚点算 `REQ.source_zone_relation`（`same_zone`/`cross_zone`/`unknown`）。
3. 非网关节点不借网关的隧道，而是嵌套 bootstrap URL（`NODE_INFO.cross_zone_relay`）经网关 TCP 中转、端到端握手：Target 看到的 RTCP 对端就是签断言的节点。流量仍全部从网关出去，但不是“每 Zone 一条跨 Zone 隧道”。
4. P1-3 实测结论：`known_owner` 握手不做外部解析，首次见到的外 Zone 设备落到 `anonymous: reject`；同机任一进程解析过该设备文档（共享缓存 `$BUCKYOS_ROOT/local/did_docs`，如 verify-hub 处理 HTTPS 降级登录时）后下一次握手才放行。所以首次登录总走 HTTPS，RTCP 在 ≤5 min 的重探后可用；SOCKS 层的拒绝在 CrossZoneClient 里按传输失败处理（单测 `a_refusing_rtcp_entry_is_a_transport_failure_too`），这放宽了 §2 规则 5 在 RTCP 段的含义（准入拒绝与不可达在 SOCKS 层无法区分）。
5. P1-2 后 SN 自己发起的隧道（裸 did:dev）会被 WAN 节点拒绝；节点主动连 SN 的 keep-tunnel 中转不受影响。
6. 合并顺序：buckyos-base 必须先推（cyfs-gateway 用新 `StreamInfo`）；新网关二进制与新 `boot_gateway.yaml` 必须一起发（旧二进制不写 `source_zone_relation`，所有对端都会被当外 Zone）。本地构建用 `cyfs-gateway/src/.cargo/config.toml` 的 `[patch]` 指向本地 buckyos-base（未跟踪，DV 用，提交前删）。

### 8.3 单测

`cargo test -p buckyos-api -p verify_hub -p msg_center -p homestation -p node_daemon -p scheduler -- --test-threads=1` 全过（buckyos-api lib 341 + 集成测试、verify-hub 42、msg-center 207（1 ignored）、HomeStation lib 26 + 14 个集成测试文件、node_daemon 162（3 ignored）、scheduler 80 + 21）；`--test-threads=2` 时 scheduler 的 `agent_permissions_preserve_owner_limits_and_read_only_runtime_state` 偶发失败，单跑与单线程都过，与本次改动无关。`cargo check --workspace --tests` 无错。cyfs-gateway-lib `--lib` 485/486（`tls_stack::test_tls_stack_forward_err` 固定端口 9093 与另一测试冲突，单跑过，非本次引入）；cyfs-socks 6、cyfs-sn 239；buckyos-base `buckyos-http-server` 69；`src/test/test_boot_gatweay` YAML 用例 44/44。V-11 由 `custody_cases` 共享表在库、msg-center、HomeStation、verify-hub 四处各跑一遍。

### 8.4 DV（ssotest：SN + alice did:bns NAT／bob did:bns wan_dyn／charlie did:web portmap 2981，10-30）

做法：`build_for_vm_test.py` → `buckyos-devtest ssotest stop/update/start`。注意 devkit `update` 只同步 `bin/`，新 `boot_gateway.yaml` 要另行推到各 OOD 的 `/opt/buckyos/etc/`（旧文件留 `.bak-xzone`）。cyfs-gateway 用 `cyfs-gateway/src/.cargo/config.toml` 的 `[patch]` 指向本地 buckyos-base 构建。DV 脚本（未入库，在 job tmp）：`xzone_dv.mjs`（FROM→TO 私聊收发）、`group_xzone_dv.mjs`（charlie 建群、alice 入群同步双向发言）、`neg_dv.mjs`（宿主机用 alice 设备钥自签断言的反例）、`v78.sh`；HomeStation 用仓库里的 `test/object_url_dv/homestation_cross_zone_dv.mjs`。

| 编号 | 结果 |
|---|---|
| V-1 | 过（bns 与 did:web Zone 都走通 RTCP）：alice→bob，首条触发登录（bob verify-hub：`cross-zone login of did:bns:alice by did:bns:ood1.alice (SameZone)`），走 RTCP（bob 网关登记 ood1.alice 入站隧道，请求经 node_gateway）；后续消息复用 Token，2 s 内到达 |
| V-2 | 过：alice 入站 80/443/2980 被防火墙拒（等同 iptables 封锁），bob→alice 首次 RTCP 失败→HTTPS（经 SN 中转）登录成功；5 min 后重探，经 alice 先前拨出的隧道切回 RTCP（alice 网关见来源 `192.168.1.221:2980`） |
| V-3 | 过：charlie 建群邀请 alice，邀请卡经跨 Zone 投递到达，alice `accept_invitation`→active，alice 群消息到 charlie，charlie 回复经 `sync_joined` 到 alice；群 HTTP 全程 Bearer，无 proof 头 |
| V-4 | 过：HomeStation bob 发布、alice 拉取、评论投到 bob inbox（bob verify-hub：`... for system:homestation`）；伪造 `did:dev` 签 alice 的断言 → `custody_rejected` |
| V-5 | 单测覆盖（access 临期 refresh、401 重登、30 d 封顶沿用 External 家族逻辑）；DV 未等 15 min 过期 |
| V-6 | cyfs-gateway 单测覆盖（P1-2 端到端用例）；DV 无裸 did:dev RTCP 客户端，未做 |
| V-7 | 过：alice 设备经 RTCP 开到 bob 3184 的流被 bob 立即关闭 |
| V-8 | 过：3186 只监听 127.0.0.1（docker0 地址拒连）；无口令/错口令 SOCKS 握手被拒（curl rc 97）；带口令可达对方 verify-hub（HTTP 200） |
| V-9 | 过：经 HTTPS 自带 `X-Cyfs-Rtcp-Device: did:web:ood1.charlie.me` 的登录被正常签发（头被网关清掉；否则会是 `rtcp_identity_mismatch`） |
| V-10 | 过：bob 签给 alice 的 Token 在 charlie inbox → 401；无 Token → 401；同 Token 在 bob 通过认证（400 只因测试体无效） |
| 其他反例 | 断言重放 → `assertion_invalid`；`aud` 指向别的 Zone → `assertion_invalid`；`target=system:control-panel` → `target_not_allowed`；alice 设备替 charlie 签 → `custody_rejected` |
| V-11 | 单测四处同表（8.3） |
| V-12 | 未做：三 Zone 全新重装未执行（本轮只做 update；无持久格式需要迁移，verify-hub family claims 新增字段带默认值） |

DV 中发现并已修：
1. `did:bns` 用户按默认类型解析得到的是 Zone 文档，custody／canonical Zone 判成“无 Zone”→ 库改为先取 OwnerDocument 再取默认文档（`resolve_principal_document`，并加回归单测）。
2. 反向复用隧道上回来的流，网关以拨号时的 Zone DID（如 `did:bns:bob`）命名对端 → verify-hub 的 RTCP 一致性校验接受“对端名即期望 Zone”。
3. 老问题：msg-center `group_http` 未配置 `target_zone` 时以 `localhost` 作本 Zone，所有跨 Zone 群请求 404 `not-found` → 改为与 inbox 一致回退 runtime Zone。
4. 未修、记录：群刚创建时文档异步发布，成员在发布前 `accept_invitation` 会得到 `remote-group-not-found`，且 msg-center directory 把否定答案缓存 10 min（DV 脚本改为等文档可解析再接受）。登录刚完成时网关路由尚未登记的 `503 service_not_ready` 纯文本会让投递记一次 “invalid dispatch status body” 后重试成功（已加诊断日志）。

charlie（did:web Zone）方向的 RTCP 初测不通（走 HTTPS 降级，功能正常）：① `rtcp://charlie.me/` 把 Zone 文档当设备解析（“no exchange key”）——已在 cyfs-gateway 修；② charlie 首次见 bob 设备时 `known_owner` 离线校验失败——按设计，HTTPS 降级登录把文档写入共享缓存后放行。重建后复测：alice→charlie、bob→charlie 均走 RTCP 登录（charlie 网关 `Tunnel did:bns:ood1.bob accept ... OK`），三 Zone 两两私聊、群、HomeStation、反例全部复跑通过。见 8.5。

### 8.5 did:web Zone 的 RTCP 与 known_owner 首连

1. **已修（cyfs-gateway `rtcp.rs` `complete_zone_gateway_device`）**：Zone 目标只从 ZoneDocument 内嵌的 `devices` 取网关设备钥；did:web Zone（charlie.me）经 HTTP 发布的 ZoneDocument 只在 `oods` 里点名网关、不内嵌设备文档，于是“no exchange key”。现按 `did:<zone method>:<gateway>.<zone id>` 另行解析网关设备文档（id 一致、owner 等于 Zone owner，否则拒），钥与 `rtcp_port` 取自该设备，信任取两者较弱者；bns Zone（BNS 返回已内嵌）不变。单测 `zone_target_resolves_the_published_gateway_device`。
2. **按设计、未改**：入站握手只查本机缓存。charlie 从未验证过 bob 10-29 重建后的设备文档，故首连按 `anonymous: reject` 拒。一次 HTTPS 降级登录时 verify-hub 联网解析 bob 设备文档写入同机共享缓存 `/opt/buckyos/local/did_docs` 后，下一次握手放行（单测 `known_owner_admits_foreign_device_only_from_local_cache`）。首连即走 RTCP 需要 RTCP 预留的“有预算的未知 owner 解析”，违背“握手不出网”的规则，属 cyfs-gateway 设计变更，待定。
3. **另发现，10-30 已修（用户确认：只认 `versionId`/`versionTime`，不保留 `iat`）**：resolve-did 重构后 name-client 按冻结协议（buckyos-base `doc/http_did_resolver_api.md` §2、§7）发 `versionId`（exact）/`versionTime`（closest），值仍是 iat（“版本轴只有 iat”不变）；但服务端只读旧参数 `iat`——sys_config Zone 解析器把版本查询当 current 回答（以当前文档冒充历史版本），SN 忽略版本参数、带 `iat` 时回 400。改动：buckyos `sys_config_service/src/zone_did_resolver.rs`（`parse_identifiers_query`：两参数互斥、值须为整数，否则 400；任一存在即 501，信封里 `didDocumentMetadata.buckyos.historicalQuerySupported=false`）；cyfs-gateway `cyfs-sn`（`SnDidVersionQuery`、新错误类 `HistoricalQueryNotSupported` → 501 同形信封）；`bns-server` 删掉 `iat` 旧别名；`bns-webui` 客户端选项改为 `version: {exact}|{closest}`、demo server 同步。`iat` 现在是未知参数，按 current 查询处理。测试：sys_config resolver 25、cyfs-sn 241、bns-server 35+4+12、bns-webui vitest 245 全过。已部署 ssotest 验证：三台 OOD 的 Zone 解析器（3180）与 SN（`sn.devtests.org`/`web3.devtests.org`）对 `versionId`/`versionTime` 都回 501 信封、两者同给或非整数回 400、`iat` 按 current 处理；随后三 Zone 两两私聊 6 个方向、跨 Zone 群、HomeStation、反例全部复跑通过，网关无握手拒绝。
