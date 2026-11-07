# AICC refactor/aicc 重构对照评审

- 评审对象：`streetycat/buckyos` 分支 `refactor/aicc` @ `38d3f3e`
- 对照基线：`buckyos/buckyos` main @ `ee06dbd8`
- 评审日期：2026-09-13
- 变更规模：323 文件，+90,480 / −71,309；单人（zhangzhen）202 个提交，2026-08-30 至 09-07
- 本地验证：`cargo check -p aicc --all-targets` 无警告；`cargo test -p aicc` 408/408 通过
- 评审方法：三个并行代码审查子任务（协议层 / 元数据层 / Provider 层）+ 对每条重要结论的人工抽查。行号均对应 HEAD `38d3f3e`。未做真实厂商 API 调用。

## 0. 总体结论

**分支不是按“3 个 Step 增量重构”做的，而是一次全量重写（自称 Beta 2.2）。** 旧 aicc crate 已整体删除，新 crate 约 6.5 万行、17 个模块、408 个内联单测。

按 7 条意图逐项对照：**Step1 的 trait/协议层基本达标；Step2 的 model driver 元数据有一个阻断级缺陷（逻辑挂载模板未展开）；Step3 的 Provider 层“都有了但都不完整”，计价几乎没有落地。** 路线图 Tracker 与代码严重脱节，不能拿它当进度依据。

三个子系统的共同模式：**骨架和边界做得很好，数据与文档没跟上。** protocol 的 codec 设计、catalog 的四源选择、execution 的 pinned pricing 都是对的形状；坏在 metadata 内容回退、模板未展开、价格空缺、文档自相矛盾。单测全过是因为测试用的是合成 catalog 和手写 mounts，没有一条测试从真实 metadata 走到真实逻辑目录。

| # | 重构意图 | 状态 | 一句话结论 |
|---|---|---|---|
| 1 | 各 AI API 的 trait 设计 + 自有协议 | 基本完成 | 24 个 api_type 各有强类型 request/response，统一 canonical IR，旧的 `AiPayload/provider_options` 逃生口已清除 |
| 2 | protocol_client → provider 结构 + API→协议映射文档 | 部分完成 | codec/registry 设计干净，但 Provider 实例与 adapter 强制 1:1；没有完整映射表；派生 dialect 多轮 ProviderState 丢失 |
| 3 | 按厂商分文件列出标准模型名 | 部分完成 | 11 个 `*.model.json` 结构统一、加载校验齐全；但清单相对 main 明显回退，含自相矛盾条目 |
| 4 | model driver 字段 / 通配符 / driver 名→逻辑模型名 | **有阻断缺陷** | `{driver}`/`{model}` 模板从未展开（136 处），version_rules 用渠道模型名而非原厂名 |
| 5 | 计价 API；Usage 同时列 token 与带币种金额 | 部分完成 | 金额强制币种、按币种分桶做对了；但 Usage 只有 4 个字段，12 家里 8 家无价格，无厂商计价 API |
| 6 | 列出需要模型名转换的 provider（可更新配置） | 部分完成 | 机制在 metadata 里且可四源覆盖；但只有 OpenRouter 配了，无清单文档，豆包 endpoint id 缺失 |
| 7 | 完成所有 provider 实现 | 接近完成 | 12 家均有 json + 装配 + 测试，旧代码已删；但 fal/glm discovery 死代码、视频 cancel 缺、原生媒体缺、Tracker 自相矛盾 |

## 1. Step 1：trait 与协议层

### 1.1 做对的部分

- **公共 API 强类型化彻底。** `src/kernel/buckyos-api/src/aicc_client.rs` 定义 24 个 `ApiType`（:1041-1088），每个有独立 request/response（`typed_request!/typed_response!` 宏，:3086-3180），统一 `AiccCall` 枚举 27 个变体（:4381-4480）和逐方法的 `AiccHandler`（:5313-5750）。main 上的 `AiMethodRequest / AiPayload{input_json, options} / provider_options: Value` 全部删除，grep 零命中。
- **canonical IR 统一。** `AiRole / AiContent{Text,Image,Document,ToolUse,ToolResult,Thinking,ProviderState} / AiMessage / AiUsage / AiCost / AiArtifact`（:1942-2460）被所有方法共用；ResourceRef 贯穿图像/音频/视频输入输出。
- **协议层抽象干净。** `src/frame/aicc/src/protocol/adapter.rs`：`OperationDescriptor`(:74) → `AdapterDescriptor{base_adapter_id}`(:131) → `OperationCodec{encode,decode,decode_stream}`(:352) / `NativeTaskCodec`(:423)；`CodecRegistry` 以 (adapter, operation, api_type) 为键(:451)，派生 adapter 未覆盖的 binding 自动继承 base 的 codec（:540-575）。13 个 adapter 注册于 `provider/builtin/registry.rs:483-516`。
- **call lowering 唯一。** `call/mod.rs:397-410` 只用 `(protocol_adapter_id, operation, api_type)` 查 registry；operation 选择优先级 method > api_type > adapter 唯一默认（:579-629）。
- **反模式扫描干净。** protocol/call/routing/execution 层没有按模型名前缀、provider id 或 URL 的分支；唯一真实违规是 `provider/mod.rs:2188` 用裸字符串 `"custom"` 而非 `CUSTOM_PROVIDER_PROFILE_ID`。

已注册的 13 个 adapter：

| adapter id | family / base | operations (api_types) | 流式 | native task |
|---|---|---|---|---|
| `openai-responses` | openai / — | `responses.create`(llm, vision.ocr/caption, image.txt2img/img2img, agent.computer_use)、`embeddings.create`、`images.generate/edit`、`audio.speech/transcriptions`、`videos.create` | llm、image | videos.create (cancel) |
| `openai-chat-completions` | openai / — | `chat.completions.create`(llm, vision) | llm | — |
| `claude-messages` | claude / — | `messages.create`(llm, vision) | llm | — |
| `gemini-interactions`（status Preview） | gemini / — | `interactions.create`(llm, vision×4, audio×3, image×2)、`models.embedContent`、`models.predictLongRunning`(video×4) | llm、tts | video（无 cancel） |
| `fal-queue` | fal / — | `queue.submit`(14 个媒体 api_type) | — | 全部 (cancel) |
| `minimax-messages` | claude / claude-messages | `messages.create` + `t2a.create`、`image_generation.create`、`music_generation.create`、`video_generation.create` | llm | video（无 cancel） |
| `openrouter-openai` | openai / openai-chat-completions | `chat.completions.create` + `embeddings.create`(借自 openai-responses) + `rerank.create` | llm | — |
| `kimi-chat` / `glm-chat` | openai / openai-chat-completions | `chat.completions.create` | llm | — |
| `deepseek-/doubao-/qwen-responses` | openai / openai-responses | 仅 `responses.create`(llm) | llm | — |
| `sn-openai` | openai / openai-responses | 仅 `responses.create`，零 override | llm | — |

### 1.2 缺口与缺陷

**1. Provider 实例与 adapter 强制 1:1，“一个 provider 持有多个 protocol_client”未实现。**
`ProviderInstanceConfig` 只有一个 `protocol_adapter_id`（`provider/mod.rs:441-452`），且 `registry.rs:169-176` 与 `provider/mod.rs:2003,2202` 强制它等于 profile 的 default adapter（仅 custom 例外）。结果是 adapter 退化成“provider bundle”：`minimax-messages` 声明 family=claude、base=claude-messages，却塞进 4 个 MiniMax 原生媒体 operation（`minimax_messages.rs:40-58`）；`openrouter-openai` 从 openai-responses 里借 embeddings codec 拼装（`chat_completions_dialects.rs:84-98`）。
建议：实例保存 `protocol_adapters: Vec`，由 provider.json 的 operations map 指定每个 api_type 走哪个 adapter，registry 键不变。

**2. 派生 dialect 的 ProviderState 只在 decode 侧改 namespace，多轮对话静默丢状态（已验证）。**
`protocol/derived_responses.rs:241-256` 在解码时把 `provider:"openai"` 改写为 deepseek/doubao/qwen；但 `encode`（:146-181）把 canonical request 原样交给 base，而 base `openai_responses.rs:708-720, 800-821` 只回放 `provider == "openai"` 的 ProviderState，其余降级为纯文本。MiniMax 对 claude-messages 同理（`minimax_messages.rs:275-290`）。
影响：这 4 家的 reasoning item / encrypted state 在第二轮全部丢失。`derived_responses.rs:437` 的测试只覆盖 decode 方向。

**3. 没有“provider × api_type × operation × codec”的映射文档，且现有表与代码脱节。**
现有文档只有散落的表：`internal_module_architecture.md` §1/§4.2/§5.2、`aicc_provider_plan.md` §2、`provider_profile_schema.md:105`（只列 9/13 个 adapter，缺 minimax-messages/kimi-chat/glm-chat/deepseek|doubao|qwen-responses/fal-queue）。SN 未进入任何表。Qwen 在文档里是“qwen-responses + native media”，代码里只有一个 `responses.create/llm`（`derived_responses.rs:311` 断言 len==1）；文档目录中的 `native/dashscope_media、glm_async、doubao_media` 均不存在。
唯一接近真相的是 `call/mod.rs:1614-1690` 的 golden 测试锁定的 71 条 (profile|adapter|api_type|operation) 绑定，可以直接导出为文档。

**4. 两套 dialect 机制并存，HTTP 错误映射复制了 8 份。**
chat-completions 用 trait hook（`OpenAiChatCompletionsDialect` 6 个 hook，`openai_chat_completions.rs:60-99`，base 在 8 处调用），responses/minimax 用包装 codec + `match self.dialect` 枚举分支（`derived_responses.rs:147,163`）。HTTP status → `ProtocolErrorKind` 在 8 个文件各写一份且表不一致（`claude_messages.rs:963`, `openai_chat_completions.rs:1068`, `openai_responses.rs:1566`, `gemini.rs:2240`, `fal_queue.rs:629`, `minimax_messages.rs:206`, `chat_completions_dialects.rs:260`, `sse.rs:247`；如 422 只有 openrouter 归 InvalidRequest），`protocol/error.rs` 没有共享 helper。

**5. 测试倒退。**
main 上 `src/frame/aicc/tests/` 14 个集成测试文件（9,051 行，含 adapter_protocol/stream_semantics/task_lifecycle/routing_semantics）整体删除无替代。新测试全部内联、全部手写 `json!`，没有录制的真实厂商 payload，对上游协议漂移无防护。`minimax_media.rs` 零测试；`derived_responses.rs` 无 wire 级测试（7 个测试只测注册/namespace）。

**其它残留**：`AiResponse.extra: Option<Value>`（`aicc_client.rs:2459`）仍在但 typed response 不用；`AiccExecutionMode::Stream` 没有公共流式响应类型，stream 通过 `event_ref` 走 task_mgr 事件。

## 2. Step 2：Model Driver 与逻辑模型目录

### 2.1 做对的部分

- 11 个 `driver_metadata/models/*.model.json` 按原厂一文件，envelope 统一（format/schema_version/model_driver_id/revision_seq）。`catalog/mod.rs:60-80` 用 `deny_unknown_fields` 拒绝旧字段（`provider_driver/provider_options/origin_*/signature`）；`validate_model_driver`（:1456-1564）校验 version_rules 引用、variant/rule id 唯一、`quality_score∈[0,1]`、capabilities 禁用键。
- MatchRule（`matching/mod.rs:11-16`）字符串 glob 与多维对象两种形式，值支持 scalar/数组=OR/`{not}`/`{exists}`，未知维度编译期报错。
- 匹配优先级 exact > pattern > defaults > conservative 在 `catalog/mod.rs:952-1053` 逐段实现，跨 driver 多命中报 `AmbiguousModelDrivers`。
- `settings/mod.rs:1085` 测试保证嵌入集合与磁盘 35 个 json（12 known + 12 rules + 11 models）一致且 `build_snapshot` 成功；`validate_references`（:2013-2087）校验 `metadata_drivers`、aliases、`provider_rules_id`。

各厂商文件概览：

| 文件 | model_driver_id | exact | patterns | variants | version_rules | 备注 |
|---|---|---|---|---|---|---|
| anthropic.model.json | `claude` | 5 | 0 | 5（无 mount_suffix） | 4 | 旧 claude.json 7 patterns 改为全 exact，无兜底 |
| cohere.model.json | `cohere` | 1 | 1 | 0 | 0 | 无 provider 文件，仅经 openrouter regex mapping 到达 |
| deepseek.model.json | `deepseek` | 3 | 0 | 7（无 suffix） | 2 | 主模型无 static mounts，全靠 version_rules |
| doubao.model.json | `doubao` | 1 | 4 | 1 | 4 | 最规整 |
| fal.model.json | `fal` | 4 | 0 | 0 | 0 | 与旧文件相同，mount 手写展开 |
| gemini.model.json | `gemini` | 28 | **0**（旧 40） | 5（有 suffix） | 3 | `gemini-3.7-flash` 缺 max_context；image 模型被列进 reasoning 变体 |
| glm.model.json | `glm` | 20 | 0 | 2 | 6 | 最完整 |
| kimi.model.json | `kimi` | 2 | 0 | 2 | 3 | kimi-code 两条规则无模型能命中 |
| minimax.model.json | `minimax` | 19 | 0 | 0 | 9 | — |
| openai.model.json | `openai` | 10 | 8 | 6（有 suffix） | 4 | 见下方矛盾 |
| qwen.model.json | `qwen` | 4 | 13 | 7（无 suffix） | 3 | **全文件无 logical_mounts** |

### 2.2 缺口与缺陷

**1. 阻断级：`logical_mounts` 的 `{driver}`/`{model}` 模板从未展开（已验证）。**
`provider/mod.rs:1075-1076` 直接取 `semantics.logical_mounts` 原样入库；全 crate 唯一的替换函数 `expand_version_mount`（:1310）只替换 `{model}` 且只用于 version_rules；`model/mod.rs:1324` 的 `validate_logical_path` 不拒绝 `{`。10 个 model 文件共 136 处模板（openai 24、gemini 31、minimax 28、glm 26、anthropic 9、doubao 5、kimi 5、deepseek 3、qwen 3、cohere 2）会以字面量 `llm.{driver}.{model}` 注册为目录节点。
main 上 `metadata_resolver.rs:1469-1492` 的 `expand_mount_template` 是完整的，属重构回退。这正是意图 4 “driver 名 → 逻辑模型名”的核心机制。

**2. version_rules 用 `provider_model_id` 而非 `origin_model_id`，逻辑名依赖 Provider（已验证）。**
`provider/mod.rs:1184,1187,1195` 的 tier 分词、version_mount、auto_mounts 全部用渠道模型名。OpenRouter 的 `google/gemini-2.5-pro` 会产出 `llm.gemini.google-gemini-2-5-pro`，直连产出 `llm.gemini.gemini-2-5-pro`，token 集合也多出 `google`。另外 match_rule.md 承诺的 `family/tier/stability` 维度从未注入 context（:956-965, :1168-1177），只能编译、永远不命中。

**3. 模型清单相对 main 明显回退，且有自相矛盾条目。**
- openai.model.json 删除了 whisper-1、tts-1/hd、text-embedding-ada-002 及 41 条 pattern（o 系列、gpt-4o/4.1、gpt-5/5.1/5.2）；gemini 40 条 pattern 清零。未列出的模型只命中空 defaults → `provider/mod.rs:871-878` 静默过滤，连 exact name 都不可达。`aicc 逻辑模型目录.md:384,539,548` 仍引用 `tts-1-hd@openai`、`whisper-1@openai`。
- openai：exact `gpt-5.6` 标 `parameter_scale: "sol"` 却挂 `llm.gpt-standard`（:934-956），而 pattern `gpt-5.6-sol*` 归 gpt-pro（:1105-1127），gpt-standard 规则又 `exclude_tier_tokens` 含 `sol`（:1192）。
- openai static mounts 直接写 `llm.gpt-standard/pro/mini/nano`，违反 aicc-models-mgr.md:588 “current family 目录只由 current_mount 产生”，结果 gpt-5.6/5.5/5.4 同时留在 `llm.gpt-standard`。
- qwen.model.json 没有一个 logical_mounts，2.4t-a95b/27b/35b-a3b 等命中不了任何 version rule → 零挂载，只能按 exact name 调用。
- kimi-code 两条 version rule 没有任何模型能命中，而 `service/mod.rs` 的 builtin overlay 引用 `llm.kimi-code`。
- 变体：6 家无 `mount_suffix`；变体继承 base 全部 mounts（`model/mod.rs:612-615`），导致 `llm.gpt-standard` 同时含 base + 6 个 reasoning 变体；变体展开依赖 provider_rules 存在（`provider/mod.rs:1086`），custom provider 拿不到 Model Driver 定义的变体。
- 命名疑点需人工核对：`gpt-transcribe`、`gemini-omni-1.1-flash`、`qwen3.8-2.4t-a95b`、`glm-4-32b-0414-128k`。

**4. 文档与 metadata 脱节。**
`driver_metadata_schema.md:179-184` 的 version_rules 一节没有列任何字段（family/tier/tier_tokens/version_rank/stability/auto_mounts 全靠读代码），:125 示例 id 仍写 `google-gemini`（实际是 `gemini`）；`aicc 逻辑模型目录.md:183-236` 引用 `llm.gemini-deepthink/grok*/deepseek-reasoner/qwen-coder/qwen-small/kimi-thinking` 等无生产者的名字。没有任何测试把真实 11 个 model 文件跑过 InventoryBuilder → apply_version_rules → ModelRegistry 并断言产出的逻辑名，所以上述问题全部漏网。

**附注**：`provider/builtin/anthropic_models.rs` 是 Anthropic `/v1/models` 分页 discovery 客户端（claude 与 minimax 共用），不含模型目录数据，不违反规则；文件名易误解，建议改为 `anthropic_models_discovery.rs`。

## 3. Step 3：Provider 实现与计价

### 3.1 Usage / cost（意图 5）

**做对的：币种是强制的，且不会跨币种相加。**
`AiCost{amount, currency}`（`aicc_client.rs:2393`）、`Money{amount, currency}`（:1692，deny_unknown_fields）；写入前校验非空并大写（`execution/mod.rs:1324-1333`，`storage/mod.rs:1276-1283`）；聚合 `UsageAggregate.finance_totals: Vec<Money>` 按币种分桶（`storage/mod.rs:1244-1275`）；预算比对要求币种一致（`policy.rs:1077-1091`，`service/mod.rs:2407-2413`）。

计价链路：codec decode → `ProtocolOutput.usage` → inventory 构建时三级定价（ModelDriver → ProviderRules → Discovery，`provider/mod.rs:994-1022`）→ `ResolvedPricing` → 执行时固定为 `PinnedPricingSnapshot`（`execution/mod.rs:207-257`）→ `completion_cost()`（:260-287）→ `finance_snapshot` → `aicc_usage_event` → `usage.query`。

**缺口 1：`AiUsage` 只有 4 个字段（已验证）。**
`aicc_client.rs:2370`：`input_tokens / output_tokens / total_tokens / request_units`。没有 cache、reasoning、image 数、音视频秒数。Claude 的 `cache_read/creation_input_tokens` 被丢弃（`claude_messages.rs:912-938`），Kimi 的 `cached_tokens` 塞进 ProviderState（`chat_completions_dialects.rs:664-670`）。fal 与 MiniMax 媒体永远 `request_units(1)`（`fal_queue.rs:403,449`；`minimax_media.rs:331,455,475`），多图/按秒计价的结果错误。

**缺口 2：pinned 了 cache 单价却在结算时忽略（已验证）。**
`execution/mod.rs:262-264` `cache_input_token: _`，因为 AiUsage 没有 cache 字段 → 带缓存的调用按全价计费，系统性高估。

**缺口 3：没有厂商计价 API，12 家里 8 家无任何价格。**
只有 OpenRouter discovery 读 `/models` 的 `pricing.prompt/completion`，且币种硬编码 `"USD"`（`openrouter.rs:230`）。其余 builtin discovery 全部 `pricing: None`。静态价格只在 openai/anthropic/minimax 三个 model.json 里；12 个 `*.provider.json` 与 known-provider 全部没有 pricing；gemini/kimi/glm/deepseek/doubao/qwen/fal/sn 每次调用 `cost=None、finance_complete=false`。没有 CNY。`ProviderQuotaObserver` trait（`provider/mod.rs:582`）无任何实现；OpenRouter 响应里的 `usage.cost` 也未消费。

**缺口 4：`estimated_cost_usd` 无币种且生产恒为 None，配置成本上限会拒绝所有请求（已验证）。**
`service/mod.rs:1053` 固定 `estimated_cost_usd: None`（唯一的 `Some` 在测试里）；`policy.rs:900-905` 在配置了 `max_estimated_cost` 时对 None 直接 `CostEstimateUnavailable` 拒绝；按成本打分（`routing/mod.rs:924-927`）也没有数据。字段名 `_usd`（`routing/mod.rs:59,135,169`；`call/mod.rs:34`；`provider/mod.rs:566 remaining_cost_usd`）本身违反“金额必须带币种”的意图。

### 3.2 模型名转换（意图 6）

机制在 metadata 而非 Rust：`ProviderRulesCatalog.origin_provider_aliases / origin_mappings`（`catalog/mod.rs:314-316`，执行 `resolve_provider_origin` :1099-1125），四源优先级 system-config > local > cloud > builtin 整文件覆盖。生产代码里没有 Rust 侧模型名表（`provider/builtin/*.rs` 所有模型字面量在 `#[cfg(test)]` 之后）。

| provider | 是否需要转换 | 现状 |
|---|---|---|
| openrouter | 需要（`vendor/model`） | 已配置：regex `^(?<driver>[^/]+)/(?<model>.+)$` + aliases `{anthropic→claude, google→gemini, moonshotai→kimi, z-ai→glm}`，唯一真正用了 mapping 的 |
| sn | 需要（网关别名） | 由 `/models` 返回的 `provider_actual_model_id` 在 Rust discovery 里处理（`sn.rs`），不在可更新配置 |
| doubao | 需要（方舟 `ep-xxxx` 接入点） | **缺失**：`origin_mappings: []`，只能直接用模型名 |
| fal / minimax | 恒等 | models[] 显式 4 条，endpoint id == driver model id |
| openai / claude / gemini / deepseek / qwen / kimi / glm | 恒等 | mapping 为空或无该键 |

没有任何文档列出“哪些 provider 需要转换”；`aicc_provider_plan.md:106` 只有一句“Provider Rules 完成渠道映射”。

### 3.3 Provider 完整度（意图 7）

装配机制：`known-provider.json` 的 `protocol_adapter_id` 选 adapter；`registry.rs:333-343` 按 profile id 选 discovery 工厂，未列出的 id（fal、glm）和 doubao/qwen 走 `Standard`（按 family：claude→Anthropic、gemini→Gemini、其余→openai-compatible `GET {base}/models`）；全部包在 `FallbackDiscovery` 里，失败后回退 catalog 且 health=Degraded。`profile_from_catalog` 硬编码 `DiscoveryMode::MachineApi`（:407），`CatalogOnly` 生产中从不设置。

| provider | discovery | operations | 流式 | 异步任务 | 价格 | 主要问题 |
|---|---|---|---|---|---|---|
| openai | 真实 `/models` | llm/vision/computer_use/embed/image/audio/video | ✔ | video 全生命周期 | 静态 6/14 | — |
| claude | 真实分页 `/models` | llm/vision | ✔ | — | 静态 | cache token 丢弃 |
| gemini | 真实分页 `/models` | llm/vision×4/audio×3/image/embed/video×4 | ✔ | video，**cancel 返回 UnsupportedOperation**（`gemini.rs:1830`） | 无 | adapter 状态 Preview |
| fal | **死代码**：`fal_discovery()`（`fal.rs:42-47`）未接线，生产走 openai-compatible `/models` 必失败后 Degraded 回退 | 14 个媒体 api → queue.submit | — | submit/status/result/cancel | 无 | 每次刷新一次注定失败的探测；4 个模型“暂归” fal |
| openrouter | 真实 `/models` + 价格 | llm/embed/rerank | ✔ | — | 动态 USD 硬编码 | — |
| minimax | 真实（anthropic spec） | llm/tts/image/video/music | ✔ | video，**cancel 不支持**（`minimax_media.rs:273`） | 静态 | `minimax_media.rs` 零测试 |
| kimi | 真实 `/models` | llm/vision | ✔ | — | 无 | kimi-code 挂载空转 |
| glm | **死代码**：`glm_catalog_only_inventory`（`glm.rs:71`）未接线，同 fal | llm/vision | ✔ | — | 无 | 原生异步 API 未做 |
| deepseek | 真实 `/models` | llm/vision | ✔ | — | 无 | 多轮 ProviderState 丢失 |
| doubao | openai-compatible `/models`（方舟是否支持未验证） | llm/vision | ✔ | — | 无 | endpoint id 转换缺失；原生媒体未做 |
| qwen | 同上（base_url 模板含 workspace/region） | **仅 llm** | ✔ | — | 无 | 无任何逻辑挂载；原生媒体未做 |
| sn | 真实 `/models`，bearer 或 dynamic_login 互斥 | llm | ✔ | — | 无 | 别名转换在 Rust |

deepseek/doubao/qwen 无独立 `.rs` 是设计使然：`openai_responses_compatible.rs` 按协议兼容关系组织，派生 adapter 在 `protocol/derived_responses.rs`。

- 旧实现已彻底删除：残留扫描 `grep -rn "AIComputeCenter\|model_session\|metadata_updater\|openai_protocol\|claude_protocol" src/frame/aicc` 唯一命中是 README 里的扫描命令本身；main 上的 16 个旧源文件在 HEAD 全部不存在。`todo!/unimplemented!/TODO/FIXME` 零命中。外部调用方（control_panel、opendan、agent_tool、workflow、buckyos-agent CLI）已迁到新 `AiccClient`。
- 验收 runner 配置 `test/aicc_test/aicc_acceptance.example.toml:21` 只实例化 openai/claude/gemini/fal/minimax/openrouter/sn 七家；kimi/glm/deepseek/doubao/qwen 没有 live 实例。
- 路线图 Tracker（`aicc_reimplementation_roadmap.md:1122-1148`）把 WP-04/05/06/07/08/09/10/13/17 全标 Pending/TBD，却把 T1/T1.5 Gate 标 Done；WP-08 各子组正文写“已完成”，:434 又写“完成配置化整改前 WP-08 不得视为最终完成”；第 10 节旧实现删除清单 13 项全部未勾选（代码其实已删）。这份文档不能用来判断进度。
- 文档级临时项：fal 4 个模型“暂归入 fal.model.json”；豆包/Qwen 原生媒体 operation“另行实施”；MiniMax M3 分段价格、DeepSeek 分时价格、fal 按 compute-second/megapixel 价格均“无法由 schema 表达，未写入”。

## 4. 建议修复顺序

1. **先补一条端到端测试**：加载全部 builtin metadata → InventoryBuilder → apply_version_rules → ModelRegistry，对每个 driver 断言产出的逻辑名集合（golden 文件）。这一条会直接暴露下面 2、3 的问题。
2. 恢复 `{driver}/{model}` 模板展开（参考 main 的 `expand_mount_template`），并让 `validate_logical_path` 拒绝 `{`；version_rules 改用 `origin_model_id`；把 `family/tier/stability` 维度真正注入 match context 或从文档中删掉。
3. 派生 dialect 的 encode 侧反向改写 ProviderState namespace（deepseek/doubao/qwen/minimax），补 decode→encode 往返测试。
4. 扩 `AiUsage`（cache_input、reasoning、images、seconds），结算时用上 `cache_input_token`；把 `estimated_cost_usd` 换成 `Money` 并在路由前真正估算，否则去掉 `max_estimated_cost` 策略。
5. 补 8 家的价格（至少 provider.json 静态 + 币种，含 CNY），fal/minimax 媒体按返回的图数/秒数记 units；OpenRouter 币种从响应读取而非硬编码。
6. 修 metadata 内容：openai gpt-5.6 矛盾、static 挂 current 目录、qwen 零挂载、kimi-code 空转；把删掉的 openai/gemini pattern 兜底恢复，避免未知模型被静默丢弃。
7. Provider 实例支持多 adapter，或至少把 minimax/openrouter 的拼装改成显式声明；fal/glm 接线 catalog-only discovery；Gemini/MiniMax 视频 cancel；豆包 endpoint id 映射。
8. 文档：导出 71 条绑定为“provider × api_type × operation × codec”表；补 version_rules 字段；更新 Tracker 与逻辑模型目录文档；写“需要模型名转换的 provider”清单；统一 dialect 机制并把 HTTP 错误映射上提到 `error.rs`。

## 5. 验证记录

```bash
git clone --branch refactor/aicc --single-branch https://github.com/streetycat/buckyos.git
cd src && cargo check -p aicc --all-targets   # Finished, 0 warnings
cargo test -p aicc                            # 408 passed; 0 failed
```

在线版本：https://claude.ai/code/artifact/17e3298b-b227-4b07-aa41-46e3301f8bd1
