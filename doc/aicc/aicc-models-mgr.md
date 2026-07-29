# AICC Models Manager 与模型路由概念说明

状态：Draft  
基线：结合当前 `src/frame/aicc` 新版模型路由实现整理  
目标：明确模型路由里“逻辑目录、模型家族、Provider、模型驱动、权重、用户配置”的边界，以及一次路由结果如何被多个因素共同影响。

## 1. 问题背景

AICC 不是简单的 `model_name -> provider` 映射。一个调用方传入的 `llm.chat`、`llm.plan` 或 `llm.gpt`，最终可能落到不同 provider、不同物理模型、不同 provider options。这个结果会同时受到以下因素影响：

- 系统内置的逻辑目录定义；
- driver metadata 对 provider 模型名的语义解释；
- provider 当前可用模型 inventory；
- session 或 agent 对逻辑目录的 overlay；
- 用户对权重、provider、预算、本地优先等策略的配置；
- provider 运行时状态、价格、延迟、错误率和配额；
- session sticky binding。

因此需要把概念分层，否则 provider、driver、逻辑目录、用户配置会互相假设彼此存在，导致路由结果不可解释。

本文的核心边界是：

```text
Provider 只负责发现“我有什么物理模型”
Driver 负责解释“这些模型有什么能力、属于哪个家族、默认挂到哪里”
逻辑目录负责表达“调用方想要什么用途/能力”
权重和 policy 负责表达“多个可选结果里更偏好谁”
用户/session 配置只做 overlay，不改系统基础语义
```

## 2. 当前实现基线

当前实现已经具备新版模型路由的主要骨架：

- `model_types.rs`
  - 定义 `ProviderInventory`、`ModelMetadata`、`LogicalModelDefinition`、`ModelItem`、`RoutePolicy`、`SchedulerProfile`、`ModelCandidate`。
- `metadata_resolver.rs`
  - 根据 provider driver 的 metadata，把 provider 返回的模型 id 解析成 `ModelMetadata`。
  - 支持 exact rule、wildcard pattern、defaults、variants。
  - 内置 driver metadata 位于 `src/frame/aicc/driver_metadata/`。
  - override 路径位于 `$BUCKYOS_ROOT/etc/aicc/driver_metadata/{remote_cache,local,system-config}/`。
- `model_registry.rs`
  - 接收每个 provider 的 `ProviderInventory`。
  - 建立 exact model 索引。
  - 根据 `ModelMetadata.logical_mounts` 和 `LogicalModelDefinition.min_line` 自动生成逻辑目录的默认 items。
  - 支持同一逻辑目录下保留多个 provider 的候选。
- `default_logical_tree.rs`
  - 定义系统内置的 LLM 用途目录，如 `llm.plan`、`llm.code`、`llm.swift`、`llm.reason`、`llm.vision`、`llm.long`、`llm.fallback`。
  - 每个目录带有默认 family item 权重、`min_line`、fallback 和 scheduler profile。
- `model_session.rs`
  - 定义 `SessionConfig`、`LogicalNode`、`LogicalTreeOverlay`。
  - 支持 `inherit` / `replace` overlay、`item_overrides`、`exact_model_weights`、全局 policy 和 profile。
- `model_router.rs`
  - 展开逻辑目录。
  - 执行 hard filter。
  - 处理 fallback。
  - 根据目录 item weight 和 exact model weight 先筛出最高优先级候选集合。
- `model_scheduler.rs`
  - 在候选集合中按 profile 对 cost、latency、reliability、quality、preference、cache、local 打分。
  - 支持 session sticky binding。

需要特别注意：当前实现里 provider inventory 里的 `logical_mounts` 已经可以直接挂到用途目录或家族目录，但系统边界上更推荐“provider 只产出模型，driver 决定挂载语义”。也就是说，provider 对逻辑目录不应有存在性假设。

## 3. 核心概念

### 3.1 物理模型与精确模型名

物理模型是 provider 实际提供的模型 id，例如 `gpt-5.2`、`claude-sonnet-4.6`、`fal-ai/esrgan`。

AICC 内部把物理模型和 provider instance 组合成全局唯一的精确模型名：

```text
<provider_model_id>@<provider_instance_name>
```

例如：

```text
gpt-5.2@openai-default
claude-sonnet-4.6@claude-main
fal-ai/esrgan@fal-main
```

精确模型名的语义是“明确指定某个 provider instance 下的某个模型”。它默认不做复杂逻辑路由，也默认不 fallback，除非 request policy 显式允许 `allow_exact_model_fallback`。

reasoning effort 这类会显著改变模型行为、成本和能力边界的参数，应视为模型身份的一部分，而不是普通 request option。当前实现已经通过 driver metadata 的 `variants` 把它展开成独立精确模型名：

```text
<base_provider_model_id>:<variant>@<provider_instance_name>
```

例如：

```text
gpt-5.2:reasoning-high@openai-default
gpt-5.2:reasoning-low@openai-default
```

这类 variant exact model 在 AICC 内部是独立模型身份，用于路由、权重、trace、usage 聚合和审计。真正调用 provider 时，再由 data plane lower 成 base model 加 provider options，例如 `provider_model_id=gpt-5.2` 和 `provider_options.reasoning.effort=high`。

### 3.2 Provider

Provider 是一个可调用的模型能力提供实例，而不是厂商名。一个厂商可以配置多个 provider instance，例如：

```text
openai-primary
openai-backup
openai-work
```

Provider 的核心职责是：

- 提供 provider instance 的可信身份和类型；
- 刷新当前可用模型 inventory；
- 执行实际推理调用；
- 提供动态成本估算、健康状态或配额状态。

Provider 不应该负责定义 `llm.plan`、`llm.chat` 这类逻辑目录，也不应该假设某个逻辑目录一定存在。Provider 刷新出来的内容应该先是物理模型列表。

当前代码里 `ProviderInventory` 已经表达了这个边界：

```text
provider_instance_name
provider_type
provider_driver
models: Vec<ModelMetadata>
```

其中 `provider_driver` 表示这个 provider 使用哪个 driver metadata 来解释模型名，例如 `openai`、`claude`、`google-gemini`、`fal`、`minimax`。

### 3.3 模型驱动

模型驱动是 provider 模型名的语义解释层。它不是 provider instance，也不是逻辑目录本身。

Driver metadata 负责回答：

- 这个 provider model id 支持哪些 `api_types`；
- 它有什么能力，例如 `tool_call`、`json_schema`、`vision`、上下文长度；
- 它的默认成本、延迟、质量等级大致是什么；
- 它属于哪个模型家族或用途挂载点；
- 它是否需要展开 variant，例如 reasoning effort；
- 某些模型是否应排除。

Variant 的边界是：如果一个 provider option 会影响路由选择、价格、质量、审计或用户可见模型选择，它就应该进入模型身份。例如 reasoning effort 应展开为 `:reasoning-high`、`:reasoning-low` 这样的 exact model suffix；如果只是一次请求里的普通采样参数，则仍留在 request options 中。

当前 driver metadata 的匹配顺序是：

```text
exact models[].id
-> wildcard patterns[].pattern
-> defaults
-> conservative fallback
```

来源优先级是：

```text
builtin
-> remote_cache
-> local override
-> system-config override
```

这意味着 provider 自发现只需要返回模型 id，driver 决定这个模型 id 在 AICC 里的能力、家族和默认挂载。

### 3.4 模型家族目录

模型家族目录表达“同一类模型线 / 同一品牌能力线”，例如：

```text
llm.gpt
llm.gpt-standard
llm.gpt-mini
llm.opus
llm.sonnet
llm.haiku
llm.gemini-pro
llm.gemini-flash
llm.qwen-coder
```

家族目录不是 provider。它可以包含来自多个 provider instance 的同家族物理模型。例如 `llm.gpt` 可以同时包含：

```text
gpt-5.2@openai-primary
gpt-5.2@openai-backup
gpt-5.1@sn-ai-provider
```

家族目录需要区分“当前推荐入口”和“版本索引入口”。例如 `llm.gpt-standard` 这类 current family mount 应只挂同一 family/tier 下的当前推荐版本，但可以包含多个 provider instance 对这个推荐版本的实现；`llm.openai.gpt-5-2` 这类 version index mount 才用于保留具体版本。用途目录默认引用 current family mount，复现和锁定才使用版本索引或精确模型名。

家族目录的主要用途：

- 给用途目录引用，如 `llm.plan -> llm.opus`；
- 给用户表达偏好，如“更喜欢 GPT 系列”；
- 给 UI 展示模型族；
- 聚合多个 provider 上同类物理模型。

推荐规则是：driver 决定物理模型应该进入哪个家族目录，逻辑用途目录再引用家族目录。这样 provider 不直接依赖用途目录。

### 3.5 用途逻辑目录

用途逻辑目录表达调用方的意图，例如：

```text
llm.chat       # 通用聊天
llm.plan       # 高质量规划
llm.code       # 编码任务
llm.swift      # 快速低成本响应
llm.summarize  # 总结压缩
llm.reason     # 显式推理
llm.vision     # 视觉输入
llm.long       # 长上下文
llm.fallback   # 兜底
```

用途目录里放的是 items。每个 item 指向一个家族目录、另一个逻辑目录或精确模型，并带有权重。

例如：

```text
llm.plan
  opus        -> llm.opus         weight 2.5
  gemini      -> llm.gemini-pro   weight 2.4
  qwen_max    -> llm.qwen-max     weight 1.8
  deepseek    -> llm.deepseek-pro weight 1.5
```

这表示 `llm.plan` 首先是一个“用途目录”，它不直接关心 provider。它只说：规划任务优先考虑哪些家族，以及这些家族之间的默认偏好。

### 3.6 Mini Line

Mini line 是逻辑目录允许挂载的最小能力线，在当前实现里对应 `LogicalModelDefinition.min_line`。

它回答的是：一个模型要进入某个目录，至少要满足什么能力。

例如：

- `llm.plan` 可能要求 `tool_call=true`、`json_schema=true`、`min_context_tokens>=32768`；
- `llm.vision` 要求 `vision=true`；
- `llm.long` 要求 `min_context_tokens>=128000`；
- `llm.chat` 可以很宽松。

当前 `ModelRegistry::default_items_from_inventories()` 在生成默认 items 时会检查 `min_line`。不满足的模型不会挂入该目录，原因会写入 route trace 的 `logical_admission`。

Mini line 的边界是：

- 它是 admission 条件，不是最终调度策略；
- 它只决定能不能进入目录候选；
- 进入候选后，还要经过 request policy、provider 状态、预算、调度评分等阶段。

### 3.7 权重

AICC 里至少有四类权重或偏好来源。

第一类是逻辑目录 item weight：

```text
llm.plan -> llm.opus weight 2.5
llm.plan -> llm.gemini-pro weight 2.4
```

它影响逻辑目录展开时的优先级路径 `priority_path`。当前 `ModelRouter` 会先选择最高 `priority_path` 的候选。如果最高权重路径已经有可用候选，较低权重路径不会继续参与 scheduler 的最终打分。

因此逻辑目录 item weight 不是概率权重，也不是 scheduler 的连续评分项，而是“用途目录到家族目录”的优先级控制。

第二类是 provider weight：

```text
provider_weights
provider_weight_overrides
```

它表达用户对某个 provider instance 的整体偏好比例。例如用户可以把 `openai-backup` 调成 `0.3`，表示在同等条件下少用这个 provider；也可以调成 `0.0`，表示禁用这个 provider。Provider weight 是用户手工调整 provider 权重比例的一等概念，不应该只作为 UI 层的临时描述。

第三类是 exact model weight：

```text
global_exact_model_weights
logical_node.exact_model_weights
```

它用于微调某个具体物理模型的偏好。当前 `exact_model_weight <= 0` 会被过滤；大于 0 时参与优先级比较，并在 scheduler 中通过 `preference` 维度影响得分。

第四类是 scheduler profile weights：

```text
cost
latency
reliability
quality
preference
cache
local
```

它影响候选集合内部的最终选择。当前内置 profile 包括：

```text
cost_first
latency_first
quality_first
balanced
local_first
strict_local
```

需要区分：

- 目录 item weight 先决定“哪些家族/路径进入最高优先级候选集合”；
- provider weight 调整“同一个 provider instance 的整体使用比例”；
- scheduler profile 再决定“同一优先级候选集合中选哪个 provider/物理模型”；
- exact model weight 既能作为路径优先级补充，也能作为 scheduler preference。

当前实现采用的优先级关系是：provider weight 不参与 `select_highest_priority()` 的候选集合筛选，只在候选进入 scheduler 后作为 `preference` 维度的输入；`provider_weight <= 0` 是例外，会在 hard filter 阶段直接禁用该 provider 的候选。这样可以保持用途目录 item weight 与 exact model weight 的路径优先级语义稳定，同时让用户对 provider 的整体偏好在同一最高优先级集合内生效。

### 3.8 用户自定义配置

用户自定义配置不应该修改系统内置基础逻辑，而应该通过 overlay 表达偏好。

当前可用的配置层包括：

```text
系统基础逻辑目录       # 随系统升级，不可直接修改
用户自定义逻辑目录     # 用户可配置，作为 global/session parent
Agent 默认逻辑目录     # Agent 配置，可作为 session 默认值
Session 逻辑目录       # 保存到 session，用于本次会话
Request policy         # 单次请求附带的约束
```

当前 `SessionConfig` 支持：

- `logical_tree`：直接定义逻辑目录树；
- `logical_profile` / `logical_profiles`：一组 overlay；
- `active_logical_profile`：选择哪个 profile；
- `global_exact_model_weights`：全局精确模型权重；
- `provider_weights`：全局 provider instance 权重，`1.0` 为默认，`0.0` 表示禁用该 provider 参与路由；
- `policy`：全局路由策略；
- `ttl_seconds` / `revision`：session 配置生命周期和冲突控制。

长期持久化位置是 `services/aicc/settings.session_config.provider_weights`。该位置属于 AICC 全局 session parent 配置，不写入 provider inventory，也不修改 driver metadata。Control Panel 通过 `ai.provider.weight.list` / `ai.provider.weight.set` 读写该字段；保存时会校验 provider instance name、weight 非负有限，并触发 AICC `service.reload_settings` 使 `models.list.session_config` 立即反映新权重。

`LogicalTreeOverlay` 支持：

- `merge_mode=inherit`：继承默认目录 items，只覆盖或追加部分 item；
- `merge_mode=replace`：替换该目录 items，并默认禁用 fallback；
- `items`：显式指定目录 items；
- `item_overrides`：patch 已存在 item 的 target 或 weight；
- `exact_model_weights`：调整具体物理模型；
- `fallback`：调整 fallback；
- `route_policy_override`：调整策略。

## 4. 目录来源与叠加顺序

推荐把目录分成四层。

### 4.1 系统基础逻辑目录

系统基础逻辑目录随 BuckyOS 升级，用户不可直接修改。它定义的是 AICC 的默认品味和最小能力边界。

当前主要来自 `default_logical_tree.rs`：

- 用途目录：`llm.plan`、`llm.code`、`llm.swift` 等；
- 默认家族 item 和权重；
- `min_line`；
- fallback；
- scheduler profile。

这一层的原则：

- 表达系统默认能力语义；
- 可以随系统版本升级；
- 不承载用户个人偏好；
- 对外应可解释和可审计。

### 4.2 用户自定义逻辑目录

用户自定义逻辑目录是用户长期偏好的配置层。

它适合表达：

- 我更偏好某个模型家族；
- 我不想使用某个 provider；
- 我想让某个目录走本地优先；
- 我想把 `llm.chat` 的默认权重调成更便宜；
- 我想降低某个 provider 的全局权重。

这层应保存到用户配置或 system-config 中，并作为 session 的 parent 或 global config。

### 4.3 Agent 默认逻辑目录

Agent 可以定义自己的默认逻辑目录 profile。例如 Jarvis 可以默认：

- `llm.plan` 使用高质量 profile；
- `llm.swift` 使用低延迟 profile；
- 某些 internal task 使用 `llm.summarize`；
- 对某些工具调用强制要求 `tool_call`。

Agent 默认逻辑目录应保存在 agent 配置中。创建 session 时，Agent 可以把这层配置注入 session。

### 4.4 Session 逻辑目录

Session 逻辑目录是会话内临时配置，保存在 session 里。

它适合表达：

- 这个 session 临时使用 `quality_first`；
- 这个 session 临时禁用某个 provider；
- 这个 session 临时把 `llm.chat` 指向某个模型；
- 这个 session 里已经选择的模型保持 sticky。

Session 配置有 revision 和 TTL，适合被 UI 或 Agent 动态更新。

## 5. Provider 刷新与挂载流程

目标流程应该是单向依赖：

```text
Provider discovery
  -> Driver metadata resolve
  -> ProviderInventory
  -> ModelRegistry
  -> 默认家族目录 / 用途目录 items
  -> Session/User overlay
  -> Route resolve
```

### 5.1 Provider 自发现返回什么

Provider 自发现的最小输出应是 provider model id 列表，以及必要时的 provider 原始 metadata。

从 AICC 视角，可以理解为：

```text
provider_model_id + provider_driver + provider_instance_name
```

其中 provider model id 先不是 AICC 的完整语义。AICC 会通过 driver metadata 生成：

```text
ModelMetadata {
  provider_model_id,
  exact_model,
  model_driver,
  api_types,
  logical_mounts,
  capabilities,
  attributes,
  pricing,
  health,
  provider_options
}
```

如果需要用简写描述自发现结果，可以写成：

```text
模型名@驱动名
```

但正式 inventory 里还必须包含 provider instance，最终 exact model 是：

```text
模型名@provider_instance_name
```

### 5.2 Driver metadata 缺失时

当前实现对未知模型会使用 conservative fallback：

- 默认 `api_types` 可能回落到 `llm.chat`；
- 不声明 `tool_call`、`json_schema`、`vision`、`web_search` 等能力；
- 使用保守的成本、延迟、质量估计；
- 生成泛化挂载，例如 `llm.chat`、`llm.<driver>`、`llm.<driver>.<model>`。

目标上，如果系统没有某个 driver 的 metadata，可以通过 HTTPS 可信通道从远程 URL 更新，并缓存到：

```text
$BUCKYOS_ROOT/etc/aicc/driver_metadata/remote_cache/v1/{objects,activations,observed}/
```

如果仍没有，则使用 conservative fallback。这样系统可用性优先，但不会把未知模型误判成具备高级能力。

这里的信任边界是 HTTPS 远程更新通道。也就是说，远程 driver metadata 不是任意 provider 自称的 metadata，而是 AICC 通过可信 HTTPS 源取得的系统 metadata。Provider 自发现仍只提供模型名和必要 hints，最终能力和挂载语义以 driver metadata resolver 为准。

### 5.3 Driver 如何挂载到模型家族目录

Driver metadata 中的 `logical_mounts` 是默认挂载语义。推荐它优先挂载到模型家族目录，而不是直接把 provider 绑定到用途目录。

这里还需要引入“最新版本发现”的概念：driver metadata 不应该为每个新发布的模型都新增一条 exact metadata。更合理的方式是让 driver metadata 提供匹配表达式，从 provider model id 中提取 family、tier、version、stability 等字段，然后由 resolver/post rule 在同一 family/tier 中自动选出当前推荐版本。

当前实现先采用无需新依赖的轻量 `version_rules` schema 表达这类规则：

```text
family: gpt
tier: standard | pro | mini | nano
model_pattern: gpt-*
tier_tokens / exclude_tier_tokens
version_rank.prefix: gpt
stability.unstable_tokens: [preview, experimental, beta]
stability.current_requires_stable: true
current_mount: llm.gpt-standard
version_mount: llm.openai.{model}
```

当 provider 刷出新模型 `gpt-5.3` 时，只要它匹配这条表达式，driver 就能自动知道：

```text
family = gpt
tier = standard
version = 5.3
current family mount = llm.gpt-standard
version index mount = llm.openai.gpt-5-3
```

如果 `gpt-5.3` 比现有 `gpt-5.2` 更新且不是 preview/beta，就自动成为 `llm.gpt-standard` 的 current model。这样模型小版本更新不需要同步更新 driver metadata；只有命名规则、能力边界、tier 分类或稳定性规则变化时，才需要更新 driver metadata。

reasoning effort 这类 variant 在这个流程中应挂在 base model 之后处理：先用匹配表达式识别 base model 的 family/tier/version/stability，再按 driver metadata 的 `variants` 展开出独立 exact model。Variant 不参与 base model 的版本排序，也不改变 current family mount 的版本判断；它只改变 exact model identity 和可选挂载后缀。

例如：

```text
gpt-5.3
  -> base exact model: gpt-5.3@openai-default
  -> variant exact model: gpt-5.3:reasoning-high@openai-default
  -> provider call lowering: model=gpt-5.3, provider_options.reasoning.effort=high
```

如果 driver 希望把 reasoning variant 暴露成逻辑目录，也应通过 variant mount 表达，例如：

```text
llm.gpt-standard.reasoning-high
llm.reason
```

这样 `llm.reason` 可以选择 reasoning effort 已经固定的模型，而不是在 request 时临时追加 effort 参数。

例如 OpenAI GPT 模型：

```text
gpt-5.2 -> llm.gpt-standard / llm.gpt / llm.openai.gpt-5-2
gpt-5.2-mini -> llm.gpt-mini / llm.gpt / llm.openai.gpt-5-2-mini
```

再由系统用途目录引用家族：

```text
llm.plan -> llm.opus / llm.gemini-pro / llm.qwen-max
llm.chat -> auto admission 或 llm.gpt-standard
llm.swift -> llm.haiku / llm.gemini-flash-lite / llm.qwen-small
```

这样做的好处是：

- provider 不知道用途目录；
- driver 可以根据匹配表达式和版本命名维护家族归属；
- driver 可以自动发现同一 family/tier 的最新稳定版本，减少模型更新时的 metadata 维护；
- 用途目录可以保持稳定；
- 用户调权重时可以在用途层或家族层分别控制。

### 5.4 刷新逻辑

一次刷新可以描述为：

1. 每个 provider 拉取或读取当前支持的模型列表。
2. 每个模型 id 交给对应 driver metadata resolver。
3. Resolver 生成 `ModelMetadata`，包括 exact model、能力、家族挂载、成本和 latency 估计。
4. Provider 生成完整 `ProviderInventory`。
5. `ModelRegistry::apply_inventory()` 以 provider instance 为单位全量替换 inventory。
6. Registry 重新建立 exact model 索引。
7. 对每个逻辑目录，Registry 根据模型的 `logical_mounts` 和目录 `min_line` 生成默认 items。
8. 用户/session overlay 在 route 时叠加到默认 items 上。

如果某个 provider inventory 校验失败，当前实现会跳过该 provider，并保留其它 provider 的刷新结果，不让一个坏 provider 阻塞整体刷新。

### 5.5 空逻辑目录与 mini line 强制挂载

目标设计里，所有 provider 刷新后，空的逻辑目录也应该能按 mini line 做一次 auto admission：

```text
扫描所有物理模型
-> 检查 api_type 和 min_line
-> 满足则临时挂入该逻辑目录
```

当前实现已经具备这个能力的核心：当 `LogicalModelDefinition.mount_mode != manual` 时，`default_items_from_inventories()` 会对该 logical path 执行 `auto_admission`。

因此，一个目录即使没有 driver metadata 显式 `logical_mounts`，只要它有 `LogicalModelDefinition`，且 `mount_mode=auto/hybrid`，满足 `min_line` 的物理模型也可以被挂入。

## 6. 自动权重控制

### 6.1 同家族最新版本优先

同一模型家族中，默认应优先选择最新稳定版本。版本判断必须由 driver 定义，不应由 provider 或用途目录猜测。

这里需要区分两类家族目录：

```text
current family mount   # 稳定家族入口，只指向当前推荐版本
version index mount    # 版本索引目录，保留可锁定的具体版本
```

例如 `llm.gpt-standard` 应代表 GPT standard 线的当前推荐版本；`llm.openai.gpt-5-2` 这类索引目录才表达具体版本。用途目录默认引用 current family mount，这样系统升级或 provider 刷新后可以自然切到最新稳定版本。需要复现、审计或强制锁定时，调用方使用精确模型名或版本索引目录。

当前 OpenAI 已有 post rule 示例：

- driver 会识别 GPT tier；
- 解析模型名中的版本号；
- 保留每个 tier 的最新版本；
- 给最新模型增加角色挂载。

这个能力可以推广为 driver 级 post rules：

```text
driver parses version rank
-> groups models by family/tier
-> selects latest stable model
-> assigns higher family/current mount or higher default weight
```

版本号定义需要 driver 明确：

- 哪些 token 是主版本、次版本；
- 日期版本和语义版本如何比较；
- preview、beta、experimental 是否低于 stable；
- pro、mini、nano、flash 等 tier 不应互相比较；
- variant 不改变 base model 的版本排序。

### 6.2 默认品味权重

系统内置用途目录体现 BuckyOS 默认品味。例如：

```text
llm.plan 更偏高质量模型
llm.swift 更偏低延迟轻量模型
llm.summarize 更偏低成本长上下文模型
llm.reason 不随意 fallback
```

这些默认权重属于系统基础逻辑目录，应随版本升级维护。

### 6.3 成本与 provider 状态控制

Scheduler 不只看目录权重，还看候选的运行属性：

- `estimated_cost_usd` 或动态成本估算；
- `p95_latency_ms` 或 latency class；
- `error_rate_5m`；
- `quality_score`；
- provider type 是否 local；
- quota 是否 exhausted；
- health 是否 unavailable。

当前 hard filter 会先移除：

- api type 不匹配；
- unavailable；
- quota exhausted；
- 不满足 request required features；
- 不满足 `min_line`；
- local_only 下非本地 provider；
- 被 block 或不在 allow list 的 provider；
- 超预算或超 latency；
- exact model weight 为 0。

Scheduler 再在剩余候选中按 profile 打分。

## 7. 用户如何控制权重

用户控制权重的原则是：不改 driver 语义，不改 provider 发现结果，只通过 overlay 和 policy 改变偏好。

### 7.1 通过成本影响优先级

相同能力、相同家族或相同优先级候选中，越便宜的模型在 `cost_first` 或 `balanced` 下越容易被选中。

用户可以通过 provider 侧价格配置或 override 影响：

```text
estimated_cost_usd
input_token_usd
output_token_usd
cache_input_token_usd
```

这适合表达“同一个模型，哪个 provider 更便宜就优先用哪个”。

但成本字段需要区分两种语义：

```text
billing cost          # 真实计费成本，用于账单、审计、展示
routing cost override # 路由偏好成本，用于调度打分
```

用户为了改变优先级，不应该污染真实 billing cost。更合理的做法是设置 routing cost override 或 provider weight。只有 provider 的真实价格变化时，才更新 billing cost。

### 7.2 覆写用途目录的家族权重

用户可以在逻辑目录 overlay 中修改 item weight。

例如，把 `llm.chat` 调成更偏 Claude：

```yaml
logical_profile:
  overlays:
    - path: llm.chat
      merge_mode: inherit
      item_overrides:
        claude:
          target: llm.sonnet
          weight: 3.0
        gpt:
          target: llm.gpt-standard
          weight: 1.5
```

这会影响逻辑目录展开时的 `priority_path`。

### 7.3 调整某个 provider 的权重比例

Provider weight 是一等用户配置，用于手工调整某个 provider instance 的整体权重比例。

例如降低某个 provider 的整体使用优先级：

```yaml
provider_weights:
  openai-backup: 0.3
```

或禁用某个 provider：

```yaml
provider_weights:
  openai-backup: 0.0
```

Provider weight 的作用范围是该 provider 下的所有候选模型。它表达的是“同等条件下少用或不用这个 provider”，不是删除 provider，也不是修改 driver metadata。

当前代码还没有独立的 provider weight 字段，短期实现可以在配置物化时把 provider weight 展开成该 provider 当前 inventory 下所有 exact model 的 `global_exact_model_weights`。长期应把 provider weight 纳入 scheduler 的 `preference` 维度，并在 route trace 中独立展示。

用户也可以用 `global_exact_model_weights` 或目录内 `exact_model_weights` 调整某个 provider 的具体模型。

例如降低某个 provider 的使用概率：

```yaml
global_exact_model_weights:
  gpt-5.2@openai-backup: 0.3
```

或禁用某个精确模型：

```yaml
global_exact_model_weights:
  gpt-5.2@openai-backup: 0.0
```

当前实现中，`exact_model_weight <= 0` 会被 hard filter 移除。

### 7.4 用 provider allow/block 做硬约束

如果用户明确不想使用某个 provider，应使用 policy：

```yaml
policy:
  blocked_provider_instances:
    - openai-backup
```

如果用户只允许某些 provider：

```yaml
policy:
  allowed_provider_instances:
    - local-llama
    - openai-primary
```

这属于硬过滤，不是权重。

### 7.5 用 scheduler profile 改变整体偏好

用户可以改变 profile：

```yaml
policy:
  profile: latency_first
```

或自定义 profile weights：

```yaml
policy:
  scheduler_profiles:
    balanced:
      cost: 0.35
      latency: 0.20
      reliability: 0.20
      quality: 0.15
      preference: 0.10
      cache: 0.10
      local: 0.0
```

这影响同一优先级候选集合里的最终选择。

## 8. 一次典型路由解析

以请求 `llm.chat` 为例。

### 8.1 输入

```text
api_type = llm.chat
model = llm.chat
session_id = s1
policy = balanced
```

### 8.2 展开逻辑目录

Router 读取：

```text
系统基础逻辑目录
用户自定义配置
Agent 默认配置
Session overlay
Provider inventory default items
```

然后从 `llm.chat` 开始展开 items。

如果 `llm.chat` 直接有 provider inventory auto admission，可能得到：

```text
gpt-5.2@openai-primary
gpt-5.2@openai-backup
claude-sonnet-4.6@claude-main
gemini-3-flash@google-main
```

如果 `llm.chat` 通过家族目录展开，过程可能是：

```text
llm.chat
  -> llm.gpt-standard
    -> gpt-5.2@openai-primary
    -> gpt-5.2@openai-backup
  -> llm.sonnet
    -> claude-sonnet-4.6@claude-main
```

每条路径都会带上 `priority_path`。

### 8.3 Admission 与 hard filter

候选会经过过滤：

```text
api_type 是否匹配
model health 是否 available
quota 是否 exhausted
是否满足 request required_features
是否满足 logical definition min_line
是否满足 local_only
是否在 allowed providers 内
是否被 blocked providers 禁用
是否超预算
是否超 latency
exact_model_weight 是否 > 0
```

被过滤的原因会进入 route trace。

### 8.4 目录权重选择候选集合

Router 会比较候选的 `priority_path` 和 `exact_model_weight`，选择最高优先级集合。Provider weight 不参与这一步；它只在 hard filter 中处理 `provider_weight <= 0`，并在 scheduler 的 `preference` 维度中影响同一最高优先级集合内的排序。

这一步很重要：如果 `llm.chat -> llm.gpt-standard weight 3.0` 有可用候选，而 `llm.chat -> llm.sonnet weight 2.0` 也有可用候选，那么当前实现会只保留最高权重路径的候选进入 scheduler。

因此目录权重不是“概率权重”，而是“优先级权重”。

### 8.5 Scheduler 最终选择 provider/物理模型

Scheduler 在最高优先级候选集合中打分。

例如候选集合里有：

```text
gpt-5.2@openai-primary cost=0.01 latency=1200 quality=0.9
gpt-5.2@openai-backup  cost=0.008 latency=1500 quality=0.9
```

在 `cost_first` 下可能选择 `openai-backup`。在 `latency_first` 下可能选择 `openai-primary`。

如果 session sticky 已有绑定，且绑定模型仍在候选集合中，会优先使用 sticky binding。

### 8.6 输出

`route.resolve` 最终输出：

```text
selected_exact_model
selected_provider_instance_name
selected_provider_model_id
provider_options
fallback_attempts
enabled/disabled capability trace
route_trace
```

数据面只消费 exact model，不再重新做逻辑路由。

## 9. 关键边界总结

### 9.1 逻辑目录不依赖 provider

逻辑目录定义用途和最小能力线，不应该知道 provider 如何发现模型。

### 9.2 Provider 不依赖逻辑目录

Provider 只发现物理模型并执行调用。它不应该硬编码 `llm.plan`、`llm.chat` 是否存在。

### 9.3 Driver 是 provider 模型名到 AICC 语义的桥

Driver metadata 负责把模型 id 解释成能力、家族、默认挂载、成本和 variant。

### 9.4 家族目录连接 driver 和用途目录

Driver 通常把物理模型挂到家族目录；用途目录引用家族目录。这样系统可以分别维护“模型语义”和“任务偏好”。

### 9.5 权重分两阶段生效

目录 item weight 先决定最高优先级路径；scheduler profile 再在同一优先级候选内按成本、延迟、质量等打分。

### 9.6 用户配置是 overlay

用户不改系统基础逻辑，不改 driver 语义，只通过 overlay、exact model weight、provider allow/block、scheduler profile 和成本配置影响结果。

## 10. 与当前实现的差距和建议

### 10.1 Provider 侧应进一步弱化逻辑目录假设

当前 driver metadata 已经承担大部分挂载语义，但 provider 配置和部分 fallback mounts 仍可能直接给出逻辑挂载。建议逐步收敛为：

```text
Provider returns physical models
Driver metadata owns logical_mounts
Registry owns admission
Session/user owns overlay
```

Provider 可以提供 fallback hints，但不应成为逻辑目录真相源。

### 10.2 家族目录应成为主要 driver mount 目标

建议 driver metadata 默认挂载到稳定家族目录，例如：

```text
llm.gpt-standard
llm.gpt-mini
llm.opus
llm.sonnet
llm.gemini-pro
llm.gemini-flash
```

用途目录如 `llm.plan`、`llm.code`、`llm.swift` 主要引用家族目录。只有少量稳定通用目录，例如 `llm.chat`，可以允许 auto admission 或直接挂载。

### 10.3 版本排序规则应 driver 化

“同家族最新版本权重最大”不能靠通用字符串排序。建议每个 driver metadata 或 driver post rule 明确定义：

```text
family classifier
tier classifier
version parser
stable/preview rank
latest mount rule
```

当前 OpenAI / SN-AI GPT 规则已经通过 driver metadata `version_rules` 表达，resolver 根据 driver metadata 选择同 tier 最新 stable model 写入 current family mount，并保留 version index mount。Preview、beta、experimental 只进入 version index，不成为 current family mount；reasoning variant 在 base model current mount 选出后展开，不参与 base version 排序。

### 10.4 Provider 权重比例是一等用户配置

用户可以手工调整某个 provider 的权重比例，因此 provider weight 应是 AICC models manager 的一等概念。

建议配置入口：

```yaml
provider_weights:
  openai-primary: 1.0
  openai-backup: 0.3
  local-llama: 2.0
```

语义：

- `1.0` 表示默认权重；
- `0.0` 表示该 provider 不参与路由；
- `0.0 < weight < 1.0` 表示降低该 provider 的整体偏好；
- `weight > 1.0` 表示提高该 provider 的整体偏好。

实现方向：

1. 在用户配置写入时展开成该 provider 下所有 exact model weights；
2. 在 scheduler 增加 provider weight，作为 `preference` 的一部分。

短期建议使用第一种，新增代码少，且能复用当前 `global_exact_model_weights`。长期建议使用第二种，让 route trace 能明确展示“provider weight 影响了最终选择”。

### 10.5 空目录 auto admission 应在文档和 UI 中显式暴露

当前实现已经支持 `mount_mode=auto/hybrid` 的 auto admission。UI 和 trace 应明确展示候选来自：

```text
driver_metadata_mount
auto_admission
session_overlay
manual_override
```

这样用户才能理解“为什么我没手动挂载，它也出现在这个目录里”。

## 11. 示例配置

### 11.1 调整 `llm.plan` 家族偏好

```yaml
logical_profile:
  overlays:
    - path: llm.plan
      merge_mode: inherit
      item_overrides:
        opus:
          target: llm.opus
          weight: 3.0
        gemini:
          target: llm.gemini-pro
          weight: 2.0
        qwen_max:
          target: llm.qwen-max
          weight: 1.0
```

### 11.2 禁用某个 provider 的某个模型

```yaml
global_exact_model_weights:
  gpt-5.2@openai-backup: 0.0
```

### 11.3 让某个 session 更偏便宜

```yaml
policy:
  profile: cost_first
  max_estimated_cost_usd: 0.02
```

### 11.4 只允许本地模型

```yaml
policy:
  local_only: true
  profile: strict_local
```

### 11.5 临时替换 `llm.chat`

```yaml
logical_profile:
  overlays:
    - path: llm.chat
      merge_mode: replace
      items:
        local:
          target: qwen3@local-llama
          weight: 1.0
```

`merge_mode=replace` 会替换该目录 items，并默认禁用 fallback，适合强制指定会话内策略。

## 12. 最终模型

一条完整路由可以概括为：

```text
request model alias
  -> logical directory
  -> family items with weights
  -> exact physical models from provider inventories
  -> mini line admission
  -> user/session overlay
  -> hard filters
  -> highest priority path
  -> scheduler score
  -> sticky binding
  -> selected exact model
```

其中每一层的职责应保持单向：

```text
Provider 发现物理模型
Driver 解释物理模型
Registry 生成默认目录候选
Logical directory 表达用途
Overlay 表达用户/session 偏好
Router 展开和过滤
Scheduler 做最终选择
Executor 调用 provider
```
