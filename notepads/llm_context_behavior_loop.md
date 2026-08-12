# LLM Context 支持 Behavior Loop —— 瘦腰式扩展方案

## 0. 一句话

把 `context_loop.rs::run_inner` 当作瘦腰核——传统 Agent Loop 完整在这里。Behavior Loop **不改这个核**,而是新增一个外层入口 `run_behavior`:每个 step iteration 内部启一个内层传统 LLMContext 跑到 Done,拿 response 给 parser 解析,产出一个 StepRecord。外层 loop 只做"沉淀 step / 调度 action / 检查 next_behavior",Function 层细节(多轮 tool 调用)被内层吃掉。

---

## 1. 为什么要 Behavior Loop

按短文《why behavior loop.md》的诊断,传统 Agent Loop 焊死了三件事:

- **工具集 = LLM 认知集**:Function 层(物理能力)和 Action 层(语义动作)没有分开 → 死工具流。
- **结束信号是隐式的**:模型不返回 tool_call 即"完成",调度器没有显式的意图通道。
- **状态机只能外挂**:LangGraph 之类的存在本身就是 Loop 协议表达力不足的证据。

Behavior Loop 解开这三处耦合,但**没有引入新执行核**——它在协议层加了几个可选槽位,让 LLM 显式 commit 意图(`next_behavior`、4 段 Step Schema),让 Function/Action 在 dep 层投影分离。

---

## 2. 瘦腰要立得住,先钉住的不变量

- **LLMContext 一次 `run()` 跑到 terminal outcome 出去**。Behavior 切换 = worksession 创建新的 LLMContext,不在 loop 内做内部状态机。
- **`run_inner`(传统 Agent Loop)零修改**。Behavior 模式是新增 entry point `run_behavior`,它把 `run_inner` 当子例程调用。
- **Behavior 模式 vs 传统模式是构造时二选一**,运行时不混用——`LLMContext::new` 里靠 deps 字段组合做断言;两个模式走两个 entry point。
- **协议解析归 parser,执行归 dispatcher,Behavior 外层 loop 只读"要不要继续 / 调用什么"两个信号**。
- **嵌套关系映射"Function vs Action 解耦"**:内层 LLMContext 跑 Function 层(多轮 tool 收集信息),内层对象消失 → 内层 accumulated 自然 GC;外层 LLMContext 只看 Action 结果。

---

## 3. 收敛的关键决策

| # | 决策 | 含义 |
|---|---|---|
| D1 | 外层 state 双结构 | `state.steps: Vec<StepRecord>`(历史,可压缩)+ `state.last_step: Option<StepRecord>`(当前热数据,verbatim 渲染);外层 `state.accumulated` 恒空 |
| D2 | ToolMgr / ActionMgr **同签名**,不引新 trait | Agent Tool 已为 Action 化做好准备;Action 层就是 ToolMgr 实例的另一种装配,Behavior Loop 几乎不配 ToolMgr,而是配一个 action 视图的 ToolMgr |
| D3 | StepRecord 渲染成 `assistant(意图) + user(结果)` 一对 | 喂给 LLM 的结构是 `system(include user_init target) + [History Steps 经压缩渲染] + LastStep assistant + LastStep user`;严格 user/assistant 交替,贴合 LLM 训练分布,无 provider alternation workaround |
| D4 | next_behavior 是 terminal 信号 | parser 产出 `next_behavior: Option<String>`,`is_some()` 即 terminal;无单独 `terminal` bool;字符串语义("END" 等)归上层 worksession,loop 不解释 |
| D5 | Snapshot schema 待 Behavior Loop 落定后再冻结 | 研发期间不背向前兼容包袱;但相关结构始终保持 `Serialize`/`Deserialize` derive |
| D6 | **Behavior step = 一次内层传统 LLMContext run** | 外层每个 step iteration 启一个内层 LLMContext(无 parser/renderer/compressor),内层跑到 Done,Done.response 给外层 parser 解析,产出 StepRecord。Function 层细节(多轮 tool)被内层吃掉;无 tool_mgr 的纯 Action 场景退化为内层单次 inference |
| D7 | 研发期内层不允许 yield | 内层 WaitInput / PendingTool / ContextLimitReached 一律转 Fatal 上抛外层。Snapshot 嵌套留以后 |

---

## 4. 新增的最小语义槽位

全部落在新文件 `src/frame/llm_context/src/behavior_loop.rs`,**不进 `buckyos_api`,不污染瘦腰**。

```rust
// 一步的结构化记录(4 段意图 + 1 段动作回响)。Behavior 模式的最小历史单元。
pub struct StepRecord {
    // —— 来自 LLM 输出(parser 填,执行前完成)——
    pub assistant_text: String,         // LLM 原文,直接作为 assistant message 内容渲染
    
    pub observation: Option<String>,    // "结论"槽:LLM 对上一步动作结果的解读
    pub thought: Option<String>,        // "思考"槽
    pub action: Option<AiToolCall>,     // "动作"槽:本步意图(name + args)
    pub next_behavior: Option<String>,  // 显式跳转目标(填了即 terminal)
    
    // —— 来自动作派发(executor 填,执行后完成)——
    pub action_result: Option<Observation>,
    // terminal step(next_behavior 填了)的 action_result 可以为 None
    // —— loop 在它之后直接 finish_done,renderer 不会渲染这条
}

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
    fn render_history(&self,steps: Vec<StepRecord>) -> Vec<AiMessage>;
}

pub trait HistoryCompressor: Send + Sync {
    async fn compress(&self, steps: Vec<StepRecord>, budget: CompressBudget)
        -> Result<Vec<StepRecord>, CompressError>;
}
```

> 注:`ActionMgr` 不另立 trait。Action 的 dispatch 通过装配一个特殊的 `ToolManager` 实现完成——Agent Tool 已经为这种装配方式做好了准备。

---

## 5. deps 与 state 的扩展

### `LLMContextDeps`(`deps.rs`)

新增字段,全部 `Option`:

```rust
pub result_parser: Option<Arc<dyn LLMResultParser>>,
pub step_renderer: Option<Arc<dyn StepRenderer>>,
pub history_compressor: Option<Arc<dyn HistoryCompressor>>,
```

> `tools` 字段保持当前签名(`Arc<dyn ToolManager>`),不改 Option。Behavior 模式装配的是"以 Action 语义对外、内部委派 Function"的 ToolManager 实现,瘦腰看到的还是 ToolManager。

模式判定(`LLMContext::new` 内部 assert):

- `result_parser.is_some()` → **Behavior 模式**。要求 `step_renderer` 也已设置(否则构造失败)。
- `result_parser.is_none()` → **传统模式**。`steps` 不应被填充。

### `LLMContextState`(`state.rs`)

新增字段:

```rust
pub steps: Vec<StepRecord>,             // 已沉淀的历史 step,可走压缩;Behavior 模式专用,传统模式恒空
pub last_step: Option<StepRecord>,      // 当前最新一步(还热的,verbatim 渲染);沉淀时 push 进 steps,被新 step 替换
```

invariant:
- 传统模式:`steps.is_empty() && last_step.is_none() && !accumulated.is_empty()`(每轮 push 一对 AiMessage)
- Behavior 模式:`accumulated.is_empty()`,`steps` / `last_step` 由外层 loop 维护
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
        // budget / wallclock 检查(外层 step 维度)
        if let Some(o) = self.check_wallclock_budget() { return o; }

        // 1. 跑一个内层传统 LLMContext,得到 response
        let response = match self.run_inner_for_step().await {
            Ok(resp) => resp,
            Err(outer_outcome) => return outer_outcome,   // 内层错误 / budget / yield 已被翻译成外层 outcome
        };

        // 2. parser 解析得到 LLMBehaviorResult;失败走 FeedAsObservation(包成 error step)
        let result = match self.deps.result_parser.as_ref().unwrap().parse(&response) {
            Ok(r) => r,
            Err(e) => {
                let err_step = StepRecord::from_parse_error(&e);
                self.sediment(err_step);
                if let Some(o) = self.bump_consecutive_errors().await { return o; }
                continue;
            }
        };

        // 3. 包成 StepRecord(此时只有意图槽,action_result 还没填)
        let mut new_step = StepRecord::from_result(result);

        // 4. terminal 检查:next_behavior 填了就立刻终结,action 不执行
        if new_step.next_behavior.is_some() {
            return self.finish_done_behavior(new_step, response).await;
        }

        // 5. 派发 action(如果有);没有 action 也算终结(纯思考步等价 ReAct 自然收敛)
        if let Some(action) = new_step.action.clone() {
            let action_result = self.deps.tools.call_tool(action).await;
            new_step.action_result = Some(action_result);
        } else {
            return self.finish_done_behavior(new_step, response).await;
        }

        // 6. 沉淀:last_step 入 steps,new_step 上位;触发可选压缩
        self.sediment(new_step);
        self.maybe_compress().await;
    }
}

fn sediment(&mut self, new_step: StepRecord) {
    if let Some(prev) = self.state.last_step.replace(new_step) {
        self.state.steps.push(prev);
    }
}
```

### 6.3 内层调用 `run_inner_for_step`

每个 step 一次 sub-run。内层 deps 复用外层 llm/tools/policy/worklog/tokenizer,**剥掉** parser/renderer/compressor;内层 request 由外层渲染历史得到。

```rust
async fn run_inner_for_step(&mut self) -> Result<AiResponseSummary, LLMContextOutcome> {
    let inner_deps = self.deps.clone().into_traditional();    // 去掉 result_parser/step_renderer/history_compressor
    let inner_req = self.build_inner_request();               // 见 6.4

    let mut inner = LLMContext::new(inner_req, inner_deps);
    let outcome = inner.run_inner().await;

    // 内层 outcome 翻译
    match outcome {
        LLMContextOutcome::Done { response, usage, .. } => {
            self.merge_inner_usage(usage);
            Ok(response)
        }
        // 研发期 D7:内层 yield 视为外层 fatal
        LLMContextOutcome::WaitInput { .. }
        | LLMContextOutcome::PendingTool { .. }
        | LLMContextOutcome::ContextLimitReached { .. } => {
            Err(LLMContextOutcome::Error {
                error: LLMComputeError::Internal("inner LLMContext yielded; not supported in v1".into()),
                usage: self.state.usage.clone(),
            })
        }
        LLMContextOutcome::Error { error, .. } => Err(LLMContextOutcome::Error {
            error,
            usage: self.state.usage.clone(),
        }),
        LLMContextOutcome::BudgetExhausted { which, partial, .. } => Err(LLMContextOutcome::BudgetExhausted {
            which,
            partial,
            usage: self.state.usage.clone(),
        }),
    }
}
```

### 6.4 内层 request 装配 `build_inner_request`

外层 step_renderer 在这里被使用——把 `[steps] + last_step` 渲染成 AiMessage 序列,与外层 `request.input` (system + user_init) 拼接,作为内层 input。

```rust
fn build_inner_request(&self) -> LLMContextRequest {
    let renderer = self.deps.step_renderer.as_ref().unwrap();
    let mut messages = self.request.input.clone();                    // system + user_init
    messages.extend(renderer.render_history(self.state.steps.clone()));
    if let Some(ref last) = self.state.last_step {
        let (assistant_msg, user_msg) = renderer.render(last);
        messages.push(assistant_msg);
        messages.push(user_msg);
    }

    LLMContextRequest {
        input: messages,
        tool_policy: self.request.tool_policy.clone(),                 // 内层照原配
        output: self.request.output.clone(),                           // OutputSpec::Json + step_schema
        budget: derive_inner_budget(&self.request.budget),             // 见 6.5
        error_policy: self.request.error_policy.clone(),
        // owner / trace / model_policy / human_policy 沿用
        ..self.request.clone()
    }
}
```

最后一条永远是 user(last_step 的 action_result 渲染),内层 inference 自然产 assistant —— alternation 不破。

### 6.5 内外层 budget / usage / trace 关系

- **Budget**:外层 BudgetSpec 派生出内层 BudgetSpec(`derive_inner_budget`):剩余 token / wallclock 按当前预估 step 数分摊;或者简单做法——内层 budget 复用外层值,外层每轮自己再 check 一次外层维度 cap。
- **Usage**:每次内层 Done 把 usage 合并进外层(`merge_inner_usage`)。
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

外层 `run_behavior` 每次沉淀完成后调 `maybe_compress`:`state.steps.len()` 或 token 估算超阈值 → 调 `history_compressor.compress(steps, budget)` → 用返回值替换 `state.steps`。**只压缩 `steps`,`last_step` 不动**(热数据保 verbatim)。

这条路径和 `ResumeFill::RewrittenHistory` 是同一套语义(整段历史被替换),只是触发方在外层 loop 内部还是外部 scheduler。

压缩产物**仍是 StepRecord**,渲染形态不变。一条"摘要 step"长这样:

```rust
StepRecord {
    assistant_text: "[Steps 1-15 compressed]".into(),
    action_result: Some(Observation::Success { content: "<summary>".into(), .. }),
    action: None,
    ..Default::default()
}
```

被 `render_history` 渲染成 `assistant("[Steps 1-15 compressed]") + user("<summary>")` 或 renderer 自行决定的紧凑形态,alternation 不破。

默认提供两个实现:

- **机械压缩**:保留最早 K 条 + 最近 M 条,中间合并为一条 summary StepRecord。完全无 LLM 介入。
- **LLM 压缩**:复用现有 [llm_compress.rs](src/frame/llm_context/src/llm_compress.rs)。

选择权在 worksession 注入哪个实例。

### Resume

研发期(D7)内层不允许 yield,所以**所有 yield 都来自外层 `run_behavior`**——位置只可能在两次 step iteration 之间(沉淀完成 / 压缩后 / 派发 action 前后)。这把 snapshot 边界简化到只看外层 state:`steps + last_step + usage`。

`ResumeFill` 加一个变体,跟 `RewrittenHistory` 平级:

```rust
ReplaceSteps { steps: Vec<StepRecord> },
```

`HumanInput { message }` 在 Behavior 模式下:把 message 包成一条 `StepRecord { assistant_text: "", action_result: Some(Observation::Success { content: text, .. }), .. }`,直接进 `last_step` 槽(模拟"用户回答即上一步的 action_result"语义)。

`ResumeFromMidRun` 校验语义扩展:
- 传统模式:`accumulated.is_empty()` 不要求;`pending_tool_calls.is_empty()` 要求。
- Behavior 模式:`accumulated.is_empty()` 要求;`pending_tool_calls.is_empty()` 要求(因为内层不允许 yield,外层快照点只在 step 边界,这两者必然空)。

---

## 8. worksession 侧的心智模型

```
behaviors.run("plan")
└─ loop:
   req = build_request_for(current_behavior)   # system / action whitelist / step_schema
   deps = base_deps.clone()
          .with_tools(action_view_tool_mgr(current_behavior))
          .with_result_parser(step_parser)
          .with_step_renderer(...)
          .with_history_compressor(...)
   ctx = LLMContext::new(req, deps)
   match ctx.run().await {                     # 走 run_behavior 分支
       Done { behavior_result: Some(r), .. } => {
           audit.push(r);
           match r.next_behavior.as_deref() {
               None        => break BehaviorResult::End,            # ReAct 自然收敛
               Some("END") => break BehaviorResult::End,            # 协议级 END(约定,loop 不知道)
               Some(name)  => current_behavior = name,              # 跳转,新 LLMContext
           }
       }
       WaitInput { snapshot, .. } => return BehaviorResult::WaitForMsg(snapshot),
       // ...
   }
```

LLMContext 是一次性 run-to-END;状态机在 worksession,不在 loop 内。**两层嵌套结构同构**:worksession 每次起一个 Behavior LLMContext,Behavior LLMContext 每个 step 起一个传统 LLMContext——都遵循"一次 run 一次决策"语义。

---

## 9. 故意不做 / 划在范围外

- **不**改 `run_inner`(传统 Agent Loop)。Behavior Loop 用它当子例程,不动它的代码。
- **不**在 loop 内做 behavior 切换。一次 run 一个 behavior。
- **不**新建 ActionMgr trait。Action 通过 ToolMgr 装配实现。
- **不**改 `AiMessage` 定义,**不**进 `buckyos_api`。
- **不**在 `LLMContextOutcome` 加新变体。`Done` 加字段即可。
- **不**在传统模式路径上加任何额外开销——所有 Behavior 字段是 `Option`,`None` 走老路。
- **不**支持内层 yield(D7)。Snapshot 嵌套留到后续。
- **不**冻结 Snapshot schema。研发期允许 breaking。
- **不**允许一个 step 多 action 并发(v1 限制一个 step 至多一个 action;后续升级 `action_result: Vec<Observation>`)。

---

## 10. 实施顺序(参考)

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
   - 内层 yield → 外层 fatal 翻译路径

Snapshot/Resume 细节留到 7 之后再敲定。
