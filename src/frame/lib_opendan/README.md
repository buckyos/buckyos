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
| `state` | `AgentStateClient` 与文件实现：登记表（含 `children_of`）、活动视图、感知、认知门面、产物列表、Agent 级锁、behavior 目录（`BehaviorCatalog`、冻结）；`connect`（进程内 → AgentRoot → kRPC）、`krpc`（`KrpcAgentStateClient` 与传输无关的服务端分发 `serve_call`：读与带署名的写；驱动者的写入不上 kRPC）、`ForwardingStateClient`、`WithBehaviors` |
| `template` | Session 模板（`work / ui / self_improve / self_check`，`agent.toml [session.<class>]` 覆盖；包自定义的 class 用 `base = "<内建模板>"` 指定起点，缺省 `work`）→ `SessionSpec` 与 `session.policy`。ui 模板的 kind 是 `ui`，由 `route_key` 绑定到一个会话 |
| `host` | 一个进程托管多个 Session：`HostDeps`（每个 Session 自己的 runtime）、`ChildDriver`（推进子 Session）、`run_session`、`serve`；常驻宿主用的 `Supervisor`（按登记表托管 `driver = me` 的未结束 Session 与子 Session、`ensure_task`、按 class 的 idle unload、只读托管状态、退出时等待各循环结束而不 stop Session） |
| `runtime` | 复用 agent_tool::runtime；仅保留 Session bin/helper、绑定与环境核验 |
| `runner` | `drive`：恢复、输入路由（`inputs`）、模板视图与内建格式（`input_view`）、输入批次提交（半订阅快照 + 受控输入，开启或加入逻辑 Turn）、Outcome 处理与 Turn 关闭（`StopWhen::TurnClosed`）、挂起调用的 task 等待与回填、后台 task 跟踪、process 切换、压缩、Round 统计、子 Session 的隐式关注与 `session:<sid>` resolver（`children`）、`BehaviorAssembler`、驱动者停止请求（`StopSignal`）、出站（`outbound`：Turn 关闭的提交里写 `state.outbox`，提交后交给 `OutboundSink`，重启后原样重发） |
| `api` | `create_session` / `read_session` / `post_input` / `create_self_improve_session` / `create_sub_session` |
| `bin/xagent` | 命令行：`xagent new / run / serve / post / ctl / status / list / behaviors / xllm / schema`，以及作为 `agent-session` 被 shell 里的命令调用的层 ② 工具 |

## 使用

```rust
let channels = Arc::new(KmsgChannels::new(kmsg_client));          // 或 KmsgChannels::dir(dev_dir)
let agent = Arc::new(FsAgentStateClient::open(agent_root, agent_did, Some(channels.client()), None)?);
let sd = libopendan::create_session(&app_dir, SessionSpec::work("…"), agent.as_ref(), "app:app2@alice", channels.as_ref()).await?;
let deps = RunnerDeps::new("app:app2@alice", agent, channels,
                           Arc::new(NativeRuntime::local("rt-app2", "app:app2")), XllmDeps::default());
let result = SessionRunner::new(deps).drive(&sd, StopWhen::Finished).await;
```

LLM Provider 由 `session_config.prompt.llm_context`（xllm `.llm_context` 的 JSON 形式）决定，工具预算键为 `max_tool_iterations`。`StopWhen::MaxOutcomes { n }` 让 drive 处理 n 个 `LLMContext` outcome（每个启动或恢复的 run 段一个，任何种类）后返回 `DriveResult::OutcomesHandled`；它不是 Round（推理）数、`run()` 调用数，也不是 Turn 数。Session 的结束条件另由 `end_condition`（如 `max_turns`）按已完成的 Turn 计。Round / Step / Turn 的定义见 [LLM Context readme](../../../doc/llm_context/readme.md)。

持久格式为协议版本 5（session_input /3、session_config /5、session_state /5、binding /3，xllm RunRecord.version = 5；summary 与机械渲染保持 /2，快照版本 4）。旧格式的 session / run 不迁移、不按旧字段读取，加载时返回 RecoveryBlocked，投递返回 `session_readonly`。

RunnerDeps.runtime 使用 agent_tool::runtime::AgentRuntime；.llm_context.runtime 是构造配置，session_config.runtime.requirement 是绑定要求。binding 保存实际 target 和执行 cwd，推理与旧执行恢复前先核验。SessionToolManager 保留协议纪律，内部调用 Sandbox。Session 的 .runtime/bin 与 Agent tools 作为宿主环境注入；独立 xllm 接管校验保存的 PATH、manifest、helper 与凭据环境引用。远端 Session helper 未部署时明确报 Capability；remote_ssh 可独立用于 xllm。

宿主已注册 BuckyOS runtime 时，Session 环境与每次 native / tmux 命令执行前都会续期并读取当前 session token，注入 `BUCKYOS_APPCLIENT_SESSION_TOKEN`，覆盖父进程、Session 配置或单次调用中的旧值。续期失败或 token 为空时阻止执行；工具沿用宿主身份，不需要 owner 私钥。run 的环境核验记录只保存凭据变量名，不保存 token 值。未注册 runtime 的独立开发形态仍可显式传入该环境变量。

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

推进时 xagent 把自身包装成 session 的 `.runtime/bin/agent-session`，`shell` 里的命令用它访问本 Session 与 Agent State：`agent-session ctl activity --touch ws:foo`、`ctl perceive "…"`、`recall <tag>…`、`note <text>`、`sessions [--children] [--active]`、`read-session <sid>`、`artifact head|list`、`create-worksession --objective … [--report final|progress|none] [--wait] [--interactive]`、`wait <sid>`、`post <sid> --msg …`。

## 测试

```bash
cargo test -p libopendan -- --test-threads=1
```

| 文件 | 覆盖 |
|---|---|
| `tests/l1.rs` | 锁（epoch、inode 不变、CLOEXEC、kill -9 后接管）、反向读有界、压缩无空洞、kmsg 规则（文件队列与 kmsg 的 sled 实现）、订阅丢失重建、迁移、巡检、幂等创建、绑定失败、墓碑修复、活动视图 |
| `tests/runner_basic.rs`、`tests/runner_more.rs` | work session 端到端、finished 后拒绝输入、非驱动者 / Busy、stop、事件路由（active / semi / 未订阅丢弃 / 订阅变更按投递顺序生效）、半订阅快照、behavior loop、普通 / fork / independent 切换（同一 Turn 内交接）、`max_turns` 只计已完成 Turn、decide 与 head、activity / perception 输入、tmux runtime |
| `tests/crash.rs` | 子进程在各提交窗口 abort（`LIBOPENDAN_FAULT`，如 `input_batch:after_state_commit`、`finish_run:after_flush`）或执行中被 kill -9 后恢复；版本不支持时阻塞；xllm 接手与拒绝 |
| `tests/input_tasks.rs` | 输入协议 3 与长任务：挂起调用在上下文之外等待 task 并回填同一 run、无 resolver 拒绝接手与 Unknown 回填、等待期间 stop、后台 task 完成合成 Input 事件、run 终态落盘后崩溃仍找回后台 task、工具执行中的 stop、64 条 pending 上限、Single / Batch、`input.media = inline`、重投去重与回复路径、旧 Session 只读、模板失败不消费、用户时区半订阅 |
| `tests/self_improve.rs` | 感知幂等、self_improve 锁、整理游标、防自我回声 |
| `tests/outbound.rs` | 回复随 Turn 提交并沿来路发出、sink 不可达时保留并原样重发（同键同 ObjId）、被拒与路由不一致只记录不重试、没有 sink 时不产生 outbox |
| `tests/fixtures.rs` | 参考实现在 `doc/opendan/protocol/fixtures` 每个场景上满足 `expected.json`；`14_input_bus` 的记录处理、拒绝原因与逐字节渲染 |
| `tests/xagent_lib.rs` | 无队列 work Session 与 `prompt.initial_inputs`、`WAIT_USER_MSG` 的模板语义、`TurnClosed` / `TurnOpen`、behavior 冻结 / 补冻结 / 缺失冻结、三种 Agent State 实现结果一致、Sub Session（父等待汇报、`session:<sid>` 回填、数量与深度、交互式子提问、stop 级联、进度半订阅）、驱动者停止请求、on_context_switch 前的半订阅快照、timer bridge |
| `tests/xagent_cli.rs` | `xagent` 可执行文件（本地 OpenAI 兼容 mock）：`new --no-run` / `run` / `post` / `ctl` / `status` / `list` / `xllm` / `schema` 的退出码与 stdout 单个 JSON、无队列投递拒绝、ui 模板等输入与同一 Turn 续答、经 `shell` 的 `agent-session create-worksession --wait`、xllm 接手后由 xagent 提交结束、第二个驱动者 Busy 与 SIGINT、tmux runtime 归属 Session、`serve` |
| `tests/dv_kmsg.rs` | （`--ignored`）真实 kmsg 服务：幂等建队列 / 订阅、消费与累积 ack、finished 后拒绝并 ack；在 DV Test OOD 上以 root 运行 `cargo test -p libopendan --test dv_kmsg -- --ignored` |

重新生成 fixtures / JSON Schema：`cargo run -p libopendan --example fixtures -- ../doc/opendan/protocol/fixtures`（在 `src/` 下执行）。
