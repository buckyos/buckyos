# 媒体表示管线跑在 Workflow Service 上（系统内置 pipeline）TODO

日期：2026-11-05
来源：对 [LLM Understand Media](../doc/llm_context/LLM%20Understand%20Media.md) v0.4 §4.5–§4.8 的 review 与用户三次裁决（见 §0）。本文承接设计文档的 T1–T6，但把"本地 runner + 复用 preview 缓存"改为"Workflow Service 的系统内置 pipeline"。设计文档 §4.5 / §4.8 需按本文回写（T0）。
基线：`src/kernel/workflow` 已有 DSL、compile / analyze、Orchestrator、ExecutorRegistry（只注册了 `service::aicc.*`）、RPC 前端与 TaskMgr 镜像；第二轮记录的 50 个单测通过，不代表本轮重新验证。服务进程内 Definition / Run / cache 与 thunk dispatcher 仍是内存实现（[main.rs:180-181](../src/kernel/workflow/src/main.rs#L180)），执行在 RPC handler 内同步驱动（§1.1）；`llm_understand_media` 已有图片缩图、视频抽帧、音频转写、文档直传；nfs_server preview 有 `work_key` / `ensure` / 工具探测的样板。路径相对 `buckyos/`。

状态（第七轮，2026-11-05）：Phase A（T0、T1a–T1h、T2a–T2c、T3、T6a 单测部分）已落地代码并通过 `cargo test -p workflow`；DV（`test/workflow_test` WF-DV-009~013）、真实工具链（`cargo test -p workflow -- --ignored`）、paios 镜像重建与宿主安装未在本轮执行。Phase B：T4（agent_tool 经 WorkflowServiceClient 接入，`image_fit.rs` 删除）、T5（Desktop `app/workflow` 改接真实 RPC，系统定义只读）、T4b 的镜像 / 文档输入已改代码，但未在 Zone / 镜像上验证；T6b 未执行。各任务与验收的状态见 §4 / §5 的标注，§6 第七轮记录。

定位：本文是一系列 Agent Runtime 改造任务的前置，同时是 workflow service 第一个真实业务负载。**workflow 侧的全部工作（§4 Phase A）不依赖任何 OpenDAN 组件**（agent_tool / llm_context / libopendan / opendan），可独立开发、独立测试、独立验收；agent 侧接入（T4）与 WebUI（T5）是 Phase B。依赖方向见 §2.2。本文同时是 **FunctionObject / Thunk 通道的第一次真实落地**（§2.3）；本地 Native 案例验证其最小契约，分布式执行的身份交接、环境指纹与数据访问仍后置。

目标：**一次转换请求由 Workflow Run 承载**，输入是内容寻址的对象，产物是 NDN 对象；相同执行规格尽量复用转换结果，同一授权范围内的活动 Run 合并，已完成请求通过新 Run 的 Step cache 复用。不同 principal 保留各自 Run，跨 Run 共享有效转换缓存（§7 R1），不承诺崩溃或失败后的 exactly-once 执行。`llm_understand_media` 消费管线产物再做模型分析。管线定义随系统发布，在 Workflow WebUI 中只读可见，管理员能观察转换进度。

## 0. 用户裁决（2026-11-05）

| # | 裁决 | 直接推论 |
| --- | --- | --- |
| P1 | §4.8 的意图是复用 Preview Pipeline 的**处理模型**（PRD §9.7 workKey、§23.2 ensure、§23.4 descriptor、§23.6 状态机），不是去调 preview 组件：preview 的目标是浏览器可用，本管线的目标是 LLM 可用，Target Profile 不同。 | 不需要 NFSP 客户端；不扩 `repr` wire；不进 `preview-cache` 桶。 |
| P2 | Pipeline 的定义不另造轮子，用 [workflow service](../doc/workflow/workflow%20service.md) 已有设施。 | 管线 = `WorkflowDefinition`；一次转换 = 一个 Run；stage = `autonomous` Step；跨 Run 复用 = Orchestrator 已有的 idempotent cache（[orchestrator.rs:556](../src/kernel/workflow/src/orchestrator.rs#L556)）。设计文档 §4.5"复用 DSL 但不进 Workflow Service"的口径作废。 |
| P3 | 这类 pipeline 在 WebUI 里可以只读，但系统管理员至少能通过 UI 观察当前处理流程；这是 workflow service 的一个实际使用案例。 | Run / Step 进 task_manager 任务树（workflow service §0.1 已规定），Desktop 的 workflow 应用从 mock 切到真实 RPC，系统定义带 `system` 标记且不可编辑。 |
| P4 | 先以"系统内置 pipeline"功能跑通；单独 TODO 推进。 | 本文。workflow service 新增"系统内置 Definition"能力（T1）是前置。 |
| P5（第三轮） | **这个版本要把 FunctionObject 用起来**。thunk 要定得意图化："图片放缩"是好的 function，直接包一条命令行不是。 | 媒体函数不再是 `service::media.*` adapter，而是第一批真实发布的 FunctionObject；DSL 写 `/tool/media.<fn>@<ver>`，经 registry 解析为 `func::<obj_id>`，走已有的 `build_thunk` → `ThunkDispatcher` → `handle_thunk_result` 通道；P0 由 workflow 进程内的 `LocalThunkRunner` 执行 `FunctionType::Native`。cmd 层（ExtTools）只回答"节点有没有、什么版本"，是 FunctionObject 的 `requirements`，不是 executor。见 §2.3 |
| P6（第五轮，R1） | 同意推荐的同授权范围合并 Run、跨主体共享结果缓存。 | ensure scope 包含 principal；不同 principal 首次并发可重复执行，不做全 Zone 共享 Run。 |
| P7（第五轮，R2） | workflow 是与 scheduler 同级的内核服务，不工作视为系统故障，不需要另设其不可达的媒体降级假设。 | 删除专门的本地备用转换与业务继续运行验收，保留正常选路、错误诊断与重启清理。 |
| P8（第五轮，R3） | 可以加依赖；以 workflow 需求为先导定义 agent-runtime baseline（paios 镜像自带工具）。 | 新增依赖授权已取得；从 FunctionObject requirements 推导工具清单，同步宿主预置与镜像分发，补 T2c / T4b。 |
| P9（第六轮） | 以缓存数据可用为目标设计 ThunkObject 和 FunctionObject 的字段，描述性内容放到 run meta。 | 对象 hash 只覆盖影响结果的字段：FunctionObject 拆出 FunctionRegistration，ThunkObject 去掉 metadata。thunk ID 变成计算身份，回执改用 execution_id 关联。P5 / P8 提到的 requirements 随之移到 FunctionRegistration。用户已确认三点：timeout / requirements / best_run_weight / affinity_type 移出 hash；params 使用内容 ID（FileObject.content），由服务从已授权的 FileObject 读取；params_schema 留在 hash。见 §2.3 |

## 1. 现状对照

| 范围 | 已有 | 缺口 |
| --- | --- | --- |
| Definition / Run / 观测 | [server.rs](../src/kernel/workflow/src/server.rs)：`submit_definition`（以 owner + name + hash 幂等）/ `get` / `list` / `archive` / `dry_run`、`create_run` / `start_run` / `get_run_graph` / `list_runs`；Run → `workflow/run` task 镜像 | Definition 无 `source` 字段，没有系统内置定义的播种、只读保护与列表过滤；`submit_definition` 强制 `owner`；`create_run` 没有"同一定义 + 同一输入的进行中 Run 合并"（preview 的 ensure 语义） |
| 跨 Run 缓存 | `cache_key = hash(executor, identity, resolved_input)`，命中即回填并标 `source: cache`（[orchestrator.rs:556](../src/kernel/workflow/src/orchestrator.rs#L556) / [2333](../src/kernel/workflow/src/orchestrator.rs#L2333)）。`NamedStoreObjectStore` 已实现（[object_store.rs:60](../src/kernel/workflow/src/object_store.rs#L60)） | **main.rs 装配的是 `InMemoryObjectStore`**，缓存只活在进程内，重启即空（F5）；key 不含转换器版本与外部工具指纹；`idempotent` 默认 true 的风险见 [workflow_idempotency_review.md](workflow_idempotency_review.md) |
| Executor | `ExecutorRegistry` 直执行通道；只注册了 `AiccAdapter`（[main.rs:163](../src/kernel/workflow/src/main.rs#L163)） | 没有任何做文件转换的 executor；`operator::` 边界禁止文件系统与重计算（[executor list §6](../doc/workflow/executor%20list.md)），媒体 stage 不能走 `operator::`；`appservice::` / `http::` registry 与 adapter 待补；`/tool/` 语义链接展开缺 registry |
| 工具所在位置 | [build_aios](../build_aios) 预置 ffmpeg、ImageMagick、poppler、tesseract；未预置 LibreOffice。nfs_server 有 soffice 探测 / 转换代码，不等于宿主已安装工具 | workflow LocalThunkRunner 在宿主机；P8 要求用函数需求驱动宿主与 paios 两份环境的工具交付，不能用镜像安装代替宿主安装 |
| 工具探测样板 | nfs_server preview [tools.rs](../src/frame/nfs_server/src/preview/tools.rs)：三平台查找、版本、`hello` 只广告能产出的 profile | 只在 nfs_server crate 内，没抽出来 |
| 客户端 | buckyos-api 有 `WorkflowServiceClient`（只有 9 个 schedule 方法）与 `runtime.get_workflow_service_client()`（[workflow_service.rs](../src/kernel/buckyos-api/src/workflow_service.rs)、[runtime.rs:2774](../src/kernel/buckyos-api/src/runtime.rs#L2774)）；TS 源码在同级仓库 `buckyos-websdk/src/workflow_client.ts`，`WorkflowClient` 已有 submitDefinition / getDefinition / listDefinitions / dryRun / createRun / startRun / tickRun / getRunGraph / listRuns / submitStepOutput / amendment 方法 | Rust 客户端缺 Definition / Run 方法；TS 缺 `source` 过滤与 ensure 参数；Desktop `app/workflow` 尚未 import SDK |
| WebUI | Desktop `app/workflow`：HomePage / DetailPage / GraphCanvas，`mock/types.ts` 已定义 `DefinitionSource = 'system' \| 'user_imported' \| 'app_registered' \| 'agent_generated'` | 全部读 `mock/store.ts`，未接 RPC；没有只读态 |
| llm_understand_media | 图片 `prepare_image_content` / `attach_image_preprocess`、视频 `extract_video_frames`、音频 AICC asr、其余 `AiContent::Document`；`resolve_media` 已经读取 NamedObject（[1111](../src/frame/agent_tool/src/llm_understand_media.rs#L1111)） | 整文件读取并转 base64，需保留 obj_id 并改为轻量 probe / 按需读取；没有 `media_decision` / `coverage` / `pipeline_trace`；当前视频依赖本地抽帧，音频走 ASR，并非已有原生音视频直传（[1370](../src/frame/agent_tool/src/llm_understand_media.rs#L1370)） |
| AICC 文档直传 | Claude / Gemini / OpenAI Responses 协议接受 `Document` | OpenAI Chat Completions 协议拒收（[openai_chat_completions.rs:520](../src/frame/aicc/src/protocol/openai_chat_completions.rs#L520)），"PDF 总能直传"不成立 |


### 1.1 源码核对与方案缺口（第二轮发现，第四轮修订）

| # | 事实 | 位置 | 对本文的影响 |
| --- | --- | --- | --- |
| F1 | **执行同步阻塞 RPC**：`start_run` / `create_run(auto_start)` 在 handler 里 `await orchestrator.tick`，`tick` 的循环里 `schedule_apply_direct` 同步 `await adapter.invoke`，全过程持有该 Run 的 `Mutex` | [server.rs:401-430](../src/kernel/workflow/src/server.rs#L401)、[orchestrator.rs:99-153](../src/kernel/workflow/src/orchestrator.rs#L99)、[644-688](../src/kernel/workflow/src/orchestrator.rs#L644) | 一次几十秒的 PDF 渲染会挂住 kRPC 调用；期间 `get_run_graph` / `list_runs` 对该 run 阻塞，管理员"看到走到哪一步"（P3）不成立；D8 的"Run 异步"今天不成立 → D10 |
| F2 | orchestrator 不实现 `guards.timeout` | orchestrator.rs | D10 在 thunk 路径定义有效 deadline、取消和进程回收；只包 timeout 不足以停止外部命令 |
| F3 | step 失败后 attempt 用尽时按 `fallback` 处理，`RetryFallback` 默认 `Human` → 节点置 `WaitingHuman` | [orchestrator.rs:747-772](../src/kernel/workflow/src/orchestrator.rs#L747)、[workflow_dsl.rs](../src/kernel/buckyos-api/src/workflow_dsl.rs) `impl Default for RetryFallback` | 系统管线没有人：缺工具会把 Run 挂在 `WaitingHuman` 而不是 `Failed`，验收 3 不成立 → D14 |
| F4 | adapter 只能返回 `WorkflowError::Dispatcher(String)` 之类的字符串错误，`workflow_error_payload` 给出 `(code, message, details)` | [state.rs:472](../src/kernel/workflow/src/state.rs#L472)、[adapters/aicc.rs:413](../src/kernel/workflow/src/adapters/aicc.rs#L413) | "列出缺的工具"需要结构化 details → D14 |
| F5 | 全内存：(a) 缓存不持久；(b) Run 不持久，重启后 `run_id` not_found；(c) Definition 不持久 | [main.rs:180-181](../src/kernel/workflow/src/main.rs#L180)、[state.rs:1-15](../src/kernel/workflow/src/state.rs) 模块注释 | (a) 让验收 1 跨重启失效 → D11；(b) P0 接受，等待中的 agent 凭 D6 ensure 重建 Run；(c) 系统定义靠启动播种恢复，正好；用户定义持久化不在本文 |
| F6 | Definition `id` 固定为 `wf-<uuid>`，忽略 DSL 里的 `id`；`get_definition` / `create_run` 只接 `workflow_id`；幂等键是 `(owner, name, hash)` | [state.rs:163-199](../src/kernel/workflow/src/state.rs#L163) | 客户端没法稳定地按 `media.pdf` 调用 → D12 |
| F7 | `submit_definition` / `create_run` 信任 `params.owner`；schedule API 已校验 session；TaskTracker 以 workflow client 建任务，Run owner 并未自动成为 Task creator | [server.rs](../src/kernel/workflow/src/server.rs)、[task_tracker.rs:306](../src/kernel/workflow/src/task_tracker.rs#L306) | D13 同时校验全部 Run 读写操作，明确逻辑 owner / requested_by、实际执行身份与 Task ACL |
| F8 | DSL `idempotent` 是 `bool` 且 serde 默认 true，无法区分未声明与显式 true；现有 cache 直接依赖它 | [workflow_dsl.rs:47](../src/kernel/buckyos-api/src/workflow_dsl.rs#L47)、[orchestrator.rs:556](../src/kernel/workflow/src/orchestrator.rs#L556) | 本批 JSON 显式声明并在播种前检查；纯度门禁同时覆盖缓存读写。全 DSL 三态改造另行跟进，不声称当前 typed analyzer 能识别缺省值 |
| F9 | 没有 "Run output" 概念，只有 `node_outputs`；`get_run_graph` 整体返回 | [server.rs:438-463](../src/kernel/workflow/src/server.rs#L438) | 新增 Run output 为终端节点 id → 结果映射；本批客户端读取 output[build_catalog / fit_image]，见 §3.1 |
| F10 | workflow 的 AppDoc 没声明 bucket；KernelService 可用 `runtime.get_local_named_store()` 直写 named store；`BucketClient::put_file / put_bytes` 返回 `(ObjId, PutResult)`，并有 `obj://bucket/<scope>/<app>/<owner>/<bucket>/<key>` URL | [workflow_service.rs:18](../src/kernel/buckyos-api/src/workflow_service.rs#L18)、[runtime.rs:379](../src/kernel/buckyos-api/src/runtime.rs#L379)、[bucket_client.rs:172-260](../src/kernel/buckyos-api/src/bucket_client.rs#L172) | 产物落点要定 → D15 |
| F11 | **依赖方向健康**：workflow Cargo.toml 只依赖 buckyos-api / buckyos-kit / kRPC / named_store / ndn-lib / buckyos-http-server，不含 agent_tool / llm_context / libopendan。agent_tool 被 opendan / lib_opendan / node_executor 依赖；nfs_server 是 frame 层 | [Cargo.toml](../src/kernel/workflow/Cargo.toml) | T2 原文"搬 image_fit.rs"若做成 workflow 依赖 agent_tool 会反向；preview `tools.rs` 也不能直接 import → §2.2 |
| F12 | 部署链路齐全：scheduler `add_workflow()` 写 `services/workflow/spec`，`rootfs/bin/workflow` 已构建，`stop.py` 会杀 `workflow`；`check.py` 的进程 / 端口表没有 workflow（4070）；`main.rs` 启动硬依赖 login 与 kevent client | [system_config_builder.rs:476](../src/kernel/scheduler/src/system_config_builder.rs#L476)、[check.py:60-90](../src/check.py#L60) | check.py 补 workflow 一行即可观测 |
| F13 | 每个 Run 建一条 kevent 订阅（`watch_run`），终态 `unwatch_run` | [subscriptions.rs:151-197](../src/kernel/workflow/src/subscriptions.rs#L151)、[353](../src/kernel/workflow/src/subscriptions.rs#L353) | 本批媒体管线可跳过 human-action 订阅，但仍需明确取消入口；不泛化到所有系统定义（T1d） |
| F14 | kernel service 跑在宿主机；build_aios 已带 poppler / ffmpeg，但未带 soffice；现有 Agent baseline 检查主要覆盖标准命令 | [build_aios](../build_aios)、[baseline.rs](../src/frame/lib_opendan/src/baseline.rs)、[paios 容器需求 §6.4](../doc/app_service/paios容器需求.md) | T2c / T4b 定义并验证媒体命令基线，分别检查宿主和镜像；缺工具是部署缺陷，保留结构化错误用于诊断 |
| F15 | DV 基础设施已有：`test/workflow_test/workflow_dv.ts`（Deno + websdk，`uv run test/run.py -p workflow_test`）、5 个 fixture、`testcases.md` WF-DV-001~008；`appservice::` / `http::` 用例标 Blocked | [test/workflow_test](../test/workflow_test/testcases.md) | 媒体用例按同一套路加 fixture，不需要 OpenDAN 容器 |
| F16 | compiler 将 executor 字符串重新 hash 为 fun_id；`build_thunk` 用 `thunk` 类型算 ID，调用处却以 `workflow_thunk` 存储 | [compiler.rs:465](../src/kernel/workflow/src/compiler.rs#L465)、[orchestrator.rs:599](../src/kernel/workflow/src/orchestrator.rs#L599)、[2171](../src/kernel/workflow/src/orchestrator.rs#L2171) | T1g 必须统一真实 FunctionObject / thunk 身份、规范化序列化与读回验证，不能只补 dispatcher |
| F17 | `ThunkExecutionResult.error` 是 `Option<String>`，`sync_thunk` 仅创建 Task | [thunk_object.rs:123](../src/kernel/buckyos-api/src/thunk_object.rs#L123)、[task_tracker.rs:748](../src/kernel/workflow/src/task_tracker.rs#L748) | D14 扩展完整回执错误通路；runner 负责 thunk Task 领取、进度与终态 |
| F18 | cache key 根据函数与输入计算，该 ID 下写入的却是结果和 metrics，ID 不等于记录内容 hash | [orchestrator.rs:241](../src/kernel/workflow/src/orchestrator.rs#L241)、[2333](../src/kernel/workflow/src/orchestrator.rs#L2333) | D11 区分不可变对象和持久缓存索引；`workflow_cache:<hex>` 可解析不是内容寻址正确性的证明 |
| F19 | NDM 按 holding / read 权限授权；Agent 默认只能读取自己 owner 的 user bucket | [ndm/mod.rs:265](../src/frame/nfs_server/src/ndm/mod.rs#L265)、[bucket/mod.rs:292](../src/frame/nfs_server/src/bucket/mod.rs#L292) | D3 补输入授权和产物交付；知道 obj_id 不等于有读权，见 §2.4 |
| F20 | Run 的 `plan_version` 初始为 1，Definition 版本计数在内存中；图查询按 workflow_id 取定义 | [orchestrator.rs:72](../src/kernel/workflow/src/orchestrator.rs#L72)、[state.rs:138](../src/kernel/workflow/src/state.rs#L138)、[server.rs:444](../src/kernel/workflow/src/server.rs#L444) | D12 新增不可变 definition_revision，Run 固定绑定；plan_version 保留为 Run 内 amendment 版本 |
| F21 | thunk ID 的计算包含 metadata（run_id / node_id / attempt）；`pending_thunks`、thunk Task 及其去重键都以 thunk_obj_id 为 key | [orchestrator.rs:163](../src/kernel/workflow/src/orchestrator.rs#L163)、[610](../src/kernel/workflow/src/orchestrator.rs#L610)、[task_tracker.rs:497](../src/kernel/workflow/src/task_tracker.rs#L497)、[529](../src/kernel/workflow/src/task_tracker.rs#L529) | 相同计算跨 Run、跨重试的 thunk ID 都不同。P9 去掉 metadata 后这些 key 会冲突，统一改用 execution_id |
| F22 | scheduler 的 `calc_thunk_obj_id` 是第三种 ID 算法：直接序列化 ThunkObject，而 `params` 是 HashMap，多参数时键序不稳定 | [thunk_runner.rs:595](../src/kernel/scheduler/src/thunk_runner.rs#L595) | T1g 统一成 ThunkObject 的公共方法 |
| F23 | FileObject 的 ID 包含 name、create_time、last_update_time、owner 等字段；`cacl_file_object` 把文件 mtime 写入时间字段，`put_file` 由此生成 ID | [bucket_client.rs:183](../src/kernel/buckyos-api/src/bucket_client.rs#L183)、ndn-toolkit `cacl_file_object` | 相同字节可能得到不同的 FileObject ID：输入直接进 params 会漏命中，产物用 put_file 会让重复转换的 ID 不同。输入改用 FileObject.content，产物固定时间字段和名称，见 §2.3 |
| F24 | scheduler 从 FunctionObject 读取 `requirements` / `best_run_weight` / `affinity_type`；RunThunk 请求和 ThunkTaskData 都携带 FunctionObject | [thunk_runner.rs:274](../src/kernel/scheduler/src/thunk_runner.rs#L274)、[316](../src/kernel/scheduler/src/thunk_runner.rs#L316)、[349](../src/kernel/scheduler/src/thunk_runner.rs#L349)、[scheduler_client.rs:34](../src/kernel/buckyos-api/src/scheduler_client.rs#L34)、[taskdata.rs:726](../src/kernel/buckyos-api/src/taskdata.rs#L726) | 拆分字段时要同步这些类型；只改数据来源，不增加调度能力 |

## 2. 总体方案

```text
llm_understand_media（agent runtime）
  │ 1. 保留原始对象引用，轻量 probe；模型支持且预算允许 → 直传，不建 Run
  │ 2. 确需转换：local_file / URL 冻结为 obj_id，准备输入读取授权
  │ 3. create_run(media.<family>, ensure=true, input={obj_id, params})
  ▼
Workflow Service（Zone 服务）
  验证调用者与输入访问 → 解析 definition_revision / 有效参数 / 本机执行环境
  ensure 只合并同一授权范围内的活动 Run；Completed 不复用
  Run → capabilities → probe / render / extract… → build_catalog
        executor: /tool/media.<fn>@<ver> → 真实 FunctionObject obj_id
        构造 ThunkObject → 按 thunk ID + 执行指纹查 Step cache，命中前校验结果及引用产物；未命中才投递 LocalThunkRunner
        Native 执行 → 产物 bucket + 结果 Named Object → 结构化回执 → 后继激活
        Run / Step / Thunk → task_manager，记录请求者、执行者、进度及终态
  ▼
llm_understand_media
  │ 4. runtime 保存 run_id；短等待后 Pending，后续查询同一个 Run
  │ 5. 取得有读取授权的 manifest / 产物 → 检索与视觉覆盖 → 模型分析
  │ 6. details.media_decision / pipeline_trace / coverage
```

边界：本期管线生成页图、分页文本、帧、提取音轨和目录 manifest。ASR / OCR 模型调用与后续分析不放进本期 Native 管线，音频沿用 agent_tool → AICC ASR；不声称已经交付 `media.audio` family。跨主体只共享有效结果缓存，不保证首次并发只执行一次。workflow 与 scheduler 同为系统正常工作的内核前提，不为 workflow 故障建设第二套媒体转换路径。

### 2.1 设计决策与推荐方案

D 编号保留以便追踪。以下已吸收 review 及用户在 §7 的答复，作为实施基线；§7 保留决策原文和落地解释。

| # | 决策 | 采用的方案与取舍 |
| --- | --- | --- |
| D1 | **转换执行位置** | P0 采用 workflow 进程内 `LocalThunkRunner` 执行 Native FunctionObject。Agent 容器端点削弱独立性；新增 media service 增加部署成本；直接接 Scheduler 会扩大为分布式执行项目。三者本期不做，D16 保留迁移接缝。 |
| D2 | **输入身份** | 转换请求以 NDN obj_id 标识；URL / local_file 先冻结内容，直传不强制上传或算 hash。进入 thunk params 的是 FileObject.content（chunk / chunklist ID），由服务从已授权的 FileObject 读取，调用者不能直接指定；FileObject ID 留在 run meta。scratch 默认 7 天，但运行期间另有输入保留保障。读取授权放请求执行上下文，不进入纯函数参数、缓存键或 manifest。 |
| D3 | **复用与读取权限** | 采用既有权限体系：校验调用者有权读取输入，交付时授予精确产物的读取权。知道 obj_id 不自动获得权限。纯转换结果可跨 Run 复用，缓存命中也不能跳过授权；请求者凭据不写共享缓存。具体见 §2.4。 |
| D4 | **缓存身份与纯度** | key 由 thunk ID（真实 fun_id + 规范化有效参数，含输入内容身份）及该函数实际依赖的执行指纹决定。Native 行为改变必须升级 implementation revision。幂等决定重试安全，纯度决定结果能否缓存，允许幂等但非 pure；缓存读写统一门禁，见 §2.3。 |
| D5 | **系统定义** | 内嵌 JSON，启动播种并 compile / analyze，失败则启动失败；source 由服务决定，客户端不得自报 system。系统定义只读、仅随服务发布升级；调用仍需认证、输入授权和资源准入。系统定义归属是服务内建标记，不是可登录的 system 用户。 |
| D6 | **ensure** | 只合并活动 Run，不复用 Completed / Failed / Aborted。key 包含服务端确定的授权范围、definition_revision、规范化有效输入和执行环境 revision；查找与登记原子化。Completed 后新建 Run，重新通过 Step cache 复用结果。scope 必须包含调用 principal；不同主体不共用活动 Run，仅复用有效 Step cache（R1 已确认）。 |
| D7 | **留存与重启** | P0 不做完整 Run 续跑或跨重启历史恢复；缓存索引与产物持久化。重启清理旧实例本地 Native 媒体执行持有的悬挂 Task / 临时目录，runtime 遇 not_found 后可预算内重建 Run。产物默认 TTL 30 天；Run / Task 元数据独立回收，不将 30 天 TTL 宣称为跨重启历史保证。 |
| D8 | **等待与 Pending** | 短等待后返回 Pending(LongRunning, check_after)。runtime 保存 run_id、输入身份、有效参数和 revision，后续查询原 Run；首次请求、not_found 恢复或明确 retryable 的失败才重新 ensure，受总 deadline / 恢复次数限制。LLM 无需管理这些状态。 |
| D9 | **Office 与交付范围** | 先完成 image / PDF 的垂直验收，再交付 video / document。DOCX / PPTX / ODT 使用 soffice→PDF；页码明确属于派生 PDF。XLSX 延后，不把 PDF 当成公式 / sheet / 单元格语义；本期不引入 Office 解析 crate。 |
| D10 | **异步与取消** | 本期完成 thunk 路径异步驱动，service:: direct adapter 改造后置。有效 deadline 取 Step、Function、Run 剩余额度中的最早值；区分排队期限与执行超时。必须停止并回收外部进程、清理临时文件、处理迟到回执；仅包 tokio timeout 不算完成。 |
| D11 | **持久化分层** | FunctionObject / thunk / 结构化结果 / manifest 是不可变 Named Object；`cache_key → result_obj_id / artifact_refs / expires_at` 是持久索引，不能以输入 hash 冒充结果记录的内容 ID。采用 service-local SQLite（复用 workspace rusqlite）保存索引，所需依赖已由 §7 R3 授权；内容对象仍进入 Named Store。 |
| D12 | **定义身份与版本** | 系统逻辑名称稳定为 `media.<family>`；新增 `definition_revision = hash(canonical(definition + resolved fun_ids))`，不含时间戳或进程内版本计数。Run 固定 revision 和 compiled 快照；plan_version 仅用于 Run 内 amendment。用户定义原有身份规则不在本任务中泛化改写。 |
| D13 | **请求者、执行者与权限** | 逻辑 owner / requested_by 从 session 校验取得，params.owner 只作一致性声明；Workflow 是实际执行主体，Task creator / executor 如实保留服务身份，任务数据携带请求者供展示审计。查询、启动、取消、提交输出、amendment 与产物交付都检查授权，Task ACL 与 Run 可见性一致；不能只修 create / submit。 |
| D14 | **失败与错误通路** | 本批媒体定义显式 max_attempts=1、fallback=abort；结构化错误贯穿 Native → ThunkExecutionResult → orchestrator → Run / Step / Thunk Task / RPC。无文本、无音轨、部分页失败按 §3.1 输出条目状态；不把所有 source:system 流程一概禁止 human。 |
| D15 | **产物位置与有效性** | 文件进 workflow 的 artifacts bucket，默认 temp / 30 天；manifest 与结构化结果进 Named Store并受保留策略管理。cache hit 由编排器验证引用产物的实际可用性及消费期，失效当 miss。返回稳定 obj_id / locator 与单独的访问凭据、expires_at；凭据不进 manifest。 |
| D16 | **runner 接缝** | 实现 LocalThunkRunner 与现有 ThunkDispatcher 契约，执行表仅开放内置 Native。后续 Scheduler / Node Runner 还要处理实际执行节点指纹、数据访问、任务交接和取消，不能承诺只换 dispatcher 即完成迁移。 |
| D17 | **FunctionObject schema**（P9） | FunctionObject 只保留决定结果和调用契约的字段：func_type（增加 Native）、content（带 implementation revision 的注册 id）、is_pure、params_type、params_schema、result_type、result_schema。schema 必须实际校验输入与输出；params_type 仅保留对象引用检查元信息，并与 schema 一致性校验。name / version / description / timeout / requirements / best_run_weight / affinity_type 移到不进 hash 的 FunctionRegistration。requirements 是准入条件，不代替精确缓存指纹。字段表见 §2.3。 |
| D18 | **FunctionRegistry** | 本期进程内 registry，发布内置 FunctionObject 到 Named Store，保存 FunctionRegistration，解析 name@version → fun_id；list_functions / get_function 合并返回两者。Zone 级 registry、Agent / pikg 发布后置；没有解析到函数在编译期报 unresolved_function。 |

### 2.2 依赖方向与独立开发边界

```text
                 buckyos-api（协议、客户端、manifest、bucket）
                    ▲                         ▲
  workflow（kernel，Phase A）          agent_tool（frame，Phase B）
  └─ functions/media/*                 └─ llm_understand_media
  ✗ 不依赖 agent_tool / llm_context / libopendan / opendan / nfs_server / aicc crate
```

1. 媒体 Native 代码位于 `src/kernel/workflow/src/functions/media/`。复制必要的 image_fit / preview 工具探测和执行控制逻辑，不依赖 frame crate；不在本期新增通用 media_tools crate。第三方依赖见 §7 R3。
2. agent 仅通过 buckyos-api 的 WorkflowServiceClient 和既有数据面消费转换结果。共享契约包括 manifest、错误与交付响应；类型放 buckyos-api，并同步 websdk。确需转换时统一调用 workflow，不保留因 workflow 不可达而启用的本地 image_fit / 抽帧替代路径。
3. 测试三层：纯单测用内存对象库 / 缓存索引、NoopTaskTracker、Native test.echo / test.sleep / test.fail；工具测试为显式 `#[ignore]`；DV 使用 `test/workflow_test` 与真实 bucket。前两层不需要 Zone，三层都不需要 OpenDAN 容器。
4. 媒体函数的运行需求先定义工具基线（§2.5），再驱动 workflow 宿主预置和 paios / agent-runtime 分发。完整四 family 的正式交付必须满足对应工具、版本与字体要求；缺工具返回 tools_missing，作为部署诊断和故障注入用例，不作为正常安装默认缺功能的方案。服务仍可启动以提供诊断。本期 workflow 不依赖 tesseract。
5. Phase A 包含必要的服务 bucket 声明、身份 / Task 镜像、打包与 SDK 联动；默认复用既有 NFS 授权 API。若需要数据面新增协议，先补充本 TODO 的具体影响，不将其隐藏在“只改 workflow”范围内。

### 2.3 FunctionObject、缓存与 thunk 执行契约

**意图级函数。** `render_pdf_pages@1(content_id, dpi, max_long_edge, page_range)` 描述转换结果，命令及参数拼接仅在 Native 实现内。首批 9 个函数：capabilities、probe、fit_image、extract_pdf_text、render_pdf_pages、extract_frames、extract_audio、to_pdf、build_catalog，均在 `media.` 命名空间。

**字段设计原则（P9）。** 对象 hash 只回答一个问题：同样的输入会不会得到同样的结果。字段变化后旧结果不再可信，才进 hash；其余字段变化不能让已有缓存失效。

| 字段 | 位置 | 依据 |
| --- | --- | --- |
| func_type、content（implementation revision） | FunctionObject | 决定怎么计算 |
| is_pure | FunctionObject | 决定结果能否复用；改为 false 时旧结果应随之失效 |
| result_type、result_schema | FunctionObject | 决定缓存结果的形状 |
| params_type、params_schema | FunctionObject | 调用契约。单独变动很少见，变动时最多重算一次，换来 fun_id 能固定完整接口 |
| name、version、description、schema 说明文字 | FunctionRegistration | 查找入口和描述 |
| timeout、requirements、best_run_weight、affinity_type | FunctionRegistration | 决定能否运行、在哪运行，不改变结果 |
| fun_id、规范化后的 params | ThunkObject | 计算身份 |
| run_id、node_id、attempt、execution epoch、FileObject ID、请求者、读取授权 | run meta（ThunkExecution） | 一次执行的事实 |
| 执行预算（排队 / 执行时间、临时磁盘等）、metrics、耗时 | run meta | 约束或记录执行，不决定结果 |

FunctionRegistration 不是 Named Object。Run 绑定函数时把它快照到 run meta，执行和展示都读快照。

下面是拟议结构，正式字段与序列化在 T1g 定稿。

FunctionObject（进 hash，得到 `func:<hex>`）：

```json
{
  "func_type": "native",
  "content": "media.render_pdf_pages/impl-1",
  "is_pure": true,
  "params_type": {"content_id": {"CheckByRunner": "file_content"}},
  "params_schema": {
    "type": "object",
    "required": ["content_id", "dpi", "max_long_edge", "page_range"],
    "additionalProperties": false,
    "properties": {
      "content_id": {"type": "string"},
      "dpi": {"type": "integer", "minimum": 1},
      "max_long_edge": {"type": "integer", "minimum": 1},
      "page_range": {"type": "array", "items": {"type": "integer", "minimum": 1}}
    }
  },
  "result_type": {"Object": "media.page_images"},
  "result_schema": {
    "type": "object",
    "required": ["pages"],
    "properties": {"pages": {"type": "array", "items": {"type": "object"}}}
  }
}
```

FunctionRegistration（不进 hash）：

```json
{
  "name": "media.render_pdf_pages",
  "version": "1",
  "fun_id": "func:<hex>",
  "description": "将指定 PDF 页渲染为页图",
  "timeout": 120,
  "requirements": {"cmd/pdftoppm": 22, "runner/workflow": 0},
  "best_run_weight": {},
  "affinity_type": "Input"
}
```

ThunkObject（进 hash，得到 `thunk:<hex>`）：

```json
{
  "fun_id": "func:<hex>",
  "params": {
    "content_id": "<chunk 或 chunklist ID>",
    "dpi": 144,
    "max_long_edge": 2048,
    "page_range": [1, 2, 3]
  }
}
```

ThunkExecution（run meta，不进 hash）：

```json
{
  "execution_id": "<run_id>/<node_id>/<attempt>",
  "thunk_obj_id": "thunk:<hex>",
  "run_id": "<run_id>",
  "node_id": "render_pdf_pages",
  "attempt": 1,
  "execution_epoch": 3,
  "source_obj_id": "cyfile:<hex>",
  "requested_by": "<principal>"
}
```

示例中的 result items 需按 §3.1 补齐；函数级上限与服务资源配额在 T2 校验。实际对象身份统一使用 NDN 的 canonical 编码 / NamedObject 生成规则，不能依赖 HashMap 遍历顺序。

1. **真实身份与版本绑定。** 内部 fun_id 使用 ObjId，wire 使用完整对象 ID（如 `func:<hash>`）；`func::<obj_id>` 中的占位符是完整 ID，解析一次后不再 hash executor 字符串或重复添加类型前缀。FunctionObject、Thunk、result 各自生成并保存同一组 ID + canonical bytes；修正 F16 的 thunk 类型和投递身份不一致。thunk ID 只由 fun_id 和规范化 params 决定，相同计算跨 Run、跨重试 ID 相同。Thunk 的 params 来自运行时输入，ID 统一由 ThunkObject 公共方法经 `try_build_named_object_by_json` 生成，scheduler 的 `calc_thunk_obj_id` 同步改用（F22）。Native 实现行为改变时必须变更 content 的 implementation revision；老 Run 不悄悄绑定新函数。无法提供旧实现时明确失败，不用相同注册名替换其语义。
2. **编译与执行校验。** registry 在播种 / submit_definition 时解析 `/tool/<name>@<version>`，固定 fun_id 和 definition_revision。输入 schema、ObjId 类型 / 可访问性 / 内容可用性、requirements、native 实现均在执行前检查；输出先通过 result_schema 才能成功回灌和写缓存。params_schema 是参数结构权威，params_type 只补充对象引用语义，冲突使注册失败。进 hash 的 schema 不得包含 description / title / examples / default，注册时拒绝；说明文字放 FunctionRegistration，默认值在构造 thunk 前填好。
3. **缓存资格。** 本期保留 DSL bool，本批媒体 JSON 必须显式填写 idempotent，播种检查原始 JSON 中字段存在。FunctionObject 路径的缓存读写统一要求 `step.idempotent && function.is_pure`；不引入 `idempotent_on_impure_function` 禁止规则。capabilities 可安全重复，但读取动态环境，`is_pure:false`，永不缓存。其余函数只有在执行环境明确、易变元数据已处理且通过确定性验证后才声明 pure，否则禁用缓存。纯度门禁之外还要检查本次结果：含 retryable 条目错误或执行中断的 partial 可交付，但不写该步骤的可复用缓存；该不可缓存状态沿依赖传递到 catalog。确定的 no_text / absent 和明确请求范围不因此失去缓存资格。全 DSL 的缺省 / unknown 与外部 executor 自动重试治理继续记录在 workflow_idempotency_review，不假称本期已解决。
4. **有效执行规格。** `step_cache_key = hash(thunk_obj_id, relevant_execution_fingerprint)`，thunk params 就是 effective_input。effective_input 补齐默认值并规范化范围（如把页码展开成升序去重列表）；函数依赖的格式写成显式参数，不从 FileObject.name 读取。fingerprint 只保留函数实际依赖的工具版本 / 实现环境，不能把本机所有工具塞给每一步。与内容相关的字体、locale、编码配置等应明确纳入或固定，不能仅凭最低主版本声明确定性。`requirements` 检查“能否运行”，fingerprint 标识“以哪个环境运行”。业务函数输出不混入 run_id、时间戳、临时路径或授权凭据。
   - **输入身份：** params 使用 FileObject.content，相同字节换文件名、owner 或 mtime 后仍能命中。chunk 切分或 hash 算法不同时算作不同内容，只会少命中，不会错用结果。
   - **产物身份：** 产物 FileObject 的时间字段固定为 0、名称稳定，不能直接用 `put_file`（F23）；[`store_reader`](../src/kernel/buckyos-api/src/bucket_client.rs#L324) 路径符合这个要求。否则重复转换的 obj_id 不同，build_catalog 永远无法命中。
   - **limits：** 决定产出范围的 limits 先换算成 params（如 page_range）；只约束执行的留在 run meta。被执行预算截断的结果不写可复用缓存；命中结果超出本次 limits 时返回 resource_limit，不重算。
5. **环境一致性与 ensure。** create_run 匹配活动 Run 前，服务取得该定义所需环境 revision；Run 冻结该快照，capabilities Step 读取并核对该环境，输出可观察事实。每个 Native 执行前再次核对其相关指纹；若工具在等待期间被替换，返回 environment_changed，不以旧 key 缓存新环境结果。新请求的 ensure key 使用新环境 revision，runtime 按 D8 在预算内决定重建。远端执行时指纹必须来自实际执行节点。
6. **持久索引与命中验证。** 不可变对象按内容身份保存，缓存索引保存 `key → result_obj_id / artifact_refs / expires_at`。读命中前核验结果对象及其引用文件的实际可用性；manifest 存在不代表页图或音轨还在，bucket head 成功也需检查 Availability。失效删除索引并重算；不能将检查放在不会被 cache hit 调用的 Native 实现中。metrics 和耗时只写 run meta，不进结果对象和缓存索引（F18）。交付时另行授权与保障消费期，索引中无调用者 token。
7. **投递与回执。** Run 先入 RunStore；pending attempt 和 thunk Task 关联完成后才允许结果消费。schedule_thunk 只完成有界队列投递，不能在 Run 锁内等待执行槽或运行转换。runner 读取 FunctionObject、校验输入、获取并发槽并执行；结果落对象库后，通过结果队列回灌、更新 Task、推进 tick。投递失败使对应 attempt 明确失败；快速、重复、迟到回执按 execution_id / execution epoch 去重，终态后不复活 Run、不写入未经接受的缓存。thunk ID 不再唯一对应一次执行，`pending_thunks`、thunk Task、`schedule_thunk` / `cancel_thunk`、`ThunkExecutionResult` 都改用 execution_id（`<run_id>/<node_id>/<attempt>`，for_each 分片的 node_id 已含下标）关联（F21）。同一 thunk 可以在多个 Run 中同时执行，本期仍不做 single-flight（R1）。
8. **Task 与错误。** `workflow/run → workflow/step → workflow/thunk` 全链路记录状态；sync_thunk 的现有“只创建”逻辑不代表执行状态已实现。runner 领取并更新 thunk Task、进度和终态。统一错误 `{code, message, details, retryable}`，details 可含 tools_missing / tool / exit_code / stderr_tail；ThunkExecutionResult 的字符串 error 必须同步升级，并原样保留到任务与 RPC。
9. **deadline 与资源释放。** 函数 timeout 单位统一为秒，执行 deadline 不超过 Step / Function / Run 的最早期限；排队也受 Run deadline 约束。外部命令超时或取消后终止进程及其受管子进程并 wait 回收；CPU 图像计算放入受控工作线程，并有输入上限和取消检查，不能声称 timeout 能抢占任意同步计算。清理临时文件、释放槽后再完成失败回执。Tokio 的 timeout / 子进程默认行为见 [timeout](https://docs.rs/tokio/latest/tokio/time/fn.timeout.html) 与 [kill_on_drop](https://docs.rs/tokio/latest/tokio/process/struct.Command.html#method.kill_on_drop)。
10. **后续迁移。** 维持 ThunkDispatcher 接缝，Scheduler / Node Runner 版本再补远端授权、数据传输、指纹与任务交接。不做 Script / ExecPkg / OPTask runner、scheduler 选节点、Zone registry 或 Agent 发布。node_exector.md §6 的五项条件按本地 Native 子集回答，不能据此标成通用分布式执行已解决。

### 2.4 输入授权、结果交付与生命周期

- 输入 obj_id 的存在性不等于请求者有读取权。使用已验证 caller 检查输入访问，并给本次执行建立所需保留引用；workflow 的服务权限不得绕过请求者授权。凭据及其更新留在执行上下文，不能影响纯转换内容身份。
- 视频输入的目标形态（2026-11-05 用户补充）：ffprobe / ffmpeg 不再落临时文件，直接读标准 NDN URL（HTTP + Range）：`/ndn/<file_id>?hint={"token":"bkt_..."}`。thunk params 仍只有 `content_id`；FileObject ID 和令牌留在执行上下文，不进缓存键、manifest 和 stderr 详情。poppler / soffice 继续用 `materialize_input`。协议见 [Bucket 设计 §4.6.5](../doc/arch/应用对象桶%20Bucket%20设计.md)，主任务是其 §21 的 N17。
- 产物文件写入 workflow artifacts bucket；key 建议为 `media/<step_cache_key>/<name>`，避免同一转换因外层 family、调用者或函数注册名不同重复写入；非 pure 函数的结果不可复用，key 改用 execution_id，避免并发 Run 互相覆盖。结果与 manifest 的授权同样需要覆盖，不能只给目录本身权限。
- 复用既有 bucket 精确 key token / grant 能力；[preview.read_token](../src/frame/nfs_server/src/preview/mod.rs#L334) 是参考。响应交付层携带授权和 expires_at，稳定 manifest 只含 obj_id / locator。token 到期允许在重新授权后刷新，不触发转换重算。
- scratch 默认 7 天、产物默认 30 天；正在执行的输入和已承诺交付的产物需保留到相应执行 / 消费期结束。引用与 lease 在终态或消费期结束后释放；命中检查和交付之间也要有保留保障，避免刚检查即被 GC。
- Run 内存态按单独的数量 / 时间上限回收；Task 元数据按独立策略回收。每次 workflow 启动持久记录 execution epoch 与执行归属，只将旧实例本地 Native 媒体执行持有的 Run / Step / Thunk Task 置 Failed（code: interrupted）；确认其受管进程已停止后清理临时文件。不批量终止其他 workflow / schedule、人工等待或其他服务持有的任务。管理员可看到中断事实，但本期不保证恢复原媒体 Run 图和历史。
- ensure 的授权范围由服务端确定；按已采纳的 R1 A，scope 必须包含认证后的调用 principal 身份及相关 owner / ACL 边界，同 owner 或相同输入读取权不能使不同 principal 合并。本期不引入跨 principal 的共享 Run / 订阅者控制模型。客户端不能指定一个“公共 scope”绕过隔离；管理员观察使用既有管理权限，普通调用者只能查看和操作获授权的 Run。

### 2.5 由 workflow 需求驱动的媒体工具基线（P8）

先从本期 Native 实现和 FunctionRegistration 的 requirements 定义命令、最低能力 / 版本、探测方式、字体与 locale，再同步 agent-runtime / paios 的预置清单。该清单是现有 runtime baseline 的媒体部分，不重新定义其他 Agent 工具。

| 管线能力 | 最小运行需求 | 分发与验证 |
| --- | --- | --- |
| capabilities / build_catalog / 常规图片 probe 与 fit_image | workflow Native 实现及 image crate | 编译依赖随 workflow 发布；本批函数不因此要求额外安装 ImageMagick |
| PDF 探测 / 文本 / 页图 | pdfinfo、pdftotext、pdftoppm（poppler） | 验证版本与真实 PDF 提取 / 渲染，不只检测命令存在 |
| 视频探测 / 抽帧 / 音轨提取 | ffprobe、ffmpeg | 验证真实带音轨 / 无音轨视频、输出编码与版本 |
| DOCX / PPTX / ODT 转 PDF | soffice（LibreOffice 所需 headless 组件）、配套字体和 locale | 完整 document family 交付时必须具备；分别验证文档与演示文稿转换，记录影响输出的环境指纹 |

当前 build_aios 已有 poppler / ffmpeg，未包含 LibreOffice；[paios 容器需求](../doc/app_service/paios容器需求.md) 仍写“不含 libreoffice”，应随 T4b 回写。现有 ImageMagick / tesseract 服务于其他场景，保留其现有用途，本期不将它们升级为 workflow 的必需依赖。

P0 执行位置仍是 workflow 宿主机的 LocalThunkRunner。宿主安装 / 健康检查与 paios 镜像分别满足同一媒体需求；镜像可用不证明宿主 runner 可用。需求清单及检测规则放在双方可消费的中立位置，kernel 不依赖 libopendan。官方命令清单继续以 paios 文档 §6.4 为维护入口，同步构建、runtime 探测和未来 ext-core 的 provides，避免再维护互相矛盾的手写清单。

## 3. 系统内置管线定义（P0 清单）

四条 family 分两批落地：先 image / PDF，后 video / document。定义逻辑 id 稳定，revision 按 D12 计算。

| family | 有效输入 | steps（全部 autonomous） | Run output |
| --- | --- | --- | --- |
| media.image | `{obj_id, max_long_edge, max_pixels, max_bytes}` | capabilities → probe → fit_image | `{model_media_obj_id, original, sent, actions}` |
| media.pdf | `{obj_id, dpi, max_long_edge, page_range?, limits}` | capabilities → probe → extract_pdf_text → render_pdf_pages → build_catalog | `{manifest_obj_id, summary}`，详细页条目在 manifest |
| media.video | `{obj_id, strategy:uniform, max_frames, frame_max_edge, limits}` | capabilities → probe → extract_frames → extract_audio → build_catalog | `{manifest_obj_id, summary}`；无音轨为正常 absent，ASR 留在 agent_tool |
| media.document | `{obj_id, dpi, max_long_edge, page_range?, limits}` | capabilities → probe → to_pdf → 复用 PDF 后三步 | 同 media.pdf；同时保留原对象和派生 PDF 身份 |

每 step 的 executor 写 `/tool/media.<fn>@1`，本批操作显式 `idempotent:true` 与 `guards.retry={max_attempts:1,fallback:abort}`。capabilities 为幂等但非 pure，按 §2.3 永不缓存；cache_hit 验收仅针对具备缓存资格的转换步骤。调用者不能通过输入关闭服务上限；limits 只可在服务上限内进一步收紧。to_pdf 的确定性未验证前不得标 pure 或承诺命中缓存。

### 3.1 manifest、部分成功与覆盖范围

manifest 类型在 buckyos-api 定稿，至少包含 schema_version、原始源内容 ID、派生源内容 ID、总范围、请求范围、实际处理范围，以及每条 entry 的 `{kind, where, text_obj_id?, image_obj_id?, audio_obj_id?, status, error?}`。页码 / 时间戳必须说明所属源；空白页、无文本页、未处理页、失败页均有明确状态，不用条目消失掩盖缺失。与调用时间相关的凭据、expires_at 和运行耗时留在交付 / trace 层。build_catalog 是可缓存步骤，FileObject ID、workflow_id、definition_revision 放在 Run output / run meta，不进 manifest，否则不同定义版本或文件名无法复用同一份 manifest。

- **正常缺失**：扫描页无可提取文本、视频无音轨，返回 no_text / absent，函数成功，不触发 Run abort。
- **部分成功**：页级失败记在 entries，仍能构建 manifest；summary / coverage 明确 incomplete。含临时错误的 partial 不写整体可复用缓存，重试可复用已有合格缓存的成功步骤；没有页级缓存时允许重算整个失败步骤，不假称已经实现页级恢复。若某一步使整批无法处理或没有可用表示，则返回结构化失败，不伪装完整成功。
- **整体失败**：输入不可读、必需工具缺失、超时或资源超限，Run Failed，错误贯穿回执与 Task；用户主动取消保持 Aborted / Cancelled 语义。
- **选路与故障边界**：正常情况下按模型能力与预算选择直传或转换；具体格式处理失败后，只有模型确实支持原格式且预算允许才可尝试直传。workflow 不可达属于内核服务故障，直接报告，不触发本地转换或专门的直传容灾路径。缺少基线工具报告部署问题；已知模型不支持时不重复尝试。扫描 PDF / 纯视觉视频不能只做词法检索，Agent 必须覆盖无文本条目的视觉内容或明确报告未覆盖。
- **输出形状**：Run output 是终端节点 id → 结果的映射；这四条定义各一个终端节点，客户端按契约读取 fit_image / build_catalog 的结果。Run / history 不内联媒体字节。

### 3.2 执行资源上限

T2 设置并记录输入字节、最大页数 / 帧数、单页及累计像素、累计输出字节、临时磁盘、队列长度、排队与执行时间上限。超限返回 resource_limit，并在能形成部分结果时准确报告覆盖范围。默认值通过 fixtures 与开发机测量确定，不能仅用 CPU 核数个并发推断安全容量。

参考 preview 的执行控制而非只复制 tools.rs：ffmpeg / PDF 转换采用保守的独立并发槽，soffice 初始并发 1、每次调用独立 profile；image CPU 工作不占用异步 runtime 的执行线程。stdin 关闭，stdout / stderr 有界读取，外部命令使用参数数组，临时目录按 execution_id 隔离，成功 / 失败 / 取消 / 重启均有清理路径。

## 4. 实施任务

顺序：**T1g 的 test.echo 真实闭环 → T1 通用能力与 T2c 工具基线 → image / PDF 垂直验收 → video / document → Phase B 接入与 paios 工具交付**。T1g 可先用内存依赖验证，最终验收必须使用真实 Named Store、缓存索引和 TaskMgr。

Phase A 以 `src/kernel/workflow`、`src/kernel/buckyos-api`、同级 `buckyos-websdk`、`test/workflow_test`、`src/check.py`、必要部署声明、宿主工具预置与文档为范围，独立于 OpenDAN。Phase A 通过 RPC 与 TaskMgr 验证可观察性；Desktop Workflow WebUI 的真实接入在 Phase B T5，不将其列为 Phase A 已完成能力。

### Phase A：workflow service 跑通

- [x] **T0 文档回写**（第七轮已回写：设计文档 v0.5、executor list §7、workflow service §2–§7、wokflow engine §3.11 / §4.7、idempotency review、node_exector §6、task data schema、新增 doc/workflow/media tool baseline.md）：设计文档 §4.5 / §4.8 改为系统内置 Workflow + FunctionObject，移除“不进 Workflow Service”和 preview 组件复用口径；同步 §4.6 / §4.7 / §8 的授权、manifest、Pending、coverage 与正常选路语义，删除 workflow 不可达仍需完成媒体理解的要求。[executor list](../doc/workflow/executor%20list.md) 写 media FunctionObject / Native / registry，不新增 service::media adapter；[workflow service](../doc/workflow/workflow%20service.md) 补 source、definition_revision、ensure、持久索引及错误契约；[workflow_idempotency_review](workflow_idempotency_review.md) 区分重试安全与纯度；[node_exector.md §6](../doc/arch/node_exector.md) 写本地 Native 子集的边界。[wokflow engine](../doc/workflow/wokflow%20engine.md)、node_exector.md 和 [task data schema](../doc/task_mgr/task%20data%20schema.md) 改为 ThunkObject 不含 metadata、回执按 execution_id 关联（P9）。实现完成后再标注“已落地”，不以本 TODO 代替实现事实。
- [x] **T1g FunctionObject / thunk 最小闭环**（已落地：thunk_object.rs 拆分、function_registry.rs、local_thunk_runner.rs、execution_id 回执；单测 `orchestrator/tests.rs`、`server/tests.rs::echo_runs_end_to_end_through_the_local_runner_and_caches_the_second_run`）：按 §2.3 拆分 FunctionObject / FunctionRegistration，ThunkObject 去掉 metadata，新增 ThunkExecution；新增 function_registry.rs / local_thunk_runner.rs；统一真实 fun_id、thunk ID 和 canonical 编码，修复 F16 / F22；pending、thunk Task、dispatcher 和回执改用 execution_id（F21）；同步 scheduler_client、ThunkTaskData 与 thunk_runner 的类型，只改数据来源，不增加调度能力（F24）；编译期解析语义引用并校验 schema；提供 list_functions / get_function；Run 入表、pending / Task 关联、投递、回执接受与终态有明确顺序；接通 thunk Task 执行状态。test.echo 覆盖对象读回与身份验证、成功、失败、即时 / 重复 / 迟到回执，以及同 Run 相同 Step、内容相同的分片和重试不串回执；main.rs 用真实 runner 替换记账 dispatcher。
- [x] **T1a 系统定义与 revision**（system_defs.rs + defs/media.*.json；`source` / `revision` / 只读）：source 使用 Desktop 同名枚举，服务端分配；内嵌 JSON 播种并校验本批媒体约束。系统逻辑 id 使用 DSL id，保留 media. / sys.；拒绝客户端修改 / 归档 system。按定义和已解析 fun_id 生成 revision，Run / get_run_graph 固定读取其快照；plan_version 不作定义版本。list / get 返回 source 与 revision，支持过滤。
- [x] **T1h 身份与交付**（每方法验证 caller、NdmInputAuthorizer、`get_run_output.delivery`；单测覆盖；“两个独立 principal 的真实数据面测试”属 DV，未运行；产物精确访问授权：`get_run_output.delivery.grants` 由 `NativeIo::deliver` 按 obj_id → bucket key（SQLite `artifacts` 表）签发绑定 principal 的精确 key 只读 token，凭据不进 manifest / cache；真实 bucket 路径未在 Zone 上跑过）（D3 / D13）：复用 WorkflowCallerVerifier，校验全部 Definition / Run 读写操作；区分请求者与服务执行身份，Task 数据与 ACL 同步。接入输入授权、运行保留引用及产物精确访问授权，凭据不写 manifest / cache / trace。两个独立 principal 的真实数据面测试证明“有权限可读、仅知道 obj_id 不可读”。
- [x] **T1b 活动 Run ensure 与 output**（RunStore::insert_or_ensure、终态释放、`output` / `reused` / `revision`）：按 §2.1 D6 原子化查找 / 登记，终态移除活动索引；同范围请求只建一个 Run，完成后新请求建新 Run。参数先补默认值，环境 revision 在匹配前取得。create_run / get_run_graph 返回 status / revision / output / reused（创建响应）；output 为终端节点映射，非终态不假称已有最终结果。list_runs 增加 status 过滤与可见性检查。
- [x] **T1c 异步、deadline 与资源控制**（driver.rs 后台 tick 与回执循环、lane semaphore、有界队列、Run / Step / Function deadline、子进程 kill+wait；单测 `step_timeout_kills_the_execution_and_fails_the_run_with_timeout`）：create_run(auto_start) / start_run 入表后后台驱动，RPC 尽快返回；Run 锁内只处理短状态变更，外部 I/O、执行槽等待与媒体计算不持锁。结果队列驱动回执与 tick；有界队列、分类型 semaphore、取消和期限传播；重复 / 迟到结果不改变终态。覆盖真实子进程超时回收，不能只测 sleep future。
- [x] **T1d 对象 / 缓存持久化与重启清理**（NamedStoreObjectStore、persistence.rs SQLite 索引 / epoch / 执行账本、cleanup_stale → interrupted；check.py 增加 workflow / 4070；系统定义且无 human 步骤 / human fallback 的 Run 不再 `watch_run`（`definition_accepts_human_actions`），取消经 `cancel_run` RPC；终态 Run 内存按数量 / 时长回收（`RunStore::prune_terminal`）；`artifacts` 表记录产物 key）：Named Store 保存不可变对象，独立 SQLite 持久索引按 D11 装配（rusqlite 依赖已授权）；缓存读写按纯度门禁，命中前检查产物闭包和保留期。按持久记录的 execution epoch / 归属清理旧实例本地 Native 媒体 Task，确认受管进程停止后清理临时目录；不终止其他 workflow / schedule 任务；Run / Task / artifact 回收分别实现。check.py 增加 workflow / 4070。仅对本批没有人工控制需求的媒体 Run 跳过 human-action watch_run；取消等控制仍经明确 RPC 路径。
- [x] **T1e 结构化错误与静态约束**（ThunkExecutionError 全链路、`validate_media_definition` 静态检查）：同步 WorkflowError、ThunkExecutionResult、Task payload、Rust / TS 客户端；code / message / details / retryable 全链路保真。本批媒体定义强制显式 idempotent、max_attempts=1、fallback=abort；约束按内置媒体定义集合判断，不泛化为所有 source:system。保留外部 executor 非显式幂等重试的分析提示，并注明 F8 的现有限制。
- [x] **T1f 客户端与共享类型**（Rust WorkflowServiceClient 补 definition / run / function 方法；websdk 同步 ensure / revision / output / cancelRun / getRunOutput / listFunctions；media_manifest.rs；Rust / TS schema 一致性靠 DV 用例，未运行）：Rust WorkflowServiceClient 补 submit_definition / get_definition / list_definitions / create_run / start_run / get_run_graph / list_runs / get_history / list_functions / get_function，并同步取消控制入口，返回 typed structs；websdk 同步 ensure、revision、output、错误、授权交付与过滤参数；media_manifest.rs 定义 §3.1，校验 Rust / TS schema 一致。RPC 鉴权与数据面读取分别测试。
- [x] **T2a image / PDF Native**（functions/media/{tools,capabilities,probe,image,pdf,catalog}.rs；poppler 真实测试为 `#[ignore]`，本轮未在带工具的机器上运行）：functions/media/tools.rs 复用 preview 三平台探测；实现 capabilities、probe、fit_image、extract_pdf_text、render_pdf_pages、build_catalog。只在需要时 materialize 输入；临时文件按 execution_id 隔离；产物入 artifacts bucket，FileObject 按 §2.3 保持确定性，结构化结果入 Named Store。缺工具、输入上限、空白 / 扫描页、单页失败都有契约；相关工具指纹进入缓存，Native 行为更新带 implementation revision。
- [x] **T2b video / document Native**（video.rs、document.rs；`to_pdf` 按 `is_pure: false` 注册，确定性未验证；ffmpeg / soffice 真实测试 `#[ignore]` 未运行）：实现 extract_frames、extract_audio、to_pdf；无音轨正常成功，Office 参数透传 PDF 页范围 / 预算，soffice 独立 profile 与并发槽。对 to_pdf 等输出处理易变元数据并验证重复转换的内容身份；未满足确定性则 is_pure:false，不误用缓存。完成全部 9 个 FunctionObject 发布。
- [x] **T2c 媒体运行需求与宿主交付**（`media_tool_baseline()` 从注册推导；doc/workflow/media tool baseline.md；check.py workflow 行；宿主安装脚本未加 LibreOffice / 字体，`get_capabilities` 作为能力诊断）（P8）：按 §2.5 从 FunctionRegistration 和 Native 实现推导媒体命令、版本 / 能力、字体 / locale、检测与冒烟样例，供宿主与 runtime 分发共用；同步 workflow 宿主安装说明 / 预置与 check.py 能力诊断。实际转换节点缺基线项时报告 tools_missing，不以 paios 检查通过替代宿主检查。此任务不依赖 OpenDAN 容器；镜像消费方在 T4b 完成。
- [x] **T2d 视频输入改走 `/ndn/`**（已完成，真实 ffmpeg 用例和 DV `WF-DV-014` 通过，见 [Bucket 设计 §21](../doc/arch/应用对象桶%20Bucket%20设计.md) 的 N17）：ffprobe / ffmpeg 改读标准 NDN URL；令牌在执行上下文，有效期覆盖 Step deadline，终态撤销；stderr 脱敏；限定 ffmpeg 的输入格式和协议。真实 ffmpeg 用例和 DV 要跑过才算完成。
- [x] **T3 内置定义与 manifest**（四条 JSON 播种，全部语义链接绑定真实 fun_id；MediaManifest 条目 / partial / coverage）：四条 JSON 位于 src/kernel/workflow/defs/media.*.json，include_str! 播种；每个语义链接均解析出真实 fun_id；媒体定义允许 capabilities 幂等但非 pure。media.document 的 Reference 使用派生 PDF 的 content_id，manifest 同时保留原源内容 ID；详尽 entries / partial / coverage / 页码语义按 §3.1，参数限制按 §3.2。
- [ ] **T6a Phase A 测试**（部分：workflow 单测 + server 集成测试已写并通过；DV WF-DV-009~013 已写入 testcases / workflow_dv.ts 但未在 Zone 上运行；真实工具 fixtures 用例为 `#[ignore]`；工具升级、产物过期、执行中重启有单测，跨主体授权 / 部分结果 / 资源超限的 DV 场景未跑）：单测覆盖 T1 全链路、授权、版本、缓存资格、ensure 终态行为和错误；Named Store / 索引测试校验 ID 与内容一致、跨进程重启命中。工具 fixtures 包含图片、多页尾页答案 PDF、扫描 / 空白页、部分失败页、带 / 无音轨视频、Office 文档；完整交付时 Office 工具验证不得因开发机缺安装而跳过。DV 延续 WF-DV-009~012（缓存、同范围并发 ensure、缺工具、RPC / TaskMgr 可观察），新增工具升级、产物过期、执行中重启、跨主体授权、部分结果和资源超限场景；WebUI 验收留给 T5 / T6b。不得依赖 OpenDAN 容器。

### Phase B：Agent 与 WebUI 接入

- [ ] **T4 agent_tool 接入**（进行中：`llm_understand_media/pipeline.rs` 新增，`llm_understand_media.rs` 尚未切换到 workflow；未验证）：resolve_media 保留 NamedObject 引用，轻量 probe，确需转换才冻结输入和流式读取；调用系统 family，按 D8 保存 run_id 并轮询，不反复 create_run 代替查询。读取获授权 manifest，词法检索配合无文本视觉覆盖，输出 media_decision / pipeline_trace / coverage。补正常选路所需的目标模型能力检查和媒体内容构造，不以容灾为由新增原生音视频直传能力；确需转换时只经 workflow。workflow 不可达返回内核服务故障，移除本地 image_fit / ffmpeg 转换拷贝，不保留双路径。更新 ui_core、xllm、aigc_* cmd_skills。
- [ ] **T4b agent-runtime / paios 基线联动**（部分：build_aios `RUNTIME_PACKAGES` 加入 LibreOffice nogui、字体、locales；paios容器需求 §6.3 / §6.4 已回写；镜像未重建、amd64 / aarch64 未探测、`baseline.rs` / cmd_skills 未改）（P8，消费 T2c）：同步 paios容器需求.md §6.4、build_aios 的 RUNTIME_PACKAGES / 构建检查、runtime tools_check / 基线声明及相关 cmd_skills；补齐 LibreOffice、所需字体与 locale。现有标准命令检查不能代替媒体工具验证；在干净 amd64 / aarch64 镜像中分别探测版本并运行 PDF / 视频 / Office 样例，未验证架构如实记录。ExtTools / ext-core 对应 provides 随其落地使用同一需求，不以尚未实现的扩展体系阻塞本期预置交付。
- [ ] **T5 WebUI 只读观测**（进行中：Desktop app/workflow 组件已在修改，mock 切换与只读态未验证）：Desktop app/workflow 从 mock 接 websdk 的 listDefinitions / getDefinition / listRuns / getRunGraph / Function 查询；系统定义禁用修改，Run 显示请求者 / 执行者、revision、各 Step 来源与错误、部分结果和中断状态，可进入 TaskMgr 三层树。尊重服务端授权；不通过 UI 隐藏代替只读保护。对照 Workflow WebUI 产品需求 §6.2 / §7.3。
- [ ] **T6b 完整验收**：执行 §5；明确实际测试过的 provider / 模型 / 平台与正常选路能力；不要求 workflow 故障时继续完成媒体业务。Phase A 的 RPC / TaskMgr 检查不替代真实 WebUI / Agent 验收。

第三方依赖：本任务所需新增依赖已由 §7 R3 授权，不重复请求确认；image 复用仓库版本与裁剪 features，SQLite 使用 workspace rusqlite，schema 校验优先复用已有实现，必要时按实际需求选库并记录版本和用途。依赖授权不扩展业务范围：XLSX / 新 Office 解析路线仍后置。本次仅更新 TODO，尚未修改 manifest、安装工具或构建镜像。

## 5. 验收

以下为验收标准；每项下的“状态（第七轮）”说明 2026-11-05 第七轮的实现 / 验证情况，“已实现”指代码与单测存在，不等于在真实 Zone 验收通过。

1. **顺序复用**：同一输入在相同执行规格下连续调用产生两个 Run；第二个 Run 的所有具备缓存资格且产物有效的转换步骤 source:cache，capabilities 重新执行。trace 区分两者，不要求全部 cache_hit:true。对已验证 pure 且命中缓存的步骤不产生重复媒体产物；非 pure 步骤允许重算 / 新产物，Run / 执行记录 / 事件元数据可新增。
    - 状态（第八轮 DV）：DV WF-DV-009 在真实 Zone 通过（test.echo 与 media.image 第二次 Run 的纯步骤 source:cache，capabilities 重跑）
    - 状态（第七轮）：已实现（单测 echo / image 用例）；真实工具链与 DV 未验证
2. **并发 ensure**：相同授权范围、definition_revision、有效输入和环境 revision 下并发请求只建一个活动 Run；终态不再占用 ensure 索引。不同 principal 即使同 owner 也各有 Run；已完成有效结果可跨 Run 复用，首次并发允许重复计算。
    - 状态（第八轮 DV）：DV WF-DV-010 通过（4 并发 ensure 只建一个 Run，终态后新建）
    - 状态（第七轮）：已实现（server 单测）；DV WF-DV-010 未运行
3. **缺工具诊断**：故障注入移除 poppler 时，对应转换 Failed，tools_missing 贯穿回执 / Run / Task，并指明基线未满足；正常交付要求工具齐全，不以本地备用转换掩盖部署错误。
    - 状态（第八轮 DV）：DV WF-DV-011 走 tools_present 分支（宿主已装 poppler / ffmpeg / soffice），tools_missing 分支由单测 `media_pipeline_without_tools_fails_with_tools_missing_and_validates_input` 覆盖
    - 状态（第七轮）：已实现（`media_pipeline_without_tools_fails_with_tools_missing_and_validates_input`）；真实故障注入 WF-DV-011 未运行
4. **真实 UI**：Phase B 后管理员可见四条只读系统定义、活动 / 终态 Run、各 Step cache 来源与结构化错误、TaskMgr 树；普通调用者看不到未授权 Run。
    - 状态（第七轮）：未实现（T5 进行中，未验证）
5. **统一转换入口**：在正常运行的 Zone 中，适合直传的内容按模型能力和预算直传，确需转换的内容通过 workflow 完成；agent 无本地备用转换通道。不设“workflow 不可达仍完成媒体理解”的验收要求；RPC 故障按内核服务错误返回。
    - 状态（第七轮）：未实现（T4 进行中）
6. **版本**：相关工具或 Native implementation revision 改变后不命中旧缓存；无关工具变化不使该 Step 失效。工具在排队期间改变时返回 environment_changed，不能将新结果写入旧规格的缓存。
    - 状态（第七轮）：已实现（指纹 / `environment_changed` 单测）；真实工具升级未验证
7. **history**：主干 history 只保存文本结果、摘要与对象引用，不内联媒体字节；manifest 的未覆盖范围和 partial 状态如实传递。
    - 状态（第七轮）：未实现（agent 侧）
8. **独立性**：cargo test -p workflow -- --test-threads=1 不依赖 Zone；uv run test/run.py -p workflow_test 在无 OpenDAN 容器的 Zone 上通过 Phase A 用例。分别记录真实工具与需要在线服务的测试结果。
    - 状态（第八轮 DV）：DV WF-DV-009~013 在无 OpenDAN 容器的 dev Zone 通过；`cargo test -p workflow -- --ignored` 3 个真实工具用例通过
    - 状态（第七轮）：`cargo test -p workflow -- --test-threads=1` 通过；`uv run test/run.py -p workflow_test` 未运行
9. **重启后缓存**：已完成转换的产物仍有效时，重启 workflow 后新 Run 的纯转换步骤继续命中持久缓存；对象读回通过 NDN 内容身份验证。
    - 状态（第八轮 DV）：真实 Zone 跨 `start.py` 重启后首轮 Run 直接命中（DV 因此把输入按 run tag 唯一化）；发现并修复 Named Store 未 pin 对象被 nfs-server GC 回收的问题
    - 状态（第七轮）：已实现（`persistent_cache_index_survives_a_service_restart` 单测）；跨真实重启未验证
10. **异步与超时**：一次长转换中 start_run 在 1s 内返回已接受状态，get_run_graph 能及时读取进度；较短的 Step / Function / Run deadline 生效，超时后受管进程退出并回收、临时文件和槽释放，迟到回执不能复活 Run。
    - 状态（第七轮）：已实现（后台 tick、step timeout 单测）；1s 返回在真实 Zone 未计时
11. **失败契约**：缺工具 / 超时 / 权限不足 / 资源超限分别有稳定 code；执行错误 details 在 Native、ThunkExecutionResult、Step / Thunk Task、RPC 中一致；本批媒体不会进入 WaitingHuman。
    - 状态（第七轮）：已实现（`structured_failures_reach_run_task_and_rpc_without_waiting_human`）
12. **依赖方向**：检查 cargo tree -p workflow 的依赖图，不得包含 agent_tool、llm_context、libopendan、opendan、nfs_server、aicc crate；不将 cargo tree -i 查询失败误当作依赖检查通过。
    - 状态（第七轮）：已核对：`cargo tree -p workflow -e normal`（1732 行）不含 agent_tool / llm_context / libopendan / opendan / nfs_server / aicc
13. **身份与只读**：A session 声明 owner:B 被拒；普通客户端提交保留 id / source:system 或归档系统定义被拒；读取 / 取消 / 修改他人 Run 被拒；Task creator 如实记录 workflow，requested_by 来自已验证 caller。
    - 状态（第八轮 DV）：DV WF-DV-013 用种子用户 bob 作第二主体通过：他人 Run 的 get_run_graph / get_run_output / get_history / cancel_run 均拒绝，list_runs 不泄露
    - 状态（第七轮）：已实现（`identity_is_verified_on_every_run_operation`、`system_definitions_are_seeded_bound_and_read_only`）
14. **真实 FunctionObject**：list_functions 列出 9 个 media 函数；完整 fun_id 可读到匹配的对象，编译 / Run / thunk 使用同一 ID；投递 thunk 可按其 ID 读回；thunk Task 能从创建走到执行和终态。
    - 状态（第八轮 DV）：DV WF-DV-012 通过：9 个 media 函数 published=true，TaskMgr 三层树可见（请求者获 WholeTree 读 grant），thunk 任务终态
    - 状态（第七轮）：已实现（`registry_publishes_nine_functions_and_baseline_lists_their_tools`、thunk 读回单测）
15. **意图化与绑定**：DSL / 对象不包含可执行命令模板；requirements 和诊断允许工具名称，不能用 grep pdftoppm 零命中作为标准。schema 或 implementation revision 改变后 fun_id 改变，老 Run 保持原绑定。只改 name@version、description、timeout、requirements、best_run_weight 或 affinity_type 时，fun_id 和已有缓存都不变。
    - 状态（第七轮）：已实现（buckyos-api `function_id_ignores_registration_fields`；DSL 无命令模板）
16. **幂等与纯度**：capabilities 的 idempotent:true / is_pure:false 合法且永不读写 cache；未满足纯度的函数同样禁用缓存；schema 校验失败的输出不进入成功结果或缓存。
    - 状态（第七轮）：已实现（capabilities 永不缓存、非 pure / 非幂等不缓存、result schema 失败不缓存单测）
17. **过期与授权**：删掉任一必需产物或使其过期后，后续新 Run 对失效步骤重算；命中到交付期间内容有保留保障。授权调用者可读 manifest 及所有交付条目；另一主体仅知道 obj_id / locator 无法读取；凭据过期可重新授权，不触发转换重算。
    - 状态（第八轮 DV）：DV WF-DV-013：bob 拿 devtest 的 obj_id 调 create_run 被拒（输入授权改用 NDM 读类调用 `query_object_by_id`，`is_object_stored` 是任何上传者可用的存在性探测）；产物交付 token 的数据面读取未在 DV 中验证
    - 状态（第七轮）：部分：过期 / 产物缺失重算已实现；交付经 `delivery.grants` 签发绑定 principal 的精确 key 读 token（`ZoneNativeIo::deliver`）；仅单测了内存 IO 的 grant 形状，真实 bucket token 未在 Zone 上验证；跨主体读取拒绝单测覆盖输入侧
18. **执行中重启**：杀死 workflow 后重启，旧实例本地 Native 媒体 Task 以 interrupted 错误结束，不永久 Running；其他 workflow / schedule 任务不被误终止，临时目录仅在旧受管进程停止后清理。runtime 对原 run_id 的 not_found 在总预算内恢复，新 Run 复用已落盘的有效步骤；不承诺原 Run 续跑或完整历史恢复。
    - 状态（第七轮）：已实现（`stale_executions_of_a_previous_epoch_are_failed_as_interrupted_and_cleaned`）；runtime 预算内重建为 agent 侧，未实现
19. **媒体语义**：尾页答案、多页扫描 / 空白页、无音轨视频、部分页失败均有准确 manifest；无文本 / 无音轨不使整体无理由失败。首次临时页失败返回 incomplete，后续请求必须能重新执行失败步骤补齐，不能命中旧 partial 而长期缺页；其他已有有效步骤缓存仍可复用。Agent 的 coverage 明确实际读取、未转换和失败范围。
    - 状态（第七轮）：已实现（`build_catalog_covers_pages_frames_and_absent_audio`）；真实 fixtures（尾页答案 PDF、扫描页、无音轨视频）未运行
20. **资源与恢复界限**：超大页数 / 像素 / 输出 / 排队量触发明确上限；取消释放资源；持续失败或 run_id 丢失不会无限 ensure，runtime 到总 deadline / 恢复次数上限即终止并交付诊断。
    - 状态（第七轮）：部分：`resource_limit`、有界队列、取消释放已实现；runtime 总 deadline / 恢复次数为 agent 侧，未实现
21. **工具基线交付**：FunctionRegistration requirements、媒体需求清单、paios §6.4、宿主 / 镜像预置和检测规则一致。workflow 宿主独立完成真实 Run；干净 paios 镜像分别完成 PDF 文本 / 页图、视频抽帧 / 音轨、DOCX / PPTX 转 PDF。检查完整工具版本、字体 / locale 与实际执行指纹；仅 command -v 或标准 Agent 命令测试通过不算验收完成。镜像齐全、宿主缺工具的故障注入必须准确定位宿主缺项。
    - 状态（第七轮）：部分：清单、文档、镜像包清单已改；镜像未重建、宿主安装脚本未改、干净镜像样例未跑
22. **缓存可用性**：相同字节换文件名、owner 或 mtime 后，纯转换步骤命中缓存；相同函数和规范化参数跨 Run、跨重试得到同一 thunk ID；media.pdf 与 media.document 共用的步骤在内容和参数相同时互相命中；重复转换的产物 obj_id 相同，build_catalog 能命中；被执行预算截断的结果不写缓存；调用者无法直接指定 content_id。
    - 状态（第七轮）：已实现（`image_pipeline_fits_an_image_and_reuses_the_cache_across_file_names`、content_id 不可指定、thunk ID 跨 Run 相同单测）；media.pdf / media.document 共用步骤互相命中未单独测试

## 6. 本次记录

以下是历史记录，其中“待拍板”和测试结果仅描述当轮状态；当前方案以 §2–§5 为准，§7 的三个问题均已获得用户答复。

- 2026-11-05：review 设计文档 v0.4，先误读 §4.8 为"真的接 preview 组件"，用户纠正为复用处理模型；再定为用 workflow service 的系统内置 pipeline 承载；本文成稿，待 §2.1 D1–D9 拍板后 T1 开工。
- 2026-11-05（第二轮）：用户定位本文为 Agent Runtime 改造的前置，并要求 workflow 侧不依赖任何 OpenDAN 组件、可独立开发测试。对照 workflow service.md / executor list.md 与 `src/kernel/workflow` 实现复核：修正 §1 三处与代码不符（缓存实为进程内存、buckyos-api 已有 schedule 客户端、TS 源码在 buckyos-websdk）；新增 §1.1 F1–F15、§2.2 依赖边界、D10–D15；§4 拆为 Phase A（workflow 独立）/ Phase B（agent_tool、WebUI）；验收加 8–13。`cargo test -p workflow` 50 通过。D1–D15 仍待拍板，其中 D10 / D11 / D13 / D14 是 workflow 自身补齐项，与媒体管线无关也应做。
- 2026-11-05（第三轮）：用户问 executor / thunk 设施是否已有：executor（`service::` 直执行）有；thunk 没有——`InMemoryThunkDispatcher` 只记账，scheduler `run_thunk` 不送达节点，node_exector.md §6 整体后置。用户裁决 P5：**这个版本要把 FunctionObject 用起来**，且 thunk 要意图化（"图片放缩"是函数，包命令行不是）。据此改 D1 / D4 / D10，新增 D16–D18 与 §2.3，T2 从 adapter 改为"FunctionObject + Native 实现 + LocalThunkRunner"，新增 T1g，验收加 14–16。cmd 层（ExtTools `provides` / `tools_check.resolved`）定位为 FunctionObject 的 `requirements` 数据源，不是 executor。
- 2026-11-05（第四轮）：用户要求按 review 更新本文，仍需用户选择的项集中在文末。修正 NamedObject 已可读取、Task creator 与 Run owner 不等同等现状；补 F16–F20。将 Completed ensure 改为活动 Run 合并，定义 revision 与 plan_version 分离，缓存索引与 Named Object 分层，幂等与纯度分离；补输入 / 输出授权、真实 thunk 身份、结构化回执、Task 终态、取消 / 重启清理、部分媒体结果与资源上限。任务和验收同步重排。本轮仅改本文，未实现功能、未重跑 Rust / DV 测试。
- 2026-11-05（第五轮）：读取用户在 §7 写回的答案并保留原文，新增 P6–P8：R1 采纳同 principal 合并，R2 删除 workflow 故障时的业务备用路径，R3 授权依赖并要求 workflow 驱动 runtime baseline。同步 D6 / D11、正常选路与验收，新增 §2.5、T2c / T4b 和工具基线验收；三项 Review 已收敛。本轮只更新本文，未改代码、安装依赖或构建镜像。
- 2026-11-05（第六轮）：用户裁决 P9：以缓存数据可用为目标设计 ThunkObject 和 FunctionObject 的字段，描述性内容放 run meta。据此拆出 FunctionRegistration，ThunkObject 去掉 metadata，回执改用 execution_id；新增 F21–F24（回执 key、scheduler 第三种 ID 算法、FileObject ID 含文件名和 mtime、scheduler 依赖被移走的字段）；同步 D2 / D4 / D17 / D18、§2.3、§2.4、§3.1、T0 / T1g / T2a / T2c / T3 和验收，新增验收 22。用户随后确认了由该原则推出的三点：timeout / requirements 等移出 hash、params 改用 FileObject.content、params_schema 留在 hash。本轮只更新本文，未改代码，未运行测试。

- 2026-11-05（第七轮）：按本文实施 Phase A。落地：buckyos-api `thunk_object.rs` 拆分 FunctionObject / FunctionRegistration / ThunkObject / ThunkExecution / 结构化 ThunkExecutionResult，`media_manifest.rs`，`json_schema.rs` 从 task_manager 移入；workflow 新增 function_registry / native / local_thunk_runner / cache_index / persistence（SQLite）/ driver / system_defs / io / functions/{test_functions, media/*}，defs/media.*.json 四条系统定义；orchestrator 改为 execution_id 回执、真实 thunk ID、`idempotent && is_pure` 缓存门禁与命中校验、Run output / node_errors / node_sources、cancel_run、deadline；server 每方法验证 caller、ensure、`list_functions` / `get_function` / `get_capabilities` / `cancel_run` / `get_run_output`；scheduler thunk_runner 改读 FunctionRegistration 并用 `ThunkObject::calc_obj_id`（F22 / F24）；workflow 桶声明与 scheduler `add_workflow`；Rust / TS 客户端同步；check.py 加 workflow / 4070；DV 用例 WF-DV-009~013 与 fixtures；T0 文档回写与 media tool baseline；build_aios 加 LibreOffice / 字体 / locale。验证：`cargo test -p workflow -- --test-threads=1` 通过（workflow 单测 + server 集成测试）；依赖方向 `cargo tree -p workflow` 未含 agent_tool / llm_context / libopendan / opendan / nfs_server / aicc（第七轮核对，见 §5 第 12 项）。未做 / 未验证：DV 在 Zone 上运行、带真实 poppler / ffmpeg / soffice 的 `--ignored` 测试、paios 镜像重建与干净镜像探测、宿主安装脚本、产物精确 token 交付、T4 agent 接入（进行中）、T5 WebUI（进行中）、T6b。

- 2026-11-05（用户补充）：Bucket 设计 D22 把 `/ndn/` 定为跨 Zone 按 ObjId 读取的统一协议，标准 HTTP + Range，让 ffmpeg 能直接读文件对象。本文新增 §2.4 的目标形态和 T2d，只改了文档，workflow 代码未动，仍在用 `materialize_input`。

- 2026-11-05（第八轮）：宿主安装 poppler / ffmpeg / LibreOffice，`start.py --all` 重建 dev Zone 后跑 DV：WF-DV-009~013 全部通过（013 用种子用户 bob 作第二主体）。DV 暴露并修复三处：workflow 写入 Named Store 的对象未 pin 被 nfs-server 维护 GC 回收（现每次写 / 读续 30 天 lease，产物递归 pin）；请求者看不到 TaskMgr 任务树（run 根任务给请求者 WholeTree 读 grant）；输入授权误用存在性探测（改为 `query_object_by_id`）。旧 Zone 的 `services/workflow/spec` 缺 buckets 声明需重建配置，升级路径未做。WF-DV-001/002/007（AICC fixture `input_json` 与当前协议不符）与 WF-DV-006（TaskMgr 1.x TaskData 写法）为既有阻塞。

## 7. 用户设计 Review 答复（已收敛）

三项均已获得答复，下面保留用户原文及采用的语义；此前未选方案仅作历史讨论，不再作为实施阻塞项。

### R1：跨 principal 的并发去重——采用 A

> 按推荐

同一 principal 的活动请求按授权 scope 合并；不同 principal（包括同 owner 的不同 Agent）保留各自 Run，跨 Run 共享有效 Step cache。不同主体同时首次请求允许重复计算。本期不做共享 Run 的订阅者 / 取消规则，也不加入跨主体 Step single-flight。已同步 D6 / D13、T1b / T1h 与验收 2。

### R2：workflow 不可达时的备用路径——取消该需求

> workflow是scheduler 同级别的内核服务，不工作系统相当于挂了... 没必要做这个假设

workflow 可用是系统正常工作的前提。删除“它不可达时仍能完成媒体理解”的产品要求、本地备用转换路径及相应验收；RPC 故障直接按内核服务错误上报。正常的直传 / AICC ASR、按模型能力选择表示方式继续存在；它们不是为 workflow 故障建设的容灾能力。转换中的格式错误、缺工具、超时仍需结构化诊断，重启清理与预算内恢复也仍保留。已同步 §2 / §3.1、T4 与验收 3 / 5。

### R3：新增依赖与 runtime baseline——已授权，需求由 workflow 驱动

> 可以加依赖。应以workflow的需求为先导，定义 agent-runtime的baseline需求(paios 镜像里的自带工具)

本任务必要的依赖新增已获授权，包括 image、SQLite 索引所需 rusqlite；schema 校验按实际需要选用，不再次就同一授权请求确认。优先复用仓库已有版本与组件，不借此扩展到 XLSX 或通用执行平台。

新增要求落在 §2.5、T2c / T4b 和验收 21：先从 FunctionRegistration requirements 得出媒体工具基线，再同步宿主安装与 paios / agent-runtime 的工具分发、探测、手册和测试。当前 P0 的 LocalThunkRunner 在 workflow 宿主机执行，因此宿主与镜像必须分别满足能力要求；仅在 paios 镜像中装工具不能使宿主 runner 自动获得它们。此答复不隐含将执行位置改回 Agent 容器。
