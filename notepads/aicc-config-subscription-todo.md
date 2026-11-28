# AICC 配置订阅更新 TODO：可信订阅源、通用组件与价格基准

2026-11-27 立项，未实施，代码基线 `cbd8bbc4f`。

本文是 [Driver Metadata 云更新上线 TODO](aicc-driver-metadata-cloud-update-todo.md)（2026-09-25）的超集。旧 TODO 的决策项、已确认缺陷、实现、验收和逻辑树方案全部并入本文，逐条去向见 §9、§10。两份不一致时以本文为准。

实施方先读 §1、§2，再按 §11 的步骤做。§12 的问题定下来之前，只做 §11 里标了“不依赖决策”的步骤。

## 1. 目标与原则

### 1.1 owner 原话（2026-11-27）

> **AICC 的配置文件更新**
>
> 核心原则是从可信订阅源下载配置，用户可以配置多个订阅源。
>
> 配置是一个 named object，需要验证来源（有签名），原理上用户只信任可信的来源。
>
> 多个配置合并，相同配置项，高可信度的来源覆盖低可信度。同样的可信度，新的覆盖旧的。合并后的配置保存在本地，后续如果订阅更新失败，也不影响使用。（这些配置都有预装版本）
>
> 上述机制是系统的通用机制，应该抽象成公共组件。
>
> AICC 可以更新的配置主要是 driver meta data + 官方 provider 的配置（比如 open router 的模型名映射文件）。
>
> 价格信息在 driver meta data 里有基准值（一个模型家族里，某个模型定义为 1.0，其它的则为比例）。这样 provider 不需要定义所有的模型的价格，只需要定义基准值对应的不同货币单位值即可，比如 1.0 = 0.5 美金 / 2 RMB（允许 1 个基础单位用不同货币结算的时候值不同）。
>
> 价格信息的原则宁可不准确也不估算。也就是说 provider 的默认配置是无价格信息，上面的 driver meta data 中价格信息只有在 provider 中显式集成后才会生效。

### 1.2 原则

- **P1** 配置只从可信订阅源下载，用户可以配置多个订阅源。
- **P2** 每份配置都是 Named Object，带发布者签名。验证的是来源，不是传输通道。
- **P3** 多源合并：同一配置项，可信度高的来源覆盖低的；可信度相同时，新的覆盖旧的。
- **P4** 合并结果保存在本地，订阅更新失败不影响使用。所有配置都有预装版本。
- **P5** 这套机制做成系统公共组件，AICC 是第一个使用者，组件里不写 AICC 的概念。
- **P6** AICC 通过订阅更新的配置主要是 Model Driver metadata 和官方 Provider 的配置。后者包括 OpenRouter 的模型名映射，它现在就在 `openrouter.provider.json` 的 `model_driver_vendor_prefixes` / `model_driver_overrides` 里。
- **P7** 价格基准：Model Driver 给相对价格，一个价格组里的基准模型为 1.0，其余模型写成比例。Provider 只声明 1.0 在各币种下值多少钱，不同币种的值可以不按汇率换算。
- **P8** 价格宁可不准确也不估算。Provider 默认没有价格；Model Driver 里的相对价格只有在 Provider 显式集成之后才生效。

已有裁决里和本单相关的，继续有效：

- 只有 API 类型和协议客户端升级需要发版本，其余（包括内置 Provider 的能力升级）都靠配置更新（[协议客户端 TODO](aicc-provider-protocol-client-todo.md) 目标第 7 条）。本单就是交付这条“配置更新”通道。
- 模型事实只从 Model Driver 读，目录不认识的模型名就是不识别，不给默认值。
- AICC 不自动换模型或 Provider（[aicc_router.md §9](../doc/aicc/aicc_router.md)）。价格会进入路由（成本上限、`cost_first` 评分）。比例价会让原来价格未知的模型有价格，不能借此引入自动切换，见 §7.6。
- 默认允许 breaking change，不做旧格式兼容（`AGENTS.md`）。旧的 index / manifest / track 协议和 `revisions/` 缓存直接删除，不迁移。

### 1.3 我对原话的理解（待 owner 确认，见 §12 Q1–Q5、Q13）

- **配置项的粒度**沿用 AICC 现在的来源选择单位：一个 `(catalog_kind, catalog_id)` 文件就是一个配置项，整项覆盖，不做字段合并。通用组件只认“命名空间 + 种类 + id”，不解析内容。
- **“新”**指发布者签进对象里的发布时间 `issued_at`，不是下载时间。不同发布者各自的序号不可比，不能用序号比新旧。
- **预装版本**等于构建时官方订阅源的快照，可信度与官方源相同，也按 `issued_at` 参与比较。这样新二进制改过的项会盖过旧的云端项，没改过的项不会盖掉更新的云端热修（这就解决了旧 TODO 的 D7）。前提是预装版本和官方源用同一个工具从同一个目录生成，见 §5。
- **本机管理员的手工覆盖**（现在的 local 目录和 system-config 两层）当作两个最高可信度的本地来源，不需要签名。
- **“宁可不准确也不估算”**：按比例算出的价格可以和渠道真实价格有出入，例如某个模型单独打折而 Provider 没写例外，这可以接受。但凡是没有显式集成的地方一律不出价格，不从官方价、相近模型或别的渠道推一个出来。

## 2. 现状（2026-11-27 核对）

| 方面 | 现在的代码 | 与原则的差距 |
| --- | --- | --- |
| 订阅源 | 只有一个源：`CloudUpdateConfig { enabled, source_url, interval_secs }`（[cloud_update.rs:119](../src/frame/aicc/src/service/cloud_update.rs#L119)），默认关闭、没有默认地址，存在 `services/aicc/driver_metadata_update` | P1 |
| 来源验证 | index 不带期望 ObjId 直接取（`fetch(&index_url, None)`）。manifest 和文件按 index 给的 ObjId 校验。没有任何发布者签名，信任根是 index，而 index 只靠传输层保护 | P2 |
| 凭据 | 把 AICC 的 service session token 当 `Authorization: Bearer` 发给任意 `source_url`（[cloud_update.rs:196-273](../src/frame/aicc/src/service/cloud_update.rs#L196-L273)，[service/mod.rs:494](../src/frame/aicc/src/service/mod.rs#L494)） | 安全缺陷 |
| 版本 | 全局 manifest `revision_seq` 加本机高水位 `metadata_target_seq`。builtin 用常量 `BUILTIN_CATALOG_REVISION_SEQ = 26`（[settings/mod.rs:23](../src/frame/aicc/src/settings/mod.rs#L23)），二进制升级不推进它 | P3（没有“同级比新旧”），旧 D7 |
| 合并 | `system-config > local > cloud > builtin` 固定四层，按 `(kind, id)` 整文件选择（[settings/mod.rs:668](../src/frame/aicc/src/settings/mod.rs#L668)）。没有可信度概念，也没有时间比较 | P3 |
| 本地保存 | cloud 层存成 `data_dir/driver_metadata/cloud/revisions/<seq>/` 加 `state.json`，从不清理。合并结果不落盘，每次在内存里由四层重建 | P4 只做了一半 |
| 通道与灰度 | 只有 AICC 读 `BUCKYOS_UPDATE_CHANNEL` / `BUCKYOS_ROLLOUT_GROUP` 环境变量，`supported_features` 写死为空（[service/mod.rs:497-501](../src/frame/aicc/src/service/mod.rs#L497-L501)）。系统里没有升级通道 | 旧 D6 |
| 可更新种类 | `model_driver`、`provider_rules`、`known_provider`。逻辑树写在 Rust 里（[model_defaults.rs:594](../src/frame/aicc/src/service/model_defaults.rs#L594)、[:997](../src/frame/aicc/src/service/model_defaults.rs#L997)） | 旧 D10 |
| 价格 | 只在 Provider Rules 顶层 `model_pricing`（[catalog/schema.rs:600](../src/frame/aicc/src/catalog/schema.rs#L600)），每条都是绝对价：12 个 Provider 共 334 条（glm 101、qwen 96、openai 34、gemini 34……）。协议文档和 [provider_pricing_sources.md](../doc/aicc/provider_pricing_sources.md) 都规定 Model Driver 不带任何价格 | P7 |
| 默认无价格 | custom Provider 的规则体是 `{}`，没有价格；OpenRouter、SN 没有静态价，OpenRouter 用 discovery 实时价加 `reported_cost` | P8 的“默认无价格”已满足 |
| 价格来源顺序 | Provider Rules 静态价优先于 discovery 价（[provider/inventory.rs:1034-1066](../src/frame/aicc/src/provider/inventory.rs#L1034-L1066)）。结算时 Provider 响应里的实际费用优先 | 见 Q12 |
| Rust 里的价格特判 | 豆包 `/plan/` 端点清空价格（[inventory.rs:1047](../src/frame/aicc/src/provider/inventory.rs#L1047)、[call/mod.rs:638](../src/frame/aicc/src/call/mod.rs#L638)）；fal 的价格查询挂在 `provider_profile_id == "fal"` 上（[registry.rs:313](../src/frame/aicc/src/provider/builtin/registry.rs#L313)） | Provider 事实不该写在 Rust 里 |
| 管理接口 | `driver_metadata_update.get/set`（[management.rs:750](../src/frame/aicc/src/service/management.rs#L750)），Desktop 在 [CloudUpdateCard.tsx](../src/frame/desktop/src/app/ai-center/components/home/CloudUpdateCard.tsx)。`set` 清不掉 `source_url`（[ports.rs:204](../src/frame/aicc/src/service/ports.rs#L204)）。RBAC 在 [rbac_config.rs:188](../src/kernel/buckyos-api/src/rbac_config.rs#L188) | 要重做 |
| 文档与代码 | 协议文档说下载、验签、防回退归 NDN，AICC 不重复实现；实际全在 `cloud_update.rs` 里（旧 D11） | P5 定了方向 |
| 验收用例 | `test/aicc_test/acceptance/cloud_update_cases.ts`、`cloud_update_fixture_service.ts`、`cloud_update_transaction.ts` 按旧协议写 | 要重写 |

### 2.1 系统里能复用的部件

没有现成的“按可信度、再按时间合并多个签名来源”的组件，但零件基本齐全：

| 需要 | 已有 | 位置 |
| --- | --- | --- |
| JSON 对象的 ObjId | JCS（RFC 8785）后取 sha256；签名形式和未签名形式的 ObjId 相同 | cyfs-ndn `ndn-lib/src/object.rs`：`try_build_named_object_by_json` :373、`named_obj_to_jwt` :463（EdDSA，header 带 kid）、`verify_named_object_from_jwt` :425 |
| 按 kid 找 DID 的公钥 | `get_auth_key(kid)`、`is_key_allowed_in_scope(scope, kid)`，作用域常量 `ZONE_PUBLISH = "zone:publish"` | buckyos-base `name-lib/src/did.rs` :658、:698，`key_scope.rs` :3；`name-client` `resolve_auth_key` lib.rs:208 |
| 验证“发布者签的 Named Object JWT” | homestation 的 `verify_jwt(&dyn Directory, jwt, expected_type)`：查 `publisher` 声明、kid 授权、签名、ObjId | [homestation/src/objects.rs:201](../src/frame/homestation/src/objects.rs#L201)，`Directory` trait 在 [directory.rs:36](../src/frame/homestation/src/directory.rs#L36) |
| 防回退的先例 | 新候选的 `iat` 比现有的小就拒绝；`iat` 相同但内容不同算冲突 | [node_daemon/src/zone_boot_resolve.rs:313](../src/kernel/node_daemon/src/zone_boot_resolve.rs#L313)（LKGS 写法：tmp 文件 + rename，:108）；name-client `resolver/doc_result.rs` :551-563 |
| 多来源分级的先例 | name-client `SourceTier { Authority, Trust, Normal }`。级别来自本机登记，不来自远端内容；Normal 来源必须验签 | buckyos-base `name-client/src/resolver/source.rs` :17 |
| 订阅与拉取的先例 | homestation 订阅源列表，拉取签名的 Feed Head 再验签，同一条目只保留最大 `seq` | `homestation/src/sources.rs`、`pull.rs:240`、`ingress.rs:103` |
| 官方发布者 DID 的挂点 | `BuckyOSMachineConfig.trust_did` 默认 `did:web:buckyos.org` / `buckyos.ai` / `buckyos.io`。已声明但除测试外没有代码读它 | buckyos-base `buckyos-kit/src/machine_config.rs` :17-40，`rootfs/etc/machine.json:7` |
| 取对象 | `CyfsNdnClient` 可以钉住 ObjId 取对象。`cyfs-path-obj` 由 URL 主机所在的 Zone 签名，不是钉住的发布者，所以不能当本单的信任根 | cyfs-ndn `ndn-toolkit/src/cyfs_ndn_client.rs`：`NameClientPathVerifier` :79 |

没有可复用的：repo_service 和 package-lib 都不对 meta 验签（`decode_jwt_claim_without_verify`）；ext-tools 的在线源只用 HTTPS 加 sha256 固定；cyfs-gateway 的 `ConfigMerger` 远程 include 不做验证。

## 3. 通用组件：配置订阅（暂名 `config_feed`）

### 3.1 概念

- **配置项（item）**：一个 JSON Named Object（对象类型暂定 `cfgitem`）。身份是 `namespace`（使用者，例如 `aicc`）、`kind`、`id` 三者。头部带 `issued_at`、`schema_version`、可选的 `required_features`，正文 `content` 由使用者定义，组件不解析。
- **发布者（publisher）**：一个 DID，用它 DID Document 里授权的密钥签名。
- **源快照（feed snapshot）**：发布者签名的 Named Object JWT（对象类型暂定 `cfgfeed`）。它列出该发布者在这个命名空间、这个通道下当前提供的全部配置项 `(kind, id, obj_id, issued_at, schema_version, required_features)`，自身带 `issued_at` 和可选的 `expires_at`。一次发布只签这一次，各项内容靠 ObjId 绑定。
- **订阅源（source）**：`{ source_id, base_url, publisher_did, trust_level, enabled, interval_secs }`。`base_url` 只决定从哪儿取，信不信只看快照是不是 `publisher_did` 签的。所以官方源可以有镜像（CDN、邻近 Zone、离线拷贝），镜像不需要另外信任，`https://` 和 `cyfs://` 都可以用。
- **可信度（trust_level）**：整数，越大越可信，只来自本机配置，不来自远端内容（同 name-client `SourceTier`）。建议的预设（见 Q3）：
  - system-config 覆盖：1000（固定，不签名）
  - 本机 local 目录覆盖：900（固定，不签名）
  - 官方：100（预装版本和预置的官方订阅源）
  - 用户添加的源：默认 50，管理员可改，最高到 899
- **预装版本（preinstalled）**：嵌入二进制的官方快照，可信度等于官方，每项的 `issued_at` 与官方源发布的同一项一致（见 §5）。

### 3.2 合并规则

对每个 `(kind, id)`：

1. 候选来源有三类：预装版本，各订阅源最近一次验证通过的快照里的该项，各本地层的该项。
2. 去掉本机不支持的候选（`schema_version` 不认识，或 `required_features` 不全），以及单项校验（`validate_item`）不过的候选。
3. 在剩下的候选里按 `(trust_level, issued_at)` 取最大。
4. `trust_level` 和 `issued_at` 都相同但 ObjId 不同时，算冲突：保留上一次合并选中的那个，没有上一次就取订阅源列表里排在前面的，并在状态里记下冲突。
5. 源快照不再列出某项，表示该源不再提供它，低一级的候选重新显现。这和现在 tombstone 的语义相同。要不要支持“撤销”（发布者声明某项作废，连预装的也压掉）见 Q6。
6. 合并完成后交给使用者做整体校验（`validate_merged`）。不通过时不采用这次的新输入：保留上一次的合并结果，并记下是哪个源的哪次快照引起的。

另外：
- `issued_at` 比本机时间超前太多（建议超过 24 小时）的快照和配置项一律拒绝，防止出现一个“永远最新”的项。
- 一个快照可以对同一 `(kind, id)` 列出多个 `schema_version` 不同的变体，客户端取自己支持的最新那个。这替代了旧协议的 track，让还在跑旧客户端的 Zone 也能收到它看得懂的更新（见 Q9）。

### 3.3 获取与验证流程

对每个启用的源，按各自的间隔加随机抖动执行：

1. 取 `<base_url>/<namespace>/<channel>/snapshot.jwt`。路径约定见 Q9。**请求不带任何本 Zone 凭据。**
2. 验签：`publisher_did` → DID Document → header 的 kid → 公钥，同时检查这把密钥被授权用于发布（作用域见 Q7）。不是这个 DID 签的、签名不对、`namespace` 不符，一律拒绝。可以直接移植 homestation `verify_jwt` 的流程。
3. 防回退：快照的 `issued_at` 不得小于这个源已接受的快照；相等但 ObjId 不同算冲突，拒绝（同 `check_zone_smooth_upgrade`）。
4. 只下载本机还没有的 ObjId，用 `CyfsNdnClient` 钉住 ObjId 取，取到后重新计算 ObjId 校验。
5. 对每一项先检查快照里的 `(kind, id, issued_at)` 与对象头一致，再做 `validate_item`。
6. 用这份新快照试算一次合并，再做 `validate_merged`。
7. 全部通过后依次：原子替换这个源的缓存，写新的合并结果，推进合并代号，发事件。

任何一步失败，这个源都保持旧缓存，合并结果不变。失败原因写进状态，下次按指数退避重试，上限是这个源的正常间隔。

快照的 `expires_at` 过期只告警（源停更或被冻结），不丢弃已有数据（P4）。

### 3.4 本地保存

```text
<data_dir>/config_feed/<namespace>/
  sources/<source_id>/snapshot.jwt      # 最近一次验证通过的快照
  sources/<source_id>/accepted.json     # 已接受的 issued_at 高水位、上次成功/失败时间与原因
  objects/<obj_id>.json                 # 按 ObjId 存的配置项，各源共用
  merged/state.json                     # generation、内容摘要、预装指纹、每项的来源（source_id / 本地层 / 预装、trust、issued_at、obj_id）和被遮住的候选
```

- 写法：tmp 文件 + fsync + rename，同 LKGS。
- 启动：`merged/state.json` 有效、预装指纹没变、引用的对象都在时，直接使用，不联网。预装指纹变了（二进制升级）、本地层变了、或文件损坏时，用各源缓存、预装和本地层重新合并。
- `generation: u64` 只增，只在合并后的内容摘要变化时加 1。
- 清理：`objects/` 只保留当前合并结果和各源当前快照引用的对象（修旧 TODO §2“`revisions/` 从不清理”）。
- 新增的持久数据要登记到 [node_updater/readers.toml](../src/kernel/node_updater/readers.toml)（`kv/services/aicc/...` 新 key 和 `config_feed` 目录）。

### 3.5 接口草案（Rust）

```rust
pub struct ItemKey { pub kind: String, pub id: String }

pub struct ItemHeader {
    pub namespace: String,
    pub key: ItemKey,
    pub issued_at: u64,             // 秒
    pub schema_version: u32,
    pub required_features: Vec<String>,
}

pub struct FeedItem { pub header: ItemHeader, pub content: serde_json::Value, pub obj_id: ObjId }

pub struct SourceConfig {
    pub source_id: String,
    pub base_url: String,
    pub publisher_did: String,
    pub trust_level: i32,
    pub enabled: bool,
    pub interval_secs: u64,
}

#[async_trait]
pub trait FeedConsumer: Send + Sync {
    fn supports(&self, header: &ItemHeader) -> bool;
    fn validate_item(&self, item: &FeedItem) -> Result<(), String>;
    async fn validate_merged(&self, merged: &MergedSet) -> Result<(), String>;
}

#[async_trait]
pub trait LocalLayer: Send + Sync {   // 本机覆盖：AICC 的 local 目录、system-config
    fn layer_id(&self) -> &str;
    fn trust_level(&self) -> i32;
    async fn load(&self) -> Result<(String /* revision */, Vec<FeedItem>), String>;
}

pub struct MergedSet {
    pub generation: u64,
    pub digest: String,
    pub items: BTreeMap<ItemKey, SelectedItem>, // 含来源和被遮住的候选
}

impl ConfigFeed {
    pub async fn open(opts: FeedOptions /* namespace, data_dir, preinstalled, local_layers, consumer, fetcher, verifier */) -> Result<Arc<Self>>;
    pub fn current(&self) -> Arc<MergedSet>;            // 启动后总是有值
    pub fn subscribe(&self) -> watch::Receiver<u64>;    // generation
    pub async fn set_sources(&self, sources: Vec<SourceConfig>) -> Result<()>;
    pub async fn check_now(&self, source_id: Option<&str>) -> Result<CheckReport>;
    pub async fn reload_local(&self) -> Result<()>;
    pub async fn status(&self) -> FeedStatus;            // 各源状态 + 每项来源
    pub async fn start(self: &Arc<Self>);
    pub async fn shutdown(&self);
}
```

- `fetcher`、`verifier` 都是 trait，测试用假实现（测试 DID 和测试密钥），生产用 `CyfsNdnClient` 加 name-client。
- 发布端辅助函数放在同一个 crate，供 §5 的工具使用：`build_item`、`build_snapshot`、`sign_snapshot(key, kid)`。
- 组件不读 system-config，订阅源列表由使用者读出来后通过 `set_sources` 交给它。这样组件不必知道各服务的 key。

### 3.6 放在哪

建议新建 crate `src/kernel/config_feed`，依赖 `ndn-lib`、`ndn-toolkit`、`name-lib`、`name-client`（`buckyos-api` 已经依赖这四个，AICC 也直接依赖其中三个）。不放进 `buckyos-api`，免得所有服务都带上它。见 Q8。

### 3.7 不做

- 不做字段级合并。
- 不自动信任新来源：订阅源只能由管理员添加。
- 不上报更新状态（旧 D12 另定）。
- 不做 TypeScript 版本。将来前端或 Deno 侧需要时再按同一格式实现。

## 4. 官方源与默认配置

- 预置一个官方订阅源：`base_url` 和 `publisher_did` 待定（旧 D1）。发布者 DID 建议从 `BuckyOSMachineConfig.trust_did` 里选一个，或者让这个列表成为“可以作为官方可信度的发布者”名单。
- 默认启用，间隔 1–6 小时加抖动（旧 D4，待定）。管理员可以停用，但不能删除，也不能改它的 `publisher_did`。
- 没有网络、官方源停用或从未成功时，靠预装版本工作。

## 5. 发布端：工具、锁文件与预装版本

- **仓库是唯一事实源**：`src/frame/aicc/driver_metadata/` 下的 JSON 文件。官方源发布的内容和预装版本都由同一个工具从这个目录生成。云端热修也必须先提交到仓库再发布，不允许只在云端改。
- **锁文件** `src/frame/aicc/driver_metadata/feed.lock.json`，提交进仓库，每项记 `content_obj_id` 和 `issued_at`。工具逐项比较内容：内容变了才给新的 `issued_at`，没变就沿用。因此预装版本和云端的同一项字节一致，新二进制只覆盖它真正改过的项。
- **CI 检查**：用 cargo test 重新计算每项的 `content_obj_id`，与锁文件不一致就失败，提示“改了 metadata 但没跑工具”。这条检查替代现在每个文件手动加 `revision_seq` 的做法。各文件里的 `revision_seq` 字段要不要保留见 Q10。
- **工具命令**（放在 `config_feed` crate 的 bin 里）：`lock`（更新锁文件）、`build`（生成配置项和快照）、`sign`（用发布密钥签快照）、`publish`（上传到官方源）。`build` 的产物同时用于嵌入二进制。
- 发布前门禁：catalog 校验 + `ModelRegistry` 试建 + 三组快照测试（wire、库存、发现请求，跑法见 [协议客户端 TODO](aicc-provider-protocol-client-todo.md) 的 P1-0 和 §9）全部通过才允许 `sign`。
- 密钥保管、谁能发布、要不要审批（价格改动尤其敏感），仍待运维定（旧 D5）。

## 6. AICC 接入

### 6.1 通过订阅更新的种类

| kind | 文件 | 说明 |
| --- | --- | --- |
| `model_driver` | `models/*.model.json` | 含本单新增的相对价格（§7） |
| `provider_rules` | `providers/*.provider.json` | 官方 Provider 的配置，含 OpenRouter 的厂商前缀映射和模型改名、`model_pricing`、本单新增的价格基准（§7） |
| `known_provider` | `known-providers/*.known-provider.json` | |
| `logical_tree` | `logical-tree/*.logical-tree.json`（新增） | 旧 TODO §5，见 §8 |

custom Provider 实例的规则不走订阅，仍然是实例 settings 里的规则体。

### 6.2 替换现有实现

- 删除 `service/cloud_update.rs` 的 index / manifest / track 协议、`NdnCloudObjectFetcher` 和 `revisions/` 存储，改用 `config_feed`，命名空间 `aicc`。
- `MetadataSourceManager` 不再自己做四层选择：
  - builtin 变成组件的预装版本（§5 的 `build` 产物）；
  - cloud 变成组件的订阅源；
  - local 目录和 system-config 变成组件的两个 `LocalLayer`，路径 `etc/aicc/driver_metadata/local` 和 key `services/aicc/driver_metadata` 都不变。
  - `CatalogSnapshot` 由组件的 `MergedSet` 构建。
- `FeedConsumer` 的实现：
  - `supports`：`schema_version` 和 `required_features` 是否在本客户端支持的集合里；`supported_features` 不再写死为空。
  - `validate_item`：`MetadataFile::parse` 加单文件 schema 校验。
  - `validate_merged`：`CatalogSnapshot::from_current_files`，再用当前库存试建一次 `ModelRegistry`（修旧 TODO §2 第 5 条）。
- **`metadata_target_seq` 改为组件的 `generation`**。订阅更新、二进制升级带来的预装变化、本地覆盖变化都会推进它，Provider 收敛只看这一个值。删除 `BUILTIN_CATALOG_REVISION_SEQ`。`metadata_applied_seq` 的语义不变。
- **只改 metadata 时不做全量 discovery**：新增“沿用已保存的模型列表，用新 metadata 重建库存”的路径。discovery 只按它自己的定时器跑（修旧 TODO §2 第 4 条）。只改了 `logical_tree` 的更新连库存都不重建，只重建 `ModelRegistry`。
- local 和 system-config 的变化仍然从 settings reload 路径进来，由它调用 `reload_local()`。文件监听不做，文档里写明（旧 TODO §2 最后一条）。

### 6.3 管理接口与界面

- 用新的 RPC 替换 `driver_metadata_update.get/set`：
  - `metadata_sources.list`：订阅源及其状态（最近成功和失败时间、失败原因、快照 `issued_at`、是否过期）；
  - `metadata_sources.add / update / remove`：官方源只能停用；
  - `metadata_sources.check_now`；
  - `metadata_items.list`：每个 `(kind, id)` 当前生效的来源、可信度、`issued_at`，以及被它遮住的候选。
- 源列表存在 system-config 新 key `services/aicc/metadata_sources`，RBAC 只给 admin 读写，替换 [rbac_config.rs](../src/kernel/buckyos-api/src/rbac_config.rs) 里 `driver_metadata_update` 那两处。
- 同步 `buckyos-api/src/aicc_client.rs` 和 Desktop 的 `api/aicc_mgr.ts`。`buckyos_admin` 等 skill 手册里提到这些 RPC 的地方要一起改（`AGENTS.md` 对 skill 同步的要求）。
- Desktop：[CloudUpdateCard.tsx](../src/frame/desktop/src/app/ai-center/components/home/CloudUpdateCard.tsx) 改成“配置订阅”：源列表、添加源（URL + 发布者 DID + 可信度）、立即检查、失败原因。本地覆盖遮住了更新的官方版本时，要明确提示（“本地覆盖的 openai Model Driver 遮住了 2026-12-03 发布的官方版本”）。界面按手机宽度设计和验收。

## 7. 价格：Model Driver 相对价格 + Provider 价格基准

### 7.1 要解决的问题

现在 334 条绝对价，每个渠道的每个模型一条。官方调价或新模型上线要逐条改；按官方价或固定折扣转卖的第三方渠道没法复用官方价，只能整份复制。P7 的做法是：模型之间的比例写在 Model Driver 里，各渠道共用；每个渠道只写“1.0 值多少钱”。

### 7.2 数据里的事实（决定 schema 的形状）

- **只用一个标量比例表达不了同一价格组内的差异。** OpenAI 的 `gpt-6.1-sol` 缓存输入价是输入价的 5%（0.1 / 2.0 美元每百万 token），`gpt-6-sol` 是 10%（0.2 / 2.0）。如果每个模型只有一个比例，就必须让组内所有模型的输入、输出、缓存比例相同，这两者中必有一个算错。
- **同一个 Provider 的不同模态，币种之间的比值不一样。** MiniMax 文本模型 M3.1-Flash 是 0.15 USD 对 1 CNY（6.67 倍，输入、输出、缓存三项一致）；`speech-2.8-hd` 是 1e-4 USD 对 3.5e-4 CNY（3.5 倍）；`image-01` 是 0.0035 USD 对 0.025 CNY（7.14 倍）。所以一个 Model Driver 里要能分几个价格组，每组单独给基准值。
- **同一个价格组里，各币种的比例也可能不一致。** MiniMax `speech-2.8-turbo` 是 6e-5 USD 对 2e-4 CNY（3.33 倍），和 hd 的 3.5 倍不同。即使分了组，USD 和 CNY 两套比例也只能有一套精确。按 P8 这可以接受（不准确但不是估算），也可以对另一个币种写例外，见 Q11。

### 7.3 schema 草案

Model Driver 新增价格组和每个模型的相对价格：

```json
{
  "price_groups": [
    { "id": "text",  "base": { "model": "gpt-6.1-terra", "dimension": "input_token" } },
    { "id": "image", "base": { "model": "gpt-image-2",   "dimension": "input_token" } }
  ],
  "models": [
    {
      "id": "gpt-6.1-sol",
      "relative_pricing": { "group": "text", "input_token": 2.0, "output_token": 10.0, "cache_input_token": 0.1 }
    }
  ]
}
```

- 每个价格组的 1.0 等于基准模型在基准维度上的价格。token 维度一律按每百万 token 计，按单位计价的维度按每单位计。基准模型的基准维度必须是 1.0。
- `relative_pricing` 的字段名和结构与现有 `Pricing` 一致（各 token 维度、`unit` + `amount`、`tiers`、`rules`、`addons`），只是没有 `currency`、`source_url`、`verified_at`，数值是相对 1.0 的倍数。`time_windows`（峰谷时段）是渠道行为，不允许出现在 Model Driver 里。
- 只在 `models[]` 的精确项上允许 `relative_pricing`，`patterns[]` 上不允许，免得一个通配规则给一批模型配上价格。
- Model Driver 仍然不能出现绝对价和币种，原则改为“Model Driver 只有相对价格，且不单独生效”。

Provider Rules 新增价格基准：

```json
{
  "pricing_basis": [
    {
      "model_driver": "minimax",
      "group": "text",
      "match": { "region": "global" },
      "values": { "USD": 0.15 },
      "source_url": "https://platform.minimax.io/docs/guides/pricing-paygo",
      "verified_at": "2026-11-27"
    },
    {
      "model_driver": "minimax",
      "group": "text",
      "match": { "region": "china" },
      "values": { "CNY": 1.0 },
      "source_url": "https://platform.minimaxi.com/docs/guides/pricing-paygo",
      "verified_at": "2026-11-27"
    }
  ]
}
```

- `values` 是“这个组的 1.0 在各币种下值多少钱”。不同币种的值不要求符合汇率（P7）。
- `match` 的维度和 `model_pricing` 现在用的一样（例如 `region`）。同一个实例匹配到多个币种时怎么选，见 Q11。
- 算价：渠道价 = 相对倍数 × 该组基准值；token 维度再除以 1e6，得到现有 `Pricing` 的每 token 单价。`tiers` 和 `rules` 里的数值按同一倍数缩放。
- **显式集成**：只有 Provider Rules 的 `pricing_basis` 命中了 `(model_driver, group)`，并且该模型在 Model Driver 里有 `relative_pricing`，才产生价格。任何一边缺失，结果都是 unknown。custom Provider 和没有写 `pricing_basis` 的 Provider 默认没有价格（P8）。

### 7.4 来源顺序

库存构建时按下面的顺序取第一个可用的价格（第 1、2 两项的先后见 Q12）：

1. discovery 实时价（协议拿到的，例如 OpenRouter）；
2. Provider Rules `model_pricing` 精确项，用于渠道例外、峰谷时段、按请求参数计价；
3. Provider Rules `pricing_basis` × Model Driver `relative_pricing`；
4. unknown。

结算仍然以 Provider 响应里带币种的实际费用为先，不变。库存和用量日志里的 `PricingSource` 增加一个 `ProviderBasis`，并记下用到的 `(model_driver, group, 基准条目)`，让界面和账单看得出这是按比例算出的价格。

### 7.5 迁移现有 334 条

- 按官方直连 Provider 逐个迁移：用它的官方价格表推出 Model Driver 的相对价格，再写它的 `pricing_basis`。能由比例精确复现的 `model_pricing` 条目删除，复现不了的留作例外。迁移范围见 Q14。
- 第三方渠道（例如豆包上聚合的 DeepSeek、GLM，用的是方舟渠道价）不改，除非维护者核实了它和官方价成比例。永远不自动推断。
- **验收**：迁移前后各导出一份库存快照（`AICC_INVENTORY_SNAPSHOT_DUMP`），逐个模型比较价格，相对误差不超过 1e-9，币种和计价单位完全一致。golden 存的是摘要，直接比摘要会因浮点末位不同而失败，所以要比较 dump 出来的数值，确认后再 `AICC_INVENTORY_SNAPSHOT_UPDATE=1`。
- 两处 Rust 特判改成 metadata：豆包 `/plan/` 端点不出价，改成 Provider Rules 的 `match` 条件（`plan` 端点不命中任何价格）；fal 的价格查询属于协议客户端 TODO 的 P1-11，这里只登记。
- 同步文档：[provider_pricing_sources.md](../doc/aicc/provider_pricing_sources.md) 的“Model Driver 不保存价格”，[driver_metadata_update_protocol.md §5.1](../doc/aicc/driver_metadata_update_protocol.md) 的“不得包含 `model_pricing`、兜底成本估值”，以及 [driver_metadata_schema.md](../doc/aicc/driver_metadata_schema.md)、[provider_profile_schema.md](../doc/aicc/provider_profile_schema.md)。

### 7.6 对路由的影响

只迁移现有价格时价格不变，路由不变。之后官方直连 Provider 新加的模型会自动有价格（以前是 unknown），这会影响 `max_estimated_cost` 上限和同权重池内的 `cost_first` 评分。实施报告里要列出“从 unknown 变成有价格”的模型清单。路由仍然遵守“不自动换模型”：价格只能帮管理员做判断，不能成为自动切换 Provider 的理由（参见 [显式 fallback TODO](aicc-explicit-fallback-todo.md) 的 Q7）。

## 8. 逻辑树纳入订阅（旧 TODO §5，整体保留）

方向已在 2026-09-25 定下：要做，而且意义比 Model Driver 更新还大。逻辑树承载“特定用途下的默认模型优先级”，是最需要随市场频繁调整的部分。

### 8.1 现状

`ModelRegistry::build`（[model/mod.rs](../src/frame/aicc/src/model/mod.rs)）每次收敛都完整重建逻辑树，输入有四部分：

| 部分 | 当前来源 | 能否外置为数据 |
| --- | --- | --- |
| 任务节点定义（path、api_type、min_line、mount_mode、scheduler_profile、fallback、tier） | `builtin_logical_model_definitions()`，用 Rust 构造的 `LogicalModelDefinition`（不是 serde 类型） | 需要新 schema |
| 功能路径 → 规格引用及权重、媒体家族权重 | `builtin_logical_tree_overlay()` + `MEDIA_FAMILIES`，产出公共类型 `AiccRouteOverlay`（已可 serde） | 几乎直接可用 |
| 规格、家族、driver mount、auto mount | Model Driver catalog + 库存 | 已是数据 |
| 用户调整 | settings `session_config`（`AiccRouteOverlay` + `routing_commands`），每次重建后重放 | 已是数据 |

### 8.2 目标形态

- 新 kind `logical_tree`，文件放 `src/frame/aicc/driver_metadata/logical-tree/<id>.logical-tree.json`，和另外三类一样经过订阅合并。
- 拆成两个 id，方便分别更新：
  - `tasks`：任务节点定义（变动少、风险高）；
  - `defaults`：功能路径 → 规格、家族引用及权重，以及全局权重（变动多、风险低，订阅首版主要发它）。
- 内容是 `{ tasks: [...] }` 或 `{ overlay: <AiccRouteOverlay 子集：logical_tree、global_exact_model_weights、provider_weights> }`，外加和其它 catalog 一致的 `format`、`schema_version`、`catalog_id`。
- 用一个测试把现有 Rust 常量一次性导出成 builtin JSON，并做 golden 对比：导出前后的 `ModelRegistry` 完全一致（节点、引用、权重、fallback）。之后删除 Rust 常量，JSON 成为唯一事实源，不保留双轨。

### 8.3 运行时改动

1. `CatalogKind` 增加 `LogicalTree`。`CatalogSnapshot` 暴露 `logical_tasks()` 和 `logical_defaults_overlay()`。schema 校验：路径合法，api_type 与路径命名空间一致，权重有限且非负，引用目标形如 `llm.<spec>` 或家族路径。
2. `ServiceModelAssembler::build`（[model_defaults.rs](../src/frame/aicc/src/service/model_defaults.rs)）的定义和 factory overlay 改为从 catalog 读。逻辑树本来就在每次收敛时重建，catalog 一变树就跟着变，不需要新的触发机制。
3. 悬空引用：`defaults` 引用了当前 Model Driver 里不存在的规格时，忽略这条引用并记诊断事件（复用 `routing_command_stale` 一类事件），不让 registry 构建失败。
4. 任务节点 path 与规格或家族名冲突时，在 catalog 校验阶段判为非法，拒绝该项；运行时不应该走到 `register_specs` 报错。
5. 用户调整（`session_config`、`routing_commands`）继续在 factory 之后应用和重放，订阅更新默认值不改用户层。已有的 stale 检测覆盖“用户调整的目标被订阅删除”的情况。
6. WebUI 的路由和模型页显示每个权重的来源（预装、哪个订阅源、用户），提供“恢复默认”，也就是删除对应的用户命令。

旧 D10.1 已定为新增 kind；D10.3（用户调整优先）在新机制下自然成立。D10.2、D10.4 见 Q15。

## 9. 旧 TODO 决策项的去向

| 旧项 | 新机制下 | 状态 |
| --- | --- | --- |
| D1 默认源地址与发布者身份 | 预置官方订阅源的 `base_url` 和 `publisher_did`（§4） | 待定（运维） |
| D2 协议与信任链 | P2 定了方向：签名在快照对象上，由钉住的发布者 DID 验证，传输层不可信也不要紧，`https://` 也可以用，`cyfs-path-obj` 不再是信任根。还剩：密钥作用域、快照 `expires_at` 多长（Q7） | 部分已定 |
| D3 分发拓扑与流量 | 签名在对象上之后，镜像、CDN、邻近 Zone 都能当源。流量估算仍要运维做 | 待定（运维） |
| D4 默认开关与频率 | §4 | 待定 |
| D5 发布流程、密钥、权限 | `revision_seq` 怎么分配的问题不存在了（改为 `issued_at` + 锁文件，§5）。密钥保管、审批、CI 门禁仍待定 | 部分已定 |
| D6 投放维度 | 客户端兼容靠每项的 `schema_version` / `required_features` 加多变体（§3.2）。通道按快照路径区分（Q9）。灰度分组待定 | 部分已定 |
| D7 cloud 与 builtin 的关系 | 已解决：预装可信度与官方相同，按 `issued_at` 比较，锁文件保证没改过的项不变新；删除 `BUILTIN_CATALOG_REVISION_SEQ` | 待 owner 确认理解（Q4） |
| D8 应急与回滚 | 回滚就是用旧内容重新发布一次（新的 `issued_at`）。本机可用 local / system-config 覆盖止血。要不要 kill switch 或“撤销”见 Q6 | 待定 |
| D9 收敛失败时 Provider 是否继续可路由 | 与本单无关，保留为产品决策。建议同旧：保留旧库存继续可路由，标 Degraded 并退避重试，只有新 metadata 明确下架的模型才移除 | 待定 |
| D10 逻辑树 | §8 | 部分已定 |
| D11 NDN 与 AICC 的职责边界 | 已定（P5）：做成公共组件，不由 AICC 自己实现，也不归 NDN；组件用 NDN 客户端取对象 | 已定 |
| D12 可观测性与上报 | 本地状态由 `metadata_sources.list` / `metadata_items.list` 提供。要不要上报全网版本分布仍待定 | 待定 |

## 10. 旧 TODO 已确认缺陷的去向（2026-11-27 核对，都还在）

| 缺陷 | 处理 |
| --- | --- |
| service token 当 Bearer 发给任意 `source_url` | 组件取对象不带任何本 Zone 凭据，列为验收项 |
| index 没有完整性和来源校验 | 快照验签（§3.3） |
| 收敛失败不自动重试（推理路径不调 `before_inference`，`check_once` 没新版本时不再发事件） | 仍要修，与组件无关：收敛失败按退避重试，不依赖下一次订阅事件 |
| `reconcile_inventory` 对所有 Provider 串行做全量 discovery | §6.2“只改 metadata 时不做全量 discovery” |
| 下载阶段不试建 `ModelRegistry` | `validate_merged` 试建 |
| `revisions/<seq>/` 从不清理 | §3.4 清理 |
| `driver_metadata_update.set` 清不掉 `source_url` | 接口重做后不存在 |
| local / system-config 变更没有监听 | 不做监听，从 settings reload 调 `reload_local()`，文档写明 |

## 11. 实施步骤

**阶段 A：不依赖决策，可以先做**

- [ ] A1 `config_feed` crate：数据结构、合并规则、本地保存、启动恢复、清理、`issued_at` 超前拒绝、冲突处理；fetcher 和 verifier 用假实现，测试用测试 DID 和测试密钥签快照。单测覆盖 §3.2、§3.3 的每条规则。
- [ ] A2 AICC 接入组件，先只接预装和两个本地层，不接订阅源。验收：三组快照（wire、库存、发现请求）不变，local / system-config 覆盖行为不变。
- [ ] A3 `metadata_target_seq` 改为 `generation`，删除 `BUILTIN_CATALOG_REVISION_SEQ`；新增“沿用模型列表重建库存”的路径；收敛失败的退避重试。
- [ ] A4 发布工具的 `lock` / `build` 和锁文件、CI 检查；预装版本改成 `build` 的产物。
- [ ] A5 价格：`price_groups`、`relative_pricing`、`pricing_basis` 的 schema、校验、算价、`PricingSource::ProviderBasis`。测试至少覆盖：没写基准的 Provider 不出价；Model Driver 有比例但 Provider 没集成时不出价；`tiers` 和 `rules` 的缩放；多币种 `match`。先不迁移数据。
- [ ] A6 选一个官方直连 Provider 做价格迁移示范（建议 Claude：15 条、只有 USD、组内形状整齐），按 §7.5 验收价格不变。
- [ ] A7 逻辑树 §8.2、§8.3 的第 1–5 条（导出 + golden、Assembler 读 catalog、悬空引用降级）。只走预装和本地层，可以立刻用于调整默认优先级的 DV。

**阶段 B：依赖 §12 和运维决策**

- [ ] B1 订阅源：真实 fetcher（`CyfsNdnClient`，不带凭据）和 verifier（name-client + 密钥作用域）、退避、过期告警。
- [ ] B2 预置官方源（D1、D4），`sign` / `publish`，以及发布门禁（D5）。
- [ ] B3 新 RPC、system-config key、RBAC、`aicc_client.rs`、`aicc_mgr.ts`、skill 手册、Desktop 界面（§6.3）。
- [ ] B4 其余 Provider 的价格迁移（Q14 定范围），两处 Rust 价格特判改成 metadata。
- [ ] B5 逻辑树的 WebUI 来源显示和“恢复默认”（§8.3 第 6 条）。
- [ ] B6 重写 `test/aicc_test/acceptance/cloud_update_*` 验收用例；`node_updater/readers.toml` 登记新的持久数据。
- [ ] B7 文档（§14）。

## 12. 待 owner 定的问题

每题给出我的建议，括号里是理由。

1. **配置项粒度。** 建议一个 catalog 文件为一项，整项覆盖。（与现在的来源选择单位一致，组件不需要懂内容。）
2. **“新”的定义。** 建议用发布者签进对象的 `issued_at`。（不同发布者的序号不可比；下载时间可以被重放操纵。）
3. **可信度的表示。** 建议用整数加预设（system-config 1000 / local 900 / 官方 100 / 用户添加默认 50），界面上按可信度排序显示，不暴露数字。另一个做法是固定三四档。
4. **预装版本的可信度是否等于官方。** 建议等于，再配合 §5 的锁文件。（这样可以同时解决旧 D7 的两个问题：云端永久遮住新二进制的改进，以及二进制升级不推进版本号。）
5. **本机覆盖是否固定在所有订阅源之上、且不要求签名。** 建议是。（这是现在的行为，也满足旧 D10.3 的“用户调整优先”。）副作用是本地覆盖会一直遮住更新的官方版本，所以界面要提示（§6.3）。
6. **要不要“撤销”。** 例如预装的某个 Provider Rules 价格错了，官方想让它作废，而不是发一个修正版。建议第一期不做：发修正版就能盖掉预装版本，紧急情况用本地覆盖。
7. **发布密钥。** 用发布者 DID 里哪个作用域的密钥签快照：复用 `zone:publish`，还是新增一个配置发布作用域？快照的 `expires_at` 设多长（只用于告警）？建议新增作用域，`expires_at` 设 30 天。（发布配置和发布 Zone 内容是两种权限。）
8. **组件放在哪、第一期给谁用。** 建议新建 `src/kernel/config_feed`，第一期只接 AICC，其它候选使用者（ext-tools 目录、内置 Agent 的 behavior、Known Provider 以外的系统目录）等 AICC 跑通后再定。
9. **通道和路径约定。** 建议快照路径为 `<base_url>/<namespace>/<channel>/snapshot.jwt`，通道复用将来的系统升级通道，不在快照里做 track。灰度分组第一期不做。
10. **catalog 文件里的 `revision_seq`。** 有了锁文件之后它就多余了，但库存记录（`provider_rules_revision`）和界面在用。建议本期保留、不再参与任何比较，下一轮清理。
11. **一个实例匹配到多个币种时怎么选，以及同组内各币种比例不一致时怎么办。** 建议：基准条目用 `match`（例如 `region`）选币种，像现在 `model_pricing` 的做法；匹配到多个币种时，实例必须设置结算币种，没设就 unknown，不自动换算。比例不一致时（MiniMax speech 的 3.5 倍和 3.33 倍），以主要币种为准，另一个币种写 `model_pricing` 例外，或者按 P8 接受误差。请 owner 定默认取哪一种。
12. **实时价和静态价谁优先。** 现在静态价（`model_pricing`）优先于 discovery 实时价。建议改成实时价优先（呼应“尽量通过协议拿实时价格”）。会受影响的是同时有两种价格的 Provider，例如 fal。
13. **“宁可不准确也不估算”的理解**（§1.3 最后一条）是否正确。
14. **价格迁移范围。** 建议官方直连 Provider 全部迁移（openai、claude、gemini、deepseek、glm、kimi、minimax、qwen、doubao、fal、typesafe），复现不了的留作例外；第三方渠道不动。另一个做法是只上新机制，现有 334 条保持不动，以后新增的才用比例。
15. **逻辑树的剩余决策**（旧 D10.2、D10.4）：订阅首版只开放 `defaults`（权重和引用），`tasks` 暂不通过订阅更新？权重调整能否走比 Model Driver 更轻的发布流程（例如运营发布、双人确认）？建议首版只发 `defaults`。

旧 TODO 里需要运维定的 D1、D3、D4、D5（密钥与审批部分）、D9、D12，见 §9。

## 13. 验收

**离线（cargo test）**

- 伪造签名、错发布者、错命名空间、`issued_at` 超前、快照回退、同 `issued_at` 不同内容，全部拒绝，且合并结果不变。
- ObjId 与内容不符时拒绝。
- 取快照和对象的请求里没有任何本 Zone 凭据（用假 fetcher 断言请求头）。
- 合并规则：高可信度胜；同可信度时新的胜；不支持的 `schema_version` 退回低一级候选；源不再列出的项退回低一级候选；冲突时保留上次选择。
- 某个源的新快照导致 `validate_merged` 失败（例如与逻辑树冲突、registry 试建失败）时，整份快照被拒，合并结果和 `generation` 不变。
- 断网重启后直接使用保存的合并结果；预装指纹变化后重新合并，并且只有改过的项变化。
- 锁文件 CI 检查：改了 metadata 内容但没跑 `lock` 时测试失败。
- 价格：§11 A5 列出的用例；迁移前后库存价格一致（§7.5）。
- 三组快照在 A2、A3 之后不变。

**测试环境**

- 用测试发布者 DID 搭两个源（可信度不同），走完整流程：发布 → 拉取 → 合并 → 收敛 → 界面显示来源。再验证四种情况：同级更新的源胜出；低可信度源发更新的版本不生效；用旧内容重新发布实现回滚；停掉所有源后照常工作。
- 镜像：同一份签名快照放到另一个 URL，作为官方源能被接受；内容被篡改的镜像被拒。

**DV（本机 Zone，需要 root 重启 AICC，要用户配合）**

- 开启官方源观察一个完整周期：`generation` 与各 Provider 的 `metadata_applied_seq` 一致，没有 unmatched 回归，价格来源字段正确（`ProviderRules` / `ProviderBasis` / `Discovery`）。
- 做一次只改 `defaults` 逻辑树的发布：不触发 discovery，路由权重按预期变化，用户调整保留。

## 14. 文档同步

- 重写 [driver_metadata_update_protocol.md](../doc/aicc/driver_metadata_update_protocol.md)：删除 index / manifest / track、全局 `revision_seq`、“NDN 负责验签”；改为引用 `config_feed` 的格式与合并规则，说明 AICC 的四种 kind。
- 重写 [driver_metadata_update_storage.md](../doc/aicc/driver_metadata_update_storage.md) §1–§3：`metadata_target_seq` = 组件的 `generation`，持久数据归组件。
- 新增组件文档（建议 `doc/arch/` 下，名称随 Q8）：概念、对象格式、合并规则、本地保存、发布工具。
- 价格相关的四份文档（§7.5）。
- 逻辑树：[aicc 逻辑模型目录.md](../doc/aicc/aicc%20逻辑模型目录.md)、[frozen_model_driver_and_logical_model_fs.md](../doc/aicc/frozen_model_driver_and_logical_model_fs.md)。
- [doc/aicc/README.md](../doc/aicc/README.md) 和 [provider_architecture_durable_data_schema.md](../doc/aicc/provider_architecture_durable_data_schema.md) 里提到 `driver_metadata_update` 的地方。
