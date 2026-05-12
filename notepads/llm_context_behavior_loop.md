# LLM Context 支持 Behavior Loop —— 瘦腰式扩展方案

## 0. 一句话

把 `context_loop.rs` 当作瘦腰核(`infer → 解析 → 派发 → 累计 → loop`),Behavior Loop 不改这个核,而是通过 **一个解析器 + 一份并行的结构化历史 + 两个可选渲染器** 把传统 Agent Loop 升级为它的超集。两种模式共用一个执行核,靠 deps 注入哪些组件来区分,运行时不混用。

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
- **核心 loop 的三阶段骨架不变**:`build_inference_request` → `llm.infer` → `dispatch + accumulate`。
- **Behavior 模式 vs 传统模式是构造时二选一**,运行时不混用——LLMContext::new 里靠 deps 字段组合做断言。
- **协议解析归 parser,执行归 dispatcher,loop 只读"要不要继续 / 调用什么"两个信号**。

---

## 3. 收敛的关键决策

| # | 决策 | 含义 |
|---|---|---|
| D1 | 双 buffer 并存 | `state.accumulated: Vec<AiMessage>`(传统)和 `state.steps: Vec<StepRecord>`(behavior),invariant: 永远恰好一个非空 |
| D2 | ToolMgr / ActionMgr **同签名**,不引新 trait | Agent Tool 已为 Action 化做好准备;Action 层就是 ToolMgr 实例的另一种装配,Behavior Loop 几乎不配 ToolMgr,而是配一个 action 视图的 ToolMgr |
| D3 | StepRecord 渲染成 `assistant(意图) + user(结果)` 一对 | 喂给 LLM 的结构是 `system(include user_init target)  + [History Steps] + LastStep assistant + Last Step user`;严格 user/assistant 交替,贴合 LLM 训练分布,无 provider alternation workaround |
| D4 | next_behavior 是 terminal 信号 | parser 产出 `next_behavior: Option<String>`,`is_some()` 即 terminal;无单独 `terminal` bool;字符串语义("END" 等)归上层 worksession,loop 不解释 |
| D5 | Snapshot schema 待 Behavior Loop 落定后再冻结 | 研发期间不背向前兼容包袱;但相关结构始终保持 `Serialize`/`Deserialize` derive |

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
pub steps: Vec<StepRecord>,    // Behavior 模式专用;传统模式恒空
```

invariant:`accumulated.is_empty() ^ steps.is_empty()`(初始两者都空,运行中至少一个非空)。

### `LLMContextOutcome::Done`(`outcome.rs`)

新增字段:

```rust
behavior_result: Option<LLMBehaviorResult>,   // 传统模式恒为 None
```

`Done.next_behavior` 不单独提出 —— 上层从 `behavior_result.as_ref().and_then(|r| r.next_behavior.clone())` 读。

---

## 6. `context_loop.rs` 的 4 处切入点

每处都是局部 if/else,不重构主干。

### 切入点 A:`build_inference_request`(组装 messages)

```rust
let messages = if self.is_behavior_mode() {
    let renderer = self.deps.step_renderer.as_ref().unwrap();
    let mut msgs = self.request.input.clone();           // system + user_init,装配阶段已就位
    let history_msgs = renderer.render_history(&self.state.steps)
    msgs.extend(history_msgs)
    let (assistant_msg,user_msg) = renderer.render(&self.state.last_step)
    msgs.extend(assistant_msg,user_msg);        
    msgs
} else {
    self.state.accumulated.clone()
};
```

最后一条永远是 user(最近一步的 action_result 渲染),下次 inference 自然产 assistant —— 严格 alternation 不破。

`tool_specs` 那一行不动:Behavior 模式下装配的 ToolManager 自然返回 action 视图的 specs。

### 切入点 B:`infer` 之后、`dispatch` 之前

```rust
self.account_response(&response);
self.last_response = response.clone();
// budget 检查等不变 ...

let parsed: Option<LLMBehaviorResult> = if let Some(parser) = &self.deps.result_parser {
    match parser.parse(&response) {
        Ok(r) => Some(r),
        Err(e) => {
            // 走现成 FeedAsObservation:把解析失败当成 recoverable error,LLM 下一轮自纠错
            if let Some(o) = self.handle_error(ErrorClass::Recoverable(
                LLMComputeError::OutputParse(e)
            )).await { return o; }
            continue;
        }
    }
} else { None };

// loop 视角的真理源:有 parser 就听 parser
let (tool_calls, terminal_next) = match &parsed {
    Some(r) => (r.tool_calls.clone(), r.next_behavior.clone()),
    None    => (response.tool_calls.clone(), None),
};

if terminal_next.is_some() || tool_calls.is_empty() || self.request.tool_policy.mode == ToolMode::None {
    return self.finish_done(response, parsed).await;
}
```

### 切入点 C:dispatch 后累计历史

传统模式照旧 push `accumulated`;Behavior 模式合成 StepRecord 入 `steps`:

```rust
if self.is_behavior_mode() {
    // parser 已经把 assistant_text / observation / thought / action / next_behavior / raw 填好,
    // 这里只补 action_result 槽
    let mut step: StepRecord = parsed.unwrap().into();
    step.action_result = observations.into_iter().next();   // v1: 单 action / 单 result
    self.state.steps.push(step);
} else {
    self.state.accumulated.push(assistant_tool_call_message(...));
    for obs in observations { self.state.accumulated.push(tool_observation_message(...)); }
}
```

> v1 假设一个 Step 对应一个 action 调用;后续要支持并发 action 时,`action_result` 升级成 `Vec<Observation>`,renderer 的 user 消息部分变成多 observation 聚合。

### 切入点 D:错误反馈路径(`handle_error` 的 FeedAsObservation 分支)

[context_loop.rs:535](src/frame/llm_context/src/context_loop.rs#L535) 那条 `accumulated.push(system error msg)`:

- 传统模式:不动。
- Behavior 模式:把错误塞成一条 `StepRecord { assistant_text: "", action: None, action_result: Some(Observation::Error{...}), .. }`,push 到 `steps`。下一轮 renderer 渲染成 `assistant("") + user(error_text)`(或由 renderer 自行决定的等价形态)。

---

## 7. 压缩与 Resume

### 压缩

`steps.len()` 或 token 估算超阈值 → 调 `history_compressor.compress(steps, budget)` → 用返回值替换 `state.steps`。这条路径和 `ResumeFill::RewrittenHistory` 是同一套语义(整段历史被替换),只是触发方在 loop 内部还是外部 scheduler。

压缩产物**仍是 StepRecord**,渲染形态不变。一条"摘要 step"长这样:

```rust
StepRecord {
    assistant_text: "[Steps 1-15 compressed]".into(),
    action_result: Some(Observation::Success { content: "<summary>".into(), .. }),
    action: None,
    raw: json!({ "kind": "compressed", "dropped": 15 }),
    ..Default::default()
}
```

渲染成 `assistant("[Steps 1-15 compressed]") + user("<summary>")`,alternation 不破。

默认提供两个实现:

- **机械压缩**:保留最早 K 条 + 最近 M 条,中间合并为一条 summary StepRecord。完全无 LLM 介入。
- **LLM 压缩**:复用现有 [llm_compress.rs](src/frame/llm_context/src/llm_compress.rs)。

选择权在 worksession 注入哪个实例。

### Resume

`ResumeFill` 加一个变体,跟 `RewrittenHistory` 平级:

```rust
ReplaceSteps { steps: Vec<StepRecord> },
```

`HumanInput { message }` 在 Behavior 模式下:把 message 包成一条 `StepRecord { observation: Some(text), .. }` 入 `steps`。

`ResumeFromMidRun` 校验语义扩展:Behavior 模式下要求 `accumulated.is_empty() && pending_tool_calls.is_empty()`。

---

## 8. worksession 侧的心智模型

```
behaviors.run("plan")
└─ loop:
   req = build_request_for(current_behavior)   # system / action whitelist / step_schema
   deps = base_deps.clone()
          .with_tools(action_view_tool_mgr(current_behavior))
          .with_result_parser(step_parser)
          .with_history_renderer(...)
          .with_next_turn_renderer(...)
          .with_history_compressor(...)
   ctx = LLMContext::new(req, deps)
   match ctx.run().await {
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

LLMContext 是一次性 run-to-END;状态机在 worksession,不在 loop 内。

---

## 9. 故意不做 / 划在范围外

- **不**在 loop 内做 behavior 切换。一次 run 一个 behavior。
- **不**新建 ActionMgr trait。Action 通过 ToolMgr 装配实现。
- **不**改 `AiMessage` 定义,**不**进 `buckyos_api`。
- **不**在 `LLMContextOutcome` 加新变体。`Done` 加字段即可。
- **不**在传统模式路径上加任何额外开销——所有 Behavior 字段是 `Option`,`None` 走老路。
- **不**冻结 Snapshot schema。研发期允许 breaking。

---

## 10. 实施顺序(参考)

1. `behavior_loop.rs`:落类型 + trait 签名(无实现)。
2. `deps.rs`:加 4 个 `Option` 字段 + with_xxx 方法。
3. `state.rs`:加 `steps`,加 `is_behavior_mode()` helper。
4. `outcome.rs`:`Done` 加 `behavior_result`。
5. `context_loop.rs`:4 处切入点 + `is_behavior_mode()` 分支。
6. 默认实现:`DefaultStepRenderer`(直接 `assistant_text` + `format(action_result)` 成对)、`MechanicalCompressor`。
7. 测试:传统模式回归 + Behavior 模式基础闭环(一个 dummy parser + dummy ToolMgr 跑两轮)。

Snapshot/Resume 细节留到 7 之后再敲定。
