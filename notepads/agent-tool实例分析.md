# buildin agent_tool 开发套路 —— 以 `read_file` / `write_file` 为范本

参考代码：[src/frame/agent_tool/src/file_tools.rs](src/frame/agent_tool/src/file_tools.rs)
关键 trait：[src/frame/agent_tool/src/tool.rs](src/frame/agent_tool/src/tool.rs)（`TypedTool` / `CallingConventions` / `CliInvocation` / `ToolCtx`）

> 注：beta2.2 起内置 tool 全部走 `TypedTool`，`AgentTool` 是由 `TypedToolHandle` 自动桥接出来的旧接口。新写的工具一律实现 `TypedTool`。

---

## 1. 一个内置 tool 的标准骨架

一个内置 tool 在源码里通常包含 5 个东西：

| 项 | 角色 | `read_file` 例 | `write_file` 例 |
|---|---|---|---|
| 工具名常量 | 注册名/调度名，避免散字符串 | `TOOL_READ_FILE` | `TOOL_WRITE_FILE` |
| `Args` 结构体 | LLM/CLI/Bash 三套入参反序列化的目标 | `ReadFileArgs` | `WriteFileArgs` |
| `Output` 结构体 | `execute` 的产物，同时是 LLM 看到的 JSON 结果 | `ReadFileOutput` | `WriteFileOutput` |
| Tool 结构体 + `new` | 持有配置 / 后端依赖（policy、audit…） | `ReadFileTool { cfg }` | `WriteFileTool { cfg, write_audit }` |
| `impl TypedTool` | 元数据 + 三种入口的解析 + 执行 + 摘要 | 见后文 | 见后文 |

### 1.1 `Args` 的写法

```rust
#[derive(Deserialize, JsonSchema)]
pub struct ReadFileArgs {
    pub path: String,
    #[serde(default)]
    pub range: Option<Json>,        // 故意保留 Json，下游自己解析（支持 "10,20" / [10,20] / {start,end} 等）
    #[serde(default)]
    pub first_chunk: Option<String>,
}
```

要点：

- `Deserialize + JsonSchema` 是 `TypedTool::Args` 的硬约束（`schemars` 用来自动生成给 LLM 的 args schema）。
- 可选字段一律 `#[serde(default)] Option<…>`，不要为了"默认值"写 `Default::default`，那会污染 schema。
- 入参里的"灵活 DSL"（如 `range`）保留 `Json`，让 `execute` / `parse_xxx_args` 内部去归一化，**不要**在 schema 阶段就锁死类型。

### 1.2 `Output` 的写法

```rust
#[derive(Serialize, JsonSchema)]
pub struct ReadFileOutput {
    pub content: String,
    pub matched: bool,
    pub line_range: String,
    pub bytes: usize,
    pub line_count: usize,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
    pub preview_truncated: bool,

    // —— 下面是给 build_summary / build_title 用的"内部字段"——
    #[serde(skip)]
    pub file_path: String,
    #[serde(skip)]
    pub preview: String,
    #[serde(skip)]
    pub cmd_line: String,
}
```

惯例：

- 真正给 LLM 的字段照常 `Serialize`。
- 只给 host 渲染 summary/title 用的中间值用 `#[serde(skip)]` 留在结构里，避免污染 LLM 的 JSON。
- 不要在 Output 里塞业务侧的句柄（`Arc<…>`、连接），Output 必须是纯数据。

### 1.3 Tool 结构体

```rust
#[derive(Clone)]
pub struct WriteFileTool {
    cfg: FileToolConfig,                              // 策略（root / policy / size limit）
    write_audit: Arc<dyn FileWriteAuditBackend>,      // 跨切面副作用（审计）
}
```

约束：

- `Clone + Send + Sync + 'static`（`TypedTool: Send + Sync + 'static`）。
- 重的依赖（数据库、tracing sink 等）必须以 `Arc<dyn Backend>` 形式注入，不要在 tool 内部 new。
- 配置（`FileToolConfig`）和后端（`FileWriteAuditBackend`）分开持有：前者是值，后者是 trait object。这样测试时可以塞 `NoopFileWriteAudit`。
- 凡是涉及"在 host 端拿点东西"，**优先**走 `ToolCtx::host()` 的访问器（`host().memory_load()` 等），只有当一个 backend 不属于 `ToolHost` 公共面（比如 file write audit 里现在仍然双轨）时才直接构造时注入。

---

## 2. `impl TypedTool` 的 12 个 hook

`TypedTool` 把"一个工具"拆成 12 块，除了 `name` 和 `execute` 是必填，其他都有合理默认值。下面按"必填 → 元数据 → 三种入口 → 输出渲染"的顺序讲。

### 2.1 必填：`name` + `execute`

```rust
fn name(&self) -> &str { TOOL_READ_FILE }

async fn execute(&self, ctx: &ToolCtx<'_>, args: Self::Args)
    -> Result<Self::Output, AgentToolError> { … }
```

- `name` 用 `&'static str` 常量，注册时 dispatcher 拿这个字符串；不要在这里返回带后缀/配置变体的动态串。
- `execute` 的 `args` 已经是反序列化过的强类型；它只关心"做事 + 返回 Output"。
- 错误用 `AgentToolError`，常用两种：
  - `InvalidArgs(String)` —— 参数/前置条件错误（CLI 里会映射到 usage 退出码）。
  - `ExecFailed(String)` —— 真在执行中炸了（IO、远端等）。
- `ctx.session()` 是 `SessionRuntimeContext`，做 audit / 拿 cwd / 拿 session id 都从这里走。

### 2.2 元数据三件套：`description` / `calling` / `usage`

```rust
fn description(&self) -> &str { "Read file." }
fn calling(&self) -> CallingConventions {
    CallingConventions::from_legacy(true, false, true) // bash + llm，不支持 action
}
fn usage(&self) -> Option<String> {
    Some("read_file <path> [range] [first_chunk]\n\trange: 1-based; …".into())
}
```

- `calling` 是 bitflags（`BASH | ACTION | LLM`），决定该 tool 出现在哪些注册表里：
  - `BASH`：能在 LLM 的 bash shell 里被 `tool_name args…` 这样调到（要实现 `parse_bash_args`）。
  - `ACTION`：作为系统 action 被外部触发（写类工具一般是 `ACTION`，例如 `WriteFileTool` / `EditFileTool` 都只声明 `ACTION`）。
  - `LLM`：作为标准 LLM tool_call 暴露 schema。
  - 默认 `ALL`。`read_file` 是 `BASH | LLM`（不是 ACTION，因为它纯读，没必要走 action 路径）。
- `usage` 仅在 CLI/Bash 入口报错时给 LLM 打印，不影响 schema。

### 2.3 三种入口的 args 解析

`TypedTool` 把"调用"按来源分成三条路径，`Args` 是它们汇合的终点：

```
                ┌─ LLM tool_call ────► (Args 直接由 schema 反序列化)
                │
 调用进来 ──────┼─ Bash one-liner ───► parse_bash_args(tokens) → Json → Args
                │
                └─ CLI argv ─────────► parse_cli_args(tokens)  → CliInvocation
                                                                 ├─ Bash{ line }      → 复用 bash 路径
                                                                 └─ Json{ args, content_input }
```

#### `parse_bash_args`

`read_file` 写得很典型：支持位置参数 `read_file <path> [range] [first_chunk]`，也支持 `key=value`，但不允许混用：

```rust
fn parse_bash_args(&self, tokens: &[String], shell_cwd: Option<&Path>)
    -> Result<Json, AgentToolError>
{
    let mut args = parse_read_file_bash_args(tokens)?;
    if let Some(cwd) = shell_cwd {
        rewrite_read_file_path_with_shell_cwd(&mut args, cwd);
    }
    Ok(args)
}
```

惯例：

- 解析逻辑独立成 `parse_xxx_bash_args(tokens) -> Json` 自由函数（方便单测、方便 `parse_cli_args` 复用）。
- `shell_cwd` 一定要应用到 path 字段，否则 LLM 在 bash 里 cd 之后写相对路径就会拼错。
- 拒绝混用 positional / `key=value` 是有意为之 —— 减少歧义。

#### `parse_cli_args`

CLI 走的是 argv，常见两种风格：

1. **像 read_file**：直接复用 bash 解析，把结果包成 `CliInvocation::Json { args, content_input: None }`。
2. **像 write_file / edit_file**：自己解析 `--mode value --content value | --content-stdin`，因为 content 可能巨大要走 stdin。这就是 `WriteOrEditCliSpec` + `parse_write_or_edit_cli` 的存在意义 —— 两个写类 tool 共享一份 spec：

```rust
pub(crate) struct WriteOrEditCliSpec {
    pub tool_name: &'static str,
    pub extra_required: Option<(&'static str, &'static str)>, // edit_file 多一个 --pos-chunk
    pub content_flag: &'static str,            // --content / --new-content
    pub content_stdin_flag: &'static str,      // --content-stdin / --new-content-stdin
    pub content_field: &'static str,           // 在 Args 里的字段名
}
```

`CliInvocation::Json { content_input: Some((field, ContentInput::Stdin)) }` 表示"args 已经 OK，但 `field` 这个字段的值要由 dispatcher 从 stdin 读"。这套机制让"大文本走 stdin"对 tool 作者是透明的 —— execute 里拿到的就是已经填好的 `String`。

#### `cli_plain_text_stdout`

```rust
fn cli_plain_text_stdout(&self) -> bool { true }   // read_file 才返 true
```

只有 `read_file` 这种纯文本读取在非交互 shell 下需要"剥掉 envelope，直接打 content 出来"，其他工具一律 false。

### 2.4 三个渲染 hook：`build_cmd_line` / `build_summary` / `build_title`

它们决定了**这个 tool 的调用在 host UI / 日志里长什么样**，互不替代：

| hook | 谁用 | 出现时机 | 输入 |
|---|---|---|---|
| `build_cmd_line` | `AgentToolResult.cmd_name` | 调用前/后都有 | `&Args`（注意：是入参！） |
| `build_summary` | `AgentToolResult.summary` | 执行成功后 | `&Output` |
| `build_title` | UI 折叠条上的标题 | 执行成功后 | `&Output` |

约定：

- `build_cmd_line` 输出**人类可读**的"等价 CLI 命令"，要拿 `compact_cmd_param_preview` 把多行/超长字段截短，否则一段 200 行的 `pos_chunk` 直接糊到日志里。
- `build_summary` 是给 LLM 看的、对 Output 的口语化摘要。`read_file` 的 summary 里直接把前 200 行 preview 用 ` ```content ``` ` 围栏贴进去，方便 LLM 不展开 details 就能用；`edit_file` 把 diff 围栏贴进去；写类工具常带"写了 X 字节 / Y 行"。
- `build_title` 一行短句 + 状态尾巴（`success` / `anchor not found` / `no change`），是 UI 折叠时只露一行的那段。

记住一个反复出现的 helper：

```rust
fn build_summary_with_optional_block(base: String, fence: &str, block: &str) -> String {
    if block.trim().is_empty() { base }
    else { format!("{base}\n```{fence}\n{block}\n```") }
}
```

新工具如果 summary 也要"主句 + 可选 fenced block"，直接复用这个模式。

---

## 3. `execute` 的写法套路

把 `WriteFileTool::execute` 拆开看，几乎所有"写类"工具都长这样：

```rust
async fn execute(&self, ctx: &ToolCtx<'_>, args: Self::Args)
    -> Result<Self::Output, AgentToolError>
{
    // 1. 入参归一化 + 必填校验（不要相信 schema，schemars 不强校 trim/empty）
    let file_path = args.path.trim().to_string();
    if file_path.is_empty() { return Err(InvalidArgs("missing required arg `path`".into())); }
    let mode = normalize_write_mode(args.mode.as_deref())?;

    // 2. 路径解析 + policy 检查（root 之外的路径直接拒）
    let abs_path = resolve_path_from_root(&self.cfg.root_dir, &file_path)?;
    self.cfg.ensure_write_path_allowed(&abs_path, &file_path)?;

    // 3. 读旧状态（决定 created/changed）
    let exists = fs::metadata(&abs_path).await.is_ok();
    let original = if exists { read_text_file_lossy(&abs_path).await? } else { String::new() };

    // 4. 计算新状态 + 二次 policy（create / size）
    let updated = …;
    if !exists { self.cfg.ensure_create_allowed(&file_path)?; }
    self.cfg.ensure_write_size_allowed(&file_path, updated.len())?;

    // 5. 真正落盘（mkdir -p 父目录，然后 write）
    if let Some(parent) = abs_path.parent() { fs::create_dir_all(parent).await?; }
    fs::write(&abs_path, updated.as_bytes()).await?;

    // 6. 副作用：审计（失败仅 warn，不冒泡）
    if let Err(err) = self.write_audit.record_file_write(ctx.session(), &audit_args, &record).await {
        warn!("file_tool.write_file_audit_failed: …");
    }

    // 7. 装配 Output（含给 summary/title 用的 #[serde(skip)] 字段）
    Ok(WriteFileOutput { … })
}
```

几条不那么显然的规矩：

- **policy 早 → 落盘晚**：path 校验放最前；`ensure_create_allowed` / `ensure_write_size_allowed` 放在已经知道"会写、会写多大"之后。
- **审计失败不冒泡**：`record_file_write` 的错误用 `warn!` 打日志、不返回 `Err` —— 否则审计后端临时挂了会让真实的 IO 操作"看起来失败但其实已落盘"。这是一个被反复确认过的取舍。
- **created / changed 都算**：写类 tool 的 Output 必须能让上层判断"有没有真的变"（`changed`）和"是不是新建"（`created`）。`build_simple_diff` 即使没改也要算，方便审计。
- **`read_text_file_lossy`**：所有读文本都过 `String::from_utf8_lossy`，不要为了"严格"用 `from_utf8` —— 工具是给 LLM 用的，遇到坏字节就直接 ✨ 替换好过整体失败。

读类工具（`read_file::execute`）的骨架更短，但思路相同：

```
1. 校验 path → 2. policy 检查 (read root) → 3. 读全文
4. 按 first_chunk 锚定起点（不命中就 matched=false 返空）
5. 按 range 切片（parse_line_range 已把 1-based / 负数 / $ / +N 全部归一）
6. 装 preview（最多 200 行，超出标 ... truncated）+ 装 Output
```

`read_file` 没有写类的 audit/落盘步骤，但有自己的"切片 DSL"—— 这种"参数语义复杂"的工具，把 DSL parser 写成纯函数 + 表驱动（`LineRangeSpec` / `RangeDefaultEnd`）+ 单测覆盖（文件末尾那批 `parse_line_range_*` 测试），是这套代码里反复出现的范式。

---

## 4. 注册与暴露

写完 tool 之后，host 侧的注册套路：

```rust
mgr.register_typed_tool(ReadFileTool::new(cfg.clone()))?;
mgr.register_typed_tool(WriteFileTool::new(cfg.clone(), audit.clone()))?;
mgr.register_typed_tool(EditFileTool::new(cfg, audit))?;
```

`AgentToolManager::register_typed_tool<T: TypedTool>` 内部会用 `TypedToolHandle::new(tool)` 把它包成 `Arc<dyn AgentTool>`，然后按 `calling()` 的位拆到 `bash_tools` / `action_tools` / `llm_tools` 三张表。所以**作者只管实现 `TypedTool`，永远不要直接去 `impl AgentTool`**。

---

## 5. 写一个新的 buildin tool 的最小 checklist

按这个顺序对着抄就基本不会漏：

1. **常量** `pub const TOOL_FOO: &str = "foo";`
2. **`FooArgs`**（`Deserialize + JsonSchema`，可选字段 `Option<…> + #[serde(default)]`，复杂 DSL 留 `Json`）。
3. **`FooOutput`**（`Serialize + JsonSchema`，UI/summary 用的中间字段用 `#[serde(skip)]`）。
4. **`FooTool`**（持有 `FooConfig` + `Arc<dyn Backend>`；`Clone + Send + Sync`）。
5. **`impl TypedTool for FooTool`**：
   - `name` 返回常量；`description` 一两句话即可。
   - `calling()` 想清楚：是给 LLM 用？给 bash 用？是 action？默认 `ALL` 几乎一定不对。
   - 如果有 bash 入口：写 `parse_bash_args`，单独一个自由函数，别塞在 trait 里。
   - 如果有 CLI 入口（用户在 shell 里直接敲）且参数复杂/有大文本：写 `parse_cli_args` 返 `CliInvocation::Json`，大文本字段用 `ContentInput::Stdin`。否则可以不重写。
   - `build_cmd_line` / `build_summary` / `build_title` 都建议实现，否则 UI 一片"ok"。
   - `execute`：参数归一 → policy → 读旧态 → 计算新态 → 副作用（写盘/调用后端）→ audit（warn-only）→ 装 Output。
6. **测试**：把所有 parser / DSL 写成纯函数，在文件尾巴的 `mod tests` 里铺 case（参考 `parse_line_range_*` 那套）。
7. **注册**：在 host 启动处 `mgr.register_typed_tool(FooTool::new(...))`。

---

## 6. 容易踩的坑

- **不要在 schema 里把灵活字段类型化死**。`range` 用 `Option<Json>` 而不是 `RangeSpec`，是因为既要兼容 LLM 的 `"10-20"`，又要兼容人的 `[10, 20]`，还要兼容 `{ start, count }`。先收 `Json`，到 `parse_line_range_spec` 里统一归一。
- **不要混用 positional 和 key=value**。`parse_read_file_bash_args` 里显式拒绝混用，不然 `read_file path=foo bar` 这种东西没法稳定解释。
- **不要把 audit 的失败冒泡成 tool 失败**。warn 即可；写盘已经成功就当成功。
- **CLI 大文本走 stdin**。不要让 LLM 把一兆字节往 argv 里塞，`ContentInput::Stdin` + dispatcher 自动从 stdin 读才是正解。
- **`build_cmd_line` 必须截断**。多行/长字段用 `compact_cmd_param_preview`，给一个 `(total N lines)` 的提示就行，原文进 details。
- **Output 里的派生字段用 `#[serde(skip)]`**。给人看的（`cmd_line`、`preview`、`file_path` 这种 host 端要的）一律不进 LLM JSON。
- **`ToolCtx::session()` 才是 audit / cwd / session id 的来源**，不要在 tool 里自己存 session。
- **beta2.2 是 breaking change 窗口**。这套 `TypedTool` / `CliInvocation` 接口还在收尾，新加 hook / 改字段不需要保留旧路径，但要把已有的内置 tool 一起改干净。
