# Agent Session 协议（反写 Spec）

- 版本：2（对应 `opendan.session_config/2`、`opendan.session_state/2`、`opendan.session_summary/2`、渲染器 `libopendan.mechanical/2`、快照 `snapshot_version = 3`、xllm `RunRecord.version = 2`）。版本 2 引入逻辑 Turn（`turn_seq` / `open_turn` / `turns_completed`）、新的 worklog 条目与拆分的 flush 游标，并把工具预算改名为 `max_tool_iterations`；Round / Step / Turn 的定义见 [LLM Context readme](../../llm_context/readme.md)
- 日期：2026-09-29；2026-10-01 按 Round / Step / Turn 术语统一更新
- 来源：由 Rust 参考实现 `src/frame/lib_opendan`（crate `libopendan`）反写（[实现计划](<../Agent Session SDK 实现计划.md>) L6 / V6）。字段以 `src/protocol/` 的类型为准，本目录的 JSON Schema 由这些类型导出。
- 读者：实现其它语言 runner（buckyos-websdk 的 ts-runner 等）的开发者，以及审查协议的人。

## 1. 什么是协议，什么不是

| 是协议（所有语言必须一致） | 不是协议（各语言自行实现） |
|---|---|
| session 目录布局与各文件格式 | Runner 的内部结构（drive 循环、装配器、工具管理器） |
| 提交顺序（什么先落盘、什么是提交点） | 提示词内容与渲染细节（只要求同一渲染器版本内确定） |
| 锁语义（长期持有的排他文件锁） | LLM Provider 的接入方式 |
| 输入消息格式、消费与确认规则 | kmsg / kevent 客户端的实现 |
| AgentRoot 上 Agent State 的文件布局与单写者规则 | `AgentStateClient` 的接口形态（文件版 / kRPC 版） |
| xllm 的 run 目录（`runs/<run_id>/`）及其中宿主相关字段 | 执行跟踪的平台实现（本实现用 Linux `/proc`） |

## 2. 文档

| 文档 | 内容 |
|---|---|
| [Session Directory Protocol](<Session Directory Protocol.md>) | 目录结构、session_config / state / summary / worklog / static / binding / runs、逻辑 Turn 的打开 / 继续 / 关闭、提交顺序、恢复规则、sid 规则 |
| [Lease Protocol](<Lease Protocol.md>) | 推进权与各类锁、锁文件内容、接管前的执行核对 |
| [Session Input Protocol](<Session Input Protocol.md>) | kmsg 队列与消息格式、五类输入、消费进度、输入批次与 receipt（`turn` / `opens_turn`）、control 命令、唤醒 |
| [Agent State Protocol](<Agent State Protocol.md>) | AgentRoot 布局、登记表、活动视图、感知、认知边界、产物列表与 decide |

## 3. 机器可读材料

- `schema/*.schema.json`：由 `libopendan::protocol::json_schemas()` 导出，覆盖全部持久结构（`cargo run -p libopendan --example session -- schema <dir>` 可重新导出）。
- `fixtures/<NN>_<scenario>/`：由 `cargo run -p libopendan --example fixtures -- doc/opendan/protocol/fixtures` 生成的黄金目录。每个场景包含 `agent_root/`、`app/<sid>/`、`kmsg/`（开发用文件队列，语义同 kmsg）和 `expected.json`。文件里的绝对路径已替换为 `${FIXTURE_ROOT}`，使用前替换为拷贝后的场景目录。
- `expected.json` 的 `sessions[]` 记录解析结果（state 摘要、live_run、run.json 的门槛与在途、worklog 边界），`next` 记录符合协议的 runner 下一步必须做什么。Rust 参考实现用 `tests/fixtures.rs` 在每个场景上验证 `next`；其它语言应跑同一组断言。

| 场景 | 要点 |
|---|---|
| 01_new_work_session | 刚创建、目录在 AgentRoot 之外；首次推进先绑定 runtime，再以 `on_init` 输入批次打开第一个 Turn |
| 02_finished_work_session | finished + acceptance pending；只接受 `control(decide)` |
| 03_orphan_run_pending_host_commit | 新 run 已写快照与门槛，state 未引用：删除孤儿 run，重新取输入；xllm 拒绝接手 |
| 04_gate_pending_after_state_commit | state 已提交、门槛未清：清门槛、补确认，续跑同一 run，不重复追加消息 |
| 05_uncommitted_worklog_tail | run 已到终态、worklog 有未提交尾部：截断后从终态记录重做结束，不调用 LLM |
| 06_killed_during_exec | 在途 c1 + 执行标识：先确认旧执行已停止，再把 c1 物化为“结果未知”续跑，不重放工具 |
| 07_receipt_ahead_of_state | 续用 run 的快照领先 state 一批：按 receipt 补交，不重复注入 |
| 08_finished_with_decide | 应用 decide(accept) 移动产物 head，拒绝迟到的 msg 并记 worklog |
| 09_semi_subscription | 半订阅按登记表 rev 拉取变化，随下一个输入批次注入，不额外推理 |
| 10_active_overlap | 两个活动 session 同一 workspace：渲染对方 activity 并标出交集 |
| 11_worklog_with_summary | 反向读 worklog 在 summary 起点停止 |
| 12_fork_child_live | fork 子 run 在跑、父 run 挂起在 process_stack；父子 run 同属仍打开的 Turn 1 |
| 13_unsupported_snapshot_version | 快照版本不支持：RecoveryBlocked，保留现场 |

其余 §11 场景（选择性消费、订阅丢失重建、ack 不回退、持有者退出后接管、非驱动者被拒、binding 准备中断后修复、discard 与 accept 并发、历史段确定性、跨 Agent/App/owner 的 sid）由 `src/frame/lib_opendan/tests/` 的用例覆盖。

## 4. 版本规则

- 各 JSON 文件带 `schema` 字段（`opendan.<name>/<major>`）。读者遇到不认识的主版本必须拒绝（恢复时返回 RecoveryBlocked），不得猜测。当前实现加载 session 时要求 `state.json` / `session_config.json` 的 schema 精确等于 `/2`，更旧的主版本同样以 RecoveryBlocked 拒绝。
- 同一主版本内只做**加法**：新增字段都有默认值；读者必须容忍未知字段，写者必须保留自己不理解的宿主元数据（如快照的 `state.host`）。
- 主版本升级是 breaking change（beta 2.2）：不提供 serde 别名、不双读双写、不迁移旧目录；版本 1 的 session 与 run 只能保留现场，不能被版本 2 的 runner 推进。
- 快照：`LLMContextState.snapshot_version` 当前为 3（v3 把工具预算字段改为 `tool_iterations_left` / `tool_batch.batch_error`）。`LLMContext::resume` 只接受当前版本，更旧或更新的快照都拒绝恢复（RecoveryBlocked）。
- xllm run 记录：`RunRecord.version` 当前为 2（`config.limits.max_tool_iterations`）；不等于当前版本时 xllm 拒绝接手，libopendan 返回 RecoveryBlocked。
