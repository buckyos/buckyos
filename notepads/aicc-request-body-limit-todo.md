# AICC 请求体 1 MiB 上限修正 TODO

日期：2026-11-02

状态：**已裁决（Q1 32 MiB、Q2 A、Q3 维持整份 INFO）并已实施，未提交、DV 未做**（§8）。来源：[Work Session 上下文压缩与工具预算修正](./opendan-work-context-budget-fix-todo.md) 实施后的 DV：用户重建 DV 后跑一个长研究任务，3 个 `explore` 子任务失败，其中 2 个是 `413 Payload Too Large`。用户裁决：**修改 AICC 的 1 MiB 默认上限**（不靠缩小上下文窗口绕过）。

范围：AICC 服务（接收上限、写入 TaskMgr 的任务 input、`aicc.llm.input` 日志）、Desktop Task Center 的请求展示、AICC 文档。buckyos-base、cyfs-gateway、TaskMgr、llm_context / agent_tool **不改**（理由见 §2；agent_tool 侧的兜底见 §7，另起一单）。

路径均相对 `buckyos/`，另注明的除外。

## 1. DV 现象（2026-11-02）

Agent `jarvis.test.buckyos.io`（容器形态），plan `work-20261103-3`（`plan_simple`）派发的 `explore` Todo：

| Session | 结局 | 证据 |
| --- | --- | --- |
| `work-20261103-4` | 18:25:26 失败 | `last_error = {"kind":"provider","message":"… aicc helper.llm_chat failed: rpc call error: 413 Payload Too Large: Payload Too Large (http_error)","recoverable":false}`；40 轮，最后一轮 provider 计输入 184,928 token；快照 1,151,862 字节 |
| `work-20261103-5` | 18:24:10 失败 | 同上；29 轮，最后 201,332 token；快照 1,073,820 字节 |
| `work-20261103-7` | 成功 | 结束时上下文 JSON 1,013,852 字节，离上限只差约 34 KB |

- 两次失败都发生在 `write_file` 写完一份 36–44 KB 的报告之后：文件内容在调用参数里，把下一次请求推过了 1 MiB。
- 所有 run 的窗口都是 1,050,000（`llm.chat` / `llm.plan` / `llm.code` → codex-proxy 的 `gpt-6.1-sol` / `gpt-6-astra`，AICC `max_context_tokens`），80% 触发线 84 万 token，`compactions = 0`。
- 上下文组成（work-4 快照 `state.accumulated`，1.09 MB）：工具结果 707 KB（40 条）、`provider_state` 194 KB、`thinking` 118 KB（Responses 编码时被丢弃，但随请求发出）、`write_file` 参数 43 KB。按本地估算器折算约 4.6–5.0 字节 / 估算 token，**约 21 万估算 token 就会超过 1 MiB**。
- AICC 日志：18:24:10.393 有 `172.17.0.3:51366=>POST /kapi/aicc`，之后没有对应的 `aicc.llm.input`——请求在读 body 时就被拒绝。cyfs-gateway 日志里没有这条 `/kapi/aicc`：容器直连 AICC 端口（`172.17.0.1:4040`）。
- xllm 把 kRPC 的 413（`RPCErrors::Detailed{code:"http_error", data.status:413}`）归为 `ProviderFailure::Unknown`，不可恢复，session 直接失败，libopendan 的上下文重写没有机会介入。

同一次 DV 顺带看到的规模问题（不改上限也已存在，改了上限会放大）：

| 位置 | 现状（约 40 分钟 DV） |
| --- | --- |
| TaskMgr `task-mgr-main.db`（`/opt/buckyos/local/task-manager/`，95 MB） | 268 个 `aicc.compute/v1` 任务的 `input_json` 共 **79 MB**，最大 999 KB：每次推理都把整个上下文存成任务 input |
| AICC 日志 `aicc.1330079.log`（92 MB） | 270 条 `aicc.llm.input` 共 **85.9 MB**：每次推理整份请求打一行 INFO；`aicc.llm.output` 268 条只有 4.3 MB |

## 2. 现状（源码核对）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| 1 MiB 从哪来 | buckyos-base `buckyos-http-server` 的 `DEFAULT_MAX_RPC_BODY_SIZE = 1024 * 1024`，`serve_http_by_rpc_handler` 用它调 `serve_http_by_rpc_handler_with_limit`；`read_body_with_limit` 先看 Content-Length，再边读边计，超限回 `413 Payload Too Large` | `buckyos-base/src/buckyos-http-server/src/lib.rs:233-262,550-594` |
| 已有可调接口 | `serve_http_by_rpc_handler_with_limit(req, info, handler, max_body_size)` 已公开，服务可以自己定上限，**不需要改 buckyos-base** | 同上 `:244` |
| AICC 用的是缺省 | `AiccHttpServer::serve_request`：除 `ARTIFACT_OPEN_PATH` 外的 POST 都走 `serve_http_by_rpc_handler(request, info, self)`；AICC 只有普通 kRPC，没有 s2s 路径 | `src/frame/aicc/src/service/mod.rs:336-351` |
| 网关不设上限 | cyfs-gateway 转发时把 body 流式交给上游；`max_body_buffer_bytes`（缺省 64 KB）只决定能否为 HTTP 状态重试缓冲重放，不拒绝请求 | `cyfs-gateway/src/components/cyfs-gateway-lib/src/forward/plan.rs:150-160`、`server/http_server.rs:1318-1340` |
| 客户端不设上限 | buckyos-api `AiccClient`（kRPC）直接发 JSON；`DEFAULT_AICC_KRPC_TIMEOUT_SECS` 已是 900 s，几 MB 的上传不成问题 | `src/kernel/buckyos-api/src/aicc_client.rs`、`runtime.rs:54` |
| **AICC 把整份请求写进 TaskMgr** | `ensure_task` 调 `create_delegated_task`，`input = {"request": {"version":1, tenant_id, trace_id, "request": spec.input}}`，`spec.input` 就是 `canonical_body`（整份请求）。TaskMgr 自己也走 `serve_http_by_rpc_handler`（1 MiB），**只放开 AICC 的话，超过 1 MiB 的请求会在 `create_delegated_task` 被 TaskMgr 以 413 拒绝** | `src/frame/aicc/src/execution/mod.rs:1404-1426`、`service/ports.rs:391-442`、`src/kernel/task_manager/src/server.rs:2115` |
| TaskMgr 里这份 input 的读者 | AICC 自己的恢复只读本地执行库（`ExecutionRecord` 只存 `body_fingerprint`），不读 TaskMgr input。唯一的读者是 Desktop Task Center 的任务详情：`aiccRequest.params = detail.input.request.request`，提供“复制请求到 Playground”（文案：粘贴到 AI Center → Playground → 高级模式可重新调用） | `src/frame/desktop/src/api/task_mgr.ts:122,501-504`、`app/task-center/pages/TaskDetailPage.tsx:449-462`、`i18n/ai-center-playground.ts:73-74` |
| 结果侧已有门禁 | `runner_complete` 的结果按 TaskMgr 上限检查：`MAX_TASK_RESULT_PAYLOAD_BYTES = DEFAULT_MAX_RPC_BODY_SIZE − 64 KiB`，超限明确失败（2026-10-11 codex-proxy 图片 base64 撞 413 后加的）。它对应的是 **TaskMgr** 的上限，改 AICC 接收上限时不能跟着变 | `src/frame/aicc/src/execution/mod.rs:28-30,170-194,2233`；`doc/aicc/AICC.md:164`、`doc/aicc/aicc_api设计.md:555` |
| 全量请求日志 | `log_llm_input`：LLM 调用在 INFO 级把 `canonical_request.to_params()` 整份打进 `aicc.llm.input`；测试只断言事件名 | `src/frame/aicc/src/execution/mod.rs:925-939,3571,3628` |
| 与 AICC 原则冲突 | AICC.md 硬性原则：“任何日志 / metrics / tracing 不记录原始 base64 或资源原文”；整份请求里有 `provider_state` 的加密内容、工具结果原文 | `doc/aicc/AICC.md:156-164` |

结论：接收上限是 AICC 自己用了缺省 1 MiB，在 AICC 里换成 `serve_http_by_rpc_handler_with_limit` 即可；但 AICC 随后把整份请求转发给 TaskMgr（同样 1 MiB，且每次推理都存一份），所以**必须同时改写入 TaskMgr 的任务 input**，否则 413 只是从 AICC 挪到了 TaskMgr。日志同理，上限放大后每行可达数 MB。

## 3. 方案

### 3.1 AICC 接收上限

- `service/mod.rs` 新常量 `AICC_MAX_RPC_BODY_SIZE`（Q1 建议 32 MiB），`serve_request` 改调 `serve_http_by_rpc_handler_with_limit(request, info, self, AICC_MAX_RPC_BODY_SIZE)`。
- 依据：1.05M 窗口按约 5 字节 / token 满载约 5.3 MB，80% 触发线约 4.2 MB；`input.media = inline` 的图片 / 文档以 base64 进消息，单张就可能几 MB。32 MiB 与主流 provider 自己的单请求上限同一量级（Anthropic Messages API 文档给出 32 MB），AICC 不再是第一个拒绝的环节。
- `MAX_TASK_RESULT_PAYLOAD_BYTES` 保持从 `buckyos_http_server::DEFAULT_MAX_RPC_BODY_SIZE` 推导（它是 TaskMgr 的上限），在常量旁注明两者的区别。
- 413 仍可能出现（超过 32 MiB），错误照旧；xllm 侧的兜底见 §7。

### 3.2 写入 TaskMgr 的任务 input（Q2）

`ensure_task` 不再无条件写整份请求：

- 序列化后 ≤ `MAX_TASK_REQUEST_INLINE_BYTES`（建议 256 KiB）→ 照旧写 `request`（Playground 复现的典型场景不受影响）；
- 超过 → 不写 `request`，改写 `request_summary`：`{ bytes, fingerprint, messages, tools, params }`。`bytes` 为序列化字节数；`fingerprint` 复用 `execution/mod.rs:1404` 已算的 `body_fingerprint`（经 `TaskSpec` 传入，不重算）；`messages` 是条数与按 role 的计数；`tools` 是工具名列表；`params` 是除 `messages` / `tools` 外的标量参数（模型、`max_output_tokens`、`response_format` 类型等）。`trace_id` 已在任务 input 里，宿主可据此找到调用方的完整上下文（OpenDAN 的 `trace_id` = run_id，run 快照里有整份上下文）。
- 共享类型：`buckyos-api` `taskdata.rs` `AiccComputeTaskRequest` 加 `request_summary: Option<Value>`（缺省不序列化）；不升 schema 版本（开发阶段，按 AGENTS.md 不做兼容）。
- Desktop：`task_mgr.ts` 解析 `request_summary`；`TaskDetailPage` 在没有 `request` 时显示摘要和一句“请求过大（N KB），未保存原文；完整上下文在调用方的 run（trace_id …）”，不显示“复制到 Playground”按钮；i18n 补中英文案。

### 3.3 `aicc.llm.input` 日志（Q3）

- INFO 级只记元数据：现有 metadata + `request_bytes`、消息条数（按 role）、工具名、标量参数，以及最后一条 user 消息的文本预览（≤ 2 KiB，不含 base64 / provider_state）。
- 整份请求降到 DEBUG（排障时临时开 DEBUG）。
- `aicc.llm.output` / `aicc.llm.error` 不变。

## 4. 待裁决

| Q | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| Q1 | AICC 接收上限取多少、是否可配 | A 32 MiB 常量 / B 16 MiB 常量 / C 放进 AICC settings 可配 | **A**。常量足够，settings 多一个要校验和下发的键；以后真有需要再改。注意 body 在鉴权前读入内存（kRPC token 在 body 里），并发 × 32 MiB 是内存上界；AICC 只在 Zone 内可达（宿主 4040，外网由防火墙挡），本单不加限流 |
> 32 Mib
| Q2 | TaskMgr 任务 input | A 小请求原样、大请求只存摘要 / B 同时放开 TaskMgr 上限、继续存整份 / C 整份存成 named object，任务 input 只放 obj_id | **A**。B 让 TaskMgr 库按“每轮整份上下文”增长（这次 40 分钟 79 MB，窗口放大后是这个数量级的数倍）；C 能保住大请求的 Playground 复现，但要对象生命周期和 GC，留作后续 |
> 选A吧
| Q3 | `aicc.llm.input` | A INFO 只记元数据与预览、整份降到 DEBUG / B 维持整份 INFO | **A**。符合 AICC.md 的日志原则；B 在上限放开后每次推理可能写几 MB 日志，滚动后排障信息很快被冲掉 |
> 正式产品肯定用warn级别，开发用info没问题。毕竟每个token都花了钱的，日志里存一份是可以的。

## 5. 实施清单

### A. AICC（`src/frame/aicc/`）

- [x] A1 `service/mod.rs`：`AICC_MAX_RPC_BODY_SIZE` + `serve_http_by_rpc_handler_with_limit`；`execution/mod.rs:28-30` 常量注释写明结果上限对应 TaskMgr。
- [x] A2 `execution/mod.rs` / `service/ports.rs`：`TaskSpec` 带上 `body_fingerprint`；`ensure_task` 按 §3.2 写 `request` 或 `request_summary`（新函数 `task_request_view`，纯函数便于单测）。
- [x] A3 `execution/mod.rs` `log_llm_input`：按 §3.3 分级。
- [x] A4 测试（`service/tests.rs`、`execution/mod.rs` 测试模块）：
  - 2 MiB 的合法 kRPC 请求经 `AiccHttpServer::serve_request` 不再 413、到达 handler；超过 `AICC_MAX_RPC_BODY_SIZE` 仍 413；
  - `task_request_view`：小请求原样；大请求只有摘要（字节数、指纹与 `canonical_body_fingerprint` 一致、消息计数、工具名、标量参数），序列化结果远小于 TaskMgr 上限；
  - 一次 > 1 MiB 的 LLM 请求走完整执行路径（in-process TaskMgr 替身记录 `create_delegated_task` 的 input 大小），任务创建成功；
  - `aicc.llm.input`：INFO 事件不含整份 `messages`、含 `request_bytes`；现有 `3571` / `3628` 的事件名断言照过。
- [x] A5 `cargo test -p aicc`（并发跑多个 cargo 时加 `-j 8`）。

### B. 共享类型与 Desktop

- [x] B1 `src/kernel/buckyos-api/src/taskdata.rs` `AiccComputeTaskRequest.request_summary`；`cargo test -p buckyos-api`。websdk 没有这个类型的 TS 定义（只有 schema id 常量），不用改；按 AGENTS.md 联动规则核对后在记录里写明。
- [x] B2 `src/frame/desktop/src/api/task_mgr.ts`、`app/task-center/pages/TaskDetailPage.tsx`、`i18n/ai-center-playground.ts`：摘要展示与文案；`pnpm check`、`pnpm lint`。

### C. 文档

- [x] C1 `doc/aicc/AICC.md`（§3.2 硬性原则附近）与 `doc/aicc/aicc_api设计.md`（§1.5 任务生命周期 / 结果门禁 `:555` 旁）：写明请求体上限 32 MiB（AICC 自己的上限，结果仍受 TaskMgr 1 MiB 约束）、TaskMgr 任务 input 的原样 / 摘要规则、`aicc.llm.input` 的分级。
- [x] C2 cmd_skills 联动检查：`aigc_*`、`buckyos_admin`、`xllm` 是否写了 AICC 请求大小或任务 input 的形状；没有写就在记录里注明“无需修改”。

## 6. 验证与 DV

- 单测：§5 A4、B1、B2。
- DV（AICC 是宿主原生进程：`cd src && uv run buckyos-build.py --skip-web -s aicc`，替换 `/opt/buckyos/bin/aicc/aicc` 后由 node-daemon 拉起；Desktop 改动需重建 control-panel web）：
  1. 复跑同类长研究任务（plan + 2～3 个 `explore`）：不再出现 `413`；上下文超过 1 MiB 的推理成功（AICC 日志 `aicc.llm.input` 的 `request_bytes` > 1,048,576 且有对应 `aicc.llm.output`）。
  2. TaskMgr：`sqlite3 -readonly /opt/buckyos/local/task-manager/task-mgr-main.db "select count(*), sum(length(input_json)), max(length(input_json)) from task where schema_id='aicc.compute/v1'"`，大请求的 input 只有摘要（单条 ≤ 几 KB），总量与同等轮数的这次 DV（79 MB）对比。
  3. AICC 日志：同样轮数下 `aicc.llm.input` 的总量（这次 85.9 MB）。
  4. 上游核对：codex-proxy（CLIProxyAPI，`/opt/cli-proxy-api/`）对几 MB 请求的接收，用一次 3–5 MB 的 `chat.completions.create` 直接测；若上游也拒，记下其上限与错误形态。
  5. Desktop Task Center：小请求仍可“复制到 Playground”；大请求显示摘要与 trace_id。
- 证据放 `/home/lzc/project/work_context_budget_dv_runs/aicc_body_limit/`。

## 7. 关联与后续

- [Work Session 上下文压缩与工具预算修正](./opendan-work-context-budget-fix-todo.md) §8.3：窗口 1.05M 的问题。本单之后 1 MiB 不再是硬墙，84 万 token 的触发线在技术上可达，但每轮重发几十万 token 的成本与延迟仍在，是否给窗口加上限仍待那一单裁决。
- 另起 agent_tool 小单（按 llm_context / agent_tool 修改单列、先行的规则）：
  1. xllm 把 kRPC `http_error` 且 `status = 413` 归为 `ProviderFailure::ContextLimit`，让 llm_context 让出 `ContextLimitReached`、由宿主重写，而不是让会话失败（超过 32 MiB 或上游拒收时的兜底）；
  2. 同一次 DV 的另一个失败 `work-20261103-6`：codex-proxy 返回 503（上游 `server_is_overloaded`），AICC 已标 `retriable: true, retry_after_ms: 60000`，但 xllm `aicc_provider_failure` 只把 `Timeout` 与 HTTP 408 / 504 当作可重试，忽略 `retriable`（现有测试还断言 502 不可恢复），会话直接失败。是否改为读取 `retriable` 需裁决。
- 后续（不在本单）：Q2-C 大请求存为 named object 以便 Playground 复现；`thinking` 块在 Responses 编码时被丢弃却随 kRPC 发送（这次约 11% 的体积），能否在客户端按目标协议裁掉需先确认 Claude 等协议对 thinking 签名回放的要求。

## 8. 实施记录

2026-11-02 实施，三仓同日：`buckyos`（aicc、buckyos-api、desktop、doc、cmd_skills）。未提交，DV（§6）未做。

**裁决落地**

- Q1 → `service/mod.rs` `AICC_MAX_RPC_BODY_SIZE = 32 MiB`，`serve_request` 改走 `serve_http_by_rpc_handler_with_limit`；常量注释写明与 `ResourceLimits`（inline 4 MiB/个、16 个）的关系、body 在鉴权前读入内存的前提、结果仍受 TaskMgr 1 MiB 约束。
- Q2 → `execution/mod.rs`：`MAX_TASK_REQUEST_INLINE_BYTES = 256 KiB`、`TaskSpec.body_fingerprint`（`execute` 把已算好的指纹传入，不重算）、纯函数 `task_request_view(input, fingerprint) -> TaskRequestView::{Inline, Summary}`。摘要形状 `{bytes, fingerprint, messages: {count, by_role}, tools: [name], params, provider_state_bytes}`；`params` 保留标量与 ≤256 字节字符串，长字符串 / 数组 / 对象只留 `<N bytes>` / `<array of N>` / `<object with N keys>`，`messages` / `tools` / `provider_state` 不进 `params`。`service/ports.rs` 新 `task_input_json(tenant_id, trace_id, view)` 生成任务 input（`input.request.request` 或 `input.request.request_summary`）。
- Q3 → 按用户裁决**不降级**：`aicc.llm.input` 仍在 INFO 打整份请求（正式环境跑 WARN、开发跑 INFO，每个 token 都付了费，日志留一份是对的），只加 `request_bytes`。review 时提的“DEBUG 也脱敏”随之作废；AICC.md 已把这条写成有意为之。

**共享类型与 Desktop**

- `buckyos-api` `taskdata.rs` `AiccComputeTaskRequest.request_summary: Option<Value>`（缺省不序列化，不升 schema 版本）。websdk 只有 `AICC_COMPUTE_TASK_SCHEMA_ID` 常量，无需改。
- Desktop `api/task_mgr.ts`：`aiccRequestFields()` 从 `input.request` 取 `request` 或 `request_summary`，后者映射为 `Task.aiccRequestSummary`（`AiccRequestSummary` 类型）；`TaskDetailPage` 在没有 `aiccRequest` 时显示“AICC 请求摘要”区块与文案 `taskRequestTooLarge`（大小 + trace_id），不出“复制到 Playground”按钮；`i18n/ai-center-playground.ts` 补 `taskRequestSummary` / `taskRequestTooLarge` 中英；`ai-center/UI_DATAMODEL.md` 补一句。`pnpm check` 过；`pnpm lint` 的 5 个 error 都在 RequestFields / PublishComposer / AgentDevices / MainContent（已有，与本单无关），本单文件无告警。

**测试（A4）**

- `service/tests.rs` `body_limit`：2 MiB kRPC 请求经 `AiccHttpServer::serve_request` 不再 413、得到 kRPC JSON 响应；Content-Length 超 `AICC_MAX_RPC_BODY_SIZE` 仍 413 且不进 handler；`task_input_json` 对 >1 MiB 请求产出 <4 KiB 的 input、能按 `AiccComputeTaskData` 反序列化、小请求原样。
- `execution/mod.rs`：`task_request_view` 小请求原样；大请求摘要字段逐项断言、不含原文、<4 KiB、同请求两次结果相同（确定性，对应 TaskMgr `replay_created` 的 `input_digest` 比对）；`execute_hands_the_body_fingerprint_to_the_task_spec` 跑完整执行路径，MemoryTasks 收到的 `TaskSpec.body_fingerprint` 与 `canonical_body_fingerprint` 一致、>1 MiB 的 input 判为 Summary；`aicc.llm.input` 断言新增 `request_bytes > 0`。
- 与任务单的差别：A4 第 3 条原写“in-process TaskMgr 替身记录 create_delegated_task 的 input 大小”，现有测试替身在 `TaskManagerPort` 层（拿到的是 `TaskSpec`），所以改为“执行路径断言 `TaskSpec` + `task_input_json` 单测断言 JSON 大小”，没有另造 TaskMgr kRPC 替身。
- `cargo test -p aicc -p buckyos-api -j 8`：792 + 343 全过。

**文档与手册（C）**

- `doc/aicc/AICC.md` §硬性原则加 3 条（32 MiB、任务 input 原样 / 摘要规则、`aicc.llm.input` INFO 有意为之）；`doc/aicc/aicc_api设计.md` 结果门禁段后加一段说明 1 MiB 是 TaskMgr 的、请求侧规则与确定性。
- cmd_skills：只有 `xllm.md` 写了“AICC's 1 MiB RPC body”，改为 32 MiB；`aigc_*`、`buckyos_admin`、`opendan_agent_dev` 未提请求大小或任务 input 形状，无需修改。

**待办**

- §6 DV 全部未做（AICC 原生进程需重建替换、Desktop 需重建 control-panel web；核对 codex-proxy 对 3–5 MB 请求的接收）。
- §7 agent_tool 小单（413 → `ContextLimit`、读 AICC `retriable`）未动；工作区里另有未跟踪的 `notepads/xllm-aicc-error-mapping-todo.md`，可能已是那一单。
