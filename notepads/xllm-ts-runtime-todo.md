# xllm TypeScript 运行时 TODO：同一套任务语义进浏览器（先在页面里跑通，独立验证后再被 AiWorkspace 等集成）

日期：2026-10-31

状态：**待开工。** 2026-11-07 用户定了实施顺序（§4）：第一步在浏览器里跑通 agent loop，开发者照示例自己构造 runtime（M1）；第二步完成基于 Deno、与 Rust 兼容的 TS xllm（M2–M4）；第三步基于 DID Object 协议和 RESTful API，让现有页面简单改造就得到标准 runtime，xllm 代用户操作当前页面（M5，§2.1 的基础工具从 M1 移到这里）；Worker 宿主移到第三步之后（M6），许愿格集成随之后移。同日用户补充：第三步讨论的是面向 Human-Agent loop 的理想页面形态，不模拟 DOM 点击；验收不要太复杂，不用 HomeStation。里程碑和任务编号随之重排，下面 11-06 记录里的编号是当时的。2026-11-06 用户逐条回复了 §2：Q1–Q13 全部确认，其中 Q4 改了方向（SDK 不提供通用的浏览器 runtime，由各网站自己构建；可以试一套基础工具，方案见 §2.1，11-07 用户补充了其中“RPC 命令化”的含义；kind 因此从 `browser` 改名 `site`），§2 末尾三项也已裁决（Rust 落点见 T2-0）。同日用户裁决：第一步是在浏览器里跑起 xllm loop，并示范如何在浏览器里构造一个 runtime（M1）；浏览器的能力是 Deno 的子集，这一步 xllm 的能力不超过 HTML 内嵌 JS 的能力（§0）；修 AiWorkspace 的程序 Worker 并入本单（MW）。里程碑因此重排：原 M0 + 原 M1 的余下部分 → M2，原 M2 → M3，原 M3 → M4，原 M4 → M5，原 M5 → M6；同日按源码修正了 Q2–Q10 和 §1 里已过期的事实。2026-11-01 按 [llm_context 冻结 TODO](./done/llm-context-freeze-todo.md) 的落地结果回改了 L2 行与 Q1 / Q3 / Q7 / Q8 / Q9 (e) / T4-2 / T4-3 (ii)（waist 不再有挂起原语、单一 `CheckpointHook`、快照 v6、RunRecord v7 无 `inflight[]`、等待预算 `task_wait_ms`）。2026-10-31 用户裁决：xllm 是整个 Agent 体系里的独立工具，后端是命令行 / SDK，浏览器环境下应是一个本地 JS Worker；这件事非常通用（按 xllm 的抽象，还涉及浏览器下如何构造 runtime 的长期架构），**从 [许愿格浏览器优先 TODO](./aiworkspace-wish-browser-first-todo.md) 中抽出来独立做、独立验证，AiWorkspace 之后再集成**。

依据：

- [xllm PRD](<../product/xllm/PRD.md>) §1（“这一层长期独立存在，既供 shell 脚本使用，也供上层 Agent 调用”；“首版交付独立的 `xllm` 命令，并由同一 SDK 提供其核心能力”）、A19（CLI 与 SDK 一致）、§8.3 / §8.4（独立处理的工作、与后续 Agent 工具的关系）。
- [xllm Rust SDK 参考](<../doc/llm_context/xllm_rust_sdk.md>)（“本文只记录 Rust 实现落实 PRD 时固定下来的协议决定，**供 websdk 的 TS 版本对照**”）、§10 宿主装配（`prepare_hosted`：配置以 JSON 传入、宿主给 system、xllm 追加 `capabilities / cmd_manual / runtime_protocol`）。
- [LLM Context 设计](<../doc/llm_context/LLM Context 设计.md>) §0（narrow waist 纪律）、§2 四层（L1 provider / L2 waist / L3 调度 / L4 语义层）、§2.1（LLMContext 是 lib 不是 service，承载方式由调度方选）。
- [AgentRuntime 设计](<../doc/llm_context/AgentRuntime 设计.md>) §1 / §3（runtime kind 表；规划中的名称报 `Capability`，不回退到 native）。
- [AiWorkspace 核心架构设计冻结思路](<../doc/workspace/BuckyOS AiWorkspace 核心架构设计冻结思路.md>) §8.6（标准许愿格在当前用户的网页中运行，后端不参与 AI 执行）与 [内核架构改进](<../doc/workspace/BuckyOS AiWorkspace 内核架构改进.md>) D21（Worker + postMessage，不用 iframe；其“Worker 没有 cookie 访问”一句实测不成立，见 MW）——这是第一个集成方，但除 MW 外本单不做集成。
- memory [[xllm-rust-sdk-impl]]：2026-09-18 Rust 重写时已写明“下一步是在 websdk 里做 TS 版本，对照 xllm_rust_sdk.md 保持 CLI/SDK 语义一致（PRD A19）”。

目标：在 `buckyos-websdk` 里交付 **xllm 的 TypeScript 运行时**——与 Rust 实现同一套任务语义（配置 → 有效配置 → 提示词装配 → 执行循环 → 预算 / 六态结束 → Run 记录与恢复 → 观察事件），核心只用 Web 标准 API，同一份代码在浏览器页面、浏览器 Web Worker、Node / Deno 上运行；SDK 不提供通用的浏览器 runtime：runtime 由各网站自己构建（新 kind `site`，没有 shell，能力由页面代码声明，§0、Q4），SDK 给出构造方法和一套可选的基础工具（§2.1）；模型访问和工具都来自页面，Worker 形态下经 postMessage 桥到主线程；Deno 宿主实现 `native` kind，并且**能接手同机、同工作目录下 Rust 执行到一半的 run 目录**（能否接手只看快照协议、runtime descriptor、工具能力与锁是否兼容，不看实现语言）；Rust 与 TS 用同一组向量（JSON fixtures）证明一致。验证通过后，AiWorkspace 的本地执行器、以及任何需要“在用户的浏览器里跑一个有界 LLM 任务”的前端功能都改为用它，不再各自手写循环。

## 0. 能力边界（2026-11-06 用户裁决）

1. **上限：不超过页面内嵌 JS。** 浏览器里的 xllm 就是页面引入的一个 JS 库。判据：把 xllm 去掉，页面自己的 JS 做不到的事，浏览器 runtime 就不能提供。所以没有新的服务端接口、特权 token、浏览器扩展 API，也不支持 `on_behalf_of`；工具是页面写的 JS 函数，模型调用用页面自己的 session。SDK 也不提供 DOM、存储、网络这类通用工具：纯浏览器的 runtime 意义不大，能做什么由每个网站决定（Q4）。
2. **上限不等于授权。** 页面 JS 持有用户 session，而 loop 由模型驱动，输入里可能有提示词注入。所以实际能力由场景声明收窄：页面代码写明这个场景有哪些工具（`createSiteRuntime`，Q4），用基础工具时是哪些资源、action 和命令（§2.1）。配置、模板、导入的 run 只能进一步收窄，不能放大。这里不沿用冻结 TODO F13 的“预设只是配置”：Rust 上配置作者就是机器主人，web 上配置可能来自别人分享的模板。
3. **浏览器是 Deno 的子集。** `site` runtime 的能力（计算、网络、页面注入的工具）⊂ Deno `native` 的能力（再加文件、进程、flock）。core 和 `site` runtime 只用两边都有的 Web 标准 API，同一份代码不改就能在 Deno 上跑和测；Deno 宿主在此基础上加能力，不另写一套。子集说的是 runtime 的能力，不是每个 API：实测 Deno 2.9.8 有 `fetch`、`AbortController`、`TextEncoder`、`crypto.subtle`、`structuredClone`、`Worker`、`MessageChannel`、`caches`、`navigator.locks`，没有 `document`、`indexedDB`、`navigator.storage`。后三者只能出现在页面注入的工具或 `hosts/browser/` 里，不进 core。
4. **模型生成的代码不算页面 JS，权限要比页面更低。** loop 和 runtime 从不执行模型输出（没有 `eval`、`new Function`，不动态 `import` 模型内容）。要执行就走 MW 的沙箱。同源 Worker 不是隔离边界（MW 的实测），循环 Worker（M6）的作用只是不卡 UI，不算安全措施。

## 1. 现状（2026-10-31 源码核对，2026-11-06 修正版本与符号；文中行号已偏移，按符号名搜索）

### 1.1 Rust 侧：要对照移植的东西在哪

| 层 | 位置 | 内容 | 对 TS 的含义 |
| --- | --- | --- | --- |
| L2 waist | [`src/frame/llm_context/`](../src/frame/llm_context/src/)：`deps.rs`（`LlmClient` / `ToolManager` / `PolicyEngine` / `WorklogSink` / `Tokenizer` / 唯一的 `CheckpointHook`（`before_tool_call` / `before_inference` → `Injection` / `validate_report` / `review_decision`）/ `LLMContextDeps`）、`context_loop.rs`（主循环，批次内串行派发、每个调用前一个 checkpoint）、`behavior_loop.rs`（behavior 模式、`LLMResultParser` / `StepRenderer` trait、End / Handover / Yield 三种 step 形状）、`state.rs`（快照 v6 `LLMContextSnapshot { request, state }`，`Suspension` 只剩上下文超限）、`outcome.rs`（六态 `Done / BudgetExhausted / Error / ContextLimitReached / Interrupted / Settled`）、`error.rs`（`LLMComputeError`、`ProviderFailure { Transient, Permanent, Unknown, ContextLimit, Refused { code: String } }`）、`request.rs`（`LLMContextRequest`、`owner: Value`、`ToolPolicy.max_calls_per_round`、`BudgetSpec`、`ErrorPolicy`）。`XmlStepRenderer`、`prompt_engine`、`xml_util`、任务模型（`task_model`）在 agent_tool，`msg_parser` 在 libopendan | crate 依赖 `buckyos-api`（`AiMessage` 等类型）、`tokio`；测试 `src/tests/`（function_call_loop 12、behavior_loop 18、budgets 12、error_policy 15、cancel 10、reply_contract 5、thinking 3） | **不能直接编 wasm**（见 Q1）；但测试用例是现成的向量来源 |
| L4 oneshot / 宿主装配 | [`src/frame/agent_tool/src/xllm.rs`](../src/frame/agent_tool/src/xllm.rs)（约 13800 行，76 个测试）：常量（:118–176，`RUNTIME_PROTOCOL_VERSION = "xllm/2"`、`RUN_RECORD_VERSION = 7`、`DEFAULT_MAX_TOOL_ITERATIONS = 8`、超时 3600 / 600、`MAX_TEXT/IMAGE_ATTACHMENT_BYTES`、`SECTION_ALIASES`）、`XllmError`（:225）、配置解析与合并（`parse_llm_context_file` :1468、`merge_config_layers` :1786、`TaskOverrides` :1942、`EffectiveConfig` / `EffectiveTools` / `RunLimits` :2226–2296）、模板与提示词（`TemplateEnv` :2436、`PromptPlan` :2566、`build_runtime_protocol_for` :2731、`assemble_prompt` :2934）、XML（`scan_xml_elements` :3199、`XllmActionParser` :3426）、事件（`RunPhase` / `RunEvent` / `RunObserver` :3700–3751）、`XllmDeps`（:3875）、`XllmToolManager`（:4025）、Provider（`AiccLlmClient` :5263：`exact_model` 含 `@` 走 `chat.completions.create`，否则 `helper.llm_chat` + `HelperModelRequirement`；`OpenAiLlmClient` :5553；`TimeoutLlmClient` :5854；`aicc_provider_failure`、`provider_error_from_rpc` :5506）、Run 记录（`RunRecord` :6013、`RunStore::{Disk, Memory}` :6270）、入口（`XllmTask::prepare` :6991、`prepare_hosted` :7500、`rebuild_toolset` :7789、`hosted_request` :7944（`max_calls_per_round: 16`）、`hosted_waist_deps` :8041、`XllmRun::start` :8333 / `resume` :8461、`build_waist_deps` :8296、`classify_error` :8725） | 文件 / 进程 / 锁用 `std::fs`、`fs2`、进程组 | TS 的命名、字段名（snake_case JSON）、状态机、协议文本都以此为准 |
| 压缩与 hint | [`llm_compress.rs`](../src/frame/agent_tool/src/llm_compress.rs)（1253 行：按轮分段、头尾预算、分块滚动折叠、`keep_exact`；经 AICC 时摘要模型是 `llm.summarize`，见 `EffectiveConfig::summary_model`）、[`hints.rs`](../src/frame/agent_tool/src/hints.rs)（`HintLedger`、`once` / `dedup`、`<runtime_hints>` 渲染）；`RunRecord.hints_once` | 2026-11-02、11-03 才落地，`xllm_rust_sdk.md` 部分没跟上，以代码为准 | hint 在 M2 移植，压缩在 M3 移植 |
| Runtime | [`src/frame/agent_tool/src/runtime/mod.rs`](../src/frame/agent_tool/src/runtime/mod.rs)：`AgentRuntime` trait（`descriptor / config / info / open / describe_interrupted`）、`Sandbox: ToolManager`、`RuntimeDescriptor { runtime_id, kind, host, target, workdir, capabilities }`、`RuntimeInfo`（`platform / shell_kind / baseline`）、`Capabilities { tools, network, os }`（`tools` 是探测到的命令，native 上是 `bash / realpath / base64`，不是模型可用的工具）；kind：`native / remote_ssh / remote_node`；`container / container_host / http_proxy_runtime` 为规划名，`from_config` 报 `Capability`（:113） | — | `site` 要成为新的规划 kind（Q4） |
| 对象与 action | [`doc/key url.md`](<../doc/key url.md>) §1、[`read_tool.rs`](../src/frame/agent_tool/src/read_tool.rs)、[`agent-did-object-lib`](../src/frame/agent-did-object-lib/src/runtime.rs)、[`ActionRisk`](../src/kernel/buckyos-api/src/did_object_provider.rs)、[协议 §9](<../doc/opendan/protocol/Agent DID-Object Protocol Spec.md>) | `read` 读 DID Object 时列出属性、`Actions:` 和 `risk`；`xcall(object, action, params)` 调用 action | 基础工具沿用这些名字和语义（§2.1） |
| 命令 | websdk [`cli/core/command.ts`](../../buckyos-websdk/cli/core/command.ts)（`CommandDefinition`、`AccessLevel`）、[`argv.ts`](../../buckyos-websdk/cli/core/argv.ts)（`parseShellLine`、`parseCommandArgs`、`validateSchema`）、[`registry.ts`](../../buckyos-websdk/cli/core/registry.ts)（`describe`）；Rust [`agent_tool_cli_dev`](../src/frame/agent_tool_cli_dev/src/lib.rs) 的 `XCALL_USAGE` | Rust agent 的 `xcall` 是在 `shell` 里跑的命令。CLI 的解析函数是纯 TS，但 websdk 构建的 `rootDir` 是 `src`，`src/` 不能直接引用 `cli/`；CLI 经 `buckyos/node` 引用 SDK | `exec` 的声明和解析沿用这些，T5-1 把纯函数挪进 `src/internal/`（§2.1） |
| 文档 | `doc/llm_context/xllm_rust_sdk.md`、`LLM Context 设计.md`、`AgentRuntime 设计.md`、`llm_context_behavior_loop.md`、`LLM Compress.md`、`readme.md` | — | 新增 `xllm_ts_runtime.md`（M1 首版，M2 补全） |

### 1.2 websdk 侧：能复用什么

| 已有 | 位置 | 用法 |
| --- | --- | --- |
| AICC 类型与客户端 | [`src/aicc_client.ts`](../../buckyos-websdk/src/aicc_client.ts)：`AiMessage / AiContent / AiToolSpec / AiToolCall / AiUsage / AiccError / AiccErrorCode`、`AICC_POLICY_REFUSAL_CODES`、`aiccRefusalOf()`、`AiccClient.chatCompletionsCreate()`、`helper.llm_chat` | TS `AiccLlmClient` 直接用；错误分类读 `RPCError.code` / `data`（[`krpc_client.ts:46-55`](../../buckyos-websdk/src/krpc_client.ts)），注意 Q6 的两个坑 |
| Worker 先例 | [`src/internal/hash_worker.ts`](../../buckyos-websdk/src/internal/hash_worker.ts)（`new Worker(getWorkerBlobUrl())`，Worker 源码内联为 Blob URL，不依赖打包器的 worker 入口） | Worker 脚本的交付方式（Q2） |
| 打包与导出 | `vite.config.ts` 多入口 lib（index / browser / node / nfsp / content / aiws_ext / app-frame / provision）；`package.json#exports` 子路径（`./browser`、`./node`、`./provision`…）；`external: [/^node:/]` | 新增 `xllm` 入口与 `./xllm` 子路径 |
| 跨仓向量先例 | `tests/fixtures/object_url_vectors.json`、`aicc_video_generate_cases.json`（Rust 生成 / TS 消费） | 一致性向量放 `tests/fixtures/xllm/`（Q9） |
| 测试宿主 | `jest`（node 环境，`tests/**/*.test.ts`）；共享用例 `tests/ndm_client_cases.ts`（同一组用例在 jest 和真实浏览器里跑）；真实浏览器 `tests/browser/real-browser/*.html + *_runner.ts + *.spec.js`，由 `vite.test-browser.config.ts` 打到 `dist-tests/`、`tests/scripts/prepare_real_browser_test.mjs` 拷进 systest 站点（要 Zone），页面写 `#status[data-state]` / `#result`；`scripts/test-cli-conformance.mjs`（Node / Deno 双宿主等价） | M1 用共享用例跑 jest、Deno 和不依赖 Zone 的静态页；M4 用 conformance 模式 |
| CLI 宿主抽象 | [`cli/runtime/host.ts`](../../buckyos-websdk/cli/runtime/host.ts) `ToolHost`（Node / Deno 的文件、环境、进程、stdio 统一接口） | Deno 宿主的文件后端可参照其路径策略与 `Deno.Command` 用法（M4）；flock 它没有，`FileRunStore` 直接用 `Deno.FsFile.lock` |

### 1.3 第一个集成方今天怎么做的（除 MW 外本单不改，只说明替换目标）

Desktop AiWorkspace 的本地执行器是一套**手写** function_call 循环：[`executor/localExecutor.ts`](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts)（`loop()`：system/user/tools 来自服务下发的 stage brief，`MAX_ITERATIONS = 60`，工具调用回传服务 `proc.tool`，`run_program` 进 Worker）、[`chat.ts`](../src/frame/desktop/src/app/aiworkspace/executor/chat.ts)（AICC 逻辑 / 精确模型两条路 + OpenAI 开发端点；kRPC 错误包成纯 `Error`）、[`programHost.ts`](../src/frame/desktop/src/app/aiworkspace/executor/programHost.ts) + [`program.worker.ts`](../src/frame/desktop/src/app/aiworkspace/executor/program.worker.ts)（Worker 内跑 `aiws.js` 程序，`Deno` 全局用内存快照模拟；这个 Worker 和页面同源，不是隔离边界，见 MW）。它没有：预算枚举与 `limit_reached`、错误分类、快照 / 恢复、behavior 循环、上下文压缩、`runtime_protocol`、用量记录。这些正是 xllm 已经定下来的语义——重写一遍只会再分叉一次。

## 2. 裁决 Q1–Q13（**2026-11-06 用户已逐条确认**；Q4 按用户意见改写，Q8、Q10 有补充）

| # | 问题 | 建议 | 理由 |
| --- | --- | --- | --- |
| Q1 | **TS 重写，还是 Rust → wasm** | **TS 重写**。`llm_context` 依赖 `buckyos-api`、`tokio`（冻结后 waist 只剩 `AiMessage` 等类型依赖，`prompt_engine` 已搬到 agent_tool，但仍不值得单独编 wasm）；`xllm.rs` 的 Run 存储、锁、进程组、Deno/SSH/Node 执行体都是宿主 OS 的事。拆出一个 wasm-able 内核的重构量大于 TS 重写，且 wasm 里还是要把 provider / 工具 / 存储桥回 JS。一致性不靠共享二进制，靠**共享向量**（Q9） | websdk 本就是 BuckyOS 的 TS SDK 且已含 AICC 类型；PRD §1 说“不规定 SDK 类型或执行引擎的实现方式” |
| Q2 | **代码位置与包形态** | `buckyos-websdk/src/xllm/`：`core/`（waist 与 L4 的纯逻辑，只用 Web 标准 API；依赖只有 `aicc_client.ts` 的类型与 `krpc_client.ts` 的 `RPCError`）、`runtime/`（`AgentRuntime` 接口、`site.ts`、`site_tools.ts`、工具派发、任务模型）、`providers/`（`aicc.ts`、`timeout.ts`）、`dev/`（`openai.ts`、`scripted.ts`，开发和测试用）、`worker/`（`protocol.ts`、`worker.ts`、`host.ts`，M6）、`hosts/browser/`（`IndexedDbRunStore`，M5）、`hosts/deno/`（M4）。导出子路径：`buckyos/xllm`（core + `createSiteRuntime` + `siteTools` + AICC provider，给页面用）、`buckyos/xllm/dev`、`buckyos/xllm/worker`（M6，Worker 入口脚本，独立 vite entry；同时提供 `xllmWorkerBlobUrl()` 内联形态，参照 `hash_worker.ts`，让不控制打包器的页面也能起 Worker）、`buckyos/xllm/deno`（M4）。**浏览器入口的包里不含 shell、native、Deno 的代码**：结构上没有，比运行时拒绝可靠，构建后用 grep 检查（§5） | Desktop 已依赖 `buckyos` 包；三种 runtime 的 SDK 定位（SDK.md）；§0 第 1、3 条 |
| Q3 | **API 形状** | **镜像 Rust 命名与 JSON 字段**：`XllmDeps { llm: LlmClient; runtime: AgentRuntime; store: RunStore; tasks?: RunningTaskResolver; observer?: RunObserver; tokenizer?: Tokenizer; hostPromptSections?; hostProtocol? }`（工具由 `runtime.open()` 给出，同 Rust；`AgentRuntime` 接口是 `descriptor() / config() / info() / open(ctx, toolsCfg, deps) / describeInterrupted(run, call)`；`tasks` 是宿主注入的任务后端，属 runtime 层，不进 core；调用内等待预算是 `RuntimeConfig.task_wait_ms`）；`prepareHosted(llmContext: LlmContextJson, origin: string, hostSystem: string, deps): HostedTask`、`prepare(input: TaskInput, overrides: TaskOverrides, deps)`（无文件发现：配置就是一个 JSON 对象，等价 Rust `prepare_hosted` 的 `llm_context_json`，`deny_unknown_fields` 同样严格）；`XllmRun.start(prepared, deps)` / `resume(store, runId, limits, deps)` / `injectInput(text)`（`paused` 的 run 注入一条新输入再续跑，同 Rust `XllmRun::inject_input` / `xllm --resume "<message>"`）/ `execute(): Promise<RunOutcome>` / `interrupter()`；`RunRecord`（`version: 7`，没有 `inflight[]`，含 `hints_once`，字段 snake_case 与 Rust 逐一对应，TS 侧加 `runtime_descriptor.kind = "site"`）、`LLMContextSnapshot`（v6）；`RunEvent` / `RunPhase` 同名。类型从 Rust 结构抄，**不**另设“更 TS 风格”的别名 | 同一份 run.json / 快照用同一套眼睛排查；向量可以直接比对；xllm_rust_sdk.md 的每一节都能对号入座 |
| Q4 | **runtime 由谁构建；场景怎么表达** | **SDK 不提供通用的浏览器 runtime，由各网站自己构建。** SDK 只给两层，都由页面代码主动调用：(1) `createSiteRuntime({ scene, origin, hostname, tools, env? })` 把页面给的工具包成 `AgentRuntime`，本身不带任何工具；`AgentRuntime` 接口公开导出，网站也可以直接实现它（AiWorkspace 预计如此，Q10）。(2) 可选的基础工具 `siteTools({ resources, commands })`（§2.1，第三步 M5），返回值就是 `tools`，core 不知道它的存在。kind 只有一个 **`site`**，场景不是新 kind：`scene` 是场景名（如 `aiws.wish`）；`tools` 是这个场景的全部工具（名字 → `{ spec, call(args, ctx), idempotent?, describeInterrupted?(args, ctx) }`，实现就是页面的 JS 函数，`ctx = { run_id, call_id, signal }`，`idempotent` 可以是布尔值或 `(args) => boolean`；这几项是 runtime 不退化成工具清单的依据，见 M1 开头的表）。descriptor：`{ kind: "site", runtime_id: "site:<scene>", host: null, target: { origin, scene }, workdir: "/", capabilities: { tools: {}, network: true, os: "browser" } }`。`target` **不放 `user_agent`**，`capabilities.tools` **留空**：恢复要求 descriptor 逐字段相等，放了这两项，浏览器一升级或页面加一个工具，旧 run 就恢复不了。反过来，`scene` 进 `target` 后，现有的相等检查就能拦住 A 场景的 run 到 B 场景恢复，不需要新机制。`RuntimeInfo { platform: "browser", os: "browser", shell: "", shell_kind: "", cwd: "/", hostname, tools: [] }`（`{{runtime.*}}` 模板照常可用；`origin`、`hostname` 由页面传入，runtime 不读 `location`，所以能在 Deno 上测）。**能力只来自页面代码，配置只能收窄**（§0）：配置里的 `tools` 只能从页面给的工具里选；`group:bash` / `group:bash_core` / `shell` / `bash_tools` / MCP、`runtime.kind` 不是 `site`、`provider.type` 不是 `buckyos`、`provider` 里的 `base_url` / `api_key` / `session_token`、`on_behalf_of` 都报 `XllmError.Capability`，不静默降级；`{{env.*}}` 只读页面显式传入的 `env`。这些检查放在 `prepare` 里按 `descriptor.kind` 判，不放进 `createSiteRuntime`，网站自建的 `AgentRuntime` 也走同一道门。这些限制只属于 `site` kind：Deno 宿主实现的是 **`native`** kind（bash 组文件工具 + `shell`，M4），descriptor 与 Rust native 同规则。Rust 侧把 `site` 加进规划名单：`RuntimeRegistry::from_config` 对 `kind: site` 报 `Capability`（与 `container` 等同列），AgentRuntime 设计 §3 表加一行 | 用户裁决：“纯浏览器意义不大，这个应该是各个网站自己构建”。AgentRuntime 设计 §3 “缺少依赖报 Capability，不回退到 native”。OPFS、WebGPU 本地模型这类能力不作为 SDK 的规划，网站要用就自己写成工具，同样要过 §0 的判据 |
| Q5 | **Worker 里有什么、token 在哪（M6）** | Worker 形态不在三步之内，放到第三步之后（§4，2026-11-07），目的是不让 loop 卡 UI，**不是安全边界**：它和页面同源，`fetch` 会自动带 cookie（MW 实测）。所以循环 Worker 只跑可信的 SDK 代码和页面自己注册的 JS 工具，**模型生成的代码不进这个 Worker**。默认“桥接依赖”：Worker 里跑 waist 循环、提示词装配、Run 记录（内存）与纯 JS 工具；`LlmClient.infer`、需要身份的工具、持久化写入经 postMessage 到主线程执行（主线程持 session，调 AICC；主线程也可把工具再转给某个服务，如 AiWorkspace 的 `proc.tool`）。主线程对每个 `call` 校验：`kind` 在协议表内，`tool.call` 的名字在场景白名单内。主线程 API：`createXllmWorker({ llm, runtime, store?, onEvent? }): { start(task) / resume(run) / interrupt(reason) }`。桥协议（`worker/protocol.ts`）：↓ `start { id, task }` / `resume { id, record, snapshots }` / `interrupt { id, reason }` / `reply { id, call_id, ok, value | error }`；↑ `call { id, call_id, kind: "llm.infer" \| "tool.call" \| "tool.specs" \| "store.put" \| "store.get", payload }` / `event { id, event: RunEvent }` / `done { id, outcome, record }`。`AbortSignal` 经 `interrupt` 传到 Worker，再由 Worker 对未完成的 `call` 发 `cancel`。核心本身**不知道桥的存在**：桥只是 `XllmDeps` 的一种实现，同一 core 在主线程 / Node / Deno 直接注入本地实现 | 单点鉴权；UI 线程不被循环卡住 |
| Q6 | **Provider 与错误模型** | `AiccLlmClient`：`new AiccLlmClient(client)`，`client` 是页面传入的 websdk `getAiccClient()`，身份就是页面当前的 session（§0），xllm 不碰 session。规则同 Rust `infer_with_client`：`exact_model` 含 `@` → `chat.completions.create`，否则 `helper.llm_chat` + `HelperModelRequirement { tool_call: allow_tool_calls && tools.length > 0, json_schema: force_json }`；`force_json` 没有 schema 时也走 `json_schema`，用宽松 schema（`any_json_object_schema`，`strict: false`，AICC 已没有 `json_object`）；assistant 消息的内容块原样回传（`AiMessage` 没有 `raw` 字段，provider 状态在 `thinking` / `provider_state` 块里）；`status: running` → `Permanent`（不轮询异步任务）。`OpenAiLlmClient` 在 `buckyos/xllm/dev`，只能由页面代码显式注入（开发 / mock；CORS 由对端负责），配置不能选。错误**全部**落到 `LLMComputeError` 同名枚举，表与 Rust `aicc_provider_failure` / `provider_error_from_rpc` 一致，按顺序判：`quota_exhausted` / `quota_budget_exceeded` / `resource_not_permitted` / `attribution_unavailable` → `Provider { failure: Refused { code } }`；`quota_unavailable` → `Transient`（websdk 的 `AICC_POLICY_REFUSAL_CODES` 含它，不能直接拿这个常量判 Refused）；`context_length_exceeded` 或 `details.http_status == 413` → `ContextLimit`；`timeout` 或 `retriable: true` → `Transient`；kRPC HTTP 错误：413 → `ContextLimit`，408 / 504 → `Transient`，401 / 403 → `Refused { code: "credentials" }`，其余 → `Unknown`；传输失败 → `Transient`；`InvalidToken` / `TokenExpired` / `NoPermission`（线上形式以向量 (d) 为准）→ `Refused { code: "credentials" }`；`UnknownMethod`、`ServiceNotValid` 等明确的永久错误 → `Permanent`；其余 → `Unknown`。OpenAI：`context_length_exceeded` → `ContextLimit`，401 / 403 → `Refused { code: "credentials" }`，400 / 404 / 422 → `Permanent`，408 / 409 / 425 / 429 / 500 / 502 / 503 / 504 → `Transient`，其余 → `Unknown`。**前置**：[`krpc_client.ts`](../../buckyos-websdk/src/krpc_client.ts) 现在把 HTTP 状态只写在错误消息里，传输失败也没有 code，要改成结构化（`code: 'http_error', data: { status }`，与 Rust 同形；传输失败 `code: 'transport'`）。**不做字符串匹配**（[[feedback-error-enum-only]]）：凭据错误走 `Refused { code: "credentials" }`（本节末尾已裁决 1），run 进 `paused`（`last_error.kind = credentials`），重新登录后 `resume`。`TimeoutLlmClient` 用 `AbortSignal.timeout` | 与 Rust 同一张错误表才能共享向量 (d)；集成方（AiWorkspace G4）需要的就是这个分类 |
| Q7 | **循环范围与分期** | **P0**（M1 最小闭环，M2 收齐）：function_call 循环（原生工具批次、`max_calls_per_round = 16`、工具失败继续、解析纠错重试、`DEFAULT_MAX_CONSECUTIVE_ERRORS = 3`）、预算（`max_tool_iterations` 共用额度与收尾模式 `ToolPolicy.wrap_up_tools` / `wrap_up_rounds` / 快照 `wrap_up_rounds_used`（xllm 自己不设，宿主写；见 [先行 TODO](./done/llm-context-context-budget-prereq-todo.md) P2）、wallclock `timeout`、`llm_timeout`、`max_tokens`）、六态 Outcome → `RunStatus { running, interrupted, paused, completed, failed, limit_reached }` 映射（xllm_rust_sdk.md §6 表逐行；有运行中任务时的回复 → `paused`，`last_error.kind = tasks_running`）、批次内串行派发、单一 `CheckpointHook`（每个工具调用前 + 每次推理前快照；`before_inference` 的 `Injection` 只有持久形态）、interrupt（`XllmInterrupter`）、`RunObserver` 事件、`usage.llm_requests` 累加、`json` / `json_schema` 回复契约（`OutputSpec::Json { strict, max_repairs: 1 }`）。**P1**（M3）：behavior 循环（`scan_xml_elements` + `XllmActionParser` + `XmlStepRenderer`、`<report end="true">` H4、`handover` 记录、Yield 立即结束 run → `paused`（`last_error.kind = yield`）、`<artifacts>` 并入 `report_result.artifacts`）、上下文压缩（移植 `llm_compress.rs`：按轮分段、头尾预算、分块折叠、`keep_exact`，每个 run 最多 3 次，`ByteHeuristicTokenizer`，`context_window` × `context_yield_ratio`（缺省 0.80）让出）、`file_model` 阶段（图片附件 `[image N]` + `<image_analysis>`）。**P2**（M4）：Deno 宿主（`native` runtime、`FileRunStore`、接手 Rust run）。**不做**：section 行号 / 组 / 外部组文件的多层合并（浏览器没有目录层级；单层 JSON 配置的 section 行号拼接仍要做，因为 `prepare_hosted` 也经过它）、MCP 发现。挂起原语（PendingTool / `allow_deferred`）在 Rust 里已删（冻结 F2 = B），TS 无此物 | 第一个集成方只要 P0；behavior 是 OpenDAN 的主路径，P1 做完 TS 版才算“同一套语义” |
| Q8 | **Run 持久化与恢复；谁能接手谁的 run** | `RunStore` 接口同 Rust（`create_run / write_record / read_record / write_snapshot / get_snapshot / lock_run / is_run_live / remove_run`），run 目录布局同 Rust §5（`run.json`、`snapshots/NNNN.json`、`.lock`、`exec/<call_id>/`），写入先 fsync 再原子 rename。P0：`MemoryRunStore` + `exportRun(runId): { record, snapshots }` / `importRun(...)`，宿主自己决定存哪里（AiWorkspace 会存到服务的 run 记录里）。M5：`IndexedDbRunStore`（库名 `xllm-runs`，同 schema，放 `hosts/browser/`），刷新页面后 `resume`；run 锁用 `navigator.locks`（没有就假设单 tab）；前提是同源下没有模型生成的代码（MW 已完成），否则程序能读写这个库。P2：`FileRunStore`（Deno 宿主，`~/.xllm/runs` 等真实目录）。**浏览器里恢复和导入的数据不可信**（§0）：`importRun` 和 IndexedDB 里的 record、快照可能来自服务端或被改过，里面的工具白名单要和当前场景取交集，超出就 `NotResumable`；模型访问仍用页面注入的 `LlmClient`，不读 record 里的凭据引用。**能否接手不按实现语言判断，按兼容条件判断**（2026-10-31 用户裁决；2026-11-06 用户再次确认 Deno 要能接手 Rust 的目录）：TS 的 `resume` 走与 Rust `XllmRun::resume` 相同的门——终态只读；`status == running` 且 `.lock` 被持有 → `RunBusy`；`version != 7`、快照版本 != `SNAPSHOT_FORMAT_VERSION`（6）、`prompt.protocol_version != xllm/2` → `NotResumable`；`host_commit_pending`、`handover` → `NotResumable`（宿主 Session 的事）；`host` 非空（libopendan 装配的 run）→ `NotResumable`，**这条是 TS 的收紧**：Rust 只拒绝 `workspace_binding`、已提交的结束 report、待提交的最终 Step，其余 hosted run 校验环境后可以接手；原 `workdir` 不存在 → `NotResumable`；取 run 锁与工作目录锁（同一 flock 文件 `<runs_dir>/<run_id>/.lock`、`<lock_dir>/<blake3(workdir)[..24]>.lock`，Deno 用 `Deno.FsFile.lock()`，与 fs2 是同一把 advisory flock）；按保存的 `config.runtime` 重建 runtime 并要求 `descriptor` 与 `config.runtime_descriptor` **逐字段相等**，否则 `RuntimeMismatch`（native 的 `host` 由同一条 `native_host_id()` 规则算出：环境变量 `AGENT_TOOL_HOST_ID`（`HOST_ID_ENV`）或 `/etc/hostname` + `/etc/machine-id`，所以“同机”是自然推论而不是额外规则）；`rebuild_toolset`：`config.tools.native / actions` 的每一项在本宿主都有实现（bash 组文件工具、`shell`；MCP 与具名宿主工具缺实现 → `NotResumable`，与 Rust “具名宿主工具缺失即报错”同义）；对模型广告的 spec 用 run.json 保存的版本，这要 Rust 先落地冻结 TODO F6（M4 前置），否则接手后模型看到的是 TS 写的 spec；没有结果的调用从最近的 checkpoint 重新派发，ToolManager **按 call_id 先找再做**（冻结 F12）：shell 先查 `local:shell:<call_id>` 任务，查不到就按 `describe_interrupted` 读执行目录给 `Unresolved { effect_unknown: true }`，幂等工具（read / glob / grep / task 查询）重跑，其余同样给 `Unresolved`，不核验不停止任何进程；`local:` 任务属于原进程，本宿主查询得到 unknown；凭据按 `SecretRef`（`env:` / 配置文件字段）重新解析，Provider 在本宿主可构造（openai 直接；buckyos 需要 Deno 侧 session，见 M4）。满足这些，**Deno 接手 Rust 跑到一半的 run 目录、Rust 接手 Deno 的，都是正常路径**。`site` 与 `native` 的 descriptor 必然不等，所以浏览器既不接手 native run 也不被接手，这是 descriptor 门的结果，不是单独的禁令。Node 宿主没有 flock（引入锁库要先与用户确认），P2 只做 Deno | F05 恢复是 PRD 首版能力；集成方的“关标签页 → interrupted → 续跑”依赖它；接手条件就是 xllm_rust_sdk.md §6 / §10 的 resume 检查，TS 只是第二个遵守它们的执行器 |
| Q9 | **怎样证明“同一套语义”（A19 延伸到 TS）** | 共享向量 JSON，**Rust 生成并校验、TS 消费**：(a) `prompt_assembly.json`：`{ llm_context, origin, host_system, flavor, host_prompt_sections, json }` → 期望 `prompt.system` 全文与 `prompt.runtime_protocol`（含 `xllm/2` 开场白、function_call 有 / 无 `report` 工具、无工具三种分支；behavior 分支 P1 补）；(b) `xml_behavior.json`：回复文本 → 解析出的 actions / report / 错误；(c) `loop_scripts.json`：脚本化 LLM 回复序列 + 工具结果 → 期望 `status / limit_reason / usage.llm_requests / 工具调用序列`（从 `llm_context/src/tests/{function_call_loop, budgets, error_policy, cancel}.rs` 与 `xllm.rs` 测试里挑 15–20 个）；(d) `provider_errors.json`：AICC / OpenAI 错误体 → `LLMComputeError` JSON。位置：`buckyos/src/frame/llm_context/tests/vectors/*.json`（Rust 测试 `xllm::vectors` 读取比对，`XLLM_VECTORS_REGEN=1` 重生成）→ 复制到 `buckyos-websdk/tests/fixtures/xllm/`（两仓手工同步，文件头 `vectors_version` 对齐）。(e) `runs/`：Rust 用 OpenAI mock 跑出的**真实 run 目录**（一个 `paused` 在 Transient 错误上、已执行过一次 `write_file`；一个被 SIGKILL 在 `shell` 执行中、快照停在该调用的 `before_tool_call` checkpoint 且调用无结果），原样入库作 fixture，TS 的接手测试不依赖本机 Rust 二进制也能跑。(f) `hints.json`：`HintLedger` 的 `once` / `dedup` 语义与 `<runtime_hints>` 渲染（冻结 TODO §3.8 要求）。(g) `task_text.json`：“仍在运行”等任务文本（冻结 TODO I1 要求）。(h) `runtime_identity.json`：native 的 `runtime_id` 派生与 `workdir_lock_path`（冻结 TODO §4 要求，要逐字节一致）。(i) `compress.json`：压缩的分段、头尾选择与摘要消息形状（M3 补）。补两条 Rust ↔ TS 端到端：同一个 OpenAI mock（Desktop `tests/aiworkspace/mock-llm.mjs` 的脚本化方式）分别被 `agent_tool xllm` CLI 与 TS core 驱动，比对 run.json 的 `status / usage / result.raw`；以及 **Rust 中断 → Deno 恢复完成**（M4 T4-3） | 两份实现、一张表；以后 Rust 改协议文本时向量测试会红，TS 跟着改 |
| Q10 | **为 AiWorkspace 集成预留什么（除 MW 外本单不做）** | 用户判断 aiws 的 runtime 会高度定制，这是后面迭代的方向。所以下面的映射只是起点，本单不替它定 runtime 的形状，它可以直接实现 `AgentRuntime`（Q4）。服务下发的 stage brief `{ system, user, tools, model, iterations }` 可直接映射为一个 hosted 任务：`llm_context = { provider: { type: "buckyos" }, model, loop_model: "function_call", max_tool_iterations: iterations, tools: { enabled: true, tools: tools.map(t => ({ name: t.name })) } }`，`hostSystem = system`，`host_prompt_sections = NONE`（AiWorkspace 的提示词自带约束与回复契约；`runtime_protocol = false` 只允许 function_call，正好）；场景是 `aiws.wish`，stage 工具是页面注入的工具（主线程转 `proc.tool`，服务端已做“执行身份权限 ∩ 输出声明”的校验，这才是真正的边界）；`run_program` 也是主线程工具，程序在 MW 的沙箱里跑，**不进循环 Worker**（否则程序能读 loop 状态、伪造桥接消息）；`aiws.llm.map` 直接用 providers 的 `LlmClient.infer({ json_schema })`，不需要循环。本单只保证这些入口存在并有测试覆盖：页面工具注入、`host_prompt_sections = NONE`、L1 直接推理、Worker 内工具与桥接工具并存 | 集成改动留给许愿格 TODO 的 M3（已改为依赖本单） |
| Q11 | **第一步的 loop 跑在哪** | 页面主线程，不经桥；Worker 放 M6 | §0 的上限在主线程上最直接；loop 大部分时间在等网络，不会明显卡 UI；core 不知道桥的存在，后加 Worker 不用改 core。2026-10-31 裁决的 Worker 形态仍是最终目标 |
| Q12 | **模型生成的程序能否联网（MW）** | 默认不能：沙箱 CSP 禁一切网络，[`prompts/execute.md:39`](../src/frame/aiworkspace/aiws/prompts/execute.md) 的“程序可以联网”改口。以后确有需要，再加带白名单的桥接接口，由主线程用 `credentials: 'omit'` 代发并记入 external data | 保留联网就保留了把快照发到外部的通道；[设计文档](<../doc/workspace/BuckyOS AiWorkspace 核心架构设计冻结思路.md>)（:440）写了分析 Workspace 上下文“不等于已经授权它自由探索外部世界”。代价：现在依赖外部数据的程序在本地模式下会失败；服务内执行路径（`wish/runner.rs`）行为不变，直到许愿格 TODO 把它下线 |
| Q13 | **D21 的“不用 iframe”怎么办（MW）** | 保留它对页面形态的决定（独立域名、SEO 友好的 URL），只放开一种用法：隐藏 iframe 作为程序沙箱的容器，不承载页面 | 当时否决的是用 iframe 承载 Workspace 页面；代码里已有 sandbox iframe + CSP 的先例（`WishPreviews.tsx`、`ContentPreview.tsx`） |

**已裁决（2026-11-06，用户确认）：**

1. **凭据错误用 `Refused { code: "credentials" }`。** waist 不用改，不再匹配字符串。TS 从 M1 起按这个映射（Q6），Rust 落点是 T2-0。现在的字符串匹配还会误判：`Permanent` 的消息只要含 `token` 就算凭据，比如 `max_tokens` 超限也会进 `paused`。
2. **`check_after` 取小，工具只能把等待缩短。** `wait = min(check_after（有才算）, 等待预算, MAX_IN_TOOL_WAIT_MS, deadline)`。[runtime review 单](./xllm-agent-runtime-review-todo.md) §3 原来的公式 `check_after ?? policy ?? runtime` 其实不是取小，工具仍能把等待拉到上限，已按这条改掉。代码里没有工具在设置非空的 `check_after`（只有 shell 透传和 dev CLI 解析），所以不影响现有工具。Rust 落点是 T2-0。该单其余 Q1、Q3–Q9 是新增字段，用户还没裁决，M1–M3 用不到，M4 前要先问（`RuntimeConfig` 拒绝未知字段，TS 落后就接手不了 Rust 的 run）。
3. **Deno 宿主引入 `@noble/hashes` 算 blake3。** 只在 `hosts/deno/` 引用，不进浏览器包；与 Rust 逐字节一致由向量 (h) 验证。

### 2.1 可选的基础工具：REST + 命令（第三步，M5）

用户原话（11-06，Q4）：“纯浏览器意义不大，这个应该是各个网站自己构建。但可以尝试建立新套路，类似基础的4个工具，围绕‘RESTful 语义’和 RPC‘命令化’来进行设计。”

11-07 补充：“RPC 命令化 xcall 是可以直接支持的，这里是说不符合规范的 rpc，要么封装成 did-object 格式，可以用 xcall，要么就自己转成命令。”

方向已确认。工具名 `exec`、声明形状和解析规则是按这两段话写的，用户可以改。

`bash_core` 的 4 个工具是文件世界的基础。网站没有文件和 shell，对应的是资源和命令：

| `bash_core` | 网站工具 | 语义 | 恢复时重新派发 |
| --- | --- | --- | --- |
| `read` | `read { uri, offset?, limit? }` | GET。集合的 `read` 就是列表，过滤和翻页写进 query（`notes?tag=x`，同 key url §6） | 重跑 |
| `write_file` | `write { uri, content, mode? }` | PUT。`mode` 同 `write_file`：`write` / `new` / `append` | `Unresolved`（`new`、`append` 不幂等，统一处理） |
| `edit_file` | `edit { uri, old_string, new_string }` | PATCH。`old_string` 必须恰好出现一次，同 `edit_file` | `Unresolved` |
| `shell` | `exec { command }` | 执行一条命令，参数名同 `shell`。内置 `xcall`；网站自己的命令在 `commands` 里声明 | `xcall` 看 action 的 `risk`，`read_only` / `idempotent_write` 重跑；自定义命令看 `access`，`read` 重跑；其余 `Unresolved` |

RPC 分两种，都经 `exec` 调用：

- **符合 DID Object 规范的，直接用 `xcall`。** action 声明在资源上（`actions`，带 `risk`）。模型写 `xcall <object> <action> [--params <json>] [key=value ...] [--key=value ...]`，`<object>` 是资源 URI。用法同 Rust 的 `xcall` 命令（[`agent_tool_cli_dev`](../src/frame/agent_tool_cli_dev/src/lib.rs) 的 `XCALL_USAGE` 和 `parse_object_x_call_cli_command`），模型在 OpenDAN 里学到的写法可以直接用。不符合规范的 RPC，网站可以在页面里把它封装成这个形状，再走 `xcall`。
- **不封装的，网站自己转成命令。** 在 `commands` 里声明。字段取 websdk CLI [`CommandDefinition`](../../buckyos-websdk/cli/core/command.ts) 的子集：`summary`、`description`、`positionals`、`options`、`inputSchema`、`access`、`examples`。`handler(ctx, input)` 换成 `call(input)`，因为 `CommandContext` 带的是 CLI 宿主的配置、会话和 host，页面里没有。

这些都沿用现有约定，没有新发明：

- [key url §1](<../doc/key url.md>) 已有同样的划分：能直接修改同一资源的走文件操作，只能通过业务动作修改的走 action。
- Rust 的 agent 也是这样分的：`xcall`、`buckyos` CLI 都是在 `shell` 里跑的命令。`read` 读 DID Object 时会列出 `Actions:` 和 `risk`。
- `risk` 的取值同 `ActionRisk`：`read_only` / `idempotent_write` / `write` / `destructive` / `admin`。命令的 `access` 同 CLI 的 `AccessLevel`：`read` / `write` / `destructive` / `privileged`。
- 选工具的规则和 `bash_core` 一样：REST 能表达的用 `read` / `write` / `edit`，表达不了的才用 `exec`。删除也是 action（`risk: destructive`），`bash_core` 同样没有删除工具。

页面这样声明：

```ts
const tools = siteTools({
  resources: {
    notes: { description: 'All notes, supports ?tag=', read: ({ query }) => listNotes(query.get('tag')) },
    'notes/{id}': {
      description: 'One note',
      read: ({ params }) => getNote(params.id),   // 可以返回 { content, version }
      write: ({ params }, { content, mode, ifVersion }) => putNote(params.id, content, mode, ifVersion),
      actions: {                                  // exec "xcall notes/1 delete"
        delete: { description: 'Delete this note', risk: 'destructive', input: { type: 'object', properties: {} },
                  call: ({ params }) => removeNote(params.id) },
      },
    },
  },
  commands: {                                     // exec "notes-export --tag work"
    'notes-export': {
      summary: 'Export notes as one markdown document',
      options: [{ name: 'tag', type: 'string', description: 'Only notes with this tag' }],
      inputSchema: { type: 'object', properties: { tag: { type: 'string' } } },
      access: { mode: 'fixed', level: 'read' },
      call: ({ tag }) => exportNotes(tag),        // 页面里已有的、不符合规范的 RPC
    },
  },
})
```

语义：

- **发现。** `read uri="/"` 返回声明的全部资源、action（含 `risk`）和命令（名字和 `summary`），`read` 的工具说明让模型从这里开始。`exec "<命令> --help"` 返回这条命令的用法，格式同 CLI 的 `describe`。四个工具的 spec 是固定的，资源和命令变多不增加每轮的 token。
- **`edit`。** 默认由 `siteTools` 组合：`read` → 检查 `old_string` 恰好出现一次 → `write`，并把 `read` 返回的 `version` 作为 `ifVersion` 传给 `write`，冲突时报错，不静默覆盖。资源也可以自己实现 `edit`。
- **结果。** `read` 的文本按行分页，默认上限 64 KB，超出时说明总行数和下一页的 `offset`；返回对象时先序列化成 JSON。`exec` 的结果也受 64 KB 上限约束；`xcall` 的结果带 `risk`。
- **`exec` 一次一条命令，不是 shell。** 分词沿用 CLI 的 `parseShellLine`（单双引号、反斜杠转义）。引号外出现 `|`、`;`、`&`、`>`、`<`、`` ` ``、`$` 时报错，未知命令也报错。两种错误都要说明 `exec` 不是 shell，并列出可用命令。模型习惯 bash，`xcall … | jq` 这类写法一定会出现，要在错误里讲清楚，不能让 `|` 被当成参数吞掉。
- **参数。** 自定义命令按 CLI 的规则解析：位置参数、`--opt value` / `--opt=value`、布尔选项、`--`、未知选项报错，最后用 `validateSchema` 按 `inputSchema` 校验。`xcall` 只认 `--params`、`key=value`、`--key=value`，按 action 的 `input` 校验。Rust 版的 `--config`、`--session-id`、`--idempotency-key`、`--confirm-token`、`--trace-id` 报未知选项：路由由页面声明，确认由页面实现。
- **复用 CLI 的代码。** websdk 构建的 `rootDir` 是 `src`，`src/` 不能引用 `cli/`。T5-1 把上面用到的纯函数和类型挪到 `src/internal/command_args.ts`：`parseShellLine`、参数解析的核心、`validateSchema`，以及 `JsonSchema`、`PositionalDefinition`、`OptionDefinition`、`AccessLevel`。这个文件自包含，不 import 其它文件；`cli/core/{argv,command}.ts` 用相对路径引用它（vite 会打包进 `cli.mjs`，不需要新的公开导出），REPL 全局选项那段留在 CLI。CLI 行为不变，`pnpm test:cli` 作回归。

边界（§0 第 2 条在这里落地）：

- **资源和命令逐个声明，没有通配。** 资源模式只有字面段和 `{name}`，`{name}` 只匹配一段；命令名按字面匹配。未声明的 URI、资源没实现的方法（没写 `write` 就是只读）、未声明的 action、未声明的命令，都得到 `Observation.Error`。
- **SDK 不提供把 uri 转发给 `fetch` 的默认实现。** 页面自己这样写，等于把 session 交给模型，§0 第 2 条就失效了。命令同理：不要写一个把参数原样转成 kRPC 调用的通用命令（`krpc <service> <method> <json>`）。这是这个套路最容易踩的坑，`xllm_ts_runtime.md` 要写明。
- **URI 先规范化再匹配。** 用 `new URL(uri, base)` 处理 `..`，只接受相对路径和页面声明过的绝对 URL；解码后的段含 `/`、`\` 或等于 `..` 时拒绝，否则 `fetch('/api/notes/' + id)` 这样的实现会被穿越。`xcall` 的 `<object>` 也走这一步。
- **收窄。** 配置只能在四个工具里选：`tools: [{ name: 'read' }]` 就是只读。更细的收窄（哪些资源、action、命令）由页面代码决定，不加配置字段，Rust 的 `ToolSource::Named` 不用改。
- **确认。** 非只读操作要不要用户确认，由页面在实现里决定，SDK 不设审批门（同 `xcall` 现有的选项 C，[对象 URL TODO](./done/agent-object-url-todo.md)）。

这是试验：

- 代码只在 `runtime/site_tools.ts`，core 不依赖它。页面可以混用：`tools: { ...siteTools(…), my_tool: {…} }`。
- 现在没有证据说明模型用这四个工具比用具名工具好，也没有证据说明它在 `exec` 里写命令比调具名工具少出错。T5-6 用真实模型跑一次，记录轮数和误用，结果交给用户决定留不留。不留就删掉 `site_tools.ts`，挪出去的解析函数 CLI 照用，其它不受影响。
- BuckyOS 上的网站，资源模式可以直接写 `obj://…`。服务端已经用 `KrpcObjectProvider` 把现有 kRPC 方法包成 DID Object，M5 的 T5-2 把它的 Profile 映射成 `resources`（`didObjectResources`），现有页面接上它就是第三步说的“简单改造”。AiWorkspace 的实体（`obj://aiworkspace/…`）以后也可以这样接（Q10），本单不做。

## 3. 代码接入点

| 位置 | 改动 |
| --- | --- |
| `buckyos-websdk/src/xllm/core/{types,loop,outcome,errors,run,store,observer,tokenizer}.ts` | 新建（M1） |
| `buckyos-websdk/src/xllm/runtime/{types,site}.ts`、`src/xllm/providers/aicc.ts`、`src/xllm/dev/scripted.ts`、`src/xllm/index.ts`、`src/xllm/dev.ts` | 新建（M1） |
| `buckyos-websdk/src/xllm/runtime/site_tools.ts`、DID Object HTTP 客户端（`src/`） | 新建（M5，§2.1） |
| `buckyos-websdk/src/internal/command_args.ts`、`cli/core/{argv,command}.ts` | 新建，从 `cli/core/{argv,command}.ts` 挪入命令解析的纯函数和类型；CLI 改为用相对路径引用，行为不变（M5，§2.1） |
| `buckyos-websdk/src/krpc_client.ts` | HTTP 与传输错误结构化（M1，Q6）；检查其它调用方是否依赖错误消息文本 |
| `buckyos-websdk/vite.config.ts`、`package.json#exports` / `files`、`tsconfig.build.json` | 入口 `xllm`、`xllm_dev`，子路径 `./xllm`、`./xllm/dev`（M1）；`xllm_worker` 与 `./xllm/worker`（M6）；`xllm_deno` 与 `./xllm/deno`（M4） |
| `buckyos-websdk/examples/xllm_browser/{index.html,main.ts,README.md}` | 示范页（M1） |
| `buckyos-websdk/tests/xllm_cases.ts`、`tests/xllm/*.test.ts`、`tests/browser/xllm/*.spec.js`、`tests/deno/xllm_cases_test.ts` | 共享用例，三个宿主跑同一组（M1） |
| `buckyos-websdk/src/xllm/core/{config,prompt,protocol}.ts`、`src/xllm/runtime/{tool_manager,hints,task_model}.ts`、`src/xllm/providers/timeout.ts`、`src/xllm/dev/openai.ts`、`tests/fixtures/xllm/*.json` | 新建（M2） |
| `buckyos/src/frame/agent_tool/src/xllm.rs`、`lib_opendan/src/runner/outcome.rs`、`aiworkspace/server/src/wish/llm.rs`、`llm_context/src/error.rs`（只改注释）、`doc/llm_context/agent_tool/agent_tool_result_protocol.md` | 凭据错误与 `check_after`（T2-0） |
| `buckyos/src/frame/agent_tool/src/xllm.rs` | 新增 `mod vectors` 测试：读 / 重生成向量（M2） |
| `buckyos/src/frame/agent_tool/src/runtime/mod.rs:113` 附近 | `"site"` 加入规划名单 → `Capability`（M2） |
| `buckyos/src/frame/llm_context/tests/vectors/*.json` | 新建（M2） |
| `buckyos/doc/llm_context/xllm_ts_runtime.md`（新）、`xllm_rust_sdk.md`（顶部加“TS 版本对照”指针、修过期内容）、`AgentRuntime 设计.md` §3 表、`readme.md` “命令行工具”节 | 文档（M1 首版，M2 / M7 补全） |
| `buckyos-websdk/src/xllm/worker/{protocol,worker,host}.ts`、`src/xllm/worker_entry.ts`、`tests/browser/xllm/worker.spec.js` | 新建（M6） |
| `buckyos-websdk/src/xllm/core/xml.ts`、`src/xllm/runtime/{step_renderer,compress}.ts` | 新建（M3） |
| `buckyos-websdk/src/xllm/hosts/browser/store.ts` | 新建（M5） |
| `buckyos-websdk/src/xllm/hosts/deno/{store,native_runtime,index}.ts`、`tests/deno/xllm_takeover_test.ts`、`tests/fixtures/xllm/runs/` | 新建（M4：`FileRunStore` + flock、`native` runtime、接手测试与 Rust 生成的 run 目录 fixture） |
| `buckyos-websdk/package.json` | 加 `@noble/hashes`，只由 `src/xllm/hosts/deno/` 引用（M4，§2 已裁决 3） |
| `buckyos/src/frame/agent_tool/src/xllm.rs` `rebuild_toolset` | 落地冻结 TODO F6：恢复时对模型广告 run.json 保存的 spec（M4 前置） |
| `buckyos/src/frame/desktop/src/app/aiworkspace/executor/{programHost,program.worker}.ts`、`vite.config.ts`、`tests/aiworkspace/program-sandbox.spec.ts`、`src/frame/aiworkspace/aiws/prompts/execute.md`、`doc/workspace/BuckyOS AiWorkspace 内核架构改进.md` D21 | 程序沙箱（MW） |

## 4. 里程碑

执行顺序（2026-11-07 用户裁决，分三步）：

1. **第一步 M1：** 在浏览器里跑通 agent loop。开发者照示例自己构造浏览器里的 runtime，有哪些工具都由开发者自己定。接口设计不好，runtime 就会退化成 MCP 那样的工具清单，M1 开头的表列出了防止这一点要有的东西。
2. **第二步 M2 → M3 → M4：** 在第一步的基础上，完成基于 Deno、与 Rust 实现兼容的 TS xllm。M2 是契约向量与 P0，M3 是 P1（behavior 循环、压缩），M4 是 Deno 的 `native` runtime 与接手 Rust run。
3. **第三步 M5：** 基于 DID Object 协议和 RESTful API，让现有页面只需简单改造就得到一个相对标准的 runtime，并让 xllm 代替用户操作当前页面（§2.1）。

另外三项：

- **MW** 不依赖其它里程碑，修的是已上线路径，建议先做，或与 M1 并行。
- **M6 Worker 宿主**不在三步之内，第三步之后再定做不做。许愿格 TODO 的 M3 现在依赖它（§7）。
- **M7 文档与回写**最后做。M5 的 `IndexedDbRunStore` 依赖 MW。

### MW 程序 Worker 隔离（修已上线的 AiWorkspace，不依赖其它里程碑）

**问题。** [`program.worker.ts`](../src/frame/desktop/src/app/aiworkspace/executor/program.worker.ts) 用同源 Worker 执行模型生成的程序。D21 认为它“没有 cookie 访问，不持 token”，但 Worker 只是读不到 `document.cookie`，它的 `fetch` 仍会自动带上同源 cookie。

实测（2026-11-06，本机 Chromium 156，不依赖 Zone 的玩具页面，cookie 非 HttpOnly）：

| 模型生成的程序能做什么 | 现在：同源 module Worker | 修后：sandbox iframe + CSP + classic Worker |
| --- | --- | --- |
| 带 session cookie 请求本源 | **能** | 被拦 |
| 向外发数据（`fetch`、WebSocket、远程 `import`、嵌套 Worker） | **4 种都到达接收端** | 0 次到达 |
| 读 IndexedDB | **能** | `SecurityError` |
| 覆盖 Cache Storage 里的 JS 资源 | **能** | `caches` 不存在 |
| 动态 `import` blob 里的程序并运行 | 能 | 能 |

另外两个变体：sandbox 里的 module Worker 启动失败（所以只能用 classic）；只有 sandbox、没有 CSP 时，向外发数据的 4 种方式仍然全部成功（所以两者都要）。Firefox、WebKit 未测。

按代码推断、未在 Zone 上实测：

- 网关按路径把 [`/sso_refresh`](../src/rootfs/etc/boot_gateway.yaml) 转给 control-panel，不看主机名，[响应体带 `session_token`](../src/frame/control_panel/src/sys_auth_backend.rs)（:632）。同源 Worker POST 一次就能拿到真 token，之后可以调任何 kRPC。这个 token 具体能调哪些服务，没有逐项核对。
- `/nfs/v1`、`/bucket`、`/ndm` 接受 SSO cookie（[`nfs_server/src/auth.rs`](../src/frame/nfs_server/src/auth.rs)）。
- Service Worker 的 `asset()` 优先读缓存（[`sw.template.js`](../src/frame/desktop/src/service-worker/sw.template.js):126）：程序覆盖缓存后，下次加载页面就会在持有 token 的主线程里执行。
- aiws 页面没有 CSP，`aiws.js` 的 `#trackFetch` 对外部 `fetch` 只记录不拦截。

**沙箱形状：**

```html
<iframe sandbox="allow-scripts" hidden srcdoc="
  <meta http-equiv='Content-Security-Policy'
        content=&quot;default-src 'none'; script-src 'unsafe-inline' blob:; worker-src blob:&quot;>
  <script>/* 收到 runner 文本后 new Worker(blobUrl)，不带 type: 'module'；转发消息 */</script>
"></iframe>
```

runner 再 `await import(blobUrl)` 执行程序。iframe 的源是 `null`，所以没有 cookie、IndexedDB、Cache；CSP 禁掉网络和远程脚本，Worker 从 blob 创建时继承这份 CSP。程序放在 Worker 而不是 iframe 脚本里：iframe 脚本能把自己导航到外部 URL，CSP 拦不住，Worker 没有这个能力。

- [ ] **TW-1 修前确认与跨浏览器验证。** 在 DV Zone 用 devtest 账号复现一次：`aiws-<id>` 页面的本地模式程序里 `fetch('/sso_refresh', { method: 'POST', credentials: 'include' })` 能否拿到 `session_token`，`/nfs/v1` 能否带 cookie 访问（确认上面的推断；会轮换 devtest 的 token）。把沙箱形状在 Firefox、WebKit 上跑一遍（`npx playwright install firefox webkit`，不加 npm 依赖）。任一浏览器跑不通就停下报告，备选是给程序一个独立的无 cookie 源，那要改网关。
- [ ] **TW-2 `programHost.ts`。** 用隐藏 iframe 承载 Worker。主线程只接受 `event.source === iframe.contentWindow` 的消息，类型只有 `llm_map` / `done`，字段按形状校验。`targetOrigin` 只能是 `*`（沙箱的源是 `null`），所以发进去的只能是快照文件、程序源码和 `llm_map` 结果，不能有 token。超时和取消时 `iframe.remove()`，并验证死循环的程序确实被杀掉。
- [ ] **TW-3 `program.worker.ts` 打包。** 沙箱里不能 `new Worker(new URL(...), { type: 'module' })`。构建时把它连同 `aiws.js` 打成一段 classic 脚本文本，dev 和 production 都能用（参照 `vite.config.ts` 的 `desktop-service-worker` 插件和 websdk `hash_worker.ts` 的内联；`worker: { format: 'es' }` 是给 Replica Worker 的，不动）。不加新依赖。
- [ ] **TW-4 联网（Q12）。** 改 `prompts/execute.md:39`：本地模式的程序不能联网。确认 `flush()` 的 `external` 为空时全链路正常，程序调 `fetch` 失败时错误文本能让模型看懂并改写。服务内路径 `wish/runner.rs` 不动。
- [ ] **TW-5 测试。** `tests/aiworkspace/program-sandbox.spec.ts`（不依赖 Zone）：程序尝试本源带 cookie 请求、`POST /sso_refresh`、外部 POST、WebSocket、远程 `import`、IndexedDB、Cache 写入、嵌套 Worker，全部失败，mock 接收端 0 次命中，页面的缓存和 IndexedDB 不变；正常程序照常产出 `results` / `files`；`llm.map` 经桥可用；死循环程序超时后被杀，页面仍可操作。回归：`tests/aiworkspace/wish.spec.ts` 的本地执行器用例。
- [ ] **TW-6 真实 Zone DV（手动，一次）。** `aiws-<id>` 页面跑一个本地模式的许愿格到 Apply；同一个程序里 `POST /sso_refresh` 失败、token 未轮换。记录到 §8。
- [ ] **TW-7 文档与手册。** D21（[`内核架构改进.md`](<../doc/workspace/BuckyOS AiWorkspace 内核架构改进.md>):1088 和 :950）改成实测结论，写明 Q13 的放开范围；按 AGENTS.md 的 cmd_skills 规则核对 `aiworkspace_run`、`aiworkspace_template_dev` 手册，不受影响也要写明依据。
- 完成条件：`npx playwright test --config playwright.aiworkspace.config.ts tests/aiworkspace/program-sandbox.spec.ts tests/aiworkspace/wish.spec.ts` 绿；三种浏览器的结果和 DV 记录在 §8。
- 不在 MW：主线程 XSS 与页面级 CSP；HTML 预览等其它执行上下文的审计；提示词注入诱导模型滥用白名单内的工具，以及程序经 `llm_map` 或结果夹带数据（这些靠工具白名单、服务端校验和额度限制）。

### M1 第一步：在浏览器里跑起 xllm loop，并示范如何构造 runtime

目标：一个普通 HTML 页面引入 `buckyos/xllm`，照示例构造一个 runtime，跑完一次 function_call loop。有哪些工具全由开发者自己定，SDK 在这一步不提供任何工具。xllm 的能力不超过页面内嵌 JS（§0）。

**runtime 不能退化成工具清单。** 如果 runtime 只是“名字 → 函数”，它就和 MCP 的工具列表没有区别，xllm 的恢复、副作用和环境语义都用不上。Rust 的 `AgentRuntime` 比工具清单多出下面四件事，这一步的接口和示例都要有：

| 多出来的 | Rust | 网站 runtime 怎么体现 |
| --- | --- | --- |
| 身份 | `descriptor()`，恢复时逐字段比较 | `scene` 进 `target`；换场景恢复报 `RuntimeMismatch` |
| 绑定到 run | `open(ctx, tools_cfg, deps)` 按 run 给出工具，配置只能收窄 | 工具调用拿到 `ctx = { run_id, call_id, signal }`；配置里的 `tools` 只能从页面给的工具里选 |
| 副作用账 | 非幂等的调用恢复时得到 `Unresolved`，说明文本来自 `describe_interrupted` | 除了 `idempotent`，工具还可以给 `describeInterrupted(args, ctx)`，恢复时告诉模型这次调用实际有没有生效；没给就用通用说明 |
| 环境描述 | `info()` → `RuntimeInfo` → `{{runtime.*}}` | `RuntimeInfo` 由 `scene`、`origin`、`hostname` 生成，提示词模板照常可用 |

有了 `call_id`，页面可以用它给写操作去重，自己把写操作做成幂等的，`idempotent: true` 才有依据；`signal` 在 `interrupter()` 中断时触发。长任务（`pending` + `check_after`）在 M2 随 AgentToolResult 信封补上。第三步的资源和 action（§2.1）是在这之上再加一层统一语义，不替代这几项。

页面代码大致是这样（示意，字段以 Q3 / Q4 为准）：

```ts
import { createSiteRuntime, AiccLlmClient, MemoryRunStore, XllmTask, XllmRun } from 'buckyos/xllm'
import { getAiccClient } from 'buckyos/browser'

// 这个场景能做的事全写在这里：工具名、参数、实现都由页面自己定
const runtime = createSiteRuntime({
  scene: 'demo.notes',
  origin: location.origin,
  hostname: location.hostname,
  tools: {
    get_selection: {
      spec: { description: 'The text selected on this page', args_schema: { type: 'object', properties: {} } },
      idempotent: true,
      call: () => window.getSelection()?.toString() ?? '',
    },
    append_note: {
      spec: { description: 'Append text to the saved note',
              args_schema: { type: 'object', properties: { text: { type: 'string' } }, required: ['text'] } },
      idempotent: true,                       // 按 call_id 去重，恢复时重跑不会写两次
      call: ({ text }, { call_id }) => appendNoteOnce(call_id, text),
    },
    clear_note: {
      spec: { description: 'Delete the saved note', args_schema: { type: 'object', properties: {} } },
      call: () => localStorage.removeItem('note'),
      describeInterrupted: () =>
        localStorage.getItem('note') === null ? 'The note is already deleted.' : 'The note was not deleted.',
    },
  },
})

const deps = { llm: new AiccLlmClient(getAiccClient()), runtime, store: new MemoryRunStore(), hostPromptSections: 'NONE' }
const task = await XllmTask.prepareHosted(
  { provider: { type: 'buckyos' }, model: 'llm.chat', loop_model: 'function_call', max_tool_iterations: 6,
    tools: { enabled: true, tools: [{ name: 'get_selection' }, { name: 'append_note' }, { name: 'clear_note' }] } },
  'demo.notes', 'You help the user take notes from this page.', deps,
)
const run = await XllmRun.start(task, deps)
const outcome = await run.execute()
```

- [ ] **T1-1** `core/types.ts`：从 Rust 抄 `XllmError`、`ProviderConfig`、`LlmContextJson`（严格键）、`EffectiveConfig / EffectiveTools / RunLimits`、`RunRecord / RunStatus / RunErrorRecord / UsageRecord`、`LLMContextSnapshot / LLMContextState`、`LLMContextOutcome`、`LLMComputeError`、`Observation`、`RunEvent / RunPhase`。类型一次抄全，M2 不再返工。
- [ ] **T1-2** `core/loop.ts`、`outcome.ts`、`errors.ts`：function_call waist（Q7 P0 清单除 JSON 回复契约），六态 → `RunStatus`。`ContextLimitReached` 在 M3 之前没有压缩可走，按 Rust 无窗口情形处理为 `failed`（`context_capacity`）。
- [ ] **T1-3** `runtime/site.ts`：`createSiteRuntime`（Q4），`AgentRuntime` 接口公开导出。工具形状 `{ spec, call(args, ctx), idempotent?, describeInterrupted? }`，`ctx = { run_id, call_id, signal }`。派发：未知工具或参数错误 → `Observation.Error`；工具抛异常 → `Observation.Error`；返回值 → `Observation.Success`；恢复时重新派发的调用，`idempotent` 的重跑，其余 `Unresolved { effect_unknown: true }`，说明文本取 `describeInterrupted`，没有就用通用文本；`interrupter()` 触发 `signal`。AgentToolResult 信封、hint、调用内等待放 M2。
- [ ] **T1-4** `core/run.ts`、`store.ts`、`observer.ts`：`XllmTask.prepareHosted`（只接受 Q10 用到的字段子集，严格键；只支持 `host_prompt_sections = NONE`，其余报 `Capability`，M2 补）、`XllmRun.start / execute / resume / injectInput / interrupter`、`MemoryRunStore`、`exportRun / importRun`。Q4、Q8 的拒绝规则在这一步就要有。
- [ ] **T1-5** `providers/aicc.ts`（Q6，错误映射先覆盖 AICC 路径，含 `credentials`）、`krpc_client.ts` 错误结构化、`dev/scripted.ts`（`ScriptedLlmClient`，对照 Rust wishkit 的 `ScriptedModel`，按调用序号回复）。
- [ ] **T1-6** 示范页 `examples/xllm_browser/`，不依赖 Zone 也能打开（默认用 `ScriptedLlmClient`，登录后可切到 AICC）。依次演示：(1) 构造 runtime：场景名 + 页面自己写的具名工具，对照 M1 开头的表说明每一项在代码里的位置；再给一个直接实现 `AgentRuntime` 的版本；(2) 跑 loop，事件实时显示在页面上，可以中断；(3) `exportRun` 后用同场景的新 runtime `resume` 到完成：中断在 `get_selection`、`append_note` 上的调用重跑，笔记里只有一份；中断在 `clear_note` 上的得到 `Unresolved`，说明文本来自 `describeInterrupted`；换一个场景的 runtime `resume` → `RuntimeMismatch`；(4) 边界：配置里写 `group:bash`、`runtime.kind: native`、`provider.base_url`、页面没给的工具各报 `Capability`；模型调用不在本次工具集里的工具得到 `Observation.Error`。页面写 `#status[data-state]` / `#result`，约定同现有真实浏览器用例。
- [ ] **T1-7** `doc/llm_context/xllm_ts_runtime.md` 首版：§0 的能力边界 + “网站怎么构建自己的 runtime”（场景、`createSiteRuntime`、工具的 `ctx` / `idempotent` / `describeInterrupted`、直接实现 `AgentRuntime`、配置只能收窄、恢复规则、为什么 runtime 不是工具清单），示例代码与示范页一致。
- [ ] **T1-8** 用例写成 `tests/xllm_cases.ts`（参照 `ndm_client_cases.ts`），三个宿主跑同一组：jest、Deno（`tests/deno/xllm_cases_test.ts` 引入构建产物，证明 §0 第 3 条）、真实浏览器（`tests/browser/xllm/xllm.spec.js`，本地静态服务打开示范页，不依赖 Zone）。覆盖：正常完成、工具失败继续、迭代预算用尽、wallclock 超时、中断、export / resume、场景不符、T1-6 (4) 的拒绝路径、错误映射（含 `credentials` → `paused`）。
- [ ] **T1-9** 真实 Zone DV（手动，一次）：在 `systest.test.buckyos.io` 页面用 websdk 登录 devtest 后，`AiccLlmClient` 跑一个用到示范页工具的任务，中途中断一次，再 `resume` 到 `completed`；AICC 用量表 `user_id = devtest`、`app` 为该页面身份。run.json 片段记录到 §8。
- 完成条件：§5 的 M1 命令全绿；浏览器入口的构建产物不含 `Deno.`、`child_process`；`src/xllm/{core,runtime,providers}` 没有环境 API 和 `eval` / `new Function` / 动态 `import`（§5 的 grep 为空）；DV 记录在 §8。
- 不在 M1：Worker、向量、behavior 循环、压缩、完整提示词装配、hint、任务后端、IndexedDB、Deno `native`、审批门、基础工具与 DID Object 适配（M5）。

### M2 契约、向量与 P0 收齐（Rust 仓出向量，websdk 全部通过）

- [ ] **T2-0 Rust 先落地 §2 的两项裁决**（向量要按改后的行为生成）：
  - (a) 凭据：`xllm.rs` 加 `REFUSED_CREDENTIALS = "credentials"`；`provider_error_from_rpc` 把 `InvalidToken` / `TokenExpired` / `NoPermission` 和 `http_error` 的 401 / 403 映射成它（`http_error` 现在是 `Unknown`），`OpenAiLlmClient` 的 401 / 403 同样；`classify_error` 删掉字符串匹配，`Permanent` 只报 `provider_permanent`，`Refused { credentials }` 报可恢复的 `credentials`。其它读 `Refused` 的地方要排除它，否则凭据问题会被说成额度问题：[`lib_opendan/src/runner/outcome.rs`](../src/frame/lib_opendan/src/runner/outcome.rs):763 和 [`aiworkspace/server/src/wish/llm.rs`](../src/frame/aiworkspace/server/src/wish/llm.rs):195（后者会报成 `AI_QUOTA_EXHAUSTED`）。[`llm_context/src/error.rs`](../src/frame/llm_context/src/error.rs) 的 `Refused` 注释写的是额度和资源策略，放宽说明（只改注释）。测试：`xllm.rs` 现有的 `"invalid token: expired"` 用例改用结构化错误；加一例 `Permanent` + `max_tokens` 消息 → `failed`。
  - (b) `check_after`：`XllmToolManager::call_tool` 改成取小（§2 已裁决 2）；测试：大于预算被截、小于预算生效；更新 [`agent_tool_result_protocol.md`](../doc/llm_context/agent_tool/agent_tool_result_protocol.md) 的 `check_after` 一节。[runtime review 单](./xllm-agent-runtime-review-todo.md) 实施时不要重复做。
  - 按 AGENTS.md 核对 `xllm` 手册，不受影响也要写明依据。
- [ ] **T2-1** `doc/llm_context/xllm_ts_runtime.md` 补全：分层与 API（Q3）、`site` runtime kind（Q4）、错误映射表（Q6）、`RunStore` 接口（Q8）、向量格式与同步规则（Q9）、不做清单；基础工具与 IndexedDB 布局在 M5、桥协议消息表（Q5）在 M6 落地时补。`xllm_rust_sdk.md` 修过期内容（TS 以它对照）：`task_wait_ms` 默认是 25 秒不是 30 秒、压缩的摘要模型、RunRecord 字段（`hints_once`、`compactions`、`pid`）、两个重号的 §10；`LLM Context 设计.md` §11.6 和冻结 TODO §8.3 里“F11 未做”改掉（已在 `5184e1947` 实现）。
- [ ] **T2-2** Q9 的向量 (a)(b)(c)(d)(f)(g)(h) + `xllm::vectors` 测试；`loop_scripts.json` 每条注明来源测试名。(e)：用 OpenAI mock 与 `AGENT_TOOL_HOST_ID=xllm-fixture` 跑出两个 run 目录（`paused` 于 Transient、SIGKILL 于 `shell`），连同 `describe_interrupted` 期望文本一起入 `tests/vectors/runs/`。
- [ ] **T2-3** `runtime/mod.rs`：`site` → `Capability`；AgentRuntime 设计 §3 表加 `site`（“网站用 TS 运行时自己构建；Rust 侧报 Capability”）；`xllm_rust_sdk.md` 顶部指针。
- [ ] **T2-4** `core/config.ts`：单层 `LlmContextJson` 全量解析（未知字段 / 类型 / 别名冲突 → `Config { file, field, reason }`）、默认值（`DEFAULT_*` 同值）、`compute_tools_config` 等价（`enabled` 默认 false、`tools: [{ name }]` 只接受场景白名单内的工具、Q4 列出的项 → `Capability`）。
- [ ] **T2-5** `core/prompt.ts` + `protocol.ts`：section 行号拼接（别名表同 Rust）、`TemplateEnv`（`{{runtime.*}}` / `{{env.*}}`、`\{{` 转义、缺失变量 → `Template`）、`build_runtime_protocol_for` 全文移植（OneShot / Session 开场白；function_call 三分支；behavior 分支 M3）、`assemble_prompt` / hosted 追加段与 `HostPromptSections`，放开 M1 的 `NONE` 限制。向量 (a) 全过。
- [ ] **T2-6** `core/loop.ts`：补 `json` / `json_schema` 回复契约；`run.ts` 补 `XllmTask.prepare`。向量 (c) 全过。
- [ ] **T2-7** `runtime/tool_manager.ts`：AgentToolResult 信封解析、调用内等待（`task_wait_ms`，`check_after` 取小，§2 已裁决 2）、按 call_id 恢复、`hints.ts`（`HintLedger`、`once` / `dedup`、`<runtime_hints>` 渲染、`hints_once` 写 run.json、压缩时清 `dedup`）、`task_model.ts`（`tasks_running` → `paused`）。向量 (f)(g) 全过。
- [ ] **T2-8** `providers/timeout.ts`、`dev/openai.ts`、错误映射补 OpenAI 路径。向量 (d) 全过。
- [ ] **T2-9** jest：`tests/xllm/{config,prompt,loop,providers,resume}.test.ts`；`resume` 用 `exportRun` → 新 `MemoryRunStore` → `importRun` → 续跑到完成。
- 完成条件：`cargo test -p agent_tool --lib xllm` 与 `cargo test -p llm_context` 绿；向量文件已提交；`cargo test -p agent_tool --lib runtime` 含 `site` 报 Capability 一例；`cargo test -p libopendan`、`cargo test -p aiworkspace`（T2-0 改了它们）绿；`pnpm test -- tests/xllm` 绿；`pnpm types` 过。

### M3 core P1

- [ ] **T3-1** behavior 循环：`core/xml.ts`（`scan_xml_elements` / `xml_unescape` / `strip_code_fences`）、`XllmActionParser`（属性 → 参数、正文 / CDATA → 正文参数、“无动作 + `<report end="true">` 即完成”、阶段 report 不结束）、`runtime/step_renderer.ts`（`XmlStepRenderer`，hosted 无时间戳形态）、`handover` 记录；向量 (b) 与 (c) 的 behavior 条目。
- [ ] **T3-2** 上下文压缩（`runtime/compress.ts` 移植 `llm_compress.rs`，向量 (i)）与 `context_window`、`file_model` 阶段（图片 `AiContent` 走 `AiccLlmClient`）。
- 完成条件：jest 绿；`xllm_rust_sdk.md` §4 / §6 的每条在 TS 测试里有对应用例名（写进 `xllm_ts_runtime.md` 的对照表）。

### M4 Deno 宿主：`native` runtime 与同机接手 Rust run

前置：Rust 落地冻结 TODO F6；runtime review 单的新增字段已跟进（那些项用户还没裁决，到这一步前要先问）。Deno 宿主复用 core 和工具注入口，在其上加文件、进程和锁（§0 第 3 条），入口 `buckyos/xllm/deno` 不进浏览器包。

- [ ] **T4-1** `hosts/deno/store.ts`：`FileRunStore`（`<runs_dir>/<run_id>/run.json` + `snapshots/NNNN.json` + `.lock`，布局与原子写法同 Rust §5；`lock_run` / `is_run_live` 用 `Deno.FsFile.lock(true)` / `tryLock`，先在 Deno 2.9 上验证与 Rust fs2 flock 互斥：Rust 持锁时 Deno `RunBusy`，反之亦然）；工作目录锁文件名 = `blake3(workdir)` 前 24 位（`@noble/hashes`），向量 (h) 验证；`~` 展开、相对 `runs_dir` 语义同 Rust；`deno check --config cli/deno.json` 覆盖 `src/xllm/core` 与 `hosts/deno`。
- [ ] **T4-2** `hosts/deno/native_runtime.ts`：`native` kind——descriptor 完全按 Rust：`target = { host: native_host_id(), uid }`，`capabilities = { os, network: true, tools: 探测到的命令 }`，`os` / `uid` / `tools` / `cwd` 用同一段 `LOCAL_PROBE` 脚本（`runtime/ssh.rs`）取得，`runtime_id` 由 blake3 派生，向量 (h) 验证；bash 组 `read_file / write_file / edit_file`（`filesystem_policy: workspace` 的真实路径校验同 Rust）、`shell`（没有任务后端，命令在调用内等到超时——Rust 的“预算 ≥ 超时”情形，`task_wait_ms` 不起作用；`Deno.Command`，执行目录 `runs/<run_id>/exec/<call_id>/{command,stdout,stderr,exit}`，输出头 1/4 + 尾 3/4 保留 64KB，`XLLM_CALL_ID` 环境变量；超时只能结束直接子进程，不能整组 kill——记入 §8 差异）、`describe_interrupted`（读执行目录：有 `exit` 给退出码与输出尾部，没有给“可能仍在运行”与位置）；转任务（需要任务后端）、`bash_core` / task 工具、MCP、具名宿主工具 → 重建工具集时 `NotResumable` / 新任务 `Capability`。Provider：openai 直接；buckyos 用 websdk node runtime 的 `BUCKYOS_APPCLIENT_SESSION_TOKEN` 构造 `getAiccClient()`，没有 session 时 Provider 构造失败即 `paused`（与 Rust 凭据问题同类）。
- [ ] **T4-3** 接手测试（Deno test，`tests/deno/xllm_takeover_test.ts`，用 Q9 (e) 的 fixture run 目录拷到临时 `runs_dir`，`workdir` 按 fixture 重建到临时目录并把 record 的 `workdir` / descriptor 改写为该路径——fixture 自带一个 `rewrite_paths.ts`，`target.uid` 也随机器变化，一并改写；Rust 侧生成 fixture 时把 `host` 固定为 `AGENT_TOOL_HOST_ID=xllm-fixture`，Deno 测试设同一环境变量）：
  - (i) **Rust 中断 → Deno 恢复完成**：`paused` 的 run，Deno `resume` 装回快照、mock 继续给最终答案 → `completed`；Rust 写的文件仍在，`usage.llm_requests` 在原值上累加，`pid` 换成 Deno 的，`latest_snapshot_idx` 递增，run.json 能被 Rust `agent_tool xllm result --run <id>` 读出（有二进制时跑，没有则 skip）。
  - (ii) 停在 shell 调用前 checkpoint、调用无结果的 run：Deno 重新派发时按 call_id 读执行目录得到 `Unresolved { effect_unknown: true }`，继续到 `completed`；说明文本与 Rust `describe_interrupted` 的向量一致。
  - (iii) 拒绝路径各一例：Rust 持锁 → `RunBusy`；`version` 改成 5 → `NotResumable`；descriptor 的 `workdir` 不同 → `RuntimeMismatch`；工具表含 `mcp:` 来源 → `NotResumable`；`host` 非空 → `NotResumable`。
  - (iv) 反向：Deno 跑到 Transient 错误 `paused` 的 run，由 Rust `agent_tool xllm --resume --run <id>` 完成（有二进制时跑）。
- [ ] **T4-4** `scripts/test-xllm-conformance.mjs`：起 OpenAI mock，(a) 同一任务分别由 `agent_tool xllm --no-tools "…"` 与 Deno 宿主跑完，比对 `status / usage.llm_requests / result.raw`；(b) 现场重演 T4-3 (i)：Rust 跑到 mock 的 Transient 错误 → `paused`，同一 `runs_dir` 下 Deno `resume` 到 `completed`。缺 Rust 二进制时 skip 并打印原因。
- 完成条件：`deno test` 与 `pnpm run check:cli` 绿；conformance 脚本在有 Rust 二进制的机器上通过；§8 记录 Deno `shell` 与 Rust 的差异。

### M5 第三步：现有页面简单改造，得到标准 runtime，xllm 代用户操作当前页面

目标（2026-11-07 用户裁决）：基于 DID Object 协议和 RESTful API，让现有页面只需简单改造就能得到一个相对标准的 runtime，并让 xllm 代替用户操作当前页面。用户明确：不模拟 DOM 点击，这里讨论的是**面向 Human-Agent loop 的理想页面形态**。“操作”是调用页面本来就在用的业务接口和页面声明的 action，用页面当前的 session 执行；能力仍不超过页面 JS（§0）。工具形状见 §2.1。

验收口径是下面四条，由用户这句话推出来：

- **人和 agent 用同一套声明。** 页面按钮调用的业务函数，就是声明给 agent 的资源和 action，不另写一份给 agent 用。
- **人看得到 agent 做了什么。** agent 的写操作经页面自己的状态更新反映到界面上，loop 的事件在页面上可见。
- **agent 看得到人改了什么。** `read` 读的是当前状态：人在 run 中途改了内容，agent 下一次 `read` 就能看到。
- **人随时能接管。** 中断经 `interrupter()` 和 `signal` 生效；非只读操作要不要确认，由页面在实现里决定（§2.1）。

前置：按顺序在 M4 之后做；技术上只依赖 M1 和 M2（`xcall` 的 `pending` 结果要用 M2 的 AgentToolResult 信封）。

现状（2026-11-07 源码核对）：

- 服务端已经有把现有 kRPC 方法包成 DID Object 的做法，即 [`KrpcObjectProvider`](../src/kernel/buckyos-api/src/did_object_provider.rs)：按路径模式声明对象类型，属性、action、IndexTrait 查询都调用服务现有的 kRPC 方法，用调用者自己的 session token，所以 RBAC 与 kRPC 方法相同。HTTP 布局是 `/kapi/<service>/obj/<path>`，Profile 在 `?type=profile`。在用的有 [homestation](../src/frame/homestation/src/obj_provider.rs)（`home/{user}`、`feed`、`entry`、`inbox`）和 [opendan](../src/frame/opendan/src/obj_provider.rs)（Agent 的 profile 和集合）。
- 这两个都只声明了属性和 index，**没有 action**。本单不给 provider 加 action，action 由页面在 `resources` 里声明。HomeStation 的产品还要调整很多（用户 11-07），不拿它做验收。
- websdk 里没有 DID Object 的 HTTP 客户端，只有 [`object_url.ts`](../../buckyos-websdk/src/object_url.ts) 解析 `obj://`。

- [ ] **T5-1** `runtime/site_tools.ts`：`siteTools({ resources, commands })`，语义和边界全部按 §2.1：四个工具的 spec、`read /` 发现、`edit` 的组合实现、`exec` 的分词与“不是 shell”报错、`xcall` 和自定义命令的解析与校验、`--help`、按 `risk` / `access` 决定是否重跑、模式匹配与 URI 规范化。同一步把 CLI 的解析纯函数挪到 `src/internal/command_args.ts`，`cli/core` 改为引用，`pnpm test:cli` 通过。
- [ ] **T5-2** DID Object 适配：`didObjectResources({ roots, maxRisk? })` 返回 `siteTools` 的 `resources`。读 `roots` 下的 Profile：属性 → `read`，IndexTrait → 集合的 `read`（query 同 key url §6），action → `xcall`（`risk` 取 Profile 的 `x-buckyos:risk`）。请求走 `/kapi/<service>/obj/<path>`，用页面当前的 session。只能到达页面列出的 `roots`，`risk` 高于 `maxRisk` 的 action 不暴露。它不算 §2.1 禁止的“把 uri 转发给 `fetch`”：能到达什么由 Profile 和 `roots` 决定，不由模型给的 URI 决定。websdk 补一个最小的 DID Object HTTP 客户端（读 Card / Profile、调 action），放在 `src/`，复用 `object_url.ts`。
- [ ] **T5-3** `hosts/browser/store.ts`：`IndexedDbRunStore` + `navigator.locks`。代用户操作页面时可能导航或刷新，run 要能在页面重新加载后恢复。真实浏览器用例加“start → `location.reload()` → resume 完成”。前提：MW 已完成。
- [ ] **T5-4** 验收页，不要太复杂（用户 11-07）：`examples/xllm_site/` 按“现有页面”的样子写一个笔记列表加详情的小页面。数据走页面里一个 REST 风格的小客户端，内存实现，不依赖 Zone；按钮直接调这个客户端。改造前后两份都留在目录里（`before/`、`after/`）。改造只做三件事：用 `siteTools` 把同一个客户端声明成 `notes`、`notes/{id}` 和 `delete` action；加一个 agent 面板；写操作后刷新页面状态。两份的 diff 行数记录到 §8，作为“简单改造”的度量。M5 开头的四条在这个页面上逐条演示，xllm 代用户完成一个先读后写的任务。Desktop 现有页面的改造不在本单。
- [ ] **T5-5** 用例：`tests/xllm/{site_tools,did_object}.test.ts`（provider 用 mock）。覆盖：未声明的 URI、`note/..%2fadmin`、未声明的 action 或命令、`exec "xcall note clear | cat"`（错误里列出可用命令）、`maxRisk` 以上的 action 不出现在 `read /` 里；恢复时 `read` 和 `access: read` 的命令重跑，`write` 和非幂等的 `xcall` 得到 `Unresolved`。
- [ ] **T5-6** 真实 Zone DV（手动，一次）：在 T5-4 的页面上用 `AiccLlmClient` 跑一个要用到四个工具的任务到 `completed`：`read /` 发现资源 → 读 → `write` 或 `edit` 保存 → `exec` 跑一条自定义命令和一次 `xcall`；run 中途人在页面上改一条笔记，确认 agent 后面读到的是改过的。AICC 用量表 `user_id = devtest`、`app` 为该页面身份。另外用 `didObjectResources` 读一次 Zone 里现有 provider 的对象（比如 Agent 的 profile，只读），确认 HTTP 布局和 mock 一致。把 run.json 片段、LLM 轮数和工具误用（读未声明的 URI、该用 `edit` 却用 `write`、在 `exec` 里写管道或 shell 命令之类）记录到 §8，交给用户决定基础工具留不留（§2.1）。
- [ ] **T5-7** 文档：`xllm_ts_runtime.md` 加“现有页面怎么改造”：什么时候把 RPC 封装成 DID Object 走 `xcall`、什么时候自己转成命令、`didObjectResources`、M5 开头的四条、不要写转发 `fetch` 的资源或原样转 kRPC 的命令；示例与 T5-4 的 `after/` 一致。
- 完成条件：§5 的 M5 命令全绿；DV 和 T5-4 的改动行数记录在 §8；用户给出基础工具的去留决定。
- 不在 M5：读取或点击 DOM 的通用工具；Desktop 现有页面（包括 HomeStation）的改造；给 provider 加 action；AiWorkspace 的 runtime（Q10）。

### M6 Worker 宿主（浏览器）

不在三步之内，第三步之后再定做不做（2026-11-07）。许愿格 TODO 的 M3 现在依赖它（§7）。

- [ ] **T6-1** `worker/protocol.ts`（Q5 消息表，带 `protocol_version: 1`）、`worker/worker.ts`（Worker 侧：桥接 `LlmClient` / `ToolManager` / `RunStore` 实现 + 直通 `MemoryRunStore` 与纯 JS 工具注册 `registerWorkerTool(name, spec, fn)`，只放页面自己写的可信函数）、`worker/host.ts`（主线程：`createXllmWorker()`，校验每个 `call`，并发 run 用 `id` 区分，`terminate()`）。
- [ ] **T6-2** 打包：`src/xllm/worker_entry.ts` 入口；`exports` 子路径；`xllmWorkerBlobUrl()` 内联形态（构建时把 worker bundle 文本注入，参照 `hash_worker.ts` 的做法）。
- [ ] **T6-3** 真实浏览器用例 `tests/browser/xllm/worker.spec.js`：页面内起 Worker，`llm` 是页面侧的 `ScriptedLlmClient`，工具一个在页面（`echo`）一个在 Worker（`sum`），跑向量 (c) 中的 3 条 + interrupt 一条 + `exportRun` 后 `resume` 一条 + 白名单外的 `tool.call` 被主线程拒绝一条；结果写 `#result`。
- [ ] **T6-4** 真实 Zone DV（手动，一次）：T1-9 的任务换成 Worker 形态再跑一次，记录到 §8。
- 完成条件：`npx playwright test tests/browser/xllm/` 绿；DV 记录在 §8。

### M7 文档与回写

- [ ] `xllm_ts_runtime.md` 写实施差异与对照表；`llm_context/readme.md` 加“TS 运行时”一节；PRD 不改产品行为，§10 末尾加一行指向 TS 文档。
- [ ] [许愿格浏览器优先 TODO](./aiworkspace-wish-browser-first-todo.md) §0 状态行改“依赖已满足，可开工 M3”。
- [ ] 本文件状态行改“已实施”，memory 记录。

## 5. 验证命令

```bash
# MW（在 buckyos/src/frame/desktop）
npx playwright test --config playwright.aiworkspace.config.ts tests/aiworkspace/program-sandbox.spec.ts tests/aiworkspace/wish.spec.ts

# M1（在 buckyos-websdk）
pnpm test -- tests/xllm
pnpm types && pnpm build
deno test --allow-read tests/deno/xllm_cases_test.ts   # 同一组用例在 Deno 上跑构建产物
npx playwright test tests/browser/xllm/                # 真实浏览器，不依赖 Zone

# 能力边界检查（在 buckyos-websdk，都应为空）
grep -rn "window\.\|document\.\|process\.\|Deno\.\|navigator\.\|location\." src/xllm/core src/xllm/runtime src/xllm/providers
grep -rn "eval(\|new Function\|import(" src/xllm/core src/xllm/runtime src/xllm/providers
grep -l "Deno\.\|child_process" dist/xllm.mjs

# Rust 向量与 Capability（M2，在 buckyos/src）
cargo test -p agent_tool --lib xllm
cargo test -p agent_tool --lib runtime
cargo test -p llm_context
XLLM_VECTORS_REGEN=1 cargo test -p agent_tool --lib xllm::vectors   # 协议文本有意变更时重生成，再 diff 向量

# M4（在 buckyos-websdk）
pnpm run check:cli
deno test --allow-read --allow-write --allow-env --allow-run tests/deno/   # 接手测试（fixture run 目录，不需 Rust 二进制）
node scripts/test-xllm-conformance.mjs   # 需 /tmp/dev-cache-root/cargo-target/debug/agent_tool（`cargo build -p agent_tool_cli_dev`）

# M5（在 buckyos-websdk）
pnpm test -- tests/xllm/site_tools tests/xllm/did_object
pnpm test:cli                                          # 解析函数挪到 src/internal 后 CLI 行为不变
npx playwright test tests/browser/xllm/                # 含 reload 后 resume
```

坑（来自 [[xllm-rust-sdk-impl]] 与 [[wish-impl]]）：本 harness 的 stdin 是 socket，跑 Rust CLI 时显式 `</dev/null`；`pkill -f` 会匹配到自身，用 `pkill -f "[o]penai_mock"` 形式；e2e 的脚本化模型靠 system 文本片段分辨阶段，向量 (c) 的脚本要按**调用序号**而不是文本匹配回复；Rust 的 `serde_json` 不保序，向量里对象键按字典序写，TS 比对前先 canonical 化；本机没有 `rg` 时 `cargo test -p agent_tool --lib` 的 3 个 `grep_tool` 用例会失败，与本单无关。

## 6. 不在本单

- AiWorkspace 的集成（替换 `localExecutor.ts` / `chat.ts`、`llm.map` 走 L1、`run_program` 走沙箱工具）——[许愿格浏览器优先 TODO](./aiworkspace-wish-browser-first-todo.md) M3。MW 例外：它修的是已上线路径，不等集成。
- 其它 Rust xllm 行为改动（本单只有 T2-0 的两项、向量测试、`site` 的 Capability，以及 M4 前置的 F6）。
- SDK 层面的浏览器执行体（OPFS、WebContainer、WebGPU 本地模型之类）、MCP 发现。网站要用可以自己写成工具，同样要过 §0 的判据。
- Agent Session（libopendan）语义：session_input、behavior 切换状态机、`call_behavior`、deferred task。
- `.llm_context` 多层目录合并与外部组文件（Deno 宿主的新任务也只接单个 JSON 配置；接手 Rust run 用的是保存在 run.json 里的有效配置，不重读 `.llm_context`）。
- Node 宿主的接手与锁（无 flock；引入锁库须先确认）；`site` 与 `native` 之间的接手（descriptor 必不相等）；Deno 侧 `shell` 的 `auto` 模式与进程内 task-mgr、远端 runtime（`remote_ssh` / `remote_node`）。
- 一个 TS 的 `xllm` 命令行（PRD 的 CLI 由 Rust 交付；Deno 宿主只为测试、接手与 websdk CLI 内部复用）。

## 7. 对许愿格 TODO 的影响（已同步改写）

- 许愿格 TODO §0 状态加“M3 依赖本单”；M3 的前置是本单 M1、M2、M6（有向量、有 Worker 宿主）和 MW（2026-11-07 Worker 宿主移到第三步之后，许愿格 M3 因此排在本单第三步之后，用户同日同意）；Q1（`llm.map` 谁调模型）的实现载体改为本单 providers 的 L1 推理；G4 / G5（错误分类、预算写死）由本单的错误枚举与 `RunLimits` 解决；M3 的 T3-1 / T3-2 改为“接入 `buckyos/xllm`”，手写循环与 `chat.ts` 删除；`run_program` 不进 xllm Worker，`programHost.ts` 保留为 MW 的沙箱。
- 许愿格 TODO 的 M0 / M1 / M2（服务端：默认本地、`llm_cache`、下线服务内执行）与 M4 的服务端测试迁移不依赖本单，可并行。

## 8. 实施回写（code agent 填写）

- 出入 / 自定裁决：
- 向量数量与来源：
- MW：三种浏览器的结果、TW-1 复现结果、TW-6 DV 记录：
- M1 T1-9 DV 记录：
- M5：T5-4 的页面和改动行数；T5-6 的 LLM 轮数、工具误用、用户对基础工具的去留决定：
- M6 T6-4 DV 记录：
- 遗留：
