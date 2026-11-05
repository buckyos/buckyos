# 媒体表示管线跑在 Workflow Service 上（系统内置 pipeline）TODO

日期：2026-11-05
来源：对 [LLM Understand Media](../doc/llm_context/LLM%20Understand%20Media.md) v0.4 §4.5–§4.8 的 review 与用户三次裁决（见 §0）。本文承接设计文档的 T1–T6，但把"本地 runner + 复用 preview 缓存"改为"Workflow Service 的系统内置 pipeline"。设计文档 §4.5 / §4.8 需按本文回写（T0）。
基线：`src/kernel/workflow` 已有 DSL、compile / analyze、Orchestrator、ExecutorRegistry（只注册了 `service::aicc.*`）、RPC 前端与 TaskMgr 镜像，50 个单测全部通过且不依赖 Zone（`cargo test -p workflow`，0.01s）；但服务进程内全部状态都是内存实现（`DefinitionStore` / `RunStore` / `InMemoryObjectStore` / `InMemoryThunkDispatcher`，[main.rs:180-181](../src/kernel/workflow/src/main.rs#L180)），执行在 RPC handler 内同步完成（§1.1）；`llm_understand_media` 已有图片缩图、视频抽帧、音频转写、文档直传；nfs_server preview 有 `work_key` / `ensure` / 工具探测的完整样板。路径相对 `buckyos/`。

定位（2026-11-05 第二轮补充）：本文是一系列 Agent Runtime 改造任务的前置，同时是 workflow service 第一个真实业务负载——借它把这个系统服务跑通、用起来。**workflow 侧的全部工作（§4 Phase A）不依赖任何 OpenDAN 组件**（agent_tool / llm_context / libopendan / opendan），可独立开发、独立测试、独立验收；agent 侧接入（T4）与 WebUI（T5）是 Phase B。依赖方向见 §2.2。

目标：**一次格式转换 = 一次 Workflow Run**，输入是内容寻址的对象，产物是 NDN 对象，同样的转换在整个 Zone 内只算一次；`llm_understand_media` 只消费管线产物再做模型分析。管线定义随系统发布，在 Workflow WebUI 中只读可见，系统管理员能看到每一次转换走到了哪一步。

## 0. 用户裁决（2026-11-05）

| # | 裁决 | 直接推论 |
| --- | --- | --- |
| P1 | §4.8 的意图是复用 Preview Pipeline 的**处理模型**（PRD §9.7 workKey、§23.2 ensure、§23.4 descriptor、§23.6 状态机），不是去调 preview 组件：preview 的目标是浏览器可用，本管线的目标是 LLM 可用，Target Profile 不同。 | 不需要 NFSP 客户端；不扩 `repr` wire；不进 `preview-cache` 桶。 |
| P2 | Pipeline 的定义不另造轮子，用 [workflow service](../doc/workflow/workflow%20service.md) 已有设施。 | 管线 = `WorkflowDefinition`；一次转换 = 一个 Run；stage = `autonomous` Step；跨 Run 复用 = Orchestrator 已有的 idempotent cache（[orchestrator.rs:556](../src/kernel/workflow/src/orchestrator.rs#L556)）。设计文档 §4.5"复用 DSL 但不进 Workflow Service"的口径作废。 |
| P3 | 这类 pipeline 在 WebUI 里可以只读，但系统管理员至少能通过 UI 观察当前处理流程；这是 workflow service 的一个实际使用案例。 | Run / Step 进 task_manager 任务树（workflow service §0.1 已规定），Desktop 的 workflow 应用从 mock 切到真实 RPC，系统定义带 `system` 标记且不可编辑。 |
| P4 | 先以"系统内置 pipeline"功能跑通；单独 TODO 推进。 | 本文。workflow service 新增"系统内置 Definition"能力（T1）是前置。 |

## 1. 现状对照

| 范围 | 已有 | 缺口 |
| --- | --- | --- |
| Definition / Run / 观测 | [server.rs](../src/kernel/workflow/src/server.rs)：`submit_definition`（以 owner + name + hash 幂等）/ `get` / `list` / `archive` / `dry_run`、`create_run` / `start_run` / `get_run_graph` / `list_runs`；Run → `workflow/run` task 镜像 | Definition 无 `source` 字段，没有系统内置定义的播种、只读保护与列表过滤；`submit_definition` 强制 `owner`；`create_run` 没有"同一定义 + 同一输入的进行中 Run 合并"（preview 的 ensure 语义） |
| 跨 Run 缓存 | `cache_key = hash(executor, identity, resolved_input)`，命中即回填并标 `source: cache`（[orchestrator.rs:556](../src/kernel/workflow/src/orchestrator.rs#L556) / [2333](../src/kernel/workflow/src/orchestrator.rs#L2333)）。`NamedStoreObjectStore` 已实现（[object_store.rs:60](../src/kernel/workflow/src/object_store.rs#L60)） | **main.rs 装配的是 `InMemoryObjectStore`**，缓存只活在进程内，重启即空（F5）；key 不含转换器版本与外部工具指纹；`idempotent` 默认 true 的风险见 [workflow_idempotency_review.md](workflow_idempotency_review.md) |
| Executor | `ExecutorRegistry` 直执行通道；只注册了 `AiccAdapter`（[main.rs:163](../src/kernel/workflow/src/main.rs#L163)） | 没有任何做文件转换的 executor；`operator::` 边界禁止文件系统与重计算（[executor list §6](../doc/workflow/executor%20list.md)），媒体 stage 不能走 `operator::`；`appservice::` / `http::` registry 与 adapter 待补；`/tool/` 语义链接展开缺 registry |
| 工具所在位置 | poppler / tesseract / ffmpeg 只在 Linux 容器 `extended` 层，soffice 在 nfs_server 侧（[paios 容器需求 §6.4](../doc/app_service/paios容器需求.md)） | workflow service 是 Zone 服务，运行节点不一定有这些工具 → 转换 stage 在哪执行是 §2 的 D1 |
| 工具探测样板 | nfs_server preview [tools.rs](../src/frame/nfs_server/src/preview/tools.rs)：三平台查找、版本、`hello` 只广告能产出的 profile | 只在 nfs_server crate 内，没抽出来 |
| 客户端 | buckyos-api 有 `WorkflowServiceClient`（只有 9 个 schedule 方法）与 `runtime.get_workflow_service_client()`（[workflow_service.rs](../src/kernel/buckyos-api/src/workflow_service.rs)、[runtime.rs:2774](../src/kernel/buckyos-api/src/runtime.rs#L2774)）；TS 源码在同级仓库 `buckyos-websdk/src/workflow_client.ts`，`WorkflowClient` 已有 submitDefinition / getDefinition / listDefinitions / dryRun / createRun / startRun / tickRun / getRunGraph / listRuns / submitStepOutput / amendment 方法 | Rust 客户端缺 Definition / Run 方法；TS 缺 `source` 过滤与 ensure 参数；Desktop `app/workflow` 尚未 import SDK |
| WebUI | Desktop `app/workflow`：HomePage / DetailPage / GraphCanvas，`mock/types.ts` 已定义 `DefinitionSource = 'system' \| 'user_imported' \| 'app_registered' \| 'agent_generated'` | 全部读 `mock/store.ts`，未接 RPC；没有只读态 |
| llm_understand_media | 图片 image_fit（不是独立文件，是 [llm_understand_media.rs](../src/frame/agent_tool/src/llm_understand_media.rs) 内 `prepare_image_content` / `attach_image_preprocess` 一段逻辑，2614 行文件）、视频 ffmpeg 抽帧（`extract_video_frames`）、音频 AICC asr、其余 `AiContent::Document` 直传 | 整个文件先读进内存转 base64 再做任何判断（`resolve_media`）；`ResourceRef::NamedObject` 在 `media_bytes` 里直接报错 "was not resolved to bytes"（[1427](../src/frame/agent_tool/src/llm_understand_media.rs#L1427)），即今天**读不了 obj_id**，读管线产物也要补；没有 `media_decision` / `coverage` / `pipeline_trace`；local_file 没有内容身份 |
| AICC 文档直传 | Claude / Gemini / OpenAI Responses 协议接受 `Document` | OpenAI Chat Completions 协议拒收（[openai_chat_completions.rs:520](../src/frame/aicc/src/protocol/openai_chat_completions.rs#L520)），"PDF 总能直传"不成立 |


### 1.1 本轮 review 发现（2026-11-05 第二轮，决定任务拆分）

| # | 事实 | 位置 | 对本文的影响 |
| --- | --- | --- | --- |
| F1 | **执行同步阻塞 RPC**：`start_run` / `create_run(auto_start)` 在 handler 里 `await orchestrator.tick`，`tick` 的循环里 `schedule_apply_direct` 同步 `await adapter.invoke`，全过程持有该 Run 的 `Mutex` | [server.rs:401-430](../src/kernel/workflow/src/server.rs#L401)、[orchestrator.rs:99-153](../src/kernel/workflow/src/orchestrator.rs#L99)、[644-688](../src/kernel/workflow/src/orchestrator.rs#L644) | 一次几十秒的 PDF 渲染会挂住 kRPC 调用；期间 `get_run_graph` / `list_runs` 对该 run 阻塞，管理员"看到走到哪一步"（P3）不成立；D8 的"Run 异步"今天不成立 → D10 |
| F2 | orchestrator 不实现 `guards.timeout`（源码无 timeout 处理） | orchestrator.rs | 卡死的 ffmpeg 会卡死 Run → D10 在 adapter 调用外包 timeout |
| F3 | step 失败后 attempt 用尽时按 `fallback` 处理，`RetryFallback` 默认 `Human` → 节点置 `WaitingHuman` | [orchestrator.rs:747-772](../src/kernel/workflow/src/orchestrator.rs#L747)、[workflow_dsl.rs](../src/kernel/buckyos-api/src/workflow_dsl.rs) `impl Default for RetryFallback` | 系统管线没有人：缺工具会把 Run 挂在 `WaitingHuman` 而不是 `Failed`，验收 3 不成立 → D14 |
| F4 | adapter 只能返回 `WorkflowError::Dispatcher(String)` 之类的字符串错误，`workflow_error_payload` 给出 `(code, message, details)` | [state.rs:472](../src/kernel/workflow/src/state.rs#L472)、[adapters/aicc.rs:413](../src/kernel/workflow/src/adapters/aicc.rs#L413) | "列出缺的工具"需要结构化 details → D14 |
| F5 | 全内存：(a) 缓存不持久；(b) Run 不持久，重启后 `run_id` not_found；(c) Definition 不持久 | [main.rs:180-181](../src/kernel/workflow/src/main.rs#L180)、[state.rs:1-15](../src/kernel/workflow/src/state.rs) 模块注释 | (a) 让验收 1 跨重启失效 → D11；(b) P0 接受，等待中的 agent 凭 D6 ensure 重建 Run；(c) 系统定义靠启动播种恢复，正好；用户定义持久化不在本文 |
| F6 | Definition `id` 固定为 `wf-<uuid>`，忽略 DSL 里的 `id`；`get_definition` / `create_run` 只接 `workflow_id`；幂等键是 `(owner, name, hash)` | [state.rs:163-199](../src/kernel/workflow/src/state.rs#L163) | 客户端没法稳定地按 `media.pdf` 调用 → D12 |
| F7 | `submit_definition` / `create_run` 直接信任 `params.owner`；schedule 那组 API 却经 `authenticated_schedule_owner` 从 session 校验 | [server.rs:210](../src/kernel/workflow/src/server.rs#L210)、[322](../src/kernel/workflow/src/server.rs#L322)、[796](../src/kernel/workflow/src/server.rs#L796) | Run task 的 creator 由此决定；任何 principal 可冒充他人建 Run → D13 |
| F8 | DSL `idempotent` 是 `bool` 且 serde 默认 true，"没写"与"写了 true"不可区分；静态分析没有 executor 维度规则，已有 rule code 为 snake_case（`invalid_reference`、`budget_overcommit` …） | [workflow_dsl.rs:47](../src/kernel/buckyos-api/src/workflow_dsl.rs#L47)、[analysis.rs](../src/kernel/workflow/src/analysis.rs) | D4 "缺 `idempotent: true` 报 error"在 DSL 改成三态前做不到；P0 用播种 JSON 显式写 true + 单测断言代替 |
| F9 | 没有 "Run output" 概念，只有 `node_outputs`；`get_run_graph` 整体返回 | [server.rs:438-463](../src/kernel/workflow/src/server.rs#L438) | 约定 Run output = 终端 step（`build_catalog` / `fit_image`）的 `node_output`，客户端从 `node_outputs[<step>]` 读 |
| F10 | workflow 的 AppDoc 没声明 bucket；KernelService 可用 `runtime.get_local_named_store()` 直写 named store；`BucketClient::put_file / put_bytes` 返回 `(ObjId, PutResult)`，并有 `obj://bucket/<scope>/<app>/<owner>/<bucket>/<key>` URL | [workflow_service.rs:18](../src/kernel/buckyos-api/src/workflow_service.rs#L18)、[runtime.rs:379](../src/kernel/buckyos-api/src/runtime.rs#L379)、[bucket_client.rs:172-260](../src/kernel/buckyos-api/src/bucket_client.rs#L172) | 产物落点要定 → D15 |
| F11 | **依赖方向健康**：workflow Cargo.toml 只依赖 buckyos-api / buckyos-kit / kRPC / named_store / ndn-lib / buckyos-http-server，不含 agent_tool / llm_context / libopendan。agent_tool 被 opendan / lib_opendan / node_executor 依赖；nfs_server 是 frame 层 | [Cargo.toml](../src/kernel/workflow/Cargo.toml) | T2 原文"搬 image_fit.rs"若做成 workflow 依赖 agent_tool 会反向；preview `tools.rs` 也不能直接 import → §2.2 |
| F12 | 部署链路齐全：scheduler `add_workflow()` 写 `services/workflow/spec`，`rootfs/bin/workflow` 已构建，`stop.py` 会杀 `workflow`；`check.py` 的进程 / 端口表没有 workflow（4070）；`main.rs` 启动硬依赖 login 与 kevent client | [system_config_builder.rs:476](../src/kernel/scheduler/src/system_config_builder.rs#L476)、[check.py:60-90](../src/check.py#L60) | check.py 补 workflow 一行即可观测 |
| F13 | 每个 Run 建一条 kevent 订阅（`watch_run`），终态 `unwatch_run` | [subscriptions.rs:151-197](../src/kernel/workflow/src/subscriptions.rs#L151)、[353](../src/kernel/workflow/src/subscriptions.rs#L353) | 纯 autonomous 的系统管线不需要人类回灌，可按 definition 跳过订阅（D7 开销项） |
| F14 | kernel service 跑在宿主机；poppler / ffmpeg / tesseract 只在 Linux `extended` 容器层 | [paios 容器需求 §6.4](../doc/app_service/paios容器需求.md) | D1 (d) 在开发机要手工装工具；"有则用"必须真的能在缺工具时给出结构化失败（验收 3） |
| F15 | DV 基础设施已有：`test/workflow_test/workflow_dv.ts`（Deno + websdk，`uv run test/run.py -p workflow_test`）、5 个 fixture、`testcases.md` WF-DV-001~008；`appservice::` / `http::` 用例标 Blocked | [test/workflow_test](../test/workflow_test/testcases.md) | 媒体用例按同一套路加 fixture，不需要 OpenDAN 容器 |

## 2. 总体方案

```text
llm_understand_media (agent_tool, 在 agent runtime)
  │ 1. 解析 media → 内容身份 obj_id（local_file 先 put_file 到 agent scratch 桶）
  │ 2. probe 只读文件头；预算内且模型原生支持 → 直传（最小内核，不建 Run）
  │ 3. 否则 workflow.create_run(ensure) 系统内置定义 media.<family>@<ver>，input = { obj_id, params }
  ▼
Workflow Service（Zone 服务）
  Run ──▶ Step capabilities ──▶ Step render/extract… ──▶ Step build_catalog
          每个 Step: cache_key 命中则回填(cache)；未命中调 service::media.* 执行，产物写 NDN
          Run / Step 镜像到 task_manager 任务树；WebUI 只读展示
  ▼
llm_understand_media
  │ 4. 读 Run 输出里的 manifest obj_id → 宿主检索循环（设计文档 §4.7）→ 模型分析（xllm run，不进管线）
  │ 5. details.media_decision / pipeline_trace(run_id, 各 step cache_hit) / coverage
```

边界：管线只到"LLM 可用的表示"（页图、分页文本、目录 manifest、帧、转写）。模型分析 stage 不进管线：它按设计文档 §4.5.5 本来就不缓存，且要带父 history 快照与工具自己的 run 恢复语义。

### 2.1 待拍板的设计决策

| # | 决策 | 选项 | 建议 |
| --- | --- | --- | --- |
| D1 | **转换 stage 在哪执行、executor 叫什么** | (a) agent runtime 容器作为 `appservice::` 暴露转换端点，工具就在那里，但要先补 AppService registry / adapter，且容器不是稳定服务；(b) 独立 `media_tools` 标准服务 `service::media.*`，部署在带工具的节点；(c) `func::` + Scheduler / Node Runner，一期非主路径；(d) workflow service 进程内 adapter 直接 shell 出本节点工具 | DSL 从第一天就写 `service::media.<fn>`，P0 以 (d) 实现（与 `AiccAdapter` 同位置的 `MediaAdapter`），工具"有则用"并像 preview 一样广告能力；后续换成 (b) 或 (c) 不改定义与 cache key |
| D2 | **管线输入身份** | 只收 NDN `obj_id`（PRD §9.3）；local_file / URL 由 agent_tool 先落对象 | 落到 agent app 的 `scratch` 桶（7 天）；只在确实要转换时才算 hash（PRD §23.2 第 3 条），直传不算 |
| D3 | **产物存哪、谁能复用** | 产物 NDN 对象 + Orchestrator cache（Zone 全局，按内容寻址）；读权限 = 持有 obj_id | 接受"持有内容 hash 即可读派生产物"，cache record 不带 permission scope；OCR / 转写类 stage 以后再加 scope。安全口径，owner 定 |
| D4 | **cache key 的版本与工具指纹** | 现 key = executor + `ref:<executor>` + input，无版本 | 版本进 executor 名（`service::media.render_pdf_pages/1`）；工具指纹由首个 `capabilities` step 输出并进入下游 input，使不同节点、不同 ffmpeg 版本自然分键；所有 stage 显式 `idempotent: true`，不靠默认 |
| D5 | **系统内置 Definition 的语义** | 服务启动时从编译期内嵌 JSON 播种；`owner = system`；新增 `source: system`；`submit_definition` / `archive_definition` 对 system 定义拒绝外部修改，升级只随服务版本；任何 Zone 内 principal 可 `create_run` | 字段名、owner 取值、谁能跑由 owner 定 |
| D6 | **create_run 的 ensure 语义** | 现无"同定义 + 同输入的进行中 Run 合并"；并发两个 agent 转同一文件会跑两次 | 给 `create_run` 加 `idempotency_key = hash(workflow_id, plan_version, input)`：命中进行中 Run 返回同一 `run_id`，命中已完成返回其输出。改 workflow API，owner 定 |
| D7 | **Run 的留存与开销** | 每次调用一个 Run + N 个 task，永久留在 workflow 与 task_manager | P0 接受并度量；转换类 Run 设 TTL（如 30 天）由 workflow 维护任务回收；后续若开销显著再加"先查缓存不建 Run"的入口 |
| D8 | **等待与 Pending** | Run 异步；`llm_understand_media` 等待预算内完成，超时返回 `Pending(LongRunning, check_after)` 并在 details 带 `run_id` | 下次调用凭 D6 的 ensure 命中同一 Run，无需 agent 记 run_id |
| D9 | **DOCX / XLSX 文本从哪来** | `import_office` / `import_table` 是 Deno 命令且无文本导出模式；或 Rust 解析 crate；或只走 soffice → PDF | P0 只做 PDF 与图片、视频、音频；DOCX 先 soffice → PDF（工具在哪见 D1）；XLSX 延后，待 owner 定复用方式与新依赖 |
| D10 | **Run 驱动异步化 + timeout**（F1 / F2） | (a) `start_run` 只 spawn 一个后台 tick 任务就返回，tick 内仍持锁同步 await adapter；(b) 直执行改"投递-回执"：`schedule_apply_direct` 只置 `Running` + spawn `adapter.invoke`，完成后经新增 `complete_direct_apply(run_id, node_id, attempt, Result)` 回灌并再 tick（同 `handle_thunk_result` 的形状），锁只在状态变更瞬间持有；invoke 外包 `tokio::time::timeout(guards.timeout)` | (b)。`attempt` 守卫丢弃过期回执；`start_run` / `create_run(auto_start)` 立即返回 `status: running`；仍保留同步 tick 给单测与 `tick_run` 运维入口。这是 workflow 自身的能力补齐，不止服务媒体管线 |
| D11 | **缓存持久化**（F5a） | (a) `main.rs` 用 `runtime.get_local_named_store()` 装配已有的 `NamedStoreObjectStore`；(b) service-local kv（sqlite，与 nfs_server / agent_tool 同款 rusqlite） | (a)，缓存记录是 Zone 全局、内容寻址的，天然属于 Named Store。先核实 `deterministic_object_id` 产出的 `workflow_cache:<hex>` 是否能被 `ObjId::new` 接受（[object_store.rs:121](../src/kernel/workflow/src/object_store.rs#L121)），不行则换合法 obj type。Run / Definition 持久化不在本文 |
| D12 | **系统 Definition 的身份**（F6） | record.id = DSL `id`，保留前缀 `media.` / `sys.`；version 随服务版本（内容 hash 变则 +1，id 不变，运行中 Run 绑定 `plan_version`） | 采纳。`submit_definition` 对保留前缀与 `source: system` 一律拒绝（`error: reserved_definition`）；`get_definition` / `create_run` 的 `workflow_id` 直接填 `media.pdf` |
| D13 | **owner 来源**（F7） | `create_run` / `submit_definition` 像 schedule API 一样从 session 校验 `(user_id, app_id)`，`params.owner` 只做一致性声明 | 采纳，复用 `WorkflowCallerVerifier`。系统定义 owner 取值（如 `{user_id: "system", app_id: "workflow"}`）由 owner 定；系统定义允许任何已认证 Zone 内 principal `create_run`，Run 与 task creator 是调用者 |
| D14 | **失败语义与错误契约**（F3 / F4） | 每个 step 显式 `guards.retry = {max_attempts: 1, fallback: "abort"}`（缺工具重试无意义；临时错误交给 agent 侧下次 ensure 再跑）；新增 `WorkflowError::ExecutorFailed { code, message, details: Value }`，`details` 约定 `{ tools_missing: [..], tool, exit_code, stderr_tail, retryable }` | 采纳。`workflow_error_payload` 把 details 原样进事件与 Step task error；客户端把 `Failed` / `Aborted` 当失败，`WaitingHuman` 视为定义写错（静态分析对 `source: system` 定义的 `fallback: human` 报 error） |
| D15 | **产物落点**（F10，细化 D3） | (a) 直接 `named store put_object / put_chunk`：无 TTL、无 GC、无 URL；(b) workflow 自己的 bucket（AppDoc 声明 `artifacts: {kind: data, data_policy: temp, default_ttl_days: 30}`），`BucketClient::put_bytes / put_file`，返回 obj_id + `obj://bucket/...` URL | (b)，TTL 与 D7 对齐。key = `<family>/<src_obj_id>/<step>@<ver>/<params_hash>/<name>`；manifest 同时给 `obj_id` 与 `url`。缓存记录比产物活得久的问题：cache 命中后由 adapter 侧 `head` 校验产物还在，不在则当未命中重跑（`lookup_cache` 加可选校验钩子） |


### 2.2 依赖方向与独立开发边界

```text
                buckyos-api（DSL wire、WorkflowServiceClient、常量、BucketClient）
                   ▲                               ▲
   workflow (kernel, 本文 Phase A)         agent_tool (frame) → llm_context → libopendan → opendan
   └─ adapters/media/*（工具探测、转换、catalog）            └─ llm_understand_media（Phase B，T4）
   ✗ 不得依赖：agent_tool、llm_context、libopendan、opendan、nfs_server、aicc crate
```

1. **workflow crate 新增依赖只能来自 kernel 层或外部 crate**：buckyos-api、named_store、ndn-lib、`image`（agent_tool 已用 0.25.5，`default-features = false, features = [jpeg, png, webp, bmp]`，可直接复用同一版本声明）、tokio::process。不能加 agent_tool / llm_context / libopendan / nfs_server / aicc。验收 12 用 `cargo tree -p workflow -i agent_tool` 为空兜底。
2. **复用只能"下沉"，不能"上拉"**：`llm_understand_media.rs` 里的 image_fit 逻辑和 nfs_server preview 的 `tools.rs`（341 行，三平台查找 / 版本解析 / `Probe::describe`）复制或下沉到 `src/kernel/workflow/src/adapters/media/`；若 nfs_server 以后也想用同一份探测，再抽 `src/kernel/media_tools` crate，本文不做。T4 完成后 agent_tool 删除自己的拷贝，只经 RPC 消费。
3. **agent 侧只认 RPC**：T4 通过 buckyos-api 的 `WorkflowServiceClient` 调 `create_run` / `get_run_graph`；workflow 对 agent、WorkSession、history 一无所知。manifest schema 是两侧唯一的共享契约，定稿在设计文档 §4.6.2，Rust 类型放 buckyos-api（`workflow_types.rs` 旁新增 `media_manifest.rs`），两侧都 import 它。
4. **测试三层，前两层不需要 Zone，三层都不需要 OpenDAN 容器**：
   - (a) `cargo test -p workflow`：`InMemoryObjectStore` + `NoopTaskTracker` + 用 `NamespaceAdapter` 伪造的 `service::media.*`，覆盖 ensure、cache 命中 / 版本分键、Failed 而非 WaitingHuman、异步驱动与 timeout、系统定义只读。现有 server.rs 16 个测试就是这个写法。
   - (b) 工具层测试 `#[ignore]`：带 `tests/fixtures/sample.pdf` 等小样例，`cargo test -p workflow -- --ignored` 在装了 poppler / ffmpeg 的机器上跑；缺工具时断言 `tools_missing` 结构。
   - (c) DV：`test/workflow_test` 加 `wf-dv-media-*.json` fixture 与用例 WF-DV-009~012，脚本用 websdk `WorkflowClient` + bucket 上传样例文件后 `createRun`，`uv run test/run.py -p workflow_test`。
5. **开发机前提**：D1 (d) 要在运行 workflow 的宿主机装 `poppler-utils`、`ffmpeg`、`libreoffice`（可选）、`tesseract`（可选）；缺哪个就按验收 3 失败。写进 `test/workflow_test/testcases.md` 运行前提。

## 3. 系统内置管线定义（P0 清单）

每条定义一个 family，`id = media.<family>`，内嵌于 workflow service，`schema_version` 随 DSL：

| family | 输入 | steps（全部 `autonomous`、`idempotent: true`） | 输出（Run output） |
| --- | --- | --- | --- |
| `media.image` | `{obj_id, max_long_edge, max_pixels, max_bytes}` | `capabilities` → `probe` → `fit_image` | `{model_media_obj_id, original, sent, actions}`（即今天 `image_fit` 的结果，搬进 adapter） |
| `media.pdf` | `{obj_id, dpi, max_long_edge, page_range?}` | `capabilities` → `probe` → `extract_pdf_text` → `render_pdf_pages` → `build_catalog` | `{manifest_obj_id, pages, text_obj_ids[], image_obj_ids[], missing[]}` |
| `media.video` | `{obj_id, strategy: uniform, max_frames, frame_max_edge}` | `capabilities` → `probe` → `extract_frames` → `extract_audio` → `build_catalog` | `{manifest_obj_id, frames[{t, obj_id}], audio_obj_id?}`；转写仍由 agent_tool 调 AICC asr（模型调用不进管线） |
| `media.document` | `{obj_id}`（docx / pptx / odt…） | `capabilities` → `probe` → `to_pdf` → 复用 `media.pdf` 的后三步 | 同 `media.pdf`；`to_pdf` 的产物与 preview 的 `preview-document` 语义相同但独立缓存 |

`build_catalog` 的 manifest 是设计文档 §4.6.2 的 `index.json`：源 obj_id、定义 id 与版本、每条 entry 的 `{kind, where(页码/时间戳), text_obj_id?, image_obj_id?, status}`，空白页 / 无文本页 / 失败页都有 entry。manifest 自身是 NDN 对象，Run output 只放其 obj_id 与摘要（不内联 base64，workflow service §5.1）。

每个 step 显式写 `"idempotent": true` 与 `"guards": {"retry": {"max_attempts": 1, "fallback": "abort"}, "timeout": "120s"}`（D14 / F8；播种时单测断言，不靠 DSL 默认值）。`capabilities` 的输出以 `tools` 字段进入每个下游 step 的 `input`，使工具版本自然进入 cache key（D4）；`capabilities` 自身 `idempotent: false`，否则换了节点也命中旧指纹。

`service::media.*` 的首批函数与参数沿用设计文档 §4.5.6 的表；`capabilities` 返回本节点可用工具及版本（D4），缺工具的 step 以结构化错误失败，Run 进入 `Failed`，`llm_understand_media` 据此退回直传并在 `media_decision.fallbacks` 记录。

## 4. 实施任务

拆成两个阶段。**Phase A 只改 `src/kernel/workflow`、`src/kernel/buckyos-api`、同级 `buckyos-websdk`、`test/workflow_test`、`src/check.py` 与文档**，完成后 workflow service 就已经是一个能被任何客户端调用、能在 WebUI/TaskMgr 观察的真实服务；Phase B 两项互不依赖，都只消费 Phase A 的 RPC。

### Phase A：workflow service 跑通（不依赖 OpenDAN）

- [ ] **T0 文档回写**：设计文档 §4.5（本地 runner → Workflow Service 系统内置 pipeline，删去 §4.5.1 "不进 Workflow Service" 的表）、§4.5.5 落地方式注、§4.8（改为"复用 Pipeline 处理模型"，指向本文 §2）、§0 依赖前置、§4.6.1 阶梯第 ① 步、§9 / §10；修正两处与代码不符：`resolve_media` 整文件读入内存、OpenAI Chat Completions 拒收 Document。[executor list](../doc/workflow/executor%20list.md) 加 `service::media.*` 一节；[workflow service](../doc/workflow/workflow%20service.md) §3.1 加 `source` / 保留 id、§3.2 加 ensure 与异步返回语义、§5.1 加"当前实现：内存，缓存走 Named Store"现状注、§9 一期范围加"系统内置 Definition"；[workflow_idempotency_review](workflow_idempotency_review.md) 建议 1（三态 idempotent）标注为本文 F8 的前置。
- [ ] **T1a 系统内置 Definition**（D5、D12、D13）：`DefinitionRecord` 加 `source: system | user_imported | app_registered | agent_generated`（与 Desktop `mock/types.ts` 的 `DefinitionSource` 同名同值）；`DefinitionStore::seed_system(defs)` 启动时从 `include_str!` 的 JSON 播种，compile / analyze 失败则服务启动失败；record.id = DSL `id`，保留前缀检查；`submit_definition` / `archive_definition` 对 `source: system` 返回 `reserved_definition`；`list_definitions` 加 `source` 过滤；`create_run` / `submit_definition` 改为经 `WorkflowCallerVerifier` 校验 owner；`to_summary_value` / `to_value` 带 `source`。
- [ ] **T1b `create_run` ensure**（D6）：参数 `ensure: true`；key = `hash(workflow_id, plan_version, canonical(input))`；`RunStore` 加 `(key → run_id)` 索引，命中非终态 Run 返回同一 `run_id` 与 `reused: true`，命中 `Completed` 返回其 `run_id` + `output`；`list_runs` 加 `status` 过滤；响应统一带 `output`（= 无后继 step 的 `node_outputs`，F9）。
- [ ] **T1c Run 驱动异步化与 timeout**（D10）：`schedule_apply_direct` 改投递-回执；`complete_direct_apply` 入口 + `attempt` 守卫；`guards.timeout` 解析（已有 `"120s"` 字串格式）并在 invoke 外包 `tokio::time::timeout`，超时按 `ExecutorFailed { code: "timeout" }`；`start_run` 立即返回；`RunHandle` 锁粒度缩到状态变更；并发上限（per-service semaphore，默认 CPU 数）。单测：一个 sleep 2s 的 fake adapter，`start_run` < 100ms 返回，期间 `get_run_graph` 可读，完成后状态 `Completed`。
- [ ] **T1d 缓存持久化与可观测**（D11、F12）：`main.rs` 用 `runtime.get_local_named_store()` 装配 `NamedStoreObjectStore`；核实 / 修正 `deterministic_object_id` 的 obj type；`check.py` 进程表与端口表加 `workflow` / 4070；系统管线 Run 不做 kevent `watch_run`（F13，按 definition 没有 human step 判断）。
- [ ] **T1e 错误契约**（D14）：`WorkflowError::ExecutorFailed { code, message, details }`；`workflow_error_payload` 透传 details；静态分析新规则 `system_definition_human_fallback`（`source: system` 且任一 step `fallback != abort` → error）与 `executor_retry_without_idempotent`（external executor 配了 `max_attempts > 1` 但 `idempotent` 非显式 true → warning，F8 落地前只能 warning）。
- [ ] **T1f 客户端**：buckyos-api `WorkflowServiceClient` 加 `submit_definition` / `get_definition` / `list_definitions` / `create_run(ensure)` / `start_run` / `get_run_graph` / `list_runs` / `get_history`，返回 typed struct（`WorkflowRunGraph`、`WorkflowRunSummary` 与 TS 侧同名）；websdk `WorkflowClient` 加 `source` 过滤、`ensure` 参数、`output` 字段；两侧共享 `media_manifest` 类型（§2.2 第 3 条）。
- [ ] **T2 `service::media.*` adapter**（P0 进程内，D1 (d)，位置 `src/kernel/workflow/src/adapters/media/`）：`tools.rs`（复制 nfs_server preview 探测，去掉 Profile 语义，保留三平台查找 / 版本 / `describe`）；`capabilities` / `probe`（只读文件头 + `pdfinfo` / `ffprobe` / image 解码头）/ `fit_image`（下沉 image_fit 逻辑）/ `extract_pdf_text`（`pdftotext -layout` 分页）/ `render_pdf_pages`（`pdftoppm -r <dpi> -scale-to <max_long_edge>`）/ `extract_frames` / `extract_audio` / `to_pdf`（`soffice --headless --convert-to pdf`）/ `build_catalog`；executor 名带版本 `service::media.render_pdf_pages/1`；输入 obj_id → 经 named store / bucket 取到本地临时文件（`$BUCKYOS_ROOT/tmp/workflow-media/<run_id>/`，step 结束即删）；产物 `BucketClient::put_file` 到 `artifacts` bucket（D15，AppDoc 声明 bucket）；每个外部命令 `tokio::process` + timeout + stderr 截尾；缺工具 → `ExecutorFailed { code: "media.tool_missing", details.tools_missing }`；并发由 T1c 的 semaphore 统一管。
- [ ] **T3 内置定义 JSON**：§3 四条 family 放 `src/kernel/workflow/defs/media.*.json`，`include_str!` 进二进制；`dry_run` 通过；每 step 显式 `idempotent` / `retry` / `timeout`（§3 注）；`media.document` 复用 `media.pdf` 后三步时通过 Reference 把 `to_pdf.output.pdf_obj_id` 接到 `extract_pdf_text.input.obj_id`；manifest schema 定稿写进设计文档 §4.6.2 并落 buckyos-api 类型。
- [ ] **T6a 测试（Phase A 验收）**：(a) 单测覆盖 §2.2 第 4 条列表；(b) `#[ignore]` 工具测试 + `tests/fixtures/`（1 页 PDF、1 张 4000px JPG、3s MP4，均 < 200KB）；(c) DV fixture `wf-dv-media-pdf.json` / `-image.json` / `-video.json` / `-missing-tool.json`，`testcases.md` 加 WF-DV-009（同文件两次 Run 全 cache）/ 010（并发 ensure 只一个 Run）/ 011（缺工具 Failed + details）/ 012（WebUI / TaskMgr 可见 system 定义与 Run），运行前提补工具安装；全部不需要 OpenDAN 容器。

### Phase B：接入方（依赖 Phase A 完成，两项可并行）

- [ ] **T4 agent_tool 接入**：`resolve_media` 改为只读文件头 probe；local_file → scratch 桶（D2，`BucketClient::put_file`，`HOMESTATION_BUCKET_SCRATCH`）；补 `ResourceRef::NamedObject` 的读取（F15 之前读不了 obj_id）；直传 / 缩小 / 转换阶梯（设计文档 §4.6.1）里"转换"= `WorkflowServiceClient::create_run(ensure)`；等待与 Pending（D8，超预算返回 `Pending(LongRunning, check_after)`，下次凭 ensure 命中）；读 manifest → 宿主检索循环（§4.7，v1 词法检索）→ 模型分析；`details.media_decision` / `pipeline_trace`（run_id、每 step `cache_hit`、`source`）/ `coverage`；直传失败的错误码 → 降级映射；删除 agent_tool 内的 image_fit / ffmpeg 抽帧拷贝；更新工具描述与 cmd_skills（`ui_core`、`xllm`、`aigc_*`）。无 workflow 可达时的全部直传路径保持可用（验收 5）。
- [ ] **T5 WebUI 只读观测**（P3）：Desktop `app/workflow` 从 `mock/store.ts` 切到 websdk `WorkflowClient`（`listDefinitions` / `getDefinition` / `listRuns` / `getRunGraph`）+ task_manager 子任务；`source: system` 徽标，编辑 / 归档 / 提交入口对 system 定义禁用；Run 详情能点到 TaskMgr 任务树；对照 [Workflow WebUI 产品需求](../product/workflow/BuckyOS%20Workflow%20WebUI%20产品需求.md) §6.2（system 定义在组织树内，`source` badge）与 §7.3。
- [ ] **T6b 验收**：§5 全部。

新增第三方依赖：workflow 加 `image`（已在 workspace 其它 crate 使用）；PDF / 视频 / Office 全部 shell 出外部工具，不引入 Rust 解析 crate；docx / xlsx 解析 crate 按仓库约定单独确认，不在本文默认范围。

## 5. 验收

1. 同一 PDF 被两个 agent 先后理解：第二次 Run 的每个转换 step 都 `source: cache`，`pipeline_trace` 全部 `cache_hit: true`，无新 NDN 产物。
2. 两个 agent 并发理解同一文件：只存在一个进行中 Run（D6）。
3. workflow service 节点缺 poppler：`media.pdf` Run 以结构化错误 `Failed`，`llm_understand_media` 退回直传，`media_decision.fallbacks` 与错误 message 列出缺的工具。
4. 管理员打开 Workflow WebUI：看到四条 `system` 定义且不可编辑，看到进行中与已完成的转换 Run 及其 step 状态，能进入 TaskMgr 任务树。
5. 没有 workflow service 可达时，图片、音频、短视频、模型原生支持的 PDF 仍通过直传得到报告（设计文档 §8 第 16 项）。
6. 工具版本变化后同一输入产生新 cache key（D4）。
7. 主干 history 体积仍只随 `output` 文本线性增长（设计文档 §8 第 14 项）。
8. **独立性**：`cargo test -p workflow -- --test-threads=1` 不依赖 Zone 通过；`uv run test/run.py -p workflow_test` 在没有任何 OpenDAN 容器运行的 Zone 上通过 WF-DV-009~012。
9. **重启**：kill 并重启 workflow 进程后，同一 PDF 的新 Run 每个转换 step 仍 `source: cache`（D11）。
10. **非阻塞**：一次 ≥ 30s 的转换期间，`start_run` 在 1s 内返回 `running`，`get_run_graph` 随时能读到各 step 状态；`guards.timeout` 到期的 step 以 `code: timeout` 失败（D10）。
11. **失败形态**：缺工具的 Run 终态是 `Failed`（不是 `WaitingHuman`），`error.details.tools_missing` 非空，Step task 的 error 字段里能看到同样内容（D14）。
12. **依赖方向**：`cd src && cargo tree -p workflow -i agent_tool` / `-i llm_context` / `-i libopendan` / `-i nfs_server` 均为空（§2.2）。
13. **身份**：用 A 的 session 带 `owner: B` 调 `create_run` 被拒；`submit_definition` 提交 `id: media.pdf` 被拒 `reserved_definition`（D12 / D13）。

## 6. 本次记录

- 2026-11-05：review 设计文档 v0.4，先误读 §4.8 为"真的接 preview 组件"，用户纠正为复用处理模型；再定为用 workflow service 的系统内置 pipeline 承载；本文成稿，待 §2.1 D1–D9 拍板后 T1 开工。
- 2026-11-05（第二轮）：用户定位本文为 Agent Runtime 改造的前置，并要求 workflow 侧不依赖任何 OpenDAN 组件、可独立开发测试。对照 workflow service.md / executor list.md 与 `src/kernel/workflow` 实现复核：修正 §1 三处与代码不符（缓存实为进程内存、buckyos-api 已有 schedule 客户端、TS 源码在 buckyos-websdk）；新增 §1.1 F1–F15、§2.2 依赖边界、D10–D15；§4 拆为 Phase A（workflow 独立）/ Phase B（agent_tool、WebUI）；验收加 8–13。`cargo test -p workflow` 50 通过。D1–D15 仍待拍板，其中 D10 / D11 / D13 / D14 是 workflow 自身补齐项，与媒体管线无关也应做。
