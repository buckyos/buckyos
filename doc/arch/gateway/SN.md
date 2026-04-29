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

SN 的关键信息保存在 `ZoneBootConfig.sn` 字段（见 [zone.rs](buckyos-base/src/name-lib/src/zone.rs) `ZoneBootConfig`），随 Zone 的 BOOT TXT Record 一起在 Zone 外可信发布。SN 因此是 Zone 在世界上最早可被信任的外部节点，OOD 还没有 system-config 的时候就能用。

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

### 是否需要 SN：以 net_id 为锚点

SN 是否被使用、被使用到什么程度，由入口节点的 `net_id` 决定。判定逻辑实现在 [node_daemon.rs:864](src/kernel/node_daemon/src/node_daemon.rs:864)：**仅当配置了 SN 且 `net_id` 不以 `wan` 开头时才会 `keep_tunnel` 到 SN**。

| net_id | 是否 keep_tunnel 到 SN | SN 必备能力 |
| --- | --- | --- |
| `nat` | 是 | DDNS + TXT + 证书 + HTTPS/RTCP relay + keep-tunnel target |
| `portmap` | 是（443 不可达时） | DDNS + HTTPS relay + keep-tunnel target |
| `wan_dyn` | 否 | DDNS + TXT 引导（IP 会变动） |
| `wan` | 否 | 可选；最多承担命名/证书便利 |

`ddns_sn_url` 只在 `net_id` 为 `wan_dyn` 或 `portmap` 时由激活流程自动写入（见 [active_server.rs:507](src/kernel/node_daemon/src/active_server.rs:507)），用于运行期持续上报地址变化。

### 集群演进中 SN 角色的弱化

随着 Zone 的形态升级，SN 承担的职责会逐步迁出 Zone（与 `Zone集群化.md`"多 OOD 与集群初始化"对应）：

- **单 OOD + 无公网入口 + SN**：最常见家庭形态，SN 兜底所有公网相关能力；
- **单 OOD + 公网 ZoneGatewayNode**：用一台廉价 VPS 充当 ZoneGateway，DDNS/TXT/keep-tunnel/relay 大部分迁到 ZoneGateway，SN 可降级为引导兜底；
- **2n+1 OOD（多 LAN）**：OOD 间的 keep-tunnel 主要靠 ZoneGateway 或公网 OOD，SN 仍然是最坏情况下的 relay 候选；
- **wan + 自有域名**：唯一可以完全不需要 SN 的形态。

也就是说，SN 是"用户可达性能力还不完整时的合成器"，而不是 Zone 永久依赖的中心点。从 ZoneBootConfig 移除 SN 配置只会减少候选路径，不会破坏 Zone 内的真相源（system-config 仍由 OOD quorum 决定）。

## SN 的关键职责（使用者视角）

下面以"用户/集成者"的视角总结 SN 真正提供给 Zone 的能力。每条都对应 `sn_client.rs` 中的 RPC 方法或 `node_daemon` 中的对接点。

### 1. 引导命名 —— 让 Zone 在 Zone 外可被找到

- 提供二级域名（例如 `xxx.buckyos.io`）或绑定用户自有域名作为 Zone hostname；
- 自动配置 TXT 三件套：`BOOT`（ZoneBootConfig JWT）、`PKX`（Owner 公钥）、`DEV`（DeviceMiniConfig JWT）；
- 对应实现：`sn_bind_zone_config()`（[sn_client.rs:477](src/kernel/buckyos-api/src/sn_client.rs:477)），激活时调用，把 zone boot JWT 注册到 SN。

### 2. DDNS —— 让动态地址保持可解析

- OOD/ZoneGateway 周期性上报当前 WAN 地址、设备状态；
- SN 用最新地址刷新 A/AAAA 与 TXT；
- 对应实现：`sn_update_device_info()`、`report_ood_info_to_sn()`（[node_daemon.rs:678](src/kernel/node_daemon/src/node_daemon.rs:678)）；
- 触发条件：`net_id` 为 `wan_dyn` 或 `portmap` 时由激活流程自动写入 `ddns_sn_url`。

### 3. 设备登记与查询 —— 给同 Zone 设备提供发现兜底

- `sn_register_device()` 在激活时把 OOD 的 mini-doc JWT 登记到 SN；
- `sn_get_device_info()` 提供"在没有 system-config 的前提下"获取设备公网地址的能力，给 Boot 阶段的 OOD 之间互联兜底；
- 这部分**不**用于服务级调度，scheduler 仍以 system-config 中的 `devices/*/info` 为真相源。

### 4. TLS 证书协助

- SN 持有自己 hostname 的 TLS 证书。HTTPS 流量先到 SN，再以 SNI/RTCP 方式 relay 到 ZoneGateway；
- 对于使用 SN 二级域名的 Zone，SN 侧可代为完成 ACME 挑战（HTTP-01 / DNS-01）；
- 对于使用自有域名的 Zone，SN 也可以在 SN 侧绑定该 hostname 来承担 relay。
- 注意：Zone 自己的 TLS 证书（`$zonehostname` 与 `*.$zonehostname`）由 ZoneGateway/OOD 持有，SN 永远拿不到 Zone 私钥。SN 拥有命名解析权意味着它可以在用户不知情的情况下重新申请证书，这一点是 SN 信任模型的核心边界（参见 `zone-boot-config与zone-gateway.md`"伪造 TLS 证书风险"一节）。

### 5. HTTPS Relay —— 443 不可达时的公网入口

- 当入口节点是 `nat` 或 `portmap` 且无法映射 443 时，外部浏览器先连 SN，SN 再把流量转发到 ZoneGateway；
- 这是 SN 最常被使用的能力，几乎所有家庭部署都依赖它。

### 6. RTCP Relay 与 keep-tunnel target

- SN 作为 RTCP relay 节点，OOD 通过 `--keep_tunnel <sn_hostname>` 与 SN 长连接（cyfs-gateway 启动参数，见 [node_daemon.rs:883](src/kernel/node_daemon/src/node_daemon.rs:883)）；
- 真实 hostname 通过 `get_real_sn_host_name()`（[sn_client.rs:572](src/kernel/buckyos-api/src/sn_client.rs:572)）从 SN 查询得到；
- SN 上的 relay 可能基于自己的策略限制访问（例如只允许 OOD↔OOD 中转，不开放给任意第三方），具体策略由 SN 运营方决定。

### 7. P2P 打洞辅助（rudp call/called）

- SN 提供 rudp 信令面，帮助两端在 NAT 后完成穿透；
- 打洞成功后通信不再经过 SN；打洞失败回落到上面的 RTCP relay。

## 配置与对接点

| 配置项 | 含义 | 位置 |
| --- | --- | --- |
| `ZoneBootConfig.sn` | SN 主机名/地址，Zone 在 Zone 外可信发布 | `buckyos-base/src/name-lib/src/zone.rs` |
| `ZoneConfig.sn_url` | SN API URL，由 BootConfig 生成或激活流程写入 | `system_config_builder.rs add_boot_config` |
| `DeviceConfig.ddns_sn_url` | 仅 `wan_dyn`/`portmap` 入口节点设置，运行期 DDNS 上报 | `active_server.rs:507` |
| `sn_username` / `sn_rpc_token` | 调用 SN API 的会话凭证，激活时获得 | `active_server.rs:240-339` |
| SN API 根路径 | `/kapi/sn` 与 `/kapi/sn/bns`（kRPC over HTTPS） | `sn_client.rs:10` |
| 历史变更 | `sn_url`：旧 `web3.buckyos.io/kapi/sn` → 新 `web3.buckyos.ai/kapi/sn`；其语义即 `sn_api_url` | — |

判定与上报逻辑：

```text
if zone_config.sn_url.is_some() {
    if ood1.device_config.net_id 不以 "wan" 开头 {
        keep_tunnel_to_sn()    // 长连接，让 SN 可以反向 relay
    }
    report_ood_info_to_sn()    // 周期性上报，让 SN 拿到当前 WAN 地址与设备信息
}
```

未设置 `sn_url`、且 `net_id` 不以 `wan` 开头的 OOD 不会触发任何 SN 逻辑（这种 Zone 实际上只能在局域网内使用）。

## 一句话总结

**SN 是 Zone 在缺少公网 IP、固定 IP、自有域名时，向 Zone 外借来的"可达性合成器"**：它替 Zone 承担命名、DDNS、TLS、relay、keep-tunnel 与打洞辅助；但它不是真相源，也不参与服务级调度——随着 Zone 拥有自己的公网 ZoneGateway 或固定公网入口，SN 的角色会逐步弱化甚至完全退出。
