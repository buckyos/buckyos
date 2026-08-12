# Agent Context Messages

说明在LLM Context + Agent Session的支持下，会构造出哪些典型的 Agent Context Message List

本文会从原始语义，说明相同目的下如何实现对LLM Context的Message List的管理

## 1个标准 Round

模式一 标准Agent Loop里的Message Pair: 通过一个Input User Message触发，得到Assistant Message结束。中间通常有多个tool call-result message pair

[system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1 [user [a-call u-result] [a-call u-result]  assistant]@round2 [user assistant]@round3 **user**

模式二 Behavior Loop里的一次beahvior run: 通过一个on_behavior_switch 触发，得到 behavior_result  (next_behavior有值)

[behavior-system] ([user:step0-step3-history + beahvior-on-switch]) [agent:intent user:last_step_result]@step4 [[a-call u-result] agent:intent user:last_step_result]@step5 **beahvior-on-switch + pending inputs** 

Behavior Step:通过last_behavior_results触发，得到下一个agent intent. 这个粒度比tool call->tool result要大 
[behavior-system] ([user:step0-step3-history + beahvior-on-switch]) [agent:intent user:last_step_result]@step4 [[a-call u-result] agent:intent user:last_step_result]@step5 agent:intent **user:last_step_result + pending inputs**


> Behavior Loop 可以选择在构造包含last_step_result的user_message时，读取session的pending-input,也可以只在on-switch（通常是一个激活节点）时读取pending input（就像传统AgentLoop必须得到一个assistant message之后才能读取pending input)


## User Message：推理的驱动力

> 这一节原理上属于 Agent Session 层（涉及 session 状态机决定何时让 LLM 继续推理）。本文从"什么样的 message 序列合法"角度切入，跟前面的 Round 抽象呼应。

### Round 不变量：UserMessage = 进入下一个 Round 的驱动力

抽象出 Round 概念的核心目的就是这条：**一个 UserMessage 的目标 = 推动进入 NextRound**。原则上 round 中间不应该插入 UserMessage。

- 标准 Agent Loop 严格遵守：只有 round 结束（assistant 完成）后才消费 pending input
- Behavior Loop 有 step 这个更细的颗粒，**step observation 阶段也可以消费 pending input**
  - `last_step_result` 更准确的名字是 `last_step_observation`——这是观察阶段
  - 观察阶段读 pending input，等价于"把外部新信号纳入下一步的输入面"

> 例外只在 **default / 顶层 behavior** 上合理（它本来就是状态机入口 + loader）。**Sub behavior 不应该消费 pending input**——会破坏 fork / 独立切换的隔离语义。真正干活的 behavior 通常是被 fork 出来的，更不该消费。

### Pending Input 的两类来源：Message vs Event

| 维度 | Pending Message | Pending Event |
|---|---|---|
| 信号强度 | 强 | 中 |
| 消费 | 显性，必然成为驱动力 | 可能完全不消费 |
| Ban 机制 | 不适用 | ✅ |
| 半订阅机制 | ❌ 不存在 | ✅ |

> Message 可以视为一种特殊的 Event，但语义差异足够大，值得分开处理。

#### Pending Message：强驱动力

到达 → 分发到 session → 进 pending input 区 → 序列化 → session 状态机决定时机消费。**只要 session 状态允许，必然成为驱动力。**

#### Pending Event：可被 Ban，可半订阅

**Ban 机制**：当前 session 状态生成一个 mask，不通过 mask 的 event 老实待在 pending input 区，不形成驱动力。

- 例：Wait User Message 状态下，所有非 user message 的 event（哪怕是 session 关心的 Delegate Task 完成）都被屏蔽
- 实现层：Filter Layer 用当前状态生成 mask；mask 只让特定 input 产生驱动力

> **TODO**：当前实现里 `PullEventPolicy::Filter(...)` 只支持静态字符串过滤（命名空间前缀），尚未做"按 session 当前状态动态生成 mask"。详见末尾"未完成事项"§1。

**半订阅机制**：event 本身**没有驱动力**，但在构造 UserMessage 时，渲染模板里的 Background Environment 段会观察这些 event 的当前状态并植入 message。当状态产生真正改变（通常是状态级变化）时，新状态进入 UserMessage 对应段落。

- 典型场景：Timer 事件。如果每个 tick 都成为驱动力，session 永远停不下来
- 落地形态：模板里有 `<background>` 段，半订阅事件源在这里只读取"当前快照"，不做驱动

### Session END 的两种解释

| 视角 | END 含义 | 新 input 行为 |
|---|---|---|
| WorkSession（OpenDAN 设计） | 硬关闭，session 变只读 | 必须开新 WorkSession |
| 通用 LLM Context | 一轮结束，default behavior 无强约束 | 新 pending input 直接驱动新 LLM Context 继续跑 |

> **硬关闭是产品级判断，不是技术限制**。现有 code agent（Cursor / Claude Code 等）面向能自己划清 objective 边界的专业程序员——一个任务做完自然就开新 session。但对普通用户而言，天然倾向于反复往同一个 session 里灌消息让它一直跑：objective 持续漂移、上下文越积越脏、产物归属不清。强制 read-only 等价于**替用户做"任务分段"决定**，逼他们在 objective 达成时显式开新 session。这条设计的实质是 OpenDAN 对目标用户群（不只是专业开发者）的判断。

OpenDAN WorkSession 的 END 是 route 层的设计抉择，核心假设：

1. 每个 WorkSession focus 在一个明确 objective 上
2. Objective 达成后 session 变只读，不应再进新消息
3. **WorkSession 与 Workspace 分离**：session = 确定话题（一次性），workspace = 持久状态（一直可改）
4. 新 session 的 single source of truth = workspace 文件，**不是**上一个 session 的历史

> 实际边界并不好控制：用户做完构建可能想接着部署，强相关任务有上下文最好。当前处理方式：用户向已结束 session forward message 时返回错误，明确告知大模型"必须开新 session 承载"。上一个 session 的历史以日志形式固化在 workspace，新 session 想读可读，但不强制喂入。

### Driver：驱动力归属在 Session 层（具体形态见 [Agent配置改进.md](./Agent配置改进.md) §4.2）

**驱动力（什么时候 fire / 从 pending queue 取什么 / 失败怎么处理）属于 session 层、不属于 behavior 层**——这条结论不变。为什么必须放 session 层：

1. **同一个 behavior 在不同 session 下驱动力本来就不同**（`chat_route` 在 ui session 是 per_peer，在 group 是 per_group）。绑死在 behavior 上无法跨 session 复用。
2. **同一个 session 多个 behavior 通常共享驱动策略**。每个 behavior 重复声明是 DRY 违反。
3. **Behavior 配置只剩纯渲染**，心智模型干净。

但配置形态在本节早期草案后做了**大幅简化**，当前在用的形态（beta2.2 落地）：

- `[session.<class>.driver]` 顶层挂 `keep_alive` / `switch_mode` / `inject_background_environment` / `report_delivery`
- 四个 hook point 子表：`on_init` / `on_behavior_switch` / `on_behavior_step_ob` / `on_wakeup`
- 每个 hook 只暴露三个枚举字段：`filter`、`pull_msg`（None/One/All）、`pull_event`（None/All/Filter）

被砍掉的早期设计（git history 里能查到原草案）：

| 早期形态 | 简化结果 |
|---|---|
| `fire = [...]` 触发规则数组 | 替换为四个固化 hook point |
| `bind.* = { from, strategy }` 显式绑定 | 替换为 hook 内 `pull_msg` / `pull_event` 直拉 |
| `on_failure = "rollback_all"` 事务策略 | 砍掉 |
| `pop_one` / `pop_all` / `pop_n` / `peek_all` binding 类型 + 模板引用追踪 + commit 对账双闸 | 砍掉 |
| 模板引擎"纯函数 + 幂等不变量" | 不再要求 |

**核心语义变化：从"显式 pop + 模板必须引用 + commit 对账 + 失败 rollback"塌缩到"pull = 推理即消费"**。pull_msg / pull_event 一旦在某 hook 触发，对应 pending input 在构造完 user message 后立即出队，behavior 模板是否引用不影响出队——简化代价是出现"pull 了但模板没用 ⇒ 静默丢失"路径，换来的是 driver / template / runtime 不需要三方对账，模板引擎也不必维持幂等不变量。

> driver 拉的 pending input 已经 freeze 进 render context，模板若有副作用也无所谓——runtime 不靠模板幂等性保证正确性。

### 两种 Loop 的 hook 时机对照

| 维度 | 标准 Agent Loop | Behavior Loop |
|---|---|---|
| 自动产生 UserMessage | ❌ 不会 | ✅ step 观察阶段可由 hook 触发 |
| Driver hook 时机 | `on_init` / `on_wakeup` | `on_init` / `on_behavior_switch` / `on_behavior_step_ob` / `on_wakeup` |
| Pending input 消费时机 | 只在 round 边界 | round 边界 + step 边界 |
| Render context 构造 | hook 触发时按 `pull_msg` / `pull_event` 拉取，喂给模板 | 同左 |



## 停止运行中的Round

外部信号（用户 `stop`/`cancel`、新 user input 到达、调度器决定换 behavior）在一个 round/step 进行中时切入。当前实现两种模式，分别对应不同的"代价 vs 痕迹"取舍。

抢占前共同形状（一个标准 round 正在跑，assistant 已发出多个 tool_use，部分 tool_result 还没回来）：

```
[system] [user [a-call u-result] [a-call ?in-flight?] **a-call pending** *partial assistant text*
```

### 模式 A：Graceful（温和收尾 / wind-down）

对应代码 `InterruptMode::Graceful`，由 `stop` 触发。

- **不打断**正在跑的 inference（即便 LLM 正在出 token 也让它出完）
- 给所有 pending 的 tool_use 补一个**合成的** tool_result = `Cancelled`
- 把 `max_rounds` 临时设为 0，让 LLM 不能再发新 tool_call
- 同 llm_context resume，让 LLM 在这个被截断的 round 里**自然吐一句 ack / 总结**收尾

抢占后形状：
```
[system] [user [a-call u-result] [a-call u-result=Cancelled] [a-call u-result=Cancelled] assistant: 收尾文本]
```

特点：

- partial assistant 完整保留
- 历史里能看到"被中断过"的痕迹（Cancelled 标记）
- 多花一次 wind-down 推理
- 后续 user input 走全新 round，但 KV cache 前缀保得最长

### 模式 B：Discard（硬截断）

对应代码 `InterruptMode::Discard`，由 `cancel` 触发。

- 立刻 fire `LLMContextInterruptHandle.interrupt(reason)` 把正在跑的 inference abort 掉
- 定位末尾那条带未完 tool_use 的 assistant turn，**整条切掉**
- 清空 `pending_tool_calls`，**不补任何 tool_result**
- 直接落盘截断后的 snapshot

抢占后形状：
```
[system] [user [a-call u-result]]
```
（末尾 assistant turn 连同它的 partial 文本和 tool_use 一起消失，像上一轮干净结束。）

特点：
- partial 工作完全丢失
- 历史看不出来被中断过
- 不花额外推理
- 截断点之后开新 round，KV cache 前缀变短（少了被切掉的那段）

### 选择维度

| 维度 | Graceful | Discard |
|---|---|---|
| in-flight inference | 跑完 | 即刻 abort |
| partial assistant | 保留 | 删除 |
| pending tool_use | 补 `Cancelled` 合成 result | 清空，无 result |
| 历史可读性 | 有中断痕迹 | 似无事发生 |
| 额外推理开销 | 1 次 wind-down | 0 |
| KV cache | 同 ctx 继续 append | 截断后下一轮 prefix 变短 |
| 典型场景 | 用户喊停但希望 agent 优雅交接 | 用户后悔了 / 调度器要 hard reset |


## 压缩 History Message

会把一部分处在U形注意力中间的message,压缩成压缩成更短的形式。压缩的结果通常有两种表现形式：要么是在原有的 message 上直接修改，要么就是把一组消息压缩成一个 消息对

压缩肯定会导致KV Cache失效，因此

- 压缩历史记录后，通常会留出较大的空间，支持未来的几轮
- 可以积极的在hot tail做压缩（对KV Cache影响较小）

### 机械压缩

Call-Result 压缩：
目的： 减少ToolResult在 Message List中的长度或则删除 call-result pair

- 基于Agent Tool Result协议，可以实现Tool Result的分级压缩
- 基于Agent Tool Result协议，可以及时丢弃不必要的call-result pair


Step Record History 压缩：

- 将多个 `[agent:intent user:last_step_result]` 合并成一个（还是两个？）step-record-history message
- 在 step-record-history 中，可以对 action-result 进行降级，直到完全丢弃
- 通过保留每个 step 的"观察 — 思考"等关键信息，让旧 step 在不喂 action_result 的情况下仍可读

通过分级机制，可以**有机会完全丢弃 ActionResults，只保留思考链路骨架**。

> 分级机制的精确定义 参考Agent Tool Result Protocol 以及 LLM Compress

### 在Input触发推理前，通过(LLM)压缩，释放Context Window

目的：将一组中间的 Message Pair,压缩成一个Message Pair
[system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1 .... [user [a-call u-result] [a-call u-result]  assistant]@round22 [user assistant]@round23  **user**
压缩后再开始推理
[system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1 [user:压缩需求1 agent:压缩结果1] [user:压缩需求2 agent:压缩结果2] [user assistant]@round3 **user**

这种机制基本上是模式无关的，只要边界切对就好

## 状态机切换（仅限 Behavior Loop)

以session的共同状态为基础，sessionn在完成不同任务（关键是系统提示词不同）的多个LLM Context中切换。让每个LLM Context在运行时，能有独立的 Message List。

> do->check->do->check->end

### 普通切换（注意：这是反模式）


[do-behavior-system] ([user:step0-step3-history + beahvior-on-switch]) [agent:intent user:last_step_result]@step4 [[a-call u-result] agent:intent user:last_step_result]@step5 agent:next_behavior=CHECK

执行切换后,填入on_behavior_switch构造的user_message

[check-behavior-system] ([user:step0-step3-history + beahvior-on-switch]) [agent:intent user:last_step_result]@step4 [[a-call u-result] agent:intent user:last_step_result]@step5 agent:next_behavior=CHECK **check-beahvior-on-switch** --继续推理-->

相当于agent的next_beahvior回触发一个特殊的on-switch调用，构造一个user message插入并推动进入下一个step. 这个由[agent:behavior-switch + user:on-switch]的Message Pair是一个特殊的的状态机切换Message Pair

> 这是从状态机切换可用路径中推导出来的理论模式，但实际上不可用

### 独立切换 

[do-behavior-system] **do-beahvior-on-switch** [agent:intent user:last_step_result]@step1 [[a-call u-result] agent:intent user:last_step_result]@step2 agent:next_behavior=CHECK

执行切换后，Resume target behavior的llm_context,并填入on_behavior_switch构造的user_message:

[check-behavior-system] **check-beahvior-on-switch**  （从头开始）

check模式 LLM推理,切换回DO:

[check-behavior-system] check-beahvior-on-switch [agent:intent user:last_step_result]@step1 [agent:intent user:last_step_result]@step2 agent:next_behavior=DO

执行切换，DO从上一个点回复,并继续推理到CHECK

[do-behavior-system] do-beahvior-on-switch [agent:intent user:last_step_result]@step1 [[a-call u-result] agent:intent user:last_step_result]@step2 [agent:next_behavior=CHECK **do-beahvior-on-switch**]  [agent:intent user:last_step_result]@step3 agent:next_behavior=CHECK

执行切换，CHECK恢复独立的LLM Context，并继续执行到END

[check-behavior-system] check-beahvior-on-switch  [agent:intent user:last_step_result]@step1 [agent:intent user:last_step_result]@step2 [agent:next_behavior=DO **check-beahvior-on-switch**] [agent:intent user:last_step_result]@step3 [agent:intent user:last_step_result]@step4 agent:next_behavior=END

> 该模式其实就是LangChain 的状态机实现


### Fork模式切换

下文介绍

## Fork 创建旁路LLM

旁路LLM可以使用不同的系统提示词+继承历史记录的方式专注于一个特定的任务，任务完成后，只把结果join回主干，最终保障了主干的Context Windows的大小

### 在标准AgentLoop中fork是一次常规的tool call触发

fork前
[system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1 [user [a-call u-result] a-call **决定fork**

思路一 重新渲染：把parent context的history，渲染进入tool的user message （目前这种用的比较多，兼容性强）

[tool-system] **[history from parent context] + tool user** --推理--> assistant message(u-result)

如果旁路压根不渲染任何parent context的记录，这是context window负担最轻的方法，缺点是要求上层对参数的使用要非常精确：

[tool-system] **tool-params user** --推理--> assistant message(u-result)

join后主干：[system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1 [user [a-call u-result] a-call u-result
看起来就是一次标准的tool call完成,完全丢弃了tool内部推理累计的消息。

思路二 直接继承：插入两条消息继续推理 （注意：这是一条反模式）

[system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1  [user [a-call u-result] **插入固定assistant message**] **插入固定user message** --推理--> assistant message(u-result)
join后主干 [system] [user [a-call u-result] [a-call u-result] [a-call u-result] assistant]@round1 [user [a-call u-result] a-call u-result
看起来也是一次标准的tool call完成

> 插入 **插入固定assistant message** 这一条通常会带来语义漂移，因此是反模式


### 在Behavior Loop中,fork是一次特殊的beahvior状态切换

plan
 +->DO->END
 |
 last_do_report
 |
 +->DO->END
 |
 last_do_report
 |
 END

fork前
[PLAN-behavior-system] **plan-beahvior-on-switch** [agent:intent user:last_step_result]@step1 [[a-call u-result] agent:intent user:last_step_result]@step2 agent:next_behavior=DO

fork 后的 sub-ctx 入口是**一条** user message，内部分两段（XML section 或类似形式）：一段是从父继承的 StepRecord history，一段是 on_behavior_switch payload：

[DO-behavior-system] [user: \<inherited_steps\>...\</inherited_steps\> \<on_behavior_switch\>DO-beahvior-on-switch\</on_behavior_switch\>] --多step推理--> agent:next_behavior=END (join)

> sub 看到的是**结构化的 StepRecord history**（thought / observation / next_behavior / self_report 等字段），不是父的 message 序列原样回放。这是 fork 跟"原样继承消息"路线的根本区别。

**继承粒度是由 fork 模板决定，不是 0/1**：

| 粒度 | 内容 | 适用 |
|---|---|---|
| 全套 StepRecord | thought + observation + actions + action_results + next_behavior + self_report | sub 需要看父的完整决策过程 |
| 只到上次 behavior 边界 | 只当前 behavior 段内的 step | sub 不关心更早阶段 |
| 只 thought + next_behavior | 决策链路骨架，扔掉 tool 噪声 | sub 只要"父为什么走到这一步" |
| 只 self_report | 每段 behavior 的最终交接点 | sub 只要"父做完了什么" |

最薄的"只 self_report"等价于下面"不继承历史"——self_report 本来就是父 behavior 的交接 statement。

如果选择不继承历史记录 fork（所有需要的共享状态都打包进 DO-behavior-system 或 on_behavior_switch payload）:
[DO-behavior-system] [user: \<on_behavior_switch\>DO-beahvior-on-switch\</on_behavior_switch\>] --多step推理--> agent:next_behavior=END (join)

对 sub-behavior 来说，这是 context window 负担最轻的方法，缺点就是要求上层共享状态的使用要非常精确。

join 后的主干：
[PLAN-behavior-system] plan-beahvior-on-switch [agent:intent user:last_step_result]@step1 [[a-call u-result] agent:intent user:last_step_result]@step2 agent:next_behavior=DO
**user:PLAN-beahvior-on-switch(可以包含do-behavior的last report)** , 继续前进

> sub 跑出来的 step 只在内存里，join 时**整段丢弃**，只有 last_report 通过 on_behavior_switch payload 流回父。

#### 跟"独立切换首次进入"的形状对照

注意 fork 后的 sub-ctx 形状跟独立切换**首次**进入某 behavior 时几乎一样：

|  | fork sub-ctx 入口 | 独立切换首次进入 |
|---|---|---|
| 形状 | `[DO-system] [user: 继承payload + on_behavior_switch]` | `[DO-system] [user: on_behavior_switch]` |
| sub 的 step 流向 | 内存，join 时丢弃 | 自己的 `.snap` 落盘 |
| 再次进入 | 每次都是全新 sub | resume 自己之前的 stream |

**结构相近，所有权不同**。读者注意区分。

#### Fork 的隔离边界

fork 的 invariant 只在 **message list 一层**：

- ✅ Message list 隔离：sub 的 step 不进父的 history
- ❌ 文件系统操作：不隔离
- ❌ worklog 事件：不隔离
- ❌ messages_sent（对外发消息）：不隔离
- ❌ session 全局状态：不隔离

也就是说 fork **不是沙箱**。如果想用 sub 做 dry-run，必须自己在 sub-system / 工具集层面构造隔离，waist 不提供。

#### Fork 嵌套

sub-ctx 自己也能 fork 出 sub-sub-ctx，形成调用栈，每层独立 join。为了防止跑飞，目前限制的栈深通常很浅（1-2 层），语义上不限。


