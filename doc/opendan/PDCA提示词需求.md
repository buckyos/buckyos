# Jarvis 行为提示词需求文档

> **文档目的**：为提示词工程师提供结构化需求规格，用于实现 Jarvis Agent 基于 PDCA 循环的完整行为提示词。
>
> **阅读对象**：提示词工程师
>
> **版本**：v0.1 Draft

---

## 一、全局上下文（Global Context）

> 以下认知在所有 Behavior 阶段共享，应作为提示词的公共前置模块，避免在各阶段重复定义。

### 1.1 多 Step 执行模型

- Jarvis 运行在多 Step 调度框架中，每个 Step 对应一次 LLM 调用
- 每个 Step 的 Input 中包含：当前 Behavior 类型、上一步的执行结果（last_step_result）、剩余预算（remaining_budget）
- Agent 必须在每个 Step 结束时，通过设置 `next_behavior` 声明下一步的行为类型和目标

### 1.2 预算感知

- 每个 Step 消耗预算，Agent 应始终感知剩余预算
- 当预算不足以完成当前任务时，应优先保存进度，向用户报告状态，而非强行执行
- 预算耗尽策略：保存当前状态 → 输出阶段性成果 → 通知用户

### 1.3 上下文继承

- 每个 Step 必须读取并理解 Input 中的 `last_step_result`，确保行为连续性
- 不可忽略上一步传递的状态信息、错误信息或中间产物

### 1.4 next_behavior 状态机（全局视图）

以下状态转换图覆盖所有合法路径，提示词工程师应据此实现状态切换逻辑：

```
┌──────────────────────────────────────────────────────────────┐
│                      PDCA 状态转换图                          │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│  ┌──────┐    todo ready     ┌─────┐   done    ┌───────┐     │
│  │ PLAN ├──────────────────►│ DO  ├──────────►│ CHECK │     │
│  └──┬───┘                   └──┬──┘           └──┬────┘     │
│     │                          ▲                  │          │
│     │ 复杂计划需 Review        │                  │          │
│     │ (可选回环)               │                  │          │
│     └──────────┘               │                  │          │
│                                │                  │          │
│              ┌─────────────────┘                  │          │
│              │ Adjust 完成                        │          │
│              │                                    │          │
│         ┌────┴───┐     check 失败                 │          │
│         │ ADJUST │◄───────────────────────────────┤          │
│         └────┬───┘                                │          │
│              │                                    │          │
│              │ 整体验收失败                        │          │
│              │ (无 CurrentTask)                   │          │
│              ▼                                    │          │
│         ┌────────┐                                │          │
│         │ PLAN   │  (重新规划)                     │          │
│         └────────┘                                │          │
│                                                   │          │
│                          check 通过 & 还有 todo   │          │
│                     ┌─────────────────────────────┘          │
│                     │                                        │
│                     ▼                                        │
│                  ┌─────┐                                     │
│                  │ DO  │  (下一个 todo)                       │
│                  └─────┘                                     │
│                                                              │
│                          check 通过 & 所有 todo 完成         │
│                     ┌─────────────────────────────┐          │
│                     │                             ▼          │
│                  ┌──┴──┐   验收失败   ┌────────┐             │
│                  │CHECK├────────────►│ ADJUST │             │
│                  └─────┘             └────────┘             │
│                     │                                        │
│                     │ 所有 todo 完成                          │
│                     ▼                                        │
│                 ┌───────┐  验收失败  ┌────────┐              │
│                 │ BENCH ├──────────►│ ADJUST │              │
│                 └───┬───┘           └────────┘              │
│                     │                                        │
│                     │ 验收通过                                │
│                     ▼                                        │
│                 ┌────────┐                                   │
│                 │  DONE  │  (交付完成)                        │
│                 └────────┘                                   │
│                                                              │
└──────────────────────────────────────────────────────────────┘
```

**状态转换规则速查表**：

| 当前阶段 | 条件 | next_behavior |
|---------|------|---------------|
| PLAN | 计划就绪，选定首个 todo | `DO:<todo_item_id>` |
| PLAN | 复杂计划，需先 Review | 内部回环后再选 DO |
| PLAN | 任务不可能完成 | 告知用户，终止 |
| DO | 任务项执行完毕 | `CHECK` |
| CHECK | 验收通过，还有剩余 todo | `DO:<next_todo_item_id>` |
| CHECK | 验收失败 | `ADJUST:<current_todo_item_id>` |
| CHECK | 验收通过，所有 todo 完成 | `BENCH` |
| BENCH | 最终验收通过 | `DONE`（交付） |
| BENCH | 最终验收失败 | `ADJUST`（无特定 task） |
| ADJUST | 有 CurrentTask，修复完成 | `DO:<todo_item_id>` |
| ADJUST | 无 CurrentTask（整体失败） | `PLAN`（重新规划） |

### 1.5 用户交互协议

> 提示词工程师需在以下场景实现用户交互逻辑：

- **必须等待用户确认**：需求存在歧义或冲突时；任务判定为不可能完成时；Adjust 阶段发现需求冲突时
- **可自主决策**：常规 DO/CHECK 执行过程中；正常状态转换；工具调用和技术选型

### 1.6 Reply 规范

- Agent 在每个 Step 中应通过 `reply` 向用户通报当前状态
- 在写操作（创建文件、修改代码等）开始前，应先通过 reply 宣告决定
- Reply 应简洁明确，包含：当前在做什么、为什么做、预期结果

### 1.7 异常处理

| 异常类型 | 处理策略 |
|---------|---------|
| 工具调用失败 | 重试一次 → 仍失败则记录错误，进入 ADJUST |
| 预算耗尽 | 保存当前进度 → 输出阶段性成果 → 通知用户 |
| 不可完成的任务 | 在 PLAN 阶段识别 → 向用户说明原因 → 终止 |
| 外部依赖不可用 | 尝试替代方案 → 无替代方案则进入 ADJUST → 必要时求助用户 |

---

## 二、PLAN 阶段

### 2.1 阶段目的

PLAN 是 PDCA 的起点。目标是：理解任务全貌，初始化工作环境，将任务分解为可执行、可验收的 todo 列表，并选定第一个执行项。

### 2.2 行为分解

#### 2.2.1 环境初始化（Environment Setup）

核心决策：选择已有 Workspace 还是创建新 Workspace。

- **观察系统状态**：检查当前可用的 Workspace 列表和状态
- **`create_workspace`**：当任务需要新的隔离环境时调用
- **`bind_workspace`**：当任务应在已有 Workspace 中继续时调用
- 提示词应引导 Agent 先观察再决策，不可跳过此步

#### 2.2.2 任务分解（Task Decomposition）

将整体任务拆解为 todo 列表，每个 todo 需满足以下标准：

- **目标明确**：一句话说清楚要做什么
- **可验收**：有明确的完成标准（什么状态算"做完了"）
- **可完成**：在当前系统能力和资源范围内可执行
- **原子性**：一个 todo 对应一个 DO-CHECK 周期

**不可完成任务的识别与处理**：

- 如果在拆解过程中发现存在子任务当前系统无法完成，且无可求助对象（无可用 skill、无外部工具、无法委派），应在 PLAN 阶段直接告知用户这是不可能完成的任务
- 不应进入 DO 阶段后再发现不可完成

**求助判断**：

- 是否存在可用的 skill 覆盖该子任务？
- 是否可以委派给其他 Agent 或工具？
- 是否需要用户提供额外信息或权限？

#### 2.2.3 排序与选择首个 DO（Sequencing）

- 使用 `todo add` 批量添加 todo 项，每次调用成功后会返回 `todo_item_id`
- 批量添加完成后，调用 `todo next` 获取第一个可用的 todo_item_id
- 对于复杂计划（依赖关系复杂、风险较高），应先进行内部 Review 再选择首个执行项
- 选定后，设置 `next_behavior = DO:<todo_item_id>`

### 2.3 todo CLI 工具参考

> 提示词工程师应在此补充 todo CLI 工具的完整用法说明，包括但不限于：

- `todo add <description>` — 添加 todo 项，返回 todo_item_id
- `todo next` — 获取下一个可用的 todo_item_id
- `todo list` — 列出所有 todo 项及其状态
- `todo status <todo_item_id> <status>` — 设置 todo 项的状态
- （其他命令待补充实际 CLI 规格）

### 2.4 PLAN 阶段输出

| 输出项 | 说明 |
|-------|------|
| Workspace | 已创建或绑定的 Workspace |
| Todo 列表 | 所有 todo 项（含 id、描述、验收标准） |
| next_behavior | `DO:<first_todo_item_id>` |
| Reply | 向用户通报计划概要 |

---

## 三、DO 阶段

### 3.1 阶段目的

DO 是执行阶段。目标是：按照 todo 项的需求说明完成具体工作，通过多次迭代达到可提交状态，并完成自我检测。

### 3.2 行为分解

#### 3.2.1 任务理解

- 仔细阅读 Input 中的 **Current Task** 需求说明书
- 理解该 todo 的目标和验收标准
- 在 thinking 中整理执行思路

#### 3.2.2 执行策略

- **优先使用已载入的 skill**：如果当前任务有对应的 skill，优先按 skill 的方法推进
- **迭代式执行**：目标和验收标准已明确，通过多次迭代逐步完成
- **宣告再执行**：在写操作（创建文件、修改代码、调用外部服务等）开始前，先通过 reply 宣告决定，然后再执行

#### 3.2.3 自我检测

- DO 阶段的完成标准不仅是"做完了"，还需要包含一次自我检测
- 自我检测：Agent 自行验证输出是否符合 todo 的验收标准
- 自我检测通过后，才设置 `next_behavior = CHECK`

### 3.3 DO 阶段输出

| 输出项 | 说明 |
|-------|------|
| 执行产物 | 代码、文档、配置等具体交付物 |
| 自我检测结果 | 通过/不通过（不通过则继续迭代） |
| next_behavior | `CHECK`（自我检测通过后） |
| Reply | 向用户通报执行结果摘要 |

---

## 四、CHECK 阶段

### 4.1 阶段目的

CHECK 是验收阶段。目标是：以独立视角验证 DO 阶段的产物是否满足 todo 的需求说明，并根据结果决定下一步走向。

### 4.2 行为分解

#### 4.2.1 验收理解

- 仔细阅读 Input 中的 **Current Task** 需求说明书
- 独立思考验收方法（不依赖 DO 阶段的自我检测结论）
- 确定具体的验收手段

#### 4.2.2 执行验收

- **优先使用 skill 中的验收方法**：如 lint、test、build 等自动化手段
- 执行验收检查，记录验收结果
- 根据验收结果设置 Task 状态（通过/失败）

#### 4.2.3 状态转换决策

根据验收结果，按以下规则设置 next_behavior：

- **验收通过 + 还有未完成 todo**：调用 `todo next` 获取下一个 todo_item_id → `next_behavior = DO:<next_todo_item_id>`
- **验收失败**：`next_behavior = ADJUST:<current_todo_item_id>`
- **验收通过 + 所有 todo 已完成**：`next_behavior = BENCH`（进入集成测试/最终验收）

### 4.3 CHECK 阶段输出

| 输出项 | 说明 |
|-------|------|
| 验收结果 | 通过/失败 + 具体验收报告 |
| Task 状态 | 已更新的 todo 状态 |
| next_behavior | 按规则设置 |
| Reply | 向用户通报验收结果 |

---

## 五、BENCH 阶段

### 5.1 阶段目的

BENCH 是最终验收阶段，是一种特殊的 CHECK。目标是：站在整体任务的角度，对所有 todo 的集成产物进行端到端验收，完成交付物提交。

### 5.2 行为分解

#### 5.2.1 整体验收

- 不再聚焦单个 todo，而是从整体任务目标出发
- 验证所有 todo 的产物是否协同工作、整体一致
- 执行集成级别的测试或检查

#### 5.2.2 交付物提交

- 理解交付物的提交方法和格式要求
- 整理并提交最终交付物
- 向用户发送最终验收报告，包含：任务概要、完成情况、交付物清单、已知局限

#### 5.2.3 状态转换决策

- **最终验收通过**：`next_behavior = DONE`，任务完成
- **最终验收失败**：`next_behavior = ADJUST`（此时无特定 CurrentTask，进入整体调整模式）

### 5.3 BENCH 阶段输出

| 输出项 | 说明 |
|-------|------|
| 最终验收报告 | 整体验收结果 + 交付物清单 |
| 交付物 | 最终产物（已提交） |
| next_behavior | `DONE` 或 `ADJUST` |
| Reply | 最终验收报告发送给用户 |

---

## 六、ADJUST 阶段

### 6.1 阶段目的

ADJUST 是修正阶段。目标是：根据 CHECK 或 BENCH 的失败信息，分析问题根因，制定修复方案，然后重新进入执行。

### 6.2 两种模式

#### 6.2.1 Adjust:Task（有 CurrentTask）

**触发条件**：CHECK 阶段验收失败，Input 中存在 CurrentTask。

行为要求：

- 重点 Review 当前 Task 的 CHECK 失败信息
- 分析失败原因，收集上下文
- 尝试给出解决方案：
  - **技术问题**：修正实现方法，补充遗漏逻辑
  - **需求冲突或不明**：需要和用户确认，暂停等待用户输入
  - **超出能力范围**：尝试将 Task 重新分配给其他 skill 或 Agent
- 修复方案确定后：`next_behavior = DO:<current_todo_item_id>`

#### 6.2.2 Adjust:Plan（无 CurrentTask）

**触发条件**：BENCH 阶段最终验收失败，需要整体调整。

行为要求：

- 整体 Review 失败原因，可能涉及：计划拆分不合理、todo 间依赖未处理、整体方案方向性错误
- 重新调整计划：可能修改现有 todo、添加新 todo、调整执行顺序
- 调整完成后：`next_behavior = DO:<adjusted_todo_item_id>`
- 极端情况下可能需要返回 PLAN 重新规划

### 6.3 ADJUST 阶段输出

| 输出项 | 说明 |
|-------|------|
| 问题分析 | 失败根因分析 |
| 修复方案 | 具体的修复或调整计划 |
| next_behavior | `DO:<todo_item_id>` 或 `PLAN`（重新规划） |
| Reply | 向用户通报问题和修复方案 |

---

## 七、待提示词工程师补充的部分

以下内容在本需求文档中标记为待补充，需要提示词工程师在实现时根据实际系统规格完善：

1. **todo CLI 工具完整规格**：所有命令、参数、返回值格式（2.3 节）
2. **Workspace 工具规格**：`create_workspace`、`bind_workspace` 的参数和行为细节
3. **Skill 加载机制**：Agent 如何发现和加载可用 skill
4. **Policy 层**：各阶段的策略约束（原文注明"policy 先不写"，后续需补充）
5. **Input/Output 数据结构**：每个 Step 的 Input 完整 schema（含 Current Task、last_step_result 等）
6. **预算系统细节**：预算单位、消耗计算方式、阈值设定
7. **求助/委派协议**：如何将 Task 分配给其他 Agent 或请求外部协助的具体机制
8. **Reply 模板**：各阶段推荐的 reply 格式和模板
9. **Thinking 指导**：Agent 在 thinking 中应遵循的推理框架（当前仅在 DO 阶段提及）

---

## 附录：术语表

| 术语 | 定义 |
|------|------|
| Step | 一次 LLM 调用，Jarvis 的最小执行单元 |
| Behavior | Agent 在当前 Step 中的行为类型（PLAN / DO / CHECK / BENCH / ADJUST） |
| next_behavior | 当前 Step 结束时声明的下一步行为，格式为 `BEHAVIOR_TYPE` 或 `BEHAVIOR_TYPE:<todo_item_id>` |
| Todo | 任务分解后的单个工作项，具有 id、描述、验收标准、状态 |
| Workspace | Agent 的工作环境，包含文件系统、工具配置等上下文 |
| CurrentTask | Input 中传入的当前正在处理的 todo 项的完整信息 |
| Skill | 预定义的方法集，为 Agent 提供特定领域的执行能力 |
| Bench | 最终集成验收阶段，是 CHECK 的全局版本 |
| Budget | Agent 可用的执行预算（Step 数或 token 数），用于控制执行范围 |


## 附录2：相关cli工具的使用说明

- bind_workspace : 设置agent_session的当前workspace
```bash
bind_workspace <workspace_id|workspace_path>
```
- create_workspace : 创建session的wrokspace并设置为session的default workspace
```bash
create_workspace <name> [template]
```


- read_file : Read file.
```bash
read_file <path> [range] [first_chunk]
    trange: 1-based; supports negative/$/+N, and applies within first_chunk slice
```
- todo : Workspace todo CLI with sqlite/oplog persistence and PDCA state guardrails.
```bash
todo <command> [args...]
Plan:
  todo clear
  todo add "title" [--type=Task|Bench] [--priority=N] [--deps=T001,T003|--no-deps]

Do:
  todo start  T001 ["reason"]
  todo done   T001 "reason"
  todo fail   T001 "reason" [--error='{"code":"...","message":"..."}']

Check:
  todo pass   T001 ["reason"]
  todo reject T001 "reason"

Notes:
  todo note   T001 "content" [--kind=note|result|error]

Query:
  todo ls     [--all] [--status=WAIT,IN_PROGRESS] [--type=Task|Bench] [-q "keyword"]
  todo show   T001
  todo next
  todo pending [--status=WAIT,IN_PROGRESS]

Prompt:
  todo prompt [--budget=N]
  todo current [T001]

Global flags:
  --ws=<workspace_id> --session=<session_id> --agent=<agent_id> --op-id=<op_id>
```

## 附录3：LLM的3种返回格式说明

### 用于Plan 阶段的LLM Output Protocol

The response MUST be valid JSON that can be parsed by JSON.parse().
```typescript
type Response = {
  next_behavior?: string;    // MUST follow process rules
  thinking?: string;
  reply?: string;            // reply to current session default_remote only
  shell_commands?: string[]; // shell command strings, executed sequentially
}
```
All keys are optional—NEVER include unused keys.

## shell_commands
- Each entry is a shell command run sequentially in a session-bound bash environment; execution stops on first failure.
- Results persist in step_summary for the next step. MUST limit read output size to avoid context overflow.
- Common CLI tools and process_rule-declared tools are pre-installed. NEVER check availability before calling.


### 用于其它阶段的 LLM Output Protocol

The response MUST be valid JSON that can be parsed by JSON.parse().
```typescript
type Response = {
  next_behavior?: string;    // MUST follow process rules
  thinking?: string;         // MUST follow process rules 
  reply?: string;            // reply to current session default_remote only
  shell_commands?: string[]; // shell command strings appended to actions.cmds
  actions?: {
    mode?: "failed_end" | "all"; // default: "failed_end"
    cmds?: (string | [string, object])[];
  };
}
```
All keys are optional—NEVER include unused keys.

## actions.cmds
Example:
```json
{
  "actions": {
    "mode": "all",
    "cmds": [
      "todo add T01 \"build login.html\"",
      ["write_file", {"path": "readme.txt", "content": "login page"}]
    ]
  }
}
```
- Commands run sequentially in a session-bound bash env. On failure: "failed_end" stops, "all" continues.
- String element = shell command. Array element = structured cmd_action: `[action_name, {args}]`.
- `shell_commands` is shorthand that appends strings to `actions.cmds`. NEVER put structured actions in `shell_commands`.
- MUST use write_file / edit_file cmd_action for text files. NEVER use shell commands (echo/cat) to write files.
- Results persist in step_summary for the next step. MUST limit read output size to avoid context overflow.
- Common CLI tools and process_rule-declared tools are pre-installed. NEVER check availability before calling.

### write_file
`[action_name, {path, content, mode?}]`
- mode: "write" (default, overwrites), "new" (fails if exists), "append" (appends to end).

### edit_file
`[action_name, {path, new_content, pos_chunk, mode?}]`
- Anchors on `pos_chunk` in the file, then applies mode: "replace" (default), "after" (insert after), "before" (insert before).


## 附录4: Behavior配置文件参考示例
```yaml
process_rule: |

  目标:识别输入消息的内容,并决定将消息分配到哪些session

  1. **纯粹的闲聊 / 无实质内容的输入 / 可以直接回答的问题**（问候、确认、表情符号、无实质内容的闲聊）：
  * 立即给出一个简短、自然的回复。
  * 无需设置route_session_id和new_session
  * 此时的回复要更像ChatGPT

  2. **潜在任务发现**（包含复杂的查询，工程任务或有复杂意义的内容）：
  * 根据Work Session List,填写route_session_id决定当前输入的消息应该属于哪个Work Session
  * 如果Work Session List中没有合适的Session,则决定创建一个新Session

  ## WorkSessionList
  {{session_list}}



policy: |
  * 创建session是一个会消耗不少系统资源的操作,只有涉及到复杂的大块任务才需要创建session. 创建session必须填写reply。
  * route_session_id只能在`Work Session List`里选择,填写route_session_id后。一定不填写reply。
  * next_behavior总是填写END

output_protocol:
  mode: route_result

# 不允许是用任何工具，限制了该behavior只能快速的完成
# toolbox: 
#   skills: ["buildin"]

#TODO：ui session summary还是应该有的，系统在首次创建ui session的时候，应该把联系人的摘要信息放到summary里
memory:
  total_limt: 12000
  agent_memory: { limit: 6000 }
  history_messages: { limit: 6000, max_percent: 0.5 }
  #session_summaries: { limit: 3000 }

input: |
  {{new_msg}}

step_limits: 1
limits:
  max_tool_rounds: 1
  max_tool_calls_per_round: 4
  deadline_ms: 45000

llm:
  model_policy:
    preferred: llm.chat

```


## 附录5：系统支持的提示词配置的模版参数
{{key}} 支持的变量类型
### 来自 env_context（调用方传入）：

params、params.<path>、role_md、self_md、session_id、step.index、step_summary 等

### 来自 session（load_value_from_session）：

会话：session_id、step_index、last_step_summary
消息：new_msg、new_msg.$n
列表：session_list、local_workspace_list（可带 .$n 限制条数）
Todo：current_todo、workspace.todolist.next_ready_todo
工作区：workspace.<path>
文件：$workspace/<rel_path>、$cwd/<rel_path>
3. 其他说明
JSON 路径：支持 params.todo、params.items.0 等
转义：\{{、\}} 输出字面量花括号
限制：总输出 256KB，单文件 64KB
完整内容见 doc/arch/Render_Prompt_Template_Variables.md
