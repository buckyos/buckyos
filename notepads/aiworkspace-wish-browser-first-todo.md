# 许愿格浏览器优先 TODO：本地执行成为默认与唯一标准路径，服务内执行下线

日期：2026-10-31

状态：**待开工；M3（Desktop 执行器）依赖 [xllm TypeScript 运行时 TODO](./xllm-ts-runtime-todo.md) 先独立验证通过（2026-10-31 用户裁决：浏览器里的模型循环是通用的 xllm Worker 运行时，不在 AiWorkspace 里手写；独立 TODO 验收后再集成）。M0–M2、M4 的服务端部分不依赖它，可先做。** 2026-11-07：xllm TS 单的 M1、M2、MW（程序 Worker 沙箱）已实施（未提交），M6 Worker 宿主未做，所以 M3 仍不能开工。 依据 [《核心架构设计冻结思路》](<../doc/workspace/BuckyOS AiWorkspace 核心架构设计冻结思路.md>) §1.2（计算扩展执行位置明确）、§7.1 表（“标准许愿格 = 带 AI 的客户端计算流程”，“Agent 型许愿格 = 计划增加”）、§7.2（Get → 客户端计算 → Set 是现有、低内核负担的基本路径）、§8.6（标准模型在当前登录用户的网页中运行；**从 Workspace 后端看，它没有参与 AI 执行过程**；依赖当前用户的 AICC 权限；三种授权分开）、§8.7–8.8（托管模型是第二种、面向互信协作者）、§13 冻结清单 F10 / F13，以及 [《内核架构改进》](<../doc/workspace/BuckyOS AiWorkspace 内核架构改进.md>) §12.3 M3 的已实施状态（T3-1～T3-8 已实施，T3-9 未做）与 §13.5 D14 / D21 / D22。本单只处理**标准模型（浏览器本地执行）**；托管模型（T3-5～T3-7）保持现状，不扩展也不拆。

目标：把今天“可选的本地模式”变成**许愿格的默认且唯一的标准执行路径**——新建 Zone 与新建工作区默认在浏览器执行；服务不再持有进程内 xllm 循环、Deno 子进程与 `wish-host` HTTP 路由（T3-9 提前，条件改为“本地模式在真实 Zone 用浏览器 UI 全程 DV 通过”）；本地模式下**所有模型调用都出自发起者自己的 AICC 会话**（含程序里的 `aiws.llm.map`），服务只做数据内核的事：物化快照、回答 `ws_*` 只读工具、校验程序产物、规划候选、事务 Apply。

## 1. 现状与差距（已对照源码，code agent 直接采信）

### 1.1 已有（不重做）

- **本地执行器已实现（T3-4）**：Desktop [`executor/localExecutor.ts`](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts)（claim → 心跳 = 租约/3 → 模型循环 / 程序 → `proc.report{finished}`；claim token 存 `localStorage['aiworkspace.claims']`，`pagehide` 静默停止，租约到期转 `interrupted` 后同一浏览器可续跑）、[`chat.ts`](../src/frame/desktop/src/app/aiworkspace/executor/chat.ts)（AICC：逻辑模型 `helper.llm_chat`、含 `@` 的精确模型 `chat.completions.create`，用户 session；开发覆盖 `localStorage['aiworkspace.dev'].llm.baseUrl` 走 OpenAI 格式）、[`programHost.ts`](../src/frame/desktop/src/app/aiworkspace/executor/programHost.ts) + [`program.worker.ts`](../src/frame/desktop/src/app/aiworkspace/executor/program.worker.ts)（Worker 内用服务同一份 [`aiws/aiws.js`](../src/frame/aiworkspace/aiws/aiws.js) 跑程序，`Deno` 全局由内存快照模拟，无 token、无 kRPC；D21 形态已定）。
- **服务侧阶段宿主已实现**：[`server/src/wish/stages.rs`](../src/frame/aiworkspace/server/src/wish/stages.rs) `host_open` / `host_tool` / `host_program` / `host_files` / `drive_host`（:914–1083）；[`exec.rs`](../src/frame/aiworkspace/server/src/wish/exec.rs) `proc.claim/heartbeat/report/tool/program/files`、执行上下文、`check_executor`、租约过期 → `interrupted`。
- **协议与测试**：README “Execution modes” 段与方法表（`proc.*` 仍 `experimental`）；e2e [`wish.spec.ts`](../src/frame/desktop/tests/aiworkspace/wish.spec.ts) 第 400 行“T3-4: the local executor”一例通过；服务测试 `server/tests/wish.rs::an_outside_executor_works_on_the_services_stage_through_proc_tool_and_proc_program`；真实 Zone 用 kRPC 脚本驱动过本地协议全程（验收报告 §4.3），**但没有用浏览器 UI 在真实 Zone 跑过一次完整许愿**。
- **UI**：`WishPanel` 的 `aiws-wish-exec-mode` 选择器（列 service / local / hosted，记在 `localStorage['aiworkspace.execution']`）、`aiws-wish-resume` 续跑、仅 `run` 权限者直接走 hosted。

### 1.2 差距（本单要做的）

| # | 差距 | 代码依据 |
| --- | --- | --- |
| G1 | **默认仍是服务内执行**：`default_execution: "service"`、`allow_service_execution: true` | [`buckyos-api/src/aiworkspace_client.rs:175-176`](../src/kernel/buckyos-api/src/aiworkspace_client.rs)；scheduler 种子 [`system_config_builder.rs:630`](../src/kernel/scheduler/src/system_config_builder.rs) 用 `AiWorkspaceSettings::default()`；`WishService.executionMode()` 先取 localStorage 再取 `wish_execution.default` |
| G2 | **本地模式下 `aiws.llm.map` 仍由服务调模型**：Worker `__aiwsLlmMap` → 主线程 → `proc.tool llm_map` → 服务 [`llm.rs::llm_map`](../src/frame/aiworkspace/server/src/wish/llm.rs)（以 run 归属 `on_behalf_of` 调 AICC、缓存在 `<ws>/cache/llm_map/`）。与 §8.6“后端不参与 AI 执行”和“本地 run 归属 = 发起者自己的 session”不一致（模型循环计 `user_id=发起者, app=system:control-panel, mode=direct`，llm.map 计 run_ref + on_behalf_of，两套口径） | `localExecutor.ts:program()`；`stages.rs::host_tool` 的 `llm_map` 分支 |
| G3 | **服务内执行路径仍在**：进程内 xllm（`stages.rs::run_xllm/analyze/execute/rerun`，`agent_tool::xllm` 依赖）、Deno runner（[`runner.rs`](../src/frame/aiworkspace/server/src/wish/runner.rs)）、`POST /kapi/aiworkspace/wish-host/<token>/llm_map`（[`lib.rs:189/851`](../src/frame/aiworkspace/server/src/lib.rs)）、配置项 `deno` / `program_memory_mb` / `stage_timeout_secs` / `program_timeout_secs`、`WishRuntime.stage_by_token`、`ActiveRun.child/interrupter`、`cancel()` 的 `kill(pid)` | T3-9 行内条件“托管 DV 通过后”——本单改为“本地 DV 通过后” |
| G4 | **浏览器侧 AICC 错误丢失分类**：`chat.ts` 把 kRPC 失败当普通 `Error` 抛出，`localExecutor.run()` 报 `EXECUTOR_FAILED{detail}`；服务路径有 `refusal_sub_code` → `AI_QUOTA_EXHAUSTED` 等，Desktop `session.ts::aiRefusalText` 已有文案但本地路径到不了。websdk `RPCError` 带 `code`（[`krpc_client.ts:46-55`](../../buckyos-websdk/src/krpc_client.ts)） | 违反 [[feedback-error-enum-only]]（错误只按枚举判断）。**由 xllm TS 运行时的 `LLMComputeError` / `RefusalKind` 枚举解决**（其 TODO Q6），本单只做 run 报告的映射 |
| G5 | **浏览器侧预算写死**：`PROGRAM_TIMEOUT_MS = 120_000`、`MAX_ITERATIONS = 60` 常量；`stage.iterations` 已由服务下发，超时没有；`llm_map_max_items` 预算只在服务侧 | `localExecutor.ts:15-16`。**循环侧预算（迭代、wallclock、llm 超时）由 xllm TS 的 `RunLimits` 承担**；本单只下发并转交 |
| G11 | **手写模型循环与 xllm 分叉**：`localExecutor.loop()` + `chat.ts` 是一套不含预算枚举、错误分类、快照、`runtime_protocol`、用量记录的 function_call 循环，与 Rust xllm 语义不一致 | 2026-10-31 裁决：浏览器模型循环统一用 [xllm TS 运行时](./xllm-ts-runtime-todo.md)（Worker + postMessage 桥），本单 M3 改为接入 |
| G6 | **快照整包下行**：`proc.files` 把 `context/`、`catalog/`、`lib/`、`request.json`、`WORKSPACE.md` 一次性 text/base64 返回（单文件 > 16 MiB 跳过），浏览器 Worker 同步 `Deno.readTextFileSync` 必须预取。大表格快照（`rows.jsonl`）可能数十 MiB | `stages.rs::host_files`；`program.worker.ts` 的 `deno` 垫片 |
| G7 | **AICC 不可用（Guest / 无额度）在点击后才失败**：§8.8 的“Guest 没有 AICC”情形没有前置提示；只有 `noSession` 一条文案 | `chat.ts::AiccChat.complete` |
| G8 | **归属设置与本地模式矛盾**：`ws.set_wish_aicc { mode: workspace_owner \| current_user }`（`wish_aicc_mode`，面板 `aiws-wish-aicc` 显示）只对服务内执行有意义；本地固定 `force_current_user`，托管固定 Owner | `mod.rs::run_attribution(.., force_current_user)`；`lib.rs:599` |
| G9 | **e2e / 服务测试绑定服务内执行**：`wish.spec.ts` 其余 7 例用默认（service）模式；服务 `tests/wish.rs`（13 例，27 处）、`wish_control.rs`（12 例）、`quality.rs`（15 例）经 `common/wishkit.rs` 的 `ScriptedModel` + `run_test` 驱动进程内 xllm；`tests/runner.rs` 依赖本机 Deno | T3-9 必须先把这些测试改成“测试侧扮演外部执行器”形态 |
| G10 | **文档**：README 方法表 `proc.*` 为 `experimental`（T0-3 约定“M3 结束升稳定”）；许愿格详细设计 §9.3 仍写“规范运行器在服务侧”“浏览器离线运行不进首版”；§19.2 `llm.map` 回调写 HTTP 路由 | 需回写 |

## 2. 本单裁决项 Q1–Q6（**建议值；用户未另行裁决时按建议执行**）

| # | 问题 | 建议 | 理由 / 代码依据 |
| --- | --- | --- | --- |
| Q1 | **本地模式的 `aiws.llm.map` 谁调模型** | **A：浏览器调**。把 `llm.rs::llm_map` 的逻辑（分批 ≤ 50、schema 子集校验、失败项逐个重试、项数预算、`calls/cached` 统计）移植为 Desktop `executor/llmMap.ts`，模型调用用 `buckyos/xllm` providers 的 `LlmClient.infer({ json_schema })`（L1 直接推理，xllm TS TODO Q10；用户 session；模型 = 服务下发的 `map_model`）。**缓存仍在服务侧但只是 KV**：新增阶段工具 `llm_cache { get: [keys] }` / `llm_cache { put: [{ key, value }] }`，键 = `sha256(canonical({ m, i, s, x }))` 与今天同构，服务只存取 `<ws>/cache/llm_map/<key>.json`，不调模型。B（保留服务调、改为发起者 `on_behalf_of`）成本低但服务仍持有模型客户端，与 §8.6 冲突 | 一致的归属口径（全部出自发起者 session）；服务不再需要 `agent_tool::xllm` 的 LLM 客户端；缓存语义（§9.5“只重跑程序只对新增项调模型”）不丢。托管模式的 Agent 今天用 Deno 跑 `lib/run.js` 而 CLI 没有设置 `AIWS_HOST`，其 `llm.map` 本就不可用——本单不修，记入 §7 |
| Q2 | **T3-9 的下线条件** | **改为“本地模式真实 Zone 浏览器 UI 全程 DV 通过”**，不再等托管 DV | 冻结思路 §7.1：标准模型是“已有实现”，托管是“计划增加”；服务内路径只是过渡（内核架构改进 §13.5 D14 建议段）。托管 DV 仍是 T3-5 的事，与本单解耦 |
| Q3 | **配置项取舍** | 删 `allow_service_execution`；`default_execution` 保留，取值 `local \| hosted`，默认 `local`；`service` 或未知值 → 当 `local` 并 `warn!` 一行（这是配置校验，不是兼容分支）。删 `deno`、`program_memory_mb`、`stage_timeout_secs`；`program_timeout_secs`、`analyze/execute_tool_iterations`、`llm_map_max_items`、`max_output_tokens`、三个模型名保留并**随 stage 下发**给执行器 | DV 全新安装（[[feedback-dv-fresh-install]]）；scheduler 种子 `insert_json_if_absent`，老 Zone 的 `"service"` 走校验回退 |
| Q4 | **`ws.set_wish_aicc` / `wish_aicc_mode`** | **删除**（RPC、store 字段、`ws.get_info.wish_aicc_mode`、面板 `aiws-wish-aicc` 的“模式”文案）。归属规则写死：本地 = 发起者；托管 = 执行环境所有者（Owner）。`run_attribution` 去掉 `force_current_user` 参数，按 `Mode` 决定 | G8；冻结思路 §8.6/8.7 三种授权分开，归属跟执行位置走不是工作区设置；契约 fixture 用 `AIWS_REGEN=1` 重录 |
| Q5 | **AICC 不可用的前置处理（§8.8 Guest 问题）** | 只做最小前置检查与引导：打开面板时若当前身份没有 session（guest / 公共端）或 `aicc` 服务探测失败（一次轻量 `helper.llm_chat` 不做，用 `ws.get_info` 之外的现成账号信息判断 principal_kind），在执行按钮旁显示“本浏览器无法调用模型”并（若 `hosted_exec.enabled`）引导走托管；Guest 自带 Endpoint / 跨 Zone AICC **不做**（§8.8 待设计） | [[feedback-capability-before-security]]：先把主路径做顺；不发明凭据传递协议 |
| Q6 | **快照下行预算（G6）** | `proc.files` 加 `limit_bytes`（服务配置 `snapshot_download_max_bytes`，默认 64 MiB）：超限返回 `SNAPSHOT_TOO_LARGE{ bytes, limit }`，面板提示“输入过大，请缩小范围（表格用视图）或改用托管执行”；不做分页 / 按需读取（Worker 是同步文件 API，按需读要重写 `aiws.js` 读路径，收益不明） | 错误枚举化；大数据本就是托管 / 批处理的场景（冻结思路 §7.4） |

## 3. 当前代码与接入点

| 范围 | 现状 → 改动 |
| --- | --- |
| [`buckyos-api/src/aiworkspace_client.rs`](../src/kernel/buckyos-api/src/aiworkspace_client.rs) `AiWorkspaceWishSettings`（:137–180） | 删 `deno` / `program_memory_mb` / `stage_timeout_secs` / `allow_service_execution`；`default_execution` 默认 `"local"`，注释改为 `local \| hosted`；新增 `snapshot_download_max_bytes: u64`（Q6） |
| [`scheduler/src/system_config_builder.rs:630`](../src/kernel/scheduler/src/system_config_builder.rs) | 无改动（用 default）；`main.rs:499` 的断言不变 |
| [`server/src/wish/mod.rs`](../src/frame/aiworkspace/server/src/wish/mod.rs) `WishConfig`（:40–130）、`start`（:384）、`run_attribution`（:322）、`cancel`（:557）、`ActiveRun`（child / interrupter / `stage_by_token`） | 配置项同步删增；`start` 去掉 `Mode::Service` 分支与 `allow_service_execution` 检查，`stages::drive` 只剩 Mock（`PROGRAM_MOCK`）路径；`run_attribution(ws, caller, mode)`；`cancel` 去 `kill(pid)` / `interrupter`；`WishRuntime.stage_by_token` 删 |
| [`server/src/wish/exec.rs`](../src/frame/aiworkspace/server/src/wish/exec.rs) `Mode`（:18–40）、`execution_context`（:48）、`new_exec_record`（:90） | `Mode` 去 `Service`；`claim` 返回的 stage 带预算（Q3）；`check_executor` 不变 |
| [`server/src/wish/stages.rs`](../src/frame/aiworkspace/server/src/wish/stages.rs) | 删 `run_xllm`（:253）、`analyze`（:367）、`execute`（:545）、`rerun`（:653）及其 `RunObserver`（:216–250）；保留 `open_stage`、`analyze_user`、`execute_prepare`、`rerun_prepare`、`*_tail`、`mock`、`provided_analysis` 与整个阶段宿主段（:908–1083）；`host_open` 的 brief 增加 `budget { program_timeout_secs, llm_map_max_items, map_model, max_output_tokens }`；`host_tool` 增 `llm_cache`（Q1），删 `llm_map` 分支；`host_files` 加预算（Q6） |
| [`server/src/wish/llm.rs`](../src/frame/aiworkspace/server/src/wish/llm.rs) | **整文件删**（`validate` / `extract_json` 在 tools.rs、stages.rs 均无引用，只服务于 `llm_map`；schema 子集校验逻辑移植到 Desktop `executor/llmMap.ts`，其单元测试 `schema_subset_and_json_extraction` 改写为 vitest）。`Cargo.toml` 去 `agent_tool` 依赖（先 `cargo tree -i agent_tool -p aiworkspace` 确认无其他用途） |
| [`server/src/wish/runner.rs`](../src/frame/aiworkspace/server/src/wish/runner.rs)、[`server/tests/runner.rs`](../src/frame/aiworkspace/server/tests/runner.rs) | 整文件删 |
| [`server/src/wish/tools.rs`](../src/frame/aiworkspace/server/src/wish/tools.rs) | `Kind::RunProgram` 保留（只作为模型工具目录项，执行在执行器）；`blocking_call` 中 `RunProgram` 分支改为固定回“在执行器运行并经 proc.program 交回”（今天 `host_tool` 已这么答） |
| [`server/src/lib.rs`](../src/frame/aiworkspace/server/src/lib.rs) | 删 `wish-host` 路由（:189）与 `wish_llm_map`（:851）；删 `ws.set_wish_aicc`（:599）；`ws.get_info` 去 `wish_execution.service` 与 `wish_aicc_mode`，加 `wish_execution.default`（已有）；方法表 `proc.*` 全部升 `stable`（README 方法表与 `METHOD_LEVELS` 有镜像测试）；`RUN_TOKEN_METHODS` 不变 |
| [`server/src/main.rs`](../src/frame/aiworkspace/server/src/main.rs) | `--wish-config` 帮助文案去 `deno`；`WishConfig::from_settings(.., json!({type: buckyos}))` 的 provider 参数随 llm.rs 删除一并去掉；线程栈 8 MiB 的注释（:56）若只为 xllm 则一并评估 |
| [`store/src/wish/runs.rs`](../src/frame/aiworkspace/store/src/wish/runs.rs) `wish_run_expire_lease`（:183） | `exec.mode == "service"` 判断删 |
| [`store/src/workspace.rs:797-803`](../src/frame/aiworkspace/store/src/workspace.rs) `wish_aicc_mode` / `set_wish_aicc_mode` 及 `WISH_AICC_CURRENT_USER` 常量 | 删（Q4） |
| [`aiws/aiws.js:333-351`](../src/frame/aiworkspace/aiws/aiws.js) `llm.map` | 删 `host.hostUrl` HTTP 分支，只留 `globalThis.__aiwsLlmMap` 桥；无桥时报“llm.map 只在许愿运行中可用”。`AIWS_HOST` / `AIWS_TOKEN` 读取（:203-204）删。README 同步 |
| Desktop [`executor/localExecutor.ts`](../src/frame/desktop/src/app/aiworkspace/executor/localExecutor.ts) | **`loop()` 删除**，改为 `buckyos/xllm` 的 `createXllmWorker()`：stage brief → hosted 任务（映射见 xllm TS TODO Q10：`llm_context = { provider: { type: "buckyos" }, model, loop_model: "function_call", max_tool_iterations: budget.iterations, timeout: budget.stage_timeout_secs, tools: { enabled: true, tools: stage.tools.map(t => ({ name: t.name })) } }`，`hostSystem = stage.system`，`host_prompt_sections = NONE`）；stage 工具作为具名宿主工具注入，主线程转 `proc.tool`；`run_program` 是主线程工具，程序由 `programHost.ts` 放进沙箱运行（xllm TS TODO MW），不进 xllm Worker；`program()` 的 `llmMap` 改为本地 `runLlmMap(req, budget, llm, cache)`（新文件 `executor/llmMap.ts`，`llm` 是 providers 的 `LlmClient`）；失败分类（G4）：xllm `RunOutcome` 的 `last_error.kind` 映射为 `EXECUTOR_FAILED` 之外的子码 `AI_QUOTA_EXHAUSTED` / `AI_QUOTA_BUDGET_EXCEEDED` / `AI_RESOURCE_NOT_PERMITTED` / `AI_UNAVAILABLE`，`limit_reached` → `ITERATIONS_EXHAUSTED` / `TIMEOUT`，`proc.report{error:{code, detail, aicc}}`，服务原样落 `detail.error`，面板用 `aiRefusalText`；`interrupted` / `exportRun` 用于关标签页后的续跑（xllm TS TODO Q8） |
| Desktop [`executor/chat.ts`](../src/frame/desktop/src/app/aiworkspace/executor/chat.ts) | **删除**（AICC / OpenAI 适配由 `buckyos/xllm` providers 提供；dev 覆盖 `localStorage['aiworkspace.dev'].llm.baseUrl` 改为构造 `OpenAiLlmClient`）；Q5 的可用性探测改为检查当前身份 principal_kind |
| Desktop [`ui/wish/WishService.ts`](../src/frame/desktop/src/app/aiworkspace/ui/wish/WishService.ts) `executionModes`（:240）、`executionMode`（:256） | 去 `service`；默认顺序：`runOnly` → hosted；否则 localStorage 记忆 → `wish_execution.default` → `local` |
| Desktop [`ui/wish/WishPanel.tsx`](../src/frame/desktop/src/app/aiworkspace/ui/wish/WishPanel.tsx) | `EXECUTION_LABEL` 去 service；`aiws-wish-aicc` 改为只显示本次 run 的归属（`aicc.user_id`）；Q5 的不可用提示 `aiws-wish-no-aicc`；Q6 的 `SNAPSHOT_TOO_LARGE` 文案 |
| Desktop [`api/types.ts:48`](../src/frame/desktop/src/app/aiworkspace/api/types.ts)、[`api/session.ts::errorCodeText`](../src/frame/desktop/src/app/aiworkspace/api/session.ts) | `wish_execution: { default: 'local' \| 'hosted' }`；加 `SNAPSHOT_TOO_LARGE`、`AI_UNAVAILABLE` 文案；[`i18n/aiworkspace/wish.ts`](../src/frame/desktop/src/i18n/aiworkspace/wish.ts) en + zh 同步（删 `execution.service`） |
| [`schemas/error-codes.json`](../src/frame/aiworkspace/schemas/) | 加 `SNAPSHOT_TOO_LARGE`；`EXECUTOR_FAILED` 子码说明 |
| 测试：[`server/tests/common/wishkit.rs`](../src/frame/aiworkspace/server/tests/common/wishkit.rs)、`tests/wish.rs`、`wish_control.rs`、`quality.rs`；Desktop [`tests/aiworkspace/wish.spec.ts`](../src/frame/desktop/tests/aiworkspace/wish.spec.ts)、[`mock-llm.mjs`](../src/frame/desktop/tests/aiworkspace/mock-llm.mjs)、[`start-backend.mjs`](../src/frame/desktop/tests/aiworkspace/start-backend.mjs)；DV [`tests/dv/aiworkspace.journeys.spec.ts`](../src/frame/desktop/tests/dv/aiworkspace.journeys.spec.ts) | 见 M4 |
| 文档：[`frame/aiworkspace/README.md`](../src/frame/aiworkspace/README.md)（“Execution modes”:840、方法表 :1141–1148、HTTP 表 :1178）、许愿格详细设计 §9.3 / §9.5 / §13.3 / §19、内核架构改进 §12.3 T3-9 行与 M3 进度注、验收报告 §6、手册 [`aiworkspace_run.md`](../src/apps/jarvis_agent/agent/cmd_skills/aiworkspace_run.md) | 见 M5 |

## 4. 里程碑

依赖顺序 M0 → M1 → M2 → M3 → M4 → M5；M1 与 M2 内部可并行。每完成一项在本文件行首标“**已实施（日期）**”，出入写进 §7。

### M0 契约（只改协议文件，不写实现）

- [ ] README “Execution modes”改写：`local`（默认）与 `hosted` 两种；删除 `service` 段；`proc.claim` 返回的 `stage` 增加 `budget` 字段说明；`proc.tool` 工具表删 `llm_map` 加 `llm_cache { get } \| { put }`；`proc.files` 增 `SNAPSHOT_TOO_LARGE`；HTTP 表删 `wish-host`；方法表 `proc.*` 改 `stable`。
- [ ] `schemas/error-codes.json` 加 `SNAPSHOT_TOO_LARGE`；若有 `settings` schema / 文档，删四个配置项、加 `snapshot_download_max_bytes`。
- [ ] 许愿格详细设计加 §9.3 修订（“规范运行器 = 执行器侧：浏览器 Worker 或 Agent 的 Deno；服务不运行程序”）与 §9.5（缓存经 `llm_cache` 工具）；在 §19.2 把“`llm.map` 回调 HTTP 路由”一条标为已废弃并指向本单。
- 完成条件：README 方法表镜像测试（`server/tests/contract.rs` / `METHOD_LEVELS`）在 M2 后为绿；本里程碑只改文本。

### M1 服务端：默认本地、`llm_cache`、预算下发、归属简化

- [ ] **T1-1** `AiWorkspaceWishSettings` 按 Q3 增删；`WishConfig` / `apply_json` 同步；`default_execution` 解析：`local \| hosted`，其他值 `warn!` 后作 `local`（`exec::Mode::parse` 返回 `None` 时由 `start` 负责回退，只回退“缺省值”，请求显式给 `service` 仍是 `INVALID_OPERATION unknown execution mode`）。
- [ ] **T1-2** `exec::Mode` 去 `Service`；`start` 去服务内分支；`wish.mock@1` 仍走 `stages::drive`（Mock 结果由服务规划，与第二期 D8 一致）；`new_exec_record` 总是发 claim token。
- [ ] **T1-3** `run_attribution(ws, caller, mode)`：`Local` → 发起者（要求发起者是本 Zone Account，否则 `AI_ATTRIBUTION_UNAVAILABLE`，文案已有 `noOwnResources`）；`Hosted` → Owner。删 `ws.set_wish_aicc`、store 的 `wish_aicc_mode`、`ws.get_info.wish_aicc_mode`、`ws.get_info.wish_execution.service`（Q4）。
- [ ] **T1-4** `host_open` brief 加 `budget { program_timeout_secs, llm_map_max_items, map_model, max_output_tokens, iterations }`（`iterations` 今天已在顶层，保留一处即可）。
- [ ] **T1-5** `host_tool` 新增 `llm_cache`：`{ get: [key…] }` → `{ hits: { key: value } }`；`{ put: [{ key, value }] }` → `{ ok }`；键格式校验 `^[0-9a-f]{64}$`，单次 ≤ 2000 键、单值 ≤ 64 KiB；目录仍 `<ws>/cache/llm_map/`；`StageState.note_map(calls, cached)` 改为由执行器在 `proc.program` 的 `results.llm_map` 统计中带回（`aiws.js` `summary()` 今天已写 `llm_map: this.llmStats`，服务从 `results.json` 读）。删 `host_tool` 的 `llm_map` 分支。
- [ ] **T1-6** `host_files` 加 Q6 预算：累计字节超 `snapshot_download_max_bytes` → `SNAPSHOT_TOO_LARGE{ bytes, limit }`，并把 run 置 `failed`（`detail.error` 同码）。
- 完成条件：`cargo test -p aiworkspace`（M4 改测试前允许 wish/quality/wish_control 失败，其余全绿）；`server/tests/wish.rs::an_outside_executor_works_on_the_services_stage_through_proc_tool_and_proc_program` 扩展覆盖 `llm_cache` 与 `budget`。

### M2 服务端：下线服务内执行（T3-9）

- [ ] **T2-1** 删 `stages.rs` 的 `run_xllm` / `analyze` / `execute` / `rerun` / `RunObserver` / `program_env` 等只为进程内执行服务的代码；`drive_inner` 只剩 Mock 与 `provided_analysis`。
- [ ] **T2-2** 删 `runner.rs`、`tests/runner.rs`；`llm.rs` 按 §3 裁剪；`cancel` 去 `kill` / `interrupter`；`ActiveRun` 去 `child` / `interrupter`；`WishRuntime` 去 `stage_by_token`；`lib.rs` 删 `wish-host` 路由与 `wish_llm_map`；`Cargo.toml` 去 `agent_tool`（确认后）。
- [ ] **T2-3** `aiws.js` 删 HTTP 分支与 `AIWS_HOST/AIWS_TOKEN`；`aiws/README.md` 与 `aiws.d.ts` 同步；`cd frame/aiworkspace/aiws && deno test -A`。
- [ ] **T2-4** README 方法表 `proc.*` 升 `stable`，`METHOD_LEVELS` 同步；契约 fixture `AIWS_REGEN=1 cargo test -p aiworkspace --test contract` 重录（`ws.get_info` 形状变了）。
- 完成条件：`rg -n "xllm|Deno|deno|wish-host|AIWS_HOST|allow_service_execution|wish_aicc" src/frame/aiworkspace/server src/frame/aiworkspace/store src/frame/aiworkspace/aiws src/kernel/buckyos-api/src/aiworkspace_client.rs` 只剩注释里的历史说明或托管手册引用；三 crate 编译通过。

### M3 Desktop：浏览器执行器接入 xllm TS 运行时（**前置：[xllm TS 运行时 TODO](./xllm-ts-runtime-todo.md) M1、M2、M6（Worker 宿主；2026-11-07 起排在那边第三步之后）与 MW 完成、`buckyos/xllm` 已发布到 Desktop 依赖的 websdk 版本**）

- [ ] **T3-0** 升级 Desktop 的 `buckyos` 依赖到含 `buckyos/xllm` 与 `buckyos/xllm/worker` 的版本；Vite 下 Worker 入口用 `new Worker(new URL('buckyos/xllm/worker', import.meta.url), { type: 'module' })` 或 `xllmWorkerBlobUrl()`（按打包结果择一，记入 §7）。
- [ ] **T3-1** `executor/llmMap.ts`：移植 `llm.rs::llm_map`（分批 `batch` 默认 20、上限 50；`MAP_SYSTEM` 同文；schema 子集校验移植 `validate`——`type` / `enum` / `type: [..]` / `null`；失败项逐个重试；项数预算 `budget.llm_map_max_items` 超限返回 `{ error }`；返回 `{ results, calls, cached }`）；缓存：先 `proc.tool llm_cache{get}` 批量查，命中跳过，新结果 `llm_cache{put}`；模型 = `budget.map_model`，模型调用 = providers `LlmClient.infer({ json_schema })`（AICC 走用户 session；dev 覆盖用 `OpenAiLlmClient`——`mock-llm.mjs` 已按 system 含 `item-by-item judge` 回 `mapJudge`）。
- [ ] **T3-2** `localExecutor.ts`：按 §3 行改写——`createXllmWorker()` 跑模型阶段，stage 工具经桥转 `proc.tool`，`run_program` 是主线程工具，程序走 MW 的沙箱（`programHost.ts`），`program()` 用 T3-1；超时 / 迭代走 xllm `RunLimits`（`budget.stage_timeout_secs` / `budget.iterations`，去常量）；`RunOutcome` → `proc.report{ finished }` 或 `{ error: { code, detail, aicc: { user_id } } }`（G4 子码表见 §3）；`chat.ts` 删除；`SNAPSHOT_TOO_LARGE` 原样上抛给面板；关标签页：`interrupt()` + `exportRun()` 存 `localStorage['aiworkspace.claims']` 旁，续跑时 `resume`。
- [ ] **T3-3** `WishService` / `WishPanel`：去 `service` 模式；默认 `local`；`aiws-wish-aicc` 只显示 run 归属；Q5 的 `aiws-wish-no-aicc` 提示与托管引导（仅当 `hosted_exec.enabled`）；`aiws-wish-exec-mode` 在没有 hosted 时隐藏（只剩一个选项不必显示）。
- [ ] **T3-4** i18n en + zh：删 `execution.service`，加 `noAicc` / `snapshotTooLarge` / `aiUnavailable`；`pnpm check`、eslint、`pnpm build`。
- 完成条件：Desktop 源码中不再有自写的模型循环（`grep -rn "chat.completions.create\|helper.llm_chat" src/app/aiworkspace` 只剩 `llmMap.ts` 经 providers 的调用，或为空）；e2e “T3-4: the local executor”通过且不再 `selectOption('local')`（默认即本地）；新增一例覆盖 `llm.map` 走浏览器 + 缓存命中（第二次只重跑程序 `calls = 0`），以及 `SNAPSHOT_TOO_LARGE`（`start-backend.mjs` 的 `wish.json` 设极小 `snapshot_download_max_bytes` 的单独用例或 `test.describe` 内覆盖配置）。

### M4 测试迁移

- [ ] **T4-1** `server/tests/common/wishkit.rs` 增加“测试侧执行器” `BrowserLikeExecutor`：对一个 run 执行 `proc.claim` → 按 `stage.kind` 用 `ScriptedModel` 产出工具调用 → `proc.tool` / 本地用 Deno 跑 `lib/run.js`（测试进程可用本机 Deno；`find_deno` 逻辑搬到 wishkit）→ `proc.program` → `proc.report{finished}`；`llm.map` 由测试侧按同样脚本答并走 `llm_cache`。
- [ ] **T4-2** 把 `tests/wish.rs`（`analyze_execute_apply_rerun_feedback`、`llm_map_cache_mock_and_cancel`、`wish_runs_use_the_owners_or_the_starters_aicc` → 改为“本地 = 发起者 / 托管 = Owner”两例、`restart_reports_interrupted_and_reconciles_applying` 等）、`wish_control.rs`、`quality.rs` 全部改经 T4-1；删除只验证进程内 `.llm_context` 文件的断言（`llm_context_of`）。保持 §11.2 场景 → 测试名对照（验收报告 §5）不缺项。
- [ ] **T4-3** Desktop e2e：`wish.spec.ts` 其余 7 例在默认（本地）模式下跑通；`start-backend.mjs` 的 `wish.json` 去 `program_timeout_secs: 60` 以外的 Deno 相关项；`mock-llm.mjs` 中“service takes the program as run_program's source”注释改正；`AIWS_E2E_LLM_PORT` 的 CORS 保留。全套 `AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts` 与 10-23 基线对比（161/170，7 个已知失败）。
- [ ] **T4-4** DV 用例：`tests/dv/aiworkspace.journeys.spec.ts` 的许愿流程从 `mock` 执行器改为真实 `xllm` + 本地执行（或新增一例保留 mock 例）；跑法见 [[aiworkspace-kernel-m0-impl]]（本机 Zone `test.buckyos.io`，devtest / bucky2025；服务二进制 cp 到 `.new` 再 mv，`pkill -f "^/opt/buckyos/bin/aiworkspace/aiworkspace"`；Desktop `rsync --delete rootfs/bin/control-panel/web/`）。**验收口径**：季度经营分析任务在浏览器 UI 分析 → 执行 → 应用 → 只重跑程序 → 反馈轮全程通过；AICC 用量记录全部 `user_id = 发起者`、无 `system:aiworkspace` 条目（`sqlite3 /opt/buckyos/local/task-manager/task-mgr-main.db` 或 AICC 用量表核对）；关闭标签页 → `interrupted` → 续跑 attempt 2。
- 完成条件：三 crate 全绿（`-- --test-threads=1`）；e2e 不低于基线；DV 报告写入验收报告新节。

### M5 文档与手册

- [ ] 内核架构改进 §12.3：T3-9 行首标已实施、条件改写；M3 进度注加“2026-xx-xx 浏览器优先”段；D14 / D21 “决定”段补一句“服务内执行已下线”。
- [ ] 验收报告 §6 去 T3-9 未完成项，加本单 DV 结果；§5 对照表更新测试名。
- [ ] 许愿格详细设计 §19 加 19.4“浏览器优先（本单）”：执行位置、llm.map、归属、删除的配置；§13.3 配置表同步。
- [ ] 冻结思路 §7.1 表“标准许愿格”一行状态可由“已有实现”补注“默认执行位置 = 浏览器（2026-xx）”；§8.6 末尾加“实现注”指向本单（文档性质是思路整理稿，只加注不改正文）。
- [ ] 手册 `aiworkspace_run.md`：删除对 `AIWS_HOST` 的任何暗示（今天没有，确认即可）；在“Protocol”处写明 `llm.map` 在 Agent 侧暂不可用（§7 遗留）。
- [ ] 本文件状态行改“已实施”，并在 memory 记录。

## 5. 验证命令

```bash
# 服务（在 buckyos/src）
cargo test -p aiworkspace-core -p aiworkspace-store -p aiworkspace -- --test-threads=1
AIWS_REGEN=1 cargo test -p aiworkspace --test contract   # 契约 fixture 重录（ws.get_info 形状变化）
cd frame/aiworkspace/aiws && deno test -A
# Desktop（在 buckyos/src/frame/desktop）
pnpm check && pnpm exec eslint src/app/aiworkspace && pnpm build
AIWS_BIN=/tmp/dev-cache-root/cargo-target/debug/aiworkspace pnpm exec playwright test --config=playwright.aiworkspace.config.ts tests/aiworkspace/wish.spec.ts
# 残留检查
rg -n "xllm|deno|wish-host|AIWS_HOST|allow_service_execution|wish_aicc|execution\.service" src/frame/aiworkspace src/frame/desktop/src/app/aiworkspace src/kernel/buckyos-api/src/aiworkspace_client.rs
```

坑（来自 [[wish-impl]] / [[aiworkspace-kernel-m0-impl]]）：serde_json 不保序；e2e 别断言新结果 Block 可见，用 `toBeAttached`；mock-llm 用 system 中的“the **analysis stage**”区分阶段；`erasableSyntaxOnly` 禁止构造器参数属性；Bash 工作目录会被重置，命令里用绝对路径；等服务重启别用 `ps | grep 路径`。

## 6. 不在本单

- **xllm TypeScript 运行时本身**（core / providers / Worker 桥 / 向量 / 真实浏览器用例）——[独立 TODO](./xllm-ts-runtime-todo.md)，本单只是它的第一个集成方。
- 托管执行模式（T3-5 真实 Zone DV、Agent 侧 `llm.map`、XL Agent）；用户后台模式 / Node Executor。
- Guest 自带模型 Endpoint、跨 Zone AICC（冻结思路 §8.8、§14.3“AI 能力来源”）。
- 许愿格“新建数据源”边界（§8.5）、执行权限 ≠ 编辑权限的细化（§8.6 三种授权今天只有 `run` 能力 + 输出声明）。
- Mock 执行器（`HtmlExecutor`，页面主上下文）形态不动，仍只用于测试与演示。
- 画布上的输入依赖箭头 / 预览层（§8.1，详细设计 §19.2 “预览在面板中”）。
- Worker 内存上限（浏览器无此能力，接受差异；程序超时仍有）。

## 7. 实施回写（code agent 填写）

- 出入 / 自定裁决：
- 遗留：托管模式 Agent 跑 `lib/run.js` 时 `aiws.llm.map` 不可用（CLI 未提供桥；应改为 CLI 侧用 Agent 自己的 AICC 实现 `__aiwsLlmMap` 等价桥 + `llm_cache`），随 T3-5 DV 一并处理。
