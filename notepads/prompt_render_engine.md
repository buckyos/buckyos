# Prompt Render Engine — 重构需求

> 目标：把 `src/frame/opendan/src/prompt_render_engine.rs` 从「agent_environment 大杂烩」**改造成一个纯粹的、可复用的、基于模板的文本渲染引擎**，并把整个渲染 + 预算 + 装配栈下沉到 `llm_context` crate。
>
> 同时把 `old/opendan/src/behavior/prompt.rs` 里**与模板渲染相关**的合并进来；**所有与 session / workspace / todo / msg-queue / contact 相关的逻辑都不属于引擎**，要么由调用方预渲染成字符串塞进 KV，要么通过 `ValueLoader` trait 注入。
>
> **为什么放进 `llm_context`**：未来还有别的 LLM 上层（workflow DSL、oneshot 类调度、文档生成 / 报告渲染……）也要构造 prompt 文本。把「模板 → 预算 → 装配成 `Vec<AiMessage>`」这条流水线放在 `llm_context` 里，与已有的 `Tokenizer` / `LLMResultParser` / `StepRenderer` / `prompt_budget` 同住，能让 opendan 与 workflow 共用同一套工具盒，opendan 只剩下「填 ValueLoader + 配 SectionSpec」一层很薄的胶水。

## 1. 设计原则

1. **纯函数式**：输入 = 模板字符串 + KV 上下文 + （可选）外部解析回调；输出 = 渲染后的文本 + 统计。没有任何全局状态、没有 session/workshop 依赖。
2. **小而强类型的扩展点**：四种语法基元 + 一个调用方提供的异步 value loader 闭包，构成全部能力。引擎不感知 agent 语义。
3. **沙箱默认安全**：文件包含必须显式给定 root（不允许逃逸）；shell 执行默认关闭，调用方按需开启。
4. **失败可恢复**：单个指令失败（文件不存在、命令超时、变量未注册）不会让整个渲染崩溃；返回 `RenderStats` 让调用方决定如何处置。
5. **零 agent 词汇**：源码里不应再出现 `Workshop` / `Session` / `Contact` / `Todo` / `MsgQueue` / `Memory` 等词。`OPENDAN_` 这种命名空间前缀也一并清掉。

## 2. 新的 API 形状

```rust
//! src/frame/llm_context/src/prompt_engine.rs
//! 重命名建议：`prompt_render_engine` → `prompt_engine`（与 crate 内
//! 其它模块 `prompt_budget` / `prompt_compose` 对齐；下文使用新名）。

pub struct PromptRenderEngine {
    config: EngineConfig,
}

#[derive(Clone, Debug)]
pub struct EngineConfig {
    /// 单个 __INCLUDE__ 加载的最大字节数。默认 64 KiB。
    pub max_include_bytes: usize,
    /// 整个渲染输出的硬上限。默认 256 KiB；超出截断并标记。
    pub max_total_bytes: usize,
    /// __EXEC__ 单次命令的超时。默认 10 s。
    pub exec_timeout: Duration,
    /// 是否允许 __EXEC__ 指令。默认 false（沙箱友好）。
    pub allow_exec: bool,
    /// __INCLUDE__ 的根目录白名单；空则禁用文件包含。
    pub include_roots: Vec<PathBuf>,
    /// 单次渲染的总递归深度（防止 __INCLUDE__ 循环引用）。默认 8。
    pub max_recursion_depth: u8,
}

impl Default for EngineConfig { /* 见默认值 */ }

impl PromptRenderEngine {
    pub fn new(config: EngineConfig) -> Self;
    pub fn with_defaults() -> Self;

    /// 渲染入口。`vars` 提供静态 KV；`loader` 提供动态 KV 的异步解析；
    /// 二者都不存在的占位符走 `not_found` 计数器（不报错）。
    pub async fn render<L>(
        &self,
        template: &str,
        vars: &RenderVars,
        loader: L,
    ) -> Result<RenderResult, RenderError>
    where
        L: ValueLoader;
}

/// 调用方提供的"按需取值"回调。所有 agent / session 相关的 KV
/// 计算都通过这个 trait 注入，引擎本身完全无知。
#[async_trait]
pub trait ValueLoader: Send + Sync {
    /// 解析 `$expr`（来自 __VAR__）或 `{name}` 占位符。
    /// 返回 `Ok(None)` 表示「不认识这个名字」，引擎会把它计入 `not_found`。
    async fn load(&self, expr: &str) -> Result<Option<serde_json::Value>, RenderError>;
}

/// 静态 KV。`env` 走 __ENV__ 指令；`vars` 走 {name} / __VAR__。
#[derive(Default, Clone, Debug)]
pub struct RenderVars {
    pub env: HashMap<String, serde_json::Value>,
    pub vars: HashMap<String, serde_json::Value>,
}

/// 渲染结果 + 统计。`rendered` 永远是最终文本。
#[derive(Debug)]
pub struct RenderResult {
    pub rendered: String,
    pub stats: RenderStats,
    /// __VAR__ 注册的变量名 → 是否成功解析。
    pub resolved_vars: HashMap<String, bool>,
    /// 输出是否触顶 `max_total_bytes` 被截断。
    pub truncated: bool,
}

#[derive(Default, Debug, Clone)]
pub struct RenderStats {
    pub env_expanded: u32,
    pub env_not_found: u32,
    pub content_loaded: u32,
    pub content_failed: u32,
    pub exec_run: u32,
    pub exec_failed: u32,
    pub var_registered: u32,
    pub var_failed: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("template syntax error: {0}")]
    Syntax(String),
    #[error("recursion depth exceeded ({0})")]
    RecursionTooDeep(u8),
    #[error("loader failure: {0}")]
    Loader(String),
}
```

设计要点：

- **没有 `AgentEnvironment`**、**没有 `PromptTemplateContext`**、**没有 `register_workshop_tools`**。
- **不在公共签名上暴露 `upon` 类型**，避免外部代码绑死在某个模板库实现上（内部实现可以继续用 `upon`）。
- `ValueLoader` 是异步 trait，能接外面任意能算字符串的东西（数据库、session 状态、远程 API…）。引擎对调用方一无所知。
- `EngineConfig` 把所有沙箱开关收敛在一处，方便不安全路径单测。

## 3. 模板语法（保留 + 重命名）

四个指令 + 一个 `{name}` 占位符。**旧的 `__OPENDAN_*` 前缀全部去掉**（beta2.2 的破坏性 rename，干净利落）。

| 旧形式 | 新形式 | 语义 |
|--------|--------|------|
| `__OPENDAN_ENV(expr)__` | `__ENV(expr)__` | 从 `RenderVars.env` 取值。`expr` 是单个 key（`session_id`）或点路径（`owner.name`）。 |
| `__OPENDAN_CONTENT(path)__` | `__INCLUDE(path)__` | 加载文件。`path` 必须在 `EngineConfig.include_roots` 白名单下；支持 `~` / `$HOME` 展开；硬上限 `max_include_bytes`。 |
| `__OPENDAN_EXEC(cmd)__` | `__EXEC(cmd)__` | 执行 shell；仅当 `allow_exec=true` 时启用，超时按 `exec_timeout`，stdout 作为返回值，stderr 仅日志。 |
| `__OPENDAN_VAR(name, $expr)` | `__VAR(name, $expr)__` | 注册一个动态变量：渲染时调用 `loader.load("$expr")` 拿值，绑定到 `vars[name]`，供后续 `{name}` 引用。 |
| `{var}` | `{var}` | 静态/动态变量替换（内部仍走 `upon`）。字面 `{` `}` 用 `{{` `}}` 转义。 |

实现细节：
- 多 pass：先扫一遍所有 `__VAR__` 声明做名字收集与去重；再异步并发解析；最后一遍 substitute + upon。
- `__INCLUDE__` 加载到的文本递归再过一遍引擎（受 `max_recursion_depth` 约束），所以模板片段可以嵌套别的指令。
- 引擎内置 sentinel-based escape pass，保证文件内容里的 `{` `}` 不会被 `upon` 误解析（即原有 `ESCAPED_*_SENTINEL` 机制保留）。
- 所有失败都计数后用 `<!-- $directive failed: $reason -->` 占位（或可配置为空串），**不抛错**。

## 4. 与 old/opendan/src/behavior/prompt.rs 的合并

old prompt.rs 是「behavior 层的 LLM 请求装配」。它干的事可以拆成 3 段，每段都能通用化：

```
①  渲染各 section 的模板（include / env / var）   ← prompt_engine
②  按 token 预算把多段截断 / 丢弃                  ← prompt_budget
③  把渲染 + 预算编排起来，输出 Vec<AiMessage>       ← prompt_compose
```

**3 段全部住进 `llm_context` crate**。剩下的 opendan / behavior 私有部分（哪些 bucket 存在、history 怎么从 SQLite 拿、DID 怎么提取人名……）通过 `ValueLoader` 接入。

### ① `llm_context::prompt_engine`
模板渲染本身。设计已在 §2 / §3 描述。无 agent 词汇、无 buckyos_api / name_lib 依赖。

### ② `llm_context::prompt_budget`
token 预算装配的纯算法。详细 API 见 §6.1 / §6.2 / §6.3。无 agent 词汇，依赖现有 `llm_context::Tokenizer` trait。

### ③ `llm_context::prompt_compose`
渲染 + 预算的编排器。把"section 配置 + 模板 + 数据源"翻译成 `Vec<AiMessage>`。详细 API 见 §6.4。

输入是一组 `SectionSpec { key, role, template, priority, min_tokens, trunc }`，输出是 `Vec<AiMessage>`。它内部按顺序：
1. 对每个 section 用 `prompt_engine::render` 渲染模板拿到字符串
2. 把所有渲染后的字符串作为 `BudgetedSection` 喂给 `prompt_budget::PromptBudgeter::fit`
3. 把保留的 section 按原顺序 wrap 成 `AiMessage`（role 来自 SectionSpec）

这一层**没有任何 agent / workflow 词汇**：它只知道「模板字符串 + 数据回调 + 预算 → 一组消息」。
opendan / workflow 各自实现自己的 `ValueLoader` + 写一份 `SectionSpec` 列表即可复用整条流水线。

### 完全不进 `llm_context` 的（仍归 opendan）
opendan 私有的「数据源」逻辑独立成 `src/frame/opendan/src/prompt_sources.rs`（新文件，命名建议）：
- 时间线渲染（`render_memory_timeline_text` / `format_history_*`）
- 历史消息加载与格式化（`load_history_messages_with_limit` / `render_history_msg_line` / `render_history_sender` / `extract_name_from_did` / `normalize_history_multiline_text`）
- session step records / agent memory 加载（`load_session_step_records_with_limit` / `load_agent_memory_with_limit`）
- bucket 名字常量、`contains_any_marker` 等 opendan 私有 heuristic
- 顶层 `OpenDanValueLoader: ValueLoader`：把上面这些拼起来响应 `loader.load("history.messages.recent")` 一类查询；agent_session 里调用 `prompt_compose::compose(...)` 时把它作为 loader 传入。

opendan 不再有"composition" 模块——composition 已经下沉。

### workflow DSL 的复用面
workflow 那边只需要：
- 自己实现一个 `WorkflowValueLoader: ValueLoader`，把 job inputs / 上游 node 输出包装成 `loader.load(...)` 的响应。
- 自己定义一组 workflow 风味的 `SectionSpec`（DSL 里可能直接长成这样）。
- 调用 `prompt_compose::compose(...)`，输出直接是 `Vec<AiMessage>`，可以直接喂进 `LLMContextRequest.input`。

→ workflow 完全不依赖 opendan；与 opendan 共享同一条 LLM 入口装配流水线。

## 5. 要从原 prompt_render_engine.rs（即 agent_environment.rs）删掉的

以下条目全部移除（要么搬到 composition layer，要么搬到调用 site）：

| 类别 | 删除清单（in `prompt_render_engine.rs`） |
|------|----------------------------------------|
| 结构体 | `AgentEnvironment` / `PromptTemplateContext` / `AgentTemplateRenderResult` / `TemplateRenderMode` |
| Workshop / Session 入口 | `register_workshop_tools*` / `agent_env_root` / `local_workspace_manager` |
| 大型分发函数 | `load_value_from_session`（~470 行的"特殊 key 巨型 match"）→ 这些 key 全部下沉到 composition layer 的 ValueLoader 实现 |
| 包装层 | `render_prompt` / `render_prompt_template` / `render_input_block_template` / `build_prompt_template_context` |
| 业务渲染 | `render_new_events_from_kmsgqueue` / `render_human_readable_msg_line` / `build_session_skill_roots` / `resolve_todo_db_path` / `resolve_session_workspace_id` / `load_owner_value_for_prompt` |
| 路径解析 | `non_empty_path` / `resolve_bound_workspace_id` 等——这些已经在 `workspace_path.rs`，保持原位即可 |
| 依赖 import | 砍掉 `buckyos_api` / `name_lib` / `ndn_lib` / `rusqlite` / `tokio::process` 等所有非渲染相关 import |

保留并清理：
- `escape_template_literals` / `unescape_template_literals` / `normalize_text_output` / `prepare_prompt_template` / `truncate_utf8` — 纯字符串工具，留在引擎内部当 private helper。
- `expand_system_env_vars` — 留作 `__INCLUDE__` 路径解析时使用（`$HOME` / `~`），重命名为 `expand_path_env_vars`。
- 4 个指令的 scanner + 多 pass 主循环 — 重组成 `PromptRenderEngine::render` 的 private impl。

## 6. 文件布局

```
src/frame/llm_context/src/
├── prompt_engine.rs       # 新建：纯模板引擎（详 §2 / §3），预计 <700 行
├── prompt_budget.rs       # 新建：通用 token 预算装配（详 §6.1 / §6.2）
└── prompt_compose.rs      # 新建：渲染 + 预算编排器（详 §6.4）

src/frame/opendan/src/
├── prompt_sources.rs      # 新建：opendan 私有的数据源 + OpenDanValueLoader
└── agent_environment.rs   # 删除（功能拆散到 prompt_sources + agent + workspace 三处）
                           #   原 prompt_render_engine.rs 整体删除（内容已下沉）
```

调用关系：
```
opendan::agent_session
      │
      │  compose_llm_input(behavior_cfg, session_state)
      ▼
opendan::prompt_sources::build_section_specs(...)  ──→  Vec<SectionSpec>
      │                                                       │
      │   OpenDanValueLoader { session, workshop, … }          │
      │                                                       ▼
      └──────────────────────────────────────────►  llm_context::prompt_compose::compose
                                                              │
                                                              │  对每个 section：
                                                              ▼
                                                  llm_context::prompt_engine::render
                                                              │
                                                              ▼
                                                  llm_context::prompt_budget::PromptBudgeter::fit
                                                              │
                                                              ▼
                                                          Vec<AiMessage>
```

依赖单向、闭包清晰：
- `prompt_engine` ← 无依赖（在 llm_context 内）
- `prompt_budget` ← 依赖 `llm_context::Tokenizer`
- `prompt_compose` ← 依赖前两者 + `Tokenizer` + `AiMessage` / `AiRole`
- opendan 只依赖以上 3 个模块的公共 API，**反向无依赖**
- workflow 同样只依赖以上 3 个模块（不依赖 opendan）

### 6.1 `llm_context::prompt_budget` API 草案

```rust
//! src/frame/llm_context/src/prompt_budget.rs
//!
//! Token 预算装配。纯算法层 + 配置驱动；调用方提供 sections + tokenizer，
//! 返回按预算 fit 后的最终内容。与 PromptRenderEngine 正交：engine 产
//! 字符串，budgeter 决定如何拼。

use crate::deps::Tokenizer;

/// 一段待装配文本。
#[derive(Debug, Clone)]
pub struct BudgetedSection {
    /// 用于调试 / 返回结果回查；budgeter 不关心其含义。
    pub key: String,
    pub content: String,
    /// 优先级越高越保留。预算先按 priority 桶分配，再在桶内 round-robin。
    pub priority: u8,
    /// 即使整个 section 装不下，仍要保留这么多 **token**；
    /// 若连这都装不下，整段丢弃，调用方在 `dropped` 里能看到。
    pub min_tokens: u32,
    /// 超出预算时从哪头截。
    pub trunc: TruncFrom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TruncFrom {
    /// 从开头截（保留尾部，适合"最新消息"）
    Head,
    /// 从结尾截（保留头部，适合"目录大纲"）
    Tail,
    /// 中间打 `…` 省略号（保留首尾，适合"长文档摘要"）
    Middle,
}

pub struct PromptBudgeter<'a> {
    pub tokenizer: &'a dyn Tokenizer,
    pub total_budget_tokens: u32,
}

#[derive(Debug, Clone)]
pub struct FitOutcome {
    /// 最终保留的 section（按输入顺序），content 已经按预算截好。
    pub kept: Vec<FittedSection>,
    /// 完全丢弃的 section key（连 min_tokens 都装不下）。
    pub dropped: Vec<String>,
    pub tokens_used: u32,
    pub tokens_remaining: u32,
}

#[derive(Debug, Clone)]
pub struct FittedSection {
    pub key: String,
    pub content: String,
    pub tokens: u32,
    /// 是否被截断（true 表示 content < 原 section.content）
    pub truncated: bool,
}

impl<'a> PromptBudgeter<'a> {
    pub fn new(tokenizer: &'a dyn Tokenizer, total_budget_tokens: u32) -> Self;

    /// 多段装配。算法见 §6.2。
    pub fn fit(&self, sections: Vec<BudgetedSection>) -> FitOutcome;

    /// 单段截断到指定预算。供 composition 层做"只压一段"的简单调用。
    pub fn fit_single(&self, text: &str, budget_tokens: u32, trunc: TruncFrom) -> String;
}
```

### 6.2 多 section 装配算法

1. 先用 tokenizer 算每个 section 的 raw token 数；总和 ≤ 预算 ⇒ 全部直接保留，结束。
2. 否则按 priority 分桶（高 → 低），每桶内按输入顺序：
   - 先给每个 section 分 `min_tokens`；若桶内 min 之和已超剩余预算 ⇒ 这个桶按入桶顺序逐个 drop，直到能塞下。
   - 剩余预算按 raw 大小做 round-robin 加配额，直到该 section 拿到的额度 ≥ 自身 raw token 数（即"全保留"）或预算耗尽。
3. 桶处理顺序：高优先级先吃饱，再喂下一桶。
4. 每个 section 用二分查找把 content 截到分配到的额度（按 `TruncFrom` 决定截哪头），保留至少 `min_tokens`。
5. 返回 `FitOutcome`：`kept` 维持输入顺序（不按 priority 重排），`dropped` 记录被踢掉的 key。

### 6.3 prompt_budget 测试要求

`prompt_budget.rs` 独立单测（mock Tokenizer 即可，不依赖 agent）：
1. 全部装得下 ⇒ 全部原样保留，`truncated=false`。
2. 单段超出 ⇒ 按 `TruncFrom::{Head|Tail|Middle}` 各跑一次，验证保留区域正确。
3. 多段不同优先级 ⇒ 高优先级先吃满；低优先级被截或被 drop 到 `dropped`。
4. 一个 section 的 `min_tokens` 大于全部预算 ⇒ 整段进 `dropped`，剩余预算让给其它。
5. 边界：空 section 列表 / 预算为 0 / Tokenizer 返回 0。

### 6.4 `llm_context::prompt_compose` API 草案

```rust
//! src/frame/llm_context/src/prompt_compose.rs
//!
//! Render-then-budget 编排器。把一组 SectionSpec 翻译成 Vec<AiMessage>，
//! 内部依次走 prompt_engine::render → prompt_budget::PromptBudgeter::fit。
//! 完全通用：opendan / workflow / 其它任意 LLM 上层都能用同一个入口。

use buckyos_api::{AiMessage, AiRole};
use crate::deps::Tokenizer;
use crate::prompt_budget::{BudgetedSection, PromptBudgeter, TruncFrom};
use crate::prompt_engine::{
    EngineConfig, PromptRenderEngine, RenderError, RenderStats, RenderVars, ValueLoader,
};

/// 一段待装配的 prompt 内容。模板由 `prompt_engine` 渲染、字符串由
/// `prompt_budget` 装配，最终 wrap 成 1 条 `AiMessage`。
#[derive(Debug, Clone)]
pub struct SectionSpec {
    /// 调试 / 统计回查用，budgeter 也用它作 `key`。
    pub key: String,
    pub role: AiRole,
    pub template: String,
    pub priority: u8,
    pub min_tokens: u32,
    pub trunc: TruncFrom,
    /// section 自己的渲染变量（在全局 vars 之外叠加，**会覆盖**同名全局 var）。
    pub local_vars: Option<RenderVars>,
}

pub struct CompositionRequest<'a> {
    pub sections: Vec<SectionSpec>,
    pub total_budget_tokens: u32,
    /// 全局共享渲染变量。每个 section 渲染时与 `local_vars` 合并。
    pub vars: &'a RenderVars,
    pub engine: &'a PromptRenderEngine,
    pub tokenizer: &'a dyn Tokenizer,
}

#[derive(Debug)]
pub struct CompositionOutcome {
    /// 保留下来的 AiMessage，按 `sections` 输入顺序排列。
    pub messages: Vec<AiMessage>,
    /// 渲染期间被 drop 的 section key（min_tokens 装不下）。
    pub dropped: Vec<String>,
    /// 每个 section 的渲染统计（即使被 drop 也保留，便于排查）。
    pub render_stats: std::collections::HashMap<String, RenderStats>,
    pub tokens_used: u32,
    pub tokens_remaining: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum CompositionError {
    #[error("render section `{key}`: {source}")]
    Render { key: String, #[source] source: RenderError },
}

pub async fn compose<L>(
    request: CompositionRequest<'_>,
    loader: &L,
) -> Result<CompositionOutcome, CompositionError>
where
    L: ValueLoader;
```

### 6.5 prompt_compose 测试要求

`prompt_compose.rs` 独立单测（用 in-memory ValueLoader / mock Tokenizer，不依赖 opendan）：
1. 单段模板渲染 + 预算装得下 ⇒ 输出 1 条 AiMessage，role 正确，内容原样。
2. 多段 + 预算够 ⇒ 输出按输入顺序，role / 内容一一对应。
3. 多段 + 预算不够 ⇒ 低优先级 section 被截，验证截断方向。
4. 某 section min_tokens 超预算 ⇒ 进 `dropped`，其它 section 拿到更多预算。
5. `local_vars` 覆盖 `vars` 同名 key。
6. ValueLoader 在某 section 渲染时返回错误 ⇒ `CompositionError::Render { key, .. }`，且 key 正确。
7. 同一 role 的连续 section ⇒ 输出仍是多条 AiMessage（不强行合并；调用方按需后处理）。

## 7. 测试要求

`prompt_render_engine.rs` 必须有的覆盖（不依赖 agent / session）：
1. 纯 `{var}` 替换 + 转义 `{{` `}}`
2. `__ENV(simple)__` + `__ENV(dotted.path)__` 命中 & 未命中
3. `__INCLUDE(path)__` 命中 / 路径不在白名单 / 文件太大被截断 / 嵌套包含触发递归限制
4. `__EXEC__` 默认禁用时被当作 `not_found` / 启用后正常执行 / 超时被截断 / 失败计入 `exec_failed`
5. `__VAR(name, $expr)__` 通过 loader 解析，后续 `{name}` 引用成功；`$expr` 未注册被计入 `var_failed`
6. 多 pass：`__INCLUDE__` 加载的内容里包含 `__VAR__` / `__ENV__` 也会被处理
7. 输出触顶 `max_total_bytes` 时 `truncated=true`
8. 渲染统计 `RenderStats` 与 `resolved_vars` 数字正确

## 8. 实施 checklist（给 CodeAgent）

按编译可独立通过的顺序。每个阶段独立 `cargo test -p llm_context` 全绿后再进下一阶段；llm_context crate 不依赖 opendan，所以前 8 步完全可以脱离 opendan 跑通。

### 阶段 A — llm_context 内部建栈

1. **`llm_context::prompt_engine` 框架**：定义新 API 类型（`PromptRenderEngine` / `EngineConfig` / `RenderVars` / `RenderResult` / `RenderStats` / `RenderError` / `ValueLoader` trait），`render` 暂返回 `todo!()`，跑通编译。
2. **搬运 escape/unescape/truncate/path-env 工具函数**：从老 `prompt_render_engine.rs`（即 agent_environment.rs）挑纯字符串 helper 进来，加单测。
3. **实现 `{var}` + `__ENV__` + `__VAR__` 三个指令**：占模板能力 80%，覆盖 §7 的 1/2/5/6/7/8。
4. **实现 `__INCLUDE__`**：含 root 白名单、递归限制、字节上限。
5. **实现 `__EXEC__`（默认关闭）**：含 `allow_exec` gate 和超时。
6. **`llm_context::prompt_budget`**：按 §6.1 / §6.2 实现 `PromptBudgeter` + 单元测试（§6.3）。
7. **`llm_context::prompt_compose`**：按 §6.4 实现 `compose(...)` + 单元测试（§6.5）。用 in-memory `ValueLoader` 跑全套，不依赖 opendan。
8. **lib.rs 导出**：把 `PromptRenderEngine` / `PromptBudgeter` / `prompt_compose::compose` 一系列公共类型公开到 crate 根。

### 阶段 B — opendan 接入

9. **新建 `src/frame/opendan/src/prompt_sources.rs`**：搬 `old/opendan/src/behavior/prompt.rs` 里 memory / timeline / history / step_record / agent_memory 加载 + 格式化的所有逻辑；同时实现 `OpenDanValueLoader: ValueLoader`，把当前 `load_value_from_session` 那个巨大 match 翻译过来。
10. **新建 opendan-private 的 SectionSpec 配置翻译**：把 behavior_cfg 里的 "system / output_protocol / memory / input" 等 prompt 段翻译成 `Vec<SectionSpec>`；这一层是 opendan 私有，不进 llm_context。
11. **`agent_session::compose_llm_input` 入口**：调用 `prompt_sources::build_section_specs` → `prompt_compose::compose` → 返回 `Vec<AiMessage>` 给 `LLMContextRequest.input`。
12. **删 `agent_environment.rs`**：旧的 `AgentEnvironment` / `PromptTemplateContext` / `register_workshop_tools*` 全部消失。
13. **回归**：opendan crate `cargo build` + `cargo test` 全绿；UI session 跑一遍最小回路确认 prompt 渲染输出与旧版字节大致一致（允许 diff 由命名空间 rename 带来的差异）。

### 阶段 C — workflow DSL（后续，不在本次重构范围）

workflow 那边只需复用阶段 A 的 3 个模块 + 自己写 `WorkflowValueLoader` 和 DSL → `SectionSpec` 的翻译；不涉及 llm_context 内部改动。

## 9. 后续可选

- `prompt_engine` 内部除 `AiMessage` / `AiRole`（已在 `buckyos_api`）外，理论上能完全独立于 llm_context 的其它部分。如果将来 buckyos 其它子系统（docs / report）想用这套，可以把 `prompt_engine` + `prompt_budget` + `prompt_compose` 整体上移到一个更通用的位置（例如新 crate `text_template` 或 `prompt_kit`）——但本次重构**不**做，留好这个迁移空间即可。
- 给 `ValueLoader` 提供一个 `CompositeLoader { loaders: Vec<Arc<dyn ValueLoader>> }` 默认实现，方便多源拼接（session loader + workspace loader + 全局静态 loader）。
- 给 `prompt_engine` 加一个可选的 `RenderTracer` hook，emit 每个指令的命中事件，方便排查模板问题。
- `prompt_compose` 提供一个可选的"同 role 连续 section 合并成一条 AiMessage"后处理 helper（默认不开，避免破坏 caller 的语义假设）。
