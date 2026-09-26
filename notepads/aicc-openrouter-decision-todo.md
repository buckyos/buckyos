# AICC OpenRouter Provider：接入 alpha Decisions / Jev TODO

日期：2026-09-25。状态：接入实现完成，已部署 AICC，真实 Jev 三题混合推理通过；全量 E2E self-test、T1 与 Deno 类型检查仍有既有 fixture/声明阻塞，见第 8 节。

目标：让 DV 已有的 `openrouter-default` 实例发现并调用 Jev decision 模型，复用其 OpenRouter key、`https://openrouter.ai/api/v1` base URL 和现有 AICC `decision.evaluate`，同时保持 LLM、embedding、rerank 可用。

本任务承接 [decision API TODO](aicc-decision-api-todo.md) 与 [decision 公共契约](../doc/aicc/decision_api.md)。核对基线为 HEAD `d05e1fb1304b58accb4bcf959497a774e998386b` 加当前工作区尚未提交的 decision 实现；不能假定该 HEAD 单独包含这些实现。第 1 节保留任务开始前的现状定位；最终实现、部署与验证以第 8 节为准。

## 1. 已确认的问题与边界

### 当前实现

| 环节 | 当前行为 | 需要补齐 |
| --- | --- | --- |
| 发现请求 | `provider/builtin/openrouter.rs` 已请求 `/api/v1/models?output_modalities=all` | 保留该查询，补齐返回值解释，不新增“开启 alpha”开关。 |
| API 类型 | 识别 `rerank`、`embeddings`，其余落到 `llm` | `output_modalities: ["decisions"]` 应识别为公共 API Type `decision`。 |
| 模型身份 | 普通 `vendor/model` 精确匹配，没有 Jev 渠道别名映射 | 核验渠道型号与原厂型号关系，有限、显式地映射。 |
| Provider Rules | 只有 responses、embeddings、rerank operation | 增加 decision 的渠道能力、限制、operation 及价格来源。 |
| 协议 | `openrouter-responses` 未注册 decision codec | 在同一 Adapter 中增加 alpha Decisions operation。 |
| 原厂实现 | `typesafe-systemone` 调用 `/v1/systemone` | 可复用已核验相同的题型转换；不能直接套用整个原厂响应解码。 |

`~/.buckycli/buckyos_boot.toml` 的 `"services/aicc/settings"` 是 JSON 字符串形式的启动覆盖，当前 OpenRouter 已配置 `auto_sync_models=true`。现有 settings 没有 `include_alpha` 参数。单改 TOML 无法补上类型、身份及 codec 缺口；将整个 `base_url` 改成 `/api/alpha` 还会破坏现有 URL 拼接。该文件也不是即时更新运行中 settings 的接口。

### 官方与实测事实

- [OpenRouter Jev 教程](https://openrouter.ai/blog/tutorials/how-to-use-jev/) 给出的接口是 `POST /api/alpha/decisions`，使用 OpenRouter Bearer key；请求含 `model`、共享 `state`、按 ID 索引的 `questions`，题型为 `choice`、`score`、`noul`。
- 同一教程的响应示例带有 `model: "typesafe/jev-1.13-20260917"`、`answers`、`usage.input_tokens/output_tokens/cost`、`id`、`provider`。请求模型名、响应构建名与 AICC origin model ID 不一定相同，必须分别处理。
- 只读查询 [完整模型列表](https://openrouter.ai/api/v1/models?output_modalities=all) 时，`typesafe/jev-1.13` 与 `~typesafe/jev-latest` 均返回 `text->decisions`、空 `supported_parameters`、32,000 context；`~` 是后者 ID 的一部分。`typesafe/jev-router` 返回文本 modality，是另一个模型，不能按 `jev-*` 名称授予 decision 能力。
- [Jev 模型页](https://openrouter.ai/typesafe/jev-1.13/api) 当时标明 32K context、输入 USD 0.042 / 百万 token、输出免费；模型列表对应 `prompt="0.000000042"`、`completion="0"`。这些是 OpenRouter 渠道事实，执行时重新核验。
- 2026-09-25 19:38 PDT 的 DV 检查中，`openrouter-default` healthy，刷新成功，revision `6:2`，有 112 个可执行 OpenRouter 模型但没有 Jev。混合题 requirements 的 `routing.preview` 无候选，`route.resolve` 返回 `no_candidate_model`。实际推理调用 0、费用 0。112 和 revision 仅为当时观测，不作为未来断言。

DV 脱敏报告在本地忽略文件 `test/aicc_test/reports/openrouter-jev-smoke-inspect.json`，概要已记录于前一 TODO；报告可能不随 checkout 存在。期间用户执行过 `start.py --all`，Gateway 恢复后完成上述检查，不能把短暂服务故障与 Provider 能力缺口混为一谈。

## 2. 发现、身份与库存

- [x] 开工先读 `AGENTS.md`、当前 AICC 文档和前置 decision 实现，检查工作区。保留用户已有修改，不回退基线、不重做公共 decision API。
- [x] 在 [OpenRouter discovery](../src/frame/aicc/src/provider/builtin/openrouter.rs) 明确识别 `decisions`，产出 `ApiType::Decision` 与对应 remote method；防止 decision 落到 `llm`/`responses.create`。为未知或冲突 modality 定义保守行为并保留诊断，不静默宣称支持。
- [x] 保持 `output_modalities=all`，补齐其 URL 与响应 fixture 测试。E2E 官方目录抓取也使用完整列表，避免生产发现能看到而验收基线漏掉。
- [x] 核验 `typesafe/jev-1.13` 与当前 Model Driver `typesafe` / `jev-1.13.0` 的版本关系后，添加明确映射。不能通过补 `.0`、删版本后缀、前缀匹配等规则推导身份；无法证实等价时保留未匹配原因。
- [x] 对 `~typesafe/jev-latest` 单独定义有限别名映射及版本核验，保留 wire ID。未知未来 Jev 版本、未知 dated build、歧义或别名漂移应不可执行并给出原因，不能悄悄套用旧型号事实。
- [x] 明确排除 `typesafe/jev-router` 的 decision 归类。测试普通 vendor 匹配、带 `~` 别名、未知型号以及重复身份，确保其他厂商匹配不退化。
- [x] 验证模型事实、真实发现库存、渠道规则、codec 能力求交后才形成可执行模型；仅新增 metadata 不得制造库存。刷新失败沿用现有错误及最后可用快照机制，不误清空其他可用模型。

## 3. 渠道规则、能力与计费

- [x] 更新 [openrouter.provider.json](../src/frame/aicc/driver_metadata/providers/openrouter.provider.json)，为已确认的有限 Jev ID 绑定 decision operation，建议名 `decisions.create`（最终与注册表统一）。检查规则合并后不会给 decision 模型额外开放 LLM/embedding。
- [x] 将 decision 题型、概率、结构化 state/规则等能力按已确认模型事实与渠道协议求交。空 `supported_parameters` 不等于不支持 decision，也不能据此声明 tools、JSON Schema 等 LLM 特性。
- [x] 核验 OpenRouter 的 context、题数、选项、等级及请求大小限制。当前原厂 Model Driver 的 64K context 与渠道列表的 32K 不同，渠道只能收窄有效能力；不能直接继承全部原厂上限。
- [x] 区分 token 限额和 AICC UTF-8 JSON byte 防护。复用 `question_count`、`max_options`、`max_levels`、`input_bytes`、`max_state_question_bytes` 等 requirements；byte 上限不冒充 tokenizer 结果或计费用量。
- [x] 定价来源限定为 OpenRouter discovery/渠道规则。保留已核验的零输出费率；缺失、未知和非法价格不能转成免费，也不能回退到 TypeSafe 直连价格。
- [x] 按官方 wire 映射实际 token 和 `usage.cost`，复用 OpenRouter 现有 `reported_cost` 的 USD / total_request_cost 语义及公共入账逻辑。实际 `output_tokens` 即使收费为零也应保留；不按答案字符数估算 token，不把 reported cost 与估算成本重复相加。
- [x] 对缺失 cost、缺失 usage、负数/非有限数值明确处理，保留“未报告”和“已报告为零”的区别；预算估算、实际费用、失败调用的计费状态按既有公共语义记录。

## 4. alpha Decisions 协议接入

- [x] 在现有 [openrouter_responses_adapter](../src/frame/aicc/src/protocol/chat_completions_dialects.rs) 组合注册入口添加 decision descriptor/codec，并检查 [derived 插件](../src/frame/aicc/src/protocol/plugins/derived.rs) 和导出。复用 `openrouter-responses` 实例配置；首版仅声明 `Immediate`，不伪造 stream、webhook 或远端取消能力。
- [x] 实现 operation 专属 endpoint：标准实例仍以 `/api/v1` 为 base，decision 请求去同源 `/api/alpha/decisions`，其他 operation 路径保持各自语义。沿用已有 URL 处理模式，覆盖尾斜杠及受支持代理前缀；不得硬编码官方 host、生成 `/api/v1/api/alpha/decisions`，或更换整个实例 base。
- [x] 复用现有 Credential/HTTP transport、超时、响应体大小和错误机制；Bearer 只发往解析后的预期上游。不在 codec 中引入独立直连 client 或另一个重试循环。
- [x] 对照官方协议实现 canonical 请求转换：公共 `boolean` 对应 `noul`；保留结构化 state、instructions、criteria、问题与候选 ID，以及同批独立求值语义。一次混合请求保持一次上游批量调用。
- [x] 检查 [TypeSafe codec](../src/frame/aicc/src/protocol/typesafe.rs) 可复用的题型/答案映射，仅共享经核验相同的部分。OpenRouter 的顶层 `id`、`provider` 和 `usage.cost` 在原厂严格 decoder 中会被拒绝，应有独立 wire envelope，不能全局放宽字段校验来绕过差异。
- [x] 明确请求渠道 ID、响应 dated build、AICC origin identity 三者关系。当前执行层会核对 `ProtocolOutput.model` 与 origin model；在渠道边界完成已核验身份规范化，同时保留真实响应模型用于 trace。不得直接比较三种字符串，也不得无条件覆盖响应模型以掩盖版本漂移。
- [x] 解码完整 answers，复用公共 ID/类型、候选覆盖、概率和、score 与等级期望、confidence 校验。缺题、多题、越界或无效结果应失败，不补造概率，不把 confidence 当答案概率。
- [x] 根据官方 schema 核验并覆盖认证失败、参数错误、限流、超时及上游错误；保留可用 request ID、Retry-After 等信息。错误和 trace 脱敏，不输出 key、session token 或完整账号对象。

## 5. 路由、管理面与文档

- [x] 复用 `route.resolve(api_type=decision)` → `decision.evaluate(exact_model=...)`；exact、逻辑路由与 `routing.preview` 使用相同混合题 requirements，不可由某个入口绕过容量或能力约束。
- [x] 复用当前目录挂点、逐层权重展开、候选调度、fallback/failover。所有候选必须保持 decision 与请求要求，不能降级成 LLM；别名不得在验收计划中制造重复物理模型调用。
- [x] 验证 OpenRouter decision 沿用鉴权、租户隔离、snapshot、幂等、取消和一次入账流程；不新增公共 RPC、OpenRouter 专属客户端 API 或专用任务系统。
- [ ] 在现有 Models/Routing/Usage/Events 页面核对 API 类型、实例、origin/channel ID、价格、候选与错误原因。SDK/UI 已有 decision 支持，优先验证通用映射，只修改实际缺口。
- [x] 同步 [operation 绑定表](../doc/aicc/provider_operation_bindings.md)、[Provider 冻结设计](../doc/aicc/frozen_provider_implementation.md)、[Provider schema](../doc/aicc/provider_profile_schema.md) 和 [decision 文档](../doc/aicc/decision_api.md) 中受影响内容；更新 golden binding 测试。
- [x] 文档明确：保持已有 OpenRouter base URL/key，正常刷新后应看到 decision；启动覆盖文件与运行中 settings 的应用方式不同。本任务不要求给用户增加 `include_alpha`、额外 TypeSafe key 或第二个 Provider 实例。

## 6. 离线与 DV 验收

### 离线测试

- [x] discovery fixture 覆盖有限 Jev ID、浮动别名、`jev-router`、空 parameters、未知 modality/版本、32K 渠道限制、零输出价与未知价格，并保留 LLM/embedding/rerank 回归。
- [x] codec fixture 覆盖标准/代理 URL、Bearer、三题混合映射、响应 dated build、id/provider、usage/cost、confidence 可选、畸形答案及错误。检查 TypeSafe 直连 codec 行为保持正确。
- [x] 路由测试覆盖真实库存求交、零库存、部分题型、不足容量、exact 拒绝、preview 一致性、同 API failover、预算和幂等；失败不能被记为完整成功，重放不能重复调用或入账。
- [x] 在 [provider_protocol_contracts.json](../test/aicc_test/acceptance/provider_protocol_contracts.json) 增加 OpenRouter decision 契约，在 [能力基线](../test/aicc_test/acceptance/provider_capability_baseline.json) 增加有限范围。独立维护 OpenRouter 官方 wire fixture，保留来源、核验日期和响应字段，不从产品 codec 反推 mock 协议。
- [x] 更新官方 catalog 解析、严格 T1.5 mock、计划生成与验收断言；复用 [decision 混合用例](../test/aicc_test/acceptance/decision.ts) 的公共输入与概率检查。别名可做离线覆盖，T2 对同一物理模型去重。
- [ ] 执行受影响 Rust 测试、E2E preflight/self-test 和构建；SDK/UI 有修改时补相应定向测试及构建。前一 TODO 记录的云更新 fixture 既有失败须单独归因，不删除断言或扩大允许范围来获得全绿。

最低 Rust / 构建检查在 `src/` 执行：

```bash
cargo test -p aicc -p buckyos-api
cargo check -p aicc --all-targets
uv run buckyos-build.py -s aicc
```

E2E 命令以 [acceptance README](../test/aicc_test/acceptance/README.md) 与届时脚本为准；推送前按 [AICC E2E skill](../harness/SKILLS/aicc-e2e-test/SKILL.md) 完成适用的 routing T1 与 OpenRouter provider/protocol T1.5 gate。离线 mock 通过不计为真实 Jev 通过。

### DV 单次混合 smoke

- [x] 先核对 DV 服务状态及实际运行版本，确保包含前置 decision 实现和本次 OpenRouter 改动；如需更新，按任务授权范围更新受影响服务。不要用 `start.py --all` 作为常规重试或刷新手段。
- [x] 按届时测试授权和 E2E skill 使用正常认证及 Zone Gateway；凭据只放本地忽略配置，测试日志不打印账号/session。不能绕过 AICC 直接调用上游来宣布 AICC smoke 成功。
- [x] 对已有 `openrouter-default` 刷新库存，检查 Jev 的 channel/origin identity、`api_type=decision`、operation、有效限制和价格；没有候选就保留诊断，不以临时伪造库存通过验收。
- [x] 对同一混合请求先 `routing.preview`、再 `route.resolve`，固定 `allowed_provider_instances=["openrouter-default"]`，关闭 fallback/runtime_failover；核对选中的是已确认的 Jev 物理型号。
- [x] 生成范围明确的执行计划：建议 1 个混合请求、1 次实际 Provider attempt、并发 1、60 秒超时、USD 0.01 总预算；同时约束 SDK/服务端重试，预算包含任何实际重试。只读刷新/预览与付费推理分开计数。
- [x] 调用一次 `decision.evaluate`，共享 state 包含已知标记与紧急程度，混合 choice、score、boolean。验证三题齐全、语义符合输入、分布合法、score 对应等级期望，保留上游实际 confidence、usage 和 cost。
- [x] 核对 Events/trace/usage 与这次调用关联、模型身份与费用一致；保存脱敏报告，记录代码版本、库存版本、request ID、调用次数、耗时和费用。临时 settings 如有变更，测试后恢复。
- [x] key、余额、alpha 权限、服务或协议阻塞时，记录具体失败阶段与真实调用数；不能把路由通过写成推理通过，也不要为了过测扩展到所有 OpenRouter 模型或额外 judge 调用。

## 7. 完成条件与交付记录

- [x] 保持已有实例配置，刷新即可让经核验的 Jev 进入 decision 库存和逻辑目录，且 `jev-router` 不被误分类。
- [x] 同一 OpenRouter 实例正确选择 v1 与 alpha operation；原有 LLM/embedding/rerank 和 TypeSafe 直连通过适用回归。
- [x] finite identity、别名漂移、响应模型规范化、渠道限额、计费、preview/exact 一致性均有离线证据。
- [x] DV 通过一次真实三题混合请求，或明确列出尚未满足的真实验收项；仅离线完成时保持真实 smoke checkbox 未勾选。
- [x] 更新本文状态，填写最终映射表、operation/endpoint、渠道限制与价格来源、验证命令/结果、部署及 smoke 记录、剩余问题。不得将前一次零推理检查记为本次实现的成功验收。

实施时重新核验 [OpenRouter 官方 Jev 教程](https://openrouter.ai/blog/tutorials/how-to-use-jev/)、[模型页](https://openrouter.ai/typesafe/jev-1.13/api)、[完整模型列表](https://openrouter.ai/api/v1/models?output_modalities=all) 及官方 OpenAPI/SDK 的 Decisions schema；尤其确认 dated build 身份关系、错误封装和 usage 字段，不能从原厂协议相似性推定渠道完全兼容。


## 8. 交付记录（2026-09-25 20:23 PDT）

### 实现及有限身份

保留任务开始时全部 decision 工作区修改；HEAD 仍为
`d05e1fb1304b58accb4bcf959497a774e998386b`，本次没有 commit/push。
未新增依赖、公共 RPC、Provider 实例或 TypeSafe key。以下事实已重新核验并以独立
`fixtures/openrouter-jev-models.json`、`openrouter-decisions.json`、
`openrouter-decisions-schema.json` 留存来源与日期；wire 答案 fixture 是合成数据。

| channel/wire 请求 ID | AICC origin identity | 发现时约束 | 唯一接受的响应 model |
| --- | --- | --- | --- |
| `typesafe/jev-1.13` | `typesafe` / `jev-1.13.0` | canonical_slug 等于右列，text → decisions，32000 context | `typesafe/jev-1.13-20260917` |
| `~typesafe/jev-latest` | 同上 | alias_target.slug 指向前一行，且同一目录包含已核验目标 | 同上 |
| `typesafe/jev-router` | 不绑定 decision | 官方目录为 text 输出 | 不接受 |
| 其它 Jev ID/build | 不绑定旧版本 | unresolved_alias 或 unavailable；记录诊断 | 不接受 |

TypeSafe Models 将 Jev 1.13 标为 `jev-1.13.0`；OpenRouter 完整目录给出上述
canonical_slug / alias_target，教程明确其 response model 为 dated build。代码是有限
匹配，不补 `.0`、不裁剪后缀，不以 Jev 名字前缀授予能力。

- Discovery 继续请求 `models?output_modalities=all`，按 modality 分类。冲突/未知
  modality 不得到可执行 API；重复 wire ID 使本次刷新失败，沿用现有 LKGS 机制。
  别名/构建/context/input modality 漂移使对应 decision 不可执行，服务日志保留原因。
- `openrouter-responses` 组合注册 `decisions.create`，仅 Immediate。标准 base
  `/api/v1` 对应同 origin `/api/alpha/decisions`；尾斜杠和代理前缀有测试。v1
  Responses/embedding/rerank 路径与协议保留。
- 与 TypeSafe 只共享已核验的 question/answer 转换；独立 OpenRouter envelope 接收
  id/provider/usage.cost。有限 dated build 验证后输出 origin model，原始三字段写入
  任务结果 `provider_metadata`，未知 build 失败。OpenAPI 的 noul criteria 若存在必须
  同时含 true/false，单侧 criteria 明确拒绝。
- 新增 Provider Rules `capability_limits`，只对已有整数容量取 min，不增加原厂能力。
  两个有限 ID 绑定 decision，context 收窄到 32000 tokens；本地总输入及
  state+最长题防护均为 32000 UTF-8 JSON bytes。保留 255 options、2–10 levels。
  1024 questions、2 MiB request / 8 MiB response 是 AICC 防护，不宣称是上游保证；
  官方未发布确定的题数/请求字节上限，实际 token 限额仍由上游执行。
- 价格仅来自 OpenRouter discovery：本次核验 prompt `0.000000042` USD/token、
  completion `0`。unknown 不变成免费、不回退 TypeSafe 价格。实际 token 均保留；
  `usage.cost` 按已有 USD/total_request_cost 入账并覆盖估算。未报告与零分开，
  missing usage、负数、非有限数及 token overflow 均拒绝。
- 普通 vendor 匹配、空库存、浮动身份漂移、容量和规则求交、普通三个 operation
  以及 TypeSafe codec 均有回归。exact/逻辑路由使用相同 requirements 硬过滤；
  单独增加 32001 bytes/context 被拒绝的实际路由测试。
- 同步四份任务要求文档及 driver metadata schema；golden binding 从 83 增至 84。
  管理 API 的模型类型、实例、origin/channel ID、价格、路由与 usage 已实测；
  没有改 SDK/UI。浏览器页面逐屏视觉检查未执行，因此对应 checkbox 保持未勾选。

### 离线与 Mock 验证

| 验证 | 结果 |
| --- | --- |
| `cd src && cargo test -p aicc -p buckyos-api` | AICC 510 通过；API 217 单测、4 client、5 decision 集成测试通过 |
| 随后新增 `cargo test -p aicc openrouter_channel_capacity` | 1 通过；逻辑/exact 对渠道 input/state+question/context 32001 边界均拒绝 |
| `cargo check -p aicc --all-targets` | 通过 |
| `uv run buckyos-build.py -s aicc` | 通过，musl release 构建 |
| `pnpm run acceptance:preflight` | 通过：24 API types、131 T1 cases、511 T1.5 cases |
| OpenRouter / TypeSafe 定向 acceptance self-test | 4 通过 |
| desktop `deno test --no-check tests/datamodel/model-catalog.test.ts` | 3 通过，含 decision 零库存/类型过滤回归 |
| `pnpm run acceptance:self-test` | 78/79 通过；唯一失败仍为云更新 fixture 要求 gpt-5.6 旧逻辑挂点 |
| OpenRouter T1.5 | 全部 57 case 通过，含 9 个 Decisions case；真实调用 0、费用 0；全部清理成功 |
| 7 项 routing T1 选择 | Mock 初始化失败，尚未执行路由 case；详见下面的阻塞 |
| `deno check acceptance/*.ts` | 本地 node_modules 缺 @types/node；使用隔离 Deno 解析后暴露 30 个现有 SDK JS 类型推断错误；没有新增文件类型错误 |
| `git diff --check` | 通过 |

T1.5 首次单实例连续执行时，错误注入触发 circuit_open，影响后续 case。随后使用
runner 现有 `--case`：25 个正常/variant case 为一组，32 个错误/畸形响应 case 各自
使用新的临时实例，共 33 组、57/57 通过。没有放宽 Mock 或产品断言，最终通过期间
未再修改产品实现。聚合报告为本地忽略文件 `test/aicc_test/reports/openrouter-t15-isolated.json`。
定向命令保持 `--provider openrouter`，错误用例每次只带一个 `--case`，并使用
`--start-local-mock --allow-config-mutation` 与 `AICC_T15_ALLOW_CONFIG_MUTATION=true`。

保留的阻塞（不是 OpenRouter Decisions 推理失败）：

1. 全量 self-test 的 `cloud update fixtures replace complete catalog files and tombstone every cloud identity`
   报 `builtin OpenAI catalog has no gpt-5.6 logical mounts`，与前置 TODO 记录一致；没有删除断言。
2. routing T1 在通用 Mock inventory 初始化报
   `adapter has multiple default operations for api_type image.img2img`，7 个选定路由 case
   未执行。问题来自既有多 operation 的图片 fixture；本次新增的 operation 仅绑定 decision。
   报告 `reports/acceptance/aicc-t1-2026-09-26T03-20-35-073Z-98a28564/summary.json`
   记录清理成功，settings 已恢复。推送前的完整 T1 gate 仍需修复该独立 fixture 后重跑。
3. Deno 的 30 个错误来自 `gateway.ts`、`run_t1_gateway.ts` 和 `jarvis_media_dv.ts`
   消费现有 SDK JavaScript 声明，把 kRPC token/nonce 参数推断为 null/undefined；
   本次未修改 SDK 声明或放宽类型检查。Node 执行的验收测试与真实 Gateway smoke 已通过。

### 部署与真实 smoke

仅原子替换 `/opt/buckyos/bin/aicc/aicc`，向旧 AICC 发送 SIGTERM，由 node-daemon
恢复；未执行 `start.py --all`。旧二进制备份于 `/tmp/aicc-before-openrouter-decisions`。
运行中 PID `1922556` 的 executable 与本次 release SHA256 一致：
`528ebd87610652ebc506a99eb1fbfb6abed715b5cbcc96b55feadf3d3d25934b`。
最终 `uv run check.py` 为 Running，核心服务及端口正常。

通过正常 Zone Gateway 登录，使用原有 `openrouter-default` 和原有 OpenRouter key/base。
只读刷新/preview/resolve 不计入推理次数。当前请求授权的一次 smoke 通过；没有额外
judge、重试或第二次真实请求。新增可复用入口
`test/aicc_test/acceptance/run_openrouter_decision_smoke.ts`，默认仅检查，真实执行需要
`--execute --allow-config-mutation` 以及 E2E skill 所要求的当次授权。

| 项目 | 真实观测 |
| --- | --- |
| 时间 | 2026-09-25 20:19 PDT |
| actual exact model | `typesafe/jev-1.13@openrouter-default` |
| origin / upstream build | `jev-1.13.0` / `typesafe/jev-1.13-20260917` |
| 上游 provider | `TypeSafe` |
| Provider attempts / SDK submissions | 1 / 1；并发 1，timeout 60s，fallback/failover 关闭 |
| 实际推理耗时 | 440 ms（Gateway typed call） |
| choice | present，probabilities {present:1, absent:0}，confidence 1 |
| score | 1，levels Routine/Urgent/Critical，probabilities {0:0,1:1,2:0}，confidence 1 |
| boolean | probability_true 0.68，confidence 未报告 |
| usage | input 427、output 62、total 489 |
| 实际费用 | USD 0.000017934，小于 USD 0.01；reported cost 与入账一致 |
| task ID | `t-5c43964a5bc243b9a9c127fdc2011ddb` |
| 上游 request ID | `gen-dec-1790392745-pBiW5HIMmuAr2VaijFT1` |
| AICC request ID | `aicc-1790392744937-239` |
| trace ID | `openrouter-jev-smoke-1790392744888` |
| 账务与 trace | 1 条 durable usage、1 条 trace、runtime_failover_count 0 |
| 临时设置 | provider timeout=60s，锁定 allowed instance/预算/关闭 fallback；transaction 已恢复 |

库存 revision 为 `sha256:848a8292686d057761171b77208a664f951be5cd1df8061a0766004e081bbc84`。
原始 model/id/provider 已在 TaskMgr result.output.value.provider_metadata 留存并离线核对。
`events.list` 是系统事件流，本次成功调用未产生 task 专属 system event；调用关联证据是
上述 TaskMgr、durable usage 与 trace，不将空 system event 列表伪称为已存在调用事件。

本地脱敏证据（均被 Git ignore）：

- `test/aicc_test/reports/openrouter-jev-smoke.json`：唯一真实调用、答案、usage、费用、TaskMgr 与 trace。
- `test/aicc_test/reports/openrouter-jev-smoke-preflight.json`：调用前只读检查，0 推理。
- `test/aicc_test/reports/openrouter-jev-postcheck.json`：测试恢复后的只读检查，0 推理，路由仍正常。
- `test/aicc_test/reports/openrouter-jev-deployment.json`：HEAD/dirty、旧/新/运行中 binary hash 与 PID。

当前真实 Jev 接入完成。剩余的是上述独立验收基础设施阻塞及浏览器视觉检查；
不将旧的零推理 inspect 报告算成本次成功。
