# 本 Zone 访问误走 RTCP（自连）TODO

日期：2026-11-18。源码基线：`buckyos@317534347`、`cyfs-gateway@f0ef92f2`。状态：待 review，未改代码。

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

- [ ] **`CrossZoneClient` 认识本 Zone，本 Zone 只走 HTTPS，且走本地 origin。**
  - `from_runtime` 从 `runtime.zone_id.to_host_name()` 取本 Zone host（msg_center 的 `local_zone_host()` 已这样取）。本地 origin 默认 `http://127.0.0.1:<node gateway 端口>`（`DEFAULT_NODE_GATEWAY_PORT` = 3180，runtime 里有 `node_gateway_port`）。
  - 增加 `with_local_zone(host, origin)`，供 `new` 构造的调用方（standalone、测试）显式设置。
  - `transport_order` 对本 Zone host 直接返回 `[Https]`，`record` 不为本 Zone 维护 RTCP 回退状态。
  - 只针对本 Zone，不推广到"所有声明了 origin 的 host"：`settings.peers` 和 msg_center 的 `zone_origins` 里是真实的远端 Zone，它们仍应先试 RTCP。
- [ ] **homestation `main.rs:149-150` 手工插入本 Zone origin 的代码**：上一条落地后变成默认行为，可以删掉，或保留作为显式覆盖。
- [ ] **（可选，纵深防御）网关 `cross_zone_socks` hook 拒绝本 Zone**：目标为 `$NODE_INFO.this_zone_host` 或 `*.$NODE_INFO.this_zone_host` 时返回 `REJECT`。SOCKS 回 `HostUnreachable`，reqwest 视为连接错误，立即回退 HTTPS，不会再发起名字解析和拨号。`NODE_INFO.this_zone_host` 已在 `node_gateway` 块里使用（`boot_gateway.yaml:212`）。
- [ ] **确认 msg_center 不受负面影响**：msg_center 也是 `from_runtime` 构造（`msg_center/src/main.rs:1452`）。本地收件人已在 `cyfs_dispatch.rs` 内部按 `is_local_recipient` 处理；需确认 `group_sync`、`remote_attachment` 在目标是本 Zone 时没有依赖 RTCP（预期没有）。

### 1.3 测试与验收

- [ ] `cross_zone_client.rs` 单测（参照现有的 `client(origin, socks)` 和 `closed_port()`）：
  - SOCKS 地址指向一个会记录连接的监听端口；`request_host(<本 zone>)` 不产生 SOCKS 连接，直接到本地 origin；
  - 远端 Zone 仍然先 RTCP；
  - `transport_state(<本 zone>)` 不会进入 `HttpsFallback`。
- [ ] DV 验收：重启后 homestation 日志不再出现 `cross-zone Rtcp to <本 zone> failed`，网关日志不再有发往本 Zone host 的 SOCKS 连接，同 Zone 用户的 feed 拉取和投递正常。

## 2. RTCP stack 直接拒绝自己连自己

### 2.1 现状

- **出站不检查目标是否为自己**：`RTcp::create_tunnel`（`cyfs-gateway-lib/src/rtcp/rtcp.rs:3663`）解析对端身份（3687 `resolve_remote_tunnel_identity`）得到 `canonical_dev_did`（3694），然后查复用、抢 `create_flights`（3740）、按地址候选拨号。全程不和 `this_device_dev_did` 比较。这次是因为 DNS 指向 SN，自连变成了"发给 SN 的错误 Hello"；如果候选地址是本机自己的 IP，就会真的连上自己的 2980。
- **失败不缓存**：3 个并发请求各自完整走了一遍解析和拨号。`create_flights` 只串行化，不缓存失败结果。
- **入站也不检查**：`on_new_tunnel`（3025）的 I1 步骤①-③做签名绑定和候选身份的预过滤，不检查发起方是否就是本机设备。
- **错误层层放大**：上层把这类失败当普通连接错误，每次打 3 条 ERROR（`tunnel_mgr.rs:361`、`tunnel_builder.rs:41`、`socks5.rs:350`）。

### 2.2 改法

- [ ] **出站拒绝**：`create_tunnel` 在 3694 得到 `remote_dev_did` 之后、查复用和拨号之前，`remote_dev_did == self.this_device_dev_did` 时立即返回错误，例如 `rtcp target <stack id> resolves to this device (<did:dev>); self tunnel is not supported`。
  - 比较的是规范化后的 did:dev，所以 `did:bns:<zone>`、`did:bns:ood1.<zone>`、`did:dev:<self>` 以及带 bootstrap 的 `<relay>@<self>` 形式都能覆盖。
  - 不发起任何连接，也不登记隧道。
- [ ] **入站拒绝**：`on_new_tunnel` 在 I1 步骤③（`parse_source_candidate` 之后、possession proof 之前，3074 附近）从候选公钥得到发起方的 did:dev，等于 `this_device_dev_did` 时拒绝。这一步只做拒绝、不访问 resolver，符合 I1"证明前只能拒绝"的约束。主要用于兜住经中转或回环绕回自己的情况。
- [ ] **错误类型与日志**：自连拒绝用可区分的错误（新增 `TunnelError` 变体，或在 `ConnectError` 里约定前缀），让 tunnel_mgr、socks 等上层识别后降到 info 或 debug，不再打成 ERROR。SOCKS 侧仍回 `GeneralFailure`，或者回 `HostUnreachable`；`CrossZoneClient` 都会把它当连接错误回退。
- [ ] **其他入口**：确认 `probe_url`、keep_tunnel 维护等也经过 `create_tunnel`，或在各自入口做同样的检查。

### 2.3 测试

- [ ] `create_tunnel` 目标是本机 did:dev，以及目标 DID 解析到本机 did:dev（mock resolver）时，都返回自连错误，且没有发生任何 TCP connect。
- [ ] 带 bootstrap 的 `<relay>@<self>` 同样被拒，且没有打开 bootstrap 流。
- [ ] 入站：用本机 key 签发的 Hello 被 `on_new_tunnel` 拒绝。
- [ ] 回归：到其他设备的隧道（同 Zone 的其他节点、SN，以及同一物理机上容器或 VM 里的其他 stack）不受影响。

### 2.4 自连的判定标准（已确认）

- 一个设备就是一个身份：同一个设备 key 不会对应两个 RTCP stack，所以"自连"只按规范化 did:dev 判定，不需要带端口，也不做成可配置。
- 一台物理机可以通过隔离的容器或 VM 运行多个 RTCP stack，但每个都是独立设备、有各自的 key 和 did:dev，彼此互连是正常的，不属于自连。
- 因此判定**不能用地址**（回环地址、本机 IP、相同宿主地址加端口映射等）。容器和 VM 可能与宿主共用地址，按地址判定会误伤它们之间的合法隧道。
- 测试补充：同一物理机上、另一个隔离环境里的 stack（不同 device key，可共用宿主地址并做端口映射）之间建隧道，不受影响。

## 3. 不在本 TODO 范围

- SN 收到目标不是自己的 Hello 时直接断开、不回错误码，客户端只能猜测原因（`likely because it rejected the Hello or uses an incompatible RTCP handshake`）。可以另立项让 SN 回一个明确的结果码。
