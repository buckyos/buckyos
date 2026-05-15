//! LLMContext 切换辅助层（设计草案）
//!
//! ## 触发点
//!
//! 一轮 LLMContext 跑完后，session 端需要根据信号（next_behavior / 工具内部
//! 请求 / WaitInput 结束等）造下一个 LLMContext。三种模式描述的是 **session
//! 内部状态机** 给 helper 提供 base snapshot 和 overrides 时的语义差异，
//! 不是暴露给 LLM/工具的并列选项。
//!
//! ## 三种模式的核心语义
//!
//! ### Switch（图结构，全局共享 step stream）
//!
//! - 多个 behavior 共享**同一个** step record stream + accumulated 历史
//! - 切换 behavior 等价于换 system prompt / 工具集，所有历史照旧可见
//! - 切到 B 再切回 A，A 看到完整历史（包括 B 期间产生的 step）
//! - 在 session 内是一张**图**：节点 = behavior name，边 = switch 跳转，
//!   状态绑在边上而不是节点上（节点本身无独立状态）
//! - **next_behavior** 由 LLM 通过 `<next_behavior>` tag 主动声明
//!
//! ### Fork（函数调用 / 协程，父子栈结构）
//!
//! - 父 ctx 在 fork 点挂起，子 ctx 从父的当前快照继承（默认全部继承，
//!   helper 允许 caller 在跑前覆写 env：system prompt / 工具集 / objective）
//! - **不是并行**：父等子，子跑完父继续
//! - 子 ctx **禁止 next_behavior**，只能走到自然 End。这是 fork 的硬约束
//!   ——必须由 waist 强制（见下方未决项）
//! - 子 End 之后控制流回 fork 点，父 ctx 续跑。父**看不到**子的任何 step
//!   record，只看到子的"最终结果"（ContextOutput / behavior_result.text），
//!   类似函数 return value
//! - Fork 可嵌套（子也能 fork），形成调用栈
//! - **fork 是 session 私有原语**，不在 agent tool 暴露面、也不在 tool spec
//!   上做任何标记。session-aware 工具（`try_create_worksession` 等，构造时
//!   拿到 `Weak<AIAgent>` / session 句柄）在自己的 `call()` 实现里直接调
//!   `session.fork_and_run(...)` 这类内部 API，把 sub.output 作为
//!   `Observation::Ok` 返回。
//!   - 99% 的标准 CLI 工具（exec_bash / read_file / ...）走
//!     `AgentToolManager` 静态路径，没有 session 句柄，碰不到 fork
//!   - 不需要 `ToolKind::Fork` 元数据、也不需要新增 `Observation::ForkRequested`
//!     之类的变体；session-aware vs 标准工具在构造层面就已经天然分开
//!
//! ### Independent（并存的多个协程，按 behavior name 隔离 step stream）
//!
//! - 每个 behavior name = 一个独立"进程"，**自己持有 step record stream**
//! - 父进程把控制权交给子进程（switch 到 independent behavior），子用自己
//!   存量的 stream 续跑（首次则从 0 起）
//! - 子 End → 控制权回父进程，父也从自己存量 stream 续跑
//! - 父看不到子内部；但**下次**父再切到同一个子 behavior name 时，
//!   子能看到自己之前的 stream（多次进入是续跑同一进程，不是新建）
//! - **持久化**：每个 behavior 一个独立 snapshot 文件
//!   `.meta/behavior_<name>.snap`；session 在切换时按 name 装载/保存
//! - **next_behavior** 同样由 LLM 主动声明，但语义是"切换进程"而非
//!   "切换 system prompt"
//!
//! ## helper 暴露的原语：2 个函数覆盖 3 种模式
//!
//! ```ignore
//! pub fn rebuild_with_inherit(
//!     base_snap: LLMContextSnapshot,
//!     overrides: RequestOverrides,
//!     deps:      LLMContextDeps,
//! ) -> Result<LLMContext, LLMComputeError>;
//!
//! pub fn build_fresh(
//!     request: LLMContextRequest,
//!     deps:    LLMContextDeps,
//! ) -> LLMContext;
//! ```
//!
//! 三种模式只在 **base snapshot 从哪儿来 / 跑完写到哪 / 控制流去哪** 上不同，
//! helper 本身不区分。session 端调用方式：
//!
//! | 模式        | base snapshot 来源                    | helper 调用          | 跑完写回处                            | 控制流去向                  |
//! |-------------|---------------------------------------|----------------------|---------------------------------------|----------------------------|
//! | switch      | session 当前 `.meta/state.snap`        | rebuild_with_inherit | 覆盖 `.meta/state.snap`               | 留在新 behavior 继续 turn   |
//! | fork        | 父 ctx 当前 snapshot（栈顶 / 内存）    | rebuild_with_inherit | **不写盘**；sub-ctx 跑完即弃          | 子 End → 父续跑（栈弹出）   |
//! | independent | `.meta/behavior_<name>.snap`（按 name）或空 | rebuild_with_inherit / build_fresh | 覆盖 `.meta/behavior_<name>.snap` | 子 End → 父切回（也按 name resume） |
//!
//! `independent` 首次进入某 behavior 时（该 behavior_<name>.snap 不存在）走
//! `build_fresh`；后续进入走 `rebuild_with_inherit`（从该 behavior 的存量
//! snapshot 继承）。
//!
//! ## 需要 llm_context 新增的内部接口
//!
//! 1. 纯数据结构 `RequestOverrides`：
//!    ```ignore
//!    pub struct RequestOverrides {
//!        pub system_messages: Option<Vec<AiMessage>>, // 替换前导 System 段
//!        pub tool_policy:    Option<ToolPolicy>,
//!        pub objective:      Option<String>,
//!        pub trace:          Option<Option<String>>,
//!        pub model_policy:   Option<ModelPolicy>,
//!        pub budget:         Option<BudgetSpec>,
//!        pub human_policy:   Option<HumanPolicy>,
//!        pub error_policy:   Option<ErrorPolicy>,
//!        pub output:         Option<OutputSpec>,
//!
//!        // 计数器旋钮
//!        pub reset_rounds:   bool, // 重置 state.rounds_left = new tool_policy.max_rounds
//!        pub reset_errors:  bool,  // 清 state.consecutive_errors
//!
//!        // Fork 专用硬约束（见下方未决项）
//!        pub forbid_next_behavior: bool, // 子 ctx 必须自然 End，不允许声明 next_behavior
//!    }
//!    ```
//!
//! 2. `LLMContextSnapshot::apply_overrides(self, ov: RequestOverrides) -> Self`
//!    —— 纯 data 函数。同时改 `request` 与 `state.accumulated` 的前导 System 段，
//!    保持两边一致；按 reset_* 决定是否动 state 计数器；把
//!    `forbid_next_behavior` 落到 snapshot 里某个新字段（state 或 request 上）。
//!
//! 先在 helper 文件 inline 做（自由函数 `apply_overrides_to_snapshot` 操作
//! 已暴露的 `LLMContextSnapshot`），跑通后下沉到 llm_context crate。
//!
//! ## 关键 invariant：system 段同步
//!
//! `state.accumulated` 在新建时 = `request.input.clone()`，之后只 append。
//! 覆写 system messages 时必须**两边同步**：
//! - request.input: 剥头部连续 System 段 → 塞新 system → 后面跟原本的非 System 部分
//! - state.accumulated: 同样的剥 + 塞操作（头部连续 System 段，后面 history 不动）
//!
//! 当前 opendan 没有在 accumulated 中段插 system message 的用法，规则成立。
//!
//! ## Session-level 状态结构（仅示意，本文件不实现）
//!
//! ```ignore
//! struct SessionRuntime {
//!     current_behavior: String,
//!
//!     // Switch 模式：所有 behavior 共享的全局 stream（沿用现有 .meta/state.snap）
//!     // 无需新字段，仍然走 self.load_latest_snapshot() / self.persist_snapshot(snap)
//!
//!     // Fork 模式：调用栈
//!     fork_stack: Vec<LLMContextSnapshot>, // 每帧 = 父 ctx 在 fork 点的 snap
//!     // 进程内即可，不持久化（fork 是一次性旅程；崩溃恢复时 sub 任务丢弃，
//!     // 父从 .meta/state.snap 重启即可。代价：fork 进行中崩溃会丢 sub 进度
//!     // ——可接受）
//!
//!     // Independent 模式：每个 behavior name 的独立持久化
//!     // 落盘在 .meta/behavior_<name>.snap，按需 lazy 装载
//! }
//! ```
//!
//! ## 旋钮（未决，先给倾向值）
//!
//! - **rounds_left**：
//!   - switch:      不重置（continue 全局预算）
//!   - fork:        重置为 new tool_policy.max_rounds（sub 独立预算）
//!   - independent: 重置（每个 behavior 自己的预算）
//! - **consecutive_errors**：
//!   - switch:      不清（防 LLM 靠切 behavior 绕错误上限）
//!   - fork:        清零（sub 是新生命）
//!   - independent: 清零（每个 behavior 独立计数）
//! - **trace**：sub-ctx 推荐 `<parent_trace>::fork-<n>` / `<sid>::beh-<name>-<n>`，
//!   caller 拼好塞 `RequestOverrides.trace`
//! - **pending_tool_calls 检查**：rebuild_with_inherit 要求
//!   `base_snap.state.pending_tool_calls.is_empty()`，否则返回
//!   `SnapshotCorrupted`，caller 决定先等续跑还是抛错
//!
//! ## 未决项 / 需要 waist 配合的地方
//!
//! 1. **fork 子 ctx 禁止 next_behavior** —— 这是 fork 语义的硬约束（保证控制
//!    流必须回父）。三种实现位置：
//!    - 在 `RequestOverrides.forbid_next_behavior` 上加 flag，waist 的
//!       behavior loop 看到该 flag 时硬忽略 LLM 给出的 next_behavior（最干净）
//!    - 把 BehaviorCfg.allow_next_behavior 渲染进 system prompt 让 LLM 自己遵守
//!       （软约束，LLM 可能违反）
//!    - 工具白名单不包含任何能触发切换的工具（如果切换是通过工具触发）+ XML
//!       parser 配 strict 拒绝裸 `<next_behavior>`
//!
//!    倾向方案 1。落地需要 waist 暴露这条 flag。
//!
//! 2. **fork sub-ctx output 回填父**：sub-ctx End 时产出 `ContextOutput::Text`
//!    / `behavior_result.text`。session-aware 工具的 `call()` 实现里同步
//!    `await session.fork_and_run(overrides, deps)` 拿到 sub.output，包成
//!    `Observation::Ok { ... }` 返回给 ToolManager —— 父 ctx 看到的就是
//!    一次普通 tool 调用的结果，waist 的 PendingTool / ToolResults 路径
//!    天然跑通，不需要新机制。
//!
//! 3. **independent behavior 间互相 switch 时的边界**：父切到子的瞬间，
//!    父的状态要落盘到 `.meta/behavior_<父name>.snap`（不是 `state.snap`），
//!    然后从 `.meta/behavior_<子name>.snap` 装子；子 End 时反向。这意味着
//!    `state.snap` 的角色需要重新定义：它到底是 switch 模式的"当前活动状态"，
//!    还是 independent 模式下的"当前活跃 behavior 的别名"？倾向后者，
//!    `state.snap` 不变 → 是当前活跃 behavior 的快照，独立文件
//!    `behavior_<name>.snap` 只存"非当前 behavior 的挂起态"。这样 switch 模式
//!    其实就是 independent 模式的退化（只有一个 behavior）。
//!
//! ## 实施顺序
//!
//! 1. ✅ **Phase 1**：本文件实现 `RequestOverrides` + `apply_overrides_to_snapshot`
//!    + `rebuild_with_inherit` + `build_fresh`。`forbid_next_behavior` 在
//!    overrides 上占位，waist 接收 flag 的逻辑作为单独 PR 后跟。
//! 2. ✅ **Phase 2 — switch 模式**：`agent_session::handle_outcome::Done` 接
//!    `final_snapshot = ctx.snapshot()`、`switch_behavior` 用
//!    `apply_overrides_to_snapshot` 重建 snapshot 并 persist。下一轮
//!    `build_or_resume` 在新 system prompt + 完整历史下续跑。
//! 3. ✅ **Phase 3 — independent 模式**：`SessionMeta` 加
//!    `process_entry` / `process_stack`，每个 process 一份
//!    `.meta/behavior_<entry>.snap`；切入子 process 时父快照存到独立文件、子
//!    state.snap 从存量恢复或 `LLMContextState::from_request` 新建；子 `END`
//!    pop 栈 → 装载父 snapshot 到 state.snap + 注入"[independent process
//!    `<X>` ended]"系统消息唤醒父；非顶层自然 Done 也保留 stream（不丢
//!    state.snap）。
//! 4. ✅ **Phase 4 — fork 模式**：`AgentSession::fork_and_run(overrides,
//!    sub_behavior_name) -> Result<ContextOutput>` 私有 async 原语 + 进程
//!    内 `fork_stack: Arc<Mutex<Vec<String>>>`（不持久化，每帧 = 父 trace
//!    id）；`AIAgent::get_session(id) -> Option<Arc<AgentSession>>` 公开
//!    访问器供 session-aware 工具拿 session 句柄；`TryCreateWorksessionTool`
//!    （`try_create_worksession`，在 `worksession_tools.rs`）在 `execute()`
//!    内调 `session.fork_and_run(...)`，子 ctx 跑到 Done 后把 `ContextOutput`
//!    打成 JSON 返回——父 ctx 视角是一次普通 tool 调用，无需新机制。
//!    子 ctx suspended outcome（WaitInput / PendingTool / ContextLimitReached）
//!    映射为错误（fork 无 resume 路径）。
//! 5. ⏳ **Phase 5（未来）**：把 `RequestOverrides` + `apply_overrides` +
//!    `forbid_next_behavior` flag 提案到 llm_context crate；helper 退化为
//!    opendan-specific 胶水（system messages 渲染 / deps 装配 / trace 命名）。
//!    `try_create_worksession` 的 sub-context 渲染补全（worksession list +
//!    parent recent history），让 sub-LLM 真正能做"复用还是新建 workspace"
//!    的判断。

use buckyos_api::{AiMessage, AiRole};
use llm_context::{
    context_loop::LLMContext,
    deps::LLMContextDeps,
    error::LLMComputeError,
    outcome::ResumeFill,
    request::{
        BudgetSpec, ErrorPolicy, HumanPolicy, LLMContextRequest, ModelPolicy, OutputSpec,
        ToolPolicy,
    },
    state::LLMContextSnapshot,
};

/// Overlay applied to a base [`LLMContextSnapshot`] before rebuilding the next
/// [`LLMContext`]. Every field is `Option`/`bool` so callers only specify what
/// they want to change; unset fields are inherited verbatim from the base.
///
/// All field semantics are documented in the file-level doc above.
#[derive(Debug, Clone, Default)]
pub struct RequestOverrides {
    pub system_messages: Option<Vec<AiMessage>>,
    pub tool_policy: Option<ToolPolicy>,
    pub objective: Option<String>,
    /// Outer Option = override or not; inner Option = override target value
    /// (`Some(Some(_))` set, `Some(None)` clear, `None` keep).
    pub trace: Option<Option<String>>,
    pub model_policy: Option<ModelPolicy>,
    pub budget: Option<BudgetSpec>,
    pub human_policy: Option<HumanPolicy>,
    pub error_policy: Option<ErrorPolicy>,
    pub output: Option<OutputSpec>,

    /// Reset `state.rounds_left` to the new `tool_policy.max_rounds`. Caller
    /// sets `true` for fork / independent, leaves `false` for switch (which
    /// continues the parent budget).
    pub reset_rounds: bool,
    /// Reset `state.consecutive_errors` to 0. Caller sets `true` for fork /
    /// independent — switch keeps the counter so a behavior swap cannot
    /// silently bypass the error cap.
    pub reset_errors: bool,

    /// Fork-only hard constraint. Currently informational — waist support
    /// will land in a follow-up PR. When set, downstream behavior loop should
    /// ignore any `<next_behavior>` produced by the LLM.
    pub forbid_next_behavior: bool,
}

/// Apply [`RequestOverrides`] to a snapshot, returning a new snapshot ready to
/// feed into [`LLMContext::resume`] with [`ResumeFill::ResumeFromMidRun`].
///
/// Maintains the invariant that `request.input` and `state.accumulated` share
/// the same leading System segment.
pub fn apply_overrides_to_snapshot(
    mut snap: LLMContextSnapshot,
    ov: RequestOverrides,
) -> LLMContextSnapshot {
    if let Some(new_system) = ov.system_messages {
        replace_leading_system(&mut snap.request.input, &new_system);
        replace_leading_system(&mut snap.state.accumulated, &new_system);
    }

    if let Some(tp) = ov.tool_policy {
        if ov.reset_rounds {
            snap.state.rounds_left = tp.max_rounds;
        }
        snap.request.tool_policy = tp;
    } else if ov.reset_rounds {
        // No new policy supplied but caller asked for reset — reset to the
        // existing policy's max_rounds.
        snap.state.rounds_left = snap.request.tool_policy.max_rounds;
    }

    if ov.reset_errors {
        snap.state.consecutive_errors = 0;
    }

    if let Some(obj) = ov.objective {
        snap.request.objective = obj;
    }
    if let Some(trace) = ov.trace {
        snap.request.trace = trace;
    }
    if let Some(mp) = ov.model_policy {
        snap.request.model_policy = mp;
    }
    if let Some(b) = ov.budget {
        snap.request.budget = b;
    }
    if let Some(hp) = ov.human_policy {
        snap.request.human_policy = hp;
    }
    if let Some(ep) = ov.error_policy {
        snap.request.error_policy = ep;
    }
    if let Some(o) = ov.output {
        snap.request.output = o;
    }

    // TODO(waist-followup): plumb forbid_next_behavior into a snapshot/state
    // field once llm_context exposes one. For now the flag is dropped after
    // logging — fork callers should not rely on it as a hard guarantee yet.

    snap
}

/// Replace the leading run of `System`-role messages in `msgs` with `new_system`.
/// Non-System messages following the leading block are left untouched.
fn replace_leading_system(msgs: &mut Vec<AiMessage>, new_system: &[AiMessage]) {
    let leading = msgs
        .iter()
        .position(|m| m.role != AiRole::System)
        .unwrap_or(msgs.len());
    let tail = msgs.split_off(leading);
    msgs.clear();
    msgs.extend(new_system.iter().cloned());
    msgs.extend(tail);
}

/// Build the next [`LLMContext`] from a base snapshot, inheriting `state` (and
/// therefore `accumulated` / `steps` / `usage`) while applying `overrides` to
/// the request side. Used by **switch** (caller writes snapshot back to the
/// session) and **fork** (caller throws sub snapshot away after sub run ends).
///
/// Returns `Err(SnapshotCorrupted)` if the base snapshot is in a suspended
/// state (has `pending_tool_calls`) — caller must resume that one first via
/// the normal [`LLMContext::resume`] flow before inheriting from it.
pub fn rebuild_with_inherit(
    base_snap: LLMContextSnapshot,
    overrides: RequestOverrides,
    deps: LLMContextDeps,
) -> Result<LLMContext, LLMComputeError> {
    if !base_snap.state.pending_tool_calls.is_empty() {
        return Err(LLMComputeError::SnapshotCorrupted(
            "rebuild_with_inherit: base snapshot has pending tool calls — \
             resume the pending tools before forking/switching"
                .to_string(),
        ));
    }
    let rebuilt = apply_overrides_to_snapshot(base_snap, overrides);
    LLMContext::resume(rebuilt, ResumeFill::ResumeFromMidRun, deps)
}

/// Build a fresh [`LLMContext`] with no inherited state. Thin wrapper over
/// [`LLMContext::new`] — exists so all opendan-side ctx construction paths go
/// through this module and gain uniform logging / trace handling later.
pub fn build_fresh(request: LLMContextRequest, deps: LLMContextDeps) -> LLMContext {
    LLMContext::new(request, deps)
}

#[cfg(test)]
mod tests {
    use super::*;
    use buckyos_api::AiContent;
    use llm_context::request::{ContextOwnerRef, ToolMode};
    use llm_context::state::LLMContextState;

    fn msg(role: AiRole, text: &str) -> AiMessage {
        AiMessage {
            role,
            content: vec![AiContent::Text {
                text: text.to_string(),
            }],
        }
    }

    fn snap_with(input: Vec<AiMessage>, accumulated: Vec<AiMessage>) -> LLMContextSnapshot {
        let request = LLMContextRequest {
            owner: ContextOwnerRef::Agent {
                session_id: "s".into(),
            },
            trace: None,
            objective: String::new(),
            input,
            model_policy: ModelPolicy::default(),
            tool_policy: ToolPolicy::default(),
            output: OutputSpec::default(),
            budget: BudgetSpec::default(),
            human_policy: HumanPolicy::default(),
            error_policy: ErrorPolicy::default(),
        };
        let mut state = LLMContextState::from_request(&request, 0);
        state.accumulated = accumulated;
        state.rounds_left = request.tool_policy.max_rounds;
        LLMContextSnapshot { request, state }
    }

    #[test]
    fn replace_leading_system_replaces_block_and_keeps_tail() {
        let mut msgs = vec![
            msg(AiRole::System, "old-1"),
            msg(AiRole::System, "old-2"),
            msg(AiRole::User, "u-1"),
            msg(AiRole::Assistant, "a-1"),
            msg(AiRole::User, "u-2"),
        ];
        let new_sys = vec![msg(AiRole::System, "new")];
        replace_leading_system(&mut msgs, &new_sys);
        assert_eq!(msgs.len(), 4);
        assert_eq!(msgs[0].role, AiRole::System);
        assert_eq!(msgs[0].text_content(), "new");
        assert_eq!(msgs[1].role, AiRole::User);
        assert_eq!(msgs[1].text_content(), "u-1");
        assert_eq!(msgs[3].text_content(), "u-2");
    }

    #[test]
    fn replace_leading_system_with_no_existing_system_prepends() {
        let mut msgs = vec![msg(AiRole::User, "u")];
        let new_sys = vec![msg(AiRole::System, "s")];
        replace_leading_system(&mut msgs, &new_sys);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, AiRole::System);
        assert_eq!(msgs[1].role, AiRole::User);
    }

    #[test]
    fn replace_leading_system_with_empty_new_strips_existing() {
        let mut msgs = vec![
            msg(AiRole::System, "old"),
            msg(AiRole::User, "u"),
        ];
        replace_leading_system(&mut msgs, &[]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, AiRole::User);
    }

    #[test]
    fn apply_overrides_syncs_system_in_request_and_accumulated() {
        let snap = snap_with(
            vec![msg(AiRole::System, "sys-old"), msg(AiRole::User, "u")],
            vec![
                msg(AiRole::System, "sys-old"),
                msg(AiRole::User, "u"),
                msg(AiRole::Assistant, "a-runtime"),
            ],
        );
        let ov = RequestOverrides {
            system_messages: Some(vec![msg(AiRole::System, "sys-new")]),
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);

        assert_eq!(out.request.input[0].text_content(), "sys-new");
        assert_eq!(out.request.input[1].text_content(), "u");
        assert_eq!(out.state.accumulated[0].text_content(), "sys-new");
        assert_eq!(out.state.accumulated[1].text_content(), "u");
        // Accumulated tail (assistant from earlier rounds) survives.
        assert_eq!(out.state.accumulated[2].text_content(), "a-runtime");
    }

    #[test]
    fn apply_overrides_reset_rounds_uses_new_policy() {
        let snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        let mut tp = ToolPolicy::default();
        tp.mode = ToolMode::Whitelist;
        tp.max_rounds = 99;
        let ov = RequestOverrides {
            tool_policy: Some(tp),
            reset_rounds: true,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert_eq!(out.state.rounds_left, 99);
        assert_eq!(out.request.tool_policy.max_rounds, 99);
    }

    #[test]
    fn apply_overrides_no_reset_keeps_rounds_left() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        snap.state.rounds_left = 3; // mid-run state
        let mut tp = ToolPolicy::default();
        tp.max_rounds = 99;
        let ov = RequestOverrides {
            tool_policy: Some(tp),
            reset_rounds: false,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert_eq!(out.state.rounds_left, 3, "switch must not reset budget");
        assert_eq!(out.request.tool_policy.max_rounds, 99);
    }

    #[test]
    fn apply_overrides_reset_errors_clears_counter() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        snap.state.consecutive_errors = 2;
        let ov = RequestOverrides {
            reset_errors: true,
            ..Default::default()
        };
        let out = apply_overrides_to_snapshot(snap, ov);
        assert_eq!(out.state.consecutive_errors, 0);
    }

    #[test]
    fn apply_overrides_trace_can_set_or_clear() {
        let mut snap = snap_with(vec![msg(AiRole::User, "u")], vec![msg(AiRole::User, "u")]);
        snap.request.trace = Some("parent::1".into());

        let out = apply_overrides_to_snapshot(
            snap.clone(),
            RequestOverrides {
                trace: Some(Some("parent::1::fork-0".into())),
                ..Default::default()
            },
        );
        assert_eq!(out.request.trace.as_deref(), Some("parent::1::fork-0"));

        let out_cleared = apply_overrides_to_snapshot(
            snap.clone(),
            RequestOverrides {
                trace: Some(None),
                ..Default::default()
            },
        );
        assert!(out_cleared.request.trace.is_none());

        let out_unchanged = apply_overrides_to_snapshot(
            snap,
            RequestOverrides {
                trace: None,
                ..Default::default()
            },
        );
        assert_eq!(out_unchanged.request.trace.as_deref(), Some("parent::1"));
    }
}

