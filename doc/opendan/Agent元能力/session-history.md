# Session History 设计

> 状态：设计草案 / beta2.2
> 范围：AgentSession 的对话与行为历史；与 llm_context 解耦但语义对齐
> 目标：提供一个**不被压缩破坏**、以文件系统为主接口、可由任意模块按 round 索引读取的历史视图

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
3. **Round 为对外索引单元**。worker 实际消费一批 pending input 并驱动一次 `LLMContext` run / resume = 一个 round，由 `round_index: u64` 标识。任何外部模块都用 round_index 寻址
4. **Round 内 entry_seq 平铺**。不引入 step / turn 的二级索引，所有事件按出现顺序编号
5. **写入低耦合但不靠隐式 diff 猜语义**。AgentSession 在已有边界点调 writer；chat 终态 assistant 输出、behavior hot `last_step`、PendingTool resume 等必须作为显式写入对象处理
6. **Reader 与 Writer 解耦**。任何模块/进程能独立打开 reader 读 `{session_dir}/round_history/`，不依赖 AgentSession 运行态、不持锁
7. **文件系统优先，HTTP 只做基础语义封装**。`round_history/` 与 `.meta/round_logs.jsonl` 是主接口；HTTP 只提供 list / read / latest 这类薄封装，不在 v1 承担复杂查询、聚合、watch
8. **round_history 是显式资产，但写入权归 HistoryWriter**。`{session_dir}/round_history/` 直接挂在 session 工作区下（非隐藏目录），属于 session 的可读取资产：
   - LLM 自身可通过通用文件工具列目录 / 读单个 round 文件做"自我回忆"
   - 工具 / 子 agent / 外部进程可以直接按文件路径引用，不需要走 RPC
   - 内部索引 / 状态机用的元数据（`round_logs.jsonl`）放在 `{session_dir}/.meta/` 下与资产分开，避免污染 LLM 视野
   - 文件工具应把 `round_history/` 视为只读或受保护路径；审计可信度来自“只有 HistoryWriter 追加写”，不能允许普通 LLM 工具改写历史

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
| sediment (last_step → steps) | `context_loop.rs:978-984` | 写入时机：上层必须记录“本次 run 新产生的完整 StepRecord 集合”，包括最终仍在 `last_step` 的终态 step |
| chat 模式 accumulated 追加 | `run_inner` | 写入时机：记录新增 tool_use / tool_result；最终 assistant 文本从 `LLMContextOutcome::Done.response/output` 显式补写 |
| fork / independent sub-context | `context_loop.rs:633-647` | sub-context 独立产生自己的 LLMContext 对象；history 视角下它属于 **同一 round 内的延续**，sub-context 产出的 entries 续编 entry_seq |

**关键决定**：history entry 是**模式相关的载荷**。Chat 模式记录 `AiMessage`，behavior 模式记录 `StepRecord`，而不是统一成"反正都渲染成 AiMessage"。原因：

- StepRecord 含有 XmlStepParser 解析后的结构化字段（thought / action / observation / next_behavior），渲染成 AiMessage 后这些结构被序列化进文本就**回不来**了
- 审计 / 回放 / 前端展示都需要这些结构（前端要分别渲染思考与动作）
- 模式由本次 round 的 behavior 决定（`deps.result_parser` 是否存在）。session 可能通过 `switch_behavior` 切换模式，因此 `mode` 是 round / entry 的属性，不能假设整个 session 同质

---

## 4. 概念定义

### 4.1 Round

```rust
struct RoundSummary {
    schema_version: u32,                    // v1 = 1
    round_index: u64,                     // per-session 自增，从 1
    trigger: RoundTrigger,
    input_keys: Vec<String>,                // 本 round 实际消费的 PendingInput.dedup_key()
    started_at: DateTime<Utc>,
    ended_at: Option<DateTime<Utc>>,
    status: RoundStatus,
    entry_count: u32,
    mode: ContextMode,                    // Chat | Behavior，本 round 的模式
}

enum RoundTrigger {
    UserMsg { preview: String },          // 本批用户文本合并后的前 100 字预览
    SystemEvent { source: String, event_kind: String },  // 调度唤醒、外部 API、tool 异步回调等
    Mixed,                                // 同一批次里同时有 Msg 与 Event
    Resume,                               // 进程重启后继续未完成 round（兜底，可选）
}

enum RoundStatus {
    Open,                                 // 正在写入
    Completed,                            // LLMContextOutcome::Done
    Interrupted,                          // interrupt() 触发
    Errored,                              // LLMContextOutcome::Error / BudgetExhausted
    WaitingTool,                          // LLMContextOutcome::PendingTool 等异步等待
}
```

### 4.2 Entry

```rust
struct Entry {
    schema_version: u32,                  // v1 = 1
    seq: u32,                             // round 内自增
    ts: DateTime<Utc>,
    mode: ContextMode,                    // 冗余存储，方便直接读单个 jsonl 文件
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
    Event(HistoryEvent),                  // 控制面，MsgOnly / Full 视图过滤掉，Raw 保留
}

enum HistoryEvent {
    SystemInput { source: String, payload: serde_json::Value },  // PendingInput::Event 落地
    Outcome {                             // 每次 LLMContextOutcome 都记一条
        kind: OutcomeKind,                // Done | PendingTool | ContextLimitReached | BudgetExhausted | Error | Interrupted
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

`schema_version` 的 v1 策略：
- `schema_version=1` 同时写在 `RoundSummary` 与每条 `Entry` 上，reader 遇到缺失版本时可按 v1 草案兼容处理
- v1 只允许 additive 字段扩展；不得重命名 / 改变已有字段语义
- `AiMessage` / `StepRecord` 作为嵌入载荷原样 serde，History 自身只承诺外层 envelope 的兼容性；若上游载荷发生破坏性变更，需要 bump history schema 或提供迁移

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
    /// - Event entries 仍丢弃；需要审计 / 回放细节时读 Raw
    Full,

    /// 原始文件视图：
    /// - 返回 round_history/{round}.jsonl 中所有 Entry，包括 Event
    /// - 文件系统直接读取天然就是 Raw；Rust reader 提供 Raw 只是方便测试 / 内部调用
    Raw,
}
```

`MsgOnly` 在 behavior 模式下需要一个固定的"还原对话"算法。建议：
- 用户输入 → user 消息
- 每个 Step → 一对 `{role: assistant, content: [Text(thought + assistant_text 末段)]}` + `{role: tool, content: [Text(observation)]}`
- 这样 behavior 与 chat 在 MsgOnly 视图下对前端长得一样

---

## 5. 存储布局

两部分：**资产目录**（主要读取接口）与 **meta 索引**（内部导航）。

```
{session_dir}/
├── round_history/                # ★ 资产目录：每个 round 一个 jsonl 文件
│   ├── 000001.jsonl              #   append-only
│   ├── 000002.jsonl
│   └── ...
└── .meta/
    └── round_logs.jsonl          # ★ 索引：每个 round 一行 RoundSummary，append-only
```

设计意图：
- `round_history/` 是 session 资产和主读取接口，LLM / 工具 / 外部进程优先通过文件路径读取；命名 `000001.jsonl` 按 round_index 零填充 6 位，便于人眼和 LLM 排序
- `.meta/round_logs.jsonl` 是内部状态机用的导航索引，**不暴露给 LLM**。崩溃恢复 / reader list 用它，避免每次扫整个 `round_history/`
- 写权限只属于 `SessionHistoryWriter`；普通文件工具默认不应改写 `round_history/` 和 `.meta/round_logs.jsonl`

### 5.1 .meta/round_logs.jsonl

Append-only jsonl，每个 round 在 `finalize_round`（或状态变更）时**追加一行** `RoundSummary` 快照。同一 round 可能多次出现（先 Open，再 Completed），reader 取 round_index 最后一次出现为准。

```jsonl
{"schema_version":1,"round_index":1,"trigger":{"kind":"user_msg","preview":"帮我看下 .env 文件"},"input_keys":["msg:local-1"],"started_at":"2026-05-15T10:00:00Z","ended_at":"2026-05-15T10:00:08Z","status":"completed","entry_count":7,"mode":"chat"}
{"schema_version":1,"round_index":2,"trigger":{"kind":"system_event","source":"schedule","event_kind":"tick"},"input_keys":["event:/schedule/tick:42"],"started_at":"2026-05-15T10:05:00Z","ended_at":null,"status":"waiting_tool","entry_count":3,"mode":"behavior"}
```

Append-only 的好处：
- 写入是一次 `O_APPEND` write，无需先读全文件再 rewrite
- 进程崩溃时最多丢最后一行，前面所有 round 不受影响
- reader 一次顺序扫描即可构建 `HashMap<round_index, RoundSummary>`（取 last-write-wins）

### 5.2 round_history/000001.jsonl（chat 模式示例）

```jsonl
{"schema_version":1,"seq":1,"ts":"...","mode":"chat","payload":{"kind":"message","message":{"role":"user","content":[{"type":"text","text":"帮我看下 .env 文件"}]},"llm_call":null}}
{"schema_version":1,"seq":2,"ts":"...","mode":"chat","payload":{"kind":"message","message":{"role":"assistant","content":[{"type":"thinking","text":"..."},{"type":"tool_use","name":"read_file","input":{"path":".env"}}]},"llm_call":1}}
{"schema_version":1,"seq":3,"ts":"...","mode":"chat","payload":{"kind":"message","message":{"role":"tool","content":[{"type":"tool_result","content":"FOO=bar\n..."}]},"llm_call":1}}
{"schema_version":1,"seq":4,"ts":"...","mode":"chat","payload":{"kind":"message","message":{"role":"assistant","content":[{"type":"text","text":".env 里有 FOO=bar"}]},"llm_call":2}}
{"schema_version":1,"seq":5,"ts":"...","mode":"chat","payload":{"kind":"event","event":{"type":"outcome","kind":"done","behavior_result":null,"usage_delta":{"input_tokens":420,"output_tokens":58}}}}
```

### 5.3 round_history/000007.jsonl（behavior 模式示例）

```jsonl
{"schema_version":1,"seq":1,"ts":"...","mode":"behavior","payload":{"kind":"message","message":{"role":"user","content":[{"type":"text","text":"清理过期 session"}]}}}
{"schema_version":1,"seq":2,"ts":"...","mode":"behavior","payload":{"kind":"step","step":{"assistant_text":"<thought>...</thought><action>list_sessions</action>","thought":"...","action":"list_sessions","observation":null,"next_behavior":null,"action_result":{"kind":"success","output":"[...]"}},"llm_call":1}}
{"schema_version":1,"seq":3,"ts":"...","mode":"behavior","payload":{"kind":"step","step":{"assistant_text":"...","thought":"...","action":"delete","observation":"deleted 3 sessions","next_behavior":null,"action_result":{"kind":"success","output":"ok"}},"llm_call":2}}
{"schema_version":1,"seq":4,"ts":"...","mode":"behavior","payload":{"kind":"event","event":{"type":"compaction","target":"steps","dropped":5,"kept_head":0,"kept_tail":3,"summary_preview":"前期清理 5 个过期 session"}}}
{"schema_version":1,"seq":5,"ts":"...","mode":"behavior","payload":{"kind":"event","event":{"type":"outcome","kind":"done","behavior_result":{"do_actions":[],"next_behavior":null,"assistant_text":"清理完毕","thought":"...","observation":"deleted 3 sessions"}}}}
```

### 5.4 持久化保证

- **每条 entry**：append 到 `round_history/{round_index}.jsonl`，`write_all` 进 OS 缓冲，**不强制 fsync**
- **round close 时**：`finalize_round` 先 fsync 当前 round 的 jsonl，再 append 一行新 `RoundSummary` 到 `.meta/round_logs.jsonl` 并 fsync
- **状态更新**（如 Open → WaitingTool）：直接 append 一行新 RoundSummary 即可，无需重写老行
- **pending input 幂等**：`input_keys` 记录本 round 已消费的 `PendingInput.dedup_key()`。如果 turn 失败且 pending input 被保留重试，恢复逻辑必须能识别同一批 input 已经对应过一个未完成 round，避免重复开 round
- **崩溃恢复**：启动时扫描 `round_history/*.jsonl` 与 `round_logs.jsonl`：
  - `round_logs.jsonl` 末行残缺 → 丢弃
  - 某 round 在 `round_logs.jsonl` 最后一次状态是 Open / WaitingTool，但当前进程不是原 owner → 补一行 `status=Errored` 兜底（或保留 WaitingTool，由 AgentSession 决定是否 resume）
  - 存在 `round_history/{n}.jsonl` 但 `round_logs.jsonl` 完全没有 n 的记录 → 补一行 `Errored`
- **末行残缺**：reader 按行 deserialize，遇到 parse error **静默丢弃当前行**，下次 reader 调用自动看到完整数据

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
struct SessionHistoryWriter {
    /* round_logs.jsonl 句柄 + 内存里的 round_index → RoundSummary 缓存
       + 当前 Open round 的 jsonl 句柄 + entry_seq 计数器 */
}

impl SessionHistoryWriter {
    async fn open(session_dir: &Path) -> Result<Self>;
    async fn begin_round(
        &mut self,
        trigger: RoundTrigger,
        input_keys: Vec<String>,
        mode: ContextMode,
    ) -> Result<u64>;
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
| worker drain pending | `run_worker` drain 出 `turn_inputs` 后、`run_one_turn` 前 | 用本批 `consumed_keys` / event keys `begin_round(...)`；不要在 `enqueue_pending` 时开 round |
| 系统事件进入 turn | `PendingInput::Event` 被 drain 并格式化后 | `append_event(SystemInput)`；若与用户消息同批则仍属于同一个 round |
| chat 模式新增消息 | `run_one_turn` 前后 + outcome | 记录本 run 新增的 tool_use / tool_result；`Outcome::Done.response/output` 显式补写最终 assistant message |
| behavior 模式新增 step | `run_one_turn` 前后 + final_snapshot | 记录本 run 新增的 `steps`，并补写最终 `last_step`，不能只 diff `steps.len()` |
| Outcome 产出 | `run_one_turn` 处理 LLMContextOutcome 时 | `append_event(Outcome{...})` |
| 压缩 | `agent_session.rs:1583-1633` 的 `compress()` 调用点 / `maybe_compress` 后 | `append_event(Compaction{...})` |
| Interrupt | `execute_interrupt` 实际处理 barrier 时 | `append_event(Interrupt{...})` → `finalize_round(Interrupted)` |
| Fork / Join | sub-context 进入/退出 | `append_event(Fork/Join)` |
| Round 终结 | `run_one_turn` 返回时（Outcome 已记录后） | `finalize_round(Completed/WaitingTool/Interrupted/Errored)` |

**llm_context 不感知 history**：v1 钩子仍放在 agent_session 层，但不能只靠 `state.accumulated.len()` / `state.steps.len()` diff 完成写入：

- chat：`finish_done()` 产出的最终 assistant 文本在 `Outcome::Done.response/output` 中，不保证已进入 `accumulated`，必须从 outcome 显式 append
- behavior：`sediment()` 会把旧 `last_step` 推入 `steps`，新 step 留在 `last_step`；终态 step 必须从 `final_snapshot.state.last_step` 或 outcome 的 `behavior_result` 显式 append
- PendingTool resume / graceful interrupt 会通过 `ResumeFill::ToolResults` 追加 tool observations，需要把 resume 前后的新增 message / step 也归入原 WaitingTool round

如果后续发现 agent_session 层补写逻辑过脆，再引入 `llm_context` 的 `StateListener`；v1 先保持低侵入，但测试必须覆盖上述三个漏写点。

### 6.3 pending input 与 round 幂等

`enqueue_pending` 只负责把 input 持久化到 `.meta/session.json` 并唤醒 worker，不创建 round。原因：

- 现有 worker 在 turn 成功后才 `discard_consumed`
- turn 失败 / 进程崩溃时 pending input 会保留并重放
- 如果入队时就开 round，会产生重复 round 或只有 input、没有 LLM 输出的假历史

round 创建点是 worker 确定要消费一批 input 并即将调用 `run_one_turn` / `resume_with_tool_results` 的时刻。writer 写入 `input_keys`，恢复时用它判断是否已经存在对应未完成 round。

### 6.4 多 user msg 聚合的归属（quirk）

worker 在 `run_one_turn` 前会 drain `pending_inputs` 一批输入。如果一次拿到多条 user msg：

- 同一批输入只开一个 round
- 每条 user msg 都作为独立 `Message{role:user}` entry 写入，`RoundSummary.trigger.preview` 只保存合并预览
- LLM 实际只跑一次（一次 turn），新增的 step/message 归属这个 round

文档化此 quirk，前端可以把"连续多条用户消息合并展示"作为 UX 处理。

### 6.5 fork / independent sub-context 归属

fork 出 sub-context 时：

- 父 round 写 `Fork{child_label}` event
- sub-context 期间的 step/message append 继续写到**父 round** 的 `round_history/{n}.jsonl`，entry_seq 递增
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
    Raw { entries: Vec<Entry> },
}

enum RoundFullPayload {
    Chat { messages: Vec<AiMessage> },
    Behavior { steps: Vec<StepRecord> },
}
```

Reader 与 Writer 不共享内存：每次调用都 cold-open `.meta/round_logs.jsonl` 顺序扫一遍构建 `HashMap<round_index, RoundSummary>`（last-write-wins），再按需打开 `round_history/{n}.jsonl`。性能足够前端访问场景（O(round_count) 索引扫描 + O(round_size) 读取）。需要更高吞吐时可以加 mmap / 内存 LRU / inotify 增量更新，后置。

### 7.2 HTTP 接口（agent runtime 暴露）

HTTP 不是主查询面，只做文件系统 reader 的基础语义封装，方便前端和不能直接读 session 目录的调用方。v1 不做复杂过滤、聚合、server-push 或跨 session 搜索。

```
GET /agents/{agent_id}/sessions/{session_id}/history?index={n}&type={msgonly|full|raw}
    → 200 RoundView (JSON)
    → 404 round 不存在

GET /agents/{agent_id}/sessions/{session_id}/history?from={a}&to={b}&type={msgonly|full}
    → 200 Vec<RoundView>

GET /agents/{agent_id}/sessions/{session_id}/history/index?from={a}&to={b}
    → 200 Vec<RoundSummary>           # 导航/分页

GET /agents/{agent_id}/sessions/{session_id}/history/latest
    → 200 { round_index: u64 }        # 配合 polling
```

参数默认值：`type=msgonly`；range 上限默认 50，调用方可加 `limit`。`raw` 只要求支持单 round 读取，range raw 可以后置，避免一次 HTTP 拉出大量 tool_result / provider_state。

`type` 字段允许扩展，未识别值返回 400。

### 7.3 watch / 增量

v1 不做 server-push。前端 polling `/history/latest` + 按需拉新增 round。后续如果前端明确需要，再加 SSE：

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
| 持久化 | `round_history/` + `.meta/round_logs.jsonl` | `.meta/state.snap` | `.meta/state.snap` |
| LLM 可见 | ✅ 只读资产目录 | ❌ 内部状态 | ❌ 内部状态 |
| 范围 | 全部历史 | 滑动窗口 | 滑动窗口 |
| 反序列化 | reader 独立 | 由 LLMContext 加载 | 由 LLMContext 加载 |

**重建关系**（可选，后续阶段）：

- `accumulated` / `steps` 可以从 `history` 的 Full 视图重建
- 这样 `state.snap` 退化为 cache，丢失也能恢复（按 token budget 截断）
- v1 不实现重建，仍依赖 `state.snap` 做活态恢复；history 只负责"读"的真理源

**压缩仍在原位**：`compress()` / `maybe_compress` 不变，只是新增一个 `append_event(Compaction)` 副作用。压缩**不再删除任何历史**。

---

## 9. 边界与已知 quirk

1. **多 user msg 聚合**：见 §6.4
2. **WaitingTool 的延续归属**：用户在 round N 触发，LLM 返回 PendingTool，外部 tool 异步回调到达时仍归 round N（status 仍为 Open / WaitingTool，append 续编 seq）。直到 Outcome::Done / Error / Interrupted 才 finalize
3. **fork sub-context**：见 §6.5，归属父 round，加 Fork/Join 事件
4. **Compaction 不进 msgonly/full 视图**：审计需要走 `Raw` / 直接读文件
5. **没有 user msg 的纯系统触发**：开 SystemEvent round，trigger.preview 留空 / 用 source 字段
6. **崩溃恢复**：见 §5.4
7. **大 round**：单 round 内 entries 数量上限不设硬限。预期一个 behavior 跑 100+ step 是可能的，jsonl 单文件几 MB，reader 一次性读入 OK。极端情况下可在 round 级别做分片（`round_history/000001.0.jsonl` / `000001.1.jsonl`），v1 不做
8. **mode 可随 round 变化**：session 可通过 `switch_behavior` 进入不同模式。reader 不能假设 session 同质，必须按 `RoundSummary.mode` / `Entry.mode` 判断
9. **资产可读但不可随意写**：`round_history/` 是给 LLM / 工具读取的资产，不是普通工作文件。写入必须走 HistoryWriter，否则审计和恢复语义不成立

---

## 10. 实施路线

### Phase 1：写入与读取（独立 PR）

1. 新建 crate `session_history`（或放在 opendan 内的 `history` 模块），定义 RoundSummary / Entry / Writer / Reader
2. `SessionHistoryWriter` 实现：begin / append_message / append_step / append_event / finalize；`round_history/*.jsonl` + `.meta/round_logs.jsonl` 双 append-only 持久化与崩溃兜底
3. `SessionHistoryReader` 实现：list_rounds / read_round / read_range；MsgOnly 还原算法
4. AgentSession 接入：字段、worker drain 时 begin_round、`run_one_turn` / `resume_with_tool_results` 显式补写新增 message/step、interrupt 钩子、compress 钩子
5. 旧 session 兼容：reader 看到无 `round_history/` 时返回空，由调用方决定是否走 `accumulated` fallback
6. 单元测试：chat / behavior 各跑一遍，含最终 assistant 输出补写、behavior `last_step` 补写、PendingTool resume、compaction、interrupt、fork、多 user msg 聚合、失败重试不重复开 round

### Phase 2：HTTP 暴露

1. 在 opendan runtime 加基础 router，`/agents/{aid}/sessions/{sid}/history*` 一组端点
2. 鉴权对齐现有 RPC 体系
3. 不做复杂查询与 watch；前端先用 `latest` + 单 round 拉取

### Phase 3（可选）：accumulated 重建

1. 从 history Full 视图按 token budget 截断重建 `accumulated` / `steps`
2. 把 `state.snap` 标记为可丢弃 cache
3. 影响：冷启动开销变大，但崩溃恢复 100% 完整

---

## 11. 迁移说明

- beta2.2 允许 breaking，不做双写
- 旧 session 没有 `round_history/`：reader 返回空；前端 UI 展示"无完整历史，仅可读取最近上下文窗口"，由用户选择是否 fallback 读 `accumulated`
- 新 session 一开始就走 history，state.snap 仍存在但**不再是历史**

---

## 12. 开放问题

- **fsync 频率**：每 entry / 每 round / 由配置控制？v1 默认每 round close 时对 `round_history/{n}.jsonl` 和 `round_logs.jsonl` 各 fsync 一次。生产环境若发现崩溃丢尾，再加强
- **round_logs.jsonl 压缩**：append-only 长期可能膨胀（同 round 多次状态更新会堆叠）。round 数量到达某阈值时离线 compact 成新文件（保留每个 round_index 的 last record），v1 不做
- **MsgOnly 在 behavior 模式下的还原算法**：§4.3 给出一个候选；实施前需要拉前端一起 review，确认 thought / observation 在 UI 上怎么呈现
- **LLM 访问 round_history 的工具表面**：v1 按文件系统优先，先用通用只读文件工具。是否增加 `recall_round(index, view)` 取决于实际使用效果
- **fork sub-context 是否要展开成树**：v1 平铺在父 round 内；若前端需要折叠展示，加 `sub_context_label` 字段即可
- **多 agent 共享 session**：现在不支持，将来若引入，需要在 RoundSummary 加 `actor: AgentId`，写入侧 begin_round 时传入
- **保留策略**：是否设上限自动归档老 round？v1 不做；将来加 `archive/` 目录 + 配置阈值

---

## 13. 关键文件与位置（实施时回查）

| 模块 | 路径 |
|---|---|
| AgentSession 主体 | `src/frame/opendan/src/agent_session.rs` |
| enqueue_pending | `agent_session.rs:586` |
| interrupt | `agent_session.rs:629` |
| run_worker / run_one_turn | `agent_session.rs:703-1122` / `agent_session.rs:1646` |
| compress 调用点 | `agent_session.rs:1672-1722` |
| state.snap 持久化 | `agent_session.rs:1598-1644` |
| LLMContextState | `src/frame/llm_context/src/state.rs:17-90` |
| LLMContextSnapshot | `state.rs:87-90` |
| StepRecord / LLMBehaviorResult | `src/frame/llm_context/src/behavior_loop.rs:14-46` |
| sediment | `src/frame/llm_context/src/context_loop.rs:978-984` |
| run_behavior / run_inner | `context_loop.rs:700-871` / `context_loop.rs:205-508` |
| LLMContextOutcome | `src/frame/llm_context/src/outcome.rs:89-129` |
| Observation | `src/frame/llm_context/src/observation.rs:11-32` |
| AiMessage / AiContent | `src/kernel/buckyos-api/src/aicc_client.rs:322-436` |
| llm_compress | `src/frame/agent_tool/src/llm_compress.rs:93` |
