# AICC 模型目录与配置例行维护 TODO

日期：2026-11-16。

状态：已执行（2026-11-16，基线 `af0c007f9`）。部署回读与在线 T1/T1.5/T2 未做，见 §6。本文已基于当前源码做静态 Review，基线为 `120d1811b`；本轮只编写任务文档，未修改 AICC 实现、运行测试、调用真实 Provider 或联网核验最新型号。执行 agent 应先复核基线后的变化，再完成下列实现与验证。

> **2026-11-23**：本文的目标 2（每个系列都有廉价 fallback）与新原则冲突，不再有效：系统不内置 fallback，只执行管理员手工配置的模型路由（[aicc_router.md §9](../doc/aicc/aicc_router.md#9-fallback-与失败处理)）。本轮已实施的 `llm.fallback` 分层和用途默认回退将按 [显式 fallback 改造 TODO](aicc-explicit-fallback-todo.md) 移除。下文保留为执行记录。

## 1. 目标与范围

AICC 的模型配置维护是一项持续运营工作。本轮先修复默认目录覆盖与 fallback 策略，再完成一次主流模型核对，并留下后续可重复执行的维护步骤。

需要达到三个结果：

1. **任意一个主流通用 LLM 接入后，能服务全部能力适配的 `llm.usecase`。** 不要求用户为 chat、plan、code、摘要等用途各接一个模型，也不需要逐用途手工补挂点。
2. **每个主流模型系列都有廉价、快速的可用 fallback。** 优先使用低成本、低延迟档位；只有更高档模型接入时，也应能使用现存模型中合适的可执行预设。
3. **按执行日检查主流厂商的型号、能力、价格与下线信息，更新 AICC metadata 和必要的渠道规则。** 新型号加入后，应形成发现、身份匹配、规格归档、路由与协议执行的闭环。

本轮范围是默认逻辑目录、LLM fallback、模型与渠道配置、相关回归及维护文档。复用已有 catalog、registry、overlay、刷新和验收机制。云端 catalog 正式发布、签名与分发平台、逻辑树外置、自动测速平台另见独立任务，不作为本轮前置；不顺带重构非 LLM 的图片、音频、视频路由。

### 1.1 术语及能力边界

- `llm.usecase` 是本文对用途节点的统称，不是实际路径。当前有 `llm.chat`、`llm.plan`、`llm.code`、`llm.swift`、`llm.summarize`、`llm.translate`、`llm.vision`；`llm.fallback` 单独验收。执行时从当前定义枚举，不能把上述名单当成永久固定集合。
- 运营讨论中的 GPT、Claude、Gemini 等“模型家族”，在代码中需要拆成厂商 driver、产品规格 spec、具体版本 family、固定 effort。例如 `gpt-standard` 是规格，`llm.gpt-5-5` 是版本家族。本单按各主流系列维护便宜快档，同时验证每个纳入支持范围的具体模型单独接入的可用性；不要求把所有历史版本平权加入 fallback。
- “覆盖”指目录能够找到满足原请求、原任务、渠道和调用策略约束的真实候选。纯文本模型不能被伪装成视觉模型，缺少 tools/JSON 能力的模型不能通过删除门槛来服务 plan/code。例外必须有能力证据，不能把漏配解释为能力不足。
- “最快”是有依据的运营选型，不是当前调度器已经具备的实时测速承诺。没有一个档位同时最便宜、最快时，记录费用口径、延迟依据及取舍；缺少价格或测速信息时保留未知。

## 2. 当前实现 Review

以下为源码观察；单模型结果是静态推导，尚不是测试通过的结论。历史设计和 `notepads/done/` 只能提供背景。

| 范围 | 当前事实 | 本轮影响 |
| --- | --- | --- |
| 默认用途目录 | [model_defaults.rs](../src/frame/aicc/src/service/model_defaults.rs) 的 `builtin_logical_model_definitions()` 将七个 LLM 用途设为 `Manual + Strict`；`builtin_logical_tree_overlay()` 分别列出规格引用 | 接入模型不等于所有用途可达，需要修复引用与显式兜底策略 |
| 默认 fallback | `llm.fallback` 自身是 `Manual`，后续回退为 `Disabled`；builtin overlay 未给它填入 items | 目前是空目录。仅更新 Model Driver 型号不会使它可用 |
| 目录层次 | [model/mod.rs](../src/frame/aicc/src/model/mod.rs) 的 `ModelRegistry::build`、`materialize_families` 构造“用途 → 规格 → 版本家族/固定 effort → exact instance” | 零库存也有静态规格，动态家族依赖实际 inventory；metadata 有条目不代表 Provider 可调用 |
| 目录约束 | `validate_llm_tree` 禁止 LLM 的通用 Auto/Hybrid 和隐式 Parent fallback，校验规格引用、`direct_only`、预设与图结构 | 不得恢复“把所有模型直接挂到所有用途”的旧路径；优先复用合法规格引用和显式 fallback |
| 回退约束 | 逻辑 fallback 成功后保留原任务的 scheduler、default options、disable line 和能力门槛 | 只把 fallback profile 改成 `CostFirst`，不能证明从 plan 回退也按这个 profile 选型 |
| 调度与执行失败 | [routing/mod.rs](../src/frame/aicc/src/routing/mod.rs) 的 `LatencyFirst` 排序使用 cost/local 权重，不使用实测延迟；已有 `p95_latency_ms` / `max_latency_ms` 用于上限过滤。[inference.rs](../src/frame/aicc/src/service/inference.rs) 的运行期 failover 使用已展开候选池的 `fallback_candidates` | 区分目录无候选回退、候选池内重试、执行失败换模型；不能把新增 `llm.fallback` 当成已改造所有失败重试 |
| 模型配置 | 当前有 15 份 Model Driver、15 份 Provider Rules、13 份 Known Provider；仓库已配置 GPT 6.1 和多种 Claude 5.x 条目 | AICC 并非一直没有更新。型号是否最新、仍在售及各渠道是否开放，需要执行日重新核实 |
| metadata 生效 | [settings/mod.rs](../src/frame/aicc/src/settings/mod.rs) 的 `MetadataSources::resolve` 按身份整文件选择：`system-config > local > cloud > builtin`；[build.rs](../src/frame/aicc/build.rs) 将 builtin JSON 嵌入构建 | 不逐模型合并。旧覆盖文件可能遮住本次新增型号；改 builtin 源文件也不会立即改变已运行的二进制 |
| revision 与云更新 | `BUILTIN_CATALOG_REVISION_SEQ` 当前为 22；[cloud_update.rs](../src/frame/aicc/src/service/cloud_update.rs) 默认仍关闭且无源地址；catalog kind 仍为三类 | 核对现有 revision 约定，不照抄旧笔记中的数值，也不把逻辑目录视为已经可云端下发 |

### 2.1 已定位的单模型缺口

以下假设 inventory 含该模型对应的可执行 effort 和完整有效能力，且没有用户 overlay。它们用于建立回归，不是推荐继续使用这些型号。

| 唯一接入模型 | 规格 | 当前默认目录可达用途 | 缺少用途 |
| --- | --- | --- | --- |
| `gpt-5.5` | `gpt-standard` | chat、plan、code、vision | swift、summarize、translate |
| `claude-sonnet-5` | `claude-sonnet` | chat、vision | plan、code、swift、summarize、translate |
| `gemini-3.8-flash` | `gemini-flash` | chat、swift、summarize、translate | plan、code、vision |

当前 metadata 为这些模型声明了通用工具、结构化输出和视觉能力，上述缺口主要来自目录规格引用。实现 agent 应先用生产 registry/route 路径复现，再修复；不能只写一个扫描 JSON 的脚本就宣布路由通过。

### 2.2 优先阅读与修改入口

| 入口 | 用途 |
| --- | --- |
| [service/model_defaults.rs](../src/frame/aicc/src/service/model_defaults.rs)、[model/mod.rs](../src/frame/aicc/src/model/mod.rs)、[routing/mod.rs](../src/frame/aicc/src/routing/mod.rs) | 默认用途、规格偏好、候选展开、显式回退和调度 |
| [driver_metadata/models](../src/frame/aicc/driver_metadata/models)、[catalog/schema.rs](../src/frame/aicc/src/catalog/schema.rs)、[catalog/validation.rs](../src/frame/aicc/src/catalog/validation.rs) | 原厂身份、能力、spec、effort、稳定性和权重 |
| [driver_metadata/providers](../src/frame/aicc/driver_metadata/providers)、[driver_metadata/known-providers](../src/frame/aicc/driver_metadata/known-providers)、[provider/inventory.rs](../src/frame/aicc/src/provider/inventory.rs) | 渠道别名、操作映射、能力收窄、价格、发现和匹配 |
| [settings/mod.rs](../src/frame/aicc/src/settings/mod.rs)、[provider/runtime.rs](../src/frame/aicc/src/provider/runtime.rs)、[service/settings_runtime.rs](../src/frame/aicc/src/service/settings_runtime.rs) | 有效来源、inventory 收敛、reload 与 registry 发布 |
| [service/tests.rs](../src/frame/aicc/src/service/tests.rs)、[model/llm_tests.rs](../src/frame/aicc/src/model/llm_tests.rs)、[provider/builtin/upgrade_tests.rs](../src/frame/aicc/src/provider/builtin/upgrade_tests.rs) | 真实 builtin catalog、单模型 inventory、规格约束、渠道 effort 和价格来源回归 |
| [验收入口](../test/aicc_test/acceptance/README.md)、[package.json](../test/aicc_test/package.json) | 现有 T1 路由、T1.5 协议 Mock、T2 真实 Provider 验收 |
| [Metadata Schema](../doc/aicc/driver_metadata_schema.md)、[冻结目录契约](../doc/aicc/frozen_model_driver_and_logical_model_fs.md) | 需同步的契约；与源码冲突时记录差异，例如版本偏好现在由显式 weight 决定，不能照旧文档恢复名称解析排序 |
| [现有模型核对记录](../doc/aicc/driver_metadata_review_20261102.md)、[云更新任务](aicc-driver-metadata-cloud-update-todo.md) | 历史线索和范围边界，不能直接当作当前缺陷列表 |

## 3. 执行 TODO

### P0：复核当前实现，建立修改前基线

- [x] 阅读当前 `AGENTS.md`、AICC 目录文档和上述源码，记录执行日期、提交号及已有相关改动。
- [x] 从现有 builtin metadata 和实际支持的渠道出发，列出本轮主流模型范围。至少核对 GPT/OpenAI、Claude/Anthropic、Gemini；对仓库已有的 Qwen、GLM、DeepSeek、Kimi、MiniMax、Doubao、Grok 等逐项标记已检查、需要更新或本轮未覆盖及原因。
- [x] 用生产 catalog/registry 路径生成“单模型 × 用途”覆盖结果，复现 §2.1。逐项区分目录漏配、能力不满足、effort 未映射、inventory 缺失、Provider 禁用、用户策略限制。
- [x] 检查已有 overlay、`routing_commands` 和 `direct_only` 语义；验证默认修复不会取消用户的禁用、显式 strict 或指定 exact/family 的选择。
- [x] 在实施记录写清最终采用的用途补全与 fallback 规则、修改入口和回归范围，然后继续实施。普通实现选择由 agent 根据证据完成，不把已经明确的目标重新变成待立项事项。

### P1：补齐默认用途覆盖

- [x] 保留用途的优选规格和相对偏好，通过已有合法规格引用、低优先级候补或显式 fallback 补齐覆盖。优先采用改动最少的方案，避免维护一份与现有模型归档重复的清单。
- [x] 满足单模型接入：无论唯一通用模型属于便宜档还是高档，只要满足某用途的硬约束，就能被该用途选择；不能要求同时接入该厂商的 nano/haiku/flash-lite 等型号。
- [x] 新增通用型号归入已有规格后即可继承覆盖；新增规格时，现有校验或新增覆盖回归必须暴露遗漏，避免每轮依赖人工发现。
- [x] 保留真实能力门槛：当前 plan/code 需要 tool_call、json_schema、32K context，summarize 需要 16K context，vision 需要视觉及 32K context；渠道限制和请求更高要求继续生效。
- [x] 不绕过 `direct_only`、未知型号拒绝、禁用状态、缺失 effort 或实验模型策略；不把非 LLM 专用模型纳入通用兜底。
- [x] 同步 `model_defaults.rs` 中的目录说明、相关契约和原有测试。将“默认无 fallback”断言改为新默认行为时，保留无合格候选和能力拒绝测试。

### P2：建立可维护的廉价快速 fallback

- [x] 输出一张运营选型表：系列、规格/具体模型、可执行 effort、适用渠道、费用口径、速度依据、备选顺序、来源和核对日期。每个纳入本轮的主流系列都必须有结论。
- [x] 选择实际可用的低成本、低延迟档位填充 `llm.fallback`；便宜档不在当前 inventory 时，能继续选择现有合格档位。最低 effort 必须是原厂支持且该渠道已映射的预设，不能统一伪造 `none`。
- [x] 保留规格引用的固定 effort 语义。低档预设通过合法的 `spec` / `additional_specs` 绑定或经校验的明确机制表达，不能扫描 `supported_efforts` 后直接挂入用途，也不能为降低 fallback 成本而改写所有用途共用的 spec effort 或 family `default_effort`。
- [x] 费用比较注明渠道、币种、输入/输出及缓存条件；不能将缺失价格当零，不能把原厂报价当成 SN/OpenRouter 等渠道的实际报价。低 effort 不自动等于更低单价，速度也不能只由型号后缀推断。
- [x] 明确直接调用 `llm.fallback` 与用途节点进入 fallback 的排序规则。结合当前“保留原任务 scheduler”的实现验证选择结果；不能只改一个 profile 名字。
- [x] 明确无候选时的显式回退链、终止条件和 trace。保留 exact/family 直选的 strict 语义、用户关闭 fallback 的能力，以及原请求全部约束。
- [x] 分别验证目录回退和运行期 failover。若本轮不改执行失败后的候选池行为，明确记录现有边界，不能宣称上游任意错误都会转到 `llm.fallback`。

### P3：完成本轮模型与渠道 metadata 更新

- [x] 在执行日查阅原厂官方模型目录、API 文档、release notes、定价及 deprecation 页面；涉及渠道差异时再查对应渠道资料。记录准确 URL、核对日期、已知/新增/变更/下线/渠道未开放结论。本文不预设“最新型号”。
- [x] 核对 canonical ID、别名和版本、稳定/实验状态、上下文与输出上限、输入模态、tools、结构化输出、支持及默认 effort。未知信息保持未知，不通过通配规则给未来型号自动授予能力。
- [x] 将原厂技术事实写入 Model Driver；将渠道别名、operation、effort lowering、能力收窄和 `model_pricing` 放在 Provider Rules；仅在 endpoint/adapter/discovery 配置确有变化时调整 Known Provider。
- [x] 检查精确规则与通配规则优先级，新增价格不能意外遮蔽技术映射。复用现有 `source_url`、`verified_at` 等价格来源字段。
- [x] 对新增型号完成“发现/静态库存 → 原厂身份匹配 → specs/efforts → 用途/fallback → 实际 wire”检查，审阅 `unmatched_models` 和渠道不支持的预设；仅能精确调用或暂不可用的情况必须有理由。
- [x] 区分原厂停供、渠道下架、停止默认推荐与删除技术定义；其他渠道或本地部署仍提供的模型不能被全局误删。退役规格前清理悬空引用并验证用户 overlay 的行为。
- [x] 按当前约定更新受影响 catalog 的 `revision_seq` 和 builtin revision；验证 factory/system/user/session 覆盖、整文档替换及无效更新保留旧快照。
- [ ] 回读有效 metadata 来源、inventory 和逻辑树。发现 local/system-config 遮蔽更新时，记录具体来源与必要调整，不直接清空用户配置。（未部署，未回读运行中的 Zone；见 §6）

### P4：固化重复运营步骤与回写

- [x] 在 `doc/aicc/maintenance/` 留下可重复执行的维护说明并加入现有索引；优先复用已有工具，只补确有缺口的覆盖检查。
- [x] 将首次修复和后续每轮工作分开：后续沿用“核对官方变化 → 对照 catalog/渠道 → 更新选型和配置 → 验证 → 回读生效 → 留记录”，无变更也记录已检查范围。
- [x] 每轮记录新增/修改/退役项目、用途覆盖变化、fallback 选型依据、有效配置来源与 revision、验证结果、未验证渠道及回退办法。厂商发布、调价、能力变化或下线时追加维护；本文不创建定时任务。
- [x] 检查消费方：Jarvis 默认 `llm.chat`、behavior 的 `llm.plan`/`llm.code`、上下文压缩的 `llm.summarize`。相关行为必须仍可工作。
- [x] 按仓库规则检查 `src/apps/jarvis_agent/agent/cmd_skills/xllm/SKILL.md` 和 `opendan_agent_dev/SKILL.md`；涉及媒体能力时再检查 `aigc_*`。在交付说明列出修改或无需修改的依据。

## 4. 验收矩阵

复用 `service/tests.rs` 的 `builtin_tree()`、真实 builtin catalog 和 `model/llm_tests.rs` 的 inventory helper。新增测试走生产 parser、registry、router；不要复制一套选择算法作为预期。官方能力与协议预期保留独立依据。

| 编号 | 场景 | 必须满足 |
| --- | --- | --- |
| A1 | 零 Provider / 零 inventory | 用途和规格可查询，动态家族/候选为空；不虚构实例，不产生环或 panic |
| A2 | §2.1 的三个模型分别单独接入 | 修复前复现缺口；修复后所有满足能力约束的用途均有真实可执行候选 |
| A3 | 本轮支持的主流通用模型逐个单独接入 | 表驱动覆盖全部当前用途；覆盖便宜档和高档，不以多模型混合 inventory 掩盖单模型缺口 |
| A4 | 缺少 vision/tools/JSON/context 或渠道 effort | 拒绝或报告明确原因；原用途约束在 fallback 后仍生效，不伪造能力或预设 |
| A5 | 同系列便宜档与高档同时存在、便宜档缺失、全部缺失 | fallback 按已记录策略选择；便宜档缺失时仍能使用现存合格档位；全部缺失时正确终止 |
| A6 | 多系列、不同渠道价格、价格未知 | 选型可解释；未知费用不按零处理；不宣称已测得最快 |
| A7 | 用户 overlay、strict、禁用 fallback、exact/family 直选、direct_only | 默认修复不绕过显式选择和隔离；无非法跨家族或回根行为 |
| A8 | 删除/禁用 Provider、同模型由多个 Provider 提供、重建与 reload | 动态内容正确增删；静态规格保留；没有重复权重、悬空边或半成品快照 |
| A9 | 有效/无效 metadata 更新及来源覆盖 | 真实生效来源可回读；整文件优先级正确；无效更新不发布部分目录；重复维护结果稳定 |
| A10 | 新型号/新 effort 的协议与消费方 | T1 验证路由，T1.5 验证实际 adapter wire；按环境完成最小 T2 和 Agent 用途验证，未执行项明确标记 |

当前可用的离线验证命令：

```bash
# buckyos/src/，先按修改模块定向验证，完成后运行 AICC 单元测试
cargo test -p aicc --lib -- --test-threads=1

# buckyos/test/aicc_test/，静态验收契约与 runner 自测
pnpm run acceptance:preflight
pnpm run acceptance:self-test
```

`preflight` 检查独立契约完整性，不读取实现 metadata，不能替代 Rust catalog/registry 测试。按改动同步 `provider_protocol_contracts.json`、`provider_capability_baseline.json`、`logical_entrypoint_baseline.json`、`route_exposure_contracts.json` 中受影响部分；不要从产品 metadata 自动生成测试期望。

T1/T1.5/T2 的环境准备、运行选择与清理按[现有验收 README](../test/aicc_test/acceptance/README.md)及仓库 AICC 验收手册执行。真实上游不可用时保留未验证项，不把 Mock 通过或 skipped 算成线上通过。builtin 变化需构建并按实际部署流程生效；完整构建、workspace 测试及 DV 按影响范围执行并如实记录。

## 5. 完成交付

- [x] 本文 Review 已按执行时源码复核，实施差异与原因已回写。
- [x] 单主流通用模型覆盖矩阵、各系列 fallback 选型表、官方模型核对表齐全。
- [x] 实现、metadata、相关独立验收契约、维护说明及受影响手册已同步。
- [ ] 提供实际验证命令、结果、配置生效证据和未验证项；没有将静态推导写成运行结论。（命令与结果已提供；部署后的配置生效证据未取得，见 §6）
- [x] 后续执行者能按维护说明重复完成一次更新，无须重新猜测目录规则、价格来源或生效方式。

## 6. 实施记录

2026-11-16，基线 `af0c007f9`。完整记录（Review 补充、覆盖矩阵、fallback 选型表、官方核对表、配置变更、验证）见 [driver_metadata_review_20261116.md](../doc/aicc/driver_metadata_review_20261116.md)；后续轮次按 [LLM 模型目录例行维护](../doc/aicc/maintenance/llm_catalog_routine_maintenance.md) 执行。

- Review 补充：§2.1 的缺口是普遍现象，修改前 113 个非 `direct_only` 内置 LLM 中有 110 个单独接入时至少一个用途纯目录不可达；缺口全部来自规格引用。`model_defaults.rs` 注释树漏写若干已有条目，已同步并把一致性测试改为双向。
- 最终规则：七个用途显式 `target_logical` 到 `llm.fallback`；`llm.fallback` 引用全部 38 个非 `direct_only` 规格，按 4.0 廉价快速 / 3.0 性价比 / 2.0 标准 / 1.0 高档与特化分层，自身 `disabled`、直接请求 `cost_first`，从用途回退沿用原用途 scheduler 与能力门槛。plan/code 加低优先级 `claude_sonnet`，vision 加低优先级 `gemini_flash`。规格 effort 与 `default_effort` 未改；运行期 failover 行为未改（只在本轮候选池内）。
- 结果：修改后单模型 × 用途不可达为 0，剩余空结果都是带缺失项的能力拒绝。
- 配置变更：新增 claude-fable-5-2、gemini-4-carbon、gemini-4-hydrogen（原生渠道）；补齐 gpt-6-sol、gpt-6-luna 价格；claude-haiku-4-5 结构化输出；相关 catalog `revision_seq` 与 `BUILTIN_CATALOG_REVISION_SEQ`（22 → 23）递增；同步 `provider_protocol_contracts.json` 与 `xllm` 手册。
- 验证：`cargo test -p aicc --lib` 789 通过、11 失败（均为 `cloud_update` / `storage` 在本机沙箱的临时目录与 SQLite 权限问题，与本轮无关）；acceptance preflight 通过、self-test 162/162（需临时把需求文档转为 LF）。
- 剩余：未构建部署、未回读运行中 Zone 的有效 metadata 来源与目录、未跑 T1/T1.5/T2；Qwen、DeepSeek、Kimi、MiniMax、Doubao 需要更新但本轮未改；2026-11-30、2026-12-01、2027-01-01 有已知价格/下线节点需复核。
