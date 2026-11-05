# `llm_understand_media` 技术需求文档

| 项目 | 内容 |
|---|---|
| 文档版本 | v0.4（2026-11-05：并入发送前决策、目录表示与 preview 共享转换缓存；v0.3 引入本地轻量媒体理解管线，复用 workflow DSL） |
| 所属系统 | OpenDAN / BuckyOS — `agent_tool` 受控 `llm_*` 工具族 |
| 组件类别 | `llm_*` 受控旁路工具（封装层提供，非开放 fork 原语） |
| 实现位置 | [src/frame/agent_tool/src/llm_understand_media.rs](src/frame/agent_tool/src/llm_understand_media.rs)（图片 / 音频 / 视频 / 文档直传已实现；§4.6–§4.8 待实现） |
| 依赖前置 | `LLMContext` waist、`OneShotRequest` / `semantic_hash()`、`AiContent::Image`、`ResourceRef::NamedObject`、`ndn_lib::ObjId`；可选：nfs_server preview `repr` / `get_repr`（§4.8，有则用） |
| 需求来源 | 本文档 v0.3 的设计，加上开发者对 Agent runtime baseline 应携带哪些工具的思考（2026-11-05 Review）；[paios 容器需求 §6.4](doc/app_service/paios容器需求.md) 是当前 baseline 清单，作为可用性约束引用 |
| 状态 | Draft — 供实现；§1.4、§4.6–§4.8、§8 第 15–20 项、§10 为 2026-11-05 Review 并入的待实现需求 |

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
| 共享表示缓存（转换产物） | nfs_server preview：NFSP `repr`（ensure）/ `get_repr`（query），`work_key = sha256(input ObjId, converter@version, params)`，结果存系统桶 `zone/nfs-server/_/preview-cache`，按源 ACL 判权后签短期只读令牌 | [src/frame/nfs_server/src/preview/mod.rs](src/frame/nfs_server/src/preview/mod.rs)、[rpc.rs](src/frame/nfs_server/src/preview/rpc.rs) |
| 转换档（Target Profile） | `Profile::{PreviewImage, PreviewVideo, PreviewAudio, Thumbnail, PreviewDocument}`；`preview-document` 用 soffice 把 Office / ODF / RTF 转 PDF | [convert.rs](src/frame/nfs_server/src/preview/convert.rs) |
| 转换工具探测（有则用） | preview `tools`：ffmpeg / ffprobe / soffice 三平台查找，缺失即 `unsupported`，`hello` 只广告能产出的 profile | [tools.rs](src/frame/nfs_server/src/preview/tools.rs) |
| Office / 表格的现成解析器 | `import_office`（docx / pptx → Blocks，含标题、表格；图片丢弃并报告）、`import_table`（csv / xlsx / sqlite → TableSources，按 sheet） | [import_office.ts](src/tools/buckyos-agent/commands/import_office.ts)、[import_table.ts](src/tools/buckyos-agent/commands/import_table.ts) |
| 官方工具清单（可用性约束） | poppler-utils、tesseract、ffmpeg 只在 Linux 容器 `extended` 层；soffice 在 nfs_server 侧；Desktop Lite / host 上两者都可能缺席 | [paios 容器需求 §6.4](doc/app_service/paios容器需求.md) |

> 当前实现：模型分析 stage 以 xllm run 执行（`XllmTask::prepare` + `XllmRun::start` / `execute`，run 目录为 `run.json` + `snapshots/`，见 [llm_understand_media.rs](src/frame/agent_tool/src/llm_understand_media.rs)）。代码中已没有 `OneShotRequest` / `semantic_hash()` / `drive_to_terminal`，本文出现这些名称处是早期设计接口，恢复语义以 §5.2 的当前实现说明为准。

> 当前实现（2026-11-05 核对）：图片经 image_fit 缩图后以 `AiContent::Image` 发送；音频先转写为文本；视频按均匀时间采样抽帧（默认 8 帧、上限 16，`LLM_UNDERSTAND_MEDIA_VIDEO_FRAMES`），帧以 base64 内联后临时目录即删；PDF 等其余非归档格式原样作为 `AiContent::Document` 直传，不做模型能力与预算检查，模型不支持时才失败。模型选路由 `LLM_UNDERSTAND_MEDIA_{IMAGE,VIDEO,AUDIO}_MODEL` / `LLM_UNDERSTAND_MEDIA_MODEL` 环境变量与内置默认别名决定，§4.4 的 TOML 路由表尚未实现。内置提示词要求 `Do not call tools`。agent_tool 不依赖任何 NFSP 客户端，目前没有到 preview 的调用通路。

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
- 在进入模型分析前，先执行一条**本地轻量媒体理解管线**：机械识别 → 机械预处理 → 传统分析 → 模型分析；管线结构复用 `src/kernel/workflow` 的 DSL，但不复用分布式 Workflow Service 的调度语义（见 §4.5）；
- 采用 **fork-and-discard** 语义：旁路在返回瞬间整体蒸发，主干只看到一对 `AiContent::ToolUse` / `AiContent::ToolResult`；
- 不向 agent 暴露底层 fork 机制，agent 只表达"理解某个媒体"这一意图（intent over function call）。

### 1.3 与既有架构的一致性

- **context purification / side-channel execution**：旁路是一次性认知动作，主干只接收结论不接收过程；
- **L4-only 压缩纪律**（[llm_compress.rs:1](src/frame/agent_tool/src/llm_compress.rs:1) 的模块 doc）：媒体的 materialize（NDN 对象 → 真正像素）与 compaction（像素 → 结构化报告）都是显式的、发生在明确边界（一次工具调用、一次压缩改写）上的事件，不隐式继承；
- **crash-resume 自相似**（设计目标，当前实现见 §5.2）：旁路整体以 `OneShotRequest` 形式发起，`semantic_hash()` 覆盖目标 + 输入消息，与 `LocalLLMContext` 按推理边界提交快照的模型同构。

### 1.4 基础工具的尽力原则（有则用）

`llm_understand_media` 是 Agent 的基础工具：所有 Session、所有形态（容器、Desktop Lite、node-daemon host）都有它，但它周围的能力（nfs_server preview、ffmpeg、soffice、poppler、OCR、ASR、Office 解析器）在不同形态下不一定存在。因此：

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
- **可接受（容器内产物）**：`{kind: "local_file", path}`——上一步 shell 产出的文件。它没有 NDN 身份；要进共享转换缓存须先确定内容身份（§4.8.3），确定不了就只做本次私有转换。
- **禁止作为长期 history 引用**：`Base64 { mime, data_base64 }`——一旦写回主干 `tool_result`，base64 永久驻留 history，违背本设计的核心收益。`llm_understand_media` 在写回时**必须**剔除 inline base64。详见 §6.1。
- **MIME 来源**：模型路由必须基于媒体 MIME / media kind，而不是仅基于 `obj_id`。`Url` 可使用 `mime_hint` 或 HTTP `Content-Type`，`Base64` 自带 `mime`；`NamedObject` 当前不在 `ResourceRef` 内携带 MIME，工具应在 materialize / open chunk reader 时从 FileObject meta 获取 MIME，缺失时读取开头字节做 magic sniff。调用方传入的 `mime_hint` 只作为 override / fallback，不是常规必填参数。

> 实现范围（2026-11-05）：图片、音频（先转写）、视频（抽帧）、文档（`AiContent::Document` 直传）均已接入；归档必须先解包。§4.6 起的发送前决策、目录表示与共享转换缓存为待实现。

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
│  → LocalMediaPipelineRunner                                  │
│      probe -> preprocess -> traditional analysis -> model     │
│      model stage uses LocalLLMContext::drive_to_terminal      │
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

### 4.5 本地轻量媒体理解管线

`llm_understand_media` 的长期定位不是“图片转文字”的单点工具，而是 OpenDAN / BuckyOS 理解非结构化数据的本地旁路。通用执行顺序为：

```text
机械识别(probe) -> 机械预处理(preprocess) -> 传统分析(traditional analysis) -> 模型分析(model analysis)
```

这条管线应复用 `src/kernel/workflow` 已有 DSL 来表达 stage、输入输出 schema、依赖关系和可扩展 executor，但执行时不进入分布式 Workflow Service / Thunk Dispatcher。原因是本工具的执行范围是一次 `llm_*` 旁路调用，生命周期短、工作目录临时、输出只写回 `AgentToolResult`；引入分布式调度会把一次媒体理解扩展成长期任务，破坏 fork-and-discard 的成本边界。

设计原则是：**执行纯本地，结果全局可复用**。`llm_understand_media` 不把本次 stage 投递到远端节点执行，也不等待分布式调度完成；但所有确定性 stage 都通过 NDN / named_store 读取和写入内容寻址结果。因此如果别的节点、别的 workflow、或另一次本地调用已经用相同参数算出某个派生产物，本地 runner 可以直接命中 NDN cache 并复用结果。这让工具保持旁路调用的低复杂度，同时享受系统级分布式计算已经产生的结果。

#### 4.5.1 与 Workflow Service 的边界

| 维度 | 分布式 Workflow Service | `llm_understand_media` 本地管线 |
|---|---|---|
| DSL | 复用 `WorkflowDefinition` / `StepDefinition` / `edges` / `defs` | 同左 |
| 编译 | 可复用 `compile_workflow` 的静态分析与 Expr Tree 语义 | 可复用；也可先实现 profile 校验后逐步接入 compiler |
| 执行器 | Workflow Orchestrator + ExecutorRegistry + Thunk Dispatcher | `agent_tool` 进程内 LocalMediaPipelineRunner |
| 调度 | 支持长期 run、分布式节点、human wait、map/parallel | v0 只支持短生命周期 DAG；无 human wait；无 Thunk；不发起远端执行 |
| 状态 | WorkflowRun / task tracker / object store | 临时 work_dir + `AgentToolResult.details.pipeline_trace` + 可复用的 NDN stage cache |
| 恢复 | workflow run 可恢复、可审查、可人工干预 | 旁路按 `OneShotRequest` 整体恢复或丢弃重跑 |

本地管线可视为 Workflow DSL 的一个受限 profile：共享描述语言，不共享长期运行时。

#### 4.5.2 Local Media Pipeline Profile

本工具支持的 DSL 子集应明确受限，避免把 `llm_understand_media` 变成第二套 workflow engine：

1. `steps` 仅支持 `type: "autonomous"`。`human_confirm` / `human_required` 不在本地管线内执行；
2. `edges` 支持线性 DAG；后续可支持有界并行，但必须受本工具预算和超时约束。
3. `nodes` 中的 `branch` 可作为后续优化，用于按 MIME 选择图片 / 视频 / 文档路径；v0 可先通过代码选择默认 pipeline。
4. `for_each` / `parallel` 不作为 v0 必需能力。视频抽帧、多页 PDF 等批量场景后续可在本地 runner 中以有界并发实现。
5. `executor` 必须使用当前 `ExecutorRef::parse` 已支持的形式。内置 stage 使用 `operator::media.*`；用户可扩展能力优先通过 `/tool/media.*` 语义路径解析到 `appservice::` / `http::` / `operator::`，不要新增 `media::` namespace。
6. 每个 stage 的输出必须是 JSON value，并通过 `output_schema` 约束。二进制派生产物只能以 `ResourceRef` / `obj_id` / 临时 work_dir path 引用，不得内联 base64 写入 `details` 或主干 history。
7. 声明 `idempotent: true` 的 stage 必须接入系统级缓存。cache key 语义与 Workflow Orchestrator 保持一致：`executor identity + resolved input + stage version`，结果写入 NDN / named_store 后用 obj_id 引用。

#### 4.5.3 Stage 分类

| Stage | 典型 executor | 输入 | 输出 | 说明 |
|---|---|---|---|---|
| 机械识别 | `operator::media.probe` | `ResourceRef`、`mime_hint` | `mime`、`size_bytes`、图片宽高、EXIF 摘要、视频时长/codec/fps | 完全确定性，不调用 LLM |
| 机械预处理 | `operator::media.preprocess_image` / `operator::media.preprocess_video` | 原始媒体 + probe metadata + provider 限制 | `model_media`、派生 artifact、缩放/转码参数 | 图片可自动缩小以塞入 vision provider；视频可抽首帧/关键帧 |
| 传统分析 | `/tool/media.ocr` / `appservice::media_ocr.extract` | 原始或预处理 artifact | OCR 文本、layout、barcode、字幕、ASR 等 | 可选 stage；结果带来源和置信度，不能和模型观察混在一起 |
| 模型分析 | `operator::media.model_analyze` | `model_media` + metadata + traditional findings + goal + purified history | `UnderstandingReport` | 内部仍用 `OneShotRequest` / `LocalLLMContext` 调 AICC |
| 汇总渲染 | `operator::media.render_report` | report + pipeline trace | `AgentToolResult.output` 文本 | 控制写回主干的体积 |

> 当前实现：图片的 probe + preprocess 以 Rust 代码内置（[image_fit.rs](src/frame/agent_tool/src/llm_understand_media/image_fit.rs)），尚未走 DSL / stage cache。
>
> - **判断**：只读文件头。长边 ≤ 2048、像素 ≤ 2048²、字节 ≤ 3.5 MiB、格式为 png/jpeg/webp/gif、且无 EXIF 旋转时，原样透传，不重新编码（声明的 MIME 与实际字节不符时只纠正 MIME）。
> - **处理**：固化 EXIF 方向，按长边与像素数中更紧的一个等比缩小，然后编码：有透明或源图无损时优先 PNG，否则 JPEG（质量依次 85/75/60）；仍超字节上限时，每轮把长边乘 0.75 再试，最多 3 轮。解码缓冲超过 512 MiB 直接报错。
> - **格式**：`image` crate 解不了的格式（超限 GIF、TIFF、AVIF…）先用 ffmpeg 转成 PNG（动图取首帧）。aios 镜像的 ffmpeg 6.1 读不了 HEIC，ImageMagick 也缺 HEVC 插件，HEIC 目前会明确报错。
> - **URL 图片**：先下载到工具内再判断。
> - **视频帧**：抽帧时缩到长边 ≤ 1280。
> - **视频抽帧**：ffprobe 取时长后按均匀时间采样，默认 8 帧、上限 16（`LLM_UNDERSTAND_MEDIA_VIDEO_FRAMES`），每帧前有时间戳说明；编码层关键帧不等于语义关键画面，场景切换与去重待评估（§4.6.5）。
> - **音频**：先转写为文本再送模型。**文档（PDF 等）**：原样作为 `AiContent::Document` 直传，未做能力与预算检查（§4.6 待实现）。
> - **配置**：上述限制可用环境变量 `LLM_UNDERSTAND_MEDIA_IMAGE_MAX_EDGE` / `_IMAGE_MAX_PIXELS` / `_IMAGE_MAX_BYTES` / `_VIDEO_FRAME_MAX_EDGE` 覆盖。
> - **结果**：原图与实际送出图的 mime / 宽高 / 字节数，以及所做的处理（`actions`），写入 `details.media_preprocess`。缩过图时，旁路模型在图片前收到一条说明，主干 `output` 末尾也追加一行提示。

#### 4.5.4 本地管线上下文

本地 runner 在各 stage 之间传递一个结构化上下文，stage 输出只追加确定字段：

```rust
struct MediaUnderstandingContext {
    source: ResourceRef,
    goal: String,
    parent_history: Vec<AiMessage>,
    metadata: Option<MediaMetadata>,
    derived_artifacts: Vec<DerivedArtifact>,
    traditional_findings: Vec<TraditionalFinding>,
    model_media: Option<ResourceRef>,
    report: Option<UnderstandingReport>,
    pipeline_trace: Vec<StageTrace>,
}
```

关键约束：

- `metadata` 保存机械识别结果，如原始分辨率、EXIF 摘要、视频时长；这些信息进入 `AgentToolResult.details`，也可作为模型分析的文本上下文。
- `derived_artifacts` 保存预处理产物引用，例如缩小后的图片、抽帧图片、提取出的音轨。长期有价值的产物应写入 NDN；仅服务本次旁路的临时文件必须随 work_dir 清理。
- `traditional_findings` 保存 OCR / ASR / barcode 等传统算法结果，并标明 `source_stage`、`artifact_ref`、`confidence`。模型分析阶段可以引用这些 finding，但最终 report 必须保留 provenance。
- `pipeline_trace` 记录 stage 名、executor、输入摘要、输出摘要、耗时、错误、`cache_key`、`cache_hit`、`result_obj_id`。trace 进入 `details`，不进入 `output`。

#### 4.5.5 NDN + Thunk-style 系统级缓存

媒体处理天然适合内容寻址缓存：输入媒体是 `ResourceRef::NamedObject { obj_id }`，预处理参数是结构化 JSON，stage 版本可显式声明。因此只要 stage 是幂等的，相同输入必然得到相同语义输出。`llm_understand_media` 本地管线应复用 Workflow / Thunk 的缓存思想，得到跨工具、跨会话、跨 workflow 的系统级处理缓存。

这里的“复用分布式计算结果”不是指本工具发起分布式计算。`llm_understand_media` 的 stage 执行仍然是纯本地的；它只是在执行前查 NDN cache，发现已有结果就复用。已有结果可以来自同机的历史调用，也可以来自其它节点执行过的 Workflow / Thunk / AppService，只要对应 cache record 和 artifact 已经通过 NDN 可寻址即可。

缓存 key 规则：

```text
stage_cache_key = hash({
  executor: "operator::media.preprocess_image",
  identity: executor_identity_or_function_object_id,
  version: stage_version,
  input: resolved_input_json
})
```

其中 `resolved_input_json` 必须使用内容稳定的引用：

- 原始媒体使用 NDN `obj_id`，不要使用临时文件路径。
- 派生 artifact 使用其 NDN `obj_id`。
- 预处理参数、OCR 语言、抽帧策略、provider 限制等全部进入 input。
- executor 实现版本、模型无关的算法版本、外部工具版本摘要进入 `version` 或 `identity`。

命中流程：

1. 本地 runner 执行 `idempotent: true` stage 前，先计算 `stage_cache_key`。
2. 到 named_store 查询 `media_stage_cache/<stage_cache_key>` 或等价命名对象。
3. 若命中，读取缓存记录中的 `result_obj_id` / `artifact_refs`，直接回填该 stage output，并在 `pipeline_trace` 标记 `cache_hit: true`。
4. 若未命中，执行 stage；完成后把 JSON output 与需要长期复用的派生 artifact 写入 NDN，再写 cache record。

示例：别的 workflow 已经用同一个原图 `obj_id=O1`、同样的 `max_long_edge=2048`、同样的 `operator::media.preprocess_image@v1` 生成过缩略图 `obj_id=T1`。本次 `llm_understand_media` 走到 `preprocess` stage 时，计算出的 `stage_cache_key` 相同，因此直接复用 `T1`，不再解码、缩放、重新编码图片。

这套缓存语义同时兼容未来的真正 Thunk 路径：当某个 stage 从本地 `operator::media.*` 升级为 `func::<objid>` 时，`FunctionObject + params = ThunkObject` 的内容寻址 id 可作为同一类执行 key；Scheduler / Node Daemon Runner 执行出的结果仍写入 NDN，本地管线只需要按同一 cache record 读取结果。也就是说，本地 runner 不必一开始走分布式 Thunk 调度，但 stage 的命名、输入、版本和结果存储必须从第一天就按 Thunk 可缓存语义设计。

缓存边界：

- `probe`、`preprocess_image`、`preprocess_video`、OCR、barcode、PDF text extraction 等确定性 stage 默认 `idempotent: true`，应启用缓存。
- `model_analyze` 默认 `idempotent: false`，不要默认跨调用复用。即便输入相同，不同时间的 provider、系统 prompt、route policy 也可能导致不同结果；后续若要缓存模型结果，必须显式把 model/provider/prompt/policy 版本纳入 key。
- 涉及隐私策略或用户授权的 stage，其 cache record 必须带 owner / zone / permission scope；不能因为内容 hash 相同就跨权限边界复用。
- cache record 只保存 JSON output 和 artifact 引用，不保存 base64 payload。

> **落地方式（2026-11-05 决定）**：转换类产物（缩图、页图、分页文本、目录、抽帧、转写）的系统级缓存不再单独实现 `media_stage_cache/<key>` 的 named_store 记录，而是复用 nfs_server preview 已有的 `repr` / `get_repr` 与 `preview-cache` 桶（§4.8）。preview 的 `work_key = sha256(input ObjId, converter@version, params)` 与上面的 `executor identity + resolved input + stage version` 同构：`converter@version` 对应 executor identity 与 version，`params` 对应 resolved input 中源对象以外的部分。本节的 key 规则、缓存边界与 `pipeline_trace.cache_hit` 要求不变；`model_analyze` 仍不缓存。preview 不可达时按 §1.4 退到本地转换，不阻塞理解。

#### 4.5.6 从媒体理解提炼的 FunctionObject

这个场景可以反过来帮助定义真正有价值的 FunctionObject。FunctionObject 不应只是数学意义上的低层算子；`operator` 更适合表达 `sort`、`topk`、`json.pick` 这类无业务语义的纯算子。媒体理解需要的是强语义、可复用、可被人类和 agent 识别的函数组件，例如“生产图片缩略图”“探测媒体元信息”“从图片提取可见文字”。

判断一个媒体 FunctionObject 是否成立，使用以下规则：

```text
函数名 + 版本 + 参数 + 输入文件 obj_id = 一个确定的结果 obj_id / JSON output
```

其中“输入文件”必须是 NDN `obj_id`，不能是临时路径；“参数”必须是稳定 JSON；“函数名”必须表达业务语义，而不是实现细节。例如 `produce_image_thumbnail@v1(input=O1, max_long_edge=2048, format=jpeg)` 是一个合格函数；`run_ffmpeg_cmd@v1(args="...")` 不是合格函数，因为它泄漏实现、参数不可治理、缓存语义也不稳定。

首批建议沉淀的 FunctionObject：

| 语义函数名 | 建议语义路径 | 输入文件 | 关键参数 | 输出 | 纯度 / 缓存 |
|---|---|---|---|---|---|
| 生产图片缩略图 | `/tool/media.produce_image_thumbnail` | `image_obj_id` | `max_long_edge`、`max_pixels`、`format`、`quality`、`strip_metadata` | `thumbnail_obj_id` + width/height/format/size | 纯函数，默认缓存 |
| 规范化图片供视觉模型使用 | `/tool/media.prepare_image_for_vision` | `image_obj_id` | `provider_profile`、`max_long_edge`、`max_bytes`、`allow_format` | `model_media_obj_id` + transform report | 纯函数，默认缓存 |
| 探测媒体元信息 | `/tool/media.probe_metadata` | `media_obj_id` | `include_exif`、`include_streams`、`sniff_bytes` | MIME、size、图片尺寸、EXIF 摘要、视频时长/codec/fps | 纯函数，默认缓存 |
| 提取图片 EXIF | `/tool/media.extract_image_exif` | `image_obj_id` | `fields`、`privacy_mode` | EXIF JSON 摘要 | 纯函数，默认缓存；隐私字段受策略控制 |
| 图片 OCR | `/tool/media.extract_visible_text_from_image` | `image_obj_id` | `languages`、`return_layout`、`engine_profile` | text、blocks、layout、confidence | 对固定 engine/profile 视为纯函数，默认缓存 |
| 识别二维码/条码 | `/tool/media.detect_codes_in_image` | `image_obj_id` | `code_types`、`return_regions` | code list + bbox + confidence | 纯函数，默认缓存 |
| 视频元信息探测 | `/tool/media.probe_video_metadata` | `video_obj_id` | `include_streams`、`include_chapters` | duration、streams、codec、fps、resolution、audio tracks | 纯函数，默认缓存 |
| 抽取视频关键帧 | `/tool/media.extract_video_keyframes` | `video_obj_id` | `strategy`、`max_frames`、`interval_ms`、`scene_threshold`、`image_format` | keyframe image obj_ids + timestamps | 固定算法版本下纯函数，默认缓存 |
| 抽取视频首帧 | `/tool/media.extract_video_first_frame` | `video_obj_id` | `timestamp_ms`、`image_format`、`max_long_edge` | frame image obj_id + timestamp | 纯函数，默认缓存 |
| 提取音轨 | `/tool/media.extract_audio_track` | `video_obj_id` | `track_index`、`codec`、`sample_rate`、`channels` | audio obj_id + duration/codec | 纯函数，默认缓存 |
| 音频转写 | `/tool/media.transcribe_audio` | `audio_obj_id` | `language`、`engine_profile`、`return_timestamps` | transcript、segments、confidence | 对固定 engine/profile 视为纯函数；可缓存但需权限 scope |
| PDF 文本抽取 | `/tool/media.extract_pdf_text` | `pdf_obj_id` | `page_range`、`include_layout` | text/pages/layout | 纯函数，默认缓存 |
| PDF 页面渲染为图片 | `/tool/media.render_pdf_pages` | `pdf_obj_id` | `page_range`、`dpi`、`format`、`max_long_edge` | page image obj_ids + page metadata | 纯函数，默认缓存 |

这些 FunctionObject 的共同输出约定：

- 大结果和二进制 artifact 一律写 NDN，返回 `obj_id` 和结构化 metadata。
- JSON output 必须包含 `function_name`、`function_version`、`input_obj_id`、`params_hash`、`result_obj_id`、`metrics`。
- 若函数内部依赖外部工具，如 ffmpeg / tesseract / poppler，必须把工具版本或 engine profile 纳入 `function_version` / `identity`，否则不能跨节点安全复用 cache。
- 如果结果受权限或隐私策略影响，如 EXIF 隐私字段、音频转写、OCR 文本，cache record 必须带 owner / zone / permission scope。

在本地管线中，这些函数可以先由 `operator::media.*` 内置实现或 `appservice::media_tools.*` 直执行；一旦某个函数需要跨节点计算或复用独立 runner，就把语义路径解析到 `func::<function_object_id>`。无论执行路径如何，`函数名 + 参数 + 输入文件 = 确定结果` 的缓存语义保持不变。

表中 `produce_image_thumbnail` / `render_pdf_pages` / `extract_pdf_text` / `extract_video_keyframes` / `extract_audio_track` 这类纯转换函数，当前对应 preview 的转换档（§4.8.1）：函数名加版本即 `converter@version`，关键参数即 `params`。先以 preview profile 落地，FunctionObject 注册表成熟后再把同一 work_key 语义映射到 `func::`。

#### 4.5.7 默认图片管线 DSL 示例

默认图片路径可以用 Workflow DSL 表达如下。这里的 `operator::media.input` 是本地 runner 注入调用参数的虚拟起点；真实实现中它不读取外部系统，只把 `{ media, goal, parent_history }` 规整成下游 schema。

```json
{
  "schema_version": "0.2.0",
  "id": "llm-understand-media-image-default",
  "name": "Default image understanding pipeline",
  "trigger": { "type": "manual" },
  "steps": [
    {
      "id": "input",
      "name": "Normalize invocation input",
      "type": "autonomous",
      "executor": "operator::media.input",
      "output_schema": { "$ref": "#/defs/MediaInvocation" },
      "idempotent": true,
      "skippable": false
    },
    {
      "id": "probe",
      "name": "Probe media metadata",
      "type": "autonomous",
      "executor": "operator::media.probe",
      "input": {
        "media": "${input.output.media}",
        "mime_hint": "${input.output.mime_hint}"
      },
      "output_schema": { "$ref": "#/defs/MediaProbeOutput" },
      "idempotent": true,
      "skippable": false
    },
    {
      "id": "preprocess",
      "name": "Prepare image for model",
      "type": "autonomous",
      "executor": "operator::media.preprocess_image",
      "input": {
        "media": "${input.output.media}",
        "probe": "${probe.output}",
        "limits": {
          "max_long_edge": 2048,
          "max_pixels": 4194304
        }
      },
      "output_schema": { "$ref": "#/defs/ImagePreprocessOutput" },
      "idempotent": true,
      "skippable": false
    },
    {
      "id": "ocr",
      "name": "Extract visible text",
      "type": "autonomous",
      "executor": "/tool/media.ocr",
      "input": {
        "media": "${preprocess.output.model_media}",
        "probe": "${probe.output}"
      },
      "output_schema": { "$ref": "#/defs/TraditionalFindingSet" },
      "idempotent": true,
      "skippable": true
    },
    {
      "id": "model",
      "name": "Model analysis",
      "type": "autonomous",
      "executor": "operator::media.model_analyze",
      "input": {
        "goal": "${input.output.goal}",
        "media": "${preprocess.output.model_media}",
        "metadata": "${probe.output}",
        "traditional_findings": "${ocr.output}"
      },
      "output_schema": { "$ref": "#/defs/UnderstandingReport" },
      "idempotent": false,
      "skippable": false
    }
  ],
  "edges": [
    { "from": "input", "to": "probe" },
    { "from": "probe", "to": "preprocess" },
    { "from": "preprocess", "to": "ocr" },
    { "from": "ocr", "to": "model" },
    { "from": "model" }
  ],
  "defs": {
    "MediaInvocation": { "type": "object" },
    "MediaProbeOutput": { "type": "object" },
    "ImagePreprocessOutput": { "type": "object" },
    "TraditionalFindingSet": { "type": "object" },
    "UnderstandingReport": { "type": "object" }
  }
}
```

v0 实现可以先内置等价的 Rust plan，不必立刻把 JSON DSL 暴露为用户配置；但内部类型和 stage 命名应按上述 DSL 收敛，保证后续把默认 plan 外置时不重构语义。示例中 `probe`、`preprocess`、`ocr` 都是 `idempotent: true`，必须按 §4.5.5 接入 NDN stage cache；`model` 是 `idempotent: false`，默认不缓存。

#### 4.5.8 用户可扩展性

用户扩展应分两层开放：

1. **配置扩展**：允许用户调整 MIME route、图片缩放限制、是否启用 OCR、OCR executor 选择、视频抽帧策略等。这类配置不改变 stage 代码，只改变默认 DSL 或 stage 参数。
2. **受控 stage 扩展**：允许 `/tool/media.*` 语义路径解析到用户提供的 `appservice::` / `http::` endpoint，后续再支持 `func::`。扩展 stage 必须声明 `input_schema` / `output_schema`、超时、权限、是否幂等；输出只能是 JSON 和 `ResourceRef`，不能把媒体 payload 内联进 history。

不允许在本工具里直接执行任意 shell 脚本作为用户扩展。若需要本地外部命令能力，应先进入 workflow executor registry / AppService / FunctionObject 的受控模型，再由本地 runner 按 `ExecutorRef` 调用。

### 4.6 发送前决策：直传 / 缩小 / 转换

给定一个文件，模型分析之前先回答三个问题。这是 §4.5 管线中 probe 之后、preprocess 之前的规划步骤，由本地 runner 在工具进程内完成，agent 不可见；调用方仍只表达"理解这个文件"的目标，不自行拼接转换命令。

| 问题 | 判断依据 | 不能偷懒的地方 |
|---|---|---|
| 1. 能直接发给目标模型吗？ | 实际选中的模型与 provider 的输入格式、模态和大小限制 | 不能只看扩展名或模型别名；能力信息有就查（`ai_provider` 元数据、AICC 能力标识），没有就按 route 配置假定支持、直传，失败再降级 |
| 2. 需要先缩小以节约 token 吗？ | goal、分辨率、页数、时长、剩余上下文预算 | 模型支持原格式不等于应该整文件直传；缩图、分段、抽帧、按需读取都是候选 |
| 3. 需要先转换格式吗？ | 原格式不支持、成本过高或不便检索 | 转成图片、纯文本，或由 index + 图片 + 文本组成的目录（§4.6.2） |

统一流程：识别文件 → 判断模型能力与预算 → 获取或生成转换产物 → 按 goal 选取内容 → 理解并返回带来源的结果。

#### 4.6.1 决策阶梯（有则用）

按 §1.4，三个问题的每个答案都是尽力的。runner 从上到下取第一个可行的分支，任一步失败退到下一步：

```text
probe: mime / size / 页数 / 时长
  │
  ├─ 预算内、模型原生支持 ──────────────→ 直传（最小内核，总是可用）
  │
  ├─ 原生支持但超预算
  │     图片 → image_fit 缩图（已实现）
  │     视频 → 抽帧（已实现）；长视频按时间段分批
  │     PDF / 文档 → 有共享表示则按页取；没有则按页范围分批直传
  │
  ├─ 原生不支持或不便检索
  │     ① nfs_server preview 可达且有对应 profile → repr 取共享产物（§4.8）
  │     ② 本机有转换工具（poppler / soffice / ffmpeg / import_office）→ 本地转换，
  │        能回写共享缓存则回写，否则只服务本次
  │     ③ 都没有 → 直传试一次
  │
  └─ 直传也被拒 → AgentToolStatus::Error，message 列出：缺的能力、探测过的工具、
     可走的回退（换模型 / 先 import_office / 手动转换）
```

每一层的工具探测沿用 preview `tools` 的约定：有则使用，没有就失败；三平台查找；版本进缓存键。

#### 4.6.2 三类转换产物

| 产物 | 适用情况 | 必须保留的信息 |
|---|---|---|
| 图片 | 页面排版、图表、扫描件、视频帧等视觉内容 | 页码 / 帧时间戳、原始尺寸、缩放关系 |
| 纯文本 | 可提取正文、表格数据、字幕或转写 | 章节、页码、sheet / 单元格范围或时间范围 |
| 目录：index + 图片 + 文本 | 多页文档、复杂表格、长视频，需要跨部分检索和综合 | 完整条目清单、顺序、各产物位置及到原文件的对应关系 |

目录是正式的理解输入，不能只把首页图片或 index 的摘要当作整个文件。逻辑结构如下，存储格式在实现时确定（它在 preview 协议中的表示见 §4.8.2）：

```text
representation/
  index.json       # 源对象、转换版本 / 参数、完整条目与来源映射、缺失 / 失败项
  text/            # 按页、章节、sheet 或时间段组织的可检索文本
  images/          # 页图、图表、关键帧；index 关联对应文本
```

要求：

- 统一的表示描述与 index schema；空白页、无文本页、转换失败页也要有记录，避免默默遗漏。
- 单文件产物和目录表示由同一套解析读取；目录可以由缓存对象引用装配，不强制复制到每个 Agent 私有目录。
- 产物对应 §4.5.4 的 `derived_artifacts`：长期有价值的进共享缓存，仅服务本次的随 work_dir 清理。

#### 4.6.3 输入预算与质量策略

- 预算来自父预算的剩余额度（§6.3），不重置。
- 缩小后无法辨认的区域可按需读高分辨率版本；不能为省 token 静默丢失关键信息。
- 全篇总结分批覆盖各部分再汇总，不要求一次把所有图片和文本塞进上下文。
- 各模型的默认预算值在实现时确定：先按 provider 文档值配置，再按真实数据调。

#### 4.6.4 决策记录

每次调用把决策写进 `details.media_decision`，不进 `output`：

```jsonc
{
  "mode": "direct | shrink | convert",
  "reason": "pdf 42 pages exceeds budget; model accepts image/*",
  "representation": { "work_key": "…", "cache_hit": true, "profile": "document-catalog" },
  "tools": { "probed": ["pdftoppm", "soffice"], "used": ["pdftoppm 24.02"], "missing": ["tesseract"] },
  "fallbacks": ["shared cache unreachable → local poppler"]
}
```

工具缺失导致走回退或失败时，这份记录就是给调用方的具体失败信息与回退路径。

#### 4.6.5 格式支持目标

| 输入 | 转换策略 | 验收重点 |
|---|---|---|
| PDF | 模型支持且预算合适时直传；否则提取分页文本、渲染页图，生成目录；扫描页按需 OCR | 文本 PDF、扫描 PDF、图文混排及长 PDF 均可理解；能找到后部页面的信息并引用页码 |
| DOCX | 提取正文、标题、表格和内嵌图片（复用 `import_office` 的解析）；需要版式时复用 preview `preview-document` 转 PDF，再生成页图 / 文本目录 | 正文、表格与图片信息均可检索；页码只引用实际渲染版本，避免虚构固定页码 |
| Excel | 按工作簿 / sheet 建目录（复用 `import_table` 的解析），文本保留行列与单元格位置；图表和必要区域转图片；大表按范围分块 | 多 sheet、大表、合并单元格与图表；区分公式和已有计算值，缺失计算值需说明，不能把渲染 PDF 当作全部表格数据 |
| 视频 | 关键帧 / 分段抽帧、时间戳和可用字幕 / 音轨转写组成目录；必要时再读取局部片段 | 长视频可按时间检索；视觉与音频证据可关联；明确抽样覆盖及可能遗漏的短暂事件 |

视频关键帧策略先沿用现有的可配置均匀时间采样，再评估场景切换与去重；编码层关键帧不必然等于语义上的关键画面。直传视频也必须检查目标模型的实际能力与预算。`import_office` / `import_table` 是 TS 实现，复用方式（调用 CLI 还是抽成库）在实现时确定。

### 4.7 目录理解与检索

目录表示进入模型分析时，不能把 index 摘要或首页当作全文。检索循环有两种做法：

| 做法 | 说明 | 影响 |
|---|---|---|
| **宿主检索循环（默认）** | 本地 runner 在工具进程内完成：读 index → 在全部文本条目中检索 goal 相关内容 → 选取相关页图 / 图表 / 帧 → 分批送模型 → 汇总。旁路模型仍不开工具，仍是一次或几次无工具的 JSON 输出 | 不触碰 §4.1 fork-and-discard、§6.3 深度 = 1 与 `Do not call tools`；检索质量取决于 runner 的文本检索与选页策略 |
| 受控旁路工具（后续） | 给旁路模型列目录、搜索、读文本、读图四个受控工具，由模型自己检索 | 推翻 `tool_policy.allow_tools = false`，需要新的调用次数与 token 预算约束，输出 schema 也要从单次 JSON 改为多轮；待宿主循环有真实数据后再评估 |

无论哪种做法：

- "搜索整个目录"指检索覆盖全部条目，不是一次塞进全部内容；全篇总结分批覆盖各部分再汇总。
- 内置提示词改为：先读 index，再读检索出的相关文本，再读相关页图 / 图表 / 帧，最后综合回答。`Do not call tools` 在宿主循环下保留，在旁路工具方案下同步修改。
- 每条重要 observation 带定位：页码、sheet / 单元格范围或视频时间戳。沿用 §3 的 observations / reasoning / conclusion / confidence 语义，定位作为 `ObservationItem` 的可选字段 `where`。
- runner 知道自己送了什么，因此 `details.coverage` 由 runner 而非模型填写：已检查的条目范围、未覆盖部分、转换缺失项；`output` 末尾追加一行覆盖说明。局部抽样不能声称完整阅读或观看。

### 4.8 共享表示缓存：复用 nfs_server preview

转换产物进入 preview 的共享表示缓存，由 Preview UI、不同 App、Agent 和 Session 复用；`llm_understand_media` 不在每次调用中重复落一份私有转换缓存。"所有人受益"指有权读取同一内容的调用方共享转换成果。preview 本身也是"有则用"：nfs_server 不可达或 `hello` 不广告 `repr` 时，按 §4.6.1 退到本地转换或直传，并在 `details.media_decision.fallbacks` 记录。

#### 4.8.1 沿用什么

preview 已有的机制直接沿用，不另造：

- `repr`（ensure）/ `get_repr`（query）与任务生命周期：`processing → completed | failed`；同一 `work_key` 的进行中任务合并等待，调用方离开不中止它；失败进 10 分钟负缓存，显式 Retry 走 CAS；服务重启把孤儿任务标为可重试失败。
- 缓存键 `work_key = sha256(input ObjId, converter@version, params)`。页图尺寸、OCR 语言、抽帧策略等影响结果的参数必须进入 `params`；内容或转换器版本变化自然失效。
- 结果存系统桶 `preview-cache`，按源文件 ACL 判权后签短期只读 `bkt_` 令牌；`get_repr` 要求同一 actor 先 `repr` 过（授权记录在进程内保存一小时，nfs_server 重启后需重新 `repr`）。目录及子产物的读取权限随源文件约束，无权限调用不能读缓存。
- 资源限额、超时、30 天未读过期与容量上限按 LRU 回收；缓存被回收后可重建。
- 多阶段转换复用中间结果：docx → PDF（`preview-document`，已有）→ 页图 / 文本。Preview 与理解工具不重复生成相同中间产物。

#### 4.8.2 需要扩展什么

- **新 profile**：在 `Profile` 枚举上扩展文本、页图、目录等转换档（名称实现时定，例如 `document-text` / `document-pages` / `document-catalog` / `video-frames`）。
- **PDF 作为输入**：preview 当前对 `%PDF` 的 sniff 返回 `None`，并有"PDF 为规划 unsupported"的测试；需要把 PDF 加进 `Kind` 识别并更新该测试。
- **多文件产物**：当前 `Output` 只有单个 `path`，`result_json.key` 对应单个桶对象，`Plan::params` 只有 profile 与 hevc。目录表示需要新的 result 形态：一个 manifest 对象（即 `index.json`）加多个子产物 key，或把目录打成单对象。目录的 `completed` 状态必须反映各子产物的完整性，部分失败记录在 index 里而不是整体失败。
- **转换器在两侧**：soffice / ffmpeg 由 nfs_server 探测与执行；poppler-utils / tesseract 按官方清单只在 Linux 容器 `extended` 层。谁负责 PDF → 页图 / 文本是本节最大的决定点：要么 nfs_server 侧也探测 poppler（Lite / host 上多半没有，则该 profile 不广告），要么容器内转完再回写缓存。两条路都符合"有则用"，但回写路径需要 preview 开放写入接口并判权。
- **调用通路与身份**：agent_tool 目前不依赖任何 NFSP 客户端。需要一条 agent_tool → nfs_server 的 `repr` 通路（buckyos-api 的 nfs_server 客户端或经宿主 runtime），并确定以哪个身份调用：`repr` 要用调用者的 session token / principal 过源文件 ACL，不能用系统身份绕过。

#### 4.8.3 内容身份

- `NamedObject` 的 `obj_id` 直接作为 `input ObjId`。
- NFS 路径（`at`）由 nfs_server 解析到 binding 的 `obj_id`。
- 容器内 `local_file` 与 URL 不在 NFS 命名空间，没有 obj_id：共享复用前先确定稳定内容身份（上传为对象，或本地按同一 ObjId 算法算 hash 后以 `args.source.obj_id` 调用，后者要求对象已在 NDN 可读）。不能只以路径或 URL 当作内容身份；确定不了身份时只做本次私有转换，不进共享缓存。

#### 4.8.4 边界

- 区分可复用的**转换产物**与本次 goal / 模型相关的**理解结论**：某次任务的摘要不是原文件的通用转换结果，不进 preview 缓存（与 §4.5.5 `model_analyze` 不缓存一致）。
- 目录产物是否也应让 `read` 工具直接读取（当前 `read` 把非文本对象一律推给 `llm_understand_media`），使"按需读某页文本"不必经过 LLM 旁路，见 §9。

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

图片理解通常不触发 pending。长文档 / 长视频走共享转换时，preview 任务处于 `processing` 可能超出单次调用的等待预算，此时以 `LongRunning` + `check_after` 返回，下次调用按同一 `work_key` 命中；接口已现成，无需额外协议工作。

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
7. **本地管线**：模型分析前先执行本地轻量媒体管线。v0 图片路径至少包含 probe 与 preprocess；probe 输出原始分辨率、size、MIME、可得的 EXIF 摘要；preprocess 在图片过大时生成适合 vision provider 的派生图。
8. **Workflow DSL 复用**：默认管线的 stage / edges / input-output schema 可用 `WorkflowDefinition` 表达；本地 runner 使用 Local Media Pipeline Profile，不走 Workflow Service / Thunk Dispatcher，不新增不被 `ExecutorRef::parse` 支持的 executor namespace。
9. **NDN / Thunk-style 缓存**：所有 `idempotent: true` stage 必须基于 `executor identity + resolved input + stage version` 计算 cache key；命中时直接复用 NDN 中的 JSON output / artifact refs，并在 `pipeline_trace` 标记 `cache_hit`。
10. **传统分析 provenance**：OCR / ASR / barcode 等传统分析结果若启用，必须作为 `traditional_findings` 进入 `details` 与模型输入，并标明来源 stage / artifact / confidence；最终 report 不得把传统分析结果伪装成模型直接观察。
11. **native vision / MIME route**：工具在 probe / materialize 阶段解析 MIME，按配置表得到 AICC 逻辑模型名并写入模型分析 stage 的 `OneShotRequest.model_policy.preferred`；target 媒体以原生 `AiContent::Image` 进入旁路，无辅助转文字中间层。当前 `ModelPolicy` 不声明 `features::VISION` requirement，若 AICC 无法处理该逻辑模型下含 image 的 LLM 请求，应返回明确 Error（本期不强制实现辅助降级）。后续能力 requirement 透传属于 `LLMContext` / AICC adapter 边界改造。
12. **崩溃恢复**：模型分析 stage 的推理边界快照生效（每次推理前提交）；主干在 `AgentToolResult` 返回前不提交该调用的结果，结果随主干下一个 checkpoint 提交，工具结果提交不等于 Turn 完成（见 §5.2）；模型分析 stage 必须把 `pipeline_id` / `pipeline_version` / `compiled_pipeline_hash` 写入自己的 `OneShotRequest.input`，使 `OneShotRequest::semantic_hash()` 覆盖 (goal, 父 history 快照, obj_id, 管线定义/版本)。
13. **深度 & 预算**：旁路 `OneShotRequest.tool_policy.allow_tools = false`；本地管线和模型分析 stage 共享父预算剩余额度，不能各自重置 default 无上限；外部扩展 stage 必须有 timeout 和权限约束。
14. **体积度量**：构造长 session（多次媒体理解）测试，验证主干 history 体积仅随 `tool_result` 文本线性增长，与媒体数量 / 大小解耦。
15. **发送前决策**：每次调用 `details.media_decision` 记录直传 / 缩小 / 转换的选择、原因、探测过与用到的工具；缺少转换工具时 Error message 列出缺的能力和可用回退路径，而不是泛化失败（§4.6）。
16. **最小内核可用**：在没有 nfs_server preview、没有 poppler / soffice / ffmpeg 的环境里，图片、音频、短视频、模型原生支持的 PDF 仍能通过直传得到报告（§1.4）。
17. **目录表示**：PDF / DOCX / Excel / 视频可生成 index + 图片 + 文本的目录；空白页、无文本页、转换失败页在 index 中有记录；单文件产物与目录由同一套解析读取（§4.6.2）。
18. **目录检索与覆盖**：答案能命中 PDF 后部页面、DOCX 内嵌图片、Excel 非首个 sheet 和视频后段内容；observations 带页码 / sheet 范围 / 时间戳；`details.coverage` 准确报告已检查范围、未覆盖部分和转换缺失项（§4.7）。
19. **共享缓存**：同一文件被不同有权限的 Agent / Session 并发理解时，相同转换只运行一次；已有 preview 产物可命中；内容 / 参数 / 转换器版本变化会失效；无权限调用不能读取缓存；docx → PDF → 页图的中间产物只生成一次（§4.8）。
20. **集成验证**：四类格式覆盖不能直传、超预算、扫描件、长文件、部分转换失败和工具缺失六种情形（§4.6.5）。

---

## 9. 待定项 / 后续迭代

| 项 | 说明 | 处理 |
|---|---|---|
| 父 history 目的导向裁剪 | §4.2.4，长 session 下控制旁路重喂 token 量 | 预留可插拔策略点；先靠 `llm_compress` 全量压缩兜底，依据真实成本数据再实现 |
| 报告粒度调优 | §3.5，`observations` 完整度 vs 重 fork 频率的平衡点 | 依据真实重复 fork 频率数据调内置 prompt |
| 文档 / 视频 / 音频的发送前决策 | §2.2 直传已接入；§4.6 决策、§4.6.5 格式目标待实现 | 按 §10 T1–T5 推进 |
| 本地管线 runner | §4.5，复用 Workflow DSL 但不走分布式调度 | 先实现线性 DAG + `operator::media.*` 内置 stage；后续再接 branch / bounded parallel / executor registry |
| 转换产物缓存 | §4.5.5 → §4.8，复用 preview `repr` / `get_repr` | 不再单独实现 `media_stage_cache` named_store 记录；FunctionObject 成熟后把同一 work_key 语义映射到 `func::` |
| 媒体 FunctionObject catalog | §4.5.6，从真实媒体理解场景提炼强语义函数 | 先沉淀 probe / thumbnail / vision image / OCR / keyframe 等函数的 schema；再接 registry 到 `operator::` / `appservice::` / `func::` |
| 用户扩展 stage | §4.5.8，`/tool/media.*` 语义路径扩展 | 第一阶段只允许配置扩展；第二阶段接 AppService / HTTP；FunctionObject 待 workflow executor 模型成熟后再开放 |
| OCR / 传统分析依赖 | §4.5.3，OCR 可显著降低 vision 漏字 | 不作为 v0 硬依赖；引入新库或系统依赖前需确认，并保持可选降级 |
| 嵌套 `llm_*` | §6.3，本期深度 = 1 | 如需开放，先定深度/预算上限并接入 L4 预算账户 |
| 辅助视觉模型降级 | §4.3 末段，纯文本主模型回退 | 本期返回明确 Error；后续实现时复用 `agent_tool` 已有 `llm_*` 工具调用模式 |
| 目录产物与 `read` 的边界 | `llm_read_media.rs` 占位文件已不存在；`read` 目前把非文本对象一律推给本工具 | 实现 §4.6.2 时确定目录里的分页文本是否可被 `read` 直接读取，使"按需读某页"不必经 LLM 旁路 |
| agent_tool → preview 调用通路与身份 | §4.8.2，目前无 NFSP 客户端依赖 | T1 / T2 前置：定客户端与调用身份 |
| PDF 转换执行侧 | §4.8.2，poppler 在容器、soffice 在 nfs_server | T2 前定：nfs_server 侧探测 poppler，或容器转完回写 |
| 目录在 preview 协议中的表示 | §4.8.2，现有 `Output` / `result_json` 为单文件 | T2 定 manifest + 子产物 key 的 result 形态 |
| 本地文件 / URL 的内容身份 | §4.8.3 | T1 定：上传为对象或本地算 ObjId |
| 检索方式 | §4.7，默认宿主循环 | 旁路工具方案待宿主循环有数据后评估 |
| 复用 `import_office` / `import_table` | §4.6.5，TS 实现 | T2 / T4 定调用方式（CLI 或库） |
| 各模型预算默认值 | §4.6.3 | 先按 provider 文档配置，再按真实数据调 |
| 新增第三方依赖 | 任何 OCR / PDF / Office 库 | 按仓库约定单独确认 |

---

## 10. 实施清单（2026-11-05 Review）

状态：待实现。对应 §4.6–§4.8，验收见 §8 第 15–20 项。每一项都在 §1.4 的尽力原则下实现：缺少某层能力时工具退回下一层，最终退回直传。

- [ ] T1：统一发送前决策与表示协议（§4.6），记录直传 / 缩小 / 转换的选择及原因；缺少转换工具时给出具体失败信息和可用回退路径。前置：确定 agent_tool → preview 的调用通路与身份、本地文件的内容身份（§4.8.2–§4.8.3）。
- [ ] T2：扩展 preview 的文本 / 页图 / 目录 profile 与多文件 result 形态，接受 PDF 输入，打通 PDF 和 DOCX 的缓存链路（§4.8.2）；DOCX 文本复用 `import_office`。
- [ ] T3：打通目录检索、图片读取与提示词（§4.7，默认宿主循环）；同步工具描述、`hello` 能力广告、本文档及受影响的 cmd_skills（`ui_core`、`xllm`、`aigc_*`）。
- [ ] T4：支持 Excel 的 sheet / 范围索引与图表表示，复用 `import_table`。
- [ ] T5：将现有视频抽帧接入共享表示，抽帧参数进 `params`，补充时间索引及字幕 / 转写关联。
- [ ] T6：集成验证四类格式，覆盖不能直传、超预算、扫描件、长文件、部分转换失败和工具缺失；验证 §8 第 16、19、20 项。

新增第三方依赖前按仓库约定单独确认。
