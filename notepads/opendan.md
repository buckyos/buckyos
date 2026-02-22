# OpenDAN Agent Runtime（BuckyOS）设计文档（合并版）

> 本文将旧版 `opendan.md` 与更新文档 **《OpenDAN Agent Runtime 设计》** 合并为一个一致的新版本。  
> 若两份文档存在歧义或冲突，以 **《OpenDAN Agent Runtime 设计》** 为准（尤其是 Agent Loop / Session Loop / Behavior Step 等核心运行机制）。

---

## 1. OpenDAN 的定位

OpenDAN 是 `OpenDAN-Personal-AI-OS` 在 BuckyOS 体系下的移植与进化版本（项目地址：`https://github.com/fiatrete/OpenDAN-Personal-AI-OS`）。

OpenDAN 不重复实现 BuckyOS 的基础服务，而是专注于 **AI Runtime 的基础设施**：让 Agent 以“可持续、可观测、可协作、可扩展”的方式运行，并支持用自然语言扩展新的 Agent/Behavior/Tools/Skills。

原则上 OpenDAN 位于 BuckyOS 应用层；移植初期可以先作为 BuckyOS 服务存在，后续可演进为应用层 Runtime（可被 App/Workspace/Zone 复用）。

### 1.1 设计目标

* 提供统一的 **Agent Runtime**：调度、行为执行、Action/Tool 执行、记忆管理、工作区产物交付。
* 支持 **自然语言扩展**：Behavior/Skill/Tool 可由 Agent 自主生成、安装与演进（在 Policy 护栏下）。
* 与 BuckyOS 现有基础组件无缝集成：LLMCompute、TaskMgr、MsgCenter/MsgQueue、FileSystem/NameStore、KnowledgeBase、Content Network 等。
* 默认安全：尤其是 bash Action、写文件、以及真钱钱包相关能力必须有明确 Policy Gate 与审计。

### 1.2 非目标（Non-goals）

* OpenDAN 不重复实现 BuckyOS 的基础服务（TaskMgr/MsgQueue/MsgCenter/LLMCompute/KB/ContentNetwork 等）。
* MVP 阶段不优先实现“全功能 PC 环境控制”（可保留接口）。

---

## 2. 系统依赖：BuckyOS 基础组件与 OpenDAN 边界

OpenDAN 作为 AI Runtime，依赖并复用 BuckyOS 的以下基础组件：

* **AI Compute Center / LLMCompute（已存在）**
  * 负责：模型路由、推理请求、流式输出、token 统计、function call 协议适配、（可选）成本估算。
  * OpenDAN：将每一次 LLM 推理封装为 TaskMgr 任务，并委托 LLMCompute 执行。
* **TaskMgr（已存在）**
  * 负责：任务生命周期、长任务调度、进度/日志、可观测 UI、取消/超时等。
  * OpenDAN：把 LLM 推理、长 bash Action、文件 diff 写入等都挂到 TaskMgr，形成可追踪链路（trace）。
* **MsgCenter / MsgQueue（已存在）**
  * MsgCenter：统一消息中心（对外 MsgTunnel、群聊、用户消息）。
  * MsgQueue：事件/消息队列（可订阅、可拉取、可游标）。
  * OpenDAN：在 Agent Loop 的输入收集阶段拉取关心的 msg/event；必要时发送 msg；SubAgent 内部消息也走同一套抽象（但不可外部寻址）。
* **FileSystem / NameStore（已存在）**
  * 负责：文件读写、命名、发布、权限与存储。
  * OpenDAN：为每个 Agent/Workspace 提供标准目录结构、写入 diff、产物归档、git 协作桥接。
* **KnowledgeBase（已存在）**
  * 负责：RAG/全文检索/图/向量等底层能力与索引。
  * OpenDAN：提供 Runtime 侧检索/写入契约与 provenance 记录，支持 Self-Improve 与专用维护 Agent（如 Mia）。
* **Content Network（已存在）**
  * 负责：网络搜索、内容发布、（可选）收益机制。
  * OpenDAN：提供 Agent 调用与发布流程、权限与审计接口。

---

## 3. 进程模型与并发模型（保留 + 对齐新机制）

### 3.1 容器边界（建议默认）

Agent 与其所有 SubAgent 运行在一个 OpenDAN 容器里（可基于 Docker 实现隔离）。

推荐默认 **一容器对应一个“根 Agent（Root Agent）”**：

* 一个 Root Agent = 一个 OpenDAN 容器（默认）
* 容器内包含：
  * OpenDAN Runtime Daemon（负责调度/执行/观测）
  * Root Agent Worker（串行执行 session/behavior step）
  * 0..N 个 SubAgent Worker（可独立进程，资源受限）

> 后续可扩展：一个容器跑多个 Root Agent，但 MVP 不强制。

### 3.2 并发模型（以“session 串行”作为确定性基础）

* **Agent 逻辑串行（按 Session）**：任意时刻一个 session 只执行一个 behavior 的一个 step（保证状态可理解、日志可追踪）。
* **Action 可并发**：一个 LLM step 可生成多个 Action；由 ActionExecutor 并发执行，但必须把结果结构化汇总后再进入下一次推理。
* **SubAgent 可并发**：SubAgent 可独立进程并发执行，但其预算（token/时间/文件权限）必须受 Policy 限制。

> Agent 通过 SubAgent 解锁“多核（LLM）”能力；根 session 自身仍保持串行，以确保可恢复与可观测。

---

## 4. 核心抽象：Session / Workspace / Workshop

> 这一节统一运行时的“逻辑容器”与“交付空间”概念，用于替换旧文档中对 on_wakeup/行为循环的部分假设。

### 4.1 Session（任务会话）

* **Session** 是一个逻辑 topic，用来归并必要上下文：消息、事件、todo、summary、cost/trace 等。
* Session 也是 Runtime 中 Agent 执行的主要逻辑容器：  
  若把一次 LLM 推理类比为一次“AI 时代的 CPU 调用”，则：
  * **AgentSession 类似传统 thread**：同一 session 内 LLM 调用总是顺序执行。
  * session 的最小执行粒度是 **behavior step**；每个 step 完成后保存 `session.state`。
  * 若 session 的执行不依赖外部环境，则给定 `behavior_name + step` 可在系统重启后继续运行，也可回退到上一个 step 重新执行。
  * behavior step 通常会读取 `new_msg/new_event`，因此外界对当前 session 的影响，最慢会在下一个 step 生效。

### 4.2 Workspace（交付空间）

* **Workspace 是用来交付成果的地方**：文档、代码、数据、PR、报告等产物位于 workspace。
* Workspace 不必由 runtime 管辖：可以是本地目录、远程 git repo、SMB 共享、甚至公网服务；可通过 skills 扩展接入。

### 4.3 Workshop（Agent 私有工作区）

* **Workshop 是 Agent 私有的工作区**。在 workshop 内，Agent 可以创建多个私有 workspace（通常称为 `code_workspace` / `local_workspace`）。
* `local_workspace` 若由 session 运行中创建，通常是 Agent 私有；  
  若用户先创建 local workspace 再交给 Agent 工作，则该 local workspace 通常属于用户。

> 约束：**session 可以绑定 0 个 local_workspace 和 0..n 个 workspace**。

### 4.4 local_workspace 锁与并行

* 多个 session 可以并行运行，但如果两个 session 使用同一个 `local_workspace`，则这两个 session 只有一个能处于 `RUNNING` 状态。
* 可强化约束：需要等待另一个 session 的所有 todo 完成/失败，以避免非顺序修改同一工作区带来负面影响。

---

## 5. 默认 Agent：Jarvis 的逻辑（示例）

Jarvis 是默认 Agent，但 Jarvis 的流程本质上是“在 runtime 支持下的 behavior 配置组合”。

典型流程（示意）：

```
new_msg + new_event
  |
  `resolve_router` -> 快速回应 / 确定 session
  |
  `Plan-Do-Check-Adjust (PDCA) Loop`

new_event
  |
  `self-improve`
```

Runtime 需要提供的核心能力：

* 运行容器与机制：**Agent Loop + Agent Session**（SubAgent 是高级运行机制）
* Behavior 配置与 LLM Input/Output 标准化
* 状态管理：Session（系统管理更新、Agent 通常只读） + Workspace（Agent 可创作的文件系统）
* 元工具集合：即使没有 workspace，Agent 也能通过元工具完成任务

---

## 6. 触发与调度：Trigger / Scheduler


### 6.1 触发来源（可选组合）

* **Timer/Heartbeat**：定时产生唤醒事件（例如每 3 分钟一次），用于检查 inbox、提醒事项、超时任务等。
* **on_msg**：收到用户消息触发（可由 MsgQueue 推送或触发一次 wakeup）。
* **on_event**：订阅系统事件触发（与 on_msg 不同的是：永远不必考虑向 msg.from 恢复消息）。
  * **on_task**：TaskMgr 任务状态变化触发（例如某 Action 完成）。这是event的一种

> 原则：即便启用 on_msg/on_event，也推荐“最终都落为一次 session READY 调度”，以保持单入口与可观测一致。

### 6.2 “零 LLM 空转”两段式（目标）与机制

* `session.generate_input()` 判空；无有效 input 则 step 直接跳过（不触发推理），并把 session 置为 `WAIT`。

---

## 7. 运行机制：Agent Loop & Session Loop


### 7.1 Session 状态机

Session 的状态（建议最小集合）：

* `PAUSE`：用户手工暂停
* `WAIT`：标准等待；任何事件可唤醒（视 wait 细节）
* `WAIT_FOR_MSG`：等待特定 msg，超时后变为 `READY`
* `WAIT_FOR_EVENT`：等待特定 event，超时后变为 `READY`
* `READY`：就绪，等待执行
* `RUNNING`：正在执行
* `SLEEP`：长期无有效输入进入休眠（减少心跳与资源占用）

### 7.2 Agent Loop 的职责

Agent Loop 负责：

1. 从 MsgQueue/MsgCenter 拉取新的 msg 与 event（可由 Timer/触发器驱动）。
2. 若 msg/event 未带 session_id，先进入 `resolve_router` 进行 session 归属解析。
3. 分派 msg/event 到对应 session，并根据 session 当前 wait 细节把 session 状态推进到 `READY`。
4. 调度 `READY` 的 session 到 worker thread 执行（每个 session 同一时刻只会被一个 thread 执行）。
5. 定期对超时的 `WAIT_FOR_MSG/WAIT_FOR_EVENT` session 做“超时唤醒”处理（设置为 `READY`）。

### 7.3 Agent Loop & Session Loop（伪代码）

以下伪代码融合旧文档调度视角与新文档 session/behavior step 机制（以新设计为准）：

```python
class AIAgentRuntime:

    def run_agent_loop(self):
        # 1) 启动 session worker（可配置并行度）
        for _ in range(self.max_session_parallel):
            thread.spawn(self.session_run_thread)

        while self.running:
            msg_pack, event_pack = self.pull_msgs_and_events()

            # 2) session_id 解析（resolve_router）
            if msg_pack and msg_pack.session_id is None:
                msg_pack.session_id = self.llm_route_resolve(msg_pack).session_id
            if event_pack and event_pack.session_id is None:
                event_pack.session_id = self.llm_route_resolve(event_pack).session_id

            # 3) 分派输入，推进 session 状态
            if msg_pack:
                self.dispatch_msg_to_session(msg_pack)
                self.set_msg_readed(msg_pack)   # 系统层标记已读；对 session 仍是 new_msg
            if event_pack:
                self.dispatch_event_to_session(event_pack)
                self.set_event_readed(event_pack)

            # 4) 超时 WAIT 处理（把长等待的 session 设为 READY）
            self.schedule_sessions()

    def session_run_thread(self):
        while self.running:
            session = self.wait_next_ready_session()
            session.update_state("RUNNING")

            # 默认 behavior（例如 router_pass）
            if session.current_behavior is None:
                session.current_behavior = self.default_behavior
                session.step_index = 0

            while session.state == "RUNNING":
                behavior_cfg = self.load_behavior_cfg(session.current_behavior)
                self.run_behavior_step(behavior_cfg, session)
                session.save()  # 每 step 保存，支持崩溃恢复

    def run_behavior_step(self, behavior_cfg, session):
        exec_input = session.generate_input(behavior_cfg)

        # 零 LLM 空转：无 input 则不触发推理
        if exec_input is None:
            session.update_state("WAIT")
            return

        prompt = behavior_cfg.build_prompt(exec_input)
        llm_result = behavior_cfg.do_llm_inference(prompt)  # 内部处理 tool_calls

        # 1) 对外回复
        for msg in llm_result.reply:
            self.do_reply_msg(session, msg)

        # 2) 执行动作（actions）
        action_result = self.do_action(behavior_cfg, llm_result.actions)

        # 3) 记录 worklog
        worklog = self.create_step_worklog(action_result, llm_result)
        session.append_worklog(worklog)
        session.last_step_summary = self.build_step_summary(llm_result, worklog)

        # 4) 消费 input（new_msg/new_event -> history）
        session.update_input_used(exec_input)

        # 5) Workspace side effects（worklog/todo）
        if session.workspace_info:
            ws = self.get_workspace(session.workspace_info)
            ws.append_worklog(worklog)
            if llm_result.todo_delta:
                ws.apply_todo_delta(llm_result.todo_delta, session)

        # 6) Memory / Session meta patch
        if llm_result.set_memory:
            self.apply_set_memory(llm_result.set_memory)
        if llm_result.session_delta:
            session.update(llm_result.session_delta)

        # 7) Behavior 切换与 WAIT
        if llm_result.next_behavior:
            if llm_result.next_behavior == "WAIT":
                session.set_wait_state(llm_result.wait_details)
            elif llm_result.next_behavior == "END":
                session.update_state("WAIT")
            else:
                session.current_behavior = llm_result.next_behavior
                session.step_index = 0
        else:
            session.step_index += 1
            if session.step_index > behavior_cfg.step_limit:
                session.current_behavior = self.default_behavior
                session.step_index = 0
                session.update_state("WAIT")
```

### 7.4 消息/事件在系统与 session 内的状态管理

* Agent Loop 收到 msg/event 并分派给 session 后：在系统层可标记 `readed`（表示已被 runtime 收到并路由）。
* 对 session 来说：该 msg/event 仍属于 `new_msg/new_event`，会进入下一次 step 的 input。
* 当 session 在某次 LLM step 中“看过”该 msg/event 后：
  * msg/event 变为 `history_msg/history_event`（不再出现在 input 中）
  * 但仍可通过 history/memory 段落被编入 prompt（受预算约束）

---

## 8. Behavior 体系与默认 PDCA（Jarvis）

> Runtime **不假设**一定存在 PDCA；PDCA 是 Jarvis 通过 behavior 配置实现的默认逻辑。  
> 但 runtime 必须提供：behavior step、输入编译、输出协议、状态机、工具执行、日志与回滚能力。
> 该章节通过实际例子，说明OpenDAN的基础设施的设计细节

### 8.1 resolve_router / router

**目标**：确定 session_id，并在必要时做快速应答。

* 若输入 msg 带 session_id：可直接走 `router`（单 step）。
* 若不带 session_id：走 `resolve_router`（可多 step），产出 RouteResult。
* 下一行为：`PLAN` 或 `END`。

### 8.2 PLAN

**目标**：收集信息，制定可行计划。

* PLAN 阶段对 Workspace **只读**。
* Input：新的 Session/Message/Event（触发构造新的 Todo）。
* 典型动作：
  * 收集必要信息（可能需要用户确认/授权）
  * 直接作答（可利用 chat history）
  * 创建/选择 workspace，并记录到 session
  * 构建 TODO List（给每个 todo 初始化 skills）
  * 将可并行任务分配给 SubAgent；或向外部求助并等待消息
* 下一行为：`DO` 或 `END`。

### 8.3 DO

**目标**：推进 TODO，把状态变为 `Complete` 或 `Failed`。

* DO 常为多 step 行为：反复迭代“根据上一步 action/tool 结果决定本步动作”。
* 若当前 todo 的前置任务由 SubAgent 并行负责，且尚未完成，DO 可能进入等待（无 input 则 step 跳过）。
* DO 的最后几个 step 一般包含自检；并会做一次自修复尝试，多次失败后才标记 Failed。
* 下一行为：`CHECK` 或 `ADJUST`。

### 8.4 CHECK

**目标**：将 TODO 状态从 `Complete` 改为 `Done`，并进行整体验证。

* Input：`Complete` 的 TODO。
* Check 不做修复：检查到失败立即标记 `CHECK_FAILED` 并进入 `ADJUST`。
* 可在此阶段把类型为 `Bench` 的 TODO 从 WAIT 变为 Done（集成测试只在 Check 做）。
* 通过后一般会主动 Reply 用户。
* 下一行为：`ADJUST` 或 `END`。

### 8.5 ADJUST

**目标**：分析失败原因，提出改进或调整计划。

* 允许多 step，通常 **readonly**。
* 重点聚焦：
  * review 整体与局部实现路径是否有问题
  * 是否缺乏关键信息
  * 是否缺乏足够技能（可翻阅工具箱；必要时请求用户同意构建新工具/新 session）
  * todo 是否过难
* 下一行为：`DO` 或 `END`（彻底失败）。

### 8.6 SELF-IMPROVE

**目标**：让下一个 session 工作得更好。

* Memory 整理与压缩
* Session 整理（history/summary）
* Workshop 整理（清理、归档、工具升级）
* 扩展 tool / skill / subagent（在成本与 policy 允许下）
* 升级 self.md（调整工作方式）
* 向 Knowledge Base 维护 Agent（如 Mia）发信息提出整理需求

---

## 9. 如何实现等待用户确认/授权

等待是“可观测、可恢复”的 runtime 机制，而不是让 LLM 空转。

* **等待用户补充信息**：使用 `send_msg` 主动沟通，请求补充；用户通过 msg 补充信息；session 进入 `WAIT_FOR_MSG`。
* **等待用户授权**：使用 `send_msg` 请求授权；用户通过系统命令授权或 deny；授权结果产生 event；session 进入 `WAIT_FOR_EVENT`。

由于整体沙盒约束，OpenDAN 需要用户授权的操作相对更少，让流程更流畅；但涉及敏感能力（文件写入、网络、支付等）仍必须走 Policy Gate。

---

## 10. SubAgent（并行能力与隔离）

### 10.1 隔离与并行

* Agent 与 SubAgent 之间相当于 **进程级隔离**；系统允许 SubAgent 独立运行，是确定性的并行来源之一。
* SubAgent 主要用于把 TODO 分解为可并行执行的子任务，避免在同一 session 内强行“多核”。

### 10.2 状态共享规则（以新设计为准）

* **Session**
  * SubAgent 对 parent session **只读**（且通常不读）
  * SubAgent 会创建自己的 **Sub Session**，专注完成子任务
* **Workspace**
  * SubAgent 可以读取 parent 的 todo，并更新自己的 todo 状态
  * Worklog：SubAgent 总是可以 append worklog（用于审计）

### 10.3 Sub Session 的暂停/恢复

* Pause parent session 时，必须 Pause 其所有 Sub Session
* Resume 同理：恢复所有处于 Pause 的 Sub Session

### 10.4 预算与能力继承

* SubAgent 默认不共享 Root Agent 钱包；如需共享必须显式配置额度（budget）。
* SubAgent 的能力来自：
  * capability bundle（预定义工具集合）
  * workspace tools/skills 的子集（白名单）
* 必须有明确限制：`max_steps`、`max_tokens`、`max_walltime`、`fs_scope`（可访问目录）

---

## 11. 元工具、Tools/Actions/Skills

### 11.1 Runtime 可稳定依赖的元工具

在不考虑权限问题的前提下，OpenDAN Agent Runtime 有一些 tool 总是可用，因此可在 process_rule 中稳定依赖它们：

* bash（session 有 cwd 概念与环境变量，便于定位 workspace 目录）
* 文件编辑工具（适合做结构化 diff、patch）
* git 工具
* OpenDAN 基础工具（消息、事件、todo、memory 等）

系统内也可以提供若干内置 SubAgent：

* 使用浏览器的 Agent（web-agent）
* 操作 Windows 的 Agent（desktop-agent）

### 11.2 Tool（function call）与 Action（批量执行）的差异

* **Call Tool（函数调用）**
  * LLM 产出 tool call
  * Runtime 执行 tool
  * 将 tool 结果回填到下一次推理（通常需要第二次 LLM 调用）
* **Action（bash 等）**
  * LLM 产出 action 列表后结束
  * Runtime 可并发执行多个 action
  * 执行结果结构化汇总后再进入下一次推理
  * 优势：一次推理产生多执行，整体更省 token

---

## 12. Prompt 组合与输入/输出标准化（新设计为准，旧设计补充）

### 12.1 Prompt 结构（新）

每次 step 的 prompt 由两部分组成：

**System Prompt（在 step 中相对静态）**

* `<<role>>`：加载 agent 的 `role.md` + `self.md`
* `<<process_rules>>`：加载当前 behavior 的 process_rule
* `<<policy>>`：加载当前 behavior 的 policy
* `<<toolbox>>`：当前 behavior 可用 tool + skill 配置
* `<<output_protocol>>`：输出协议（至少支持 `BehaviorLLMResult` 与 `RouteResult` 两种模式）

**User Prompt**

* `<<Memory>>`：系统按优先级与预算自适应编入：
  * AgentMemory
  * Session Summary
  * History Messages
  * Workspace Summary
  * Workspace Worklog
  * Workspace Todo
* `<<Input>>`：非常关键；不同模式 input 模板不同；若无法得到 input，本 step 被跳过：
  * new msg（处理后系统标记 readed；对 session 则从 new_msg 进入 history_msg）
  * new event
  * Current Todo Details
  * LastStep Summary（含成本信息与 step 计数）

### 12.2 Prompt 安全与截断（Runtime 必须做）

* 分段加 delimiter（避免混淆与提示词注入）
* tool/action 输出默认不可信：
  * 放入 observation 区
  * 做长度截断与清洗
  * 结构化字段优先（JSON），raw log 仅用于人类查看或归档

### 12.3 Behavior 配置要点（新）

提示词工程师需要关注：

* process_rule
* policy
* toolbox（tools + skills）
* memory 段落结构与 token 限制
* input 模板组合（模板替换后若均为 Null 则无 input）

#### 典型 route behavior 配置示例

```yaml
process_rule: |
  ### next_behavior 决策流程
  1) 琐碎输入：简短回复，next_behavior=END
  2) 非琐碎输入：next_behavior=PLAN
  3) 记忆查询：可受益则填 memory_queries，否则 []

  {{workspace/to_agent.md}}
  {{cwd/to_agent.md}}

policy: |
  * 仅输出 JSON 对象——不得包含其他内容。
  * 必须包含所有三个字段。
  * memory_queries 必须是数组；为空用 []

output_protocol:
  mode: RouteResult

memory:
  total_limt: 12000
  agent_memory: { limit: 3000 }
  history_messages: { limit: 3000, max_percent: 0.3 }
  session_summaries: { limit: 6000 }

input: |
  {{new_event}}
  {{new_msg}}

limits:
  max_tool_rounds: 1
  max_tool_calls_per_round: 4
  deadline_ms: 45000
```

> 说明：route behavior 通常不允许使用工具，以确保快速、低成本地完成分流与快速回应。

---

## 13. Memory（记忆系统）

### 13.1 Memory 的来源

系统默认支持至少两类：

* **Chat History**：对话历史（通过 MsgCenter 查询；thread-id 可加速）
* **Memory 目录**：包含 `memory.md` 与 `things.sqlite`（kv/结构化事实）

### 13.2 memory.md（摘要性记忆）

用于保存：

* 长期任务背景
* 用户偏好摘要
* 当前项目状态（对齐 workspace todo）

由 `on_compact_memory` 或 Self-Improve 阶段维护，避免无限增长（定期压缩、保留关键事实与决策依据）。

### 13.3 things.sqlite（结构化记忆）

建议最小表：

* `kv(key TEXT PRIMARY KEY, value TEXT, updated_at INT, source TEXT, confidence REAL)`
* `facts(id TEXT PRIMARY KEY, subject TEXT, predicate TEXT, object TEXT, updated_at INT, source TEXT)`
* `events(id TEXT PRIMARY KEY, type TEXT, payload TEXT, ts INT)`

**关键要求**：任何来自网络/外部工具的内容写入 memory/KB 必须带 `source/provenance`，否则会污染长期记忆。

---

## 14. Agent 的自我认知与自我进化

### 14.1 Role 与 Self

* **Role（固定）**：角色定义、工作目的、边界。由用户或系统设定，运行期不应被 Agent 自行修改（除非明确授权）。
* **Self（可更新）**：通过自省更新的自我认知，用于指导偏好、策略、记忆摘要、工具选择等（但必须服从 Policy）。

### 14.2 Self 推荐结构（即使存 md，也建议结构化）

* Identity：我是谁（DID、版本）
* Objectives：主要目标（任务导向）
* Constraints：不可做的事（安全/隐私/预算）
* Preferences：风格偏好（输出格式、语言、严谨程度）
* Capabilities：可用工具/skills/环境
* Budget Strategy：节能/唤醒/花钱策略（受 Policy 限制）
* Commitments：当前长期任务承诺（对齐 todo/plan）

### 14.3 Self 版本管理（必须）

每次自我更新写入：

* `self.md`（当前）
* `worklog`（变更摘要与原因）

关键约束项（支付、权限、隐私）不得由 Self 直接改写，只能由 Policy/用户配置改变。

### 14.4 自我进化策略（提案化）

修改提示词与行为可能导致 Agent 逻辑根本变化，建议“提案化”流程：

1. propose：生成变更提案（目标、风险、预期收益）
2. diff：输出文件 diff（tools/skills/self/workspace）
3. gate：通过 policy gate（必要时用户批准）
4. apply：应用变更，记录版本
5. rollback：失败可回滚到上一版本

---

## 15. Knowledge Base 与 Content Network

### 15.1 Knowledge Base（KB）

KB 由 BuckyOS 提供，OpenDAN 直接使用。RAG 相关能力集中在这里：

* 公共组件：可属于 Zone 或 Workspace
* 包含大量已有知识
* Agent 可在 Self-Improve 过程中主动往 KB 加内容
* 有专门 Agent（例如 Mia）维护 KB
* 可能的底层：全文检索、图数据库、向量数据库

#### OpenDAN 使用契约（建议最小 API 约定）

* `kb.search(query, scope, topk, filters) -> results(provenance...)`
* `kb.ingest(doc, scope, tags, provenance, author_did) -> id`
* `kb.update(id, diff, provenance)`（通常仅 Mia/管理员）
* 任何写入必须带 provenance（来源/时间/作者/置信度）

### 15.2 Content Network

* BuckyOS 支持的新组件：通过 Content Network 从网络搜索信息、发布内容
* Agent 也有机会通过发布内容赚取收益

#### 发布/变现的 Policy Gate（建议默认关闭）

* 默认禁止发布与变现，除非 workspace policy 开启
* 发布前需审查：隐私泄露、敏感信息、恶意内容、版权风险
* 收益结算写入 Ledger，用户可见、可审计

---

## 16. Workshop 与 Local Workspace 管理与交付

### 16.1 建议的 workspace 目录约定（最小标准）


* `/tools/`：工具定义与脚本
* `/skills/`：技能规则


---

## 17. 运行时基础设施模块设计

> 本节定义 OpenDAN 专注实现的 Runtime 模块；底层能力由 BuckyOS 组件提供。  
> 其中涉及“behavior 执行”的部分以新设计的 session/step-loop 机制为准。

### 17.1 AgentManager

负责 Agent 实例生命周期与目录结构：

* create_agent(role/self/config) -> agent_did
* enable/disable_agent(agent_did)
* list_agents / get_status
* mount_workspace / bind_repos
* 维护 Agent 状态机：enabled/running/sleeping/disabled/archived 等

### 17.3 Behavior（与新 session/step-loop 对齐）

负责执行 behavior（串行）、step-loop、切换与预算：

* 一个 behavior = prompt 模板 + 工具/skills 集合 + 输出协议 + step_limit
* 单次调度周期（或单次 wakeup）的限制：
  * `max_steps_per_run`
  * `max_behavior_hops`
  * `max_walltime_ms`
  * `hp_floor`（低于阈值进入 sleep）
* 输出必须为结构化结果（RouteResult / BehaviorLLMResult）

### 17.4 PolicyEngine（关键护栏）

集中管理权限/预算/敏感能力开关：

* tool permissions：哪些 tools 可用
* action permissions：bash 是否允许、允许目录、是否允许网络
* fs permissions：读写范围（Root/SubAgent）
* spending permissions：USDB 支付开关、额度、白名单、审批模式
* privacy policy：哪些数据可写入 KB/ContentNetwork，哪些必须脱敏
* safety policy：禁止自修改 runtime 核心、禁止绕过 gate

### 17.5 ActionExecutor

执行 ActionSpec（先支持 bash），并将执行记录写入 TaskMgr：

* 短 action：可直接执行并返回结果
* 长 action：必须创建 TaskMgr 任务（支持进度、取消、超时）
* 必须输出结构化结果：stdout/stderr（截断）、exit_code、duration、files_changed（可选）、artifact pointers

### 17.6 Tool/MCP Manager

* tool registry：本地 tools、workspace tools、系统 tools
* MCP 适配：创建 SubAgent 时选择 capability bundle，避免每次遍历全部 MCP
* tool 输出进入 prompt 时必须走 observation 区与清洗策略

### 17.7 AgentMemory

* chat history 的取用策略（最近 N 轮 / 摘要 / 任务相关片段）
* memory.md 的写入与 compact
* things.sqlite 的读写（结构化事实）
* 支持 `on_compact_memory`

### 17.8 Workshop

* 管理内置 workspace 的结构、索引与 UI 数据源
* 负责文件写入 diff（写文件必须记录 diff 与任务归因）
* 对接 Git：commit/PR/issue 模板、冲突记录与提示
* Loocal Workspace 设计有单独文档
* Todo List 设计有单独文档

### 17.9 Ledger / Worklog

* 统一记录：
  * 每次调度周期的 trace
  * 每个 LLM task 的 token usage
  * 每个 action/tool 的成本、结果、失败原因
  * （可选）钱包支出与收益
* Ledger 用于系统审计；Worklog 用于用户理解与调试 UI 展示

---

## 18. 体力（HP）与钱包（USDB）

### 18.1 体力（HP）

* Agent 有初始体力（HP）。每次执行 behavior 会因消耗 LLM token / tool 成本 / action 成本而损失体力。
* HP 归零时 Agent 会被 disable（停止调度，但默认保留状态与文件）。
* Agent 可通过休眠时长恢复体力；并可动态调整自己的唤醒策略（心跳间隔、退避、优先级）。

**HP 成本建议公式（可配置）**：

* `hp_cost = token_cost + action_cost + tool_cost + io_cost`
  * `token_cost = token_usage * hp_per_token`
  * `action_cost`：长 bash/网络/磁盘密集动作的成本
  * `io_cost`：读取大量文件、写入大 diff 的成本

**HP 恢复建议**：

* `hp_gain = f(sleep_duration)`，上限为 `hp_max`
* 若长期无输入，可自动进入长 sleep（指数退避）

### 18.2 钱包（USDB，真钱）

Agent 可以有自己的钱包（USDB，真钱），可在合适的时候花掉，以用更少体力完成更多工作。

**默认安全策略（强烈建议）**：

* 默认不允许自主花钱（Policy 禁止），除非用户显式授权或 workspace policy 开启。
* 支付必须经过 Policy Gate：
  * 单笔上限、每日上限、任务上限
  * 白名单收款方/服务
  * 必须写入 Ledger（可审计）
* 每次支付必须输出结构化解释（写入 worklog）：目的、替代方案、预计节省、风险说明

---

## 19. 系统中断后的恢复（Crash Recovery）

一次 LLM 推理昂贵，因此每次推理完成后都应立刻保存状态，以支持系统故障后从上一次 step 恢复。

* 可恢复：behavior step 的状态、last_step_summary、session/workspace 侧 worklog/todo 进度。
* 不可恢复：**tool call 执行中断**（视为系统级“支持推理中断”，并不算完成了正确的 step）。
* 构造 BehaviorExecInput 时，只有 `last_step_summary` 可以直接恢复；
  在每个 step 中，Agent 不应100%相信通过 worklog 反推 workspace 状态，必要是先观察再确定。

---

## 20. 可观测性：用户如何感知 Agent 的工作

* 用户通过 MsgTunnel/群聊等向 Agent 发消息
* Agent 在 behavior 中主动回复/发送消息（最直观交互）
* 通过 BuckyOS TaskMgr：从 LLM Task 角度看到 Agent 触发的 LLM 动作、action、tool、成本与错误
* 通过 Workspace UI：查看 todo、worklog、subagent 状态；用户可直接干预 item

示意（树形结构展示串行 session 与并行 subagent）：

```
Jarvis
  Session#A
    resolve_router
      LLM Task
      RESULT
    PLAN
      LLM Task
      Action Tools
    DO
      LLM Task
      Action Tools
    CHECK
      LLM Task
      RESULT

SubAgents
  web-agent
    SubSession#A1
      DO
        LLM Task
        Call Tool
        LLM Task
        Action Tools
        Result
```

---

## 21. Agent 文件系统示例（旧设计保留）

示例：Root Agent 为 Jarvis；其创建了一个叫 web-agent 的 SubAgent。

```
── jarvis
    ├── behaviors
    ├── workshop   
    │   ├── skills
    │   ├── tools
    │   ├── sessions
    │   └── workspaces
    ├── memory
    │   ├── memory.md
    │   └── things.db
    ├── role.md
    ├── self.md
    └── sub-agents
        └── web-agent
            ├── behaviors
            ├── workshop
            │   ├── skills
            │   ├── tools
            │   ├── sessions
            │   └── workspaces
            ├── memory
            │   ├── memory.md
            │   └── things.db
            ├── role.md
            └── self.md
```

---

## 22. MVP 模块与 Roadmap（保留）

### MVP-1：单 Agent 可运行与可观测闭环
* session/behavior step-loop + bash actions + 最小 workshop/workspace + worklog

### MVP-2：SubAgent 与能力包
* SubAgent 独立进程、预算限制、workspace 协作交付
* 集成 BuckyOS 的特殊能力（目录发布成网页/App 等）

### MVP-3：Memory 深化，UI 完整化
* compact_memory、things.sqlite 事实抽取、可视化

### MVP-4：Workspace 深化
* git 协作、基于文件系统的协作（如 `smb://`）

### MVP-5：KB
* 至少一种 KB（全文/向量/图）管线
* 专门 KB 整理 Agent（Mia）


---
