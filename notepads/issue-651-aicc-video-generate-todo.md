# Issue #651：统一视频生成与官方协议接入 TODO

Review 日期：2026-10-22。需求来源：[Issue #651](https://github.com/buckyos/buckyos/issues/651) 及[补充评论：重点接入 MiniMax H3、Seedance 2.x](https://github.com/buckyos/buckyos/issues/651#issuecomment-6094948666)。

本轮范围（按 2026-10-22 讨论收敛并补充）：先完成统一视频 API，以及 MiniMax H3、Seedance 2.x 的官方云端协议接入；同时补齐 OpenRouter 的 H3/H3-Max 渠道名称映射、视频发现、任务执行、产物下载与视频计费。H3 私有部署协议仍可能演化，等首次实际接入时，依据选定后端和版本再调整适配；私服实现、能力登记及专项验收不作为本轮完成条件。公共 API 按通用生成语义设计，本轮不为 SGLang 等后端预设专用字段或扩展框架。

代码基线：本地 `buckyos/main` HEAD `a27716879a513d81f3a685893ae5410a61b163fe`，同级 `buckyos-websdk` HEAD `1f8e8ec59cfb6f90189fefa2f20c8264529decb4`；按当前工作区源码核查，未把历史 notepad 当作已实现能力。复选框在 2026-10-22 实施后更新：`[x]` 为已实现并有自动化验证，`[ ]` 为未完成或未验证，实施记录见 §9。

OpenRouter 补充核查基线：同日 `buckyos` HEAD 已为 `401d7b071259ca661c2b381f52d86ff7094f9edc`；下文渠道映射、discovery、adapter 与计费现状依据该工作区源码及用户提供的运行日志重新核对。

## 1. Review 结论与范围

本次应沿用 AICC 的 DTO → 路由 → canonical lowering → ResourceManager → Protocol Adapter → ExecutionEngine 链路，收敛视频公共协议并补齐请求级能力校验，不需要重建视频任务框架。

- H3 云端 v2、Seedance Ark 和 Veo 已有原生异步任务实现；本轮补齐多条件输入、角色与组合约束、输出规格和引用映射。H3 私有部署留待首次实际接入。
- Issue 中“尚无使用方”不能作为删除时无需联动的依据：当前 Agent 视频命令、Desktop Playground、SDK 和验收代码都引用旧接口。尚未核实外部生产调用方；仓内与同级 SDK 消费面必须同批迁移。
- `video.generate` 固定 `exact_model`，`helper.video_generate` 接受逻辑模型；两者共享业务结构、校验和需求推导。移除旧生成接口及别名，不做兼容 dispatcher。
- 不能把所有 `video.video2video` 机械改为参考视频生成：现有 OpenAI 分支有编辑语义，Doubao 分支则发送 `reference_video`，且未使用旧请求的 `preserve_motion/time_range`。须逐 operation 区分生成、编辑、续接。
- 重点交付 H3/H3-Max 原厂云 API、OpenRouter 官方视频 API 和 Seedance 2.x 官方 Ark API；Veo 用来检验公共协议的可映射子集。其它已接入视频模型也要完成接口迁移或明确撤下不成立的能力声明。
- OpenRouter 已有 H3/H3-Max 模型，但当前 AICC 对这两个渠道 ID 同时存在身份映射、视频模态、视频 operation 与计费缺口。补名称映射只能解决身份识别，不能据此宣称已经可生成视频。
- 按具体模型、官方协议和当前适配器声明支持范围；不静默丢条件、丢音轨要求、改输出规格，或用 prompt 替代强制关键帧。

## 2. 当前实现与改动入口

下列链接相对本文件；表内提及的符号便于重新定位，行号不作为长期锚点。

| 层次 | 当前实现证据 | 本次改动 |
| --- | --- | --- |
| 公共 Rust 协议 | [aicc_client.rs](../src/kernel/buckyos-api/src/aicc_client.rs)：`ai_methods`、`ApiType`、三类旧视频 request/response、`AiccCall`、client/handler/dispatcher；结果为单个 `video`。已有 `ResourceRef`、`TimeRange`、执行模式和结构化错误。 | 新增 generate/helper、强类型 conditions、视频专用 output/result，删除旧公共入口；保持请求未知字段拒绝。 |
| 服务入口 | [service/mod.rs](../src/frame/aicc/src/service/mod.rs)、[service/inference.rs](../src/frame/aicc/src/service/inference.rs)：现有 helper 只有 LLM/image，`route_input_for_call` 未自动推导视频结构化需求。 | 统一 exact/helper/route 的需求语义，接入共享校验器。 |
| lowering 与 canonical | [call/mod.rs](../src/frame/aicc/src/call/mod.rs)：`canonical_option_keys`、`api_type_name`、`rewrite_canonical_options`；[canonical/mod.rs](../src/frame/aicc/src/canonical/mod.rs)：字段映射与约束。 | 覆盖 conditions/output 的全部显式语义；当前缺少 mapping 可走默认/omit，不能照搬为视频强约束的成功路径。 |
| 素材 | [resource/mod.rs](../src/frame/aicc/src/resource/mod.rs)：授权、inspect/materialize、MIME/字节限制；[call/mod.rs](../src/frame/aicc/src/call/mod.rs) 的 `collect_resource_requirements` 递归扫描数组内 ResourceRef。 | 复用 `/conditions/N/resource` 发现与授权，补真实媒体信息、片段、音轨和派生素材管理。URL 可直传，不等于已验证其内容。 |
| 任务与产物 | [execution/mod.rs](../src/frame/aicc/src/execution/mod.rs)、[service/provider_execution.rs](../src/frame/aicc/src/service/provider_execution.rs)、[protocol/result.rs](../src/frame/aicc/src/protocol/result.rs)、[storage/mod.rs](../src/frame/aicc/src/storage/mod.rs)。 | 复用幂等、TaskMgr、native task 恢复、取消、usage 和 artifact 归属；补提交结果不确定时的去重边界及 `videos[]`。 |
| 模型与部署声明 | [catalog/schema.rs](../src/frame/aicc/src/catalog/schema.rs)、[catalog/validation.rs](../src/frame/aicc/src/catalog/validation.rs)、[model/mod.rs](../src/frame/aicc/src/model/mod.rs)、[service/model_defaults.rs](../src/frame/aicc/src/service/model_defaults.rs)、[driver_metadata](../src/frame/aicc/driver_metadata)。 | 增加结构化视频能力，迁移逻辑树、Model Driver、Provider Rules、adapter binding 和 inventory。 |
| H3 云端 | [minimax_media.rs](../src/frame/aicc/src/protocol/minimax_media.rs)：`MiniMaxVideoCodec` 已有 v2 submit/query/result/cancel，当前只编码文本或单张首帧；[minimax.provider.json](../src/frame/aicc/driver_metadata/providers/minimax.provider.json) 已绑定 H3/H3-Max。 | 按官方云 API 扩展真实支持子集；私有协议适配延期。 |
| OpenRouter H3 | [provider/builtin/openrouter.rs](../src/frame/aicc/src/provider/builtin/openrouter.rs)：matcher 未映射 hailuo-3，`classify_modalities` 不识别 video，价格只解析 prompt/completion；[chat_completions_dialects.rs](../src/frame/aicc/src/protocol/chat_completions_dialects.rs)、[derived_responses.rs](../src/frame/aicc/src/protocol/derived_responses.rs) 的 OpenRouter 分支无视频 native task codec；[openrouter.provider.json](../src/frame/aicc/driver_metadata/providers/openrouter.provider.json) 无视频 operation。 | 按 OpenRouter 官方视频合同贯通身份、发现、路由、执行、下载和费用，不套用原厂 MiniMax operation。 |
| Seedance | [doubao_media.rs](../src/frame/aicc/src/protocol/doubao_media.rs)：`DoubaoVideoCodec`，标准/plan Ark task；单图固定 `first_frame`，单视频使用 `reference_video`。 | 统一多素材 role、版本限制、参数和结果映射，校正 edit/extend 语义。 |
| 其它适配器 | [gemini.rs](../src/frame/aicc/src/protocol/gemini.rs)、[openai_responses.rs](../src/frame/aicc/src/protocol/openai_responses.rs)、[fal_queue.rs](../src/frame/aicc/src/protocol/fal_queue.rs)、[glm_media.rs](../src/frame/aicc/src/protocol/glm_media.rs)、[qwen_media.rs](../src/frame/aicc/src/protocol/qwen_media.rs)。 | 逐模型迁移支持子集；泛化 queue/透传不能代替模型合同验证。 |
| TS SDK 与工具 | [WebSDK aicc_client.ts](../../buckyos-websdk/src/aicc_client.ts)、[Agent lib/aicc.ts](../src/tools/buckyos-agent/lib/aicc.ts)、[lib/types.ts](../src/tools/buckyos-agent/lib/types.ts)、[commands](../src/tools/buckyos-agent/commands)。 | SDK 白名单、调用映射、生成命令、任务恢复和 `videos[]` 解析同步迁移。系统 CLI 当前 `ai` 模块没有视频生成子命令。 |
| UI 与验收 | [Playground DataModel](../src/frame/desktop/src/app/ai-center/datamodel/playground.ts)、[aicc_mgr.ts](../src/frame/desktop/src/api/aicc_mgr.ts)、[PlaygroundResult.tsx](../src/frame/desktop/src/app/ai-center/components/playground/PlaygroundResult.tsx)、[acceptance](../test/aicc_test/acceptance)。 | 迁移表单/API 列表/default route/mock/i18n；结果 renderer 已递归处理数组，可复用并补回归。 |

### 2.1 OpenRouter 补充发现与现有名称映射机制

用户提供的 `/opt/buckyos/logs/aicc/aicc.879368.log` 已核查：860、861 行分别记录 `minimax/hailuo-3`、`minimax/hailuo-3-max` 为 `unmatched: UnresolvedAlias`；540、565 行还记录两者的 output modalities 无法分类。这是本机当时的诊断证据，不等于 OpenRouter 没有这些模型。

[OpenRouter H3](https://openrouter.ai/minimax/hailuo-3) 与 [H3 Max](https://openrouter.ai/minimax/hailuo-3-max) 页面将这两个 ID 分别标为 MiniMax H3、H3 Max。2026-10-22 只读查询其 `/api/v1/videos/models` 也返回两者。`hailuo-3` 是 OpenRouter 渠道 ID 的组成部分；原厂与本地 canonical 名仍为 `MiniMax-H3`，按渠道名称映射处理即可，无需重命名既有 Model Driver。

当前 `ModelDriverCatalog` / `ModelExactRule` **没有通用 `aliases` 字段**。已有映射机制位于 Provider 层：`ProviderRulesCatalog.model_driver_overrides` 及实例级 `ProviderInstanceRules.model_driver_overrides`。`InventoryBuilder` 的身份解析顺序是“实例 override → Provider Rules override → Provider 自定义 matcher → 通用 catalog matcher”；自定义 matcher 返回 Failed 时不继续猜测。OpenRouter 的 `canonical_slug/alias_target` 目前只在 JEV 特殊验证中使用，并非通用别名解析器。

本轮优先在 OpenRouter Provider Rules 复用以下映射（待实现），不新造 Model Driver alias schema，也不复制一份 OpenRouter 专属 H3 Model Driver：

```json
{
  "model_driver_overrides": {
    "minimax/hailuo-3": "minimax/MiniMax-H3",
    "minimax/hailuo-3-max": "minimax/MiniMax-H3-Max"
  }
}
```

映射只统一模型身份：向 OpenRouter 提交的 `provider_model_id` 仍保留 `minimax/hailuo-3`，exact model 仍可为 `minimax/hailuo-3@openrouter-main`；模型能力取渠道与适配器实际支持的交集，价格属于对应 Provider 实例。原厂直连和 OpenRouter 不能因身份相同而合并 operation、任务或费用。

本节名称映射用于渠道身份识别；旧视频 API 的兼容别名仍按 P1 删除。

## 3. 开工前先冻结的协议决策（P0）

- [x] 确定 `VideoGenerateRequest` / helper 共享的业务字段：`prompt?`、`negative_prompt?`、默认空 `conditions`、`output?`、`seed?`，以及现有 trace/session/idempotency/task/execution 契约；空白 prompt 与空 conditions 同时出现时报错。模型要求 prompt 时再做模型级检查。
- [x] 定义带 `type` 标签、拒绝未知字段的 `VideoCondition`：`reference_image`、`reference_video`、`reference_audio`、`keyframe`；所有项要求唯一 `id` 和 `resource`。`time_range` 仅属于视频/音频；`include_audio` 仅属于视频且默认 false；`position=start/end` 仅属于 keyframe。复用 `ResourceRef` 与现有 `TimeRange`，不要复制传输协议。
- [x] 冻结 ID 字符集/长度、大小写及字面量转义。建议 ID 用 ASCII 标识符（如 `[A-Za-z][A-Za-z0-9_-]*`），字面量 `@{...}` 用明确的转义规则；最终规则须以 Rust/TS 共用正反例冻结。重复 ID、悬空引用、非法转义报错；未被 prompt 引用的条件仍必须生效。
- [x] 冻结关键帧重复位置处理、派生音轨的内部身份及计数规则。公共 ID 不随数组重排或厂商编号变化；一个视频拆成图像流与音轨后仍可回溯原条件，派生 ID 不与用户 ID 冲突。
- [x] 定义 `VideoOutputSpec`：`duration_seconds`、`aspect_ratio`、`size`（`{short_edge}` 与 `{width,height}` 严格二选一）、`fps`、`audio=auto/generate/none`、`media_type`。确认数值类型、有限正数/整数边界、精确尺寸与比例冲突规则、短边尺寸对齐与实际尺寸记录方式。与通用 `AiOutputOptions` 分开，避免影响图片/音频。
- [x] 明确省略与显式值的区别：省略可采用部署默认值；显式不支持值必须拒绝。`audio=none` 若需要去音轨，必须作为明确处理步骤，不能将模型生成有声结果直接报告为满足要求。负向提示词不支持时拒绝；seed 不承诺跨模型复现。
- [x] 固定新结果为 `videos[]`：每项携带 ResourceRef 及实际时长、宽高、fps、音轨信息；可选尾帧 artifact。区分 Provider 声明与实测元数据；pending/failed 状态不强制已有结果，成功状态不得假报规格。第一版单结果，不添加自动多次推理；消费端按数组读取。
- [x] 逐一裁决旧 `video.video2video`：参考重生成转 generate；具有真实源视频编辑合同的 operation 转独立 `video.edit`；无法证明的能力撤下。`video.extend` / `video.upscale` 保持独立职责，续接不得等同普通参考。不为本轮凭空补齐所有编辑能力。
- [x] 选择 `ModelRequirement` 中视频结构化约束的具体结构，并和 `route.resolve`、helper 共用。区分调用方提出的需求与服务端探测事实；route 的预选不能替代执行前对实际素材的复核。

**P0 完成条件：** Rust/TS JSON 样例、错误样例、每个 Provider 的“支持／拒绝／待验证”矩阵、旧 operation 迁移表一起评审；没有依赖数组位置、品牌猜测或无约束 `provider_options` 的设计。

## 4. 实施任务

### P1：公共协议与服务入口

- [x] 联动 method 常量、`is_ai_method/is_aicc_core_method`、`ApiType`、`Capability` 映射、DTO、`AiccCall`、Rust client、`AiccHandler`、dispatcher、服务 handler 宏和 `ProtocolOutput::validate_canonical_type`。helper 复用同一 generate 校验及执行分支；同步 `service/management.rs`、`route_input_for_call` 和 `exact_call_for_route`。
- [x] exact 调用只校验并执行指定实例，不换模型；helper 在实际素材事实可用后推导完整需求，候选及所有 fallback 都必须满足同一条件。不能让调用方传入的 requirements 降低请求本身的要求。
- [x] 明确两阶段校验：先检查 schema、授权并落实有效 privacy/local_only 策略，在任何 URL fetch/远程探测前过滤不合格部署，再做受限素材探测与完整需求推导；之后“路由 → lowering 确定 adapter/operation/输入形式 → 素材准备 → 最终语义校验/编码提交”。现状是先路由再物化，新增探测不能绕过这个外发保护；复用探测结果，避免每个候选重复下载。
- [x] 复用 `AiccError`，为错误 details 统一提供字段路径、condition ID、期望限制和实际值；schema 错误、不可路由、不支持能力、素材不可访问和 Provider 故障按现有错误分类表达，不泄露凭据。
- [x] 删除旧公开生成 method、ApiType、DTO、SDK 方法和 logical mounts。旧 method 返回 UnknownMethod，旧字段不能落入透传；仅在明确的历史文档或旧接口拒绝测试中保留旧名称。

### P2：素材事实、引用与精确语义

- [x] 在现有 ResourceManager 上增加视频所需事实：实际 MIME/容器、宽高、时长、fps、音轨；资源声称的元数据不能直接当真。覆盖 URL/Base64/Named Object，保留 tenant 授权、下载大小及超时限制。
- [x] 明确 URL 校验方式。当前 `UrlOrBytes/UrlOnly` 可以直传 HTTPS URL，不能据此声称已检查实际时长或音轨；必须用受控探测/可信资源元数据或明确拒绝无法验证的请求，并确保验证与最终提交使用同一份内容。
- [x] 校验 `[start_seconds,end_seconds)`：非负、有限、start < end、end 不超过实测时长；片段与输出时长相互独立。目标协议不能精确表达时，明确准备裁剪素材或拒绝，不以改输出时长冒充输入裁剪。
- [x] 落实 `include_audio`：false 时保证目标模型不使用源音轨；true 时必须有音轨、保持片段同步并计入音频限制。明确无音轨、分离失败、音视频时长不一致的错误，不静默忽略。
- [x] 复用资源机制保存派生素材及其原始条件映射，规定临时文件/缓存的权限、清理和重启恢复。探测/裁剪工具先核查现有运行环境；若需新增依赖或通用组件，按仓库约定在方案具体后确认，不能先假设服务器已有 ffmpeg/ffprobe。
- [x] 为每个 adapter 生成实际提交顺序的 `id → 厂商素材标记` 映射，再进行语法化 prompt 替换；禁止全局字符串 replace。覆盖重排、重复引用、转义、不同模态独立编号、派生音轨及关键帧。
- [x] 保存原始 prompt、提交 prompt、映射与实际采用的预处理版本到有权限的任务执行记录；普通 trace/日志只保留允许展示的摘要。不把 H3 Context-IR 格式引入公共 DTO，本轮不为私有推理预建 prompt 预处理流水线。
- [x] 审查资源限额：当前默认最多 16 项、单项 32 MiB、总量 64 MiB、inline 4 MiB。按原始/派生素材分别计数，核对多素材真实大小和 Provider 限额；通过明确部署配置调整，不全局无界放宽。

### P3：能力约束、lowering 与模型目录

- [x] 在现有 Model Driver / Provider Rules / adapter / instance 能力交集中表达：条件类型与角色、关键帧位置、各类及总数、单段/总时长、格式/尺寸/音轨、组合互斥，以及条件与输出规格关联限制。部署只能在真实支持范围内声明能力。
- [x] 处理 canonical 缺 mapping 时可省略的现有行为：显式 conditions/output/negative_prompt/seed 必须“精确映射、明确允许的转换或拒绝”；不能依赖默认 `strict=false/allow_fuzzy=true` 接受语义损失。
- [x] 同步 `canonical_option_keys`、`api_type_name`、资源 JSON pointer、`rewrite_canonical_options`、operation 参数拆分、结果校验。生成原始/准备后/提交三个阶段的可追踪证据，确保 Provider Rules 的 set/remove 不会删除强条件。
- [x] 迁移 `driver_metadata/models/{minimax,doubao,gemini,openai,fal,glm,qwen}.model.json` 中实际存在的旧视频声明，以及对应 providers、known-providers、`model_defaults.rs`、inventory 和逻辑目录；按文件实际模型集合修改，不只改 H3 条目。
- [x] H3/H3-Max 云端与 Seedance 2.0/2.5 等具体版本分别登记，保存模型身份、官方协议版本及验证依据。本轮不新增私有 FL2VA/Ref2VA 的能力条目，不从同名模型推断未接入实例的能力。
- [ ] 校验已有 settings 自定义逻辑树、overlay、inventory LKGS 和 metadata 更新后的旧 ApiType 处理：明确开发阶段旧配置失效、重建/刷新方式，错误可定位；不偷偷保留旧别名，也不清空其它服务数据。

### P4：重点 Provider 官方协议

- [x] **MiniMax H3/H3-Max 官方云 API**：依据官方 v2 合同，在现有 `MiniMaxVideoCodec` 上扩展 `content[]`、素材角色、规格、错误与结果映射，保留现有 native task/usage/cancel。检查 media adapter 在 `minimax_messages.rs` 中的注册，避免只改独立 codec。
- [x] 按具体云端模型的官方支持范围覆盖文本、首帧、尾帧、首尾帧、图片/视频/音频参考及混合输入；逐项验证组合限制，不把 SGLang 的组合能力套到云 API。执行方式由条件与模型能力推导，不添加公共 txt2video/ref2video mode。
- [ ] 保持现有 privacy/local_only 策略有效：官方云实例不能当作 `LocalInference`，要求 local_only 且无合格本地候选时应在访问素材前失败。本轮验证拒绝路径，不要求部署 H3 私服或交付本地完整生成流程。
- [x] **Seedance 2.x** 扩展现有 `DoubaoVideoCodec`，把公共 condition 精确映射到 `content` 与 role；标准与 agent-plan 端点均验证。所有生成输入形态一致处理 seed、audio、时长/尺寸等显式参数；补全组合、audio-only 和版本限制。核对 `derived_responses.rs` 注册。
- [ ] **Veo** 按可用 endpoint 核验首尾帧、参考图与续接，支持子集清楚声明；不支持的参考视频/音频/组合明确拒绝。官方当前页面已出现旧 preview 停服提示，必须先确认实际模型与端点，不把历史 mock 当在线证明。
- [x] **其它适配器** 检查 OpenAI、fal、GLM、Qwen 的生成/编辑语义及输出规格；新增 binding 与 metadata 必须同步。未实现的模式撤下声明，不借统一方法扩大能力承诺。
- [x] 针对已有字段丢失路径补回归：fal 的 `canonical_body` 删除 `output`；GLM 视频编码只读取部分规格；Qwen 部分 img2video 路径移除 ratio；OpenAI 的 `ignore_unsupported_options` 忽略部分 seed/audio/preserve_motion 参数。新接口须精确映射或明确拒绝，不能继承这些静默处理。
- [x] 官方协议需要的预处理或后处理显式记录，不开放任意公共 provider 参数；不为尚未接入的私有后端新增 steps、scheduler、flow shift 等配置。

#### P4.1：OpenRouter H3/H3-Max 完整接入

- [x] **身份映射**：在 `openrouter.provider.json` 加入 §2.1 两项 `model_driver_overrides`，保留实际渠道 ID、H3/Max 独立身份以及映射来源。对未知后缀、无效目标和有歧义的名称继续明确失败；记录官方目录的 `id/canonical_slug` 与核查版本，不做全局 hailuo→H3 字符串替换。
- [x] **视频发现**：扩展 `OpenRouterDiscovery` 和 `classify_modalities`，接入官方 `/videos/models` 的能力及报价，与现有 `/models?output_modalities=all` 按 ID 校验、合并。保留未识别模型的诊断；正确区分输入 video 的理解模型与输出 video 的生成模型。映射成功、渠道能力、adapter binding 三者齐备后才允许路由，不能以静态补库存掩盖缺失的 codec。
- [x] **operation 与任务**：按 OpenRouter 官方合同增加视频 `NativeTaskCodec` 和 `video.generate` binding，同步 `openrouter_responses_adapter`、derived adapter 装配及 Provider Rules；实现 `POST /api/v1/videos`、`GET /api/v1/videos/{jobId}` 和内容下载。复用 ExecutionEngine 的持久任务、恢复与幂等；取消能力以官方确实提供的接口为准，不能因存在 cancelled 状态就宣称支持主动取消。保持现有聊天/embedding/rerank/decision 可用。
- [x] **字段与能力**：依据模型目录和官方请求合同把 keyframe、参考图、output、audio、seed 等转换为渠道字段；检查 `frame_images` 与 `input_references` 同传时上游优先使用前者的语义，不能导致参考条件被丢弃。参考视频/音频、关键帧混合等只有在渠道合同与实际映射成立时开放，不从 H3 原厂或模型介绍继承。上游 provider passthrough 白名单仅可服务于受控 lowering，不暴露任意公共 `provider_options`。
- [x] **产物与认证**：解析 job 状态、`polling_url`、`unsigned_urls`、generation ID 与终态错误；下载走现有 artifact 授权和 Provider credential 机制，验证原始 job/租户/来源关系及 URL 有效期，不能把带鉴权要求的内容地址当匿名下载链接。保留 known-provider 的 Bearer、region/base URL/path prefix 契约，不能替换为 MiniMax `/v2` 或假设兼容 OpenAI multipart 视频协议。
- [x] **视频计费**：当前 discovery 只读 token 单价；补官方 `pricing_skus` 的受支持维度和终态 `usage.cost`。优先复用 `PricingUnit::VideoSecond`、`AiUsage.video_seconds`、条件价格规则、固定任务价格快照，以及已有 USD `reported_cost=total_request_cost` 语义；上游总费用与本地估价不得重复累计。按计费时长、分辨率、音频/参考素材等适用维度估价，不复制 MiniMax 直连价格，不把零 token 当免费。未知单价/计量明确保持未知；若按秒加按图等复合计价超出现有结构，先冻结最小计量与估价补充方案，不把它误记为单一秒价。
- [x] **费用与计量传递**：`service/provider_execution.rs::poll_native` 当前在成功状态到结果下载间只搬运 `result_usage.video_seconds`；须把终态 `usage.cost` 映射到 `AiUsage.reported_cost`，保留并合并至最终 `ProtocolOutput.usage`，覆盖下载失败、恢复与重复轮询。区分上游计费秒数、产物实测时长和请求估值；现有 `resume.output_video_seconds` 来自提交参数且为 `u64`，不能充作实测值，不能截断渠道支持的小数秒。
- [x] **调用前估价**：`service/inference.rs::estimate_model_cost` 当前主要用 token 和一次请求，不能声称已按视频规格准确估价。为视频请求提供所需的输出时长/规格及计费输入摘要，更新新 output 路径对应的价格匹配，并按现有费用策略处理未知值；本轮不顺带扩建新的通用配额系统。
- [x] **完成门槛**：增加 OpenRouter 两个模型从官方目录 → 身份映射 → generate/helper 路由 → 提交/轮询/恢复 → `videos[]` 下载 → 费用归因的完整用例。按用户计划，实施完成后的首次真实闭环优先通过 OpenRouter 完成，保存任务、可下载产物与费用归因证据。仅消除 `UnresolvedAlias`、模型出现在 UI 或 mock 可返回视频，均不足以宣称真实接入完成。

### P5：任务、幂等、取消与结果

- [x] 沿用 ExecutionStore/TaskMgr。将原始请求、准备素材、引用映射、实际模型/operation、native task handle 的持久边界写清，保证重启不重复进行具有费用或外部副作用的步骤。
- [x] 专门修正/验证“Provider 已接受或接受状态不确定”边界：当前 `ProviderStartFailure::after_accept` 仍可根据错误允许 retry/failover，`can_failover` 未以 `provider_accepted` 阻断。native submit 超时或响应解析失败不能跨 Provider 再提交；只有有证据未接受时才允许受控重试，已绑定 remote task 只查询/恢复。
- [x] 幂等 fingerprint 保留原始公共请求的条件、clip、include_audio、output、seed 等业务差异；同 key 同请求回到原任务，同 key 不同请求拒绝。当前数组顺序参与 fingerprint，内部准备重排/裁剪/厂商 prompt 不得改写原请求身份。当前 helper 先转 exact 再计算 canonical body，须专项处理同 key 重试时路由结果改变的问题，恢复原绑定任务而非冲突或重复提交。
- [x] 取消覆盖预处理、排队、native 推理、结果回收阶段；上游不支持取消时按现有协议明确反馈，不伪报已取消。覆盖取消/完成竞态、重启后继续轮询、usage/配额一次性结算。
- [x] 将 `videos[]` 和可选尾帧接到 ProtocolOutput、artifact 登记/授权下载、TaskMgr output 和 typed response。当前 Base64 会物化，Provider URL 可保留 URL；沿用生命周期并明确过期行为，不宣称全部产物已持久为 Named Object。
- [x] 真实读取产物校验 duration/width/height/fps/audio，区分声明值与实测值；Provider 返回与显式约束不符时报告失败/偏差，不能将请求规格原样回填为“实际结果”。
- [x] 审查持久 execution JSON、旧 pending 视频任务和 schema 版本。无兼容实现不代表可以忽略旧上游任务；升级时给出排空/终止与旧记录失效规则，避免遗留后台付费任务或阻断其它任务恢复。

### P6：SDK、Agent 工具与 Playground

- [x] 同级 WebSDK 更新 method/ApiType、request/response、client/export 和运行时白名单；新增 conditions/output 嵌套校验，覆盖 helper。删除旧调用与导出，不能只改 TypeScript 类型。
- [x] Agent 的 `TypedRequestMap`、`API_TYPE_BY_METHOD`、`invokeTyped/resolveExactModel` 接新接口；逻辑模型路径使用新 helper 或完整需求路由，不把新 conditions 留在选模之后。
- [x] 收敛 `gen_video.ts`、`img2video.ts`、`video2video.ts` 的命令设计。建议统一生成命令表达条件 ID/类型/资源/clip/音轨；旧编辑参数只有在明确 `video.edit` 支持时保留。同步 `aicc-tool.ts`、`deno.json` 与命令源码删除，避免 `build_aios` 自动生成失效 symlink。
- [x] `aiResponseArtifacts` 当前只读单 `video`，`pickArtifact` 只取首个。增加 `videos[]` 解析、元数据、文件命名/数量检查，更新 `aiResponseData` 和任务恢复后的下载；首版只支持单结果也不能静默截断异常多结果。
- [x] Playground 合并三套旧表单，明确素材条件和输出规格；更新 API union、mock、default route、导入导出 JSON、i18n、异步结果与错误呈现。复用递归 ResourceRef renderer，补数组结果/下载回归。
- [x] 同步 [aigc_video](../src/apps/jarvis_agent/agent/cmd_skills/aigc_video.md)、[Agent 工具说明](../src/tools/buckyos-agent/readme.md)、[AICC Agent CLI 文档](../doc/aicc/aicc_agent_cli_tools.md)；核查 aigc_image/aigc_audio 是否受共享输出解析影响并记录依据。
- [x] 更新冻结协议/用户 API、路由与 Model Driver schema、Provider 接入、运行时持久数据及 E2E 需求文档。更新主文档，历史 archive 不批量改写为“已支持”。
- [ ] 通过源码构建同步 SDK 分发包和 Agent 镜像。Agent `deno.json` 读的是 `src/rootfs/libexec/buckyos-tool/dist/node.mjs`，只 build 同级 SDK 会留下旧副本；不要手改 rootfs/dist 生成文件。

## 5. 验收矩阵与完成条件（P7）

| 层次/场景 | 必须证明的行为 |
| --- | --- |
| DTO/Rust/TS | 新 exact/helper dispatch 与 JSON 一致；旧方法、旧字段、未知嵌套字段拒绝；四种条件、output size 两分支、默认值及数值边界完整覆盖。 |
| ID/引用 | 重复/非法 ID、悬空引用、转义、无引用条件；重排与派生音轨后标记仍指向正确素材，原始与提交 prompt 可按权限追溯。 |
| 素材 | URL/Base64/Named Object；错误 MIME、越权、过大、损坏、缺音轨、非法/越界 clip；`include_audio=false` 不向模型暴露音轨。 |
| 路由/canonical | exact 不换模型；helper 与 route.resolve 需求一致；组合和输出不支持时拒绝；fallback 不删条件、不降规格；零 Provider 下静态目录仍有效。 |
| H3/H3-Max 云端文本与关键帧 | 按官方合同覆盖文本、仅首帧、仅尾帧、首尾帧；实测输出规格及音轨，逐模型验证支持范围。 |
| H3/H3-Max 云端参考生成 | 图/视频/音频及官方支持的混合；各类/合计数量与时长边界；支持的组合与不支持组合的明确拒绝；clip 与音轨语义正确。 |
| OpenRouter 身份/发现 | 两个 hailuo-3 ID 映射到各自 MiniMax canonical 身份，发送 ID 保持渠道原值；覆盖 override 优先级、无效目标、未知名称及目录合并；仅加映射但无 video codec 时仍不可路由。 |
| OpenRouter 视频协议 | 官方目录 fixture → generate/helper → submit/poll/result/授权下载完整闭环；覆盖不支持的条件组合、认证失败、限流、过期、终态错误、重启恢复与提交结果不确定时不重复调用。 |
| OpenRouter 视频费用 | 按规格/时长及适用参考素材维度估价；终态费用经下载/恢复后仍进入 reported_cost，上游总费用优先；计费秒数、实测时长、请求估值不混用，小数秒不截断；未知费用不算零、目录更新不改旧任务价格快照；失败/取消/恢复不重复入账；UI/usage 归因到 OpenRouter 实例。 |
| Seedance 2.x | 文本、首帧、首尾帧、参考图、图视频音频混合；逐版本限制与非法混合拒绝；标准/plan 端点字段、轮询、取消和结果正确。 |
| Veo/其它模型 | 根据真实 operation 只声明可支持子集；生成/编辑/续接语义正确，不能把不支持条件当普通 prompt。 |
| 任务故障 | 相同 key 重入、不同请求冲突；提交已接受但响应丢失、轮询失败、进程重启、取消/完成竞态；无跨 Provider 重复提交、无重复记账。 |
| privacy/local_only 回归 | 官方云实例不能满足 local_only；无合格本地候选时在素材外发前拒绝，fallback 不绕过策略。本轮不要求私服真实推理。 |
| 消费/结果 | SDK → Agent/Playground → TaskMgr → videos[] → 授权下载；实际元数据与文件一致；失效 URL、任务恢复和单结果限制清楚反馈。 |

- [x] **单测/模块合同**：扩展现有 `aicc_client_test`、call/canonical/model/resource/execution、H3 v2、OpenRouter discovery/adapter/价格、Doubao、Gemini、`protocol/review_tests.rs` 和 `provider/builtin/upgrade_tests.rs`；Provider 映射复用 `provider/tests.rs` 的 instance/provider-rules override 用例。增加新语义断言，不只替换 method 字符串。
- [ ] **T1/T1.5**：迁移 `test/aicc_test/acceptance` 的 payloads、gateway、manifest、cases、各 runner、mock、preflight/self-test，以及 `provider_protocol_contracts.json`、`provider_capability_baseline.json`、`logical_entrypoint_baseline.json`、`route_exposure_contracts.json`。按统一 API 内的条件场景保留覆盖，不能因 API 数量减少删掉首尾帧/多素材用例。
- [x] T1.5 的 Provider wire 期望来自固定日期/版本的官方云 API 合同，不能从 AICC 实现或 metadata 反向生成。现有 mock 通过只能证明协议映射，不代表真实推理成立；不增加私有后端协议 fixture。
- [x] **T2**：实施完成后首选 OpenRouter 对 H3/H3-Max 做真实生成验收；用户已准备 API token，实测时通过现有 Provider 凭据配置使用。凭据是否已配置、认证、具体模型权限与可用额度仍待核验，token 已准备不代表在线验收已通过。准备可访问测试素材、端点/版本证据和有界调用预算，验证 exact/helper 确实使用目标 OpenRouter 实例。MiniMax 直连 H3/H3-Max 与 Seedance 按各自具体实例独立验收，原厂直连成功不能替代 OpenRouter 渠道实测。缺少环境时保持该格“未验证”，不能用 mock 结果替代；Veo 先核实可用 endpoint。H3 私有实例不在本轮矩阵内，不影响本轮完成判定。
- [ ] **T3/交付**：Agent 通过实际工具生成并交付可下载视频，恢复长任务；Playground 完成条件输入、错误显示和产物预览/下载。检查新 SDK/命令确实进入运行镜像。

## 6. 实施顺序与验证入口

建议提交顺序：① P0 协议/迁移表与合同样例 → ② P1–P3 共享类型、素材事实和严格路由（SDK 可据冻结结构并行）→ ③ P4 H3/Seedance 与 OpenRouter 官方云协议适配、视频计费、P5 任务边界 → ④ P6 消费面和其它 adapter 清理 → ⑤ P7 分层验收。新能力只有在 metadata、adapter 与验收证据一致后才进入可路由目录。私有部署适配在首次实际接入时单独开展。

以下是**实施后的计划命令，本次没有执行**。按修改范围先跑定向检查，之后再做构建和 DV；所有命令工作目录已标注。

```bash
# buckyos/src
cargo test -p buckyos-api -- --test-threads=1
cargo test -p aicc -- --test-threads=1

# buckyos-websdk
pnpm exec jest --runInBand tests/aicc_client.test.ts
pnpm run build

# buckyos/src：同时刷新本地 WebSDK/CLI 分发包
uv run buckyos-build.py -s aicc

# buckyos/src/tools/buckyos-agent
deno check --config deno.json aicc-tool.ts
deno test --allow-read --allow-write --allow-env lib/aicc_test.ts lib/io_test.ts commands/ai_provider_test.ts

# buckyos/src/frame/desktop
pnpm check
pnpm build
pnpm exec playwright test tests/e2e/aicc-playground.spec.ts
pnpm exec playwright test --config playwright.aicc.config.ts

# buckyos/test/aicc_test
pnpm run acceptance:preflight
pnpm run acceptance:self-test
```

注意：`playwright.aicc.config.ts` 当前未包含 Playground，必须保留上面单独的 Playground 命令。T1/T1.5 需要真实 Zone/Gateway、认证和独立 Mock；具体参数沿用 [acceptance README](../test/aicc_test/acceptance/README.md)。真实模型开关、实例与预算按 runner 约定准备；本 TODO 不执行或宣称通过在线验证。涉及 Agent 命令/容器内容时，后续按根目录 AGENTS 的 `./build_aios --local-test` 开发流程验证镜像，不能用宿主机测试替代容器验收。

## 7. 官方资料核查与待落实差异

以下为 2026-10-22 查阅的外部协议依据，不代表当前仓库或目标部署已实现：

- [MiniMax H3 官方云 API](https://platform.minimax.io/docs/api-reference/video-generation-v2-create) 与[视频生成指南](https://platform.minimax.io/docs/guides/video-generation)：本轮 H3 协议依据，创建入口为 `/v2/video_generation`，通过 `content[]` 传入多模态素材，使用 `duration/resolution/ratio` 指定规格。模型、角色/组合限制、任务查询及产物交付按云端合同验证；不从本地模型或 SGLang 文档推导云端能力。
- [OpenRouter 视频模型目录合同](https://openrouter.ai/docs/api/api-reference/video-generation/list-all-video-generation-models)：提供 `/api/v1/videos/models`、canonical slug、支持规格及 `pricing_skus`。本次只读目录观察到 H3 的 `duration_seconds/reference_images` 与 Max 的分辨率相关 SKU；不能只解析现有 token 价格，也不能把文档示例的 SKU 名当唯一固定格式。
- [OpenRouter 视频调用指南](https://openrouter.ai/docs/guides/overview/multimodal/video-generation)：独立异步视频接口，包含任务提交、查询与内容下载，终态可返回 `usage.cost`。P4.1 按此合同落地；其中普通参考图与关键帧的优先级及渠道实际支持子集须明确校验，不能套用 MiniMax 直连协议。
- [Seedance 创建任务官方文档](https://docs.volcengine.com/docs/ark/create-video-generation-task-api?lang=zh)：当前同时列出 2.0/2.5；首帧、首尾帧、全模态参考为互斥场景，2.0 不支持仅音频参考。按明确模型版本登记，不能把“prompt 提示某图作为首尾帧”算作强制 keyframe 支持。
- [Veo 官方文档](https://ai.google.dev/gemini-api/docs/veo)：页面标注 Gemini API 的 Veo 3.1 Preview 于 2026-10-22 停服。保留公共协议适配性检查，但在线验收须另核对仍可用的模型/端点；本轮不据此自动扩大到其它视频产品接入。

## 8. 后续：首次 H3 私有部署接入时再开展

本节记录延期事项，不属于 P0–P7 的实施与验收范围，也不阻塞官方云协议交付。

- 取得实际部署后，重新核查所用框架、版本、模型分区、认证、素材传输和任务生命周期，再决定复用或增加 adapter；需要时依据真实差异调整 AICC 设计。当前不冻结私有协议，不预建私有能力条目。
- [MiniMax H3 官方仓库](https://github.com/MiniMax-AI/MiniMax-H3) 与 [SGLang H3 文档](https://docs.sglang.io/cookbook/diffusion/MiniMax/MiniMax-H3) 仅作为后续参考。首次接入时重新确认关键帧/参考组合、音轨选择、输入时间区间和输出规格，不沿用本次查阅版本的假定。
- 届时补齐私有实例登记、local_only 全流程隔离、任务恢复/取消及真实推理验收；推理参数和本地预处理需求按实际后端决定，不把云端完整 H3 能力直接赋给本地实例。

本次交付验证：核对 issue 正文/评论、上述源码与相关调用入口、文档链接和差异，并按讨论收敛本轮范围；仅维护本文件。尚未执行 Rust/SDK/UI 测试、构建或真实 Provider 生成，本轮实现与在线验收仍是待办；私有部署另待首次实际接入。

## 9. 实施记录（2026-10-22）

冻结设计见 [video_generation.md](../doc/aicc/video_generation.md)。改动未提交，工作区仍有其它任务的修改（catalog/tests.rs、provider/runtime.rs 等只做了必要的窄改动，未重排它们的格式）。

**落点**：公共 DTO `kernel/buckyos-api/src/aicc_video.rs`（Rust/TS 共用 fixture `tests/fixtures/video_generate_cases.json`）；AICC `src/video/`（合同、`plan_video`、需求判定、discovery 合并）、`src/media/`（MP4/MOV/WAV/MP3/图片探测与 MP4 去音轨，未引入新依赖）、`service/video_materials.rs`、`protocol/video.rs`、`protocol/openrouter_videos.rs`，以及各 codec 与 driver_metadata；WebSDK `aicc_client.ts`；Agent `lib/video.ts`、`gen_video`、新增 `edit_video`（删除 `img2video`/`video2video`）；Desktop Playground；`aigc_video` 手册与 14 份主文档。

**P5 补充的边界**：显式 `idempotency_key` 在路由前回放（不重新路由、不重复下载素材）；视频 API 一旦被 Provider 接受（含 `after_accept`）不再重试/failover；native 提交前检查恢复描述所需资源；启动恢复时无法解析的旧记录（旧 method/ApiType）被退役为 failed 并上报 TaskMgr，不阻断其它任务；OpenAI `videos/edits` 按官方 OpenAPI 只发送 `video` + `prompt`；exact 模型不满足视频需求时返回字段级 `unsupported_operation`（受限用户的路由错误原本被统一遮成 `no_candidate_model`）。

**自动化验证**：`cargo test -p aicc`（700）、`-p buckyos-api`、`-p llm_context`、`-p workflow` 全过；`cargo check --workspace --all-targets` 通过（`llm_context` 的 `AiUsage` 初始化补了 `reference_images`）；`uv run buckyos-build.py --skip-web -s aicc` 通过并同步 SDK 分发包；WebSDK jest 17 + `pnpm run build`；Agent `deno check aicc-tool.ts` + `deno test`（20）；Desktop `pnpm check`、Playground Playwright 18；`test/aicc_test` 的 `acceptance:self-test`（121）与 `acceptance:preflight`。T1/T1.5 只完成迁移和离线自检，未在真实 Zone + Mock 上运行。

**T2（OpenRouter 真实生成，`openrouter-main`，devtest，本机 DV）**：
- helper `video.generate`，限定 openrouter-main，480p 5s 文本 → 路由到 `minimax/hailuo-3-max`，约 20 s 完成，`reported_cost` 0.2475 USD，Named Object 2,278,333 字节；实测 832×480、24 fps、5.184 s、**带音轨**。同 key 重放返回原任务，同 key 不同请求为 `idempotency_conflict`。
- helper `video.generate`，H3-Max + 640×480 PNG 首帧（inline Base64）→ 640×480（比例随首帧）、5.184 s、0.2475 USD，`video_seconds=5`。
- 不发上游的拒绝：H3-Max `audio=generate`、H3 `audio=none`、H3 20 s、H3-Max 参考图 → 字段级 `unsupported_operation`；逻辑路由要 1080p → `no_candidate_model`。
- 据实测修正：OpenRouter 目录 `generate_audio=false` 改为 `unknown`（文件仍带音轨，不承诺 none/generate）；比例偏差容差 2%→3%（832×480 是编码器对齐后的 16:9）；OpenRouter 计费秒数取提交的 `duration`。
- exact `minimax/hailuo-3@openrouter-main` 文本 5s → 约 3.3 分钟完成，0.6435 USD，4,316,413 字节；实测 2560×1440（确认 2K 短边 1440）、24 fps、5.167 s、带音轨。
- T2 合计花费 1.1385 USD（三次生成，均由 OpenRouter `usage.cost` 上报并进入 `reported_cost`）。

**仍未完成**：上面保留 `[ ]` 的各项；MiniMax 直连、Seedance、Veo（Gemini API 的 Veo 3.1 Preview 已停服）没有真实验收；`./build_aios --local-test` 未运行，Agent 新命令未进入镜像；T3 未做；时间片裁剪不支持（`time_range` 必须覆盖整段，否则拒绝）；Provider URL 结果不做实测元数据。H3-Max 的音轨不是静音（ffmpeg volumedetect：首帧那条平均 −22.4 dB、峰值 −7.4 dB；文本那条 −43.4/−29.1 dB），与目录 `generate_audio=false` 不符，印证按 `unknown` 处理。三个产物已下载到 `/home/lzc/project/aicc_video_t2_2026-10-22/`。
