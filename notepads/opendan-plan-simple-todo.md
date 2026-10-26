# OpenDAN plan_simple：单 Turn、带 intent 的 function call 实施 TODO

日期：2026-10-25

状态：T01–T07 已于 2026-10-25 实施（未提交），落点、验证与真实模型证据见 §10；跨 runtime 交接、节点 / Windows / macOS 与 Zone DV 未验证。

目标：参考 `work_simple.toml` 新增 `plan_simple.toml`，用原生 function call 实现 `Plan → Todo → 独立 Subsession → Report → Review`。父 Plan 保存整体目标、任务清单与决策；子会话执行有界任务并交回证据。

路径均相对 `buckyos/`。实施前重新读取 `AGENTS.md` 和当前源码；本次检查时工作区已有较多未提交改动，尤其涉及 runtime、Session 与工具装配，不得覆盖或回滚其他工作。

## 0. 背景与选型原因

根据用户补充的项目运行经验，当前对接的云端主流 LLM 对内部 CoT（Chain of Thought，思维链）采取隐藏或限制输出的处理；原有显式 behavior 模式中要求模型展开思考过程的提示和输出约定，在这些服务链路上容易遇到 HTTP 403 或拒绝。**适应云端模型的 CoT 隐藏机制、减少这类协议兼容问题，是引入 plan_simple 的直接原因。** 这里记录的是项目当前观察与判断，具体 403 的原因仍以 provider 的响应和诊断信息核对。

新模式通过 provider 原生 function call 表达动作，用每次调用的 `intent` 保留可观察的行动目的：准备做什么、预期得到什么结果，以及相关的已确认事实。intent 不要求模型披露内部 CoT、隐藏 reasoning 或完整的逐步推理，也不能只是把这些要求换一个字段名继续索取。

实施时检查新提示词、工具描述和自动注入的运行协议，移除要求显式展开内部思考的约定；正常的任务计划、简短行动说明、验收理由与证据报告仍然需要保留。遇到拒绝时记录实际错误并按现有错误策略处理，不回退到要求输出完整思维链的旧提示。

## 1. 已确认的设计约定

- **整个父 Plan 是一个逻辑 Turn**：从任务输入到最终报告，包含多个 Round、Todo 派发、等待、Review 和重试。不是一个 Todo 一个父 Turn，不新增结束 Todo 后自动开启下一 Turn 的机制。
- **每次 function call 都必须携带 `intent`**：范围是 `plan_simple` 发出的每个原生调用，包括 shell、读取、等待和 report；同一个 Round 内多个调用各自携带。不能用一条普通 assistant 文本、一个 Round 总说明或 shell 注释代替。
- **函数结果返回后可注入 `background_context`**：由宿主在下一次推理前提供最新的相关状态，与 XML behavior loop 的动作结果后观察阶段对齐。单调用在该调用返回后注入；同批多个调用在结果配对完整后统一注入，不打断工具调用/结果协议。
- **TodoList 是计划和执行状态的唯一来源**：复用现有存储、attempt、依赖、报告引用及 review，不另建一套 plan JSON 或 DAG 服务。
- **每个 Todo attempt 由独立子 Work Session 执行**：串行任务也如此，包括与父会话相同 runtime/workspace 的任务；不走 `next_behavior=do` 或 `create_sub_context`。默认执行 behavior 为 `work_simple`，只读调查可使用 `explore`。
- **执行完成与验收通过分开**：宿主同步子会话状态与报告；父 Plan 读取证据后记录 Review，不能把子会话成功直接当成 Todo 已被接受。
- **等待只挂起，不结束父 Turn**：子结果回填后继续当前 Turn。压缩、恢复和进程重启也不得因为换了 run 段就另开一个父 Turn。
- **最终交付必须显式报告**：只有整体目标得到验证、所需结果已验收且所有任务已妥善处理，才提交 `report(is_end=true)`；失败则明确说明阻碍与剩余工作。

第一版沿用 `group:bash`、`report` 与 `agent-session todo ...` 命令面，通过 function call 参数表达 intent。讨论中的 `dispatch_todo` 等名称是接口示意，不要求再造一整套工具。若现有装配已有合适的原生 Todo 门面，可复用；所有入口必须调用同一套存储和状态迁移。

本次不迁移旧 `plan`/`do`，不默认替换所有 Work 的入口，不全局强制其他 behavior 携带 intent。子 `work_simple`/`explore` 的协议独立配置，不能因为父开启 intent 而意外改变它们的工具契约。

## 2. 当前实现与复用入口

以下为 2026-10-25 对照源码的检查结果；实施时须重新确认，历史 notepad 不作为当前实现依据。

| 文件或符号 | 可复用能力与需要处理的问题 |
| --- | --- |
| `src/apps/jarvis_agent/agent/behaviors/work_simple.toml` | 已用 `mode = "function_call"`、`group:bash` 和 `report` 直接执行有界任务，是新 behavior 的配置起点。 |
| `src/apps/jarvis_agent/agent/behaviors/plan.toml`、`do.toml`、`explore.toml` | 可参考任务说明、验收及隔离约定。旧 PLAN/DO 的 XML、`next_behavior`、Step 和 report-only yield 规则不能直接复制进 function_call。 |
| `src/frame/agent_tool/src/todo_tools.rs` | 已有 Todo/attempt/review/依赖与持久存储。`serial_pick()` 检查未 review 结果和依赖；`begin_child()` 本身只做部分状态检查，不能把它当作完整调度准入。 |
| `src/frame/lib_opendan/src/todos.rs` | 已有 `child_spec()`、`dispatch_serial()`、`sync()`、报告读取和按 `(parent, todo, attempt)` 派发的幂等机制。串行默认使用 `work_simple` 并继承 workspace。 |
| `src/frame/lib_opendan/src/xagent_cli.rs` | 目前 `todo dispatch` 固定走 Explore 派发，即使指定 `--behavior work_simple` 也会改变 Todo mode，并采用 Explore 的 workspace 默认值。不能直接当作保留串行语义的接口。 |
| `src/frame/lib_opendan/src/runner/live.rs`、`drive.rs` | function_call 的 wait 回填走 `fill()`，返回 `filled=true`，后续 `hold_inputs` 会跳过输入批次中的 `todos::sync()`。需要补齐继续推理前的同步，否则 Todo 可能仍是 running、没有报告引用。 |
| `src/frame/lib_opendan/src/runner/children.rs` | `SessionTaskResolver` 已解析 `session:<sid>`；`mark_waited()` 防止同一结果又作为事件重复交付。该标记不等于 Todo 已同步。 |
| `src/frame/lib_opendan/src/runner/reports.rs` | 已有最终报告的子会话与 Todo 收敛检查。`ReportArgs` 使用 `deny_unknown_fields`，直接把新增 intent 透传进去会报错。 |
| `src/frame/lib_opendan/src/runner/tools.rs`、`flush.rs`、`live.rs` | Session 工具装配/执行及原生调用的持久记录入口。现有 args 和 call_id 可承载、关联 intent，不需要另建日志系统。 |
| `src/frame/llm_context/src/deps.rs`、`context_loop.rs` | `CheckpointHook::before_inference` 与 `LLMContext::inject()` 已支持安全边界注入：function_call 在工具结果批次后追加 accumulated，behavior 在 Step 边界附到观察消息。挂起或批次未完成时不能直接 inject。 |
| `src/frame/lib_opendan/src/runner/hook.rs`、`background.rs`、`receipts.rs` | 当前 Session hook 的 before_inference 只做 boundary 并返回 None；background_context 的行渲染、变化筛选及回执机制主要在受控输入批次使用。本次需为 plan_simple 接上函数返回后的宿主观察，不能误认为它已自动生效。 |
| `src/frame/lib_opendan/src/protocol/behavior.rs`、`state/behaviors.rs`、`template.rs` | behavior 配置、冻结及 Session 模板入口。Work 默认一个 Turn，但 completion 默认 natural；需为新模式落实 explicit_report。 |

## 3. intent 契约：schema、执行与记录必须一致

### 3.1 模型可见的参数

intent 放在每个原生函数的 arguments 顶层，与业务参数并列。示例是实现目标，当前工具 schema 尚未支持这一契约：

```json
{
  "name": "shell",
  "arguments": {
    "intent": "T01 的根因报告已验收，派发 T02 执行修复并提供回归证据。",
    "command": "agent-session todo dispatch T02 --behavior work_simple"
  }
}
```

```json
{
  "name": "report",
  "arguments": {
    "intent": "所需 Todo 均已验收，提交最终结果及可追溯的验证证据。",
    "report": "完成内容、验证结果、限制及产物说明。",
    "artifacts": [],
    "is_end": true
  }
}
```

- intent 是简短、可审查的行动目的：为什么现在做这件事，准备得到什么信息或结果；相关时写 Todo ID 和已验收依据。无需输出模型的完整思考过程。
- 所有暴露给 Plan 的函数 schema 都声明 `intent: string` 且 required；执行时验证去掉首尾空白后非空。不要依赖 provider 是否严格执行 schema。
- 缺失、空白或类型错误返回可修正的工具错误；该调用不产生业务副作用，允许模型补齐后继续同一 Turn。
- intent 不能替代完整 Todo 正文、验收条件、Review reason 或宿主权限/依赖检查。模型宣称“已验收”不构成状态事实。

### 3.2 实现边界

- 优先在既有 Session 工具装配/执行链中加入一个明确、可冻结的 behavior 级要求，限定 `plan_simple` 启用。配置字段名由实现按现有结构收敛，不在多个层次散布 `behavior == "plan_simple"` 的硬编码。
- 原生 `AiToolCall.args` 已能携带 intent，无需为此修改 AICC 的公共调用结构。工具枚举、实际发给模型的 schema 与执行校验必须一致，覆盖首次创建、恢复、重建和压缩后的路径。
- 保留原始调用及完整 args；仅从传给业务工具的执行副本中提取、删除 intent。它是宿主记录用的元数据，不应进入 shell 命令文本或作为未知参数传给下层工具。
- `report` 的宿主特殊分支也必须经过同一校验和剥离流程，不能绕过，更不能因 `deny_unknown_fields` 导致最终交付失败。
- 复用 snapshot、原生消息和 worklog 的 `run_id + call_id + args` 记录。通过现有查看会话/worklog 的路径能看到 intent、动作及结果；如展示层丢弃它，补最小投影，不新增独立数据库或完整 UI 项目。已完成调用可随正常压缩退出活跃上下文，其原始 intent 仍可从持久 worklog/归档追溯，不要求永久保留全部旧参数在模型上下文中。
- 未完成/Pending 调用在快照、回填和 inflight 恢复中完整保留原 intent；恢复不得重复执行已提交的动作。重试不覆盖旧调用/旧 attempt 的记录，新调用填写本次重试自己的 intent。原始历史与执行副本的职责须保持清楚。

## 4. Todo 派发、等待与完成

### 4.1 派发

- 提供保留 Todo mode 的派发路径：Serial 复用 `dispatch_serial()`，Explore 复用已有探索路径。CLI 的最终参数和命名可按当前实现收敛，但不得通过把所有 Todo 转成 Explore 绕开串行检查。
- 在宿主/存储侧校验派发条件：Todo 可执行、依赖结果已 accepted、串行已有执行不会重入；复用 `serial_pick()` 的待 review 结果约束。检查与提交需有合适的锁边界，避免检查之后状态改变。
- 串行一次只运行一个；独立探索可以并行，但涉及相同文件的写任务不能因换了 sid 就视为隔离。
- Todo 保存或稳定派生执行所需的 runtime、workspace、skills、behavior。重试不能丢失先前指定的执行环境或意外退回默认 explore。
- 子会话 objective/首条消息包含完整任务、验收标准、允许修改范围、必要输入和已验收结果引用；不要只传一行 intent，也不要默认复制父会话全部历史。
- 每个 attempt 的 child_sid 稳定绑定。重复派发和重启恢复不得创建重复子会话；旧 attempt 的迟到结果不得覆盖新 attempt。
- 验证同 runtime、继承 workspace 的串行子会话能实际运行，父子之间遵守现有 workspace 准入与租约机制，不发生父持有资源等待子、子又等待父的死锁。

### 4.2 等待和恢复

第一版使用独立的 `shell` function call 执行 `agent-session wait <child_sid>`，该调用同样带 intent，例如“等待 T02 的验证报告，随后检查其回归证据”。不要把 Pending 命令与其他 shell 命令拼接成一个复合脚本。

- 复用现有 `Pending(session:<sid>)` 和 resolver。等待期间宿主观察状态，父模型无需重复推理或忙轮询。
- 回填后、下一次父推理之前，同步已提交的子状态到 TodoStore，保证 `todo show --report` 和 `todo review` 立即可用；与结果去重、快照提交及崩溃恢复顺序一起验证。
- 明确消费 tool result 的状态：wait 也可能因需要输入/决定或截止时间而返回，不能把“调用返回”当作“子任务成功”。
- `report(is_end=false)` 只记录阶段进度，不承担等待或 Turn 结束语义；不复制旧 PLAN 的 XML report-only yield 规则。
- `wait A` 只等 A，不是 wait-any。一次挂起多个 wait 也不能承诺谁先完成就恢复。第一版可先等待选定子会话，再检查其他结果；不把新增 wait-any 作为完成条件。
- 同一长 run 内 Todo 视图不会因为提示词写了“每轮可见”就自动刷新。本次必须按 §4.4 接上函数返回后的 background_context；list/show 仍用于主动查询和读取细节，不能默认注入子 worklog。

### 4.3 Review 与最终报告

- 读取完整报告并检查产物后，按每个 attempt 记录 accept/reject/need-evidence 和理由。未被接受的结果不得满足后继任务依赖。
- 缺证据或失败时 retry，或创建补充 Todo，并保留旧报告和失败原因。运行中的任务不能用删除记录或 cancel pending 的方式伪装收敛。
- 原有收敛检查区分“分支已处理”和“目标成功”；一个失败分支被 review 不等于目标已完成。最终报告需如实说明成功、部分完成或无法完成。
- 新模式的 Session 必须是单 Turn Work，且 `completion = "explicit_report"`。优先用专用 class（例如 `[session.plan_simple]`、`base = "work"`、`default_behavior = "plan_simple"`）绑定完成策略；CLI/手册明确给出实际创建方式。不得只在提示词里要求显式报告。
- 首版可通过 `agent-session create-worksession --class plan_simple --objective "..."` 显式选择。不要顺带把 `[session.work].default_behavior` 全部改成新模式。

### 4.4 函数返回后的 background_context

这条接线使 function_call 保留现有 behavior loop 的“行动 → 观察 → 下一次决策”能力，而无需要求模型输出内部思维过程：

| XML behavior 的语义 | plan_simple 的对应语义 |
| --- | --- |
| 显式行动意图 | function arguments 中的 intent |
| actions | 原生 function calls |
| 动作结果及边界观察 | tool results + 宿主注入的 background_context |
| 结束报告 | report(is_end=true) |

这里只对齐观察与决策语义，不把 function_call 的 Round 伪装成 Behavior Step，也不新增 Turn。

**注入时机与顺序：**

```text
LLM 发出带 intent 的 function call(s)
→ 宿主执行并收齐当前批次的 tool results（Pending 则先等待并恢复）
→ 同步已提交的 Todo、子会话及相关环境状态
→ 在下一次推理前追加发生变化的 background_context
→ 持久化注入后的上下文及已展示版本
→ LLM 在同一 Turn 内根据结果与最新状态继续决策
```

- 每次结果返回都应有检查背景变化的机会；没有相关变化就不添加空块，不为了刷新背景额外调用模型。
- 多工具批次不在某个 call 和对应 result 之间插入独立消息，也不在仍有未配对结果时注入。先完成当前批次，再统一提供最新观察；此前已生成的同批调用不会因此被视为重新决策。
- 覆盖普通成功、可修正错误、阶段 report 返回和 wait/Pending 恢复。已接受的最终 report 直接结束，不为注入背景再启动一次推理；挂起等待期间不忙轮询 LLM。
- 优先使用现有 checkpoint/injection 接口，复用背景行渲染和状态投影。工具业务返回值保持完整，宿主观察使用独立、明确标识的 background_context 块，不伪装成工具自身的输出，也不修改旧消息或 system prompt 来刷新状态。
- 这一机制只为新模式接线；现有 hook 中“只在受控输入批次注入”的说明需同步修订适用范围，不顺带改变其他 Session 的输入消费策略。

**内容、去重与持久化：**

- 内容来自宿主已提交状态，包括紧凑 Todo 变化（id、attempt、状态、review、summary、report_ref）、子会话结束/阻碍、相关 runtime/workspace/其他活动会话变化，以及已有订阅机制允许呈现的背景信息。只放影响下一步决策的内容。
- 不包含子会话完整 transcript、工具日志或内部推理；报告全文仍需 show --report。background_context 是状态观察，不是自动验收，也不替代依赖检查或新的用户输入。
- 优先复用 `background::select/render`、Todo rev、子状态 rev 与现有回执机制；同一版本只展示一次。没有显示某项变化不代表其原状态被删除，清除/失效必须明确表达。
- `before_inference` 返回 Injection 后会再次进入 hook，以持久化注入后的快照。必须防止同一边界反复注入相同内容；时间等动态字段复用节流规则，不因每次取时不同形成注入循环。
- 已展示版本必须随可恢复的注入记录提交，不能先标记 shown 再因崩溃丢失消息。恢复时保证变更不丢失、不重复展示，不重放已经执行的业务调用；复用原有提交纪律，不另建背景状态真相源。

## 5. plan_simple 提示词骨架

下列是内容约束草案。最终按项目既有语言和提示词风格收敛，工具命令以实现后的 help/schema 为准。intent 的行为级配置需在实施中补齐，下面没有虚构尚不存在的 TOML 键。

```toml
[meta]
name = "plan_simple"
objective = "Plan with Todos, delegate execution to sub sessions, review evidence, and deliver the result."

[prompt]
mode = "function_call"
system = """
你是一个负责整体任务的 Plan Session。整体计划在当前一个 Turn 内完成。
你负责拆解、派发、验收与最终交付；实际执行交给独立子会话。
可以读取材料和检查产物，以支持规划和验收。

每次 function call 都在 arguments 中填写非空 intent，包括读取、shell、等待和 report。
用简短的一句话说明为什么现在做这件事、希望得到什么信息或结果。
引用相关 Todo 和已确认的证据；不要只写“调用工具”或用 intent 代替完整任务。
只描述可公开的行动目的，不要求展示内部思维链或隐藏 reasoning。

TodoList 是计划和执行状态的唯一来源。先读取已有状态，再决定下一步。
工具结果之后，宿主可能提供 background_context，包含自上次展示以来变化的任务与环境状态。
结合这些观察调整下一步；未出现的新背景不表示已有状态被清空。
背景摘要不能替代完整报告和验收；需要细节时主动 list/show 或读取证据。
每个 Todo 写清任务、输入、范围、验收条件、依赖、执行环境及是否允许修改文件。
子会话看不到你的完整对话，必须给足它需要的上下文和可访问引用。
只拆出当前明确的工作；根据已验收结果补充后续任务。

有返回结果时，读取 todo show --report，对照验收条件检查证据，并记录 todo review。
执行成功不代表结果被接受。缺少证据时补任务或 retry，并说明需要改变什么。

有可执行任务时，通过 Todo 派发入口交给 work_simple；只读调查可用 explore。
有依赖的工作等待依赖被接受；串行一次一个，独立探索可以并行。
不直接创建绕过 Todo 绑定的工作子会话，不交接到 do 子 context。

暂时没有可做工作时，单独执行 agent-session wait <child_sid>，并填写等待意图。
等待只挂起当前执行；恢复后仍是当前 Turn。先检查返回状态，再读取最新 Todo 和报告。
不忙轮询，不把 report(is_end=false) 当成等待。

整体目标已验证、所需结果已接受且所有分支已处理后，调用 report(is_end=true)。
报告包含结果、验证证据和重要 artifacts。无法完成时说明阻碍与剩余工作。
阶段进度用 report(is_end=false)；不以普通文本回答代替最终报告。
"""

[capabilities]
tool_whitelist = ["group:bash", "report"]

[model]
preferred = "llm.plan"
```

预算根据父会话的规划、等待调用、读取与 Review 开销设置，不直接复制 `work_simple` 的 24 次工具迭代。沿用现有预算与错误恢复机制，记录真实测试中采用的数值；不要把 max_tool_iterations 当成 Round 数。

## 6. 预期执行轨迹

以“定位乱码、修复并验证”为例。命令参数省略，每个工具调用都各有 intent：

| 父 Round | function call | 结果及边界 |
| --- | --- | --- |
| P1 | shell：添加定位任务 | 返回 T01。 |
| P2 | shell：派发 T01 | 返回子会话 A 的 sid。 |
| P3 | shell：wait A | Pending；父 Turn 保持打开，等待期间无父推理。 |
| — | A 自己的工具循环和最终 report | A 的 Turn 结束；宿主同步 T01、回填 P3，并在下一次父推理前注入变化的 background_context。 |
| P4 | shell：show T01 --report | 获得完整证据，不自动导入 A 的执行历史。 |
| P5 | shell：review T01 --accept | 记录验收理由。 |
| P6 | shell：添加修复任务，依赖 T01 | 返回 T02，正文携带已确认根因与验收要求。 |
| P7 | shell：派发 T02 | 返回子会话 B 的 sid。 |
| P8 | shell：wait B | 同一个父 Turn 再次挂起。 |
| P9 | shell：show T02 --report | 获得修复报告与验证证据。 |
| P10 | shell：必要的产物检查 | 补足验收依据；需要额外执行工作则建 Todo。 |
| P11 | shell：review T02 --accept | 记录验收。 |
| P12 | report：is_end=true | 宿主检查收敛后结束父 Turn。 |

以上 Round 数仅作示意。每个继续执行的工具结果批次之后都可按 §4.4 注入变化的背景，不只在 wait 返回时注入。不得在同一 Round 的独立工具调用中使用尚未返回的 todo_id/child_sid。父会话全程只打开一个 Turn；子会话各自有独立 Turn、历史和报告。

## 7. 实施清单

- [x] **T01 核对基线与入口**：确认当前源码、已有改动、工具装配、behavior 冻结和模板解析路径。新模式通过专用 Work class 或等价的明确配置启用单 Turn + explicit_report。
- [x] **T02 实现全调用 intent**：行为级启用；模型 schema、执行前校验、执行副本剥离、report 特殊分支、原始持久记录和恢复一致。不要只改提示词或 shell 一个工具。
- [x] **T03 补齐串行子会话派发**：复用 dispatch_serial、依赖与 Review 检查、串行互斥、环境继承、幂等及 retry；同步 CLI 帮助，清理只适用于 next_behavior=do 的提示。
- [x] **T04 同步结果并注入背景**：普通工具结果批次及 wait 回填后，在继续推理前同步 Todo 与相关状态、按变化注入 background_context；验证报告可读、Review 可用、工具消息配对、注入去重、提交顺序和 crash/restart，不增加父 Turn。
- [x] **T05 新增 behavior 与创建入口**：落地 `src/apps/jarvis_agent/agent/behaviors/plan_simple.toml`、专用模板与完整任务交接；预算足以走完真实案例，按冻结配置生效。
- [x] **T06 文档和手册联动**：更新 `cmd_skills/opendan_agent_dev.md`、相关 Session/Todo 协议说明；若改了通用 xllm 工具契约，同步 `cmd_skills/xllm.md`。明确旧 PLAN/DO 与新 function_call 模式的区别，不把旧 XML 等待指令带入新模式。
- [x] **T07 自动化与真实模型验收**：完成下一节用例，记录已运行结果和未验证项；真实模型轨迹不能由 mock 或测试驱动代替模型决策。

## 8. 验收要求

### 8.1 自动化检查

复用或扩展 `src/frame/lib_opendan/tests/plan_do_todo.rs`、`report.rs`、`xagent_cli.rs`、`xagent_lib.rs` 和必要的下层测试；允许新增聚焦的新测试文件。

- **配置与 schema**：加载真实 Jarvis plan_simple、专用模板和工具集；模型实际收到的所有函数都有 required intent，包括 strict report；没有 XML action/DO 交接协议混入。
- **intent 正反例**：shell、读取、wait、阶段/最终 report 正常；缺失、空白、非字符串被拒绝且无副作用；一次 Round 的多个调用各自校验；修正后继续同一 Turn。
- **记录与执行分离**：业务工具没有收到额外 intent，持久调用记录保留原始值；按 call_id 能关联结果；未完成调用在 snapshot、Pending 和 restart 后不丢失、不重复执行，已完成调用压缩后仍可从 worklog/归档追溯。
- **真实串行路径**：同 runtime 的 T01、T02 各有独立 child_sid；没有 DO 子 context；T01 未结束或未验收时 T02 不得执行，依赖检查不能绕过。
- **wait 回填**：子结束后首次恢复即可 show --report 并 review，不能仍为 running/null；覆盖子失败、需要输入/决定、等待截止、进程恢复和重复通知。
- **background_context**：检查模型实际收到的下一次推理请求；普通结果、错误纠正和 Pending 恢复后可见最新相关变化，工具 call/result 配对完整，多调用批次只在安全边界追加观察；没有变化不重复注入，不自动带入子执行全文。
- **背景恢复与对齐**：覆盖注入前后和已展示版本提交前后的崩溃；恢复不丢失/重复观察，不额外开启 Turn 或重执行工具；同一状态变化在 XML Step 边界与 function_call 结果批次边界的可见事实一致，不要求两者消息结构完全相同。
- **分支与重试**：独立探索可并行，前置全部 accepted 才放行后继；retry 保留旧 attempt/报告和执行环境；相同派发不重复建子会话。
- **workspace 与产物**：同 workspace 串行执行不死锁，独立探索遵守隔离；跨 runtime 交接使用可访问对象/引用，不把子本地路径当作父本地文件。
- **完成检查**：running、pending、未 review 或未处理的子会话存在时拒绝最终 report；普通文本不能绕开 explicit_report；失败报告不伪装目标成功。
- **Turn 断言**：正常案例父只有一次 turn_started 和一次 completed turn_ended；等待/恢复/Review/重试期间 turn id 不变。子会话的 turn 独立计数。
- **范围回归**：未启用 intent 的 work_simple、旧 PLAN/DO、UI/Goal 等原有契约不因共享工具装配被强制改变。

从 `buckyos/src/` 运行相关测试，例如：

```bash
cargo test -p libopendan --test plan_do_todo -- --test-threads=1
cargo test -p libopendan --test report -- --test-threads=1
cargo test -p libopendan --test xagent_cli -- --test-threads=1
cargo test -p libopendan --test xagent_lib -- --test-threads=1
```

若修改了 agent_tool/llm_context 通用路径，再运行对应包的相关测试。新增测试目标按实际文件补入记录；模板打包按 `uv run buckyos-build.py -s jarvis_agent` 验证。不得把文档中的建议命令当成已执行。

### 8.2 真实模型最小验收

优先复用 `test/plan_do_todo_dv/` 的可重复 fixture 和运行/取证能力，新增 function_call 变体，保留旧测试。至少覆盖两个串行 Todo 的完整闭环，以及一个证据不足后 retry/补任务的场景；并行能力另用两个独立探索验证。

必须使用正式 plan_simple 提示词、真实工具 schema、xagent 和子会话运行链。测试驱动只准备材料、启动和检查，不能替模型创建 Todo、填写 intent、接受报告或修补状态。

保留父/子 sid、冻结 behavior/schema、TodoList 与 attempts、worklog、background_context 注入及版本记录、报告和产物证据。验收能直接回答：每次调用想做什么、实际做了什么、返回后模型看到了哪些状态变化、哪个子会话执行哪个 Todo、依据什么验收、父 Turn 是否始终为同一个。

针对本次选型原因，记录真实 provider、模型和拒绝/403 的诊断信息。若已有可复现的旧 behavior 失败样例，在同任务、同 provider 条件下对比新模式；明确区分模型/协议拒绝与鉴权、权限等错误，不能仅凭状态码断定是 CoT 限制，也不能把一次成功当成所有 provider 都不会再拒绝。

## 9. 公开经验与取舍依据

- [Claude Code Subagents](https://code.claude.com/docs/en/sub-agents)：有界委派、独立上下文、结果交回主会话。用于参考父子职责与上下文控制。
- [Claude Code Agent Teams](https://code.claude.com/docs/en/agent-teams)：lead、独立 worker、共享任务、依赖和完成检查；该功能是实验性的，不直接照搬其所有生命周期机制。
- [Anthropic 长任务 harness](https://www.anthropic.com/research/effective-harnesses-for-long-running-agents)：外置状态、增量任务和验证后的交接。

上述经验支持任务隔离、明确证据和持久状态，并不规定一个 Todo 对应父会话一个 Turn。本项目采用“整个 Plan 一个 Turn、每次 function call 带 intent”，是本次讨论后的明确选择；不要重新引入此前讨论过的逐 Todo 关闭父 Turn 方案。

完成后在本文追加实际实现入口、测试命令与结果、真实模型证据位置，以及未验证的平台/provider（见 §10）。

## 10. 实施记录（2026-10-25，未提交）

### 10.1 落点

| 项 | 落点 |
| --- | --- |
| behavior 与创建入口 | `src/apps/jarvis_agent/agent/behaviors/plan_simple.toml`（function_call、`[capabilities] call_intent = true`、`[input] background = "after_tools"`、`tool_whitelist = ["group:bash", "report"]`、`max_tool_iterations = 160` / `max_consecutive_errors = 6`、`llm.plan`）；`agent.toml [session.plan_simple]`（`base = "work"`、`default_behavior = "plan_simple"`、`completion = "explicit_report"`、`max_process_depth = 8`、`ask_user_timeout_secs = 10800`）。创建：`agent-session create-worksession --class plan_simple --objective "…"`（`xagent new --class plan_simple`）。`[session.work]` 未改。 |
| intent 配置与冻结 | `protocol/behavior.rs` `BehaviorCapabilities.call_intent` → `protocol/config.rs` `BehaviorEntry.call_intent`（behavior loop 声明即拒绝）；`runner/live.rs::own_run_context` 只在 function_call 时写 run.json `host.extra.call_intent = true`，并在运行协议开头补一句契约。恢复、重建工具、fork、压缩后都从 run 记录读取。 |
| intent 执行 | `runner/tools.rs`：`with_intent`（每个函数 schema 加 required `intent: string, minLength 1`，report 保持 `additionalProperties: false`）、`without_intent`（缺失 / 空白 / 非字符串 → `Observation::Error`，调用不执行、不登记 in-flight）、`SessionToolManager`：`list_tool_specs` 增补 schema，`call_tool` 先校验再执行；in-flight 记录、快照、worklog 保留原始参数，`report` 宿主分支、consolidation 工具与内层工具拿到去掉 intent 的副本。 |
| Todo 派发 | `agent_tool::todo_tools`：`TodoList::admit_serial`、`TodoRecord/NewTodo/TodoAttempt/AttemptTarget.behavior`、`TodoRecord::view_line`、`task_text` 依赖行带 `read-session` 提示与 artifacts；`lib_opendan/src/todos.rs`：`dispatch_as` 对新串行 attempt 在列表锁内调 `admit_serial`，`todo_behavior`（派发指定 > Todo 记录 > 形态缺省），绑定时写回 behavior；`sync` 对独立 Work Session 的串行 attempt 改用 `observe_child`（原 `end_serial` 每次对账都 bump rev）。`xagent_cli.rs`：`todo dispatch` 按 Todo 形态走 `dispatch_serial` / `dispatch`，`todo add --behavior`，`add` / `dispatch` / `retry` 的 note 按会话是否有 Todo 执行器（behavior loop + `[entry] todo`）给出 DO 或 `todo dispatch` / `agent-session wait` 用法；用法文本更新。Jarvis `plan.toml` 的 `todo dispatch` 说明改为“重派被拒的探索，串行交给 DO”。 |
| 等待回填 | `runner/live.rs::fill`：回填含 `session:<sid>` 的结果、发布快照、提交 attention 之后 `todos::sync`；崩溃在其间时由 drive 开头的对账补。`xagent_cli.rs::pending_on_session` 给 `--wait-ms` 设 `check_after`（见 10.2）。 |
| 背景 | 新模块 `runner/observe.rs`（`EXTRA_BACKGROUND`、`at_boundary`）；`runner/hook.rs::before_inference` 在 boundary（发布快照）后调用；`protocol/input.rs` `HostMeta.observed / observed_epoch`；`protocol/config.rs` `InputSection.background: InputBackground {batch, after_tools}`；`runner/children.rs::end_delivered`；`runner/background.rs` 的 `child_line / digest / BackgroundLine::new` 改为模块内可见。 |
| 文档 / 手册 | `cmd_skills/opendan_agent_dev.md`（新键、`[session.plan_simple]`、plan_simple 与 PLAN/DO 的区别）；`doc/opendan/Agent TodoList.md`（v3，§4a）；`protocol/Session Directory Protocol.md`（HostMeta 与 run.json 键）；`protocol/Agent State Protocol.md`（behavior TOML 键）；`doc/sdk/OpenDAN_Agent_Dev_Guide.md` §5.1；`doc/opendan/protocol/schema` 重导出（`behavior_config / behavior_entry / host_meta / session_config / todo_list`）；`test/plan_do_todo_dv/README.md`。`cmd_skills/xllm.md` 不改：xllm 的工具契约与 CLI 未变（intent 是 libopendan 宿主对冻结 behavior 的要求，不进 `AiToolCall`、不进 xllm schema）。llm_context 层改动记在 [llm_context AgentRuntime TODO §11.2](llm-context-agent-runtime-todo.md#112-plan_simple-实施时的补充改动2026-10-25)。 |

### 10.2 实施时的决定与修正

- **intent 是 run 级冻结配置**：behavior 键 `[capabilities] call_intent` 决定，写进 run.json，`SessionToolManager` 从 run 记录读，不在代码里判断 behavior 名；子 `work_simple` / `explore` 不声明，契约不受父影响（测试中子会话不带 intent 的调用照常执行）。
- **背景只放 Todo 与未绑定 Todo 的子 Session**：每个 Todo 一行（与视图同一个 `<todo …>` 元素，XML Step 边界的视图与函数结果边界的观察是同一事实）；子 Session 运行中只报状态，不报活动摘要，避免每批结果都有噪音。Session 自己的 runtime / workspace 冻结、首条输入已给出，不进背景（`runtime.info()` 在本机会起探测进程、SSH 会走网络，不适合每个批次调用）；工作 Session 的观察范围是 `events`，不含其它活动会话；半订阅事件仍只随受控输入批次（需要回执清除）。
- **去重与提交**：已展示行记在快照 `HostMeta.observed`（行键 → 摘要），随注入消息一起发布；hook 第二次调用（注入后）发布带消息的快照。function call 下第二次调用末条已是用户消息，不会再注入；历史重写后 `observed_epoch` 不等于 `history_epoch`，重新展示。
- **“已收到”**：背景里已经展示了结束状态（行摘要与当前一致、快照已发布）的子 Session 记 `attention = finished`，与被 wait 回填的子 Session 相同；否则 PLAN 没有 wait 的并行探索会让结束检查永远报 “child sessions are unsettled”。
- **串行准入**：`todo dispatch` 不再把串行 Todo 转成探索；串行准入（pending、无其它串行在执行、无未 Review 结果、`after` 已接受）与 attempt 记录在同一把列表锁内；准入拒绝不记 `refusals`（不是并发闸），退出码 3。旧 PLAN 的 `settle` 路径经同一检查（`serial_pick` 已保证）。同一 attempt 重复派发返回同一子 Session。
- **既有缺陷修正**：`agent-session wait --wait-ms` / `create-worksession --wait --wait-ms` 的截止时间只写在 `details.until_ms`，shell 转发 Pending 时读的是 `check_after`（秒），截止时间从未生效；改为同时设 `check_after = ceil(ms/1000)`。
- **wait-any**：不做；提示词写明 wait 只等一个，其余结果靠背景与 `todo list` 看到。
- **普通文本**：`explicit_report` 下普通文本回复使 Session 以 `missing_end_report` 失败（既有行为），没有新增同 run 纠正；提示词写明只有 `report(is_end=true)` 结束。

### 10.3 验证（`buckyos/src/` 下，root，`--test-threads=1`）

- 新增 `cargo test -p libopendan --test plan_simple`（6 项，安装本仓库的 Jarvis 包为 AgentRoot，模型脚本化，其余真实：`xagent` 作 `agent-session`、ChildDriver 驱动子会话）：
  - `the_plan_simple_class_and_its_function_contract`：类的 `explicit_report` / 默认 behavior，`[session.work]` 仍是 `plan`；模型实际收到的 `shell` / `report` 都有 required `intent`，report 仍 strict；运行协议有契约句、无 XML action / next_behavior；冻结的 behaviors 无 `do`；子 behavior 不声明 call_intent。
  - `two_serial_todos_in_sub_sessions_in_one_turn`：缺失 / 空白 / 非字符串 intent 被拒且无副作用（同一批中合法调用照常执行），修正后同一 Turn 继续；T02（`--after T01`）在 T01 被接受前派发被拒；T01、T02 各在继承 Workspace 的 `work_simple` 子会话执行（父在 wait 时交出 Workspace，无死锁）；重复派发返回同一子会话；wait 回填后第一次推理即可 `todo show --report`（报告非空）并 Review；T02 未收敛时 `report(is_end)` 被拒；背景只在变化时出现（重复派发、被拒的结束不产生背景），wait 后显示 completed + review pending，两调用批次的背景排在两个结果之后；worklog 只有一个 `turn_started` / `turn_ended`、无 `step`；每个执行过的调用的 intent 保留在 worklog；子会话各有自己的 Turn。
  - `parallel_explorations_a_retry_and_ends_heard_from_the_background`：两个探索并行（执行窗口重叠）；A 被 reject 后 retry（第 2 个 attempt、新子会话、runtime / workspace / behavior 不变，旧 attempt 与报告保留）；B 从未被 wait，靠背景展示结束即算收到，最终报告一次通过。
  - `a_wait_deadline_and_a_failed_sub_session`：`wait --wait-ms 500` 在子会话仍运行时以 running 状态返回；之后子会话失败，回填时列表已是 failed、背景显示 failed + review pending；Review 为 reject 后以说明失败的报告结束。
  - `the_observation_survives_a_crash_before_and_after_its_commit`：子进程宿主在 wait 后的注入发布前（`observe:before_inject#3`）或发布后（`observe:after_inject_commit#3`）abort；新宿主下该观察只出现一次，同一个 Turn，子会话不重建、不重跑。
- 组件：`cargo test -p agent_tool --lib todo_tools`（新增 `serial_admission_and_a_kept_behavior`）；`cargo test -p libopendan --lib runner::`（新增 `runner::tools` intent schema / 剥离、`runner::observe` 选择）。
- 改动的既有用例：`plan_do_todo::tool_layer_replays_and_crashes` 的 T03 原先 `todo add` 后 `todo dispatch`（依赖“串行被转成探索”），改为 `todo explore`（同一崩溃点）；`opendan/tests/loader.rs::the_jarvis_package_loads_and_answers` 增加 `plan_simple` 的解析与入口断言。
- 全量（2026-10-25）：`cargo test -p agent_tool --lib` 275 通过 / 5 忽略；`cargo test -p libopendan` 全部 test target 通过（plan_do_todo 10、plan_simple 6、report 19、xagent_cli 11、xagent_lib 21、fixtures 19、workspace_binding 9、session_runtime 6 …）；`cargo test -p opendan` 通过（loader 25 + 1 忽略）。`xagent schema` 重导出；fixtures 用旧文件即通过（重生成只改变 run id / 时间戳，未提交）。
- 未执行：`uv run buckyos-build.py -s jarvis_agent`、`./build_aios --local-test`、真实 Zone DV；跨 runtime（节点 / SSH）的 Todo 交接与 Windows / macOS。

### 10.4 真实模型验收（2026-10-25）

驱动：`PLAN_DO_CASE_ROOT=/home/lzc/project/plan_do_todo_dv_runs PLAN_DO_DV=1 python3 test/plan_do_todo_dv/main.py all --mode plan_simple --variant normal|failure`（在 `buckyos/` 下）。provider：本机 CLIProxyAPI（`http://127.0.0.1:8317/v1`，OpenAI 兼容，Codex 账号），`llm.plan` / `llm.code` / `llm.chat` 都映射到 `gpt-6-sol`；正式 `plan_simple` / `work_simple` / `explore` 提示词、真实工具 schema、`xagent` 与子会话运行链；测试驱动只准备材料（只读 fixture、注册 `$CASE/workspace`、`xagent new --class plan_simple --workspace <id>`）、启动和检查。任务文本 `task_simple.md` 只说交付物要在 workspace，不规定 Todo 的 runtime / workspace 怎么选。

| 用例 | 结果 | 证据目录 |
| --- | --- | --- |
| normal | 40/40 通过；236 s；600 522 tokens（父 + 4 个子会话） | `/home/lzc/project/plan_do_todo_dv_runs/20261025T185759-plan_simple-normal/` |
| failure（fixture 缺 `idempotency.py`） | 34/34 通过；385 s；1 008 476 tokens（父 + 6 个子会话） | `/home/lzc/project/plan_do_todo_dv_runs/20261025T190320-plan_simple-failure/` |

可直接回答的问题（证据文件在用例目录下）：

- **每次调用想做什么、实际做了什么**：`parent/intents.json`（call_id、工具、intent、命令）；normal 22 个调用全部带 intent（如“等待调查 A 子会话完成以读取库存失败路径的实际报告。”），没有因缺 intent 被拒的调用，最长的 intent 53 个字符。
- **返回后看到了哪些状态变化**：`parent/backgrounds.json`（normal 14 个 `<<background_context>>`，只含 `<todo …>` 行与一句话结果，无子会话工具输出）。
- **哪个子会话执行哪个 Todo、依据什么验收**：`parent/todos.json`（attempt → child_sid、behavior、workspace_id、report_ref，review 理由）、`children/<sid>/`。normal：T01/T02 探索并行（窗口重叠）→ 各自 `wait` → `todo show --report` → accept；T03（`--after T01 --after T02`）→ T04（`--after T03`，写 workspace）串行、各一个 `work_simple` 子会话；计划自己读回 workspace 的两个文件后 accept T04。
- **workspace 分配**：探索都没有 workspace（读 fixture 绝对路径）；T03、T04 继承 workspace，`review.md` / `decisions.md` 由 T04 写入 workspace，引用每项发现的 Todo、子会话 ID、报告与复现证据。
- **父 Turn 是否始终一个**：两例父会话都只有一个 `turn_started` / `turn_ended`，`turn_seq = 1`，无 `step`、无 DO。
- **证据不足后的处理**（failure）：T02 第 1 次 `need_evidence` → `retry`（理由写明需核查合法模块来源）→ 第 2 次 `reject`；依赖它的 T03 同样 need_evidence → retry → reject；最终报告写明只证实了库存层缺陷，租户隔离与公开入口不能裁定、缺什么才能重跑。
- normal 中出现一次可修正错误：最终 `report` 把 workspace 的绝对路径放进 `artifacts`（“artifact paths must be relative files”），模型随即修正；之后把提示词的完成段改为“workspace 中的文件在报告正文中写明，`artifacts` 只接受自己工作目录的相对路径或对象 id”（failure 例在改后运行，没有再出现）。
- **403 / 拒绝**：两例的父、子会话 worklog 与 `drive.stderr.log` 中没有 403、`invalid_prompt` 或拒绝（`provider.refusals_recorded` = none）。项目里唯一记录过的拒绝样例是 2026-10-12 DO 步骤的 `invalid_prompt`（约 1/14 偶发，已随 `<thinking>`→`<intent>` 改名缓解，见该次记录），现在没有可复现的旧模式失败样例，因此没有做同条件对比；两次成功只说明这个 provider / 模型下没有遇到拒绝，不代表其它 provider 不会拒绝。

测试驱动的修正：第一次启动时 `prepare` 里一个循环变量覆盖了 `mode`，用例按旧 PLAN 跑了约 1 分钟即被手动中止，该目录已删除；`background.no_child_output` 检查最初把 Todo 的一句话结果（子报告首行，设计上展示）误判为泄漏，已改为排除各 attempt 的 summary 后对同一用例重跑 `verify` 通过（没有重新调用模型）。

### 10.5 未做 / 风险

- 跨 runtime（`--runtime` 指向节点 / SSH session-runtime，Workspace 在另一台机器）的 Todo 交接、`agent-session copy` 搬运，只有单元与既有集成测试覆盖，没有 plan_simple 的真实模型或 DV 用例；Windows / macOS 未运行。
- 未在真实 Zone 上跑（Loader 托管、`create-worksession --class plan_simple` 由 UI / Goal 发起），也未执行 `uv run buckyos-build.py -s jarvis_agent`、`./build_aios --local-test`；意图分析（`analyze_intent`）没有提到 plan_simple，用户目前只能显式选择该类。
- `explicit_report` 下模型若回普通文本，Session 直接失败（既有行为），没有同 run 纠正。
- 背景按批次结束时的状态生成：子会话在上一个批次之后才结束、计划随即 `report(is_end)` 时，结束检查先拒绝一次（“child sessions are unsettled” / 列表未收敛），这次错误结果之后的背景带出该结束，计划 Review 后再提交；不会误判通过，但多一轮推理。
- 背景只覆盖 Todo 与未绑定 Todo 的子会话；其它活动会话与半订阅事件不在函数结果后呈现。
