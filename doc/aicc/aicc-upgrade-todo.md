# AICC 新 API 与模型体系升级 TODO（历史实现记录）

> 本文记录旧实现演进过程，不再定义 Beta 2.2 重构目标。目标协议以 `aicc_api设计.md`、`aicc-new-api.md`、`provider_profile_schema.md`、`driver_metadata_schema.md` 和 `provider_architecture_durable_data_schema.md` 为准。

本文档基于 `doc/aicc/aicc-new-api.md` 的分层设计，并按当前仓库实现修订。目标不是从零设计，而是在已有第一版实现上收敛边界：把“逻辑模型路由”和“物理模型推理”真正拆开，同时继续推进模型 metadata、逻辑模型名、auto-mount 与 session profile overlay。

## 进度复核（2026-05-30，按实际代码核对）

本次复核基于对 `src/frame/aicc/src/*`、`src/kernel/buckyos-api/src/aicc_client.rs`、`src/tools/buckyos-agent/lib/aicc.ts`、`src/kernel/workflow/src/adapters/aicc.rs` 及 `src/frame/aicc/tests/*` 的逐文件核对，不再依赖旧勾选。

**底层（后端）升级状态：Phase 1–5 基本完成，可以进入 PRD/UI 阶段。**

- ✅ **Phase 1 新 API 边界**：`route.resolve` / `chat.completions.create` / `images.generate` / `helper.llm_chat` / `helper.text_to_image` 全部定义、dispatch、有 handler；`route.resolve` 拒绝 exact model 输入，typed inference 强制 `exact_model`；helper 走两阶段（resolve + typed inference），不再转发旧 all-in-one。`RouteResolveResponse` 带 `enabled/disabled_capabilities`。（`aicc_client.rs:17-21,1332-1353`，`aicc.rs:3307-3528,4286-4324`）
- ✅ **Phase 2 SDK/Workflow 迁移**：Agent SDK 有 `llmChat()/textToImage()`（透明走 helper）；workflow adapter `LLM_CHAT/IMAGE_TXT2IMG` 走 `helper_*`。唯独 **CLI 两阶段调试命令仍缺**。（`aicc.ts:145-149,280-330`，`adapters/aicc.rs:387-392`）
- ✅ **Phase 3 Metadata Resolver（历史实现记录）**：`metadata_resolver.rs`（968 行）schema 齐全；match 优先级 exact→pattern→default→conservative 正确；五个 driver（openai/claude/gemini/minimax/fal）全部接入 resolver。旧 cloud activation 描述已被 Beta 2.2 简化目标取代，目标协议见 `driver_metadata_update_protocol.md`。
- ✅ **Phase 4 Logical Definition + Auto-Mount**：`LogicalModelDefinition`（全字段）、`ModelRequirement/min_line`、`ModelDisable/disable_line`、`MountMode(manual/auto/hybrid)`、admission check、auto-mount、manual override、route trace 来源标注全部落地。（`model_types.rs:405-502,878-912`，`model_registry.rs:264-425`，`default_logical_tree.rs:455-485`）
- ✅ **Phase 5 Session Overlay**：`SessionLogicalProfile`/`LogicalTreeOverlay`/`OverlayMergeMode(inherit|replace)`、`EffectiveSessionConfig`、overlay 覆盖 disable_line、`route_policy_override` 独立、overlay trace、inherit 可 fallback / replace 失败 均有实现且有测试。（`model_session.rs:93-369`，`model_router.rs:1086-1233`）
- ❌ **Phase 6 Remote Metadata Sync：目标已简化**。NDN 承担版本发现、下载、校验、替换和目标 seq；AICC 只需 Provider applied seq 与双触发点的全局库存收敛。

**进入 PRD/UI 前仍建议收尾的小项（非阻塞）：**
- `route_trace` 仍是裸 `Value`（`RouteResolveResponse.route_trace: Option<Value>`），未提升为 typed struct（§1.2）。
- 旧 all-in-one `ai_methods::*` 仍全部 public 且未标记 legacy/deprecated，breaking-change 版本的删除/隐藏策略未定（§1.2）。
- CLI resolve/invoke/helper 调试命令缺失（§1.2 / §7.2 / Phase 2）。
- metadata 缺失/损坏/远端不可用下可启动：行为已实现但缺显式测试（§11）。

**下一步（用户思路）：底层已就绪 → 修改 AI Center PRD → 完成 UI（§7 整块，外加 §2.2 末尾文案统一、§9 UI 透出）。**

## 0. 当前实现基线

### 0.1 已落地的新 API 雏形

- `buckyos-api` 已定义新 API 方法常量：
  - `route.resolve`
  - `chat.completions.create`
  - `images.generate`
  - `helper.llm_chat`
  - `helper.text_to_image`
  - 入口：`src/kernel/buckyos-api/src/aicc_client.rs`
- `buckyos-api` 已定义第一版请求/响应结构：
  - `RouteResolveRequest` / `RouteResolveResponse`
  - `LlmChatInvokeRequest` / `LlmChatInvokeResponse`
  - `TextToImageInvokeRequest` / `TextToImageInvokeResponse`
- `AiccServerHandler` 已能 dispatch 上述新方法。
- `AIComputeCenter` 已实现：
  - `resolve_route()`
  - `create_chat_completion()`
  - `generate_image()`
  - `handle_route_resolve()`
  - `handle_chat_completions_create()`
  - `handle_images_generate()`
  - 入口：`src/frame/aicc/src/aicc.rs`

### 0.2 当前语义差距

- 新 typed inference API 目前仍转换成旧 `AiMethodRequest`，再走 `complete_with_method()`。
  - 好处：复用现有任务、事件、provider、usage、resource 处理。
  - 风险：新 API 的“数据面只接受 exact model、不做逻辑路由”边界仍依赖转换层约束。
- `chat.completions.create` / `images.generate` 会先用 `ExactModelName::parse()` 校验 `exact_model`，并设置 `allow_fallback=false`、`runtime_failover=false`。
  - 这是正确方向。
  - 仍需补测试，确保 exact model 不会因内部 route exact fallback 或 runtime failover 发生隐式切换。
- `route.resolve` 已返回：
  - `selected_exact_model`
  - `provider_instance_name`
  - `provider_driver`
  - `provider_model_id`
  - `provider_options`
  - `fallback_attempts`
  - `route_trace`
  - `inventory_revision`
  - `session_config_revision`
  - 但尚未返回 `enabled_capabilities` / `disabled_capabilities`。
- `provider_options` 目前由 `lower_provider_model_options()` 根据 `provider_model_id` 中的 `:variant` 临时推导。
  - 这只适合作为过渡实现。
  - 最终应由 driver metadata / variant resolver 产生。
- `helper.llm_chat` / `helper.text_to_image` 当前在 service handler 内直接转发到旧 all-in-one 方法。
  - 这与设计里的“helper 是 route.resolve + typed inference 的组合层”还不一致。
- 旧 all-in-one `ai_methods::*` 仍是公开 service 方法，且 `src/tools/buckyos-agent/lib/aicc.ts` 仍使用旧请求形态。
  - breaking change 版本需要给它们明确 legacy/helper 定位。

### 0.3 当前模型与路由实现

- AICC 已有模型级 `ModelMetadata` / `ProviderInventory`：
  - `provider_model_id`
  - `exact_model`
  - `api_types`
  - `logical_mounts`
  - `capabilities`
  - `attributes`
  - `pricing`
  - `health`
  - 入口：`src/frame/aicc/src/model_types.rs`
- `ModelRegistry` 已按 inventory 构建 exact index，并能从 `logical_mounts` 生成默认 logical items。
- `ModelRouter` 已支持：
  - logical path 展开
  - exact model 解析
  - fallback chain
  - hard filters：api type、health、quota、required features、local only、allow/blocked provider、cost、latency、weight
  - route trace
- `ModelScheduler` 已负责候选排序、sticky binding、scheduler profile。
- `SessionConfig` 已支持：
  - global logical tree
  - per-session store
  - `items` 覆盖
  - `item_overrides`
  - exact model weights
  - policy merge / lock
  - 但还没有显式 `SessionLogicalProfile` / `LogicalTreeOverlay` / `merge_mode=inherit|replace` 结构。
- `default_logical_tree.rs` 已有内置二级逻辑目录，例如 `llm.plan`、`llm.code`、`llm.swift`、`llm.reason`。
  - 当前主要是静态模板 + provider inventory `logical_mounts`。
  - 还不是基于 `LogicalModelDefinition.min_line` 的 admission / auto-mount。

### 0.4 当前 provider metadata 状态

- OpenAI 当前仍以 `/models` 或配置模型列表为主，靠代码规则生成 metadata 和 mounts。
  - `default_features()` 仍偏乐观，默认包含 `plan/json_output/tool_calling/web_search`。
  - OpenAI GPT 分层、latest mount、音频/图像排除等已在代码里有规则，但不是独立 driver metadata。
- Claude 已有更细的 per-model classifier，会按模型名修剪 `plan/web_search/vision/context/output` 等能力。
  - 方向接近 driver metadata resolver，但仍写死在 provider adapter 内。
- Gemini / Minimax / Fal 也都有各自代码内 metadata 构造逻辑。
- 尚未有本地 per-driver metadata 文件、metadata resolver、remote sync cache、签名校验。

## 1. API 分层升级

### 1.1 目标语义

- `route.resolve` 是控制面：
  - 输入逻辑模型名和本次请求的 routing constraints。
  - 输出确定的 AICC exact model、provider 信息、provider options、fallback candidates 和 trace。
- typed inference API 是数据面：
  - 只接受 `exact_model`。
  - 不接受逻辑模型名。
  - 不做逻辑 fallback。
  - 不隐式重新 route。
- helper 是客户端空间组合：
  - `logical request -> route.resolve -> typed inference`
  - helper 不拥有独立路由逻辑。

### 1.2 TODO

- [x] 定义 `route.resolve` / `chat.completions.create` / `images.generate` 方法名。
- [x] 定义第一版 Route / LLM Chat / Text-to-Image 请求响应结构。
- [x] AICC service dispatch 新方法。
- [x] typed inference 转换层校验 `exact_model`。
- [x] typed inference 默认关闭 `allow_fallback` 和 `runtime_failover`。
- [x] 在协议文档中明确旧 `llm.chat`、`image.txt2img` 等 all-in-one 方法进入 legacy/helper 兼容层，不再作为 AICC 本体核心语义。
- [ ] 明确 breaking change 后旧 all-in-one 方法的删除、保留或隐藏策略。
- [x] `route.resolve` 应禁止 `logical_model` 传 exact model；如需 exact 诊断，另设字段或另设方法，避免控制面语义混乱。
- [x] `RouteResolveResponse` 增加 `enabled_capabilities` / `disabled_capabilities`，表达本次 route 后实际启用/禁用的能力集合。
- [x] 明确 `fallback_attempts` 语义：
  - 是 route 建议候选顺序，不是 lease。
  - 是否受 `runtime_failover` 和 `fallback_limit` 限制。
  - 是否包含 primary 之外所有同分候选、scheduler 后候选，还是只包含运行时 failover 候选。
- [ ] `route_trace` 使用稳定结构而不是裸 `Value`，至少在 Rust API 层保留 typed struct，外部序列化为 JSON。
- [x] `provider_options` 的来源从 `lower_provider_model_options()` 迁移到 driver metadata / variant resolver。
- [x] typed inference 内部不再经过“逻辑 route”路径；短期可继续复用 `complete_with_method()`，但必须用测试锁定 exact-only 行为。
- [x] 如果 `helper.*` 继续保留在 AICC service，改为显式调用 `resolve_route()` + typed inference，而不是直接转发旧 all-in-one 方法。
- [ ] Agent SDK / Web SDK / CLI 提供 helper API，并把默认调用迁移到 helper 或显式两阶段调用。

### 1.3 必补测试

- [x] `route.resolve` 输入 `llm.chat` 返回 `selected_exact_model`、provider 信息、trace。
- [x] `route.resolve` 输入 exact model 被拒绝，错误码明确。
- [x] `chat.completions.create` 输入逻辑模型名被拒绝。
- [x] `images.generate` 输入逻辑模型名被拒绝。
- [x] typed inference 的 primary exact model quota exhausted / unavailable 时，不 fallback 到其它模型。
- [x] typed inference 失败后，由调用方重新 `route.resolve` 才能换模型。
- [x] `helper.llm_chat` 展开后的行为等价于 `route.resolve + chat.completions.create`。
- [x] `helper.text_to_image` 展开后的行为等价于 `route.resolve + images.generate`。

## 2. 命名与请求约束收敛

### 2.1 当前实现

- `ModelCapabilities` 是模型能力真相源。
- `Requirements` 已包含结构化 `ModelRequirement`，同时保留 `must_features` 字符串兼容层。
- `ModelDisable` 已存在，并且 `apply_disabled_capabilities()` 会同时处理结构化 `disable` 和 legacy `requirements.extra.disable_capabilities`。
- `RoutePolicy` 已表达 profile、local only、fallback、runtime failover、allowed/blocked provider、cost、latency。

### 2.2 TODO

- [x] 保留 `must_features -> RequiredModelFeatures` 转换。
- [x] 支持结构化 `disable`。
- [x] legacy `disable_capabilities` 已能映射到结构化禁用逻辑。
- [x] 文档明确 `Feature` 不是 inventory 真相源，只是旧请求兼容表达。
- [x] 新逻辑禁止继续依赖 `ProviderInstance.features` 做 capability 判断。
- [ ] `requirements` 只表达硬能力约束。
- [ ] `disable` 只表达本次必须关闭的能力。
- [ ] `options` 只表达本次 provider request lowering 参数。
- [ ] `policy` 只表达路由策略，不再承担“禁用能力”的表达。
- [ ] UI / AI Center 文案避免 feature/capability 混用，统一展示为：
  - 模型能力
  - 路由要求
  - 本次禁用
  - 本次启用
  - 路由策略

## 3. Driver Metadata Resolver

### 3.1 目标

Provider 自发现只负责发现 provider model id。AICC 通过 driver metadata resolver 把 provider model id 转成 AICC model metadata。

目标流程：

```text
provider /models reported ids
+ builtin driver metadata
+ local override / system-config override
+ optional remote cache
+ provider instance runtime state
= final ProviderInventory.models
```

### 3.2 TODO

- [x] 定义 driver metadata schema 文档。
- [x] 按 driver 拆分本地 metadata 文件：
  - `openai.json`
  - `claude.json`
  - `gemini.json`
  - `fal.json`
  - `minimax.json`
- [x] schema 至少包含：
  - `schema_version`
  - `provider_driver`
  - `revision`
  - `models`
  - `patterns`
  - `defaults`
  - `variants`
  - `signature`
- [x] 明确匹配优先级：
  - exact `models`
  - `patterns`
  - `defaults`
  - conservative fallback
- [x] 明确 override 优先级：
  - builtin
  - remote cache
  - local override
  - system-config override
- [x] 新增 `metadata_resolver` 模块。
- [x] OpenAI inventory 改为 `/models` + resolver。
- [x] Claude classifier 收编为 resolver 规则或 metadata。
- [x] Gemini / Minimax / Fal 迁移到 resolver。
- [x] unknown model fallback 不默认声明高风险能力：
  - 不默认 tool_call
  - 不默认 web_search
  - 不默认 vision
  - 不默认 json_schema
- [x] metadata 文件缺失、损坏时 AICC 仍能以 conservative fallback 启动。

## 4. Reasoning Variant 与 Provider Options Lowering

### 4.1 目标语义

对用户来说，“同一 base model + 不同 reasoning effort”应表现为不同 AICC exact model，而不是普通请求参数。

示例：

```text
gpt-5.1:reasoning-high@openai-primary
```

route output lowering：

```text
provider_model_id = gpt-5.1
provider_options.reasoning.effort = high
```

### 4.2 当前实现

- `lower_provider_model_options()` 已能把 `provider_model_id` 中的 `:reasoning-*` 转成 `provider_options.reasoning.effort`。
- 该 lowering 主要用于 `RouteResolveResponse`。
- typed inference 仍依赖调用方把 `provider_options` 传入第二段请求；provider adapter 本身尚未统一处理 AICC variant exact model。

### 4.3 TODO

- [x] 在 driver metadata schema 中定义 `variants`。
- [x] OpenAI metadata 中用 variants 表达 reasoning effort 档位。
- [x] AICC exact model 使用 variant 后的 model id。
- [x] route.resolve 输出 base provider model id + provider_options。
- [x] typed inference 若收到 variant exact model，应能按 metadata 自动 lower；不应要求调用方必须手动补 provider_options。
- [x] provider adapter 调用前统一把 AICC variant 还原成 provider base model + provider options。
- [x] 用户传入的 provider_options 与 route provider_options 的 merge 规则放在 helper 层，不放在数据面协议里。
- [x] usage / trace / audit 使用 AICC exact model 聚合，避免不同 reasoning 档位混在一起。
- [x] audit 额外保留 provider actual model 和 provider options，便于复现。

## 5. Logical Model Definition 与 Auto-Mount

### 5.1 当前实现

- 当前 logical mount 主要来自 `ModelMetadata.logical_mounts`。
- `ModelRegistry::default_items_for_path()` 会把 inventory 中同名 mount 的 exact models 生成默认 items。
- `default_logical_tree.rs` 提供内置二级目录模板。
- 尚未有独立 `LogicalModelDefinition`，也没有基于 `min_line` 的 admission check。

### 5.2 目标结构

```text
LogicalModelDefinition
  path
  api_type
  min_line
  disable_line
  default_options
  mount_mode
  scheduler_profile
  fallback
  route_policy
  user_visible_tier
```

`min_line` 是 hard gate，只决定模型是否能挂载到该逻辑模型名。

`disable_line` 是能力禁用约束，影响本次 request lowering 和 trace，不表达 fallback / provider 范围。

### 5.3 TODO

- [x] 定义 `LogicalModelDefinition` schema。
- [x] 定义 `ModelRequirement` / `min_line` schema。
- [x] 定义 `ModelDisable` / `disable_line` schema，字段集合与 `ModelRequirement` 对称。
- [x] 定义 `mount_mode`：
  - `manual`
  - `auto`
  - `hybrid`
- [x] 实现 admission check：`ModelMetadata.capabilities` 是否满足 `min_line`。
- [x] 实现 auto-mount：扫描 `ProviderInventory.models`，满足 min_line 的模型自动生成 logical items。
- [x] manual override 可以覆盖 auto item 的 weight / enabled / blocked。
- [x] `default_logical_tree.rs` 从静态 item 模板逐步迁移为 logical definition + scheduler profile。
- [x] route trace 记录 item 来源：
  - builtin definition
  - driver metadata mount
  - auto admission
  - manual override
  - session overlay
- [x] route trace 解释模型不满足 min_line 的原因。
- [x] route trace 解释能力被 disable_line 禁用的原因。

## 6. Session Profile Overlay

### 6.1 当前实现

- `SessionConfigStore` 已支持按 `session_id` 存储 session config。
- `LogicalNode.items` 可以表达 replace。
- `LogicalNode.item_overrides` 可以表达 inherit-style patch。
- `exact_model_weights` 可以表达偏好权重。
- policy merge / lock 已存在。
- 当前结构还没有显式 `merge_mode`，也没有 session profile 名称、overlay 来源 trace、disable override。

### 6.2 目标语义

```text
Base Logical Tree
+ Session Profile Layer
= Effective Logical Tree
```

两种核心模式：

- `inherit`：优先某个模型，但保留原候选和 fallback。
- `replace`：Only 模式，只保留指定模型；不可用时失败。

### 6.3 TODO

- [x] 定义 `SessionLogicalProfile` schema。
- [x] 定义 `LogicalTreeOverlay` schema。
- [x] 明确 `merge_mode=inherit|replace` 与现有 `item_overrides|items` 的映射关系。
- [x] 路由前生成 `EffectiveSessionConfig`。
- [x] Router / Scheduler 只看 effective config，不关心 overlay 来源。
- [x] Provider adapter 只看最终 exact model，不关心 session profile。
- [x] overlay 可以覆盖 `disable_line`。
- [x] route policy 覆盖必须走 `route_policy_override`，不能混入 disable。
- [x] trace 记录 overlay 来源：
  - `logical_profile_scope=session`
  - `overlay_path`
  - `merge_mode`
  - `selected_from_overlay`
- [x] `inherit` 模式下指定模型 quota exhausted 可以 fallback。
- [x] `replace` 模式下指定模型 quota exhausted 直接失败。
- [x] 第一版可不支持从 inherited view 中删除单个 inherited item；Only 用 `replace`。

## 7. UI / Agent / Workflow 迁移

### 7.1 当前实现

- `src/tools/buckyos-agent/lib/aicc.ts` 仍构造旧 `AiMethodRequest`，通过旧 method 调用 AICC。
- workflow adapter 位于 `src/kernel/workflow/src/adapters/aicc.rs`，需要检查是否仍直接依赖旧 all-in-one 形态。
- AI Center / control panel 侧有 AICC settings，但 routing UI 尚未围绕新 logical model definition 展示。

### 7.2 TODO

- [x] Agent SDK 新增 helper：
  - `llmChat()`
  - `textToImage()`
  - 内部执行 `route.resolve + typed inference`。
- [ ] CLI tool 新增显式两阶段调试命令：
  - resolve route
  - invoke exact model
  - helper call
- [x] workflow adapter 使用 helper 或显式两阶段调用，不直接把逻辑模型名传给数据面。
- [ ] AI Center Routing UI 围绕逻辑模型名展示，不围绕 provider 参数面板展示。
- [ ] 每个逻辑模型名展示：
  - min_line
  - disable_line
  - mount mode
  - candidate pool
  - scheduler profile
  - fallback
  - active session overlay
- [ ] 手工指定模型时，配置层使用 logical tree overlay / manual item，不直接改请求参数。
- [ ] 配置时检测指定模型是否满足该逻辑模型名 min_line。
- [ ] 不满足时给出明确警告或拒绝。

## 8. Remote Metadata Sync

### 8.1 目标

避免为了更新 provider model metadata 频繁发 BuckyOS 版本，同时不在 AICC 中复制 NDN 的文件更新能力。

### 8.2 TODO

- [ ] 接入 NDN 当前 metadata 文件集合和 `metadata_target_seq`。
- [ ] 每个 Provider inventory 持久化 `metadata_applied_seq`，刷新前临时捕获 `metadata_updating_seq`，成功后才提交。
- [ ] 推理进入路由前检查所有 Provider 的 applied/target seq 并执行全局收敛。
- [ ] 任一 Provider Instance 定时库存刷新前执行相同检查；列表未变化且 seq 相同时仅探测。
- [ ] 并发触发合并为一次刷新；不得按请求、Provider 或模型做局部 metadata 更新。
- [ ] 删除 AICC 旧 remote cache、signature、activation、LKGS、水位和回滚目标。

## 9. Trace / Usage / Audit

### 9.1 当前实现

- `RouteTrace` 已记录 request、session、api type、requested model、candidate count、filtered candidates、ranked candidates、fallback chain、selected exact model、scheduler profile、user summary。
- `route.resolve` 已把 trace 序列化到响应中。
- task result extra 中已有 route_trace 透出路径。

### 9.2 TODO

- [ ] Route trace 记录 requested logical path，且区分 legacy all-in-one / route.resolve / helper 来源。
- [ ] Route trace 记录 effective logical tree 来源：
  - base
  - driver metadata
  - auto-mount
  - manual
  - session overlay
- [ ] Route trace 记录 selected AICC exact model。
- [ ] Route trace 记录 provider actual model。
- [ ] Route trace 记录 provider options derived from variant。
- [ ] Route trace 记录 enabled / disabled capabilities。
- [ ] Route trace 记录 helper options merge 结果。
- [ ] Usage 按 AICC exact model 聚合。
- [ ] Audit 保留 provider actual model、provider options、inventory revision、session config revision。

## 10. 实施顺序建议

### Phase 1: 新 API 边界加固

- [x] 补协议文档，明确 route control plane / inference data plane / helper 三层。
- [x] 补 typed inference exact-only 测试。
- [x] `route.resolve` 禁止 exact model 输入。
- [x] `RouteResolveResponse` 补 capabilities 字段。
- [x] 明确并测试 `fallback_attempts` 语义。
- [x] `helper.*` 改为两阶段组合或从 service 核心协议中移出。

### Phase 2: SDK / Workflow 迁移

- [x] TypeScript Agent SDK 增加新 helper。
- [x] buckyos-agent 默认走 helper。
- [x] workflow adapter 迁移到 helper 或显式两阶段调用。
- [ ] CLI 增加 resolve / invoke / helper 调试命令。（唯一未完成项）

### Phase 3: Metadata Resolver（已完成，远端同步除外）

- [x] 定义 driver metadata schema。
- [x] 新增 metadata resolver。（`metadata_resolver.rs`）
- [x] OpenAI 接入 resolver。
- [x] Claude classifier 收编到 resolver。
- [x] unknown model conservative fallback。（有测试 `openai_unknown_fallback_is_conservative`）

### Phase 4: Logical Definition + Auto-Mount（已完成）

- [x] 定义 logical model schema。
- [x] 实现 min_line admission。
- [x] 实现 mount_mode。
- [x] `default_logical_tree` 迁移到 definition。
- [x] route trace 展示 admission 和 auto-mount 结果。

### Phase 5: Session Overlay（已完成）

- [x] 定义显式 overlay schema。
- [x] 支持 inherit / replace。
- [x] 接入 session profile。
- [x] trace 展示 overlay 生效情况。

### Phase 6: Remote Sync

- [ ] NDN 文件替换后的 `metadata_target_seq` 接入。
- [ ] 推理前/Provider 定时库存刷新时统一收敛全部 seq 落后的 Provider。
- [ ] 为每个 Provider 库存刷新循环增加停止事件；停止、禁用、删除、替换实例及服务退出时发送 `Stop` 并等待优雅退出，禁止迟到写入。
- [ ] 管理开关和诊断日志。

## 11. 最小验证集

- [x] `route.resolve(llm.chat)` 能返回 exact model、provider 信息、fallback attempts、route trace。
- [x] `route.resolve(exact_model)` 被拒绝。
- [x] `chat.completions.create(exact_model)` 能跑通。
- [x] `chat.completions.create(logical_model)` 被拒绝。
- [x] exact model unavailable 时 typed inference 不 fallback。
- [x] helper `llm_chat` 行为等价于 route + typed inference。
- [x] OpenAI `/models` 返回 `gpt-5.1`，resolver 展开 base model 和 reasoning variants。
- [x] `gpt-5.1:reasoning-high@openai` 推理前还原成 `model=gpt-5.1` + `reasoning.effort=high`。
- [x] `llm.plan` 的 min_line 能过滤掉不满足 tool_call / json_schema / min_context 的模型。
- [x] auto-mount 能把满足 `llm.chat` min_line 的多个 provider 模型挂入候选池。
- [x] session inherit overlay 能提高指定模型权重，并在 quota exhausted 后 fallback。
- [x] session replace overlay 只保留指定模型，并在 quota exhausted 后失败。
- [x] unknown model 不默认声明高风险能力。（`metadata_resolver.rs` conservative fallback + 测试）
- [~] metadata 文件缺失、损坏、远端不可用时 AICC 可启动。（行为已实现：缺失/损坏 `warn` 跳过退回 builtin；缺显式启动测试）
- [x] route trace 能解释 base tree、auto-mount、session overlay 和 provider lowering。（`model_router.rs` LogicalItemSourceTrace / LogicalAdmissionTrace / SessionOverlayTrace + variant lowering）
