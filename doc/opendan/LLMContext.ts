// =============================================================================
// LLMContext — 伪代码（TypeScript 表达，不可运行，仅作设计校对）
//
// 对应文档：doc/opendan/LLM Context 设计.md
// 心智模型：LLMContext = LLM 执行的进程上下文（PCB）
//
// 重点不是把所有类型写齐，重点是把 run() / resume() 这个 **核 loop** 的规则
// 一字一字落到代码上。任何字段是否值得进 waist，回到 Preamble 双中立性测试。
// =============================================================================


// =============================================================================
// §3.2 / 3.9  不可变输入
// =============================================================================

type ContextOwnerRef =
  | { kind: "Agent"; sessionId: string }
  | { kind: "Workflow"; instanceId: string; nodeId: string }
  | { kind: "OneShot"; id: string; label: string };

type ContextInput =
  | { kind: "Text"; text: string }
  | { kind: "Messages"; messages: ChatMessage[] }
  | { kind: "Structured"; value: unknown };

type PromptSpec =
  | { kind: "Prebuilt"; system: ChatMessage[]; user: ChatMessage[] }
  | {
      kind: "Compiled";
      roleMd?: string;
      selfMd?: string;
      behaviorMd?: string;
      lastStepMd?: string;
      sources: ContextSources;        // 见 §3.9，回调取数据
    };

type ToolPolicy = {
  mode: "None" | "Whitelist" | "All";
  whitelist: string[];
  maxRounds: number;                  // 0 = 单次推理即返回
  maxCallsPerRound: number;
  maxObservationBytes: number;
  deferred: string[];                 // §3.5：触发 PendingTool 的工具
  parallel?: boolean;                 // §11.1 待决，默认 false
};

type OutputSpec =
  | { kind: "Text" }
  | { kind: "Json"; schema?: unknown; strict: boolean }
  | { kind: "Xml"; root: string; strict: boolean }
  | { kind: "BehaviorLLMResult" };    // 过渡兼容 Agent 行为机


// =============================================================================
// LoopPolicy —— **核 loop 的终止判据 + 累积单位**
//
// 我们在系统里实际同时存在两套 loop 习惯，必须在 waist 里都能表达：
//
//   ┌──────────────────────────────────────────────────────────────────────┐
//   │  Mode = "FunctionCall"   —— 标准 LLM 工具循环                         │
//   │  - 终止判据：本次 inference **没有 tool_calls**                       │
//   │  - 累积单位：state.messages（assistant + tool observation 一路堆）    │
//   │  - 一句话：LLM 不再 call function 即视为任务结束                      │
//   │  适用：workflow 简单 LLM 节点 / OneShot / 单段问答                    │
//   ├──────────────────────────────────────────────────────────────────────┤
//   │  Mode = "Step"           —— OpenDAN 既有的 behavior 步进循环          │
//   │  - 终止判据：assistant 输出里出现显式 **behavior_end 信号**           │
//   │    （XML 模式下是 <behavior_end/> 之类的 sentinel，JSON 模式下是      │
//   │     output.next_behavior == "END" 之类的字段）                        │
//   │  - 累积单位：state.steps（每次 inference + 该轮 tool_calls/observations│
//   │    打包成一条 StepRecord）；messages 仍然存在，作为下一步 inference   │
//   │    的输入，但产物语义在 StepRecord 维度                              │
//   │  - 关键差异：**就算这一步没有 tool_calls，loop 也不一定结束** ——      │
//   │    LLM 可以连续多步纯思考 / 改写 memory / 切 next_behavior，直到自己   │
//   │    显式按下"结束键"才退出                                             │
//   │  适用：Agent 行为机（Jarvis）现状；多步规划/反思的 workflow 节点      │
//   └──────────────────────────────────────────────────────────────────────┘
//
// 双中立性体检：
//   - Scheduler 中立：FunctionCall 让 workflow / OneShot 自然，Step 让 Agent /
//     多步反思自然，两条都不偏向某个 scheduler。选哪种是 owner 在构造
//     LLMContextRequest 时声明的契约，不是 scheduler 内置假设。
//   - Provider 中立：sentinel 由 OutputSpec（XML root tag / JSON field name）
//     决定，waist 只看 parsed 出来的"是否 end"位，不绑某家模型的 stop_reason。
//
// **Step 不等于 next_behavior**：next_behavior（"切到下一个行为机"）是
// Agent 专属语义，按 Appendix A.1 不进 waist；Step 这里只关心"这一段
// LLMContext 何时结束"这个原子能力，沿用 OS 类比就是"进程的 main() return"
// 与"call printf() return"的区别。
// =============================================================================

type LoopPolicy =
  | { kind: "FunctionCall" }
  | {
      kind: "Step";
      maxSteps: number;                 // 整次 LLMContext 允许的最大步数
      endSignal: StepEndSignal;         // 如何在 assistant 输出里识别 end
      recordObservations: boolean;      // StepRecord 是否内联 tool observation
    };

type StepEndSignal =
  /** XML 输出里出现该 root 子节点即认定结束（OpenDAN 现状） */
  | { kind: "XmlTag"; tag: string }                    // e.g. "behavior_end"
  /** JSON 输出里某字段为真值即认定结束 */
  | { kind: "JsonField"; path: string; truthy: true }  // e.g. "control.end"
  /** OutputSpec::BehaviorLLMResult 时的内置识别（兼容路径） */
  | { kind: "BehaviorLLMResult" };

/**
 * 一次 inference + 该轮 tool 调用 + observation 的快照。
 * Step 模式下每轮一份；FunctionCall 模式下不产生（messages 已是全部）。
 *
 * 注意：StepRecord 是 worklog 之外的、给上层（Agent loop / Workflow）做
 * "回放、调试、审计、UI 展示"用的中间产物。它不是 worklog 的替代，
 * worklog 仍旧 emit 事件流。
 */
type StepRecord = {
  index: number;
  startedAtMs: number;
  endedAtMs: number;
  assistant: unknown;                 // 这一步 LLM 的原始 assistant 消息
  toolCalls: ToolCall[];
  observations: ToolObservation[];    // recordObservations=false 时为 []
  usage: TokenUsage;
  endSignaled: boolean;               // 这一步是否携带了 end 信号
};

type BudgetSpec = {
  maxTotalTokens?: number;
  maxCompletionTokens?: number;
  maxWallclockMs?: number;
  maxHpCost?: number;
  onExhausted: "Fail" | "ReturnPartial" | "EscalateHuman";
};

type HumanPolicy = {
  approvalRequired: string[];
  allowRequestInput: boolean;
  waitTimeoutMs?: number;
};

interface ContextSources {
  renderTemplateVar(key: string): string | undefined;
  memoryBlock(tokenBudget: number): string | undefined;
  worklogBlock(tokenBudget: number): string | undefined;
  historyTail(tokenBudget: number): string | undefined;
}

type LLMContextRequest = {
  owner: ContextOwnerRef;
  trace: SessionRuntimeContext;
  objective: string;
  input: ContextInput;
  prompt: PromptSpec;
  toolPolicy: ToolPolicy;
  output: OutputSpec;
  loop: LoopPolicy;                   // 见上方：终止判据 + 累积单位的契约
  modelPolicy: ModelPolicy;
  limits: StepLimits;                 // 单步硬限：deadline / tool round
  budget: BudgetSpec;                 // 整次执行预算
  humanPolicy: HumanPolicy;
};


// =============================================================================
// §3.1 / state.rs  可变运行态（= 进程的寄存器+栈）
//
// snapshot = request（代码段，不变）+ state（寄存器+栈，可序列化）。
// 必须自包含：调度器拿着它跨进程/跨节点重启都能 resume。
// =============================================================================

type LLMContextState = {
  // tool loop 已累积的 messages（含 system / user / assistant / tool 观察）
  messages: ChatMessage[];

  // 还能用几轮 tool round（= toolPolicy.maxRounds 起步，每轮 -1）
  roundsLeft: number;

  // 已消耗
  usage: TokenUsage;
  hpUsed: number;
  startedAtMs: number;

  // 上一次 LLM 输出里待填回的 tool_call_id（PendingTool 挂起时非空）
  pendingCallIds: string[];

  // 编译产物缓存（PromptSpec::Compiled 第一次调用 ContextSources 后落定，
  // 防止 resume 后 ContextSources 数据漂移导致 prompt 重新展开成另一份）
  compiledPromptDigest?: string;

  // —— 仅 Step 模式使用 ————————————————————————————————————————————————
  // 已完成的步进记录。snapshot 必须把它一并带走，resume 才能续计 maxSteps、
  // 以及让上层 UI 在恢复后还能看到完整的 step 流。
  steps: StepRecord[];
  stepsLeft: number;                  // = LoopPolicy.Step.maxSteps 起步，每完成一步 -1
};

type LLMContextSnapshot = {
  request: LLMContextRequest;
  state: LLMContextState;
};

type ResumeFill =
  | { kind: "ToolResults"; observations: ToolObservation[] }   // PendingTool 回填
  | { kind: "HumanInput"; input: ContextInput };               // WaitInput 回填


// =============================================================================
// §4  退出态：5 态并集
//
// 注意 Done / Error / BudgetExhausted 是终态，对象消耗；
//      WaitInput / PendingTool 是 cooperative yield，必须吐出 snapshot。
// =============================================================================

type LLMContextOutcome =
  | {
      kind: "Done";
      output: ContextOutput;
      usage: TokenUsage;
      tracking: LLMTrackingInfo;
      steps?: StepRecord[];           // Step 模式时填，FunctionCall 模式 undefined
    }
  | {
      kind: "WaitInput";
      reason: string;
      promptToHuman?: string;
      snapshot: LLMContextSnapshot;
      deadlineMs?: number;
    }
  | {
      kind: "PendingTool";
      pending: PendingToolCall[];
      snapshot: LLMContextSnapshot;
      deadlineMs?: number;
    }
  | {
      kind: "BudgetExhausted";
      which: "Tokens" | "Wallclock" | "HP" | "ToolRounds";
      partial?: ContextOutput;
      usage: TokenUsage;
    }
  | { kind: "Error"; error: LLMComputeError; usage: TokenUsage };


// =============================================================================
// §3.1 / context.rs  LLMContext 主对象
//
// 关键不变量（写在这里，run() 里照着执行）：
//
//   I1. 所有 yield 都是 cooperative。LLM inference 一旦开始就跑完，
//       不在 token stream 中途切换。check 点只在「整段 inference 之后」。
//
//   I2. yield 必须配合 snapshot。任何返回 WaitInput / PendingTool 的路径
//       都要先把当前 state 落盘成 snapshot，否则等于丢进程。
//
//   I3. 终态不吐 snapshot。Done / Error / BudgetExhausted 不需要恢复。
//       BudgetExhausted::ReturnPartial 也只吐 partial output，不吐 snapshot。
//
//   I4. budget 检查与 tool round 检查是两条独立绳子，谁先绷断算谁的。
//       limits（单步） vs budget（整次）并存，分别归类到 ToolRounds /
//       Tokens / Wallclock / HP。
//
//   I5. 一次 LLM 推理产生的 tool_calls，要么本轮全部派发完（同步部分先跑、
//       deferred 部分一并打包成 PendingTool 一次性挂起），要么 0 个 tool
//       直接进 parse → Done。不存在「只派发一半就回到 LLM」。
//       —— §11.2 决议：任一 deferred 即整轮挂起，把同轮非 deferred 的
//       observations 也一起塞进 snapshot.state.messages 带回。
//
//   I6. worklog / policy / tool 执行 / aicc 都走既有通道（§5 映射表）。
//       LLMContext 不引入第二条调用路径。
//
//   I7. resume 不重建 prompt。resume 只把 ResumeFill 拼成新的
//       observation message，复用原 state.messages 与 compiledPromptDigest。
//       如果调度器想重新编 prompt，那是 owner 的事，构造一个新 LLMContext。
//
//   I8. **终止判据由 LoopPolicy 决定，不由代码隐式假设**：
//        - FunctionCall：本次 inference toolCalls 为空 ⇒ 终止
//        - Step       ：本次 inference 的 assistant 输出里命中 endSignal ⇒ 终止
//       两种模式下"还能继续吗"的语义完全不同，混在一个分支里最容易错。
//       Step 模式即便没有 tool_calls 也可能继续（纯思考步），FunctionCall
//       模式即便有未触发的 end sentinel 也只看 tool_calls。
//
//   I9. **累积单位也由 LoopPolicy 决定**：
//        - FunctionCall：只累积 state.messages，不产 StepRecord
//        - Step       ：每个 step 边界产 1 条 StepRecord 进 state.steps，
//          messages 仍然继续累积作为下一步的输入
//       这避免上层在两套 owner 里做翻译：FunctionCall 的 owner 看 messages，
//       Step 的 owner 看 steps，各取所需。
// =============================================================================

class LLMContext {
  private request: LLMContextRequest;
  private state: LLMContextState;
  private deps: LLMContextDeps;

  constructor(req: LLMContextRequest, deps: LLMContextDeps) {
    this.request = req;
    this.deps = deps;
    this.state = {
      messages: [],
      roundsLeft: req.toolPolicy.maxRounds,
      usage: zeroUsage(),
      hpUsed: 0,
      startedAtMs: nowMs(),
      pendingCallIds: [],
      steps: [],
      stepsLeft: req.loop.kind === "Step" ? req.loop.maxSteps : 0,
    };
  }

  // ---------------------------------------------------------------------------
  // resume：从 snapshot 恢复（context switch in）
  //
  // 关键：不重新编 prompt，不动 state.messages 历史，只把回填内容**追加**
  // 成新的 message，然后让 run() 继续 loop。
  // ---------------------------------------------------------------------------
  static resume(
    snapshot: LLMContextSnapshot,
    fill: ResumeFill,
    deps: LLMContextDeps,
  ): LLMContext {
    const ctx = Object.create(LLMContext.prototype) as LLMContext;
    ctx.request = snapshot.request;
    ctx.state = structuredClone(snapshot.state);
    ctx.deps = deps;

    if (fill.kind === "ToolResults") {
      // 校验：observations 的 call_id 必须是 state.pendingCallIds 的子集
      assertCallIdsMatch(ctx.state.pendingCallIds, fill.observations);
      for (const obs of fill.observations) {
        ctx.state.messages.push(toolObservationMessage(obs));
      }
      ctx.state.pendingCallIds = [];          // 清空 pending，loop 可以继续
    } else {
      // HumanInput：把人填的内容拼成一个 user message
      ctx.state.messages.push(humanInputMessage(fill.input));
    }
    return ctx;
  }

  snapshot(): LLMContextSnapshot {
    return { request: this.request, state: structuredClone(this.state) };
  }

  // ===========================================================================
  // run() —— **核 loop**。这是整个 LLMContext 的全部业务。
  //
  // 用文字写一遍执行规则（与下面代码一一对应）。所有"是否继续"的判定都
  // 走 LoopPolicy 分支（I8 / I9），任何隐含假设都是 bug。
  //
  //   R0. 进入 loop 前确保 prompt 已就绪（首次 run 时编译；resume 时已是续跑，
  //       compiledPromptDigest 已锁定，跳过编译）。
  //
  //   R1. 每次 loop 顶端先做 budget 检查（I4）：
  //         - wallclock / tokens / HP / toolRounds / steps 任一耗尽 →
  //           按 onExhausted 返回 BudgetExhausted，loop 结束。
  //
  //   R2. 调一次 inference：do_inference_once(messages)。
  //         - 把 returned tokens 累加进 state.usage。
  //         - 把 inference.assistant push 进 state.messages（这是后续无论哪条
  //           分支都要做的：它是 tool observation 的锚点，也是下一步的输入）。
  //
  //   R3. **终止判据**（I8）—— 按 LoopPolicy 分支：
  //         FunctionCall：
  //           - inference.toolCalls 为空 ⇒ 走 R6 终结。
  //           - 否则 ⇒ 走 R4 处理工具。
  //         Step：
  //           - assistant 命中 endSignal ⇒ 把当前 inference 也封一条 StepRecord
  //             （endSignaled = true，可能 toolCalls 非空但本设计选择忽略之 ——
  //             "我宣布结束"优先于"我还想再调一个工具"，避免歧义），走 R6 终结。
  //           - 否则继续往下。
  //
  //   R3a. 人工输入挂起（任一模式都适用，cooperative yield 出口 1）：
  //         - inference.requestHumanInput && humanPolicy.allowRequestInput
  //           ⇒ 落 snapshot, 返回 WaitInput, loop 结束。
  //         - allowRequestInput == false ⇒ 降级走正常路径（不挂起）。
  //
  //   R4. 一次 tool round（I5：要么整轮派发，要么不动）：
  //         a. 经 PolicyEngine.gate_tool_calls 过滤（policy 驳回 → 写 worklog,
  //            视严重度走 Error 或者把驳回理由当作 observation 让 LLM 重试）。
  //         b. 切两堆：sync_calls / deferred_calls。
  //         c. 先把 sync_calls 跑了（串行；toolPolicy.parallel = true 才并发）。
  //            每个结果 emit(ToolCallFinished) 并塞进 state.messages。
  //         d. 若 deferred_calls 非空 → 把同轮 sync 结果带回 messages 后，
  //            将 deferred 打包，写 state.pendingCallIds，落 snapshot，
  //            返回 PendingTool，loop 结束。
  //         e. 否则全同步完成。
  //
  //   R5. **步进登记**（I9）—— 仅 Step 模式：
  //         - 把本次 inference + 该轮 toolCalls/observations 封成 StepRecord，
  //           push 进 state.steps；stepsLeft -= 1。
  //         - FunctionCall 模式跳过此节，只在 R4e 后做 roundsLeft -= 1。
  //         - 注意：Step 模式下"没有 tool_calls"也是一步（纯思考步），同样
  //           要登 StepRecord、扣 stepsLeft，然后回到 R1。
  //
  //   R6. **终结**（按 OutputSpec 解析）：
  //         - parse(最后一次 assistant 消息, OutputSpec)
  //         - 解析失败 → 返回 Error（usage 仍带回，I3）。
  //         - 解析成功 → emit(LLMFinished)；
  //           Step 模式把 state.steps 一并放进 Outcome::Done.steps；
  //           FunctionCall 模式 Outcome::Done.steps = undefined。
  // ===========================================================================
  async run(): Promise<LLMContextOutcome> {
    // ---- R0：prompt 编译（仅首次） -----------------------------------------
    if (this.state.messages.length === 0) {
      this.state.messages = await this.compilePrompt(this.request.prompt);
      this.deps.worklog.emit({ kind: "LLMStarted", owner: this.request.owner });
    }

    const loop = this.request.loop;

    // ---- 主循环 -------------------------------------------------------------
    while (true) {
      // R1. budget gate（每轮顶端检查，I4）
      const budgetHit = this.checkBudget();
      if (budgetHit) {
        return this.makeBudgetExhausted(budgetHit);
      }

      // R2. 调一次 LLM
      const stepStartMs = nowMs();
      let inference: InferenceResult;
      try {
        inference = await this.deps.aicc.doInferenceOnce({
          messages: this.state.messages,
          modelPolicy: this.request.modelPolicy,
          limits: this.request.limits,
        });
      } catch (e) {
        return { kind: "Error", error: toLLMComputeError(e), usage: this.state.usage };
      }
      this.state.usage = mergeUsage(this.state.usage, inference.usage);
      this.deps.worklog.emit({ kind: "LLMRoundFinished", usage: inference.usage });

      // assistant 消息无论后面走哪条分支都先入栈（既是 tool 观察的锚点，
      // 也是 Step 模式下下一步的输入）
      this.state.messages.push(assistantMessage(inference.assistant));

      // R3a. 人工输入挂起（cooperative yield 出口 1，两种模式通用）
      if (
        inference.requestHumanInput &&
        this.request.humanPolicy.allowRequestInput
      ) {
        return {
          kind: "WaitInput",
          reason: inference.requestHumanInput.reason,
          promptToHuman: inference.requestHumanInput.prompt,
          snapshot: this.snapshot(),
          deadlineMs: this.request.humanPolicy.waitTimeoutMs
            ? nowMs() + this.request.humanPolicy.waitTimeoutMs
            : undefined,
        };
      }

      // R3. **终止判据**（I8）—— 分模式判断
      const endSignaled =
        loop.kind === "Step" && detectEndSignal(inference.assistant, loop.endSignal);

      if (loop.kind === "FunctionCall") {
        // FunctionCall：tool_calls 为空即终止
        if (!inference.toolCalls || inference.toolCalls.length === 0) {
          return this.finalize(inference, /*steps*/ undefined);
        }
      } else {
        // Step：endSignal 命中即终止；命中时本步的 tool_calls 被本设计忽略
        // （"我宣布结束"优先于"我还想再调一个工具"，避免歧义）
        if (endSignaled) {
          this.recordStep(inference, [], stepStartMs, /*endSignaled*/ true);
          return this.finalize(inference, this.state.steps);
        }
      }

      // R4. 处理 tool_calls（如果有）
      let observations: ToolObservation[] = [];
      const hasToolCalls = !!inference.toolCalls && inference.toolCalls.length > 0;

      if (hasToolCalls) {
        // R4a. policy gate
        const gated = this.deps.policy.gateToolCalls({
          owner: this.request.owner,
          calls: inference.toolCalls!,
          toolPolicy: this.request.toolPolicy,
          humanPolicy: this.request.humanPolicy,
        });
        if (gated.kind === "RejectFatal") {
          return {
            kind: "Error",
            error: { code: "PolicyRejected", message: gated.reason },
            usage: this.state.usage,
          };
        }
        // 软驳回：把 reason 当观察喂回 LLM，下一轮重试（不视作完整一步）
        if (gated.kind === "RejectSoft") {
          for (const r of gated.observations) {
            this.state.messages.push(toolObservationMessage(r));
          }
          if (loop.kind === "FunctionCall") {
            if (--this.state.roundsLeft <= 0) {
              return this.makeBudgetExhausted("ToolRounds");
            }
          } else {
            // Step 模式：软驳回也封一步，便于审计；但不消耗 stepsLeft 上限的逻辑
            // 由 owner 通过 maxSteps 自然约束（这里仍 -1，保持简单）
            this.recordStep(inference, [], stepStartMs, false);
            if (--this.state.stepsLeft <= 0) {
              return this.makeBudgetExhausted("ToolRounds");
            }
          }
          continue;
        }

        // R4b/c. 切两堆，先跑同步
        const { syncCalls, deferredCalls } = partitionCalls(
          gated.calls,
          this.request.toolPolicy.deferred,
        );
        observations = await this.runSyncCalls(syncCalls);
        for (const obs of observations) {
          this.state.messages.push(toolObservationMessage(obs));
        }

        // R4d. 任一 deferred 即整轮挂起（§11.2），同步结果已带回 messages
        if (deferredCalls.length > 0) {
          // 注意：deferred 时不在这里登 StepRecord —— 这一步还没"做完"，
          // resume 后才会在下一轮统一处理。pending 在 messages 里有锚点。
          this.state.pendingCallIds = deferredCalls.map(c => c.callId);
          return {
            kind: "PendingTool",
            pending: deferredCalls.map(toPendingToolCall),
            snapshot: this.snapshot(),
            deadlineMs: this.computeDeadline(),
          };
        }
      }

      // R5. 步进登记 + 计数器扣减
      if (loop.kind === "Step") {
        this.recordStep(inference, observations, stepStartMs, /*endSignaled*/ false);
        if (--this.state.stepsLeft <= 0) {
          return this.makeBudgetExhausted("ToolRounds");
        }
      } else {
        // FunctionCall：只有发生过 tool round 的迭代才扣 roundsLeft
        if (hasToolCalls) {
          if (--this.state.roundsLeft <= 0) {
            return this.makeBudgetExhausted("ToolRounds");
          }
        }
        // FunctionCall + 无 toolCalls 这条已经在 R3 里 finalize 了，此处不应到达
      }
      // continue → 下一轮 inference
    }
  }

  // ---------------------------------------------------------------------------
  // R5：把这一步封成 StepRecord 落进 state.steps（仅 Step 模式调用）
  // ---------------------------------------------------------------------------
  private recordStep(
    inf: InferenceResult,
    observations: ToolObservation[],
    startedAtMs: number,
    endSignaled: boolean,
  ): void {
    const policy = this.request.loop;
    if (policy.kind !== "Step") return;
    this.state.steps.push({
      index: this.state.steps.length,
      startedAtMs,
      endedAtMs: nowMs(),
      assistant: inf.assistant,
      toolCalls: inf.toolCalls ?? [],
      observations: policy.recordObservations ? observations : [],
      usage: inf.usage,
      endSignaled,
    });
    this.deps.worklog.emit({ kind: "StepRecorded", index: this.state.steps.length - 1 });
  }

  // ---------------------------------------------------------------------------
  // R6：终结，按 OutputSpec 解析最后一次 assistant 消息
  //   - FunctionCall 模式：steps 传 undefined
  //   - Step 模式：steps 传 state.steps（已经把"终止那一步"也封进去了）
  // ---------------------------------------------------------------------------
  private finalize(inf: InferenceResult, steps?: StepRecord[]): LLMContextOutcome {
    const parsed = parseOutput(inf.assistant, this.request.output);
    if (parsed.kind === "Err") {
      return {
        kind: "Error",
        error: { code: "OutputParseFailed", message: parsed.message },
        usage: this.state.usage,
      };
    }
    this.deps.worklog.emit({ kind: "LLMFinished", owner: this.request.owner });
    return {
      kind: "Done",
      output: parsed.value,
      usage: this.state.usage,
      tracking: buildTracking(this.state, inf),
      steps,
    };
  }

  // ---------------------------------------------------------------------------
  // R1：budget 体检。返回 null 表示通过；否则给出哪根绳子先绷断。
  // ---------------------------------------------------------------------------
  private checkBudget(): "Tokens" | "Wallclock" | "HP" | "ToolRounds" | null {
    const b = this.request.budget;
    if (b.maxTotalTokens && this.state.usage.totalTokens >= b.maxTotalTokens) return "Tokens";
    if (b.maxCompletionTokens && this.state.usage.completionTokens >= b.maxCompletionTokens) return "Tokens";
    if (b.maxWallclockMs && nowMs() - this.state.startedAtMs >= b.maxWallclockMs) return "Wallclock";
    if (b.maxHpCost && this.state.hpUsed >= b.maxHpCost) return "HP";
    if (this.state.roundsLeft < 0) return "ToolRounds";
    return null;
  }

  private makeBudgetExhausted(
    which: "Tokens" | "Wallclock" | "HP" | "ToolRounds",
  ): LLMContextOutcome {
    const action = this.request.budget.onExhausted;
    let partial: ContextOutput | undefined;
    if (action === "ReturnPartial") {
      // 尽力解析最后一次 assistant 消息；解析不出就不带 partial
      partial = bestEffortParse(this.state.messages, this.request.output);
    }
    if (action === "EscalateHuman" && this.request.humanPolicy.allowRequestInput) {
      // 升级成 WaitInput：把预算耗尽信息作为 prompt_to_human
      return {
        kind: "WaitInput",
        reason: `BudgetExhausted:${which}`,
        promptToHuman: `执行预算（${which}）已耗尽，请人工决策是否继续/终止。`,
        snapshot: this.snapshot(),
      };
    }
    return { kind: "BudgetExhausted", which, partial, usage: this.state.usage };
  }

  // ---------------------------------------------------------------------------
  // R4c：同步 tool 调用执行
  //   - 默认串行（§11.1 待决，倾向 false）
  //   - 单个 observation bytes 截断到 toolPolicy.maxObservationBytes
  // ---------------------------------------------------------------------------
  private async runSyncCalls(calls: ToolCall[]): Promise<ToolObservation[]> {
    const out: ToolObservation[] = [];
    if (this.request.toolPolicy.parallel) {
      const results = await Promise.all(calls.map(c => this.runOne(c)));
      out.push(...results);
    } else {
      for (const c of calls) out.push(await this.runOne(c));
    }
    return out;
  }

  private async runOne(call: ToolCall): Promise<ToolObservation> {
    const obs = await this.deps.tools.callTool(call);
    return truncateObservation(obs, this.request.toolPolicy.maxObservationBytes);
  }

  // ---------------------------------------------------------------------------
  // R0：PromptSpec → ChatMessage[]。
  //   Prebuilt：直接拼。
  //   Compiled：调 PromptBuilder + ContextSources，落 digest 防止 resume 漂移。
  // ---------------------------------------------------------------------------
  private async compilePrompt(spec: PromptSpec): Promise<ChatMessage[]> {
    if (spec.kind === "Prebuilt") {
      return [...spec.system, ...spec.user, ...inputAsMessages(this.request.input)];
    }
    const built = await this.deps.promptBuilder.build({
      roleMd: spec.roleMd,
      selfMd: spec.selfMd,
      behaviorMd: spec.behaviorMd,
      lastStepMd: spec.lastStepMd,
      sources: spec.sources,
      input: this.request.input,
    });
    this.state.compiledPromptDigest = digest(built);
    return built;
  }

  private computeDeadline(): number | undefined {
    const b = this.request.budget;
    return b.maxWallclockMs ? this.state.startedAtMs + b.maxWallclockMs : undefined;
  }
}


// =============================================================================
// §3.1 / deps.rs  LLMContextDeps —— effect 边界
//
// 全部是 trait（接口），具体实现在 waist 之外。Appendix A.4 决议：
// snapshot 存储 / worklog 后端 / tool 任务队列 / 并发限流……都不进 waist。
// =============================================================================

interface LLMContextDeps {
  aicc: AiccClient;                   // 原样复用
  tools: AgentToolManager;            // 原样复用
  policy: PolicyEngine;               // 原样复用
  worklog: WorklogSink;               // 原样复用
  promptBuilder: PromptBuilder;       // 仅 Compiled 用
  tokenizer: Tokenizer;
}


// =============================================================================
// 占位类型（waist 不定义实现，引用 frame/opendan 的既有类型）
// =============================================================================

type ChatMessage = unknown;
type ToolCall = { callId: string; name: string; args: unknown };
type ToolObservation = { callId: string; name: string; output: unknown; bytes: number };
type PendingToolCall = { callId: string; name: string; args: unknown };
type ContextOutput =
  | { kind: "Text"; text: string }
  | { kind: "Json"; value: unknown }
  | { kind: "Xml"; root: string; body: string }
  | { kind: "Behavior"; result: unknown };
type TokenUsage = { totalTokens: number; promptTokens: number; completionTokens: number };
type LLMTrackingInfo = unknown;
type LLMComputeError = { code: string; message: string };
type SessionRuntimeContext = unknown;
type ModelPolicy = unknown;
type StepLimits = unknown;
type InferenceResult = {
  assistant: unknown;
  toolCalls?: ToolCall[];
  usage: TokenUsage;
  requestHumanInput?: { reason: string; prompt?: string };
};
interface AiccClient { doInferenceOnce(req: unknown): Promise<InferenceResult> }
interface AgentToolManager { callTool(call: ToolCall): Promise<ToolObservation> }
interface PolicyEngine {
  gateToolCalls(req: unknown):
    | { kind: "Pass"; calls: ToolCall[] }
    | { kind: "RejectSoft"; observations: ToolObservation[] }
    | { kind: "RejectFatal"; reason: string };
}
interface WorklogSink { emit(ev: unknown): void }
interface PromptBuilder { build(req: unknown): Promise<ChatMessage[]> }
interface Tokenizer { count(s: string): number }


// =============================================================================
// 辅助函数声明（实现略；保留命名以便对照 §6 流程）
// =============================================================================

declare function nowMs(): number;
declare function zeroUsage(): TokenUsage;
declare function mergeUsage(a: TokenUsage, b: TokenUsage): TokenUsage;
declare function inputAsMessages(i: ContextInput): ChatMessage[];
declare function assistantMessage(a: unknown): ChatMessage;
declare function toolObservationMessage(o: ToolObservation): ChatMessage;
declare function humanInputMessage(i: ContextInput): ChatMessage;
declare function toPendingToolCall(c: ToolCall): PendingToolCall;
declare function partitionCalls(
  calls: ToolCall[],
  deferred: string[],
): { syncCalls: ToolCall[]; deferredCalls: ToolCall[] };
declare function truncateObservation(o: ToolObservation, max: number): ToolObservation;
declare function parseOutput(
  assistant: unknown,
  spec: OutputSpec,
): { kind: "Ok"; value: ContextOutput } | { kind: "Err"; message: string };
declare function bestEffortParse(msgs: ChatMessage[], spec: OutputSpec): ContextOutput | undefined;
declare function buildTracking(state: LLMContextState, inf: InferenceResult): LLMTrackingInfo;
declare function toLLMComputeError(e: unknown): LLMComputeError;
declare function digest(x: unknown): string;
declare function assertCallIdsMatch(expect: string[], obs: ToolObservation[]): void;

/**
 * Step 模式的终止哨兵识别：
 *   - XmlTag        ：assistant XML 输出里含 <tag/> 或 <tag>...</tag>
 *   - JsonField     ：assistant JSON 输出里 path 指向的字段为真值
 *   - BehaviorLLMResult：解析为 BehaviorLLMResult 后看 next_behavior == "END"
 * waist 自身不绑死哪一种 sentinel 写法；具体匹配实现挂在 effect 边界外。
 */
declare function detectEndSignal(assistant: unknown, signal: StepEndSignal): boolean;


// =============================================================================
// 流程对照
//
// ── FunctionCall 模式（标准 LLM 工具循环）─────────────────────────────────
//   new(loop=FunctionCall) → run()
//     R0 compilePrompt
//     loop {
//       R1 budget gate
//       R2 inference + push assistant
//       R3 toolCalls 为空? → R6 finalize(steps=undefined) → Done
//       R4 gate / sync / deferred?(→ PendingTool yield)
//       R5 roundsLeft -= 1
//     }
//   终止时 Outcome::Done 不带 steps；上层只看 messages / output。
//
// ── Step 模式（OpenDAN behavior 步进循环）─────────────────────────────────
//   new(loop=Step{ maxSteps, endSignal }) → run()
//     R0 compilePrompt
//     loop {
//       R1 budget gate
//       R2 inference + push assistant
//       R3 endSignal 命中? → recordStep(endSignaled=true) → R6 finalize(steps) → Done
//       R4 gate / sync / deferred?(→ PendingTool yield，不登 step)
//       R5 recordStep + stepsLeft -= 1
//       —— 注意：即便 toolCalls 为空（纯思考步），只要没命中 endSignal
//          就继续下一轮 inference，这是 Step 模式的核心差异
//     }
//   终止时 Outcome::Done.steps = state.steps；上层（Agent loop）按
//   StepRecord 流去更新 session 行为机 / UI / worklog 视图。
//
// ── 通用挂起出口（两种模式共用）────────────────────────────────────────
//   §6.2 PendingTool（cooperative yield 1）：
//     ctx1.run() → R4d 命中 deferred → return PendingTool { snapshot }
//     调度器把 pending 排队，回填后：
//     ctx2 = LLMContext.resume(snapshot, ToolResults) → run() 从 R1 继续
//
//   §6.3 WaitInput（cooperative yield 2）：
//     inference.requestHumanInput && humanPolicy.allowRequestInput
//     → R3a 命中 → return WaitInput { snapshot }
//     ctx2 = LLMContext.resume(snapshot, HumanInput) → run() 从 R1 继续
//
//   两种模式下 snapshot 的差异仅在 state.steps / state.stepsLeft 是否非空，
//   resume 后 run() 回到同一条主循环。
// =============================================================================
