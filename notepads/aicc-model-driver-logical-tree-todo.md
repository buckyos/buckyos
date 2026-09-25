# AICC Model Driver 与 Logical Tree 改造 TODO

日期：2026-09-25。面向后续 CodeAgent；本文是实现任务单，本轮仅整理设计与验收要求，未执行实现。

目标：以厂商 Model Driver metadata 定义规格和官方模型事实，构建 `功能 -> 厂商规格 -> 模型家族:effort -> 物理 instance`。仅加载厂商 metadata、没有任何 Provider 时，也能完成 catalog 校验、逻辑树构建和目录查询。

**本版本是 breaking change，不考虑新旧兼容。新设计应直接替换旧实现，删除不必要的逻辑、减少核心生产代码量；不能只在旧系统上叠加一套新结构。** 不为旧配置、旧目录名、旧内部类型或旧测试保留兼容层。

**本任务只改 AICC 的 Model Driver 与 logical tree，以及直接服务于这两者的通用装配、路由和测试。不改现有 Provider 实现或配置，不扩展为 Provider / Adapter 重构。** 此范围约束优先于下列文档中涉及 Provider、协议转换或计费链路的整体迁移要求。

## 1. 设计依据与当前状态

先阅读：

- [model_defaults.rs 头部设计契约](../src/frame/aicc/src/service/model_defaults.rs)：功能树、厂商规格、权重及选择原则。
- [openai.model.json](../src/frame/aicc/driver_metadata/models/openai.model.json)：已完成 Review 调整的 v2 配置样例，不能改回旧格式来适配旧代码。
- [Model Driver Metadata Schema](../doc/aicc/driver_metadata_schema.md#llm-target-contract-vendor-specifications-and-model-families)：v2 字段、派生规则与校验。
- [Model Driver 与逻辑模型 FS](../doc/aicc/frozen_model_driver_and_logical_model_fs.md#33-llm-厂商规格与模型家族目标)、[逻辑目录](<../doc/aicc/aicc 逻辑模型目录.md>)、[路由设计](../doc/aicc/aicc_router.md)。

已完成的是配置与文档：

- [x] OpenAI 声明 `specs`；模型条目声明 `llm.spec / effort / default_effort / supported_efforts / stability`。
- [x] 已归规格的 OpenAI 模型删除重复的 `logical_mounts`，独立非 LLM 模型保留原挂点。
- [x] 删除手写 `version_order`，排序从官方模型版本推导。
- [x] OpenAI 删除重复的 Model Driver `variants` 模型清单与参数模板。
- [x] 全部 11 份 builtin Model Driver 删除 `model_pricing`，未将旧价格搬到 Provider Rules。

尚未实现：当前 parser 仍按 v1 工作；其他厂商仍需迁移规格与 effort；旧逻辑树依赖 inventory/mounts 生成部分目录，旧代码还保留 Model Driver variant/价格相关路径。不要把当前配置可通过 JSON 语法检查当作新设计已经能运行。

## 2. 工作范围与 Provider 边界

以下路径均相对于 `src/frame/aicc/`：

| 范围 | 允许完成的工作 |
| --- | --- |
| `driver_metadata/models/*.model.json` | 按厂商迁移 Model Driver；统一新 schema；保留与本次无关的能力和非 LLM 语义。 |
| `src/catalog/{schema,validation,mod,tests}.rs` | Model Driver 类型、解析、规格索引、模型语义、effort/版本派生和校验。共用文件里的 Provider Rules / Known Provider 解析契约保持不变。 |
| `src/model/mod.rs` | metadata 驱动的规格节点、家族与实例关系、目录视图、图校验、候选展开及清理。 |
| `src/service/model_defaults.rs`、`src/service/tests.rs` | 将头部契约落到 builtin 定义、overlay、`ServiceModelAssembler` 和目录输出测试。 |
| `src/routing/` | 仅限消费逻辑树层次、按规格及版本选择、约束过滤和显式 fallback；不重写 Provider 调度器。 |
| `src/runtime/`、`src/settings/` | 仅限 Model Driver 新结构的装配、原子 snapshot 更新和相关测试。 |
| `doc/aicc/` | 同步本任务实际完成的 schema/tree 行为，明确仍未实施的 Provider 接入事项。 |

不在范围内：

- 现有 Provider 生产代码，包括 `src/provider/builtin/`、discovery、认证、Provider inventory 构建/缓存与刷新逻辑。
- `driver_metadata/providers/`、`driver_metadata/known-providers/`、Provider 配置、模型映射或渠道价格。
- `src/protocol/` 的 Adapter/codec、真实请求参数转换、请求发送和计费/usage 链路。
- 新增 Provider、真实厂商 API 联调、联网模型探测、UI/SDK/公共 RPC 协议改版。

复用现有库存输入类型；Model Driver/树层只消费已经提供的渠道模型身份、能力和可用变体。不要为了新树绕过现有 Provider 去构造第二套 Provider 实现。

Model Driver 与 logical tree 内部类型、函数签名及任务范围内的调用方可以直接按新设计修改。不要为了维持旧调用面而保留空字段、返回默认值的旧方法、旧类型包装或转换适配层。若删除旧接口暴露出 Provider 生产代码的范围外依赖，应报告具体调用点与最小接入需求；不得恢复兼容壳、擅自扩大 Provider 修改范围，或将未闭合的编译依赖标为完成。

现有 `provider/builtin/openai.rs`、`claude.rs` 等文件内有 `#[cfg(test)]` 测试直接断言旧 Model Driver 的价格、patterns、variants。允许只将这些 **Model Driver 格式断言** 迁移到 catalog/model 专属测试或更新对应 fixture；不得借此修改 Provider 逻辑、删减 Provider 行为测试或忽略失败。交付时单独列出此类纯测试差异。

## 3. 已确认的设计规则

### 3.1 厂商规格与模型事实

- 一厂商一份 Model Driver。顶层 `specs[].id` 是稳定规格名，路径为 `llm.{spec.id}`；不预设跨厂商统一五档，不要求规格前缀等于 driver ID。
- GPT 为 nano/mini/standard/pro/max 五个通用规格，另有 `gpt-codex` 专用规格；其他厂商按头部契约维护自己的产品线或部署分组。
- 主体为官方 `models[].id`、`api_types`、`capabilities` 与 `llm`。每条有效、未排除的 LLM 规则恰好归入本厂商一个已声明规格。
- `effort` 固定该模型进入规格时的强度；`default_effort` 用于家族直选；二者都必须属于非空、无重复的 `supported_efforts`。
- 家族默认路径为 `llm.{规范化官方模型ID}`，例如 `gpt-5.6-sol -> llm.gpt-5-6-sol`。只规范化目录名，不能改官方调用身份。`family_id` 仅用于消除命名冲突。
- `supported_efforts` 是强度事实的唯一来源；从中派生 `reasoning-{effort}` 语义身份，不再配置一份 Model Driver `variants`。`native` 使用 base model，不制造 effort 参数；`thinking` 不得伪装为 `medium`。
- 本任务实现 effort 的语义与树层约束。最终 wire 参数转换仍属于 Adapter/Provider，不在 Model Driver 中重建参数模板，也不在本任务改 Adapter。
- 已归规格的模型省略 `logical_mounts`。功能目录引用规格，由通用树负责；能力声明本身不等于自动获准进入某个任务。
- 独立图片、音频、视频、embedding、rerank 模型保留现有非 LLM 挂点设计。对原先由 LLM 直接挂载的非 LLM 功能，须在通用树侧明确引用与 API 约束，不能通过保留 `api_types` 假装路由已经保留，也不能把 LLM 整批自动挂入这些任务。
- Model Driver 不接受 `model_pricing` 或内联价格，不产出默认价/估值。缺失价格是 unknown，不能解释为免费；Provider Rules 的价格结构及行为不变。

### 3.2 版本与选择

- 不填写 `version_order`；按厂商命名约定提取官方发布版本，不能把参数量、日期或产品后缀当版本。
- 缺失分量补零；单数字 minor/patch 使用 `major * 100 + minor * 10 + patch`，例如 `5.6 -> 560`、`5.5 -> 550`、`6 -> 600`、`5.6.1 -> 561`。多位分量用数值元组比较，保证 `5.6 < 5.10 < 6.0`，不能按浮点数或字符串比较。
- 同版本允许同值，在同规格内以规范化家族 ID 升序稳定决胜；不得通过配置声明顺序制造版本差异。不可识别的版本在同稳定性类别的已识别版本之后兜底。
- 先按请求/任务约束、库存和可执行性过滤，再按任务权重选择非空规格；规格同权重时按规格 ID 排序。规格内先选合格的最新稳定版本，旧版兜底；无合格稳定版且策略允许时才使用实验版。
- 功能到规格的权重与规格内版本顺序分层比较，不相乘、不跨规格比较版本，也不能被通用评分重新压平成全局模型排名。
- 同家族的多个物理 instance 由现有调度机制选择，不增加家族引用次数或权重。规格耗尽后才尝试下一规格。
- `llm` 只作命名空间；`llm.fallback` 默认空。任务、规格、家族禁止隐式 Parent fallback；家族直选默认 strict。显式 fallback 保留原请求及任务约束。

### 3.3 静态树与动态树

```text
仅厂商 metadata + builtin 功能定义/overlay
  -> 功能目录 + 已声明规格目录 + 功能到规格的带权引用

再叠加有效库存数据
  -> 动态家族 + 固定 effort 选择器 + 物理实例引用
```

零 Provider 时，所有已加载的规格仍存在且可查询；规格内容为空，不预建家族/effort 实例，不伪造可执行模型。请求推理应得到无候选，而不是目录缺失、panic 或回退到根。

库存移除最后一个对应模型后，删除动态家族、预设和入边，保留规格、功能目录和偏好权重。官方渠道模型消失不等于其他渠道的同家族也消失。`:high` 是家族选择器，不是 `.high` 子目录。

即使零库存也必须校验：规格 ID 唯一、引用存在、模型归属唯一、effort 有效、家族命名不冲突、图无环。规格须被功能引用或显式 `direct_only`；`direct_only` 不能通过任务/fallback 暗中接入，显式接入任务时须同时解除该标记。

### 3.4 Breaking change 与实现简化

- 只保留一套 Model Driver schema 和一条 LLM 建树/选择流程；旧 schema 明确拒绝，不自动升级、不双读双写、不使用 feature flag 保留旧路径。
- 新字段替代旧职责时，同时删除相应旧字段、解析器、编译产物、校验、运行时分支和过时测试。不能只删除 JSON 中的字段而保留其实现。
- 不兼容旧 LLM 目录名、挂点与别名，不自动把旧 `llm.{driver}.{model}` 重定向到新家族。真实 Provider 渠道身份映射仍按原有边界处理，不属于旧逻辑目录兼容。
- 能从 `specs / llm / supported_efforts` 或官方模型 ID 确定的信息只派生一次，不维护第二套成员列表、排序表、挂载展开器或 variant 参数字典。
- 复用确有新设计用途的图结构、overlay、匹配器和校验；不要为旧类型仍被引用而增加中间层，也不要为本次简单派生另造通用插件/规则引擎。
- 保留非 LLM 行为是本任务的功能边界，不是保留旧 LLM 实现的理由。共用 helper 有非 LLM 使用者时，删除其失效的 LLM 分支，不能误删其他功能。
- 以受影响核心生产代码净减少为目标；测试、文档和 metadata 数据量单独统计。不得靠删除有效测试、压缩格式或隐藏到新文件中凑减少量。

## 4. 实施清单

### P0：Model Driver v2 与独立 catalog

- [ ] 在 catalog 层加入 typed `specs` / `llm` 与派生的家族、effort、版本视图，沿用现有精确匹配优先和规则解析框架。
- [ ] 单独提升 Model Driver schema；Provider Rules、Known Provider 和 system-config envelope 的版本与契约不变。按仓库规则不保留旧 LLM schema/行为的双轨兼容。
- [ ] 允许 `CatalogDocuments` 只有 `model_drivers`，`provider_rules`、`known_providers` 均为空；不要求通过 Provider 注册才能枚举或校验 Model Driver。
- [ ] 区分 catalog 内模型事实校验与树层功能引用校验；`direct_only`/功能引用关系在组装后的树中验证，不能要求 catalog 加载器连接 Provider。
- [ ] 删除 Model Driver 的价格字段、编译表、查询与兜底逻辑；resolver 结果中删除只为旧 Model Driver 价格服务的槽位，不保留“始终为空”的兼容字段。Provider Rules 仍需使用的价格类型/编译器不删；范围外调用依赖按 §2 记录。
- [ ] effort 身份从支持列表派生；无 `variants` 表能解析、校验和建树，禁止用硬编码模型列表替代已删除的配置。

### P1：迁移厂商 metadata

- [ ] 以 OpenAI 审阅稿为样例，迁移所有 builtin Model Driver 的新格式；完整内置树引用的规格必须同时有声明，不能只完成 OpenAI 后屏蔽其他厂商的失败。
- [ ] 依据头部契约归档已有官方模型，明确通用规格与专用规格；不能用宽泛 pattern 自动收录未知未来型号，也不能用参数量猜规格。
- [ ] 删除 LLM 的旧 `version_rules.current_mount / auto_mounts`、重复挂点和手写 variant 参数表，保留必要的非 LLM 行为。没有可靠能力/effort 依据时列出具体缺口，不能编造。
- [ ] Model Driver 身份保持与现有渠道引用一致；不通过改 Provider metadata 来完成本次迁移。

### P2：树构建、视图与重建

- [ ] 在 `ModelRegistry::build` 中先从 catalog 建规格，再处理库存；不要把规格声明藏在 `register_inventories` 或动态 mount 流程里。
- [ ] 将 `model_defaults.rs` 的头部功能树/权重落实到 builtin definitions 与 overlay；对 LLM 禁止通用 Auto/Hybrid 绕过规格归属。
- [ ] 使用 catalog 官方身份与现有库存身份的交集创建家族；渠道别名仍由现有上游解析，本层不猜 alias 或最新模型。
- [ ] 从模型支持 effort 与库存明确提供的可用变体形成家族候选；不能仅因 metadata 有某个 effort 就宣称某个真实渠道可以执行它。无可执行强度的实例不成为对应预设候选。
- [ ] 复用 `factory -> system -> user -> session` overlay 顺序与既有图校验；允许显示空规格，更新目录/定义输出，保持公共 RPC 形状不变。
- [ ] 重建具有确定性且幂等；失败保留旧 snapshot，不发布半棵树；同一份 metadata 与库存的顺序变化不影响最终结构。

### P3：树层选择与回归

- [ ] 实现分层选择、稳定性及版本顺序，保留请求约束、显式 fallback 和家族直选语义。
- [ ] 逻辑树消费端保留选中的固定 effort，不允许树层请求合并把它换成另一档；真实请求最终参数的锁定/转换另列 Provider/Adapter 接入任务。
- [ ] 更新原有“零库存没有 `llm.gpt-standard`”的测试；这是需要修正的旧行为，不能通过删掉测试来规避。
- [ ] 检查非 LLM 目录和现有 Provider 库存输入在新树中的消费，记录没有现成渠道变体/参数映射可用的模型；不改 Provider 来补足。

### P4：删除旧实现并检查代码量

- [ ] 删除 LLM `version_rules` 的旧 current winner、版本挂点和 `auto_mounts` 展开流程；只保留新版本派生与规格内排序所需代码。
- [ ] 删除 LLM 的重复 `logical_mounts` 展开、通用 Auto/Hybrid 自动吸入、旧目录别名及隐式 Parent fallback 分支，不让它们成为新流程之外的备用路径。
- [ ] 删除 Model Driver 手写 `variants` 的解析、匹配、参数模板与默认 lowering；语义身份由 effort 派生，不用生成一份旧 variant 配置再调用旧引擎来实现。
- [ ] 删除 Model Driver 价格入口与专用代码，清理失效类型、索引、缓存、helper、错误分支和默认值；不影响仍被 Provider 或非 LLM 使用的共用逻辑。
- [ ] 更新所有任务范围内的调用方、fixture 与断言，删除只验证被废弃行为的测试，并用新契约测试覆盖对应职责；不保留 v1 fixture 作为兼容测试。
- [ ] 给出旧职责到新实现的对应关系及实际删除位置，统计受影响生产代码的新增/删除行数。若没有净减少，解释必要新增与尚可删除的复杂度，不能只以测试通过作为简化已经完成的证据。

## 5. 必须实现的离线测试

测试必须走生产 parser、catalog compiler、registry/overlay 和候选选择代码，不能另写一套测试专用建树算法。测试名称可调整，下面的输入条件和断言必须保留。

### 5.1 零 Provider、仅厂商 metadata

主 fixture 明确满足：`CatalogDocuments { model_drivers: ..., provider_rules: [], known_providers: [] }`，registry 的库存输入为 `[]`。不创建 Provider 对象、Provider registry、fake Provider、credentials、网络客户端或外部服务；也不依赖真实数据库、NDN、启动脚本和环境中的 API Key。

必须有两种真实配置 fixture：

1. **单厂商**：直接加载仓库 `openai.model.json`，提供只引用 GPT 已声明规格的功能定义/overlay。不能给完整跨厂商 overlay 后静默忽略其他厂商的悬空引用。
2. **完整 builtin**：直接加载全部 `driver_metadata/models/*.model.json`，使用真实 `builtin_logical_model_definitions()` 和 `builtin_logical_tree_overlay()`；不加载 `providers/` 或 `known-providers/`。

| ID | 输入/操作 | 必须断言 |
| --- | --- | --- |
| Z01 | 单厂商真实 JSON，零 Provider catalog，零库存 | catalog 构建成功；官方模型、spec 与 effort 可查询；不需要 Provider 注册。 |
| Z02 | Z01 加 GPT 功能树 | 六个 GPT 规格存在；任务到规格的 target 和权重可见；家族/物理实例均不存在，规格候选内容为空。 |
| Z03 | 全部真实厂商 metadata + 完整 builtin 树 | 所有功能引用的规格均存在；每个规格有引用或 `direct_only`；无命名冲突和悬空边。不能只测试手写 mini JSON。 |
| Z04 | 通过现有目录视图、定义输出和候选解析入口查询 Z03 | 能区分存在但为空的规格和不存在的路径；功能可列出规格引用，最终 exact 候选为空；不会假造库存。 |
| Z05 | 对空树请求 `llm.plan`、某规格、`llm` 根 | 无合格 exact 候选；不 panic，不隐式回根，不把空规格误报为配置缺失。 |
| Z06 | 最小 metadata 变体：重复/不存在的规格、LLM 缺失归属、无效 effort、规范化名称冲突 | 零库存也失败；错误包含 driver、model/spec 等定位信息，不能等到 Provider 接入才发现。 |
| Z07 | 未引用规格、`direct_only`、显式任务/fallback 引用、引用环 | 普通未引用规格失败；合法 `direct_only` 可独立存在；绕过隔离或形成环失败。 |
| Z08 | 缺少 `variants` / `version_order` / `model_pricing` 的真实 v2 JSON；再注入已删除字段或改回 v1 | 正常配置成功；拒绝旧 schema、手写排序、Model Driver 参数表和价格输入，不自动转换为新格式。 |
| Z09 | 交换厂商和模型条目顺序、重复构建、修改规格声明后重建 | 输出确定且无重复节点；功能仍引用已删除规格时更新失败，保留旧树。 |
| Z10 | 有效 system/local/cloud/builtin Model Driver 替换与无效更新 | 沿用按身份整文档优先级，不跨来源 merge；metadata 变更无需 Provider 才生效；失败不发布部分树。 |

### 5.2 手工库存数据：动态家族与选择

这组测试只手工构造现有 `model::ProviderInventory / InventoryModel` 数据，表示已经解析的库存结果；仍不创建或调用任何 Provider。fixture 的原厂模型定义应来自真实 OpenAI metadata，只有渠道 ID、实例名称、可用性等库存事实使用合成数据。版本边界/非法配置可用最小专项 fixture。

| ID | 场景 | 必须断言 |
| --- | --- | --- |
| D01 | 一个已归规格模型及明确可用的 `high` 变体 | 创建规范化家族和 `:high` 选择器；规格只引用配置指定 effort；exact 使用原始渠道 ID；不创建 `.high` 子目录。 |
| D02 | 两个不同渠道 ID 已归一到同一官方模型 | 共用一个家族和规格引用，挂两个物理 instance，版本顺序/权重不因实例数量翻倍。 |
| D03 | 零库存 -> 增加两个实例 -> 删除一个 -> 删除最后一个 | 静态树始终保留；家族只在最后一个实例消失时清理；反复重建无残留入边。 |
| D04 | `5.5 / 5.6 / 6 / 5.6.1 / 5.10`、同版本不同后缀、无版本、日期及参数量后缀 | 验证整数示例、数值分段排序与稳定 tie-break；不因声明顺序、日期或参数量改变版本关系。 |
| D05 | 高权重空规格；新版不满足能力；稳定/实验版混合 | 先过滤可执行性，跳过空规格；合格旧稳定版可兜底；实验版受策略约束；不比较跨规格版本。 |
| D06 | 高权重规格内多个版本与低权重规格的“更高版本”竞争 | 先选规格再选版本；同规格候选耗尽才换规格；不得将任务权重与版本值相乘。 |
| D07 | nano 模型满足 plan 的能力；库存携带旧 `llm.plan` mount；overlay 试图绕过规格 | 不能通过旧 mount、Auto/Hybrid 或直接 exact 引用绕过任务到规格的契约。 |
| D08 | `effort` 与 `default_effort` 不同；固定选择器；库存缺少指定 effort；`native` / `thinking` | 规格使用 effort，家族直选使用 default_effort；不支持时无候选，不偷换强度；native 无合成 effort 变体。 |
| D09 | 任务/规格/家族耗尽；显式 fallback 到能力不足模型 | 无隐式 Parent fallback；家族不自动升级；显式 fallback 仍执行原请求与任务约束。 |
| D10 | 官方实例移除，第三方同家族仍在；非 LLM 库存回归 | 家族保留；图片/音频/视频等已有独立挂点与 API 约束保持，LLM 不自动获得非 LLM 任务入口。 |

旧逻辑目录负向断言：例如原先的 `llm.openai.gpt-5-6-sol` 不得通过别名或兼容分支自动解析到新家族；只接受新契约明确支持的路径。

另加 Model Driver 价格负向测试：输入中出现 `model_pricing` 或内联 `pricing` 被拒绝；resolver 无渠道数据时不产出价格/0/成本估值。Provider 渠道价格与实际结算的测试不在本任务新增范围内，现有测试仍须回归。

## 6. 后续接入事项：只记录，不在本任务实现

- Adapter 将选中的 effort 转成真实请求参数；Provider 渠道差异、能力限制和最终参数锁定；没有对应转换时不得宣称真实调用已支持。
- Provider inventory 内部原有的 Model Driver 价格分支/来源枚举及旧缓存清理。当前任务先保证 Model Driver 已无价格输入和输出，不能把清空配置表等同于整条运行时计费路径已迁移。
- Provider 侧对派生 effort 身份的完整接入、真实厂商 API 回归和新模型渠道支持。

这些是范围外的后续工作，不应通过修改 `.provider.json`、Provider Rust 实现或 codec 来“顺手完成”。交付说明要分别列出本任务已完成的纯 metadata/tree 行为与仍依赖后续接入的真实调用行为。

## 7. 验证与交付

- [ ] 先跑新增的 catalog/model/service 定向测试，再在 `src/` 下执行 `cargo test -p aicc` 和 `cargo check -p aicc --all-targets`。
- [ ] 执行受影响 Rust 文件的格式检查与 `git diff --check`；新增测试应可在无 API Key、无网络和无外部服务的环境运行。
- [ ] 记录环境导致的构建阻塞及已执行的命令，不能把未执行或被忽略的测试写为通过。不要为跑测试启动真实 Provider 或调用付费 API。
- [ ] Review 改动范围：Provider 生产代码、Provider/known-provider JSON、Adapter/codec 无修改；纯 metadata 测试迁移单独列出。
- [ ] 扫描旧 schema、LLM `version_rules`/挂载分支、Model Driver `variants`/价格实现和兼容壳的残留；注明保留代码的现有非 LLM/Provider 使用者，不能笼统标为“以后再删”。
- [ ] 单独报告核心生产代码的新增/删除量和复杂度减少点；测试、文档、模型清单的变化不用于抵消生产代码增长。
- [ ] 交付至少包含：新 schema 与所有 builtin Model Driver 迁移、真实 builtin 零 Provider 树测试、动态数据测试、旧路径删除说明、文档更新、范围外接入清单。

完成标准：使用仓库真实厂商 metadata，在没有 Provider 的情况下能够校验、构建和查询结构完整的逻辑树；注入手工构造的库存数据后，规格/家族/effort/实例的展开、选择和清理满足上述契约。新设计已经替换并简化旧实现，没有旧 LLM 双轨流程或兼容层。不能仅以 JSON 能解析、节点数量正确或 OpenAI 单厂商测试通过作为完成依据。
