# 《BuckyOS 跨Zone通信方案思考》review

日期：2026-10-30。被评稿：[BuckyOS 跨Zone通信方案思考.md](BuckyOS%20跨Zone通信方案思考.md)（10-29 稿）。对照：[cross-zone-auth-overview.md](cross-zone-auth-overview.md)、[混合 SSO TODO](hybrid-sso-implementation-todo.md)、[#689 分析](issue-689-cross-zone-attachments-protocol-todo.md)。源码基线：`buckyos@688df1577`、`cyfs-gateway@1aa9edd`、`buckyos-base@35bac92`。本文只评审，不改方案稿、不改代码。

## 0. 总评

方向成立：Token 只由 Target Zone 自己签、业务服务只认本 Zone Token、RTCP 与 HTTPS 是两条传输而不是两套认证。这与 overview 建议 1（session token 不跨 Zone、不公开 verify-hub 公钥）一致，方案不是 overview 里的路径 C，而是把路径 A 的 custody 验证搬进 verify-hub、再复用混合 SSO 已有的 External 会话模型。

三个需要你拍板的分歧：

1. **一次性投递是否也要先换 Token。** 方案 §2.1 把"向 Inbox 投递一条消息"列为最重要场景，但 §7 要求它也先登录。现状 msg-center 用消息自身的设备签名 JWT 一步完成身份证明，接收方离线时发送方按 msg id 重试即可。先登录再投递变成两个往返加一份会话状态，对"最小实现对端"（§7.4）也更重。建议把 §12.2 的"业务可要求独立签名"从例外改成正式的第二种输入：**自认证请求**（签名信封或签名头）用于投递和一次性读取，**verify-hub 换 Token** 用于会话型访问。两者共用一份 custody 验证实现。
2. **custody 验证收口的位置。** overview 建议 2 收到 buckyos-api 库，方案收到 verify-hub 服务。两者不冲突：库是实现，verify-hub 是库的一个调用方；msg-center 入站、HomeStation reader proof 也调用同一个库。若只做 verify-hub 一处，现有三处重复代码不会消失，只会变四处。
3. **HTTPS 路径的来源证明（§8.3）不必留空。** HomeStation 已有 `Authorization: DID <proof>`（iss＝请求者 DID、aud＝目标、TTL 300s、设备钥签）。建议两条路径都用这一种签名头向 verify-hub 登录；RTCP 握手得到的设备身份只做一致性校验（签名头 kid 的设备 ＝ 隧道对端设备）。这样 verify-hub 只有一种登录输入，§6.5 的"清理再注入 Header"从必需降为加固项。

## 1. 已核实的现状（对应方案 §14.2 "待核"表）

| 方案条目 | 现状 | 依据 |
|---|---|---|
| RTCP 默认端口 2980 | 是。stack id 不带端口即 2980；监听端口由必填的 `bind` 决定；非默认端口可写在 stack id `did:port` 或 URL 主机名后 | `protocol.rs:57,108-160`；`rtcp_stack.rs:2351-2355` |
| 端口配置能力"已存在" | 一半。`DeviceDocument.rtcp_port` 字段存在，但 cyfs-gateway 的 RTCP 客户端从不读它，连接端口永远取自 URL；只有 buckyos scheduler 拼 Zone 内路由 URL 时用它 | `device.rs:120-121`；`rtcp.rs:3859`；`zone_route_builder.rs:181-186` |
| 发现数据位置 | `DeviceDocument` 有 `ips`（v4/v6 混放）、`net_id`、`rtcp_port`、`ddns_sn_url`；`ZoneDocument` 有 `sn`、内嵌 `devices`、`oods`；没有 relay 字段。RTCP 不用 `net_id`/`ddns_sn_url`。跨 Zone 目标无 DeviceInfo，地址只来自 `DeviceDocument.ips` 或 DNS NameInfo | `zone.rs:541-591`；`name_client.rs:619-797,953-996` |
| 外 Zone 设备文档能否解析 | 能。did:web 走 `/.well-known/device.json` → `/1.0/identifiers` → SN；did:bns 走 BNS → SN。did:dev/did:key 没有文档，解析直接报 InvalidDID | `web_provider.rs:238-339`；`resolve.rs:135-140` |
| 未配置时默认尝试 2980 | 现状比方案倾向更接近"未配置即不可连"：ips 为空且 DNS 无记录就不拨号；有记录则按默认 2980。差距只在读取 `rtcp_port` | `rtcp.rs:3839-3857` |
| RTCP 基于 TCP | 是，且仅 TCP。`rudp` 是 TCP 上的定长帧 | `rtcp.rs:1576,2630` |
| Tunnel 双向复用 | 是：入站 tunnel 登记在同一 map，可被反向复用；有 ROpen。**但每条数据流是一条新 TCP 连接，总由 tunnel 发起方拨号**，控制隧道不复用数据。无空闲超时，只有 `keep_tunnel` 列表内的对端有心跳 | `rtcp.rs:3279,5342-5432,5818-5926,6239-6270` |
| 中转模型 | 已是方案 §5.6 要的隧道级模型：中转表示为嵌套 bootstrap URL `rtcp://<enc(rtcp://relay/target:port)>@target:port`，中转节点只做 TCP splice，端到端握手穿透。buckyos 拼的 via-sn URL path 为空，`get_dest_info_from_url_path` 对空 path 报错（未实跑） | `protocol.rs:162-197`；`tunnel.rs:168-174`；`boot.rs:301-317` |
| "默认只允许 Zone 内中转" | **只是 YAML 注释**。`new_rtcp_stream` 不存在于代码；`node_rtcp` 默认块无条件 `forward`，只有 3184 端口查 `EXECUTOR_ACL` | `boot_gateway.yaml:56-64` |
| SN 的 RTCP 权限"较收敛" | 对。SN 入站 admission 为 `known_owner`，隧道/流钩子只 `accept` 不转发；`relay_mgr::check_relay_admission` 存在但零调用；SN-Relay.md 自述跨 Zone 准入未实现 | `web3_gateway.yaml:145-201`；`SN-Relay.md:595,619` |
| 入站设备准入 | 默认 `anonymous: reject` + `named_min_relation: same_zone`，外 Zone 命名设备默认被拒，buckyos 未改此默认。**漏洞样行为**：不带 `device_doc_jwt` 的对端直接以 `KeyDid` 放行，不受 `anonymous` 约束 | `rtcp.rs:84-89,2354-2363,2493-2522` |
| RTCP 身份传给 HTTP | **传不到**。`server <name>` 交接只传 `StreamInfo`，该结构无 DID 字段；没有注入 RTCP 身份的 process-chain 命令。HTTP 层有 `map-add REQ`/`map-remove REQ` 可改请求头；对客户端自带头无任何清理（X-Forwarded-For 追加、X-Real-IP 仅缺失时设） | `rtcp_stack.rs:452-464`；`buckyos-http-server/lib.rs:161-169`；`http_server.rs:1108-1135,1172` |
| 本地 SOCKS5 | 存在（cyfs-socks 组件，仅 TCPConnect）。buckyos 绑 `0.0.0.0:3185`，只按来源 IP `127.*`/`172.*` 放行，可选用户名密码；目标可写 DID 主机名 | `boot_gateway.yaml:89-118`；`socks5.rs:110-125` |
| 出站是否限制对端 | 不限制。发起方总带自己的 `device_doc_jwt`，被对面默认 same_zone 拒绝；跨 Zone 要对面改 `known_owner`/`any` | `rtcp.rs:1617-1624,3554-3984` |
| verify-hub 能否给外 Zone 身份签 Token | 只有浏览器联合登录一条路，且只允许 control-panel 调用。`login_by_jwt` 只信本地 users/devices/AgentSpec，按 `iss` 取钥、忽略 `kid`；从不解析外 Zone DID | `verify_hub main.rs:606-716,1349-1482,2148-2281` |
| External 会话 | 已有：`external: true`、scope 限 `msg_group/aiworkspace/homestation`、target 固定 control-panel、access 15 分钟、refresh 7 天、联合家族 30 天封顶、续期不向 canonical 复核。RBAC 无 External 角色，各服务自己读 claim，多数服务拒绝 | `hybrid_sso.rs:122-168`；`main.rs:38-41,2249-2258` |
| 业务侧 Token 验证 | `iss`、`kid` 必须是 `"verify-hub"`，钥只来自本 Zone `verify_hub_info`；**不校 aud、target、zone** | `runtime.rs:1836-1889` |
| 跨 Zone 客户端 | 没有。buckyos-api 只拼本 Zone URL，rtcp 选路代码注释掉并标 never entered；无按目标 Zone 的 token 缓存。msg-center/HomeStation 自己拼 `https://<zone>` 并自签 | `runtime.rs:2880-2931,2941-2979`；`cyfs_dispatch.rs:613-756` |
| `cyfs-original-user` | 网关不清理不注入；只有 msg-center 群路径读并与签名/会话比对 | `group_http.rs:108-153` |

## 2. 方案的设计问题

### 2.1 投递场景被置于例外（§2.1 vs §7 vs §12.2）

见总评 1。补一条现实约束：方案 §7.4 要求允许"最小实现对端"，而最小对端在现有栈里就是无文档的 did:dev（KeyDid）。它既没有 Zone 也没有 User，verify-hub 对它只能签设备级会话。若投递必须先登录，最小对端反而比带签名信封的投递更难做。

### 2.2 verify-hub 新增登录方式的具体落点

要让 §7 成立，verify-hub 需要一种新的登录输入：外 Zone 设备签的短 TTL 断言，附 custody 证明。要点：

- `verify_trusted_jwt` 现在按 `iss` 查本地钥、忽略 `kid`；路径 A 按 `kid` 定位设备文档。两者要统一到按 `kid` 解析设备文档、再验 `iss`（用户）与设备的 custody。
- custody 规则必须与 msg-center/HomeStation 同一份（"签名设备所在 Zone ＝ 用户 canonical Zone 即放行"，已是 known risk）。方案 §7.3 列的四层判断不能在 verify-hub 另写一套，否则 overview §5.1 的"三处重复"变四处。
- 签发的会话直接复用 External 模型，不要新造一类。需要扩的是：target 允许 App/service（现固定 control-panel）、claims 增加 `aud`/zone（现 `iss` 固定 `"verify-hub"` 无 Zone 标识，隔离全靠"只有本 Zone 钥能验"）、App 维度身份（现 AppInstanceId ＝ `<app_id>@<短 owner>`，外 Zone App 需要 `app@did` 形）。
- 联合会话续期不向 canonical 复核、30 天封顶，这个已拍板的 Q-04 应直接沿用到设备签名登录，方案 §9.3 不必重新讨论。

### 2.3 HTTPS 路径的来源证明（§8.3）

见总评 3。现成的 `DID <proof>` 头已绑定 aud、iss、TTL；补 `jti` 防重放和路径/方法绑定即可。这也回答了 overview 建议 3。

### 2.4 RTCP 身份注入（§6.5）的工作量被低估

方案把"清理再注入"当作 Gateway 的既有能力调整。现状是整条链都不存在：`StreamInfo` 无 DID 字段、rtcp→http 交接不传 REQ 变量、无注入命令、无清理。若采纳 2.3（签名头统一登录），RTCP 身份只需在网关层做准入和与签名头的一致性比对，注入可以后做。若坚持注入，至少要：扩 `StreamInfo`、rtcp_stack forward 到 http stack 时写受保护头、http 入口对非 RTCP 来源 `map-remove` 同名头。

### 2.5 入站准入（§6.4）要先关 KeyDid 的口子

默认策略下外 Zone 命名设备被拒，但无 `device_doc_jwt` 的裸 did:dev 直接放行。开放跨 Zone 时若把 `named_min_relation` 放到 `known_owner`/`any`，而 KeyDid 口子不动，等于匿名设备比有身份的设备更容易进来。方案 §6.4 应明确：跨 Zone 准入策略包含 KeyDid 的处理（拒绝、或仅允许到达 verify-hub 登录端口）。另外 `known_owner`/`any` 下对面只做离线验证（`not_allow_request`），文档不在本地缓存时会落到 `anonymous` 规则，这一点需要实测。

### 2.6 反向建连（§8.4）的执行者

verify-hub 没有 RTCP 栈，反向建连只能由网关做，需要 verify-hub→gateway 的触发接口（或 verify-hub 只记录"对方可达候选"，由网关异步尝试）。同时 Source 侧也要开放 Target 的入站（同 2.5）。§15.2 的 SSRF 边界要落到：反向建连目标只能是从 DID 文档解析出的地址，不能来自登录请求体。

数据流由 tunnel 发起方拨号这一点对 §8.4 不构成障碍：Target 作为发起方可达 Source 的监听口即可。但对"两端都在 NAT 后"的场景，反向建连和正向一样都不成立，只能走 Relay。

### 2.7 集群共享（§11）已有现成基础

scheduler 已为 Zone 内设备生成 `via-zone-gateway` 路由（嵌套 bootstrap 经网关 OOD 中转），这就是"共享实际连接能力"的现成实现。建议方案直接规定：**跨 Zone 出口统一经 Zone 网关设备**，其他节点经 Zone 内中转使用网关的隧道；不要每台设备各建跨 Zone 隧道。这同时回答了 §5.1 "哪个设备有资格代表 Zone"（ZoneDocument 的默认网关设备，RTCP 解析 Zone 目标时已取它的钥）。§11.2 的"共享可用性信息"则降为网关内部状态。

### 2.8 Relay（§5.6、§5.7）

隧道级中转已经是 RTCP 的实现模型，方案不用再论证。缺的是：发现数据里没有 relay 字段（现只有 `ZoneDocument.sn` 和 SN 分配的 `relay_sn`）；SN 侧 relay 准入零调用；buckyos via-sn URL 空 path 疑似不可用。方案 §5.7 "通用 Relay 独立于 SN"与 cyfs-sn 已有的 `RelayNode`/`sn_relay.yaml` 拆分部署方向一致，可引用。

### 2.9 Tunnel 生命周期

跨 Zone 隧道无空闲超时、非 `keep_tunnel` 对端无心跳：半开连接只在下次读写时发现。方案 §10.3 的传输状态机需要加"空闲回收"和"探活"两项，否则"RTCP 可用"状态会长期失真。

### 2.10 msg-center 与 verify-hub 的 Zone 主机解析不一致

msg-center `auto_route` 取 OwnerDocument `binded_zone_list[0]` 或 DID 自身主机名；verify-hub 联合登录取 OwnerDocument → canonical Zone → ZoneDocument hostname。方案 §5.4 要求"从 Zone DID 出发可信发现"，落地时应统一为后者，并由同一个库提供。

## 3. 与 overview 建议的关系

| overview 建议 | 方案态度 | 建议回写 |
|---|---|---|
| 1 路径 A 主线、token 不跨 Zone | 一致 | 方案 §7 明确"Target 签的是本 Zone External 会话，不是路径 C" |
| 2 custody 收口到 buckyos-api | 收到 verify-hub | 合并：库实现，verify-hub、msg-center、HomeStation 三个调用方 |
| 3 普通 HTTPS 签名头 | §8.3 留空 | 采纳为两条路径共同的登录输入（总评 3） |
| 4 rtcp 身份只做网关 ACL | §6.5 要注入 | 先做准入与一致性校验，注入后置（2.4） |
| 5 对照测试 | 未提 | §15 增加：三处 custody 同输入同结果，Zone 不可达可重试 |

## 4. 文字修订建议

- §3 术语 `cyfs-gateway` 行：SOCKS5 由 cyfs-socks 组件提供，挂在 TCP stack 上。
- §5.3：把"可配置端口从初始设计就存在"改为"字段存在（`DeviceDocument.rtcp_port`），RTCP 客户端尚未读取"。
- §5.5：现状已是"ips/DNS 有地址即按 2980 尝试"，方案倾向与现状一致，去掉"尚未定"。
- §5.7：补"relay_mgr 与 check_relay_admission 已有但零调用"。
- §6.2："绑定回环"改为现状 `0.0.0.0:3185` + 来源 IP 过滤，并指出 `172.*` 放行意味着任意 App 容器可用设备身份出站，这就是 §6.3 的空白。
- §6.4：补 KeyDid 放行与默认 same_zone 两条现状。
- §8.5 表"建连入口"行：加注"数据流每条一个新 TCP 连接、由隧道发起方拨号"。
- §9.3：直接引用混合 SSO Q-03/Q-04 的数值，不再写"尚未确定"。
- §14.2 表：按本文 §1 更新为已核实。

## 5. 10-30 用户裁决与回应

| 分歧 | 裁决 | 影响 |
|---|---|---|
| 1 投递是否先换 Token | **先换 Token，投递也走统一路径。** 聊天是反复投递，隧道与 Token 保持一段时间有效即可 | 签名并没有消失，只是从"每条消息"移到"登录断言"：首跳仍是设备签名 JWT + custody 证明，之后复用 Token。Token 寿命沿用 External（access 15 min / refresh 7 d / 30 d 封顶）；隧道要补空闲回收与探活（§2.9）。最小实现对端仍需能签登录断言 |
| 2 custody 收口位置 | **按建议：buckyos-api 库实现，verify-hub / msg-center / HomeStation 调用** | 三处重复在本轮合并，不新增第四处 |
| 3 HTTPS 路径来源证明 | 用户质疑："跨 Zone 协议按最小级设计，HTTPS 客户端填的 Authorization 不可信" | **待用户确认**，见下 |

### 5.1 对分歧 3 的回应

不可信的是**未签名**的头（`cyfs-original-user`、自报的 device header）。建议的 `Authorization: DID <proof>` 是发送方设备私钥签的 JWT，接收方从 DID 文档取公钥验签，并校 aud（目标 Zone+路径）、TTL、jti。信任根是 DID 文档里的设备公钥，与 RTCP 握手完全相同：RTCP 握手也是"对端用长期 Ed25519 钥签临时钥，接收方按 DID 文档验签"。两者区别只在信道绑定与重放防护，签名断言用 aud+jti+短 TTL 解决。

"按最小级设计"反而更支持签名断言：最小对端只需要一把钥和能签 JWT，不需要实现 RTCP 握手；HomeStation reader proof 与 msg-center 消息 JWT 今天就是这样跨 Zone 跑的。

如果分歧 3 的本意是 §8.3 的"反查"（Target verify-hub 回连 Source Zone 确认本次登录），那是用户级同意流程（联合登录用 code 交换做的事），对设备级/custody 级登录不需要；它还要求 Source Zone 有可达的 HTTPS 入口，比"DID 文档可解析"这一前提更强。

统一后 verify-hub 只有一种跨 Zone 登录输入：签名断言。RTCP 路径同样提交它，握手身份只做一致性校验（断言 kid 的设备 ＝ 隧道对端设备）并作为准入输入。

### 5.2 本轮一并修的老模块问题

用户原则：系统很少真跑跨 Zone，老模块问题被隐藏，真实需求来了一并修掉。按本文 §1/§2 归集：

1. RTCP 客户端读取 `DeviceDocument.rtcp_port`（§1 端口行）。
2. 关 KeyDid 口子：无 `device_doc_jwt` 的对端不得绕过 `anonymous` 策略（§2.5）。
3. 入站准入策略可配置到跨 Zone（`known_owner`/`any`），并验证离线解析不可用时的退路（§2.5）。
4. 跨 Zone 隧道空闲回收与探活（§2.9）。
5. via-sn 路由 URL 空 path 问题实跑确认并修（§1 中转行）。
6. 删除或实现 YAML 注释里的"Zone 内中转限制"，不留注释代替策略（§1）。
7. SOCKS5 本地调用者鉴权：不能只按 172.* 放行（§2 文字修订 §6.2）。
8. verify-hub 新增签名断言登录；`verify_trusted_jwt` 改按 `kid` 定位设备文档（§2.2）。
9. External 会话扩 target 到 App/service，claims 加 aud/zone；外 Zone App 身份表示（§2.2）。
10. 业务侧 Token 验证补 aud/zone 校验（§1 业务侧行）。
11. msg-center `auto_route` 的 Zone 主机解析改为 OwnerDocument → canonical Zone → ZoneDocument hostname，与 verify-hub 同一实现（§2.10）。
12. 跨 Zone 出口统一经 Zone 网关设备，复用 via-zone-gateway 路由（§2.7）。
13. 网关对受保护头的清理（即便先不注入 RTCP 身份，也要清客户端自带的 `cyfs-original-user` 等）（§2.4）。
14. 对照测试：三处 custody 同输入同结果，Zone 不可达可重试（§3 建议 5）。
