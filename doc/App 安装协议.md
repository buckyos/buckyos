# BuckyOS App 安装协议规范（Draft v0.5）

> 本版本在原有安装引导、多源下载、信任校验和经济模型基础上，引入 **App Document 统一入口**、**`resolve_did(App DID, "app")` 可信解析**、**分阶段安装流水线** 与 **Personal AI Package（`.pikg`）** 文件格式。
>
> 本协议中的 **MUST / 必须**、**SHOULD / 应当**、**MAY / 可以** 分别表示强制要求、推荐要求和可选能力。

---

## 0. 本次修订摘要

本次修订解决的核心问题是：原有流程把对象解析、网络下载、内容校验、配置确认和本地部署混合在一次“安装”操作中。对于成熟生态和稳定网络，这一流程可以工作；但对于早期生态、弱网络、开发者本地调试、高级用户以及 Agent 自主构建并安装应用的场景，外部依赖过多，失败边界不清晰，安装成功率和开发循环效率均不理想。

本版本做出以下核心调整：

1. **App Document 是安装流程的统一起点。** 名称、DID、Object ID、URL、分享对象和应用商店条目都只是取得 App Document 的不同入口。
2. **引入 `.pikg` 文件格式。** `.pikg` 将 App Document、其引用的部分或全部 Package Meta，以及部分或全部实体内容组织为一个可携带、可校验、可离线安装的包。
3. **将“下载”和“安装”明确分离。** 推荐流程由“点击安装后再逐步下载”调整为“先取得本地 `.pikg`，确认内容就绪，再进入实际部署”。
4. **安装流程拆分为多个可恢复 Stage。** 每个 Stage 具有明确输入、输出、中间状态、失败原因和重试边界。
5. **允许面向当前安装目标的部分包。** App Document 可以声明多个平台，但一个 `.pikg` 只需携带当前构建或分享场景所需的平台内容。
6. **支持开发与 Agent 自构建短路路径。** 对本地、自有、明确授权的开发包，可以跳过公开发行场景中的部分外部信任和网络解析步骤，但不能跳过基础安全与内容一致性检查。
7. **DID 解析按 `(did, doc_type)` 执行。** App 的解析单元固定为 `(App DID, "app")`；包、URL、应用商店和分享渠道只提供候选 body 或内容下载位置，不能自行取得 DID method 的发布权威地位。
8. **分离签字权与发布权。** App Document 签名证明 owner 授权构造了该内容，权威发布状态证明该内容已经公开生效；已签名但未发布的候选不能覆盖已发布结果或撤销状态。
9. **交付格式统一命名为 `pikg`。** 文件扩展名固定为 `.pikg`，避免被误读为 Python `pip` 包或重量单位组合。
10. **统一 subpackage 归档命名。** `pikg` 内携带的 subpackage 优先命名为 `$sub_pkg_name.tar.gz`，归档文件 hash 必须与对应 Package Meta 的内容配置一致。
11. **绑定 App DID 与 subpackage 命名空间。** AppInstaller 接受外部 App Document 时，必须从可信 App DID 派生该 App 可占用的 package namespace，并拒绝 `pkg_list` 中越权占用其它 App、系统包或宿主目录名字的 Package ID。

---

## 1. 协议目标与设计原则

### 1.1 协议目标

本协议旨在提供一套统一、去中心化、对工程实践友好的 App 安装标准，使第三方网页、应用商店、好友分享、本地文件、开发工具和 Agent 均可引导用户在 BuckyOS 上安装 App。

协议覆盖：

- App 身份与 App Document 解析；
- `.pikg` 文件交付；
- 多源下载与本地缓存复用；
- 内容完整性、签名与信任校验；
- 安装参数收集、权限确认、部署与启动；
- 分享、收录、推荐和应用商店聚合；
- 支付、安装成功证明与生态激励；
- 升级、回滚和生命周期管理。

### 1.2 核心设计原则

#### 1.2.1 App Document First

安装流程从 App Document 开始。任何入口最终都必须归一化为 `(App DID, "app")` 的解析结果和一份与该结果一致的 App Document body。

#### 1.2.2 Content Addressed

Package Meta、Chunk 和其他对象继续使用 Object ID、Chunk ID 或 Digest 进行内容寻址。对象从哪个 Source 获得，不影响其身份与完整性判断。

#### 1.2.3 Download Before Install

网络获取、对象补齐和完整性校验应尽可能发生在实际部署事务之前。系统只有在当前安装目标所需内容和 DID 信任均已就绪后，才进入 Prepare、Deploy 和 Activate 阶段。

#### 1.2.4 Offline First, Network Optional

Installer 必须分别判断当前目标是否“内容离线就绪”和“DID 信任离线就绪”。若所需内容与可接受的解析证据已经存在于 Zone Resolver、本机 DID cache、Object Store、已安装内容或 `.pikg` 中，安装过程不应强制访问网络。

#### 1.2.5 Stage Isolation

解析、规划、获取、验证、准备、部署和启动应彼此隔离，允许持久化、恢复、重试和短路。

#### 1.2.6 DID Resolution Is Method-scoped

DID resolver-provider 由内核按 DID method 配置，每个 method 至多一个权威发布渠道，并按显式顺序查询少数补充源。App、`.pikg`、Source 或 Curator 不得动态注册 DID resolver-provider，也不得让其它 method 的结果覆盖目标 App DID。

#### 1.2.7 Trust Is Contextual

公开分发、本地开发、好友分享和自有应用不应强制使用完全相同的信任策略。系统应保留基础安全检查，同时根据来源和使用场景调整外部信任要求。

#### 1.2.8 Identity-bound Package Namespace

App DID 验证只证明“哪份 App Document 属于该身份”，不自动授权文档占用任意 Package ID、gateway server 名或宿主文件目录。Installer 必须把 App DID 绑定到一个确定的 package namespace；App 自有的全部 `pkg_list` entry 都必须位于该命名空间内。该约束属于身份授权和文件系统安全边界，不能由 Source、Curator、有效内容 hash 或本地开发模式绕过。

### 1.3 术语定义

- **OOD（Owner Online Device）**：个人 AI 服务器（Personal AI Server），用户的核心计算节点。
- **App DID**：应用的逻辑名字；以 `doc_type = "app"` 解析其当前可信 App Document。`did:key`、`did:dev` 等 key 类 DID 不能作为 `resolve_did` 入参。
- **App Document / APPDOC**：应用安装与运行的核心结构化描述文档。
- **App Document Object ID**：某一不可变 App Document 内容或签名对象的内容寻址标识。
- **doc_type**：同一个 DID 名字下独立发布的内容类型。App 安装协议使用固定值 `app`。
- **Document Result（DR）**：DID resolver 已经给出回答；回答可以是文档，也可以是 `Missing`、`Revoked`、`Tombstoned` 等发布状态。
- **unknown**：DID resolver 因断网、超时等原因没有得到回答；与权威源明确返回的 `Missing` 不同。
- **expected_owner**：验证 App Document 时使用的 owner，只能来自 DID method 权威源的 owner 绑定或名字结构的确定性默认值，不能由候选文档自证。
- **Package Meta**：描述某个 subpackage 的平台选择条件、实体内容、Chunk、依赖和启动信息的结构化对象。
- **subpackage**：App 的一个可独立选择、下载或部署的内容单元，例如特定平台的服务包、Web 资源、模型或 Agent 资产。
- **App Package Namespace**：从可信 App DID 确定性派生、由该 App 独占的 package name 前缀。它约束 `AppDoc.pkg_list` 中 App 自有 subpackage 的 Package ID，也约束由 Package ID 派生的 gateway server 名和宿主友好目录名。
- **pikg**：Personal AI Package 的规范短名称，扩展名为 `.pikg` 的 App 交付文件。
- **Curator**：应用收录源或收录人，为 App 提供收录证明、分类、评分或审查信息。
- **Source**：提供 App Document、Package Meta、Chunk 或 `.pikg` 下载的内容源。
- **Referrer**：推荐人或分享者。
- **Builder**：构建特定平台 subpackage 的主体。
- **Packager**：将 App Document 和相关内容组装为 `.pikg` 的主体。
- **Installer**：负责检查安装包、生成安装计划、补齐内容并执行本地部署的系统组件。
- **Object Store**：本地或网络中的内容寻址对象存储。

---

## 2. App Document：统一安装起点

### 2.1 App Document 的地位

现有入口可以只携带 App Object ID。该 Object ID 本质上只固定某个 App Document JSON 内容或签名对象，不能单独证明该内容是 App DID 当前已发布的结果。

从协议语义看，真正驱动安装流程的不是某一种 URL、名称或分发渠道，而是 **App Document**：

```text
App Identifier
    ↓ Normalize
(App DID, doc_type = "app") + candidate body / package location
    ↓ resolve_did + Fetch
Resolved App Document
    ↓ Inspect / Plan
Required Packages and Permissions
    ↓ Acquire / Verify
Installable Content Set
    ↓ Deploy / Activate
Installed App
```

### 2.2 App Document 与 DID

App Document 作为需要验证的 Document，解析单元固定为：

```text
resolve_did(app_did, doc_type = "app")
```

- App DID 表示逻辑应用身份；
- `doc_type = "app"` 表示在该名字下解析 App Document，而不是默认的 `zone` 文档；
- App Document Object ID 表示某一不可变 body；App Document 的 `version` 表示应用版本；resolver metadata 中的 `document_version` / W3C `versionId` 表示当前发布文档的 revision `iat`，三者不得混用；
- 同一 App DID 可以随生命周期发布多个 App Document，但普通解析返回当前生效结果。安装历史版本应使用被明确固定且仍可验证为已发布集合成员的 Object ID 或版本解析能力，不能把版本标签拼进 App DID 后假定其天然可信；
- 安装器必须记录 App DID、`doc_type`、实际安装的 App Document Object ID、发布版本和解析证据，不能只记录可变化的名称或应用版本字符串。

App Document 的 body 可以来自权威渠道、`.pikg`、Object Store、URL、好友或应用商店，但 body 的来源不决定其可信度。DID resolver 按取回信道标注 `evidence`，并执行分层校验：

```text
所有 body：document.did == app_did
Anchored body：权威源给出 doc_hash 时，body hash 与之相等
NeedProof body：document.owner == expected_owner
                ∧ hash 匹配（若有锚点）
                ∧ 由 iat 时刻有效的 owner key 验签
                ∧ 满足 owner document 的 valid_iat 等策略
```

验证 `NeedProof` body 的 owner 时递归调用 `resolve_did(expected_owner, "owner")`。Owner Document 是递归基，只接受 DID method 权威渠道或其明确锚定的结果。Installer 不得自行从 App Document 的 `owner` 字段建立信任链。

#### 2.2.1 App DID 与 Package Namespace 绑定

AppInstaller 是接受外部 App 的安全入口。任何外部 App Document 在进入 InstallPlan、内容获取或目录准备之前，必须完成 App DID 与 `pkg_list` Package ID 的命名空间归属校验。

对于本版本已经冻结映射的标准 BNS App DID：

```text
did:bns:$app_name.$owner_name
```

其中 `$app_name` 与 `$owner_name` 都是单个合法 BNS label。其 App Package Namespace 按与当前 `PackageId::from_did` 一致的规则派生：

```text
package_namespace = $owner_name + "_" + $app_name
```

例如：

```text
App DID:             did:bns:app1.user1
AppDoc.name:         app1
package_namespace:   user1_app1

允许的 subpackage unique name:
  user1_app1
  user1_app1-web
  user1_app1-agent
  user1_app1-amd64-docker-image

拒绝：
  app1
  user1_other-app
  user1_app10
  control-panel
  ../control-panel
```

采用 `$owner_name_$app_name` 而不是 DID 中的 `$app_name.$owner_name` 顺序，是为了把发布者命名空间放在平铺 package/file namespace 的最前部：同一 owner 的包可以按前缀归组，Installer 也可以在不信任候选文档自声明字段的情况下执行明确的所有权边界检查。

校验规则如下：

1. `AppDoc.did` 必须等于已解析的 App DID；对于上述标准 BNS 形态，`AppDoc.name` 必须等于 `$app_name`，`expected_owner` 必须与 `$owner_name` 对应的 owner DID 一致。
2. Installer 必须解析 `pkg_list` 中每一个 `SubPkgDesc.pkg_id`，取得去除 channel/platform env 前缀及版本/ObjId 后的 `unique_name`。
3. `unique_name` 必须等于 `package_namespace`，或以 `package_namespace + "-"` 开头。这里的 `-` 是命名空间边界；只做无边界的字符串 `starts_with(package_namespace)` 不合格，`user1_app10` 不能被 `user1_app1` 接受。
4. `unique_name` 必须是安全的单段名字，只能包含 ASCII 小写字母、数字、下划线和连字符，不得包含点、`/`、`\\`、`..`、绝对路径、控制字符或规范化后发生变化的编码。可选 env 前缀必须由目标 PackageEnv 识别，不能用自定义点前缀绕过 `unique_name` 校验。
5. `pkg_objid` 指向的 Package Meta，其 `name` 在去除合法 env 前缀后必须得到同一个 `unique_name`；内容寻址和签名正确不能替代命名空间归属校验，也不得通过静默改写 Package Meta 名称来“修复”不一致。
6. 本规则约束 `AppDoc.pkg_list` 中由该 App 部署并可能映射到宿主目录、gateway server 或运行入口的自有 subpackage。Package Meta 的第三方依赖可以位于其它 namespace，但必须通过独立的依赖解析、完整性和信任策略，不能因此取得当前 App 的目录或 gateway 名称所有权。
7. 标准映射尚未冻结的 DID method 或非标准 BNS 形态，必须由权威 resolver 结果提供等价、不可由候选 body 自声明的 `package_namespace` 绑定；在该能力实现前，外部公开安装必须拒绝，而不能猜测或截断 DID。`SYSTEM_INTERNAL` 例外必须来自系统编译期/管理员 allowlist，不能来自 App Document 自声明。

任一规则失败时，Installer 必须在 Inspect Stage 返回 `APP_PACKAGE_NAMESPACE_MISMATCH`（归类为 `INVALID_PACKAGE`），且不得 Acquire payload、创建目录、更新 PackageEnv 友好链接、生成 gateway 配置或进入 Deploy。该检查是所有安装策略等级的硬规则。

### 2.3 DID 解析结果与安装语义

`resolve_did_ex` 的对外结果应保留 W3C 风格的三段信息：`resolution_metadata`、`document` 和 `document_metadata`。Installer 使用的归一化结果至少包含：

```text
ResolvedAppDocument {
    app_did,
    doc_type: "app",
    app_doc_object_id,
    document,
    resolution_metadata {
        resolver_id, evidence, cache_status, verification_status, warnings, error
    },
    document_metadata {
        version_id,
        buckyos { doc_type, document_status, document_version, authority_seq }
    },
    expected_owner,         // 来自权威 owner 绑定或名字结构，不来自候选 body
    acquisition_context
}
```

Installer 必须区分以下结果：

| 解析结果 | 安装语义 |
|---|---|
| `Active` / 已锚定 | 权威信道 body 校验 `did` 和 `doc_hash` 后可以进入 Inspect；来自补充信道或包内的 `NeedProof` body 还必须完成 expected-owner 与签名验证。 |
| `Missing` | 权威源明确表示该 `(did, "app")` 从未发布；只有策略明确允许时，已验证的自签名候选才可进入本地开发或降级流程。 |
| `Expired` | 权威源明确表示发布结果已过期；与 `Missing` 一样，只有策略明确允许且重新完成 owner 验证的候选才可降级使用，不得当作网络 `unknown`。 |
| `Revoked` / `Tombstoned` | 终止安装或升级，清除正缓存；不得回退到旧包、旧 cache 或好友提供的候选。 |
| `Migrated` | 仅按 resolver 返回的 `migration_target` 继续解析，并向用户展示身份迁移；不得把跨 method 弱别名当作同一身份。 |
| `unknown` | 表示本次没有得到权威回答；优先使用未作废的已验证 cache。宽松策略下的观察候选必须以 `evidence = NeedProof`、`cache_status = ObservedFallback` 和未通过的 `verification_status` 明确标记，公开安装默认拒绝。 |
| `LocalAuthorityOverride` | 仅限带 scope 的 Zone / 本机开发测试旁路；UI、日志和安装记录必须显示警告，不得导出为普通发布结果。 |

`Missing` 是回答，`unknown` 是没有回答，二者不得共享同一个“未找到”错误码或相同 fallback。

### 2.4 App Document body 的获取方式

系统可以通过多种方式取得 App Document，包括但不限于：

- App DID （App 名称或 BNS 名称）
  - 当用户输入 app1.owner 的时候，系统会自动尝试did:bns:app1.owner
  - 但用户输入 app1.example.com的时候，系统会自动尝试did:web:app1.example.com
- App Document Object ID；
- App Document URL；
- `.pikg` 内置的 `APPDOC.jwt` 或 `APPDOC.json`；
- 应用商店或订阅 Source；
- Inclusion Proof 中引用的内容对象；
- 好友分享、二维码、短码或 Inbox ActionObject；
- 本地开发目录、构建工具或 Agent 生成结果；
- 本地 Object Store 或历史安装记录。

除 App DID / BNS 名称外，其余入口取得的 App Document 都先视为候选 body。若权威解析只返回 `Active + doc_hash`，任意 Source 提供的匹配 body 均可补齐内容；若权威结果已是 `Revoked` / `Tombstoned`，任何 Source 都不能恢复该 body 的安装资格。

### 2.5 Installer 的两类标准输入

Installer 至少应支持两类逻辑入口。

#### 2.5.1 标识符入口

```text
install_app(identifier, referrer?, options?)
```

`identifier` 可以是 App DID、名称、Object ID、URL 或分享对象。系统必须归一化出 App DID 和候选 body / 包位置，再对 `(App DID, "app")` 执行标准解析。若 URL、Object ID 或分享对象本身不携带 App DID，可以先执行不产生安装副作用的最小 Acquisition，读取 Manifest / App Document 以提取 App DID；在可信 Resolve 完成前不得进入 Inspect 或 Deploy。

#### 2.5.2 本地包入口

```text
install_package(local_pikg_path, options?)
```

Installer 直接检查本地 `.pikg`。该入口的内容校验始终可以离线执行；只有当 Zone Resolver / 本机 DID cache 已持有可接受的解析结果，或用户显式启用带警告的本地开发覆盖时，才可以在不访问网络的情况下同时完成 DID 信任校验并进入 Deploy。

### 2.6 服务运行与暴露配置

服务配置分为三个不同所有者的数据面：

- `AppDoc.service_config_tips` 由 App 开发者发布，只声明运行端点、挂载点、RDB、运行参数以及期望的暴露方式；这些内容是安装 UI 的输入，不是 App 进程可读取的运行配置。
- `ServiceSettings` 记录用户选择。服务是否暴露以及采用何种暴露方式完全由用户决定；开发者声明的暴露信息只能作为建议值。
- `ServiceSpecConfig` 是 Installer / scheduler 根据 AppDoc 与用户选择构造的系统配置。scheduler 和 node-daemon 消费该结构，用户和 App 进程都不应直接修改或读取它。

`ServiceEndpointInfo` 描述 App 自身监听的一个运行端点：

```rust
pub struct ServiceEndpointInfo {
    pub protocol: ServiceProtocol, // http | https | tcp | udp
    pub inner_port: u16,
    pub required: bool,
    pub description: HashMap<String, String>,
    pub expose: Option<ServiceExposeTips>,
}
```

`required = true` 只表示该运行端点不能被关闭，不表示该端点必须暴露。用户可以启用服务但不配置任何 `expose`。运行与暴露必须保持独立：Zone 内多个 Node 可以同时运行 SMB 端点，但 ZoneGateway 对 Zone 外最多暴露一个 SMB 入口。

ZoneGateway 暴露路由使用强类型枚举：`Web` 通过子域名或 URI 路由，`Port` 通过独立端口路由。实例实际使用的端口不属于共享的 `ServiceSpecConfig` 暴露定义，仍由 scheduler 写入每个 `AppServiceInstanceConfig.service_ports_config`。

`ServiceConfigTips.rdb_instances` 与 `ServiceSpecConfig.rdb_instances` 都使用完整的 `RdbInstanceConfig`。开发者可声明 backend、schema 和连接需求；最终 connection string 由 scheduler 在构造运行配置时分配。该分配流程以及权限审批不属于本次数据定义。

---

## 3. 分阶段安装流水线

### 3.1 原有流程的问题

一个完整 App 可能包含多个 subpackage，并引用大量结构化对象与非结构化内容。传统流程通常需要：

1. 解析 App Document；
2. 展开 subpackage 和依赖；
3. 从不同位置获取 Package Meta；
4. 下载容器镜像、静态资源、模型、Prompt、配置或 Chunk；
5. 校验所有对象；
6. 等待内容全部 ready；
7. 准备运行环境；
8. 执行安装与启动。

当这些步骤被绑定在一个单体“安装”事务内时，任意网络源、DID 解析、Registry、作者 OOD 或分享者 OOD 暂时不可用，都可能表现为“安装失败”。这对开发循环、Agent 自构建、早期生态和弱网络场景尤其不利。

### 3.2 标准 Stage

安装实现应至少拆分为以下 Stage：

| Stage | 名称 | 主要输入 | 主要输出 |
|---|---|---|---|
| 1 | Resolve | 名称、DID、Object ID、URL 或 `.pikg` | `(App DID, "app")` 的 `ResolvedAppDocument`，含状态、证据和 warning |
| 2 | Inspect | App Document、目标设备和用户选项 | `InstallPlan`、权限摘要、缺失内容清单 |
| 3 | Acquire | 缺失对象清单、Source 列表、本地包 | `AcquiredContentSet` |
| 4 | Verify | `ResolvedAppDocument`、Package Meta、Chunk、包级签名 | `VerificationReport` |
| 5 | Prepare | 已验证内容、安装参数、目标 Node | `PreparedDeployment` |
| 6 | Deploy | PreparedDeployment | 已部署文件、容器、配置和服务注册 |
| 7 | Activate | 已部署应用 | 运行状态、健康检查和安装成功证明 |

Inspect Stage 必须先完成 §2.2.1 的 App Package Namespace 校验，再选择目标平台 package 或生成 `InstallPlan`。Verify Stage 必须使用实际取得的 Package Meta 重新核对相同 namespace；两个 Stage 任一处不一致都必须使计划失效。

### 3.3 Stage 隔离要求

每个 Stage 应当具备：

- 明确的输入和输出；
- 可持久化的中间状态；
- 可重复执行或幂等处理能力；
- 独立错误码和用户可理解的失败原因；
- 对已满足依赖的识别能力；
- 从上次成功 Stage 恢复的能力；
- 对本地缓存、`.pikg` 和网络 Source 的可替换解析策略；
- 取消操作和资源清理策略。

Resolve Stage 负责 DID 身份、发布状态和 owner 信任链；Verify Stage 负责 App Document Object ID、Package Meta、Chunk、包结构以及可选 Packager Signature。Verify 不得用“文件签名有效”重新解释或覆盖 Resolve 已得到的发布状态。

实际部署事务的开始点应位于内容获取和验证完成之后：

```text
Resolve → Inspect → Acquire → Verify
                         ↓ all ready
                   Prepare → Deploy → Activate
```

### 3.4 短路路径

#### 3.4.1 完整离线包

```text
Local pikg
    ↓ Resolve + Inspect
Content Ready + Trust Ready
    ↓ Verify
Prepare → Deploy → Activate
```

仅在本地已有可接受的 DID 解析证据时跳过网络 Acquire。包内内容完整但信任链不可用时，应报告 `TRUST_RESOLUTION_REQUIRED`，不能把“无需下载内容”展示成“可安全离线安装”。

#### 3.4.2 已安装内容或本地缓存复用

如果 App Document 引用的 Package Meta、Chunk 或等价内容已经存在且校验通过，Installer 必须直接复用，不得重复下载。

#### 3.4.3 本地开发与 Agent 自构建

```text
Build App
    ↓ Generate APPDOC and Package Meta
Pack Local pikg
    ↓ Zone cache / LocalAuthorityOverride（显式 scope + warning）
Local Inspect / Required Verification
    ↓
Deploy → Activate → Test
```

该路径可以不依赖公共 Source、应用商店、外部 Curator、作者 OOD 或链上证明，但不能把开发候选伪装成公开已发布结果。优先使用 Zone Resolver 的 cluster 级开发覆盖；单机测试才使用本机 `LocalAuthorityOverride`。覆盖必须带 scope、不可合并、不可导出，并在测试完成后由真正的权威发布替代。

#### 3.4.4 好友分享

分享方可以发送包含 App Document、当前目标 Package Meta 和实体内容的 `.pikg`。接收方只补齐本地和包内均不存在的内容，从而提高离线或半离线安装成功率。

---

## 4. Personal AI Package（pikg）文件格式

### 4.1 定位

`pikg` 是 Personal AI Package 的规范短名称，文件扩展名为：

```text
.pikg
```

它是 BuckyOS App 的标准交付与本地安装载体，其逻辑内容为：

```text
pikg
= App Document
+ bundled Package Meta entries
+ bundled structured objects
+ bundled unstructured content / chunks
+ optional transport and install auxiliary metadata
```

`pikg` 不替代 DID、Named Object 或 Chunk 体系，而是把一次交付所需的对象组织到同一个可携带文件中。包内内容仍然通过 Object ID、Chunk ID 或 Digest 独立验证。

名称 `pikg` 应作为一个完整单词读取，不拆分解释为 Python `pip` 包或其它计量单位组合。协议字段、Object ID 前缀、Schema、URL 参数和文件关联都必须使用同一名称。

### 4.2 逻辑目录结构

本版本先定义逻辑条目和内置 subpackage 的首选归档规则；`pikg` 外层容器自身的归档、压缩、随机访问和流式编码方式在后续版本中固定。

推荐逻辑结构：

```text
example.pikg
├── APPDOC.jwt                # 已签名 App Document，可选
├── APPDOC.json               # 未签名 App Document，可选
├── PACKAGE_META.json         # 包内 Package Meta 与内容索引
├── amd64_docker_image.tar.gz # 推荐：$sub_pkg_name.tar.gz
├── web.tar.gz                # 另一个名为 web 的 subpackage
├── objects/                  # 其他可选结构化对象（可选)
│   └── <object-id>.json
├── chunks/                   # 内容寻址 Chunk 或二进制 Blob (可选)
│   └── <chunk-id>
└── assets/                   # 可选的人类可读辅助资源
    └── ...
```

一个合法 `.pikg` 必须至少包含 `APPDOC.jwt` 或 `APPDOC.json` 之一。

内置 subpackage 的文件命名与 hash 绑定规则如下：

- `sub_pkg_name` 是 App Document 中引用该 subpackage 的逻辑名称或 key，例如 `amd64_docker_image`；
- 单文件归档的首选名称为 `$sub_pkg_name.tar.gz`，例如 `amd64_docker_image.tar.gz`；
- `sub_pkg_name` 必须是安全的单段文件名，只能包含字母、数字、点、下划线和连字符，不得包含 `/`、`\\`、`..` 或绝对路径；
- Package Meta 必须声明该归档的格式、字节长度和 hash；默认使用 `sha256:<hex>`；
- hash 的计算对象是 `.tar.gz` 文件本身的最终压缩字节，不是解压后的目录、tar 中间流或某个 Source 的传输封装；
- Installer 必须重新计算归档文件 hash，并同时核对 Package Meta 和 `PACKAGE_META.json.content_index`。任一处的 hash、size 或路径不一致都必须判定为 `INVALID_PACKAGE`；
- 若某类 subpackage 不能使用 `tar.gz`，Package Meta 必须显式声明实际 `format` 和 `path`。这属于例外，不改变 `$sub_pkg_name.tar.gz` 的首选规则。

### 4.3 APPDOC.jwt 与 APPDOC.json

- `APPDOC.jwt`：签名封装版本，签名与编码规则遵循 BuckyOS DID Document Resolve 约定；
- `APPDOC.json`：未签名或开发态 JSON 版本；
- 若两者同时存在，Installer 必须验证二者表达的规范化 App Document 一致；默认优先采用验证通过的签名版本；
- `.pikg` 中的 App Document 只是一份候选 body，不因位于包内、带有签名或由可信 Source 下载就自动成为已发布结果；
- `APPDOC.jwt` 的签名只证明 owner 授权构造了该 body。公开安装还必须通过 `(App DID, "app")` 的发布状态、`expected_owner`、`doc_hash` 和 owner policy 校验；
- 权威源返回 `Active + doc_hash` 时，匹配该 hash 的包内 body 可以补齐权威结果；权威源返回 `Revoked` / `Tombstoned` 时必须拒绝包内 body；
- 未签名 App Document 不等于格式非法，但只能在 `LOCAL_DEVELOPER` / `SYSTEM_INTERNAL` 等明确策略和本地认证上下文中安装，并必须标记为未发布或本地覆盖。

### 4.4 PACKAGE_META.json

`PACKAGE_META.json` 用于集中携带当前 `.pikg` 所包含的 Package Meta 对象和实体内容索引。

App Document 仍然使用 Object ID 引用 Package Meta；`PACKAGE_META.json` 只是在传输层将其中部分或全部对象内置到包中。

示例：

```jsonc
{
  "@schema": "buckyos.pikg.package-meta.v1",
  "app_doc_id": "obj:appdoc:sha256:...",

  // key 是 App Document 中引用的 Package Meta Object ID；
  // value 必须按该对象的规范化规则重新计算并验证 Object ID。
  "package_objects": {
    "obj:pkgmeta:linux-amd64:sha256:...": {
      "name": "amd64_docker_image",
      "selector": {
        "os": "linux",
        "arch": "x86_64"
      },
      "content": [
        {
          "kind": "archive",
          "format": "tar.gz",
          "path": "amd64_docker_image.tar.gz",
          "size": 104857600,
          "digest": "sha256:..."
        }
      ]
    }
  },

  // 描述实际存在于当前 pikg 中的实体内容。
  "content_index": {
    "sha256:...": {
      "sub_pkg_name": "amd64_docker_image",
      "path": "amd64_docker_image.tar.gz",
      "format": "tar.gz",
      "size": 104857600,
      "digest": "sha256:..."
    }
  }
}
```

要求：

- `package_objects` 可以只包含 App Document 所引用 Package Meta 的子集；
- key 必须与 value 的规范化内容哈希匹配；
- `content_index` 只能声明包内实际存在的内容；
- `content_index` 中的 `sub_pkg_name / path / format / size / digest` 必须与对应 Package Meta 的内容项一致；
- 使用首选命名时，`path` 必须等于 `$sub_pkg_name.tar.gz`；示例中的 `amd64_docker_image` 对应 `amd64_docker_image.tar.gz`；
- 路径必须是包内相对路径，不得包含目录穿越或外部绝对路径；
- 未被当前安装目标使用的包内内容可以不加载；
- Installer 不得仅因为对象位于 `.pikg` 内就跳过内容校验。

### 4.5 部分包与目标完备性

一个 App Document 可以声明多个平台和多个 subpackage，但一个 `.pikg` 不要求包含全部平台资源。

例如：

```text
App Document 声明：
- windows-x86_64
- linux-x86_64
- linux-aarch64

当前 pikg 携带：
- windows-x86_64
```

该包在 Windows x86_64 上可以是完整离线包，在 Linux ARM64 上则可能需要联网补齐，或者直接判定当前平台不可用。

因此协议区分：

- **Document Syntax Validity**：App Document 的 schema、编码、`did` 和 Object ID 是否一致；
- **DID Trust Readiness**：本地是否已有可接受的发布状态、owner 绑定/结构约束、owner document 和验证证据；
- **Package Integrity**：`.pikg` 中声明存在的对象是否完整且校验通过；
- **Content Readiness**：针对当前设备、安装选项和目标 Node，全部必需内容是否已存在；
- **Target Readiness**：目标 Node 的 OS、架构、Kernel、BuckyOS runtime/SDK 版本与数值能力是否满足 AppDoc 约束；
- **Config Readiness**：安装参数能否与 `ServiceConfigTips` 确定性合成为合法的最终 `ServiceSpecConfig`；
- **Install Readiness**：上述七个维度是否同时满足；
- **Full Ecosystem Completeness**：是否包含 App Document 声明的所有平台和可选组件。该属性不是普通安装的必要条件。

### 4.6 Offline Ready 判定

Installer 在不访问网络的情况下，基于以下信息生成当前目标的就绪结论：

- 当前操作系统与 CPU 架构；
- 目标 BuckyOS Node、Kernel、runtime/SDK 版本和能力快照；
- 用户选择的功能、安装参数和可选组件；
- 本地已安装内容；
- Zone Resolver / 本机 DID cache 中 `(App DID, "app")` 与 owner document 的可用证据；
- 本地 Object Store 和 Chunk Cache；
- `.pikg` 中的 Package Meta 和实体内容。

至少应输出以下状态之一：

```text
OFFLINE_READY             内容与 DID 信任均已离线就绪，可直接进入安装
CONTENT_DOWNLOAD_REQUIRED 当前目标缺少内容，需要联网获取
TRUST_RESOLUTION_REQUIRED 内容齐全，但缺少可接受的 DID/owner 解析证据
IDENTITY_REVOKED           App Document 已 Revoked/Tombstoned，禁止 fallback
UNSUPPORTED_TARGET        当前目标没有匹配的 package
INVALID_PACKAGE           包结构、对象或内容校验失败
CONFIG_BLOCKED            安装参数、权限或运行条件尚未满足
```

“完整离线包”是相对于当前安装目标的动态结论，而不是 `.pikg` 的永久全局属性。`Content Readiness = true` 也不等于 `DID Trust Readiness = true`。

### 4.7 统一 Object Provider

通过 `.pikg` 安装时，Installer 仍然按 Object ID 请求对象，不建立第二套对象身份体系。

统一 Object Provider 的可用来源包括：

1. 本地已安装内容；
2. 本地 Object Store / Chunk Cache；
3. 当前 `.pikg` 的 `PACKAGE_META.json`、根目录 `$sub_pkg_name.tar.gz`、`objects/` 和 `chunks/`；
4. 标准 Named Object / Content Network；
5. 配置的 Source、Registry、作者 OOD、分享者 OOD 或其他远程源。

逻辑要求是：

```text
resolve_object(object_id, policy, pikg_context?) -> verified object or missing
```

实现可以将 `.pikg` 注册为标准 Object Provider。为兼容现有实现，也可以在标准系统查询未命中后再尝试包内对象。处于明确离线模式时，Object Resolver MUST NOT 访问网络。

这里的 Object Provider 只解决 `object_id → bytes`，与 `resolve_did(did, doc_type)` 的 DID resolver-provider 是两套不同机制。`.pikg`、App、Source 和 Curator 不得借此注册或替换 DID method 的权威渠道。

### 4.8 Load 与 Import 分离

从 `.pikg` 加载对象不等于必须导入本地公共 Object Store。

Installer 应区分：

```text
load_from_pikg(object_id)     # 仅供当前安装事务使用
import_to_object_store(object) # 写入本地可复用存储
```

导入策略可以由内容大小、隐私属性、缓存策略、用户设置和磁盘空间决定，但不得成为安装成功的强制前置条件。

### 4.9 包级签名与角色

App 作者、App Document Owner、Builder、Packager、Source 和 Referrer 可能是不同主体：

- App Document 签名证明 `expected_owner` 的有效 key 授权构造了该 Document；它不单独证明该 Document 已公开发布；
- DID method 的权威发布状态证明哪个 App Document 当前生效；签字权与发布权必须分别验证；
- Package Meta / Chunk 的 Object ID 或签名证明具体内容身份；
- 可选的整包签名证明谁组装了 `.pikg`，以及包文件在传输后是否被修改；
- Source 的传输签名或 HTTPS 身份只证明下载来源，不替代内容寻址校验。

后续版本可以增加独立的包级 Manifest 和 Packager Signature，但不能把包级签名错误地等同为原始作者签名。

---

## 5. Package Acquisition 与 Package Installation

### 5.1 两段式流程

新的推荐流程将应用交付收束为两个边界清晰的阶段：

```text
阶段 A：Package Acquisition
identifier / URL / share / store / local build
                    ↓
             local .pikg

阶段 B：Package Installation
local .pikg
    ↓ inspect / configure / acquire missing / verify
    ↓ prepare / deploy / activate
installed app
```

### 5.2 阶段 A：Package Acquisition

目标是把一个可打开的 `.pikg` 放到 Installer 可以访问的本地路径或本地受控 Staging Area。

输入可以是：

- App DID、名称或 Object ID；
- App Document URL；
- `.pikg` URL 或 `.pikg` Object ID；
- 应用商店条目；
- 好友分享、P2P 对象、二维码或短码；
- 本地文件；
- 开发构建输出。

该阶段负责：

- 归一化 App 名称 / DID，并显式以 `doc_type = "app"` 执行 DID 解析；
- 对不直接携带 App DID 的包或 URL，先只获取足以提取 App DID 和候选 body 的最小内容，再进入可信解析；
- 保留 `document_status`、`evidence`、`verification_status`、`cache_status`、`expected_owner` 与 warning，区分 `Missing`、负状态和 `unknown`；
- Source 选择和回退；
- 文件下载、P2P 获取、断点续传和重试；
- 传输层完整性检查；
- 基础包结构检查；
- 将最终文件原子化放入本地 Staging Area；
- 输出稳定的本地包句柄或路径。

下载失败应报告为 Acquisition 失败，而不应混淆为 Deploy 或 Activate 失败。

### 5.3 阶段 B：Package Installation

该阶段从本地 `.pikg` 开始，负责：

1. 读取 App Document；
2. 将包内 body 与 Resolve Stage 的 App DID、发布状态、`doc_hash`、`expected_owner` 和 owner policy 对齐，并验证内容身份；
3. 从可信 App DID 派生 App Package Namespace，校验 `AppDoc.name` 和全部 `pkg_list.*.pkg_id` 的归属；
4. 根据目标设备生成 InstallPlan；
5. 展示应用、权限、来源和离线就绪状态；
6. 收集安装参数；
7. 获取并验证当前目标仍缺失的内容，并用实际 Package Meta 再次校验 namespace；
8. 准备目录、配置、容器、网络和服务；
9. 部署；
10. 启动和健康检查；
11. 记录安装状态并生成安装成功证明。

只有当必需内容已全部就绪，且 DID 发布/owner 证据与所有内容验证均通过后，系统才应进入实际 Deploy 阶段。

### 5.4 先下载再安装

旧流程常表现为：

```text
点击安装 → 进入安装 → 逐步下载 → 下载失败 → 安装失败
```

新流程应表现为：

```text
取得本地 pikg → 检查并补齐内容/信任证据 → Content + Trust Ready → 执行安装
```

即使 UI 仍提供一个“一键安装”按钮，内部状态也必须清楚区分：

```text
ACQUIRING / VERIFYING ≠ INSTALLING / DEPLOYING
```

### 5.5 安装事务状态示例

```text
NEW
  ↓
RESOLVED
  ↓
TRUST_READY
  ↓
INSPECTED
  ↓
WAITING_FOR_CONFIG
  ↓
ACQUIRING_CONTENT          # 可选
  ↓
VERIFIED
  ↓
PREPARED
  ↓
DEPLOYING
  ↓
ACTIVATING
  ↓
INSTALLED
```

若 Resolve 得到 `unknown` 且本地没有策略允许的已验证缓存，事务应进入 `WAITING_FOR_TRUST_RESOLUTION`；若得到 `Revoked` / `Tombstoned`，事务应进入不可重试的 `IDENTITY_REVOKED`，除非 DID method 权威源之后发布了新的状态。

任何失败状态应保存：

- 失败 Stage；
- 错误码；
- 可重试性；
- 已完成的内容；
- 用户可执行的修复动作。

---

## 6. 安装交互流程

### 6.1 打开本地 pikg

操作系统或 BuckyOS Desktop 应将 `.pikg` 文件关联到 Installer。

打开后，Installer 首先完成包结构检查和目标就绪分析，不应立即写入系统目录或启动容器。

### 6.2 标准三步 UI

#### 第一步：应用信息与就绪状态

展示：

- App 名称、图标、版本和描述；
- App DID、`doc_type = "app"`、App Document Object ID 和发布版本；
- Author、Owner、Builder、Packager、Source 和 Referrer；
- DID 发布状态、body 证据等级、签名验证结果、cache 状态、resolver warning、Curator 背书和信任提示；
- 权限摘要；
- 当前目标平台支持情况；
- 是否完全离线就绪；
- 缺失对象、预计下载量和可用 Source；
- 可能的费用与授权要求。

典型提示：

```text
该包已包含当前设备所需的全部内容，本地 DID cache 也包含可接受的发布与 owner 证据，无需网络即可安装。
```

或：

```text
该包已包含全部应用内容，但缺少可接受的 App DID/owner 解析证据。联网完成信任解析后才可安装。
```

#### 第二步：安装参数与权限确认

App Document 应提供足够的参数定义，使用户在大体积下载或部署前确定：

- 目标 Node；
- 安装组件；
- 数据目录和持久化策略；
- 服务端口、域名和公开访问方式；
- 开机启动；
- 模型或 Runtime 选择；
- 文件、网络、系统和设备权限；
- 可选功能与资源配额。

参数确认后，Installer 生成稳定的 `InstallPlan`。如果参数变化会改变 package selector，必须重新计算 Content Readiness 和 Install Readiness。

#### 第三步：内容获取与安装

- `OFFLINE_READY`：内容和 DID 信任均已就绪，跳过网络 Acquire，直接验证并进入 Prepare；
- `CONTENT_DOWNLOAD_REQUIRED`：先下载和校验缺失内容；全部成功后再进入 Prepare；
- `TRUST_RESOLUTION_REQUIRED`：内容已齐全，但必须先取得可接受的 DID 发布状态和 owner 证据；
- `IDENTITY_REVOKED`：禁止安装，不得提供“仍使用包内版本”或旧 cache 的绕过入口；
- 安装完成后执行健康检查，并明确区分“已部署但启动失败”和“安装成功”。

### 6.3 点击安装（Web to Native）

第三方网页可以放置“安装 App”按钮。

推荐入口：(jump的流程会引导到current_zone的对应url)

```text
https://jump.buckyos.ai/sysdlg/app_installer?
    identifier=$ENCODED_APP_IDENTIFIER
    &ref=$REFERRER_ID

jump后实际打开
https://bob.web3.buckyos.ai/sysdlg/app_installer?
    identifier=$ENCODED_APP_IDENTIFIER
    &ref=$REFERRER_ID
```

`identifier` 可以是 App DID、App Document Object ID、App Document URL 或 `.pikg` URL。

流程： (参考 buckyos 通用jump协议设计)

### 6.4 开发者与 Agent 自构建流程

推荐开发工具链：

```text
source / prompt / config
        ↓ build
subpackage content
        ↓ generate
Package Meta objects
        ↓ generate
APPDOC.json or APPDOC.jwt
        ↓ pack
local app.pikg
        ↓ install_package
run / test / iterate
```

构建产物一旦位于本地，Installer 可以在显式开发上下文中直接进入 Inspect，不依赖公共网络和公开发布流程。此时必须使用 Zone Resolver 开发覆盖或本机 `LocalAuthorityOverride`，或把结果明确标记为未发布候选；不能伪装成公开 `Active`。

开发安装模式可以：

- 接受未签名 `APPDOC.json`；
- 跳过 App Store 收录、Curator 证明和外部信用查询；
- 不要求先发布到公共 DID 或 Source；
- 复用本地构建目录和 Chunk Cache；
- 支持重复安装、覆盖部署或快速更新。

Zone 级开发应优先使用 Zone Resolver cache 注入 `(App DID, "app")` 的完整解析结果；单机、CI 或无 Zone Resolver 环境才使用本机覆盖。本机覆盖必须带 `machine / test-env / CI` scope、`LocalAuthorityOverride` warning，且不得合并进普通 cache 或向外同步。

但以下检查不得被跳过：

- JSON、对象和路径结构合法性；
- App Document 的 `did == App DID`，以及开发上下文声明的 owner 约束；
- App DID、`AppDoc.name`、App Package Namespace 与全部 `pkg_list.*.pkg_id` 的归属约束；
- Package Meta 与 Object ID 一致性；
- Chunk / 文件 Digest；
- 目标平台与 Runtime 兼容性；
- 权限声明与危险系统操作提示；
- 目录穿越、符号链接逃逸和非授权宿主路径访问；
- 安装事务的失败回滚边界。

“开发者就是当前用户”必须由本地认证会话、可信构建上下文、有效签名或用户明确开启的开发模式确认，不能只相信未签名 JSON 中自称的 `owner` 字段。

---

## 7. 应用分享

### 7.1 HTTPS 链接分享

兼容格式：

```text
https://$USER_ZONE_HOST/sysdlg/share
    ?type=app_doc|pikg
    &id=$OBJECT_ID
    &ref=$REFERRER_ID
```


分享页负责调用 Gateway / URL Scheme，并尽量先完成 `.pikg` Acquisition。

### 7.2 文件分享

好友可以直接通过局域网、移动存储、聊天工具或 P2P 发送 `.pikg` 文件。只要当前目标需要的内容已经包含在包内或本地缓存中，接收方不依赖分享者 OOD 在线即可安装。

这是提高“离线成功率”的首选方式。

### 7.3 二维码分享（文本分享)

> 这个和AppInstaller的new app text 的输入一致，应能自动识别

二维码可以编码：

- HTTPS 分享链接；
- App DID 或 App Document Object ID；
- `.pikg` Object ID / 下载描述；


二维码本身通常不携带完整 `.pikg` 实体内容，除非应用非常小并采用专门的多码传输方案。


### 7.5 s应用商店（规划)

- 内容聚合：用户自管理 App Document + 订阅 Source List + Curator Inclusion Proof；
- 去重：按 App DID 聚合同一逻辑应用，按 App Document Object ID 区分具体版本；
- 可信解析：商店条目和 Inclusion Proof 只提供候选 App DID、body 和获取位置；安装前仍必须对 `(App DID, "app")` 执行标准解析；
- 变体：同一 App Document 可以对应多个平台或内容覆盖范围不同的 `.pikg`；
- 获取：应用商店应优先完成 `.pikg` Acquisition，再交给 Installer；
- 历史：凡触发过获取或安装的 App，都可以记录到用户自管理列表，标明取得、检查、下载、安装和启动的分别状态；
- 重试：下载失败只重试 Acquisition；部署失败只重试对应安装 Stage。

---

## 8. 分发、多源下载与完整性

### 8.1 分发对象层次

BuckyOS App 分发至少包含三层：

```text
App DID / Name
    ↓ resolve_did(doc_type = "app")
Trusted App Document + resolution metadata
    ↓ references
Package Meta Objects
    ↓ references
Chunks / Images / Static Assets / Models / Prompts
```

`.pikg` 是以上对象的一种本地聚合交付形式，不改变各对象的逻辑身份。

### 8.2 多源回退

远程获取可以使用以下 Source：

1. 本地 Object Store 和已安装内容；
2. 当前 `.pikg`；
3. 公共容器或制品源；
4. App Document 中声明的可验证 URL，例如 GitHub Releases；
5. Curator / App Source 服务器；
6. 分享者 OOD 或 P2P Source；
7. App 作者或 Owner OOD；
8. 用户配置的镜像源。

具体优先级可以由网络策略、隐私、价格、速度和 Source 信誉决定。所有来源最终必须通过相同内容身份校验。

本节的“多源”只指 App Document body、Package Meta、Chunk 和 `.pikg` 的内容传输。DID 主动查询不按下载 Source 竞价或并发选优：它由内核为目标 DID method 选择至多一个权威发布渠道和显式有序的少数补充源，严格 first-win。真正的多来源合并发生在 DID cache，且必须先比较证据等级，再在同级内只按文档 `iat` 排序，并用 content hash 判断同一性与同 revision 冲突。

### 8.3 下载可靠性

Package Acquisition 应支持：

- 断点续传；
- 临时文件与完成文件原子切换；
- 分块并行下载；
- Source 失败回退；
- 已下载 Chunk 复用；
- 可选的带宽和费用上限；
- 下载完成前不向 Installer 暴露“可安装”状态；
- 对缺失内容给出精确清单，而非笼统“网络错误”。

### 8.4 完整性校验

- App Document、Package Meta 和其他结构化对象必须按规范化编码计算 Object ID；
- App Document 的 Object ID 校验只能证明“内容是什么”，不能证明“该内容已由 App DID 当前发布”；DID 发布状态与 owner 验证必须独立通过；
- Chunk 和二进制内容必须包含 Digest，例如 `sha256:...`；
- 无论从哪个 Source 下载，校验通过前不得进入 Deploy；
- 如果对象 ID 与内容不一致，必须视为内容损坏或恶意替换；
- `.pikg` 中的每一个索引条目必须与实际包内内容一致；
- 对 `$sub_pkg_name.tar.gz`，Digest 必须针对最终 `.tar.gz` 文件的压缩后字节计算，并与对应 Package Meta 及 `content_index` 同时匹配；其它压缩格式也必须在 Package Meta 中明确 hash 的计算对象，且不得混用；
- 对流式安装，应在内容单元校验完成后才允许消费该单元。

---

## 9. 信任与安全机制

### 9.1 身份角色分离

系统应区分：

- **Author**：原始软件作者；
- **Owner**：控制 App DID 或 App Document 的主体；
- **Builder**：构建某个 subpackage 的主体；
- **Packager**：生成 `.pikg` 的主体；
- **Curator**：收录和评价主体；
- **Source**：提供下载的主体；
- **Referrer**：推荐或分享主体。

UI 不应把“由好友分享”“由可信 Source 下载”错误展示为“由原作者签名”。

### 9.2 作者与 Owner 信任

- `NeedProof` App Document 的签名证明签字权，DID method 的权威状态证明发布权，二者不能互相替代；从权威信道直接取得的 `Anchored` body 不重复要求外部签名，但仍必须通过 `did` 与权威 `doc_hash` 等一致性检查；
- `expected_owner` 只能来自权威源 owner 绑定或 DID 名字结构的确定性默认值。候选 App Document 自声明的 `owner` 只能用于一致性检查，不得作为寻找验签 key 的起点；
- 所有 body 的 `document.did` 必须等于输入 App DID；`NeedProof` body 的 `document.owner` 还必须等于 `expected_owner`。任一不一致都应拒绝并记录高风险 warning；
- 需要证明的 App Document 必须递归解析 `resolve_did(expected_owner, "owner")`，按文档 `iat` 时刻有效的 owner key 验签，并应用 `valid_iat` 等当前 owner policy；
- `need_proof` 由取回信道和 `doc_type` 契约决定，不能因 body 缺少签名字段而降级为“无需验证”。`doc_type = "app"` 是需要验证的 Document，不得走 Info 免验证路径；
- Owner 变更或委托只有经 DID method 权威源发布才生效；App Document 自己修改 `owner` 字段不能改变所有权；
- 联系人关系、第三方信用 Oracle 和未签名状态只影响附加信任提示或安装策略，不能放宽 `did / expected_owner / doc_hash / terminal status` 等硬约束。

### 9.3 发布状态、证据与 Cache

Installer 必须遵守 resolver 返回的证据与 cache 语义：

1. `Revoked` / `Tombstoned` 是权威回答和终止状态。必须删除或屏蔽 positive cache，禁止回退到旧 App Document、过期 cache、自签名候选或好友分享包；只有权威源的新状态可以翻篇。
2. `Missing` 表示权威源确认从未发布该 `doc_type`；`unknown` 表示权威源没有回答。网络错误不得伪装成 `Missing`，两者的 UI、错误码和重试策略必须不同。
3. DID cache 的证据等级为 `Published > Verified > Unverified`；解析结果的 body 信道证据则是 `Anchored / NeedProof`，二者不得混成一个字段。Cache 合并必须先比较证据等级，同级只比较文档 revision `iat`，同一 `iat` 的不同 content hash 必须作为冲突拒绝；`version_seq` 不参与比较。更新的自签名候选不能覆盖旧一些的已发布结果。
4. Zone Resolver 是 Zone 内权威 L1 cache；其明确回答可以直接使用，`unknown` 才进入本机 L2 cache 和 provider 链。本机覆盖短路所有正常查询，必须带 `LocalAuthorityOverride` warning 和 scope。
5. 权威源不可达时，正常离线路径是使用未作废且策略允许的已验证 cache。只有本地没有发布/负状态记忆且策略明确允许时，未验证观察候选才可以 `ObservedFallback` 状态露面；`STRICT_PUBLIC` 和 `NORMAL` 默认不得据此 Deploy。

Installer 应把 `document_status`、`evidence`、`verification_status`、`cache_status`、`expected_owner`、发布版本和 warnings 固化到安装记录，便于升级、审计和风险提示。

### 9.4 Curator 信任

- 高信誉 Source 或 Curator 的 Inclusion Proof 可以为 App 提供背书；
- Inclusion Proof 可以包含 `rank`、collection、review URL 和有效期；
- Curator 背书不能替代 App Document 的 DID 发布/owner 验证与实体内容完整性校验。

### 9.5 Referrer 信任

- Referrer 表示“谁推荐给我”，不等于“谁收录”“谁构建”或“谁发布”；
- 好友推荐可以提高 UI 展示优先级，但不能自动授予高危权限；
- 推荐链必须防止循环归因和伪造。

### 9.6 安装策略等级

建议至少支持：

```text
STRICT_PUBLIC      公开分发严格验证
NORMAL             默认用户模式
TRUSTED_SHARE      已信任联系人分享
LOCAL_DEVELOPER    本地开发模式
SYSTEM_INTERNAL    系统内置应用模式
```

每个等级可以调整自签名候选、`ObservedFallback` 结果、Curator、Source 和网络查询要求，但不能绕过以下硬规则：`did / expected_owner` 一致性、App Package Namespace 归属、权威 `doc_hash`、`Revoked / Tombstoned` 终止状态、内容 Object ID / Digest 和基础沙箱安全。公开模式必须拒绝未验证结果；开发模式的放宽必须通过带 scope 和 warning 的本地覆盖表达。

### 9.7 用户干预

用户可在系统面板调整对以下实体的信任策略：

- Author / Owner；
- Builder / Packager；
- Curator / Source；
- Referrer；
- 特定 App DID、App Document Object ID 或包 Digest。

支持白名单、黑名单、仅提示和每次询问等策略。用户白名单不能把 `Revoked / Tombstoned`、owner 冒充或 `doc_hash` 不匹配降级为普通提示。

### 9.8 安装安全边界

Installer 必须防范：

- 包内目录穿越；
- 符号链接逃逸；
- 未声明的宿主文件访问；
- 端口和域名冲突；
- 权限升级；
- 恶意安装脚本；
- Chunk ID 碰撞或规范化差异；
- TOCTOU（校验后文件被替换）；
- 部分部署残留；
- App Document 与实际启动内容不一致；
- 合法 App DID 的持有者通过越权 Package ID 占用其它 App、系统包、gateway server 或宿主友好目录的 namespace；
- 仅做无边界字符串前缀匹配，使 `user1_app10` 被错误识别为 `user1_app1` 的 subpackage；
- 使用候选文档自声明的 owner 完成“自选 owner、自签名、自验证”；
- 权威源不可达时用更新的自签名候选覆盖已发布或负状态记忆；
- 将 Object Provider、应用商店、`.pikg` 或 App 动态注册成 DID resolver-provider。

---

## 10. 经济模型


> 该章节是 《BNS + CYFS 通用内容发行协议》的一部分，这里只简要说明 


### 10.1 安装成功证明

生态价值事件应定义为应用完成 Activate 并通过规定的健康检查，而不是仅点击安装或下载完成。

示例：

```jsonc
{
  "action": "installed",
  "app_did": "did:bns:filebrowser.buckyos",
  "doc_type": "app",
  "app_doc_id": "obj:appdoc:sha256:...",
  "did_resolution": {
    "document_status": "Active",
    "document_version": 1769990000,
    "evidence": "Anchored",
    "verification_status": "Passed",
    "cache_status": "ZoneHit"
  },
  "package_meta_ids": [
    "obj:pkgmeta:linux-amd64:sha256:..."
  ],
  "pikg_digest": "sha256:...",
  "userid": "did:bucky:user_id",
  "device_id": "did:dev:device_id",
  "iat": 1769990599,
  "exp": 1801094599,
  "details": {
    "referrer": "did:bucky:referrer_id",
    "curator": "did:web:gitpot.ai",
    "source": "did:web:source.example"
  }
}
```

安装证明应避免泄露不必要的设备和用户隐私，并允许使用场景化或匿名化身份。`did_resolution` 用于证明安装器基于哪一类解析证据完成安装，但不得把本地覆盖或 `ObservedFallback` 伪装为公开 `Active / Anchored`。

### 10.2 购买对象与购买证明

购买对象通常是 App 的特定版本、版本范围或授权系列。

```jsonc
{
  "action": "purchased",
  "app_did": "did:bns:filebrowser.buckyos",
  "version_range": "^2.0.0",
  "buyer": "did:bucky:user_id",
  "tx_hash": "0x..."
}
```

### 10.3 支付模式

- **传统付费**：通过 Source 网关使用法币或信用卡支付，由 Source 与作者结算；
- **USDB 付费**：调用标准支付合约，根据 `revenue_split` 向 Author、Source 和 Referrer 自动分账；
- **HTTP 402**：允许作者或 Source 提供自定义付费网关；
- **离线授权**：可以把可验证 Receipt 或授权 Token 放入 Acquisition Context 或 `.pikg` 的独立授权区，但授权对象不应改变内容 Object ID。

### 10.4 支付与下载原子性

为避免“支付成功但内容无法取得”，推荐：

1. 先生成明确的内容和价格计划；
2. 通过托管、可退款授权或分阶段付款锁定资金；
3. Package Acquisition 成功并完成验证后释放付款；
4. 若主 Source 失败，允许从其他能提供相同内容 ID 的 Source 补救；
5. 只有内容已取得但本地部署失败时，按授权条款决定是否退款，而不是把下载和本地配置错误混为一类。

### 10.5 BDT 激励与风险

- 安装成功证明可以提交给 BDT DAO 合约参与激励；
- 奖励可采用时间衰减和长尾基础奖励；
- 需要抗女巫攻击，例如活跃 OOD、设备信誉、Staking、成本证明或隐私保护的唯一性机制；
- 兼容移植应用应支持真实作者认领和权益转移；
- 无主收益可以进入 DAO 公共池。

---

## 11. 核心数据结构

> 本章字段以当前实现为准（`buckyos-api` / `control_panel` / `ndn-lib` / `package-lib`）。Object ID、JCS 与 schema 冻结规则见 §14.0 D1/D2；安装事务与长期记录见 §14.0 D3。

### 11.1 InclusionProof

收录证明定义在 `ndn-lib`（obj type `cyinc`），由 RepoService 作为 `RepoProof::Collection` 持久化；Installer 安装流水线不依赖本结构，但应用商店/Curator 收录仍使用它。

```rust
pub const OBJ_TYPE_INCLUSION_PROOF: &str = "cyinc";

#[derive(Serialize, Deserialize, Clone)]
pub struct InclusionProof {
    /// 被收录内容的 ObjId，必须与 content_obj 一致
    pub content_id: String,
    pub content_obj: serde_json::Value,
    pub curator: DID,
    pub editor: Vec<String>,
    pub meta: Option<serde_json::Value>,
    /// 1-100
    pub rank: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub collection: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_url: Option<String>,
    pub iat: u64,
    pub exp: u64,
}
```

### 11.2 App Document（AppDoc）

实现类型为 `buckyos_api::AppDoc`：在 `PackageMeta`（flatten）之上叠加 App 专用字段。不另造第二套 schema；`doc_type` 必填且固定为 `"app"`，`did` 必填。Object ID 规则：`ObjId = appdoc:hex(sha256(JCS(body)))`。

Rust 形状（省略 serde 细节）：

```rust
pub struct AppDoc {
    pub doc_type: AppDocType,              // 固定 "app"
    #[serde(flatten)]
    pub _base: PackageMeta,                // 含 did/name/version/owner/author/...
    pub pkg_list: SubPkgList,              // key → SubPkgDesc
    pub show_name: String,
    pub presentation: Option<AppPresentation>,
    pub sdk_version: Option<String>,
    pub req_capbilities: HashMap<String, i64>,
    pub permissions: Vec<PermissionItem>,
    pub selector_type: SelectorType,       // single | static | random | by_event | custom
    pub service_config_tips: ServiceConfigTips,
}
```

`PackageMeta` flatten 后常见 JSON 字段（来自 `BaseContentObject` / `FileObject` / `PackageMeta`）：

| 字段 | 说明 |
|---|---|
| `did` | App DID（必填） |
| `name` | 逻辑应用名（亦作 `app_id`） |
| `version` | 应用语义版本；不是 DID `document_version` |
| `version_tag` | 可选版本标签 |
| `author` / `owner` | 作者与 owner DID |
| `create_time` / `last_update_time` / `exp` | 时间戳 |
| `deps` | `pkg_name → version_req` |
| `tags` / `categories` | `categories[0]` 表达 `AppType`（见 §11.6） |
| `meta` | 自由扩展；描述文案常落在 `meta.description.detail.<lang>` |
| `content` / `size` | FileObject 内容引用（App 级通常为空/`0`） |

App 专用字段：

| 字段 | 说明 |
|---|---|
| `doc_type` | 固定 `"app"` |
| `pkg_list` | subpackage 映射，见 §11.3 |
| `show_name` | 展示名 |
| `presentation` | 可选：`title` / `summary` / `description` / `icons` / `links` / `license` |
| `permissions` | `PermissionItem[]`，见 §11.5 |
| `selector_type` | 实例选择策略 |
| `service_config_tips` | 安装 UI 提示（端点、挂载、RDB、instance volume 等），不是最终运行配置 |
| `sdk_version` / `req_capbilities` | SDK 与能力需求 |

`sdk_version` 使用去掉可选 `v` 前缀后的 SemVer 下限语义（目标 runtime 必须 `>=` 该版本）；任一侧不是合法 SemVer 时只允许规范化前的字符串完全相等。`req_capbilities` 的每个值都是最小数值要求，目标缺少该 key 也视为不支持，而不是按 `0` 或“未知但允许”处理。

示例（对齐当前样例包与实现字段名；非完整必填清单）：

```jsonc
{
  "did": "did:bns:pikg-docker.root",
  "doc_type": "app",
  "name": "pikg-docker",
  "show_name": "PIKG Docker Fixture",
  "version": "0.1.0",
  "author": "did:bns:root",
  "owner": "did:bns:root",
  "categories": ["dapp"],
  "create_time": 1800000000,
  "last_update_time": 1785800758,
  "deps": {
    "nightly-linux-aarch64.root_pikg-docker-image": "0.1.0"
  },
  "meta": {
    "description": {
      "detail": {
        "en": "Local docker fixture for app_installer lifecycle testing."
      }
    }
  },
  "pkg_list": {
    "aarch64_docker_image": {
      "pkg_id": "root_pikg-docker-image#0.1.0",
      "pkg_objid": "pkg:67e624552871d410c63a2611b5500d32a72ff866caddc039249700ca6642ba8c",
      "docker_image_name": "local/pikg-docker:0.1.0-arm64",
      "docker_image_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111"
      // 可选: "source_url",
      //       "selector": { "os": "linux", "arch": "aarch64" },
      //       "required": true
    }
  },
  "selector_type": "single",
  "service_config_tips": {
    "service_endpoints": {
      "www": {
        "protocol": "http",
        "inner_port": 80,
        "required": true,
        "expose": {
          "route": { "type": "web" },
          "scope": "",
          "allow_guest": false
        }
      }
    },
    "data_mount_points": {},
    "local_cache_mount_points": {},
    "external_mount_points": {},
    "rdb_instances": {},
    "instance_volume": { "mode": "required" }
  },
  "permissions": [
    {
      "scope_path": "user/home",
      "required": true,
      "actions": ["read", "write"],
      "exp": null
    }
  ]
}
```

要点：

- 旧草案中的 `packages[]` / `install{}` / `economics` / `curators` / `@schema: buckyos.app.document.v1` **不是**当前 `AppDoc` 字段；平台选择在 `pkg_list.*.selector`（或按 key 派生），安装/暴露提示在 `service_config_tips`，收录背书走 `InclusionProof`。
- `owner` 与 `did:bns:$app_name.$owner_id` 的结构默认值应一致；验证时以权威源 owner 绑定优先，候选自声明不得充当 `expected_owner`。
- `did`、`owner`、签名与发布 `doc_hash` 属于 DID 验证约束，不得只做 App schema 格式检查。
- 旧名称 `APP_META_JSON` 仅作兼容入口别名；新协议与 API 统一使用 `App Document / APPDOC`。

### 11.3 Package Meta 与 SubPkgDesc

存在两层“Package Meta”，不得混用：

1. **Named Object `PackageMeta`**（`package-lib`，obj type `pkg`）：描述某个 subpackage 内容对象本身。
2. **`.pikg` 内 `PACKAGE_META.json`**（`PikgPackageMetaFile`，`@schema = buckyos.pikg.package-meta.v1`）：传输层索引，把若干 `PackageMeta` 与包内 payload 绑在一起；结构见 §4.4。

#### 11.3.1 SubPkgDesc（AppDoc.pkg_list 的 value）

平台选择条件挂在 App Document 的 `pkg_list` entry 上，不挂在 Package Meta Named Object 上：

```rust
pub struct PackageSelector {
    pub os: Option<String>,
    pub arch: Option<String>,
    pub min_kernel_version: Option<String>,
}

pub struct SubPkgDesc {
    pub pkg_id: String,
    pub pkg_objid: Option<ObjId>,          // Package Meta Object ID（pkg:...）
    pub docker_image_name: Option<String>,
    pub docker_image_digest: Option<String>,
    pub source_url: Option<String>,        // 内容获取位置，与 pkg_objid 独立
    pub selector: Option<PackageSelector>, // 省略时按已知 key 派生
    pub required: Option<bool>,            // 省略视为 true
}
```

已知 `pkg_list` key（`amd64_docker_image`、`web`、`agent` 等）未显式声明 `selector` 时按固定命名表派生；未知 key 无显式 selector 时不参与自动选择。

本版本的 Docker 安装计划不接受只给 registry tag 的松散引用。任何被选中的 Docker package 必须同时提供 `pkg_objid` 与 `docker_image_digest = sha256:<64 hex>`：前者把镜像归档纳入标准 Package Meta/内容获取链，后者绑定最终加载或拉取后的 runtime image identity。两者都会写入 `InstallPlan` 并参与 fingerprint；缺失或格式非法直接返回 `INVALID_PACKAGE`，不得生成看似 `OFFLINE_READY`、实际到 Deploy 才隐式 pull 的计划。

`pkg_id` 不是一个仅由内容 hash 保护的自由字符串。对于外部 App，它同时声明了后续可能使用的 PackageEnv 名、宿主友好目录名、Static Web gateway server 名和运行入口名，因此必须通过 §2.2.1 的 App Package Namespace 校验。Installer 必须校验 `pkg_list` 的全部 entry，而不只是当前平台最终选中的 entry，避免攻击者把越权 Package ID 隐藏在另一平台或可选 package 中，待升级、迁移或重新调度时触发。

以 `did:bns:app1.user1` 为例，推荐的各角色 package 为：

```text
user1_app1-web#1.0.0
user1_app1-agent#1.0.0
user1_app1-amd64-docker-image#1.0.0
```

点号前的合法 PackageEnv qualifier 不属于 `unique_name`，但必须由 Installer 根据目标环境识别；开发者不能通过任意 qualifier 改变或绕过 `user1_app1` 所有权前缀。

#### 11.3.2 PackageMeta Named Object

```jsonc
{
  "name": "nightly-linux-aarch64.root_pikg-docker-image",
  "version": "0.1.0",
  "author": "did:bns:root",
  "owner": "did:bns:root",
  "create_time": 1785800758,
  "last_update_time": 1785800758,
  "size": 1896103,
  // content 为 chunk / mix256 等内容寻址引用，不是路径列表
  "content": "mix256:a7dd73a1d37ee65cbbdf3a27dfaee22d62ef01accb2721e8687c9af8bcaf45d2b69c98"
}
```

`pkg_list` key（如 `aarch64_docker_image`）是 App 内逻辑名；`PackageMeta.name` 是包内容对象名。对不带 env qualifier 的 `pkg_id`，PackageEnv 可以添加目标环境的合法 qualifier；去除该 qualifier 后，`PackageMeta.name` 必须与 `pkg_id` 的 `unique_name` 相同，并满足 §2.2.1 的 App Package Namespace。`.pikg` 内首选 payload 名为 `$sub_pkg_name.tar.gz`；Installer 对该文件最终压缩字节计算 SHA-256，并要求与 `PACKAGE_META.json.content_index` 及（可用时）Package Meta 内容引用一致。

Package Meta 对象“已存在”本身不等于该 package 内容就绪。Inspect/Acquire 必须读取其 body、校验 schema/namespace，并把 `content` 指向的 payload 展开进 `required_contents`；只有 meta 与全部展开后的 payload 都可用时，Content Readiness 才能为 `READY`。

### 11.4 InstallPlan

`InstallPlan` 是 Inspect Stage 的输出，定义在 `buckyos_api::app_install`。当前持久化格式由 `APP_INSTALL_SCHEMA_VERSION = 3` 标识；这是 beta 2.2 breaking schema，缺少或不等于当前版本的 Task/Plan 必须拒绝，不做旧字段兼容。v3 把安装分类和稳定 App 实例身份写入任务与长期记录。

```rust
pub struct InstallPlan {
    pub schema_version: u32,
    pub app: AppDocumentRef,
    pub resolution: DidResolutionSnapshot,
    pub target: InstallTarget,
    pub selected_packages: Vec<SelectedPackage>,
    pub required_contents: Vec<PlannedContent>,
    pub readiness: PlanReadiness,
    pub target_issues: Vec<String>,
    pub config_issues: Vec<String>,
    pub permission_options: Vec<PermissionItem>,
    pub install_params: InstallParams,
    pub service_spec_config: ServiceSpecConfig,
    pub estimated_download_bytes: u64,
    pub plan_fingerprint: String,
    pub created_at: u64,
}
```

相关子结构：

```rust
pub struct AppDocumentRef {
    pub did: DID,
    pub object_id: ObjId,
    pub name: String,
    pub version: String,                     // App Document.version
}

pub struct InstallTarget {
    pub node_did: Option<DID>,
    pub node_id: Option<String>,
    pub os: String,
    pub arch: String,
    pub kernel_version: Option<String>,
    pub runtime_version: Option<String>,
    pub capabilities: BTreeMap<String, i64>,
}

pub struct SelectedPackage {
    pub sub_pkg_name: String,                // pkg_list key
    pub pkg_id: String,
    pub package_meta_id: Option<ObjId>,
    pub docker_image_name: Option<String>,
    pub docker_image_digest: Option<String>,
    pub required: bool,
}

pub struct PlannedContent {
    pub content_id: String,                  // ObjId 或 sha256:<hex>
    pub sub_pkg_name: Option<String>,
    pub package_meta_id: Option<ObjId>,
    pub expected_docker_image_digest: Option<String>,
    pub format: Option<String>,
    pub size: Option<u64>,
    pub location: ContentLocation,           // installed | named_store | pikg | missing
    pub sources: Vec<String>,
}

pub struct InstallParams {
    pub selected_components: Vec<String>,
    pub permissions: Vec<PermissionItem>,
    pub data_mount_points: HashMap<PathBuf, MountPointConfig>,
    pub local_cache_mount_points: HashMap<PathBuf, MountPointConfig>,
    pub external_mount_points: HashMap<PathBuf, MountPointConfig>,
    pub service_settings: ServiceSettings,
    pub bash_envs: HashMap<String, String>,
    pub res_pool_id: Option<String>,
    pub auto_start: bool,                    // 缺省 true
}

/// 七维 readiness；install 为综合结论（§4.5 / §4.6）
pub struct PlanReadiness {
    pub document_syntax: ReadinessState,     // READY | NOT_READY | UNKNOWN
    pub trust: ReadinessState,
    pub package_integrity: ReadinessState,
    pub content: ReadinessState,
    pub target: ReadinessState,
    pub config: ReadinessState,
    pub install: InstallReadiness,           // OFFLINE_READY | CONTENT_DOWNLOAD_REQUIRED | ...
}
```

`service_spec_config` 是 Inspect 阶段由 `AppDoc.service_config_tips + InstallParams` 唯一推导的最终运行配置，也是用户实际批准的配置。Prepare 必须原样使用它，禁止确认后再按另一套规则二次构造。无法满足的必需 endpoint/env、未知 service 或不安全 mount 写入 `config_issues`；SDK/runtime 或数值能力不满足写入 `target_issues`。

`permission_options` 是 AppDoc 声明的完整权限候选；`InstallParams.permissions` 是本次实际批准并将写入 `AppServiceSpec.permission` 的完整 `PermissionItem` 条目。普通安装默认预选全部 `required=true` 条目，`SYSTEM_INTERNAL` 自动确认默认预选全部候选。批准项必须是候选项的子集，按 `scope_path` 唯一，且 `required/actions/exp` 必须与 AppDoc 声明完全一致；不得通过安装参数扩权或篡改权限语义，任何必需权限都不能漏选。

数据与 external mount 的 tips 是可选范围，不是自动授权：只有用户在 `InstallParams` 中选择、且 container path 已由 AppDoc 声明的项才进入最终配置；AppDoc 声明为只读的 mount 不得被参数提升为可写。local-cache tips 位于应用私有缓存根下，可自动生成默认映射并允许用户在声明范围内覆盖。

默认目标快照从 `devices/{node}/info` 读取：`DeviceDocument.capbilities` 是通用能力来源，DeviceInfo 的内存/GPU 实测字段覆盖同名标准能力；`kernel_version`、`runtime_version`（或 `buckyos_version`）从 Device Document 扩展字段读取。不得拿 Control Panel 自己的编译版本冒充目标 Node 版本；目标没有上报而 AppDoc 又声明要求时，必须进入 `UNSUPPORTED_TARGET`。

`InstallParams` 使用 `deny_unknown_fields`。旧版的自由 JSON key（例如顶层 `sub_hostname` / `expose_ports`）不会被静默忽略，而是在 RPC 解析阶段直接报错；调用方必须提交上面的强类型结构。

示例（省略未使用的空配置字段）：

```jsonc
{
  "schema_version": 3,
  "app": {
    "did": "did:bns:pikg-docker.root",
    "object_id": "appdoc:0bb3711c71b8dd4606f430f4884586f58aaab064e69238549010e8f4c19abe85",
    "name": "pikg-docker",
    "version": "0.1.0"
  },
  "resolution": {
    "app_did": "did:bns:pikg-docker.root",
    "doc_type": "app",
    "app_doc_object_id": "appdoc:0bb3711c71b8dd4606f430f4884586f58aaab064e69238549010e8f4c19abe85",
    "document_status": "Active",
    "document_version": 1785800758,
    "expected_owner": "did:bns:root",
    "evidence": "Anchored",
    "verification_status": "Passed",
    "cache_status": "ZoneHit",
    "warnings": []
  },
  "target": {
    "node_id": "node1",
    "os": "linux",
    "arch": "aarch64",
    "runtime_version": "0.7.0",
    "capabilities": { "memory": 8589934592 }
  },
  "selected_packages": [{
    "sub_pkg_name": "aarch64_docker_image",
    "pkg_id": "root_pikg-docker-image#0.1.0",
    "package_meta_id": "pkg:67e624552871d410c63a2611b5500d32a72ff866caddc039249700ca6642ba8c",
    "docker_image_name": "local/pikg-docker:0.1.0-arm64",
    "docker_image_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    "required": true
  }],
  "required_contents": [{
    "content_id": "pkg:67e624552871d410c63a2611b5500d32a72ff866caddc039249700ca6642ba8c",
    "sub_pkg_name": "aarch64_docker_image",
    "expected_docker_image_digest": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
    "format": "named_object",
    "location": "pikg",
    "sources": []
  }],
  "readiness": {
    "document_syntax": "READY",
    "trust": "READY",
    "package_integrity": "READY",
    "content": "READY",
    "target": "READY",
    "config": "READY",
    "install": "OFFLINE_READY"
  },
  "target_issues": [],
  "config_issues": [],
  "permission_options": [{
    "scope_path": "wan",
    "required": false,
    "actions": [],
    "exp": null
  }],
  "install_params": {
    "permissions": [{
      "scope_path": "wan",
      "required": false,
      "actions": [],
      "exp": null
    }],
    "service_settings": { "services": {} },
    "auto_start": true
  },
  "service_spec_config": { "res_pool_id": "default" },
  "estimated_download_bytes": 0,
  "plan_fingerprint": "planfp:...",
  "created_at": 1785800800
}
```

`plan_fingerprint` 使用 JCS + SHA-256，绑定 schema version、`AppDocumentRef`、除 `resolved_at` 外的完整 resolver 信任结论、完整 target（含 runtime/capabilities）、强类型安装参数、最终 `ServiceSpecConfig` 与 selected packages（含 Docker digest）。任一变化必须重新 Inspect 和确认；Verify 必须现场重算 fingerprint，并同时核对 approval 中的 fingerprint、target 和参数。若 resolver 后续返回 `Revoked` / `Tombstoned`，任何尚未 Deploy 的计划必须立即作废。`os`/`arch` 必须来自目标 Node 信息，禁止用 Control Panel 编译期 `cfg!(target_*)` 代替。

Resolver 没有返回可安装 App Document body 时，不得用零值 Object ID、空版本或固定 fingerprint 伪造占位 `InstallPlan`。事务保留 `DidResolutionSnapshot` 并以 `TRUST_RESOLUTION_REQUIRED` 暂停，待重新 Resolve 后再首次生成真实 Plan。

### 11.5 权限、解析快照与安装记录

#### PermissionItem

```rust
pub struct PermissionItem {
    pub scope_path: String,       // 如 user/home、wan、kapi/repo-service
    pub required: bool,
    pub actions: Vec<String>,     // 如 ["read","write"]；可空
    pub exp: Option<u32>,         // None = 长期有效
}
```

安装确认接口不再接受独立的 `accepted_permissions` 字符串列表。调用方在 `install_params.permissions` 中提交完整 `PermissionItem`；Installer 校验其为 `permission_options` 的合法子集，并把 fingerprint 绑定的同一组条目原样写入 `AppServiceSpec.permission`。

#### DidResolutionSnapshot

`(App DID, "app")` 解析证据快照，写入 plan / task data / install_record / proof：

```rust
pub struct DidResolutionSnapshot {
    pub app_did: DID,
    pub doc_type: AppDocType,                      // 序列化固定 "app"
    pub app_doc_object_id: Option<ObjId>,
    pub resolver_id: Option<String>,
    pub document_status: DocumentStatus,           // Active|Missing|Expired|Revoked|Tombstoned|Migrated|Unknown
    pub document_version: Option<u64>,             // 发布 revision = iat
    pub authority_seq: Option<u64>,
    pub effective_owner: Option<DID>,
    pub expected_owner: Option<DID>,
    pub evidence: Option<DidEvidenceLevel>,        // Anchored|NeedProof|UnproofInfo
    pub verification_status: Option<DidVerificationStatus>,
    pub cache_status: Option<DidCacheStatus>,
    pub doc_hash: Option<String>,                  // sha256:<hex>
    pub warnings: Vec<String>,
    pub migration_target: Option<DID>,
    pub resolved_at: Option<u64>,
}
```

#### AppInstallTaskData / InstallRecord

- in-flight 事务：`Task.data` → `AppInstallTaskData { schema_version, request, state: InstallTransactionState }`（含 `plan` / `approval` / `verification` / `prepared` 等）。`request.app_class` 固定本次安装为 `user_installed` 或 `zone_installed`；`AppUpdateTaskData` 使用相同的 `schema_version` 和状态结构。
- 长期记录：个人 App/Agent 写 `users/{uid}/apps|agents/{app_name}/install_record`；Zone App 写 `zone/apps/{app_name}/install_record`。

```rust
pub struct InstallRecord {
    pub schema_version: u32,
    pub app: AppDocumentRef,
    pub user_id: String,
    pub app_instance_id: String,
    pub app_class: AppClass,
    pub resolution: DidResolutionSnapshot,
    pub package_meta_ids: Vec<ObjId>,
    pub pikg_digest: Option<String>,
    pub target: InstallTarget,
    pub state: InstallRecordState, // prepared|deploying|installed|deployed_but_activation_failed|rolled_back|failed
    pub task_id: i64,
    pub proof_id: Option<String>,
    pub plan_fingerprint: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub last_error: Option<InstallError>,
}
```

`AppServiceSpec` 只承载调度/部署所需的 `app_doc`、`app_class`、`permission`、`spec_config` 等，不复制解析证据与任务历史。其稳定实例身份为 `<app_doc.name>@<user_id>`；Zone App 的 `user_id` 固定为 `system`。

### 11.6 App 类型变体

实现枚举 `AppType`，写入 `AppDoc.categories[0]`：

| AppType | categories[0] | 典型 pkg_list | 说明 |
|---|---|---|---|
| `Service` | `service` | 无 web / docker | 系统服务 |
| `AppService` | `dapp` | docker 或 script / 原生 app | 应用服务（含 Docker / Script Host 等部署形态） |
| `Web` | `web` | `web`；`selector_type = static` | 静态网页 |
| `Agent` | `agent` | `agent`，可选 `agent_skills` / `agent_tools` | AI Agent |

同一 App Document 可通过多个 subpackage 组合前端、服务、Agent 等能力；发行方也可采用更严格的系统签名与升级策略，但这不是单独的 `AppType` 枚举值。Docker 容器与 Script Host 是 `AppService` 下的部署形态差异（由 `pkg_list` 选用 `*_docker_image` 或 `script` 等 key 表达），不是额外的顶层类型。

### 11.7 安装分类与作用域

`AppType` 描述运行形态，`AppClass` 描述安装及生命周期作用域，两者正交：

| `app_class` | 创建方式 | spec / install_record | Owner 与默认用户范围 |
|---|---|---|---|
| `system_builtin` | 随 BuckyOS 交付，Installer 不可创建 | 由系统 registry 合成 | Owner=`system`；所有有效用户 |
| `user_installed` | 用户为自己安装；管理员可代装 | `users/{owner}/apps/{app}/...` | Owner 为安装用户；默认仅 Owner |
| `zone_installed` | 仅 Admin/Root 可安装 | `zone/apps/{app}/...` | Owner=`system`；所有当前及未来有效用户 |

`apps.install` 与 `apps.install_package` 必须显式或默认提交 `app_class`；默认是 `user_installed`。Agent 只能是 `user_installed`。升级、启动、停止和卸载以 `app_instance_id` 为外部操作主键，不能用基础 `app_id` 猜测实例。Zone App 只保存一份 spec，不复制到各用户目录。

App–User 自定义可用关系独立保存在 Control Panel 命名空间，不写入安装记录，也不改变 `app_class`。个人 App 分享给 `users` 组后仍是 `user_installed`，生命周期仍属于原 Owner。

---
## 12. 命名、版本与生命周期

### 12.1 唯一性

- **App DID**：逻辑应用身份；
- **App Document Object ID**：某一不可变版本文档；
- **Package Meta Object ID**：某一平台或组件描述；
- **Chunk ID / Digest**：具体实体内容；
- **pikg Digest**：某次交付文件的字节级身份。

同一个 App Document 可以被组装为多个不同 `.pikg`：

- 不同平台子集；
- 不同可选组件；
- 不同压缩方式；
- 不同 Packager；
- 完整包或精简包。

因此 `.pikg` Digest 不能替代 App DID 或 App Document Object ID。

建议逻辑命名：

```text
did:bns:$app_name.$owner_name
```

版本不属于 App DID 本身：应用语义版本保存在 App Document `version`，权威发布 revision 保存在 resolution metadata 的 `document_version / versionId`，其值等于当前发布文档的 `iat`，精确内容由 App Document Object ID 固定。不得用 `#$version_tag` 绕过 `(did, "app")` 的独立版本与撤销语义。

BuckyOS 不强制显示名称全局唯一，但 Source 内部应唯一；推荐命名格式：

```text
$author_$appname
```

上述 `$author_$appname` 不是显示名规则，而是标准 BNS App DID 在平铺 package namespace 中的 owner-first 编码。对于经 AppInstaller 接受的外部 App，它是 `pkg_list` 自有 subpackage 的强制基础前缀：

```text
did:bns:$appname.$author
    ↓ PackageId::from_did
$author_$appname
```

subpackage 的 `unique_name` 必须等于该基础前缀，或使用 `-` 增加角色后缀，例如 `$author_$appname-web`。App 的展示名称仍可自由设置；第三方依赖也不要求使用当前 App 的前缀，但不能被注册为当前 App 的宿主目录、gateway server 或运行入口。

### 12.2 升级流程

1. 客户端调用 `resolve_did(App DID, "app")` 获取当前可信 App Document 及 resolution metadata；
2. 若结果为 `Revoked` / `Tombstoned`，停止升级且不得回退到旧包；若为 `unknown`，按策略使用未作废的已验证 cache 或等待恢复；
3. 对比当前安装记录中的 App Document Object ID、发布版本与新解析结果；
4. 生成升级 InstallPlan；
5. 复用已安装内容和本地 Chunk；
6. 获取新的 `.pikg` 或仅补齐差异对象；
7. 若权限或安装参数发生变化，强制用户确认；
8. 若仅代码更新且策略允许，可以弱提示或自动升级；
9. 在新版本 Activate 和健康检查成功前保留部署回滚状态。

安装部署回滚与 DID 解析 fallback 是两个概念：Activate 失败时可以恢复本机上一套已安装文件，但 resolver 不得因此把一个已 `Revoked / Tombstoned` 的旧 App Document 重新视为当前可信结果。

### 12.3 卸载与数据保留

卸载应遵循 App Document 中声明的 persistence 策略，并明确区分：

- 应用二进制与缓存；
- 用户数据；
- 配置；
- 模型或共享 Chunk；
- 授权与购买凭证；
- 安装历史和信任决策。

共享内容仍被其他 App 使用时不得直接删除。

---

## 13. 兼容性与迁移



### 13.1 旧直接下载源

App Document 中既有的 Docker、GitHub Releases、Source OOD 和作者 OOD 地址仍可作为 Object Provider。迁移不要求所有发行者立即生成 `.pikg`，但推荐 Source 在服务端或客户端 Acquisition 阶段生成可下载的 `.pikg`。这些地址只提供内容，不得注册成 DID resolver-provider 或改变 App DID 的发布状态。

### 13.2 pikg 与网络对象共存

`.pikg` 可以是：

- 完整离线包；
- 只含 App Document 与少量 Package Meta 的引导包；
- 只针对一个平台的包；
- 开发态本地包；
- 带有缓存内容的分享包。

Installer 必须按当前目标计算缺失内容，不能假定所有 `.pikg` 都完整，也不能因为包不含其他平台内容就判定其损坏。

---

## 14. 待确定事项与 Roadmap

### 14.0 已冻结事项（D1-D5：2026-07-16；D6：2026-08-04，v0.5 实现基线）

以下 D1-D6 决策已冻结为实现基线。本节与 §14 其余小节及正文冲突时，以本节为准；§14.1-§14.4 中未被本节覆盖的条目仍是 Roadmap。

#### D1. `pikg` 外层编码（冻结 §14.1 的容器部分）

- 首版容器固定为 ZIP（按需 ZIP64），MIME 为 `application/vnd.buckyos.pikg+zip`。`.pikg` 扩展名仅用于 UX；格式判断以容器 magic（`PK\x03\x04` local file header）与包内结构为准。
- 格式版本由 `PACKAGE_META.json` 的 `@schema = buckyos.pikg.package-meta.v1` 表达；缺失或不识别的 `@schema` 判定 `INVALID_PACKAGE`。
- entry 名必须是合法 UTF-8 的包内相对路径；拒绝绝对路径、`..` 路径段、反斜杠分隔符、NUL、重复 entry（含目录/文件类型冲突）与 symlink entry。
- 首版限额：entry 总数 ≤ 4096；`APPDOC.jwt` / `APPDOC.json` 单个 ≤ 1 MiB；`PACKAGE_META.json` 与 `objects/*.json` 单个 ≤ 8 MiB；结构化 metadata（上述文件）解压总量 ≤ 64 MiB。payload entry 流式读取校验，不整体载入内存；ZIP 声明 size 与实际解压字节数不一致判定 `INVALID_PACKAGE`。
- payload entry 允许 stored 或 deflate 压缩方式；digest 的计算对象始终是 entry 解压后的字节（即 `$sub_pkg_name.tar.gz` 文件本身的最终压缩字节）。推荐 Packager 对已压缩的 `.tar.gz` payload 使用 stored。
- `pikg_digest = sha256(整个 .pikg 文件字节)`，用于 staging 固定（防 TOCTOU）、安装记录与安装证明。

#### D2. App Document schema 与 Object ID（冻结 §14.2 与 §14.3 的 schema 部分）

- 复用现有 `AppDoc`（PackageMeta flatten）；App DID 使用父链 `BaseContentObject` 已有的必填 `did` 字段，删除重复的 `AppDoc.id`，并保留必填 `doc_type`（固定 `"app"`）。反序列化时缺失 `did` / `doc_type`，或 `doc_type` 不为 `app`，一律拒绝。不另造第二套 App Document 类型。
- canonical JSON 固定为 JCS（RFC 8785），实现即 `ndn-lib::build_named_object_by_json` 的 `serde_jcs` 路径；禁止对 `serde_json::to_string()` 结果直接做 hash。
- App Document Object ID 的 obj type 固定为 `appdoc`：`ObjId = appdoc:hex(sha256(JCS(body)))`。Package Meta 沿用 `pkg` obj type 与同一 JCS 规则。
- `APPDOC.jwt`（JWT 封装）的 Object ID 对 JWT claims（payload JSON）按同一规则计算；`APPDOC.jwt` 与 `APPDOC.json` 的一致性判断 = 两者 canonical JSON 相等（等价于 App Document Object ID 相等）。
- `version` 仅表示应用语义版本；`document_version / versionId` 仅存在于 resolver metadata 与 `DidResolutionSnapshot`，值等于发布文档的 revision `iat`，两者不得复用同一字段。
- `pkg_list` 各 entry（SubPkgDesc）新增 `selector { os?, arch?, min_kernel_version? }` 与 `required`（省略视为 `true`）；已知 key（`amd64_docker_image`、`web`、`agent` 等）未显式声明 selector 时按固定命名表派生，未知 key 无显式 selector 时不参与自动选择。`pkg_objid`（Package Meta Object ID）与 `source_url`（内容获取位置）保持独立字段。
- `permissions` 是 `PermissionItem[]`，每项只包含 `scope_path`、`required`、`actions` 与可选有效期 `exp`；旧 `PermissionRequest.grant/items/constraints` 结构已删除。InstallPlan 用 `permission_options` 暴露 AppDoc 候选，安装确认通过 `install_params.permissions` 提交完整批准条目；旧 `accepted_permissions` 字符串通道已删除。
- App DID 的建议命名沿用 §12.1：`did:bns:$app_name.$owner_name`；builder 必须显式接收 App DID 或按该规则显式构造，禁止把 candidate 自声明 owner 拼出的 DID 当权威身份。

#### D3. 安装记录真相源

- in-flight 安装事务的唯一真相源是 TaskManager 的 `Task.data`（`AppInstallTaskData`，可恢复 transaction）。
- 长期安装记录独立保存在 system-config：个人安装写 `users/{uid}/apps/{app_name}/install_record` 或 `users/{uid}/agents/{app_name}/install_record`，Zone 安装写 `zone/apps/{app_name}/install_record`（`InstallRecord` JSON）。
- 写入顺序：Prepare 完成先写 `install_record(state=prepared)` → 写 spec（Deploy 的开始点）→ Activate 与健康检查成功后更新 `install_record(state=installed)` → 写 installed proof → Task Completed。失败与回滚更新同一记录。
- `AppServiceSpec` 继续只承载 scheduler/node-daemon 所需的部署 spec、`app_class` 与最终批准的 `permission`，不复制解析证据与任务历史。

#### D4. LOCAL_DEVELOPER authority override

- 本轮不新增 Zone Resolver cache-injection 管理 API。Zone Resolver 数据面 `resolver/cache/{did}/{doc_type}/{state|doc}` 仍只接受 RBAC 管控的 system-config KV 写入（kernel/system 角色或 root/su_admin），仅供开发/测试环境显式种入解析证据。
- 在受控注入 API 落地前，`LOCAL_DEVELOPER` 策略下若 resolver（含 Zone Resolver 与本机 DID cache）没有可接受的解析证据，Installer 必须返回 `TRUST_RESOLUTION_REQUIRED`；不得把"本地文件存在"当隐式信任。Installer 自身不得写任何 `resolver/cache/*` key。
- `LocalAuthorityOverride` 的 scope 与 warning 在 `DidResolutionSnapshot` 中保留字段位置（warnings、cache_status），上游注入 API 就绪后 Installer 侧直接透传，无需再改协议状态。

#### D5. 本地包交付边界

- `apps.install_package` 只接受经 Control Panel 上传通道换取的不可猜测 staging handle；服务端将 handle 解析到受控 staging root 下的 immutable 文件，canonical path 必须位于 staging root 内，否则拒绝。
- 外部 RPC 一律不接受服务端文件路径；只有进程内调用（测试、系统内部）允许直接提供本地 Path。

#### D6. App DID 与 Package Namespace 绑定

- AppInstaller 必须在 Inspect Stage、任何 payload Acquisition 和目录副作用之前，对外部 App Document 的全部 `pkg_list` entry 执行 §2.2.1 的 namespace 归属校验；Verify Stage 取得 Package Meta 后必须再次校验。
- 标准 `did:bns:$app_name.$owner_name` 的 namespace 固定为 `$owner_name_$app_name`，与当前 `PackageId::from_did` 规则一致。subpackage `unique_name` 只能等于该 namespace，或以 `namespace-` 为边界添加角色后缀。
- 该规则必须检查全部平台和可选 entry，而不是只检查当前 InstallPlan 选中的 package。Namespace 不匹配统一返回 `APP_PACKAGE_NAMESPACE_MISMATCH` / `INVALID_PACKAGE`，不得创建 PackageEnv 目录、友好链接或 gateway 配置。
- App DID/owner 签名、App Document Object ID、Package Meta Object ID 和内容 Digest 均不能替代 namespace 授权；它们证明身份或内容，不证明该身份有权占用任意包名。
- `LOCAL_DEVELOPER`、好友分享和自有 App 不能跳过安全名字语法及 namespace 归属检查；`SYSTEM_INTERNAL` 只能通过系统 allowlist 使用例外 namespace。

### 14.1 pikg 文件编码

> 容器格式、版本表达、限额与 digest 计算对象已由 §14.0 D1 冻结；本节其余条目仍为 Roadmap。

需要进一步确定：

- 归档容器格式；
- ZIP64、Tar、CAR 或自定义随机访问格式的选择；
- Header、版本号和 MIME Type；
- 流式读取和断点续传支持；
- `pikg` 外层容器的压缩算法与 Digest 计算对象；
- 不能采用 `$sub_pkg_name.tar.gz` 的特殊 subpackage 格式及其 hash 规则；
- 大模型和超大 Chunk 的外置引用规则；
- 包级 Manifest 与 Packager Signature；
- 加密包和私有授权内容。

### 14.2 Canonical JSON 与 Object ID

需要固定：

- JSON 规范化算法；
- 数字、Unicode、字段顺序和空值处理；
- `APPDOC.jwt` 与 `APPDOC.json` 的一致性判断；
- Package Meta Object ID 计算规则；
- Hash 字段标准，例如 `digest: "sha256:..."`。

### 14.3 App DID 解析契约

需要进一步固化：

- `doc_type = "app"` 的正式 schema、JWT/编码和 `requires_verification = true` 契约；
- `ResolvedAppDocument` 对 `resolution_metadata / document / document_metadata` 的字段映射；
- `Active / Missing / Revoked / Tombstoned / Migrated / unknown` 到 Installer 错误码和 UI 的映射；
- 离线安装所需的最小 DID cache 证据及其过期、owner replay guard 和负状态规则；
- App Document Object ID 与权威 `document_ref(doc_hash)` 的规范化匹配方式。

### 14.4 安装事务与回滚

需要明确：

- Prepare、Deploy、Activate 的原子性边界；
- 容器、文件、数据库迁移和服务注册的回滚机制；
- 部分失败后的恢复规则；
- 多 Node 安装的一致性模型。


### 14.5 支付与 HTTP 402

需要定义 BuckyOS 对 HTTP 402 的标准 UI、Receipt 格式、授权缓存、退款和跨 Source 补救下载流程。

### 14.6 Anti-Sybil

安装证明激励需要引入抗女巫机制，并在隐私、去中心化和可验证性之间取得平衡。

### 14.7 评论与版权保护

- 去中心化评论、垃圾信息过滤和 AI 摘要；
- Runtime 授权校验与 DRM；
- 兼容作者认领、版权转移和收益确权。

---

## 15. 总结

本协议将 BuckyOS App 安装体系统一为以下模型：

```text
App DID / Name / Object ID / URL / Share / Local Build
                         ↓ normalize
                 (App DID, "app") + candidate body
                         ↓ resolve_did
       Trusted App Document + status / evidence / warnings
                         ↓
              Package Acquisition
                         ↓
                    Local pikg
                         ↓
          Inspect + Configure + Readiness
              ┌──────────┴──────────┐
              │                     │
        OFFLINE_READY     CONTENT/TRUST REQUIRED
              │                     │
              └──────────┬──────────┘
                         ↓
                       Verify
                         ↓
              Prepare → Deploy → Activate
                         ↓
                  Installation Proof
```

其核心升级是：

> App Document 负责描述“安装什么”，`resolve_did(App DID, "app")` 负责证明“哪份描述当前可信”，Object ID 与 Chunk 负责证明“内容是什么”，`.pikg` 负责把安装所需内容可靠地带到本地，Installer 则在内容和信任均就绪后负责“如何安全地部署和启动”。

通过这一拆分，正式生态仍可保留多源分发、DID 信任、Curator 背书和经济模型；与此同时，本地开发、Agent 自构建、好友离线分享和弱网络安装可以获得更少的外部依赖、更明确的失败边界和更高的成功率。
