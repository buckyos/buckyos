# OpenDAN Agent Runtime（BuckyOS）设计文档（合并版 v3）

> 本文将旧版设计与 **MsgTunnle 单通道事实源 + 内部 Session 投影架构改造（方案三）** 合并为一个一致的新版本。  
> 若存在歧义或冲突，以方案三为准（尤其是 MsgTunnle 事实源、Route→Link 投影、Expectation/Claim 仲裁等核心机制）。
>
> **本次改造核心共识：**
> 1. **MsgTunnle 是事实源，不删不迁移**
> 2. **Session 是内部工作容器，不是 UI 会话**
> 3. **Route 的输出是 link（投影），不是 session_id（归属）**
> 4. **读可以多路，写必须单路：授权/动作用 EXCLUSIVE + Claim**

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
  * **MsgTunnle**：两实体之间唯一通信通道（用户 ↔ Agent、Agent ↔ 设备…），保存所有消息历史，是**唯一事实源**。核心原则：**不可删除/不可从中迁移掉消息**。提供 message_id、时间戳、source_key 等。
  * OpenDAN：在 Agent Loop 的输入收集阶段从 MsgTunnle 拉取关心的 msg/event；必要时发送 msg（回复也 append 到 MsgTunnle）；SubAgent 内部消息也走同一套抽象（但不可外部寻址）。
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

## 4. 核心抽象：MsgTunnle / Session / Link / Workspace / Workshop

> 这一节统一运行时的"逻辑容器"与"交付空间"概念。
> **核心改造：把"消息的事实存储"与"Agent 干活的上下文/状态机"彻底解耦。**

### 4.0 改造动机与核心痛点

#### 单通道是用户体验的必然

用户不愿意切会话、不理解 Session 切换，也不应该承担"选错会话白干"的风险。

#### 主动干活是状态机问题

授权/确认/重试/跨天恢复，都要求一个明确的"工作容器"（Session/Run/Workflow instance）来承载状态。

**因此我们需要：**

* UI 侧：单通道（MsgTunnle）稳定、自然
* 系统侧：多工作实例（internal Session）可恢复、可并行、可观测
* 两者之间：靠 Link 做投影，不靠迁移/删除


### 4.1 MsgTunnle（事实源 — 唯一消息存储）

* 两实体之间唯一通信通道（用户 ↔ Agent、Agent ↔ 设备…）
* 保存所有消息历史
* **不可删除/不可从中迁移掉消息（核心原则）**
* 提供 message_id、时间戳、source_key 等
* 用户在 UI 上只看到 MsgTunnle 的历史

#### 4.1.1 存储策略与生命周期管理

"不删不迁移"是逻辑层原则，但物理存储必须考虑长期可持续性：

* **冷热分层（推荐）**：近期消息（如 90 天内）保留在热存储（低延迟 DB/缓存）；历史消息自动归档到冷存储（对象存储/压缩归档），但仍可按 message_id 按需拉取。归档不改变逻辑语义——消息依然"在 MsgTunnle 中"，只是访问延迟不同。
* **本地读缓存**：Session generate_input 时通过 link 拉取 MsgTunnle 正文，若 MsgTunnle 在远端存储，应在 Worker 本地维护 LRU 缓存（按 message_id），避免每次 step 的网络 RTT 成为瓶颈。缓存可丢失、可重建，不影响正确性。
* **容量预警**：建议对 MsgTunnle 设置容量监控（每通道消息数、总存储量），在接近阈值时通知用户或触发自动摘要压缩（压缩后的摘要写入 SessionNote，原始消息保留但降级到冷存储）。

#### 4.1.2 隐私合规与数据删除

"不可删除"原则的适用边界需要明确：

* **GDPR/隐私合规场景**：若用户依据法律要求删除个人数据（right to erasure），MsgTunnle 必须支持**逻辑软删除**——消息标记为 `REDACTED`，正文替换为占位符（如 `[message redacted by user request]`），但 message_id、时间戳、link 关系保留。这样不破坏 session 的 link 引用链完整性，同时满足合规要求。
* **软删除对 Session 的影响**：Session 通过 link 拉取到 REDACTED 消息时，应将其视为"无内容的历史标记"，不编入 prompt，但不影响 session 状态机的正确推进。
* **审计日志**：所有软删除操作必须写入 Ledger（who/when/why），不可被 Agent 自主触发，只能由用户或系统管理员发起。

### 4.2 Session（内部工作容器 — 非 UI 会话）

* **Session** 是 Agent 内部"干活"的容器（PDCA loop / workflow run），UI 对用户只读或完全不展示。
* Session 也是 Runtime 中 Agent 执行的主要逻辑容器：
  若把一次 LLM 推理类比为一次"AI 时代的 CPU 调用"，则：
  * **AgentSession 类似传统 thread**：同一 session 内 LLM 调用总是顺序执行。
  * session 的最小执行粒度是 **behavior step**；每个 step 完成后保存 `session.state`。
  * 若 session 的执行不依赖外部环境，则给定 `behavior_name + step` 可在系统重启后继续运行，也可回退到上一个 step 重新执行。
* Session 保存：状态、worklog、workspace 绑定、expectation、skills、step 进度等。
* **Session 不再存"消息正文"**，只存引用 MsgTunnle message_id 的链接（Link）。

#### 4.2.1 Session 生命周期与 GC 策略

MsgTunnle 是永久的，但 Session 作为内部工作容器，需要明确的生命周期管理：

* **创建**：由 Dispatcher 在路由时按需创建（新任务创建新 session；inbox session 作为默认兜底始终存在）。
* **活跃→休眠**：Session 进入 `WAIT` 且超过 `idle_timeout`（建议默认 7 天）后，自动转为 `SLEEP`。
* **归档**：Session 处于 `SLEEP` 且超过 `archive_timeout`（建议默认 30 天）后，触发归档流程：
  * 生成 session summary（派生摘要），写入 SessionNote
  * 将 session 元数据（不含 link 详情）压缩存入冷存储
  * Link 记录保留（因为 link 是轻量的引用，存储成本低），但不再主动消费
  * session 状态标记为 `ARCHIVED`
* **销毁**：`ARCHIVED` session 超过 `retention_period`（建议默认 180 天）后，可选择销毁 session 元数据和 link 记录。MsgTunnle 中的消息不受影响。
* **复活**：已归档但未销毁的 session 可通过 relink 新消息重新激活（状态从 `ARCHIVED` → `READY`），此时从 SessionNote 的摘要恢复上下文，而非重新拉取全部历史 link。

### 4.3 Link（投影/附着 — 消息与 Session 的关联机制）

* Session 不存消息正文，只存"引用 MsgTunnle message_id 的链接"
* 一个消息可被多个 session link（允许多归属）
* 但对授权/副作用类必须支持独占策略（EXCLUSIVE）

推荐的 Link 数据结构：

```text
SessionMessageLink {
  link_id
  session_id
  msg_tunnle_message_id
  pack_id
  state: NEW | USED | ACKED
  policy: NORMAL | EXCLUSIVE
  reason: EXPECTATION_MATCH | ACTIVE_SESSION | INBOX_FALLBACK | ...
  created_at
}
```

可选：Session 内额外存"提炼后的派生摘要"，而不是原文副本：

```text
SessionNote {
  session_id
  derived_summary_md
  extracted_entities_json
  updated_at
}
```

这样做到：MsgTunnle 永远是事实源；Session 只是投影 + 状态；可重算、可撤销、可纠错。

### 4.4 Expectation（等待用户回复/授权）

* 某 session 进入"等待用户回复"的状态
* Expectation router 用来把后续用户回复匹配回正确 session
* **授权类 expectation 默认 EXCLUSIVE（独占）**

#### 4.4.1 Expectation 数据结构

```text
Expectation {
  expectation_id
  session_id
  type: AUTH | CONFIRM | INFO_REQUEST | GENERIC
  match_hint: string          # 用于辅助匹配的语义提示（如"用户应回复授权码"或"用户应确认是否继续"）
  policy: EXCLUSIVE | SHARED
  created_at
  timeout_ms                  # 超时后自动降级
  state: PENDING | MATCHED | EXPIRED | CANCELLED
}
```

#### 4.4.2 Expectation 冲突处理状态机

当用户回复到达时，Dispatcher 按以下流程处理：

```
用户回复到达
  │
  ├─ 匹配到 0 个 expectation → 走 Active Session / Inbox 路由（正常 link 流程）
  │
  ├─ 匹配到恰好 1 个 expectation → 直接 link 到该 session
  │   └─ 若该 expectation 是 EXCLUSIVE → 创建 EXCLUSIVE link
  │
  └─ 匹配到 ≥2 个 expectation → 进入冲突解决：
      │
      ├─ 优先级仲裁（自动）：
      │   若其中恰好 1 个是 AUTH 类型 → 优先匹配 AUTH（授权安全高于信息补充）
      │   若存在 timeout 即将到期的 → 优先匹配即将超时的（避免饿死）
      │
      ├─ 若自动仲裁无法决定 → 向用户发送澄清消息：
      │   Agent 通过 MsgTunnle 发送结构化澄清请求，列出冲突的 session 摘要，
      │   请用户指定回复目标。
      │   此时所有冲突 expectation 保持 PENDING，不消费用户原始回复。
      │   原始回复被 link 到一个临时的 clarification session（只读暂存）。
      │
      └─ 澄清超时处理：
          若用户在 clarification_timeout（建议 5 分钟）内未回复，
          则按"最近活跃 session 优先"降级匹配，并记录 worklog 标注为"自动降级"。
```

#### 4.4.3 Expectation 超时与取消

* 每个 expectation 有独立的 `timeout_ms`，超时后状态变为 `EXPIRED`，session 收到超时事件后由 behavior 决定下一步（重试/放弃/降级）。
* Session 主动取消 expectation（如任务被用户取消）时，状态变为 `CANCELLED`。
* 已 EXPIRED 或 CANCELLED 的 expectation 不再参与后续匹配。

### 4.5 Claim（行动仲裁）

* 多个 session 可能都"看见"同一消息（多 link）
* 但**外部副作用（发消息/发邮件/执行动作）必须单路提交**
* 通过 `claim(intent_id)` 确保只有一个 session 真正执行外部动作

> 一句话规则：**读可以多路，写必须单路；理解可并行，行动必须仲裁。**

#### 4.5.1 Claim 实现机制

```text
ClaimRecord {
  claim_id
  intent_id           # 标识"要做什么"（如 reply_to_msg#123, execute_payment#456）
  session_id           # 请求 claim 的 session
  state: ACQUIRED | RELEASED | REJECTED
  acquired_at
  released_at
}
```

**竞争模型**：基于存储层的 CAS（Compare-And-Swap）操作：

```python
def claim(session_id, intent_id) -> bool:
    # 原子操作：仅当 intent_id 无 ACQUIRED 记录时才写入
    success = claim_store.cas_insert(
        intent_id=intent_id,
        session_id=session_id,
        state="ACQUIRED",
        # CAS 条件：intent_id 不存在或已 RELEASED
    )
    return success
```

* 单进程内：可退化为内存互斥锁（mutex + dict）
* 跨进程（SubAgent 场景）：必须通过共享存储（SQLite WAL / Redis SETNX / DB row-level lock）
* Claim 有 TTL（建议 30s），超时自动 RELEASED，防止 session crash 后永久锁定

#### 4.5.2 Claim 失败后的 Session 行为

claim 失败意味着另一个 session 已经在执行同类外部动作：

```
claim_or_abort(session, intent_id):
  if not claim(session.session_id, intent_id):
      # 1) 记录冲突到 worklog（可观测）
      session.append_worklog(ClaimConflictLog(intent_id, winner=...))
      # 2) 不执行 action，跳过本步的副作用部分
      # 3) session 不终止，继续推进到下一个 behavior step
      #    （下一步可能决定等待 winner 的结果、放弃、或走其他路径）
      return CLAIM_FAILED
```

关键原则：**claim 失败不等于 session 失败**。Session 只是放弃了"写"的权利，仍然保留"读/理解/规划"的能力，可以在后续 step 中根据 winner session 的结果调整策略。

#### 4.5.3 intent_id 的生成规则

intent_id 必须能够唯一标识"要做的事"，而不仅仅标识"谁想做"：

* **回复类**：`reply:{msg_tunnle_message_id}` — 防止多个 session 对同一条用户消息重复回复
* **动作类**：`action:{action_type}:{target_resource}:{content_hash}` — 防止同一动作被多个 session 重复执行
* **支付类**：`payment:{payee}:{amount}:{nonce}` — 防止重复扣款

不同 session 对同一输入产生语义不同的 action 时，因 intent_id 不同，不会互相阻塞——这是预期行为。但如果两个 session 对同一条消息各自决定回复用户，它们的回复 intent_id 相同（`reply:{msg_id}`），只有一个能成功，另一个的回复草稿会记录在 worklog 中供事后审计。

### 4.6 Workspace（交付空间）

* **Workspace 是用来交付成果的地方**：文档、代码、数据、PR、报告等产物位于 workspace。
* Workspace 不必由 runtime 管辖：可以是本地目录、远程 git repo、SMB 共享、甚至公网服务；可通过 skills 扩展接入。

### 4.7 Workshop（Agent 私有工作区）

* **Workshop 是 Agent 私有的工作区**。在 workshop 内，Agent 可以创建多个私有 workspace（通常称为 `code_workspace` / `local_workspace`）。
* `local_workspace` 若由 session 运行中创建，通常是 Agent 私有；
  若用户先创建 local workspace 再交给 Agent 工作，则该 local workspace 通常属于用户。

> 约束：**session 可以绑定 0 个 local_workspace 和 0..n 个 workspace**。

### 4.8 local_workspace 锁与并行

* 多个 session 可以并行运行，但如果两个 session 使用同一个 `local_workspace`，则这两个 session 只有一个能处于 `RUNNING` 状态。
* 可强化约束：需要等待另一个 session 的所有 todo 完成/失败，以避免非顺序修改同一工作区带来负面影响。

---

## 5. 默认 Agent：Jarvis 的逻辑（示例）

Jarvis 是默认 Agent，但 Jarvis 的流程本质上是“在 runtime 支持下的 behavior 配置组合”。


典型流程（示意）：

```
new_msg + new_event
  │
  │ [写入 MsgTunnle（事实落盘）]
  │
  `Dispatcher route → link` -> 快速回应 / link 到内部 Session
  │
  `Plan-Do-Check-Adjust (PDCA) Loop`
  │
  `回复 append 到 MsgTunnle` -> 用户可见

timer_event
  │
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



### 7.2 执行模型：双线程（UI/Worker）+ Dispatcher

#### 7.2.1 UI Thread（消息线程）

职责：

* 从外部接收消息
* 写入 MsgTunnle（事实落盘）
* 做轻量判断：简单答复 or 投递后台
* 触发 route/link，让相关 session 变 READY

原则：**轻量、快速、不可阻塞**，不做全量历史扫描。

#### 7.2.2 Dispatcher（必选组件）

Dispatcher 是 Expectation 优先级路由、EXCLUSIVE 策略执行、成本管控的唯一承载点，应作为必选组件存在。没有 Dispatcher，UI Thread 就必须承担全部 route→link 决策逻辑，违背"UI Thread 轻量不可阻塞"原则。

职责：

* 做"route → link"的优化决策（Expectation > Active > Inbox）
* 决定 link 到 1..N session
* 管控成本和冲突（广播是特例，不是默认）
* 执行 Expectation 冲突解决流程（见 4.4.2）

初期可退化为最简模式（全部 link 到 inbox session），但组件本身必须存在，为后续 Phase 2-3 的 Expectation 路由和多 session 并行提供扩展点。

#### 7.2.3 Worker（PDCA Loop / Session Loop）

职责：

* 消费 session 的 input links（从 MsgTunnle 按 message_id 拉内容）
* 执行行为 step、工具调用、worklog、workspace side effects
* 进入 WAIT / END，可断点恢复

### 7.3 Route 的新定义

> **Route 不再等于"决定唯一 session_id"。**
> Route 等于：**把新输入投影（link）到合适的内部 Session，并施加安全与成本策略。**

#### 7.3.1 默认路由优先级（推荐）

1. **Expectation Match（最高优先级）**
   * 如果某 session 正在等待回复/授权
   * 命中则 link 到该 session
   * 授权类/敏感类：**EXCLUSIVE claim**

2. **Active Session（高置信度活跃工作容器）**
   * 最近活跃、正在推进任务的 session
   * 普通 link（可纠错，可撤销）

3. **Inbox/Pending Session（内部默认工作篮）**
   * 收纳"暂时无法判定归属"的消息
   * 由后台触发澄清或新建 session

#### 7.3.2 默认 Link 策略表

| 消息类型 | 默认 Link 策略 | 说明 |
| --- | --- | --- |
| 匹配到 EXCLUSIVE expectation 的回复（授权码、确认 yes/no） | **单 link + EXCLUSIVE** | 只投递给发起 expectation 的 session，防止授权被误用 |
| 匹配到 SHARED expectation 的回复（信息补充） | **单 link + NORMAL** | 投递给发起 expectation 的 session，但不阻止其他 session 后续 link |
| 与 active session 明确相关的消息（语义匹配度高） | **单 link + NORMAL** | 默认保守单 link，允许通过 Dispatcher 规则开放多 link |
| 纯信息/上下文类消息（不触发任何副作用） | **允许多 link + NORMAL** | 多个 session 可以引用同一条消息做理解/摘要/跨项目引用 |
| 无法判定归属的消息 | **单 link 到 Inbox + NORMAL** | 兜底策略，后续由 inbox session 做澄清或新建 session |

原则：**不确定时宁可单 link 到 inbox 也不广播**——广播的成本（多 session 触发推理 + 多 worker 可能冲突）远高于单路 link 后纠错的成本。

### 7.4 Agent Loop 的职责（新模型）

Agent Loop 负责：

1. 从 MsgTunnle 拉取新的 msg 与 event（可由 Timer/触发器驱动）。
2. **永远先写入 MsgTunnle（事实源）**，只标记已读，不删除。
3. 通过 route_and_link 把消息**投影（link）到 1..N 个内部 Session**，对敏感场景做 EXCLUSIVE claim。
4. 调度 `READY` 的 session 到 worker thread 执行（每个 session 同一时刻只会被一个 thread 执行）。
5. 定期对超时的 `WAIT_FOR_MSG/WAIT_FOR_EVENT` session 做"超时唤醒"处理（设置为 `READY`）。

### 7.5 Agent Loop & Session Loop（伪代码）

以下伪代码体现"route→link"与 MsgTunnle 事实源的核心改造：

### 7.5 Agent Loop & Session Loop（伪代码）

以下伪代码体现"route→link"与 MsgTunnle 事实源的核心改造：

```python
class AIAgentRuntime:

    def run_agent_loop(self):
        for _ in range(self.max_session_parallel):
            thread.spawn(self.session_run_thread)

        while self.running:
            msg_pack, event_pack = self.pull_msgs_and_events_from_msgtunnle()

            if msg_pack:
                routing = self.route_and_link_msg_pack(msg_pack)  # route 不再 resolve session_id
                self.set_msg_readed_in_msgtunnle(msg_pack)        # 只标记已读，不删除
                for sid in routing.linked_session_ids:
                    self.mark_session_ready(sid)

            if event_pack:
                routing = self.route_and_link_event_pack(event_pack)
                self.set_event_readed(event_pack)
                for sid in routing.linked_session_ids:
                    self.mark_session_ready(sid)

            self.schedule_sessions()

    def session_run_thread(self):
        while self.running:
            session = self.wait_next_ready_session()
            session.update_state("RUNNING")

            if session.current_behavior is None:
                session.current_behavior = self.default_behavior
                session.step_index = 0

            # behavior_loop
            while session.state == "RUNNING":
                behavior_cfg = self.load_behavior_cfg(session.current_behavior)
                self.run_behavior_step(behavior_cfg, session)
                session.save()  # 每 step 保存，支持崩溃恢复

    def run_behavior_step(self, behavior_cfg, session):
        # 会从session的kmsgqueue中pull_input_item
        exec_input = session.generate_input(behavior_cfg, msgtunnle=self.msgtunnle)

        # 零 LLM 空转：无 input 则不触发推理
        if exec_input is None:
            session.update_state("WAIT")
            return

        llm_result = behavior_cfg.do_llm_inference(
            behavior_cfg.build_prompt(exec_input)
        )

        # 1) 对外回复：append to MsgTunnle（事实源）+ link back to session（审计/回放）
        for msg in llm_result.reply:
            out_id = self.append_msg_to_msgtunnle(session, msg)
            self.link_outbound_msg_to_session(session.session_id, out_id)

        # 2) 执行动作（actions）：外部副作用前必须 claim（防止并行 session 误用）
        action_result = None
        if llm_result.actions:
            claim_ok = self.claim(session.session_id, llm_result.actions.intent_id)
            if claim_ok:
                action_result = self.do_action(behavior_cfg, llm_result.actions)
            else:
                # claim 失败：记录冲突，跳过副作用，session 继续推进
                session.append_worklog(ClaimConflictLog(
                    intent_id=llm_result.actions.intent_id,
                    resolution="skipped_action"
                ))

        # 3) 记录 worklog
        session.append_worklog(self.create_step_worklog(action_result, llm_result))

        # 4) 消费 input：mark used（不再迁移/归档 MsgTunnle）
        session.mark_input_used(exec_input)

        # 5) Workspace side effects（worklog/todo）
        if session.workspace_info:
            ws = self.get_workspace(session.workspace_info)
            ws.append_worklog(self.create_step_worklog(action_result, llm_result))
            if llm_result.todo_delta:
                ws.apply_todo_delta(llm_result.todo_delta, session)

        # 6) Memory / Session meta patch
        if llm_result.set_memory:
            self.apply_set_memory(llm_result.set_memory)
        if llm_result.session_delta:
            session.update(llm_result.session_delta)

        # 7) Behavior 切换与 WAIT
        self.apply_behavior_transition(session, behavior_cfg, llm_result)
```

### 7.6 消息/事件在 MsgTunnle 与 Session 内的状态管理

* **消息永远留在 MsgTunnle**，MsgTunnle 是唯一事实源。
* Agent Loop 收到 msg/event 后：在 MsgTunnle 层标记 `readed`（已被 runtime 收到并路由），**不删除、不迁移**。
* 通过 Link 机制将消息关联到 session：
  * Link 状态 `NEW`：session 尚未处理
  * Link 状态 `USED`：session 已确认处理完毕
* Session 通过 link 从 MsgTunnle 按 message_id 拉取消息正文，而非在 session 内存储消息副本。
* 已消费的消息仍可通过 history/memory 段落被编入 prompt（受预算约束）。
* 系统实际使用MsgCenter的MsgRecord作为Link实现，Msg的原始内容是MsgObject保存在MsgTunnel的inbox中

### 7.7 读/推理放大控制策略

"全量扫描式（每个 worker 扫历史）"带来的是读放大/推理放大/token 放大，不适合作为长期默认。

#### 建议的长期默认策略

* 默认：**被动触发式（Dispatcher link → Worker 消费 link）**
* 兜底：允许 worker 做"按需检索"，但必须是增量扫描（只看 cursor 之后的新消息）或 top-k 相关检索（embedding/tag/recency）

#### 成本模型对比

* 全量扫描成本：`O(worker数 × 历史长度 × 推理开销)`，并行一多必炸
* link 模式成本：`O(新输入量 × 路由开销 + 少量 session 执行)`，可控、可优化、可缓存

#### 关键底线：多 worker 并行时必须仲裁

否则会出现多个 worker 同时回复用户、多个 worker 同时执行外部动作（灾难级）。解决：Claim 机制（见 4.5）。


### 7.8 端到端时序：从用户消息到 Agent 回复

以下时序图展示一条用户消息从发出到 Agent 回复的完整链路：

```
用户                UI Thread           MsgTunnle          Dispatcher          Session Worker       外部系统
 │                    │                    │                    │                    │                  │
 │── 发送消息 ──────>│                    │                    │                    │                  │
 │                    │── 写入消息 ──────>│                    │                    │                  │
 │                    │                    │── 返回 msg_id ──>│                    │                  │
 │                    │── 请求路由 ──────────────────────────>│                    │                  │
 │                    │                    │                    │                    │                  │
 │                    │                    │  ┌─ Expectation Match? ──── 命中 ──> link(EXCLUSIVE)     │
 │                    │                    │  │  Active Session?   ──── 命中 ──> link(NORMAL)        │
 │                    │                    │  │  Inbox fallback    ────────────> link(NORMAL)        │
 │                    │                    │  └──────────────────────────────────┐                    │
 │                    │                    │                    │                │                    │
 │                    │                    │                    │── mark_ready ─>│                    │
 │                    │                    │                    │                │                    │
 │                    │                    │                    │                │── generate_input() │
 │                    │                    │<──── 按 msg_id 拉正文 ─────────────│                    │
 │                    │                    │── 返回消息正文 ──────────────────>  │                    │
 │                    │                    │                    │                │                    │
 │                    │                    │                    │                │── LLM 推理         │
 │                    │                    │                    │                │── 产出 reply+action│
 │                    │                    │                    │                │                    │
 │                    │                    │                    │                │── claim(intent_id) │
 │                    │                    │                    │                │   (若有外部动作)    │
 │                    │                    │                    │                │── 执行动作 ───────>│
 │                    │                    │                    │                │<── 动作结果 ───────│
 │                    │                    │                    │                │                    │
 │                    │                    │<── append 回复到 MsgTunnle ────────│                    │
 │<── 用户看到回复 ──│<── 推送新消息 ─────│                    │                │                    │
 │                    │                    │                    │                │── mark_link_used() │
 │                    │                    │                    │                │── save session     │
```

关键观察：

* 消息从进入到回复，经过两次 MsgTunnle 写入（用户输入 + Agent 回复），保证事实源完整。
* Session Worker 通过 link 间接访问 MsgTunnle，从不直接存储消息正文。
* 外部动作执行前必须 claim 成功，否则跳过副作用但 session 继续推进。

---

## 8. Behavior 体系与默认 PDCA（Jarvis）

> Runtime **不假设**一定存在 PDCA；PDCA 是 Jarvis 通过 behavior 配置实现的默认逻辑。  
> 但 runtime 必须提供：behavior step、输入编译、输出协议、状态机、工具执行、日志与回滚能力。
> 该章节通过实际例子，说明OpenDAN的基础设施的设计细节

### 8.1 resolve_router / router（route→link 模型）

**目标**：将新输入投影（link）到合适的内部 Session，并施加安全与成本策略。在必要时做快速应答。

* Route 的输出不再是"唯一 session_id"，而是 `linked_session_ids + link_ids`。
* 路由优先级：Expectation Match > Active Session > Inbox/Pending Session（见 7.3.1）。
* 对授权/敏感场景做 EXCLUSIVE claim。
* 快速应答（琐碎输入直接回复）仍通过 UI Thread 轻量处理。
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

等待是"可观测、可恢复"的 runtime 机制，而不是让 LLM 空转。

* **等待用户补充信息**：使用 `send_msg`（append 到 MsgTunnle）主动沟通，请求补充；用户通过 msg 补充信息（写入 MsgTunnle）；session 进入 `WAIT_FOR_MSG`。
* **等待用户授权**：使用 `send_msg` 请求授权；用户通过系统命令授权或 deny；授权结果产生 event；session 进入 `WAIT_FOR_EVENT`。

由于整体沙盒约束，OpenDAN 需要用户授权的操作相对更少，让流程更流畅；但涉及敏感能力（文件写入、网络、支付等）仍必须走 Policy Gate。

### 9.1 EXCLUSIVE Expectation（单选独占）

* 当 session 发出"授权/确认"请求时，创建一个 EXCLUSIVE expectation（数据结构与匹配规则见 4.4.1）。
* 后续用户回复只能命中一个 expectation（否则必须向用户澄清，流程见 4.4.2 冲突处理状态机）。
* Expectation router 把后续用户回复匹配回正确 session，授权类默认 EXCLUSIVE。
* 每个 expectation 有独立超时，超时后自动降级（见 4.4.3）。

### 9.2 Claim 仲裁（外部动作单路提交）

即使一条消息被多个 session link：

* 多个 session 可以"读/理解"
* 但只有 claim 成功的那个 session 才能"做/发/改/删/付"（实现机制见 4.5.1）

claim 失败不等于 session 失败——session 放弃"写"的权利但保留"读/理解/规划"的能力（失败处理见 4.5.2）。

这能从机制上避免"授权回复被多个 session 误用"。

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
* 文件编辑工具（适合做结构化 diff、patch）
  - read_file 读取文件
  - write_file 覆盖/创建/在尾部追加 文件。
  - edit_file 基于字符串匹配的精确修改（Surgical edits）
* bash（session 有 cwd 概念与环境变量，便于定位 workspace 目录）
  - 必定有git工具

> 下面能力已经被移除

* git 工具 (已经包含在bash里了)
* OpenDAN 基础工具（消息、事件、todo、memory 等）
  - OpenDAN & BuckyOS Runtime的tools是高级接口，不属于元能力

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

### 12.1 Prompt 结构

每次 step 的 prompt 由两部分组成：
> 仔细考虑注意力框架的U型结构 关键点在提示词的头部和尾部

**System Prompt（在 step 中相对静态）**

* `<<role>>`：加载 agent 的 `role.md` + `self.md`
* `<<process_rules>>`：加载当前 behavior 的 process_rule
* `<<output_protocol>>`：输出协议（至少支持 `BehaviorLLMResult` 与 `RouteResult` 两种模式）
* `<<policy>>`：加载当前 behavior 的 policy
* `<<toolbox>>`：当前 behavior 可用 tool + skill 配置 （这是一个列表）
  * `$loaded action & skills`: 已经选择的skills会在这里，会被后续的llm.result.choose skill影响

**User Prompt**

* `<<Memory>>`：系统按优先级与预算自适应编入：
  * AgentMemory (可裁剪)
  * Workspace Summary (一个固定的文件替换)
  * Workspace Worklog （可裁剪）
  * History Messages （可裁剪）
  * Session Summary （细节还要讨论，通常在Adjust阶段，反思错误的时候深度更新）
  * Session@Workspace Todolist  

* `<<Input>>`：非常关键；不同模式 input 模板不同；若无法得到 input，本 step 被跳过。这里的核心是模板替换，下面只是常见的例子
  * {{new_msg}}（处理后系统标记 readed；对 session 则从 new_msg 进入 history_msg）
  * {{new_event}}
  * Current Todo Details (`通用模版替换得到`)
  * LastStep Summary（含成本信息与 step 计数） （`observation区，怎么构造?`)

> OpenAI的API会把tools的定义放在最后，这个是协议层的组合，无法调整顺序


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

  {{$workspace/to_agent.md}}
  {{$cwd/to_agent.md}}

policy: |
  * 仅输出 JSON 对象——不得包含其他内容。
  * 必须包含所有三个字段。
  * memory_queries 必须是数组；为空用 []

toolbox:
  skills: ["coding/rust"] # 实际的skills 是 ["buildin","coding/rust"]
  default_load_skills: ["buildin"]

output_protocol:
  mode: RouteResult

# 注意,history是严格按时间混编的。worklog和chat message log会裁剪后混在一起
memory:
  total_limt: 12000
  agent_memory: { limit: 3000 }
  history_messages: { limit: 3000, max_percent: 0.3 }
  session_summaries: { limit: 6000 }

input: |
  {{last_step_summary}}
  {{new_event}}
  {{new_msg}}
  {{new_msg.1}}
  {{workspace.todolist.next_ready_todo}}
  {{workspace.todolist.__OPENDAN_ENV(params.todo)__ }}

# step = 0 的时候不构造这个
step_summary: |
  {{llm_result.thinking}}
  {{llm_result.action_results}}

limits:
  max_tool_rounds: 1
  max_tool_calls_per_round: 4
  deadline_ms: 45000

llm_model:
  name : llm.code # 让router机制决定llm.code目前具体是哪个
  thinking_level : medium
```

> 说明：route behavior 通常不允许使用工具，以确保快速、低成本地完成分流与快速回应。

### 12.4 Toolbox的提示词拼接

Toolbox 是 **Behavior 级别** 的能力边界，不是 Agent 的理论全能力集合。

* 如果本次 step 的 prompt 没有拼接 `<<toolbox>>`，就应视为纯聊天模式：不触发 tool call，不执行 actions。
* 同一个 Agent 在不同 Behavior 下应有不同 Toolbox；不要把“全能力”一次性全塞给所有 Behavior。

#### 12.4.1 配置目标与原则

* 目标是“约束优化”，不是“能力堆叠”。
* 工具越精准，模型决策噪音越小，成功率通常越高。
* Tool 选择本身也是行为决策的一部分：先判断是否足够，再决定是否需要扩展能力。

#### 12.4.2 两种配置模式（继承 / 覆盖）

可按下面两条心智模型理解：

* 继承：`Behavior Toolbox = 默认工具集合 + Behavior 补充 skills`
* 覆盖：`Behavior Toolbox = 显式声明的工具集合（不再扩展默认集合）`

**A) 继承模式（默认推荐）**

适用场景：需要先探索，再决定是否深入调用工具的 Behavior（如路由、计划）。

```yaml
toolbox:
  skills: ["buildin"] # 实际的skills 是 ["buildin","coding/rust"]
  default_load_skills: ["buildin"]
```

要点：

* 保留基础能力，支持先观察再执行。
* 只补充当前 Behavior 真正需要的 skill，避免无关 skill 干扰。

**B) 覆盖/隔离模式（精确控制）**

适用场景：高风险、高成本、或目标非常单一的 Behavior。

```yaml
toolbox:
    mode: alone
    skills: ["coding/rust"] # 这个例子没有default_load_skills，纯拼
    default_allow_functions: ["read_file","bash"] # 在function里使用read tool
   
```

要点：

* `alone` 只允许名单内工具，缩小决策空间。
* 适合“只读分析”“受限执行”“审计优先”等场景。

**C) 纯聊天/禁用工具模式**

适用场景：快速分流、寒暄回复、低成本应答。

```yaml
toolbox:
    mode: none
```

要点：

* 显式禁止工具，避免误触发。
* 与 route 类 behavior 的“快进快出”目标一致。

#### 12.4.3 工程约束（提示词工程师必看）

* 优先使用 `skills`，和 `default_load_skills`,
* 使用`allow_tools`， `default_load_actions` 时必须清楚的知道自己在干啥，不要和当前加载的skills冲突了
* 一个 Behavior 只给完成目标所需的最小工具集，不要“为了保险”扩大集合。
* 在 process_rule 中明确工具使用策略：  
  先用已有工具完成；确认不足时再触发加载/切换。

#### 12.4.4 Loop 中的状态表达（审计必需）

Tool 加载会改变行为边界，属于状态变更，不是无副作用操作。发生加载时，建议在 step 结果和 worklog 中显式记录：

```json
{
  "choose_skills":["coding/webapp"],
  "reason":"I will use this skill to build web app"
}
```

这保证了：

* 后续 step 能基于新边界稳定推理；
* 行为变更可追踪、可回放、可审计。

## 13. Memory（记忆系统）

- 构造提示词的时候，总是会根据behavior的配置自动拼接Agent Memory,不依赖任何的action
  - 该行为，受到session里的上一次ll_result.memory_queries的影响
- 通过function的read_file / bash ，也可以实现搜索本地文件系统并加载memory
- BehaviorLLMResult中没有memory_queries? 是因为我们鼓励提示词把有价值的memory写入session summary,而不是文档的持有memory item

> 在 UI Session的 Virtual Topic tag机制将会更好的使用memeory

详细的操作见 独立文档

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

* chat history 的取用策略（最近 N 轮 / 摘要 / 任务相关片段）。在新架构下，chat history 的事实源是 MsgTunnle 而非 session 内部存储；AgentMemory 通过 link 引用和 SessionNote 派生摘要来构建 prompt 中的 history 段落。
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

* 用户通过 MsgTunnle 向 Agent 发消息（用户只看到 MsgTunnle 历史）
* Agent 在 behavior 中主动回复/发送消息（append 到 MsgTunnle，最直观交互）
* 通过 BuckyOS TaskMgr：从 LLM Task 角度看到 Agent 触发的 LLM 动作、action、tool、成本与错误
* 通过 Workspace UI：查看 todo、worklog、subagent 状态；用户可直接干预 item

### 20.1 Link 模型带来的可观测性增强

Link 模型会让 debug 变得非常直接：

* 一条 MsgTunnle 消息 link 到了哪些 session？
* 为什么 link（reason/score）？
* 哪个 session claim 了外部动作？
* 哪些 input links 已消费/未消费？
* session 当前卡在哪个 WAIT 条件？

建议核心指标：

* link 命中率（Expectation/Active/Inbox）
* EXCLUSIVE expectation 冲突率（需要澄清的比例）
* claim 失败率（并发冲突）
* 平均每条消息 link 到多少 session（控制广播）
* 每个 session 平均 token/step 成本

示意（树形结构展示串行 session 与并行 subagent）：

```
Jarvis
  MsgTunnle (用户可见)
    msg#1 → link → Session#A, Session#B
    msg#2 → link → Session#A (EXCLUSIVE claim)
  
  Session#A (内部工作容器)
    resolve_router (route→link)
      LLM Task
      RESULT
    PLAN
      LLM Task
      Action Tools
    DO
      LLM Task
      Action Tools (claim before external action)
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

## 21. Agent 文件系统示例

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

## 22. MVP 模块与 Roadmap

> Roadmap 已与 MsgTunnle + Link 架构改造的分阶段落地计划对齐。

### MVP-1：单 Agent 可运行与可观测闭环 + Link 基础引入（对应 Phase 1）
* session/behavior step-loop + bash actions + 最小 workshop/workspace + worklog
* 引入 MsgTunnle 作为事实源 + SessionMessageLink 数据结构
* Dispatcher 以最简模式运行（全部 link 到 inbox session）
* generate_input 从 link 消费，mark_link_used 替代旧的 history 迁移

### MVP-2：Expectation 路由 + 授权安全（对应 Phase 2）
* 引入 Expectation 数据结构与 Expectation Match 路由优先级
* EXCLUSIVE expectation 用于授权/确认链路
* Expectation 冲突解决流程（自动仲裁 + 用户澄清）
* 保证"等待回复"不乱跳

### MVP-3：多 Session 并行 + Claim 仲裁（对应 Phase 3）
* Active Session 路由启用
* Claim 机制上线（CAS 实现 + TTL + 失败降级）
* 多 worker 并行调度
* 核心可观测指标上线（link 命中率、claim 冲突率、expectation 冲突率）

### MVP-4：SubAgent 与能力包
* SubAgent 独立进程、预算限制、workspace 协作交付
* 跨进程 claim 支持（共享存储层）
* 集成 BuckyOS 的特殊能力（目录发布成网页/App 等）

### MVP-5：Memory 深化 + 存储优化（对应 Phase 4）
* compact_memory、things.sqlite 事实抽取、可视化
* MsgTunnle 冷热分层存储
* Topic/Tag 检索优化（减少 token、提高相关性）
* Session GC 策略上线（归档 + 可选销毁）

### MVP-6：Workspace 深化
* git 协作、基于文件系统的协作（如 `smb://`）

### MVP-7：KB
* 至少一种 KB（全文/向量/图）管线
* 专门 KB 整理 Agent（Mia）


---

## 23. MsgTunnle + Link 架构改造落地指南

> 本节说明此次架构改造对现有系统的具体改动点与渐进落地建议。

### 23.1 对现有系统的改动点清单

原有 runtime 本来就有 session worker + behavior step 框架，真正需要改的点集中在三个接口语义：

#### route 层：从 resolve(session_id) 改成 link(session)

* 删除（或降级为辅助）：`llm_route_resolve(msg_pack) -> session_id`
* 新增：`route_and_link_msg_pack(msg_pack) -> linked_session_ids + link_ids`

#### input 层：session.generate_input 从"session 内消息队列"改成"消费 links"

* 新增：`SessionMessageLink` store
* `generate_input()` 从 link 拉 message_id，再去 MsgTunnle 拉正文

#### 消费语义：从"移动到 history"改成"mark link used"

* 不再迁移/归档 MsgTunnle
* 只更新 link 状态：NEW → USED（或 ACKED）

#### 新增组件

* `Expectation` store + Expectation 冲突解决逻辑（Phase 2）
* `ClaimRecord` store + CAS claim 逻辑（Phase 3）
* MsgTunnle 本地读缓存（LRU by message_id）
* Session GC 定时任务

> 原有的 worklog/workspace/memory/skill/behavior 切换基本不动。

### 23.2 渐进落地建议（最小风险上线）

#### Phase 1：引入 Link，不改行为逻辑

* MsgTunnle 仍然是消息源
* route 先只 link 到 inbox session（保守）
* generate_input 先只消费 inbox links
* 验收标准：现有单 session 工作流不退化，link 状态可观测

#### Phase 2：加入 Expectation Router + EXCLUSIVE

* 授权/确认链路先跑通
* 保证"等待回复"不乱跳
* 验收标准：授权回复 100% 命中正确 session，冲突场景能触发用户澄清

#### Phase 3：引入 Active Session 路由 + 多 worker 并行

* 增加 claim 仲裁
* 增加 metrics
* 验收标准：多 session 并行不出现重复回复或重复执行外部动作

#### Phase 4：Topic/Tag/检索优化（可选）

* 用于减少 token、提高相关性
* 但不影响主架构正确性

---

## 24. 架构改造 FAQ

### Q1：用户是不是看不到 session 了？

是的。用户只看到 MsgTunnle 的历史；Session 作为内部工作容器，UI 只读或完全不展示。

### Q2：消息可以被多个 session link 吗？

可以（架构允许）。但对授权/确认/外部动作类默认要 EXCLUSIVE/claim，先保守再开放。具体的默认策略见 7.3.2 Link 策略表。

### Q3：如果 link 错了怎么办？

因为我们不迁移事实源，纠错只需要 unlink / relink 或调整权重/原因，无需改写 MsgTunnle 历史，风险显著降低。

### Q4：Topic/Tag 在新架构里还需要吗？

需要，但定位变了：Topic/Tag 用于"检索与压缩相关上下文"，不是替代 Session 的状态机容器。

### Q5：MsgTunnle "不可删除"和 GDPR 矛盾吗？

不矛盾。"不可删除"是逻辑层的工作原则（runtime 不会因为路由/归档需要而删除消息），但隐私合规场景支持软删除（REDACTED），详见 4.1.2。

### Q6：claim 失败了 session 会怎样？

claim 失败不等于 session 失败。Session 放弃"写"权利但保留"读/理解/规划"能力，在后续 step 中可以根据 winner session 的结果调整策略。失败的 action 草稿记录在 worklog 中供审计，详见 4.5.2。

### Q7：两个 session 同时等授权，用户回复"好的"怎么办？

Dispatcher 先尝试自动仲裁（AUTH 优先、即将超时的优先），无法决定则向用户发送澄清消息列出冲突 session 的摘要。澄清超时后按最近活跃 session 降级匹配，详见 4.4.2。

### Q8：Session 会一直累积吗？

不会。Session 有完整的生命周期管理：活跃 → 休眠 → 归档 → 可选销毁。归档后只保留轻量摘要，MsgTunnle 中的原始消息不受影响，详见 4.2.1。

### Q9：新旧模型核心差异对比

| 维度 | 旧模型：session 归属/迁移 | 新模型：MsgTunnle 事实源 + Link 投影 |
| --- | --- | --- |
| 消息存储 | session 内常扮演事实存储 | **MsgTunnle 是唯一事实源** |
| "route"的含义 | resolve 到唯一 session_id | **link 到 1..N session（可多归属）** |
| 删除/迁移 | 容易引入迁移/归档语义 | **不迁移、不删除 MsgTunnle** |
| 并行任务 | 依赖 session 切分准确 | 允许多个 session 并行，但要有仲裁 |
| 授权安全 | 依赖"路由选对" | **EXCLUSIVE expectation + claim 仲裁** |
| 成本模型 | 路由难、无 session 难 | route 稳定，worker 不扫全量历史 |

---
