# Agent Session 协议（反写 Spec）

- 版本：5（session_input 为 /3，session_config 为 /5，session_state 为 /5，binding 为 /3，xllm RunRecord.version = 5；summary 与机械渲染保持 /2，快照 snapshot_version = 4）。版本 5（2026-10-03，xagent）把 session_config 升到 /5：`session.policy`（Session 模板）、`origin.report / created_by_call`（Sub Session）、`prompt.frozen`（behavior 冻结）、`prompt.initial_inputs`（无输入队列的首批输入）；登记表 `status` 增加 `waiting_for / turn_open`；内部输入源 `_bootstrap`、`_child` 与 `session:<sid>` task；tmux runtime 的 id 等于 session_id。同一版本内的增量（2026-10-03，OpenDAN Loader，均为可缺省字段）：`state.outbox`（出站消息的待发列表；2026-10-04 增加 `purpose` 与占位 / 最终 edit）、`state.turn_tasks` 与 `origin.parent_task`（Turn ↔ task 绑定，2026-10-04）、`channels.outbound`（回复坐标，原为未定型的占位）、登记表 `route_key`、正式的 ui kind。版本 4（2026-10-03）引入 Session Input Bus 协议 3：总线记录收敛为 `msg / event / control`，消息体直接用 MsgObject，输入模板 `on_init / on_input / on_context_switch` 与半订阅快照，`pending_events` / `reply` / `watched_tasks`，挂起调用在上下文之外等待 task；旧版本的 Session 在显式迁移前只读。版本 3 引入共享 Runtime 的有效配置与实际目标绑定，不改变 worklog 形状。版本 2 引入逻辑 Turn（`turn_seq` / `open_turn` / `turns_completed`）、新的 worklog 条目与拆分的 flush 游标，并把工具预算改名为 `max_tool_iterations`；Round / Step / Turn 的定义见 [LLM Context readme](../../llm_context/readme.md)
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
| [Session Input Protocol](<Session Input Protocol.md>) | Agent 输入：总线记录（`opendan.session_input/3`）、MsgObject 消息与 AgentEvent、校验与拒绝、路由、消费进度、半订阅状态、Input 链路与内建渲染、receipt 与提交、bridge、长任务等待、Sub Session 的创建 / 汇报 / 等待 |
| [Session Control Protocol](<Session Control Protocol.md>) | Session 控制：stop / decide / subscribe / unsubscribe / activity / perceive；驱动者自己的停止请求 |
| [Agent State Protocol](<Agent State Protocol.md>) | AgentRoot 布局、登记表、活动视图、感知、认知边界、产物列表与 decide、behavior 目录与 Session 模板 |

## 3. 机器可读材料

- `schema/*.schema.json`：由 `libopendan::protocol::json_schemas()` 导出，覆盖全部持久结构（`cargo run -p libopendan --bin xagent -- schema <dir>` 可重新导出）。
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
| 09_semi_subscription | 半订阅按登记表 rev 拉取变化并入 `pending_events`，在下一次受控输入之前作为半订阅快照消息注入，不额外推理 |
| 10_active_overlap | 两个活动 session 同一 workspace：渲染对方 activity 并标出交集 |
| 11_worklog_with_summary | 反向读 worklog 在 summary 起点停止 |
| 12_fork_child_live | fork 子 context 的 run 在跑、调用方的 run 作为 `caller` 帧挂起在 process_stack；两个 run 同属仍打开的 Turn 1 |
| 13_unsupported_snapshot_version | 快照版本不支持：RecoveryBlocked，保留现场 |
| 14_input_bus | 没有 session 状态：手工投递的总线记录（`records/`）及各自的处理、每种拒绝原因的记录（`rejected/`）、同一批输入的逐字节渲染（`rendering/`：`input_text.xml`、内建模板、半订阅快照、六个示例模板的 `.tpl` / `.out`、模板变量 `vars.json`、`input.media = reference / inline` 的 user 消息） |

其余 §11 场景（选择性消费、订阅丢失重建、ack 不回退、持有者退出后接管、非驱动者被拒、binding 准备中断后修复、discard 与 accept 并发、历史段确定性、跨 Agent/App/owner 的 sid）由 `src/frame/lib_opendan/tests/` 的用例覆盖。

## 4. 版本规则

- 各 JSON 文件带 `schema` 字段（`opendan.<name>/<major>`）。读者遇到不认识的主版本必须拒绝（恢复时返回 RecoveryBlocked），不得猜测。当前实现加载 session 时要求 `state.json` / `session_config.json` 的 schema 精确等于当前版本；更旧的主版本以 RecoveryBlocked 拒绝推进，投递返回 `session_readonly`：旧 Session 在显式迁移前只读（可查看、导出，不能 append 或 drive）。
- 同一主版本内只做**加法**：新增字段都有默认值；读者必须容忍未知字段，写者必须保留自己不理解的宿主元数据（如快照的 `state.host`）。
- 主版本升级是 breaking change（beta 2.2）：不提供 serde 别名、不双读双写、不迁移旧目录；版本 1 的 session 与 run 只能保留现场，不能被版本 2 的 runner 推进。
- 快照：`LLMContextState.snapshot_version` 当前为 3（v3 把工具预算字段改为 `tool_iterations_left` / `tool_batch.batch_error`）。`LLMContext::resume` 只接受当前版本，更旧或更新的快照都拒绝恢复（RecoveryBlocked）。
- xllm run 记录：`RunRecord.version` 当前为 3（有效 runtime 与 runtime_descriptor）；不等于当前版本时 xllm 拒绝接手，libopendan 返回 RecoveryBlocked。
