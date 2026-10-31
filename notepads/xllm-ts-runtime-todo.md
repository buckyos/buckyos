# xllm TypeScript 运行时 TODO：同一套任务语义进浏览器 Worker（独立验证后再被 AiWorkspace 等集成）

日期：2026-10-31

状态：**待开工。** 2026-10-31 用户裁决：xllm 是整个 Agent 体系里的独立工具，后端是命令行 / SDK，浏览器环境下应是一个本地 JS Worker；这件事非常通用（按 xllm 的抽象，还涉及浏览器下如何构造 runtime 的长期架构），**从 [许愿格浏览器优先 TODO](./aiworkspace-wish-browser-first-todo.md) 中抽出来独立做、独立验证，AiWorkspace 之后再集成**。

依据：

- [xllm PRD](<../product/xllm/PRD.md>) §1（“这一层长期独立存在，既供 shell 脚本使用，也供上层 Agent 调用”；“首版交付独立的 `xllm` 命令，并由同一 SDK 提供其核心能力”）、A19（CLI 与 SDK 一致）、§8.3 / §8.4（独立处理的工作、与后续 Agent 工具的关系）。
- [xllm Rust SDK 参考](<../doc/llm_context/xllm_rust_sdk.md>)（“本文只记录 Rust 实现落实 PRD 时固定下来的协议决定，**供 websdk 的 TS 版本对照**”）、§10 宿主装配（`prepare_hosted`：配置以 JSON 传入、宿主给 system、xllm 追加 `capabilities / cmd_manual / runtime_protocol`）。
- [LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>) §0（narrow waist 纪律）、§2 四层（L1 provider / L2 waist / L3 调度 / L4 语义层）、§2.1（LLMContext 是 lib 不是 service，承载方式由调度方选）。
- [AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>) §1 / §3（runtime kind 表；规划中的名称报 `Capability`，不回退到 native）。
- [AiWorkspace 核心架构设计冻结思路](<../doc/workspace/BuckyOS AiWorkspace 核心架构设计冻结思路.md>) §8.6（标准许愿格在当前用户的网页中运行，后端不参与 AI 执行）与 [内核架构改进](<../doc/workspace/BuckyOS AiWorkspace 内核架构改进.md>) D21（Worker + postMessage，不用 iframe）——这是第一个集成方，但本单不做集成。
- memory [[xllm-rust-sdk-impl]]：2026-09-18 Rust 重写时已写明“下一步是在 websdk 里做 TS 版本，对照 xllm_rust_sdk.md 保持 CLI/SDK 语义一致（PRD A19）”。

目标：在 `buckyos-websdk` 里交付 **xllm 的 TypeScript 运行时**——与 Rust 实现同一套任务语义（配置 → 有效配置 → 提示词装配 → 执行循环 → 预算 / 六态结束 → Run 记录与恢复 → 观察事件），核心零环境依赖，可在浏览器 Web Worker（优先）、浏览器主线程、Node / Deno 三类宿主运行；浏览器是一种新的 runtime kind（`browser`），没有 shell，工具与模型访问由宿主注入或经 postMessage 桥到主线程；Rust 与 TS 用同一组向量（JSON fixtures）证明一致。验证通过后，AiWorkspace 的本地执行器、以及任何需要“在用户的浏览器里跑一个有界 LLM 任务”的前端功能都改为用它，不再各自手写循环。

## 1. 现状（2026-10-31 源码核对，code agent 直接采信）

### 1.1 Rust 侧：要对照移植的东西在哪

| 层 | 位置 | 内容 | 对 TS 的含义 |
| --- | --- | --- | --- |
| L2 waist | [`src/frame/llm_context/`](../src/frame/llm_context/src/)：`deps.rs`（`LlmClient` / `ToolManager` / `PolicyEngine` / `WorklogSink` / `Tokenizer` / `InferenceHook` / `CheckpointHook` / `LLMContextDeps`）、`context_loop.rs`（2616 行主循环）、`behavior_loop.rs` + `xml_behavior.rs` + `step_record.rs`（behavior 模式、`XmlStepRenderer`）、`state.rs`（快照 v5 `LLMContextSnapshot { request, state }`）、`outcome.rs`（六态 `Done / PendingTool / BudgetExhausted / Error / …`）、`error.rs`（`LLMComputeError`、`ProviderFailure { Transient, Permanent, Unknown, ContextLimit, Refused(RefusalKind) }`、`RefusalKind { QuotaExhausted, QuotaBudgetExceeded, ResourceNotPermitted, AttributionUnavailable }`）、`request.rs`（`LLMContextRequest`、`ToolPolicy.max_calls_per_round`、`BudgetSpec`、`ErrorPolicy`）、`prompt_budget / prompt_compose / prompt_engine` | crate 依赖 `buckyos-api`（reqwest / kRPC）、`tokio`、`fs2`，`prompt_engine.rs` 直接用 `std::fs`；测试 `src/tests/`（function_call_loop 12、behavior_loop 21、budgets 6、error_policy 15、cancel 12、reply_contract 5、thinking 4） | **不能直接编 wasm**（见 Q1）；但测试用例是现成的向量来源 |
| L4 oneshot / 宿主装配 | [`src/frame/agent_tool/src/xllm.rs`](../src/frame/agent_tool/src/xllm.rs)（13208 行，65 个测试）：常量（:118–160，`RUNTIME_PROTOCOL_VERSION = "xllm/2"`、`RUN_RECORD_VERSION = 6`、`DEFAULT_MAX_TOOL_ITERATIONS = 8`、超时 3600 / 600、`MAX_TEXT/IMAGE_ATTACHMENT_BYTES`、`SECTION_ALIASES`）、`XllmError`（:183）、配置解析与合并（`parse_llm_context_file` :1430、`merge_config_layers` :1738、`TaskOverrides` :1893、`EffectiveConfig` / `EffectiveTools` / `RunLimits` :2182–2275）、模板与提示词（`TemplateEnv` :2377、`PromptPlan` :2507、`build_runtime_protocol_for` :2672、`assemble_prompt` :2874）、XML（`scan_xml_elements` :3139、`XllmActionParser` :3366）、事件（`RunPhase` / `RunEvent` / `RunObserver` :3605–3660）、`XllmDeps`（:3783）、`XllmToolManager`（:3934）、Provider（`AiccLlmClient` :5028：`exact_model` 含 `@` 走 `chat.completions.create`，否则 `helper.llm_chat` + `HelperModelRequirement`；`OpenAiLlmClient` :5351；`TimeoutLlmClient` :5655；`provider_error_from_rpc` :5304、`refusal_kind_name` :5260）、Run 记录（`RunRecord` :5814、`RunStore::{Disk, Memory}` :6062）、入口（`XllmTask::prepare` :6790、`prepare_hosted` :7298、`hosted_request` :7740（`max_calls_per_round: 16`）、`hosted_waist_deps` :7838、`XllmRun::start` :8119 / `resume` :8220、`build_waist_deps` :8090、`classify_error` :8581） | 文件 / 进程 / 锁用 `std::fs` 48 处、`fs2`、进程组 | TS 的命名、字段名（snake_case JSON）、状态机、协议文本都以此为准 |
| Runtime | [`src/frame/agent_tool/src/runtime/mod.rs`](../src/frame/agent_tool/src/runtime/mod.rs)：`AgentRuntime` trait（:353 `descriptor / config / info / open / describe_interrupted`）、`Sandbox: ToolManager`（:373）、`RuntimeDescriptor { runtime_id, kind, host, target, workdir, capabilities }`（:254）、`RuntimeInfo`（:264，`platform / shell_kind / baseline`）、`Capabilities { tools, network, os }`；kind：`native / remote_ssh / remote_node`；`container / container_host / http_proxy_runtime` 为规划名，`from_config` 报 `Capability`（:87） | — | `browser` 要成为新的规划 kind（Q4） |
| 文档 | `doc/llm_context/xllm_rust_sdk.md`、`LLM Context 设计.md`、`AgentRuntime 设计.md`、`llm_context_behavior_loop.md`、`readme.md` | — | 新增 `xllm_ts_runtime.md`（M0） |

### 1.2 websdk 侧：能复用什么

| 已有 | 位置 | 用法 |
| --- | --- | --- |
| AICC 类型与客户端 | [`src/aicc_client.ts`](../../buckyos-websdk/src/aicc_client.ts)：`AiMessage / AiContent / AiToolSpec / AiToolCall / AiUsage / AiccError / AiccErrorCode`、`AICC_POLICY_REFUSAL_CODES`、`aiccRefusalOf()`（:139）、`AiccClient.chatCompletionsCreate()`（:609）、`helper.llm_chat`（:35） | TS `AiccLlmClient` 直接用；错误分类用 `aiccRefusalOf` + `RPCError.code`（[`krpc_client.ts:46-55`](../../buckyos-websdk/src/krpc_client.ts)） |
| Worker 先例 | [`src/internal/hash_worker.ts`](../../buckyos-websdk/src/internal/hash_worker.ts)（:102 `new Worker(getWorkerBlobUrl())`，Worker 源码内联为 Blob URL，不依赖打包器的 worker 入口） | Worker 脚本的交付方式（Q2） |
| 打包与导出 | `vite.config.ts` 多入口 lib（index / browser / node / nfsp / content / aiws_ext / app-frame / provision）；`package.json#exports` 子路径（`./browser`、`./node`、`./provision`…）；`external: [/^node:/]` | 新增 `xllm` 入口与 `./xllm` 子路径 |
| 跨仓向量先例 | `tests/fixtures/object_url_vectors.json`、`aicc_video_generate_cases.json`（Rust 生成 / TS 消费） | 一致性向量放 `tests/fixtures/xllm/`（Q9） |
| 测试宿主 | `jest`（node 环境，`tests/**/*.test.ts`）；真实浏览器 `tests/browser/real-browser/*.html + *_runner.ts + *.spec.js`，由 `vite.test-browser.config.ts` 打到 `dist-tests/`、`tests/scripts/prepare_real_browser_test.mjs` 拷进 systest 站点，页面写 `#status[data-state]` / `#result`；`scripts/test-cli-conformance.mjs`（Node / Deno 双宿主等价） | M1 用 jest；M2 加真实浏览器 runner；M4 用 conformance 模式 |
| CLI 宿主抽象 | [`cli/runtime/host.ts`](../../buckyos-websdk/cli/runtime/host.ts) `ToolHost`（Node / Deno 的文件、环境、进程、stdio 统一接口） | Node / Deno 宿主的 `RunStore` / 文件后端可以建在它上面（M4） |

### 1.3 第一个集成方今天怎么做的（本单不改，只说明替换目标）

Desktop AiWorkspace 的本地执行器是一套**手写** function_call 循环：[`executor/localExecutor.ts`](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts)（`loop()`：system/user/tools 来自服务下发的 stage brief，`MAX_ITERATIONS = 60`，工具调用回传服务 `proc.tool`，`run_program` 进 Worker）、[`chat.ts`](../src/frame/desktop/src/app/aiworkspace/executor/chat.ts)（AICC 逻辑 / 精确模型两条路 + OpenAI 开发端点；kRPC 错误包成纯 `Error`）、[`programHost.ts`](../src/frame/desktop/src/app/aiworkspace/executor/programHost.ts) + [`program.worker.ts`](../src/frame/desktop/src/app/aiworkspace/executor/program.worker.ts)（Worker 内跑 `aiws.js` 程序，`Deno` 全局用内存快照模拟）。它没有：预算枚举与 `limit_reached`、错误分类、快照 / 恢复、behavior 循环、上下文压缩、`runtime_protocol`、用量记录。这些正是 xllm 已经定下来的语义——重写一遍只会再分叉一次。

## 2. 裁决 Q1–Q10（**建议值；用户未另行裁决时按建议执行**）

| # | 问题 | 建议 | 理由 |
| --- | --- | --- | --- |
| Q1 | **TS 重写，还是 Rust → wasm** | **TS 重写**。`llm_context` 依赖 `buckyos-api`（kRPC / reqwest）、`tokio`、`fs2`，`prompt_engine` 直接读文件；`xllm.rs` 的 Run 存储、锁、进程组、Deno/SSH/Node 执行体都是宿主 OS 的事。拆出一个 wasm-able 内核的重构量大于 TS 重写，且 wasm 里还是要把 provider / 工具 / 存储桥回 JS。一致性不靠共享二进制，靠**共享向量**（Q9） | websdk 本就是 BuckyOS 的 TS SDK 且已含 AICC 类型；PRD §1 说“不规定 SDK 类型或执行引擎的实现方式” |
| Q2 | **代码位置与包形态** | `buckyos-websdk/src/xllm/`：`core/`（纯 TS，不碰 DOM / Node / Deno API；依赖只有 `aicc_client.ts` 的类型与 `krpc_client.ts` 的 `RPCError`）、`providers/`（`aicc.ts`、`openai.ts`）、`worker/`（`protocol.ts`、`worker.ts`、`host.ts`）、`hosts/`（`browser_main.ts`、`node_deno.ts`，M4）。导出子路径 `buckyos/xllm`（core + providers + Worker 宿主 API）与 `buckyos/xllm/worker`（Worker 入口脚本，独立 vite entry；同时提供 `xllmWorkerBlobUrl()` 内联形态，参照 `hash_worker.ts`，让不控制打包器的页面也能起 Worker） | Desktop 已依赖 `buckyos` 包；三种 runtime 的 SDK 定位（SDK.md） |
| Q3 | **API 形状** | **镜像 Rust 命名与 JSON 字段**：`XllmDeps { llm: LlmClient; tools: ToolManager; store: RunStore; observer?: RunObserver; tokenizer?: Tokenizer; hostPromptSections?; hostProtocol? }`；`prepareHosted(llmContext: LlmContextJson, origin: string, hostSystem: string, deps): HostedTask`、`prepare(input: TaskInput, overrides: TaskOverrides, deps)`（无文件发现：配置就是一个 JSON 对象，等价 Rust `prepare_hosted` 的 `llm_context_json`，`deny_unknown_fields` 同样严格）；`XllmRun.start(prepared, deps)` / `resume(store, runId, limits, deps)` / `execute(): Promise<RunOutcome>` / `interrupter()`；`RunRecord`（`version: 6`，字段 snake_case 与 Rust 逐一对应，TS 侧加 `runtime_descriptor.kind = "browser"`）、`LLMContextSnapshot`（v5）；`RunEvent` / `RunPhase` 同名。类型从 Rust 结构抄，**不**另设“更 TS 风格”的别名 | 同一份 run.json / 快照用同一套眼睛排查；向量可以直接比对；xllm_rust_sdk.md 的每一节都能对号入座 |
| Q4 | **浏览器是什么 runtime** | 新 kind **`browser`**：`RuntimeDescriptor { kind: "browser", runtime_id: <origin>, host: null, target: { origin, user_agent }, workdir: "/" , capabilities: { tools: <宿主注入的工具名→来源>, network: true, os: "browser" } }`；`RuntimeInfo { platform: "browser", os: "browser", shell: "", shell_kind: "", cwd: "/", hostname: location.hostname, tools: [...] }`（`{{runtime.*}}` 模板照常可用）。**没有 shell**：配置里出现 `group:bash` / `group:bash_core` / `shell` / `bash_tools`、或 `runtime.kind` 不是 `browser`，TS 侧报 `XllmError.Capability`，不静默降级；文件工具（`read_file` 等）不内置，宿主要就当具名工具注入。Rust 侧把 `browser` 加进规划名单：`RuntimeRegistry::from_config` 对 `kind: browser` 报 `Capability`（与 `container` 等同列），AgentRuntime 设计 §3 表加一行 | AgentRuntime 设计 §3 “缺少依赖报 Capability，不回退到 native”；浏览器 runtime 的长期形态（OPFS 文件、WebGPU 本地模型、扩展 API）都挂在这个 kind 下演进 |
| Q5 | **Worker 里有什么、token 在哪** | Worker **不持 token**（D21）。默认“桥接依赖”：Worker 里跑 waist 循环、提示词装配、Run 记录（内存）与**纯 JS 工具**；`LlmClient.infer`、需要身份的工具、持久化写入经 postMessage 到主线程执行（主线程持 session，调 AICC；主线程也可把工具再转给某个服务，如 AiWorkspace 的 `proc.tool`）。主线程 API：`createXllmWorker({ llm, tools, store?, onEvent? }): { start(task) / resume(run) / interrupt(reason) }`。桥协议（`worker/protocol.ts`）：↓ `start { id, task }` / `resume { id, record, snapshots }` / `interrupt { id, reason }` / `reply { id, call_id, ok, value | error }`；↑ `call { id, call_id, kind: "llm.infer" \| "tool.call" \| "tool.specs" \| "store.put" \| "store.get", payload }` / `event { id, event: RunEvent }` / `done { id, outcome, record }`。`AbortSignal` 经 `interrupt` 传到 Worker，再由 Worker 对未完成的 `call` 发 `cancel`。核心本身**不知道桥的存在**：桥只是 `XllmDeps` 的一种实现，同一 core 在主线程 / Node / Deno 直接注入本地实现 | 单点鉴权；与 AiWorkspace 现状一致（Worker 只见文件不见 token）；UI 线程不被循环与程序执行卡住 |
| Q6 | **Provider 与错误模型** | `AiccLlmClient`（`buckyos/xllm` providers）：规则同 Rust :5103–5200——`exact_model` 含 `@` → `chat.completions.create`，否则 `helper.llm_chat` + `HelperModelRequirement { tool_call: tools.length > 0, json_schema? }`；assistant 消息的 `raw` 原样回传（AICC 在其中保存 provider 状态）；身份 = 当前浏览器 session（websdk `getAiccClient()`），**不支持 `on_behalf_of`**（配置里出现即 `Capability`）。`OpenAiLlmClient`：Chat Completions（开发 / mock；CORS 由对端负责）。错误**全部**落到 `LLMComputeError` 同名枚举：AICC `RPCError.code` ∈ `AICC_POLICY_REFUSAL_CODES` → `Provider { failure: Refused(kind) }`（`quota_exhausted → QuotaExhausted` 等四个）、`retriable: true` → `Transient`、超时 → `Timeout`、HTTP 5xx / 网络 → `Transient`、4xx → `Permanent`、`context_length_exceeded` → `ContextLimit`；`classify_error` 的“疑似凭据问题 → paused”规则照搬。**不做字符串匹配**（[[feedback-error-enum-only]]）。`TimeoutLlmClient` 用 `AbortSignal.timeout` | 与 Rust 同一张错误表才能共享向量 (d)；集成方（AiWorkspace G4）需要的就是这个分类 |
| Q7 | **循环范围与分期** | **P0**（M1）：function_call 循环（原生工具批次、`max_calls_per_round = 16`、工具失败继续、解析纠错重试、`DEFAULT_MAX_CONSECUTIVE_ERRORS = 3`）、预算（`max_tool_iterations` 共用额度、wallclock `timeout`、`llm_timeout`、`max_tokens`）、六态 Outcome → `RunStatus { running, interrupted, paused, completed, failed, limit_reached }` 映射（xllm_rust_sdk.md §6 表逐行）、interrupt（`XllmInterrupter`）、快照（每次推理前 + outcome 边界）、`RunObserver` 事件、`usage.llm_requests` 累加、`json` / `json_schema` 回复契约（`OutputSpec::Json { strict, max_repairs: 1 }`）。**P1**（M3）：behavior 循环（`scan_xml_elements` + `XllmActionParser` + `XmlStepRenderer`、`<report end="true">` H4、`handover` 记录）、上下文压缩（`ByteHeuristicTokenizer` + 同一段压缩提示词、最多 3 次、`context_window` 75% 让出）、`file_model` 阶段（图片附件 `[image N]` + `<image_analysis>`）。**P2**（M4）：Node / Deno 宿主。**不做**：section 行号 / 组 / 外部组文件的多层合并（浏览器没有目录层级；单层 JSON 配置的 section 行号拼接仍要做，因为 `prepare_hosted` 也经过它）、MCP 发现、PendingTool / `allow_deferred`（xllm 本就关闭） | 第一个集成方只要 P0；behavior 是 OpenDAN 的主路径，P1 做完 TS 版才算“同一套语义” |
| Q8 | **Run 持久化与恢复** | `RunStore` 接口同 Rust（`create_run / write_record / read_record / write_snapshot / read_snapshots / lock_run / remove_run`）。P0：`MemoryRunStore` + `exportRun(runId): { record, snapshots }` / `importRun(...)`，宿主自己决定存哪里（AiWorkspace 会存到服务的 run 记录里）。P1：`IndexedDbRunStore`（库名 `xllm-runs`，同 schema），刷新页面后 `resume`；run 锁用 `navigator.locks`（没有就假设单 tab）。**Rust ↔ TS 不互相接手** run（schema 相同是为了排查与导出，`resume` 时校验 `runtime_descriptor.kind` 不符即 `NotResumable`，与 Rust 的 descriptor 核验同义） | F05 恢复是 PRD 首版能力；集成方的“关标签页 → interrupted → 续跑”依赖它 |
| Q9 | **怎样证明“同一套语义”（A19 延伸到 TS）** | 共享向量 JSON，**Rust 生成并校验、TS 消费**：(a) `prompt_assembly.json`：`{ llm_context, origin, host_system, flavor, host_prompt_sections, json }` → 期望 `prompt.system` 全文与 `prompt.runtime_protocol`（含 `xllm/2` 开场白、function_call 有 / 无 `report` 工具、无工具三种分支；behavior 分支 P1 补）；(b) `xml_behavior.json`：回复文本 → 解析出的 actions / report / 错误；(c) `loop_scripts.json`：脚本化 LLM 回复序列 + 工具结果 → 期望 `status / limit_reason / usage.llm_requests / 工具调用序列`（从 `llm_context/src/tests/{function_call_loop, budgets, error_policy, cancel}.rs` 与 `xllm.rs` 测试里挑 15–20 个）；(d) `provider_errors.json`：AICC / OpenAI 错误体 → `LLMComputeError` JSON。位置：`buckyos/src/frame/llm_context/tests/vectors/*.json`（Rust 测试 `xllm::vectors` 读取比对，`XLLM_VECTORS_REGEN=1` 重生成）→ 复制到 `buckyos-websdk/tests/fixtures/xllm/`（两仓手工同步，文件头 `vectors_version` 对齐）。补一条 Rust ↔ TS 端到端：同一个 OpenAI mock（Desktop `tests/aiworkspace/mock-llm.mjs` 的脚本化方式）分别被 `agent_tool xllm` CLI 与 TS core 驱动，比对 run.json 的 `status / usage / result.raw` | 两份实现、一张表；以后 Rust 改协议文本时向量测试会红，TS 跟着改 |
| Q10 | **为 AiWorkspace 集成预留什么（本单不做）** | 服务下发的 stage brief `{ system, user, tools, model, iterations }` 可直接映射为一个 hosted 任务：`llm_context = { provider: { type: "buckyos" }, model, loop_model: "function_call", max_tool_iterations: iterations, tools: { enabled: true, tools: tools.map(t => ({ name: t.name })) } }`，`hostSystem = system`，`host_prompt_sections = NONE`（AiWorkspace 的提示词自带约束与回复契约；`runtime_protocol = false` 只允许 function_call，正好）；stage 工具作为具名宿主工具注入（主线程转 `proc.tool`），`run_program` 是 Worker 内 JS 工具；`aiws.llm.map` 直接用 providers 的 `LlmClient.infer({ json_schema })`，不需要循环。本单只保证这些入口存在并有测试覆盖：具名宿主工具注入、`host_prompt_sections = NONE`、L1 直接推理、Worker 内工具与桥接工具并存 | 集成改动留给许愿格 TODO 的 M3（已改为依赖本单） |

## 3. 代码接入点

| 位置 | 改动 |
| --- | --- |
| `buckyos-websdk/src/xllm/core/{types,config,prompt,protocol,xml,loop,outcome,errors,run,store,observer,tokenizer}.ts` | 新建（M1 / M3） |
| `buckyos-websdk/src/xllm/providers/{aicc,openai,timeout}.ts` | 新建（M1） |
| `buckyos-websdk/src/xllm/worker/{protocol,worker,host}.ts`、`src/xllm/index.ts`、`src/xllm/worker_entry.ts` | 新建（M2） |
| `buckyos-websdk/src/xllm/hosts/node_deno.ts` | 新建（M4，基于 `cli/runtime/host.ts` 的 `ToolHost` 做文件型 `RunStore`） |
| `buckyos-websdk/vite.config.ts`、`package.json#exports` / `files`、`tsconfig.build.json` | 入口 `xllm`、`xllm_worker`；子路径 `./xllm`、`./xllm/worker`（M2） |
| `buckyos-websdk/tests/xllm/*.test.ts`、`tests/fixtures/xllm/*.json` | jest（M1 / M3） |
| `buckyos-websdk/tests/browser/real-browser/xllm_runner.ts` + `xllm.html` + `xllm.spec.js`、`vite.test-browser.config.ts`、`tests/scripts/prepare_real_browser_test.mjs`、`package.json#scripts.test:browser:real` | 真实浏览器 Worker 用例（M2） |
| `buckyos/src/frame/agent_tool/src/xllm.rs` | 新增 `mod vectors` 测试：读 / 重生成四组向量（M0）；**不改行为** |
| `buckyos/src/frame/agent_tool/src/runtime/mod.rs:87` 附近 | `"browser"` 加入规划名单 → `Capability`（M0） |
| `buckyos/src/frame/llm_context/tests/vectors/*.json` | 新建（M0） |
| `buckyos/doc/llm_context/xllm_ts_runtime.md`（新）、`xllm_rust_sdk.md`（顶部加“TS 版本对照”指针）、`AgentRuntime 设计.md` §3 表、`readme.md` “命令行工具”节 | 文档（M0 / M5） |

## 4. 里程碑

### M0 契约与向量（Rust 仓，不写 TS）

- [ ] **T0-1** `doc/llm_context/xllm_ts_runtime.md`：分层与 API（Q3）、`browser` runtime kind（Q4）、桥协议消息表（Q5）、错误映射表（Q6）、`RunStore` 接口与 IndexedDB 布局（Q8）、向量格式与同步规则（Q9）、不做清单。
- [ ] **T0-2** 四组向量 + `xllm::vectors` 测试；`loop_scripts.json` 每条注明来源测试名。
- [ ] **T0-3** `runtime/mod.rs`：`browser` → `Capability`；AgentRuntime 设计 §3 表加 `browser`（“TS 运行时实现；Rust 侧报 Capability”）；`xllm_rust_sdk.md` 顶部指针。
- 完成条件：`cargo test -p agent_tool --lib xllm` 与 `cargo test -p llm_context` 绿；向量文件已提交；`cargo test -p agent_tool --lib runtime` 含 `browser` 报 Capability 一例。

### M1 core P0（websdk，jest）

- [ ] **T1-1** `types.ts`：从 Rust 抄 `XllmError`、`ProviderConfig`、`LlmContextJson`（严格键）、`TaskOverrides`、`EffectiveConfig / EffectiveTools / RunLimits`、`PromptPlan`、`RunRecord / RunStatus / RunErrorRecord / UsageRecord`、`LLMContextSnapshot / LLMContextState`、`LLMContextOutcome`、`LLMComputeError`、`RunEvent / RunPhase`。
- [ ] **T1-2** `config.ts`：单层 `LlmContextJson` 解析（未知字段 / 类型 / 别名冲突 → `Config { file, field, reason }`）、默认值（`DEFAULT_*` 同值）、`compute_tools_config` 等价（`enabled` 默认 false、`tools: [{ name }]` 只接受具名宿主工具、出现 bash 组 / shell / `on_behalf_of` / 非 `browser` runtime → `Capability`）。
- [ ] **T1-3** `prompt.ts` + `protocol.ts`：section 行号拼接（别名表同 Rust）、`TemplateEnv`（`{{runtime.*}}` / `{{env.*}}`、`\{{` 转义、缺失变量 → `Template`）、`build_runtime_protocol_for` 全文移植（OneShot / Session 开场白；function_call 三分支；behavior 分支 M3）、`assemble_prompt` / hosted 追加段与 `HostPromptSections`。向量 (a) 全过。
- [ ] **T1-4** `loop.ts`：function_call waist（Q7 P0 清单）；`outcome.ts` 六态 → `RunStatus`；`errors.ts` 分类（`classify_error` 等价）；`run.ts`（`XllmTask.prepareHosted / prepare`、`XllmRun.start / execute / resume / interrupter`）；`store.ts`（`MemoryRunStore`、`exportRun / importRun`）；`observer.ts`。向量 (c) 全过。
- [ ] **T1-5** `providers/`：`AiccLlmClient`（Q6 规则；只在有 `getAiccClient()` 的运行时可构造）、`OpenAiLlmClient`、`TimeoutLlmClient`；`errors.ts` 的 AICC / OpenAI 映射。向量 (d) 全过。
- [ ] **T1-6** jest：`tests/xllm/{config,prompt,loop,providers,resume}.test.ts`；一个 `ScriptedLlmClient`（对照 Rust wishkit 的 `ScriptedModel`）；`resume` 用 `exportRun` → 新 `MemoryRunStore` → `importRun` → 续跑到完成。
- 完成条件：`pnpm test -- tests/xllm` 绿；`pnpm types` 过；core 目录 `grep -rn "window\.\|document\.\|process\.\|Deno\." src/xllm/core src/xllm/providers` 为空。

### M2 Worker 宿主（浏览器）

- [ ] **T2-1** `worker/protocol.ts`（Q5 消息表，带 `protocol_version: 1`）、`worker/worker.ts`（Worker 侧：桥接 `LlmClient` / `ToolManager` / `RunStore` 实现 + 直通 `MemoryRunStore` 与纯 JS 工具注册 `registerWorkerTool(name, spec, fn)`）、`worker/host.ts`（主线程：`createXllmWorker()`，并发 run 用 `id` 区分，`terminate()`）。
- [ ] **T2-2** 打包：`src/xllm/index.ts`、`src/xllm/worker_entry.ts` 两个入口；`exports` 子路径；`xllmWorkerBlobUrl()` 内联形态（构建时把 worker bundle 文本注入，参照 `hash_worker.ts` 的做法）。
- [ ] **T2-3** 真实浏览器用例 `xllm_runner.ts`：页面内起 Worker，`llm` 是页面侧的 `ScriptedLlmClient`，工具一个在页面（`echo`）一个在 Worker（`sum`），跑向量 (c) 中的 3 条 + interrupt 一条 + `exportRun` 后 `resume` 一条；结果写 `#result`；`xllm.spec.js` 断言。
- [ ] **T2-4** 真实 Zone DV（手动，一次）：在 `systest.test.buckyos.io` 页面用 websdk 登录 devtest 后，`AiccLlmClient` + Worker 跑“读一段材料回答 + 调一次 `echo` 工具”的 function_call 任务到 `completed`；AICC 用量表 `user_id = devtest`、`app` 为该页面身份；记录 run.json 片段到 §8。
- 完成条件：`pnpm run test:browser:real` 含 `xllm.spec.js` 绿；DV 记录在 §8。

### M3 core P1

- [ ] **T3-1** behavior 循环：`xml.ts`（`scan_xml_elements` / `xml_unescape` / `strip_code_fences`）、`XllmActionParser`（属性 → 参数、正文 / CDATA → 正文参数、“无动作 + `<report end="true">` 即完成”、阶段 report 不结束）、`XmlStepRenderer`（hosted 无时间戳形态）、`handover` 记录；向量 (b) 与 (c) 的 behavior 条目。
- [ ] **T3-2** 上下文压缩与 `context_window`、`file_model` 阶段（图片 `AiContent` 走 `AiccLlmClient`）。
- [ ] **T3-3** `IndexedDbRunStore` + `navigator.locks`；真实浏览器用例加“start → `location.reload()` → resume 完成”。
- 完成条件：jest + real-browser 绿；`xllm_rust_sdk.md` §4 / §6 的每条在 TS 测试里有对应用例名（写进 `xllm_ts_runtime.md` 的对照表）。

### M4 Node / Deno 宿主

- [ ] **T4-1** `hosts/node_deno.ts`：文件型 `RunStore`（`<runs_dir>/<run_id>/run.json` + `snapshots/NNNN.json`，布局同 Rust §5，用 `ToolHost` 的文件 API）；`deno check --config cli/deno.json` 覆盖 `src/xllm/core`。
- [ ] **T4-2** Rust ↔ TS 端到端（Q9 末条）：`scripts/test-xllm-conformance.mjs` 起 OpenAI mock，分别跑 `agent_tool xllm --no-tools "…"`（需本机 Rust 二进制，缺则 skip 并打印原因）与 TS core（Node），比对 `status / usage.llm_requests / result.raw`。
- 完成条件：`pnpm run check:cli` 绿；conformance 脚本在有 Rust 二进制的机器上通过。

### M5 文档与回写

- [ ] `xllm_ts_runtime.md` 写实施差异与对照表；`llm_context/readme.md` 加“TS 运行时”一节；PRD 不改产品行为，§10 末尾加一行指向 TS 文档。
- [ ] [许愿格浏览器优先 TODO](./aiworkspace-wish-browser-first-todo.md) §0 状态行改“依赖已满足，可开工 M3”。
- [ ] 本文件状态行改“已实施”，memory 记录。

## 5. 验证命令

```bash
# Rust 向量与 Capability（在 buckyos/src）
cargo test -p agent_tool --lib xllm
cargo test -p agent_tool --lib runtime
cargo test -p llm_context
XLLM_VECTORS_REGEN=1 cargo test -p agent_tool --lib xllm::vectors   # 协议文本有意变更时重生成，再 diff 向量

# websdk（在 buckyos-websdk）
pnpm test -- tests/xllm
pnpm types && pnpm build
pnpm run test:browser:real          # 含 xllm.spec.js；需要本机 systest 站点（见 tests/scripts/prepare_real_browser_test.mjs）
pnpm run check:cli                  # M4
node scripts/test-xllm-conformance.mjs   # M4，需 /tmp/dev-cache-root/cargo-target/debug/agent_tool

# 零环境依赖检查
grep -rn "window\.\|document\.\|process\.\|Deno\.\|navigator\." src/xllm/core src/xllm/providers   # 应为空
```

坑（来自 [[xllm-rust-sdk-impl]] 与 [[wish-impl]]）：本 harness 的 stdin 是 socket，跑 Rust CLI 时显式 `</dev/null`；`pkill -f` 会匹配到自身，用 `pkill -f "[o]penai_mock"` 形式；e2e 的脚本化模型靠 system 文本片段分辨阶段，向量 (c) 的脚本要按**调用序号**而不是文本匹配回复；Rust 的 `serde_json` 不保序，向量里对象键按字典序写，TS 比对前先 canonical 化。

## 6. 不在本单

- AiWorkspace 的集成（替换 `localExecutor.ts` / `chat.ts`、`llm.map` 走 L1、`run_program` 作 Worker 工具）——[许愿格浏览器优先 TODO](./aiworkspace-wish-browser-first-todo.md) M3。
- Rust xllm 的任何行为改动（本单只加向量测试与 `browser` 的 Capability）。
- shell / 文件工具的浏览器执行体（OPFS、WebContainer 之类）、MCP 发现、WebGPU 本地模型；都在 `browser` kind 下以后再议。
- Agent Session（libopendan）语义：session_input、behavior 切换状态机、`call_behavior`、deferred task。
- Rust ↔ TS 跨实现 resume；`.llm_context` 多层目录合并与外部组文件。
- 一个 TS 的 `xllm` 命令行（PRD 的 CLI 由 Rust 交付；Node / Deno 宿主只为测试与 websdk CLI 内部复用）。

## 7. 对许愿格 TODO 的影响（已同步改写）

- 许愿格 TODO §0 状态加“M3 依赖本单”；Q1（`llm.map` 谁调模型）的实现载体改为本单 providers 的 L1 推理；G4 / G5（错误分类、预算写死）由本单的错误枚举与 `RunLimits` 解决；M3 的 T3-1 / T3-2 改为“接入 `buckyos/xllm`”，手写循环与 `chat.ts` 删除。
- 许愿格 TODO 的 M0 / M1 / M2（服务端：默认本地、`llm_cache`、下线服务内执行）与 M4 的服务端测试迁移不依赖本单，可并行。

## 8. 实施回写（code agent 填写）

- 出入 / 自定裁决：
- 向量数量与来源：
- M2 T2-4 DV 记录：
- 遗留：
