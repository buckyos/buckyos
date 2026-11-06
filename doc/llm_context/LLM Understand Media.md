# `llm_understand_media` 技术需求文档

| 项目 | 内容 |
|---|---|
| 文档版本 | v0.6（2026-11-05：按 TODO 第八轮 DV 结果与 agent 侧代码核对回写，§10 列出剩余差距和待确认项；v0.5 把 §4.5 / §4.8 改为 Workflow Service 系统内置 pipeline + FunctionObject，按 [notepads/llm-media-pipeline-workflow-todo.md](notepads/llm-media-pipeline-workflow-todo.md) 回写；v0.4 并入发送前决策与目录表示） |
| 所属系统 | OpenDAN / BuckyOS — `agent_tool` 受控 `llm_*` 工具族 |
| 组件类别 | `llm_*` 受控旁路工具（封装层提供，非开放 fork 原语） |
| 实现位置 | agent 旁路：[src/frame/agent_tool/src/llm_understand_media.rs](src/frame/agent_tool/src/llm_understand_media.rs)、[llm_understand_media/pipeline.rs](src/frame/agent_tool/src/llm_understand_media/pipeline.rs)；系统内置管线：[src/kernel/workflow/src/functions/media/](src/kernel/workflow/src/functions/media/) 与 [src/kernel/workflow/defs/](src/kernel/workflow/defs/) |
| 依赖前置 | `LLMContext` waist、xllm run（`XllmTask` / `XllmRun`）、`AiContent`、`ResourceRef::NamedObject`、`ndn_lib::ObjId`；workflow service（内核服务，与 scheduler 同级，不可达视为系统故障）及其 `media.*` 系统定义；`buckyos_api::MediaManifest` |
| 需求来源 | 本文档 v0.3 的设计，加上开发者对 Agent runtime baseline 应携带哪些工具的思考（2026-11-05 Review）；[paios 容器需求 §6.4](doc/app_service/paios容器需求.md) 是当前 baseline 清单，作为可用性约束引用 |
| 状态 | Workflow 侧（§4.5）：代码已提交，真实 Zone 上 DV WF-DV-009~013 通过；T2d（视频输入走 `/ndn/`）改动未提交、未部署。Agent 侧（§4.6–§4.8）：主流程已提交（9214ce073），单测通过，**没有在真实 Agent Session 上运行过**；差距与待确认项见 §10。 |

---

## 0. 与 buckyos 现有基础设施的对应关系（先读这一节）

设计文档里出现的抽象名词，全部映射到现有代码：

| 文档中的概念 | buckyos 实际类型 / 模块 | 位置 |
|---|---|---|
| 集群 content-addressed 对象存储 | `ndn_lib`（NDN 命名对象库，beta2.2 外部依赖） | crate `ndn_lib`，在 `agent_tool` 中以 `use ndn_lib::ObjId` 引入 |
| 内容哈希引用 | `ndn_lib::ObjId`（命名对象 id） | 见 `Cargo.toml` 中 `ndn_lib = { git = "...cyfs-ndn..." }` |
| 对外的资源引用包装 | `buckyos_api::ResourceRef`（`Url` / `Base64` / `NamedObject { obj_id }`） | [aicc_client.rs:1427](src/kernel/buckyos-api/src/aicc_client.rs#L1427) |
| 多模态 content block | `AiContent::Image { source: ResourceRef }` / `AiContent::Document { source, title }` | [aicc_client.rs:2380](src/kernel/buckyos-api/src/aicc_client.rs#L2380) |
| 消息 IR | `AiMessage { role: AiRole, content: Vec<AiContent> }` | [aicc_client.rs:2530](src/kernel/buckyos-api/src/aicc_client.rs#L2530) |
| LLMContext waist | `LLMContext` / `LLMContextRequest` | [src/frame/llm_context/src/context_loop.rs](src/frame/llm_context/src/context_loop.rs)、[request.rs](src/frame/llm_context/src/request.rs) |
| Agent 调用入口 | Session helper 命令 `llm_understand_media`（运行时生成，转到 `agent_tool llm_understand_media`），Agent 经 `shell` 调用；`LlmUnderstandMediaTool` 实现了 `AgentTool`，但只从 crate 导出，没有 runtime 注册 | [bin_overlay.rs:179](src/frame/lib_opendan/src/runtime/bin_overlay.rs#L179)、[opendan main.rs:538](src/frame/opendan/src/main.rs#L538) |
| 旁路模型推理与持久化 | xllm run（`XllmTask::prepare` + `XllmRun::start` / `execute`，run 目录为 `run.json` + `snapshots/`）；每次推理（Round）前提交 `LLMContextSnapshot`，outcome 边界再提交一次 | [src/frame/agent_tool/src/xllm.rs](src/frame/agent_tool/src/xllm.rs) |
| 旁路用到的压缩 | `llm_compress::compress` / `LlmSummarizeCompressor` | [src/frame/agent_tool/src/llm_compress.rs:219](src/frame/agent_tool/src/llm_compress.rs#L219) |
| 工具结果信封 | `AgentToolResult`（`status` + `summary` + `details` + `output`）；LLM 可见文本由 `observation_text()` 决定 | [src/frame/agent_tool/src/lib.rs:243](src/frame/agent_tool/src/lib.rs#L243)、[lib.rs:434](src/frame/agent_tool/src/lib.rs#L434) |
| Pending 状态 | `AgentToolStatus::Pending` + `AgentToolPendingReason` | [lib.rs:209](src/frame/agent_tool/src/lib.rs#L209)、[lib.rs:220](src/frame/agent_tool/src/lib.rs#L220) |
| 视觉能力标识 | `features::VISION = "vision"`、`ModelRequirement.vision`（AICC 能力标识；当前 LLMContext 桥接层尚未透传 vision requirement） | [aicc_client.rs:1364](src/kernel/buckyos-api/src/aicc_client.rs#L1364)、[aicc_client.rs:1650](src/kernel/buckyos-api/src/aicc_client.rs#L1650) |
| 模型策略 | `ModelPolicy` 当前只承载 preferred / fallbacks / temperature / max_completion_tokens / provider_options；`ModelRequirement` 由 `AiccLlmClient` 根据工具调用与 JSON 输出生成 | [llm_context/src/request.rs](src/frame/llm_context/src/request.rs)、[xllm.rs:5393](src/frame/agent_tool/src/xllm.rs#L5393) |
| Workflow DSL | `buckyos_api::workflow_dsl::WorkflowDefinition` / `StepDefinition` / `ControlNodeDefinition` | [workflow_dsl.rs](src/kernel/buckyos-api/src/workflow_dsl.rs)、[src/kernel/workflow/src/dsl.rs](src/kernel/workflow/src/dsl.rs) |
| Workflow executor 表达 | `ExecutorRef` 支持 `service::` / `http::` / `appservice::` / `operator::` / `func::` 与 `/agent/` / `/skill/` / `/tool/` 语义路径 | [workflow_types.rs](src/kernel/buckyos-api/src/workflow_types.rs)、[doc/workflow/executor list.md](doc/workflow/executor%20list.md) |
| 共享表示缓存（转换产物） | workflow service 的 Step cache：`step_cache_key = hash(thunk_obj_id, 工具指纹)`，索引在 service-local SQLite（`data/srv/workflow/workflow.db`），结果 / manifest 为 Named Object，产物在 workflow zone 桶 `artifacts`（temp，30 天）；活动 Run 按 principal 合并（ensure） | [src/kernel/workflow/src/cache_index.rs](src/kernel/workflow/src/cache_index.rs)、[persistence.rs](src/kernel/workflow/src/persistence.rs)、[orchestrator.rs](src/kernel/workflow/src/orchestrator.rs) |
| 转换函数 | 九个 `media.*` Native FunctionObject（capabilities、probe、fit_image、extract_pdf_text、render_pdf_pages、extract_frames、extract_audio、to_pdf、build_catalog），由 `/tool/media.<fn>@1` 引用 | [src/kernel/workflow/src/functions/media/mod.rs](src/kernel/workflow/src/functions/media/mod.rs) |
| 转换工具探测 | workflow 宿主启动时探测 poppler / ffmpeg / soffice（三平台查找，版本进指纹）；缺工具时对应步骤 Failed(`tools_missing`)，是部署缺陷而非正常降级（基线见 [doc/workflow/media tool baseline.md](doc/workflow/media%20tool%20baseline.md)） | [functions/media/tools.rs](src/kernel/workflow/src/functions/media/tools.rs) |
| Office / 表格的 Agent 命令 | `import_office`（docx / pptx → Blocks）、`import_table`（csv / xlsx / sqlite → TableSources）。它们是 Agent 手册命令，媒体管线不使用（TODO D9） | [import_office.ts](src/tools/buckyos-agent/commands/import_office.ts)、[import_table.ts](src/tools/buckyos-agent/commands/import_table.ts) |
| 媒体工具基线（可用性约束） | workflow 宿主和 paios 镜像都要满足同一份清单（poppler、ffmpeg、LibreOffice、字体、locale）；官方命令清单仍以 paios 文档为入口 | [doc/workflow/media tool baseline.md](doc/workflow/media%20tool%20baseline.md)、[paios 容器需求 §6.4](doc/app_service/paios容器需求.md) |

> 当前实现（2026-11-05 核对）：Agent 经 `shell` 运行 helper 命令；工具进程解析 locator，按 §4.6 选路，确需转换时创建 `media.<family>` Run 并等待结果，再以 xllm run 做模型分析。音频走 AICC `audio.asr` 转写后进模型；视频与 Office 文档一律经 workflow 转换；PDF 在协议接受 Document 且不超过 20 MiB 时直传。模型选路由 `LLM_UNDERSTAND_MEDIA_{IMAGE,VIDEO,AUDIO}_MODEL` / `LLM_UNDERSTAND_MEDIA_MODEL` 环境变量与内置默认别名决定（默认 `llm.vision`，音频 `llm.chat`），§4.4 的 TOML 路由表尚未实现。内置提示词要求 `Do not call tools`。

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

- 是一个**嵌套的模型上下文**——内部以 xllm run 执行，`goal` 进入它的 user message；
- 可接收父 history 的"提纯快照"（见 §4.2；当前入口不提供，见 §10 Q2）；
- 在进入模型分析前，确需转换的媒体由 **Workflow Service 的系统内置 pipeline**（`media.<family>` 定义，FunctionObject + LocalThunkRunner）完成机械识别与机械转换；工具只消费其 manifest / 产物再做模型分析（见 §4.5）；
- 采用 **fork-and-discard** 语义：旁路在返回瞬间整体蒸发，主干只看到一次 `shell` 工具调用及其文本结果；
- 不向 agent 暴露底层 fork 机制，agent 只表达"理解某个媒体"这一意图（intent over function call）。

### 1.3 与既有架构的一致性

- **context purification / side-channel execution**：旁路是一次性认知动作，主干只接收结论不接收过程；
- **L4-only 压缩纪律**（[llm_compress.rs:1](src/frame/agent_tool/src/llm_compress.rs#L1) 的模块 doc）：媒体的 materialize（NDN 对象 → 真正像素）与 compaction（像素 → 结构化报告）都是显式的、发生在明确边界（一次工具调用、一次压缩改写）上的事件，不隐式继承；
- **崩溃恢复互不渗透**：旁路以 xllm run 执行，按推理边界提交快照；主干恢复时不接手旁路（见 §5.2），转换结果靠 workflow 的 Step cache 复用。

### 1.4 基础工具的选路原则

`llm_understand_media` 是 Agent 的基础工具：所有 Session、所有形态（容器、Desktop Lite、node-daemon host）都有它。转换能力由 workflow service 在宿主执行；workflow 是与 scheduler 同级的内核服务，不可达即系统故障，不设"workflow 不可达仍需完成媒体理解"的要求（裁决 R2）。"有则用"只保留给模型能力与预算选路：

- **最小内核极小**：识别 MIME → 选模型 → 把文件（或转写文本）直接发给模型 → 产出报告。这条路只依赖 AICC 和模型本身。
- **确需转换只有一条路**：经 workflow。agent 没有本地缩图、抽帧、转 PDF 的备用通道，也不因 workflow 或工具的故障退回它们。
- **缺基线工具是部署缺陷**：对应步骤以 `tools_missing` 失败并列出缺项，不是正常降级（裁决 R3，清单见 [media tool baseline](doc/workflow/media%20tool%20baseline.md)）。错误要具体：失败的步骤、错误码、缺的工具。
- **尽力不等于静默**：每次调用在 `details` 里记录走了哪条路、为什么（§4.6.4）；局部抽样不能声称完整阅读（§4.7）。
- **先探测再承诺**：工具描述与 cmd_skills 只描述当前形态真正具备的能力，不把"有工具时能做"写成"总能做"。

这条原则来自裁决 P8：先由 workflow 的 FunctionRegistration requirements 定义媒体工具基线，再同步宿主预置与 paios 镜像。工具不反过来依赖 Agent 容器里有什么转换器。

---

## 2. 接口定义

### 2.1 Agent 可见面

实际入口是 Session helper 命令，Agent 经 `shell` 调用（手册见 [ui_core.md](src/apps/jarvis_agent/agent/cmd_skills/ui_core.md)）：

```text
llm_understand_media <file | object id | url> [--goal <text>] [options]
agent_tool llm_understand_media --media <json | locator> [--goal <text>] [options]
```

| 参数 | 说明 |
|---|---|
| `<locator>` | 本地文件（相对当前目录或绝对路径）、对象 id（`cyfile:…`、chunk id，例如消息附件）或 http(s) URL |
| `--media <json>` | 同上的 JSON 形式：`{"kind":"named_object","obj_id":"cyfile:…","mime_hint":"image/png"}`、`{"kind":"local_file","path":"/abs/frame.png"}`、`{"url":"https://…"}` |
| `--goal <text>` | 要了解什么；缺省时要求详细描述内容，让看不到原文件的人也能使用 |
| `--max-completion-tokens <n>` | 输出预算，默认 2048，按 2048 / 4096 / 8192 分档向上取整 |
| `--model <alias>` | AICC 逻辑模型名，覆盖 §4.4 的路由 |
| `--work-dir <path>` | xllm run 目录 |
| `--history-file <path>` | 父 history 快照（JSON `Vec<AiMessage>`），见 §4.2 |
| `--resume <run_id>` | 接回之前 pending 结果的转换 Run，见 §5.3（Q1 推荐删除） |

- agent 只表达目标媒体和 goal。底层的 fork、预算、内置提示词、转换管线，**全部对 agent 不可见**。
- `LlmUnderstandMediaTool`（`AgentTool` 实现）的 `spec()` 要求 `media`、`goal`、`max_completion_tokens`，可选 `resume`；它目前没有被任何 runtime 注册，只从 crate 导出。
- 工具名以 `llm_` 前缀标识其"本质是一个 LLM 旁路"的语义，与同目录下 `llm_compress` / `llm_explore` / `llm_bash` / `llm_tool_carft` 一致。

### 2.2 `media` 实参的来源约束

`media` 字段必须可解析为 `ResourceRef` 或本地文件，且**强烈建议**为 `ResourceRef::NamedObject`：

- **首选**：`NamedObject { obj_id: ObjId }`——content-addressed、跨 zone/peer 可寻址、可由 `ndn_lib` GC 模型统一管理。工具只读 FileObject 和内容头部（4 KiB）做 probe，不整文件读入。
- **可接受**：`Url { url, mime_hint }`——当媒体来源是远端 HTTP(S) 且 buckyos 这一侧不需要长期持有该资源时；但调用方需自承"URL 过期 / 跨设备失效"风险，本工具不为其增引用计数。也接受 `data:` URL。
- **可接受（本机产物）**：`{kind: "local_file", path}`——上一步 shell 产出的文件。它没有 NDN 身份；需要转换时先冻结为对象再调用管线（§4.8.3），直传不应强制上传（当前实现无条件冻结，见 §10 G2）。
- **禁止作为长期 history 引用**：`Base64 { mime, data_base64 }`——一旦写回主干 `tool_result`，base64 永久驻留 history，违背本设计的核心收益。`llm_understand_media` 在写回时**必须**剔除 inline base64。详见 §6.1。
- **MIME 来源**：模型路由必须基于媒体 MIME / media kind，而不是仅基于 `obj_id`。
  - `NamedObject`：头部 magic sniff（识别为归档但文件名有更具体的类型时不采用）→ 文件名扩展名 → FileObject meta 的 `mime_type`（`application/octet-stream` 不算）→ 调用方 `mime_hint`。
  - `local_file`：`mime_hint` → 扩展名 → sniff。
  - `Url`：`ResourceRef.mime_hint` → `media.mime_hint` → sniff → URL 文件名的扩展名。当前不读 HTTP `Content-Type`。
  - `Base64`：自带 `mime`。

> 实现范围（2026-11-05）：图片、音频、视频、PDF、Office 文档与文本均已接入；归档必须先解包；XLSX 未接入转换（§4.6.5）。

### 2.3 `ToolResult`（写回主干的内容）

旁路对外的返回严格遵循 `AgentToolResult`（[lib.rs:243](src/frame/agent_tool/src/lib.rs#L243)）：

```rust
AgentToolResult {
    agent_tool_protocol: "1",
    tool: Some("llm_understand_media"),
    status: AgentToolStatus::Success | Error | Pending,
    summary: String,          // 一行人类可读总结（成为 fallback to LLM）
    details: Json,            // 序列化后的 UnderstandingReport（见 §3）及 §4.6.4 的决策记录
    output: Some(String),     // 喂回 LLM 用的纯文本渲染（见下方约束）
    pending_reason: Option<AgentToolPendingReason>,  // §5.3
    ..Default::default()
}
```

写回主干 history 时由 caller 把 `AgentToolResult` 装配进 `AiContent::ToolResult`：

> **硬约束**（沿用 `agent_tool` 现有纪律）：写回 LLM 的 `AiContent::ToolResult.content` **只塞 `AiToolResultContent::Text { text: output }` 一个块**——`output` 是 report 的紧凑文本渲染（或 `summary` 作为 fallback）。**不要**把整份 `AgentToolResult.to_value()` 序列化进 tool_result——否则随着 session 推进历史压不下来。`details` 是结构化报告原文，仅留给 worklog / replay，不进 LLM。

并且**绝对不允许**：`tool_result` 中包含任何 `AiToolResultContent::Image`、`base64` 字符串、或任何形式的 media payload。这是装配层硬约束，由 `llm_understand_media` 自身在生成 `AgentToolResult` 时强制满足。

shell 路径下的实际行为：helper 把整个 `AgentToolResult` 以 JSON 打到 stdout（`details` 的线上字段名是 `detail`）。命令不含 shell 运算符时，`shell` 解析并转发为工具结果（[llm_bash.rs:1633](src/frame/agent_tool/src/llm_bash.rs#L1633)），LLM 只看到 `observation_text()`，`detail` 留给宿主。命令带 `&&`、管道等运算符时不转发，整段 JSON 进入 LLM；其中没有媒体字节，但多了结构化细节，手册应让 Agent 直接运行该命令。

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

`UnderstandingReport` 自身的紧凑文本渲染（按 `observations` → `reasoning` → `conclusion` → `confidence` 顺序拼接）作为 `AgentToolResult.output` 字段，是真正进 LLM 的内容。§4.7 要求的定位字段 `where` 尚未加入（§10 G7）。

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
   │  agent 经 shell 运行：llm_understand_media <locator> --goal "..."
   ▼
┌──────────────────────────────────────────────────────────────────┐
│  旁路进程（helper 命令）                                           │
│    resolve_media：解析 locator，取 MIME / size / 内容头部           │
│    prepare_media_content：选路（§4.6）                             │
│      direct  → 直接构造 AiContent                                 │
│      convert → workflow media.<family> Run（确需转换时）           │
│                capabilities -> probe -> convert -> build_catalog  │
│                读取 manifest / 产物，选页，构造 AiContent           │
│    xllm run（XllmTask::prepare + XllmRun::start / execute）        │
│      input = [ System: 内置提示词,                                 │
│                ...父 history 的提纯快照（§4.2，如有）...,           │
│                User: [媒体内容..., "Goal: <goal>"] ]               │
│      model = --model 或 route_model(mime)                         │
│      tools 关闭、web_search 关闭、输出 JSON 报告                    │
│    → 产出 UnderstandingReport，写成 AgentToolResult                │
└──────────────────────────────────────────────────────────────────┘
   │
   │  旁路返回（工作目录的去留见下）
   ▼
主干 LLMContext
   仅新增: shell 的 AiContent::ToolUse + AiContent::ToolResult { content: [Text { output }] }
```

- 设计要求：旁路里的媒体像素、派生 artifact 和模型推理中间状态在返回瞬间随工作目录蒸发，主干 history 永远只留一对 `ToolUse` / `ToolResult`。当前实现只满足"主干不留"：默认工作目录在系统临时目录，结束后没有清理，且快照含 Base64 媒体（§10 G3）。
- 旁路对主干**无副作用**——除了最终写回的 `tool_result`。
- 旁路中的模型分析 stage 受 xllm run 的推理边界快照保护（每次推理前提交）；转换阶段在 workflow 的 Run 里，由 Step cache 复用（见 §5）。
- 旁路中的推理（包括 §4.2 压缩父 history 的摘要调用和模型分析 stage）是这次工具调用内部的嵌套模型调用，归属关系见 §5.2。

### 4.2 父 history 继承策略（提纯投影，非全量拷贝）

> 当前实现：提纯与压缩代码已有，也有单测，但只有 `--history-file` 能传入父 history，现有 behavior 与手册都不传，`AgentTool` 入口也未注册。所以在真实 Session 里旁路没有父 history，`goal` 必须自带所需上下文。是否接通见 §10 Q2。

旁路子 context 继承父 history 时，遵循统一原则：

> **`llm_*` 工具的子 context，继承的是父 history 的"语义"，不继承父 history 的"原始 modality 与冗余"；它向父 history 写回的，只有结论文本，不含任何 modality payload。**

具体规则（实现侧）：

1. **modality 降级**：遍历父 history 中每条 `AiMessage`，对其 `content` 中所有 `AiContent::Image` / `AiContent::Document` 进行如下替换：
   - **唯一例外**：本次 `goal` 显式 target 的那条 `ResourceRef`，在旁路构造的"最后一条 User message"中以真实媒体内容出现。
   - **其余所有媒体块**降级为 `AiContent::Text { text: "[media omitted: obj_id=..., mime=...]" }` 占位（**不**把 `ResourceRef` 透出给子 context 的视觉模型）。
   - 即："history 可见"与"history 里的 media 可见"是两件事，默认只给前者。

2. **快照而非引用式共享**：子 context 的 `input` 是一份确定的 `Vec<AiMessage>`，而非对父 `LLMContext` 的活引用。理由：
   - 与 xllm run 按推理边界提交的快照一致：旁路 input 是一份确定的数据，可重放；
   - 父 context 可能在旁路执行期间继续推进（并行 agent loop），快照语义明确、可重放。

3. **超长父 history → 走 `llm_compress`**：本工具构造旁路 input 前调用 `llm_compress::compress`（[llm_compress.rs:219](src/frame/agent_tool/src/llm_compress.rs#L219)）把降级后的快照压到目标预算内（默认 24000 token）。压缩边界与 head-keep / hot-tail 规则全部沿用 `llm_compress` 已有约定。

4. **（可选优化，非本期必须）目的导向裁剪**：长 session 下可能希望根据 `goal` 进一步裁剪父 history 切片。本期实现先做"全量降级快照 → llm_compress"，若旁路成本仍显著再引入语义裁剪。**实现者应将此预留为可插拔策略点。**

### 4.3 native vision vs 辅助降级

旁路子 context 先根据 target `media` 的 MIME / media kind 选择内容形态与 AICC 逻辑模型名（§4.6）。图片以 `AiContent::Image` 放入最后一条 user message，文档直传时用 `AiContent::Document`，转换结果是若干文本块加页图 / 帧，音频是转写文本；由 AICC provider 适配层负责 lowering。

- `LLMContextRequest.model_policy` 对应的 `ModelPolicy` 不包含能力要求字段；`AiccLlmClient` 只根据工具调用与 JSON 输出设置 `ModelRequirement.tool_call` / `json_schema`，`llm_understand_media` 尚不能声明 `ModelRequirement.vision`。当前阶段通过 MIME → AICC 逻辑模型名选路，工具本身不硬编码 provider 模型名。后续若需要强制 vision-capable provider，应在 `LLMContext` → `AiccLlmClient` 边界增加能力 requirement 透传。
- AICC 已能接收 `ResourceRef::NamedObject`：用调用者 session 向 NDM 校验读权限（[inference.rs:823](src/frame/aicc/src/service/inference.rs#L823)），默认由 AICC 读出字节后内联给 provider；只有配置了 `public_named_object_base_url` 且协议接受 URL 时，才把对象 URL 交给 provider（[inference.rs:16](src/frame/aicc/src/service/inference.rs#L16)）。当前工具没有用这条路，一律自己读字节转 Base64（§10 Q3）。
- **不得**在主模型已原生支持 vision 时，仍强制把图片经辅助视觉模型转文字再喂入——这会引入不必要的延迟、成本与信息损失。
- 仅当 buckyos 当前可路由的模型都不支持 `features::VISION`，或对象大小/格式不被该 provider 接受时，才回退到"辅助视觉模型预转文字"路径。该回退路径本期不做，以 `AgentToolStatus::Error` + 明确 message 报错。
- 路由决策由 AICC route policy 驱动（[aicc_client.rs:2247](src/kernel/buckyos-api/src/aicc_client.rs#L2247)），不在 `llm_understand_media` 内部硬编码模型名。

### 4.4 MIME → AICC 逻辑模型配置

`llm_understand_media` 不直接硬编码 provider / exact model，而是把 MIME 映射到 AICC 逻辑模型名。下面的 TOML 是目标形态，匹配顺序从上到下，首个命中项生效；未命中则返回明确 Error。

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

> 当前实现（[llm_understand_media.rs:1516](src/frame/agent_tool/src/llm_understand_media.rs#L1516)）：`--model` 优先；其次 `LLM_UNDERSTAND_MEDIA_{VIDEO,IMAGE,AUDIO}_MODEL`；再次 `LLM_UNDERSTAND_MEDIA_MODEL`；最后内置默认（音频 `llm.chat`，其余 `llm.vision`）。归档没有路由。TOML 路由表待实现。

### 4.5 媒体表示管线：Workflow Service 的系统内置 pipeline

`llm_understand_media` 的长期定位不是"图片转文字"的单点工具，而是 OpenDAN / BuckyOS 理解非结构化数据的旁路。通用执行顺序为：

```text
机械识别(probe) -> 机械转换(convert) -> 模型分析(model analysis)
```

2026-11-05 第二次裁决（[notepads/llm-media-pipeline-workflow-todo.md](notepads/llm-media-pipeline-workflow-todo.md) P2 / P5）：机械识别与机械转换**由 Workflow Service 的系统内置 pipeline 承载**，不再在 `agent_tool` 进程内实现一个本地 runner，也不复用 nfs_server preview 组件（只借鉴其 ensure / work key / descriptor / 状态机的处理模型）。模型分析仍在 `llm_understand_media` 旁路内进行，不进管线。

- **管线 = `WorkflowDefinition`**：四条系统定义 `media.image` / `media.pdf` / `media.video` / `media.document` 随 workflow service 发布（`src/kernel/workflow/defs/media.*.json`，启动时播种，`source: system`，只读）。
- **一次转换 = 一个 Run**：`llm_understand_media` 调 `workflow.create_run` 创建 Run；输入是内容寻址对象，产物是 NDN 对象。
- **stage = `autonomous` Step**，executor 是 `/tool/media.<fn>@1` 语义链接，编译期经 FunctionRegistry 绑定到真实 FunctionObject（`func:<hex>`）。这是 FunctionObject / Thunk 通道的第一次真实落地：九个 `media.*` Native 函数在 workflow 进程内的 `LocalThunkRunner` 执行。
- **跨 Run 复用 = Orchestrator 的 Step cache**：`step_cache_key = hash(thunk_obj_id, 执行指纹)`，持久化在 workflow 的 SQLite 索引里，命中前校验结果对象与全部产物仍可读。
- **可观测**：Run / Step / Thunk 进 task_manager 任务树，请求者获得 Run 根任务的 WholeTree 读 grant，管理员可在 TaskMgr 观察（DV WF-DV-012 已验证）；Desktop `app/workflow` 已改接真实 RPC，未在 Zone 上验证（§10）。

已验证（第八轮，dev Zone，无 OpenDAN 容器）：WF-DV-009~013 通过；`cargo test -p workflow -- --ignored` 的 poppler / ffmpeg / soffice 用例通过。验收明细见 TODO §5。

#### 4.5.1 与 Workflow Service 的分工

| 维度 | Workflow Service（系统内置 pipeline） | `llm_understand_media`（agent_tool 旁路） |
|---|---|---|
| 做什么 | probe、缩图、PDF 文本 / 页图、视频抽帧 / 音轨、Office → PDF、目录 manifest | 选路（直传 / 转换）、读取 manifest 与产物、检索与视觉覆盖、ASR、模型分析、写回 `AgentToolResult` |
| 身份 | 已验证的调用 principal（`user_id/app_id`）是 Run 的 `requested_by`；workflow 是实际执行主体，Task creator 如实记 workflow | 以 Agent 自己的 session token 调 RPC，不能自报 owner |
| 输入 | `{obj_id, 参数}`；服务端把 `obj_id` 解析为 FileObject.content（chunk / chunklist id），调用者不能直接指定 `content_id` | 保留 `ResourceRef::NamedObject` 引用；`local_file` / URL 先冻结成对象再调用 |
| 产物 | workflow zone 桶 `artifacts`（temp，30 天）；manifest / 结构化结果为不可变 Named Object。写入和读取都续 30 天 lease，产物递归 pin（第八轮 DV 发现未 pin 的对象会被 nfs-server 维护 GC 回收） | 只持有 obj_id；不内联媒体字节进主干 history |
| 失败 | 结构化错误 `{code, message, details, retryable}` 贯穿 Native → 回执 → Run / Step / Thunk Task → RPC；本批定义 `max_attempts=1`、`fallback=abort`，不会进入 `WaitingHuman` | workflow 不可达按内核服务故障报错，不建本地备用转换路径（裁决 R2） |

#### 4.5.2 四条系统定义

每条定义的 step 全部 `autonomous`，显式 `idempotent`，`guards.retry = {max_attempts: 1, fallback: abort}`；`trigger.input_schema` 声明调用输入及默认值（`additionalProperties: false`，不含 `content_id`），`trigger.output_schema` 是服务端补齐后的 `${trigger.output.*}` 形态（多出 `content_id`、`size`、`name`）。

| 定义 | 有效输入（默认值） | steps | Run output（终端节点） | Run 期限 |
|---|---|---|---|---|
| `media.image` | `obj_id`、`max_long_edge` 2048、`max_pixels` 4194304、`max_bytes` 3670016 | capabilities → probe → fit_image | `fit_image`: `{model_media_obj_id, mime, original, sent, actions}` | 10 分钟 |
| `media.pdf` | `obj_id`、`dpi` 144、`max_long_edge` 2048、`page_range` `[]`（全部）、`limits.max_pages` 50（上限 200） | capabilities → probe → extract_pdf_text → render_pdf_pages → build_catalog | `build_catalog`: `{manifest_obj_id, summary, complete}` | 30 分钟 |
| `media.video` | `obj_id`、`strategy` uniform、`max_frames` 16（上限 64）、`frame_max_edge` 1280、`limits.max_duration_ms` 3600000（上限 4 小时） | capabilities → probe → extract_frames → extract_audio → build_catalog | 同上；无音轨为正常 `absent`，ASR 留在 agent_tool | 30 分钟 |
| `media.document` | `media.pdf` 的输入 + `format`（docx / pptx / odt / odp / doc / ppt / rtf / txt，必填） | capabilities → probe → to_pdf → PDF 三步（以 `${to_pdf.output.pdf_content_id}` 为源） | 同上；manifest 同时保留原源与派生 PDF 内容 ID | 45 分钟 |

`capabilities` 是 `idempotent: true` 但 `is_pure: false` 的函数：每次 Run 都真实执行，输出本机工具环境事实，永不进缓存。其余函数只在 `step.idempotent && function.is_pure` 时读写缓存。

`to_pdf` 按 `is_pure: false` 注册（[document.rs:11](src/kernel/workflow/src/functions/media/document.rs#L11)）。实测 LibreOffice 25.2 对同一输入转两次，文件大小相同，但有 91 字节不同，差异在 `CreationDate`、`/ID`、`DocChecksum`。因此每次 Run 都重跑 soffice，并得到新的 `pdf_content_id`，后续文本、页图、manifest 全部不命中。处理方式见 §10 Q5。

#### 4.5.3 `media.*` FunctionObject（首批 9 个）

注册在 `src/kernel/workflow/src/functions/media/mod.rs`，`content = "<name>/impl-1"`，`func_type = native`，参数以 `params_schema` 为权威、`params_type` 只标出 `content_id` 这类 `CheckByRunner("file_content")` 引用。Native 行为变化必须升级 `impl-<n>`，老 Run 不会悄悄绑定新函数。

| 函数 | 关键参数 | 结果 | 工具（requirements） |
|---|---|---|---|
| `media.capabilities` | 无 | `{revision, tools, missing}` | 无；读取环境，非 pure |
| `media.probe` | `content_id`、`family` | `{kind, mime, size, width?, height?, pages?, duration_ms?, has_audio?, has_video?, format?}` | 按 family 用 pdfinfo / ffprobe |
| `media.fit_image` | `content_id`、`max_long_edge`、`max_pixels`、`max_bytes` | `{model_media_obj_id, mime, original, sent, actions}` | image crate；非常见格式可选 ffmpeg |
| `media.extract_pdf_text` | `content_id`、`page_range`、`max_pages` | `{pages: [{page, status, text_obj_id?, chars?, error?}], total_pages}` | pdfinfo、pdftotext |
| `media.render_pdf_pages` | `content_id`、`dpi`、`max_long_edge`、`page_range`、`max_pages` | `{pages: [{page, status, image_obj_id?, width, height, error?}], total_pages}` | pdfinfo、pdftoppm |
| `media.extract_frames` | `content_id`、`strategy`、`max_frames`、`frame_max_edge`、`max_duration_ms` | `{frames: [{index, time_ms, image_obj_id, width, height, status}], duration_ms}` | ffprobe、ffmpeg |
| `media.extract_audio` | `content_id`、`max_duration_ms` | `{status: ok | absent, audio_obj_id?, codec?, duration_ms?}`；输出固定为 mp3，`codec` 是源音轨编码 | ffprobe、ffmpeg |
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

- `LocalThunkRunner`：`schedule_thunk` 只做有界入队（`max_queue` 64）即返回；执行任务按 lane 取槽，默认 Light 8、Image 2、Pdf 2、Video 1、Office 1。
- 资源上限（`ExecutionLimits` 默认值，超限返回 `resource_limit`）：输入 512 MiB、200 页、64 帧、单页 4096² 像素、累计 200 × 2048² 像素、累计输出 2 GiB、临时盘 4 GiB、排队 300 秒、执行 900 秒。
- 执行前校验：环境指纹与投递时一致（否则 `environment_changed`）、`requirements` 的工具存在（否则 `tools_missing`，details 列出缺项与搜索路径）、params 通过 `params_schema`、`CheckByRunner` 内容可读；结果通过 `result_schema` 才回灌与写缓存。
- 输入读取：poppler / soffice 把内容落到执行临时目录。ffprobe / ffmpeg 读标准 NDN URL `/ndn/<file_id>?hint={"token":"bkt_..."}`（HTTP + Range，不落临时文件）：FileObject 绑定在 `zone/workflow/zone/scratch` 的 `inputs/<execution_id>`，令牌只读这一个 key，有效期到 deadline 后 60 秒，执行结束时撤销并解绑，崩溃靠过期兜底；限定 `-format_whitelist`（不含 hls / dash / concat）与 `-protocol_whitelist`；令牌不进日志、错误 details、Task 数据和 manifest。这部分（TODO T2d / [Bucket 设计 §21](doc/arch/应用对象桶%20Bucket%20设计.md) N17）代码和真实 ffmpeg 用例已写，尚未提交、未部署，经 `/ndn/` 的 DV 未跑。
- deadline 取 Run `guards.max_duration`、Step `guards.timeout`、函数 `timeout` 中最早者；超时或取消时 kill 并 wait 外部进程、清理 `data/cache/workflow/exec/<execution_id>` 临时目录、释放槽后才发失败回执。
- 回执按 `execution_id = <run_id>/<node_id>/<attempt>` 关联；重复、迟到、终态后的回执被拒绝，不复活 Run、不写缓存。
- 重启：SQLite 记录执行 epoch 与活动执行；新实例把旧 epoch 的本地执行置 `interrupted` 并清理临时目录，不触碰其他任务。

#### 4.5.6 用户可扩展性

本期只开放系统内置定义；用户不能提交 `media.` / `sys.` 前缀的 id，也不能自报 `source: system`。后续扩展遵循同一路径：注册新的 FunctionObject（Native 或其它 FunctionType）到 registry，再在定义里以 `/tool/<name>@<version>` 引用；不允许在定义里写命令行模板。

### 4.6 发送前决策：直传 / 缩小 / 转换

给定一个文件，模型分析之前先回答三个问题。这是 `llm_understand_media` 在工具进程内完成的规划步骤，agent 不可见；调用方仍只表达"理解这个文件"的目标。

| 问题 | 判断依据（目标） | 当前实现 |
|---|---|---|
| 1. 能直接发给目标模型吗？ | 实际选中的模型与 provider 的输入格式、模态和大小限制 | 只有两条判断：协议是否接受 Document（按 `protocol_adapter_id`：`claude-messages`、`gemini-interactions`、`openai-responses`）；图片 MIME 是否属于 png / jpeg / webp / gif。`route_resolve` 返回的 `enabled_capabilities` 未使用。路由查询失败时视为不接受，文档走转换 |
| 2. 需要先缩小以节约 token 吗？ | goal、分辨率、页数、时长、剩余上下文预算 | 图片看尺寸与字节数；PDF 只看字节数，不看页数（§10 G8）；父预算未接入（§6.3） |
| 3. 需要先转换格式吗？ | 原格式不支持、成本过高或不便检索 | 转成页图 / 分页文本 / 帧 / 音轨，由 manifest 组织（§4.6.2） |

统一流程：轻量 probe → 判断模型能力与预算 → 需要转换时 `create_run(media.<family>, ensure=true)` → 读取 manifest 按 goal 选取内容 → 理解并返回带来源的结果。

#### 4.6.1 决策表

由 [`prepare_media_content`](src/frame/agent_tool/src/llm_understand_media.rs#L1620) 实现，自上而下首个命中生效：

| 输入 | 条件 | 去向 |
|---|---|---|
| 归档（zip / 7z / rar / gzip / tar / zstd / bzip2 / xz） | — | Error，要求先解包 |
| 音频 | — | AICC `audio.asr` 转写，文本进模型；不经 workflow |
| 图片 | MIME 属于直传列表、头部能读出尺寸、在图片限制内 | 直传 |
| 图片 | 其余 | `media.image`，读 `model_media_obj_id` |
| 视频 | — | `media.video`；帧进模型，音轨转写后进模型 |
| Office（docx / pptx / odt / odp / doc / ppt / rtf，按 MIME 或文件名） | — | `media.document` |
| PDF | 协议接受 Document 且 ≤ 20 MiB | 直传 |
| PDF | 其余 | `media.pdf` |
| `text/*`、`application/json` | ≤ 2 MiB | 文本内联；更大时 Error，提示用 `read` |
| 其他（含 XLSX） | 协议接受 Document 且 ≤ 20 MiB | 直传 |
| 其他 | 其余 | Error，说明 MIME 与协议 |

环境变量（`LLM_UNDERSTAND_MEDIA_` 前缀）：

| 变量 | 默认 | 用途 |
|---|---|---|
| `IMAGE_MAX_EDGE` / `IMAGE_MAX_PIXELS` / `IMAGE_MAX_BYTES` | 2048 / 4194304 / 3670016 | 图片直传判断，同时是 `media.image` 输入 |
| `PDF_DPI` / `PAGE_MAX_EDGE` / `MAX_PAGES` | 144 / 2048 / 50 | `media.pdf` / `media.document` 输入 |
| `DOCUMENT_MAX_BYTES` | 20 MiB | 文档直传上限 |
| `VIDEO_FRAMES` / `VIDEO_FRAME_MAX_EDGE` | 16 / 1280 | `media.video` 输入 |
| `TEXT_CHARS` / `MAX_IMAGES` | 60000 / 16 | 一次送给模型的文本字符数与图片数 |

失败处理：

- 转换 Run 失败 → `AgentToolStatus::Error`，message 带 workflow 的结构化错误（code、message、details），`details.media_decision.kind = failed`。`tools_missing`、`environment_changed`、`function_not_found`、`interrupted` 标注为部署 / 内核诊断。
- workflow 不可达属于内核服务故障，直接报告；不触发本地转换或专门的直传容灾路径。已知模型不支持的格式不重复尝试直传。
- v0.5 的"转换失败后再试直传"已删除：走到转换时，要么协议不接受，要么超预算，这条分支没有可走的情况。
- 直传被 provider 拒收时目前直接 Error；应退到转换一次（§10 G8）。

#### 4.6.2 manifest（目录表示）

`media.build_catalog` 产出的 manifest 是 buckyos-api 的 `MediaManifest`（[src/kernel/buckyos-api/src/media_manifest.rs](src/kernel/buckyos-api/src/media_manifest.rs)，对象类型 `mmanifest`），是纯、可缓存的 Named Object。`media.image` 没有 catalog 步骤，结果就是 `fit_image` 的输出。

```jsonc
{
  "schema_version": 1,
  "family": "pdf | video | document",
  "source_content_id": "<原件 chunk / chunklist id>",
  "derived_content_id": "<document → PDF 时的派生内容 id>",
  "probe": { … },
  "range": { "total": 42, "requested": [1, 2, 3], "processed": [1, 2], "failed": [3] },
  "entries": [
    { "kind": "page", "where": { "source": "original | derived", "page": 1 }, "text_obj_id": "…", "image_obj_id": "…", "status": "ok" },
    { "kind": "page", "where": { "source": "original", "page": 2 }, "status": "no_text", "image_obj_id": "…" },
    { "kind": "page", "where": { "source": "original", "page": 4 }, "status": "unprocessed" },
    { "kind": "frame", "where": { "source": "original", "time_ms": 12000, "index": 3 }, "image_obj_id": "…", "status": "ok" },
    { "kind": "audio", "where": { "source": "original", "time_ms": 0 }, "status": "absent" }
  ],
  "complete": false
}
```

- 条目状态：`ok` / `no_text`（扫描页、空白页）/ `absent`（无音轨）/ `unprocessed`（在源中但不在请求范围）/ `failed`（带 `error {code, message, retryable}`）。缺失以状态表达，不让条目消失：文档族从第 1 页到最后一页每页一条。
- `failed` 条目可能仍带另一种表示（文本成功、页图失败时保留 `text_obj_id`）。
- 坐标：文档的 `range` 和 `where.page` 是页码（`total` 是总页数）；视频的 `range` 是帧时间戳（`total` 是时长毫秒）。
- `complete` 只有整份源都处理成功才为 true：存在 `failed` 或 `unprocessed` 条目都是 false，所以按范围读取时它总是 false。「本次请求是否处理完」看 `range.requested` 与 `range.processed`；[media_manifest.rs:113](src/kernel/buckyos-api/src/media_manifest.rs#L113) 的注释写的是后者，与实现不符，应改注释。
- 正常缺失（无文本、无音轨）函数成功，不 abort Run；部分失败仍能建 manifest；含可重试条目错误的 partial 不写可复用缓存，后续请求重新执行失败步骤。
- manifest 不含 FileObject ID、workflow_id、revision、凭据、耗时；这些在 Run output / run meta / 交付层。

#### 4.6.3 输入预算与质量策略

- 目标：预算来自父预算的剩余额度（§6.3），不重置。当前 helper 入口拿不到父预算，只有 `--max-completion-tokens` 与上表的 `TEXT_CHARS` / `MAX_IMAGES`（§10 Q2）。
- 缩小后无法辨认的区域可按需以更高 `dpi` / `max_long_edge` 再建 Run 读取；不能为省 token 静默丢失关键信息。当前工具不会自动发起，图片缩小时在 `output` 追加说明，提示 Agent 裁剪后再看。
- 全篇总结分批覆盖各部分再汇总（未实现，§10 G4）。
- 各模型的默认预算值先按 provider 文档配置，再按真实数据调。

#### 4.6.4 决策记录

每次调用把决策写进 `details.media_decision`、管线事实写进 `details.pipeline_trace`、转换摘要写进 `details.media_preprocess`，都不进 `output`：

```jsonc
{
  "media_decision": {
    "kind": "direct | converted | pending | failed",
    "reason": "PDF pages are rendered and their text extracted",
    "family": "media.pdf",          // 直传时为 null
    "obj_id": "cyfile:…",
    "mime": "application/pdf"
  },
  "pipeline_trace": {               // 没建 Run 时不存在
    "workflow_id": "media.pdf",
    "run_id": "run-…",
    "revision": "<definition_revision>",
    "status": "completed",
    "reused": false,                // 加入了已有活动 Run，或接回账本里的 Run
    "recoveries": 0,                // 重建 Run 的次数
    "steps": {
      "capabilities":     { "state": "completed", "source": "executed", "attempts": 1 },
      "render_pdf_pages": { "state": "completed", "source": "cache", … }
    },
    "run_error": null
  },
  "media_preprocess": { "manifest_obj_id": "…", "summary": { … } }
  // 图片：{ original, sent, actions, model_media_obj_id? }
}
```

`source` 取自 Run 的 `node_sources`（`cache` / `executed` / `direct` / `submitted`）；不要求所有步骤都 `cache`。步骤失败时 `steps.<node>.error` 与 `run_error` 原样携带 workflow 的结构化错误。

#### 4.6.5 格式支持目标

| 输入 | 转换策略 | 验收重点 |
|---|---|---|
| PDF | 模型支持且预算合适时直传；否则 `media.pdf` 产出分页文本 + 页图 + manifest；扫描页靠页图做视觉覆盖，本期不做 OCR | 文本 PDF、扫描 PDF、图文混排及长 PDF 均可理解；能找到后部页面并引用页码 |
| DOCX / PPTX / ODT / ODP / DOC / PPT / RTF | `media.document`：soffice → PDF，再复用 PDF 三步；页码明确属于派生 PDF | 正文与版式信息可检索；页码只引用派生 PDF |
| Excel | 延后：不把 PDF 当成公式 / sheet / 单元格语义，本期不引入 Office 解析 crate。协议接受 Document 时走直传，否则 Error，Agent 可用 `import_table` | — |
| 视频 | `media.video`：均匀抽帧 + 音轨；音轨转写在 agent_tool 走 AICC ASR；ASR 失败时只分析帧并在 coverage 记录 | 无音轨为正常 absent；按时间段检索未实现（`extract_frames` 只有 `uniform` 策略，没有时间范围参数） |
| 音频 | AICC `audio.asr` 转写；要求同步返回，异步任务报错 | 转写文本带来源说明，不声称识别出音乐、环境音 |

### 4.7 目录理解与检索

manifest 进入模型分析时，不能把 summary 或首页当作全文。目标做法是**宿主检索循环**：工具进程读 manifest → 在全部文本条目中检索 goal 相关内容 → 选取相关页图 / 帧 → 分批送模型 → 汇总；旁路模型仍不开工具。受控旁路工具方案待宿主循环有真实数据后再评估。

当前实现（[`plan_pages`](src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L906)）是一次 Run 的 manifest 上做一轮选择，没有循环：

1. 文本：全部页文本不超过 `TEXT_CHARS` 就全送；否则按 goal 的词法命中打分，从高到低选到预算用完。
2. 页图：无文本的页优先（视觉覆盖），其次是已选文本的页，直到 `MAX_IMAGES`。
3. 没送给模型的页记入 `coverage.uncovered`，并在 `output` 末尾追加一行说明。
4. 视频：帧按时间顺序取前 `MAX_IMAGES` 张，其余记入 `uncovered`。

要求与差距：

- 扫描 PDF / 纯视觉视频不能只做词法检索：`no_text` 条目必须用页图 / 帧做视觉覆盖，或在 coverage 中明确报告未覆盖（已实现）。
- 超过 `limits.max_pages`（默认 50）的页只标 `unprocessed`，不会继续处理（§10 G4）。
- 中文 goal 的词法打分退化（§10 G5）；产物读取失败时 coverage 虚报（§10 G6）。
- 每条重要 observation 带定位：页码（并说明属于原件还是派生 PDF）或时间戳，作为 `ObservationItem.where`（§10 G7）。
- `details.coverage` 由工具填写；局部抽样不能声称完整阅读：

```jsonc
"coverage": {
  "family": "pdf",
  "total": 42,                 // 总页数；视频为时长毫秒
  "requested": [1, 2, 3],      // 页码；视频为帧时间戳
  "processed": [1, 2],
  "failed": [3],
  "text_delivered": [1, 2],    // 文本送给了模型的坐标
  "image_delivered": [2],      // 图片送给了模型的坐标
  "uncovered": [{ "where": 3, "reason": "tool_failed: render" }],
  "audio": null,               // 视频：absent / transcribed / failed: … / not_requested
  "complete": false            // manifest 的 complete，不代表模型都看到了
}
```

### 4.8 共享表示缓存与授权

转换产物由 workflow 的 Step cache 跨 Run、跨 principal 共享；不再复用 nfs_server preview 的 `repr` / `preview-cache`（裁决 P1：preview 面向浏览器可用，本管线面向 LLM 可用，Target Profile 不同）。

#### 4.8.1 复用规则

- **活动 Run 合并（ensure）**：`create_run(ensure=true)` 以 `hash(principal, definition_revision, trigger_output, environment_revision)` 查找活动 Run（[server.rs:755](src/kernel/workflow/src/server.rs#L755)）；同 key 的进行中 Run 直接返回（`reused: true`）。`trigger_output` 含 `obj_id`、`content_id`、`size`、`name` 和有效参数，所以相同内容但 FileObject 不同的两次请求不合并，只靠 Step cache 复用。只合并活动 Run；Completed / Failed / Aborted 不复用，终态即释放索引。不同 principal（含同 owner 的不同 Agent）各有自己的 Run，首次并发允许重复计算（裁决 R1）。
- **已完成结果复用**：新 Run 的具备缓存资格的步骤按 §4.5.4 命中 Step cache，事件 `step.completed{source: cache}`；`capabilities` 每次重新执行。
- **失效**：工具版本（指纹）或 implementation revision 改变不命中旧缓存；无关工具变化不影响；产物过期或被删后命中校验失败，重算。
- **Agent 侧账本**：工具把"等待中的 Run"记在 `sessions/<id>/.llm_understand_media/runs/`（helper 入口没有 session id，退到系统临时目录），键是 `hash(family, obj_id, 有效参数)`。同一请求再次调用时查询原 Run，不再 `create_run`。

#### 4.8.2 授权

- 请求者凭 session token 验证；`create_run` 用请求者身份调 NDM 的读类接口 `query_object_by_id` 检查其可读输入 `obj_id`（[server.rs:136](src/kernel/workflow/src/server.rs#L136)；存在性探测 `is_object_stored` 对任何上传者开放，不能用于授权）。DV WF-DV-013 验证了种子用户 bob 只知道他人 obj_id 时被拒绝。
- Run / 定义的读取、启动、取消、输出获取都按 owner 校验，管理员可观察全部；Task 树 creator 是 workflow，`requested_by` 记录已验证调用者。
- `get_run_output` 返回稳定 obj_id 与单独的 `delivery` 块：`artifacts`（Run output 里直接出现的 obj_id）、`grants`（对其中绑定了 bucket key 的产物给出 bucket key / locator / 绑定 principal 的只读 token / `expires_at`，6 小时）、`principal`、`expires_at`；凭据不进 manifest、cache、trace。
- 缓存命中也不跳过授权；请求者凭据不写共享缓存。

两个与直觉不同的事实（处理见 §10 Q6）：

- **Agent 有全局对象读权限。** 策略表里 `p, agent, obj://ndm/objects, read, allow`（[rbac_config.rs:264](src/kernel/buckyos-api/src/rbac_config.rs#L264)），所以 Agent 只要知道 obj_id 就能读，"知道 obj_id 不等于有读权"对 Agent 不成立，`create_run` 的输入校验对 Agent 总是通过。
- **`delivery` 只覆盖 `media.image`，Agent 也没有用它。** Agent 用 NDM 读权限直接读 manifest 和产物。`media.image` 的 output 里是 `model_media_obj_id`，有 token；另外三条只有 `manifest_obj_id`，它不绑定 bucket key，没有 token，manifest 引用的页图和文本也不在 `delivery` 里。结果是没有 NDM 读权限的主体读不到文档和视频的转换结果。经 token 的数据面读取没有 DV 覆盖。

#### 4.8.3 内容身份

- `NamedObject` 的 `obj_id`（FileObject）直接作为输入；服务端解析其 `content` 作为计算身份。
- 容器内 `local_file` 与 URL 没有 obj_id：确需转换时先冻结为对象，再以 `obj_id` 调用；确定不了身份就不能进管线。当前冻结经 NDM 写入 Named Store，靠上传 hold 保留 24 小时（`HOLD_TTL_SECS`），不进 bucket；`scratch` 桶（7 天）目前只用于 T2d 的 `inputs/<execution_id>`。TODO D2 写的是 scratch 7 天；24 小时足够覆盖一次 Run 的期限（最长 45 分钟），本文按实现记录，不再要求 scratch。
- 冻结当前有三个问题（整文件入内存、选路前无条件上传、FileObject 时间字段取当前时间），见 §10 G2。

#### 4.8.4 边界

- 区分可复用的**转换产物**与本次 goal / 模型相关的**理解结论**：理解结论不进 workflow 缓存。
- manifest 中的分页文本是否也让 `read` 工具直接按页读取，见 §9。

---

## 5. 崩溃恢复语义

### 5.1 模型分析 stage 作为 xllm run

转换阶段在 workflow 的 Run 里，幂等靠 Step cache；真正进入 LLM 的模型分析 stage 是一个 xllm run，input 里已经是构造好的媒体内容和 goal。v0.5 及之前设想的 `OneShotRequest::semantic_hash()` 已不存在，恢复不依赖语义哈希。管线版本记录在 `details.pipeline_trace.revision`，用于诊断。

### 5.2 恢复规则

下面三个边界彼此独立，不能混为"per-turn 持久化"（术语见 [readme.md](readme.md)）：

- **模型推理 checkpoint**：模型分析 stage 以 xllm run 执行（[xllm.rs](src/frame/agent_tool/src/xllm.rs)），持久化粒度是推理边界：每次推理（Round）开始前由 `InferenceHook` 把 `LLMContextSnapshot` 写入 run 目录的 `snapshots/`，到达 outcome 时再提交一次；不存在"每个 Turn 一份快照"。主干 run 同理：function call 模式在每次推理前 checkpoint，behavior 模式只在 Step 边界 checkpoint（`CheckpointHook`）。
- **工具结果提交**：对主干而言，整个 `llm_understand_media` 只是一次工具调用，身份是 `call_id`。function call 模式下 `ToolUse` / `ToolResult` 随主干下一次推理前的 checkpoint 一起持久化（与同一响应的其它调用同属一个工具批次，共消耗一次工具迭代）；behavior 模式下结果写入当前 Step 的 `action_results`，随 Step 边界 checkpoint 持久化。libopendan 宿主在调用开始前登记 in-flight 记录，只有包含该结果（或显式 `Unresolved`）的 checkpoint 才清除它；之后 flush 时写成 Session worklog 的 `action_result {call_id}`。
- **Turn 完成**：工具结果提交不完成 Step（behavior 模式下 Step 要等它的全部 action 结果就绪），更不完成 Turn。Turn 是否完成只由 Session 在 run 结束时判定。

旁路推理的归属：旁路里的推理（模型分析 stage、§4.2 压缩父 history 的摘要调用）使用工具自己的 `LlmClient` 和 run，是这次工具调用内部的嵌套模型调用：

- 不是主干的 Round：不经过主干 run 的 `LlmClient::infer`，不计入主干 `run.json` 的 `usage.llm_requests` 或 libopendan `static.json` 的 `rounds`，只记在旁路自己的 run 记录里；只有经主干主循环发起的推理才是主干的 Round。
- 不是主干的 Step：主干在 behavior 模式下，这次调用只是当前 Step 的一个 action。
- 不是 Session Turn：旁路没有 Session 输入，不开启、推进或完成任何 Turn。

崩溃时：

- 主干在调用进行中崩溃 → 当前实现不续跑旁路。libopendan 宿主恢复时把没有已持久化结果的 in-flight 调用显式回填为 `unresolved`（效果未知），由主干模型决定是否重新调用；单独运行的 xllm 主干则从上一次推理前的快照恢复，重新推理。旁路的 run 目录不会被主干接手。"旁路状态可恢复时自动 resume"仍是设计目标，校验与接手规则待定。
- 重新调用时，账本（§4.8.1）接回仍在进行的 Run，已完成的 pure 步骤从 Step cache 复用。
- workflow 重启：旧实例的本地执行以 `interrupted` 结束，Run 不续跑；工具查询原 Run 得到 not found 时在预算内重建（§5.3）。
- 旁路内部中间状态不渗透到主干 recovery 模型。
- 由于 `media` 是 `ResourceRef::NamedObject { obj_id }`、媒体存于 `ndn_lib` NDN 对象，重跑时 media 必然仍可寻址、内容必然未变（`ObjId` 即 content hash），重跑结果语义一致。

> 嵌套调用的恢复因此两层互不渗透：主干按自己的推理 / Step 边界快照恢复，旁路有自己的 run 与快照。

### 5.3 长耗时转换

图片理解通常不等待转换。长文档 / 长视频的 Run 可能持续数分钟到数十分钟。

**当前实现（[pipeline.rs:636](src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L636)）：**

- `create_run(ensure=true)` 后每 400 毫秒查询 `get_run_output`，2 秒内没到终态就返回 `Pending`（`LongRunning`、`check_after` 5 秒、`task_id = run_id`），`details.resume` 里带 run_id 和有效参数。
- 账本记录等待中的 Run；同一请求（或带 `--resume`）再调用时查询原 Run。
- `run_not_found`（服务重启）或明确 `retryable` 的失败才重新 ensure，最多 3 次，从首次请求算起总期限 30 分钟；超限后报 `run_lost` / `deadline_exceeded`。

**问题：这个 Pending 在真实 Session 里走不通（§10 G1）。** shell 把 Pending 转发给 runtime，runtime 按 `task_id` 去等 TaskMgr 任务（[xllm.rs:4377](src/frame/agent_tool/src/xllm.rs#L4377)）；`run-…` 不是 TaskMgr 任务 id，结果是 `Unknown`，模型看到"task state unknown"的错误，`resume` 也不可见。这是代码路径推导，未实跑。

**目标语义不变**（TODO D8）：LLM 不管理转换状态；同一请求不重复建 Run；恢复受总期限和次数限制。如何达成见 §10 Q1。`shell` 已经有长任务模型：命令超过等待预算（默认 25 秒）后交给任务管理器，结束时完整结果自动回到 Agent（[llm_bash.rs](src/frame/agent_tool/src/llm_bash.rs) 模块注释）。

`AgentToolPendingReason::UserApproval`（如 goal 触发隐私敏感判断）目前未使用。

---

## 6. 约束与边界

### 6.1 媒体引用强烈倾向 content-addressed

- **首选** `ResourceRef::NamedObject { obj_id: ObjId }`——可寻址、可完整性校验、由 `ndn_lib` 统一管理生命周期、天然跨 zone/peer。
- **可接受** `ResourceRef::Url { .. }`，但调用方须自承过期 / 跨设备失效风险，本工具不持有引用计数。
- **禁止**任何写回主干 `tool_result.content` 的 `AiToolResultContent::Image` / `Base64` payload；用户原始消息里若是 `Base64`，应在装配层尽早转存为 NDN 对象、改写为 `NamedObject` 再进 history（这是 `AiccClient` 层的职责，不在本工具范围）。

### 6.2 NDN 对象 GC 与引用计数

- **GC 根**：主干 history 中引用某 `obj_id` 的 `AiContent::Image` / `AiContent::Document`（用户原始消息或 `tool_result`）。
- 实现要求：`Msg-Center` 在消息写入 / 持久化时，若消息内含 `ResourceRef::NamedObject`，负责经由 `ndn_lib` 对对应 `obj_id` 增引用计数 / 打 pin。该 pin 挂钩是消息中心的职责，**不在本工具实现范围**。
- `llm_understand_media` 的调用和处理流程**不管理 GC**：它只消费 `ResourceRef`，不新增长期引用根，不负责增删引用计数，也不决定对象回收时机。冻结本地文件时的 24 小时上传 hold 和 workflow 产物的 30 天 lease 都是短期保留，不是长期引用。
- 必须保证：只要主干 history 中仍存在引用该媒体的消息，该 NDN 对象不被回收。否则主干 history 看似完好，重 fork 时对象已悬空。
- 引用计数的根是**主干 history**，不是旁路——旁路本就是临时的，不持有长期引用。

### 6.3 旁路深度控制

- `llm_understand_media` 是封装层提供的、语义明确的受控 `llm_*` 工具，**不是开放给 agent 的通用 fork 原语**。
- 因此"旁路内部是否再触发嵌套 `llm_*`"是**设计时静态可推**的，而非运行时不可控。
- 本期模型分析 stage 的 xllm run 必须：
  - 关闭工具（`TaskOverrides.tools = Some(false)`，已实现）——旁路内部**不得**再触发嵌套 `llm_*` 旁路（深度上限 = 1）；
  - `budget` 显式继承父预算的剩余额度，**不允许**重置为 default 无上限——这是防"旁路成本黑洞"的硬约束。**未实现**：helper 入口拿不到父预算（§10 Q2）。
- 若未来确需嵌套，必须设定明确的深度上限与预算上限，并接入 L4 scheduler 对 `llm_*` 调用的统一预算账户。

### 6.4 主干 history 的体积特征

正确实现后，主干 history 因媒体理解产生的增长**仅随 `AgentToolResult.output` 的文本长度线性增长**，与"读过多少次媒体"无关，与媒体实体大小无关。这是本设计的核心收益，实现验收时应据此度量（见 §8）。

---

## 7. 典型流程示例

```
用户上传截图：
  → 附件落入 NDN，得 obj_id=O1
  → 消息带 [attachment image "error.png" -> O1] 进入主干 history
  → 持久化层对 O1 增引用计数（§6.2）

主干推理（第 1 次 Round）:
  → shell: llm_understand_media O1 --goal "解释这个报错"
  → 旁路进程：
      resolve_media: 读 FileObject 和内容头部 4 KiB，得 mime=image/png、size、1920x1080；
      选路: 在图片限制内 → 直传，不建 Run（6000x4000 的照片则走 media.image，
            fit_image 返回 model_media_obj_id，模型看缩图，output 追加缩放说明）；
      model: xllm run，input = 内置提示词 + 父 history 快照（如有）+
             User[图片, "Goal: 解释这个报错"]；工具关闭。
  → AICC 处理含 image 的 LLM 请求 → 产出 UnderstandingReport:
        observations: [
          obs-1: 红色错误框,
          obs-2: 文本 "OutOfMemoryError",
          obs-3: 堆栈中反复出现 ArrayList.grow ...
        ]
        reasoning: "由 obs-2 与 obs-3，堆栈在集合扩容处反复出现，
                    结合 obs-2 的错误类型，判断为内存泄漏 ..."
        conclusion: "这是一个内存泄漏导致的 OutOfMemoryError ..."
        confidence: Inferred
  → 装配 AgentToolResult{ status: Success, output: <report 紧凑渲染>, detail: <report + 决策记录>, .. }
  → 主干仅新增: shell 的 ToolUse + ToolResult { content: [Text { text: output }] }
  → O1 像素不再出现在主干

主干推理（第 30 次 Round，可能已在之后的 Turn）：怀疑"内存泄漏"判断是否成立
  → 主干 history 中仍有第 1 次 Round 发起的那次调用的 tool_result.text（含 reasoning）
  → agent 直接在主干上复核 reasoning，无需重新 fork
  → 仅当需要 observations 之外的新视觉细节时，才再次 fork(O1 仍可寻址，
      因 §6.2 引用计数挂钩，O1 未被 GC)

用户上传 42 页扫描 + 文字混排的 PDF，模型协议不接受 Document：
  → shell: llm_understand_media O2 --goal "第三季度的营收是多少"
  → 旁路进程：
      create_run(media.pdf, ensure=true, {obj_id: O2, ...})
      workflow: capabilities → probe → extract_pdf_text → render_pdf_pages → build_catalog
      读 manifest：31 页 ok、10 页 no_text、1 页 failed
      选内容：按 goal 选文本页；no_text 页优先送页图，至多 16 张
      model: xllm run，User[目录说明, 各页文本, 页图..., Goal]
  → output 末尾："Note: 15 of 42 requested parts were not shown to the model (...)"
  → details: media_decision / pipeline_trace（第二次问同一文件时转换步骤 source=cache）/ coverage
```

---

## 8. 实现验收标准

workflow 侧的验收以 TODO §5 为准，这里只列整个工具的验收。状态：**已验证**＝真实 Zone 上跑过；**单测**＝代码和单测都有，未在真实 Session 运行；**未满足**＝与要求有差距；**未做**＝还没有实现或还没测。

| # | 项 | 要求 | 状态 |
|---|---|---|---|
| 1 | 接口 | 入口是 helper 命令，agent 只表达 media 与 goal；fork / history 继承 / 内置 prompt 不可见 | 单测 |
| 2 | 单点存储 | 媒体实体仅存于 NDN 对象；主干 history 与旁路 input 中均只持有 `ResourceRef::NamedObject`（或可接受的 `Url`），不持有 `Base64` 副本 | 未满足（旁路 input 用 Base64，G3） |
| 3 | fork-and-discard | 主干只含一对 `ToolUse` / `ToolResult`；`ToolResult.content` 内仅有文本，无 `Image` / `base64`；旁路工作目录在返回后清理 | 部分：主干已满足（单测），工作目录未清理（G3） |
| 4 | 报告结构 | `details` 反序列化为 `UnderstandingReport`，含 `observations`（带 id）/ `reasoning` / `conclusion` / `confidence`；`reasoning` 中每个事实可追溯到某 `observation.id` | 单测 |
| 5 | 写回最小化 | `output` 是 report 的紧凑文本渲染，只进一个 `Text` 块；不出现 `to_value(AgentToolResult)` 全量塞进 LLM 的反模式 | 单测；带 shell 运算符的命令例外（§2.3） |
| 6 | 提纯继承 | 提供父 history 时，除 target `media` 外其他 `AiContent::Image` / `AiContent::Document` 全部替换为 `Text` 占位 | 单测；真实入口不传（Q2） |
| 7 | 系统内置管线 | 确需转换的媒体经 `media.<family>` Run 完成 probe 与转换；`media.image` 的 probe 输出原始分辨率、size、MIME，`fit_image` 在图片过大时生成派生图并返回 `model_media_obj_id` | workflow 侧已验证；Agent 调用未做 |
| 8 | Workflow Service 承载 | 四条管线是系统定义（`source: system`，只读，`/tool/media.*@1` 绑定真实 FunctionObject）；agent 只经 `WorkflowServiceClient` 调用，无本地 image_fit / ffmpeg 转换路径 | 已验证（定义与只读，WF-DV-012 / 013）；agent 无本地路径（代码核对） |
| 9 | Step cache | `idempotent && is_pure` 的步骤按 `hash(thunk_obj_id, 工具指纹)` 复用；命中前校验结果对象与产物可用；`details.pipeline_trace` 按 `node_sources` 标记每步来源 | 复用已验证（WF-DV-009）；trace 单测 |
| 10 | 传统分析 provenance | OCR / barcode 等若启用，必须标明来源，不得把传统分析结果伪装成模型直接观察 | 本期只有 ASR，转写文本带来源说明；OCR 未做 |
| 11 | MIME route | 解析 MIME，按配置得到 AICC 逻辑模型名；target 媒体不经辅助转文字中间层；AICC 无法处理时返回明确 Error | 单测（环境变量路由）；TOML 路由表未做 |
| 12 | 崩溃恢复 | 模型分析 stage 的推理边界快照生效；主干在结果返回前不提交该调用，结果随下一个 checkpoint 提交，工具结果提交不等于 Turn 完成（§5.2）；重新调用接回进行中的 Run | 快照沿用 xllm；接回未实跑 |
| 13 | 深度 & 预算 | 旁路工具关闭；管线和模型分析共享父预算剩余额度，不各自重置为无上限 | 工具关闭：单测；预算未做（Q2） |
| 14 | 体积度量 | 长 session（多次媒体理解）下主干 history 仅随 `tool_result` 文本线性增长，与媒体数量 / 大小解耦 | 未做 |
| 15 | 发送前决策 | 每次调用 `details.media_decision` 记录选择与原因；转换失败时 Error 带 workflow 结构化错误，缺工具时列出缺项 | 单测 |
| 16 | 正常选路 | 适合直传的按能力与预算直传，确需转换的经 workflow；workflow 不可达按内核故障返回；缺基线工具报告 `tools_missing` | 单测；Agent 路径未实跑 |
| 17 | 目录表示 | PDF / DOCX / 视频可生成 manifest；空白页、无文本页、转换失败页在 manifest 中有记录 | 真实工具用例通过（`--ignored`）；WF-DV-011 验证 media.pdf；document / video 的 DV 未做 |
| 18 | 目录检索与覆盖 | 答案能命中 PDF 后部页面和视频后段内容；observations 带页码 / 时间戳；`details.coverage` 准确报告已检查范围、未覆盖部分和转换缺失项（§4.7） | 未满足（G4、G5、G6、G7） |
| 19 | 共享缓存 | 同一 principal 并发合并到一个活动 Run；不同 principal 各有 Run 但复用有效 Step cache；内容 / 参数 / 工具版本 / implementation revision 变化会失效；无权限调用者不能读取输入或产物；docx 的中间产物在缓存有效期内只生成一次 | 合并与复用已验证（WF-DV-009 / 010）；Agent 主体的读隔离不成立（Q6）；docx 未满足（G9） |
| 20 | 集成验证 | 四类格式覆盖不能直传、超预算、扫描件、长文件、部分转换失败和工具缺失六种情形（§4.6.5），标明实际测试的 provider / 模型 / 平台 | 未做 |

---

## 9. 待定项 / 后续迭代

| 项 | 说明 | 处理 |
|---|---|---|
| 父 history 目的导向裁剪 | §4.2.4，长 session 下控制旁路重喂 token 量 | 预留可插拔策略点；先看 Q2 |
| 报告粒度调优 | §3.5，`observations` 完整度 vs 重 fork 频率的平衡点 | 依据真实重复 fork 频率数据调内置 prompt |
| 管线执行位置 | §4.5，P0 在 workflow 进程内 `LocalThunkRunner` 执行 Native | Scheduler / Node Runner 的分布式执行（节点指纹、数据访问、任务交接）后置，保留 `ThunkDispatcher` 接缝 |
| 跨主体 single-flight | §4.8.1 | 本期不做（裁决 R1） |
| 后续媒体函数 | §4.5.3 | OCR / ASR / 关键帧去重等按同一规则新增 FunctionObject；`media.audio` family 未交付 |
| 用户扩展 stage | §4.5.6 | 本期只有系统内置定义；用户 / Agent 发布 FunctionObject 与 Zone 级 registry 后置 |
| OCR | §4.6.5，可显著降低 vision 漏字 | 不进本期；workflow 不依赖 tesseract |
| 嵌套 `llm_*` | §6.3，本期深度 = 1 | 如需开放，先定深度/预算上限并接入 L4 预算账户 |
| 辅助视觉模型降级 | §4.3 末段，纯文本主模型回退 | 本期返回明确 Error；后续实现时复用 `agent_tool` 已有 `llm_*` 工具调用模式 |
| 目录产物与 `read` 的边界 | `read` 目前把非文本对象一律推给本工具 | 确定 manifest 里的分页文本是否可被 `read` 直接读取，使"按需读某页"不必经 LLM 旁路 |
| Excel | §4.6.5 | 延后；不引入 Office 解析 crate |
| 视频按时间段检索 | §4.6.5，需要 `extract_frames` 增加时间范围或关键帧策略 | 有真实需求后再加 |
| 能力信息选路 | §4.6，`enabled_capabilities` 未使用 | 协议白名单先用着，出现误判再接 |
| 各模型预算默认值 | §4.6.3 | 先按 provider 文档配置，再按真实数据调 |
| 新增第三方依赖 | 任何 OCR / PDF / Office 库 | TODO §7 R3 的授权不覆盖这些，按仓库约定单独确认 |

---

## 10. 改造清单（2026-11-05 第二次核对）

workflow 侧（Phase A）的任务与验收以 [notepads/llm-media-pipeline-workflow-todo.md](notepads/llm-media-pipeline-workflow-todo.md) §4 / §5 为准；本节列把 `llm_understand_media` 改造做完还差什么。

### 10.1 现状

- agent 侧主流程已提交：`resolve_media` 对 NamedObject 只读头部；按 §4.6 选路；`media.*` Run 驱动、账本与恢复；manifest 消费、`plan_pages`、coverage；本地 `image_fit` / 抽帧已删除。TODO T4 的状态行（"尚未切换到 workflow"）已过时。
- 本次运行的检查：`cargo test -p agent_tool llm_understand_media`（27 通过）；`cargo test -p workflow`（66 通过，6 个需要真实工具的 `#[ignore]` 未跑）；soffice 重复转换实测（§4.5.2）。
- 没有运行：真实 Agent Session 上的任何媒体调用；当前工作树的 Zone 部署；T2d 的 `/ndn/` DV；干净 paios 镜像探测。

### 10.2 差距清单

G1–G6 不解决就过不了 §8 的验收，其中 G1、G2 影响真实 Session 的基本通路。下表的推导都来自读代码，没有实跑，开工第一步是在 dev Zone 上跑通并确认。

| # | 差距 | 依据 | 处理 |
|---|---|---|---|
| G1 | Pending 在 Session 里变成错误：`task_id` 是 run_id，runtime 去等 TaskMgr 任务，查不到就报 state unknown，`resume` 不进 LLM 可见文本 | [llm_understand_media.rs:540](src/frame/agent_tool/src/llm_understand_media.rs#L540)、[llm_bash.rs:1633](src/frame/agent_tool/src/llm_bash.rs#L1633)、[xllm.rs:4377](src/frame/agent_tool/src/xllm.rs#L4377)、[tasks_zone.rs](src/frame/agent_tool/src/tasks_zone.rs)、[task_model.rs:322](src/frame/agent_tool/src/task_model.rs#L322) | Q1 |
| G2 | `local_file` / URL / base64 输入：① 整文件读入内存，agent 侧没有大小上限；② 选路前无条件冻结，直传也要上传；只读 Agent（`agent_read_only`）按策略表没有 `obj://ndm/uploads` 写权限，连直传都会失败；③ FileObject 时间字段取当前时间，同一文件两次冻结 ID 不同，账本键和 ensure 键都含 obj_id，重复调用接不回原 Run | [llm_understand_media.rs:1215-1291](src/frame/agent_tool/src/llm_understand_media.rs#L1215)、[pipeline.rs:325-333](src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L325) | 确需转换时才冻结；时间字段固定为 0；按块流式写入并设上限；只读 Agent 需要转换时报清楚的权限错误（是否放开随 Q6） |
| G3 | 媒体字节以 Base64 进旁路 input，随 xllm 快照落盘（快照路径没有剥离逻辑）；默认工作目录在系统临时目录，结束后不清理；16 张页图还要在 agent 进程里全部读入并编码 | [llm_understand_media.rs:1941-1952](src/frame/agent_tool/src/llm_understand_media.rs#L1941)、[llm_understand_media.rs:2439](src/frame/agent_tool/src/llm_understand_media.rs#L2439) | Q3；另外正常结束后删除默认工作目录，解析失败时保留诊断 |
| G4 | 超过 `max_pages`（默认 50）的页只标 `unprocessed`，工具不再处理；`page_range` 恒为空；50 页全部渲染 PNG，但最多只看 16 张 | [pipeline.rs:111](src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L111)、[functions/media/mod.rs:237](src/kernel/workflow/src/functions/media/mod.rs#L237) | Q4 |
| G5 | 中文 goal 的词法打分失效：按非字母数字切词，CJK 字符算字母，整句成一个词，几乎匹配不到，选页退化成按页序 | [pipeline.rs:886](src/frame/agent_tool/src/llm_understand_media/pipeline.rs#L886) | CJK 按二元组切词，补单测 |
| G6 | coverage 虚报：页图读取失败只打日志，`image_delivered` 仍包含该页；页文本读取失败被当作无文本页 | [llm_understand_media.rs:1919-1951](src/frame/agent_tool/src/llm_understand_media.rs#L1919) | 先读产物再定 coverage；读失败记入 `uncovered` |
| G7 | `ObservationItem` 没有 `where`，内置提示词不要求定位 | [llm_understand_media.rs:54-74](src/frame/agent_tool/src/llm_understand_media.rs#L54) | 加可选 `where`（页码 / 时间戳 / 所属源），提示词和 schema 同步 |
| G8 | 直传只看字节数：一份 15 MiB、300 页的 PDF 会直传，超过 provider 页数限制时直接 Error，没有退路 | [llm_understand_media.rs:1667-1695](src/frame/agent_tool/src/llm_understand_media.rs#L1667) | provider 拒收时退到 `media.pdf` 一次，`media_decision` 记录 |
| G9 | `to_pdf` 非 pure，每次重跑 soffice 并使后续步骤全部不命中（§4.5.2） | [document.rs:11](src/kernel/workflow/src/functions/media/document.rs#L11) | Q5 |
| G10 | 文档和视频的 `delivery` 只有无 token 的 `manifest_obj_id`，页图和文本不在其中（§4.8.2）。Agent 靠 NDM 读权限不受影响，但 TODO 把 T1h 标为完成、验收 17 写"可读 manifest 及所有交付条目"，与此不符 | [server.rs:970](src/kernel/workflow/src/server.rs#L970)、[server.rs:2128](src/kernel/workflow/src/server.rs#L2128)、[io.rs](src/kernel/workflow/src/io.rs) | Q6 |
| G11 | 手册过时：`aigc_video.md` 第 15、78 行说 `llm_understand_media` 依赖本机 ffmpeg / ffprobe；`ui_core.md` 第 11 行写了 `pending` / `--resume`，并让模型看 `detail.*`（转发后模型看不到） | [aigc_video.md](src/apps/jarvis_agent/agent/cmd_skills/aigc_video.md)、[ui_core.md](src/apps/jarvis_agent/agent/cmd_skills/ui_core.md) | 随 Q1 结论更新，并按 AGENTS.md 的 cmd_skills 规则验证 |
| G12 | 小清理：`summary.audio_codec` 不存在，`audio_mime` 永远走默认值（恰好对，因为输出固定 mp3）；`image.rs` 模块注释指向已删除的 `image_fit.rs`；`media_manifest.rs:113` 注释与实现不符 | [llm_understand_media.rs:2040](src/frame/agent_tool/src/llm_understand_media.rs#L2040) | 顺手改 |

### 10.3 待确认（Review 时请拍板）

| # | 问题 | 推荐 | 其他选择与代价 |
|---|---|---|---|
| Q1 | 长转换怎么等？TODO D8 定的是"短等待后 Pending"，但这条路不通（G1） | **helper 阻塞到 Run 终态再做模型分析，不再返回 Pending。** `shell` 超过 25 秒自动转任务，结束时完整报告回到 Agent，模型不用管 run_id，正好是 D8 要的"LLM 无需管理"。shell 默认 30 分钟超时；被停后 Run 还在，再调用靠账本接回。删除 `resume`、`--resume`、Pending 结果，保留账本和恢复逻辑。这是对 D8 的修改 | Pending 的 `task_id` 改成 Run 根任务的 TaskMgr id（`t-…`）：`create_run` 要返回 `root_task_id`；任务结束后模型还要再调一次才拿到报告，多一轮推理 |
| Q2 | 真实 Session 没有父 history 和父预算，设计里的继承要接通吗？ | **不接。** 明确"goal 必须自带上下文"，§4.2 的提纯逻辑留给 `--history-file`，预算靠 `--max-completion-tokens` 与 `TEXT_CHARS` / `MAX_IMAGES`。Agent 写 goal 时本来就知道上下文，接通的收益不明显 | runtime 把会话快照路径和剩余预算经环境变量传给 helper：要改 libopendan 的 helper 装配，旁路多一次压缩调用的成本 |
| Q3 | 模型输入继续用 Base64，还是改 NamedObject？ | **改 `ResourceRef::NamedObject`。** AICC 已支持，并用调用者 session 校验读权限；快照不再含字节，agent 进程也不用读图。音频和文档同样处理。要先确认：配置了 `public_named_object_base_url` 的 Zone 里，Claude / OpenAI Chat 这类协议会拿到对象 URL，私有产物在 `/ndn/` 上没有 hint 读不到，需要 AICC 对这种情况退回内联 | 保留 Base64，只做工作目录清理：改动最小，但不满足 §8 第 2 项，快照在运行期间仍然很大 |
| Q4 | 长 PDF 怎么覆盖后部页面？ | **文本先行，分两轮 Run。** 定义增加 `render_page_range`（`render_pdf_pages` 函数本身不变）。第一轮取全部文本（上限 200 页），页图只渲染很少几页；工具选页后第二轮只渲染选中的页，文本步骤命中缓存。超过 200 页用 `page_range` 开窗。同时解决 G4 的白渲染 | 不改定义，按 50 页窗口多跑几轮：每轮渲染 50 张 PNG，一份 200 页 PDF 渲染全部页面，慢很多。只报告 `unprocessed`：不满足 §8 第 18 项 |
| Q5 | `to_pdf` 怎么进缓存？ | **改为 `is_pure: true`，语义是"结果可复用"，不要求字节相同。** 第一次的输出写入缓存，之后 `pdf_content_id` 稳定，后续步骤全部命中。指纹加入字体集摘要，实现版本升到 `impl-2`。这放宽了 TODO §2.3"通过确定性验证后才声明 pure"的规则，需要你确认 | 规范化 PDF 元数据后再判 pure：要引入 qpdf 之类的新依赖或做字节改写。维持现状：每次提问都重跑 soffice 和后续所有步骤 |
| Q6 | Agent 的全局对象读权限（`agent, obj://ndm/objects, read`）和不完整的 `delivery` 怎么处理？ | **本期维持 Agent 经 NDM 读取，文档如实记录，TODO 的 T1h / 验收 17 改为部分完成。** 补全 `delivery` 等出现非 Agent 消费方时再做。只读 Agent 的上传权限、收紧 Agent 读权限另作安全任务，不并入本次改造 | 现在补全 `delivery`：manifest 绑 bucket key，`get_run_output` 展开引用并逐个签 token（50 页 PDF 约 100 个），当前没有消费方。收紧 Agent 读权限、改走 `delivery`：`read_tool` 读消息附件等现有路径都受影响，范围远超本任务 |

### 10.4 动手顺序

Q1–Q6 确认后：

1. 部署当前工作树（含 Bucket §21），用真实 Agent Session 跑一遍小图、PDF、DOCX、视频，确认 G1、G2 的推导和 Agent 主体的 NDM 读写权限。
2. G1（Q1）、G2：打通基本通路。
3. G3（Q3）、G5、G6、G7：模型输入、检索与 coverage。
4. G4（Q4）：定义改动，新的 `definition_revision`；补 WF-DV 用例。
5. G8、G9（Q5）、G10（Q6）、G12。
6. G11 手册；执行 §8 与 TODO §5 的剩余验收。

### 10.5 相关任务状态

- [x] 管线定义与执行（workflow 侧 T1–T3）：已落地，WF-DV-009~013 在真实 Zone 通过。WF-DV-011 走的是工具齐全的分支，`tools_missing` 由单测覆盖。
- [ ] T2d：视频输入走 `/ndn/`：代码和真实 ffmpeg 用例已写，未提交、未部署，DV 未跑。依赖 Bucket 设计 §21 的发布顺序（cyfs-ndn 先行）。
- [ ] T4：`llm_understand_media` 接入：主流程已提交，剩余见 §10.2。
- [ ] T4b：paios / agent-runtime 工具基线：`build_aios` 与 paios 文档已改；镜像未重建，amd64 / aarch64 未探测；宿主安装脚本未加 LibreOffice 与字体；`baseline.rs` 未按媒体基线探测。
- [ ] T5：Desktop Workflow WebUI 只读观测：已改接真实 RPC（`app/workflow/api/store.ts`），有只读态；未在 Zone 上验证。
- [ ] T6b：完整验收（§8），标明实际测试过的 provider / 模型 / 平台。

新增第三方依赖已在 TODO §7 R3 授权；XLSX / 新 Office 解析路线仍后置。
