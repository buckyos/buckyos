# OpenDAN 通用 `llm_message_compress` 技术需求文档

- 文档版本：v1.0 Draft
- 整理日期：2026-05-22
- 适用范围：OpenDAN Agent Runtime、LLM Session、Work Session、Behavior Loop、UI Session
- 模块名称：`llm_message_compress`
- 文档目标：将基于语音记录形成的设计思路整理为可进入实现阶段的技术需求文档。

---

## 1. 背景

在 OpenDAN 的 Agent Loop 中，一个 LLM Session 会持续积累历史消息。随着消息数量增加，Session 上下文会逐渐接近模型的 context window 上限。若不进行处理，会带来以下问题：

1. 当前轮推理可能因为上下文过长而失败。
2. Prompt token 成本持续上升。
3. 旧消息对当前任务的噪声越来越大，降低推理质量。
4. 频繁改动历史消息又会破坏 prompt cache 的稳定性。
5. Tool call / tool result 产生的大量结构化消息会快速挤占上下文。

因此，需要设计一个通用的 `llm_message_compress` 机制，用于在合适时机把一段历史消息压缩成更短、更稳定、可追踪的摘要块。

该模块的目标不是为某一个具体 Agent 写一套专用摘要逻辑，而是提供一个可配置、可复用、可替换 prompt 的通用消息历史压缩框架。不同 Agent 可以通过参数和附加 prompt 调整压缩关注点，但核心消息选择、压缩边界、缓存稳定性和元数据结构应保持一致。

---

## 2. 问题定义

`llm_message_compress` 要解决的问题可以拆成两个层次：

### 2.1 给定一组消息，如何选择并压缩中间历史

这是本模块的核心问题。输入是一组 LLM Session messages，输出是一组新的 messages。新消息应保留：

1. System Message。
2. 会话头部若干关键消息，即 Head Keep。
3. 一个或多个已生成的压缩摘要块，即 Compressed Pair。
4. 最近若干轮完整消息，即 Hot Tail。

中间被替换掉的历史消息则进入 Compress Block，由 LLM 或机械规则压缩。

### 2.2 给定一组消息，能否机械性减少部分低价值内容

这是与核心语义压缩相关但不完全相同的问题。例如：

1. 旧的 tool result 是否可以折叠。
2. 大型文件读取结果是否可以只保留路径、hash、范围、长度等元信息。
3. 很久以前的写操作结果是否可以只保留“已成功写入某文件”的操作摘要。
4. 重复日志、成功命令输出、无错误的中间结果是否可以删除或压缩。

该类处理属于机械压缩，不调用 LLM，具有可预测、低成本、低语义理解能力的特点。它可以作为 `llm_message_compress` 的一部分，但不应与 LLM 语义压缩混为一个步骤。

---

## 3. 设计目标

`llm_message_compress` 的设计目标如下：

1. **释放上下文空间**：当当前 messages 接近 context window 阈值时，压缩中间历史，给后续推理留下足够空间。
2. **保留推理连续性**：至少保留最近 2 个完整 Message Pair，避免当前任务状态被破坏。
3. **保留头部关键意图**：System Message 和早期关键消息通常包含原始任务、约束和初始目标，应支持 Head Keep。
4. **保持消息边界完整**：尽量以 Message Pair 为单位选择 Compress Block，避免随意截断 tool call 或 assistant response。
5. **保持 prompt cache 稳定性**：压缩不应频繁发生；一次压缩应释放足够空间，保证后续若干轮不会立刻再次压缩。
6. **优先低成本压缩**：如果机械压缩可以达到目标，就不调用 LLM。
7. **避免重复有损压缩**：已生成的 Compressed Pair 应成为稳定边界，不应在后续压缩中再次被压缩。
8. **支持通用 prompt 框架**：压缩 LLM 的 system prompt 应由固定基础提示词和调用方附加关注点组成。
9. **支持但不强制 Memory 提取**：压缩流程可以输出 `memory_candidates`，但第一版不自动调用 `set_memory`。
10. **可观测、可调试、可回溯**：每次压缩都应记录压缩范围、策略、token 变化、summary hash 和 warnings。

---

## 4. 非目标

第一版明确不做以下事情：

1. 不实现通用长期记忆系统。
2. 不在压缩过程中自动写入 Memory。
3. 不做多轮 LLM tool call 压缩流程。
4. 不追求精确输出 token 数或精确字数。
5. 不在每一次 function call / tool call 后触发压缩。
6. 不把已有 Compressed Pair 再次纳入新的 Compress Block。
7. 不在第一版实现复杂的压缩质量自动评估闭环。
8. 不替代后台 Memory Scan / Session Review 等专门整理记忆的流程。
9. 只使用与标准的llm loop,首个版本不在behavior loop上启用

---

## 5. 核心概念

### 5.1 LLM Message

LLM Message 是传给模型的基础消息单元。它通常包含：

- `role`：`system` / `user` / `assistant` / `tool`。
- `content`：文本、结构化内容或 tool result。
- `id`：唯一 ID。
- `token_count`：可选的 token 统计。
- `pair_id`：所属 Message Pair。
- `turn_index`：所在轮次。
- `is_compressed`：是否为压缩生成的消息。
- `compressed_from`：压缩来源范围。

### 5.2 Message Pair

Message Pair 指一轮用户输入及其对应的 assistant 完整响应。

在传统聊天中，一个 Message Pair 通常是：

```text
UserMessage
AssistantMessage
```

但在 Agent Runtime 中，一个 Message Pair 内部可能包含：

```text
UserMessage
AssistantMessage(tool_call)
ToolResultMessage
AssistantMessage(tool_call)
ToolResultMessage
AssistantMessage(final)
```

因此，压缩边界不应简单按单条 message 切割，而应尽量按 Message Pair 切割。这样可以避免把 tool call 和 tool result 分开，破坏语义完整性。

### 5.3 Active Pair

Active Pair 指当前尚未完成的一轮 Message Pair。例如 assistant 已经发起 tool call，但 tool result 或 final answer 尚未完成。

压缩不应发生在 Active Pair 内部。若当前 Session 正处于 tool loop 中，应等待本轮完成后再判断是否需要压缩。

### 5.4 Hot Tail

Hot Tail 指最近若干轮完整 Message Pair。它直接影响当前任务状态，默认不进入 Compress Block。

默认要求：

1. 至少保留最近 2 个完整 Message Pair。
2. `hot_tail_pairs` 应可配置。
3. 如果 context window 允许，可以保留更多 Hot Tail。
4. Active Pair 即使不满足完整 pair 条件，也必须被保护，不能压缩。

建议默认值：

```text
hot_tail_pairs = 2
```

### 5.5 Head Keep

Head Keep 指会话开头的关键消息。很多 Session 中，最早的几条消息包含：

1. 原始任务目标。
2. 用户的长期约束。
3. 初始文件、项目、产品背景。
4. Agent 角色或工作方式的补充说明。

从注意力衰减和任务一致性的角度看，头部信息不能简单丢弃。因此，压缩结构应支持“取头取尾”。

建议默认策略：

1. 永远保留 System Message。
2. 可选保留最早 1 到 2 个 Message Pair。
3. 是否启用 Head Keep、保留几轮，由调用方配置。

建议默认值：

```text
head_keep_pairs = 1
```

### 5.6 Compress Block

Compress Block 指本次计划压缩的连续消息区间。它通常位于 Head Keep 和 Hot Tail 之间。

```text
SystemMessage
Head Keep
[ Compress Block ]
Hot Tail
```

压缩后，Compress Block 会被一个 Compressed Pair 替换。

### 5.7 Compressed Pair

Compressed Pair 是压缩后的摘要消息块。推荐使用一个可识别的消息对，而不是只塞入一条普通消息。

推荐结构：

```text
AssistantMessage(is_compressed=true):
  描述这里是一次历史消息压缩结果，包含压缩意图、压缩时间、原始范围等元信息。

UserMessage(is_compressed=true):
  放置压缩后的 summary 内容。
```

这样做的好处是：

1. 在消息流中保留“这里发生了一次压缩”的意图。
2. Summary 作为一个明确的历史上下文输入，便于后续模型理解。
3. 可通过 `is_compressed` 和 `compressed_from` 识别稳定边界。
4. 调试界面可以清晰展示压缩范围和摘要内容。

实现上也可以使用单条特殊 UserMessage 承载压缩结果，但必须保留足够元数据。

### 5.8 Stable Compressed Boundary

Compressed Pair 是一次有损压缩的结果。后续压缩不应再次把它纳入 Compress Block，否则会出现摘要反复摘要，导致信息逐轮衰减。

因此，已生成的 Compressed Pair 应成为 Stable Compressed Boundary。

下一次压缩应从上一个 Compressed Pair 之后的消息开始选择新的 Compress Block。

示例：

```text
SystemMessage
Head Keep
Compressed Pair #1
Messages after #1
Compressed Pair #2
Hot Tail
```

其中 `Compressed Pair #1` 不会被 `Compressed Pair #2` 再次压缩。

### 5.9 Mechanical Compression

Mechanical Compression 指不调用 LLM 的确定性压缩。它适合处理结构化、可预测、低语义价值的消息，例如旧 tool result、重复日志、成功命令输出、大文件读取结果等。

机械压缩的特点：

1. 成本低。
2. 可预测。
3. 可在执行前估算节省 token。
4. 不适合处理复杂语义总结。

### 5.10 LLM Compression

LLM Compression 指调用专门的摘要模型，对 Compress Block 做语义压缩。

它适合处理：

1. 多轮讨论形成的设计结论。
2. 决策过程。
3. 未完成任务。
4. 跨消息的上下文关系。
5. 非结构化自然语言历史。

---

## 6. 压缩后的消息结构

推荐的压缩后 Session 结构如下：

```text
SystemMessage
Head Keep Messages
Compressed Pair
Hot Tail Messages
```

多次压缩后的结构如下：

```text
SystemMessage
Head Keep Messages
Compressed Pair #1
Messages after #1
Compressed Pair #2
Hot Tail Messages
```

关键要求：

1. System Message 永远不进入 Compress Block。
2. Head Keep 默认不进入 Compress Block。
3. Hot Tail 默认不进入 Compress Block。
4. 已存在的 Compressed Pair 不进入 Compress Block。
5. 新 Compressed Pair 替换原 Compress Block 后，必须携带元数据。
6. 消息顺序必须稳定，不因压缩导致 Head Keep / Hot Tail 位置变化。

---

## 7. 压缩触发策略

### 7.1 不应在单次函数调用中触发压缩

压缩不应在每一次 function call / tool call 后触发。原因是：

1. Tool call 和 tool result 通常属于同一个 Message Pair 的内部结构。
2. 在 tool loop 中间压缩会破坏 pair 边界。
3. 频繁改写 messages 会破坏 prompt cache 命中。
4. 压缩本身有成本，不应变成每轮默认操作。

因此，推荐在以下时机触发判断：

1. 一轮完整 Message Pair 结束后。
2. Session Manager 发现 context window 使用率达到阈值后。
3. Behavior Loop 或调用方显式请求压缩。
4. 预计下一轮输入会使上下文接近或超过上限时。

### 7.2 基于 context window 比例触发

推荐默认触发条件：

```text
current_tokens / context_window_tokens >= trigger_ratio
```

建议默认值：

```text
trigger_ratio = 0.80
```

可接受范围：

```text
0.75 <= trigger_ratio <= 0.85
```

压缩后目标比例建议：

```text
target_ratio = 0.50
```

可接受范围：

```text
0.45 <= target_ratio <= 0.55
```

设计意图是：触发一次压缩后，应释放足够空间，保证后续若干轮不会立刻再次触发压缩。

#### 7.2.1 压缩生效线与最低释放量

为了保护 KV Cache / Prompt Cache 的稳定性，任何一次压缩一旦改写历史，都必须释放足够大的 context window 空间，避免压缩后很快再次触发压缩。

默认策略下：

```text
trigger_ratio = 0.80
target_ratio = 0.50
```

因此压缩后的总 token 数必须满足：

```text
new_total_tokens <= ceil(context_window_tokens * 0.50)
```

换句话说，本次压缩必须至少释放：

```text
required_saved_tokens = current_total_tokens - ceil(context_window_tokens * target_ratio)
```

在刚好达到普通触发线时：

```text
current_total_tokens ~= context_window_tokens * 0.80
required_saved_tokens ~= context_window_tokens * 0.30
```

示例：

| context window | 普通触发线 80% | 目标线 50% | 至少释放 |
| --- | ---: | ---: | ---: |
| 16k | 12.8k | 8k | 4.8k |
| 32k | 25.6k | 16k | 9.6k |
| 64k | 51.2k | 32k | 19.2k |
| 128k | 102.4k | 64k | 38.4k |

如果已经达到硬上限附近：

```text
current_total_tokens ~= context_window_tokens * 0.95
required_saved_tokens ~= context_window_tokens * 0.45
```

该规则同时约束机械压缩和 LLM 压缩。机械压缩只有在预计结果满足 `new_total_tokens <= target_token_budget` 时才允许生效；否则必须放弃本次机械压缩结果，直接进入 LLM 压缩路径。

### 7.3 最小压缩间隔

为保护 prompt cache 稳定性，建议配置：

```text
min_turns_between_compress = 2 或更高
```

含义：距离上一次压缩之后，如果完成的 Message Pair 数太少，即使 token 比例略高，也可以延后压缩，除非已经接近硬上限。

### 7.4 硬上限保护

当上下文接近模型硬上限时，应允许忽略 `min_turns_between_compress`，强制触发压缩或返回错误。

建议参数：

```text
hard_limit_ratio = 0.95
```

当超过该比例时：

1. 优先执行压缩。
2. 如果没有可压缩块，返回 `no_compressible_message_range`。
3. 调用方应停止继续追加上下文，或切换更大 context window 的模型。

---

## 8. Cache 稳定性原则

Prompt cache 的稳定性是该模块的重要约束。压缩逻辑应遵守以下原则：

1. 不频繁触发压缩。
2. 一次压缩应释放足够空间。
3. System Message 保持原样。
4. Head Keep 保持原样。
5. Hot Tail 保持原样。
6. Compress Block 的选择应尽量稳定，不要因微小 token 波动反复改变边界。
7. 机械压缩虽然低成本，但也会改变 prompt 内容，因此同样不应随意频繁执行。
8. 策略选择应避免“先机械压缩一次，马上又 LLM 压缩一次”的连续变动。

推荐策略：

```text
if 机械压缩能达到 target_ratio:
    本次只做机械压缩
else:
    放弃本次机械压缩结果，直接做一次 LLM 压缩
```

这样可以减少 messages 被连续改写的次数。

---

## 9. 压缩策略选择

### 9.1 总体流程

```text
1. 统计当前 messages token 数。
2. 判断是否达到 trigger_ratio。
3. 如果未达到，返回 changed=false。
4. 判断当前是否存在 Active Pair。
5. 如果存在 Active Pair，原则上延后到本轮结束。
6. 根据 System / Head Keep / Stable Boundary / Hot Tail 选择 Compress Block。
7. 如果没有有效 Compress Block，返回 changed=false，并给出 warning。
8. 估算机械压缩可释放 token。
9. 如果机械压缩可达到 target_ratio，则执行机械压缩。
10. 否则构造 LLM 压缩输入。
11. 调用压缩模型生成 summary。
12. 构造 Compressed Pair。
13. 替换原 Compress Block。
14. 返回 new_messages、压缩范围、token 变化和元数据。
```

### 9.2 策略选择伪代码

```ts
function compressMessages(req: CompressRequest): CompressResponse {
  const stats = countTokens(req.messages);

  if (!shouldTrigger(stats, req.trigger_policy)) {
    return noChange("below_trigger_ratio");
  }

  const pairs = buildMessagePairs(req.messages);

  if (hasActivePair(pairs) && !nearHardLimit(stats, req.trigger_policy)) {
    return noChange("active_pair_not_finished");
  }

  const range = selectCompressBlock(pairs, req.select_policy);

  if (!range) {
    return noChange("no_compressible_message_range");
  }

  const mechanicalPlan = estimateMechanicalCompression(range, req.mechanical_policy);

  if (mechanicalPlan.canReachTargetRatio) {
    return applyMechanicalCompression(req.messages, mechanicalPlan);
  }

  const llmInput = buildLlmCompressInput(range, req.llm_policy);
  const llmOutput = callCompressModel(llmInput);
  const compressedPair = buildCompressedPair(range, llmOutput);

  return replaceRangeWithCompressedPair(req.messages, range, compressedPair);
}
```

---

## 10. Compress Block 选择规则

### 10.1 基本规则

Compress Block 的选择应满足：

1. 不包含 System Message。
2. 不包含 Head Keep。
3. 不包含 Hot Tail。
4. 不包含 Active Pair。
5. 不包含已有 Compressed Pair。
6. 尽量以 Message Pair 为单位。
7. 必须是连续区间。
8. 应优先选择足够大的区间，避免压缩效果过低。

### 10.2 选择顺序

推荐选择流程：

```text
1. 识别 System Message。
2. 构造 Message Pair 列表。
3. 标记 Head Keep 范围。
4. 标记 Hot Tail 范围。
5. 找到最后一个 Stable Compressed Boundary。
6. 从该 boundary 之后到 Hot Tail 之前，选择可压缩区间。
7. 根据 max_llm_input_tokens 限制输入规模。
8. 若最后一个 pair 略微超过策略上限，可允许包含。
```

### 10.3 Head Keep 和 Stable Boundary 的关系

第一次压缩时：

```text
SystemMessage
Head Keep
Compress Block
Hot Tail
```

第二次压缩时：

```text
SystemMessage
Head Keep
Compressed Pair #1
Compress Block
Hot Tail
```

此时 `Compressed Pair #1` 和它之前的内容都视为稳定前缀，不再参与新的 Compress Block 选择。

### 10.4 Token 上限与“允许超过一个 Pair”

LLM 压缩输入应配置最大 token 上限：

```text
max_llm_input_tokens
```

但该上限是压缩策略上限，不是模型硬上限。它的作用是避免一次压缩输入过大，而不是严格阻止任何超过。

当存在以下情况时，应允许最后一个 Message Pair 使输入略微超过 `max_llm_input_tokens`：

1. 该 Pair 不进入压缩会导致释放空间明显不足。
2. 该 Pair 是一个大文件读取、大段文本分析或大型 tool result 的完整语义单元。
3. 压缩模型的真实 context window 仍然可以容纳该输入。

原则：

1. 优先保证压缩效果。
2. 不要为了严格卡 token，使 Compress Block 太小，导致压缩几乎没有收益。
3. 如果单个 Pair 已经超过压缩模型硬上限，应返回 warning，并交由调用方决定是否先做专门的大消息压缩。

### 10.5 最小收益阈值

应配置最小预期收益：

```text
min_estimated_saved_tokens
```

如果候选 Compress Block 过小，预计释放空间不足，则不应压缩，除非已经达到硬上限。

---

## 11. 机械压缩需求

机械压缩不破坏结构，根本上只有两种方法:
- 在AgentToolResult Protocol的帮助下，对ToolResult/ActionResult进行压缩，该压缩不会影响AiMessage的总数。通常是从旧消息开始往新消息压,越旧的消息压缩级别越高
- 将多个消息pair合成一个消息对[user:压缩需求 agent:机械压缩后得到的历史记录] ，典型的压缩结果有两种形态

**AgentLoop:**
```
History:
  user:  xxxx
    call(xxxx) => xxxx
    call(xxxx) => xxxx
  agent: xxxx
  user:  xxxx
    call(xxxx) => xxxx
    call(xxxx) => xxxx
  agent: xxxx
```


**Behavior Loop:**
```
History:
  Step1
  - 观察:xxx
  - 思考:xxx
  - 动作
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx
  - 报告
  Step2
  - 观察:xxx
  - 思考:xxx
  - 动作
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx  
```

注意机械压缩产生的History块是可以合并的，也就是说再触发一次机械压缩，有可能会让老的Histroy Block变长 

### 11.1 机械压缩适用对象

机械压缩适合以下内容：

1. 旧的 tool result。
2. 大型文件读取结果。
3. 重复日志。
4. 无错误的成功命令输出。
5. 写操作成功结果。
6. 已经被后续消息确认无须再次查看的中间结果。
7. 大型检索结果中的低相关项。

上述要求，都由各个Agent Tool的是实现者根据Agent Tool Protocl协议自行实现。机械压缩流程不做判断

### 11.2 机械压缩收益评估

机械压缩必须在执行前估算收益：

```text
estimated_saved_tokens = original_tokens - compressed_tokens
```

只有当机械压缩预计能使整体上下文回到 `target_ratio` 以下时，才应作为本次压缩策略直接执行。

如果不能达到目标，则本次不应先执行机械压缩再执行 LLM 压缩，而应直接进入 LLM 压缩路径。

默认配置下，机械压缩的目标线是 50% context window：

```text
target_token_budget = ceil(context_window_tokens * 0.50)
mechanical_can_apply = mechanical_new_total_tokens <= target_token_budget
```

所以在 80% 触发时，机械压缩通常必须至少释放约 30% context window 的 token。这个门槛是为了保证压缩后的历史能稳定运行几轮，而不是每轮都改写历史。

机械压缩内部按成本从低到高尝试：

1. **ToolResult 协议分级压缩**：对旧 ToolResult 解析 `AgentToolResult` envelope，并根据位置降级到 `Medium` 或 `Min`。
2. **Agent Loop History Block 折叠**：当协议分级仍不能达到 `target_token_budget` 时，尝试把多个完整 message pair 合并为一个机械 History 块。

如果第 1 步已经让整体 token 数回到目标线以下，本轮只采用 ToolResult 分级压缩，不再生成 History Block。

#### 11.2.1 History Block 生成条件

Agent Loop 的机械 History Block 只有在以下条件全部满足时才生成：

1. 当前消息不是 Behavior Loop 起源。若消息中存在 `<<step_history>>`、`<<last_step_action_results>>`，或 system/developer prompt 明确属于 behavior 形态，则不走 Agent Loop History Block。
2. 本次 Compress Block 已经按完整 Message Pair / Span 选好，且不包含 System、Head Keep、Hot Tail、Active Pair 或 Stable Compressed Boundary。
3. Compress Block 内的每个候选 span 都已经满足“可机械折叠”条件：
   - span 中存在已经降到 `Min` 级别的机械 ToolResult；或
   - span 本身已经是一个旧的机械 History Block。
4. ToolResult 协议分级压缩后，整体 token 数仍然高于 `target_token_budget`。
5. 折叠为 History Block 后，整体 token 数必须小于等于 `target_token_budget`。

满足条件后，输出形态为一对消息：

```text
user: Historical message pairs <start>..<end> were mechanically folded ...
assistant:
  [LLM_MECHANICAL_COMPRESS_META_V1]
  {... "message_pairs_in_history_block": N, "rule_name": "agent_loop_history_block_v1", ...}
  History:
    user: ...
      call(...) ...
    agent: ...
```

再次触发机械压缩时，旧 History Block 可以被识别并与更老/相邻的可折叠 span 合并，形成一个更长的 History Block；不应产生多个并列的机械 History Block。

### 11.3 机械压缩结果元数据

机械压缩后的消息应标记：

```ts
interface MechanicalCompressedMeta {
  is_mechanically_compressed: true;
  message_pairs_in_history_block:number;//为0说明没创建history blcok
  original_token_count?: number;
  compressed_token_count?: number;
  rule_name: string;
  compressed_at: number;
}
```

---

## 12. LLM 压缩需求

### 12.1 LLM 压缩输入

LLM 压缩输入应由以下部分组成：

1. 压缩专用 System Prompt。
2. 调用方附加的任务关注点 Prompt。
3. 被压缩的原始消息历史，编码为一个完整 user message。
4. 必要的消息元信息，例如 role、message id、pair id、tool name、时间等。

不建议默认把原始 Session 的 System Prompt 全量放入压缩模型输入中。原因是：

1. 原始 System Prompt 可能很长。
2. 它可能对摘要没有帮助，反而干扰压缩任务。
3. 压缩模型应使用独立、稳定、专用的 system prompt。

但应提供配置：当调用方认为原始 System Prompt 对摘要判断非常关键时，可以选择注入。

建议参数：

```ts
include_original_system_prompt?: boolean; // 默认 false
```

### 12.2 压缩模型选择

压缩过程通常不需要强工具调用能力，也不需要复杂多轮交互。它更接近一次性摘要任务。

建议选择：

1. 成本较低的摘要模型。
2. context window 足够容纳 Compress Block 的模型。
3. 输出稳定、遵循 JSON 或结构化格式能力较好的模型。

### 12.3 LLM 压缩输出

推荐输出 JSON：

```json
{
  "summary": "压缩后的历史摘要",
  "decisions": ["已经形成的关键决策"],
  "pending_actions": ["后续仍需处理的事项"],
  "open_questions": ["仍未解决或不确定的问题"],
  "important_entities": ["关键文件、模块、人物、概念、约束"],
  "memory_candidates": [
    {
      "key_hint": "可选的 memory key 建议",
      "content": "可能值得进入长期记忆的信息",
      "reason": "为什么可能值得保留",
      "confidence": "low | medium | high"
    }
  ]
}
```

第一版可以只强依赖 `summary` 字段，其他字段作为可选增强。

### 12.4 不做精确字数控制

不建议要求模型输出“最多 N 个字”或“精确压缩到 N token”。

原因：

1. 模型可能把注意力浪费在估算字数上。
2. 精确 token 控制并不可靠。
3. 摘要任务更重要的是保留关键信息，而不是机械满足字数。

推荐提示方式：

```text
请生成相对简洁但信息完整的摘要。不要为了变短而丢弃关键任务目标、约束、决策、未完成事项和重要实体。
```

### 12.5 防止过度压缩

LLM 摘要常见问题不是太长，而是过度压缩。为避免摘要过短导致信息损失，Prompt 中应强调：

1. 保留用户目标。
2. 保留关键约束。
3. 保留已做出的设计选择。
4. 保留未完成事项。
5. 保留影响后续执行的文件路径、接口名、模块名、错误信息。
6. 对不确定内容明确标记不确定。
7. 不要把仍可能影响后续任务的 tool result 完全省略。

---

## 13. 压缩 Prompt 设计

### 13.1 Prompt 结构

压缩 Prompt 分为两层：

1. 固定基础提示词。
2. 调用方附加关注点。

固定基础提示词定义通用摘要原则。调用方附加关注点用于不同 Agent 或任务的差异化压缩。

### 13.2 固定基础提示词草案

```text
你是 OpenDAN Agent Runtime 的历史消息压缩器。

你的任务是把一段历史 messages 压缩成可供后续 LLM 推理继续使用的上下文摘要。

请遵守以下原则：
1. 保留用户的原始目标、关键约束、偏好和明确要求。
2. 保留已经形成的设计决策、技术结论和重要 tradeoff。
3. 保留未完成事项、待确认问题、下一步行动。
4. 保留关键实体，包括文件路径、模块名、接口名、函数名、错误信息、数据结构、配置项。
5. 保留重要 tool call 的结果，尤其是失败原因、错误栈、测试结论、写入结果和外部事实。
6. 丢弃寒暄、重复表达、无效中间过程和已经不影响后续推理的细节。
7. 不要编造历史中不存在的信息。
8. 如果某个结论不确定，请明确标记不确定。
9. 摘要应尽量简洁，但不要为了变短而丢失会影响后续任务的信息。
10. 输出必须能让后续模型在不读取原始 Compress Block 的情况下继续当前任务。
```

### 13.3 调用方附加关注点示例

#### Code Agent

```text
额外关注：文件路径、代码修改点、接口变化、失败测试、编译错误、尚未提交的 patch、用户明确禁止或要求的实现方式。
```

#### Product Agent

```text
额外关注：产品目标、用户价值、功能边界、版本范围、决策背景、尚未确认的需求。
```

#### Research Agent

```text
额外关注：信息来源、证据链、未验证假设、重要引用、结论可信度。
```

#### Memory Agent

```text
额外关注：长期稳定偏好、用户身份相关事实、可复用知识、跨 Session 仍有价值的约束。
```

### 13.4 输入消息编码格式

建议把 Compress Block 编码成一个大 user message，例如：

```text
以下是需要压缩的历史 messages。请只总结这段历史，不要总结未出现的信息。

<messages>
[message_id=m_001 role=user pair_id=p_001]
...

[message_id=m_002 role=assistant pair_id=p_001]
...

[message_id=m_003 role=tool pair_id=p_001 tool_name=read_file]
...
</messages>
```

这样可以避免压缩模型把输入当成真实对话继续执行，而是明确作为待摘要材料处理。

---

## 14. Memory 关系与取舍

### 14.1 压缩时是否调用 `set_memory`

压缩点确实是一个适合发现 Memory 的时机，因为原始历史即将离开 active context。但自动在压缩过程中调用 `set_memory` 存在风险：

1. 会把一次摘要任务变成多轮 tool call 任务。
2. 可能破坏压缩流程的一次性和稳定性。
3. 模型可能过度提取 memory，产生太多低价值记忆。
4. 什么值得进入 Memory 与 Agent identity、用户身份、任务类型强相关。
5. Memory 写入需要去重、过期、冲突处理、可信度判断，而这些不是压缩模块的核心职责。

### 14.2 第一版决策

第一版不自动调用 `set_memory`。

但可以允许 LLM 压缩输出：

```ts
memory_candidates?: MemoryCandidate[]
```

这些候选只作为后续 Memory 流程的输入，不直接写入长期记忆。

### 14.3 如果未来支持 Memory 写入

如果未来要在压缩过程中触发 Memory，应优先采用批量结构化返回，而不是让模型一条一条调用工具。

示例：

```json
{
  "memory_candidates": [
    {
      "key_hint": "user.preference.response_style",
      "content": "用户偏好技术文档直接形成可执行需求，而不是泛泛总结。",
      "reason": "多次任务中体现出的稳定偏好",
      "confidence": "medium"
    }
  ]
}
```

然后由独立 Memory Manager 决定是否写入。

### 14.4 推荐方案：后台 Memory Scan

相比压缩时直接写入 Memory，更推荐独立的后台 Memory Scan：

1. 定时扫描尚未处理的历史记录。
2. 专门以 Memory 提取为目标运行。
3. 使用更严格的 memory prompt。
4. 可控地限制每日 token 成本。
5. 更容易做去重、合并、淘汰、过期判断。
6. 更容易根据 Agent identity 和用户上下文调整提取策略。

结论：`llm_message_compress` 可以给 Memory 系统提供候选材料，但不应成为 Memory 系统本身。

---

## 15. 接口需求

### 15.1 核心接口

```ts
interface CompressRequest {
  session_id: string;
  messages: LlmMessage[];

  context_window_tokens: number;
  current_tokens?: number;

  trigger_policy?: CompressTriggerPolicy;
  select_policy?: CompressSelectPolicy;
  mechanical_policy?: MechanicalCompressPolicy;
  llm_policy?: LlmCompressPolicy;

  agent_identity?: string;
  task_hint?: string;

  dry_run?: boolean;
}
```

```ts
interface CompressResponse {
  changed: boolean;
  strategy: "none" | "mechanical" | "llm";

  original_token_count: number;
  new_token_count: number;
  estimated_saved_tokens: number;

  new_messages: LlmMessage[];

  compressed_range?: {
    start_message_id: string;
    end_message_id: string;
    start_pair_id?: string;
    end_pair_id?: string;
  };

  compressed_pair_id?: string;
  summary?: string;
  memory_candidates?: MemoryCandidate[];

  plan?: CompressPlan;
  warnings?: CompressWarning[];
  errors?: CompressError[];
}
```

### 15.2 触发策略配置

```ts
interface CompressTriggerPolicy {
  enabled: boolean;

  trigger_ratio: number;       // 默认 0.80
  target_ratio: number;        // 默认 0.50
  hard_limit_ratio?: number;   // 默认 0.95

  min_turns_between_compress?: number;
  preserve_cache_stability: boolean;
}
```

### 15.3 消息选择策略配置

```ts
interface CompressSelectPolicy {
  hot_tail_pairs: number;       // 默认至少 2
  head_keep_pairs?: number;     // 默认 1

  max_llm_input_tokens: number;
  min_estimated_saved_tokens?: number;

  prefer_pair_boundary: boolean;
  allow_exceed_by_one_pair: boolean;

  skip_existing_compressed_pair: boolean;
  protect_active_pair: boolean;
}
```

### 15.4 LLM 压缩策略配置

```ts
interface LlmCompressPolicy {
  model?: string;

  system_prompt: string;
  extra_focus_prompt?: string;

  output_format: "text" | "json";
  include_message_metadata: boolean;
  include_original_system_prompt?: boolean;

  allow_memory_candidates: boolean;
  prompt_version?: string;
}
```

### 15.5 机械压缩策略配置

```ts
interface MechanicalCompressPolicy {
  enabled: boolean;

  compress_tool_results: boolean;
  compress_large_read_results: boolean;
  compress_success_outputs: boolean;
  compress_old_write_results: boolean;
  compress_repeated_logs: boolean;

  min_estimated_saved_tokens: number;
}
```

### 15.6 LlmMessage 元数据结构

```ts
interface LlmMessage {
  id: string;
  role: "system" | "user" | "assistant" | "tool";
  content: unknown;

  created_at?: number;

  pair_id?: string;
  turn_index?: number;

  token_count?: number;

  tool_call_id?: string;
  tool_name?: string;

  is_compressed?: boolean;
  compressed_kind?: "llm_summary" | "mechanical";

  compressed_from?: {
    start_message_id: string;
    end_message_id: string;
    start_pair_id?: string;
    end_pair_id?: string;
    original_token_count?: number;
    compressed_token_count?: number;
    strategy: "mechanical" | "llm";
    created_at: number;
    prompt_version?: string;
    model?: string;
    summary_hash?: string;
  };
}
```

### 15.7 MemoryCandidate 结构

```ts
interface MemoryCandidate {
  key_hint?: string;
  content: string;
  reason?: string;
  confidence: "low" | "medium" | "high";
  source_message_ids?: string[];
}
```

### 15.8 CompressPlan 结构

```ts
interface CompressPlan {
  should_compress: boolean;
  reason: string;

  selected_strategy: "none" | "mechanical" | "llm";

  head_keep_range?: MessageRange;
  hot_tail_range?: MessageRange;
  compress_range?: MessageRange;

  estimated_original_tokens: number;
  estimated_new_tokens: number;
  estimated_saved_tokens: number;

  mechanical_estimate?: {
    enabled: boolean;
    can_reach_target_ratio: boolean;
    estimated_saved_tokens: number;
    rules: string[];
  };

  warnings?: CompressWarning[];
}
```

---

## 16. Dry Run

应支持：

```ts
dry_run: true
```

Dry Run 不实际调用 LLM，也不改写 messages，只返回压缩计划。

Dry Run 应返回：

1. 是否需要压缩。
2. 为什么需要或不需要压缩。
3. 计划保留的 Head Keep。
4. 计划保留的 Hot Tail。
5. 计划压缩的 Compress Block。
6. 预计释放 token。
7. 计划使用机械压缩还是 LLM 压缩。
8. 是否会跳过已有 Compressed Pair。
9. 风险提示。

Dry Run 对调试 UI 和压缩策略调参非常重要。

---

## 17. 错误处理

### 17.1 无可压缩内容

当 Head Keep、Stable Boundary 和 Hot Tail 已占据大部分上下文时，可能没有有效 Compress Block。

返回：

```json
{
  "changed": false,
  "strategy": "none",
  "warnings": ["no_compressible_message_range"]
}
```

### 17.2 当前存在 Active Pair

如果当前 tool loop 未完成，应返回：

```json
{
  "changed": false,
  "strategy": "none",
  "warnings": ["active_pair_not_finished"]
}
```

除非已经接近硬上限。

### 17.3 LLM 压缩失败

如果 LLM 调用失败：

1. 不得破坏原 messages。
2. 返回原始 messages。
3. 标记错误原因。
4. 调用方可以选择稍后重试、降级到机械压缩、或切换更大 context window。

返回示例：

```json
{
  "changed": false,
  "strategy": "llm",
  "errors": ["llm_compress_failed"]
}
```

### 17.4 JSON 解析失败

如果要求 JSON 输出但解析失败：

1. 可尝试从输出中提取纯文本 summary。
2. 若能提取，则继续构造 Compressed Pair，并记录 warning。
3. 若无法提取，则压缩失败。
4. 不应写入 `memory_candidates`。

### 17.5 单条消息过大

如果单个 Message Pair 超过 `max_llm_input_tokens`，但仍可被压缩模型容纳，可以允许超过策略上限。

如果超过压缩模型硬上限，返回：

```json
{
  "changed": false,
  "warnings": ["single_pair_exceeds_compress_model_context"]
}
```

未来可引入针对大消息的专门 chunk 压缩策略。

---

## 18. 可观测性与调试

每次压缩都应记录以下信息：

1. `session_id`
2. 压缩触发时间
3. 使用策略：`none` / `mechanical` / `llm`
4. 原始 token 数
5. 压缩后 token 数
6. 预计节省 token
7. 实际节省 token
8. Head Keep 范围
9. Hot Tail 范围
10. Compress Block 范围
11. 是否跳过已有 Compressed Pair
12. 是否存在 Active Pair
13. 压缩 prompt version
14. 压缩模型名称
15. summary hash
16. warnings / errors
17. mechanical compression rules 命中情况

调试界面应能展示：

1. 压缩前 messages。
2. 压缩后 messages。
3. 被保留的 Head Keep。
4. 被保留的 Hot Tail。
5. 被压缩的 Compress Block。
6. 生成的 summary。
7. 为什么选择该范围。
8. 为什么使用机械压缩或 LLM 压缩。
9. 为什么没有触发压缩。
10. 已有 Compressed Pair 为什么被跳过。

---

## 19. 测试需求

### 19.1 单元测试

至少覆盖：

1. token ratio 未达到阈值时不压缩。
2. token ratio 达到阈值时选择 Compress Block。
3. 默认保留最近 2 个 Message Pair。
4. Head Keep 生效。
5. System Message 永不压缩。
6. Active Pair 不被压缩。
7. 已有 Compressed Pair 不被重复压缩。
8. Message Pair 边界不被随意切开。
9. 允许最后一个 Pair 超过策略 token 上限。
10. 无可压缩内容时返回 warning。
11. Dry Run 不改写 messages。
12. JSON 输出解析失败时降级处理。

### 19.2 机械压缩测试

至少覆盖：

1. 成功 tool result 折叠。
2. 大型 read_file result 折叠。
3. write_file 成功结果折叠。
4. 失败日志不被错误折叠。
5. 机械压缩收益不足时不执行机械压缩，直接进入 LLM 压缩计划。
6. 机械压缩收益足够时不调用 LLM。

### 19.3 LLM 压缩测试

至少覆盖：

1. 输出 summary 字段。
2. 保留关键决策。
3. 保留 pending actions。
4. 保留重要实体。
5. 不编造历史中不存在的信息。
6. 不因“简洁”丢弃关键任务约束。
7. 可选输出 `memory_candidates`。
8. 不自动写入 Memory。

### 19.4 集成测试

至少覆盖：

1. 长 Session 在 80% context window 左右触发压缩。
2. 压缩后上下文回落到目标比例附近。
3. 压缩后继续追加多轮消息，不会立刻再次压缩。
4. 多次压缩后，已有 Compressed Pair 不被再次压缩。
5. Tool call loop 中间不触发压缩。
6. 压缩后 Agent 能继续当前任务。

---

## 20. 验收标准

第一版实现完成时，应满足以下验收标准：

1. 提供 `compressMessages(request)` 或等价核心接口。
2. 支持基于 `trigger_ratio` 的触发判断。
3. 支持 `target_ratio` 和压缩后 token 估算。
4. 默认保留最近至少 2 个 Message Pair。
5. 支持 Head Keep 配置。
6. 支持识别并跳过已有 Compressed Pair。
7. 支持按 Message Pair 选择 Compress Block。
8. 支持 Dry Run。
9. 支持 LLM 压缩并生成 Compressed Pair。
10. 支持基础机械压缩框架。
11. 当机械压缩足够时，不调用 LLM。
12. 当机械压缩不足时，直接执行一次 LLM 压缩，不连续改写两次消息。
13. 压缩结果包含 `compressed_from` 元数据。
14. LLM 压缩 Prompt 可配置。
15. 可选输出 `memory_candidates`，但不自动调用 `set_memory`。
16. 压缩失败时不破坏原 messages。
17. 调试日志可说明“为什么压缩 / 为什么不压缩 / 为什么选这个范围”。

---

## 21. 第一版实现范围

第一版建议实现：

1. 基于 token ratio 的触发判断。
2. Head Keep + Hot Tail + Compress Block 的消息选择。
3. Active Pair 保护。
4. Message Pair 边界保护。
5. 避免重复压缩已有 Compressed Pair。
6. LLM 语义压缩。
7. 基础机械压缩框架。
8. 少量高价值机械压缩规则。
9. 压缩元数据写回。
10. Dry Run。
11. 可配置压缩 Prompt。
12. 可选输出 `memory_candidates`。

第一版暂不实现：

1. 自动 `set_memory`。
2. 多轮压缩 tool call。
3. 精确 token 输出控制。
4. 复杂跨 Session Memory 整理。
5. 压缩质量自动评估闭环。
6. 大型单消息 chunk 化压缩。
7. 根据不同 Agent identity 自动选择 memory 写入策略。

---

## 22. 后续演进方向

后续可以考虑：

1. 引入专门的后台 Memory Scan，与压缩流程解耦。
2. 对不同 Agent identity 提供不同压缩 focus preset。
3. 支持更细粒度的 tool result 机械压缩规则。
4. 支持压缩质量评估，例如通过 replay 或关键问题检查摘要是否保留必要信息。
5. 支持多级 summary，例如 session summary、topic summary、task summary。
6. 支持对压缩结果进行版本化和回溯调试。
7. 支持压缩策略 A/B Test。
8. 支持单个超大 Pair 的 chunk 压缩。
9. 支持把压缩结果写入 Agent Notebook 或 Session Notebook。
10. 支持跨 Session 的长期历史摘要索引。

---

## 23. 实现建议

### 23.1 分层实现

建议分为以下模块：

```text
llm_message_compress/
  index.ts
  token_counter.ts
  pair_builder.ts
  range_selector.ts
  mechanical_compressor.ts
  llm_compressor.ts
  compressed_pair_builder.ts
  dry_run.ts
  types.ts
  prompts/
    base_compress_prompt.ts
```

### 23.2 模块职责

#### `pair_builder.ts`

负责：

1. 根据 `pair_id` / `turn_index` / role 推断 Message Pair。
2. 识别 Active Pair。
3. 识别 tool call / tool result 所属关系。

#### `range_selector.ts`

负责：

1. 标记 System Message。
2. 标记 Head Keep。
3. 标记 Hot Tail。
4. 标记已有 Compressed Pair。
5. 选择 Compress Block。
6. 生成 CompressPlan。

#### `mechanical_compressor.ts`

负责：

1. 执行规则匹配。
2. 估算节省 token。
3. 在收益足够时改写 messages。

#### `llm_compressor.ts`

负责：

1. 构造压缩 prompt。
2. 调用压缩模型。
3. 解析输出。
4. 生成 summary 和 memory candidates。

#### `compressed_pair_builder.ts`

负责：

1. 构造 AssistantMessage marker。
2. 构造 UserMessage summary。
3. 写入 `compressed_from` 元数据。
4. 生成 summary hash。

### 23.3 推荐默认配置

```ts
const defaultCompressPolicy = {
  trigger_policy: {
    enabled: true,
    trigger_ratio: 0.80,
    target_ratio: 0.50,
    hard_limit_ratio: 0.95,
    min_turns_between_compress: 2,
    preserve_cache_stability: true,
  },
  select_policy: {
    hot_tail_pairs: 2,
    head_keep_pairs: 1,
    max_llm_input_tokens: 32000,
    min_estimated_saved_tokens: 4000,
    prefer_pair_boundary: true,
    allow_exceed_by_one_pair: true,
    skip_existing_compressed_pair: true,
    protect_active_pair: true,
  },
  mechanical_policy: {
    enabled: true,
    compress_tool_results: true,
    compress_large_read_results: true,
    compress_success_outputs: true,
    compress_old_write_results: true,
    compress_repeated_logs: true,
    min_estimated_saved_tokens: 4000,
  },
  llm_policy: {
    output_format: "json",
    include_message_metadata: true,
    include_original_system_prompt: false,
    allow_memory_candidates: false,
  },
};
```

---

## 24. 关键设计取舍总结

### 24.1 为什么要取头取尾

只保留尾部会丢失原始任务目标和关键约束。只保留头部又会破坏当前任务连续性。因此推荐：

```text
SystemMessage + Head Keep + Compressed Pair + Hot Tail
```

### 24.2 为什么不频繁压缩

每次压缩都会改变 prompt 内容，影响 prompt cache。即使是机械压缩，也会破坏缓存连续性。因此压缩应以“少触发、一次释放足够空间”为原则。

### 24.3 为什么机械压缩优先但不叠加执行

机械压缩低成本且可预测。如果它能达到目标，应优先使用。

但如果机械压缩不能达到目标，不应先机械压缩再 LLM 压缩，因为这会造成连续两次消息结构变动。推荐直接执行一次 LLM 压缩。

### 24.4 为什么已有压缩块不再压缩

摘要再次摘要会持续损耗信息。已有 Compressed Pair 应成为稳定边界，后续只压缩它之后的新历史。

### 24.5 为什么不自动写 Memory

Memory 写入需要更强的身份、长期价值、去重、过期和质量判断。压缩时可以产生 memory candidates，但第一版不应自动写入。

---

## 25. 一句话技术定义

`llm_message_compress` 是 OpenDAN Agent Runtime 中用于维护 LLM Session 长上下文稳定性的通用压缩模块。它通过 Head Keep、Hot Tail、Stable Compressed Boundary、机械压缩和 LLM 语义摘要，在尽量保护 prompt cache 和推理连续性的前提下，把中间历史替换为可追踪、可调试、可配置的压缩摘要块。

---

## 26. 核心原则清单

1. 不是简单把历史变短，而是重建稳定上下文边界。
2. System Message 不压缩。
3. 默认保留头部关键意图。
4. 默认保留尾部至少 2 个完整 Message Pair。
5. 不在 tool loop 中间压缩。
6. 不重复压缩已有 Compressed Pair。
7. 尽量按 Message Pair 选择边界。
8. 机械压缩能达到目标时优先机械压缩。
9. 机械压缩不能达到目标时直接 LLM 压缩。
10. 不做精确字数控制。
11. 防止摘要过度压缩。
12. Memory 候选可以输出，但不自动写入。
13. 每次压缩必须可观测、可调试、可回溯。
14. 模块保持通用，任务差异通过附加 prompt 和 policy 配置表达。


---
## 附录 几个压缩后的例子

### Agent Loop的传统消息流
[system] [user agent]  [user [a-call u-result] agent] * [user [a-call u-result] [a-call u-result] agent]  [user [a-call u-result] [a-call u-result] agent] [user agent]  * [user agent]  [user agent]

尝试机械压缩 （假设配置成保留head=2,tail=2)

机械压缩能做的事情 
[a-call u-result] => a-call, u-result的细节减少 ：
[user [a-call u-result] [a-call u-result] agent]  [user [a-call u-result] [a-call u-result] agent]  => 机械合并（细节压到最少）,
```
History:
  user:  xxxx
    call(xxxx) => xxxx
    call(xxxx) => xxxx
  agent: xxxx
  user:  xxxx
    call(xxxx) => xxxx
    call(xxxx) => xxxx
  agent: xxxx
```

LLM压缩: 压缩成消息对

[system] [user agent]  [user [a-call u-result] agent] * [u:压缩范围 a:压缩结果]  * [user agent]  [user agent]

压缩结果通常会保留一个hints列表（列出操作过的外部对象/线索）



### Behavior Loop的消息流

[behavior-system] [user:beahvior-on-switch] * [agent:intent user:last_step_result]@step1 [agent:intent user:last_step_result]@step2 [agent:intent user:last_step_result]@step3 [agent:intent user:last_step_result]@step4 [[a-call u-result] agent:intent user:last_step_result]@step5 agent:intent **user:last_step_result + pending inputs**

[a-call u-result] => 可以安全删除
[agent:intent user:last_step_result] => 减少user:last_step_result中的result细节
[agent:intent user:last_step_result]@step1 [agent:intent user:last_step_result]@step2 [agent:intent user:last_step_result]@step3  => 机械合并,得到[user:压缩范围 agent:history]
```
History:
  Step1
  - 观察:xxx
  - 思考:xxx
  - 动作
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx
  - 报告
  Step2
  - 观察:xxx
  - 思考:xxx
  - 动作
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx
    - do(xxxx) => xxxx  
```

LLM压缩: 压缩成消息对

[behavior-system] [user:beahvior-on-switch] * [agent:intent user:last_step_result]@step1 [user:压缩范围 agent:压缩结果] [[a-call u-result] agent:intent user:last_step_result]@step5 agent:intent 
