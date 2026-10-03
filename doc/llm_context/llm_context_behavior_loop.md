# LLM Context 支持 Behavior Loop —— 瘦腰式扩展方案

> 状态(2026-10-03):本文是 Behavior Loop 的扩展方案,主体已落地。文中已按当前实现修正挂起/恢复、behavior 切换(由目标 behavior 的进入模式决定,普通切换已移除)、工具迭代额度和 hook 的描述;接口细节以 [LLM Context 设计](LLM%20Context%20设计.md) §6 / §9 / §10 和 `src/frame/llm_context` 代码为准。Round / Step / Turn 的定义见 [readme](readme.md)。

## 0. 一句话

把 `context_loop.rs::run_inner` 当作瘦腰核——传统 Agent Loop 完整在这里。Behavior Loop **不改这个核**,而是新增一个外层入口 `run_behavior`:每个 Step 内部启一个内层传统 LLMContext 跑到 Done,拿 response 给 parser 解析,产出一个 StepRecord。外层 loop 只做"沉淀 step / 调度 action / 检查 next_behavior",Function 层细节(一个 Step 内多个 Round 的原生 tool 调用)被内层吃掉。

---

## 1. 为什么要 Behavior Loop

按短文《why behavior loop.md》的诊断,传统 Agent Loop 焊死了三件事:

- **工具集 = LLM 认知集**:Function 层(物理能力)和 Action 层(语义动作)没有分开 → 死工具流。
- **结束信号是隐式的**:模型不返回 tool_call 即"完成",调度器没有显式的意图通道。
- **状态机只能外挂**:LangGraph 之类的存在本身就是 Loop 协议表达力不足的证据。

Behavior Loop 解开这三处耦合,但**没有引入新执行核**——它在协议层加了几个可选槽位,让 LLM 显式 commit 意图(`next_behavior`、4 段 Step Schema),让 Function/Action 在 dep 层投影分离。

---

## 2. 瘦腰要立得住,先钉住的不变量

- **LLMContext 一次 `run()` 调用跑到一个 Outcome(终态或挂起态)返回**,不在 loop 内做 behavior 状态机。切换由上层在 `run()` 返回 `Done{next_behavior}` 之后决定:libopendan 按目标 behavior 的进入模式(`extensions.opendan.behaviors.<name>.mode`)调度——`switch_context` 进入目标自己的 run,`create_sub_context` / `fork` 新建子 run 并在结束后返回调用方;交接从不在同一个 run 里改 `behavior_name` 或 system(见 [readme](readme.md))。
- **`run_inner`(传统 Agent Loop)零修改**。Behavior 模式是新增 entry point `run_behavior`,它把 `run_inner` 当子例程调用。
- **Behavior 模式 vs 传统模式是构造时二选一**,运行时不混用——`LLMContext::new` 里靠 deps 字段组合做断言;两个模式走两个 entry point。
- **协议解析归 parser,执行归 dispatcher,Behavior 外层 loop 只读"要不要继续 / 调用什么"两个信号**。
- **嵌套关系映射"Function vs Action 解耦"**:内层 LLMContext 跑 Function 层(多个 Round 的原生 tool 调用收集信息),Step 完成后内层对象消失 → 内层消息(inner transcript)自然 GC;外层 LLMContext 只看 Action 结果。Step 未完成就挂起时,inner transcript 暂存在外层快照里(§7)。

---

## 3. 收敛的关键决策

| # | 决策 | 含义 |
|---|---|---|
| D1 | 外层 state 双结构 | `state.steps: Vec<StepRecord>`(历史,run 内只追加,重写只经 `RewrittenSteps`)+ `state.last_step: Option<StepRecord>`(最近沉淀的热 step,verbatim 渲染);外层 `state.accumulated` = `request.input` + 进行中 Step 的 inner transcript(Step 之间为空尾) |
| D2 | ToolMgr / ActionMgr **同签名**,不引新 trait | Agent Tool 已为 Action 化做好准备;Action 层就是 ToolMgr 实例的另一种装配,Behavior Loop 几乎不配 ToolMgr,而是配一个 action 视图的 ToolMgr |
| D3 | StepRecord 渲染成 `assistant(意图) + user(结果)` 一对 | 喂给 LLM 的结构是 `system(include user_init target) + [History Steps 经压缩渲染] + LastStep assistant + LastStep user`;严格 user/assistant 交替,贴合 LLM 训练分布,无 provider alternation workaround |
| D4 | next_behavior 是 terminal 信号 | parser 产出 `next_behavior: Option<String>`,不带 action / sendmsg 的 Step 上 `is_some()` 即 terminal(带 action 时跳转目标被丢弃,结果须先被观察);无单独 `terminal` bool;字符串语义("END" 等)归上层 worksession,loop 不解释——**唯一例外**是字面量 `END`(2026-09-18 修正:`END` 与 `<actions>` 同现时不得静默丢弃,否则模型永远不收敛;见 [Agent Actions](Agent%20Actions.md) §2.2) |
| D5 | Snapshot schema 版本化 | 当前 `SNAPSHOT_FORMAT_VERSION = 3`,`resume` 只接受这个版本(更旧或更新都 `SnapshotCorrupted`);beta 期 breaking change 直接升版本,不做迁移 |
| D6 | **Behavior step = 一次内层传统 LLMContext run** | 外层每个 Step 启一个内层 LLMContext(`into_traditional()`:无 parser/renderer/step_result_hook/checkpoint_hook),内层跑到 Done,Done.response 给外层 parser 解析,产出 StepRecord。Function 层细节(一个或多个 Round 的原生 tool 调用)被内层吃掉;无 tool_mgr 的纯 Action 场景退化为内层单次 inference(一个 Round) |
| D7 | 内层挂起翻译为外层挂起 | 内层 `PendingTool` / `ContextLimitReached` / `Interrupted` 翻译为外层同名 Outcome,携带外层快照;进行中 Step 的 inner transcript 和被截断的 `tool_batch` 留在外层 state,恢复后继续同一个 Step(§6.3、§7)。原方案"研发期内层不允许 yield、一律转 Fatal"已废弃 |

---

## 4. 新增的最小语义槽位

全部落在新文件 `src/frame/llm_context/src/behavior_loop.rs`,**不进 `buckyos_api`,不污染瘦腰**。

```rust
// 一步的结构化记录(意图槽 + 动作回响)。Behavior 模式的最小历史单元。
// Step 身份 = (run, meta.step_index);step_index 在解析出 Step 时分配。
pub struct StepRecord {
    pub meta: StepMeta,                 // behavior_name / step_index / started_at_ms / ended_at_ms

    // —— 来自 LLM 输出(parser 填,执行前完成)——
    pub assistant_text: String,         // LLM 原文,直接作为 assistant message 内容渲染
    pub assistant_message: Option<AiMessage>,

    pub observation: Option<String>,    // "结论"槽:LLM 对上一步动作结果的解读
    pub thought: Option<String>,        // "思考"槽
    pub actions: Vec<AiToolCall>,       // "动作"槽:一个 Step 可以有多个 action(<actions> 容器)
    pub next_behavior: Option<String>,  // 显式跳转目标 / END(规则见 §6.2)
    pub self_report: Option<String>,    // <report>
    pub messages_sent: Vec<SendMessageRecord>, // <sendmsg>

    // —— 来自动作派发(executor 填,执行后完成)——
    pub action_results: Vec<Observation>,      // 与 actions 按下标对齐
    pub next_user_message: Option<AiMessage>,  // 覆盖默认的动作结果渲染
}
// StepRecord::is_correction():解析失败 / policy 拒绝的合成纠错 Step(占用 step_index,不是行为决策)

// 一次 LLM 推理产物的结构化形式。parser 负责生产。
pub struct LLMBehaviorResult {
    // —— loop 读这两个字段 ——
    pub do_actions: Vec<AiToolCall>,     // 从"动作"槽抽出的可派发调用(借用 AiToolCall 形状,语义即 ActionCall)
    pub next_behavior: Option<String>,   // 终止信号 + 跳转目标
    // —— loop 不解释,原样塞进 StepRecord / Done ——
    pub assistant_text: String,          // 原始响应文本,流向 StepRecord.assistant_text
    pub observation: Option<String>,
    pub thought: Option<String>,
}

pub trait LLMResultParser: Send + Sync {
    fn parse(&self, response: &AiResponseSummary) -> Result<LLMBehaviorResult, String>;
}


pub trait StepRenderer: Send + Sync {
    //1个完整的step可以得到2条消息
    fn render(&self, step: &StepRecord) -> (AiMessage,AiMessage);
    fn render_history(&self, steps: Vec<StepRecord>, current_behavior: &str,
        summaries: Vec<HistorySummaryRecord>, inputs: Vec<HistoryInputRecord>) -> Vec<AiMessage>;
}

// Step 有动作结果后、沉淀前的扩展点(可覆盖下一条 user 消息)
pub trait StepResultHook: Send + Sync {
    async fn on_behavior_step_ob(&self, snapshot: &LLMContextSnapshot, step: &StepRecord)
        -> Result<StepResultHookOutput, String>;
}
```

> 原方案中的 `HistoryCompressor` 已移除:loop 内不压缩历史,见 §7 与 [append-only history](llm_context_append_only_history.md)。

> 注:`ActionMgr` 不另立 trait。Action 的 dispatch 通过装配一个特殊的 `ToolManager` 实现完成——Agent Tool 已经为这种装配方式做好了准备。

---

## 5. deps 与 state 的扩展

### `LLMContextDeps`(`deps.rs`)

新增字段,全部 `Option`:

```rust
pub result_parser: Option<Arc<dyn LLMResultParser>>,
pub step_renderer: Option<Arc<dyn StepRenderer>>,
pub step_result_hook: Option<Arc<dyn StepResultHook>>,
```

两个推理前 hook 在 behavior 模式下的分工(两者都是通用依赖,不是 Behavior 专属):

- `inference_hook: Option<Arc<dyn InferenceHook>>`:同步,每个 Round(每次推理)前调用。`into_traditional()` 保留它,所以 behavior 模式下它在 Step 内层的每个 Round 前执行,看到扁平化的内层快照。它不是 Session Turn 的钩子。
- `checkpoint_hook: Option<Arc<dyn CheckpointHook>>`:异步,看外层快照,可以注入消息。behavior 模式只在外层 Step 边界(每个 Step 的 do-action 之后、下一个 Step 的内层推理之前)调用;`into_traditional()` 去掉它,Step 内层的推理前不会调用。

> `tools` 字段保持当前签名(`Arc<dyn ToolManager>`),不改 Option。Behavior 模式装配的是"以 Action 语义对外、内部委派 Function"的 ToolManager 实现,瘦腰看到的还是 ToolManager。

模式判定(`LLMContext::new` 内部 assert):

- `result_parser.is_some()` → **Behavior 模式**。要求 `step_renderer` 也已设置(否则构造失败)。
- `result_parser.is_none()` → **传统模式**。`steps` 不应被填充。

### `LLMContextState`(`state.rs`)

新增字段:

```rust
pub steps: Vec<StepRecord>,             // 已沉淀的历史 step;Behavior 模式专用,传统模式恒空
pub last_step: Option<StepRecord>,      // 最近沉淀的一步(还热的,verbatim 渲染);下一步沉淀时 push 进 steps
pub action_step: Option<ActionStep>,    // 已解析、action 尚未派发完的 Step(未沉淀)
pub next_step_index: u32,               // 下一个待分配的 step_index(分配位置,不是已完成 Step 数)
pub next_action_id: u32,
```

共用的预算 / 挂起字段:`tool_iterations_left`(工具迭代额度,原生批次与 action Step 共享)、`suspended`、`tool_batch`(被 deferred 截断的原生批次,含 `batch_error`)。

invariant:
- 传统模式:`steps.is_empty() && last_step.is_none() && !accumulated.is_empty()`(每个 Round push assistant message 及其工具结果)
- Behavior 模式:`accumulated` = `request.input` + 进行中 Step 的 inner transcript,Step 完成后截回 `request.input`;`steps` / `last_step` / `action_step` 由外层 loop 维护
- 任何时刻不会两种模式痕迹同存

### `LLMContextOutcome::Done`(`outcome.rs`)

新增字段:

```rust
behavior_result: Option<LLMBehaviorResult>,   // 传统模式恒为 None
```

`Done.next_behavior` 不单独提出 —— 上层从 `behavior_result.as_ref().and_then(|r| r.next_behavior.clone())` 读。

---

## 6. Behavior Loop 的实现形态 —— `run_inner` 零修改

核心思想:**Behavior Loop 是一个新 entry point `run_behavior`,内部把 `run_inner`(传统 Agent Loop)当子例程调用。** 现有 `context_loop.rs::run_inner` 一行不动。

### 6.1 外层 entry point

```rust
impl LLMContext {
    pub async fn run(&mut self) -> LLMContextOutcome {
        if self.is_behavior_mode() {
            self.run_behavior().await
        } else {
            self.run_inner().await   // 现有传统 Agent Loop,零修改
        }
    }
}
```

`is_behavior_mode()` = `deps.result_parser.is_some()`。

### 6.2 外层 `run_behavior` 形态

```rust
async fn run_behavior(&mut self) -> LLMContextOutcome {
    loop {
        // 0. 已解析、action 未派发完的 Step(刚解析,或 deferred action 回填后)先续派
        if self.state.action_step.is_some() {
            if let Some(o) = self.run_step_actions().await { return o; }
            continue;
        }

        // budget / wallclock 检查(外层 Step 维度)
        if let Some(o) = self.check_wallclock_budget() { return o; }

        // Step 边界:CheckpointHook 以外层快照落盘 / 注入观察。内层原生批次被 deferred
        // 截断、回填后续跑时不是 Step 边界,不调用。
        if self.state.tool_batch.is_none() {
            if let Some(o) = self.run_checkpoint_hook().await { return o; }
        }

        // 1. 跑一个内层传统 LLMContext(一个或多个 Round),得到 response
        let response = match self.run_inner_for_step().await {
            Ok(resp) => resp,
            Err(outer_outcome) => return outer_outcome,   // 内层错误 / budget / 挂起已被翻译成外层 outcome
        };

        // 2. parser 解析得到 LLMBehaviorResult;失败走 FeedAsObservation(包成合成纠错 Step,
        //    同样分配 step_index)
        let result = match self.deps.result_parser.as_ref().unwrap().parse(&response) {
            Ok(r) => r,
            Err(e) => {
                let err_step = self.prepare_step(StepRecord::from_parse_error(&e));
                self.sediment(err_step);
                if let Some(o) = self.bump_consecutive_errors(..) { return o; }
                continue;
            }
        };

        // 3. 包成 StepRecord,分配 step_index / action call_id(此时 action_results 还没填)
        let mut new_step = self.prepare_step(StepRecord::from_result(result));

        // 4. 带 action 的 Step 需要一次工具迭代
        if !new_step.actions.is_empty() && self.state.tool_iterations_left == 0 {
            return LLMContextOutcome::BudgetExhausted { which: BudgetKind::ToolIterations, .. };
        }

        // 5. report / sendmsg 副作用;action policy gate(拒绝 ⇒ 合成纠错 Step,continue);
        //    带 action / sendmsg 时跳转目标被丢弃(结果必须先被观察),只保留 END

        // 6. 派发前扣一次工具迭代;Step 进入 action_step,在循环顶部按序派发
        if !new_step.actions.is_empty() {
            self.state.tool_iterations_left -= 1;
        }
        self.state.action_step = Some(ActionStep { step: new_step, response });
    }
}

// run_step_actions:按序派发,第一个非成功结果之后的 action 记为 Unresolved;
//   Pending + allow_deferred ⇒ PendingTool(Step 留在 action_step,不沉淀)。
// 全部派发 / 跳过后 complete_step:
//   next_behavior 生效 / 这一步什么都没做 / StepResultHook 要求结束 ⇒ finish_done_behavior
//   否则 sediment(last_step 入 steps,新 step 成为热 step),有动作错误则 bump,进入下一个 Step

fn sediment(&mut self, new_step: StepRecord) {
    if let Some(prev) = self.state.last_step.replace(new_step) {
        self.state.steps.push(prev);
    }
}
```

### 6.3 内层调用 `run_inner_for_step`

每个 Step 一次 sub-run。内层 deps 复用外层 llm/tools/policy/worklog/tokenizer/inference_hook,**剥掉** parser/renderer/step_result_hook/checkpoint_hook;内层 request 由外层渲染历史得到。

```rust
async fn run_inner_for_step(&mut self) -> Result<AiResponse, LLMContextOutcome> {
    let inner_req = self.build_inner_request();               // 见 6.4,末尾是 inner transcript
    let prefix_len = inner_req.input.len() - self.inner_transcript().len();
    let mut inner = LLMContext::new(inner_req, self.deps.clone().into_traditional());
    inner.abort = self.abort.clone();                         // 外层 interrupt_handle 能中断内层推理
    inner.state.usage = self.state.usage.clone();
    inner.state.started_at_ms = self.state.started_at_ms;
    inner.state.consecutive_errors = self.state.consecutive_errors;
    inner.state.tool_iterations_left = self.state.tool_iterations_left;  // 工具迭代内外共享
    inner.state.tool_batch = self.state.tool_batch.take();              // 被截断的批次先续派
    let outcome = inner.run_inner().await;

    // 无论结果如何都交回 usage / 错误计数 / 剩余工具迭代(tool_trace / llm_task_ids 同样并回)
    self.state.usage = inner.state.usage.clone();
    self.state.consecutive_errors = inner.state.consecutive_errors;
    self.state.tool_iterations_left = inner.state.tool_iterations_left;
    if !matches!(outcome, LLMContextOutcome::Done { .. }) {
        // 进行中 Step 的 inner transcript 与被截断的 tool_batch 留在外层 state,
        // 外层快照恢复时继续这个 Step,不重放已执行的工具
        self.set_inner_transcript(inner.state.accumulated[prefix_len..].to_vec());
        self.state.tool_batch = inner.state.tool_batch.take();
    }

    match outcome {
        LLMContextOutcome::Done { response, .. } => {
            self.clear_inner_transcript();
            Ok(response)
        }
        LLMContextOutcome::PendingTool { pending, .. } => {
            self.state.suspended = inner.state.suspended.take();
            Err(LLMContextOutcome::PendingTool { pending, snapshot: self.snapshot(), .. })   // 外层快照
        }
        LLMContextOutcome::ContextLimitReached { which, mut accumulated, .. } => {
            self.state.suspended = inner.state.suspended.take();
            accumulated.truncate(prefix_len);                 // 可重写的历史不含 inner transcript
            Err(LLMContextOutcome::ContextLimitReached { which, accumulated, snapshot: self.snapshot(), .. })
        }
        LLMContextOutcome::Interrupted { reason, abort, .. } => {
            Err(LLMContextOutcome::Interrupted { reason, snapshot: self.snapshot(), abort, .. })
        }
        LLMContextOutcome::Error { error, .. } => Err(self.finish_error(error)),
        LLMContextOutcome::BudgetExhausted { which, partial, .. } => {
            Err(LLMContextOutcome::BudgetExhausted { which, partial, usage: self.state.usage.clone() })
        }
    }
}
```

### 6.4 内层 request 装配 `build_inner_request`

外层 step_renderer 在这里被使用——把 `[steps] + last_step` 渲染成 AiMessage 序列,与外层 `request.input` (system + 输入) 拼接,再接上进行中 Step 的 inner transcript,作为内层 input。

```rust
fn build_inner_request(&self) -> LLMContextRequest {
    let renderer = self.deps.step_renderer.as_ref().unwrap();
    let mut messages = self.request.input.clone();                    // system + 输入
    messages.extend(renderer.render_history(
        self.state.steps.clone(),
        &self.request.behavior_name,
        self.state.history_summaries.clone(),
        self.state.history_inputs.clone(),
    ));
    if let Some(ref last) = self.state.last_step {
        let (assistant_msg, user_msg) = renderer.render(last);
        messages.push(assistant_msg);
        messages.push(user_msg);
    }
    messages.extend(self.inner_transcript());                        // 进行中 Step 的内层消息

    // tool_policy / output / budget / error_policy / owner / trace / model_policy 照原配
    LLMContextRequest { input: messages, ..self.request.clone() }
}
```

Step 开始时最后一条是 user(last_step 的 action_results 渲染),内层 inference 自然产 assistant —— alternation 不破;Step 内续跑时最后一条是 tool result。

### 6.5 内外层 budget / usage / trace 关系

- **Budget**:内层 request 复用外层 BudgetSpec;内层实例以外层的 `usage` / `started_at_ms` / `consecutive_errors` / `tool_iterations_left` 起步,返回后交回,所以按 Step 重建内层不能绕过 token / wallclock 预算、错误上限和工具迭代额度。工具迭代:内层每个完成的原生批次扣一次,外层带 action 的 Step 派发前扣一次,两者共享同一额度;推理(Round)本身不扣。
- **Usage**:内层 usage 无论结果如何都交回外层。
- **Trace**:内层 tool_trace 合并进外层 ContextRunTrace。内层 llm_task_ids 同。

### 6.6 终结

`finish_done_behavior(last_step, response)` 把最终的 LLMBehaviorResult 一并塞进 `Done.behavior_result`,并把 last_step 也沉淀进 steps(便于上层审计完整链路):

```rust
async fn finish_done_behavior(&mut self, last_step: StepRecord, response: AiResponseSummary) -> LLMContextOutcome {
    let behavior_result = LLMBehaviorResult::from_step(&last_step);
    self.sediment(last_step);
    LLMContextOutcome::Done {
        reason: None,
        output: ContextOutput::Text { content: response.text.clone().unwrap_or_default() },
        behavior_result: Some(behavior_result),
        usage: self.state.usage.clone(),
        response,
        trace: self.build_trace(),
    }
}
```

---

## 7. 压缩与 Resume

### 压缩

loop 内不压缩历史(原方案的 `maybe_compress` / `HistoryCompressor` 已移除,原因见 [append-only history](llm_context_append_only_history.md)):一次 `run()` 内 `steps` 只追加,已发送的 prefix 保持稳定。需要压缩时由宿主在 `ContextLimitReached` 之后用 `ResumeFill::RewrittenSteps` 显式重写:

- 替换 `request.input` / `history_summaries` / `steps` / `last_step`;`steps` 与 `last_step` 只能保留快照中已有的(可压缩的)step,按 `step_index` 识别。摘要以 `HistorySummaryRecord` 放进 `history_summaries`,由 `render_history` 渲染,alternation 不破。
- 把整个物化历史折叠进 `input`(steps 为空、没有热 step)也合法;libopendan 就是这样做的:先把 steps flush 进 Session worklog,再以 system + 重建的会话历史作为新 input。
- Step / action 编号、`history_inputs` 和进行中 Step 的 inner transcript 保持不变。

压缩策略(机械压缩、LLM 摘要)属于宿主,不是 waist 注入的 trait。

### Resume

behavior 模式的快照总是**外层**快照。挂起点有两类:Step 之间派发 action 时(deferred action),以及 Step 内层(原生工具 deferred、物化 prompt 装不下、推理被中断)。Step 未完成时的恢复状态都在外层 `LLMContextState`:

- inner transcript:`accumulated` 中 `request.input` 之后的消息,即进行中 Step 的内层原生工具 Loop(tool_use 与已得结果),尚未折叠进 `StepRecord`;
- `tool_batch`:内层被 deferred 截断的原生批次(未派发的调用与 `batch_error`);
- `action_step`:已解析、action 尚未派发完的 Step 与它的 response(未沉淀,`step.action_results` 是已得结果);
- `next_step_index` / `next_action_id`:下一个待分配的编号。step_index 在解析出 Step 时就分配(合成纠错 Step 也占一个),是分配位置,不是已完成 Step 数;`action_step` 里的 Step 已有编号但未完成。

`ResumeFill`(与 function call 模式共用,快照版本 3):

| 挂起 | fill | behavior 模式下的语义 |
|---|---|---|
| `PendingTool` | `ToolResults` | 内层原生工具:结果写入 inner transcript,批次续派,整批完成才扣一次工具迭代;deferred action:结果写入 `action_step.step.action_results`,续派其后的 action(该 Step 在派发前已扣过工具迭代) |
| `ContextLimitReached` | `RewrittenSteps` | 见上;inner transcript 原样保留 |
| 未挂起(`Interrupted` 之后、outcome / checkpoint 边界) | `ResumeFromMidRun` | 有 `action_step` / `tool_batch` 先续派;否则从这次推理前重新推进,同一个 Step 继续 |

`resume` 在任何推理 / 工具调用之前校验:fill 必须与 `suspended` 对应;`tool_batch` 与 `action_step` 不能同时存在,`action_step` 只能出现在 behavior 模式;`accumulated` 中除 `tool_batch` 尚未派发的调用外不能有未配对的 tool_use;有续派时拒绝重写。不重跑已完成的工具、不重复解析或派发 action、不重复扣工具迭代。

"等待用户输入"不是 waist 挂起态:`WAIT_USER_MSG` 由上层解释;新输入在 Step 边界经 `LLMContext::inject` / `CheckpointHook` 注入,behavior 模式并入热 step 的 `next_user_message`。原方案的 `ReplaceSteps` / `HumanInput` fill 没有采用。

---

## 8. worksession 侧的心智模型

当前实现的 worksession 是 libopendan 的 `SessionRunner`(完整伪代码见 [readme](readme.md)):

```
SessionRunner 推进 live run(behavior 模式的 LLMContext,deps 含 action 视图的 ToolManager、parser、renderer、checkpoint_hook)
└─ loop:
   match ctx.run().await {                     # 走 run_behavior 分支;一次 run() 调用 → 一个 Outcome
       Done { behavior_result: Some(r), .. } => match r.next_behavior.as_deref() {
           None | Some("END")    => 结束 run,Turn completed;按 end_condition 结束 Session 或等待输入
           Some("WAIT_USER_MSG") => 结束 run,等待输入;已交付答复才完成 Turn,否则下一条输入并入同一 Turn
           Some(name) => match behavior_entry(name).mode {   # 目标 behavior 的进入配置,没有默认模式
               没有条目           => Turn failed(behavior_config);在子 context 内则以 failed 返回调用方
               switch_context     => 当前 run 挂起入 process_stack(Parked),恢复目标自己挂起的 run,
                                    否则按目标自己的 system / 工具 / 模型新建;下一次输入批次带 on_behavior_switch
               create_sub_context => 当前 run 作为 Caller 帧入栈,derive_child 新建子 run(目标的 system + inherit 选择的历史)
               fork               => 当前 run 作为 Caller 帧入栈,fork_snapshot 新建子 run(调用方的 system + 完整有效历史)
                                    子 run 无论以什么结束都返回调用方(<process_result behavior status>),Turn 不结束
           }
       }
       # 当前 run 是子 context 时,END / WAIT_USER_MSG / 交接到 switch_context 目标 / 不可重试错误 / 预算耗尽都改为返回调用方(ok / needs_user_input / failed)
       PendingTool(call_behavior 的 task "subctx:<call_id>") => 调用方保留未完成的批次 / Step 入栈(Caller),
                                    子 run 结束后用 ToolResults 按 call_id 回填,同一 Turn 继续
       PendingTool / ContextLimitReached / Interrupted => run 保留(外层快照),处理后 resume,同一 Turn 继续
       Error / BudgetExhausted => 可重试的错误暂停 run;否则结束 run,Turn 记为 failed / budget_exhausted
   }
```

LLMContext 一次 `run()` 只执行一个 behavior,状态机在 worksession,不在 loop 内;一个 run 的 `behavior_name` 和 system 自始至终不变,behavior 交接总是换到另一个 run(目标自己的,或新建的子 run)。子 context 的快照由 `llm_context::context_derive` 的 `derive_child` / `fork_snapshot` 从调用方快照派生,调用方快照不被修改。**两层嵌套结构同构**:worksession 推进 Behavior LLMContext,Behavior LLMContext 每个 Step 起一个传统 LLMContext——都遵循"一次 run() 调用返回一个 Outcome"语义。Outcome 不等于 Turn 结束,Turn 的开始 / 继续 / 结束由 session 判定。

---

## 9. 故意不做 / 划在范围外

- **不**改 `run_inner`(传统 Agent Loop)。Behavior Loop 用它当子例程,不动它的代码。
- **不**在 loop 内做 behavior 切换。一次 `run()` 调用一个 behavior;切换在 `run()` 返回后由上层做,且总是换到另一个 run(§8)。
- **不**新建 ActionMgr trait。Action 通过 ToolMgr 装配实现。
- **不**改 `AiMessage` 定义,**不**进 `buckyos_api`。
- **不**在 `LLMContextOutcome` 加新变体。`Done` 加字段即可。
- **不**在传统模式路径上加任何额外开销——所有 Behavior 字段是 `Option`,`None` 走老路。
- **不**做 Snapshot 嵌套:内层挂起只把 inner transcript / `tool_batch` 存进外层 state,没有独立的内层快照(D7)。
- **不**做旧快照迁移:schema 变化直接升 `SNAPSHOT_FORMAT_VERSION`(当前 3),`resume` 只接受当前版本。
- **不**并发派发一个 Step 的多个 action:v2 协议允许一个 Step 带多个 action(`actions` / `action_results: Vec<_>`),按文档顺序串行派发,第一个非成功结果之后的 action 不执行。

---

## 10. 实施顺序(历史记录)

> 以下是原方案的实施顺序,已完成,之后又有演进:`HistoryCompressor` / `maybe_compress` / `MechanicalCompressor` 已移除(压缩上移到宿主,§7);内层挂起改为翻译成外层挂起(D7);`run_inner` 增加了 `tool_batch` 续派、`CheckpointHook` 与 `InferenceHook`。当前接口见 §4–§7。

1. `behavior_loop.rs`:落类型 + trait 签名(`StepRecord` / `LLMBehaviorResult` / `LLMResultParser` / `StepRenderer` / `HistoryCompressor`),无实现。
2. `deps.rs`:加 3 个 `Option` 字段(`result_parser` / `step_renderer` / `history_compressor`) + `with_xxx` 方法 + `into_traditional()` helper(剥掉 behavior 字段,内层用)。
3. `state.rs`:加 `steps: Vec<StepRecord>` + `last_step: Option<StepRecord>` + `is_behavior_mode()` helper。
4. `outcome.rs`:`Done` 加 `behavior_result: Option<LLMBehaviorResult>`。
5. `context_loop.rs`:
   - 改 `run()`:按 `is_behavior_mode()` 分发到 `run_behavior()` 或 `run_inner()`。
   - 新增 `run_behavior()`(§6.2)/ `run_inner_for_step()`(§6.3)/ `build_inner_request()`(§6.4)/ `sediment()` / `maybe_compress()` / `finish_done_behavior()`(§6.6)。
   - **`run_inner` 本身一行不动。**
6. 默认实现:`DefaultStepRenderer`(`assistant_text` + `format(action_result)` 成对)、`MechanicalCompressor`(保留 K + M,中间合成 summary step)。
7. 测试:
   - 传统模式回归(无 parser 装配,走 `run_inner`,行为应字节级一致)
   - Behavior 模式基础闭环:dummy parser + dummy renderer + dummy ToolMgr,两 step + 一次 next_behavior 跳转的端到端
   - 内层挂起 → 外层同名挂起 Outcome + inner transcript 恢复(原计划为转 fatal)

Snapshot/Resume 细节留到 7 之后再敲定。
