# libopendan

以某个 Agent 的身份推进一个 Agent Session 的 Rust 参考实现（[实现计划](<../../../doc/opendan/Agent Session SDK 实现计划.md>)）。协议（目录结构、提交顺序、锁语义、输入格式、xllm run 目录）见 [`doc/opendan/protocol/`](../../../doc/opendan/protocol/README.md)；本 crate 里 Runner 的结构不是协议。

## 模块

| 模块 | 内容 |
|---|---|
| `protocol` | 全部磁盘 / 队列结构（serde + JSON Schema 导出） |
| `fsutil`、`lock` | 原子替换、批量追加、反向读、不覆盖发布；长期持有的 flock 锁 |
| `session` | `SessionDir`（读取、创建发布）、`Session`（持锁提交 state.json）、worklog、`runs/`（xllm RunStore 封装） |
| `channel` | kmsg 输入（`KmsgInput`）、开发用文件队列 `DirMsgQueue`（kmsg 语义）、kevent 唤醒 |
| `bridge` | msg bridge（msg-center 记录 → 总线记录，只过滤与分流：只放行 Owner，群消息要求开关与 @；较早记录转成 `delivery.context` 上下文消息）、task bridge（task 状态 → `AgentEvent`）、回复信封与出站记录；宿主内的 `EventBridge`：timer、kevent |
| `state` | `AgentStateClient` 与文件实现：登记表（含 `children_of`）、活动视图、感知、认知门面、产物列表、Agent 级锁、behavior 目录（`BehaviorCatalog`、冻结）；`connect`（进程内 → AgentRoot → kRPC）、`krpc`（`KrpcAgentStateClient` 与传输无关的服务端分发 `serve_call`：读与带署名的写；驱动者的写入不上 kRPC）、`ForwardingStateClient`、`WithBehaviors`；`goals`（`GoalStore`：Goal 定义 / 修订 / 变更 CAS、按调用方类型的权限、驱动者的 Goal State / Activation 记录发布、宿主的 epoch 准备与计划进度） |
| `memory` | Agent Memory 组件门面（[需求](<../../../doc/opendan/Agent Memory 认知管理需求.md>) 附录 A）：感知写入（宿主绑定身份、授权主体与来源，幂等键、文件短锁）、两层召回 `query_topic` / `query` 与按引用读取（可见性、范围、纠正兜底、分池预算）、`changes_since`（版本向量快照，快速路径不打开 Graph）、Session 侧 `ObservationState`（topic、阀门、pending、read_set）、持整理 lease 的 `Consolidator`（批次、单 envelope 提交、清理、到期清扫）、`pending_work` / `due_reviews`（内置整理 Goal 的 gate 用）；Graph 在 `agent_tool::agent_memory`（schema 4.0：stability / last_evidence_at / review_at） |
| `goal` | Goal 与 Goal Session 的共用部分：`ensure_goal_session`（按 `(goal_id, epoch)` 派生）、运行机会事件（`post_activation` / `post_feedback`，稳定 key）、有效预算与统计窗口、Work 准入（`origin.goal`、周期上限） |
| `notify` | 系统通知的 Agent 侧（`doc/message_hub/System Notify.md`）：宿主接口 `NotifyHost`（send / state / list / cancel / answer）、`ZoneNotify`（msg-center + TaskMgr；Loader 用它，Session 里的 `agent-session` 在文件版 Agent State 没有通知服务时用 `for_session_process()` 以会话 token 直连）、`LocalNotify`（`state/notifies/` 文件实现，目录存在即启用，供 `--dev` 与测试）、`StateNotify`、`complete_notify_source`（按 Session 补全所属 Goal 与用户所在会话）、`notify_event`（`_notify` 观察事件）。Work 的 `ask-user` 经通知有界等待、Goal `questions` 发成通知都在 runner 里，宿主只给 `RunnerDeps.notify` / `HostDeps.notify` |
| `template` | Session 模板（`work / ui / self_check / goal`，`agent.toml [session.<class>]` 覆盖；包自定义的 class 用 `base = "<内建模板>"` 指定起点，缺省 `work`）→ `SessionSpec` 与 `session.policy`。ui 模板的 kind 是 `ui`，由 `route_key` 绑定到一个会话 |
| `host` | 一个进程托管多个 Session：`HostDeps`（每个 Session 自己的 runtime）、`ChildDriver`（推进子 Session）、`run_session`、`serve`；常驻宿主用的 `Supervisor`（按登记表托管 `driver = me` 的未结束 Session 与子 Session、`ensure_task`、按 class 的 idle unload、只读托管状态、退出时等待各循环结束而不 stop Session）；`host::goals`（`GoalHost`：按 Goal 列表补建 Goal Session、epoch 迁移与结束、`ScheduleBackend` / `LocalScheduleBackend`、可扩展的准入链，`Supervisor::with_goals`、`goal_activate`、`goal_feedback`）；`host::maintenance`（内置整理 Goal `memory_maintenance` 的播种与 30m / 2h 确定性 gate，水位在 `schedule.json.gate`） |
| `runtime` | 复用 agent_tool::runtime；Session 的 session-runtime 构造（`runtime_for_session`）、Session bin/helper、绑定（`binding/5`，workdir = Session 工作目录）与环境核验、`run_probe`、`check_baseline`、`workspace_view` |
| `baseline` | 标准环境的命令清单（`xagent runtime baseline [--check]`，镜像构建与 runtime 检查共用）、host 的多入口布置（`install_entries`）与 AICC 命令启动（`run_host_entry`） |
| `session_context` | Session 进程的上下文（`.opendan/session.json` + session token、`SessionClient`）与 AgentService 侧 `SessionService`（`session.helper / task / token_refresh / service_token`） |
| `transfer` | runtime 间复制（`agent-session copy`）：两端 `RuntimeFiles`、临时文件 + `expected_size` 追加、整文件 hash 核验、rename 提交、`state/transfers/` 记录、重试核验与取消 |
| `runner` | `drive`：恢复、输入路由（`inputs`）、模板视图与内建格式（`input_view`）、输入批次提交（半订阅快照 + 受控输入，开启或加入逻辑 Turn）、Outcome 处理与 Turn 关闭（`StopWhen::TurnClosed`）、挂起调用的 task 等待与回填、后台 task 跟踪、process 切换、压缩、Round 统计、子 Session 的隐式关注与 `session:<sid>` resolver（`children`）、`BehaviorAssembler`、驱动者停止请求（`StopSignal`）、出站（`outbound`：Turn 关闭的提交里写 `state.outbox`，提交后交给 `OutboundSink`，重启后原样重发）、Goal Session（`goal`：Goal State 随 Session 提交再发布、状态闸门、Goal 头、Activation 的打开 / 预算 / 关闭与写回、Work 的跨 epoch 吸收）、整理（`consolidation`：`memory_consolidation` lease、宿主工具 `memory_consolidation_begin / commit`、receipt 与崩溃恢复、机械维护、`ctl perceive` 经 Memory 门面写入） |
| `api` | `create_session` / `read_session` / `post_input` / `create_sub_session` |
| `bin/xagent` | 命令行：`xagent new / run / serve / post / ctl / status / list / behaviors / xllm / schema / goal`，以及作为 `agent-session` 被 shell 里的命令调用的层 ② 工具 |

## 使用

自动 Session ID 与目录名均为 `<kind前缀>-YYYYMMDD-N`（如 `work-20260110-23`），保持 `sessions/<session_id>/` 布局。日期取分配主机本地日期，同一 Agent 按当天已有目录、登记和预留记录的最大编号加 1；幂等身份键到短 ID 的映射保存在 `state/sessions/.ids/`，重试和跨日恢复沿用原 ID。节点工作目录同样使用短 Session ID；长期 Workspace 使用创建或导入时显式指定的目录。

```rust
let channels = Arc::new(KmsgChannels::new(kmsg_client));          // 或 KmsgChannels::dir(dev_dir)
let agent = Arc::new(FsAgentStateClient::open(agent_root, agent_did, Some(channels.client()), None)?);
let sd = libopendan::create_session(&app_dir, SessionSpec::work("…"), agent.as_ref(), "app:app2@alice", channels.as_ref()).await?;
let deps = RunnerDeps::new("app:app2@alice", agent, channels,
                           Arc::new(NativeRuntime::local("rt-app2", "app:app2")), XllmDeps::default());
let result = SessionRunner::new(deps).drive(&sd, StopWhen::Finished).await;
```

LLM Provider 由 `session_config.prompt.llm_context`（xllm `.llm_context` 的 JSON 形式）决定，工具预算键为 `max_tool_iterations`。`StopWhen::MaxOutcomes { n }` 让 drive 处理 n 个 `LLMContext` outcome（每个启动或恢复的 run 段一个，任何种类）后返回 `DriveResult::OutcomesHandled`；它不是 Round（推理）数、`run()` 调用数，也不是 Turn 数。Session 的结束条件另由 `end_condition`（如 `max_turns`）按已完成的 Turn 计。Round / Step / Turn 的定义见 [LLM Context readme](../../../doc/llm_context/readme.md)。

持久格式为协议版本 7（session_input /3、session_config /7、session_state /6、binding /4，xllm RunRecord.version = 6；summary /2、机械渲染 /3、快照版本 5）。旧格式的 session / run 不迁移、不按旧字段读取，加载时返回 RecoveryBlocked，投递返回 `session_readonly`。

RunnerDeps.runtime 使用 agent_tool::runtime::AgentRuntime；.llm_context.runtime 是构造配置，session_config.runtime.requirement 是绑定要求。binding 保存实际 target 和执行 cwd，推理与旧执行恢复前先核验。SessionToolManager 保留协议纪律，内部调用 Sandbox。Session 的 .runtime/bin 与 Agent tools 作为宿主环境注入；独立 xllm 接管校验保存的 PATH、manifest、helper 与凭据环境引用。远端 Session helper 未部署时明确报 Capability；remote_ssh 可独立用于 xllm。

宿主已注册 BuckyOS runtime 时，Session 环境与每次 native 命令执行前都会续期并读取当前 session token，注入 `BUCKYOS_APPCLIENT_SESSION_TOKEN`，覆盖父进程、Session 配置或单次调用中的旧值。续期失败或 token 为空时阻止执行；工具沿用宿主身份，不需要 owner 私钥。run 的环境核验记录只保存凭据变量名，不保存 token 值。未注册 runtime 的独立开发形态仍可显式传入该环境变量。

Session 的宿主 `report` 工具提交阶段报告或以 `is_end=true` 请求完成；Behavior XML 用 `<report end="true">`，结束与动作/切换互斥。`session.policy.completion = explicit_report` 可要求显式完成（默认 natural）；无输入队列时漏报会失败，有输入队列的普通 Done 关闭 Turn 并等待。报告、显式稳定产物和可选 JSON result 统一持久化，恢复不重执行报告、不新增推理；子 context 的结束只交回调用方。持久身份、提交窗口与历史投影见 [Session Directory Protocol §7.1](<../../../doc/opendan/protocol/Session Directory Protocol.md>)。

UI Session 的结构化回复只交付 `reply` 正文，`topic` 和 `artifacts` 分别更新话题和附件；空正文不回退到 JSON 原文。格式或附件校验纠正一次后仍失败时，Turn 以 `output_format` 错误结束，由宿主发送失败提示，原文保留在 run / worklog 中供诊断。恢复已结束的 run 时执行相同的校验和交付规则。

UI Session 的 `function_call` context 在已关闭的 Turn 之后继续同一 context、继承 `recent_dialogue` 且尚未生成摘要时，从 `last_run` 快照的 `accumulated` 原样继承非 system 消息列表，保留角色、工具调用与结果、媒体等内容，不机械截断或包装成 `<session_history>`。新 run 的快照保存完整历史前缀，之后清理旧 run 不影响续聊。原始历史超过 `history_budget_tokens` 时先实际压缩；压缩后，以及 behavior loop / context 切换的历史装配，仍使用 `<session_history>` 中的摘要与机械渲染 worklog。持久格式不变，见 [Session Directory Protocol §6](<../../../doc/opendan/protocol/Session Directory Protocol.md>)。

## xagent

设计见 [xAgent](../../../doc/opendan/xAgent.md)。xagent 是 xllm 的上一层：xllm 把一个 run 推进到一个 Outcome，xagent 把一个 Agent Session 推进到一个 Turn 关闭，或常驻地推进它。

在 `src/` 下执行 `uv run buckyos-build.py -s xagent` 会构建并将二进制放入 `rootfs/bin/opendan/xagent`，随后 `uv run buckyos-install.py` 将其安装到 `$BUCKYOS_ROOT/bin/opendan/xagent`（Windows 为 `xagent.exe`），与提供 `xllm` 子命令的 `agent_tool` 同目录。`build_aios` 同样打包该二进制，paios 容器内可直接执行 `xagent`。

```bash
export OPENDAN_AGENT_ROOT=/tmp/x/agent_root OPENDAN_AGENT_DID=did:bns:jarvis.alice LIBOPENDAN_WHO=app:app2@alice
export LIBOPENDAN_QUEUE_DIR=/tmp/x/kmsg     # 开发用文件队列；不设则使用所在 zone 的 kmsg
LC='{"provider":{"type":"openai","base_url":"http://127.0.0.1:8000/v1","api_key_env":"KEY"},"model":"m","tools":{"enabled":true}}'
cargo build -p libopendan --bin xagent
xagent new --objective "…" --llm-context "$LC" --msg "…"          # work：一个 Turn、无队列；创建并推进到 Turn 关闭
xagent new --class ui --objective "…" --llm-context "$LC" --no-run
xagent run <sid|dir> [--msg <text>] [--until turn|finished|idle|outcomes:<n>] [--detach-children]
xagent serve <sid>... [--idle-unload <secs>]
xagent post <sid> --msg <text> [--from <did>] [--attach <obj_id>]… | --json <file | ->
xagent ctl <sid> stop | decide accept|discard | subscribe <spec> | unsubscribe <id> | activity … | perceive <text>
xagent status <sid> [--worklog <n>] [--report] [--run] [--events] ; xagent list [--active]
xagent behaviors [--frozen <sid>] ; xagent xllm <sid> ; xagent schema <dir>
```

驱动命令的 stdout 是一个带 `kind` 的 `DriveResult` JSON（`new` 另带 `session_id` / `path`，仍有未结束的子 Session 时带 `children`），日志与诊断在 stderr。退出码：0 完成，1 失败，2 参数 / 配置错误，3 未完成（idle、Turn 仍打开、input_full），4 被 stop / 中断，5 Busy，6 阻塞（非驱动者、未登记、绑定、runtime 不符、RecoveryBlocked、只读）。SIGINT 对 `new` / `run` 是对所驱动 Session 的 stop；对 `serve` 只结束托管。

推进时 xagent 把自身包装成 session 的 `.runtime/bin/agent-session`，`shell` 里的命令用它访问本 Session 与 Agent State：`agent-session ctl activity --touch ws:foo`、`ctl perceive "…"`、`recall <tag>…`、`note <text>`、`sessions [--children] [--active]`、`read-session <sid>`、`artifact head|list`、`create-worksession --objective … [--behavior <b>] [--runtime <id>] [--report final|progress|none] [--wait] [--interactive]`、`wait <sid>`、`post <sid> --msg …`、`runtime list|get|check`、`ask-user "<问题>" [--option <id>=<label>]... [--allow-text]`（打印 `Pending{userinput:<call_id>}`；ui 类 Session 把问题以 `thread → 占位` + `agent_ask` 发给用户，下一条用户消息回填为这次调用的结果；没有队列的 Work 在宿主有通知服务时把问题作为 `input` 通知发出，有界等待其结果，见 [Session Input Protocol](<../../../doc/opendan/protocol/Session Input Protocol.md>) §9）、`notify send|list|show|answer|ack|cancel`（Session 内：发告知、列出本 Agent 的通知、代用户回答 / 确认在本会话呈现过的通知，不能处理本 Session 发起的；`xagent notify …` 在 Session 外扮演用户，供 `--dev` 与测试）。

behavior 可以用 `[tool]` 把自己（`create_sub_context` / `fork`）声明为调用方模型看到的具名函数，调用方在 `tool_whitelist` 写 `behavior:<name>`；运行时由 `runner::tools::BehaviorTool` 生成（通用的 `call_behavior` 也是它），内部协议同 `call_behavior`。fork 目标的 `[budget]` 作用于分支的 run。

## 测试

```bash
cargo test -p libopendan -- --test-threads=1
```

| 文件 | 覆盖 |
|---|---|
| `tests/report.rs` | 阶段/最终报告、拒绝可修正、批次配对、显式完成策略、子 context 归属、稳定产物/JSON result、崩溃重做与历史去重 |
| `tests/l1.rs` | 锁（epoch、inode 不变、CLOEXEC、kill -9 后接管）、反向读有界、压缩无空洞、kmsg 规则（文件队列与 kmsg 的 sled 实现）、订阅丢失重建、迁移、巡检、幂等创建、绑定失败、墓碑修复、活动视图 |
| `tests/runner_basic.rs`、`tests/runner_more.rs` | work session 端到端、finished 后拒绝输入、非驱动者 / Busy、stop、事件路由（active / semi / 未订阅丢弃 / 订阅变更按投递顺序生效）、半订阅快照、behavior loop、普通 / fork / independent 切换（同一 Turn 内交接）、`max_turns` 只计已完成 Turn、decide 与 head、activity / perception 输入 |
| `tests/crash.rs` | 子进程在各提交窗口 abort（`LIBOPENDAN_FAULT`，如 `input_batch:after_state_commit`、`finish_run:after_flush`、`user_input:after_question_commit`、`user_input:after_input_checkpoint`）或执行中被 kill -9 后恢复；问题只发一次、答案只消费一次；版本不支持时阻塞；xllm 接手与拒绝 |
| `tests/input_tasks.rs` | 输入协议 3 与长任务：挂起调用在上下文之外等待 task 并回填同一 run、无 resolver 拒绝接手与 Unknown 回填、等待期间 stop、后台 task 完成合成 Input 事件、run 终态落盘后崩溃仍找回后台 task、工具执行中的 stop、64 条 pending 上限、Single / Batch、`input.media = inline`、重投去重与回复路径、旧 Session 只读、模板失败不消费、用户时区半订阅 |
| `tests/memory_maintenance.rs` | 内置整理 Goal：播种幂等与零模型调用、材料经宿主工具整理、`memory_consolidation` lease、不记录自身感知、未提交批次的报告检查、pause / resume 的 gate、receipt 各阶段崩溃恢复（子进程 abort） |
| `tests/memory_sessions.rs` | Memory 多 Session 模拟的十个场景（与 `cargo run -p libopendan --example memory_sessions` 共用 `examples/support/memory_*.rs`），每个场景记录的检查都必须通过 |
| `tests/memory_component.rs` | Memory 组件：并发写与观察无空隙、幂等键、批次与终局处置冲突、提交故障注入与恢复、pending 溢出重建、失败不当空结果、未变化与分页、整理 lease、预算分池与淡出、校验、派生物删除与修复、清理与追加并发、写锁期间观察、运行摘要、两进程 |
| `tests/ask_user.rs` | `ask-user` 与 `[tool]`（意图分析 TODO V05–V10、V12、V14、V15、V22）：具名函数进入 fork、前缀字节一致、占位立即发出、问题 `thread → 占位` 带 `agent_ask`、等待 `input`、下一条消息回填（文本 / 选项 / 非回复 / 无占位）、`report` 的 `submission` 交回、Work 不能问、stop、嵌套、事件留给调用方、xllm 拒绝接手、fork 预算、同名函数与非法目标被拒 |
| `tests/runtimes.rs` | Runtime 登记（V01–V04）：内建补建与保留 owner 修改、内建不可删、CAS、kRPC 署名、Workspace 按登记判断（`local` 即 `container`、`host` 拒绝）、`create-worksession --runtime` 查登记 |
| `tests/session_runtime.rs` | session-runtime 与 Workspace 解耦（primary-runtime TODO）：Workspace 不选 runtime、子 Session 换 runtime 继承 Workspace、默认 cwd 与提示词里的 Workspace、cmd_skills 按声明基线 / 平台过滤与显式指定被拒、session token 与 `session.helper / task / token_refresh`（真实 `xagent`）、Session ↔ Workspace 的复制（中文路径、9 MiB、重试不重复、`--replace`、目录目标、拒绝的情形）与 `agent-session copy` |
| `tests/outbound.rs` | 回复随 Turn 提交并沿来路发出、sink 不可达时保留并原样重发（同键同 ObjId）、被拒与路由不一致只记录不重试、没有 sink 时不产生 outbox |
| `tests/fixtures.rs` | 参考实现在 `doc/opendan/protocol/fixtures` 每个场景上满足 `expected.json`；`14_input_bus` 的记录处理、拒绝原因与逐字节渲染 |
| `tests/xagent_lib.rs` | 无队列 work Session 与 `prompt.initial_inputs`、`WAIT_USER_MSG` 的模板语义、`TurnClosed` / `TurnOpen`、behavior 冻结 / 补冻结 / 缺失冻结、三种 Agent State 实现结果一致、Sub Session（父等待汇报、`session:<sid>` 回填、数量与深度、交互式子提问、stop 级联、进度半订阅）、驱动者停止请求、on_context_switch 前的半订阅快照、timer bridge |
| `tests/xagent_cli.rs` | `xagent` 可执行文件（本地 OpenAI 兼容 mock）：`new --no-run` / `run` / `post` / `ctl` / `status` / `list` / `xllm` / `schema` 的退出码与 stdout 单个 JSON、无队列投递拒绝、ui 模板等输入与同一 Turn 续答、经 `shell` 的 `agent-session create-worksession --wait`、xllm 接手后由 xagent 提交结束、第二个驱动者 Busy 与 SIGINT、`serve` |
| `tests/notify.rs` | 系统通知（通知人 TODO §5 的 V07–V13、V15–V18）：Work 经通知提问、等待与回填（来源、截止时间、HumanInput 等待原因）；超时 / 忽略 / 撤销作为结果；stop 撤销；卸载期间被回答；没有服务时维持原报错；UI 的 `_notify` 观察不开 Turn；`agent-session notify` 发送 / 列出 / 代答 / 拒绝代答本 Session 的 / 已结束返回冲突、`xagent notify` 扮演用户；Supervisor recheck 重新装载等待中的 Work。崩溃窗口见 `tests/crash.rs` 的 `asking_through_a_notification_survives_a_crash_at_every_stage`，Goal 问题见 `tests/goal_session.rs` 的 `goal_questions_reach_the_user_and_answers_come_back` |
| `tests/goal_session.rs` | Goal Session（Goal Session TODO §5 的 V01–V20）：宿主补建与幂等、定时 Activation 与 Goal 头、错过的槽合并、Activation 预算、错误结果被拒、Work 创建与跨 Activation 吸收、修订提示、周期上限、问题结束 Activation、暂停 / 恢复、归档级联、自我归档与 reopen、restart 接管 Work、停止单次 Activation、历史中的 Goal 头、调用方权限、kRPC、skills 控制与反馈；崩溃窗口（建 Session 前、Activation 中 kill -9、写回前后、fire 已记录未投递） |
| `tests/dv_kmsg.rs` | （`--ignored`）真实 kmsg 服务：幂等建队列 / 订阅、消费与累积 ack、finished 后拒绝并 ack；在 DV Test OOD 上以 root 运行 `cargo test -p libopendan --test dv_kmsg -- --ignored` |

重新生成 fixtures / JSON Schema：`cargo run -p libopendan --example fixtures -- ../doc/opendan/protocol/fixtures`（在 `src/` 下执行）。

生成器将超长 fixture 路径缩短为 `_files/<编号>`，并写入 `_paths.json` 映射，避免 Windows checkout 超过路径长度限制；测试加载时还原完整路径。格式见 [协议 fixtures](../../../doc/opendan/protocol/README.md#3-机器可读材料)。

Workspace 管理由 `AgentStateClient.workspaces()` 统一提供，CLI 为 `xagent workspace` / `agent-session workspace`；位置的 Runtime 引用 `AgentStateClient.runtimes()` 的登记（`state/runtimes/`，宿主补建 `container` 与 `host`，CLI `xagent runtime` / `agent-session runtime`，[Agent State Protocol](<../../../doc/opendan/protocol/Agent State Protocol.md>) §10）。创建或导入先显式指定 Runtime、绝对目录和操作 key，创建 Session 时用 `--workspace <稳定ID>` 绑定；Session 的执行位置（session-runtime）与 Workspace 独立选择，默认 cwd 永远是 Session 工作目录，Workspace 在另一 runtime 上时用 `agent-session copy` 交换文件。旧 create_workspace / bind_workspace 工具已移除。格式、迁移和能力边界见 [Agent Workspace Protocol](<../../../doc/opendan/protocol/Agent Workspace Protocol.md>)。
