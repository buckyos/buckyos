# zone-did-resolver 需求整理

## 1. 任务确认

本文基于当前实现 `src/kernel/sys_config_service/src/zone_did_resolver.rs`、Booting 阶段设计文档 `doc/arch/BuckyOS Booting阶段 resolve-did-document的特殊处理说明.md`，整理 `zone-did-resolver` 的定位、已实现能力、待扩展能力和验收要求。

当前 beta 2.2 是 breaking change，本需求不考虑向前兼容；但当前代码里为了同一 build 内旧 `name-client` 的兼容行为，需要在需求中标清楚哪些是临时兼容实现，避免后续误删。

## 2. 核心定位

`zone-did-resolver` 不是全局 DID resolver，也不是 `name-client` provider 链中的普通 provider。它是挂在 `system-config` 服务上的 Zone 内控制面 cache：

```text
zone 内客户端
  -> NodeGateway / cyfs-gateway 127.0.0.1:3180
  -> system-config / ZoneDidResolver 127.0.0.1:3200
  -> SYS_STORE: boot/config, devices/*, users/*, agents/*
```

它对外暴露标准 HTTP DID resolver 形态：

```text
GET /1.0/identifiers/{did}?type={doc_type}
GET /.well-known/{doc_type}.json
```

但它的语义是：

- 对 Zone 内名字和 Zone 自身，`zone-did-resolver` 是当前 Zone 控制面给出的权威回答。
- 对 Zone 外名字，`zone-did-resolver` 默认无意见，必须让客户端回退到 local cache / method authority / supplement provider 链。
- 它返回的是 DID Resolution Result 信封，并通过 `didDocumentMetadata.buckyos.documentStatus` 表达 `active` / `missing` 等语义。
- 它不负责 Boot 阶段的 Accept 状态机。Booting 阶段的“是否接受新 Zone Document”由 `node_daemon/src/zone_boot_resolve.rs` 的 LKGS 和 smooth upgrade 逻辑决定。

## 3. 当前实现快照

### 3.1 服务入口

当前实现入口：

- `src/kernel/sys_config_service/src/main.rs`
  - `SystemConfigServer` 处理 `/kapi/system_config` POST RPC。
  - `ZoneDidResolver` 注册在 HTTP 根路径 `/`。
- `src/rootfs/etc/boot_gateway.yaml`
  - `/1.0/identifiers/*` 和 `/.well-known/*` 被转发到 `system_config`。
- `src/kernel/sys_config_service/src/zone_did_resolver.rs`
  - 解析 HTTP 请求、读取 `SYS_STORE`、返回 resolver 响应。

### 3.2 Zone 身份来源

`ZoneDidResolver` 通过 `SYS_STORE["boot/config"]` 读取 `ZoneConfig`，再从 `ZoneConfig.zone_document` 解出 `ZoneDocument`：

```text
boot/config
  -> ZoneConfig
  -> zone_document
  -> ZoneIdentity {
       zone_doc,
       raw,
       raw_is_zone_doc,
     }
```

`ZoneIdentity` 是 resolver 判断当前 Zone DID、Zone hostname、Zone owner、公钥和 boot 文档的基础。

当 `boot/config` 不存在、无法解析或 `zone_document` 损坏时，当前 resolver 返回 `503 NoOpinion`，让客户端回退本机解析管线。

### 3.3 查询对象分类

当前实现把查询目标分成三类：

| 分类 | 输入例子 | 语义 |
| --- | --- | --- |
| `ZoneItself` | Zone DID；`did:web:<zone_host>`；Host 等于 Zone hostname 的 well-known 请求 | 查询当前 Zone 自己的文档 |
| `InZone(short)` | 裸短名 `ood1`；`did:web:ood1.<zone_host>`；历史兼容的无点 `did:web:ood1` | 查询 Zone 内二级名字 |
| `Foreign(did)` | `did:bns:alice`；`did:web:other.example.com` | Zone 外名字，只有在 store 中持有且文档自述 id 匹配时才回答，否则无意见 |

`did:dev:*` 和 `did:key:*` 这类 key-class DID 当前被拒绝为非法 resolve 输入，返回 `400`。设备和身份解析应走逻辑名或 DID Document 内部引用，key 只出现在文档内容中。

### 3.4 当前支持的 doc_type

`DidDocType` 由 `name-lib` 定义，默认类型是 `zone`。当前 resolver 的主要行为：

| doc_type | Zone 自身 | Zone 内短名 | Zone 外 DID |
| --- | --- | --- | --- |
| `zone` / 缺省 | 返回 `boot/config.zone_document`；如果原始是完整 `ZoneDocument` 则尽量原样返回 | legacy any-doc：按 `device -> agent -> owner` 尝试 | 仅保留按真实 DID 找 agent 的 legacy 行为；否则无意见 |
| `boot` | 返回 `zone_doc.boot_jwt`；为空则 `missing` | 不支持，返回 `missing` | 无意见 |
| `device` | `missing` | `devices/<short>/doc` | 无意见 |
| `info` | `missing` | `devices/<short>/info` | 无意见 |
| `owner` / `user` | `missing` | `users/<short>/doc` | 若 `users/<did.id>/doc` 的 `owner_doc.id == did`，返回该 owner doc |
| `agent` | `missing` | `agents/<short>/doc` | 先按 key，再扫描 `agents/*/doc` 找 `agent_doc.id == did` |

当前 agent 使用 `DidDocType::Custom("agent")`。

### 3.5 响应契约

当前 resolver 的回答类型：

| 回答 | HTTP | content-type | 语义 |
| --- | --- | --- | --- |
| `Active(JsonLd)` | `200` | `application/did-resolution+json` | DID Resolution Result 信封，`documentStatus=active` |
| `Active(Jwt)` | `200` | `application/did+jwt` | 临时兼容：JWT 文档返回裸 body |
| `Bare` | `200` | `application/json` | legacy `self` 专用，返回裸 ZoneDocument JSON |
| `Missing` | `404` | `application/did-resolution+json` | Zone 对该 `(did, doc_type)` 有权威，确认从未发布 |
| `NoOpinion` | `503` | `application/json` | Zone 对该名字没有意见，客户端应回退 |
| `BadRequest` | `400` | `application/json` | 非法入参 |
| `Internal` | `500` | `application/json` | Zone 内条目损坏或内部错误 |

`Active(JsonLd)` 信封中的 buckyos 扩展至少包含：

```jsonc
{
  "docType": "device",
  "documentStatus": "active",
  "documentVersion": 7,
  "effectiveOwner": "did:bns:owner",
  "docHash": "sha256:..."
}
```

`Info` 类实时数据没有“已发布 body”语义，当前不带 `docHash`。

### 3.6 对 HTTP DID Resolver API 的支持状态

协议参考：`/Users/liuzhicong/project/buckyos-base/doc/http_did_resolver_api.md`。

`zone-did-resolver` 当前实现的是该协议的 Zone 内控制面子集。它复用协议定义的 resolver endpoint 和 DID Resolution Result 信封，但不等价于一个完整的、method-agnostic 的全局 HTTP resolver。

支持状态：

| 协议能力 | 当前支持情况 | 说明 |
| --- | --- | --- |
| `GET /1.0/identifiers/{did}?type={doc_type}` | 已支持 | 主查询入口；`type` 缺省映射为 `zone` |
| `Accept: application/did-resolution` | 部分支持 | 当前服务端不依赖 `Accept`，JSON 信封返回 `application/did-resolution+json`；后续如要严格对齐协议，需要统一 content-type |
| W3C DID Resolution Result 信封 | 部分支持 | JsonLd active / missing 已走信封；JWT active 当前为兼容旧客户端返回裸 JWT |
| `didDocumentMetadata.buckyos.docType` | 已支持 | active / missing 信封中携带 |
| `documentStatus=active` | 已支持 | Zone 持有该文档时返回 |
| `documentStatus=missing` | 已支持 | Zone 对 Zone 内名字有权威且确认不存在时返回 |
| `documentStatus=revoked/tombstoned` | 未产生 | 当前没有 Zone 内吊销登记；引入 `resolver/cache` 后补 `410` 信封 |
| `documentStatus=migrated/expired` | 未产生 | 当前无迁移和过期状态控制面 |
| `documentVersion` / `versionId` | 部分支持 | 从文档 `version_seq` 提取；没有 version 时缺省 |
| `effectiveOwner` | 部分支持 | Zone 自身和 Owner doc 可带；DeviceInfo / DeviceDocument 等不一定带 |
| `docHash` | 部分支持 | 对有“已发布 body”语义的 active 文档计算编码后 body 的 sha256；Info 类不带 |
| anchor-only `docHash` | 未支持 | 当前 active 命中都内联返回文档，不只返回锚点 |
| `authoritySeq` | 未支持 | 当前 Zone 控制面无该序列 |
| `migrationTarget` | 未支持 | 需等 migrated 状态实现 |
| `iat` 历史 owner 查询 | 未支持 | 当前 handler 只解析 `type`，忽略 `iat`；不能把当前结果当历史快照使用 |
| `501 historicalQuerySupported=false` | 未支持 | 若后续公开 owner 历史查询能力，需要按协议区分“无历史能力”和“历史 missing” |
| `Cache-Control` | 未支持 | 当前主要经本机 / Zone 内访问；如公开暴露 resolver，需要补缓存策略，尤其是负状态 |

状态码边界：

- 协议要求 `documentStatus` 才是发布状态语义来源，不能只看 HTTP status。
- 当前 `zone-did-resolver` 对 Zone 内明确 missing 返回 `404 + documentStatus=missing`，符合协议。
- 当前 `zone-did-resolver` 对 Zone 外无意见返回 `503`，这是 Zone L1 cache 的特殊语义：让 `ZoneResolverClient` 回退本机 cache 和 provider 链。协议中的普通 provider `NotApplicable` 可以用 `404` 且不带 `documentStatus` 表达；Zone cache 不采用这个形态，是为了避免裸 404 和强负 missing 在旧链路里混淆。
- 当前 `500` 的语义还需统一：服务端注释倾向把 Zone 内损坏视为“不能外查顶替”的坏回答，但 `ZoneResolverClient` 当前把 `500` 当 unknown 回退。实现强负状态 / 通用 cache 前必须先决策。

`/.well-known/*` 边界：

- 协议文档把 `/.well-known/did.json`、`/.well-known/{doc_type}.json` 定义为 did:web / BuckyOS 静态发布面；严格协议下应返回裸 DID Document 或裸 doc body，不能返回 DID Resolution Result 信封。
- 当前 `ZoneDidResolver` 的 `GET /.well-known/{doc_type}.json` 是动态 resolver 兼容入口：通过 Host 构造 `did:web:<host>` 后进入同一套 `resolve()`，JsonLd active 会返回 resolution 信封。
- 因此当前 `/.well-known` 行为应标记为 ZoneGateway 内部动态别名，不应宣称为标准 did:web 静态发布面的完整实现。若未来要对公网提供标准 did:web 兼容，应把静态发布面和 `/1.0/identifiers` resolver API 分开处理。

## 4. 与 Boot Resolve 的边界

Booting 阶段设计文档的核心原则是：

```text
Resolve 是发现
Accept 是 Boot State Machine 决策
```

当前 `node_daemon/src/zone_boot_resolve.rs` 已经按这个原则实现：

- Boot 首先安装本地 Owner trust：`node_identity.owner_public_key` 合成最小 OwnerDocument，注入 name-client local authority override。
- Boot 发现 Zone Document 的顺序：
  1. `resolve_did(zone_did, zone)`
  2. `resolve_did(zone_did, boot)`
  3. 直查 `DnsProvider` 的 boot 文档
- 网络来源的 Zone / Boot document 只接受 owner-signed JWT，不接受网络返回的 JsonLd。
- candidate 必须用 `node_identity.owner_public_key` 验签，并且 `zone_did`、`owner_did`、key material 与本地 `node_identity` 锚定一致。
- First Boot 没有 LKGS，失败就是 Boot Failed。
- Warm Restart 有 LKGS，resolve 失败或不可平滑演进时忽略 candidate，继续使用 LKGS。
- Smooth upgrade 明确拒绝版本回滚、单 OOD 与多 OOD 共识模型互转、无 OOD、当前节点角色变化。

因此 `zone-did-resolver` 的需求边界是：

- 可以帮助发现当前 Zone 已接受的 `zone` / `boot` 文档。
- 不决定 Boot 是否接受新 Zone Document。
- 不在线替换 Root Trust。
- 不把外部 OwnerDocument 的 key rotation 自动应用到当前系统。

## 5. 目标能力

### 5.1 Zone 内二级名字的权威 resolver

目标：对当前 Zone namespace 内的名字，`zone-did-resolver` 必须给出稳定、可区分、不会误触发外部回退的回答。

Zone 内名字包括：

- 裸短名：`ood1`
- Zone hostname 下的 did:web：`did:web:ood1.<zone_host>`
- 历史兼容无点 did:web：`did:web:ood1`
- well-known 请求中由 Host 转换出的 `did:web:<host>`

查询规则：

- `type=device`：读取 `devices/<short>/doc`。
- `type=info`：读取 `devices/<short>/info`。
- `type=owner` / `type=user`：读取 `users/<short>/doc`。
- `type=agent`：读取 `agents/<short>/doc`。
- `type=zone` / 缺省：保留当前 legacy any-doc 兜底，但后续应逐步要求调用方显式传 `doc_type`。

返回规则：

- Zone 内存在文档：`200 active`。
- Zone 内不存在该 `(short, doc_type)`：`404 missing`，这是强负回答。
- Zone 内条目存在但损坏：`500 internal`，同时记录 error 日志。
- Zone 外名字不能因为本地没有 key 而返回 `404 missing`，必须返回 `503 NoOpinion`，否则会把外部解析错误地缓存成强负状态。

### 5.2 Zone 内 device_info resolver

目标：支持网关和 runtime 通过 resolver 获取设备自报 IP、状态和运行时信息。

当前路径：

```text
GET /1.0/identifiers/did:web:<device>.<zone_host>?type=info
  -> devices/<device>/info
  -> DeviceInfo JSON
```

要求：

- `DeviceInfo` 是 Info 类文档，不要求 owner 签名，也不带 `docHash`。
- 返回信封时 `didDocument` 应是 `DeviceInfo` JSON object，`documentStatus=active`。
- 找不到设备 info 时，在 Zone 内名字语义下返回 `404 missing`。
- 解析到的 `all_ip` 等网络信息只能作为运行时候选，连接层仍要结合 DeviceDocument、ZoneDocument membership 和 RTCP 持钥证明。

### 5.3 Zone 自身文档 resolver

目标：对当前 Zone DID 和 Zone hostname 返回已接受的 Zone 文档。

规则：

- `type=zone`：返回 `boot/config.zone_document`。
  - 如果 `boot/config.zone_document` 原始保存的是完整 `ZoneDocument`，优先原样返回，保留签名或原始编码。
  - 如果原始保存的是 `ZoneBootDocument` JWT，则返回由 `ZoneConfig.zone_document()` 重建出的 `ZoneDocument` JSON。
- `type=boot`：返回 `zone_doc.boot_jwt`。
- 其它 doc_type：当前 Zone 自身没有该文档，返回 `404 missing`。
- `GET /1.0/identifiers/self` 保留 legacy 行为，返回裸 ZoneDocument JSON，用于旧 websdk 从顶层 `hostname` / `id` 取 Zone host。

### 5.4 Zone 级别 cache / override

目标：后续支持通过 SystemConfig 管理 Zone 级别 DID Document cache，使 Zone 控制面可以对特定 `(did, doc_type)` 给出显式回答，覆盖默认外部解析路径。

这类 cache 的语义不是普通本机缓存，而是 Zone 控制面策略：

- 可用于 Zone 内私有文档发布，例如只在 Zone 内可见的 device / agent / service 文档。
- 可用于强负状态，例如 Zone 内吊销、tombstone 或明确 missing。
- 可用于临时 override，例如维护期间 pin 住某个 DID Document。
- 不应被普通用户绕过；一旦 Zone 明确回答，客户端不再继续查外部 provider。

建议后续数据模型：

```text
resolver/cache/<escaped_did>/<doc_type>/state
resolver/cache/<escaped_did>/<doc_type>/doc
resolver/cache/<escaped_did>/<doc_type>/metadata
```

状态字段建议：

```jsonc
{
  "document_status": "active|missing|revoked|tombstoned|migrated|expired",
  "document_version": 1,
  "effective_owner": "did:bns:...",
  "authority_seq": 1,
  "doc_hash": "sha256:...",
  "migration_target": "did:bns:...",
  "updated_at": 0,
  "updated_by": "system|admin|scheduler"
}
```

权限要求：

- 只能由系统服务、scheduler、admin/root 维护。
- 普通 app 不能写入 resolver cache。
- 写入 `revoked` / `tombstoned` / `missing` 这类强负状态需要更高权限，并记录审计日志。

当前实现尚未有该通用 cache，仅有 hard-coded 的 `boot/config`、`devices/*`、`users/*`、`agents/*` 查询。

## 6. 特殊 DID 解析保护

### 6.1 Root Trust 不通过 resolver 自动替换

Booting 设计文档要求：

- Activation 建立信任。
- Boot 恢复信任。
- Runtime 演进状态。
- Boot 永远不在线 resolve OwnerDocument 来替换 Root Trust。

因此 `zone-did-resolver` 不得把外部 OwnerDocument 的公钥变化自动变成当前系统信任根。OwnerDocument 更新属于 Root Trust Migration，必须进入维护 / recovery 流程。

### 6.2 关键 OwnerDocument 合并

目标：当 resolver 返回“当前 Zone owner”的 OwnerDocument 时，可以吸收权威源中非密钥类的 profile / metadata 更新，但必须保留系统当前正在使用的 owner key material，除非系统进入显式维护流程替换本地 Root Trust。

适用对象：

- `did == boot/config.zone_document.owner`
- 或等价于 `node_identity.owner_did` 的 owner DID

合并原则：

```text
解析结果 =
  权威源 OwnerDocument 的非公钥部分
  + 当前本地 Root Trust 的公钥部分
```

其中“当前本地 Root Trust”优先级：

1. `node_identity.owner_public_key`，如果当前进程可以安全读取。
2. Boot 已接受并写入 `boot/config.zone_document` 的默认 key material。
3. 显式维护流程写入的系统内部 root trust 记录。

不得使用普通在线 owner 文档更新来覆盖上述 key material。

建议合并字段：

| 字段类别 | 来源 |
| --- | --- |
| `id` | 必须与本地 owner DID 一致；不一致则拒绝 |
| `verificationMethod` 中默认 key | 本地 Root Trust |
| `authentication` / `assertionMethod` / `capabilityInvocation` 中引用默认 key 的部分 | 本地 Root Trust |
| key material 相关扩展、`keyScope` | 默认保留本地 Root Trust 对应信息；如需扩展 key scope，必须经维护流程或受控系统配置 |
| `name` / `display_name` / `avatar` / `meta` / `wallets` / `binded_zone_list` / service profile | 可来自权威源 |
| `version_seq` / `mini_version_seq` / `valid_iat` | 可来自权威源，但不能导致本地 key 被替换 |

如果权威源 OwnerDocument 丢失、损坏或公钥变化，resolver 的行为应是：

- Boot / Root Trust 使用路径继续使用本地 owner trust。
- Runtime profile 展示可以退化为本地最小 OwnerDocument。
- 必须记录日志，说明权威 owner doc 与本地 Root Trust key material 不一致。

### 6.3 Zone Document 解析保护

`zone-did-resolver` 返回的 Zone Document 应代表“当前 Zone 已接受状态”，不是“外部最新状态”。

要求：

- `type=zone` 只能返回 `boot/config` 中当前已接受的 `zone_document`。
- 不应在 resolver 请求中临时访问外部 provider 并返回一个尚未被 Boot 状态机接受的新 Zone Document。
- Zone Document 的平滑升级判断在 node-daemon Boot 状态机中完成。
- 如果后续引入 Zone cache override，不能绕过 LKGS / smooth upgrade 直接替换 `boot/config.zone_document`。

### 6.4 key-class DID 保护

当前实现已拒绝 `did:dev:*` 和 `did:key:*` 作为 resolver 输入。该规则应保留：

- key-class DID 表示密钥或设备自认证身份，不是 Zone 内逻辑名字。
- DeviceDocument 查询应使用设备逻辑名、`did:web:<device>.<zone_host>` 或后续明确设计的 device DID 映射。
- DID Document 内可以引用 key-class DID，但 resolver API 不应把它当作普通二级名字解析。

## 7. Zone 内与 Zone 外的权限控制

需求目标：后续根据调用来源和配置，对 Zone 内请求、Zone 外请求、公开请求做选择性返回。

当前实现没有区分请求身份，只根据名字是否属于 Zone namespace 决定是否回答。

建议策略：

| 请求来源 | 默认可见 doc_type | 说明 |
| --- | --- | --- |
| 本机 / Zone 内服务 | `zone`、`boot`、`device`、`info`、`owner`、`user`、`agent` | 当前主要使用场景 |
| 公开网关请求 | `zone`、`boot`、必要的公开 owner/profile | 应避免泄露 Zone 内私有 device info |
| 已登录用户 / app | 按 RBAC 决定 | 与 system-config 权限模型对齐 |
| Zone 外匿名请求 | 默认只暴露公开 DID Document | 不返回内部 `DeviceInfo`、私有 agent、内部 cache override |

实现上需要注意：

- DID resolver HTTP GET 当前没有 session token。若要做强权限，需要通过 gateway 注入来源信息、内网 ACL、或新增受控查询接口。
- 不要让权限不足误返回 `404 missing`，否则调用方会把它当强负证据。权限不足应返回 `403` 或无意见语义，具体需与 `ZoneResolverClient` 客户端解释规则一起改。

## 8. 与 name-client 的互操作要求

`name-client` 当前有独立的 `ZoneResolverClient`，默认查询：

```text
http://127.0.0.1:3180/1.0/identifiers/{did}?type={doc_type}
```

互操作要求：

- `documentStatus=active` / `missing` / `revoked` / `tombstoned` 是语义来源，不能只依赖 HTTP status。
- 裸 `404` 在客户端语义里可能是 unknown；明确 Missing 必须带 `didDocumentMetadata.buckyos.documentStatus = "missing"`。
- `503` / `502` / `504` 表示 Zone L1 cache unknown，客户端回退本机 cache 和 provider 链。
- `200` 但 body malformed 是坏回答，不是 cache miss。
- JWT 文档当前因旧客户端兼容返回裸 JWT；等所有 zone 内客户端升级后，应统一改为 DID Resolution Result 信封，以便携带 `docHash` / `documentVersion` / `effectiveOwner`。

当前 `zone_did_resolver.rs` 顶部注释与 `ZoneResolverClient` 对 `500` 的解释存在差异：服务端注释倾向把 Zone 内条目损坏视为不能外查顶替，但当前客户端测试 `server_error_is_zone_unknown_and_falls_back` 把 `500` 当 unknown。后续实现通用强负状态或内部错误策略时，必须同步修改服务端、客户端和协议文档，不能只改一侧。

## 9. 非目标

本阶段不做：

- 不实现全局 BNS resolver。
- 不让 ZoneDidResolver 主动访问外部 resolver provider。
- 不在 resolver 请求路径里执行 Boot Accept / smooth upgrade。
- 不通过普通 runtime resolve 自动更新 OwnerDocument 公钥。
- 不新增依赖。
- 不把所有 Zone 内文档默认公开给 Zone 外匿名请求。
- 不为 beta 2.2 之前的旧协议保留额外兼容逻辑；当前裸 JWT 兼容只服务于同一 build 内还未统一的旧客户端解析路径。

## 10. 验收要求

### 10.1 当前能力验收

- `GET /1.0/identifiers/self` 返回当前 ZoneDocument 裸 JSON，包含 `hostname` 或 `id`。
- `GET /1.0/identifiers/<zone_did>?type=zone` 返回当前 `boot/config.zone_document`。
- `GET /1.0/identifiers/<zone_did>?type=boot` 在 `boot_jwt` 存在时返回 boot JWT，不存在时返回 `404 missing` 信封。
- `GET /1.0/identifiers/did:web:<device>.<zone_host>?type=device` 返回 `devices/<device>/doc`。
- `GET /1.0/identifiers/did:web:<device>.<zone_host>?type=info` 返回 `devices/<device>/info`，信封中 `documentStatus=active`。
- Zone 内不存在的短名返回带 `documentStatus=missing` 的 404。
- Zone 外 DID 不命中本地持有文档时返回 503，不返回 404。
- `did:dev:*` / `did:key:*` 返回 400。

### 10.2 特殊保护验收

- Boot 过程中 OwnerDocument 只来自 `node_identity` / local authority override，不因在线解析结果改变。
- Warm Restart resolve 到不可平滑演进的 Zone Document 时，node-daemon 继续使用 LKGS。
- 对当前 Zone owner 的 owner doc 查询，如果权威源公钥与本地 Root Trust 不一致，resolver 不应输出会替换本地 Root Trust 的结果。
- 更新 Owner key 的流程必须通过维护 / recovery 路径验证，不能通过普通 `users/<owner>/doc` 写入或外部 resolver 响应完成。

### 10.3 后续扩展验收

- 引入通用 `resolver/cache` 后，active / missing / revoked / tombstoned / migrated / expired 都能通过 DID Resolution Result 信封表达。
- cache override 的写入受 RBAC 控制，并有审计日志。
- 权限不足不会伪装成 `missing`。
- 服务端和 `ZoneResolverClient` 对 500 / 403 / 404 / 410 / 503 的解释保持一致，并有单测覆盖。
- `/1.0/identifiers` 与 `http_did_resolver_api.md` 的请求、信封、content-type、`iat` 历史查询不支持语义有明确对齐说明。
- 若继续保留 `/.well-known/*` 动态入口，文档必须明确它不是 did:web 静态发布面；若要做标准 did:web 静态发布面，则返回裸 DID Document / 裸 doc body。

## 11. 风险与待确认

- `500 Internal` 在服务端注释与当前 `ZoneResolverClient` 行为之间语义不一致，需要单独决策。
- OwnerDocument 合并需要明确最终实现位置：可以在 `ZoneDidResolver` 内对当前 owner 的查询做合并，也可以在 `name-client` local authority override 层完成；两者不能同时各自改 key material。
- 如果 `ZoneDidResolver` 要直接读取 `node_identity`，需要定义只读注入方式，避免把运行时文件路径和 Root Trust 作为普通 system-config key 暴露。
- `resolver/cache` 的 key 编码、RBAC 资源路径和迁移策略需要在实现前补充具体 schema。
- 当前 `type=zone` 对 Zone 内短名的 any-doc fallback 是 legacy 行为，后续是否收紧为严格 doc_type 需要与 `name-client` 和调用方一起确认。
- `/.well-known/*` 当前动态信封响应与 `http_did_resolver_api.md` 中静态发布面的裸文档要求不一致，需要决定是改实现、改路由命名，还是把它正式定义为 ZoneGateway 私有兼容入口。
- `iat` 历史 owner 查询当前会被忽略；如果外部调用方按协议使用该参数，可能误以为拿到了历史快照。
