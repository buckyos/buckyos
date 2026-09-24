# AICC Models Manager 后续 TODO（历史实现记录）

> 本文保留已完成任务的历史记录，不作为 Beta 2.2 Provider 架构重构规范。目标模型边界以 `aicc-models-mgr.md`、`provider_profile_schema.md` 和 `driver_metadata_schema.md` 为准。

来源：`doc/aicc/aicc-models-mgr.md`  
状态：Draft  
范围：AICC Models Manager、模型目录、模型路由、Provider inventory、driver metadata、Desktop 展示与配置入口。

## 0. 已完成基线

- [x] `provider_weights` 成为 `SessionConfig` 的一等字段。
  - 文件：`src/frame/aicc/src/model_session.rs`
  - 语义：`1.0` 默认，`0.0` 禁用 provider，`0.0 < weight < 1.0` 降低整体偏好，`weight > 1.0` 提高整体偏好。
- [x] Router 将 `provider_weights` 写入候选，并对 `provider_weight <= 0` 做 hard filter。
  - 文件：`src/frame/aicc/src/model_router.rs`
  - Trace drop reason：`provider_weight_zero`
- [x] Scheduler 将 provider weight 纳入 `preference` 维度。
  - 文件：`src/frame/aicc/src/model_scheduler.rs`
  - 当前算法：`preference_penalty = 1 / (exact_model_weight * provider_weight)`
- [x] Ranked trace 暴露 `provider_weight`。
  - 文件：`src/frame/aicc/src/model_types.rs`
- [x] Desktop API 和 mock 类型同步 `provider_weights`。
  - 文件：`src/frame/desktop/src/api/aicc_mgr.ts`
  - 文件：`src/frame/desktop/src/app/ai-center/mock/types.ts`
  - 文件：`src/frame/desktop/src/app/ai-center/mock/seed.ts`
- [x] 已验证：
  - `cargo test -p aicc`
  - `pnpm check` in `src/frame/desktop`
  - `git diff --check`

## 1. P0：保持路由语义可解释

- [x] 为 `provider_weights` 增加 route trace 独立解释项。
  - 当前 ranked candidate 只有 `provider_weight` 数值。
  - 目标：用户能区分“provider weight 影响”和“exact model weight 影响”。
  - 建议新增 trace 字段：
    - `provider_weight_sources`
    - 或在 `RankedCandidateTrace` 中增加 `preference_score_inputs`
  - 已实现：`RankedCandidateTrace.preference_score_inputs` 独立展示 `exact_model_weight`、`provider_weight`、combined weight、preference penalty 与 up/down/neutral/disabled effect。
  - 验证：
    - provider weight 为 `0.3` 时 trace 能展示该 provider 被降权。
    - provider weight 为 `0.0` 时 hard filter reason 为 `provider_weight_zero`。

- [x] 明确 provider weight 与 exact model weight 的优先级关系。
  - 当前 `select_highest_priority()` 仍只比较 `priority_path` 与 `exact_model_weight`，provider weight 只进入 scheduler。
  - 需要确认这是否符合最终语义：
    - 方案 A：provider weight 只影响同一最高优先级集合内 scheduler。
    - 方案 B：provider weight 也参与最高优先级集合筛选。
  - 推荐先保持方案 A，并在文档中明确。
  - 已实现：保持方案 A，并在 `doc/aicc/aicc-models-mgr.md` 的权重语义和候选集合筛选段落中明确。

- [x] 补充 `models.list` / Desktop route trace 展示。
  - 当前 Desktop 类型已经能接收 `provider_weights`，但 UI 未提供显式编辑和解释。
  - 目标：
    - Provider 详情页展示当前 provider weight。
    - Routing trace 展示 candidate 的 `exact_model_weight` 与 `provider_weight`。
    - 用户能看出为什么某 provider 被禁用或降权。
  - 已实现：Desktop 类型同步 `preference_score_inputs`，Provider 详情页展示当前 routing weight，Routing trace 与首页 trace 摘要展示 exact/provider/combined weights。

## 2. P0：Provider 发现边界收敛

- [x] 清理 Provider 侧直接挂用途目录的逻辑假设。
  - 目标边界：
    - Provider 只发现物理模型。
    - Driver metadata 负责能力、家族、默认挂载。
    - Registry 负责 admission。
    - Session/User overlay 负责偏好。
  - 排查入口：
    - `src/frame/aicc/src/openai.rs`
    - `src/frame/aicc/src/claude.rs`
    - `src/frame/aicc/src/gemini.rs`
    - `src/frame/aicc/src/minimax.rs`
    - `src/frame/aicc/src/fal.rs`
  - 注意：不要一次性删除所有 role mounts，应先补测试确认用途目录仍可通过家族目录路由。
  - 已实现：
    - Gemini / MiniMax / fal provider inventory 构造不再通过 fallback mounts 主动写逻辑目录。
    - Claude 远端 provider inventory 归一化改为通过 driver metadata 重新解析 logical mounts，并只合并动态 pricing / health。
    - OpenAI 远端 provider inventory 不再保留 provider 返回的 logical mounts，避免 provider 成为目录真相源。
    - 新增测试确认 `llm.plan` 可经 driver-owned family mount `llm.gemini-pro` 路由。

- [x] 区分 provider fallback hints 与 driver-owned logical mounts。
  - Provider 可以提供 raw hints，但不能成为 `llm.plan`、`llm.chat` 等用途目录的真相源。
  - 需要确定 `DriverModelResolveRequest.fallback_logical_mounts` 的长期边界：
    - 只用于未知 driver 的保守 fallback。
    - 或允许 provider override，但必须标记 source。
  - 已实现：
    - `fallback_logical_mounts` 只在没有匹配到 driver rule 时参与保守 fallback，不再覆盖 driver-owned logical mounts。
    - 对未知 driver 的 fallback hints 会过滤 `llm.plan`、`llm.chat` 等 role path，只保留非用途目录 hint。
    - 新增 resolver 单测覆盖 known driver 忽略 provider fallback mounts、unknown driver 过滤 role path。

## 3. P1：家族目录成为主要 driver mount 目标

- [x] 调整 OpenAI GPT 的默认挂载策略。
  - 当前 OpenAI post rule 仍会给最新 GPT 直接增加部分用途目录 mount，例如 `llm.plan`、`llm.code`、`llm.reason`。
  - 目标：
    - current family mount：`llm.gpt-standard`、`llm.gpt-pro`、`llm.gpt-mini`、`llm.gpt-nano`
    - version index mount：`llm.openai.gpt-5-2`
    - 用途目录通过 `default_logical_tree.rs` 引用 family mount。
  - 风险：
    - 会影响现有 `openai::*latest_gpt_mounts*` 测试。
    - 需要同步 Desktop 目录树展示。
  - 已实现：
    - OpenAI / SN-AI GPT latest post rule 只给最新 tier 模型增加 `llm.gpt-standard`、`llm.gpt-pro`、`llm.gpt-mini`、`llm.gpt-nano` family mount，不再增加 `llm.plan`、`llm.code`、`llm.reason`、`llm.swift`、`llm.summarize`。
    - 版本索引继续保留 `llm.openai.<model>`。
    - Desktop mock 目录树示例改为展示 family mount。

- [x] 梳理 Claude family mount。
  - 目标 current family mount：
    - `llm.opus`
    - `llm.sonnet`
    - `llm.haiku`
  - version index mount 示例：
    - `llm.anthropic.claude-opus-4-7`
    - `llm.anthropic.claude-sonnet-4-6`
  - 检查 vision/tool/json 能力不要从 family 名称倒推。
  - 已实现：
    - Claude driver metadata 使用 `llm.opus`、`llm.sonnet`、`llm.haiku` 和 `llm.anthropic.<model>`，移除 LLM 用途目录直接挂载。
    - 保留 vision 相关非 LLM 方法挂载，能力仍来自 metadata capability 字段。

- [x] 梳理 Gemini family mount。
  - 目标 current family mount：
    - `llm.gemini-pro`
    - `llm.gemini-flash`
    - `llm.gemini-flash-lite`
    - `llm.gemini-deepthink`
  - 需要沿用已有 Gemini 版本保留/alias 逻辑，避免 versioned model 与 alias 混乱。
  - 已实现：
    - Gemini driver metadata 使用 `llm.gemini-pro`、`llm.gemini-flash`、`llm.gemini-flash-lite`、`llm.gemini-deepthink` family mount，移除 LLM `llm.chat` 直接挂载。
    - 保留已有 `llm.gemini.<model>` 版本/alias 索引样式。

- [x] 梳理国产/其它模型 family mount。
  - 目标 family mount 示例：
    - `llm.qwen-max`
    - `llm.qwen-coder`
    - `llm.qwen-small`
    - `llm.deepseek-pro`
    - `llm.deepseek-reasoner`
    - `llm.kimi`
    - `llm.kimi-thinking`
    - `llm.glm`
    - `llm.glm-flash`
    - `llm.grok-fast`
    - `llm.grok-heavy`
  - 已实现：
    - Resolver 增加轻量语义 family classifier，覆盖 qwen/deepseek/kimi/glm/grok 示例 family mount。
    - OpenAI-compatible / SN provider inventory 归一化复用同一 family classifier。

## 4. P1：版本排序规则 driver 化

- [x] 抽象 driver post rule。
  - 当前 OpenAI 版本排序逻辑是 Rust 代码里的专用 post rule。
  - 目标：把 family/tier/version/stability 的规则逐步 driver 化。
  - 先不要引入新依赖；如果要引入 regex/表达式解析依赖，必须单独确认。
  - 已实现：resolver 新增通用 `version_rules` post rule，OpenAI / SN-AI 不再通过 provider_driver 分支写死 GPT latest mount。

- [x] 定义 driver metadata 的版本规则字段。
  - 草案字段：
    - `family`
    - `tier`
    - `version_rank`
    - `stability`
    - `current_mount`
    - `version_mount`
  - 需要兼容现有 `models`、`patterns`、`defaults`、`variants` schema。
  - 如果 schema 版本变化，必须同步：
    - `metadata_resolver.rs`
    - builtin driver metadata JSON
    - override schema 校验
    - 文档
  - 已实现：schema version 保持 1，新增向后兼容的 `version_rules` 字段；内置 `openai.json` 已迁移 GPT standard/pro/mini/nano 规则；文档已同步说明当前字段。

- [x] 明确 preview/beta/experimental 排序。
  - stable 应优先于 preview/beta/experimental。
  - preview 可以进入 version index，但默认不应成为 current family mount，除非 driver 明确允许。
  - 已实现：OpenAI `version_rules.stability.current_requires_stable=true`，`preview` / `beta` / `experimental` 只保留 version index，不写 current family mount。

- [x] 明确 variant 不参与 base version 排序。
  - reasoning variant 应在 base model 选出 current mount 后展开。
  - variant exact model 应保留独立 identity：
    - `<base_model>:reasoning-high@provider`
  - provider call lowering 继续使用：
    - `provider_actual_model_id`
    - `provider_options`
  - 已实现：resolver 先对 base models 应用 `version_rules`，再展开 `variants`；variant exact model 继续通过 `provider_actual_model_id` / `provider_options` lowering。

## 5. P1：用户配置持久化与控制入口

- [x] 确定 `provider_weights` 的长期持久化位置。
  - 候选位置：
    - `services/aicc/settings`
    - `services/control_panel/ai_models/provider_overrides`
    - 新增 AICC model route config key
  - 要求：
    - 不污染 provider inventory。
    - 不修改 driver metadata。
    - 能作为 global/session parent 配置。
  - 已实现：使用 `services/aicc/settings.session_config.provider_weights`，AICC `reload_settings` 会把该配置合并到默认 global session config。

- [x] Control Panel 增加 provider weight 配置读写。
  - 当前 control_panel 有 provider overrides、policies、model catalog 等 key。
  - 需要设计：
    - provider weight 字段位置。
    - 默认值如何返回。
    - 保存时如何校验 provider instance name 与 weight。
  - 已实现：新增 `ai.provider.weight.list` / `ai.provider.weight.set`，默认权重按 `1.0` 返回；保存时校验 provider instance name 已存在于 AICC settings，weight 为非负有限数，保存后触发 `ai.reload`。

- [x] Desktop Provider 管理 UI 增加 weight 编辑。
  - 控件建议：
    - slider 或 number input。
    - `0.0` 显示为 disabled for routing。
    - `1.0` 显示为 default。
  - 不要把 provider weight 展开成一堆 exact model weight 展示给用户。
  - 已实现：Provider 详情页增加 routing weight slider + number input，通过 Control Panel 保存 provider instance 权重。

- [x] Session patch API 示例补齐。
  - 示例：
    ```json
    {
      "session_config_patch": {
        "provider_weights": {
          "openai-backup": 0.3,
          "local-llama": 2.0
        }
      }
    }
    ```
  - 已实现：`doc/aicc/maintenance/krpc_aicc_calling_guide.md` 增加 request 级 `session_config_patch.provider_weights` 示例。

## 6. P1：成本语义拆分

- [ ] 区分 billing cost 与 routing cost override。
  - 当前 `ModelPricing.estimated_cost_usd` 同时容易被用于展示和路由。
  - 目标：
    - billing cost：真实计费、审计、展示。
    - routing cost override：只影响 scheduler 打分。
  - 可能新增字段：
    - `ModelPricing.routing_estimated_cost_usd`
    - 或 session/user overlay 中增加 cost override map。

- [ ] Route trace 展示实际使用的 cost 来源。
  - 来源候选：
    - provider pricing
    - dynamic cost estimate
    - user routing override
    - fallback class estimate

## 7. P2：远程 driver metadata 更新

- [ ] 按 Beta 2.2 简化目标替换旧 remote cache/activation 实现。
  - 云端按客户端版本/通道/灰度分组配置兼容发布；NDN 负责版本选择、下载、校验、防回退和当前 metadata 文件替换，保证不足时向 NDN 提交 bug。
  - 文件替换后由 NDN 推进全局 `metadata_target_seq`，每个 Provider inventory 保存 `metadata_applied_seq`。
  - 下一次推理前或任一 Provider Instance 定时库存刷新时，统一收敛所有 applied seq 落后的 Provider，成功后才提交该 Provider 的新序列。
  - model 列表未变化且 seq 相同时仅探测、不重写库存；禁止按本次推理或当前 Provider 只更新部分落后库存。
  - AICC 不再维护 signature 校验、remote cache、candidate、activation、LKGS 或 revision 回滚流程。

## 8. P2：空目录 auto admission 与 UI 可解释性

- [ ] 在 `models.list` 中返回 logical item source 与 admission trace。
  - 当前 route trace 中有：
    - `logical_item_sources`
    - `logical_admission`
  - 但目录列表视图未必能稳定展示这些 source。
  - 目标：UI 能显示候选来自：
    - `driver_metadata_mount`
    - `auto_admission`
    - `session_overlay`
    - `manual_override`

- [ ] Desktop 目录树展示 auto admission 来源。
  - 用户需要能理解“没有手动挂载，为什么模型仍出现在目录里”。
  - 对 rejected admission 展示原因：
    - `api_type_mismatch`
    - `tool_call`
    - `json_schema`
    - `vision`
    - `min_context_tokens:<n>`

## 9. P2：文档与测试补齐

- [ ] 更新 `doc/aicc` 下模型路由文档。
  - 需要同步：
    - `provider_weights`
    - family mount/current mount/version mount
    - provider weight 与 exact model weight 的关系
    - route trace source 字段

- [ ] 增加 DV Test。
  - 场景：
    - provider weight 0 禁用 provider。
    - provider weight 0.3 降低但不禁用。
    - provider weight 2.0 在同等成本/延迟/质量下优先。
    - exact model weight 与 provider weight 组合生效。

- [ ] 增加 Desktop mock case。
  - mock 数据中加入：
    - 一个 provider weight 为 `0.3`
    - 一个 provider weight 为 `0.0`
    - route trace 中展示 provider weight

## 10. 每次后续改动必须验证

- Rust：
  - `cd src && cargo test -p aicc`
- Desktop：
  - `cd src/frame/desktop && pnpm check`
- 构建检查：
  - `cd src && uv run buckyos-build.py --skip-web`
  - 涉及 Desktop UI 时不要跳过 web build。
- 路由行为：
  - 至少覆盖 `llm.chat`、`llm.plan`、exact model、variant exact model。
- 文档/协议联动：
  - 改字段、命名、存储结构时，必须同步前后端类型和文档。
