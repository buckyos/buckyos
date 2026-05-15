# Session History 设计

> 状态：设计草案 / beta2.2
> 范围：AgentSession 的对话与行为历史；与 llm_context 解耦但语义对齐
> 目标：提供一个**不被压缩破坏**、可由任意模块按 round 索引读取的历史视图

---

## 1. 背景与问题

当前 AgentSession 的"历史"实际上是 `LLMContextSnapshot.state` 的两个字段：

- chat / traditional 模式：`accumulated: Vec<AiMessage>`
- behavior 模式：`steps: Vec<StepRecord>` + `last_step: Option<StepRecord>`

两者都会被 **压缩破坏**：

- `accumulated`：`llm_compress::compress()` 保留 system + 尾部 K 条，中间段 LLM 总结成一条 system summary message，**原始中间消息被丢弃**（`src/frame/agent_tool/src/llm_compress.rs:93`）。触发点 `agent_session.rs:1583-1633` 的 `ContextLimitReached` 处理，直接 `prepared.state.accumulated = rewritten`
- `steps`：`maybe_compress` 把 `Vec<StepRecord>` 整段交给压缩器，返回更短序列；XmlStepRenderer 等渲染器把老步折叠为摘要（`src/frame/llm_context/src/context_loop.rs:17-31, 841-882`）

`{session_dir}/.meta/state.snap` 是唯一持久化，存的就是压缩后的状态。**没有任何独立的、未压缩的完整历史存储**。

后果：

| 你想读 | 现状 |
|---|---|
| LLM 下一次将看到的上下文 | ✅ 读 accumulated / steps |
| 用户和 agent 真实说过的全部话 | ❌ 早期会被压成 summary |
| 某一轮真实的 tool_use / tool_result / action / observation | ❌ 同上 |
| 用于审计、回放、前端展示的完整对话 | ❌ 不可靠 |

根因：**"LLM 上下文"和"对话历史"被同一个字段承载**。前者要可裁剪、后者要不可变。两者职责绑死。

---

## 2. 设计原则

1. **History 与 LLM Context State 解耦**。History 是 append-only 的真理源；`accumulated` / `steps` 退化为派生缓存，是"下一次喂给 LLM 的窗口"，可被压缩任意改写
2. **以 llm_context 的语义为锚**。Entry 的种类、字段命名、模式区分都对齐 llm_context 暴露的 `AiMessage` / `StepRecord` / `LLMContextOutcome` / `Observation`，不发明新词
3. **Round 为对外索引单元**。一条用户消息（或一次系统触发）= 一个 round，由 `round_index: u64` 标识。任何外部模块都用 round_index 寻址
4. **Round 内 entry_seq 平铺**。不引入 step / turn 的二级索引，所有事件按出现顺序编号
5. **写入低耦合**。AgentSession 在已有的几个边界点调 writer 即可，llm_context 内部不感知 history 存在
6. **Reader 与 Writer 解耦**。任何模块/进程能独立打开 reader 读 `{session_dir}/.history/`，不依赖 AgentSession 运行态、不持锁

---

## 3. 与 llm_context 的语义映射

History 的命名严格对齐 `src/frame/llm_context/` 暴露的概念：

| llm_context 概念 | 来源 | 在 History 中的角色 |
|---|---|---|
| `AiMessage` (role + Vec<AiContent>) | `aicc_client.rs` | chat 模式的 entry 载荷 |
| `StepRecord` (assistant_text + thought + action + observation + next_behavior + action_result) | `behavior_loop.rs:20-46` | behavior 模式的 entry 载荷 |
| `Observation` (Success / Error / Pending / Cancelled) | `observation.rs:11-32` | 嵌在 StepRecord.action_result，原样保留 |
| `LLMContextOutcome` (Done / PendingTool / ContextLimitReached / BudgetExhausted / Error) | `outcome.rs:89-129` | 每次 LLM context 调用结束写一条 OutcomeRecord 事件；驱动 round status |
| `LLMBehaviorResult` (do_actions + next_behavior + assistant_text + thought + observation) | `behavior_loop.rs` | OutcomeRecord 的 payload 一部分 |
| `accumulated` 的压缩 | `llm_compress.rs` | 写一条 Compaction 事件（审计），**不动 history 主体** |
| `steps` 的压缩 | `context_loop.rs:maybe_compress` | 同上 |
| sediment (last_step → steps) | `context_loop.rs:841-882` | 写入时机：每次 sediment 时把新沉积的 StepRecord append 到 history |
| chat 模式 accumulated 追加 | `run_inner` | 写入时机：每次新追加一条/多条 AiMessage 时同步 append |
| fork / independent sub-context | `context_loop.rs:633-647` | sub-context 独立产生自己的 LLMContext 对象；history 视角下它属于 **同一 round 内的延续**，sub-context 产出的 entries 续编 entry_seq |

**关键决定**：history entry 是**模式相关的载荷**。Chat 模式记录 `AiMessage`，behavior 模式记录 `StepRecord`，而不是统一成"反正都渲染成 AiMessage"。原因：

- StepRecord 含有 XmlStepParser 解析后的结构化字段（thought / action / observation / next_behavior），渲染成 AiMessage 后这些结构被序列化进文本就**回不来**了
- 审计 / 回放 / 前端展示都需要这些结构（前端要分别渲染思考与动作）
- 模式由 session 创建时决定（`deps.result_parser` 是否存在），不会切换 —— 一个 session 的 history 文件里 entries 是同质的

---

## 4. 概念定义

### 4.1 Round

```rust
struct RoundSummary {
    round_index: u64,                     // per-session 自增，从 1
    trigger: RoundTrigger,
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    status: RoundStatus,
    entry_count: u32,
    mode: ContextMode,                    // Chat | Behavior，整 session 不变
}

enum RoundTrigger {
    UserMsg { preview: String },          // 用户文本（前 100 字预览）
    SystemEvent { source: String, kind: String },  // 调度唤醒、外部 API、tool 异步回调等
    Resume,                               // 进程重启后 first round（兜底，可选）
}

enum RoundStatus {
    Open,                                 // 正在写入
    Completed,                            // LLMContextOutcome::Done
    Interrupted,                          // interrupt() 触发
    Errored,                              // LLMContextOutcome::Error / BudgetExhausted
    WaitingInput,                         // LLMContextOutcome::PendingTool 等异步等待
}
```

### 4.2 Entry

```rust
struct Entry {
    seq: u32,                             // round 内自增
    ts: DateTime<Utc>,
    payload: EntryPayload,
}

enum EntryPayload {
    Message {                             // chat 模式
        message: AiMessage,
        llm_call: Option<u64>,            // 同次 LLM 调用产出的多条 message 共享同一 id
    },
    Step {                                // behavior 模式
        step: StepRecord,                 // 原样存 llm_context 的 StepRecord
        llm_call: u64,
    },
    Event(HistoryEvent),                  // 控制面，msgonly/full 视图过滤掉
}

enum HistoryEvent {
    SystemInput { source: String, payload: serde_json::Value },  // PendingInput::Event 落地
    Outcome {                             // 每次 LLMContextOutcome 都记一条
        kind: OutcomeKind,                // Done | PendingTool | ContextLimitReached | BudgetExhausted | Error
        behavior_result: Option<LLMBehaviorResult>,
        usage_delta: Option<AiUsage>,
        error: Option<String>,
    },
    Compaction {                          // 压缩审计
        target: CompactionTarget,         // Accumulated | Steps
        dropped: u32,                     // 被丢弃的消息/步数
        kept_head: u32,
        kept_tail: u32,
        summary_preview: String,
    },
    Interrupt {
        mode: InterruptMode,              // Graceful | Discard
        reason: Option<String>,
    },
    Fork {                                // 进入 sub-context
        child_label: String,
    },
    Join {                                // sub-context 结束
        child_label: String,
        outcome_kind: OutcomeKind,
    },
}
```

注意 Step.step 直接复用 `llm_context::StepRecord`，意味着 history 模块对 llm_context 有 **read-only 依赖**，但 llm_context 对 history **无依赖**。

### 4.3 View

```rust
enum HistoryView {
    /// 仅对话本体：
    /// - chat 模式：role ∈ {user, assistant} 且 AiContent::Text 块；丢弃 tool_use / tool_result / thinking
    /// - behavior 模式：每个 step 提取 thought + observation + assistant_text-text-portion（去 XML）作为伪 AiMessage 对，
    ///   或者只取 step.observation 与最末 step 的 assistant 输出
    /// - 全部 Event entries 丢弃
    MsgOnly,

    /// 完整结构化数据：
    /// - chat 模式：完整 AiMessage 列表（含所有 AiContent 块）
    /// - behavior 模式：完整 StepRecord 列表
    /// - Event entries 仍丢弃（留给 v2 的 Raw 视图）
    Full,
}
```

`MsgOnly` 在 behavior 模式下需要一个固定的"还原对话"算法。建议：
- 用户输入 → user 消息
- 每个 Step → 一对 `{role: assistant, content: [Text(thought + assistant_text 末段)]}` + `{role: tool, content: [Text(observation)]}`
- 这样 behavior 与 chat 在 MsgOnly 视图下对前端长得一样

---

## 5. 存储布局

```
{session_dir}/.history/
  index.json                    # Vec<RoundSummary>，每个 round 完成时 rewrite 整个文件
  rounds/
    000001.jsonl                # 一个 round 一个文件，append-only
    000002.jsonl
    ...
```

### 5.1 index.json

数组形式。round 数量预期百-千级，整体 rewrite 可以接受。

```json
[
  {
    "round_index": 1,
    "trigger": {"kind": "user_msg", "preview": "帮我看下 .env 文件"},
    "started_at": "2026-05-15T10:00:00Z",
    "ended_at": "2026-05-15T10:00:08Z",
    "status": "completed",
    "entry_count": 7,
    "mode": "chat"
  },
  {
    "round_index": 2,
    "trigger": {"kind": "system_event", "source": "schedule", "kind": "tick"},
    "started_at": "2026-05-15T10:05:00Z",
    "ended_at": null,
    "status": "waiting_input",
    "entry_count": 3,
    "mode": "behavior"
  }
]
```

### 5.2 rounds/000001.jsonl（chat 模式示例）

```jsonl
{"seq":1,"ts":"...","payload":{"kind":"message","message":{"role":"user","content":[{"type":"text","text":"帮我看下 .env 文件"}]},"llm_call":null}}
{"seq":2,"ts":"...","payload":{"kind":"message","message":{"role":"assistant","content":[{"type":"thinking","text":"..."},{"type":"tool_use","name":"read_file","input":{"path":".env"}}]},"llm_call":1}}
{"seq":3,"ts":"...","payload":{"kind":"message","message":{"role":"tool","content":[{"type":"tool_result","content":"FOO=bar\n..."}]},"llm_call":1}}
{"seq":4,"ts":"...","payload":{"kind":"message","message":{"role":"assistant","content":[{"type":"text","text":".env 里有 FOO=bar"}]},"llm_call":2}}
{"seq":5,"ts":"...","payload":{"kind":"event","event":{"type":"outcome","kind":"done","behavior_result":null,"usage_delta":{"input_tokens":420,"output_tokens":58}}}}
```

### 5.3 rounds/000007.jsonl（behavior 模式示例）

```jsonl
{"seq":1,"ts":"...","payload":{"kind":"message","message":{"role":"user","content":[{"type":"text","text":"清理过期 session"}]}}}
{"seq":2,"ts":"...","payload":{"kind":"step","step":{"assistant_text":"<thought>...</thought><action>list_sessions</action>","thought":"...","action":"list_sessions","observation":null,"next_behavior":null,"action_result":{"kind":"success","output":"[...]"}},"llm_call":1}}
{"seq":3,"ts":"...","payload":{"kind":"step","step":{"assistant_text":"...","thought":"...","action":"delete","observation":"deleted 3 sessions","next_behavior":null,"action_result":{"kind":"success","output":"ok"}},"llm_call":2}}
{"seq":4,"ts":"...","payload":{"kind":"event","event":{"type":"compaction","target":"steps","dropped":5,"kept_head":0,"kept_tail":3,"summary_preview":"前期清理 5 个过期 session"}}}
{"seq":5,"ts":"...","payload":{"kind":"event","event":{"type":"outcome","kind":"done","behavior_result":{"do_actions":[],"next_behavior":null,"assistant_text":"清理完毕","thought":"...","observation":"deleted 3 sessions"}}}}
```

### 5.4 持久化保证

- **每条 entry**：写入后立刻 `write_all` 到 OS 缓冲，**不强制 fsync**
- **round close 时**：`finalize_round` 内对 jsonl fsync 一次，再 atomic rewrite `index.json`（tmp + rename + fsync 目录）
- 崩溃恢复：扫描 `rounds/*.jsonl`，比对 `index.json`。发现 `index.json` 未登记的 jsonl → 把其视作 status=Errored 并补登记；发现 `index.json` 标 Open 但进程不是当前 PID → 同样置 Errored
- 末行可能残缺（writer 未 flush）：reader 按行 deserialize，遇到末行 parse error **静默丢弃**，下次 reader 调用会自动看到完整数据

---

## 6. 写入接入点

`AgentSession` 加字段：

```rust
struct AgentSession {
    ...
    history: Arc<Mutex<SessionHistoryWriter>>,
}
```

### 6.1 Writer 接口

```rust
struct SessionHistoryWriter { /* index + 当前 round 文件句柄 + 计数器 */ }

impl SessionHistoryWriter {
    async fn open(session_dir: &Path, mode: ContextMode) -> Result<Self>;
    async fn begin_round(&mut self, trigger: RoundTrigger) -> Result<u64>;
    async fn append_message(&mut self, msg: AiMessage, llm_call: Option<u64>) -> Result<()>;
    async fn append_step(&mut self, step: StepRecord, llm_call: u64) -> Result<()>;
    async fn append_event(&mut self, event: HistoryEvent) -> Result<()>;
    async fn finalize_round(&mut self, status: RoundStatus) -> Result<()>;
    fn current_round(&self) -> Option<u64>;
}
```

### 6.2 钩子位置（`src/frame/opendan/src/agent_session.rs`）

| 钩子点 | 现有位置 | 调用 |
|---|---|---|
| 用户输入入队 | `enqueue_pending` line 535（`PendingInput::Msg`）| 关旧 round（若 Open & 非 WaitingInput）→ `begin_round(UserMsg)` |
| 系统事件入队 | `enqueue_pending` line 535（`PendingInput::Event`）| 当前有 Open round：`append_event(SystemInput)`；无 Open round：`begin_round(SystemEvent)` + `append_event(SystemInput)` |
| chat 模式追加消息 | `run_one_turn` 内部 / `LLMContext` 产出回调 | `append_message`（每条新 AiMessage 即时写） |
| behavior 模式沉积 | `context_loop.rs:841 sediment` 后 / `run_one_turn` 拿到 outcome 时遍历 newly sedimented | `append_step`（每条新 StepRecord） |
| Outcome 产出 | `run_one_turn` 处理 LLMContextOutcome 时 | `append_event(Outcome{...})` |
| 压缩 | `agent_session.rs:1583-1633` 的 `compress()` 调用点 / `maybe_compress` 后 | `append_event(Compaction{...})` |
| Interrupt | `interrupt()` line 559 | `append_event(Interrupt{...})` → `finalize_round(Interrupted)` |
| Fork / Join | sub-context 进入/退出 | `append_event(Fork/Join)` |
| Round 终结 | `run_one_turn` 返回时（Outcome 已记录后） | `finalize_round(Completed/WaitingInput/Errored)` |

**llm_context 不感知 history**：钩子全在 agent_session 层。需要把"newly sedimented step"和"newly appended AiMessage"暴露给上层。两种实现路径：

- **A. 上层在 outcome 后做 diff**：`run_one_turn` 拿到 outcome 时，对比调用前后的 `state.steps` 长度 / `state.accumulated` 长度，把新增段塞给 writer。零侵入，但 fork/independent 场景的归属需要小心
- **B. llm_context 暴露一个 `&mut dyn StateListener`**：sediment / append_to_accumulated 时回调。侵入小但需要修 llm_context 接口

**推荐 A**，避免动 llm_context。

### 6.3 多 user msg 聚合的归属（quirk）

worker 在 `run_one_turn` 前会 drain `pending_inputs` 一批输入。如果一次拿到多条 user msg：

- 每条 user msg 各开一个 round
- LLM 实际只跑一次（一次 turn），新增的 step/message 归属 **最后一个 Open round**
- 早期 round 只含 user msg，状态置 Completed，无后续 entries

文档化此 quirk，前端可以把"连续多条用户消息合并展示"作为 UX 处理。

### 6.4 fork / independent sub-context 归属

fork 出 sub-context 时：

- 父 round 写 `Fork{child_label}` event
- sub-context 期间的 step/message append 继续写到**父 round** 的 jsonl，entry_seq 递增
- sub-context 结束写 `Join{child_label, outcome_kind}` event

不另起 round，因为从用户视角这仍是同一次触发的延续。如果将来需要展开 sub-context 树形结构，可以在每个 entry 上加 `sub_context_label: Option<String>` 字段（前向兼容，老 reader 忽略即可）。

---

## 7. 读取 API

### 7.1 Rust 同进程

```rust
struct SessionHistoryReader { session_dir: PathBuf }

impl SessionHistoryReader {
    fn open(session_dir: &Path) -> Result<Self>;
    fn list_rounds(&self, range: Option<Range<u64>>) -> Result<Vec<RoundSummary>>;
    fn read_round(&self, round_index: u64, view: HistoryView) -> Result<RoundView>;
    fn read_range(&self, from: u64, to: u64, view: HistoryView) -> Result<Vec<RoundView>>;
    fn latest_round_index(&self) -> Result<Option<u64>>;
}

struct RoundView {
    summary: RoundSummary,
    payload: RoundPayload,
}

enum RoundPayload {
    MsgOnly { messages: Vec<AiMessage> },       // 见 §4.3 还原算法
    Full(RoundFullPayload),
}

enum RoundFullPayload {
    Chat { messages: Vec<AiMessage> },
    Behavior { steps: Vec<StepRecord> },
}
```

Reader 与 Writer 不共享内存：每次调用都 cold-open `index.json` + 目标 jsonl。性能足够前端访问场景（O(round_size)）。需要更高吞吐时可以加 mmap / 内存 LRU，后置。

### 7.2 HTTP 接口（agent runtime 暴露）

```
GET /agents/{agent_id}/sessions/{session_id}/history?index={n}&type={msgonly|full}
    → 200 RoundView (JSON)
    → 404 round 不存在

GET /agents/{agent_id}/sessions/{session_id}/history?from={a}&to={b}&type={...}
    → 200 Vec<RoundView>

GET /agents/{agent_id}/sessions/{session_id}/history/index?from={a}&to={b}
    → 200 Vec<RoundSummary>           # 导航/分页

GET /agents/{agent_id}/sessions/{session_id}/history/latest
    → 200 { round_index: u64 }        # 配合 polling
```

参数默认值：`type=msgonly`；range 上限默认 50，调用方可加 `limit`。

`type` 字段允许扩展（后续添加 `raw` 暴露所有 Event entries），未识别值返回 400。

### 7.3 watch / 增量

v1 不做 server-push。前端 polling `/history/latest` + 按需拉新增 round。后续可加 SSE：

```
GET /agents/.../history/stream?since={n}
    → SSE: event=round_update, data={round_summary}
```

---

## 8. 与 accumulated / steps 的关系

| 属性 | history | accumulated | steps |
|---|---|---|---|
| 角色 | 真理源 | LLM 上下文窗口（chat 模式） | LLM 上下文窗口（behavior 模式） |
| 可变性 | append-only | 可压缩、可重写 | 可压缩、可重写 |
| 持久化 | `.history/` | `.meta/state.snap` | `.meta/state.snap` |
| 范围 | 全部历史 | 滑动窗口 | 滑动窗口 |
| 反序列化 | reader 独立 | 由 LLMContext 加载 | 由 LLMContext 加载 |

**重建关系**（可选，后续阶段）：

- `accumulated` / `steps` 可以从 `history` 的 Full 视图重建
- 这样 `state.snap` 退化为 cache，丢失也能恢复（按 token budget 截断）
- v1 不实现重建，仍依赖 `state.snap` 做活态恢复；history 只负责"读"的真理源

**压缩仍在原位**：`compress()` / `maybe_compress` 不变，只是新增一个 `append_event(Compaction)` 副作用。压缩**不再删除任何历史**。

---

## 9. 边界与已知 quirk

1. **多 user msg 聚合**：见 §6.3
2. **WaitingInput 的延续归属**：用户在 round N 触发，LLM 返回 PendingTool，外部 tool 异步回调到达时仍归 round N（status 仍为 Open / WaitingInput，append 续编 seq）。直到 Outcome::Done 才 finalize。这是与"每条用户消息一个 round"一致的延伸
3. **fork sub-context**：见 §6.4，归属父 round，加 Fork/Join 事件
4. **Compaction 不进 msgonly/full 视图**：审计需要的话走 `type=raw`（后续添加）
5. **没有 user msg 的纯系统触发**：开 SystemEvent round，trigger.preview 留空 / 用 source 字段
6. **崩溃恢复**：见 §5.4
7. **大 round**：单 round 内 entries 数量上限不设硬限。预期一个 behavior 跑 100+ step 是可能的，jsonl 单文件几 MB，reader 一次性读入 OK。极端情况下可在 round 级别做分片（rounds/000001.0.jsonl / 000001.1.jsonl），v1 不做
8. **mode 不可变**：session 创建时确定 chat / behavior，round 的 `mode` 字段冗余存方便 reader 直接判断；不支持中途切换

---

## 10. 实施路线

### Phase 1：写入与读取（独立 PR）

1. 新建 crate `session_history`（或放在 opendan 内的 `history` 模块），定义 RoundSummary / Entry / Writer / Reader
2. `SessionHistoryWriter` 实现：begin / append_message / append_step / append_event / finalize；jsonl + index.json 持久化与崩溃兜底
3. `SessionHistoryReader` 实现：list_rounds / read_round / read_range；MsgOnly 还原算法
4. AgentSession 接入：字段、`enqueue_pending` 钩子、`run_one_turn` 前后 diff、interrupt 钩子、compress 钩子
5. 旧 session 兼容：reader 看到无 `.history/` 时返回空，由调用方决定是否走 `accumulated` fallback
6. 单元测试：chat / behavior 各跑一遍，含 compaction、interrupt、fork、多 user msg 聚合

### Phase 2：HTTP 暴露

1. 在 opendan runtime 加 router，`/agents/{aid}/sessions/{sid}/history*` 一组端点
2. 鉴权对齐现有 RPC 体系
3. SSE / polling 决策（依据前端需求）

### Phase 3（可选）：accumulated 重建

1. 从 history Full 视图按 token budget 截断重建 `accumulated` / `steps`
2. 把 `state.snap` 标记为可丢弃 cache
3. 影响：冷启动开销变大，但崩溃恢复 100% 完整

---

## 11. 迁移说明

- beta2.2 允许 breaking，不做双写
- 旧 session 没有 `.history/`：reader 返回空；前端 UI 展示"无完整历史，仅可读取最近上下文窗口"，由用户选择是否 fallback 读 `accumulated`
- 新 session 一开始就走 history，state.snap 仍存在但**不再是历史**

---

## 12. 开放问题

- **fsync 频率**：每 entry / 每 round / 由配置控制？v1 默认每 round close。生产环境若发现崩溃丢尾，再加强
- **index.json rewrite vs append-only `index.jsonl`**：round 数量预期百-千级，rewrite 简单且 reader 不用扫整段 jsonl 建索引。v1 用 rewrite
- **MsgOnly 在 behavior 模式下的还原算法**：§4.3 给出一个候选；实施前需要拉前端一起 review，确认 thought / observation 在 UI 上怎么呈现
- **fork sub-context 是否要展开成树**：v1 平铺在父 round 内；若前端需要折叠展示，加 `sub_context_label` 字段即可
- **多 agent 共享 session**：现在不支持，将来若引入，需要在 RoundSummary 加 `actor: AgentId`，写入侧 begin_round 时传入
- **保留策略**：是否设上限自动归档老 round？v1 不做；将来加 `archive/` 目录 + 配置阈值

---

## 13. 关键文件与位置（实施时回查）

| 模块 | 路径 |
|---|---|
| AgentSession 主体 | `src/frame/opendan/src/agent_session.rs` |
| enqueue_pending | `agent_session.rs:535` |
| interrupt | `agent_session.rs:559` |
| run_worker / run_one_turn | `agent_session.rs:634-824` |
| compress 调用点 | `agent_session.rs:1583-1633` |
| state.snap 持久化 | `agent_session.rs:1521-1563` |
| LLMContextState | `src/frame/llm_context/src/state.rs:17-90` |
| LLMContextSnapshot | `state.rs:87-90` |
| StepRecord / LLMBehaviorResult | `src/frame/llm_context/src/behavior_loop.rs:14-46` |
| sediment | `src/frame/llm_context/src/context_loop.rs:841-882` |
| run_behavior / run_inner | `context_loop.rs:586-758` |
| LLMContextOutcome | `src/frame/llm_context/src/outcome.rs:89-129` |
| Observation | `src/frame/llm_context/src/observation.rs:11-32` |
| AiMessage / AiContent | `src/frame/buckyos-api/src/aicc_client.rs:322-436` |
| llm_compress | `src/frame/agent_tool/src/llm_compress.rs:93` |
