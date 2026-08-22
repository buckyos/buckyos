# BuckyOS App 全生命周期与基础设施职责边界

> 状态：Draft · 日期：2026-08-21 · 适用版本：BuckyOS beta 2.2 及以后，不考虑向前兼容。

## 1. 文档目的

本文定义 BuckyOS App 从开发、测试、签名、发布、下载、安装、升级到运行收敛的完整生命周期，以及各基础设施在该生命周期中的职责边界。

本文首先表达产品和架构意图，不冻结具体命令名称、命令行参数、RPC schema 或 UI 交互。后续工具和实现设计必须从这些意图出发，不得重新把底层对象关系暴露成普通 App 开发者需要手工编排的流程。

本文的核心决策是：

> **PIKG 是普通 App 生态唯一公开的构建、测试、签名、发布、分发、安装和升级单元。**

AppDoc、PackageMeta、Chunk、NamedStore、Repo、BNS、InstallPlan、AppSpec 等仍然是重要的内部协议对象或系统边界，但不是普通 App 开发者需要分别操作的发行物。

本文不替代 [App 安装协议](../App%20安装协议.md)。PIKG 格式、AppDoc schema、Object ID、安装事务和安全校验的协议细节以安装协议为准。两份文档发生冲突时，应先判断冲突属于“产品意图”还是“当前协议细节”，再推动协议和实现向本文确定的意图收敛。

## 2. 适用范围与基本约束

### 2.1 普通 App

本文主要面向第三方开发者构建的普通 App。为了降低学习成本和出错概率，普通 App 当前采用以下约束：

- 开发者只选择 `docker`、`script` 或 `static-web` 等运行形态；
- 普通 App 的交付模型对开发者表现为平台无关，不要求开发者构造 OS/Arch package matrix；
- 普通 App 不允许声明需要 Installer 独立解析和下载的第三方 package 依赖；
- 应用运行所需代码、静态资源和依赖必须已经进入 Docker image、script bundle 或 static-web bundle，并最终被当前 PIKG 的内容图绑定；
- 每个对外发布的 App 版本都有一个完整、自包含、可离线验证的 PIKG。

这些限制不是底层系统能力的上限，而是普通 App 生态的产品边界。优先保证开发者能够正确完成完整生命周期，再考虑开放更复杂的组合能力。

### 2.2 系统组件与系统 App

BuckyOS 系统组件和系统 App 仍可能存在多平台构建、平台 selector、独立 PKG、系统级依赖和对象级更新需求。这些能力属于 BuckyOS CI、ROM、系统升级或内部 provisioning 流程，不应因此进入普通 App 的公开开发模型。

系统内部可以复用 PackageMeta、PackageEnv、Repo 和内容寻址能力，但必须与普通 App 的 PIKG-first 流程保持清晰边界。

### 2.3 本文不决定的事项

本文不决定：

- CLI 的最终命令名称和参数；
- PIKG 上传服务的具体网络协议和存储产品；
- Owner key 的具体钱包、HSM 或托管签名器实现；
- Repo、NamedStore 和 CDN 的具体部署形态；
- 对象级增量下载和差分更新的具体算法；
- 系统组件多平台发布流程的完整细节。

## 3. 为什么以 PIKG 为首要概念

旧的 AppDoc + SubPkgMeta 工作流把内部对象图直接暴露给开发者和发布者。使用者必须理解并决定：

- 先构造 AppDoc 还是先构造 SubPkgMeta；
- 哪些对象需要签名、由谁签名；
- payload、SubPkgMeta 和 AppDoc 按什么顺序上传；
- 本地测试使用 candidate、Object ID 还是权威 AppDoc；
- 修改一个 subpackage 后需要重新生成和发布哪些对象；
- AppDoc 已发布而部分内容尚未部署时系统处于什么状态；
- 安装入口应该是 AppDoc、PackageMeta、URL、Object ID 还是本地文件。

这些问题分别可以被文档解释，但其组合会产生大量中间状态和错误路径。问题不只是文档不够完整，而是内部实现细节成为了用户工作流。

PIKG 用少量空间和带宽换取更低的认知成本、更少的状态组合和更稳定的验证边界。其产品模型接近 Android APK：开发者、发布者和用户围绕一个应用包完成生命周期，包管理器负责理解并验证内部结构。

PIKG-first 不否定对象级能力。BuckyOS 仍可在内部使用内容寻址、缓存、去重、按对象下载和 subpackage 部分更新，只是这些优化不得改变普通 App 的公开操作模型。

## 4. 一条贯穿全生命周期的主线

普通 App 的主流程必须保持线性：

```text
开发者：build → test
发布者：sign → upload → publish
普通用户：download → install / update
运行系统：schedule → deploy → converge
```

对应的唯一公开交付物始终是 PIKG：

```mermaid
flowchart LR
    Source["App source / local artifact"]
    Unsigned["app.pikg\nunsigned candidate"]
    Tested["same logical PIKG\nlocally tested"]
    Signed["app.pikg\ncontains APPDOC.jwt"]
    Available["uploaded PIKG\ndownloadable"]
    Published["BNS AppDoc\nauthoritative"]
    Downloaded["downloaded app.pikg"]
    Plan["verified InstallPlan"]
    Spec["AppSpec"]
    Running["running instance"]

    Source -->|build| Unsigned
    Unsigned -->|local install and test| Tested
    Tested -->|sign exact candidate| Signed
    Signed -->|upload| Available
    Available -->|publish embedded AppDoc| Published
    Published -->|discover and download| Downloaded
    Downloaded -->|inspect and verify| Plan
    Plan -->|commit| Spec
    Spec -->|schedule and converge| Running
```

从产品视角看，一个 `app pikg` 贯穿头尾。签名会在容器内增加签名证明，因此文件字节和文件 digest 会变化，但 AppDoc Object ID、PackageMeta Object ID 和 payload digest 不得变化；它仍然是开发阶段验证过的同一个逻辑应用版本。

## 5. PIKG 的产品语义与信任语义

### 5.1 PIKG 是交付容器

PIKG 把一个 App 版本需要的可验证对象和 payload 收纳到一个文件中，提供统一的构建、上传、下载和安装体验。

PIKG 容器本身不是新的身份对象，也不是最终信任根。ZIP entry 顺序、压缩参数或重新封装造成的整文件 digest 变化，不应改变 App 内容身份和发布身份。

`pikg_digest` 可以用于 staging 固定、防止 TOCTOU、缓存、安装记录和运维审计，但不能代替内部 Object ID、内容 digest、AppDoc 签名或 BNS 权威状态。

### 5.2 “签名 PIKG”的准确含义

“签名 PIKG”是面向使用者的产品操作，其准确语义是：

1. 打开并重新验证未签名 PIKG；
2. 读取其中的 `APPDOC.json` canonical body；
3. 使用 App Owner 授权的 key 对完全相同的 AppDoc claims 签名；
4. 生成 `APPDOC.jwt`；
5. 将 `APPDOC.jwt` 与原有 AppDoc、PackageMeta 和 payload 重新封装为正式 PIKG。

它不是对整个 ZIP 文件字节做签名。普通 App 模型不需要：

- `app.pikg.sig`；
- Packager Signature；
- 以整包签名证明 PIKG 由谁组装；
- 把 PIKG 文件 digest 当作 App 身份或发布信任锚。

签名工具不得在签名过程中修改 AppDoc claims，不得向 claims 注入签名时间、发布环境、BNS revision 或其它导致 AppDoc Object ID 改变的字段。签名 key、算法和签名证明应位于签名封装中；发布 revision 和时间属于 BNS/Resolver metadata 或 PublicationReceipt。

### 5.3 签名前后的不变量

正式 PIKG 必须满足：

```text
signed AppDoc Object ID == tested AppDoc Object ID
signed PackageMeta Object IDs == tested PackageMeta Object IDs
signed payload digests == tested payload digests
```

当 PIKG 同时包含 `APPDOC.json` 和 `APPDOC.jwt` 时，两者必须表达相同的 canonical AppDoc。任何不一致都必须拒绝，不能由签名工具自动“修复”。

### 5.4 Sub PackageMeta 不独立签名

普通 App 的 Sub PackageMeta 是 PIKG 内部对象，不要求独立签名。其信任链是：

```text
Owner signature on APPDOC.jwt
  └─ AppDoc.pkg_list[].pkg_objid
       └─ PackageMeta Object ID = hash(canonical PackageMeta)
            └─ PackageMeta.content
                 └─ payload digest
```

Owner 签名的 AppDoc 授权了确切的 PackageMeta Object ID；PackageMeta Object ID 保证 meta 内容不可篡改；PackageMeta 中的内容引用保证 payload 不可篡改。因此再次签名每个 Sub PackageMeta 不增加普通 App 安装所需的信任语义，反而会引入额外签名顺序、密钥和状态组合。

PackageMeta 中的 `owner` 和 `author` 字段只是被 AppDoc 绑定的内容，不应被单独解释成所有权证明。

独立于 AppDoc 发布并被多个系统组件直接解析的系统 PackageMeta，可以有自己的权威发布和签名策略，但不属于普通 App 生命周期。

## 6. 开发与本地测试

### 6.1 开发者看到的输入与输出

普通 App 开发者提供：

- App 的基本描述和运行配置；
- Docker image、script bundle 或 static-web bundle；
- 权限、服务入口、数据目录和其它 App 级声明。

开发工具负责在内部：

- 规范化输入；
- 生成必要的 PackageMeta 和内容 ID；
- 生成未签名 AppDoc candidate；
- 组装完整 PIKG；
- 使用与 Installer 一致的规则重新打开并验证 PIKG。

开发者的主要构建产物是：

```text
app.pikg
```

工具可以输出日志、诊断报告和机器可读测试结果，但不应要求开发者把 `APPDOC.json`、Sub PackageMeta、Object ID 清单或 payload 目录分别交给下一阶段。

### 6.2 未签名 PIKG 可以完成开发测试

开发阶段构建的 PIKG 可以只包含未正式签名的 `APPDOC.json`。开发者使用该 PIKG 完成本地安装、运行、更新、健康检查和行为测试，不需要生产 Owner key，也不需要先把 App 发布到 BNS。

未签名不等于跳过验证。开发安装仍必须验证：

- PIKG 结构；
- AppDoc 和 PackageMeta schema；
- 所有 Object ID 和 payload digest；
- App namespace、运行配置、权限和路径安全；
- PIKG 是否自包含且不存在普通 App 禁止的第三方 package 依赖。

开发环境通过显式、受限、可撤销的 local developer authority 接受该 candidate。该权限只能用于指定的本地 Zone、测试任务或 CI job，并且不能被普通公开安装策略接受。

### 6.3 开发阶段禁止依赖正式发布能力

开发与 CI 不应：

- 持有或读取生产 Owner/BNS 私钥；
- 为了 build 而依赖运行中的 Control Panel、Repo Service 或 system-config；
- 调用公开 BNS 发布接口；
- 把本地验证成功解释为已经正式发布；
- 绕过 Installer 直接写 AppSpec、Gateway 或 RBAC；
- 要求开发者分别发布 AppDoc、PackageMeta 或 payload。

开发工具可以连接本地开发 Zone 完成 staging、安装和运行测试，但所有系统副作用仍通过标准 Installer、Scheduler 和 Node Daemon 完成。

## 7. 正式签名与发布

### 7.1 发布者的唯一输入

普通 App Publisher 的核心输入应是开发和测试完成的 PIKG，而不是一组松散的 AppDoc、PackageMeta 和 payload 参数。

Publisher 必须把开发 PIKG 当作不可信输入重新验证，不能只信任开发工具给出的“测试通过”标志。验证成功后才能使用正式 Owner key 签名。

AppDoc 签名权和 BNS 权威发布权是两个独立授权。一次发布工作流可以编排两者，但不能因为某个 principal 能签 AppDoc 就推断其有权更新 BNS，也不能因为某个服务有 BNS 发布权限就允许它替 Owner 构造或修改 AppDoc。

### 7.2 发布顺序

正式发布应严格遵循：

```text
verify candidate PIKG
    ↓
sign embedded AppDoc and produce signed PIKG
    ↓
upload signed PIKG and verify it is downloadable
    ↓
extract the exact APPDOC.jwt from that signed PIKG
    ↓
publish that AppDoc to BNS as doc_type=app
    ↓
read back BNS/Indexer and verify the result
```

上传必须早于 BNS 发布。这样发布失败最多留下一个已签名、已上传但尚未生效的 PIKG，不会让普通用户发现一个内容尚不可用的新版本。

Publisher 必须发布签名 PIKG 中的原始 `APPDOC.jwt`，不能根据外部参数重新构造另一份 AppDoc。发布完成后必须满足：

```text
BNS current AppDoc Object ID
    == signed PIKG embedded AppDoc Object ID
```

发布和发现基础设施还必须建立“当前 AppDoc Object ID → 正式 PIKG 下载位置”的确定映射，使通过 App DID 安装最终可以取得完整 PIKG。该映射放在 AppDoc、BNS publication metadata 还是索引服务中，由后续协议决定，但必须满足：

- 不要求普通开发者分别公布内部 PackageMeta 和 payload 地址；
- 不允许下载位置在签名后反向修改 AppDoc canonical body；
- 如果下载位置属于 AppDoc claims，就必须在本地测试前已经确定；
- 如果下载位置属于发布 metadata，就必须被权威发布记录绑定到准确的 AppDoc Object ID；
- Installer 从任何下载位置取得 PIKG 后都执行相同的本地验证，不能信任索引或传输来源代替内容校验。

### 7.3 发布状态

发布系统内部可以记录以下状态：

```text
UnsignedLocal
    ↓ sign
SignedUnpublished
    ↓ upload
SignedAvailable
    ↓ publish AppDoc
Published
```

这些状态用于恢复、重试和审计，不应要求 App 开发者手工管理其中的 AppDoc、PackageMeta 或 payload 状态。

- `UnsignedLocal` 只能在明确的开发信任策略下安装；
- `SignedUnpublished` 只证明 Owner 授权了内容，不代表该版本已经公开生效；
- `SignedAvailable` 表示内容已可下载，但 BNS 当前版本尚未改变；
- `Published` 必须同时具有有效 Owner 签名、可获取的 PIKG 和 BNS 权威发布状态。

### 7.4 发布失败和重试

发布过程必须支持幂等恢复：

- 签名失败不得产生部分修改的 PIKG；
- 上传失败不得更新 BNS；
- BNS 发布失败时，已上传的 PIKG 保持未发布状态，旧版本继续有效；
- BNS RPC 成功但回读不一致时，不得报告发布完成；
- 重试不得重新构造不同的 AppDoc claims 或 payload；
- PublicationReceipt 应记录 App DID、AppDoc Object ID、签名 key、PIKG 获取位置、BNS revision 和回读证据。

## 8. 下载、安装与升级

### 8.1 普通用户只安装 PIKG

普通用户的产品体验应是“获得并安装一个 PIKG”。用户可以：

- 通过 App DID、应用商店或索引发现当前版本，再由系统下载 PIKG；
- 直接打开通过文件、好友或其它渠道获得的 PIKG。

无论入口是什么，Installer 最终都处理一个 PIKG。BNS 解析、AppDoc、PackageMeta、Object ID 和内容 digest 校验由系统在后台完成，不要求用户选择或理解这些对象。

公开安装不能只因为 PIKG 内存在签名 AppDoc 就信任它。Installer 仍需验证 BNS 当前权威 AppDoc、Owner 绑定、发布状态和 PIKG 内 AppDoc 的一致性。带外获得的旧版本或已撤销版本不能通过重新打包恢复成当前有效版本。

### 8.2 Installer 的职责

Installer 把来自外部、默认不可信的 PIKG 转换为当前 Zone 可以执行的 AppSpec。其内部仍可使用：

```text
Resolve → Inspect → Acquire → Verify → Prepare → Deploy → Activate
```

但对普通用户应呈现为一个可观察、可确认、可取消和可恢复的安装任务。

Installer 必须：

- 验证 PIKG 及其完整对象闭包；
- 解析并验证 App DID、Owner 和 BNS 权威状态；
- 检查普通 App 没有第三方 package 依赖和公开的平台矩阵；
- 生成权限、存储、网络入口和资源使用摘要；
- 在提交 AppSpec 前把运行所需内容准备到 Zone 内；
- 记录安装来源、AppDoc Object ID、PIKG staging digest 和验证结果；
- 在用户确认的计划与最终 AppSpec 之间保持确定性。

### 8.3 AppSpec 是安装与运行的提交边界

AppSpec 表示用户已经批准且 Installer 已准备完成的目标状态，是安装协议与 Scheduler 之间唯一的提交边界。

- 写入 AppSpec 前，Installer 可以失败、暂停或要求用户确认，Scheduler 不应看到该 App；
- 写入 AppSpec 后，Scheduler 负责分配 Instance，Node Daemon 负责部署和启动；
- Scheduler 和 Node Daemon 不得重新解释外部 PIKG、BNS 或用户安装决策；
- 普通 AppSpec 只能由 Installer 创建或更新，系统镜像构建走明确的 internal provisioning 路径。

### 8.4 升级仍然是安装一个新 PIKG

对开发者和用户而言，App 升级是发布和安装一个新版本 PIKG：

```text
old app.pikg → new app.pikg
```

系统内部可以比较新旧 Object ID，只下载、复制或部署发生变化的内容，也可以复用本地 NamedStore 中已经存在的对象。即使未来实现 subpackage 级部分更新、delta、Range 下载或跨版本去重，公开模型仍然是“一个新版本 PIKG”，不能要求普通开发者单独发布或更新 Sub PackageMeta。

优化必须保持：

- 新版本 PIKG 可独立验证；
- 从全新 Zone 安装不依赖历史版本；
- 增量路径与完整安装路径得到相同 AppSpec 和运行内容；
- 失败时可以恢复旧 AppSpec 和旧版本内容。

## 9. 调度与运行收敛

调度与运行领域从 AppSpec 开始，不处理 App 的外部发现、签名、发布身份和用户安装决策。

Scheduler 负责：

- 读取 AppSpec、Node 状态和资源约束；
- 分配目标 Node 和 InstanceReplica；
- 生成 NodeConfig、ServiceInfo、Gateway 派生配置和调度状态；
- 在 AppSpec 或 Zone 状态变化时重新计算目标状态。

Node Daemon 负责：

- 读取本机 NodeConfig；
- 从 Zone 内已准备内容部署 Docker、Script 或 Static Web App；
- 启动、停止、升级或移除 Instance；
- 上报 InstanceReport、健康状态和错误；
- 持续重试，直到实际状态收敛到 NodeConfig。

Scheduler 和 Node Daemon 不得：

- 解析 BNS 或判断 App Owner 信任；
- 从外部 Repo 临时选择另一个 App 版本；
- 重新执行普通 App 的平台 package 选择；
- 修改 AppDoc、InstallPlan 或用户批准结果；
- 访问 App 的外部发行源补齐 Installer 未准备的内容。

## 10. 基础设施职责边界

| 领域 | 面向使用者的输入 | 核心职责 | 核心输出 |
|---|---|---|---|
| 开发工具 | App source、Docker/Script/Static Web artifact | build、离线验证、本地测试编排 | 未签名且测试通过的 PIKG |
| 发布工具 | 测试通过的 PIKG、Owner 授权 | 重验、签名、上传、发布内嵌 AppDoc、回读 | 已发布 PIKG、PublicationReceipt |
| BNS/Resolver | App DID、已签名 AppDoc | 维护当前权威 AppDoc 和 revision | ResolvedAppDocument |
| PIKG/Object Provider | PIKG 或内部 Object ID | 提供字节，不决定身份和发布状态 | 可验证内容 |
| Installer | PIKG 或可解析到 PIKG 的 App 标识 | 解析信任、验证、准备、用户确认、提交 | InstallPlan、AppSpec、install record |
| Scheduler | AppSpec、Zone 状态 | 分配 Instance 并推导目标状态 | NodeConfig、InstanceReplica |
| Node Daemon | NodeConfig、Zone 内已准备内容 | 部署、启动、停止和运行收敛 | InstanceReport、运行状态 |

职责边界的核心规则是：

- Builder 不发布；
- Publisher 不重新构建 App 内容；
- BNS 不承载 PIKG payload；
- PIKG Provider 不决定 App 是否可信或当前有效；
- Installer 不签名和发布；
- Scheduler 不安装和重新选择版本；
- Node Daemon 不访问外部发行源或重新解释 AppDoc。

## 11. 真相源与写入权

| 数据 | 含义 | 唯一写入者 | 主要读取者 |
|---|---|---|---|
| 未签名 PIKG | 本地可验证发行候选 | 开发工具 | 本地 Installer、Publisher |
| 正式 PIKG | 已加入 Owner AppDoc 签名的交付物 | Publisher | PIKG Provider、Installer |
| BNS AppDoc | 当前权威发布版本和 revision | Publisher/权威发布服务 | Resolver、Installer |
| `Task.data` | 进行中的安装事务 | Installer | Control Panel、Task UI |
| InstallPlan | 当前安装决策快照 | Installer Planner | 用户确认、Verify、Prepare |
| install record | 长期安装审计状态 | Installer | Control Panel、升级和恢复流程 |
| AppSpec | 用户批准的期望运行状态 | Installer | Scheduler |
| NodeConfig | 某 Node 的目标实例状态 | Scheduler | Node Daemon |
| InstanceReport | 某 Instance 的实际运行状态 | Node Daemon/运行时 | Scheduler、Installer、Control Panel |

AppDoc body、Sub PackageMeta 和 payload 是 PIKG 内部内容图的一部分，不再作为普通 App 开发者需要独立维护的工作流真相源。

派生数据不得反向覆盖输入真相源。例如：

- 运行结果不能修改已批准的 AppSpec；
- Repo 中存在某个 AppDoc body 不能自动使其成为 BNS 当前版本；
- PIKG 已上传不能自动解释为已经发布；
- PIKG 文件 digest 相同或不同都不能代替内部对象和 BNS 校验。

## 12. 对工具与 API 设计的约束

后续 CLI、RPC 和 UI 设计应遵守以下约束：

1. 普通 App 的 build 主要输出一个 PIKG，不要求用户管理松散的 AppDoc、PackageMeta 和 payload 发布清单。
2. 本地 test/install 直接消费 PIKG，不要求先把内部对象发布到 Repo/BNS。
3. sign 只接受经过验证的 PIKG，签名其内嵌 AppDoc，并原子地产生签名后 PIKG。
4. publish 以签名 PIKG 为核心输入，自动编排上传、发布同一份 AppDoc 和权威回读。
5. 普通 App 不提供必须由开发者使用的 `pack-subpkg`、`sign-package-meta`、`publish-package-meta` 或依赖解析工作流。
6. install 面向 PIKG 或可解析到 PIKG 的 App 标识；对象级入口只作为内部、诊断或系统组件能力。
7. Builder、Publisher 和 Installer 必须复用同一套 PIKG parser、canonicalization、Object ID 和 verifier。
8. UI 只展示 App、PIKG、版本、签名/发布状态和安装任务，不把内部对象状态矩阵变成用户必须处理的步骤。
9. 内部存储、下载和更新优化不得改变上述公开模型。

一个重要的可用性验收标准是：

> 使用者即使没有阅读 AppDoc、SubPkgMeta 和 Named Object 协议，也能根据当前 PIKG 的状态自然判断下一步是测试、签名、发布还是安装。

## 13. 当前实现的收敛方向

当前实现已经具备可以复用的基础：

- Control Panel 的 `PikgBuilder` / `PikgReader` 可以构造和严格验证 PIKG；
- PIKG 已支持 `APPDOC.json` 与 `APPDOC.jwt` 共存并检查 canonical 一致性；
- AppDoc 可以直接引用 PackageMeta Object ID，PackageMeta 可以绑定 payload digest；
- v0.5 Installer 已有 Resolve、Inspect、Acquire、Verify、Prepare、Deploy、Activate 状态机；
- AppSpec、Scheduler、NodeConfig 和 Node Daemon 已形成运行收敛主链路。

后续实现应优先完成以下收敛：

1. 把 PIKG build/reader/verifier 抽成开发工具、Publisher 和 Installer 可共享的核心库。
2. 让本地 build 不依赖运行中的 Control Panel、Repo Service、system-config 或正式密钥。
3. 建立明确的 PIKG staging/upload 能力，不再让普通调用者传服务端本地路径。
4. 建立 scoped local developer authority，使未签名 PIKG 可以通过标准 Installer 测试而不进入公开信任域。
5. 将当前复合 `app.publish` 收敛为以 PIKG 为输入的签名和发布工作流，不再负责读取源码目录并重新打包。
6. 实现 Owner AppDoc 签名、PIKG 上传、BNS 发布和发布后回读闭环。
7. 普通 App admission 拒绝第三方 package 依赖和开发者可见的平台矩阵。
8. Scheduler 和 Node Daemon 只执行 AppSpec 中的确定结果，不重新解释完整 AppDoc 或平台包选择。
9. 保留对象级存储和部分更新能力，但从普通 App 开发与发布界面中移除相关操作。

## 14. 验收标准

### 14.1 开发体验

- 未启动 BuckyOS、没有正式发行密钥时，可以 build 和离线验证 PIKG；
- 开发者只需要操作一个 PIKG 即可完成本地安装和测试；
- 开发者不需要理解或独立发布 AppDoc、Sub PackageMeta 和 payload；
- Docker、Script 和 Static Web 使用同一条 build/test 主流程；
- 普通 App 中出现第三方 package 依赖或平台 package matrix 时明确拒绝，而不是进入隐式复杂流程。

### 14.2 签名与发布

- Publisher 以测试通过的 PIKG 为唯一主要输入；
- 签名后 AppDoc Object ID、PackageMeta Object ID 和 payload digest 全部保持不变；
- Sub PackageMeta 不需要独立签名；
- PIKG 不需要整包签名或 `app.pikg.sig`；
- PIKG 可下载前不会更新 BNS；
- BNS 发布的是签名 PIKG 内完全相同的 `APPDOC.jwt`；
- 发布后可以回读相同的 App DID、AppDoc Object ID 和 revision；
- 发布失败不影响旧版本继续安装和运行。

### 14.3 安装与升级

- 普通用户只需要下载并安装 PIKG；
- 公开安装同时验证 Owner 签名、BNS 当前状态、内部 Object ID 和 payload digest；
- AppSpec 写入前目标内容已经在 Zone 内 ready；
- 安装计划、用户确认、AppSpec 和最终运行内容保持一致；
- 全新安装与内部增量优化得到相同结果；
- 升级对用户仍表现为安装一个新版本 PIKG。

### 14.4 架构边界

- Builder、Publisher、Installer、Scheduler 和 Node Daemon 没有重复实现彼此的核心职责；
- 普通 App 与系统组件的高级 PKG/多平台流程边界清晰；
- 底层可以继续优化对象级下载、缓存和更新，而无需改变 App 开发者工作流。

## 15. 当前实现入口

| 领域 | 主要入口 |
|---|---|
| PIKG 格式与 Builder/Reader | `src/frame/control_panel/src/pikg.rs` |
| 当前复合 Publisher | `src/frame/control_panel/src/app_installer.rs` 中的 `app.publish` |
| AppDoc/InstallPlan 共享类型 | `src/kernel/buckyos-api/src/app_doc.rs`、`app_install.rs` |
| App namespace 验证 | `src/frame/control_panel/src/app_package_namespace.rs` |
| Installer Planner | `src/frame/control_panel/src/app_install_planner.rs` |
| Installer 状态机 | `src/frame/control_panel/src/app_install_engine.rs` |
| Prepare/Deploy/Activate | `src/frame/control_panel/src/app_install_deployer.rs` |
| Repo/NamedStore 内容原语 | `src/frame/repo_service/src/service.rs` |
| Scheduler | `src/kernel/scheduler/src/app.rs`、`system_config_agent.rs` |
| Node 运行收敛 | `src/kernel/node_daemon/src/app_loader.rs`、`app_mgr.rs` |

## 16. 相关文档

- [App 安装协议](../App%20安装协议.md)：PIKG、AppDoc、InstallPlan、安装事务和安全规则的当前协议基础。
- [BuckyOS App 安装流程](BuckyOS%20App安装流程.md)：Repo 内容原语、证明和 App 生命周期背景。
- [App PKG System](app-pkg-system.md)：PackageEnv、Repo、Node Daemon 和对象级升级能力的历史设计背景；普通 App 公开流程以本文为准。
- [publish_app_to_repo local_dir 格式](../repo_service/publish_app_to_repo_local_dir格式说明.md)：当前复合 Publisher 的历史输入布局，不代表目标开发者工作流。
