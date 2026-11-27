# OpenDAN 提示词机械构造整改 TODO（工具结果渲染、模板里的对象）

> 状态：待实施。方向由 owner 于 2026-11-26 确定（§1），同日 owner 裁决了 Q1–Q9（§10），裁决已写进各条目，可以交给 CodeAgent 实施。Q1 有 owner 的补充；Q6、Q7 按起草人的理解实施，owner 已确认。现状核实于 2026-11-26，基线是 buckyos `f19441731`（工作区干净），下文行号以它为准，实施时请重新定位。
>
> 范围：工具结果怎样进入 prompt（长度、分级、提示、去重），以及模板和内置输入块里的"对象"怎样呈现。
>
> 不在范围：behavior / skill 的提示词措辞、`llm_compress` 的摘要算法、AICC。

下文路径缩写：

| 缩写 | 目录 |
| --- | --- |
| `at/` | `src/frame/agent_tool/src/` |
| `lo/` | `src/frame/lib_opendan/src/` |
| `lc/` | `src/frame/llm_context/src/` |
| `obj/` | `src/frame/agent-did-object-lib/src/` |
| `od/` | `src/frame/opendan/src/` |
| `jv/` | `src/apps/jarvis_agent/agent/` |
| `ws/` | `buckyos-websdk/`（另一个仓库） |

## 1. 方向（owner，2026-11-26）

1. **AgentTool 的返回要正确地渲染进提示词。** 一是长度合适；二是用 hint 做渐进式披露；三是同一 session 里重复出现的相同内容（典型是重复 Read 同一内容）只给一句提示，不再给全文。
2. **模板引入的"对象"不插完整 JSON。** 按上下文需要嵌入人可读的文本，通常两行就够。

整改时还要遵守 owner 的"机制越少越好"原则：优先复用已有字段（`title` / `summary` / `hints` / `dedup`）和已有的时机（新 context、rewrite），不新增持久账本或宿主硬机制。

## 2. 实施须知（给执行的 CodeAgent）

**主路径与次要路径**

- Jarvis 内置 behavior 全部是 `mode = "function_call"`（`jv/behaviors/*.toml`），这是主路径。
- behavior 模式（`XmlStepRenderer`）现在只有第三方 behavior 和 xllm 的 behavior 模式会用到，相关条目优先级较低。

**有副本、要一起改的地方**

- **TS 端口**：`ws/src/xllm/runtime/hints.ts`、`tool_result.ts`、`step_renderer.ts` 是 Rust 实现的移植。
  - 共享向量在 `src/frame/llm_context/tests/vectors/`。
  - 重写向量：`XLLM_VECTORS_REGEN=1 cargo test -p agent_tool --lib xllm::vectors`。看过 diff 后复制到 `ws/tests/fixtures/xllm/`。
- **协议 fixtures**：在 `doc/opendan/protocol/fixtures/` 下。
  - 重新生成：`cargo run -p libopendan --example fixtures -- <out_dir>`，然后逐个场景审 diff。
  - `14_input_bus/rendering/` 逐字节固定了内置输入格式，`<runtime>` 的 JSON 也出现在多个 snapshot 里。
- **历史渲染器版本**：`HISTORY_RENDERER = "libopendan.history/4"`（`lo/protocol/summary.rs:14`）。只要改了从 worklog 重建历史的形态，就升到 `/5`。
- **不做向下兼容**（Q8，与 `AGENTS.md` 的默认规则一致）：worklog、run.json、协议 JSON 的格式变化都不为旧数据写回退或迁移代码，fixtures 和向量重新生成即可。

**测试入口**

- 单元测试：
  - `cargo test -p agent_tool`：覆盖 step_record / hints / read_tool / file_tools / prompt_engine 的单测和向量。
  - `cargo test -p libopendan`：覆盖 history、assembler、fixtures。
  - `cargo test -p agent-did-object-lib`、`cargo test -p opendan`。
  - websdk 的测试。
- 真实模型回放：`src/frame/lib_opendan/tests/{plan,goal,memory}_replay.rs`，用 `--ignored` 运行。

## 3. 字段语义与落盘（A，先做：分级和去重都依赖它）

### A1 typed tool 把正文塞进了 `summary`

**整改前**

- `finalize_typed_result`（`at/tool.rs:403`）只产出两样东西：`summary`（来自 `build_summary`）和 `detail`（整个 Output 结构体）。它不填 `output`。
- 各工具把正文当代码块拼进 `summary`：
  - read_file 拼 200 行预览（`at/file_tools.rs:765`）。
  - edit_file 拼 diff（`at/file_tools.rs:204`）。
  - glob 拼文件列表（`at/glob_tool.rs:119`）。
  - grep 拼匹配内容（`at/grep_tool.rs:174`）。
- 结果是 `summary` 实际上就是 Full 正文。协议里的 Medium 视图并不存在，所有"降到 summary"的做法都省不了长度。

**整改目标**

- `TypedTool` 增加 `build_output(&Output) -> Option<String>`，默认返回 `None`。预览、diff、列表、匹配等正文都放进 `output`。
- `summary` 只写几行能独立读懂的话：做了什么、结果如何、关键数字、是否截断。不带代码块。
- `title` 不变。
- 协议文档 §5 加一条规则：`summary` 不得包含正文块。
 
**验收**

- 加一个遍历全部内置 typed tool 的单测：`summary` 不含 ```` ``` ```` 块，且不超过 3 行。
- function_call 下模型看到的文本与改前等价，因为正文现在由 `output` 承载。

### A2 两种 loop 选正文的规则不一致

**整改前**

- function_call 用 `AgentToolResult::observation_text`（`at/lib.rs:435`），取值顺序是 `output` → `summary` → `details` 的紧凑 JSON（`at/lib.rs:443`）。
- behavior 模式的 Full 用 `tool_result_full_body`（`at/step_record.rs:586`），顺序是 `output` → 字符串 `detail` → `detail` 的 pretty JSON（`at/step_record.rs:600`）→ `summary` → `title`。
- 结果：同一个 typed tool 结果，function_call 下模型看到 `summary`，behavior 模式下看到整个 Output 结构体的 pretty JSON，里面有 `content` 全文、`cyfs_url`、`preview` 等全部字段。

**整改目标**

- 由一个函数决定 Full 正文，建议放在 `AgentToolResult` / `ToolResultView` 上，两种 loop 都调用它。
- 取值顺序：`output` → 字符串 `detail` → `summary` → `title`。
- 对象或数组形式的 `detail` 只在以上都为空时才用，用紧凑 JSON，并受 B1 的单结果预算约束。
- TS 端同步修改。

**验收**

- 新增单测：同一结果在两种 loop 下正文一致。
- 改写 `protocol_tool_result_full_renders_structured_detail_as_json` 用例。

### A3 worklog 只存了工具结果的最终文本

**整改前**

- `lo/runner/flush.rs:146` 和 `:237` 写入 `WorklogBody::ActionResult { result: text }`。`text` 是模型当时看到的文本，可能带 `<output>` / `<runtime_hints>` 包装，也可能就是 dedup 那一句。
- 信封里的 `title`、`summary`、dedup key、显示过的 once key 都没有落盘。
- 后果：跨 run 重建历史时（`lo/runner/history.rs:422`）只能原样回放，既做不了 C1 的按位置降级，也恢复不了 D1 / D4 的账本。

**整改目标**

- `ActionResult` 增加以下可选字段：`title`、`summary`、`dedup_key`、`hint_once_keys`。
- 结果被 dedup 替换时，加 `dedup_of: <call_id>`。
- 这些字段的来源是 `Observation.host` 里的信封（`ToolResultView::from_host`）。
- 不兼容旧 worklog（Q8）：按新格式写入和读取，不为缺字段的旧条目写回退。
- 历史渲染器升到 `/5`。

**验收**

- 写入和读回的单测。
- 协议 fixtures 按新格式重新生成。

## 4. 长度与"怎么继续"（B）

### B1 单个结果没有统一上限

**整改前**

- **shell**：`EXEC_MAX_OUTPUT_BYTES = 64 KiB`（`at/xllm.rs:213`）。stdout 和 stderr 各截各的（`output_from_exec_dir`，`at/llm_bash.rs:722`），一个结果合起来最多约 128 KiB。
- **read_file**：
  - 不给 range 时整个文件都读入。
  - 预览只按 200 行截（`READ_FILE_PREVIEW_MAX_LINES`，`at/file_tools.rs:23`），没有字节上限，也没有单行长度上限。
  - 压缩过的单行文件或超长 JSON 行会整个进来。
- **read 对象 URL**：
  - `read_url_input` 传的是 `max_tokens: None`（`obj/tool.rs:242`）。
  - `read_token_limit`（20K token，`at/lib.rs:142`）只在对象库自己的工具里生效。
- **派发器与 llm_context**：都不再截断（`at/xllm.rs:4587`、`lc/context_loop.rs:2470`）。

**整改目标**

- 单结果预算是 **16K token**（Q3）。
  - 在派发器里统一兜底，位置在 `map_result_to_observation_with_hints` 之前。各工具自己的上限可以更小。
  - token 数用 llm_context deps 的 tokenizer 估算。默认的 `ByteHeuristicTokenizer` 按 4 字节一个 token（`lc/deps.rs:439`），16K token 约等于 64 KiB。
- 超出预算时保留头尾、标明省略了多少，并按 B2 给出继续的办法。
- read_file 预览同时按行数和字节数截。单行超长时，截断那一行本身。
- `read_token_limit` 并入这个预算：`DEFAULT_READ_TOKEN_LIMIT` 改为 16K，作为可配置的上限，和派发器兜底用同一个值。

**验收**

- 三个用例：超大 stdout、1 MB 的单行文件、大对象属性。三者模型可见的文本都不超过预算，并且带有继续的办法。

### B2 截断提示不告诉模型怎么继续

**整改前**

- **渲染器**：`XmlStepRenderer` 的 `clip` 只留开头，加上 `[truncated]`（`at/step_record.rs:862`、`:674`）。
- **shell**：只写 `…[N bytes omitted]…`（`at/llm_bash.rs:695`）。全文其实在执行目录里（native 下是 `runs/<run_id>/exec/<call_id>/stdout`，见 `exec_dir_for`，`at/llm_bash.rs:95`），但没给路径。
- **read_file**：
  - 只写 `... truncated to 200 lines`（`at/file_tools.rs:1398`）。
  - `summary` 同时写 "read N bytes across M lines at 1-M"，模型会以为自己看到了 M 行。
  - 不给文件总行数，也不给下一段的 range。
- **glob**：只写 `, truncated`（`at/glob_tool.rs:124`）。
- **grep**：只回显 limit / offset（`at/grep_tool.rs:637`），不给下一页的 offset。

**整改目标**

每个截断标记都写清两件事：还剩多少，下一步具体怎么读。写在 `output` 里，因为它是这次结果的一部分，不走 hints。

| 来源 | 提示的写法 |
| --- | --- |
| shell | `…[N bytes omitted; full stdout: <path>]…`。remote 写远端路径；没有执行目录时不写路径 |
| read_file | `summary` 改为 "shown lines a–b of T"。预览被截时写 "lines b+1–T not shown: read with range=b+1-…" |
| glob / grep | 写出总数和下一页的参数 |
| 渲染器 clip | A2 / B1 做完后不应再二次截断（见 D3）。如果仍然截，写明省略了多少 |

**验收**

- 每种截断各一个用例，断言提示里有可以直接照做的参数。

### B3 hints 的渐进式披露基本没用起来（可选）

**整改前**

- 带 `hints` 的结果只有一处：shell 转 task 时的跟进说明（`at/llm_bash.rs:1595`）。

**整改目标**

- 通用用法用 once hint，在第一次截断时说一次。例如分页、range 写法、读对象时的 property 参数。key 用 `read.paging`、`grep.paging`、`glob.paging`。
- B2 里的具体参数仍然写在 `output` 里。
- 不新增机制。

**验收**

- 同一 session 第二次截断时不再出现这条 hint。压缩后会再次出现，见 D4。

## 5. 按位置降级（C）

### C1 function_call 模式没有按位置降级

**整改前**

- run 内的旧工具结果原样留在 `accumulated` 里。只有图片会按数量降级（`omit_older_tool_images`，`lc/context_loop.rs:2528`）。
- 跨 run 时，`lo/runner/history.rs` 从 worklog 重建历史，"nothing truncated"。
- 只有超出预算才处理：走 `llm_compress` 做摘要，或走 `trim_tool_results` 做裁剪（`at/llm_compress.rs:381`）。
- 协议文档 §1 写的三档（hot tail 用 Full、compact 用 summary、digest 用 title）没有实现。

**整改目标**（降级时机按 Q1）

- **两个降级时机：**
  - **压缩时**（`rewrite_for_limit`、`maybe_compact`）：压缩本身一定会让缓存失效，所以总是降级，不看时间。先降级；降级后仍然超出预算，再做摘要。
  - **新 run 从 worklog 重建历史时**：按时间判断，只在缓存大概率已经失效时降级。
    - 判断复用现有的 `RunnerOptions.prompt_cache_idle`（`lo/runner/mod.rs:204`，默认 1 小时）。
    - 算法与 `refresh_frozen` 相同：取上一个 run 记录的 `updated_at_ms`，到现在的间隔不小于 `prompt_cache_idle`，就认为缓存已不在（`lo/runner/drive.rs:648-652`）。把它抽成一个共用函数，`refresh_frozen` 和这里都调用。
    - xllm 用同一规则，时间取它自己记录的上一次推理时间。拿不到时间就不降级。
- run 内永远不改写已经发给模型的消息（S-20）。
- 降级时按距离分三档：
  - 最近 K 个 Round（或最近 X token）用 Full。
  - 再往前用 `summary`。A1 做完后它才真的变短。
  - 更早的用 `title`。
- 降级后的结果末尾加一句怎么找回原文。现成的命令是 `agent-session read-session <sid> --turn <n>`（`lo/runner/history.rs:52`）。
- 依赖 A1、A3。

**验收**

- 构造一个 30 个 Round 的 worklog：
  - 上一个 run 的时间在 `prompt_cache_idle` 以内时重建，结果与现在相同，不降级。
  - 超过 `prompt_cache_idle` 后重建，各档形态正确，总 token 下降。
  - 触发压缩时，不论上一个 run 是什么时间都会降级。降级后能装下就不调用摘要模型。
- 同一份 worklog 在同样条件下重建两次，结果逐字节相同。

### C2 behavior 模式的分级参数是摆设

**整改前**

- `render_history` 对当前 behavior 的每个 step 都调 `render_full`（`at/step_record.rs:156`）。
- `StepMeta.compression_level` 只在 `lc/context_loop.rs:2341` 取默认值 `Full`，之后从来不改。渲染器只是把它打印成属性（`at/step_record.rs:827`），所以继承记录明明是 compact 形态，属性却写着 `compression="full"`。
- `summary_chars`（`at/step_record.rs:43`）没有任何代码读取。
- `inject` 之后，`next_user_message` 被冻结成 Full 文本（`lc/context_loop.rs:272`）。

**整改目标**（Q2 已定）

- 删掉 `compression_level` 和 `summary_chars`。
- behavior 模式的降级与 C1 用同一规则，即在 rewrite 或新 context 时降级。
- 去掉继承记录上那个错误的 `compression` 属性。
- 文档同步修改。

**验收**

- 删除字段后编译通过，fixtures 和向量已更新。

## 6. 相同内容去重（D：同一 session 里重复 Read）

### D1 去重范围是 run，不是 session

**整改前**

- `SessionHintLedger` 的 dedup 集合只存在内存里（`lo/runner/hints.rs:14`）。
  - 新 run context 时清空（`lo/runner/live.rs:263`）。
  - rewrite 时清空（`lo/runner/live.rs:1197`）。
  - xllm 的做法相同（`at/xllm.rs:4204`）。
- 跨 run 时历史从 worklog 重建，上一次读到的原文仍在 prompt 里，但再读同一文件仍然给出全文。
- 进程重启后恢复同一个 run，账本也会丢。

**整改目标**

- 账本只记"当前上下文里还看得见的结果"，不新增持久账本。
- 每次构造 context 时（新 run 重建历史、rewrite 之后、从快照恢复时），用 A3 落盘的 `dedup_key` 重新登记那些以 Full 形态出现在新历史里的结果。
- 被压缩、被 C1 降级、被裁剪掉的结果不登记。
- 提示文案见 D4。

**验收**

- 同一 session 两个 Turn 读同一文件：第二次只给一句话。
- 压缩后再读：给全文。
- 进程重启并恢复后：表现与上面相同。

### D2 key 用的是模型写的原始路径

**整改前**

- `with_read_dedup`（`at/read_tool.rs:427`）的 key 是 `read:{what}#{window}:{version}:{digest}`，其中 `what` 是参数原文。`DedupReadFile`（`at/read_tool.rs:441`）也一样。
- 于是 `a.rs`、`./a.rs`、绝对路径、`file://…` 各算各的。

**整改目标**

- `what` 改用解析后的规范形式：
  - 文件用 FileBackend resolve 之后的绝对路径（用 `cyfs_url` 也可以）。
  - 对象 URL 用结果里的 canonical object（`details.object.object`，即 adapter 的 `cache_key`）。
- 对象读取的窗口里加上 `property`。现在窗口只有 `range`，靠内容摘要才区分不同属性。
- 范围（Q4 已定）：只覆盖 read / read_file 以及按 D5 返回 `dedup` 的工具；只有窗口完全相同才算重复；shell 的 `cat` / `sed` 不纳入。

**验收**

- 四种写法读同一文件的同一窗口，只出一次全文。

### D3 结果被截断展示后，dedup 仍然让模型"用上次的内容"

**整改前**

- 派发器先登记 key（`at/hints.rs:117`），behavior 渲染器之后又把正文截到 4096 字符（`at/step_record.rs:58`）。
- 再读同一窗口时，回答是 "unchanged … use that content"，可模型其实只看到了前 4 KB。

**整改目标**

- 只有完整展示过的结果才登记。
- B1 之后由派发器保证单结果不超预算，渲染器的 `max_result_chars` 默认取同一预算，不再二次截断。
- 如果宿主配了更小的值，被截断的结果不登记（或撤销登记）。

**验收**

- 渲染器上限小于结果长度时，第二次读给出全文。

### D4 once hint 在压缩后不再出现；dedup 文案不指出是哪一次

**整改前**

- once 账本按 session 持久保存（`SessionState.hints_once`，`lo/runner/hints.rs:29`），`clear_dedup` 只清 dedup（`lo/runner/hints.rs:37`）。
- 历史压缩之后，带这条 hint 的结果已经不在 prompt 里了，hint 却再也不会出现。
- dedup 文案是 "unchanged since you read it earlier in this run"（`at/read_tool.rs:436`），不说明是哪一次调用。D1 做完后它可能跨 run 出现，"in this run" 就不对了。

**整改目标**

- once 采用与 D1 相同的规则：构造 context 时，按新历史里实际还在的 hint 重建 once 集合（用 A3 的 `hint_once_keys`）。
- 最低限度的做法：rewrite 时清空 once。多显示一次没有害处。
- dedup 文案改为指向那次调用，例如 "same as the result of call <call_id> above; unchanged"。

**验收**

- 压缩后第一次 shell 转 task 时，跟进说明再次出现。

### D5 对象库自己的"未变化"机制改为返回 `dedup`，内置 did-object 的读取都由 runtime 自动去重

**整改前**

- `obj/runtime.rs:327` 的 `read_response_to_tool_result` 调用 `detect_and_update_read_state`（`obj/runtime.rs:441`）。
- 状态存在系统临时目录（`obj/runtime.rs:504`），按 session 记录，不考虑是否可见。
- 文案是中文 "和上一次read相比没有变化"（`obj/runtime.rs:24`）。
- 只有 `AgentDIDObjectReadTool` / `AgentDIDObjectRuntime::read` 走这条路。OpenDAN（`od/urls.rs`）和 `agent_tool_cli_dev` 用的是 `ReadTool` + `ObjectUrlReader`，都不经过它。

**已有的基础**（核实于基线）

- `dedup` 和 `hints` 会随协议 JSON 序列化（`at/lib.rs:285`、`:290`）。
- shell 把内层命令输出的 AgentToolResult 原样转发（`try_forward_inner_agent_tool_result`，`at/llm_bash.rs:1633`）。
- 所以任何工具或 CLI，只要返回带 `dedup` 的结果，都会被派发器的 `apply_hints` 自动去重，不需要自己维护状态。

**整改目标**（Q4）

- 删掉 `detect_and_update_read_state` 和临时目录里的状态。`read_response_to_tool_result` 改为在结果上设置 `dedup`，规则与 D2 相同：
  - key 由 canonical object（`cache_key`）、`version`、窗口（range、property、content_only）和内容摘要组成。
  - 文案按 D4。
- 这样 `AgentDIDObjectReadTool`、经 shell 调用的对象读取 CLI、ReadTool 三条路径的去重行为一致。
- 内置 did-object 对象的读取都"顺手支持"：逐个检查内置 adapter 和 provider 是否填了 `cache_key` / `version`。目前 `obj/adapters/cyfs.rs:146` 和 `obj/adapters/mod.rs:121` 的 `version` 是 `None`，只能靠内容摘要判断。能给出版本的就填上。

**验收**

- 对象库的测试已更新，仓库里不再有 `READ_UNCHANGED_MESSAGE`。
- 同一对象读两次：经 ReadTool 和经 shell 调用 CLI，第二次都只有一句话。

## 7. 模板里的对象（E，与 A–D 没有依赖，可以并行）

### E1 引擎遇到对象时：要么输出 JSON，要么报错

**整改前**

- `__ENV($x)__` 对 object / array 输出紧凑 JSON（`json_value_to_compact_text`，`at/prompt_engine.rs:1098`）。
- `xml`、`attr`、`truncate`、`oneline`、`quote` 等 filter 拿到对象时也输出紧凑 JSON（`upon_text`，`at/prompt_engine.rs:217`）。
- `{{ x }}` 遇到 map 或 list 时，upon 的默认 formatter 报 "unformattable type"（upon-0.10.0 `src/fmt.rs:223`），引擎没有设自己的默认 formatter。
  - 报错后 `render_template` 失败，这批输入保持未消费（`lo/runner/assembler.rs:111` 的注释）。
  - 这与引擎设计原则 4 "失败可恢复"（`doc/llm_context/prompt_render_engine.md` §1）不符。
- 对象层面没有展示约定。`render_format` 只注册了输入总线上的几种对象（`lo/runner/input_view.rs:690`）。

**整改目标**

- **用对象自己的字段**（Q6）：不给对象统一加 `title` / `summary` 之类的新字段。两行可读文本从对象已有的字段里取，并沿用这些字段原来的名字。例如：
  - session 取 objective 和 status。
  - goal 取 title 和 state。
  - workspace 取 name。
  - FileObject 取 name、size 和 mime。
  - 消息取 from 和 text。
- **已知类型**：每类对象的取法写在它自己的格式函数里，即宿主注册的 `render_format`，或 E5 中 provider 生成的行。
- **引擎的默认 formatter**：给 map 和 list 设一个默认 formatter，"尽力"从对象自有的常见字段里拼出最多两行：
  - 第一行按顺序取 `title`、`name`、`subject` 中第一个有值的。
  - 第二行按顺序取 `summary`、`description`、`status` 中第一个有值的。
  - 数组每个元素各渲染一行。
- **取不到可读字段时**（Q7）：回退成紧凑 JSON，受 B1 预算约束；同时 `log::warn!` 记下模板和变量路径，并计入 `RenderStats`。不报错，这批输入照常消费。
- `__ENV` 和各 filter 走同一套规则。明确要 JSON 时写 `| json`，不记日志。

**验收**

- prompt_engine 单测覆盖五种情况：
  - 有常见字段的对象。
  - 没有常见字段的对象：回退 JSON，有日志，有计数。
  - 数组。
  - `| json`。
  - 已注册格式的对象。
- 模板里写 `{{ handover.process_result }}` 不再卡住输入。

### E2 宿主喂给模板的变量是原始 Value

**整改前**

`render_input` 的 vars（`lo/runner/assembler.rs:425`）里有三项是原始数据：

- `runtime.status`：RuntimeInfo、workspace、runtimes 整体。
- `handover.process_result.submission`（`:417`）。
- `background`：`BackgroundLine` 数组，每项有 `text` 字段。

**整改目标**

- 每个对象变量要么给出符合 E1 约定的可读视图，要么给出现成的 `_text` 文本。已有的 `runtime_text`、`handover_text`、`background_context` 直接复用。
- 原始数据只在确实需要时，以显式的名字保留，例如 `runtime.raw`。
- 模板文档列出每个变量是什么、默认渲染成什么。

**验收**

- 更新 `14_input_bus/rendering/vars.json`。
- 内置模板的输出不变，或按 E3 改变。

### E3 `<runtime>` 块是整段 JSON

**整改前**

- `Blocks::of` 输出 `<runtime>{status}</runtime>`（`lo/runner/assembler.rs:563`）。
  - `status` 是 RuntimeInfo、workspace、runtimes 的紧凑 JSON（`lo/runner/drive.rs:1911`）。
  - 里面有 arch、hostname、shell 等，还有 `current_time`，与 `<session_input time>` 重复。
- plan 视图的问题（Jarvis 在 `jv/agent.toml:93` 配了 `runtime_view = "all"`）：
  - 只有 `lines` 被取出来做成 `<runtimes>`。
  - `all` 数组仍留在 JSON 里，每个 runtime 有十几个字段（`runtime_entry`，`lo/runtime/mod.rs:664`），同一信息出现两遍（`lo/runtime/mod.rs:804-805`）。
- 每个使用内置模板的批次都带这一块：
  - work / plan 的 on_init（它们的自定义模板只覆盖了 on_input）。
  - 没配模板的 behavior（例如 goal）的每一批。
- 协议 fixtures 里固定了这段 JSON，例如 `16_goal_session/.../snapshots/0001.json`。
- 现成的正例：UI 会话的背景上下文用一句话一行写同样的信息（`lo/runner/background.rs:115-206`），例如：
  - "your private runtime `x` on host, cwd …, shell …"
  - "owner's default runtime …"
  - "workspace … at … on runtime …"

**整改目标**

- `<runtime>` 改成可读的行，复用 `background.rs` 的句式（抽成共用函数）：
  - 工作 runtime 一行。
  - 私有 runtime 一行，与工作 runtime 相同时省略。
  - workspace 一行。
- 去掉 `current_time`。
- plan 的 `<runtimes>` 保留那些行，删掉 JSON 里的 `all`。

**验收**

- `14_input_bus/rendering/on_input_builtin.txt` 等 fixtures 重新生成，并审过 diff。
- token 数下降。

### E4 `<process_result>` 里的 submission

**整改前**

- `lo/runner/assembler.rs:489` 把子 behavior 交回的结构化结果（submission）写成紧凑 JSON。

**整改目标**（Q5 已定）

- 保留 JSON：这是调用方要的数据，不是"引用一个对象"。
- 但它要受 B1 的单结果预算约束，超出时给出读取全文的路径。

### E5 读对象时的属性和集合

**整改前**

- 显式读属性时，值不是字符串就输出 pretty JSON（`obj/adapters/did_object.rs:407`）。
- 对象文档的 Summary 段，每个 summary 属性都是 pretty JSON（`obj/adapters/did_object.rs:671`）。
- Agent 的四个集合（sessions / workspaces / goals / artifacts），每行的 `cells` 是整条 RPC 记录（`od/obj_provider.rs:32`）。所以 `read <agent>/sessions` 会把每个 session 的全部字段都倒出来。
- 已经符合方向的正例：
  - system 里的 `## Objects` 一行一个对象（`lo/runner/assembler.rs:250`）。
  - 对象文档的 Properties / Actions / Events 段是可读的行。

**整改目标**

- 集合行的内容由 provider 按集合类型决定（Q9），并沿用对象自己的字段名（Q6）。
  - 每个集合声明列表里显示哪几个字段，例如 sessions 显示 `kind`、`status`、`objective`。
  - `cells` 只带这几个字段，不再带整条记录。
- adapter 把每行渲染成一行：`- <object url> — kind: ui · status: running · objective: …`。要看全部字段，就 read 那个对象。
- Summary 段的属性值按 E1 的规则：能取到可读字段就用，否则回退紧凑 JSON，受预算约束。
- 显式读属性时保持数据形式（JSON），但改用紧凑形式，并受 B1 的预算约束。

**验收**

- 有 10 个 session 时，`read <agent>/sessions` 每个 session 只占一行。
- 相关测试已更新（`src/frame/opendan/tests/owner_alias_read.rs` 等）。

## 8. 文档（F，随每一步一起改）

- **`doc/llm_context/agent_tool/agent_tool_result_protocol.md`**，需要改写或补充：
  - 新增"`summary` 不含正文"的规则。
  - 把"StepRecord 渲染规则"改写成实际实现。
  - hints / dedup 的范围改为 session 级、按是否可见。
  - 截断提示的要求。
  - 单结果预算。
- **`doc/llm_context/agent_tool/OpenDAN AgentTool 开发指南.md`**：
  - 说明 `build_output` 和 `build_summary` 的分工。
  - 更新示例（`:321` 处 read_file 的 summary）。
- **`doc/llm_context/prompt_render_engine.md`**、**`doc/llm_context/Render_Prompt_Template_Variables.md`**：
  - 补上对象的默认渲染规则。
  - 后者开头写的引擎路径已经过时：写的是 `llm_context/src/prompt_engine.rs`，实际在 `agent_tool/src/prompt_engine.rs`。
  - 后者 §2 "object 序列化成紧凑 JSON" 要改。
- **`doc/llm_context/xllm_ts_runtime.md` §7**：向量有变化时同步。

## 9. 实施顺序

1. A1–A3（字段语义与落盘）。
2. B1–B3 和 D1–D5。
3. C1–C2。
4. E1–E5（与 1–3 没有依赖，可以并行）。
5. F 随每一步一起改。

整改时不要破坏以下现有做法：

- `input_formats` 的 message / event / attachments / todo 格式（逐字节固定）。
- `background.rs` 的可读行。
- `## Objects` 一行一个对象。
- shell 转 task 时的 once hint。
- read dedup 用"内容摘要 + 版本"判断内容是否相同。

## 10. 裁决（owner，2026-11-26）

"原话"一栏是 owner 写在本文档里的原文。

| # | 问题 | 原话 | 落到 |
| --- | --- | --- | --- |
| Q1 | function_call 模式什么时候降级 | 压缩的时候，重建历史要看上一次推理的时间（缓存是否还在），缓存大概率不在了才会降级 | C1 |
| Q2 | behavior 模式的分级 | 按你的建议（删掉 `compression_level` / `summary_chars`，与 C1 同一规则） | C2 |
| Q3 | 单结果预算 | 16K Token | B1、D3 |
| Q4 | 去重覆盖范围 | 按你的建议（只覆盖 read / read_file，窗口完全相同才算）。这个很多内置 did-object 对象的 read 都应该顺手支持。我记得之前有个讨论，只要正确返回 agent-tool-result，runtime 能自动去重 | D2、D5 |
| Q5 | `<process_result>` 的 submission | 按你的建议（保留 JSON，受预算约束） | E4 |
| Q6 | 对象展示用什么字段 | 尽力用对象本身自己的字段名 | E1、E5 |
| Q7 | 模板遇到没有可读字段的对象 | 回退成 json 并记录日志？ | E1 |
| Q8 | worklog 旧数据 | breaking change，不用管 | §2、A3 |
| Q9 | 集合行由谁生成 | 按你的建议（provider 按集合类型生成） | E5 |

owner 的补充与确认（2026-11-26）：

- **Q1 补充**：压缩必定会导致缓存失效，所以压缩时总是降级；需要按时间判断的只有重建历史。时间判断复用现有的 `prompt_cache_idle`（`refresh_frozen` 用的就是它）。
- Q6、Q7 按下面起草人的理解实施（owner 已确认）。
- **Q6**：理解为不给对象新增统一的展示字段，两行文本从对象已有的字段里取，并沿用原字段名。
  - 已知类型由各自的格式函数或 provider 决定取哪几个字段。
  - 引擎的通用回退按 `title` / `name` / `subject`、`summary` / `description` / `status` 两组常见字段去取。
  - 如果原意是"渲染时以对象自己的字段名作标签"，E5 的行格式已经这样做了，E1 也照此调整。
- **Q7**：原话带问号，按"回退紧凑 JSON + `log::warn!` + `RenderStats` 计数"实施。回退的 JSON 同样受 16K 预算约束。

## 11. 验证

- 第 2 节列出的单测、向量、fixtures 全部通过。向量和 fixtures 的 diff 要逐项审，不能只看通过与否。
- 在本机 Zone 上用真实模型跑五个场景：
  1. 一个 work session，在两个 Turn 里读同一个文件。
  2. 同一个 did-object 对象，先经 `read` 读一次，再经 shell 里的 CLI 读一次。
  3. 一条输出超长的 shell 命令。
  4. plan_simple 的 on_init。
  5. 一个 UI 会话，空闲超过 `prompt_cache_idle` 后再发消息。可以临时调小这个值来跑。
- 从 run 的 snapshot 里检查实际 prompt，记录以下几项和改前的对比：
  - 第二次读（包括对象读取）是否只有一句话。
  - 截断提示能否照做。
  - `<runtime>` 是否已是可读行。
  - 空闲前后历史的形态：空闲前不降级，空闲后才降级。
  - token 数的变化。
