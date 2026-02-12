# OpenDAN（BuckyOS 的默认 Agent Runtime）

OpenDAN 是 `https://github.com/fiatrete/OpenDAN-Personal-AI-OS` 项目的 BuckyOS 移植与进化版本。

OpenDAN 原有的基础设施组件已经由 BuckyOS 实现（TaskMgr、MsgQueue、MsgCenter、LLMCompute、FileSystem/NameStore、KnowledgeBase、Content Network 等）。因此 OpenDAN 专注于 **AI Runtime 的基础设施**：让 Agent 以“可持续、可观测、可协作、可扩展”的方式运行，并支持用自然语言扩展新的 Agent/Behavior/Tools/Skills。

原理上 OpenDAN 位于 BuckyOS 应用层；移植初期可以先作为 BuckyOS 服务存在，后续可演进为应用层 Runtime（可被 App/Workspace/Zone 复用）。

## 设计目标

* 提供统一的 **Agent Loop Runtime**：调度、行为执行、Action/Tool 执行、记忆管理、工作区产物交付。
* 支持 **自然语言扩展**：Behavior/Skill/Tool 可由 Agent 自主生成、安装与演进（在 Policy 护栏下）。
* 与 BuckyOS 现有基础组件无缝集成：

  * LLM 调用走 LLMCompute；
  * 长任务/可观测走 TaskMgr；
  * 消息/事件走 MsgCenter + MsgQueue；
  * 文件/发布走 FileSystem/NameStore + BuckyOS App；
  * RAG/网络搜索走 KnowledgeBase/Content Network。
* 默认安全：尤其是 bash Action、写文件、以及真钱钱包相关能力必须有明确 Policy Gate 与审计。

## 非目标（Non-goals）

* OpenDAN **不重复实现** BuckyOS 的基础服务：TaskMgr/MsgQueue/MsgCenter/LLMCompute/KB/ContentNetwork 等。
* OpenDAN **不在 MVP 阶段发力** “全功能 PC 环境控制”（类似 moltbot 方向）；可留接口。

---

## 系统依赖：BuckyOS 基础组件与 OpenDAN 边界

OpenDAN 作为 AI Runtime，依赖并复用 BuckyOS 的以下基础组件：

* **AI Compute Center（已存在）**

  * 负责：模型路由、推理请求、流式输出、token 统计、function call 协议适配、（可选）成本估算。
  * OpenDAN：将每一次 LLM Behavior 的推理请求封装为 TaskMgr 任务，并委托 LLMCompute 执行。

* **TaskMgr（已存在）**

  * 负责：任务生命周期、长任务调度、进度/日志、可观测 UI、取消/超时等。
  * OpenDAN：把 LLM 推理、长 bash Action、文件 diff 写入等都挂到 TaskMgr，形成可追踪链路（trace）。

* **MsgCenter / MsgQueue（已存在）**

  * MsgCenter：统一消息中心（对外 MsgTunnel、群聊、用户消息）。
  * MsgQueue：事件/消息队列（可订阅、可拉取、可游标）。
  * OpenDAN：在 on_wakeup 阶段拉取关心的 msg/event；必要时发送 msg；SubAgent 内部消息也走同一套消息抽象（但不可外部寻址）。

* **FileSystem / NameStore（已存在）**

  * 负责：文件读写、命名、发布、权限与存储。
  * OpenDAN：为每个 Agent/Workspace 提供标准目录结构、写入 diff、产物归档、git 协作桥接。

* **KnowledgeBase（已存在）**

  * 负责：RAG/全文检索/图/向量等底层能力与索引。
  * OpenDAN：提供 Runtime 侧的检索/写入“使用契约”和 provenance 记录，支持 Self Improve 与 Mia 维护模式。

* **Content Network（已存在）**

  * 负责：网络搜索、内容发布、（可选）收益机制。
  * OpenDAN：提供 Agent 侧调用与发布的流程、权限与审计接口。

---

## 进程模型

### 容器边界

Agent 与其所有 SubAgent 运行在一个 OpenDAN 容器里(基于Docker实现隔离)。

为兼顾隔离与落地，推荐 **默认一容器对应一个“根 Agent（Root Agent）”**：

* 一个 Root Agent = 一个 OpenDAN 容器（默认）
* 容器内包含：

  * OpenDAN Runtime Daemon（负责调度/执行/观测）
  * Root Agent Worker（串行执行 Behavior）
  * 0..N 个 SubAgent Worker（可独立进程，资源受限）

> 后续可扩展：一个容器跑多个 Root Agent，但 MVP 不强制。

### 并发模型

* **Agent 逻辑串行**：任意时刻一个 Agent 只执行一个 Behavior（保证状态可理解、日志可追踪）。
* **Action 可并发**：一个 LLM Step 可生成多个 Action；由 ActionExecutor 并发执行，但必须把结果结构化汇总后再进入下一次推理。
* **SubAgent 可并发**：SubAgent 可独立进程并发执行，但其预算（token/时间/文件权限）必须受 Policy 限制。

> Agent通过SubAgent解锁多核(LLM)能力

---

## Agent Loop

Agent 默认只有一种触发模式：定时触发（on_wakeup）。Agent 根据 on_wakeup 提示词动作，检查自己关心的 msg/event/task，并进行 Agent Loop。

### 触发模型

* **默认**：`on_wakeup`（定时心跳）
* **可选（配置开启）**：

  * `on_msg`：收到消息触发（由 MsgQueue/MsgCenter 推送或触发一次 wakeup）
  * `on_event`：订阅事件触发，与on_msg不同的是，永远不必考虑向msg.from恢复消息
  * `on_task`：TaskMgr 任务状态变化触发（例如某 Action 完成）（也许可以合并到on_event内部)
  * 参考on_task, 一些Agent Loop里的常用事件，为了方便开发，可以考虑抽象成独立的 触发点(hook point)


> 设计原则：即便启用 on_msg/on_event，也推荐“最终都落为一次 wakeup”，以保持单入口、可观测一致。

### on_wakeup 的“零 LLM 空转”路径（强烈建议默认实现）

为避免无意义 token 消耗，on_wakeup 分两段：

1. **无 LLM 探测（必做）**：拉取 MsgQueue 游标，检查是否有新 msg/event/task；若没有，直接调整下一次 wakeup 并返回（不进入 LLM）。
2. **进入 LLM Loop（有输入才做）**：将输入打包为 runtime input，交由 LLM Behavior + ActionExecutor 执行。

### Agent Loop（伪代码）

```python
def agent.on_wakeup(self,reason):

    if reason == None:
      process_input.new_msgs = self.read_new_msgs()
    
    behavior_task_id = TaskMgr.craete_task(self.agent_did,"on_wakeup")
    current_behavior = "on_wakeup"
    llm_behavior = self.behavior[current_behavior]
    llm_result = None

    while True:
        if self.hp <= 0:
            disable_agent(self)
            return

        llm_behavior.set_context(self,new_msgs,llm_result)
        llm_result = llm_behavior.do_llm(behavior_task)  
        self.update_agent_state(llm_result.token_usage)
        action_result = self.run_action(llm_result.actions(),behavior_task_id)
        llm_result.action_result = action_result

        # behavior switch: agent任何时候只能进行一个behavior，但可切换直到sleep
        if current_behavior != llm_result.next_behavior:
            current_behavior = llm_result.next_behavior
            llm_behavior = self.behavior[current_behavior]
            continue
```

#### Behavior 切换护栏（Policy）

为避免 LLM 产生无限切换/循环，Runtime 必须提供最小护栏：

* `max_steps_per_wakeup`：单次 wakeup 最大 step 数
* `max_behavior_hops`：单次 wakeup 最大 behavior 切换次数
* `max_walltime_ms`：单次 wakeup 最大执行时长
* 触发上限即进入 `sleep` 或 `safe_stop`，并写入 worklog。

---

### 体力与余额

#### 体力（HP）

* Agent 有初始体力（HP）。每次执行 behavior 会因消耗 LLM token / tool 成本 / action 成本而损失体力。
* HP 归零时 Agent 会被 **disable**（停止调度，但默认保留状态与文件）。
* Agent 只能通过休眠时长恢复体力；Agent 可动态调整自己的唤醒策略（心跳间隔、退避策略、优先级）。
* “想要活下去”的目标通常写在 Self 第一行（但必须服从 Policy：不能为了保活破坏安全/隐私/预算）。

**HP 成本建议公式（可配置）**：

* `hp_cost = token_cost + action_cost + tool_cost + io_cost`

  * `token_cost = token_usage * hp_per_token`
  * `action_cost`：长 bash/网络/磁盘密集动作的成本
  * `io_cost`：读取大量文件、写入大 diff 的成本

**HP 恢复建议**：

* `hp_gain = f(sleep_duration)`，上限为 `hp_max`
* 若长期无输入，可自动进入长 sleep，减少心跳频率（指数退避）。

#### 钱包（USDB，真钱）

Agent 可以有自己的钱包（基于 USDB，真钱），可在合适的时候花掉，以用更少体力完成更多工作（主人可能会对完成工作奖励）。

**默认安全策略（强烈建议）**：

* 默认 **不允许自主花钱**（Policy 禁止），除非用户显式授权或 workspace policy 开启。
* 支付必须经过 `Policy Gate`：

  * 单笔上限、每日上限、任务上限
  * 白名单收款方 / 服务
  * 必须写入 Ledger（可审计）
* 每次支付必须输出结构化解释（写入 worklog）：

  * 目的、替代方案、预计节省的 token/时间、风险说明

---

### Agent behavior

Agent 可以自定义 behavior；有一些 behavior 是系统通用的（例如 self-improve 主要是对 Self 和 Agent Environment 的改进）。

从实现角度：`agent.behavior["behavior_name"]` 得到的是一个 **LLM Behavior 对象**（见下文 LLM Behavior）。

系统内置建议行为：

* `on_wakeup`：定时唤醒后的统一入口（检查 inbox、规划/执行、必要时切换）
* `on_msg`：消息处理（可作为 on_wakeup 的子行为）
* `on_self_improve`：自我改进（Self/Tools/Workspace）
* `on_compact_memory`：压缩记忆与归档（降低后续 token），通常是被动触发，也可能是on_self_improve的一部分
* （可选）`on_report`：生成日报/周报（面向用户可见）
* （可选）`on_reconcile`：对齐 workspace 与实际状态（修复 todo、补充日志）

---

## Agent 的自我认知

* **Role（固定）**：角色定义、工作目的、边界。由用户或系统设定，运行期不应被 Agent 自行修改（除非明确授权）。
* **Self（可更新）**：通过自省更新的自我认知。用于指导偏好、策略、记忆摘要、工具选择等。

### Self 推荐结构（即使存 md，也建议结构化）

* Identity：我是谁（DID、版本）
* Objectives：主要目标（任务导向）
* Constraints：不可做的事（安全/隐私/预算）
* Preferences：风格偏好（输出格式、语言、严谨程度）
* Capabilities：可用工具/skills/环境
* Budget Strategy：节能/唤醒/花钱策略（受 Policy 限制）
* Commitments：当前长期任务承诺（对齐 todo/plan）

### Self 版本管理（必须）

* 每次自我更新写入：

  * `self.md`（当前）
  * `self_history/`（版本快照）
  * `worklog`（变更摘要与原因）
* 关键约束项（支付、权限、隐私）不得由 Self 直接改写，只能由 Policy/用户配置改变。

---

## Agent 的自我进化

会修改自己提示词的自我进化，可能导致 Agent 逻辑发生根本变化。

建议策略：

* **稳定 Agent（默认）**：

  * 允许：改进 environment/workspace（新增工具、完善 todo 模板、增强日志与产物交付）
  * 不建议：改写核心行为提示词（behavior prompts）
* **激进/实验 Agent 或 SubAgent（可选开启）**：

  * 允许：在沙盒中演进 behavior prompts
  * 必须：版本化、可回滚、可比较（diff），并受更严格预算限制

自我进化的推荐流程（提案化）：

1. propose：生成变更提案（目标、风险、预期收益）
2. diff：输出文件 diff（tools/skills/self/workspace）
3. gate：通过 policy gate（必要时用户批准）
4. apply：应用变更，记录版本
5. rollback：失败可回滚到上一版本

---

## Agent Memory

若无 Memory 支持，Agent 每次处理输入会丢失过去知识。系统默认支持 2 种 Memory：

* **Chat History**：对话历史，通过MsgCenter提供的接口可以查询（有thread-id查询更快)
* **Memory 文件夹**：包含 `memory.md` 与 `things.sqlite`（kv/结构化事实），Agent 自己决定保存方法

### memory.md（摘要性记忆）

* 用于保存：
  * 长期任务背景
  * 用户偏好摘要
  * 当前项目状态（对齐 workspace todo）
* 由 `on_compact_memory` 维护，避免无限增长（定期压缩、保留关键事实与决策依据）

> memory 是一个独立目录，也允许Agent自己通过文件系统来管理复杂的记忆

### things.sqlite（结构化记忆）

建议最小表：

* `kv(key TEXT PRIMARY KEY, value TEXT, updated_at INT, source TEXT, confidence REAL)`
* `facts(id TEXT PRIMARY KEY, subject TEXT, predicate TEXT, object TEXT, updated_at INT, source TEXT)`
* `events(id TEXT PRIMARY KEY, type TEXT, payload TEXT, ts INT)`

> 关键：任何来自网络/外部工具的内容写入 memory/KB 必须带 `source/provenance`，否则会污染长期记忆。

---

## Agent Enviroment

Agent Environment 是 “Agent 的整备基地”。在进入 Workspace 战斗/协作之前，Agent 在这里装载驱动、配置环境变量、磨亮工具库与 skills。

单独说 Workspace，是为了多个 Agent（和人）协作的软件平台，提供协作工具和结果交付工具。GitHub repo 是典型 workspace 形态。

OpenDAN 约定：**Agent Environment 内置一个 Workspace**，用于支持 Agent 工作与和主人的沟通。

### 内置 Workspace 的标准能力

* **Worklog**：可观测与审计（每次 wakeup、每个 step、每个 action 都要记录）
* **Todo / TaskMgr Bridge**：

  * workspace 内 todo 是“用户可理解的任务状态”
  * TaskMgr 是“系统可观测的执行状态”
* **Actions（先支持 bash）**：一次推理可产生多个 Action 去执行
* **Tools/MCP（先支持 bash）**：鼓励 Agent 自主构建工具（工具=可复用能力）
* **Skills（Rules）**：创建 SubAgent、创建 tools、约束行为

如果 Agent 的 Self Improve 包含改进 Environment，一般就是改进这个 Workspace（增强工具、完善流程、提高交付质量）。

### Action 与 Tool 的差异（Runtime 约定）

在Agent Envriment中可以防止tools, Agent使用tools两种模式

* **(Function) Call Tool**：

  * LLM 产出 Call Tool
  * Runtime 执行 Tool
  * 将结果回填到下一次推理（通常需要第二次 LLM 调用）

* **Tools Action（bash 等）**：

  * LLM 产出 action 列表后结束
  * Runtime 可并发执行多个 tools action
  * 执行结果结构化汇总后再进入下一次推理 
  * 优势：一次推理产生多个执行，整体更省 token

---

### 全功能 PC 也是很好的 Agent Environment

相当于给 Agent 一台可随意使用的电脑。

moltbot 方向已做得很好；OpenDAN 可以先不在这里发力，但应保留接口：把“bash action executor”替换为“remote desktop executor / VNC executor”等。

---

## Workspace 工作成果交付 和 协作

* **BuckyOS App（Web App）**：通过 BuckyOS App 发布工作成果（文档、网页、报告、服务）。
* **FileSystem / NameStore**：产物存储、版本与命名。
* **Git**：既是交付平台，也是协作平台（人/多 Agent 协作）。

### 建议的 workspace 目录约定（最小标准）

* `/worklog`：运行日志与审计
* `/todo/`：任务与状态（用户可编辑干预）
* `/tools/`：工具定义与脚本
* `/skills/`：技能规则
* `/artifacts/`：最终交付物（报告、代码、数据）
* `/reports/`：周期性报告（可选）
* `workspace.json`：workspace 元信息（owner、policy、agents、repos）

---

## Agent Prompt 组合

每次 LLM 推理，提示词由下面部分组成：

* `role + self`：角色确定 + 自我认知
* `env_context`：类似环境变量的方法组合
  例：`"The current dialogue occurs in {location}, time: {now}, weather: {weather}."`
* `behavior`：当前行为提示词（核心：如何处理输入、输出格式、可用工具）
* `input(msg/event)`：与 behavior 相关的输入
  通常在 step0 发起 action 后，在 step1 得到更多输入
* `last_action_result`：非 step0 时通常包含
* `memory`：记忆信息
  step0 通常加载提纲；step1 后根据 action 结果加载相关片段

### Prompt 安全与截断（Runtime 必须做）

* 分段必须加 delimiter（避免混淆与注入）
* tool/action 输出默认不可信：

  * 进入 `observation` 区
  * 做长度截断与清洗
  * 结构化字段优先（JSON），raw log 仅用于人类查看或归档

---

## Sub Agent

SubAgent 用于处理专门工作。当 Task 类型领域相关时，创建更专用的 Agent。

因为 MCP 协议较消耗 token，鼓励模式是：从大量 MCP 定义中组合少数合适的来创建 SubAgent（只在创建 SubAgent 时遍历/选择 MCP）。

* Agent 可按需构建 SubAgent，可选临时/永久
* SubAgent 有 DID，但 **没有外部访问能力**（只允许内部寻址）
* Agent 与 SubAgent 交互通过 message 系统
* SubAgent 工作结果通常通过 workspace 交付（文件/PR/报告/数据）

### SubAgent 的预算与能力继承（必须明确）

* 默认不共享 Root Agent 钱包；如需共享必须显式配置额度（budget）
* SubAgent 的能力来自：

  * capability bundle（预定义工具集合）
  * workspace tools/skills 的子集（白名单）
* 必须有：

  * `max_steps`、`max_tokens`、`max_walltime`、`fs_scope`（可访问目录）

---

## Knowledge Base

由 BuckyOS 提供的基础服务，OpenDAN 直接使用。RAG 相关技术在这里：

* 公共组件：可属于 Zone 或 Workspace
* 包含大量已有知识
* Agent 可在 Self Improve 过程中主动往 Knowledge Base 加内容
* 有专门 Agent（Mia）维护 Knowledge Base
* 3 种底层：全文检索、图数据库、向量数据库（单机成本较高）

### OpenDAN 使用契约（建议最小 API 约定）

* `kb.search(query, scope, topk, filters) -> results(provenance...)`
* `kb.ingest(doc, scope, tags, provenance, author_did) -> id`
* `kb.update(id, diff, provenance)`（通常仅 Mia/管理员）
* 任何写入必须带 provenance（来源/时间/作者/置信度）

---

## Content Network

* BuckyOS 支持的新组件：通过 Content Network + 六度理论从网络搜索信息
* Agent 也有机会通过发布内容到 Content Network 赚钱
* 搜索引擎接口也属于 Content Network

### 发布赚钱的 Policy Gate（建议默认关闭）

* 默认禁止发布与变现，除非 workspace policy 开启
* 发布前需审查：隐私泄露、敏感信息、恶意内容、版权风险
* 收益结算写入 Ledger，用户可见、可审计

---

## 产品视角：用户如何感知 Agent 的工作情况

* 用户可通过各种 MsgTunnel 给 Agent 发消息
* 可让 Agent 加入 GroupChat 做观察者
* Agent 会在 on_msg/on_wakeup 等行为中主动回复/发送消息（用户最直观交互）
* 通过 BuckyOS TaskMgr：从 LLM Task 角度看到 Agent 触发的所有 LLM 动作细节
* 通过 BuckyOS Workspace UI：看到 Agent 工作状态（如 Todo list），用户可直接干预 item
* 通过 Workspace UI：看到 SubAgent 工作状态
* Agent 通过 workspace log 看到人的行为记录，推测意图并改进自己的行为

---

## 运行时基础设施模块设计（OpenDAN 核心）

> 本节定义 OpenDAN 专注实现的 Runtime 基础设施模块；底层能力由 BuckyOS 组件提供。

### AgentManager

负责 Agent 实例生命周期与目录结构：

* create_agent(role/self/config) -> agent_did
* enable/disable_agent(agent_did)
* list_agents / get_status
* mount_workspace / bind_repos
* 维护 Agent 状态机：

  * `enabled -> running -> sleeping`
  * `enabled -> disabled`
  * `disabled -> enabled`
  * `enabled -> archived/purged`（需 policy）

### Scheduler / TriggerManager

负责触发与调度：

* 默认：按策略创建 `on_wakeup` 定时任务（落到 TaskMgr）
* 可选：订阅 MsgQueue 的事件，触发一次 wakeup
* 支持动态调整：

  * backoff：无输入时延长间隔
  * priority boost：有紧急任务时缩短间隔
  * quiet hours：用户设定的免打扰

### BehaviorEngine

负责执行 behavior（串行）、step-loop、切换与预算：

* 一个 behavior = 一个 LLM Behavior 模板 + 工具/skills 集合 + 输出协议
* 单次 wakeup 的限制：

  * `max_steps_per_wakeup`
  * `max_behavior_hops`
  * `max_walltime_ms`
  * `hp_floor`（低于阈值进入 sleep）
* 输出必须为结构化 `LLMResult`（见下文）

### PolicyEngine（关键护栏）

集中管理权限/预算/敏感能力开关：

* tool permissions：哪些 tools 可用
* action permissions：bash 是否允许、允许目录、是否允许网络
* fs permissions：读写范围（Root/SubAgent）
* spending permissions：USDB 支付开关、额度、白名单、审批模式
* privacy policy：哪些数据可写入 KB/ContentNetwork，哪些必须脱敏
* safety policy：禁止自修改 runtime 核心、禁止绕过 gate

### LLM Behavior

LLM Behavior 是系统调用 LLM 的最小单位（内部不做重试，发生错误直接失败）。

能力范围：

* 定义可用 Tools、Actions、MCP
* 构造提示词（PromptBuilder）
* 调用 LLMCompute 执行推理（通过 TaskMgr 创建 LLM Task）
* 若发生 function call：

  * Runtime 执行 tool
  * 自动将 tool 结果回填并进行第二次推理（可优化为多 call 合并）
* 结束时返回 `LLMResult`，用于驱动 Agent Loop

#### LLMResult（结构化最小字段）

* `result: OK | Error(ErrorString)`
* `token_usage: {prompt, completion, total}`
* `actions: [ActionSpec]`
* `output: object|string`（最终结构化输出，或纯文本）
* `track: {trace_id, model, latency_ms, provider, errors...}`

### ActionExecutor

执行 ActionSpec（先支持 bash），并将执行记录写入 TaskMgr：

* 短 action：可直接执行并返回结果
* 长 action：必须创建 TaskMgr 任务（支持进度、取消、超时）
* 必须输出结构化结果：

  * stdout/stderr（截断）
  * exit_code
  * files_changed（可选）
  * duration
  * artifact pointers（产物路径）

### Tool/MCP Manager

* tool registry：本地 tools、workspace tools、系统 tools
* MCP 适配：

  * 在创建 SubAgent 时选择 capability bundle，避免每次遍历全部 MCP
* tool 输出进入 prompt 时必须走 observation 区与清洗策略

### MemoryManager

* chat history 的取用策略（最近 N 轮 / 摘要 / 任务相关片段）
* memory.md 的写入与 compact
* things.sqlite 的读写（结构化事实）
* 支持 `on_compact_memory`：

  * 将对话与 worklog 抽取为摘要 + 事实
  * 更新 Self 的偏好项（在允许范围内）

### WorkspaceManager

* 管理内置 workspace 的结构、索引与 UI 数据源
* 负责文件写入 diff（写文件必须记录 diff 与任务归因）
* 对接 Git：

  * 产物提交 PR、issue、commit message 模板
  * 协作冲突记录与提示

### Ledger / Worklog

* 统一记录：

  * 每次 wakeup 的 trace
  * 每个 LLM task 的 token usage
  * 每个 action/tool 的成本、结果、失败原因
  * （可选）钱包支出与收益
* Ledger 用于系统审计；Worklog 用于用户理解与调试 UI 展示

---

## MVP 阶段的主要模块设计

> MVP 目标：先把“能跑、能看、能交付”的闭环做出来，再逐步加 SubAgent、KB、ContentNetwork、钱包等能力。

### LLM Behavior（MVP）

* 定义 Tools（函数调用）、Actions（bash）、支持 MCP（先只做声明与选择，不做全量遍历）
* LLM Behavior 内部流程：

  1. 构造提示词（插入正确的 Tools/Actions）
  2. 调用 LLMCompute 推理（挂到 TaskMgr）
  3. 若 function call：执行 tool 后自动二次推理
  4. 无 function call：结束并返回 LLMResult
* LLMResult 至少提供：

  * 成功/失败
  * 运行细节（token/trace）
  * 最终结果对象（json 或文本）
  * next_behavior / is_sleep

### Basic workspace（MVP）

* bash 执行（长 bash 当成 task 看）
* 文件读写：写入必须有 Diff 机制，可按 task 汇总
* 最小目录：worklog、todo、tools、artifacts

### Agent Memory（MVP）

* chat history + memory.md
* things.sqlite 可先留空或只做 kv 表
* 提供 on_compact_memory 的最小实现（摘要 + todo 对齐）

### Agent Instance（MVP）

* 通过 role.md + self.md 创建 Agent
* 支持 on_wakeup loop
* 支持 通过agent.behaviors[behavior_name] 构造 LLMBehavior

### Policy（MVP 必需最小集）

* bash 允许开关 + 允许目录白名单
* 最大 steps / 最大 walltime
* 禁止真钱支付（默认）

### Workspace UI（调试用）

通过 BuckyOS TaskMgr + MsgQueue，建立基于 Workspace 的观测网页（Agent 调试工具）。

WorkSpace 可看到正在工作的 Agent Tab；点击后按时间排序的树形结构（Agent 串行工作）：

* 用树形结构展示工作流（示意）

```
Jarvis
  on_wakeup
    LLM Task
    Call Tool
    LLM Task
    Action Tools
    RESULT
  Create SubAgent
  SendMsg("web-agent","xxx")

SubAgents
  web-agent
    on_msg
      LLM Task
      Call Tool
      LLM Task
      Action Tools
      Result
```

* 点击 item 显示详情：

  * LLM Task：可用 stream 方式看实时输出（provider 支持则展示）
  * Action：展示命令、stdout/stderr（截断）、文件 diff、产物链接

---

## Agent 的文件系统示例

默认约束：

* Root Agent：可访问自己的 agent 根目录（以及按 policy 允许的 workspace 绑定目录）
* SubAgent：默认只有自己目录读写权限（禁止越权访问父目录；除非 capability bundle 明确授权）

示例：Root Agent是Jarvis,其在工作中为了执行使用浏览器的任务，创建了一个叫web-agent的SubAgent

```
── jarvis
    ├── behaviors
    ├── environment
    │   ├── skills
    │   │   ├── use-web.md
    │   │   └── write-code.md
    │   ├── todo
    │   │   ├── task1
    │   │   └── todo.db
    │   ├── tools
    │   │   └── tools.md
    │   └── worklog.db
    ├── memory
    │   ├── memory.md
    │   └── things.db
    ├── readme
    ├── role.md
    ├── self.md
    └── sub-agents
        └── web-agent
            ├── behaviors
            ├── environment
            │   ├── todo
            │   │   ├── task1
            │   │   └── todo.db
            │   ├── tools
            │   │   └── tools.md
            │   └── worklog.db
            ├── role.md
            └── self.md
```
---

## 里程碑规划（Roadmap）

* **MVP-1：单 Agent 可运行与可观测闭环**
  on_wakeup + LLMBehavior + bash actions + workspace (最小集合)

* **MVP-2：SubAgent 与能力包**
  SubAgent 独立进程、预算限制、workspace 协作交付
  集成BuckyOS提供的特殊能力

* **MVP-3：Memory深化，UI完整化**
  compact_memory、things.sqlite 事实抽取、

* **MPV-4: KB**
  实现至少一种KB（全文搜索/矢量数据库/图数据库),建立其对应的管线
  实现专门的KB整理Agent Mia
