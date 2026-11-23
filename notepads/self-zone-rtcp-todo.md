# 本 Zone 访问误走 RTCP（自连）TODO

日期：2026-11-18。问题分析基线：`buckyos@317534347`、`cyfs-gateway@f0ef92f2`。实现基于 `buckyos@ea263076c`、`cyfs-gateway@402d5361`。Linux 验证结果见 §4，原 Windows 环境待复验。

## 0. 现象

DV Test 环境 lzctest（单 OOD，Windows）重启后，cyfs-gateway 日志出现对自己 Zone 的 RTCP 握手失败：

```
cyfs_gateway.35548.log
412-417  SOCKS 127.0.0.1:3186 ← 3 个连接，目标 lzctest.web3.buckyos.ai:80，PROXY rtcp://lzctest.web3.buckyos.ai/
424      resolved RTCP peer did:bns:lzctest as did:dev:lCl5… (trusted_zone_snapshot via zone-locker)
453      key confirmation to 45.76.64.86:2980 error: HelloAck read error: early eof
```

- `did:dev:lCl5…` 就是本机：404 行到 SN 的保活隧道 key 为 `did:dev:lCl5…_did:dev:eM53…`，前半段是本机设备。
- 拨号地址取自 DNS A 记录 `45.76.64.86`，是 SN。本机把"发给本机设备"的 Hello 发给了 SN，SN 不是这个设备，直接断开。

调用方是 homestation（`homestation.45616.log` 20-27 行）。它打开 3 个用户的 HomeStation 后，对 `http://lzctest.web3.buckyos.ai/kapi/verify-hub` 发起 3 次跨 Zone 登录，RTCP 全部失败。之后回退到本地 origin 成功，网关日志 1300-1301 行出现 `GET /home/jarvis/feed`、`GET /home/testa/feed`：zone owner `lzctest` 在拉同 Zone 用户的 feed。11:13:27 又重试了一轮。

功能本身没受影响（回退成功），代价是：RTCP 回退状态按 host 记录，每个 homestation 进程对本 Zone 每 5 分钟至少重试一轮（并发请求时会有多次）。每次都要做一次名字解析、连一次 SN、被 SN 拒一次，网关每次打 3 条 ERROR。

## 1. HomeStation 的现状和改法

### 1.1 现状

- **本 Zone 也走跨 Zone 协议，这是设计**：每个用户的 HomeStation 是独立主体。跨用户读取和投递都以本站 owner 身份，到目标 Zone 的 verify-hub 登录后再访问，这样被读方才能按读者身份做受众分级和准入。本 Zone 用户之间也一样（`homestation/src/main.rs:145-146` 的注释）。
- **调用点**：
  - follow 拉取：`Station::remote_get_as(.., as_owner = true)`（`pull.rs:56`）→ `cross_zone.request_host(zone, HOMESTATION_SERVICE, owner, ..)`。用于 changes/display feed、objects、chunks、namespace（`pull.rs:165,202,271,282,310`）。
  - 投递：`delivery.rs:219` → `request_host`。
- **两份"本 Zone 走本地"的配置，都管不到 RTCP**：
  - `main.rs:149-151`：`CrossZoneClient.with_origins({zone: "http://127.0.0.1:3180"})`，只影响 HTTPS 这条传输。
  - `main.rs:189-191`：`NameDirectory` 的 origins `{zone: "http://127.0.0.1:4130"}`，只用于匿名读取和 locate。`remote_get_as` 在 `as_owner` 分支直接忽略 `origin` 参数，`delivery.rs:203` 也是 `let _ = origin;`。
- **`CrossZoneClient::transport_order`**（`buckyos-api/src/cross_zone_client.rs:530-543`）：只要配置了 SOCKS 入口，对任何 host 都先 RTCP 再 HTTPS，不区分本 Zone。RTCP 失败后该 host 记为 `HttpsFallback`，300 秒（`RTCP_REPROBE_INTERVAL`）后再试 RTCP。
- **网关 `cross_zone_socks` hook**（`rootfs/etc/boot_gateway.yaml:171-177`）：只检查目标是域名、端口是 80，不认识本 Zone，照样转成 `PROXY rtcp://<本 zone host>/`。

### 1.2 改法

- [x] **`CrossZoneClient` 认识本 Zone，本 Zone 只走 HTTPS，且走本地 origin。**
  - `from_runtime` 优先取 ZoneDocument 公布的主机名（与 HomeStation 及自定义 bridge 一致），缺省时从 `runtime.zone_id.to_host_name()` 推导。本地 origin 默认 `http://127.0.0.1:<node gateway 端口>`（`DEFAULT_NODE_GATEWAY_PORT` = 3180，runtime 里有 `node_gateway_port`）。
  - 增加 `with_local_zone(host, origin)`，供 `new` 构造的调用方（standalone、测试）显式设置。
  - `transport_order` 对本 Zone host 直接返回 `[Https]`，`record` 不为本 Zone 维护 RTCP 回退状态。
  - 只针对本 Zone，不推广到"所有声明了 origin 的 host"：`settings.peers` 和 msg_center 的 `zone_origins` 里是真实的远端 Zone，它们仍应先试 RTCP。
- [x] **homestation `main.rs:149-150` 手工插入本 Zone origin 的代码**：已删除。服务模式只传入 peers，standalone 显式传入本 Zone 的监听入口或目录指定的 origin。
- [x] **（可选，纵深防御）网关 `cross_zone_socks` hook 拒绝本 Zone**：目标为 `$NODE_INFO.this_zone_host` 或 `*.$NODE_INFO.this_zone_host` 时返回 `REJECT`。SOCKS 回 `HostUnreachable`，reqwest 视为连接错误，立即回退 HTTPS，不会再发起名字解析和拨号。`NODE_INFO.this_zone_host` 已在 `node_gateway` 块里使用（`boot_gateway.yaml:212`）。
- [x] **确认 msg_center 不受负面影响**：msg_center 也是 `from_runtime` 构造（`msg_center/src/main.rs:1452`）。本地收件人已在 `cyfs_dispatch.rs` 内部按 `is_local_recipient` 处理；`group_sync` 的成员会话和 `remote_attachment` 的取票均使用客户端的认证请求，未要求 RTCP 来源头。

### 1.3 测试与验收

- [x] `cross_zone_client.rs` 单测（参照现有的 `client(origin, socks)` 和 `closed_port()`）：
  - SOCKS 地址指向一个会记录连接的监听端口；`request_host(<本 zone>)` 不产生 SOCKS 连接，直接到本地 origin；
  - 远端 Zone 仍然先 RTCP；
  - `transport_state(<本 zone>)` 不会进入 `HttpsFallback`。
- [x] Linux `test.buckyos.io` 服务模式重启验收。结果见 §4。
- [ ] Windows `lzctest` DV 验收：重启后 homestation 日志不再出现 `cross-zone Rtcp to <本 zone> failed`，网关日志不再有发往本 Zone host 的 SOCKS 连接，同 Zone 用户的 feed 拉取和投递正常。

## 2. RTCP stack 直接拒绝自己连自己

### 2.1 现状

- **出站不检查目标是否为自己**：`RTcp::create_tunnel`（`cyfs-gateway-lib/src/rtcp/rtcp.rs:3663`）解析对端身份（3687 `resolve_remote_tunnel_identity`）得到 `canonical_dev_did`（3694），然后查复用、抢 `create_flights`（3740）、按地址候选拨号。全程不和 `this_device_dev_did` 比较。这次是因为 DNS 指向 SN，自连变成了"发给 SN 的错误 Hello"；如果候选地址是本机自己的 IP，就会真的连上自己的 2980。
- **失败不缓存**：3 个并发请求各自完整走了一遍解析和拨号。`create_flights` 只串行化，不缓存失败结果。
- **入站也不检查**：`on_new_tunnel`（3025）的 I1 步骤①-③做签名绑定和候选身份的预过滤，不检查发起方是否就是本机设备。
- **错误层层放大**：上层把这类失败当普通连接错误，每次打 3 条 ERROR（`tunnel_mgr.rs:361`、`tunnel_builder.rs:41`、`socks5.rs:350`）。

### 2.2 改法

- [x] **出站拒绝**。`create_tunnel` 在共享的 `resolve_remote_tunnel_identity` 中比较解析所得的 `canonical_dev_did` 与本机身份。相等时立即返回 `TunnelError::SelfTunnel`。
  - 比较的是规范化后的 did:dev，所以 `did:bns:<zone>`、`did:bns:ood1.<zone>`、`did:dev:<self>` 以及带 bootstrap 的 `<relay>@<self>` 形式都能覆盖。
  - 不发起任何连接，也不登记隧道。
- [x] **入站拒绝**：`on_new_tunnel` 在 I1 步骤③（`parse_source_candidate` 之后、possession proof 之前，3074 附近）从候选公钥得到发起方的 did:dev，等于 `this_device_dev_did` 时拒绝。这一步只做拒绝、不访问 resolver，符合 I1"证明前只能拒绝"的约束。主要用于兜住经中转或回环绕回自己的情况。
- [x] **错误类型与日志**。隧道管理器及 bootstrap 路径保留 `SelfTunnel`，SOCKS 将自连记录为 debug 并返回 `HostUnreachable`。
- [x] **其他入口**。`probe_url` 计算隧道键时经过同一身份检查，对自连返回 `Unreachable`。keep_tunnel 调用该探测入口。

### 2.3 测试

- [x] `create_tunnel` 目标是本机 did:dev，以及目标 DID 解析到本机 did:dev（mock resolver）时，都返回自连错误，且没有发生任何 TCP connect。
- [x] 带 bootstrap 的 `<relay>@<self>` 同样被拒，且没有打开 bootstrap 流。
- [x] 入站：用本机 key 签发的 Hello 被 `on_new_tunnel` 拒绝。
- [x] 回归：到其他设备的隧道（同 Zone 的其他节点、SN，以及同一物理机上容器或 VM 里的其他 stack）不受影响。

### 2.4 自连的判定标准（已确认）

- 一个设备就是一个身份：同一个设备 key 不会对应两个 RTCP stack，所以"自连"只按规范化 did:dev 判定，不需要带端口，也不做成可配置。
- 一台物理机可以通过隔离的容器或 VM 运行多个 RTCP stack，但每个都是独立设备、有各自的 key 和 did:dev，彼此互连是正常的，不属于自连。
- 因此判定**不能用地址**（回环地址、本机 IP、相同宿主地址加端口映射等）。容器和 VM 可能与宿主共用地址，按地址判定会误伤它们之间的合法隧道。
- 测试补充：同一物理机上、另一个隔离环境里的 stack（不同 device key，可共用宿主地址并做端口映射）之间建隧道，不受影响。

## 3. 不在本 TODO 范围

- SN 收到目标不是自己的 Hello 时直接断开、不回错误码，客户端只能猜测原因（`likely because it rejected the Hello or uses an incompatible RTCP handshake`）。可以另立项让 SN 回一个明确的结果码。

## 4. 本次验证

- `buckyos-api` 库测试通过 413 项，3 项由原测试标记忽略。新增用例覆盖本 Zone 登录与业务请求的 SOCKS 连接数为零、远端配置 origin 后先试 RTCP，以及自定义 BNS 主机名和非默认节点网关端口。
- HomeStation 的 `zone`、`network` 测试通过 7 项。msg_center 的远端附件测试通过 25 项。远端群组的旧邀请恢复用例首次失败，完整复跑通过 18 项。
- 网关规则测试通过 48 项，含网关节点和经中转的普通节点。SOCKS 测试通过 7 项，覆盖自连返回 `HostUnreachable`。隧道管理器和保活测试通过 27 项。RTCP 模块通过 91 项，另外 1 项在独立启动 BNS fixture 后通过。`cargo check -p cyfs_gateway` 通过。
- Linux 隔离环境中重启 HomeStation standalone 后，两个本 Zone 用户完成受限 feed 拉取和 inbox 投递。实际网关记录登录、GET 和 PUT 成功。进程连接跟踪显示访问了节点网关，访问 3186 和 2980 的连接数均为零。该环境使用 standalone 的 verify-hub。
- Linux 运行中的开发 Zone 另用新编译的 `CrossZoneClient::from_runtime` 和临时读者身份，通过真实 verify-hub 登录并读取 `/home/devtest/feed`，返回 HTTP 200。连接跟踪只记录了 `127.0.0.1:3180`。
- 在运行中的 Linux `test.buckyos.io` 临时替换并重启 HomeStation。切换前的 325 秒内有 1 次本 Zone SOCKS 请求和 1 次 RTCP 失败。新版就绪后的 325 秒内有 3 次 SameZone 登录、23 次经节点网关的 feed GET、1 次 inbox PUT，本 Zone SOCKS 请求和 RTCP 失败均为 0。第 307 秒再次主动同步已有订阅成功。原发送者幂等重投既有关注对象得到 HTTP 200 和 `accepted`，回执条数与时间戳均保持一致。验收后已恢复原二进制，HomeStation 经网关返回 HTTP 200。node-daemon 和网关一直运行。
- 两个 Docker 容器使用独立设备钥，经同一宿主 IP 上的不同端口映射完成 RTCP 握手和 ping。两端分别连接自己的 DID 时均返回 `SelfTunnel`。
