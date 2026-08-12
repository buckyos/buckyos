# llm_message_compress 实现指导

面向后续 code agent 的实现说明。规范侧请配套读：

- [doc/opendan/LLM Compress.md](../doc/opendan/LLM Compress.md) —— 总体设计 / 触发策略 / 接口
- [doc/agent_tool/agent_tool_result_protocol.md](../doc/agent_tool/agent_tool_result_protocol.md) —— `AgentToolResult` 协议与 StepRecord 渲染分级

本文档解决"散落在各处的细节"和"实现路径"。

---

## 1. 现状 vs 目标

### 1.1 已经存在的资产（不要重复造）

| 资产 | 位置 | 作用 |
| --- | --- | --- |
| `AgentToolResult` envelope | [src/frame/agent_tool/src/lib.rs:356](../src/frame/agent_tool/src/lib.rs) | 协议字段全集：`agent_tool_protocol/status/cmd_name/cmd_args/title/summary/output/detail/...` |
| `AgentToolResult::to_tool_result_view()` | [src/frame/agent_tool/src/lib.rs:651](../src/frame/agent_tool/src/lib.rs) | envelope → 渲染层视图的现成转换器 |
| `ToolResultView` | [src/frame/llm_context/src/observation.rs:27](../src/frame/llm_context/src/observation.rs) | StepRecord 消费的结构化视图 |
| `Observation::Success { tool_result: Option<ToolResultView>, ... }` | [src/frame/llm_context/src/observation.rs:76](../src/frame/llm_context/src/observation.rs) | Behavior 模式下每个 action 的结果中已经挂了结构化视图 |
| `StepRecord { actions, action_results, ... }` | [src/frame/llm_context/src/behavior_loop.rs:72](../src/frame/llm_context/src/behavior_loop.rs) | Behavior 一步的内存结构 |
| Full / Medium / Min / Mini 分级渲染 | [src/frame/llm_context/src/step_record.rs](../src/frame/llm_context/src/step_record.rs)（`XmlStepRenderer`）+ `AgentToolResult::render_for_level()` ([lib.rs:629](../src/frame/agent_tool/src/lib.rs)) | "越老越狠"的协议侧实现已经存在 |
| `<<last_step_action_results>>` / `<<step_history>>` 包装 | [step_record.rs](../src/frame/llm_context/src/step_record.rs) | Behavior 模式的 "History 块" 渲染基线 |
| Session 持久化：`round_history/round_<N>.jsonl` | [src/frame/opendan/src/round_history.rs](../src/frame/opendan/src/round_history.rs) | 每个 round 的 entry 流，包含 `Message` / `Step` / `Event` |
| `SessionHistoryReader::read_round / read_range` | [round_history.rs:671](../src/frame/opendan/src/round_history.rs) | 在内存外重新拿到原始记录的唯一权威入口 |
| LLM 压缩主体（当前实现） | [src/frame/agent_tool/src/llm_compress.rs](../src/frame/agent_tool/src/llm_compress.rs) | 已能跑：System+Head Keep+Hot Tail 选 Compress Block，LLM summary，marker 标 stable boundary |

### 1.2 目标差距（要做的事）

按重要性排序：

1. **机械压缩接 AgentToolResult Protocol（路径 A）**
   当前 [`mechanically_compress_tool_result`](../src/frame/agent_tool/src/llm_compress.rs:457) 用 `text.len() > 2048` 这种自定义阈值 + 硬编码模板，**绕开了**已有的 `to_tool_result_view()` + `render_for_level()` 分级。要改成：解析 envelope → 选 level → 用协议侧渲染。
2. **多 pair 合并产物（路径 B）**
   把若干完整 message pair 合成一对 `[user:压缩范围说明, assistant:History 块]`。Behavior 模式可复用 `<<step_history>>` 形态；Agent Loop 没有现成对应物，需要新增。
3. **结构化机械压缩元数据**
   按 [LLM Compress.md §11.3](../doc/opendan/LLM Compress.md) 新版定义：
   ```ts
   interface MechanicalCompressedMeta {
     is_mechanically_compressed: true;
     message_pairs_in_history_block: number; // 0 → 路径 A；>0 → 路径 B
     original_token_count?: number;
     compressed_token_count?: number;
     rule_name: string;
     compressed_at: number;
   }
   ```
   现在没有承接它的字段。`AiMessage` 也没有 `is_compressed/compressed_kind/compressed_from`。一期可以先把 meta JSON 内联进 message text（marker 路线），但接口要预留升级路径。
4. **History 块可叠加合并**
   再次机械压缩时，已存在的 History 块可被识别并扩张（吸收更多老 pair）。靠 #3 的 meta 来识别。
5. **`extra_focus_prompt` / `agent_identity` 注入点**
   `compress()` 现在 system prompt 是硬编码英文（[llm_compress.rs:186-197](../src/frame/agent_tool/src/llm_compress.rs)）。给一个 `extra_focus_prompt: Option<&str>` 参数即可。

---

## 2. llm_compress 怎么拿到"原始记录"

这是这次实现最关键的设计点，先把它讲透。

### 2.1 in-memory 路径（首选）

`compress()` 收到的就是 `&[AiMessage]`，本身就是当前活跃上下文的"原始记录"。但 **`AiMessage::Tool` 的 `AiContent::ToolResult.content` 只携带最终文本**，结构化 envelope 是否能恢复，取决于 ToolManager 把工具结果转换成 AiMessage 时是否保留了 envelope JSON。

实测情况：

- agent_tool crate 内部工具（[llm_explore.rs](../src/frame/agent_tool/src/llm_explore.rs)、[llm_understand_media.rs](../src/frame/agent_tool/src/llm_understand_media.rs)、[workspace.rs](../src/frame/agent_tool/src/workspace.rs)、[llm_tool_carft.rs](../src/frame/agent_tool/src/llm_tool_carft.rs)）emit 出来的 stdout 是合法 `AgentToolResult` JSON（带 `agent_tool_protocol: "1"`）。
- `exec_bash` 在 stdout 是合法 envelope 时会作为 AgentToolResult 转发；普通 bash stdout 不会被强行包装成 envelope。
- 因此 **AiContent::ToolResult.content 里的字符串如果以 `agent_tool_protocol` JSON 起头，就可以反解；否则就是 plain text**。

落到代码上：

```rust
// 伪代码 — 在 llm_compress 里把一条 Tool message 升级回结构化视图
fn try_extract_tool_view(text: &str) -> Option<ToolResultView> {
    let trimmed = text.trim();
    if !trimmed.starts_with('{') { return None; }
    let env: AgentToolResult = serde_json::from_str(trimmed).ok()?;
    if env.agent_tool_protocol != AGENT_TOOL_PROTOCOL_VERSION { return None; }
    Some(env.to_tool_result_view())
}
```

这是最直接的"拿到原始记录"路径 —— 不读盘、不依赖 Behavior 模式。

### 2.2 Behavior 模式额外渠道：StepRecord

Behavior 模式下 ToolManager 已经把 `Observation::Success { tool_result: Some(view), ... }` 喂回 context_loop。但等到 message 落到 `accumulated`（即 `AiMessage::Tool`）时，view 没有跟着挂在 AiMessage 上。

两种选择：

- **(A) 复用 in-memory 路径 2.1**：tool_result 序列化成 envelope JSON 写进 ToolResult text，再在 llm_compress 里反解。优点：和 Chat 模式同构。缺点：序列化/反序列化双向都要做对，且 LLM prompt 里多了一层 JSON。
- **(B) 给 `AiMessage` / `AiContent::ToolResult` 加结构化字段**：直接挂 `Option<ToolResultView>`。优点：零序列化损耗，llm_compress 拿到就是结构化。缺点：`AiMessage` 在 beta2.2 已经重设计过一次（见 [project_aimessage_redesign_beta2_2](../.claude/projects/-Users-liuzhicong-project-buckyos/memory/project_aimessage_redesign_beta2_2.md)），再加一个字段属于 breaking change —— 但 beta2.2 阶段允许 breaking。

**推荐路线**：第一期走 (A)，因为不动 AiMessage 形状、向前兼容、Chat/Behavior 走同一条解析路径。后续若需要在 prompt 里压掉冗余 JSON 再考虑 (B)。

### 2.3 落库路径（仅诊断 / 跨 round 用）

`SessionHistoryReader::read_round(round_index, HistoryView::Raw|Full|MsgOnly)` 读 `round_history/round_<N>.jsonl`，能拿到：

- Chat 模式：`RoundFullPayload::Chat { messages: Vec<AiMessage> }`
- Behavior 模式：`RoundFullPayload::Behavior { steps: Vec<StepRecord> }` —— 这里的 `step.action_results` 是带 `Option<ToolResultView>` 的 `Observation`，结构化最完整。

但 **llm_compress 在线运行时不应走这条路**：

1. 压缩点的 messages 已经在内存里，重新读盘是浪费。
2. round_history 是 audit trail，不是热路径数据源；引入对它的依赖会让 llm_compress 和 opendan crate 紧耦合（当前它在 agent_tool crate）。
3. 离线工具 / dev test / 调试场景才用 `SessionHistoryReader`。

**约束**：llm_compress crate 不要 `use opendan::round_history`。需要持久化诊断时，由 caller（agent_session）从 round_history 加载后再喂给 llm_compress。

---

## 3. 实现步骤

### 阶段 1：机械压缩接协议（路径 A）

**目标**：删掉自拼模板，改用协议分级。

具体动作：

1. 在 [llm_compress.rs](../src/frame/agent_tool/src/llm_compress.rs) 增加 `fn try_decode_agent_tool_envelope(content_text: &str) -> Option<AgentToolResult>`。
2. 改写 `mechanically_compress_tool_result`：
   - 拿到 Tool message 后先尝试 envelope 解析
   - 解析失败 → 走 fallback：保留原逻辑（长 text → 截断 + hash），并把 `rule_name` 标成 `"plain_text_truncate_v1"`
   - 解析成功 → 用 `result.render_for_level(level)` 生成新文本，把同一份 `tool_call_id` 喂回 `AiContent::tool_result_text(call_id, new_text, is_error)`
3. 加入"level 选择"逻辑（参数化，不要写死）：
   - 距尾巴 ≥ N1 pair → `Medium`（用 `summary`）
   - 距尾巴 ≥ N2 pair（更远）→ `Min`（用 `title`）
   - 失败（`is_error`）/带 `task_id` 的 pending → 默认不降级（这是协议精神：失败不机械折叠）
4. `MECHANICAL_TOOL_RESULT_TEXT_THRESHOLD = 2048` 这个硬编码阈值降级为"无 envelope 时的 fallback 阈值"，不再用于 envelope 路径。

**测试**：复用 [`mechanical_tool_result_compresses_when_enough`](../src/frame/agent_tool/src/llm_compress.rs:1153) 的骨架；新加两个用例：
- envelope 解析成功 → 输出包含 `summary` 文本而不是 `ToolResultCompressed:` 模板
- `is_error=true` 的 envelope → 不被机械压缩

### 阶段 2：结构化机械压缩 meta

**目标**：让下一轮压缩能识别"这一条已经被机械压缩过 + 当前 level"。

最小落地（不动 AiMessage shape）：

1. 在 message text 头部追加一行 marker + JSON：
   ```
   [LLM_MECHANICAL_COMPRESS_META_V1]
   {"is_mechanically_compressed":true,"message_pairs_in_history_block":0,"rule_name":"protocol_level_medium","level":"medium","original_token_count":3200,"compressed_token_count":140,"compressed_at_ms":...}
   ```
2. 加 `fn read_mechanical_meta(msg: &AiMessage) -> Option<MechanicalMeta>` 把 marker 行解出来。
3. 下一次 compress 时：
   - 看到已经是 `Min` 的不再降级
   - 看到 `Medium` 的可以再降到 `Min`（升级压缩级别）
   - 失败 envelope 不动

**注意**：marker 必须放在 ToolResult `content` 文本的最前面 + 单独占一行，方便 LLM 看得见但解析又稳定。不要塞进 envelope JSON 内部 —— envelope 是协议字段，不该被压缩流程污染。

### 阶段 3：路径 B —— Agent Loop 的 History 块

**目标**：当 N 个连续 pair 全都 `Min` 级别后还压不下来，把它们合并成一对消息。

形态（参考 [LLM Compress.md 附录](../doc/opendan/LLM Compress.md)）：

```
[user: 压缩范围说明 / 元信息]
[assistant: History 块 — 多 pair 的紧凑文本]
```

History 块文本格式 （Agent Loop）：

```text
History:
  user: <原 user message 一行摘要>
    call(<cmd_name>): <title>
    call(<cmd_name>): <title>
  agent: <原 assistant 一行摘要>

  user: <...>
    call(<...>): <...>
  agent: <...>
```

实现要点：

1. Compress Block 选完后，先做阶段 1+2 的协议降级；如果降到 `Min` 还压不下来，触发路径 B：把选中的 pair 们 fold 成上面这种文本。
2. 产物是 2 条 AiMessage（一对），meta 的 `message_pairs_in_history_block = N`。
3. 下一轮压缩看到 `message_pairs_in_history_block > 0` 的 assistant message，可以把更老的相邻 pair 继续 fold 进同一个 History 块（"老 block 变长"），靠重新拼字符串实现，不要保留多个 block。
4. **Behavior 模式不走这套**：复用 `<<step_history>>` + `XmlStepRenderer` 已有路径。llm_compress 检测到当前 messages 是 Behavior 起源（system prompt / behavior 标记）时，直接跳过路径 B，让 Behavior 自己的 history sediment 机制做。

### 阶段 4：参数注入点

`compress()` 签名扩展：

```rust
pub async fn compress(
    history: &[AiMessage],
    deps: &LLMContextDeps,
    target_token_budget: u32,
    model_alias: &str,
    extra_focus_prompt: Option<&str>,   // 新增
) -> Result<Vec<AiMessage>, LLMComputeError>;
```

在 LLM 压缩路径里把 `extra_focus_prompt` 追加到压缩 system prompt 末尾（保留前面那段 base prompt 不变，这样 prompt cache 稳定性最好）。

---

## 4. 边界与约束（容易踩坑的地方）

### 4.1 协议侧分工不能反

文档原话："上述要求，都由各个 Agent Tool 的实现者根据 Agent Tool Protocol 协议自行实现。机械压缩流程不做判断。"

意思是：

- 工具开发者负责让自家结果在 `title/summary/output/detail` 上都有合理填充，且 `summary` 真的能"独立读懂"（[protocol §5](../doc/agent_tool/agent_tool_result_protocol.md)）。
- llm_compress 只负责选 level（按位置 + token budget），**绝不**根据 `cmd_name` / `tool_name` 自己改文本结构（不许有 `if tool == "read_file" then ...`）。
- 出现"summary 不可读"的问题 → bug 报到工具实现，不报到 llm_compress。

### 4.2 失败 / pending 默认不机械压缩

协议明确 `status = error|pending` 时 `summary` 仍承担可读结论，但**信息密度高、易丢关键内容**（错误栈、task_id 等）。

实现策略：

- `is_error == true` 或 `status = pending` → 跳过协议降级，进 LLM 压缩路径（保留为原文）。
- 但 LLM 压缩输入里允许它们出现，靠 base prompt 强调"保留失败原因 / 错误堆栈"。

### 4.3 不要触碰 stable boundary

当前 [`is_compressed_pair_at`](../src/frame/agent_tool/src/llm_compress.rs:414) / [`is_stable_boundary_message`](../src/frame/agent_tool/src/llm_compress.rs:428) 通过 `COMPRESS_META_MARKER` / `COMPRESS_SUMMARY_MARKER` 识别已压缩对。

新增的机械压缩 meta marker (`[LLM_MECHANICAL_COMPRESS_META_V1]`) **不是** stable boundary —— 它只是"这一条已经压过一次"的标记，下一轮可以继续升级压缩等级，也可以被 LLM 压缩吸收。识别函数要区分这两类 marker，不要让机械压缩 meta 错位变成 boundary。

### 4.4 `AiContent::ToolResult.content` 是 `Vec<AiToolResultContent>`

当前 `mechanically_compress_tool_result` 写了 `content.len() != 1` 直接 return None。如果一个 tool result 有多段（例如 text + image），这条规则会跳过它。后续如果工具开始返回多段内容（LLM understand media 已经有这倾向），需要：

- 找 envelope 段（`AiToolResultContent::Text` 且 starts_with `{` 且能解析）
- 其他段（image / 二进制引用）原样保留 —— 媒体段不要乱降级，那是 LLM 看图用的

### 4.5 不要写"先机械再 LLM"两次改写

[LLM Compress.md §8](../doc/opendan/LLM Compress.md) 明文反对：

> 推荐策略：
> if 机械压缩能达到 target_ratio: 本次只做机械压缩
> else: 放弃本次机械压缩结果，直接做一次 LLM 压缩

目前实现 [llm_compress.rs:165-167](../src/frame/agent_tool/src/llm_compress.rs:165) 是对的（机械压不够直接进 LLM 路径），但**阶段 1+2+3 之后这条还要继续守住**：阶段 3 的路径 B 也属于"机械"，如果路径 B 走完还没达到 target，整体放弃机械结果，重头走一次 LLM 压缩 —— 不要在路径 B 输出上再叠 LLM。

---

## 5. 测试位置

- 单元测试：[`src/frame/agent_tool/src/llm_compress.rs`](../src/frame/agent_tool/src/llm_compress.rs) 末尾 `mod tests`。已有 `mechanical_tool_result_compresses_when_enough` / `existing_compressed_pair_is_stable_boundary` / `over_budget_summarizes_middle` 等，照同一套 `make_deps + msg + compressed_pair` 套路写。
- 集成测试：`#[ignore]` 的 `dev_compress_real_codex_session_jsonl` 已经从真实 Codex jsonl 加载 history，可以扩展成"加载 → 跑机械压缩 → 验证 level 降档命中"的回归。
- Round history 相关测试：[round_history.rs:1040+](../src/frame/opendan/src/round_history.rs)。如果做"从落库记录验证 envelope 还原"，在那里加。

---

## 6. 不要做的事

1. **不要把 llm_compress 拉去依赖 `opendan` crate**。当前 llm_compress 在 `agent_tool` crate，依赖链：`agent_tool → llm_context → buckyos_api`。要保持。round_history 是 opendan 的，单向引用：opendan 调 llm_compress，不反过来。
2. **不要在 llm_compress 里 `match tool_name` 做工具特化**。所有 per-tool 行为都该在工具自己的 `AgentToolResult` 填充上体现。
3. **不要扩 `AiMessage` 字段（一期）**。结构化 meta 一期靠 marker 行 + JSON inline。等阶段 3 的产物形态稳定下来再讨论是否给 AiMessage 加 `compressed_from`。
4. **不要让机械压缩流程触发外部 IO**。读盘（round_history）、网络只在 LLM 压缩路径才允许。
5. **不要在 Behavior 模式下启用路径 B**。Behavior 已有 `<<step_history>>` + sediment，重叠会冲突。判断方式：检查 messages 里是否有 `<<step_history>>` 或 `behavior` 命名空间的 system message。
6. **不要修改 `AGENT_TOOL_PROTOCOL_VERSION` 或 envelope 字段语义**。压缩流程是消费者，不是协议作者。新字段缺失就用 None，不要为压缩需求反推到协议。

---

## 7. 验收清单

完成后能复述出这些事实，且对应测试都过：

- [ ] 机械压缩**只**通过 `AgentToolResult::render_for_level()` 改写消息（无 envelope 走 fallback 路径）。
- [ ] 同一条 ToolResult 在第二轮压缩时能从 `Medium` 升级到 `Min`，靠 marker meta 识别。
- [ ] Compress Block 选完 → 协议降级若不够 → 触发路径 B（Agent Loop）合并 N 个 pair 成 `[user, assistant History 块]` 对。
- [ ] 路径 B 产物在再下一轮可被识别，并把更老的相邻 pair 继续 fold 进同一个 History 块（不产生多个并列 block）。
- [ ] 失败 / pending tool result 不被机械压缩。
- [ ] Behavior 模式的 messages 不进路径 B（让 `<<step_history>>` 自管）。
- [ ] `compress()` 多了 `extra_focus_prompt` 参数，注入位置不破坏 base prompt 的前缀（prompt cache 友好）。
- [ ] llm_compress crate **没有**新增对 opendan crate 的依赖。
- [ ] 单元测试覆盖：envelope 解析成功 / 失败 fallback / level 升级 / 路径 B 合并 / 路径 B 叠加。
