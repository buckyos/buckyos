# mini_agent_demo — echo-bot

最小可运行的 AgentRootFS,只有两个文件:

```
mini_agent_demo/
├── agent.toml                  # Gateway + Session 类骨架
└── behaviors/
    └── ui_default.toml         # 唯一的 behavior
```

完整 schema 见 [../Agent配置改进.md](../Agent配置改进.md)(§4 = `agent.toml`,§5 = `behaviors/*.toml`,§11 = 本 demo 的来源)。

## 心智模型(三层)

| 层 | 谁负责 | 改动频率 | 落盘位置 |
|---|---|---|---|
| Identity + Gateway + Dispatcher + Session 类 | runtime | 极少 | `agent.toml` |
| Behavior(每个就是一段提示词 + 工具白名单 + 异常旁路) | 业务 | 经常 | `behaviors/<name>.toml` |
| Tools / Skills / TaskPlans | 业务 | 经常 | `tools/` `tool_plans/` `skills/`(本 demo 没用) |

**读 `agent.toml` 应该能复述 Agent 对外的形态**,读 `behaviors/` 目录应该能列出 Agent 全部分支。

## 怎么把它接到 opendan 跑起来

1. 把 `mini_agent_demo/` 复制成 buckyos 上的 agent root。本机开发时常见路径:

   ```bash
   cp -r doc/opendan/mini_agent_demo /tmp/echo-bot-root
   ```

2. 当前 `opendan` 不再接受 `--agent-root` / `OPENDAN_AGENT_ROOT` 作为主进程启动入口。Agent root 由 BuckyOS runtime 的 appid / owner 解析:

   ```bash
   get_buckyos_app_data_dir(<appid>, <owner_user_id>)
   ```

   本机开发时应先把 demo 内容同步到对应 app 的数据目录,再用 `service_debug <appid> <owner_user_id>` 启动。

3. 启动后 opendan 会:
   - 解析 `agent.toml`,挂上 `msg_center` channel,装配 `[dispatch]` 规则表
   - 收到 `msg.chat` ⇒ dispatcher 路由到 `session.ui` 类
   - `session_id_strategy = "per_peer"` ⇒ session_id = `ui-<sanitized_peer_did>`
   - 该 session 首次创建 ⇒ 加载 `behaviors/ui_default.toml`,渲染 `[prompt].on_init` 作为 system 提示
   - LLM 输出 `<report>` + `<next_behavior>END</next_behavior>`,runtime 把 `<report>` 内容回发给用户

4. 给它发任意消息,LLM 应该回声同一段文本。

## 接下来想做更复杂的 Agent 时只加不改

| 想做的事 | 加在哪 |
|---|---|
| 监听新的 kevent(比如 task_mgr 完成事件) | `[[channel]]` 加 `type = "kevent"` + `filters`;`[[dispatch.rule]]` 加一条到目标 session class |
| 引入 Work session(自主任务) | `[session.work]` 加一段,配 `loop_mode = "behavior"` + `kind = "work"` + 选一个 `session_id_strategy` |
| 加一个新 behavior(planner / executor / summarizer …) | 在 `behaviors/` 加一个 `.toml`,用 `[meta]` `[prompt]` `[capabilities]` 写好;LLM 用 `<next_behavior>NAME</next_behavior>` 自己切过去 |
| 上下文满了想压缩后继续而非中止 | 目标 behavior 加 `[on_context_limit_reached]\nmode = "compress_then_continue"` |
| 想在接近 context window 时提前自动压缩 | 目标 behavior 加 `[on_llm_message_compress]\nmode = "context_window_ratio"\ntrigger_ratio = 0.80\ntarget_ratio = 0.50` |
| Provider 失败时降级到备用 behavior | `[on_provider_failed]\nmode = "fallback_behavior"\ntarget = "safe_mode"` |
| 让人审某个工具调用 | `[capabilities].approval_required = ["exec_bash"]`(沿用 v0 占位语义) |

**不要做的事**(v0 故意拦住,见 §7.1):

- 在 `agent.toml` 写 `when = "..."` 之类的表达式 ⇒ 不支持
- 在 `behaviors/*.toml` 给 `session_id` 写模板插值 ⇒ 不支持(strategy 是 4 选 1 的枚举)
- 让 LLM 决定 `driver.switch_mode = "fork"` 还是 `"normal"` ⇒ LLM 只挑 `<next_behavior>`,模式归 session driver

复杂状态机请整体跳到 Workflow LLMContext DSL,不要在这份 schema 上叠 `when` / 脚本钩子。

## 文件清单速读

- [agent.toml](./agent.toml) — Identity / Gateway / Dispatcher / Session 类 全部在 50 行内
- [behaviors/ui_default.toml](./behaviors/ui_default.toml) — 一个 behavior 的最小骨架:meta + prompt + capabilities + budget + model

两个文件就足以让 opendan 跑起来。
