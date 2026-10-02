# libopendan

以某个 Agent 的身份推进一个 Agent Session 的 Rust 参考实现（[实现计划](<../../../doc/opendan/Agent Session SDK 实现计划.md>)）。协议（目录结构、提交顺序、锁语义、输入格式、xllm run 目录）见 [`doc/opendan/protocol/`](../../../doc/opendan/protocol/README.md)；本 crate 里 Runner 的结构不是协议。

## 模块

| 模块 | 内容 |
|---|---|
| `protocol` | 全部磁盘 / 队列结构（serde + JSON Schema 导出） |
| `fsutil`、`lock` | 原子替换、批量追加、反向读、不覆盖发布；长期持有的 flock 锁 |
| `session` | `SessionDir`（读取、创建发布）、`Session`（持锁提交 state.json）、worklog、`runs/`（xllm RunStore 封装） |
| `channel` | kmsg 输入（`KmsgInput`）、开发用文件队列 `DirMsgQueue`（kmsg 语义）、kevent 唤醒 |
| `state` | `AgentStateClient` 与文件实现：登记表、活动视图、感知、认知门面、产物列表、Agent 级锁 |
| `runtime` | 复用 agent_tool::runtime；仅保留 Session bin/helper、绑定与环境核验 |
| `runner` | `drive`：恢复、输入批次提交（开启或加入逻辑 Turn）、Outcome 处理与 Turn 关闭、观察边界、process 切换、压缩、Round 统计 |
| `api` | `create_session` / `read_session` / `post_input` / `create_self_improve_session` |

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

持久格式为协议版本 3（session_state/session_config/binding 均为 /3，xllm RunRecord.version = 3；summary 与机械渲染保持 /2，快照版本 3）。旧格式的 session / run 不迁移、不按旧字段读取，加载时返回 RecoveryBlocked。

RunnerDeps.runtime 使用 agent_tool::runtime::AgentRuntime；.llm_context.runtime 是构造配置，session_config.runtime.requirement 是绑定要求。binding 保存实际 target 和执行 cwd，推理与旧执行恢复前先核验。SessionToolManager 保留协议纪律，内部调用 Sandbox。Session 的 .runtime/bin 与 Agent tools 作为宿主环境注入；独立 xllm 接管校验保存的 PATH、manifest、helper 与凭据环境引用。远端 Session helper 未部署时明确报 Capability；remote_ssh 可独立用于 xllm。

## 开发 CLI

```bash
export OPENDAN_AGENT_ROOT=/tmp/x/agent_root OPENDAN_AGENT_DID=did:bns:jarvis.alice LIBOPENDAN_QUEUE_DIR=/tmp/x/kmsg LIBOPENDAN_WHO=app:app2@alice
cargo run -p libopendan --example session -- create --parent /tmp/x/app --objective "…" --llm-context '{"provider":{"type":"openai","base_url":"http://127.0.0.1:8000/v1","api_key_env":"KEY"},"model":"m","tools":{"enabled":true}}'
cargo run -p libopendan --example session -- run /tmp/x/app/<sid> [--until finished|idle|outcomes:<n>]
cargo run -p libopendan --example session -- read <sid> --worklog 20 --report
cargo run -p libopendan --example session -- decide <sid> accept
cargo run -p libopendan --example session -- active | holder <dir> | post <sid> --text … | schema <dir>
```

`run` 把自身包装成 session 的 `.runtime/bin/agent-session`，`exec` 里的命令可以用 `agent-session activity --touch ws:foo` / `agent-session perceive "…"` 向本 session 投递。

## 测试

```bash
cargo test -p libopendan -- --test-threads=1
```

| 文件 | 覆盖 |
|---|---|
| `tests/l1.rs` | 锁（epoch、inode 不变、CLOEXEC、kill -9 后接管）、反向读有界、压缩无空洞、kmsg 规则（文件队列与 kmsg 的 sled 实现）、订阅丢失重建、迁移、巡检、幂等创建、绑定失败、墓碑修复、活动视图 |
| `tests/runner_basic.rs`、`tests/runner_more.rs` | work session 端到端、finished 后拒绝输入、非驱动者 / Busy、stop、变化注入与合并、半订阅、behavior loop、普通 / fork / independent 切换（同一 Turn 内交接）、`max_turns` 只计已完成 Turn、decide 与 head、activity / perception 输入、tmux runtime |
| `tests/crash.rs` | 子进程在各提交窗口 abort（`LIBOPENDAN_FAULT`，如 `input_batch:after_state_commit`、`finish_run:after_flush`）或执行中被 kill -9 后恢复；版本不支持时阻塞；xllm 接手与拒绝 |
| `tests/self_improve.rs` | 感知幂等、self_improve 锁、整理游标、防自我回声 |
| `tests/fixtures.rs` | 参考实现在 `doc/opendan/protocol/fixtures` 每个场景上满足 `expected.json` |
| `tests/dv_kmsg.rs` | （`--ignored`）真实 kmsg 服务：幂等建队列 / 订阅、消费与累积 ack、finished 后拒绝并 ack；在 DV Test OOD 上以 root 运行 `cargo test -p libopendan --test dv_kmsg -- --ignored` |

重新生成 fixtures / JSON Schema：`cargo run -p libopendan --example fixtures -- ../doc/opendan/protocol/fixtures`（在 `src/` 下执行）。
