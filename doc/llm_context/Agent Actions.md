# Agent Actions

> 本文定义 OpenDAN Behavior 模式下 `<actions>` 容器的最终形态（v2）。读完本文，你应该掌握：
>
> 1. Action 为什么存在、为什么必须保持极小集合
> 2. v2 的 7 个 Action 各自的语义、参数、XML 形态
> 3. `<report>` 跟 `<next_behavior>` 的关系、`last_report` 的生命周期
> 4. 从旧版 `<action tool="...">` 协议迁移过来需要做什么

**实施状态（2026-10-10）**：两套 parser、运行循环、xllm 接手和 Session 宿主已实现 `<report end="true">` 完成协议；`<next_behavior>` 保留切换和等待语义，END / done 被拒绝。协议与恢复规则见 §2.2，验收记录见 [Context Switch TODO](../../notepads/llm-context-switch-support-todo.md) 的 H4。

---

## 0. Preamble — 设计意图与准入原则

### 0.1 Action 不是 tool registry

Action 看起来像 ToolManager 注册表的 XML 投影，但**它不是**。tool registry 关心"运行时有什么能力可调用"；Action 集合关心"提示词里 LLM 见过哪几种标签可以输出"。两者是错位的：

- **tool registry 由 runtime 决定**：插上 MCP、装上 plugin、本地有什么二进制，它就长出对应能力。
- **Action 集合由 prompt 决定**：LLM 没在提示词里见过 `<foo_bar>` 的示例，它不会输出。即使 ToolManager 注册了 `foo_bar`，对 Behavior 模式来说也等于不存在。

所以 Action 集合不能随 ToolManager 自动膨胀。**它必须是一个手工维护的、与提示词模板严格对齐的、极小的固化集合。**

### 0.2 为什么需要 Action：bash 写不出来的"写"

如果 Action 只是为了"调用一个工具"，那 `shell` 已经包打天下——任何 shell 表达式都能跑。Action 真正不可替代的场景只有一个：

> **把一段大段任意内容写进文件 / 发出去 / 落进 session**。

bash 表达"写内容"是地狱级难度：heredoc 转义、`$` 反引号冲突、文件内容里再出现 EOF 标记、JSON 嵌套 JSON……让 LLM 用 `echo >>` 拼出一个 React 组件或一篇 Markdown 文档，准确率会塌方。

而 XML body 配 CDATA 是 LLM 训练里见过最多的"原文嵌入"语法之一，几乎不会出错。

**所以 Action 的存在理由是收敛到一个动作：把原文嵌入一个 XML 标签里，让运行时把它"写"到某个目的地。** 至于"目的地"是文件、是上层 Agent 的信箱、还是 session 内存，是 Action 名字的区别。

### 0.3 准入原则

新增 Action 提案必须同时通过这两条：

1. **bash 不可表达**：这个能力没办法以"一条 shell 命令 / 一个 PATH overlay 里的 shim 二进制"的形式给 LLM 用。
2. **必须走 session 内存**：能力本身依赖 session 的可变状态（事件订阅句柄、内存 KV、worksession 上下文），不能被无状态命令覆盖。

任何一条不通过——**默认不加 Action，做成 shim**。OpenDAN runtime 已有 `BinOverlayConfig` 4 层 PATH overlay 机制（Session > Agent > Runtime > System），所有"调用一个能力"型需求应该走这条路。

> **PR 评审标准**：每个想往 `<actions>` 里加新标签的 PR，必须显式回答"为什么不是 shim？"。

### 0.4 Non-Goals

- ❌ Action 不追求"覆盖所有可能的工具调用"——那是 tool registry 的职责，跟 Behavior XML 协议无关。
- ❌ Action 集合不追求"按需扩展"——它是一个 prompt-coupled 固化集合，扩展需要同步改提示词模板。
- ❌ Action 不解决"如何调用一个新二进制"——加 shim 到 overlay 即可。
- ❌ Action 不替代 ToolManager——`shell` 之外的 tool 调用，要么走 shim、要么直接走 provider native tool_calls 通道，不进 `<actions>`。

### 0.5 关于版本

v2 是 beta2.2 节奏下的**breaking change**：旧的 `<action tool="...">` 协议、v1 Action 集合、对应的 ToolManager 注册项一并废弃，不保留向前兼容、不留 deprecation shim。所有 Behavior 提示词模板与本版本同步切换。本文涉及"v1"/"旧"字样仅用于说明差异来源，不代表运行时还会接受 v1 形态。

---

## 1. Action 集合（v2）

固化为 **7 个 Action**，外加一个 actions 外的 LastState 标签 `<report>`：

| 组别 | Action | 准入理由 |
|---|---|---|
| **写大段内容**（核心） | `write_file` / `edit_file` | bash heredoc 转义不可行 |
| **LastState 标签** | `report` | 输出物要进 `LLMContext.last_report`，bash 表达不了；不放入 `<actions>` |
| **过程通信** | `sendmsg` | 消息要路由到 user / agent / room，不应污染 `last_report` |
| **读取**（协议化） | `read` | 占位为"万能读"，承接绕过 bash 输出截断 + 协议化扩展 |
| **执行** | `shell` | 通用 shell，所有非"写大段内容"的能力都走它 |
| **session 控制** | `subscribe_event` / `unsubscribe_event` | 异步注册句柄，挂在 session 上，一次性命令表达不了 |

### 1.1 `<shell>`

```xml
<shell cwd="src" timeout_ms="30000">ls -la | head -20</shell>
```

| 字段 | 形态 | 说明 |
|---|---|---|
| body | text | shell 命令，必填，非空 |
| `cwd` | attr | 工作目录，必须在 workspace 之下 |
| `timeout_ms` | attr | 超时（毫秒），被 `max_timeout_ms` 钳制 |
| `env` | 不支持 attr | 需要环境变量请在 body 里写 `FOO=bar cmd ...` |
| `target` | attr | 执行目标，默认 `local`，预留未来 tmux / 容器 |

实现：复用 [`ExecBashTool`](../../src/frame/agent_tool/src/llm_bash.rs)，仅 XML 适配层变更。

### 1.2 `<write_file>`

```xml
<write_file path="src/foo.rs"><![CDATA[
pub fn bar() -> u32 { 42 }
]]></write_file>
```

| 字段 | 形态 | 说明 |
|---|---|---|
| body | CDATA / text | 文件全量内容（覆写语义） |
| `path` | attr | 目标路径，必须在 workspace 之下；不存在则创建 |

不接受 base64、不接受 JSON 嵌套——body 即原文。

### 1.3 `<edit_file>`

```xml
<edit_file path="src/foo.rs">
  <old_string><![CDATA[println!("hello");]]></old_string>
  <new_string><![CDATA[println!("hi");]]></new_string>
</edit_file>
```

| 字段 | 形态 | 说明 |
|---|---|---|
| `path` | attr | 目标路径 |
| `old_string` | child body | 要被替换的原文，必须在文件中只出现一次 |
| `new_string` | child body | 替换后的文本，必须不同于 `old_string` |

如果 `old_string` 没有命中或命中多处，`edit_file` 失败，不做修改。

### 1.4 `<read>` —— uri 风格的"万能读"

```xml
<read uri="src/foo.rs" offset="0" limit="4096"/>
<read uri="src/foo.rs"/>
<read uri="file:///absolute/path/under/workspace/src/foo.rs"/>
```

**决策：uri 风格**。有 `://` 协议头时，scheme 即协议，由解析器按 scheme 分发；没有 `://` 协议头时，`uri` 默认按 `file://` 文件路径处理。其它通用参数（offset / limit）以 attribute 形式给出，protocol-specific 寻址放在 uri 里。

| 字段 | 形态 | 说明 |
|---|---|---|
| `uri` | attr | 必填。协议 + 地址；无 `://` 时默认是文件路径 |
| `offset` | attr | 通用读起点（字节或行，由 scheme 定义） |
| `limit` | attr | 通用读上限 |
| body | 空 | `<read>` 永远是空标签 / 自闭合 |

**v2 首版只实现文件读取**：显式 `file://` 和无协议头的文件路径等价。它的存在理由是"绕开 `shell` 的 `max_output_bytes` 截断"，所以它必须支持分页（`offset` / `limit`），并且**不被 `max_output_bytes` 限制**——否则不如直接 `cat`。

**路线图**（不在 v2 首版）：

- `kv://agent/foo` — 读 agent KV 存储
- `event://session/last?type=...` — 读事件历史
- `http://...` — 受策略约束的外网读取
- `mcp://server/resource` — MCP resource 桥接

> 注：v2 删除旧的 `read_file` Action。`read` 是改名后的占位；文件路径可直接写成 `src/foo.rs`，不必强制写 `file://`。

### 1.5 `<report>` / `<sendmsg>` —— LastState 与过程通信分离

```xml
<!-- Self report：更新当前 LLMContext.last_report -->
<report><![CDATA[
任务完成，产出文件 src/foo.rs，覆盖 3 个用例。
]]></report>

<!-- SendMessage：过程通信，不写 last_report -->
<sendmsg target="user"><![CDATA[
进度更新：已完成第 2 步，正在执行第 3 步……
]]></sendmsg>

<sendmsg target="agent://reviewer"><![CDATA[
请审阅 src/foo.rs 的实现是否符合规范。
</sendmsg>
```

`<report>` 字段：

| 字段 | 形态 | 说明 |
|---|---|---|
| body | CDATA / text | 写入 `last_report` 的正文 |

`<sendmsg>` 字段：

| 字段 | 形态 | 说明 |
|---|---|---|
| body | CDATA / text | 消息正文 |
| `target` | attr，必填 | SendMessage 到指定收件方 |

**语义分支：**

- **`<report>`：Self Report / LastState Update**
  - 内容写入当前 `LLMContext` 的 `last_report` 字段（覆盖语义）
  - **不**自动终止 Behavior loop——是否终止由 `<next_behavior>` 决定
  - 主要用途：fork 一个 llm-context 跑子任务，子任务结束后从其终态快照里读 `last_report`
- **`<sendmsg target="...">`：SendMessage**
  - 内容路由到 `target` 指定的收件方（`user` / `agent://name` / `chat://room` 等）
  - **不**写 `last_report`，**不**终止 Behavior loop
  - 主要用途：进度反馈、跨 Agent 通信、Delegate 任务后的后续沟通

**为什么不合并语义：** 见 §3。

### 1.6 `<subscribe_event>` / `<unsubscribe_event>`

```xml
<subscribe_event topic="kv.changed" filter="prefix:agent/foo/"/>
<unsubscribe_event subscription_id="sub-7f3a"/>
```

这一对是 v2 唯一保留的控制面 Action。准入理由：

- bash 表达不了"注册一个长期句柄并让后续消息走回调"
- 注册结果（subscription_id）必须挂在 session 上，无状态命令做不到

订阅语义与配置职责见 [Agent Session 架构设计 §12.8](<../opendan/OpenDAN Agent Session架构设计.md#128-订阅唤醒与输入装配>)；具体参数由 Runtime / SDK 收敛。

### 1.7 与 v1 的差异

| v1 Action | v2 处置 | 替代方案 |
|---|---|---|
| `shell` | 保留 | — |
| `read_file` | **删除** | 改名 `read`，uri 风格 |
| `write_file` / `edit_file` | 保留 | — |
| `Glob` / `Grep` | **删除** | `find` / `grep` / `rg`（走 shell） |
| `get_session` / `list_session` | **删除** | shim：`opendan-session list` |
| `create_workspace` / `bind_workspace` / `list_external_workspaces` / `bind_external_workspace` | **删除** | shim：`opendan-workspace ...` |
| `load_memory` / `set_memory` / `remove_memory` | **删除** | shim：`opendan-memory get|set|rm` |
| `todo_manage` | **删除** | shim：`opendan-todo add|done|list` |
| `subscribe_event` / `unsubscribe_event` | 保留 | — |
| **新增** `report` | — | 更新当前 `LLMContext.last_report` |
| **新增** `sendmsg` | — | 过程通信 / 跨 Agent 通信 |

被删除的所有"通过 shim 替代"的 Action，需要随 v2 一起在 overlay 里提供对应的 shim 二进制，否则迁移阻塞。shim 的实现位置与命名规约在 [`Agent Enviroment.md`](../opendan/Agent%20Enviroment.md) 里定义。

---

## 2. XML 协议（v2）

### 2.1 顶层结构

```xml
<response>
  <observation>...</observation>
  <thinking>...</thinking>

  <actions>
    <shell>cargo test</shell>
    <write_file path="src/foo.rs"><![CDATA[ ... ]]></write_file>
    <sendmsg target="user"><![CDATA[已开始测试...]]></sendmsg>
  </actions>

  <report><![CDATA[
本步骤完成总结...
]]></report>
</response>
```

**关键变化：**

1. 新增 `<actions>` 容器。所有 Action 是它的直接子元素，**一级标签即 Action 名**，不再用 `<action tool="...">`。
2. **Self Report `<report>` 在 `<actions>` 外面**——它是当前 `LLMContext` 的 LastState 更新，跟 `<observation>` / `<thinking>` / `<next_behavior>` 同级。
3. **SendMessage 形态用 `<sendmsg target=...>` 放在 `<actions>` 里面**——它是这一步内执行的副作用动作之一。
4. `<next_behavior>` 保留 behavior 切换和 `WAIT_USER_MSG` 等待输入的语义；切换前仍须观察 action 结果。完成改用 `<report end="true">`，不再输出 `<next_behavior>END</next_behavior>`。终结报告不能与 actions 同时出现；先执行动作、观察结果，再单独提交终结报告。
5. LLM 不需要给每个 Action 输出 ID。运行时执行前会为 dispatchable action 分配上下文内唯一的自增 `call_id`；之后的推理把这个 Step 渲染成 assistant/user message pair 时，assistant 消息保持 LLM 原文，`last_step_action_results` 里每个结果的标题前加 `#<call_id>` 前缀（继承到 `<step_record>` 里的结果不带编号）。

### 2.2 `<report>` 与 `<next_behavior>` 的共存规则

**当前契约：** `<report>` 提交报告，`end` 缺省或为 `false` 时不请求结束；`end="true"` 在提交报告的同时请求结束当前 context。报告不等同于向用户发送消息，接收方和结束范围由宿主按调用关系裁决。`<next_behavior>` 继续表达切换或等待，不承担完成信号。

| 出现 | 含义 |
|---|---|
| `<actions>…</actions>` | 执行动作，观察结果后继续 |
| `<report>` 或 `<report end="false">` | 提交阶段性报告，更新最近报告；不因报告本身结束 |
| `<report end="true">` | 提交最终报告并请求结束当前 context；不得同时输出 actions |
| `<next_behavior>CHECK</next_behavior>` 等目标 | 按目标进入模式切换或调用 context；不表示 Session 完成 |
| `<next_behavior>WAIT_USER_MSG</next_behavior>` | 保留等待输入语义；子 context 按宿主规则向调用方返回需要输入 |
| 阶段性 report 与 next_behavior | 提交报告后切换或等待；report 本身不增加结束意图 |

```xml
<response>
  <report end="true"><![CDATA[任务完成，结果说明……]]></report>
</response>
```

完成请求的范围是当前 context：子 context 的结果交给调用方，不能关闭父 context 或整个 Session；顶层 context 是否完成 Turn / Session 由宿主完成策略裁决。终结报告必须携带本次最终结果正文，不能隐式提升之前的阶段报告；最终报告正文必须非空，内容保持自由文本 / Markdown。`end="true"` 不等于产物已验收。传统 function-call 的 `report(..., is_end=true)` 与此共享报告和完成意图的语义；工具参数为 `report: string`、可选 `artifacts: string[]`、可选 `result: JSON`、可选 `is_end: bool`（缺省 false）。

**已实施的校验与副作用边界：**

- `end` 只接受精确的 `true` / `false`，缺省 false；重复 report、重复 end、重复 next_behavior、空最终报告进入纠错。
- 结束报告不得同时带非空 next_behavior、XML 动作、sendmsg 或同一 response 的原生 tool_calls。空 `<actions/>` 允许。冲突在原生工具派发、报告更新和其它副作用之前拒绝；此前 Round 已执行并观察的工具不构成冲突。
- 无动作、报告、切换或等待的空决策 / 纯 thinking 进入纠错，不隐式完成。普通 report 即使没有动作也继续。
- 可选的兄弟节点 `<artifacts>` 是 JSON 字符串数组，`<result>` 是任意 JSON 值；两者必须与 report 同次出现，各最多一次。宿主验证 artifacts 是工作目录内的显式文件，并将内容复制为稳定引用。
- `LLMBehaviorResult` / `StepRecord` 保存 `report_end`、`report_artifacts`、`report_result`；snapshot v5 保留完成意图。恢复交付本次 final Step / report journal 的正文，不提升旧阶段报告，也不交付 XML 控制文本。
- Session 工具 `report` 的 `is_end=true` 被接受后，保存独立 journal，补齐同批未执行调用的结果配对，再由宿主收尾；不追加推理。XML 结束报告则从输入阶段禁止同批动作，两者使用相同的宿主报告与完成规则。

```xml
<response>
  <report end="true"><![CDATA[已完成验证，交付文件如下。]]></report>
  <artifacts>["output/result.txt"]</artifacts>
  <result>{"verified": true}</result>
</response>
```

### 2.3 转义协议：CDATA

**所有 `body` 字段统一使用 XML CDATA 包裹**。提示词模板里显式给出 CDATA 示例，LLM 训练里这语法见过无数次，跟得住。

```xml
<write_file path="x.md"><![CDATA[
任意内容，包括 </write_file>、`$var`、<tag>、\n 都不需要转义
]]></write_file>
```

CDATA 自身的闭合 `]]>` 在自然语言文本里几乎不会出现；如果真出现（例如写一篇讲 XML 的教程），约定用 `]]]]><![CDATA[>` 的标准 XML 拆分方式——这是 XML 规范本身的解，不引入新约定。

> 解析器同时识别 CDATA 与严格 XML escape（`&lt;` `&gt;` `&amp;`）两种 body 形态——这不是为了兼容旧协议，而是为了容忍 LLM 偶发的非 CDATA 输出。提示词模板里始终只示范 CDATA 形态。

### 2.4 解析容忍度

继承 v1 的宽松策略（[xml_behavior.rs](../../src/frame/llm_context/src/xml_behavior.rs) §Tolerance）：

1. 仍然剥 ` ```xml ` / ` ``` ` 围栏
2. `<response>` 仍可省略
3. 各 action 标签找不到闭合时 fallback 到"读到下一个已知标签或 EOF"
4. provider native `tool_calls` 仍优先于 `<actions>` 解析（用于 OpenAI/Anthropic function calling 场景）
5. 空 `<actions>` 本身不报错；若整个决策没有 report、动作或非空 next_behavior，则进入纠错，见 §2.2。

以上是已有解析容忍度；完成意图的校验不能直接套用宽松解析，尤其不能把非法 `end` 值解释成成功完成。

### 2.5 一个 Step 的产物与 Session Turn

一次 `<response>` 涉及几个不同层次的东西，不能互相替代（Round / Step / Turn 的定义见 [readme.md](readme.md)）：

- **决策（Step）**：一次 `<response>` 解析后记为一个 `StepRecord`。得到它之前，Step 内部可以经过多个 Round（原生工具 loop）；解析失败 / 策略拒绝生成的合成纠错 Step 也占一个 `step_index`。
- **动作批次**：这个 Step 的 `<actions>`，按序派发，遇到失败即停。带 action 的 Step 扣一次工具迭代额度（`ToolPolicy.max_tool_iterations`），与 action 个数无关；Step 内的原生工具批次也从同一额度扣。`<sendmsg>` 不经 ToolManager，只记为 `messages_sent`（xllm 把 `sendmsg` 显式配置为 action 时除外）。
- **`<report>`**：提交报告并更新最近报告，不自动投递；只有 `end="true"` 额外请求结束当前 context。
- **context 让出与完成**：`next_behavior` 的切换 / 等待与终结报告是不同原因；宿主必须区分，不能把所有 `Done` 都当作显式完成。当前实现按 `report_end` 区分显式结束，END / done 和空决策不再是完成入口。
- **Turn / Session 完成**：只由 Session 判定。子 context 返回和 context 切换保持 Turn 打开；`WAIT_USER_MSG` 保留等待语义。当前 libopendan 用本 run 是否有 `last_report` 或末 Step 是否有 `<sendmsg>` 决定等待时是否完成 Turn，这不等于报告已经投递；`session.policy.completion=explicit_report` 要求获准的结束报告才正常关闭 Session；普通 Done 在有输入队列的 Session 关闭 Turn 并等待，在无输入队列的 WorkSession 以漏报失败。消息侧的对照见 [Agent Message.md §3.7](<Agent Message.md>)。

---

## 3. `<report>` 详解

### 3.1 为什么 SendMessage 和 Report 要拆成两个标签

`<report>` 更新的是当前 `LLMContext` 的 LastState，`<sendmsg>` 表达的是对外过程通信。二者虽然都是"写一段 body 到某个目的地"，但目的地的生命周期和审计语义不同，继续用 `<report target=...>` 会让 action 通信和 last-state 更新混在同一个标签里。

拆分后的规则：

1. **`<report>` 负责结果提交**：更新最近报告，不带 `target`；`end="true"` 额外携带当前 context 的完成请求，由宿主裁决。
2. **`<sendmsg>` 只负责过程通信**：必须带 `target`，不写 `last_report`。
3. **位置表达语义**：`<sendmsg>` 放在 `<actions>` 内；`<report>` 放在 `<actions>` 外，作为本 Step 的状态产物。

### 3.2 阶段报告、终结报告与 context 切换

本轮将完成请求与结果提交放在同一份报告中，同时保留 context 切换 / 等待的独立字段：

- **阶段报告**：`<report>` / `<report end="false">` 可以反复提交，不请求结束。
- **终结报告**：`<report end="true">` 绑定本次最终结果与完成意图，不与 actions 同时输出。
- **切换 / 等待**：仍用 `<next_behavior>`，不把跳转目标塞进 report，也不再以 `END` 表示完成。

### 3.3 `last_report` 的生命周期 —— 跟 LLMContext 走

`last_report` 是 `LLMContext` 上的一个字段（覆盖语义，只保留最后一次 Self Report）。它的生命周期**完全等同于 LLMContext 自身的生命周期**：

- LLMContext 创建时 → `last_report = None`
- 每次 Self Report 执行 → 覆盖 `last_report`
- LLMContext 终止/快照 → `last_report` 跟着进入快照
- LLMContext 销毁 → `last_report` 一起销毁

**这跟 worksession 里的 llm-context 快照机制是同构的**——见 [`LLM Context 设计.md`](LLM%20Context%20设计.md) 的 Snapshot 章节。

**主要用途：fork-and-collect 模式（以下为新完成协议的目标流程）**

```
父 LLMContext
   ↓ fork
子 LLMContext（跑 sub-task）
   ↓ ... 多个 Step ...
   ↓ 最后一个 Step：<report end="true">子任务结果</report>（无 actions）
   ↓ 宿主接受并持久化该次终结报告
父 context 收到该次报告作为调用结果
```

子 context 不需要再让模型另发一条通信消息；宿主按已有调用关系把终结报告返回给父 context。`last_report` 仍可作为最近报告的状态视图，但不能代替最终提交记录。

当前实现（libopendan）：`create_sub_context` / `fork` 的 child 结束时，`finish_run` 把结果和 `ok | failed | needs_user_input` 放进 `state.process_result`；behavior 触发的调用经交接输入返回，工具触发的调用按 `call_id` 回填。child 返回不完成 Session Turn。`switch_context` 使用目标自己的快照，已无 normal 切换。

正常完成、恢复、父子返回和最终展示使用同一份已接受的终结报告：工具来源读取持久 report journal，XML 来源关联最终 Step，xllm 接手后在最终 Step 保存 paused + snapshot.state.report_end，宿主恢复会验证并提交该 Step；若宿主校验拒绝则继续纠错，不提前完成 run。没有终结提交时不能把 snapshot.last_report 中的阶段报告升级为最终结果。

**`sendmsg` 不进 `last_report`**：因为 SendMessage 有自己的收件方，已经"出去"了，不应该污染本 context 的快照产物字段。

### 3.4 与 Worklog / Memory 的关系

- **Worklog**：记录所有 step、所有 action 的完整流水（含每次 Self Report 的历史），是审计/回放用的。
- **`last_report`**：只是 LLMContext 当前的"对外暴露字段"，是给 fork 的父端用的最简产物口。
- **Memory**：跨 LLMContext 持久化的，由 Agent 显式写入（通过 shim `opendan-memory set ...`）。

三者职责清晰、互不替代。

---

## 4. Breaking Change 清单与影响面

### 4.1 一刀切的变更

v2 与 v1 之间没有 transition window，所有变更同步发布：

| 项 | 变更 |
|---|---|
| `<action tool="...">` 解析 | 删除，仅按 Action 名一级标签解析 |
| `read_file` / `Glob` / `Grep` Action | 从 ToolManager 注册中直接删除 |
| `get_session` / `list_session` / `*workspace*` / `*memory*` / `todo_manage` Action | 从 ToolManager 注册中直接删除，由 overlay shim 承接 |
| Provider native `tool_calls` 路径 | 不变 —— 仍优先于 XML 解析 |
| 提示词模板 | 全量切到 v2 形态，与本版本同 PR 合入 |

### 4.2 代码影响面

| 文件 | 改动 |
|---|---|
| [`src/frame/llm_context/src/xml_behavior.rs`](../../src/frame/llm_context/src/xml_behavior.rs) | 解析器主体改写：识别 `<actions>` 容器、一级标签即 Action 名、CDATA body 提取 |
| [`src/frame/llm_context/src/context_loop.rs`](../../src/frame/llm_context/src/context_loop.rs) | 派发逻辑：Self Report 直接更新 `LLMContext.last_report`；`<sendmsg target=...>` 走 message bus 记录；其它 Action 调 ToolManager |
| [`src/frame/opendan/src/behavior_cfg.rs`](../../src/frame/opendan/src/behavior_cfg.rs) | `tool_whitelist` 保留旧语义（ToolManager 暴露的工具名白名单）；v2 默认 Action 面包含 `shell`/`write_file`/`edit_file`/`read`/`sendmsg`/`subscribe_event`/`unsubscribe_event`。`<report>` 不进 whitelist——它在 parser/dispatcher 走特殊路径，不经过 ToolManager |
| `LLMContext` 结构 | 新增 `last_report: Option<ReportRecord>` 字段，进快照 |
| `src/frame/agent_tool` | v2 Action registry 不再注册 Glob / Grep / session/workspace/memory/todo_manage / read_file；新增 `read` Tool（带 uri scheme dispatch，无协议头默认文件路径） |
| Overlay shim 二进制 | 新增 `opendan-session` / `opendan-workspace` / `opendan-memory` / `opendan-todo` 4 个 shim |
| 提示词模板 | 所有 Behavior 提示词更新到 v2 XML 形态 |

### 4.3 验收门槛

v2 落地的最小验收集合：

1. **解析器单测**：覆盖 7 个 Action 的标签解析 + CDATA / 严格 escape 双形态 + `<sendmsg>` 与 `<report>` 的组合
2. **fork 集成测试**：父 LLMContext fork 子，子写 Self Report 后终止，父能从快照读到 `last_report` 内容
3. **shim 等价性测试**：对每个被删 Action，对应的 shim 通过 `shell` 调用能产出原 Action 的结构化结果
4. **提示词全量切换**：项目内所有 Behavior 提示词模板已重写为 v2 形态，端到端代表性任务用例通过

---

## Appendix A — 决策记录

| 编号 | 决策 | 理由摘要 |
|---|---|---|
| D-01 | Action 集合固化为 7 个，`report` 作为 LastState 标签独立存在 | bash 不可表达 + 必须走 session 内存的双重过滤 |
| D-02 | XML 用一级标签而非 `<action tool="...">` | LLM 提示词信号更强、训练对齐更好 |
| D-03 | body 用 CDATA | 转义最稳、训练分布最匹配 |
| D-04 | `read` 用 uri 风格 | 协议名即 scheme，参数同构 |
| D-05 | `read` v2 首版只实现 `file://` | 占名字、立框架，避免空头扩展 |
| D-06 | 阶段 report 不结束；`report end="true"` 提交结果并请求结束；next_behavior 保留切换 / 等待 | 2026-10-10 已实现；终结报告不得与动作或非空调度同现 |
| D-07 | `<sendmsg target=...>` 不写 `last_report` | 已"出去"的消息不污染本 context 产物 |
| D-08 | `last_report` 生命周期跟 LLMContext 走 | 复用快照机制，fork-and-collect 零成本 |
| D-09 | session/workspace/memory/todo 一律迁 shim | 准入原则的具体应用 |
| D-10 | `subscribe_event` 保留为 Action | 异步注册句柄，bash 不可表达 |
