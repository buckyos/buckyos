# AICC `decision` API Type 支持 TODO

日期：2026-09-25。状态：实现与离线验收完成；Jev 真实链路未验证（缺少可用测试配置/凭据）。

本 TODO 基于本地 HEAD `d05e1fb1304b58accb4bcf959497a774e998386b` 核对。目标是让后续 codeagent 在当前 AICC 架构中完整支持新的 **API Type `decision`**，以 TypeSafe Jev 为首个实际接入对象，为其他结构化决策模型保留相同能力接口。本次已完成下列实现与离线工作；真实验证和既有测试问题见第 8 节。

## 1. 开工前重新核对基线

- [x] 先读仓库 `AGENTS.md`，检查工作区和最新 AICC 提交。以下提交是定位线索，不是要求回退或固定代码版本；若执行时已有后续变更，基于最新实现更新任务判断。

| 提交 | 已发生的变化 | 对本任务的约束 |
| --- | --- | --- |
| `b8e294c7` | Model Driver v2、LLM 规格与家族树 | 沿用 v2 metadata；独立非 LLM 模型不填写 `llm` 规格/effort 字段。 |
| `5f548c37`、`6e6fc7e6` | Provider 身份、显式库存、渠道预设、计费及 discovery 修复 | 模型事实、渠道库存、协议和价格分层处理；不能靠新增模型 metadata 就制造可执行库存。 |
| `7ca26ac7`、`cf5662fb` | 模型目录与管理 UI 更新 | 检查模型展示、类型过滤和目录数据映射。 |
| `42f73c7b` | 路由按各层可用 item 权重展开 | 复用现有通用展开与调度，不新增决策模型专用排序算法。 |
| `d05e1fb1` | 路由 UI、`routing.preview`、`events.list` 和 trace 更新 | 新 API 类型必须贯通预览、事件和用量展示。 |

- [x] 阅读 [AICC 文档入口](../doc/aicc/README.md)、四份 `frozen_*.md`、[Model Driver schema](../doc/aicc/driver_metadata_schema.md)、[Provider schema](../doc/aicc/provider_profile_schema.md)、[路由设计](../doc/aicc/aicc_router.md) 和 [operation 绑定表](../doc/aicc/provider_operation_bindings.md)。
- [x] 以当前代码确定“已实现”行为。`model_driver_v2_implementation.md` 的初始报告之后还有 Provider/routing 改造；`aicc_router.md` 部分旧段落仍描述版本推导、unknown model conservative fallback，与最新 schema/代码不一致。不要重新引入这些已移除路径；修改所涉及的文档时同步消除对应矛盾。

## 2. 范围与接口语义

**固定目标：`ApiType::Decision` 序列化为 `decision`，并增加粗粒度 `Capability::Decision`。** API Type、RPC method、逻辑目录分别定义，不能混用。建议 typed RPC method 为 `decision.evaluate`，沿用现有“`route.resolve` 选择 exact model，再执行 typed inference”的调用方式；首版不要求另加 Helper。

`decision` 的语义是：给定共享状态和调用方定义的问题/候选规则，返回有类型的判断结果及概率。它不要求某种模型架构，也不保证判断正确。首版覆盖文本与 JSON 状态，支持一批问题混合求值，不包含自由文本生成、解释生成、任意 JSON 抽取或动作执行。

| 问题 | 必须表达的输入 | 必须保留的结果语义 |
| --- | --- | --- |
| Choice | 判断指令、候选 ID 与各自的说明 | 被选候选、各候选概率；不得返回候选集外的值。 |
| Score | 判断指令、有序等级及说明 | 数值评分、等级概率及等级对应关系；明确评分是否为等级索引的期望值，不能混同百分比。 |
| Yes/No（Jev 称 Noul） | 判断指令，可选的真/假判定标准 | 为真的概率；不能只返回经阈值处理的布尔值而丢失概率。 |

- [x] 先冻结强类型 request/response schema 及示例，再接实现。问题/答案使用带类型标签的 enum 和稳定问题 ID；共享 `state` 接受 string/object/array，指令与规则应保留上游支持的结构化表达。公共字段命名由协议设计确定，Jev 专有字段的转换放在 Adapter；只保留一套规范字段，不增加同义别名。
- [x] 一次请求中的问题共享同一状态、独立求值，不引用同批其他答案；有依赖的判断由调用方分轮组织。请求保持批量结构，不在公共层无条件拆成每题一次远端调用。
- [x] `confidence` 与答案概率分开保存；上游没有提供时保持缺失，不以最大概率补造。记录不同实现的 confidence/校准语义，不承诺跨模型数值可直接比较，也不把“符合输出类型”写成“零判断错误”。
- [x] 校验非空问题集、合法问题 ID、候选唯一性、合法等级、输入大小和模型限制。响应校验问题 ID/类型匹配、候选或等级覆盖、有限数值、概率范围及概率和的容差。缺题、额外题、非法分布和越界评分应明确失败，不能作为完整成功结果；保留原始答案，不静默补齐或归一化坏数据。
- [x] 复用 typed request/response 的 `exact_model`、执行模式、幂等、task/session、usage/cost、trace/error 等公共字段。首版 Adapter 仅声明实际支持的执行模式；不支持 stream 时显式拒绝，不伪造文本 token 流。

首版完成范围是 **公共契约 + 路由/执行链 + Jev Provider/Adapter + SDK/管理面联动 + 验收**。Kev 等仅用于检验契约是否过度绑定厂商，不要求本任务接入所有同类模型；通用 LLM 模拟决策、训练模型和专用决策编辑 UI 均不在首版范围。

## 3. 公共协议与执行链

- [x] 在 [aicc_client.rs](../src/kernel/buckyos-api/src/aicc_client.rs) 补全 method 常量、`ApiType`/`Capability` 映射、typed DTO、`AiccCall`、Rust client、handler、dispatcher 和所有相关穷举分支。检查直接调用与 `AiccCall::from_method_and_params` 两条入口均执行必要校验。
- [x] 贯通 `src/frame/aicc/src/` 下的 `api/`、`service/inference.rs`、`call/`、`canonical/`、`protocol/`、`execution/`，复用现有 lowering、执行、错误、任务、幂等和 usage 入账机制。`ProtocolOutput.value` 能装 JSON 不等于 typed decision 契约已完成，返回公共响应前须验证业务结果。
- [x] 以现有非 LLM typed method 为实现参照，检查可覆盖参数列表、API 类型字符串、公共响应封装、错误转换等映射。不能将 decision 结果塞入 `AiMessage`、`tool_calls` 或 `rerank.results` 来绕过新接口。
- [x] 校验鉴权、租户隔离、超时、重试、取消和 runtime snapshot 语义仍由现有公共链路承担，避免旁路直连厂商或新增独立任务/计费系统。

## 4. Model Driver、Provider 与路由

- [x] 重新核验 TypeSafe 当前官方 API、实际模型 ID、别名、认证、模型列表/静态库存、限制和价格，记录来源与日期。添加原厂的 `.model.json`、渠道 `.provider.json` 和必要 Known Provider 配置；复用现有 catalog 加载与自动打包机制。
- [x] Model Driver v2 使用有限精确模型集合。`jev-latest` 等浮动别名按当前 Provider 身份解析机制绑定已确认的模型事实，不用 `jev-*` 通配注册未来型号。独立 decision 模型沿用非 LLM `logical_mounts`，不得伪装成 LLM 或套用 LLM 的规格/思考预设。
- [x] TypeSafe 的新 wire protocol 用现有 `ProtocolAdapterPlugin`、`OperationCodec` 注册机制接入，明确 `(protocol_adapter_id, operation_id, api_type=decision)` 绑定，完成认证、请求/答案映射、usage 和错误转换。模型字段映射、operation 选择、渠道限制和价格按现有职责放入 Provider Rules；不能把新接口当作 OpenAI chat/completions 兼容协议。
- [x] 同 wire 协议的自托管实现可复用 Adapter；只有已核验的协议兼容才共享 codec。Provider profile 和 model driver ID 保持开放，不因首个后端是 Jev 就写死品牌白名单。
- [x] 增加一个可调用的 `decision` 逻辑入口及必要的模型挂点，先完成最小目录结构。`decision.classify/score/route` 可按后续需求添加；三种问题类型不拆成三个 API Type。零 Provider 时入口仍可查询，推理返回明确无候选，不能伪造可用模型。
- [x] 候选必须同时满足 `api_type=decision` 和该请求实际使用的问题类型、概率输出、输入/选项数等能力限制。复用已有 capability/feature/requirement 扩展方式；部分支持的后端不得承接混合请求，不能因为模型会输出 JSON 就自动获得 decision 能力。exact 调用同样不能绕过限制。
- [x] 复用当前逐层“最高可用权重组、同权全部展开、该组无候选再尝试下一组”的规则和候选池调度。权重不沿路径相乘，不从名称猜版本；fallback/failover 始终保留原 API 与问题能力要求，不静默降级到普通 `llm`/`rerank`。
- [x] 价格只进入渠道 Provider Rules/discovery；缺少价格保持 unknown。输出免费应写明已核验的零费率，不能把真实 `output_tokens` 清零，也不能按答案 JSON 长度自行估算 token。复用当前 `AiUsage`、价格快照、成本估算、预算门槛与一次入账逻辑，检验纯输入收费场景。

## 5. SDK、工具与管理界面联动

- [x] 检查同工作区 `buckyos-websdk/src/aicc_client.ts` 及其测试，按该仓库规则同步 API 类型、问题/答案联合类型和 typed 调用。检查 `src/tools/buckyos-agent/`、`src/frame/agent_tool/src/aicc_model_tools.rs`、workflow AICC adapter 的 method/type 分派和模型查询；通用透传已覆盖的地方不另造接口。不要修改 `node_modules` 或手工编辑 SDK 构建产物。
- [x] 更新 `src/frame/desktop/src/api/aicc_mgr.ts`、`src/app/ai-center/`（相对于 desktop）中的类型、API 列表、类别统计、目录顺序、过滤器及实际使用的 mock/datamodel。当前 `normalizeApiType`、`toApiTypes` 等会将未知类型回落为 `llm`，必须保证 `decision` 不被吞掉或误分类。
- [x] 检查最新 Models/Routing 页面、`routing.preview`、`events.list`、usage/trace 展示：类型、候选、未选原因、执行模式、价格与结果状态要与后端一致。复用已有管理页面，不新增专用产品流程。
- [x] 同步四份冻结设计中受影响部分、API/逻辑模型目录/Provider 文档、schema 和 `provider_operation_bindings.md`。该绑定表受 `every_builtin_provider_operation_has_a_golden_lowering_binding` 测试校验，新增 Adapter 不能只改文档或只改代码。

## 6. 验收与完成条件

- [x] **契约测试**：三类单题、同批混合题、结构化 state/规则；序列化与反序列化；非法问题/答案、unknown field、缺题与错误类型；直接 client 和 kRPC dispatcher 均覆盖。
- [x] **Adapter 测试**：使用核验过的官方协议 fixture/mock HTTP，验证请求路径、模型名、认证、完整答案、概率/confidence、真实 usage 映射，以及认证失败、限流、超时、畸形响应；测试不依赖真实 key。
- [x] **路由测试**：无 Provider、正确库存、未知/浮动身份、只支持部分问题的后端、混合请求、exact 调用、同权/次权重组、同 API fallback/failover、未知价格与零输出费率；预览和实际调用的资格判断一致。
- [x] **生命周期与计费**：幂等重放不会重复调用/入账；失败和取消遵守现有状态机；decision usage/cost 可查询，日志/trace 不泄露凭据。
- [x] **SDK/UI 回归**：新类型不回落为 llm，目录/过滤/用量/事件/trace 可用，既有 LLM、embedding、rerank 和媒体链路不退化。零库存与含 decision 库存各验证一次。
- [ ] **真实链路**：在已配置的 Jev Provider 上完成 `route.resolve(api_type=decision)` → `decision.evaluate(exact_model=...)` 的三类混合请求，核对结果与 usage；补入当前 AICC E2E 框架。无凭据或服务不可用时完成可做的离线工作，单独记录真实验证未完成，不能把 mock 通过当成线上已验证。

最低 Rust 验证（在 `src/` 执行）：

```bash
cargo test -p buckyos-api
cargo test -p aicc
cargo check -p aicc --all-targets
```

按实际改动运行 websdk、工具及 desktop 定向测试；desktop 至少执行其 `check` 和 `build` 脚本，最后按仓库要求完成受影响构建检查。修订测试基线时保留旧能力的回归断言，不用删除用例或放宽 schema 掩盖缺口。

交付时更新本 TODO 的完成状态，记录实际基线、最终公共契约、Provider/模型支持范围、验证命令和结果，以及未完成的真实验证。Beta 2.2 不做旧协议/旧 metadata 兼容；控制改动范围，优先复用组件，新增依赖遵循仓库规则。

## 7. 外部协议参考

以下参考已于 2026-09-25 查看，执行时须重新核验动态事实；不把项目宣传的速度/准确率作为验收标准。

- [TypeSafe API](https://docs.typesafe.ai/api)：当前 `POST /v1/systemone`、state/questions/answers、模型与 usage 字段。
- [问题原语](https://docs.typesafe.ai/primitives)、[Score](https://docs.typesafe.ai/primitives/score)、[Confidence](https://docs.typesafe.ai/confidence)：候选、评分和概率语义。
- [TypeSafe 文档索引](https://docs.typesafe.ai/llms.txt)：查找执行时的模型、SDK 和限制文档。
- [Kev 项目](https://github.com/jaredpalmer/kev)：同类模型和 TypeSafe 风格接口的参考；接口相似不能证明模型能力、限制和 confidence 定义相同。


## 8. 实现与验证记录（2026-09-25）

### 实际基线与交付

基于 `d05e1fb1304b58accb4bcf959497a774e998386b` 实现，没有回退历史提交。初始 BuckyOS 工作区只有本 TODO 未跟踪；同工作区 `buckyos-websdk` 初始干净。本次未引入依赖、未部署、未推送，SDK 构建产物由构建生成并从源码交付 diff 中恢复。

- 公共契约冻结在 [decision_api.md](../doc/aicc/decision_api.md)，类型与业务校验在 [aicc_decision.rs](../src/kernel/buckyos-api/src/aicc_decision.rs)。`ApiType::Decision` / `Capability::Decision`、`decision.evaluate`、client/dispatcher、带标签问题/答案、批量混合求值均已接入。Score 是零起点等级期望，boolean 保留为真概率，confidence 缺失不补造。
- [TypeSafe Adapter](../src/frame/aicc/src/protocol/typesafe.rs) 复用插件/codec、认证、错误和执行链。有限模型为 `jev-1.13.0`；默认静态库存仅该精确版本；显式别名 `jev-latest` / `jev-preview` 绑定已核验版本，响应版本漂移失败。仅 immediate，同 wire 自托管可选择该 Adapter。
- 路由复用现有权重展开、候选池和预算；题型、结构化输入、概率及容量要求贯通 exact、逻辑目录与 preview，目录下限取集合并集/数值最大值。Model Driver 与 Provider/codec 能力求交，零库存不制造可用模型。unknown 价格与已核验零输出费率分开处理，真实 output_tokens 保留。
- 同步了 [WebSDK](../../buckyos-websdk/src/aicc_client.ts)、工具 typed 分派/能力筛选、workflow method schema、desktop 目录/过滤/统计。管理事件与 trace 沿用现有通用展示。四份冻结设计、API/目录/schema/Provider 文档及 golden operation 绑定表已更新；相关旧 unknown fallback / 从名称猜版本描述已消除。
- E2E 加入独立官方 [System One fixture](../test/aicc_test/acceptance/fixtures/typesafe-systemone.json)、严格请求 mock、官方目录解析、有限别名归并和 T2 `route.resolve -> decision.evaluate` 三题混合 cell。每个 cell 只发一批真实推理；离线 mock 不表示线上已通过。

### 已执行验证

Rust 使用 `CARGO_HOME=/tmp/dev-cache-root/cargo`、`CARGO_TARGET_DIR=/tmp/dev-cache-root/cargo-target`。以下命令按对应仓库目录执行：

| 范围 | 命令 | 实际结果 |
| --- | --- | --- |
| AICC / 公共 API（`src/`） | `cargo test -p aicc -p buckyos-api` | AICC 504 通过；公共 API 217 单元 + 4 原集成 + 5 decision 集成通过 |
| AICC 全 target（`src/`） | `cargo check -p aicc --all-targets` | 通过，保留仓库已有 warnings |
| workflow（`src/`） | `cargo test -p workflow` | 49 通过 |
| Agent 模型工具（`src/`） | `cargo test -p agent_tool aicc_model_tools` | 2 通过 |
| WebSDK | `pnpm test -- --runInBand tests/aicc_client.test.ts`；`pnpm run build` | 12 通过；构建通过 |
| Desktop | `pnpm run check`；`pnpm run build` | 类型检查、生产构建通过 |
| Desktop datamodel | `deno test --allow-read --allow-env --sloppy-imports tests/datamodel/model-catalog.test.ts tests/datamodel/aicc-routing.test.ts tests/datamodel/aicc-finance-totals.test.ts` | 8 通过，含 decision 零库存/有库存、API 过滤与路由/用量回归 |
| CLI tools | `deno test --allow-read --allow-env --allow-write --sloppy-imports --import-map=/tmp/aicc-decision-import-map.json commands/ai_provider_test.ts lib/aicc_test.ts` | 6 通过；临时映射到同工作区新 SDK 的构建输出与声明，未修改旧安装包 |
| E2E preflight | `pnpm run acceptance:preflight` | 通过：24 API Type，13 Provider；完整预检记录在 `/tmp/aicc-decision-preflight.log` |
| E2E 自测 | `pnpm run acceptance:self-test` | 76 通过，1 个既有失败，详见下文；TypeSafe 新增协议/混合概率/目录归并用例通过 |
| E2E runner 类型 | `deno check --node-modules-dir=none --sloppy-imports --import-map=/tmp/aicc-decision-import-map.json acceptance/run_gateway.ts acceptance/run_t15_gateway.ts` | 通过 |
| 受影响 release 构建（`src/`） | `uv run buckyos-build.py -s aicc workflow` | 通过，SDK 打包及 `x86_64-unknown-linux-musl` 的 AICC/workflow release 构建完成 |
| 工作区检查 | `git diff --check` | 通过 |

工具测试的临时 import map 将 `buckyos` 指向 bridge；bridge 使用 `@deno-types` 引用本地 `buckyos-websdk/dist/node.d.ts`，并导出 `dist/node.mjs`。重测时先构建本地 SDK，再创建相同映射，或通过正常构建/安装流程更新消费端 SDK；不要编辑 `node_modules`。

核心覆盖包括：三类单题与混合题、结构化 state/规则、unknown fields、重复/缺失/错类型答案、非法概率与评分；请求/响应 fixture、Bearer/路径/模型、401/422/429/504/529、usage/confidence、凭据脱敏；部分能力后端、exact 限制、同权/次权重候选、未知身份/已知别名、自托管协议复用、未知价格预算与已知零费率；decision 重试/failover、幂等与一次入账。共享链路的租户、取消、失败状态机和 snapshot 测试随 AICC 全套通过。

### 真实验证与既有问题

1. **Jev 真实链路未完成**：本地基础服务检查 `uv run src/check.py` 返回 Running，但没有找到可用的 Jev 验收配置（`test/aicc_test/` 只有 example TOML）或 TypeSafe/Jev key/token 环境变量。本次没有发送真实 Provider 推理请求，实际调用数 0、费用 0；也没有临时修改 AICC settings。后续使用配置好的 TypeSafe 实例和本地凭据，通过 `acceptance:gateway -- --config <local.toml> --provider typesafe` 生成计划，并在明确授权及调用次数/预算限制下执行一个混合 cell。
2. **既有 E2E 云更新 fixture 失败**：`cloud update fixtures replace complete catalog files and tombstone every cloud identity` 仍要求 `gpt-5.6.logical_mounts`，与当前 Model Driver v2 不一致。已用未修改的 HEAD archive 调用同一 `buildCloudUpdateFiles(42, "v1")`，复现完全相同的 `builtin OpenAI catalog has no gpt-5.6 logical mounts`。未删除用例、未放宽断言；此旧 LLM 云更新 fixture 未纳入本次 decision 修复。证据：`/tmp/aicc-decision-head-baseline.log`、`/tmp/aicc-decision-acceptance.log`。
3. **额外全量 agent_tool 检查未全绿**：`cargo test -p workflow -p agent_tool` 在 agent_tool 上为 184 通过、4 失败、2 ignored；其中 3 个 grep 测试因机器缺少 `rg`，另一个既有 local_llm_context 测试遇到 `RunBusy`，单独重跑已通过。受影响 `aicc_model_tools` 和 workflow 的定向检查均通过。未为此引入依赖或修改无关实现。证据：`/tmp/aicc-decision-rust-final.log`、`/tmp/aicc-decision-agent-flake.log`。

其余主要日志在 `/tmp/aicc-decision-rust-workflow.log`、`/tmp/aicc-decision-build-final.log`、`/tmp/aicc-decision-sdk-final.log`、`/tmp/aicc-decision-desktop-final.log`、`/tmp/aicc-decision-tools-test.log`。线上 Jev 对当前 wire、限额和价格的实际行为仍待真实验证；日期化官方来源已记录，不把 fixture 结果写成线上结论。


### 补充：DV OpenRouter Jev smoke（2026-09-25 19:38 PDT）

用户授权通过 DV 中已有 OpenRouter 实例尝试 Jev smoke。期间用户执行 `start.py --all`，初次 Gateway 不可达；恢复后已通过 Zone Gateway 的 `control-panel auth.login` 正常登录。对 `openrouter-default` 执行一次 `provider.refresh_models` 成功，库存 revision `6:2`，112 个可执行模型，但没有 Jev。`routing.preview(paths=["decision"], requirements=混合题型要求)` 返回 `available=false`；同条件 `route.resolve` 返回 `no_candidate_model`。因此 smoke 在路由阶段阻塞，未发送 `decision.evaluate`，实际 Provider 推理调用 0、费用 0；未修改 settings 或凭据。

[OpenRouter 官方 Jev 教程](https://openrouter.ai/blog/tutorials/how-to-use-jev/) 说明渠道接口是 `POST /api/alpha/decisions`，模型 `typesafe/jev-1.13`。当前本次实现只接入 TypeSafe 原厂 `/v1/systemone`；现有 `openrouter-responses` 的操作绑定没有 decision，OpenRouter 的该模型 ID 也还没有到 `typesafe/jev-1.13.0` 的显式映射。这是 OpenRouter 渠道接入缺口，不能通过已有 key 或刷新库存自动补齐。尚不能宣称 OpenRouter Jev 真实推理通过。

脱敏实测证据：`test/aicc_test/reports/openrouter-jev-smoke-inspect.json`（本地忽略的报告）。本轮没有修改产品实现或部署；下一步需补齐 OpenRouter Decisions codec/operation、身份与库存发现、渠道定价及对应官方 T1.5 fixture，再验证该混合 smoke。
