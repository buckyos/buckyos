//! `LocalLLMContext` — 生产级 L4 OneShot runtime,绑定一个本地目录。
//!
//! 给一个目录 + 一个 [`OneShotRequest`],就能一路跑到终态;中间崩溃后
//! 用同一个目录再启动,自动 resume 最近未完成的 run。
//!
//! 这是 `doc/opendan/LLM Context 设计.md` §0.1 中提到的 `LLMOneShotContext`
//! 的生产实现,负责把"本地目录 + 自然语言 objective + 一段输入"lower 成
//! waist 的 `LLMContextRequest + LLMContextDeps`,并提供围绕 waist
//! [`LLMContext::run`] 的标准 outcome loop + crash recovery。
//!
//! ## 顶层使用形态
//!
//! ```text
//!   // 全新 run
//!   let ctx = LocalLLMContext::new_run(dir, req, llm)?;
//!   let outcome = ctx.drive_to_terminal().await?;
//!
//!   // 崩溃后同一个 dir 再启动
//!   let ctx = LocalLLMContext::resume_or_new(dir, req, llm)?;
//!   //   - dir 里有 status=running 的 run → resume
//!   //   - 否则按 req 开新 run
//!   let outcome = ctx.drive_to_terminal().await?;
//! ```
//!
//! ## 核心定位:L4 builder + outcome driver,**不是另一种 waist runtime**
//!
//! 设计文档 §0.1 / §2.2 反复强调:L4 `LLM*Context` 的全部职责是
//! **lowering** —— 把"DSL / 配置文件用户面对的东西"降级成 waist 的
//! `LLMContextRequest + LLMContextDeps`,**然后由 waist 自己的
//! `LLMContext::run` 跑**。L4 不能也不该自己提供一个并行的 `run`:
//! 那会立刻破坏双中立性。
//!
//! 那 [`LocalLLMContext::drive_to_terminal`] 算什么?它是一个**严格围绕
//! `LLMContext::run` 的 outcome 分发器**,职责被刻意限制为:
//!
//! 1. 每次调用 `ctx.run().await` 得到一个 [`LLMContextOutcome`];
//! 2. 落盘 snapshot(crash recovery);
//! 3. 根据 outcome 决定:
//!    - 终态(`Done` / `Error` / `BudgetExhausted`)→ 退出,把结果返还 caller;
//!    - `ContextLimitReached` → 调外部压缩 → `ResumeFill::RewrittenHistory`
//!      喂回 → `LLMContext::resume` → 继续 loop;
//!    - `PendingTool` → 把控制权交还 caller(OneShot 自己没办法消化
//!      异步任务回调,见 §6.3 / §6.4)。
//!
//! 它不解读输出、不切换行为状态机、不维护长期记忆——所有"业务"都靠
//! 输入侧的 request 与输出侧的 outcome 表达。**任何想往 driver 里塞
//! "看到 next_behavior 字段就切状态" / "把 tool_calls 解出来重路由"
//! 的提议,都属于 `LLMAgentContext` 而不是 OneShot**。
//!
//! ## 目录布局(crash-recovery 持久化)
//!
//! 同一个 `dir` 可以承载**多次**完整的 OneShot run,每次一个子目录:
//!
//! ```text
//!   <dir>/
//!   ├── runs/
//!   │   ├── 20260510-103045-a7f3/        ← run_id(时间戳 + 短随机)
//!   │   │   ├── state.json                ← RunMetaState:current_status / request_hash / ...
//!   │   │   ├── request.json              ← 原始 OneShotRequest(用于审计 + 校验 resume 兼容性)
//!   │   │   ├── snapshots/
//!   │   │   │   ├── 0001.snap.json
//!   │   │   │   ├── 0002.snap.json
//!   │   │   │   └── ...
//!   │   │   ├── outcomes/
//!   │   │   │   └── final.json            ← 终态 outcome 的归档(完成后写入)
//!   │   │   └── worklog.jsonl             ← append-only 事件流(可选,见下文)
//!   │   └── 20260510-115422-b9e1/
//!   │       └── ...
//!   ├── workspace/                        ← 工具 root(read/write/edit + exec_bash cwd)
//!   ├── bin/                              ← exec_bash PATH overlay(用户脚本投放点,chmod +x 后盖过系统 PATH)
//!   └── .lock                             ← 进程级 flock,防止两个 OneShot 抢同一个 dir
//! ```
//!
//! Snapshot 命名是 zero-padded 单调递增序号(`0001.snap.json` → `0002...`),
//! resume 时取**最大序号**的那一份。完成的 run 在 `state.json` 里被标记成
//! `Completed`,下次启动时不会被自动 resume。
//!
//! ## Crash recovery 粒度
//!
//! **目标:每轮 LLM 推理前落盘**(本次重启不会重复扣费已经付过的推理)。
//!
//! 当前 waist 接口只在 outcome 边界回到调用方,没有提供"轮间 hook";
//! 也就是说 `LLMContext::run` 内部跑了多轮 tool loop 后才回到我们手里
//! 的话,中间的 LLM 推理是无法在 OneShot 这一层"轮前"落盘的。为此本
//! 实现按下面的纪律工作:
//!
//! 1. **每个 outcome 边界都落盘**——run 启动前一次 + 每次 `ctx.run()`
//!    返回后一次 + 每次 `ctx.resume()` 前一次。这是 OneShot 自己能保证的
//!    最细粒度。
//! 2. **保留 worklog event sink 的扩展点**:当 waist 在 `LLMContextDeps`
//!    上加入"每轮 LLM 推理前的 hook"(参见模块底部 `TODO: turn-level hook`)
//!    后,本实现把 snapshot 落盘逻辑从 sink 接进来,真正做到"轮前落盘"。
//!    届时 API 不需要变。
//!
//! 在 `LLMContext::run` 内部多轮 tool loop 的场景下,**如果你的 ToolManager
//! 是确定性的(同 args → 同 result),崩溃重启会重复执行一些工具调用**。
//! 这条权衡是 §A.4 "ToolManager 内部 retry / 副作用幂等性是 effect 实现层
//! 私事"的具体体现——OneShot 不在 waist 上面再造一层,只在自己能控制
//! 的边界上落盘。
//!
//! ## 与 §A.1 / §A.3 / §A.4 Non-Goal 的对齐
//!
//! 本模块**严格不引入**任何新的 waist 字段。所有 OneShot 特有的东西
//! (工作目录、snapshot 路径策略、工具 sandbox、压缩函数、resume 策略)
//! 都封在本类型内部,对 waist 不可见:
//!
//! - **目录 + workspace** → `OneShotState` + `LocalDirToolManager`,§A.3
//!   "执行环境绑定不进 waist"。
//! - **snapshot 存储介质** → `FileSnapshotStore`,§A.4 "snapshot 存储介质是
//!   `SnapshotStore` 接口实现细节"。
//! - **压缩函数** → `Compressor` trait + caller 注入的具体实现,§A.4
//!   "上下文压缩策略不进 waist"。
//! - **bash / fs 等具体工具** → ToolManager 内部实现,§A.3 "特定 tool
//!   不直接抬到 waist"。
//! - **错误归一化 wire format** → 由 ToolManager / provider adapter 自定,
//!   §A.4 "错误归一化 wire format 不进 waist"。

use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use buckyos_api::{AiMessage, AiToolCall};
use fs2::FileExt;
use serde::{Deserialize, Serialize};

use llm_context::deps::{
    LLMContextDeps, LlmClient, NoopWorklogSink, ToolManager, ToolSpecLite, TurnHook, WorklogSink,
};
use llm_context::observation::Observation;
use llm_context::outcome::{LLMContextOutcome, ResumeFill};
use llm_context::request::{
    BudgetSpec, ContextOwnerRef, ContextThreshold, ErrorPolicy, LLMContextRequest, ModelPolicy,
    OutputSpec, ToolPolicy,
};
use llm_context::state::LLMContextSnapshot;
use llm_context::LLMContext;

use crate::llm_compress::LlmSummarizeCompressor;
use crate::{
    AgentToolManager, AgentToolResult, AgentToolStatus, BinOverlayConfig, EditFileTool,
    ExecBashTool, FileToolConfig, LlmBashConfig, NoopFileWriteAudit, SessionRuntimeContext,
    WriteFileTool,
};

// =========================================================================
// 公共配置常量
// =========================================================================

/// 默认 context 压缩阈值(token window 的 75%)。
pub const DEFAULT_CONTEXT_YIELD_RATIO: f32 = 0.75;

/// 默认连续错误上限。可恢复错误会被回灌成 system observation 让 LLM
/// 自我修复,连续超过这个上限才升格成终态 Error 退出。
pub const DEFAULT_MAX_CONSECUTIVE_ERRORS: u32 = 3;

/// `drive_to_terminal_auto` 在调用方没显式给 token 预算时使用的兜底。
/// 选 32K 是经验数:既小到不会让 summarize 出来还是塞不下 provider window,
/// 又大到能给 LLM 留出连贯的 system + 最近若干轮上下文。如果 caller 在 budget
/// 里给了更精确的信号(max_total_tokens / AbsoluteTokens),会优先用那些。
pub const DEFAULT_AUTO_COMPRESS_TARGET_TOKENS: u32 = 32_768;

// =========================================================================
// 用户面接口:OneShotRequest
// =========================================================================

/// **OneShot 的用户面 request**——不直接暴露 waist 类型,但允许覆盖关键策略。
///
/// 设计纪律:
/// - **必选字段最少**:`objective` + `input`。其它一律 `Option`,由 lowering
///   阶段填默认值。这样调用方在最简单场景下只写一行就行。
/// - **覆盖项以"原值"传入**,不是 builder 链。OneShotRequest 自身是可序列化的
///   (`Serialize` + `Deserialize`),因为它要被持久化到 `<run>/request.json`
///   用于 audit + resume 时的兼容性校验。Builder 链对 serde 不友好。
/// - **不允许覆盖 `owner` / `trace`**:那些是 OneShot 自己生成的(基于 run_id),
///   暴露出去会让调用方误以为可以跨 run 复用 trace id。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OneShotRequest {
    /// 自然语言目标。会被原文写入 `LLMContextRequest.objective`,供 worklog
    /// 阅读,**不进 prompt**。要给 LLM 看的目标请放进 `input` 的 system /
    /// user message。
    pub objective: String,

    /// 已编译好的对话历史。L4 的 prompt 编译职责在 OneShot 这里被刻意简化:
    /// 调用方自己把 system prompt / user instruction / 早期 message 全部
    /// 准备好。OneShot 不提供模板能力(那是更上层的事)。
    pub input: Vec<AiMessage>,

    /// 模型策略覆盖(`None` → `ModelPolicy::default()`)。
    pub model_policy: Option<ModelPolicy>,

    /// 工具策略覆盖(`None` → `ToolPolicy::default()`)。
    pub tool_policy: Option<ToolPolicy>,

    /// 输出契约(`None` → `OutputSpec::Text`)。
    pub output: Option<OutputSpec>,

    /// 预算覆盖(`None` → 启用默认 75% context 阈值的 `BudgetSpec`)。
    /// 注意:即便调用方传入 `Some(custom_budget)`,如果 custom_budget 的
    /// `context_yield_threshold` 是 `None`,OneShot 会**自动注入默认阈值**
    /// (这是 OneShot 的硬约束之一:无穷增长的对话不是合理的生产行为)。
    pub budget: Option<BudgetSpec>,

    /// Human-in-the-loop 策略覆盖(`None` → `HumanPolicy::default()`)。
    pub human_policy: Option<llm_context::request::HumanPolicy>,

    /// 错误策略覆盖(`None` → `Suspend` mode + 默认连续错误上限)。
    pub error_policy: Option<ErrorPolicy>,
}

impl OneShotRequest {
    /// 最简构造:只给 objective + input。其它字段全 `None`,由 lowering
    /// 阶段填 OneShot 默认。
    pub fn new(objective: impl Into<String>, input: Vec<AiMessage>) -> Self {
        Self {
            objective: objective.into(),
            input,
            model_policy: None,
            tool_policy: None,
            output: None,
            budget: None,
            human_policy: None,
            error_policy: None,
        }
    }

    /// 计算请求的"语义哈希",用于 resume 时校验兼容性。
    ///
    /// 用途:如果 `<dir>/runs/<run_id>/request.json` 反序列化后的 hash 与
    /// 当前传入的 request 不匹配,说明调用方拿同一个 dir 跑了语义不同的
    /// 第二个任务——`resume_or_new` 会**拒绝 auto-resume**,要求 caller
    /// 显式选择(新 run / 强制 resume / 改 dir)。
    ///
    /// 这是 #2 决定"自动 resume"安全性的关键保险。
    pub fn semantic_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut h = DefaultHasher::new();
        // 注意:只 hash 真正影响"这是同一个任务吗"的字段。
        // budget / human_policy / error_policy 的微调不应否决 resume。
        self.objective.hash(&mut h);
        // AiMessage 的 hash:序列化后字节流哈希,避免 trait 边界问题。
        if let Ok(json) = serde_json::to_vec(&self.input) {
            json.hash(&mut h);
        }
        h.finish()
    }

    /// **核心 lowering**:把 OneShotRequest + run-level 标识 → 标准
    /// `LLMContextRequest`。
    ///
    /// 这里把 OneShot 的所有默认值一次性灌进去:75% context 阈值、
    /// Suspend 错误模式、owner = OneShot { id: run_id } 等。
    pub fn lower_to_waist(&self, run_id: &str) -> LLMContextRequest {
        let mut budget = self.budget.clone().unwrap_or_default();
        // 硬约束:OneShot 永远启用 context 阈值,即便 caller 传了 budget。
        if budget.context_yield_threshold.is_none() {
            budget.context_yield_threshold = Some(ContextThreshold::Ratio {
                value: DEFAULT_CONTEXT_YIELD_RATIO,
            });
        }

        LLMContextRequest {
            owner: ContextOwnerRef::OneShot {
                id: run_id.to_string(),
            },
            trace: Some(run_id.to_string()),
            objective: self.objective.clone(),
            behavior_name: String::new(),
            input: self.input.clone(),
            model_policy: self.model_policy.clone().unwrap_or_default(),
            tool_policy: self.tool_policy.clone().unwrap_or_default(),
            output: self.output.clone().unwrap_or(OutputSpec::Text),
            budget,
            human_policy: self.human_policy.clone().unwrap_or_default(),
            error_policy: self.error_policy.clone().unwrap_or(ErrorPolicy {
                max_consecutive_errors: DEFAULT_MAX_CONSECUTIVE_ERRORS,
            }),
            forbid_next_behavior: false,
        }
    }
}

// =========================================================================
// Run 级状态:持久化到 <run>/state.json
// =========================================================================

/// 一个 run 的元数据,会被 OneShot 在每个 outcome 边界刷盘到
/// `<run>/state.json`。这是 crash recovery 的"目录索引"。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunMetaState {
    pub run_id: String,
    pub created_at_unix_ms: u64,
    pub last_updated_unix_ms: u64,
    pub request_semantic_hash: u64,
    pub status: RunStatus,
    /// 当前最新的 snapshot 序号。`None` 表示还没产出过 snapshot(run 刚启动)。
    /// resume 时按这个值定位 `snapshots/{:04}.snap.json`。
    pub latest_snapshot_idx: Option<u32>,
    /// 最近一次挂起态(如果有)——用于 resume 时知道 `ResumeFill` 该填什么形态。
    /// `None` 表示当前不在挂起态(刚启动 / 已完成 / 已 fail)。
    pub last_suspend_kind: Option<SuspendKind>,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum RunStatus {
    /// 进程持有 `.lock` 并正在跑。崩溃时这个状态留在盘上没有被改写,
    /// 是 `resume_or_new` 识别"上一次崩溃了"的唯一信号。
    Running,
    /// LLMContext 进入挂起态。OneShot 自己消化不掉,等 caller 显式 resume。
    Suspended,
    /// 终态:Done / Error / BudgetExhausted 都归到这里(具体由 `outcomes/final.json` 区分)。
    Completed,
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum SuspendKind {
    PendingTool,
    /// 注意:`ContextLimitReached` 在 OneShot 内被 `drive_to_terminal`
    /// 透明消化(走 Compressor → resume),不会把 run 标记成 Suspended。
    /// 如果 caller 用更底层的 `step()` 接口手动驱动,才会看到这个值。
    ContextLimitReached,
    /// §3.13 inference interrupt。snapshot 是本轮 inference 发起前的状态;
    /// resume 走 `ResumeFromMidRun`,与"运行中崩溃恢复"共享语义。
    Interrupted,
}

// =========================================================================
// 顶层类型:LocalLLMContext
// =========================================================================

/// L4 OneShot runtime。生命周期 = "一次围绕 `dir` 的完整 LLM 任务执行"。
///
/// 调用方拿到这个类型之后两条路:
/// 1. [`Self::drive_to_terminal`] — 一路跑到终态或挂起态(95% 的生产场景)。
/// 2. [`Self::step`] — 手动驱动一个 outcome,适合需要精细 outcome 分发的
///    caller(例:把 OneShot 嵌进 workflow 的 leaf node)。
///
/// 不持有任何 session / 行为机 / 容器句柄——OneShot 是无状态的(对外而言),
/// 它的"状态"全部在 `dir` 的文件系统里。
pub struct LocalLLMContext {
    /// 根工作目录。
    dir: PathBuf,

    /// 当前 run 的 id(`<dir>/runs/<run_id>/`)。
    run_id: String,

    /// run 级元数据,镜像 `<run>/state.json`。
    meta: RunMetaState,

    /// 当前正在跑的 waist 进程上下文。
    ///
    /// `Option`:刚启动时(下一行就要 `ctx.run()`)是 `Some`;进入挂起态
    /// 后被 `take()` 出去序列化,变 `None`;`drive_to_terminal` 决定 resume
    /// 时再 `LLMContext::resume(...)` 装回来。这样能在挂起态防止误调
    /// `step()` —— 类型系统帮我们卡一道。
    ctx: Option<LLMContext>,

    /// Snapshot 持久化抽象。生产实现是 `FileSnapshotStore`,测试可注入 in-memory。
    snapshot_store: Arc<dyn SnapshotStore>,

    /// 持有原始 request,resume 时用得到(校验 semantic_hash + 重新 lower)。
    request: OneShotRequest,

    /// LLM client — 保留在 L4 自己手上,resume 时直接复用,不再从 deps 反掏。
    llm: Arc<dyn LlmClient>,
}

impl LocalLLMContext {
    /// **开新 run**。如果 `dir` 下已经有 `status=Running` 的 run,会**报错**
    /// (要求 caller 用 [`Self::resume_or_new`] 或显式删除旧 run)。
    ///
    /// 这条约束保护"自动 resume"的安全性:`new_run` 永远不会静默丢掉
    /// 上一次未完成的 run。
    pub fn new_run(
        dir: impl Into<PathBuf>,
        request: OneShotRequest,
        llm: Arc<dyn LlmClient>,
    ) -> Result<Self, LocalLLMContextError> {
        let dir = dir.into();
        Self::ensure_dir_layout(&dir)?;
        Self::acquire_dir_lock(&dir)?;

        if let Some(existing) = Self::find_running_run(&dir)? {
            return Err(LocalLLMContextError::RunningRunExists {
                run_id: existing.run_id,
                hint: "use `resume_or_new` to auto-resume, or delete the run dir to start fresh"
                    .into(),
            });
        }

        let run_id = generate_run_id();
        let now = now_unix_ms();
        let meta = RunMetaState {
            run_id: run_id.clone(),
            created_at_unix_ms: now,
            last_updated_unix_ms: now,
            request_semantic_hash: request.semantic_hash(),
            status: RunStatus::Running,
            latest_snapshot_idx: None,
            last_suspend_kind: None,
        };

        let snapshot_store: Arc<dyn SnapshotStore> =
            Arc::new(FileSnapshotStore::new(dir.join("runs").join(&run_id)));

        // 写 request.json + state.json 初稿
        write_run_request(&dir, &run_id, &request)?;
        write_run_state(&dir, &meta)?;

        // 构造 deps + waist LLMContext。注入 TurnHook 让 waist 在每轮 LLM
        // 推理前把当前 snapshot 写盘——这是"crash recovery 不重复扣费"的关键
        // 落点(§3.12 / §6.6)。
        let deps = build_deps(&dir, &run_id, llm.clone(), snapshot_store.clone())?;
        let waist_req = request.lower_to_waist(&run_id);
        let ctx = LLMContext::new(waist_req, deps);

        // **启动前快照** —— crash recovery 粒度纪律的第一个落盘点。
        let s = ctx.snapshot();
        let idx = snapshot_store.put_next(&s)?;
        let mut meta = meta;
        meta.latest_snapshot_idx = Some(idx);
        meta.last_updated_unix_ms = now_unix_ms();
        write_run_state(&dir, &meta)?;

        Ok(Self {
            dir,
            run_id,
            meta,
            ctx: Some(ctx),
            snapshot_store,
            request,
            llm,
        })
    }

    /// **自动 resume 或开新 run**。
    ///
    /// 行为(按优先级):
    /// 1. `dir` 下存在 `status=Running` 的 run **且** 它的 `request_semantic_hash`
    ///    与传入 request 匹配 → resume 那个 run,忽略传入的 request 内容
    ///    (使用 run 目录里持久化的 `request.json`)。
    /// 2. `dir` 下存在 `status=Running` 的 run 但 hash **不**匹配 →
    ///    返回 `Err(SemanticHashMismatch)`,**不**静默接管。caller 需要显式
    ///    决定:换 dir / 删除旧 run / 接受新 request(后者需要专门的 API,
    ///    本版不提供——保守起见)。
    /// 3. `dir` 下没有 `Running` run → 等价于 `new_run`。
    ///
    /// 这套策略是 #2 决策"自动 resume"的安全网:auto-resume 只在"完全是
    /// 同一个任务"时发生,任何不一致都要求 caller 介入。
    pub fn resume_or_new(
        dir: impl Into<PathBuf>,
        request: OneShotRequest,
        llm: Arc<dyn LlmClient>,
    ) -> Result<Self, LocalLLMContextError> {
        let dir = dir.into();
        Self::ensure_dir_layout(&dir)?;
        Self::acquire_dir_lock(&dir)?;

        match Self::find_running_run(&dir)? {
            None => {
                // 锁已经在上面拿了,但 new_run 内部也会再拿一次。
                // 这里释放再调 new_run 会有 TOCTOU 窗口。
                // 解决:把 new_run 拆成 lock-free 的内部函数,或者让 acquire_dir_lock
                // 幂等(同进程重入返回 Ok)。当前实现采用后者,见 acquire_dir_lock 注释。
                drop_lock_if_held(&dir);
                Self::new_run(dir, request, llm)
            }
            Some(existing) => {
                if existing.request_semantic_hash != request.semantic_hash() {
                    return Err(LocalLLMContextError::SemanticHashMismatch {
                        run_id: existing.run_id,
                        on_disk_hash: existing.request_semantic_hash,
                        provided_hash: request.semantic_hash(),
                    });
                }
                Self::do_resume(dir, existing, llm)
            }
        }
    }

    /// 内部:真正的 resume 流程。从 `state.json` 读最新 snapshot,装回
    /// `LLMContext`。**不接受**传入的 request——以盘上的 `request.json` 为准。
    fn do_resume(
        dir: PathBuf,
        meta: RunMetaState,
        llm: Arc<dyn LlmClient>,
    ) -> Result<Self, LocalLLMContextError> {
        let run_id = meta.run_id.clone();
        let request = read_run_request(&dir, &run_id)?;

        let snapshot_store: Arc<dyn SnapshotStore> =
            Arc::new(FileSnapshotStore::new(dir.join("runs").join(&run_id)));

        // 取最新 snapshot。
        let snapshot = match meta.latest_snapshot_idx {
            Some(idx) => snapshot_store.get(idx)?,
            None => {
                return Err(LocalLLMContextError::CorruptedRun {
                    run_id: run_id.clone(),
                    reason: "running run has no snapshot on disk".into(),
                })
            }
        };

        // **关键决策**:崩溃后的 resume 形态分流:
        //
        // 1. `PendingTool` 挂起期间崩溃 —— OneShot 自己没办法提供
        //    ToolResults,需要 caller 走更底层的 fill API(本版不暴露,
        //    保守报错让 caller 介入)。
        // 2. `ContextLimitReached` 挂起期间崩溃 —— `drive_to_terminal` 本应
        //    立刻调 compressor 消化掉,crash 时压缩 in-flight 的状态没法保证,
        //    同样保守要求 caller 介入。
        // 3. "运行中"崩溃(`status=Running` 且 `last_suspend_kind=None`)——
        //    走 §3.1 / §6.6 的 `ResumeFromMidRun` 路径,直接复用 waist 公共
        //    接口。
        if let Some(kind) = meta.last_suspend_kind {
            return Err(LocalLLMContextError::CrashedInSuspended {
                run_id: run_id.clone(),
                kind,
                hint:
                    "crashed while suspended; caller must use a lower-level resume API providing ResumeFill"
                        .into(),
            });
        }

        let deps = build_deps(&dir, &run_id, llm.clone(), snapshot_store.clone())?;
        let ctx =
            LLMContext::resume(snapshot, ResumeFill::ResumeFromMidRun, deps).map_err(|e| {
                LocalLLMContextError::CorruptedRun {
                    run_id: run_id.clone(),
                    reason: format!("ResumeFromMidRun failed: {e}"),
                }
            })?;

        Ok(Self {
            dir,
            run_id,
            meta,
            ctx: Some(ctx),
            snapshot_store,
            request,
            llm,
        })
    }

    /// **核心 driver**:一路跑到终态或外部挂起态。
    ///
    /// 行为:
    /// - `Done` / `Error` / `BudgetExhausted` → 写 `outcomes/final.json`,
    ///   把 status 改成 `Completed`,返回 outcome。
    /// - `ContextLimitReached` → 调 `compressor`(caller 注入),用
    ///   `ResumeFill::RewrittenHistory` 喂回,继续 loop。
    /// - `PendingTool` → 写 `state.json`(status=Suspended +
    ///   last_suspend_kind),返回 outcome,把控制权交还 caller。
    ///
    /// 这是 §6.4 处理范式的标准落点,**不解读输出字段**,完全按 waist
    /// outcome 类型分发。
    pub async fn drive_to_terminal(
        &mut self,
        compressor: &dyn Compressor,
    ) -> Result<LLMContextOutcome, LocalLLMContextError> {
        loop {
            let outcome = self.step().await?;

            match outcome {
                LLMContextOutcome::ContextLimitReached {
                    accumulated,
                    snapshot,
                    ..
                } => {
                    // 压缩 → resume。落盘发生在 step() 内部,不需要再写一次。
                    let rewritten = compressor.compress(accumulated, &self.dir).await?;
                    self.resume_with_rewritten_history(snapshot, rewritten)?;
                    continue;
                }
                terminal_or_suspend => {
                    // 终态 + PendingTool 都直接交还 caller。
                    // 终态写 final.json 由 step() 内部已经做了(see step's tail)。
                    return Ok(terminal_or_suspend);
                }
            }
        }
    }

    /// 与 [`Self::drive_to_terminal`] 等价,但**自动用** [`LlmSummarizeCompressor`]
    /// 作为压缩策略 —— caller 不用自己挑 compressor。
    ///
    /// 默认 compressor 的来源:
    /// - **LLM client** = `self.llm`(同一个 client,retry / quota / 路由都复用)。
    /// - **summarize 模型** = `request.model_policy.preferred`。如果该字段为空,
    ///   返回错误(我们不假装猜一个模型 alias —— 那会让排错变成"为什么调用
    ///   了一个我从没配过的模型")。
    /// - **target_token_budget** = `auto_compress_target_tokens(&self.request)`(优先
    ///   读 `budget.max_total_tokens` / `context_yield_threshold = AbsoluteTokens`,
    ///   都没有则用 `DEFAULT_AUTO_COMPRESS_TARGET_TOKENS`)。
    ///
    /// 需要更精细控制(改 keep-recent 长度、换不同的副本模型 summarize、走
    /// 非 LLM 的截断策略)时,显式构造一个 [`Compressor`] 实现传给
    /// [`Self::drive_to_terminal`]。
    pub async fn drive_to_terminal_auto(
        &mut self,
    ) -> Result<LLMContextOutcome, LocalLLMContextError> {
        let compressor = self.default_compressor()?;
        self.drive_to_terminal(&compressor).await
    }

    /// 构造默认 [`LlmSummarizeCompressor`]。暴露这个方法是给那些"想自己拿来
    /// 跑别的循环,但又懒得手写 deps"的高级 caller(例:测试、外部 driver)。
    pub fn default_compressor(&self) -> Result<LlmSummarizeCompressor, LocalLLMContextError> {
        let model_alias = self
            .request
            .model_policy
            .as_ref()
            .map(|p| p.preferred.clone())
            .unwrap_or_default();
        if model_alias.trim().is_empty() {
            return Err(LocalLLMContextError::ToolWiringFailed(
                "cannot build default compressor: request.model_policy.preferred is empty"
                    .to_string(),
            ));
        }
        let target = auto_compress_target_tokens(&self.request);
        let deps = build_deps(
            &self.dir,
            &self.run_id,
            self.llm.clone(),
            self.snapshot_store.clone(),
        )?;
        Ok(LlmSummarizeCompressor::new(deps, model_alias, target))
    }

    /// **单步驱动**:跑一次 `LLMContext::run().await`,落盘,刷新 meta。
    ///
    /// 通常通过 [`Self::drive_to_terminal`] 调用;暴露出来给需要逐步驱动
    /// 的高级 caller(例:测试、嵌入 workflow 的 leaf node)。
    ///
    /// 调完之后 `self.ctx` 在挂起态下会变 `None`(等待 resume);终态下
    /// 也变 `None`(对象一次性消耗)。
    pub async fn step(&mut self) -> Result<LLMContextOutcome, LocalLLMContextError> {
        let mut ctx = self
            .ctx
            .take()
            .ok_or(LocalLLMContextError::NoActiveContext)?;

        let outcome = ctx.run().await;

        // **每个 outcome 边界落盘**(crash recovery 纪律)。
        let idx = self.snapshot_store.put_next(&ctx.snapshot())?;
        self.meta.latest_snapshot_idx = Some(idx);
        self.meta.last_updated_unix_ms = now_unix_ms();

        // 按 outcome 形态更新 status / last_suspend_kind。
        match &outcome {
            LLMContextOutcome::Done { .. }
            | LLMContextOutcome::Error { .. }
            | LLMContextOutcome::BudgetExhausted { .. } => {
                self.meta.status = RunStatus::Completed;
                self.meta.last_suspend_kind = None;
                write_run_state(&self.dir, &self.meta)?;
                write_run_outcome(&self.dir, &self.run_id, &outcome)?;
                // ctx 已经 take,且终态不可 resume → 保持 None。
            }
            LLMContextOutcome::PendingTool { .. } => {
                self.meta.status = RunStatus::Suspended;
                self.meta.last_suspend_kind = Some(SuspendKind::PendingTool);
                write_run_state(&self.dir, &self.meta)?;
            }
            LLMContextOutcome::ContextLimitReached { .. } => {
                // 注意:不把 status 改成 Suspended——`drive_to_terminal` 会立刻
                // 在外面消化掉。如果 caller 用 `step()` 自己驱动,会看到这个
                // outcome 但 status 仍是 Running,这是有意的:context limit 不是
                // "等外部输入"的真正挂起,只是 waist 让 scheduler 决定压缩策略
                // 的让出点。
                self.meta.last_suspend_kind = Some(SuspendKind::ContextLimitReached);
                write_run_state(&self.dir, &self.meta)?;
            }
            LLMContextOutcome::Interrupted { .. } => {
                // §3.13:scheduler 从 run 外部抢占了本轮 inference。
                // snapshot 是 s0(本轮 inference 前的状态),恢复路径与"运行
                // 中崩溃"共享 `ResumeFromMidRun` 语义。把 status 标成 Suspended
                // 并记下 SuspendKind::Interrupted,让 resume_or_new 看到时能区
                // 分"挂起态崩溃"与"正常运行中崩溃"。
                self.meta.status = RunStatus::Suspended;
                self.meta.last_suspend_kind = Some(SuspendKind::Interrupted);
                write_run_state(&self.dir, &self.meta)?;
            }
        }

        Ok(outcome)
    }

    /// 内部:处理 `ContextLimitReached` 之后用压缩后的 history 继续。
    fn resume_with_rewritten_history(
        &mut self,
        snapshot: LLMContextSnapshot,
        rewritten: Vec<AiMessage>,
    ) -> Result<(), LocalLLMContextError> {
        let deps = build_deps(
            &self.dir,
            &self.run_id,
            self.llm.clone(),
            self.snapshot_store.clone(),
        )?;
        // **resume 前落盘**(crash recovery 第二个落点)。
        self.snapshot_store.put_next(&snapshot)?;
        let ctx = LLMContext::resume(
            snapshot,
            ResumeFill::RewrittenHistory { history: rewritten },
            deps,
        )
        .map_err(|e| LocalLLMContextError::CorruptedRun {
            run_id: self.run_id.clone(),
            reason: format!("resume with RewrittenHistory failed: {e}"),
        })?;
        self.ctx = Some(ctx);
        self.meta.last_suspend_kind = None;
        self.meta.last_updated_unix_ms = now_unix_ms();
        write_run_state(&self.dir, &self.meta)?;
        Ok(())
    }

    // ---------- 公共 accessors ----------

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn meta(&self) -> &RunMetaState {
        &self.meta
    }

    pub fn request(&self) -> &OneShotRequest {
        &self.request
    }

    // ---------- 内部 helpers ----------

    fn ensure_dir_layout(dir: &Path) -> Result<(), LocalLLMContextError> {
        std::fs::create_dir_all(dir.join("runs"))?;
        std::fs::create_dir_all(dir.join("workspace"))?;
        // `<dir>/bin` is the user-owned PATH overlay slot for exec_bash; the
        // agent_tool side only prepends it onto PATH, so we have to mkdir it
        // here. Users drop chmod +x scripts in to get LLM-callable CLIs.
        std::fs::create_dir_all(dir.join("bin"))?;
        Ok(())
    }

    /// 拿目录级 flock。**同进程重入返回 Ok**(`resume_or_new` 内部会用到)。
    ///
    /// 这是工程权衡:严格的"per-process lock 只能拿一次"会让 `resume_or_new`
    /// 的实现非常啰嗦(必须把 `new_run` 拆成无锁内部版本)。考虑到 OneShot
    /// 总是单进程使用,同进程重入是安全的。
    ///
    /// 实现:`<dir>/.lock` 文件 + `fs2::FileExt::try_lock_exclusive`。锁在
    /// 进程级 registry 里挂着,生命周期 = 进程退出(File 句柄关闭 → flock 释放)。
    /// 不再尝试 release(避免 TOCTOU),配合 `drop_lock_if_held` 的 no-op 语义。
    fn acquire_dir_lock(dir: &Path) -> Result<(), LocalLLMContextError> {
        let key = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
        let mut guard = dir_lock_registry()
            .lock()
            .map_err(|_| LocalLLMContextError::LockFailed("dir lock registry poisoned".into()))?;
        if guard.contains_key(&key) {
            return Ok(());
        }
        let lock_path = dir.join(".lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(&lock_path)?;
        FileExt::try_lock_exclusive(&file).map_err(|e| {
            LocalLLMContextError::LockFailed(format!(
                "another process holds the dir lock on {}: {e}",
                dir.display()
            ))
        })?;
        guard.insert(key, file);
        Ok(())
    }

    /// **基于上一次跑完的 run 拼一个"追加 user 消息"的 follow-up request**。
    ///
    /// 用途:让 OneShot 在 CLI 层"像 agent 一样"被驱动——上一轮 run 跑完
    /// (`status=Completed`)后,用这个方法把上一轮 snapshot 里的 `accumulated`
    /// 历史 + 一条新消息打成新的 [`OneShotRequest`],再交给 [`Self::new_run`]
    /// 起新 run。新 run 的 `run_id` **不复用**,每"轮对话"对应一个独立的
    /// `<dir>/runs/<run_id>/`,审计链保持清晰。
    ///
    /// 行为:
    /// - `dir` 下找最近一次 run(按 `last_updated_unix_ms`)。不存在 →
    ///   [`LocalLLMContextError::NoCompletedRunToAppend`]。
    /// - 该 run 必须是 [`RunStatus::Completed`];`Running` / `Suspended` 都拒绝
    ///   (那些有自己的处理路径:`resume_or_new` / 低层 `ResumeFill` API)。
    /// - 拷贝 `request.json` 的其它字段(objective / model_policy / tool_policy /
    ///   output / budget / human_policy / error_policy);`input` 设为 snapshot
    ///   的 `state.accumulated` + `new_message`。
    ///
    /// **不持锁、不写盘**——纯只读。caller 拿到结果之后还需要调
    /// [`Self::new_run`] 才会真正启动新 run(那里会走完整 lock + write 流程)。
    pub fn prepare_followup_request(
        dir: &Path,
        new_message: AiMessage,
    ) -> Result<OneShotRequest, LocalLLMContextError> {
        let latest = Self::find_latest_run(dir)?.ok_or_else(|| {
            LocalLLMContextError::NoCompletedRunToAppend {
                hint: format!("no prior run found under {}/runs", dir.display()),
            }
        })?;
        if latest.status != RunStatus::Completed {
            return Err(LocalLLMContextError::NoCompletedRunToAppend {
                hint: format!(
                    "latest run `{}` has status {:?}; only Completed runs can be appended to",
                    latest.run_id, latest.status
                ),
            });
        }
        let idx = latest
            .latest_snapshot_idx
            .ok_or_else(|| LocalLLMContextError::CorruptedRun {
                run_id: latest.run_id.clone(),
                reason: "completed run has no snapshot on disk".into(),
            })?;
        let snapshot_store = FileSnapshotStore::new(dir.join("runs").join(&latest.run_id));
        let snapshot = snapshot_store.get(idx)?;
        let prior_request = read_run_request(dir, &latest.run_id)?;
        let mut input = snapshot.state.accumulated;
        input.push(new_message);
        Ok(OneShotRequest {
            objective: prior_request.objective,
            input,
            model_policy: prior_request.model_policy,
            tool_policy: prior_request.tool_policy,
            output: prior_request.output,
            budget: prior_request.budget,
            human_policy: prior_request.human_policy,
            error_policy: prior_request.error_policy,
        })
    }

    /// 扫描 `<dir>/runs/*/state.json`,返回 `last_updated_unix_ms` 最大的那条
    /// meta(不限 status)。给 [`Self::prepare_followup_request`] 用——它需要
    /// 找到"最后那次跑的 run"无论状态,再自己校验。
    fn find_latest_run(dir: &Path) -> Result<Option<RunMetaState>, LocalLLMContextError> {
        let runs_dir = dir.join("runs");
        if !runs_dir.exists() {
            return Ok(None);
        }
        let mut candidates: Vec<RunMetaState> = Vec::new();
        for entry in std::fs::read_dir(&runs_dir)? {
            let entry = entry?;
            let state_path = entry.path().join("state.json");
            if !state_path.is_file() {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&state_path) {
                if let Ok(meta) = serde_json::from_slice::<RunMetaState>(&bytes) {
                    candidates.push(meta);
                }
            }
        }
        candidates.sort_by_key(|m| m.last_updated_unix_ms);
        Ok(candidates.pop())
    }

    /// 扫描 `<dir>/runs/*/state.json`,找 `status=Running` 的那个。
    /// 同一时刻应当最多有一个;有多个时返回 `latest_updated` 最大的那个,
    /// 并 emit 一条警告(暗示之前有过未释放的崩溃残留)。
    fn find_running_run(dir: &Path) -> Result<Option<RunMetaState>, LocalLLMContextError> {
        let runs_dir = dir.join("runs");
        if !runs_dir.exists() {
            return Ok(None);
        }
        let mut candidates: Vec<RunMetaState> = Vec::new();
        for entry in std::fs::read_dir(&runs_dir)? {
            let entry = entry?;
            let state_path = entry.path().join("state.json");
            if !state_path.is_file() {
                continue;
            }
            if let Ok(bytes) = std::fs::read(&state_path) {
                if let Ok(meta) = serde_json::from_slice::<RunMetaState>(&bytes) {
                    if meta.status == RunStatus::Running {
                        candidates.push(meta);
                    }
                }
            }
        }
        candidates.sort_by_key(|m| m.last_updated_unix_ms);
        Ok(candidates.pop())
    }
}

// =========================================================================
// 配套抽象:SnapshotStore / Compressor
// =========================================================================

/// snapshot 持久化抽象(§A.4 "snapshot 存储介质是接口实现细节" 的落点)。
///
/// 默认实现 `FileSnapshotStore` 把 snapshot 写到 `<run>/snapshots/<idx>.snap.json`;
/// 测试可注入 in-memory 实现。
pub trait SnapshotStore: Send + Sync {
    /// 写一份 snapshot,返回它在 store 内的单调递增 idx。
    fn put_next(&self, snapshot: &LLMContextSnapshot) -> Result<u32, LocalLLMContextError>;
    /// 按 idx 读回 snapshot。
    fn get(&self, idx: u32) -> Result<LLMContextSnapshot, LocalLLMContextError>;
    /// 列出所有 idx,小到大。
    fn list(&self) -> Result<Vec<u32>, LocalLLMContextError>;
}

/// 压缩策略抽象(§A.4 "上下文压缩策略不进 waist" 的落点)。
///
/// caller 注入具体实现。典型选择:
/// - 朴素 drop-oldest 保留最近 N 轮(单测 / 简单 CLI 场景);
/// - 调一个独立的 LLM 做 summarize-and-replace(生产 OneShot 默认);
/// - 把 accumulated 切片后压成 system summary block + tail(更精细)。
#[async_trait]
pub trait Compressor: Send + Sync {
    /// 输入:当前累积的对话历史 + 工作目录(允许 compressor 把中间产物
    /// 写到工作目录里,例:summarize 的中间 prompt / 结果)。
    /// 输出:压缩后的对话历史。
    async fn compress(
        &self,
        accumulated: Vec<AiMessage>,
        dir: &Path,
    ) -> Result<Vec<AiMessage>, LocalLLMContextError>;
}

// =========================================================================
// 默认 SnapshotStore 实现:文件系统
// =========================================================================

pub struct FileSnapshotStore {
    run_dir: PathBuf,
}

impl FileSnapshotStore {
    pub fn new(run_dir: PathBuf) -> Self {
        Self { run_dir }
    }
    fn snapshots_dir(&self) -> PathBuf {
        self.run_dir.join("snapshots")
    }
    fn path_for(&self, idx: u32) -> PathBuf {
        self.snapshots_dir().join(format!("{:04}.snap.json", idx))
    }
}

impl SnapshotStore for FileSnapshotStore {
    fn put_next(&self, snapshot: &LLMContextSnapshot) -> Result<u32, LocalLLMContextError> {
        std::fs::create_dir_all(self.snapshots_dir())?;
        let next_idx = self.list()?.last().copied().map_or(1, |i| i + 1);
        let bytes = serde_json::to_vec_pretty(snapshot)
            .map_err(|e| LocalLLMContextError::Serialization(e.to_string()))?;
        // 简单原子写:先写 .tmp 再 rename。
        let path = self.path_for(next_idx);
        let tmp = path.with_extension("snap.json.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        Ok(next_idx)
    }

    fn get(&self, idx: u32) -> Result<LLMContextSnapshot, LocalLLMContextError> {
        let bytes = std::fs::read(self.path_for(idx))?;
        serde_json::from_slice(&bytes)
            .map_err(|e| LocalLLMContextError::Serialization(e.to_string()))
    }

    fn list(&self) -> Result<Vec<u32>, LocalLLMContextError> {
        let dir = self.snapshots_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut idxs: Vec<u32> = std::fs::read_dir(&dir)?
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let stem = name.strip_suffix(".snap.json")?;
                stem.parse::<u32>().ok()
            })
            .collect();
        idxs.sort();
        Ok(idxs)
    }
}

// =========================================================================
// 错误类型
// =========================================================================

#[derive(Debug, thiserror::Error)]
pub enum LocalLLMContextError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("serialization error: {0}")]
    Serialization(String),
    #[error("dir already has a running run `{run_id}`; {hint}")]
    RunningRunExists { run_id: String, hint: String },
    #[error(
        "semantic hash mismatch for running run `{run_id}` (on-disk: {on_disk_hash}, \
         provided: {provided_hash}); refuse to auto-resume"
    )]
    SemanticHashMismatch {
        run_id: String,
        on_disk_hash: u64,
        provided_hash: u64,
    },
    #[error("run `{run_id}` is corrupted: {reason}")]
    CorruptedRun { run_id: String, reason: String },
    #[error("run `{run_id}` crashed while suspended ({kind:?}); {hint}")]
    CrashedInSuspended {
        run_id: String,
        kind: SuspendKind,
        hint: String,
    },
    #[error("no active LLMContext; either already terminated or awaiting external resume")]
    NoActiveContext,
    #[error("compressor failed: {0}")]
    CompressorFailed(String),
    #[error("cannot prepare follow-up request: {hint}")]
    NoCompletedRunToAppend { hint: String },
    #[error("failed to acquire dir lock: {0}")]
    LockFailed(String),
    #[error("failed to wire tool: {0}")]
    ToolWiringFailed(String),
}

// =========================================================================
// 内部:目录持久化函数
// =========================================================================

fn write_run_state(dir: &Path, meta: &RunMetaState) -> Result<(), LocalLLMContextError> {
    let run_dir = dir.join("runs").join(&meta.run_id);
    std::fs::create_dir_all(&run_dir)?;
    let path = run_dir.join("state.json");
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(meta)
        .map_err(|e| LocalLLMContextError::Serialization(e.to_string()))?;
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

fn write_run_request(
    dir: &Path,
    run_id: &str,
    req: &OneShotRequest,
) -> Result<(), LocalLLMContextError> {
    let run_dir = dir.join("runs").join(run_id);
    std::fs::create_dir_all(&run_dir)?;
    let path = run_dir.join("request.json");
    let bytes = serde_json::to_vec_pretty(req)
        .map_err(|e| LocalLLMContextError::Serialization(e.to_string()))?;
    std::fs::write(path, bytes)?;
    Ok(())
}

fn read_run_request(dir: &Path, run_id: &str) -> Result<OneShotRequest, LocalLLMContextError> {
    let path = dir.join("runs").join(run_id).join("request.json");
    let bytes = std::fs::read(&path)?;
    serde_json::from_slice(&bytes).map_err(|e| LocalLLMContextError::Serialization(e.to_string()))
}

fn write_run_outcome(
    dir: &Path,
    run_id: &str,
    outcome: &LLMContextOutcome,
) -> Result<(), LocalLLMContextError> {
    let run_dir = dir.join("runs").join(run_id);
    std::fs::create_dir_all(run_dir.join("outcomes"))?;
    let path = run_dir.join("outcomes").join("final.json");
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(outcome)
        .map_err(|e| LocalLLMContextError::Serialization(e.to_string()))?;
    std::fs::write(&tmp, &bytes)?;
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

/// 给 `drive_to_terminal_auto` 用的目标 token 预算推导。
///
/// 优先级:
/// 1. `budget.max_total_tokens` → 取 60%(留 40% 余量给 resume 后继续累计)。
/// 2. `context_yield_threshold = AbsoluteTokens(N)` → 同样取 60%(yield 阈值
///    意味着"再增长就要让出来",压缩目标显然得低于它)。
/// 3. 兜底 [`DEFAULT_AUTO_COMPRESS_TARGET_TOKENS`]。
///
/// 不处理 `ContextThreshold::Ratio` —— 那是相对 provider window 的比例,OneShot
/// 这一层不知道 window 大小;靠 caller 给绝对信号才能用上。
fn auto_compress_target_tokens(req: &OneShotRequest) -> u32 {
    if let Some(b) = req.budget.as_ref() {
        if let Some(max) = b.max_total_tokens {
            return ((max as u64) * 60 / 100).max(8_192).min(u32::MAX as u64) as u32;
        }
        if let Some(ContextThreshold::AbsoluteTokens { value }) = b.context_yield_threshold {
            return ((value as u64) * 60 / 100).max(8_192) as u32;
        }
    }
    DEFAULT_AUTO_COMPRESS_TARGET_TOKENS
}

fn generate_run_id() -> String {
    // 形如 20260510-103045-a7f3。用本地时间(给 ops 读 dir 时友好),
    // 后缀 4 字节 hex 防同秒冲突。
    let ts = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let suffix = format!("{:04x}", (now_ms & 0xffff) ^ ((now_ms >> 16) & 0xffff));
    format!("{}-{}", ts, suffix)
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// **故意 no-op**(进程级 lock 不释放,避免 `resume_or_new → new_run` 之间的
/// TOCTOU 窗口)。当 `LocalLLMContext` drop / 进程退出时,registry 里的 `File`
/// 句柄随之关闭,OS 自动释放 flock。
fn drop_lock_if_held(_dir: &Path) {}

static DIR_LOCK_REGISTRY: OnceLock<Mutex<HashMap<PathBuf, File>>> = OnceLock::new();

fn dir_lock_registry() -> &'static Mutex<HashMap<PathBuf, File>> {
    DIR_LOCK_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn build_deps(
    dir: &Path,
    run_id: &str,
    llm: Arc<dyn LlmClient>,
    snapshot_store: Arc<dyn SnapshotStore>,
) -> Result<LLMContextDeps, LocalLLMContextError> {
    let tools: Arc<dyn ToolManager> = Arc::new(LocalDirToolManager::new(
        dir.to_path_buf(),
        run_id.to_string(),
    )?);
    let worklog: Arc<dyn WorklogSink> = Arc::new(NoopWorklogSink);
    let hook: Arc<dyn TurnHook> = Arc::new(SnapshotPersistingTurnHook { snapshot_store });
    Ok(LLMContextDeps::new(llm, tools)
        .with_worklog(worklog)
        .with_turn_hook(hook))
}

/// `TurnHook` 实现:每轮 LLM 推理前把当前 snapshot 落盘。落盘失败时仅吞掉
/// 错误——hook 不允许中断 waist 主循环(§3.12 约束)。最坏情况下当前轮的
/// snapshot 会丢失,但 outcome 边界还会再写一次,所以崩溃恢复仍然安全,
/// 只是恢复粒度退化到 outcome 边界。
struct SnapshotPersistingTurnHook {
    snapshot_store: Arc<dyn SnapshotStore>,
}

impl TurnHook for SnapshotPersistingTurnHook {
    fn before_inference(&self, snapshot: &LLMContextSnapshot) {
        let _ = self.snapshot_store.put_next(snapshot);
    }
}

// =========================================================================
// LocalDirToolManager:把 agent_tool 适配到 waist
// =========================================================================

/// 把 `agent_tool::AgentToolManager` 适配到 waist 的 `ToolManager` trait。
///
/// 设计契约(摘自上一版 review 的结论,这里保留作为规范注释):
/// - **入站**:`AiToolCall { name, args, call_id }` —— waist 归一化产物。
/// - **出站**:`Observation::{Success | Error | Pending}` —— waist 归一化产物。
/// - 中间一段对 `AgentToolResult.status` 的三态做映射:
///     - `Success` → `Observation::Success`(`content` 取 `details`)
///     - `Error`   → `Observation::Error`(`message` 取 `summary`)
///     - `Pending` → `Observation::Pending`(仅当 `ToolPolicy.allow_deferred = true`
///       时合法,否则 waist 视为 Fatal,见 §3.5)
/// - 工具 root 是 `<dir>/workspace`,所有 read/write/edit/glob/grep 必须
///   sandbox 在这个目录下(防止 LLM "改我的源码")。
///
/// **不要把 agent_tool 内部类型泄漏到 waist 公共接口** —— effect 实现层
/// 细节,泄漏即破坏双中立性(§A.2)。
pub struct LocalDirToolManager {
    workspace: PathBuf,
    inner: AgentToolManager,
    step_idx: AtomicU32,
    session_template: SessionRuntimeContext,
}

/// exec_bash 默认 timeout(LLM 单次工具调用),与 OpenDAN agent_bash 默认值对齐。
const EXEC_BASH_DEFAULT_TIMEOUT_MS: u64 = 30_000;
/// exec_bash 上限 timeout,防止 LLM 把 timeout_ms 设到天文数字。
const EXEC_BASH_MAX_TIMEOUT_MS: u64 = 120_000;
/// exec_bash 单次合并输出上限(stdout+stderr 截断阈值)。
const EXEC_BASH_MAX_OUTPUT_BYTES: usize = 256 * 1024;

impl LocalDirToolManager {
    /// 构造:在 `<dir>/workspace` 上注册 WriteFile / EditFile /
    /// ExecBash。`dir` 是 OneShot 根目录(由 `ensure_dir_layout`
    /// 保证 `workspace` 和 `bin` 子目录存在)。`run_id` 进
    /// `SessionRuntimeContext.trace_id` / `session_id`,让 agent_tool 的日志
    /// 能跟 OneShot run 关联。
    pub fn new(dir: PathBuf, run_id: String) -> Result<Self, LocalLLMContextError> {
        let workspace = dir.join("workspace");
        let bin_dir = dir.join("bin");
        let inner = AgentToolManager::new();
        let cfg = FileToolConfig::new(workspace.clone());
        let write_audit = Arc::new(NoopFileWriteAudit);
        // `read` is owned by agent-did-object-lib at the runtime/CLI layer.
        // Local one-shot context keeps file inspection available through
        // exec_bash.
        inner
            .register_typed_tool(WriteFileTool::new(cfg.clone(), write_audit.clone()))
            .map_err(|e| LocalLLMContextError::ToolWiringFailed(e.to_string()))?;
        inner
            .register_typed_tool(EditFileTool::new(cfg, write_audit))
            .map_err(|e| LocalLLMContextError::ToolWiringFailed(e.to_string()))?;

        // exec_bash:cwd = `<dir>/workspace`,PATH overlay = `<dir>/bin`。
        // 用户把 chmod +x 的脚本放进 bin/,LLM 通过 exec_bash 直接命中,
        // 同名时盖过系统 PATH。
        let bash_cfg = LlmBashConfig::local_workspace(workspace.clone())
            .with_overlay(BinOverlayConfig::local(bin_dir))
            .with_default_timeout_ms(EXEC_BASH_DEFAULT_TIMEOUT_MS)
            .with_max_timeout_ms(EXEC_BASH_MAX_TIMEOUT_MS)
            .with_max_output_bytes(EXEC_BASH_MAX_OUTPUT_BYTES)
            .with_allow_env(true);
        inner
            .register_tool(ExecBashTool::new(bash_cfg))
            .map_err(|e| LocalLLMContextError::ToolWiringFailed(e.to_string()))?;

        let session_template = SessionRuntimeContext {
            trace_id: run_id.clone(),
            agent_name: "oneshot".to_string(),
            behavior: "oneshot".to_string(),
            step_idx: 0,
            wakeup_id: String::new(),
            session_id: run_id,
            read_token_limit: crate::DEFAULT_READ_TOKEN_LIMIT,
        };
        Ok(Self {
            workspace,
            inner,
            step_idx: AtomicU32::new(0),
            session_template,
        })
    }

    pub fn workspace(&self) -> &Path {
        &self.workspace
    }
}

#[async_trait]
impl ToolManager for LocalDirToolManager {
    async fn call_tool(&self, call: AiToolCall) -> Observation {
        let call_id = call.call_id.clone();
        let mut ctx = self.session_template.clone();
        ctx.step_idx = self.step_idx.fetch_add(1, Ordering::SeqCst) + 1;
        match self.inner.call_tool(&ctx, call).await {
            Ok(result) => map_result_to_observation(call_id, result),
            Err(e) => Observation::Error {
                call_id,
                message: e.to_string(),
                tool_result: None,
            },
        }
    }

    fn list_tool_specs(&self) -> Vec<ToolSpecLite> {
        self.inner
            .list_tool_specs()
            .into_iter()
            .map(|spec| ToolSpecLite {
                name: spec.name,
                description: spec.description,
                args_schema: spec.args_schema,
            })
            .collect()
    }
}

/// 三态映射：旧 `content/message` 继续保留为 provider tool-result fallback，
/// 结构化 `tool_result` 则给 StepRecord renderer 做 Full/Medium/Min 分级渲染。
fn map_result_to_observation(call_id: String, result: AgentToolResult) -> Observation {
    let tool_result = Some(result.to_tool_result_view());
    match result.status {
        AgentToolStatus::Success => {
            let text = result
                .output
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| result.summary.clone());
            let bytes = text.len();
            Observation::Success {
                call_id,
                content: serde_json::Value::String(text),
                bytes,
                truncated: false,
                tool_result,
            }
        }
        AgentToolStatus::Error => {
            let message = if !result.summary.trim().is_empty() {
                result.summary
            } else if let Some(out) = result
                .output
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                out.to_string()
            } else {
                "tool error".to_string()
            };
            Observation::Error {
                call_id,
                message,
                tool_result,
            }
        }
        AgentToolStatus::Pending => Observation::Pending {
            call_id,
            tool_result,
        },
    }
}

// =========================================================================
// 后续待办(本 L4 自己能闭环的,已不依赖 waist)
// =========================================================================
//
// 1. **轮前落盘的失败可观测性** —— `SnapshotPersistingTurnHook::before_inference`
//    当前吞掉 IO 错误以保持"hook 不打断主循环"约束。后续可以挂一个 log::warn
//    + 计数器,让 ops 能看到"轮前落盘最近失败次数"。
