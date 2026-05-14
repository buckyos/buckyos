# new opendan agent runtime

> 重构目标：把 opendan 从「自己写 Agent Loop / Behavior 解析 / step 记录」改造成
> 「**只负责构造正确的 LLMContextRequest + 正确的 LLMContextDeps，调度 LLMContext.run() / resume()，并消化 Outcome**」。
>
> 真正的 LLM 推理循环、tool dispatch、step 记录、错误自动反馈、快照/resume，
> 全部下沉到 `llm_context` crate（slim-waist 已经实现）。
> opendan 是这个 waist 之上的 L3/L4 调度器 + 持久化层。

## 核心架构原则（必须遵守）

1. **外部边界 = buckyos-api**：从系统获取 Message / Event 一律走 `buckyos-api` 类型
   （`MsgCenterClient` / `KEventClient`），不绕开 SDK，不自己造跨进程协议。
2. **内部 dispatch = tokio 队列基础设施**：进程内的输入传递、worker 唤醒、shutdown 信号一律
   用 tokio 原语（`mpsc` / `Notify` / `select!`），不自己造调度器。
3. **AgentSession 是状态管理的核心**：任何"已经从系统取走、但还没被 LLM 真正消费"的 msg / event
   必须落到 `AgentSession.meta.pending_inputs` 持久化字段里；session 自己在 worker loop 的合适
   状态下从 pending queue 取出消费。
4. **ack 上游 ↔ session 持久化对齐**：
   - 落盘前进程崩 → msg-center 仍是 `Reading` → 下次 boot 重新拉同一条
   - 落盘成功 → 立刻 `update_record_state(Readed)` ack 给 msg-center
   - 这条 invariant 决定了 pump / dispatcher / session 三方的分工（见 §4 伪代码）。

## 0. 与新 llm_context 的接口契约

opendan 通过两个对象与 waist 交互：

- **`LLMContextRequest`**（不可变输入，[request.rs](src/frame/llm_context/src/request.rs)）
  - `owner: ContextOwnerRef::Agent { session_id }`
  - `input: Vec<AiMessage>` — 已渲染的完整对话（system+user+history），不再有"模板"概念到达 waist
  - `model_policy / tool_policy / output / budget / human_policy / error_policy`
  - `tool_policy.mode` + `whitelist` 决定该步允许的工具集（用于 behavior 切换时收窄/放开权限）

- **`LLMContextDeps`**（运行依赖，[deps.rs](src/frame/llm_context/src/deps.rs:195)）
  - `llm: LlmClient` — 由 `aicc_client` 适配
  - `tools: ToolManager` — 由 opendan 的 `AgentToolManager` 适配（4 层 bin 合成在这里完成）
  - `policy: PolicyEngine` — `AgentPolicy`：基于 behavior_cfg 做 gate
  - `worklog: WorklogSink` — opendan 把 `WorkEvent` 翻译成 `WorklogService` 写盘 + 更新 session "一句话状态"
  - `tokenizer: Tokenizer` — 选 ByteHeuristic 起步
  - `turn_hook: TurnHook` — **每次推理前**回调，opendan 在这里把 `LLMContextSnapshot` 落盘到 `session/.meta/$state`
  - 三个可选项决定是「Agent Loop」还是「Behavior Loop」：
    - `result_parser: LLMResultParser` — opendan 的 `XmlBehaviorParser`（待实现，对应空的 `xml_behavior.rs`）
    - `step_renderer: StepRenderer` — 把 `StepRecord` 还原成 `(assistant, user)` 对喂回下一轮推理
    - `history_compressor: HistoryCompressor` — 可选，长 step 历史压缩

**结论：opendan 不再实现 LLM 循环本身**，只实现这 7 个 trait 的"opendan 风味"版本，以及围绕 session 的调度。

---

## 1. 状态分层

### AgentRuntime（进程级，单例）

waist 的 deps 公共依赖 + 边界客户端：
- `aicc_client: Arc<AiccClient>` — 适配为 `LlmClient`
- `worklog: Arc<WorklogService>` — 全局 SQLite 句柄
- `msg_center: Option<Arc<MsgCenterClient>>` — 边界：msg-center get_next / update_record_state
- `kevent_client: Option<Arc<KEventClient>>` — 边界：订阅 `/msg_center/{owner}/box/**` 等模式
- `contact_mgr`（TODO）— 给 forward_msg / forward 类工具用
- `task_mgr`（TODO）— 异步工具结果回填、跨 session 任务通知

`msg_center` / `kevent_client` 为 `Option` 是为了让 CLI / 单测可以在不连 zone 服务的情况下
跑 AIAgent — 这时 inbox 退化成「只接受 `submit_text` 注入」模式（见 §9.6 进度）。

### Agent（AgentRootFS，对齐 paios 容器需求 §9）

Agent Root 位置按 paios 契约：`/opt/buckyos/data/home/$userid/.local/share/$agentid/`（Instance Volume）。
目录内布局（Agent Bin 层落在这里）：

```
/role.md + /self.md                      # 自我介绍，进 system prompt
/users/$user_id.md | group_$gid.md       # 针对调用者的系统提示词片段
/memory/                                 # AgentMemory 模块初始化
/notepads/$notepadname/                  # 多本 notepad，AgentMemory 初始化
/skills/$category/$skill_dir/            # Agent 加载的真实 skills（可 self-improve）
/tools/                                  # Agent 自写脚本工具（§2 4 层 bin 中的 Agent Bin 层）
/behaviors/$name.toml                    # Behavior 模板（系统提示词 + 允许工具 + parser/renderer 配置）
/archive/skills                          # 导入原始 skills，Agent 不直接看
/archive/sessions/$session_id            # 已归档 session
/archive/workspace/$workspace_id         # 已归档 workspace
/archive/worklog.db                      # SQLite 归档
/workspace/$workspace_id/                # 工作区目录
/workspace_list.md                       # 最近活跃 workspace 列表，有大小上限
/sessions/$session_id/                   # session 目录（含 .meta/session.json、.meta/state.snap）
```

**Session Bin 层不在 Agent Root 内**——按 paios 契约落在 `/opt/buckyos/tools/$agentid/$sessionid/`
（rwx 卷，session 启动时按权限渲染）。System / Runtime Bin 层也在 `/opt/buckyos/tools/` 下
（`store/` + `bin/`），见 §2 与 §9.2 残项。这是相对旧版 opendan 把 session-bin 放在
`session/<sid>/.tool` 的破坏性路径变化。

### Workspace（local）

代表 Agent 的私有工作区。**workspace 优先拥有 task**（task 跟着 workspace 走，session 只是其执行载体）。

```
./.workspace.json     # 结构化状态，含与 session 的绑定关系
./readme.md           # 目录结构说明，会作为环境上下文片段进入提示词
```

参考现有 `LocalWorkspaceManager` ([local_workspace.rs](src/frame/opendan/src/local_workspace.rs))；
新 runtime 保留其数据模型（`WorkshopWorkspaceRecord` / `SessionWorkspaceBinding`），
但 session 绑定改由 `AgentSession` 自己持有引用，不再走全局 mgr 的 in-memory 快照。

### AgentSession

```
./.meta/session.json   # session 元信息：id / agent_did / owner / current_behavior /
                       #   status / one_line_status / pending_inputs[]
./.meta/state.snap     # 最新 LLMContextSnapshot（由 turn_hook 写入）
./.meta/state.$N.snap  # 历史快照，按 behavior 切换时归档
./readme.md            # session 目录说明，进环境上下文
./bin/                 # session 级别 binary，软链接 + 脚本
./report.md            # worksession 完成后的工作报告
./archive/             # 完整 history（包括 worklog 子集），可翻看
```

`pending_inputs` 是「核心原则 #3」落在持久化层的字段，存的是 `enum PendingInput { Msg, Event }`
（见 [agent_session.rs](src/frame/opendan/src/agent_session.rs) `PendingInput`）。
写入路径走 `AgentSession::enqueue_pending(input)`：append → `flush_meta()`（tmp + rename
的 crash-consistent 写法）→ 唤醒 worker。worker 在 turn 成功后才从 pending_inputs 里删除已消费项
并 flush_meta，失败则保留以便重启 / 下次唤醒重放（at-least-once）。

**Session 类型**：
- **UI Session**：永远活跃，每个 UI tunnel 对应一个；天然带 `try_create_worksession` / `forward_msg` 等工具
- **Work Session**：状态机，非 END 状态下都算活跃；由 UI session 用 `try_create_worksession` 派生

---

## 2. AgentTool 4 层合成

`AgentToolManager::list_tool_specs()` 返回的工具集是 4 层合并的结果（同名后者覆盖前者）：

| 层 | 范围 | 来源 | 权限 |
|----|------|------|------|
| System Bin | 所有 Agent 可见 | BuckyOS 发行镜像 | 只读 |
| Runtime Bin | 特定 Agent 可见 | 用户安装到 Agent 工具卷的二进制 | 通常只读，按权限放开 |
| Agent Bin | 特定 Agent 可见 | Agent 自己写的脚本（在 `/tools/`） | Agent 可修改 |
| Session Bin | 特定 session 可见 | Session 启动时按权限创建（软链接 + 脚本） | Session 内可修改 |

合成发生在 **`AgentToolManager` 构造 / 每次 session 启动** 时，结果缓存在 manager 内部，
对 waist 暴露统一的 `ToolManager` 接口。`tool_policy.whitelist` 在 behavior 切换时控制可见子集（不重新合成，只 gate）。

**4 层 bin 的物理路径契约**（来自 [paios 容器需求.md §9](paios容器需求.md)）：

| 层 | 路径 | 权限 | 承载 |
|----|------|------|------|
| System Bin | `/opt/buckyos/tools/store/` | rx，所有 App 共享 | Worker Image 预置 CLI（ffmpeg/pandoc/...） |
| Runtime Bin | `/opt/buckyos/tools/bin/` | rx，App-scoped symlink view | 从 `store/` + ExtTool Volume 渲染（Crafter 镜像产出工具包接入处） |
| Agent Bin | Agent Root 下 `/tools/`（Instance Volume 内） | rwx 给 Agent | Agent 自演化脚本；升级走文件级合并（paios §7.4 R-15） |
| Session Bin | `/opt/buckyos/tools/$agentid/$sessionid/` | rwx | session 启动时按权限创建 |

PATH overlay 顺序：**Session > Agent > Runtime > System**（前者优先，同名覆盖）。

**UI Session 默认工具集**（写入 `behaviors/ui_default.toml` 的 whitelist）：
- `exec_bash` / `read_file` / `glob` / `grep` / `edit_file` / `write_file`
- `try_create_worksession { reason }` — fork 出 sub-LLMContext，基于近况和 worksession
  列表决定复用已有 / 新建。最终由 sub-context 调 `create_worksession` 落地，结果原样回传给
  UI session。详见 §8.1–§8.3。
- `forward_msg { target_worksession_id }` — 把"触发本轮推理的最近 user 消息"作为
  `PendingInput::Msg` 派发到指定 worksession（进程内路由，不走 msg-center）。详见 §8.4。
- `update_session_tags` — 主动触发一次 memory 召回

---

## 3. Behavior Config

每个 behavior 是一份配置（建议 TOML，落在 `/behaviors/$name.toml`）：

```toml
name = "ui_default"
system_prompt_template = "..."        # 引用 role.md / self.md / users/*.md 的渲染模板
tool_whitelist = ["exec_bash", "read_file", "try_create_worksession", "forward_msg"]

# Behavior 模式（决定 LLMContextDeps 是否装 parser/renderer）
mode = "behavior"                     # "agent" | "behavior"
                                      # agent: 走传统 Agent Loop（provider 原生 tool_calls）
                                      # behavior: 装上 parser+renderer，走 Behavior Loop

# Parser / Renderer 选择（mode = "behavior" 时生效）
parser = "xml"                        # 默认 "xml" → llm_context::XmlBehaviorParser
renderer = "xml"                      # 默认 "xml" → llm_context::XmlStepRenderer
parser_strict = false                 # XmlBehaviorParser.strict：true 时纯文本回复算错误

# Renderer 调参（XmlStepRenderer 的旋钮，全部可选）
renderer_recent_full_steps = 2        # 最近 N 步全量渲染；更老的步骤压缩
renderer_summary_chars = 280          # 压缩步骤的 assistant_text 字符上限
renderer_max_result_chars = 4096      # 单步 action_result 上限（0 = 不截断）

# 输出与预算
output = "text"                       # "text" | "json"（json 时需在 output_spec 里给 schema）
max_rounds = 16                       # ToolPolicy.max_rounds
max_consecutive_errors = 3            # ErrorPolicy.max_consecutive_errors

# Behavior 切换语义
switch_mode = "normal"                # "normal" | "fork" | "independent"
```

opendan 的 `agent_config` 负责把这份 TOML 翻译成 `LLMContextRequest` + `LLMContextDeps`
（往 `deps.result_parser` / `deps.step_renderer` 装入对应实现）。当 `mode = "behavior"` 时，
默认装 `Arc<XmlBehaviorParser>` 和 `Arc<XmlStepRenderer>`（已实现，见下）。

### 默认 XML Behavior 协议（已实现）

系统默认实现已落在 `llm_context` crate 里，opendan 直接 `use` 即可：

- [`llm_context::XmlBehaviorParser`](src/frame/llm_context/src/xml_behavior.rs) — `impl LLMResultParser`
- [`llm_context::XmlStepRenderer`](src/frame/llm_context/src/step_record.rs) — `impl StepRenderer`

**LLM 输出的 wire format**（每个 tag 都可选；外层 `<response>` 也可选，存在时收窄扫描范围）：

```xml
<response>
  <thinking>...自由形式推理...</thinking>
  <observation>...对上一步 action_result 的解读...</observation>
  <action tool="exec_bash" call_id="optional">
    {"command": "ls -la"}
  </action>
  <next_behavior>END</next_behavior>
</response>
```

`<action>` 体的解析规则：
- Body 解析为 JSON 对象 → 作为 `args`
- 否则 → `args["content"] = body`（保持原文）
- 非保留属性（`tool` / `name` / `call_id` 之外）作为字符串 args 注入
- Provider 原生 `tool_calls`（function-calling）优先于 `<action>` 扫描

**强容错**（无需额外旁路 LLM 修复就能 cover 大多数 case）：
- 剥离 ```` ```xml ```` / ```` ``` ```` markdown fence
- 缺失 close tag 时从开标签取到 EOF
- 属性值支持双引号/单引号/无引号；5 个 XML 实体（`&amp;` 等）会解码
- 整段 response 没有可识别结构时，`assistant_text` 原样保留，被当作"自然收敛终态步骤"
- 真正失败的（完全空 response，或 `parser_strict=true` 且既无 action 又无 next_behavior）才返回 `Err`，
  由 waist 自动合成一条 error step 喂回 LLM 自我纠正

**Renderer 行为**：
- `render(step) → (assistant, user)`：assistant = verbatim `assistant_text`；user = `<action_result tool="X" call_id="Y" status="ok|error|pending"[ truncated="true"]>BODY</action_result>`（或 `<step_ack/>` 当 step 无 action）
- `render_history(steps)`：两档压缩——最近 `recent_full_steps` 个步骤全量；更老的步骤用 `<thinking>summary</thinking>` 形式收敛 assistant_text，action_result body 截断
- 严格保持 `(assistant, user)` 交替，对所有 provider 都兼容

**自定义实现**：worksession 想用别的协议（JSON 行、ReAct markdown、自定义 DSL……）时，
实现 `LLMResultParser` + `StepRenderer` 两个 trait 装到 `deps` 即可，不需要改 waist。

### Behavior 切换的三种模式（switch_mode）

- `normal`：同一 `LLMContext` 实例，重新渲染 system prompt（替换 `request.input` 中的 system 段），保留全部 step 历史
- `fork`：基于当前快照 `clone()` 出新 `LLMContext`，继承 step records，子上下文结束时丢弃其快照
- `independent`：开一个全新的 `LLMContext`，不继承 step records；子上下文结束时丢弃其快照

---

## 4. 顶层伪代码

数据流（满足核心原则 #1~#4）：

```text
buckyos-api (msg-center / kevent)            ← 边界
        │
        │  pull_event / get_next / update_record_state
        ▼
msg_center_pump.rs   (fetcher — 只翻译，不 ack)
        │
        │  tokio mpsc<Inbound>                ← 内部 dispatch
        ▼
AIAgent::dispatch_inbound
        │
        │  session.enqueue_pending(PendingInput)
        │     ├─ meta.pending_inputs.push(...)
        │     └─ flush_meta()  (落盘成功才返回 Ok)
        │
        ▼
AgentSession  (持久化状态中心)
        │
        ├─ enqueue 返回 Ok → AIAgent.ack_msg_record(record_id)
        │                       ↑
        │                       └─ msg_center.update_record_state(Readed)
        │
        ▼
session worker loop
        │  Idle / WaitingInput 状态下从 meta.pending_inputs 取
        │  run_one_turn 成功 → discard_consumed(keys) + flush_meta
        │  run_one_turn 失败 → 保留在 pending_inputs，下次 Wakeup 重放
```

### 入口 + 分发（当前实现）

```rust
pub async fn AIAgent::run(self: Arc<Self>) -> Result<()> {
    self.restore_active_sessions().await;        // 重建非 Ended 的 session
                                                 // 每个 session worker 启动后会自动消费其 pending_inputs
    let pump = self.spawn_msg_center_pump();     // 只在 msg_center + kevent_client +
                                                 // parseable agent_did 都齐时才 spawn
    loop {
        tokio::select! {
            item = self.inbox_rx.recv() => match item {
                Some(it) => self.dispatch_inbound(it).await?,
                None     => break,
            },
            _ = self.shutdown_rx.recv() => break,
        }
    }
    self.pump_shutdown.notify_waiters();
    if let Some(h) = pump { let _ = h.await; }   // 等 pump 把 EventReader close 干净
    self.stop_all_sessions().await;
    Ok(())
}

async fn dispatch_inbound(&self, item: Inbound) -> Result<()> {
    match item {
        Inbound::Msg { record_id, from, session_id, text } => {
            let sid = session_id.unwrap_or_else(|| self.resolve_ui_session(&from));
            let session = self.get_or_create_session(sid, from.clone()).await?;
            session.enqueue_pending(PendingInput::Msg { record_id: record_id.clone(), from, text }).await?;
            self.ack_msg_record(record_id).await;   // 落盘后才 ack 给 msg-center
        }
        Inbound::Event { event_id, target_session_id, data } => {
            // MVP：只处理预路由的 event；session_sub_kevent 路由待补
            let Some(sid) = target_session_id else { warn!("event dropped"); return Ok(()); };
            if let Some(s) = self.session_by_id(&sid) {
                s.enqueue_pending(PendingInput::Event { event_id, data }).await?;
            }
        }
    }
    Ok(())
}
```

### msg-center pump（已实现，[msg_center_pump.rs](src/frame/opendan/src/msg_center_pump.rs)）

```rust
async fn run(cfg: PumpConfig) {
    let patterns = build_msg_center_event_patterns(&cfg.owner_did);
    let mut reader: Option<Arc<EventReader>> = None;
    loop {
        if reader.is_none() { reader = cfg.kevent_client.create_event_reader(patterns.clone()).await.ok().map(Arc::new); }
        let mut boxes = Vec::new();
        tokio::select! {
            _ = cfg.shutdown.notified() => { /* close reader; return */ }
            res = reader.as_ref().unwrap().pull_event(Some(1000)) => match res {
                Ok(Some(evt)) => collect_event_pull_targets(&evt, &mut boxes),  // 根据 eventid 选 BoxKind
                Ok(None)      => append_all_inbox_boxes(&mut boxes),            // 超时 → 全 inbox sweep
                Err(KEventError::ReaderClosed(_)) => { reader = None; append_all_inbox_boxes(&mut boxes); }
                Err(_)        => append_all_inbox_boxes(&mut boxes),
            }
        }
        for kind in boxes {
            // get_next(state=[Unread], lock_on_take=true, with_object=true)
            // 翻译成 Inbound::Msg 后扔进 cfg.inbox_tx —— 不在这里 mark Readed
            drain_box(&cfg, kind).await;
        }
    }
}
```

> **关于"每个活动 session 一个线程"**：保留——UI session 天然活跃；worksession 在非 END 状态时也活跃。
> 每个活动 session 一个 tokio task 跑 worker 循环，免去自写调度器，关闭/重启路径也简单
> （task abort + 从最新 snapshot resume + 重放 pending_inputs）。代价是空闲 session 也占一份
> task，但相比 LLM 调用成本可忽略。

### Session Worker（持久化队列消费模型）

```rust
// SessionInput 现在只是【唤醒信号】，载荷在 meta.pending_inputs 里
enum SessionInput { Wakeup, Cancel }

async fn AgentSession::run_worker(self: Arc<Self>, inbox_rx: &mut mpsc::Receiver<SessionInput>) {
    loop {
        // 1) 抢先消费 Cancel（不能被一个长 turn 卡住）
        while let Ok(Cancel) = inbox_rx.try_recv() { self.set_status(Idle).await; /* break if Work */ }

        // 2) 快照 pending —— 不在这里删，等 turn 成功再删
        let pending = self.meta.lock().await.pending_inputs.clone();
        if pending.is_empty() {
            match inbox_rx.recv().await {
                None | Some(Cancel) => return,
                Some(Wakeup)        => continue,
            }
        }

        // 3) 分流：Msg 喂 LLM；Event 在 MVP 阶段 warn 后丢
        let (texts, consumed_keys) = split_pending(&pending);
        if texts.is_empty() { self.discard_consumed(&consumed_keys).await; continue; }

        self.set_status(Running).await;
        match self.run_one_turn(texts).await {
            Ok(NextAction::Idle)        => { self.discard_consumed(&consumed_keys).await; self.set_status(Idle).await; }
            Ok(NextAction::WaitForMsg)  => { self.discard_consumed(&consumed_keys).await; self.set_status(WaitingInput).await; }
            Ok(NextAction::End)         => { self.discard_consumed(&consumed_keys).await; self.set_status(Ended).await; return; }
            Err(err) => {
                // 失败：保留 pending_inputs，等下次 Wakeup 重放 / 人工介入
                self.set_status(Error).await;
                // wait — 否则会 hot-loop 在同一 bad input
                let _ = inbox_rx.recv().await;
            }
        }
    }
}
```

```rust
// ===== 构造/恢复 LLMContext =====
async fn build_or_resume_context(&self, inputs: Vec<SessionInput>) -> LLMContext {
    let deps = self.make_deps();        // 见 §0；turn_hook = self.snapshot_writer

    // A) 有快照 → 优先 resume
    if let Some((snap, fill)) = self.try_make_resume_fill(&inputs).await {
        return LLMContext::resume(snap, fill, deps).expect("snapshot integrity");
    }

    // B) 新 session 或 behavior 切换后的全新 context
    let behavior = self.current_behavior_cfg();
    let mut messages = self.render_system_messages(&behavior).await;   // role.md / self.md / users/* / workspace/readme.md / session/readme.md
    messages.push(self.compose_environment_message(&inputs).await);    // "环境感知 message"：自动召回 memory + workspace/session 当前状态 + 新事件/新消息
    messages.extend(self.replay_visible_history().await);              // 历史片段（受限于压缩策略）

    let request = LLMContextRequest {
        owner: ContextOwnerRef::Agent { session_id: self.id.clone() },
        trace: Some(format!("{}::{}", self.id, self.next_trace_id())),
        objective: behavior.objective.clone(),
        input: messages,
        model_policy: behavior.model_policy.clone(),
        tool_policy: ToolPolicy {
            mode: ToolMode::Whitelist,
            whitelist: behavior.tool_whitelist.clone(),
            max_rounds: behavior.max_rounds,
            ..Default::default()
        },
        output: behavior.output_spec(),
        budget: behavior.budget.clone(),
        human_policy: behavior.human_policy.clone(),
        error_policy: ErrorPolicy {
            mode: ErrorMode::FeedAsObservation,
            max_consecutive_errors: behavior.max_consecutive_errors,
        },
    };
    LLMContext::new(request, deps)
}
```

```rust
// ===== Resume 选型 =====
async fn try_make_resume_fill(&self, inputs: &[SessionInput])
    -> Option<(LLMContextSnapshot, ResumeFill)>
{
    let snap = self.load_latest_snapshot().await?;
    let fill = match (&snap.state.pending_tool_calls.is_empty(), inputs) {
        // 之前 yield 在 WaitInput → 把新到的 user/tunnel 消息打成 HumanInput
        (true, inputs) if inputs.has_human_msg() =>
            ResumeFill::HumanInput { message: inputs.compose_human_message() },
        // 之前 yield 在 PendingTool → 等到了 tool 结果
        (false, inputs) if inputs.has_tool_results() =>
            ResumeFill::ToolResults { results: inputs.take_tool_results() },
        // 崩溃恢复 / 启动后第一次唤起，没有 pending → ResumeFromMidRun
        (true, _) => ResumeFill::ResumeFromMidRun,
        // pending 不空但没收齐 → 不能 resume，继续等
        _ => return None,
    };
    Some((snap, fill))
}
```

```rust
// ===== Outcome 消化 =====
async fn handle_outcome(&self, outcome: LLMContextOutcome) -> NextStep {
    match outcome {
        LLMContextOutcome::Done { output, behavior_result, response, trace, .. } => {
            // UI session：转 MessageObject 发回 tunnel
            // Work session：写 report.md / append step history（其实 step 已经在快照里）
            self.commit_done(output, behavior_result, response, trace).await;

            // behavior_result.next_behavior 是 worksession 的状态机信号
            if let Some(next) = behavior_result.and_then(|r| r.next_behavior) {
                return NextStep::SwitchBehavior(next);
            }
            // 自然收敛
            if self.is_ui_session() { NextStep::WaitForMsg }
            else { self.classify_work_session_done() }   // END / WAIT_FOR_TASK / WAIT_FOR_MSG
        }

        LLMContextOutcome::WaitInput { snapshot, prompt_to_human, deadline_ms, .. } => {
            self.persist_snapshot(snapshot).await;          // turn_hook 已写过，这里只是覆盖确认
            self.show_prompt_to_human(prompt_to_human).await;
            self.set_deadline(deadline_ms);
            NextStep::WaitForMsg
        }

        LLMContextOutcome::PendingTool { pending, snapshot, deadline_ms } => {
            self.persist_snapshot(snapshot).await;
            self.task_mgr.dispatch_async_tools(pending);    // 等回填
            self.set_deadline(deadline_ms);
            NextStep::WaitForTask
        }

        LLMContextOutcome::ContextLimitReached { snapshot, accumulated, .. } => {
            // 这里走 opendan 自己的压缩器（不同于 behavior_loop 内部的 HistoryCompressor）：
            // 把 accumulated 重写后用 ResumeFill::RewrittenHistory 续跑
            let rewritten = self.compress_messages(accumulated).await;
            self.queue_rewritten_history(snapshot, rewritten).await;
            NextStep::Continue
        }

        LLMContextOutcome::BudgetExhausted { which, partial, .. } => {
            self.mark_one_line_status(format!("budget exhausted: {:?}", which));
            // 不写快照（这次推理算"失败"），等自动/手动重试
            NextStep::WaitForMsg
        }

        LLMContextOutcome::Error { error, .. } => {
            // §6 错误处理：waist 已经处理过 Recoverable（FeedAsObservation），
            // 走到这里就是真正不可恢复的异常
            self.mark_one_line_status(format!("error: {error}"));
            self.discard_pending_snapshot().await;
            NextStep::WaitForMsg
        }
    }
}
```

---

## 5. 运行跟踪 / 快照 / 一句话状态

新 runtime 把"运行跟踪"全部对齐到 waist 的 hook：

- **塞入新消息时**：opendan 在 `compose_environment_message` 阶段附加一条"环境感知 message"，
  包含自动召回的 memory、workspace 状态、上次到现在的事件/消息 diff。这条消息**在 waist 之外**构造，不属于 step。
- **压缩**：分两层
  - waist 内的 `HistoryCompressor`（behavior 模式下，step 维度，可选）
  - opendan 自己的消息压缩（响应 `ContextLimitReached`，message 维度，必须）
- **worklog hook**：`WorklogSink::emit(WorkEvent::...)` 中——
  - 每次 `LLMStarted` / `LLMFinished` / `ToolCallPlanned` / `ToolCallFinished` 都更新 session 的"一句话当前状态"（给 UI 看的）
  - 同时落到 `WorklogService` 的 SQLite
- **每次推理返回时**：
  - `TurnHook::before_inference` 已经在**下一次**推理前把当前快照写盘了（"no double-bill on crash"）
  - opendan 额外在 `Outcome` 落地时做一次终态快照（Done 终止；WaitInput / PendingTool / ContextLimitReached 的 snapshot 直接持久化）
- **取消**：
  - 标准取消 = session 进入 idle，下一轮 worker 检查到 cancel flag 后不再启动新 LLMContext
  - 强制取消 = abort tokio task + 用最新快照标 `aborted`，下次进 worker 时按用户意图决定是否 resume

---

## 6. 错误处理

waist 已经分了 `ErrorClass::Fatal` 和 `ErrorClass::Recoverable`，opendan 只关心两件事：

1. **解析错误**（XmlBehaviorParser 失败）走 waist 内部的"合成错误 step → 下一轮自我纠正"路径；
   opendan **只在** parser 内部做强容错（机械修复 → 旁路 LLM 修复 → 抛错让 waist 合成错误 step）。

2. **真正的异常**（`Outcome::Error`）：
   - aicc 链路不可用、tool dispatch 内部 panic、snapshot 损坏
   - opendan：不写快照（这次推理失败）、更新 session 一句话状态为"异常失败"、等自动重试或手动重试

**AgentTool 内部的所有异常都必须正常返回**（`Observation::Error { message }`），让 waist 走
FeedAsObservation——这是为了利用 LLM 的自我修复能力。

---

## 7. UI Session 结果回送

`Outcome::Done` 时，opendan 把 `ContextOutput::Text` 转成 `MessageObject` 发回原 tunnel。
WorkSession 的 `Done` 不需要特别处理——下游通过 worksession 的 `report.md` + worklog 拿结果。

---

## 8. WorkSession 工具：try_create_worksession / create_worksession / forward_msg

UI session 不直接构造 worksession——它经过一个 **fork 出来的 sub-LLMContext** 来决定
"复用已有 worksession 还是新建一个"，由 sub-context 调一个全参数的 `create_worksession`
落地。这套设计把"探索 / 选择"和"实际落地"拆成两个工具，加上 §8.4 的 `forward_msg`
组成 UI session 操纵 worksession 的全部入口。

### 8.1 `create_worksession`（全参数版，立即生效）

> 不暴露给 UI session 顶层；只出现在 `try_create_worksession` fork 出的 sub-context 白名单里。

```rust
create_worksession {
    title: String                        // worksession的标题，在某些场合会出现在worksesison list中
    objective: String,                   // 新 worksession 的目标
    workspace_id: Option<String>,        // worksession的bind的workspace None ⇒ 新建 workspace；Some ⇒ 复用已有
    behavior: Option<String>,            // 默认 = AgentConfig.default_work_behavior
    reason_message: Vec<String>,         // 描述意图的原始 message  
}
```

执行步骤：
1. workspace 解析 / 创建：`workspace_id = Some(id)` → `LocalWorkspaceManager::load_record(id)`；
   `None` → 生成新 workspace_id（建议 `ws-<ulid>`），`create_or_open(new_id, objective, ...)`
2. 创建 worksession 目录 + `.meta/session.json`：
   - 写入 `title` / `objective` / `behavior` / `workspace_id`（`SessionMeta` 需新增
     `title` / `objective` 字段；旧 JSON 用 `#[serde(default)]` 兼容）
   - 渲染 `readme.md`：包含 `title` / `objective` / 一段"起源消息（reason_message）"——
     按时序把 `Vec<String>` 拼成块状文本，worksession 推理时作为环境上下文片段进入
     system prompt（参考 §5 的"环境感知 message"）
3. `workspace.set_current_session(workspace_id, Some(new_session_id))` 绑定
4. **立即唤醒新 worksession**：`status = Idle` → 启动 worker → 因为 session.json 已有
   `objective`，worker 在 build_or_resume 阶段构造首轮 `LLMContextRequest` 时把
   objective 渲染进 system prompt 即可开始推理，**不需要外部消息触发**。这是 worksession
   与 UI session 的本质区别——它是"任务驱动"而非"对话驱动"的。
   - 因此 `reason_message` 不进 `pending_inputs`：它只是 readme 里的起源凭据，objective
     才是工作驱动力
   - 这要求 worker loop 的"空 pending 等待"分支增加一种情形：work session + 有 objective
     且尚未跑过任何一轮 → 直接进 turn，而不是 block 在 `inbox_rx.recv()` 上（§4 伪代码
     需要相应小改）
5. 返回 JSON：
   ```json
   { "session_id": "...", "title": "...", "workspace_id": "...",
     "workspace_status": "created" | "reused",
     "behavior": "...", "status": "created" }
   ```

### 8.2 `try_create_worksession`（UI session 唯一暴露的入口）

UI session LLM 看到的 args 只有一个：

```rust
try_create_worksession { reason: String }
```

实现路径：UI session 当前 turn 内 fork 出一个 sub-LLMContext，让 sub-context 基于
"最近聊天 + 现有 worksession 列表 + reason" 自由决定，最终通过 `create_worksession`
落地。`create_worksession` 的返回值即 `try_create_worksession` 的 tool result。

**Sub-context 的产出义务**（即调用 `create_worksession` 时要填齐的字段）：
- `title` —— 由 sub-LLM 概括一个简短标题（会出现在未来的 worksession list 里）
- `objective` —— 由 sub-LLM 从聊天记录＋reason 综合提炼
- `workspace_id` —— sub-LLM 决定：复用某个已有 workspace（填 id）还是新建（留空）。
  决策依据是 sub-context 注入的 worksession 列表 + UI session 当前绑定的 workspace
- `behavior` —— 一般留空走 `default_work_behavior`；只在 sub-LLM 明确判断需要特定 behavior 时填
- `reason_message: Vec<String>` —— sub-LLM 从最近聊天里**挑选**出真正促成本次创建的若干条
  原始 user 消息（保持原文，不要 LLM 改写），按时序传入。这是 worksession 起源凭据，
  会出现在新 worksession 的 `readme.md` 起源段

**Sub-context 构造（fork mode）**：

- `switch_mode = "fork"` — clone UI session 当前 `LLMContext` 快照，子上下文结束时丢弃其
  快照（不污染 UI session 的 step history）。这是 §3 中 Fork 模式的首个真实用例，
  落地它会同时关闭 §9.4 残项里 "Fork 真实实现" 一项。
- **系统提示词（fork 后重新渲染）**：
  - 标准的 role.md / self.md / users/*.md
  - **"现有 work session 列表"片段**：按 `updated_at_ms` desc 取，硬上限
    `MAX_WORKSESSION_LIST = 64`，每行 `<session_id> | <one_line_status> | <updated_at>`；
    数据源 = `AgentSession.meta` 扫盘
- **第一条 user message**（不属于 UI 主历史；fork 后注入）：
  - 最近 `MAX_FORWARDED_HISTORY = 32` 条 UI session 聊天记录的精简渲染（message 维度，
    跳过纯 tool_result）
  - 调用方传入的 `reason`
- **工具 whitelist** = UI session whitelist − { `try_create_worksession`, `forward_msg` }
  ＋ { `create_worksession` }。即"和 UI session 基本同构，去掉两个特殊工具，加上
  实际落地的工具"
- **`max_rounds`** 用独立常量 `WORKSESSION_PICK_MAX_ROUNDS = 8`，避免 sub-context 无限探索
- `output_spec` 用 `Text`——sub-context 最终必须通过工具调用落地，不需要 schema 化输出

**结果回传**：

- sub-context 成功（即调过 `create_worksession`）→ 把 `create_worksession` 的返回
  JSON 原样作为 `try_create_worksession` 的 `Observation::Ok` 返回给 UI session
- sub-context 失败（budget / error / 终止时未调过 `create_worksession`）→
  `Observation::Error { message }`，body 携带 `{ outcome, sub_trace_id, reason }`，
  让 UI session 下一轮 LLM 自己判断重试还是放弃（错误信息走标准 tool result 喂回，
  不抛 fatal）
- sub-context 的 step 历史**不**回写到 UI session（fork 语义保证）；sub snapshot
  在 fork 结束时清理

### 8.3 工程依赖

- §9.7 `local_workspace`：`create_or_open` / `set_current_session` 已就位；新建路径
  需要补一个 workspace_id mint 函数
- §9.4 残项中的 `Fork` switch_mode 真实实现 — 这两个工具一起拉通
- **不**依赖 `contact_mgr` / `task_mgr`

### 8.4 `forward_msg`（UI ↔ WorkSession 进程内路由）

UI session 唯一能直接把消息推到一个 worksession 的工具。典型场景：worksession 在
工作中向用户发了一个确认问题，用户在 UI 回复后，UI session 的 LLM 决定把这条用户
回复 forward 回原 worksession 让它继续推进。

```rust
forward_msg { target_worksession_id: String }
```

**"被转发的消息" = 触发本轮 UI session 推理的最近 user 消息**：即本轮 worker 从
`pending_inputs` 取走的 `PendingInput::Msg`，多条时取最新的一条。worker 在 turn
入口要把这个句柄存到本轮 tool context 里供 `forward_msg` impl 取用。

**路由实现**：
1. 校验 target：存在 / `kind == Work` / `status != Ended`，任一不满足返回
   `Observation::Error { message }`，下一轮 LLM 自我纠正（不抛 fatal）
2. 构造 `PendingInput::Msg`：
   - `record_id` 用合成 namespace `forward:<src_session_id>:<seq>`，**不会**进入
     `ack_msg_record` 路径（msg-center 不参与本次路由）
   - `from` = UI session 的 owner DID（保持来源可追溯）
   - `text` = 原 user 消息内容
3. `target_session.enqueue_pending(input)`：落盘成功后即返回
   `Observation::Ok { forwarded: true, target_session_id, record_id }`
4. target worksession 的 worker 会在 Idle / WaitingInput 状态下自然消费这条 pending

**不做的事**：
- 不调 `msg_center.post_send`（这是进程内路由，跨 tunnel 的转发是未来另一个工具
  的事，本工具不涉及）
- 不复制原 msg-center `record_id` / `message_id`（避免和真实记录冲突）
- 不支持 cross-agent（只能 forward 到本 agent 的 worksession）

---

## 9. 重构 checklist（给 CodeAgent）

### 当前进度（2026-05-14，下半轮更新）

**本批次新增完成：**
- §9.4 `PendingTool` 真接通：
  - `SessionMeta.pending_task_calls: Vec<PendingTaskCall>` 持久化字段（call_id ↔ task_id ↔ event_pattern 三元映射）
  - `handle_outcome::PendingTool` 现在调 `TaskDispatch::dispatch_async_tool` 创建 task_mgr 任务、`subscribe_event("/task_mgr/<task_id>")` 加订阅并持久化 mapping，返回 `NextAction::WaitForTool`
  - 新增 `persist_snapshot()`（tmp+rename 原子写）确保 PendingTool snapshot（含 `pending_tool_calls`）落盘，TurnHook 的 pre-inference 写入只覆盖 happy path
  - worker loop 重写：把 `pending_inputs` 里命中 `pending_task_calls` event_pattern 的 Event 单独成桶 → 用 `observation_from_task_event` 翻译 `to_status=Completed/Failed/Canceled` → 凑齐 snapshot.pending_tool_calls 后调 `LLMContext::resume(snap, ResumeFill::ToolResults{...})` 续跑；命中不全则保留 pending、wait
  - 续跑后自动 `clear_pending_task_calls()` + `unsubscribe_event(pattern)`，下一轮 PendingTool 干净起跑
  - `AgentSession.event_pump: Option<Arc<SessionEventPump>>` 字段让 `subscribe_event` / `unsubscribe_event` 立即推回 pump，agent 层不再需要中转 `refresh_session_subscriptions`（worker 里调 subscribe 直接生效）
  - 单测覆盖 `observation_from_task_event` 三种终态分支
- §9.4 PendingTaskCall + SessionMeta 字段（title / objective / bootstrap_done）round-trip 测试就位（48/48 全绿）
- §9.6 残项 from_name enrichment：`PumpConfig.contact_lookup: Option<Arc<ContactLookup>>` + `deliver_record` 在 record.from_name 缺失时调 `lookup.from_name(did)`；`Inbound::Msg` / `PendingInput::Msg` 新增 `from_name: Option<String>` 字段（持久化、round-trip 覆盖）；`AIAgent::spawn_msg_center_pump` 自动构造 ContactLookup 并喂给 pump
- §8.1 / §8.4 worksession 控制工具（`worksession_tools.rs`）：
  - `CreateWorksessionTool`（`create_worksession`）/ `ForwardMsgTool`（`forward_msg`）都是 `TypedTool` 实现，持 `Weak<AIAgent>` 防止 Arc 环
  - `ensure_session_inner` 在 `build_session_tools` 之后调 `register_worksession_tools(&tools, Arc::downgrade(&self), &session_id)` 注册到每个 session 的 manager（实际 LLM 是否能看到由 behavior whitelist 控制）
  - `AIAgent::create_work_session(params)`：workspace 解析（reuse/create）→ mint `ws-<uuid12>` session_id → 写 `readme.md`（title / objective / origin / reason_message）→ 调 `ensure_session_inner` 走标准创建路径 → `session.wake()` 触发 bootstrap turn
  - `AIAgent::forward_message(target, source, text)`：校验 target 是 Work 且未 Ended → `enqueue_pending(PendingInput::Msg { record_id: "forward:<src>:<uuid>", ... })`
  - Work session bootstrap：`SessionMeta.bootstrap_done` 标志 + worker loop 在「空 pending + 未 bootstrap + 有 objective」时自动跑首轮，无需外部消息（与 §8.1 step 4 对齐）
  - `render_system_messages` 现在把 `objective` / `title` 作为独立 `## Objective: <title>` 段插到 readme 前

### 当前进度（2026-05-14）

**已完成：**
- §9.1 `llm_context::xml_behavior` + `step_record`：`XmlBehaviorParser` / `XmlStepRenderer` 落地，27 项单测覆盖容错/多 action/压缩/交替等场景。
- §9.2 `opendan::ai_runtime`：5 个 deps 适配器全部实现。
  - `AiccLlmClient` / `OpendanToolAdapter` / `AgentPolicy` / `OpenDanWorklogSink` / `SessionSnapshotHook`
  - `AgentRuntime { aicc, worklog, msg_center, kevent_client, task_mgr }` —— 后三个为 `Option`，用 `with_msg_center` / `with_kevent_client` / `with_task_mgr` builder 注入；CLI 与单测可不连这三条边界
  - `SessionDepsInput { parser_renderer, ... }` + `build_session_deps()` 入口
  - `AgentPolicy` 做两道闸：approval list 与 whitelist 防御性二次校验
- §9.3 `opendan::behavior_cfg` + `opendan::agent_config`：
  - `BehaviorCfg` TOML 解析、`SwitchMode` / `BehaviorMode` / `BehaviorOutput` 翻译到 waist `ToolPolicy` / `HumanPolicy` / `ErrorPolicy` / `BudgetSpec` / `OutputSpec` / `ModelPolicy`，`build_parser_and_renderer()` 装 `XmlBehaviorParser` + `XmlStepRenderer`
  - `AgentConfig::open` 容忍 agent.toml 缺失；`builtin_ui_default()` 兜底；`list_behavior_names()` 扫盘
- §9.4 `opendan::agent_session`（**已升级为状态管理核心**）：
  - `AgentSession` + `AgentSessionBuild { existing_meta }` + `SessionInput { Wakeup, Cancel }` / `SessionReply` / `SessionMeta` / `SessionStatus`
  - **`PendingInput { Msg { record_id, from, from_did, tunnel_did, text }, Event { event_id, data } }` + `SessionMeta.pending_inputs: Vec<PendingInput>`** —— 持久化进 `.meta/session.json`，`#[serde(default)]` 兼容老格式；新增字段 `peer_did` / `peer_tunnel_did` / `event_subscriptions: Vec<EventSubscription>` / `workspace_id` 也全部持久化并能 round-trip 老 JSON
  - **`enqueue_pending(input)`**：dedup（按 `dedup_key`）+ push → `flush_meta()`（tmp + rename crash-consistent）→ Wakeup worker；落盘成功才返回 Ok（外部 ack 依赖这个返回值）
  - **`flush_meta()` 改成 `Result` 返回**，所有 caller 显式处理错误
  - **worker 改为从 `meta.pending_inputs` 消费**：snapshot pending → run_one_turn → 成功才 `discard_consumed` + flush_meta；失败保留以供下次 Wakeup / 重启重放（at-least-once）；`SessionInput` 现在是纯信号；Event pending 翻译成 `[environment event] {eventid} {json}` 与 Msg 文本同轮喂入
  - `build_or_resume`：优先尝试 `state.snap` resume；HumanInput / ResumeFromMidRun fill 已通；`AgentSessionBuild::existing_meta` 让 restore 路径保留 pending_inputs / peer / 订阅 / workspace_id
  - `handle_outcome` 覆盖 Done / WaitInput / PendingTool（warn）/ Budget / Error / ContextLimit；Done 路径会调 `post_outbound_text` 走 `msg_center.post_send` 把回复发回 peer
  - `switch_behavior`（Normal-only；Fork / Independent warn 后按 Normal 处理）
  - 新增 API：`subscribe_event` / `unsubscribe_event` / `subscription_patterns` / `set_workspace` / `workspace_id` / `update_peer`（私有）/ `post_outbound_text`（私有）
- §9.5 `opendan::agent_bash`：`build_session_tools(workspace, session_dir)` 注册 `exec_bash` + read/write/edit/glob/grep；`SessionBinLayout` 持有 4 层 bin 路径（System / Runtime / Agent / Session），目前 overlay 仅落 Session 层（upstream `BinOverlayConfig` 是单 `bin_dir`，待 §9.5 后续扩展）
- §9.6 `opendan::agent` + `opendan::msg_center_pump` + `opendan::session_event_pump`（**msg-center / kevent / outbound 全部接入**）：
  - `AIAgent::open(root, runtime)` 加载 `AgentConfig`、`AIAgent::run()` 驱动 dispatch loop（`tokio::select` { inbox, shutdown }）
  - **`Inbound { Msg { record_id, from, from_did, tunnel_did, session_id, text }, Event { event_id, target_session_id, data } }`** —— `from_did` / `tunnel_did` 用于 outbound 回送，`target_session_id` 由 session_event_pump 填充
  - **`msg_center_pump`**：用 `KEventClient` 订阅 `/msg_center/{owner}/box/**` 系列模式，`pull_event(1s)` hit / miss / ReaderClosed 全部走同一条 `msg_center.get_next` 路径（kevent 是加速通道、不是真理来源——超时落到 sweep all inbox boxes）；翻译成 `Inbound::Msg`（含 sender DID 全形 + `route.tunnel_did`）后 send 到 `inbox_tx`，**自己不做 ack**
  - **`session_event_pump`**（新模块）：单 `EventReader` 聚合所有 session 的 `event_subscriptions` 模式；`set_session_subscriptions` / `remove_session` + `refresh` Notify 触发 reader 重建；`pull_event` 命中后调 `match_event_patterns` 对每个匹配 session fan-out `Inbound::Event { target_session_id: Some(sid), ... }`；shutdown / `ReaderClosed` / 空订阅状态全部分支处理
  - **`dispatch_inbound`**：Msg 路由到 session → `enqueue_pending(...)` 落盘完成 → `ack_msg_record(record_id)` 调 `msg_center.update_record_state(Readed)`；Event 按 `target_session_id` 投到匹配 session
  - `restore_active_sessions()` 从盘上的 `.meta/session.json` 恢复非 Ended，**通过 `AgentSessionBuild::existing_meta` 把订阅 / pending / peer / workspace_id 一并还原**；重启自动重放 `pending_inputs` 里残留的输入，并把订阅推回 event_pump
  - `AIAgent::refresh_session_subscriptions(sid)` 公开接口，工具实现修改完 `AgentSession::subscribe_event` 后调它通知 event_pump
  - **outbound 回送**：`AgentSession::post_outbound_text` 用 `agent_did` 当 sender、`peer_did` 当 to、`peer_tunnel_did` 当 `preferred_tunnel`，组装 `MsgObject { thread.{topic,correlation_id} = session_id, meta.session_id = session_id }` 后调 `msg_center.post_send`；失败仅 warn，本地 reply 不受影响
  - reply 收集任务：每个 session 起一个 logger，把 AssistantText / Error / PromptToHuman / Ended 写日志（outbound 已经在 session 内部送出，logger 仅为可观测性）
  - `main.rs`：bootstrap 拉 `MsgCenterClient` + 构造 `KEventClient::new_full(OPENDAN_SERVICE_NAME, None)` + `TaskManagerClient`（任一不可用时 warn 降级），SIGINT 走 `shutdown()` graceful 退出
  - shutdown 协调：`pump_shutdown: Arc<Notify>` 同时让 msg_center_pump + session_event_pump 关掉各自的 EventReader
- §9.7 `opendan::local_workspace`（**已重写为 AgentSession-owned 绑定**）：
  - `WorkspaceRecord { workspace_id, name, created_by_session, current_session, created_at_ms, updated_at_ms, status }` + `WorkspaceStatus { Ready, Archived, Error }`
  - `LocalWorkspaceManager` 无内存状态（只持 `workspaces_root: PathBuf`），`create_or_open` / `load_record` / `save_record` / `set_current_session` / `list` / `archive` 全部直读直写盘；tmp + rename crash-consistent
  - `validate_workspace_id` 拒 `..` / `/` / 空 id（防 path traversal）
  - 旧版的全局 `session_bindings: HashMap` 删除——session 绑定由 `AgentSession.meta.workspace_id` 持有（持久化进 session.json），workspace 记录里的 `current_session` 仅作冲突检测 hint
  - `AIAgent` 持 `LocalWorkspaceManager`，`ensure_session_inner` 现在调 `create_or_open` 维护工作区记录 + `set_current_session` 双向绑定（session-side 是真理源），重启从 `existing_meta.workspace_id` 复用工作区；`workspaces()` accessor 给后续 `try_create_worksession` 工具
- §9.8 `opendan::contact` + `opendan::task_dispatch`（**task_mgr / contact_mgr 骨架就位，等具体功能接入**）：
  - `ContactLookup { msg_center, owner }` —— `from_name(did)` 走 `msg_center.get_contact`，TTL 分级缓存（hit 5min / miss 1min / 错误不缓存）；`invalidate()` 手动清缓存
  - `TaskDispatch { client: Arc<TaskManagerClient>, user_id, app_id }` —— `dispatch_async_tool(session_id, tool_name, payload)` 创建 `TASK_TYPE_OPENDAN_TOOL = "opendan.async_tool"` 任务并返回 `DispatchedTask { task_id, task }`；`mark_task_completed(task_id, success)` 给 PendingTool 收尾用
  - `AgentRuntime.task_mgr` 字段 + `main.rs` bootstrap（不可用时 warn 降级）；ContactLookup 当前由调用方按需 `new(msg_center, owner)` 构造，未塞进 AgentRuntime 是因为 owner 会随 agent_did 变化
- 工程脚手架：`cargo test -p opendan --lib` **44/44 全绿**（新增覆盖 outbound 字段 round-trip、event_subscriptions / workspace_id 字段持久化、session_event_pump 路由 + dedup、local_workspace CRUD / 校验、contact TTL、task_dispatch tag 常量等）

**仍未完成：**
- §9.2 残项：`OpendanToolAdapter` 真正的 4 层 bin 合成。路径契约已由 [paios 容器需求.md §9](paios容器需求.md) 锁定，按 paios 路径落地即可：
  - **System Bin**：`/opt/buckyos/tools/store/` — 全局只读，所有 App 共享（rx）
  - **Runtime Bin**：`/opt/buckyos/tools/bin/` — App-scoped symlink view，按本 App 授权从 `store/` + ExtTool Volume 渲染（rx）。第一版只需在 schema 上预留这一层，真正承载（ExtTool Volume）由后续 Crafter 镜像产出
  - **Agent Bin**：Agent Root 内的 Agent 自写脚本卷，落在 Instance Volume（rwx 给 Agent）。升级合并按 paios §7.4 R-15：上游新文件追加 / 本地未改的可被覆盖 / 本地已改的保留
  - **Session Bin**：`/opt/buckyos/tools/$agentid/$sessionid/` — rwx，session 启动时按权限创建。**当前 [agent_bash.rs](src/frame/opendan/src/agent_bash.rs) `SessionBinLayout` 把 session 层放在 `session/<sid>/.tool`，需要切到 paios 路径**（paios 文档显式标注"和现有实现对比，主要是修改了 sessions/<id>/.tool 的位置"）
  - **PATH overlay 顺序**：Session > Agent > Runtime > System（前者优先，同名覆盖）；权限校验按 paios §9 权限矩阵
  - 实施前置：upstream `BinOverlayConfig` 当前是单 `bin_dir`，需扩展成有序多层（或换成 path list + 每层权限属性）；同时 `SessionBinLayout` 需要拿到 `BUCKYOS_ROOT` + `agent_id` 才能算出 paios 路径
- §9.4 残项：
  - `try_create_worksession` 工具（§8.2 fork sub-context 探索 + 由 sub-LLM 调 `create_worksession` 落地）；同步拉通 `Fork` switch_mode 真实快照 clone
  - `Independent` switch_mode 真实实现（目前 fall-through 到 Normal）
  - `ContextLimitReached` 的消息层压缩 + `ResumeFill::RewrittenHistory` 续跑
  - "环境感知 message"（auto-recall memory / workspace 状态 / 事件 diff）
  - `forward_msg` 自动抓取 "本轮 origin user 消息"（当前实现要求 LLM 显式传 `message` arg，未来 worker 应把句柄塞进 ToolCtx 让 tool 自取）


### 工程顺序（剩余）

1. **§8.2 `try_create_worksession` + §9.4 Fork switch_mode 真实化** — 同一个改动：先在 waist (`llm_context::LLMContext`) 上加一个 `try_clone_with_deps`-类的 API 让 fork 拿到独立快照；opendan 再实现 `try_create_worksession`（fork 子上下文 + reason / 现有 worksession 列表注入 + sub-LLM 决策后调 `create_worksession`）。这一步落地的同时也把 `forward_msg` 自动抓 "本轮 origin user 消息" 的句柄通路打通（worker 把 PendingInput::Msg 句柄塞进 `SessionRuntimeContext::origin_msg`）。
2. **§9.4 Independent switch_mode + ContextLimitReached 重写 + 环境感知 message** — 三个独立子任务，集中在 `agent_session::handle_outcome` / `build_or_resume` 周围，适合一次性把外壳改到位。
3. **4 层 bin overlay 实施**（§9.2 残项）— 按 [paios §9](paios容器需求.md) 路径契约扩展 upstream `BinOverlayConfig` 到多层有序列表，迁移 `SessionBinLayout` 到 `/opt/buckyos/tools/<agent>/<session>/`，预留 Runtime Bin 接口给 ExtTool Volume。

每个阶段独立编译 + 跑 `cargo test`。当前 opendan 已可：
- 从 msg-center 拉 msg → `Inbound::Msg` → UI session → `enqueue_pending` 落盘 → ack `Readed` → worker 在合适状态下走 `exec_bash` + 读文件 → outcome `Done` 时把回复 `post_send` 回原 peer DID（用 record 上的 `route.tunnel_did` 当 `preferred_tunnel`）
- 进程崩 / 重启：未消费的 msg、peer 路由信息、kevent 订阅列表、workspace 绑定、`pending_task_calls` 全部从 `.meta/session.json` 的对应字段还原（at-least-once）
- 任何 session 通过 `subscribe_event(pattern)` 加订阅 → 直接走 `event_pump.set_session_subscriptions` 立即生效 → `session_event_pump` 重建 reader → kevent 命中自动派发回该 session 的 `pending_inputs`
- LLM 触发 `PendingTool` outcome → 自动转 `task_mgr` 任务、订阅 `/task_mgr/<task_id>`、session 进入 `WaitingTool` → 完成事件回来后自动 `ResumeFill::ToolResults` 续跑
- LLM 调 `create_worksession { title, objective, ... }` → 新建 workspace（按需）+ 新建 Work session 目录 + `readme.md` + 自动 bootstrap 首轮推理（objective 渲染进 system prompt）
- LLM 调 `forward_msg { target_worksession_id, message }` → 进程内路由进 target 的 `pending_inputs`（合成 `record_id`，不走 msg-center）
- msg-center pump 自动用 `ContactLookup` 给缺 `from_name` 的 record 补显示名（hit 5min / miss 1min TTL），LLM 提示词看到的是人名而非裸 DID
