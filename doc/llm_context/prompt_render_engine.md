# Prompt Render Engine — 重构需求

> 目标：把 `src/frame/opendan/src/prompt_render_engine.rs` 从「agent_environment 大杂烩」**改造成一个纯粹的、可复用的、基于模板的文本渲染引擎**，放在 Runtime 层的 `agent_tool` crate（`agent_tool::prompt_engine`）。
>
> 同时把 `old/opendan/src/behavior/prompt.rs` 里**与模板渲染相关**的合并进来；**所有与 session / workspace / todo / msg-queue / contact 相关的逻辑都不属于引擎**，要么由调用方预渲染成字符串塞进 KV，要么通过 `ValueLoader` trait 注入。
>
> **实现状态**：引擎已落地为 `src/frame/agent_tool/src/prompt_engine.rs`（§2 / §3）。没有通用的 token 预算 / 装配层：各宿主自己把渲染结果拼成 `Vec<AiMessage>`。当前使用方：libopendan 的 behavior system 模板与自定义输入模板（`runner/assembler.rs::render_template`，`__EXEC__` 关闭，命名格式由 `runner/input_view.rs::input_formats()` 注册；没配模板的输入由 `SessionAssembler` 直接拼接，见 §4 末尾），`agent_tool::runtime` 的 `RuntimeValueLoader` / `SandboxPromptExec`（§2 末尾）。
>
> **为什么放进 `agent_tool`，不放进 `llm_context`**：`llm_context` 只跑 loop，进 waist 时 `request.input` 已经是展开好的消息，模板、提示词段落、输入格式都是宿主的事。引擎要接 Runtime 的执行体（`__EXEC__` 走同一 Sandbox）和执行环境信息（`$runtime.*`），和这些能力同住在 Runtime 层；别的 LLM 上层（workflow DSL、oneshot 类调度、文档生成 / 报告渲染……）也可以直接依赖它，只需「填 ValueLoader + 自己装配消息」一层很薄的胶水。

## 1. 设计原则

1. **纯函数式**：输入 = 模板字符串 + KV 上下文 + （可选）外部解析回调；输出 = 渲染后的文本 + 统计。没有任何全局状态、没有 session/workshop 依赖。
2. **小而强类型的扩展点**：四种语法基元 + 一个调用方提供的异步 value loader 闭包，构成全部能力。引擎不感知 agent 语义。
3. **沙箱默认安全**：文件包含必须显式给定 root（不允许逃逸）；shell 执行默认关闭，调用方按需开启。
4. **失败可恢复**：单个指令失败（文件不存在、命令超时、变量未注册）不会让整个渲染崩溃；返回 `RenderStats` 让调用方决定如何处置。
5. **零 agent 词汇**：源码里不应再出现 `Workshop` / `Session` / `Contact` / `Todo` / `MsgQueue` / `Memory` 等词。`OPENDAN_` 这种命名空间前缀也一并清掉。

## 2. 新的 API 形状

```rust
//! src/frame/agent_tool/src/prompt_engine.rs（原 `prompt_render_engine` 改名为 `prompt_engine`）

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
    pub executor: Option<Arc<dyn PromptExec>>,
    /// 以 `/` 开头的 __INCLUDE__ 路径的虚拟根。
    pub include_root: Option<PathBuf>,
    /// 根模板所在目录：相对 __INCLUDE__ 路径以它为基准，嵌套包含以被包含文件的目录为基准。
    pub template_dir: Option<PathBuf>,
    /// __INCLUDE__ 的根目录白名单；空则禁用文件包含。
    pub include_roots: Vec<PathBuf>,
    /// 单次渲染的总递归深度（防止 __INCLUDE__ 循环引用）。默认 8。
    pub max_recursion_depth: u8,
    /// 宿主注册的命名格式（`render_format`）与额外 filter，引擎自己不带宿主词汇。
    pub extensions: RenderExtensions,
}

#[async_trait]
pub trait PromptExec: Send + Sync + std::fmt::Debug {
    async fn exec(&self, request: PromptExecRequest) -> Result<PromptExecOutput, String>;
}
// PromptExecRequest: command, timeout: Duration, max_output_bytes
// PromptExecOutput: exit_code, stdout, stderr, timed_out

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
        loader: &L,
    ) -> Result<RenderResult, RenderError>
    where
        L: ValueLoader + ?Sized;
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

`executor` 默认 `None`。启用 `__EXEC__` 但未注入执行器时返回 `RenderError::Loader`，模板引擎不自行启动本地 shell。`prompt_engine` 只定义执行接口，`agent_tool::runtime::SandboxPromptExec` 把请求交给同一 Sandbox，沿用启动握手、记录与目标侧终止；适配器记录命令与耗时。调用方应在执行前装好 registrar。

`RuntimeValueLoader` 为 `__ENV($runtime.*)__` 提供执行体信息。核心键包括 id、kind、os、arch、hostname、shell、cwd、tools、current_time、timezone；SSH 时在远端探测。Session system 使用稳定字段，新鲜时间放输入批次；xllm 一次性任务可在 system 使用打开时的时间。`__INCLUDE__` 的来源与 include_roots 仍是控制侧模板素材。

## 3. 模板语法（保留 + 重命名）

四个指令 + 一个 `{name}` 占位符。**旧的 `__OPENDAN_*` 前缀全部去掉**（beta2.2 的破坏性 rename，干净利落）。

| 旧形式 | 新形式 | 语义 |
|--------|--------|------|
| `__OPENDAN_ENV(expr)__` | `__ENV(expr)__` | 从 `RenderVars.env` 取值。`expr` 是单个 key（`session_id`）或点路径（`owner.name`）。 |
| `__OPENDAN_CONTENT(path)__` | `__INCLUDE(path)__` | 加载文件。`path` 必须在 `EngineConfig.include_roots` 白名单下；支持 `~` / `$HOME` 展开；硬上限 `max_include_bytes`。 |
| `__OPENDAN_EXEC(cmd)__` | `__EXEC(cmd)__` | 通过注入的 `PromptExec` 执行；仅当 `allow_exec=true` 时启用，超时按 `exec_timeout`，stdout 作为返回值，stderr 仅日志。 |
| `__OPENDAN_VAR(name, $expr)` | `__VAR(name, $expr)__` | 注册一个动态变量：渲染时调用 `loader.load("$expr")` 拿值，绑定到 `vars[name]`，供后续 `{name}` 引用。 |
| `{var}` | `{var}` | 静态/动态变量替换（内部仍走 `upon`）。字面 `{` `}` 用 `{{` `}}` 转义。 |

实现细节：
- 多 pass：先扫一遍所有 `__VAR__` 声明做名字收集与去重；再异步并发解析；最后一遍 substitute + upon。
- `__INCLUDE__` 加载到的文本递归再过一遍引擎（受 `max_recursion_depth` 约束），所以模板片段可以嵌套别的指令。
- 引擎内置 sentinel-based escape pass，保证文件内容里的 `{` `}` 不会被 `upon` 误解析（即原有 `ESCAPED_*_SENTINEL` 机制保留）。
- 所有失败都计数后用 `<!-- $directive failed: $reason -->` 占位（或可配置为空串），**不抛错**。

## 4. 与 old/opendan/src/behavior/prompt.rs 的合并

old prompt.rs 是「behavior 层的 LLM 请求装配」。它干的事拆成两类：

```
①  渲染各段模板（include / env / var / exec）          ← agent_tool::prompt_engine（通用）
②  决定有哪些段、怎么控制长度、怎么装成 Vec<AiMessage>  ← 宿主自己（libopendan SessionAssembler / xllm PromptPlan）
```

**只有 ① 通用化**，住在 `agent_tool`。② 不做通用层：各宿主的段落结构、长度口径和历史来源都不同；请求装不下时 `llm_context` 只报出 `ContextLimitReached` 这个事实，由宿主重写历史。宿主私有的数据（session 历史、Todo、记忆、DID 怎么提取人名……）由宿主预渲染成字符串放进 `RenderVars`，或通过 `ValueLoader` 接入。

### ① `agent_tool::prompt_engine`
模板渲染本身。设计已在 §2 / §3 描述。无 agent 词汇、无 buckyos_api / name_lib 依赖，也不依赖 `llm_context`。

### 宿主侧（不进引擎）
libopendan 的 `SessionAssembler`（`runner/assembler.rs`）决定 system 段和每个输入批次的内容：

- behavior 配置了 system 模板或输入模板（`on_init` / `on_context_switch` / `on_input`，以及半订阅快照模板）时，用 `render_template` 经本引擎渲染：`__EXEC__` 关闭，模板变量是宿主预先算好的 JSON（`input` / `handover` / `todos_text` / `background_text` / `builtin` 等），命名格式由 `input_formats()` 注册。
- 没配模板时由 `DefaultAssembler` 直接拼接，不经过引擎。

xllm 的 system 文本由 `PromptPlan` 按段拼接（宿主 / 组文本 + capabilities + cmd_manual + runtime_protocol），见 [LLM Context 设计](<LLM Context 设计.md>) §11.2。

#### History、Step、动作回响和输入批次的实际来源（当前实现）

这些都不是本引擎的模板变量，Round / Turn 编号也没有作为模板变量暴露：

- **Step 历史与动作回响**：由宿主装配的 `StepRenderer`（`agent_tool::XmlStepRenderer`）在每次推理前从 `LLMContextState.steps` / `last_step` 渲染：其它 behavior 的 Step 进 `<<step_history>>`，当前 behavior 的 Step 渲染成 assistant 决策 + `<<last_step_action_results>>` 动作回响。动作回响总是默认渲染；宿主要补充的内容经 `LLMContext::inject` 并入该 Step 的 user 消息（接在动作回响之后）。
- **Session 历史**：libopendan Runner 从 worklog 渲染成一条 `<session_history>` user message（`runner/history.rs`，`MechanicalCompress`）。
- **每个输入批次的 user message**：libopendan 由 `SessionAssembler::render_input(cfg, state, templates, &InputMaterial)` 渲染成 `<session_input hook=… time=…>`（规则同上：有输入模板走本引擎，否则 `DefaultAssembler` 直接拼接）；system 段由 `SessionAssembler::system_text` 生成。

### workflow DSL 的复用面
workflow 那边只需要：
- 自己实现一个 `WorkflowValueLoader: ValueLoader`，把 job inputs / 上游 node 输出包装成 `loader.load(...)` 的响应。
- 用 `PromptRenderEngine::render` 渲染自己的各段模板，自己装成 `Vec<AiMessage>` 喂进 `LLMContextRequest.input`。

→ workflow 完全不依赖 opendan；与 opendan 共享同一个模板引擎。

## 5. 要从原 prompt_render_engine.rs（即 agent_environment.rs）删掉的

以下条目全部移除（要么搬到宿主的装配层，要么搬到调用 site）：

| 类别 | 删除清单（in `prompt_render_engine.rs`） |
|------|----------------------------------------|
| 结构体 | `AgentEnvironment` / `PromptTemplateContext` / `AgentTemplateRenderResult` / `TemplateRenderMode` |
| Workshop / Session 入口 | `register_workshop_tools*` / `agent_env_root` / `local_workspace_manager` |
| 大型分发函数 | `load_value_from_session`（~470 行的"特殊 key 巨型 match"）→ 这些 key 由宿主预渲染进 `RenderVars` 或由宿主的 ValueLoader 实现 |
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
src/frame/agent_tool/src/
├── prompt_engine.rs       # 纯模板引擎（详 §2 / §3）：PromptRenderEngine / EngineConfig / RenderVars /
│                          #   ValueLoader / PromptExec / RenderExtensions；escape_xml_text / escape_xml_attr
└── runtime/mod.rs         # SandboxPromptExec（__EXEC__ 走同一 Sandbox）、RuntimeValueLoader（$runtime.*）

src/frame/lib_opendan/src/runner/
├── assembler.rs           # SessionAssembler / DefaultAssembler / BehaviorAssembler；render_template
└── input_view.rs          # input_formats()：输入批次用的命名格式
```

调用关系：
```
libopendan::runner::SessionAssembler
      │  behavior 配了 system / 输入模板？
      ├── 是 ──► render_template(template, vars)
      │              │  EngineConfig { extensions: input_formats(), ..default }（__EXEC__ 关闭）
      │              ▼
      │          agent_tool::prompt_engine::PromptRenderEngine::render(template, &RenderVars, &NullValueLoader)
      │              ▼
      │          String ──► system 段 / <session_input> 正文
      └── 否 ──► DefaultAssembler 直接拼接

宿主自己把各段装成 Vec<AiMessage> ──► LLMContextRequest.input
```

依赖单向、闭包清晰：
- `prompt_engine` ← 无依赖（不依赖 `llm_context`，`escape_xml_*` 在 crate 根重导出，`XmlStepRenderer` 也用它们）
- `agent_tool::runtime` 实现 `PromptExec` / `ValueLoader`，把执行体接进引擎
- libopendan / workflow 只依赖引擎的公共 API，**反向无依赖**

## 7. 测试要求

`prompt_engine.rs` 必须有的覆盖（不依赖 agent / session）：
1. 纯 `{var}` 替换 + 转义 `{{` `}}`
2. `__ENV(simple)__` + `__ENV(dotted.path)__` 命中 & 未命中
3. `__INCLUDE(path)__` 命中 / 路径不在白名单 / 文件太大被截断 / 嵌套包含触发递归限制
4. `__EXEC__` 默认禁用时被当作 `not_found` / 启用后正常执行 / 超时被截断 / 失败计入 `exec_failed`
5. `__VAR(name, $expr)__` 通过 loader 解析，后续 `{name}` 引用成功；`$expr` 未注册被计入 `var_failed`
6. 多 pass：`__INCLUDE__` 加载的内容里包含 `__VAR__` / `__ENV__` 也会被处理
7. 输出触顶 `max_total_bytes` 时 `truncated=true`
8. 渲染统计 `RenderStats` 与 `resolved_vars` 数字正确

## 8. 实施 checklist（给 CodeAgent）

按编译可独立通过的顺序。每个阶段独立 `cargo test -p agent_tool` 全绿后再进下一阶段；引擎不依赖 opendan / libopendan，阶段 A 完全可以脱离宿主跑通。

### 阶段 A — agent_tool 内建引擎

1. **`agent_tool::prompt_engine` 框架**：定义 API 类型（`PromptRenderEngine` / `EngineConfig` / `RenderVars` / `RenderResult` / `RenderStats` / `RenderError` / `ValueLoader` / `PromptExec` / `RenderExtensions`），`render` 暂返回 `todo!()`，跑通编译。
2. **搬运 escape/unescape/truncate/path-env 工具函数**：从老 `prompt_render_engine.rs`（即 agent_environment.rs）挑纯字符串 helper 进来，加单测。
3. **实现 `{var}` + `__ENV__` + `__VAR__` 三个指令**：占模板能力 80%，覆盖 §7 的 1/2/5/6/7/8。
4. **实现 `__INCLUDE__`**：含 root 白名单、递归限制、字节上限。
5. **实现 `__EXEC__`（默认关闭）**：含 `allow_exec` gate 和超时；执行经注入的 `PromptExec`。
6. **lib.rs 导出**：`pub mod prompt_engine`，`escape_xml_text` / `escape_xml_attr` 重导出到 crate 根。

### 阶段 B — 宿主接入

7. **Runtime 适配**：`agent_tool::runtime` 实现 `SandboxPromptExec`（`PromptExec`）与 `RuntimeValueLoader`（`ValueLoader`，`$runtime.*`）。
8. **libopendan**：`runner/assembler.rs::render_template` 渲染 behavior 的 system 模板与输入模板，命名格式在 `runner/input_view.rs::input_formats()` 注册；没配模板时 `DefaultAssembler` 直接拼接。
9. **回归**：`cargo test -p agent_tool`、`cargo test -p libopendan`；UI session 跑一遍最小回路确认 prompt 渲染输出符合预期。

### 阶段 C — workflow DSL（后续，不在本次重构范围）

workflow 那边只需复用阶段 A 的引擎 + 自己写 `WorkflowValueLoader` 和 DSL → 各段模板 → `Vec<AiMessage>` 的装配；不涉及 `llm_context` 或引擎内部改动。

## 9. 后续可选

- `prompt_engine` 只依赖 `upon`、`serde_json`、`tokio` 等通用库，不依赖 `llm_context` 和 agent_tool 的其它部分。如果将来 buckyos 其它子系统（docs / report）想用这套，可以把它整体上移到一个更通用的位置（例如新 crate `text_template` 或 `prompt_kit`）——但本次重构**不**做，留好这个迁移空间即可。
- 给 `ValueLoader` 提供一个 `CompositeLoader { loaders: Vec<Arc<dyn ValueLoader>> }` 默认实现，方便多源拼接（session loader + workspace loader + 全局静态 loader）。
- 给 `prompt_engine` 加一个可选的 `RenderTracer` hook，emit 每个指令的命中事件，方便排查模板问题。
