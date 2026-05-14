# new opendan agent runtime

> 重构目标：把 opendan 从「自己写 Agent Loop / Behavior 解析 / step 记录」改造成
> 「**只负责构造正确的 LLMContextRequest + 正确的 LLMContextDeps，调度 LLMContext.run() / resume()，并消化 Outcome**」。
>
> 真正的 LLM 推理循环、tool dispatch、step 记录、错误自动反馈、快照/resume，
> 全部下沉到 `llm_context` crate（slim-waist 已经实现）。
> opendan 是这个 waist 之上的 L3/L4 调度器 + 持久化层。

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

waist 的 deps 公共依赖：
- `aicc_client` — 适配为 `LlmClient`
- `contact_mgr` — 给 forward_msg / forward 类工具用
- `task_mgr` — 事件订阅、跨 session 任务通知
- 全局 `WorklogService` 句柄

### Agent（AgentRootFS，目录结构不变）

```
/role.md + /self.md                      # 自我介绍，进 system prompt
/users/$user_id.md | group_$gid.md       # 针对调用者的系统提示词片段
/memory/                                 # AgentMemory 模块初始化
/notepads/$notepadname/                  # 多本 notepad，AgentMemory 初始化
/skills/$category/$skill_dir/            # Agent 加载的真实 skills（可 self-improve）
/tools/                                  # Agent 自写脚本工具
/behaviors/$name.toml                    # Behavior 模板（系统提示词 + 允许工具 + parser/renderer 配置）
/archive/skills                          # 导入原始 skills，Agent 不直接看
/archive/sessions/$session_id            # 已归档 session
/archive/workspace/$workspace_id         # 已归档 workspace
/archive/worklog.db                      # SQLite 归档
/workspace/$workspace_id/                # 工作区目录
/workspace_list.md                       # 最近活跃 workspace 列表，有大小上限
/session/$session_id/                    # session 目录
```

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
./.meta/session.json   # session 元信息：id / agent_did / owner / current_behavior / status / one_line_status
./.meta/state.snap     # 最新 LLMContextSnapshot（由 turn_hook 写入）
./.meta/state.$N.snap  # 历史快照，按 behavior 切换时归档
./readme.md            # session 目录说明，进环境上下文
./bin/                 # session 级别 binary，软链接 + 脚本
./report.md            # worksession 完成后的工作报告
./archive/             # 完整 history（包括 worklog 子集），可翻看
```

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

**UI Session 默认工具集**（写入 `behaviors/ui_default.toml` 的 whitelist）：
- `exec_bash` / `read_file` / `glob` / `grep` / `edit_file` / `write_file`
- `try_create_worksession` — 类 llm_explorer 流程，对话记录走一次专门的 LLMContext 推理，
  选择/创建 worksession + 绑定默认 workspace，返回结构化结果给 UI session
- `forward_msg` — 转发消息给指定 tunnel/session
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

```rust
// ===== 入口 =====
pub async fn AIAgent::run(self: Arc<Self>) {
    self.init_subscribers().await;             // 读取 Agent 层关心的事件类型，订阅 task_mgr
    self.restore_active_sessions().await;      // 从盘上恢复所有非 END 的 session，重建 worker

    loop {
        tokio::select! {
            msg_pack = self.get_messages() => self.dispatch_msg_pack(msg_pack).await,
            evt_pack = self.get_events()   => self.dispatch_event_pack(evt_pack).await,
            _ = self.shutdown.recv()       => break,
        }
    }
}

// ===== 消息分发 =====
async fn dispatch_msg_pack(&self, pack: MsgPack) {
    for msg in pack {
        // 1) 是某个 worksession 发出消息的回复？→ append 回该 worksession
        if let Some(ws_id) = self.detect_worksession_reply(&msg) {
            self.session_mgr.get(&ws_id).append_msg(msg).await;
            continue;
        }
        // 2) 否则按 from-tunnel 路由到对应的 UI session（必定是 UI session）
        let ui_sid = self.resolve_ui_session_for_tunnel(&msg.from);
        self.session_mgr.get(&ui_sid).append_msg(msg).await;
    }
}

async fn dispatch_event_pack(&self, pack: EventPack) {
    // 找到订阅了该事件的 session 列表，逐个 append_event（events 进入下一次推理的"环境感知 message"）
    for evt in pack {
        for sid in self.subscribers_for(&evt) {
            self.session_mgr.get(&sid).append_event(evt.clone()).await;
        }
    }
}
```

> **关于"每个活动 session 一个线程"**：建议保留——UI session 天然活跃；worksession 在非 END 状态时也活跃。
> 每个活动 session 一个 tokio task 跑 worker 循环，免去自写调度器，关闭/重启路径也简单（task abort + 从最新 snapshot resume）。
> 代价是空闲 session 也占一份 task，但相比 LLM 调用成本可忽略。

```rust
// ===== Session Worker（UI / Work 都用同一个驱动）=====
async fn AgentSession::worker(self: Arc<Self>) {
    loop {
        // 1) 等到至少一条新输入（msg / event / human resume / tool result）
        let input_batch = self.inbox.drain_or_wait().await;
        if input_batch.is_empty() && self.status.is_idle() {
            // UI session 永不退出；work session 在 END/IDLE 且 inbox 空可退出
            if self.is_work_session() && self.status.is_end() { break; }
            continue;
        }

        // 2) 构造或恢复 LLMContext
        let mut ctx = self.build_or_resume_context(input_batch).await;

        // 3) 跑到一个 Outcome
        let outcome = ctx.run().await;

        // 4) 消化 Outcome
        match self.handle_outcome(outcome).await {
            Continue              => continue,              // 例如恢复后立刻进入下一轮
            WaitForMsg            => self.set_idle_wait_msg(),
            WaitForTask           => self.update_subscriptions_and_idle(),
            SwitchBehavior(name)  => self.switch_behavior(name).await,
            EndSession            => { self.archive().await; break; },
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

## 8. 创建 WorkSession

```rust
async fn create_work_session(objective: String, source_session_id: String) -> WorkSessionRef {
    // 1) 选择 workspace：默认从 source UI session 的当前绑定继承；否则在 try_create_worksession
    //    工具内部用一个独立的 LLMContext 跑 llm_explorer，让 LLM 选已有 workspace 或新建一个
    let workspace = self.workspace_mgr.pick_or_create(&source_session_id, &objective).await;
    // 2) 建 session 目录 + 写 .meta/session.json，写 readme.md（用 objective 渲染）
    let ws_session = self.session_mgr.create_work_session(&workspace, &objective).await;
    // 3) 绑定 workspace ↔ session（reentrant lock）
    self.workspace_mgr.bind(&workspace.id, &ws_session.id).await;
    // 4) 返回 ref：核心是说明 worksession 的目录结构，UI session 后续可读
    ws_session.as_ref()
}
```

---

## 9. 重构 checklist（给 CodeAgent）

工程顺序建议：

1. ~~**`llm_context::xml_behavior` + `llm_context::step_record`** — 实现 `XmlBehaviorParser` + `XmlStepRenderer`。~~ ✅ 已完成（27 项单测覆盖容错/多 action/压缩/交替等场景）。详见 §3。
2. **`opendan::ai_runtime`** — 装配 `LLMContextDeps`，含：
   - `AiccLlmClient`（包 `aicc_client`，实现 `LlmClient`）
   - `AgentToolManager`（4 层 bin 合成，实现 `ToolManager`）
   - `AgentPolicy`（读取 behavior_cfg，实现 `PolicyEngine`）
   - `OpenDanWorklogSink`（翻译 `WorkEvent` 到 `WorklogService` + 更新 session 一句话状态）
   - `SessionSnapshotHook`（实现 `TurnHook`，写 `session/.meta/state.snap`）
3. **`opendan::agent_config`** — 加载 `behaviors/$name.toml`；定义 `BehaviorCfg` 结构（含 `switch_mode`、`tool_whitelist`、`output_spec` 等）。
4. **`opendan::agent_session`** — `AgentSession` 类型、worker 循环、`build_or_resume_context`、`handle_outcome`、`switch_behavior`（实现三种 mode）。
5. **`opendan::agent_bash`** — UI session 默认工具集中的 `exec_bash`（拼 4 层 PATH）+ session bin 的脚本管理。
6. **`opendan::agent`** — `AIAgent::run`、`dispatch_msg_pack` / `dispatch_event_pack`、`restore_active_sessions`、订阅管理。
7. **`opendan::local_workspace`** — 已有数据模型保留；删掉对老 BehaviorLoop 的依赖；session 绑定逻辑下放到 `AgentSession`。
8. **删除老代码**：`behavior/`、`workspace/`、`skill_tool.rs`、`step_record.rs`（功能已下沉到 llm_context）；旧的 `AgentSessionMgr` 同步重写。

每个阶段独立编译 + 跑 `cargo test`；阶段 4 之后能拉起一个 UI session 跑 `exec_bash` + `read_file` 的最小回路就算 MVP 通了。
