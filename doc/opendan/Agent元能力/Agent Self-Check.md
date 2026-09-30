# Self-Check Behavior 系统提示词编写指南

> 文件名建议：`Self-Check behaivor 系统提示词编写指南.md`  
> 本文档用于指导 Self-Check Behavior 的系统提示词编写。它描述 Self-Check 在后台如何读取 Notebook、如何理解用户意图、如何创建 / 取消 / 维护计划任务，以及如何控制执行成本和上下文窗口。

---

## 1. 文档目标

Self-Check Behavior 的系统提示词，目标不是让模型“执行某个具体任务”，而是让模型在后台周期性运行时，稳定地完成以下工作：

1. 消费 Agent Notebook 中的条目 (重点是新增条目)。
2. 结合用户环境、历史信息、已有计划任务和执行报告，判断哪些事项需要进入计划任务系统。
3. 创建、更新、取消或保持观察 计划任务 的执行情况。
4. 为每个 Notebook Item 写入可追踪的 Self-Check Review 状态。
5. 避免重复创建任务、过早创建任务、过度取消任务，以及无限膨胀上下文。

一句话概括：

> Self-Check 是一个基于 Notebook 唯一真相源的后台计划整理器。它负责把自然语言记录中的用户意图，稳定地转换为合适的计划任务。
> 从认知学上，就是反复的检查自己是否有一些“满足条件才能做的任务”，核心是分析模糊的开始条件变成确定的添加，转换成计划任务，而不是立刻执行计划任务 
>  也包括检查主人的计划任务，但更多是提醒“主人去做”

以定时汇报为例：Self-Check关注的是：定时汇报的触发时机，和基本的任务目标，而Self-Improve则关心定时汇报任务的目标设定和执行方法

---

## 2. 核心定位

Self-Check 模式的核心目标与 Plan 模式类似：它需要读取 Notebook 中与提醒、计划、待办、周期性检查、未来安排相关的内容，并判断是否需要进一步处理。

但 Self-Check 和普通执行 Agent 有本质区别：

| 维度 | Self-Check | Executor / WorkSession |
|---|---|---|
| 核心职责 | 发现、整理、维护计划任务 | 执行具体任务 |
| 输入核心 | Notebook、Schedule-Task、执行报告、环境信息 | 某个明确的 To-do / Schedule-Task |
| 输出核心 | 创建 / 更新 / 取消 Schedule-Task，写入 Review 标记 | 执行结果、任务报告 |
| 工作方式 | 周期性后台检查 | 到点触发或用户触发 |
| 成功标准 | 计划系统正确、无重复、不过早、不遗漏重要事项 | 具体任务被完成 |

系统提示词中必须明确：

> Self-Check 不负责真正执行任务。它负责创建正确的计划任务，并在计划任务失效时取消或调整它们。

例如，“每天 14:00 检查邮件里有没有待办事项”这类记录，Self-Check 的职责是创建一个合适的计划任务；真正到点打开邮箱、检查邮件、生成结果报告，是后续执行流程的职责。



---

## 3. 关键概念

### 3.1 Notebook

Notebook 是用户自然语言记录的集合。它包含用户随手写下、说出或由系统沉淀的各种事项。

它是日程、提醒、计划意图的唯一原点。

### 3.2 Notebook Item

Notebook 中的一条记录。它可能是明确计划，也可能只是模糊想法。

例如：

- “明天上午提醒我带护照。”
- “最近要研究一下照片整理。”
- “每天两点看一下邮箱有没有待办。”
- “周五去赶火车，别迟到。”

### 3.3 Schedule-Task

Schedule-Task 是基于 Notebook Item 经过深度 Review 后生成的结构化计划任务。

它不是原始事实，而是 Notebook 的派生物。

> Agent自己创建的计划任务是符合上述规则的，但系统也允许别人手工给Agent指派任务，通过任务的创建者，Agent可以区分两者

### 3.4 Self-Check Round

每次 Self-Check 被唤醒、完成读取、分析、创建 / 取消 / 更新任务、写入结果的完整过程，称为一个 Round。

### 3.5 Execution Report

Schedule-Task 到点触发后，由执行流程产生的执行报告。Self-Check 可以读取这些报告，用来判断计划是否持续有效。

> Self-Check只关注是否要调整该计划任务，而不会深度的探索执行的流程是否正确

### 3.6 Scan / Review 标记

Self-Check 每次处理 Notebook Item 后，都应写入或更新该 Item 的 Review 状态，便于查询侧知道哪些内容已经被处理、哪些仍待处理。

---

## 4. 最重要的设计原则

### 4.1 Notebook 是唯一真相源

Schedule-Task 是 Notebook 的衍生品，而不是最终事实源。

```text
Notebook Item
  -> Self-Check Review
  -> Schedule-Task / Reminder / Schedule Task
```

因此：

- 用户原始意图以 Notebook 为准。
- Schedule-Task 必须记录来源 Notebook Item。
- 当 Notebook 中出现新的否定、删除、变更信息时，Self-Check 需要重新评估对应 Schedule-Task。
- 查询“我现在有哪些安排”时，应综合 Notebook、Self-Check 标记和 Schedule-Task，而不是只看 Self-Check 是否刚刚运行。

### 4.2 Self-Check 管理计划系统，不执行计划内容

Self-Check 的主要动作是：

- Review Notebook Item。
- 检查是否已有对应 Schedule-Task。
- 创建新的 Schedule-Task。
- 更新已有 Schedule-Task。
- 取消失效 Schedule-Task。
- 查看执行报告并发现长期失败或无效计划。
- 写入本轮 Self-Check Round 结果。

Self-Check 不应该：

- 到点执行邮件检查。
- 实际整理照片。
- 直接完成开放式研究任务。
- 在没有 Schedule-Task 的情况下临时执行用户任务。

### 4.3 创建任务要谨慎，但不能漏掉明确事项

Self-Check 不是“尽量多创建任务”的系统。

正确目标是：

> 创建正确的计划任务。

正确包括：

- 正确理解用户意图。
- 正确选择任务类型。
- 正确设置时间和重复规则。
- 正确生成提醒文案或任务描述。
- 正确关联 Notebook Item。
- 正确避免重复。
- 正确处理取消条件。

### 4.4 取消任务也要谨慎

Self-Check 需要取消已经无效的计划任务，但不能因为弱信号就随意取消。

必须区分：

```text
明确取消
vs
暂时搁置
vs
优先级降低
vs
信息不完整
```

只有在证据足够明确时，才应取消任务。

### 4.5 72 小时窗口是核心机制

72 小时有双重含义：

1. **语义观察窗口**：模糊事项可以在 72 小时内等待更多信息自然浮现。
2. **上下文管理窗口**：Self-Check 只加载最近 72 小时内的 Round 历史，避免 History Prompt 上下文无限膨胀。

### 4.6 优先选择低复杂度任务类型

任务类型复杂度从低到高：

```text
Reminder / SendMessage
  < Workflow Pipeline
  < Agent WorkSession
```

提示词应引导模型：

- 能用 Reminder 解决，不要创建 Workflow。
- 能映射到固定 Workflow Pipeline，不要创建开放 Agent Task
- 只有任务确实需要动态推理或开放执行时，才创建 Agent Task。

---

## 5. Self-Check 的输入范围

Self-Check 系统提示词应明确模型每轮可以读取哪些输入。

### 5.1 必需输入

1. 当前 Notebook 中与提醒、计划、待办、未来安排、周期性任务相关的全部候选条目。
2. 上一次 Self-Check 后新增或修改的 Notebook Item。
3. 每个 Notebook Item 的 Self-Check 标记。
4. 已存在的 Schedule-Task。
5. Schedule-Task 与 Notebook Item 的关联关系。
6. 已取消、已完成、已失败或执行中的 Schedule-Task。
7. Schedule-Task 的历史执行报告。
8. 最近 72 小时内的 Self-Check Round 历史。
9. 当前时间、用户时区、系统可用能力。

### 5.2 可选输入

在需要时，Self-Check 可以结合：(background env)

- 用户常用位置。
- 用户日程或上下文信息。
- 联系人、邮件、照片、文件等可用资源的摘要。
- 系统内已有 Workflow Pipeline 列表。
- 用户历史上对某些词的定义。
- 用户偏好，例如提醒提前量、工作时间、勿扰时间。

例如，用户只写“周五去赶火车，别迟到”，如果系统有足够环境信息，Self-Check 可以尝试推导：

- 具体日期。
- 可能的出发地和目的地。
- 合理提醒时间。
- 是否需要提前出门提醒。

但如果推导证据不足，应继续观察或创建低风险提醒，而不是虚构细节。

---

## 6. Self-Check 的输出范围

每轮 Self-Check 至少应产生以下几类输出。

### 6.1 Notebook Item Review 结果

对每条相关 Notebook Item，应记录：

- 是否已扫描。
- 是否已深度 Review。
- 判断结果。
- 是否进入 72 小时观察窗口。
- 是否关联已有 Schedule-Task。
- 是否创建了新 Schedule-Task。
- 是否取消或更新了已有 Schedule-Task。
- 不创建任务的原因。

### 6.2 Schedule-Task 动作

可能的动作包括：

- `create_task`：创建新的计划任务。
- `update_task`：更新已有计划任务。
- `cancel_task`：取消已有计划任务。
- `link_task`：将 Notebook Item 与已有 Schedule-Task 关联。
- `keep_observing`：继续观察，暂不创建。
- `mark_no_action`：明确无需创建任务。
- `flag_for_user`：需要用户确认，但不要在 Self-Check 中假装已确认。

### 6.3 Round Summary

每轮应写入一份简短执行摘要，包括：

- 本轮扫描了多少条 Notebook Item。
- 重点新增条目有哪些。
- 创建了哪些 Schedule-Task。
- 取消或更新了哪些 Schedule-Task。
- 哪些事项进入观察窗口。
- 哪些事项因为信息不足未处理。
- 是否发现长期失败任务或异常任务。

---

## 7. Notebook Item 的 Self-Check 标记设计

为了支持查询侧和下一轮 Self-Check，Notebook Item 应具备可追踪状态。

建议字段：

```yaml
self_check:
  last_scanned_at: "2026-05-27T10:15:00-07:00"
  last_review_round_id: "round_abc123"
  review_status: "observing"
  review_depth: "deep"
  observation_started_at: "2026-05-27T10:15:00-07:00"
  observation_until: "2026-05-30T10:15:00-07:00"
  linked_plan_task_ids:
    - "task_123"
  decision: "keep_observing"
  decision_reason: "User expressed weak intent, but time and action are not clear yet."
  confidence: 0.62
```

### 7.1 推荐状态枚举

| 状态 | 含义 |
|---|---|
| `unscanned` | 尚未被 Self-Check 扫描 |
| `scanned` | 已扫描，但未深度 Review |
| `deep_reviewed` | 已完成深度 Review |
| `observing` | 进入 72 小时观察窗口 |
| `task_created` | 已创建对应 Schedule-Task |
| `linked_existing_task` | 已关联已有 Schedule-Task |
| `no_action` | 明确无需创建任务 |
| `needs_user_confirmation` | 信息不足，需要用户确认 |
| `superseded` | 被其他更新或计划覆盖 |
| `cancel_signal_detected` | 检测到取消信号 |

### 7.2 为什么需要标记

因为用户可能在 Self-Check 尚未运行前就询问：

> 你现在帮我做了哪些日程安排？

此时聊天侧不能假装最新 Notebook Item 已经被深度分析。它应该能根据标记坦然回答：

- 已创建了哪些计划任务。
- 哪些 Notebook Item 已被扫描但未创建任务。
- 哪些新增事项仍未被 Self-Check 深度 Review。
- 哪些事项正在 72 小时观察窗口内。

---

## 8. Self-Check Round 与上下文管理

Self-Check 是 Plain Prompt 驱动的后台推理流程，每次运行至少消耗一次 LLM 推理。因此必须控制频率和上下文。

### 8.1 Round 定义

一次完整 Self-Check Round：

```text
Self-Check 被唤醒
  -> 读取 Notebook 增量与相关上下文
  -> Review 已有 Schedule-Task 与执行报告
  -> 创建 / 更新 / 取消 / 观察
  -> 写入 Notebook Item 标记
  -> 写入 Round Summary
  -> Round 结束
```

### 8.2 只加载最近 72 小时 Round History

Self-Check 的历史记录只作为短期推理痕迹，不是长期事实源。

```text
Notebook / Schedule-Task = 长期状态
Self-Check Round History = 短期推理痕迹
```

提示词应要求：

- 只使用最近 72 小时内的 Self-Check Round History。
- 超过 72 小时的 Round 不再进入上下文。
- 长期事实必须沉淀到 Notebook、Schedule-Task 或用户偏好中。
- 不要依赖旧 Round History 来恢复计划状态。

### 8.3 触发频率

Self-Check 不应无条件高频运行。

建议策略：

| 状态 | 频率建议 |
|---|---|
| Notebook 有新增 / 修改 Item | 较高频率，例如 15 分钟级别 |
| Notebook 无变化 | 降低频率，例如 30 分钟级别或更低 |
| 没有待 Review Item | 可继续降频 |
| 有观察窗口内事项 | 保持周期性检查 |
| 系统资源紧张 | 优先处理明确高价值事项 |

核心原则：

> Notebook 没有新增或变化时，Self-Check 可以不运行或降低频率。

---

## 9. 72 小时观察窗口

### 9.1 为什么需要 72 小时

很多用户意图第一次出现时并不完整。例如：

- “有空看看这个。”
- “最近可能要整理照片。”
- “也许要弄个 demo。”
- “周五那个事情别忘了。”

这些信号可能只是临时想法，不一定应该立刻变成任务。

72 小时窗口允许系统等待更多自然信息出现：

- 用户再次提到同一事项。
- Notebook 中出现补充上下文。
- 相关邮件、日历、文件、位置等信息出现。
- 时间变得更接近，风险变高。

### 9.2 什么时候立即创建任务

即使在 72 小时窗口内，以下情况也应立即创建任务：

1. 用户表达明确提醒请求。
2. 时间、动作、对象都足够清晰。
3. 错过会造成明显损失。
4. 该事项是强 deadline 或强时间约束。
5. Notebook Item 明确说“提醒我”“每天”“每周”“到点帮我做”。

例如：

```text
明天早上 8 点提醒我带护照。
```

应立即创建 Reminder。

### 9.3 什么时候进入观察

以下情况更适合观察：

1. 意图模糊。
2. 时间不清楚。
3. 动作不清楚。
4. 用户只是表达想法，不是安排。
5. 需要等待更多上下文。
6. 创建任务可能造成干扰。

例如：

```text
最近要不要做一个照片整理的流程？
```

更适合进入观察，而不是立即创建每日任务。

### 9.4 72 小时后如何处理

如果 72 小时后仍没有更多信息：

- 明确事项：仍应创建任务或保持已有任务。
- 低置信模糊事项：标记为 `no_action` 或 `needs_user_confirmation`。
- 可能有价值但缺关键参数：保留 Notebook 原文，不创建计划任务。
- 不应无限观察同一事项。

---

## 10. Schedule-Task 的三种类型

Self-Check 在创建计划任务时，首先要判断任务类型。

### 10.1 类型一：Reminder / SendMessage

这是最简单、最常见、最稳定的计划任务。

本质：

```text
schedule
  -> send fixed message to target user
```

适合：

- 到点提醒。
- 周期性提醒。
- 不需要工具调用。
- 不需要复杂推理。
- 不需要动态变量。

示例：

```text
每工作 30 分钟提醒我喝水。
```

可创建：

```yaml
type: reminder
schedule: "every 30 minutes during working session"
recipient: "user"
message: "该喝水了。"
source_notebook_item_id: "note_123"
```

#### Reminder 创建规则

1. 消息内容应在创建时确定。
2. 不要依赖运行时动态生成复杂变量。
3. 时间、重复规则、接收者必须明确。
4. 文案要简短、可直接发送。
5. 如果时间不明确，优先观察或需要确认。

### 10.2 类型二：Workflow Pipeline

Workflow Pipeline 是固定的、结构化的工具流。

本质：

```text
schedule
  -> trigger known pipeline
  -> pipeline executes predefined steps
```

适合：

- 系统已经有标准管线。
- 用户意图能映射到某个已定义流程。
- 流程需要组合工具，但不需要每次自由推理。
- 希望稳定、可观察、可调试。

示例：

```text
每天整理一下今天新收到的照片。
```

如果系统中已有“整理照片”Pipeline，且“整理”被定义为：

- 检测新增照片。
- OCR 名片。
- 提取地点和时间。
- 聚类。
- 归档。

则 Self-Check 应创建 Pipeline Task，而不是让 Agent 每天重新解释“整理”是什么意思。

#### Pipeline 创建规则

1. 只映射到已有 Pipeline。
2. 不在 Self-Check 中临时发明复杂 DSL。
3. 必须填充必要参数。
4. 如果 Pipeline 不存在，考虑创建 Agent WorkSession 或进入观察。
5. 如果用户对词语有历史定义，应优先使用该定义。

### 10.3 类型三：Agent WorkSession

Agent WorkSession 是最通用、但成本最高、稳定性最低的计划任务。

本质：

```text
schedule
  -> launch agent work session
  -> execute explicit todo
```

适合：

- 开放式任务。
- 需要动态推理。
- 无法映射到固定 Pipeline。
- Reminder 不足以完成用户意图。
- 需要读取上下文、分析、总结、生成报告。

示例：

```text
每周 review 一下我最近的重要项目风险。
```

可创建：

```yaml
type: agent_worksession
schedule: "weekly"
todo: "Review the user's recent important project notes and summarize key risks, blockers, and suggested next actions."
source_notebook_item_id: "note_456"
session_policy: "reuse_session"
```

#### Agent WorkSession 的关键要求

创建这类任务时，Self-Check 必须写清楚 To-do。

不能只写：

```text
帮用户处理一下。
```

而应写：

```text
每周一上午读取最近 7 天项目相关 Notebook Item，识别风险、阻塞点和下一步建议，并向用户发送摘要。
```

### 10.4 WorkSession 的两种 Session 模式

#### 模式 A：复用固定 Session

```text
schedule_task_id
  -> same work session
```

适合长期连续上下文：

- 长期项目 review。
- 长期研究。
- 长期数据跟踪。
- 用户希望同一个任务持续积累记忆。

#### 模式 B：每次创建新 Session

```text
scheduler trigger
  -> create subtask
  -> new work session
```

适合每次独立执行：

- 每日摘要。
- 每次检查邮件。
- 每次独立生成报告。
- 不希望历史执行污染当前判断。

### 10.5 任务类型选择规则

系统提示词中应明确如下优先级：

```text
1. 如果只是到点发送固定消息 -> Reminder
2. 如果可以映射到已知固定管线 -> Workflow Pipeline
3. 如果需要动态推理或开放执行 -> Agent WorkSession
4. 如果信息不足 -> 观察或需要用户确认
```

---

## 11. 创建 Schedule-Task 的决策规则

Self-Check 创建任务前，应依次检查以下问题。

### 11.1 是否存在真实用户意图

判断 Notebook Item 是否表达了：

- 用户想被提醒。
- 用户想让系统周期性执行。
- 用户想在未来某个时间处理。
- 用户想持续关注某件事。

不是所有记录都应该创建任务。

例如：

```text
今天看到一个有意思的照片整理工具。
```

这可能只是记录，不一定是计划。

### 11.2 是否具备足够执行条件

至少需要明确：

- 做什么。
- 什么时候做或何时触发。
- 对谁做。
- 任务类型。
- 是否重复。
- 结果如何反馈。

如果缺失关键条件，应观察或标记需要确认。

### 11.3 是否已有相同或等价 Schedule-Task

创建前必须检查已有任务：

- 是否已有相同 Notebook Item 创建的任务。
- 是否已有同义任务。
- 是否已有更宽泛任务覆盖该事项。
- 是否用户手工创建过类似任务。
- 是否其他 Agent 已创建。

如果已有对应任务：

- 不重复创建。
- 关联 Notebook Item。
- 必要时更新已有任务。

### 11.4 是否应等待 72 小时

如果意图不稳定，应进入观察。

如果意图明确，应立即创建。

### 11.5 是否会造成用户打扰

Self-Check 应避免创建高频、低价值提醒。

例如，“有空看看”不应被转换成每天提醒。

### 11.6 是否有取消条件

计划任务最好带有取消或失效条件。

例如：

- 某日期后自动失效。
- 关联 Notebook Item 被取消时取消。
- 连续多次失败后进入 review。
- 用户明确说不用做时取消。

---

## 12. 取消 Schedule-Task 的决策规则

Self-Check 需要管理已有 Schedule-Task 的生命周期。

### 12.1 应取消的典型情况

1. Notebook 中出现明确取消信号：
   - “不用做了。”
   - “取消这个提醒。”
   - “这个计划删掉。”
   - “这件事不跟了。”

2. 任务已经过期：
   - 事件已经发生。
   - 截止日期已过。
   - 错过后执行没有意义。

3. 前提条件不再成立：
   - 用户已改变计划。
   - 目标被替代。
   - 相关资源不存在。

4. 被新计划覆盖：
   - 新 Notebook Item 明确替代旧安排。
   - 新任务范围包含旧任务。

5. 长期执行失败：
   - 多次执行都失败。
   - 原因不可自动恢复。
   - 继续执行只会浪费资源或骚扰用户。

### 12.2 长期失败任务的处理

例如，Schedule-Task 是：

```text
每天下午 2 点检查电子邮件里的待办事项。
```

如果执行报告显示连续多天失败：

- 邮箱打不开。
- 权限失效。
- 登录失败。
- 工具不可用。

Self-Check 应重新评估：

- 是否需要提示用户重新授权。
- 是否暂停该任务。
- 是否建议取消。
- 是否更新执行方式。

如果失败原因可能恢复，优先暂停 / 标记异常 / 请求用户处理；如果明确无意义，再取消。

### 12.3 不应取消的情况

以下情况不应直接取消：

- 用户只是说“以后再说”。
- 用户只是降低优先级。
- 执行偶发失败一次。
- 暂时缺少上下文。
- 任务仍在有效时间范围内。

---

## 13. 查询侧与 Self-Check 的边界

当用户在聊天 UI 中询问：

> 你现在帮我做了哪些日程安排？

这不是 Self-Check 当场执行的职责。

查询侧应该读取：

1. Notebook 中的原始日程相关记录。
2. Notebook Item 的 Self-Check 标记。
3. 已创建的 Schedule-Task。
4. Schedule-Task 的创建者、来源、状态。
5. 用户手工创建的计划任务。

然后如实回答：

- 已经创建了哪些计划任务。
- 哪些是 Agent / Self-Check 创建的。
- 哪些是用户手工创建的。
- 哪些 Notebook Item 还未被 Self-Check 深度 Review。
- 哪些事项仍在观察窗口内。

核心原则：

> Self-Check 是后台深度分析流程；UI 查询侧负责基于现有状态如实展示。

---

## 14. Schedule-Task 数据结构建议

Schedule-Task 至少应包含以下字段。

```yaml
plan_task:
  id: "task_123"
  type: "reminder | workflow_pipeline | agent_worksession"
  status: "active | paused | cancelled | completed | failed | expired"
  creator: "self_check | user | agent | system"
  source:
    notebook_id: "notebook_1"
    notebook_item_ids:
      - "note_123"
    self_check_round_id: "round_abc123"
  schedule:
    kind: "one_time | recurring | event_based"
    time: "2026-05-28T08:00:00-07:00"
    recurrence: null
    timezone: "America/Los_Angeles"
  payload:
    message: "明天早上记得带护照。"
    workflow_id: null
    todo: null
  execution_policy:
    session_policy: null
    retry_policy: "default"
    failure_review_threshold: 3
  lifecycle:
    expires_at: null
    cancel_conditions:
      - "source notebook item explicitly cancelled"
  metadata:
    created_at: "2026-05-27T10:15:00-07:00"
    updated_at: "2026-05-27T10:15:00-07:00"
```

不同类型任务使用不同 payload：

### Reminder

```yaml
payload:
  message: "该喝水了。"
```

### Workflow Pipeline

```yaml
payload:
  workflow_id: "photo_organize_pipeline"
  parameters:
    source: "new_photos_today"
    scope: "today"
```

### Agent WorkSession

```yaml
payload:
  todo: "读取最近 7 天项目相关 Notebook Item，识别风险、阻塞点和建议动作，并发送摘要。"
  session_policy: "reuse_session | new_session_per_trigger"
```

---

## 15. 系统提示词应包含的结构

Self-Check Behavior 的系统提示词建议包含以下模块。

### 15.1 Role

说明模型身份。

示例：

```text
你是 Self-Check Agent。你的职责是周期性读取 Notebook 中的用户记录，结合已有 Schedule-Task、执行报告和环境信息，判断是否需要创建、更新、取消或继续观察计划任务。你不负责执行计划任务本身。
```

### 15.2 Source of Truth

强调 Notebook 是唯一真相源。

```text
Notebook 是用户日程、提醒和计划意图的唯一原点。Schedule-Task 是你基于 Notebook 深度 Review 后创建的派生状态。创建、更新或取消 Schedule-Task 时，必须保留与来源 Notebook Item 的关联。
```

### 15.3 Inputs

列出每轮输入。

```text
你会收到：当前相关 Notebook Item、新增或修改 Item、每个 Item 的 Review 标记、已有 Schedule-Task、Schedule-Task 执行报告、最近 72 小时 Self-Check Round History、当前时间和可用系统能力。
```

### 15.4 Core Duties

明确职责。

```text
你的核心职责是：
1. 深度 Review 新增或修改的 Notebook Item。
2. 检查已有 Schedule-Task，避免重复创建。
3. 判断是否需要创建 Reminder、Workflow Pipeline 或 Agent WorkSession。
4. 判断已有 Schedule-Task 是否失效、过期、被取消或长期失败。
5. 写入每条 Notebook Item 的 Review 状态。
6. 写入本轮 Round Summary。
```

### 15.5 Decision Policy

规定创建、观察、取消策略。

```text
创建任务时必须谨慎。只有当用户意图足够明确，或错过会造成明显损失时，才创建 Schedule-Task。对于模糊、不完整、低置信度的事项，应进入 72 小时观察窗口。取消任务时也必须谨慎，只有在明确取消、过期、前提失效、被新计划覆盖或长期失败时才取消。
```

### 15.6 Task Type Policy

规定三类任务选择。

```text
优先选择最低复杂度的任务类型：
- 如果只是到点发送固定消息，创建 Reminder / SendMessage。
- 如果意图可以映射到已有固定 Workflow Pipeline，创建 Pipeline Task。
- 如果需要开放式推理或动态执行，创建 Agent WorkSession，并写清楚 To-do。
```

### 15.7 Context Policy

规定 72 小时历史限制。

```text
只使用最近 72 小时内的 Self-Check Round History。不要依赖更早的 Round History 作为事实来源。长期事实必须来自 Notebook、Schedule-Task 或系统状态。
```

### 15.8 Output Contract

要求结构化输出。

```text
你的输出必须包含：item_reviews、task_actions、round_summary。每个 task_action 都必须说明来源、动作、原因、置信度和是否需要用户确认。
```

---

## 16. 系统提示词模板

下面是一份可作为起点的系统提示词草案。

```text
你是 Self-Check Agent，一个周期性运行的后台计划整理器。

你的目标不是执行具体任务，而是基于 Notebook 这个唯一真相源，持续 Review 用户自然语言记录中的提醒、计划、待办、周期性检查和未来安排，并管理对应的 Schedule-Task。

【唯一真相源】
Notebook 是用户日程安排、提醒和计划意图的原点。Schedule-Task 是你基于 Notebook Item 深度 Review 后产生的派生状态。任何 Schedule-Task 的创建、更新、取消都必须保留来源 Notebook Item 的关联。

【你会收到的输入】
1. 当前相关 Notebook Item。
2. 自上次 Self-Check 后新增或修改的 Notebook Item。
3. 每个 Notebook Item 的 Self-Check Review 标记。
4. 已有 Schedule-Task，包括 active、paused、cancelled、completed、failed、expired 等状态。
5. Schedule-Task 的执行历史与 Execution Report。
6. 最近 72 小时内的 Self-Check Round History。
7. 当前时间、用户时区、可用环境信息和系统能力。

【核心职责】
1. 重点分析新增或修改的 Notebook Item。
2. 使用旧 Notebook Item 作为上下文，但不要每轮重复深度分析所有旧内容。
3. 判断 Notebook Item 是否表达了真实、可执行、值得计划化的用户意图。
4. 创建正确类型的 Schedule-Task：Reminder / SendMessage、Workflow Pipeline 或 Agent WorkSession。
5. 创建前必须检查是否已有相同或等价 Schedule-Task，避免重复创建。
6. 检查已有 Schedule-Task 是否过期、失效、被明确取消、被新计划覆盖或长期执行失败。
7. 对每条相关 Notebook Item 写入或更新 Self-Check Review 标记。
8. 写入本轮 Round Summary。

【72 小时窗口】
对于模糊、不完整、低置信度的事项，不要急于创建任务。将其放入 72 小时观察窗口，等待更多 Notebook、环境或用户行为信息自然浮现。

如果用户意图明确，或者错过会造成明显损失，不需要等待 72 小时，应立即创建任务。

你只能使用最近 72 小时内的 Self-Check Round History。更早的 Round History 不应作为上下文加载。长期事实应来自 Notebook、Schedule-Task 或系统状态。

【任务类型选择】
优先选择最低复杂度的任务类型：
1. Reminder / SendMessage：适用于到点发送固定消息的提醒类任务。消息内容应在创建时确定，不要依赖复杂运行时变量。
2. Workflow Pipeline：适用于可以映射到已有固定管线的任务。不要临时发明复杂 DSL；只映射到系统已知 Pipeline，并填充必要参数。
3. Agent WorkSession：适用于开放式、需要动态推理的任务。创建时必须写清楚明确 To-do。根据任务性质选择复用固定 Session 或每次触发创建新 Session。

【创建 Schedule-Task 的规则】
只有当以下条件基本满足时，才创建 Schedule-Task：
- 存在真实用户意图。
- 动作、时间或触发条件足够明确。
- 可以确定合适任务类型。
- 不存在相同或等价的已有 Schedule-Task。
- 创建任务不会造成明显不必要打扰。

【取消 Schedule-Task 的规则】
只有在以下情况才取消 Schedule-Task：
- Notebook 中出现明确取消、删除、不再跟进信号。
- 任务已过有效期，继续执行没有意义。
- 前提条件不再成立。
- 新计划明确覆盖旧计划。
- 执行报告显示长期失败，且没有可自动恢复路径。

不要因为一次失败、弱否定、优先级降低或信息暂时不足就取消任务。

【执行边界】
你不负责执行 Schedule-Task 的具体内容。比如“每天 14:00 检查邮件”中，你只负责创建正确计划任务；真正到点打开邮箱、检查邮件、生成报告，由执行流程完成。

【输出格式】
每轮输出必须包含以下结构：

item_reviews:
- notebook_item_id: string
  review_status: unscanned | scanned | deep_reviewed | observing | task_created | linked_existing_task | no_action | needs_user_confirmation | superseded | cancel_signal_detected
  decision: create_task | update_task | cancel_task | link_task | keep_observing | mark_no_action | flag_for_user
  reason: string
  confidence: number
  linked_plan_task_ids: string[]
  observation_until: timestamp | null

task_actions:
- action: create_task | update_task | cancel_task | link_task | none
  task_type: reminder | workflow_pipeline | agent_worksession | null
  source_notebook_item_ids: string[]
  target_plan_task_id: string | null
  payload: object
  reason: string
  confidence: number
  requires_user_confirmation: boolean

round_summary:
  scanned_count: number
  deeply_reviewed_count: number
  created_task_count: number
  updated_task_count: number
  cancelled_task_count: number
  observing_count: number
  unresolved_count: number
  notes: string

输出中不要隐藏不确定性。对信息不足、推导不确定、需要用户确认的事项，要明确标记，不要假装已经完成。
```

---

## 17. 示例决策

### 17.1 示例一：简单提醒

Notebook Item：

```text
每工作 30 分钟提醒我喝水。
```

判断：

- 意图明确。
- 任务简单。
- 不需要工具。
- 应创建 Reminder。

Schedule-Task：

```yaml
type: reminder
schedule:
  kind: recurring
  recurrence: "every 30 minutes during active work session"
payload:
  message: "该喝水了。"
```

### 17.2 示例二：每日邮件检查

Notebook Item：

```text
每天下午 2 点帮我检查电子邮件，看看有没有待办事项。
```

判断：

- 这是周期性任务。
- 需要访问邮件并提取待办。
- 如果系统有固定邮件待办提取 Pipeline，则创建 Workflow Pipeline。
- 如果没有固定 Pipeline，但 Agent 可执行，则创建 Agent WorkSession。

Agent WorkSession To-do 示例：

```text
每天下午 2 点检查用户新收到的电子邮件，识别其中明确的待办事项、截止时间和相关联系人，并向用户发送摘要。若邮件访问失败，生成执行报告说明失败原因。
```

### 17.3 示例三：照片整理

Notebook Item：

```text
每天整理一下今天新收到的照片。
```

判断：

- “整理”可能有用户历史定义。
- 如果已有“照片整理 Pipeline”，应映射到 Pipeline。
- 不应让 Agent 每天自由解释整理规则。

Schedule-Task：

```yaml
type: workflow_pipeline
workflow_id: photo_organize_pipeline
schedule:
  kind: recurring
  recurrence: daily
parameters:
  scope: "photos received today"
```

### 17.4 示例四：赶火车

Notebook Item：

```text
周五赶火车，别迟到。
```

判断：

- 有提醒意图。
- 缺少具体车次、时间、地点。
- 可以结合环境信息尝试推导。
- 如果推导仍不足，应进入观察或创建保守提醒。

可能动作：

```yaml
decision: keep_observing
reason: "User mentioned a train trip but exact departure time and station are not known. Observe for additional context within 72 hours."
```

如果已有日历 / 票务信息能确认车次，则可以创建提前提醒。

### 17.5 示例五：取消计划

Notebook Item：

```text
明天那个火车不用去了，取消提醒。
```

判断：

- 明确取消。
- 应查找相关火车提醒 Schedule-Task。
- 取消对应任务。

动作：

```yaml
action: cancel_task
reason: "Notebook explicitly says the train trip reminder should be cancelled."
```

### 17.6 示例六：长期失败任务

Schedule-Task：

```text
每天 14:00 检查邮箱待办。
```

Execution Reports：

```text
连续 5 次失败：邮箱授权失效。
```

判断：

- 不应无限继续失败。
- 应标记异常。
- 可建议用户重新授权。
- 如系统策略允许，可暂停任务；若明确无恢复路径，可取消。

动作：

```yaml
action: update_task
status: paused
reason: "Email access failed repeatedly due to authorization issue. Pause until user re-authorizes."
```

---

## 18. 常见错误与反模式

### 18.1 看到模糊想法就创建任务

错误：

```text
用户说“有空研究一下照片整理”，系统创建每天提醒。
```

正确：

- 进入 72 小时观察。
- 等待更多信息。
- 或标记需要用户确认。

### 18.2 不检查已有任务导致重复创建

错误：

```text
同一 Notebook Item 每轮都创建一个新提醒。
```

正确：

- 创建前检查已有 Schedule-Task。
- 如果已有，关联或更新。

### 18.3 把 Schedule-Task 当成唯一真相源

错误：

```text
只看 Schedule-Task，不看 Notebook 中用户已经说“取消”。
```

正确：

- Notebook 是原点。
- Schedule-Task 是派生状态。

### 18.4 取消过度敏感

错误：

```text
用户说“这事晚点再看”，系统取消整个计划。
```

正确：

- 判断为延期或优先级降低。
- 不直接取消。

### 18.5 在 Reminder 中设计复杂动态变量

错误：

```text
提醒文案运行时再根据一堆状态动态生成。
```

正确：

- Reminder 应是固定消息。
- 复杂逻辑应放入 Pipeline 或 WorkSession。

### 18.6 在 Self-Check 中发明复杂 Workflow DSL

错误：

```text
模型临时写出一套复杂 pipeline DSL。
```

正确：

- 当前阶段只映射已有 Pipeline。
- DSL 合成可以作为后续扩展能力。

### 18.7 无限加载历史 Round

错误：

```text
每轮加载所有 Self-Check 历史。
```

正确：

- 只加载 72 小时内 Round History。
- 长期信息沉淀到 Notebook / Schedule-Task。

---

## 19. 提示词编写检查清单

编写 Self-Check Behavior 系统提示词时，应检查以下问题。

### 职责边界

- [ ] 是否明确 Self-Check 不执行任务，只管理 Schedule-Task？
- [ ] 是否明确 Notebook 是唯一真相源？
- [ ] 是否明确 Schedule-Task 是派生状态？

### 输入输出

- [ ] 是否列出 Notebook、Schedule-Task、Execution Report、Round History 等输入？
- [ ] 是否要求输出 item_reviews、task_actions、round_summary？
- [ ] 是否要求写入 Notebook Item Review 标记？

### 创建规则

- [ ] 是否要求创建前检查重复任务？
- [ ] 是否定义三种任务类型？
- [ ] 是否要求优先选择低复杂度任务？
- [ ] 是否要求 Agent WorkSession 必须有明确 To-do？

### 取消规则

- [ ] 是否定义明确取消、过期、前提失效、被覆盖、长期失败等取消条件？
- [ ] 是否提醒不要因为弱信号取消？

### 72 小时窗口

- [ ] 是否把 72 小时作为模糊意图观察窗口？
- [ ] 是否把 72 小时作为 Round History 加载窗口？
- [ ] 是否说明 72 小时后不要无限观察？

### 成本控制

- [ ] 是否避免每轮深度 Review 所有旧 Notebook Item？
- [ ] 是否重点分析新增和修改 Item？
- [ ] 是否允许 Notebook 无变化时降频？

### 查询侧边界

- [ ] 是否说明用户查询日程时，不依赖 Self-Check 当场运行？
- [ ] 是否说明查询侧应读取 Notebook 标记和 Schedule-Task 状态？

---

## 20. 最终总结

Self-Check Behavior 的系统提示词，应围绕一个中心展开：

> 基于 Notebook 唯一真相源，持续、谨慎、可追踪地管理计划任务。

它要解决的问题不是“怎么执行一个任务”，而是：

- 哪些 Notebook Item 真的应该变成计划任务？
- 应该变成哪一种计划任务？
- 是否已经有任务覆盖了它？
- 是否需要继续观察？
- 是否有已有任务已经过期、失效或应该取消？
- 如何在成本可控的情况下持续运行？

因此，Self-Check 的提示词设计必须同时强调：

1. **Notebook 原点性**：Notebook 是唯一真相源。
2. **Schedule-Task 派生性**：任务是基于 Review 后的结构化结果。
3. **创建正确性**：不是多创建，而是创建正确任务。
4. **取消谨慎性**：识别无效任务，但避免误取消。
5. **任务类型分层**：Reminder、Workflow Pipeline、Agent WorkSession。
6. **72 小时窗口**：既是语义观察窗口，也是上下文管理边界。
7. **可追踪性**：每条 Notebook Item 都应有 Review 标记。
8. **成本可控性**：重点处理增量，不无限加载历史。
9. **执行边界清晰**：Self-Check 管理计划，不执行计划。

只要系统提示词能稳定约束以上行为，Self-Check 就可以成为一个可靠的后台计划发现与维护机制。
