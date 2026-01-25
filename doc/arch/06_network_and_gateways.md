# 06. 网络与访问（cyfs-gateway / NodeGateway / ZoneGateway / SN）

BuckyOS 的网络设计默认假设：
- 多设备跨 NAT、多网段、网络拓扑经常变化；
- 不能依赖固定公网 IP 或稳定端口映射；
- “能连上”与“能安全地连上”同等重要；
- 内网/公网/跨网段访问路径要能被同一个运行时（buckyos-api-runtime）解释。

这章的目标不是复述所有网络实现细节，而是给出一个足够稳定的心智模型：
- 服务访问的默认入口在哪里（NodeGateway / ZoneGateway）；
- rtcp 在整个体系里“到底解决什么”；
- boot/multi-OOD 场景下，局域网 discovery 如何补位。

## 关键组件与职责边界（先分清楚谁在干什么）

### 1) cyfs-gateway（NodeGateway）：每台设备的网络内核
- 角色：把“访问某个 service”的请求，转换为合适的底层路径（直连 / rtcp / 中转），并在本机提供一致的 HTTP 入口。
- 结果：上层 app/service 可以尽量不关心 NAT、内网 IP、端口映射、证书等现实问题。

### 2) ZoneGateway：对外入口与暴露控制点
ZoneGateway 通常由 OOD 承担，但也可以是独立节点。
- 角色：承载公网访问（HTTP/HTTPS）与子域名路由；是“系统对外暴露策略”的控制点。
- 职责：证书/TLS 终止、外部请求转发、与 SN 协作等（具体取决于是否启用 SN）。

### 3) SN（Super Node）：可选但重要的公网协助
SN 提供的能力主要围绕“公网可达性”与“域名/证书”辅助：
- DDNS
- 证书挑战/协助
- 转发/中转协助（keep-tunnel）

### 4) rtcp：可信、常态化的跨 NAT 隧道
- 作用一：把 NAT 当成默认场景，提供“强身份 + 加密 + 可中转”的连接底座。
- 作用二：把大量“本来要每个服务自己处理的安全传输与身份校验”下沉到通道层（服务协议可以是明文 HTTP，但传输仍加密）。

参考：
- `new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`
- `new_doc/ref/doc/cyfs/rtcp.md`（旧文档；以代码为准）

## NodeGateway 与 127.0.0.1:3180 一致入口

buckyos-api-runtime 与 notepads 都强调一种最大兼容访问方式：
- `http://127.0.0.1:3180/kapi/<service_name>`

含义：
- 先访问本机 cyfs-gateway（NodeGateway），再由其转发到真正处理请求的节点/实例。
- 这本质上是把“访问入口”固定为本机 loopback，把“怎么到达目标节点/实例”的复杂性交给网关处理。

### 关键运行时字段/常量（理解可配置点）
对应代码：`src/kernel/buckyos-api/src/runtime.rs`
- `BuckyOSRuntime.node_gateway_port: u16`：运行时选择本机 NodeGateway 的 HTTP 入口端口。
- `DEFAULT_NODE_GATEWAY_PORT: u16 = 3180`：默认值。

为什么要显式把端口变成 runtime knob：
- 避免“服务端口后置发现”之外，再把网关入口也写死在上层逻辑里。
- 为“同机多实例/多 profile/测试环境”预留空间（但见本章末尾的 pitfalls）。

## URL 选择与 rtcp 的关系（运行时视角）

BuckyOS 的一个核心点是：客户端并不直接“算出某个服务的最终 IP:port”，而是先获得 service_info，再由 runtime 选择访问路径。

下面的伪代码刻意只表达概念，不等同于实现逐行复刻；实现线索见：`src/kernel/buckyos-api/src/runtime.rs`。

### 1) Client / Service / Kernel 不同 runtime 的 URL 选择

```text
choose_service_url(runtime_type, service_name, https_only):
  if runtime_type == AppClient:
    # 面向外部/浏览器/用户的入口：优先 ZoneGateway
    schema = https_only ? "https" : "http"
    host = zone_id.to_host_name()
    return f"{schema}://{host}/kapi/{service_name}"

  # 运行在节点上的 service（容器内）
  if runtime_type in {AppService, FrameService}:
    url, is_local = try_pick_local_instance_url(service_info)
    if is_local:
      return url

    # 否则统一走本机 NodeGateway（loopback），让网关去决定 rtcp/中转/直连
    return f"http://127.0.0.1:{node_gateway_port}/kapi/{service_name}"

  # kernel service / kernel：可能更“懂网络”，可以直接返回 rtcp 形态
  if runtime_type in {KernelService, Kernel}:
    return pick_best_url_from_service_info(service_info)
```

### 2) NodeGateway 的内部“概念转换”：HTTP 入口 -> rtcp 访问

从 notepads 的描述来看，NodeGateway 内部经常把请求归一为 rtcp 访问路径，例如：

```text
# client hits:
http://127.0.0.1:3180/kapi/<service>

# conceptually becomes (one possible form):
rtcp://<target_node_did>/127.0.0.1:<real_service_port>/kapi/<service>

# if relay needed (SN or a gateway node):
rtcp://<relay_node_did>/rtcp://<target_node_did>/127.0.0.1:<port>/kapi/<service>
```

这解释了两个看起来“反直觉”的设计选择：
- 为什么大量服务默认 `bind 127.0.0.1` 仍然可被别的设备访问：因为访问不是直连到服务，而是通过 rtcp tunnel + NodeGateway 转发。
- 为什么“公网 https 能访问 ZoneGateway”不代表“系统内部访问路径就简单”：内部访问常常更依赖 rtcp/NodeGateway 的稳定与权限策略。

参考：
- `new_doc/ref/notepads/buckyos-api-runtime.md`
- `new_doc/ref/notepads/基于process_chain的service selector.md`

## rtcp / keep tunnel：跨 NAT 的常态机制（更具体一点）

rtcp 默认端口是 2980（TCP），且是“强身份”的：两端都要信任对方公钥。
- 对家庭场景而言，这等价于把“我能不能连到你”从 IP/端口问题，变成“我是否信任你的设备身份 + 我们是否有一条可用隧道”。

在 boot 阶段：
- OOD 会努力与其它 OOD 建立并保持 tunnel（keep tunnel），直到满足启动进入 system-config 阶段的条件。

在常态运行阶段：
- 设备之间的访问优先直连；不通则尝试中转（SN / 特定 ZoneGateway/OOD）；成功后保持 tunnel，后续访问路径会更稳定。

参考：
- `new_doc/arch/02_boot_and_activation.md`
- `new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`

## NodeFinder：局域网 UDP 2980 discovery（boot/multi-OOD 的补位机制）

在“同一局域网内多 OOD”或“设备初次入网、还没连上 system-config”时，一个现实问题是：
- 我知道目标节点的 DID/名字，但我不知道它当前的内网 IP。

NodeFinder 是一个局域网去中心发现机制（UDP 广播）：
- 服务端绑定：`0.0.0.0:2980/udp`
- 客户端行为：枚举本机网卡的 IPv4 广播地址，周期性发送 LookingForReq；收到 LookingForResp 后确认对方存在。

对应代码：
- `src/kernel/node_daemon/src/finder.rs`

概念伪代码：

```text
nodefinder_server():
  udp_bind("0.0.0.0:2980")
  loop:
    req, addr = recv()
    if req is LookingForReq:
      send(addr, LookingForResp{ seq=req.seq, resp=this_device_jwt })

nodefinder_client(looking_for_node_id):
  for each iface in ipv4_ifaces:
    sock = udp_bind(f"{iface.ip}:0")
    enable_broadcast(sock)
    every 2 seconds:
      send(f"{iface.broadcast}:2980", LookingForReq{ node_id, seq, iam=this_device_jwt })
      if recv LookingForResp:
        return resp_source_ip
```

它与 rtcp 的关系：
- NodeFinder 解决的是“在局域网内先找到 IP”这一步（发现）；
- rtcp 解决的是“找到之后如何建立可信连接并长期保持”这一步（连接 + keep tunnel）。

注意：2980 同时被用于 rtcp（TCP）与 NodeFinder（UDP），端口号相同但传输层不同。

## Pitfalls（常见误解/工程坑）

这些坑在 notepads 里反复出现，且很容易在“多网关/多 OOD/公网暴露”场景被放大。

### 1) 同一台设备上多 gateway 实例：端口/路由/权限语义可能变得不确定
- 现象：同机多个 cyfs-gateway/NodeGateway 实例争抢同一入口（如 3180），或者同一个 service 被多个 gateway 以不同策略暴露。
- 后果：服务访问表现为“偶发正确/偶发走错路由/偶发权限不一致”。
- 建议：把 NodeGateway 视为“设备级单实例网络内核”；做多实例/多 profile 时必须明确隔离入口端口与配置域。

### 2) 假设有公网 IP / 假设端口映射稳定
- 误区：把家庭网络当成云网络，默认用固定 IP + 固定端口直连。
- 后果：一旦 NAT/路由器重启/换网络，整个访问模型崩溃；并且会绕开 BuckyOS 的网关/隧道优势。

### 3) DNS 污染/劫持导致“连上了假网关”的风险
`new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md` 提到一个关键担忧：
- 如果用于引导/发现的某些记录（例如历史上的 PX1）缺少可验证签名，那么 DNS 污染可能把客户端导向 fake 节点，导致 rtcp 建立到错误目标。

原则上应把“网关设备信息”放进可验证的对象里（例如带 owner 签名的 JWT 或通过更可信的查询渠道获得），避免把安全性押在 DNS 纯文本记录上。

参考：
- `new_doc/ref/notepads/再次整理zone-boot-config与zone-gateway.md`

## 与类似系统的差异点
- BuckyOS 把“网关 + tunnel”当作内核主链路依赖，而不是可选网络插件。
- 对比：
  - K8s 更倾向于假设节点网络相对稳定（或由 CNI 解决），且对家庭 NAT 场景不是默认优化目标；
  - 家用 NAS 通常提供单机入口或厂商云中继，但不强调 Zone 内统一的、可编排的服务暴露模型。
