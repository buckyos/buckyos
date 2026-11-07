# xllm：AICC 错误归类修正（先行 TODO）

日期：2026-11-02

状态：**已裁决（Q1–Q3 均按建议，2026-11-02）、已实施（未提交，单测通过，DV 随宿主单做）**，实施记录见 §7。来源：[Work Session 上下文压缩与工具预算修正](./opendan-work-context-budget-fix-todo.md) 实施后的 DV（2026-11-02，`jarvis.test.buckyos.io` 容器形态）。AICC 请求体 1 MiB 上限另有 [aicc-request-body-limit-todo.md](./aicc-request-body-limit-todo.md) 在处理；本单只处理调用方（agent_tool / xllm）把 AICC 错误归成什么类。按用户规则（llm_context / agent_tool 修改单独成 TODO、先于宿主），宿主侧的重试节奏与上限在 [opendan-dv-followup-todo.md](./opendan-dv-followup-todo.md)，排在本单之后。

范围：`agent_tool/src/xllm.rs` 的 AICC 错误归类；Q3 选 A 时连带 buckyos-api `AiccErrorCode` 与 AICC 的错误映射。不改 llm_context（`ProviderFailure` 的变体够用）。

路径均相对 `buckyos/src/frame/`，另注明的除外。

## 1. DV 现象

| Session | 错误 | xllm 归类 | 结局 |
| --- | --- | --- | --- |
| `work-20261103-4` / `-5`（`explore`） | `rpc call error: 413 Payload Too Large`：上下文 JSON 超过 AICC kRPC 1 MiB | `ProviderFailure::Unknown` → `recoverable: false` | session 失败；上下文中途重写本可以把请求缩回去，但没有被触发 |
| `work-20261103-6`（`explore`） | AICC `provider_error`，`details.http_status = 503`、`retry_after_ms = 60000`、**`retriable: true`**；上游 `server_is_overloaded: … Please try again later.` | `Unknown` → 不可恢复 | session 第 10 轮失败，已做的 9 轮全部作废，plan 只能另开新 session 重做这个 Todo |

## 2. 现状（源码核对）

| 主题 | 事实 | 位置 |
| --- | --- | --- |
| kRPC 错误 → 失败类别 | `provider_error_from_rpc`：先按 `AiccError` 归类；否则 `Detailed{code:"http_error"}` 只有 `status` 408 / 504 算 `Transient`，其余 `Detailed` 一律 `Unknown`。kRPC 客户端对非 2xx 回 `RPCErrors::Detailed{code:"http_error", data:{"status":N}}`（结构化） | `agent_tool/src/xllm.rs:5525-5567`；buckyos-base `src/kRPC/src/lib.rs:529-568` |
| AICC 失败响应 → 类别 | `aicc_provider_failure`：额度 / 权限拒绝 → `Refused`；`QuotaUnavailable`、`Timeout`、`ProviderError` 且 `details.http_status` ∈ {408, 504} → `Transient`；**不看 `AiccError.retriable`**。测试 `aicc_http_timeout_is_recoverable_in_rpc_and_failed_responses` 断言 `(ProviderError, 502, retriable=true)` 不可恢复 | `xllm.rs:5490-5521`、`:11240-11283` |
| 直连 OpenAI 的路径不一致 | xllm 自己的 OpenAI 兼容客户端：408 / 409 / 425 / 429 / 500 / 502 / 503 / 504 → `Transient`，`error.code == "context_length_exceeded"` → `ContextLimit` | `xllm.rs:5776-5796` |
| AICC 怎么定 `retriable` | `ProtocolError → AiccError`：`retriable` = `Transport | Timeout | DeadlineExceeded`；`details` 带 `http_status`、`retry_after_ms`。AICC 返回之前已经做过同模型重试（`MAX_SAME_MODEL_ATTEMPTS = 2`）和候选模型 fallback | `aicc/src/error/mod.rs:600-640`、`aicc/src/execution/mod.rs:25-27` |
| “上下文放不下”在 AICC 线上没有码 | provider 的 `context_length_exceeded` 被归成 `ProtocolErrorKind::InvalidRequest` → `AiccErrorCode::InvalidRequest`（只在 `provider_code` 里留了原码）；路由层的 `context_length_insufficient` 也没有专门的码。`AiccErrorCode` 没有对应变体 | `aicc/src/protocol/openai_responses.rs:3957-3977`、`aicc/src/error/mod.rs:612`、`aicc/src/routing/mod.rs:914,943`、`kernel/buckyos-api/src/aicc_client.rs:1441-1472` |
| `ContextLimit` 的契约 | 只能来自结构化的 provider 码，不能从文本或裸 HTTP 400 推断；waist 把它变成 `ContextLimitReached{ProviderRefused}`，libopendan 据此中途重写、最多连续 3 次 | `llm_context/src/error.rs:27-32`、`lib_opendan/src/runner/live.rs` `run_compacting` |
| 宿主怎么用类别 | libopendan `is_retryable_error`：`Transient` → `recoverable: true`，run 保留为 `Paused`、Turn 不关；`Unknown` / `Permanent` → Turn 失败，无人应答的会话（`finish_failed`）直接失败 | `lib_opendan/src/runner/outcome.rs:273-279,749-782` |

## 3. 方案

### P1 413 → `ContextLimit`

`provider_error_from_rpc`：`Detailed{code:"http_error", data.status = 413}` → `ProviderFailure::ContextLimit`。依据：413 是结构化状态码，含义是“这次请求放不下”，与 `ContextLimit` 的补救方式一致（缩小上下文后再发）。llm_context 让出 `ContextLimitReached{ProviderRefused}`，libopendan 中途重写；连续 3 次仍放不下照旧暂停 run。AICC 上限放到 32 MiB 之后它是兜底（内联媒体、单条巨型消息、其它服务的上限），并且不依赖 AICC 那一单的进度。`ContextLimit` 的文档注释补一句“含传输层 413”。

### P2 AICC 标为可重试的错误 → `Transient`

`aicc_provider_failure`：不是额度 / 权限拒绝、且 `error.retriable == true` → `Transient`；`Timeout` 照旧。现有“`ProviderError` + 408 / 504”这条在 `retriable` 为真时被覆盖（Q2 定是否保留作补充）。`retriable` 是 AICC 的结构化字段，判断不读文本。改测试：`(ProviderError, 502/503, retriable=true)` → 可恢复；`retriable=false` → 照旧不可恢复。

`retry_after_ms` 不往上带：`ProviderFailure::Transient` 没有字段，llm_context 已冻结；宿主用自己的退避（宿主单 H1）。Q2 也问是否要带。

**部署顺序（宿主单 R7）**：P2 把 AICC 标 `retriable` 的 503 / 429 变成可重试后，宿主单 H1 落地前子会话会每秒重打过载的上游。两单代码可以分别合入，部署时 P2 不先于 H1。

### P3 “上下文放不下”的线上错误码（Q3）

buckyos-api `AiccErrorCode` 加 `ContextLengthExceeded`（wire 码 `context_length_exceeded`）；AICC 把各协议适配器识别出的 provider 上下文超限（OpenAI 的 `context_length_exceeded` 等结构化码）和路由层的 `context_length_insufficient` 映射到它；xllm 把它归为 `ContextLimit`。按“错误只按枚举判断”的原则，跨 RPC 的错误类别要在 wire 上有码，两端同时加。AICC 部分与 [aicc-request-body-limit-todo.md](./aicc-request-body-limit-todo.md) 同属 AICC，实施时协调，避免两单同时改 `aicc/src/error/mod.rs`。

## 4. 待裁决

| Q | 问题 | 选项 | 建议 |
| --- | --- | --- | --- |
| Q1（已定 A） | 413 是否算 `ContextLimit` | A 是（P1）/ B 新加 `ProviderFailure` 变体（改 llm_context） / C 维持 `Unknown` | **A**。补救方式相同（缩小上下文），`ContextLimit` 已经有完整的宿主路径；B 要改冻结的 llm_context，收益只是名字更准 |
| Q2（已定 A） | 可重试判定 | A 以 AICC `retriable` 为准，`Timeout` 保留 / B 照抄直连路径的状态码表（429 / 5xx） / C 两者都看 | **A**。AICC 已经按协议错误种类定了 `retriable` 并做过自己的重试与 fallback，调用方不应再按状态码另判一遍；`retry_after_ms` 不往上带，由宿主退避（真需要再给 `Transient` 加字段，属 llm_context 改动） |
| Q3（已定 A） | 上下文超限错误码 | A 本单加 `AiccErrorCode::ContextLengthExceeded`（P3） / B 只做 P1，P3 另起 / C xllm 读 `provider_code` 字符串 | **A**。窗口元数据（1.05M）与 provider 实际接受的上下文不一致时，provider 的拒绝今天直接让会话失败；C 违反“只按枚举判断” |

## 5. 实施清单

- [x] X1（P1）`agent_tool/src/xllm.rs` `provider_error_from_rpc`：`http_error` + 413 → `ContextLimit`；`llm_context/src/error.rs` `ContextLimit` 的文档注释补“传输层 413”（只改注释）。
- [x] X2（P2）`aicc_provider_failure` 读 `retriable`；改 `aicc_http_timeout_is_recoverable_in_rpc_and_failed_responses` 及同组测试。
- [x] X3（P3，Q3 = A）`kernel/buckyos-api/src/aicc_client.rs` 加 `ContextLengthExceeded`；`aicc/src/error/mod.rs` 与协议适配器把结构化的上下文超限码映射到它（`ProtocolErrorKind` 需要能区分时加一个 kind）；路由层 `context_length_insufficient` 同样；xllm 映射到 `ContextLimit`。websdk 若有 AICC 错误码的 TS 枚举，同步。
- [x] X4 测试：
  - 413：kRPC `Detailed{http_error, status:413}` → `ContextLimit`；经 llm_context 的 waist 出 `ContextLimitReached{ProviderRefused}`（`agent_tool` 里用脚本化客户端跑一次）；
  - `retriable`：`ProviderError` 503 `retriable=true` → `Transient`、run 记录 `recoverable: true`；`retriable=false` → 照旧；额度拒绝不受影响；
  - P3：各协议适配器的上下文超限样例 → `ContextLengthExceeded` → `ContextLimit`。
- [x] X5 `cargo test -p agent_tool`、`cargo test -p llm_context`；做 P3 再加 `cargo test -p aicc`、`cargo test -p buckyos-api`（并发跑多个 cargo 时加 `-j 8`）。
- [x] X6 文档：`doc/llm_context/xllm_rust_sdk.md` / `local_llm_context_protocol.md` 里的错误归类说明（如有）；cmd_skill `xllm.md` 若描述了可恢复错误，按 AGENTS.md 联动规则核对。

## 6. 验证

- 单测见 X4。
- DV 随宿主单一起做（重试退避在那边）：人为制造 413（AICC 那一单落地前天然存在：上下文过 1 MiB）→ worklog 出现 `context_rewritten` + `compaction`，同一个 run 继续；AICC 不可用或 provider 过载 → run `Paused`，宿主按退避重试后继续，而不是 session 失败。

## 7. 实施记录

2026-11-02 实施，未提交。Q1–Q3 均按建议（A）。

| 条目 | 落点 |
| --- | --- |
| X1 | `agent_tool/src/xllm.rs` `provider_error_from_rpc`：先取 `Detailed{code:"http_error"}` 的 `status`，413 → `ContextLimit`，408 / 504 照旧 `Transient`；`llm_context/src/error.rs` `ContextLimit` 注释补“AICC `context_length_exceeded`、传输层 413”（只改注释） |
| X2 | `aicc_provider_failure`：拒绝码 → `Refused` / `QuotaUnavailable` → `Transient`（不变）；`context_length_exceeded` 或 `details.http_status = 413` → `ContextLimit`；`timeout` 或 `retriable` → `Transient`；其余 `None`（→ `Unknown`）。原“`ProviderError` + 408 / 504”删除：AICC 把 408 / 504 归 `ProtocolErrorKind::Timeout` → `timeout`，其余 5xx 落 `Transport` → `retriable` |
| X3 | buckyos-api `AiccErrorCode::ContextLengthExceeded`（`context_length_exceeded`，走 `ReasonError` body，非 policy refusal）；AICC `ProtocolErrorKind::ContextLengthExceeded` → 该码、`retriable = false`，不同模型重试、不 failover（与原 `InvalidRequest` 行为一致），健康度记 Permanent（不计入熔断）；`error/mod.rs` `openai_error_kind`（结构化 `error.code == context_length_exceeded` 优先于 HTTP 状态），用在 `openai_responses.rs` `openai_http_error` / `response_failure`（`response.failed` 事件）与 `openai_chat_completions.rs` `decode_error_response` / 流内 error chunk；路由：`routing/mod.rs` `CONTEXT_LENGTH_INSUFFICIENT` + `RoutingError::is_context_length_insufficient`（精确模型的过滤原因全是它，或逻辑路由中有候选**只**因它被过滤——缩小后可路由；隐藏模型带 disabled 原因不会命中），`service/inference.rs` 据此报 `context_length_exceeded`，受限用户的文案不含模型名；websdk `src/aicc_client.ts` `AiccErrorCode` 加 `'context_length_exceeded'` |
| X4 | xllm：`aicc_retriable_flag_decides_recoverable_in_rpc_and_failed_responses`（替换原 timeout 测试，7 组 code/status/retriable）、`aicc_refusals_stay_refused_even_when_marked_retriable`、`aicc_context_refusals_are_context_limits`（新码与 413 两条路径）、`rpc_payload_too_large_compacts_and_continues_the_same_run`（脚本化客户端，413 → waist `ContextLimitReached{ProviderRefused}` → 压缩 1 次、同一 run 完成）、`aicc_retriable_provider_error_pauses_the_run`（503 `retriable` → `Paused`、`recoverable: true`；`retriable=false` → `Failed`）。AICC：`error/mod.rs` 映射、`openai_responses.rs` HTTP 400 / `response.failed`、`openai_chat_completions.rs` HTTP 400 / 流内 chunk（含“文本提到 context 但码不是”的反例）、`routing/mod.rs` 逻辑路由 `NoCandidate` 与精确模型 `ExactModelUnavailable` 命中、输出超限不命中。buckyos-api `context_length_exceeded_has_its_own_wire_code` |
| X5 | `cargo test -p aicc`（793）、`-p agent_tool`（343，5 ignored）、`-p llm_context`（105）、`-p buckyos-api`（lib 344 + 集成）全过；AGENTS.md 回归 `cargo test -p libopendan --test xagent_lib work_sessions_take`、`--test xagent_cli skills_are`、`--test context_limit`（8）全过；websdk `tsc --noEmit` 对 `aicc_client.ts` 无新错误 |
| X6 | `doc/llm_context/local_llm_context_protocol.md` §4 补 AICC 错误归类表（原写“`Failed` 转 Unknown”已过时）、§9.3；`xllm_rust_sdk.md` 压缩触发与 paused 行；`doc/aicc/frozen_trait_protocol.md` §6 冻结码表 + 语义、`aicc_api设计.md` 错误码表、`aicc_router.md` §14 注明实现码；cmd_skill `xllm.md` “When it fails”：413 / 上下文超限改为压缩续跑（≤3 次，仍放不下 exit 1 `context_capacity`），exit 3 注明 AICC `retriable` |

与原方案的出入：

- P2 在 `retriable` 判定前先判 `details.http_status = 413` → `ContextLimit`。AICC `protocol_error_kind_from_http_status` 的兜底 `_ => Transport` 让 Provider 的 413 也带 `retriable: true`，不先判就会被当成可重试反复重发同一个放不下的请求。AICC 的状态码映射没改（媒体类 adapter 共用它，413 归成上下文码不合适）。
- P3 只接了 OpenAI 兼容协议（Responses、Chat Completions 及其方言）的结构化码。Claude（`invalid_request_error` + 文本）、Gemini（`INVALID_ARGUMENT` + 文本）的上下文超限没有专门的码，照“只按枚举判断”不识别，仍为 `invalid_request` → `Unknown`；MiniMax 的数字码（`minimax_messages.rs` 1039 等）是否表示上下文超限没有官方依据可核，未接。
- 路由层：xllm 目前不传 `estimated_input_tokens` / `min_context_tokens`，路由的 `context_length_insufficient` 对 xllm 暂不会出现；映射已就位，宿主单 H2（窗口上限）或后续估算接入后生效。

未做 / 风险：

- **部署顺序**：P2 不先于宿主单 H1 部署（§3 P2）。
- DV 未做（§6，随宿主单）。
- websdk `AiccErrorCode` 原本就缺 `unsupported_operation`，与本单无关，未补。
- 2026-11-06 发现：P1 的 413 兜底对直连的大请求不生效。超过服务上限且约 2 MB 以上时，客户端拿不到 413，而是 `Broken pipe`，被归为 `Transient`。根因和修复方向见 [AICC 上限单](./aicc-request-body-limit-todo.md) §9。
