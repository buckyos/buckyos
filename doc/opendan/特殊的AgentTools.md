# 特殊的 AgentTool

## 1. 什么算"特殊"

OpenDAN 里的 Agent Tool 在工程上由 `agent_tool` crate 的 `AgentToolManager` 管理。每个工具通过 `CallingConventions`（位标志）声明自己支持哪几种调用入口：

| 位 | 含义 | 提示词里 LLM 看到的形态 |
|---|---|---|
| `BASH`  | 可以作为命令行被 `exec_bash` 调起 | overlay PATH 里的一个 shim 二进制（或真二进制） |
| `ACTION` | 可以作为 Behavior 模式下 `<actions>` 里的一个 XML 标签出现 | `<write_file>`、`<exec_bash>` 这类标签 |
| `LLM`   | 可以作为 provider 原生 tool_call 被调用 | OpenAI/Anthropic tool/function schema |

> 实现：[`agent_tool/src/tool.rs:31`](src/frame/agent_tool/src/tool.rs:31) 定义 `CallingConventions`，各 tool 通过 `fn calling(&self)` 声明。

90% 的 Agent Tool 是同时带 `BASH` 位的——它们在 4 层 PATH overlay（Session > Agent > Runtime > System，见 [`agent_bash.rs`](src/frame/opendan/src/agent_bash.rs)）里有一个对应的可执行文件，LLM 通过 `exec_bash` 像 shell 调用一样把它们用起来。这是默认的、推荐的扩展方式：写一个二进制丢进 overlay，立刻能用。

**"特殊"指的是那些不带 `BASH` 位的工具**：它们没有 shim、不在 PATH 上，永远不会被 `exec_bash` 看到。原因通常只有一个——

> **这些工具依赖 session 的可变状态**（session id、订阅句柄、`Weak<AIAgent>`、消息队列），无状态的 CLI 进程根本拿不到这些上下文。所以它们只能由 Session 层在创建 session 的时候手工注册到当前 session 的 ToolManager 上。

参见 [Agent Actions.md §0.2-0.3](doc/opendan/Agent%20Actions.md)：Action 准入原则的"必须走 session 内存"这条，本质上就是在筛选哪些工具必须"特殊"。

## 2. Function 还是 Action？

特殊工具可以走两种入口：

- **Function**（`CallingConventions::LLM`）：直接挂在 provider 原生 tool_calls 通道上，由 LLM 在思考过程中以 JSON 参数发起调用。
- **Action**（`CallingConventions::ACTION`）：作为 Behavior 模式 `<actions>` 容器里的一个 XML 标签，由 `msg_parser` 在 assistant 回复里解析出来后派发。

原则上两者可互换：所有 function 都能改写成 action，反之亦然。但工程上：

- Behavior 模式下的固化 Action 集合是 **prompt-coupled** 的——LLM 在提示词模板里见过的标签才会输出。所以 Action 集合不能跟着 ToolManager 自动膨胀，必须手工跟提示词同步维护（见 [Agent Actions.md §0.1](doc/opendan/Agent%20Actions.md)）。
- 走 Function 路线只需要在创建 session 时调一次 `register_typed_tool`，不需要改提示词模板；适合 Behavior 之外、走 free-form LLM 推理的场景。

实操上同一个能力会在两个通道各注册一次，看运行场景被哪个入口先命中。

## 3. 当前的特殊工具清单

下面列出所有当前注册时不带 `BASH` 位的工具——它们都是 Session 层在 `AIAgent::create_session` 流程里专门挂上去的（见 [`agent.rs:788`](src/frame/opendan/src/agent.rs:788) 附近）。

### 3.1 事件订阅类

在 [`opendan/src/buildin_tool.rs`](src/frame/opendan/src/buildin_tool.rs) 实现，入口函数 `register_event_subscription_tools`。两者都是 `CallingConventions::LLM`；同名的 XML 标签由 Behavior 模式作为 Action 暴露（见 [Agent Actions.md §1.6-1.7](doc/opendan/Agent%20Actions.md)）。

| 工具 | 用途 | 关键参数 |
|---|---|---|
| `subscribe_event` | 把当前 session 订阅到一个 KEvent 路径模式上；匹配的事件会被批量翻译成自然语言用户唤醒消息送回 session | `pattern`（如 `/task_mgr/42` 或 `/approval/**`）、`message_template`（可选，渲染模板，支持 `{event_id}` `{data}` 及顶层 JSON 字段占位符） |
| `unsubscribe_event` | 移除一个已存在的订阅 | `pattern` |

两者都需要回拿当前 session 才能调 `subscribe_event_with_template` / `unsubscribe_event`，所以工具结构体内部持有 `Weak<AIAgent>` + `source_session_id`——这就是它们不能做成 shim 的根本原因。
> 虽然session的配置文件里有序列化kevent的订阅，但是修改这个配置文件文件只能让订阅在session的下次实例化时生效，不会立刻生效

### 3.2 Worksession 控制类

在 [`opendan/src/worksession_tools.rs`](src/frame/opendan/src/worksession_tools.rs) 实现，入口函数 `register_worksession_tools`。三者都是 `CallingConventions::LLM`。

| 工具 | 用途 | 关键参数 |
|---|---|---|
| `create_worksession` | 全参数版本：直接创建一个新的 Work session，绑 workspace，并绑定 TaskMgr Task；默认起 worker 执行首轮，也可只创建不执行 | `task_id`（可选，指定已有 Task 时可从 Task data 回填 `title` / `objective` / `workspace_id`）、`title`、`objective`、`workspace_id`（可选）、`behavior`（可选）、`reason_message`（可选，列表，写进 readme 留痕）、`auto_start`（可选，默认 `true`） |
| `forward_msg` | 把本轮 origin user message（或 LLM 显式指定的文本）转发到另一个 worksession 的 pending 输入队列 | `target_worksession_id`、`message`（可选，省略时从 session 的 `current_origin_user_message()` 自动取） |
| `try_create_worksession` | UI session 专用：fork 出一个短任务子上下文，让子 LLM 决定要不要建——它内部会调用 `create_worksession`；最终结果作为 JSON 透传给父 LLM | `reason`（自由文本说明） |

三者都需要持有 `Weak<AIAgent>` 才能调 `create_work_session` / `forward_message` / `fork_and_run`，且 `source_session_id` 决定调用者身份和审计字段。
> try_create_worksession 和 forward_msg 是UISession的核心路由Function , create_worksession 只在try_create_worksession启动的旁路llm_context中注册

`create_worksession` 的 Task 绑定规则：

- 调用时传入 `task_id`：直接绑定该 Task，并按顺序从 `data.agent_delegate.title` / `data.title`、`data.agent_delegate.purpose` / `data.agent_delegate.input.text` / `data.objective`、`data.agent_delegate.route.workspace_id` / `data.agent_delegate.execution.workspace_id` 回填缺省字段。
- 调用时不传 `task_id`：自动创建一个 `agent.delegate` Task，`data.agent_delegate.execution.session_id` 指向新 WorkSession。
- WorkSession 后续状态变化会写回 Task 的 `status` 和 `data.agent_delegate.execution`；TaskMgr 将 Task 改为 `Paused` / `Canceled` 时，AgentRuntime 轮询到后会中断对应 WorkSession。

### 3.3 关于 Action 集合

完整的 v2 Action 7 件集合（`exec_bash` / `read` / `write_file` / `edit_file` / `sendmsg` / `subscribe_event` / `unsubscribe_event`）以及 actions 外的 `<report>` LastState 标签已经在 [Agent Actions.md](doc/opendan/Agent%20Actions.md) 单独定稿。本文不重复 Action 部分；这里只列出"被 Session 层手工注册、没有 CLI 化身"的工具——这才是把"特殊"四个字落到代码上的判定标准。

## 4. 增加新特殊工具的检查表

新加一个特殊工具前，先按 [Agent Actions.md §0.3 准入原则](doc/opendan/Agent%20Actions.md) 自查：

1. **能不能做成 shim？** 4 层 PATH overlay 已经能覆盖绝大多数"调一个能力"的需求。能做 shim 就别开特殊工具。
2. **是否真的需要 session 内存？** 如果工具内部不需要 `Weak<AIAgent>` / `source_session_id` / 任何挂在 session 上的句柄，它就不应该"特殊"——把它做成普通本地工具，由 `build_default_tool_manager` 统一注册即可。
3. **要不要进 `<actions>` 提示词？** 如果决定走 Action 通道，必须同步：注册时加上 `CallingConventions::ACTION` 位、给 Behavior 提示词模板加上 XML 示例、在 [Agent Actions.md §1](doc/opendan/Agent%20Actions.md) 的固化集合表里登记。
4. **注册时机：** 在 `AIAgent::create_session` 里跟 `register_event_subscription_tools` / `register_worksession_tools` 类比，新增一个 `register_xxx_tools(&tools, Arc::downgrade(&self), &session_id)` 调用。不要塞进 `build_default_tool_manager`——那里只放与 session 无关的本地工具。



## 5. 固定的Token消耗

UI Session的Function List的Token消耗:

Work Session用来解释XML Behavior以及说明Action使用方式的提示词所消耗的Token:
