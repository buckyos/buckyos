# Model Driver v2 与逻辑树实现记录

日期：2026-09-25。对应 [任务单](../../notepads/aicc-model-driver-logical-tree-todo.md)。

本次实现 `功能 -> 厂商规格 -> 家族:effort -> 物理 instance`。全部 11 份 builtin Model Driver 使用 v2；OpenAI 保持已审阅的原文件，其余 10 份迁移。仅加载真实厂商 JSON 即可完成 catalog 校验、完整 builtin 树和目录查询，Provider Rules、Known Providers 与库存均可为空。

## 主要入口与旧职责替换

以下源码路径相对于 `src/frame/aicc/src/`。

| 原职责/删除位置 | 当前实现 |
| --- | --- |
| `catalog/schema.rs` 的 Model Driver `ModelVariant`、版本规则结构、`model_pricing`/resolver pricing 字段 | typed `ModelSpec`、`LlmSemantics`、`Effort`、`ModelStability` 和派生 `LlmModel`；原字段直接拒绝。 |
| `catalog/mod.rs` 的 Model Driver pricing/variant/version 编译索引、`effective_model_variants`、Model Driver variant 匹配 | catalog 只编译官方模型事实和规格；有限 ID pattern 复用 matcher，exact 优先、pattern 首个命中。Provider Rules 原有独立编译器保留。 |
| `catalog/validation.rs` 的 variant、version、Model Driver 价格校验 | 零库存校验规格归属、effort、名称、冲突和禁止旧字段；Provider 价格校验继续使用共享 Pricing 类型。 |
| `provider/inventory.rs` 的 `apply_version_rules`、current winner、`version_rank`、版本挂点及 `auto_mounts` 展开 | `model/mod.rs` 从官方 ID 派生版本，在规格内部排序；inventory 不再承担树编排。 |
| `provider/inventory.rs` 的 Model Driver 默认价格和 variant fallback | 仅消费已有 Provider Rules 变体及渠道价格/discovery；未知价格保持空值。 |
| `call/mod.rs` 的 Model Driver variant 参数兜底及对应错误分支 | 只查现有 Provider Rules；无映射明确返回 `MissingProviderVariant`。 |
| `model/mod.rs` 旧 LLM metadata mount/Auto/Hybrid 自动吸入和隐式父级回退 | `register_specs` 在库存前建静态规格，`materialize_families` 按 metadata/库存交集填充；`validate_llm_tree` 约束准入和隔离。非 LLM mount/Auto/Hybrid 保留。 |
| `service/mod.rs` 依赖 exact 库存判断目录可见的过滤 | 空规格也显示，`:effort` 作为家族选择器显示引用，不制造 `.high` 子目录。 |
| `service/model_defaults.rs` 旧功能表、别名、Hybrid 模式 | 落实头部厂商规格权重；LLM Manual/strict；删除 `llm.summary/reason/long`；非 LLM 服务入口显式引用规格并检查 API。 |
| `routing/mod.rs` 将所有 exact 候选按通用评分比较 | LLM 先比较规格权重/规格 ID，再比较规格内稳定性/版本/家族 ID，最后比较同家族实例；fallback 保留任务和请求约束。 |

版本排序使用补零的数字元组，支持 `5.6 < 5.10 < 6`，未知版本排后，同版本以家族 ID 决胜。目录 ID 规范化不改变真实调用 ID。`native` 使用 base；其他 effort 与库存已提供的 `reasoning-{effort}` 取交集，不生成渠道能力。多个实例只增加家族叶子，不重复规格引用。

规格、家族、任务名称冲突在零库存时仍失败。每个规格须有功能引用或 `direct_only`；任务/fallback 不能通过家族或 base exact 绕过隔离。LLM root 仅作命名空间，禁止 Parent fallback 和 Auto/Hybrid。显式 fallback 与 item 图共同检查环。

## Metadata 迁移与具体缺口

删除宽泛 LLM pattern 对未来型号的自动准入；已有确定 ID 使用 exact 条目。未归规格的独立图片、视频、音频、embedding/rerank 模型保留非 LLM 挂点；MiniMax 非 LLM 原版本挂点改为明确挂点。GLM OCR 只保留专用 API，不参与 LLM 规格竞争。

以下缺口是显式空规格或不可执行的预设，没有用 Provider 改造补齐：

- `doubao-code/mini/pro` 以及 `kimi-code/code-highspeed`：原文件只有宽泛匹配，没有足够的具体官方 ID/能力事实；规格存在，等待补充可核验的精确模型条目。`kimi-code-highspeed` 保持 direct_only。
- Qwen 3.5/3.6/3.7 与 Kimi K2.5/K2.6 的 thinking 模式使用 `thinking` 身份，现有 Provider Rules 仍主要以 `reasoning-medium` 表达开关，因此这些 thinking 预设不会自动得到可执行候选。GLM 多个开关式模型存在同样缺口。
- Haiku 4.5 的 none/thinking、Gemini 3.1 Flash-Lite 的四档和 Gemini 2.5 Flash 的 none/thinking 缺少对应的现成渠道映射；不以 base 冒充所需 effort。
- Gemini 的现有 `reasoning-mini` 渠道命名不等于新 `reasoning-minimal`，未做兼容重命名。其部分 medium 模板实际传 high，需在后续 Provider 接入中核验。
- Qwen/DeepSeek 现有宽泛渠道参数表可能存在别名折算或型号差异；本任务只约束语义身份和库存交集，不宣称最终 wire 参数准确。
- Doubao Lite 只声明旧配置已明确表达的 none；thinking budget/其他档位仍缺少本次核验依据，没有新增档位。

迁移额外核对的官方资料（2026-09-25）如下；AICC 的规格归属和固定 effort 选择是路由配置，不等同于厂商默认值：

- Qwen 托管 3.8 的独立强度为 low/medium/xhigh，其他标准值存在映射；此前系列以 thinking 开关/预算为主。[Qwen API](https://docs.qwencloud.com/api-reference/chat/openai-chat)
- Qwen3.8-2.4T-A95B 权重模型强制思考，只声明 low/medium/xhigh；27B 则可关闭。3.5/3.6 权重版以 none/thinking 表达开关，不按托管模板补档。[2.4T model card](https://huggingface.co/Qwen/Qwen3.8-2.4T-A95B)、[27B model card](https://huggingface.co/Qwen/Qwen3.8-27B)、[3.6 model card](https://huggingface.co/Qwen/Qwen3.6-35B-A3B)、[3.5 model card](https://huggingface.co/Qwen/Qwen3.5-397B-A17B)
- DeepSeek V4-Pro 当前参数区分 none/low/high/max；不把兼容别名 minimal/medium/xhigh 当作独立事实。旧 V4-Flash 采用已确认的 none/high/max，未将浮动别名自动改归新版。[DeepSeek API](https://api-docs.deepseek.com/api/create-chat-completion/)、[V4 model card](https://fe-static.deepseek.com/chat/transparency/deepseek-V4-model-card-EN.pdf)
- Haiku 4.5 支持 extended thinking；因此使用 none/thinking，不能因旧表漏配就认定为不可调 native。[Anthropic 文档](https://platform.claude.com/docs/en/build-with-claude/extended-thinking)
- Gemini 3.1 Flash-Lite 支持 minimal/low/medium/high；Gemini 2.5 使用 thinking budget，不支持 thinkingLevel，故采用 none/thinking。[Google 文档](https://ai.google.dev/gemini-api/docs/generate-content/thinking)

## 离线验收对应

主要 fixture 位于 [model/llm_tests.rs](../../src/frame/aicc/src/model/llm_tests.rs)。它直接读仓库真实 JSON，通过生产 parser/compiler/registry 构建，明确使用空 `provider_rules`、空 `known_providers`，不创建 Provider、credentials 或网络客户端。动态测试仅手工构造已有 `ProviderInventory/InventoryModel`。

| 验收项 | 覆盖位置/断言 |
| --- | --- |
| Z01/Z02 | `z01_z02_real_openai_catalog_and_empty_specifications_need_no_provider`：真实 OpenAI、六规格、权重、空家族/实例、catalog facts。 |
| Z03/Z04/Z05 | `service/tests.rs` 完整 builtin 零库存树、目录和定义输出；空规格/任务/root 无 exact；`service_assembler_builds_with_only_builtin_model_metadata`。 |
| Z06/Z08 | `z06_z08_invalid_catalogs_fail_without_inventory`：重复/缺失规格、无归属、无效 effort、冲突；旧 schema、参数表、价格、手写版本均拒绝。有限 pattern 测试还检查 resolver 序列化不存在价格。 |
| Z07 | `z07_specs_require_explicit_admission_and_direct_only_cannot_be_bypassed`、`direct_only_exact_fallback_cannot_bypass_a_missing_effort`：引用、隔离、任务重名和循环。 |
| Z09 | 完整 builtin 模型/厂商倒序、重复建树；版本排序测试倒序库存；原子更新测试拒绝仍被任务引用的规格改名。 |
| Z10 | `model_metadata_sources_replace_whole_documents_and_publish_trees_atomically`：builtin/cloud/local/system 整文档优先、无跨来源 merge，catalog 或 registry 更新失败保留相同旧 Arc。 |
| D01/D02/D03/D10 | `d01_d02_d03_d10_families_share_instances_and_disappear_only_after_last_inventory`：真实模型、不同渠道 ID、固定 selector、同家族多实例、逐个移除、旧路径无 alias。 |
| D04 | `d04_versions_are_numeric_ignore_dates_sizes_and_tie_break_by_family`；catalog 测试覆盖 Claude 两种官方版本命名。 |
| D05/D06 | routing 的 `llm_spec_preference_precedes_versions_and_preserves_instance_scheduling`、`llm_stability_is_filtered_before_ranking_and_fallback_keeps_requirements`：高权重空规格、能力筛选、旧稳定版、实验策略、跨规格版本隔离与实例耗尽。 |
| D07/D08 | `d07_d08_efforts_never_fabricate_channel_support_or_bypass_task_membership`、`d08_native_uses_base_and_thinking_is_not_medium`：旧 mount/Auto/Hybrid/exact 绕过失败，spec/default 不同，缺少变体无候选，native 和 thinking。 |
| D09 | routing 能力不足 fallback 负例和 `d09_explicit_fallback_chain_retains_each_task_requirement`；LLM 无隐式父级回退。 |
| 非 LLM | `non_llm_tasks_use_explicit_spec_links_and_require_the_requested_api` 及既有 model/provider/service 回归：明确引用、API 过滤、独立非 LLM 挂点。 |

Z10 使用真实 `RuntimeState` 发布/捕获流程和 `ServiceModelAssembler`，测试 backend 仅传递 metadata，并提供 Runtime API 要求的空 `RuntimeProviderRegistry` 容器；没有构造业务 Provider registry、Provider 对象、数据库或外部服务。主零 Provider fixture 不依赖此 runtime 容器。

定向测试先执行 catalog（16 项）、model（22 项；随后新增隔离/fallback 用例计入全量）、service（44 项）。最终执行 `cargo test -p aicc --offline`：**473 passed，0 failed，0 ignored**；`cargo check -p aicc --all-targets --offline` 通过。受影响 Rust 文件执行 rustfmt 检查，`git diff --check` 通过。使用本机缓存依赖，无 API Key/真实 Provider/付费调用；现有 dead-code warning 未扩展为无关清理。未运行整仓 BuckyOS 构建或真实厂商调用。

## 范围审查与后续接入

原任务 §2 与删除旧接口的要求存在调用依赖冲突。用户明确授权：“可以修改调用方，但只需要编译通过即可”。据此修改 `provider/inventory.rs`、`provider/mod.rs` 和 `call/mod.rs` 的必要依赖；未增加新 Provider 架构、参数转换或价格逻辑。具体删除见上表。

`provider/builtin/{claude,gemini,glm,mod,openai,openai_responses_compatible,registry,sn}.rs` **仅修改 cfg(test) 内容**：更新 v2 fixture、已废弃 metadata 格式断言和库存 golden 摘要。生产部分逐文件比对未变化。`provider/tests.rs` 更新 fixture，旧 version mount 测试职责由 D 系列承接；Provider 请求、discovery、映射与价格回归仍运行。`settings/mod.rs` 也只有 fixture 修改。

`driver_metadata/providers/`、`driver_metadata/known-providers/`、`src/protocol/`、认证、Provider 配置、公共 RPC/SDK、Cargo 依赖均无修改。复用现有 RuntimeState 原子装配和 metadata 来源优先级，未改变存储协议。

仍需后续工作：

- Adapter/Provider 的完整 effort 转换、真实渠道能力核验、最终请求参数锁定及厂商 API 回归。本次内部 `RoutingRequest.allow_experimental` 默认 false，未新增公共策略字段。
- 旧持久 inventory 的 pricing source 枚举/历史缓存清理。`provider::PricingSource::ModelDriver` 和 `service/inference.rs` 对历史来源的映射仍属于既有存储/计费消费者；新 InventoryBuilder 不再产出该来源。没有保留 Model Driver resolver 空价格字段或兜底表。
- Shared `Pricing`/`ModelPricingRule`/`CompiledPricingTable`、Provider variant compiler 仍被 Provider Rules 实际使用；非 LLM mount/Auto/Hybrid/Parent helper 仍服务独立非 LLM，不是旧 LLM 备用路径。

## 生产代码量

以本次改动前 HEAD 为基准，对受影响 Rust 文件分离内联 cfg(test) 模块和独立测试文件；生产统计排除空行与整行注释，使用相同行差分口径。新 `model/llm_tests.rs` 全部计入测试。数据与文档不抵消生产增长。

| 生产文件 | 新增 | 删除 |
| --- | ---: | ---: |
| `call/mod.rs` | 17 | 39 |
| `catalog/mod.rs` | 111 | 151 |
| `catalog/schema.rs` | 123 | 51 |
| `catalog/validation.rs` | 144 | 90 |
| `error/mod.rs` | 1 | 8 |
| `model/mod.rs` | 402 | 30 |
| `provider/inventory.rs` | 15 | 191 |
| `provider/mod.rs` | 1 | 3 |
| `routing/mod.rs` | 66 | 6 |
| `service/mod.rs` | 4 | 34 |
| `service/model_defaults.rs` | 281 | 175 |
| 合计 | **1165** | **778** |

生产净增加 **387 行**。测试物理行新增 1392、删除 918（含替换旧断言与新增完整 fixture）。
Model metadata 新增 1124、删除 340 行；文档变化单独保留在 `doc/aicc/` 和任务单，不计入上述生产代码。

生产代码未达到净减少目标。增长主要来自以前没有的零库存图校验、direct_only 隔离、动态家族/selector、分层排序和非 LLM 的显式规格引用表；旧 Provider 版本展开、Model Driver variant/价格路径已实际删除，没有双轨流程。继续减少 Provider 价格来源/缓存及统一 lowering 的复杂度需要后续接入任务，不能为了行数改动本次边界外消费者。当前变化的取舍是用显式校验替代原先缺失的约束，而非声称代码量已经简化到净减。

文档与任务单新增 357、删除 574 行（含本报告）；独立于生产代码和 metadata 统计。
