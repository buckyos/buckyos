# BNS 去中心的名字系统

本文不再解释“为什么需要 BNS”这类入门概念。相关背景放在 `认识BNS.md`。本文专注于当前 BuckyOS 代码已经形成的名字、DID Document、resolver、gateway、verify-hub 和内容对象之间的技术组合方式。

需要先区分两层事实：

- **当前已落地的名字能力**：`did:bns` / `did:web` / `did:dev` 已经进入 Owner、Zone、Device、Agent、AppDoc、content name、gateway、verify-hub 等链路；Zone 内 resolver 已能从 system-config 查询 `users/*/doc`、`devices/*/doc|info`、`agents/*/doc` 和 `boot/config`。
- **BNS 最终形态**：全局名字注册、名字资产转移、链上 `doc_type` 版本状态、吊销、别名和内容版权流转仍属于协议设计目标；当前仓库里更多是 DNS TXT、SN、system-config、local did docs 和 HTTP resolver 的过渡实现。

因此，本文会用“当前实现”描述已经能从仓库读到的行为，用“协议目标”描述 BNS 完整实现需要承接的能力。

## 当前实现入口

当前与 BNS / DID 解析最相关的代码入口：

| 入口 | 作用 |
| --- | --- |
| `src/kernel/sys_config_service/src/zone_did_resolver.rs` | Zone 内 DID resolver，挂在 system-config service 上，提供 HTTP DID 查询入口 |
| `src/kernel/scheduler/src/system_config_builder.rs` | 首次 boot 时生成 `boot/config`、默认用户 doc、默认 agent doc、默认 device doc |
| `src/kernel/node_active/active_lib.ts` | 激活时生成设备 key，并构造 `did:dev:<device_public_key.x>` |
| `src/make_config.ts` | 开发环境的 Owner、Device 和 Zone 身份配置构造入口 |
| `src/rootfs/local/did_docs/*.doc.json` | 构建时缓存的内置 AppDoc / DID doc |
| `doc/arch/gateway/zone-boot-config与zone-gateway.md` | DNS TXT `BOOT` / `PKX` / `DEV` 与 ZoneBootConfig 的详细设计 |
| `doc/arch/02_boot_and_activation.md` | activation -> ZoneBootConfig -> `boot/config` -> trust keys 的启动主链路 |
| `doc/arch/07_identity_and_rbac.md` | verify-hub、session token、trust keys、RBAC 的当前实现说明 |

当前 `ZoneDidResolver` 支持两个 HTTP 形态：

```text
GET /1.0/identifiers/<did>?type=<doc_type>
GET /.well-known/<doc_type>.json
```

其中 `/.well-known/<doc_type>.json` 会把 Host 转成 `did:web:<host>` 后进入同一个查询流程。

## 为什么只需要三种 DID

BuckyOS 的名字层当前只需要三种 DID method：`did:bns`、`did:dev`、`did:web`。

| DID method | 语义 | 典型对象 | 何时使用 |
| --- | --- | --- | --- |
| `did:bns` | BNS 语义名字，需要 resolver，最终应回到链上名字状态 | Owner、Zone、App、Agent、Group、Content name、Service name | 需要长期信用、可读名字、控制权更新、支付归属、别名或转移 |
| `did:dev` | 设备自认证 DID，id 是设备公钥材料，能提供确定性设备身份 | Device、GatewayDevice、RTCP 对端 | 需要确认“连到的是某台具体设备”，而不是只连到某个 Zone 名字 |
| `did:web` | Web 兼容 DID，把已有 host / self domain 接入 DID resolver | 自有域名 Zone、`.well-known` 文档、过渡期 provider | 需要兼容 DNS / HTTPS / Web host，或把传统域名作为 discovery 入口 |

这三者覆盖了 BuckyOS 目前需要的三类信任目标：

```text
did:bns -> 长期语义和资产控制权
did:dev -> 具体设备和 RTCP 建链确定性
did:web -> 兼容现有 Web host / DNS / .well-known 发现
```

其它 DID method 不是不能存在，而是当前 BuckyOS 主链路没有必要把它们作为默认协议面暴露。用户、Zone、App、Agent、内容权益需要的是可更新的语义名字，所以用 `did:bns`；设备连接需要自认证公钥身份，所以用 `did:dev`；传统 Web 兼容入口用 `did:web`。

## DID Document 的关键设计

受 CYFS 历史命名影响，仓库里很多 DID Document 被叫作 `XXXConfig`。这些不是普通运行配置，而是带身份语义、可签名、可缓存、可被 resolver 返回的文档。

### OwnerConfig

`OwnerConfig` 是顶层 owner 的 DID Document。当前构造入口包括：

- `src/make_config.ts` 的身份配置构造流程
- `src/kernel/scheduler/src/system_config_builder.rs` 的 `add_user_doc(...)`

最小设计视图：

```text
OwnerConfig
  id = did:bns:<owner_name>
  name / display name
  default owner public key
  authentication / verification method
  default_zone_did
  optional payment / controller / profile fields
```

当前 `system_config_builder` 会在首次 boot 时写：

```text
users/<admin>/doc -> OwnerConfig JSON
```

`OwnerConfig` 的核心作用是给后续文档提供 owner 根：

- 验证 ZoneConfig / ZoneBootConfig 是否归属于该 owner。
- 验证 DeviceConfig 是否由该 owner 纳入 Zone。
- 验证 AppDoc / content meta / AgentDocument 是否由 owner 或授权 controller 签发。
- 给 verify-hub / RBAC / contact / payment 等系统提供稳定 owner DID。

### ZoneBootConfig

`ZoneBootConfig` 是 boot 前的最小 Zone Document。它的目标不是完整描述 Zone，而是在 Zone HTTP server 和 system-config 尚未可用时，让设备能拿到足够信息完成安全启动。它目前主要用于构造 DNS TXT Record，以适应 TXT Record 的大小限制。

当前 DNS TXT 设计把它拆成三条记录：

```text
BOOT = ZoneBootConfig JWT
PKX  = Owner public key x
DEV  = Gateway/OOD DeviceMiniConfig JWT
```

最小字段视图：

```text
ZoneBootConfig
  id
  oods
  sn
  exp
  owner
  owner_key        # 来自 PKX，不一定序列化进 BOOT
  extra_info
```

`BOOT` 提供 Zone boot 主体，`PKX` 搬运 Owner public key 的紧凑材料，`DEV` 提供 gateway / OOD 的最小设备信息，例如设备名、公钥和 RTCP 端口。

这就是为什么 DNS TXT 在当前系统里不仅是 DNS。它在 Zone HTTP server 启动前承载了一部分 global profile / boot profile 能力：owner key 材料、最小 Zone boot 信息和 gateway device mini doc。但 TXT 返回的内容只能作为候选材料；客户端必须通过 BNS / 合约拿到可信 OwnerConfig / OwnerDocument 后，用其中的 Owner 公钥验证 `BOOT` 和 `DEV`。`PKX` 可以作为兼容 hint 或交叉检查对象，但不能替代 BNS Owner 公钥成为最终信任根。

### ZoneConfig

`ZoneConfig` 是 system-config 中 `boot/config` 的内部配置。当前由 scheduler `--boot` 基于 `ZoneBootConfig` 和 `start_config.json` 生成，并通过 `zone_document` 字段承载可公开的 Zone DID Document / boot document。

当前代码直接依赖的关键字段包括：

```text
ZoneConfig
  zone_document              # boot JWT/JSON or full ZoneDocument JWT/JSON
    id                       # zone DID
    owner                    # owner DID
    owner key / default key
    oods / gateway info
    sn / sn_url
  verify_hub_info.public_key # internal trust key for verify-hub
  docker_repo_base_url       # internal deployment setting
```

它承担两类职责：

- **连接职责**：`zone_document` 描述 Zone 当前有哪些 OOD / gateway / SN，提供建立 RTCP / gateway 访问的入口。
- **信任职责**：system-config 和 runtime 会从 `boot/config` 刷新 trust keys；owner key 来自 `zone_document`，verify-hub public key 来自内部 `ZoneConfig`。

因此，`ZoneConfig` 不只是网络配置。它是 Zone 启动后的可信控制面根之一。

### DeviceConfig、DeviceMiniConfig 和 DeviceInfo

设备有三种相关对象：

```text
DeviceConfig      # 设备 DID Document，owner 签发，长期身份和能力
DeviceMiniConfig  # boot TXT 里使用的紧凑设备描述
DeviceInfo        # 运行时上报信息，包含 all_ip、状态、系统信息等
```

`DeviceConfig` 的关键字段包括：

```text
DeviceConfig
  id = did:dev:<device_public_key.x> 或等价设备 DID
  name
  owner
  zone_did
  verificationMethod / authentication
  rtcp_port
  ips
  net_id
  ddns_sn_url
  support_container
  capabilities
```

当前 system-config 里常见路径：

```text
devices/<short_name>/doc   -> signed DeviceConfig JWT
devices/<short_name>/info  -> DeviceInfo JSON
```

`ZoneDidResolver` 对 `?type=info` 有特殊处理，会返回 `devices/<short_name>/info` 中的 DeviceInfo JSON。其它 doc type 默认走 signed doc / agent doc / owner doc 查询。

### AgentDocument

默认 Jarvis agent 在首次 boot 时创建：

```text
agents/buckyos_jarvis/doc -> AgentDocument
agents/buckyos_jarvis/key -> Agent private key
users/<admin>/agents/<jarvis-agent-id>/spec -> AgentSpec(AgentServiceBinding -> runtime AppInstanceId)
```

`system_config_builder` 会按 Zone DID method 生成 Jarvis DID：

```text
did:<zone_method>:jarvis.<zone_id>
```

例如 Zone 是 `did:bns:alice` 时，Agent DID 可以是：

```text
did:bns:jarvis.alice
```

AgentDocument 的关键语义：

- Agent 是可被 DID resolver 发现的主体。
- Agent 有自己的 key，但 owner 仍是 Zone owner。
- Agent 的运行服务、消息入口和工具能力可以通过 ServiceInfo / AppServiceSpec / DID Object Profile 继续展开。

### AppDoc 和内容 meta

`src/rootfs/local/did_docs/*.doc.json` 保存了内置 AppDoc 的缓存。AppDoc 通常描述：

- App 名字、版本、owner、author。
- 平台包 / docker image / pkg id。
- service ports、权限、安装提示。
- 具体内容对象或 package object id。

BNS 不负责托管 App package bytes。BNS 负责让 `did:bns:<app>` 或 `did:bns:<app>.<owner>` 解析到可信 AppDoc / content meta，再由 ObjId / hash 校验实际内容。

## Resolve 的核心模型

### document = did + doc_type

BNS resolver 不应该只接受一个裸名字。更准确的查询键是：

```text
document_key = did + doc_type
```

原因是同一个名字可以承载多个文档类型：

| did | doc_type | 返回对象 |
| --- | --- | --- |
| `did:bns:alice` | `owner` | OwnerConfig / global profile |
| `did:bns:alice` | `boot` | ZoneBootConfig |
| `did:bns:alice` | `zone` | ZoneConfig |
| `did:bns:alice` | `service` | ServiceInfo |
| `did:bns:jarvis.alice` | `agent` | AgentDocument |
| `did:bns:filebrowser.buckyos` | `app` | AppDoc |
| `did:bns:book1.alice` | `content` | content meta |
| `did:dev:<pkx>` | `doc` | DeviceConfig |
| `did:dev:<pkx>` | `info` | DeviceInfo |

这样做有两个好处。

第一，同一个名字可以继承已有信用。`did:bns:alice` 的 owner 信用、支付历史、社交关系、内容历史和 Zone 入口可以聚合在一个名字上，而不是为 owner、boot、zone、profile、service 分别注册昂贵的一级名字。

第二，客户端可以明确表达自己要的是什么。启动流程要 `boot`，连接流程要 `zone` / `device doc` / `info`，安装器要 `app`，支付合约要 `content`。返回对象不同，验证规则也不同。

### 当前 ZoneDidResolver 流程


当前实现可以抽象成：

```text
resolve(did, doc_type):
  if doc_type == "info":
    normalize did to short name
    return devices/<short_name>/info

  if did is DID:
    try agents by exact DID
    normalize did to short name / id
    try devices/<key>/doc
    try agents/<key>/doc
    try users/<key>/doc
    else NotFound

  if did is not DID:
    if did == "self":
      return boot/config
    try devices/<name>/doc
    try agents/<name>/doc
    try users/<name>/doc
```

`did:web:<host>` 会被归一化：如果 host 以当前 Zone host 结尾，会剥掉 Zone host 后缀得到设备短名。例如：

```text
did:web:ood2.test.buckyos.io -> ood2
```
> resolve短名，相当于  resolve $nickname.$zoneid

这让传统 host、`did:web`、Zone 内短名和 system-config 的 KV 路径能对齐。

### 当前 provider 优先级

当前代码还没有一个完整的全局 BNS provider 调度器。已有优先级应按阶段理解。

Boot 阶段：

```text
local debug zone.json
  -> BNS / 合约解析 OwnerConfig / OwnerDocument，得到可信 Owner 公钥
  -> DNS TXT BOOT / PKX / DEV 作为候选 boot 材料
  -> 用 BNS Owner 公钥验证 BOOT / DEV，并可对比 PKX
  -> scheduler --boot 写 boot/config
```

Zone 已启动后：

```text
NodeGateway / cyfs-gateway name provider
  -> system-config ZoneDidResolver
  -> boot/config、devices/*、users/*、agents/*
```

App / package 过渡期：

```text
local did_docs cache
  -> Source / repo index
  -> BNS / future global resolver
  -> ObjId / hash 下载校验
```

Web 兼容路径：

```text
https://<host>/.well-known/did.json
https://<host>/.well-known/<doc_type>.json
GET /1.0/identifiers/<did>?type=<doc_type>
```

Web / DNS / HTTPS provider 返回的是候选文档，不自动成为最终信任根。最终仍要回到 DID Document、owner 签名、BNS 状态、DeviceConfig、PathObject / Semantic Path JWT 或 ObjId 校验。

## 场景 1：为什么要有 did:dev

`did:bns:alice` 或 `did:web:alice.example.com` 能表达“我要连接 Alice 的 Zone”。但 Zone 可能有多个 GatewayDevice：

```text
did:bns:alice
  -> ZoneBootConfig / ZoneConfig
  -> gateway candidates: ood1, gate1, gate2
```

如果连接只停留在 Zone 名字层，客户端最多知道自己正在访问 Alice Zone 的某个入口。它还不能确定对端到底是哪台设备、是否是预期 gateway、是否持有对应设备私钥。

`did:dev` 提供这层确定性：

```text
did:dev:<gateway_device_public_key.x>
  -> DeviceConfig / DeviceMiniConfig
  -> RTCP handshake proves possession of device private key
  -> client knows the peer is this GatewayDevice
```

一个典型建链流程：

```text
1. client resolve did:bns:alice，doc_type 为 boot 或 zone。
2. ZoneBootConfig / ZoneConfig 返回 gateway device hints。
3. TXT DEV 或 devices/<gateway>/doc 返回候选 DeviceMiniConfig / DeviceConfig。
4. 用 BNS Owner 公钥或已授权 Zone key 验证 DeviceConfig。
5. DeviceConfig.id 是 did:dev:<pkx>，或可以映射到该自认证设备身份。
6. client 向候选 IP / SN relay / ZoneGateway 发起 RTCP 连接。
7. RTCP handshake 验证对端持有 did:dev:<pkx> 对应私钥。
8. 只有这一步完成后，才能把 tunnel 视为连接到了该 GatewayDevice。
```

这把“与 Zone 建立连接”收敛成“与 Zone 的某个已验证 GatewayDevice 建立连接”。之后上层访问 `system-config`、`verify-hub`、`/kapi/*` 或 `cyfs://` 内容时，才有明确的设备身份基础。

## 场景 2：理解 resolve 的流程

假设 Bob 要访问 Alice Zone 里的 `ood1`：

```text
target = did:bns:ood1.alice
```

推荐流程不是直接把它当 DNS host：

```text
1. resolve(did:bns:alice, boot)
   -> 得到 ZoneBootConfig，适合 boot / 未启动场景。

2. resolve(did:bns:alice, zone)
   -> 得到 ZoneConfig，适合 Zone 已启动后的完整连接和 trust keys。

3. resolve(did:bns:ood1.alice, doc)
   -> 得到 DeviceConfig，确认设备 owner、zone_did、公钥、rtcp_port、net_id。

4. resolve(did:bns:ood1.alice, info)
   -> 得到 DeviceInfo，获取当前 all_ip、状态和运行时网络信息。

5. open_rtcp_tunnel(DeviceConfig + DeviceInfo)
   -> 优先直连，失败则尝试 ZoneGateway / SN relay。
```

如果 Alice Zone 的 HTTP server 还没启动，步骤 2、3、4 不一定可用。此时 DNS TXT 的 `BOOT` / `PKX` / `DEV` 可以先提供最小 profile 候选材料：

```text
BOOT -> 最小 Zone boot 信息
PKX  -> owner public key 材料，用于交叉检查
DEV  -> gateway DeviceMiniConfig
```

这解释了 TXT 记录的存在价值：它不是完整 BNS，也不是最终信任根。客户端仍要通过 BNS / 合约拿到正确 Owner 公钥，再验证 TXT 返回的 `BOOT` / `DEV` 签名，才能把这些材料用于启动或连接。

## 场景 3：出售内容版权意味着什么

内容系统里必须区分两个对象：

```text
content_name = did:bns:book1.alice
content_id   = ObjId / hash
```

`content_id` 表示某个不可变内容版本。`content_name` 表示长期版权、更新权、支付和购买权益的对象。

如果 Alice 只是发布一个新的 `ObjId`，那只是版本更新。如果 Alice 出售 `did:bns:book1.alice` 的版权，含义是这个长期内容名字的控制权、收益目标或 controller 发生变化。

这类名字必须上链，原因是：

- 购买 receipt 绑定的是 `content_name`，不是一次性 `ObjId`。
- 索引者、推荐者、钱包、支付合约需要判断当前 owner / beneficiary。
- 新 owner 需要能发布后续版本。
- 旧版本和旧 receipt 仍需要按历史 owner / 历史签名验证。
- 如果只在某个 Source 数据库里改 owner，无法让其它客户端和支付合约获得同一个事实。

一个版权转移可以这样表达：

```text
before:
  did:bns:book1.alice + doc_type=content
    owner = did:bns:alice
    beneficiary = did:bns:alice
    current_content_id = obj:v1

transfer:
  BNS name state changes controller / owner / beneficiary policy

after:
  did:bns:book1.alice + doc_type=content
    owner = did:bns:publisher-team
    beneficiary = split contract or did:bns:publisher-team
    current_content_id = obj:v2
```

历史状态不能被覆盖。Bob 在转移前购买的 receipt 仍然绑定 `did:bns:book1.alice`，但验证时需要能回到购买发生时的版本状态、签名 key 和权益规则。

## 场景 4：建立别名和改名

别名不是本地昵称。只要别名影响全局信用、购买权益、更新权或客户端展示，它就必须进入 BNS 可验证状态。

### 二级名字变成一级名字

常见路径：

```text
old = did:bns:jarvis.alice
new = did:bns:jarvis
```

流程应类似：

```text
1. Alice 注册或获得 did:bns:jarvis。
2. did:bns:jarvis 的 app / agent / content doc 指向新的权威文档。
3. did:bns:jarvis.alice 设置 alias / migrated_to = did:bns:jarvis。
4. 新旧两个名字都保留可验证迁移记录。
5. 客户端展示“已迁移”，但不静默重写历史 receipt、收藏和签名记录。
```

这样 `jarvis.alice` 的历史信用可以被新一级名字继承，但继承必须是可验证的。不能由某个客户端私下约定。

### Web 域名或旧名作为兼容别名

另一个常见路径是把已有 Web host 接入 BNS：

```text
did:web:alice.example.com -> did:bns:alice
```

`did:web` 可以作为 discovery 入口，但如果它要继承 BNS 信用，需要在 BNS 侧或 OwnerConfig 中声明绑定关系，并验证 Web host 返回的文档与 BNS owner / controller 一致。

### 改名成本

改名必定上链，因为改的是全局语义对象的可验证引用关系。它不是 UI label 变更。

客户端必须理解这个成本：

- 改名可能需要支付链上费用。
- 改名可能需要等待确认。
- 改名后旧名仍要保留 tombstone / alias / migration proof。
- 历史签名、历史 receipt、旧版本内容、旧索引 proof 不应被批量改写。
- 安装器、钱包、搜索和 Agent 都要把 alias 作为安全状态展示，而不是只显示最终名字。

## 场景 5：完全不用 CA 如何工作

传统 HTTPS 依赖 CA 证明“这个 TLS 证书属于这个 host”。BuckyOS 的强验证路径可以不依赖 CA，但前提是客户端是 BuckyOS-aware，而不是普通浏览器只看 TLS 锁。

一个不依赖 CA 的访问链，应先建立 BNS Owner 信任根，再使用 DNS TXT / HTTP / SN 返回的候选连接材料：

```text
1. resolve(did:bns:alice, owner)
   -> 通过 BNS / 合约得到可信 OwnerConfig / OwnerDocument 和 Owner 公钥。

2. 从 DNS TXT、HTTP .well-known、SN 或本地缓存拿候选 boot / zone / gateway 材料。
   -> TXT 中的 BOOT / PKX / DEV 只是候选材料或兼容 hint。

3. 用 BNS Owner 公钥验证候选 ZoneBootConfig / ZoneConfig / DeviceMiniConfig / DeviceConfig。
   -> 只有验签、版本、exp、吊销和 owner 关系都通过后，才得到 verified_owner / verified_zone / gateway list。

4. 从 verified gateway list 中选择 GatewayDevice。
   -> 验证 Gateway DeviceDocument 属于当前 Zone，且具备 RTCP gateway / exchange capability。

5. open_rtcp_tunnel(did:dev:<gateway_pkx>)
   -> RTCP 握手验证 gateway device private key。

6. 在 RTCP tunnel 内发 HTTP / kRPC / cyfs:// 请求。
   -> 传输安全来自 RTCP，不来自 CA。

7. 如果请求是语义连接，继续验证响应中的 PathObject / Semantic Path JWT。
   -> issuer 必须等于 verified_zone；
   -> kid 只能在 verified ZoneDocument / ZoneConfig 中选 key；
   -> payload 必须覆盖 host / path / object_id / iat / exp 等关键字段。

8. 下载或读取对象 bytes。
   -> 验证 hash(bytes) == PathObject / Semantic Path JWT 中绑定的 ObjId。
```

如果请求的是内容：

```text
cyfs://did:bns:alice/ndn/<object>
  -> resolve(did:bns:alice, owner)，从 BNS 得到可信 Owner 公钥
  -> 读取 DNS TXT / SN / HTTP 返回的候选 ZoneBootConfig / Gateway DeviceMiniConfig
  -> 用 BNS Owner 公钥验证这些候选文档
  -> 建立到 verified GatewayDevice 的可信 RTCP tunnel
  -> 读取 PathObject / Semantic Path JWT 和 object bytes
  -> 验证 PathObject 签名、path 与 object_id 绑定关系
  -> 校验 object bytes 的 ObjId / hash
```

DNS、HTTP、SN、镜像源都可以参与发现和加速，但不能伪造最终结果。攻击者可以让请求失败，也可以返回候选假数据，但只要无法伪造 BNS Owner 公钥对应签名、gateway device 私钥、PathObject / Semantic Path JWT 签名或 ObjId，就不能让客户端接受错误内容。

普通 Web 兼容仍可以使用 HTTPS 和 CA。区别是：CA 路径服务最大兼容性，DID / RTCP / ObjId 路径服务强验证。

## 场景 6：新的 SSO 联合登录

当前已经落地的基础能力：

- verify-hub 签发 `session_token`。
- 服务通过 trust keys 验证 token。
- `boot/config` 提供 owner key 和 verify-hub public key。
- control panel 有 `/login`，同 Zone Web SSO 传入 `redirect_url`。
- control panel 通过 redirect URL 对应的 Gateway 路由解析 AuthTarget，并在目标 origin 写入 SSO cookie。

新的联合登录有两条路径：

- **Zone SSO**：用户已经有自己的 Zone，由该 Zone 的 verify-hub 充当 IdP。
- **浏览器钱包登录**：用户还没有 Zone，但浏览器里有支持 BuckyOS DID / BNS 的钱包扩展；联合登录页检测到该扩展后，可以让钱包对登录 challenge 签名，从而完成 SSO 身份证明。

第一条路径把每个 Zone 变成自己的身份提供方，不需要中心化 IdP。

### 已有 Zone 的用户

```text
1. App 声明自己的 DID。
   app_did = did:bns:app1.publisher 或 did:web:app.example.com

2. User 输入或选择自己的 DID。
   user_did = did:bns:alice

3. App resolve(user_did, owner)
   -> 得到 OwnerConfig 和 default_zone_did。

4. App resolve(default_zone_did, zone)
   -> 得到 ZoneConfig 和 verify_hub_info.public_key。

5. App 跳转到 Alice Zone 的 /sso/login。
   client_id = app_did
   scope = requested capabilities
   challenge = app nonce
   redirect_uri = app callback

6. Alice 的 verify-hub 完成本地认证和授权确认。

7. verify-hub 签发 app-specific token。
   userid = did:bns:alice 或 Zone 内 user id
   appid = app_did
   iss = Alice Zone verify-hub / Zone DID
   exp = short ttl

8. App 验证 token。
   使用 ZoneConfig.verify_hub_info.public_key，
   或调用 Alice verify-hub 的 verify_token。
```

这个路径的结果是 Zone verify-hub 签发的 app-specific `session_token`。它既能证明用户身份，也能在该 Zone 的 RBAC / scope 约束下访问 Zone 资源。

### 没有 Zone 的用户：浏览器钱包登录

没有 Zone 的用户没有自己的 verify-hub，也就不能签发 Zone session token。但他仍然可以通过钱包证明“我控制这个 DID / BNS owner key”。

联合登录页需要先做 capability detection：

```text
1. App 打开 /sso/login 或等价联合登录页面。
2. 页面检测浏览器是否存在 BuckyOS wallet extension。
3. 如果检测不到钱包扩展，回到普通 Zone SSO、输入 DID、安装钱包或创建 Zone 的路径。
4. 如果检测到钱包扩展，页面展示“用钱包登录”路径。
```

钱包登录推荐流程：

```text
1. App 生成登录 challenge。
   app_did = did:bns:app1.publisher
   challenge = random nonce
   redirect_uri = app callback
   requested_scope = profile / payment / basic identity

2. SSO 页面请求钱包返回 active DID / BNS name / public key。
   wallet_did = did:bns:alice 或钱包持有的 owner identity

3. 如果 wallet_did 是 did:bns，App 或 SSO 页面 resolve(wallet_did, owner)。
   -> 通过 BNS / 合约拿到可信 OwnerConfig / OwnerDocument。

4. 钱包对登录断言签名。
   assertion payload 至少包含：
     app_did
     challenge
     redirect_uri
     wallet_did
     iat / exp
     requested_scope

5. App 验证钱包签名。
   -> 对 did:bns 身份，用 BNS OwnerDocument 中的公钥验证；
   -> 对尚未上链的临时钱包身份，只能作为 self-asserted wallet identity，不能继承 BNS 信用。

6. App 建立自己的登录会话。
   -> 可以把 verified_wallet_did 作为用户身份；
   -> 可以后续引导用户创建 Zone，把钱包 owner key 绑定到 OwnerConfig。
```

这个路径的结果不是 Zone verify-hub 的 `session_token`，而是一个 wallet-signed login assertion。它适合没有 Zone 的用户完成跨应用登录、证明钱包身份、读取公开 profile、购买内容或创建 Zone；但它不能自动获得任何现有 Zone 的私有资源权限。需要访问 Zone 资源时，仍要走对应 Zone 的 verify-hub / RBAC 授权。

这里 DID resolver 提供的是联合登录元数据发现和 trust root：

- 用户 DID 找到自己的 Zone。
- ZoneConfig 找到 verify-hub。
- verify-hub public key 让 App 验证 token。
- `appid = app_did` 让 token 绑定到具体应用。
- 钱包登录时，BNS OwnerDocument 让 App 验证钱包签名是否属于某个 `did:bns` owner。
- RBAC / scope 决定 token 能访问什么。

这与传统 OAuth 的差异是：IdP 不是固定大平台。已有 Zone 的用户由自己的 Zone verify-hub 充当 IdP；没有 Zone 的用户可以先由钱包充当最小身份提供方。`did:bns` / `did:web` 让 App 能发现并验证 Zone IdP，BNS OwnerDocument 让 App 能验证钱包 owner 身份。

当前仓库已经有 session-token、`/sso/login`、钱包运行态和钱包签名激活基础，但跨 Zone 的标准 redirect、scope、challenge、app DID 校验、token audience、钱包扩展 detection 和 wallet-signed assertion 格式还需要继续协议化。

## 场景 7：Agent 和 App 服务的名字

Agent 名字需要同时回答四件事：

```text
1. 这个 Agent 是谁。
2. 它归哪个 owner / Zone 控制。
3. 怎么把它当成一个实体发消息、邀请、授权或加入关系网络。
4. 怎么调用它背后的 App 服务、工具能力或 HTTP / kRPC endpoint。
```

因此 Agent 不能只用 URL、service name 或 package name 表示。Agent 是传统服务的上层实体：一个 Agent 至少提供“向这个 Agent 发送消息”的服务，还可以继续扩展出 action、tool、event、HTTP、kRPC、DID Object 等传统服务能力。

Agent 和 App 服务的核心区别是：

```text
Agent       = 可参与社交网络的实体 + 一组可调用服务
App Service = 被调度和调用的能力端点
```

这会影响命名方式。Agent 可以作为一个“传统实体”出现在传统社交网络里，例如：

```text
- 用户把 did:bns:jarvis.alice 加入群聊。
- 另一个 Agent 给 did:bns:jarvis.alice 发送 A2A message。
- 群成员列表里记录 did:bns:jarvis.alice 这个成员。
- ACL / RBAC 可以授权给这个 Agent 身份。
```

但一个普通 App 服务不适合这样使用。不能把 `repo-service`、`filebrowser service` 或某个 `/kapi/*` endpoint 加入群聊；它们只能作为被调用的服务能力存在。

推荐组合：

```text
agent_did = did:bns:jarvis.alice
owner_did = did:bns:alice
zone_did  = did:bns:alice 或 did:web:alice.example.com
agent_doc = agents/buckyos_jarvis/doc
agent_key = agents/buckyos_jarvis/key
service   = users/alice/agents/buckyos_jarvis/spec
app_did   = did:bns:jarvis-app.buckyos 或 AppDoc 里的 package identity
```

当前默认 Jarvis 的实现路径是：

```text
system_config_builder.add_default_agents()
  -> AgentDocument(id = did:<zone_method>:jarvis.<zone_id>, owner = did:bns:<user>)
  -> agents/buckyos_jarvis/doc
  -> agents/buckyos_jarvis/key
  -> users/<user>/agents/buckyos_jarvis/spec
```

其中 `AgentDocument` 表达“这个 Agent 是谁”，`AppServiceSpec` 表达“这个 Agent 背后的服务怎么运行”。AgentDocument 不应被退化成 AppServiceSpec 的别名，因为它还承载社交实体身份、owner、签名 key、消息入口和能力声明。

外部系统、用户或其它 Agent 访问它时，应先 resolve Agent DID：

```text
resolve(did:bns:jarvis.alice, agent)
  -> AgentDocument
  -> owner / public key / message endpoint / capability declarations
  -> ServiceInfo / AppServiceSpec / gateway route
  -> AgentMsg / A2A message / DID Object Protocol / service call
```

### 把 Agent 加入群聊

```text
1. 群主邀请 did:bns:jarvis.alice。
2. 群系统 resolve(did:bns:jarvis.alice, agent)。
3. 解析 AgentDocument，得到 Agent public key、owner、message endpoint 和 profile。
4. 群成员列表记录 agent_did，而不是记录某个 service endpoint。
5. 群消息通过 Agent message endpoint 发送。
6. Agent 回复时用 Agent key 或授权 key 签名，接收方按 AgentDocument 验证。
```

这里 Agent 的名字就是群聊成员身份。它可以有头像、profile、关系、权限和历史信用。服务 endpoint 可以迁移，不应改变群成员身份。

### 调用 Agent 的 App 服务能力

```text
1. 调用方 resolve(did:bns:jarvis.alice, agent)。
2. 读取 AgentDocument 中声明的能力或 DID Object Profile。
3. 找到对应 ServiceInfo / AppServiceSpec / gateway route。
4. 按能力要求申请 token、scope 或 capability grant。
5. 调用 HTTP / kRPC / Agent Action / Tool endpoint。
```

这时调用的是 Agent 背后的服务能力，但授权和审计仍应回到 Agent DID。这样同一个 `did:bns:jarvis.alice` 既可以在群聊里作为成员，也可以在自动化流程里作为能力提供者。

如果 Agent 暴露 DID Object 能力，则 BNS 只是可信起点：

```text
did:bns:jarvis.alice
  -> AgentDocument / DID Object Card
  -> DID Object Profile
  -> declared property / action / event
```

Agent 的 message endpoint、服务 endpoint、运行容器、节点位置和 package 都可以迁移或升级。只要 Agent DID 和文档链保持可验证，外部社交关系、群成员身份、授权记录和服务调用引用就不需要变。

## 设计约束

1. `did:bns` 用于长期语义和控制权，不能退化成普通昵称。
2. `did:dev` 用于设备确定性，尤其是 GatewayDevice 和 RTCP 对端验证。
3. `did:web` 用于兼容 Web host，但返回文档必须继续验签。
4. resolver 的输入应是 `did + doc_type`，输出应包含文档和信任上下文。
5. DNS TXT 在当前实现里是 boot profile 兼容路径，不是完整 BNS；TXT 返回内容必须用 BNS Owner 公钥验证后才能使用。
6. 任何影响 owner、beneficiary、alias、content_name 权益的变更都应进入链上状态。
7. 历史签名、历史 receipt、旧 ObjId、旧 owner 状态不能被当前状态覆盖。
8. Source、Repo、SN、HTTPS、`.well-known`、local cache 都可以加速发现，但不能成为最终控制权。
9. verify-hub / SSO 的跨 Zone 版本应以 DID resolver 发现 IdP，以 ZoneConfig 的 verify-hub key 验证 token。
10. Agent、Service、App、Content 应尽量通过 DID Document + ServiceInfo / AppDoc / ObjId 组合表达，避免各自发明不可组合的身份体系。

## 未完成风险

当前仓库仍有一些需要明确的未完成点：

- 全局 BNS 合约、name state、`doc_type` 版本状态、吊销和 alias API 尚未在本仓库落地。
- `ZoneDidResolver` 是 Zone 内 resolver，不等价于全局 BNS resolver。
- RTCP 安全握手和中转隐私在 gateway 文档中仍有 TODO。
- 非 OOD 节点的完整 boot 连接流程仍有未实现分支。
- `/sso/login` 已有 app-specific token 基础，但完整联合登录协议还需要补齐 redirect、audience、scope、challenge 和跨 Zone token 验证规则。
- AppDoc / content meta 的链上发布、版权转移和支付 receipt 还需要与 RepoService、支付合约和客户端安装器联动定义。


## 思考：寻找真实场景，可以覆盖BNS的所有关键能力?

### 没有https, 通过indexer找到一个app并安装，app的owner发布新版本，完成自动升级 

- 异常分支：indexer想阻止app发布新版本？
- 异常分支: app的新版本有病毒，导致app owner的私钥丢失，如何撤回发布？
  鼓励所有的app都基于source dao,不会出现高权限私钥
  定义“信用机构”，信用机构发布第三方Review Report（指向名字或特定ObjId)
  系统有2个固定“信用结构” + 1个Indexer 信用



### 通过did:bns:bob 添加好友后，得到其头像信息

bob没有自己的zone,使用其家庭的zone作为默认zone

did:bns:bob在 OwnerConfig中配置 face url (cyfs:///$objid) 和 binded zone: did:web:zhicong.me
（注意一个属性一旦在OwnerConfig中配置，就无法被覆盖）

得到头像的方法
首先是 User Zero Config
然后根据其 owner_config 的 binded zone，去尝试拼接一个链接 `cyfs://zhicong.me/ndn/$objid`
通过这个链接去下载这个头像

通过 https://bob.zhicong.me/.well-known/doc.json 也能得到一个非上链的profile(UserInfo),可以定义更多的实时信息
如果通过 http://bob.zhicong.me/.well-known/doc.json 访问，这个就必须是一个JWT（有必要的签名），密钥用的是 key@zone
