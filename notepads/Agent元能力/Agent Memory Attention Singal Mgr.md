# Agent Memory Attention Signal 组件技术需求文档

版本：v0.1
目标阶段：支持 **Self Improve Stage-1：Clue Discovery / Attention Signal Extraction**
设计目标：为 **Memory Stage-2：Signal Consumption / Attention State Merge** 和 **Skill Signal Consumer** 做好数据准备

---

## 1. 背景

Agent Memory 的长期目标不是简单保存对话摘要，而是让 Agent 能从用户的长期 Session History 中持续发现：

* 正在发生的事件
* 被反复提及的对象
* 对对象的观察
* 对象之间的关系线索
* 用户世界中值得继续关注的语义结构
* Skill 系统中值得后续独立分析的 coverage gap 线索

当前设计将 Self Improve 拆成两个阶段：

```text
Stage-1:
Session History
    ↓
Attention Signal / Clue

Stage-2:
Attention Signal / Clue
    ↓
Signal Consumption / Attention State Merge
    ↓
Attention State / Memory Graph Candidate / Recallable Memory
```

本组件负责第一阶段。

第一阶段的核心职责是：

> 把 Session History 翻译成结构化的语义线索，而不是直接生成 Memory。

这里的“语义线索”有两类边界：

```text
Memory Graph Signals:
    Event / ObjectObservation / Relationship
    由 Stage-2 消费，进入 Attention State / Memory Graph Candidate。

Skill Self-Improve Signals:
    SkillCoverageGap 等 skill 相关 signal
    由独立 Skill Improvement / Candidate Skill Miner LLM 消费。
```

也就是说，本组件可以复用 Stage-1 的扫描、证据、游标和存储模型来发现 skill 相关线索，但不负责消费这些线索，也不负责生成或修改 Skill。

---

## 2. 组件名称

组件名：

```text
Agent Memory Attention Signal
```

但在 Prompt、LLM Tool、API 设计中，不建议使用过于抽象的：

```text
AddAttentionSignal
```

因为它对 LLM 不够语义化。

推荐在第一阶段使用更具体的语义动作：

```text
DiscoverEvent
DiscoverObjectObservation
DiscoverRelationship
```

底层仍然可以统一存储为：

```text
AttentionSignal
```

但面向 LLM 的接口应该是语义化的。

---

## 3. 核心目标

### 3.1 第一阶段目标

Agent Memory Attention Signal 组件必须支持从 Session History 中发现三类线索：

```text
1. Event Signal
2. Object Observation Signal
3. Relationship Signal
```

其中：

* Event Signal 关注时间轴上的事件。
* Object Observation Signal 关注语义图中的对象节点及其观察。
* Relationship Signal 关注一次 Session 扫描中出现的对象关系线索，以及线索里的态度、偏好、状态或关联。

这三类是 Memory Graph 的核心 Signal。

为了支持 Skill Self-Improve，Stage-1 还可以发现一类扩展 Signal：

```text
4. Skill Coverage Gap Signal
```

它关注：

```text
Plan 中有明确 TODO
但没有合适 Skill 覆盖
Do 阶段通过通用推理 / Global Object 探索完成
Reporter / Supervisor / User 认可结果
```

或：

```text
Plan 分配了 Skill
但实际 Skill 没有被使用
最终仍然成功
```

这类 Signal 只说明“Skill Graph 可能存在 coverage gap”，不等于新 Skill，也不进入 Memory Graph 的 Stage-2 merge 流程。

---

### 3.2 第二阶段准备目标

第一阶段生成的 Signal 不直接参与 Recall。

它们必须为第二阶段消费提供足够信息，包括：

* 来源 Session
* 来源 Message
* 时间窗口
* 证据片段
* 置信度
* 可能的归一化对象
* 可能的别名
* 可合并 Key
* 初始权重建议
* 是否适合进入观察期
* 是否具有用户私有价值

---

## 4. 非目标

Stage-1 不负责以下事情：

```text
1. 不直接写入 Recallable Memory
2. 不直接更新最终 Attention State / Memory Graph
3. 不直接参与自动 Recall
4. 不负责最终对象归一化
5. 不负责长期权重衰减
6. 不负责判断某事实是否应该永久保存
7. 不负责 Notebook / Character Event 的最终更新
8. 不负责生成 Candidate Skill
9. 不负责更新、拆分、合并、淘汰已有 Skill
10. 不负责消费 Skill Coverage Gap Signal
```

Stage-1 只做：

```text
发现线索
结构化线索
保存线索
标记来源
等待下游消费
```

---

## 5. 整体架构

```text
Session History
    ↓
History Window Reader
    ↓
Stage-1 Signal Extractor
    ↓
Semantic Tools
    ├── DiscoverEvent
    ├── DiscoverObjectObservation
    ├── DiscoverRelationship
    └── DiscoverSkillCoverageGap
    ↓
Attention Signal Store
    ├── Memory Stage-2 Pending Queue
    └── Skill Signal Pending Queue
```

这里的 Pending Queue 是逻辑视图，不是本文要定义的完整消费协议。本文只要求 Stage-1 将有效 Signal 写入 Store，并使下游能按 `agent_scope_id + target_consumer + lifecycle_status = pending_stage2 + created_at` 查询。Memory Stage-2 的 claim / lease / retry / merge / promote 协议由第二阶段文档定义。

对于 Skill Coverage Gap Signal，下游不是 Memory Stage-2，而是独立的 Candidate Skill Miner LLM。Store 可以使用同一套 source / evidence / lifecycle 基础字段，但必须通过 `signal_type` 或 `target_consumer` 明确路由，避免被 Memory Stage-2 当成普通对象或关系信号消费。

---

## 6. 基本处理模型

### 6.1 时间窗口扫描

Stage-1 按时间窗口处理 Session History。

例如：

```text
window_start = 2026-06-03 08:00
window_end   = 2026-06-03 12:00
```

系统会读取该时间窗口内所有未处理的 Session History。

```text
Session A: 08:00 ~ 12:00 未处理记录
Session B: 08:00 ~ 12:00 未处理记录
Session C: 08:00 ~ 12:00 未处理记录
```

每个 Session 在当前窗口内独立扫描。扫描输入应来自现有 Session History 组件，而不是从 LLM context snapshot 中反推。具体锚点使用：

```text
session_id
round_index
entry_seq
HistoryView.Raw / Full
```

其中 `round_index` 是 Session 内一次 LLM Loop 消费的索引，`entry_seq` 是该 round 内的平铺 entry 序号。Entry 可能是 chat message、behavior step 或控制事件，因此 Stage-1 不应假设所有输入都有 `message_id`。

扫描结果统一进入 Attention Signal Store。

---

### 6.2 扫描触发时机

Stage-1 可以在以下时机触发：

```text
1. 用户一段时间没有新输入
2. Session 进入 idle 状态
3. 固定时间窗口结束
4. 后台 Self Improve Job 调度
5. 手动触发 Self Improve
```

优先推荐在用户较长时间无输入后触发，以尽量保证一轮表达的完整性。

---

### 6.3 扫描游标

每个 Session History 需要维护扫描状态。

示例：

```json
{
  "session_id": "session_123",
  "last_scanned_round_index": 12,
  "last_scanned_entry_seq": 8,
  "last_scanned_at": "2026-06-03T12:00:00Z",
  "scan_status": "up_to_date"
}
```

Stage-1 只能扫描未处理记录。未处理范围由 `(session_id, round_index, entry_seq)` 判定。

成功写入 Signal 后，才能推进扫描游标。

---

### 6.4 输入隔离与防自我回声

Stage-1 的输入只允许来自普通 UI / Work Session 的原始 Session History，或明确标记为外部输入的系统事件。

以下内容不得作为 Stage-1 的输入：

```text
1. Self Improve Session 自己的 Session History
2. Attention Signal Store 中已有 Signal
3. Stage-2 生成的 Attention State / Memory Graph
4. 已经 emit 到 Agent Memory 的 Attention Hint
5. Self Improve 自己生成的 report / observation
```

因此本组件不需要靠自动召回过滤自我回声：Signal 默认不进入 Recall，Self Improve Session History 也不会被本阶段重新扫描。防回声边界由 History Window Reader 和调度输入控制保证。

---

## 7. Attention Signal 类型

第一阶段默认产生三种核心 Memory Graph Signal。

```text
AttentionSignal
    ├── EventSignal
    ├── ObjectObservationSignal
    └── RelationshipSignal
```

此外，为支持 Skill Self-Improve，可以扩展产生：

```text
AttentionSignal
    └── SkillCoverageGapSignal
```

核心约束：

```text
Event / ObjectObservation / Relationship
    → Memory Stage-2 消费
    → Attention State / Memory Graph Candidate

SkillCoverageGap
    → Candidate Skill Miner LLM 消费
    → Candidate Skill Case / Skill Draft / Reject
```

SkillCoverageGap 不应被 Memory Stage-2 提升成 Recallable Memory。

---

# 8. Event Signal

## 8.1 定义

Event Signal 表示一个有生命周期的事件。

事件必须具有某种时间性：

```text
有开始
有结束
或至少有阶段变化
```

事件可以是：

```text
正在计划的项目
即将发生的会议
正在推进的任务
一次旅行计划
一次运动会
一个产品发布
一个设计讨论阶段
```

---

## 8.2 非事件

以下不应该作为 Event Signal：

```text
某人的职业是软件工程师
用户喜欢 Costco
某台电脑很慢
Bob 是 Alice 的同事
```

这些更适合成为：

```text
Object Observation Signal
Relationship Signal
```

---

## 8.3 Event Signal 必填字段

```typescript
type EventSignal = {
  signal_type: "event";

  title: string;
  phase: EventPhase;

  description?: string;

  time_info: {
    start_time?: string;
    end_time?: string;
    time_range_text?: string;
    is_time_precise: boolean;
  };

  participants?: EntityMention[];

  source: SignalSource;

  evidence: Evidence[];

  confidence: number;

  stage2_hints?: Stage2Preparation;
};
```

---

## 8.4 Event Phase

事件阶段建议使用固定枚举。

```typescript
type EventPhase =
  | "idea"
  | "planning"
  | "scheduled"
  | "active"
  | "waiting"
  | "blocked"
  | "completed"
  | "cancelled"
  | "abandoned"
  | "unknown";
```

---

## 8.5 示例

用户说：

> 我们现在先讨论 Self Improve 第一阶段，第二阶段后面再消费这些信号。

生成：

```json
{
  "signal_type": "event",
  "title": "Self Improve 第一阶段设计讨论",
  "phase": "active",
  "description": "用户正在讨论 Self Improve Stage-1 如何从 Session History 中提取 Attention Signal。",
  "time_info": {
    "is_time_precise": false
  },
  "participants": [
    {
      "name": "Self Improve",
      "entity_type": "project_or_component"
    },
    {
      "name": "Agent Memory Attention Signal",
      "entity_type": "component"
    }
  ],
  "confidence": 0.91
}
```

---

# 9. Object Observation Signal

## 9.1 定义

Object Observation Signal 表示：

> 发现了一个具体对象，并且发现了关于这个对象的一条观察。

这里的 Object 是唯一性个体，不是类型。

---

## 9.2 应该提取的对象

```text
用户的笔记本电脑
Lucy
Bob
Alice
某个具体项目
某个内部系统
某个组件
某个家庭成员
某个设备
某个邮箱
某个联系人
某个公司内部角色
```

---

## 9.3 不应重点提取的对象

如果没有个性化上下文，以下公共对象通常不需要单独提取：

```text
Costco
Google
Apple
ChatGPT
World Market
```

但如果用户表达了私有偏好或关系，则可以提取。

例如：

```text
用户更喜欢 Costco，不喜欢 World Market。
```

此时应该生成 Relationship Signal，而不是简单忽略。

---

## 9.4 Object Observation Signal 必填字段

```typescript
type ObjectObservationSignal = {
  signal_type: "object_observation";

  object: EntityMention;

  observation: string;

  observation_type?: ObservationType;

  source: SignalSource;

  evidence: Evidence[];

  confidence: number;

  stage2_hints?: Stage2Preparation;
};
```

---

## 9.5 Object 字段

```typescript
type EntityMention = {
  mention_text: string;

  entity_type:
    | "person"
    | "project"
    | "component"
    | "device"
    | "organization"
    | "location"
    | "account"
    | "email"
    | "document"
    | "unknown";

  canonical_id_candidate?: string;

  alias_candidates?: string[];

  is_public_entity?: boolean;

  is_user_private_entity?: boolean;

  uniqueness_hint?: string;
};
```

---

## 9.6 Observation Type

```typescript
type ObservationType =
  | "status"
  | "preference"
  | "problem"
  | "capability"
  | "attribute"
  | "role"
  | "usage"
  | "note"
  | "uncertain";
```

---

## 9.7 示例

用户说：

> 我的笔记本电脑最近很慢。

生成：

```json
{
  "signal_type": "object_observation",
  "object": {
    "mention_text": "用户的笔记本电脑",
    "entity_type": "device",
    "is_user_private_entity": true
  },
  "observation": "用户认为自己的笔记本电脑最近运行很慢。",
  "observation_type": "problem",
  "confidence": 0.94
}
```

---

# 10. Relationship Signal

## 10.1 定义

Relationship Signal 表示：

> 在一次 Session History 扫描中，发现两个对象之间可能存在某种关系、态度、偏好、依赖、拥有、参与或关联。

Relationship Signal 是候选线索，不是 Memory Graph / Attention State 中已经成立的事实边。

```text
Object A
    ↓ relationship clue
Object B
```

Stage-1 只保存“这段历史里出现过这样的关系证据”。是否合并为已有对象之间的关系、是否转成 Memory Graph 的边、是否进入 Memory Hint，都由 Stage-2 决定。

---

## 10.2 Relationship 可以包含态度

Relationship 不只是事实连接线索，也可以包含态度。

例如：

```text
用户喜欢 Costco
用户不喜欢 World Market
Bob 对 Alice 不满意
Lucy 是某项目的负责人
用户把财务报表发给过 Lucy 的工作邮箱
```

---

## 10.3 Relationship Signal 必填字段

```typescript
type RelationshipSignal = {
  signal_type: "relationship";

  subject: EntityMention;

  predicate: string;

  object: EntityMention;

  attitude?: RelationshipAttitude;

  relation_type?: RelationshipType;

  temporal_context?: {
    event_title?: string;
    time_range_text?: string;
    is_time_bound: boolean;
  };

  source: SignalSource;

  evidence: Evidence[];

  confidence: number;

  stage2_hints?: Stage2Preparation;
};
```

---

## 10.4 Relationship Type

```typescript
type RelationshipType =
  | "likes"
  | "dislikes"
  | "owns"
  | "uses"
  | "works_with"
  | "responsible_for"
  | "participates_in"
  | "sent_to"
  | "received_from"
  | "depends_on"
  | "related_to"
  | "alias_of"
  | "contact_info"
  | "attitude_towards"
  | "unknown";
```

---

## 10.5 Relationship Attitude

```typescript
type RelationshipAttitude = {
  polarity: "positive" | "negative" | "neutral" | "mixed" | "unknown";

  sentiment_text?: string;

  strength?: "weak" | "medium" | "strong";

  is_explicit: boolean;
};
```

---

## 10.6 示例

用户说：

> Bob 对 Alice 不太满意。

生成：

```json
{
  "signal_type": "relationship",
  "subject": {
    "mention_text": "Bob",
    "entity_type": "person",
    "is_user_private_entity": true
  },
  "predicate": "is dissatisfied with",
  "object": {
    "mention_text": "Alice",
    "entity_type": "person",
    "is_user_private_entity": true
  },
  "relation_type": "attitude_towards",
  "attitude": {
    "polarity": "negative",
    "sentiment_text": "不太满意",
    "strength": "medium",
    "is_explicit": true
  },
  "confidence": 0.9
}
```

---

## 10.7 扩展：Skill Coverage Gap Signal

Skill Coverage Gap Signal 表示：

> 在一次 Session History 扫描中，发现某个明确 TODO 没有被现有 Skill 覆盖，但 Agent 仍然通过通用推理、工具探索或 Global Object 探索完成了任务。

它不是 Memory Graph 的对象、事件或关系，也不是新 Skill。

它只是一条需要交给 Skill Self-Improve 独立流程消费的 attention signal。

典型触发条件：

```text
1. Plan 有明确 TODO
2. assigned_skills 为空，或 assigned skill 实际未被使用
3. Do 阶段成功完成
4. Reporter / Supervisor / User 对结果给出正向或接受信号
```

必填字段建议：

```typescript
type SkillCoverageGapSignal = {
  signal_type: "skill_coverage_gap";

  todo: {
    description: string;
    expected_result?: string;
  };

  assigned_skills: string[];

  actual_execution: {
    used_skills: string[];
    critical_actions: string[];
    tools_used?: string[];
    global_objects_used?: string[];
  };

  result: {
    status: "success" | "partial" | "failure" | "unknown";
    report_summary?: string;
    supervisor_accepted?: boolean;
    user_feedback?: string;
  };

  gap_signal: {
    no_skill_assigned: boolean;
    assigned_skill_unused: boolean;
    success_without_skill: boolean;
    repeated_pattern?: boolean | "unknown";
  };

  target_consumer: "candidate_skill_miner";

  source: SignalSource;

  evidence: Evidence[];

  confidence: number;
};
```

消费边界：

```text
SkillCoverageGapSignal
    ↓
Candidate Skill Miner LLM
    ↓
重复性 / 稳定触发条件 / 稳定执行路径判断
    ↓
Candidate Skill Case / Skill Draft / Reject
```

Stage-1 不应该因为发现了 SkillCoverageGapSignal 就直接生成 Skill Draft。

---

# 11. Signal 通用结构

底层统一存储结构建议如下。

```typescript
type AttentionSignal = {
  id: string;

  signal_type:
    | "event"
    | "object_observation"
    | "relationship"
    | "skill_coverage_gap";

  lifecycle_status:
    | "pending_stage2"
    | "watching"
    | "consumed"
    | "converted"
    | "dropped"
    | "expired";

  payload:
    | EventSignal
    | ObjectObservationSignal
    | RelationshipSignal
    | SkillCoverageGapSignal;

  target_consumer:
    | "memory_stage2"
    | "candidate_skill_miner";

  source: SignalSource;

  evidence: Evidence[];

  extraction: {
    extractor_version: string;
    prompt_version: string;
    model_name?: string;
    extracted_at: string;
    extraction_window_id: string;
  };

  quality: {
    confidence: number;
    ambiguity_level: "low" | "medium" | "high";
    private_value_score?: number;
    user_relevance_score?: number;
    noise_risk_score?: number;
  };

  stage2_hints: Stage2Preparation;

  created_at: string;
  updated_at: string;
  expires_at?: string;
};
```

---

# 12. Source 与 Evidence

## 12.1 Source

每条 Signal 必须可追溯到原始 Session History。

```typescript
type SignalSource = {
  owner_id: string;

  agent_id: string;

  agent_scope_id: string;

  user_id: string;

  session_id: string;

  round_refs: RoundEntryRef[];

  window_start: string;
  window_end: string;

  source_type: "session_history";
};
```

其中 `agent_scope_id` 用于隔离同一 owner 下不同 Agent 的 Memory / Self-Improve 状态，通常可由 Agent RootFS identity 或运行时路径推导。

```typescript
type RoundEntryRef = {
  round_index: number;

  entry_seq: number;

  entry_kind?: "message" | "step" | "event";

  llm_call?: number;
};
```

---

## 12.2 Evidence

每条 Signal 必须包含证据。

```typescript
type Evidence = {
  round_index: number;

  entry_seq: number;

  entry_kind: "message" | "step" | "event";

  role: "user" | "assistant" | "tool" | "system";

  text_excerpt: string;

  start_offset?: number;
  end_offset?: number;

  created_at?: string;
};
```

要求：

```text
1. Evidence 必须来自真实 Session History。
2. 不允许凭空生成没有证据的 Signal。
3. 如果模型只是推测，必须降低 confidence。
4. 高价值 Signal 必须至少有一条明确 Evidence。
5. Evidence 必须能通过 `(session_id, round_index, entry_seq)` 回读原始 entry。
```

---

# 13. Stage-2 / Downstream Preparation 字段

Stage-1 必须为下游消费准备以下字段。

对 Memory Graph Signal，下游是 Memory Stage-2。

对 SkillCoverageGapSignal，下游是 Candidate Skill Miner LLM，其中 `suggested_action` 应使用：

```text
route_to_skill_candidate_miner
```

```typescript
type Stage2Preparation = {
  suggested_merge_key?: string;

  canonicalization_candidates?: CanonicalizationCandidate[];

  possible_memory_path?: string[];

  suggested_initial_attention_weight?: number;

  suggested_action:
    | "consider_event"
    | "consider_object"
    | "consider_relationship"
    | "route_to_skill_candidate_miner"
    | "consider_alias"
    | "watch"
    | "drop_if_unreinforced"
    | "unknown";

  retention_hint:
    | "short_lived"
    | "watch_72h"
    | "likely_promotable"
    | "requires_more_evidence";

  privacy_scope:
    | "user_private"
    | "public"
    | "mixed"
    | "unknown";

  recall_candidate_hint: boolean;
};
```

---

## 13.1 Canonicalization Candidate

用于支持 Stage-2 做对象归一化。

```typescript
type CanonicalizationCandidate = {
  mention_text: string;

  candidate_id?: string;

  candidate_source:
    | "contact_manager"
    | "memory_graph"
    | "session_context"
    | "llm_inference"
    | "unknown";

  confidence: number;

  reason?: string;
};
```

示例：

```json
{
  "mention_text": "Lucy",
  "candidate_id": "did:bnms:xxxx",
  "candidate_source": "contact_manager",
  "confidence": 0.96,
  "reason": "Contact Manager 中存在用户手工维护的 Lucy 联系人。"
}
```

如果只是上下文推理：

```json
{
  "mention_text": "Lucy",
  "candidate_source": "session_context",
  "confidence": 0.58,
  "reason": "上下文中 Lucy 与某工作邮箱共同出现，但缺少显式确认。"
}
```

---

# 14. Signal 生命周期

## 14.1 状态机

```text
created
   ↓
pending_stage2
   ↓
 ┌───────────────┬───────────────┬───────────────┐
 │               │               │               │
converted       dropped         watching        expired
 │                               │
 consumed                         ↓
                                  pending_stage2
```

---

## 14.2 状态说明

### pending_stage2

Stage-1 新生成的 Signal 默认状态。

```text
可被 target_consumer 指向的下游消费
不可被 Recall 系统召回
```

---

### watching

下游 consumer 认为该 Signal 有潜在价值，但当前证据不足。

例如：

```text
可能是跨天事件
可能是新对象首次出现
可能是关系初次暗示
可能需要后续强化
```

默认观察期：

```text
72 小时
```

---

### converted

该 Signal 已被下游 consumer 转化为：

```text
Attention State / Memory Graph Candidate
Memory Hint
Event Update
Object Update
Relationship Update
Alias Update
Candidate Skill Case
```

---

### dropped

下游 consumer 认为该 Signal 是噪音或价值不足。

---

### expired

Signal 超过观察期，且没有被强化。

---

## 14.3 Stage-1 状态限制

Stage-1 只能创建：

```text
pending_stage2
```

Stage-1 不应该直接设置：

```text
converted
dropped
consumed
```

这些由 `target_consumer` 指向的下游决定。

---

# 15. LLM Tool / API 设计

## 15.1 不推荐接口

不推荐只暴露一个抽象工具：

```typescript
AddAttentionSignal(signal: string)
```

原因：

```text
1. 对 LLM 不语义化
2. 无法引导模型区分 Event / Object / Relationship
3. 容易产生低质量泛化文本
4. 不利于 Stage-2 消费
```

---

## 15.2 推荐接口

### DiscoverEvent

```typescript
DiscoverEvent(input: {
  title: string;
  phase: EventPhase;
  description?: string;
  time_info?: {
    start_time?: string;
    end_time?: string;
    time_range_text?: string;
    is_time_precise: boolean;
  };
  participants?: EntityMention[];
  evidence: Evidence[];
  confidence: number;
  stage2_hints?: Stage2Preparation;
}): AttentionSignal;
```

---

### DiscoverObjectObservation

```typescript
DiscoverObjectObservation(input: {
  object: EntityMention;
  observation: string;
  observation_type?: ObservationType;
  evidence: Evidence[];
  confidence: number;
  stage2_hints?: Stage2Preparation;
}): AttentionSignal;
```

---

### DiscoverRelationship

```typescript
DiscoverRelationship(input: {
  subject: EntityMention;
  predicate: string;
  object: EntityMention;
  relation_type?: RelationshipType;
  attitude?: RelationshipAttitude;
  temporal_context?: {
    event_title?: string;
    time_range_text?: string;
    is_time_bound: boolean;
  };
  evidence: Evidence[];
  confidence: number;
  stage2_hints?: Stage2Preparation;
}): AttentionSignal;
```

---

# 16. Prompt 需求

Stage-1 Extractor Prompt 必须明确告诉 LLM：

```text
你不是在总结对话。
你不是在生成长期记忆。
你不是在判断最终是否应该记住。
你是在从 Session History 中发现结构化线索。
```

---

## 16.1 Prompt 核心规则

LLM 必须遵守：

```text
1. 默认只提取三类 Memory Signal：Event、ObjectObservation、Relationship。
2. 不要提取普通常识。
3. 不要提取没有用户私有价值的公共实体。
4. Event 必须具有生命周期或阶段。
5. Object 必须是具体对象，不是抽象类型。
6. Object 通常必须附带 Observation。
7. Relationship 必须表达对象之间的边。
8. Relationship 可以包含态度、偏好、情绪、职责、拥有、参与等。
9. Signal 必须引用 Evidence。
10. 不确定时降低 confidence。
11. 不要直接生成 Memory。
12. 不要直接更新用户 Profile。
13. 不要假设没有证据的信息。
14. 只有发现明确 Skill coverage gap 时，才提取 SkillCoverageGapSignal。
15. SkillCoverageGapSignal 必须路由给 Candidate Skill Miner，不要直接生成 Skill。
```

---

## 16.2 Event 判断规则

LLM 应提取：

```text
有明确阶段的项目
正在计划的事项
即将发生的会议
正在执行的任务
已经完成或取消的事项
```

LLM 不应把稳定属性当作 Event。

例如：

```text
“Lucy 是工程师”不是 Event。
“Lucy 下周参加发布会”是 Event。
```

---

## 16.3 Object 判断规则

LLM 应优先提取：

```text
用户私有上下文中的人
用户私有设备
用户项目
内部组件
具体文档
具体邮箱
具体账号
具体组织关系
```

LLM 不应因为公共实体被提到就提取。

例如：

```text
“我去 Costco 买东西”不一定需要提取 Costco。
“我更喜欢 Costco，不喜欢 World Market”应该提取关系。
```

---

## 16.4 Relationship 判断规则

LLM 应提取：

```text
用户喜欢 A
用户不喜欢 B
Bob 对 Alice 不满意
Lucy 负责某项目
某邮箱属于 Lucy
用户把某文档发给某人
某对象参与某事件
```

Relationship 中的态度应该保留。

---

## 16.5 Skill Coverage Gap 判断规则

LLM 只有在以下条件同时较强时，才应提取 SkillCoverageGapSignal：

```text
Plan / TODO 明确
现有 Skill 未覆盖，或分配了 Skill 但实际未使用
Do 阶段成功完成或被上级接受
成功路径主要来自临时探索、通用推理、工具组合或 Global Object 探索
```

LLM 不应提取：

```text
普通成功任务
没有明确 TODO 的聊天
只是在讨论 Skill 设计的抽象观点
已有 Skill 正常发挥作用的任务
失败但没有明确重复错误路径的任务
```

SkillCoverageGapSignal 的输出目标是：

```text
Candidate Skill Miner LLM
```

而不是：

```text
Memory Stage-2
Skill Draft Generator
Skill Update Flow
```

---

# 17. 扫描流程

## 17.1 主流程

```text
1. Scheduler 创建扫描窗口
2. History Window Reader 查询窗口内未处理 Session History round entries
3. 对每个 Session 构造 Stage-1 Extraction Input
4. 调用 LLM Extractor
5. LLM 使用 DiscoverEvent / DiscoverObjectObservation / DiscoverRelationship / DiscoverSkillCoverageGap
6. Signal Validator 校验结果
7. Signal Store 写入 Attention Signal
8. Scan Cursor Manager 推进游标
9. Signal 按 target_consumer 进入 Memory Stage-2 或 Skill Signal pending 队列
```

---

## 17.2 伪代码

```typescript
async function runStage1Extraction(window: TimeWindow) {
  const sessions = await historyReader.getSessionsWithUnscannedEntries(window);

  for (const session of sessions) {
    const entries = await historyReader.getUnscannedEntries(session.id, window);

    if (entries.length === 0) {
      continue;
    }

    const extractionResult = await signalExtractor.extract({
      owner_id: session.owner_id,
      agent_id: session.agent_id,
      agent_scope_id: session.agent_scope_id,
      session_id: session.id,
      window,
      entries
    });

    const validSignals = signalValidator.validate(extractionResult.signals);

    await signalStore.insertMany(validSignals);

    await scanCursorManager.markScanned({
      session_id: session.id,
      last_round_index: entries[entries.length - 1].round_index,
      last_entry_seq: entries[entries.length - 1].entry_seq,
      window
    });
  }
}
```

---

# 18. Signal Validator

写入前必须校验。

## 18.1 通用校验

```text
1. signal_type 必须合法
2. evidence 不得为空
3. confidence 必须在 0 到 1 之间
4. source.agent_scope_id 必须存在
5. source.session_id 必须存在
6. source.round_refs 必须存在
7. evidence 必须包含 round_index 和 entry_seq
8. extracted_at 必须存在
9. lifecycle_status 必须为 pending_stage2
```

---

## 18.2 Event 校验

```text
1. title 不得为空
2. phase 必须合法
3. 不能只包含稳定属性
4. 如果没有时间信息，必须说明 is_time_precise = false
```

---

## 18.3 Object Observation 校验

```text
1. object.mention_text 不得为空
2. observation 不得为空
3. object 不应只是普通类型词
4. 如果是公共实体，必须存在用户私有观察或关系
```

---

## 18.4 Relationship 校验

```text
1. subject 不得为空
2. predicate 不得为空
3. object 不得为空
4. subject 与 object 不应完全相同
5. attitude 如果存在，必须标明 polarity
```

---

# 19. 去重与幂等

## 19.1 Stage-1 只做轻量去重

Stage-1 不负责跨 Session 的语义级合并。

但必须避免同一次扫描重复写入完全相同 Signal。

---

## 19.2 幂等 Key

建议生成：

```text
idempotency_key =
hash(
  agent_scope_id,
  user_id,
  session_id,
  signal_type,
  normalized_evidence_round_entry_refs,
  payload_core_text
)
```

---

## 19.3 不应过度合并

Stage-1 不应把跨 Session、跨时间窗口的相似 Signal 强行合并。

这些应交给 Stage-2。

例如：

```text
上午用户提到笔记本电脑很慢
下午用户再次提到笔记本电脑很慢
```

Stage-1 可以产生两条 Signal。

Stage-2 再判断它们是否强化同一个 Object Observation。

---

# 20. 存储设计

## 20.1 attention_signals 表

```sql
CREATE TABLE attention_signals (
  id TEXT PRIMARY KEY,

  owner_id TEXT NOT NULL,

  agent_id TEXT NOT NULL,

  agent_scope_id TEXT NOT NULL,

  user_id TEXT NOT NULL,

  signal_type TEXT NOT NULL,

  lifecycle_status TEXT NOT NULL,

  payload_json JSON NOT NULL,

  source_json JSON NOT NULL,

  evidence_json JSON NOT NULL,

  extraction_window_id TEXT NOT NULL,

  extractor_version TEXT NOT NULL,

  prompt_version TEXT NOT NULL,

  confidence REAL NOT NULL,

  ambiguity_level TEXT,

  private_value_score REAL,

  user_relevance_score REAL,

  noise_risk_score REAL,

  suggested_merge_key TEXT,

  suggested_initial_attention_weight REAL,

  retention_hint TEXT,

  recall_candidate_hint BOOLEAN,

  idempotency_key TEXT UNIQUE,

  created_at TIMESTAMP NOT NULL,

  updated_at TIMESTAMP NOT NULL,

  expires_at TIMESTAMP
);
```

推荐索引：

```sql
CREATE INDEX idx_attention_signals_stage2
  ON attention_signals(agent_scope_id, lifecycle_status, created_at);

CREATE INDEX idx_attention_signals_source
  ON attention_signals(agent_scope_id, user_id, extraction_window_id);
```

---

## 20.2 scan_checkpoints 表

```sql
CREATE TABLE scan_checkpoints (
  id TEXT PRIMARY KEY,

  owner_id TEXT NOT NULL,

  agent_id TEXT NOT NULL,

  agent_scope_id TEXT NOT NULL,

  user_id TEXT NOT NULL,

  session_id TEXT NOT NULL,

  last_scanned_round_index INTEGER,

  last_scanned_entry_seq INTEGER,

  last_scanned_at TIMESTAMP,

  scan_window_start TIMESTAMP,

  scan_window_end TIMESTAMP,

  status TEXT NOT NULL,

  updated_at TIMESTAMP NOT NULL
);
```

---

## 20.3 extraction_windows 表

```sql
CREATE TABLE extraction_windows (
  id TEXT PRIMARY KEY,

  owner_id TEXT NOT NULL,

  agent_id TEXT NOT NULL,

  agent_scope_id TEXT NOT NULL,

  user_id TEXT NOT NULL,

  window_start TIMESTAMP NOT NULL,

  window_end TIMESTAMP NOT NULL,

  status TEXT NOT NULL,

  created_at TIMESTAMP NOT NULL,

  completed_at TIMESTAMP
);
```

---

# 21. Stage-2 消费准备

Stage-1 生成的每条 Memory Graph Signal 必须能被 Stage-2 直接消费。

这里的 Stage-2 特指：

```text
Memory Stage-2: Signal Consumption / Attention State Merge
```

对于 `SkillCoverageGapSignal`，下游是：

```text
Candidate Skill Miner LLM
```

它复用 source / evidence / confidence / lifecycle 等通用字段，但不要求回答“是否进入 Memory Graph / Recallable Memory”。

Stage-2 消费时需要能够回答：

```text
1. 这是什么类型的线索？
2. 它来自哪里？
3. 有什么证据？
4. 它是否和用户强相关？
5. 它可能对应哪个已有对象？
6. 它可能和哪些旧 Signal 合并？
7. 它是否值得进入 Attention State / Memory Graph Candidate？
8. 它是否只是噪音？
9. 它是否应该进入 72 小时观察期？
10. 它是否有机会成为可召回 Memory？
```

因此 Stage-1 不能只存一句自然语言描述。

必须存结构化字段。

---

# 22. 与 Attention State / Memory Graph 的边界

Stage-1 生成的 Attention Signal 不等于 Attention State，也不等于 Memory Graph 中的对象或关系。

```text
Attention Signal:
短生命周期线索，默认不可召回。

Attention State / Memory Graph Candidate:
经过 Stage-2 消费后形成的被观察对象、事件、关系候选。

Recallable Memory:
进一步突破权重阈值后，可被自动召回系统使用。
```

SkillCoverageGapSignal 还有额外边界：

```text
SkillCoverageGapSignal:
Skill Self-Improve 的中间线索，默认不可召回。

Candidate Skill Case:
经过 Candidate Skill Miner LLM 消费后形成的新 Skill 候选 case。

Runtime Skill:
经过 Skill Draft / Review / Install / Validation 后才进入运行时 Skill 系统。
```

推荐分层：

```text
Session History
    ↓
Attention Signal
    ├── Memory Stage-2
    │       ↓
    │   Attention State / Memory Graph Candidate
    │       ↓
    │   Recallable Memory
    └── Candidate Skill Miner LLM
            ↓
        Candidate Skill Case / Skill Draft / Reject
```

---

# 23. 权重字段预留

Stage-1 不计算最终权重，但可以给 Stage-2 一个初始建议。

```typescript
suggested_initial_attention_weight?: number;
```

建议范围：

```text
0.0 ~ 1.0
```

参考因素：

```text
1. 是否用户主动表达
2. 是否和用户私有对象有关
3. 是否多次出现
4. 是否包含明确态度
5. 是否涉及正在进行的事件
6. 是否涉及联系人、设备、项目、任务等长期对象
```

示例：

```json
{
  "suggested_initial_attention_weight": 0.72,
  "retention_hint": "likely_promotable",
  "recall_candidate_hint": false
}
```

注意：

```text
recall_candidate_hint = false
```

不表示永远不能召回，只表示 Stage-1 不直接让它进入召回。

---

# 24. 72 小时观察期支持

Stage-1 不决定是否进入观察期，但必须支持 Stage-2 设置：

```text
watching
expires_at = now + 72h
```

Stage-1 可提供建议：

```json
{
  "retention_hint": "watch_72h",
  "suggested_action": "watch"
}
```

适合进入观察期的 Signal：

```text
1. 新对象首次出现，但上下文不足
2. 新事件首次出现，但阶段不清楚
3. 关系有暗示，但证据不够
4. 可能跨 Session / 跨天继续展开
5. 可能与用户长期偏好有关
```

---

# 25. Recall 隔离要求

Stage-1 产生的 Signal 必须与自动 Recall 系统隔离。

要求：

```text
1. 默认不进入 Recall Index
2. 默认不参与 Memory Search
3. 默认不进入 Prompt Context
4. 默认不作为用户长期 Profile
5. 只有 Stage-2 转换后的结果才可能进入 Recallable Memory
```

这是硬性边界。

否则系统会出现：

```text
低质量线索污染长期记忆
临时噪音被反复召回
过期信息干扰 Agent 行为
```

---

# 26. 配置项

```typescript
type AttentionSignalConfig = {
  extraction_window_minutes: number;

  max_entries_per_extraction: number;

  max_signals_per_session_window: number;

  min_confidence_to_store: number;

  default_signal_ttl_hours: number;

  default_watching_ttl_hours: number;

  enable_public_entity_filter: boolean;

  enable_stage1_light_dedup: boolean;

  enable_canonicalization_candidates: boolean;

  enable_skill_coverage_gap_signal: boolean;

  extractor_model?: string;

  prompt_version: string;
};
```

推荐默认值：

```json
{
  "extraction_window_minutes": 240,
  "max_entries_per_extraction": 200,
  "max_signals_per_session_window": 50,
  "min_confidence_to_store": 0.55,
  "default_signal_ttl_hours": 72,
  "default_watching_ttl_hours": 72,
  "enable_public_entity_filter": true,
  "enable_stage1_light_dedup": true,
  "enable_canonicalization_candidates": true,
  "enable_skill_coverage_gap_signal": false
}
```

---

# 27. 日志与可观测性

组件必须记录：

```text
1. 每个窗口处理了多少 Session
2. 每个 Session 读取了多少 History Entry
3. 生成了多少 Event Signal
4. 生成了多少 Object Observation Signal
5. 生成了多少 Relationship Signal
6. 生成了多少 Skill Coverage Gap Signal
7. 被 Validator 拒绝了多少 Signal
8. 平均 confidence
9. 每个窗口的 Signal 数量
10. 重复 Signal 数量
11. Memory Stage-2 后续消费率
12. Skill Signal 后续消费率
```

---

## 27.1 指标示例

```text
attention_signal.stage1.windows_processed
attention_signal.stage1.sessions_processed
attention_signal.stage1.entries_scanned
attention_signal.stage1.signals_created
attention_signal.stage1.event_signals_created
attention_signal.stage1.object_signals_created
attention_signal.stage1.relationship_signals_created
attention_signal.stage1.skill_gap_signals_created
attention_signal.stage1.signals_rejected
attention_signal.stage1.avg_confidence
attention_signal.stage1.duplicate_signals_skipped
attention_signal.stage2.pending_count
attention_signal.skill_candidate_miner.pending_count
```

---

# 28. 安全与隐私要求

Attention Signal 可能包含非常私有的信息。

例如：

```text
联系人关系
邮箱
家庭成员
工作项目
用户偏好
用户设备问题
人与人之间的不满
财务文档流转
```

因此要求：

```text
1. 所有 Signal 必须按 agent_scope_id + user_id 隔离。
2. Evidence 不应暴露给无权限组件。
3. Signal 不应进入公共日志。
4. 包含联系人、邮箱、身份信息的 Signal 应标记 privacy_scope = user_private。
5. Stage-1 不应将私有 Signal 发送给非授权外部服务。
6. 删除用户数据时必须删除 Signal、Evidence、Checkpoint。
```

---

# 29. 示例：Stage-1 输出节选

输入 Session History：

```text
用户：我觉得 Attention Signal 这个词对 LLM 不够语义化。
用户：第一阶段应该发现事件、对象和关系。
用户：比如 Bob 对 Alice 不太满意，这就是一种关系。
用户：我的笔记本电脑最近很慢。
```

输出 Signals：

> 为突出 payload 与 quality，下面示例省略 `id`、`source`、`evidence`、`extraction`、`created_at` 等系统字段。真实写入必须包含这些字段，并能通过 `(session_id, round_index, entry_seq)` 回读原始 Session History。

```json
[
  {
    "signal_type": "event",
    "lifecycle_status": "pending_stage2",
    "payload": {
      "title": "Agent Memory Attention Signal 第一阶段设计讨论",
      "phase": "active",
      "description": "用户正在设计 Self Improve Stage-1 如何从 Session History 中发现事件、对象和关系线索。",
      "time_info": {
        "is_time_precise": false
      }
    },
    "quality": {
      "confidence": 0.93,
      "ambiguity_level": "low",
      "private_value_score": 0.84,
      "user_relevance_score": 0.91
    },
    "stage2_hints": {
      "suggested_action": "consider_event",
      "retention_hint": "likely_promotable",
      "recall_candidate_hint": false
    }
  },
  {
    "signal_type": "relationship",
    "lifecycle_status": "pending_stage2",
    "payload": {
      "subject": {
        "mention_text": "Bob",
        "entity_type": "person",
        "is_user_private_entity": true
      },
      "predicate": "is dissatisfied with",
      "object": {
        "mention_text": "Alice",
        "entity_type": "person",
        "is_user_private_entity": true
      },
      "relation_type": "attitude_towards",
      "attitude": {
        "polarity": "negative",
        "sentiment_text": "不太满意",
        "strength": "medium",
        "is_explicit": true
      }
    },
    "quality": {
      "confidence": 0.88,
      "ambiguity_level": "medium",
      "private_value_score": 0.82,
      "user_relevance_score": 0.67
    },
    "stage2_hints": {
      "suggested_action": "consider_relationship",
      "retention_hint": "watch_72h",
      "recall_candidate_hint": false
    }
  },
  {
    "signal_type": "object_observation",
    "lifecycle_status": "pending_stage2",
    "payload": {
      "object": {
        "mention_text": "用户的笔记本电脑",
        "entity_type": "device",
        "is_user_private_entity": true
      },
      "observation": "用户认为自己的笔记本电脑最近很慢。",
      "observation_type": "problem"
    },
    "quality": {
      "confidence": 0.95,
      "ambiguity_level": "low",
      "private_value_score": 0.9,
      "user_relevance_score": 0.88
    },
    "stage2_hints": {
      "suggested_action": "consider_object",
      "retention_hint": "likely_promotable",
      "recall_candidate_hint": false
    }
  }
]
```

---

# 30. 验收标准

## 30.1 功能验收

组件必须满足：

```text
1. 能按时间窗口扫描未处理 Session History。
2. 能维护每个 Session 的 scan checkpoint。
3. 能生成 Event Signal。
4. 能生成 Object Observation Signal。
5. 能生成 Relationship Signal。
6. 启用 skill gap 扩展时，能生成 Skill Coverage Gap Signal。
7. 每条 Signal 都有 source 和 evidence。
8. Signal 默认状态为 pending_stage2。
9. Signal 不进入 Recall Index。
10. Signal 可按 target_consumer 路由到 Memory Stage-2 或 Skill Signal consumer。
11. 重复扫描不会重复生成相同 Signal。
```

---

## 30.2 语义验收

组件输出必须满足：

```text
1. 不把稳定属性误判为 Event。
2. 不把普通类型词误判为 Object。
3. 不因为公共实体出现就生成 Object Signal。
4. 能识别对象上的 Observation。
5. 能识别带态度的 Relationship。
6. 能保留模糊、不确定、推测性信息的 confidence。
7. 能区分显式事实和上下文推断。
8. 不把普通成功任务误判为 Skill Coverage Gap。
9. 不把 Skill Coverage Gap 直接改写为 Candidate Skill。
```

---

## 30.3 Stage-2 准备验收

每条 Memory Graph Signal 必须能支持 Memory Stage-2 判断：

```text
1. 是否要合并到已有对象。
2. 是否要创建新对象。
3. 是否要创建新事件。
4. 是否要创建或强化关系。
5. 是否进入 72 小时观察期。
6. 是否作为噪音丢弃。
7. 是否产生 Memory Hint。
8. 是否影响 Attention Weight。
```

每条 SkillCoverageGapSignal 必须能支持 Candidate Skill Miner 判断：

```text
1. 是否是明确 Skill coverage gap。
2. 是否只是一次普通成功任务。
3. 是否需要等待更多重复样本。
4. 是否进入 Candidate Skill Case。
5. 是否明确 Reject。
```

---

# 31. 后续扩展

本版本的 Memory Graph 核心只支持三类 Signal。

未来可以扩展：

```text
1. Task Signal
2. Preference Signal
3. Conflict Signal
4. Goal Signal
5. Habit Signal
6. Schedule Signal
7. Risk Signal
```

但 v0.1 不建议过早扩展。

SkillCoverageGapSignal 是为了支持 Skill Self-Improve 的扩展通道，不属于 Memory Graph 核心类型扩张。它不改变 Event / ObjectObservation / Relationship 这三类核心 Memory Signal 的设计。

当前三类已经覆盖核心图结构：

```text
Event:
时间轴

Object Observation:
节点 + 注释

Relationship:
节点之间的边
```

---

# 32. Stage-1 / 下游 Consumer 生命周期对接协议（v0.1 补充）

> 本节补齐 §14（生命周期）与 §24（72 小时观察期）之间一直悬空的核心决策：**下游 consumer 消费一条 Signal 之后，这条 Signal 默认怎么处置。**
>
> 早期讨论没有结论，导致实现里只落了状态词汇表，没有落状态跃迁——所有 Signal 永久停在 `pending_stage2`，`expires_at` 记录但无人执行。本节给出决策与对接契约。注意：跃迁的 **触发判断（promote / merge / drop / watch 决策本身）属于下游 consumer 协议**，本节只规定 **Stage-1 Store 必须暴露的状态机能力**，使 Memory Stage-2 或 Candidate Skill Miner 能够实现下述语义。

---

## 32.1 两个候选模型

```text
模型 A（resolve-by-default，消费即出池）
  下游 consumer 每轮必须把看到的每条 pending 信号 resolve 掉：
  强信号 → converted/consumed（离开 pending 池），
  弱信号 → watching（留 72h 等强化），
  噪音   → dropped。
  稳态 pending 池 = 新信号 + 一小撮 watching。

模型 B（keep-all，仅靠 timeout 淘汰）
  下游 consumer 读完不改状态，信号一直留到 72h 过期或手工删除。
  pending 池 = 过去 72h 的全部信号，每轮重复处理。
```

---

## 32.2 决策：采用模型 A（resolve-by-default）

理由：

```text
1. 不变量编码为持久状态，而非每轮重新推导。
   "提升幂等"（同一现实事实最多变成一个 hint）是核心不变量。
   模型 A 用 converted/consumed 跃迁把它编码进信号生命周期，
   信号一旦出池物理上无法二次提升；
   模型 B 只能靠每轮拿信号和已有 hint 比对来重新推导，脆弱且会漂移。

2. 成本规模。Memory Stage-2 和 Candidate Skill Miner 都可能是 LLM 推理步，成本 ∝ 每轮要看的信号数 × 判断难度。
   模型 A 每条信号一生只判一次，每轮成本 ∝ 增量（平）；
   模型 B 每轮重看全量、且需把已有 hint 一起喂入去重，
   成本 ∝ 累积信号数 × 已有 hint 数（复利增长）。

3. 模型 B 仅有的两个优点都可被模型 A 吸收：
   - 崩溃容错：把 convert 跃迁与 hint 落库放进同一事务，
     或"先持久化 hint、再 mark converted"，加 hint 侧幂等键兜底即可。
   - 弱信号强化可见性：由 watching 桶承担（见 32.3）。
```

---

## 32.3 默认行为是"必须 resolve"，不是"必须消费"

每轮下游 consumer 必须把每条 pending 信号 resolve 到一个终态或 watching：

```text
强证据 / 可直接提升   → converted   （出池，写 hint 回指）
折叠进已有 hint       → consumed    （出池，写 merge 回指）
噪音 / 价值不足        → dropped     （出池）
证据不足 / 有歧义      → watching    （留 72h，可被强化后重新评估）
```

关键约束：**`watching` 是低质量信号的自动归宿，由信号质量驱动，而不是"手工保留"。**

```text
自动进入 watching 的典型条件：
  quality.confidence 低 / ambiguity_level = high
  stage2_hints.retention_hint = watch_72h
  stage2_hints.suggested_action = watch
  §24 列出的"首次出现、跨天展开、暗示性关系"等情形
```

这样既保住模型 A 的小工作集，又把模型 B 唯一值得要的"复现强化"能力补回来——弱信号留在 watching 桶里，72h 内被新证据强化则重新进 pending 评估，否则 `expired`。

---

## 32.4 Store 必须暴露的状态跃迁 API

Stage-1 §14.3 限制"只能创建 pending_stage2"针对的是 **create** 路径；**跃迁是独立的 update 路径**，供下游 consumer 调用。Store 至少须提供：

```typescript
mark_converted(id, hint_ref: string)                 // → converted
mark_consumed(id, merged_into_hint_ref: string)      // → consumed
mark_watching(id)                                     // → watching, 重算 expires_at = now + watching_ttl
mark_dropped(id, reason?: string)                     // → dropped
```

要求：

```text
1. 所有跃迁 bump updated_at。
2. 合法跃迁仅允许 pending_stage2 → {converted, consumed, dropped, watching}
   以及 watching → {pending_stage2, converted, consumed, dropped, expired}。
   非法跃迁返回 InvalidInput。
3. converted/consumed 跃迁与下游产物落库的原子性由下游 consumer 保证
   （同事务，或 hint 先持久化再跃迁 + hint 侧幂等键兜底）。
4. create 用的 validate_signal "只能 pending_stage2" 约束与 update 路径分离，
   不得相互绕过。
```

---

## 32.5 hint 回指与去重支持

为缓解下游 consumer "既要判信号重复、又要判生成产物重复"的双重压力，Store 须提供两项：

```text
1. 回指字段：converted/consumed 跃迁必须持久化 promoted_to_hint_ref，
   使信号能反查"我已变成哪个 hint / 折叠进哪个 hint"。

2. merge_key 可查：suggested_merge_key 从纯建议字段提升为可查询列 + 索引，
   让下游 consumer 能 SELECT ... WHERE suggested_merge_key = ? 快速判
   "这个语义是否已有 hint"，而非全表 JSON 扫或重新 LLM 决策。
```

对应 §20.1 表结构增量：

```sql
ALTER TABLE attention_signals ADD COLUMN promoted_to_hint_ref TEXT;

CREATE INDEX idx_attention_signals_merge_key
  ON attention_signals(agent_scope_id, suggested_merge_key);
```

---

## 32.6 过期执行（与 §24 配套）

`expires_at` 必须真正生效，否则模型 A 退化成"只进不出"：

```text
1. list_pending_stage2 查询加过期过滤：
   WHERE lifecycle_status = 'pending_stage2'
     AND (expires_at IS NULL OR expires_at > now)
   （惰性过滤，立刻止血）

2. watching 使用 default_watching_ttl_hours 重算 expires_at，
   不复用 signal_ttl_hours。

3. 后台 sweep 兜底：把到期的 pending/watching 翻成 expired，
   回收工作集（参考"定时 sweep 兜底"原则）。
```

---

## 32.7 当前实现差距（待落地清单）

截至 v0.1 实现（`agent_attention_signal.rs`），相对本节的缺口：

```text
[ ] 无任何状态跃迁 API（mark_converted/consumed/watching/dropped 全缺）
[ ] attention_signals 表无 UPDATE 路径，信号永久停在 pending_stage2
[ ] expires_at 已写入但无任何查询/ sweep 读取，过期信号被无限期返回
[ ] watching 独立 TTL（default_watching_ttl_hours）从未被应用
[ ] 无 promoted_to_hint_ref 回指字段
[ ] suggested_merge_key 未提升为列、未建索引
[ ] watching 信号的"被强化后重新评估"回路缺失
[ ] AttentionSignalConfig 的 min_confidence_to_store 等策略旋钮未被 Store 消费
```

落地优先级：**先补 32.4 跃迁 API + 32.6 过期过滤**（直接解决信号密度与 Stage-2 决策压力），再补 32.5 hint 回指/索引（解决 hint 去重），最后补 watching 再评估回路。

---

# 33. 总结

Agent Memory Attention Signal 组件的第一阶段职责是：

```text
把 Session History 翻译成结构化线索。
```

它不负责直接记忆，也不负责召回。

核心 Memory Graph 输出只有三类：

```text
Event Signal
Object Observation Signal
Relationship Signal
```

为支持 Skill Self-Improve，Stage-1 可以额外输出：

```text
Skill Coverage Gap Signal
```

但这类 Signal 的下游是独立 Candidate Skill Miner LLM，不是 Memory Stage-2。

这些 Signal 是短生命周期中间产物。

它们必须携带：

```text
来源
证据
置信度
时间窗口
语义结构
下游消费提示
```

Memory Stage-2 再负责：

```text
消费 Signal
合并 Signal
归一化对象
更新 Attention State / Memory Graph Candidate
调整权重
决定是否进入 Recallable Memory
```

整体分层如下：

```text
Session History
    ↓
Stage-1: Attention Signal Extraction
    ↓
Pending Attention Signals
    ├── Memory Stage-2: Signal Consumption / Attention State Merge
    │       ↓
    │   Attention State / Memory Graph Candidate
    │       ↓
    │   Recallable Memory
    └── Candidate Skill Miner LLM
            ↓
        Candidate Skill Case / Skill Draft / Reject
```

这个组件的关键设计原则是：

> Stage-1 要多发现、轻判断、强结构化；下游 consumer 再合并、淘汰、归一化、提升或生成候选 case。

对于 SkillCoverageGapSignal，还需要补充一条边界：

> Stage-1 只发现 Skill coverage gap；是否形成新 Skill 候选，由独立 Skill Self-Improve 流程判断。
