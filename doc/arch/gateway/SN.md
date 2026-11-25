# SN（Super Node）

本文配合 `doc/arch/gateway/readme.md`、`doc/arch/gateway/Zone集群化.md` 与 `doc/arch/gateway/zone-boot-config与zone-gateway.md` 阅读，回答三个问题：

1. SN 为什么存在，它解决了什么问题？
2. 在 Zone 集群化的不同阶段，SN 的角色如何定义、如何演化？
3. 从使用者视角，SN 的关键职责有哪些？

## SN 的设计意义

BuckyOS 把家庭、个人、小型团队的设备组织成 Zone，但这类用户的网络环境基本不具备传统集群的前提：

- 没有自己的顶级域名，难以独立完成命名、TXT 引导、TLS 证书签发；
- 没有公网 IP，或只有一个频繁变化的公网 IP；
- 没有可映射 443 的端口，甚至完全在 NAT 之后；
- 没有专人运维，无法手工维护 DNS、DDNS、证书、反向代理。

只解决"局域网内访问 Zone"是不够的。Zone 必须在用户离家、用手机外出、把设备分散在多个 LAN 时仍然可达。否则 Zone 就退化成一台局域网 NAS。

SN 就是为了在这种环境下让 Zone "公网可达"而设计的协助节点。它不是 Zone 的一部分，而是 Zone 选择信任的一个外部节点，用来补齐 Zone 自身缺失的能力：公网入口、稳定域名、DDNS、TLS、relay 与 keep-tunnel 兜底。

如同 `zone-boot-config与zone-gateway.md` 所述，Zone 想要稳定支持公网访问，至少要满足以下条件之一：

- `ZoneBootConfig` 配置了 SN；
- `ZoneBootConfig` 或 `ZoneConfig` 中能描述一个公网可达的 ZoneGatewayNode；
- 用户自有域名、DDNS、证书、端口映射和公网入口全部由用户自己维护。

绝大多数家庭用户落在第一类。**SN 不是性能优化项，而是在普通家庭网络下"公网可达"的基础依赖。**

## SN 与 ZoneGateway 的边界

SN 与 ZoneGateway 容易被混淆，但定位完全不同：

| 对比项 | SN | ZoneGateway |
| --- | --- | --- |
| 归属 | Zone 外部、由 SN 运营方维护 | Zone 内部节点（OOD 或专门的 GatewayNode） |
| 是否持有 zone hostname 的 TLS 证书 | 否（SN 不持有 Zone 私钥） | 是 |
| 主要职责 | 域名/DDNS/relay/keep-tunnel target/引导 | Zone 公网入口，承担 TLS、URL Router |
| 是否能直接看到 Zone 内服务 | 否，只能转发到 ZoneGateway/OOD | 是，通过 NodeGateway 访问 Zone 内服务 |
| 替代关系 | 当 Zone 拥有公网 ZoneGateway 时，SN 可降级为可选 | 不可被 SN 替代 |

典型链路（也见 `zone-boot-config与zone-gateway.md`）：

```text
浏览器 --https--> SN（无 zone TLS 证书） --https--> ZoneGateway（有 TLS 证书） --rtcp--> NodeGateway --> Service
浏览器 --https--> ZoneGateway（有 TLS 证书） --rtcp--> NodeGateway --> Service
```

SN 不解码 Zone 内业务流量，只在 TLS 层做 SNI 转发或在 RTCP 层做 tunnel relay。Zone 内服务的真相源仍然是 system-config + scheduler，这一点与 `Zone集群化.md` 中"分层职责"一致——SN 只属于 Boot 层与公网入口层，不参与服务级调度。

## SN 在 Zone 集群化中的角色

`Zone集群化.md` 把问题拆成 Boot 层、调度层、Gateway/Tunnel 层。SN 在这三层中只出现在 Boot 层和 Gateway/Tunnel 层的"中转候选"中，绝不参与调度层的真相生产。

### Boot 层：最小可达性的兜底

SN 的控制面入口保存在 `ZoneBootConfig.sn` 字段（见 [zone.rs](buckyos-base/src/name-lib/src/zone.rs) `ZoneBootConfig`），随 Zone 的 BOOT TXT Record 一起在 Zone 外可信发布。实际承担 RTCP 流量的 relay 由 SN 分配，通过 `zone.get_info` 的 `relay_sn` 返回；两者不要求是同一 hostname。SN 因此是 Zone 在世界上最早可被信任的外部节点，OOD 还没有 system-config 的时候就能用。

OOD 的 Boot 流程在 `looking_zone_boot_config` 之后会同时使用 SN：

- 单 OOD：如果该 OOD 不可公网直达，OOD 通过 SN keep-tunnel 把自己"挂"到公网上，从而让外部 Device 可以经 SN 找到自己；
- 2n+1 OOD：SN 作为 OOD 之间的中转候选之一，让 OOD 即便分布在不同 NAT 后，也能在 Boot 阶段建立 RTCP tunnel，最终凑齐 quorum 进入 system-config 启动阶段（`zone-boot-config与zone-gateway.md` 第 7.2 节）。

### Gateway/Tunnel 层：显式 route candidate

SN 对外提供两类 tunnel 能力：HTTPS relay（443 不可直达时）与 RTCP relay（用于 keep-tunnel target）。这些路径以**显式 route candidate** 出现在配置里，而不是 Gateway 在直连失败后自动发明（参见 `Zone集群化.md` 路径选择目标模型，以及 `服务的多链路选择.md` 第 7.1 节"不使用隐式中转"）。

举例：当 Zone 内只有某个 OOD 与 SN keep-tunnel，外部 Device 访问其它 Node 时的多跳路径必须显式写入 candidate list：

```text
Device -> SN -> OOD1 -> Target Node
```

这条路径对应的 Tunnel URL 形如 `rtcp://<SN-did>/rtcp://<OOD1-did>/rtcp://<Target-did>/`，其中"经过 SN"是配置事实，不是兜底动作。

### 对 SN 的依赖：三件独立的事

走不走 SN 中转、SN 应不应答地址、SN 应不应答其它记录，由不同的东西决定，不能合成一个“需不需要 SN”：

| 事情 | 由什么决定 | 规则 |
| --- | --- | --- |
| 流量经 SN 中转 | 只看接入方式（`net_id`） | `nat` 全部；`portmap` 只有 HTTP(S)，RTCP 直连；`wan_dyn` 平时直连，保留后备隧道；`wan` 不用 |
| SN 应答地址（A / AAAA） | 名字是否由 SN 解析：默认域名、自有域名 NS 委派给 SN（SN 托管），或记录 CNAME 到 `<用户名>.web3.<SN>` | `nat`、`portmap` 应答 relay；`wan_dyn` 应答上报的公网 IP；`wan` 应答签名文档里的固定 IP |
| SN 应答其它记录（证书挑战等 TXT） | 域名的 NS 是否在 SN：默认域名和 SN 托管 | 自有域名的手工和 DNS API 托管不用这一项 |

隧道的判定在 [sn_zone_info.rs](../../../src/kernel/node_daemon/src/sn_zone_info.rs) 的 `device_requires_sn_keep_tunnel`：`net_id` 不是 `wan` 就连 relay。地址上报和 `self_cert` 上报对所有接入方式都做。

SN 的应答规则没有改，由激活发布的文档决定：

| net_id | keep_tunnel 到 SN | 激活发布的文档 | SN 应答的地址 |
| --- | --- | --- | --- |
| `nat` | 是 | mini 设备文档不写 `net_id` | relay |
| `portmap` | 是 | mini 设备文档不写 `net_id` | relay |
| `wan_dyn` | 是，作后备 | mini 设备文档写 `net_id = wan_dyn` | 设备上报的公网 IP |
| `wan` | 否 | `DeviceDocument.ips` 和 `ZoneDocument` 的 `gateway_ips` 写固定 IP | `gateway_ips` |

走激活页面的 Zone 都有 SN 账号，`ZoneDocument.sn`、`BootDocument.sn` 和 `DeviceDocument.ddns_sn_url` 一律写入，不代表要中转。没有 SN 的 Zone 用 `src/active.ts` 激活。

自有域名的三种 DNS 托管（SN 托管、手工、DNS API）见 [zone-boot-config与zone-gateway.md](zone-boot-config与zone-gateway.md) 的 B1 和 C 节。

### 集群演进中 SN 角色的弱化

随着 Zone 的形态升级，SN 承担的职责会逐步迁出 Zone（与 `Zone集群化.md`"多 OOD 与集群初始化"对应）：

- **单 OOD + 无公网入口 + SN**：最常见家庭形态，SN 兜底所有公网相关能力；
- **单 OOD + 公网 ZoneGatewayNode**：用一台廉价 VPS 充当 ZoneGateway，DDNS/TXT/keep-tunnel/relay 大部分迁到 ZoneGateway，SN 可降级为引导兜底；
- **2n+1 OOD（多 LAN）**：OOD 间的 keep-tunnel 主要靠 ZoneGateway 或公网 OOD，SN 仍然是最坏情况下的 relay 候选；
- **wan + 自有域名 + `src/active.ts` 命令行激活**：唯一完全不需要 SN 的形态。走激活页面的 `wan` + 自有域名（手工或 DNS API 托管）不用 SN 应答地址、签证书和中转，但仍有 SN 账号。

也就是说，SN 是"用户可达性能力还不完整时的合成器"，而不是 Zone 永久依赖的中心点。从 ZoneBootConfig 移除 SN 配置只会减少候选路径，不会破坏 Zone 内的真相源（system-config 仍由 OOD quorum 决定）。

### SN 自身的容灾

`ZoneBootConfig.sn` 是单值字符串（hostname），看起来是单点，但 SN 自身的高可用由 SN 运营方在 SN hostname 这一层提供：

- SN hostname 在 DNS 层面解析为多 IP（多机房 / 多入口）。
- 浏览器、传统 HTTPS 客户端无法在应用层做 selector，因此**容灾必须落在 DNS 与 Anycast 上**，而不是让 ZoneBootConfig 维护多 SN 列表。
- BuckyOS 客户端通过同一个 SN hostname 访问时，依靠系统解析 + 连接重试拿到任一可用 IP；这部分对 ZoneBootConfig 是透明的。

这一架构选择意味着：

- ZoneBootConfig 没有必要扩展为多 SN 列表来做应用层 selector——浏览器场景享受不到，BuckyOS 客户端也已经能从 DNS 多 IP 中受益。
- 单个 SN 运营方故障的容灾边界是"换 SN 运营方 → 重新生成 ZoneBootConfig 并刷新 TXT"，而不是 ZoneBootConfig 本身做热切换；这个流程对应 SN 运营方迁移，不是日常故障路径。

## SN 的关键职责（使用者视角）

下面以"用户/集成者"的视角总结 SN 真正提供给 Zone 的能力。每条都对应 `sn_client.rs` 中的 RPC 方法或 `node_daemon` 中的对接点。

### 1. 引导命名 —— 让 Zone 在 Zone 外可被找到

- 提供二级域名（例如 `xxx.buckyos.io`）或绑定用户自有域名作为 Zone hostname；
- SN API 新路径下，`/kapi/sn` 只承载 `auth` 和 `deviceinfo`；BNS 文档发布迁到 `/kapi/bns`；
- `node_active` 不再把 `auth.register` / `auth.login` 返回的 access token 当作 BNS 写权限，也不再通过 `zone.bind_config` 发布 ZoneBootConfig；
- BNS domain 的文档发布走 BNS 写路径；当前 BuckyOS 侧通过 `bns-client` / `bns-indexer` 在 `node_daemon` 内构造并提交 EVM TX，`user_domain` 仍通过 SN 的传统域名校验和 DNS 协助流程处理。

### 2. DDNS —— 让动态地址保持可解析

- OOD/ZoneGateway 周期性上报当前 WAN 地址、设备状态；
- SN 用最新地址刷新 A/AAAA 与 TXT；
- 对应实现：`sn_update_device_online()`、`report_ood_info_to_sn()`（[node_daemon.rs](../../../src/kernel/node_daemon/src/node_daemon.rs)）；
- 触发条件：激活流程对所有接入方式写入 `ddns_sn_url`；SN 应答什么地址由激活发布的文档决定（见“对 SN 的依赖”）。

### 3. 设备登记与查询 —— 给同 Zone 设备提供发现兜底

- `sn_register_device_online()` / `sn_update_device_online()` 只上报设备在线态和连接信息，不再承载 device mini doc 更新；
- 激活期和运行期上报使用当前设备私钥生成的 `sn_device_proof` / device session token，BuckyOS 不再保存或转发网页登录 access token；
- `deviceinfo.resolve_ood_by_did` / `deviceinfo.resolve_ood_by_hostname` 提供"在没有 system-config 的前提下"获取设备公网地址的能力，给 Boot 阶段的 OOD 之间互联兜底；
- 这部分**不**用于服务级调度，scheduler 仍以 system-config 中的 `devices/*/info` 为真相源。

### 4. TLS 证书协助

- SN 持有自己 hostname 的 TLS 证书。HTTPS 流量先到 SN，再以 SNI/RTCP 方式 relay 到 ZoneGateway；
- 对于使用 SN 二级域名的 Zone，以及 NS 委派给 SN 的自有域名（SN 托管），节点把 ACME DNS-01 的挑战写到 SN（`sn-dns`）；
- 自有域名的 DNS API 托管由节点把挑战写到 DNS 服务商（`dns-api`），手工托管由用户上传证书，SN 都不参与；
- relay 只按绑定关系路由，自有域名不要求 NS 委派给 SN；HTTPS 转发还要 `self_cert = true`，由网关按 TLS 栈实际加载到 `<域名>` 和 `*.<域名>` 的证书上报，与证书来源无关。
- 注意：Zone 自己的 TLS 证书（`$zonehostname` 与 `*.$zonehostname`）由 ZoneGateway/OOD 持有，SN 永远拿不到 Zone 私钥。SN 拥有命名解析权意味着它可以在用户不知情的情况下重新申请证书，这一点是 SN 信任模型的核心边界（参见 `zone-boot-config与zone-gateway.md`"伪造 TLS 证书风险"一节）。

### 5. HTTPS Relay —— 443 不可达时的公网入口

- 当入口节点是 `nat` 或 `portmap` 且无法映射 443 时，外部浏览器先连 SN，SN 再把流量转发到 ZoneGateway；
- 这是 SN 最常被使用的能力，几乎所有家庭部署都依赖它。

### 6. RTCP Relay 与 keep-tunnel target

- SN 作为 RTCP relay 节点，OOD 通过 `keep_tunnel: [<relay_sn>]` 与实际分配的 relay 长连接；
- 激活完成远端发布和设备登记后，`node-active` 立即用账号 access token 调用 `SnClient::get_zone_info()`，把完整响应保存到 `$BUCKYOS_ROOT/etc/sn_zone_info.json`；
- node-daemon 重启后的第一份 boot gateway 配置直接读取该文件：`relay_sn` 非空时使用它，为 `null` 时使用 `ZoneBootConfig.sn` 自身作为 relay，因此首次启动 cyfs-gateway 就能指向有效 relay，而不再调用旧的 `get_real_sn_host_name()` HTTP 探测；
- 进入运行态后，node-daemon 用设备私钥生成的短期 device token 立即刷新一次，此后每小时刷新；刷新和在线态上报都在主循环之外的后台任务里，同一时间各只有一个，失败退避（刷新 15 秒到 5 分钟，上报 30 秒到 2 分钟），本机 IP 变化时从头退避；`relay_sn` 改变会改变 keep-tunnel 配置并走现有的 cyfs-gateway restart 路径；刷新失败时继续使用上次成功保存的值；
- SN 上的 relay 可能基于自己的策略限制访问（例如只允许 OOD↔OOD 中转，不开放给任意第三方），具体策略由 SN 运营方决定。

### 7. P2P 打洞辅助（rudp call/called）

- SN 提供 rudp 信令面，帮助两端在 NAT 后完成穿透；
- 打洞成功后通信不再经过 SN；打洞失败回落到上面的 RTCP relay。

## 配置与对接点

| 配置项 | 含义 | 位置 |
| --- | --- | --- |
| `ZoneBootConfig.sn` | SN 主机名/地址，Zone 在 Zone 外可信发布 | `buckyos-base/src/name-lib/src/zone.rs` |
| `sn_zone_info.json` | `zone.get_info` 的本机缓存；非空 `relay_sn` 是 keep-tunnel 目标，为 `null` 时回退到 `ZoneBootConfig.sn` | `$BUCKYOS_ROOT/etc/` |
| `ZoneConfig.sn_url` | SN API URL，由 BootConfig 生成或激活流程写入 | `system_config_builder.rs add_boot_config` |
| `DeviceConfig.ddns_sn_url` | 激活时一律写入，运行期地址和 `self_cert` 上报 | `active_server.rs` |
| `system/domain_hosting` | 自有域名的托管方式和证书来源（`DomainHosting`），默认域名没有 | `src/kernel/buckyos-api/src/domain_hosting.rs` |
| `sn_username` / `sn_device_proof` | 激活期 SN deviceinfo 上报的账号名与设备私钥证明 | `active_server.rs` |
| SN API 根路径 | `/kapi/sn/auth` 与 `/kapi/sn/deviceinfo`（kRPC over HTTPS） | `cyfs-gateway-api/sn_client.rs` |
| BNS API 根路径 | `/kapi/bns`，用于 BNS 匿名读与 EVM TX 写入 | `bns-client` |
| BNS EVM 写入配置 | `bns_url`、`bns_evm`、`bns_evm_private_key`，也可用 `BNS_SERVER_URL` / `BNS_RPC_URL` / `BNS_CHAIN_ID` / `BNS_CONTRACT_ADDRESS` / `BNS_PRIVATE_KEY` | `active_server.rs` |
| 历史变更 | `/kapi/sn/bns` 已迁出 SN 命名空间；BuckyOS 侧不应再调用该旧路径 | — |

判定与上报逻辑：

```text
if zone_document.sn.is_some() {
    if ood1.device_config.net_id 不是 "wan" {
        zone_info = load_or_refresh_sn_zone_info()
        keep_tunnel_to(zone_info.relay_sn.unwrap_or(zone_document.sn))
    }
    report_ood_info_to_sn()    // 周期性上报，让 SN 拿到当前 WAN 地址与设备信息
}
```

未设置 SN 的 OOD 不会触发任何 SN 逻辑。这种 Zone 只有 `wan` 能从外网访问，其它接入方式只能在局域网内使用。

## 一句话总结

**SN 是 Zone 在缺少公网 IP、固定 IP、自有域名时，向 Zone 外借来的"可达性合成器"**：它替 Zone 承担命名、DDNS、TLS、relay、keep-tunnel 与打洞辅助；但它不是真相源，也不参与服务级调度——随着 Zone 拥有自己的公网 ZoneGateway 或固定公网入口，SN 的角色会逐步弱化甚至完全退出。
