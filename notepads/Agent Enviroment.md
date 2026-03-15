# Agent Enviroment 需求

## 1. 目标与范围

### 1.1 目标

在 Agent Runtime 的 `agent_environment` 中实现一个 **安全、确定性、可审计** 的模板替换引擎，用于将 Behavior 配置与 Prompt 组装中的模板占位符（`{{...}}`）渲染为最终文本，支持：

* **变量替换**：如 `{{new_msg}}`、`{{new_event}}`（运行时输入源）。
* **文件片段 include**：如 `{{workspace/to_agent.md}}`、`{{cwd/to_agent.md}}`（从工作区/当前目录注入片段）。
* **Null 语义**：模板替换后若 `input` 为空，则 `generate_input()` 返回 `None`，step 跳过推理并进入 `WAIT`（零 LLM 空转）。

### 1.2 适用位置（MVP）

引擎必须至少能在下列字段中使用（它们来自 Behavior 配置的文本字段）：

* `process_rule`
* `policy`（虽然示例主要展示 process_rule，但 prompt 组装中它也是 system prompt 的组成）
* `input`（关键：决定是否推理）

### 1.3 非目标（Non-goals）

* 不支持执行脚本/表达式求值（禁止 `eval` 类能力）。
* 不提供网络访问能力。
* 不负责 Prompt 的 delimiter/observation 清洗截断（这是 prompt builder 的职责，但模板引擎需要提供元信息以便上层做安全处理）。

---

## 2. 术语与概念

* **Render**：将模板字符串渲染为最终字符串（或 Null）。
* **Placeholder**：`{{ ... }}` 内的内容。
* **Resolver**：将 placeholder 映射到具体值的解析器（变量解析器/文件解析器）。
* **Null**：表示“无内容、应当被整体忽略”的语义（不是空字符串的简单同义）。`input` 渲染结果为 Null 时会导致跳过推理。

---

## 3. 模板语法（MVP）

### 3.1 基本形式

* 占位符使用双大括号：

  * `{{name}}`
  * `{{ workspace/to_agent.md }}`（允许前后空白）
* 匹配规则：**非嵌套** `{{` … `}}`，最左匹配到最近的 `}}` 结束（MVP 简化）。

### 3.2 Placeholder 内容语法（MVP）

MVP 支持两类：

1. **变量名**

* 语法：`[A-Za-z_][A-Za-z0-9_]*`
* 例：`new_msg`、`new_event`

2. **命名空间路径**（文件 include）

* 语法：`<ns>/<rel_path>`
* `ns`（MVP 必须支持）：

  * `workspace`
  * `cwd`
* `rel_path`：相对路径（禁止绝对路径、禁止 `..` 穿越）

### 3.3 转义（MVP）

* MUST：支持输出字面量 `{{`、`}}`
  建议语法：`\{{`、`\}}`（或 `{{"{{"}}` 这类会引入表达式，不推荐）。

> 选择最易实现且不引入表达式求值的方式。

---

## 4. 渲染上下文与变量字典

### 4.1 RenderContext（建议数据结构）

模板引擎不直接依赖全局单例，所有信息通过 `RenderContext` 注入，确保可测试、可并发：

* `session_id`
* `agent_id` / `subagent_id`
* `cwd_path`（当前执行目录）
* `workspace_roots`（0..N 个 workspace 根目录；至少包含 session 绑定的 workspace，如有）
* `input_sources`（运行时输入源）：

  * `new_msg`（string?）
  * `new_event`（string?）
  * SHOULD：`current_todo_details`、`last_step_summary` 等（文档列出 Input 可能包含这些来源，后续行为会用到）
* `policy`：PolicyEngine 句柄/接口（用于 fs 权限校验）
* `limits`：安全与性能限制（见第 8 节）

> 注：文档明确 `<<Input>>` 的来源可能多种，且若无法得到 input 本 step 跳过。

---

## 5. 解析与替换规则

### 5.1 变量解析（MVP MUST）

* 若 placeholder 为变量名（如 `new_msg`）：

  * 从 `RenderContext.input_sources` 查找同名字段
  * 取值类型：

    * `None` → 解析结果为 Null
    * 非空字符串 → 解析结果为该字符串
    * 空字符串（仅空白）→ 视为 Null（默认；可配置）

### 5.2 文件 include 解析（MVP MUST）

* 若 placeholder 为 `workspace/<rel_path>` 或 `cwd/<rel_path>`：

  * 先确定根目录：

    * `cwd` → `RenderContext.cwd_path`
    * `workspace` → `RenderContext.workspace_roots`（若多个 workspace：MVP 选择 “primary workspace”；或按顺序第一个；必须在需求里固定一种确定性规则）
  * 路径拼接与规范化：

    * MUST：拒绝绝对路径（`/`、`C:\`）
    * MUST：拒绝包含 `..` 的路径穿越
    * MUST：最终规范化路径必须落在根目录内（realpath containment）
  * 权限校验：

    * MUST：调用 PolicyEngine 进行读权限校验（Root/SubAgent fs_scope 都在这里统一限制）。
  * 文件读取：

    * 默认 UTF-8
    * 二进制/不可解码：按“读取失败”处理（见错误策略）
    * 内容过大：按截断/拒绝策略处理（见第 8 节）

### 5.3 递归渲染（SHOULD）

* 允许 include 的文件内容中继续包含 `{{...}}`，引擎递归渲染。
* MUST：设置最大递归深度与循环检测（避免互相 include 死循环）。
* 默认建议：`max_depth = 3~5`。

---

## 6. Null 语义与渲染模式（关键）

文档明确：`session.generate_input()` 判空，无有效 input 则 step 跳过并进入 `WAIT`。

因此模板引擎必须至少提供两种渲染模式：

### 6.1 Mode A：Text 模式（用于 process_rule/policy 等）

* 返回类型：`string`（永不返回 None）
* Null 行为：

  * placeholder 解析为 Null → 替换为空字符串 `""`
  * 渲染后执行轻度规范化（SHOULD）：

    * 移除多余的连续空行（例如 3 行以上压到 2 行）
    * 去掉行尾多余空白
* 用途：system prompt 的组成部分（process_rule、policy 等）。

### 6.2 Mode B：InputBlock 模式（用于 behavior_cfg.input → exec_input）

* 返回类型：`Optional<string>`（可能为 None）
* 输入模板通常多行拼装（如 `{{new_event}}\n{{new_msg}}`）。
* Null 行为（MVP MUST，给出确定规则，确保“零 LLM 空转”准确）：

  1. 先按 Text 模式替换 placeholder（Null→空串）
  2. 然后做 **行级清理**：

     * 按行 split
     * 对每行 `trim()`；若为空行则丢弃
  3. 重新用 `\n` join
  4. 若最终字符串为空 → 返回 `None`
* 上层行为：

  * `exec_input is None` → `session.update_state("WAIT")` 并 return，不触发推理。

> 这条规则是实现“零 LLM 空转”的关键验收点。

---

## 7. 错误处理与策略（MVP 必须可控）

模板替换会遇到缺变量、缺文件、权限拒绝等情况。为了运行时稳定，必须定义默认策略，并允许配置严格模式。

### 7.1 错误类型（建议枚举）

* `UnknownVariable`
* `FileNotFound`
* `PermissionDenied`
* `PathTraversalDenied`
* `DecodeError`
* `IncludeTooLarge`
* `RecursionLimitExceeded`
* `CyclicIncludeDetected`
* `TemplateSyntaxError`（例如缺 `}}`）

### 7.2 默认策略（MVP 建议）

* **容错为主**（避免因为一个可选片段导致整个 step 失败）：

  * UnknownVariable → 视为 Null（替换为空）
  * FileNotFound → 视为 Null
  * PermissionDenied → 视为 Null + 记录审计（安全上不暴露更多信息）
  * TemplateSyntaxError → 返回原文不替换 + 记录错误（或仅跳过该占位符）
* 但必须有 **Strict 开关**（用于开发/测试或关键模板）：

  * Strict=true 时，上述错误应导致渲染失败（抛错/返回 error），由上层决定中止 step 或降级。

### 7.3 Error 输出与可观测性（MVP MUST）

* 模板引擎必须输出 `RenderReport`（见第 9 节），至少记录：

  * errors（类型、placeholder 原文、原因）
  * resolved_refs（解析成功的变量/文件路径）
  * skipped_refs（被当作 Null 的原因：缺失/权限/为空等）

---

## 8. 安全与资源限制（MVP MUST）

文档强调：Prompt 必须分段加 delimiter，tool/action 输出默认不可信并需要截断清洗；同时 PolicyEngine 负责 fs permissions、SubAgent fs_scope 等护栏。

模板引擎需要落实其中与“include 文件”和“内容进入 prompt”相关的底层安全要求：

### 8.1 FS 权限与 SubAgent 限制（MUST）

* 所有文件 include 必须走 PolicyEngine 校验读权限：

  * Root Agent 与 SubAgent 必须按各自 `fs_scope` 约束。SubAgent 的 fs_scope 文档明确要求限制。 
* 必须禁止路径穿越、符号链接逃逸（realpath containment）。

### 8.2 大小与深度限制（MUST）

防止 prompt 被无意撑爆、或被恶意模板撑爆：

* `max_placeholder_len`：placeholder 字符长度上限（例如 256）
* `max_include_bytes`：单个文件 include 最大字节数（例如 64KB，超限截断或报错）
* `max_total_render_bytes`：一次渲染的总输出上限（例如 256KB）
* `max_depth`：递归 include 深度上限（例如 5）

> 截断策略必须写入 RenderReport，便于审计与调试。

### 8.3 内容分类元信息（SHOULD）

虽然 delimiter/observation 清洗由上层负责，但模板引擎应在报告中标注每个片段的来源类型，帮助上层决定放在哪个 prompt 区域（system/user/observation）。文档要求 tool/action 输出默认不可信并放 observation。

---

## 9. 对外接口（工程可直接实现）

### 9.1 API 设计（建议）

提供一个纯函数式核心接口，便于在 Rust/Go/TS 中实现：

```text
render(template: string, ctx: RenderContext, opt: RenderOptions) -> RenderResult
```

#### RenderOptions（建议字段）

* `mode`: `TEXT | INPUT_BLOCK`
* `strict`: bool
* `allow_recursive`: bool
* `max_depth`
* `max_include_bytes`
* `max_total_render_bytes`
* `trim_blank_lines`: bool（INPUT_BLOCK 默认 true）

#### RenderResult（建议结构）

* `text`: string | null
* `report`: RenderReport

#### RenderReport（MVP 必须至少包含）

* `placeholders_total`: int
* `resolved_variables`: list<{name, bytes}>
* `included_files`: list<{ns, rel_path, abs_path_hash?, bytes, truncated: bool}>
* `skipped`: list<{placeholder, reason, error_type?}>
* `errors`: list<{placeholder, error_type, message}>
* `duration_ms`
* `cache_hits`（如实现缓存）

> abs_path 建议不要直接落日志（避免泄露），可用 hash + rel_path + ns 组合。

### 9.2 与 Runtime 的集成点（MVP MUST）

必须落在文档描述的 step-loop 里：

1. `session.generate_input(behavior_cfg)`

* 对 `behavior_cfg.input` 做 `render(..., mode=INPUT_BLOCK)`
* 若 `RenderResult.text == null` → 返回 `None` 触发零空转逻辑。

2. `behavior_cfg.build_prompt(exec_input)`

* 对 `process_rule/policy` 做 `render(..., mode=TEXT)`
* 将渲染结果放入 System Prompt 的对应段落。

---

## 10. 规范化与一致性要求（防止提示词不稳定）

### 10.1 确定性（MUST）

同样的 `template + ctx` 必须输出完全一致的结果：

* 不允许读取“当前时间”之类的隐式变量
* 不允许遍历目录并按不稳定顺序拼接（如未来支持 `workspace/*.md`，必须排序；MVP 不支持通配符）

### 10.2 并发安全（MUST）

* 引擎必须可在多 session worker 并发调用（文档允许多个 session 并发，且 SubAgent 可并发）。
* 若实现缓存：必须线程安全；推荐 **每次 render 使用局部 cache**（一次渲染内缓存 include），避免全局共享复杂度。

---

## 11. 参考行为（Examples）

### 11.1 route behavior 示例（来自文档）

* `process_rule` include workspace/cwd 文件片段
* `input` 拼装 `new_event/new_msg`

预期渲染行为：

* 如果 `workspace/to_agent.md` 不存在：该占位符为 Null → 在 Text 模式中替换为空串
* 如果当前 step 没有 `new_event` 且没有 `new_msg`：

  * INPUT_BLOCK 模式行级清理后为空 → 返回 None → step 不推理，session 进入 WAIT。

### 11.2 文档强调的 cwd 概念（对 include 有用）

* 文档提到 session 有 cwd 概念，便于定位 workspace 目录（这也是支持 `cwd/...` include 的依据）。

---

## 12. 测试用例与验收标准（工程落地清单）

### 12.1 单元测试（必须）

1. **变量替换**

* 模板：`"A={{new_msg}}"`
* ctx.new_msg="hi" → `"A=hi"`

2. **变量为 Null**

* ctx.new_msg=None
* TEXT 模式 → `"A="`
* INPUT_BLOCK 模式：模板 `"{{new_msg}}"` → `None`

3. **多行 input 拼装**

* 模板：

  ```
  {{new_event}}
  {{new_msg}}
  ```
* new_event=None, new_msg="m" → 输出 `"m"`（无空行）
* new_event="e", new_msg=None → 输出 `"e"`
* 两者都 None → 输出 None（触发零空转）

4. **文件 include 成功**

* workspace_root 下存在 `to_agent.md`
* 模板 `{{workspace/to_agent.md}}` → 输出文件内容（可包含换行）

5. **文件不存在**

* → 视为 Null（默认容错）+ RenderReport 记录 FileNotFound

6. **权限拒绝**

* PolicyEngine deny read
* → 视为 Null + RenderReport 记录 PermissionDenied（不泄露绝对路径）

7. **路径穿越攻击**

* `{{workspace/../secret}}` → 必须拒绝（PathTraversalDenied）
* strict=false：替换为空 + 记录
* strict=true：返回 error

8. **递归 include 与循环**

* A include B, B include A → 必须循环检测并终止（CyclicIncludeDetected 或 RecursionLimitExceeded）

9. **大小限制**

* include 文件超过 `max_include_bytes`：

  * 若策略=truncate：输出截断内容 + truncated=true
  * 若策略=error：按 strict 行为决定失败或置 Null

### 12.2 集成测试（必须）

1. 在 session step-loop 中：

* `generate_input()` 渲染为空 → exec_input=None → 不触发 LLM 推理 → session 状态变 WAIT。

2. 在 prompt builder 中：

* 渲染后的 process_rule/policy 放入 System Prompt，并且上层仍然对 tool/action 输出走 observation + 清洗截断（模板引擎不破坏该约束）。

---

## 13. 建议的迭代规划

### MVP（建议 1~2 周可落地）

* `{{var}}`、`{{workspace/rel}}`、`{{cwd/rel}}`
* 两种模式：TEXT / INPUT_BLOCK
* Null 语义 + 行级清理
* PolicyEngine 文件读权限校验
* 递归 include（可先不做，或只做 1 层）+ 深度限制
* RenderReport（最小字段）



