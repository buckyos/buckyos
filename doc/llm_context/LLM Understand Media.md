# `llm_understand_media` 技术需求文档

| 项目 | 内容 |
|---|---|
| 文档版本 | v0.5（2026-11-05：§4.5 / §4.8 改为 Workflow Service 系统内置 pipeline + FunctionObject，按 [notepads/llm-media-pipeline-workflow-todo.md](notepads/llm-media-pipeline-workflow-todo.md) 回写；v0.4 并入发送前决策与目录表示） |
| 所属系统 | OpenDAN / BuckyOS — `agent_tool` 受控 `llm_*` 工具族 |
| 组件类别 | `llm_*` 受控旁路工具（封装层提供，非开放 fork 原语） |
| 实现位置 | [src/frame/agent_tool/src/llm_understand_media.rs](src/frame/agent_tool/src/llm_understand_media.rs)（agent 旁路）；[src/kernel/workflow/src/functions/media/](src/kernel/workflow/src/functions/media/) 与 [src/kernel/workflow/defs/](src/kernel/workflow/defs/)（系统内置管线，Phase A）。agent 侧接入 §4.6–§4.8 为 Phase B（TODO T4），未验证前不得视为已完成 |
| 依赖前置 | `LLMContext` waist、`OneShotRequest` / `semantic_hash()`、`AiContent::Image`、`ResourceRef::NamedObject`、`ndn_lib::ObjId`；workflow service（内核服务，与 scheduler 同级，不可达视为系统故障）及其 `media.*` 系统定义；`buckyos_api::MediaManifest` |
| 需求来源 | 本文档 v0.3 的设计，加上开发者对 Agent runtime baseline 应携带哪些工具的思考（2026-11-05 Review）；[paios 容器需求 §6.4](doc/app_service/paios容器需求.md) 是当前 baseline 清单，作为可用性约束引用 |
| 状态 | Draft — 供实现；workflow 侧管线（§4.5）已落地代码，agent 侧接入（§4.6–§4.8、§8 第 15–20 项、§10）待实现 |

---

## 0. 与 buckyos 现有基础设施的对应关系（先读这一节）

设计文档里出现的抽象名词，全部映射到现有代码：

| 文档中的概念 | buckyos 实际类型 / 模块 | 位置 |
|---|---|---|
| 集群 content-addressed 对象存储 | `ndn_lib`（NDN 命名对象库，beta2.2 外部依赖） | crate `ndn_lib`，在 `agent_tool` 中以 `use ndn_lib::ObjId` 引入 |
| 内容哈希引用 | `ndn_lib::ObjId`（命名对象 id） | 见 `Cargo.toml` 中 `ndn_lib = { git = "...cyfs-ndn..." }` |
| 对外的资源引用包装 | `buckyos_api::ResourceRef`（`Url` / `Base64` / `NamedObject { obj_id }`） | [aicc_client.rs:119](src/kernel/buckyos-api/src/aicc_client.rs:119) |
| 多模态 content block | `AiContent::Image { source: ResourceRef }` / `AiContent::Document { source, title }` | [aicc_client.rs:324](src/kernel/buckyos-api/src/aicc_client.rs:324) |
| 消息 IR | `AiMessage { role: AiRole, content: Vec<AiContent> }` | [aicc_client.rs:440](src/kernel/buckyos-api/src/aicc_client.rs:440) |
| LLMContext waist | `LLMContext` / `LLMContextRequest` | [src/frame/llm_context/src/context_loop.rs](src/frame/llm_context/src/context_loop.rs)、[request.rs](src/frame/llm_context/src/request.rs) |
| L4 调度器 + 推理边界持久化 | `OneShotRequest` + `LocalLLMContext`（当前实现为 xllm run，见表后说明）；每次推理（Round）前提交 `LLMContextSnapshot`，outcome 边界再提交一次 | [src/frame/agent_tool/src/xllm.rs:181](src/frame/agent_tool/src/xllm.rs:181) |
| 语义哈希（resume 安全性） | `OneShotRequest::semantic_hash()` | [xllm.rs:238](src/frame/agent_tool/src/xllm.rs:238) |
| 旁路用到的压缩 | `llm_compress::compress` / `LlmSummarizeCompressor` | [src/frame/agent_tool/src/llm_compress.rs:139](src/frame/agent_tool/src/llm_compress.rs:139) |
| 工具结果信封 | `AgentToolResult`（`status` + `summary` + `details` + `output`） | [src/frame/agent_tool/src/lib.rs:354](src/frame/agent_tool/src/lib.rs:354) |
| Pending 状态 | `AgentToolStatus::Pending` + `AgentToolPendingReason` | [lib.rs:336](src/frame/agent_tool/src/lib.rs:336) |
| 视觉能力标识 | `features::VISION = "vision"`（AICC 能力标识；当前 LLMContext 桥接层尚未提供 vision requirement 透传） | [aicc_client.rs:100](src/kernel/buckyos-api/src/aicc_client.rs:100)、[ai_runtime.rs](src/frame/opendan/src/ai_runtime.rs) |
| 模型策略 | `ModelPolicy` 当前只承载 preferred / fallbacks / temperature / max_completion_tokens / provider_options；`Requirements.must_features` 由 AICC adapter 根据 tool/json 输出等能力生成 | [llm_context/src/request.rs](src/frame/llm_context/src/request.rs)、[ai_runtime.rs](src/frame/opendan/src/ai_runtime.rs) |
| Workflow DSL | `buckyos_api::workflow_dsl::WorkflowDefinition` / `StepDefinition` / `ControlNodeDefinition` | [workflow_dsl.rs](src/kernel/buckyos-api/src/workflow_dsl.rs)、[src/kernel/workflow/src/dsl.rs](src/kernel/workflow/src/dsl.rs) |
| Workflow executor 表达 | `ExecutorRef` 支持 `service::` / `http::` / `appservice::` / `operator::` / `func::` 与 `/agent/` / `/skill/` / `/tool/` 语义路径 | [workflow_types.rs](src/kernel/buckyos-api/src/workflow_types.rs)、[doc/workflow/executor list.md](doc/workflow/executor%20list.md) |
| 共享表示缓存（转换产物） | workflow service 的 Step cache：`step_cache_key = hash(thunk_obj_id, 工具指纹)`，索引在 service-local SQLite，结果 / manifest 为 Named Object，产物在 workflow zone 桶 `artifacts`（temp，30 天）；活动 Run 按 principal 合并（ensure） | [src/kernel/workflow/src/cache_index.rs](src/kernel/workflow/src/cache_index.rs)、[persistence.rs](src/kernel/workflow/src/persistence.rs)、[orchestrator.rs](src/kernel/workflow/src/orchestrator.rs) |
| 转换函数 | 九个 `media.*` Native FunctionObject（capabilities、probe、fit_image、extract_pdf_text、render_pdf_pages、extract_frames、extract_audio、to_pdf、build_catalog），由 `/tool/media.<fn>@1` 引用 | [src/kernel/workflow/src/functions/media/mod.rs](src/kernel/workflow/src/functions/media/mod.rs) |
| 转换工具探测 | workflow 宿主启动时探测 poppler / ffmpeg / soffice（三平台查找，版本进指纹）；缺工具时对应步骤 Failed(`tools_missing`)，是部署缺陷而非正常降级（基线见 [doc/workflow/media tool baseline.md](doc/workflow/media%20tool%20baseline.md)） | [functions/media/tools.rs](src/kernel/workflow/src/functions/media/tools.rs) |
| Office / 表格的现成解析器 | `import_office`（docx / pptx → Blocks，含标题、表格；图片丢弃并报告）、`import_table`（csv / xlsx / sqlite → TableSources，按 sheet） | [import_office.ts](src/tools/buckyos-agent/commands/import_office.ts)、[import_table.ts](src/tools/buckyos-agent/commands/import_table.ts) |
| 官方工具清单（可用性约束） | poppler-utils、tesseract、ffmpeg 只在 Linux 容器 `extended` 层；soffice 在 nfs_server 侧；Desktop Lite / host 上两者都可能缺席 | [paios 容器需求 §6.4](doc/app_service/paios容器需求.md) |

> 当前实现：模型分析 stage 以 xllm run 执行（`XllmTask::prepare` + `XllmRun::start` / `execute`，run 目录为 `run.json` + `snapshots/`，见 [llm_understand_media.rs](src/frame/agent_tool/src/llm_understand_media.rs)）。代码中已没有 `OneShotRequest` / `semantic_hash()` / `drive_to_terminal`，本文出现这些名称处是早期设计接口，恢复语义以 §5.2 的当前实现说明为准。

> 当前实现（2026-11-05 核对）：图片经 image_fit 缩图后以 `AiContent::Image` 发送；音频先转写为文本；视频按均匀时间采样抽帧（默认 8 帧、上限 16，`LLM_UNDERSTAND_MEDIA_VIDEO_FRAMES`），帧以 base64 内联后临时目录即删；PDF 等其余非归档格式原样作为 `AiContent::Document` 直传，不做模型能力与预算检查，模型不支持时才失败。模型选路由 `LLM_UNDERSTAND_MEDIA_{IMAGE,VIDEO,AUDIO}_MODEL` / `LLM_UNDERSTAND_MEDIA_MODEL` 环境变量与内置默认别名决定，§4.4 的 TOML 路由表尚未实现。内置提示词要求 `Do not call tools`。agent_tool 尚未调用 workflow service（Phase B T4）。

> beta2.2 是 breaking-change 版本：`AgentToolResult` / 各 `AiContent` 变体允许直接扩展，不必为旧调用方留兼容层。

---

## 1. 背景与设计动机

### 1.1 要解决的问题

在长期运行的 chat session 中，用户输入的图片/媒体若以 `AiContent::Image { source: ResourceRef::Base64 { .. } }` 形式永久驻留在主干 `Vec<AiMessage>` 历史里，会带来持续的成本：

- 每次推理（Round）都重新序列化、重新经由 `AiccClient` 上传、重新计入 token 的媒体实体；
- `llm_compress`（[llm_compress.rs](src/frame/agent_tool/src/llm_compress.rs)）当前的策略只对 `AiToolResultContent::Text` 做机械折叠，不会替换历史里的 `AiContent::Image` 块——一旦图片以 base64 形式进入 history，它会原样跟到 session 结束；
- 媒体通常是**一次性消费**的——用户发图提问，agent 看一眼提取信息，之后数十次推理都在处理衍生任务，原始像素不再有信息增量；
- 把高熵的原始 modality 永久背在 history 上，违反 LLMContext "context 里每个 token 都应持续贡献价值" 的经济学原则。

### 1.2 设计结论

媒体实体**只存在一处**（`ndn_lib` 寻址的 NDN 对象），主干 history 中只保留**引用**（`AiContent::Image { source: ResourceRef::NamedObject { obj_id } }`）。对媒体的"理解"通过一个受控的旁路 `llm_*` 工具 `llm_understand_media` 按需触发。该工具：

- 是一个**嵌套的 LLMContext**——内部以 `OneShotRequest` 形式发起，`goal` 是它的 user message；
- 启动时继承父 history 的"提纯快照"（见 §4.2）；
- 在进入模型分析前，确需转换的媒体由 **Workflow Service 的系统内置 pipeline**（`media.<family>` 定义，FunctionObject + LocalThunkRunner）完成机械识别与机械转换；工具只消费其 manifest / 产物再做模型分析（见 §4.5）；
- 采用 **fork-and-discard** 语义：旁路在返回瞬间整体蒸发，主干只看到一对 `AiContent::ToolUse` / `AiContent::ToolResult`；
- 不向 agent 暴露底层 fork 机制，agent 只表达"理解某个媒体"这一意图（intent over function call）。

### 1.3 与既有架构的一致性

- **context purification / side-channel execution**：旁路是一次性认知动作，主干只接收结论不接收过程；
- **L4-only 压缩纪律**（[llm_compress.rs:1](src/frame/agent_tool/src/llm_compress.rs:1) 的模块 doc）：媒体的 materialize（NDN 对象 → 真正像素）与 compaction（像素 → 结构化报告）都是显式的、发生在明确边界（一次工具调用、一次压缩改写）上的事件，不隐式继承；
- **crash-resume 自相似**（设计目标，当前实现见 §5.2）：旁路整体以 `OneShotRequest` 形式发起，`semantic_hash()` 覆盖目标 + 输入消息，与 `LocalLLMContext` 按推理边界提交快照的模型同构。

### 1.4 基础工具的尽力原则（有则用）

`llm_understand_media` 是 Agent 的基础工具：所有 Session、所有形态（容器、Desktop Lite、node-daemon host）都有它。转换能力由 workflow service 在宿主执行；workflow 是与 scheduler 同级的内核服务，不可达即系统故障，不设“workflow 不可达仍需完成媒体理解”的要求（裁决 R2）。“有则用”保留给模型能力与预算选路：

- **最小内核必须极小**：识别 MIME → 选模型 → 把文件（或缩后的图 / 抽出的帧 / 转写文本）直接发给模型 → 产出报告。这条路只依赖 AICC 和模型本身，总是可用；它就是今天已实现的路径。
- **每一层增强都是"有则用"**：共享转换缓存有就命中、没有就本地转；本地转换工具有就用、没有就直传；直传模型不收才报错。缺某个工具不是失败原因，只是少一层优化；某层失败后退回下一层，最终退回直传。
- **只有直传也不行时才失败**，而且错误要具体：缺的是哪种能力、本机探测过哪些工具、调用方可走的回退路径（换模型、先 `import_office`、手动转换）。
- **尽力不等于静默**：每次调用在 `details` 里记录走了哪条路、为什么（§4.6.4）；局部抽样不能声称完整阅读（§4.7）。
- **先探测再承诺**：工具描述、`hello` 能力广告、cmd_skills 只描述当前形态真正具备的能力，不把"有工具时能做"写成"总能做"。

这条原则来自 baseline 取舍本身：Agent runtime 的 baseline 要带哪些工具是运维与镜像层的决定（paios 容器需求 §6.4），本工具不反过来要求 baseline 必须带某个转换器；baseline 带了就多一层优化，没带就少一层。它同时约束下文的设计：§4.5–§4.8 描述的管线、目录表示和共享缓存都是最小内核之上的可选层，任何一层缺席时工具仍应能工作。

---

## 2. 接口定义

### 2.1 工具签名（agent 可见面）

旁路本身是一个注册到 `agent_tool` 的 `llm_*` tool，其 `args` 在 `AiContent::ToolUse.args: HashMap<String, Value>` 中传递：

```jsonc
// args
{
  "media": {
    "kind": "named_object",       // 复用 ResourceRef tag —— "url" / "base64" / "named_object"
    "obj_id": "<ndn_lib::ObjId 字符串形式>",
    "mime_hint": "image/png"      // 可选 override；通常不要求调用方传
  },
  "goal": "解释这个报错是什么意思"
}
```

- agent 只见到 `media` + `goal` 两个 arg。底层的 fork、history 继承策略、预算、内置提示词，**全部对 agent 不可见**。
- `media.resource` 语义复用 `ResourceRef`（[aicc_client.rs:119](src/kernel/buckyos-api/src/aicc_client.rs:119)）；`mime_hint` 是可选 override，不属于 `ResourceRef::NamedObject` 自身，正常路径不要求调用方提供。
- 工具名以 `llm_` 前缀标识其"本质是一个 LLMContext"的语义，与同目录下 `llm_compress` / `llm_explore` / `llm_bash` / `llm_tool_carft` 一致。

### 2.2 `media` 实参的来源约束

`media` 字段必须可解析为 `ResourceRef`，且**强烈建议**为 `ResourceRef::NamedObject`：

- **首选**：`NamedObject { obj_id: ObjId }`——content-addressed、跨 zone/peer 可寻址、可由 `ndn_lib` GC 模型统一管理。
- **可接受**：`Url { url, mime_hint }`——当媒体来源是远端 HTTP(S) 且 buckyos 这一侧不需要长期持有该资源时；但调用方需自承"URL 过期 / 跨设备失效"风险，本工具不为其增引用计数。
- **可接受（容器内产物）**：`{kind: "local_file", path}`——上一步 shell 产出的文件。它没有 NDN 身份；需要转换时先上传为对象再调用管线（§4.8.3），直传不强制上传。
- **禁止作为长期 history 引用**：`Base64 { mime, data_base64 }`——一旦写回主干 `tool_result`，base64 永久驻留 history，违背本设计的核心收益。`llm_understand_media` 在写回时**必须**剔除 inline base64。详见 §6.1。
- **MIME 来源**：模型路由必须基于媒体 MIME / media kind，而不是仅基于 `obj_id`。`Url` 可使用 `mime_hint` 或 HTTP `Content-Type`，`Base64` 自带 `mime`；`NamedObject` 当前不在 `ResourceRef` 内携带 MIME，工具应在 materialize / open chunk reader 时从 FileObject meta 获取 MIME，缺失时读取开头字节做 magic sniff。调用方传入的 `mime_hint` 只作为 override / fallback，不是常规必填参数。

> 实现范围（2026-11-05）：图片、音频（先转写）、视频（抽帧）、文档（`AiContent::Document` 直传）均已接入；归档必须先解包。§4.6 起的发送前决策、manifest 消费与经 workflow 的转换为 Phase B 待实现（TODO T4）。

### 2.3 `ToolResult`（写回主干的内容）

旁路对外的返回严格遵循 `AgentToolResult`（[lib.rs:354](src/frame/agent_tool/src/lib.rs:354)）：

```rust
AgentToolResult {
    agent_tool_protocol: "1",
    tool: Some("llm_understand_media"),
    status: AgentToolStatus::Success | Error | Pending,
    summary: String,          // 一行人类可读总结（成为 fallback to LLM）
    details: Json,            // 序列化后的 UnderstandingReport（见 §3）
    output: Some(String),     // 喂回 LLM 用的纯文本渲染（见下方约束）
    pending_reason: Option<AgentToolPendingReason>,  // §5.3
    ..Default::default()
}
```

写回主干 history 时由 caller 把 `AgentToolResult` 装配进 `AiContent::ToolResult`：

> **硬约束**（沿用 `agent_tool` 现有纪律）：写回 LLM 的 `AiContent::ToolResult.content` **只塞 `AiToolResultContent::Text { text: output }` 一个块**——`output` 是 report 的紧凑文本渲染（或 `summary` 作为 fallback）。**不要**把整份 `AgentToolResult.to_value()` 序列化进 tool_result——否则随着 session 推进历史压不下来。`details` 是结构化报告原文，仅留给 worklog / replay，不进 LLM。

并且**绝对不允许**：`tool_result` 中包含任何 `AiToolResultContent::Image`、`base64` 字符串、或任何形式的 media payload。这是装配层硬约束，由 `llm_understand_media` 自身在生成 `AgentToolResult` 时强制满足。

---

## 3. 理解报告结构（`UnderstandingReport`）

报告结构由 `llm_understand_media` 的**内置 system prompt** 强制约束。agent 不感知、不可配置此结构；它只提供 `goal`。

### 3.1 两段式结构（写进 `AgentToolResult.details`）

```rust
struct UnderstandingReport {
    observations: Vec<ObservationItem>,   // 第 1 段：你看到了什么
    reasoning:    String,                 // 第 2 段：如何从所见推出结果
    conclusion:   String,                 // 针对 $goal 的最终答复
    confidence:   Confidence,             // Observed | Inferred | Uncertain
}

struct ObservationItem {
    id:          String,    // 可寻址标识，如 "obs-3"，供 reasoning 引用
    description: String,    // 对该元素的客观描述
}
```

`UnderstandingReport` 自身的紧凑文本渲染（按 `observations` → `reasoning` → `conclusion` → `confidence` 顺序拼接）作为 `AgentToolResult.output` 字段，是真正进 LLM 的内容。

### 3.2 第 1 段 — `observations`（你看到了什么）

- 对媒体内容的**客观元素清单**，每项带可寻址 `id`。
- 作用：为主干后续推理留下一份**事实底座**。即便旁路已焚毁，主干 history 中仍保有"媒体里客观存在哪些东西"的记录；若后续追问的细节恰好在清单内，agent 无需重新 fork。
- 要求：描述客观元素，不夹带结论。

### 3.3 第 2 段 — `reasoning`（如何从所见推出结果）

- 显式陈述从 `observations` 到 `conclusion` 的推理链。
- **硬约束：`reasoning` 只能引用 `observations` 中已列出的 `id`，不得引入清单之外的新"事实"。** 若某步推理需要清单外信息，必须显式标注为推测。
- 作用有三：
  1. **解耦"看图"与"基于所见的推理"**——后者作为可复用资产留在主干，agent 后续可直接在主干上复核或质疑推理，无需重新 fork 看图；
  2. **可审计**——当旁路视觉模型产生幻觉（结论与 observations 矛盾）时，主干可发现并纠正；
  3. **抑制 post-hoc rationalization**（见 §3.4）。

### 3.4 内置提示词必须强制的规则

`llm_understand_media` 的内置 system prompt **必须**包含以下约束，实现者不得弱化：

1. **顺序与因果固定**：必须先完整产出 `observations`，再产出 `reasoning`；`reasoning` 在因果上后于、且依赖于 `observations`。
2. **所见在前、推理在后、推理不得引入新事实**：`reasoning` 引用的每个事实都必须能追溯到某个 `observation.id`。
3. **置信度分层**：任何无法仅凭 `observations` 推出的结论，必须在 `reasoning` 中明确标注为"推测"，并使 `confidence` 字段反映之（`Inferred` / `Uncertain`）。
4. **目的**：防止视觉模型先猜出答案、再反向幻觉视觉细节去支撑答案这一常见失败模式。

> 设计意图：报告的两段式不是单纯的"格式要求"。它把旁路的**推理链本身**变成可复用资产，并使旁路的错误**可在主干上被发现和纠正**，而非以不可质疑的黑箱结论形式静默污染主干 history。

### 3.5 报告信息粒度

由于 fork-and-discard，`details`/`output` 是旁路全部价值的**唯一出口**。内置提示词应**鼓励结构化、自足的提取**：

- 在回答 `goal` 字面问题的同时，顺带固化"显然会被追问"的相邻事实（体现在 `observations` 清单的完整度上）；
- 目标是压低重复 fork 的频率。这是在"写回信息量"与"重 fork 成本"之间取一个平衡点。
- 注意：在本系统中重 fork 成本可控（NDN 对象不悬空、深度可控，见 §6、§5.2），故报告不必追求"一次穷尽"；粒度可后续依据真实重复 fork 频率数据调优。

---

## 4. 执行模型

### 4.1 fork-and-discard 生命周期

```
主干 LLMContext
   │
   │  agent emit AiContent::ToolUse {
   │      name: "llm_understand_media",
   │      args: { media, goal }
   │  }
   ▼
┌──────────────────────────────────────────────────────────────┐
│  旁路 = OneShotRequest（agent_tool::xllm）       │
│    objective = goal                                          │
│    input = [                                                 │
│      AiMessage::text(System, BUILT_IN_PROMPT_V1),            │
│      ...父 history 的提纯快照（§4.2）...,                       │
│      AiMessage::new(User, vec![                              │
│          AiContent::Image { source: media },                 │
│          AiContent::text(goal),                              │
│      ]),                                                     │
│    ]                                                         │
│    model_policy = ModelPolicy {                              │
│        preferred = route_by_mime(media_mime), ..             │
│        // 当前不能在此直接声明 features::VISION requirement │
│    }                                                         │
│    tool_policy = ToolPolicy { allow_tools: false, .. }       │
│  → workflow media.<family> Run（确需转换时）                  │
│      capabilities -> probe -> convert -> build_catalog        │
│  → model stage uses LocalLLMContext::drive_to_terminal        │
│  → 产出 UnderstandingReport                                   │
└──────────────────────────────────────────────────────────────┘
   │
   │  旁路返回 → LocalLLMContext 的工作目录连同 materialize 的
   │  媒体像素一起蒸发（discard 语义见 §5）
   ▼
主干 LLMContext
   仅新增: AiContent::ToolUse + AiContent::ToolResult { content: [Text { output }] }
```

- 旁路里那次完整管线执行（含媒体像素、派生 artifact、传统分析中间结果和模型推理）在返回瞬间随 OneShot 工作目录蒸发。主干 history 永远只留一对 `ToolUse` / `ToolResult`。
- 旁路对主干**无副作用**——除了最终写回的 `tool_result`。
- 旁路中的模型分析 stage 受 xllm run 的推理边界快照保护（每次推理前提交）；其前面的确定性本地 stage 可在恢复时按 pipeline hash 重算（见 §5）。
- 旁路中的推理（包括 §4.2 压缩父 history 的摘要调用和模型分析 stage）是这次工具调用内部的嵌套模型调用，归属关系见 §5.2。

### 4.2 父 history 继承策略（提纯投影，非全量拷贝）

旁路子 context 继承父 history 时，遵循统一原则：

> **`llm_*` 工具的子 context，继承的是父 history 的"语义"，不继承父 history 的"原始 modality 与冗余"；它向父 history 写回的，只有结论文本，不含任何 modality payload。**

具体规则（实现侧）：

1. **modality 降级**：遍历父 history 中每条 `AiMessage`，对其 `content` 中所有 `AiContent::Image` / `AiContent::Document` 进行如下替换：
   - **唯一例外**：本次 `goal` 显式 target 的那条 `ResourceRef`，在旁路构造的"最后一条 User message"中以原始 `AiContent::Image { source: media }` 出现（真实像素）。
   - **其余所有媒体块**降级为 `AiContent::Text { text: "[media omitted: obj_id=..., mime=...]" }` 占位（**不**把 `ResourceRef` 透出给子 context 的视觉模型）。
   - 即："history 可见"与"history 里的 media 可见"是两件事，默认只给前者。

2. **快照而非引用式共享**：子 context 的 `input` 是父 history 在 fork 时刻深拷贝出来的 `Vec<AiMessage>`，而非对父 `LLMContext` 的活引用。理由：
   - 与 `LocalLLMContext` 按推理边界提交的快照一致：旁路 input 是一份确定的数据，可重放；
   - 父 context 可能在旁路执行期间继续推进（并行 agent loop），快照语义明确、可重放；
   - 该快照参与 `OneShotRequest::semantic_hash()`（[xllm.rs:238](src/frame/agent_tool/src/xllm.rs:238)），自动获得 resume 兼容性保护。

3. **超长父 history → 走 `llm_compress`**：本工具构造旁路 input 前，**应**调用 `llm_compress::compress`（[llm_compress.rs:139](src/frame/agent_tool/src/llm_compress.rs:139)）把降级后的快照压到目标预算内，再喂给 OneShot。这复用 OneShot 自身的 graceful-degrade 策略，不引入新逻辑。压缩边界与 head-keep / hot-tail 规则全部沿用 `llm_compress` 已有约定。

4. **（可选优化，非本期必须）目的导向裁剪**：长 session 下可能希望根据 `goal` 进一步裁剪父 history 切片。本期实现先做"全量降级快照 → llm_compress"，若旁路成本仍显著再引入语义裁剪。**实现者应将此预留为可插拔策略点。**

### 4.3 native vision vs 辅助降级

旁路子 context 先根据 target `media` 的 MIME / media kind 选择内容形态与 AICC 逻辑模型名。图片把 target `media` 以 `AiContent::Image { source }` 直接放入最后一条 user message（音频先转写、视频抽帧、文档以 `AiContent::Document` 直传，见 §0 表后说明；是否直传由 §4.6 决定），由 AICC provider 适配层负责 lowering（NamedObject → 字节流 → provider 原生 image content block / Gemini parts / Anthropic image 等）。

- 当前实现状态：`LLMContextRequest.model_policy` 对应的 `ModelPolicy` 不包含 `requirements` / `must_features` 字段；OpenDAN 的 `AiccLlmClient` 只会根据 tool calling / JSON output 自动生成 `Requirements.must_features`，尚不能从 `llm_understand_media` 直接声明 `features::VISION`。
- v0 实现方式：先解析 / 获取 MIME；若不是 `image/*` 或 MIME 缺失且无法确认是图片，返回明确 `AgentToolStatus::Error`。根据配置表把 MIME pattern 映射为 AICC 逻辑模型名，并写入 `OneShotRequest.model_policy.preferred`。若当前 AICC 路由无法为该逻辑模型处理含 image 的 LLM 请求，也应返回明确 Error。
- 后续若需要强制 vision-capable provider，应在 `LLMContext` → `AiccLlmClient` 边界增加能力 requirement 透传；当前阶段通过 MIME → AICC 逻辑模型名配置选路，工具本身不硬编码 provider 模型名。
- **不得**在主模型已原生支持 vision 时，仍强制把图片经辅助视觉模型转文字再喂入——这会引入不必要的延迟、成本与信息损失。
- 仅当 buckyos 当前可路由的模型都不支持 `features::VISION`，或对象大小/格式不被该 provider 接受时，才回退到"辅助视觉模型预转文字"路径。该回退路径本期可作为 TODO，先以 `AgentToolStatus::Error` + 明确 message 报错。
- 该路由决策由 AICC route policy 驱动（[aicc_client.rs:215](src/kernel/buckyos-api/src/aicc_client.rs:215)），不在 `llm_understand_media` 内部硬编码模型名。

### 4.4 MIME → AICC 逻辑模型配置

`llm_understand_media` 不直接硬编码 provider / exact model，而是读取一份 MIME pattern 到 AICC 逻辑模型名的配置。匹配顺序从上到下，首个命中项生效；未命中则返回明确 Error。

```toml
[llm_understand_media]
default_model = "llm.media"

[[llm_understand_media.routes]]
mime = "image/*"
model = "llm.vision"

[[llm_understand_media.routes]]
mime = "application/pdf"
model = "llm.document"

[[llm_understand_media.routes]]
mime = "audio/*"
model = "llm.audio"

[[llm_understand_media.routes]]
mime = "video/*"
model = "llm.video"
```

- 路由表按 MIME 选逻辑模型；选中模型后能否直传、要不要缩小或转换由 §4.6 决定，不在路由表里表达。
- `model` 是 AICC 逻辑模型名，最终 exact provider / model 由 AICC route policy 解析。
- `default_model` 仅在 MIME 已识别但没有更具体 route 时使用；MIME 无法识别时不盲目 fallback。
- 对 `NamedObject`，MIME 探测发生在打开 chunk reader / materialize 阶段：优先 FileObject meta，其次首块 magic sniff，最后才使用调用方 `mime_hint`。

> 当前实现：路由由环境变量与内置默认别名决定（见 §0 表后说明），TOML 路由表待实现。选中模型后的能力与预算检查见 §4.6；能力信息查不到时按"有则用"假定支持、直传，失败再降级。

### 4.5 媒体表示管线：Workflow Service 的系统内置 pipeline

`llm_understand_media` 的长期定位不是“图片转文字”的单点工具，而是 OpenDAN / BuckyOS 理解非结构化数据的旁路。通用执行顺序为：

```text
机械识别(probe) -> 机械转换(convert) -> 模型分析(model analysis)
```

2026-11-05 第二次裁决（[notepads/llm-media-pipeline-workflow-todo.md](notepads/llm-media-pipeline-workflow-todo.md) P2 / P5）：机械识别与机械转换**由 Workflow Service 的系统内置 pipeline 承载**，不再在 `agent_tool` 进程内实现一个本地 runner，也不复用 nfs_server preview 组件（只借鉴其 ensure / work key / descriptor / 状态机的处理模型）。模型分析仍在 `llm_understand_media` 旁路内进行，不进管线。

- **管线 = `WorkflowDefinition`**：四条系统定义 `media.image` / `media.pdf` / `media.video` / `media.document` 随 workflow service 发布（`src/kernel/workflow/defs/media.*.json`，启动时播种，`source: system`，只读）。
- **一次转换 = 一个 Run**：`llm_understand_media` 调 `workflow.create_run` 创建 Run；输入是内容寻址对象，产物是 NDN 对象。
- **stage = `autonomous` Step**，executor 是 `/tool/media.<fn>@1` 语义链接，编译期经 FunctionRegistry 绑定到真实 FunctionObject（`func:<hex>`）。这是 FunctionObject / Thunk 通道的第一次真实落地：九个 `media.*` Native 函数在 workflow 进程内的 `LocalThunkRunner` 执行。
- **跨 Run 复用 = Orchestrator 的 Step cache**：`step_cache_key = hash(thunk_obj_id, 执行指纹)`，持久化在 workflow 的 SQLite 索引里，命中前校验结果对象与全部产物仍可读。
- **可观测**：Run / Step / Thunk 进 task_manager 任务树，管理员可在 TaskMgr / Workflow WebUI 观察（WebUI 真实接入为 Phase B，见 §10）。

#### 4.5.1 与 Workflow Service 的分工

| 维度 | Workflow Service（系统内置 pipeline） | `llm_understand_media`（agent_tool 旁路） |
|---|---|---|
| 做什么 | probe、缩图、PDF 文本 / 页图、视频抽帧 / 音轨、Office → PDF、目录 manifest | 选路（直传 / 转换）、读取 manifest 与产物、检索与视觉覆盖、模型分析、写回 `AgentToolResult` |
| 身份 | 已验证的调用 principal（`user_id/app_id`）是 Run 的 `requested_by`；workflow 是实际执行主体，Task creator 如实记 workflow | 以 Agent 自己的 session token 调 RPC，不能自报 owner |
| 输入 | `{obj_id, 参数}`；服务端把 `obj_id` 解析为 FileObject.content（chunk / chunklist id），调用者不能直接指定 `content_id` | 保留 `ResourceRef::NamedObject` 引用；`local_file` / URL 先冻结成对象再调用 |
| 产物 | workflow zone 桶 `artifacts`（temp，30 天）；manifest / 结构化结果为不可变 Named Object | 只持有 obj_id；不内联媒体字节进 history |
| 失败 | 结构化错误 `{code, message, details, retryable}` 贯穿 Native → 回执 → Run / Step / Thunk Task → RPC；本批定义 `max_attempts=1`、`fallback=abort`，不会进入 `WaitingHuman` | workflow 不可达按内核服务故障报错，不建本地备用转换路径（裁决 R2） |

#### 4.5.2 四条系统定义

每条定义的 step 全部 `autonomous`，显式 `idempotent`，`guards.retry = {max_attempts: 1, fallback: abort}`；`trigger.input_schema` 声明调用输入及默认值（`additionalProperties: false`，不含 `content_id`），`trigger.output_schema` 是服务端补齐后的 `${trigger.output.*}` 形态。

| 定义 | 有效输入（默认值） | steps | Run output（终端节点） |
|---|---|---|---|
| `media.image` | `obj_id`、`max_long_edge` 2048、`max_pixels` 4194304、`max_bytes` 3670016 | capabilities → probe → fit_image | `fit_image`: `{model_media_obj_id, mime, original, sent, actions}` |
| `media.pdf` | `obj_id`、`dpi` 144、`max_long_edge` 2048、`page_range` `[]`、`limits.max_pages` 50 | capabilities → probe → extract_pdf_text → render_pdf_pages → build_catalog | `build_catalog`: `{manifest_obj_id, summary, complete}` |
| `media.video` | `obj_id`、`strategy` uniform、`max_frames` 16、`frame_max_edge` 1280、`limits.max_duration_ms` | capabilities → probe → extract_frames → extract_audio → build_catalog | 同上；无音轨为正常 `absent`，ASR 留在 agent_tool |
| `media.document` | `media.pdf` 的输入 + `format`（docx / pptx / odt / odp / doc / ppt / rtf / txt，必填） | capabilities → probe → to_pdf → PDF 三步（以 `${to_pdf.output.pdf_content_id}` 为源） | 同上；manifest 同时保留原源与派生 PDF 内容 ID |

`capabilities` 是 `idempotent: true` 但 `is_pure: false` 的函数：每次 Run 都真实执行，输出本机工具环境事实，永不进缓存。其余函数只在 `step.idempotent && function.is_pure` 时读写缓存。`to_pdf` 的确定性未验证前按 `is_pure: false` 注册（见 `src/kernel/workflow/src/functions/media/document.rs`，核对实际值）。

#### 4.5.3 `media.*` FunctionObject（首批 9 个）

注册在 `src/kernel/workflow/src/functions/media/mod.rs`，`content = "<name>/impl-1"`，`func_type = native`，参数以 `params_schema` 为权威、`params_type` 只标出 `content_id` 这类 `CheckByRunner("file_content")` 引用。Native 行为变化必须升级 `impl-<n>`，老 Run 不会悄悄绑定新函数。

| 函数 | 关键参数 | 结果 | 工具（requirements） |
|---|---|---|---|
| `media.capabilities` | 无 | `{revision, tools, missing}` | 无；读取环境，非 pure |
| `media.probe` | `content_id`、`family` | `{kind, mime, size, width?, height?, pages?, duration_ms?, has_audio?, has_video?, format?}` | 按 family 用 pdfinfo / ffprobe |
| `media.fit_image` | `content_id`、`max_long_edge`、`max_pixels`、`max_bytes` | `{model_media_obj_id, mime, original, sent, actions}` | image crate；非常见格式可选 ffmpeg |
| `media.extract_pdf_text` | `content_id`、`page_range`、`max_pages` | `{pages: [{page, status, text_obj_id?}], total_pages}` | pdfinfo、pdftotext |
| `media.render_pdf_pages` | `content_id`、`dpi`、`max_long_edge`、`page_range`、`max_pages` | `{pages: [{page, status, image_obj_id?, width, height, error?}], total_pages}` | pdfinfo、pdftoppm |
| `media.extract_frames` | `content_id`、`strategy`、`max_frames`、`frame_max_edge`、`max_duration_ms` | `{frames: [{index, time_ms, image_obj_id, status}], duration_ms}` | ffprobe、ffmpeg |
| `media.extract_audio` | `content_id`、`max_duration_ms` | `{status: ok | absent, audio_obj_id?, codec?, duration_ms?}` | ffprobe、ffmpeg |
| `media.to_pdf` | `content_id`、`format` | `{pdf_content_id, pdf_obj_id, pages}` | soffice（headless，独立 profile，并发 1）；pdfinfo 数页 |
| `media.build_catalog` | `family`、`source_content_id`、`derived_content_id?`、`probe`、`text?`、`images?`、`frames?`、`audio?` | `{manifest_obj_id, summary, complete}` | 无 |

共同约定：

- 参数里的输入身份是 FileObject.content（chunk / chunklist ID），相同字节换文件名、owner、mtime 后仍命中；FileObject ID、run_id、请求者、授权留在 run meta。
- 产物以确定性 FileObject 写入（时间字段为 0、名称稳定），pure 函数的桶 key 为 `media/<step_cache_key>/<name>`，非 pure 为 `exec/<execution_id>/<name>`。
- 页码 / 时间戳需要明确属于哪个源（原件或派生 PDF），见 §4.6.2 manifest。
- 被执行预算截断或含可重试条目错误的结果 `cacheable: false`：可交付，不写可复用缓存。

#### 4.5.4 缓存身份

```text
fun_id         = func:<JCS(FunctionObject)>      # func_type / content / is_pure / params_type / params_schema / result_type / result_schema
thunk_obj_id   = thunk:<JCS({fun_id, params})>    # 跨 Run、跨重试相同
step_cache_key = hash(thunk_obj_id, fingerprint)  # fingerprint = 该函数实际依赖的工具版本（cmd/<tool> -> version）
```

name / version / description / timeout / requirements 在 `FunctionRegistration` 里，不进任何 hash：只改这些不使缓存失效。缓存索引保存 `key → result_obj_id / artifact_refs / fingerprint / expires_at`（默认 30 天）；命中前读回结果对象并按 NDN 内容身份校验，再逐个检查产物可用性，失效即删索引重算。metrics、耗时只写 run meta。

#### 4.5.5 执行与资源

- `LocalThunkRunner`：`schedule_thunk` 只做有界入队（`max_queue` 64）即返回；执行任务按 lane（Light / Image / Pdf / Video / Office）取槽，默认 Office 并发 1、Video 1、Pdf 2、Image 2。
- 执行前校验：环境指纹与投递时一致（否则 `environment_changed`）、`requirements` 的工具存在（否则 `tools_missing`，details 列出缺项与搜索路径）、params 通过 `params_schema`、`CheckByRunner` 内容可读；结果通过 `result_schema` 才回灌与写缓存。
- deadline 取 Run `guards.max_duration`、Step `guards.timeout`、函数 `timeout` 中最早者；超时或取消时 kill 并 wait 外部进程、清理 `data/cache/workflow/exec/<execution_id>` 临时目录、释放槽后才发失败回执。
- 回执按 `execution_id = <run_id>/<node_id>/<attempt>` 关联；重复、迟到、终态后的回执被拒绝，不复活 Run、不写缓存。
- 重启：SQLite 记录执行 epoch 与活动执行；新实例把旧 epoch 的本地执行置 `interrupted` 并清理临时目录，不触碰其他任务。

#### 4.5.6 用户可扩展性

本期只开放系统内置定义；用户不能提交 `media.` / `sys.` 前缀的 id，也不能自报 `source: system`。后续扩展遵循同一路径：注册新的 FunctionObject（Native 或其它 FunctionType）到 registry，再在定义里以 `/tool/<name>@<version>` 引用；不允许在定义里写命令行模板。

### 4.6 发送前决策：直传 / 缩小 / 转换

给定一个文件，模型分析之前先回答三个问题。这是 `llm_understand_media` 在工具进程内完成的规划步骤，agent 不可见；调用方仍只表达"理解这个文件"的目标。

| 问题 | 判断依据 | 不能偷懒的地方 |
|---|---|---|
| 1. 能直接发给目标模型吗？ | 实际选中的模型与 provider 的输入格式、模态和大小限制 | 不能只看扩展名或模型别名；能力信息有就查，没有就按 route 配置假定支持、直传，失败再按下一层处理 |
| 2. 需要先缩小以节约 token 吗？ | goal、分辨率、页数、时长、剩余上下文预算 | 模型支持原格式不等于应该整文件直传 |
| 3. 需要先转换格式吗？ | 原格式不支持、成本过高或不便检索 | 转成页图 / 分页文本 / 帧 / 音轨，由 manifest 组织（§4.6.2） |

统一流程：轻量 probe → 判断模型能力与预算 → 需要转换时 `create_run(media.<family>, ensure=true)` → 读取 manifest 按 goal 选取内容 → 理解并返回带来源的结果。

#### 4.6.1 决策阶梯

```text
轻量 probe: mime / size（不整文件读入、不 base64）
  │
  ├─ 预算内、模型原生支持 ──────────────→ 直传，不建 Run
  │
  ├─ 原生支持但超预算
  │     图片 → media.image（fit_image）
  │     视频 → media.video（抽帧 + 音轨）；长视频按时间段分批
  │     PDF / 文档 → media.pdf / media.document，按 page_range 分批
  │
  ├─ 原生不支持或不便检索 → 对应 family 的 Run；读取 manifest
  │
  └─ 转换失败（缺工具 / 超时 / 资源超限）→ 只有模型确实支持原格式且预算允许才再试直传，
     否则 AgentToolStatus::Error，message 带 workflow 的结构化错误（code / details.tools_missing …）
```

workflow 不可达属于内核服务故障，直接报告；不触发本地转换或专门的直传容灾路径。已知模型不支持的格式不重复尝试直传。

#### 4.6.2 manifest（目录表示）

`media.build_catalog` 产出的 manifest 是 buckyos-api 的 `MediaManifest`（[src/kernel/buckyos-api/src/media_manifest.rs](src/kernel/buckyos-api/src/media_manifest.rs)，对象类型 `mmanifest`），是纯、可缓存的 Named Object：

```jsonc
{
  "schema_version": 1,
  "family": "pdf | video | document | image",
  "source_content_id": "<原件 chunk / chunklist id>",
  "derived_content_id": "<document → PDF 时的派生内容 id>",
  "probe": { … },
  "range": { "total": 42, "requested": [1, 2, 3], "processed": [1, 2], "failed": [3] },
  "entries": [
    { "kind": "page", "where": { "source": "original | derived", "page": 1 }, "text_obj_id": "…", "image_obj_id": "…", "status": "ok" },
    { "kind": "page", "where": { "source": "original", "page": 2 }, "status": "no_text", "image_obj_id": "…" },
    { "kind": "frame", "where": { "source": "original", "time_ms": 12000, "index": 3 }, "image_obj_id": "…", "status": "ok" },
    { "kind": "audio", "where": { "source": "original" }, "status": "absent" }
  ],
  "complete": false
}
```

- 条目状态：`ok` / `no_text`（扫描页、空白页）/ `absent`（无音轨）/ `unprocessed`（在源中但不在请求范围）/ `failed`（带 `error {code, message, retryable}`）。缺失以状态表达，不让条目消失。
- 正常缺失（无文本、无音轨）函数成功，不 abort Run；部分失败仍能建 manifest，`complete: false`；含可重试条目错误的 partial 不写可复用缓存，后续请求重新执行失败步骤。
- manifest 不含 FileObject ID、workflow_id、revision、凭据、耗时；这些在 Run output / run meta / 交付层。

#### 4.6.3 输入预算与质量策略

- 预算来自父预算的剩余额度（§6.3），不重置。
- 缩小后无法辨认的区域可按需以更高 `dpi` / `max_long_edge` 再建 Run 读取；不能为省 token 静默丢失关键信息。
- 全篇总结分批覆盖各部分再汇总。
- 各模型的默认预算值先按 provider 文档配置，再按真实数据调。

#### 4.6.4 决策记录

每次调用把决策写进 `details.media_decision`、管线事实写进 `details.pipeline_trace`，都不进 `output`：

```jsonc
{
  "media_decision": {
    "mode": "direct | convert",
    "reason": "pdf 42 pages exceeds budget; model accepts image/*",
    "run": { "workflow_id": "media.pdf", "run_id": "run-…", "revision": "<definition_revision>", "reused": true },
    "error": null
  },
  "pipeline_trace": [
    { "node_id": "capabilities", "source": "executed" },
    { "node_id": "render_pdf_pages", "source": "cache", "cache_key": "…" }
  ]
}
```

`source` 取自 Run 的 `node_sources`（`cache` / `executed` / `direct` / `submitted`）；不要求所有步骤都 `cache`。转换失败时 `error` 原样携带 workflow 的结构化错误。

#### 4.6.5 格式支持目标

| 输入 | 转换策略 | 验收重点 |
|---|---|---|
| PDF | 模型支持且预算合适时直传；否则 `media.pdf` 产出分页文本 + 页图 + manifest；扫描页按需 OCR（agent_tool 侧） | 文本 PDF、扫描 PDF、图文混排及长 PDF 均可理解；能找到后部页面并引用页码 |
| DOCX / PPTX / ODT | `media.document`：soffice → PDF，再复用 PDF 三步；页码明确属于派生 PDF | 正文与版式信息可检索；页码只引用派生 PDF |
| Excel | 延后：不把 PDF 当成公式 / sheet / 单元格语义，本期不引入 Office 解析 crate | — |
| 视频 | `media.video`：均匀抽帧 + 音轨；音轨转写仍在 agent_tool 走 AICC ASR | 长视频可按时间检索；无音轨为正常 absent |

### 4.7 目录理解与检索

manifest 进入模型分析时，不能把 summary 或首页当作全文。默认做法是**宿主检索循环**：工具进程读 manifest → 在全部文本条目中检索 goal 相关内容 → 选取相关页图 / 帧 → 分批送模型 → 汇总；旁路模型仍不开工具。受控旁路工具方案待宿主循环有真实数据后再评估。

- 扫描 PDF / 纯视觉视频不能只做词法检索：`no_text` 条目必须用页图 / 帧做视觉覆盖，或在 coverage 中明确报告未覆盖。
- 每条重要 observation 带定位：页码（并说明属于原件还是派生 PDF）或时间戳，作为 `ObservationItem.where`。
- `details.coverage` 由 runner 填写：实际读取的条目、未转换范围（`unprocessed`）、失败范围（`failed`）；`output` 末尾追加一行覆盖说明。局部抽样不能声称完整阅读。

### 4.8 共享表示缓存与授权

转换产物由 workflow 的 Step cache 跨 Run、跨 principal 共享；不再复用 nfs_server preview 的 `repr` / `preview-cache`（裁决 P1：preview 面向浏览器可用，本管线面向 LLM 可用，Target Profile 不同）。

#### 4.8.1 复用规则

- **活动 Run 合并（ensure）**：`create_run(ensure=true)` 以 `hash(principal, definition_revision, 规范化后的 trigger 输入, environment_revision)` 查找活动 Run；同 key 的进行中 Run 直接返回（`reused: true`）。只合并活动 Run；Completed / Failed / Aborted 不复用，终态即释放索引。不同 principal（含同 owner 的不同 Agent）各有自己的 Run，首次并发允许重复计算（裁决 R1）。
- **已完成结果复用**：新 Run 的具备缓存资格的步骤按 §4.5.4 命中 Step cache，事件 `step.completed{source: cache}`；`capabilities` 每次重新执行。
- **失效**：工具版本（指纹）或 implementation revision 改变不命中旧缓存；无关工具变化不影响；产物过期或被删后命中校验失败，重算。

#### 4.8.2 授权

- 请求者凭 session token 验证；`create_run` 用请求者身份（NDM proxy，调用者的 Bearer）检查其可读输入 `obj_id`——知道 obj_id 不等于有读权。
- Run / 定义的读取、启动、取消、输出获取都按 owner 校验，管理员可观察全部；Task 树 creator 是 workflow，`requested_by` 记录已验证调用者。
- `get_run_output` 返回稳定 obj_id 与单独的 `delivery` 块：`artifacts`（obj_id 列表）、`grants`（每个产物的 bucket key / locator / 绑定 principal 的只读 token / `expires_at`）、`principal`、`expires_at`；凭据不进 manifest、cache、trace。
- 缓存命中也不跳过授权；请求者凭据不写共享缓存。

#### 4.8.3 内容身份

- `NamedObject` 的 `obj_id`（FileObject）直接作为输入；服务端解析其 `content` 作为计算身份。
- 容器内 `local_file` 与 URL 没有 obj_id：先上传为对象（scratch，7 天），再以 `obj_id` 调用；确定不了身份就不能进管线。

#### 4.8.4 边界

- 区分可复用的**转换产物**与本次 goal / 模型相关的**理解结论**：理解结论不进 workflow 缓存。
- manifest 中的分页文本是否也让 `read` 工具直接按页读取，见 §9。

---

## 5. 崩溃恢复语义

### 5.1 模型分析 stage 作为 `OneShotRequest`

本地管线中的 probe / preprocess / traditional analysis 是确定性本地 stage；真正进入 LLM 的 model analysis stage 仍然是一个 `OneShotRequest`。该 request 的 `input` 必须包含目标媒体、父 history 提纯快照、机械 metadata、传统分析结果、以及当前 pipeline id/version/hash，使 `semantic_hash()` 自然覆盖：

```rust
// OneShotRequest::semantic_hash() 已 hash:
//   - objective ( == goal )
//   - serde_json::to_vec(&input)
//     ( input 含 pipeline hash + 父 history 提纯快照 + metadata +
//       traditional_findings + 末尾 AiContent::Image { source: model_media } )
// → ResourceRef::NamedObject { obj_id } 或派生 artifact 引用的字节序列自然进入 hash。
```

实现侧不需要另写一套 hash 算法，但必须保证影响模型分析语义的管线版本、预处理参数和传统分析结果都被放入 `OneShotRequest.input`，否则 resume 时无法区分不同管线定义。

### 5.2 恢复规则

下面三个边界彼此独立，不能混为"per-turn 持久化"（术语见 [readme.md](readme.md)）：

- **模型推理 checkpoint**：模型分析 stage 以 xllm run 执行（[xllm.rs](src/frame/agent_tool/src/xllm.rs)），持久化粒度是推理边界：每次推理（Round）开始前由 `InferenceHook` 把 `LLMContextSnapshot` 写入 run 目录的 `snapshots/`，到达 outcome 时再提交一次；不存在"每个 Turn 一份快照"。主干 run 同理：function call 模式在每次推理前 checkpoint，behavior 模式只在 Step 边界 checkpoint（`CheckpointHook`）。probe / preprocess / traditional analysis 应保持幂等或在 `pipeline_trace` 中记录不可恢复错误。
- **工具结果提交**：对主干而言，整个 `llm_understand_media` 只是一次工具调用，身份是 `call_id`。function call 模式下 `ToolUse` / `ToolResult` 随主干下一次推理前的 checkpoint 一起持久化（与同一响应的其它调用同属一个工具批次，共消耗一次工具迭代）；behavior 模式下结果写入当前 Step 的 `action_results`，随 Step 边界 checkpoint 持久化。libopendan 宿主在调用开始前登记 in-flight 记录，只有包含该结果（或显式 `Unresolved`）的 checkpoint 才清除它；之后 flush 时写成 Session worklog 的 `action_result {call_id}`。
- **Turn 完成**：工具结果提交不完成 Step（behavior 模式下 Step 要等它的全部 action 结果就绪），更不完成 Turn。Turn 是否完成只由 Session 在 run 结束时判定。

旁路推理的归属：旁路里的推理（模型分析 stage、§4.2 压缩父 history 的摘要调用）使用工具自己的 `LlmClient` 和 run，是这次工具调用内部的嵌套模型调用：

- 不是主干的 Round：不经过主干 run 的 `LlmClient::infer`，不计入主干 `run.json` 的 `usage.llm_requests` 或 libopendan `static.json` 的 `rounds`，只记在旁路自己的 run 记录里；只有经主干主循环发起的推理才是主干的 Round。
- 不是主干的 Step：主干在 behavior 模式下，这次调用只是当前 Step 的一个 action。
- 不是 Session Turn：旁路没有 Session 输入，不开启、推进或完成任何 Turn。

崩溃时：

- 主干在调用进行中崩溃 → 当前实现不续跑旁路。libopendan 宿主恢复时把没有已持久化结果的 in-flight 调用显式回填为 `unresolved`（效果未知），由主干模型决定是否重新调用；单独运行的 xllm 主干则从上一次推理前的快照恢复，重新推理。旁路的 run 目录不会被主干接手。"旁路状态可恢复时自动 resume"仍是设计目标，校验与接手规则待定（原设计依赖的 `semantic_hash()` 已不存在）。
- 旁路内部中间状态不渗透到主干 recovery 模型。
- 由于 `media` 是 `ResourceRef::NamedObject { obj_id }`、媒体存于 `ndn_lib` NDN 对象，重跑时 media 必然仍可寻址、内容必然未变（`ObjId` 即 content hash），重跑结果语义一致。

> 嵌套 LLMContext 的恢复因此两层互不渗透：主干按自己的推理 / Step 边界快照恢复，旁路有自己的 run 与快照。

### 5.3 `pending` 状态

若旁路需要长耗时操作或用户授权，可直接复用 `AgentToolResult.status = AgentToolStatus::Pending` + `pending_reason`（[lib.rs:336](src/frame/agent_tool/src/lib.rs:336)）：

- `AgentToolPendingReason::LongRunning`：例如大尺寸图像分块理解；
- `AgentToolPendingReason::UserApproval`：例如 goal 触发隐私敏感判断；
- `check_after: Option<u64>` / `estimated_wait: Option<String>` 字段已现成可用。

图片理解通常不触发 pending。长文档 / 长视频经 workflow 转换时，Run 可能超出单次调用的等待预算：`create_run` 短等待后返回仍在 Running 的 Run，工具以 `LongRunning` + `check_after` 返回 Pending，runtime 保存 `run_id`、输入身份、有效参数和 revision，后续查询同一个 Run（`get_run_graph` / `get_run_output`）；只有首次请求、`run_not_found`（服务重启）或明确 `retryable` 的失败才重新 `create_run(ensure=true)`，受总 deadline / 恢复次数限制。

---

## 6. 约束与边界

### 6.1 媒体引用强烈倾向 content-addressed

- **首选** `ResourceRef::NamedObject { obj_id: ObjId }`——可寻址、可完整性校验、由 `ndn_lib` 统一管理生命周期、天然跨 zone/peer。
- **可接受** `ResourceRef::Url { .. }`，但调用方须自承过期 / 跨设备失效风险，本工具不持有引用计数。
- **禁止**任何写回主干 `tool_result.content` 的 `AiToolResultContent::Image` / `Base64` payload；用户原始消息里若是 `Base64`，应在装配层尽早转存为 NDN 对象、改写为 `NamedObject` 再进 history（这是 `AiccClient` 层的职责，不在本工具范围）。

### 6.2 NDN 对象 GC 与引用计数

- **GC 根**：主干 history 中引用某 `obj_id` 的 `AiContent::Image` / `AiContent::Document`（用户原始消息或 `tool_result`）。
- 实现要求：`Msg-Center` 在消息写入 / 持久化时，若消息内含 `ResourceRef::NamedObject`，负责经由 `ndn_lib` 对对应 `obj_id` 增引用计数 / 打 pin。该 pin 挂钩是消息中心的职责，**不在本工具实现范围**。
- `llm_understand_media` 的调用和处理流程**不管理 GC**：它只消费 `ResourceRef`，不新增长期引用根，不负责增删引用计数，也不决定对象回收时机。
- 必须保证：只要主干 history 中仍存在引用该媒体的消息，该 NDN 对象不被回收。否则主干 history 看似完好，重 fork 时对象已悬空。
- 引用计数的根是**主干 history**，不是旁路——旁路本就是临时的，不持有长期引用。

### 6.3 旁路深度控制

- `llm_understand_media` 是封装层提供的、语义明确的受控 `llm_*` 工具，**不是开放给 agent 的通用 fork 原语**。
- 因此"旁路内部是否再触发嵌套 `llm_*`"是**设计时静态可推**的，而非运行时不可控。
- 本期 `llm_understand_media` 的模型分析 stage 构造 `OneShotRequest` 时，必须设：
  - `tool_policy.allow_tools = false`（或同义的"无任何 tool 注册"配置）——旁路内部**不得**再触发嵌套 `llm_*` 旁路（深度上限 = 1）；
  - `budget` 显式继承父预算的剩余额度，**不允许**重置为 default 无上限——这是防"旁路成本黑洞"的硬约束。
- 若未来确需嵌套，必须设定明确的深度上限与预算上限，并接入 L4 scheduler 对 `llm_*` 调用的统一预算账户。

### 6.4 主干 history 的体积特征

正确实现后，主干 history 因媒体理解产生的增长**仅随 `AgentToolResult.output` 的文本长度线性增长**，与"读过多少次媒体"无关，与媒体实体大小无关。这是本设计的核心收益，实现验收时应据此度量（见 §8）。

---

## 7. 典型流程示例

```
用户上传截图：
  → AiccClient 装配层把截图落入 ndn_lib，得 obj_id=O1
  → 用户消息以 AiMessage::new(User, vec![
        AiContent::Image { source: ResourceRef::NamedObject { obj_id: O1 } },
        AiContent::text("这个报错是什么意思?"),
    ]) 写入主干 history
  → 持久化层对 O1 增引用计数（§6.2）

主干推理（第 1 次 Round）:
  → emit AiContent::ToolUse {
        name: "llm_understand_media",
        args: { media: { kind: "named_object", obj_id: "O1" },
                goal: "解释这个报错" }
    }
  → llm_understand_media 启动本地轻量管线：
      probe: 读取 FileObject meta / magic bytes，得到 mime=image/png、
             原始分辨率、size、可得 EXIF 摘要；
      preprocess: 按 executor + O1 + 参数计算 cache key；若已有缩略图 artifact 则直接复用，
             否则在图片超过 provider 限制时生成缩小后的 model_media 并写入 NDN cache；
      traditional analysis: 可选 OCR，提取截图中的可见文本并记录 provenance；
      model: 构造 OneShotRequest（input = pipeline hash + 父 history 提纯快照 +
             metadata + OCR findings + 末尾 User message 含 AiContent::Image { source: model_media }）；
             target media 以原生 AiContent::Image 进入旁路；
             tool_policy.allow_tools = false。
  → model stage 通过 LocalLLMContext::drive_to_terminal → AICC 处理含 image 的 LLM 请求 →
      结合图片、metadata、OCR finding 产出 UnderstandingReport:
        observations: [
          obs-1: 红色错误框,
          obs-2: 文本 "OutOfMemoryError",
          obs-3: 堆栈中反复出现 ArrayList.grow ...
        ]
        reasoning: "由 obs-2 与 obs-3，堆栈在集合扩容处反复出现，
                    结合 obs-2 的错误类型，判断为内存泄漏 ..."
        conclusion: "这是一个内存泄漏导致的 OutOfMemoryError ..."
        confidence: Inferred
  → 装配 AgentToolResult{ status: Success, output: <report 紧凑渲染>, details: <report json>, .. }
  → 主干仅新增: ToolUse + ToolResult { content: [Text { text: output }] }
  → 旁路工作目录蒸发，O1 像素不再出现在主干

主干推理（第 30 次 Round，可能已在之后的 Turn）：怀疑"内存泄漏"判断是否成立
  → 主干 history 中仍有第 1 次 Round 发起的那次调用的 tool_result.text（含 reasoning）
  → agent 直接在主干上复核 reasoning，无需重新 fork
  → 仅当需要 observations 之外的新视觉细节时，才再次 fork(O1 仍可寻址，
      因 §6.2 引用计数挂钩，O1 未被 GC)
```

---

## 8. 实现验收标准

实现完成后，应满足：

1. **接口**：`llm_understand_media` 在 `agent_tool` 中正式注册为一个 `llm_*` 工具，接受 `{ media, goal }` 两个 arg；agent 侧不可见 fork / history 继承 / 内置 prompt。
2. **单点存储**：媒体实体仅存于 `ndn_lib` NDN 对象；主干 history 与旁路 input 中均只持有 `ResourceRef::NamedObject`（或可接受的 `Url`），不持有 `Base64` 副本。
3. **fork-and-discard**：旁路返回后，主干 history 经检查只含一对 `ToolUse` / `ToolResult`；`ToolResult.content` 内仅有 `AiToolResultContent::Text`，**无任何 `Image` / `base64` payload**。
4. **报告结构**：`AgentToolResult.details` 反序列化为 `UnderstandingReport`，含 `observations`（带 id）/ `reasoning` / `conclusion` / `confidence`；`reasoning` 中每个事实可追溯到某 `observation.id`。
5. **写回最小化**：`AgentToolResult.output` 是 report 的紧凑文本渲染，`ToolResult.content` 只塞一个 `Text` 块（沿用 `agent_tool` 现有约定）；不出现 `to_value(AgentToolResult)` 全量塞进 LLM 的反模式。
6. **提纯继承**：旁路子 context 的 `input` 中，除 target `media` 外，父 history 的其他 `AiContent::Image` / `AiContent::Document` 全部已被替换为 `Text` 占位。
7. **系统内置管线**：确需转换的媒体经 workflow `media.<family>` Run 完成 probe 与转换；`media.image` 的 probe 输出原始分辨率、size、MIME，`fit_image` 在图片过大时生成适合 vision provider 的派生图并返回 `model_media_obj_id`。
8. **Workflow Service 承载**：四条管线是 workflow 的系统定义（`source: system`，只读，`/tool/media.*@1` 绑定真实 FunctionObject）；agent 只经 `WorkflowServiceClient` 调用，不保留本地 image_fit / ffmpeg 转换双路径。
9. **Step cache**：`idempotent && is_pure` 的步骤按 `hash(thunk_obj_id, 工具指纹)` 复用；命中前校验结果对象与产物可用；`details.pipeline_trace` 按 Run 的 `node_sources` 标记每步来源。
10. **传统分析 provenance**：OCR / ASR / barcode 等传统分析结果若启用，必须作为 `traditional_findings` 进入 `details` 与模型输入，并标明来源 stage / artifact / confidence；最终 report 不得把传统分析结果伪装成模型直接观察。
11. **native vision / MIME route**：工具在 probe / materialize 阶段解析 MIME，按配置表得到 AICC 逻辑模型名并写入模型分析 stage 的 `OneShotRequest.model_policy.preferred`；target 媒体以原生 `AiContent::Image` 进入旁路，无辅助转文字中间层。当前 `ModelPolicy` 不声明 `features::VISION` requirement，若 AICC 无法处理该逻辑模型下含 image 的 LLM 请求，应返回明确 Error（本期不强制实现辅助降级）。后续能力 requirement 透传属于 `LLMContext` / AICC adapter 边界改造。
12. **崩溃恢复**：模型分析 stage 的推理边界快照生效（每次推理前提交）；主干在 `AgentToolResult` 返回前不提交该调用的结果，结果随主干下一个 checkpoint 提交，工具结果提交不等于 Turn 完成（见 §5.2）；模型分析 stage 必须把 `pipeline_id` / `pipeline_version` / `compiled_pipeline_hash` 写入自己的 `OneShotRequest.input`，使 `OneShotRequest::semantic_hash()` 覆盖 (goal, 父 history 快照, obj_id, 管线定义/版本)。
13. **深度 & 预算**：旁路 `OneShotRequest.tool_policy.allow_tools = false`；本地管线和模型分析 stage 共享父预算剩余额度，不能各自重置 default 无上限；外部扩展 stage 必须有 timeout 和权限约束。
14. **体积度量**：构造长 session（多次媒体理解）测试，验证主干 history 体积仅随 `tool_result` 文本线性增长，与媒体数量 / 大小解耦。
15. **发送前决策**：每次调用 `details.media_decision` 记录直传 / 缩小 / 转换的选择、原因、探测过与用到的工具；缺少转换工具时 Error message 列出缺的能力和可用回退路径，而不是泛化失败（§4.6）。
16. **正常选路**：适合直传的内容按模型能力与预算直传，确需转换的内容经 workflow 完成；workflow 不可达按内核服务故障返回，不要求此时仍完成媒体理解；缺基线工具时报告 `tools_missing` 的部署问题而非静默降级。
17. **目录表示**：PDF / DOCX / Excel / 视频可生成 index + 图片 + 文本的目录；空白页、无文本页、转换失败页在 index 中有记录；单文件产物与目录由同一套解析读取（§4.6.2）。
18. **目录检索与覆盖**：答案能命中 PDF 后部页面、DOCX 内嵌图片、Excel 非首个 sheet 和视频后段内容；observations 带页码 / sheet 范围 / 时间戳；`details.coverage` 准确报告已检查范围、未覆盖部分和转换缺失项（§4.7）。
19. **共享缓存**：同一 principal 的并发请求合并到一个活动 Run；不同 principal 各有 Run 但复用有效 Step cache（首次并发允许重复计算）；内容 / 参数 / 工具版本 / implementation revision 变化会失效；无权限调用者不能读取输入或产物；docx → PDF → 页图的中间产物在缓存有效期内只生成一次（§4.8）。
20. **集成验证**：四类格式覆盖不能直传、超预算、扫描件、长文件、部分转换失败和工具缺失六种情形（§4.6.5）。

---

## 9. 待定项 / 后续迭代

| 项 | 说明 | 处理 |
|---|---|---|
| 父 history 目的导向裁剪 | §4.2.4，长 session 下控制旁路重喂 token 量 | 预留可插拔策略点；先靠 `llm_compress` 全量压缩兜底，依据真实成本数据再实现 |
| 报告粒度调优 | §3.5，`observations` 完整度 vs 重 fork 频率的平衡点 | 依据真实重复 fork 频率数据调内置 prompt |
| 文档 / 视频 / 音频的发送前决策 | §2.2 直传已接入；§4.6 决策、§4.6.5 格式目标待实现 | 按 §10 T1–T5 推进 |
| 管线执行位置 | §4.5，P0 在 workflow 进程内 `LocalThunkRunner` 执行 Native | Scheduler / Node Runner 的分布式执行（节点指纹、数据访问、任务交接）后置，保留 `ThunkDispatcher` 接缝 |
| 转换产物缓存 | §4.5.4 / §4.8，workflow Step cache | 已落地（SQLite 索引 + Named Object）；跨主体 single-flight 本期不做 |
| 媒体 FunctionObject catalog | §4.5.3，首批 9 个 `media.*` | 已注册为真实 FunctionObject；OCR / ASR / 关键帧去重等后续函数按同一规则新增 |
| 用户扩展 stage | §4.5.6 | 本期只有系统内置定义；用户 / Agent 发布 FunctionObject 与 Zone 级 registry 后置 |
| OCR / 传统分析依赖 | §4.5.3，OCR 可显著降低 vision 漏字 | 不作为 v0 硬依赖；引入新库或系统依赖前需确认，并保持可选降级 |
| 嵌套 `llm_*` | §6.3，本期深度 = 1 | 如需开放，先定深度/预算上限并接入 L4 预算账户 |
| 辅助视觉模型降级 | §4.3 末段，纯文本主模型回退 | 本期返回明确 Error；后续实现时复用 `agent_tool` 已有 `llm_*` 工具调用模式 |
| 目录产物与 `read` 的边界 | `llm_read_media.rs` 占位文件已不存在；`read` 目前把非文本对象一律推给本工具 | 实现 §4.6.2 时确定目录里的分页文本是否可被 `read` 直接读取，使"按需读某页"不必经 LLM 旁路 |
| agent_tool → workflow 调用通路与身份 | §4.8.2 | 经 `runtime.get_workflow_service_client()`，以 Agent session token 调用；输入读取权由服务端按调用者身份检查 |
| PDF / Office 转换执行侧 | §4.5.5 | 在 workflow 宿主执行；宿主与 paios 镜像分别满足 [media tool baseline](doc/workflow/media%20tool%20baseline.md) |
| 目录表示 | §4.6.2 | 已定：`buckyos_api::MediaManifest`（`mmanifest` 对象） |
| 本地文件 / URL 的内容身份 | §4.8.3 | 确需转换时上传为对象（scratch 桶）后以 obj_id 调用 |
| 检索方式 | §4.7，默认宿主循环 | 旁路工具方案待宿主循环有数据后评估 |
| 复用 `import_office` / `import_table` | §4.6.5，TS 实现 | T2 / T4 定调用方式（CLI 或库） |
| 各模型预算默认值 | §4.6.3 | 先按 provider 文档配置，再按真实数据调 |
| 新增第三方依赖 | 任何 OCR / PDF / Office 库 | 按仓库约定单独确认 |

---

## 10. 实施清单（2026-11-05 Review，第七轮更新）

workflow 侧（Phase A）的任务与验收以 [notepads/llm-media-pipeline-workflow-todo.md](notepads/llm-media-pipeline-workflow-todo.md) §4 / §5 为准；本节只列 agent 侧（Phase B）。

- [x] 管线定义与执行（workflow 侧 T1–T3）：四条 `media.*` 系统定义、九个 FunctionObject、LocalThunkRunner、Step cache、ensure、结构化错误、任务树镜像已落地代码；真实工具 / DV 验收见 TODO §5。
- [ ] T4：`llm_understand_media` 接入——`resolve_media` 保留 NamedObject 引用、轻量 probe、按模型能力与预算选路；确需转换时冻结输入并 `create_run(media.<family>, ensure=true)`，按 §5.3 以 run_id 轮询；读取 manifest，词法检索配合无文本条目的视觉覆盖；输出 `media_decision` / `pipeline_trace` / `coverage`；移除本地 image_fit / ffmpeg 转换拷贝；更新 `ui_core`、`xllm`、`aigc_*` cmd_skills。
- [ ] T4b：paios / agent-runtime 工具基线联动（LibreOffice、字体、locale；镜像内冒烟验证）。
- [ ] T5：Desktop Workflow WebUI 只读观测系统定义与 Run。
- [ ] T6b：完整验收（§8 第 15–20 项），标明实际测试过的 provider / 模型 / 平台。

新增第三方依赖已在 TODO §7 R3 授权；XLSX / 新 Office 解析路线仍后置。
