# OpenDAN 意图分析（try_analyze_user_intent）TODO

日期：2026-10-11

状态：v0.2.1，I1–I7 已实施（2026-10-11，记录见 §11）；I8 的 DV 三种对话待做（§11.5）。本文把 UI Session 的 `task_route`（原 `try_create_worksession`）改造成架构文档 §5 的意图分析入口 `try_analyze_user_intent`，并定义它反问用户时的完整消息流（Agent 侧 + Loader + MessageHub）。

修订记录：

- 2026-10-11 v0.1 初稿（`create_sub_context` + 子 context 等待 `WAIT_USER_MSG`）。
- 2026-10-11 v0.2 按与用户的讨论重写：改用 `fork` 以复用提示词前缀缓存；反问改成 CLI `ask-user` + PendingTool `userinput:`；新增问答消息流（占位锚点 + `thread` 问答 + `agent_ask`）与 MessageHub 输入态；用户决定见 §8.1。
- 2026-10-11 v0.2.1：behavior 可以声明 `[tool]` 把自己暴露成一个具名函数（§3.13）。UI 看到的是 `try_analyze_user_intent(request)`，不再暴露通用的 `call_behavior`。
- 2026-10-11 实施：I1–I7 完成，§11 是实施记录（落点、与本文不同的决定、验证、DV 状态、遗留）。
- 2026-10-25：§2 源码基线的 runtime kind 一行注明 tmux runtime 已全平台移除。

依据：

- [OpenDAN Agent Session 架构设计](<../doc/opendan/OpenDAN Agent Session架构设计.md>)：§3.1、§4.2—§4.4、§5（全部）、§6.1—§6.2、§11.8、§14.1 “Intent Analysis Result”、§14.5、§14.8、§15.2 阶段四、§15.3、§17.2 前两行。
- [xAgent](../doc/opendan/xAgent.md) §3.3—§3.7（子 context、fork、`call_behavior`、PendingTool）、§4.11、§5.3、§11.5 第 1 条。
- [Context 调度 TODO](llm-context-switch-support-todo.md) §7—§9；[long-tool TODO](llm-context-long-tool-todo.md)（PendingTool 与 resolver 契约）。
- [OpenDAN Agent Workspace 设计](<../doc/opendan/OpenDAN Agent Workspace 设计.md>) §6.2—§6.3、§7；[Agent Workspace Protocol](<../doc/opendan/protocol/Agent Workspace Protocol.md>)。
- [Goal Session TODO](opendan-goal-session-todo.md)（`agent-session goal create` 已可用）。
- MessageHub：[CYFS 标准对象 §16 MsgObject v2]（`relates_to`、`thread`、`edit`）、[Message Center](<../doc/message_hub/Message Center.md>)、[MessageHub UI 数据模型](../src/frame/desktop/src/app/messagehub/UI_DATAMODEL.md)、[Agent Chat 占位 TODO](messagehub-agent-chat-task-todo.md)。
- [Agent 通知人 TODO](opendan-agent-notify-todo.md)：占位，本文不实现任何“Agent 主动通知人”的投递。

读者：实施本文的 code agent。路径相对 `buckyos/`。行号是 2026-10-11 的源码，实施前先核对。

---

## 0. 结论先行

| 事项 | 决定 | 依据 / 理由 |
|---|---|---|
| 入口形态 | UI 的 LLM 看到一个具名函数 `try_analyze_user_intent(request)`。它由子 context behavior `analyze_intent` 的 `[tool]` 声明生成（§3.13），内部走 `call_behavior` 的子 context 协议；`call_behavior` 本身不再进 UI 的工具列表。取代 `task_route` | 用户决定：LLM 可见的函数要有可配置性；xAgent §3.6 的协议不变，只是从“LLM 可见工具”降为宿主内部协议 |
| 进入模式 | `entry.mode = fork`：保留 UI 的 system 与完整历史，只追加一条 user message（`on_context_switch` 模板）说明可用 CLI 并引导分析。目的是**复用提示词前缀缓存** | 用户决定；fork 语义见 `src/frame/llm_context/src/context_derive.rs:188-202` |
| 工具 | 分析用到的全部是层 ② CLI（`agent-session …`），本来就在 UI Session 的 PATH 上，经 `shell` 调用；分析 context 的原生工具集与 UI 相同（fork 继承） | fork 禁止换工具集 / system（`protocol/config.rs:616-626`），正好不需要 |
| 模型 | fork 不能换模型；分析用 UI 的 `llm.chat`（本来就是快速分析用） | 用户决定 |
| 反问用户 | 新 CLI `agent-session ask-user`，打印 `Pending{task_id: "userinput:<call_id>"}`，run 挂起在这次调用上；Session 把下一条用户消息作为这次调用的**工具结果**回填。函数“只有 Session 拿到 user message 才返回” | 用户决定；复用 `create-worksession --wait` 的 PendingTool 路径（`bin/xagent.rs:1064-1075`） |
| 几条消息算答案 | 一条。等待期间到达的第一条 msg 回填，其余留在队列 | 用户决定（符合传统习惯；UI 配合后可再改） |
| Turn 语义 | 问答都在 UI 的**同一个 Turn** 内，Turn 不因提问关闭；主 context 自己的 `WAIT_USER_MSG` 与 D2 规则不动 | 用户决定 |
| 问答消息流 | 占位 P 是锚点；问题 Q 以 `relates_to{thread → P}` 发出并带 `agent_ask` meta；用户的回答 A 由 MessageHub 自动发成 `thread → P`；Turn 结束 P 被 edit 成结果 R；Q/A 折叠在 R 下。用户最终看到 `usermsg → R（▸ N 条问答）` | 用户决定（“关键范式的不妥协实现”） |
| MessageHub 输入态 | 会话最后一条消息是 Agent 的 `agent_ask` 时，输入框**只能**回答这个问题：文本问题 → 锁定为对 P 的回复；选项问题 → 输入框变成选项。原则：按逻辑调整当前 Session 的 input style 与 target | 用户决定 |
| 分析的产物 | 四种有效结局 `work_created / goal_created / direct / declined`；两种分析状态 `needs_confirmation / unsupported`。缺信息在子 context 内 `ask-user`，不作为结局返回 | 架构 §5.1、§5.5 |
| 自动创建 | 条件齐备就在子 context 内直接创建；需要用户拍板的选择作为澄清问题问，问完仍自动创建 | 架构 §5.5；按“先能力后安全” |
| Work 的形态 | **plan_do**（现有 `plan` → `do`）与 **simple**（新 `work_simple`，function_call 循环），都是 `class = work`，由分析按规模选择，经 `create-worksession --behavior` 指定 | 用户理解第 1 点 |
| Goal | 复用 `agent-session goal create`；分析负责问清 §5.4 的字段 | Goal Session TODO §3.7 |
| 计划任务 | 不实现；识别出“未来某时执行”返回 `unsupported` | 用户理解第 1 点 |
| Runtime 管理 | Agent State 新增 Runtime 登记 `state/runtimes/`，Loader 幂等补建 `container` 与 `host`；分析经 `agent-session runtime list` 看到 | 用户理解第 2 点；Workspace 设计 §7 |
| `host` 的可执行性 | 只做登记、列举与选择，`availability = runtime_unavailable`；容器 → 宿主机执行器另议 | agent_tool 没有执行器（`agent_tool/src/runtime/mod.rs:80-100`） |
| 报告 | 子 context 以 `report({report, result, is_end: true})` 结束；UI 收到 `{status, behavior, submission: {report, result}}` | H4 已实施：`runner/live.rs:1238-1283` |
| llm_context / agent_tool | 预期不改：fork 与 PendingTool 原语都已有。若实施中发现必须改，按约定单列 TODO 先行 | [`feedback-llm-context-first`] |
| 协议版本 | 只增字段，不升 `session_config` / `session_state` 主版本；改了结构就重生成 fixtures | `doc/opendan/protocol/README.md` |

---

## 1. 范围

### 1.1 做

- **libopendan**：`userinput:` 等待（ask-user）、问题的 Turn 中途出站（`thread → P` + `agent_ask`）、回答回填为工具结果、确保占位先发、崩溃 / 卸载 / stop 的处理；Runtime 登记；`create-worksession --runtime` 查登记。
- **xagent CLI**：`agent-session ask-user`、`agent-session runtime …`。
- **Loader（opendan）**：出站支持 Turn 中途消息；Runtime 默认记录补建；首页展示 Runtime；端到端测试。
- **MessageHub（desktop）**：`agent_ask` 输入态（锁定回复 / 选项输入）、Agent 锚点下 `thread` 问答折叠、mock 与 e2e。
- **Jarvis 包**：`analyze_intent`（fork）、`work_simple`；`chat_route` / `groupchat_route` 改调并按新结果协议转述；删 `task_route`。
- **文档**：架构文档补“意图分析的实现形态”一节；协议、UI 数据模型、README 回写。

### 1.2 不做

- 计划任务；宿主机执行器；Agent 主动通知人；Work 执行期的 Task 修订。
- 顶层 UI context 自己的 `WAIT_USER_MSG` 改成 ask-user（以后另议）。
- Telegram 等 tunnel 把 `thread` 映射成平台 reply；不可编辑频道上的问答只是普通消息。
- msg-center 未读数把占位 / edit / Q / A 各算一条的问题（记为已知）。
- 分析过程的写权限隔离：只在提示词里约束“分析不改外部状态”（§8.3）。
- 用户迟迟不回答的超时：不加，UI Session 本来就长期等待。

---

## 2. 当前基础与缺口

### 2.1 Agent 侧（libopendan / xagent / Loader）

| 入口 | 已有基础 | 缺口 |
|---|---|---|
| `src/frame/llm_context/src/context_derive.rs:175-202` `ForkOptions` / `fork_snapshot` | 父 system + 分叉点前完整历史；由工具触发时分叉点在触发批次之前；request 保持 `behavior_name` / model / output 使历史渲染一致；`tool_policy` 可换但“可能失去前缀缓存”；`budget` 可独立 | 够用，不改 |
| `src/frame/lib_opendan/src/protocol/config.rs:616-626` `BehaviorEntry::validate` | fork 目标禁止 `prompt.system` 与 `llm_context` 覆盖、禁止 `inherit` | 正是需要的约束；确认 fork 目标允许自带 `prompt.on_context_switch` 与 `[budget]`（`runner/live.rs:227-310` `fork_run_context` 取 `parent_record` 的 config，预算覆盖点要核对） |
| `src/frame/lib_opendan/src/runner/tools.rs:208-320` `CallBehaviorTool`、`runner/live.rs:101`、`protocol/config.rs:421, 650` | 一个通用工具 `call_behavior({behavior, task})`，目标 = 冻结 closure 里 `mode.is_sub_context()` 的 behavior；需在 `tool_whitelist` 显式列 `call_behavior` | 泛化为 `BehaviorTool`：按 behavior 的 `[tool]` 声明生成具名工具（§3.13）；`BehaviorEntry` 冻结 `[tool]` |
| `src/frame/lib_opendan/src/runner/assembler.rs:395-405` | `<sub_task>` 包装固定写 “do not ask the user” | 对允许提问的 Session 改文案（§3.7 第 9 条） |
| `src/frame/lib_opendan/src/bin/xagent.rs:1064-1075` `pending_on_session` | CLI 打印 `AgentToolStatus::Pending` + `task_id`，`shell` 原样转发为调用结果 | 新增 `ask-user` 用同一形状，前缀 `userinput:` |
| `src/frame/lib_opendan/src/runner/tools.rs:160-200, 206, 318-350` | `SessionToolManager` 登记 Pending 的 task；`SUB_CONTEXT_TASK_PREFIX = "subctx:"`；`pending_sub_call` 识别挂起在子 context 调用上的快照 | 新增 `USER_INPUT_TASK_PREFIX = "userinput:"` 与对应识别 |
| `src/frame/lib_opendan/src/runner/children.rs:366-405` `SessionTaskResolver` | `session:<sid>` 由 Session 的 resolver 解析；`can_resolve` 让 xllm 拒绝接手 | `userinput:` 不走 resolver 轮询，由 drive 循环从输入队列回填（§3.7） |
| `src/frame/lib_opendan/src/runner/drive.rs:1565-1572` | `hold_inputs`：子 context 进行中 msg / event 全留给调用方 | 等 `userinput:` 时，msg 候选取一条回填，event / `_child` / 观察仍留 |
| `src/frame/lib_opendan/src/runner/drive.rs:1499` | 等待工具时 `WaitingKind::Tool` | `userinput:` 写 `WaitingKind::Input`，refs 带 `userinput:<call_id>` |
| `src/frame/lib_opendan/src/runner/outcome.rs:760-800`、`runner/outbound.rs:307-395` | 回复只在关 Turn 时 `queue_reply(TurnReply)`；占位 `OutboxPurpose::Placeholder`，最终 `FinalEdit` 跟随占位拿到的 `msg_id` | 新增 Turn 中途的 `OutboxPurpose::Question`：`thread → P`，等 P 的 `msg_id` 再发（与 FinalEdit 同样的“跟随”逻辑） |
| `src/frame/lib_opendan/src/protocol/state.rs:288-330` `OutboxPurpose` / `OutboxEntry` | `Reply / Placeholder / FinalEdit`；entry 带 `msg_id` | 加 `Question` |
| `src/frame/lib_opendan/src/protocol/input.rs:547-575` `PostedInput::msg` | 输入里带完整 `MsgObject`（含 `relates_to`、`content.machine`） | Session 自己能算 `in_reply_to_question`（`relates_to.target == 占位 msg_id`）与选项答案，不需要 Loader 改入站 |
| `src/frame/opendan/src/ui.rs:171-300` | `OutboundTexts`（`accepted / stopped / finished`）、`placeholder()` 查 `edit_capability` | 中途消息走 `OutboundSink`；问题文本是模型写的，不需要新文案；`placeholder()` 要能被 ask-user **立即**触发 |
| `src/frame/lib_opendan/src/runner/turn_task.rs:172-175`、`src/frame/opendan/src/tasks.rs:288-293` | `WaitingKind::Input` → TaskMgr `Other("input")` | 够用；MessageHub 任务区据此显示“等待” |
| `src/frame/lib_opendan/src/runner/outcome.rs:260`、`runner/live.rs:1258-1266`、`tests/runner_more.rs:1006-1040` | 子 context 的 `WAIT_USER_MSG` 一律交回调用方 | **保持不变**（这是哨兵路径）；`ask-user` 是另一条路径 |
| `src/frame/lib_opendan/src/template.rs:107-114` | `ui`：`Turns::Unbounded`、`WaitPolicy::Allowed`、`InputChannel::Queue` | `ask-user` 的允许条件由此推出（§3.7 第 1 条） |
| `src/frame/lib_opendan/src/api.rs:471-501`、`bin/xagent.rs:1077-1150` | `create-worksession` 已有 `--class / --behavior / --workspace / --runtime / --context / --report` | `--runtime` 要查登记；`work_simple` 不存在 |
| `src/frame/lib_opendan/src/bin/xagent.rs:1160-1260` `workspace` | 完整 CLI；位置 `runtime_id` 只认 `local`（`protocol/workspace.rs:9`，`runtime/mod.rs:63-90`） | 接 Runtime 登记 |
| `src/frame/agent_tool/src/runtime/mod.rs:80-100` | kind 只有 `native / tmux / remote_ssh`（tmux 已于 2026-10-25 移除）；容器 / 宿主机 kind 报 “planned but not implemented” | 没有登记，没有执行器 |
| `src/frame/lib_opendan/src/state/{mod,krpc,workspaces}.rs` | `workspaces()` / `goals()` 文件版 + kRPC 版样板 | 没有 `runtimes()` |
| `src/frame/opendan/src/home.rs`、`web/` | 首页展示 Workspace | 不展示 Runtime |
| `src/frame/lib_opendan/src/bin/xagent.rs:1403-1470`、`:1300` | `goal create …`；`goal_caller` 只按 principal / session 区分 | 够用；核对 UI Session 可用 `goal list` |
| `src/frame/lib_opendan/src/runner/reports.rs` | `report` 的 `result` 任意 JSON，`submission` 原样交回 | 够用 |
| `src/apps/jarvis_agent/agent/behaviors/{chat_route,groupchat_route,task_route,plan,do}.toml`、`agent.toml` | `call_behavior(task_route)` + 三种文本结局 | 全部按 §3 改 |

### 2.2 MessageHub 侧（`src/frame/desktop/src/app/messagehub/`）

| 入口 | 已有基础 | 缺口 |
|---|---|---|
| `protocol/msgobj.ts:65-90, 150-190` | `TopicThread.reply_to`（语义线索）；`relates_to.rel ∈ edit/redact/reaction/thread`；`agent_task` 只在锚点上读 | 加 `agent_ask` 类型；回答消息的 `content.machine` |
| `conversation/history/relations.ts:64-170` `foldMessageRelations` | `edit / redact / reaction` 折叠进目标；`thread` 消息保留自己一行并引用目标 | 目标是带 `agent_task` 的 Agent 锚点时，`thread` 消息折叠到锚点下方（§3.9） |
| `conversation/input/ConversationComposer.tsx:50-58, 353, 561` | `relation = {kind: reply|edit, message}`，发送写 `relates_to{thread,target}`，“回复 X”横幅 | 新增由会话状态驱动的**锁定回复态**与**选项输入态** |
| `conversation/tasks/MessageTask.tsx`、`tasks/taskView.ts` | 任务区按 TaskMgr 状态显示，Waiting 带 wait reason | 等输入时文案“等待你的回答”（wait reason `input`） |
| `UI_DATAMODEL.md`、`mock/data.ts`、`i18n/messagehub.ts` | 数据模型文档、mock 种子、文案 | 补 `agent_ask`、折叠规则、文案、mock 场景 |
| `tests/e2e/pages/messagehub-agent-task.spec.ts`、`tests/datamodel/messagehub-agent-task.test.ts`、`tests/e2e/real/messagehub-agent-task.real.spec.ts` | 占位 / edit / 任务区的 mock e2e 与真实 zone 用例 | 加问答流用例 |

---

## 3. 模型与契约

### 3.1 分析结果（`report.result`）

子 context 必须以一次 `report({report: <文字报告>, result: <下面的 JSON>, is_end: true})` 结束。字段名是约定，命名可调，语义不变：

```json
{
  "outcome": "work_created | goal_created | direct | declined | needs_confirmation | unsupported",
  "request": "用户需求的完整复述（Work / Goal 看不到这段对话，这是唯一依据）",
  "clarifications": [ {"question": "…", "answer": "…"} ],
  "assumptions": ["分析自己做的、没有问用户的假设"],
  "evidence": ["探索现实状态得到的关键事实及来源（路径 / 命令 / 对象）"],
  "work": {
    "session_id": "…", "shape": "plan_do | simple", "objective": "…",
    "workspace_id": "… | null", "runtime_id": "container | host | …", "report": "final"
  },
  "goal": { "goal_id": "…", "objective": "…", "lifecycle": "achieve | maintain" },
  "proposal": { "summary": "…", "options": ["…"], "why_confirm": "…" },
  "gaps": ["needs_confirmation / declined / unsupported 时：还缺什么、为什么不能创建"],
  "reason": "declined / unsupported 的一句话原因"
}
```

- `work` 只在 `work_created` 时出现，`goal` 只在 `goal_created` 时出现，`proposal` 只在 `needs_confirmation` 时出现。
- `clarifications` 必须完整：UI 主 context 看不到子 context 的对话，用户在澄清中说的每一句约束只能从这里回到 UI（架构 §4.3）。
- `report` 文字面向用户：创建了什么、它会做什么、用了哪些假设、哪些问题用户已答。不重复探索过程（架构 §5.2）。
- `direct` 表示“这是个对话就能答的问题”，UI 自己回答；分析可以在 `report` 里给出已查到的事实。

### 3.2 流程

```text
chat_route 收到用户消息
  ├─ 显然的闲聊 / 简单问答 → 直接回答（现有规则不变）
  └─ 其它 → try_analyze_user_intent({request: "<用户请求完整复述>"})   ← behavior analyze_intent 的 [tool]
        analyze_intent（fork：UI 的 system + 完整历史 + 一条引导 user message）
          1. 读懂请求；列出要判定的事项（Task 还是 Goal、规模、Workspace、Runtime、缺口）
          2. 先查再问：workspace list / runtime list / sessions / goal list / 目录文件（§3.3）
          3. 缺口按影响排序；需要用户的 → shell(agent-session ask-user …)（§3.4、§3.7）→ 拿到回答回到 2
          4. 收敛：
             - 有界任务 → 选形态（§3.5）→ 选/建 Workspace（§3.10）→ create-worksession → work_created
             - 持续目标 → goal create → goal_created
             - 问答 → direct；做不了 / 不该做 → declined；计划任务 → unsupported
             - 方案成形但必须用户拍板且不适合用一个问题问清 → needs_confirmation
          5. report(…, is_end: true)
        UI 收到 submission → 按 outcome 转述（§3.11）
```

分析探索现实状态是必须的（架构 §5.2）：“新项目还是在已有项目上继续”“目录里有什么”“有没有同目标的 Work 在跑或刚结束”都要看过再问。

### 3.3 分析可用的命令（层 ② CLI，经 `shell`）

| 命令 | 用途 |
|---|---|
| `agent-session ask-user "<问题>" [--option <id>=<文字>]... [--allow-text]`（新） | 问用户；有限选项时给 `--option`，输入框会变成选项（§3.8） |
| `agent-session workspace list [--active] [--text …]` / `get` / `check` / `create` / `import` | 已登记 Workspace；创建 / 导入（`--key` 幂等） |
| `agent-session runtime list` / `get <id>`（新） | 可用 Runtime、可用性、建议根目录（§3.12） |
| `agent-session sessions [--children] [--active]`、`read-session <sid> [--report]` | 在跑 / 刚结束的 Work；失败 Work 的报告作为新 Work 的输入 |
| `agent-session goal list` / `show <id>` | 已有 Goal；挂到已有 Goal 用 `goal post` |
| `agent-session create-worksession --objective … --msg … [--behavior work_simple] --workspace <id>|none [--runtime <id>] --context recent:6 --report final` | 创建 Work（不加 `--wait`） |
| `agent-session goal create …` | 创建 Goal |
| `agent-session recall <tag>…` | Memory 线索 |
| 普通 shell（只读用途） | 看目录、文件、git 状态 |

### 3.4 反问用户（“grill me” 式澄清）

写进 `analyze_intent` 的引导 message：

1. **先查后问**：能从 Workspace、文件、历史 Session、Goal 列表、Memory 得到答案的不问。
2. **一次一个问题**（或一组必须同时回答的紧密问题），按对结果的影响排序。
3. **带默认选项**：每个问题给出分析倾向的答案并说明依据；答案是有限集合时用 `--option`，把倾向的放第一个。
4. **复述已定事项**：提问前一两句说明已确认了什么。
5. **停止条件**：没有会改变“建什么 / 形态 / Workspace / 验收条件”的分歧时停止；超过 5 轮仍不收敛，带明确假设推进（写进 `assumptions`）或返回 `needs_confirmation`。
6. **提问格式**：直接写给用户的自然语言，不带内部字段名。
7. **回答不是回答**（`in_reply_to_question = false`、答非所问、明显换话题）：重新判断；用户明显放弃原请求时返回 `declined`（`reason` 写明），不要替用户把原请求建下去。

Task 就绪性（架构 §5.3）在创建 Work 前逐项核对：目标、范围与非目标、可验证结束条件、必要输入、关键依赖、Runtime、Workspace、允许的动作。Goal 就绪性（架构 §5.4）：追求什么状态、作用于什么对象、服务谁、如何判断进展、边界。

### 3.5 任务规模判定与 Work 形态

| 形态 | 适用 | 创建方式 |
|---|---|---|
| **simple**（新 `work_simple`，function_call 循环） | 单一明确动作、无需分步计划、预计十几次工具调用内完成、不需要跨文件改动与独立验证流程 | `create-worksession --behavior work_simple …` |
| **plan_do**（现有 `plan` → `do`） | 其它：多步、要规划、要验证、多产物、在 Workspace 里持续改 | `create-worksession …`（缺省 behavior 来自 `[session.work].default_behavior = plan`） |

两种都是 Work：有界、Final Report、失败明确（架构 §6）。开放等待、盯住 Issue 直到关闭之类不建 Work（架构 §6.2）。

### 3.6 `analyze_intent` 的 fork 形态与缓存

behavior 文件没有 system、没有模型 / 工具覆盖：

```toml
[meta]
name = "analyze_intent"
objective = "Analyze the user's intent in a branch of the UI context and create the work or goal it needs."

[entry]
mode = "fork"

# LLM 看到的函数（§3.13）
[tool]
name = "try_analyze_user_intent"
description = "Hand a request that may need a work session or a long-lived goal to intent analysis. It explores the environment, asks the user what it must, creates what is needed and returns a report. Call it instead of answering when the request likely takes multiple steps, needs durable workspace state, should continue independently, or asks for something to be watched over time."
parameters = { type = "object", additionalProperties = false, required = ["request"], properties = { request = { type = "string", description = "The user's request, restated completely; the analysis does not see this turn's message." } } }
task = "{{args.request}}"

[prompt]
on_context_switch = """
<intent_analysis>
You are now in a branch of this conversation … (角色、可用 CLI 清单、§3.2 的步骤、§3.4 的提问规则、
§3.5 的形态判定、§3.10 的 Workspace / Runtime 选择、§3.1 的 result 格式、禁止事项、
“do not call try_analyze_user_intent again from here”)
Task: {{sub_task}}
</intent_analysis>
"""

[budget]
max_tool_iterations = 40
max_consecutive_errors = 4
```

缓存成立的条件（实施时核对，不是新功能）：

- 子 run 的第一个请求 = 父上一次请求的前缀（system + 分叉点前历史，字节一致）+ 一条新 user message。分叉点在触发 `call_behavior` 的批次之前（`context_derive.rs:191-196`）。
- 子返回后父的下一个请求 = 同一前缀 + `call_behavior` 调用 + 工具结果，父也继续命中。
- 同模型、同工具定义（`ForkOptions.tool_policy` 不设）。工具列表（含 `try_analyze_user_intent` 本身）是前缀的一部分，按 Session 冻结一次就不再变；fork 子 context 因此也看得到这个函数，靠引导文字约束不再调用，嵌套受 `max_process_depth` 限制。
- 验证：子 run 与父 run 的 `usage.cache_read_input_tokens`（`llm_context/src/context_loop.rs:2423`）在支持前缀缓存的 provider 上应接近前缀长度。llm_context 目前不给 Anthropic 打 `cache_control` 断点，AICC 只是透传（`aicc/src/protocol/derived_responses.rs:440-464`）；这是独立问题，记在 §8.3，不在本文修。

子 context 的工具输出只在子 run 里；返回后 UI 重建 `<session_history>` 时排除已返回子 context 的 transcript（xAgent §3.5），UI 历史里只有一次 `call_behavior` 与它的结果。

### 3.7 `ask-user` 与 `userinput:` 等待（libopendan）

**CLI**：`agent-session ask-user "<问题>" [--option <id>=<文字>]... [--allow-text]`，在 Session 内运行（`$OPENDAN_SESSION_ID`、`$XLLM_CALL_ID`）。打印：

```json
{ "tool": "ask-user", "status": "pending", "task_id": "userinput:<call_id>",
  "details": { "question": "…", "input": { "kind": "text | choice", "options": [{"id": "a", "label": "…"}], "allow_text": false } } }
```

`shell` 把它转发为自己的结果，run 以 PendingTool 挂起（现有机制）。

**Session 的处理**：

1. **允许条件**：Session 有输入队列（`InputChannel::Queue`）且 `policy.wait_user_msg == Allowed`（`ui` 类）。其它 Session 里 `ask-user` 直接返回错误观察 “this session cannot ask the user”（Work 无队列）；不新增模板字段。
2. **登记等待**：看到 `userinput:` 前缀的 Pending → `run_state = waiting`、`waiting_for = {kind: Input, refs: ["userinput:<call_id>"]}`，Turn 不关（`turn_end = None`），TaskMgr 的 Turn task 进 Waiting(`input`)。
3. **出站问题**：§3.8。
4. **回填**：drive 循环里 `hold_inputs` 对这种等待放行**一条** msg 候选：按队列顺序取第一条 msg，receipt 照常确认，作为该 call 的工具结果 `ResumeFill::ToolResults`；event / `_child` / 半订阅观察继续留给调用方，不进子 run。工具结果 JSON：

   ```json
   { "text": "…", "attachments": [{"obj_id": "…", "name": "…"}],
     "option": "a | null", "in_reply_to_question": true,
     "msg_id": "…", "from": "<did>", "at_ms": 0 }
   ```

   `in_reply_to_question` = 该消息 `relates_to{thread, target}` 的 target 等于本 Turn 占位 P 的 `msg_id`；`option` 来自 `content.machine {intent: "agent_ask.answer", data: {call_id, option}}` 且 `call_id` 匹配，否则 null。
5. **恢复后**：`waiting_for` 清空，run 从挂起点继续，Turn 仍打开；再次 `ask-user` 重复 2—4。
6. **崩溃 / 卸载**：等待全在 `state.json`（live run + `waiting_for` + outbox）；Supervisor 空闲卸载后有新输入再装载即可继续。`reconcile.rs` 对挂起 run 的对齐沿用 PendingTool 规则；问题入 outbox 前 / 后、回答提交前 / 后四个窗口各恰好一次。
7. **stop**：`/stop`、`control(stop)`、TaskMgr 取消 → 现有 stop 路径（栈清空、Turn Stopped、占位收尾文案）。
8. **接手**：`userinput:` 只有 Session 能解析，xllm 的 `can_resolve = false`，拒绝接手（与 `subctx:` 同）。
9. **`<sub_task>` 包装**（`assembler.rs:401`）：允许提问的 Session 里改为 “Your result returns to the caller: finish with your report. To ask the user, run `agent-session ask-user`; its result is the user's answer.”；不允许的保持原文。
10. **嵌套**：孙 context 同样适用，总是当前 live 的最内层 run 在问。
11. 主 context 自己的 `WAIT_USER_MSG` 路径（`outcome.rs:261-300`）与子 context 的哨兵路径（`:260`）**不动**。

### 3.8 问答消息流（Agent 侧出站）

```text
[用户]  帮我把这个项目的文档整理一下                          ← 用户消息
[Agent] P：正在处理…（agent_task，任务区：等待你的回答）        ← 占位，Turn 打开
          └ Q1 Agent（thread → P，agent_ask）：你是指 docs/ 还是整个仓库？ ○ docs/ ○ 整个仓库
          └ A1 用户（thread → P，machine: agent_ask.answer）：docs/
          └ Q2 Agent（thread → P，agent_ask）：放到现有的“课程资料”Workspace 吗？
          └ A2 用户（thread → P）：是
[Agent] R：已创建 Work …（P 被 edit 成 R，任务区：成功）  ▸ 4 条问答（收起）
```

1. **确保锚点**：`ask-user` 被登记时，若频道可编辑（`edit_capability`）且本 Turn 还没发占位，**立即**发占位 P（不等 `placeholder_delay_ms`，不要求已有工具调用）。
2. **问题 Q**：`OutboxPurpose::Question`，`relates_to = {rel: thread, target: P.msg_id}`，`to_session` 同 Turn 的回复路由，正文 = 问题文字（选项问题时正文附带选项列表，供不懂 `agent_ask` 的客户端阅读），meta：

   ```json
   "agent_ask": { "anchor": "<P 的 ObjId>", "call_id": "<call_id>", "turn": 3,
                  "input": { "kind": "text | choice", "options": [{"id": "a", "label": "…"}], "allow_text": false } }
   ```

   P 的 `msg_id` 还没回来时 Q 等它（与 `FinalEdit` 跟随占位 id 的逻辑相同，`outbound.rs:372-395`）。幂等键 `(sid, turn, run_id, n)` 沿用，重启不重发。
3. **回答 A**：由 MessageHub 发出（§3.9），`relates_to{thread → P}`，正文 = 用户文字或选项 label，选项时 `content.machine = {intent: "agent_ask.answer", data: {call_id, option}}`。Agent 侧不要求 A 一定带这些：任何下一条 msg 都是答案（§3.7 第 4 条），关系与 machine 只是线索。
4. **结果 R**：Turn 结束，P 被 edit 成 R（现有 `FinalEdit`）。Q/A 作为 R 的线程留在历史里。
5. **不可编辑频道**：没有 P；Q 作为顶层消息（仍带 `agent_ask`，`anchor` 为空）；A 是普通消息；R 是顶层消息。这是今天“其余情况只发最终回复”的延伸。
6. Turn 因 stop / 失败结束时，占位收尾文案照旧；已发出的 Q 不撤回。

### 3.9 MessageHub：输入态与问答折叠

**会话输入态**（新概念，写进 `UI_DATAMODEL.md`）：当前 Session 的输入框样式与目标由逻辑决定，不只是用户手动选的回复。本次只定义一个来源：

- **判定**：会话时间线最后一条可见消息是 Agent 发的、带 `agent_ask` 的消息 → 进入 **回答态**；否则普通态。用户发出回答后最后一条是用户的，自然退出。多设备同步靠同一规则（都看时间线）。
- **回答态 / text**：输入框锁定为对 `agent_ask.anchor` 的回复（无 anchor 时对 Q 本身）；横幅“回答 {{agent}} 的提问”，**没有取消按钮**，不能发别的（附件允许）。发送写 `relates_to{thread → anchor}`。
- **回答态 / choice**：输入框替换成选项（单选按钮 / 列表），点选即发送：正文 = label，`content.machine = {intent: "agent_ask.answer", data: {call_id, option}}`，`relates_to{thread → anchor}`。`allow_text = true` 时额外提供“其他…”切回文本输入。
- 回答态下 `/stop` 仍可发（它是控制命令，也是“我不想答了”的出口）。

**折叠**：`foldMessageRelations` 新规则：`thread` 消息的目标是带 `agent_task` 的 Agent 锚点（P / R）时，不占独立行，归入锚点的 `ui_relations.thread = [{id, from, content, at, agent_ask?}]`，锚点下方渲染为问答列表。

- 锚点的 task 在 Waiting 时默认展开；任务结束（R）后默认收起为“▸ N 条问答”。用户在等待中手动收起过就记住（per-viewer，`localStorage`，与任务区展开态同一机制 `useTaskExpanded`）。
- 其它 `thread` 消息（普通回复）渲染不变。
- 任务区 wait reason `input` 显示“等待你的回答”。

**mock**：`mock/data.ts` 加一个“问答中”的 Agent 会话与一个“已完成含问答”的会话；e2e 覆盖回答态（text / choice）、折叠展开、回答后退出。

### 3.10 Workspace 选择

沿用 `task_route` 已有的规则，并补 Runtime 维度：

- 长期项目优先用已登记 Workspace（`workspace list`，按 `workspace_id` 选）；要新建时先决定 **Runtime**（§3.12 的列表，按 Workspace 设计 §7.2：桌面部署优先 `host` 下用户方便访问的目录，NAS 类部署可用 `container` 下 Agent 自管目录），再决定目录；目录位置是典型的要问用户的问题（带默认建议，用 `--option` 列候选）。
- 一次性任务 `--workspace none`。
- Workspace 位置的 `runtime_id` 接 Runtime 登记（I1）：`local` 作为 `container` 的别名只读兼容；`resolve_workspace` / `verify_workspace` 改为按登记可用性与 `runtime_host` 判断。`host` 在执行器落地前不可用，选了它的创建会失败并把原因带回分析。

### 3.11 UI 的转述规则（`chat_route`）

| `outcome` | UI 做什么 |
|---|---|
| `work_created` | 告诉用户已开始、会做什么、用了哪些假设；结果稍后以 `_child` 输入到达，到时 `read-session <sid> --report` 后转述 |
| `goal_created` | 告诉用户目标已建立、怎么推进；后续在 Goal Session |
| `direct` | 自己回答（可用 `report` 里的事实） |
| `declined` / `unsupported` | 转述 `reason` 与 `gaps`；计划任务要明说“还不支持” |
| `needs_confirmation` | 讲 `proposal`；用户拍板后**再调一次** `analyze_intent`，task 带上用户的决定 |
| `status = failed` | 告诉用户分析没完成及原因；不要自己把活干了 |

UI 不重复问 `clarifications` 里已答的问题（它们已经作为 Q/A 出现在消息流里，用户看过了）；UI 不把 `evidence` 的探索细节全讲给用户。

### 3.12 Runtime 登记

Agent State 新增 `state/runtimes/<runtime_id>.json`，与 Workspace 登记同构：

```json
{
  "version": 1,
  "runtime_id": "container | host | <用户添加的 id>",
  "kind": "container | host_native | remote_ssh",
  "name": "…", "description": "…",
  "runtime_host": "<native_host_id()>",
  "exec": { "<agent_tool RuntimeConfig>": "container = {kind: native}；host 在执行器落地前为 null" },
  "roots": ["该 Runtime 内建议放 Workspace 的根目录"],
  "capabilities": { "tools": {}, "network": true, "os": "linux" },
  "availability": "available | runtime_unavailable | permission_denied",
  "builtin": true,
  "revision": 1, "registered_by": "…", "registered_at_ms": 0, "checked_at_ms": null
}
```

- Loader 启动时幂等补建两条 `builtin` 记录（不可 unregister，可改 `name / description / roots`）：`container`（Agent 进程自己所在环境；`debug_jarvis.sh` / `--dev` 下就是宿主机进程，id 仍叫 `container`，description 注明；`exec = {kind: native}`，`roots` 取 AgentRoot 的 Workspace 根与 `data/home/<owner>` 挂载点）；`host`（`exec = null`、`availability = runtime_unavailable`、description 说明需要宿主机执行器）。
- `AgentStateClient::runtimes()`：`list / lookup / register / update / check / unregister`（文件版 + kRPC `runtimes.*`，照 `workspaces` 的样板；写要署名）。
- CLI：`agent-session runtime list [--available]` / `get <id>` / `register` / `check <id>`。
- `create-worksession --runtime <id>` 先查登记：不存在或不可用拒绝（退出码 6）。
- 首页在 Workspace 旁列出 Runtime。
- RuntimeGrant / 临时授权（xAgent §5.4）不在范围。

### 3.13 behavior 的 `[tool]` 声明：LLM 可见函数的可配置性

现状：子 context 只能经通用工具 `call_behavior({behavior, task})` 调用（`runner/tools.rs:208-320`），模型要先记住系统提示里的约定、再拼对 behavior 名；工具名本身不携带路由信号。新规则：**一个 behavior 可以声明自己作为 LLM 可见的具名函数**，宿主按声明生成工具，内部沿用 `call_behavior` 的子 context 协议。libopendan 仍不认识任何 behavior 名字。

```toml
[tool]
name = "try_analyze_user_intent"      # 原生 function 名（provider 规则：字母数字下划线）；缺省 = behavior 名
description = "…"                      # 什么时候该调；这就是路由指南
parameters = { … }                     # JSON Schema；缺省 {type: object, required: [task], properties: {task: string}}
task = "{{args.request}}"              # 渲染 ChildCall.task 的模板；缺省 = 参数 JSON 原文
```

- **谁能用**：调用方 behavior 的 `meta.next` 里、`entry.mode` 为 `create_sub_context | fork`、且带 `[tool]` 的目标。`tool_whitelist` 条目写 `behavior:<name>`（也支持只写 `group:…` 时自动注入所有带 `[tool]` 的 next 目标，二选一在实施时定，倾向显式 `behavior:<name>`）。
- **冻结**：`[tool]` 随 behavior 一起冻结进 `session_config.prompt.frozen`（`BehaviorEntry` 加 `tool: Option<BehaviorTool>`），Session 存续期间工具定义不变（前缀缓存）。
- **实现**：`CallBehaviorTool` 泛化为 `BehaviorTool { behavior, spec, task_template, depth, max_depth }`，`call()` 与今天完全一样：校验目标、校验深度与 `allow_deferred`，返回 `Pending{subctx:<call_id>}`；`ChildCall.task` = 模板渲染结果；`ChildCall.behavior` = 目标名。结果回填（`sub_result_observation`）、worklog、崩溃恢复不变。
- **`call_behavior` 的去向**：保留为可选的通用入口（给没写 `[tool]` 的目标用），但不再缺省进任何白名单；Jarvis 包的 UI behavior 不再列它。
- **behavior（XML）循环**：`[tool]` 同样可以作为 action 暴露（`action_whitelist` 里写 `behavior:<name>`），本次不需要，不实现，但协议上不排除。
- **文档**：xAgent §3.6 的 `call_behavior` 协议不变，加一段“`[tool]` 声明”；`protocol/behavior.rs` 的 schema 与 `doc/opendan/protocol` 同步。

---

## 4. 实施 TODO

### I1：Runtime 登记（libopendan + Loader + xagent）

- [x] `protocol/runtime.rs`：`RuntimeRecord` 等类型 + schema。
- [x] `state/runtimes.rs`（CAS 修订、`builtin` 保护）；`state/mod.rs` 门面 `runtimes()`；`state/krpc.rs` 的 `runtimes.*`；`KrpcAgentStateClient`。
- [x] Loader 启动补建 `container` / `host`（`loader.rs`，Agent State 建好之后、Supervisor 之前）；`--dev` 同样。
- [x] `xagent runtime list|get|register|check`；Session 内 `agent-session runtime …`。
- [x] `create-worksession --runtime` 查登记；Workspace 位置 `runtime_id` 接登记（`local` 别名只读），`resolve_workspace` / `verify_workspace` 改判。
- [x] 首页展示。
- [x] 测试：登记 CRUD 与 `builtin` 保护；kRPC 往返；`--runtime host` 被拒；Loader 测试里能看到两条记录。

### I2：`ask-user` 与问答出站（libopendan + xagent）

- [x] `bin/xagent.rs`：`cmd_ask_user`，打印 §3.7 的 Pending；参数校验（`--option` 至少 2 个或没有；id 唯一）。
- [x] `runner/tools.rs`：`USER_INPUT_TASK_PREFIX`，识别挂起在 `userinput:` 上的快照（照 `pending_sub_call`）。
- [x] `runner/drive.rs`：登记等待（`WaitingKind::Input` + refs）、不允许的 Session 返回错误观察、`hold_inputs` 放行一条 msg 回填、`in_reply_to_question` 与 `option` 的计算（对照 outbox 里本 Turn 占位的 `msg_id`）。
- [x] `runner/outbound.rs` / `protocol/state.rs`：`OutboxPurpose::Question`，`thread → P`，等 P 的 `msg_id`，`agent_ask` meta；`ask-user` 触发立即占位（`OutboundSink::placeholder` 的调用时机）。
- [x] `reconcile.rs`：四个崩溃窗口；卸载 / 装载。
- [x] `assembler.rs:401` 文案切换。
- [x] **`[tool]` 声明**（§3.13）：`protocol/behavior.rs` 加 `BehaviorToolConfig {name, description, parameters, task}` 与校验（名字合法、schema 是 object、模板变量只引用 `args.*`）；`protocol/config.rs` 的 `BehaviorEntry` 冻结它；`runner/tools.rs` 的 `CallBehaviorTool` 泛化为 `BehaviorTool`，`runner/live.rs:101` 按白名单 `behavior:<name>` 注册；`call_behavior` 仍可显式列入；`tool_whitelist` 解析支持 `behavior:<name>`。
- [x] 测试（`tests/runner_more.rs` / `tests/context_switch.rs` / `tests/crash.rs` / `tests/outbound.rs` / `tests/xagent_cli.rs`）：具名工具调用进入子 context、`task` 模板渲染、未在 `next` 里的目标不生成工具、无 `[tool]` 的目标仍只能经 `call_behavior`、名字冲突（两个 behavior 同名 `[tool]`）在冻结时被拒；fork 子 context 内 ask-user → waiting(Input) → 问题入 outbox 一次（`thread → P`、`agent_ask`）→ msg 回填（文本 / 选项 / 非回复）→ 继续 → report 返回；期间 event 不进子 run；Work 里 ask-user 报错；stop；kill -9 四窗口；嵌套；xllm 拒绝接手；`sub_context_wait_and_failure_return_to_the_caller` 原断言不变。
- [x] 协议文档：`Session Input Protocol.md`（`userinput:`、回填结果格式）、`Session Control Protocol.md`（waiting_for.refs）、outbox purpose；fixtures 重生成。

### I3：Loader（opendan）

- [x] `ui.rs`：`OutboundSink` 支持 `Question`（中途消息，`relates_to` 透传，不加文案）；`placeholder()` 可被立即触发；`has_placeholder` 处理。
- [x] `tests/loader.rs` 端到端（脚本化 LLM）：用户消息 → `call_behavior(analyze_intent)` → `ask-user` → 用户收到 P 与 Q（Q `thread → P`，带 `agent_ask`）→ 用户回答（`thread → P`）→ 子 context report → P 被 edit 成 R；只有一个 Turn、一个 TaskMgr task，task 中途 Waiting(`input`)。
- [x] 选项版：Q 带 `input.kind = choice`，回答带 `machine.agent_ask.answer`，工具结果 `option` 正确。

### I4：MessageHub（desktop）

- [x] `protocol/msgobj.ts`：`AgentAsk` 类型、`messageAgentAsk()`；回答消息的 `machine` 构造。
- [x] 会话输入态：从时间线推导（§3.9）；`ConversationComposer.tsx` 的锁定回复态（无取消）与选项态；`/stop` 仍可发。
- [x] `conversation/history/relations.ts`：Agent 锚点下 `thread` 折叠为 `ui_relations.thread`；渲染组件（问答列表、展开 / 收起、默认值、per-viewer 记忆）。
- [x] 任务区 wait reason `input` 文案。
- [x] `mock/data.ts`、`i18n/messagehub.ts`（中英至少）、`UI_DATAMODEL.md`。
- [x] 测试：`tests/datamodel/messagehub-agent-task.test.ts`（折叠与输入态推导）、`tests/e2e/pages/messagehub-agent-task.spec.ts`（text 回答态、choice 回答态、回答后退出、折叠默认值与记忆、普通 `thread` 不受影响）、`tests/e2e/real/messagehub-agent-task.real.spec.ts` 加问答流（配 `test/test_opendan/test_agent_chat_task.ts` 的问答脚本）。

### I5：`work_simple` behavior（Jarvis 包）

- [x] `behaviors/work_simple.toml`：`mode = "function_call"`；system：单一有界任务，直接用工具完成，验证后 `report(is_end: true)`，做不到就报告失败；`tool_whitelist = ["group:bash"]`（`report` 由 function_call Session 默认包含）；`budget.max_tool_iterations = 24`；`model = llm.chat`；无 `[entry]`。
- [x] 用例：`create-worksession --behavior work_simple` 的 Work 用 mock 跑到结束并有 `report.md`。

### I6：`analyze_intent` 与路由（Jarvis 包）

- [x] `behaviors/analyze_intent.toml`（§3.6）：`[tool] name = "try_analyze_user_intent"`；`on_context_switch` 引导文字按 §3.2—§3.5、§3.10 写，含 `result` 格式与禁止事项（不修改外部状态、不扩大原始需求、不替用户做授权决定、不建重复 Work / Goal、不加 `--wait`、不用 `WAIT_USER_MSG` 只用 `ask-user`、不再调 `try_analyze_user_intent`）。
- [x] `--objective` / `--msg` 规范：objective 自包含；`--msg` 带 `clarifications` 与 `evidence` 要点、验收条件；`--context recent:6`。
- [x] 删除 `task_route.toml`；`chat_route.toml` / `groupchat_route.toml` 的 `next = ["analyze_intent"]`，`tool_whitelist = ["group:bash", "behavior:analyze_intent"]`（去掉 `call_behavior`），Routing 段按 §3.11 重写并缩短（路由条件已在工具 description 里；保留 “analysis asks the user directly; do not repeat its questions”）；群聊版加“只路由 owner 的请求”。
- [x] `agent.toml`：`[session.ui]` 的 `max_process_depth` 够用（4）；核对 fork 目标的 `[budget]` 生效。

### I7：文档

- [x] 架构文档新增 **§5.6 意图分析的实现形态**：具名函数由 behavior `[tool]` 声明（LLM 可见函数的可配置性）、fork 与前缀缓存（为什么不是 create_sub_context）、`ask-user` 的 PendingTool 语义、问答消息流（P/Q/A/R）、与 D2 的关系、不可编辑频道的退化；§15.5 表加“意图分析”行；§17.2 前两行填已采用。
- [x] xAgent.md §3.6 加 “`[tool]` 声明” 一段；`doc/opendan/protocol` 的 behavior schema 加 `tool`。
- [x] `doc/opendan/特殊的AgentTools.md` §3.2、`NewOpenDANRuntime.md` §8：加“已被 `call_behavior` + `analyze_intent` 取代，见本文”。
- [x] `src/frame/lib_opendan/README.md`、`src/frame/opendan/README.md`：`ask-user`、`runtime …`、`runtimes.*`、`Question` 出站、`agent_ask`。
- [x] `doc/message_hub/Message Center.md` 或 UI 数据模型：`agent_ask` meta 与 `agent_ask.answer` machine intent 的约定（谁读、谁忽略）。
- [x] `doc/opendan/protocol/Agent State Protocol.md` 加 Runtime 登记；`Agent Workspace Protocol.md` §1 的 “Runtime 引用为 `local`” 改引用登记。

### I8：验证（退出条件）

- [ ] §5 全部通过；§6 全绿。
- [ ] DV 三种真实对话：① 简单一次性任务 → `work_simple` 完成并转述；② 需要澄清的项目任务 → 至少一轮 text 提问 + 一轮 choice 提问 → Workspace 选定 → `plan` Work；③ 长期目标 → 提问后 `goal create` → Goal Session 第一次 Activation。记录消息序列（P/Q/A/R）、Turn/task 数、UI 历史里只剩 `call_behavior` 结果、子 run 与父 run 的 `cache_read_input_tokens`。

---

## 5. 验收矩阵

| # | 场景 | 期望 |
|---|---|---|
| V01 | Runtime 登记 CRUD、`builtin` 不可删、kRPC 往返 | 通过 |
| V02 | Loader 启动后 `runtime list`：`container` available、`host` runtime_unavailable；重启不重复、不覆盖用户改过的 name | 通过 |
| V03 | `create-worksession --runtime host` | 退出码 6 |
| V04 | Workspace 位置 `container`（及旧 `local`）可解析；`host` 创建 Workspace 失败并说明 | 通过 |
| V05 | fork 子 context 内 `ask-user`（ui Session）→ waiting(Input, `userinput:`)，Turn 打开，P 立即发出，Q 入 outbox 一次（`thread → P`，`agent_ask`） | 通过 |
| V06 | 下一条 msg 回填为工具结果（文本；`thread → P` 时 `in_reply_to_question = true`；非回复时 false），调用方未推进，event 留在总线 | 通过 |
| V07 | choice：Q 带 options，回答带 `machine.agent_ask.answer` → 工具结果 `option` 正确 | 通过 |
| V08 | 子 context `report(is_end)` → 调用方以 `ToolResults` 恢复，`submission` 含 `report` 与 `result`；P 被 edit 成 R；`turns_completed` +1 | 通过 |
| V09 | Work（无队列）里 `ask-user` | 错误观察，Work 不挂起 |
| V10 | 子 context `WAIT_USER_MSG` 哨兵 | 原行为不变（`runner_more.rs:1006`） |
| V11 | kill -9：Q 入 outbox 前 / 后、A 提交前 / 后 | 各恰好一次 |
| V12 | 等待期间 `control(stop)` / TaskMgr 取消 | 栈清空，Turn Stopped，占位收尾 |
| V13 | 空闲卸载 → 用户回答 → 重新装载继续 | 通过 |
| V14 | 嵌套：孙 context 提问 | Q/A 落在孙 run |
| V15 | 子 run 第一个请求 = 父前缀 + 一条 user message（字节一致）；父返回后的请求 = 前缀 + call + result | 通过（库级断言渲染结果） |
| V16 | Loader 端到端（脚本化 LLM）：用户收到 P、Q、R 三条，Q `thread → P`，R 是 P 的 edit；一个 Turn、一个 task 且中途 Waiting(`input`) | 通过 |
| V17 | MessageHub：最后一条是 `agent_ask` → 回答态（text 锁定回复 / choice 选项）；回答后退出；`/stop` 可发 | 通过 |
| V18 | MessageHub：Agent 锚点下 `thread` 折叠；等待中展开、结束后收起；记忆；普通 `thread` 不变 | 通过 |
| V19 | `work_simple` Work 用 mock 跑到 `report(is_end)` 并有 `report.md` | 通过 |
| V20 | `analyze_intent` 能解析、冻结；fork 校验通过；`task_route` 不存在且 `chat_route.next` 合法 | 通过 |
| V22 | `[tool]`：UI 的工具列表里有 `try_analyze_user_intent` 而没有 `call_behavior`；调用进入 `analyze_intent` 子 context，`<sub_task>` 是 `request`；无 `[tool]` 的 next 目标不生成工具；同名 `[tool]` 冻结时被拒 | 通过 |
| V21 | DV 三种对话（I8） | 通过并记录 |

---

## 6. 验证命令

Rust（在 `src/`）：

```bash
cargo test -p libopendan -- --test-threads=1
cargo test -p opendan -- --test-threads=1
cargo check --workspace
cargo run -p libopendan --example fixtures -- <绝对路径>/doc/opendan/protocol/fixtures   # 改协议后
```

Desktop（在 `src/frame/desktop/`）：tsc、eslint、deno datamodel 测试、`npx playwright test tests/e2e/pages/messagehub-agent-task.spec.ts tests/e2e/pages/messagehub.spec.ts tests/e2e/pages/messagehub-group.spec.ts tests/e2e/pages/messagehub-actions.spec.ts --workers=2`（分批，避免与 cargo 并行被 OOM 杀）。

DV：`cd src && ./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io`；消息用 `test/test_opendan/test_agent_loader.ts` / `test_agent_chat_task.ts`（`OPENDAN_URL=http://127.0.0.1:<port>/kapi/opendan`）；`xagent status <sid> --worklog 50 --run`；desktop 静态文件可直接 `cp -r dist/. /opt/buckyos/bin/control-panel/web/`；真实 e2e `MESSAGEHUB_REAL_ZONE_IP=127.0.0.1`。已有 UI Session 冻结了旧 behavior，发 `/stop` 开新一代才会用上 `analyze_intent`。DV 结束把测试 Goal 归档。

---

## 7. 实施顺序

1. I2（libopendan `ask-user` / `userinput:` / `Question` 出站）先做并通过库级测试；它与 Jarvis 包和 UI 无关，是整个形态的基础。
2. I1（Runtime 登记）、I4（MessageHub）、I5（`work_simple`）并行。I4 可以先靠 mock 数据做，不等 Agent 侧。
3. I3（Loader 端到端）、I6（`analyze_intent` + 路由）。
4. I8 DV → I7 文档回写。

---

## 8. 决定与待确认

### 8.1 用户决定（2026-10-11）

1. 分析用 `fork` 复用前缀缓存；新“工具”是 CLI，通过一条 user message 引导，不改工具集。架构文档要把这类细节写详细（I7）。
2. 反问做成“函数只有 Session 拿到 user message 才返回”；一条消息就是答案（以后 UI 配合再优化）。
3. 问答消息流：用户的回答自动成为对占位消息的 reply；最终 `usermsg → resultmsg（含问答 reply）`；这是 MessageHub 为 Agent 定制才能做到的，值得不妥协地实现。
4. 顶层 UI context 的 `WAIT_USER_MSG` 不动。
5. 分析用 `llm.chat`。
6. 删 `task_route`。
7. 问题挂在 P 下（不是 edit P）；回答态退出条件 = 时间线最后一条不再是 `agent_ask`；折叠等待中展开、结束后收起并记忆；未读数问题先接受。
8. 回答态下**不能发别的**：输入框锁定为回复；选项问题时输入框变成选项。原则：按逻辑调整当前 Session 的 input style 与 target。
9. UI 看到具名函数 `try_analyze_user_intent` 而不是通用的 `call_behavior`；做成 behavior 的 `[tool]` 声明（§3.13）。原则：LLM 可见的一些基础函数要有可配置性。

### 8.2 本文的实现决定（如需调整，实施前提出）

- `ask-user --option` 给出的问题缺省只能选（`allow_text = false`）；`--allow-text` 才有“其他…”。
- 不可编辑频道上 Q 带 `agent_ask` 但 `anchor` 为空，MessageHub 对其以 Q 自身为回复目标。
- `analyze_intent` 结果格式由引导文字约定，libopendan 不校验；UI 对缺字段按 `failed` 处理。
- `needs_confirmation` 后的再调用是新的一次 `call_behavior`（新 fork），不恢复旧子 context。
- `in_reply_to_question` 与 `option` 由 Session 从 `MsgObject` 算出，Loader 入站不改。
- `work_simple` 用 function_call 循环，不用 behavior（XML）循环。
- Agent 侧对“等待期间用户发了非回复消息”不做特殊路由：仍是答案，`in_reply_to_question = false`。回答态锁定后这种情况只会来自其它客户端或 tunnel。

### 8.3 已知风险（按“先能力后安全”记录，不加机制）

- 分析子 context 有 `group:bash`，提示词约束“只读”，没有执行体强制。
- 自动创建 Work / Goal 不经二次确认；错误分析直接消耗预算（Goal 有周期预算，Work 有 run 预算）。
- 等待期间 UI Session 对其它请求“失聪”：其它客户端发来的新消息会被当成回答。
- 前缀缓存是否真的命中取决于 provider 链路；llm_context 不打 Anthropic `cache_control` 断点，需另行处理。
- `host` 登记可见但不可用，模型可能反复尝试；引导文字要求先看 `availability`。
- msg-center 未读数把 P / Q / A / edit 都算一条。

---

## 9. 禁止的捷径

- 不要把提问做成子 context 返回 `needs_user_input`、由 UI 主 context 转问再重调（那是现状，会把 UI 历史填满中间步骤，也丢掉已探索的上下文）。
- 不要用 `WAIT_USER_MSG` 哨兵给子 context 加等待语义；提问只走 `ask-user` 的 PendingTool 路径。
- 不要用 `--interactive` Sub Session 代替子 context 做分析。
- 不要让子 context 的提问关闭 Turn 或计入 `turns_completed`。
- 不要给 `analyze_intent` 写 system 或换模型 / 工具集来“增强”它，那会丢掉前缀缓存；要加的说明都放进引导 message。
- 不要在 libopendan 里认识 `analyze_intent` / `try_analyze_user_intent` 这些名字或它的结果字段；具名工具只能来自 behavior 的 `[tool]` 声明，不要写死一个 `TryAnalyzeUserIntentTool`。
- 不要让 MessageHub 靠 TaskMgr 状态决定输入态；只看时间线最后一条消息。
- 不要实现计划任务、通知投递、宿主机执行器、Task 修订。
- 不要改 llm_context / agent_tool；需要时单列 TODO 先行。
- 不要把 `host` 记录伪装成 available。

---

## 10. 需求来源（用户 2026-10-11 的理解，原文摘录）

1. 正确实现 `try_analyze_user_intent` 的本质是一个深度意图分析路由：是否可以创建一个 worksession；是否可以创建一个 goal-session；是否应该创建一个计划任务（在未来执行的 worksession？这个先不实现）。创建 worksession 的时候，要选择/创建 workspace，对任务规模进行判断。普通任务用 Plan-Do，简单任务用传统的 function loop worksession。创建 worksession 和 goal-session 都需要确认需求是否清晰，这个过程可能需要反问用户（可以参考一点网络上流行的 grill me skill）。收集信息后，在内部完成创建（如满足提交），并返回结果。
2. 出于创建 workspace 的需要，现在 agent 需要知道自己有哪些 runtime 可以用。目前系统应该默认带 2 个 runtime：容器 runtime 和 host native runtime。这个需要 agent 内部有 runtime 管理模块，后续允许用户添加。
3. 拷问用户的时候，是从 `try_analyze_user_intent` fork 的 llm context 里要求等待用户输入，要看一下现在的 ui agent session 是否支持这种情况下变成 WAIT_USER_INPUT。（核对结果：不支持；用 fork 是为了复用 llm 缓存，实现应是添加一条 user message 说明新的可用 CLI 工具、引导 LLM 用 shell 完成分析与创建；等待用户输入相当于内部调用一个类似 wait_input 的函数，只有 Session 拿到 user message 才返回。→ §3.6、§3.7。）
4. 结束后，fork 的 llm context 结束，UI Session 只看到函数返回的报告，省去大量中间步骤。报告要包含创建结果、用户输入等信息。（→ §3.1；已返回子 context 的 transcript 在重建 `<session_history>` 时被排除。）
5. 反问时 UI 上消息输入变成回复（不能发别的）；问题是选项时，输入框变成 selection。按逻辑调整当前 MessageHub Session 的 input style 和 target。（→ §3.8、§3.9。）

---

## 11. 实施记录（2026-10-11）

### 11.1 落点

| 项 | 位置 |
|---|---|
| `[tool]` 声明 | `lib_opendan/src/protocol/behavior.rs`（`BehaviorToolConfig`、`BEHAVIOR_TOOL_PREFIX`、task 模板只认 `{{args.<name>}}`、fork 的 `[budget]` 不进 `llm_context`）；`protocol/config.rs`（`BehaviorEntry.tool`、`behavior_tools()`、`behaviors()` 校验同名函数与 `behavior:<x>` 目标）；`runner/tools.rs`（`BehaviorTool::any / named`、`behavior_of_function`、`pending_sub_call` 按函数名找回 behavior）；`runner/live.rs`（`xllm_deps_for` 注册 `behavior:<name>`、`Shared.behavior_functions` 让这些调用按只读处理、`fork_run_context` 应用冻结的 `[budget]`） |
| `ask-user` / `userinput:` | `protocol/input.rs`（`UserInputRequest / Spec / Answer`、`AGENT_ASK_META`、`AGENT_ASK_ANSWER_INTENT`、`USER_INPUT_TASK_PREFIX`）；`runner/ask.rs`（允许条件、问题入 run.json、`waiting_for`、取答案、结果）；`runner/tools.rs`（Pending 检查）；`runner/outcome.rs` 与 `runner/drive.rs::commit_waiting`（问题与等待同一次提交）；`runner/live.rs`（`fill` / `fill_answer` / `commit_answer`、`pending_results` 的回答与 stop）；`session/runs.rs`（`save_ask / ask`）；`runner/assembler.rs`（`<sub_task>` 文案）；`runner/turn_task.rs`（“waiting for the user's answer”）；`bin/xagent.rs`（`ask-user`） |
| 出站 | `protocol/state.rs`（`OutboxPurpose::Question`）；`runner/outbound.rs`（`push_placeholder`、`queue_question`、`question_targets`、`retarget_questions`、`OutboundSink::question` 钩子） |
| Runtime 登记 | `protocol/runtime.rs`（`RuntimeRecord / Register / Update`、`CONTAINER_RUNTIME / HOST_RUNTIME / LEGACY_LOCAL_RUNTIME`、`builtin_runtimes`、`canonical_runtime / is_native_runtime`）；`state/runtimes.rs`；`state/mod.rs`（`RuntimeRegistry`、`AgentStateClient::runtimes()`）；`state/krpc.rs`（`runtimes.*`）；`state/workspaces.rs` 与 `runtime/mod.rs`（按登记判断、`local` 即 `container`）；`api.rs`（`create_sub_session` 查登记）；`bin/xagent.rs`（`runtime …`）；`LOCAL_WORKSPACE_RUNTIME` 已删 |
| Loader | `opendan/src/loader.rs`（启动补建 `container` / `host`，`container_roots`）；`opendan/src/ui.rs`（`MsgCenterSink.owner`，群聊问题 `mentions` 主人）；`web/`（首页 `RuntimeList`、`runtimes.list`、mock、i18n） |
| MessageHub | 子任务完成，见 `UI_DATAMODEL.md` §3.5.5：`protocol/msgobj.ts`、`conversation/input/inputState.ts`（新）、`conversation/history/AgentQuestions.tsx`（新）、`relations.ts`、composer、tasks、mock、i18n、测试 |
| Jarvis 包 | `behaviors/analyze_intent.toml`、`behaviors/work_simple.toml`（新）；`chat_route.toml` / `groupchat_route.toml` 改为 `next = ["analyze_intent"]`、`tool_whitelist = ["group:bash", "behavior:analyze_intent", "report"]`；`task_route.toml` 已删 |
| 测试 | `lib_opendan/tests/ask_user.rs`（12）、`tests/runtimes.rs`（4）、`tests/crash.rs::asking_the_user_survives_a_crash_at_every_stage`、`tests/xagent_cli.rs::ask_user_prints_a_pending_question`；`opendan/tests/loader.rs`（Jarvis 包断言、`a_question_and_its_answer_stay_in_the_turn`、`a_question_in_a_group_mentions_the_owner`）；`web/tests/e2e/home.spec.ts`；desktop 见上 |
| DV 驱动 | `test/test_opendan/test_agent_ask.ts`（`send / answer [--option] / wait / stop`，提问时退出码 10，结果后打印 `AGENT_ASK_MSG=<占位>` 给 `messagehub-agent-task.real.spec.ts`） |
| 文档 | 架构文档 §5.6、§15.5 表、§17.2 前两行；xAgent §3.6 / §6.2；协议 README、Session Input §9、Session Directory（outbox `question`、`waiting_for`、子 context 表）、Agent State §7 / §10、Agent Workspace §1；`Message Center.md` §2.1.2；`特殊的AgentTools.md` §3.2、`NewOpenDANRuntime.md` §8 标注已取代；两个 README；schema `runtime_record / user_input_request / user_input_answer`；fixtures 已重生成 |

### 11.2 与本文不同或本文没写的决定

1. **回答的提交**：答案作为工具结果回填，同一快照里追加一张不注入消息的输入 receipt（`inputs = [该 msg]`、`parts = []`、`extra.user_input = <call_id>`），走 §8.3 的提交顺序。回执加入打开的 Turn。
2. **问题先于答案**：问题与 `waiting_for = input` 在同一次提交；`drive` 的等待分支先 `commit_waiting` 再取答案。崩溃测试发现过“恢复时问题还没发出就把队列里的消息当答案”的顺序问题，已修。
3. **问题的持久化**：`ask-user` 的问题在快照挂起前记入 run.json `host.extra.asks[<call_id>]`，恢复时据此补发；“是否已入 outbox”看 state 的 `waiting_for.refs` 是否已含该调用，不看 outbox（outbox 会裁剪）。问题的幂等键是 `<sid>:<turn>:ask:<call_id>`，不是 `(sid, turn, run_id, n)`。
4. **允许条件**多一条“不是子 Session”：交互式子 Session 的“用户”是父 Session，问题发不出去。
5. **具名函数的可见性**：调用方显式写 `behavior:<name>`（不做 `group:` 自动注入）。`call_behavior` 仍在宿主工具里，只是不进任何白名单。具名函数调用按只读处理（不登记在途），与 `call_behavior` 一致。
6. **fork 的 `[budget]`**：冻结时从 `llm_context` 叠加里剔除 `max_tool_iterations / max_tokens`（否则 fork 校验拒绝），由 `fork_run_context` 经 `ForkOptions.tool_policy / budget` 与 `error_policy` 作用于分支；不改 llm_context。
7. **`report` 进 UI 白名单**：fork 继承工具集，分析要用 `report` 交 `result`，所以 `chat_route` / `groupchat_route` 白名单加 `report`；UI 的提示词说明它属于分析。子 context 的最终 report 不受“子 Session 未结束”限制（已有规则），分析建完 Work 可以直接交。
8. **引导文字的变量**用 `{{handover.sub_task.task}}`（实际可用的渲染变量），不是 `{{sub_task}}`。
9. **Runtime**：`create-worksession --runtime` 未登记 / 不可用返回 `Bind` 错误（退出码 6）；`container` 不再生成 runtime requirement（它就是本进程环境）。`container` 的 `roots` = AgentRoot `workspaces/` + AgentRoot 所在的 owner home；描述按 `/.dockerenv` 区分容器与宿主机进程。Workspace 可用性 = `container` 登记可用 且 owner 没用 `set-runtime` 关掉。
10. **群聊**：MessageHub 只对问题 `mentions` 的成员进入回答态，所以加了 `OutboundSink::question` 钩子，Loader 在群聊问题上 `mentions` 主人（只有主人的请求会到群 Session）。
11. **MessageHub**（子任务的取舍，已采纳）：只折叠问答（问题；带 `agent_ask.answer` 的消息或有未答问题时提问者以外的人的下一条 `thread`），不是所有指向带 `agent_task` 锚点的 `thread`——每条 Agent 回复都带 `agent_task`，全折叠会吞掉普通回复；默认展开 / 收起看时间线（锚点被 edit 成 R 后收起），不看 TaskMgr；回答态横幅与选项头有“停止任务”按钮（发 `/stop`），文本回答态里输入 `/stop` 也作为控制命令发出。
12. Turn task 等待文案为 “waiting for the user's answer”（MessageHub 按 wait reason `input` 显示“等待你的回答”）。
13. 顺手修了 `opendan/tests/loader.rs::the_agentroot_of_an_earlier_agent_is_archived` 的竞态（它读到前一个 Loader 写的 generation 1）；新的启动 I/O 让它更常失败。

### 11.3 验证（都在 `src/` 下，除注明外）

- `cargo test -p libopendan -- --test-threads=1`：全绿（新增 ask_user 12、runtimes 4、crash 1、xagent_cli 1）。`goal_session::the_period_limit_skips_fires_and_refuses_work_but_absorbs_results` 在全量并跑时偶发失败一次，单跑与整组重跑 2/2 通过（定时类测试，未改动相关代码）。
- `cargo test -p opendan -- --test-threads=1`：loader 19 全绿（连续两次）。
- `cargo check --workspace --tests`：通过。
- fixtures：`cargo run -p libopendan --example fixtures -- /home/lzc/project/buckyos/doc/opendan/protocol/fixtures` 后 `--test fixtures` 19 通过。
- OpenDAN WebUI（`src/frame/opendan/web`）：`pnpm check`、eslint、`playwright test tests/e2e/home.spec.ts` 8 通过。
- Desktop（`src/frame/desktop`）：`npx tsc -b`、eslint、deno datamodel 49 通过；Playwright `messagehub-agent-task` 12、`messagehub` / `group` / `actions` 43、`issue638` / `worklog` / `deeplink` / `media` / `admission` 17 通过；浅色 / 深色截图已看过。
- `test/test_opendan/test_agent_ask.ts`：`deno check` 通过，未对真实 zone 跑。

### 11.4 已知风险（按 §8.3，另加）

- 等待期间，用户在问题发出前发的消息也会被当成答案（`in_reply_to_question = false`，由分析判断）。
- `analyze_intent` 的 `result` 只由引导文字约定，宿主不校验；UI 按 `chat_route` 的说明处理缺字段。

### 11.5 DV（I8）状态：未完成

2026-10-11 用 `./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io` 起了新 Loader：启动正常，日志 `loader: runtimes container=Available, host=RuntimeUnavailable`，AgentRoot 已从源码包同步（新 behavior 已在 AgentRoot 里）。但 xiaobai 已有的 4 个 UI Session 是 `opendan.session_state/5`（旧镜像写的），当前源码是 `/6`，`recovery_blocked: unsupported state.json schema`（早上 04:18 的调试运行已经如此，与本次改动无关）。devtest 与 xiaobai 的对话落在其中一个 Session 上，新消息不会有回答。按以往做法要把这些 Session（`sessions/<sid>`、`state/sessions/<sid>.json`、`state/perception/<sid>.jsonl`）移走让 Loader 开新一代，这一步被权限拦下，需要用户决定（自己移走，或授权我移走），然后：

1. `./debug_jarvis.sh devtest --agent xiaobai.test.buckyos.io --no-build`；
2. `cd test/test_opendan && deno run --config ../deno.json --allow-net --allow-env --allow-read --allow-write --unsafely-ignore-certificate-errors test_agent_ask.ts send "<请求>"`，按提示 `answer …`，跑 §4 I8 的三种对话；
3. 记录 P/Q/A/R、Turn / task 数、UI 历史只剩函数调用结果、子 run 与父 run 的 `cache_read_input_tokens`；
4. desktop `pnpm build` 后 `cp -r dist/. /opt/buckyos/bin/control-panel/web/`，跑 `messagehub-agent-task.real.spec.ts`（`MESSAGEHUB_REAL_E2E=1 MESSAGEHUB_REAL_ZONE_IP=127.0.0.1 AGENT_ASK_MSG=<上一步打印的占位>`，或 `AGENT_ASK_LIVE=1`）。

注意：停掉调试 Loader 后 node-daemon 拉回的旧镜像容器读不了新 behavior 里的 `[tool]`，它新建的 UI Session 冻结 `chat_route` 会失败；旧 Session 不受影响。以本地镜像验证前要先 `./build_aios --local-test`。

### 11.6 遗留

- I8 DV（V21）与前缀缓存命中的实测。
- `[tool]` 作为 behavior（XML）循环的 action 暴露（协议不排除，未实现）。
- Anthropic `cache_control` 断点（llm_context，单列）。
- 计划任务、宿主机执行器、Task 修订、通知人（本文范围外）。
- MessageHub：深链到被折叠的问答显示“消息不存在”；问答条目不能打开详情。
